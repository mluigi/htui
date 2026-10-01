# Blueprint: MOD-37 milestone 1 - run state and visibility

Input: `mod-37-run-state.plan.md` (CONFIRMED) and the PRD. Decisions fixed by the maintainer: R-44 and R-53 re-deferred (no code); R-3 render is a third line unless the 43-column test says otherwise, never writing `run.failure`; R-40 reuses `FrameKind::Changed`; T7 spikes option (b) first, falls back to (a).

Line numbers are from the tree at `41f1508`. Run-lifecycle tests live in two places: `htui-worker/src/runtime.rs` (module `role_gate`, Recording/Timed sinks) and `crates/htui/src/run_worker.rs` (store-loop tests: `frames()` `:2067`, `run_stream_frames_carry_the_subscription_seq` `:2092`, `a_command_preempted_in_the_lock_queue_publishes_its_refusal` `:~2243`). T3 and T4 tests go in the latter, which the plan did not list (flagged below).

## Order and lanes
Lane 1 serial: T1, T2, T3, T4, T5. Lane 2: T6 (shares no file with lane 1). Then one docs commit (close-out). Implementers commit per step; each commit is green on its own.

## Gates (every task; scope with `-p` while iterating, workspace before hand-off)
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p <crates> --all-features -- --test-threads=1
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/orch.log; grep -n SIGABRT /tmp/orch.log   # must print nothing
cargo insta test -p htui --all-features --check -- --test-threads=1                                  # pane tasks; expect zero diffs
```
`htui` integration tests (`tests/chat.rs`) need `--all-features` (testkit) or they run 0 tests and report ok. Check the test count is non-zero.

## T1 - R-3 `gate_note` on `RunStepSummary`
**Files (touched set):** `htui-core/src/model/run.rs`, `htui-core/src/store/mem.rs`, `htui-core/src/store/conformance.rs`, `htui-store/src/pg/rows.rs`, `htui-store/src/pg/read.rs`, `htui-store/src/cache/read.rs`, `htui-store/.sqlx/` (one file deleted, one added), `htui-orch/src/closeout.rs`, `htui/src/ui/tabs/backlog/detail/runs.rs` (test fixtures only), **plus** `htui-store/tests/cache.rs` (plan omitted it; it is the only mirror leg).
**Symbols:**
- `RunStepSummary` `run.rs:703`: add `pub gate_note: Option<String>` after `agent_name` (doc: `run_step.gate_note`).
- `State.run_steps` `mem.rs:1275`: `gate_note: step.gate_note.clone()`.
- `StepRow` `rows.rs:~137` (after `agent_name`) and `StepRow::into_summary` `rows.rs:145`: append the field last (positional `query_as!`).
- `PgStore::runs` `pg/read.rs:400`: the SELECT ends `a.name AS "agent_name?"`; add `, s.gate_note` after it (comment: appended, positional).
- `CacheStore::runs` `cache/read.rs:~560-615`: add `s.gate_note` to the SELECT, `gate_note: opt_text(row, "gate_note")?` in the builder. This is `sqlx::query` (string), so no prepare entry. `cache/refresh.rs:1190` already mirrors the column.
- Fixtures that spell every field need `gate_note: None`: `closeout.rs:272`, `runs.rs` tests at `:1765, :1941, :2041, :2055, :2129, :2150, :2580, :3455` (the `..base` ones need nothing). The compiler lists them.
**Failing test first.** The seeded fixtures all have `gate_note: None` (`fixtures.rs:1512,1571,1607`), so extending leg (g) (`run_and_steps_round_trip` `conformance.rs:12756`) cannot go red. Use instead:
1. `gate_answers_write_their_outcome` (`conformance.rs:~9531`, `approved` step of RUN_2, note `"looks right"`): after the existing row assertion add `store.runs(ids::HTUI_FEAT_3)` -> RUN_2 -> `approved` summary, `assert_eq!(listed.gate_note.as_deref(), Some("looks right"), "{CASE}: the summary carries the note the row does")`.
2. `the_0003_columns_reach_the_mirror` (`cache.rs:556`): add `gate_note = 'parked: no tests'` to the `UPDATE run_step ... WHERE id = $1` for `STEP_IMPL`, and `step.gate_note.as_deref()` to the compared tuple. Cache equals Postgres is already asserted below it.
Protocol: add the field with `gate_note: None` in the three builders (compiles), write the two assertions, run, see red (Mem, then Pg and cache), then fill the builders. Pg and cache cases skip silently without a DB (`common::demo_db()` returns `None`): the red run proves the DB was reached.
**sqlx (docs/hr-sandbox.md:196):**
```
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features
cargo sqlx prepare --check
```
Expect exactly `query-aede103b...json` deleted and one new file; nothing else in `.sqlx`. "Potentially unused queries" from `--check` is expected. The test DSN and the prepare DSN are different databases; the compose `htui` database is empty.
**Gate:** `cargo test -p htui-core -p htui-store --all-features -- --test-threads=1`; `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1` + SIGABRT grep (closeout fixture); `cargo test -p htui --all-features --no-run`.
**Commit:** one: `feat(mod-37): carry gate_note on RunStepSummary (R-3)` (field, three builders, fixtures, tests, `.sqlx`; body states the red run).
**Data flow:** `run_step.gate_note` (written by `answer_gate`, the judge park, `interrupt_step`) -> three builders -> `RunSummary.steps[].gate_note` -> `StoreReply::Runs` -> pane.
**Must NOT touch:** migrations, `RunStep`, `run.failure` or any writer, `pg/write.rs`, `cache/refresh.rs`, any pane render code, `.sqlx` by hand, `fixtures.rs` (a note there churns snapshots).

## T2 - R-3 render
**Files:** `crates/htui/src/ui/tabs/backlog/detail/runs.rs` only.
**Symbols:** new `fn note_line(step: &RunStepSummary, theme: &Theme) -> Option<Line<'static>>` beside `step_lines` (`:1061`): `Some` only for `StepStatus::AwaitingApproval` with a non-empty `gate_note`; `Span::raw(blank(INDENT))` + `fit(note, PANE - INDENT)` in `theme.accent` (the `cancel requested` precedent, `run_lines:987`). `list_lines` (`:1449`): after `step_lines(..)` and before the `permission_lines` block, `lines.extend(note_line(..))`; `cursor_end = lines.len()` already follows, so scroll counts it.
**Do not change `step_lines`' `[Line; 2]` return**: `every_step_row_fits_forty_three_columns` (`:1866`) destructures it.
**Failing tests first** (red because `note_line` does not exist; then assert):
- `a_parked_step_shows_its_reason_on_a_third_line` (beside `:1781`): pane from `feat_1_runs()` with step 2 `AwaitingApproval` and note `"judge failed: orderings disagree"`; in `lines(&pane,&shell)` the row after that step's second line starts with 8 spaces and contains the note; steps without a note still take two (the existing `every_step_takes_two_lines` stays untouched and green).
- `a_reason_line_is_forty_three_columns_whatever_the_note` (beside `:1866`): 300-char note containing `\n` and a tab: `note_line(..).width() == PANE`, ends with `CUT`.
Decision rule: third line wins when both pass. Fall back to folding on the run line only if the width test shows the third line cannot hold the sentence readably (35 visible columns); no `run.failure` write either way.
**Gate:** pane tests; `cargo insta test -p htui --all-features --check` (no snapshot should change: no demo step has a note; a diff means T1 leaked a fixture note).
**Commit:** `feat(mod-37): show a parked step's reason in the Runs pane (R-3)`.
**Must NOT touch:** `step_lines`/`tail`/`TAIL_WIDTH` (R-44), `run_lines`, `on_reply`, any `htui-*` crate.

## T3 - R-40 step-start frame
**Files (refined):** `htui-orch/src/engine.rs`, `htui-worker/src/views.rs`, `crates/htui/src/run_worker.rs` (tests). Dropped from the plan's set: `runtime.rs`, `runs.rs` (`Changed` already invalidates, `runs.rs:626`), `fake.rs` (the `FakeOrchestrator` impl at `engine.rs:6610` takes the default).
**Symbols:**
- `trait SessionSink` `engine.rs:~193-201`: add a **sync**, defaulted, infallible `fn started(&self, _item: ItemId, _run: RunId, _step: StepId) {}`. Sync keeps the big engine futures from growing (stack headroom at the 2 MiB test limit).
- New private `Engine::step_started(&self, run: &Run, step: StepId)`: `if let Ok(item) = Self::item_of(run) { self.parts.sink.started(item, run.id, step) }`. Call it right after each successful `move_step(.., Pending, Running, ..)?`: `walk_step` `:3321-3344`, `run_candidate` `:3937`, `run_judge` `:4476`. Not `fail_group_before_a_token` `:3897` (the step is failed in the same breath) and not the sweep's `interrupt_step` moves.
- `impl SessionSink for ProgressSink<S>` `views.rs:525`: `fn started(..)` publishes `RunFrame { item, run: Some(run), kind: FrameKind::Changed }` through `self.publisher`. Doc comment on `FrameKind::Changed` (`:160`) gains "a step went live".
**Failing tests first:**
- `engine.rs` tests (mirror `Stranger` `:9665`): `a_sink_hears_started_before_after_done_for_each_step_that_goes_live`: a recording sink over `FakeOrchestrator` appends `"started"`/`"after_done"`; a one-step walk gives `["started","after_done"]`. Red: the sink method does not exist (compile) then empty log.
- `run_worker.rs` (mirror `:2243`): `a_step_that_goes_live_publishes_a_frame_before_its_session_ends`: `parked(..)`, drain `frames(&mut worker)`, push `Play::Stall`, send `RetryStep`, `stall.reached.notified()`, then `frames(..)` contains a `FrameKind::Changed` with `run == Some(run)` while the session is still stalled. Red: none arrives until the walk rests.
`FrameKind::Changed` is kept unless the second test shows the pane cannot tell the frame from a refresh; it does not need to, since both re-read.
**Gate:** `cargo test -p htui-orch -p htui-worker --all-features --no-fail-fast -- --test-threads=1` + SIGABRT grep; `cargo test -p htui --all-features -- --test-threads=1` (frame-count assertions in `run_worker.rs`: `a_command_with_no_walk_still_publishes_a_frame` expects exactly 1 frame; it runs no step, so it is unaffected, but confirm).
**Commits:** (1) `feat(mod-37): SessionSink::started, called after each step goes live` (engine + its test); (2) `feat(mod-37): publish a Changed frame when a step starts (R-40)` (views + run_worker test).
**Must NOT touch:** `after_done` or any existing `SessionSink` impl (`NoSink :216`, `FakeOrchestrator :6610`, `Stranger :9670`, `SupersedeWinner conformance.rs:7471`, gix test sinks), `runtime.rs`, `runs.rs`, `FrameKind` variants.

## T4 - R-51 waiting frame
**Files:** `htui-worker/src/runtime.rs`, `htui-worker/src/views.rs`, `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/src/run_worker.rs` (tests).
**Symbols:**
- `FrameKind::Waiting` (new, `views.rs:~166`, doc: "a command is queued behind a live walk of the run"). Compiler-forced: `invalidates` `runs.rs:620` returns `false` for it (fix its doc: "every kind but `Subscribed` and `Waiting`").
- `Shared::lock_unless_cancelled` `runtime.rs:452`: new `lock_announcing(run, walk, waiting: impl FnOnce())`. Order: (1) if `walk.token.is_cancelled()` or the signal reads cancel, `None` (keeps the biased cancel-first rule); (2) `self.locks.try_lock(run)` hit -> `Some(guard)`; (3) miss -> `waiting()`, then the existing `select!`. `lock_unless_cancelled` becomes `lock_announcing(.., || {})`, so `reclaim :1723`, `resumed :1929`, `start_run :2094` stay byte-identical (sweep tasks answer nobody; a fresh run has no walk).
- Four command callers switch to `lock_announcing(run, &walk, || ctx.publish(Some(run), FrameKind::Waiting))`: `on_run_unless_claimed :2182` (all gate, retry, select verbs; it tags the item first), `cancel_run :2396`, `unblock :2518`, `cleanup :2540`. `TaskCtx::publish` no-ops without an item tag.
- Pane: `RunsTab` gets `waiting: BTreeSet<RunId>`; `on_reply` `:1351` new arm before the invalidating one: `RunStream(frame)` for this item with `Waiting` inserts `frame.run` and emits nothing; the invalidating arm removes `frame.run` (all when `None`). `list_lines` pushes `fit(WAITING_LINE, PANE)` in `theme.accent` after `run_lines` (do not change `run_lines`' signature, `:2002` test calls it), `WAITING_LINE = "waiting for the walk"`.
**Failing tests first:**
- `run_worker.rs` (clone of the preempted test `:~2243`): `a_command_queued_behind_a_live_walk_publishes_a_waiting_frame`: stalled retry holds the lock, `AnswerGate` queues; after the 200 ms sleep `frames(..)` holds exactly one `Waiting` with `run == Some(run)` and no `Rested` yet. Red: none.
- `runs.rs` (beside `:3289`): `a_waiting_frame_shows_the_run_waiting_and_a_later_frame_clears_it`: after `on_reply(Waiting)` `shell.emit` is empty (no re-read), `lines()` contains the sentence (<= 43 columns); after `Rested` it is gone and one re-read was requested. Extend nothing in `a_run_stream_frame_re_reads_the_runs`.
**Gate:** `cargo test -p htui-worker -p htui --all-features -- --test-threads=1`; insta check; SIGABRT grep not needed (no orch change).
**Commits:** (1) `feat(mod-37): FrameKind::Waiting, which the pane does not re-read on`; (2) `feat(mod-37): publish Waiting when a command queues behind a live walk (R-51)`; (3) `feat(mod-37): the Runs pane says a command is waiting for the walk`.
**Must NOT touch:** the cancel/preempt semantics, `walked`, `Walks`, `RunLocks`, the three sweep callers, any std mutex across an `.await`, `engine.rs`.

## T5 - R-41 surface a dropped failure
**Files:** `runs.rs` only.
**Symbol:** `RunsTab::on_reply` `:1351`, the invalidating `RunStream` arm: when `frame.kind` is `FrameKind::Error(sentence)`, `ctx.emit(Action::Error(sentence.clone()))` before `re_read` (`Action::Error` through `ctx.emit` is already used in `on_document :~432`). The frame carries the subscription's `seq`, which stays fresh (`run_stream_frames_carry_the_subscription_seq`), so it survives `App::is_fresh` (`state.rs:329`) where an `Orch` `Failed` reply does not.
**Failing test first:** `a_run_stream_error_frame_reaches_the_status_line` (beside `:3289`): `on_reply(frame(FEAT_1, Error("the walk failed")))`; `shell.emit.take()` holds `Action::Error("the walk failed")` and one re-read; another item's `Error` frame emits nothing. Mirror of `update.rs:692`, at pane level (the app-level gate needs no edit: the frame is not stale).
**Risk:** a refused command publishes the frame then answers `Failed`, so the status may be set twice (frame sentence, then `"name: sentence"`, same channel order). Exact-status assertions in `tests/chat.rs` (`a_second_promotion_is_refused_while_a_chat_is_open`: `"promote_step: end the open chat first..."`) rely on the later write winning; run the full `-p htui --all-features -- --test-threads=1`. Errors of an item the pane is not on stay unreported (per-item stream); note it, do not widen.
**Commit:** `fix(mod-37): a RunStream error frame reaches the status line (R-41)`.
**Must NOT touch:** `app/update.rs`, `app/state.rs`, `Action`, `StoreReply`, the worker.

## T6 - T7 chat stream residual window
**Spike first (no commit; record the verdict in the first commit body).** Option (b) needs, at accept time, a non-`Orch` `ReplyAddr` that `App::is_fresh` passes. Check: (1) `bind_promoted` `agent_worker.rs:~951` receives only the promotion's `addr` (an `Orch` one); (2) freshness is `App.latest[(origin, request kind)] == seq`, private to the shell (`state.rs:307-331`), so the worker cannot mint a fresh seq; (3) the `Publisher`'s subscription address (`address.rs:~160`) is the Backlog origin's, not the Chat tab's. If all three hold, (b) has nothing to move to: take (a). If the spike finds a Chat-origin non-`Orch` fresh address, use it and stop.
**Option (a) design.** Issue the follow when the tab learns the step, not at the acceptance.
- `chat/mod.rs` `Promoted` arm `:625`: after the resets, `ctx.request(StoreRequest::ChatFollow { step_id: *step })`; set new field `followed: Option<StepId> = Some(*step)`.
- `ChatAccepted` arm `:549-568`: send `ChatFollow` only when `followed != Some(*step_id)`; then `followed = None`. A first acceptance (already followed) sends nothing, because a second follow supersedes the first in `App::latest` and would reopen the window. A re-sent acceptance from `hand_over` (D185) still follows.
- `agent_worker.rs`: `AgentRuntime` gets `pending_follow: Option<(StepId, ReplyAddr)>`. `follow` `:~1925`: live entry open -> `chat.stream.follow(addr)` as now; entry absent -> stash; entry present but closed -> no-op (an ended chat has sent its last frame). `bind_promoted` `:~951`: at `Frames::new(replies.clone(), addr)` use the stashed address when `pending_follow` names this step (take it), and take it on every refusal path (including the `hand_over` branch) so no stale slot survives. One slot, so memory is bounded.
- `store_worker.rs:213`: doc comment of `ChatFollow` only.
**Failing tests first:**
- `tests/chat.rs`, through the store loop like `promoting_a_running_step_preempts_its_walk` (`:~1779`, raw `send(seq, origin, request)`): `a_follow_served_before_the_bind_keeps_every_chat_frame_fresh`: send `Orch(PromoteStep)` at seq 2 from the chat origin; on its `Promoted` reply send `ChatFollow` at seq 3, then a second `Orch` request at seq 4; drive one turn; assert `ChatAccepted` and every `StoreReply::Chat` envelope carry `seq == 3`. Red: today the follow lands before the bind and is a no-op, so they carry seq 2 (superseded by 4).
- `chat/mod.rs` unit test beside `:1070` (`a_promotion_opens_on_its_step...`): `a_promoted_reply_asks_for_the_follow_at_once`: `Promoted` emits one `ChatFollow`; the first `ChatAccepted` emits none; a second `ChatAccepted` for the same step emits one.
- Regression guards that must stay green unchanged: `a_second_promotion_is_refused_while_a_chat_is_open` (`tests/chat.rs:1688`), `two_promotions_before_a_bind_start_one_session` (`:1728`).
**Gate:** `cargo test -p htui --all-features -- --test-threads=1` (testkit; the keyring fake is process-wide, so never parallel); fmt; clippy.
**Commits:** (1) `fix(mod-37): a ChatFollow served before the bind is kept for the bind (T7)` (agent_worker + store_worker doc + chat.rs test); (2) `fix(mod-37): the Chat tab follows a promoted chat when the promotion answers (T7)` (chat/mod.rs + unit test).
**Must NOT touch:** `Stream::hand_over`, `Stream::follow`, `Frames`, the `Orch` request path, `htui-worker`, `run_worker.rs`, `testkit.rs` (the raw store-loop test needs no harness change), `runs.rs`.

## Touched-file sets and intersections
| Task | Set | Versus the plan |
|---|---|---|
| T1 | core `run.rs`, `mem.rs`, `conformance.rs`; store `pg/rows.rs`, `pg/read.rs`, `cache/read.rs`, `.sqlx`, `tests/cache.rs`; orch `closeout.rs`; `runs.rs` (fixtures) | adds `tests/cache.rs` |
| T2 | `runs.rs` | same |
| T3 | `engine.rs`, `views.rs`, `htui/src/run_worker.rs` | drops `runtime.rs`, `runs.rs`, `fake.rs`; adds `run_worker.rs` |
| T4 | `runtime.rs`, `views.rs`, `runs.rs`, `htui/src/run_worker.rs` | adds `run_worker.rs` |
| T5 | `runs.rs` | same |
| T6 | `agent_worker.rs`, `chat/mod.rs`, `store_worker.rs` (doc), `tests/chat.rs` | drops `testkit.rs` |
Beyond the plan: `run_worker.rs` is shared by T3 and T4 (already serial). T1 also touches `runs.rs` fixtures (as planned). Lane 2 shares nothing with lane 1.

## Close-out (after T6, one commit)
`docs(mod-37): M1 close-out, R-44 and R-53 re-deferred`: HANDOFF R-lines (R-3, R-40, R-41, R-51, T7 closed; R-44 re-deferred to the Runs pane's next layout change; R-53 "mitigated by D171/D184, no code"), the MOD-37 write-up and DECISIONS index as the MOD-66 close-out did. Reviewer gate over the full change set before this commit.
