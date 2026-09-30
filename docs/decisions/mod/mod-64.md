# MOD-64 - Concepts search in the TUI (done, 2026-09-30)

**Requirements:** `R-STO-8`, `R-TUI-2`, `R-NF-3`.
**Origin:** MOD-50 (`docs/decisions/mod/mod-50.md`), minted at the maintainer's request. Until
now the concepts index was reachable only from the command line (`htui --index-items`,
`htui --search-items [--decisions]`).
**Artifacts:** plan [`.claude/plans/mod-64-concepts-search.plan.md`](../../../.claude/plans/mod-64-concepts-search.plan.md)
(routed plan 2026-09-29: C3 fired, C4 at the threshold; decisions D230–D240, OQ-1..OQ-4 confirmed
with their defaults) and blueprint
[`.claude/plans/mod-64-concepts-search.blueprint.md`](../../../.claude/plans/mod-64-concepts-search.blueprint.md)
(findings F1–F13, decisions D241–D262). Run in a TOOL-7 sandbox on `hr/MOD-64`.
**Commits:** plan and blueprint `47ca5dd`, `9a1cf27`, `ac9cae5`, `5c12aa9`; T1 `191a7ec`,
`a4c84bf`; T3 `e6be155`, `7ec185c`, `f7f2657`, `a03b13a`, `7b10870` (merged `ba6cb48`); T2
`f4b68a1`, `4b7e946`; T4 `02c8567`, `e42ca29`, `06e5ed5`; review fixes `84374ef`, `229e84b`,
`1c978ba`, `0f71736`, `abe71a1`, `88d097f`, `033579e`, `d8abc4b`, `6cf2696`; plus the close-out
commit.

## What shipped

- **`Ctrl+F` opens a concepts search overlay from every tab** (D236; help text `find`, amended by
  the maintainer at blueprint F1 so the status row still fits 100 columns). The overlay is modal:
  typed text never reaches a global key. The Chat tab now passes `Ctrl`/`Alt` chords to the shell
  before its composer sees them (F7), so `Ctrl+F` works with the composer open.
- **The overlay** (`ui/overlay/concepts_search.rs`): a query field, a header naming the project
  scope and the decisions flag, the hits printed with `concepts::format_hit` verbatim and clipped by
  display width (`ui::cells`), and one status row. `Enter` searches when the query or a toggle
  changed, otherwise opens the highlighted hit (D234, OQ-4). `Up`/`Down` move, `Ctrl+D` toggles
  decisions, `Ctrl+P` cycles *all projects of the workspace → each project* (D233, OQ-2), `Ctrl+R`
  re-indexes the current scope and prints the `--index-items` report line (D237, OQ-1), `Esc`
  closes. The first search of each overlay says `loading the embedding model…` (F10). An error
  stays inside the box and, when there are no hits, wraps across the empty hit rows.
- **Off the UI thread and off the store worker's serial loop** (D231). `SearchConcepts` and
  `IndexConcepts` are served by a new `ConceptsRuntime` (`concepts_worker.rs`), preview-style: one
  spawned task per request, the superseded one of the same origin and kind aborted, the task
  sending its own reply. A search never waits for an index run.
- **Errors never reach the status line** (D232). The reply is `StoreReply::Concepts(Box<ConceptsReply>)`
  with the error inside each answer kind; `StoreReply::Failed` is never used, because the shell
  turns every fresh `Failed` into a status-line error.
- **The index seam** (D230): an object-safe `ConceptIndex` trait. `QdrantIndex` reads the keyring
  per request on the blocking pool, loads `FastEmbedder` once through a single shared in-flight load
  (a failure is not cached; the next search retries once), and reuses its Qdrant connection while
  the stored URL and key are unchanged, dropping it after any failed call (D238 as amended at
  review). `MemIndex` wraps `htui-store`'s `MemVectorStore` for tests; `testkit` now enables
  `htui-store/test-support`.
- **Reveal** (D235). `Enter` on an item or document hit selects the item in the Backlog; on a
  requirement hit, the Requirements tab opens it. The shell routes `Action::Reveal(RevealTarget)` to
  the tab registered for its kind (`reveal_tabs`, the `replay_tab` precedent) and calls the new
  defaulted `Tab::reveal`. Each tab unfolds or clears the filter that hides the row, or parks the
  target until its next list reply; a target that is not there is reported by key; a refused list
  read disarms it.
- **A closed overlay forgets its freshness entries** (D240). `App::latest` was never cleared on a
  pop, so a reply to a closed overlay could land in a reopened one. Every pop path (close, close
  all, scope change) now goes through one helper. This fixes every overlay, not only the search.
- **Quit is bounded.** `main` builds its runtime and shuts it down with `htui::SHUTDOWN`, so quitting
  during the first model download no longer waits for the download (review 1).
- **Shared formats.** `concepts::query` and `concepts::report_line` are used by both the CLI and the
  TUI, so `--decisions` and the report wording have one source (D239). `FastEmbedder` is `Clone`.

## Decisions (maintainer)

- 2026-09-29, CONFIRM: OQ-1 re-index in the TUI (`Ctrl+R`); OQ-2 "all" is the current workspace;
  OQ-3 `Ctrl+F`; OQ-4 search on `Enter`.
- 2026-09-29, blueprint F1: the help text is `find`; 27 status-row snapshots were re-blessed, only
  their last line changed.

## Deviations from the plan

- **Reply shape** (blueprint F2): one `ConceptsReply` with the error inside each kind, not a bare
  `Result`, so the overlay knows whether a search or an index run failed.
- **Harness routing** (F3): the runtime is settled from `drive` every round; `Harness::settle` is
  unchanged. `spawn_with_runtimes` keeps its signature and delegates to a crate-private
  `spawn_with_concepts`.
- **Reveal targets carry the key** (F4) so a miss can name the item; the Backlog reports a miss on
  the status line (F5), the Requirements tab on its own notice. The private
  `RequirementsTab::reveal` became `unhide` (F6).
- **Model load and connection** changed at review (findings 2 and 3): the blueprint's
  `OnceCell` design (D245, F9, §3.2) is superseded by the single shared load and the cached
  connection described above.

## Review

`rust-reviewer`: APPROVE WITH FIXES, no CRITICAL or HIGH. All eight findings applied (three
MEDIUM: bounded quit, one model load at a time, connection reuse; five LOW: two tests that could
not fail, a stale pending reveal after a refused read, the refusal wording, long errors).

## Gates

`cargo test --workspace --all-features -- --test-threads=1` against the sandbox Postgres: 2653
passed, 0 failed, 26 ignored. `cargo clippy --workspace --all-features --all-targets -- -D warnings`
and `cargo fmt --check` clean. `Cargo.lock` unchanged.

## Left to other items

- **MOD-41** still owns the automatic index sync; the TUI's `Ctrl+R` is the manual one meanwhile.
- **Searching other workspaces** is not offered: a `Hit` carries no project or workspace, so a hit
  elsewhere could not be revealed without a lookup the index does not hold (OQ-2).
- **`cargo doc -p htui`** fails on three private-item links that predate this item
  (`agent_worker.rs:729`, `ui/text_area.rs:18`, `ui/text_field.rs:5`); the workspace baseline in
  `HANDOFF.md` does not list them because `--keep-going` never reaches `htui` past the
  `htui-core`/`htui-store` errors.
