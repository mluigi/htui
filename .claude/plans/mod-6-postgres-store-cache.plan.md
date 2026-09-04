# Plan: MOD-6 Postgres store + cache

**Source**: `HANDOFF.md` item MOD-6 (from ANA-9). Design authority: `docs/ANA-9.md` (§4 settled
questions, §5 DDL, §6 cache contract, §7 key queries, §9 scope, §11 validation criteria).
**Requirements**: `R-STO-1..6`, `R-ENT-1..12`, `R-USR-2`; `R-NF-2`, `R-NF-3` by construction.
**Complexity**: Large
**Routing**: routed as **plan** by maintainer override (table verdict was PRD at threshold, C2 +
C4; ANA-9 already answers every design question a PRD would ask). Ultracode was recommended for
the implement phase on C4; this plan proposes **not** using it (see Tasks). Reviewer:
`rust-reviewer` (`.claude/workflow-config.json`). Models per maintainer: plan and reviewer on
Fable, every implementer on Opus, never Fable for implementation.

## Summary
Add a third crate, `htui-store`, holding the Postgres backend (`PgStore: WriteStore`), the per-box
SQLite mirror (`CacheStore: ReadStore`), the refresh task, the keyring DSN, the box identity file
and the `Backend` enum that the TUI already holds (moved out of `htui-core`, gaining the `Online` /
`Offline` variants of ANA-9 §6.1). `htui` learns to start from the cache, connect to Postgres off
the UI task, ask before applying migrations, show `online` / `offline · 3m` in the top bar, and
run the cursor refresh in the background. The MOD-1 conformance suite runs unchanged against
`PgStore`, plus the concurrency, replay and cache criteria of ANA-9 §11.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | New crate `crates/htui-store` (lib) depending on `htui-core`; `htui` depends on `htui-store`. `Backend` **moves** from `htui-core::store::backend` to `htui_store::Backend` with variants `Memory(MemStore)`, `Online { pg: PgStore, cache: CacheStore }`, `Offline { cache: CacheStore, since: DateTime<Utc> }`. `htui-core` keeps traits, `MemStore`, conformance, fixtures. | MOD-1 plan D1 reserved exactly this crate. `Backend` cannot stay in core once it names `PgStore`; the domain crate must not pull `sqlx` + bundled SQLite. Type-level "no write path in `Offline`" (§6.1) holds because `WriteStore` is implemented by `PgStore` only and `Backend` exposes writes through `Backend::writable() -> Option<&PgStore>`. |
| D2 | `sqlx` 0.9.0, features `runtime-tokio, postgres, sqlite, migrate, macros, chrono, uuid, json, tls-rustls-ring-native-roots`. Postgres queries use the compile-time macros (`query!` / `query_as!`) with **committed offline data** (`crates/htui-store/.sqlx/`) and `SQLX_OFFLINE=true` in a workspace `.cargo/config.toml`; SQLite mirror queries use runtime `sqlx::query_as` + `FromRow`. | ANA-9 §9.1 and §11.1 ask for `sqlx` offline query checking on the Postgres side. A crate has one `DATABASE_URL` for macros, so the mirror (our own schema, exercised by the cache tests) is runtime-checked. Hermetic builds: `cargo build`/`clippy`/`doc` never need a server; a query change needs `DATABASE_URL=… cargo sqlx prepare -p htui-store` and the regenerated `.sqlx` files committed. |
| D3 | `htui-core` gains an optional feature `sqlx` that adds `#[derive(sqlx::Type)] #[sqlx(transparent)]` to the ID newtypes and a weak-enum `sqlx::Type` derive (per-variant `#[sqlx(rename = …)]` driven by the same literal) to every `str_enum!` enum. `htui-store` enables it. | Rows decode straight into model types on both databases (probe V2). One macro edit covers all 15 enum sites (V10); no parallel row structs per enum. |
| D4 | Migrations: `crates/htui-store/migrations/0001_init.sql` is ANA-9 §5 verbatim, embedded with `sqlx::migrate!`. On connect: `list_applied_migrations` vs the embedded set. Applied version unknown to the binary → **refuse** (`StoreError::Backend("schema is newer than this htui")`). Pending → report `MigrationState::Pending(n)`; the TUI asks (overlay) and only then `apply_migrations()`. Checksum mismatch → refuse. | `R-STO-5`, ANA-9 §5.0. `Migrator::run` already errors on an unknown applied version (`VersionMissing`) and on checksum drift (`VersionMismatch`); the pending count comes from the applied list. |
| D5 | Seed on first connect (§5.10, global part only): one `app_user` (name from `USERNAME` / `USER`, fallback `htui`), the ten `capability_tag` rows, `app_setting` defaults (`cache_refresh_seconds` 30, `cache_overlap_seconds` 300). **No `agent` rows**: `agent.launch` and `transport` are ANA-4's shape; MOD-2 seeds them. Per-project seeding (kinds, graphs, templates) is MOD-15's create-project. | Seeding a `NOT NULL` JSONB launch shape before ANA-4 concludes would be a guess ANA-4 then has to migrate away. |
| D6 | Box identity: `<config_dir>/htui/box.toml` (`box_id` UUIDv7, `hostname`), minted on first launch. On first connect `PgStore::register_box` upserts the `box` row on `(user_id, hostname)` with what `std::env::consts` and `gethostname` know (`os_family`, `arch`, `hostname`, `htui_version`; `os_version` empty). If the hostname already has a row with another id, `box.toml` adopts the DB id. `last_seen_at` bumped per connect. The full probe (`box_tool`, tags, RAM, GPU) stays MOD-7. | `item_revision.box_id`, `run.target_box_id` and the top bar need a real row; MOD-7 owns the probe, not the identity. |
| D7 | Keyring: `keyring` 3.6.3, features `windows-native, apple-native, sync-secret-service, crypto-rust`. Entry `("htui", "postgres-dsn")`. Set/clear through the binary: `htui --set-dsn` reads the DSN from stdin (no echo, before the TUI starts), `htui --clear-dsn`. The binary reads the DSN from the keyring only; **no env-var fallback in the binary** (`R-STO-1`). Tests use `HTUI_TEST_DATABASE_URL`. TLS is whatever the DSN says (`sslmode=`), handled by `PgConnectOptions::from_str` (`R-STO-2`). | Linux secret service via D-Bus is the ANA-9 choice; `crypto-rust` avoids OpenSSL. Linux is not built on this box (same caveat as MOD-1). |
| D8 | Cache: `sqlx` SQLite, `<config_dir>/htui/cache/<sha256(host:port/dbname)>/cache.sqlite`, WAL, `busy_timeout` 5 s, `cache_migrations/0001_mirror.sql` (§4.4 mirror: the fifteen mirrored tables, `cache_meta`, `cache_cursor`; UUID as TEXT, timestamps as INTEGER microseconds, arrays and JSONB as TEXT). `CacheStore::open` rebuilds (delete file, re-create) when `cache_meta.schema_version` ≠ the embedded Postgres migration version, when `last_full_refresh_at` is older than seven days (flag, full pass on next connect), or on `CacheStore::rebuild()` (the future `Settings > Rebuild cache`). | §4.4 verbatim. No feature gate for SQLite in this item: ANA-9 §10 lists the gate as a mitigation to reach for if the bundled build ever fails on a box; the build passed on MSVC here (V1). |
| D9 | Refresh task (`cache::refresh`): one `tokio` task per `Online` backend, owns the only writer to `cache.sqlite`. Per pass, per project in the current scope (a `watch::Receiver<Vec<ProjectId>>` the store worker updates on every `Items` request), tables in FK order per §6.2, `ts_col > high_water - overlap`, PK upsert, tombstoned `item_link` rows deleted from the mirror, cursor set to `max(ts_col)` of the fetched rows; then last-N `session_event` copy (`project.settings.cached_transcript_steps`, default 20) and trimming; then `pending/*.jsonl` upload. Workspaces, `workspace_project` and the own `box` row refresh unscoped. Interval `cache_refresh_seconds`. First pass right after connect. | §6.2 / §6.3 verbatim; `R-NF-3` (never on the UI task); `R-STO-6`. |
| D10 | Startup and reconnect (`connect.rs`): the store worker starts with `Backend::Offline` over the opened cache and label `connecting`; a connect task tries Postgres and sends one `ConnEvent` (`Online(PgStore)`, `MigrationsPending(PgStore, n)`, `Failed(String)`). The worker swaps the `Backend` in place (it owns it, plan D4 of MOD-1). While `Offline`, the worker retries every 30 s; on success it swaps to `Online` and starts the refresher. `--offline` skips the connect entirely (for demos and tests). `--demo` stays `Memory`. | §4.4 "open the cache, render immediately, connect off the UI thread". Swapping inside the worker keeps every view free of store handles. |
| D11 | Top bar: `StoreRequest::StoreState` → `StoreReply::StoreState { label, migrations_pending }`; the shell issues it on the same fourth-tick refresh as `BoxInfo`. Label texts: `memory`, `connecting`, `online`, `offline · <age>` (`s`/`m`/`h` units). A pending migration count opens the `MigrationPrompt` overlay (`y` applies via `StoreRequest::ApplyMigrations`, `n` stays offline). | `R-STO-5` confirmation without a blocking prompt on the UI task; the overlay registry exists for this. |
| D12 | Live reads while `Online` go to Postgres; `Offline` reads go to the mirror; writes exist only on `PgStore`. `Backend` inherent reads (`workspaces`, `box_info`, `active_runs`, `projects`) get a `PgStore` and a `CacheStore` implementation each. `step_events` on `CacheStore` returns `None` for a step outside the last N (§6.1 "None = not cached"). | §4.4 last paragraph. |
| D13 | Tests against a real server: `tests/common/mod.rs` reads `HTUI_TEST_DATABASE_URL` (a maintenance DSN with `CREATEDB`), creates `htui_test_<uuid-suffix>`, runs the migrator, hands out a `PgStore` loaded with `DemoData` through `PgStore::load_demo` (feature `demo`), and drops the database at the end of the test. Env unset → the test prints `skipped: HTUI_TEST_DATABASE_URL not set` and returns, so `cargo test --workspace --all-features` stays green on a box without Postgres. | Fresh database per test is what makes the concurrency cases deterministic. No `#[sqlx::test]` (it panics without `DATABASE_URL`). |
| D14 | Retention sweep (`R-HIS-3`) and the `LISTEN`/`NOTIFY` wake-up are **out of scope**: not in the MOD-6 text nor in ANA-9 §9's five points. The importer mint variant is MOD-8. | Scope discipline; noted in the close-out. |

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-core/src/store/{traits,mem,backend}.rs` | snake_case modules, one public type per file, `store/<backend>.rs` for a backend, `mod.rs` re-exports. Column names verbatim from §5 on every struct field. |
| Errors | `crates/htui-core/src/store/error.rs:9-31` | One `StoreError` enum; `Backend(String)` is the reserved arm for `sqlx` errors; unique-violation / FK / CHECK errors map to `Constraint`, missing rows to `NotFound`. |
| Logging | `crates/htui/src/lib.rs:65-82` | `tracing` to a file under `HTUI_LOG`, never stdout. Refresh pass logs at `debug` per table, `info` per pass summary, `warn` on connect failure. |
| Data access | `crates/htui-core/src/store/mem.rs:697-747`, `docs/ANA-9.md` §7 | `ReadStore` / `WriteStore` impls as thin dispatchers; §7.1 / §7.2 CTEs used verbatim as the `query!` text. |
| Tests | `crates/htui-core/tests/mem_store.rs`, `crates/htui-core/src/store/conformance.rs:20-60` | One integration file binds the suite to a store: `conformance::run_all(|| async { … })`. `CASES` names stay; `run_all` is made iteration-driven so `PgStore` reports per case (MOD-1 watch item). |
| Worker seam | `crates/htui/src/store_worker.rs:35-115` | New requests are variants of `StoreRequest` / `StoreReply` plus one arm in `serve`; the event loop does not change. |

## Files to Change

Task file sets are the independence facts for §3.5; T2 ∩ T3 = ∅ is verified in V6.

| File | Action | Task | Why |
|---|---|---|---|
| `Cargo.toml` (workspace) | UPDATE | T1 | member `crates/htui-store`; workspace deps `sqlx`, `keyring`, `dirs`, `gethostname`, `sha2`, `toml`, `htui-store` |
| `.cargo/config.toml` | CREATE | T1 | `[env] SQLX_OFFLINE = "true"` |
| `crates/htui-core/Cargo.toml` | UPDATE | T1 | optional dep `sqlx` (no runtime features; `postgres`, `sqlite`, `uuid`), feature `sqlx` |
| `crates/htui-core/src/model/ids.rs`, `src/model/mod.rs` | UPDATE | T1 | `cfg_attr(feature = "sqlx", …)` derives (D3) |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | T1 | `run_all` iteration-driven over `CASES`, per-case reporting hook `run_case(name, make)` |
| `crates/htui-store/Cargo.toml`, `src/lib.rs`, `src/error.rs` | CREATE | T1 | crate, `sqlx::Error` → `StoreError` |
| `crates/htui-store/migrations/0001_init.sql` | CREATE | T1 | ANA-9 §5 in FK order, trigger loop, deferred FKs to `run_step` |
| `crates/htui-store/src/pg/mod.rs` | CREATE | T1 | `PgStore { pool, this_box, this_user }`, `connect(dsn) -> Connected`, `MigrationState`, `apply_migrations`, `seed_if_empty`, `register_box`, `pool()` accessor |
| `crates/htui-store/src/pg/demo.rs` | CREATE | T1 | `PgStore::load_demo(&DemoData)` (feature `demo`) inserting every table in FK order with explicit ids and timestamps |
| `crates/htui-store/src/identity.rs` | CREATE | T1 | config dir layout, `box.toml`, `db_fingerprint(dsn)` |
| `crates/htui-store/tests/common/mod.rs`, `tests/migrations.rs` | CREATE | T1 | fresh-DB helper (D13); migrations apply on a clean server, re-run is a no-op, newer applied version refuses, demo loads |
| `crates/htui-store/src/pg/rows.rs`, `src/pg/read.rs`, `src/pg/write.rs` | CREATE | T2 | `ReadStore` + inherent reads; `WriteStore` (§7.1, §7.2, status CAS) |
| `crates/htui-store/tests/pg_conformance.rs`, `tests/pg_criteria.rs` | CREATE | T2 | conformance suite; §11.2 concurrent mint + rollback, §11.3 CAS race, §11.4 5 000-event replay (Postgres half) |
| `crates/htui-store/.sqlx/query-*.json` | CREATE | T2, T3 | offline data; additive per query hash, no shared file |
| `crates/htui-store/cache_migrations/0001_mirror.sql` | CREATE | T3 | mirror schema (D8) |
| `crates/htui-store/src/cache/{mod,read,refresh,pending}.rs` | CREATE | T3 | `CacheStore`, mirror reads, cursor pass, pending upload |
| `crates/htui-store/tests/cache.rs` | CREATE | T3 | §11.4 mirror half, §11.6 overlap window (2 s window, transaction held open across a pass), tombstone removal, last-N trimming, rebuild on schema mismatch, §11.7 pending upload idempotent |
| `crates/htui-store/src/backend.rs`, `src/secret.rs`, `src/connect.rs` | CREATE | T4 | `Backend` (D1), keyring (D7), startup + reconnect (D10) |
| `crates/htui-core/src/store/backend.rs` | DELETE | T4 | moved |
| `crates/htui-core/src/store/mod.rs`, `src/lib.rs` | UPDATE | T4 | drop the `Backend` re-export and doc mentions |
| `crates/htui/Cargo.toml`, `src/cli.rs`, `src/lib.rs`, `src/store_worker.rs`, `src/testkit.rs`, `src/app/{state,update}.rs` | UPDATE | T4 | dep on `htui-store`; `--set-dsn`, `--clear-dsn`, `--offline`; worker owns `ConnEvent` + swap; `StoreState`, `ApplyMigrations`; label plumbing |
| `crates/htui/src/ui/overlay/migration_prompt.rs`, `src/ui/overlay/mod.rs` | CREATE / UPDATE | T4 | the `R-STO-5` confirmation overlay, registered |
| `crates/htui/tests/shell.rs` (+ snapshots) | UPDATE | T4 | snapshot: top bar `offline · 3m`; migration prompt open |
| `README.md` | UPDATE | T4 | DSN setup, cache location, offline mode, test env var, `cargo sqlx prepare` note, Postgres 16 minimum |
| `compose.yaml` | CREATE | pre-T1 (done by the router at CONFIRM) | dev `postgres:16` on host port 5433 for the test suite and the manual runs |

## Tasks

Waves: A = {T1} serial (foundation); B = {T2, T3} **parallel** on Opus in git worktrees (disjoint
file sets, V6; both read `PgStore::pool()` and `load_demo` from T1); C = {T4} serial (touches
`htui`, the core `Backend` removal and README, which both B tasks would otherwise share). TDD per
task: tests first, then code. Two parallel tasks and two serial ones are not a workflow, so this
plan proposes running B as two direct `Agent` calls (`model: "opus"`, `isolation: "worktree"`)
instead of an ultracode script; the maintainer can force ultracode at CONFIRM.

Every implementer prompt carries: ANA-9 has priority over this plan where they disagree (MOD-1
precedent); graphify-first for codebase questions; `.sqlx` must be regenerated and committed with
any query change; never hold a lock or a pool connection across a UI await.

### Task 1: crate, migration, connect, seed, identity, demo loader, test harness
- **Action**: workspace edits and `.cargo/config.toml` (D2). `htui-core` feature `sqlx` (D3).
  `htui-store` crate with `error.rs` (`sqlx::Error::Database` with SQLSTATE `23xxx` →
  `Constraint`, `RowNotFound` → `NotFound`, everything else → `Backend`). `migrations/0001_init.sql`
  transcribed from §5 in the order §5.0 names, with the `set_updated_at` trigger loop and the three
  `ALTER TABLE … ADD CONSTRAINT` forward references. `pg/mod.rs`: `PgStore::connect(dsn: &str) ->
  Result<Connected>` where `Connected { store, migrations: MigrationState }`,
  `MigrationState::{UpToDate, Pending(usize)}`, refusal errors for newer / checksum-mismatched
  schema; `apply_migrations`; `seed_if_empty` (D5); `register_box(identity)` (D6); `pool()`.
  `identity.rs`: `config_dir()/htui`, `box.toml` read-or-mint, `db_fingerprint`. `pg/demo.rs`:
  `load_demo` inserting `DemoData` in FK order (users, boxes, workspaces, projects,
  workspace_project, graphs, phases, kinds, templates, agents, counters, items, revisions, links,
  notes, documents, runs, steps, events) with the fixture's ids and timestamps. `conformance.rs`:
  `run_all` iterates `CASES` through a `match` on the name (length assert stays). Test harness
  `tests/common/mod.rs` (D13). Tests: migration applies on a clean database and a second `run` is
  a no-op; a fake `_sqlx_migrations` row with version 9999 makes `connect` refuse; `load_demo`
  round-trips a count per table; `box.toml` mint is stable across two reads.
- **Mirror**: `docs/ANA-9.md` §5, §5.0, §5.10; `crates/htui-core/src/store/error.rs`.
- **Validate**: `cargo build -p htui-store`, `cargo clippy -p htui-store --all-targets -- -D warnings`,
  `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --test migrations`.

### Task 2: PgStore reads and writes (parallel with T3)
- **Action**: `pg/rows.rs` `FromRow` structs where a query cannot decode straight into a model
  type (`RunSummary` with `box_hostname` join, `LinkNode` with `project_slug`, `ItemSummary`).
  `pg/read.rs`: the seven `ReadStore` methods and the four inherent reads. `items` implements
  every `ItemFilter` field in SQL (`status = ANY`, `project_id = ANY`, `required_tags @>`, the
  §7.4 readiness `NOT EXISTS`, `ILIKE` on key and title), ordered as `MemStore` orders (project
  position, then `key_prefix`, then `key_number`). `links` is a recursive CTE over live edges in
  both directions up to `hops`, cross-project by UUID. `step_events` is §7.5. `pg/write.rs`:
  `mint_item` is §7.1 in one transaction with the kind-belongs-to-project check and nil-author
  rejection; `update_item` is §7.2 plus the `Diverged { head, ancestor }` path on zero rows;
  `transition` is the status CAS with `closed_at` following terminal status both ways, never
  touching `version`. Tests: `tests/pg_conformance.rs` runs `conformance::run_all` over a fresh
  demo database; `tests/pg_criteria.rs` covers §11.2 (two pools minting the same prefix 50 times
  each → 100 consecutive numbers; a mint inside a rolled-back transaction leaves no gap), §11.3
  (two edits from the same version → exactly one `Updated`, one `Diverged` with
  `ancestor.version == start`), §11.4 Postgres half (5 000 events inserted, replayed in `seq`
  order, content equal).
- **Mirror**: `crates/htui-core/src/store/mem.rs` semantics (the conformance suite is the spec);
  `docs/ANA-9.md` §7.1, §7.2, §7.4, §7.5.
- **Validate**: `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --features demo --test pg_conformance --test pg_criteria`;
  `DATABASE_URL=… cargo sqlx prepare -p htui-store` then `cargo clippy -p htui-store --all-targets -- -D warnings` with `SQLX_OFFLINE=true`.

### Task 3: CacheStore, refresh, pending upload (parallel with T2)
- **Action**: `cache_migrations/0001_mirror.sql` (D8). `cache/mod.rs`: `CacheStore::open(dir,
  fingerprint, pg_schema_version) -> Result<CacheStore>` (creates dirs, WAL, rebuild rule),
  `rebuild()`, `meta()`. `cache/read.rs`: the seven `ReadStore` methods and the four inherent
  reads over the mirror, same ordering rules as T2 (shared by the §11.4 equality test).
  `cache/refresh.rs`: `Refresher::spawn(pool: PgPool, cache: CacheStore, projects:
  watch::Receiver<Vec<ProjectId>>, settings: RefreshSettings) -> JoinHandle`, `run_pass` as a
  free function for tests, per-table `fetch_since` on the Postgres side via `query!`, PK upsert on
  the SQLite side, tombstone deletes, last-N transcript copy and trim, `cache_meta` bookkeeping.
  `cache/pending.rs`: `upload_pending(pool, dir)` reading `pending/*.jsonl` (one `session_event`
  row per line, `run_id` from the filename), inserting `run` (`kind = 'chat'`), `run_step`
  (`phase_name = 'chat'`) and events in one transaction with `ON CONFLICT DO NOTHING` on the
  event PK, deleting the file on commit. Tests (`tests/cache.rs`, each over a fresh demo
  database + a temp cache dir): a pass mirrors the demo; §11.4 mirror half replays 5 000 events
  equal to Postgres; §11.6 with `overlap = 2 s`: a transaction that updates an item, stays open
  across one pass, commits, and is present after the next pass; a tombstoned link disappears from
  the mirror; steps beyond N lose their events (`step_events` → `None`); a schema-version change
  rebuilds; §11.7 upload lands ordered and running it twice does not duplicate.
- **Mirror**: `docs/ANA-9.md` §4.3 (offline buffer), §4.4, §6.2, §6.3.
- **Validate**: `HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --features demo --test cache`;
  `cargo sqlx prepare` + clippy as in T2.

### Task 4: Backend, keyring, shell wiring, README
- **Action**: `backend.rs` (D1, D12) with `label()`, `is_writable()`, `writable()`, inherent
  reads and `ReadStore` delegation over three variants; delete the core `Backend`; fix re-exports
  and doc links. `secret.rs` (D7). `connect.rs` (D10): `start(args) -> (Backend,
  mpsc::Receiver<ConnEvent>, Option<Refresher>)` plus the reconnect ticker. `htui`: `cli.rs`
  flags; `lib.rs` uses `connect::start`; `store_worker.rs` becomes a `select!` over requests and
  `ConnEvent`s, swaps the backend, forwards the scope of every `Items` request to the refresher's
  `watch`; `StoreRequest::{StoreState, ApplyMigrations}` and their replies; `App` writes
  `top_bar.store` from `StoreState` and opens `MigrationPrompt` on a pending count; the overlay
  itself; `testkit` builds a `Backend::Memory` exactly as before. README sections. Tests: worker
  unit tests for the swap (`Offline` → `Online` on a fake `ConnEvent`) and `StoreState`; shell
  snapshots for the `offline · 3m` label and the open prompt; a `Backend::Offline` write attempt
  is a compile error by construction (documented, not tested).
- **Mirror**: `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/overlay/workspace_switcher.rs`
  (overlay shape), `README.md` sections.
- **Validate**: full validation block below, plus a manual run: `htui --set-dsn`, `htui` against
  the local server (top bar `online`), stop the server, `htui` again (renders from cache,
  `offline · …`).

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5432/postgres cargo test -p htui-store --all-features
cargo doc --workspace --no-deps
cargo run -p htui -- --demo
cargo run -p htui                                            # online, then with the server stopped
```
ANA-9 §11 mapping: 1 → T1 test on Postgres 18.6 (local) and 16 (`docker run postgres:16`,
daemon is up, V5); 2, 3 → `pg_criteria`; 4 → `pg_criteria` + `cache`; 5 → manual timing on this
box, recorded in the close-out; 6 → `cache` with a 2 s window; 7 → `cache`.

**Prerequisite (met 2026-09-04)**: a reachable Postgres for T1–T4 test runs. Maintainer chose a
dev container over the local scoop server: `compose.yaml` at the repo root runs `postgres:16`
(16.15) on host port 5433, user `postgres`, password `htui`, database `htui`; brought up with
`docker compose up -d`. Maintenance DSN for the tests:
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5433/postgres`. ANA-9 §11.1 on
Postgres 18 uses the local scoop server (not running) when the maintainer starts it; 16 is the
floor and is what the suite runs against by default.

**CONFIRM (2026-09-04)**: maintainer accepted the plan as written (D1–D14, no ultracode, direct
Opus `Agent` calls for every implementer, Fable for the reviewer).

## Verified claims (§3.5 fact-check)
`probe` = scratch crate built and run against rustc 1.98.0 / cargo 1.98.0 on this box
(2026-09-04); `tree` = grep/read of the repo; `registry` = crate source under `~/.cargo/registry`.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `sqlx` 0.9.0 with `runtime-tokio, postgres, sqlite, migrate, macros, chrono, uuid, json, tls-rustls-ring-native-roots` builds on 1.98 (MSRV 1.94) including bundled SQLite on MSVC; two `sqlx::migrate!` in one crate; `Migrator::run` on SQLite; `SqliteConnectOptions::journal_mode(Wal)`; `PgConnectOptions::from_str` accepts `sslmode` | **true** | probe printed `pg migrator versions [1], cache 1`, `sqlite rows [...]`, `probe ok` |
| V2 | `#[derive(sqlx::Type)] #[sqlx(transparent)]` on a `Uuid` newtype and a weak-enum derive with `#[sqlx(rename)]` compile once for both databases; `FromRow` decodes `UUID`, `TEXT[]`, `JSONB`, `TIMESTAMPTZ` into `Uuid`, `Vec<String>`, `serde_json::Value`, `DateTime<Utc>` | **true** | probe: `sqlite enum decode Done`; `PgRow: FromRow` type-checked |
| V3 | `keyring` 3.6.3 `windows-native` builds and `Entry::get_password` distinguishes `NoEntry` | **true** | probe: `keyring: NoEntry (windows-native works)` |
| V4 | `dirs` 6 `config_dir()` is `%APPDATA%` on Windows; `gethostname` 1.1 returns the hostname; `toml` 1.1 round-trips a `Uuid` field; `sha2` 0.10 digests | **true** | probe lines `config_dir Some("C:\\Users\\…\\AppData\\Roaming")`, `hostname "DESKTOP-JBKR0TS"`, `toml BoxToml {…}`, `fp 2d77…` |
| V5 | Postgres tooling: `psql` 18.6 via scoop, server **not running** (`pg_isready`: no response; `pg_ctl status`: no server); docker daemon 29.7.2 up for a Postgres 16 container | **true** | `psql --version`, `pg_isready`, `pg_ctl status`, `docker info` |
| V6 | T2 ∩ T3 = ∅ | **true** | T2 = `src/pg/{rows,read,write}.rs`, `tests/pg_conformance.rs`, `tests/pg_criteria.rs`; T3 = `cache_migrations/`, `src/cache/**`, `tests/cache.rs`. Shared reads only: `src/pg/mod.rs`, `src/pg/demo.rs`, `tests/common/mod.rs` (T1, frozen during B). `.sqlx/query-<hash>.json` files are per query, additive |
| V7 | `Backend` is referenced from five `htui` / core files outside its own module, so the move is bounded | **true** | `grep -l Backend`: `htui/src/{app/state.rs, lib.rs, store_worker.rs, testkit.rs}`, `htui-core/src/{lib.rs, store/mod.rs, store/backend.rs, store/error.rs, store/mem.rs, store/traits.rs}` (the last three are doc mentions) |
| V8 | `sqlx-core` 0.9 `Migrator` exposes `iter()`, `migrations`, `run`, `set_ignore_missing`; `MigrateError` has `VersionMissing`, `VersionMismatch`, `Dirty`; the `Migrate` trait has `list_applied_migrations` | **true** | registry `sqlx-core-0.9.0/src/migrate/{migrator,error,migrate}.rs` |
| V9 | ANA-9 §5 DDL applies cleanly on Postgres 18 / 16 | **deferred to T1** | needs the server (V5); the T1 migration test is the check |
| V10 | `str_enum!` is the single declaration path for every database-text enum (15 sites) | **true** | `grep -c "str_enum!("` over `crates/htui-core/src` = 15 |
| V11 | `UNIQUE NULLS NOT DISTINCT` needs Postgres 15+; both available servers qualify | **true** | ANA-9 §3; PG 18.6 local, 16 via docker |
| V12 | `sqlx::query!` macros build without a server once `.sqlx/` exists and `SQLX_OFFLINE=true` | **known behaviour, not probed** | documented `sqlx` offline mode; first exercised in T1 with the server up |

## Blueprint errata (post-CONFIRM, 2026-09-04)
`.claude/plans/mod-6-postgres-store-cache.blueprint.md` §H lists 17 errata found against ANA-9,
the code and the sqlx 0.9.0 sources; the implementation follows the blueprint's corrected
reading. Three were re-probed by the router against the live databases and hold: H.8 (a
`str_enum!` enum needs `#[sqlx(type_name = "text")]`; the plain derive fails decode and bind
against `TEXT`), H.9 (sqlx decodes `Uuid` on SQLite as BLOB, so mirror UUIDs go through a
`String` helper), H.10 (an INTEGER-microseconds column does not decode as `DateTime`, error not
silent; helper required). Plan-level amendments: D3 reads "strong-enum derive with
`type_name = "text"`"; D10's `start` returns `Started` and the worker spawns the refresher on
`Online`; D1's `Offline.since` is an `Option` (`None` = `connecting`); the pending file is
`pending/<project_id>.<run_id>.jsonl`; `cargo sqlx prepare` runs from `crates/htui-store`.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| `sqlx` 0.9.0 is weeks old; docs and examples online describe 0.8 | Medium | V1/V2/V8 probed against the real crate; implementers read `registry` source, not blog posts |
| `.sqlx` drift: a query edited without `cargo sqlx prepare` fails the hermetic build | Medium | Every task's validate step runs `prepare` then clippy with `SQLX_OFFLINE=true`; reviewer checklist item |
| Two parallel agents both regenerate `.sqlx/` | Low | Files are per query hash; cherry-pick merges cleanly; T4 runs one final `prepare` |
| Long-transaction visibility gap in the cursor pass | Low | Overlap window + weekly full pass per ANA-9 §10; §11.6 test with a 2 s window |
| `keyring` Linux build (D-Bus) untested here | Medium | Same caveat as MOD-1 (Windows-only box); feature set is the crate's documented Linux default |
| Refresher and TUI both hit Postgres on connect | Low | Pass runs on its own task, per-table, `debug` logged; TUI reads Postgres directly (§10) |
| SQLite pool held across a UI await | Low | Reviewer checklist; `CacheStore` methods are self-contained `async fn` on their own pool |
| Test databases leak on a panicking test | Low | Names carry the `htui_test_` prefix; README notes `DROP DATABASE` cleanup |

## Acceptance
- [x] All tasks complete (commits 2b0125f, f8364e6, e977bc8, d0fe7b1, 8427cf9, f16c972, 946bfed)
- [x] Validation block passes; 157 tests green with the env var set (Postgres 16.15, plus 17.11 for migrations and conformance)
- [x] ANA-9 §11 criteria 1–4, 6, 7 mapped to passing tests; 5 (warm start) left as a manual measurement in the write-up
- [x] Patterns mirrored, not reinvented (§5 DDL, §7 queries, §6.2 algorithm verbatim; reviewer verified)
- [x] `rust-reviewer` findings F1–F9 all applied; verdict approve at f16c972, F8/F9 at 946bfed
- [x] HANDOFF / DECISIONS bookkeeping per `lifecycle.md` P2, validator green
