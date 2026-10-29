# Plan: MOD-144 — Item status reduced to open, blocked, closed

**Source**: `HANDOFF.md` MOD-144; design `docs/ANA-35.md` §7.1 (verdict), §7.10 (migration order), §6
(analysis). Routed as **plan** by maintainer override (2026-10-27; the verdict was PRD on 2 criteria).
**Complexity**: Large (every crate but htui-agent; one migration; ~70 snapshots).
**Status**: DONE 2026-10-29 (confirmed 2026-10-27, Decisions A, B, C as recommended; PD18 refuse; write-up `docs/decisions/mod/mod-144.md`).

## Summary

Item `Status` shrinks from eight stored values to three (`open`, `blocked`, `closed`). Progress
(`queued`, `running`, `needs you`, `stopped`, `failed`, `done`) is derived from the item's **latest graph
run** — newest `kind = 'graph'` run by `(queued_at, id)`, `cancelled` skipped — through one pure
function `ItemState::of(status, latest_run_status)`. The store stops mirroring run moves onto the item;
"one live graph run per item" moves from the item-status lock to an explicit check plus a partial
unique index; readiness, release of blockers, waiting-on-you, close-out and `default_for` are re-anchored
on run facts. Migration `0023` maps the five removed values per ANA-35 §7.1. Search keeps its collection
name and drops/reindexes on a payload stamp (MOD-159 decision, 2026-10-20).

## Deltas from ANA-35 (tree moved since 2026-10-18)

| # | ANA-35 says | Tree today | Plan |
|---|---|---|---|
| Δ1 | display table has 5 run states | MOD-125 added `RunStatus::Stopped`: live (`is_active`), resumable, holds no slot (`run.rs:40-63`); kit `STOPPED` is `mark("⊘","stopped",Tone::Idle)` (`kit/label.rs:81`), the same glyph as item `blocked` (`kit/status.rs:49-53`) | **Decision A** — a 9th `ItemState::Stopped`, ranked after `running`, with a glyph of its own |
| Δ2 | live = queued/running/awaiting_approval | live = `is_active()` incl. `stopped` (write.rs:1661, 8039, 10118) | every "live graph run" test includes `stopped`: `uq_run_item_live`, `create_run`, readiness, `start_enabled`, migration pre-flight |
| Δ3 | Qdrant `v2 → v3` rename | MOD-159: no versioned names in development; collection `htui_concepts_v2` (`vector.rs:39`); no drop mechanism, only the embedder stamp (`check_existing`, `vector.rs:456-490`) | **Decision B** — a payload-schema stamp beside the embedder stamp; a mismatch drops and recreates the collection, the next `sync` refills it |
| Δ4 | Backlog grouping and `h` toggle key on `ItemState` | neither exists (MOD-119 work); list groups by project only (`list.rs:186`) | MOD-144 only rekeys what exists: status facet, list/detail/graph marks, theme, kit table; MOD-119 builds grouping and `h` on `ItemState` |
| Δ5 | — | list/detail/graph print raw `as_str()` (`list.rs:178,210`, `body.rs:124`, `graph.rs:535-561`) | they render the kit mark of `ItemState` (the redesign's words) |
| Δ6 | — | `ui/overlay/leave.rs:139-140` marks via `Status::InProgress/AwaitingApproval` | use `ItemState::Running/NeedsYou` marks |
| Δ7 | `toggle_queued` refuses `closed` | today refuses `Done \| Closed` (`backlog/mod.rs:544`); `prune_finished_entries` would drop a `done` item's entry at once | refuse `ItemState::Done \| Closed` (today's behaviour) |
| Δ8 | `create_run` check "stays" | the only guard today is the implicit `legal_move(status, Queued)` (write.rs:5861; mem.rs:5755) | explicit "status `open` and no live graph run" check under the item `FOR UPDATE`, today's refusal sentence |
| Δ9 | `default_for` reads the snapshot's last phase | `Run.graph_snapshot: Option<Value>`; typed `GraphSnapshot.phases[..].output_kind` exists (`run.rs:516, 604`), no accessor | add `GraphSnapshot::last_output_kind(&Value) -> Option<String>` |
| Δ10 | — | `closed_at` stamped by `transition` on `done` (write.rs:1923) / `is_terminal` (mem.rs:2243) | only `close_out` sets `closed_at`; `transition` never touches it |

## Decisions (CONFIRM answers these; recommended defaults are what the tasks assume)

- **A. A stopped latest run** → `ItemState::Stopped` ("stopped"), attention order
  `needs you, failed, running, stopped, blocked, queued, open, done, closed`. Glyph: kit `STOPPED` moves
  from `⊘` (shared with `blocked`) to `■` (U+25A0, single-width), so runs, steps and items read the same
  and one glyph means one thing; MOD-125's run/step snapshots move with it. Alternatives: keep `⊘` for
  both (word and tone differ, glyph-only columns are ambiguous); fold stopped into `running`
  (misleading: it holds no slot and waits for a person's resume).
- **B. Search payload change** → payload-schema stamp (`PAYLOAD_SCHEMA = 2`) checked in
  `ensure_collection`; mismatch or missing stamp → `drop_collection` + recreate + stamp; `Indexer::sync`
  refills. Alternatives: (b) no drop — deterministic point IDs overwrite item points, but section
  points of unparsable items are never cleaned; (c) refuse like the embedder stamp and tell the person
  to run `--index-items` by hand.
- **C. Pre-flight count** (ANA-35 §7.10) is host-only: the sandbox Postgres is empty and the host's is
  unreachable. Optional, before merging on the host:
  `docker exec -i htui-postgres psql -U postgres -d htui -c "<the §7.10 pre-flight SELECT, plus the two-live-graph-runs count>"`.
  The migration is correct for any counts; a non-zero two-live-runs count would make `0023` fail at
  `uq_run_item_live`, so the migration raises a readable error naming the items first.

## The rules (normative for every task)

- **Stored**: `Status = open | blocked | closed`. `can_move_to`: `open ↔ blocked` only; `closed` only by
  `close_out`. `transition` CAS, never bumps `version`, never touches `closed_at`.
- **Latest graph run**: newest `run` with `item_id = i.id AND kind = 'graph' AND status <> 'cancelled'`
  by `(queued_at DESC, id DESC)`.
- **Live graph run**: `kind = 'graph' AND status IN ('queued','running','awaiting_approval','stopped')`.
- **`ItemState::of(status, latest: Option<RunStatus>)`**: `closed → Closed`; `blocked → Blocked`;
  `open` → by latest: `AwaitingApproval → NeedsYou`, `Failed → Failed`, `Running → Running`,
  `Stopped → Stopped`, `Queued → Queued`, `Done → Done`, `None → Open` (`Cancelled` cannot be latest).
- **Ready**: status `open` AND no non-cancelled graph run at all *(equivalent to ANA-35's "no live run,
  latest neither failed nor done", since every non-cancelled run status is live, failed or done)* AND
  every live `blocked_by` target **released**.
- **Released**: target `closed`, OR target's latest graph run is `done` (MOD-133 adds "landed").
- **Waiting-on-you**: `i.status = 'blocked' OR EXISTS (run awaiting_approval)` (the item literal
  `'awaiting_approval'` goes).
- **Close-out**: refused while any run of the item is live (unchanged). `done`/`concluded` need a
  finished item (latest graph run `done`); `rejected`/`withdrawn`/`superseded`/`duplicate` close `open`
  or `blocked` whatever its runs did. `default_for`: finished and last phase `output_kind = 'verdict'` →
  `concluded`; finished → `done`; else `withdrawn`.
- **Queue prune** (`prune_finished_entries`): item closed OR finished.

## Migration `0023_item_status.sql`

Per ANA-35 §7.10 row 1, with Δ2: pre-check (raise with item keys if any item has two live graph runs),
`CREATE UNIQUE INDEX uq_run_item_live ON run (item_id) WHERE kind = 'graph' AND status IN
('queued','running','awaiting_approval','stopped')`; `trg_item_updated_at` off; temp `unexplained`
(done/failed whose latest graph run status differs); notes as `created_by`; unexplained `done` →
`closed`/`done`/`closed_at`; unexplained `failed` → `blocked`; the rest of the five → `open`,
`closed_at = NULL`; trigger on; `DROP CONSTRAINT item_status_check`; `ADD CONSTRAINT chk_item_status
CHECK (status IN ('open','blocked','closed')) NOT VALID; VALIDATE`. No new table (TABLES 51 stays); no
cache migration (cache `item.status` has no CHECK; the version bump rebuilds every mirror).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Enum | `htui-core/src/model/mod.rs:25-77` `str_enum!` | `ItemState` via the same macro (as_str, ALL, serde, FromStr) |
| Migration CHECK swap | `migrations/0019_stop_runs.sql:25-31` | drop the auto-named CHECK, add a named one |
| Backfill with trigger off | `migrations/0006_requirements.sql:23-25` | `ALTER TABLE item DISABLE/ENABLE TRIGGER trg_item_updated_at` |
| Migration test | `htui-store/tests/migrations.rs:1333,1672,1701` | `MIGRATOR.run_to(22)` → plant → `MIGRATOR.run` → assert; repin `Pending(22)`→`23` (:3561,3662,3671,3694) |
| Latest-run LATERAL | `pg/read.rs:2430-2443` (`queue_rows`) | same shape + `AND r.status <> 'cancelled'` for `latest_run_status`; `queue_rows`' own LATERAL keeps cancelled runs (`CancelledInBatch` reads them) and gains a separate `latest_run_status` column |
| Appended field | `model/item.rs:183` doc "Appended, never inserted (plan D10)" | `latest_run_status: Option<RunStatus>` after `touched_paths` |
| Store conformance | `htui-core/src/store/conformance.rs` `CASES` (:76), pins `htui-core/tests/mem_store.rs:37` and `htui-store/tests/pg_conformance.rs:43` (`EXPECTED_CASES`) | each rule gets a case run against Mem and Pg; both pins move together |
| Stamp on collection | `vector.rs:455-470` `check_existing` / `EMBEDDER_KEY` | `PAYLOAD_SCHEMA_KEY` beside it |
| Kit marks | `ui/kit/status.rs:19-76`, `kit/label.rs:126-137` | TABLE keyed by `ItemState`, attention order; `run_status` keeps reusing the marks |

## Tasks

Dependency chain is by crate: `htui-store` and `htui-orch` depend on `htui-core` only (htui-orch never
pulls htui-store, `Cargo.toml:17-20`); `htui-worker` depends on `htui-store` and `htui-orch`; `htui-mcp`
depends on `htui-orch` and dev-depends on `htui-store`; `htui` depends on all. Order: T1 → {T2 ∥ T3} →
{T3b ∥ T4} → T5 → T6. File sets of every parallel pair are disjoint crates; no snapshot, `.sqlx` or
fixture file is shared (fixtures live in htui-core, T1's). TDD per task: conformance/unit tests
first, then code. Each task commits incrementally on `hr/MOD-144`.

### T1 — htui-core: model, MemStore, conformance (serial, first)
Files: `crates/htui-core/src/model/{item.rs,run.rs,queue.rs,link.rs}`, `src/store/{traits.rs,mem.rs,conformance.rs}`,
`src/prompt/{render.rs,fixtures.rs}`, `src/fixtures.rs`, `crates/htui-core/tests/**` (incl. `mem_store.rs` pin, prompt snapshots).
- `Status` → 3 variants; `can_move_to`; drop `is_terminal` (callers move to the release rule).
- `ItemState` (`str_enum!`, attention-order `ALL`) + `ItemState::of`; `ItemSummary.latest_run_status`
  appended, `ItemSummary::state()`; `ItemFilter.states: Option<Vec<ItemState>>` added (`statuses` stays).
- `Resolution::closes_from(self, status, finished: bool)`, `Resolution::default_for(finished_output_kind: Option<&str>)`;
  `GraphSnapshot::last_output_kind(&Value)`.
- `Wait::NotReady(ItemState)`; `classify_entry`: live = `is_active()` (Δ2: stopped → `Running`/`Elsewhere`
  rules unchanged for queued/running; stopped and parked handled via the latest run), `Failed` from the
  latest run, HardGate/Judge from "open + parked run", Blocked arms unchanged.
- `UpstreamEntry` and `LinkNode` gain the latest run status (appended); prompt upstream prints the
  `ItemState` word.
- Store trait docs; `finish_run_item_mirror` deleted. MemStore: `create_run` explicit live check,
  `claim_run`/`promote_step`/`park_step`/`finish_run` drop item writes, MissingTags `open → blocked`,
  readiness/release/waiting/close-out/prune per the rules, `item_summaries` fills `latest_run_status`,
  `states` filter.
- Conformance: rewrite the 24 cases using removed values; add cases: live-run refusal incl. `stopped`;
  readiness with cancelled skipped; release by a `done` run; rework run holds dependents, its cancel
  releases them; waiting candidates; close-out law; `default_for` from snapshot (`verdict` →
  `concluded`); `ItemState` derivation per row; `transition` leaves `closed_at`. Repin `CASES`.
- Fixtures: demo items "across statuses" become `open` items with runs in those states (+ one
  `blocked`, one `closed`); `every_status_appears` → every `ItemState` appears. Prompt snapshots move.
- Validate: `cargo test -p htui-core --all-features`.

### T2 — htui-store: migration, Pg, cache, search, demo, .sqlx (after T1; parallel with T3)
Files: `crates/htui-store/migrations/0023_item_status.sql`, `src/pg/{write.rs,read.rs,rows.rs,demo.rs}`,
`src/cache/{read.rs,…}`, `src/vector.rs`, `src/vector_sync.rs`, `.sqlx/**`, `crates/htui-store/tests/**`.
- Migration + `migrations.rs` test planting ANA-35 §7.10's rows (one per removed status with a
  matching run; `done` with no run; `done` whose latest run failed; `done` whose `done` run is followed
  by a cancelled one; `failed` with no run; two live runs → readable failure on a scratch DB) and the
  `Pending` repins.
- Pg writes per the rules (`create_run`, `claim_run`, `promote_step`, `park_step`, `finish_run_on`,
  `transition`, `close_out`, `close_batch` comment, `prune_finished_entries`).
- Pg reads: `items` (`query_as!` at `pg/read.rs:100`) / `ready_items` (`query_as!` at `:2167`) /`queue_rows`/`waiting_candidates`/`requirement_coverage`/link nodes/
  upstream project `latest_run_status`; release and readiness SQL; `states` filter (a `CASE` over the
  latest run).
- Cache SQLite: `items(ready)`, `waiting_candidates`, `item_summary_of`, link nodes — same rules.
- Search: 3-value `status` payload; payload-schema stamp (Decision B); docs at `vector.rs:35-38`.
- Repin `htui-store/tests/pg_conformance.rs:43` `EXPECTED_CASES` to T1's `CASES` count.
- `.sqlx` regenerated with `cargo sqlx prepare` against a migrated scratch DB (`docs/hr-sandbox.md`).
- Validate: `cargo test -p htui-store --all-features -- --test-threads=1` with `HTUI_TEST_DATABASE_URL`
  and `HTUI_TEST_QDRANT_URL`; Pg runs core conformance.

### T3 — htui-orch (after T1; parallel with T2)
Files: `crates/htui-orch/src/{engine.rs,command.rs,gate.rs,closeout.rs,conformance.rs}`,
`crates/htui-orch/tests/**`.
- Drop the 7 mirror `.transition(` calls (engine.rs:1899, 2639, 2993, 3233, 4815, 5643, 6816).
- `refuse_no_candidate` (:3484), `block_and_fail` (:5785), `gate.rs::escalate` (:1063): `open → blocked`;
  `refuse_rung_four`/`refuse_missing_tags` doc comments updated; FollowRun (:1760) `blocked → open`.
- `enqueue_with` (:724) `Open | Failed` → `Open`.
- `start_enabled` = open and no live graph run; `unblock_enabled` Resume = blocked + parked run (a
  stopped run still refuses as walking — it resumes through MOD-125's resume); `close_out_enabled` uses
  the new `default_for`; `NotBlocked`/`NotClosable` carry `ItemState` where they print.
- Rewrite the unit tables (`start_enabled_is_the_law`, close-out table, unblock cases) and the 33
  conformance references; repin `fake_conformance.rs:18` and `src/conformance.rs:8391` together if
  cases are added.
- Validate: `cargo test -p htui-orch --all-features`.

### T3b — htui-worker (after T2 and T3; parallel with T4)
Files: `crates/htui-worker/src/views.rs`, `crates/htui-worker/tests/**`.
- 20 test references to removed values (views.rs tests, `tests/auto_queue.rs:1773`); `views.rs::verdicts`
  and `unblock_case` follow T3's predicates.
- Validate: `cargo test -p htui-worker --all-features -- --test-threads=1` (its `*_pg` tests need 0023).

### T4 — htui-mcp (after T2 and T3)
Files: `crates/htui-mcp/src/tools/{status.rs,search.rs}`, their snapshots.
- `item_status` and `search_concepts` enums → 3 values; tool description words.
- Validate: `cargo test -p htui-mcp --all-features`.

### T5 — htui TUI and binary (after T3b and T4)
Files: `crates/htui/src/ui/{theme.rs,kit/status.rs,kit/label.rs,overlay/leave.rs,overlay/queue.rs}` (label.rs:81 `STOPPED` glyph, Decision A),
`ui/tabs/backlog/{filter.rs,list.rs,mod.rs,detail/body.rs,detail/graph.rs}` (a `LinkNode` literal at graph.rs:929), `ui/tabs/requirements/detail.rs`,
`src/queue_overview.rs`, `src/app/update.rs`, `src/store_worker.rs`, `src/run_worker.rs`, `crates/htui/tests/**`, snapshots.
- `status_style` and the kit TABLE keyed by `ItemState` (attention order); status facet over
  `ItemState` through `ItemFilter.states`; list/detail/graph/coverage/leave render `ItemState` marks;
  `toggle_queued` refuses `Done | Closed` states; `queue_overview.rs:78` gate on `ItemState::Open`.
- Full insta run, review every moved snapshot (raw words → redesign words), accept.
- Validate: `cargo test -p htui --all-features -- --test-threads=1`.

### T6 — docs and gate (last)
- `README.md:566-571` collection wording; `CONCEPTS.md` if it states the eight statuses; HANDOFF
  coordinates (next migration `0024`), MOD-119/MOD-135/MOD-150 notes pointing at `ItemState`.
- Validate: `bash scripts/gate.sh`; `cargo clippy --workspace -- -D warnings` (featureless).

## Validation

```bash
cargo test -p htui-core --all-features
cargo test -p htui-store --all-features -- --test-threads=1   # needs HTUI_TEST_DATABASE_URL, HTUI_TEST_QDRANT_URL
cargo test -p htui-orch -p htui-worker -p htui-mcp --all-features
cargo test -p htui --all-features -- --test-threads=1
cargo clippy --workspace -- -D warnings
bash scripts/gate.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Workspace stays red from T1 until T5 lands | Certain | per-crate validation per task; `-p` builds; commits stay per crate |
| A derived-state SQL drifts between Pg, cache and Mem | Medium | conformance cases run on Mem and Pg; cache `items(ready)` covered by its own test with the same rows |
| `stopped` forgotten in one live set | Medium | one `LIVE_RUN_STATUSES` SQL fragment/const per store; conformance case with a stopped run |
| Snapshot churn hides a real regression | Medium | review lens: every moved snapshot must only swap words/glyphs; reviewer checks the diff |
| `uq_run_item_live` fails on real data | Low | readable pre-check in the migration; optional host pre-flight (Decision C) |
| `.sqlx` stale entries | Medium | regenerate against a migrated scratch DB; offline build in the gate |
| Pin moves (`CASES`, `Pending(N)`) collide at merge with other branches | Medium | renumber at collect per HANDOFF coordinates |

## Acceptance

- [ ] All tasks complete; every removed variant gone (`Status::{Queued,InProgress,AwaitingApproval,Failed,Done}` absent)
- [ ] Migration test asserts every §7.10 mapping and both notes
- [ ] Conformance covers each normative rule on Mem and Pg
- [ ] `bash scripts/gate.sh` green; featureless clippy green
- [ ] Reviewer (`rust-reviewer`) findings applied or deferred with the maintainer

## Verified claims

Step 3.5, 2026-10-27: an independent verifier plus inline probes. Amendments applied above (worker split
out of T3 into T3b; T4 after T2; Decision A glyph; two more pins; two `query_as!` sites; queue LATERAL).

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `RunStatus` has 7 variants; `is_active` = queued, running, awaiting_approval, stopped | TRUE | `run.rs:33-51, 57-62` |
| 2 | kit has a `STOPPED` mark; kit TABLE keyed by `Status` | PARTLY → amended | `label.rs:81,132`; glyph `⊘` collides with blocked (`status.rs:49-53`) — Decision A |
| 3 | collection `htui_concepts_v2`; embedder stamp; `drop_collection` test-only; `ensure_collection` | TRUE | `vector.rs:39, 456, 771, 779-805` |
| 4 | `ItemSummary` append-only, last `touched_paths`; `ItemFilter.statuses` | TRUE | `item.rs:183, 207-210, 217` |
| 5 | 7 mirror calls, 3 block writes, FollowRun, `enqueue_with Open\|Failed` | TRUE | engine.rs:1899, 2639, 2993, 3233, 4815, 5643, 6816; 3484, 5785; gate.rs:1063; 1760; 724 |
| 6 | Pg `create_run`'s only one-live-run guard is `legal_move(.., Queued)` | TRUE | `write.rs:5847, 5861`; mem.rs:5755 |
| 7 | `transition` stamps `closed_at` (Pg on done, Mem on `is_terminal`) | TRUE | `write.rs:1923`; `mem.rs:2244` |
| 8 | both prunes read done\|closed | TRUE | `write.rs:10154`; `mem.rs:1266` |
| 9 | cache has no `ready_items`/`queue_rows` | TRUE | `backend.rs:542-548, 693-698`; rules at `cache/read.rs:404-407, 1343-1345` |
| 10 | `Pending(22)` pins; `MIGRATOR.run_to` | TRUE | `migrations.rs:3561, 3662, 3671, 3694` |
| 11 | conformance pins 206 / 109 | PARTLY → amended | plus `pg_conformance.rs:43`, `htui-orch/src/conformance.rs:8391` |
| 12 | `toggle_queued` refuses Done\|Closed | TRUE | `backlog/mod.rs:539, 544` |
| 13 | leave overlay marks via `Status` | TRUE | `leave.rs:139-140` |
| 14 | no `h` toggle, no status grouping | TRUE | `list.rs:186` groups by project |
| 15 | `GraphSnapshot.phases[].output_kind`, no accessor; `graph_snapshot: Option<Value>` | TRUE | `run.rs:238, 516, 526, 604` |
| 16 | `ItemSummary` construction sites | PARTLY → amended | 7 literals, no spreads; plus `query_as!` at `pg/read.rs:100, 2167` |
| 17 | `UpstreamEntry`/`LinkNode` carry `Status`; prompt prints `as_str` | TRUE | `link.rs:84, 136`; `render.rs:369, 388`; literal `graph.rs:929` |
| 18 | MCP enums from `Status::ALL`; `item_status` only writes a note | TRUE | `status.rs:34, 79`; `search.rs:55-61` |
| 19 | README names the collection | TRUE | `README.md:566-571` |
| 20 | DDL compiles: partial unique incl. `stopped`, trigger off/on, temp table, CHECK swap NOT VALID + VALIDATE; sqlx wraps each migration in a transaction | TRUE | probe on a scratch DB with 0001-0022 (BEGIN…ROLLBACK, planted rows incl. a stopped run); no `-- no-transaction` in any migration |
| 21 | readiness ≡ "open and no non-cancelled graph run" | TRUE | every non-cancelled run status is live or done/failed |
| 22 | `item_note.created_by` NOT NULL; `item.created_by` exists | TRUE | `\d item_note`, `\d item` |
| 23 | CHECK auto-named `item_status_check`; trigger `trg_item_updated_at` | TRUE | `pg_constraint`/`pg_trigger` on the scratch DB |
| 24 | cache mirrors `run` with `kind`, `status`, `queued_at` | TRUE | `cache_migrations/0001_mirror.sql:103-110` |
| 25 | crate deps: orch ⟂ store; worker → store; mcp dev → store | TRUE | each `Cargo.toml` |
| 26 | T2 ∩ T3, T3b ∩ T4 file sets empty | TRUE | disjoint crate dirs; no shared snapshots/`.sqlx`/fixtures |
