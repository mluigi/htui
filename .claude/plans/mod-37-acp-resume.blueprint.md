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
