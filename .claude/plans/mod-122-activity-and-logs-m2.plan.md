# Plan: MOD-122 M2 — Worker logs in the main instance

**Source PRD**: `.claude/prds/mod-122-activity-and-logs.prd.md`
**Selected Milestone**: 2 — Worker logs in the main instance
**Complexity**: Large (four tasks, new table and store trait, worker shipper, Logs tab and Boxes settings)
**Status**: drafted, awaiting CONFIRM
**Routing**: PRD path by `/handoff-run MOD-122` (3 criteria fired; resumed at `plan`); ultracode accepted for the implement phase.
**Grounded at**: `a05deeac` (branch `hr/MOD-122`). Requirements: `R-TUI-15`; PRD milestone 2. Touches `R-ID-7` / `R-SEC-3` (nothing off the box unscrubbed).

## Summary

An `htui worker` keeps its log lines in a bounded local buffer and, in batches, writes them to a new
Postgres table `worker_log`. Which lines it ships is set per box from the main instance (Settings ›
Boxes), default `warn`, and a change applies within one cycle without a restart. The Logs tab reads
the table once a second and lists one extra source per worker box under the local sources; a worker
problem is read there without a shell on the worker's box. Stored lines expire at a maximum retention
(default 14 days, `app_setting` `log_retention_days`), a per-box row cap bounds growth, and a box
deleted from the database takes its non-error lines with it while its errors stay until retention.
Every shipped line is scrubbed on the worker first and a line the scrubber refuses is replaced, never
shipped.

## Decisions

PRD open questions that M2 touches are answered here (D2–D6). Maintainer checks are marked *check*.

- **D1 — one narrow trait, not a widening of `WriteStore`/`ReadStore`.** `WorkerLogStore`
  (`htui-core/src/store/worker_log.rs`) has four methods and two implementors, `MemStore` and
  `PgStore`. The memory feature put its methods on `ReadStore`/`WriteStore` and had to touch `Writer`,
  `Backend`, `UsageSpy` and the test doubles; a log table does not need that reach. The TUI reads
  through new inherent `Backend::worker_logs` (Online → Postgres, Memory → `MemStore`, Offline → empty).
  The worker writes through `PgStore` directly, as `spawn_index_job` already does.
  ```rust
  pub trait WorkerLogStore: Send + Sync {
      /// One batch from `box_id`; returns the rows written. Stamps user and box name; another user's box is NotFound.
      fn append_worker_logs(&self, box_id: BoxId, lines: &[NewWorkerLog]) -> impl Future<Output = Result<u64>> + Send;
      /// Rows with `id > after` oldest first, at most `limit`; `None` = the newest `limit`, still oldest first.
      fn worker_logs(&self, after: Option<i64>, limit: u32) -> impl Future<Output = Result<Vec<WorkerLogLine>>> + Send;
      /// Deletes rows received before `older_than`, then rows beyond `per_box_cap` per box (oldest first); returns the count.
      fn prune_worker_logs(&self, older_than: DateTime<Utc>, per_box_cap: u32) -> impl Future<Output = Result<u64>> + Send;
      /// Sets (`Some`) or clears (`None`) `box.settings.log_level`; every other key of the blob survives. `false` = no such box of mine.
      fn set_box_log_level(&self, box_id: BoxId, level: Option<LogShipLevel>) -> impl Future<Output = Result<bool>> + Send;
  }
  ```
- **D2 — batching.** The shipper flushes at **200 lines or 2 s**, whichever comes first, one
  `INSERT … SELECT … FROM UNNEST(…)` per flush (at most 500 rows a statement; a larger batch loops).
  Provisional: T2 measures it (below) and records the number in this plan before the defaults freeze.
  *(PRD open question "batch size and interval" answered, subject to the measurement.)*
- **D3 — worker buffer cap and what is dropped.** 5 000 lines. At the cap the oldest line **below
  `error`** goes first; an `error` line goes only when the buffer holds nothing else. Drops are
  counted and shipped as one `warn` line, `N log lines were dropped while the database was
  unreachable`, with the next successful flush. A failed flush keeps its lines and retries on the
  next cycle with a 2 s → 30 s back-off. *(Open question "worker-side cap" answered.)*
- **D4 — retention.** `app_setting` key `log_retention_days` (integer 1–365, default **14**) read on
  each sweep through `WorkerHost::app_settings()`; plus a cap of **100 000 rows per box**. The sweep is
  `prune_worker_logs`, run by each worker's shipper at start and hourly. It is idempotent, so two
  workers sweeping together is harmless and no lock is taken. It does **not** wait on MOD-100's sweeper
  (open, no code). No Settings field in M2: M3 adds the activity table, and one retention field then
  covers both (`R-TUI-8` surface decided there). *(Open question "default retention and sweeper"
  answered.)* *check*
- **D5 — "errors".** Level `ERROR` only; a `WARN` is an ordinary line. Failure *events* are M3.
  *(Open question "how errors is defined" answered; the PRD's assumption confirmed.)*
- **D6 — deletion with the worker.** `worker_log.box_id` is `REFERENCES box(id) ON DELETE SET NULL`
  and the row keeps `box_name` (the hostname when written). A `BEFORE DELETE` trigger on `box` deletes
  that box's rows with level `<> 'error'` first, so the FK clause only ever touches errors, which stay
  attributable by name and expire at retention. No product path deletes a box today (only test
  helpers, claim V9), so this is a schema-level guarantee tested with a raw `DELETE FROM box`.
- **D7 — per-box shipping level.** `LogShipLevel { Off, Error, Warn, Info, Debug }`, default `Warn`,
  stored in `box.settings.log_level` (a string, read leniently like `executor`: an unknown value
  reads as the default, never fails the whole blob). The worker reads it from its own box row each
  flush cycle (`WorkerHost::box_row`) and applies it live. `Off` ships nothing and the stderr/file log
  is unchanged. Written by `set_box_log_level` (D1) from Settings › Boxes; not part of `BoxEdit`'s
  compare-and-set (it is a level, last write wins, and `edit_version` is not bumped).
- **D8 — how the worker captures.** `init_worker_tracing` keeps today's `fmt` + `sentry` layers
  behind the same `HTUI_LOG_FILTER` (default `info`), now attached **per layer** (`and_then(...)
  .with_filter(env)`), and adds a `ShipLayer` behind its own dynamic `filter_fn` that reads the level
  (an `AtomicU8`). A level change calls `tracing::callsite::rebuild_interest_cache()`: without it a
  callsite first seen while disabled stays disabled (claim V2). The ship layer ignores events below
  `WARN` whose target is `sqlx` / `sqlx::…` or starts with `sqlx_` (claim V18: every statement logs
  `DEBUG` under `sqlx::query`, so at `debug` each flush would ship its own statement log, a feedback
  loop). A slow-statement `WARN` still ships, at most one per slow flush, which is bounded. The shipper
  logs only on state changes (ok → failing, failing → ok), never per flush.
- **D9 — secrets.** Each line is scrubbed on the worker before it enters the buffer, with
  `htui_core::scrub::MinimalScrubber` through the `Scrubber` trait (a `Value::String` leaf, as
  `htui-agent`'s `scrubbed_text` does); a refused line keeps its level and target and its text becomes
  `[line withheld: it matched a credential rule]`. This is defence in depth: the PRD's requirement stays
  that no secret reaches a log call. T2 adds no log call that formats a DSN, key, token or env value,
  and the reviewer checks every new `tracing` call (PRD risk row).
- **D10 — the Logs tab.** The sources list stays `all`, the local sources (M1, unchanged), then one
  row per worker box that has stored lines, labelled `worker · <box name>` (a deleted box:
  `worker · <name> (removed)`). **`all` keeps its M1 meaning, this process only**: a worker's lines carry
  the worker's clock and a merged timeline would imply an order the clocks cannot give. *check*
  - Fetch: `on_activate` and `on_refresh` send `StoreRequest::WorkerLogs { after }` unless one is in
    flight; the first has `after: None` (newest 2 000), later ones re-read from `max_id − 500` and
    de-duplicate by id, so a row a concurrent transaction committed late (lower id, later commit) is
    not missed (claim V10).
  - Each worker source keeps a ring of 10 000 lines (M1's `CAPACITY`); counts are of retained lines.
  - The detail pane, minimum level `v`, follow `F`, `G` and scroll pausing work as for a local source.
    The worker lines' targets map through `Source::of` only for their colour-free `target` column
    under the worker row; they are not split into the local sources.
  - Crossing between a local source and a worker source keeps the follow/paused mode; a paused anchor
    has no meaning in the other sequence space, so it lands on the newest page, paused (M1 B4's "another
    source never moves what a paused reader sees" holds within a space).
  - A failed read is absorbed (`Tab::absorbs_failure("worker_logs")`) so a down database does not fill the status
    line once a second; the last rows stay.
  - History `location`/`restore`: `source:worker:<box id>`; a box with no lines restores to `all`.
- **D11 — Settings › Boxes.** The box detail gets one row, **Worker log level**, edited like the
  executor (`Mode::Executor` in `settings/boxes.rs`): `←`/`→` or the new `boxes.log_level` key cycles
  `off · error · warn · info · debug`, saved at once through `StoreRequest::SetBoxLogLevel`. Every key is
  configurable (`keys.toml`), none reserved. A box that is not a headless worker shows the row with
  "applies to `htui worker` on this box". Local TUI logging stays Settings › Logging (M1).
- **D12 — what is not in M2.** The TUI's own in-process run runtime keeps logging to its memory buffer
  (M1) and does not ship. `htui mcp` is unchanged. No Settings field for retention (D4). No demo-data lines
  for worker logs. The sources list does not search or export (PRD out of scope).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store module | `crates/htui-store/src/pg/relay.rs` (`permission` `:251`, `settle_permissions` `:322`), trait `store/worker.rs` `RelayStore` `:85-101` | A feature's Postgres code in its own file, behind its own trait |
| Migration | `crates/htui-store/migrations/0011_permission_relay.sql`, `0030_link_kinds.sql` | Header comment, forward-only, `clock_timestamp()`, named `chk_` constraints, "not mirrored" note |
| Box settings key | `htui-core/src/model/box_.rs` `BoxSettings` `:323`, `executor_lenient` `:240`; `PgStore::edit_box` `pg/write.rs:2734` (`jsonb_set`, object guard) | Lenient typed key in the blob; one statement that keeps the other keys |
| Worker job | `htui/src/concepts.rs:448` `spawn_index_job(pg)`; use `worker_cmd.rs` `serve` | A task sharing the pool, aborted at shutdown |
| Worker tracing | `htui/src/worker_cmd.rs:299` `init_worker_tracing`; M1 `htui/src/logging/buffer.rs` `MemoryLayer`, `LineVisitor` | `HTUI_LOG_FILTER`, no ANSI in files, a layer that formats before it locks |
| Scrub seam | `htui-core/src/scrub.rs:171` `Scrubber::scrub(&mut Value)`; `htui-agent/src/record.rs:2290` `scrubbed_text` | Fails closed, drop the value on `Unmasked`, never log the text |
| Store request | `htui/src/store_worker.rs` `StoreRequest::name` `:1337`, a reply per request, `Tab::absorbs_failure` | New request + reply + `name` arm; the tab never holds a handle |
| Tab with a pull | `htui/src/ui/tabs/logs/mod.rs` (`pull`, `on_refresh`, `on_activate`, `Scroll`) | Sequence-anchored scroll, `empty::render` with a next step |
| Boxes row editor | `htui/src/ui/tabs/settings/boxes.rs` `Mode::Executor` `:259`, `open_executor` `:538`, `on_executor_key` `:651` | A one-field mode on the box detail, notice on refusal |
| Conformance | `htui-core/src/store/conformance.rs` `run_case` `:430`, `run_case_inline` `:434` | One case body generic over the store, run for `MemStore` and Postgres |
| Tests | `testkit::Harness`, `SectionBench`; Postgres tests `htui-store/tests/*.rs` with `HTUI_TEST_DATABASE_URL` | Snapshot render; real-database tests behind the test DSN |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/worker_log.rs` | CREATE | T1 | `LogShipLevel`, `WorkerLogLevel`, `NewWorkerLog`, `WorkerLogLine` |
| `crates/htui-core/src/model/mod.rs` | UPDATE | T1 | export them |
| `crates/htui-core/src/model/box_.rs` | UPDATE | T1 | `BoxSettings.log_level: Option<LogShipLevel>` (lenient) |
| `crates/htui-core/src/store/worker_log.rs` | CREATE | T1 | `WorkerLogStore` (D1) |
| `crates/htui-core/src/store/mod.rs` | UPDATE | T1 | export |
| `crates/htui-core/src/store/mem.rs` | UPDATE | T1 | `MemStore` impl: a `Vec` with a row id counter in `State` |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | T1 | `worker_log` cases, generic over `WorkerLogStore` |
| `crates/htui-store/migrations/0031_worker_log.sql` | CREATE | T1 | table, indexes, box trigger (D6) |
| `crates/htui-store/src/pg/worker_log.rs` | CREATE | T1 | the four statements |
| `crates/htui-store/src/pg/mod.rs` | UPDATE | T1 | `mod worker_log`; `impl WorkerLogStore for PgStore` forwards |
| `crates/htui-store/src/backend.rs` | UPDATE | T1 | inherent `Backend::worker_logs` |
| `crates/htui-store/.sqlx/query-*.json` | CREATE | T1 | offline entries (generated against a migrated scratch DB) |
| `crates/htui-store/tests/worker_log_pg.rs` | CREATE | T1 | the conformance cases on Postgres, the trigger, the FK, the user scope |
| `crates/htui-store/tests/migrations.rs`, `tests/connect.rs` | UPDATE | T1 | the known count pins (30 → 31) and `TABLES` |
| `crates/htui/src/worker_log/mod.rs`, `ship.rs` | CREATE | T2 | `ShipLayer`, `ShipHandle`, buffer (D3), flusher, level poll, sweep |
| `crates/htui/src/worker_cmd.rs` | UPDATE | T2 | per-layer filter (D8), install the layer, spawn and abort the flusher |
| `crates/htui/src/lib.rs` | UPDATE | T2 | `mod worker_log;` (one line) |
| `crates/htui/tests/worker_log_pg.rs` | CREATE | T2 | worker → Postgres end to end, the load measurement (`#[ignore]`) |
| `docs/htui-worker.md` | UPDATE | T2 | shipping, level, retention, "an `error` line is also a GlitchTip report" |
| `crates/htui/src/store_worker.rs` | UPDATE | T3, T4 | T3: `WorkerLogs` request/reply/`name`; T4: `SetBoxLogLevel` |
| `crates/htui/src/ui/tabs/logs/workers.rs` | CREATE | T3 | per-box rings, de-dup by id, counts |
| `crates/htui/src/ui/tabs/logs/mod.rs`, `logs/snapshots/*` | UPDATE | T3 | worker rows, selection type, fetch, `absorbs_failure`, history |
| `crates/htui/src/ui/tabs/settings/boxes.rs` + snapshots | UPDATE | T4 | the **Worker log level** row |
| `crates/htui/src/keys/catalogue.rs`, `keys/views.rs` | UPDATE | T4 | `Act::BoxesLogLevel`, in the boxes stack |
| `docs/htui-keys.md` | UPDATE | T4 | regenerated (`keys_file.rs` pins it) |

## Tasks

Order and lanes. **T1 first, alone.** Then two lanes in separate worktrees: **lane A: T3 → T4**
(they share `store_worker.rs`), **lane B: T2**. T2 touches none of lane A's files (claim V12), so the
lanes merge without hand edits except the generated `.sqlx` entries, which T1 owns.

### Task 1: The table, the trait, the two stores (serial, first; foundation → one verifier)
- **Action**: tests first.
  - Conformance cases, once, run for `MemStore` and for Postgres:
    - append then read: ids rise, the order is by id, `after` is exclusive, `None` returns the newest `limit` oldest-first;
    - the box name is stamped from the box row, another user's box is `NotFound`, a read returns only my rows;
    - `prune_worker_logs`: rows older than the cut go, the per-box cap keeps the newest, a box's cap does not touch another's;
    - `set_box_log_level`: sets one key and leaves `executor` and `max_concurrent_items`; `None` removes the key; a settings blob that is not an object is a `Constraint`, not a panic; an unknown box answers `false`.
  - `BoxSettings` reads `{"log_level":"debug"}`, absent, and `{"log_level":7}` (→ default, the rest of the blob still read).
  - Postgres only: a raw `DELETE FROM box` removes that box's non-error rows, keeps its error rows with `box_id NULL` and `box_name` intact; the CHECKs refuse an unknown level.
  - `migrations.rs` / `connect.rs` pins move (30 → 31 `Pending`, the `TABLES` list gains `worker_log`), and the connect test that pins `schema_version`.

  Then implement. Migration `0031_worker_log.sql` (forward-only, not mirrored, `MIRRORED_TABLES` unchanged, so `schema_version` moves and each box rebuilds its mirror once; a headless worker never migrates — migrate from a TUI first):
  ```sql
  CREATE TABLE worker_log (
      id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      user_id     UUID        NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
      box_id      UUID        REFERENCES box(id) ON DELETE SET NULL,
      box_name    TEXT        NOT NULL,
      logged_at   TIMESTAMPTZ NOT NULL,                       -- the worker's clock, display only
      received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),  -- the database's; retention keys on it
      level       TEXT        NOT NULL,
      target      TEXT        NOT NULL,
      text        TEXT        NOT NULL,
      CONSTRAINT chk_worker_log_level CHECK (level IN ('error','warn','info','debug','trace'))
  );
  CREATE INDEX idx_worker_log_box ON worker_log (box_id, id);
  CREATE INDEX idx_worker_log_received ON worker_log (received_at);
  CREATE FUNCTION worker_log_box_delete() RETURNS trigger AS $$
  BEGIN DELETE FROM worker_log WHERE box_id = OLD.id AND level <> 'error'; RETURN OLD; END $$ LANGUAGE plpgsql;
  CREATE TRIGGER trg_box_worker_log_delete BEFORE DELETE ON box FOR EACH ROW EXECUTE FUNCTION worker_log_box_delete();
  ```
  Text is capped at 8 KiB per line by the **appender** (truncate at a char boundary, `…` suffix), not by a CHECK. `set_box_log_level`: `UPDATE box SET settings = jsonb_set(settings,'{log_level}',to_jsonb($2::text)) WHERE id=$1 AND user_id=$3 AND jsonb_typeof(settings)='object'` (clear = `settings - 'log_level'`). Regenerate `.sqlx` against a migrated scratch database (memory: the compose `htui` database is empty; in a sandbox use `psql -h localhost -p 5439 -U postgres`).
- **Mirror**: `pg/relay.rs`; `0011`/`0030` headers; `edit_box`'s one-statement `jsonb_set`; the conformance case registration in `run_case_inline`.
- **Validate** (crate-scoped, no workspace gate):
  `cargo test -p htui-core --all-features --lib -- worker_log box_settings --test-threads=1`;
  `cargo test -p htui-store --all-features --test worker_log_pg --test migrations --test connect -- --test-threads=1`;
  `cargo clippy -p htui-core -p htui-store --all-targets --all-features -- -D warnings`; `cargo fmt -p htui-core -p htui-store`;
  `cargo sqlx prepare --check` for `htui-store` against the scratch database.

### Task 2: The worker shipper (lane B, after T1; worktree)
- **Action**: tests first.
  - `ShipLayer` under `with_default` (never a global subscriber in a test): lines at or above the level land in the buffer, below do not; `Off` ships nothing; `sqlx::query` and `sqlx_postgres::…` events below `WARN` are ignored at level `debug`, while a `sqlx` `WARN` ships; a changed level takes effect for a callsite that was disabled before (the V2 case, with the global dispatcher confined to a child process or the test calling `rebuild_interest_cache`).
  - Buffer: the cap drops the oldest non-error first, then an error; the drop counter becomes one `warn` line; a failed flush keeps its lines and backs off 2 s → 30 s; a recovered flush ships the backlog in order.
  - Scrub: a line holding a known credential shape is withheld with level and target kept; a refused line's text appears nowhere (buffer, fake store, tracing output).
  - Level poll: the level comes from `box.settings.log_level` of the worker's own box each cycle; a changed row changes what is shipped within one cycle.
  - The shipper logs on transitions only: a run of ten failing flushes writes one `warn` and a recovery one `info`.
  - Sweep: runs at start and hourly with `log_retention_days` from `app_settings` (missing/invalid → 14), against a fake store that records the cut it was given.
  - Postgres (`tests/worker_log_pg.rs`, `testkit`): a worker's `warn` line is readable through `Backend::worker_logs` within `2 s + one poll`; a level change through `set_box_log_level` is picked up.
  - **Measurement** (`#[ignore]`, run once in the sandbox's Postgres, number recorded here): 10 000 `info` lines/s for 30 s through the real layer and flusher against Postgres: shipped, dropped, p95 flush time, and the effect on a concurrent `claim_run`-shaped query. The defaults of D2/D3 stand if nothing drops and p95 stays under 200 ms; otherwise adjust the constants and say so in the close-out.

  Then implement `worker_log/` (`ShipLayer`, `ShipHandle { set_level, … }`, `spawn_flusher(pg, handle) -> JoinHandle`), change `init_worker_tracing` to the per-layer filter (D8; the existing worker tracing tests must stay green unchanged) and return the handle, and spawn/abort the flusher beside `index_job` in `serve`. Add `docs/htui-worker.md` text.
- **Mirror**: `spawn_index_job`; `MemoryLayer`/`LineVisitor`; `scrubbed_text`.
- **Validate**: `cargo test -p htui --all-features --lib -- worker_log worker_cmd --test-threads=1`;
  `cargo test -p htui --all-features --test worker_log_pg -- --test-threads=1`;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`; the featureless `cargo clippy -p htui -- -D warnings` (memory: `--all-features` hides dead code); `cargo fmt -p htui`.

### Task 3: Store request and the Logs tab (lane A, after T1; worktree)
- **Action**: tests first, through `testkit::Harness`.
  - Store worker: `WorkerLogs { after }` answers the rows from a `MemStore` backend and answers an empty reply offline; its refusal is `Failed { "worker_logs" }`.
  - `workers.rs` unit tests: ring cap 10 000 and counts follow the drop; a repeated id is not added twice; a lower id arriving late is inserted in order (V10); warnings = `WARN` + `ERROR`; a box with no lines is not listed; a deleted box (`box_id None`) is its own `(removed)` source keyed by name.
  - Tab snapshots: no workers (M1's snapshots unchanged byte for byte), two workers with counts, a removed box, the worker row selected with lines coloured by level and the target column, narrow width, empty worker source (next step names Settings › Boxes › Worker log level).
  - Key tests: selecting a worker row, `v`, `F`, `G`, scroll up pauses; crossing local ↔ worker keeps the mode and lands paused on the newest page (D10); a reply while a request is in flight is not doubled; a failed reply leaves the rows and sets no status line; `location`/`restore` round-trip.

  Then implement. The selection type becomes `enum Pick { All, Local(Source), Worker(WorkerKey) }` replacing `Option<Source>`; the M1 code paths keep their bodies.
- **Mirror**: Logs tab `pull`/`Scroll`; the store worker's other read requests; `Tab::absorbs_failure`.
- **Validate**: `cargo test -p htui --all-features --lib -- ui::tabs::logs store_worker --test-threads=1`;
  `cargo insta test -p htui --all-features --lib` limited to the changed snapshots, reviewed one by one; clippy and fmt as T2.

### Task 4: Worker log level in Settings › Boxes (lane A, after T3; same worktree)
- **Action**: tests first, with the existing boxes `SectionBench`.
  - The row renders the current level (default `warn` when the key is absent) with help beneath; `←`/`→` and `boxes.log_level` cycle `off → error → warn → info → debug`; each change emits `SetBoxLogLevel`; a refused or failed write shows the notice and keeps the old level; the row shows a changed-elsewhere note on a stale read.
  - Catalogue invariants (`every_act_has_exactly_one_row`, `names_are_unique_per_context`, `context_tables_and_headings`) and `docs/htui-keys.md` regenerated.
  - Store worker: `SetBoxLogLevel` reaches `set_box_log_level` through `Backend::writable()`; offline it is refused with the usual unreachable sentence.

  Then implement the mode, the request and the key.
- **Mirror**: `Mode::Executor` and `on_executor_key`; `changed_elsewhere_executor`.
- **Validate**: `cargo test -p htui --all-features --lib -- ui::tabs::settings::boxes keys:: store_worker --test-threads=1`;
  `cargo test -p htui --all-features --test keys_file --test box_settings`; clippy and fmt as T2.

## Validation

```bash
bash scripts/gate.sh                                    # the repo gate, by the main thread once per lane
cargo clippy --workspace -- -D warnings                 # featureless
cargo test -p htui --all-features -- --test-threads=1   # memory: suite green is scheduling-dependent
cargo insta test -p htui --all-features                 # full snapshot run, review every .snap.new
```
Manual check, in a PTY or on the host (needs a worker and a TUI on one database):
1. `htui worker` on a box; in Settings › Boxes set its **Worker log level** to `info`.
2. Within a few seconds the Logs tab lists `worker · <box>`; lines carry the worker's `info` output.
3. Lower the level to `error`; no new `info` lines arrive. Stop the database for a minute: the worker keeps running, then ships the backlog and one "dropped" line only if it overflowed.
4. A line with a credential-shaped string in a worker log call appears as `[line withheld …]`.
Owed on the host: live-Qdrant tests (as in M1) and the real-keyring smoke; the Postgres gate on the merged tree; the Windows build.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Shipping floods Postgres or slows runs | Medium | High | Batching and a 5 000-line cap (D2, D3), per-box level default `warn`, `Off`; the load measurement in T2 decides the constants |
| A secret reaches `worker_log` | Medium | High | D9 scrubber on the worker, withheld text; no new call formats a secret; reviewer checks every new `tracing` call; test asserts the refused text is nowhere |
| Shipper logging feeds itself | Medium | High | `sqlx` below `WARN` ignored; transitions only (D8); a test counts the lines one flush adds |
| Late-committed lower id missed by the cursor | Medium | Medium | 500-row overlap and de-dup by id (D10, V10), tested |
| The level change does nothing for a callsite already cached disabled | High without the fix | Medium | `rebuild_interest_cache()` on every change (V2), tested with a callsite hit before and after |
| New migration collides with a sibling run's | High | Low | Known recipe: renumber at collect, move the pins again (memory: new migration moves count pins) |
| Snapshot churn in Logs and Boxes | Medium | Low | M1's snapshots must be unchanged with no workers; changed ones reviewed |
| `store_worker.rs` merge between lane A tasks | Certain, but serial | Low | T3 then T4 in one lane, one worktree |
| Per-layer filter changes the worker's Sentry behaviour | Low | Medium | `fmt` and `sentry` stay behind the same filter via `and_then`; the existing worker tracing tests unchanged; reviewer checks |
| The row cap sweep is slow on a large table | Low | Low | Hourly, indexed by `(box_id, id)`; measured in T2 |

## Verified claims

Checked at `a05deeac`. "Probe" = compile-and-run against this repo's toolchain, `--offline`, in `/tmp/probe122m2`.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | The latest migration is `0030_link_kinds.sql`, so M2's is `0031` | ✓ | Migration listing; `HANDOFF.md:54` live coordinates |
| V2 | A per-layer dynamic level (`filter_fn` over an atomic) works beside a second layer under `EnvFilter`, and changing it needs `rebuild_interest_cache()` under a global dispatcher | ✓ **and amended** | Probe: under `with_default` the level change applied; under `set_global_default` the same callsite shipped nothing after the level rose (`ship []`), and shipped both `DEBUG` lines once `tracing::callsite::rebuild_interest_cache()` ran after each change. The other layer was unaffected. D8 and the T2 test follow |
| V3 | `init_worker_tracing` installs `fmt` (file or stderr) and `sentry_tracing::layer()` behind one global `EnvFilter` (`HTUI_LOG_FILTER`, default `info`) | ✓ | `worker_cmd.rs:299-328` (read) |
| V4 | A job sharing the worker's pool is spawned beside `serve`'s loop and aborted after it | ✓ | `worker_cmd.rs` `serve` (read): `index_job = concepts::spawn_index_job(pg.clone())`, `index_job.abort()`; `concepts.rs:448` |
| V5 | The worker reaches its own box row and `app_setting` rows through `WorkerHost` | ✓ | `store/worker.rs:534-653` (read): `box_row`, `app_settings` |
| V6 | `Backend::writable()` yields only a `PgStore`; `Memory` answers `None` | ✓ | `backend.rs` (read) — which is why D1 gives the TUI an inherent read and the worker `PgStore` |
| V7 | `Scrubber::scrub(&mut Value)` fails closed with `Unmasked` and its text is never in the error; `scrubbed_text` is private to `htui-agent` | ✓ | `scrub.rs:166-209`, `record.rs:2290` (read) |
| V8 | `BoxSettings` is `#[serde(default)]` with a lenient `executor` and `edit_box` writes one key with `jsonb_set` and an object guard | ✓ | `box_.rs:323-337`, `pg/write.rs:2734-2816` (read) |
| V9 | No product code deletes a `box` row; only tests and `testkit::demo_db` do | ✓ | Text search `DELETE FROM box`: `testkit.rs:177`, `tests/migrations.rs:4972`; `cache/refresh.rs:673` deletes from the SQLite mirror |
| V10 | A `GENERATED … IDENTITY` id can commit out of order across concurrent inserters | ✓ (Postgres semantics, not probed here) | Standard: sequences are assigned at statement time, visible at commit. Mitigation D10; the late-id test in T3 pins the client side, the real-database ordering is not claimed |
| V11 | `LogLine.source` is a `Copy` enum and `SourceCounts` a fixed array, so worker sources cannot reuse them | ✓ | `logging/buffer.rs`, `source.rs` (read) |
| V12 | File sets: T1 ∩ T2 = T1 ∩ T3 = T1 ∩ T4 = ∅; **T2 ∩ T3 = T2 ∩ T4 = ∅**; **T3 ∩ T4 = {`store_worker.rs`}** | ✓ by listing | The "Files to Change" table. T3 and T4 are therefore **not** independent: one lane, serial. T1's `.sqlx` and `Cargo.lock`-free changes touch no later task's files |
| V13 | New migrations move count pins | ✓ | `tests/connect.rs:140,242`, `tests/migrations.rs:134,4168,4269,4278,4301,4455` (search); `TABLES` and `schema_version` pins found by running both tests once |
| V14 | `MIRRORED_TABLES` is a 22-entry list that a non-mirrored table does not join | ✓ | `cache/mod.rs:47`; migration `0005`/`0011` headers state the same practice |
| V15 | `.sqlx` offline entries exist (478 files) and a new `query!` needs one | ✓ | `crates/htui-store/.sqlx`; memory "sqlx prepare needs a migrated scratch DB" |
| V16 | The Logs tab's scroll anchor is a sequence in one space | ✓ | `logs/mod.rs` `Scroll::At(u64)`, `top()` (read) — D10's crossing rule follows |
| V17 | `StoreRequest::name` is an exhaustive match, so a new variant needs an arm | ✓ (sweep) | `store_worker.rs:1337-1514` (explore) |
| V18 | `sqlx` logs each statement under target `sqlx::query` | ✓ **and amended** | Probe `/tmp/probe122sqlx` (sqlx 0.9.0, the sandbox Postgres): one `SELECT 1` emitted `DEBUG sqlx::query`; the connect emitted `DEBUG sqlx_postgres::options::pgpass`. D8's filter covers `sqlx`, `sqlx::…` and `sqlx_…`; T2's feedback-loop test counts the lines one flush adds at `debug` and requires zero |

## Acceptance
- [ ] All tasks complete, TDD order; T1 verified; lane B and lane A merged and the gate green on the merged tree
- [ ] Featureless clippy clean; full insta run reviewed; M1 Logs snapshots unchanged with no workers
- [ ] A worker's `warn` line is readable in the Logs tab within `2 s + one poll`; the level changes live from Settings › Boxes
- [ ] No secret reaches `worker_log` (tests on the shipping path; every new `tracing` call reviewed)
- [ ] The measurement recorded in this plan; constants D2/D3 confirmed or changed
- [ ] Retention, cap, and the box-delete trigger tested on Postgres
- [ ] Patterns mirrored, not reinvented
- [ ] Owed on the host and said so: live-Qdrant tests, the Postgres gate on the merged tree, the Windows build, the migration renumber at collect
