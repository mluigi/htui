# MOD-6 - Postgres store + cache (done, 2026-09-04)

## Summary

Third crate `htui-store`: the Postgres backend `PgStore: WriteStore`, the per-box SQLite mirror
`CacheStore: ReadStore`, the cursor refresh task, the pending chat-buffer upload, the keyring DSN,
the `box.toml` identity and the `Backend` enum (moved out of `htui-core`) with the `Online` /
`Offline` variants of ANA-9 §6.1. The shell starts from the mirror, dials Postgres off the UI
task, asks before applying migrations, shows `online` / `offline · 3m` in the top bar, drops to the
mirror when the server goes away mid-session and re-dials every 30 s. The MOD-1 conformance
suite runs unchanged against `PgStore`; ANA-9 §11 criteria 1-4, 6 and 7 are pinned by tests.
Requirements addressed: `R-STO-1..6`, `R-ENT-1..12`, `R-USR-2`; `R-NF-2`, `R-NF-3` by construction.

Artifacts: plan `.claude/plans/mod-6-postgres-store-cache.plan.md` (decisions D1-D14,
verified-claims table, errata note), blueprint
`.claude/plans/mod-6-postgres-store-cache.blueprint.md` (signatures, SQL, mirror DDL, 17 errata
against ANA-9 / the code / sqlx 0.9). Design authority stays `docs/ANA-9.md`.

## What was built

1. **`htui-store` crate** (`crates/htui-store`). `sqlx` 0.9.0 with `runtime-tokio, postgres,
   sqlite, migrate, macros, chrono, uuid, json, tls-rustls-ring-native-roots`. Postgres queries are
   compile-time checked `query!` / `query_as!` with the offline data committed under
   `crates/htui-store/.sqlx/` (68 files) and `SQLX_OFFLINE=true` set by `.cargo/config.toml`, so a
   build never needs a server; a query change is followed by `cargo sqlx prepare -- --all-targets
   --all-features` **from the crate directory**. Mirror queries are runtime `sqlx::query_as`.
   `htui-core` gained an optional `sqlx` feature: transparent `sqlx::Type` on every ID newtype and
   a strong-enum `Type` with `type_name = "text"` and per-variant `rename` on every `str_enum!`.
2. **Migration** `migrations/0001_init.sql`: the 32 tables of ANA-9 §5 in FK order (ANA-9 §3 says
   "thirty"; the entity map draws 32), the `set_updated_at` trigger loop over twenty tables, the
   three forward references to `run_step` added last. `PgStore::connect(dsn, identity)` compares
   the applied set against the embedded one: an applied version the binary does not know refuses
   with "schema is newer than this htui", a checksum drift refuses, pending migrations are reported
   as `MigrationState::Pending(n)` and applied only through `apply_migrations` after the TUI's
   prompt (`R-STO-5`). Verified on Postgres 16.15 (dev container) and 17.11 (throwaway container).
3. **Seed and identity.** First connect seeds one `app_user` (keyed on table emptiness under a
   `SHARE ROW EXCLUSIVE` lock, `R-USR-2`), the ten `capability_tag` rows and the `app_setting`
   defaults (`cache_refresh_seconds` 30, `cache_overlap_seconds` 300). `box.toml` under the config
   root holds `box_id` (UUIDv7) and hostname; `register_box` upserts on `(user_id, hostname)` and
   adopts the database's id when the hostname is already registered. `PgStore` does no file I/O
   itself; `connect::start` loads or mints the identity and persists an adopted id.
4. **`PgStore` reads and writes.** The seven `ReadStore` methods and the four inherent reads
   (`workspaces`, `box_info`, `active_runs`, `projects`) with `MemStore`'s ordering; `items`
   implements every `ItemFilter` field in SQL (readiness per §7.4, text as a literal substring on
   every backend); `links` is a recursive CTE over live edges in both directions, cross-project.
   `mint_item` is §7.1 in one transaction with the kind-in-project guard; `update_item` is §7.2
   with `COALESCE` patch semantics, `Diverged { head, ancestor }` on a lost race and a
   `Constraint` when the patched kind is foreign; `transition` is the status CAS, `closed_at`
   following terminal status both ways, `version` untouched.
5. **`CacheStore` and the refresher.** `cache_migrations/0001_mirror.sql` mirrors the fifteen
   tables of §4.4 (UUIDs as TEXT, timestamps as INTEGER microseconds, arrays and JSONB as TEXT;
   decode goes through helpers because sqlx maps `Uuid` on SQLite to BLOB and cannot read
   microsecond integers as `DateTime`). `<config>/htui/cache/<sha256(host:port/dbname)>/cache.sqlite`
   in WAL mode, rebuilt on a schema-version or fingerprint change, cursors cleared after a
   seven-day-old full refresh. `Refresher` is the mirror's only writer: per project in the current
   scope, tables in FK order, `ts_col > high_water - overlap`, PK upsert, tombstoned links deleted,
   `high_water = max(ts_col)` of the fetched rows; `run_step_commit` rides its parent step's
   `updated_at` and the three unscoped small tables are replaced whole (blueprint H.3, H.4); the
   last N **finished** steps keep their events. The pass ends with the pending upload.
6. **Pending chat buffer.** `pending/<project_id>.<run_id>.jsonl` (one `SessionEvent` per line;
   the project id is in the name because `run.project_id` is `NOT NULL`, blueprint H.5) lands as
   `run` (`chat`), `run_step` (`chat`) and events in one transaction, idempotent on the event
   primary key, file deleted after commit; a malformed or server-refused file is left in place
   and never blocks the next one.
7. **Backend, keyring, shell.** `Backend::{Memory, Online { pg, cache }, Offline { cache, since }}`;
   labels `memory` / `connecting` / `online` / `offline · <n>s|m|h`; writes only through
   `writable() -> Option<&PgStore>`. DSN in the OS keyring (`htui` / `postgres-dsn`), set with
   `htui --set-dsn` from stdin, removed with `--clear-dsn`; the binary has no environment
   fallback (`R-STO-1`), TLS is whatever the DSN says (`R-STO-2`). `--offline` never dials. The
   store worker owns the swap: `Offline → Online` on a successful dial, `MigrationsPending` held
   aside until the `MigrationPrompt` overlay answers `y`, `Online → Offline` when a read or a
   refresh pass returns `StoreError::Unreachable` (sqlx I/O, pool closed or timed out, SQLSTATE
   class 08, 57P01-03), reconnect ticker every 30 s with `MissedTickBehavior::Delay`.
   `StoreRequest::{StoreState, ApplyMigrations}` join the fourth-tick top-bar refresh.
8. **Dev container.** `compose.yaml`: `postgres:16` on host port 5433 (`postgres` / `htui`),
   database `htui`. README covers DSN, container, cache layout, offline mode, the migration
   prompt, the test env var and the `.sqlx` workflow.

## Decisions worth keeping

- **Plan-path override.** The routing table said PRD (C2 + C4 at threshold); the maintainer
  routed as plan because ANA-9 already answered every design question. No ultracode: two parallel
  tasks and two serial ones ran as direct `Agent` calls (Opus for every implementer, Fable for the
  plan and the reviewer, per the maintainer).
- **No `agent` seed rows** (plan D5, deviation from ANA-9 §5.10): `agent.launch` and
  `transport` are ANA-4's shape. MOD-2 seeds them once ANA-4 concludes.
- **No SQLite feature gate** (plan D8): ANA-9 §10 lists it as a mitigation if the bundled build
  ever fails on a box; it built on MSVC here. Reach for it on evidence.
- **Retention sweep and `LISTEN`/`NOTIFY`** are not in this item (plan D14). The importer mint
  variant is MOD-8.
- **Blueprint errata over the plan.** Seventeen recorded in the blueprint §H; three re-probed by
  the router against the live databases before implementation (H.8 strong enum `type_name`,
  H.9 SQLite UUID as BLOB, H.10 microsecond timestamps).
- **Tests are hermetic:** every Postgres test creates and drops its own `htui_test_<hex>`
  database from `HTUI_TEST_DATABASE_URL` (skip with a printed line when unset), config roots are
  per-test temp dirs, keyring tests run against `keyring::mock` (one `#[ignore]`d real-backend
  case), nothing touches `%APPDATA%\htui` or the real credential store.
- **`PoolTimedOut` counts as unreachable.** A transient pool timeout drops the shell to the mirror
  for up to one reconnect interval. Intended (the pool has eight connections and two sequential
  users); it is the one false-positive path of the classifier.

## Process

Selected by `/handoff-run next` (R2, unblocks four items). Plan fact-checked with a scratch probe
crate against rustc 1.98 and, after the blueprint, against the live Postgres 16 and SQLite (12
claims, 10 true by probe or tree, one deferred to T1, one documented behaviour). Implementation:
T1 foundation serial, T2 `PgStore` and T3 cache in parallel git worktrees (disjoint file sets
verified before CONFIRM; both merged clean), T4 shell wiring serial. Review gate: the ecc
`rust-reviewer` prompt on Fable, **request-changes** with two HIGH (running steps frozen in the
mirror; seed keyed on the OS user name), three MEDIUM (no mid-session `Online → Offline`; a
server-refused pending file blocked the rest; keyring tests wrote real credentials), two LOW
(`ILIKE` wildcards; a 10 s test wait) - all seven applied, re-review **approve** with two LOW
follow-ups (seed race across two first-ever connects; a latent spin in `go_offline`), both applied.

## Watch items for later modules

- **MOD-2:** seed the `agent` rows; write `session_event` rows through `PgStore` (chunk
  coalescing stays in the driver); the offline chat writes `pending/<project_id>.<run_id>.jsonl`
  with `SessionEvent` serde lines. `PgStore::revision(id, version)` is private; promote it when
  the three-way view (MOD-13) or MCP (MOD-11) needs it.
- **MOD-7:** the box probe fills what `register_box` leaves empty (`os_version`, CPU, RAM, GPU,
  `box_tool`, tags). `Backend::box_info` on the mirror holds the own row only.
- **MOD-15:** `Settings > Rebuild cache` calls `CacheStore::rebuild()`; project creation seeds
  kinds, graphs and templates (`load_demo` shows the FK order).
- **Backpressure:** request/reply channels stay unbounded (MOD-1 D4); revisit once `PgStore`
  latency is measured under load.
- **ANA-9 §11.5** (warm start under one second) was not measured on a terminal in this session;
  the release binary rendered from the mirror before the dial completed in the agent's logged
  run. Measure on the reference workstation with `HTUI_LOG` timestamps.
- **Linux / macOS** not built (Windows-only box, as in MOD-1). The keyring features chosen are the
  crate's documented defaults for those platforms.

## Validation

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D
warnings`, `cargo test --workspace --all-features` with `HTUI_TEST_DATABASE_URL` (157 tests, 1
ignored) and without it (Postgres suites skip), `cargo test -p htui` (no features), `cargo doc
--workspace --no-deps` (0 warnings), `cargo sqlx prepare --check -- --all-targets --all-features`
from `crates/htui-store`, `cargo build --release -p htui`. Migration and conformance suites also
green on Postgres 17.11. No leaked test databases, temp roots, config dirs or credentials after
the runs. Manual online / offline / migration-prompt run against the dev container recorded in the
T4 report; repeat with:

```
docker compose up -d
echo postgres://postgres:htui@localhost:5433/htui | cargo run -p htui -- --set-dsn
cargo run -p htui                  # connecting → online
docker compose stop postgres
cargo run -p htui                  # from the mirror: offline · 0s, ticking
cargo run -p htui -- --clear-dsn
```

## Commits

- `9b2cfd0` chore(mod-6): dev Postgres compose and fact-checked plan
- `6446e42` docs(mod-6): blueprint with 17 errata, plan amended for the probed ones
- `2b0125f` feat(mod-6): htui-store crate, 0001_init migration, connect, seed, identity, demo loader
- `f8364e6` feat(mod-6): PgStore reads and writes, conformance and ANA-9 criteria suites
- `e977bc8` feat(mod-6): SQLite mirror, CacheStore reads, cursor refresher, pending upload
- `d0fe7b1` test(mod-6): cache suite for ANA-9 criteria 4, 6 and 7 and the rebuild rules
- `73ac577` Merge branch 'worktree-agent-a682d20515c17eabe'
- `8427cf9` feat(mod-6): Backend with Online/Offline, keyring DSN, connect and reconnect, migration prompt
- `f16c972` fix(mod-6): apply rust-reviewer findings F1-F7
- `946bfed` fix(mod-6): lock app_user during the seed (F8), clear the health watch on every go_offline (F9)
