# Plan: MOD-84 reflow item bodies before wrapping

**Source**: HANDOFF `MOD-84` (from the TUI design review of 2026-10-04, finding 11; `R-TUI-3`)
**Routed**: plan path via `/handoff-run` (0 criteria fired), accepted by the maintainer 2026-10-04. Sandbox run
`hr/MOD-84`.
**Complexity**: Small (one new pure module, one span-aware wrap beside `cells::wrap`, the Body pane's render and
scroll clamp, snapshot updates)
**Status**: done 2026-10-04 (`docs/decisions/mod/mod-84.md`)

## Summary

The Body sub-tab hands the raw body to `Paragraph::new(Text::raw(..)).wrap(..)` (`detail/body.rs` `render`). Bodies
are hard-wrapped Markdown, so every source line ending short of the pane's width leaves a ragged row:
`top bar and a` / `backlog tab, ...` in `backlog__detail_body.snap`, and the inline code shows its backticks. The fix
classifies the body's lines into blocks, joins soft-wrapped lines inside a paragraph or list item, and wraps the
result itself at the pane's width. It keeps blank lines, list markers, fenced blocks and other structural lines as
written, and draws inline code in a style instead of between backticks. Because the pane now wraps the body itself,
the scroll clamp counts the rows on screen, as the Notes pane does (MOD-13 review L1).

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: a line classifier, not a Markdown parser, and no new dependency.** A new private module
  `crates/htui/src/ui/markdown.rs` (`mod markdown;` in `ui/mod.rs`, beside `mod cells;`) holds pure functions over
  `&str`. A CommonMark crate (`pulldown-cmark`, `comrak`) is not in `Cargo.lock`, and it would also render emphasis,
  links and headings, which this item does not ask for. Those belong with MOD-80 (theme) and MOD-82 (pane chrome).
- **D2: line classes**, in this order, per source line:
  1. **Fence**: up to 3 spaces, then 3 or more `` ` `` or `~`. Every line is kept verbatim up to a closing run of the
     same character at least as long. An unclosed fence runs to the end of the body. The fence lines themselves are
     shown as written.
  2. **Blank**: whitespace only. Kept as one empty row, and it closes the open paragraph or list item.
  3. **List item**: an optional indent, then `-`, `*`, `+`, or 1 to 9 digits followed by `.` or `)`, then a space or
     the end of the line. It opens a new item and closes the previous block. The indent and the marker are kept as
     written.
  4. **Verbatim line**: an ATX heading (1 to 6 `#`, then a space or the end of the line), a rule or setext underline
     (3 or more of only `-`, `*`, `_` or `=`, spaces allowed), a table row (trimmed, starts with `|`), a block quote
     (trimmed, starts with `>`), or, when no paragraph or list item is open, a line indented 4 or more spaces or by a
     tab (indented code). It stands alone: it closes the open block, and the next text line starts a new paragraph.
  5. **Text**: anything else. With a paragraph or list item open, it is a continuation and is joined to it with one
     space, leading and trailing whitespace trimmed. Otherwise it opens a paragraph. A non-indented line after a
     list item joins the item (CommonMark's lazy continuation).
  A line ending in two or more spaces or in `\` is a hard break. The block continues on a new row; in a list item, the
  new row hangs under the item's text. The `\` is dropped.
- **D3: drawing at width `w`.**
  - Paragraphs and verbatim lines are wrapped by the new `cells::wrap_spans` (D5).
  - A list item's first row is the indent, the marker and a space, followed by the text. Continuation rows hang under
    the text's first cell. When that leaves fewer than 10 cells, continuation rows start at column 0 instead.
  - Fenced and indented-code lines are not joined. Each is tab-expanded (D6) and wrapped on its own, and inline code
    is not parsed inside them.
  - Width 0 (before the first render) means no wrap: one row per block row. This mirrors `notes.rs` `wrap_width`.
- **D4: inline code.** A run of n backticks opens a code span, which closes at the next run of exactly n. The
  backticks are dropped, and when the content both starts and ends with a space, one is stripped from each end
  (CommonMark). A run with no closing match stays literal. Code spans are parsed after joining, so a span cut by a
  soft wrap in the source still pairs up. Style: `theme.accent`. `Theme` gains no field (`theme.rs` has six styles,
  none for code); a dedicated code colour is MOD-80's palette.
- **D5: `cells::wrap_spans(spans: &[(Cow<str>, Style)], width) -> Vec<Vec<Span<'static>>>`.** This is the
  span-aware twin of `cells::wrap`, with the same rules: break at U+0020, split by grapheme only when a word alone is
  wider than the width, keep leading spaces, flatten controls. A word may cross a style boundary. Its invariant test:
  with one style, the rows' text equals `cells::wrap`'s rows.
- **D6: `expand_tabs` and `TAB_STOP` move from `notes.rs` to `cells.rs`** (`pub(crate)`). Notes calls
  `cells::expand_tabs`, so tabs in fenced code draw as the Notes pane and `TextArea` draw them.
- **D7: Body pane wiring (mirrors `notes.rs`).** `BodyTab` gains `drawn: Cell<u16>`, the body area's width at the
  last render.
  - `len()` becomes `markdown::row_count(&item.body, width)`.
  - `render` sets `drawn`, builds `markdown::lines(&item.body, body_area.width, ctx.theme)`, and draws them with
    `Paragraph::new(..)` with no `Wrap`. The scroll is clamped to the last row, because a pane that widened since the
    last key has fewer rows (`notes.rs` `render`).
  - The head block is unchanged.
  - The module doc's "rendered raw, not parsed" sentence is replaced by a summary of D1 to D4.
- **D8: scope is the Body pane.** The Prompt sub-tab shows the assembled prompt as it is sent, which must stay
  verbatim. The Docs, Notes and Reqs panes keep their own drawing. A follow-up is filed only if the maintainer wants
  the same reflow there.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Wrap at last width | `crates/htui/src/ui/tabs/backlog/detail/notes.rs` `wrap_width`, `lines`, `row_count`, `drawn: Cell<(u16, u16)>`, `render` (clamp `min(last)`) | Rows counted at the last render's width; 0 means unwrapped |
| Cell-accurate wrap | `crates/htui/src/ui/cells.rs` `wrap`, `clip_spans`, `graphemes`, `cell_width`, `flatten` | Grapheme- and cell-correct; controls flattened; spans keep their style |
| Pure helpers + unit tests | `crates/htui/src/ui/cells.rs` `#[cfg(test)] mod tests` | Table-like cases, one assertion message per case |
| Pane tests | `notes.rs` tests via `compose::bench::{Shell, drawn, key}` | Render at small width with `drawn(w, h, ..)`, `PageDown` loop, assert on text |
| Snapshots | `crates/htui/tests/backlog.rs` `the_six_sub_tabs_render_the_selected_item` (`detail_body`) | `insta::assert_snapshot!` at 100x30 |
| Errors | n/a | Pure rendering; nothing fails |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/cells.rs` | UPDATE | `wrap_spans` (D5); `expand_tabs`/`TAB_STOP` moved in (D6); tests |
| `crates/htui/src/ui/tabs/backlog/detail/notes.rs` | UPDATE | Use `cells::expand_tabs` (D6); no behaviour change |
| `crates/htui/src/ui/markdown.rs` | CREATE | Blocks (D2), inline code (D4), `lines`/`row_count` (D3); tests |
| `crates/htui/src/ui/mod.rs` | UPDATE | `mod markdown;` |
| `crates/htui/src/ui/tabs/backlog/detail/body.rs` | UPDATE | D7 wiring, module doc, pane tests |
| `crates/htui/tests/snapshots/*.snap` (up to 12 that show the Body pane) | UPDATE | Reflowed bodies, reviewed per file |
| `docs/decisions/mod/mod-84.md`, HANDOFF, DECISIONS index | CREATE/UPDATE | Close-out per `lifecycle.md` P2 |

## Tasks

The tasks run serially in one lane: T2 uses T1's functions, and T3 uses T2's. Nothing is marked independent.

### Task 1: `cells::wrap_spans` and `expand_tabs` (D5, D6)
- **Action**: Tests first: one-style parity with `wrap` over its existing cases; a code word crossing a style
  boundary stays on one row; an overlong word splits by grapheme; width 0 reads as 1. Then implement. Move
  `expand_tabs`/`TAB_STOP` and keep notes' tab test green.
- **Mirror**: `cells::wrap`, `cells::clip_spans`.
- **Validate**: `cargo test -p htui --lib ui::cells`, `cargo test -p htui --lib notes`.

### Task 2: `ui/markdown.rs` (D1 to D4)
- **Action**: Tests first, each over a small body:
  - a hard-wrapped paragraph joins;
  - a blank line is kept;
  - a `Shape` line followed by `- item` stays two blocks;
  - a list item's indented continuation joins and hangs under the text;
  - nested and ordered markers are kept;
  - a fenced block (with a tab and a backtick inside) is verbatim, and an unclosed fence runs to the end;
  - a heading, a table row, a quote and a rule each stand alone;
  - two-space and `\` hard breaks;
  - inline code without backticks in `accent`, and a code span joined across a source line break;
  - an unmatched backtick stays literal;
  - a double-backtick span holding a backtick works;
  - `row_count(b, w) == lines(b, w, t).len()` for the FEAT-1 fixture body at several widths;
  - width 0 does not wrap.
- **Mirror**: `cells.rs` test style.
- **Validate**: `cargo test -p htui --lib ui::markdown`.

### Task 3: Body pane (D7) and snapshots
- **Action**: Tests first, in `body.rs` with `compose::bench`:
  - FEAT-1 at width 43 shows `a top bar and a backlog tab` with no ragged break, and no backtick appears;
  - in a narrow pane, `PageDown` reaches the body's last line (`The terminal is restored`), mirroring notes' review
    L1 test.
  Then wire `render` and `len`. Run `cargo insta test -p htui --features testkit`, review every changed snapshot
  (only body rows may move; head, list and chrome rows identical), and accept.
- **Mirror**: `notes.rs` `render`, `the_newest_note_of_a_long_thread_can_be_scrolled_to`.
- **Validate**: `cargo test -p htui --features testkit --test backlog`, then the full gate below.

### Task 4: docs and close-out
- **Action**: `docs/decisions/mod/mod-84.md` write-up; HANDOFF check and summary recount; DECISIONS index; plan
  status; validator.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A body relies on line breaks this classifier joins (e.g. a poem, an address) | Low | Hard breaks (two spaces, `\`) and fences keep them. This matches how every Markdown renderer treats it |
| Snapshot churn collides with MOD-81/82/83, which share these snapshots | Medium | Only body rows move; a merge re-accepts the snapshots after review |
| Lazy continuation joins a line the author meant as its own paragraph | Low | CommonMark behaves the same; a blank line separates |
| `accent` for code reads as "selected" | Low | Code is not underlined or reversed; MOD-80 can give it its own palette slot |
| Scroll clamp before the first render | Low | Width 0 counts unwrapped rows, the old lower bound (notes' rule) |

## Amendments after the blueprint (accepted by the maintainer 2026-10-04)

The blueprint (`mod-84-body-reflow.blueprint.md`) found these; the blueprint is authoritative where it differs.

- **A-1**: the rule check (R3) runs before the list-item check (R4), per CommonMark 4.1. Otherwise `* * *` would open a
  list item, and `a\n* * *\nb` would draw as `a` / `* * * b`.
- **A-2**: the T3 width-43 test asserts the row `top bar and a backlog tab, all reading`. `a top bar and a backlog
  tab` does not fit on one row at that width.
- **A-3**: 13 snapshots carry the head meta line, not 12. Six show a body (FEAT-1 in `backlog__detail_body`, ANA-1 in
  five), and only those may change. The other seven show `No body for this item.` and must not change.
- **A-4**: a backtick fence's opening line holds no further backtick (CommonMark), so a paragraph line like
  `` ```x``` `` does not open a fence.
- **B-2**: table rows keep their backticks and are only tab-expanded, so their columns stay aligned. Headings, rules
  and quotes do get inline code. This is the maintainer's choice.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Body pane renders the raw body with `Text::raw` + `Wrap` | true | `detail/body.rs` `render`: `Paragraph::new(Text::raw(item.body.as_str())).wrap(Wrap { trim: false })` |
| The scroll clamps against source lines, not drawn rows | true | `body.rs` `len` = `item.body.lines().count()`; `detail/mod.rs` `Scroll::on_key` caps at `len - 1` |
| The defect is in the committed snapshot | true | `backlog__detail_body.snap`: `top bar and a` / `backlog tab, ...`, `` `htui-core` `` |
| FEAT-1's fixture has inline code, a label line directly above a list, indented continuations, no fence | true | `htui-core/src/fixtures.rs` `FEAT_1_BODY` (`Shape` then `- Two crates`, two-space continuations) |
| ratatui's `Paragraph::line_count` is unavailable without an unstable feature | true | `ratatui-widgets-0.3.2/src/paragraph.rs:329-332` `instability` `rendered-line-info`; workspace `ratatui = "0.30.2"` with no feature list, no `unstable-rendered-line-info` in `Cargo.toml` |
| No Markdown or wrap crate is in the lock file | true | `grep -cE '^name = "(pulldown-cmark\|comrak\|textwrap\|tui-markdown)\b' Cargo.lock` = 0 |
| The Notes pane already wraps at the last width and clamps on drawn rows | true | `notes.rs` `wrap_width`, `lines`, `row_count`, `drawn`, `render` (`min(last)`) |
| `cells::wrap` and `clip_spans` exist, `pub(crate)`, cell-accurate | true | `ui/cells.rs:185`, `:234` |
| `Theme` has no code style | true | `ui/theme.rs`: `base dim title accent selected error` |
| `expand_tabs`/`TAB_STOP` are private to `notes.rs` | true | `notes.rs:80-84` |
| `compose::bench` is reachable from `body.rs` tests | true | `compose.rs:637` `pub(super) mod bench`; `notes.rs` tests import it as a sibling |
| `ui/cells` is a private module, so a private `ui/markdown` is reachable from `ui::tabs` | true | `ui/mod.rs:4` `mod cells;` used by `detail/notes.rs` as `crate::ui::cells` |
| 12 snapshots show the Body pane's head meta line | true | `grep -rln "· priority [0-9] · version" crates --include=*.snap` = 12 files (backlog, integration, shell, waiting) |
| Task independence | n/a | Serial lane by design (T2 uses T1, T3 uses T2); no parallel marking to check |

## Acceptance

- [ ] FEAT-1's body reads as paragraphs and list items at any pane width; no ragged row from a source break
- [ ] Blank lines, list markers, fences and structural lines kept; inline code styled without backticks
- [ ] `J`/`PageDown` reach the body's last row in a narrow pane
- [ ] Every changed snapshot reviewed: only body rows moved
- [ ] Validation passes; reviewer gate (`rust-reviewer`) run
