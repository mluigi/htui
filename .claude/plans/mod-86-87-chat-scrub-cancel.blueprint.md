# Blueprint: MOD-86 + MOD-87, chat prompts scrubbed before sending and chat cancel answered mid-turn

**Plan**: `.claude/plans/mod-86-87-chat-scrub-cancel.plan.md` (confirmed 2026-10-07; D1-D10, D5 = no UI change).
Produced by `code-architect`. One serial lane, one implementer, a commit after each of T1, T2 and T3, then T4 (gates and
close-out). Line numbers are for `baf0a8fd` (`crates/htui/src/agent_worker.rs` unless a path says otherwise). They
drift as soon as T1 lands, so anchor on names.

## Plan amendments (for the session to accept or reject before T1)

- **A-1 (defect, D2/D8): the between-turns wait must be an inner loop.** Today a `ChatCommand::Answer` between turns
  (`:4857-4866`) is refused and then falls out of the `match`. The `loop` (`:4768`) then re-enters `run_turn` with no
  follow-up sent, and `next_event` runs on an idle session. On the fake an idle pull is `Ok(None)`
  (`htui-agent/src/fake.rs:454-456`), so `run_turn` returns `Err(DriverError::Closed)` (`:5040-5045`) and the chat
  **fails**. On a real transport the pull blocks and the chat wedges. A stale permission digit that lands after the
  turn's `done` is enough to cause it. D8's refusal arm hits the same bug if it is written as a fall-through. The
  fix: the wait between turns becomes an inner loop that leaves only once a follow-up was sent or the chat ends
  (section 2.4). T2 pins it with `a_stale_answer_between_turns_is_refused_and_the_chat_goes_on`.
- **A-2 (defect, T1 "call sites do not change"): `a_cancel_queued_at_the_acceptance_cancels_the_help` (`:6961-6996`)
  must keep the queued-first shape.** It drives a **help** through `run` with a script that never emits `Done`
  (`ScriptEvent::ExpectCancel`, `:6970`), and what it pins is the MOD-55 A-2 cancel read before the first pull.
  Under a Done-wait it hangs. A Done-wait cancel on a help is also racy anyway: help closes its receiver *before*
  `recorder.finish` (`:4895-4897`). The fix: the old `run` body survives as `run_with_cancel_queued`, and this one
  call site switches to it.
- **A-3 (T1 hosted `Live::end`): the helper cannot see frames.** The test owns the receiver (`let (tx, _rx)` at every
  call site, `:8105-8417`), and `Live` holds only the sender. The fix: wait on the step's **log** for a `done` row
  past the rows that existed before the task was spawned. `live()` gains a `&Backend` parameter to read that
  baseline, which changes 7 call sites mechanically (section 1.4).
- **A-4 (fact, D10 "no production effect today" is incomplete).** A help in a **provider** project resolves its
  secrets in `run_chat` (`start` builds `chat_secrets` for both modes, `:2392-2405`; `run_chat` resolves them,
  `:4567-4588`). But `start` scrubbed the help prompt over `cfg(not(test))`'s empty env (`:2290-2291`). So a
  provider value in an edited item's body reaches the help driver unmasked. **Recommendation:** run D7's `run_chat`
  opening pass in **every** mode (no `is_help` branch). That closes the gap for help at no cost. The second pass is
  idempotent over an already-masked assembly: `new` masks everything `from_resolved` masks, plus short values, and
  `mask` steps over `[REDACTED]`. The constructor mismatch itself stays D10's. If this is rejected, gate the pass with
  `if !mode.is_help()`.
- **A-5 (gap, T2 tests): no existing stall lets a turn finish after a mid-turn command.** `StallAt::AfterChunk`
  releases only on `cancel` (`:6766`, `!self.cancelled`). The fix: a third variant,
  `StallAt::AfterChunkUntilReleased` (section 2.6).
- **A-6 (gap, T3 tests): the fake records no follow-up.** `SpecSlot` keeps the spec only (`fake.rs:94-120`).
  `FailingStarts` keeps `(spec, prompt)` per start (`:5888-5930`), and no wrapper sees `send_follow_up`. The fix: a
  `SendSpy` session wrapper inside `FailingStarts` (section 3.5).

## 0. Shapes (types and signatures)

- `use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};` and
  `use htui_core::prompt::{SectionRefused, scrub_section};` (both are `pub` in `htui-core/src/prompt/mod.rs:1311,1327`).
- **`StartFailure`**, private, placed beside `TurnEnd` (`:4524`):

```rust
/// MOD-86 D7: why a chat's session did not start: the transport refused it, or the text it was to
/// open with did not scrub clean and was never sent.
enum StartFailure {
    Driver(DriverError),
    Refused(SectionRefused),
}

impl StartFailure {
    /// The sentence the `Failed` reply and the stream's `Failed` carry.
    fn message(&self) -> String {
        match self {
            Self::Driver(err) => err.to_string(),
            Self::Refused(refused) => format!("not sent: {refused}"),
        }
    }
}
```

  **Why an enum and not a `DriverError` or a `String`:**
  - A `DriverError::Transport("not sent …")` would claim a wire fault where nothing reached a wire. `falls_back` and
    D60's `Spawn` re-probe both read `DriverError`, and a scrub refusal must never trip them.
  - A bare `String` loses the `DriverError::Spawn` test that gates the re-probe (`:4716`), so it would need a second
    flag beside it.
  - The enum keeps both facts at the cost of one `match`.
- **`run_turn`** (`:4933`):

```rust
async fn run_turn(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    commands: &mut mpsc::UnboundedReceiver<ChatCommand>,
    deferred: &mut VecDeque<ChatCommand>,
    policy: &PermissionPolicy,
    calls: &mut HashMap<String, ToolCallEvent>,
    frames: &Frames,
    grace: Duration,
    help: bool,
) -> Result<TurnEnd, DriverError>
```

  Keep the existing `#[expect(clippy::too_many_arguments, …)]`.
- **`turn_command`** (replaces `help_command`, `:5104`). It has 8 parameters, so it takes the same `#[expect]` with
  reason "one turn's collaborators plus the command":

```rust
async fn turn_command(
    help: bool,
    deferred: &mut VecDeque<ChatCommand>,
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, Writer>,
    ui: &mut mpsc::Receiver<DriverEnvelope>,
    frames: &Frames,
    grace: Duration,
    command: Option<ChatCommand>,
) -> Option<TurnEnd>
```

- **`answer_queued`** (replaces `answer_after_help`, `:5162`):

```rust
async fn answer_queued(
    deferred: &mut VecDeque<ChatCommand>,
    commands: &mut mpsc::UnboundedReceiver<ChatCommand>,
    frames: &Frames,
    last_stop: StopReason,
)
```

## 1. T1: the test helpers cancel after the turn (behaviour-neutral on the unchanged runtime)

### 1.1 Inventory (every `ChatCancel` in tests; `crates/htui/tests/*.rs`, `store_worker.rs` and `testkit.rs` checked)

| Site | Kind | T1 action |
|---|---|---|
| `run` `:6022` (16 callers, listed below) | chat, queued before the first poll | body becomes `end_after_turn` (1.2) |
| `a_cancel_queued_at_the_acceptance_cancels_the_help` `:6974` | help through `run`, never emits `Done` | switch to `run_with_cancel_queued` (A-2) |
| `a_cancel_mid_turn_closes_the_help_cancelled` `:6611` | help, inline queued cancel, parked script | **unchanged** (help already listens; it pins exactly that) |
| `cancel_at_stall` `:6861` | help, served at the stall | unchanged in T1; generalized in T2 (2.6) |
| `attach_and_end` `:7074` (9 callers) | promoted, queued before the first poll | `between(step_id).await`, then `end_after_turn` |
| `live_steps_drops_an_ended_chat` `:7946` | chat, queued before the first poll, `_rx` unused | rename `_rx` to `rx`, then `end_after_turn` |
| hosted `tool_leases::end` `:8069` (7 callers) | task spawned, test owns `rx` | log wait (1.4, A-3) |
| `chat_secrets::end` `:15491` (2 callers) | chat, queued before the first poll | replace the fn with `use super::end_after_turn as end;` |
| `tests/chat_live.rs:78-88`, `chat_live_cli.rs:296-318`, `chat_live_agy.rs:306-326` | already serve after `Done` | none |
| `tests/chat.rs` (harness `Esc Esc` after `harness.drive()`) | keys after a quiet drive | none (`testkit.rs:240-265`: a drive settles the turn first) |
| `testkit.rs:284`, `store_worker.rs:2019,3009` | request routing lists | none |

The `run` callers: `:6065`, `:6425`, `:8309` (hosted), `:8472`, `:8489`, `:8532` (cap), `:8656`, `:8696`, `:10547`,
`:13884` (recorder refusal), `:15528`, `:15581` (D13 refusal: no `Done`, the task ends by itself), `:15796`, `:15836`.

**Where the Done-wait needs care.** Every chat caller's script emits a `Done` frame. In the recorder-refusal case
(`:13872`) the chunk is refused at capture (`Recorder::record` then `refuse`, `record.rs:977-983`), and `done` still
records and frames. The cap case frames the cap's own `done{cancelled}`, and on the current-thread runtime the stream's
`Ended` usually follows in the same poll, so the stream-ended guard below skips the cancel. If it does not, the cancel
is answered: the old runtime drops it, and the new one answers it `Ended{Cancelled}` after the stream's `Ended`. Either
way `replies.last()` at `:8572` is still `Ended{Cancelled}`. A script that parks a request policy cannot answer, or
that ends in `ExpectCancel`, never emits `Done`. Only `:6974` does that, and A-2 handles it. The 30 s timeout in the
helper turns any future one into a failure instead of a hang. No test in this module uses paused time or
`flavor = "multi_thread"`.

### 1.2 `converse` and `end_after_turn` (test module, beside `run`)

```rust
/// Whether `reply` is a turn's `done` as the stream carries it.
fn is_done_frame(reply: &ReplyEnvelope) -> bool

/// Whether the stream (seq 7: `run`'s `ChatStart`, `promote_addr`) has sent its `Ended`.
fn stream_ended(replies: &[ReplyEnvelope]) -> bool

/// Drives `task` **inline** and collects every reply. After the k-th `Done` frame it serves `batches[k]`
/// (each asserted `Served::Deferred`) unless the stream has already ended. `tests/chat_live.rs:78-88`'s shape:
/// since MOD-87 a chat reads commands while it pulls, so a command queued before the first poll would land
/// mid-turn.
async fn converse(
    runtime: &mut AgentRuntime,
    backend: &Backend,
    (tx, mut rx): (mpsc::UnboundedSender<ReplyEnvelope>, mpsc::UnboundedReceiver<ReplyEnvelope>),
    task: impl Future<Output = ()>,
    batches: Vec<Vec<RequestEnvelope>>,
) -> Vec<ReplyEnvelope>

/// `Esc Esc` once the first turn is done: `converse` with one batch, `ChatCancel` at seq 8.
/// A chat that ends by itself before a `Done` (a refused or failed start) is sent none.
async fn end_after_turn(
    runtime: &mut AgentRuntime,
    backend: &Backend,
    channel: (mpsc::UnboundedSender<ReplyEnvelope>, mpsc::UnboundedReceiver<ReplyEnvelope>),
    step_id: StepId,
    task: impl Future<Output = ()>,
) -> Vec<ReplyEnvelope>
```

The body of `converse`, wrapped in `tokio::time::timeout(Duration::from_secs(30), …).expect("the chat ends")`:

```rust
let mut task = std::pin::pin!(task);
let mut batches = batches.into_iter();
let mut replies = Vec::new();
loop {
    tokio::select! {
        biased;
        Some(reply) = rx.recv() => {
            let done = is_done_frame(&reply);
            replies.push(reply);
            if done {
                // Everything the task sent before it yielded, so a stream that already ended is seen.
                while let Ok(more) = rx.try_recv() { replies.push(more); }
                if let Some(batch) = batches.next() && !stream_ended(&replies) {
                    for request in batch {
                        let served = runtime.serve(backend, &tx, &request).await;
                        assert!(matches!(served, Served::Deferred), "{served:?}");
                    }
                }
            }
        }
        () = &mut task => break,
    }
}
drop(tx);
while let Some(reply) = rx.recv().await { replies.push(reply); }
replies
```

- **Inline, not `tokio::spawn`.** `:7696` installs a thread-local `tracing` default around `attach_and_end`, and inline
  polling keeps the "awaited inline by the harness" property `run_chat`'s D60 comment relies on (`:4708-4711`).
  Spawned tasks (the T2 stall cases) pass `async move { handle.await.expect("the chat task does not panic") }`.
- **Why this is neutral on the old runtime.** The `Done` frame is forwarded synchronously inside `record`
  (`:5207-5222`), and `run_turn` returns right after it with no further `select`/`recv` (`:5094`). A cancel served
  after the frame is therefore always read between turns, on the old runtime and on the new one.
- **`run`** keeps its signature. Its body: serve at seq 7, `let Served::Start { step_id, task } … else panic`, then
  `(step_id, end_after_turn(runtime, backend, (tx, rx), step_id, task).await)`. Rewrite the doc comment (the
  "queued **before** the future is polled" paragraph is now false).
- **`run_with_cancel_queued`**: today's `run` body verbatim, with a doc saying it pins a cancel read before the first
  pull (MOD-55 A-2). Used by `:6974` only.

### 1.3 `attach_and_end`, `live_steps_drops_an_ended_chat`, `chat_secrets::end`

- `attach_and_end`: keep the signature and `between(step_id).await` (it must still run before the task is first
  polled), then `end_after_turn`. Its doc changes from "queues the user's `Esc Esc` before the session is polled" to
  "after its first turn".
- `live_steps_drops_an_ended_chat`: the `live_steps() == vec![step_id]` assertion stays before
  `end_after_turn(…).await`. The `caps(step_id).is_some()` assertion still holds, because the cancel's `serve` runs
  while the chat is live, so nothing is swept.
- `chat_secrets`: delete `end` and add `end_after_turn as end` to the `use super::{…}` list. The two call sites
  (`:15685`, `:15753`) already pass `(runtime, backend, (tx, rx), step_id, task)`.

### 1.4 Hosted `tool_leases::{Live, live, end}` (A-3)

- `Live` gains `rows_before: usize`.
- `live(served, replies, host, slot, backend: &Backend)`: **before** `tokio::spawn(task)`, set
  `rows_before = backend.writer().expect(…).step_events(step_id).await.expect(…).map_or(0, |rows| rows.len())`.
  That is 0 for a fresh chat and the tail for a promoted step.
- `end`: before serving the cancel, poll every 5 ms for at most 20 s (the slot poll's shape, `:8040-8047`) until
  `rows.len() > live.rows_before && rows.last().is_some_and(|row| row.kind == EventKind::Done)`. The rest is
  unchanged. Once the `done` row is stored, the `Done` envelope has been pulled, so the turn reads no more commands.
- The call sites `:8109, 8172, 8238, 8249, 8342, 8396, 8421` add `&backend`. Import `htui_core::model::EventKind`
  at module level.

**T1 gate:** `cargo test -p htui --all-features --lib agent_worker -- --test-threads=1`, green on the unchanged
runtime. Commit `test(mod-87): chat helpers cancel after the turn`.

## 2. T2: MOD-87, a chat listens while it pulls (D1-D4)

### 2.1 `run_turn` (`:4933`)

- The parked branch (`:4947-5021`) is **unchanged**. A `Send` while parked is still "answer the permission request
  first", and `None` is still `Ok(TurnEnd::Cancelled)`.
- Replace `let pulled = if listen { … } else { session.next_event().await };` (`:5023-5038`) with the unconditional
  form:

```rust
let pulled = tokio::select! {
    biased;
    command = commands.recv() => {
        match turn_command(help, deferred, session, recorder, ui, frames, grace, command).await {
            Some(end) => return Ok(end),
            None => continue,
        }
    }
    pulled = session.next_event() => pulled,
};
```

- Doc (`:4924-4928`): drop "A chat passes `false` and reads commands between turns, as it always has". Say that
  commands are served while pulling in every mode (MOD-55 H1 for help, MOD-87 D1 for a chat), that `help` only
  decides what a mid-turn `Send` gets, and that `deferred` collects a chat's mid-turn follow-ups.

### 2.2 `turn_command` (D1)

```rust
let reply = match command {
    Some(ChatCommand::Cancel { reply }) => reply,
    None => None,
    Some(ChatCommand::Send { reply, .. }) if help => { /* refuse "a help turn takes no follow-up", as today */ return None; }
    // MOD-87 D1: the tab's type-ahead (`chat/mod.rs:264-275`); served after the turn, in order.
    Some(command @ ChatCommand::Send { .. }) => { deferred.push_back(command); return None; }
    Some(ChatCommand::Answer { request_id, reply, .. }) => { /* refuse with the id, as today */ return None; }
};
```

The tail is today's: a failed graceful cancel is logged at `debug` ("the session did not cancel gracefully", with
"help" dropped from the sentence), then `drain`, then `Ended{Cancelled}` at `reply` when there is one, then
`Some(TurnEnd::Cancelled)`. The doc is generalized: "a command read while pulling, nothing parked: help or chat".

### 2.3 `answer_queued` (D3)

```rust
commands.close();
let mut answer = |command: ChatCommand| { /* today's per-command match: Cancel{None} skipped, Cancel{Some} -> Ended{last_stop},
                                            Send -> ended("chat_send"), Answer -> ended("chat_answer") */ };
for command in deferred.drain(..) { answer(command); }
while let Some(command) = commands.recv().await { answer(command); }
```

The deferred commands go first: they arrived before anything still in the channel. A plain `fn answer_one(frames,
command, last_stop)` also works if the closure fights the borrow checker. The doc says it serves help (MOD-55 H1) and
chat (MOD-87 D3), and states the placement rule in 2.5.

### 2.4 `run_chat`'s loop (D2, A-1)

- `let mut deferred: VecDeque<ChatCommand> = VecDeque::new();` sits before the loop. The loop is labeled `'chat: loop`.
- The `run_turn` call gains `&mut deferred` and passes `mode.is_help()` as `help`.
- The help one-turn block (`:4816-4822`) is unchanged.
- Replace `match commands.recv().await { … }` (`:4830-4890`) with an inner wait that leaves **only** to start a turn:

```rust
let (text, reply) = loop {
    let command = match deferred.pop_front() {
        Some(command) => Some(command),
        None => commands.recv().await,
    };
    match command {
        Some(ChatCommand::Send { text, reply }) => break (text, reply), // T3 puts the scrub here
        Some(ChatCommand::Answer { reply, .. }) => { /* refuse as today (:4858-4865) */ }   // stays between turns (A-1)
        Some(ChatCommand::Cancel { reply }) => { /* today's arm (:4870-4883) */ break 'chat; }
        None => { /* today's arm (:4885-4888) */ break 'chat; }
    }
};
// Today's Send body (:4832-4855), unchanged: send_follow_up, on Err reply Failed + status Failed + frames.failed + `break 'chat`,
// record_follow_up, the reply frame once at `reply`.
```

### 2.5 `run_chat`'s tail (D3)

```rust
// MOD-55 review H1: a help answers what is still queued before its recorder closes (its pinned frame order).
if mode.is_help() {
    answer_queued(&mut deferred, &mut commands, &frames, last_stop).await;
}
if let Err(err) = recorder.finish().await { … unchanged … }
binding.close(&writer, status).await;
frames.ended(last_stop);
// MOD-87 D3: a conversation answers what is still queued **last**. Blueprint D206 reads a closed receiver
// as a returned task (`live_steps` :954-960, `bind_promoted` :1015-1027), so it may close only once the
// run is closed and the stream has ended.
if !mode.is_help() {
    answer_queued(&mut deferred, &mut commands, &frames, last_stop).await;
}
```

Replace the comment at `:4893-4894` ("A chat keeps its behaviour").

### 2.6 T2 test fixtures

- **`StallAt::AfterChunkUntilReleased`** (A-5): "the pull after the first reply chunk waits for
  `StallProbe::release`, once. Each pull that re-enters the stall notifies `reached` again, so a case can tell a
  command was served mid-turn." `StallSession` gains `released: bool`. Its `next_event` becomes:
  `if AfterChunkUntilReleased && chunk_seen && !released { reached.notify_one(); release.notified().await; released = true; }`.
  The `Notified` future is dropped when a command wins the `select`, and the next pull re-registers it, so a case
  must release only **after** its second `reached` wait. At that point the waiter is registered, and no notification
  rides on a dropped future.
- **`at_stall(runtime, backend, (tx, rx), served: Served, probe, then: impl FnOnce(StepId) -> Vec<RequestEnvelope>) -> (StepId, Vec<ReplyEnvelope>)`**:
  spawn the task, wait for `reached` (30 s), serve `then(step_id)` in order (each `Deferred`), `release.notify_one()`,
  await the task (5 s), and collect. `cancel_at_stall(runtime, backend, request: StoreRequest, probe)` serves
  `request` at 7, then calls `at_stall(…, |step| vec![envelope(8, ChatCancel { step_id: step })])`. The two existing
  callers (`:6926`, `:7017`) pass `help(agent_id, "old\n", "new")`. The panic text becomes "a session opens".

### 2.7 T2 tests (red first; `at_seq` is `:6896`; "stream" means `at_seq(&replies, 7)`)

Conversation tests must assert the stream's end on **seq 7**, not `replies.last()`: D3 now answers queued commands
after the stream's `Ended`.

1. `a_cancel_mid_stream_with_nothing_parked_cancels_the_chat`. Mirrors `:6912`, with `start(agent_id, "hello")`,
   `fixture_with_stall(one_turn([chunk, ExpectCancel]), AfterChunk)` and `cancel_at_stall`.
   - `at_seq 8 == [Ended{Cancelled}]`.
   - `probe.cancelled`.
   - The stream's last reply is `Ended{Cancelled}`.
   - `run.status == Cancelled`.
   - The log's last row is `done` with `stop_reason == "cancelled"`.
2. `a_second_cancel_behind_a_cancelled_turn_is_answered_once`. Same fixture; `at_stall` with
   `then = [cancel@8, cancel@9]`.
   - `at_seq 8 == [Ended{Cancelled}]` (`turn_command`).
   - `at_seq 9 == [Ended{Cancelled}]` (`answer_queued`, `last_stop == Cancelled`).
   - The stream ends `Cancelled`.
   - Old runtime: seq 9 is missing.
3. `a_follow_up_sent_mid_turn_is_sent_after_the_turn`. `AfterChunkUntilReleased`,
   `Script::turns([[chunk("first"), ends(EndTurn)], [chunk("second"), ends(EndTurn)]])`. Spawn the start, wait for
   `reached`, serve `ChatSend{"next"}@8` (`Deferred`), wait for `reached` again, release, then
   `converse(…, async { handle.await.expect(..) }, vec![vec![], vec![cancel@9]])`.
   - `at_seq 8 == [Chat(Event(follow_up "next"))]`.
   - No `Failed` anywhere. The fake errors on a follow-up before `done` (fake rule 5, `fake.rs:551-556`), so success
     proves the order.
   - The log kinds run `Prompt, Other, AssistantText, Done, FollowUp, AssistantText, Done`, and the follow-up row is
     on turn 1.
   - `at_seq 9 == [Ended{EndTurn}]`.
   - `run.status == Done`.
4. `an_answer_mid_turn_with_nothing_parked_is_refused_and_the_turn_goes_on`. As in 3, with one turn,
   `ChatAnswer{request_id:"req-x", Selected("allow")}@8`, and batches `[[cancel@9]]`.
   - `at_seq 8 == [Failed{"chat_answer", "no permission request `req-x` is waiting"}]`.
   - `run.status == Done`.
   - `at_seq 9 == [Ended{EndTurn}]`.
5. `a_stale_answer_between_turns_is_refused_and_the_chat_goes_on` (A-1). `fixture(Script::turns([[ends],[ends]]))`,
   serve the start, then `converse` with `[[answer("req-x")@8, send("next")@9], [cancel@10]]`.
   - `at_seq 8 == [Failed{"chat_answer", "no permission request is waiting"}]`.
   - `at_seq 9` is the follow-up frame.
   - There is no `ChatFrame::Failed`.
   - `run.status == Done`.
   - Old runtime: the chat fails `Closed`.
6. `a_promoted_chat_cancelled_mid_turn_leaves_the_step_to_the_engine` (D4). `fixture_with_stall(one_turn([chunk,
   ExpectCancel]), AfterChunk)`; `at_stall` over `attach_promoted(promote_addr(), promoted(agent_id, handoff path))`
   with `then = [cancel@8]`.
   - `at_seq 8 == [Ended{Cancelled}]`.
   - The stream ends `Cancelled`.
   - `RUN_1`'s and `STEP_PLAN`'s status and `finished_at` are unchanged (mirror `a_promoted_chat_never_closes_a_run`,
     `:7925-7940`).
   - `active_runs` is unchanged.
   - The log is the tail plus `follow_up`, banner, `assistant_text`, `done{cancelled}`.
7. `live_steps_keeps_a_chat_until_its_run_is_closed` (D3/D206). `fixture_with_stall(one_turn([ends(EndTurn)]),
   InCancel)`, chat start, task spawned. Collect `rx` until a `Done` frame, then serve `cancel@8` (the between-turns
   `session.cancel` stalls and notifies `reached`). Wait for `reached`.
   - `assert_eq!(runtime.live_steps(), vec![step_id])`.
   - Serve `ChatSend{"late"}@9` and assert `Deferred`.
   - Release. Then `while !handle.is_finished() { if runtime.live_steps().is_empty() { assert run status is Done } yield_now }`.
     This is a weak ordering pin on a current-thread runtime; the D3 placement is the review-level guarantee.
   - Collect the rest.
   - `at_seq 8 == [Ended{EndTurn}]`.
   - `at_seq 9 == [Failed{"chat_send", "this chat has ended"}]`. Old runtime: missing.
   - `live_steps()` is empty and `run.status == Done`.
8. Existing help tests stay green unchanged, including `a_cancel_queued_as_the_help_turn_ends_is_answered_once`.

**T2 gate:** the `--lib` gate, plus `cargo clippy -p htui --all-features -- -D warnings`. Commit
`fix(mod-87): a chat serves commands while it pulls`.

## 3. T3: MOD-86, scrub before sending (D5-D9)

### 3.1 `AgentRuntime::start`, `Opening::Chat` (D6; `:2296`)

```rust
Opening::Chat(text) => {
    // MOD-86 D6: the chat's own construction (`run_chat`), so its second pass over the same env is
    // deterministic; the short keys are `run_chat`'s to log. Before the box, the registry row, the
    // lease and the run: a refusal leaves nothing and reaches no driver.
    let (scrubber, _short) = MinimalScrubber::from_resolved(&env);
    match scrub_section(&scrubber, &text, "prompt") {
        Ok(masked) => (masked, ChatMode::Conversation),
        Err(refused) => {
            return Ok(Served::Reply(StoreReply::Failed {
                request: "chat_start",
                message: format!("not sent: {refused}"),
            }));
        }
    }
}
```

The offline refusal stays first: the `writer` check at `:2284` comes before the opening. Also update the
`Opening::Chat` doc (`:2773`) to: "`ChatStart`'s text, scrubbed here before anything is minted, then again with the
resolved scrubber in `run_chat` (MOD-86 D6, D7)."

### 3.2 `run_chat`'s prelude (D7). The order is resolve, then the scrubber, then the opening scrub, then the deferred run

```rust
// MOD-86 D7: read before `secrets` is consumed. The opening's refusal arm closes a run only if one exists.
let run_deferred = secrets.is_some() && matches!(binding, ChatBinding::Fresh(..));
if let Some(ChatSecrets { source, project }) = secrets {
    match resolve_project(…).await {
        Ok(resolved) => spec.env = …,                 // unchanged
        Err(cause) => { /* unchanged D13 arm */ return; }
    }
}                                                     // start_chat_run moves out of this block
let (scrubber, short) = MinimalScrubber::from_resolved(&spec.env);
if !short.is_empty() { tracing::warn!(…); }           // unchanged
// MOD-86 D7, D9: the text the driver opens with, scrubbed with the resolved scrubber before anything starts.
// Every mode (A-4).
let section = match &binding { ChatBinding::Fresh(..) => "prompt", ChatBinding::Promoted { .. } => "opening" };
let prompt = match scrub_section(&scrubber, &prompt, section) {
    Ok(masked) => masked,
    Err(refused) => {
        let message = format!("not sent: {refused}");
        frames.to_stream(StoreReply::Failed { request: mode.request(&binding), message: message.clone() });
        if !run_deferred {
            binding.close(&writer, RunStatus::Failed).await;
        }
        frames.failed(message);
        return;
    }
};
if run_deferred && let ChatBinding::Fresh(chat, closed) = &binding {
    /* today's start_chat_run arm (:4590-4599), verbatim, including closed.store(false) */
}
```

The refusal arms, per binding:

| Binding | Run state at the refusal | Frames, in order | Run write |
|---|---|---|---|
| Fresh, provider (`run_deferred`) | not written; the `closed` flag is still raised (H-11), so a panic answer touches nothing | `Failed{chat_start \| edit_help}`, `ChatFrame::Failed` (D13's shape) | none |
| Fresh, non-provider | written by `start` | `Failed{…}`, then `binding.close(Failed)`, then `ChatFrame::Failed` (the start-failure arm's shape, no notice) | closed `failed` |
| Promoted (provider or not) | the engine's | `Failed{promote_step}`, a no-op close, `ChatFrame::Failed` | none (`close` is a no-op, D205) |

The fresh non-provider arm cannot be reached in practice: `start` passed the same text through the same constructor
over the same env, and masking is idempotent. It is handled anyway, and it is untested. No arm records
`record_failure`, which matches the start-failure arm. Every one of these arms runs **before** `frames.accept`, so the
tab cannot yet hold a session to send commands to (the tab's `session` is set on `ChatAccepted`).

### 3.3 The fallback handoff and the `started` match (D7; `:4649-4723`)

- The `started` value's error type becomes `(StartFailure, Option<DriverEnvelope>)`. `(Err(err), _)` becomes
  `Err((StartFailure::Driver(err), None))`.
- The `falls_back` arm keeps `record_opening(ResumeFailed)` and the notice row first (H-7: the resume *did* fail).
  Then:

```rust
let handoff_spec = SessionSpec { resume: None, ..spec };
match scrub_section(&scrubber, &fallback.handoff, "handoff") {
    Err(refused) => Err((StartFailure::Refused(refused), envelope)),
    Ok(handoff) => match driver.start(handoff_spec, handoff.clone()).await {
        Ok(session) => Ok((session, handoff, StepOpening::ResumeFailed, envelope)),
        Err(err) => Err((StartFailure::Driver(err), envelope)),
    },
}
```

- The start-failure arm becomes `Err((failure, notice))`, with `let message = failure.message();`. The notice frame
  still goes first. The re-probe test becomes `if let (StartFailure::Driver(DriverError::Spawn(_)), Some(reprobe)) =
  (&failure, reprobe)`.
- The first `driver.start(spec.clone(), prompt.clone())` (`:4641`) now sends the masked `prompt`. From there,
  `opening_text` (the `prompt` row and local frame at `:4747-4750`, the promoted `follow_up` row and frame at
  `:4755-4758`) carries the masked copy with no further change.

### 3.4 The between-turns `Send` (D8)

In 2.4's inner loop:

```rust
Some(ChatCommand::Send { text, reply }) => match scrub_section(&scrubber, &text, "follow-up") {
    Ok(masked) => break (masked, reply),
    Err(refused) => frames.reply(&reply, StoreReply::Failed {
        request: "chat_send",
        message: format!("not sent: {refused}"),
    }),                                   // D5: nothing sent, recorded or framed; keep waiting
},
```

Deferred mid-turn sends pass through the same arm when they are popped.

### 3.5 T3 fixtures (A-6)

- `type SendLog = Arc<Mutex<Vec<String>>>;` and `struct SendSpy { inner: Box<dyn AgentSession>, sends: SendLog }`.
  `SendSpy` implements `AgentSession` by delegating, as `StallSession` does (`:6748-6806`). `send_follow_up` pushes
  `text.clone()` (with the lock released before the delegate's future), then delegates.
- `FailingStarts` and `FailingBuilder` gain `sends: SendLog`. `FailingStarts::start` returns
  `Box::pin(async move { let inner = started.await?; Ok(Box::new(SendSpy { inner, sends }) as Box<dyn AgentSession>) })`,
  where `started` is `self.inner.start(spec, prompt)` after the log push and the failure check.
- `fixture_with_send_log(script) -> (MemStore, Backend, AgentRuntime, AgentId, StartLog, SendLog)` is the body.
  `fixture_with_failing_starts` keeps its signature and drops the log. Add `fn sends_of(&SendLog) -> Vec<String>`.

### 3.6 T3 tests (red first)

`KEY` is `format!("ghp_{}", "A1b2".repeat(9))`, which trips the `github_token` rule, as at `:6279`. `SECRET` is
`"hunter2-secret-value"` through `with_session_env` (`:847`), as at `:6528`.

1. `a_chat_prompt_with_a_credential_is_refused_before_anything_is_minted`. Mirrors `:6278` with
   `start(agent_id, &format!("use {KEY}"))` over `fixture_with_spec_spy`.
   - `Served::Reply(Failed{"chat_start", "not sent: the prompt matches the github_token rule"})`.
   - The message has no `ghp_`.
   - `runtime.steps()` and `runtime.live_steps()` are empty.
   - `active_runs` is unchanged.
   - The slot is `None`.
   - The channel is empty after `drop(tx)`.
2. `the_chat_driver_is_sent_the_masked_prompt`. `fixture_with_failing_starts(one_turn([ends]), vec![])` plus
   `SECRET`; `run(start(agent_id, &format!("use {SECRET}")))`.
   - `starts_of[0].1 == "use [REDACTED]"`.
   - The `prompt` row's `payload.text` is the same.
   - The seq-7 local frame (`Event(Other{update:"prompt", body.text})`) is the same.
   - `!format!("{log:?}{replies:?}").contains(SECRET)`.
3. `a_follow_up_is_masked_before_it_is_sent`. `fixture_with_send_log(Script::turns([[ends],[ends]]))` plus `SECRET`;
   serve the start, then `converse` with `[[send(format!("use {SECRET}"))@8], [cancel@9]]`.
   - `sends_of == ["use [REDACTED]"]`.
   - The one `FollowUp` row's text is the same.
   - `at_seq 8` is that follow-up frame.
   - Nothing contains `SECRET`.
4. `a_refused_follow_up_is_not_sent_and_the_chat_goes_on`. As in 3, with `[[send(format!("use {KEY}"))@8,
   send("clean")@9], [cancel@10]]`.
   - `at_seq 8 == [Failed{"chat_send", "not sent: the follow-up matches the github_token rule"}]`, with no `ghp_`
     in any reply.
   - `sends_of == ["clean"]`.
   - Exactly one `FollowUp` row, and its text is `clean`.
   - `at_seq 9` is the `clean` frame.
   - `at_seq 10 == [Ended{EndTurn}]`.
   - `run.status == Done`.
5. `a_promoted_opening_with_a_credential_is_refused`. `fixture_with_failing_starts(one_turn([ends]), vec![])` and
   `attach_and_await(promoted(agent_id, OpeningPath::Handoff{ text: format!("use {KEY}"), digest }))`.
   - The stream is exactly `[Failed{PROMOTE_STEP, "not sent: the opening matches the github_token rule"},
     ChatFrame::Failed{same}]`.
   - `starts_of` is empty.
   - The `STEP_PLAN` log equals the tail.
   - `RUN_1`'s status is unchanged.
6. `a_handoff_with_a_credential_fails_the_chat_after_the_notice`. Mirrors `:7422`, with failures
   `[Transport("gone")]` and a resume path whose fallback text is `format!("use {KEY}")`.
   - `starts_of.len() == 1`.
   - Order: notice frame, then `Failed{PROMOTE_STEP, "not sent: the handoff matches the github_token rule"}`, then
     `ChatFrame::Failed`.
   - The log is the tail plus 1 (the notice), with no `FollowUp`.
   - `opening_of == ResumeFailed`.
7. In `chat_secrets`, using `fixture_with_failing_starts(echo("ok"), vec![])` (import it and `starts_of`) with
   `plant` and `resolving(&[("API_KEY", VALUE)])`:
   - `a_provider_chat_masks_its_resolved_value_before_sending`: `run(start(agent_id, &format!("use {VALUE}")))`.
     `starts_of[0].1 == "use [REDACTED]"`, the `prompt` row and frame are masked, there is no `VALUE` anywhere, and
     `starts_of[0].0.env` is the resolved map.
   - `a_provider_chat_whose_opening_is_refused_writes_no_run` (D7). The prompt is
     `format!("{VALUE}ghp_{}", "A1b2".repeat(9))`. Unmasked, `ghp_` follows `9`, so it is not at a token start and
     `start`'s pass over an empty env accepts it. Masked, it follows `]` and trips `github_token`. Assert that
     precondition first with `scrub_section(&MinimalScrubber::from_resolved(&BTreeMap::new()).0, &prompt, "prompt").is_ok()`.
     Then:
     - `is_refusal(&stream(&replies), "chat_start", "not sent: the prompt matches the github_token rule")`.
     - `starts_of` is empty.
     - `store.run(run_of(step_id))` is `None`.
     - `rows` is empty.
     - `active_runs` is unchanged.
     - `source.calls() == 1`.
   - `a_promoted_opening_is_masked_with_the_resolved_value`: `attach_and_end(promoted(agent_id, Handoff{ text:
     format!("use {VALUE}") }))`. `starts_of[0].1 == "use [REDACTED]"`, and the new `FollowUp` row and its frame are
     masked.
8. Chat tab (`crates/htui/src/ui/tabs/chat/mod.rs` tests),
   `a_refused_follow_up_shows_on_the_hint_and_keeps_the_session_live`. `let mut tab = live(&shell);` (`:861`) and a
   recorded `tab.transcript.len()`. Then `on_reply(Failed{"chat_send", "not sent: the follow-up matches the
   github_token rule"})`.
   - `tab.hint()` equals that sentence.
   - `tab.session().is_some_and(|s| s.ended.is_none())`.
   - The transcript length is unchanged.
   - `shell.emit.take()` is empty (nothing is re-sent).

Docs: the `SectionRefused.section` doc (`htui-core/src/prompt/mod.rs:1328`) becomes:
"`"name"`, `"body"` or `"request"` for a help prompt; `"prompt"`, `"follow-up"`, `"opening"` (a promoted step's) or
`"handoff"` (a failed resume's fallback) for a chat (MOD-86 D9)."

**T3 gate:** the `--lib` gate, `cargo test -p htui-core --all-features prompt`, and clippy. Commit
`fix(mod-86): chat text is scrubbed before it is sent`.

## 4. T4

Run the gates from the plan's Validation. `cargo test -p htui --all-features -- --test-threads=1` also covers
`tests/chat.rs` (`Esc Esc` after `drive`, follow-ups after a quiet drive; the harness polls every chat future once per
round, `testkit.rs:240-265`). Then `--workspace --no-fail-fast`, grepping for `FAILED|SIGABRT|panicked`. Then
`cargo insta test … --check`; no snapshot should change. The close-out documents also record A-1 (the stale-answer
wedge, fixed) and A-4 (help on a provider project, covered if accepted).

## Data flow (after T3)

`ChatStart` takes this path:
1. `start` checks the writer.
2. `scrub_section(from_resolved(env), "prompt")` runs. A refusal answers `Failed{chat_start}` and nothing else exists.
3. Mint, lease, then the run if the project has no provider.
4. `run_chat` resolves the provider's secrets, builds `from_resolved(spec.env)`, and runs `scrub_section(…,
   "prompt" | "opening")`. A refusal takes the arms in 3.2.
5. The deferred `start_chat_run` runs.
6. `driver.start(masked)`. If the resume fails, `scrub_section(handoff, "handoff")` runs before the second start.
7. `accept`, then the opening's row and frame (masked).
8. Turns: `run_turn` selects commands-first. A cancel cuts the turn, an answer is refused, and a send is deferred.
9. Between turns: the deferred sends first, then the channel. `scrub_section(…, "follow-up")` refuses at the send's
   address, or sends, records and frames the masked text.
10. At the end: `finish`, `close`, `ended`, then `answer_queued`, which closes the receiver (D206).

## Hazards

- **H-1 (cancel safety).** The `select!` drops the pending `next_event` future whenever a command wins. Both
  transports' pulls are an mpsc `recv` (the `run_turn` doc, MOD-55 H1), and so are the fake and the stall test
  doubles. A future test double whose `next_event` holds state across an `.await` must be cancel-safe too (the
  `StallSession` variants re-enter cleanly).
- **H-2 (D206 ordering).** In a conversation the receiver must stay open until after `binding.close` **and**
  `frames.ended`, because `live_steps` and `bind_promoted`'s guard and sweep read `is_closed()` as "task returned".
  Do not drop `commands` early on any path that reaches the tail. The early-return start arms (D13, the 3.2 refusals,
  the start failure) still drop it unanswered when they return. That is unchanged, and safe: they precede
  `ChatAccepted`, so the tab has no session to send on. The `Spawn` re-probe arm drops it explicitly (`:4717`).
- **H-3 (one reply per request, at its own address).** `turn_command`'s cancel reply goes to `reply`, and the stream's
  `Ended` comes from `frames.ended`, as the parked path already does. A refused follow-up gets one `Failed` at its
  address and **no** stream frame. A deferred send's success frame goes once, at its own address (`:4849-4855`'s
  rule). Do not also put it on the stream: `App::is_fresh` (`app/state.rs:416`) would accept both and render it twice.
- **H-4 (staleness, pre-existing).** `App::is_fresh` keeps only the newest `seq` per (origin, request kind). With two
  type-ahead sends, the first's reply, whether a frame or a D5 refusal, is stale once the second is dispatched, and the
  tab drops it. That already happens today with the queue. D5's refusal can therefore go unseen if the user sent again
  meanwhile. This is out of scope; record it in the decision write-up.
- **H-5 (`replies.last()`).** In a conversation the last reply can now be a queued command's answer, after the
  stream's `Ended` (D3). New tests assert the stream on seq 7. The existing `.last()` assertions stay valid under the
  Done-wait helpers (1.1).
- **H-6 (shutdown).** `shutdown` sends `Cancel{reply: None}` (`:1465`). A chat mid-turn now cuts the turn and closes
  `cancelled` within `grace` instead of finishing its turn under the `grace * 2` timeout. That follows from D1, and
  the write-up should note it.
- **H-7 (the parked `None`).** The parked path still returns `Cancelled` on a closed channel without cancelling the
  session (`:5020`), while D1's unparked `None` cancels and drains. This is pre-existing; do not unify it here.
- **H-8 (deferred sends and a later park).** A send deferred before a permission request parks stays deferred and goes
  out after the turn. A send that arrives while parked is still refused (the parked path is unchanged). The asymmetry
  is accepted by D1.
- **H-9 (masking marker as a token start).** `[REDACTED]` ends in `]`, which `TOKEN_START` treats as a token boundary
  (`scrub.rs:81`), so masking can expose a credential prefix glued to a secret. That is why D7's second pass can
  refuse where `start`'s did not, and 3.6 #7 depends on it. It is correct, fail-closed behaviour.
- **H-10 (zeroize).** A refused text is dropped, not zeroized, as help's refusal is today. The text never leaves the
  task, and no `Debug` or `Display` of `SectionRefused` carries it.

## Build sequence

1. T1: section 1 (test helpers only; green on the old runtime). Commit.
2. T2: section 0's `run_turn`, `turn_command` and `answer_queued`, plus 2.1-2.5, then tests 2.6-2.7 (write them red,
   then implement). Commit.
3. T3: section 0's `StartFailure` and imports, plus 3.1-3.4, fixtures 3.5, tests 3.6, and the two doc edits. Commit.
4. T4: the gates and the close-out documents (`docs/decisions/mod/mod-86.md`, `mod-87.md`, `HANDOFF.md`,
   `DECISIONS.md`), with A-1, A-4 and H-4/H-6 recorded.
