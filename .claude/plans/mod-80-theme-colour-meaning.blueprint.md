# Blueprint: MOD-80 theme and colour meaning (T1-T6)

**Plan**: `.claude/plans/mod-80-theme-colour-meaning.plan.md` (confirmed 2026-10-04: D1-D6 as written, plus the four
maintainer calls: bold+underline for the active tab, the waiting count in `warning`, `dim = Indexed(244)`,
`running = Blue`, `added = Green`). Produced by `code-architect` against `hr/MOD-80` @ `10e4c480`. Line numbers are
at that commit. Where this file differs from the plan, it says so and gives the reason (B-1..B-7, collected under
**Deviations** at the end).

All paths are under `crates/htui/src/` unless they start with `crates/`. Tasks run **serial, one lane, one commit
per task**. Every task commit leaves `cargo test -p htui --lib` green (B-7 explains the order that makes this hold).

---

## T1: `Theme` roles, palette, `select`, monochrome (D1-D4) — `ui/theme.rs`, `lib.rs`

### Struct (field order = doc order; `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` unchanged)

```rust
pub struct Theme {
    /// Ordinary text.
    pub base: Style,
    /// Secondary text: hints, empty states, the status line, separators. `Indexed(244)`, a grey
    /// at least 3.5:1 on light and dark profiles alike (MOD-80 D3).
    pub dim: Style,
    /// Block titles, group headers and the top bar's workspace: the terminal's own foreground,
    /// bold, so it reads on any profile (D3).
    pub title: Style,
    /// Focus: the row an overlay or form is on, a focused form label, the composer prompt, the
    /// Agents table highlight. Nothing else is cyan (D1).
    pub accent: Style,
    /// The selected row of a list. It replaces every cell's colours: a multi-span row goes
    /// through [`Theme::select`], so no span's own colour shows through (D2).
    pub selected: Style,
    /// Failure text (`StoreReply::Failed`, a rejected gate, a failed status).
    pub error: Style,
    /// Item and requirement keys, and the Graph sub-tab's node labels (D1).
    pub key: Style,
    /// Something in progress: an `in_progress` item, a running run or step or graph node (D1).
    pub running: Style,
    /// Something that needs a person: `awaiting_approval`, a degraded store, the waiting
    /// count (D1, D6).
    pub warning: Style,
    /// A diff's `+` lines (D1).
    pub added: Style,
    /// The active entry of a tab strip: main tabs, detail sub-tabs, Settings sections. Bold and
    /// underlined, so it shows without colour (D5).
    pub active_tab: Style,
    /// The one-cell text cursor in `TextField` / `TextArea`, and the flow view's cursor node
    /// border (D1).
    pub cursor: Style,
}
```

### `Default` (B-7: `selected` flips in T2, not here)

| field | T1 value | after T2 |
|---|---|---|
| `base` | `Style::new()` | same |
| `dim` | `Style::new().fg(Color::Indexed(244))` | same |
| `title` | `Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD)` | same |
| `accent` | `Style::new().fg(Color::Cyan)` | same |
| `selected` | `Style::new().add_modifier(Modifier::REVERSED)` (unchanged in T1) | `Style::new().fg(Color::Black).bg(Color::Cyan)` |
| `error` | `Style::new().fg(Color::Red)` | same |
| `key` | `Style::new().add_modifier(Modifier::BOLD)` (`base` + BOLD; `base` is empty) | same |
| `running` | `Style::new().fg(Color::Blue)` | same |
| `warning` | `Style::new().fg(Color::Yellow)` | same |
| `added` | `Style::new().fg(Color::Green)` | same |
| `active_tab` | `Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD \| Modifier::UNDERLINED)` | same |
| `cursor` | `Style::new().add_modifier(Modifier::REVERSED)` | same |

Write `key` as `base.add_modifier(BOLD)` and `active_tab` as `accent.add_modifier(BOLD | UNDERLINED)` with
local `let base = Style::new(); let accent = …;` in `default()`, so the "base plus" / "accent plus" relations are
visible in the code.

### `impl Theme`

```rust
/// MOD-80 D4: no colour, only modifiers, for `NO_COLOR` terminals (crossterm 0.29 drops colour
/// codes there and keeps attributes). `selected`/`cursor` reverse, `accent`/`key`/`title` are bold,
/// `dim` is DIM, `active_tab` bold+underlined; `error`, `warning`, `running`, `added` are plain,
/// because the status word or diff gutter already says what they mean.
#[must_use]
pub fn monochrome() -> Self
```
Values: `base`, `error`, `running`, `warning`, `added` = `Style::new()`; `dim` = `add_modifier(DIM)`; `title`,
`accent`, `key` = `add_modifier(BOLD)`; `active_tab` = `add_modifier(BOLD | UNDERLINED)`; `selected`, `cursor` =
`add_modifier(REVERSED)`.

```rust
/// crossterm's `NO_COLOR` rule (`style/types/colored.rs` `ansi_color_disabled`): set and non-empty
/// means [`Theme::monochrome`], anything else [`Theme::default`]. Takes the value, not the
/// environment, so it is testable.
#[must_use]
pub fn from_no_color(value: Option<&OsStr>) -> Self   // match value { Some(v) if !v.is_empty() => monochrome, _ => default }
```
`use std::ffi::OsStr;` at the top (the workspace warns on `unused_qualifications`).

```rust
/// D2: `line` with [`Self::selected`] patched over the line **and over every span**, so a span's
/// own `fg` cannot show through (a `Line::style` alone leaves a cyan key cyan-on-cyan).
/// Modifiers a span had (a key's BOLD) survive.
#[must_use]
pub fn select<'a>(&self, line: Line<'a>) -> Line<'a>
```
Generic over `'a`: both multi-span call sites hand it a `Line<'static>` (`backlog/list.rs::row` builds from owned
`Vec<Span<'static>>`; `requirements/tree.rs::pad` from `cells::clip_spans`), and the Documents cells (T2) also pass
`Line<'static>`. Body shape (ratatui-core 0.1.2 has `Line::patch_style` / `Span::patch_style`, both
`fn(self, impl Into<Style>) -> Self`):
```rust
let mut line = line.patch_style(self.selected);
for span in &mut line.spans { span.style = span.style.patch(self.selected); }
line
```
`use ratatui::text::Line;` added to the imports.

`status_style`:
```rust
Status::Open | Status::Queued => self.base,
Status::InProgress => self.running,
Status::AwaitingApproval => self.warning,
Status::Blocked | Status::Failed => self.error,
Status::Done | Status::Closed => self.dim,
```

### Wiring (B-1: not `app/state.rs`)
- `lib.rs:176-177` (`run`): after `app.top_bar.store = label;` add
  `app.theme = ui::Theme::from_no_color(std::env::var_os("NO_COLOR").as_deref());` with a one-line comment
  `// MOD-80 D4: a NO_COLOR terminal gets the modifier-only theme.`
- `app/state.rs:238` keeps `theme: Theme::default()`; the `theme` field doc (state.rs:151 `/// The palette.`) becomes
  `/// The palette: \`Theme::default()\` until \`run\` applies \`NO_COLOR\` (MOD-80 D4).`

### Existing tests that change in T1 (all from `dim` leaving `DarkGray`)
`ui/tabs/backlog/detail/runs/execution_graph.rs`:
- `the_winner_carries_a_check_and_the_loser_is_dim` (1223, 1224): `Color::DarkGray` →
  `Theme::default().dim.fg.expect("dim has a colour")` (bind it once as `let dim = …;`).
- `a_node_draws_its_chips_dim_on_the_third_line` (1676): same.

### New tests first (`ui/theme.rs`, new `#[cfg(test)] mod tests`, `use super::*; use ratatui::text::Span;`)
1. `the_default_palette_gives_each_role_its_colour`: `dim.fg == Some(Indexed(244))`; `title == fg(Reset)+BOLD`;
   `running.fg == Blue`, `warning.fg == Yellow`, `added.fg == Green`, `accent.fg == Cyan`; `key.fg == None` and
   `key.add_modifier` contains BOLD; `active_tab.fg == accent.fg` and contains `BOLD | UNDERLINED`;
   `cursor.add_modifier` contains REVERSED. (T2 adds the `selected` line.)
2. `in_progress_is_running_and_awaiting_is_a_warning`: every `Status` variant → its role, all ten arms.
3. `select_paints_the_selection_over_every_span`: theme =
   `Theme { selected: Style::new().fg(Color::Black).bg(Color::Cyan), ..Theme::default() }` (the T2 value, inline so
   T1 does not depend on the flip); line = `[Span::styled("FEAT-1", key), Span::styled("x", fg(Cyan)),
   Span::styled("d", dim)]`; after `select`: every span `fg == Some(Black)`, `bg == Some(Cyan)`; the key span still
   has BOLD; `line.style.bg == Some(Cyan)`.
4. `select_under_monochrome_reverses_every_span`: same line built from `Theme::monochrome()` roles; every span
   contains REVERSED and has `fg == None`.
5. `no_color_set_and_non_empty_is_monochrome`: `from_no_color(None) == default()`;
   `from_no_color(Some(OsStr::new(""))) == default()`; `Some("1")` and `Some("0")` both `== monochrome()`.
6. `the_monochrome_theme_names_no_colour`: an array of all twelve roles of `monochrome()`; each has `fg == None`
   and `bg == None`; `selected` and `cursor` contain REVERSED; `active_tab` contains `BOLD | UNDERLINED`; `dim`
   contains DIM.

**Validate**: `cargo test -p htui --lib -- ui::theme ui::tabs::backlog::detail::runs::execution_graph`

---

## T2: selection and cursor (D2, D1 `cursor`)

### Call sites
| file:line | old | new |
|---|---|---|
| `ui/tabs/backlog/list.rs:234-238` (`row`) | `if selected { line.style(theme.selected) } else { line }` | `if selected { theme.select(line) } else { line }` |
| `ui/tabs/backlog/list.rs:224` doc | "so the selected style covers the whole line" | "so the selection ([`Theme::select`]) covers the whole line" |
| `ui/tabs/requirements/tree.rs:339-343` (`pad`) | `line.style(theme.selected)` | `theme.select(line)` |
| `ui/tabs/requirements/tree.rs:333` doc | "padded out to it so the selected style covers the whole row" | "… so the selection covers the whole row" |
| `ui/tabs/backlog/detail/documents.rs:327-339` (`render_table`) | four `Cell::from(Line::styled(..))`, then `row.style(selected)` on the cursor row | `let on_cursor = at == self.cursor; let cell = \|line: Line<'static>\| Cell::from(if on_cursor { ctx.theme.select(line) } else { line });` and each column `cell(Line::styled(..))`; keep `row.style(ctx.theme.selected)` on the cursor row (it paints the column gaps) |
| `ui/text_field.rs:338` | `if focused { theme.selected } else { theme.base }` | `if focused { theme.cursor } else { theme.base }` |
| `ui/text_field.rs:301` doc | "the cursor cell carries `theme.selected`" | "… `theme.cursor`" |
| `ui/text_area.rs:333` | `Span::styled(at, theme.selected)` | `Span::styled(at, theme.cursor)` |
| `ui/text_area.rs:258` doc | "the cursor cell is `theme.selected`" | "… `theme.cursor`" |
| `ui/tabs/backlog/detail/runs/execution_graph.rs:231` (`StepNode::border`) | `return self.theme.selected;` | `return self.theme.cursor;` |
| `execution_graph.rs:228` doc | "`selected` on the cursor" | "`cursor` on the cursor node" |
| `ui/theme.rs` `Default` | `selected: REVERSED` | `selected: Style::new().fg(Color::Black).bg(Color::Cyan)` — **last step of T2** (B-7) |

Single-style `theme.selected` users need no edit (each is `Line::styled(text, style)` or one `Span`, verified at
`settings/boxes.rs:906`, `settings/connection.rs:546`, `settings/prompt.rs:628`, `settings/qdrant.rs:463`,
`skills/attach.rs:728/1018/1097`, `skills/library.rs:1442`, `skills/templates.rs:940`). `attach.rs:1018` styles the
focused activation value only: it now reads black-on-cyan instead of reversed; acceptable.

### Existing tests that change
- `ui/text_area.rs:1260` `a_long_cursor_line_scrolls_to_show_the_cursor`: `assert_eq!(cursor_cell.style,
  theme.selected)` → `theme.cursor`. (The plan said these tests keep `REVERSED`; this one compares to the field,
  so it must follow the rename.)
- `ui/tabs/backlog/detail/documents.rs:1517-1533` `the_cursor_row_is_drawn_selected`: rewrite the assertion.
  `let selected = Theme::default().selected;` then for every `x in 0..43` at `y = 2`:
  `buffer[(x, 2)].fg == selected.fg.unwrap()` and `.bg == selected.bg.unwrap()`; at `y` in `[0, 1, 3]`,
  `buffer[(0, y)].bg != Color::Cyan`. Drop `use ratatui::style::Modifier;` (631, otherwise unused → clippy
  `-D warnings` fails); add `use ratatui::style::Color;` and `use crate::ui::Theme;`.
- Unchanged and still green because `cursor` stays `REVERSED`: `text_field.rs` 665/668/671/918/922,
  `text_area.rs` 1199/1205/1315/1329/1432/1563, `execution_graph.rs` 1184/1185 and 1435.

### New tests first
1. `ui/tabs/backlog/list.rs` `a_selected_item_row_is_one_even_block`: the `summary`/`ProjectRef` fixtures of
   `a_wide_key_and_title_keep_every_row_the_pane_width`, the first item `Status::InProgress`,
   `selected: Some(Selection::Item(items[0].id))`; `lines(&view, &theme, 50)`, rendered with
   `Paragraph::new(lines).render(Rect::new(0, 0, 50, 3), &mut buffer)` (`Buffer::empty`, `Widget as _`, the
   `text_field.rs::drawn` idiom). Row `y = 1`: every `x in 0..50` has `fg == Black`, `bg == Cyan`; the key cells
   `x in INDENT..INDENT + key_width` also contain BOLD (T3 gives the key BOLD; before T3 drop that sub-assert or
   add it in T3). Row `y = 2` (unselected) has `bg == Reset` at `x = 0`.
2. `ui/tabs/requirements/tree.rs` `a_selected_requirement_row_is_one_even_block`: `platform().await`,
   `TreeView { selected: Some(Row::Requirement(ids::REQ_ENT_1)), folded: &[], filter: "", .. }`, `lines(&view, 45,
   &theme)`, render to a 45-wide buffer; on the row whose text contains `R-ENT-1` every cell is Black on Cyan.
   Add `Row` to the `use super::{…}` list at tree.rs:348.
3. `documents.rs` rewritten test above.
4. `ui/theme.rs` test 1 gains `assert_eq!(theme.selected, Style::new().fg(Color::Black).bg(Color::Cyan))`.

**Validate**: `cargo test -p htui --lib -- ui::theme ui::tabs::backlog ui::tabs::requirements ui::text_field ui::text_area`
(B-5: several filters go after `--`.)

---

## T3: one meaning per colour (D1 `key`, `running`, `warning`, `added`)

### Call sites
| file:line | old | new |
|---|---|---|
| `ui/tabs/backlog/list.rs:206` | `Span::styled(cells::pad(&item.key, key_width), theme.accent)` | `…, theme.key)` |
| `ui/tabs/requirements/tree.rs:305` | `if withdrawn { theme.dim } else { theme.accent }` | `… else { theme.key }` |
| `tree.rs:295` doc | "the key accented" | "the key in `key`" |
| `ui/tabs/backlog/detail/body.rs:88` | `Span::styled(item.key.as_str(), ctx.theme.accent)` | `ctx.theme.key` |
| `ui/tabs/backlog/detail/graph.rs:548` | `(theme.accent, theme.status_style(..), theme.base)` | `(theme.key, …)`; the cursor glyph at 554 stays `accent` (focus) |
| `ui/tabs/backlog/detail/requirements.rs:355` | `(theme.accent, theme.base)` | `(theme.key, theme.base)` |
| `ui/tabs/requirements/detail.rs:121` (`coverage`) | item key `theme.accent` | `theme.key` |
| `ui/tabs/requirements/detail.rs:162` (`revision`) | `by {deciding_key}` in `theme.accent` | `theme.key` (B-4: it is a key) |
| `ui/tabs/backlog/detail/runs.rs:962` / `973` | `Style::new().fg(Color::Cyan)` | `theme.running` |
| `runs.rs:963` / `974` | `Style::new().fg(Color::Yellow)` | `theme.warning` |
| `runs.rs:68` | `use ratatui::style::{Color, Style};` | `use ratatui::style::Style;` (Color now unused outside tests) |
| `execution_graph.rs:235` | `StepStatus::Running \| StepStatus::AwaitingApproval => self.theme.accent` | `StepStatus::Running => self.theme.running, StepStatus::AwaitingApproval => self.theme.warning` |
| `ui/diff.rs:36` | `theme.accent` | `theme.added` |
| `ui/diff.rs:28` doc | "`+` accents, `-` errors" | "`+` in `added`, `-` in `error`" |

### Existing tests that change
- `ui/diff.rs:73-96` `an_added_line_is_accented` → rename `an_added_line_is_drawn_added`; line 85
  `assert_eq!(added.style, theme.accent)` → `theme.added`.
- `execution_graph.rs:1519-1534` `running_and_awaiting_steps_have_the_accent_border` → rename
  `running_and_awaiting_steps_have_their_status_border`; doc "take the accent border" → "take the `running` and
  `warning` borders"; 1532 → `Theme::default().running.fg.expect(..)` (Blue), 1533 → `warning.fg` (Yellow).
- `ui/tabs/requirements/tree.rs:661` (`withdrawn_rows_render_dim`): `span.style == theme.accent` →
  `theme.key`, message "an active key is bold".
- `ui/tabs/backlog/detail/requirements.rs:937` (`withdrawn_rows_render_dim`): `theme.accent` → `theme.key`,
  message "an active key is a key"; doc 907-908 "an active one's key is accented" → "an active one's key is in
  `key`".

### New tests first
1. `list.rs` `an_item_key_is_bold_and_in_progress_is_running` (buffer, same fixture, nothing selected): the key
   cell at `(INDENT, 1)` contains BOLD and `fg == Color::Reset` (not Cyan); the status cell at
   `(INDENT + key_width + GAP, 1)` has `fg == Color::Blue` (`theme.running.fg`). Add the BOLD sub-assert to T2's
   selected-row test here.
2. `runs.rs` `run_and_step_statuses_take_the_running_and_warning_roles` (pure, beside
   `the_figure_is_thousands_with_a_bang_when_trimmed`): `run_style(&t, RunStatus::Running) == t.running`,
   `(AwaitingApproval) == t.warning`, `step_style(&t, StepStatus::Running) == t.running`,
   `(AwaitingApproval) == t.warning`.
3. The renamed `diff.rs` and `execution_graph.rs` tests above are red until the swap.

**Validate**: `cargo test -p htui --lib`

---

## T4: active tabs without colour (D5)

### Call sites
| file:line | old | new |
|---|---|---|
| `ui/tabs/registry.rs:239` (`render_strip`) | `theme.accent` | `theme.active_tab` |
| `registry.rs:233` doc | "the active one accented" | "the active one in `active_tab` (bold, underlined)" |
| `ui/tabs/settings/mod.rs:406` (`render_strip`) | `theme.accent` | `theme.active_tab` |
| `settings/mod.rs:398` doc | "the active one accented" | same as above |
| `ui/tabs/backlog/detail/mod.rs:390` (`strip_line`) | `theme.accent` | `theme.active_tab` |
| `detail/mod.rs:382` doc | "The accent covers the title only" | "`active_tab` covers the title only" |

Style swap only. In the two padded strips (`" 1 Backlog "`, `" Agents "`) the underline runs under the padding
spaces; splitting the span would be a visual nicety beyond D5 and is not done here.

### New tests first (all under `Theme::monochrome()`, so only modifiers can mark the active entry)
1. `ui/tabs/registry.rs` new `#[cfg(test)] mod tests`: `active_main_tab_is_bold_and_underlined_without_colour`.
   `TabRegistry::new()`, `register(Box::new(BacklogTab::new()))`, `register(Box::new(SkillsTab::new()))`; draw
   `render_strip` into `TestBackend::new(40, 1)`; the cell under `1` (x = 1) contains `BOLD | UNDERLINED`; the cell
   under `2` (x = 1 + width of `" 1 Backlog "` = 12) contains neither. Cells have no `fg` other than `Reset`.
2. `ui/tabs/settings/mod.rs` tests: `active_section_is_bold_and_underlined_without_colour`.
   `SettingsTab::with_sections(vec![Box::new(ConnectionSection::new()), Box::new(QdrantSection::new())])`,
   `render_strip(frame, area, &tab.sections, &Theme::monochrome())` on a 40×1 backend; first title's first letter
   cell (x = 1) has both modifiers, the second title's (x = 1 + width of the first `" {title} "` + 1) neither.
3. `ui/tabs/backlog/mod.rs` beside `the_detail_strip_fits_the_detail_pane` (2183):
   `the_active_sub_tab_is_bold_and_underlined_without_colour`. `BacklogTab::new()`,
   `detail::strip_line(&tab.detail, &Theme::monochrome())`; the span whose content is the active title (the first
   registered) contains `BOLD | UNDERLINED`; every other non-blank span contains neither; separator spans have
   default style.

**Validate**: `cargo test -p htui --lib -- ui::tabs` (and no `.snap.new` anywhere: `find crates -name '*.snap.new'`)

---

## T5: degraded store label (D6) — `ui/top_bar.rs`

- Add `use ratatui::style::Style;`.
- New pure fn above `render`:
  ```rust
  /// MOD-80 D6: the store label in `warning` while the store is degraded — `connecting`, or
  /// `offline · <age>` (`Backend::label`) — and `base` for `online` and `memory`.
  fn store_style(label: &str, theme: &Theme) -> Style  // label == "connecting" || label.starts_with("offline") → warning, else base
  ```
- `render`, line 46: `Span::styled(state.store.clone(), theme.base)` → `…, store_style(&state.store, theme))`.
- Lines 34-39: comment → `// MOD-69 plan T3.3, MOD-80 D6: the waiting part in \`warning\` when a person is owed
  something; no plural rule, so "1 working · 1 waiting" reads as written.`; `theme.accent` → `theme.warning`.

### Existing test that changes
`the_waiting_count_is_accented_only_when_non_zero` (95-144) → rename `the_waiting_count_is_a_warning_only_when_non_zero`;
doc "is accented" → "is a warning"; 117/135/140 `theme.accent.fg` → `theme.warning.fg`; messages "no accent" →
"no warning", "working is never accented" → "working is never a warning".

### New tests first (mirror the existing `draw` / `line` helpers)
1. `store_style_warns_only_while_degraded` (pure): `"online"` and `"memory"` → `theme.base`; `"connecting"` and
   `"offline · 3m"` → `theme.warning`.
2. `a_degraded_store_label_is_drawn_as_a_warning` (buffer): state as in the waiting test with
   `store: "offline · 3m"`; `x` = char count of `"Platform · DESKTOP-HTUI · "`; `buf[(x, 0)].symbol() == "o"` and
   `buf[(x, 0)].fg == Color::Yellow` (= `theme.warning.fg.unwrap()`); repeat with `"connecting"`; with `"online"`
   assert `buf[(x, 0)].fg == Color::Reset`. Compare `cell.fg` (a `Color`), not `cell.style().fg` against
   `theme.base.fg`: `style().fg` is `Some(Reset)` while `base.fg` is `None`.

**Validate**: `cargo test -p htui --lib -- ui::top_bar`

---

## T6: docs

- `ui/theme.rs` module doc, replacing lines 1-4:
  ```
  //! The one place a colour is chosen.
  //!
  //! Views take the theme out of their [`Ctx`](crate::app::Ctx) instead of naming colours. Each role is one
  //! meaning (MOD-80 D1): `accent` is focus only, `key` keys, `running` work in progress, `warning` what needs
  //! a person, `added` a diff's `+`, `active_tab` the strip entry you are on, `cursor` the text cursor, and
  //! `selected` a selected row, which [`Theme::select`] paints over every span (D2).
  //!
  //! `NO_COLOR` set and non-empty selects [`Theme::monochrome`], the same roles drawn with modifiers alone
  //! (D4); `run` applies it once at startup.
  ```
- The decisions write-up (D1-D6 and B-1..B-7) is close-out work (lifecycle P2), not the implementer's.

---

## Sweep: what the plan's survey did not list

1. **`theme.accent` asserted for a key**: only `tree.rs:661` and `detail/requirements.rs:937` (T3). No other test
   asserts `accent` on a key. Other `accent` asserts: `chat/transcript.rs:898` (resume-failed note, not a key,
   unchanged), `top_bar.rs` (T5), `tests/settings.rs:114` `accented_lines` (Settings section cursor rows, still
   `accent`; it reads x = 0 of a drawn *section*, never the strip).
2. **`line.style` / `row.style` / `Row::style` with `selected`**: exactly the three the plan names
   (`list.rs:235`, `tree.rs:340`, `documents.rs:337`). `row_highlight_style` appears once
   (`settings/agents.rs:1853`, `accent`, stays). No `patch_style` anywhere today.
3. **`theme.title` in integration tests**: none. `crates/htui/tests/*.rs` read colour in five places only:
   `hierarchy.rs:1453`, `kinds.rs:843`, `personas.rs:126`, `prompt_settings.rs:766` (`error.fg`, unchanged) and
   `settings.rs:114` (`accent.fg`, unchanged). They run under `Harness`/`SectionBench`, whose themes are
   `Theme::default()`; B-1 keeps it so.
4. **Colour literals the plan missed in tests**: `execution_graph.rs` 1223, 1224, 1676 assert `Color::DarkGray`
   and break with D3 (B-2, handled in T1).
5. **`theme.selected` compared by value in a test**: `text_area.rs:1260` (B-3, T2).
6. **Imports that go unused** (clippy `-D warnings`): `documents.rs:631` `Modifier` (T2), `runs.rs:68` `Color`
   (T3).
7. **Cyan meanings the plan leaves in `accent`** (B-6; not changed in MOD-80):
   - "needs a person": `runs.rs:1097` (cancel requested), `1223` (`note_line`, a parked step's reason), `1280`
     (`asks:` permission summary), `1737`/`1759` (`WAITING_LINE`); `chat/transcript.rs:624` ("permission
     waiting on you"). These match D6's `warning` reasoning.
   - labels and headers: `chat/mod.rs:257` (replay header), `355`/`371` (chat lead, agent name),
     `chat/transcript.rs:532` (`you`), `652` (note), `settings/personas.rs:1152` (`path:`),
     `settings/connection.rs:570`, `settings/prompt.rs:703`, `settings/qdrant.rs:496/520` (editor labels),
     `requirements/forms.rs:404` (`[value]`), `requirements/mod.rs:980` (` /` filter prompt),
     `chat/permission.rs:37` (`[n]` answer digits).
   - state or data: `backlog/divergence.rs:576` (Theirs/Mine), `execution_graph.rs:373` (edge labels),
     `backlog/detail/documents.rs:328` (the document `kind` column; overridden on the cursor row by T2).
   - focus, correctly `accent`: `runs.rs:1181` and `1765` (step/run cursor), `graph.rs:554` (cursor glyph),
     overlay rows (`concepts_search.rs:425`, `waiting_list.rs:221`, `workspace_switcher.rs:135`,
     `path_picker.rs:309/360`), form labels and settings cursor rows (`agents.rs`, `hierarchy.rs`, `kinds.rs`,
     `personas.rs:1134/1361`, `forms.rs:570`), the composer (`composer.rs:114/116`).

---

## Gates (run by the implementer after T5 and again after T6)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                                   # featureless: catches code dead without testkit
cargo test -p htui --all-features -- --test-threads=1                     # integration tests need testkit; snapshots must not move
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod80-gate.log
grep -nE "SIGABRT|FAILED|panicked" /tmp/mod80-gate.log                    # htui-orch stack headroom; qdrant_live load timeouts: re-run serially
find crates -name '*.snap.new'                                            # must print nothing
```
Without `--features testkit` / `--all-features`, `crates/htui/tests/*.rs` run 0 tests and report ok.

---

## Deviations from the plan

- **B-1: `NO_COLOR` is read in `lib.rs::run`, not in `App::new` (`app/state.rs`).** `App::new` also builds the
  test shells (`testkit.rs:128` `Harness::over_backend`, `app/update.rs` tests). Reading the environment there
  would make every harness test depend on the developer's shell: with `NO_COLOR=1` exported,
  `tests/settings.rs::accented_lines` and the four `error_text` helpers would find no coloured cells and fail.
  `run` is the binary's only `App::new` (`lib.rs:176`), and `App.theme` is already `pub`.
- **B-2: three `Color::DarkGray` asserts in `execution_graph.rs` (1223, 1224, 1676) break with D3.** The plan's
  fact-check listed only 1532-1533. T1 updates them to read `Theme::default().dim.fg`.
- **B-3: `text_area.rs:1260` compares the cursor span to `theme.selected` by value.** It moves to `theme.cursor` in
  T2. The plan said "tests keep REVERSED", which holds for the modifier asserts but not for this one.
- **B-4: `requirements/detail.rs:162` (`revision`, `by <deciding key>`) becomes `key`** along with `coverage`'s key
  at 121. The plan named the file without lines. This one is a key too.
- **B-5: the plan's T2 validate command does not parse.** `cargo test` takes one positional filter. Several
  filters go after `--`, as written in each task above.
- **B-6: the plan's survey leaves 20-odd non-focus `accent` uses** (sweep item 7). Changing them is out of the
  confirmed scope, and no test pins them, so MOD-80 does not touch them. The acceptance line "Cyan means focus and
  selection only" therefore holds for the meanings the plan lists (keys, `in_progress`/running, diff `+`, active
  tab, top bar), not for the whole TUI. The "needs a person" group fits `warning` by D6's own reasoning. This is
  a candidate follow-up for the maintainer at close-out.
- **B-7: order inside T1/T2.** T1 adds `cursor` but keeps `selected = REVERSED`. T2 swaps the three cursor sites
  and the two `select` sites, rebuilds the Documents cursor cells, and only then flips `selected` to Black on
  Cyan. If T1 flipped it, the T1 commit would fail the eleven `REVERSED` cursor asserts in `text_field` /
  `text_area` / `execution_graph` and the Documents assert. T1's `select` test therefore uses an inline
  Black-on-Cyan `selected`.

## Edge cases (recorded)
1. A selected Backlog **project header** row: its `title` span keeps BOLD under `select`; its `fg Reset` becomes
   Black. Covered by `select` keeping modifiers; no separate test.
2. A selected row under `monochrome()`: spans that were `DIM` become `DIM | REVERSED`. Still visible.
3. `Indexed(244)` on a 16-colour terminal falls back to the nearest grey (plan risk table). Nothing to do.
4. The main and Settings strips underline their padding spaces (T4 note). The detail strip does not, because it
   has no padding inside the span.
