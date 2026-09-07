//! Recorder unit tests (plan MOD-2 T4, design row D6).
//!
//! Every case drives a scripted [`DriverEnvelope`] vector through a [`Recorder`] into a
//! `MemStore::demo()` and asserts on the **persisted rows** read back through
//! [`ReadStore::step_events`]. No driver, no process and no transport is involved: the recorder is
//! the unit under test, and `docs/ANA-4.md` §11 criteria 2, 3 and 4 are statements about the
//! store, not about a `Vec` in memory.
//!
//! `run_step.usage` and `run_step.prompt_digest` have no reader on the `ReadStore` /`WriteStore`
//! seam (`RunStepSummary` carries neither, and a chat run's `item_id` is `NULL`, so
//! `ReadStore::runs` cannot reach the step at all). [`SpyStore`] therefore wraps `MemStore` and
//! records the [`WriteStore::set_step_usage`] calls the recorder makes, which is the recorder's
//! half of criterion 3; that `set_step_usage` then lands in `run_step` is T2's conformance case
//! `set_step_usage_writes_usage_and_digest`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_agent::driver::{AgentSession, AgentSessionRef, DriverFuture, PermissionAnswer};
use htui_agent::error::DriverError;
use htui_agent::event::{
    DoneEvent, DriverEnvelope, DriverEvent, EditProposalEvent, OtherEvent, PermissionOption,
    PermissionOptionKind, PermissionRequestEvent, StopReason, TextChunk, ToolCallEvent, ToolKind,
    ToolResultEvent, ToolResultStatus, UsageEvent,
};
use htui_agent::record::{AnsweredBy, CHUNK_FLUSH_BYTES, RecordError, Recorder, pump};
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, ChatRunSpec, DocumentHead, EventKind, EventRole, Item, ItemFilter, ItemId,
    ItemPatch, ItemSummary, LinkGraph, NewItem, Note, RunId, RunStatus, RunSummary, Scope,
    SessionEvent, Status, StepId,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{
    MemStore, ReadStore, Result as StoreResult, StoreError, UpdateOutcome, WriteStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// The one secret every case masks: the value a `SessionSpec.env` entry would carry.
const SECRET: &str = "fake-secret-9f8e7d";

/// A fixed capture time, so a persisted row is a function of the script and of nothing else.
fn at() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the demo epoch is a valid instant")
}

/// The scrubber every case uses: `MinimalScrubber` over the one env value.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([SECRET.to_owned()])
}

/// An envelope with no `raw`.
fn env(event: DriverEvent) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: None,
        at: at(),
    }
}

/// An envelope carrying a verbatim wire message.
fn raw_env(event: DriverEvent, raw: Value) -> DriverEnvelope {
    DriverEnvelope {
        event,
        raw: Some(raw),
        at: at(),
    }
}

/// One assistant chunk in a named message group.
fn chunk(text: &str, message_id: &str) -> DriverEnvelope {
    env(DriverEvent::AssistantChunk(TextChunk {
        text: text.to_owned(),
        message_id: Some(message_id.to_owned()),
    }))
}

/// A minimal tool call.
fn tool_call(id: &str) -> DriverEnvelope {
    env(DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: id.to_owned(),
        title: "read a file".to_owned(),
        tool_kind: ToolKind::Read,
        input: json!({ "path": "README.md" }),
        locations: Vec::new(),
    }))
}

/// A tool result whose output is whatever the case wants scrubbed.
fn tool_result(id: &str, output: Value) -> DriverEnvelope {
    env(DriverEvent::ToolResult(ToolResultEvent {
        tool_call_id: id.to_owned(),
        status: ToolResultStatus::Completed,
        output: Some(output),
        locations: Vec::new(),
        terminal_reason: None,
    }))
}

/// The `run` / `run_step` pair every case records into. Minted once so two stores can be given
/// the *same* step id, which is what makes the criterion 2 replay comparison meaningful.
fn chat_spec() -> ChatRunSpec {
    ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    )
}

/// A demo store with the chat's two rows already minted.
async fn open_chat(chat: &ChatRunSpec) -> SpyStore {
    let store = SpyStore::demo();
    store
        .start_chat_run(chat)
        .await
        .expect("the chat run and step must mint");
    store
}

/// The persisted log of a step, in `seq` order.
async fn rows(store: &SpyStore, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("reading the log must not fail")
        .expect("the chat step has a log")
}

/// `payload.text` of a row.
fn text_of(row: &SessionEvent) -> &str {
    row.payload
        .get("text")
        .and_then(Value::as_str)
        .expect("the row carries a `text` key")
}

// ---------------------------------------------------------------------------------------------
// `SpyStore`: `MemStore` plus a log of the `set_step_usage` calls
// ---------------------------------------------------------------------------------------------

/// One `set_step_usage` call the recorder made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageCall {
    /// The `usage` document written to `run_step.usage`.
    usage: Value,
    /// The digest argument: `Some` exactly once per step (plan D15(b), X8).
    prompt_digest: Option<String>,
}

/// A `WriteStore` that delegates everything to `MemStore`, remembers the `set_step_usage` calls
/// the read seam does not expose, and can be told to refuse the next few appends.
#[derive(Debug)]
struct SpyStore {
    inner: MemStore,
    usage_calls: Mutex<Vec<UsageCall>>,
    /// Appends still to be refused before one is let through again.
    refuse_appends: Mutex<usize>,
}

impl SpyStore {
    fn demo() -> Self {
        Self {
            inner: MemStore::demo(),
            usage_calls: Mutex::new(Vec::new()),
            refuse_appends: Mutex::new(0),
        }
    }

    /// Every `set_step_usage` call so far, in order.
    fn usage_calls(&self) -> Vec<UsageCall> {
        self.usage_calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }

    /// Makes the next `count` `append_events` calls fail with the one store error that means
    /// *retrying may work*, writing nothing.
    fn refuse_next_appends(&self, count: usize) {
        *self
            .refuse_appends
            .lock()
            .expect("the spy log is never poisoned") = count;
    }

    /// Whether this append is one of the refused ones, consuming it if so.
    fn refuses_this_append(&self) -> bool {
        let mut left = self
            .refuse_appends
            .lock()
            .expect("the spy log is never poisoned");
        if *left == 0 {
            return false;
        }
        *left -= 1;
        true
    }
}

impl ReadStore for SpyStore {
    async fn items(&self, scope: &Scope, filter: &ItemFilter) -> StoreResult<Vec<ItemSummary>> {
        self.inner.items(scope, filter).await
    }
    async fn item(&self, id: ItemId) -> StoreResult<Option<Item>> {
        self.inner.item(id).await
    }
    async fn links(&self, id: ItemId, hops: u8) -> StoreResult<LinkGraph> {
        self.inner.links(id, hops).await
    }
    async fn documents(&self, id: ItemId) -> StoreResult<Vec<DocumentHead>> {
        self.inner.documents(id).await
    }
    async fn notes(&self, id: ItemId) -> StoreResult<Vec<Note>> {
        self.inner.notes(id).await
    }
    async fn runs(&self, id: ItemId) -> StoreResult<Vec<RunSummary>> {
        self.inner.runs(id).await
    }
    async fn step_events(&self, step: StepId) -> StoreResult<Option<Vec<SessionEvent>>> {
        self.inner.step_events(step).await
    }
}

impl WriteStore for SpyStore {
    async fn mint_item(&self, new: NewItem) -> StoreResult<Item> {
        self.inner.mint_item(new).await
    }
    async fn update_item(
        &self,
        id: ItemId,
        expected_version: i32,
        patch: ItemPatch,
    ) -> StoreResult<UpdateOutcome> {
        self.inner.update_item(id, expected_version, patch).await
    }
    async fn transition(&self, id: ItemId, from: Status, to: Status) -> StoreResult<bool> {
        self.inner.transition(id, from, to).await
    }
    async fn append_events(&self, events: &[SessionEvent]) -> StoreResult<usize> {
        // The guard is dropped before the await: no lock is ever held across one.
        if self.refuses_this_append() {
            return Err(StoreError::Unreachable(
                "the spy store refused this append".to_owned(),
            ));
        }
        self.inner.append_events(events).await
    }
    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> StoreResult<()> {
        self.inner
            .set_step_usage(step, usage.clone(), prompt_digest.clone())
            .await?;
        self.usage_calls
            .lock()
            .expect("the spy log is never poisoned")
            .push(UsageCall {
                usage,
                prompt_digest,
            });
        Ok(())
    }
    async fn upsert_agent(&self, agent: &Agent) -> StoreResult<()> {
        self.inner.upsert_agent(agent).await
    }
    async fn upsert_agent_box(&self, row: &AgentBox) -> StoreResult<()> {
        self.inner.upsert_agent_box(row).await
    }
    async fn start_chat_run(&self, chat: &ChatRunSpec) -> StoreResult<()> {
        self.inner.start_chat_run(chat).await
    }
    async fn finish_chat_run(
        &self,
        run: RunId,
        step: StepId,
        status: RunStatus,
        finished_at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.inner
            .finish_chat_run(run, step, status, finished_at)
            .await
    }
}

// ---------------------------------------------------------------------------------------------
// A scripted `AgentSession`, for `pump` only
// ---------------------------------------------------------------------------------------------

/// Replays a fixed envelope vector through the [`AgentSession`] seam so [`pump`] has something to
/// drive. The real one is T6's `FakeDriver`; this is four stub methods and a queue.
#[derive(Debug)]
struct ScriptedSession {
    events: VecDeque<DriverEnvelope>,
}

impl AgentSession for ScriptedSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        None
    }
    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move { Ok(self.events.pop_front()) })
    }
    fn send_follow_up<'a>(&'a mut self, _text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move { Err(DriverError::Closed) })
    }
    fn answer_permission<'a>(
        &'a mut self,
        _request_id: htui_agent::driver::PermissionRequestId,
        _answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move { Err(DriverError::Closed) })
    }
    fn cancel<'a>(&'a mut self, _grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }
}

// ---------------------------------------------------------------------------------------------
// Criterion 2: coalescing and replay equality
// ---------------------------------------------------------------------------------------------

/// Twenty `assistant_message_chunk` values across two `message_id` groups, interleaved with one
/// `tool_call`, persist as exactly three `assistant_text` rows and one `tool_call` row in `seq`
/// order; replaying the identical vector into a second store yields byte-identical rows
/// (`docs/ANA-4.md` §11 criterion 2).
#[tokio::test]
async fn coalesces_chunks_and_replays_byte_identically() {
    let script: Vec<DriverEnvelope> = (0..10)
        .map(|i| chunk(&format!("a{i} "), "msg-a"))
        .chain(std::iter::once(tool_call("call-1")))
        .chain((10..15).map(|i| chunk(&format!("a{i} "), "msg-a")))
        .chain((15..20).map(|i| chunk(&format!("b{i} "), "msg-b")))
        .collect();
    assert_eq!(
        script
            .iter()
            .filter(|e| matches!(e.event, DriverEvent::AssistantChunk(_)))
            .count(),
        20,
        "the script is the criterion's twenty chunks"
    );

    let chat = chat_spec();
    let scrubber = scrubber();

    let first = open_chat(&chat).await;
    let mut recorder = Recorder::new(&first, &scrubber, chat.step_id, false, None);
    for envelope in script.clone() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");
    let a = rows(&first, chat.step_id).await;

    let kinds: Vec<EventKind> = a.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::AssistantText,
            EventKind::AssistantText
        ],
        "one row per contiguous run, the tool call between them"
    );
    assert_eq!(
        a.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1, 2, 3],
        "seq is gapless from 0 in emission order"
    );
    assert_eq!(text_of(&a[0]), "a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 ");
    assert_eq!(text_of(&a[2]), "a10 a11 a12 a13 a14 ");
    assert_eq!(text_of(&a[3]), "b15 b16 b17 b18 b19 ");
    assert_eq!(
        a[1].tool_call_id.as_deref(),
        Some("call-1"),
        "the tool call id reaches its own column, which idx_session_event_tool joins on"
    );

    let second = open_chat(&chat).await;
    let mut recorder = Recorder::new(&second, &scrubber, chat.step_id, false, None);
    for envelope in script {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");
    let b = rows(&second, chat.step_id).await;

    assert_eq!(
        a, b,
        "the same fixture replayed twice yields identical rows"
    );
    assert_eq!(
        serde_json::to_string(&a).expect("rows serialise"),
        serde_json::to_string(&b).expect("rows serialise"),
        "byte-identical, not merely equal: no wall-clock and no scheduling dependency"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 3: `seq`, `turn` and the prompt digest
// ---------------------------------------------------------------------------------------------

/// A prompt, two follow-ups and their turns: `seq` gapless `0..n`, `turn` `0,1,2`, the `prompt`
/// row at `seq = 0` / `turn = 0` / `role = htui`, and its payload `digest` equal to the digest the
/// recorder wrote through `set_step_usage` (`docs/ANA-4.md` §11 criterion 3).
#[tokio::test]
async fn seq_is_gapless_turns_count_and_digest_reaches_the_step() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record_prompt(
            "summarise the backlog",
            json!([{"name": "task", "tokens": 4, "trimmed": false}]),
            at(),
        )
        .await
        .expect("the prompt row must land");
    recorder
        .record(chunk("first answer", "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("and the risks?", at())
        .await
        .expect("the follow-up row must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("thanks, stop there", at())
        .await
        .expect("the follow-up row must land");
    let summary = recorder
        .finish()
        .await
        .expect("the recorder must close cleanly");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("the log is short")).collect::<Vec<_>>(),
        "seq is gapless 0..n with exactly one writer"
    );
    assert_eq!(log[0].seq, 0, "the prompt is seq 0");
    assert_eq!(log[0].turn, 0, "the prompt is turn 0");
    assert_eq!(log[0].kind, EventKind::Prompt);
    assert_eq!(
        log[0].role,
        EventRole::Htui,
        "`htui` authors the prompt row"
    );

    let turns: Vec<i32> = log
        .iter()
        .filter(|row| matches!(row.kind, EventKind::Prompt | EventKind::FollowUp))
        .map(|row| row.turn)
        .collect();
    assert_eq!(turns, vec![0, 1, 2], "turn increments once per follow-up");
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::FollowUp)
            .map(|row| row.role)
            .collect::<Vec<_>>(),
        vec![EventRole::User, EventRole::User],
        "a follow-up is the user's row"
    );

    let expected = format!("{:x}", Sha256::digest(b"summarise the backlog"));
    let digest = log[0]
        .payload
        .get("digest")
        .and_then(Value::as_str)
        .expect("the prompt payload carries a digest");
    assert_eq!(digest, expected, "sha256 over the assembled prompt text");
    assert_eq!(
        summary.prompt_digest.as_deref(),
        Some(expected.as_str()),
        "the summary reports the same digest"
    );

    let carried: Vec<Option<String>> = store
        .usage_calls()
        .into_iter()
        .map(|call| call.prompt_digest)
        .collect();
    assert_eq!(
        carried.iter().filter(|d| d.is_some()).count(),
        1,
        "the digest is computed once and written once; later usage writes pass None"
    );
    assert_eq!(
        carried.into_iter().flatten().next(),
        Some(expected),
        "run_step.prompt_digest is the payload digest (X8: Some until milestone 9)"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 4: raw retention
// ---------------------------------------------------------------------------------------------

/// `retain_raw = false` leaves every `raw` NULL; the same script with `retain_raw = true`
/// produces the same rows with `raw` populated (`docs/ANA-4.md` §11 criterion 4).
#[tokio::test]
async fn raw_is_null_unless_retained() {
    let script = || {
        vec![
            raw_env(
                DriverEvent::AssistantChunk(TextChunk {
                    text: "hello ".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                json!({ "sessionUpdate": "agent_message_chunk", "n": 1 }),
            ),
            raw_env(
                DriverEvent::AssistantChunk(TextChunk {
                    text: "world".to_owned(),
                    message_id: Some("m1".to_owned()),
                }),
                json!({ "sessionUpdate": "agent_message_chunk", "n": 2 }),
            ),
            raw_env(
                DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::EndTurn,
                }),
                json!({ "stopReason": "end_turn" }),
            ),
        ]
    };

    let chat = chat_spec();
    let scrubber = scrubber();

    let bare = open_chat(&chat).await;
    let mut recorder = Recorder::new(&bare, &scrubber, chat.step_id, false, None);
    for envelope in script() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");
    let without = rows(&bare, chat.step_id).await;

    let kept = open_chat(&chat).await;
    let mut recorder = Recorder::new(&kept, &scrubber, chat.step_id, true, None);
    for envelope in script() {
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");
    let with = rows(&kept, chat.step_id).await;

    assert!(
        without.iter().all(|row| row.raw.is_none()),
        "keep_raw_events = false means every row has raw IS NULL"
    );
    assert!(
        with.iter().all(|row| row.raw.is_some()),
        "keep_raw_events = true populates raw on every row"
    );
    assert_eq!(
        without,
        with.iter()
            .map(|row| SessionEvent {
                raw: None,
                ..row.clone()
            })
            .collect::<Vec<_>>(),
        "flipping the flag changes raw and nothing else"
    );
}

// ---------------------------------------------------------------------------------------------
// The remaining D6 obligations
// ---------------------------------------------------------------------------------------------

/// Trigger 4: a run longer than the 16 KiB bound is cut at the bound rather than buffered
/// unboundedly, and the cut is a function of byte counts alone (no timer, `docs/ANA-4.md` §4.1).
#[tokio::test]
async fn chunks_flush_at_the_16_kib_bound() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let piece = "x".repeat(1024);
    for _ in 0..17 {
        recorder
            .record(chunk(&piece, "m1"))
            .await
            .expect("recording must land");
    }
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 2, "17 KiB of chunks is cut once at the bound");
    assert_eq!(
        text_of(&log[0]).len(),
        CHUNK_FLUSH_BYTES,
        "the open run is flushed as soon as it reaches 16 KiB"
    );
    assert_eq!(
        text_of(&log[1]).len(),
        1024,
        "the remainder opens a new run"
    );
    assert_eq!(log[0].seq, 0);
    assert_eq!(log[1].seq, 1);
}

/// An unmapped protocol update lands as kind `other` with its body verbatim, which is what makes
/// "every event" hold without a schema change per protocol revision.
#[tokio::test]
async fn other_event_keeps_its_verbatim_body() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let body = json!({ "sessionId": "sess-1", "modes": ["default", "plan"], "n": 7 });
    recorder
        .record(env(DriverEvent::Other(OtherEvent {
            update: "session_info_update".to_owned(),
            body: body.clone(),
        })))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].kind, EventKind::Other);
    assert_eq!(log[0].role, EventRole::Agent);
    assert_eq!(
        log[0].payload.get("update").and_then(Value::as_str),
        Some("session_info_update")
    );
    assert_eq!(
        log[0].payload.get("body"),
        Some(&body),
        "the body is stored verbatim"
    );
}

/// `usage` rows are deltas; `run_step.usage` is their sum (`docs/ANA-4.md` §4.1, §7).
#[tokio::test]
async fn usage_deltas_sum_into_step_usage() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    for (input, output, cost) in [(10, 5, 100), (20, 7, 250), (0, 3, 1)] {
        recorder
            .record(env(DriverEvent::Usage(UsageEvent {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cost_micros: Some(cost),
                ..UsageEvent::default()
            })))
            .await
            .expect("recording must land");
    }
    let summary = recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 3, "each usage report is its own row");
    assert!(log.iter().all(|row| row.kind == EventKind::Usage));

    let expected = json!({
        "input_tokens": 30,
        "output_tokens": 15,
        "cache_read_tokens": Value::Null,
        "cache_write_tokens": Value::Null,
        "cost_micros": 351,
    });
    assert_eq!(summary.usage, expected, "the summary carries the sum");
    let last = store
        .usage_calls()
        .pop()
        .expect("the recorder wrote run_step.usage");
    assert_eq!(
        last.usage, expected,
        "run_step.usage is the sum of the step's usage rows"
    );
}

/// A secret from `SessionSpec.env` that reaches a tool result is `[REDACTED]` in the persisted row
/// (`R-SEC-3`, ANA-4 §9: scrub before either write path).
#[tokio::test]
async fn env_values_are_masked_in_persisted_rows() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, true, None);

    recorder
        .record(raw_env(
            DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id: "call-1".to_owned(),
                status: ToolResultStatus::Completed,
                output: Some(json!({ "text": format!("export TOKEN={SECRET} && run") })),
                locations: Vec::new(),
                terminal_reason: None,
            }),
            json!({ "wire": format!("...{SECRET}...") }),
        ))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1);
    let rendered = serde_json::to_string(&log[0]).expect("the row serialises");
    assert!(
        !rendered.contains(SECRET),
        "no persisted row may carry an env value, payload or raw"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "the occurrence is masked, not dropped: {rendered}"
    );
}

/// Fail-closed (`R-SEC-3`, plan D6): a credential the scrubber could not mask blocks that row's
/// write entirely, leaves one `error { code: "scrub_residue" }` in its place, and makes `finish`
/// report `RecordError::Unmasked`. The recorder does **not** touch the step's status: that is the
/// step owner's write (`finish_chat_run` in milestone 3, MOD-4 for graph steps).
#[tokio::test]
async fn scrub_residue_refuses_the_write() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(chunk("before", "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(tool_result(
            "call-1",
            json!({ "output": "leaked sk-ant-api03-abcdefghijklmnopqrstuvwx here" }),
        ))
        .await
        .expect("a refused row is not a recording failure");
    recorder
        .record(chunk("after", "m2"))
        .await
        .expect("recording must land");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "finish reports the residue, got {outcome:?}"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::ToolResult)
            .count(),
        0,
        "the offending row is dropped entirely"
    );
    let errors: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "exactly one scrub_residue row in its place"
    );
    assert_eq!(errors[0].role, EventRole::Htui);
    assert_eq!(
        errors[0].payload.get("code").and_then(Value::as_str),
        Some("scrub_residue")
    );
    let message = errors[0]
        .payload
        .get("message")
        .and_then(Value::as_str)
        .expect("the error row carries a message");
    assert!(
        message.starts_with("anthropic_api_key at "),
        "the message is `<rule> at <path>`, got {message}"
    );
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("sk-ant-"),
        "the offending text never reaches the store, not even through the error row"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("short")).collect::<Vec<_>>(),
        "dropping a row does not open a gap in seq"
    );
}

/// Edit proposals dedupe per `(tool_call_id, path)`: the buffered row is updated before the flush,
/// never written twice (plan D6, `docs/ANA-4.md` §4.3).
///
/// Run under **both** `retain_raw` settings, because the attribution of the update is only
/// observable when `raw` is kept: with `retain_raw = false` a proposal update that landed on the
/// wrong row would look identical to one that landed on the right one. The update owns the row's
/// `diff`, its `accepted`, its capture time and its wire message; the unrelated proposal buffered
/// after it owns none of them.
#[tokio::test]
async fn edit_proposals_dedupe_per_call_and_path() {
    let later = at() + chrono::TimeDelta::seconds(5);
    let proposal =
        |path: &str, diff: &str, accepted: Option<bool>, wire: &str, at| DriverEnvelope {
            event: DriverEvent::EditProposal(EditProposalEvent {
                tool_call_id: Some("call-1".to_owned()),
                path: path.to_owned(),
                diff: diff.to_owned(),
                accepted,
            }),
            raw: Some(json!({ "wire": wire })),
            at,
        };

    for retain_raw in [false, true] {
        let chat = chat_spec();
        let scrubber = scrubber();
        let store = open_chat(&chat).await;
        let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, retain_raw, None);

        recorder
            .record(proposal("src/a.rs", "@@ first", None, "a-first", at()))
            .await
            .expect("recording must land");
        recorder
            .record(proposal("src/b.rs", "@@ other file", None, "b-only", at()))
            .await
            .expect("recording must land");
        recorder
            .record(proposal(
                "src/a.rs",
                "@@ second",
                Some(true),
                "a-second",
                later,
            ))
            .await
            .expect("recording must land");
        recorder.finish().await.expect("close");

        let log = rows(&store, chat.step_id).await;
        assert_eq!(
            log.len(),
            2,
            "one row per (tool_call_id, path), not one per proposal"
        );
        assert_eq!(
            log[0].payload.get("path").and_then(Value::as_str),
            Some("src/a.rs")
        );
        assert_eq!(
            log[0].payload.get("diff").and_then(Value::as_str),
            Some("@@ second"),
            "the buffered row is updated in place"
        );
        assert_eq!(
            log[0].payload.get("accepted").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            log[1].payload.get("path").and_then(Value::as_str),
            Some("src/b.rs")
        );
        assert_eq!(
            log[0].at, later,
            "the deduped row is the update, so it carries the update's capture time"
        );
        assert_eq!(
            log[1].at,
            at(),
            "a row the update did not touch keeps its own capture time"
        );

        if retain_raw {
            assert_eq!(
                log[0].raw,
                Some(json!([{ "wire": "a-first" }, { "wire": "a-second" }])),
                "both wire messages behind the deduped row belong to that row"
            );
            assert_eq!(
                log[1].raw,
                Some(json!({ "wire": "b-only" })),
                "the update's raw may not be charged to whichever row was buffered last"
            );
        } else {
            assert!(
                log.iter().all(|row| row.raw.is_none()),
                "keep_raw_events = false means every row has raw IS NULL"
            );
        }
    }
}

/// A secret split across two chunks only exists once the run is assembled, so the coalesced row
/// is scrubbed again at the flush; masking is idempotent, so the pieces already scrubbed at
/// capture cost a pass and change nothing.
#[tokio::test]
async fn a_secret_split_across_chunks_is_masked_at_the_flush() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let (head, tail) = SECRET.split_at(6);
    recorder
        .record(chunk(&format!("token {head}"), "m1"))
        .await
        .expect("recording must land");
    recorder
        .record(chunk(&format!("{tail} used"), "m1"))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log.len(), 1, "one coalesced row");
    assert_eq!(
        text_of(&log[0]),
        "token [REDACTED] used",
        "the halves are masked once they are one string"
    );
}

/// The same at the fail-closed end: a credential neither chunk carries on its own still blocks the
/// coalesced row's write.
#[tokio::test]
async fn a_credential_split_across_chunks_is_refused_at_the_flush() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(chunk("the key is sk", "m1"))
        .await
        .expect("neither half trips a rule on its own");
    recorder
        .record(chunk("-ant-api03-abcdefghijklmnop", "m1"))
        .await
        .expect("neither half trips a rule on its own");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "the assembled run is refused, got {outcome:?}"
    );
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Error],
        "the coalesced row is dropped and the scrub_residue row takes its seq"
    );
    assert_eq!(log[0].seq, 0, "no gap opens where the dropped row was");
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("sk-ant-"),
        "the offending text never reaches the store"
    );
}

/// A paused chat tab must never stall the agent: the UI channel is bounded and `try_send`, every
/// row still persists, and the drops are counted (`docs/ANA-4.md` §4.1 "Persistence and the UI").
#[tokio::test]
async fn bounded_ui_channel_drops_are_counted() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx));

    for i in 0..8 {
        recorder
            .record(env(DriverEvent::Usage(UsageEvent {
                input_tokens: Some(i),
                ..UsageEvent::default()
            })))
            .await
            .expect("a full UI channel never fails a recording");
    }
    assert_eq!(
        recorder.dropped(),
        7,
        "capacity 1, nothing drained: one frame delivered, the rest counted"
    );
    let summary = recorder.finish().await.expect("close");

    assert_eq!(summary.dropped, 7, "the summary reports the same count");
    assert_eq!(
        rows(&store, chat.step_id).await.len(),
        8,
        "a dropped render frame never costs a row"
    );
}

/// `record_permission_answer` writes the `permission_answer` row of ANA-9 §4.3, with the role the
/// answer's author implies.
#[tokio::test]
async fn permission_answers_carry_their_author() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    let request_id = htui_agent::driver::PermissionRequestId::new("req-1");
    recorder
        .record(env(DriverEvent::PermissionRequest(
            PermissionRequestEvent {
                request_id: request_id.clone(),
                tool_call_id: Some("call-1".to_owned()),
                options: vec![PermissionOption {
                    id: "allow".to_owned(),
                    label: "Allow".to_owned(),
                    kind: PermissionOptionKind::AllowOnce,
                }],
            },
        )))
        .await
        .expect("recording must land");
    recorder
        .record_permission_answer(&request_id, Some("allow"), AnsweredBy::User, false, at())
        .await
        .expect("the answer row must land");
    recorder
        .record_permission_answer(&request_id, None, AnsweredBy::Policy, true, at())
        .await
        .expect("the cancellation row must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::PermissionRequest,
            EventKind::PermissionAnswer,
            EventKind::PermissionAnswer
        ]
    );
    assert_eq!(log[1].role, EventRole::User);
    assert_eq!(
        log[1].payload.get("option_id").and_then(Value::as_str),
        Some("allow")
    );
    assert_eq!(
        log[1].payload.get("by").and_then(Value::as_str),
        Some("user")
    );
    assert_eq!(
        log[2].role,
        EventRole::Htui,
        "a policy answer is htui's row"
    );
    assert_eq!(
        log[2].payload.get("option_id"),
        Some(&Value::Null),
        "a cancelled request has no option"
    );
    assert_eq!(
        log[2].payload.get("cancelled").and_then(Value::as_bool),
        Some(true)
    );
}

/// `pump` drives one turn to its `done` and records everything on the way (the seam MOD-4 and
/// milestone 3 call; T6's `FakeDriver` is what exercises it over a real script).
#[tokio::test]
async fn pump_drives_one_turn_to_done() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    let mut session = ScriptedSession {
        events: VecDeque::from(vec![
            chunk("thinking ", "m1"),
            tool_call("call-1"),
            chunk("done", "m1"),
            env(DriverEvent::Done(DoneEvent {
                stop_reason: StopReason::EndTurn,
            })),
            chunk("never reached", "m2"),
        ]),
    };

    let done = pump(&mut session, &mut recorder)
        .await
        .expect("the turn must reach done");
    assert_eq!(done.stop_reason, StopReason::EndTurn);
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::AssistantText,
            EventKind::Done
        ],
        "pump stops at done and leaves the rest of the queue for the next turn"
    );
    assert_eq!(session.events.len(), 1, "one envelope is left unread");
}

// ---------------------------------------------------------------------------------------------
// A store that fails, a turn that is refused, and a mask that goes too far
// ---------------------------------------------------------------------------------------------

/// A flush the store refuses commits **nothing**: the rows it numbered are still owed, `seq` has
/// not advanced, and the next flush writes them at the numbers they were given. Gapless `seq` with
/// exactly one writer is the guarantee that makes `PRIMARY KEY (run_step_id, seq)` a backstop
/// rather than the allocator, and it has to survive a store that fails once.
#[tokio::test]
async fn a_refused_append_costs_a_retry_and_never_a_seq() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    store.refuse_next_appends(1);
    let outcome = recorder
        .record_prompt("summarise the backlog", json!([]), at())
        .await;
    assert!(
        matches!(outcome, Err(RecordError::Store(_))),
        "the store refused the prompt row, got {outcome:?}"
    );
    assert!(
        store
            .step_events(chat.step_id)
            .await
            .expect("reading the log must not fail")
            .is_none(),
        "a refused append writes no row at all"
    );

    // The caller carries on rather than retrying: the prompt row is owed, not lost.
    recorder
        .record(chunk("an answer", "m1"))
        .await
        .expect("recording must land");
    let summary = recorder
        .finish()
        .await
        .expect("the recorder closes once the store takes the batch");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::Prompt, EventKind::AssistantText],
        "the row the failed flush numbered is written by the next one, in its own order"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1],
        "seq 0 is not spent by a flush that wrote nothing"
    );
    assert_eq!(
        summary.seq,
        i32::try_from(summary.rows).expect("the log is short"),
        "every seq the recorder authored reached the store: rows == seq"
    );
    assert_eq!(
        summary.rows,
        log.len(),
        "the summary counts the stored rows"
    );
}

/// A follow-up the scrubber refuses still opens its turn: the `scrub_residue` row that stands in
/// for it, and every agent row that answers it, carry the new `turn`. Numbering them under the
/// turn that just ended would make `turn` a function of the scrubber (`docs/ANA-4.md` §4.1).
#[tokio::test]
async fn a_refused_follow_up_still_opens_the_next_turn() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record_prompt("summarise the backlog", json!([]), at())
        .await
        .expect("the prompt row must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    recorder
        .record_follow_up("use sk-ant-api03-abcdefghijklmnopqrstuvwx", at())
        .await
        .expect("a refused follow-up is not a recording failure");
    recorder
        .record(chunk("an answer", "m2"))
        .await
        .expect("recording must land");
    recorder
        .record(env(DriverEvent::Done(DoneEvent {
            stop_reason: StopReason::EndTurn,
        })))
        .await
        .expect("recording must land");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "finish still reports the residue, got {outcome:?}"
    );
    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .map(|row| (row.kind, row.turn))
            .collect::<Vec<_>>(),
        vec![
            (EventKind::Prompt, 0),
            (EventKind::Done, 0),
            (EventKind::Error, 1),
            (EventKind::AssistantText, 1),
            (EventKind::Done, 1),
        ],
        "the refused follow-up opens turn 1 all the same"
    );
    assert_eq!(
        log[2].payload.get("code").and_then(Value::as_str),
        Some("scrub_residue"),
        "the row standing in for the follow-up is the residue row"
    );
}

/// An `agent.settings.env` value that happens to equal a closed-vocabulary wire string is masked
/// like any other occurrence, and the masked payload then no longer reads back as its own type.
/// Assumption A7 accepts that false positive **in masking**; it does not accept turning one into a
/// failed turn. The row is still persisted under the kind the unscrubbed event decided, and the
/// session carries on.
#[tokio::test]
async fn a_masked_vocabulary_token_costs_readability_not_the_turn() {
    let chat = chat_spec();
    // `read` is `tool_call.tool_kind`'s wire string as well as a plausible env value.
    let scrubber = MinimalScrubber::new(["read".to_owned()]);
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);

    recorder
        .record(tool_call("call-1"))
        .await
        .expect("a masked vocabulary token is not a recording failure");
    recorder
        .record(chunk("carried on", "m1"))
        .await
        .expect("the turn continues");
    let summary = recorder
        .finish()
        .await
        .expect("a masking false positive never aborts the session");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![EventKind::ToolCall, EventKind::AssistantText],
        "the row keeps the kind the unscrubbed event decided"
    );
    assert_eq!(
        log[0].payload.get("tool_kind").and_then(Value::as_str),
        Some("[REDACTED]"),
        "the false positive costs the value's readability and nothing else"
    );
    assert_eq!(
        log[0].tool_call_id.as_deref(),
        Some("call-1"),
        "the column is taken from the masked document, so it is still filled"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![0, 1],
        "an unreadable payload is a row like any other: no gap, no residue"
    );
    assert_eq!(summary.rows, 2);
}

/// What the render channel actually guarantees (module doc item 4, E2's ruling): it is offered the
/// **capture-time** scrub, one frame per chunk. A secret contained in one chunk never reaches the
/// tab; a secret split across two does, in cleartext, while the persisted row - scrubbed again
/// over the assembled run - carries neither half. Milestone 3 owns the chat tab and any stronger
/// render-path rule; this case exists so that weaker guarantee cannot drift silently.
#[tokio::test]
async fn the_render_channel_gets_capture_time_masking_only() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, Some(tx));

    let (head, tail) = SECRET.split_at(6);
    for text in [
        format!("whole {SECRET} here"),
        format!(" split {head}"),
        format!("{tail} done"),
    ] {
        recorder
            .record(chunk(&text, "m1"))
            .await
            .expect("recording must land");
    }
    let summary = recorder.finish().await.expect("close");
    assert_eq!(summary.dropped, 0, "the channel had room for every frame");

    let mut frames = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let DriverEvent::AssistantChunk(text) = frame.event else {
            panic!("every frame of this script is an assistant chunk");
        };
        frames.push(text.text);
    }
    assert_eq!(
        frames.len(),
        3,
        "one frame per chunk, not one per flushed row"
    );
    assert_eq!(
        frames[0], "whole [REDACTED] here",
        "a secret contained in one chunk is masked before the frame is offered"
    );
    assert!(
        frames[1].contains(head) && frames[2].contains(tail),
        "the halves of a split secret do reach the tab in cleartext: {frames:?}"
    );

    let log = rows(&store, chat.step_id).await;
    assert_eq!(
        text_of(&log[0]),
        "whole [REDACTED] here split [REDACTED] done",
        "the persisted row is masked over the assembled run, which is the stronger guarantee"
    );
}

/// `raw` reaches the persisted row **and** the render frame when a chat tab is attached: the
/// recorder hands the row its own copy only because there is a live channel that still needs the
/// envelope. With no channel the blob is moved instead, which no observer can tell apart - the
/// frame is what would notice, so the frame is what this case checks.
#[tokio::test]
async fn raw_reaches_both_the_row_and_a_live_render_channel() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, true, Some(tx));

    let wire = json!({ "sessionUpdate": "agent_message_chunk", "n": 1 });
    recorder
        .record(raw_env(
            DriverEvent::AssistantChunk(TextChunk {
                text: "hello".to_owned(),
                message_id: Some("m1".to_owned()),
            }),
            wire.clone(),
        ))
        .await
        .expect("recording must land");
    recorder.finish().await.expect("close");

    let log = rows(&store, chat.step_id).await;
    assert_eq!(log[0].raw.as_ref(), Some(&wire), "the row keeps the blob");
    let frame = rx
        .try_recv()
        .expect("the attached tab is offered the frame");
    assert_eq!(
        frame.raw.as_ref(),
        Some(&wire),
        "and so does the frame: the row's copy is a copy, not a move"
    );
}

/// A session that closes without a `done` is a transport failure, not a silent success.
#[tokio::test]
async fn pump_reports_a_session_that_closed_without_done() {
    let chat = chat_spec();
    let scrubber = scrubber();
    let store = open_chat(&chat).await;
    let mut recorder = Recorder::new(&store, &scrubber, chat.step_id, false, None);
    let mut session = ScriptedSession {
        events: VecDeque::from(vec![chunk("half a sentence", "m1")]),
    };

    let outcome = pump(&mut session, &mut recorder).await;
    assert!(
        matches!(outcome, Err(DriverError::Closed)),
        "a closed transport mid-turn is DriverError::Closed, got {outcome:?}"
    );
    recorder.finish().await.expect("close");
    assert_eq!(
        rows(&store, chat.step_id).await.len(),
        1,
        "what did arrive is still recorded"
    );
}
