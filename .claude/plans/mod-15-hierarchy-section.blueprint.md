# Blueprint: MOD-15 milestone 3 — the app can take typed input

Elaborates `.claude/plans/mod-15-hierarchy-section.plan.md` (D1–D15, F-1…F-29 honoured, none
re-opened). PRD D1/D8/D11/D13 and M1 D3/D4/D5/D10 win on conflict. Tree at `e226d3a`. Graphify
consulted through `graphify-out/graph.json` (built at `3e346107`; the query CLI is not importable
in this environment, so the neighbourhoods of `SettingsSection`, `SettingsTab`, `StoreRequest`,
`StoreReply`, `Backend`, `Bench`, `ProbeSection`, `WorkspaceSummary`, `WorkspaceSwitcher` were read
from the JSON), every line below then read from the tree. Sibling format:
`.claude/plans/mod-15-hierarchy-seam.blueprint.md`, `.claude/plans/mod-15-project-seed.blueprint.md`.

## 0. Flags — read before building

The plan is buildable. Thirteen things in it are imprecise or unbuildable as written; none re-opens
D1–D15, each is the smallest fix that keeps the plan's rule.

| # | Where | Problem | Resolution used below |
|---|---|---|---|
| **A** | D9 delete copy | The illustrative list ("{n} items, {n} runs, …, {n} key counters") is **not** `DeleteReach` field order and omits `workspace_links`, `workspace_box_paths`, `command_runs`. D9's own rule is "order = `DeleteReach` field order `traits.rs:744-792`". | The rule wins over the illustration: 21 labels in field order (§6.4), zero counts omitted, destructured without `..` so a 22nd field is a compile error. |
| **B** | D9 "every key not listed is `Consumed` (the modality of `answer_consent`)" | `answer_consent` is **not** blanket-consuming: `_ => Handled::Pass` at `agents.rs:799`, with a comment (`:791-797`) that a blanket `Consumed` would kill `q`, `?`, `Tab`, digits. | D9's rule wins (a typed confirmation must swallow letters). One carve-out: chords carrying `CONTROL` pass in every modal state so `ctrl-c` (global) still quits. §9.3. |
| **C** | D8 status-line text ``set_repo_path: `/x` is a link to nothing`` | `StoreError::Constraint`'s `Display` is `"constraint violated: {0}"` (`error.rs:21`) and `failed()` renders `err.to_string()` (`store_worker.rs:603-608`). | The line reads ``set_repo_path: constraint violated: `/x` is a link to nothing``. Tests assert `contains(refusal.to_string())`. |
| **D** | D4 "`shell_empty.snap:19` moves with it" | The copy is pinned in **two** snapshots: `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap:19` and `crates/htui/tests/snapshots/shell__switcher_empty.snap:19` (`tests/shell.rs:192`). | Both move in T3. §10. |
| **E** | D15 "same seven fields" | `Bench` has **six** fields (`tests/settings.rs:126-133`); the seventh `Ctx::new` argument is the fixed `Origin::Tab(SettingsTab::ID)` (`:156`). | `SectionBench` has six fields, `origin` is a constant inside `ctx()`. |
| **F** | D14/D15 section frames in `tests/hierarchy.rs` | `render_section`/`draw_section_at`/`text_of` are private to `tests/settings.rs` (`:56-98`); D15 promotes only `Bench`. | `SectionBench` gains a sixth method `render_section(&self, section, width) -> String` over `testkit::buffer_text` (`testkit.rs:426-440`). `tests/settings.rs` keeps its own helpers (they also serve `accented_lines` and `SECTION_BORDERED`). |
| **G** | D14 snapshot names `hierarchy_demo` … | `insta::assert_snapshot!("agents_demo", ..)` in `tests/settings.rs` yields `settings__agents_demo.snap`; `"hierarchy_demo"` in `tests/hierarchy.rs` would yield `hierarchy__hierarchy_demo.snap`. | Macro names are the bare `demo`, `editor_repo`, `delete_warn`, `delete_typed`, `stale`, `no_workspace`, `offline` → files `hierarchy__{name}.snap` as the plan's file table says. |
| **H** | D4 strip-width pin in T1 | T1 cannot name `HierarchySection` (T3's file). | T1 writes the pin over `vec![Box::new(AgentsSection::new())]`; T3 appends `Box::new(HierarchySection::new())` — one line in `tests/settings.rs`, added to T3's file set. |
| **I** | D9 "mirror rebuilt / no mirror / mirror not rebuilt: {err}" | Three texts for four `MirrorAfterDelete` variants. | `NotNeeded` → `"no rebuild needed"`. |
| **J** | D13 `Mode::Deleting { target, slug, stage }` with `stage: Warn` and `Typed { field }` | The warn pane and the typed pane both render the counts, so `DeleteReach` has to live in the stage. | `DeleteStage::{Counting, Warn(DeleteReach), Typed { reach: DeleteReach, field: TextField }, InFlight(DeleteReach)}`. |
| **K** | D2 "the `captures_input()` check is the first statement of `on_key`" | `SettingsRegistry.active()` returns `Option<&dyn SettingsSection>` (`settings/mod.rs:108-111`) — the check needs `is_some_and`. | `if self.sections.active().is_some_and(SettingsSection::captures_input) { return self.delegate(key, ctx); }` with the existing `get_mut` delegation (`:209-212`) hoisted into `fn delegate`. |
| **L** | D8 guard, non-symlink `canonicalize` failure | An existing non-link path whose `canonicalize` fails (EACCES on a parent) is covered by none of the four variants. | Mapped to `Missing` and documented; no fifth variant. |
| **M** | D1 window "last `width - 1` chars ending at the cursor" | Ambiguous whether the cursor cell is one of the `width - 1`. | The window is `…` + `width - 2` chars + the cursor cell = `width` cells; masked fields reserve the ` (n)` suffix first. Rule and four worked examples in §1.3. |

## 1. `TextField` (T1)

File: `crates/htui/src/ui/text_field.rs` (new). `crates/htui/src/ui/mod.rs` (10 lines): insert
`pub mod text_field;` between `pub mod tabs;` (`:7`) and `pub mod theme;` (`:8`); append
`pub use text_field::{FieldOutcome, TextField};` after `pub use theme::Theme;` (`:10`).

### 1.1 Types

```rust
//! One single-line text field with a char cursor and an optional mask (MOD-15 milestone 3, D1;
//! PRD D1). The widget MOD-22, MOD-23 and milestone 6's DSN entry consume; the mask has no
//! consumer until milestone 6.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::Theme;

/// What one key did to the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldOutcome {
    /// The field edited or moved; nothing for the caller to do.
    Consumed,
    /// `Enter`: the caller reads [`TextField::text`] or [`TextField::take`].
    Submit,
    /// `Esc`: the caller decides what cancelling means.
    Cancel,
    /// Not a field key (`Tab`, `BackTab`, `Up`, `Down`, `F(n)`, any `CONTROL`/`ALT` chord): the
    /// caller keeps its own bindings.
    Pass,
}

/// A single-line buffer with a cursor, counted in `char`s (D1: no `unicode-width` is declared).
#[derive(Clone, Default)]
pub struct TextField {
    text: String,
    /// Char index, `0..=text.chars().count()`.
    cursor: usize,
    masked: bool,
}

/// Never the text (D1): `StoreRequest`, `RequestEnvelope` and every section derive `Debug`.
impl core::fmt::Debug for TextField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextField")
            .field("masked", &self.masked)
            .field("len", &self.len())
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl TextField {
    #[must_use] pub fn new() -> Self;                       // empty, unmasked
    #[must_use] pub fn masked() -> Self;                    // empty, masked
    #[must_use] pub fn with_text(text: &str) -> Self;       // unmasked, cursor at end
    pub fn on_key(&mut self, key: KeyEvent) -> FieldOutcome;
    #[must_use] pub fn text(&self) -> Option<&str>;         // None when masked
    #[must_use] pub fn take(&mut self) -> String;           // moves the buffer out, cursor = 0
    pub fn clear(&mut self);
    #[must_use] pub fn len(&self) -> usize;                 // chars
    #[must_use] pub fn is_empty(&self) -> bool;
    #[must_use] pub const fn is_masked(&self) -> bool;
    #[must_use] pub fn line(&self, width: u16, focused: bool, theme: &Theme) -> Line<'static>;
}
```

### 1.2 `on_key` table

| Key (`key.code`, `key.modifiers`) | Effect | Returns |
|---|---|---|
| `Char(c)`, neither `CONTROL` nor `ALT`, `!c.is_control()` | insert at cursor, cursor + 1 | `Consumed` |
| `Char(c)`, neither `CONTROL` nor `ALT`, `c.is_control()` | nothing (ANA-10 `:1341`) | `Consumed` |
| `Backspace` | remove char before cursor if `cursor > 0`, cursor − 1 | `Consumed` |
| `Delete` | remove char at cursor if `cursor < len` | `Consumed` |
| `Left` / `Right` | cursor ∓ 1, saturating at `0` / `len` | `Consumed` |
| `Home` / `End` | cursor = `0` / `len` | `Consumed` |
| `Enter` | — | `Submit` |
| `Esc` | — | `Cancel` |
| anything else (incl. any chord with `CONTROL` or `ALT`) | — | `Pass` |

`SHIFT` is allowed on `Char` (a terminal reports `N` as `Char('N')` + `SHIFT`; `KeyChord::new`
drops the flag, `keymap.rs:38-40`). Insertion is by char index: `let byte = self.text.char_indices().nth(self.cursor).map_or(self.text.len(), |(b, _)| b); self.text.insert(byte, c);`.

### 1.3 `line()` — the render window (flag M)

```
width   = usize::from(width)
suffix  = masked ? format!(" ({})", len) : ""          // dim span, always after the window
budget  = width.saturating_sub(suffix.chars().count()) // cells for the window, cursor cell included
glyphs  = masked ? ['•'; len] : text.chars()
if budget < 2            → window = cursor cell only (no ellipsis)
elif cursor + 1 <= budget → start = 0, ellipsis = false
else                      → start = cursor + 2 - budget, ellipsis = true   // '…' + (budget-2) chars + cursor cell
before  = glyphs[start..cursor]                       (theme.base)
at      = glyphs.get(cursor).unwrap_or(' ')           (focused ? theme.selected : theme.base)
after   = glyphs[cursor+1..] cut to budget - ellipsis - before.len() - 1   (theme.base)
spans   = [ellipsis ? "…" (theme.dim)] + before + at + after + [suffix (theme.dim)]
```

Computed on every call from the `Rect` the caller passes; nothing cached (ANA-10 `:1333-1334`).
Only a **leading** `…` exists; clipped chars after the cursor vanish silently.

Worked examples (cells shown with `|` boundaries; `[x]` = `theme.selected` when focused):

| # | width | text | cursor | masked | focused | cells (exact) | `buffer_text` line (trailing blanks trimmed) |
|---|---|---|---|---|---|---|---|
| 1 | 10 | `abc` | 3 | no | yes | `a b c [ ] . . . . . .` | `abc` |
| 2 | 10 | `abcdefghijkl` | 12 | no | yes | `… e f g h i j k l [ ]` | `…efghijkl` |
| 3 | 10 | `abcdefghijkl` | 5 | no | yes | `a b c d e [f] g h i j` (`k`,`l` clipped) | `abcdefghij` |
| 4 | 12 | `hunter2` | 7 | yes | yes | `• • • • • • • [ ] ␠ ( 7 )` (suffix ` (7)`, budget 8) | `•••••••  (7)` |
| 5 | 8 | 12 chars | 12 | yes | yes | `… • [ ] ␠ ( 1 2 )` (suffix ` (12)`, budget 3, start 11) | `…•  (12)` |

Unfocused: identical glyphs, `at` in `theme.base`. Example 1's cursor is a **space** — invisible in
snapshot text (`testkit.rs:436` trims), visible only as a style: tests assert the cursor through a
`Buffer` cell (hazard E-17).

### 1.4 Unit tests (`#[cfg(test)] mod tests`, helper `fn key(code: KeyCode) -> KeyEvent { KeyEvent::from(code) }` as `composer.rs:103-110`)

`inserts_at_the_cursor`, `backspace_and_delete_at_both_ends`, `home_and_end_move`,
`a_control_char_is_swallowed`, `enter_esc_tab_outcomes` (`Submit`/`Cancel`/`Pass`; `ctrl-c` `Pass`),
`the_window_leads_with_an_ellipsis_at_width` (example 2 through a `Buffer` drawn with
`Paragraph::new(field.line(10, true, &Theme::default()))`), `the_cursor_cell_is_selected_only_when_focused`
(example 3, cell `(5, 0)` has `Modifier::REVERSED`), `a_masked_field_renders_dots_and_a_count`
(example 4), `text_is_none_when_masked`, `take_empties_the_field`,
`debug_never_prints_the_text` (`format!("{field:?}")` of a field holding `"secret"` for both modes
does not contain `secret`).

## 2. `SettingsSection::captures_input` and the gate (T1)

File: `crates/htui/src/ui/tabs/settings/mod.rs`.

Trait (`:42-58`): after `fn on_scope_change(&mut self, scope: &Scope);` add the trait's first default
body:

```rust
    /// Whether this section is consuming every printable key right now, so the tab must not take
    /// `h`/`l`/`[`/`]`/`Left`/`Right` for section cycling (ANA-10 §4.9; the chat tab's rule,
    /// `chat/mod.rs:403-412`, one tab across). Derived from a mode, never a flag.
    fn captures_input(&self) -> bool {
        false
    }
```

`SettingsTab::on_key` (`:197-213`) becomes:

```rust
    fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
        // A section that is taking typed text answers first: `l` is a letter there, not a cycle.
        if self.sections.active().is_some_and(SettingsSection::captures_input) {
            return self.delegate(key, ctx);
        }
        match key.code {
            KeyCode::Char('l') | KeyCode::Char(']') | KeyCode::Right => { self.sections.cycle_next(); return Handled::Consumed; }
            KeyCode::Char('h') | KeyCode::Char('[') | KeyCode::Left => { self.sections.cycle_prev(); return Handled::Consumed; }
            _ => {}
        }
        self.delegate(key, ctx)
    }
```

with `fn delegate(&mut self, key, ctx) -> Handled` holding the current `:209-212` body
(`self.sections.sections.get_mut(self.sections.active)` → `section.on_key` / `Handled::Pass`),
placed in `impl SettingsTab` after `with_sections` (`:168`). No other line of the file moves in T1;
T3 adds `pub mod hierarchy;` after `pub mod agents;` (`:11`) and
`pub use hierarchy::HierarchySection;` after `pub use agents::AgentsSection;` (`:26`).

## 3. `SectionBench` (T1)

File: `crates/htui/src/testkit.rs`. Insert after `fn buffer_text` (`:440`), before `#[cfg(test)] mod tests` (`:442`).
New imports at `:15-28`: `use htui_core::model::{ProjectRef, Scope, StepId};`,
`use crate::app::{Action, App, Ctx, Emit, Handled, TopBarState};`, `use crate::store_worker::{self, Origin, …};`,
`use crate::ui::Theme;`, `use crate::ui::tabs::settings::{SettingsSection, SettingsTab};`,
`use ratatui::{Terminal, TerminalOptions, Viewport}; use ratatui::layout::Rect;`.

```rust
/// Everything [`Ctx::new`] borrows, owned in one place, for testing a section without a shell
/// around it (MOD-15 milestone 3, D15; was `Bench` in `tests/settings.rs`). Holding the [`Emit`]
/// queue is how a test sees what the section asked for without a store existing (`R-NF-3`).
#[derive(Debug)]
pub struct SectionBench {
    scope: Scope,
    projects: Vec<ProjectRef>,
    top_bar: TopBarState,
    keymap: Keymap,
    theme: Theme,
    emit: Emit,
}

impl SectionBench {
    /// A bench in the demo fixture's first workspace (`Graphics`, workspaces are ordered by name).
    pub async fn new() -> Self;                               // scope = Scope::from_workspace(MemStore::demo().workspaces().await[0])
    /// A context addressed to the Settings tab: `Origin::Tab(SettingsTab::ID)`.
    #[must_use] pub fn ctx(&self) -> Ctx<'_>;
    /// Feeds one key, written as `KeyChord::parse` reads it. Panics on a non-chord.
    pub fn key(&self, section: &mut dyn SettingsSection, chord: &str) -> Handled;
    /// Hands one reply to a section.
    pub fn reply(&self, section: &mut dyn SettingsSection, reply: &StoreReply);
    /// Everything emitted since the last call, leaving the queue empty.
    #[must_use] pub fn drained(&self) -> Vec<Action>;
    /// The `Action::Error` texts since the last call.
    #[must_use] pub fn errors(&self) -> Vec<String>;
    /// Draws one section into a `width`x30 buffer and returns it as snapshot text (flag F).
    pub fn render_section(&self, section: &dyn SettingsSection, width: u16) -> String;
}
```

Bodies are `tests/settings.rs:137-188` verbatim; `render_section` is `draw_section_at` (`:56-68`)
followed by `buffer_text`. `Keymap` derives `Debug` (`keymap.rs:182`), `Emit` too (`state.rs:44`),
so `#[derive(Debug)]` satisfies `missing_debug_implementations`.

`tests/settings.rs`: delete `Bench` and `impl Bench` (`:120-189`) and `demo_scope` (`:34-42`) if
nothing else uses it; `use htui::testkit::{Harness, SectionBench};`; 46 occurrences of `Bench::new`
become `SectionBench::new` (`grep -c "Bench::new"` = 46); prune imports that only `Bench` used —
`Emit`, `TopBarState` (`:7`), `Keymap` (`:8`), `Origin` (`:9`), `ProjectRef` (`:19`) — the compiler
says which (E-12).

## 4. `tests/settings.rs` — two new tests (T1)

```rust
/// A section that is taking text: `l` must reach it, not cycle the strip (D2).
#[derive(Debug, Default)]
struct CapturingProbe { seen: Vec<KeyCode> }
impl SettingsSection for CapturingProbe {
    fn id(&self) -> SectionId { SectionId("capturing") }
    fn title(&self) -> &str { "Capturing" }
    fn captures_input(&self) -> bool { true }
    fn on_key(&mut self, key: KeyEvent, _ctx: &mut Ctx<'_>) -> Handled { self.seen.push(key.code); Handled::Consumed }
    fn render(&self, frame, area, ctx) { message(frame, area, &format!("seen {}", self.seen.len()), ctx.theme) }
    // wants_requests / on_scope_change / on_reply as ProbeSection (`:878-888`)
}

#[tokio::test]
async fn a_capturing_section_receives_l_and_a_plain_one_cycles() {
    // [AgentsSection, CapturingProbe]: `l` cycles to Capturing (Agents does not capture),
    // a second `l` renders "seen 1" and the strip did not move; `h` renders "seen 2".
    // Then `q`: the shell must still be running — `harness.app().should_quit` is false, because
    // the capturing probe consumed it; `esc`… stays in Capturing (no mode here), so the test
    // ends there. The Browse-mode `q` case is T3's (`q_quits_from_browse`).
}

#[tokio::test]
async fn the_section_strip_fits_the_frame() {
    // D4 / PRD risk `:378`. `render_strip` draws `format!(" {title} ")` per section
    // (`settings/mod.rs:243-258`): width = Σ (chars + 2). Two titles = 19 ≤ SECTION_WIDE.
    let sections: Vec<Box<dyn SettingsSection>> = vec![Box::new(AgentsSection::new())]; // T3 appends HierarchySection (flag H)
    let width: usize = sections.iter().map(|s| s.title().chars().count() + 2).sum();
    assert!(width <= usize::from(SECTION_WIDE), "strip is {width} columns");
}
```

## 5. `canonical_root` (T2)

File: `crates/htui-core/src/root_path.rs` (new). `crates/htui-core/src/lib.rs` (18 lines): insert
`pub mod root_path;` between `pub mod prompt;` (`:12`) and `pub mod scrub;` (`:13`).
`crates/htui-core/Cargo.toml` `[dev-dependencies]` (`:26-28`): add `tempfile = "3"` after `insta`
(no workspace `tempfile` entry exists — root `Cargo.toml` grep empty — so the literal version, as
`crates/htui/Cargo.toml:59`).

```rust
//! The per-box root guard (MOD-15 milestone 3, D8; PRD D11): a workspace root or a repo checkout
//! is stored canonical or refused, and a refusal never names a link's target (`R-BOX-4`; the
//! message rule of `htui-agent/src/excerpt.rs:419-430`).

use std::path::{Path, PathBuf};

/// Why a typed path was not stored. Each carries the path **as typed**.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RootRefusal {
    #[error("`{}` is not an absolute path", .0.display())]
    Relative(PathBuf),
    #[error("`{}` does not exist on this box", .0.display())]
    Missing(PathBuf),
    #[error("`{}` is a link to nothing", .0.display())]
    Dangling(PathBuf),
    #[error("`{}` is not a directory", .0.display())]
    NotADirectory(PathBuf),
}

/// The canonical directory `path` names, or why it is refused. Sync and `std::fs` only: the
/// caller (`htui`'s store worker) wraps it in `spawn_blocking`; `htui-core`'s tokio is
/// `macros, rt` (`Cargo.toml:27`). Windows `\\?\` prefixes are stored as returned (MOD-16).
pub fn canonical_root(path: &Path) -> Result<PathBuf, RootRefusal> {
    let typed = || path.to_path_buf();
    if !path.is_absolute() {                                   // 1
        return Err(RootRefusal::Relative(typed()));
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| RootRefusal::Missing(typed()))?;  // 2
    let canonical = match std::fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) if meta.is_symlink() => return Err(RootRefusal::Dangling(typed())),          // 3
        Err(_) => return Err(RootRefusal::Missing(typed())),                                 // flag L
    };
    let is_dir = std::fs::metadata(&canonical).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {                                               // 4
        return Err(RootRefusal::NotADirectory(typed()));
    }
    Ok(canonical)
}
```

Check order is fixed: **Relative → Missing → Dangling → NotADirectory**; a link to a directory
passes as the directory (`/tmp` reasoning, `excerpt.rs:396-398`). Tests (`#[cfg(test)]`, `tempfile`):
`a_relative_path_is_refused_first` (`"x/y"` and the four `Display` strings pinned by `assert_eq!`),
`a_missing_path_is_missing`, `a_link_to_a_directory_is_stored_canonical` (`#[cfg(unix)]`,
`std::os::unix::fs::symlink`), `a_dangling_link_is_refused_without_naming_its_target`
(`#[cfg(unix)]`; the message contains the typed path and not the target string),
`a_file_is_not_a_directory`, `a_directory_is_stored_canonical`.

## 6. `crates/htui/src/hierarchy.rs` (T2)

`crates/htui/src/lib.rs` (`:12-20`): insert `pub mod hierarchy;` between `pub mod event_loop;`
(`:15`) and `pub mod keymap;` (`:16`).

### 6.1 Snapshot types (D5)

```rust
//! The hierarchy the Settings tab edits: one worker-assembled snapshot per read, twelve served
//! writes, identity filled here and never on the render side (MOD-15 milestone 3, D5/D6/D10).

use chrono::{DateTime, Utc};
use htui_core::model::{BoxId, Project, ProjectId, ProjectRef, Repo, RepoBoxPath, RepoId, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspaceProject, WorkspaceSummary, NewWorkspace, NewProject, NewRepo, WorkspacePatch, ProjectPatch, RepoPatch};
use htui_core::store::{CasOutcome, DeleteReach, DeleteTarget, ReadStore, Result, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE};
use crate::store_worker::{StoreReply, StoreRequest};

/// A workspace, its projects in position order, their repos by name, this box's paths.
#[derive(Debug, Clone, PartialEq)]
pub struct HierarchySnapshot {
    pub workspace: Workspace,
    pub this_box: Option<BoxId>,
    pub root_path: Option<WorkspaceBoxPath>,     // this box's, if any
    pub projects: Vec<ProjectEntry>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEntry { pub link: WorkspaceProject, pub project: Project, pub repos: Vec<RepoEntry> }
#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry { pub repo: Repo, pub local_path: Option<RepoBoxPath> }

/// What the worker did to the mirror after a delete (D10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirrorAfterDelete { Rebuilt, NoMirror, NotNeeded, Failed(String) }

impl HierarchySnapshot {
    /// The switcher row for this tree, so a section can emit `Action::SetScope` (D11).
    #[must_use]
    pub fn summary(&self) -> WorkspaceSummary {
        WorkspaceSummary {
            workspace_id: self.workspace.id, slug: self.workspace.slug.clone(), name: self.workspace.name.clone(),
            projects: self.projects.iter().map(|p| ProjectRef { project_id: p.project.id, slug: p.project.slug.clone(), name: p.project.name.clone(), position: p.link.position }).collect(),
        }
    }
}

/// One read of the whole tree; `None` when the workspace does not exist.
pub async fn snapshot<S: ReadStore + WriteStore + ?Sized>(store: &S, ws: WorkspaceId, this_box: Option<BoxId>) -> Result<Option<HierarchySnapshot>>
```

`snapshot`: `store.workspace(ws)` (`traits.rs:334`) → `None` short-circuits; `workspace_box_paths(ws)`
(`:375`) filtered `box_id == this_box`; `workspace_projects(ws)` (`:361`, already by position) →
per link `store.project(link.project_id)` (`ReadStore`, `:126`; a missing project is skipped, not
an error — the link cascades with the project on Pg, `MemStore` keeps them in step) → `repos(id)`
(`:429`, by name) → per repo `repo_box_paths(repo.id)` (`:442`) filtered to `this_box`.

### 6.2 `serve` (D6, D8, D10)

```rust
/// The twelve names, in `StoreRequest` order; `name()`'s arms and the section's `Failed` match
/// both read from here.
pub const REQUEST_NAMES: [&str; 12] = ["hierarchy", "create_workspace", "update_workspace", "set_workspace_root", "create_project", "update_project", "create_repo", "update_repo", "set_repo_path", "delete_reach", "delete_workspace", "delete_project"];

/// Serves one hierarchy request. `Err(Unreachable)` on `Backend::Offline` (`writer()` is `None`,
/// `backend.rs:150-156`) so `try_serve`'s caller keeps its offline transition.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let writer = backend.writer().ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;
    let this_box = backend.box_info().await?.map(|b| b.box_id);     // `backend.rs:254-260`
    match request {
        StoreRequest::Hierarchy(ws) => Ok(StoreReply::Hierarchy(snapshot(&writer, *ws, this_box).await?.map(Box::new))),
        StoreRequest::CreateWorkspace { slug, name, description } => { let created_by = backend.this_user().await?; let ws = writer.create_workspace(NewWorkspace { id: WorkspaceId::new(), slug, name, description, created_by }).await?; reread(&writer, ws.id, this_box).await }
        StoreRequest::UpdateWorkspace { id, expected, patch } => cas(&writer, *id, this_box, writer.update_workspace(*id, *expected, patch.clone()).await?).await,
        StoreRequest::SetWorkspaceRoot { id, path } => { let canonical = canonical(path).await?; writer.upsert_workspace_box_path(&WorkspaceBoxPath { workspace_id: *id, box_id: box_id(this_box)?, root_path: canonical, updated_at: Utc::now() }).await?; reread(&writer, *id, this_box).await }
        StoreRequest::CreateProject { workspace, slug, name, description } => { create_project; let links = writer.workspace_projects(*workspace).await?; upsert_workspace_project(&WorkspaceProject { workspace_id, project_id, position: i32::try_from(links.len()).unwrap_or(i32::MAX) }); reread(workspace) }
        StoreRequest::UpdateProject { id, expected, patch } => { outcome = update_project; ws = workspace_of(&writer, *id)?; cas-style reply over `ws` }
        StoreRequest::CreateRepo { project, name, remote_url, default_branch, is_primary } => { create_repo(NewRepo { id: RepoId::new(), .. }); reread(workspace_of(project)) }
        StoreRequest::UpdateRepo { id, expected, patch } => { outcome = update_repo; project = outcome row's project_id; ws = workspace_of(project); cas-style reply }
        StoreRequest::SetRepoPath { repo, path } => { canonical; upsert_repo_box_path(RepoBoxPath { repo_id, box_id: box_id(this_box)?, local_path, updated_at: Utc::now() }); reread(workspace_of(project_of(repo))) }
        StoreRequest::DeleteReach(target) => Ok(StoreReply::DeleteReach(writer.delete_reach(*target).await?)),
        StoreRequest::DeleteWorkspace(id) => Ok(StoreReply::Deleted { target: DeleteTarget::Workspace(*id), reach: writer.delete_workspace(*id).await?, mirror: MirrorAfterDelete::NotNeeded }),
        StoreRequest::DeleteProject(id) => { let reach = writer.delete_project(*id).await?; let mirror = match backend.cache() { Some(cache) => match cache.rebuild().await { Ok(()) => Rebuilt, Err(err) => Failed(err.to_string()) }, None => NoMirror }; Ok(Deleted { target: Project(*id), reach, mirror }) }
        other => Err(StoreError::Backend(format!("not a hierarchy request: {}", other.name()))),
    }
}
```

Helpers, all private: `reread(writer, ws, this_box) -> Result<StoreReply>` =
`Hierarchy(Some(Box::new(snapshot(..)?.ok_or(NotFound { entity: "workspace", id })?)))`;
`cas(..)`: `Applied(_)` → `reread`, `Stale(_)` → `HierarchyStale(Box::new(snapshot ..))` (the
worker re-reads the tree, D6); `box_id(this_box) -> Result<BoxId>` =
`ok_or(NotFound { entity: "box", id: "(this box)".to_owned() })` (shape of `backend.rs:174-177`);
`canonical(path: &str) -> Result<String>` =
`tokio::task::spawn_blocking(move || canonical_root(Path::new(&path))).await.map_err(|e| StoreError::Backend(e.to_string()))?.map(|p| p.to_string_lossy().into_owned()).map_err(|r| StoreError::Constraint(r.to_string()))`
— the first `spawn_blocking` in `crates/htui/src` (grep: only `htui-store/src/cache/pending.rs:126,162`).
`workspace_of(writer, project) -> Result<WorkspaceId>`: **no reverse reader exists on the seam** —
resolve it from `backend.workspaces().await?` (`backend.rs:241-247`, every `WorkspaceSummary` with
its `ProjectRef`s) by `projects.iter().any(|p| p.project_id == project)`; `NotFound { entity: "workspace_project" }`
when none links it. `project_of(writer, repo)`: no `repo(id)` reader either — the section knows the
project; **`SetRepoPath` and `UpdateRepo` therefore carry `project: ProjectId` too** (see §7.1; the
plan's field lists are extended by that one field, not changed). `UpdateRepo`'s `Applied(row)`
carries `row.project_id` but `Stale` does too, so the extra field is what makes `SetRepoPath` (which
gets no row back) resolvable — open item O-2 records the alternative.

### 6.3 `DeleteReach` copy helpers (flag A)

```rust
/// One `"{n} {label}"` per non-zero count, in `DeleteReach` field order (`traits.rs:745-792`). The
/// destructuring has no `..`: a field added to the struct fails to compile here rather than being
/// silently left out of the warning (zero-omission rule, PRD D13).
#[must_use]
pub fn reach_parts(reach: &DeleteReach) -> Vec<String> {
    let DeleteReach { workspace_links, workspace_box_paths, items, item_key_counters, item_kinds, step_graphs, phases, phase_agents, prompt_templates, repos, repo_box_paths, skill_bindings, runs, run_steps, session_events, run_step_commits, command_runs, notes, revisions, links, documents } = *reach;
    [
        (workspace_links, "workspace links"), (workspace_box_paths, "box root paths"), (items, "items"),
        (item_key_counters, "key counters"), (item_kinds, "kinds"), (step_graphs, "graphs"), (phases, "phases"),
        (phase_agents, "phase agents"), (prompt_templates, "templates"), (repos, "repos"), (repo_box_paths, "repo paths"),
        (skill_bindings, "skill bindings"), (runs, "runs"), (run_steps, "run steps"), (session_events, "session events"),
        (run_step_commits, "run step commits"), (command_runs, "command runs"), (notes, "notes"), (revisions, "revisions"),
        (links, "links"), (documents, "documents"),
    ].into_iter().filter(|(n, _)| *n > 0).map(|(n, label)| format!("{n} {label}")).collect()
}

/// `(rows, tables)`: the sum of every count and how many are non-zero.
#[must_use]
pub fn reach_totals(reach: &DeleteReach) -> (u64, usize)
```

Unit tests in-module: `reach_parts_omits_zeros_and_keeps_field_order` (a struct with three non-zero
fields renders exactly three parts in field order), `reach_totals_counts_rows_and_tables`.

## 7. `store_worker.rs` edits (T2)

Imports (`:14-30`): add `use chrono::{DateTime, Utc};`; extend the `htui_core::model` list
(`:18-21`) with `ProjectPatch, RepoId, RepoPatch, WorkspaceId, WorkspacePatch`; add
`use htui_core::store::{DeleteReach, DeleteTarget}` to `:22` (open item O-1: confirm the re-export
path in `crates/htui-core/src/store/mod.rs`); add
`use crate::hierarchy::{self, HierarchySnapshot, MirrorAfterDelete};`.

### 7.1 `StoreRequest` (`:56-225`): twelve variants after `ApplyMigrations` (`:224`), before `}`

```rust
    /// The whole tree of one workspace for `Settings > Hierarchy` (MOD-15 milestone 3, D5).
    Hierarchy(WorkspaceId),
    /// `created_by` is the worker's (`Backend::this_user`); the section never holds a `UserId`.
    CreateWorkspace { slug: String, name: String, description: String },
    /// CAS on `updated_at` (M1 D3): `Stale` answers `StoreReply::HierarchyStale`.
    UpdateWorkspace { id: WorkspaceId, expected: DateTime<Utc>, patch: WorkspacePatch },
    /// This box's root, canonicalised or refused (D8). `path` is what the user typed.
    SetWorkspaceRoot { id: WorkspaceId, path: String },
    /// Creates and links at `position = links.len()`.
    CreateProject { workspace: WorkspaceId, slug: String, name: String, description: String },
    UpdateProject { id: ProjectId, expected: DateTime<Utc>, patch: ProjectPatch },
    CreateRepo { project: ProjectId, name: String, remote_url: Option<String>, default_branch: String, is_primary: bool },
    /// `project` is the repo's owner, so the reply can re-read its workspace (§6.2).
    UpdateRepo { project: ProjectId, id: RepoId, expected: DateTime<Utc>, patch: RepoPatch },
    SetRepoPath { project: ProjectId, repo: RepoId, path: String },
    /// Counts before the act; `None` when the target is gone.
    DeleteReach(DeleteTarget),
    DeleteWorkspace(WorkspaceId),
    /// Deletes, then rebuilds the mirror from the worker (D10).
    DeleteProject(ProjectId),
```

Every field is `Debug`-safe (no secret; `TextField` never enters a request). Count 26 → 38.

### 7.2 `name()` (`:230-259`, `const fn`): twelve arms after `Self::ApplyMigrations => "apply_migrations"` (`:257`)

`Self::Hierarchy(..) => "hierarchy"`, `Self::CreateWorkspace { .. } => "create_workspace"`,
`Self::UpdateWorkspace { .. } => "update_workspace"`, `Self::SetWorkspaceRoot { .. } => "set_workspace_root"`,
`Self::CreateProject { .. } => "create_project"`, `Self::UpdateProject { .. } => "update_project"`,
`Self::CreateRepo { .. } => "create_repo"`, `Self::UpdateRepo { .. } => "update_repo"`,
`Self::SetRepoPath { .. } => "set_repo_path"`, `Self::DeleteReach(..) => "delete_reach"`,
`Self::DeleteWorkspace(..) => "delete_workspace"`, `Self::DeleteProject(..) => "delete_project"`.
String literals keep the fn `const`. Test `hierarchy_names_are_stable` (beside
`name_arms_are_stable`, `:1652-1676`) asserts each equals `hierarchy::REQUEST_NAMES[i]`.

### 7.3 `StoreReply` (`:263-367`): four variants before `Failed` (`:361`)

```rust
    /// Answer to `Hierarchy` and to every hierarchy write that applied: the tree as it is now.
    /// `None` only for `Hierarchy` of a workspace that does not exist.
    Hierarchy(Option<Box<HierarchySnapshot>>),
    /// A CAS write found the row changed (M1 D3): the tree as it is now, for the editor to reload.
    HierarchyStale(Box<HierarchySnapshot>),
    /// Answer to `DeleteReach`.
    DeleteReach(Option<DeleteReach>),
    /// Answer to `DeleteWorkspace` / `DeleteProject`: what was removed and what the mirror did.
    Deleted { target: DeleteTarget, reach: DeleteReach, mirror: MirrorAfterDelete },
```

### 7.4 `try_serve` (`:550-599`): one or-ed arm before `StoreRequest::StoreState` (`:594`)

```rust
        StoreRequest::Hierarchy(..)
        | StoreRequest::CreateWorkspace { .. }
        | StoreRequest::UpdateWorkspace { .. }
        | StoreRequest::SetWorkspaceRoot { .. }
        | StoreRequest::CreateProject { .. }
        | StoreRequest::UpdateProject { .. }
        | StoreRequest::CreateRepo { .. }
        | StoreRequest::UpdateRepo { .. }
        | StoreRequest::SetRepoPath { .. }
        | StoreRequest::DeleteReach(..)
        | StoreRequest::DeleteWorkspace(..)
        | StoreRequest::DeleteProject(..) => hierarchy::serve(backend, request).await?,
```

No guard, no wildcard (F-12). The `?` is what keeps `:755-765`'s `go_offline` on an `Unreachable`.

## 8. `tests/hierarchy.rs` — worker half (T2)

Header `#![cfg(feature = "testkit")]` from day one (T3's half needs `Harness`; the gate is
`--all-features`). Imports: `htui::store_worker::{StoreReply, StoreRequest, serve}`,
`htui::hierarchy::{MirrorAfterDelete, REQUEST_NAMES}`, `htui_core::fixtures::ids`,
`htui_core::store::{DeleteTarget, MemStore}`, `htui_store::{Backend, CacheStore, PgStore, testkit}`.
`fn demo() -> Backend { Backend::memory(MemStore::demo()) }`; scope ids `ids::WORKSPACE_GRAPHICS`,
`ids::PROJECT_VULKAN` (`fixtures.rs:136,142`).

| Test | Drives | Asserts |
|---|---|---|
| `a_hierarchy_read_returns_the_demo_tree` | `Hierarchy(WORKSPACE_GRAPHICS)` | `Some`, `workspace.slug == "graphics"`, `this_box == Some(ids::BOX)`, projects by position, `root_path.is_none()` |
| `a_hierarchy_read_of_nil_is_none` | `Hierarchy(WorkspaceId::default())` | `Hierarchy(None)` |
| `create_workspace_answers_its_own_tree` | `CreateWorkspace { slug: "ops", .. }` | `Hierarchy(Some(s))` with `s.workspace.slug == "ops"`, `created_by == ids::USER` |
| `create_project_links_at_the_end` | `CreateProject { workspace: GRAPHICS, .. }` | second `ProjectEntry`, `link.position == 1`, 35 seeded rows visible through a following `DeleteReach` |
| `create_repo_then_p_moves_the_primary` | two `CreateRepo` (first `is_primary: true`), then `UpdateRepo { patch: RepoPatch { is_primary: Some(true), ..Default::default() } }` on the second | exactly one `repo.is_primary` afterwards, and it is the second |
| `a_stale_update_answers_the_current_tree` | `UpdateWorkspace` twice from one `expected` | first `Hierarchy(Some)`, second `HierarchyStale(s)` with `s.workspace.name` = first patch's |
| `a_root_is_stored_canonical_and_a_link_is_refused` | `tempfile::tempdir()`; `SetWorkspaceRoot { path: dir }` then `{ path: dir/"gone" }` and `#[cfg(unix)]` dangling symlink | `root_path.unwrap().root_path == dir.canonicalize()`; `Failed { request: "set_workspace_root", message }` where `message.contains("does not exist on this box")` / `"is a link to nothing"` (flag C) |
| `delete_reach_equals_deleted_reach` | `DeleteReach(Project(PROJECT_VULKAN))` then `DeleteProject(PROJECT_VULKAN)` | `Some(reach) == deleted.reach`, `mirror == NoMirror`, then `Hierarchy(GRAPHICS)` has no project |
| `delete_workspace_keeps_its_projects` | `DeleteWorkspace(WORKSPACE_GRAPHICS)` | `mirror == NotNeeded`, `reach.workspace_links == 1`, `Hierarchy(GRAPHICS)` → `None`, `ReadStore::project(PROJECT_VULKAN)` still `Some` |
| `offline_refuses_every_hierarchy_request_by_name` | `CacheStore::open(root.path(), "hierarchy-offline", PgStore::schema_version())` + `testkit::seed_mirror` (`chat_offline.rs:112-129`), `Backend::Offline { cache, since: Some(Utc::now()) }`, all twelve requests | each `Failed { request, message }` with `request == REQUEST_NAMES[i]` and `message.contains(DATABASE_UNREACHABLE)` |
| `a_box_without_a_row_cannot_set_a_path` | `Backend::memory(MemStore::new())` — no, `MemStore::new()` has no user either; use `demo` with `this_box` cleared is not constructible → **dropped**; covered by unit test of `box_id()` helper in `hierarchy.rs` |

## 9. `HierarchySection` (T3)

File: `crates/htui/src/ui/tabs/settings/hierarchy.rs` (new). Mirrors `agents.rs:248-319`,
`:1117-1207`, `:1209-1269`, `:1271-1295`.

### 9.1 Types (D13, flag J)

```rust
pub struct HierarchySection {
    snapshot: Option<HierarchySnapshot>,
    /// `Some(text)` after `Failed { request: "hierarchy" }`: the pane says "hierarchy needs Postgres".
    unavailable: Option<String>,
    cursor: usize,
    mode: Mode,
    /// The request in flight, by `name()`; a second write is refused until the reply (`probing`, `agents.rs:256-259`).
    busy: Option<&'static str>,
    notice: Option<String>,
}
impl HierarchySection { pub const ID: SectionId = SectionId("hierarchy"); pub fn new() -> Self; }

enum Mode { Browse, Editing(Editor), Deleting { target: DeleteTarget, slug: String, stage: DeleteStage } }
enum DeleteStage { Counting, Warn(DeleteReach), Typed { reach: DeleteReach, field: TextField }, InFlight(DeleteReach) }
struct Editor { kind: EditorKind, fields: Vec<Field>, focus: usize, expected: Option<DateTime<Utc>> }
struct Field { label: &'static str, input: TextField, required: bool }
enum EditorKind { NewWorkspace, EditWorkspace(WorkspaceId), NewProject, EditProject(ProjectId), NewRepo(ProjectId), EditRepo { project: ProjectId, id: RepoId }, WorkspaceRoot(WorkspaceId), RepoPath { project: ProjectId, id: RepoId } }
enum Row { Workspace, Project { index: usize }, Repo { project: usize, index: usize } }
```

All derive `Debug` (`TextField`'s hand-written impl makes `Field`, `Editor`, `Mode` safe).
`captures_input()` = `!matches!(self.mode, Mode::Browse)`.

Editor field lists (label, prefill, required): **NewWorkspace/EditWorkspace** `slug`\*, `name`\*,
`description`; **NewProject/EditProject** same; **NewRepo** `name`\*, `remote_url`,
`default_branch`\* (prefilled `main`), `primary (y/n)`\* (prefilled `y` when the project has no
repo, else `n`); **EditRepo** `name`\*, `remote_url`, `default_branch`\* — no primary (D12);
**WorkspaceRoot/RepoPath** `path`\* (prefilled with the stored path or empty). `expected` is the
row's `updated_at` for the three `Edit*` kinds, `None` otherwise. Submit builds: `UpdateWorkspace`
/`UpdateProject` with every column `Some(text)`; `UpdateRepo` with
`RepoPatch { name: Some, remote_url: Some(empty → None), default_branch: Some, is_primary: None }`;
`CreateRepo.is_primary` from the `y`/`n` field (anything else → notice "`primary (y/n)` is y or n").

### 9.2 Rows (D13)

Flat `Vec<Row>` rebuilt from the snapshot on every reply; `cursor` clamped as `agents.rs:312-319`.
Lines: workspace `{name} ({slug}) — root on this box: {root_path.root_path | "unset"}`; project
`  {slug}  {name}`; repo `    {"*" if is_primary else " "}{name}  {default_branch}  {remote_url | "—"}  {local_path.local_path | "unset"}`.
Cursor row in `theme.accent` (as `accented_lines` reads it, `tests/settings.rs:100-118`).

### 9.3 Key tables

**Browse** (`captures_input` false; `q`, `?`, digits, `Tab`, `ctrl-c`, `-` are the global table's
and `h`/`l`/`[`/`]`/arrows the tab's — `agents.rs:1128-1130`):

| Key | Row | Effect | Returns |
|---|---|---|---|
| `j` / `k` | any | cursor ± 1, no wrap (`agents.rs:294-306`) | `Consumed` |
| `N` | any, or no snapshot | `Editing(NewWorkspace)` | `Consumed` |
| `n` | workspace → `NewProject`; project or repo → `NewRepo(project)` | `Consumed` |
| `e` | workspace / project / repo → `EditWorkspace` / `EditProject` / `EditRepo` | `Consumed` |
| `p` | repo → `UpdateRepo { is_primary: Some(true) }`, `busy = Some("update_repo")`; other rows → notice "`p` wants a repo row" | `Consumed` |
| `b` | workspace → `WorkspaceRoot`; repo → `RepoPath`; project → notice "`b` wants the workspace or a repo row" | `Consumed` |
| `d` | workspace / project → `Deleting { stage: Counting }` + `ctx.request(DeleteReach(target))`; repo → notice "repos are not deleted here" | `Consumed` |
| `r` | any | `ctx.request(Hierarchy(ctx.scope.workspace_id))` | `Consumed` |
| `Esc` | any | `notice = None` | `Consumed` if a notice was showing, else `Pass` |
| `N`/`n`/`e`/`p`/`b`/`d` while `busy.is_some()` | — | notice "`{busy}` is still in flight" | `Consumed` |
| `N`/`n`/`e`/`p`/`b`/`d` while `unavailable.is_some()` or (`snapshot.is_none()` and key ≠ `N`) | — | notice "no workspace — `N` creates one" | `Consumed` |
| anything else | — | — | `Pass` |

**Editor** (`captures_input` true):

| Key | Effect | Returns |
|---|---|---|
| any → `fields[focus].input.on_key` = `Consumed` | field edited | `Consumed` |
| … = `Submit` | every `required` field non-empty else notice "`{label}` is required"; then one request per `EditorKind`, `busy = Some(name)`, editor **stays open** until the reply | `Consumed` |
| … = `Cancel` (`Esc`) | `mode = Browse`, `notice = None` | `Consumed` |
| … = `Pass` and `Tab` / `Down` | `focus = (focus + 1) % len` | `Consumed` |
| … = `Pass` and `BackTab` / `Up` | `focus = (focus + len - 1) % len` | `Consumed` |
| … = `Pass` and modifiers contain `CONTROL` | — (`ctrl-c` reaches the global table) | `Pass` |
| … = `Pass`, anything else | swallowed | `Consumed` |

**Deleting** (`captures_input` true; flag B):

| Stage | Key | Effect | Returns |
|---|---|---|---|
| any | `CONTROL` chord | — | `Pass` |
| `Counting` | `Esc` / `n` | `Browse` | `Consumed` |
| `Warn` | `y` | `Typed { field: TextField::new() }` | `Consumed` |
| `Warn` | `n` / `Esc` | `Browse` | `Consumed` |
| `Typed` | field `Submit`, text `== slug` | `ctx.request(DeleteWorkspace(id) \| DeleteProject(id))`, `busy`, `InFlight` | `Consumed` |
| `Typed` | field `Submit`, text `!= slug` | notice "that is not the slug; nothing was deleted", `field.clear()` | `Consumed` |
| `Typed` | field `Cancel` | `Browse` | `Consumed` |
| `Typed` | field `Consumed` / `Pass` | typed / swallowed | `Consumed` |
| `InFlight` | anything | swallowed | `Consumed` |
| any | anything else | swallowed | `Consumed` |

### 9.4 Hint line per mode (`theme.dim`, `format!("{keys} · {notice}")` when a notice is set, as `agents.rs:973-977`)

| State | Text |
|---|---|
| Browse, snapshot present | `j/k · N workspace · n project/repo · e edit · p primary · b path · d delete · r reload` |
| Browse, `busy` and no notice | same + ` · {busy} in flight` |
| Browse, no snapshot (`Hierarchy(None)`) | `N workspace · r reload` |
| Browse, `unavailable` | `r reload` |
| Editing | `Tab/Shift+Tab field · Enter save · Esc cancel` |
| Deleting Counting | `counting rows… · Esc stop` |
| Deleting Warn | `y continue · n/Esc stop` |
| Deleting Typed | `Enter confirm · Esc stop` |
| Deleting InFlight | `deleting…` |

### 9.5 Pane copy (D7, D9, flags A/I)

Layout `Layout::vertical([Constraint::Min(3), Constraint::Length(pane_len), Constraint::Length(1)])`
(`agents.rs:1271-1294`), `pane_len = 0` in Browse with no notice-only pane.

- Rows pane, no snapshot: ``message(frame, rows, "no workspace — `N` creates one", theme)``;
  `unavailable`: `"hierarchy needs Postgres"` (`agents.rs:1281` family).
- Editor pane: one line per field `{label}: ` + `input.line(width - label_width - 2, focused, theme)`;
  the focused label in `theme.accent`.
- Stale (D7): rows replaced; editor kept, fields untouched; `expected` = current row's
  `updated_at` (looked up by the `EditorKind` id in the new snapshot); id gone → `Browse` with
  notice `"deleted elsewhere while you were editing"`; otherwise notice
  `"changed elsewhere since you opened it — reloaded; Enter retries against the current row"`
  rendered in `theme.error`.
- Delete Warn/Typed, every line `theme.error`:
  - project: ``This deletes project `{slug}` and its entire history. Gone for good: {reach_parts joined ", "}.``
    (empty parts → `Gone for good: no other rows.`; never observed — M2 seeds 35 rows), then
    ``Nothing here can be undone. `y` to continue, `n` or `Esc` to stop.``
  - workspace: ``This deletes workspace `{slug}`: {workspace_links} project links and {workspace_box_paths} box root paths. Its projects survive and stay reachable from other workspaces.``
    then the same second line.
  - Typed adds ``Type `{slug}` to confirm:`` + the field line.
- Counting: `counting rows…`. InFlight: ``deleting `{slug}`…``.
- After `Deleted`: notice ``deleted `{slug}`: {rows} rows across {tables} tables; {mirror}`` with
  `mirror` ∈ `mirror rebuilt` / `no mirror` / `no rebuild needed` / `mirror not rebuilt: {err}`.
- After a path write whose stored string differs from the typed one: notice ``stored as `{canonical}```.

### 9.6 `on_reply`

| Reply | Effect |
|---|---|
| `Hierarchy(Some(s))` | `busy = None`, `unavailable = None`, `snapshot = Some`, rows rebuilt, cursor clamped; if `mode` is `Editing` and `busy` was a write → `Browse`; **scope follow (D11)**: if `s.workspace.id != ctx.scope.workspace_id` or `s.projects.iter().map(\|p\| p.project.id) != ctx.scope.project_ids` → `ctx.emit(Action::SetScope { workspace: s.summary() })` |
| `Hierarchy(None)` | `busy = None`, `snapshot = None`, `mode = Browse` |
| `HierarchyStale(s)` | `busy = None`, rows replaced, editor kept (D7 above); **no** scope follow |
| `DeleteReach(Some(reach))` | `Counting` → `Warn(reach)` |
| `DeleteReach(None)` | notice `already gone`, `Browse`, re-request `Hierarchy` |
| `Deleted { target: Project(_), reach, mirror }` | notice (§9.5), `Browse`, `ctx.request(Hierarchy(scope))` → D11 emits `SetScope` |
| `Deleted { target: Workspace(_), .. }` | notice, `Browse`, `ctx.request(StoreRequest::Workspaces)` |
| `Workspaces(list)` | if `snapshot` is `None` or its workspace is not in `list`: first summary → `Action::SetScope`; empty list → nothing (the shell stays on the dead scope, pane says "no workspace — `N` creates one") |
| `Failed { request: "hierarchy", message }` | `unavailable = Some(message)`, `busy = None` |
| `Failed { request, .. } if REQUEST_NAMES.contains(request)` | `busy = None`; editor/delete stage kept (the status line already shows `{request}: {message}`, `update.rs:132-134`) |
| `_` | `{}` |

`on_scope_change`: `snapshot = None`, `mode = Browse`, `busy = None`, `cursor = 0`; `notice` kept
(so "deleted `x`" survives the scope change it causes). `wants_requests(scope)` =
`vec![StoreRequest::Hierarchy(scope.workspace_id)]`.

## 10. Registration and copy (T3)

- `crates/htui/src/app/mod.rs:14`: `use crate::ui::tabs::settings::{AgentsSection, HierarchySection};`;
  `:47-49`: `SettingsTab::with_sections(vec![Box::new(AgentsSection::new()), Box::new(HierarchySection::new())])`.
- `crates/htui/src/ui/overlay/workspace_switcher.rs:115`:
  ``"no workspaces — `N` in Settings > Hierarchy creates one"``.
- Both snapshots' line 19 (flag D): the box is 51 columns inside; the new copy is 50 chars
  (old 48), so it still fits without moving the frame — re-accept, read the diff is one line each.
- `tests/settings.rs` strip pin: append `Box::new(HierarchySection::new())` (flag H); import it.

## 11. `tests/hierarchy.rs` — section half and snapshots (T3)

Appended under `// ---- section (T3) ----`. Helpers: `async fn hierarchy_over(store: MemStore) -> Harness`
= `Harness::over(store).with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(AgentsSection::new()), Box::new(HierarchySection::new())])))` + `settle`, then `harness.key("l")` + `settle` to reach the section.

| Test | Method | Asserts / snapshot |
|---|---|---|
| `the_demo_tree_renders` | harness | `hierarchy__demo.snap` (Graphics, `vulkan-tutorials`, root `unset`) |
| `q_quits_from_browse` | harness, `q` | `should_quit` |
| `n_on_a_project_opens_the_repo_editor` | harness `j`, `n` | `hierarchy__editor_repo.snap`; `l` while editing types an `l` (frame contains it in the name field), strip unchanged |
| `d_on_a_project_warns_with_counts_then_asks_for_the_slug` | harness `j`, `d`, settle | `hierarchy__delete_warn.snap` (35 seeded rows visible: `5 kinds, 5 graphs, 15 phases, 10 templates` plus the fixture's items); `y` → `hierarchy__delete_typed.snap`; type `vulkan-tutorials`, `enter`, settle → project gone, `top_bar`/`app().projects` empty (SetScope), notice contains ``deleted `vulkan-tutorials``` and `no mirror` |
| `a_wrong_slug_deletes_nothing` | harness | notice, `app().projects.len() == 1` |
| `a_stale_reply_keeps_the_typed_text` | `SectionBench` | open `EditWorkspace` via `e`, type, `bench.reply(HierarchyStale(Box::new(snapshot with a newer updated_at)))`, `render_section` → `hierarchy__stale.snap`; then `enter` → `drained()` holds `Action::Store(UpdateWorkspace { expected: <new>, .. })` |
| `no_workspace_says_so` | `Harness::empty()` + tab | `hierarchy__no_workspace.snap` |
| `offline_says_it_needs_postgres` | `Harness::over_backend(Backend::Offline { .. }).with_store_state("offline · 0s", None)` (E-16) | `hierarchy__offline.snap`; frame contains `hierarchy needs Postgres` |
| `p_moves_the_primary` | `SectionBench` over a snapshot with two repos | `drained()` = `[Store(UpdateRepo { patch: RepoPatch { is_primary: Some(true), .. }, .. })]` |
| `b_on_the_workspace_row_sets_the_root` | `SectionBench` | type `/srv/htui`, `enter` → `Store(SetWorkspaceRoot { path: "/srv/htui" })` — never stat'd |
| `a_reply_from_another_workspace_moves_the_scope` | `SectionBench` | `Hierarchy(Some(s))` with `s.workspace.id != bench scope` → `drained()` contains `Action::SetScope` |

Paths in every frame are `unset`, or the literal `/srv/…` handed in through a `RepoBoxPath`/
`WorkspaceBoxPath` row, never a machine path (D14).

## 12. Data flow — one pass each

Chain for every pass: key → `App::on_key` (`state.rs:330-422`; overlay first, then
`tab.on_key`) → `SettingsTab::on_key` (§2 gate, cycle, delegate) → `HierarchySection::on_key` →
`ctx.request(r)` = `Emit.push(Action::Store(r))` (`state.rs:110-112`) → `drain` →
`dispatch(Origin::Tab("settings"), r)` records `(origin, discriminant(r)) → seq` (`state.rs:269-282`)
→ `RequestEnvelope` → worker loop `other => try_serve` (`store_worker.rs:755`) → §7.4 arm →
`hierarchy::serve` → `Writer` seam call → `Ok(reply)` / `Err` → `failed(name, err)` (`:603-608`) →
`ReplyEnvelope` → `App::on_reply` (`update.rs:124-196`): `observe_reply` (`_ => {}`), `is_fresh`
(`state.rs:290-294`), `Failed` → `Action::Error("{request}: {message}")` (`:132-134`), then
`tab.on_reply` (`:151-161`) → `SettingsTab::on_reply` (every section, `settings/mod.rs:215-219`) →
`HierarchySection::on_reply` (§9.6) → `drain` (`:164`) applies what the section emitted → next
frame `render` (§9.5).

1. **Create** (`N`, workspace): Browse `N` → `Editing(NewWorkspace)` (`captures_input` now true) →
   typed keys hit `fields[focus].input` → `Enter` → `Submit` → required check →
   `ctx.request(CreateWorkspace { slug, name, description })`, `busy = Some("create_workspace")`
   → worker: `writer()` (`Memory`/`Online`), `this_user()` (`backend.rs:172-181`) →
   `create_workspace(NewWorkspace { id: WorkspaceId::new(), .., created_by })` (`traits.rs:314`)
   → `snapshot(new id)` → `Hierarchy(Some(s))` → section: `busy = None`, `Browse`, rows;
   `s.workspace.id != ctx.scope.workspace_id` → `emit(SetScope { workspace: s.summary() })` →
   `set_scope` (`update.rs:89-104`): scope, projects, `on_scope_change` on every tab (section drops
   snapshot, keeps notice), overlays closed, `activate_tab` re-issues `wants_requests` →
   `Hierarchy(new id)` → second `Hierarchy(Some)` equals the scope → no emit; loop ends.
   `n` on a project row is the same with `CreateProject { workspace: snapshot.workspace.id, .. }`
   (`create_project` + `upsert_workspace_project` at `links.len()`) and the scope follows because
   `project_ids` changed.
2. **Edit + CAS miss** (`e`): `Editing(EditWorkspace(id))`, `expected = Some(row.updated_at)`,
   fields prefilled → `Enter` → `UpdateWorkspace { id, expected, patch }` → worker
   `update_workspace` (`traits.rs:323-328`) → `CasOutcome::Stale(_)` → `snapshot(id)` →
   `HierarchyStale(Box<s>)` → section: rows replaced, editor kept with its text, `expected` = the
   current row's `updated_at`, notice in `theme.error` (§9.5), `busy = None`; user presses `Enter`
   again → same request with the new token → `Applied` → `Hierarchy(Some)` → `Browse`. No
   automatic retry (PRD D8). No `SetScope` on the stale path.
3. **Path set** (`b` on the workspace row): `Editing(WorkspaceRoot(id))`, field prefilled →
   `Enter` → `SetWorkspaceRoot { id, path }` → worker: `box_info()` → `box_id` or
   `NotFound { entity: "box", id: "(this box)" }` → `spawn_blocking(canonical_root)` →
   `Err(refusal)` → `StoreError::Constraint(refusal.to_string())` → ``Failed { request: "set_workspace_root", message: "constraint violated: `/x` is a link to nothing" }``
   → status line (`update.rs:132-134`) + section `busy = None`, editor kept; or `Ok(canonical)` →
   `upsert_workspace_box_path` (`traits.rs:369`) → `Hierarchy(Some)` → `Browse`, notice
   ``stored as `{canonical}``` when it differs. `b` on a repo row: `SetRepoPath { project, repo, path }`
   → `upsert_repo_box_path` (`:436`) → same.
4. **Primary move** (`p` on a repo row): no editor; `UpdateRepo { project, id, expected: repo.updated_at, patch: RepoPatch { is_primary: Some(true), ..Default::default() } }`,
   `busy` → `update_repo` (`traits.rs:418-423`; M1 D10 demotes the other primary in the same
   transaction) → `Applied` → `snapshot(workspace_of(project))` → `Hierarchy(Some)` → rows show `*`
   moved. `Stale` (someone edited the repo meanwhile) → `HierarchyStale` → Browse keeps rows, notice
   `changed elsewhere … Enter retries` is **not** shown (no editor); notice `reloaded; press p again`.
5. **Delete, two stages** (`d` on a project row): `Deleting { target: Project(id), slug, stage: Counting }`,
   `ctx.request(DeleteReach(Project(id)))` → `delete_reach` (`traits.rs:594`) →
   `DeleteReach(Some(reach))` → `Warn(reach)`: pane copy from `reach_parts` (§6.3), hint
   `y continue · n/Esc stop` → `y` → `Typed { reach, field }` (prompt + field) → typed slug →
   `Enter` → equal → `DeleteProject(id)`, `InFlight(reach)`, `busy` → worker `delete_project`
   (`:608`) → `backend.cache()` (`backend.rs:226-231`) → `rebuild()` (`cache/mod.rs:176-195`) or
   `NoMirror` → `Deleted { target, reach, mirror }` → section: notice
   ``deleted `{slug}`: {rows} rows across {tables} tables; {mirror}``, `Browse`,
   `ctx.request(Hierarchy(scope))` → `Hierarchy(Some)` without the project → `project_ids` differ
   → `SetScope` → Backlog's `on_scope_change` drops the deleted project's rows. Workspace: same
   until `Deleted { target: Workspace, mirror: NotNeeded }` → `ctx.request(Workspaces)` →
   `Workspaces(list)` → first remaining → `SetScope`; empty → pane "no workspace — `N` creates one".
6. **Scope follow** from elsewhere: the switcher's `Enter` → `SetScope` (`workspace_switcher.rs:169`)
   → `set_scope` → `on_scope_change` (snapshot dropped, `Browse`) → `wants_requests` →
   `Hierarchy(ws)` → the reply's ids equal the scope's → no emit. A `Hierarchy` read overtaken by a
   second one (scope changed twice) is dropped by `is_fresh` (same origin, same discriminant); a
   write's `Hierarchy` reply is keyed by the **write's** discriminant and is never dropped by a
   concurrent read (E-11).

## 13. Build order and commit sequence

T1 ∥ T2 (file-disjoint; **gates serialized**, plan F-29), then T3, then T4. Every step's first
failing test is named; every gate runs on the committed tree with nobody else mid-write.

| # | Task | Commit | First red test → where | Compiles after | Gate |
|---|---|---|---|---|---|
| 1 | T1 | `feat(htui): TextField, the single-line field with a window and a mask (MOD-15 M3 D1)` | `text_field::tests::inserts_at_the_cursor` in `ui/text_field.rs` (module registered in `ui/mod.rs` first, so the file compiles empty) | yes | `cargo test -p htui text_field` |
| 2 | T1 | `feat(htui): SettingsSection::captures_input gates the strip cycle (D2)` | `tests/settings.rs::a_capturing_section_receives_l_and_a_plain_one_cycles` (red: `l` cycles) | yes (default body) | `cargo test -p htui --all-features --test settings` |
| 3 | T1 | `test(htui): SectionBench moves into testkit; the strip width is pinned (D4, D15)` | `the_section_strip_fits_the_frame` is green on write (a pin); the move is red until the import lands | yes | T1 gate: `cargo test -p htui --all-features` (five `settings__agents_*` byte-identical), `cargo clippy -p htui --all-targets --all-features -- -D warnings`, `cargo doc -p htui --no-deps` |
| 4 | T2 | `feat(core): canonical_root refuses or canonicalises a per-box root (MOD-15 M3 D8)` | `root_path::tests::a_relative_path_is_refused_first` | yes | `cargo test -p htui-core` |
| 5 | T2 | `feat(htui): hierarchy snapshot, twelve requests and four replies served off the UI task (D5, D6, D10)` | adding the variants first is the red: `name()` and `try_serve` fail E0004; then `tests/hierarchy.rs::a_hierarchy_read_returns_the_demo_tree` | after §7 | `cargo test -p htui --all-features --test hierarchy` |
| 6 | T2 | `test(htui): worker cases for CAS miss, path guard, delete reach and offline refusal` | the remaining rows of §8 | yes | T2 gate on the **merged** tree: `cargo test -p htui-core`, `cargo test -p htui --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 7 | T3 | `feat(htui): Settings > Hierarchy — browse, edit, paths, primary (D11, D12, D13)` | `tests/hierarchy.rs::the_demo_tree_renders` (red: no section) | after §9–10 | `cargo test -p htui --all-features --test hierarchy`; `.snap.new` read, then accepted |
| 8 | T3 | `feat(htui): two-stage delete with counts, typed slug and the worker-side rebuild (D7, D9)` | `d_on_a_project_warns_with_counts_then_asks_for_the_slug` | yes | same |
| 9 | T3 | ``feat(htui): the switcher names `N` in Settings > Hierarchy (D4)`` | `shell_empty` and `switcher_empty` red until re-accepted | yes | T3 gate: `cargo test -p htui --all-features` (7 new + 2 moved snapshots, nothing else in `snapshots/` changed — `git status crates/htui/tests/snapshots crates/htui/src/snapshots`), `--demo` smoke (plan T3) |
| 10 | T4 | `docs(mod-15): milestone 3 landed` | — | — | `cargo doc --workspace --no-deps`; `git diff --stat` = `HANDOFF.md`, the PRD |

Implementers commit each step before the next (uncommitted subagent work dies with the session).
Before blaming Postgres in any gate: `df -h /`.

## 14. Hazards

- **E-1 Exhaustive matches when 12 + 4 variants land.** Must change: `StoreRequest::name()`
  (`store_worker.rs:230-259`, exhaustive `const fn`, +12 arms) and `try_serve` (`:551-599`,
  exhaustive, +1 or-ed arm). Compile untouched, verified by wildcard: `spawn_with` loop
  `other => try_serve` (`:755`); `AgentRuntime::serve` `other =>` (`agent_worker.rs:644`);
  `Harness::drive` `(request, _) =>` (`testkit.rs:227`) and `settle` (`:368`); reply sites
  `update.rs:223`, `agents.rs:1267`, `chat/mod.rs:524`, `workspace_switcher.rs:177` (let-else),
  `migration_prompt.rs:104`, `backlog/mod.rs:113,226`, the detail panes (`imports_from` edges in the
  graph, all `if let`/`let-else`), `agent_worker.rs:5123,5671` (tests, non-exhaustive patterns).
  Proof is `cargo check -p htui --all-targets --all-features` after §7.1/§7.3, before §7.2/§7.4.
- **E-2 `settings/mod.rs` is T1's and T3's.** T1 touches `:42-58` (trait) and `:197-213` (`on_key`);
  T3 touches `:11` and `:26` only. Disjoint hunks; T3 starts from T1's commit (serial anyway).
- **E-3 `tests/hierarchy.rs` is T2's then T3's.** T2 creates it with `#![cfg(feature = "testkit")]`
  and the worker half; T3 appends below a divider and never renames a T2 test. Without the cfg
  line T3's `Harness` import breaks a non-`testkit` `cargo test -p htui`.
- **E-4 No cross-referenced-test-name gate for `htui`.** `every_cross_referenced_test_name_exists`
  scans only `conformance.rs`, `mem.rs`, `pg_criteria.rs` (`crates/htui-agent/src/conformance.rs:3783-3848`;
  the M2 blueprint §8 says so); `grep -rn read_to_string crates/htui/tests crates/htui/src/testkit.rs`
  finds no doc scanner. Doc comments in `htui` may name tests freely; `broken_intra_doc_links = deny`
  still applies to `` [`..`] `` links, so ``[`Harness::settle`]``-style links must resolve.
- **E-5 Snapshot acceptance.** `insta` writes `.snap.new`; read every one, then
  `cargo insta accept` or rename, never `INSTA_UPDATE=always`. Expected delta: `tests/snapshots`
  45 → 52 files, plus one-line changes in the two switcher snapshots (flag D); `git diff --stat`
  on `settings__agents_*` must be empty.
- **E-6 Shared `target/`.** T1 gates first; T2 re-runs its full gate on the merged tree; no gate
  result from a half-written tree counts. One `cargo` lock, one disk — `df -h /`.
- **E-7 `name()` is `const fn`.** New arms are string literals; no `format!`.
- **E-8 `Constraint` prefix (flag C).** Status-line assertions use `contains`.
- **E-9 Modal key rule (flag B).** Deleting swallows unlisted keys; `CONTROL` chords pass. Test
  `q_quits_from_browse` plus a `q` inside `Typed` that does **not** quit.
- **E-10 `SetScope` loop.** Terminates because the second `Hierarchy` reply's ids equal the scope;
  `HierarchyStale` never emits. `on_scope_change` drops an open editor (D5) — keep `notice`.
- **E-11 Staleness index.** Keyed by `(Origin, Discriminant<StoreRequest>)` (`state.rs:158, 273`);
  a `CreateProject` reply is never dropped by a later `Hierarchy` read. `busy` is what stops two
  `UpdateRepo`s racing; `r` is allowed while busy.
- **E-12 `Bench` rename.** 46 `Bench::new` sites; pruned imports or `-D warnings` fails.
- **E-13 `missing_debug_implementations`.** `SectionBench`, `HierarchySection`, `Editor`, `Mode`
  derive `Debug`; `TextField` is hand-written (§1.1) so the derives never print text.
- **E-14 `missing_docs` / `unused_qualifications`.** Every `pub` item in `text_field.rs`,
  `hierarchy.rs`, `root_path.rs`, `testkit.rs` additions carries a doc line; import `PathBuf`
  rather than writing `std::path::PathBuf` inline.
- **E-15 Snapshot names (flag G).** Bare macro names.
- **E-16 Offline frame.** `TempDir` must outlive the harness; the status line carries
  `hierarchy: store unreachable: this box browses its read-only cache and starts no run`
  (`writer.rs:638`) — deterministic, part of the snapshot.
- **E-17 Invisible cursor.** A cursor at the end is a styled space; frames show nothing; assert
  style through `Buffer` (`tests/settings.rs:100-118` pattern).
- **E-18 First `spawn_blocking` in `htui`.** `JoinError` → `StoreError::Backend`; works on
  `#[tokio::test]`'s current-thread runtime.
- **E-19 Offline transition.** `Unreachable` from `hierarchy::serve` on `Online` flips the worker
  offline (`:760-761`, wanted); on `Offline`, `go_offline` is a no-op (`went_offline` false,
  `:879-881`). On `Memory` it never fires (`writer()` is `Some`).
- **E-20 `MemStore::new()` has no user.** `CreateWorkspace` over `Harness::empty()` fails with
  `app_user "(none loaded)" not found` (`backend.rs:174-177`); `hierarchy_no_workspace` is
  render-only. The smoke script uses `--demo`.
- **E-21 Strip pin second touch (flag H).**
- **E-22 `SectionBench::render_section` (flag F)** — a sixth method, stated.
- **E-23 `CreateProject` is two seam calls, not one transaction.** A failure between
  `create_project` and `upsert_workspace_project` leaves an unlinked project; accepted by the plan
  (no seam change). The section's `Failed` keeps the editor open; `r` shows nothing new. Recorded
  for close-out.
- **E-24 First `Hierarchy` before the first `Workspaces`.** Scope nil → `Hierarchy(None)` →
  "no workspace" for one frame, then `SetScope` re-reads. `settle` drains both; fine.
- **E-25 `workspace_of`/`project_of` (§6.2).** No reverse readers on the seam; `backend.workspaces()`
  resolves a project's workspace, and the request carries the repo's project. A project linked to
  two workspaces resolves to the first by name — the section always passes the scope's, so this is
  cosmetic for the reply tree.

## 15. What must NOT change

`crates/htui-core/src/store/**` (no seam method; `CASES.len() == 36`,
`crates/htui-core/tests/mem_store.rs:35-41`), `crates/htui-store/**` (no query; `EXPECTED_CASES == 36`,
`crates/htui-store/tests/pg_conformance.rs:19`; `.sqlx/` byte-identical; `cargo sqlx prepare --check`
still passes), `migrations/`, `crates/htui-agent/**`, `crates/htui/src/ui/tabs/chat/composer.rs`
(D3), `crates/htui/src/app/action.rs`, `crates/htui/src/keymap.rs`, the five
`crates/htui/tests/snapshots/settings__agents_{demo,empty,probed,quota,unknown_row}.snap`, every
other existing `.snap` except the two switcher ones (flag D). No `updated_at` is set by hand (the
two path upserts pass `Utc::now()` into a field the store's trigger overwrites — `traits.rs:363-364`).
No view holds a `UserId` or `BoxId`.

## 16. Open items for the implementer (resolve by reading, never by guessing)

- **O-1** The re-export path of `DeleteReach`, `DeleteTarget`, `CasOutcome`, `WriteStore` from
  `htui_core::store` — read `crates/htui-core/src/store/mod.rs`; the conformance suite's imports show it.
- **O-2** §6.2's `project: ProjectId` on `UpdateRepo`/`SetRepoPath`: confirm no `repo(id)` reader
  exists on `ReadStore`/`WriteStore` (`traits.rs` grep for `fn repo(`); if one landed since
  `e226d3a`, drop the extra field and resolve worker-side.
- **O-3** `MemStore::create_project`'s `created_by` check — whether it refuses an unknown user
  (`mem.rs`, the `create_workspace` arm); decides whether E-20 is a `Constraint` or a `NotFound`
  text in the smoke script.
- **O-4** Whether `MemStore::upsert_workspace_box_path` honours or overwrites the passed
  `updated_at` (`mem.rs`); affects nothing rendered, but the worker test asserting
  `root_path.updated_at` must not pin a clock.
- **O-5** The exact demo item count of `vulkan-tutorials` for `hierarchy__delete_warn.snap`
  (`fixtures.rs` items by project) — read from the accepted snapshot, not typed by hand.
