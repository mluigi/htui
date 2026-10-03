# Blueprint: MOD-37 milestone 5 - ACP resume (R-48, ANA-27 §5.1 T5)

**Contract**: `.claude/plans/mod-37-acp-resume.plan.md` (confirmed 2026-10-03; the Maintainer decisions
and the Verified claims table are settled and not re-opened here). Scope unchanged. Where the code
disagrees with the plan, the blueprint says so under **Plan amendments** at the end, each with evidence.
**Line numbers** are from `1b31282e`. Every symbol given a change below was read in full.
**Order**: T1 (main tree) → T2 (main tree) ∥ T3 (worktree, named branch cut from T1's head, merged
`--no-ff`) → T4 → T5 → T6 (docs). Implementers commit per step, each commit green unless marked red.

## Design decisions
- **D1 `opening` lives on `RunStepSummary` only.** It is not added to `RunStep`, so no engine `RunStep`
  literal moves. `MemStore` keeps a side map, `State.openings: HashMap<StepId, StepOpening>`, beside
  `steps`, as `lease_owners` sits beside `runs` (`mem.rs:238-241`).
- **D2 `record_opening` is a `WriteStore` op, not a `WorkerStore` one.** The only writer is the TUI's
  agent worker, through the bind's `htui_store::Writer` (`agent_worker.rs` `ChatArgs.writer`). The
  `WorkerStore` family (`htui-core/src/store/worker.rs`, `htui-store/src/worker.rs`) is untouched. It is
  unfenced, like `promote_step`: a promoted step's run is parked and holds no lease (plan D87).
- **D3 no `COMMENT ON COLUMN` in 0014.** `the_ana_column_comments_are_present_and_verbatim`
  (`tests/migrations.rs:547-640`) asserts exactly forty-four commented columns across the tables it
  names, and `run_step` is one of them (H-2). The migration header documents the column instead.
- **D4 ACP restore order** is the maintainer's: `session/resume` when advertised and allowed, else
  `session/load` with the replay drained, else refuse. The route is a pure function, so the ACP unit
  test can pin it without a wire.
- **D5 the worker builds one recorder before the first `start`.** The `resume_failed` row and the
  later `follow_up` must come from the same continuing recorder. A second recorder over the stale
  `tail` would reuse its `seq` and hit the `(run_step_id, seq)` primary key (H-6).
- **D6 the `resume_failed` row is `htui`'s.** It is recorded as `kind = other`, `role = htui` through a
  new `Recorder::record_notice` (T4, `htui-agent/src/record.rs`). `Recorder::record` would stamp it
  `agent` (`record.rs:1004-1015`), and the agent said nothing. Amendment A-3.
- **D7 shared strings.** T3 adds `htui_orch::promote::CONTEXT_NOT_CARRIED = "context not carried;
  handoff prompt only"`. T4 adds `htui_agent::event::RESUME_FAILED = "resume_failed"` beside
  `SESSION_STARTED` (`event.rs:163`). The worker, the transcript, the Chat tab and the Runs pane all
  read these two.

---

## T1 - `run_step.opening`, store to summary

**Files**: `crates/htui-store/migrations/0014_run_step_opening.sql` (new),
`crates/htui-store/cache_migrations/0005_run_step_opening.sql` (new); `htui-core`
`src/model/{run,mod}.rs`, `src/store/{traits,mem,conformance}.rs`, `tests/mem_store.rs`; `htui-store`
`src/pg/{write,read,rows}.rs`, `src/cache/{refresh,read}.rs`, `src/writer.rs`, `.sqlx/`,
`tests/{pg_conformance,cache,connect,migrations}.rs`; `htui-agent` `src/conformance.rs`,
`tests/recorder.rs`; `htui-orch/src/closeout.rs`; `htui/src/ui/tabs/backlog/detail/runs.rs`,
`runs/execution_graph.rs` (literals only). `store/mod.rs` and `htui-store/src/worker.rs` do **not**
change: there is no new store type, and D2 keeps the op off `WorkerStore`.

### SQL
`migrations/0014_run_step_opening.sql`:
```sql
-- 0014_run_step_opening.sql - MOD-37 milestone 5 (R-48, ANA-27 §5.1 T5).
-- Forward-only (R-STO-5).
--
-- run_step.opening records how a promoted step's chat opened: 'resumed' (the step's own agent
-- session, restored through session/resume, session/load or the CLI's --resume), 'handoff' (a fresh
-- session opened with the handoff prompt) or 'resume_failed' (a requested resume failed, and the chat
-- fell back to the handoff prompt in the same bind). NULL on every step never bound to a promoted chat;
-- no older row knows how its chat opened, so nothing is backfilled. A second promotion overwrites it.
-- The mirror gains the column in cache_migrations/0005_run_step_opening.sql, and schema_version becomes
-- 14, so each box rebuilds its mirror once on first start. A headless worker never migrates: migrate
-- from a TUI first.

ALTER TABLE run_step ADD COLUMN opening TEXT
  CHECK (opening IN ('resumed','handoff','resume_failed'));
```
`cache_migrations/0005_run_step_opening.sql`:
```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-37 milestone 5 (R-48): run_step.opening, the way a promoted step's chat opened
-- (migrations/0014_run_step_opening.sql). TEXT with no CHECK, as 0003 mirrors verify_outcome.
--
-- cache_meta.schema_version moves to 14 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN opening TEXT;
```
`CacheStore::open` (`cache/mod.rs:131-152`) runs `CACHE_MIGRATOR` on the existing file first, so 0005
applies to an old mirror and the version mismatch then rebuilds it. `build.rs` already reruns on a new
file in either directory.

### htui-core
**`model/run.rs`**, after `VerifyOutcome` (`:153-163`):
```rust
str_enum!(
    /// `run_step.opening` (MOD-37 M5, R-48): how a promoted step's chat opened. `NULL` on a step
    /// never bound to a promoted chat.
    StepOpening {
        /// The step's own agent session was restored.
        Resumed => "resumed",
        /// A fresh session opened with the handoff prompt.
        Handoff => "handoff",
        /// A requested resume failed; the chat opened with the handoff prompt instead.
        ResumeFailed => "resume_failed",
    }
);
```
`RunStepSummary` (`:738-786`): append after `gate_note`:
```rust
    /// `run_step.opening` (MOD-37 M5): how the step's promoted chat last opened; `None` when no
    /// chat was ever bound to it.
    pub opening: Option<StepOpening>,
```
**`model/mod.rs`**: `StepOpening` joins `pub use run::{…}` (`:161-167`).
`run_enums_match_check_lists` (`:280-311`) gains
`check_enum(StepOpening::ALL, &["resumed", "handoff", "resume_failed"]);`.

**`store/traits.rs`**, `WriteStore`, right after `promote_step` (`:1484`):
```rust
    /// MOD-37 M5 (R-48): `run_step.opening = opening`, replacing any earlier value: how the step's
    /// promoted chat opened. Unfenced, as [`promote_step`](Self::promote_step) is, because a promoted
    /// step's run is parked and holds no lease. `updated_at` moves (the store's), so the mirror
    /// picks the column up.
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run_step" }`.
    async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()>;
```
Import `StepOpening` in the trait's model `use`.

**`store/mem.rs`**:
- `State` (`:138`), after `lease_owners` (`:241`):
  `/// `run_step.opening` (MOD-37 M5), beside `steps` as `lease_owners` sits beside `runs`: the column is a`
  `/// [`RunStepSummary`] field and not a [`RunStep`] one.` `openings: HashMap<StepId, StepOpening>,`.
  `State` derives `Default`. Add `openings: HashMap::new(),` to the `from_demo` literal (`:303-350`).
- `State::record_opening`, beside `State::promote_step` (`:5196`):
  ```rust
  /// MOD-37 M5: the step's opening, replacing any earlier one; the step's `updated_at` moves.
  fn record_opening(&mut self, step: StepId, opening: StepOpening, now: DateTime<Utc>) -> Result<()> {
      let row = self.steps.get_mut(&step).ok_or_else(|| StoreError::NotFound {
          entity: "run_step",
          id: step.to_string(),
      })?;
      row.updated_at = now;
      self.openings.insert(step, opening);
      Ok(())
  }
  ```
- `WriteStore for MemStore`, beside `promote_step` (`:7145`):
  `async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> { let now = self.now(); self.write(|state| state.record_opening(step, opening, now)) }`.
- `State::run_steps` (`:1287`): `opening: self.openings.get(&step.id).copied(),` after `gate_note`.
- `State::delete_project` (`:4117`): `self.openings.retain(|id, _| !gone.steps.contains(id));` beside
  `self.steps.retain`. That is the only site that drops steps.

**`store/conformance.rs`**: `"record_opening_lands_on_the_run_summary_and_bumps_updated_at"` is appended
to `CASES` (after `"hand_written_rows_round_trip"`, `:187`) with its `run_case` arm. Counts: `CASES`
135 → **136** in `tests/mem_store.rs` (message gains "and MOD-37 milestone 5's one for
`run_step.opening` (R-48)"), and `EXPECTED_CASES` 135 → 136 in `htui-store/tests/pg_conformance.rs:28`
(its doc and its message too).

### htui-store
**`pg/write.rs`**, beside `promote_step` (`:5353`):
```rust
    /// MOD-37 M5: one `UPDATE`; `trg_run_step_updated_at` moves `updated_at`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `{ entity: "run_step" }` when no row has that id.
    async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> {
        let written = sqlx::query!(
            "UPDATE run_step SET opening = $2 WHERE id = $1",
            step.as_uuid(),
            opening.as_str(),
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?
        .rows_affected();
        if written == 1 {
            Ok(())
        } else {
            Err(StoreError::NotFound { entity: "run_step", id: step.to_string() })
        }
    }
```
**`pg/read.rs`** `PgStore::runs` step statement (`:397-403`): a comma after `s.gate_note`, then
```
                   -- Appended, positional: `run_step.opening` (MOD-37 M5).
                   s.opening AS "opening: htui_core::model::StepOpening"
```
**`pg/rows.rs`** `StepRow` (`:94-146`): append last
`/// `run_step.opening` (MOD-37 M5), appended last (positional).` `pub(crate) opening: Option<StepOpening>,`.
`into_summary` (`:150`) gains `opening: self.opening,`. `StepRow`'s only reader is `PgStore::runs`.

**`cache/refresh.rs`**: in `RUN_STEP_COLUMNS` (`:1179-1203`), `"opening"` goes between `"promoted_at"`
and `"updated_at"`. In `refresh_run_step` (`:1205-1264`), the `query!` text becomes
`… verify_outcome, verify_exit_code, promoted_at, opening, updated_at …`, and
`.bind(row.opening.as_deref())` goes after the `promoted_at` bind. Column order and bind order must
agree, because `upsert_sql` (`:380`) is positional.
**`cache/read.rs`** `CacheStore::runs` (`:519-647`): the select ends `a.name AS agent_name, s.gate_note, s.opening`,
and the summary gains `opening: get::<Option<StepOpening>>(row, "opening")?,` (as `gate_outcome` is
read). Import `StepOpening`. `CacheStore::run_steps` (`:909`) answers `RunStep` and is untouched.

**`writer.rs`**, beside `promote_step` (`:1085`):
`async fn record_opening(&self, step: StepId, opening: StepOpening) -> Result<()> { match self { Self::Memory(store) => store.record_opening(step, opening).await, Self::Online(pg) => pg.record_opening(step, opening).await } }`.

### The other `WriteStore` implementors and literals
- `UsageSpy` (`htui-agent/src/conformance.rs`, near `promote_step` `:1221`) and `SpyStore`
  (`htui-agent/tests/recorder.rs`, near `:947`): `async fn record_opening(&self, step: StepId, opening: StepOpening) -> StoreResult<()> { self.inner.record_opening(step, opening).await }`,
  with `StepOpening` imported from `htui_core::model`. Gortex finds no other implementor besides
  `MemStore`, `Writer` and `PgStore`.
- `RunStepSummary` literals gain `opening: None,`: `htui-orch/src/closeout.rs:272`,
  `htui/src/ui/tabs/backlog/detail/runs.rs:2365`, `runs/execution_graph.rs:693`. Every other literal
  in those files is `..base`/`..step` (text search, 30 hits).

### `.sqlx` (the expected diff, exactly)
1. The `PgStore::runs` step `SELECT` changes hash (rename, one column added: `ordinal 21 opening`,
   nullable `true`).
2. The `refresh_run_step` `SELECT` changes hash (rename, one column added).
3. One file added: `UPDATE run_step SET opening = $2 WHERE id = $1`.

Any other `.sqlx` change is a leak: stop. Prepare against a migrated scratch DB, not the test DSN
(`docs/hr-sandbox.md:196-210`):
```
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations          # 14 applied
cargo sqlx prepare -- --all-targets --all-features
cargo sqlx prepare --check -- --all-features         # "potentially unused queries" is expected
```

### T1 red tests (write first)
| # | Name / file | Asserts | Red on today's code |
|---|---|---|---|
| 1 | conformance `record_opening_lands_on_the_run_summary_and_bumps_updated_at` (both stores) | A fresh `create_run(new_run(PROJECT_HTUI, HTUI_ANA_2, vec![]))` plus `create_step(new_run_step(run, 0, 1, 0))`. The summary (`runs(HTUI_ANA_2)` → run → step) has `opening == None`. Then `before = step_row(..).updated_at`, and `record_opening(step, ResumeFailed)` is `Ok`. Now the summary says `Some(ResumeFailed)` and `step_row(..).updated_at > before`. A second `record_opening(step, Handoff)` replaces it with `Some(Handoff)`. A sibling step created at position 1 still reads `None`. `record_opening(StepId::new(), Resumed)` is `NotFound { entity: "run_step", .. }` | Compile red first. Then, per `2ece53cc`, land `todo!()` bodies so the case panics on both stores, with the DB reached (`HTUI_TEST_DATABASE_URL`) |
| 2 | `htui-store/tests/cache.rs` `the_0005_opening_reaches_the_mirror` | The shape of `the_0003_columns_reach_the_mirror` (`:642`): `db.store.record_opening(ids::STEP_IMPL, StepOpening::ResumeFailed)`, refresh, then `CacheStore::runs(<STEP_IMPL's item>)` finds the step with `opening == Some(ResumeFailed)` | The mirror has no column, so the refresh/read fails or reads `None` |
| 3 | `tests/migrations.rs` `the_opening_column_admits_three_values_and_null` (`demo_db`) | `UPDATE run_step SET opening = $2 WHERE id = $1` with each of the three texts on `ids::STEP_IMPL` succeeds. `'bogus'` fails with SQLSTATE `23514`. `NULL` succeeds | No column |
| 4 | migration pins 13 → 14 | `migrations.rs:96-103` (`vec![1..=14]`, message names "MOD-37 milestone 5's 0014_run_step_opening.sql"), `:1017-1019`, `:1117`, `:1122`, `:1141`, `:1291` (the count, "fourteen"); `connect.rs:139-159`, `:243-245`. Exactly `7c7e78c2`'s sites | They fail once 0014 exists. Move them in the same commit as the migration |
| 5 | `model/mod.rs` `run_enums_match_check_lists` | The `check_enum` line above | Compile red |

**Gate T1**
```
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-store --all-features -- --test-threads=1      # check the DB was reached: pg cases do not skip
cargo test -p htui-agent --all-features --test recorder && cargo test -p htui-agent --all-features --lib conformance
cargo test -p htui-orch --all-features --lib closeout
cargo test -p htui --all-features --lib runs -- --test-threads=1
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-features)
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
**Commits**: (1) `test(mod-37): run_step.opening conformance, mirror and migration cases (red)`, with the
stub bodies; (2) `feat(mod-37): run_step.opening across both stores and the mirror (R-48)`. Merge them
if a red commit cannot compile. T2 and T3 start from (2).

---

## T2 - the ACP driver restores a session (main tree, parallel with T3)

**Files**: `crates/htui-agent/src/acp/mod.rs`, `crates/htui-agent/src/registry.rs`,
`crates/htui-agent/tests/acp_driver.rs`. `tests/launch.rs` does not move (`:333-334` pins the settings'
defaults only), and neither does `tests/driver_contract.rs` (`:380-394` pins `DriverCaps::default()`).

### registry.rs - `caps_from` (`:157-172`)
`resume: settings.acp.session.resume || settings.acp.session.load,`, with the comment
`// MOD-37 M5 (R-48): either restore route resumes; session_main picks `session/resume` first.`

### acp/mod.rs - imports
Add `LoadSessionRequest, ResumeSessionRequest, AgentCapabilities` to the `schema::v1` import (`:32`) and
`RestoredSession` to the `agent_client_protocol` import (`:37`, if it is not reachable through
`agent_client_protocol::RestoredSession`). Add `crate::launch::SessionSettings`. No Cargo change: the
drain polls with `std::task::Waker::noop()` (stable since 1.85; the toolchain is 1.98.1), so `futures`
stays out of `htui-agent`'s dependencies.

### acp/mod.rs - new private items (after `handshake_error`, `:1701-1714`)
```rust
/// MOD-37 M5 (R-48): which request restores `spec.resume`: `session/resume` when the agent
/// advertises `sessionCapabilities.resume` and `settings.acp.session.resume` allows it, else
/// `session/load` on `loadSession` and `settings.acp.session.load`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restore {
    Resume,
    Load,
}

/// [`Restore`]'s decision, or why neither request may be sent, naming each missing capability or
/// setting.
fn restore_route(caps: &AgentCapabilities, settings: &SessionSettings) -> Result<Restore, String> {
    let resume = match (caps.session_capabilities.resume.is_some(), settings.resume) {
        (true, true) => return Ok(Restore::Resume),
        (false, _) => "`session/resume` is not advertised (`sessionCapabilities.resume`)",
        (true, false) => "`session/resume` is off (`settings.acp.session.resume`)",
    };
    let load = match (caps.load_session, settings.load) {
        (true, true) => return Ok(Restore::Load),
        (false, _) => "`session/load` is not advertised (`loadSession`)",
        (true, false) => "`session/load` is off (`settings.acp.session.load`)",
    };
    Err(format!("{resume}; {load}"))
}

/// Restores `previous` by `route` (MOD-37 M5). The step is recorded first, as `session/new`'s
/// is, so a refusal answered from the connection's error (`answer_from_connection`) names the
/// request that was actually sent. A loaded session's replay is discarded ([`drain_replay`]).
async fn restore_session(
    cx: &ConnectionTo<Agent>,
    spec: &SessionSpec,
    previous: &AgentSessionRef,
    route: Restore,
    ready: &Mutex<ReadyCell>,
    child: &Mutex<ChildGuard>,
) -> Result<agent_client_protocol::ActiveSession<'static, Agent>> {
    match route {
        Restore::Resume => {
            at_step(ready, "session/resume");
            let request = ResumeSessionRequest::new(previous.as_str().to_owned(), spec.cwd.clone())
                .additional_directories(spec.extra_dirs.clone());
            cx.resume_session_from(request)
                .block_task()
                .start_session()
                .await
                .map(RestoredSession::into_session)
                .map_err(|err| handshake_error("session/resume", &err, child))
        }
        Restore::Load => {
            at_step(ready, "session/load");
            let request = LoadSessionRequest::new(previous.as_str().to_owned(), spec.cwd.clone())
                .additional_directories(spec.extra_dirs.clone());
            let mut session = cx
                .load_session_from(request)
                .block_task()
                .start_session()
                .await
                .map(RestoredSession::into_session)
                .map_err(|err| handshake_error("session/load", &err, child))?;
            let discarded = drain_replay(&mut session);
            tracing::debug!(discarded, "session/load: the history replay was discarded");
            Ok(session)
        }
    }
}

/// Discards every update already queued on `session`, without waiting: the history a
/// `session/load` replays ahead of its response (SDK `concepts/sessions.rs:86-88`; the handler
/// queues at `session.rs:1223` before the ordered response is dispatched, `jsonrpc.rs:6011`).
/// `read_update` is a `futures` mpsc `next()` (`session.rs:1054`), which is cancel-safe, so one
/// poll with a no-op waker is `Ready` while something is queued and `Pending` once it is empty.
/// A closed channel ends the drain too; the turn loop reports it.
fn drain_replay(session: &mut agent_client_protocol::ActiveSession<'static, Agent>) -> usize {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut discarded = 0;
    loop {
        let next = std::pin::pin!(session.read_update());
        match next.poll(&mut cx) {
            std::task::Poll::Ready(Ok(_)) => discarded += 1,
            std::task::Poll::Ready(Err(_)) | std::task::Poll::Pending => return discarded,
        }
    }
}
```
(`Result` here is the module's `crate::error::Result<_>` alias, as `open_session` uses it. The SDK's
`SessionId: From<String>` is what the compile probe used. If the alias differs, spell
`Result<_, DriverError>`.)

### acp/mod.rs - `session_main` step 2 (`:1101-1124`), the branch
The `let mut session = match cx.build_session_from(..)…` block becomes:
```rust
    // 2. The session: `session/new`, or the step's own restored (MOD-37 M5, R-48): resume, else
    //    load (its replay discarded), else refuse. ANA-4 risk 11: `block_task` only here.
    let mut session = match spec.resume.as_ref() {
        None => {
            at_step(ready, "session/new");
            /* today's NewSessionRequest block, unchanged, including its error arm */
        }
        Some(previous) => {
            let restored = match restore_route(&init.agent_capabilities, &options.settings.acp.session) {
                Ok(route) => restore_session(&cx, &spec, previous, route, ready, child).await,
                Err(why) => Err(DriverError::Transport(format!(
                    "cannot resume session `{}`: {why}",
                    previous.as_str()
                ))),
            };
            match restored {
                Ok(session) => session,
                Err(err) => {
                    answer(ready, Err(err));
                    kill(child).await;
                    return;
                }
            }
        }
    };
```
Everything after it is unchanged. `session_id`/`session_ref` read the restored session's id, which is
the requested one, so step 3's banner carries `previous`. A re-promoted step's `promote::banner` then
finds the same id. `model_values` and `model_option` work on a restored session: `restored_session`
passes the response's config options into the `ActiveSession` (SDK `session.rs:321-340`). The drain runs
before the banner and before the first `send_prompt`, so nothing from the replay can reach `on_message`.
`at_step` names are exactly `"session/new"`, `"session/resume"`, `"session/load"`. The refusal sends
no request and needs no `at_step`. Its text is
``cannot resume session `<id>`: `session/resume` is not advertised (`sessionCapabilities.resume`); `session/load` is not advertised (`loadSession`)``
(each half switches to its `is off (`settings.acp.session.*`)` form when the setting is the cause).
A refused request reads `session/resume failed: <agent's message>[\n<stderr tail>]` or
`session/load failed: …`, which is `handshake_error`'s D61 shape whichever arm composes it. Every exit
kills and reaps the child as the `session/new` arm does.

The doc on `session_main`'s step 2 and the module doc lines that say the driver ignores
`SessionSpec.resume` are rewritten to the above.

### T2 red tests (`tests/acp_driver.rs`, `#[cfg(unix)]`)
One configurable raw JSON-RPC fake in `refuse_session_new`'s style (`:256-293`):
```rust
/// MOD-37 M5: an agent that advertises `caps` in its `initialize` answer, answers `session/new`
/// with [`FRESH_SESSION`], answers `session/resume` per `resume` and `session/load` per `load`
/// (writing `replay` as `session/update` notifications for the requested id first), and on
/// `session/prompt` sends one `agent_message_chunk` "fresh turn" and ends the turn `end_turn`.
/// Every request's method and params go to `seen`.
async fn restoring_agent(stream: DuplexStream, caps: Value, resume: Answer, load: Answer,
                         replay: Vec<Value>, seen: mpsc::UnboundedSender<(String, Value)>)
```
`Answer` is `Ok` (`result: {}`) or `Refuse` (`error: { code: -32000, message: VENDOR_REFUSAL }`). The
`initialize` result is `{ "protocolVersion": 1, "agentCapabilities": caps }`. The replay updates are
`{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old reply"}}` and
`{"sessionUpdate":"tool_call","toolCallId":"old-call","title":"read old","kind":"read","status":"completed"}`.
`spec_resuming(cwd, id)` is `spec(cwd)` with `resume: Some(AgentSessionRef::new(id))` and
`extra_dirs: vec![cwd.join("extra")]`.

| # | Name | Asserts | Red on today's code |
|---|---|---|---|
| a | `a_resume_sends_session_resume_and_never_session_new` | caps `{"sessionCapabilities":{"resume":{}},"loadSession":true}`. `open_session` is `Ok`. `seen` holds `session/resume` with `params.sessionId == "prior"`, `params.cwd == cwd` and `params.additionalDirectories == [cwd/extra]`, and holds neither `session/new` nor `session/load`. The first `next_event` is the banner with `body.session_id == "prior"`. The next events are the "fresh turn" chunk, then `Done(EndTurn)` | Today the driver sends `session/new`, so the banner says `FRESH_SESSION` and `seen` has `session/new` |
| b | `a_load_discards_the_replay_before_the_first_turn` | caps `{"loadSession":true}`, two replay updates. `seen` has `session/load` (`sessionId`, `cwd`, `additionalDirectories`) and no `session/new`. The banner carries `"prior"`. No `AssistantChunk` with "old reply" and no `ToolCall` "old-call" ever arrives. The first event after the banner is the "fresh turn" chunk, then `Done` | Today no `session/load` is sent, so the `seen` assertion fails |
| c1 | `a_resume_the_agent_cannot_restore_is_refused_and_reaped` | Over a `sleep 1000` child (the `a_refused_session_new_…` shape, `:419-480`), caps `{}`. The answer is `Err(Transport(m))` with `m` containing `cannot resume session \`prior\``, `sessionCapabilities.resume` and `loadSession`. `seen` has no `session/new`. `assert_reaped(pid)` | Today `session/new` succeeds, so `Ok` |
| c2 | `a_resume_the_settings_forbid_is_refused` | caps advertise both, but `options.settings.acp.session = SessionSettings { load: false, resume: false }`. `m` contains `settings.acp.session.resume` and `settings.acp.session.load`. No restore request and no `session/new` are sent | Today `Ok` |
| d | `a_refused_session_load_answers_with_the_agents_own_message` | caps `{"loadSession":true}`, `load: Refuse`, `sleep` child. `m` contains `session/load failed` and `VENDOR_REFUSAL`. `assert_reaped(pid)` | Today `Ok` (`session/new`) |
| e | `caps_for_an_acp_row_resumes_when_load_or_resume_is_on` | An ACP row (`caps_for` is already imported, `:68`) with `acp.session` `{resume:false, load:true}` gives `resume == true`. `{true,false}` gives `true`. `{false,false}` gives `false` | `{false,true}` gives `false` today |
| f | `src/acp/mod.rs` `#[cfg(test)] mod tests` `restore_route_prefers_resume_then_load` | Pure: both → `Resume`. Load only → `Load`. Resume advertised but off with load on → `Load`. Neither → `Err` naming both capability keys | Compile red (new fn) |

Notifications the fake writes carry no `id` and get no answer. The fake answers notifications from the
client (`session/cancel`) with nothing. `(b)` is the plan's drain pin. A late replay (after the
response) would show up here as an extra event (H-5).

**Gate T2**
```
cargo test -p htui-agent --all-features --test acp_driver
cargo test -p htui-agent --all-features
cargo clippy -p htui-agent --all-targets --all-features -- -D warnings
```
**Commits**: `test(mod-37): ACP restore cases (red)` (tests a-e compile on today's code; f lands with the
fix), then `feat(mod-37): the ACP driver resumes, loads or refuses spec.resume (R-48)`.

---

## T3 - the engine resumes ACP steps and always carries the handoff (worktree, parallel with T2)

**Files**: `crates/htui-orch/src/{promote,command,engine,conformance}.rs`; `crates/htui/src/agent_worker.rs`
(`:1034`, `:5140` only); `crates/htui/tests/chat.rs` and `crates/htui/tests/runs_pg.rs` (test rows
only, H-1). `htui-worker/src/runtime.rs:2386-2389` matches `{ .. }` and is unaffected.
**Worktree** (memory rules): `git branch hr/MOD-37-m5-t3 <T1 head>` first, then
`git worktree add ../htui-m5-t3 hr/MOD-37-m5-t3`. Edit with file tools on the worktree path, because
Gortex `edit` writes to the primary checkout. Expect about 10 GB of `target/`. Merge back with
`git merge --no-ff hr/MOD-37-m5-t3` after T2 has committed. Then run `git worktree remove ../htui-m5-t3`
before `git branch -d`.

### promote.rs
- New constant beside `RESUME_OPENING` (`:17`):
  ```rust
  /// MOD-37 M5 (ANA-27 T5): what a chat that opened with the handoff prompt did not carry. The Runs
  /// pane, the `resume_failed` row and the Chat tab all say it in these words.
  pub const CONTEXT_NOT_CARRIED: &str = "context not carried; handoff prompt only";
  ```
- `opening_kind` (`:57-72`) **drops its `transport` parameter**: `pub fn opening_kind(caps: DriverCaps, events: &[SessionEvent]) -> OpeningKind`,
  with the body `if !caps.resume { return OpeningKind::Handoff; } banner(events).map_or(OpeningKind::Handoff, OpeningKind::Resume)`.
  An unused named parameter would warn (amendment A-1). Its only production caller is `engine.rs:1372`.
  The `Transport` import goes if nothing else uses it. Doc: "Resume whenever the agent's caps say
  `resume` and [`banner`] finds the session's id: the CLI through `--resume`, ACP through
  `session/resume` or `session/load` (MOD-37 M5, R-48). The handoff text is built either way, as the
  fallback a failed resume opens with."
- The module doc (`:1-5`) and `OpeningKind::Resume`'s doc gain "either transport".
- **A-7 (maintainer, 2026-10-03):** `banner` (`:30-52`) takes the **latest** `session_started` row:
  `.max_by_key(|(seq, _)| *seq)` replaces `.min_by_key`. Its doc becomes "The step's latest `other`
  row whose update is `session_started` … (MOD-37 M5 A-7, amending D192: after a resume that failed
  and fell back, the latest session is the handoff's, which holds the chat)". The message in
  `a_resumable_cli_agent_with_a_banner_resumes` (`:266`) is reworded to "latest". New red test
  `the_latest_banner_wins`: two banners (`seq` 1 `sess_old`, `seq` 5 `sess_new`), stored out of
  order, give `Resume(sess_new)`. Today's code returns `sess_old`.
- Tests (`:262-322`): every `opening_kind(caps, Transport::Cli, events)` becomes `opening_kind(caps, events)`.
  `an_acp_agent_hands_off_even_when_its_caps_say_resume` (`:315`) becomes
  `an_acp_agent_with_resume_caps_and_a_banner_resumes`. It asserts
  `opening_kind(acp_caps(), &banner_events()) == OpeningKind::Resume(AgentSessionRef("sess_1".into()))`,
  where `acp_caps()` is `registry::caps_for`'s ACP profile spelled out (all of `permission_requests`,
  `edit_proposals`, `plans`, `thoughts`, `follow_up_in_session`, `resume`, `usage`, `usage_mid_turn`,
  `authenticate` true). A new `an_acp_agent_without_resume_caps_hands_off` (`acp_caps()` with
  `resume: false`) asserts `Handoff`.

### command.rs - `OpeningPath::Resume` (`:273-289`)
```rust
    /// The step's own agent session, resumed; `text` is `promote::RESUME_OPENING`, recorded as the
    /// chat's first `follow_up`. `handoff` and `digest` are the handoff opening the same promotion
    /// would have built: what the chat opens with instead when the resume fails (MOD-37 M5).
    Resume {
        /// The session id the step's `session_started` banner recorded.
        session_ref: AgentSessionRef,
        /// The first message the chat sends.
        text: String,
        /// The assembled, scrubbed handoff prompt, the fallback.
        handoff: String,
        /// Its digest.
        digest: String,
    },
```
And an accessor, so tests and the worker read the handoff from either variant:
```rust
impl OpeningPath {
    /// The handoff prompt and its digest, whichever opening this is (MOD-37 M5).
    #[must_use]
    pub fn handoff(&self) -> (&str, &str) {
        match self {
            Self::Resume { handoff, digest, .. } | Self::Handoff { text: handoff, digest } => (handoff, digest),
        }
    }
}
```

### engine.rs - `opening` (`:1337-1465`) and the extracted helper
`Engine::opening` keeps its signature. After `trees`, `repos` and `(cwd, extra_dirs)`:
```rust
        let caps = htui_agent::registry::caps_for(&agent);
        // MOD-37 M5: the handoff is built for both openings. A `Resume` carries it as the fallback
        // the worker opens with when the resume fails. Boxed: this future sits in every dispatch
        // future (`every_case_name_dispatches`, H-9).
        let (handoff, digest) = Box::pin(self.handoff_opening(
            run, snapshot, step, phase, item, &events, &trees, &repos,
        ))
        .await?;
        let path = match promote::opening_kind(caps, &events) {
            OpeningKind::Resume(session_ref) => OpeningPath::Resume {
                session_ref,
                text: promote::RESUME_OPENING.to_owned(),
                handoff,
                digest,
            },
            OpeningKind::Handoff => OpeningPath::Handoff { text: handoff, digest },
        };
```
The new private method holds today's `OpeningKind::Handoff` arm verbatim (`:1378-1453`): `phase_spec`
with its three refusal sentences, the `HANDOFF_TEMPLATE` lookup (`ResolveError::NoTemplate`), `roots`,
`step_commits`, the advisory diff, `failure_reason`, `handoff_spec` and `assemble`:
```rust
    /// §4.6(c)'s handoff text and digest for a promoted `step` (MOD-4 D193): what a `Handoff`
    /// opening sends, and what a `Resume` opening falls back to (MOD-37 M5). Nothing here writes.
    #[expect(
        clippy::too_many_arguments,
        reason = "the step's run, snapshot, row and phase, its item, and the three reads `opening` \
                  already made; re-reading them here would double the promotion's store round trips"
    )]
    async fn handoff_opening(
        &self,
        run: &Run,
        snapshot: &GraphSnapshot,
        step: &RunStep,
        phase: &SnapshotPhase,
        item: ItemId,
        events: &[SessionEvent],
        trees: &[RunStepTree],
        repos: &[Repo],
    ) -> Result<(String, String), EngineError>
```
It returns `Ok((assembled.text, assembled.digest))`. Add `SessionEvent` to the `htui_core::model` import
(`:33-40`). Rewrite `opening`'s doc (`:1326-1336`): "resumed on either transport when the agent's
caps say `resume` and the log has a banner (MOD-37 M5); the handoff is built in both cases".
**Behaviour change (plan T3 note)**: a `Resume` promotion now also needs the `handoff` template and a
buildable phase spec. When either is missing it fails on the existing "opening cannot be built" path,
with the step promoted and no chat.

### The existing pins that flip (H-1), and what each becomes
The orch fakes (`htui_agent::fake::FakeSession::open`, `fake.rs:231-250`) queue a `session_started`
banner on every session. Fixture `AGENT_CLAUDE` is the ACP seed with `acp.session {load: true,
resume: true}` (`seeds/agent_claude.json`). So after T3, every walked-then-promoted claude step resumes:
- `conformance.rs` `promote_keeps_the_step_and_writes_no_chat_run` (`:5500`): replace the `let OpeningPath::Handoff { text, digest } = … else { panic!(…) }`
  with a precondition, `assert_eq!(promote::banner(&orch.store().step_events(prd.id).await…), Some(AgentSessionRef::new(format!("fake-{}", prd.id))))`
  ("the fake walk recorded its banner"). Then
  `assert!(matches!(&opening.path, OpeningPath::Resume { session_ref, text, .. } if session_ref.as_str() == format!("fake-{}", prd.id) && text == promote::RESUME_OPENING))`
  and `let (text, digest) = opening.path.handoff(); assert!(!text.is_empty() && !digest.is_empty());`.
  The case now pins the resume.
- `conformance.rs` `promote_a_failed_step_of_a_parked_run` (`:5598`): `let (text, _) = opening.path.handoff();`.
  Its later assertions about the failure reason in the text stand, since the handoff is the same text.
- `engine.rs` `a_handoff_spec_carries_no_excerpts_and_runs_no_pass` (`:14652`): `let (text, _) = opening.path.handoff();`.
- `crates/htui/tests/chat.rs`: `graph_store` (`:1007-1018`) registers the scripted row through a new
  `fn handoff_only(mut row: Agent) -> Agent { row.settings["acp"] = json!({ "session": { "load": false, "resume": false } }); row }`.
  Both keys go false because T2 makes either one resume. With that, `promotion_opens_the_chat_on_the_same_step`
  (header `· handoff ·`, `:1131`), `the_handoff_opening_is_one_follow_up_row` (`:1380-1424`) and the
  `chat__chat_promoted.snap` snapshot keep the handoff path they name. Rewrite the doc at `:1130`
  ("an `acp` row always opens with the handoff prompt, R-48") to "this row's settings turn resume
  off, so it opens with the handoff prompt".
- `crates/htui/tests/runs_pg.rs` `seed` (`:231-262`): `settings: json!({ "acp": { "session": { "load": false, "resume": false } } })`,
  so the promotion case (`:700-845`, "the handoff opening") keeps the path it names.
- `agent_worker.rs:1034` becomes `OpeningPath::Resume { session_ref, text, .. } => (Some(session_ref), text),`
  (T4 replaces it). `:5140` gains `handoff: "the handoff".to_owned(), digest: "d".to_owned(),`.

### T3 red tests
| # | Name / file | Asserts | Red on today's code |
|---|---|---|---|
| 1 | `promote.rs` `an_acp_agent_with_resume_caps_and_a_banner_resumes` | Above | `Handoff` today (the CLI-only rule) |
| 2 | `promote.rs` `an_acp_agent_without_resume_caps_hands_off` | `Handoff` | Pin, green |
| 3 | `engine.rs` `a_resume_opening_carries_the_handoff_a_handoff_opening_would` (beside `:14610`) | The `a_handoff_spec_carries_no_excerpts_and_runs_no_pass` setup. `PromoteStep { chat_open: false }` gives `OpeningPath::Resume { handoff: h1, digest: d1, .. }`. Then `AGENT_CLAUDE`'s row is rewritten with `acp.session {load:false, resume:false}` through the store's agent writer, which the fake `GraphSource::agent` reads (`fake.rs:894-901`). `PromoteStep` again (a promoted `awaiting_approval` step under a parked run is promotable) gives `OpeningPath::Handoff { text: h2, digest: d2 }`, with `(h1, d1) == (h2, d2)` | Compile red. On today's code the first promotion is `Handoff` |
| 4 | conformance `promote_keeps_the_step_and_writes_no_chat_run` (rewritten) | Above | Its new `Resume` assertion fails today |

**Gate T3** (in the worktree)
```
cargo test -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/m5-t3.log; grep -c SIGABRT /tmp/m5-t3.log   # 0
cargo test -p htui --all-features --test chat -- --test-threads=1      # check the count is non-zero
cargo test -p htui --all-features --test runs_pg -- --test-threads=1   # DB reached
cargo test -p htui --all-features --lib agent_worker -- --test-threads=1
cargo clippy -p htui-orch -p htui --all-targets --all-features -- -D warnings
```
**Commits**: `test(mod-37): ACP steps with a banner resume; the handoff rides along (red)`, then
`feat(mod-37): the engine resumes either transport and carries the handoff fallback (R-48)`.
---

## T4 - the worker falls back on a failed resume and records the opening (after T1, T2, T3)

**Files**: `crates/htui/src/agent_worker.rs`; `crates/htui-agent/src/{record,event}.rs` and
`crates/htui-agent/tests/recorder.rs` (amendment A-3); `crates/htui/tests/chat.rs` (one new case).

### htui-agent
**`event.rs`**, beside `SESSION_STARTED` (`:163`):
```rust
/// MOD-37 M5: the update of the `other` row `htui` records when a promoted chat's resume failed and
/// the chat fell back to the handoff prompt. Body: `{ session_id, reason, note }`.
pub const RESUME_FAILED: &str = "resume_failed";
```
**`record.rs`**, `Recorder`, after `record_follow_up` (`:758-781`):
```rust
    /// MOD-37 M5: one `other` row `htui` authors about the session (`role = htui`), such as
    /// [`RESUME_FAILED`](crate::event::RESUME_FAILED). It opens no turn, so it lands in the current one,
    /// after that turn's last row, and the next [`Self::record_follow_up`] opens the next. Like a
    /// follow-up it is not sent to the tab; the caller sends the frame.
    ///
    /// # Errors
    /// [`RecordError::Store`] when the append fails; [`RecordError::Encode`] never, in practice.
    pub async fn record_notice(&mut self, notice: &OtherEvent, at: DateTime<Utc>) -> Result<(), RecordError> {
        let at = stamp(at);
        let mut payload = encode(notice)?; // `{ update, body }`: what `replay` decodes an `other` row as
        self.flush().await?;
        match self.scrubber.scrub(&mut payload) {
            Ok(()) => {}
            Err(unmasked) => return self.refuse(unmasked, at).await,
        }
        self.push(PendingRow {
            kind: EventKind::Other,
            role: EventRole::Htui,
            tool_call_id: None,
            payload,
            raw: Vec::new(),
            at,
        });
        self.flush().await
    }
```
The reason carries the adapter's stderr tail, so it is scrubbed like any payload. A refused scrub
writes the residue row, as `record_follow_up` does.

### agent_worker.rs - types
```rust
/// MOD-37 M5: what a promoted chat opens with when resuming the step's own session fails: the
/// session it tried, and the handoff prompt the engine built for this promotion.
#[derive(Debug, Clone)]
struct ResumeFallback {
    session_ref: AgentSessionRef,
    handoff: String,
}
```
`ChatBinding::Promoted` (`:2195-2202`) gains `fallback: Option<ResumeFallback>` ("`Some` exactly
when the opening is `OpeningPath::Resume`"). The plan says `ChatArgs`; putting the field on the
`Promoted` binding, beside the `tail`, means a fresh chat cannot carry one at all (amendment A-4,
cosmetic). `ChatBinding` stays `Debug`.

`bind_promoted` (`:1033-1036`):
```rust
        let (resume, opening_text, fallback) = match opening.path {
            OpeningPath::Resume { session_ref, text, handoff, .. } => (
                Some(session_ref.clone()),
                text,
                Some(ResumeFallback { session_ref, handoff }),
            ),
            OpeningPath::Handoff { text, .. } => (None, text, None),
        };
```
and `binding: ChatBinding::Promoted { step_id, tail, fallback },`. Update the `attach_promoted` doc
(`:921-934`): "`resume` on [`OpeningPath::Resume`], whose handoff the session falls back to (MOD-37 M5)".

New free fns near `follow_up_frame` (`:4423`):
```rust
/// MOD-37 M5: whether a failed resume is worth a handoff start. Not when the adapter cannot run at
/// all: a missing or unspawnable command (`Spawn`), an unresolved launch placeholder (`Unresolved`)
/// or no transport (`UnknownAdapter`) would fail the handoff start the same way.
const fn falls_back(err: &DriverError) -> bool {
    !matches!(err, DriverError::Spawn(_) | DriverError::Unresolved(_) | DriverError::UnknownAdapter(_))
}

/// MOD-37 M5: the `resume_failed` notice: the session tried, why it failed, and what the chat
/// opens with instead.
fn resume_failed_notice(session_ref: &AgentSessionRef, reason: &str) -> OtherEvent {
    OtherEvent {
        update: htui_agent::event::RESUME_FAILED.to_owned(),
        body: json!({
            "session_id": session_ref.as_str(),
            "reason": reason,
            "note": htui_orch::promote::CONTEXT_NOT_CARRIED,
        }),
    }
}

/// MOD-37 M5: `run_step.opening`, written through the bind's writer. A failed write is logged and
/// never fails the chat: the column is a label, and the session is what the user is waiting on.
async fn record_opening(writer: &Writer, step: StepId, opening: StepOpening) {
    if let Err(err) = writer.record_opening(step, opening).await {
        tracing::warn!(%err, %step, %opening, "the chat's opening could not be recorded");
    }
}
```

### agent_worker.rs - `run_chat` (`:3929-4149`), the control flow
1. **The recorder moves above the start** (D5). The `(ui_tx, ui_rx)` channel, `retain_raw`, the
   `recorder` match over `binding` with `with_quota_latch` and `with_run_cap`, all of `:3984-4002`,
   move to just after `let step_id = binding.step_id();`. A start that fails drops a recorder that
   wrote nothing (`Recorder` has no `Drop` side effect).
2. **The first start**: `let first = driver.start(spec.clone(), prompt.clone()).await;`.
3. Decide:
   ```rust
   let fallback = match &binding { ChatBinding::Promoted { fallback, .. } => fallback.clone(), ChatBinding::Fresh(..) => None };
   // (session, the opening text recorded as the `follow_up`, the opening, the notice owed the tab)
   let started = match (first, fallback) {
       (Ok(session), fallback) => Ok((session, prompt, if fallback.is_some() { StepOpening::Resumed } else { StepOpening::Handoff }, None)),
       (Err(err), Some(fallback)) if falls_back(&err) => {
           let at = Utc::now();
           let notice = resume_failed_notice(&fallback.session_ref, &err.to_string());
           // (a) the column first, so the pane is truthful even if the row write fails;
           record_opening(&writer, step_id, StepOpening::ResumeFailed).await;
           // (b) the row, at the step's current turn, before any second session can write;
           if let Err(record_err) = recorder.record_notice(&notice, at).await {
               tracing::error!(%record_err, "the resume_failed row could not be written");
           }
           let envelope = DriverEnvelope { event: DriverEvent::Other(notice), raw: None, at };
           // (c) the second start: no resume, the handoff text.
           let handoff_spec = SessionSpec { resume: None, ..spec };
           match driver.start(handoff_spec, fallback.handoff.clone()).await {
               Ok(session) => Ok((session, fallback.handoff, StepOpening::ResumeFailed, Some(envelope))),
               Err(err) => Err((err, Some(envelope))),
           }
       }
       (Err(err), _) => Err((err, None)),
   };
   ```
   `StepOpening` comes from `htui_core::model`, and `OtherEvent`/`AgentSessionRef` are imported.
   `spec` is moved only in the fallback arm; the first start took a clone.
4. **`Err((err, notice))`**: today's failure arm, with the notice first. If `notice` is `Some`, call
   `frames.event(notice)` first, so the tab shows the report before the refusal. Then
   `frames.to_stream(Failed { request: binding.request(), message: err.to_string() })`,
   `binding.close(..)`, `frames.failed(..)`, and the D60 `Spawn` reprobe `if let` unchanged. A
   promoted chat's `reprobe` is always `None` (`bind_promoted` `:1080`), so for a promotion this is a
   no-op, as today (amendment A-5). **What the chat answers when both starts fail**: the second
   start's error text, as `Failed { request: "promote_step" }` plus `ChatFrame::Failed`, after the
   `resume_failed` event. The step keeps `opening = resume_failed`, its log has the notice row, and
   it has no `follow_up`.
5. **`Ok((session, opening_text, opening, notice))`**: `frames.accept(ChatAccepted { session_ref:
   session.session_ref().cloned(), .. })` as today. Then, for a promoted binding whose opening is not
   `ResumeFailed`, `record_opening(&writer, step_id, opening).await` (`ResumeFailed` was written at
   3(a)). Then `if let Some(envelope) = notice { frames.event(envelope); }`, then the existing
   prompt/`follow_up` match over `binding`, with `opening_text` in place of `prompt`. A fresh chat
   records no opening.

Order on a fallback, end to end: `opening = resume_failed` → `other/resume_failed` row (seq `last+1`,
turn = tail's last turn, role `htui`) → second `start` → `ChatAccepted` → `ChatFrame::Event(resume_failed)`
→ `follow_up` row with the handoff (seq `last+2`, turn `last+1`) and its frame → the turn loop. The
tab gets the report after the acceptance and before the opening (H-7). A Handoff opening writes
`handoff` and a successful Resume writes `resumed`, each only after its start succeeded. A failed
Handoff start and a non-falling-back resume failure write no opening (H-8).

### T4 red tests
A new test driver beside `SpecSpy` (`:4907`):
```rust
/// MOD-37 M5: the fake driver, failing its first starts with `failures` (front first) and writing
/// down every `(spec, prompt)` it was started with.
#[derive(Debug)]
struct FailingStarts { inner: Box<dyn AgentDriver>, starts: StartLog, failures: Arc<Mutex<VecDeque<DriverError>>> }
type StartLog = Arc<Mutex<Vec<(SessionSpec, String)>>>;
```
`start` records `(spec.clone(), prompt.clone())`, then
`if let Some(err) = failures.pop_front() { return Box::pin(async move { Err(err) }) }`, else
delegates. `DriverError` is `Clone`. Its builder mirrors `FakeBuilder`. The fixture
`fixture_with_failing_starts(script, failures: Vec<DriverError>) -> (MemStore, Backend, AgentRuntime, AgentId, StartLog)`
mirrors `fixture_with_spec_spy` (`:4848-4879`). A helper `opening_of(&store, step) -> Option<StepOpening>`
reads `store.runs(<RUN_1's item>)` → `RUN_1` → `STEP_PLAN`. A `resume(handoff)` path builder:
`OpeningPath::Resume { session_ref: AgentSessionRef::new("banner-1"), text: RESUME_OPENING, handoff: "HANDOFF TEXT", digest: "d" }`.
Cases (c)-(e) attach, then await the task and collect replies, **not** `attach_and_end`: its
`ChatCancel` assertion presumes a chat that is still live. Case (c) uses `attach_and_end`.

| # | Name (`agent_worker.rs` tests) | Asserts | Red on today's code |
|---|---|---|---|
| a | `a_handoff_promotion_records_handoff` | Handoff path. One start, `resume == None`. `opening_of == Some(Handoff)` | Compile (T1 is in); `None` today |
| b | `attach_promoted_resumes_a_cli_step_with_its_banner` (extended, `:5130`) | Plus `opening_of == Some(Resumed)` and no `resume_failed` row | `None` today |
| c | `a_failed_resume_reports_then_opens_the_handoff` | failures `[Transport("session/load failed: no such session")]`. Two starts: the first has `resume == Some("banner-1")` and prompt `RESUME_OPENING`, the second has `resume == None` and prompt `"HANDOFF TEXT"`, with the same `step_id`, `cwd` and `extra_dirs`. The log past the tail is exactly `[other (seq last+1, turn last_turn, role Htui, payload {update: "resume_failed", body: {session_id: "banner-1", reason ∋ "no such session", note: CONTEXT_NOT_CARRIED}}), follow_up (seq last+2, turn last_turn+1, text "HANDOFF TEXT"), …]`. `opening_of == Some(ResumeFailed)`. Replies: `ChatAccepted` at seq 7 comes before a `Chat(Event(Other{update: "resume_failed"}))`, which comes before the `follow_up` event. No `Failed` reply | Today the chat fails (`Failed { promote_step }`), with no row and no second start |
| d | `a_failed_resume_whose_handoff_fails_too_fails_the_chat` | failures `[Transport("a"), Transport("second refusal")]`. Two starts. Replies, in order: `Event(resume_failed)`, then `Failed { request: "promote_step", message ∋ "second refusal" }`, then `Chat(Failed)`. The log has the notice row and no `follow_up`. `opening_of == Some(ResumeFailed)` | One start today; no row; `None` |
| e | `a_resume_that_cannot_spawn_does_not_fall_back` | Run once with `Spawn("gone")` and once with `Unresolved("node")`. One start each, a `Failed` reply, no notice row, `opening_of == None` | Pin (green apart from compile) |

`crates/htui-agent/tests/recorder.rs` `a_notice_is_htuis_other_row_in_the_current_turn`: a continuing
recorder over a tail ending at seq 4, turn 0. `record_notice` writes seq 5, turn 0, `Other`/`Htui`,
payload `{update, body}`. Then `record_follow_up` writes seq 6, turn 1. `replay::envelope_from_row`
of the notice is `Other(notice)`. Red by compile.

`crates/htui/tests/chat.rs` `an_acp_promotion_with_a_banner_resumes_its_session`: `graph_store` with
the scripted row **without** `handoff_only` (a `resumable_graph_store()` variant), then
`parked` + `promote`. The header reads `promoted · <phase> · resumed · scripted · sonnet`, the
follow-up row is `RESUME_OPENING`, and `store.runs(ANA_2)`'s step has `opening == Some(Resumed)`. The
`FakeDriver` ignores `spec.resume` and mints the same `fake-<step>` id, so the banner matches. Red:
`via` is `handoff` today (the CLI-only rule), and `opening` is `None`.

**Gate T4**
```
cargo test -p htui-agent --all-features --test recorder
cargo test -p htui --all-features --lib agent_worker -- --test-threads=1
cargo test -p htui --all-features --test chat -- --test-threads=1
cargo clippy -p htui-agent -p htui --all-targets --all-features -- -D warnings
```
**Commits**: `feat(mod-37): Recorder::record_notice for htui's own other rows` (with its test);
`test(mod-37): a failed resume reports and falls back (red)`; `feat(mod-37): a failed resume falls back
to the handoff in the same bind; run_step.opening is recorded (R-48)`.
---

## T5 - Runs pane and Chat tab (after T1, T4)

**Files**: `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/src/ui/tabs/chat/mod.rs`,
`crates/htui/src/ui/tabs/chat/transcript.rs` (amendment A-9).

### runs.rs
`PANE` is 43 and `INDENT` is 8 (`:131`, `:150`), so a step's extra line has 35 columns. Both plan
strings are wider: "context not carried; handoff prompt only" is 40 columns, and the `resume_failed`
one is 55. `cells::fit` would cut both (H-3). They are wrapped at `; ` onto two lines instead, verbatim
and at the same indent (amendment A-6):
```rust
/// MOD-37 M5 (ANA-27 T5): the two lines under a step whose promoted chat opened without its context,
/// split at `; ` so neither is cut at the pane's 35 free columns. Joined with a space they are
/// `promote::CONTEXT_NOT_CARRIED`, and `resume failed; ` before it.
const OPENING_HANDOFF: [&str; 2] = ["context not carried;", "handoff prompt only"];
const OPENING_RESUME_FAILED: [&str; 2] = ["resume failed; context not carried;", "handoff prompt only"];

/// MOD-37 M5: [`OPENING_HANDOFF`] or [`OPENING_RESUME_FAILED`] under a step whose `opening` says the
/// context was not carried, at any status; nothing for `resumed` or no opening.
fn opening_lines(step: &RunStepSummary, theme: &Theme) -> Vec<Line<'static>> {
    let parts = match step.opening {
        Some(StepOpening::Handoff) => OPENING_HANDOFF,
        Some(StepOpening::ResumeFailed) => OPENING_RESUME_FAILED,
        Some(StepOpening::Resumed) | None => return Vec::new(),
    };
    parts
        .iter()
        .map(|part| {
            Line::from(vec![
                Span::raw(blank(INDENT)),
                Span::styled(cells::fit(part, PANE - INDENT), theme.dim),
            ])
        })
        .collect()
}
```
Call sites: in `list_lines` (`:1648`) and `flow_head` (`:1616`), `lines.extend(opening_lines(step, theme));`
goes right after `lines.extend(note_line(step, theme));` and before the permission lines. R-3's reason
comes first and the opening second. `cursor_end`, the scroll and the height already follow
`lines.len()` (`:1570`, `:1589`, `:1655`).

### transcript.rs
`TranscriptRow` gains:
```rust
    /// MOD-37 M5: a promoted chat's resume failed and it opened with the handoff prompt instead.
    ResumeFailed {
        /// Why, as the driver said it (the adapter's stderr tail included).
        reason: String,
        /// `promote::CONTEXT_NOT_CARRIED`.
        note: String,
    },
```
`apply_other` (`:344-370`), before the catch-all:
`htui_agent::event::RESUME_FAILED => self.rows.push(TranscriptRow::ResumeFailed { reason: str_of("reason"), note: str_of("note") }),`
where an absent `note` reads `promote::CONTEXT_NOT_CARRIED`. Its doc becomes "the rows `htui` authors
itself…". `render_row` (`:501-624`): `resume failed  <first reason line>` in `theme.error`, each
further reason line indented two columns in `theme.dim`, then `note` in `theme.accent`. The live path
(`ChatFrame::Event`) and `from_rows` (replay of the `other` row, `replay.rs` `EventKind::Other` arm)
reach the same arm, so the sentence shows once either way.

### chat/mod.rs - `on_reply`, the `ChatFrame::Event` arm (`:597-606`)
After `self.transcript.apply(envelope);`:
```rust
                // MOD-37 M5 (ANA-27 T5): the resume failed and the chat fell back to the handoff
                // prompt, so the header stops saying `resumed`. `via` was set by `Promoted` before the
                // bind; no other reply carries the outcome.
                if let DriverEvent::Other(other) = &envelope.event
                    && other.update == htui_agent::event::RESUME_FAILED
                    && let Some(promoted) = self.promoted.as_mut()
                {
                    promoted.via = Via::Handoff;
                }
```
The header (`:318-372`) needs no change. `PromotedHeader.via`'s doc gains "turned to `Handoff` by a
`resume_failed` event". There is no new reply or frame type.

### T5 red tests
| # | Name / file | Asserts | Red on today's code |
|---|---|---|---|
| 1 | `runs.rs` `an_opening_without_its_context_adds_two_lines_under_the_step` (beside the R-3 test, `:2000-2030`) | `feat_1_runs()`. `steps[2]` gets `AwaitingApproval`, `gate_note = NOTE` and `opening = Some(ResumeFailed)`. Line `at+2` is the note, `at+3` is `"        resume failed; context not carried;"`, `at+4` is `"        handoff prompt only"`. The non-empty count grows by 3 over the fixture. `steps[1].opening = Some(Resumed)` adds nothing. `steps[0].opening = Some(Handoff)` (status `Done`) adds its two lines | Compile red until T1 (it is in); no lines today |
| 2 | `runs.rs` `opening_lines_fit_the_pane_and_are_never_cut` | Each line's `width() <= PANE`, none ends with `cells::ELLIPSIS`, `OPENING_HANDOFF.join(" ") == CONTEXT_NOT_CARRIED`, and `OPENING_RESUME_FAILED.join(" ") == format!("resume failed; {CONTEXT_NOT_CARRIED}")` | Compile red |
| 3 | `runs.rs` `the_flow_head_shows_the_cursor_steps_opening` | The flow view (the MOD-28 L3 flow-head tests' setup) with the cursor on a step with `opening = Some(Handoff)`: `flow_head` contains both lines; on a `Resumed` step, neither | No lines today |
| 4 | `chat/mod.rs` `a_resume_failed_event_turns_the_header_to_handoff` (beside `:1060`) | `Promoted { via: Via::Resumed }`, then `ChatAccepted` for that step, then `Chat(Event(resume_failed envelope))` (body `{session_id: "s1", reason: "session/load failed: gone", note: CONTEXT_NOT_CARRIED}`). `tab.promoted().via == Via::Handoff`. The rendered header contains `· handoff ·` and not `resumed`. `transcript.rows()` holds exactly one `ResumeFailed { reason, note }`. A later `follow_up` event leaves `via` at `Handoff`. The same event on a fresh (unpromoted) chat changes no header | `via` stays `Resumed` |
| 5 | `transcript.rs` `a_resume_failed_notice_is_one_row_with_its_reason_and_note` | Live `apply` gives one `ResumeFailed` row rendered `resume failed  …` then the note. `from_rows` over the persisted row (`kind: Other`, `role: Htui`, payload `{update, body}`) gives the same rows | Today it is `Other { update: "resume_failed" }`, rendered `· resume_failed` |

No snapshot should move. Fixture steps carry `opening: None`, and `chat__chat_promoted.snap` stays on
the handoff path through T3's `handoff_only` row. A moved snapshot is a finding to read, not to
accept blindly (`cargo insta test --review`).

**Gate T5**: `cargo test -p htui --all-features -- --test-threads=1` (check the `tests/*.rs` counts
are non-zero) and `cargo clippy -p htui --all-targets --all-features -- -D warnings`.
**Commits**: `test(mod-37): the Runs pane and the Chat tab say when context was not carried (red)`,
then `feat(mod-37): ANA-27 T5 - opening lines in the Runs pane; resume_failed flips the chat header`.

---

## T6 - close-out (docs only; as the plan)
`HANDOFF.md` (R-48 struck, "closed by MOD-37 phase 5"; a phase 5 note; the MOD-37 close decision
against the PRD's success metric, with R-32's D131 half, R-44, R-53 and R-55 re-deferred or documented),
`docs/ANA-2.md:1239` §4.8 amendment line (ACP now resumes: `session/resume`, else `session/load` with
the replay discarded; a failed resume falls back to the handoff, labelled), `docs/decisions/mod/mod-37.md`
write-up (it records the amendments below and H-4's open question), the DECISIONS index, and PRD row 5
`complete`. Then `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## Hazards
- **H-1 T3 flips existing pins in three crates.** `FakeSession::open` queues a `session_started`
  banner (`htui-agent/src/fake.rs:231-250`). The seed `claude` row (`AGENT_CLAUDE`) is ACP with
  `acp.session {load: true, resume: true}`. The `chat.rs`/`runs_pg.rs` scripted rows are ACP with
  default settings. So once the CLI-only rule goes, every walked-then-promoted step there resumes.
  Affected: `htui-orch` `conformance.rs:5500`, `:5598`, `engine.rs:14652`; `htui/tests/chat.rs`
  `promotion_opens_the_chat_on_the_same_step`, `the_handoff_opening_is_one_follow_up_row` and
  `chat__chat_promoted.snap`; possibly `runs_pg.rs:700-845`. **Resolution**: T3's list grows
  (amendment A-8). The orch cases pin the resume and read the handoff through `OpeningPath::handoff()`.
  The htui test rows turn resume off (`handoff_only`, both keys), so they keep the path they name. T4
  adds the resumable chat.rs case.
- **H-2 the column-comment pin.** `the_ana_column_comments_are_present_and_verbatim` asserts exactly
  forty-four commented columns over tables that include `run_step`. **Resolution**: 0014 writes no
  `COMMENT ON COLUMN` (D3). Anyone who adds one must add a `MOD37_COLUMN_COMMENTS` chain and move the
  count to forty-five.
- **H-3 the pane strings do not fit.** There are 35 free columns. The strings are 40 and 55.
  **Resolution**: wrap at `; ` (amendment A-6).
- **H-4 a re-promotion after `resume_failed` retries the dead id.** `promote::banner` took the
  **first** `session_started` row by `seq` (`promote.rs:30-52`, blueprint D192). The fallback handoff
  session's banner comes later, so the next promotion would resume the dead session again, fail and
  fall back again. **Resolution (maintainer, 2026-10-03, A-7): `banner` takes the latest
  `session_started` row by `seq`** (`max_by_key`). After a fallback, a re-promotion resumes the
  handoff session, which holds the chat so far. After a successful resume, the restored session's
  banner has the same id, so nothing changes. This amends MOD-4 D192. The work is in T3: see
  "promote.rs".
- **H-5 late replay.** An agent that streams `session/load` replay after its response would leave
  duplicate rows. That is visible, never lost. Accepted per the plan. T2 (b) pins the ordered case,
  and `session/resume` is preferred.
- **H-6 `seq` collision.** A second recorder built from the bind's stale `tail` would reuse
  `last+1`. **Resolution**: one recorder, built before the first start (D5).
- **H-7 frame order.** The notice frame goes after `ChatAccepted` on success and before `Failed` on
  failure. The tab's `ChatFrame::Event` arm needs no session. T4 (c) and (d) pin the order.
- **H-8 an opening can be absent or stale.** A failed Handoff start, or a resume that does not fall
  back, writes nothing. A later promotion whose start fails keeps the earlier value. The pane then
  shows the last chat that actually opened. Accepted, LOW.
- **H-9 stack.** Every promotion now builds the handoff, so the extracted helper sits behind
  `Box::pin` in `opening`. Gate `htui-orch` with `--no-fail-fast` and grep for `SIGABRT`
  (`every_case_name_dispatches`).
- **H-10 `Resume` depends on the handoff.** A missing `handoff` template or an unbuildable phase spec
  now fails a resumable promotion, on the existing "opening cannot be built" path. This is the plan's
  T3 note.
- **H-11 `caps.resume = resume || load`.** An ACP row hands off only with both keys off. Test rows
  that mean "handoff" must set both (H-1).
- **H-12 a resumed session may not offer the model.** If the `session/resume` or `session/load`
  response carries no config options, `model_option` is `None` and step 4 emits its existing
  `model_unavailable` row for the step's model. That is the truth: `htui` could not set it, and the
  restored session keeps its own. Accepted.
- **H-13 MemStore side map.** `delete_project` is the only site that drops steps (`mem.rs:4117`), and
  `openings` is dropped there.
- **H-14 mirror.** An old mirror gets 0005 from `CACHE_MIGRATOR`, then rebuilds on the version
  mismatch. That is one rebuild per box, as with 0011.
- **H-15 `.sqlx`.** Only T1 changes query text, and its expected diff is listed. Prepare against the
  migrated scratch DB, never the empty compose `htui` DB.
- **H-16 test harness.** `tests/*.rs` run 0 tests without `--all-features`; check the counts. The
  `htui` suite is scheduling-dependent: confirm with `--test-threads=1`. Postgres cases skip silently
  without `HTUI_TEST_DATABASE_URL`, so check that the DB was reached.
- **H-17 sandbox.** Check `df -h .` before creating T3's worktree (about 10 GB of `target/`). Dev
  Postgres crash-loops under disk pressure. Remove the worktree before `branch -d`.
- **H-18 what falls back.** `Unresolved` and `UnknownAdapter` are excluded with `Spawn`, because a
  handoff start would fail the same way (amendment A-2).

## Plan amendments (for the maintainer)
- **A-1** `promote::opening_kind` drops its `transport` parameter (`opening_kind(caps, events)`). With
  the CLI-only rule gone it would be unused, which is a warning under `-D warnings`.
- **A-2** T4's fallback rule is "anything but `Spawn`, `Unresolved` or `UnknownAdapter`", not
  "anything but `Spawn`". The same reason covers all three.
- **A-3** T4 also edits `htui-agent`: `event.rs` (`RESUME_FAILED`), `record.rs`
  (`Recorder::record_notice`) and `tests/recorder.rs`. The plan's T4 lists only `agent_worker.rs`.
  `Recorder::record` would author the row as `agent`, and the only `htui`-role writers today are the
  prompt, the cap breach and the scrub residue.
- **A-4** The fallback rides on `ChatBinding::Promoted { fallback }`, not on `ChatArgs`. The effect is
  the same.
- **A-5** Plan T4 (e) says "the reprobe runs as today". A promoted chat never carries a reprobe
  (`bind_promoted` sets `reprobe: None`, `agent_worker.rs:1080`), so (e) asserts no fallback and an
  unchanged failure, and no reprobe.
- **A-6** The Runs pane line is two lines, split at `; ` with the text verbatim, not "a fitted line"
  (H-3).
- **A-7** (decided by the maintainer 2026-10-03: **latest**) `promote::banner` takes the latest
  `session_started` row by `seq`, not the first (H-4). This amends D192 and is implemented in T3.
- **A-8** T3's file list grows: `htui-orch/src/conformance.rs`, the `engine.rs` test,
  `htui/tests/chat.rs` and `htui/tests/runs_pg.rs` (H-1). This stays disjoint from T2, which touches
  only `htui-agent`.
- **A-9** T5 also edits `chat/transcript.rs`. Today an unknown `other` update renders as
  `· resume_failed` only, which cannot "show the sentence".
- **A-10** `htui-core/src/store/mod.rs` and `htui-store/src/worker.rs` (plan T1's list) do not change.
  There is no new store type, and the op stays off `WorkerStore` (D2).
- **A-11** T1's conformance work is one case, not several. Its assertions cover round-trip, replace,
  `NotFound` and `updated_at`. The counts go 135 → 136.

## Build order
1. **T1** on `hr/MOD-37` (main tree): migrations, the enum, the field, the op, the three builders,
   `.sqlx`, the pins. Commit.
2. **T2** on the main tree **∥ T3** in `../htui-m5-t3` (branch `hr/MOD-37-m5-t3` from T1's head). T3
   touches `htui-orch`, `htui/src/agent_worker.rs` (two lines) and the htui test rows; T2 touches only
   `htui-agent`. T2 commits, then T3 is merged `--no-ff`. Run T2's and T3's gates again on the merged tree.
3. **T4** on the merged tree (it needs T1's op, T2's restore, and T3's `handoff` field and
   `CONTEXT_NOT_CARRIED`).
4. **T5** (it needs T1's field and T4's `RESUME_FAILED`).
5. **T6** docs.

## Validation (the plan's, on the final tree)
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast 2>&1 | tee /tmp/m5-gate.log; grep -c SIGABRT /tmp/m5-gate.log   # 0
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/m5-orch.log; grep -n SIGABRT /tmp/m5-orch.log   # nothing
cargo test -p htui --all-features -- --test-threads=1          # scheduling-dependent suite; check the tests/*.rs counts
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
