# Plan: MOD-30 — the detail sub-tab strip overflows at the pinned width

**Source**: `HANDOFF.md` MOD-30 (from MOD-2, finding F-71). **Requirement**: `R-TUI-3`.
**Routing**: PRD path by the criteria count (C3 + C4, exactly at threshold); **routed as plan by
maintainer override** (2026-09-24), with the open fix choice answered at the routing gate.
**Why now**: MOD-4 PRD D5 — MOD-30 lands before MOD-4 milestone 6, which re-records the Backlog
snapshots; landing it first keeps that re-record to one pass.

## Decisions

- **D1 — Single-space separators, detail strip only.** `detail/mod.rs::render_strip` draws
  ` Body Runs Graph Documents Notes Prompt ` (one leading space, one space between titles, one
  trailing space): **40** columns against the **43**-column inner pane, 3 spare. The Settings strip
  (`settings/mod.rs:381`) and the top-level strip (`registry.rs:175`) keep their double spacing:
  neither overflows, and changing them would re-record every snapshot for no fix.
  `DETAIL_PERCENT` 45 -> 47 was rejected: it fits with 0 spare, so a seventh sub-tab breaks it
  again, and it narrows the backlog list.
- **D2 — The active accent covers the title only.** Separator spaces are unstyled spans. Before,
  the accent covered ` title `; snapshots are text-only, so this changes no snapshot beyond D1.
- **D3 — The width is pinned by a test, not by a snapshot.** A `strip_line(registry, theme) ->
  Line` helper builds the strip; `render_strip` renders it. A unit test in `backlog/mod.rs` asserts
  `strip_line(..).width()` fits inside the detail pane's inner width at the harness's pinned
  100×30. The pane is computed through the same split `render` uses (a `panes(area)` helper
  extracted from `BacklogTab::render`) over `layout::chrome`, so a seventh sub-tab, a longer title
  or a `DETAIL_PERCENT` change fails a named test instead of being re-accepted as a snapshot diff.
  **Amended at review (M1, L4):** the test reads the pane's inner area through
  `detail::frame_block`, the border `render` itself draws, and the size from
  `testkit::DEFAULT_SIZE`, now `pub(crate)`, so a harness resize moves the pin with it.

## Review

`rust-reviewer`: approve-with-fixes, no CRITICAL/HIGH. Applied: M1 (shared `frame_block`), L2
(`strip_line` is `pub(crate)`), L3 (doc comments describe the rule, not the 45/43 arithmetic), L4
(`DEFAULT_SIZE` shared). Skipped with the maintainer: `Vec::with_capacity` (not a hot path). Kept:
the red commit `f882d2c` (compiles and passes clippy; repo convention).

## Files

| File | Change |
|---|---|
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | `strip_line` added; `render_strip` uses it; doc comment `:220` updated |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | `panes(area)` extracted from `render`; `#[cfg(test)] mod tests` with the width pin |
| `crates/htui/tests/snapshots/*.snap` (24) and `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap` (1) | re-accepted: strip row only. 24 carried the clip; the 25th, `prompt_preview__preview_feat_1.snap`, renders wider than 100 columns and never clipped, but draws the same strip |

## Tasks (serial; one task, too small to split)

1. **Red**: add the width-pin test in `backlog/mod.rs` (with `panes` and `strip_line` extracted,
   behaviour unchanged). It fails: 45 > 43. Commit the red test only if it compiles and passes
   clippy (repo convention); otherwise land with task 2.
2. **Green**: single-space separators in `strip_line`. Pin test passes.
3. **Snapshots**: `cargo insta test -p htui --accept` limited to the strip diff. Review each diff:
   the only changed line per snapshot is the strip row (plus nothing else). Commit.

## Validation

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui -- --test-threads=1
```

`--test-threads=1` per the repo's keyring-fake constraint. No migration, no `.sqlx` change.

## Acceptance

- The strip renders ` Body Runs Graph Documents Notes Prompt ` whole at 100×30; no snapshot
  contains ` Promp│`.
- The pin test fails if the strip exceeds the pane (checked by temporarily registering a seventh
  sub-tab title locally, not committed).
- Settings and top-level strip snapshots unchanged.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Strip is 45 columns today | confirmed | `sum(len(t)+2)` over the six titles = 45 (python probe) |
| Single-space strip is 40 columns | confirmed | `1 + len(" ".join(titles)) + 1` = 40 |
| Inner pane is 43 at 100×30 | confirmed | `shell_empty.snap`: `│ Body  Runs  Graph  Documents  Notes  Promp│` = 43 between borders |
| `DETAIL_PERCENT = 45` at `backlog/mod.rs:30` | confirmed | grep |
| `render_strip` exists in detail, settings, registry | confirmed | `detail/mod.rs:222`, `settings/mod.rs:381`, `registry.rs:175` |
| "all 21 re-accepted snapshots carry the clip" (HANDOFF) | **falsified — 24** | 23 in `crates/htui/tests/snapshots/`, 1 in `crates/htui/src/snapshots/` (`grep -l ' Promp│'`) |
| Harness pinned at 100×30 | confirmed | `testkit.rs:34` `DEFAULT_SIZE: (u16, u16) = (100, 30)` (private) |
| Body region spans full frame width | confirmed | `layout.rs::chrome` splits vertically only |
| `Line::width()` exists in ratatui 0.30 | confirmed | `ratatui-core/src/text/line.rs:441` |
| `Theme: Default` | confirmed | `theme.rs:26` |
| No test asserts the double-spaced detail strip text | confirmed | grep over `crates/htui/src` and `crates/htui/tests/*.rs`: only `prompt_settings.rs:789`, the Settings strip, untouched by D1 |
| `backlog/mod.rs` has no test module yet | confirmed | no `cfg(test)` in the file |
| Task independence | n/a | single serial task list |
| (post-implementation) 24 snapshots change | **amended — 25** | `prompt_preview__preview_feat_1.snap` is a wide render: unclipped before, re-spaced after. Every diff line is the strip row (`git diff -U0 -- '*.snap'`) |
