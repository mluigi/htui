# Plan: MOD-159 M2 — Freshness

**Source PRD**: `.claude/prds/mod-159-knowledge-indexer.prd.md`
**Selected Milestone**: 2 — Freshness
**Complexity**: Medium
**Status**: complete (`b4509a9f..d9020fcf`, review fix round R1 `d9020fcf`)
**Run**: `/handoff-run MOD-159` in an hr sandbox (`hr/MOD-159`); CONFIRM waived by `/handoff-run`. Lane shape: one
serial lane T1 → T2 → T3 → T4 (T2 and T3 have disjoint file sets and may run side by side after T1, but only in
separate worktrees: both build `htui-store` with `SQLX_OFFLINE=true`, so T2's new `sqlx::query!` breaks T3's builds
on a shared tree until T2's two `.sqlx` entries exist; see C49). Verifier on T1 (foundation) only. No migration.

## Summary

M1 left the index job reconciling each held collection in full once per `concepts_sync_minutes` (15). M2 adds an
**incremental pass every 60 s** between full reconciles: the job asks the store which items, requirements, item
documents and owner documents were written since its last pass (pulled back by `cache_overlap_seconds`, 300 s, as the
cache refresh is), and rebuilds only those rows' points. The "since" watermark is a store-clock instant kept in
process, per collection; a process that takes a lease over starts with a full reconcile (M1's behaviour) and only
then runs incremental passes. Deletes of whole rows stay with the full reconcile; an incremental pass deletes only the
stale points of the rows it rebuilds, under M1's `DeleteGuard`. The incremental pass reuses the key the last full
reconcile read, so a 60 s cycle never touches the keyring. Pass requests and release on exit (M3) are out of scope.

## Decisions taken in this plan

- **D1 Watermark in process, per collection.** Not on the lease row: a new holder must reconcile in full anyway
  (it cannot know what the old holder's pass missed, and M1 already runs a full reconcile on every take: `JobState.held`
  has no entry for a newly taken collection, so it is due). Keeping it in process needs no migration, no `.sqlx` for a
  cursor column, and no write per cycle. Losing it (restart, lease lost, abandon, settings unresolved, key failure, URL
  change) costs one full reconcile, which those events already trigger.
- **D2 The cursor seam.** `htui_core::model::IndexCursor { pub rows: DateTime<Utc> }` (in `model/indexer.rs`,
  re-exported from `model`): the store-clock instant the last completed pass of that collection began. One field per
  source: MOD-112 (4) adds `transcripts: Option<(StepId, i64)>` (`run_step_id`, `seq`) beside `rows`, advanced by its
  own read; nothing else in this plan names the rows source, so that addition touches only the struct, its read and
  the job's advance. Constructor `IndexCursor::at(rows: DateTime<Utc>) -> Self`. Derives `Debug, Clone, Copy,
  PartialEq, Eq`.
- **D3 Change read: a narrow trait** `htui_core::store::IndexChangesStore` (new file `store/index_changes.rs`,
  re-exported from `store/mod.rs` beside `IndexerLeaseStore`), implemented by `MemStore` and `PgStore` only (as M1's
  D3; not `ReadStore`, which five types implement, and not `ItemFilter`, which is serialised and read by the mirror):
  ```rust
  pub trait IndexChangesStore: Send + Sync {
      /// The store's clock now (Postgres `clock_timestamp()`, `MemStore::now()`).
      fn index_clock(&self) -> impl Future<Output = Result<DateTime<Utc>>> + Send;
      /// Rows of `projects` written strictly after `since`, by project.
      fn index_changes(&self, projects: &[ProjectId], since: DateTime<Utc>)
          -> impl Future<Output = Result<IndexChanges>> + Send;
  }
  ```
  Model types in `model/indexer.rs`:
  ```rust
  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub struct IndexChanges { pub projects: BTreeMap<ProjectId, ProjectChanges> }   // only projects with a change
  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub struct ProjectChanges {
      pub items: BTreeSet<ItemId>,               // updated_at > since, or a document version created_at > since
      pub requirements: BTreeSet<RequirementId>, // updated_at > since
      pub owner_documents: BTreeSet<DocumentId>, // item_id IS NULL, created_at > since
  }
  impl IndexChanges { pub fn is_empty(&self) -> bool }
  ```
  An item is listed under its **current** project (an item document joins `item`, not `document.project_id`; the two
  agree anyway, `fk_document_item_project` (`0026_workspace_scope.sql:74-75`) pins `(item_id, project_id)` to the
  item's, and no write moves an item between projects today, C40). An item status move, close-out or edit lands in
  `items` because every `UPDATE` of `item` stamps `updated_at` (`set_updated_at` trigger in Postgres; `MemStore`
  stamps it in each write). A new document version does **not** touch its item's `updated_at` on either store (C35),
  hence the document arm. Rows written at exactly `since` are out (strict `>`), as the cache cursor's are.
  **Document stamps are the writer's, not the store's** (C34): `document.created_at` is `NewDocument.created_at` /
  `NewOwnerDocument.created_at` on both stores (`mem.rs:8401, 8433`; `pg/write.rs:962-993` binds `$8`), and the
  product's writers fill it with their own `Utc::now()` (`hand_written.rs:195`, `owner_documents.rs:122`,
  `run_worker.rs:727`). Item and requirement stamps are the store's (`now()` at insert, `clock_timestamp()` per
  update). So the document arms compare a writer-clock stamp with a store-clock `since`: a writer whose clock lags
  the database's by more than the overlap is caught only by the full reconcile, the exposure the cache cursor already
  has on `document` (`cache/refresh.rs:1072-1117`). Accepted for M2 (Risks).
- **D4 Postgres SQL** (`htui-store/src/pg/index_changes.rs`, child of `pg` as `indexer_lease.rs` is; macros, so two
  new `.sqlx` entries):
  ```sql
  SELECT clock_timestamp() AS "now!"
  ```
  ```sql
  SELECT 'item' AS "kind!", i.project_id AS "project_id!", i.id AS "id!"
    FROM item i WHERE i.project_id = ANY($1) AND i.updated_at > $2
  UNION
  SELECT 'item', i.project_id, i.id
    FROM document d JOIN item i ON i.id = d.item_id
   WHERE i.project_id = ANY($1) AND d.created_at > $2
  UNION
  SELECT 'requirement', r.project_id, r.id
    FROM requirement r WHERE r.project_id = ANY($1) AND r.updated_at > $2
  UNION
  SELECT 'owner_document', d.project_id, d.id
    FROM document d WHERE d.item_id IS NULL AND d.project_id = ANY($1) AND d.created_at > $2
  ```
  `$1` binds `Vec<Uuid>` of the project IDs. `item` and `requirement` ride the cache cursor's
  `(project_id, updated_at)` indexes; `document` has no `created_at` index (see Risks). An unknown `kind` is a
  `StoreError::Backend` (cannot happen; no `unreachable!`).
- **D5 Overlap and clock.** `since = cursor.rows - overlap`, overlap = `cache_overlap_seconds` from the `app_setting`
  map the beat already reads (`IndexSource::index_settings`), parsed by a new `overlap_window(app) -> Duration`
  in `concepts.rs`: a positive JSON integer of seconds, otherwise 300 (the rule of `connect.rs::refresh_settings`).
  Key constant `OVERLAP_KEY = "cache_overlap_seconds"`, default `DEFAULT_OVERLAP_SECONDS: u64 = 300`. The watermark
  is read **before** the pass's read (`index_clock()`), so a row committed during the pass is re-read next cycle;
  a commit that lands later than its stamp by less than the overlap is still caught, as is a document whose writer's
  clock lags the database's by less than the overlap (D3, C34); the full reconcile is the backstop.
- **D6 Cycle: a constant, not a setting.** `pub const INCREMENTAL_CYCLE: Duration = Duration::from_secs(60)` in
  `concepts.rs` (PRD: 60 s, ANA-37 Q4; no `app_setting` key exists or is added, so no seed, migration or count pin
  moves). Revisit with MOD-46 (NOTIFY).
- **D7 Incremental sync in the indexer.** `htui_store::vector_sync::Indexer::sync_changes`:
  ```rust
  pub async fn sync_changes(read: &impl ReadStore, scope: &Scope, changes: &IndexChanges,
                            store: &impl VectorStore, guard: &impl DeleteGuard)
      -> Result<SyncReport, StoreError>
  ```
  For each project of `scope.project_ids` (scope order) that has an entry in `changes`: one `store.indexed(project)`
  read, grouped by owner as `sync` does; then
  - each listed item: `read.item(id)`; gone, or in another project than the one it is listed under → skipped (its
    own project's entry, or the full reconcile, settles it; no write moves an item today, C40, so this guards a
    hand-built or stale `IndexChanges`); otherwise the same freshness test as `sync` (item point's `updated_at` and `status`,
    latest-document set) and, when stale, the same rebuild; its off-ID old points join `doomed.stale`;
  - each listed requirement: `read.requirement(id)`; gone or in another project → skipped; otherwise rebuilt when the
    point's `version` differs; off-ID old points stale; upserts in `UPSERT_BATCH` chunks;
  - any listed owner document: `sync_owner_documents` for the project as today, then its orphans
    (`doomed.documents`) are dropped: only superseded (listed, not newest) points stay doomed;
  - no item or requirement orphans are collected: deleting a row's points is the full reconcile's;
  - the batch is `doomed.stale` only (nothing to confirm); when not empty, `guard.before_delete()`, `false` →
    `abandoned`, return; as `sync`.
  The item and requirement rebuild bodies move into private helpers shared by `sync` and `sync_changes`
  (behaviour-preserving: every existing `vector_sync` test passes unchanged).
- **D8 The job's state** (`concepts.rs`). `JobState.held` becomes `HashMap<IndexedCollection, Held>` with
  ```rust
  struct Held {
      reconciled: tokio::time::Instant,      // when the last full reconcile started (M1's value)
      passed: tokio::time::Instant,          // when the last pass, full or incremental, started
      cursor: Option<IndexCursor>,           // None until a full reconcile completes (not abandoned, no error)
  }
  ```
  plus `JobState.keyed: Option<QdrantSettings>` (the settings, key included, the last due full reconcile resolved;
  `QdrantSettings`'s `Debug` redacts the key) and `JobState.incremental_failed: HashSet<IndexedCollection>`
  (log-once set, see D9). `JobState::held()` keeps its signature. Every place M1 forgets a held collection also
  forgets its cursor (same map). `keyed` is cleared where M1 calls `index.forget(None)` / `forget_synced()`, and
  replaced each time `with_key` succeeds.
- **D9 The beat** (`index_beat`), per held collection after the take (M1 order unchanged):
  1. full reconcile due (no entry, or `reconciled` older than `sync_interval`) → M1's path, preceded by
     `as_of = source.index_clock()`; a completed, non-abandoned sync sets `cursor = IndexCursor::at(as_of)` and
     `passed = now`. M1's `reconcile` returns `()` and logs its outcome inside (`concepts.rs:844-912`), so it gains a
     return value saying whether the workspace sync completed un-abandoned (C43); an open failure, a sync error, an
     abandon and the global collection (no scope) set no cursor. A failed clock read skips the collection this beat
     (entry removed, so it is due next beat).
  2. else incremental due (`cursor` is `Some` and `passed.elapsed() >= INCREMENTAL_CYCLE`) and the collection has a
     scope (not `Global`) → `as_of = index_clock()`; `changes = index_changes(&scope.project_ids, cursor.rows -
     overlap)`; `passed = now`. Empty → `cursor = at(as_of)`, nothing opened (no key, no model, no Qdrant call).
     Otherwise open the store with `state.keyed` (when `None`, `with_key` once, as a due reconcile does, honouring
     `key_failed_at`), then `Indexer::sync_changes` under `Renewing`, inside `while_renewing` as the reconcile is.
     Success → `cursor = at(as_of)`, `incremental_failed.remove`; abandoned → forget the collection (info line, as
     M1); open or sync failure → `index.forget(Some(&store))` on a sync failure, cursor kept (same window retried next
     cycle), logged at `warn` the first time per collection and `debug` after until a pass succeeds.
  3. A URL that differs from `state.keyed`'s → `keyed = None` and every held entry is dropped, so each collection
     reconciles in full now against the new Qdrant (it may be empty).
  The beat's sleep becomes `min(times.refresh, max(time until the earliest held collection's incremental is due,
  1 s))`: only the due term is floored, since `LeaseTimes::refresh` may itself be under 1 s (`ttl / 3` in ms, e.g.
  333 ms for a 1 s TTL, `htui-orch/src/recover.rs:64-82`) and must not be lengthened (C45). With the default 40 s
  refresh a collection's pass runs every 60 s, not every 80 s.
- **D10 `IndexSource` gains the change read as a supertrait**: `pub trait IndexSource: ReadStore +
  IndexChangesStore`. `index_beat`, `index_job`, `reconcile` and `IndexJob` keep their bounds. `index_under_leases`
  (`--index-items`) and `Ctrl+R` stay full passes (M3's).
- **D11 Docs.** `docs/htui-worker.md`'s index-job bullets gain the incremental cycle (60 s, the overlap, what it does
  not delete, no keyring read). No CLI change.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store trait | `crates/htui-core/src/store/indexer_lease.rs:1-104`; re-export `store/mod.rs:9, 25` | `fn … -> impl Future<Output = Result<T>> + Send`, impls write `async fn`; module doc says why it is narrow |
| Mem impl | `crates/htui-core/src/store/mem.rs:12455-12540` (`impl IndexerLeaseStore for MemStore`) | `self.now()` read first, then `self.read(|state| …)` |
| Pg impl, child module | `crates/htui-store/src/pg/indexer_lease.rs:1-120`; `pg/mod.rs:10` | `sqlx::query!` macros, `map_sqlx`, server clock only |
| Own case list | `crates/htui-core/src/store/conformance.rs:3051-3143` (`INDEXER_LEASE_CASES`, `run_indexer_lease_case(_inline)`, `run_all_indexer_leases`); `case_names_are_unique` `:37740-37769`; `run_indexer_lease_case_accepts_every_name` `:37944` | `const` name list, boxed arms, unknown name panics |
| Case pins | `crates/htui-core/tests/it/mem_store.rs:42-45, 135-140`; `crates/htui-store/tests/it/pg_conformance.rs:223-245` | one test per list per store, count pinned with a "why" message |
| Cursor + overlap | `crates/htui-store/src/cache/refresh.rs:223-226, 347-352` (`since`), `882-934` (`refresh_item`), `1072-1117` (`refresh_document`), `1661-1710` (`refresh_requirement`) | `ts > high_water - overlap`, server time |
| Setting parse | `crates/htui/src/concepts.rs:461-477` (`SYNC_MINUTES_KEY`, `sync_interval`); `crates/htui-store/src/connect.rs:643-674` | positive integer or default, re-read each beat |
| Job beat | `crates/htui/src/concepts.rs:588-760` (`JobState`, `index_beat`), `763-790` (`with_key`), `804-841` (`while_renewing`), `844-912` (`reconcile`) | log once, forget on failure, next beat is the retry |
| Guarded deletes | `crates/htui-store/src/vector_sync.rs:82-178` (`DeleteGuard`, `Unguarded`, `Doomed`), `193-268` (`Indexer::sync`) | one batch per project, guard right before it |
| Tests | `vector_sync.rs:1298-1330` (`CountingGuard`), `1421-1600` (`Unlisted`); `concepts.rs:2232-2530` (`mod index_job`: `TokioClock`, `Counting`, `Fakes`, `spawn_job`), `2834` (`beats_with_nothing_due_read_no_key`) | `#[tokio::test(start_paused = true)]`, sentence names |

## Files to Change

| Task | Files (owned) |
|---|---|
| T1 | `crates/htui-core/src/model/indexer.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/index_changes.rs` (new), `crates/htui-core/src/store/mod.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/it/mem_store.rs` |
| T2 | `crates/htui-store/src/pg/index_changes.rs` (new), `crates/htui-store/src/pg/mod.rs`, `crates/htui-store/.sqlx/query-*.json` (two new), `crates/htui-store/tests/it/pg_conformance.rs` |
| T3 | `crates/htui-store/src/vector_sync.rs` |
| T4 | `crates/htui/src/concepts.rs`, `docs/htui-worker.md` |

Intersections: T2 ∩ T3 = ∅ (both `htui-store`, different files; T2 does not touch `vector_sync.rs`, T3 not `pg/`).
Build coupling: both compile `htui-store` under `SQLX_OFFLINE=true` (`.cargo/config.toml`), so on one tree T2's
`query!` without its `.sqlx` entries fails T3's builds; side by side only in separate worktrees, else T2 first (C49).
T4 depends on T1, T2 (`PgStore: IndexChangesStore` for the supertrait) and T3.

## Tasks

### T1: Change-read model, narrow trait, MemStore, conformance cases (foundation; tests first)

- **Depends on**: nothing.
- **Action**: D2, D3. Add `IndexCursor`, `IndexChanges`, `ProjectChanges` to `model/indexer.rs` (update its module
  doc: "and what changed since a pass, MOD-159 M2") and re-export them at `model/mod.rs:144`. New
  `store/index_changes.rs` with `IndexChangesStore` (module doc mirrors `indexer_lease.rs`'s: why narrow, two
  implementors, store clock, strict `>`); `mod index_changes;` + `pub use index_changes::IndexChangesStore;` in
  `store/mod.rs`. `impl IndexChangesStore for MemStore` beside `impl IndexerLeaseStore for MemStore`:
  `index_clock` = `self.now()`; `index_changes` reads `State.items` (`updated_at`), `State.documents` (item's current
  `project_id`), `State.requirements` (`updated_at`), `State.owner_documents` (`created_at`).
- **Tests first** — new list `INDEX_CHANGES_CASES` in `conformance.rs` with `run_index_changes_case(_inline)` and
  `run_all_index_changes` bound `S: WriteStore + IndexChangesStore`, a fresh store per case (`MemStore::demo()` /
  `demo_db`), each `since` taken from `index_clock()` (never the test's clock). Document rows carry the caller's
  `created_at` on both stores (C34), so cases 3, 5 and 6 stamp a document to be listed `since + 1 s` and one to be
  left out `since - 1 s`, never `Utc::now()` (the Postgres server's clock is not the test's):
  1. `index_changes_list_an_item_minted_or_edited_after_since` (`mint_item`, then `update_item`; a `since` read after
     the edit lists nothing);
  2. `index_changes_list_an_item_whose_status_moved` (`transition`);
  3. `index_changes_list_an_item_that_gained_a_document_version` (`write_document`; listed in `items`, not in
     `owner_documents`);
  4. `index_changes_list_a_requirement_minted_or_amended_after_since` (`mint_requirement`, `amend_requirement`);
  5. `index_changes_list_an_owner_document_of_a_project_or_a_scope_row` (`write_owner_document`, both owners);
  6. `index_changes_leave_out_rows_before_since_and_projects_not_asked` (a write in another project; an empty
     `projects` slice answers empty);
  7. `the_index_clock_brackets_a_write` (`before <= row.updated_at <= after`, from the returned row).
  `case_names_are_unique` (`conformance.rs:37741-37769`) already chains five lists (`CASES`, `READ_CASES`,
  `WORKER_LOG_CASES`, `ACTIVITY_CASES`, `INDEXER_LEASE_CASES`) while its comment and message still say "four"
  (C41): chain `INDEX_CHANGES_CASES` as the sixth and make both say "six lists"; add
  `run_index_changes_case_accepts_every_name` beside `run_indexer_lease_case_accepts_every_name`. Unit test in
  `model/indexer.rs`: `IndexChanges::is_empty` on a default and on one entry.
- **Pins**: `mem_store.rs` gains `mem_store_index_changes_conformance` (`run_all_index_changes(|| async {
  MemStore::demo() })`) and, in `demo_store_loads_the_fixture`, `INDEX_CHANGES_CASES.len() == 7` with a "MOD-159
  M2's seven change-read cases (plan D3)" message. `CASES` (325), `READ_CASES` (15), `INDEXER_LEASE_CASES` (10)
  unchanged.
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features -E 'binary(it) and
  test(/^mem_store::/)'`; `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features --lib -E
  'test(/conformance::tests::|model::indexer::/)'`; `cargo clippy -p htui-core --all-features --all-targets -- -D
  warnings`; `cargo clippy -p htui-core -- -D warnings`; `cargo fmt -p htui-core --check`.

### T2: PgStore change read (tests first)

- **Depends on**: T1.
- **Action**: D4. `pg/index_changes.rs` (module doc: the cache cursor's predicates, server clock, why `UNION`),
  `mod index_changes;` in `pg/mod.rs` beside `mod indexer_lease;`. `index_changes` with an empty `projects` returns
  `IndexChanges::default()` without a query. Rows fold into the `BTreeMap`/`BTreeSet`s by `kind`.
- **Tests first**: `pg_conformance.rs` gains `pg_store_index_changes_conformance` (`#[cfg(feature = "demo")]`,
  one `demo_db` per case, boxed, pin `INDEX_CHANGES_CASES.len() == 7` "as `mem_store.rs` pins them"), mirroring
  `pg_store_indexer_lease_conformance`.
- **`.sqlx`**: two new entries. Recipe `docs/hr-sandbox.md` "Changing SQL queries in a run": scratch DB on
  `localhost:5439`, `cd crates/htui-store`, `DATABASE_URL=postgres://postgres@localhost:5439/<scratch>`,
  `cargo sqlx migrate run --source migrations`, `cargo sqlx prepare -- --all-targets --all-features`; commit only the
  new files (a pre-existing entry that changes means the SQL is not byte-identical to what was intended).
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and
  test(/^pg_conformance::/)'` (with the sandbox's test DSN, `docs/hr-sandbox.md`); `cargo sqlx prepare --check --
  --all-targets --all-features` from `crates/htui-store`; `cargo clippy -p htui-store --all-features --all-targets --
  -D warnings`; `cargo clippy -p htui-store -- -D warnings`; `cargo fmt -p htui-store --check`.

### T3: `Indexer::sync_changes` (tests first)

- **Depends on**: T1. Independent of T2 (disjoint files; needs nothing T2 writes: its tests use `MemStore` and
  T1's `MemStore::index_changes`). Shares `htui-store`'s offline build with T2: separate worktree, or after T2 (C49).
- **Action**: D7. Update the module doc (an incremental pass beside the reconcile; what it never deletes).
  Extract the item rebuild (freshness test + rebuild + stale points) and the requirement rebuild into private helpers
  used by both `sync` and `sync_changes`; `sync`'s behaviour and its `SyncReport` counts unchanged.
- **Tests first** (unit tests in `vector_sync.rs`'s `mod tests`, over `MemStore` + `MemVectorStore`; the
  `IndexChanges` built by hand or by `MemStore::index_changes`):
  - `sync_changes_indexes_a_new_item_and_its_documents`;
  - `sync_changes_rebuilds_only_the_listed_items` (an unlisted stale item keeps its old point);
  - `a_new_document_version_reaches_the_index_through_its_item` (old version's sections deleted, guard asked once);
  - `sync_changes_rebuilds_an_amended_requirement`;
  - `sync_changes_replaces_a_superseded_owner_document`;
  - `sync_changes_leaves_a_deleted_rows_points_to_the_full_reconcile` (item gone from the store: its points stay,
    the guard is not asked);
  - `sync_changes_skips_an_item_listed_under_a_project_it_left`;
  - `a_refused_guard_stops_sync_changes_with_nothing_deleted` (`CountingGuard::answering(false)`, `abandoned`);
  - `after_sync_changes_a_full_sync_rebuilds_nothing` (changes from `index_changes` since before the writes; then
    `sync` reports zero rebuilt and zero deleted).
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features --lib -E
  'test(/vector_sync::/)'`; `cargo test -p htui-store --all-features --test it --no-run`; clippy (both forms) and
  `cargo fmt -p htui-store --check`.

### T4: The 60 s cycle in the index job (tests first)

- **Depends on**: T1, T2, T3.
- **Action**: D1, D5, D6, D8, D9, D10, D11 in `concepts.rs`: `INCREMENTAL_CYCLE`, `OVERLAP_KEY`,
  `DEFAULT_OVERLAP_SECONDS`, `overlap_window`, `Held`, `JobState.keyed` / `incremental_failed`, the incremental
  branch (a helper `incremental(source, index, collection, scope, holder, overlap, state)` beside `reconcile`), the
  sleep computation, `reconcile`'s new outcome (C43), `IndexSource: ReadStore + IndexChangesStore` (only `PgStore`
  and `MemStore` implement `IndexSource`, both covered by T1/T2; no test wrapper implements it, C38).
  `index_beat`'s doc comment gains the cycle.
  `docs/htui-worker.md`: a bullet after "A collection it holds is reconciled once when it is taken…": every 60 s
  between reconciles the holder indexes what was written since its last pass (pulled back by
  `cache_overlap_seconds`), reads no key for it, and leaves deleted rows to the reconcile.
- **Tests first** (in `mod index_job`, `start_paused`, demo with `TokioClock`; writes stamped in the past through
  `MemStore::handle_at`; a document's `created_at` is the caller's (C34), so a test stamps it from the store's clock
  (`read.index_clock()`, or the `handle_at` instant), never `Utc::now()`: under paused time `TokioClock` runs ahead
  of the wall clock by every advance, and a wall-clock stamp would fall before the overlap):
  - `an_item_filed_after_the_reconcile_is_indexed_within_one_cycle` (no second full sync, the new item's point
    exists after `INCREMENTAL_CYCLE`). `Counting.syncs` counts every `indexed(PROJECT_HTUI)` read, and an
    incremental pass over a change in htui makes one too (C46), so "one full sync" is told apart another way (e.g. a
    counter of the store's `items` listings, or `syncs` rising by exactly the changed projects' reads);
  - `an_amended_requirement_and_a_new_owner_document_are_indexed_within_one_cycle`;
  - `a_cycle_with_nothing_written_opens_nothing_and_reads_no_key` (keyring counter as
    `beats_with_nothing_due_read_no_key`; `Fakes::opened` unchanged across cycles);
  - `a_write_stamped_inside_the_overlap_is_still_indexed` (stamped `cursor - 100 s`) and
    `a_write_stamped_before_the_overlap_waits_for_the_reconcile` (stamped `cursor - 400 s`, found after
    `concepts_sync_minutes`);
  - `an_incremental_pass_whose_lease_was_taken_over_deletes_nothing_and_stops_holding` (`Takeover`);
  - `a_failed_incremental_pass_keeps_its_window_and_retries_next_cycle` (`Counting.fail`);
  - `a_taken_over_collection_starts_with_a_full_reconcile` (second job after the first's `stop`);
  - `a_changed_url_reconciles_every_held_collection_at_once`;
  - `the_job_wakes_for_the_cycle_between_lease_beats` (pass at 60 s with a 40 s beat);
  - `the_overlap_reads_cache_overlap_seconds_with_a_300_second_default` (unit, as
    `the_interval_reads_concepts_sync_minutes_with_a_15_minute_default`).
  Existing M1 job tests keep passing; where one pins a count that an incremental `indexed` read now moves (e.g.
  `Counting.syncs`), adapt the assertion and say why in the test, never delete it.
- **Validate**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features --lib -E
  'test(/concepts::/)'`; `cargo test -p htui --all-features --test it --no-run`; `cargo clippy -p htui --all-features
  --all-targets -- -D warnings`; `cargo clippy -p htui -- -D warnings`; `cargo fmt -p htui --check`.

## Validation

```bash
# main thread, once after the lane (sandbox):
env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh
# owed on the host: the live-Qdrant suites (unchanged by M2; no new live case)
```

Bookkeeping after the lane (not a task): `HANDOFF.md` pins line (`INDEX_CHANGES_CASES` 7 beside
`INDEXER_LEASE_CASES` 10), the PRD's M2 row (`pending` → `complete`, Plan cell → this file; this plan run did not
edit the PRD, per its brief).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A transaction commits more than 300 s after it stamped its rows | Low | Full reconcile every `concepts_sync_minutes` is the backstop (D5) |
| `document` has no `created_at` index: the change read scans `document` once a minute per held workspace | Medium (size grows) | Small today; an index `(project_id, created_at)` is a later migration if the plan of `EXPLAIN` shows it; noted in the write-up |
| A bulk write (import) makes one incremental pass long | Low | `while_renewing` keeps the leases; the guard abandons a pass that lost its lease |
| The cached key is rotated: incremental opens fail until the next reconcile reads it again | Low | Failure re-reads the key once via `with_key` when `keyed` is cleared; log once (D9) |
| M1 job tests that count `indexed` reads move | Medium | Adapt with a comment (T4) |
| `MemStore` misses an `updated_at` stamp that Postgres's trigger makes | Low | Conformance cases 1-3 pin the stamps that matter on both stores |
| A document writer's clock lags the database's by more than the overlap (`document.created_at` is the writer's `Utc::now()`, C34) | Low | Same exposure as the cache cursor's `document` fetch; the full reconcile every `concepts_sync_minutes` is the backstop; noted in the write-up |
| A full reconcile that fails leaves no cursor, so no incremental pass until the next interval | Medium | Accepted: the same retry cadence as M1; noted in the write-up |
| Demo fixture rows stamped near "now" would make the first cycle re-read them | Low | Re-reads are idempotent (no upsert when in step); claim C24 checks the stamps |

## Acceptance

- [ ] All tasks complete, each with its own commits
- [ ] Sandbox gate green (`PASSED (1 skipped: qdrant)`)
- [ ] Patterns mirrored, not reinvented
- [ ] Live-Qdrant suites listed as owed on the host

## Claims to verify (step 3.5 fact-check)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| C1 | `JobState.held` is `HashMap<IndexedCollection, tokio::time::Instant>` and a collection with no entry is due (`concepts.rs:588-617, 717-724`) | true | `concepts.rs:592` `held: HashMap<IndexedCollection, tokio::time::Instant>`; due test `:729-732` (`is_none_or(\|last\| last.elapsed() >= every)`), not `:717-724` |
| C2 | `index_beat` reads the key via `with_key` only when a collection is due, once per beat (`concepts.rs:633-760, 763-790`) | true | `keyed` is a per-beat local (`concepts.rs:704`), filled by `with_key` (`:763-790`) only after `due` (`:735-737`); a key failure releases every lease and `break`s (`:738-743`) |
| C3 | `IndexSource` is `pub trait IndexSource: ReadStore` with `index_scopes`, `index_settings`, `index_box`, implemented only for `PgStore` and `MemStore` (`concepts.rs:482-526`) | true | `concepts.rs:482` `pub trait IndexSource: ReadStore` (3 methods); text search `IndexSource for` finds only `:493` PgStore, `:507` MemStore |
| C4 | `index_settings` returns the full `app_setting` map (`PgStore::app_settings`, `MemStore::app_settings`) | true | both `index_settings` call `app_settings()`: `mem.rs:796-804` (whole map), `pg/read.rs:2082-2088` (`SELECT key, value FROM app_setting`) |
| C5 | `sync_interval` treats absent/non-positive/non-integer as 15 min (`concepts.rs:470-477`) | true | `concepts.rs:470-477`: `as_u64().filter(>0).unwrap_or(15)` (fractions, strings, 0, negatives fall back) |
| C6 | `connect.rs::refresh_settings` (`:643-674`) keeps the default for a non-positive or non-numeric overlap; default 300 (`cache/refresh.rs:30`) | true | `connect.rs:643-674`: `as_i64().filter(>0)` else keep base; `cache/refresh.rs:31` `DEFAULT_OVERLAP_SECONDS = 300` |
| C7 | `cache/refresh.rs` reads `item`/`requirement` by `updated_at > since` and `document` by `created_at > since`, `since = high_water - overlap` (`:347-352, 882-934, 1072-1117, 1661-1710`) | true | `refresh.rs:348-352` `since(hw, overlap)`; `:893` item and `:1672` requirement `updated_at > $2`; `:1086` document `created_at > $2` (per `project_id`) |
| C8 | Every `UPDATE` of `item` and `requirement` stamps `updated_at = clock_timestamp()` (`0001_init.sql:21-25, 574-580`; `0006_requirements.sql:139`) | true | `0001_init.sql:21-25` `set_updated_at` = `clock_timestamp()`; `:574-580` trigger on `item` (0024 disables it only inside its own migration, `:81/:101`); `0006_requirements.sql:139-140` |
| C9 | `MemStore`'s `transition` stamps `updated_at` (`mem.rs:2611-2632`); `update_item`, close-out and requirement amend/withdraw do too | true | `State.transition` `mem.rs:2631`, `State.update` `:2582`, `State.close_out` `:8694`, `State.revise_requirement` (amend and withdraw) `:9375`, each `updated_at = now` from `self.now()` |
| C10 | `item(project_id, updated_at)` and `requirement(project_id, updated_at)` indexes exist (`0001_init.sql:334`, `0006_requirements.sql:87`); `document` has no `created_at` index | true | `0001_init.sql:334` `idx_item_updated_at`; `0006_requirements.sql:87` `idx_requirement_updated_at`; `document` indexes are `idx_document_item(item_id, kind, version)` and `uq_document_owner_kind_version(project_id, item_id, kind, version)` (`0026:78-80`), none on `created_at` |
| C11 | `document.item_id` is NULL for owner documents and `document.project_id` is set on every row (`0026_workspace_scope.sql`) | true | `0026_workspace_scope.sql:68-73` (`project_id` NOT NULL, `item_id` nullable); `:74-75` FK `(item_id, project_id)` -> `item(id, project_id)` |
| C12 | `Document` has `item_id`, `created_at`; `OwnerDocument` has `project_id`, `created_at` (`model/document.rs:10-57`) | true | `model/document.rs:10-57` |
| C13 | `Requirement` has `project_id` and `updated_at` (`model/requirement.rs:~145`); `Item` has `project_id`, `updated_at`, `status` | true | `Requirement.updated_at` `model/requirement.rs:145`; `Item.updated_at` `model/item.rs:191` (with `project_id`, `status`) |
| C14 | ID newtypes derive `Ord` (`model/ids.rs:19-21`), so `BTreeMap<ProjectId,_>`/`BTreeSet<ItemId>` compile | true | `model/ids.rs:19-22` `id_newtype!` derives `PartialOrd, Ord, Hash`; `ProjectId`, `ItemId`, `DocumentId`, `RequirementId` declared at `:77-122`; no probe needed |
| C15 | `MemStore` keeps `items`, `documents`, `owner_documents`, `requirements` in `State` (`mem.rs:327-338, 403`) and has `now()` (`:1749`) and `handle_at(now)` (`:1654`) | true | `mem.rs:327` items, `:335` documents, `:338` owner_documents, `:403` requirements; `now()` `:1749`; `handle_at` `:1654` (`#[cfg(feature = "test-support")]`) |
| C16 | `MemClock` default is the untruncated wall clock (`mem.rs:133-157`) | true | `mem.rs:133-157`: `MemClock(None)` = `Utc::now()` untruncated |
| C17 | `IndexerLeaseStore` is re-exported at `store/mod.rs:25`; model indexer types at `model/mod.rs:144`; `mod indexer_lease;` at `pg/mod.rs:10` | true | `store/mod.rs:9, 25`; `model/mod.rs:144`; `pg/mod.rs:10` |
| C18 | `INDEXER_LEASE_CASES` (10) at `conformance.rs:3058`, `run_all_indexer_leases` at `:3134`, `case_names_are_unique` chains four extra lists (`:37740-37769`), `run_indexer_lease_case_accepts_every_name` at `:37944` | true | `conformance.rs:3058` (10 names), `run_all_indexer_leases` `:3134`, `case_names_are_unique` `:37741-37769` chains four lists after `CASES`, accepts-every-name test `:37942-37947`. See C41 for what T1 said about it |
| C19 | Pins: `mem_store.rs:135-140` (`INDEXER_LEASE_CASES` 10), `mem_store_indexer_lease_conformance` at `:45`; `pg_conformance.rs:223-245` | true | `mem_store.rs:43-45`, `:135-140`; `pg_conformance.rs:223-245` |
| C20 | `pg/indexer_lease.rs` uses `sqlx::query!` macros, so new Pg queries need `.sqlx` entries; `= ANY($1)` over a UUID array is used elsewhere (`.sqlx/query-655b33e7….json`) | true | `pg/indexer_lease.rs:41, 102, 118, 135, 149` are `sqlx::query!`; `.sqlx/query-655b33e7….json`: `SELECT id FROM item WHERE project_id = ANY($1) …`, parameter `UuidArray` |
| C21 | `WriteStore` has `mint_item` (`traits.rs:418`), `update_item` (`:~421`), `transition` (`:435`), `write_document` (`:2088`), `write_owner_document` (`:2099`), `mint_requirement` (`:2285`), `amend_requirement` (`:2300`) | true | `traits.rs` `mint_item:418`, `update_item:420`, `transition:435`, `write_document:2088`, `write_owner_document:2099`, `mint_requirement:2285`, `amend_requirement:2300` |
| C22 | `Indexer::sync(read, scope, store, guard)` collects one delete batch per project, confirms orphans, then asks the guard (`vector_sync.rs:193-268`); `sync_owner_documents` puts unlisted docs in `doomed.documents` and superseded ones in `doomed.stale` (`:345-400`) | true | `vector_sync.rs:193-268` (`Doomed::confirm`, then guard, then one `delete`); `sync_owner_documents:345-406` puts listed-but-not-newest in `stale`, unlisted in `documents` |
| C23 | `ConceptPoint::id` is deterministic: item UUID, SHA-256 of (document, section), requirement-derived (`vector.rs:~299-316`) | true | `vector.rs:307-316` `ConceptPoint::id`; `document_point_id:322`, `requirement_point_id:~334` (SHA-256) |
| C24 | `MemStore::demo()`'s item/requirement/document stamps are fixed in the past (older than 300 s before a test's now) | true | fixtures use `demo_at(day, hour)` = 2026-09-01 + offset (`fixtures.rs:113`, items `:1278`); Pg `load_demo` inserts `updated_at` explicitly (`pg/demo.rs:421-424, 604-606`) and runs no UPDATE |
| C25 | `VectorStore` has exactly `upsert`, `delete`, `indexed(project)`, `search` (`vector.rs:422-431`); no new method is needed | true | `vector.rs:422-431` |
| C26 | `QdrantSettings` is `Clone` with a redacting `Debug` (`qdrant_settings.rs:7-22`); `QdrantUrl` is `PartialEq` | true | `qdrant_settings.rs:7-22` (`#[derive(Clone)]`, hand `Debug` redacting `api_key`); `QdrantUrl` derives `PartialEq, Eq` (`htui-core/src/qdrant.rs:18-19`) |
| C27 | `LeaseTimes::from_app` refresh is at most TTL/3 (40 s by default) (`htui-orch/src/recover.rs:45-82`; `docs/htui-worker.md:456-458`) | true | `recover.rs:64-82` `refresh = min(refresh, ttl/3 ms)`, defaults 120/60 so 40 s; `docs/htui-worker.md:455-457`. Can be < 1 s: see C45 |
| C28 | The job tests' `TokioClock` makes `MemStore`'s clock follow paused tokio time (`concepts.rs:2258-2278`); `Counting.syncs` counts `indexed` reads of the htui project (`:2288-2310`) | partial | `TokioClock` `concepts.rs:2259-2270` (base + paused elapsed, truncated to µs); `Counting.syncs` (`:2293`, incremented in `indexed` `:~2340`) counts **every** `indexed(PROJECT_HTUI)` read, so an incremental pass over htui moves it too. amended: T4 test 1 (C46) |
| C29 | No `app_setting` key for an incremental cycle exists; the seeded keys are in `pg/mod.rs:~50-60` | partial | no cycle key anywhere (migrations, `pg/mod.rs`, docs). But seeded keys are not only `pg/mod.rs:52-56` (`cache_refresh_seconds`, `cache_overlap_seconds`): migrations seed others (`max_concurrent_items`, `token_budget`, …). D6's conclusion is unaffected; no amendment |
| C30 | `--index-items` (`index_under_leases`, `concepts.rs:381-433`) and `Ctrl+R` call `Indexer::sync` and are untouched by D10 | true | `Indexer::sync` callers: `concepts.rs:412` (`index_under_leases`), `:899` (`reconcile`), `concepts_worker.rs:1064` (`QdrantIndex::sync`, the Ctrl+R path), `:1328/:1384` (`MemIndex`); none bound on `IndexSource` |
| C31 | `docs/hr-sandbox.md:227-236` holds the `sqlx prepare` recipe; `.sqlx` hash is of the literal query | true | `docs/hr-sandbox.md:224-236`; offline mode from `.cargo/config.toml` `SQLX_OFFLINE = "true"` |
| C32 | `nextest -E 'binary(it) and test(/^pg_conformance::/)'` selects the Pg conformance module of `htui-store`'s `it` target | true | `pg_conformance.rs` is module `pg_conformance` of `htui-store`'s `it` target (AGENTS.md TOOL-15 layout) |
| C33 | Task file sets T2 and T3 do not intersect; T4's only non-code file is `docs/htui-worker.md` | true | Files table: T2 = `pg/index_changes.rs`, `pg/mod.rs`, `.sqlx`, `pg_conformance.rs`; T3 = `vector_sync.rs`: no common path. T4's only non-`.rs` file is `docs/htui-worker.md`. Build coupling: C49 |
| C34 | Document `created_at` (item and owner) is stamped by the store's clock | false | both stores keep the caller's `NewDocument.created_at` / `NewOwnerDocument.created_at` (`mem.rs:8401, 8433`; `pg/write.rs:962-993` binds `$8`); product writers pass `Utc::now()` (`hand_written.rs:195`, `owner_documents.rs:122`, `run_worker.rs:727`). amended: D3 (writer-clock note), D5, Risks row, T1 cases 3/5/6 stamp `since ± 1 s`, T4 stamps documents from the store clock |
| C35 | `write_document` does not bump the item's `updated_at`, so D4 needs its document arm | true | Mem `State.write_document` (`mem.rs:8370-8406`) never touches the item; Pg `write_document` (`pg/write.rs:9512-9531`) only `SELECT 1 FROM item … FOR UPDATE`. So D4's document arm is required; D3 now says so |
| C36 | MemStore stamps `updated_at` in `update_item`, `transition`, close-out, amend and withdraw | true | see C9; `MemStore.update_item` `:11251`, `close_out` `:12046`, `amend_requirement` `:12188`, `withdraw_requirement` `:12207` all pass `self.now()` |
| C37 | `MemStore::demo()` exists and works as T1's per-case factory | true | `mem.rs:477` (`#[cfg(feature = "demo")]`); `conformance.rs:37945` already calls `run_indexer_lease_case(name, &MemStore::demo())`; `mem_store.rs:44` the factory form |
| C38 | Every `IndexSource` implementor gets `IndexChangesStore` from T1/T2 (supertrait adds no wrapper work) | true | implementors are only `PgStore`/`MemStore` (C3). In `concepts.rs` tests `Takeover` (`:2282`) is a plain struct, `Counting` a `VectorStore`, `Fakes` a `JobIndex`; `Unlisted` (`vector_sync.rs:1421`) implements `ReadStore` only. amended: T4 Action notes it |
| C39 | `ReadStore` offers `item(id)`, `requirement(id)`, `owner_documents(project)` (D7) | true | `traits.rs:126`, `:278`, `:395`; also `document(id)` and `project(id)` that `sync` uses |
| C40 | An item can move between projects (D3 "or move", D7 "now in another project") | false | no `UPDATE item SET project_id` anywhere; `fk_document_item_project` has no `ON UPDATE CASCADE`. amended: D3 drops "move", D7 keeps the skip as a guard for a hand-built/stale `IndexChanges` (T3's test stays valid) |
| C41 | T1: "`case_names_are_unique` chains the fifth list … 'five lists'" | false | it already chains five lists (`conformance.rs:37750-37755`) while its comment/message say "four". amended: T1 says sixth list, "six lists" |
| C42 | `item`/`requirement` INSERTs stamp `updated_at` from the server (case 7 brackets hold on Pg) | true | `mint_item` / `mint_requirement` INSERTs omit `updated_at` (`pg/write.rs:2693-2695`, `:10392-10394`): `DEFAULT now()` (transaction start, after a prior `clock_timestamp()` read) |
| C43 | D9 step 1 can tell a completed, non-abandoned reconcile from M1's `reconcile` | false | `reconcile` returns `()` and only logs (`concepts.rs:844-912`). amended: D9 step 1 and T4 Action give it a return value |
| C44 | M1 keeps the keyed settings only for one beat; D8's `JobState.keyed` is new | true | `keyed` is a beat-local (`concepts.rs:704`); D8 deliberately moves it into `JobState`. `JobState` derives `Debug, Default` (`:587`), so `Held` needs `Debug`; `Option<QdrantSettings>` keeps both derives. No amendment |
| C45 | Flooring the whole beat sleep at 1 s is harmless | false | `LeaseTimes::refresh` is `ttl/3` in ms and can be < 1 s (`recover.rs:64-82`); flooring the min would lengthen the lease beat. amended: D9 floors only the due term |
| C46 | `Counting` can show "one full sync" in T4's first test | partial | `syncs` counts every `indexed(PROJECT_HTUI)` read (`concepts.rs:~2335-2345`), which `sync_changes` also makes. amended: T4 test 1 |
| C47 | Status moves stamp `updated_at` on both stores | true | Pg `transition` is `UPDATE item SET status = $3 …` (`pg/write.rs:2890`) under the trigger; Mem `:2631`. `vector_sync.rs:5-6`'s "a status move does not touch `updated_at`" is stale (T3 may correct it with the module doc) |
| C48 | `MemStore`'s `index_changes` can read everything D3 needs from `State` | true | `items: HashMap<ItemId, Item>` (`project_id`, `updated_at`), `documents: Vec<Document>` (`item_id`, `created_at`), `owner_documents: Vec<OwnerDocument>` (`project_id`, `created_at`), `requirements` (`project_id`, `updated_at`) |
| C49 | T2 and T3 are independent and may run side by side | partial | file sets disjoint (C33) and neither edits the other's files; T3's tests need only T1 (`MemStore::index_changes`). But `SQLX_OFFLINE=true` (`.cargo/config.toml`): T2's new `query!` without `.sqlx` entries fails every `htui-store` build, T3's included, on a shared tree. amended: header, Intersections, T3 Depends (separate worktrees, or T2 first) |
