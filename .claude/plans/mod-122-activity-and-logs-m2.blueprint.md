# Blueprint: MOD-122 M2 — Worker logs in the main instance (T1–T4)

**Plan**: `.claude/plans/mod-122-activity-and-logs-m2.plan.md` (D1–D12 settled; not reopened here).
**Anchored at**: `hr/MOD-122` @ `a9da81f4`. No source file is written by this blueprint.
Findings amend the plan where the code disagrees. None falsifies a decision's substance. F8 changes
D11's key detail, F9 changes T4's write path, and F12 widens V12's file sets without breaking its
independence claim.

## Findings

**F1 — Conformance registration (T1).** *Plan:* the cases go in `run_case_inline` and run for
MemStore and Postgres. *Code:* `run_case<S: WriteStore>` (`conformance.rs:430`) and
`run_case_inline` (`:434`) are bound to `WriteStore`. The precedent for a second bound is
`READ_CASES` (`:1314`) + `run_read_case<S: ReadStore>` (`:1337`) + `run_all_reads` (`:1371`),
bound in `htui-core/tests/mem_store.rs:22-25` and `htui-store/tests/pg_conformance.rs:117-128`. If
the cases go into `CASES`, two things follow: `run_case`'s bound widens, and the
`EXPECTED_CASES = 290` pins move (`pg_conformance.rs:61`, `mem_store.rs:36`). *Delta:* add a third
list, `WORKER_LOG_CASES`, with `run_worker_log_case<S: WriteStore + WorkerLogStore>` and
`run_all_worker_logs`. Box every arm (`Box::pin`, the MOD-137 note at `:426-429`). Bind the list in
`mem_store.rs` (`MemStore::demo()`) and in `pg_conformance.rs`: a third
`#[cfg(feature = "demo")] #[tokio::test(flavor = "multi_thread")]` loop, one
`Box::pin(common::demo_db())` per case, `db.drop_db()` after. That keeps every conformance list in
one Pg file, and `tests/worker_log_pg.rs` keeps only the Pg-only tests. Extend
`case_names_are_unique` (`:31462`) to all three lists and add
`run_worker_log_case_accepts_every_name` beside `:31611`. Pin `WORKER_LOG_CASES.len()` in
`mem_store.rs`'s `demo_store_loads_the_fixture`. The bound is `WriteStore +` because the
blob-survives case needs `edit_box` and `boxes()`. No feature work is needed: htui-store's dev-deps
switch on `test-support` (`htui-store/Cargo.toml:58-59`), and the DSN is `HTUI_TEST_DATABASE_URL`.

**F2 — What a generic case cannot plant (T1).** *Plan:* the conformance cases include another
user's box (`NotFound`), a box's cap leaving another box alone, and a non-object blob
(`Constraint`). *Code:* the fixture has one user (`ids::USER`) and one box (`ids::BOX`,
`fixtures.rs:142-146`). No trait method creates a box or a user, and nothing can write a non-object
`settings` blob (`edit_box` guards it). The existing "another user's box" proofs are per backend:
`mem.rs:13036` (`MemStore::from_demo` with a pushed `AppUser` and `BoxRow`) and
`box_identity.rs:688` (raw SQL plant). *Delta:* the generic cases cover these:
- an unknown `BoxId` is `NotFound`;
- order, exclusive `after`, and `None` returning the newest `limit` oldest-first;
- the age cut, and the cap within `ids::BOX`;
- set and clear keep the blob minus `log_level` (compare whole objects, not named keys);
- an unknown box answers `false`.

The foreign user, the second box's cap and the non-object blob go to `mem.rs` unit tests (the
`from_demo` plant) and to `worker_log_pg.rs` (the SQL plant). **Gotcha:**
`every_cross_referenced_test_name_exists` (`conformance.rs:~31495`) panics on a doc comment in
`conformance.rs` that names a `worker_log_pg.rs::…` test, because that file is not in its table.
Put such cross-references in `mem.rs`, or add the file to the table.

**F3 — Postgres module layout (T1).** *Plan:* `pg/mod.rs` gets `mod worker_log` plus an
`impl WorkerLogStore for PgStore` that forwards. *Code:* the narrow-trait impls for `PgStore` live
in `htui-store/src/worker.rs` (`RelayStore` `:59`, forwarding by path), not in `pg/mod.rs`.
`pg/relay.rs` exposes `pub(super) async fn x(store: &PgStore, …)` free functions, called from the
sibling `write.rs:9501`. A `pub(super)` function in `pg/worker_log.rs` is not reachable from
`crate::worker`. *Delta:* put `impl htui_core::store::WorkerLogStore for PgStore` **inside**
`pg/worker_log.rs`, which is a child of `pg`, so it can read the private `pool` field. `pg/mod.rs`
gets one line, `mod worker_log;`, in the module list (`:7-12`). The `UNNEST` insert mirrors
`record_box_probe` (`write.rs:2611-2617`: `INSERT … SELECT $1, t.… FROM UNNEST($2::text[], …) AS
t(…)`) and `create_phase_agents` (`:4216`). `timestamptz[]` binds from `&[DateTime<Utc>]`. One
`query!` text serves every 500-row chunk, so there is one `.sqlx` entry (memory: the hash is the
literal query). For the non-object guard, reuse `BOX_SETTINGS_NOT_AN_OBJECT` (`traits.rs:3135`)
and `edit_box`'s miss re-read (`write.rs:2797-2812`): the `UPDATE`'s `WHERE` refuses, then one
re-read tells `false` (no such box of mine) from `Constraint`.

**F4 — Migration pins (T1).** Exact list:

| File:line | Pin | Change |
|---|---|---|
| `migrations.rs:25-31` | doc "The 52 tables …" | 53, `0031_worker_log.sql` |
| `migrations.rs:33-95` `TABLES` | list | append `// 0031_worker_log.sql (MOD-122)` `"worker_log"` |
| `migrations.rs:125-134` | `vec![1..=30]` and message | add `31` and the file name |
| `migrations.rs:150-152`, `:164-171` | `TABLES.len(), 52` and the "+1" message | 53 |
| `migrations.rs:4168-4169` | `Pending(30)`, "thirty" | 31 |
| `migrations.rs:4269-4270`, `:4278-4279`, `:4301-4302` | `Pending(30)`, `MigrationsPending(30)` | 31 |
| `migrations.rs:4453-4456` | `count(_sqlx_migrations) == 30` | 31 |
| `connect.rs:140-142`, `:155-158`, `:242-244` | `Pending(30)`, `pending, 30` | 31 |

There is **no `schema_version` pin**. The plan's "the connect test that pins `schema_version`" does
not exist: `cache.rs:3666` asserts a synthetic `2`. `pg_criteria.rs:3345` `TABLES: [&str; 31]` is
`delete_project`'s cascade chain, which `worker_log` is not part of. `MIRRORED_TABLES`
(`cache/mod.rs:47`) is unchanged, so no `connection__confirm` snapshot moves.

**F5 — `Backend::worker_logs` (T1).** *Code:* the three-arm inherent pattern is `Backend::box_row`
(`backend.rs:595-601`). Its Offline arm for server-only tables is `Err(orchestration_offline())`.
D1 chose Offline → empty, which is fine: the Logs tab absorbs failures either way. The arms:
- Memory: `htui_core::store::WorkerLogStore::worker_logs(store, after, limit)` **by path**, because
  the trait method has the same name as the inherent one.
- Online: the same call, on `pg`.

*Consequence for T3:* an Offline `Ok(vec![])` must **merge** (add nothing), never replace the
rings. An `Unreachable` from the Online arm goes through `go_offline` (`store_worker.rs:2408-2412`).
The store worker passes `limit = 2000`.

**F6 — Reading `log_level` (T1, T2, T4).** *Plan:* `BoxSettings.log_level` is read leniently, like
`executor`. *Code:* a bad sibling key fails `BoxSettings`'s whole decode (`max_concurrent_items` is
`Option<u32>`). That is why I-1 reads `Executor::of(&settings)`, one key (`box_.rs:212-222`; doc
`:331-335`). *Delta:* add `LogShipLevel::KEY = "log_level"` and `LogShipLevel::of(&Value) -> Self`:
missing or unknown → `Warn`, non-object → `Warn`. The worker's level poll (T2) and the section's
row (T4) both read through it. `BoxSettings.log_level` stays for display and round-trips.
`BoxSettings` derives `Serialize, Deserialize, Eq`, so `LogShipLevel` must too. Update its doc
("the blob's only writer is `edit_box`", `box_.rs:316-320`).

**F7 — T2 tracing details.**
- (a) *Composition is proven.* M1's `logging/mod.rs:115-141` already does
  `sentry().with_filter(env).boxed()` and `MemoryLayer.and_then(fmt).with_filter(env)`. `EnvFilter`
  is not `Clone`, so `fmt_layer.and_then(sentry_tracing::layer()).with_filter(env)` (one parse) is
  the right shape. The `fmt` layer is already `.boxed()` (`worker_cmd.rs:308-321`).
- (b) *sqlx logs through `log`.* A bridged event's metadata target is `log`. The real target is the
  field `log.target`, which M1's `LineVisitor` reads (`logging/buffer.rs:228-252`). The ship
  `filter_fn` sees the real target only on the bridge's `enabled` pass. `on_event` sees `log`.
  Apply the sqlx exclusion **also** in `on_event` on the visitor's `log_target`, and take the
  target column from it, as `MemoryLayer::on_event` does (`:195-206`). Test with M1's bridged shape
  (`buffer.rs:438`). `LineVisitor` is private, so either `pub(crate)` it (this adds
  `logging/buffer.rs` to T2's files) or copy it.
- (c) *Max-level hint.* `try_init` sets the `log` crate's max level once, from the hint at init.
  `rebuild_interest_cache` does not move it (only `reload` does). Build the ship filter as
  `filter_fn(..).with_max_level_hint(LevelFilter::DEBUG)` (the static ceiling of `LogShipLevel`) so
  the init-time bridge level already admits `debug`. The cost is that every bridged `debug` record
  is built, then refused.
- (d) *Where the handle flows.* `init_worker_tracing` runs in `run` (`worker_cmd.rs:66`), before
  `serve`, and `pg` exists only in `serve` (`:94`). Return `anyhow::Result<ShipHandle>`, pass it as
  `serve(args, ship)` (`run`'s signature is unchanged), spawn the flusher beside `index_job`
  (`:117`) and abort it beside `:139`.
- (e) *Scrubber.* Use `MinimalScrubber::new([])` (patterns only; precedent
  `backlog/detail/prompt.rs:642`). `PATTERN_RULES` (`scrub.rs:38-67`) has no DSN-userinfo rule, and
  the DSN is wiped after connect (`worker_cmd.rs:101`), so it is not kept for masking. T2's
  withheld-line test needs a pattern shape (for example `ghp_` + 30 alphanumerics), not a DSN.
- (f) *Fake store.* Don't bound the flusher on `WorkerHost`: 31 methods, plus `Store: WorkerStore`
  with 71. Mirror `IndexSource` (`concepts.rs:257-271`): a crate-local trait implemented for
  `PgStore` through the inherent `PgStore::box_row` (`pg/read.rs:2191`) and
  `PgStore::app_settings`, which are inherent so no `WorkerHost` import is needed, plus
  `WorkerLogStore` by path. Keep the loop generic and `spawn_flusher(pg: PgStore, …)` concrete, as
  `spawn_index_job` (`concepts.rs:447-466`) is. The retention reader mirrors `sync_interval`
  (`concepts.rs:248-255`).
- (g) *Module visibility.* `pub mod worker_log;`, not `mod`: `crates/htui/tests/worker_log_pg.rs`
  must reach `spawn_flusher` and `ShipLayer`. Every `lib.rs` module is `pub` (`:19`, `:28`, `:53`).
- (h) *Locking.* Format and scrub before taking the buffer lock, and never emit a `tracing` event
  while holding it: the layer would re-enter and deadlock.
- (i) *Existing tests.* No unit test calls `init_worker_tracing`. The binary tests in
  `tests/worker_pg.rs` (`:1691`, `:1857`, `:1884`) cover it end to end and now also run the
  flusher. They must stay green.
- (j) *No `query!` in htui.* Runtime-checked SQL only (`htui/Cargo.toml` dev-dep `sqlx` comment), so
  the htui crate adds no `.sqlx` entries.

**F8 — D11's keys meet section navigation (T4).** *Plan:* the row is "edited like the executor
(`Mode::Executor`): `←`/`→` or `boxes.log_level` cycles". *Code:* `Mode::Executor` is a **y/n
confirmation** opened by `w` over an `edit_version` compare-and-set (`boxes.rs:259-317`
`ExecutorFlip`, stack `BOXES_EXECUTOR` `views.rs:560`). It is not a cycler. In `BOXES_BROWSE`
(`views.rs:533-549`), `←`/`→` (and `h`/`l`/`[`/`]`) resolve to `settings.next_section` /
`prev_section` through the `SETTINGS` layer (`views.rs:67`; defaults `catalogue.rs:2597-2600`).
`tests/no_hardcoded_keys.rs` forbids raw `KeyCode`s. *Delta:* a Browse act `Act::BoxesLogLevel`,
`settings.boxes.log_level`, default `v`. `v` is free in this stack: its other rows are in the
BacklogRuns, BacklogDocs, BacklogReqs and Logs contexts. Over a listed box, the act cycles
`off → error → warn → info → debug` and sends `SetBoxLogLevel` at once, gated by `blocked()` as
the other writes are. Drop `←`/`→`. Keeping them would need a new mode with the `PICK` layer
(`views.rs:178`), a new stack, `stack.rs` `DECLARED` (`:254-259`), the `views.rs` stack list
(`:1407`) and `DELEGATES` (`:1484`). The plan's "mirror `on_executor_key`" does not apply.
*Maintainer FYI at review.*

**F9 — T4's write path is the `Writer`, not `writable()` (T1, T4).** *Plan:* `SetBoxLogLevel`
reaches `set_box_log_level` "through `Backend::writable()`". *Code:* box requests are served by
`crate::box_settings::serve` (`box_settings.rs:93-153`) over `backend.writer()`, which is
`Writer::Memory` or `Writer::Online` (`writer.rs:61-66`). `writable()` is `None` on Memory
(`backend.rs:138-143`), so every `SectionBench` and Harness test would be refused. *Delta:* T1 adds
`impl htui_core::store::WorkerLogStore for Writer` in `htui-store/src/worker.rs`, beside the
`RecorderStore for Writer` precedent, forwarding per arm. T4 then:
- routes `SetBoxLogLevel` into the box arm of `try_serve` (`store_worker.rs:2547-2549`; update its
  "three" comment);
- adds an arm in `box_settings::serve`: `true` → `Boxes(fresh)`, `false` (box gone) →
  `BoxesStale(fresh)`, the `EditBox` `NotFound` rule;
- grows `REQUEST_NAMES` from `[&str; 3]` to `[&str; 4]` (`box_settings.rs:158`) and updates the
  name-arm comment (`store_worker.rs:1352-1356`);
- adds a `doing()` phrase (`boxes.rs:194-201`, pinned by `a_write_in_flight_reads_in_words`
  `:1363-1372`);
- adds the `Failed` arm in `on_reply` (`:851-857`);
- adds the new request to the offline zip in `tests/box_settings.rs:493-494`.

There is no token, so T4's test "a changed-elsewhere note on a stale read" cannot happen. Replace it
with "a box gone answers `BoxesStale`, and the section says `DELETED_ELSEWHERE`".

**F10 — Keys and snapshot churn (T4).**
- Catalogue: `CATALOGUE.len()` 246 → 247 (`catalogue.rs:3283-3284`); append to `ALL` and
  `position` (`Act::BoxesLogLevel => 246`, after `:3243`); the row in the `[settings.boxes]` block
  (`:1475-1508`); the act in the `BOXES_BROWSE` layer; check `the_view_stacks_resolve_as_designed`
  (`views.rs:1626`).
- Key reference: regenerate `docs/htui-keys.md` with
  `cargo run -q -p htui -- --default-keys --print-keys` and paste the output into its ```` ```toml ````
  block (pinned by `keys_file.rs:727-745`).
- Snapshots: the Boxes snapshots are **integration** snapshots,
  `crates/htui/tests/snapshots/box_settings__*.snap` (10 files, from `tests/box_settings.rs`), not
  under `settings/`. A new `HINT_BROWSE` entry (`boxes.rs:98-106`) moves every browse frame,
  `box_settings__demo_80` included, whose 80-column fit was already tight (MOD-51 F-10, hence
  `p probe`). The detail row moves `demo`, `two_boxes`, `stale` and `offline`.

**F11 — T3 request plumbing.**
- *Sending.* `Ctx::request` (`app/state.rs:397-399`) is available in `on_refresh` and `on_activate`
  (`registry.rs:113`, `:119`). `on_activate` runs right after `wants_requests` is issued
  (`state.rs:1003-1013`), so `wants_requests` stays empty.
- *In-flight flag.* The freshness gate keys on `(origin, discriminant)` (`state.rs:1018-1043`): a
  second `WorkerLogs` sent while one is out drops the first one's reply. The flag is therefore
  required. Clear it on the `WorkerLogs` reply and on `Failed { request: "worker_logs" }`. Replies
  reach an inactive tab (`app/update.rs:451-453`), and `forget()` touches only overlays.
- *Failures.* Absorb them through `Tab::absorbs_failure` (`registry.rs:66`, asked at
  `update.rs:484-497`).
- *Request and reply shape.* `StoreReply` derives only `Debug, Clone` (`store_worker.rs:1517`):
  `WorkerLogLine` needs `Debug + Clone`, plus `PartialEq` for tests. Add the `worker_logs` and
  `set_box_log_level` assertions to `name_arms_are_stable` (`store_worker.rs:5995`). `WorkerLogs` is
  served inline in `try_serve` like `ProjectTypes` (`:2567-2572`), as one read.
- *M1 snapshots.* The six `logs_*` snapshots stay byte-identical: `MemStore`'s `worker_logs` is empty
  under `Harness::empty()` and `demo()`.

**F12 — V12 file-set independence.** *Confirmed*, with additions:
- T1 also writes `htui-store/src/worker.rs` (the F9 `Writer` impl) and `pg/worker_log.rs` (F3).
- T2 may write `logging/buffer.rs` (F7b). No other task touches `logging/*`: T3 only reads
  `logging::{Source, CAPACITY}`.
- T4 also writes `crates/htui/src/box_settings.rs`, `crates/htui/tests/box_settings.rs` and
  `crates/htui/tests/snapshots/box_settings__*.snap`. It writes no `keys/stack.rs` under F8's
  Browse-only design.

T2 ∩ T3 = T2 ∩ T4 = ∅ holds, provided T2 keeps its documentation to `docs/htui-worker.md`.
`README.md:108` (the `--log` row) and `:450` (the Settings table) are tempting to both lanes, so
give README edits to lane A only. T3 ∩ T4 = {`store_worker.rs`} holds: T3 adds no key or act, and T4
adds no Logs file. No `Cargo.toml` changes: `tracing-subscriber`, `sentry-tracing` and `htui-core`
are already htui dependencies, and `log` is already a dev-dependency. `lib.rs` is T2's alone.

**F13 — MemStore details (T1).** `from_demo` builds `State` with an explicit field list and no
`..Default` (`mem.rs:421-506`), so the new fields (`worker_logs`, `worker_log_next_id`) go there
too. Stamp `received_at` from `self.clock.now()` (`MemClock`, `mem.rs:128-131`). Have
`set_box_log_level` stamp `row.updated_at`, as `edit_box` stands in for the `set_updated_at`
trigger (`mem.rs:2802-2806`), and leave `edit_version` alone. `this_user()` is an `Option`
(`mem.rs:404-417`): `None` means `NotFound` on append and an empty read.

**F14 — Prune and level-type details (T1).**
- *Prune scope.* Make `prune_worker_logs` database-wide (`R-USR-2`: one `app_user` per database)
  and say so in its doc.
- *Cap partition.* Cap only `WHERE box_id IS NOT NULL`. `PARTITION BY box_id` would put every
  removed box's error rows into one capped partition, which breaks D6's "errors stay until
  retention".
- *Level type.* Map `WorkerLogLevel` either as text through
  `#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]`, like the other status enums (htui-store
  enables htui-core's `sqlx` feature, `htui-store/Cargo.toml:25`), or decode the string by hand.

## Per-task notes

### T1 — registration points
- `htui-core/src/store/mod.rs`: `mod worker_log;` beside `mod worker;` (`:10`); `pub use
  worker_log::WorkerLogStore;` beside `:63`. Trait signature as in D1, with `fn … -> impl Future<…>
  + Send` (the P-2 form of `store/worker.rs`).
- `htui-core/src/model/mod.rs`: export `LogShipLevel { KEY, of }`, `WorkerLogLevel`,
  `NewWorkerLog`, `WorkerLogLine`.
- `mem.rs`: the `impl WorkerLogStore for MemStore` goes in `mem.rs`, or in `store/worker_log.rs`
  beside the trait, as `store/worker.rs` does for its traits. `State` fields: F13.
- `conformance.rs`: `WORKER_LOG_CASES`, `run_worker_log_case`, `run_all_worker_logs` after
  `run_all_reads` (`:1371-1381`). Tests: F1.
- `htui-store`: `migrations/0031_worker_log.sql`; `pg/worker_log.rs` (statements and the trait
  impl, F3); `pg/mod.rs:7-12` `mod worker_log;`; `worker.rs` `impl WorkerLogStore for Writer`
  (F9); `backend.rs` `pub async fn worker_logs(&self, after: Option<i64>, limit: u32) ->
  Result<Vec<WorkerLogLine>>` (F5).
- Tests: `tests/pg_conformance.rs` (the third loop), `tests/worker_log_pg.rs` (trigger, FK, CHECK,
  foreign user, second box's cap, non-object blob), `tests/migrations.rs` and `tests/connect.rs`
  (F4).
- Validate filter addition: `cargo test -p htui-core --all-features --test mem_store` (the binding
  lives there, not in `--lib`).

### T2 — anchors
- `worker_cmd.rs`: `init_worker_tracing` `:299-328` → returns `ShipHandle`; `run` `:65-72` passes it
  on; `serve` `:80` gains a `ship` parameter; spawn at `:117`; abort at `:139`.
- New `crates/htui/src/worker_log/{mod.rs, ship.rs}`. `lib.rs`: `pub mod worker_log;` (F7g).
- Shape: the ship filter is `filter_fn(move |m| level_admits(m) && !is_sqlx_below_warn(m))
  .with_max_level_hint(LevelFilter::DEBUG)`. `on_event` re-checks the bridged `log.target` (F7b).
  A level change calls `tracing::callsite::rebuild_interest_cache()`.
- The store seam is a crate-local trait implemented for `PgStore` (F7f). The level comes from
  `LogShipLevel::of(&row.settings)` (F6). Retention: `app.get("log_retention_days")
  .and_then(Value::as_u64).filter(1..=365)`, else 14.

### T3 — anchors
- `store_worker.rs`: `StoreRequest::WorkerLogs { after: Option<i64> }`; the name arm (exhaustive,
  `:1337-1513`); `StoreReply::WorkerLogs(Vec<WorkerLogLine>)`; served inline in `try_serve` as
  `backend.worker_logs(*after, 2000)`. Assertion: F11.
- `ui/tabs/logs/mod.rs`: `selected: Option<Source>` (`:~93`) becomes `Pick`; `on_refresh` and
  `on_activate` (`:~255-260`) send under an `in_flight: bool`; `on_reply` (`:~245`); add
  `absorbs_failure`; `location` and `restore` (`:~266-290`).
- New `ui/tabs/logs/workers.rs`, declared in `logs/mod.rs` (not in `lib.rs`).

### T4 — anchors
- `catalogue.rs`: `Act::BoxesLogLevel` (enum near `:571`, row `:1475-1508`, `ALL`, `position` 246,
  count 247). `views.rs:533-549`: add it to the `BOXES_BROWSE` layer.
- `box_settings.rs` and `store_worker.rs`: F9. `boxes.rs`: the detail row in `detail()`
  (`:~1000-1140`), reading `LogShipLevel::of`; with `Executor::of(..) != Worker` it carries
  "applies to `htui worker` on this box"; the `on_key` Browse arm (`:797-832`); `HINT_BROWSE`
  (`:98-106`); a `busy` name.
- Snapshots: `crates/htui/tests/snapshots/box_settings__*.snap`, through `cargo insta test -p htui
  --all-features --test box_settings`.
