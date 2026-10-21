# MOD-115 M3 blueprint (code-architect)

Companion to `mod-115-tui-shell-m3.plan.md` (CONFIRMED 2026-10-20). Where the two differ, this file wins, because
it comes from reading the code. Every `PLAN DEVIATION` is listed at the end with its evidence. Paths are under
`crates/htui/src/` unless they start with `crates/`, `docs/` or `tests/` (`tests/` = `crates/htui/tests/`). Line
anchors are for `c93584ec` (`hr/MOD-115`).

## Conventions (unchanged from M1/M2)
- Every module opens with a `//!` doc. Pure geometry is `#[must_use]`. Styles come from `Theme` roles only, each
  with a monochrome test.
- Tests live in the file they cover. They read a `TestBackend` back as strings and are named as sentences.
- Labels arrive already resolved from `Keys`. Glyph widths go through `cells::cell_width`.
- Gate serially (`--test-threads=1`). Run featureless clippy too (`cargo clippy --workspace -- -D warnings`). Use
  `--all-features` for `tests/*.rs`: without the testkit feature they run 0 tests and still report ok.
- New for M3:
  - a ported view takes its accent from `ctx.accent()`, never `theme.accent` (which now means a modal's or an
    overlay's focus only);
  - a status is a `kit::label::Mark` (glyph + word);
  - a count with a noun goes through `kit::label::plural`.

## Decisions taken here (settle what the plan left open)
- **C1. A single-pane tab's pane is `PaneId::Detail`.**
  - That is the focus `TabView::of` already gives `Panes::Single` (`app/state.rs:159-163`). So Settings and Chat
    draw `Pane::…focus(ctx.pane_focus(PaneId::Detail))`.
  - On a bench with no shell focus, `Ctx::focus()` is `List` (M2 B7), so a bench-rendered single pane reads
    unfocused and carries no hints.
  - Only `Harness` renders (through `App`) draw the hints. `SectionBench::render_section` draws a section's
    content only (no pane), so it is unaffected.
- **C2. The shell still owns the activity placeholder (D1 refined).**
  - For a `draws_chrome` tab, `paint_panes` still draws the placeholder and restyles its border for focus. It skips
    only the List/Detail restyle.
  - When the activity pane is the focused one, the shell paints the tab's `key_hint` on the placeholder's bottom
    border. The placeholder is a shell pane, so no tab can put hints there.
- **C3. Content is drawn before the frame where a pane needs a number only the content knows**, such as a
  scrollbar total.
  - `Pane::inner(area)` gives the inner rect without drawing.
  - A ported view draws its content into it, then calls `Pane::…render`.
  - `Block::render` only touches border cells (the block has no style), so the order is safe. A T1 test pins it.
- **C4. Shared Settings strings that name a key become functions; their old consts stay public until T4 deletes
  them.**
  - `CHANGED_ELSEWHERE` has users in seven lane files.
  - A `pub(crate)` const that the last lane stops using would fail `-D warnings` (dead code) on the merged tree,
    and no single lane would see it.
  - Making it `pub` (the `settings` module is reachable from the lib root, `ui/tabs/mod.rs:7`) avoids that.
    T4 deletes it.
  - `settings::message` is already `pub`, so it is treated the same way.
- **C5. MD-A covers prose as well as hint specs.**
  - A guide or notice sentence that names a key (`type one and press Enter`, `Esc again discards`) is built from
    `Keys::label` through the mode's stack, like the `answers`/`asked` helpers (`settings/mod.rs:100-118`).
  - A literal that is a verb, not a key (`qdrant.rs:590` "Enter API key"), is reworded.
- **C6. Act choice for the converted literal hints:**
  - `Enter` on a text field → `Act::FieldSubmit`.
  - `Enter breaks the line` (a `TextArea`) → also `Act::FieldSubmit`. The shell turns `field.submit`'s chord into
    the canonical `Enter`, which the area reads as a newline, so the hint names the key that breaks the line.
  - `Esc …` in a capturing stack → `Act::FieldCancel`. Its compiled default holds `Esc` (MD7), so the text is
    unchanged under the compiled keys.
  - Every converted spec is checked against its mode's stack. Each such stack has a `[field]` layer: `CAPTURE`,
    `AGENTS_FORM`, `HIERARCHY_EDITOR`, `KINDS_EDITOR`, `BOXES_EDITOR`, `PERSONAS_FORM`, `PERSONAS_EDITOR`,
    `SECRETS_FORM`, `AREA_FORM` (`keys/views.rs:729-734`), `REQUIREMENTS_FORM`, `REQUIREMENTS_WITHDRAW`.
  - So `Keys::hint` never drops one. Each lane pins this with a test: the converted hint reads the same under the
    compiled keys, and names the rebound chord under `[field] submit = ["ctrl-t"]`.
- **C7. Under an overlay, a ported tab's pane drops focus; an unported one keeps the shell accent (review RV1 L2,
  recorded, no code change).**
  - A ported tab draws its own pane through `Ctx::pane_focus`, which answers `Unfocused` while an overlay covers
    the tab (`covered`), so its border goes faint and its hints go.
  - An unported (group B) tab is still restyled by the shell's `paint_focus`, which keeps the focused pane's
    accent under an overlay.
  - The two read differently until M4 ports group B; the ported behaviour (the overlay has the keys, so no pane
    under it reads focused) is the target, and M4's port brings group B in line with it.

## 1. Task 0: seam, neutral helpers, stubs (serial, primary tree; no snapshot may move)

### 1.1 `Tab::draws_chrome` (`ui/tabs/registry.rs`, after `restore`, `:153-155`)
```rust
/// MOD-115 M3 D1: whether this tab draws its own kit panes (title, chips, focus border, hints).
/// The shell then neither restyles its list/detail borders nor paints the hint adapter over them;
/// the hidden-pane note and the activity placeholder stay the shell's. Defaulted to `false`.
/// M4's last task deletes it with the adapter.
fn draws_chrome(&self) -> bool { false }
```

### 1.2 `Ctx` (`app/state.rs`, impl at `:189`; beside `focus()` `:296`)
```rust
/// MOD-115 M3 D1: `pane` as a kit pane is drawn, focused when it is the shell's focused pane
/// (`focus()`, so `PaneId::List` on a bench, M2 B7).
#[must_use]
pub fn pane_focus(&self, pane: PaneId) -> Focus {
    if self.focus() == pane { Focus::Focused } else { Focus::Unfocused }
}
/// MOD-115 M3 D1: the accent of the tab this context is addressed to (`Theme::tab_accent`), else
/// the theme's focus accent (an overlay's context, a test origin).
#[must_use]
pub fn accent(&self) -> Style {
    match &self.origin { Origin::Tab(id) => self.theme.tab_accent(id.0), _ => self.theme.accent }
}
```
- `Focus` is `crate::ui::kit::pane::Focus`, already imported for `paint_focus`. `Style` comes from
  `ratatui::style`.
- `SectionBench::ctx` (`testkit.rs:765-775`) is `Origin::Tab(SettingsTab::ID)`, so its sections get the
  Settings accent with no testkit change.

### 1.3 `App::render` (`app/state.rs:1514-1625`): what becomes conditional
Let `chrome = self.tabs.active().is_some_and(|tab| tab.draws_chrome())`.
- **`paint_panes`** (`:697-738`, called at `:1575-1577`) becomes `paint_panes(frame, &areas, chrome)`:
  - the placeholder branch (`:706-725`) is unchanged;
  - the focus loop (`:728-737`) skips every pane but `PaneId::Activity` when `chrome` is true;
  - when `chrome` holds, the activity pane is focused and nothing covers it (`overlays.is_empty()`,
    `!editor_visible()`), the tab's `key_hint` is drawn there with `paint_bottom_hints(buf, bottom row of
    activity, line, theme.base)` (C2).
- **Hint adapter** (`:1585-1604`): its guard gains `&& !tab.draws_chrome()`. Both its branches (with a claim and
  without one) are skipped for a ported tab.
- **`paint_hidden_note`** (`:1607-1611`, body `:745-780`) is **unchanged**:
  - `paint_bottom_note` writes onto any rule run, `╰─…─╯` included;
  - it runs after the tab drew its `Pane::hints`, so its one-rule-cell gap rule still holds (`kit/pane.rs:331-365`).
- `render_help` and the no-tab body (`:1559-1565`) are not touched; they are T4's group-B list (§9).

**Mouse and claims for a `draws_chrome` list + detail tab (Requirements).** Nothing changes in the shell:
- The tab still calls `ctx.split(region, REQUIREMENTS_LAYOUT)` (`requirements/mod.rs:1350`). That records
  `pane_claim` (`app/state.rs:260-267`), and `render` copies it into `last_claim` (`:1569-1574`).
- The tab draws one kit `Pane` into `areas.list` and one into `areas.detail`: the same rects the bare `Block`s
  used.
- So `App::on_mouse`'s pane hits, seam drags (`layout::hit`/`seam` read `PaneAreas`, never glyphs) and the
  MD8 wheel map (`App::wheel_key`) behave exactly as in M2.
- `TabView::of` (`:141-171`) and `hides` (`:176-187`) read `last_claim` as before.

### 1.4 T0 tests (`app/state.rs` tests, beside `SplitTab` `:2150-2240` and `split_app` `:2243`)
A `ChromeTab` double: `SplitTab`'s `ctx.split`, but each pane drawn as
`Pane::new(..).focus(ctx.pane_focus(id)).accent(SENTINEL)`, where `SENTINEL = Style::new().fg(Color::Red)`.
Its `key_hint` returns `j/k move`, but it draws no hints itself, and `draws_chrome() -> true`.
- `a_chrome_tab_keeps_its_own_borders`: the focused list's corner keeps `SENTINEL` (the shell would have repainted
  it with `tab_accent`). The unfocused detail keeps `theme.border_faint`.
- `a_chrome_tab_gets_no_adapter_hint`: no row of the frame contains `j/k move`.
- `the_hidden_pane_note_is_still_painted_on_a_chrome_tab`: `layout.toggle_list` puts the note on the detail's
  `╰─…─╯`.
- `the_activity_placeholder_is_still_the_shells`: with the `ide_activity` preset the placeholder is drawn. With the
  activity focused it takes `tab_accent` and the tab's hint (C2).
- `pane_focus_follows_the_shell_and_defaults_to_the_list`: on a bench, `List → Focused` and `Detail → Unfocused`.
  Under the shell, `app.focus.insert(.., Detail)` flips both.
- `accent_is_the_addressed_tabs`: under the shell it is `tab_accent("backlog")`. A bench with
  `Origin::Tab(SettingsTab::ID)` gets the Settings accent, and `Origin::Overlay(_)` gets `theme.accent`.
- `a_tab_that_does_not_draw_chrome_is_painted_as_in_m2`: the existing
  `the_focused_pane_has_the_tab_accent_and_the_other_the_faint_border` (`:2274`) and `the_hints_follow_the_focused_pane`
  (`:2307`) stay green, unedited.

### 1.5 Neutral homes for the cross-tab helpers (D2)
| Helper (today) | New home | Signature |
|---|---|---|
| `settings::wrapped` (`settings/mod.rs:139-145`) + its 5 tests (`:631-689`, `FAMILY` `:629`) | **new `ui/kit/text.rs`** | `pub fn wrapped(text: &str, width: usize) -> Vec<String>` |
| `settings::key_line` (`:121-123`) | `ui/kit/chips.rs` (after `chip_line`, `:31-41`) | `pub fn key_line(text: String, theme: &Theme) -> Option<Line<'static>>` |
| `settings::modal_rest` (`:89-95`) | `ui/kit/modal.rs` (after `impl Modal`, `:122`) | `pub fn modal_rest(stack: Stack<'_>, chord: KeyChord) -> Handled` (imports `crate::keys::{KeyChord, Stack}`, `crate::app::Handled`) |
`kit/text.rs` also gets the kit notice line (D4, deviation D-12):
`pub enum Tone { Info, Error }`, and `pub fn notice(text: &str, tone: Tone, theme: &Theme) -> Line<'static>`
(dim for `Info`, error for `Error`, no wrap). The module doc names `wrapped`'s history (M5 O-5, B7, MOD-60 D5),
moved verbatim from `settings/mod.rs:126-138`.

**Every importer** (found with a search over all of `src/`, not sampled), each line rewritten to the new path:
| File:line | Today imports | Becomes |
|---|---|---|
| `ui/tabs/backlog/divergence.rs:37` | `settings::wrapped` | `kit::text::wrapped` |
| `ui/tabs/backlog/item_form.rs:51` | `settings::wrapped` | `kit::text::wrapped` (doc comment `:1081` "the `settings::wrapped` precedent" → `kit::text::wrapped`) |
| `ui/tabs/backlog/detail/compose.rs:40` | `wrapped` | `kit::text::wrapped` |
| `ui/tabs/backlog/detail/requirements.rs:66` | `modal_rest` | `kit::modal::modal_rest` |
| `ui/tabs/backlog/detail/graph.rs:27` | `key_line` | `kit::chips::key_line` |
| `ui/tabs/backlog/detail/documents.rs:62` | `{key_line, wrapped}` | `kit::{chips::key_line, text::wrapped}` |
| `ui/tabs/backlog/detail/notes.rs:54` | `{key_line, wrapped}` | same |
| `ui/tabs/backlog/detail/runs.rs:87` | `{key_line, modal_rest}` | `kit::{chips::key_line, modal::modal_rest}` (doc links `:1023`, `:1059` name `modal_rest`: unchanged text) |
| `ui/tabs/backlog/mod.rs:62` | `{key_line, modal_rest}` | same as runs.rs |
| `ui/tabs/backlog/filter.rs:190` | doc only (`modal_rest`) | unchanged |
| `ui/tabs/requirements/mod.rs:53` | `{modal_rest, wrapped}` | `kit::{modal::modal_rest, text::wrapped}` |
| `ui/tabs/requirements/detail.rs:16` | `wrapped` | `kit::text::wrapped` |
| `ui/tabs/chat/mod.rs:45` | `key_line` | `kit::chips::key_line` |
| `ui/tabs/skills/mod.rs:27` | `key_line` | `kit::chips::key_line` |
| `ui/tabs/skills/agent_help.rs:33` | `modal_rest` | `kit::modal::modal_rest` |
| `ui/tabs/skills/attach.rs:43` | `modal_rest` | same |
| `ui/tabs/skills/library.rs:51` | `{modal_rest, wrapped}` | `kit::{modal::modal_rest, text::wrapped}` |
| `ui/tabs/skills/templates.rs:37` | `{modal_rest, wrapped}` | same |
| `ui/tabs/settings/agents.rs:93-96` | `modal_rest` in the block; `super::key_line` `:2754` | block keeps the others; `use crate::ui::kit::{chips::key_line, modal::modal_rest};` |
| `ui/tabs/settings/boxes.rs:36-39`, `:826` | `modal_rest, wrapped`; `super::key_line` | same pattern |
| `ui/tabs/settings/connection.rs:42`, `:933` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/hierarchy.rs:42`, `:1530` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/kinds.rs:45-48`, `:1590` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/personas.rs:40-43`, `:1962` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/prompt.rs:48-51`, `:921` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/qdrant.rs:12`, `:499` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/queue.rs:31-34`, `:694` | `modal_rest, wrapped`; `super::key_line` | same |
| `ui/tabs/settings/secrets.rs:47-50`, `:1545` | `wrapped`; `super::key_line` | `kit::{chips::key_line, text::wrapped}` |
`settings/mod.rs` loses the three items and the `cells` import if nothing else uses it, keeping the rest of its
helpers (`some_text`, `yes_or_no`, `is_error`, `answers`, `asked`, the CAS consts, `message`). Nothing outside
`src/` imports the three (`tests/` has no hit). `ui/kit/empty.rs:138` is a test *name* containing "wrapped", not
a use.

### 1.6 Stub modules T0 declares (`ui/kit/mod.rs:8-13`)
`pub mod filter; pub mod form; pub mod label; pub mod text;` in alphabetical order, and the module doc's part list
gains "form rows, the filter band, display labels, text".
- `form.rs`, `filter.rs` and `label.rs` are `//!` doc only, naming the task that fills them.
- `text.rs` is real (§1.5).

## 2. Task 1: kit form rows, filter band, chip overflow (serial; files `kit/{form,filter,pane}.rs`)

### 2.1 `Pane` (`ui/kit/pane.rs`)
- **Chip strip with overflow** (replaces `sub_tab_rects`' "dropped whole, from the right", `:219-251`):
```rust
/// Where the sub-tab strip sits on a pane drawn into `area` titled `title`.
pub struct ChipStrip { pub chips: Vec<Rect> /* one per title; width 0 = cut off */,
                       pub before: Option<Rect> /* the `‹` mark */, pub after: Option<Rect> /* `›` */ }
#[must_use] pub fn chip_strip(area: Rect, titles: &[&str], active: usize) -> ChipStrip;
#[must_use] pub fn sub_tab_rects(area: Rect, titles: &[&str], active: usize) -> Vec<Rect> { chip_strip(..).chips }
```
  Algorithm (pure, `cells::cell_width`):
  - Each chip is `w_i = width(title_i) + 2` cells. `avail = area.width - 2` (between the corners). `n` is the
    chip count.
  1. If `Σw ≤ avail`, every chip is shown, ending flush with the right corner: today's geometry, so
     `sub_tab_chips_end_flush_with_the_right_corner` (`:425`) stays.
  2. Otherwise a window `lo..=hi` starts at `active`. The marks cost `(lo > 0) + (hi < n-1)` cells. The window
     grows one chip at a time, **right then left, alternately**, while `used + w_next + marks_after ≤ avail`, and
     stops when neither side fits.
  3. Layout, right to left:
     - `…[‹][chip lo]…[chip hi][›]` ends at `area.right() - 1`;
     - the marks are 1 cell each, `‹` U+2039 and `›` U+203A, in `theme.dim`;
     - every chip outside the window gets `Rect::new(corner, y, 0, 1)` (no click hits it).
  4. Degenerate cases:
     - `avail < w_active + marks`: the marks go first;
     - still `avail < w_active`: the active chip is drawn clipped with `…` (`cells::clip`);
     - `avail < 3`: nothing.
  - `render` (`:135-153`) draws from `chip_strip(area, &self.sub_tabs, self.active)`. The title keeps today's
    precedence (`:155-167`, `room` up to the first drawn cell, the `‹` mark included): it gives way before any chip
    does (D-10).
- **`pub fn inner(area: Rect) -> Rect`**: `Block::bordered().inner(area)`, without drawing (C3).
- **`pub fn scrollbar_span(mut self, top: u16, height: u16) -> Self`**:
  - the scrollbar runs over inner rows `top..top + height` only (Chat's transcript, between its banner and its
    strip/composer);
  - the default is the whole inner rect;
  - `paint_scrollbar` (`:195-217`) takes the span instead of `inner`.
- Tests (sentence-named; `draw` helper `:381`):
  - `chips_that_do_not_fit_keep_the_active_one_and_mark_the_rest`: ten Settings titles at 80, `active` 0, 4 and 9;
    `‹` only when something is cut on the left, `›` only on the right; the active chip's text always present.
  - `the_rects_agree_with_what_is_drawn`: for each width 20..=120 and each `active`, the text under every non-zero
    rect is ` title `, and the marks sit at `before`/`after`.
  - `a_strip_that_fits_is_today_s_strip`: the M1 tests at `:425-503` keep their expectations. The test
    `chips_that_do_not_fit_are_dropped_whole_from_the_right` (`:480`) is rewritten: `active` 0 at width 14 keeps
    `Body` and marks `›`.
  - `the_title_gives_way_before_a_chip`: overflow at 80 with title `Agents` puts no title on the border.
  - `the_marks_are_dim_without_colour`: monochrome gives `DIM`.
  - `content_drawn_before_the_frame_survives_it` (C3).
  - `the_scrollbar_stays_inside_its_span`.

### 2.2 `kit::form` (`ui/kit/form.rs`)
```rust
//! Form rows (MOD-115 M3 D3): a shared label column, one value per row, the focus marker (`kit::list`'s gutter
//! and band), the focused row's help under it and an error under the row it concerns. Rendering only: the
//! arrow-changes-value behaviour is MOD-123.
pub enum Value<'a> {
    Text(&'a TextField),          // the field's own `line(width, focused && editing, theme)`; cursor only while editing
    Toggle(bool),                 // `● on` (base) / `○ off` (dim)
    Choice(&'a str),              // `‹ value ›`: marks dim, value base (the caller passes a `kit::label` word)
    Secret(bool),                 // true: `•••••••• stored` (dots base, word dim); false: `not stored` (dim)
    ReadOnly(Cow<'a, str>),       // plain text, wrapped under the value column (`text::wrapped`)
    Lines(Vec<Line<'static>>),    // pre-rendered rows: a `TextArea`'s `lines(..)`, a styled value; each ellipsized
}
pub struct Field<'a> { /* label, value, help, error, style */ }
impl<'a> Field<'a> {
    pub fn new(label: &'a str, value: Value<'a>) -> Self;
    pub fn help(self, text: impl Into<Cow<'a, str>>) -> Self;    // shown only while this row is focused
    pub fn error(self, text: impl Into<Cow<'a, str>>) -> Self;   // shown whatever has focus
    pub fn style(self, style: Style) -> Self;                    // whole-row base (a dimmed/withdrawn row)
}
pub struct Form<'a> { /* items: Vec<Item<'a>>, focus: Option<usize>, editing: bool, label_width: Option<u16> */ }
impl<'a> Form<'a> {
    pub fn new() -> Self;
    pub fn field(self, field: Field<'a>) -> Self;
    pub fn heading(self, text: impl Into<Cow<'a, str>>) -> Self; // a group row in `theme.title`; focusable
    pub fn blank(self) -> Self;
    pub fn focus(self, item: usize) -> Self;                     // index over fields + headings + blanks
    pub fn editing(self, on: bool) -> Self;                      // the focused row's `Text` is being typed into
    pub fn label_width(self, cells: u16) -> Self;                // share one column across forms/sections
    #[must_use] pub fn label_column(&self, width: u16) -> u16;
    #[must_use] pub fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>>;
    #[must_use] pub fn focus_line(&self, width: u16) -> Option<usize>;   // first line of the focused row (scrolling)
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, offset: usize, accent: Style, theme: &Theme);
}
```
**Row geometry**:
- A row reads `[gutter 1][label column L][gap 2][value width - 1 - L - 2]`.
- `L` is `label_width` if set, else `max(cell_width(label)) + 1`. Every field counts, those scrolled off
  included, so the column never jumps. `L` is capped at `width / 3` and is at least 1.
- Each label is `cells::fit(label, L)` (padded; a longer one is cut with `…`, MOD-60). That one column is shared by
  every row of the form; `label_width(n)` lets two forms or a form and a key-value list line up.

**Focus marker**:
- The focused item's gutter is `list::GUTTER` in `accent` (the caller passes `ctx.accent()`).
- Browsing (`editing == false`): the row's label, gap and value go through `list::selection_band`, as a list row.
- Editing: only the label column is banded. The value is the field's own line with its cursor, so the cursor
  (`theme.cursor`, REVERSED) stays visible in monochrome (a REVERSED band over a REVERSED cursor would hide it).

**Help and error**:
- Under the focused row come its help lines, then its error lines, each wrapped to the value column and indented
  to it (`1 + L + 2`). Help is `theme.dim`; errors are `theme.error`.
- An unfocused row with an error shows only the error.
- This is D3's fix for audit item 3: the help moves from ~20 rows down to the line under the field.

**Value kinds**: as in the enum above.
- A `ReadOnly` value wraps under its own column (the Connection/Secrets `push_row` rule, `secrets.rs:1581-1603`).
- A `Lines` value takes one row per line.
- A `Choice`'s word comes from `kit::label`, never `as_str()`.

**Tests**:
- each value kind's text (`● on`, `○ off`, `‹ later ›`, `•••••••• stored`, `not stored`);
- the shared column, with `label_column` and a long label cut with `…` at `width / 3`;
- the focus marker while browsing (a full band) and while editing (label band, cursor visible, checked in
  monochrome);
- the help under the focused row only, the error under its own row whether focused or not, both wrapped and
  indented;
- headings and blanks are not fields, but are focusable and numbered;
- `focus_line`;
- monochrome styles.

**How the existing rows map** (so lanes do not reinvent):
- **Connection rows** (`connection.rs:594-615`):
  - today `"  {label:<lw}  {chunk}"`, value wrapped, row in `theme.selected`;
  - becomes `Form::new().field(Field::new(row.label(), Value::ReadOnly(self.row_text(row, keys).into())))` per
    `Row::ALL`, with `.focus(self.selected)`.
  - While editing, the DSN row's value becomes `Value::Text(&editor.input)` with `.help(guide)` and `.error(notice)`
    when the notice is an error, and `.editing(true)`. The separate `pane()` field (`:618-650`) goes, and the input
    is on the row it edits.
- **Secrets form** (`secrets.rs:1606-1636`):
  - today every label is in `theme.accent` (the MOD-85 case);
  - becomes `labels.zip(fields)` → `Field::new(label, Value::Text(field))`, `.focus(focus).editing(true)`, so the
    accent marks the focused row only.
  - `push_row` (`:1581`) rows become `ReadOnly` fields of the same form.
- **Queue rows** (`queue.rs:770-772` `value_line`):
  - today `"  {label:<14}  {value}"`;
  - becomes `Field::new(label(target.key()), Value::ReadOnly(shown(..).into()))`, with `.heading(..)` for the
    `project …` / app / `this box (…)` header rows and `.label_width(14)` to keep today's column.
  - The editor's separate pane (`:529-542`, label `"{label} ({unit}): "`) becomes `Value::Text` on the cursor row,
    with `.help(unit)`.
- **Requirements forms** (`requirements/forms.rs:607-621` `field_line`, `LABEL = 15` `:31`):
  - `.label_width(15)`; `code`/`title`/`deciding item`/`key` → `Value::Text`;
  - `priority` → `Value::Choice(label::priority(p))`. This replaces the `[must]  later ` pair, whose accent is a
    MOD-85 case (`:434-440`).
  - `body`/`rationale` → `Value::Lines(area.lines(w, h, focused, theme))`, with `h` from the lane's `Layout` split
    of the remaining rows (today's `Fill(2)`/`Fill(1)`, `:399-407`);
  - the area code rule (`:143-146`) and the withdraw sentence (`:562-565`) → `.help(..)` on their rows.
- **Settings editors** (`agents.rs:2262-2287`, `hierarchy.rs:1599-1624`, `kinds.rs:1672`, `personas.rs:1452`):
  `"{label}: "` padded, focused label accented → `Field::new(label, Value::Text(&field.input))`.
  `PathsForm::lines` (`agents.rs:2362`) keeps its third-of-width cap, which is `Form`'s own cap.

### 2.3 `kit::filter` (`ui/kit/filter.rs`)
```rust
//! The filter band (MOD-115 M3 D3): `/ query`, a capturing marker while typing, `n of m`.
pub enum Filter<'a> {
    Typing { field: &'a TextField, shown: usize, total: usize },
    Applied { query: &'a str, shown: usize, total: usize },
}
#[must_use] pub fn line(filter: &Filter<'_>, width: u16, accent: Style, theme: &Theme) -> Line<'static>;
pub fn render(frame: &mut Frame<'_>, area: Rect /* one row */, filter: &Filter<'_>, accent: Style, theme: &Theme);
```
- **Typing**: `▌` (`accent`), then `/ ` (`theme.key`), then the field's `line(room, true, theme)`, then
  `{shown} of {total}` right-aligned in `theme.dim`.
- **Applied**: ` / ` dim, the query in `theme.base`, and the count. An empty applied query is no band at all: the
  caller does not reserve the row.
- The count is never cut; the field gives way.
- Tests: both states, the count right-aligned and kept at narrow widths, the field clipped first, the marker
  only while typing, monochrome.
- Requirements is its only M3 user. Backlog's filter form adopts it in M4.

## 3. Task 2: display labels (`ui/kit/label.rs`; `kit/status.rs` keeps `TABLE`)
```rust
/// A status as shown: one glyph, one human word, and how it reads (its style through [`tone_style`]).
pub struct Mark { pub glyph: &'static str, pub word: &'static str, pub tone: Tone }
pub enum Tone { Done, Running, NeedsYou, Failed, Idle, Dim }
#[must_use] pub fn tone_style(tone: Tone, theme: &Theme) -> Style;   // Done/Dim → dim, Running → running,
                                                                     // NeedsYou → warning, Failed → error, Idle → base
#[must_use] pub fn spans(mark: Mark, theme: &Theme) -> Vec<Span<'static>>;  // glyph, space, word
#[must_use] pub fn plural(count: usize, one: &str, many: &str) -> String;  // "1 option", "3 options"; never "(s)"
```
- Every function below is an **exhaustive `match`** with no wildcard, so a new variant fails to compile.
- Tests go table by table over each enum's `ALL` (both macros generate it, `htui-core/src/model/mod.rs:47`,
  `htui-agent/src/lib.rs:87`; `BudgetSource` has none, so it gets a hand list):
  - no label contains `_` (D-4);
  - every multi-word value is relabelled;
  - every glyph is one cell;
  - the plural cases 0, 1, 2.

| Enum (crate path) | Shown at today (raw) | fn | Label / mark per variant |
|---|---|---|---|
| `StopReason` (`crates/htui-agent/src/event.rs:96`) | `chat/transcript.rs:652-655` `turn ended ({end_turn})` | `stop_reason(r) -> &str` | end_turn `turn ended` · max_tokens `token limit reached` · max_turn_requests `turn limit reached` · refusal `refused` · cancelled `cancelled` |
| `ToolKind` (`event.rs:44`) | `transcript.rs:584` `{kind.as_str()} {title}` | `tool_kind(k) -> &str` | read · edit · delete · move · search · execute `run` · think · fetch · switch_mode `switch mode` · other `tool` |
| `PlanEntryStatus` (`event.rs:129`) | `transcript.rs:641` `[{status}]` | `plan_entry(s) -> Mark` | pending `○ pending` Idle · in_progress `◐ in progress` Running · completed `● done` Done |
| `RunStatus` (`crates/htui-core/src/model/run.rs:35`) | no group-A site (Runs, group B) | `run_status(s) -> Mark` | queued `◌ queued` · running `◐ running` · awaiting_approval `⚠ needs you` · stopped `⊘ stopped` · done `● done` · failed `✗ failed` · cancelled `⊘ cancelled` |
| `StepStatus` (`run.rs:104`) | none in group A | `step_status(s) -> Mark` | pending `○ pending` · running · awaiting_approval `⚠ needs you` · stopped · done · failed · cancelled (as `RunStatus`) · superseded `↷ superseded` (Dim) |
| `Transport` (`crates/htui-core/src/model/agent.rs:11`) | `settings/agents.rs:1981` | `transport(t) -> &str` | acp `ACP` · cli `CLI` |
| `Billing` (`agent.rs:21`) | `agents.rs:1982` | `billing(b) -> &str` | subscription `subscription` · per_token `per token` |
| `ProbeStatus` (`crates/htui-agent/src/probe.rs:290-302`) | `agents.rs:1335-1342` (raw `missing`/`unauthenticated`/`failed` in the 13-wide cell) | `probe(s) -> Mark` | ready `● ready` Done · unauthenticated `⚠ needs login` NeedsYou (13 cells, fits the column) · missing `✗ missing` Failed · failed `✗ failed` Failed (D-18) |
| `BudgetSource` (`crates/htui-core/src/prompt/settings.rs:613`) | `settings/prompt.rs:1023` (and `present_source` `:1010`, `switch` `:986`) | `setting_source(s) -> &str` | phase `phase` · project `project` · app_setting `app setting` · app_setting_default `default` |
| `Priority` (`crates/htui-core/src/model/requirement.rs:14`) | `requirements/tree.rs:316`, `detail.rs:76`, `forms.rs:436-438` | `priority(p) -> &str` | must · later |
| `RequirementState` (`requirement.rs:24`) | `detail.rs:76` (`must · active · v2`) | `requirement_state(s) -> Mark` | active `● active` Idle · withdrawn `✕ withdrawn` Dim (the tree's `WITHDRAWN_MARK` U+2715, `tree.rs:32`) |
| `CitationKind` (`requirement.rs:34`) | `detail.rs:123` (`KIND_WIDTH` 9) | `citation(k) -> &str` | addresses · amends · withdraws · reserves |
| `Resolution` (`crates/htui-core/src/model/item.rs:73`) | `detail.rs:116-119` | `resolution(r) -> &str` | done · concluded · rejected · withdrawn · superseded · duplicate |
| `Status` (item) | `detail.rs:110`, `:138` | **`kit::status::mark`/`spans`** (exists) | `● done` `◐ running` `⚠ needs you` … |
| `Gate` (`crates/htui-core/src/model/kind.rs:16`) | `settings/kinds.rs:2017` | `gate(g) -> &str` | always · on_failure `on failure` · never |
| `Isolation` (`kind.rs:28`) | `kinds.rs:2018-2020` | `isolation(i) -> &str` | worktree · copy · shared_serialized `shared, one at a time` · local `none (local tree)` |
| `CommandQueue` (`kind.rs:43`) | `kinds.rs:2021` | `command_queue(q) -> &str` | off `not queued` · fan_out_only `queued when fanned out` · always `always queued` |
| `OsFamily` (`crates/htui-core/src/model/box_.rs:14`) | `settings/boxes.rs:996` (`{os_family}` Display) | `os_family(o) -> &str` | windows `Windows` · linux `Linux` · macos `macOS` |
Rows that are not kit enums:
- Chat's `CallStatus` (`chat/transcript.rs:22`; words at `:568-578`) is a chat-local enum, so its `Mark` is built
  in `transcript.rs` (L-chat): `◐ running` · `● done` · `✗ failed` · `✗ rejected` · `⊘ cancelled`. The kit
  never imports a tab (D-16).
- `kinds.rs:1987-1993` `gate soft`/`hard`, `in none` and `budget inherit` are booleans or options, not enums, and
  their copy is L-kinds' (§5).

## 4. Task 3: the Settings frame (serial; `settings/mod.rs` + the 19 frame snapshots)

### 4.1 `SettingsTab::render` (`settings/mod.rs:487-504`)
```rust
fn draws_chrome(&self) -> bool { true }
fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>) {
    let titles: Vec<&str> = self.sections.titles().into_iter().map(|(_, title)| title).collect();
    let title = self.sections.active().map_or("Settings", SettingsSection::title);
    let inner = Pane::inner(area);
    match self.sections.active() {
        Some(section) => section.render(frame, inner, ctx),
        None => kit::empty::render(frame, inner, NO_SECTIONS, NextStep(NO_SECTIONS_NEXT), ctx.theme),
    }
    let mut pane = Pane::new(title).sub_tabs(&titles, self.sections.active)
        .focus(ctx.pane_focus(PaneId::Detail)).accent(ctx.accent());
    if let Some(hints) = self.sections.active().and_then(|s| s.key_hint(ctx.keys(), ctx.theme)) { pane = pane.hints(hints); }
    if let Some((total, offset)) = self.sections.active().and_then(SettingsSection::scroll) { pane = pane.scrollbar(total, offset); }
    pane.render(frame, area, ctx.theme);
}
```
- The `┌ Settings ─` block (`:488-490`) and the in-body strip are deleted: the `Layout` at `:492-494` and
  `render_strip` at `:507-524`.
- The test `active_section_is_bold_and_underlined_without_colour` (`:538-558`) is rewritten over the pane's chips:
  the active chip is `BOLD | UNDERLINED`, the others are not.
- The `Block`, `Borders`, `Paragraph`, `Wrap`, `Layout`, `Constraint`, `Span` and `Style` imports go when unused.
- The no-section text (`:497-502`) becomes `kit::empty` with a next step. `with_sections(vec![])` is a test path
  only; `register_all` always registers ten.
- **Sections receive the pane's inner rect.** `SettingsSection::render`'s doc (`:220-221`, "Draws below the
  section strip") becomes "Draws inside the Settings pane's inner rect". A section gets the full inner height:
  one row more than today, since the strip row is gone. The width is unchanged (98 at the harness's 100 columns,
  so `agents.rs:207` `NOTE_WIDTH` and `personas.rs:193` still hold).
- **Hints**: the active section's `key_hint(keys, theme)` (`:224-226`) goes to `Pane::hints`. It is the text the
  adapter painted, now drawn by the pane on the bottom border, clipped to `width - 4`. `Tab::key_hint` (`:477-481`)
  stays for the `Tab` contract; the shell no longer calls it for Settings.
- **New trait method** (`SettingsSection`, `:180-231`), defaulted: `fn scroll(&self) -> Option<(usize, usize)>
  { None }` returns (total rows, offset) as of the last render.
  - A scrolling section sets it through a `Cell` in its render: prompt `:946-953`, queue `:723-731`, kinds,
    hierarchy, agents, personas.
  - The lanes implement it (C3, D-11).
- **Notices and `message()`**:
  - T3 converts the tab's own empty case (above).
  - It adds `kit::text::notice` (T0) as the notice line sections switch to.
  - It keeps `pub fn message` (`:527-532`) unchanged for the lanes. Each lane replaces its `message(..)` calls with
    `kit::empty::render(.., NextStep(..))` and its notice rows with `kit::text::notice`.
  - T4 deletes `message` once unused (D-6).
- **`CHANGED_ELSEWHERE` → keys** (MD-A, C4, D-5):
```rust
/// The compare-and-set miss while an editor is open (M4 D8): reloaded, text kept; `retry` is the label
/// of the key that saves again through the editor's stack (`field.submit`, or `form.save` for a
/// `TextArea` editor), `None` when unbound.
pub(crate) fn changed_elsewhere(retry: Option<&str>) -> String {
    match retry {
        Some(key) => format!("changed elsewhere since you opened it \u{2014} reloaded; {key} retries against the current row"),
        None => "changed elsewhere since you opened it \u{2014} reloaded; save again to retry against the current row".to_owned(),
    }
}
```
  - The text under the compiled keys is byte-identical to `:149`.
  - `pub(crate) const CHANGED_ELSEWHERE` becomes `pub const`, with a doc saying it is transitional (C4). T4 deletes
    it.
  - `is_error` (`:83-85`) is unchanged: the text still starts `changed elsewhere`.
  - The doc comment at `:400-404` saying "`shift-right` no longer cycle" is fixed (M2 D-15's carried note).
- **`CHANGED_ELSEWHERE_CLOSED`/`DELETED_ELSEWHERE`** name no key and are unchanged.

### 4.2 T3 tests (in `settings/mod.rs`)
All are `TestBackend` tests through a bench `Ctx` with `with_focus(PaneId::Detail)`, so T3 edits no lane file:
- one rounded pane titled by the section;
- ten chips;
- at 80 columns the active chip is visible and `‹`/`›` marked;
- the section's hint is on the bottom border, focused only;
- no `┌`;
- the scrollbar follows `scroll()`;
- `changed_elsewhere(Some("Enter")) == old CHANGED_ELSEWHERE`, and it names a rebound key;
- `draws_chrome()`.
`tests/kinds.rs:865` and `tests/prompt_settings.rs:906` assert `" Agents  Hierarchy  Kinds "` on the frame. At 100
columns all ten chips fit (87 of 98 cells), adjacent, so the substring holds. T3 confirms it, and if it fails it
fixes them (serial, before their lanes).

### 4.3 Snapshots T3 re-accepts (19; the other 49 Settings snapshots are bench renders, untouched: D-1)
- `box_settings__{demo,offline}`
- `hierarchy__{delete_typed,delete_warn,demo,editor_repo,no_workspace,offline}`
- `kinds__{demo,no_workspace,offline}`
- `personas__offline`
- `probe__agents_probed_missing`
- `prompt_settings__{app_only,demo,offline}`
- `settings__agents_{demo,empty,unknown_row}`
Per-file review: only the frame changes. That means the rounded corners, the section name as title, the chips in
the top border, the strip row gone (bodies one row up, plus one row at the bottom) and the hints on `╰─`. The body
text is identical row for row.

## 5. Wave lanes

Every lane:
- converts its forms' act resolution to `ctx.pressed(&key)` (the sites listed);
- converts every literal key hint (C6) and key-naming sentence (C5);
- moves status/enum text to `kit::label`;
- replaces `message`/notice rows with `kit::empty`/`kit::text::notice`;
- adds one 80-column `Harness` frame snapshot (`.size(80, 24)`) per tab or section;
- runs the **whole** `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast` before each merge
  (D-8), accepting snapshots only from its own files.

### 5.1 L-req: `requirements/*`, `tests/requirements.rs`, `requirements__*`, **+ `tests/reveal.rs`** (D-7)
- **Self-drawn sites**:
  - `mod.rs:982` tree `Block` → `Pane::new(&format!("Requirements ({count})")).focus(ctx.pane_focus(PaneId::List))
    .accent(ctx.accent()).scrollbar(lines.len(), offset)`, plus `.hints(..)` when the list is focused. The title
    drops ` · /{filter}` (`:977-981`); the band shows it. `tests/reveal.rs:189` pins `" Requirements (3) ─"`, which
    the rounded border still satisfies.
  - `mod.rs:1062` detail `Block` → `Pane::new(title)`, the same title text without its padding spaces,
    `.focus(ctx.pane_focus(PaneId::Detail))`, `.scrollbar(self.pane_rows, self.scroll.offset())`, `.hints(..)` when
    the detail is focused. C3: lines first, then the frame.
  - `mod.rs:1162-1169` `filter_row` (`" /"` in `theme.accent`, a MOD-85 case) and the reserved bottom row
    (`:1342-1347`, `:1377`) → `kit::filter` as the tree pane's first inner row, only while typing or applied:
    `Filter::Typing{field, shown: tree::count(.., filter), total: tree::count(.., "")}`.
  - `mod.rs:1368-1376` notice → `kit::text::notice` rows under the panes (kept outside them, M2 B5).
  - `tree.rs:185-254` `lines` + `pad` (`:334-340`, its own `theme.select`) → `kit::list::Row`s:
    - the project header is `RowKind::Group{folded}`, with today's `project_header` line (`:260-293`) as its
      `line`, since `list::group_row`'s fixed format has no indent or tail;
    - the area header is a `Group` row with its two-cell indent;
    - a requirement is an `Item`;
    - `no areas` (`:221-227`) is a `Note`;
    - drawn with `kit::list::render(frame, inner, &rows, cursor, offset, ctx.accent(), theme)` over
      `inner.width - 1` (the gutter). `window` (`backlog::list`) keeps the offset (D2: imported, not edited).
  - `forms.rs:125-149`, `:390-465`, `:557-591`, `field_line` `:607-621` and `label_style` `:602-604` → `kit::form`
    (§2.2).
- **Hints (MD-A)**:
  - `mod.rs:127` `Hint::Text("Esc clear")` → `Hint::One(Act::FieldCancel, "clear")` (`CAPTURE`, `:305`);
  - `:137` → `Hint::One(Act::FieldCancel, "cancel")` (`AREA_FORM`, `REQUIREMENTS_FORM`);
  - `:152` the same (`REQUIREMENTS_WITHDRAW`).
- **`Ctx::pressed`**: `on_form_key` (`:586-590`) and `RequirementForm::on_pressed` (`forms.rs:319`) already
  resolve from it (M2 RV1). `on_filter_key` `:572` `modal_rest(stack, KeyChord::from_event(key))` →
  `ctx.pressed(&key)`.
- **MOD-85 accents**:
  - the filter ` /` → the band's capturing marker in the tab accent (focus);
  - the form `[value]` (`forms.rs:436`) → `Value::Choice` (value base, marks dim);
  - `label_style` → the form's focus marker.
  - Buffer tests pin all three, in colour and in monochrome.
- **MOD-83 copy**:
  - `detail.rs:73-79` `must · active · v2` → `label::priority` · `label::requirement_state` mark · `v2`;
  - coverage `:110`/`:138` → `kit::status::spans` (width from the marks), `:123` `label::citation`, `:116-119`
    `label::resolution`;
  - `:184-188` `{n} areas · {m} requirements` and `:214` → `plural`;
  - `tree.rs:316` → `label::priority`.
  - Empty states get a next step:
    - `NO_PROJECTS` (`mod.rs:88`) → "Settings › Hierarchy adds a project";
    - `NOTHING_MATCHES` (`:85`) → "{FieldCancel label} clears the filter";
    - `NOT_READ` (`:82`) → "they appear once the store answers";
    - `SELECT_A_ROW` (`:91`) → "{list.down/up} chooses one".
  - `REVISIONS_NEED_THE_DATABASE` (`detail.rs:19`) gains "· Settings › Connection".
- **MOD-81**: one 80-column snapshot.
- **Re-accepts**: all 7 `requirements__*`, plus new ones (80 columns; the filter band typing and applied).
- **Coupling**:
  - `tests/reveal.rs:67-69` `titled(key) = "┌ {key} "` reads both the Backlog and the Requirements detail titles
    (`:166`, `:195`, `:218`). The lane makes it accept `┌ {key} ` or `─ {key} ` (the kit's `╭─ KEY `), so the
    Backlog half stays green until M4.
  - `backlog::detail::{Scroll, Step, capture_area, STAMP}` and `backlog::list::window` are imported only.

### 5.2 L-chat: `chat/*`, `tests/chat.rs`, `chat__*`, `backlog__runs_pending_permission.snap`
- **Self-drawn sites**:
  - `mod.rs:810` `Block … " Chat "` → `Pane::new(&title).focus(ctx.pane_focus(PaneId::Detail)).accent(ctx.accent())
    .hints(key_line(hint_text(..)))`, then `.scrollbar(total, first).scrollbar_span(body.y - inner.y, body.height)`.
  - The title is `self.header(ctx, area.width.saturating_sub(6))` (`:406-460`) as plain text, or
    `replay_header`'s (`:319-333`) text. Pre-fitting it to the title's room means the pane never clips it, so the
    promoted header's guarantee that the session ref is never cut (`:418-436`) holds.
  - The header row (`:822-835`) goes: the layout becomes `[banner?, body, strip?, composer]` inside the pane. The
    composer is the pane's last inner row (D5).
  - The replay's ` · {Back label} leave` part leaves the title; `REPLAY_HINT` (`:67-71`) already names
    `Back leave replay` on the bottom border.
  - The transcript's `Scroll::At` (`transcript.rs:525`) is kept. The bar's total is `self.rendered`, and its offset
    is the first line drawn.
- **Hints (MD-A)**:
  - `:75` `Hint::Text("Esc leave")` → `Hint::One(Act::FieldCancel, "leave")` (`CAPTURE`);
  - `:86` "this chat has ended" is a note, not a key, and stays.
- **`Ctx::pressed`**: `on_key` `:584` `KeyChord::from_event(key)` → `ctx.pressed(&key)`. That covers the composer
  path under `CAPTURE`, so a rebound `[field]` chord the composer passed is not re-read as a chat act.
  `on_key_replay` (`:278-315`) has no `Ctx` and no field, and stays.
- **MOD-85 accents (decided, buffer-pinned)**:
  - the header agent / promoted lead (`:438`, `:454`) and the replay header (`:329`) move into the pane title,
    which is the tab accent, bold when focused;
  - transcript `you  ` (`transcript.rs:540`) → `theme.title` (a speaker label, not focus);
  - an unanswered `permission …` line (`:633`) → `theme.warning` (needs a person, MOD-80 D1);
  - the resume-failed note (`:661`) → `theme.warning`;
  - the permission digits (`permission.rs:70` `[{label}]` in accent) → `kit::chips::key_chip(label)`;
  - the composer `> `/`_` (`composer.rs:119`, `:121`) → `ctx.accent()` (focus, the one legitimate use). This needs
    an `accent: Style` parameter on `Composer::render` (`:114`).
- **Permission strip chrome** (`permission.rs:61-85`, D5):
  - `answer  ` stays in `theme.title`;
  - each option becomes `key_chip(label)` + ` ` + its label (base) + ` (agent remembers)` (dim) where it applies;
  - the digits stay `answer.option_*`.
  - `PermissionStrip::line` keeps its signature, so `backlog/detail/runs.rs:86-87` needs no edit, and only
    `backlog__runs_pending_permission` changes outside `chat__*` (D5). Accept it with
    `cargo insta test --all-features --test backlog`, that snapshot only.
- **MOD-83 copy**:
  - `transcript.rs:584` → `label::tool_kind`;
  - `:568-587` `[ok]`/`[…]` → the `CallStatus` mark (D-16);
  - `:641` `[{status}]` → `label::plan_entry` spans;
  - `:653` `— turn ended ({end_turn}) —` → `— turn ended —` for `EndTurn`, else `— turn ended: {label} —`;
  - `:628` `({n} options)` → `plural(n, "option", "options")` (MOD-83's `1 options`);
  - `mod.rs:328` `{n} rows` → `plural`.
  - Empty states get a next step:
    - "no agent is registered on this box" (`:875`) → "Settings › Agents adds one";
    - "type a prompt to start a chat" (`:877`) → "{ChatCompose label} composes the first prompt".
- **MOD-81**: one 80-column snapshot.
- **Re-accepts**: all 15 `chat__*` and `backlog__runs_pending_permission`.
- **Coupling**: `Composer::render`'s new parameter is only called from `chat/mod.rs:895`. `chat_live*.rs` and
  `chat_offline.rs` assert data, not frames (checked: no frame `contains` on chrome).

### 5.3 L-agents: `settings/agents.rs → settings/agents/*`, `tests/settings.rs`, `tests/probe.rs`, `settings__*`, `probe__*`, **+ `tests/auth.rs`, `tests/install.rs`** (D-7)
- **D7 split (first commit, no behaviour change; `git mv agents.rs agents/mod.rs`, then extract)**:
  - `agents/mod.rs`:
    - module doc (`:1-63`), imports, the hint specs (`:121-247`), `KeyMode`/`Mode` (`:264-273`, `:418-430`);
    - `AgentsSection` (`:517-548`);
    - `new`/`key_mode`/`stack`/`hint_text`/`note`/`refuse_write`/`open_*`/`send`/`submit*`/`on_browse_key`/
      `on_editor_key`/`on_paths_key`/`on_written` (`:550-602`, `:1447-1966`);
    - `form_navigation`, `swallows`, `in_flight` (`:2396-2428`, `:2503-2512`);
    - `impl SettingsSection` (`:2514-2786`).
  - `agents/table.rs`: `render_table` (`:1969-2086`), `on_box_cell`/`lists_a_manual_row` (`:1314-1368`), cursor
    `selected`/`move_cursor`/`clamp_cursor` (`:561-601`), `quota_cell` and its window helpers (`:2813-2918`),
    `is_manual` (`:2433`), and the column consts `NOT_PROBED`, `PROBING`, `SWITCHED_OFF`, `NONE`, `FULL`,
    `NEARLY_FULL`, `MANUAL_SUFFIX` and `MANUAL_NOTE`.
  - `agents/form.rs`:
    - `Editor`, `Target`, `Field`, `PathsForm`, `PathField` (`:432-516`) with `impl Editor` (`:2089-2288`), `impl
      PathsForm` (`:2290-2389`) and `impl Field` (`:2441-2454`);
    - `prefill`, `clash_notice`, `needs_cli_block` (`:2456-2512`);
    - `UNCHANGED`, `LAUNCH_UNREADABLE`, `GONE_CLOSED`, `NEEDS_CLI_BLOCK`, `CHANGED_ON_BOTH_SIDES`, `NOTE_WIDTH`;
    - `tool_paths_saved` (`:2970`);
    - the in-file test `a_wide_tool_name_is_fitted_to_the_label_column` (`:3028-3064`).
  - `agents/flows.rs` (the install, login, paste and probe flows):
    - `InstallState`/`AuthState` (`:315-416`);
    - `install_in_flight`…`on_install_frame` and `install_cell`/`probe` (`:602-1313`, `:1370-1390`, `:1578-1600`);
    - `auth_pane` (`:1074-1160`), `pane` (`:1405-1439`);
    - `phase_cell`, `outcome_notice`, `entry_id`, `first_line`, `human_bytes` (`:2919-2997`);
    - `declares_a_source` (`:2793`);
    - `quota_note`/`reprobes` and their test (`:153-158`, `:212-217`, `:3006-3020`);
    - the flow consts.
  - Each child module has `impl super::AgentsSection { pub(super) fn … }`. Children see the parent's private
    fields (Rust privacy), so no field becomes `pub`.
  - `settings/mod.rs:14` `pub mod agents;` and `:42` `pub use agents::AgentsSection;` are unchanged. Nothing outside
    the section names `settings::agents::…` (checked: no hit in `src/` or `tests/`).
  - The commit's gate is the unchanged `tests/settings.rs`, `probe.rs`, `auth.rs`, `install.rs` and the in-file
    tests, all green.
- **Self-drawn sites (after the split)**:
  - the table (`Table`, `row_highlight_style(accent)`, `:2066-2085`) → a header line in `theme.title`, then
    `kit::list::render` rows:
    - each row's cells are fitted to widths from `Layout::horizontal(constraints).spacing(1)` over
      `inner.width - 1`;
    - the offset is `list::scroll_into_view(selected, 0, height, total)`. That matches today's per-frame
      `TableState` clone, which starts from offset 0 each frame.
  - Pane lines (`:1405-1439`) and the chooser cursor (`:1083-1089` accent) → `kit::list` rows with the band.
  - The paste field (`:1131-1138`, the `› ` accent marker) → a one-row `kit::form` (`Value::Text`, editing).
  - Editor and paths lines (`:2262-2287`, `:2362-2388`) → `kit::form`.
  - `render` (`:2757-2785`): `message` `:2767`/`:2769` → `kit::empty` with a next step ("Settings › Connection
    stores a DSN" / "{New label} adds one"); the note `:2776-2784` → `kit::text::notice`.
  - `scroll()` (§4.1) is implemented over the table.
- **Hints (MD-A)**:
  - `:164-165` → `Hint::One(FieldSubmit, "saves"), Hint::One(FieldCancel, "cancels")` (`AGENTS_FORM`);
  - `:247` `HINT_PASTING` → `Hint::One(FieldSubmit, "sends"), Hint::One(FieldCancel, "cancels")` (`CAPTURE`);
  - `:203` `CHANGED_ON_BOTH_SIDES` → `fn changed_on_both_sides(retry: Option<&str>)`, built through
    `keys.label(AGENTS_FORM, FieldSubmit)`;
  - `:1917` `CHANGED_ELSEWHERE` → `changed_elsewhere(keys.label(AGENTS_FORM, Act::FieldSubmit))`. `on_written` has
    `keys`.
- **`Ctx::pressed`**:
  - `form_navigation(key, keys, …)` (`:2396-2413`, chord `:2399`) → `form_navigation(chord: KeyChord, keys, …)`.
    Its callers in `on_editor_key`/`on_paths_key` pass `ctx.pressed(&key)`.
  - `on_paste_key` `:897` → `ctx.pressed(&key)`.
  - The consent and chooser chords (`:702`, `:1208`) resolve from non-capturing stacks. Converting them is harmless
    and kept uniform.
- **MOD-85**:
  - the table highlight → the `kit::list` band (selection: legitimate);
  - the form labels → the form marker;
  - the chooser cursor → the band.
- **MOD-81**:
  - **The name column keeps a minimum width.** `kit::list`'s gutter costs one column, so at the harness's 100 the
    current packing would give `name` 9, below `claude-cli`'s 10.
  - The column is `Min(10)`. `on this box` is **not** the donor (D-18): it stays 13, the width of the approved
    `⚠ needs login`. The cell comes from another column (L-agents picks it, pinned by the 80-column snapshot), or
    the maintainer is asked again.
  - Below 98 cells, columns drop in this order until `name ≥ 10`: `models`, then `billing`, then `transport`.
    `default` (the model id `docs/ANA-4.md` §4.4 selects by) and `quota` stay.
  - Pin this with an 80-column snapshot and a unit test over widths 60..=160.
- **MOD-83**:
  - `:1981-1982` → `label::transport`/`billing`;
  - `:1335-1342` → `label::probe` spans in the cell (`⚠ needs login` is 13 cells and fills the 13-cell column; a failed probe reads `✗ failed`, D-18);
  - `:1992` enabled `yes`/`no` → `● on`/`○ off`;
  - `:1108` `{hidden} method(s)` → `plural(hidden, "method needs", "methods need")`;
  - `:2996` `{bytes} bytes` → `plural`;
  - the error copy "agent registry needs Postgres" (`:2767`) gains its next step.
  - Raw `transport`/`billing` *inputs* in the editor (`:2097`, `:2102`, `:2491`, `:2496`) are what the user types:
    unchanged, since editing by choice is MOD-123.
- **Re-accepts**:
  - all 10 `settings__agents_*`, including the 7 bench ones (`create_form`, `edit_form`, `paste_redirect`, `probed`,
    `quota`, `switched_off`, `tool_paths_form`);
  - `probe__agents_probed_missing`;
  - a new `settings__agents_80`.
- **Coupling** (hence D-7):
  - `tests/auth.rs` reads the `on this box` cell by fixed offsets (`on_box_cell`, `ON_BOX_AT`/`ON_BOX_WIDTH`) and
    asserts `logged in: unauthenticated` (`:533`) and `a authenticate` (`:515`);
  - `tests/install.rs` does the same (`:408-430`) and asserts `y install · n cancel` (`:467`);
  - `tests/settings.rs:287-400` holds the same constants;
  - all three move by +1 (the gutter) with the donor's new width, in this lane.

### 5.4 L-kinds: `settings/kinds.rs`, `settings/personas.rs`, `tests/kinds.rs`, `tests/personas.rs`, `kinds__*`, `personas__*`
- **Self-drawn sites**:
  - Kinds: `lines` (`kinds.rs:1227`) tree rows → `kit::list` with group rows; editor `lines` (`:1672`, accent
    `:1684`) → `kit::form`; `pane` (`:1417`); notice `:1294`; `render` `:1593` (`message` `:1613`, `:1615` →
    `kit::empty`);
  - Personas: `notice_lines` (`personas.rs:1245`, `path: ` accent `:1248`) → a `kit::form` row; editor `lines`
    (`:1452`, `:1463`) → `kit::form`; `render` `:1965`.
- **Hints**:
  - `kinds.rs:85-86` → `FieldSubmit save` / `FieldCancel cancel` (`KINDS_EDITOR`);
  - `personas.rs:71-72`, `:78-79`, `:91-92` (`Enter breaks the line` → `FieldSubmit`, C6);
  - `:85` `Esc back to the fields` → `FieldCancel`;
  - `:102` `HINT_IMPORT`;
  - prose: `:138` `UNSAVED` "Esc again discards" → `fn unsaved(cancel: Option<&str>)`;
  - `:165` `CHANGED_ON_BOTH_SIDES` → a function, as in Agents.
  - CAS: `kinds.rs:1181`, `:1206` are in `on_stale(&mut self, snapshot)` (`:1161`), which has no `&Keys`; it gains
    `keys: &Keys` from its `on_reply` caller. `personas.rs:1117` already has `keys`.
- **`Ctx::pressed`**:
  - Kinds `on_editor_key` `:799`, `on_deleting_key` `:733`, `on_confirm_key` `:878`;
  - Personas `on_editor_key` `:592`, `on_import_key` `:919`, `editor_act` `:1691` (`fn editor_act(key, …)` takes
    the chord instead), `on_delete_key` `:869`, `on_report_key` `:928`.
- **MOD-85**: the form labels go to the marker; the import path label goes to a form row.
- **MOD-83**:
  - `phase_line` (`kinds.rs:1981-2000`) `gate soft · in none · budget inherit` → `soft gate` / `hard gate`, `no
    inputs` / `inputs plan, review`, `inherits the budget` / `budget {n} tokens`;
  - `detail_lines` (`:2013-2034`) → `label::gate`, `isolation`, `command_queue`, with readable field names
    (`fan-out 1 · gate: never · isolation: none (local tree) · commands: not queued · verify: none · retries 0 ·
    output: … · template: v—`). `MOD_4_OWNS` is kept.
  - Errors: `UNAVAILABLE` "catalogue unavailable" (`:59`) and `personas unavailable` (`personas.rs:126`) gain next
    steps.
- **Re-accepts**: `kinds__*` (9), `personas__*` (7), plus two 80-column snapshots.
- **Coupling**: `tests/kinds.rs:865` strip assert (T3 keeps it passing). `tests/personas.rs:13` imports
  `settings::personas::{…}` (public consts): kept public.

### 5.5 L-hier: `settings/hierarchy.rs`, `ui/path_picker.rs`, `settings/boxes.rs`, `tests/hierarchy.rs`, `tests/box_settings.rs`, `hierarchy__*`, `box_settings__*`
- **Self-drawn sites**:
  - Hierarchy: `lines` (`hierarchy.rs:394`) → `kit::list`; `pane` `:450`; editor `lines` `:1599-1624` →
    `kit::form`; `render` `:1533-1561` (`message` `:1543`, `:1548` → `kit::empty`).
  - **Path picker** (`path_picker.rs:284-321`, `Block` + `Borders` `:309-310`) → `kit::modal::Modal::new(&title)
    .size(Fixed{..})`, which keeps today's box (`MAX_WIDTH` 96, `centered` `:294-298`):
    - the body (`:389-440`, `> ` cursor `:28`, accent row `:434`: MOD-85's "cursor row with accent alone") →
      `kit::list` rows with gutter and band;
    - the path header (`:377` accent) → `theme.title`;
    - the hints footer stays in-box text through `Keys` (`HINT_MOVE`/`HINT_MORE`, `:37-54`, already keyed);
    - the go-to field (`GOTO` `:57`) → a one-row `kit::form`.
  - Boxes: `render` `:829` (`message` `:883`, `is_error` `:902`) → kit.
- **Hints**:
  - `hierarchy.rs:87-88`;
  - `:131` `HINT_TYPED` `Enter confirm · Esc stop` → `FieldSubmit confirm`, `FieldCancel stop`. The stack is
    `CAPTURE` for `Typed`, `:530`.
  - `boxes.rs:86-87`, `:94-95`, `:108-109` (`Enter breaks the line` → `FieldSubmit`, C6). `comma-separated` and
    `blank clears` are notes.
  - Hierarchy's own `CHANGED_ELSEWHERE` (`:101-102`) → `changed_elsewhere(keys.label(HIERARCHY_EDITOR,
    FieldSubmit))`, with its local const deleted. Its own `DELETED_ELSEWHERE` (`:105`) names no key. Boxes'
    `:410`, `:423` → `changed_elsewhere(..)` (`on_stale` has `keys`).
- **`Ctx::pressed`**:
  - Hierarchy `on_editor_key` `:1029`, `on_deleting_key` `:913`, `on_picker_key` `:1059`;
  - `PathPicker::on_key` (`:149-177`, chord `:173`) gains `pressed: KeyChord`, passed from `on_picker_key`;
  - Boxes `on_editor_key` `:592`, `on_executor_key` `:608`.
- **MOD-85**: the picker cursor row → the band; the path header → `title`; the editor labels → the form marker.
- **MOD-83**:
  - `UNAVAILABLE` "hierarchy needs Postgres" (`:59`, MOD-83's example) → "… · Settings › Connection stores a
    DSN";
  - `:981` `{rows} rows across {tables} tables` → `plural`;
  - `boxes.rs:996` `{os_family}` → `label::os_family`;
  - `boxes unavailable` (`boxes.rs:55`) gains a next step.
- **Re-accepts**: `hierarchy__*` (9, `picker` included), `box_settings__*` (9), plus two 80-column snapshots.

### 5.6 L-sec: `settings/{secrets,connection,qdrant}.rs`, `tests/{secrets_settings,connection}.rs`, **new `tests/qdrant_settings.rs`** (D-13), `secrets_settings__*`, `connection__*`, `qdrant_settings__*`
- **Self-drawn sites**:
  - Connection: `lines` (`connection.rs:594-615`), `pane` (`:618-666`, `DSN: ` accent `:625`), `notice_line`
    `:673`, `render` `:936-965` → one `kit::form` with the editing row inline (§2.2), a confirm question as the
    row's error line, and the notice as `kit::text::notice`.
  - Qdrant: `render` `:502-~600` (labels accent `:563`, `:587`) → the same shape.
  - Secrets: `lines` `:1010`, `pane` `:1079`, `render` `:1548-1577`, `push_row` `:1581`, `form` `:1606-1636`
    (all labels accent `:1625`) → `kit::form`.
- **Hints**:
  - `connection.rs:69-70`;
  - `qdrant.rs:43`, `:45-46`;
  - `secrets.rs:161`, `:165-166`, `:172-173`.
  - Prose (C5):
    - `connection.rs:98` `NO_DSN_YET` "type one and press Enter" → `fn no_dsn_yet(submit: Option<&str>)`;
    - `:105` `REPLACES`;
    - `:126` `UNREADABLE_GUIDE`;
    - `qdrant.rs:19`, `:21`, `:27` likewise;
    - `qdrant.rs:590` "Enter API key, or leave blank if none" → "type the API key, or leave it blank if there is
      none" (a verb, C5).
  - CAS `secrets.rs:1323` in `on_scope_stale` (`:1307`, no `&Keys`) → threaded from `on_reply`.
- **`Ctx::pressed`**:
  - Connection `on_editor_key` `:468`, `on_confirm_key` `:519`;
  - Qdrant `on_editor_key` `:246`, `on_confirm_key` `:254`, `on_browse_key` `:284`;
  - Secrets `on_form_key` `:734`, `on_confirm_key` `:862`.
- **MOD-85**: every form label accent → the form marker.
- **MOD-83**:
  - `connection.rs:1006` `{n} migrations pending` → `plural`;
  - the unavailable texts (`connection.rs:50`, `secrets.rs:73`, `qdrant.rs:304`) gain next steps.
  - `secrets.rs:71` `NO_WORKSPACE` already names one.
- **New Qdrant test** (`tests/qdrant_settings.rs`, `#![cfg(feature = "testkit")]`, the `tests/connection.rs`
  section-half pattern):
  - a `SectionBench` over `QdrantSection::new()`, fed the section's own reply built as a literal, so no keyring is
    reached (`connection.rs:24-29`'s rule);
  - snapshots `qdrant_settings__{stored,empty,unreadable,editor,confirm}` and one 80-column `Harness` frame;
  - key/hint assertions for the converted specs.
- **Re-accepts**: `secrets_settings__*` (7), `connection__*` (5), plus new snapshots.
- **Coupling**: `tests/keys.rs:129` asserts `!frame.contains("Remove the DSN from the keyring?")`, unaffected.

### 5.7 L-prompt: `settings/{prompt,queue}.rs`, `tests/{prompt_settings,queue_settings}.rs`, `prompt_settings__*`, `queue_settings__*`
- **Self-drawn sites**:
  - Prompt: `lines` (`prompt.rs:635-669`, `theme.selected` rows) → `kit::form` read-only rows with headings;
    `pane` (`:676-730`, editor label accent `:720`) → the editing row inline plus its `help` (the spec doc, the
    range, the clamp and notes); `render` `:924-960` (`message` `:944`);
  - Queue: `lines` `:495-526`, `pane` `:529-542` (`:536`), `render` `:697-740` (`message` `:722`) → `kit::form`
    (§2.2 mapping).
- **Hints**: `prompt.rs:80-81`, `queue.rs:55-56` → `FieldSubmit`/`FieldCancel` (`CAPTURE`). CAS `prompt.rs:629`
  (`on_stale` `:606`) and `queue.rs:489` (`:470`) have no `&Keys`; it is threaded in.
- **`Ctx::pressed`**: `prompt.rs:515`, `queue.rs:397` (both `FieldOutcome::Pass → modal_rest`).
- **MOD-83 (the M3 owner of these examples)**:
  - `value_line` (`prompt.rs:1091-1103`) `({source})` → `label::setting_source`, so `(app_setting_default)`
    becomes `(default)` and `(app_setting)` becomes `(app setting)`. The switch rows' `BudgetSource::Project.as_str()`
    (`:986`) and `COMPILED_DEFAULT` go the same way.
  - `fraction_text` (`:1110-1112`) `0.1 (1000 bp)` → `10% (0.1)`, with the `bp` unit column shown as `fraction`
    and the help's range written in percent. D12's reason for the bp (refusals are phrased in bp) moves into the
    help line: `range 0–50% · typed as a fraction`.
  - `queue.rs` `(default)` and `inherit (n)` read as words already and are kept.
  - The unavailable texts gain next steps.
- **MOD-81**: the value rows wrap under their column (the form's `ReadOnly`) instead of being cut at the right edge
  at 80 columns. That is the plan's acceptance 6 bullet (but see D-9).
- **Re-accepts**: `prompt_settings__*` (6), `queue_settings__*` (5), plus two 80-column snapshots.
- **Coupling**: `tests/prompt_settings.rs:906` strip assert (T3). `tests/queue_settings.rs:21` imports
  `settings::queue::{NOTHING_SET, QueueSection, UNKNOWN_COST}` (kept public).

### 5.8 Owned by group B (not M3), for the record
- `3.000` and `hit(s)` (`ui/overlay/concepts_search.rs:273`);
- `template(s)` (`backlog/detail/prompt.rs`);
- the `0.1/0` step ids (Runs);
- `qdrant: query: connection refused` (concepts search);
- MOD-81's Backlog Prompt pane clip (D-9);
- `applied {n} migration(s)` (`app/update.rs:480`, the shell's status flash);
- the `?` box and no-tab body (`app/state.rs:1559-1565`, `:1687-1690`).

## 6. Hidden coupling checked (file sets re-run on this design)
- **Shared consts and helpers in `settings/mod.rs`**:
  - Lanes only *call* `is_error`, `answers`, `asked`, `some_text`, `yes_or_no`, `changed_elsewhere`, `message`,
    `CHANGED_ELSEWHERE_CLOSED` and `DELETED_ELSEWHERE`. No lane edits `settings/mod.rs`.
  - A need for a new shared helper is a serial kit/T3 fix between waves (plan risk row 4).
- **No section imports another section** (searched: no `settings::<section>::` or `super::<section>::` in `src/`).
  `tests/personas.rs:13` and `tests/queue_settings.rs:21` import their own section only.
- **`testkit.rs` needs no change in any lane**:
  - `SectionBench::ctx` (`:765-775`) has no `pressed`, so `ctx.pressed(&key)` falls back to the key's own chord;
  - `Origin::Tab(SettingsTab::ID)` gives `ctx.accent()` the Settings accent;
  - `render_section` (`:837-851`) draws section content only;
  - `Harness::size` (`:564`) serves the 80-column frames.
  - So no deviation moves testkit into T0.
- **Non-lane test files that read group-A frames**: they are added to their lane (D-7) or confirmed unaffected:
  - added: `tests/reveal.rs` (L-req), `tests/auth.rs` and `tests/install.rs` (L-agents);
  - confirmed unaffected: `tests/keys.rs:129`, `:564` (the `?` box), `tests/integration.rs:63` (the header),
    `tests/waiting.rs` (Runs), `tests/concepts_search.rs`/`shell.rs` (tab ids only).
- **`keys/catalogue.rs`**: no lane needs a new act. Every converted hint uses an existing `[field]` row (C6), and
  the `?` box and `docs/htui-keys.md` stay unchanged.
- **Snapshots**: no `.snap` is shared between lanes. The single cross-file one (`backlog__runs_pending_permission`)
  is L-chat's by D5. Settings' 19 frame snapshots are T3's first, then each section's lane's.
- **Kit**: `ui/kit/*` is T0/T1/T2's only. `chat/permission.rs` uses `kit::chips::key_chip` (exists).

## 7. Commit groups (the order an implementer commits; tests first in each group)
| Task | Files | Commit groups |
|---|---|---|
| T0 | `ui/tabs/registry.rs`, `app/state.rs`, `ui/kit/{mod,text,chips,modal,form,filter,label}.rs`, `settings/mod.rs`, the §1.5 importers | (a) seam tests (§1.4) + `Tab::draws_chrome`, `Ctx::{pane_focus, accent}`, `paint_panes(.., chrome)`, adapter guard; (b) `kit::text` (`wrapped` + its moved tests, `notice`/`Tone`), `kit::chips::key_line`, `kit::modal::modal_rest`, helpers out of `settings/mod.rs`, every importer; (c) stubs `form`/`filter`/`label` + `kit/mod.rs`. Gate: build, `--all-features` tests, `cargo insta test --all-features --check` **unchanged** |
| T1 | `ui/kit/{pane,form,filter}.rs` | (a) chip-strip tests, then `chip_strip`/`sub_tab_rects(.., active)`/`inner`/`scrollbar_span`; (b) `kit::form` tests, then code; (c) `kit::filter` tests, then code |
| T2 | `ui/kit/label.rs` (`status.rs` untouched unless `Mark` reuse needs it) | (a) label tables + tests; (b) `plural` + tests |
| T3 | `settings/mod.rs`, the 19 frame snapshots | (a) §4.2 tests, then the frame, `draws_chrome`, `scroll`, `changed_elsewhere` (const → `pub`), doc fixes; (b) snapshots, one commit per test file (`settings`+`probe`, `box_settings`, `hierarchy`, `kinds`, `personas`, `prompt_settings`) after its review |
| L-req | §5.1 | (a) tests (accents, filter band, labels, pressed, hints under a rebound field key); (b) panes + tree on `kit::list` + filter band; (c) forms on `kit::form`; (d) labels, plural, empty states; (e) `tests/reveal.rs` `titled`; (f) snapshots + 80-col |
| L-chat | §5.2 | (a) tests; (b) pane + title + scrollbar span + composer region; (c) permission strip chips (+ the Runs snapshot); (d) labels, plural, accents; (e) snapshots + 80-col |
| L-agents | §5.3 | (0) the D7 split, alone, existing tests green; (a) tests; (b) table on `kit::list` + min width; (c) forms + paste on `kit::form`, `form_navigation(chord)`; (d) hints/prose/CAS through `Keys`; (e) labels, plural, empty states; (f) `tests/{settings,auth,install}.rs` offsets; (g) snapshots + 80-col |
| L-kinds | §5.4 | (a) tests; (b) kinds tree/editor; (c) personas; (d) copy; (e) snapshots + 2×80-col |
| L-hier | §5.5 | (a) tests; (b) hierarchy; (c) path picker on `Modal` + `kit::list`; (d) boxes; (e) copy; (f) snapshots + 2×80-col |
| L-sec | §5.6 | (a) the new `tests/qdrant_settings.rs` snapshots of today's Qdrant section (pinned before the port, so its own port shows as a diff); (b) tests; (c) connection; (d) qdrant; (e) secrets; (f) snapshots + 80-col |
| L-prompt | §5.7 | (a) tests; (b) prompt; (c) queue; (d) copy; (e) snapshots + 2×80-col |
| T4 | `tests/kit_chrome.rs` (+ optional `tests/token_scan/mod.rs` shared with `tests/no_hardcoded_keys.rs`), `settings/mod.rs` (delete `CHANGED_ELSEWHERE`, `message` if unused), `docs/decisions/mod/mod-115-decisions.md`, `.claude/prds/mod-115-tui-shell.prd.md`, `HANDOFF.md` | (a) guard + self-tests; (b) dead transitional items; (c) records |
Every lane commit names the lane in its subject (`… (MOD-115 M3 L-req)`) and stages by path.

## 8. Build sequence
1. **T0** (serial, primary): seam, homes, stubs. Snapshots unchanged.
2. **T1, then T2** (serial, primary).
3. **T3** (serial, primary): the Settings frame and its 19 snapshots.
4. **Wave A** worktrees from T3's head: L-req, L-chat, L-agents, L-kinds. Each merges `--no-ff` into
   `hr/MOD-115` one at a time, with the full gate on the merged tree.
5. **Wave B** from the merged wave A: L-hier, L-sec, L-prompt.
6. **T4** (serial): the guard, deletions, the full Validation block, records.

## 9. Task 4: guard `tests/kit_chrome.rs` (mirrors `tests/no_hardcoded_keys.rs`)
- **Mechanism**: `proc_macro2` tokenising (dev-dep present).
  - The test-item skipping (`#[cfg(test)]`, `#[test]`, `#[tokio::test]`, an inner `#![cfg(test)]` skips the
    file) is the same as `no_hardcoded_keys.rs:56-129`.
  - Either copy it, or extract `walk`/`skip_item`/`sources`/`relative` into `tests/token_scan/mod.rs` and
    `mod token_scan;` from both files. Extraction is preferred, so the two guards cannot drift; T4 is serial, so
    editing `no_hardcoded_keys.rs` is safe.
- **Needles**: the same joint `::` check as `:98-108`, generalised. A violation is `Ident(Block)` or
  `Ident(Borders)` followed by a joint `::`, reported as `src/<path>:<line>`. A `use ratatui::widgets::{Block}`
  is no hit, and an unused import is clippy's job.
- **Scope**: every `.rs` under `src/ui/` and `src/app/`.
```rust
/// Group A (MOD-115 M3): no frame of its own anywhere in these.
const GROUP_A: &[&str] = &["ui/tabs/settings/", "ui/tabs/requirements/", "ui/tabs/chat/", "ui/path_picker.rs"];
/// The kit draws the frames.
const KIT: &str = "ui/kit/";
/// Still framing themselves; M4 ports each and deletes its entry.
const GROUP_B: &[(&str, &str)] = &[
    ("app/state.rs", "the `?` box and the no-tab body"),
    ("ui/editor_pane.rs", "the in-pane editor"), ("ui/markdown.rs", "markdown blocks"),
    ("ui/overlay/abort_confirm.rs", "overlay"), ("ui/overlay/concepts_search.rs", "overlay"),
    ("ui/overlay/migration_prompt.rs", "overlay"), ("ui/overlay/queue.rs", "overlay"),
    ("ui/overlay/workspace_switcher.rs", "overlay"),
    ("ui/tabs/backlog/detail/compose.rs", "Backlog"), ("ui/tabs/backlog/detail/mod.rs", "Backlog"),
    ("ui/tabs/backlog/detail/runs/execution_graph.rs", "Backlog"), ("ui/tabs/backlog/divergence.rs", "Backlog"),
    ("ui/tabs/backlog/filter.rs", "Backlog"), ("ui/tabs/backlog/item_form.rs", "Backlog"),
    ("ui/tabs/backlog/list.rs", "Backlog"),
    ("ui/tabs/skills/agent_help.rs", "Skills"), ("ui/tabs/skills/attach.rs", "Skills"),
    ("ui/tabs/skills/library.rs", "Skills"), ("ui/tabs/skills/templates.rs", "Skills"),
];
```
The list is the current non-test `Block::`/`Borders::` sites outside group A and the kit, from a scan of
`src/ui` and `src/app`.
- **Tests**:
  - `group_a_draws_only_through_the_kit`: any hit under `GROUP_A` fails with its `path:line`.
  - `nothing_else_frames_itself_unless_m4_lists_it`: a hit outside `KIT ∪ GROUP_A` must be in `GROUP_B`.
  - `every_m4_entry_still_frames_itself`: a stale entry fails, as `every_allow_list_entry_is_still_needed`
    (`no_hardcoded_keys.rs:191`) does. This is how M4 shrinks the list.
  - `no_group_a_file_is_listed_for_m4`.
  - Self-tests on inline sources: a `Block::` in a `cfg(test)` mod is skipped, one in a comment or string is
    ignored, a real one is reported with its line.
- **Run**: `cargo test -p htui --test kit_chrome`. It reads sources only, so it is not testkit-gated, like
  `no_hardcoded_keys.rs`.

## PLAN DEVIATIONS
- **D-1 (T3 "changes every Settings snapshot").** T3 changes only the 19 snapshots drawn through `Harness`, which
  carry the `┌ Settings ─` frame (§4.3).
  - The other 49 `settings__`/`probe__`/`box_settings__`/`connection__`/`hierarchy__`/`kinds__`/`personas__`/
    `prompt_settings__`/`queue_settings__`/`secrets_settings__` snapshots are `SectionBench::render_section`
    renders (`testkit.rs:837-851`) with no frame.
  - Those 49 move only in their lane.
- **D-2 (D1 "skips `paint_panes`").** `paint_panes` also draws the activity placeholder, a shell pane
  (`app/state.rs:706-725`). For a ported tab it keeps the placeholder and its focus restyle and skips only the
  list/detail restyle.
  - With the adapter gone, a focused placeholder would show no hints. So the shell paints the tab's `key_hint` on
    it (C2).
- **D-3 (D3 enum list).** Group A also shows several enums the plan does not list, and T2 labels them:
  - `Gate`, `Isolation` and `CommandQueue` (Kinds, `kinds.rs:2017-2021`). The plan files `gate` under
    "Backlog-only, M4".
  - `OsFamily` (Boxes, `boxes.rs:996`).
  - `ProbeStatus` (Agents' `on this box`, `agents.rs:1335-1342`).
  - `RequirementState`, `CitationKind` and `Resolution` (the Requirements detail).
  - `BudgetSource` (the Prompt section).
  - Conversely, `RunStatus`/`StepStatus` have no group-A site (Runs is group B). T2 labels them anyway for M4, and
    no lane uses them.
- **D-4 (T2 test "no label equals its snake_case value").** That rule fails by design for single-word values that
  are already human (`must`, `later`, `active`, `done`, `addresses`, `always`, `copy`). The tests instead are:
  - no label contains `_`;
  - every multi-word value is relabelled;
  - a per-enum table pins each label.
- **D-5 (T3 "converts `CHANGED_ELSEWHERE`").** Its users are 7 lane files:
  - `agents.rs:1917`, `boxes.rs:410`, `:423`, `kinds.rs:1181`, `:1206`, `personas.rs:1117`, `prompt.rs:629`,
    `queue.rs:489`, `secrets.rs:1323`;
  - and four of those `on_stale`-style fns have no `&Keys`.

  So:
  - T3 adds `changed_elsewhere(retry: Option<&str>)` and makes the const `pub` for the transition (C4: a
    `pub(crate)` const would trip `-D warnings` on the merged tree when the last lane drops it);
  - each lane converts its sites and threads `keys`;
  - T4 deletes the const.
  - Hierarchy's own copy (`hierarchy.rs:101-102`) and the `CHANGED_ON_BOTH_SIDES` pair (`agents.rs:202-203`,
    `personas.rs:164-165`) are lane-owned too.
- **D-6 (D4 notices and `message()`).** T3 converts only its own empty case. `message` (7 section users) stays
  `pub` until each lane moves to `kit::empty`/`kit::text::notice`, and T4 deletes it.
- **D-7 (lane file sets).** Two non-lane test files read group-A frames and join lanes (neither has snapshots):
  - `tests/reveal.rs` → L-req. `titled()` at `:67-69` reads `┌ KEY ` off the Requirements detail title (`:166`,
    `:195`, `:218`), and `:189` pins the tree title.
  - `tests/auth.rs` and `tests/install.rs` → L-agents. Both parse the Agents table by fixed column offsets and
    assert probe words and hints (`auth.rs:515`, `:533`, `install.rs:408-430`, `:467`).
- **D-8 (lane validation).** The plan validates a lane on "the lane's test files". Group-A frames are also read by
  other test files (D-7, `tests/kinds.rs:865`, `tests/prompt_settings.rs:906`, `tests/keys.rs`). So each lane runs
  the whole `cargo test -p htui --all-features -- --test-threads=1 --no-fail-fast`, and keeps per-file
  `cargo insta test --test <file>` for accepting snapshots only.
- **D-9 (acceptance 6, "the Prompt section wraps").** MOD-81's "the Prompt pane clips its right edge"
  (`HANDOFF.md:604`) is Backlog › Prompt (`backlog/detail/prompt.rs`, group B, M4).
  - L-prompt still wraps Settings › Prompt's value rows, since they clip at 80 columns.
  - MOD-81's own item stays open for M4.
- **D-10 (D3/D4 chip overflow vs. the title).** The `Pane` keeps its M1 precedence: the title is clipped before
  any chip is dropped (`kit/pane.rs:5-7`, `:465`).
  - When Settings' ten chips overflow (at 80 columns), the pane's title, the section's name, is not drawn, and the
    active chip carries the name.
  - `sub_tab_rects` gains `active` (it has no caller outside the kit).
- **D-11 (scrollbars in a single Settings pane).** A section's scroll is the section's.
  - T3 adds `SettingsSection::scroll() -> Option<(usize, usize)>`, defaulted to `None`, and draws the section into
    `Pane::inner` before the frame (C3).
  - T1 adds `Pane::inner` and `Pane::scrollbar_span`. Chat's bar spans only its transcript rows.
- **D-12 (where the "kit notice line" lives).** `kit::text::{notice, Tone}` is created in T0 with `wrapped`.
  T1's and T3's sets do not include `text.rs`.
- **D-13 (Qdrant test name).** The new test is `tests/qdrant_settings.rs`, so its snapshots are
  `qdrant_settings__*`, matching `prompt_settings`/`queue_settings`/`secrets_settings`. The plan wrote
  `qdrant__*`. `tests/qdrant_worker.rs` exists and is not touched.
- **D-14 (Requirements' hint row trims).** The M1 adapter ran Requirements' hints across both panes, joining the
  seam (`app/state.rs:1778-1793`). `Pane::hints` keeps them inside the focused pane.
  - With the tree focused at 100 columns (45% = a 45-wide pane, 41 cells of hint), the row reads `j/k move ·
    Enter fold · / filter · a area…`, and the write keys drop.
  - The `?` box and the about panel still list them.
  - This follows from findings §3.2 ("trimmed to what fits"). **Needs a maintainer call**: accept, or order the
    write keys first.
- **D-15 (C5, MD-A's reach).** Prose that names a key is converted too: `connection.rs:98`, `:105`, `:126`;
  `qdrant.rs:19`, `:21`, `:27`; `personas.rs:138`; the CAS sentences. `qdrant.rs:590` ("Enter API key") is a
  verb, so it is reworded rather than keyed.
- **D-16 (Chat's tool-call status).** `CallStatus` is a chat-local enum (`chat/transcript.rs:22`). Its glyph and
  word are built in `transcript.rs` (L-chat) with `kit::label::Mark`, because the kit never imports a tab.
- **D-17 (D5 accents).** `Composer::render` (`chat/composer.rs:114`) gains an `accent: Style` parameter so the
  prompt `> ` can take the tab accent (the one focus use left in Chat). Its only caller is `chat/mod.rs:895`.
- **D-18 (T2 probe marks vs. the `on this box` column; recorded by T2's repair).** Two §3/§5.3 claims did not hold:
  - `✗ probe failed` is 14 cells, one past the 13-cell `on this box` column (`agents.rs:2076-2077`). T2 shows
    `✗ failed` (the shared failed mark, and the word the cell shows today); the column already names the probe.
    L-agents' snapshot review expects `✗ failed`.
  - §5.3's donor `on this box` 13 → 12 would cut the maintainer-approved `⚠ needs login` (13 cells) to
    `⚠ needs logi`. `on_box_cell` can also append `MANUAL_SUFFIX` (`agents.rs:1352`), so the cell may already run
    past 13 today. L-agents keeps the column at 13 at least and takes the name column's extra cell from another
    column; shortening the word needs a new maintainer call. T2's test
    `a_probe_verdict_says_what_to_do_and_fits_the_agents_column` pins every probe mark at 13 cells or fewer.

## Maintainer calls
1. **D-14**: Requirements' hint row loses its write keys with the tree focused at 100 columns. Accept the trim
   (findings §3.2), or have L-req order `a area · n new · e amend · W withdraw` first.
2. **Copy that changes what a user reads, decided here but flagged** (each lane implements it; say if any should
   stay):
   - L-prompt: `0.1 (1000 bp)` → `10% (0.1)`, with the bp range moved into the help line;
   - L-agents: `unauthenticated` → `⚠ needs login`;
   - L-agents: below 98 cells the Agents table drops `models`, then `billing`, then `transport` to keep the name
     column at 10 or more.

### Maintainer answers (2026-10-21)
1. **D-14: write keys first.** L-req orders the tree pane's hint `a area · n new · e amend · W withdraw · / filter …`,
   so the keys that change something survive a cut.
2. **All three copy changes go ahead**: Prompt `10% (0.1)` with the bp range in the help line; Agents
   `⚠ needs login`; the Agents table drops `models`, then `billing`, then `transport` below 98 cells.
