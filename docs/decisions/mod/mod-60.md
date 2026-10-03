# MOD-60 - Display width in every hand-laid-out row (done, 2026-10-03)

`R-TUI-1`, `R-NF-1`. From MOD-54 (plan D18), plus the MOD-13 milestone 3 review L3 deferral (the
divergence view's notice, clip and column padding) and the overlays. Plan
`.claude/plans/mod-60-display-width.plan.md` (decisions D1-D12, the sweep allowlist), blueprint
`.claude/plans/mod-60-display-width.blueprint.md` (deviations B1-B13).

## What shipped

Every row the TUI lays out by hand now measures, pads, clips and wraps in terminal **cells**, the
way ratatui draws them, not in `char`s. A CJK, emoji, combining-mark or halfwidth-katakana string
no longer overruns its pane, pushes a column out of line, or loses its `…`.

- **`ui::cells` is the one place text is fitted** (D1). Next to MOD-54's `cell_width` and
  `graphemes` it now has `clip` (cut so the text plus `…` fits; `""` at width 0; a wide cluster that
  would straddle the edge is dropped), `pad`/`pad_left`, `fit` (clip, then pad to exactly the
  width), `wrap` (break at a space, inside a word by grapheme only when the word alone is wider,
  leading spaces kept), `clip_spans` (styled, cut from the end), `flatten` and `ELLIPSIS`. All of
  them draw a control character as one blank cell (D3, extended to `pad`/`pad_left` by the review).
- **About 16 local copies deleted**: `list::pad`/`clip`, `tree::padded`, `detail::padded`,
  `forms::padded`, `attach::cut`, `reqs::cut`, `runs::fit`/`fit_line`/`wrap_line`/`CUT`,
  `divergence::wrap_row`, `item_form::notice_lines`, `chat::clip`/`columns`, `boxes::clip`,
  `agents::fit_label`, and the per-`char` clips in the graph and execution-graph panes.
- **`settings::wrapped`** keeps its signature and its 5 outside importers, measures in cells, and
  hard-breaks a word wider than the line by grapheme (OQ-1, answered yes at CONFIRM). A CJK sentence
  has no spaces; without the break it stayed one overlong row and the notice height was wrong. The
  probe (V10) showed the break moves no existing snapshot or test.
- **Requirements tree header** (D7, B11): on a narrow pane the project **name** is elided and
  ` · read-only` kept; the marker goes only when no glyph of the name would remain. It used to cut
  the marker first and, past 12 cells over, let ratatui chop the name with no ellipsis (MOD-39
  "Carried").
- **Divergence view** (D8, B2; MOD-13 L3): the three value columns are wrapped and fitted in cells,
  so every row is exactly `22 + 3·vw` cells whatever the values hold.
- **Converted sites**: every settings section (hint checks, label columns, `fit_label`, box host
  labels, the delete-confirm prompt, the tool-path column), the backlog list/filter/item form, the
  Reqs, Runs and prompt detail panes, the requirements detail and forms, the skills
  attach/library/templates rows and prompts, the workspace switcher (column and box width, B3), the
  promoted chat header (D9, B12) and the path picker's go-to prompt.

## Why, and the decisions worth keeping

- **B10 — `cell_width` is a per-grapheme sum.** `unicode-width` applied over a whole string uses
  ligature and ZWJ rules across cluster boundaries (Arabic lam-alef measures 1), while ratatui's
  `Buffer::set_stringn` advances one grapheme at a time (lam-alef draws 2). Summing per cluster
  makes the measure equal the renderer by construction. Single-cluster results are unchanged; MOD-54's
  `TextField` window measure did move, as a fix, pinned by
  `a_ligature_before_the_cursor_does_not_widen_the_line`. Found by T0's adversarial verifier, not by
  the plan or the blueprint.
- **Correction to the item text.** The three files HANDOFF listed first — `ui/diff.rs`,
  `ui/top_bar.rs`, `ui/tabs/chat/transcript.rs` — have no char-counted layout: ratatui clips them by
  cell, `top_bar.rs` has no clock, and the transcript selects whole `Line`s. None was touched, and
  the chat-snapshot risk the item warned about did not arise. The Templates/Library diff row count
  (`Line::width().div_ceil`) is a documented lower bound, the same for ASCII word wrap; it is
  listed, not changed.
- **The audit is checkable** (D11): a fixed `rg` over `crates/htui/src/ui` for `chars().count()`,
  `chars().take(`, `.repeat(` and `format!` width specifiers. At close it returns 155 hits — 37 in
  production code, each listed with its reason in the plan's "Allowlist" (constant spaces, fills
  after a cell measure, UUID/digest prefixes, `Debug` lengths, `TextArea::len`, ASCII-only labels per
  D6), and 118 in test modules. Five `Line::width()` row estimates, which the regex cannot see, are
  listed separately (review L4).
- **No snapshot moved.** All 135 snapshots are one cell per char (V9); the only intended ASCII
  changes are width 0 (`""` instead of a lone `…`, one cell over), `"\r\n"` drawn as one blank cell,
  and the overlong-word break.

Out of scope, recorded: soft wrap, bidi, terminals that draw CJK as one cell, the composer's missing
horizontal scroll (an ASCII defect too), and `concepts_search`'s unmarked `clip`/`wrap` and
`path_picker`'s own walks (already cell-correct, MOD-54).

## How it was built

Plan path with ultracode for the implement phase. T0 (`ui::cells`, `settings::wrapped`) on the
branch, then four lanes in parallel git worktrees — T1 settings, T2 backlog, T3 requirements tab, T4
skills/overlays/chat — each as implement (tests first) → conformance and adversarial verify →
repair, then T5 (delete `list::clip`, lift the temporary `dead_code` allows, the sweep). Every
round's verifiers found something the green gate did not: B10, a header that kept the marker and
drew no name (B11), a doubled `……` in the chat header (B12), a control character shifting the
switcher's count column, and several tests that still passed with their fix reverted. rust-reviewer:
approve-with-fixes (1 MEDIUM, 4 LOW, 3 NIT), all applied — `wrap` keeps a running sum (4-9 ms → ~1 ms
on a 60 KB document), `pad`/`pad_left` flatten controls, the graph and execution-graph clips became
`cells::clip`, and the review round's own verifiers caught an unflattened graph root key.

## Commits

- Plan and blueprint: `dd47231e`, `da8320ce`, `d41581fb` (B10), `236e5edd` (allowlist, B11-B12),
  `981f7d65` (allowlist re-derived, status).
- T0: `33046f2b`, `5801808c`, `5e54774f`, `e760d7aa`, `932ed356`.
- T1 settings: `6bd1adad`, `b799ca1d`, `dca7c865`, `9a96fd30`; merged `30c4a4cc`.
- T2 backlog: `4a43fd39`, `85fa7679`, `2fdff222`; merged `f92a9b76`.
- T3 requirements: `3d76b4b9`, `678ddda9`, `b8edb93b`, `c3eade14`, `f487d354`; merged `1c3759b8`.
- T4 skills/overlays/chat: `1e5a7918`, `332715ac`, `c0e33bd8`, `64b10a00`, `aaf84e55`, `a71176b2`,
  `b2dc9eee`, `90761b9c`; merged `326b74a0`.
- T5: `444d5dc0`.
- Review fixes: `80173bb0`, `c471d781`, `9665664c`, `ea0455e7`, `27a62567`, `9b58ebe0`.

28 files under `crates/`, all in `crates/htui/src/ui/`. No migration, no new dependency, no
`Cargo.lock` change, no store surface, no new snapshot.
