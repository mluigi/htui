//! The Skills view of the Skills tab (MOD-9 milestone 3, T3; plan D82, D84; blueprint D93, D101,
//! D106, D108): the library, a skill's versions, the line diff, the body editor and the save.
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle and
//! no `UserId`, renders from the last `SkillsSnapshot` and never patches a row into it. A skill
//! body is markdown, not a template, so there is no `parse` here: the name rule and the two
//! compare-and-set tokens are the whole gate, and they live in the writer
//! ([`crate::skills`]).
//!
//! The token a save carries is the head version when the editor opened, not the version shown:
//! editing v1 while v3 is head saves v4 (plan D1, OQ-5). `skill.updated_at` is the second token
//! and guards the description (D101).

use core::cell::Cell;

use chrono::{DateTime, Utc};
use htui_core::model::{Skill, SkillVersion};
use htui_core::prompt::TokenEstimator;
// `invalid_skill_name` is reached through the module rather than a `store::` re-export: unlike
// `invalid_template_name`, T1 did not add it to `store/mod.rs`'s list, and that file is not in
// T3's. The function is the same one both stores call, which is the point — the view's sentence
// and the writer's cannot drift (D100).
use htui_core::store::traits::invalid_skill_name;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::{Action, Ctx, Handled};
use crate::editor::{ExternalEdit, ExternalEditOutcome};
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillBody, SkillsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::tabs::settings::wrapped;
use crate::ui::{FieldOutcome, TextArea, TextField, Theme, diff};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The `StoreRequest::SaveSkill` name, what `busy` holds while it is in flight. The slice index,
/// not a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const SAVE_NAME: &str = REQUEST_NAMES[1];

/// How many rows the notice may wrap to before it is cut; one when it fits.
const NOTICE_LINES: usize = 2;

/// The library's width, borders included: `  {name:<26} v{head:<3} {activation:<5} {tokens}` is at
/// most 43 chars for a three-figure estimate, a longer name cut to [`NAME_WIDTH`].
///
/// **This view's own widths, not the Templates view's** (blueprint D106, H-27): a skill row carries
/// a version, an activation and a token estimate where a template row carried a version and a
/// role. `templates.rs`'s 40 and 44 do not move, which is the structural reason the six
/// `templates__*.snap` files cannot.
const LIST_WIDTH: u16 = 46;

/// A skill row's name field, in chars: a longer name is cut to `NAME_WIDTH - 1` and `…`, so the
/// head, the activation and the estimate stay on the row.
const NAME_WIDTH: usize = 26;

/// The help column's width, borders included: the longest help line is 36 chars.
const HELP_WIDTH: u16 = 38;

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";

/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";

/// A key pressed with nothing under the cursor.
const SELECT_A_SKILL: &str = "select a skill";

/// `Esc` over a modified draft, the first time.
const UNSAVED: &str = "unsaved changes \u{2014} Esc again discards";

/// A save went out.
const SAVING: &str = "saving\u{2026}";

/// An `$EDITOR` return that came back with text.
const EDITED: &str = "edited in $EDITOR \u{2014} Ctrl+S saves";

/// An `$EDITOR` return that changed nothing.
const NO_CHANGES: &str = "no changes";

/// Appended to [`NO_CHANGES`] when the editor returned within `QUICK_EXIT`.
const WAIT_FLAG: &str = " \u{2014} a GUI editor needs its wait flag, e.g. `code --wait`";

/// The pane's bottom border while its lines overflow it.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row in Browse. The Templates view's clauses in its order, with the skills verbs: no
/// `D default` (a skill has no compiled seed) and no `h/l view` (the tab owns those).
const BROWSE_HINT: &str = "j/k move  ,/. version  b base  d diff  e description  E edit  \
                           n new  r reload";

/// The hint row while naming a skill or editing its description.
const NAMING_HINT: &str = "Tab next field  Enter confirm  Esc cancel";

/// The hint row in the editor, before the cursor's `L{line}:C{col}`.
const EDIT_HINT: &str = "Ctrl+S save  Ctrl+E $EDITOR  Esc cancel";

/// The save that landed while the head moved: the draft is kept and **both** tokens move to the
/// head the user has now been told about, so the next `Ctrl+S` is a deliberate
/// overwrite-by-append rather than a second stale refusal.
fn skill_changed_elsewhere(head: i32) -> String {
    format!(
        "this skill changed elsewhere and is now at v{head}; the draft is kept, and Ctrl+S \
         appends to v{}",
        head + 1
    )
}

/// The Skills view. Holds no store handle and no `UserId` (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct SkillsView {
    /// The last read, or `None` before the first reply.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale list.
    unavailable: Option<String>,
    /// The highlighted row, an index into [`rows`](SkillsView::rows).
    cursor: usize,
    /// The version shown for the selected skill; `None` is the head.
    shown: Option<i32>,
    /// What `d` diffs against; `None` is the shown version's predecessor.
    base: Option<DiffBase>,
    /// Which pane the Browse layout shows.
    pane: Pane,
    /// The pane's first drawn row (`J`/`K`, `PageDown`/`PageUp`). Back to the top whenever the
    /// pane shows something else: another row, another version, the other pane.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](SkillsView::scroll) clamps against. A
    /// `Cell` for [`page`](SkillsView::page)'s reason.
    pane_rows: Cell<usize>,
    /// Browsing, naming a skill, or editing its body.
    mode: Mode,
    /// The write in flight, by `StoreRequest::name`. One at a time (`settings/prompt.rs`'s rule):
    /// the staleness index keeps only the newest request of a kind.
    busy: Option<&'static str>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
    /// What `Ctrl+E` asked `$EDITOR` for, until `on_external_edit`.
    external: Option<Pending>,
    /// The editor's last drawn height: what `PageUp`/`PageDown` move by. A `Cell` because the
    /// height is known only in `render(&self)`.
    page: Cell<u16>,
}

/// One row of the library, derived from the snapshot on demand.
///
/// **One variant, not two**: the library is global (`skill` has no `project_id`, plan D92), so
/// there is no project header to draw — the first and only structural deviation from the
/// Templates view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// One library entry, by name: the row's label and its identity.
    Skill(String),
}

/// What the keys are doing.
#[derive(Debug, Default)]
enum Mode {
    /// Moving through the library.
    #[default]
    Browse,
    /// `n` (empty) or `e` (filled): the name and the description. `Tab` moves between the two
    /// fields, so create and edit-description are one code path and one hint.
    ///
    /// **Two fields, not one**: a template has no description, so `templates.rs`'s
    /// `Naming { project, field }` carries one, while a skill's name **and** description are both
    /// `R-SKL-1` ("the library is `name` + `description` + versioned body").
    Naming {
        /// The library key. Prefilled and immutable under `e`; free under `n`.
        name: TextField,
        /// `skill.description`, the list's one-liner.
        description: TextField,
        /// Which field has the cursor. The Templates view's single field has no such state, so
        /// `Tab` is what makes this one necessary.
        focus: Focus,
        /// `true` when the name is fixed (`e`), so a typing key never reaches it.
        fixed_name: bool,
    },
    /// `E`, or `Enter` on the form: the body editor.
    Editing(Editor),
}

/// Which of [`Mode::Naming`]'s two fields has the cursor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The library key, which is what `n` opens on.
    #[default]
    Name,
    /// The description, which is what `e` opens on.
    Description,
}

/// An open body editor. Never `Debug`s a body.
struct Editor {
    /// The library key, and the subject of the `upsert_skill` half of the save.
    name: String,
    /// The head version when the editor opened: the `add_skill_version` token (`None`: a new
    /// name).
    token: Option<i32>,
    /// The skill's `updated_at` when the editor opened: the `upsert_skill` token (`None`: no row).
    /// **D101**: two tokens, two tables, and the request carries both.
    updated_at: Option<DateTime<Utc>>,
    /// The version the draft started from, for the pane title (`None`: a new name).
    from: Option<i32>,
    /// The draft.
    area: TextArea,
    /// The text the draft started from: `Esc` asks only when the draft differs.
    original: String,
    /// The description the save carries: what `e` last left in the form, and the stored one when
    /// the editor was opened with `E`. Never empty in practice — an empty description would be a
    /// save that erases the row's one-liner, and the `updated_at` token is what says the row has
    /// not moved underneath.
    description: String,
    /// `Esc` warned about unsaved changes; the next one discards.
    esc_armed: bool,
    /// The body the save in flight carries: what tells that save's version from another session's,
    /// and a draft typed on since from the one that was saved.
    sent: Option<String>,
}

impl Editor {
    /// An editor over `text` (line ends normalised by `TextArea::with_text`), cursor at byte 0.
    fn new(
        name: String,
        token: Option<i32>,
        updated_at: Option<DateTime<Utc>>,
        from: Option<i32>,
        description: String,
        text: &str,
    ) -> Self {
        let area = TextArea::with_text(text);
        Self {
            name,
            token,
            updated_at,
            from,
            original: area.text().to_owned(),
            area,
            description,
            esc_armed: false,
            sent: None,
        }
    }
}

/// Lengths, never the text: `original` and `sent` are the body, as the draft is (`TextArea`'s
/// rule).
impl core::fmt::Debug for Editor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Editor")
            .field("name", &self.name)
            .field("token", &self.token)
            .field("updated_at", &self.updated_at)
            .field("from", &self.from)
            .field("area", &self.area)
            .field("original_len", &self.original.len())
            .field("description_len", &self.description.len())
            .field("esc_armed", &self.esc_armed)
            .field("sent_len", &self.sent.as_ref().map(String::len))
            .finish()
    }
}

/// An `$EDITOR` handoff in flight.
///
/// The editor travels with it: `Edited` opens it on the returned text, and a `Ctrl+E` handoff
/// that came back with nothing puts it back as it was.
#[derive(Debug)]
struct Pending {
    /// The editor the outcome opens, or returns to.
    editor: Editor,
    /// Whether the handoff came from the in-app editor (`Ctrl+E`).
    resume: bool,
}

/// One line of report.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// Dim.
    Info(String),
    /// `theme.error`: something the user has to act on.
    Error(String),
}

/// The right-hand pane in Browse.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Pane {
    /// The shown version's body.
    #[default]
    Body,
    /// The diff from the base to the shown version.
    Diff,
}

/// What `d` diffs the shown version against.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum DiffBase {
    /// The shown version's predecessor.
    #[default]
    Predecessor,
    /// One named version (`b`).
    Version(i32),
}

/// A key with no modifier but `SHIFT`, which is how a terminal reports a capital.
fn plain(key: &KeyEvent) -> bool {
    (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

impl SkillsView {
    /// Whether an editor or the form is taking every key.
    pub(super) fn captures_input(&self) -> bool {
        todo!()
    }

    /// The scope changed: the library, the editor, the pending handoff and the write in flight all
    /// belong to the workspace that was left. The notice survives, as in `settings/prompt.rs`,
    /// because the scope change is often the consequence of what it reports.
    pub(super) fn on_scope_change(&mut self) {
        todo!()
    }

    /// A key the tab did not take for the view switch.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        todo!()
    }

    /// A reply addressed to the Skills tab.
    pub(super) fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) {
        todo!()
    }

    /// The `$EDITOR` handoff came back. No handoff pending (the scope changed meanwhile): ignored.
    pub(super) fn on_external_edit(&mut self, outcome: ExternalEditOutcome, _ctx: &mut Ctx<'_>) {
        todo!()
    }

    /// Draws the view below the switch line: the content, the notice row and the hint row.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        todo!()
    }

    /// Browse (plan D82). Every key here misses the global table: `q`, `Tab`, `Shift+Tab`, the
    /// digits, `?` and `w` are not among them, and the tab took `h`/`l`/`[`/`]`/arrows first.
    fn on_browse_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        todo!()
    }

    /// A version, diff or edit key with a skill under the cursor.
    fn on_skill_key(&mut self, key: char, name: String, ctx: &Ctx<'_>) {
        todo!()
    }

    /// `n`: the form, both fields empty.
    fn open_naming(&mut self) {
        todo!()
    }

    /// `e`: the form, the name prefilled and fixed and the description prefilled and focused.
    fn open_description(&mut self) {
        todo!()
    }

    /// The two-field form. `Tab` moves the cursor, exactly as the Templates view's name prompt
    /// takes `Tab` over from the shell (H-32); a refused confirm keeps the form open, so a typo
    /// is one `Backspace` away.
    fn on_naming_key(&mut self, key: KeyEvent) -> Handled {
        todo!()
    }

    /// `Enter` on the form: the editor opens on the skill with the description the form carries.
    fn confirm_naming(&mut self) {
        todo!()
    }

    /// The body editor on `name`, at its head, with `description` the save will carry.
    fn open_editor(&mut self, name: String, description: String) {
        todo!()
    }

    /// The editor: `Ctrl+S` (the area's `Submit`), `Ctrl+E` and `Esc` are the view's, `Tab` and
    /// `Shift+Tab` pass so the shell switches tabs with the draft kept, everything else is text.
    fn on_editor_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        todo!()
    }

    /// `Ctrl+S` (D78's OQ-18 default, D101's two tokens). **No parse gate**: a skill body is
    /// markdown, not a template, so there is no byte-offset error to point at and no role's
    /// closed set. First match wins: a write in flight, then the save. A body byte-identical to
    /// the head's still appends, exactly as the Templates view appends unconditionally (PRD D5) —
    /// the diff pane makes the no-op visible *before* the save, which is where the plan puts it.
    fn save(&mut self, ctx: &Ctx<'_>) {
        todo!()
    }

    /// `Ctrl+E`: the draft goes to `$EDITOR`, and the editor waits in the pending handoff. Not
    /// while a save is in flight, for `Esc`'s reason.
    fn hand_off(&mut self, ctx: &Ctx<'_>) {
        todo!()
    }

    /// The flat list the cursor indexes, one row per library entry in the read's order
    /// (`skill.name` bytes, so the order is defined). Derived, so it cannot disagree with the
    /// snapshot.
    fn rows(&self) -> Vec<Row> {
        todo!()
    }

    /// The skill under the cursor, if the cursor is on one.
    fn selected(&self) -> Option<String> {
        todo!()
    }

    /// The version shown for `name`: the pinned one, else the head. A pin the snapshot no longer
    /// holds falls back to the head.
    fn shown_row<'a>(
        &self,
        snapshot: &'a SkillsSnapshot,
        name: &str,
    ) -> Option<&'a SkillVersion> {
        todo!()
    }

    /// The activation column: the first attachment the read answered for this skill, else an
    /// em dash. The matrix (milestone 3's T4) is where the three levels are compared; a library
    /// row is one line and can only name one of them.
    fn activation_of(&self, name: &str) -> &'static str {
        todo!()
    }

    /// `j`/`k`: one row, no wrap; the version, the base and the pane go back to the head's body,
    /// from its top.
    fn move_cursor(&mut self, down: bool) {
        todo!()
    }

    /// Keeps the cursor on a row after the library changed.
    fn clamp_cursor(&mut self) {
        todo!()
    }

    /// A `Skills` reply is the save's answer only while the save is in flight **and** it holds a
    /// version above the token whose body is the one that was sent. A read served before the save
    /// (a `Tab` away and back, `2`, `r`) refreshes the library and leaves the editor, the tokens
    /// and `busy` alone; a read showing another session's `token + 1` is not the answer either,
    /// because the save's own reply is then `SkillsStale`, which keeps the draft.
    fn land_save(&mut self) {
        todo!()
    }

    /// Browse and Naming: the library on the left, the pane on the right.
    fn render_browse(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        todo!()
    }

    /// The right-hand pane's title and lines: the form, a refusal, the empty states, the shown
    /// body, or the diff. The caller wraps and scrolls them.
    fn pane(&self, width: u16, ctx: &Ctx<'_>) -> (String, Vec<Line<'static>>) {
        todo!()
    }

    /// The editor: the draft on the left, what a save does on the right.
    fn render_editor(&self, frame: &mut Frame<'_>, area: Rect, editor: &Editor, theme: &Theme) {
        todo!()
    }
}

/// Whether a snapshot answers the scope the view is in now: a save's reply that crossed a scope
/// change is fresh to the staleness index (its kind was not re-issued), and its rows belong to the
/// workspace that was left.
///
/// **Containment, not equality, and the difference is `StoreReply::Skills`'s**: the reply carries
/// a `SkillsSnapshot` and no `Scope`, and the library half of that snapshot is global, so the
/// scope's project list cannot be read back off it. What can be checked is that no attachment
/// names a project outside the current scope, which is the case a scope change actually produces.
/// `on_scope_change` has already dropped the previous snapshot, so this only ever sees a reply
/// that was in flight across the change.
fn in_scope(snapshot: &SkillsSnapshot, ctx: &Ctx<'_>) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Emit, TopBarState};
    use crate::keymap::{KeyChord, KeyScope, Keymap};
    use crate::store_worker::Origin;
    use crate::ui::tabs::SkillsTab;
    use htui_core::fixtures::ids;
    use htui_core::model::{Activation, ProjectId, Scope, SkillAttachmentRow, SkillBindingId};

    /// The scope the tests are in, as the shell builds it.
    fn scope() -> Scope {
        Scope {
            workspace_id: ids::WORKSPACE_GRAPHICS,
            project_ids: vec![ids::PROJECT_VULKAN],
        }
    }

    /// A context addressed to the Skills tab.
    fn bench() -> (TopBarState, Keymap, Theme, Emit) {
        (
            TopBarState::default(),
            Keymap::default_global(),
            Theme::default(),
            Emit::default(),
        )
    }

    /// One attachment row on `project`.
    fn attachment(project: Option<ProjectId>) -> SkillAttachmentRow {
        SkillAttachmentRow {
            id: SkillBindingId::new(),
            skill_id: ids::SKILL_TESTS,
            name: "tests".to_owned(),
            project_id: project,
            project_slug: project.map(|_| "p".to_owned()),
            phase_id: None,
            phase_name: None,
            pinned_version: None,
            position: 0,
            activation: Activation::Always,
            globs: Vec::new(),
            languages: Vec::new(),
            updated_at: htui_core::fixtures::demo_at(0, 0),
        }
    }

    /// H-29: a reply issued for the scope that was left is not painted over the new one. The
    /// attachment half of the snapshot is what says so, because the library half is global and
    /// carries no scope at all — which is the whole reason this guard is a containment check.
    #[test]
    fn a_reply_from_a_left_scope_is_not_painted() {
        let scope = scope();
        let (top_bar, keymap, theme, emit) = bench();
        let ctx = Ctx::new(
            &scope,
            &[],
            &top_bar,
            &keymap,
            &theme,
            Origin::Tab(SkillsTab::ID),
            &emit,
        );

        assert!(
            in_scope(
                &SkillsSnapshot {
                    skills: Vec::new(),
                    attachments: vec![attachment(Some(ids::PROJECT_VULKAN))],
                },
                &ctx
            ),
            "a row on a project of this scope"
        );
        assert!(
            in_scope(
                &SkillsSnapshot {
                    skills: Vec::new(),
                    attachments: vec![attachment(None)],
                },
                &ctx
            ),
            "a global row belongs to every scope"
        );
        assert!(
            !in_scope(
                &SkillsSnapshot {
                    skills: Vec::new(),
                    attachments: vec![attachment(Some(ids::PROJECT_HTUI))],
                },
                &ctx
            ),
            "a row on the workspace that was left is not this scope's"
        );
    }

    /// D102 / F-13: every browse key the view claims is checked against the global table, so a
    /// key added to `default_global` later cannot silently become a second binding. `Tab` is the
    /// one the form takes on purpose (H-32); the editor's `Tab` passes, so it is not claimed.
    #[test]
    fn every_browse_key_misses_the_global_table() {
        let map = Keymap::default_global();
        let claimed = [
            "j", "k", "down", "up", "r", "n", "e", "E", ",", ".", "b", "d", "J", "K", "pagedown",
            "pageup", "ctrl-s", "ctrl-e",
        ];
        for spec in claimed {
            let chord = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` parses"));
            assert!(
                map.resolve(&KeyScope::Global, chord).is_none(),
                "`{spec}` is claimed by the browse keys and must not be in the global table"
            );
        }
        // `Tab` is the one deliberate exception, and only inside the form.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("tab").expect("parses"))
                .is_some(),
            "the form's `Tab` shadows the global next-tab, which is why H-32 names the hint"
        );
        // D102: `w` is T6's and is not bound anywhere, so the view leaves it alone.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("w").expect("parses"))
                .is_none(),
            "`w` is unbound and unused, so the global table does not claim it either"
        );
    }

    /// The view derives `Debug` down to the open editor, whose `original` and `sent` are the body:
    /// lengths only, as the draft's `TextArea`.
    #[test]
    fn an_open_editor_debug_prints_lengths_not_text() {
        let mut editor = Editor::new(
            "house-rules".to_owned(),
            Some(1),
            None,
            Some(1),
            "secret description".to_owned(),
            "secret original",
        );
        editor.sent = Some("secret sent".to_owned());
        let view = SkillsView {
            mode: Mode::Editing(editor),
            ..SkillsView::default()
        };
        let shown = format!("{view:?}");
        assert!(!shown.contains("secret original"), "{shown}");
        assert!(!shown.contains("secret sent"), "{shown}");
        assert!(!shown.contains("secret description"), "{shown}");
        assert!(shown.contains("original_len: 15"), "{shown}");
        assert!(shown.contains("sent_len: Some(11)"), "{shown}");
    }
}
