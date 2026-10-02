# Plan: MOD-60 — display width in every hand-laid-out row

> **Status: complete (2026-10-03).** CONFIRMED 2026-10-02 (OQ-1 yes); implemented T0 `33046f2b`..`932ed356`,
> lanes T1-T4 merged `30c4a4cc`..`326b74a0`, T5 `444d5dc0`; rust-reviewer approve-with-fixes, every
> finding applied (`80173bb0`..`9b58ebe0`). Deviations B1-B13 in the blueprint.

**Source**: `HANDOFF.md:303-317`, MOD-60 (from MOD-54, plan D18), plus the MOD-13 milestone 3 review
L3 deferral (`HANDOFF.md:449-451`: the divergence view's notice, clip and column padding) and the
overlays, which MOD-54 D18 named and the HANDOFF line dropped.

**Requirements**: `R-TUI-1` (keyboard-driven TUI), `R-NF-1` (Windows/Linux/macOS — nothing here is
platform-specific; no new dependency).

**Routing**: plan path (C4 fired, 1 of 4), confirmed by the maintainer 2026-10-02, ultracode for
the implement phase. Chain: plan → fact-check → CONFIRM → `code-architect` → T0, then T1 ∥ T2 ∥ T3
∥ T4 as one Workflow (implement → adversarial verify), then T5 → `rust-reviewer`.

**Complexity**: Medium. One shared module grows (`ui/cells.rs`), ~25 renderer files change, all
inside `crates/htui/src/ui/`. No new dependency, no `Cargo.lock` move, no migration, no store
surface, no `pub` item outside `ui/`, no `.sqlx`, no conformance case. **No snapshot is expected
to move** except as OQ-1 decides.

**Numbering**: decisions D1…D12, risks R-1…R-6, open question OQ-1, tasks T0…T5. New item, so D
restarts at D1; cite as "MOD-60 D*n*".

**Base**: `hr/MOD-60` at `aeba8f63` (main after the MOD-13/26/28/37 merges). Line numbers are the
files' at that commit.

---

## Summary

`ui::cells::cell_width` (MOD-54) is the renderer's own measurement, but about 60 layout sites in
`ui/` still pad, clip, align or wrap by `char` count (`chars().count()`, `chars().take(n)`,
`format!("{:<w$}")` — std pads by char count — and `" ".repeat(w - n)`). A CJK string is twice as
wide as it counts, so columns misalign and rows overrun their pane. This item adds the missing
shared operations to `ui::cells` (`clip`, `pad`, `pad_left`, `fit`, `wrap`, `clip_spans`), replaces
the ~14 local copies with them, and converts every layout site.

## Correction to the item text (fact-checked)

The three files HANDOFF names first have **no char-counted layout code** (claims V1–V3):

- `ui/diff.rs` makes one unwrapped `Line` per diff line; the Templates and Library panes measure
  it with `Line::width()` (cells) and draw it with `Paragraph::wrap`. The pane's row estimate
  (`line.width().div_ceil(width)`, `skills/templates.rs:979`) is a **documented lower bound** on
  the word wrap ("so the clamp never scrolls the pane blank", `:973-974`); a CJK hunk wrapping to
  more rows is the same, intended, under-count an ASCII word wrap already has. Not a MOD-60 defect.
- `ui/top_bar.rs` is one `Paragraph` of spans; there is no clock in `ui/`, and the tab strip
  (`tabs/registry.rs:216`) is also one `Paragraph` of spans. ratatui clips both by cell.
- `ui/tabs/chat/transcript.rs` has no wrap or clip: it selects whole `Line`s by row index and
  `chat/mod.rs` draws them unwrapped, so ratatui clips by cell. **None of these three files is
  touched**, and the chat transcript snapshot risk the item warns about does not arise. The real
  chat defects are `chat/mod.rs`'s header helpers (`columns`, `clip`), below.

The close-out records this correction in the write-up so the HANDOFF list is not re-inherited.

## Decisions

| # | Decision | Why / rejected |
|---|---|---|
| D1 | **`ui::cells` gains the shared operations**, all `pub(crate)`, all measuring with `cell_width` and stepping by `graphemes`: `clip(text, width) -> String` (cut so the result plus `…` fits; `…` only when something was cut; width 0 → `""`; a wide cluster that would straddle the limit is dropped, so the result may be one cell short), `pad(text, width) -> String` (append spaces up to `width` cells; never clips), `pad_left(text, width)` (right-align, for `{:>N}`), `fit(text, width)` = `pad(&clip(text, width), width)` (exactly `width` cells unless a straddling wide cluster leaves one blank — padded), `wrap(line, width) -> Vec<String>` (D5), `clip_spans(spans, width) -> Vec<Span<'static>>` (cut from the end, keep styles, `…` in the last kept span's style). | The module doc already says "every cell count in `ui/` comes from `cell_width`"; the copies are the reason it is not true. **Rejected: per-file fixes.** 14 local helpers (`list::pad`, `tree::padded`, `detail::padded`, `forms::padded`, `list::clip`, `attach::cut`, `reqs::cut`, `runs::fit`, `runs::fit_line`, `runs::wrap_line`, `chat::clip`, `chat::columns`, `boxes::clip`, `agents::fit_label`) would each be re-fixed separately and drift again. |
| D2 | **Ellipsis and width-0 rule is unified on the cell-correct precedent** (`graph.rs::clip`, `execution_graph.rs::clip`): `…` reserves one cell, width 0 returns `""`. `list::clip` and `attach::cut` currently return `"…"` at width 0 — one cell over. | Never emit a string wider than its budget. No snapshot renders a width-0 clip (V9). |
| D3 | **Control characters**: `clip`/`fit`/`wrap` draw a C0/DEL cluster as one blank cell (control → `' '`), as `reqs::cut`, `runs::fit`, `runs::fit_line`, `execution_graph::clip` and `divergence::wrap_row` already do. `pad`/`pad_left` do not rewrite content. | `cell_width("\u{1}") == 1` but ratatui draws nothing useful there; the four existing flatteners agree. For the sites that did not flatten (`list::clip`, `attach::cut`), titles/labels are single-line user text where a raw control char already rendered as garbage — flattening is the fix, and no snapshot contains one (V9). |
| D4 | **`wrap` is `divergence::wrap_row` promoted** (it is `runs::wrap_line` measured in cells — same algorithm, V7): break at a space where one fits, inside a word by grapheme only when the word alone is wider, keep leading spaces, an empty line is one empty row. `divergence::wrap_row` and `runs::wrap_line` are deleted. | Already cell-correct and already tested (`no_wrapped_row_is_wider_than_its_column`). |
| D5 | **`settings::wrapped` keeps its signature and whitespace-collapsing word wrap, measures in cells, and (OQ-1, default) hard-breaks a word wider than the width by grapheme.** It stays in `settings/mod.rs` (its 5 outside importers keep their `use`); `item_form::notice_lines` — today `wrapped` + a char-chunk hard-break — becomes a call to `wrapped`. | A CJK sentence has no spaces, so it is one "word"; without the hard-break, a cell-measured `wrapped` still leaves a CJK notice as one overlong row that ratatui clips (text lost, and the notice height that callers size from `wrapped(..).len()` is wrong). |
| D6 | **Column alignment via std `format!` width specifiers on runtime text is replaced by `cells::pad`/`pad_left`/`fit`.** `{:<w$}` on compile-time ASCII literals/enums (form labels, link kinds, status enums, digits) may stay — the gate's allowlist (D11) names each. | std's `{:<w$}` pads by `char` count; correct only when every char is one cell. Converting ASCII-only sites is churn; listing them is the audit. |
| D7 | **`requirements/tree.rs` project header: clip the name, keep the marker.** The header is `"{marker} {name} ({n})"` + `" · read-only"`. Measure in cells; keep the prefix, ` ({n})` and ` · read-only` intact and `cells::clip` the **name** to the remaining room; only if fewer than 2 cells of name would remain, drop ` · read-only` first, then fall back to `clip_spans` on the whole line. `tree::pad` otherwise becomes `clip_spans` + `pad` (body rows still cut the body). | Today `pad` cuts the last span — the marker — and at ≥12 cells over returns a lone `…` and lets ratatui chop the name with no ellipsis (MOD-39 "Carried"). The marker is the information; the name is the elidable part. Precedent: `graph.rs::fit_label` (priority-ordered fit). |
| D8 | **Divergence view (MOD-13 L3)**: value cells wrap with `item_form::notice_lines` → `wrapped` (D5) and pad with `cells::pad` instead of `{text:<vw$}`; label/state columns use `cells::fit`; headings, section labels and the hint use `cells::clip`. | The three-way row stays exactly `LABEL + STATE + 3·vw + 2` cells wide with CJK values; today a wide value pushes theirs/mine across the gaps. |
| D9 | **`chat/mod.rs`**: `columns` and `clip` are deleted for `cells::cell_width`/`cells::clip`. | `Span::width` skips the halfwidth sound-mark rule (`ｶﾞ` = 1, renderer = 2) and the per-`char` loop splits ZWJ clusters. The unit test `a_narrow_promoted_header_keeps_the_whole_session_ref` switches to `cell_width`. |
| D10 | **Not in scope**: `diff.rs`, `top_bar.rs`, `transcript.rs` (see Correction); the Templates/Library diff row estimate (documented lower bound); the composer's missing horizontal scroll (an ASCII defect too — not a width bug); soft wrap, bidi, terminals that draw CJK as one cell (item text); `text_field.rs`/`text_area.rs`/`path_picker.rs` wrap/clip/`concepts_search.rs` (already cell-correct, MOD-54). `path_picker.rs:292`'s `GOTO.len()` becomes `cell_width(GOTO)` for consistency (one line). | Scope recorded, not quietly crossed (MOD-54 D18's rule). |
| D11 | **Sweep gate**: T5 adds no script; it runs a fixed `rg` over `crates/htui/src/ui` for `chars\(\)\.count\(\)`, `chars\(\)\.take\(`, `\.repeat\(` and `:<\w+\$\}`/`:>\w+\$\}`/`:<\d+\}`/`:>\d+\}`, and every remaining hit must be on the allowlist written into this plan's "Allowlist" section at T5 (non-layout: Debug lengths, paste byte budgets, UUID/digest slicing, `TextArea::len`, test key-typing helpers, ASCII-constant `format!` widths per D6). | The item title is "every" — the audit has to be checkable, not prose. |
| D12 | **Tests**: every converted helper gets a CJK case (literals written as `\u{…}` escapes, house style) asserting `cell_width(out) <= width` and the exact output; cells.rs gets a table test per new op (ASCII, CJK, ZWJ family, combining mark, halfwidth sound mark, control char, width 0/1/2). Renderer tests are **unit tests in-file** (cells is crate-private, unreachable from `tests/`; and `testkit::buffer_text` does not skip wide-glyph continuation cells). Existing assertions that measure with `chars().count()` on rendered rows switch to `cell_width` (runs.rs:1923/2327/2384/2418/4476, divergence.rs:1127, filter.rs:886, item_form.rs:1432). **No new insta snapshot.** | TDD per repo convention; snapshot churn is the risk the item names. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Shared measurement | `crates/htui/src/ui/cells.rs:45` | `pub(crate) fn`, `#[must_use]`, module-private, doc cites the decision |
| Cell-correct clip | `crates/htui/src/ui/tabs/backlog/detail/graph.rs:618`, `runs/execution_graph.rs:257` | reserve one cell for `…`, `""` at 0 |
| Cell-correct wrap | `crates/htui/src/ui/tabs/backlog/divergence.rs:571` (`wrap_row`) | graphemes + `cell_width`, control → space |
| Priority fit | `crates/htui/src/ui/tabs/backlog/detail/graph.rs:601` (`fit_label`) | elide the name, keep the fixed parts |
| Wide-char tests | `graph.rs` `a_wide_title_never_overflows_the_pane`, `divergence.rs` `a_wide_character_body_wraps_and_shows_its_last_character`, `execution_graph.rs` `text_is_clipped_by_display_width` | `\u{…}` literals, `assert!(cell_width(x) <= w, "{x:?} against {w}")` |
| Errors / logging | — | none: pure rendering functions, no fallible path, no logging (no pattern to mirror) |

## Files to Change

| File | Task | Action | Why |
|---|---|---|---|
| `crates/htui/src/ui/cells.rs` | T0 | UPDATE | D1–D4 ops + tests; module doc lists them |
| `crates/htui/src/ui/tabs/settings/mod.rs` | T0 | UPDATE | `wrapped` in cells + hard-break (D5) |
| `crates/htui/src/ui/tabs/settings/{agents,boxes,connection,hierarchy,kinds,prompt,qdrant}.rs` | T1 | UPDATE | `fit_label`, `boxes::clip`/`list_label`, `delete_pane`, `PathsForm` column, the 5 hint checks, `Editor::lines` pads, `clash_notice` |
| `crates/htui/src/ui/tabs/backlog/{list,filter,item_form,divergence}.rs` | T2 | UPDATE | `list::pad`/`row`/key column, clips, `notice_lines`, D8; `list::clip` kept until T5 |
| `crates/htui/src/ui/tabs/backlog/detail/{requirements,runs,prompt}.rs` | T2 | UPDATE | `cut`, `fit`, `fit_line`, `wrap_line`, footers, `section_lines` |
| `crates/htui/src/ui/tabs/requirements/{tree,detail,forms}.rs` | T3 | UPDATE | `pad`/`padded` (D7), key columns, `coverage`, `revision` |
| `crates/htui/src/ui/tabs/skills/{attach,library,templates}.rs` | T4 | UPDATE | `cut`, name columns, prompt measures |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | T4 | UPDATE | name column (measure + pad) |
| `crates/htui/src/ui/tabs/chat/mod.rs` | T4 | UPDATE | D9 |
| `crates/htui/src/ui/path_picker.rs` | T4 | UPDATE | `GOTO.len()` → `cell_width` |
| `crates/htui/src/ui/tabs/backlog/list.rs` | T5 | UPDATE | delete `list::clip` once T2/T3 importers are gone |
| `.claude/plans/mod-60-display-width.plan.md` | T5 | UPDATE | D11 allowlist |

Untouched on purpose: `ui/diff.rs`, `ui/top_bar.rs`, `ui/tabs/chat/transcript.rs`, every `tests/*.rs`
and `tests/snapshots/*` (unless OQ-1's snapshot moves are accepted), `text_field.rs`, `text_area.rs`.

## Tasks

### T0 — shared operations (serial, first)
- **Action**: tests first in `cells.rs` (D12 table), then `clip`, `pad`, `pad_left`, `fit`, `wrap`
  (moved from `divergence::wrap_row` — the move itself is T2's deletion; T0 adds the cells copy),
  `clip_spans`; then `settings::wrapped` (D5) with a CJK test and an overlong-ASCII-token test.
- **Files**: `ui/cells.rs`, `ui/tabs/settings/mod.rs`.
- **Mirror**: `cells.rs` style; `execution_graph::clip` behaviour.
- **Validate**: `cargo test -p htui --lib ui::cells` and `ui::tabs::settings`; clippy.

### T1 — settings sections (parallel after T0)
- **Files**: `ui/tabs/settings/{agents,boxes,connection,hierarchy,kinds,prompt,qdrant}.rs`.
- **Action**: CJK-failing sites first (`fit_label`, `boxes::clip`/`list_label`, `PathsForm::lines`,
  `delete_pane`), each with a red unit test; then the ASCII-constant sites per D6 (convert where
  the input is runtime text — the hint checks measure a notice that can carry an error — allowlist
  the rest). `boxes::clip` and `fit_label` are deleted for `cells::clip`/`cells::fit`.

### T2 — backlog (parallel after T0)
- **Files**: `ui/tabs/backlog/{list,filter,item_form,divergence}.rs`,
  `ui/tabs/backlog/detail/{requirements,runs,prompt}.rs`.
- **Action**: `list::pad`/`row`/key column/title clip; filter and item_form clips;
  `notice_lines` → `wrapped`; D8; reqs `cut` → `cells::clip`; `runs::fit`/`fit_line`/`wrap_line`
  → `cells::fit`/`clip_spans`+`pad`/`cells::wrap`; footers; `divergence::wrap_row` deleted for
  `cells::wrap`. Switch this lane's own imports of `list::clip` to `cells::clip` but **do not
  delete `list::clip`** (tree.rs, T3, still imports it until both lanes land). Rendered-row test
  assertions per D12.

### T3 — requirements tab (parallel after T0)
- **Files**: `ui/tabs/requirements/{tree,detail,forms}.rs`.
- **Action**: D7 with red tests first (narrow pane keeps ` · read-only` and elides the name; CJK
  name; never wider than the pane at every width 1…45); `padded` ×3 → `cells::pad`; key columns and
  `coverage` measures; `use …list::clip` → `cells::clip`.

### T4 — skills, overlays, chat (parallel after T0)
- **Files**: `ui/tabs/skills/{attach,library,templates}.rs`, `ui/overlay/workspace_switcher.rs`,
  `ui/tabs/chat/mod.rs`, `ui/path_picker.rs`.
- **Action**: `attach::cut` → `cells::clip`; the two inline name cuts + `{name:<NAME_WIDTH$}` →
  `cells::fit`; `pane` prompt measures; switcher column; D9; `GOTO`. Reword "in chars" doc
  comments in these files.

### T5 — sweep and cleanup (serial, last)
- **Action**: delete `list::clip` (no importer left); run the D11 sweep and write the allowlist
  into this plan; full gate below; confirm `git diff --stat crates/htui/tests/snapshots` is empty
  (or only OQ-1-accepted moves).

**Independence** (by file set, V12): T1, T2, T3, T4 touch pairwise-disjoint files; all four
depend on T0's `cells` API; T5 depends on T2 and T3 (both stop importing `list::clip`).

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features --no-fail-fast -- --test-threads=1   # testkit needed; serial per house rule
git status --short crates/htui/tests/snapshots   # no *.snap.new, no modified .snap
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| R-1 — a snapshot moves | Low (all 135 snapshots are ASCII-width, V9) except OQ-1 | Any `.snap.new` stops the lane; moves are shown to the maintainer, never `cargo insta accept`ed silently |
| R-2 — parallel lanes collide on `list::clip` | Medium | T2 keeps it; T5 deletes it after both land (V11) |
| R-3 — `clip` drops a straddling wide cluster and leaves the row one cell short, so a right-hand column shifts left | Medium | `fit` always pads to exactly `width` (D1); column rows use `fit`, never bare `clip` |
| R-4 — D3 flattening changes a rendered title that held a control char | Low | No snapshot has one (V9); the change is the fix |
| R-5 — the sweep's allowlist hides a real site | Low | Each entry names file:line and the reason; reviewer checks the list |
| R-6 — the item is widened again (composer scroll, diff row estimate) | Medium | D10 records them as out of scope |

## Open question

- [x] **OQ-1 — Does `settings::wrapped` hard-break a word wider than the width?** **Answered at CONFIRM: yes**, by grapheme (D5). Without it a CJK sentence with no spaces is one
      overlong row and the notice-height arithmetic undercounts. Cost: an ASCII token longer than
      the pane (a long URL or path in an error) now wraps instead of being clipped at the border.
      Probe result (V10): the hard-break moves **no** existing snapshot or test. Alternative: measure in cells only, no hard-break (CJK notices stay
      clipped).

## Verified claims

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `diff.rs` has no char/byte layout code | ✓ | `rg 'chars\(\)\|\.len\(\)\|:<\|:>\|repeat\('` → only a test `lines.len()` |
| V2 | `top_bar.rs` has no tab bar/clock; no clock in `ui/` | ✓ | `rg '\bclock\b' ui/` → only test `TestClock` in library.rs; strip is `Paragraph::new(Line::from(spans))` at `tabs/registry.rs:216` |
| V3 | `transcript.rs` has no wrap/clip | ✓ | its `.len()` hits are row/line counts (`:204`, `:491-495`) and a displayed option count (`:592`) |
| V4 | Templates row estimate is a documented lower bound | ✓ | `skills/templates.rs:972-980` comment + `div_ceil` |
| V5 | `cells` is private to `ui` (`mod cells;`), so `tests/` cannot call it | ✓ | `ui/mod.rs:4` |
| V6 | `settings::wrapped` is imported by 5 files outside settings | ✓ | templates.rs:33, library.rs:45, requirements/mod.rs:51, requirements/detail.rs:15, item_form.rs:41 |
| V7 | `divergence::wrap_row` ≡ `runs::wrap_line` except measurement | ✓ | read both bodies (`divergence.rs:571-612`, `runs.rs:707-738`): same split/break/indent/control rules |
| V8 | `chat::clip` measures per `char` via `Span::width` | ✓ | `chat/mod.rs:115-139` |
| V9 | No snapshot holds a wide, combining or format char | ✓ | python scan of 135 `.snap` files for EAW W/F and Mn/Me/Cf → none |
| V10 | OQ-1 hard-break moves no existing snapshot or test | ✓ | probe: `wrapped` + char-chunk hard-break patched in, `cargo test -p htui --all-features --no-fail-fast` → 0 failures, 0 `.snap.new`; patch reverted (`git checkout`) |
| V11 | `list::clip` importers: tree.rs (T3), item_form/divergence/filter (T2) | ✓ | `rg 'list::clip'` → 4 files |
| V12 | T1–T4 file sets are pairwise disjoint | ✓ | Files to Change table: settings/* minus mod.rs ∩ backlog/* ∩ requirements/* ∩ {skills, overlay, chat/mod.rs, path_picker} = ∅; T0 owns cells.rs + settings/mod.rs alone |
| V13 | `tree::pad` cuts the last span (the marker) and returns `…` at keep 0 | ✓ | `tree.rs:297-316` + `list::clip` width-0 branch |

## Allowlist (D11, re-derived after the review round, at `9b58ebe0`)

Sweep (repo root):

```bash
rg -n 'chars\(\)\.count\(\)|chars\(\)\.take\(|\.repeat\(|:<\w+\$\}|:>\w+\$\}|:<\d+\}|:>\d+\}' crates/htui/src/ui
```

**155 hits**: 37 in production code, each listed below; 118 inside `#[cfg(test)] mod tests` (CJK/emoji fixtures built with `.repeat(n)`, ASCII row readers, test-oracle `format!` padding), counted per file after the table. No production hit pads, clips or wraps runtime text by `char`.

| Site | Hit | Reason |
|---|---|---|
| `cells.rs:158` | `out.push_str(&" ".repeat(width.saturating_sub(cell_width(&out))));` | `pad`/`pad_left`: fill after a `cell_width` measure |
| `cells.rs:167` | `let mut out = " ".repeat(width.saturating_sub(cell_width(&flat)));` | `pad`/`pad_left`: fill after a `cell_width` measure |
| `text_area.rs:124` | `/// How many chars, the `\n` between lines included (so `len() == text` | not layout: `TextArea::len` char-count API |
| `text_area.rs:131` | `self.text.chars().count()` | not layout: `TextArea::len` char-count API |
| `tabs/skills/templates.rs:1139` | `\|\| project.to_string().chars().take(8).collect(),` | not layout: UUID prefix |
| `tabs/skills/templates.rs:1147` | `format!("  {} v{head:<3} {role}", cells::fit(name, NAME_WIDTH))` | D6: integer `v{head:<3}`; the name is `cells::fit` |
| `tabs/skills/library.rs:1605` | `let mut spans = vec![Span::styled(format!("{label:<INFO_LABEL$}"), the` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/skills/library.rs:1711` | `format!("  {} v{head:<3}", cells::fit(name, NAME_WIDTH))` | D6: integer `v{head:<3}`; the name is `cells::fit` |
| `tabs/skills/attach.rs:930` | `\|\| project.to_string().chars().take(8).collect(),` | not layout: UUID prefix |
| `tabs/skills/attach.rs:1013` | `let mut spans = vec![Span::styled(format!("{name:<FORM_LABEL$}"), them` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/skills/attach.rs:1024` | `Span::styled(format!("{:<FORM_LABEL$}", "activation"), theme.base),` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/skills/attach.rs:1035` | `let label = Span::styled(format!("{:<FORM_LABEL$}", "effective:"), the` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/settings/qdrant.rs:485` | `lines.push(Line::styled(format!("  {l:<5}  {chunk}"), style));` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/settings/prompt.rs:1071` | `"  {:<width$}  {held} \| {} ({})   {}",` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/settings/connection.rs:263` | `.field("len", &text.chars().count())` | not layout: `Debug` length (redaction) |
| `tabs/settings/connection.rs:554` | `format!("  {label:<label_width$}  {chunk}"),` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/settings/boxes.rs:1114` | `Span::styled(format!("{name:<LABEL_WIDTH$}"), theme.dim)` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/settings/boxes.rs:1209` | `let digest: String = spec.digest.chars().take(DIGEST_SHOWN).collect();` | not layout: digest prefix (hex) |
| `tabs/backlog/list.rs:205` | `Span::raw(" ".repeat(INDENT)),` | spaces: constant indent/gap, one cell each |
| `tabs/backlog/list.rs:207` | `Span::raw(" ".repeat(GAP)),` | spaces: constant indent/gap, one cell each |
| `tabs/backlog/list.rs:212` | `Span::raw(" ".repeat(GAP)),` | spaces: constant indent/gap, one cell each |
| `tabs/backlog/list.rs:232` | `spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));` | row fill after a `cell_width` measure |
| `tabs/backlog/item_form.rs:846` | `let head = format!("{}{label:<LABEL$}", marker(focused));` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/backlog/filter.rs:341` | `let head = format!("{} {label:<LABEL$}", if focused { '>' } else { ' '` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/requirements/tree.rs:224` | `cells::clip(&format!("{}no areas", " ".repeat(AREA_INDENT)), width),` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:233` | `" ".repeat(AREA_INDENT),` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:307` | `format!("{}{WITHDRAWN_MARK} ", " ".repeat(AREA_INDENT))` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:309` | `" ".repeat(ROW_INDENT)` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:314` | `Span::styled(" ".repeat(GAP), style),` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:319` | `Span::styled(" ".repeat(GAP), style),` | spaces: constant indent/gap, one cell each |
| `tabs/requirements/tree.rs:337` | `spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));` | row fill after a `cell_width` measure |
| `tabs/backlog/detail/graph.rs:571` | `format!("{kind:<KIND_WIDTH$}")` | D6: link-kind enum (ASCII) |
| `tabs/backlog/detail/graph.rs:581` | `spans.push(Span::raw(" ".repeat(indent(row.level))));` | spaces: constant indent/gap, one cell each |
| `tabs/backlog/detail/prompt.rs:272` | `"{:<24}{:>8}{:>8}  {}",` | D6: ASCII header literals |
| `tabs/backlog/detail/prompt.rs:279` | `"{}{:>8}{:>8}  {}",` | D6: integer columns; the name is `cells::pad` |
| `tabs/backlog/detail/prompt.rs:307` | `format!("{label:<LABEL$}{value}")` | D6: `format!` width on a compile-time ASCII label/key |
| `tabs/backlog/detail/runs.rs:921` | `" ".repeat(width)` | `blank`: spaces (one cell each) |

Test-module hits per file: `cells.rs` 25, `overlay/concepts_search.rs` 4, `overlay/workspace_switcher.rs` 1, `tabs/backlog/detail/graph.rs` 3, `tabs/backlog/detail/requirements.rs` 2, `tabs/backlog/detail/runs.rs` 10, `tabs/backlog/detail/runs/execution_graph.rs` 1, `tabs/backlog/divergence.rs` 6, `tabs/backlog/filter.rs` 1, `tabs/backlog/item_form.rs` 6, `tabs/backlog/list.rs` 1, `tabs/chat/mod.rs` 3, `tabs/requirements/tree.rs` 6, `tabs/settings/agents.rs` 4, `tabs/settings/boxes.rs` 2, `tabs/settings/connection.rs` 1, `tabs/settings/hierarchy.rs` 4, `tabs/settings/kinds.rs` 1, `tabs/settings/mod.rs` 3, `tabs/settings/prompt.rs` 1, `tabs/settings/qdrant.rs` 1, `tabs/skills/attach.rs` 4, `tabs/skills/library.rs` 6, `tabs/skills/templates.rs` 6, `text_area.rs` 11, `text_field.rs` 5.

**`Line::width()` measures** (review L4: the regex above cannot see them; ratatui's `Line::width` is a whole-span measure that skips the halfwidth sound-mark rule). Production uses, each a deliberate estimate, not a layout budget:

| Site | Reason |
|---|---|
| `tabs/skills/templates.rs:971` | row estimate for a wrapped `Paragraph`: documented lower bound (plan Correction, V4) |
| `tabs/skills/library.rs:1472` | same lower-bound row estimate (skill body/diff pane) |
| `tabs/skills/library.rs:1652` | same lower-bound row estimate (info pane) |
| `tabs/backlog/detail/runs.rs:1427` | footer height estimate for a wrapped `Paragraph`; same lower-bound rule |
| `overlay/migration_prompt.rs:120` | box width over fixed prompt lines (ASCII text plus a pending count); no runtime string |

Found with `rg -n 'Line::width|\b(line|l|row)\.width\(\)' crates/htui/src/ui`; the remaining hits are tests and the switcher's doc comments (B3).

## Acceptance
- [x] T0–T5 complete, each lane red→green
- [x] Validation passes; no snapshot moved beyond OQ-1's accepted set
- [x] D11 sweep allowlist written and every remaining hit on it
- [x] Patterns mirrored, not reinvented (14 local helpers → `ui::cells`)
