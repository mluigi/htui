# Plan: MOD-159 M1 — Lease and collections

**Source PRD**: `.claude/prds/mod-159-knowledge-indexer.prd.md`
**Selected Milestone**: 1 — Lease and collections
**Complexity**: Large
**Run**: `/handoff-run MOD-159` in an hr sandbox (`hr/MOD-159`); CONFIRM waived by `/handoff-run`. Lane shape
(ultracode for implement), one serial lane: T1 → T2 → T3 → T4 → T5. Verifier on T1 (foundation) only.

## Summary

Replace the one shared collection `htui_concepts_v2` with one Qdrant collection per workspace (`htui_ws_<uuid>`) plus
a global one (`htui_global`, empty until MOD-167/168), and make exactly one htui process index each collection: the
holder of that collection's row in a new Postgres table `indexer_lease`. Every TUI and worker competes for every
collection (maintainer, 2026-11-14). The full reconcile runs under the lease; it renews the lease right before each
project's one delete batch and stops when the renewal touches nothing, and it deletes an "orphan" only after a fresh
read confirms its row is gone. Searches (TUI, CLI, MCP) read the right workspace collection(s) plus the global one.
`Ctrl+R` and `--index-items` index only when the lease is free or ours, and otherwise say which box holds it (the pass
request itself is M3). The incremental 60 s cycle is M2.

## Decisions taken in this plan

- **D1 Collection identity.** `htui_core::model::IndexedCollection { Workspace(WorkspaceId), Global }` with
  `fn name(&self) -> String`: `htui_ws_<workspace uuid, simple (32 hex, no hyphens)>` and `htui_global`. UUID, never
  slug (slugs can be renamed). `IndexedCollection::parse(&str) -> Option<Self>` inverts it (used to list
  `htui_ws_*` collections in Qdrant and in tests). `htui_store::vector::COLLECTION` is renamed
  `LEGACY_COLLECTION` (still `"htui_concepts_v2"`), used only to drop it (D10) and by tests/bench that name it.
- **D2 Lease table** `indexer_lease`, migration `0036_indexer_lease.sql`, **not mirrored** (live coordination state,
  like `workspace_stop`; no cache migration, header says so):
  ```sql
  CREATE TABLE indexer_lease (
      collection        TEXT PRIMARY KEY
                        CHECK (collection ~ '^htui_(global|ws_[0-9a-f]{32})$'),
      lease_owner       UUID,          -- per-process token; NULL = free
      lease_box_id      UUID,          -- box of the holder; no FK (display only, LEFT JOIN box)
      lease_expires_at  TIMESTAMPTZ,
      pass_requested_at TIMESTAMPTZ    -- M3 (Ctrl+R / --index-items request a pass); unused in M1
  );
  COMMENT ON TABLE indexer_lease IS '...MOD-159: one row per Qdrant collection; its holder alone indexes it...';
  ```
  No FK to `workspace` (a deleted workspace's row is inert; cleaning its Qdrant collection is deferred, see Risks)
  and none to `box` (no delete-reach list changes). Rows are created by the first take (`INSERT … ON CONFLICT`).
- **D3 Store seam: a narrow trait**, modelled on MOD-122's `WorkerLogStore` (`htui-core/src/store/worker_log.rs`),
  not `WriteStore` (five implementors). `htui-core/src/store/indexer_lease.rs`:
  ```rust
  pub trait IndexerLeaseStore: Send + Sync {
      /// Takes the lease when free, expired or already `owner`'s; renews it in that last case.
      fn take_indexer_lease(&self, collection: &IndexedCollection, box_id: BoxId, owner: Uuid, ttl: TimeDelta)
          -> impl Future<Output = Result<LeaseTake>> + Send;
      /// Moves the expiry when `owner` still holds it; `false` (zero rows) = abandon.
      fn renew_indexer_lease(&self, collection: &IndexedCollection, owner: Uuid, ttl: TimeDelta)
          -> impl Future<Output = Result<bool>> + Send;
      /// Frees every lease `owner` holds (clean exit); the count freed.
      fn release_indexer_leases(&self, owner: Uuid) -> impl Future<Output = Result<u64>> + Send;
      /// One collection's lease as people see it (no owner token).
      fn indexer_lease(&self, collection: &IndexedCollection)
          -> impl Future<Output = Result<Option<IndexerLease>>> + Send;
  }
  pub enum LeaseTake { Taken, Ours, Held(IndexerLease) }
  pub struct IndexerLease { pub collection: String, pub box_id: Option<BoxId>, pub box_name: Option<String>,
                            pub expires_at: Option<DateTime<Utc>>, pub live: bool }
  ```
  `live` = owner set and `expires_at > now` (store clock). TTL guarded by the existing `lease_ttl_micros`
  (`traits.rs:3636`). Model types in `htui-core/src/model/indexer.rs` (re-exported from `model`), trait re-exported
  from `store/mod.rs` beside `WorkerLogStore`. The owner token never appears in a read model (as `Run` keeps
  `lease_owner` out, `model/run.rs:317`).
- **D4 Lease times** are the run lease's: `htui_orch::recover::LeaseTimes::from_app(&app_settings)` (TTL 120 s,
  refresh clamped to ≤ TTL/3, so a 40 s beat by default). `htui` already depends on `htui-orch`.
- **D5 Owner token**: one `Uuid::now_v7()` per process for indexing, minted where the index job / `ConceptsRuntime` /
  `--index-items` start. Box id: `PgStore::this_box()` (`pg/mod.rs:765`); MemStore's `this_box`.
- **D6 Guarded reconcile.** `Indexer::sync(read, scope, store, guard: &impl DeleteGuard)` where
  ```rust
  #[allow(async_fn_in_trait)]
  pub trait DeleteGuard { async fn before_delete(&self) -> Result<bool, StoreError>; }
  pub struct Unguarded; // always true: tests and callers outside a lease
  ```
  Per project, all deletes of that project's pass (stale sections of rebuilt items, requirement stale + gone,
  owner-document stale + superseded, item orphans) are collected into **one batch at the end of the project**.
  Before it: `guard.before_delete()`; `false` → stop at once, nothing deleted, `report.abandoned = true`, return
  `Ok(report)` (`SyncReport` gains `pub abandoned: bool`; `AddAssign` ORs it). Then **orphan confirmation in the same
  pass**: an item orphan point is dropped from the batch when `read.item(id)` now returns an item of this project; a
  requirement orphan when `read.requirement(id)` returns a row of this project; an owner-document point whose document
  is not in the first listing when a re-listing (`read.owner_documents(project)`) shows it as the newest of its kind.
  Kept points are left for the next pass. Upserts are unchanged (idempotent, never guarded).
- **D7 Index runs under the lease, for every collection** (`concepts.rs`). A new `IndexJob` replaces `index_job`/
  `job_cycle`'s single-collection loop:
  - Each **beat** (`LeaseTimes.refresh`): resolve settings (no URL → log once, sleep, as today); list collections =
    every workspace from `index_scopes()` + `Global`; for each: `take_indexer_lease` (`Taken`/`Ours` → held,
    `Held` → not ours, drop any cached state). For each held collection whose last full reconcile is older than
    `concepts_sync_minutes` (or never ran in this process): connect that collection (`QdrantIndex::connect(settings,
    &collection)`), `Indexer::sync` over its workspace scope (`workspace_search_scope`) with a guard that renews this
    collection's lease; `Global` has nothing to sync in M1 (connect only, which creates it). `abandoned` → log info,
    forget the collection as held.
  - The model is loaded only when a held collection needs a sync (connect), never to contend.
  - `IndexJob::stop(self)` aborts the task and then calls `release_indexer_leases(owner)` (best effort, logged).
  - Generic over `S: IndexSource + IndexerLeaseStore` so tests drive it on `MemStore` with `MemVectorStore`s.
- **D8 Who runs it**: `htui worker` (`worker_cmd.rs:130`, as today, now `IndexJob` and `stop` instead of `abort`) and
  the **TUI**: `ConceptsRuntime::with_index_job()` (used only by `spawn_hosted`, the TUI's store worker) starts the job
  over `backend.writable()`'s `PgStore` when online and that pool changes (the same place `publish` runs each
  iteration), stops it (releasing) when offline or at `shutdown`. `--demo`/Memory backends and the test harness run no
  job.
- **D9 Searches.** `ConceptIndex::search(&self, within: SearchIn, query: SearchQuery)` with
  `pub enum SearchIn { Workspaces(Vec<WorkspaceId>), Everywhere }`. The index searches each named workspace's
  collection plus `htui_global` (`Everywhere` = every `htui_ws_*` collection Qdrant lists, via
  `client.list_collections()`, plus global); a missing collection contributes no hits; hits are merged by score
  (descending), deduplicated on `(point_type, owner, document, snippet)` (a project in two workspaces), and cut to
  `limit`. Callers:
  - TUI overlay: `StoreRequest::SearchConcepts` gains `workspace: WorkspaceId` (`ctx.scope.workspace_id`); serve
    passes `SearchIn::Workspaces(vec![workspace])`; a nil workspace answers `Ok(vec![])` as an empty scope does.
  - MCP `search_concepts` (`mcp_search.rs`): `SearchIn::Everywhere` with the session's project filter (it knows no
    workspace; agents' workspace scope stays MOD-112 (4)). Results are what they were: the project filter decides.
  - CLI `--search-items` (`concepts.rs::search_online`): one search per workspace scope from `scopes()` (each its own
    collection and that scope's projects; `--cwd` → every workspace holding the cwd project), plus global once,
    merged as above. A workspace whose collection is missing or refused is **named**: human output adds a stderr
    note `knowledge search unavailable for workspace(s): <slug>, …`; JSON adds `"unavailable": ["<slug>", …]`, omitted
    when empty. When every collection is missing the answer stays `empty` (exit 0) as today; other exit codes
    unchanged. `docs/htui-cli.md` documents the field.
- **D10 Legacy collection**: the holder of the `Global` lease drops `htui_concepts_v2` when it exists (one info line),
  then never looks again in that process. `--index-items` does the same when it takes `Global`.
- **D11 Probes** (`QdrantIndex::check`, `worker_cmd::start_check`): probe `htui_global` (a missing collection is
  `NoCollection`, as an empty Qdrant is today).
- **D12 `Ctrl+R` and `--index-items` in M1**: take the lease when free (`Taken` → sync → release that one lease;
  `Ours` → sync, keep it); `Held(l)` → no sync, and say so: the overlay line `indexing runs on <box_name or short box
  id> (lease until <HH:MM>)` in the dim (not error) style, via `ConceptsReply::Indexed(Result<IndexOutcome, String>)`
  with `pub enum IndexOutcome { Synced(SyncReport), HeldBy(IndexerLease) }`; the CLI prints
  `workspace <slug>: indexing runs on <box>; not indexed here` to stderr and continues (exit 0). The Ctrl+R lease
  comes from `ConceptsRuntime`'s owner token, so the TUI's own job's lease is `Ours`. `IndexerLeaseStore` is
  implemented for `Backend` (Online → pg, Memory → mem, Offline → `StoreError::Unreachable`) so the overlay tests
  keep running over `MemStore`. The box id: Online → `PgStore::this_box()`; Memory → `MemStore::box_info()`'s id
  (the demo has one); none → the reply is `Err("knowledge index unavailable: this box has no identity")`.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store trait | `htui-core/src/store/worker_log.rs:21-82`; Pg `htui-store/src/pg/worker_log.rs:25`; Mem `mem.rs:12363` | `fn … -> impl Future<Output = Result<T>> + Send`, impls write `async fn`; Pg impl in a child module of `pg` |
| Lease SQL | `pg/write.rs:7431-7464` (`refresh_lease`), `7710-7756` (`take_lease`), `7769-7799` (`release_lease`) | `clock_timestamp() + $n::bigint * interval '1 microsecond'`; zero rows = not ours |
| Own case list | `conformance.rs:1632-1681` (`WORKER_LOG_CASES`, `run_worker_log_case(_inline)`, `run_all_worker_logs`); pins `mem_store.rs:112-114`, `pg_conformance.rs:179-183`; uniqueness `case_names_are_unique` | `const CASE: &str = "…";` per case |
| Unmirrored table | `migrations/0019_stop_runs.sql:20, 86` | header + `COMMENT ON TABLE` say "not mirrored" |
| Heartbeat timing | `htui-orch/src/recover.rs:45-82` (`LeaseTimes`) | read app settings each beat |
| Index job | `concepts.rs:388-496` (`job_cycle`, `index_job`, `spawn_index_job_over`) | resolve per cycle, log once on no URL, forget the connection after a failure |
| Tests | `vector_sync.rs` unit tests over `MemStore` + `MemVectorStore`; `concepts_worker.rs` `MemIndex` tests | tests first, named as sentences |

## Files to Change (per task; serial lane, so overlap is allowed but listed)

| Task | Files |
|---|---|
| T1 | `crates/htui-core/src/model/indexer.rs` (new), `model/mod.rs`, `crates/htui-core/src/store/indexer_lease.rs` (new), `store/mod.rs`, `store/mem.rs`, `store/conformance.rs`, `crates/htui-core/tests/it/mem_store.rs` |
| T2 | `crates/htui-store/migrations/0036_indexer_lease.sql` (new), `crates/htui-store/src/pg/indexer_lease.rs` (new), `pg/mod.rs`, `crates/htui-store/src/backend.rs`, `crates/htui-store/.sqlx/*`, `tests/it/pg_conformance.rs`, `tests/it/migrations.rs`, `tests/it/connect.rs` |
| T3 | `crates/htui-store/src/vector.rs`, `crates/htui-store/src/vector_sync.rs`, `crates/htui-store/src/testkit.rs` (if it names `COLLECTION`), `crates/htui-store/tests/it/qdrant_live.rs` |
| T4 | `crates/htui/src/concepts_worker.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/overlay/concepts_search.rs`, `crates/htui/src/mcp_search.rs`, their snapshots |
| T5 | `crates/htui/src/concepts.rs`, `crates/htui/src/worker_cmd.rs`, `crates/htui/src/concepts_worker.rs` (`ConceptsRuntime::with_index_job`), `crates/htui/src/store_worker.rs` (`spawn_hosted`), `crates/htui/src/state_cmd.rs` (if the JSON shape lives there), `crates/htui/tests/it/qdrant_worker.rs`, `crates/htui/tests/it/state_cli_pg.rs`, `docs/htui-cli.md`, `docs/htui-worker.md` |

T4 and T5 share `concepts_worker.rs` and `store_worker.rs`: serial by construction.

## Tasks

### T1: Lease model, narrow trait, MemStore, conformance cases (foundation; tests first)

- **Action**: D1 (model only: `IndexedCollection`, `name`, `parse`), D3. Write the conformance cases first in a new
  `INDEXER_LEASE_CASES` list (own `run_indexer_lease_case(_inline)` / `run_all_indexer_leases`, bound
  `S: WriteStore + IndexerLeaseStore`, as `WORKER_LOG_CASES`):
  1. `indexer_lease_is_taken_when_no_row_exists` (`Taken`; `indexer_lease` shows this box, its hostname, `live`);
  2. `indexer_lease_held_by_another_owner_is_refused_with_its_holder` (`Held` naming the box);
  3. `indexer_lease_taken_again_by_its_owner_is_ours_and_renewed`;
  4. `an_expired_indexer_lease_is_taken_by_another_owner` (zero-ish TTL / seam clock as the run-lease cases do);
  5. `renewing_an_indexer_lease_another_owner_took_answers_false`;
  6. `releasing_frees_every_lease_of_the_owner_and_only_those` (count returned; another owner's row untouched;
     `live` false after);
  7. `indexer_leases_of_two_collections_are_independent`;
  8. `an_out_of_range_indexer_lease_ttl_is_refused` (`lease_ttl_micros`).
  Unit tests for `IndexedCollection::name`/`parse` round trip (`htui_ws_` + 32 hex; `htui_global`; garbage → None).
  MemStore: a `HashMap<String, MemIndexerLease>` in `State` (owner, box, expiry, pass_requested); `box_name` from the
  box row's hostname; store clock `self.now()`.
- **Mirror**: `WorkerLogStore` (trait, re-export, Mem impl, case list); run-lease cases `conformance.rs:7980, 8120,
  8297` for clock handling.
- **Pins**: `mem_store.rs` gains `mem_store_indexer_lease_conformance` and a pin `INDEXER_LEASE_CASES.len() == 8`;
  `case_names_are_unique` covers the new list. `CASES` stays 324.
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features -E 'binary(it) and
  test(/^mem_store::/)'`; `cargo nextest run -p htui-core --all-features --lib -E 'test(/indexer/)'`;
  `cargo clippy -p htui-core --all-features --all-targets -- -D warnings`; `cargo clippy -p htui-core -- -D warnings`;
  `cargo fmt --check`.

### T2: Postgres lease table, PgStore impl, Backend forwarding (tests first)

- **Action**: D2; `impl IndexerLeaseStore for PgStore` in `pg/indexer_lease.rs` (child module of `pg`):
  - take: `INSERT INTO indexer_lease (collection, lease_owner, lease_box_id, lease_expires_at) VALUES ($1,$2,$3,
    clock_timestamp()+…) ON CONFLICT (collection) DO UPDATE SET lease_owner=EXCLUDED.lease_owner,
    lease_box_id=EXCLUDED.lease_box_id, lease_expires_at=EXCLUDED.lease_expires_at WHERE indexer_lease.lease_owner IS
    NULL OR indexer_lease.lease_owner = EXCLUDED.lease_owner OR indexer_lease.lease_expires_at IS NULL OR
    indexer_lease.lease_expires_at <= clock_timestamp() RETURNING (xmax = 0) AS inserted, …` — or two statements;
    distinguish `Taken` vs `Ours` by the previous owner (a CTE reading the old row `FOR UPDATE` is fine). No row
    returned → read the holder (`LEFT JOIN box ON box.id = lease_box_id`) and answer `Held`.
  - renew: `UPDATE … SET lease_expires_at = clock_timestamp()+… WHERE collection=$1 AND lease_owner=$2` → rows==1.
  - release: `UPDATE … SET lease_owner=NULL, lease_expires_at=clock_timestamp() WHERE lease_owner=$1` → rows.
  - read: `SELECT … , (lease_owner IS NOT NULL AND lease_expires_at > clock_timestamp()) AS live … LEFT JOIN box`.
  `impl IndexerLeaseStore for Backend` (D12). Migration pins: `TABLES` + `"indexer_lease"` (57), applied versions
  `1..=36`, `Pending(35)` → `Pending(36)` at `migrations.rs:4266, 4367, 4376, 4399` and `connect.rs:140, 241` (verify
  each is the "one pending" shape before moving it), the stale "55 tables" doc comment corrected. `pg_conformance.rs`
  gains `pg_store_indexer_lease_conformance` pinning 8. A migration test: the table exists, the CHECK refuses
  `htui_concepts_v2`. `.sqlx`: `cargo sqlx prepare -- --all-targets --all-features` from `crates/htui-store`
  against a migrated scratch DB (sandbox: `docs/hr-sandbox.md` recipe, `localhost:5439`).
- **Mirror**: `pg/worker_log.rs`; run-lease SQL; `0019_stop_runs.sql` header for "not mirrored".
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and
  (test(/^pg_conformance::/) or test(/^migrations::/) or test(/^connect::/))'`; `cargo sqlx prepare --check` (from
  `crates/htui-store`, `-- --all-targets --all-features`); clippy (`--all-features --all-targets` and featureless) and
  fmt on `htui-store`.

### T3: Collection names, guarded reconcile, orphan confirmation (tests first)

- **Action**: D1 (store side), D6. `vector.rs`: `COLLECTION` → `LEGACY_COLLECTION`; `QdrantStore::connect`/
  `open_existing` (the un-`_to` forms) and `ExistingCollection::find` either take an `&IndexedCollection` or are
  removed in favour of the `_to`/`find_in` forms with `collection.name()` (pick one; no default collection remains).
  Add `pub async fn workspace_collections(settings) -> Result<Vec<IndexedCollection>, StoreError>` (Qdrant
  `list_collections`, parsed by `IndexedCollection::parse`, `Global` excluded) and `pub async fn
  drop_collection_named(settings, name) -> Result<bool, StoreError>` (true when it existed). `vector_sync.rs`: D6.
  Tests over `MemStore` + `MemVectorStore`:
  - a guard answering `false` leaves every point in place and sets `abandoned` (seed points, delete an item, sync);
  - the guard is asked once per project that has something to delete, and not at all on a pass with nothing to delete;
  - an item created after the listing (simulate: point for an item the first listing misses but `read.item` finds —
    a `ReadStore` wrapper whose `items` listing omits one id) keeps its points;
  - same for a requirement, and for an owner document that became the newest after the listing;
  - a deleted item's points still go (existing behaviour) — the existing sync tests keep passing with `&Unguarded`.
  `qdrant_live.rs` call sites pass `&Unguarded`; `qdrant_live.rs`: rename uses; add
  `live_qdrant_workspace_collections_lists_only_workspace_collections` (compiled, **not run** in the sandbox).
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features --lib -E
  'test(/vector/)'`; `cargo test -p htui-store --all-features --test it --no-run`;
  clippy (both forms) and fmt on `htui-store`. `htui` will not compile until T4 — T3 does **not** touch
  `crates/htui`; T4 starts from T3's API.

### T4: Per-collection index connections and every search path (tests first)

- **Action**: D9, D11, D12's reply shape. `concepts_worker.rs`:
  - `QdrantIndex` loses its `collection` field (`with_collection` goes); `Connections` caches per collection name
    (a map keyed by name; same `same_settings` rule; `forget`/`clear` per entry / all).
    `connect(&self, settings, collection: &IndexedCollection)`; `open_read_only(settings, collection)`.
  - `ConceptIndex`: `search(&self, within: SearchIn, query) -> BoxFuture<Result<Vec<Hit>, String>>`;
    `sync(&self, backend: Backend, collection: IndexedCollection, scope: Scope, guard: Box<dyn DeleteGuardDyn>)` —
    or keep `sync` lease-free and add the guard as an object-safe wrapper; implementer's choice, documented. All six
    implementors updated (`QdrantIndex`, `MemIndex`, `Recording`, `mcp_search::Unoffered`, `store_worker::Publishes`,
    `UrlChanges`).
  - `MemIndex` holds `HashMap<String, MemVectorStore>` per collection; `store(&IndexedCollection)` accessor; `seed`
    takes the collection.
  - Merge/dedupe helper `merge_hits(Vec<Vec<Hit>>, limit) -> Vec<Hit>` with unit tests (score order, dedupe, limit).
  - `ConceptsRuntime::serve`: `SearchConcepts { query, scope_row_of, workspace }` → `SearchIn::Workspaces([ws])`;
    `IndexConcepts { scope, scope_row_of }`: D12 (lease via `IndexerLeaseStore for Backend`, owner token field on
    `ConceptsRuntime`, box id from the backend), reply `Indexed(Result<IndexOutcome, String>)`.
  - `check()` probes `htui_global`.
  `store_worker.rs`: request field; reply type. Overlay: sends `workspace`; renders `IndexOutcome::HeldBy` as the dim
  line of D12 (snapshot); `Synced` as today. `mcp_search.rs`: `SearchIn::Everywhere`.
  Tests: a search over two workspaces' `MemIndex` collections finds each workspace's own item only when that
  workspace is named; `Everywhere` finds both, a project in two workspaces once; a Ctrl+R with the lease held by
  another owner (seed via `IndexerLeaseStore` on the harness `MemStore`) replies `HeldBy` and syncs nothing; a free
  lease → `Synced` and the lease is released after; existing overlay/mcp tests adapted, not deleted.
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features --lib -E
  'test(/concepts_worker|concepts_search|mcp_search|store_worker/)'`; `cargo insta test -p htui --all-features
  --lib` for moved snapshots (review each); `cargo test -p htui --all-features --test it --no-run`; clippy (both
  forms) and fmt on `htui`.

### T5: Lease-run index job in worker and TUI, CLI per-workspace search and index (tests first)

- **Action**: D7, D8, D9 (CLI), D10, D12 (CLI). `concepts.rs`: `IndexJob` (spawn/stop), beat loop over
  `IndexSource + IndexerLeaseStore`, legacy drop under `Global`, `index_items` per workspace with take/release and the
  held message, `search_online` per workspace with `unavailable`. `worker_cmd.rs`: `IndexJob`, `stop()` after the
  worker returns, `start_check` on `htui_global`. `ConceptsRuntime::with_index_job()` + `spawn_hosted` wiring;
  `shutdown` stops it. Docs: `docs/htui-cli.md` (`--index-items` held message, `--search-items` `unavailable`),
  `docs/htui-worker.md` (index job under the lease, every collection, beat = lease refresh).
  Tests (MemStore + `MemVectorStore` per collection; paused time where no real child runs):
  - two jobs with different owners over one store: exactly one indexes each collection; after the holder stops
    (release), the other takes over on its next beat;
  - a holder whose lease was taken over mid-sync (guard → false) deletes nothing and stops holding;
  - a job with no URL never takes a lease and loads no model;
  - the job syncs a collection at most once per `concepts_sync_minutes` while renewing every beat;
  - the `Global` holder drops the legacy collection once (live case only: compile it);
  - `search_projects`/scopes per workspace: `--cwd` maps to every workspace of the project; JSON `unavailable` names
    a workspace with no collection and is absent when empty;
  - `qdrant_worker.rs` / `state_cli_pg.rs` live cases renamed to per-workspace collections (compiled, **not run**).
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features --lib -E
  'test(/concepts/)'`; `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and
  (test(/^state_cli/) or test(/^worker/))'`; `cargo test -p htui --all-features --test it --no-run`; clippy (both
  forms) and fmt on `htui`.

## Validation

```bash
# main thread, once after the lane (sandbox):
env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh
# owed on the host (sandbox Qdrant times out creating collections):
#   htui-store qdrant_live, htui qdrant_worker, state_cli_pg's index/search cases,
#   concepts_worker live lib cases, plus the new live_qdrant_* cases
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A long first build outlives the TTL and another process takes over mid-sync | Medium | Guard before each delete batch → abandon; upserts are idempotent; the new holder's pass converges |
| Every TUI contends and loads the 133 MB model | Medium | Model loads only to sync a held collection; a TUI without URL or without the lease loads nothing |
| Merged hits from two collections: RRF scores are only comparable within one search | Low | Global holds no points in M1; multi-workspace merge was one fused search before — note it in the doc comment; MOD-162 can measure |
| Deleted workspace leaves its collection and lease row | Low | Deferred (no FK by design); listed in the write-up as a follow-up |
| Migration number collides at collect | High | `0036`; renumber at `scripts/hr collect` with its pins |
| Snapshot churn in the overlay | Low | Review each moved snapshot |
| `htui` does not compile between T3 and T4 | Certain | Serial lane; T3's check is crate-scoped to `htui-store` (`htui-bench` only uses `connect_to`, unchanged) |

## Acceptance

- [ ] All tasks complete, each with its own commits
- [ ] Sandbox gate green (`PASSED (1 skipped: qdrant)`)
- [ ] Patterns mirrored, not reinvented
- [ ] Live-Qdrant suites listed as owed on the host

## Verified claims (step 3.5 fact-check)

Inline against the tree, 2026-11-14 (`hr/MOD-159` at `cd65d461`).

| Claim | Verdict | Evidence |
|---|---|---|
| `htui` depends on `htui-orch`, `LeaseTimes::from_app` is public | true | `crates/htui/Cargo.toml:31`; `htui-orch/src/lib.rs:51, 84`; `recover.rs:46, 64` |
| `lease_ttl_micros` is public and re-exported | true | `traits.rs:3636`; `store/mod.rs:39` |
| `qdrant-client` 1.19 has `list_collections` | true | `qdrant_client/collection.rs:121` in the registry copy |
| `ReadStore` has `item(id)`, `requirement(id)`, `document(id)`, `owner_documents(project)` | true | `traits.rs:278` (`requirement`); `item`/`document`/`owner_documents` are called by `Indexer::sync` today |
| `Pending(35)` pins at `migrations.rs:4266, 4367, 4376, 4399` and `connect.rs:140, 241` | true | grep |
| Latest migration `0035`, `TABLES` pinned with `.len()` at `migrations.rs:164, 182` | true | `ls migrations`; grep |
| `StoreError::Unreachable` exists | true | `store/error.rs:43` |
| `PgStore::this_box()` exists | true | `pg/mod.rs:765` (`pub const fn`) |
| MemStore exposes `this_box()` | **false** → amended | only `State.this_box` (`mem.rs:247`) via `MemStore::box_info()` (`mem.rs:576`); D12 amended |
| `htui-bench` calls `Indexer::sync` or names `COLLECTION` | **false** → amended | only the literal `"htui_concepts_v2"` in a refusal list (`run.rs:764`); removed from T3 |
| `Indexer::sync` production callers are `concepts.rs:265`, `concepts_worker.rs:855, 1065` | true | grep; tests in `qdrant_live.rs` (T3) |
| `COLLECTION` production users: `concepts.rs:35, 548`, `concepts_worker.rs:27, 707`, `worker_cmd.rs:164`; tests `concepts_worker.rs:2914, 2927` | true | grep |
| `ConceptIndex` has six implementors | true | `QdrantIndex`, `MemIndex`, `Recording` (`concepts_worker.rs`), `Unoffered` (`mcp_search.rs:354`), `Publishes`, `UrlChanges` (`store_worker.rs:4303, 4413`) |
| `--search-items` JSON shape lives in `state_cmd.rs` (`SearchAnswer`, `IndexAnswer`) | true | `state_cmd.rs:579, 590`; snapshot `search_index_empty` |
| Run-lease conformance cases at `conformance.rs:7980, 8120, 8297`; `case_names_are_unique` | true | `conformance.rs:35794` |
| Task independence | n/a | one serial lane; T4/T5 overlap (`concepts_worker.rs`, `store_worker.rs`) is serial by construction |

