//! MOD-57 M1: the external editor in the TUI pane, as the shell sees it.
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
//!   owes, DSR and DA1) and, on `Exited`, reads the file back through [`TempEdit::finish`] and
//!   hands the outcome to the asking tab through [`App::finish_external_edit`], so no view's
//!   outcome handling changes. Every event carries a [`PaneId`]; an event of an editor already
//!   finished or aborted is dropped (B4).
//! - **Keys (P5, P6).** While an editor is alive [`App::on_key`] asks it first. Focused, only
//!   `editor.focus` ([`Stack::EDITOR_FOCUSED`]) is htui's; every other key, `ctrl-c` included,
//!   is encoded ([`encode_key`]) and written. Unfocused is the M1 lock
//!   ([`Stack::EDITOR_UNFOCUSED`]): `ctrl-c` and `global.quit` quit, `global.help` toggles the
//!   `?` box, `editor.focus` refocuses (bringing the asking tab back, B16), `editor.abort` kills
//!   the editor and answers [`EDITOR_ABORTED`]; anything else is refused on the status line. A
//!   focused editor a reply moved off screen loses the keys, and the key that finds it so (typed
//!   for the editor) is swallowed with the refusal rather than run through the lock.
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

use crate::app::action::{Action, TabAction};
use crate::app::state::App;
use crate::editor::pane::{PaneChild, PaneEvent, PaneId, PaneScreen, PaneSize};
use crate::editor::pane::{encode_key, encode_paste};
use crate::editor::{
    EDITOR_ABORTED, EDITOR_BUSY, EditorCommand, EditorExit, ExternalEdit, ExternalEditOutcome,
    PANE_VAR, TempEdit,
};
use crate::keys::{Act, CTRL_C, Hint, HintSpec, KeyChord, Stack};
use crate::ui::overlay::Overlay;
use crate::ui::tabs::TabId;

/// PRD Q3: the smallest claimed rect a pane uses; below it, the whole tab body. 40x8 until T7's
/// manual nvim run in the Notes compose box says otherwise.
pub const MIN_PANE: Size = Size {
    width: 40,
    height: 8,
};

/// The head of the refusal while an editor is alive and unfocused (MOD-57 P6).
pub const EDITOR_LOCKED: &str = "the editor is open";

/// The focused pane's title and status hint.
const FOCUSED_HINT: HintSpec = &[Hint::One(Act::EditorFocus, "to htui")];

/// The unfocused status line.
const UNFOCUSED_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
    Hint::One(Act::Quit, "quit"),
    Hint::One(Act::Help, "help"),
];

/// The refusal's tail.
const LOCK_HINT: HintSpec = &[
    Hint::One(Act::EditorFocus, "to the editor"),
    Hint::One(Act::EditorAbort, "abort"),
];

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
    /// back; `Exited` reads the file back ([`TempEdit::finish`]) and hands the outcome to the tab
    /// that asked ([`finish_external_edit`](Self::finish_external_edit)). An event of any other
    /// id is dropped (B4).
    pub fn on_pane_event(&mut self, event: PaneEvent) {
        if self.editor_id() != Some(event.id()) {
            return;
        }
        match event {
            PaneEvent::Output { bytes, .. } => {
                if let Some(editor) = self.editor.as_mut() {
                    let replies = editor.screen.feed(&bytes);
                    if !replies.is_empty() {
                        editor.child.write(&replies);
                    }
                    self.dirty = true;
                }
            }
            PaneEvent::Exited {
                status, elapsed, ..
            } => {
                // The child drops here: its kill request finds the wait thread gone (harmless).
                let Some(OpenEditor { tab, temp, cmd, .. }) = self.editor.take() else {
                    return;
                };
                let outcome = temp.finish(
                    &cmd,
                    status.map(|status| EditorExit::from(&status)),
                    elapsed,
                );
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
    fn editor_visible(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| self.tabs.active_id() == Some(editor.tab))
    }

    /// A key while an editor is alive (P5, P6): focused, the editor's unless it is the focus
    /// toggle; unfocused, the M1 lock. `false` hands the key back to [`App::on_key`]'s overlay
    /// path: only while unfocused under a modal overlay, which a reply can open over the editor
    /// (the migration prompt) and which never reaches the asking tab. Otherwise nothing reaches
    /// an overlay, a tab or a keymap.
    pub(super) fn editor_key(&mut self, key: KeyEvent, chord: KeyChord) -> bool {
        let visible = self.editor_visible();
        let covered = self.overlays.top().is_some_and(Overlay::is_modal);
        let Some(editor) = self.editor.as_mut() else {
            return false;
        };
        // A focused editor that is not on screen (a reply moved the active tab): htui has the
        // keys from now on. The key that finds it so was typed for the editor (a vim `ctrl-c`,
        // the `q` of `:wq`), so it is swallowed with the refusal, never run as a lock command:
        // it must not quit, abort or move a tab.
        // The same for a modal overlay drawn over it (T5 verify round 3): the key must not answer
        // a prompt the user had not seen yet.
        if editor.focused && (!visible || covered) {
            editor.focused = false;
            self.status = Some(format!(
                "{EDITOR_LOCKED}: {}",
                self.keys.hint(Stack::EDITOR_UNFOCUSED, LOCK_HINT)
            ));
            return true;
        }
        if editor.focused {
            if self
                .keys
                .actions(Stack::EDITOR_FOCUSED, chord)
                .contains(&Act::EditorFocus)
            {
                editor.focused = false;
                return true;
            }
            let application_cursor = editor.screen.screen().application_cursor();
            if let Some(bytes) = encode_key(key, application_cursor) {
                editor.child.write(&bytes);
            }
            return true;
        }

        // Unfocused under a modal overlay: the overlay's key, through `on_key`'s own path (C3,
        // the overlay, `Esc`/help, the modal swallow).
        if covered {
            return false;
        }
        // ANA-26 C3 holds while htui has the keys.
        if chord == CTRL_C {
            self.update(Action::Quit);
            return true;
        }
        let tab = editor.tab;
        match self
            .keys
            .actions(Stack::EDITOR_UNFOCUSED, chord)
            .first()
            .copied()
        {
            Some(Act::EditorFocus) => {
                editor.focused = true;
                self.help_visible = false;
                if !visible {
                    self.update(Action::Tab(TabAction::Focus(tab)));
                }
            }
            Some(Act::EditorAbort) => self.abort_editor(),
            // M1: no confirm; the editor is killed when `App` drops (`lib.rs`).
            Some(Act::Quit) => self.update(Action::Quit),
            Some(Act::Help) => self.update(Action::ToggleHelp),
            _ => {
                self.status = Some(format!(
                    "{EDITOR_LOCKED}: {}",
                    self.keys.hint(Stack::EDITOR_UNFOCUSED, LOCK_HINT)
                ));
            }
        }
        true
    }

    /// A paste while an editor is alive: to the editor when it is focused and on screen
    /// (bracketed if it asked), else dropped (the lock; a paste is never a key).
    pub(super) fn editor_paste(&mut self, text: &Zeroizing<String>) {
        let visible = self.editor_visible();
        if let Some(editor) = self.editor.as_mut()
            && editor.focused
            && visible
        {
            let bytes = encode_paste(text, editor.screen.screen().bracketed_paste());
            editor.child.write(&bytes);
        }
    }

    /// Records where the active tab's pane goes this frame (`editor_rect`, pane or not, so the
    /// first spawn has the view's size) and draws the editor there when its tab is the active
    /// one. The real cursor shows only when the editor has the keys and nothing is drawn over it.
    pub(super) fn draw_editor(&mut self, frame: &mut Frame<'_>, body: Rect) {
        let rect = pane_rect(self.editor_area.get(), body);
        self.editor_rect = self.tabs.active_id().map(|id| (id, rect));
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        if self.tabs.active_id() != Some(editor.tab) {
            return;
        }
        let focused = editor.focused && self.overlays.is_empty() && !self.help_visible;
        let title = if editor.focused {
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

    /// The status line while an editor is alive and no error is up; `None` without one. A focused
    /// editor whose tab is not on screen, or under a modal overlay, does not have the keys
    /// ([`editor_key`](Self::editor_key) hands them to htui), so it reads as unfocused.
    pub(super) fn editor_status(&self) -> Option<String> {
        let editor = self.editor.as_ref()?;
        let covered = self.overlays.top().is_some_and(Overlay::is_modal);
        Some(if editor.focused && self.editor_visible() && !covered {
            format!(
                "the editor has the keys \u{b7} {}",
                self.keys.hint(Stack::EDITOR_FOCUSED, FOCUSED_HINT)
            )
        } else {
            self.keys.hint(Stack::EDITOR_UNFOCUSED, UNFOCUSED_HINT)
        })
    }

    /// `editor.abort` (B8): the kill is requested and the temp file removed at once, and the
    /// asking tab is answered `Failed(EDITOR_ABORTED)`. A later event of the editor is inert.
    fn abort_editor(&mut self) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let tab = editor.tab;
        editor.child.kill();
        drop(editor);
        self.finish_external_edit(tab, ExternalEditOutcome::Failed(EDITOR_ABORTED.to_owned()));
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use htui_core::model::Scope;
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
    use crate::app::{Action, App, Ctx, Handled};
    use crate::editor::pane::{PaneChild, PaneEvent, PaneSize};
    use crate::editor::{
        EDITOR_ABORTED, EDITOR_BUSY, EditorCommand, ExternalEdit, ExternalEditOutcome, PANE_VAR,
    };
    use crate::keymap::Keymap;
    use crate::keys::{Act, Context, Keys};
    use crate::store_worker::{StoreReply, StoreRequest};
    use crate::ui::overlay::{Overlay, OverlayId};
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
    /// claims `claim` for the editor.
    struct Editing {
        id: TabId,
        seen: Shared,
        claim: Rc<RefCell<Option<Rect>>>,
        mouse: bool,
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
    /// stack closes it).
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
            if key.code == KeyCode::Esc {
                return Handled::Pass;
            }
            self.0.borrow_mut().push(key);
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
    }

    /// A shell with `ids` registered as [`Editing`] tabs, the first one active.
    fn shell(ids: &[TabId]) -> (App, Vec<Bench>) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx, Keymap::new());
        let benches = ids
            .iter()
            .map(|id| {
                let bench = Bench {
                    seen: Shared::default(),
                    claim: Rc::default(),
                };
                app.register_tab(Box::new(Editing {
                    id: *id,
                    seen: Rc::clone(&bench.seen),
                    claim: Rc::clone(&bench.claim),
                    mouse: false,
                }));
                bench
            })
            .collect();
        (app, benches)
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

    /// Row `y` of the buffer from column `x`.
    fn row_from(buffer: &Buffer, x: u16, y: u16) -> String {
        (x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    const FOCUSED_TITLE: &str = " nvim · Ctrl+4 to htui ";
    const UNFOCUSED_TITLE: &str = " nvim · htui has the keys ";
    const FOCUSED_STATUS: &str = "the editor has the keys · Ctrl+4 to htui";
    const UNFOCUSED_STATUS: &str = "Ctrl+4 to the editor · Ctrl+x abort · q quit · ? help";
    const REFUSAL: &str = "the editor is open: Ctrl+4 to the editor · Ctrl+x abort";

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

    #[test]
    fn unfocused_ctrl_c_and_q_quit_at_once() {
        for key in [ctrl('c'), plain('q')] {
            let (mut app, _benches, log) = opened();
            app.on_key(ctrl('4'));
            app.on_key(key);
            assert!(app.should_quit, "{key:?} quits");
            assert!(writes(&log).is_empty());
            drop(app);
            assert_eq!(
                log.borrow().last(),
                Some(&Call::Dropped),
                "the child goes with App"
            );
        }
    }

    #[test]
    fn unfocused_every_other_key_is_refused_and_nothing_moves() {
        let (mut app, benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        app.on_key(ctrl('4'));

        app.on_key(plain('?'));
        assert!(app.help_visible, "`?` toggles help");
        app.on_key(plain('?'));
        assert!(!app.help_visible);

        for key in [
            plain('j'),
            plain('1'),
            press(KeyCode::Tab, KeyModifiers::NONE),
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

        // Unclaimed: the body (100x27 under the top bar, the strip and the status line).
        let (mut app, _benches) = shell(&[ASKER]);
        draw(&mut app, 100, 30);
        let spawned = open(&mut app, ASKER, &log);
        assert_eq!(spawned.borrow()[0].1, PaneSize::new(26, 100));

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
                row_from(term.backend().buffer(), 0, 2).starts_with(FOCUSED_TITLE),
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
        let (mut app, _benches) = shell(&[ASKER, OTHER]);
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(app.tabs.focus(OTHER));
        let mut term = draw(&mut app, 100, 30);
        assert!(
            !text(term.backend().buffer()).contains(" nvim · "),
            "no pane"
        );
        assert!(!term.backend().cursor_visible(), "no cursor");
        // Still focused, but off screen: the status line says htui has the keys (it has).
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));
        assert!(row_from(term.backend().buffer(), 0, 29).starts_with(UNFOCUSED_STATUS));

        app.on_key(plain('a'));
        assert!(
            writes(&log).is_empty(),
            "the editor is not on screen: htui has the keys"
        );
        assert_eq!(app.status.as_deref(), Some(REFUSAL));

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

            app.on_key(key);
            assert!(!app.should_quit, "{key:?} meant for the editor never quits");
            assert!(app.editor_open(), "{key:?} never aborts");
            assert!(
                writes(&log).is_empty(),
                "{key:?} is not the editor's either"
            );
            assert_eq!(app.status.as_deref(), Some(REFUSAL), "{key:?}");
            assert_eq!(app.tabs.active_id(), Some(OTHER), "{key:?} moves nothing");
            assert!(benches[1].seen.borrow().keys.is_empty());
            assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));

            // From here on the M1 lock holds.
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
        assert_eq!(app.status.as_deref(), Some(REFUSAL));
        assert_eq!(app.editor_status().as_deref(), Some(UNFOCUSED_STATUS));

        app.on_key(plain('n'));
        assert_eq!(heard.borrow().len(), 1, "the prompt has the keys now");
        assert!(writes(&log).is_empty());
        assert!(
            benches[0].seen.borrow().keys.is_empty(),
            "the asking tab never does"
        );
        assert!(app.editor_open());

        // ctrl-c still quits at once (ANA-26 C3).
        let mut quit = app;
        quit.on_key(ctrl('c'));
        assert!(quit.should_quit);
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
        // Body at row 2, the rule on it, the screen below: row 1 of the screen is frame row 4.
        assert_eq!(
            term.get_cursor_position().expect("a position"),
            Position::new(2, 4)
        );

        app.on_key(ctrl('4'));
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(!term.backend().cursor_visible(), "unfocused: hidden");

        app.on_key(ctrl('4'));
        app.help_visible = true;
        term.draw(|frame| app.render(frame)).expect("draws");
        assert!(!term.backend().cursor_visible(), "the `?` box: hidden");

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
        let size = PaneSize::new(36, 120);
        assert_eq!(*log.borrow(), vec![Call::Resize(size)]);
        assert_eq!(
            app.editor.as_ref().map(|editor| editor.screen.size()),
            Some(size)
        );
        assert!(app.dirty);
    }

    #[test]
    fn mouse_capture_is_off_while_an_editor_is_open() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut app = App::new(tx, Keymap::new());
        app.register_tab(Box::new(Editing {
            id: ASKER,
            seen: Shared::default(),
            claim: Rc::default(),
            mouse: true,
        }));
        assert!(app.mouse_capture());
        let log = Log::default();
        open(&mut app, ASKER, &log);
        assert!(!app.mouse_capture(), "off while the editor is alive");
        exited(&mut app, 0, Duration::from_secs(2));
        assert!(app.mouse_capture(), "back after it exits");
    }

    #[test]
    fn the_title_status_and_help_come_from_the_keys() {
        let (mut app, _benches, _log) = opened();
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 2).starts_with(FOCUSED_TITLE));
        assert!(row_from(term.backend().buffer(), 0, 29).starts_with(FOCUSED_STATUS));

        app.on_key(ctrl('4'));
        let term = draw(&mut app, 100, 30);
        assert!(row_from(term.backend().buffer(), 0, 2).starts_with(UNFOCUSED_TITLE));
        assert!(row_from(term.backend().buffer(), 0, 29).starts_with(UNFOCUSED_STATUS));

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
        assert!(row_from(term.backend().buffer(), 0, 2).starts_with(" nvim · F12 to htui "));
        assert_eq!(
            app.editor_status().as_deref(),
            Some("the editor has the keys · F12 to htui")
        );
        app.on_key(press(KeyCode::F(12), KeyModifiers::NONE));
        assert_eq!(
            app.editor_status().as_deref(),
            Some("F12 to the editor · Ctrl+x abort · q quit · ? help")
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
