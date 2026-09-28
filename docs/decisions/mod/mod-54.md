# MOD-54 - Wide characters and graphemes in the text widgets (done, 2026-09-28)

`R-TUI-1`, `R-NF-1`. Found 2026-09-26 by the MOD-7 milestone 2 review. Plan
`.claude/plans/mod-54-wide-chars-graphemes.plan.md`, blueprint
`.claude/plans/mod-54-wide-chars-graphemes.blueprint.md`.

## What shipped

`TextField` and `TextArea` no longer count width in `char`s or step their cursors by code point.
A CJK or emoji line occupies the columns it is drawn in, and `Left`/`Right`/`Backspace`/`Delete`
cannot split a combining sequence.

- **`crates/htui/src/ui/cells.rs`** (new, private). The single place a cell count is computed
  (`cell_width`) and text is split into clusters (`graphemes`, UAX #29 extended). One module so
  the two widgets cannot disagree with each other or with `ratatui`, which is what actually draws
  the result.
- **`unicode-width` 0.2 and `unicode-segmentation` 1.13 declared** at `[workspace.dependencies]`
  and in `crates/htui/Cargo.toml`. Both were already compiled into the binary as transitive
  dependencies of `ratatui-core` and `ratatui-widgets`, so nothing new is compiled.
- **`TextField`.** `cursor` is a grapheme index; `line()` windows the buffer in display cells.
  `len()` is a grapheme count, so the mask's dots and the `(n)` printed beside them are the same
  number.
- **`TextArea`.** The public cursor **stays a byte offset** — `set_cursor` is fed a byte by a
  `parse` error at `skills/templates.rs:395` and `:698` and that contract does not move. Only the
  stepping changed: `Left`/`Right`/`Backspace`/`Delete` move by cluster, `set_cursor` floors to a
  cluster start, `cursor_line_col` counts clusters, `drawn()` is cell-aware, `left` is a cell column
  re-derived on every draw, and `goal_col` is a cell column.

## Why

A CJK line overran its column, a wide grapheme could land half off-screen, and `Left`/`Backspace`
split a combining sequence. `ratatui`'s own buffer was already width- and grapheme-correct and
never overruns a `Rect`; the defect was entirely in the **widgets' arithmetic** disagreeing with
the renderer. That is why the fix is a shared helper rather than a dependency swap.

## The two rules the helper exists to enforce

Both were verified by compiling a probe against the real crates, not recalled:

1. **A cluster's width is the width of its own string**, never a sum of per-`char` widths. A
   family emoji is one cluster of five code points: summed per `char` it is 6, measured as a string
   it is 2. The per-`char` sum is *also* wrong for control characters — `UnicodeWidthChar::width('\u{1}')`
   is `None` where `UnicodeWidthStr::width("\u{1}")` is `1`, so the natural `unwrap_or(0)`
   undercounts every C0 control and `DEL` by one, and a body from `$EDITOR` can hold those.
2. **The non-CJK `width()`, never `width_cjk()`.** The `cjk` feature is default-on and `ratatui`
   does not disable it. `width_cjk()` doubles every East Asian Ambiguous character, including `…`,
   `•` and `U+FFFD` — three of the glyphs these widgets draw themselves.

`cell_width` also replicates `ratatui`'s one-cell-per-`U+FF9E`/`U+FF9F` adjustment, so the widget's
arithmetic and the renderer's agree by construction (`"あﾞ"` pins at 3).

## Commits

| Commit | What |
|---|---|
| `d1431a4` | the plan, with its fact-check (three passes, ~90 claims) |
| `6b4bcaa` | CONFIRM, and OQ-1/OQ-2/OQ-3 answered as their defaults |
| `2e3dee2` | the blueprint, including the correction to the plan's D4 |
| `46f9523` | T0 — the two declarations and `ui/cells.rs` |
| `2a7b068` | T1 — `TextField` |
| `b9c4564` | T2 — `TextArea` |
| `bbe438d` | T3 — the doc corrections and the corrected D19 greps |

## The plan had a real bug, found at implementation

D4 specified the window head as the **largest** `s` whose tail fits. That quantity is the width of
the *hidden head*, not the shown tail, so maximising it minimises what the user sees: masked
`0123456789ab` at width 8 rendered `…  (12)`, dropping the character under the cursor. The correct
rule is the **smallest** `s` whose shown tail leaves room for the ellipsis and the cursor cell. It
reduces to the old `start = cursor + 2 - budget` when every width is 1, which is why ASCII output is
unchanged. A second correction: `at_width > budget` must short-circuit to a styled space, not
saturate — the probe underflowed on exactly that fixture.

## Deviations from the plan, and why

- **Two files dropped from "Files to Change".** `src/templates.rs:33,283` and
  `skills/templates.rs:249,1203` were listed for rewording; all four say the `Debug` impl prints
  lengths and never the text, which is still exactly what happens. Rewording an accurate sentence
  to match a plan line is worse than leaving it, so they are untouched.
- **Two doc sites listed for correction were already correct** — see above.
- **The D19 greps were corrected twice.** They were written when the only hits for `unicode-width`
  were the two stale "not declared" sentences, and said "expect nothing". After this item the crate
  *is* declared and `ui/cells.rs` explains in its doc how it is used, so the bare-name grep now
  matches correct new text. The gates name the two stale sentences instead, and the
  no-per-`char`-sum check is scoped to the two widget files.
- **A DSN is not necessarily ASCII** (the plan's R-3 said it was). `Dsn::parse` stores the caller's
  text verbatim and `sqlx` percent-*decodes* the password, so a masked field can hold non-ASCII and
  the on-screen `(n)` can legitimately move. That is the correct outcome — the dots and the number
  stay the same number — and no pre-existing test covered it, so
  `a_masked_field_counts_graphemes_not_code_points` is the only coverage.
- **`R-NF-1` is argued, not gated.** `cargo check -p htui --target x86_64-pc-windows-msvc` exits 101
  in `ring`'s build script on this box (`cc-rs: failed to find tool "lib.exe"`) and never reaches an
  `htui` source line, so it proves nothing about this change. Both crates are `#![no_std]`,
  `build = false`, with no `[target.*]` and no `unsafe`. **MOD-16 carries the Windows
  verification.**
- **The reviewer and the architect ran inline.** The `code-architect` agent failed twice with
  `HTTP 402 Insufficient credits` on `claude-opus-5-5`, and the `model` parameter accepts only
  `sonnet | opus | haiku | fable`, so the session model cannot be selected for a subagent. The
  blueprint and the review checklist were therefore done on the session model, per the maintainer's
  instruction.

## Gates

`cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-features --all-targets -- -D warnings`
clean; `cargo test --workspace --all-features -- --test-threads=1` green across 82 test binaries.

**The item's hard criterion held: no snapshot moved.**
`git diff --exit-code -- 'crates/*/tests/snapshots' 'crates/htui/src/snapshots'` is empty over all
four directories (8 / 28 / 88 / 1 files). None of them ever held a wide char or a combining mark,
which is exactly why the change needed its own coverage and why the snapshot gate is the layer that
cannot be argued with.

`Cargo.lock` moved by exactly 2 insertions, both inside the `htui` block, with no new `[[package]]`
and no version bump; `cargo tree` shows a single `unicode-width v0.2.2` shared with `ratatui-core`.
Note the gate is **not** `git diff --exit-code Cargo.lock`: a new edge into an already-locked package
rewrites that package's dependency list, so the lock legitimately moves and a maintainer who
"fixed" it would break the build.

## Follow-up

**MOD-59** carries the rest of the display-width problem, which this item deliberately did not
touch: `ui/diff.rs`, `ui/top_bar.rs`, the chat transcript, and every hand-laid-out row under the
settings and backlog sections. `diff.rs` is the sharpest — a CJK diff hunk overrunning its pane —
and should be first. The boundary is recorded rather than crossed because the HANDOFF item named
exactly two widgets, and widening it to every renderer would have put nine files in one untestable
change.
