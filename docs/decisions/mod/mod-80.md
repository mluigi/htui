# MOD-80 - Theme and colour meaning (done, 2026-10-04)

**Requirements:** `R-TUI-1`, `R-NF-1`.
**Origin:** the TUI design review of 2026-10-04 (https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH,
findings 5-8 and 12).
**Artifacts:**
- plan [`.claude/plans/mod-80-theme-colour-meaning.plan.md`](../../../.claude/plans/mod-80-theme-colour-meaning.plan.md): D1-D6, with its verified-claims table;
- blueprint `.claude/plans/mod-80-theme-colour-meaning.blueprint.md`: T1-T6, amendments B-1 to B-7.

Decision numbers are local to MOD-80 (the MOD-31 convention).

Routed as **plan**: no criterion fired firmly, and C3 and C4 were borderline, so the verdict was flagged
low-confidence. Ran in a TOOL-7 sandbox (`hr/MOD-80`). Every task depended on T1's `Theme`, and T2 and T3
shared three files, so the tasks ran serially in one lane.

**Decisions (maintainer, 2026-10-04):**
- route accepted, no ultracode;
- plan confirmed, with the recommendation taken on each of its four open calls: bold and underline
  for active tabs (not brackets), the waiting count in `warning`, `dim` = `Indexed(244)`, `running` =
  blue and `added` = green;
- review: M1 fixed in part, and the rest filed as **MOD-85**; M2, L1, L2, L3, L4, L6 and the nits
  applied; L5 deferred to MOD-85.

**Commits:**
- plan and blueprint: `4d83cc04`, `10e4c480`, `1e1b1b82`;
- T1 theme roles, monochrome theme, `select`: `43d087e8`;
- T2 selection overrides every cell, `cursor` role: `35a1f08c`;
- T3 one meaning per colour: `164d4030`;
- T4 active tabs bold and underlined: `3cfc736c`;
- T5 degraded store label and waiting count in `warning`: `49b5c0a6`;
- T6 `theme.rs` module doc: `92351f01`;
- review fixes: `82c6d69c` (M1), `941be946` (M2), `2659f6f0` (L1), `171e20a4` (L2), `ea072178` (L3),
  `8e7da962` (L6, L4, nits).

---

## What was decided and built

### D1 - one theme role per meaning

`Theme` (`crates/htui/src/ui/theme.rs`) keeps its six roles and gains six more:

| Role | Used for | Style |
|---|---|---|
| `key` | item and requirement keys, Graph node labels, a revision's `by <key>` | `base` + `BOLD` |
| `running` | `in_progress`, a running run or step, a running flow node | blue |
| `warning` | `awaiting_approval`, a degraded store label, the waiting count, Runs-pane lines that need a person | yellow |
| `added` | diff `+` lines | green |
| `active_tab` | the active entry of the main strip, the detail sub-tabs and the Settings sections | `accent` + `BOLD` + `UNDERLINED` |
| `cursor` | the text cursor in `text_field` / `text_area`, and the flow cursor node's border | `REVERSED` |

`accent` (cyan) is for focus and selection. About twenty older `accent` uses still mean something else
(the Documents kind column, flow edge labels, divergence state, chat headers, form values). They are
**MOD-85**. `runs.rs` no longer names a colour; every colour now comes from `theme.rs`.

### D2 - selection overrides every cell

`selected` is black on cyan. `Theme::select(line)` patches it over the line and over every span. The
plan's compile probe on ratatui 0.30.2 showed that a line style alone lets each span keep its own
`fg`: a cyan key under a cyan selection draws cyan on cyan and disappears. The Documents table's cursor
row builds its cells in `selected` for the same reason, because a `Row::style` does not override a
cell's `fg`. Rows drawn in one style already passed `theme.selected`, so they needed no code change. After
review L1, the Settings Kinds, Hierarchy and Personas cursor rows use `selected` as well (they used
`accent`), so every Settings list marks its cursor the same way.

### D3 - contrast

`title` is the terminal's own foreground (`Reset`) plus `BOLD`, so it reads on light and dark profiles
alike. `dim` is `Indexed(244)` (`#808080`), with at least 3.5:1 contrast on One Dark, white, Solarized
light and black (`DarkGray` gave 2.3:1 on One Dark). The plan has the full table.

### D4 - a monochrome theme under `NO_COLOR`

crossterm 0.29 drops colour codes under `NO_COLOR` and keeps attributes. A selection drawn only as a
background would vanish there, so `Theme::monochrome()` uses the same roles with every colour removed:
`selected`, `cursor` = `REVERSED`; `accent`, `key`, `title`, `error` (review L6) = `BOLD`; `dim` = `DIM`;
`active_tab` = `BOLD | UNDERLINED`. `Theme::from_no_color` treats a `NO_COLOR` that is set and non-empty
as monochrome (no-color.org). Because it reads `var_os`, a non-UTF-8 value counts as set, whereas
crossterm treats it as unset; the only effect is a colourless theme on a colour terminal. `NO_COLOR` is
read only in `lib.rs` `run` (B-1). Test harnesses build `Theme::default()`, so a developer shell with
`NO_COLOR` set does not change test results.

### D5 - active tabs use bold and underline

Brackets would add two columns per tab. That would break the detail-strip width pin
(`the_detail_strip_fits_the_detail_pane`), overlap MOD-81's overflow work, and move every snapshot. Bold
and underline change no text and survive `NO_COLOR`, and the cyan stays as a third cue. The Skills
`Skills │ Templates` strip is MOD-82's (one sub-tab widget).

### D6 - degraded states in `warning`

`top_bar::store_style` draws `connecting` and `offline · <age>` in `warning`, and `online` and `memory` in
`base`. A test feeds the real `Backend::label()` output into it (review L3), so renaming a label breaks
the test. The waiting count moved from `accent` to `warning`: the MOD-69 code used `accent` only because
`Theme` had no warning style.

## Tests

Snapshots are text-only (`Harness::render` returns `buffer_text`), so the new tests pin styles on the
drawn buffer. The ones that would fail on the old code:
- a selected Backlog row is one block, every cell `fg Black / bg Cyan`;
- the same for a selected withdrawn requirement (all `dim`), which catches a revert of `pad` (review M2);
- the Documents cursor row is black on cyan across its width;
- each Settings section's cursor row is black on cyan;
- a selected row under `Theme::monochrome()` is `REVERSED` and the next row is not;
- the three strips mark the active entry `BOLD | UNDERLINED` under monochrome;
- the store label's warning style;
- running and awaiting flow borders;
- the Runs pane's waiting lines in `warning`.

No snapshot text changed. Gates on the final tree: fmt clean; clippy clean with `--all-targets
--all-features` and without features; `htui` 2300 passed / 0 failed; workspace 4621 passed / 0 failed /
30 ignored (`--all-features --no-fail-fast -- --test-threads=1`).

## Carried

- **MOD-85**: the remaining `accent` uses that do not mean focus or selection (blueprint B-6, review
  M1). Also: the Skills attach activation field marks focus with `selected` (review L5), and the
  concepts-search, workspace-switcher and path-picker overlays mark their cursor row with `accent`
  alone, which is bold-only under `NO_COLOR`.
- Check by eye: the main and Settings strips underline the padding spaces around the active title,
  and a selected Kinds "graph missing" row draws its red tail black on cyan.
- Not caused by MOD-80: `cargo doc -p htui --no-deps` already fails on `main` (eight private
  intra-doc links and a redundant link target in `persona_import.rs`).
