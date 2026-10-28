# MOD-115 M4 blueprint (code-architect)

Companion to `mod-115-tui-shell-m4.plan.md` (CONFIRMED 2026-10-27). Where the two differ, this file wins, because it
comes from reading the code; every difference is a `PLAN DEVIATION` at the end, with evidence and a proposed
amendment. Paths are under `crates/htui/src/` unless they start with `crates/`, `docs/` or `tests/`
(`tests/` = `crates/htui/tests/`). Line anchors are for `5fcc625a` (`hr/MOD-115`).

## Conventions (M1-M3, unchanged)
- Every module opens with a `//!` doc. Pure geometry is `#[must_use]`. Styles come from `Theme` roles only, each with a
  monochrome test. Labels arrive resolved from `Keys`; glyph widths through `cells::cell_width`.
- Tests live in the file they cover (or a lane's new test file), are named as sentences, and are written **first**.
- Gate serially (`--test-threads=1`), `--all-features` for `tests/*.rs` (without `testkit` they run 0 tests and pass),
  plus featureless `cargo clippy --workspace -- -D warnings`.
- A ported view: `Pane::new(..).focus(ctx.pane_focus(id)).accent(ctx.accent())`, content drawn into `Pane::inner`
  first when the pane needs a number only the content knows (M3 C3); statuses `kit::status`/`kit::label::Mark`; counts
  `kit::label::plural`; empty states `kit::empty` with a `NextStep`; notices `kit::text::notice`.
- Every lane runs the **whole** `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast` before its
  merge (M3 D-8) and accepts snapshots only by name from its own set (§8).

## Decisions taken here (what the plan left open)
- **C1. Homes (E1).** `Scroll`, `Step`, `PAGE` and `window` go to a new `ui/kit/scroll.rs` ("how a pane's content
  moves"); `capture_area` to `ui/kit/pane.rs` (pane geometry, beside `Pane::inner`); `STAMP` to `ui/kit/text.rs`.
  `queue_sentence` and `DEMO_NOTHING_ADMITTED` go to a new `ui/queue_status.rs`: they speak of `QueueWrite`/
  `QueueView` (store-worker domain types) and the kit names no domain type, so not the kit (D-23).
- **C2. The split keeps `impl Tab for BacklogTab` whole in `mod.rs`** (D-8). Helpers move as `impl BacklogTab` blocks
  in child modules with `pub(super)`; children read the parent's private fields and types without any change of
  visibility on the struct.
- **C3. One overlay rule (E6): a dimmed backdrop.** `Modal::render` restyles every cell of the area it is centred over,
  outside its box, with a new `Theme::backdrop` role (`add_modifier(DIM)`, `remove_modifier(BOLD)`, both themes)
  before it clears and draws the box. Justification below (§3.1); it moves **no** snapshot text, so the M3 modals
  (`leave`, `waiting_list`, `path_picker`) and their snapshots are untouched.
- **C4. The pane that holds the keyboard is the focused pane** (L-req repair 1's rule, now for every list + detail
  tab): an open filter form → the Filter pane; an open item form or a capturing sub-tab or Skills prompt/editor → the
  detail; otherwise `ctx.focus()`. Each pane is `Focused` only when it is that pane and `!covered` (via
  `ctx.pane_focus`). Hints are drawn on that pane only (Q2).
- **C5. Detail title and chips share the top border with the title first** (D-4, Q1): T2 adds
  `Pane::title_first()`; the Backlog detail uses it so `KEY · title` survives at 100 columns and the chips window
  around the active one with `‹`/`›`.
- **C6. The in-pane editor frame and the compose areas use a kit titled rule** (`kit::rule`), not a full pane (D-14):
  the editor is drawn inside its host pane's claimed rect, and the PTY geometry (`pane_size`, height − 1) stays.
- **C7. Typed and yes/no questions use one kit part, `kit::modal::Ask`** (E3): the abort question, the migration
  prompt, the Runs close-out (both stages) and the Runs cancel question. A modal's keys are on its bottom border; the
  host pane draws no hint while it is up.
- **C8. `Tab::key_hint` survives** (E8): its one caller after T3 is the shell's activity pane, focused (M3 C2).
- **C9. Shared CLI text stays the CLI's**: `concepts::format_hit` and `concepts::report_line` print for
  `htui --search-items`/`--index-items` (`state_cmd.rs:420`, its 12 in-crate snapshots); the concepts overlay builds
  its own rows and index line (D-19).

## 1. T0: neutral homes (serial, primary tree; no snapshot moves)

### 1.1 New and changed kit items
| Item (today) | New home | Signature |
|---|---|---|
| `detail::PAGE` (`backlog/detail/mod.rs:46`, private) | `ui/kit/scroll.rs` | `pub const PAGE: isize = 10;` |
| `detail::Scroll` + impl (`:522-584`) | `ui/kit/scroll.rs` | unchanged (`offset`, `skip`, `at`, `reset`, `apply`, `follow`) |
| `detail::Step` + impl (`:455-507`) | `ui/kit/scroll.rs` | unchanged (`of`, `delta`, `by_focus`) |
| `list::window` (`backlog/list.rs:238-249`) | `ui/kit/scroll.rs` | `#[must_use] pub fn window(cursor: usize, len: usize, height: usize) -> usize` (gains `#[must_use]`) |
| `detail::capture_area` (`:509-520`) | `ui/kit/pane.rs` (after `Pane::inner`'s impl block, as a free fn) | `#[must_use] pub fn capture_area(areas: &PaneAreas, area: Rect, capturing: bool) -> (Option<Rect>, Rect)` |
| `detail::STAMP` (`:48-50`) | `ui/kit/text.rs` | `pub const STAMP: &str = "%m-%d %H:%M";` (doc names both the detail pane and Requirements) |
| `backlog::queue_sentence` (`backlog/mod.rs:1246-1315`) + `DEMO_NOTHING_ADMITTED` (`:1243-1244`) | new `ui/queue_status.rs`, `pub(crate) mod queue_status;` in `ui/mod.rs` (alphabetical, after `path_picker`) | `pub(crate) fn queue_sentence(write: &QueueWrite, key: &str, view: &QueueView, pause: Option<&str>) -> String`; `pub(crate) const DEMO_NOTHING_ADMITTED: &str` |

Tests that move with them (bodies unchanged):
- `kit/scroll.rs`: `by_focus_routes_only_motion_away_from_the_list`, `apply_steps_and_clamps`,
  `the_list_motions_are_the_detail_steps`, `a_step_scrolls_a_row_or_to_an_end` (`detail/mod.rs:599-616`, `:652-678`,
  `:747-783`).
- `kit/pane.rs`: `capture_area_gives_a_capturing_detail_the_area_only_when_zoomed_away` (`:618-650`).
- `ui/queue_status.rs`: `queue_sentence_covers_every_write`, `a_rebound_pause_key_is_the_one_the_stale_sentence_names`
  (`backlog/mod.rs:1376-1575`).
- `detail/mod.rs` keeps `Plain`, `Converted`, `Stepper` and its two registry tests; its test `use` drops `Scroll`
  and imports `Step` from the kit.
- `kit/mod.rs`: `pub mod scroll;` (alphabetical), and the module doc's part list gains "scrolling".

### 1.2 Every importer (from `git grep` over `src/` and `tests/`, not sampled)
| File:line | Today | Becomes |
|---|---|---|
| `ui/tabs/requirements/mod.rs:55` | `backlog::detail::{Scroll, Step, capture_area}` | `kit::{pane::capture_area, scroll::{Scroll, Step}}` |
| `ui/tabs/requirements/mod.rs:56` | `backlog::list::window` | `kit::scroll::window` |
| `ui/tabs/requirements/detail.rs:21` | `backlog::detail::STAMP` | `kit::text::STAMP` |
| `ui/tabs/skills/agent_help.rs:33` | `backlog::detail::Scroll` | `kit::scroll::Scroll` |
| `ui/tabs/skills/library.rs:51`, `templates.rs:37` | `backlog::detail::{Scroll, Step, capture_area}` | as Requirements |
| `ui/overlay/queue.rs:27` | `backlog::queue_sentence` | `crate::ui::queue_status::{DEMO_NOTHING_ADMITTED, queue_sentence}`; its own copy `:67-69` is deleted (the "copied so the two views stay independent" reason ended with the neutral home) |
| `ui/tabs/backlog/mod.rs:53-56` | `detail::{…, Step, capture_area}` | `detail::{…}` without them + `kit::{pane::capture_area, scroll::Step}` |
| `ui/tabs/backlog/mod.rs:1371` (tests) | `detail::{DetailId, DetailTab, Step}` | `detail::{DetailId, DetailTab}` + `kit::scroll::Step` |
| `ui/tabs/backlog/mod.rs:614` | `queue_sentence(` (same module) | `crate::ui::queue_status::queue_sentence` imported |
| `ui/tabs/backlog/detail/mod.rs` | defines them | imports `use crate::ui::kit::scroll::Step;` (the `DetailTab::step` signature, `DetailRegistry::step`); loses the `Act`/`PaneId`/`PaneAreas` imports it then no longer uses |
| `detail/body.rs:26`, `detail/prompt.rs:24` | `detail::{DetailId, DetailTab, Scroll, Step, message}` | `detail::{DetailId, DetailTab, message}` + `kit::scroll::{Scroll, Step}` |
| `detail/documents.rs:62`, `detail/requirements.rs:66` | `…, Scroll, message}` | `kit::scroll::Scroll` |
| `detail/notes.rs:54`, `detail/runs.rs:86` | `…, STAMP, Scroll, message}` | `kit::{scroll::Scroll, text::STAMP}` |
| `detail/graph.rs:26-27`, `:779` | `detail::{…, PAGE, message}`, `backlog::list`, `list::window(` | `kit::scroll::{PAGE, window}`; the `list` import goes if unused |
| test-module `use crate::ui::tabs::backlog::detail::Step;` at `documents.rs:836`, `graph.rs:1325`, `notes.rs:490`, `requirements.rs:1213`, `runs.rs:2291` | | `use crate::ui::kit::scroll::Step;` |
- The five `fn step(&mut self, step: super::Step, …)` impls (`documents.rs:493`, `graph.rs:696`, `notes.rs:332`,
  `requirements.rs:570`, `runs.rs:1771`) keep compiling through `detail/mod.rs`'s own `use … Step`; T0 rewrites
  them to `Step` with a `kit::scroll::Step` import anyway, so no path through `detail` survives.
- `tests/` imports none of these (checked: no hit for `Scroll`, `Step`, `STAMP`, `capture_area`, `window`,
  `queue_sentence` under `tests/`).
- Doc links naming the old paths (`settings/mod.rs:11` names `detail::DetailTab`, which stays) are unaffected;
  `list.rs`'s module doc keeps "the list pane" and loses `window`'s paragraph.

### 1.3 T0 tests and gate
- No new behaviour; the moved tests are the tests. Add one: `kit::scroll` `window_keeps_the_cursor_in_view_and_never_past_the_end`
  (it had none: `len <= height → 0`; cursor centred; clamped to `len - height`), written first against the old
  function's behaviour.
- Gate: `cargo build -p htui`, featureless clippy, `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`,
  `cargo insta test --all-features --check` unchanged. Test count: unchanged (+1).

## 2. T1: `backlog/mod.rs` split (serial, primary tree; a pure move)

### 2.1 Production map (`mod.rs` after T0: 1-1356 production, tests from `mod tests`)
| New file | Moves (by symbol, current lines) | Notes |
|---|---|---|
| `backlog/mod.rs` (stays) | module doc 1-30; `pub mod detail; divergence; filter; item_form; list;` + `mod hints; mod navigation; mod queue; mod writes;`; imports (trimmed to what is left); `BACKLOG_LAYOUT` 65-73; `ITEMS_READ` 87-88; `REFRESHES_PER_RUNS_POLL` 90-92; `QueueIntent` 124-136; `PendingReveal` 138-148; `BacklogTab` 150-196; `Opening` 198-208; `impl Default` 210-214; `impl BacklogTab { ID, body_areas 231-273, new 275-303, item 305-311, selected_item 313-320 }`; **`impl Tab for BacklogTab` 782-1241 verbatim**; `not_in_this_backlog` 1342-1346; `#[cfg(test)] fn panes` 1348-1356; `#[cfg(test)] mod tests;` | the state and the tab; `body_areas` stays beside `render`, its only caller, which B1 rewrites |
| `backlog/navigation.rs` | `ITEM_CURSOR`, `PROJECT_CURSOR` 75-80; `HOPS` 82-85; `impl BacklogTab { go 322-346, read_item 348-363, rows 365-371, step 373-382, jump 384-389, index 391-394, fold 396-410, reselect 412-426, select_item 428-432, focus_runs 434-453 }`; `parse_cursor` 1328-1340 | selection, cursor format; consts and `parse_cursor` `pub(super)` (read by `location`/`restore` in `impl Tab`) |
| `backlog/queue.rs` | `NO_ITEM_TO_QUEUE` 121-122; `impl BacklogTab { toggle_queued 537-556, on_queue 558-590, on_queue_written 592-620 }` | the queue toggles and their status line (`queue_sentence` is `ui::queue_status`'s since T0) |
| `backlog/writes.rs` | `impl BacklogTab { on_form_key 455-479, apply 481-492, open_new 494-518, open_edit 520-535, on_item_form_key 622-645, on_item_form 647-677, on_item_written 679-713, on_item_diverged 715-727, on_item_failed 729-779 }` | the filter form's apply and the item writes |
| `backlog/hints.rs` | `LIST_HINT` 94-103; `LIST_SEP` 105-106; `styled_spans` 108-119; `impl BacklogTab { list_hint 220-229 }`; `replay_miss` 1317-1326 | the key hints; named `hints`, not `keys`, so no `keys::` path inside `backlog` can be read as `crate::keys` |
Rules:
- Every moved fn and const becomes `pub(super)` (they were private to `mod.rs`); nothing else changes, docs move with
  their items verbatim. `#[must_use]`/attributes move with them.
- Each child `use`s only what it needs (`super::{BacklogTab, …}`, `crate::…`); `use super::*` is not used in
  production files (clippy `wildcard_imports` stays quiet).
- `Tab::reveal(self, …)` in `on_item_written` (`writes.rs`) needs `use crate::ui::tabs::registry::Tab;`.
- **`keys/views.rs` is not edited**: its source guards `include_str!` `ui/tabs/backlog/mod.rs` (`views.rs:1121`) and
  delegate `BACKLOG_BODY/GRAPH/DOCS/NOTES/PROMPT/REQS` to it (`:1288-1293`). Every act those stacks need from the
  Backlog is named in `on_key` (`Act::ListDown` … `Act::BacklogReplay`), which stays in `mod.rs`; every stack named
  only in a moved fn is named elsewhere too (`views::BACKLOG_FILTER` by `key_stack` and `filter.rs`,
  `views::ITEM_FORM` by `item_form.rs`, `views::RUNS_BROWSE` by `runs.rs`). If `cargo test -p htui keys::views`
  fails after the move, add the new files to `VIEWS` (`source!(…)`) and nothing else.
- The `#[cfg(test)] fn panes` stays immediately above `#[cfg(test)] mod tests;` (the `views.rs` `production()` split
  reads up to the first `\n#[cfg(test)]`).

### 2.2 Tests: `backlog/mod.rs` `mod tests` → `backlog/tests/`
`backlog/tests/mod.rs`: the `use` block (1360-1374, minus what T0 moved) and every shared fixture and probe:
`platform` (2018-2045), `Bench` + impl (2165-2219), `items_reads`, `press`, `done_only` (2220-2245), `queue_requests`
(1577-1592), `CapturingProbe` + impl (1820-1850), `MouseProbe` + impl + `Pointed`/`Lost` + `mouse_probe` + `click`
(2279-2345), `drawn` (2968-2989), `runs_reads`/`feat_1_review` (2682-2701), `with_status` (3099-3111), `sent`, `served`,
`ctrl`, `type_into`, `open_with`, `saved`, `notice`, `errors` (3276-3338), `an_edit_saved`, `per_item_reads`,
`a_mint_saved`, `in_the_view`, `view`, `a_project_moved`, `to_body` (where used), `OutcomeProbe`, `ComposeProbe`,
`only` (4386-4448), `DEFAULT_LIST_HINT`/`row_at` (4555-4566), `StepProbe` + types + `stepping`, `drawn_through`,
`first_project` (4705-4783); then `mod filter; mod hints; mod input; mod navigation; mod queue; mod render; mod writes;`.
A helper used by one child only may live in that child; the implementer lets the compiler decide (unused-in-mod.rs
helpers move down), never the reverse. Children open with `use super::*;` (test code; glob is the tests' existing
style).
| File | Tests (by current line) |
|---|---|
| `tests/queue.rs` | 1597, 1655, 1713, 1776 |
| `tests/input.rs` | 1851 (`a_capturing_sub_tab_gets_the_navigation_keys`), 2062, 2107, 2348, 2389, 2416, 2840, 2937, 2953 |
| `tests/navigation.rs` | 1953, 2048, 2562, 2605, 2703-2812 (the reveal tests; 2629 and 2659 are `filter.rs`'s), 2906, 3156, 3199, 4784-5036 (claims, focus motion, location/restore) |
| `tests/filter.rs` | 2247, 2438, 2468, 2491, 2508, 2520, 2629, 2659, 2869, 3112, 3140 |
| `tests/writes.rs` | 3340-4371 (item form, edits, mints, divergence, `$EDITOR` outcomes), 4450-4552 |
| `tests/hints.rs` | 4567-4704 |
| `tests/render.rs` | 2990, 3051, 3228 (`the_detail_strip_fits_the_detail_pane`), 3246 (`the_active_sub_tab_is_bold_and_underlined_without_colour`) |
The test bodies are byte-identical. `use super::*` in `tests/mod.rs` keeps reading `backlog`'s private items (a
child module sees its ancestors' private items), and the grandchildren read both through their own `use super::*`.

### 2.3 Pure-move proof (the commit message carries the numbers)
```bash
cargo test -p htui --all-features --lib -- --list 2>/dev/null | grep ': test$' > /tmp/t1-before.txt   # on T0's head
cargo test -p htui --all-features --lib -- --list 2>/dev/null | grep ': test$' > /tmp/t1-after.txt
wc -l /tmp/t1-before.txt /tmp/t1-after.txt                                       # equal
diff <(sed 's/.*:://' /tmp/t1-before.txt | sort) <(sed 's/.*:://' /tmp/t1-after.txt | sort)   # empty: same leaf names
grep -c 'ui::tabs::backlog::tests::' /tmp/t1-before.txt /tmp/t1-after.txt         # 94 and 94 (96 less T0's two)
```
plus `git diff -M --stat` showing only moves, `cargo insta test --all-features --check` unchanged and the whole
`--all-features` suite green. One commit per new production file is allowed; the tests move in one commit.

## 3. T2: kit additions (serial; tests first in each group)

### 3.1 Dimmed backdrop (C3; `ui/theme.rs`, `ui/kit/modal.rs`)
- `Theme` gains `pub backdrop: Style` — `Style::new().add_modifier(Modifier::DIM).remove_modifier(Modifier::BOLD)` in
  both `Theme::default()` and `Theme::monochrome()` (no colour: a backdrop keeps each cell's own colours and fades
  them). Doc: "what a modal leaves visible around itself".
- `Modal::render(self, frame, area, theme)`: before `Clear`, every cell of `area ∩ buffer` outside `box_area` gets
  `cell.set_style(theme.backdrop)` (ratatui applies `sub_modifier`, so a bold border loses its bold and is never
  bold and dim at once). `area` is what the caller centres over: the whole frame for overlays, a section's rect for
  the Hierarchy path picker.
- Why a backdrop, not a cleared margin: (1) the text of every snapshot is unchanged, so `leave` (4 in-crate),
  `waiting_list` (1 in-crate, `waiting__platform_mixed`) and `hierarchy__picker` need no owner; (2) it holds for
  `Fit`, `Fixed` and a modal clamped to the screen alike, where a cleared ring has no room (the path picker is 96 wide
  at 100 columns; concepts search at 60); (3) it reads under `NO_COLOR` (DIM); (4) a cleared ring still leaves the
  host pane's own `│` beside it, which is the "one-column strip" MOD-82 reports, while a dimmed strip recedes. The
  header row stays uncovered by sizing: every ported overlay uses `ModalSize::Fit`, whose `MARGIN_ROWS` keeps a row
  free at the top (E6's "stops covering the header at 60 columns").
- Tests (`kit/modal.rs`): `the_backdrop_outside_the_box_is_dimmed_and_never_bold` (both themes: every cell outside the
  box has DIM, none BOLD; colours kept), `the_box_itself_is_not_dimmed`, `a_modal_over_a_smaller_area_dims_only_that_area`.
  `theme.rs`: `the_backdrop_role_names_no_colour`.
- Then run the whole suite: any test asserting a style under a modal fails here, in T2, and is fixed here.

### 3.2 `kit::modal::Ask` (C7; E3)
```rust
/// A question in a modal (MOD-115 M4 E3): what is asked, optionally a typed answer, the keys on the bottom border.
pub struct Ask<'a> { title: &'a str, lines: Vec<Line<'a>>, input: Option<(&'a str, &'a TextField)>,
                     error: Option<Cow<'a, str>>, hint: Option<&'a str>, width: u16 }
impl<'a> Ask<'a> {
    pub fn new(title: &'a str) -> Self;                         // width defaults to 60 content cells
    pub fn line(self, line: impl Into<Line<'a>>) -> Self;       // wrapped (kit::text::wrapped) to the content width
    pub fn input(self, prompt: &'a str, field: &'a TextField) -> Self; // `prompt` (theme.title) + the field's line, cursor shown
    pub fn error(self, text: impl Into<Cow<'a, str>>) -> Self;  // theme.error, wrapped, under the input
    pub fn hint(self, hint: &'a str) -> Self;                   // on the bottom border (Modal::hint)
    pub fn width(self, content: u16) -> Self;                   // the widest the content grows
    #[must_use] pub fn rows(&self, content_width: u16) -> u16;  // the height the content needs
    pub fn render(self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) -> Rect;
}
```
- Size: `ModalSize::Fit { content_w: min(width, widest line, hint + 1), content_h: rows }` (so it inherits the margin
  and the backdrop). One blank row between the question lines and the input row; none when there is no input.
- Tests: `an_ask_draws_its_lines_then_the_input_and_the_hint_on_the_border`, `the_input_row_keeps_the_cursor_visible`
  (monochrome: REVERSED cursor), `an_error_goes_under_the_input_in_the_error_role`, `a_long_question_wraps_and_the_box_grows`,
  `an_ask_fits_its_content_and_keeps_the_margin`, monochrome.

### 3.3 The help box (E3, E7)
`kit::modal::help(frame, area, title: &str, rows: &[Line<'_>], content_w: u16, theme) -> Rect`: a `Fixed` modal of
`content_w + CHROME` by `rows + 2`, clamped, the rows drawn as given. `App::render_help` (L-shell) keeps packing
`HelpLine::rows(inner)` with today's `inner = box_width - 2`, and passes it as `content_w`: the box is one column
wider than today so no line re-wraps (the tests that read the box's contents keep their expectations).
Tests: `the_help_box_is_as_tall_as_its_rows_and_titled`, `a_help_box_taller_than_the_screen_clamps`.

### 3.4 `Pane` additions (`kit/pane.rs`)
- **`pub fn title_first(self) -> Self`** (C5, D-4): the title keeps `min(width(title) + 2, avail − w_active − marks − 1)`
  cells (clipped with `…`), and the chip window grows into what is left, right then left, as `chip_strip` does.
  Default precedence (title gives way first, M3 D-10) is unchanged; Settings does not opt in.
  `#[must_use] pub fn chip_strip_after(area: Rect, title: &str, titles: &[&str], active: usize) -> ChipStrip` exposes
  the geometry for a future hit test (no caller in M4; `pub` in a `pub` module, so no dead code).
- **`pub fn border(self, style: Style) -> Self`**: a border style whatever the focus (a flow node coloured by status).
- **`pub fn render_into(self, buf: &mut Buffer, area: Rect, theme: &Theme) -> Rect`**: `render` delegates to it
  (`Block::render` on the buffer); the Runs flow nodes are drawn on a `&mut Buffer` (`NodeContent::render`).
- Tests: `the_title_keeps_its_room_when_asked_first` (Backlog's seven chips at 45 columns: `╭─ FEAT-1 · TUI scaffold ─`
  then `Body`, `Runs` and `›`), `a_long_title_first_still_leaves_the_active_chip_and_its_marks`,
  `title_first_with_room_for_every_chip_is_the_default_layout`, `the_default_precedence_is_unchanged`
  (D-10 tests untouched), `a_border_style_overrides_the_focus`, `render_into_draws_what_render_draws`.

### 3.5 `kit::rule` (new `ui/kit/rule.rs`; C6)
```rust
//! A titled rule (MOD-115 M4): the one-row header of a part drawn inside a pane: an in-pane editor, a compose area.
pub fn render(frame: &mut Frame<'_>, row: Rect, title: &str, focused: bool, theme: &Theme) -> Rect /* the rows under it */;
```
Text: `title` flush left (clipped with `…`), then `─` to the edge — today's `Block::borders(TOP).title(..)` row, cell for
cell (`editor_pane__the_focused_frame` and `backlog__note_compose` text unchanged). Styles: focused `theme.title`
(title and rule), else `theme.dim`; the row is cleared first (the editor rule's `Clear`, `editor_pane.rs:52-53`).
Tests: `the_rule_reads_as_the_title_then_rules`, `a_long_title_is_cut_with_an_ellipsis`, `focused_is_title_else_dim`
(monochrome bold/dim), `it_returns_the_rows_under_it`.

### 3.6 `kit::filter::Filter::Summary` (D-13)
`Summary { query: &'a str, count: &'a str }`: ` / query` as `Applied`, and `count` (the caller's own words,
`11 items`) right-aligned in `theme.dim`, given way last. The Backlog reads its list already filtered and has no total
for `n of m`. Tests: `a_summary_band_shows_the_query_and_the_callers_count`, `the_count_is_kept_at_narrow_widths`,
`an_empty_summary_query_is_no_band`.

### 3.7 Labels (`kit/label.rs`; exhaustive `match`, no wildcard)
| fn | Enum (crate) | Shown today (raw) | Words |
|---|---|---|---|
| `link_kind(LinkKind) -> &str` | `htui-core` `model/link.rs:10-23` (a `str_enum!`, D-1) | Graph `← blocked_by` (`detail/graph.rs:561`, `:578`) | blocked_by `blocked by` · origin · relates · supersedes |
| `gate_outcome(GateOutcome) -> &str` | `model/run.rs:~175` | Runs gate cell `approved✓` (`runs.rs:1363-1375`) | approved · rejected · retried · skipped |
| `run_kind(RunKind) -> &str` | `run.rs:15` | run grid `graph` (`runs.rs:1320`) | graph · chat |
| `run_mode(RunMode) -> &str` | `run.rs:25` | run grid `manual` (`:1326`) | manual · automatic |
| `activation(Activation) -> &str` | `model/skill.rs:20` | Skills attach `always` (`attach.rs:856`, `:1135`) | always · by glob · off |
| `skill_level(SkillLevel) -> &str` | `skill.rs:39` | Prompt `choice_line` (`prompt.rs:248`) | global · project · phase |
Plus the existing `run_status`, `step_status`, `resolution`, `citation`, `setting_source`, `priority`,
`kit::status`. Tests table by table (over `ALL` where the macro gives one, a hand list otherwise): no `_`, one-cell
glyphs, every multi-word value relabelled. `ChoiceReason`, `TrimStrategy`, `RootSource` are **not** labelled (D-17).

### 3.8 T2 gate
`cargo test -p htui kit:: -- --test-threads=1`, the whole `--all-features` suite (the backdrop), insta `--check`
unchanged, both clippies.

## 4. Wave 1 (worktrees from T2's head): L-skills, L-overlays, L-shell
Each lane: `git worktree add -b mod-115-m4-<lane> target/wt/m4-<lane> <T2 head>`; `env -u HTUI_TEST_DATABASE_URL`;
`rustfmt` on its own files only; no edit to `keys/catalogue.rs` or `tests/kit_chrome.rs` (rule 1: its own check is
that `every_m4_entry_still_frames_itself` fails for exactly its files).

### 4.1 L-skills (E5; source `ui/tabs/skills/*`)
**Tests** `tests/skills.rs`, `tests/templates.rs`, **+ the title finders of `tests/editor_pane.rs:51`,
`tests/keys.rs:500`, `:688`, `tests/templates_pg.rs:145`** (D-11): each `format!("\u{250c} {name} v")` becomes a
helper that accepts `┌ {name} v` or `─ {name} v` (the `tests/reveal.rs:68-72` pattern), since the body pane's title
is a kit title.
**Frame** (`mod.rs:248-270`, the `Skills │ Templates` switch line): deleted. The views draw from the whole body.
`draws_chrome() -> true`. The list pane carries the view chips: `Pane::new("").sub_tabs(&["Skills", "Templates"], view)`
(the active chip names the view, so the title is empty; `skills.switch_view` unchanged).

| Site | Becomes |
|---|---|
| `library.rs:1770` list `Block " Skills "`, rows `theme.selected` (`:1785-1790`), offset `:1793` | list `Pane` (chips above), `kit::list::render` rows (`browse_row` text after the gutter), `list::scroll_into_view`, `.scrollbar(..)`; focus C4 |
| `library.rs:1810` body `Block` + ` J/K scroll ` bottom title (`:1811-1815`) | detail `Pane::new(&title)` (the `pane()` title without its padding spaces), `.scrollbar(rows, offset)`; the scroll keys join the pane's hints (`SCROLL_HINT`) when the body overflows (MOD-82 carried) |
| `library.rs:1964` import report | `Pane::new("import report")`, rows through `kit::list` (the `>` marker becomes the gutter and band) |
| `library.rs:2032` draft `Block` | `Pane::new(&title)` (`render_draft`'s title, trimmed), focused while editing; `ctx.claim_editor_area(Pane::inner(area))` unchanged geometry |
| `library.rs:1944-1949` rename form `info_pane` | `kit::form` (`name`, `description` as `Value::Text`, `.editing(true)`) |
| `library.rs:1838-1861` naming / describing / import prompts | `kit::form` one-row fields (`Value::Text`) with the import help as the row's `.help(..)` |
| `templates.rs:1212`, `:1258`, `:1375`, `:1414` | the same four shapes (list, body, draft, placeholders panel) |
| `attach.rs:812` rows (`theme.selected` `:827`) | `Pane` + `kit::list` rows; `*` winner mark kept in the row text |
| `attach.rs:1112` form; activation `theme.selected` (`:1127-1131`) | `kit::form`: activation `Value::Choice(label::activation(a))` with `activation_label` as `.help`, pin/position/languages/globs `Value::Text`, `effective:` `Value::ReadOnly`, the refusal as the globs row's `.error`, `GLOB_HELP` as its `.help` (MOD-85: the form marker, not `selected`) |
| `attach.rs:1194` picker | `Pane` + `kit::list` (band, not `selected`) |
| `agent_help.rs:697` panel, `:752` proposal pane | `Pane::new("ask an agent")` with the request as a one-row `kit::form` and the status line under it; proposal/answer `Pane::new(&title_text)` with `.scrollbar` |
- **Notices**: the views' notice rows (`library.rs:762-790`, same in `templates.rs`) → `kit::text::notice`.
- **Empty states**: `NOT_READ`, `SELECT_A_SKILL`, the unavailable line (`library.rs:1870-1880`) and their Templates twins
  → `kit::empty` with next steps ("they appear once the store answers"; "{list.down/up} chooses one";
  "Settings › Connection stores a DSN" after the unavailable sentence).
- **MD-A**: `library.rs:148` `NAMING_HINT` → `[One(FieldSubmit, "next"), Text("Esc cancel")]` (`LIBRARY_PROMPT`);
  `:173` `IMPORT_HINT` → `One(FieldSubmit, "import")`; `templates.rs:106` → `One(FieldSubmit, "create")`
  (`TEMPLATES_PROMPT`); `agent_help.rs:87` → `One(FieldSubmit, "ask")` (`HELP_ASKING`). All four stacks capture text
  (`views.rs:1785-1791`), so `Keys::hint` never drops them. `Ctx::pressed`: `library.rs:530`, `:837`, `:1056`,
  `:1138`, `:1240`; `templates.rs:630`, `:776`, `:874`; `attach.rs:407`, `:582`; `agent_help.rs:369` (`ctx` threaded
  where the fn lacks it); `mod.rs:174` stays (browse switch, non-capturing).
- **MOD-85** (pinned by buffer tests, colour and monochrome): list/attach/picker cursor → band; attach activation →
  form marker; the draft and body titles → the pane title role.
- **MOD-83**: `attach.rs:856` `activation.as_str()` → `label::activation`; `{n} globs` → `plural`.
- **Tests first** (in `library.rs`/`templates.rs`/`attach.rs`): the chips name the view and switch with
  `skills.switch_view`; no switch line; the body title carries no key literal; each converted hint names a rebound
  `[field] submit = ["ctrl-t"]`; band/marker styles; `draws_chrome()`.
- **80 columns**: `skills__library_80`, `templates__browse_80` (in `tests/skills.rs`, `tests/templates.rs`).
- **Snapshots owned**: `skills__*` (9 + 1), `templates__*` (8 + 1).

### 4.2 L-overlays (E6; source `ui/overlay/{concepts_search,workspace_switcher,migration_prompt,queue}.rs`)
**Tests** `tests/concepts_search.rs` (not `:236`, `:256`, B1's), `tests/queue_overlay.rs` (`:77`, `:86`, `:175` read
`┌ Queue `/`└`/`│> `), **new `tests/overlays.rs`** for the switcher's and the migration prompt's 80-column snapshots
(the lane may not edit `tests/shell.rs`).
- **Concepts search** (`:392-469`): `Modal::new(TITLE).size(Fit { content_w: MAX_WIDTH - CHROME, content_h: 6 + HIT_ROWS })`
  (header, query, blank, hits, status, index), hint on the border (the in-box `hint` row goes), the hits through
  `kit::list` rows (gutter + band, not `> ` + `accent`; MOD-85), the hit rows shrinking to the content height.
  - The hit row is built here, not by `format_hit` (C9): key (10), place (`item`, `{kind} document`, `requirement`,
    with `(` `label::resolution` `)` or `(withdrawn)`), the **score as a word relative to the best hit**:
    `score / best ≥ 0.999 → "best"`, `≥ 0.66 → "strong"`, `≥ 0.33 → "fair"`, else `"weak"` (Q4), then the snippet.
  - `{} hit(s)` (`:273`) → `plural(n, "hit", "hits")`; the index report line (`:384`) built here with `plural` from
    the `SyncReport` fields.
  - Errors (`self.error`, `IndexLine::Failed`) gain `" \u{2014} Settings › Qdrant"` unless they already name a
    Settings place (MOD-83's `Qdrant unreachable — Settings › Qdrant`).
  - MD-A: `IDLE` (`:71`), `CHANGED` (`:92`) → `pub fn idle(submit: Option<&str>)`, `pub fn changed(..)` through
    `keys.label(views::CONCEPTS_QUERY, Act::FieldSubmit)`; `HINT`'s `Text("Enter search/open")` (`:97`) →
    `One(FieldSubmit, "search/open")`; `on_key` `:334` → `ctx.pressed(&key)`.
- **Workspace switcher** (`:213-237`): `Modal` `Fit`, rows through `kit::list` (gutter + band; MOD-85), hint on the
  border, empty/reading through `kit::empty` (`"no workspaces"` + `"`{new}` in Settings › Hierarchy creates one"`;
  `"reading the store"` + `"the list appears once the store answers"`), `projects_label` → `plural` except the
  `no projects` word.
- **Migration prompt** (`:132-152`): `Ask::new("Schema").line(question).hint(&hint)`.
- **Queue** (`:567-595`): `Modal` `Fit`, header line, rows through `kit::list` (escalated rows keep `theme.warning` in
  their line; cursor = band; MOD-85), `READING`/`EMPTY` through `kit::empty` (`EMPTY` next step:
  `"{BacklogQueue label} on a Backlog item queues it"` via `keys.label(views::BACKLOG_BODY, Act::BacklogQueue)`), the
  unavailable line in `theme.error` with `"— Settings › Connection"`.
- **Tests first** (in-file): band rows legible in monochrome for the three lists; the score words; `plural`; the error
  next step; the converted hints under `[field] submit = ["ctrl-t"]`; the modal keeps the header row at 60×20.
- **80 columns**: `concepts_search__hits_80`, `queue_overlay__opened_80`, `overlays__switcher_80`, `overlays__migration_80`.
- **Snapshots owned**: `concepts_search__*` (5 + 1), `queue_overlay__*` (1 + 1), `overlays__*` (2), in-crate
  `htui__ui__overlay__queue__tests__*` (8), `shell__switcher_empty`, `shell__switcher_open`, `shell__migration_prompt`,
  **+ in-crate `htui__testkit__tests__shell_empty`** (it shows the switcher, D-6).

### 4.3 L-shell (E7; source `app/state.rs` (`render_help`, the no-tab body), `app/pane.rs` (tests), `ui/editor_pane.rs`, `ui/overlay/abort_confirm.rs`, `app/update.rs:495`)
**Tests** in-crate `app/pane.rs`, `app/state.rs`, `ui/editor_pane.rs`; **+ `tests/backlog_keys.rs:31-50`** (`help_box`
finds `┌ Keys`/`└`: accept `╭ Keys`/`╰`, D-12).
- **`?` box** (`state.rs:1714-1766`): `kit::modal::help(frame, area, "Keys", &rows, inner, theme)` (§3.3); closer line
  dim as today.
- **No-tab body** (`:1630-1636`): `Pane::new("")` + `kit::empty::render(inner, "No tab is registered.",
  NextStep("a tab registered at start shows here"))`.
- **Editor frame** (`editor_pane.rs:46-55`): `kit::rule::render(frame, Rect { height: 1, ..area }, title, focused,
  theme)`; `draw_screen` into the rows under it; `pane_size` (`app/pane.rs:630-632`) unchanged.
- **Abort question** (`abort_confirm.rs:111-131`): `Ask::new("Abort").line(QUESTION).hint(&hint)`; the hint moves to
  the border (`HINT`'s `Text("any other key keeps editing")` is prose, not a key: kept).
- **`applied {n} migration(s)`** (`app/update.rs:495`, and its test `:1177`) → `plural(n, "migration", "migrations")`
  (M3 §5.8 listed it for group B; no other lane owns `update.rs`, D-2).
- **Tests first**: the help box packs as before (a line that fitted still fits); the no-tab body has a next step; the
  rule reads as before and is `title`/`dim` by focus; the abort question's hint on its border.
- **80 columns**: in-crate `htui__app__pane__tests__the_abort_question_80`, `htui__app__state__tests__the_help_box_80`.
- **Snapshots owned**: `htui__app__pane__tests__*` (4 + 1), `htui__app__state__tests__the_help_box_80`,
  `htui__ui__editor_pane__tests__*` (2).

## 5. B1: the Backlog frame, list and filter (serial, primary tree, after wave 1 merges)
**Source**: `backlog/{mod.rs, list.rs, filter.rs, detail/mod.rs}`, `backlog/hints.rs` (T1), and **the frame lines of
`item_form.rs::render` and `divergence.rs::render`** (D-9). **Tests**: `tests/backlog.rs`, **+ `tests/backlog_keys.rs`
(`:84`), `tests/backlog_keys/{l_a,l_b,l_c}.rs`, `tests/concepts_search.rs:236,256`, `tests/editor_pane.rs:525,534`,
`tests/prompt_preview.rs:115,747`, `tests/waiting.rs:295`** (D-10).

### 5.1 Render (`impl Tab::render`, `mod.rs:1104-1144`)
- `fn draws_chrome(&self) -> bool { true }`.
- Panes: list (`list_area`), Filter (`form_area`, when the form is open), detail (`detail_area`); divergence: one
  pane over the whole `area`. Focus by C4 through a helper `fn keyboard_pane(&self) -> Option<Holder>` in `hints.rs`
  (`Filter` while `self.form`, `Detail` while `self.item_form` or `detail.captures_input()`, else `None` = the shell's).
- **Hints per pane** (Q2), each through the existing builders, fitted to that pane (`width − 4`):
  list → `list_hint`; Filter → `FilterForm::key_hint`; detail → item form / divergence / capturing sub-tab hint,
  else `detail.key_hint`, joined with ` · ` and `LIST_HINT` when both fit (today's join rule at the detail's width),
  else the pane's, else `LIST_HINT`. `Tab::key_hint` keeps today's composition for the activity pane (C8).
- **List pane** (`list.rs:129-165`): `Pane::new(&format!("Backlog ({n})")).scrollbar(rows, offset)`; the active
  filter is the first inner row, `kit::filter::Filter::Summary { query: &summary, count: &plural(n, "item", "items") }`;
  rows through `kit::list::render`: projects `list::group_row(name, count, folded)`, items
  `" {key}  {status mark}  {title}"` (one space after the gutter keeps the key column), the status column sized to the
  widest **mark** (`⚠ needs you` 11 cells, not `awaiting_approval` 17: MOD-81's titles regain 6 cells), status
  through `kit::status::spans`; cursor = gutter + band (`ctx.accent()`); offset `kit::scroll::window` as today.
  Empty: `NO_ITEMS` + `"{BacklogNewItem} adds one"` / `"Settings › Hierarchy adds a project"` with no project;
  `NO_MATCH` + `"{BacklogClearFilter} clears the filter"`.
- **Filter pane** (`filter.rs:452-462`): `Pane::new("Filter")`, rows through `kit::form` (`label_width(8)`):
  `status`/`project` as `Value::Lines` of today's `‹[x] word›  chosen` text with **`kit::status` words**, `tags`
  `Value::Text(&tags)` (editing on that row), `ready` `Value::Lines`. `FORM_HEIGHT` stays 6. `BacklogFilter::summary`
  uses kit words and spaces: `status: done, running · project: htui · tags: gpu, rust · ready here`.
- **Detail pane** (`detail/mod.rs:385-418`): `Pane::new(&title).title_first().sub_tabs(&titles, active)`,
  `title = format!("{} · {}", item.key, item.title)` or `Detail` with no item (BacklogTab passes the `ItemSummary`
  instead of the key: `detail::render(frame, area, registry, item: Option<&ItemSummary>, focus, hints, ctx)`). The
  in-body strip row (`render_strip`, `strip_line`, `frame_block`) is deleted; the sub-tab gets the whole inner rect
  (one row more). The two strip tests (`backlog/tests/render.rs`) are rewritten over the chips (M3 T3's pattern):
  `the_detail_chips_window_around_the_active_one_at_100_columns`, `the_active_chip_is_bold_and_underlined_without_colour`.
- **Item form / divergence frames** (D-9): `item_form::render(frame, inner, form, theme) -> Rect` draws rows only and
  `pub fn title(form: &ItemForm) -> String` gives `Edit ANA-1 (v1)` / `New item · {slug}`; `divergence::render(frame,
  inner, view, theme)` likewise with `pub fn title(view) -> String`. Their pane is drawn by `BacklogTab::render`.
  `ctx.claim_editor_area` keeps the rect `item_form::render` returns (same geometry: `Pane::inner` = `Block` inner).
  `item_form.rs` and `divergence.rs` leave `GROUP_B` at B1's merge.

### 5.2 Tests first (B1)
In `list.rs`/`filter.rs`/`detail/mod.rs`/`backlog/tests/render.rs`: the list rows (group, item, band, gutter, the mark
column width), the summary band, the filter form's marker in monochrome, the detail title `KEY · title` and the chip
window at 45 and 72 columns, `keyboard_pane` per state, hints per pane, `draws_chrome()`. Harness: `backlog__list_80`
(list + detail at 80×24) in `tests/backlog.rs`.

### 5.3 Existing tests B1 rewrites (chrome only)
- `┌ KEY` finders → one helper `fn on_item(frame, key) -> bool` (`╭─ {key} ` or `─ {key} ·`): `tests/backlog.rs`
  `:139,144,148,154,205,319,358,439,621,666,681,1277,1292,1319,1326,1340,1351,1364,1384,1450,1879,2394,3124,3408,3509,3524,3540,3549,3559`;
  `┌ Detail` `:163,227`; `backlog_keys.rs:84`; `l_a.rs:71,81,85,116`; `l_b.rs:291,334,341,511`; `l_c.rs:141,152,163,179`;
  `concepts_search.rs:236,256`; `editor_pane.rs:525,534`; `prompt_preview.rs:115,747`; `waiting.rs:295`.
- `list_title` (`backlog.rs:1712-1878`): the summary moves to the band; `:1878` `┌ Backlog (2) · status:done`.
- `┌ Filter ` finders `backlog.rs:3620,3641`, `l_a.rs:425-450` (`filter_rows`, `focused`: the marker is the gutter,
  not `> `), `l_a.rs:460-499` expectations (`> status  ‹[ ] open›  any` → kit form row), `backlog.rs:1867`.
- Hint rows (Q2): `l_a.rs:412,501` (filter hint now on the Filter pane: still the last row), `l_b.rs:367-403`,
  `:492` (Runs keys need the detail focused: the tests press the pane key first), `shell.rs:472-505` stay green
  (Body: `LIST_HINT` on the list pane, 50 of 51 cells).
- `DETAIL_INSIDE` (`backlog.rs:1493`) is unchanged (55 + border).

### 5.4 Snapshots B1 re-accepts (after a per-snapshot content-parity review)
All 41 `backlog__*`, `shell__{about_panel_on, after_switch, offline_label, switcher_empty, switcher_open, migration_prompt}`
(D-5: `about_panel_on` draws the Backlog), `concepts_search__*` (6), `waiting__*` (2), `integration__demo_shell`,
`prompt_preview__*` (2), `replay__runs_step_selected`, `htui__testkit__tests__shell_empty`, plus the new
`backlog__list_80`. Expected diff: rounded corners, the `KEY · title` title and chips on the detail border, no strip
row (bodies one row up), status marks and the shorter status column, the band/gutter, the summary band, hints per pane.

## 6. Wave 2 (worktrees from B1's head): the detail bodies
Rule 9 extends to `tests/backlog_keys/*.rs` and `tests/reveal.rs`: a lane edits only the functions that assert its own
sub-tab; the merging thread resolves textual conflicts. Each lane adds a test file of its own for its 80-column
snapshots and new harness tests.

### 6.1 L-runs (`detail/runs.rs`, `detail/runs/execution_graph.rs`; new `tests/backlog_runs.rs`)
- **Frame**: `execution_graph.rs:261` node `Block::bordered()` → `Pane::new("").border(self.border(selected)).render_into(buf, ..)`
  (§3.4; nodes become rounded).
- **Close-out and cancel** (`runs.rs:2115-2145` footer): `Mode::CloseOut(Warn)`, `Mode::CloseOut(Typed)`,
  `Mode::ConfirmCancel` draw an `Ask` centred over the Runs content rect (warn: the summary line, `WARN_HINT`;
  typed: `.input(&format!("type {} to close it:", key), field)`, `TYPED_HINT`; cancel: `"cancel this run?"`,
  `CONFIRM_CANCEL_HINT`); `RunsTab::key_hint` returns `None` in those modes (the modal carries them). The reject note
  and follow-up stay footer rows (they are not questions).
- **MOD-83 copy** (E10):
  - step ids (`slot`, `:1348-1360`; flow head `execution_graph.rs:212-218`): the slot column becomes the step in
    words: the phase name leads line 1 (it is the step's name), and line 2 opens with its place: `try {attempt}` when
    `attempt > 1`, `candidate {fanout_index + 1}` in a fan-out slot, `judge` for `-1`; nothing for a plain first try.
    The flow node head reads `{phase}` then the status mark, and its second line the place. Every datum of `0.1/0`
    stays (position is the row order). Column budget: L-runs rebalances `SLOT_WIDTH`/`STATUS_WIDTH`/`PHASE_WIDTH`/
    `INDENT`/`GATE_WIDTH` within `PANE` 43 and pins it with a test over every `StepStatus` (`↷ superseded` is 12 cells).
  - statuses: `run_status`/`step_status` (`:1290-1305`) → `label::run_status`/`step_status` marks; kind/mode →
    `label::run_kind`/`run_mode`; gate cell → `label::gate_outcome` (+ `*`, `✓` kept).
  - close-out summary (`:2129-2135`): `label::resolution`, `plural(runs, "run", "runs")`, `plural(rows, "commit row", "commit rows")`.
- **MOD-85** (E9): flow edge label `with_label_style(theme.accent)` (`execution_graph.rs:374`) → `theme.base` (a word on
  a dim stroke, not focus); run-entry cursor `kind.style = theme.accent` (`runs.rs:2084`) → `theme.select` band on the
  kind cell; step cursor label (`:1419`) → `ctx.accent()` beside the `▸` mark (focus; the glyph carries it in
  monochrome).
- **MD-A**: `Ctx::pressed` at `runs.rs:725`, `:1051`, `:1083`, `:1130`, `:1168` (threaded into `reject_key`,
  `follow_up_key`, `close_out_key`).
- **Tests first**: the step place words for every slot shape; marks fit their columns; the asks' rows and hints; edge
  and cursor styles in monochrome; `run_status` helper in `tests/backlog.rs:1506` reads the mark's word.
- **80 columns**: `backlog_runs__runs_80`, `backlog_runs__flow_80`.
- **Snapshots owned**: `backlog__{detail_runs, empty_runs, runs_*}` (12), `replay__runs_step_selected`, `backlog_runs__*`.

### 6.2 L-docs (`detail/{documents,notes,compose}.rs`; new `tests/backlog_docs.rs`)
- `compose.rs:593-597` top rule → `kit::rule::render` (focused while the area has the keys); the kind/title rows →
  a two-field `kit::form` (marker = gutter, not `> `), `body` as a form heading over the `TextArea`; notice →
  `kit::text::notice`.
- Documents table (`documents.rs:313-380`, ratatui `Table` + `selected`) → header line + `kit::list` rows; the kind
  column `theme.accent` (`:342`) → `theme.base` (MOD-85, E9: a category word, not focus); cursor = band.
- Notes thread: cursor/selection to the band; empty states (`No documents yet.`/`No notes yet.`) → `kit::empty` with
  `"{new} writes one"` through `BACKLOG_DOCS`/`BACKLOG_NOTES`.
- **80 columns**: `backlog_docs__documents_80`, `backlog_docs__notes_80`.
- **Snapshots owned**: `backlog__{detail_documents, document_form, empty_documents, detail_notes, note_compose, empty_notes}`, `backlog_docs__*`.

### 6.3 L-forms (`item_form.rs`, `divergence.rs`; new `tests/backlog_forms.rs`)
- Item form rows (`item_form.rs:1072-1160`, `marker` `:1040`) → `kit::form`: project/kind/graph `Value::Choice`
  (their words), title/priority/tags `Value::Text`, `paths` and `body` as headings over their `TextArea`s; the notice
  as the form's last rows; the returned editor rect keeps its meaning.
- Divergence state column (`divergence.rs:605-609`): `Theirs`/`Mine` `theme.accent` → `theme.base` (E9: the word names
  the side; `Conflict` stays `error`, `Same` `dim`).
- MD-A: the forms already resolve from `Ctx::pressed` (`backlog/writes.rs`); `HINT_PICK`/`HINT_TEXT`/`HINT_AREA` are
  checked for literal keys (only `Esc …`, MD7, and the RV1 `Hint::Glyphs` may remain).
- Tests: `tests/backlog.rs:2446` and `backlog_keys/l_a.rs:184,199` read `> title` (the marker): rewritten to the band.
- **80 columns**: `backlog_forms__item_form_80`, `backlog_forms__divergence_80`.
- **Snapshots owned**: `backlog__{item_form_new, item_form_edit, item_divergence}`, `backlog_forms__*`.

### 6.4 L-detail (`detail/{body,graph,prompt,requirements}.rs`; new `tests/backlog_detail.rs`)
- **Body** (`body.rs:112-138`): the head is one meta line, `{status mark} · {tags} · priority {p} · version {v}`
  (`kit::status::spans`), then a blank; the title line and the `KEY  PREFIX  status` line go (the title and key are the
  pane title since B1; the prefix is the key's own, plan E4). `HEAD` 4 → 2. `"No body for this item."` →
  `kit::empty` + `"{Edit} writes one"`.
- **Graph**: link kinds → `label::link_kind` (and `KIND_WIDTH` from the labels), node statuses → `kit::status`, the
  cursor `▸` in `theme.accent` (`:563`) → the row band + `ctx.accent()` gutter.
- **Prompt**: `{} template(s)` (`:157`) → `plural`; the picker row's `n / p` keys (MOD-82 carried) leave the row for a
  new `PromptTab::key_hint` (`Hint::Pair(PromptNextTemplate, PromptPrevTemplate, "template")` through
  `BACKLOG_PROMPT`); `budget_source.as_str()` (`:189`) → `label::setting_source`; `choice.level` → `label::skill_level`,
  `choice.activation` → `label::activation`; the audit's record words stay (D-17, Q3); clipped rows end in `…`
  (MOD-81's "clips its right edge", D-24).
- **Reqs**: citation kinds `label::citation`, statuses `kit::status`, the cursor band.
- `tests/reveal.rs:113` (`│CI matrix for the three OSes`, the Body title line) → the pane title.
- **80 columns**: `backlog_detail__{body,graph,prompt,reqs}_80`.
- **Snapshots owned**: `backlog__{detail_body, empty_body, detail_graph, graph_depth_1, graph_depth_3,
  graph_outside_workspace, graph_re_rooted, empty_graph, detail_prompt, empty_prompt, detail_reqs,
  detail_reqs_cite_picker, detail_reqs_reconfirmed, empty_reqs}` (14), `prompt_preview__*` (2),
  `shell__{about_panel_on, after_switch, offline_label, switcher_empty, switcher_open, migration_prompt}` (D-5),
  `concepts_search__*` (6), `waiting__*` (2), `integration__demo_shell`, `htui__testkit__tests__shell_empty`,
  `backlog_detail__*`.

## 7. T3: delete the transition, guard, close-out (serial, last)
- `ui/tabs/registry.rs:164-170` `Tab::draws_chrome` deleted, with every impl (`settings/mod.rs:494`,
  `requirements/mod.rs:1441`, `chat/mod.rs:583`, Backlog, Skills) and the asserts (`settings/mod.rs:661-662`,
  `requirements/mod.rs:2845`, `chat/mod.rs:1285`, the lanes' new ones). `Tab::key_hint` stays; its doc
  (`:94-97`) now says it is what the shell draws on the focused activity pane (C8).
- `app/state.rs`: `paint_shell_hints` (`:1829-1846`), `join_panes_under` (`:1848-1867`), `hint_row` and
  `HINT_SEARCH_ROWS` (`:1810-1827`) deleted with their tests (`:2029-2160`); `render` loses `draws_chrome`
  (`:1646-1650`) and the adapter block (`:1654-1678`), and its doc (`:1573-1583`) the adapter; `paint_panes(frame,
  areas)` keeps only the activity pane, now
  `Pane::new(ACTIVITY_TITLE).focus(ctx.pane_focus(PaneId::Activity)).accent(ctx.accent())` (+ `.hints(tab.key_hint(..))`
  when focused) with `kit::empty::render` inside, `ctx = self.tab_ctx(tab).with_covered(..)`; the focus loop
  (`:780-794`) and the `chrome` hint branch (`:795-807`) go.
- `ui/kit/pane.rs`: `paint_focus`, `is_box_drawing` and their five tests (`:1268-1400`) deleted (no caller left);
  `paint_bottom_hints`/`paint_bottom_note` stay; the module doc's "For panes a tab still frames itself" paragraph
  becomes the hidden note's.
- `ui/kit/empty.rs`: `placeholder` deleted with its tests (the shell builds the pane itself; D-22).
- `app/state.rs` tests: `SplitTab` draws kit panes (`ChromeTab`'s render, `ctx.accent()`, its `key_hint` as
  `Pane::hints` on the focused pane); `ChromeTab`, `chrome_app` and `a_chrome_tab_*` fold into it;
  `the_focused_pane_has_the_tab_accent…`, `the_hints_follow_the_focused_pane`, the hidden-note tests and
  `the_activity_placeholder_is_still_the_shells` keep their assertions on the kit pane.
- `tests/kit_chrome.rs`: `GROUP_B` and `every_m4_entry_still_frames_itself`, `no_group_a_file_is_listed_for_m4`
  deleted; `GROUP_A` replaced by "everything outside `KIT`"; `nothing_else_frames_itself_unless_m4_lists_it` →
  `nothing_outside_the_kit_frames_itself`; `every_own_block_entry_is_still_needed` drops its `GROUP_B` assert.
- `detail::message` (`detail/mod.rs:448`) deleted if no caller is left.
- Validation block, then the records (PRD row 4, the decisions M4 section, MOD-115's write-up folding
  `mod-115-decisions.md`, MOD-81/82/83/85 close-out lines naming what landed and what was carried: MOD-81's
  160-column cap and the Prompt pane's wrap to MOD-119).

## 8. Snapshot ownership (every `.snap` under `tests/snapshots` and `src/**/snapshots`, checked against `git ls-files`)
| Snapshots | Owner(s), in order | Notes |
|---|---|---|
| `backlog__filter_form, filter_no_match, filtered_list, list_folded, list_grouped, list_last_row` (6) | B1 | |
| `backlog__{detail_runs, empty_runs, runs_*}` (12) | B1 → L-runs | |
| `backlog__{detail_documents, document_form, empty_documents, detail_notes, note_compose, empty_notes}` (6) | B1 → L-docs | |
| `backlog__{item_form_new, item_form_edit, item_divergence}` (3) | B1 → L-forms | |
| `backlog__{detail_body, empty_body, detail_graph, graph_*×4, empty_graph, detail_prompt, empty_prompt, detail_reqs*×3, empty_reqs}` (14) | B1 → L-detail | 6+12+6+3+14 = 41 |
| `replay__runs_step_selected` | B1 → L-runs | |
| `prompt_preview__*` (2), `waiting__*` (2), `integration__demo_shell`, `shell__{about_panel_on, after_switch, offline_label}` | B1 → L-detail | `about_panel_on` added (D-5) |
| `shell__{switcher_empty, switcher_open, migration_prompt}`, `concepts_search__*` (5) | L-overlays → B1 → L-detail | |
| in-crate `htui__testkit__tests__shell_empty` | L-overlays → B1 → L-detail | L-overlays added (D-6) |
| `queue_overlay__opened`, in-crate `htui__ui__overlay__queue__tests__*` (8) | L-overlays | no Backlog under them |
| `skills__*` (9), `templates__*` (8) | L-skills | |
| in-crate `htui__app__pane__tests__*` (4), `htui__ui__editor_pane__tests__*` (2) | L-shell | test doubles, no Backlog |
| in-crate `htui__ui__overlay__leave__*` (4), `…waiting_list__tests__offline` | nobody: must not move | the backdrop changes styles only (C3) |
| in-crate `htui__state_cmd__tests__*` (12) | nobody | CLI text; `format_hit`/`report_line` untouched (C9) |
| `box_settings`, `chat`, `connection`, `hierarchy`, `kinds`, `personas`, `probe`, `project_types`, `prompt_settings`, `qdrant_settings`, `queue_settings`, `requirements`, `secrets_settings`, `settings`, `state_cli_pg` (all) | nobody | group A / JSON; T3 changes no group-A frame |
| new: `backlog__list_80` (B1); `skills__library_80`, `templates__browse_80` (L-skills); `concepts_search__hits_80`, `queue_overlay__opened_80`, `overlays__{switcher_80, migration_80}` (L-overlays); `htui__app__pane__tests__the_abort_question_80`, `htui__app__state__tests__the_help_box_80` (L-shell); `backlog_runs__*`, `backlog_docs__*`, `backlog_forms__*`, `backlog_detail__*` (their lanes) | their creator | |
Within a wave the sets are disjoint (wave 1: overlays/shell_empty vs skills vs app/editor; wave 2: the four body sets,
collateral only in L-detail). Across waves the order above is serial.

## 9. Hidden coupling checked
- **Shared test files per wave.** Wave 1: `tests/editor_pane.rs` and `tests/keys.rs` are L-skills' only (L-shell keeps
  the `?` box packing, so `keys.rs:329-347`, `:564-570` and `shell.rs:403-447` are unchanged); `tests/backlog_keys.rs`
  is L-shell's only. Wave 2: `tests/backlog.rs`, `tests/backlog_keys/*.rs`, `tests/reveal.rs` by function (rule 9).
- **`keys/views.rs`**: T1 needs no edit (§2.1); no lane adds an act (MD-A uses `FieldSubmit` rows that every
  converted stack already captures).
- **Kit**: `ui/kit/*` and `ui/theme.rs` are T0/T2/T3's only. A kit gap found in a lane is a serial fix merged before
  the next wave (plan risk 5).
- **`detail::message`** is `pub` (`ui::tabs` is `pub mod`), so a sub-tab lane dropping its last call trips no dead
  code; T3 deletes it.
- **`Tab::key_hint` and the about panel**: the about panel reads `browse_stack` (`state.rs:1617`); `key_hint`'s only
  caller after T3 is the activity pane.
- **PTY size**: unchanged (C6); `app/pane.rs:2205`, `:2211`, `:2564` keep `PaneSize::new(h - 1, w)`.

## 10. Commit groups and build sequence
| Task | Commit groups (tests first in each) |
|---|---|
| T0 | (a) `kit::scroll` (+ `window` test), `kit::pane::capture_area`, `kit::text::STAMP`, `ui::queue_status` with their moved tests; (b) every importer of §1.2; gate, insta `--check` unchanged |
| T1 | (a) one commit per production file (`navigation`, `queue`, `writes`, `hints`), suite green after each; (b) `backlog/tests/` in one commit with the §2.3 numbers in its message |
| T2 | (a) backdrop + `Theme::backdrop`; (b) `Ask`, `help`; (c) `Pane::{title_first, border, render_into}`; (d) `kit::rule`; (e) `Filter::Summary`; (f) labels |
| wave 1 | per lane: (a) tests; (b) frames; (c) hints/MD-A/pressed; (d) copy and accents; (e) 80-col + snapshots (own names only) |
| B1 | (a) tests; (b) list + summary band; (c) filter pane; (d) detail pane, title, chips; (e) item form/divergence frames; (f) `draws_chrome`, hints per pane; (g) test-file chrome rewrites; (h) snapshots, one commit per test file after review |
| wave 2 | per lane as wave 1 |
| T3 | (a) deletions + state.rs test doubles; (b) `kit_chrome.rs`; (c) records |
Sequence: T0 → T1 → T2 → wave 1 (three worktrees, merged `--no-ff` one at a time, gate on each merged tree; the
merging thread drops the lane's `GROUP_B` entries) → B1 (drops `item_form.rs`, `divergence.rs`, `list.rs`,
`filter.rs`, `detail/mod.rs` from `GROUP_B`) → wave 2 (four worktrees) → T3. Lane commits name the lane in the subject
(`… (MOD-115 M4 L-runs)`) and stage by path.

## PLAN DEVIATIONS
- **D-1 (E3 "Link and document kinds are not `htui-core` enums").** False for links: `LinkKind` is a `str_enum!`
  (`crates/htui-core/src/model/link.rs:10-23`; a grep for `enum LinkKind` misses the macro), and the Graph prints it
  raw (`backlog__detail_graph`: `← blocked_by agy:ANA-1`). Amend: T2 adds `label::link_kind`. Document kind is a
  `String` (`model/document.rs:17`): true.
- **D-2 (E3 "plural for `hit(s)`, `template(s)` and `1 options`").** `1 options` closed in M3
  (`chat/transcript.rs:766`). Group B's `(s)`: `concepts_search.rs:273`, the overlay's index line
  (`concepts.rs:710`, shared with the CLI), `prompt.rs:157`, and the shell's `applied {n} migration(s)`
  (`app/update.rs:495`), which M3 §5.8 gave to group B and no plan lane owns. Amend: L-shell takes `update.rs:495`.
- **D-3 (E10 "Runs: `gate soft · in none · budget inherit`").** That line is Settings › Kinds' `phase_line`, rewritten
  by M3 L-kinds; Runs has no such line. Amend: E10's Runs part is the step ids, the raw run/step/kind/mode/gate words
  and the close-out summary (§6.1).
- **D-4 (E4 "the detail title `KEY · title` with the sub-tabs as chips in its top border").** At the default 100
  columns the detail pane is 45 wide: the seven chips need 46 cells of the 43 between the corners, so under M3 D-10
  ("the title gives way once any chip is cut", `kit/pane.rs:222-224`) the title is never drawn, and below ~163
  columns there is no room for both. With L-detail dropping Body's key and title lines, the key would be on screen
  nowhere but the list row (acceptance 7). Amend: T2's `Pane::title_first()` for the Backlog detail (C5): the title
  keeps its room, the chips window around the active one with `‹`/`›`. **Q1.**
- **D-5 (collateral "`shell__` 5 of 6, not `about_panel_on`").** `shell__about_panel_on` draws the Backlog (list and
  Body) under the about panel. Amend: B1 and L-detail own it.
- **D-6 (L-overlays' snapshot set).** In-crate `htui__testkit__tests__shell_empty` shows the workspace switcher, so
  it moves in wave 1. Amend: L-overlays owns it first.
- **D-7 (T0's file set).** The Backlog's own sub-tabs import the moving items too: `detail/{body,documents,graph,notes,prompt,requirements,runs}.rs`
  (`Scroll`, `Step`, `STAMP`, `PAGE`, `list::window`), plus `ui/mod.rs` (`queue_status`) and `kit/mod.rs`. Amend:
  T0's set adds them (§1.2); T0 is serial, so no lane conflicts.
- **D-8 (E2 "mod.rs: … a thin `impl Tab`"; navigation "includes `location`/`restore`/`reveal`").** Those three are
  trait methods, and `keys/views.rs` reads `ui/tabs/backlog/mod.rs` (`:1121`) as the delegate that handles every
  Backlog browse stack's acts (`:1288-1293`), i.e. `on_key`. Amend: `impl Tab` stays verbatim in `mod.rs`; helpers
  move with `pub(super)` (§2.1). `mod.rs` is ~760 production lines, not thin.
- **D-9 (B1's files).** Flipping `draws_chrome` turns the adapter off for the whole tab, but the item form
  (`item_form.rs:1068`) and the divergence view (`divergence.rs:474`) frame themselves in the detail's place: from B1
  to L-forms they would show no hints and no focus. Amend: B1 converts those two frames (the `Block` lines and a
  `title` fn each); L-forms keeps the rows, copy and accents.
- **D-10 (B1 "the one task that edits `tests/backlog.rs`").** The Backlog detail title (`┌ KEY`) is also read by
  `tests/backlog_keys.rs:84`, `backlog_keys/{l_a,l_b,l_c}.rs`, `concepts_search.rs:236,256`, `editor_pane.rs:525,534`,
  `prompt_preview.rs:115,747`, `waiting.rs:295`; the filter form by `l_a.rs:425-499`; the hint row by `l_b.rs:367-403`,
  `:492`. Amend: B1 owns those lines (§5.3); wave 2's rule 9 extends to `backlog_keys/*.rs` and `reveal.rs`.
- **D-11 (L-skills' tests).** The Skills/Templates body title (`┌ {name} v`) is found by `tests/editor_pane.rs:51`,
  `tests/keys.rs:500`, `:688`, `tests/templates_pg.rs:145`. Amend: L-skills owns those helpers.
- **D-12 (L-shell's tests).** `tests/backlog_keys.rs:31-50` finds the `?` box by `┌ Keys`/`└`. Amend: L-shell owns it.
- **D-13 (E4 "the active filter through `kit::filter`").** `kit::filter` shows `n of m`; the Backlog reads its list
  already filtered (`filter.rs:103-111`) and never has the total. Amend: T2 adds `Filter::Summary` (§3.6).
- **D-14 (E7 "the in-pane editor's frame is a kit pane").** The editor draws inside its host's claimed rect (the draft
  pane's inner, `library.rs:2045`); a full pane nests a box there and shrinks the PTY by two columns and a row
  (`app/pane.rs:630-632` and the size tests `:2205`, `:2211`, `:2564`). Amend: a kit titled rule (§3.5), shared with
  `compose.rs:593`'s rule; the title still carries the editor's state.
- **D-15 (E6 margin).** Chosen: dimmed backdrop (C3), which needs a `Theme::backdrop` role: `ui/theme.rs` joins T2.
- **D-16 (Runs flow nodes).** `execution_graph.rs:261` draws on `&mut Buffer` inside rataflow's `NodeContent::render`,
  and colours the border by status; `Pane` draws on a `Frame` by focus. Amend: T2's `Pane::{render_into, border}`.
- **D-17 (acceptance 3 "no snake_case enum" vs the Prompt audit).** The Prompt sub-tab prints `trim_record`'s own
  words (`head_tail`, `stub_ladder`, `no_path`, `run_step_tree`, `missing_version`, …). `TrimStrategy::as_str` and
  `RootSource::as_str` (`htui-core/src/prompt/{trim.rs:80-86, excerpt.rs:185-190}`) say a view must render the row's
  word, and ANA-5 §12 criterion 12 is written in `no_path`. Amend: kept as the audit's vocabulary, an exception to
  acceptance 3; `budget_source`, `level`, `activation` are labelled. **Q3.**
- **D-18 (E3's label list).** The additions are `link_kind`, `gate_outcome`, `run_kind`, `run_mode`, `activation`,
  `skill_level` (§3.7); `run_status`/`step_status` exist but their marks (`↷ superseded` 12 cells) need the Runs
  grid's widths rebalanced (L-runs).
- **D-19 (E6 "scores and `hit(s)` read as words").** The overlay rows come from `concepts::format_hit` and the index
  line from `report_line`, both the CLI's (`state_cmd.rs:420`, 12 in-crate snapshots, `concepts.rs:966`). Amend: the
  overlay builds its own rows; the CLI text is unchanged. The score becomes a relative word (**Q4**).
- **D-20 (hints after the adapter).** The adapter painted the tab's whole hint from the focused pane's left edge to
  the body's right edge; per-pane hints are trimmed to one pane (M3 D-14's case, now Backlog and Skills). With the
  list focused, Backlog › Runs no longer shows `J/K step · Enter replay · v flow` (they still act). **Q2.**
- **D-21 (E8 "`Tab::key_hint` stays only if a caller survives").** One does: the shell's activity pane (C8).
- **D-22 (T3's files).** `ui/kit/empty.rs` (`placeholder` loses its only caller) and the `app/state.rs` test doubles
  (`SplitTab` frames with `Block` and relies on the restyle) join T3.
- **D-23 (E1 "a home outside both" for `queue_sentence`).** `ui/queue_status.rs`, not the kit (it names store-worker
  types).
- **D-24 (MOD-81 "the Prompt pane clips its right edge").** The Prompt pane renders a pre-split window per reply and
  scrolls by unwrapped row (`prompt.rs:116-134`, `:438-451`); wrapping would renumber the audit and break `Scroll`'s
  clamp. Amend: clipped rows end in `…`; wrapping goes to MOD-119 with the 160-column cap (named in MOD-81's close-out).

## Maintainer questions
1. **Q1 (D-4)** The Backlog detail's `KEY · title` and the seven chips on one border at 100 columns: title first, chips
   windowed with `‹`/`›` (recommended), or the title on the border and the chips as the pane's first row?
2. **Q2 (D-20)** Hints only on the focused pane, as Requirements in M3: accept that list-focused Backlog/Skills show
   the list's keys only, or draw the tab's whole hint on whichever pane is focused?
3. **Q3 (D-17)** Keep the Prompt audit's `trim_record` words (`head_tail`, `no_path`, …) as an exception to "no
   snake_case on screen"?
4. **Q4 (D-19)** Concepts-search scores as words relative to the best hit (`best`/`strong`/`fair`/`weak`, cuts at
   0.66 and 0.33), dropping the number: confirm the words and cuts.

## Maintainer answers (2026-10-28)
- **Q1 (D-4):** title first, chips windowed with `‹`/`›` (C5 as written).
- **Q2 (D-20):** hints on the focused pane only, as the group-A tabs.
- **Q3 (D-17):** the Prompt audit's record words stay, recorded as an exception.
- **Q4 (D-19):** concepts-search scores read `best`/`strong`/`fair`/`weak` relative to the top hit (cuts 0.66,
  0.33); the number goes.
