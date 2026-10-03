# Plan: MOD-72 tool-call chips in the Runs flow view

**Source**: HANDOFF `MOD-72` (from MOD-28, `docs/ANA-12.md` §3.2; `R-TUI-4`)
**Routed**: plan path via `/handoff-run` (0 criteria fired; C4 judged borderline from the item text), accepted by
the maintainer 2026-10-03. Sandbox run `hr/MOD-72`.
**Complexity**: Medium (one new read through every store layer, plus one UI line)
**Status**: done 2026-10-03 (`docs/decisions/mod/mod-72.md`)

## Summary

MOD-28's flow view draws each step of the run under the cursor as a 20×4 node showing the slot, the status and the
phase. ANA-12 §3.2 also wants each node to show what the step did with tools, as chips like `⚒ bash ×3`. The Runs tab
cannot draw those today, because it has no `session_event` data: `StepEvents` reads one step's full log and only
Replay uses it.

MOD-72 adds one new read: **per-step tool-call counts for an item's runs**, grouped by step and tool kind. It goes on
`ReadStore`, so all three backends answer it (Postgres, the SQLite mirror, `MemStore`). The worker serves it as a new
`StoreRequest::ToolCalls` / `StoreReply::ToolCalls` pair. The Runs pane asks for it next to `Runs`, and `StepNode`
gains a third line inside the node that draws the counts as chips (`⚒ read×5 exec×3`).

## Design decisions (proposed, maintainer may amend at CONFIRM)

- **D1: group by `tool_kind`, not by title.** A `tool_call` row's payload carries `title` and `tool_kind`
  (`htui-agent/src/event.rs` `ToolCallEvent`). `claude-cli` writes the tool name as its title (`Bash`,
  `cli/claude.rs:266`). ACP agents write prose instead (`Read src/main.rs`, `acp/map.rs:523`; the demo fixture's
  `Read docs/ANA-9.md`). Grouping by title would therefore give one chip per call on ACP agents. `tool_kind` is the
  ten-value ACP vocabulary that both transports fill in (`ToolKind`, `event.rs:36-67`), so the counts mean the same
  thing on every agent. The example `bash ×3` becomes `exec×3`.
- **D2: a `ReadStore` method, all three backends.** `ReadStore::tool_call_counts(item) -> Result<Vec<ToolCallCount>>`.
  `session_event` is mirrored (`cache/read.rs` `step_events`), and the trait's rule is that a read of a mirrored
  table goes on `ReadStore` (`traits.rs` `document` doc). That puts the read on `ReadStore`, not on
  `WriteStore` where `relay_view` lives, which only applies to the unmirrored relay tables (`traits.rs:35-38`).
  Offline, the mirror answers from its last-N-steps window. A step outside the window gets no chips, which is the
  same as "no calls". That is acceptable for a decoration (see D6), and it avoids an empty view or an error offline.
  This **widens the item text's "Postgres plus MemStore"** by the cache: one runtime SQLite query, no `.sqlx` entry.
  It also brings conformance coverage: the new case joins `READ_CASES`, so Mem, Pg and Cache all have to agree.
- **D3: the model row.** `htui_core::model::ToolCallCount { step: StepId, tool_kind: String, calls: u32 }`.
  `tool_kind` is a `String` because `ToolKind` lives in `htui-agent`, which no store crate depends on. A row whose
  payload has no `tool_kind` counts as `"other"` (`COALESCE`), the mapper's own fallback. Each backend
  collects its rows and then sorts them in Rust by `(step, tool_kind)` byte order (the `sort_canonical` precedent:
  Postgres would order by collation and SQLite by bytes). Only `kind = 'tool_call'` rows count. A `tool_result` is
  half of a call, not a second call.
- **D4: request shape and timing.** `StoreRequest::ToolCalls { item }` → `StoreReply::ToolCalls { item, counts }`,
  name `"tool_calls"`, served through the ordinary read path (`serve`, next to `Runs`). The pane asks for it from
  `RunsTab::on_runs`, as it already does for `RunActions` and `RelayView`, and **only while the flow view is
  shown**, mirroring `sync_graph`'s rule that "the list never pays for it". `v` into the flow view asks once. So the
  live `RunStream` re-read and the 5-refresh active-run poll refresh the chips without new wiring. A reply for
  another item is dropped (`if Some(*item) == self.item`), and the app's per-discriminant staleness gate
  (`App::dispatch`/`is_fresh`) drops a superseded one.
- **D5: the node grows one row.** `NODE_H` 4 → 5 (border, head, phase, **chips**, border) for every node, so the
  layout does not jump when a running step makes its first call. `V_GAP` stays 3 (`Y_STEP` 7 → 8). The chip line is
  blank when a step has no counts. At zoom 0.5 the interior is too small, and the existing `row >= inner.height`
  guard drops the line.
- **D6: chip format.** `⚒` once, then `label×n` chips separated by a space. Larger counts come first, ties go by
  label, and there is never a `×0`. Short labels: `read edit del move find exec think fetch mode other`, and an
  unknown wire string is drawn as itself. The chips are fitted greedily into the 18-cell interior by display width
  (`ui::cells`). Whatever does not fit becomes a trailing `+N`, where N is the number of kinds left out, with room
  for it reserved. A pure function `chips(&[ToolCallCount], width) -> String` holds this logic, so it can be
  unit-tested apart from rataflow. The chips are drawn in `theme.dim`, so they stay secondary to the head and phase.
- **D7: the pane keeps the counts by step.** `RunsTab.tool_calls: BTreeMap<StepId, Vec<ToolCallCount>>`. An item
  change clears it, and a `ToolCalls` reply replaces it and then re-syncs the graph. `ExecutionGraph::sync` takes
  the map and passes each step's slice to `StepNode::new`. A re-sync of the same run keeps the viewport still
  (MOD-28 review L2 anchor). The list view is unchanged.
- **D8: out of scope.** Chips in the list view, per-tool *names* (would need a title parser per transport), any
  click/hover on a chip (MOD-71), and pairing calls with their result status (failed calls in red). These are noted
  as possible follow-ups at close-out, and none are minted unless the maintainer asks.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Read on all backends | `htui-core/src/store/traits.rs` `ReadStore::step_events`; `htui-store/src/backend.rs:674` | one trait method; `Backend` arms Memory / Online(pg) / Offline(cache); `Writer` delegates |
| Postgres read | `htui-store/src/pg/read.rs:431` `step_events` | `sqlx::query_as!` / `query!` over `session_event`, `map_sqlx` |
| Postgres join by item | `htui-store/src/pg/relay.rs:501` `relay_view` | `JOIN run r ON r.id = … WHERE r.item_id = $1` |
| Mirror read | `htui-store/src/cache/read.rs:649` `step_events` | runtime `sqlx::query` with `?` binds, `uuid_col`/`get` helpers |
| MemStore read | `htui-core/src/store/mem.rs:6397` | `self.read(|state| …)` over the in-memory event log |
| Read conformance | `htui-core/src/store/conformance.rs:496` `READ_CASES` / `run_read_case` | a named case, `CASE:`-prefixed asserts, runs on Mem, Pg (`tests/pg_conformance.rs`) and Cache (`tests/cache.rs`) |
| Write conformance (multi-kind data) | `conformance.rs` `CASES` (e.g. `relay_view_lists_live_pending_requests_and_pending_cancels`) | writes rows, then asserts the read; runs on Mem and Pg |
| Request/reply | `htui/src/store_worker.rs:789/1271/1758` `RelayView` | struct variant carrying `item`, `name()` arm, served in `serve` |
| Pane follow-up reads | `runs.rs:409` `on_runs` | `ctx.request(...)` after the rows land; reply guarded by `Some(item) == self.item` |
| Node rendering | `execution_graph.rs:167-251` `StepNode` | precomputed strings, `cells::clip` by display width, `Theme` styles |
| Tests | `execution_graph.rs` `mod tests` (`draw`, `rows`, `synced`); `tests/backlog.rs:783` `runs_flow_fanout` snapshot | `TestBackend` 43×23, `insta::assert_snapshot!` |
| Errors | store: `Result<…>` with `map_sqlx`; UI: no new error path (a `Failed` reply goes to the status line as today) | |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/run.rs` | UPDATE | `ToolCallCount` (D3) and its canonical sort |
| `crates/htui-core/src/store/traits.rs` | UPDATE | `ReadStore::tool_call_counts` (D2) |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `MemStore` impl over its event log |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | new `READ_CASES` entry (fixture: `STEP_PLAN` → `read×1`) and a `CASES` entry with multi-kind, multi-step, item-scoped data and a payload with no `tool_kind` |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | `CASES.len()` and `READ_CASES.len()` pins, +1 each |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | `EXPECTED_CASES` 130 → 131 and its history comment |
| `crates/htui-store/src/pg/read.rs` | UPDATE | Postgres impl |
| `crates/htui-store/.sqlx/query-*.json` | CREATE | offline entry for the new `query!` |
| `crates/htui-store/src/cache/read.rs` | UPDATE | mirror impl (`json_extract(payload, '$.tool_kind')`) |
| `crates/htui-store/src/backend.rs` | UPDATE | `Backend` dispatch arm |
| `crates/htui-store/src/writer.rs` | UPDATE | `Writer` delegation |
| `crates/htui-agent/src/conformance.rs` | UPDATE | `UsageSpy` delegation (implements `ReadStore`) |
| `crates/htui-agent/tests/recorder.rs` | UPDATE | `SpyStore` delegation (implements `ReadStore`) |
| `crates/htui/src/store_worker.rs` | UPDATE | `ToolCalls` request/reply, `name()`, `serve` arm, worker test |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | `tool_calls` map, ask from `on_runs`/`v`, reply arm, sync, tests (request lists after a `Runs` reply) |
| `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | UPDATE | `NODE_H` 5, chip line, `chips()`, `sync` signature, tests (position literals 7/14 → 8/16) |
| `crates/htui/tests/backlog.rs` | UPDATE | flow snapshot of a step with a call (`FEAT-1`'s plan step, `⚒ read×1`) |
| `crates/htui/tests/snapshots/backlog__runs_flow_*.snap` | UPDATE/CREATE | two existing flow snapshots move by one node row; one new |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/mod/mod-72.md`, `docs/ANA-12.md` | UPDATE/CREATE | close-out (T4) |

Eighteen code files. Read from the tree, C4 would have fired. The path stays plan (PRD needs ≥2 criteria), and the
tasks form one dependency chain (T1 types → T2 protocol → T3 UI), so they run **serial** with no implementer
fan-out.

## Tasks

### Task 1: the store read (TDD)
- **Action**: Write the conformance cases first (a `READ_CASES` case over the fixture; a `CASES` case with
  two steps × three kinds, a second item's run that must not leak in, a `tool_result` that must not count, and a
  payload without `tool_kind` that counts as `other`). Then `ToolCallCount`, the trait method, and the Mem / Pg /
  Cache impls, the `Backend` arm, and the `Writer`/`UsageSpy`/`SpyStore` delegations. Regenerate `.sqlx` against a
  migrated scratch DB.
- **Mirror**: `step_events` across the three backends; `relay_view`'s item join.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features --test pg_conformance --test cache -- --test-threads=1`; `cargo sqlx prepare --check` (from `crates/htui-store`).

### Task 2: the worker protocol (TDD)
- **Action**: Worker test first: `serve(ToolCalls { item: HTUI_FEAT_1 })` answers the fixture count, and
  `name() == "tool_calls"`. Then the variants and the `serve` arm.
- **Mirror**: `StoreRequest::RelayView` / `StoreReply::RelayView`; `StepEvents`'s test at `store_worker.rs:3317`.
- **Validate**: `cargo test -p htui --all-features store_worker`.

### Task 3: chips in the flow view (TDD)
- **Action**: Unit tests first. `chips()` covers ordering, the `+N` overflow, wide-glyph fit, an empty input
  (no line) and an unknown kind. A node draws the chip line, and a node without counts draws a blank line. A
  `ToolCalls` reply re-syncs the graph and keeps the viewport. The pane asks for `ToolCalls` after `Runs` only in
  the flow view, and `v` asks once. A reply for another item is ignored. Then the implementation (D5–D7), the
  position literal updates, the two moved snapshots, and a new integration snapshot.
- **Mirror**: MOD-28 T2/T3 tests (`synced`, `draw`, `rows`, `corner_of`; `runs.rs:3313-3345` request-list asserts).
- **Validate**: `cargo test -p htui --all-features execution_graph runs`; `cargo test -p htui --all-features --test backlog`; `cargo insta test` review of the snapshot diffs.

### Task 4: close-out docs
- **Action**: `docs/decisions/mod/mod-72.md`, the DECISIONS index row, the HANDOFF status block and checklist tick,
  an ANA-12 status note (§3.2 chips done, by kind), and this plan's status set to done.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # HTUI_TEST_DATABASE_URL is set in the sandbox
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A count query on every Runs poll is slow on a long log | Low | asked only in the flow view (D4); the join is driven from the item's `run_step` ids through `session_event`'s primary key. **Corrected at review (L4):** `idx_session_event_tool` is partial (`WHERE tool_call_id IS NOT NULL`) and this query cannot use it |
| Backends disagree on order (collation vs bytes) | Medium | sort in Rust (D3); the `READ_CASES` entry runs on all three |
| `⚒` drawn 2 cells wide by some terminals | Low | widths come from `ui::cells` (unicode-width, which says 1); the fit reserves by measured width; fact-check probes it |
| `NODE_H` change breaks MOD-28 layout facts (fit, reveal, R8 width) | Low | width untouched; height-only literals updated; `fit_keeps_the_cursor_node_on_screen` and the reveal tests re-run unchanged |
| Spy `ReadStore` impls forgotten (compile error only under `--all-features`/tests) | Medium | Files table lists both; the validation runs `--all-targets --all-features` |
| `.sqlx` regenerated without `--all-targets --all-features` deletes entries | Medium | use the `docs/hr-sandbox.md` recipe verbatim |
| htui suite flake from scheduling | Medium | gate with `--test-threads=1`, `--no-fail-fast` |
| `EXPECTED_CASES` / `CASES.len()` pins conflict with a sibling branch at merge (MOD-26, MOD-37 bump them too) | High | expected and mechanical: on collect, the merged value is the sum of both bumps; the close-out names it |

## Acceptance

- [ ] All tasks complete, TDD order kept
- [ ] Validation passes on the real tree
- [ ] The three backends agree (`READ_CASES`)
- [ ] Patterns mirrored, not reinvented

## Verified claims (step 3.5)

Checked 2026-10-03 at `f4e577a4` (`hr/MOD-72`, clean tree).

| Claim | Verdict | Evidence |
|---|---|---|
| `ReadStore` implementors are MemStore, PgStore, CacheStore, Backend, Writer, UsageSpy, SpyStore (seven) | ✓ | `impl … ReadStore for` across `crates/`: `mem.rs:6372`, `pg/read.rs:65`, `cache/read.rs:268`, `backend.rs:625`, `writer.rs:125`, `htui-agent/src/conformance.rs:640`, `htui-agent/tests/recorder.rs:327`; no other match |
| `ReadStore` methods have no default bodies, so every implementor must add the method | ✓ | `traits.rs:89-103` declares `step_events` etc. without bodies; both spies delegate explicitly |
| `ToolKind` lives in `htui-agent`; `htui-core` and `htui-store` do not depend on it | ✓ | `wire_enum!(ToolKind …)` at `htui-agent/src/event.rs:36-67`; no `htui-agent` dependency in either `Cargo.toml` |
| `claude-cli` titles a call with the tool name; ACP with prose | ✓ | `cli/claude.rs:266` `title: name.to_owned()`; `acp/map.rs:523/533` `"Read src/main.rs"`; fixture `fixtures.rs:1752` `"Read docs/ANA-9.md"` |
| Payload key is `tool_kind`, wire values are the ten snake_case strings | ✓ | `ToolCallEvent.tool_kind` (`event.rs:310`), `ToolKind` wire strings `event.rs:46-64`; fixture payload `"tool_kind": "read"` |
| `session_event` is mirrored; payload is TEXT JSON in SQLite; the cache already uses `json_extract` | ✓ | `cache_migrations/0001_mirror.sql:125-128`; `cache/read.rs:560` |
| A read of a mirrored table belongs on `ReadStore`; `relay_view` is on `WriteStore` because its tables are not mirrored | ✓ | `traits.rs` `document` doc ("On `ReadStore` … because `document` is mirrored"); `traits.rs:35-38`, `:1762-1764` |
| `idx_session_event_tool (run_step_id, tool_call_id) WHERE tool_call_id IS NOT NULL` exists | ✓ | `migrations/0001_init.sql:530-531` |
| The demo fixture has exactly one `tool_call` (kind `read`) on `STEP_PLAN` of htui `FEAT-1` | ✓ | `fixtures.rs:1720-1830`, one `EventKind::ToolCall` row |
| The mirror suite runs `READ_CASES` and holds `STEP_PLAN`'s events | ✓ | `htui-store/tests/cache.rs:1055` `the_mirror_passes_the_read_cases`; `:425-431` asserts cache and mem `step_events(STEP_PLAN)` agree |
| `MemStore` state can map item → runs → steps → events | ✓ | `mem.rs:220` `runs`, `:222` `steps`, `:248` `events`; `run_summaries(id)` at `:6394` |
| Conformance case counts are pinned | ✓ (**plan amended**) | `htui-core/tests/mem_store.rs:36,63`; `htui-store/tests/pg_conformance.rs:26` `EXPECTED_CASES = 130`; added to Files and Risks |
| `WriteStore::append_events` exists for the multi-kind `CASES` entry | ✓ | `traits.rs:312` |
| `on_runs` asks for `RunActions` and `RelayView` after each `Runs` reply; replies are guarded by item | ✓ | `runs.rs:409-430`, `:1342`, `:1396` |
| Staleness is per (origin, request discriminant), so a new variant needs no registration | ✓ | `app/state.rs:308-334` `dispatch` / `is_fresh` |
| `StoreRequest::name()` is an exhaustive match that needs one arm | ✓ | `store_worker.rs:935-1032` |
| The Runs poll re-asks `Runs` every 5 refreshes while a run is active | ✓ | `backlog/mod.rs:69` `REFRESHES_PER_RUNS_POLL = 5`, `:702-711` |
| The node is 20×4 and the interior loop drops rows that do not fit | ✓ | `execution_graph.rs:26-28`, `:241-249` |
| MOD-28 unit tests pin y positions 7/14 | ✓ | `execution_graph.rs:616-617`, `:636`, `:825-827` |
| Two flow snapshots exist and will move | ✓ | `tests/snapshots/backlog__runs_flow_fanout.snap`, `backlog__runs_flow_reject_note.snap` |
| `⚒` and `×` are one cell each under the locked `unicode-width` | ✓ (probe) | `unicode-width = 0.2.2` (`Cargo.lock`); probe `/tmp/uwprobe`: `"⚒" 1`, `"×" 1`, `"⚒ read×5 exec×3" 15` |
| `ui::cells` has `clip` and `cell_width` | ✓ | `cells.rs:66`; `clip` already used by `StepNode::render` |
| Task independence | n/a | T1 → T2 → T3 is a type dependency chain; nothing is marked parallel |
