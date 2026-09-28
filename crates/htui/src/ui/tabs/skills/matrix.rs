//! The attachments matrix of the Skills tab (MOD-9 milestone 3, T4; plan D78's UI half, D82, D83;
//! blueprint §5, D92, D100, D103, H-33..H-37).
//!
//! One **row** per level — the global row above the projects and their phases — and one **column**
//! per skill, packed with `format!` into a bordered `Block` rather than a
//! `ratatui::widgets::Table`. D82 as amended: the three `Table` users all have a *row* cursor and
//! none a selected *cell*, and this view selects a cell; the right-hand `pane()` carries the
//! selected cell's detail and the activation form. The layout is the Templates list's own pattern
//! (`templates.rs:876-949`).
//!
//! Every read and write goes through `StoreRequest` (`R-NF-3`): the view holds no store handle, no
//! `UserId` and no `BoxId`, renders from the last [`SkillsSnapshot`] and never patches a row into
//! it. The write is refused **here**, before it is sent, in the writer's own sentences, so the two
//! halves of the milestone say the same thing about the same bytes (D100).
//!
//! The level list is not derivable from the skills read: `skill_binding` names a phase, but a
//! project whose phases carry no attachment would have no phase row and could therefore never
//! grow its first one. So the matrix asks the **catalogue** for the scope's graphs and phases when
//! it opens, and the **hierarchy** for the project's repos when the repo picker is opened. Both
//! are existing `StoreRequest` variants, so no query, table or reply variant is added; they are
//! asked for once, on demand, and never per project (D92, the `Catalogue(Scope)` doc).

use core::cell::Cell;

use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;
use htui_core::model::language::{effective_globs as expand_languages, languages as LANGUAGE_NAMES};
use htui_core::model::{
    Activation, NewSkillBinding, PhaseId, ProjectId, SkillAttachmentRow, SkillBindingId, SkillId,
    SkillVersion,
};
use htui_core::store::traits::{skill_binding_refusal, skill_pin_refusal};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::app::{Ctx, Handled};
use crate::catalogue::CatalogueSnapshot;
use crate::hierarchy::HierarchySnapshot;
use crate::skills::{READ_NAME, REQUEST_NAMES, SkillSummary, SkillsSnapshot};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::backlog::detail::Scroll;
use crate::ui::{TextArea, TextField};

/// The `StoreRequest::SetSkillBinding` name, what `busy` holds while it is in flight. The slice
/// index, not a literal, so the two cannot drift (`request_names_match_the_name_arms`).
const SET_NAME: &str = REQUEST_NAMES[2];
/// The `StoreRequest::RemoveSkillBinding` name, likewise [`crate::skills::REQUEST_NAMES`][3].
const UNSET_NAME: &str = REQUEST_NAMES[3];

/// The matrix's list column, borders included: two spaces, [`LEVEL_WIDTH`], and as many
/// [`CELL_WIDTH`] skill columns as [`MAX_COLUMNS`] says fit.
const LIST_WIDTH: u16 = 46;
/// The level column, in chars: `global`, a project slug, and `graph/phase`.
const LEVEL_WIDTH: usize = 18;
/// One skill's cell, in chars: `v12`, `v12g`, `?` and `·` all fit with room to spare.
const CELL_WIDTH: usize = 13;
/// How many skill columns the list draws at once. A library wider than this scrolls sideways
/// rather than pushing the pane off the frame, so the selected cell is always on screen.
const MAX_COLUMNS: usize = (LIST_WIDTH as usize).saturating_sub(2 + LEVEL_WIDTH) / CELL_WIDTH;
/// Rows the two multi-line fields draw, so a three-glob list is visible without scrolling.
const FIELD_ROWS: u16 = 3;
/// How many rows the notice may wrap to before it is cut.
const NOTICE_LINES: usize = 3;

/// The global row's label. Every project's row carries the project's `slug` and every phase row
/// `graph/phase`, so the label column is the level and the cells are the skills.
const GLOBAL_LABEL: &str = "global";
/// The header over the level column.
const LEVEL_HEADER: &str = "level";
/// An empty cell: attached nowhere at this level.
const NO_CELL: &str = "\u{b7}";

/// The pane before the first reply.
const NOT_READ: &str = "skills not read yet";
/// The pane after a refused read, before the message.
const UNAVAILABLE: &str = "skills unavailable";
/// A key pressed with nothing under the cursor.
const SELECT_A_CELL: &str = "select a cell";
/// A write went out.
const WRITING: &str = "writing\u{2026}";
/// The panes' bottom border while their lines overflow them.
const SCROLL_HINT: &str = " J/K PgUp/PgDn scroll ";

/// The hint row while the matrix is browsed.
const BROWSE_HINT: &str = "j/k level  \u{2190}/\u{2192} skill  g/G ends  a attach  e edit  x detach  \
                           A activation  p pin  R repo  r reload  m/Esc close";
/// The hint row while the activation form is open. `Ctrl+S` rather than `Enter`, because `Enter`
/// is what splits a line in the two multi-line fields — the `TextArea` rule (MOD-7 D44).
const FORM_HINT: &str = "Tab next field  Ctrl+S save  A activation  p pin  R repo  Esc cancel";
/// The hint row while the repo picker is open.
const PICKER_HINT: &str = "j/k repo  Enter pick  Esc cancel";

/// The attachments matrix (MOD-9 milestone 3, D82): one row per level, one column per skill, the
/// activation form and the repo picker. Holds no store handle, no `UserId` and no `BoxId`
/// (`R-NF-3`).
#[derive(Debug, Default)]
pub(super) struct MatrixView {
    /// Whether the matrix is showing. The tab asks, and `Esc` or `m` clears it; nothing else
    /// opens it, because the switch line and the strip text are the shell's and byte-identical
    /// (H-15, H-28).
    open: bool,
    /// The last read, or `None` before the first reply. The same [`SkillsSnapshot`] the library
    /// view draws, kept separately so neither patches a row into the other's copy.
    snapshot: Option<SkillsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`: the pane says so instead of a stale matrix.
    unavailable: Option<String>,
    /// The scope's graphs and phases, read when the matrix opened: the level list is not
    /// derivable from the skills read, and a project with no phase attachment would otherwise
    /// have no phase row to attach one to.
    catalogue: Option<CatalogueSnapshot>,
    /// The scope's repos, read when the repo picker opened. `None` until the reply lands.
    repos: Option<Box<HierarchySnapshot>>,
    /// The highlighted **level**, an index into [`levels`](MatrixView::levels): the global row
    /// first, then each scope project, then that project's phases.
    level: usize,
    /// The first skill column drawn, so a library wider than [`MAX_COLUMNS`] scrolls sideways and
    /// the selected cell is never off screen.
    first_column: usize,
    /// The highlighted **skill**, an index into the snapshot's `skills`.
    skill: usize,
    /// The activation form, open over the selected cell; `None` while the matrix is browsed.
    form: Option<Form>,
    /// The repo picker, open over the form's qualifier.
    picker: Option<Picker>,
    /// The pane's first drawn row.
    scroll: Scroll,
    /// The pane's rows at the last draw, what [`scroll`](MatrixView::scroll) clamps against.
    pane_rows: Cell<usize>,
    /// The write in flight, and the cell it was sent for, so the reply can name it.
    busy: Option<Busy>,
    /// The last outcome, one line above the hint.
    notice: Option<Notice>,
}

/// The three axes of the matrix, in the order the rows are drawn: the global row, then each scope
/// project, then each of that project's phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// `project_id IS NULL`: every project.
    Global,
    /// One project's row, and, under it, its phases.
    Project(ProjectId),
    /// One phase's row.
    Phase(ProjectId, PhaseId),
}

/// One drawn row: the level and the text its label column carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Level {
    /// The level, and the `(project_id, phase_id)` the write carries.
    axis: Axis,
    /// The label: `global`, a project `slug`, or `graph/phase`.
    label: String,
}

/// The activation form, open over one `(skill, level)` cell.
#[derive(Debug)]
struct Form {
    /// The cell being edited.
    skill_id: SkillId,
    /// Its library key, so the reply's notice names the skill without a second read.
    name: String,
    /// Its level, so the write carries the right `(project_id, phase_id)`.
    level: Axis,
    /// The winning attachment's `updated_at`, the CAS token (`None`: no row yet).
    token: Option<DateTime<Utc>>,
    /// `always`, `glob` or `off`.
    activation: Activation,
    /// Follows the latest, or pins a version the skill has.
    pin: Pin,
    /// The globs, one per line — what the store receives, which is why a reopen shows the
    /// **effective** list and not the typed one: the language expansion is already in it, and
    /// [`effective_globs`] de-duplicates, so saving again is a no-op.
    globs: TextArea,
    /// The languages, one per line, each of which [`LANGUAGE_GLOBS`] expands at save.
    languages: TextArea,
    /// `skill_binding.position`, typed.
    position: TextField,
    /// The `<repo>:` the picker wrote, prepended to every **typed** glob; `None` for a bare glob.
    qualifier: Option<String>,
    /// Which of the three fields `Tab` is on.
    field: usize,
}

/// Whether an attachment follows the latest version or pins one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin {
    /// `pinned_version = NULL`: the latest version is in force.
    Latest,
    /// `pinned_version = Some(n)`, cleared back to [`Pin::Latest`] by the next `p`.
    Version(i32),
}

/// The repo picker: which repo the form's qualifier names.
#[derive(Debug)]
struct Picker {
    /// The highlighted row: `0` is "every repo", then the project's repos by name.
    cursor: usize,
}

/// The write in flight, and the cell it was sent for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Busy {
    /// `StoreRequest::name`, so the reply knows which notice to give.
    request: &'static str,
    /// The cell's skill name.
    name: String,
    /// The cell's level label, which is what the user was looking at.
    level: String,
}

/// `Notice` is `library.rs`'s own type, copied verbatim: two variants, and no third state is
/// needed here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    /// An outcome that is not a failure.
    Info(String),
    /// A refusal, from this view or from the store.
    Error(String),
}

impl MatrixView {
    /// Whether the matrix is showing, and therefore whether it owns every key: `Esc` closes it
    /// and `h`/`l` stop being the tab's view switch while it is up.
    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// The scope changed: the matrix, the form, the picker and the write in flight all belong to
    /// the workspace that was left. The notice survives, as in `settings/prompt.rs`.
    pub(super) fn on_scope_change(&mut self) {
        let notice = self.notice.take();
        *self = Self {
            notice,
            ..Self::default()
        };
    }

    /// A key the tab did not take for the view switch. `m` opens the matrix and `Esc` closes it;
    /// everything else belongs to whichever of the three forms is up.
    pub(super) fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        todo!()
    }

    /// A reply addressed to the Skills tab. **A no-op until the view lands**: the tab forwards
    /// every reply to all three views, so a `todo!()` here would take the Templates and Skills
    /// suites down with it rather than leaving this one red.
    pub(super) fn on_reply(&mut self, _reply: &StoreReply, _ctx: &mut Ctx<'_>) {}

    /// Draws the view below the switch line: the matrix, the notice row and the hint row.
    pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
        todo!()
    }
}

/// The level list, in the order the rows are drawn: the global row, then each scope project, then
/// that project's phases by graph name and phase position.
fn levels(_view: &MatrixView, _ctx: &Ctx<'_>) -> Vec<Level> {
    todo!()
}

/// The row the cursor is on, or `None` when the list is empty.
fn selected_level<'a>(_levels: &'a [Level], _level: usize) -> Option<&'a Level> {
    todo!()
}

/// The skill under the cursor, if the cursor is on one.
fn selected_skill(_view: &MatrixView) -> Option<&SkillSummary> {
    todo!()
}

/// The attachment of one `(skill, level)` cell, or `None` when the level holds none.
fn cell<'a>(_view: &'a MatrixView, _skill: SkillId, _axis: Axis) -> Option<&'a SkillAttachmentRow> {
    todo!()
}

/// One cell's text: the version in force, `?` for a pin the skill does not have, a `g` for a
/// `glob` activation, and [`NO_CELL`] for no attachment at all.
fn cell_text(_view: &MatrixView, _skill: SkillId, _axis: Axis) -> String {
    todo!()
}

/// The three `(skill, level)` writes this view can refuse before it sends one (D78's order, minus
/// the token read, which is the store's). Every sentence is the one the writer would have
/// produced, so the view's notice and the store's `Constraint` cannot drift (D100).
fn refusal(_view: &MatrixView, _form: &Form) -> Option<String> {
    todo!()
}

/// `form.effective_globs()`: the typed globs, each carrying the qualifier, unioned with every
/// named language's expansion — the list the store receives and the list the form previews.
fn effective_globs(_form: &Form) -> Vec<String> {
    todo!()
}

/// The versions of the skill a pin has to name.
fn versions(_view: &MatrixView, _skill: SkillId) -> &[SkillVersion] {
    todo!()
}

/// A skill the snapshot does not name, or no reply at all.
fn in_scope(_snapshot: &SkillsSnapshot, _ctx: &Ctx<'_>) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::LANGUAGE_NAMES;
    use crate::keymap::{KeyChord, KeyScope};
    use crate::ui::overlay::OverlayId;
    use htui_core::store::traits::invalid_skill_name;

    /// D102 / F-13: every key the matrix claims is checked against the global table, so a key
    /// added to `default_global` later cannot silently become a second binding.
    #[test]
    fn every_matrix_browse_key_misses_the_global_table() {
        let map = crate::keymap::Keymap::default_global();
        let claimed = [
            "j", "k", "down", "up", "left", "right", "g", "G", "a", "e", "x", "A", "p", "r", "R",
            "m", "J", "K", "pagedown", "pageup", "ctrl-s", "esc",
        ];
        for spec in claimed {
            let chord = KeyChord::parse(spec).unwrap_or_else(|| panic!("`{spec}` parses"));
            assert!(
                map.resolve(&KeyScope::Global, chord).is_none(),
                "`{spec}` is claimed by the matrix's keys and must not be in the global table"
            );
        }
        // `Esc` is the shell's **overlay** close, under `KeyScope::Overlay(ANY)` and not
        // `KeyScope::Global`, which is what lets the matrix take it: no overlay is up while a tab
        // is drawn, so the chord never reaches the overlay's arm.
        assert!(
            map.resolve(&KeyScope::Overlay(OverlayId::ANY), KeyChord::parse("esc").expect("parses"))
                .is_some(),
            "`Esc` is the overlay close, under the overlay scope and not the global one"
        );
        // `Tab` is the one deliberate exception: it is bound globally and the form takes it to move
        // its cursor, which is what H-32 names for the Templates view's naming prompt.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("tab").expect("parses"))
                .is_some(),
            "`Tab` is the form's documented exception, so it stays in the global table"
        );
        // D102: `w` is the workspace switcher's, bound in `register_all` rather than in
        // `default_global`, and the matrix leaves it alone.
        assert!(
            map.resolve(&KeyScope::Global, KeyChord::parse("w").expect("parses"))
                .is_none(),
            "`w` is the switcher's and is bound outside `default_global`, so this table cannot \
             see it — which is why the matrix never claims it"
        );
    }

    /// The names the form lists beside the language field, so a maintainer can see what the map
    /// holds without reading the source.
    #[test]
    fn the_language_list_is_the_map_own_order() {
        assert_eq!(
            LANGUAGE_NAMES(),
            htui_core::model::language::languages(),
            "the list and the map cannot be two orders"
        );
        assert!(
            LANGUAGE_NAMES().contains(&"shell"),
            "`shell` is the three-pattern language the expansion test rides"
        );
    }

    /// The refusal sentences the view shows are the writer's own, so this pins that the two are
    /// reachable from one place rather than re-spelled here (D100).
    #[test]
    fn the_writers_sentences_are_the_ones_the_view_shows() {
        assert!(
            invalid_skill_name("House Rules").contains("must be 1-64 characters of `[a-z0-9-]`"),
            "the name rule is one function beside the template's, and the library view shows it \
             verbatim for the same reason"
        );
    }

    /// The level label column is fixed, because the cells beside it are packed by width: a wider
    /// level would push every skill column off the frame.
    #[test]
    fn the_level_label_column_holds_the_labels_the_levels_produce() {
        for label in [
            "global",
            "vulkan-tutorials",
            "feature/implement",
            "analysis/research",
        ] {
            assert!(
                label.len() <= super::LEVEL_WIDTH,
                "`{label}` is {} chars and the column is {}",
                label.len(),
                super::LEVEL_WIDTH
            );
        }
        assert_eq!(super::MAX_COLUMNS, 2, "the list's 46 columns hold two skills");
        assert_eq!(
            2 + super::LEVEL_WIDTH + super::CELL_WIDTH * super::MAX_COLUMNS,
            super::LIST_WIDTH as usize,
            "and the row is packed to exactly the block's width"
        );
    }
}
