# MOD-84 - Reflow item bodies before wrapping (done, 2026-10-04)

**Requirements:** `R-TUI-3`.
**Origin:** the TUI design review of 2026-10-04 (https://claude.ai/artifact/TxAriNUvRpTifJy8Wq6HeH finding 11).
**Artifacts:**
- plan [`.claude/plans/mod-84-body-reflow.plan.md`](../../../.claude/plans/mod-84-body-reflow.plan.md): D1-D8,
  amendments A-1-A-4 and B-2, verified-claims table;
- blueprint `.claude/plans/mod-84-body-reflow.blueprint.md`: rule table R0-R7, the T1-T3 test lists, reference
  renderings of FEAT-1 and ANA-1 at 43 columns, the accepted CommonMark deviations.

Decision numbers are local to MOD-84 (the MOD-31 convention).

Routed as **plan** (0 criteria fired). Run in a TOOL-7 sandbox (`hr/MOD-84`). The three tasks ran serially in one
lane, because each builds on the one before.

**Decisions (maintainer, 2026-10-04):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked (13 claims, none falsified);
- blueprint amendments A-1-A-4 accepted; B-2: table rows keep their backticks;
- review: LOW-1, LOW-3, LOW-5 and LOW-6 applied; LOW-2 recorded as an accepted deviation; LOW-4 deferred and
  carried below.

**Commits:**
- plan, blueprint, amendments: `ee892f75`, `0e9cfd8b`, `59aa6aa9`;
- T1 `cells::wrap_spans`, `expand_tabs` moved from notes: `26f5e22d`;
- T2 `ui::markdown`: `35d22344`;
- T3 Body pane: `9676afb7`; snapshots `7c97caf6`, `22cc89ee`;
- review fixes: `470b82e1` (LOW-1, LOW-3), `2a9010ae` (LOW-5, LOW-6), `0637b7cc` (LOW-2 in the blueprint).

---

## What was built

The Body sub-tab used to hand the raw body to `Paragraph::new(Text::raw(..)).wrap(..)`. Bodies are hard-wrapped
Markdown, so every source line ending short of the pane's width left a ragged row (`top bar and a` /
`backlog tab, ...`), and inline code showed its backticks. Now the pane draws `ui::markdown::lines(body, width,
theme)` with no `Wrap`.

### `ui::markdown` (D1-D4)

This is a line classifier, not a Markdown parser, and it adds no dependency. Emphasis, links and heading styles are
left to MOD-80 (theme) and MOD-82 (pane chrome). Rules R0-R7, applied per source line in order:
- **Fences** (`` ``` `` or `~~~`, up to 3 spaces of indent) are kept verbatim up to a matching close, or to the end of
  the body. A backtick opener holds no other backtick (A-4).
- **Blank lines** are kept, and they close the open block.
- **Rules** (`---`, `* * *`) are checked before list items (A-1, CommonMark 4.1).
- **List items** (`-`, `*`, `+`, `1.`, `1)`) keep their indent and marker. Their continuation lines join, and
  wrapped rows hang under the item's text, or start at column 0 when fewer than 10 cells would remain.
- **Headings, tables, quotes, and indented code with no block open** each stand alone.
- **Every other line** joins the open paragraph or item with one space, including a lazy continuation.
- **Hard breaks**: a line ending in two spaces breaks the row, and so does a `\` when a continuation follows. A `\`
  at a block's end stays as written (LOW-1).
- **Inline code**: CommonMark backtick-run matching, parsed after joining. It is drawn in `theme.accent` without its
  backticks. Fenced code, indented code and table rows keep their backticks (B-2), and an unmatched run stays
  literal.

`row_count(body, width)` equals `lines(..).len()` at every width, because styles never move a break. Width 0
(before the first render) does not wrap.

### `cells` (D5, D6)

`wrap_spans` is the span-aware twin of `cells::wrap`: same break rules, words split at spaces before controls are
flattened, adjacent same-style pieces merged into one span. A parity test checks it against `wrap` over `wrap`'s own
cases at widths 0-12. `expand_tabs` and `TAB_STOP` moved from `notes.rs`, so fenced code draws a tab as Notes and
`TextArea` do.

### Body pane (D7)

`BodyTab` keeps `drawn: Cell<u16>`, the body area's width at the last render. `len()` counts the rows at that width,
so `J` and `PageDown` reach the body's last row in a narrow pane, and a pane that widened clamps to its last row. This
is the Notes pane's pattern (MOD-13 review L1). Tests:
- `the_body_reads_as_paragraphs_at_the_detail_width`
- `page_down_reaches_the_last_row_in_a_narrow_pane`
- `a_pane_that_widened_clamps_to_its_last_row`
- `before_the_first_render_the_scroll_counts_unwrapped_rows`

### Snapshots

Eleven snapshots changed, and in each only body rows moved (columns 56-98, below the head block):
- `backlog__detail_body` (FEAT-1);
- five that show ANA-1 (`backlog__filter_form`, `filtered_list`, `list_grouped`, `shell__after_switch`,
  `waiting__platform_mixed`);
- five `concepts_search__*`, which show ANA-1's last body rows below the overlay. The blueprint's grep missed these,
  because the overlay hides the head line; the full serial gate found them.

The seven snapshots that show `No body for this item.` are unchanged.

### Review

`rust-reviewer`: approve-with-fixes, no CRITICAL or HIGH findings.
- **LOW-1** (fixed): a trailing `\` on a block's last line was dropped (`See C:\` drew `See C:`).
- **LOW-3** (fixed): `\t---` with nothing open read as a rule instead of indented code.
- **LOW-5** (fixed): `body.rs`'s module doc now says code and table rows keep their backticks, and `cells`'s module
  doc no longer says `wrap_spans` cuts at `ELLIPSIS`.
- **LOW-6** (fixed): tests for a wide character in an item's hanging rows, a tab-led marker, and inline-code styles
  in the column-0 item path.
- **LOW-2** (accepted deviation, recorded in the blueprint): inside a list item, text after a fence or a blank line
  starts at column 0.

## Gates

`cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
`cargo test -p htui --all-features --no-fail-fast -- --test-threads=1`: before the review fixes, 2303 passed and
0 failed (implementer, then re-run on the session); after them, 2308 passed, 0 failed, 3 ignored, no snapshot moved. No migration, no `.sqlx`, no new
dependency.

## Carried

Accepted with the maintainer, recorded here only:
- **LOW-4**: the shared `Scroll::on_key` (`detail/mod.rs`) applies the step before clamping. After a pane widens,
  the first `K` press goes from the stale offset to `min(offset - 1, last)` and does not move the view. The Notes pane
  has the same behaviour. The fix is to clamp first (`self.offset.min(max)`) and then step, and since every
  scrolling pane shares `Scroll`, it belongs to a later item that touches it.
- The Docs, Notes and Reqs panes keep their own drawing (D8). The Prompt sub-tab must stay verbatim.
