# Blueprint: MOD-122 M3 — Activity panel (T1–T7)

**Plan**: `.claude/plans/mod-122-activity-and-logs-m3.plan.md` (D1–D13 settled, maintainer answers
folded; not reopened here).
**Anchored at**: `hr/MOD-122` @ `e99acd97`. This blueprint writes no source file.
Findings amend the plan where the code disagrees, numbered F1… and naming the task sections they
change. None reverses a decision. F2 narrows D2's triggers. F3 replaces D2's duplicate guard. F6
removes one `Backend` method. F8 sets where D10's routing goes.
Postgres probes ran on the sandbox's PG 18.7 in a scratch database, which was dropped afterwards.

## Findings

**F1 — Park-rule parity: one pure test against `views`, one cross-store leg in conformance (T1, T2).**
*Plan:* `htui-worker/tests/it/activity_parity.rs` builds the three shapes through MemStore writes
and compares the result with `htui_worker::waiting`. *Code:* `waiting`'s run rules call
`selectable_slots` (`views.rs:455`, private). It decodes the run's snapshot, and `select_of` checks
the phase, so the shapes need runs whose snapshot decodes to a fan-out phase. Store writes cannot
build that cheaply. The view tests' own fixtures (`slot`, `interrupted_park`, `resolved_selection`,
`undecodable`, `views.rs:1588-1645`) are pure data in the private `mod tests`. A test in
`tests/it` cannot reach them. On its own, T1's test only proves Rust against Rust. The plpgsql
function is held only if the conformance cases run on both stores, and the plan already binds
`ACTIVITY_CASES` in `pg_conformance.rs` (T2).
*Delta:*
- T1 drops `htui-worker/tests/it/activity_parity.rs` and the `tests/it/main.rs` line. T1 instead
  owns the test module of `crates/htui-worker/src/views.rs` (tests only, no other task touches the
  file). There it adds `park_kind_agrees_with_waiting`. The test walks the fixture list of
  `every_parked_run_owns_a_row` (`views.rs:2148-2177`). For each run that is `AwaitingApproval`
  with no step parked and that owns a `JudgeFailed`/`Selection`/`Interrupted` row, it asserts
  `park_kind(steps) == (row.reason as kind, row.step)`.
- The same test pins a named **divergence list**, so that any drift fails it:
  - `undecodable(...)` shapes: `waiting` says `Interrupted` and has no rest step. `park_kind` reads
    no snapshot.
  - a slot whose candidates are all failed, with a failed judge: `waiting` says `Interrupted`.
    `park_kind` says `JudgeFailed`.
  - the interrupted step: `waiting` uses the cursor's rest step, `park_kind` uses the newest
    `failed` step. Assert that they agree on `interrupted_park`.
  - a run that already owns an `Unblock` row (`crashed_rejection`, `escalation`): `waiting` adds
    no run row, while the store writes `interrupted` beside `unblock`.
- T1 `run_park_kinds` (conformance, so both stores) builds its shapes with the
  `select_fanout_is_one_transaction` helpers (`gated_step`, `new_run_step(run, pos, att, idx)`,
  `answer_gate(.., Rejected, Some(note))` for a failed judge with a note, `conformance.rs:14242`).
  It reads the steps back and asserts the written kind and step equal **both** the literal
  expectation and `park_kind(&steps)`. That gives plpgsql == `park_kind` on every store, and
  `park_kind` == `waiting` comes from the test above. Shapes to cover:
  - the plan's three shapes;
  - a failed judge with no note → `selection` (`views.rs:696-703`);
  - a slot with a `running` candidate → `interrupted`;
  - two slots where only the newer one is unselected → `selection` on the newer slot.
- The fixture's own parks write `gate` events, so assert on the run-subject event only.
- Gotcha (M2 F2): `every_cross_referenced_test_name_exists` panics if a `conformance.rs` doc
  comment names a `views.rs::…` test. Put that cross-reference in `model/activity.rs`.
- T1 Validate: replace the nextest line with
  `cargo test -p htui-worker --all-features --lib -- views::tests::park_kind`.

**F2 — Insert triggers only on `step_permission`; narrow every `WHEN` (T2, T1).** *Plan:* there is
an `AFTER INSERT` trigger (no `WHEN`) on all four tables, and MemStore treats "absent before" as an
insert. *Code:* product inserts never create a row that should write an event:
- `run`: `queued` (`write.rs:6773`), chat `running` (`:3340`);
- `run_step`: `pending` (`:7917`, `:8875`), chat `running` (`:3355`);
- items are created `open`.
Test fixtures do insert such rows. `fixtures.rs` holds four parked/failed/blocked rows, and
`PgStore::load_demo` (`pg/demo.rs:47`) inserts them into every `demo_db`/template database.
Postgres would therefore hold events from the moment the demo loads, while `MemStore::from_demo`
holds none (it never calls `write`). That breaks D13 ("no demo-data events") and every conformance
case that counts events (`paging`, `prune`).
*Delta:*
- One trigger per table on `run`, `run_step` and `item`, `AFTER UPDATE OF status … DEFERRABLE
  INITIALLY DEFERRED`, with `WHEN (OLD.status IS DISTINCT FROM NEW.status AND NEW.status IN (…))`
  listing only the statuses that can write:
  - `run_step`: `awaiting_approval`, `failed`;
  - `run`: `awaiting_approval`, `failed`, `running`, `done`;
  - `item`: `blocked`.
- `step_permission` gets the only insert trigger, `AFTER INSERT … WHEN (NEW.status = 'pending')`,
  and no update trigger: no permission update writes an event.
- Effect: cancels, supersedes and `settle_permissions` queue no deferred call at all. The cost note
  in the Risks row then holds.
- MemStore (F5) applies the same rule: a row that appeared during the write is an event source only
  for `permissions`.

**F3 — Replace the `xmin` duplicate guard with a txid column, a unique index and `ON CONFLICT`
(T2; D2's wording).** *Plan:* `NOT EXISTS (… WHERE e.xmin = pg_current_xact_id()::xid)`, and "a
function never raises".
*Code/probe:*
- `xmin` is the inserting **sub**transaction's xid. The obvious way to make a function "never
  raise" is a plpgsql `BEGIN … EXCEPTION WHEN OTHERS` block, and that opens a subtransaction for
  every call. The guard then never matches.
- Probe: park → unpark → park in one transaction. With the `xmin` guard inside an `EXCEPTION`
  block it wrote **2** rows. Without the block it wrote 1. With `txid xid8 DEFAULT
  pg_current_xact_id()`, a unique index and `ON CONFLICT DO NOTHING` inside the same block, it
  wrote 1. `pg_current_xact_id()` returns the top-level xid even inside a subtransaction.
- The store opens no savepoints (`git grep -i savepoint crates/htui-store/src` finds none), so
  only the function body could cause this. A subtransaction per call also pushes a transaction
  that touches more than 64 rows into subxid overflow.
- Nothing in the trigger can raise on valid data: `started_by` and `created_by` are `NOT NULL` FKs,
  and every `project_id` is an FK.
*Delta (migration):*
- Add `txid xid8 NOT NULL DEFAULT pg_current_xact_id()`.
- Add `subject UUID NOT NULL`: a copy, **not** an FK. It is the step for `gate`/`step_failed`, the
  run for the five run kinds, the item for `unblock`, and the permission for `permission`.
- Add `CREATE UNIQUE INDEX uq_activity_event_once ON activity_event (txid, kind, subject)`.
- Each function `INSERT … ON CONFLICT DO NOTHING`. Keep "write only when `NEW.status` = current
  status" (D2).
- **No `EXCEPTION` block.** A raise is a bug, and the conformance cases and the full store suite
  (F12) are what catch it.
- `subject` is never set to `NULL`, so failure rows that keep their key after `ON DELETE SET NULL`
  cannot collide. Keying the index on the nullable FK columns under `NULLS NOT DISTINCT` would
  collide.
- No wraparound question: `xid8` is 64-bit and no `::xid` cast remains.
- D4's pinned query uses `DISTINCT ON (subject) … ORDER BY subject, id DESC` for "newest per
  subject" in place of a per-kind `CASE`.
- `ActivityEvent` (T1) does not expose `txid` or `subject`. MemStore needs neither (F5).
*T2 test:* add one to `activity_pg.rs`: park → unpark → park inside one explicit transaction writes
exactly one `gate`.

**F4 — Index the FK columns that `ON DELETE SET NULL` searches (T2).** *Plan:* indexes on
`(user_id,id)`, `at`, needs-you, `item_id` and `run_id`. *Code:* `delete_project` cascades over
every `run_step` and `step_permission` of the project (`:11734-11788`). Each deleted row makes the
FK action run `UPDATE activity_event SET step_id = NULL WHERE step_id = $1`, and without an index
that is a sequential scan of up to 100 000 rows per user, once per step.
*Delta:* add `idx_activity_event_step ON activity_event (step_id) WHERE step_id IS NOT NULL` and
`idx_activity_event_permission … (permission_id) WHERE permission_id IS NOT NULL`. `project_id`
and `workspace_id` fire once per delete and need no index.

**F5 — MemStore hook: status snapshot only; deletion rule in `remove_project` (T1).** *Plan:* the
`write` hook snapshots runs, steps, items and permissions, diffs them, and also applies D7 when
items and runs disappear.
*Code:*
- 140 `self.write(` sites, every one writing through `&mut State` fields (`state.runs.get_mut`, …).
  There is no setter where a version counter could hook, so counters would mean 140 edits and a
  site missed later.
- Rows are only ever removed in `State::remove_project` (`mem.rs:6013-6127`, reached by
  `delete_project` and `delete_workspace`), which already mirrors every `ON DELETE` with its
  `gone` reach.
- The demo holds dozens of rows, and no test builds more than a few hundred.
*Decision:* keep the full snapshot, made cheap:
- `struct StatusSnapshot { runs: HashMap<RunId, RunStatus>, steps: HashMap<StepId, StepStatus>,
  items: HashMap<ItemId, Status>, permissions: HashSet<PermissionId> }`. All `Copy` values, built
  with `with_capacity`.
- `MemStore::write` becomes `let now = self.now();` (MemClock, no lock) **before** taking the
  guard, then `let before = guard.status_snapshot(); let r = f(&mut guard);
  guard.record_activity(&before, now); r`.
- `record_activity` walks the *after* maps. A changed status passes F2's status filter, then D3.
  A new permission id with `pending` writes `permission`. A removed id writes nothing.
- It is a `State` method, so it reaches no `self.read` and no `this_user()` (deadlock, plan
  claim 48).
- Rejected: version counters (140 sites); reusing a scratch snapshot held in `State`. Do that only
  if T1's before/after timing of `cargo test -p htui-core --lib` shows more than 5 %, and report
  the number.
*Delta:*
- D7's MemStore half moves into `remove_project`, beside its sibling cascades. It drops the
  non-failure events whose `item`/`run` is in `gone.items`/`gone.runs`. On the survivors it sets
  `item`/`run`/`step`/`permission` to `None` when the referenced row is gone, and `project` to
  `None` for the project.
- `delete_workspace` sets `workspace` to `None` on **every** surviving event of that workspace,
  whatever its kind (`ON DELETE SET NULL` covers all rows).
- Extend `delete_project_leaves_no_row_in_any_map` (`mem.rs:14651`): every activity row that
  survives is a failure with every reference `None`.
- Accepted divergence, documented in `record_activity`'s doc comment: Postgres writes an event for
  a row that changes B→C→B inside one transaction, and the diff sees B→B. No store method does that
  in one transaction today. If a conformance case finds one, flag it rather than paper over it.

**F6 — No `Backend::prune_retention`; T3 sweeps through `backend.writer()` (T2, T3).** *Plan:*
`Backend::prune_retention(older_than)` prunes `worker_log` and `activity_event`. *Code:*
`PER_BOX_CAP` lives in `htui/src/worker_log/ship.rs:27`, which htui-store cannot import. There is
no `Backend` prune today: only `Writer` forwards `prune_worker_logs` (`worker.rs:558`). The Settings
› Boxes path already uses `backend.writer()` (`backend.rs:161`; Offline → `None`).
*Delta:*
- T2 drops `Backend::prune_retention` and **keeps** `impl ActivityStore for Writer`, its only
  consumer being T3.
- T3 serves `SweepRetention` as follows: `let Some(writer) = backend.writer() else {
  RetentionSwept(0) }`. Then `cut = now − retention_days(&backend.app_settings().await?)`. Then
  `writer.prune_worker_logs(cut, PER_BOX_CAP) + writer.prune_activity(cut, PER_USER_CAP)`.
- Store-worker test: add an Offline case that answers `RetentionSwept(0)`.

**F7 — The shared constants go in the foundation (T1, T2, T3, T4).** *Plan:* `PER_USER_CAP` sits
"beside the trait", which is T1's `store/activity.rs`, but T2 owns it there. T3 has
`ACTIVITY_LIMIT = 500` and T4 has `CAPACITY = 500` as two separate literals.
*Delta:* T1 declares `pub const PER_USER_CAP: u32 = 100_000;` in `store/activity.rs` and
`pub const ACTIVITY_PAGE: u32 = 500;` in `model/activity.rs`, beside `PINNED_LIMIT`. T3's
`ACTIVITY_LIMIT` and T4's `CAPACITY` are defined from `ACTIVITY_PAGE` (`as usize` for T4). With
that, no lane needs a type or constant from another lane:
- T4 uses `ActivityPage`, `ActivityEvent`, `WorkspaceSummary`, `RevealTarget` (existing), `Scope`
  and its own `Act`s, and no `StoreReply`;
- T3 uses T2's `Backend::activity` and `Writer` impl (same lane) plus T1's types.

**F8 — Key routing: a guarded shell step that consumes the key (T5; D10).** *Code:*
`dispatch_key` (`state.rs:1410-1502`) runs in this order:
1. the editor;
2. `translate` through `active_stack()`;
3. the overlay block;
4. `tab.on_key`;
5. `apply_keys(tab stack)`.

It fits a step between 3 and 4. One case must keep the tab first: the "T7 repair" in
`paint_panes` (`:798-806`) draws an open form (a **modal** tab stack) over a zoomed activity pane,
so the focus can be on Activity while a form holds the keys.
*Delta:*
- Add `fn activity_has_keys(&self) -> bool`. It is true when there is no overlay, the active tab's
  `focus_of(tab) == PaneId::Activity`, the tab's layout shows the pane, and **not**
  `tab.key_stack().and_then(|s| s.global()).is_some_and(|(_, l)| l.is_modal())`.
- `active_stack()` returns `ACTIVITY_BROWSE` under that predicate, so `translate`, the `?` box,
  the shell chips and `keys_line` follow without further edits.
- The shell step runs `for act in self.keys.actions(ACTIVITY_BROWSE, chord)`:
  - the panel's acts (`ActivityOpen`, `ActivityFilter`, `ListDown`/`ListUp`/`ListTop`/`ListBottom`)
    go to `self.activity.on_act(act, &self.scope)` and then the D9 jump;
  - otherwise `self.apply_keys(ACTIVITY_BROWSE, chord)`.
  It then **returns either way**: the Backlog never sees a key while the activity pane holds them.
  Today it does, and a `j` moves the hidden list.
- The pane's bottom-border hint (`:820-823`) uses the same predicate.
- Every key stays a `keys.toml` row: the step resolves only through `Keys::actions` and has no
  literal chord.
- T4: add `Act::ListBottom` to the `List` layer of `ACTIVITY_BROWSE`. Logs leaves it out only
  because of `LogsBottom`, and no layer of this stack binds `G`.
- T5 test: with a Backlog form open and Activity focused, a typed key reaches the form.

**F9 — Replies in `on_app_reply`, not `observe_reply` (T5).** *Code:*
- `observe_reply` (`update.rs:500`) is "the read-only look every reply gets, top bar only". It runs
  **before** the freshness gate and takes `&StoreReply`.
- `Activity` is only ever sent as `Origin::App`, and `Origin::App` is never `forget`-ed (only
  overlays are, `update.rs:283`), so every reply passes the gate.
- `Failed` is handled below the gate, through `absorbed` (`:485-497`, a predicate).
*Delta:*
- `on_app_reply` (`:621`) handles `StoreReply::Activity { page, workspaces }` → `activity.merge`.
  It also clears in-flight on `Failed { request: "activity" }` with `activity.failed()`.
- `absorbed`'s `Origin::App` arm gains the names `"activity"` and `"sweep_retention"` and stays
  free of side effects.
- `observe_reply` is unchanged.

**F10 — Sweep schedule by ticks, not by `StoreState` (T5).** *Plan:* "after the first online
`StoreState`, then hourly". *Code:* the App gets a label, not a backend kind, and with F6 an
Offline sweep answers `RetentionSwept(0)` for free.
*Delta:*
- `on_tick` sends `SweepRetention` on the first refresh tick and every `SWEEP_TICKS = 3600 * 4`
  ticks after it (`event_loop.rs:23`, 250 ms). The count goes in `App::sweep_due: u64`.
- A reconnect is caught by the next hourly sweep.
- Test: "sent at the first refresh, not again before `SWEEP_TICKS`".

**F11 — Store request and reply shape: no change beyond F6 and F7 (T3).**
- Checked `StoreRequest` (`store_worker.rs:157`), `name` (`:1384-1431`), `StoreReply` (`:1569`,
  `Debug, Clone`) and `serve` (`:2498`, `:2660`).
- No match on either enum outside `store_worker.rs` is exhaustive. `WorkerLogs` appears elsewhere
  only in the Logs tab's own arms, so adding variants breaks no other lane.
- `ActivityPage` must derive `Debug, Clone`, which the plan already does. `WorkspaceSummary` is
  `Clone`.
- `name_arms_are_stable` (`:6447`) gains the two names.
- Offline: `Backend::activity` gives an empty page, `backend.workspaces()` reads the mirror, and
  `SweepRetention` gives 0 (F6).
- Memory serves through MemStore.

**F12 — Triggers fire under existing raw-SQL tests; validate the whole store suite (T2).**
*Delta:*
- T2 Validate adds
  `cargo nextest run -p htui-store --all-features -E 'binary(it)'` against the test database, not
  only `activity_pg::`/`pg_conformance::`. Existing tests that `UPDATE run_step SET status = …`
  now write events, and a failure there shows a trigger bug.
- Re-run `every_cascade_table_loses_exactly_what_the_report_names`. Its hand list
  (`pg_criteria.rs:3345`) stays unchanged (plan D7 holds).
- T5: call `MemStore::push_activity` from `tests/it/activity.rs`, never from `src/testkit.rs`.
  `testkit` does not turn on `htui-core/test-support` (`htui/Cargo.toml:26`), and only the
  dev-deps (`:103`) do.

**F13 — The setting and the shipper read one key (T7).**
*Delta:* T7 adds `log_retention_days_is_the_shippers_key` to `htui/src/prompt_settings.rs`'s tests.
It asserts that `SettingKey::LogRetentionDays`'s name equals `worker_log::ship::RETENTION_KEY`,
that its spec's `min..=max` is `1..=365`, and that its default equals `DEFAULT_RETENTION_DAYS`.
It reads `ship.rs` (T3's file) without changing it, so the file sets stay disjoint.

## Checked, no change

- **D1** trait shape (`impl Future … + Send`) matches `WorkerLogStore`.
- **D4** derived pins: no change, apart from F3's `DISTINCT ON (subject)`.
- **D5** attribution.
- **D9**: the order of `SetScope` then reveal.
- **D11/T6**.
- **D12**.
- `connect.rs`/`migrations.rs` pin list (plan claim 40).
- `MIRRORED_TABLES` unchanged.
- `str_enum!` precedent.
- T7 `SettingSpec` bounds.
- Lane graph: T1 → {A: T2→T3, B: T4, C: T7} → T5 → T6. After F1 (`views.rs` tests are T1's) and F7
  (constants in T1), the file sets are still disjoint: T1 no longer touches
  `htui-worker/tests/it/main.rs`.

## Build sequence (unchanged order; deltas in brackets)

1. **T1**: model types plus `PINNED_LIMIT`/`ACTIVITY_PAGE` [F7], `park_kind`, `ActivityStore`
   plus `PER_USER_CAP` [F7], MemStore snapshot hook [F5] with D7 in `remove_project` and
   `delete_workspace` [F5], `ACTIVITY_CASES` with the cross-store park shapes [F1], the `views.rs`
   parity test [F1].
2. In parallel:
   - **T2**: migration with update-only triggers plus the permission insert trigger [F2], the
     `txid`/`subject` unique guard and no `EXCEPTION` [F3], the step/permission indexes [F4];
     `Backend::activity`, the `Writer` impl, no `prune_retention` [F6]; full store suite [F12].
     Then **T3**: `SweepRetention` through `writer()` [F6].
   - **T4**: the panel; stack with `ListBottom` [F8].
   - **T7**: plus the key-parity test [F13].
3. **T5**: `activity_has_keys` routing [F8], `on_app_reply` [F9], tick sweep [F10].
4. **T6**.
