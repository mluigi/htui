# MOD-30 - The detail sub-tab strip overflows at the pinned width (done, 2026-09-24)

**Origin**: MOD-2, finding F-71 (found during MOD-2 T67, 2026-09-14). **Requirement**: `R-TUI-3`.
**Plan**: `.claude/plans/mod-30.plan.md`. **Commits**: `f882d2c` (red pin test), `1b4c6cf` (fix and
snapshots), `15d168f` (review fixes). Landed before MOD-4 milestone 6, per MOD-4 PRD D5, so the
Backlog snapshots that milestone re-records are re-recorded once.

## Problem

MOD-2 added a sixth Backlog detail sub-tab. At two spaces per title the strip,
` Body  Runs  Graph  Documents  Notes  Prompt `, is 45 columns, but the detail pane's inner area at
the harness's pinned 100×30 is 43 columns. `Prompt` rendered clipped as ` Promp`, and every
re-accepted snapshot carried the clip: 24 files, not the 21 the HANDOFF entry counted.

## Decision

The maintainer chose between two candidate fixes at the routing gate:

- **D1 — Single-space separators, on the detail strip only.** The strip is now
  ` Body Runs Graph Documents Notes Prompt `, 40 columns with 3 to spare. The Settings and
  top-level strips keep two spaces. Neither of them overflows, and changing them would re-record
  every snapshot without fixing anything. `DETAIL_PERCENT` 45 -> 47 was rejected: it fits with 0
  columns to spare, so a seventh sub-tab would break it again, and it narrows the backlog list.
- **D2 — The accent covers the title only.** Separators are unstyled spans.
- **D3 — The width is pinned by a test, not by a snapshot.**
  `backlog::tests::the_detail_strip_fits_the_detail_pane` measures `detail::strip_line(..).width()`
  against the inner area of `detail::frame_block`, the same border `render` draws. The pane comes
  from the same `panes()` split `render` uses, at `testkit::DEFAULT_SIZE`. A seventh sub-tab, a
  longer title, a `DETAIL_PERCENT` change or a new border padding now fails a named test instead of
  being accepted as a snapshot diff.

## Result

25 snapshots were re-accepted, and each one changes only its strip row. 24 of them carried the
clip. The 25th, `prompt_preview__preview_feat_1.snap`, is a wide render that never clipped but draws
the same strip.

`rust-reviewer` returned approve-with-fixes with no CRITICAL or HIGH findings. All of the following
were applied in `15d168f`:

- **M1**: the pin used a second copy of the block, so `frame_block` is now shared.
- **L2**: `strip_line` is `pub(crate)`.
- **L3**: the doc comments state the spacing rule instead of the 45/43 arithmetic.
- **L4**: `DEFAULT_SIZE` is `pub(crate)` and is used by the pin.

Skipped with the maintainer: `Vec::with_capacity` in `strip_line`, because this is not a hot path.
Kept: the red commit `f882d2c`, which compiles and passes clippy on its own.

Gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -D
warnings`, and `cargo test -p htui --all-features -- --test-threads=1` (473 passed, 0 failed,
3 ignored). No migration and no `.sqlx` change.
