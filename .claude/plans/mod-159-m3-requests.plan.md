# Plan: MOD-159 M3 — Requests and holder visibility

**Source PRD**: `.claude/prds/mod-159-knowledge-indexer.prd.md`
**Selected Milestone**: 3 — Requests and holder visibility
**Complexity**: Medium (no migration; two `.sqlx` entries; three crates)
**Status**: complete (`c9edd91a..cfa8d817`, review fix round R1 `2025c1e9..cfa8d817`)
**Run**: `/handoff-run MOD-159` in an hr sandbox (`hr/MOD-159`); CONFIRM waived by `/handoff-run`. Lane shape: one
serial lane T1 → T2 → T3. T2 and T3 have disjoint file sets but both edit and build the `htui` crate, so they may run
side by side only in separate worktrees (as M2's T2/T3, C49 there); on one tree, T2 then T3. Verifier on T1
(foundation) only. **No migration**: `indexer_lease.pass_requested_at` already exists (`0038`, C1).

## Summary

M1 already takes a free lease and indexes on `Ctrl+R` and `--index-items`, names the holder's box when it cannot,
and lets every job lease go on a clean exit (delta table below). M3 adds the **pass request**: a requester that finds
the lease held live stamps `indexer_lease.pass_requested_at` with the store's clock and says it asked the holder; the
holder's index job, on its next beat, claims (reads and clears) the request and runs a **full reconcile** of that
collection at once, whatever `concepts_sync_minutes` says. It also closes the one exit gap: a `Ctrl+R` cut short by
quitting the TUI now lets its lease go instead of leaving it to lapse.

### Delta: what M1/M2 already deliver of the M3 row

| M3 clause | State | Evidence |
|---|---|---|
| `Ctrl+R` takes a free lease and indexes | done (M1) | `concepts_worker.rs:1420-1465` `index_under_lease`: renews the job's lease, else takes under a press token; `Taken \| Ours` → sync, release after (`:1460`) |
| `--index-items` takes a free lease and indexes | done (M1) | `concepts.rs:383-440` `index_under_leases`: take per workspace, `Held` → note + skip (`:401`), else open + `Indexer::sync` under `Renewing` |
| Show the holder's box | done (M1) | `held_note` `concepts.rs:351-356`, `holder_name` `:361`; overlay `held_line` `concepts_search.rs:687-697`, arm `:496-498`, snapshot `…__held_by_another_box.snap` |
| Record a pass request; holder serves it next beat | **missing** | `pass_requested_at` is written and read nowhere (text search: only `0038` and `migrations.rs:8769`) |
| `htui worker` exit releases | done (M1) | `worker_cmd.rs:157` `index_job.stop(WORKER_RELEASE_WAIT)`; `IndexJob::stop` `concepts.rs:1261-1273` releases by token (`:1268`) |
| TUI quit releases the job's leases | done (M1) | `store_worker.rs:3984-3985` `concepts.shutdown(); concepts.stop_index_job().await`; publish swap stops old jobs (`concepts_worker.rs` `publish`) |
| `--index-items` exit releases | done (M1) | `concepts.rs:328` releases before `report?`, so on error too |
| TUI quit mid-`Ctrl+R` releases the press lease | **missing** | `ConceptsRuntime::shutdown` (`concepts_worker.rs:1836-1840`) only aborts (every task in `tasks`: presses, searches and probes alike) and drops the handles; a fresh `Uuid::now_v7()` per press (`:1438`) is known to no one after; `docs/htui-worker.md:500-502` says it lapses |

## Decisions taken in this plan

- **D1 Storage: the existing column, no migration.** `0038_indexer_lease.sql:32` already has `pass_requested_at
  TIMESTAMPTZ` ("M3's … unused in M1", `:44-45`), pinned by `migration_0036_creates_indexer_lease`
  (`migrations.rs:8769`). So no `0041`, `TABLES` stays 60, no cache migration, the table stays not mirrored (M1's
  decision, `0038:20-23`), and `Backend` keeps refusing every lease verb offline. The applied migration's comment is
  not edited (an applied migration's SQL never is). Stamped with `clock_timestamp()` / `MemStore::now()`.
- **D2 Two new trait methods, no signature change.** Folding the request into `take_indexer_lease`'s answer would
  change `LeaseTake::Ours`/`Taken`, matched or compared at ~45 sites (`LeaseTake::` text search: 49 hits, mostly
  `assert_eq!(…, LeaseTake::Taken)` in conformance and job tests); folding it into `renew_indexer_lease` reaches only
  the delete guards and `while_renewing`, not the beat, which takes (not renews) each lease (`concepts.rs:788-792`).
  So, in `IndexerLeaseStore` (`htui-core/src/store/indexer_lease.rs`):
  ```rust
  /// Stamps a pass request on `collection`'s row when someone holds it live; answers that holder
  /// (as `indexer_lease` shows it, `live` true). `None` (no row, released, lapsed): nothing written.
  fn request_indexer_pass(&self, collection: &IndexedCollection)
      -> impl Future<Output = Result<Option<IndexerLease>>> + Send;
  /// Clears `collection`'s pass request when `owner` holds the row (expiry not judged, as
  /// `renew_indexer_lease`); answers whether one was pending. A request stamped after this call
  /// stays for the next claim.
  fn claim_indexer_pass(&self, collection: &IndexedCollection, owner: Uuid)
      -> impl Future<Output = Result<bool>> + Send;
  ```
  Implementors: `MemStore`, `PgStore`, `Backend` (forwarding; offline `orchestration_offline()`), the only three
  (C6). The request is kept by a take (owner change or not) and by a release; only a claim clears it.
- **D3 Claim before the pass, so nothing is lost.** The holder claims (atomic read-and-clear) *before* the reconcile
  reads anything; a request recorded during the pass re-stamps the cleared column and is claimed at the next beat.
  No stamp comparison, no second write after the pass. A requested pass that fails or is abandoned has consumed its
  request (the next beat does not retry it: a down Qdrant must not become a full reconcile every 40 s); M1's
  interval and the next request are the retries.
- **D4 `take_or_request_pass`**, a free fn beside the trait (exported from `store/mod.rs` as `lease_ttl_micros` is),
  so both requesters share one rule and the conformance suite pins it on both stores:
  ```rust
  pub async fn take_or_request_pass<L: IndexerLeaseStore>(leases: &L, collection: &IndexedCollection,
      box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<LeaseTurn>
  /// in model/indexer.rs, re-exported at model/mod.rs:147:
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum LeaseTurn { Ours, Requested(IndexerLease), Held(IndexerLease) }
  ```
  Up to `PASS_REQUEST_ATTEMPTS = 3`: take → `Taken | Ours` → `Ours`; `Held(_)` → `request_indexer_pass` →
  `Some(holder)` → `Requested(holder)`; `None` (the holder let go in between) → take again. After the last attempt,
  `Held(last seen)`: nothing recorded. A lease that is free or lapsed is therefore taken, never requested (the
  existing behaviour); only a live holder gets a request.
- **D5 Serving: a full reconcile on the next beat.** In `index_beat`, after a take answers `Taken | Ours` and only
  for a workspace collection (`scope.is_some()`; the global one has nothing to sync and gets no requests),
  `claim_indexer_pass(&collection, owner)`: `Ok(true)` → one `info` line (`concepts index pass requested;
  reconciling now`, `%collection`) and the collection takes M1's full-reconcile path this beat even when
  `reconciled.elapsed() < every` (the `last.filter(…)` branch at `concepts.rs:808` is skipped), so `reconciled` restarts
  from it; `Err` → `warn`, treated as `false`. Full, not incremental, because `Ctrl+R`/`--index-items` are full passes
  today and a request asks for what they would have done. A requested reconcile reads the key as a due one does
  (`with_key`, once per beat). Latency: the next beat, at most `lease_refresh_seconds` (40 s default) plus work in
  flight. No change to `JobState`, `Held` or the sleep computation.
- **D6 Wording.** CLI (`concepts.rs`, beside `held_note`): `requested_note(slug, lease)` =
  `workspace <slug>: indexing runs on <box>; asked it to reindex`. Overlay: `requested_line(lease)` =
  `held_line(lease)` + `; asked it to reindex` → `indexing runs on hal (lease until 09:41); asked it to reindex`
  (fits `MAX_WIDTH` 120). `held_note`/`held_line` stay for `LeaseTurn::Held` (nothing recorded); their tests and the
  `held_by_another_box` snapshot stay valid. New `IndexOutcome::Requested(IndexerLease)` beside `HeldBy`
  (`concepts_worker.rs:70-75`); `--index-items` still exits 0 and skips the workspace.
- **D7 One press token per runtime; exit lets it go.** `ConceptsRuntime` gains `press_owner: Uuid` (minted in `new`,
  never the job's `owner`; `pub const fn press_owner()` beside `owner()`), replacing the per-press `Uuid::now_v7()`
  (`:1438`). A press superseded half-way leaves its lease under that token, so the next press takes it back as `Ours`
  and releases it after (no self-request against its own box). `shutdown()` keeps the aborted handles;
  `stop_index_job(&mut self, backend: &Backend)` (signature change: callers `store_worker.rs:3985` and the test at
  `concepts_worker.rs:4282` only) runs two legs side by side (`tokio::join!`): (a) await the aborted handles, then
  `backend.release_indexer_leases(press_owner)`, the whole leg within one `TUI_RELEASE_WAIT` (an error, a timeout
  or an offline backend is a `debug` line, the lease lapses); (b) stop the job as today (its own two
  `TUI_RELEASE_WAIT` bounds). The tokens differ, so the legs touch disjoint rows. Concurrent, not sequential, so
  the concepts teardown's worst case stays today's 2 s: the loop first spends up to `CANCEL_GRACE` (2 s,
  `agent_worker.rs:91`) on runs and chats, and `lib.rs:321` cuts the whole store task at `SHUTDOWN` (5 s); two more
  sequential 1 s waits would reach 6 s (amended at fact-check, C28). Runs whether or not the runtime has a job (the
  early `return` on `job == None` at `concepts_worker.rs:1667` goes).
- **D8 Docs.** `docs/htui-worker.md`: index-job bullet gains "a pass request is claimed on the next beat and runs a
  full reconcile at once"; the `Ctrl+R`/`--index-items` bullet (`:498-502`) says they ask the holder and name its box,
  and that quitting lets a press's lease go (a second press reuses it). `README.md` Search section: one sentence
  after step 2 (another process indexing a workspace is asked to reindex it; the line names its box).
  `docs/htui-cli.md:108-116` quotes the `--index-items` note verbatim (`:112`, `workspace <slug>: indexing runs on
  <box>; not indexed here`): it gains the requested note as the live-holder case and keeps the old one for "nothing
  recorded" (T2; amended at fact-check, C29).
- **Requirements.** `R-STO-8` (`docs/REQUIREMENTS.md:416-440`): "One `htui` process per collection indexes it and
  keeps it current: the holder of that collection's lease" — requesters never index a held collection (D4). `R-ENT-12`,
  `R-ENT-13`, `R-ENT-16` carry no M3 wording (content coverage only). `R-NF-2` (Qdrant fails open,
  `docs/REQUIREMENTS.md:902`) is untouched: a request and a claim are Postgres writes, not Qdrant calls. A failed
  claim in the beat is a `warn` line and the beat goes on (D5). A failed request fails that one `--index-items` /
  `Ctrl+R` run as a failed take does today: `take_or_request_pass` returns `Err`, `index_under_leases` stops at the
  first lease failure (`?`, as at `concepts.rs:401`'s take) and `index_under_lease` answers `Err` (amended at
  fact-check, C31: the plan said "never a failed run").

## Shared decisions (prepend to every brief)

> MOD-159 M3. No migration: `indexer_lease.pass_requested_at` (0038) holds the request, stamped by the store clock;
> not mirrored; `TABLES` 60, `CASES` 343 unchanged. New `IndexerLeaseStore::request_indexer_pass(collection) ->
> Option<IndexerLease>` (stamps only under a live holder, answers it) and `claim_indexer_pass(collection, owner) ->
> bool` (clears it on `owner`'s row). Take and release keep the request; only a claim clears it. `take_or_request_pass`
> (htui-core) → `LeaseTurn::{Ours, Requested(lease), Held(lease)}`. The holder claims right after its beat's take, for
> workspace collections only, and runs a full reconcile at once; a request stamped during the pass survives (claim
> before the pass). A failed requested pass consumed its request. Wording: `…; asked it to reindex` (CLI after
> `indexing runs on <box>`, overlay after `held_line`). `Ctrl+R` presses use one per-runtime `press_owner` token,
> released at TUI exit. Pg SQL: `sqlx::query!` only.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow lease trait | `htui-core/src/store/indexer_lease.rs:28-96` | `fn … -> impl Future<…> + Send`, impls write `async fn`, docs say what "zero rows" means |
| Mem impl | `mem.rs:13245-13314` (`impl IndexerLeaseStore for MemStore`); row `MemIndexerLease` `:478-485`; `State.take_indexer_lease` `:13387-13412` | `self.now()` first, then `self.write(…)`; the take's `insert` rebuilds the row, so it must carry the request over |
| Pg impl | `htui-store/src/pg/indexer_lease.rs:95-115` (`renew`), `:148-173` (`indexer_lease` view with `LEFT JOIN box`) | `sqlx::query!`, `clock_timestamp()`, `rows_affected() == 1`, `map_sqlx` |
| Backend forwarding | `backend.rs:1029-1088`; test `:1536` | `Memory` / `Online { pg }` / `Offline` → `orchestration_offline()` |
| Lease cases | `conformance.rs:3150-3235` (list, `run_indexer_lease_case(_inline)`, `run_all_indexer_leases`), `PLATFORM_COLLECTION`, `indexer_lease_row` `:3242`, `LEASE` | boxed arms; zero TTL = lapsed; compare store-clock values only |
| Job tests | `concepts.rs` `mod index_job`: `reconciled` `:3349`, `Counting` (`full_syncs`, `fail`, `pause`), `index_items_skips_…` `:3658`, `a_held_note_…` `:3714` | `#[tokio::test(start_paused = true)]`, advance by `BEAT` |
| Ctrl+R tests | `concepts_worker.rs:3958-4140` (`platform()`, `serve_one`, `MemIndex::delayed`, `settle`) | over `Backend::memory` |
| Overlay test | `concepts_search.rs:1771-1825` (`Bench`, `held_line` asserts, dim colour, `insta::assert_snapshot!`) | new snapshot name, old one untouched |

## Files to Change

| Task | Files (owned) |
|---|---|
| T1 | `crates/htui-core/src/model/indexer.rs`, `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/indexer_lease.rs`, `crates/htui-core/src/store/mod.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/tests/it/mem_store.rs`, `crates/htui-store/src/pg/indexer_lease.rs`, `crates/htui-store/src/backend.rs`, `crates/htui-store/.sqlx/query-*.json` (two new), `crates/htui-store/tests/it/pg_conformance.rs` |
| T2 | `crates/htui/src/concepts.rs`, `README.md`, `docs/htui-cli.md` (added at fact-check, C29) |
| T3 | `crates/htui/src/concepts_worker.rs`, `crates/htui/src/ui/overlay/concepts_search.rs`, `crates/htui/src/ui/overlay/snapshots/htui__ui__overlay__concepts_search__tests__requested_from_another_box.snap` (new), `crates/htui/src/store_worker.rs`, `docs/htui-worker.md` |

T2 ∩ T3 = ∅; both depend on T1 only. Same crate (`htui`): parallel only in separate worktrees, else T2 then T3.

## Tasks

### T1: Pass request in the store: trait, MemStore, PgStore, Backend, cases (foundation; tests first)

- **Depends on**: nothing.
- **Action**: D1, D2, D4. `LeaseTurn` in `model/indexer.rs` (module doc: "and a pass request, M3"), re-export at
  `model/mod.rs:147`. The two methods in `IndexerLeaseStore` (module doc: the column, claim-before-pass), plus
  `PASS_REQUEST_ATTEMPTS` and `take_or_request_pass` in `store/indexer_lease.rs`, exported from `store/mod.rs`.
  `MemIndexerLease.pass_requested_at: Option<DateTime<Utc>>`; `State.take_indexer_lease` keeps the old row's value;
  releases leave it; `request_indexer_pass` stamps `now` when `row.live(now)` and answers the row as
  `State::indexer_lease_view(&name, now)` (`mem.rs:13416`) read after the stamp: `indexer_lease_of(&self, …, row)`
  cannot take a row still borrowed through `get_mut` (C27);
  `claim_indexer_pass` is `row.owner == Some(owner) && row.pass_requested_at.take().is_some()`.
  Pg (two `sqlx::query!`, so two `.sqlx` entries):
  ```sql
  WITH asked AS (
      UPDATE indexer_lease SET pass_requested_at = clock_timestamp()
       WHERE collection = $1 AND lease_owner IS NOT NULL AND lease_expires_at > clock_timestamp()
      RETURNING collection, lease_box_id, lease_expires_at)
  SELECT a.collection AS "collection!", a.lease_box_id, b.hostname AS "box_name?", a.lease_expires_at
    FROM asked a LEFT JOIN box b ON b.id = a.lease_box_id
  ```
  (`fetch_optional`, `live: true`) and
  `UPDATE indexer_lease SET pass_requested_at = NULL WHERE collection = $1 AND lease_owner = $2 AND
  pass_requested_at IS NOT NULL` (`rows_affected() == 1`). The take's `ON CONFLICT … SET` (`pg/indexer_lease.rs`
  `take_indexer_lease`) and both releases do not name the column, so they keep it already: assert, do not change.
  `Backend` forwards both.
- **Tests first** — five cases appended to `INDEXER_LEASE_CASES` (10 → 15) with arms in
  `run_indexer_lease_case_inline`:
  1. `a_pass_request_on_a_live_lease_names_its_holder_and_only_the_holder_claims_it` (box id, hostname, `live`;
     another owner's claim `false` and leaves it; the holder's claim `true`, then `false`);
  2. `a_pass_request_on_a_free_released_or_lapsed_lease_records_nothing` (no row → `None` and still no row; released;
     zero-TTL lapsed; a later taker's claim is `false`);
  3. `a_pass_request_made_after_a_claim_waits_for_the_next_claim`;
  4. `a_pass_request_survives_takes_and_releases_until_claimed` (holder re-takes `Ours`: still pending; holder releases,
     another owner takes `Taken`: its claim `true`);
  5. `take_or_request_pass_takes_a_free_lease_and_asks_a_live_holder` (`Ours` on a free row; `Requested(holder)` on a
     held one, the holder's claim `true`; `Ours` on a lapsed one).
  `backend.rs`'s forwarding test gains both methods in memory and offline (`StoreError::Unreachable`).
- **Pins**: `mem_store.rs` `INDEXER_LEASE_CASES.len() == 15` ("… and MOD-159 M3's five pass-request cases (plan D2-D4)");
  `pg_conformance.rs:243` `15` ("as `mem_store.rs` pins them"). `CASES` 343, `INDEX_CHANGES_CASES` 7 unchanged;
  `case_names_are_unique` and `run_indexer_lease_case_accepts_every_name` iterate the list (no edit).
- **`.sqlx`**: two new entries, recipe `docs/hr-sandbox.md` "Changing SQL queries in a run" (`:225-241`): scratch DB
  `htui_sqlx` on `psql -h localhost -p 5439 -U postgres`, `cd crates/htui-store`, `cargo sqlx migrate run --source
  migrations`, `cargo sqlx prepare -- --all-targets --all-features`; commit only the two new files (a changed old
  entry means altered SQL).
- **Validate**: `cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'`;
  `cargo nextest run -p htui-core --all-features --lib -E 'test(/conformance::tests::/)'`;
  `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_conformance::/)'` (test DSN per
  `docs/hr-sandbox.md`; a skip line means it did not run); `cargo nextest run -p htui-store --all-features --lib -E
  'test(/backend::tests::the_indexer_lease/)'`; `cargo sqlx prepare --check -- --all-targets --all-features` in
  `crates/htui-store`; `cargo clippy -p htui-core -p htui-store --all-features --all-targets -- -D warnings`;
  `cargo clippy -p htui-core -p htui-store -- -D warnings`; `cargo fmt -p htui-core -p htui-store --check`.

### T2: The job serves requests; `--index-items` asks (tests first)

- **Depends on**: T1.
- **Action**: D5, D6 (CLI), D8 (README, `docs/htui-cli.md:108-116`). `index_beat`: the claim after the take, the `requested` override of the
  due test, the info line; its doc comment gains the request. `index_under_leases`: `take_or_request_pass` per
  workspace (`Requested` → `requested_note`, `Held` → `held_note`, both skip); the global take unchanged; its doc and
  `index_items`' doc say "asks the holder". `requested_note` beside `held_note`, through `holder_name`.
- **Tests first** (`mod index_job` unless noted):
  - `a_pass_request_reconciles_a_held_collection_at_the_next_beat` (`reconciled`; advance `BEAT`; another owner's
    `request_indexer_pass(&PLATFORM)` is `Some`; one beat → `full_syncs() == 2`; the request is gone (holder's claim
    `false`); a further beat with none → still 2);
  - `a_request_made_during_a_requested_reconcile_is_served_by_the_next_beat` (arm `Counting`'s `pause`, request again
    from a spawned task while the reconcile waits; next beat → one more full sync);
  - `a_failed_requested_reconcile_consumes_its_request` (`Counting.fail`; the next beat runs no full sync);
  - `a_beat_claims_no_request_for_a_collection_it_does_not_hold` (another owner holds GRAPHICS with a request: the job
    leaves it, the request stays for its holder's claim);
  - adapt `index_items_skips_a_workspace_another_box_indexes_and_says_so` (`:3658`): note `workspace <graphics>:
    indexing runs on DESKTOP-HTUI; asked it to reindex`; `other`'s claim on GRAPHICS is `true`; doc comment updated;
  - `a_requested_note_names_the_box_by_hostname_or_short_id` (unit, beside `:3714`; that test stays).
  Existing M1/M2 job tests pass unchanged (a beat with no request makes one extra claim call per workspace; no
  counter pins store calls).
- **Validate**: `cargo nextest run -p htui --all-features --lib -E 'test(/^concepts::/)'`;
  `cargo test -p htui --all-features --test it --no-run`; `cargo clippy -p htui --all-features --all-targets -- -D
  warnings`; `cargo clippy -p htui -- -D warnings`; `cargo fmt -p htui --check`.

### T3: `Ctrl+R` asks, the overlay says so, quit lets the press lease go (tests first)

- **Depends on**: T1 (not T2).
- **Action**: D6 (overlay), D7, D8 (`docs/htui-worker.md`). `IndexOutcome::Requested(IndexerLease)` (doc);
  `index_under_lease(index, backend, scope, job_owner, press_owner)`: the job-renew branch unchanged, else
  `take_or_request_pass` under `press_owner` (`Ours` → sync + release as today; `Requested` → `Requested`; `Held` →
  `HeldBy`); its doc drops "a token of its own per press". `ConceptsRuntime`: `press_owner`, `press_owner()`,
  `aborted: Vec<JoinHandle<()>>` filled by `shutdown()`, `stop_index_job(&Backend)` per D7; `store_worker.rs:3985`
  passes the loop's `backend`. Overlay: `requested_line` (doc: MOD-159 M3 D6) and the `on_reply` arm (dim, as
  `HeldBy`). The test helper `indexed` (`concepts_worker.rs:1948`) panics on `Requested` too.
- **Tests first**:
  - adapt `a_ctrl_r_while_another_owner_holds_the_lease_says_who_and_syncs_nothing` (`:3960`): `Requested(lease)`,
    `lease.box_id == other_box`, the other owner's claim `true`, nothing synced (rename to `…_asks_the_holder_…`);
  - `a_ctrl_r_quit_mid_sync_lets_its_lease_go` (`start_paused`, `MemIndex::delayed`, serve, sleep 5 s, `shutdown()`,
    `stop_index_job(&backend)` → lease not live, another owner `Taken`);
  - `a_ctrl_r_after_a_superseded_one_syncs_under_the_same_token` (first press delayed; a second `IndexConcepts` from
    the same origin supersedes it; the reply is `Synced`, not `Requested`; lease not live after);
  - `a_ctrl_r_on_a_free_lease_holds_it_under_a_token_of_its_own` (`:4108`) and `…_lets_the_lease_go` keep passing
    (the press token is still never `owner()`); `the_runtimes_index_job_runs_under_its_token_and_lets_go_at_the_stop`
    (`:4233`, a Postgres test over `fresh_db`) takes the new `stop_index_job` argument: it builds its
    `Backend::Online` inline in the `publish` call today, so it binds it to a local first; `shutdown_aborts_every_task`
    (`:3730`) keeps passing unchanged (`aborted` is apart from `tasks`, so `settle` still answers 0);
  - overlay: `a_ctrl_r_another_box_indexes_asks_it_to_reindex` (as `:1771`: `requested_line` text for a named and an
    unnamed box, drawn dim, `insta::assert_snapshot!("requested_from_another_box", frame)`); review the new `.snap`
    with `cargo insta` and commit it. `held_by_another_box` must not change.
- **Validate**: `cargo nextest run -p htui --all-features --lib -E
  'test(/^concepts_worker::|^ui::overlay::concepts_search::/)'`; `cargo test -p htui --all-features --test it
  --no-run`; clippy (both forms) and `cargo fmt -p htui --check`.

## Validation

```bash
# main thread, once after the lane (sandbox): live Qdrant runs here since 3e7b7d38 (tmpfs Qdrant)
bash scripts/gate.sh
```

Fallback only for a run whose Qdrant predates the tmpfs (`4732f845`) and times out creating collections
(`.claude/skills/handoff-run/SKILL.md`, "Live Qdrant tests run here"): `env -u HTUI_TEST_QDRANT_URL bash
scripts/gate.sh` ("PASSED (1 skipped: qdrant)" is then green) and the live-Qdrant suites owed on the host (amended at
fact-check, C30: the plan used the fallback as the default).

Bookkeeping after the lane (not a task): `HANDOFF.md` pins line `:106` (`INDEXER_LEASE_CASES` 10 → 15), the MOD-159
entry (phase 3 landed, item closes if review is clean), the PRD's M3 row (`pending` → `complete`, Plan cell → this
file), decision record per `workflow-docs.md`.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A request against a holder that cannot index (no key, Qdrant down) is consumed with nothing done | Medium | D3: logged by the holder; the requester's line says "asked", not "indexed"; the next interval or request retries |
| The holder is a manual run (`--index-items`, a press): it never claims | Low | The request survives its release (case 4); the next job to take the lease claims it at its first beat, which reconciles in full anyway |
| A superseded press on another workspace keeps that lease until TTL | Low | Same as M1; the next press of it takes it as `Ours`; the TUI's exit lets it go (D7) |
| One extra `UPDATE` per held workspace per beat | Low | Zero-row when no request; per beat, not per item |
| sqlx infers CTE column nullability wrongly | Medium | Override names (`"collection!"`, `"box_name?"`) as the `indexer_lease` view does |
| Shutdown budget: press wait + job release + earlier stop | Low | The press leg (await aborted handles, release `press_owner`) has one `TUI_RELEASE_WAIT` (1 s) and runs beside the job stop (D7), so the concepts teardown stays at today's worst case of 2 s after `CANCEL_GRACE` (2 s), inside `SHUTDOWN` (5 s, `lib.rs:321`) |

## Acceptance

- [ ] All tasks complete, each with its own commits
- [ ] Sandbox gate green (`bash scripts/gate.sh`, live Qdrant included; `PASSED (1 skipped: qdrant)` only under the
      pre-tmpfs fallback above)
- [ ] Holder-visibility metric: CLI note test and overlay snapshot show the holder's box and the request
- [ ] Live-Qdrant suites listed as owed on the host only if the fallback was used

## Open questions for the maintainer

None blocking. The wording "asked it to reindex" (D6) is the plan's choice; the PRD and ANA-37 Q5 fix only that the
holder's box is shown.

## Verified claims (step 3.5 fact-check)

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| C1 | `indexer_lease.pass_requested_at TIMESTAMPTZ` exists in `0038_indexer_lease.sql:32`, commented as M3's (`:13-14, :44`), so no migration is needed | true | `sed -n` of `0038`: `:32` `    pass_requested_at TIMESTAMPTZ` (nullable, no default), `:13-14` "pass_requested_at is MOD-159 M3's … M1 never writes it", `:44-45` `COMMENT ON COLUMN … 'when Ctrl+R or --index-items asked the holder for a pass (M3); unused in M1.'` (the text is on `:45`; D1's cite now `:44-45`). Not mirrored `:20-23`. Last migration `0040_execution_targets.sql` |
| C2 | `pass_requested_at` is read or written by no Rust code; only `migrations.rs:8769` pins the column | true | Gortex text search `pass_requested_at`: 4 hits, `0038:13, 32, 44` and `tests/it/migrations.rs:8769` in `migration_0036_creates_indexer_lease` (`("pass_requested_at", "timestamp with time zone", "YES")`) |
| C3 | Migrations run through `0040_execution_targets`; `TABLES` is 60; `CASES` 343, `INDEXER_LEASE_CASES` 10, `INDEX_CHANGES_CASES` 7 (`mem_store.rs`, `pg_conformance.rs:243`) | true | `find migrations` ends at `0040_execution_targets.sql`; `TABLES` (`migrations.rs:40-120`) counted 60 names, `indexer_lease` 58th; `demo_store_loads_the_fixture` (`mem_store.rs:56-159`) pins 343 / 10 / 7; `pg_conformance.rs:243-245` pins `10` |
| C4 | Mem `State.take_indexer_lease` (`mem.rs:13387-13412`) inserts a whole new `MemIndexerLease`, so a new field must be carried over explicitly | true | Body 13387-13412: `self.indexer_leases.insert(name, MemIndexerLease { owner, box_id, expires_at })` (`:13405`), the only struct literal of the type (text search `MemIndexerLease`: 5 hits). `MemIndexerLease` (`:478-485`) has only `owner`, `box_id`, `expires_at`: T1 adds `pass_requested_at: Option<DateTime<Utc>>` (derives `Debug, Clone` only, so nothing else to update) |
| C5 | Pg take's `ON CONFLICT … SET` names only owner, box and expiry; both releases set only owner and expiry | true | `pg/indexer_lease.rs:41-60`: `INSERT … (collection, lease_owner, lease_box_id, lease_expires_at)` and `SET lease_owner, lease_box_id, lease_expires_at`; `release_indexer_leases` (`:117`) and `release_indexer_lease` (`:130`): `SET lease_owner = NULL, lease_expires_at = clock_timestamp()`. A first take's INSERT leaves the column NULL |
| C6 | `IndexerLeaseStore` is implemented by `MemStore` (`mem.rs:13245`), `PgStore` (`pg/indexer_lease.rs:23`), `Backend` (`backend.rs:1029`) only | true | Text search `IndexerLeaseStore for`: exactly those 3; text search `IndexerLeaseStore` (45 hits) shows no test fake implementing it (only bounds and imports). All three are T1 files, so the two required methods need no default body |
| C7 | `LeaseTake::` appears at 49 places, most of them test assertions; the beat takes (not renews) each lease (`concepts.rs:788-792`) | true | Text search `LeaseTake::`: count 49 (3 are doc lines in `indexer_lease.rs:31-33`; 34 in tests: conformance 21, backend test 3, pg_conformance 3, `concepts.rs` tests 5, `concepts_worker.rs` tests 2). `index_beat` `:788-793`: `source.take_indexer_lease(…)` → `Ok(LeaseTake::Taken \| LeaseTake::Ours) => {}` (`:792`) |
| C8 | `index_beat`'s incremental-vs-full branch is `last.filter(\|held\| held.reconciled.elapsed() < every)` at `concepts.rs:808` | true | Text search: `concepts.rs:808` `if let Some(held) = last.filter(\|held\| held.reconciled.elapsed() < every) {` in `index_beat` (`:698-904`) |
| C9 | `--index-items` releases its leases before `report?` (`concepts.rs:328`), so on error too | true | `index_items` (`:310-333`): `let report = index_under_leases(…).await;` then `:328` `pg.release_indexer_leases(holder.owner)`, then `report_line(&report?)` |
| C10 | `htui worker` stops its job with `WORKER_RELEASE_WAIT` (`worker_cmd.rs:157`); `IndexJob::stop` releases by token (`concepts.rs:1268`) | true | `worker_cmd.rs:157` `index_job.stop(concepts::WORKER_RELEASE_WAIT).await;` in `serve`; `IndexJob::stop` `:1261-1273` (delta table cite corrected from 1275), `:1268` `release_indexer_leases(self.owner)` under `timeout(wait, …)` |
| C11 | The TUI calls `concepts.shutdown()` then `concepts.stop_index_job().await` (`store_worker.rs:3984-3985`); `stop_index_job` returns early with no job and has two callers (that line and the test at `concepts_worker.rs:4282`) | true | `spawn_with_concepts` (`store_worker.rs:3442-3994`) ends with both calls; text search `stop_index_job`: callers only `store_worker.rs:3985` and `concepts_worker.rs:4282`; `stop_index_job` (`:1666-1676`) opens with `let Some(job) = self.job.as_mut() else { return; }`. `backend` is the loop's `mut backend` local, in scope at `:3985` |
| C12 | A `Ctrl+R` press mints `Uuid::now_v7()` (`concepts_worker.rs:1438`), releases after its sync (`:1461`), and `ConceptsRuntime::shutdown` (`:1836`) only aborts | amended | `:1438` `let press = Uuid::now_v7();` true. The release is `:1460` (`backend.release_indexer_lease(&collection, owner)`), not `:1461`; `index_under_lease` spans `:1420-1465`. `shutdown` (`:1836-1840`) only aborts and drops the handles, but it aborts **every** task in `tasks` (searches, presses, probes; `shutdown_aborts_every_task` `:3730` pins a search and a press), not only presses. Delta table amended; D7's await of the aborted handles covers searches/probes too (harmless) |
| C13 | `docs/htui-worker.md:500-502` says a `Ctrl+R` stopped half-way leaves its lease to lapse | true | `nl` of the doc: `:501-502` "A `Ctrl+R` stopped half-way (a second press, or quitting) leaves its lease to lapse within the time-to-live." The bullet itself is `:498-502` (D8 cite corrected from 497) |
| C14 | `held_note` (`concepts.rs:351-356`) and `held_line` (`concepts_search.rs:687-697`) wording is pinned by `concepts.rs:3698, 3726, 3734`, `concepts_search.rs:1789-1797` and the `held_by_another_box` snapshot only | amended | Text search `indexing runs on`: tests at `concepts.rs:3698, 3726, 3734`, `concepts_search.rs:1790, 1798` and also `:1808` (the buffer search in the same test, `:1771-1825`), snapshot `htui__ui__overlay__concepts_search__tests__held_by_another_box.snap`. Besides tests, `docs/htui-cli.md:112` quotes the CLI note verbatim and no task owned it: added to T2 (C29). Since `held_note`/`held_line` stay, every test pin stays valid |
| C15 | `IndexOutcome` has `Synced` and `HeldBy` (`concepts_worker.rs:70-75`); exhaustive matches are the overlay's `on_reply` and the test helper `indexed` (`:1948`) | true | `IndexOutcome` `:70-75` (`#[derive(Debug, Clone, PartialEq)]`); text search `IndexOutcome::` / `HeldBy`: matches are `on_reply` (`concepts_search.rs:454-503`, arms `:489-499`), `indexed` (`:1945-1951`, `HeldBy` arm `:1948`) and `let … else` / `matches!` in tests; every `ConceptsReply::Indexed` site is in T3's files |
| C16 | `lease_ttl_micros` accepts a zero TTL (`traits.rs:3879-3886`), so a lapsed lease is a zero-TTL take in the cases | true | `ttl >= TimeDelta::zero() && ttl <= MAX_LEASE_TTL => Ok(micros)`; `MemIndexerLease::live` and Pg's `live!` are strict (`expires_at > now`) |
| C17 | `case_names_are_unique` (`conformance.rs:40711-40740`) and `run_indexer_lease_case_accepts_every_name` (`:40916`) iterate the lists, needing no edit for new names | true | `case_names_are_unique` chains `INDEXER_LEASE_CASES` and sums `.len()`; `run_indexer_lease_case_accepts_every_name` (`:40915-40919`) loops `for name in INDEXER_LEASE_CASES`. A new name needs only its arm in `run_indexer_lease_case_inline` (`:3173-3219`, else `panic!`) |
| C18 | `Counting` exposes `full_syncs()`, `fail` and `pause`, and `reconciled()` (`concepts.rs:3349`) leaves one full sync done | true | `struct Counting` (`:2556-2567`): `fail: AtomicUsize` (fails the first `fail` upserts), `pause: Mutex<Option<Duration>>` (sleeps the next htui-project read); `full_syncs()` `:2579`; `reconciled` `:3349-3367` asserts `full_syncs() == 1`. `BEAT` is 40 s (`:2509`). A failed full reconcile keeps `Held.reconciled = now` (`index_beat` inserts it before `reconcile`), so the next beat is not a full one |
| C19 | `MAX_WIDTH` is 120 in `concepts_search.rs:60`, so the requested line fits | true | `:60` `const MAX_WIDTH: u16 = 120;` The 61-character `indexing runs on hal (lease until 09:41); asked it to reindex` also fits the 100-column buffer the existing test draws (`bench.buffer(&search, 100, 30)`) |
| C20 | `docs/hr-sandbox.md:224-240` holds the `sqlx prepare` recipe on port 5439 | amended | The section is `:225-241` (`### Changing SQL queries in a run`, `psql -h localhost -p 5439`, `cargo sqlx prepare -- --all-targets --all-features`); T1 cite corrected |
| C21 | No `IndexConcepts` origin other than the knowledge-search overlay sends presses, so one press token per runtime cannot run two presses of one collection at once | true | Text search `IndexConcepts {`: the only product sender is `ConceptsSearch::reindex` (`concepts_search.rs:274`); the rest are tests and routing. `serve` keys tasks by `(Origin, discriminant)` and aborts the superseded one (`concepts_worker.rs:1689-1818`); `OverlayId` is `pub struct OverlayId(pub &'static str)` (`registry.rs:17`), one per overlay kind, so one origin |
| C22 | `R-STO-8` (`docs/REQUIREMENTS.md:416-440`) names the lease holder as the one indexer; `R-ENT-12/13/16` say nothing M3 must satisfy | true | `:429-430` "One `htui` process per collection indexes it and keeps it current: the holder of that collection's lease in Postgres, worker or TUI." `git grep` for reindex / pass request / Ctrl+R / index-items in `REQUIREMENTS.md`: none; R-ENT-12 (`:305`), 13 (`:314`), 16 (`:359`) are content rules |
| C23 | (body) Pattern cite `mem.rs:13245-13330` for the Mem impl | amended | `impl IndexerLeaseStore for MemStore` opens `:13245`; its last method `indexer_lease` is `:13310-13313`, so the block ends `:13314`. Cite corrected |
| C24 | (body) `index_under_leases` is `concepts.rs:383-433` | amended | Symbol range `:383-440` (global take and `drop_legacy` run to `:440`). Delta table corrected |
| C25 | (body) `model/mod.rs:147` re-exports the indexer model; `store/mod.rs` exports `lease_ttl_micros` | true | `model/mod.rs:146` `pub use indexer::{` (its items from `:147`); `store/mod.rs:45` lists `lease_ttl_micros` in the `traits` re-export, `:27` `pub use indexer_lease::IndexerLeaseStore;` (T1 extends this line) |
| C26 | (body, toolchain) Two required trait methods `fn … -> impl Future<Output = …> + Send` can be added without touching code outside T1; `LeaseTurn` can derive `PartialEq, Eq`; `take_or_request_pass` is usable in `index_under_leases` (generic `S`) and inside `serve`'s `tokio::spawn` (over `Backend`) | true | Reasoned from code, no probe: every implementor is a T1 file (C6) and the trait is already RPITIT (not `dyn`), so no default body is needed. `IndexerLease` derives `Debug, Clone, PartialEq, Eq` (`model/indexer.rs`), so `LeaseTurn` can. The free `async fn` takes `&L` with `L: IndexerLeaseStore: Send + Sync` and awaits only `+ Send` futures, so its future is `Send` for `Backend` and for a generic `S` alike. `pub const fn press_owner(&self) -> Uuid` mirrors the existing `pub const fn owner` (`:1602`) |
| C27 | (body, toolchain) Mem `request_indexer_pass` "answers `indexer_lease_of`" | amended | `State::indexer_lease_of(&self, name, row: &MemIndexerLease, now)` (`mem.rs:13422`) needs `&state` while a `get_mut` row borrow is alive; the T1 action now answers `State::indexer_lease_view(&name, now)` (`:13416`) after the stamp. The claim shape `row.owner == Some(owner) && row.pass_requested_at.take().is_some()` over `get_mut` compiles as written |
| C28 | (body) Shutdown budget: each wait bounded by `TUI_RELEASE_WAIT`, "within `SHUTDOWN` (5 s)" | amended | `lib.rs:321` `timeout(SHUTDOWN, worker)`; before `concepts.shutdown()` the loop joins `runs.shutdown(CANCEL_GRACE)` and `runtime.shutdown(CANCEL_GRACE)` (2 s, `agent_worker.rs:91`); today's `stop_index_job` is up to 1 s + 1 s. D7's two more sequential 1 s waits made the worst case 6 s. D7 now runs one press leg (await aborted + release, one `TUI_RELEASE_WAIT`) beside the job stop with `tokio::join!`, keeping 2 s. Risk row updated |
| C29 | (body) Docs to change are `docs/htui-worker.md` and `README.md` | amended | `docs/htui-cli.md:108-116` documents `--index-items`' held note verbatim (`:112`) and the run's exit; the new note makes it stale. Added to T2's files and D8 |
| C30 | (body) Sandbox validation is `env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh`, `PASSED (1 skipped: qdrant)`, live suites owed on the host | amended | HEAD `3e7b7d38` ("run live Qdrant tests in hr sandboxes") changed `.claude/skills/handoff-run/SKILL.md`: a sandbox gates with plain `bash scripts/gate.sh`; the `env -u` recipe is a fallback only for a pre-tmpfs Qdrant. Validation and Acceptance amended |
| C31 | (body) "`R-NF-2`: a failed request or claim is a line, never a failed run (D5, T2/T3)" | amended | A failed claim is a `warn` (D5) — true. A failed request is not: D4's `take_or_request_pass` returns `Result<LeaseTurn>`, `index_under_leases` propagates lease errors with `?` (doc: "The first store, lease or index failure"), and `index_under_lease` maps them to `Err` (`.map_err(…)?`, `:1431-1442`). R-NF-2 (`REQUIREMENTS.md:902`) is Qdrant's fail-open and is untouched (Postgres writes). Requirements bullet reworded to the actual behaviour, which matches a failed take today |
| C32 | (body) Two new `.sqlx` entries, no old one changes | true | `find crates/htui-store/.sqlx -name 'query-*.json' \| wc -l` → 566 today; 568 expected after T1. Neither new statement is byte-identical to an existing one (the CTE and the `pass_requested_at = NULL` update are new text) |
| C33 | (body) Test names the plan keeps or adapts exist where cited | true | `a_ctrl_r_while_another_owner_holds_the_lease_says_who_and_syncs_nothing` `:3960`; `a_ctrl_r_on_a_free_lease_syncs_then_lets_the_lease_go` `:4003`; `a_ctrl_r_on_a_free_lease_holds_it_under_a_token_of_its_own` `:4108` (T3 cite corrected from 4100); `the_runtimes_index_job_runs_under_its_token_and_lets_go_at_the_stop` `:4233`; `index_items_skips_a_workspace_another_box_indexes_and_says_so` `concepts.rs:3658` (asserts `DESKTOP-HTUI`, the demo `ids::BOX`); `a_held_note_names_the_box_by_hostname_or_short_id` `:3714`; `a_ctrl_r_another_box_indexes_says_which_box_and_until_when` `concepts_search.rs:1771`; `serve_one` `concepts_worker.rs:1894`; backend test `the_indexer_lease_forwards_in_memory_and_is_refused_offline` `backend.rs:1536-1630`; `HANDOFF.md:106` pins `INDEXER_LEASE_CASES` 10 |
| C34 | (body) T1 ∩ T2 ∩ T3 file sets are disjoint and each task's files cover its signatures | true | Intersections empty. T2 calls `claim_indexer_pass` on `S: IndexerLeaseStore` and `take_or_request_pass` (both T1); T3 calls `take_or_request_pass(&backend, …)` through `impl IndexerLeaseStore for Backend` (T1's `backend.rs`). T3 touches nothing defined in `concepts.rs` (`requested_line` calls `held_line` → `concepts::holder_name` unchanged); `concepts.rs` matches no `IndexOutcome`; `stop_index_job`'s two callers are T3 files. Only gap was `docs/htui-cli.md` (C29) |
