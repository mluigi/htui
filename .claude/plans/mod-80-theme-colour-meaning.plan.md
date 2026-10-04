# Plan: MOD-80 theme and colour meaning

**Source**: HANDOFF `MOD-80` (TUI design review of 2026-10-04,
https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH findings 5-8, 12; `R-TUI-1`, `R-NF-1`)
**Routed**: plan path via `/handoff-run` (0 criteria fired firmly; C3 and C4 borderline, flagged low-confidence),
accepted by the maintainer 2026-10-04. Sandbox run `hr/MOD-80`.
**Complexity**: Medium. Style only, about 25 files, mostly one-line style swaps. No store, engine, migration or
`.sqlx` change.
**Status**: confirmed 2026-10-04 (all six decisions as proposed, recommendations taken on the four maintainer calls); implementation in progress

## Summary

Every colour in the TUI comes from `ui/theme.rs`. The four step and run status literals in
`backlog/detail/runs.rs` are the exceptions; the item also names `ui/cells.rs` and `execution_graph.rs`, but
their literals are in tests only. The item lists five defects: selection inverts each cell's own colour,
cyan means five things, `title` and `dim` fail contrast on common profiles, active tabs are marked by colour
alone, and the top bar draws a degraded store as ordinary text. The fix gives `Theme` a role for each meaning
and gives selection an explicit style that overrides every cell's colours. It also adds a monochrome theme
for `NO_COLOR`, because a background-only selection would vanish there (crossterm 0.29 drops colour codes
under `NO_COLOR` and keeps attributes). Snapshots are text-only and must not move; the new tests pin styles
on the rendered buffer.

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: one role per meaning.** `Theme` keeps `base`, `dim`, `title`, `accent`, `selected` and `error`, and gains:
  - `key`: item and requirement keys, and the Graph sub-tab's node labels (the item's "cross-reference links").
    `base` plus `BOLD`.
  - `running`: `in_progress`, a running run or step, a running execution-graph node. `Color::Blue`.
  - `warning`: `awaiting_approval`, a degraded store label, the waiting count. `Color::Yellow`, which today is
    a literal in three places.
  - `added`: diff `+` lines. `Color::Green`.
  - `active_tab`: the active entry of the main strip, the detail sub-tabs and the Settings sections. `accent`
    plus `BOLD | UNDERLINED`.
  - `cursor`: the single-cell text cursor in `text_field` / `text_area`, and the cursor node's border in
    `execution_graph`. `REVERSED`, which is what those three draw today, so their behaviour and tests stay
    the same.

  `accent` stays `Color::Cyan` and keeps only focus and selection: overlay and form rows marked by `accent`,
  focused form labels, the composer prompt, and the Agents table highlight.
- **D2: `selected` overrides every cell.** `selected = fg(Black).bg(Cyan)`. Selection takes the accent hue as
  its background, so cyan still means "here". A new `Theme::select(line) -> Line` patches `selected` over every
  span, as well as setting the line style, so a span's own `fg` cannot show through. The two multi-span rows
  that set `line.style(theme.selected)` (`backlog/list.rs` `row`, `requirements/tree.rs` `pad`) call it. The
  Documents table's cursor row builds its cells in `selected`. Single-style rows (Settings, Skills, boxes)
  need no code change, because their `style` value is `theme.selected` already.
- **D3: `title` = `fg(Reset)` + `BOLD`; `dim` = `fg(Indexed(244))`.** `Reset` is the terminal's own
  foreground, so it reads on light and dark profiles alike. For `dim`, the contrast ratios measured
  (WCAG formula) were:

  | colour | One Dark `#282c34` | white | Solarized light | black |
  |---|---|---|---|---|
  | `DarkGray` (One Dark `#5c6370`) | 2.32 | 6.05 | 5.60 | 3.47 |
  | `Indexed(243)` `#767676` | 3.08 | 4.54 | 4.21 | 4.62 |
  | **`Indexed(244)` `#808080`** | **3.54** | **3.95** | **3.66** | **5.32** |
  | `Indexed(245)` `#8a8a8a` | 4.06 | 3.45 | 3.20 | 6.08 |

  244 is the most balanced: at least 3.5:1 on all four. The 232-255 greys are almost never remapped by a
  theme, so the ratio holds whatever palette is loaded. The alternative, `Modifier::DIM` on the default
  foreground, follows the theme, but it gives about 3:1 on One Dark and some terminals ignore it.
- **D4: monochrome theme under `NO_COLOR`.** `Theme::monochrome()` uses the same roles with every `fg`/`bg`
  removed and these modifiers instead: `selected` = `REVERSED`, `accent` = `BOLD`, `dim` = `DIM`,
  `active_tab` = `BOLD | UNDERLINED`, `key` = `BOLD`, `title` = `BOLD`, `cursor` = `REVERSED`. `error`,
  `warning`, `running` and `added` are plain: the status word or diff gutter already says what they mean.
  `Theme::from_no_color(value: Option<&OsStr>)` follows crossterm's rule (set and non-empty means
  monochrome), so it can be tested without touching the environment. `App` builds its theme with it from
  `std::env::var_os("NO_COLOR")` where it calls `Theme::default()` today (`app/state.rs`).
- **D5: active tabs use bold and underline, not brackets.** Brackets would add two columns per tab: the
  detail strip's width is pinned against the pane (`the_detail_strip_fits_the_detail_pane`, MOD-30 D1), MOD-81
  owns strip overflow, and every snapshot would move. Bold and underline change no text, survive `NO_COLOR`,
  and the colour stays as a third cue. The Skills `Skills │ Templates` strip is left to MOD-82 (one sub-tab
  widget); here it changes only because `title` loses `White`.
- **D6: the top bar draws degraded states in `warning`.** The store label is drawn in `warning` when it is
  `connecting` or starts with `offline` (`Backend::label`); `online` and `memory` stay `base`. The match goes
  in a small pure `store_style(label, theme)` in `top_bar.rs`. The waiting count moves from `accent` to
  `warning`, since the MOD-69 T3.3 comment says it used `accent` only because `Theme` had no warning style.
  **Maintainer call:** keep the waiting count in `accent` if "owed you" should read as focus rather than
  warning.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Theme roles | `crates/htui/src/ui/theme.rs` `Theme` | One documented `pub` field per role, built in `Default`; views never name a colour |
| Status palette | `theme.rs` `Theme::status_style`, `runs.rs` `run_style` / `step_style` | `match` on the status enum to a theme field |
| Buffer style tests | `crates/htui/src/ui/top_bar.rs` tests (`draw`, `the_waiting_count_is_accented_only_when_non_zero`) | `TestBackend` draw, assert `buffer[(x, y)].style().fg` against `theme.<role>.fg` |
| Span style tests | `crates/htui/src/ui/diff.rs` `an_added_line_is_accented` | assert `line.style == theme.<role>` on the built `Line` |
| Integration style tests | `crates/htui/tests/settings.rs` `accented_lines` | read cells by colour off `Theme::default()` |
| Naming | `theme.rs` doc comments | Each field says what it is for and where it shows |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/theme.rs` | UPDATE | D1-D4: roles, palette, `select`, `monochrome`, `from_no_color`, `status_style` |
| `crates/htui/src/app/state.rs` | UPDATE | D4: theme from `NO_COLOR` |
| `crates/htui/src/ui/tabs/backlog/list.rs` | UPDATE | D2 `select`; D1 key |
| `crates/htui/src/ui/tabs/requirements/tree.rs` | UPDATE | D2 `select`; D1 key |
| `crates/htui/src/ui/tabs/backlog/detail/documents.rs` | UPDATE | D2 cursor row cells; test asserting `REVERSED` |
| `crates/htui/src/ui/text_field.rs`, `crates/htui/src/ui/text_area.rs` | UPDATE | D1 `cursor` replaces `selected` (tests keep `REVERSED`) |
| `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | UPDATE | D1 `cursor` border; `running` / `warning` borders; tests on `Color::Cyan` |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | D1 `run_style` / `step_style` literals to `running` / `warning` |
| `crates/htui/src/ui/diff.rs` | UPDATE | D1 `added`; rename `an_added_line_is_accented` |
| `crates/htui/src/ui/tabs/backlog/detail/body.rs`, `.../detail/graph.rs`, `.../detail/requirements.rs`, `crates/htui/src/ui/tabs/requirements/detail.rs` | UPDATE | D1 key |
| `crates/htui/src/ui/tabs/registry.rs`, `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | D5 `active_tab` |
| `crates/htui/src/ui/top_bar.rs` | UPDATE | D6 |
| `crates/htui/tests/*.rs` | UPDATE (only if needed) | Integration tests that read `accent` / `error` by colour keep working; fix any that keyed on keys |

## Tasks

All tasks build on T1's `Theme`, and T2 and T3 both edit `list.rs`, `tree.rs` and `execution_graph.rs`.
They run **serial**, in one implementer lane.

### T1: `Theme` roles, palette and monochrome (D1-D4)
- **Action**: tests first in `theme.rs`. `select` overrides a span's `fg`. `from_no_color` maps `None` and
  `Some("")` to default and `Some("1")` to monochrome. `monochrome` has no `fg`/`bg` on any role.
  `status_style(InProgress) == running` and `(AwaitingApproval) == warning`. Then add the fields, palette,
  `select`, `monochrome` and `from_no_color`, and wire `app/state.rs`.
- **Validate**: `cargo test -p htui --lib ui::theme`

### T2: selection and cursor (D2, D1 `cursor`)
- **Action**: buffer tests first. A selected Backlog item row has `bg == Cyan` and `fg == Black` in every cell,
  key and status included. The same for a selected requirements-tree row and the Documents cursor row. Then
  `select` in `list.rs` / `tree.rs`, Documents cursor cells, and `cursor` in `text_field`, `text_area` and the
  `execution_graph` border.
- **Validate**: `cargo test -p htui --lib ui::tabs::backlog ui::tabs::requirements ui::text_field ui::text_area`

### T3: one meaning per colour (D1 `key`, `running`, `warning`, `added`)
- **Action**: tests first. A Backlog key cell is `BOLD` and not `Cyan`. An `in_progress` status cell is
  `running`. A running step and run render `running`. A `+` diff line is `added`. Then swap the call sites
  listed under Files to Change and update the `execution_graph` tests that assert `Color::Cyan` on running
  and awaiting borders.
- **Validate**: `cargo test -p htui --lib`

### T4: active tabs without colour (D5)
- **Action**: tests first. Under `Theme::monochrome()`, the active entry of each of the three strips is
  `BOLD | UNDERLINED` and the inactive ones are not. Then use `active_tab` in the three `render_strip` /
  `strip_line` functions.
- **Validate**: `cargo test -p htui --lib ui::tabs` (snapshot texts unchanged)

### T5: degraded store label (D6)
- **Action**: tests first in `top_bar.rs`. `offline · 3m` and `connecting` are drawn with `warning.fg`;
  `online` and `memory` with `base`. Update `the_waiting_count_is_accented_only_when_non_zero` to D6's outcome.
  Then `store_style`.
- **Validate**: `cargo test -p htui --lib ui::top_bar`

### T6: docs
- **Action**: rewrite the `theme.rs` module doc so it names the roles and the `NO_COLOR` rule. Close-out
  records D1-D6 in the decisions write-up (lifecycle P2).

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --all-features -- --test-threads=1     # snapshots must not change
cargo test --workspace --all-features --no-fail-fast
```

`cargo insta` pending snapshots must be empty: a `.snap.new` means a text change, which this item must not make.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A view sets `line.style(theme.selected)` over coloured spans that this survey missed | Medium | T2 adds a sweep test per tab, and the reviewer greps `theme.selected` |
| `Blue` reads poorly on default dark xterm (`#0000ee`) | Low | Only status words use it; the text itself says `in_progress`. Maintainer can pick another at CONFIRM |
| Integration tests that find rows by `accent` colour (`tests/settings.rs` `accented_lines`) miss rows now marked by `selected` | Medium | The Agents table keeps its `accent` highlight; T2 runs the integration suite with `--all-features` (testkit) |
| MOD-67 runs in parallel and rewrites hint rows and snapshots | Medium | This item changes no text; conflicts would be in `render_strip` and hint call sites, resolved at merge |
| A 16-colour terminal approximates `Indexed(244)` | Low | It falls back to the nearest grey; still no worse than `DarkGray` |

## Acceptance

- [ ] A selected Backlog row is one even block (every cell `fg Black / bg Cyan`); pinned by a buffer test
- [ ] Cyan means focus and selection only: keys bold, `in_progress` / running `running`, diff `+` `added`
- [ ] `title` is `Reset` + bold; `dim` is `Indexed(244)`
- [ ] Under `Theme::monochrome()` the active tab, sub-tab and Settings section, and the selected row, are
      visible through modifiers alone
- [ ] `offline · …` / `connecting` drawn in `warning`
- [ ] No snapshot text changed; all gates in Validation green

## Verified claims

Step 3.5 fact-check, 2026-10-04. Compile probe: scratch crate on `ratatui = "=0.30.2"` (the `Cargo.lock` version),
`TestBackend` 10×3.

| Claim | Verdict | Evidence |
|---|---|---|
| Only `runs.rs` has non-test colour literals; those in `cells.rs` and `execution_graph.rs` are test-only | true (amends the item text) | `runs.rs:962-974` (`run_style` / `step_style`); `cells.rs` literals at 744/748, below `mod tests` at 268; `execution_graph.rs` literals at 1184+, below `mod tests` at 748 |
| `line.style(selected)` does not override a span's own `fg` | true, and worse than the item says | Probe: a `Cyan` span under a `Black/Cyan` line style draws `fg Cyan, bg Cyan`. Without D2 the key would disappear, cyan on cyan |
| Patching `selected` over every span gives an even block and keeps `BOLD` | true | Probe: `(Black, Cyan, BOLD)` on the key cells, `(Black, Cyan, NONE)` on the rest |
| A `Row::style` does not override a cell `Line`'s `fg` | true | Probe: a `DarkGray` cell under a `Black/Cyan` row draws `fg DarkGray, bg Cyan`, hence D2's per-cell Documents row |
| Exactly three sites set `selected` as a line or row style over multi-span content | true | `grep '\.style(.*selected'`: `backlog/list.rs:235`, `requirements/tree.rs:340`, `detail/documents.rs:337`. Every other `theme.selected` is passed to `Line::styled` or one `Span` (`prompt.rs`, `connection.rs`, `qdrant.rs`, `library.rs`, `templates.rs`, `boxes.rs`, `attach.rs`) |
| `theme.selected` is also the text cursor and the graph cursor border | true | `text_field.rs:338`, `text_area.rs:333`, `execution_graph.rs:231` |
| crossterm 0.29 honours `NO_COLOR` set and non-empty, dropping colour codes only | true | `crossterm-0.29.0/src/style/types/colored.rs:75` `ansi_color_disabled` (`!var.is_empty()`), checked in `Colored::fmt` (line 100); `Attribute` printing is not gated |
| `App` builds its theme with `Theme::default()` in one place | true | `app/state.rs:238` |
| Store labels are `memory` / `online` / `connecting` / `offline · <age>` | true | `htui-store/src/backend.rs:91` `Backend::label` |
| The top bar draws the store label in `base` and the waiting count in `accent` "because Theme has no warning style" | true | `top_bar.rs` `render`, MOD-69 T3.3 comment |
| The three strips mark the active entry with `accent` alone | true | `tabs/registry.rs` `render_strip`, `tabs/settings/mod.rs` `render_strip`, `tabs/backlog/detail/mod.rs` `strip_line` |
| The detail strip's width is pinned by a test | true | `tabs/backlog/mod.rs:2183` `the_detail_strip_fits_the_detail_pane` |
| Snapshots are text-only | true | `testkit.rs:594` `Harness::render` returns `buffer_text(..)`; `tests/settings.rs:85` `text_of` |
| Integration tests read `accent` / `error` by colour | true, none affected | `tests/settings.rs:113` `accented_lines` (`accent` stays `Cyan`); four `error.fg` reads (`error` unchanged) |
| `execution_graph` tests assert `Color::Cyan` on running and awaiting borders | true, to update in T3 | `execution_graph.rs:1532-1533` |
| `Indexed(244)` is `#808080` | true | xterm grey ramp: 232 + n is `8 + 10n`, so n = 12 gives 128 = `0x80` |
| Contrast ratios in D3 | computed | WCAG relative-luminance formula, see the D3 table |
| T1-T5 are independent | false, so they run serial | T2 and T3 both edit `list.rs`, `tree.rs` and `execution_graph.rs`, and every task needs T1's `Theme`. Marked serial in Tasks |
