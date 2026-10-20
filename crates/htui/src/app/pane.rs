//! MOD-57 M1, M2: the external editor in the TUI pane, as the shell sees it.
//!
//! With `HTUI_EDITOR_PANE` set, the event loop's `$EDITOR` step does not suspend the terminal: it
//! calls [`App::open_editor`], which writes the temp file ([`TempEdit`]), spawns the editor on a
//! pseudo-terminal through the closure it is handed (the loop's is
//! [`PtyChild::spawn`](crate::editor::pane::PtyChild::spawn)) and holds it as one
//! [`OpenEditor`] (P1). One at a time (P6): a second edit is answered `EDITOR_BUSY` from
//! [`App::take_external_edit`] (PD-8).
//!
//! - **Transport (P7).** The child's threads send [`PaneEvent`]s on the loop's fourth arm;
//!   [`App::on_pane_event`] parses `Output` into the [`PaneScreen`] (writing back the replies it
//!   owes, DSR and DA1) and, on `Exited`, reads the file back through [`TempEdit::finish`] (a
//!   failed wait is answered "lost track of `<value>`", R1 L-1) and hands the outcome to the
//!   asking tab through [`App::finish_external_edit`], so no view's outcome handling changes.
//!   Every event carries a [`PaneId`]; an event of an editor already finished or aborted is
//!   dropped (B4).
//! - **Keys (P5, P6, M2 E1-E3).** While an editor is alive [`App::on_key`] asks it first.
//!   Focused, only `editor.focus` ([`Stack::EDITOR_FOCUSED`]) is htui's; every other key,
//!   `ctrl-c` included, is encoded ([`encode_key`]) and written. The editor has the keys only
//!   while focused, on screen and under no modal overlay or `?` box (`editor_has_keys`, the one
//!   rule keys, pastes, the title, the status line and the cursor read); the first key or paste
//!   that finds a focused editor without them (typed for the editor) is swallowed with the
//!   refusal and unfocuses it, rather than run as a command or answer a prompt. With htui
//!   holding the keys, `ctrl-c` quits at `on_key`'s one C3 site and a modal overlay's keys are
//!   the overlay's. On the asking tab a key resolves through [`Stack::EDITOR_OPEN`]:
//!   `editor.focus` refocuses, `editor.abort` asks first (the abort question, M2 R1 M-2: only
//!   `editor.abort` again kills the editor and answers [`EDITOR_ABORTED`]; any other key keeps
//!   the edit), every global the shell maps acts (tab switches included); anything else is
//!   refused, so nothing reaches the asking view. On any other tab only `editor.focus` is
//!   checked (it brings the asking tab back, B16) and every other key and paste is that tab's.
//! - **M2 (PRD D4, D5).** E4: `global.quit` asks first ([`App::request_quit`]); `ctrl-c` quits
//!   at once. E5: nothing steers the asking tab (a scope change, a reveal into it). E6: another
//!   tab's status line is led by the open edit and the way back. E7: the `?` box lists the
//!   editor keys that act where the user is. E8: the mouse is the view's on another tab.
//! - **Where it draws (P2).** The active tab names its editing rect through
//!   [`Ctx::claim_editor_area`](crate::app::Ctx::claim_editor_area) during its render; the pane
//!   goes there when the claim is at least [`MIN_PANE`] inside the body, else over the whole
//!   body, and only while the asking tab is the active one
//!   ([`ui::editor_pane::render`](crate::ui::editor_pane::render)). The PTY follows the drawn
//!   rect after each draw ([`App::resize_editor`], PD-1).
//!
//! Nothing here blocks the UI task: `write` queues, `resize` is one ioctl, `kill` sends on a
//! channel (`R-NF-3`). The screen's contents and the bytes are never logged or `Debug`ged.

use std::io;
#[cfg(test)]
use std::path::Path;

use crossterm::event::KeyEvent;
use portable_pty::CommandBuilder;
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use zeroize::Zeroizing;

use crate::app::action::{Action, OverlayAction, TabAction};
use crate::app::state::App;
use crate::editor::pane::{PaneChild, PaneEvent, PaneId, PaneScreen, PaneSize};
use crate::editor::pane::{encode_key, encode_paste};
use crate::editor::{
    EDITOR_ABORTED, EDITOR_BUSY, EditorCommand, EditorExit, ExternalEdit, ExternalEditOutcome,
    PANE_VAR, TempEdit,
};
use crate::keys::{Act, Hint, HintSpec, KeyChord, Stack};
use crate::ui::overlay::Overlay;
use crate::ui::tabs::TabId;

/// PRD Q3: the smallest claimed rect a pane uses; below it, the whole tab body. 40x8 until T7's
/// manual nvim run in the Notes compose box says otherwise.
pub const MIN_PANE: Size = Size {
    width: 40,
    height: 8,
};

/// The head of the refusal while an editor is alive and unfocused (MOD-57 P6).
const EDITOR_LOCKED: &str = "the editor is open";

/// MOD-57 M2 E5: what the asking tab waits for before anything may steer it.
const FINISH_FIRST: &str = "finish or abort it first";

/// M2 R1 nit: what `editor.focus` under the leave dialog says.
const ANSWER_FIRST: &str = "answer the question first";

/// The refusal's tail and the status line while a modal overlay is over the editor (R0 F3): its
/// keys are the overlay's, so none of the editor's is offered. Neutral (M2 R1 L-1): it fits a
/// reply's prompt, an offered view (find, the queue, the waiting list, the switcher) and the quit
/// question alike.
const COVERED_FIRST: &str = "close this first";

/// The focused pane's title and status hint.
const FOCUSED_HINT: HintSpec = &[Hint::One(Act::EditorFocus, "to htui")];

/// The unfocused status line.
const UNFOCUSED_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
    // M2 R1 nit: tabs switch from the asking tab (E3).
    Hint::Pair(Act::NextTab, Act::PrevTab, "switch tabs"),
    Hint::One(Act::Quit, "quit"),
    Hint::One(Act::Help, "help"),
];

/// The refusal's tail.
const LOCK_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
];

/// MOD-57 M2 E1, E6: the way back from another tab.
const BACK_HINT: HintSpec = &[Hint::One(Act::EditorFocus, "back")];

/// One live in-pane editor. Field order is drop order: the child (kill requested) before the temp
/// file (removed). `Debug`: id, tab, focus, size, child, temp; never the screen.
pub(super) struct OpenEditor {
    /// Its events' id.
    id: PaneId,
    /// The tab that asked, and is answered.
    tab: TabId,
    /// The process side.
    child: Box<dyn PaneChild>,
    /// The file it edits, read back on `Exited`.
    temp: TempEdit,
    /// What it drew.
    screen: PaneScreen,
    /// For the title and `finish`'s sentences.
    cmd: EditorCommand,
    /// Whether the editor has the keys.
    focused: bool,
}

impl core::fmt::Debug for OpenEditor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OpenEditor")
            .field("id", &self.id)
            .field("tab", &self.tab)
            .field("focused", &self.focused)
            .field("size", &self.screen.size())
            .field("child", &self.child)
            .field("temp", &self.temp)
            .finish_non_exhaustive()
    }
}

impl App {
    /// MOD-57 P1: the loop's pane step. Writes the temp file, spawns through `spawn` at the size
    /// the asking tab's pane was last drawn at, and holds the editor focused. A temp-file or spawn
    /// failure answers the tab at once ([`finish_external_edit`](Self::finish_external_edit));
    /// nothing stays open.
    pub fn open_editor<F>(&mut self, tab: TabId, edit: &ExternalEdit, cmd: EditorCommand, spawn: F)
    where
        F: FnOnce(PaneId, CommandBuilder, PaneSize) -> io::Result<Box<dyn PaneChild>>,
    {
        self.dirty = true;
        if self.editor.is_some() {
            self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_BUSY.to_owned()));
            return;
        }
        let temp = match TempEdit::create(&edit.text, &edit.stem) {
            Ok(temp) => temp,
            Err(outcome) => {
                self.finish_external_edit(tab, outcome);
                return;
            }
        };
        let size = match self.editor_rect {
            Some((drawn, rect)) if drawn == tab => pane_size(rect),
            _ => PaneSize::DEFAULT,
        };
        let id = PaneId::next();
        match spawn(id, cmd.pty_command(temp.path()), size) {
            Ok(child) => {
                // R0 F4: the `?` box swallows no key, so a view can ask from under it; the editor
                // opens with the keys, so the box closes.
                self.help_visible = false;
                self.editor = Some(OpenEditor {
                    id,
                    tab,
                    child,
                    temp,
                    screen: PaneScreen::new(size),
                    cmd,
                    focused: true,
                });
            }
            Err(err) => {
                drop(temp);
                self.finish_external_edit(tab, ExternalEditOutcome::Failed(pane_failure(&err)));
            }
        }
    }

    /// One transport event (P7): `Output` of the open editor is parsed and its replies written
    /// back, and asks for a frame only while the editor's tab is on screen (M2 R1 L-3); `Exited`
    /// reads the file back ([`TempEdit::finish`]) and hands the outcome to the tab that asked
    /// ([`finish_external_edit`](Self::finish_external_edit)). An event of any other id is
    /// dropped (B4).
    pub fn on_pane_event(&mut self, event: PaneEvent) {
        if self.editor_id() != Some(event.id()) {
            return;
        }
        match event {
            PaneEvent::Output { bytes, .. } => {
                let visible = self.editor_visible();
                if let Some(editor) = self.editor.as_mut() {
                    let replies = editor.screen.feed(&bytes);
                    if !replies.is_empty() {
                        editor.child.write(&replies);
                    }
                    self.dirty |= visible;
                }
            }
            PaneEvent::Exited {
                status, elapsed, ..
            } => {
                // The child drops here: its kill request finds the wait thread gone (harmless).
                let Some(OpenEditor { tab, temp, cmd, .. }) = self.editor.take() else {
                    return;
                };
                // M2 R1 M-2: the edit the abort question asked about has ended. (The quit
                // question stays: B-M2-5.)
                self.close_abort_question();
                let outcome = match status {
                    Ok(status) => temp.finish(&cmd, Ok(EditorExit::from(&status)), elapsed),
                    // R1 L-1: the editor started (the spawn succeeded) and waiting on it failed,
                    // so `finish`'s "could not start" sentence would be wrong. Nothing is read
                    // back: the editor may not have finished writing.
                    Err(err) => {
                        drop(temp);
                        ExternalEditOutcome::Failed(pane_lost(&cmd, &err))
                    }
                };
                self.finish_external_edit(tab, outcome);
            }
        }
    }

    /// The post-draw step (PD-1): resizes the PTY and the screen when the asking tab's pane rect
    /// changed. Nothing when the pane was not drawn this frame.
    pub fn resize_editor(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let Some((tab, rect)) = self.editor_rect else {
            return;
        };
        if tab != editor.tab {
            return;
        }
        let size = pane_size(rect);
        if size != editor.screen.size() {
            editor.child.resize(size);
            editor.screen.resize(size);
            self.dirty = true;
        }
    }

    /// Whether an in-pane editor is alive.
    #[must_use]
    pub fn editor_open(&self) -> bool {
        self.editor.is_some()
    }

    /// The live editor's id (tests and the end-to-end suite pump events by it).
    #[must_use]
    pub fn editor_id(&self) -> Option<PaneId> {
        self.editor.as_ref().map(|editor| editor.id)
    }

    /// The live editor's temp file.
    #[cfg(test)]
    pub(super) fn editor_file(&self) -> Option<&Path> {
        self.editor.as_ref().map(|editor| editor.temp.path())
    }

    /// Whether the editor's tab is the one on screen.
    pub(super) fn editor_visible(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| self.tabs.active_id() == Some(editor.tab))
    }

    /// The asking tab's id while an editor is alive.
    pub(super) fn editor_tab(&self) -> Option<TabId> {
        self.editor.as_ref().map(|editor| editor.tab)
    }

    /// The asking tab's title (as the tab strip shows it), else its id; empty without an editor.
    pub(super) fn editor_title(&self) -> &str {
        let Some(tab) = self.editor_tab() else {
            return "";
        };
        self.tabs.title(tab).unwrap_or(tab.0)
    }

    /// `the editor is open in <Tab>`: the head of every sentence said away from the asking tab.
    fn editor_place(&self) -> String {
        format!("{EDITOR_LOCKED} in {}", self.editor_title())
    }

    /// MOD-57 M2 E1: `editor.focus`'s way back from another tab (`Ctrl+4 back`); empty when it
    /// is unbound.
    fn editor_back(&self) -> String {
        self.keys.hint(Stack::EDITOR_FOCUSED, BACK_HINT)
    }

    /// Whether a modal overlay is up over the editor: its keys take `on_key`'s overlay path.
    pub(super) fn editor_covered(&self) -> bool {
        self.overlays.top().is_some_and(Overlay::is_modal)
    }

    /// R0: whether the editor has the keys: focused, its tab on screen, no modal overlay and no
    /// `?` box over it. The one rule [`editor_key`](Self::editor_key),
    /// [`editor_paste`](Self::editor_paste), [`editor_status`](Self::editor_status) and
    /// [`draw_editor`](Self::draw_editor) (title and cursor) all read.
    fn editor_has_keys(&self) -> bool {
        self.editor.as_ref().is_some_and(|editor| editor.focused)
            && self.editor_visible()
            && !self.editor_covered()
            && !self.help_visible
    }

    /// The lock's refusal: under a modal overlay [`COVERED_FIRST`]; on another tab the way back
    /// (MOD-57 M2, B-M2-9: `the editor is open in Skills: Ctrl+4 back`, `editor.abort` does not
    /// act there); on the asking tab the editor's keys.
    fn lock_refusal(&self) -> String {
        if self.editor_covered() {
            format!("{EDITOR_LOCKED}: {COVERED_FIRST}")
        } else if !self.editor_visible() {
            let place = self.editor_place();
            match self.editor_back() {
                back if back.is_empty() => place,
                back => format!("{place}: {back}"),
            }
        } else {
            format!(
                "{EDITOR_LOCKED}: {}",
                self.keys.hint(Stack::EDITOR_OPEN, LOCK_HINT)
            )
        }
    }

    /// The first input (key or paste) that finds a focused editor without the keys (a reply moved
    /// the active tab, or opened a modal prompt over it): htui has the keys from now on. That input
    /// was typed for the editor (a vim `ctrl-c`, the `q` of `:wq`), so it is swallowed with the
    /// refusal, never run as a lock command or an overlay answer: it must not quit, abort, move a
    /// tab or answer a prompt the user had not seen yet (T5 verify round 3). `true` if swallowed.
    fn editor_lost_keys(&mut self) -> bool {
        if self.editor_has_keys() || !self.editor.as_ref().is_some_and(|editor| editor.focused) {
            return false;
        }
        let refusal = self.lock_refusal();
        if let Some(editor) = self.editor.as_mut() {
            editor.focused = false;
        }
        self.status = Some(refusal);
        true
    }

    /// A key while an editor is alive (P5, P6, MOD-57 M2 E1-E3); `false` hands it to the rest of
    /// [`App::on_key`]'s chain. In order, the first that applies:
    ///
    /// 1. The first key that finds a focused editor without the keys is swallowed and unfocuses
    ///    it ([`editor_lost_keys`](Self::editor_lost_keys)).
    /// 2. With the keys, the editor's, unless it is the focus toggle.
    /// 3. Under a modal overlay (a reply's prompt, the leave dialog): `false`, the overlay's
    ///    keys (`ctrl-c` there is the overlay's quit layer, or the dialog's `leave.confirm`);
    ///    `editor.focus` does not act there, and under the leave dialog says so
    ///    ([`focus_under_the_question`](Self::focus_under_the_question)).
    /// 4. On another tab: `editor.focus` ([`Stack::EDITOR_FOCUSED`]) refocuses and brings the
    ///    asking tab back (B16, [`focus_from_another_tab`](Self::focus_from_another_tab));
    ///    anything else is `false`, as if no editor were alive.
    /// 5. On the asking tab ([`asking_tab_key`](Self::asking_tab_key)): the first candidate of
    ///    [`Stack::EDITOR_OPEN`] that maps (`editor.focus`, `editor.abort`, then every global the
    ///    shell maps, `ctrl-c`'s `global.quit` included: the leave dialog); a key nothing maps is
    ///    refused on the status line. Nothing reaches the asking view.
    pub(super) fn editor_key(&mut self, key: KeyEvent, chord: KeyChord) -> bool {
        if self.editor_lost_keys() {
            return true;
        }
        let focus = self
            .keys
            .actions(Stack::EDITOR_FOCUSED, chord)
            .contains(&Act::EditorFocus);
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if editor.focused {
            if focus {
                editor.focused = false;
            } else {
                let application_cursor = editor.screen.screen().application_cursor();
                if let Some(bytes) = encode_key(key, application_cursor) {
                    editor.child.write(&bytes);
                }
            }
            return true;
        }
        if self.editor_covered() {
            return focus && self.focus_under_the_question();
        }
        if !self.editor_visible() {
            // E1: the way back, ahead of the view; E2: `editor.abort` is not checked here.
            return focus && self.focus_from_another_tab();
        }
        self.asking_tab_key(chord)
    }

    /// `editor.focus` under a modal overlay (`editor_key` step 3): the leave dialog has no use
    /// for it, so the status line says to answer the question first rather than swallow it
    /// silently (M2 R1 nit), and `true`. Under any other overlay `false`: the overlay's key.
    fn focus_under_the_question(&mut self) -> bool {
        let question = self.leave_overlay.is_some()
            && self.overlays.top().map(Overlay::id) == self.leave_overlay;
        if question {
            self.status = Some(format!("{EDITOR_LOCKED}: {ANSWER_FIRST}"));
        }
        question
    }

    /// `editor.focus` on another tab (`editor_key` step 4, E1): refocuses, closes the `?` box and
    /// brings the asking tab back (B16). Always `true`.
    fn focus_from_another_tab(&mut self) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        editor.focused = true;
        let tab = editor.tab;
        self.help_visible = false;
        self.update(Action::Tab(TabAction::Focus(tab)));
        true
    }

    /// A key on the asking tab, unfocused, under no modal overlay (`editor_key` step 5, E3): the
    /// first candidate of [`Stack::EDITOR_OPEN`] that maps; a key nothing maps is refused on the
    /// status line. Always `true`: nothing reaches the asking view.
    fn asking_tab_key(&mut self, chord: KeyChord) -> bool {
        for act in self.keys.actions(Stack::EDITOR_OPEN, chord) {
            match act {
                Act::EditorFocus => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.focused = true;
                    }
                    self.help_visible = false;
                    return true;
                }
                Act::EditorAbort => {
                    // M2 R1 M-2: asks first; the `?` box closes, as the question's keys are
                    // all its own.
                    self.help_visible = false;
                    let action = self.request_abort();
                    self.update(action);
                    return true;
                }
                other => {
                    if let Some(action) = self.action_for(other) {
                        self.update(action);
                        return true;
                    }
                }
            }
        }
        self.status = Some(self.lock_refusal());
        true
    }

    /// A paste while an editor is alive, by a key's rule (R0 F2): with the keys, to the editor
    /// (bracketed if it asked); the first that finds a focused editor without them is swallowed
    /// ([`editor_lost_keys`](Self::editor_lost_keys)); on the asking tab, unfocused, dropped (a
    /// paste is never a key). `false` hands it to [`App::on_paste`]'s own path: under a modal
    /// overlay (which never passes it to the asking tab) and on another tab, whose paste it is
    /// (MOD-57 M2 E3).
    pub(super) fn editor_paste(&mut self, text: &Zeroizing<String>) -> bool {
        if self.editor_lost_keys() {
            return true;
        }
        let visible = self.editor_visible();
        let covered = self.editor_covered();
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        if editor.focused {
            let bytes = encode_paste(text, editor.screen.screen().bracketed_paste());
            editor.child.write(&bytes);
            return true;
        }
        visible && !covered
    }

    /// Records where the active tab's pane goes this frame (`editor_rect`, pane or not, so the
    /// first spawn has the view's size) and draws the editor there when its tab is the active
    /// one. The title says whether the editor has the keys
    /// ([`editor_has_keys`](Self::editor_has_keys), as the status line does); the real cursor
    /// shows only when it has them and nothing is drawn over it.
    pub(super) fn draw_editor(&mut self, frame: &mut Frame<'_>, body: Rect) {
        let rect = pane_rect(self.editor_area.get(), body);
        self.editor_rect = self.tabs.active_id().map(|id| (id, rect));
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        if self.tabs.active_id() != Some(editor.tab) {
            return;
        }
        let has_keys = self.editor_has_keys();
        let focused = has_keys && self.overlays.is_empty();
        let title = if has_keys {
            format!(
                " {} \u{b7} {} ",
                editor.cmd.value(),
                self.keys.hint(Stack::EDITOR_FOCUSED, FOCUSED_HINT)
            )
        } else {
            format!(" {} \u{b7} htui has the keys ", editor.cmd.value())
        };
        crate::ui::editor_pane::render(
            frame,
            rect,
            editor.screen.screen(),
            &title,
            focused,
            &self.theme,
        );
    }

    /// The status line while an editor is alive on the active tab and no error is up; `None`
    /// without one or on another tab ([`editor_notice`](Self::editor_notice) leads that tab's own
    /// line, MOD-57 M2 E6). A focused editor without the keys
    /// ([`editor_has_keys`](Self::editor_has_keys): under a modal overlay or the `?` box) reads
    /// as unfocused; under a modal overlay, whose keys are the overlay's, the line offers no
    /// editor key and says to close it first (R0 F3, M2 R1 L-1).
    pub(super) fn editor_status(&self) -> Option<String> {
        if !self.editor_visible() {
            return None;
        }
        Some(if self.editor_has_keys() {
            format!(
                "the editor has the keys \u{b7} {}",
                self.keys.hint(Stack::EDITOR_FOCUSED, FOCUSED_HINT)
            )
        } else if self.editor_covered() {
            format!("{EDITOR_LOCKED}: {COVERED_FIRST}")
        } else {
            self.keys.hint(Stack::EDITOR_OPEN, UNFOCUSED_HINT)
        })
    }

    /// MOD-115 M2 D6 (`R-TUI-18`, replacing MOD-57 M2 E4's editor-only question): what
    /// `global.quit` (`q`, `ctrl-c`) resolves to: the leave dialog, always; [`Action::Quit`] only
    /// on a bare shell with no `leave_overlay`. The dialog's `leave.confirm` (`Enter`, a second
    /// `ctrl-c`) emits `Action::Quit` itself.
    #[must_use]
    pub(super) fn request_quit(&self) -> Action {
        self.leave_overlay
            .map_or(Action::Quit, |id| Action::Overlay(OverlayAction::Open(id)))
    }

    /// MOD-57 M2 R1 M-2: what `editor.abort` on the asking tab resolves to. With `abort_overlay`
    /// set, opening the abort question (only `editor.abort` again aborts); else
    /// [`Action::AbortEdit`] at once (a bare shell, as M1 did).
    #[must_use]
    fn request_abort(&self) -> Action {
        match self.abort_overlay {
            Some(id) => Action::Overlay(OverlayAction::Open(id)),
            None => Action::AbortEdit,
        }
    }

    /// MOD-57 M2 E5: `the editor is open in <Tab>: finish or abort it first` while an editor is
    /// alive: the refusal of a scope change or of a reveal into the asking tab.
    #[must_use]
    pub(super) fn editor_steer_refusal(&self) -> Option<String> {
        self.editor.as_ref()?;
        Some(format!("{}: {FINISH_FIRST}", self.editor_place()))
    }

    /// MOD-57 M2 E6: what leads another tab's status line while an editor is alive:
    /// `editing in <Tab> · Ctrl+4 back · `, the way back dropped under a modal overlay (where it
    /// does not act), while the editor is still focused (a reply moved the tab: the next key,
    /// that `Ctrl+4` too, is swallowed and its refusal offers the way back, M2 R1 L-2) or when
    /// unbound. Always ends with the separator. `None` without an editor and on the asking tab.
    #[must_use]
    pub(super) fn editor_notice(&self) -> Option<String> {
        let editor = self.editor.as_ref()?;
        if self.editor_visible() {
            return None;
        }
        let head = format!("editing in {}", self.editor_title());
        let back = if self.editor_covered() || editor.focused {
            String::new()
        } else {
            self.editor_back()
        };
        Some(if back.is_empty() {
            format!("{head} \u{b7} ")
        } else {
            format!("{head} \u{b7} {back} \u{b7} ")
        })
    }

    /// `editor.abort` (B8): the kill is requested and the temp file removed at once, and the
    /// asking tab is answered `Failed(EDITOR_ABORTED)`. A later event of the editor is inert.
    pub(super) fn abort_editor(&mut self) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        self.close_abort_question();
        let tab = editor.tab;
        editor.child.kill();
        drop(editor);
        self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned()));
    }

    /// Closes the abort question if it is open: the edit it asks about has ended (M2 R1 M-2), or
    /// the asking tab is no longer on screen (R2 LOW-3).
    pub(super) fn close_abort_question(&mut self) {
        if let Some(id) = self.abort_overlay {
            self.close_overlay(id);
        }
    }
}

/// The claim when it lies inside `body` at least [`MIN_PANE`] large (clipped to the body), else
/// the whole body.
fn pane_rect(claim: Option<Rect>, body: Rect) -> Rect {
    claim
        .map(|claim| claim.intersection(body))
        .filter(|rect| rect.width >= MIN_PANE.width && rect.height >= MIN_PANE.height)
        .unwrap_or(body)
}

/// The editor's grid in `rect`: everything below the title rule.
fn pane_size(rect: Rect) -> PaneSize {
    PaneSize::new(rect.height.saturating_sub(1), rect.width)
}

/// Why the pane could not start, and the way back to the suspend mode.
fn pane_failure(err: &io::Error) -> String {
    format!("could not run the editor in a pane ({err}); unset {PANE_VAR} to suspend instead")
}

/// The editor ran in the pane, and waiting on it failed (R1 L-1): its file is not read back.
fn pane_lost(cmd: &EditorCommand, err: &io::Error) -> String {
    format!(
        "lost track of `{}` ({err}); nothing was changed",
        cmd.value()
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use htui_core::fixtures::ids;
    use htui_core::model::{ItemId, ProjectRef, RequirementId, Scope, WorkspaceSummary};
    use portable_pty::{CommandBuilder, ExitStatus};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::{Position, Rect};
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Borders, Paragraph};
    use ratatui::{Frame, Terminal};
    use tokio::sync::mpsc;
    use zeroize::Zeroizing;

    use super::{EDITOR_LOCKED, MIN_PANE};
    use crate::app::{
        Action, App, Ctx, Handled, OverlayAction, RevealKind, RevealTarget, TabAction,
    };
    use crate::editor::pane::{PaneChild, PaneEvent, PaneSize};
    use crate::editor::{
        EDITOR_ABORTED, EDITOR_BUSY, EditorCommand, ExternalEdit, ExternalEditOutcome, PANE_VAR,
    };
    use crate::keys::{Act, Context, Keys, Stack, views};
    use crate::store_worker::{StoreReply, StoreRequest};
    use crate::ui::overlay::{AbortConfirm, LeaveDialog, Overlay, OverlayId};
    use crate::ui::tabs::{Tab, TabId};

    /// What the fake child was asked to do.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Write(Vec<u8>),
        Resize(PaneSize),
        Kill,
        Dropped,
    }

    type Log = Rc<RefCell<Vec<Call>>>;

    /// A recording `PaneChild`: no process.
    struct FakeChild {
        log: Log,
    }

    impl core::fmt::Debug for FakeChild {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("FakeChild")
        }
    }

    impl PaneChild for FakeChild {
        fn write(&mut self, bytes: &[u8]) {
            self.log.borrow_mut().push(Call::Write(bytes.to_vec()));
        }
        fn resize(&mut self, size: PaneSize) {
            self.log.borrow_mut().push(Call::Resize(size));
        }
        fn kill(&mut self) {
            self.log.borrow_mut().push(Call::Kill);
        }
    }

    impl Drop for FakeChild {
        fn drop(&mut self) {
            self.log.borrow_mut().push(Call::Dropped);
        }
    }

    /// What an [`Editing`] tab saw.
    #[derive(Debug, Default)]
    struct Seen {
        keys: Vec<KeyEvent>,
        pastes: Vec<String>,
        heard: Vec<ExternalEditOutcome>,
    }

    type Shared = Rc<RefCell<Seen>>;

    /// A tab that asks for the editor on `e`, records everything, draws `VIEW` over its area and
    /// claims `claim` for the editor. `stack` is its key stack (`None`: unconverted); `lost`
    /// counts the mouse losses it was told of.
    struct Editing {
        id: TabId,
        seen: Shared,
        claim: Rc<RefCell<Option<Rect>>>,
        mouse: bool,
        stack: Option<Stack<'static>>,
        lost: Rc<Cell<u32>>,
    }

    impl Tab for Editing {
        fn id(&self) -> TabId {
            self.id
        }
        fn title(&self) -> &str {
            self.id.0
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_scope_change(&mut self, _scope: &Scope) {}
        fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            self.seen.borrow_mut().keys.push(key);
            if key.code == KeyCode::Char('e') {
                ctx.emit(Action::EditExternally(asked()));
                return Handled::Consumed;
            }
            // M2 R2 LOW-2: a tab that tries to abort the edit itself.
            if key.code == KeyCode::Char('!') {
                ctx.emit(Action::AbortEdit);
                return Handled::Consumed;
            }
            Handled::Pass
        }
        fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
            self.seen.borrow_mut().pastes.push(text.to_owned());
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
            let lines: Vec<Line<'_>> = (0..area.height)
                .map(|_| Line::raw("VIEW ".repeat(usize::from(area.width) / 5 + 1)))
                .collect();
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::new()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", self.id.0)),
                ),
                area,
            );
            if let Some(claim) = *self.claim.borrow() {
                ctx.claim_editor_area(claim);
            }
        }
        fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
            self.seen.borrow_mut().heard.push(outcome);
        }
        fn wants_mouse(&self) -> bool {
            self.mouse
        }
        fn on_mouse_lost(&mut self) {
            self.lost.set(self.lost.get() + 1);
        }
        fn key_stack(&self) -> Option<Stack<'static>> {
            self.stack
        }
    }

    /// A modal overlay that draws nothing.
    struct Shade;

    impl Overlay for Shade {
        fn id(&self) -> OverlayId {
            OverlayId("shade")
        }
        fn title(&self) -> &str {
            "Shade"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    /// A modal overlay that records the keys it is given and passes `Esc` (so the overlay
    /// stack closes it) and `ctrl-c` (the overlay stack's quit layer, MOD-115 M2 B4), as the
    /// real prompts pass a chord they do not use.
    struct Prompt(Rc<RefCell<Vec<KeyEvent>>>);

    impl Overlay for Prompt {
        fn id(&self) -> OverlayId {
            OverlayId("prompt")
        }
        fn title(&self) -> &str {
            "Prompt"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            if key.code == KeyCode::Esc
                || crate::keys::KeyChord::from_event(key) == crate::keys::CTRL_C
            {
                return Handled::Pass;
            }
            self.0.borrow_mut().push(key);
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    /// A modal overlay that records the pastes it is given (R0 F2).
    struct Pasted(Rc<RefCell<Vec<String>>>);

    impl Overlay for Pasted {
        fn id(&self) -> OverlayId {
            OverlayId("pasted")
        }
        fn title(&self) -> &str {
            "Pasted"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, _key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled {
            Handled::Consumed
        }
        fn on_paste(&mut self, text: &str, _ctx: &mut Ctx<'_>) -> Handled {
            self.0.borrow_mut().push(text.to_owned());
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    /// A workspace switcher stub (M2 R1 M-3): `Enter` picks `Platform` as the real one does
    /// (`SetScope`, then its own close); every other key is its own.
    struct Switcher;

    impl Switcher {
        const ID: OverlayId = OverlayId("switcher");
    }

    impl Overlay for Switcher {
        fn id(&self) -> OverlayId {
            Self::ID
        }
        fn title(&self) -> &str {
            "Switcher"
        }
        fn is_modal(&self) -> bool {
            true
        }
        fn wants_requests(&self, _scope: &Scope) -> Vec<StoreRequest> {
            Vec::new()
        }
        fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
            if key.code == KeyCode::Enter {
                ctx.emit(Action::SetScope {
                    workspace: workspace("Platform"),
                });
                ctx.emit(Action::Overlay(OverlayAction::Close));
            }
            Handled::Consumed
        }
        fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}
        fn render(&self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &Ctx<'_>) {}
    }

    const ASKER: TabId = TabId("asker");
    const OTHER: TabId = TabId("other");

    /// The edit every test hands out.
    fn asked() -> ExternalEdit {
        ExternalEdit {
            text: "body\n".to_owned(),
            stem: "implement".to_owned(),
        }
    }

    /// `value` as `$VISUAL`.
    fn cmd(value: &str) -> EditorCommand {
        EditorCommand::resolve(|key| (key == "VISUAL").then(|| value.to_owned()))
    }

    /// One registered tab of the bench.
    struct Bench {
        seen: Shared,
        claim: Rc<RefCell<Option<Rect>>>,
        lost: Rc<Cell<u32>>,
    }

    /// One tab of [`shell_with`]: its id, whether it wants the mouse, and its key stack.
    type Spec = (TabId, bool, Option<Stack<'static>>);

    /// A shell with `ids` registered as [`Editing`] tabs, the first one active: no mouse, no
    /// stack.
    fn shell(ids: &[TabId]) -> (App, Vec<Bench>) {
        let specs: Vec<Spec> = ids.iter().map(|id| (*id, false, None)).collect();
        shell_with(&specs)
    }

    /// A shell with `specs` registered as [`Editing`] tabs, the first one active.
    fn shell_with(specs: &[Spec]) -> (App, Vec<Bench>) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx);
        let benches = specs
            .iter()
            .map(|&(id, mouse, stack)| {
                let bench = Bench {
                    seen: Shared::default(),
                    claim: Rc::default(),
                    lost: Rc::default(),
                };
                app.register_tab(Box::new(Editing {
                    id,
                    seen: Rc::clone(&bench.seen),
                    claim: Rc::clone(&bench.claim),
                    mouse,
                    stack,
                    lost: Rc::clone(&bench.lost),
                }));
                bench
            })
            .collect();
        (app, benches)
    }

    /// A shell with [`ASKER`] and [`OTHER`], an editor open for `ASKER` and unfocused (htui has
    /// the keys, on the asking tab).
    fn unfocused_two() -> (App, Vec<Bench>, Log) {
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        app.on_key(ctrl('4'));
        (app, benches, log)
    }

    /// Registers the abort question and names it as the abort overlay, as `register_all` does
    /// (M2 R1 M-2).
    fn abortable(app: &mut App) {
        app.overlay_factories
            .register(AbortConfirm::ID, || Box::new(AbortConfirm::new()));
        app.abort_overlay = Some(AbortConfirm::ID);
    }

    /// Registers the leave dialog and names it as the leave overlay, as `register_all` does.
    fn confirmable(app: &mut App) {
        app.overlay_factories
            .register(LeaveDialog::ID, || Box::new(LeaveDialog::new()));
        app.leave_overlay = Some(LeaveDialog::ID);
    }

    fn enter() -> KeyEvent {
        press(KeyCode::Enter, KeyModifiers::NONE)
    }

    /// `global.next_tab` (MOD-115 M2 D4: `ctrl-pgdn`; `Tab` moves the pane focus).
    fn next_tab() -> KeyEvent {
        press(KeyCode::PageDown, KeyModifiers::CONTROL)
    }

    /// `global.prev_tab` (`ctrl-pgup`).
    fn prev_tab() -> KeyEvent {
        press(KeyCode::PageUp, KeyModifiers::CONTROL)
    }

    /// The id of the overlay on top.
    fn top(app: &App) -> Option<OverlayId> {
        app.overlays.top().map(Overlay::id)
    }

    /// Whether the open editor has been handed the keys (`focused`).
    fn focused(app: &App) -> bool {
        app.editor.as_ref().is_some_and(|editor| editor.focused)
    }

    /// The keys `bench`'s tab saw, as chords' codes.
    fn codes(bench: &Bench) -> Vec<KeyCode> {
        bench
            .seen
            .borrow()
            .keys
            .iter()
            .map(|key| key.code)
            .collect()
    }

    /// What the fake spawn was handed.
    type Spawned = Rc<RefCell<Vec<(Vec<String>, PaneSize)>>>;

    /// Opens an editor for `tab` with a fake spawn that records its argv and size.
    fn open(app: &mut App, tab: TabId, log: &Log) -> Spawned {
        let spawned = Spawned::default();
        let record = Rc::clone(&spawned);
        let log = Rc::clone(log);
        app.open_editor(tab, &asked(), cmd("nvim"), move |_id, command, size| {
            record.borrow_mut().push((argv(&command), size));
            Ok(Box::new(FakeChild { log }) as Box<dyn PaneChild>)
        });
        spawned
    }

    fn argv(command: &CommandBuilder) -> Vec<String> {
        command
            .get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// A shell with one asking tab and an editor open for it.
    fn opened() -> (App, Vec<Bench>, Log) {
        let (mut app, benches) = shell(&[ASKER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        (app, benches, log)
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn plain(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn writes(log: &Log) -> Vec<Vec<u8>> {
        log.borrow()
            .iter()
            .filter_map(|call| match call {
                Call::Write(bytes) => Some(bytes.clone()),
                _ => None,
            })
            .collect()
    }

    fn output(app: &mut App, bytes: &[u8]) {
        let id = app.editor_id().expect("an editor is open");
        app.on_pane_event(PaneEvent::Output {
            id,
            bytes: bytes.to_vec(),
        });
    }

    fn exited(app: &mut App, code: u32, elapsed: Duration) {
        let id = app.editor_id().expect("an editor is open");
        app.on_pane_event(PaneEvent::Exited {
            id,
            status: Ok(ExitStatus::with_exit_code(code)),
            elapsed,
        });
    }

    /// One frame of `w`x`h`.
    fn draw(app: &mut App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("a test backend");
        term.draw(|frame| app.render(frame)).expect("the app draws");
        term
    }

    /// The buffer as text, one trimmed line per row.
    fn text(buffer: &Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            let line: String = (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// The header row's flash (MOD-115): the status text now rides the right of row 0.
    fn flash_row(buffer: &Buffer) -> String {
        row_from(buffer, 0, 0).trim_end().to_owned()
    }

    /// Row `y` of the buffer from column `x`.
    fn row_from(buffer: &Buffer, x: u16, y: u16) -> String {
        (x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    const FOCUSED_TITLE: &str = " nvim · Ctrl+4 to htui ";
    const UNFOCUSED_TITLE: &str = " nvim · htui has the keys ";
    const FOCUSED_STATUS: &str = "the editor has the keys · Ctrl+4 to htui";
    const UNFOCUSED_STATUS: &str =
        "Ctrl+4 to the editor · Ctrl+x abort · Ctrl+PgDn/Ctrl+PgUp switch tabs · q quit · ? help";
    const REFUSAL: &str = "the editor is open: Ctrl+4 to the editor · Ctrl+x abort";
    /// R0 F3, M2 R1 L-1: under any modal overlay, a prompt or a view (find, the queue, the quit
    /// question), on the asking tab.
    const COVERED: &str = "the editor is open: close this first";
    /// MOD-57 M2 (B-M2-9): the swallow's sentence off screen.
    const OFF_SCREEN: &str = "the editor is open in asker: Ctrl+4 back";
    /// MOD-57 M2 E6: the status prefix on another tab.
    const NOTICE: &str = "editing in asker \u{b7} Ctrl+4 back \u{b7} ";
    /// MOD-57 M2 E5: a steer of the asking tab, refused.
    const STEER_REFUSAL: &str = "the editor is open in asker: finish or abort it first";

    #[test]
    fn a_focused_editor_gets_every_key_ctrl_c_included() {
        let (mut app, benches, log) = opened();
        for key in [
            plain('a'),
            ctrl('c'),
            plain('q'),
            press(KeyCode::Tab, KeyModifiers::NONE),
            plain('?'),
            press(KeyCode::Esc, KeyModifiers::NONE),
            ctrl('x'),
            press(KeyCode::F(1), KeyModifiers::NONE),
            press(KeyCode::Up, KeyModifiers::NONE),
        ] {
            app.on_key(key);
        }
        let expected: Vec<Vec<u8>> = [
            &b"a"[..],
            b"\x03",
            b"q",
            b"\t",
            b"?",
            b"\x1b",
            b"\x18",
            b"\x1bOP",
            b"\x1b[A",
        ]
        .iter()
        .map(|bytes| bytes.to_vec())
        .collect();
        assert_eq!(writes(&log), expected);
        assert!(!app.should_quit, "ctrl-c is the editor's");
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the tab saw nothing"
        );
        assert!(!app.help_visible);
        assert!(app.editor_open());
    }

    #[test]
    fn the_focus_key_toggles_and_is_never_written() {
        let (mut app, _benches, log) = opened();
        app.on_key(ctrl('4'));
        assert!(writes(&log).is_empty(), "the toggle is never written");
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));

        app.on_key(plain('?'));
        assert!(app.help_visible, "unfocused, `?` is htui's");
        app.on_key(ctrl('4'));
        assert_eq!(app.editor_status().as_deref(), Some(FOCUSED_STATUS));
        assert!(!app.help_visible, "refocusing closes the `?` box");
        app.on_key(plain('a'));
        assert_eq!(writes(&log), vec![b"a".to_vec()]);
    }

    /// MOD-115 M2 D6, `R-TUI-18` (rewrites MOD-57 M2 E4's): `ctrl-c` and `q` both open the
    /// leave dialog, and a second `ctrl-c` leaves. A bare shell (no leave overlay) quits at once,
    /// as M1 did.
    #[test]
    fn unfocused_ctrl_c_and_q_open_the_leave_dialog_and_a_second_ctrl_c_leaves() {
        let (mut app, _benches, log) = opened();
        confirmable(&mut app);
        app.on_key(ctrl('4'));
        app.on_key(ctrl('c'));
        assert!(!app.should_quit, "ctrl-c asks first");
        assert_eq!(top(&app), Some(LeaveDialog::ID));
        app.on_key(ctrl('c'));
        assert!(app.should_quit, "the second ctrl-c leaves");
        assert!(writes(&log).is_empty());
        drop(app);
        assert_eq!(
            log.borrow().last(),
            Some(&Call::Dropped),
            "the child goes with App"
        );

        let (mut app, _benches, log) = opened();
        confirmable(&mut app);
        app.on_key(ctrl('4'));
        app.on_key(plain('q'));
        assert!(!app.should_quit, "`q` asks first");
        assert_eq!(top(&app), Some(LeaveDialog::ID));
        assert!(app.editor_open());
        assert!(writes(&log).is_empty());

        let (mut app, _benches, log) = opened();
        app.on_key(ctrl('4'));
        app.on_key(plain('q'));
        assert!(app.should_quit, "a bare shell quits at once");
        drop(app);
        assert_eq!(log.borrow().last(), Some(&Call::Dropped));
    }

    /// MOD-115 M2 D6: with an in-pane editor alive the leave dialog names the open edit, from
    /// the asking tab and from another; without one it does not.
    #[test]
    fn the_leave_dialog_names_the_open_edit() {
        for from_other in [false, true] {
            let (mut app, _benches, _log) = unfocused_two();
            confirmable(&mut app);
            if from_other {
                app.on_key(next_tab());
            }
            app.on_key(plain('q'));
            let frame = text(draw(&mut app, 100, 30).backend().buffer());
            assert!(
                frame.contains("\u{270e} an open edit (unsaved changes are lost)"),
                "{from_other}: {frame}"
            );
        }
        let (mut app, _benches) = shell(&[ASKER, OTHER]);
        confirmable(&mut app);
        app.on_key(plain('q'));
        let frame = text(draw(&mut app, 100, 30).backend().buffer());
        assert!(frame.contains("Leave htui?"), "{frame}");
        assert!(!frame.contains("an open edit"), "{frame}");
    }

    /// MOD-115 M2 D6, from the asking tab and from another: `n` and `Esc` keep the edit as it
    /// was (unfocused, where the user was), `Enter` leaves and the editor goes with `App`.
    #[test]
    fn q_asks_first_enter_leaves_and_n_keeps_the_editor() {
        for from_other in [false, true] {
            let (mut app, benches, log) = unfocused_two();
            confirmable(&mut app);
            if from_other {
                app.on_key(next_tab());
            }
            let here = app.tabs.active_id();

            for close in [plain('n'), press(KeyCode::Esc, KeyModifiers::NONE)] {
                app.on_key(plain('q'));
                assert_eq!(top(&app), Some(LeaveDialog::ID), "{from_other}: asked");
                assert!(!app.should_quit);
                app.on_key(close);
                assert!(app.overlays.is_empty(), "{from_other} {close:?}: closed");
                assert!(app.editor_open(), "{from_other} {close:?}: the edit stays");
                assert!(!focused(&app), "{from_other} {close:?}: unfocused");
                assert_eq!(
                    app.tabs.active_id(),
                    here,
                    "{from_other}: where the user was"
                );
            }

            app.on_key(plain('q'));
            app.on_key(plain('y'));
            assert!(!app.should_quit, "{from_other}: `y` is no answer here");
            app.on_key(enter());
            assert!(app.should_quit, "{from_other}: `Enter` leaves");
            assert!(writes(&log).is_empty());
            assert!(benches[0].seen.borrow().keys.is_empty());
            drop(app);
            assert_eq!(log.borrow().last(), Some(&Call::Dropped), "{from_other}");
        }
    }

    /// M2 R1 L-1: the covered sentence fits every modal overlay on the asking tab, the leave
    /// dialog and an offered view included, and a refused key says it too.
    #[test]
    fn under_any_modal_overlay_the_asking_tab_says_close_this_first() {
        let (mut app, _benches, _log) = unfocused_two();
        confirmable(&mut app);
        app.on_key(plain('q'));
        assert_eq!(top(&app), Some(LeaveDialog::ID));
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));
        app.on_key(press(KeyCode::Esc, KeyModifiers::NONE));

        app.push_overlay(Box::new(Shade));
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));
    }

    /// MOD-115 M2 D6 with an editor alive: `ctrl-c` opens the leave dialog on the asking tab and
    /// on another, and leaves under it (`R-TUI-18`'s second `ctrl-c`).
    #[test]
    fn ctrl_c_opens_the_dialog_on_any_tab_and_leaves_under_it() {
        for (other, asked) in [(false, false), (true, false), (false, true), (true, true)] {
            let (mut app, _benches, log) = unfocused_two();
            confirmable(&mut app);
            if other {
                app.on_key(next_tab());
            }
            if asked {
                app.on_key(plain('q'));
                assert_eq!(top(&app), Some(LeaveDialog::ID));
            } else {
                app.on_key(ctrl('c'));
                assert!(!app.should_quit, "other {other}: asked first");
                assert_eq!(top(&app), Some(LeaveDialog::ID), "other {other}");
            }
            app.on_key(ctrl('c'));
            assert!(app.should_quit, "other {other}, asked {asked}");
            assert!(writes(&log).is_empty());
        }
    }

    /// The question's stack admits no `global.quit`: a second `q` is swallowed by the modal path.
    #[test]
    fn q_under_the_question_is_swallowed_and_opens_no_second_box() {
        let (mut app, benches, _log) = unfocused_two();
        confirmable(&mut app);
        app.on_key(plain('q'));
        app.on_key(plain('q'));
        assert_eq!(app.overlays.iter().count(), 1, "one question");
        assert!(!app.should_quit);
        assert!(app.editor_open());
        assert!(benches[0].seen.borrow().keys.is_empty());
    }

    /// M2 R1 nit: `editor.focus` under the leave dialog, from the asking tab or another, is not
    /// swallowed silently: the status line says to answer it first. The question stays up.
    #[test]
    fn the_focus_key_under_the_question_says_answer_it_first() {
        for from_other in [false, true] {
            let (mut app, benches, log) = unfocused_two();
            confirmable(&mut app);
            if from_other {
                app.on_key(next_tab());
            }
            app.on_key(plain('q'));
            app.on_key(ctrl('4'));
            assert_eq!(
                app.status.as_deref(),
                Some("the editor is open: answer the question first"),
                "{from_other}"
            );
            assert_eq!(top(&app), Some(LeaveDialog::ID), "{from_other}");
            assert!(!focused(&app), "{from_other}");
            assert!(writes(&log).is_empty());
            assert!(benches[0].seen.borrow().keys.is_empty());
            assert!(
                !codes(&benches[1]).contains(&KeyCode::Char('4')),
                "{from_other}: no view had it"
            );
        }
    }

    /// B-M2-5: an editor that exits under the leave dialog leaves it up; `Enter` still leaves,
    /// `n` closes.
    #[test]
    fn an_editor_that_exits_under_the_dialog_leaves_it_up_and_enter_still_leaves() {
        for (answer, quits) in [(enter(), true), (plain('n'), false)] {
            let (mut app, benches, _log) = unfocused_two();
            confirmable(&mut app);
            app.on_key(plain('q'));
            exited(&mut app, 0, Duration::from_secs(2));
            assert!(!app.editor_open());
            assert_eq!(
                benches[0].seen.borrow().heard.len(),
                1,
                "the asker heard it"
            );
            assert_eq!(top(&app), Some(LeaveDialog::ID), "the dialog stays up");
            app.on_key(answer);
            assert_eq!(app.should_quit, quits, "{answer:?}");
            assert_eq!(app.overlays.is_empty(), !quits, "{answer:?}");
        }
    }

    /// MOD-57 M2 E3: on the asking tab, unfocused, every global acts: a tab switch leaves the
    /// editor running and undrawn, and the asking tab never sees the key.
    #[test]
    fn on_the_asking_tab_every_global_acts_and_tabs_switch() {
        let (mut app, benches, log) = unfocused_two();
        app.on_key(next_tab());
        assert_eq!(app.tabs.active_id(), Some(OTHER), "`Tab` switches");
        assert!(app.editor_open(), "the editor keeps running");
        let term = draw(&mut app, 100, 30);
        assert!(
            !text(term.backend().buffer()).contains(" nvim \u{b7} "),
            "and is not drawn"
        );
        assert!(writes(&log).is_empty());
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the asker saw nothing"
        );

        app.on_key(plain('1'));
        assert_eq!(app.tabs.active_id(), Some(ASKER), "`1` from the other tab");
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));

        app.on_key(plain('2'));
        assert_eq!(app.tabs.active_id(), Some(OTHER), "`2` from the asking tab");
        app.on_key(prev_tab());
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the asker saw nothing"
        );
        assert!(writes(&log).is_empty());
        assert!(app.status.is_none(), "nothing was refused");
    }

    /// MOD-57 M2 E3 (rewrites M1's `unfocused_every_other_key_is_refused_and_nothing_moves`):
    /// a key nothing global resolves is refused on the asking tab and never reaches it.
    #[test]
    fn on_the_asking_tab_a_non_global_key_is_refused_and_never_reaches_it() {
        let (mut app, benches, log) = unfocused_two();

        app.on_key(plain('?'));
        assert!(app.help_visible, "`?` toggles help");
        app.on_key(plain('?'));
        assert!(!app.help_visible);

        for key in [
            plain('j'),
            // Not offered in a bare shell: nothing maps it.
            plain('w'),
            plain('e'),
            ctrl('s'),
            press(KeyCode::Esc, KeyModifiers::NONE),
        ] {
            app.on_key(key);
            assert_eq!(app.status.as_deref(), Some(REFUSAL), "{key:?}");
            assert_eq!(app.tabs.active_id(), Some(ASKER), "{key:?}");
        }
        assert!(
            benches
                .iter()
                .all(|bench| bench.seen.borrow().keys.is_empty())
        );
        assert!(writes(&log).is_empty());
        assert!(!app.should_quit);
        assert!(
            app.take_external_edit().is_none(),
            "no view asked for anything"
        );
    }

    /// M2 R1 M-3 (E3, E5): an offered global opens its view from the asking tab: `w` opens the
    /// switcher, whose pick is refused (E5) with the scope unchanged and the overlays closed;
    /// `ctrl-f` opens the find overlay. The asking tab sees none of it.
    #[test]
    fn on_the_asking_tab_an_offered_global_opens_and_a_pick_is_refused() {
        let (mut app, benches, log) = unfocused_two();
        app.overlay_factories
            .register(Switcher::ID, || Box::new(Switcher));
        app.offer(
            Act::Workspaces,
            Action::Overlay(OverlayAction::Open(Switcher::ID)),
        );
        app.overlay_factories
            .register(OverlayId("shade"), || Box::new(Shade));
        app.offer(
            Act::Find,
            Action::Overlay(OverlayAction::Open(OverlayId("shade"))),
        );
        let scope = app.scope.clone();
        let name = app.top_bar.workspace.clone();

        app.on_key(plain('w'));
        assert_eq!(top(&app), Some(Switcher::ID), "`w` opens the switcher");
        assert_eq!(app.status, None);
        app.on_key(press(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.status.as_deref(), Some(STEER_REFUSAL));
        assert!(app.overlays.is_empty(), "the switcher closed");
        assert_eq!(app.scope, scope, "the scope stays");
        assert_eq!(app.top_bar.workspace, name);
        assert!(app.editor_open());
        assert_eq!(app.tabs.active_id(), Some(ASKER));

        app.status = None;
        app.on_key(ctrl('f'));
        assert_eq!(top(&app), Some(OverlayId("shade")), "`Ctrl+f` opens find");
        assert_eq!(app.status, None);
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the asker saw nothing"
        );
        assert!(writes(&log).is_empty());
    }

    /// MOD-57 M2 E3: on another tab its keys reach it, as if no editor were alive.
    #[test]
    fn on_another_tab_keys_reach_that_tab() {
        let (mut app, benches, log) = unfocused_two();
        app.on_key(next_tab());
        for key in [
            plain('j'),
            plain('x'),
            press(KeyCode::Esc, KeyModifiers::NONE),
        ] {
            app.on_key(key);
            assert_eq!(app.status, None, "{key:?} is not refused");
        }
        assert_eq!(
            codes(&benches[1]),
            vec![KeyCode::Char('j'), KeyCode::Char('x'), KeyCode::Esc]
        );
        assert!(benches[0].seen.borrow().keys.is_empty());
        assert!(writes(&log).is_empty());
        assert_eq!(app.tabs.active_id(), Some(OTHER));
    }

    /// MOD-57 M2 E1: `editor.focus` acts from another tab (ahead of the view, which never sees
    /// it), brings the asking tab back focused (B16) and closes the `?` box.
    #[test]
    fn the_focus_key_from_another_tab_brings_the_editor_back_focused() {
        let (mut app, benches, log) = unfocused_two();
        app.on_key(next_tab());
        app.on_key(plain('?'));
        assert!(app.help_visible);

        app.on_key(ctrl('4'));
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        assert_eq!(app.editor_status().as_deref(), Some(FOCUSED_STATUS));
        assert!(!app.help_visible, "the box closes");
        assert!(writes(&log).is_empty(), "the toggle is never written");
        assert_eq!(codes(&benches[1]), vec![KeyCode::Char('?')], "no `Ctrl+4`");
        app.on_key(plain('a'));
        assert_eq!(writes(&log), vec![b"a".to_vec()], "the editor has the keys");
    }

    /// MOD-57 M2 E2: `editor.abort` is the asking tab's only; elsewhere its chord is the view's.
    #[test]
    fn abort_acts_only_on_the_asking_tab() {
        let (mut app, benches, _log) = unfocused_two();
        app.on_key(next_tab());
        app.on_key(ctrl('x'));
        assert!(app.editor_open(), "not on another tab");
        assert_eq!(
            benches[1].seen.borrow().keys.last().map(|key| key.code),
            Some(KeyCode::Char('x')),
            "the view had it"
        );
        assert!(benches[0].seen.borrow().heard.is_empty());

        app.on_key(plain('1'));
        app.on_key(ctrl('x'));
        assert!(!app.editor_open(), "on the asking tab it aborts");
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())]
        );
        assert!(benches[1].seen.borrow().heard.is_empty());
    }

    /// M2 R2 LOW-2: only the abort question may emit `AbortEdit`. A tab that emits it (here the
    /// other tab, whose keys are its own) kills nothing.
    #[test]
    fn a_tab_emitting_abort_edit_kills_nothing() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_key(next_tab());
        app.on_key(plain('!'));
        assert_eq!(
            codes(&benches[1]),
            vec![KeyCode::Char('!')],
            "the view had it"
        );
        assert!(app.editor_open(), "a tab cannot abort the edit");
        assert!(log.borrow().is_empty(), "no kill");
        assert!(benches.iter().all(|b| b.seen.borrow().heard.is_empty()));
    }

    /// MOD-57 M2 (PD-8 across tabs): another tab may ask for the editor; it is answered
    /// `EDITOR_BUSY` and the open edit is untouched.
    #[test]
    fn a_second_edit_from_another_tab_is_answered_editor_busy() {
        let (mut app, benches, _log) = unfocused_two();
        app.on_key(next_tab());
        app.on_key(plain('e'));
        assert!(app.take_external_edit().is_none());
        assert_eq!(
            benches[1].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_BUSY.to_owned())]
        );
        assert!(benches[0].seen.borrow().heard.is_empty());
        assert!(app.editor_open());
    }

    /// MOD-57 M2: a paste on another tab is that tab's; on the asking tab, unfocused, dropped.
    #[test]
    fn on_another_tab_a_paste_reaches_that_tab() {
        let (mut app, benches, log) = unfocused_two();
        app.on_paste(&Zeroizing::new("d".to_owned()));
        assert!(writes(&log).is_empty());
        assert!(
            benches[0].seen.borrow().pastes.is_empty(),
            "the asker: dropped"
        );

        app.on_key(next_tab());
        app.on_paste(&Zeroizing::new("x".to_owned()));
        assert_eq!(benches[1].seen.borrow().pastes, vec!["x".to_owned()]);
        assert!(writes(&log).is_empty());
        assert!(benches[0].seen.borrow().pastes.is_empty());
    }

    /// A workspace to enter (as `update.rs`'s test helper builds one).
    fn workspace(name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: ids::WORKSPACE_PLATFORM,
            slug: name.to_lowercase(),
            name: name.to_owned(),
            projects: vec![ProjectRef {
                project_id: ids::PROJECT_HTUI,
                slug: "p".to_owned(),
                name: "P".to_owned(),
                position: 0,
            }],
        }
    }

    /// MOD-57 M2 E5 (B-M2-6): every scope change is refused while an editor is alive, from any
    /// tab (`set_scope` resets them all, the asking one included); after the exit it applies.
    #[test]
    fn a_scope_change_is_refused_while_an_editor_is_alive() {
        let (mut app, _benches, _log) = unfocused_two();
        let scope = app.scope.clone();
        let name = app.top_bar.workspace.clone();
        for other in [false, true] {
            if other {
                app.on_key(next_tab());
            }
            app.update(Action::SetScope {
                workspace: workspace("Platform"),
            });
            assert_eq!(app.status.as_deref(), Some(STEER_REFUSAL), "other {other}");
            assert_eq!(app.scope, scope, "other {other}: the scope stays");
            assert_eq!(app.top_bar.workspace, name);
            assert!(app.editor_open());
        }

        exited(&mut app, 0, Duration::from_secs(2));
        app.status = None;
        app.update(Action::SetScope {
            workspace: workspace("Platform"),
        });
        assert_eq!(app.status, None);
        assert_eq!(app.scope.workspace_id, ids::WORKSPACE_PLATFORM);
        assert_eq!(app.top_bar.workspace, "Platform");
    }

    /// MOD-57 M2 E5: a reveal into the asking tab is refused (it would re-select what it waits
    /// on); one into another tab passes.
    #[test]
    fn a_reveal_into_the_asking_tab_is_refused_and_one_elsewhere_passes() {
        let (mut app, _benches, _log) = unfocused_two();
        app.reveal_tabs = vec![(RevealKind::Item, ASKER), (RevealKind::Requirement, OTHER)];
        app.on_key(next_tab());

        app.update(Action::Reveal(RevealTarget::Item {
            id: ItemId::new(),
            key: "FEAT-1".to_owned(),
        }));
        assert_eq!(app.status.as_deref(), Some(STEER_REFUSAL));
        assert_eq!(
            app.tabs.active_id(),
            Some(OTHER),
            "the asking tab is not focused"
        );

        app.on_key(plain('1'));
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        app.update(Action::Reveal(RevealTarget::Requirement {
            id: RequirementId::new(),
            key: "R-STO-1".to_owned(),
        }));
        assert_eq!(
            app.tabs.active_id(),
            Some(OTHER),
            "another tab's reveal passes"
        );
        assert_eq!(app.status, None);
        assert!(app.editor_open());
        // A plain tab switch is not a steer (E3).
        app.update(Action::Tab(TabAction::Focus(ASKER)));
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        assert_eq!(app.status, None);
    }

    /// MOD-57 M2 E6: another tab's own status line, led by the open edit and the way back; the
    /// way back follows the keys and is dropped under a modal overlay; an error replaces it all.
    #[test]
    fn the_status_line_on_another_tab_carries_the_edit_and_the_way_back() {
        let (mut app, _benches, _log) = unfocused_two();
        app.on_key(next_tab());
        let term = draw(&mut app, 100, 30);
        let buffer = term.backend().buffer();
        assert!(
            flash_row(buffer).ends_with(NOTICE.trim_end_matches(" \u{b7} ")),
            "{}",
            flash_row(buffer)
        );
        insta::assert_snapshot!("another_tab_while_editing", text(buffer));

        app.keys = Keys::defaults().with_chords(Context::Editor, Act::EditorFocus, &["f12"]);
        let term = draw(&mut app, 100, 30);
        assert!(flash_row(term.backend().buffer()).ends_with("editing in asker \u{b7} F12 back"));

        app.status = Some("boom".to_owned());
        let term = draw(&mut app, 100, 30);
        let row = flash_row(term.backend().buffer());
        assert!(row.ends_with("boom") && !row.contains("editing"), "{row}");

        app.status = None;
        app.push_overlay(Box::new(Shade));
        let term = draw(&mut app, 100, 30);
        let row = flash_row(term.backend().buffer());
        assert!(
            row.ends_with("editing in asker") && !row.contains("back"),
            "{row}"
        );
    }

    /// MOD-57 M2 E7 (PD-M2-2): the `?` box lists the editor keys that act where the user is:
    /// `EDITOR_OPEN`'s on the asking tab (none of the asking tab's own: it refuses them),
    /// `editor.focus` first on another tab (converted or not), none under a modal overlay.
    #[test]
    fn the_help_box_lists_the_keys_that_act_here() {
        let (mut app, _benches) = shell_with(&[
            (ASKER, false, None),
            (OTHER, false, Some(views::SETTINGS_TAB)),
        ]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        app.on_key(ctrl('4'));
        app.on_key(plain('?'));
        let term = draw(&mut app, 100, 30);
        let frame = text(term.backend().buffer());
        assert!(
            frame.contains("Editor: Ctrl+4 editor focus \u{b7} Ctrl+x abort edit"),
            "{frame}"
        );
        assert!(frame.contains("Global: q/Ctrl+c quit"), "{frame}");
        assert!(frame.contains("?/F1 closes this box"), "{frame}");
        insta::assert_snapshot!("the_help_box_on_the_asking_tab", frame);

        // Another tab, a converted view: the way back first, then its own box.
        app.on_key(plain('?'));
        app.on_key(next_tab());
        app.on_key(plain('?'));
        assert!(app.help_visible);
        let term = draw(&mut app, 100, 30);
        let frame = text(term.backend().buffer());
        assert!(frame.contains("Editor: Ctrl+4 editor focus"), "{frame}");
        assert!(!frame.contains("abort edit"), "{frame}");
        let editor = frame.find("Editor:").expect("an editor line");
        let global = frame.find("Global:").expect("its own box");
        assert!(editor < global, "the way back first: {frame}");

        // Under a modal prompt on the asking tab: its keys, no editor line.
        let (mut app, _benches, _log) = unfocused_two();
        app.push_overlay(Box::new(Prompt(Rc::default())));
        app.help_visible = true;
        let term = draw(&mut app, 100, 30);
        let frame = text(term.backend().buffer());
        assert!(frame.contains("Global:"), "{frame}");
        assert!(!frame.contains("Editor:"), "{frame}");
    }

    /// M2 R1 M-2: `editor.abort` asks first. nano's habitual `ctrl-x`, `y` (the asking tab
    /// unfocused after a tab switch) keeps the edit: `y` is not the abort key, so it closes the
    /// question, and neither key reaches the asking view or the editor.
    #[test]
    fn abort_asks_first_and_the_nano_sequence_keeps_the_edit() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        assert_eq!(top(&app), Some(AbortConfirm::ID), "asked");
        assert!(app.editor_open());
        assert!(benches[0].seen.borrow().heard.is_empty());

        app.on_key(plain('y'));
        assert!(app.overlays.is_empty(), "closed");
        assert!(app.editor_open(), "the edit stays");
        assert!(!focused(&app), "as it was");
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        assert!(benches[0].seen.borrow().heard.is_empty());
        assert!(benches.iter().all(|b| b.seen.borrow().keys.is_empty()));
        assert!(writes(&log).is_empty());
        assert_eq!(app.status, None, "`y` is not refused either");
    }

    /// M2 R1 M-2: the abort key again aborts: the asking tab hears `Failed(EDITOR_ABORTED)` and
    /// the question is gone.
    #[test]
    fn abort_again_aborts() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        app.on_key(ctrl('x'));
        assert!(!app.editor_open());
        assert!(app.overlays.is_empty());
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())]
        );
        assert!(benches[1].seen.borrow().heard.is_empty());
        assert_eq!(*log.borrow(), vec![Call::Kill, Call::Dropped]);
    }

    /// M2 R2 LOW-1: only a press of the abort key confirms. Holding it past the repeat delay sends
    /// `Repeat`s of the press that opened the question; each is consumed and the question stays.
    #[test]
    fn a_held_abort_key_does_not_confirm_the_question() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_terminal_event(Event::Key(ctrl('x')));
        assert_eq!(top(&app), Some(AbortConfirm::ID), "asked");

        let held = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::CONTROL,
            KeyEventKind::Repeat,
        );
        app.on_terminal_event(Event::Key(held));
        app.on_terminal_event(Event::Key(held));
        assert_eq!(top(&app), Some(AbortConfirm::ID), "a repeat leaves it up");
        assert!(app.editor_open(), "a repeat aborts nothing");
        assert!(benches[0].seen.borrow().heard.is_empty());
        assert!(log.borrow().is_empty(), "no kill");

        app.on_terminal_event(Event::Key(ctrl('x')));
        assert!(!app.editor_open(), "a press aborts");
        assert!(app.overlays.is_empty());
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())]
        );
    }

    /// M2 R1 M-2: every other key closes the question and keeps the edit, unfocused, and is
    /// consumed: `Esc`, `?` (no box opens), `q` (no leave dialog), `ctrl-4` (no refocus).
    #[test]
    fn every_other_key_under_the_abort_question_keeps_the_edit() {
        for key in [
            plain('y'),
            press(KeyCode::Esc, KeyModifiers::NONE),
            plain('?'),
            plain('q'),
            ctrl('4'),
            press(KeyCode::Enter, KeyModifiers::NONE),
            press(KeyCode::Tab, KeyModifiers::NONE),
        ] {
            let (mut app, benches, log) = unfocused_two();
            abortable(&mut app);
            confirmable(&mut app);
            app.on_key(ctrl('x'));
            app.on_key(key);
            assert!(app.overlays.is_empty(), "{key:?}: closed, nothing opened");
            assert!(app.editor_open(), "{key:?}");
            assert!(!focused(&app), "{key:?}");
            assert!(!app.help_visible, "{key:?}");
            assert!(!app.should_quit, "{key:?}");
            assert_eq!(app.tabs.active_id(), Some(ASKER), "{key:?}");
            assert!(benches.iter().all(|b| b.seen.borrow().keys.is_empty()));
            assert!(writes(&log).is_empty(), "{key:?}");
        }
    }

    /// M2 R2 LOW-3: a tab move under the abort question (a reply's focus; keys cannot move it)
    /// takes the question along, so `ctrl-x` on the other tab is that tab's (E2); the edit stays.
    #[test]
    fn a_tab_move_under_the_abort_question_closes_it() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        assert_eq!(top(&app), Some(AbortConfirm::ID), "asked");

        app.update(Action::Tab(TabAction::Focus(OTHER)));
        assert_eq!(app.tabs.active_id(), Some(OTHER));
        assert!(app.overlays.is_empty(), "the question went with the move");
        assert!(app.editor_open(), "the edit stays");
        assert!(log.borrow().is_empty(), "no kill");

        app.on_key(ctrl('x'));
        assert!(
            app.editor_open(),
            "on the other tab the chord is the view's"
        );
        assert_eq!(codes(&benches[1]), vec![KeyCode::Char('x')]);
        assert!(benches.iter().all(|b| b.seen.borrow().heard.is_empty()));
    }

    /// M2 R1 M-2, MOD-115 M2 B4: `ctrl-c` under the abort question opens the leave dialog over
    /// it (the abort question's quit layer), and a second one leaves.
    #[test]
    fn ctrl_c_under_the_abort_question_opens_the_dialog_and_a_second_leaves() {
        let (mut app, _benches, _log) = unfocused_two();
        abortable(&mut app);
        confirmable(&mut app);
        app.on_key(ctrl('x'));
        assert_eq!(top(&app), Some(AbortConfirm::ID));
        app.on_key(ctrl('c'));
        assert!(!app.should_quit);
        assert_eq!(top(&app), Some(LeaveDialog::ID));
        app.on_key(ctrl('c'));
        assert!(app.should_quit);
        assert!(
            app.editor_open(),
            "leaving does not abort: App drops the editor"
        );
    }

    /// M2 R1 M-2: an editor that exits under the abort question takes it along (the edit it
    /// asked about has ended); the asking tab hears the exit, not an abort.
    #[test]
    fn an_editor_that_exits_under_the_abort_question_closes_it() {
        let (mut app, benches, _log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        exited(&mut app, 0, Duration::from_secs(2));
        assert!(!app.editor_open());
        assert!(app.overlays.is_empty(), "the question went with the edit");
        let heard = benches[0].seen.borrow().heard.clone();
        assert_eq!(heard.len(), 1);
        assert_ne!(
            heard[0],
            ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())
        );
    }

    /// M2 R2 test gap: the question is taken from under another overlay too (`OverlayStack::remove`
    /// off the top): the overlay above it stays on top, and the asking tab hears the exit.
    #[test]
    fn an_editor_that_exits_under_a_covered_abort_question_closes_only_it() {
        let (mut app, benches, _log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        app.push_overlay(Box::new(Shade));
        assert_eq!(top(&app), Some(OverlayId("shade")));

        exited(&mut app, 0, Duration::from_secs(2));
        assert!(!app.editor_open());
        assert_eq!(
            top(&app),
            Some(OverlayId("shade")),
            "the overlay above stays"
        );
        assert_eq!(app.overlays.iter().count(), 1, "the question is gone");
        let heard = benches[0].seen.borrow().heard.clone();
        assert_eq!(heard.len(), 1);
        assert_ne!(
            heard[0],
            ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())
        );
    }

    /// M2 R2 test gap: a paste under the abort question is dropped: it reaches neither the editor
    /// nor any tab, and the question stays.
    #[test]
    fn a_paste_under_the_abort_question_is_dropped() {
        let (mut app, benches, log) = unfocused_two();
        abortable(&mut app);
        app.on_key(ctrl('x'));
        app.on_paste(&Zeroizing::new("p".to_owned()));
        assert_eq!(top(&app), Some(AbortConfirm::ID), "the question stays");
        assert!(app.editor_open());
        assert!(writes(&log).is_empty(), "not the editor's");
        assert!(benches.iter().all(|b| b.seen.borrow().pastes.is_empty()));
    }

    /// M2 R1 M-2: the question is drawn over the asking tab, its hint generated from the keys;
    /// the `?` box closes when it opens, and the status line says to close it first.
    #[test]
    fn the_abort_question_is_drawn_with_the_abort_key() {
        let (mut app, _benches, _log) = unfocused_two();
        abortable(&mut app);
        app.on_key(plain('?'));
        assert!(app.help_visible);
        app.on_key(ctrl('x'));
        assert!(!app.help_visible, "the box closes");
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));
        let term = draw(&mut app, 100, 30);
        let frame = text(term.backend().buffer());
        assert!(frame.contains("Abort the edit and lose it?"), "{frame}");
        assert!(
            frame.contains("Ctrl+x again aborts \u{b7} any other key keeps editing"),
            "{frame}"
        );
        insta::assert_snapshot!("the_abort_question", frame);

        app.on_key(plain('n'));
        app.keys = Keys::defaults().with_chords(Context::Editor, Act::EditorAbort, &["f9"]);
        app.on_key(press(KeyCode::F(9), KeyModifiers::NONE));
        let frame = text(draw(&mut app, 100, 30).backend().buffer());
        assert!(frame.contains("F9 again aborts"), "{frame}");
        app.on_key(press(KeyCode::F(9), KeyModifiers::NONE));
        assert!(!app.editor_open(), "the rebound key aborts");
    }

    #[test]
    fn abort_answers_failed_to_the_asking_tab_only() {
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        let id = app.editor_id().expect("open");
        let file = app.editor_file().expect("a temp file").to_path_buf();
        assert!(file.exists());

        app.on_key(ctrl('4'));
        app.on_key(ctrl('x'));
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned())]
        );
        assert!(benches[1].seen.borrow().heard.is_empty());
        assert!(!app.editor_open());
        assert_eq!(*log.borrow(), vec![Call::Kill, Call::Dropped]);
        assert!(!file.exists(), "the temp file is removed");

        app.dirty = false;
        app.on_pane_event(PaneEvent::Output {
            id,
            bytes: b"late".to_vec(),
        });
        app.on_pane_event(PaneEvent::Exited {
            id,
            status: Ok(ExitStatus::with_exit_code(0)),
            elapsed: Duration::from_secs(1),
        });
        assert_eq!(
            benches[0].seen.borrow().heard.len(),
            1,
            "a late event is inert"
        );
        assert!(!app.dirty);
    }

    #[test]
    fn exited_reads_the_file_back_through_finish_external_edit() {
        let (mut app, benches, _log) = opened();
        let file = app.editor_file().expect("a temp file").to_path_buf();
        std::fs::write(&file, "new\n").expect("the editor writes");
        exited(&mut app, 0, Duration::from_secs(2));
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Edited("new\n".to_owned())]
        );
        assert!(!file.exists());
        assert!(!app.editor_open());

        let (mut app, benches, _log) = opened();
        exited(&mut app, 3, Duration::from_secs(2));
        let heard = &benches[0].seen.borrow().heard;
        assert!(
            matches!(&heard[..], [ExternalEditOutcome::Failed(why)]
                if why.contains("exited with 3")),
            "{heard:?}"
        );
    }

    /// R1 L-1: the editor ran (the spawn succeeded), and waiting on it failed: the pane's own
    /// sentence, not the suspend mode's "could not start … set $VISUAL or $EDITOR".
    #[test]
    fn a_failed_wait_says_htui_lost_track_and_removes_the_file() {
        let (mut app, benches, _log) = opened();
        let file = app.editor_file().expect("a temp file").to_path_buf();
        std::fs::write(&file, "new\n").expect("the editor writes");
        let id = app.editor_id().expect("open");
        app.on_pane_event(PaneEvent::Exited {
            id,
            status: Err(io::Error::other("waitid failed")),
            elapsed: Duration::from_secs(2),
        });
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(
                "lost track of `nvim` (waitid failed); nothing was changed".to_owned()
            )]
        );
        assert!(!file.exists(), "the temp file is removed");
        assert!(!app.editor_open());
    }

    #[test]
    fn output_reaches_the_screen_and_replies_go_back() {
        let (mut app, _benches, log) = opened();
        app.dirty = false;
        output(&mut app, b"\x1b[6n");
        assert!(app.dirty);
        assert_eq!(writes(&log), vec![b"\x1b[1;1R".to_vec()]);

        output(&mut app, b"\x1b[?1h");
        app.on_key(press(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(writes(&log).last(), Some(&b"\x1bOA".to_vec()));
    }

    /// M2 R1 L-3: output while the editor's tab is not on screen still reaches the screen (and
    /// its replies go back) but asks for no frame; the way back draws it.
    #[test]
    fn off_screen_output_feeds_the_screen_without_a_frame() {
        let (mut app, _benches, log) = unfocused_two();
        app.on_key(next_tab());
        app.dirty = false;
        output(&mut app, b"hidden-text\x1b[6n");
        assert!(!app.dirty, "nothing on screen changed");
        assert_eq!(
            writes(&log),
            vec![b"\x1b[1;12R".to_vec()],
            "the reply goes back"
        );

        app.on_key(plain('1'));
        assert!(app.dirty);
        let term = draw(&mut app, 100, 30);
        assert!(text(term.backend().buffer()).contains("hidden-text"));
    }

    #[test]
    fn a_paste_is_forwarded_only_while_focused() {
        let (mut app, benches, log) = opened();
        app.on_paste(&Zeroizing::new("a\nb".to_owned()));
        assert_eq!(writes(&log), vec![b"a\rb".to_vec()]);

        output(&mut app, b"\x1b[?2004h");
        app.on_paste(&Zeroizing::new("c".to_owned()));
        assert_eq!(writes(&log).last(), Some(&b"\x1b[200~c\x1b[201~".to_vec()));

        app.on_key(ctrl('4'));
        let before = writes(&log).len();
        app.on_paste(&Zeroizing::new("d".to_owned()));
        assert_eq!(writes(&log).len(), before, "unfocused: dropped");
        assert!(
            benches[0].seen.borrow().pastes.is_empty(),
            "never the tab's"
        );
    }

    #[test]
    fn a_second_edit_is_answered_editor_busy() {
        let (mut app, benches, _log) = opened();
        app.pending_edit = Some((ASKER, asked()));
        assert!(app.take_external_edit().is_none());
        assert_eq!(
            benches[0].seen.borrow().heard,
            vec![ExternalEditOutcome::Failed(EDITOR_BUSY.to_owned())]
        );
        assert!(app.editor_open(), "the open editor stays");
    }

    #[test]
    fn open_editor_spawns_the_pty_command_at_the_drawn_size() {
        let (mut app, benches) = shell(&[ASKER]);
        *benches[0].claim.borrow_mut() = Some(Rect::new(20, 6, 60, 12));
        draw(&mut app, 100, 30);
        let log = Log::default();
        let spawned = open(&mut app, ASKER, &log);
        let file = app.editor_file().expect("a temp file").to_path_buf();
        let (argv_seen, size) = spawned.borrow()[0].clone();
        assert_eq!(argv_seen, argv(&cmd("nvim").pty_command(&file)));
        if cfg!(unix) {
            assert_eq!(argv_seen.last(), Some(&file.to_string_lossy().into_owned()));
        }
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), "body\n");
        assert_eq!(size, PaneSize::new(11, 60));

        // Unclaimed: the body (100x29 under the one-row header, MOD-115).
        let (mut app, _benches) = shell(&[ASKER]);
        draw(&mut app, 100, 30);
        let spawned = open(&mut app, ASKER, &log);
        assert_eq!(spawned.borrow()[0].1, PaneSize::new(28, 100));

        // Never drawn.
        let (mut app, _benches) = shell(&[ASKER]);
        let spawned = open(&mut app, ASKER, &log);
        assert_eq!(spawned.borrow()[0].1, PaneSize::DEFAULT);
    }

    #[test]
    fn a_spawn_failure_answers_the_tab_and_opens_nothing() {
        let (mut app, benches) = shell(&[ASKER]);
        let file: Rc<RefCell<Option<PathBuf>>> = Rc::default();
        let seen_file = Rc::clone(&file);
        app.open_editor(ASKER, &asked(), cmd("nvim"), move |_id, command, _size| {
            *seen_file.borrow_mut() = argv(&command).last().map(PathBuf::from);
            Err(io::Error::other("no pty"))
        });
        let heard = &benches[0].seen.borrow().heard;
        assert!(
            matches!(&heard[..], [ExternalEditOutcome::Failed(why)]
                if why.contains("no pty") && why.contains(PANE_VAR)),
            "{heard:?}"
        );
        assert!(!app.editor_open());
        let file = file.borrow().clone().expect("the spawn saw the argv");
        assert!(!file.exists(), "the temp file is removed");
    }

    #[test]
    fn the_pane_draws_over_the_claimed_rect() {
        let (mut app, benches) = shell(&[ASKER]);
        *benches[0].claim.borrow_mut() = Some(Rect::new(20, 6, 60, 12));
        draw(&mut app, 100, 30);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        output(&mut app, b"hello from the pane\r\n~\r\n~");
        let term = draw(&mut app, 100, 30);
        let buffer = term.backend().buffer();
        assert!(row_from(buffer, 20, 6).starts_with(FOCUSED_TITLE));
        assert!(row_from(buffer, 20, 7).starts_with("hello from the pane"));
        insta::assert_snapshot!("the_pane_over_the_claimed_rect", text(buffer));
    }

    #[test]
    fn the_pane_falls_back_to_the_body_when_unclaimed_or_small() {
        for claim in [
            None,
            Some(Rect::new(10, 5, 30, 5)),
            Some(Rect::new(10, 40, 60, 12)),
        ] {
            let (mut app, benches) = shell(&[ASKER]);
            *benches[0].claim.borrow_mut() = claim;
            let log = Log::default();
            open(&mut app, ASKER, &log);
            let term = draw(&mut app, 100, 30);
            assert!(
                row_from(term.backend().buffer(), 0, 1).starts_with(FOCUSED_TITLE),
                "{claim:?}: the rule is on the body's first row"
            );
        }
        // `MIN_PANE` itself is honoured.
        let (mut app, benches) = shell(&[ASKER]);
        let exact = Rect::new(10, 5, MIN_PANE.width, MIN_PANE.height);
        *benches[0].claim.borrow_mut() = Some(exact);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 10, 5).starts_with(FOCUSED_TITLE));
    }

    #[test]
    fn the_pane_is_drawn_only_in_its_tab_and_refocus_brings_it_back() {
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(app.tabs.focus(OTHER));
        let mut term = draw(&mut app, 100, 30);
        assert!(
            !text(term.backend().buffer()).contains(" nvim · "),
            "no pane"
        );
        assert!(!term.backend().cursor_visible(), "no cursor");
        // Still focused, but off screen: the other tab's own line, led by the open edit (MOD-57
        // M2 E6); not the way back, since the next key is swallowed (M2 R1 L-2).
        assert_eq!(app.editor_status(), None);
        let status = flash_row(term.backend().buffer());
        assert!(status.ends_with("editing in asker"), "{status}");
        assert!(!status.contains("back"), "{status}");

        app.on_key(plain('a'));
        assert!(
            writes(&log).is_empty(),
            "the editor is not on screen: htui has the keys"
        );
        assert_eq!(app.status.as_deref(), Some(OFF_SCREEN));
        assert!(benches[1].seen.borrow().keys.is_empty(), "swallowed");

        app.on_key(ctrl('4'));
        assert_eq!(
            app.tabs.active_id(),
            Some(ASKER),
            "refocus brings its tab back"
        );
        assert_eq!(app.editor_status().as_deref(), Some(FOCUSED_STATUS));
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(text(term.backend().buffer()).contains(FOCUSED_TITLE));
    }

    #[test]
    fn the_key_that_finds_a_focused_editor_off_screen_is_swallowed() {
        for key in [ctrl('c'), plain('q'), ctrl('x'), ctrl('4')] {
            let (mut app, benches) = shell(&[ASKER, OTHER]);
            let log = Log::default();
            open(&mut app, ASKER, &log);
            // A reply moved the active tab while the user typed into the editor.
            assert!(app.tabs.focus(OTHER));
            // M2 R1 L-2: the next key is swallowed, so the way back is not offered yet.
            assert_eq!(
                app.editor_notice().as_deref(),
                Some("editing in asker \u{b7} "),
                "{key:?}"
            );

            app.on_key(key);
            assert_eq!(app.editor_notice().as_deref(), Some(NOTICE), "{key:?}");
            assert!(!app.should_quit, "{key:?} meant for the editor never quits");
            assert!(app.editor_open(), "{key:?} never aborts");
            assert!(
                writes(&log).is_empty(),
                "{key:?} is not the editor's either"
            );
            assert_eq!(app.status.as_deref(), Some(OFF_SCREEN), "{key:?}");
            assert_eq!(app.tabs.active_id(), Some(OTHER), "{key:?} moves nothing");
            assert!(benches[1].seen.borrow().keys.is_empty());
            assert_eq!(app.editor_status(), None, "off screen: the tab's own line");

            // From here on htui has the keys, and the other tab is free (MOD-57 M2 E3).
            app.on_key(plain('j'));
            assert_eq!(app.status, None, "{key:?}: the next key is not refused");
            assert_eq!(codes(&benches[1]), vec![KeyCode::Char('j')], "{key:?}");
            assert!(writes(&log).is_empty());
            app.on_key(ctrl('c'));
            assert!(app.should_quit, "the next ctrl-c is htui's");
        }
    }

    #[test]
    fn a_modal_overlay_over_a_focused_editor_takes_the_keys_after_one_is_swallowed() {
        // T5 verify round 3: a reply can open a modal prompt (the migration y/n) over a focused
        // editor. The key that finds it was typed for the editor and must not answer the prompt;
        // after that the prompt has the keys, and the asking tab never does.
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        let heard = Rc::new(RefCell::new(Vec::new()));
        app.push_overlay(Box::new(Prompt(Rc::clone(&heard))));

        app.on_key(plain('y'));
        assert!(writes(&log).is_empty(), "not the editor's");
        assert!(heard.borrow().is_empty(), "not the prompt's either");
        // R0 F3: no editor key is offered while the prompt has the keys.
        assert_eq!(app.status.as_deref(), Some(COVERED));
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));

        app.on_key(plain('n'));
        assert_eq!(heard.borrow().len(), 1, "the prompt has the keys now");
        assert!(writes(&log).is_empty());
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the asking tab never does"
        );
        assert!(app.editor_open());

        // ctrl-c is the overlay stack's quit (MOD-115 M2 B4): a bare shell, with no leave dialog,
        // quits at once.
        let mut quit = app;
        quit.on_key(ctrl('c'));
        assert!(quit.should_quit);
    }

    #[test]
    fn a_paste_that_finds_a_covered_focused_editor_is_swallowed_then_the_overlay_has_it() {
        // R0 F2: a paste follows a key's rule. The first that finds a focused editor under a
        // modal overlay is swallowed and unfocuses; after that the overlay's path has it.
        let (mut app, benches, log) = opened();
        let heard = Rc::new(RefCell::new(Vec::new()));
        app.push_overlay(Box::new(Pasted(Rc::clone(&heard))));

        app.on_paste(&Zeroizing::new("y".to_owned()));
        assert!(writes(&log).is_empty(), "not the editor's");
        assert!(heard.borrow().is_empty(), "not the prompt's either");
        assert_eq!(app.status.as_deref(), Some(COVERED));
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));

        app.on_paste(&Zeroizing::new("n".to_owned()));
        assert_eq!(*heard.borrow(), vec!["n".to_owned()], "the prompt's now");
        assert!(writes(&log).is_empty());
        assert!(
            benches[0].seen.borrow().pastes.is_empty(),
            "the asking tab never has it"
        );
    }

    #[test]
    fn a_paste_that_finds_a_focused_editor_off_screen_is_swallowed() {
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(app.tabs.focus(OTHER));

        app.on_paste(&Zeroizing::new("x".to_owned()));
        assert!(writes(&log).is_empty());
        assert_eq!(app.status.as_deref(), Some(OFF_SCREEN));
        assert!(
            benches
                .iter()
                .all(|bench| bench.seen.borrow().pastes.is_empty())
        );
        // The paste unfocused it: htui has the keys, the other tab the next paste (MOD-57 M2),
        // and the next `ctrl-c` is htui's.
        app.on_paste(&Zeroizing::new("y".to_owned()));
        assert_eq!(benches[1].seen.borrow().pastes, vec!["y".to_owned()]);
        assert!(writes(&log).is_empty());
        app.on_key(ctrl('c'));
        assert!(app.should_quit);
    }

    #[test]
    fn under_a_modal_overlay_the_title_and_status_agree_and_offer_no_editor_key() {
        // R0 F3: the title reads from the same predicate as the status line, and neither offers
        // the editor's keys while the prompt has them.
        let (mut app, _benches, log) = opened();
        let heard = Rc::new(RefCell::new(Vec::new()));
        app.push_overlay(Box::new(Prompt(Rc::clone(&heard))));
        let term = draw(&mut app, 100, 30);
        let buffer = term.backend().buffer();
        assert!(row_from(buffer, 0, 1).starts_with(UNFOCUSED_TITLE));
        assert!(flash_row(buffer).ends_with(COVERED));
        assert!(!term.backend().cursor_visible());

        app.on_key(ctrl('4'));
        assert_eq!(app.status.as_deref(), Some(COVERED), "swallowed");
        app.on_key(ctrl('4'));
        assert_eq!(
            heard.borrow().len(),
            1,
            "the prompt's: Ctrl+4 does not work here"
        );
        assert_eq!(app.editor_status().as_deref(), Some(COVERED));
        assert!(writes(&log).is_empty());

        app.on_key(KeyEvent::from(KeyCode::Esc));
        assert!(app.overlays.is_empty());
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 1).starts_with(UNFOCUSED_TITLE));
    }

    #[test]
    fn an_editor_opened_under_the_help_box_has_the_keys_and_closes_it() {
        // R0 F4: the `?` box swallows no key, so a view can ask for the editor under it. The
        // editor opens with the keys, and keys, paste, title, status and cursor agree.
        let (mut app, _benches) = shell(&[ASKER]);
        app.on_key(plain('?'));
        assert!(app.help_visible);
        app.on_key(plain('e'));
        let (tab, edit) = app
            .take_external_edit()
            .expect("the tab asked under the box");
        let log = Log::default();
        let child = Rc::clone(&log);
        app.open_editor(tab, &edit, cmd("nvim"), move |_id, _command, _size| {
            Ok(Box::new(FakeChild { log: child }) as Box<dyn PaneChild>)
        });
        assert!(!app.help_visible, "the box closes");
        let term = draw(&mut app, 100, 30);
        let buffer = term.backend().buffer();
        assert!(row_from(buffer, 0, 1).starts_with(FOCUSED_TITLE));
        assert!(flash_row(buffer).ends_with(FOCUSED_STATUS));
        assert!(term.backend().cursor_visible());

        app.on_key(plain('a'));
        app.on_paste(&Zeroizing::new("b".to_owned()));
        assert_eq!(writes(&log), vec![b"a".to_vec(), b"b".to_vec()]);
    }

    #[test]
    fn closing_the_overlay_hands_the_lock_back() {
        let (mut app, _benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        app.on_key(ctrl('4'));
        let heard = Rc::new(RefCell::new(Vec::new()));
        app.push_overlay(Box::new(Prompt(Rc::clone(&heard))));

        app.on_key(plain('y'));
        assert_eq!(
            heard.borrow().len(),
            1,
            "unfocused: the prompt answers at once"
        );
        app.on_key(KeyEvent::from(KeyCode::Esc));
        assert!(app.overlays.is_empty(), "Esc closes it");

        app.on_key(plain('a'));
        assert_eq!(app.status.as_deref(), Some(REFUSAL), "the M1 lock again");
        app.on_key(ctrl('4'));
        app.on_key(plain('a'));
        assert_eq!(writes(&log).len(), 1, "refocused: the editor's");
    }

    #[test]
    fn the_cursor_shows_only_when_focused_and_drawn() {
        let (mut app, _benches, _log) = opened();
        output(&mut app, b"ab\r\ncd");
        let mut term = draw(&mut app, 100, 30);
        assert!(term.backend().cursor_visible(), "focused: shown");
        // Body at row 1, the rule on it, the screen below: row 1 of the screen is frame row 3.
        assert_eq!(
            term.get_cursor_position().expect("a position"),
            Position::new(2, 3)
        );

        app.on_key(ctrl('4'));
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(!term.backend().cursor_visible(), "unfocused: hidden");

        app.on_key(ctrl('4'));
        app.help_visible = true;
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(!term.backend().cursor_visible(), "the `?` box: hidden");
        // R0 F4: the `?` box covers it like an overlay, for the status line too.
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));

        app.help_visible = false;
        app.push_overlay(Box::new(Shade));
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(!term.backend().cursor_visible(), "an overlay: hidden");
    }

    #[test]
    fn resize_follows_the_drawn_rect() {
        let (mut app, _benches) = shell(&[ASKER]);
        draw(&mut app, 100, 30);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        draw(&mut app, 100, 30);
        app.resize_editor();
        assert!(log.borrow().is_empty(), "same rect: nothing");

        draw(&mut app, 120, 40);
        assert!(!app.dirty);
        app.resize_editor();
        let size = PaneSize::new(38, 120);
        assert_eq!(*log.borrow(), vec![Call::Resize(size)]);
        assert_eq!(
            app.editor.as_ref().map(|editor| editor.screen.size()),
            Some(size)
        );
        assert!(app.dirty);
    }

    /// MOD-57 M2 E8 (rewrites M1's `mouse_capture_is_off_while_an_editor_is_open`): capture is
    /// off while the pane is on screen, and the view's to decide on another tab.
    #[test]
    fn mouse_capture_follows_the_pane_on_screen() {
        // The asking tab wants the mouse: off while its pane is drawn, back after the exit.
        let (mut app, _benches) = shell_with(&[(ASKER, true, None)]);
        assert!(app.mouse_capture());
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(!app.mouse_capture(), "off while the pane is on screen");
        exited(&mut app, 0, Duration::from_secs(2));
        assert!(app.mouse_capture(), "back after it exits");

        // Another tab wants it: on there while the editor waits.
        let (mut app, benches) = shell_with(&[(ASKER, false, None), (OTHER, true, None)]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(!app.mouse_capture(), "on the asking tab: off");
        app.on_key(ctrl('4'));
        app.on_key(next_tab());
        assert!(app.mouse_capture(), "on the other tab: the view's");
        assert_eq!(benches[1].lost.get(), 0);

        app.on_key(ctrl('4'));
        assert_eq!(app.tabs.active_id(), Some(ASKER));
        assert!(!app.mouse_capture(), "back on the pane: off");
        assert_eq!(benches[1].lost.get(), 1, "the other tab is told (MOD-74)");

        app.on_key(ctrl('4'));
        app.on_key(next_tab());
        assert!(app.mouse_capture());
        exited(&mut app, 0, Duration::from_secs(2));
        assert!(app.mouse_capture(), "still on after the exit");
    }

    #[test]
    fn the_title_status_and_help_come_from_the_keys() {
        let (mut app, _benches, _log) = opened();
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 1).starts_with(FOCUSED_TITLE));
        assert!(flash_row(term.backend().buffer()).ends_with(FOCUSED_STATUS));

        app.on_key(ctrl('4'));
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 1).starts_with(UNFOCUSED_TITLE));
        assert!(flash_row(term.backend().buffer()).ends_with(UNFOCUSED_STATUS));

        app.on_key(plain('?'));
        let term = draw(&mut app, 100, 30);
        assert!(
            text(term.backend().buffer())
                .contains("Editor: Ctrl+4 editor focus · Ctrl+x abort edit"),
            "the `?` box lists the editor's keys"
        );
        app.on_key(plain('?'));
        app.on_key(ctrl('x'));
        app.on_key(plain('?'));
        let term = draw(&mut app, 100, 30);
        assert!(app.help_visible);
        assert!(
            !text(term.backend().buffer()).contains("Editor:"),
            "only while open"
        );

        // Rebound: every label follows.
        let (mut app, _benches, _log) = opened();
        app.keys = Keys::defaults().with_chords(Context::Editor, Act::EditorFocus, &["f12"]);
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 1).starts_with(" nvim · F12 to htui "));
        assert_eq!(
            app.editor_status().as_deref(),
            Some("the editor has the keys · F12 to htui")
        );
        app.on_key(press(KeyCode::F(12), KeyModifiers::NONE));
        assert_eq!(
            app.editor_status().as_deref(),
            Some(
                "F12 to the editor · Ctrl+x abort · Ctrl+PgDn/Ctrl+PgUp switch tabs · q quit · ? help"
            )
        );
        app.on_key(plain('j'));
        assert_eq!(
            app.status.as_deref(),
            Some("the editor is open: F12 to the editor · Ctrl+x abort")
        );
        assert!(EDITOR_LOCKED.starts_with("the editor is open"));
    }

    #[test]
    fn an_open_editor_debug_prints_no_text() {
        let (mut app, _benches) = shell(&[ASKER]);
        let log = Log::default();
        app.open_editor(
            ASKER,
            &ExternalEdit {
                text: "SECRET handed".to_owned(),
                stem: "s".to_owned(),
            },
            cmd("nvim"),
            move |_id, _command, _size| Ok(Box::new(FakeChild { log }) as Box<dyn PaneChild>),
        );
        output(&mut app, b"SECRET drawn");
        let debug = format!("{app:?}");
        assert!(debug.contains("OpenEditor"), "{debug}");
        assert!(!debug.contains("SECRET"), "{debug}");
    }
}
