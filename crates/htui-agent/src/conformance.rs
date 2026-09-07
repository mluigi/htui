//! Transport-neutral conformance suite (feature `test-support`, plan MOD-2 D7).
//!
//! One [`CASES`] list, one [`CaseHarness`] per transport, and **adding a transport adds no case**
//! (`docs/ANA-4.md` §11 criterion 1). No transport is named anywhere in this module: a case owns a
//! [`Script`], the harness turns that script into wire traffic, and every assertion is made on the
//! rows read back through [`ReadStore::step_events`] - which is what makes criteria 2 to 5
//! statements about the store rather than about a `Vec` in memory (D7).
//!
//! Shape copied from the store's suite (`crates/htui-core/src/store/conformance.rs:20-86`): a
//! named `CASES` in run order, a `run_case` that dispatches by name and panics on one it does not
//! know, and a [`run_all`] loop that reports per case.
//!
//! **Every case starts a real session.** [`run_case`] builds a [`SessionSpec`] carrying
//! [`SECRET`] under [`SECRET_ENV_KEY`] and records a `prompt` row, so the scrubber and the digest
//! path are exercised thirteen times rather than twice; the two cases that *assert* on masking are
//! `env_values_masked_in_rows` and `scrub_residue_refuses_write`.
//!
//! **What a harness owes.** Beyond replaying the script, a transport must emit the
//! `session_started` banner, park permission requests until they are answered or cancelled,
//! synthesize a `failed` `tool_result` for every call a rejection or a cancel terminated, and
//! refuse a follow-up before the turn's `done`. `crates/htui-agent/src/fake.rs` documents each of
//! those five rules next to its implementation.

use core::future::Future;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use htui_core::fixtures::ids;
use htui_core::model::{
    Agent, AgentBox, ChatRunSpec, DocumentHead, EventKind, EventRole, Item, ItemFilter, ItemId,
    ItemPatch, ItemSummary, LinkGraph, NewItem, Note, RunId, RunStatus, RunSummary, Scope,
    SessionEvent, Status, StepId,
};
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{ReadStore, Result as StoreResult, UpdateOutcome, WriteStore};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, PermissionAnswer, PermissionPolicy,
    PermissionRequestId, SessionSpec, ToolExposure,
};
use crate::error::DriverError;
use crate::event::{
    DoneEvent, DriverEvent, EditProposalEvent, OtherEvent, PermissionOption, PermissionOptionKind,
    PermissionRequestEvent, StopReason, TerminalReason, TextChunk, ToolCallEvent, ToolKind,
    ToolResultEvent, ToolResultStatus, UsageEvent,
};
use crate::record::{AnsweredBy, CHUNK_FLUSH_BYTES, RecordError, Recorder, pump};

// ---------------------------------------------------------------------------------------------
// The script language
// ---------------------------------------------------------------------------------------------

/// What a transport is asked to put on the wire, one [`Turn`] per prompt or follow-up.
///
/// A closed language on purpose: milestone 3 may **extend** [`ScriptEvent`] additively when a real
/// protocol needs something this cannot express, but criterion 1 forbids a transport-specific
/// *case*, not a richer script (plan risk table).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    /// Turn 0 is played by `AgentDriver::start`; each later turn by one
    /// `AgentSession::send_follow_up`.
    pub turns: Vec<Turn>,
}

impl Script {
    /// A one-turn script.
    #[must_use]
    pub fn one_turn(events: Vec<ScriptEvent>) -> Self {
        Self {
            turns: vec![Turn { events }],
        }
    }

    /// A script of one turn per `Vec<ScriptEvent>`.
    #[must_use]
    pub fn turns(turns: Vec<Vec<ScriptEvent>>) -> Self {
        Self {
            turns: turns.into_iter().map(|events| Turn { events }).collect(),
        }
    }
}

/// One turn: what the agent does between a prompt (or follow-up) and its `done`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    /// The turn's events, in emission order.
    pub events: Vec<ScriptEvent>,
}

/// One step of a [`Turn`]: an event to emit, or a marker for behaviour the transport owns.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptEvent {
    /// Put this event on the wire verbatim.
    ///
    /// A [`DriverEvent::ToolResult`] for a call a rejection or a cancel already terminated is
    /// **skipped**: the transport answered that call itself and a protocol sending its own result
    /// afterwards would be sending a second one.
    Emit(DriverEvent),
    /// Emit this permission request and then stall until it is answered or the session is
    /// cancelled (`docs/ANA-4.md` §4.3: answering every outstanding request is a MUST).
    ParkPermission(PermissionRequestEvent),
    /// This turn has no `done` of its own: only `AgentSession::cancel` ends it. Reaching this
    /// marker while pulling is a transport error, so a case that forgets to cancel fails instead
    /// of hanging.
    ExpectCancel,
}

/// How a transport is built from a [`Script`]. The only thing a binding has to supply.
///
/// `crates/htui-agent/tests/fake_conformance.rs` is milestone 1's implementation; milestone 3's
/// ACP binding is the second, and it adds no case.
pub trait CaseHarness {
    /// A driver that will put `script` on this transport's wire.
    fn driver(&self, script: Script) -> Box<dyn AgentDriver>;
}

// ---------------------------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------------------------

/// Case names in run order. A name never changes: every binding reports per case.
pub const CASES: &[&str] = &[
    "coalesce_across_message_id",
    "seq_gapless_and_turns",
    "raw_iff_retain",
    "cancel_answers_parked_permissions",
    "rejected_tool_gets_failed_result",
    "done_precedes_next_follow_up",
    "usage_deltas_sum_to_step_usage",
    "unknown_update_lands_in_other",
    "edit_proposal_deduped_per_call_and_path",
    "chunk_flush_at_16kib",
    "session_banner_is_first_other_row",
    "env_values_masked_in_rows",
    "scrub_residue_refuses_write",
];

/// Runs one case by name against a transport and a store.
///
/// # Panics
///
/// On the first failed assertion, naming the case, and on an unknown `name`. `()` and not
/// `Result<(), String>` for the reason the store suite gives: the cases are `assert_eq!` chains
/// that carry their own messages, and a binding gets per-case reporting from its loop.
pub async fn run_case<H: CaseHarness, S: WriteStore>(name: &str, harness: &H, store: &S) {
    match name {
        "coalesce_across_message_id" => coalesce_across_message_id(harness, store).await,
        "seq_gapless_and_turns" => seq_gapless_and_turns(harness, store).await,
        "raw_iff_retain" => raw_iff_retain(harness, store).await,
        "cancel_answers_parked_permissions" => {
            cancel_answers_parked_permissions(harness, store).await;
        }
        "rejected_tool_gets_failed_result" => {
            rejected_tool_gets_failed_result(harness, store).await
        }
        "done_precedes_next_follow_up" => done_precedes_next_follow_up(harness, store).await,
        "usage_deltas_sum_to_step_usage" => usage_deltas_sum_to_step_usage(harness, store).await,
        "unknown_update_lands_in_other" => unknown_update_lands_in_other(harness, store).await,
        "edit_proposal_deduped_per_call_and_path" => {
            edit_proposal_deduped_per_call_and_path(harness, store).await;
        }
        "chunk_flush_at_16kib" => chunk_flush_at_16kib(harness, store).await,
        "session_banner_is_first_other_row" => {
            session_banner_is_first_other_row(harness, store).await;
        }
        "env_values_masked_in_rows" => env_values_masked_in_rows(harness, store).await,
        "scrub_residue_refuses_write" => scrub_residue_refuses_write(harness, store).await,
        other => panic!("unknown conformance case `{other}`; CASES and run_case disagree"),
    }
}

/// Runs every case in [`CASES`] against one harness, each against a store `make` produced fresh.
///
/// `make` must return a store loaded with `htui_core::fixtures::DemoData` and nothing else: a case
/// mints its own chat `run` / `run_step` pair and must not see another case's rows.
///
/// # Panics
///
/// On the first failed assertion, naming the case.
pub async fn run_all<H, S, F, Fut>(harness: &H, make: F)
where
    H: CaseHarness,
    S: WriteStore,
    F: Fn() -> Fut,
    Fut: Future<Output = S>,
{
    for name in CASES {
        run_case(name, harness, &make().await).await;
    }
}

// ---------------------------------------------------------------------------------------------
// Fixtures shared by every case
// ---------------------------------------------------------------------------------------------

/// The environment key every session carries.
pub const SECRET_ENV_KEY: &str = "FAKE_TOKEN";

/// The value under [`SECRET_ENV_KEY`]: the one secret every case's scrubber masks.
pub const SECRET: &str = "fake-secret-9f8e7d";

/// The prompt every case opens with. Fixed, so `run_step.prompt_digest` is a constant of the suite.
pub const PROMPT: &str = "summarise the backlog";

/// A credential no secret list can mask, for the fail-closed case.
const RESIDUE: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwx";

/// The suite's fixed instant: `2026-09-06T00:00:00Z`.
///
/// Every `at` in this suite is derived from it, because criterion 2 ("the same fixture replayed
/// twice yields identical rows") is exactly the statement that no row is a function of the wall
/// clock. A transport stamps its *n*-th envelope at `epoch() + n ms`.
///
/// # Panics
///
/// Never: the constant is a valid instant.
#[must_use]
pub fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the suite epoch is a valid instant")
}

/// The scrubber every case records through: `MinimalScrubber` over the session's one env value.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([SECRET.to_owned()])
}

/// The prompt's `sections[]` (ANA-5 §4.7's assembly record); a constant so the payload is one.
fn prompt_sections() -> Value {
    json!([{ "name": "task", "tokens": 4, "trimmed": false }])
}

/// The session inputs every case starts from: a fake secret in `env`, so the scrubber runs whether
/// or not the case asserts on it (D7).
fn session_spec(step: StepId, retain_raw: bool) -> SessionSpec {
    SessionSpec {
        agent_id: ids::AGENT_CLAUDE,
        step_id: step,
        cwd: PathBuf::from("."),
        extra_dirs: Vec::new(),
        env: BTreeMap::from([(SECRET_ENV_KEY.to_owned(), SECRET.to_owned())]),
        model: Some("sonnet".to_owned()),
        tools: ToolExposure::default(),
        mcp: Vec::new(),
        permission: PermissionPolicy::default(),
        retain_raw,
        resume: None,
    }
}

/// Mints a chat `run` / `run_step` pair and starts a session on it.
///
/// Two calls in one case give two independent steps of one store, which is how a case replays a
/// fixture twice without a second store.
async fn open_case<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
    script: Script,
    retain_raw: bool,
) -> (ChatRunSpec, Box<dyn AgentSession>) {
    let chat = ChatRunSpec::mint(
        ids::PROJECT_HTUI,
        ids::BOX,
        ids::USER,
        Some(ids::AGENT_CLAUDE),
        Some("sonnet".to_owned()),
    );
    store
        .start_chat_run(&chat)
        .await
        .expect("the chat run and step must mint");
    let driver = harness.driver(script);
    let session = driver
        .start(session_spec(chat.step_id, retain_raw), PROMPT.to_owned())
        .await
        .expect("the transport must start a session");
    (chat, session)
}

/// The persisted log of a step, in `seq` order.
async fn rows<S: ReadStore>(store: &S, step: StepId) -> Vec<SessionEvent> {
    store
        .step_events(step)
        .await
        .expect("reading the log must not fail")
        .expect("the chat step has a log")
}

/// The kinds of a log, in order.
fn kinds(log: &[SessionEvent]) -> Vec<EventKind> {
    log.iter().map(|row| row.kind).collect()
}

/// `payload.text` of a row.
fn text_of(row: &SessionEvent) -> &str {
    row.payload
        .get("text")
        .and_then(Value::as_str)
        .expect("the row carries a `text` key")
}

/// `payload.<key>` of a row as a string.
fn str_at<'a>(row: &'a SessionEvent, key: &str) -> Option<&'a str> {
    row.payload.get(key).and_then(Value::as_str)
}

/// The rows a *driver* authored, i.e. the log minus the three kinds `htui` writes itself.
fn driver_rows(log: &[SessionEvent]) -> Vec<&SessionEvent> {
    log.iter()
        .filter(|row| {
            !matches!(
                row.kind,
                EventKind::Prompt | EventKind::FollowUp | EventKind::PermissionAnswer
            )
        })
        .collect()
}

/// The log as one JSON string with the run's identity substituted out.
///
/// Two replays of one script are two *steps*, so `run_step_id` and the agent-side session id in the
/// banner differ by construction and by nothing else. Everything else - `seq`, `turn`, `kind`,
/// `role`, `tool_call_id`, `payload`, `raw` and, critically, **`at`** - stays under comparison,
/// which is what makes criterion 2 a statement about wall-clock and scheduling independence rather
/// than a diff with an escape hatch.
fn without_identity(
    log: &[SessionEvent],
    step: StepId,
    session: Option<&AgentSessionRef>,
) -> String {
    let mut text = serde_json::to_string(log).expect("rows serialise");
    // The session id first: it embeds the step id, so substituting the step id first would leave a
    // half-rewritten reference behind.
    if let Some(session) = session {
        text = text.replace(session.as_str(), "<session_ref>");
    }
    text.replace(&step.to_string(), "<step_id>")
}

/// An assistant chunk in a named message group.
fn chunk(text: &str, message_id: &str) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::AssistantChunk(TextChunk {
        text: text.to_owned(),
        message_id: Some(message_id.to_owned()),
    }))
}

/// A `read` tool call.
fn tool_call(id: &str) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::ToolCall(ToolCallEvent {
        tool_call_id: id.to_owned(),
        title: "read a file".to_owned(),
        tool_kind: ToolKind::Read,
        input: json!({ "path": "README.md" }),
        locations: Vec::new(),
    }))
}

/// A completed tool result.
fn tool_result(id: &str, output: Value) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::ToolResult(ToolResultEvent {
        tool_call_id: id.to_owned(),
        status: ToolResultStatus::Completed,
        output: Some(output),
        locations: Vec::new(),
        terminal_reason: None,
    }))
}

/// The turn's `done`.
fn done(stop_reason: StopReason) -> ScriptEvent {
    ScriptEvent::Emit(DriverEvent::Done(DoneEvent { stop_reason }))
}

/// A permission request over `call`, offering one allow and one reject option.
fn park(request: &str, call: &str) -> ScriptEvent {
    ScriptEvent::ParkPermission(PermissionRequestEvent {
        request_id: PermissionRequestId::new(request),
        tool_call_id: Some(call.to_owned()),
        options: vec![
            PermissionOption {
                id: "allow-once".to_owned(),
                label: "Allow".to_owned(),
                kind: PermissionOptionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject-once".to_owned(),
                label: "Reject".to_owned(),
                kind: PermissionOptionKind::RejectOnce,
            },
        ],
    })
}

/// Pulls and records envelopes until the transport emits a `permission_request`, and answers with
/// the request it emitted.
///
/// # Panics
///
/// When the stream ends, errors, or reaches its `done` without one.
async fn pump_to_permission<S: WriteStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
) -> PermissionRequestId {
    loop {
        let envelope = session
            .next_event()
            .await
            .expect("the transport must not fail before its permission request")
            .expect("the transport must reach its permission request");
        let found = match &envelope.event {
            DriverEvent::PermissionRequest(request) => Some(request.request_id.clone()),
            DriverEvent::Done(_) => panic!("the turn ended before its permission request"),
            _ => None,
        };
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
        if let Some(id) = found {
            return id;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// `UsageSpy`: the one column the read seam does not expose
// ---------------------------------------------------------------------------------------------

/// One `set_step_usage` call a recorder made.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageCall {
    usage: Value,
    prompt_digest: Option<String>,
}

/// A [`WriteStore`] that delegates to the case's store and remembers its `set_step_usage` calls.
///
/// `run_step.usage` and `run_step.prompt_digest` are returned by **no** [`ReadStore`] method -
/// `RunStepSummary` carries neither column, and a chat run has `item_id NULL`, so `ReadStore::runs`
/// cannot reach a chat step at all. Adding a read method to make them observable is exactly what
/// plan D15(a) defers to milestone 9, so this suite uses the device T4's `tests/recorder.rs`
/// already uses: wrap the store and watch the write. Generic over `S`, so a milestone 3 binding
/// over `PgStore` gets the same case for free.
struct UsageSpy<'a, S: WriteStore> {
    inner: &'a S,
    calls: Mutex<Vec<UsageCall>>,
}

impl<'a, S: WriteStore> UsageSpy<'a, S> {
    fn new(inner: &'a S) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every `set_step_usage` call so far, in order.
    fn calls(&self) -> Vec<UsageCall> {
        self.calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }
}

impl<S: WriteStore> ReadStore for UsageSpy<'_, S> {
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

impl<S: WriteStore> WriteStore for UsageSpy<'_, S> {
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
        self.inner.append_events(events).await
    }
    async fn set_step_usage(
        &self,
        step: StepId,
        usage: Value,
        prompt_digest: Option<String>,
    ) -> StoreResult<()> {
        // The write first, the log after: the guard is taken and dropped with no `.await` inside
        // its scope.
        self.inner
            .set_step_usage(step, usage.clone(), prompt_digest.clone())
            .await?;
        self.calls
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
// Criterion 2: coalescing and replay equality
// ---------------------------------------------------------------------------------------------

/// The script criterion 2 names: twenty chunks across two `message_id` groups with one `tool_call`
/// in the middle.
fn coalescing_script() -> Script {
    let mut events: Vec<ScriptEvent> = (0..10).map(|i| chunk(&format!("a{i} "), "msg-a")).collect();
    events.push(tool_call("call-1"));
    events.extend((10..15).map(|i| chunk(&format!("a{i} "), "msg-a")));
    events.extend((15..20).map(|i| chunk(&format!("b{i} "), "msg-b")));
    events.push(done(StopReason::EndTurn));
    Script::one_turn(events)
}

/// Twenty chunks across two `message_id` groups, interleaved with one `tool_call`, persist as
/// exactly three `assistant_text` rows and one `tool_call` row in `seq` order; the same script
/// replayed yields an identical row set (`docs/ANA-4.md` §11 criterion 2).
async fn coalesce_across_message_id<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let scrubber = scrubber();

    let (first, mut session) = open_case(harness, store, coalescing_script(), false).await;
    let first_ref = session.session_ref().cloned();
    let mut recorder = Recorder::new(store, &scrubber, first.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("coalesce_across_message_id: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("coalesce_across_message_id: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("coalesce_across_message_id: the recorder must close cleanly");
    let a = rows(store, first.step_id).await;

    assert_eq!(
        kinds(&a),
        vec![
            EventKind::Prompt,
            EventKind::Other,
            EventKind::AssistantText,
            EventKind::ToolCall,
            EventKind::AssistantText,
            EventKind::AssistantText,
            EventKind::Done,
        ],
        "coalesce_across_message_id: one row per contiguous run, the tool call between them"
    );
    assert_eq!(
        a.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..7).collect::<Vec<_>>(),
        "coalesce_across_message_id: seq is gapless from 0 in emission order"
    );
    assert_eq!(text_of(&a[2]), "a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 ");
    assert_eq!(text_of(&a[4]), "a10 a11 a12 a13 a14 ");
    assert_eq!(text_of(&a[5]), "b15 b16 b17 b18 b19 ");
    assert_eq!(
        a[3].tool_call_id.as_deref(),
        Some("call-1"),
        "coalesce_across_message_id: the call id reaches its own column, which \
         idx_session_event_tool joins on"
    );

    let (second, mut session) = open_case(harness, store, coalescing_script(), false).await;
    let second_ref = session.session_ref().cloned();
    let mut recorder = Recorder::new(store, &scrubber, second.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("coalesce_across_message_id: the replay's prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("coalesce_across_message_id: the replay must reach done");
    recorder
        .finish()
        .await
        .expect("coalesce_across_message_id: the replay must close cleanly");
    let b = rows(store, second.step_id).await;

    assert_eq!(
        without_identity(&a, first.step_id, first_ref.as_ref()),
        without_identity(&b, second.step_id, second_ref.as_ref()),
        "coalesce_across_message_id: the same fixture replayed twice yields identical rows - \
         `at` included, so no wall-clock and no scheduling dependency"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 3: `seq`, `turn` and the prompt digest
// ---------------------------------------------------------------------------------------------

/// A prompt and two follow-ups: `seq` gapless `0..n`, `turn` `0,1,2`, the `prompt` row at
/// `seq = 0` / `turn = 0` / `role = htui` carrying the digest of its own text
/// (`docs/ANA-4.md` §11 criterion 3; that the digest also reaches `run_step.prompt_digest` is
/// `usage_deltas_sum_to_step_usage`, which can see the write).
async fn seq_gapless_and_turns<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::turns(vec![
        vec![chunk("first answer", "m1"), done(StopReason::EndTurn)],
        vec![chunk("second answer", "m2"), done(StopReason::EndTurn)],
        vec![chunk("third answer", "m3"), done(StopReason::EndTurn)],
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);

    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("seq_gapless_and_turns: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("seq_gapless_and_turns: turn 0 must reach done");
    for text in ["and the risks?", "thanks, stop there"] {
        recorder
            .record_follow_up(text, epoch())
            .await
            .expect("seq_gapless_and_turns: the follow-up row must land");
        session
            .send_follow_up(text.to_owned())
            .await
            .expect("seq_gapless_and_turns: a follow-up after `done` is accepted");
        pump(session.as_mut(), &mut recorder)
            .await
            .expect("seq_gapless_and_turns: the turn must reach done");
    }
    let summary = recorder
        .finish()
        .await
        .expect("seq_gapless_and_turns: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("the log is short")).collect::<Vec<_>>(),
        "seq_gapless_and_turns: seq is gapless 0..n with exactly one writer"
    );
    assert_eq!(log[0].seq, 0, "seq_gapless_and_turns: the prompt is seq 0");
    assert_eq!(
        log[0].turn, 0,
        "seq_gapless_and_turns: the prompt is turn 0"
    );
    assert_eq!(log[0].kind, EventKind::Prompt);
    assert_eq!(
        log[0].role,
        EventRole::Htui,
        "seq_gapless_and_turns: `htui` authors the prompt row"
    );
    assert_eq!(
        log.iter()
            .filter(|row| matches!(row.kind, EventKind::Prompt | EventKind::FollowUp))
            .map(|row| row.turn)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "seq_gapless_and_turns: turn increments once per follow-up"
    );
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::Done)
            .map(|row| row.turn)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "seq_gapless_and_turns: every row of a turn carries that turn, the closing `done` included"
    );

    let expected = format!("{:x}", Sha256::digest(PROMPT.as_bytes()));
    assert_eq!(
        str_at(&log[0], "digest"),
        Some(expected.as_str()),
        "seq_gapless_and_turns: the prompt payload carries sha256 over the assembled text"
    );
    assert_eq!(
        summary.prompt_digest.as_deref(),
        Some(expected.as_str()),
        "seq_gapless_and_turns: the summary reports the same digest"
    );
    assert_eq!(summary.turns, 3, "seq_gapless_and_turns: three turns");
}

// ---------------------------------------------------------------------------------------------
// Criterion 4: raw retention
// ---------------------------------------------------------------------------------------------

/// `retain_raw = false` leaves every `raw` NULL; the same script with `retain_raw = true` produces
/// the same rows with `raw` populated (`docs/ANA-4.md` §11 criterion 4).
async fn raw_iff_retain<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = || {
        Script::one_turn(vec![
            chunk("hello ", "m1"),
            chunk("world", "m1"),
            tool_call("call-1"),
            tool_result("call-1", json!({ "text": "ok" })),
            done(StopReason::EndTurn),
        ])
    };
    let scrubber = scrubber();

    let mut logs = Vec::new();
    for retain_raw in [false, true] {
        let (chat, mut session) = open_case(harness, store, script(), retain_raw).await;
        let session_ref = session.session_ref().cloned();
        let mut recorder = Recorder::new(store, &scrubber, chat.step_id, retain_raw, None);
        recorder
            .record_prompt(PROMPT, prompt_sections(), epoch())
            .await
            .expect("raw_iff_retain: the prompt row must land");
        pump(session.as_mut(), &mut recorder)
            .await
            .expect("raw_iff_retain: the turn must reach done");
        recorder
            .finish()
            .await
            .expect("raw_iff_retain: the recorder must close cleanly");
        logs.push((chat.step_id, session_ref, rows(store, chat.step_id).await));
    }
    let (bare_step, bare_ref, without) = &logs[0];
    let (kept_step, kept_ref, with) = &logs[1];

    assert!(
        without.iter().all(|row| row.raw.is_none()),
        "raw_iff_retain: keep_raw_events = false means every row has raw IS NULL"
    );
    assert!(
        driver_rows(with).iter().all(|row| row.raw.is_some()),
        "raw_iff_retain: keep_raw_events = true populates raw on every row the driver authored"
    );
    assert!(
        with.iter()
            .filter(|row| row.kind == EventKind::Prompt)
            .all(|row| row.raw.is_none()),
        "raw_iff_retain: a row `htui` authored has no wire message to keep"
    );

    let cleared: Vec<SessionEvent> = with
        .iter()
        .map(|row| SessionEvent {
            raw: None,
            ..row.clone()
        })
        .collect();
    assert_eq!(
        without_identity(without, *bare_step, bare_ref.as_ref()),
        without_identity(&cleared, *kept_step, kept_ref.as_ref()),
        "raw_iff_retain: flipping the flag changes `raw` and nothing else"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 5: cancellation and tool-call terminal states
// ---------------------------------------------------------------------------------------------

/// A cancel answers every parked permission request, writes a `permission_answer` row per parked
/// request, and leaves no `tool_call` without a synthesized `tool_result`
/// (`docs/ANA-4.md` §11 criterion 5, §4.3).
async fn cancel_answers_parked_permissions<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        chunk("about to read ", "m1"),
        tool_call("call-1"),
        park("req-1", "call-1"),
        // The turn has no `done` of its own: the cancel is what ends it.
        ScriptEvent::ExpectCancel,
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("cancel_answers_parked_permissions: the prompt row must land");

    let request = pump_to_permission(session.as_mut(), &mut recorder).await;
    let stalled = session.next_event().await;
    assert!(
        matches!(stalled, Err(DriverError::Transport(_))),
        "cancel_answers_parked_permissions: a parked request stalls the session until it is \
         answered, got {stalled:?}"
    );

    session
        .cancel(Duration::from_millis(0))
        .await
        .expect("cancel_answers_parked_permissions: cancel must succeed");
    recorder
        .record_permission_answer(&request, None, AnsweredBy::Policy, true, epoch())
        .await
        .expect("cancel_answers_parked_permissions: the answer row must land");
    let stop = pump(session.as_mut(), &mut recorder)
        .await
        .expect("cancel_answers_parked_permissions: the cancelled turn still reaches done");
    recorder
        .finish()
        .await
        .expect("cancel_answers_parked_permissions: the recorder must close cleanly");

    assert_eq!(
        stop.stop_reason,
        StopReason::Cancelled,
        "cancel_answers_parked_permissions: a cancelled turn ends `cancelled`"
    );
    let log = rows(store, chat.step_id).await;
    let answers: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::PermissionAnswer)
        .collect();
    assert_eq!(
        answers.len(),
        1,
        "cancel_answers_parked_permissions: one permission_answer row per parked request"
    );
    assert_eq!(
        answers[0].payload.get("option_id"),
        Some(&Value::Null),
        "cancel_answers_parked_permissions: a cancelled request selects no option"
    );
    assert_eq!(
        answers[0].payload.get("cancelled").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        answers[0].role,
        EventRole::Htui,
        "cancel_answers_parked_permissions: a policy answer is `htui`'s row"
    );

    let calls: Vec<&str> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolCall)
        .filter_map(|row| row.tool_call_id.as_deref())
        .collect();
    let results: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolResult)
        .collect();
    assert_eq!(calls, vec!["call-1"]);
    for call in &calls {
        let result = results
            .iter()
            .find(|row| row.tool_call_id.as_deref() == Some(*call))
            .expect("cancel_answers_parked_permissions: every tool_call gets a tool_result");
        assert_eq!(
            str_at(result, "status"),
            Some(ToolResultStatus::Failed.as_str()),
            "cancel_answers_parked_permissions: a cancelled call fails"
        );
        assert_eq!(
            str_at(result, "terminal_reason"),
            Some(TerminalReason::Cancelled.as_str()),
            "cancel_answers_parked_permissions: the synthesized row says why"
        );
    }
    assert_eq!(
        log.last().map(|row| row.kind),
        Some(EventKind::Done),
        "cancel_answers_parked_permissions: `done` closes the log"
    );
}

/// A rejected permission answer terminates its tool call: the transport synthesizes one `failed`
/// `tool_result` with `terminal_reason = "rejected"`, and the protocol's own result for that call
/// never lands (`docs/ANA-4.md` §4.3 "Tool-call terminal states", §8 test strategy 2's "a tool call
/// whose result is a rejection").
async fn rejected_tool_gets_failed_result<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        tool_call("call-9"),
        park("req-9", "call-9"),
        tool_result("call-9", json!({ "text": "should never be recorded" })),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("rejected_tool_gets_failed_result: the prompt row must land");

    let request = pump_to_permission(session.as_mut(), &mut recorder).await;
    session
        .answer_permission(
            request.clone(),
            PermissionAnswer::Selected("reject-once".to_owned()),
        )
        .await
        .expect("rejected_tool_gets_failed_result: the answer must be accepted");
    recorder
        .record_permission_answer(
            &request,
            Some("reject-once"),
            AnsweredBy::User,
            false,
            epoch(),
        )
        .await
        .expect("rejected_tool_gets_failed_result: the answer row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("rejected_tool_gets_failed_result: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("rejected_tool_gets_failed_result: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let results: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolResult)
        .collect();
    assert_eq!(
        results.len(),
        1,
        "rejected_tool_gets_failed_result: exactly one result per call, the synthesized one"
    );
    assert_eq!(results[0].tool_call_id.as_deref(), Some("call-9"));
    assert_eq!(
        str_at(results[0], "status"),
        Some(ToolResultStatus::Failed.as_str())
    );
    assert_eq!(
        str_at(results[0], "terminal_reason"),
        Some(TerminalReason::Rejected.as_str())
    );
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("should never be recorded"),
        "rejected_tool_gets_failed_result: the protocol's own result for a rejected call is dropped"
    );

    let answer = log
        .iter()
        .find(|row| row.kind == EventKind::PermissionAnswer)
        .expect("rejected_tool_gets_failed_result: the answer is recorded");
    assert_eq!(str_at(answer, "option_id"), Some("reject-once"));
    assert_eq!(str_at(answer, "by"), Some(AnsweredBy::User.as_str()));
    assert_eq!(
        answer.role,
        EventRole::User,
        "rejected_tool_gets_failed_result: a user's answer is the user's row"
    );
}

/// Exactly one `done` per turn precedes the next accepted follow-up (`docs/ANA-4.md` §4.1): a
/// follow-up sent before it is a protocol error, not a queued prompt.
async fn done_precedes_next_follow_up<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::turns(vec![
        vec![chunk("still working", "m1"), done(StopReason::EndTurn)],
        vec![chunk("second turn", "m2"), done(StopReason::EndTurn)],
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("done_precedes_next_follow_up: the prompt row must land");

    let banner = session
        .next_event()
        .await
        .expect("done_precedes_next_follow_up: the first pull must not fail")
        .expect("done_precedes_next_follow_up: the session opens with an event");
    recorder
        .record(banner)
        .await
        .expect("done_precedes_next_follow_up: recording must land");
    let early = session.send_follow_up("too early".to_owned()).await;
    assert!(
        matches!(early, Err(DriverError::Transport(_))),
        "done_precedes_next_follow_up: a follow-up before the turn's done is refused, got {early:?}"
    );

    pump(session.as_mut(), &mut recorder)
        .await
        .expect("done_precedes_next_follow_up: turn 0 must reach done");
    recorder
        .record_follow_up("now then", epoch())
        .await
        .expect("done_precedes_next_follow_up: the follow-up row must land");
    session
        .send_follow_up("now then".to_owned())
        .await
        .expect("done_precedes_next_follow_up: a follow-up after done is accepted");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("done_precedes_next_follow_up: turn 1 must reach done");
    recorder
        .finish()
        .await
        .expect("done_precedes_next_follow_up: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let dones: Vec<i32> = log
        .iter()
        .filter(|row| row.kind == EventKind::Done)
        .map(|row| row.seq)
        .collect();
    let follow_up = log
        .iter()
        .find(|row| row.kind == EventKind::FollowUp)
        .expect("done_precedes_next_follow_up: the follow-up is recorded");
    assert_eq!(
        dones.len(),
        2,
        "done_precedes_next_follow_up: one done per turn"
    );
    assert!(
        dones[0] < follow_up.seq && follow_up.seq < dones[1],
        "done_precedes_next_follow_up: the follow-up sits between the two dones, got \
         {dones:?} and {}",
        follow_up.seq
    );
    assert_eq!(
        follow_up.turn, 1,
        "done_precedes_next_follow_up: the follow-up opens turn 1"
    );
    assert!(
        log.iter()
            .filter(|row| row.seq > follow_up.seq)
            .all(|row| row.turn == 1),
        "done_precedes_next_follow_up: every row after the follow-up belongs to turn 1"
    );
}

// ---------------------------------------------------------------------------------------------
// Usage, `other`, edit proposals and the flush bound
// ---------------------------------------------------------------------------------------------

/// `usage` rows are deltas and `run_step.usage` is their sum; the prompt digest reaches the same
/// write exactly once (`docs/ANA-4.md` §11 criteria 3 and 7, §4.1).
///
/// **This case reads a column its twelve siblings cannot.** `run_step.usage` and
/// `run_step.prompt_digest` are returned by no [`ReadStore`] method, and D15(a) defers adding one
/// to milestone 9, so the assertion is made on the `set_step_usage` calls a [`UsageSpy`] wrapping
/// the case's store recorded - the device T4's `tests/recorder.rs` already uses. Every other case
/// asserts on `step_events` rows; this exception is deliberate, and weakening it to something
/// `step_events` happens to expose would assert nothing about `run_step`.
async fn usage_deltas_sum_to_step_usage<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let usage = |input: i64, output: i64, cost: i64| {
        ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
            input_tokens: Some(input),
            output_tokens: Some(output),
            cost_micros: Some(cost),
            ..UsageEvent::default()
        }))
    };
    let script = Script::one_turn(vec![
        usage(10, 5, 100),
        usage(20, 7, 250),
        usage(0, 3, 1),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let spy = UsageSpy::new(store);
    let mut recorder = Recorder::new(&spy, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("usage_deltas_sum_to_step_usage: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("usage_deltas_sum_to_step_usage: the turn must reach done");
    let summary = recorder
        .finish()
        .await
        .expect("usage_deltas_sum_to_step_usage: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::Usage)
            .count(),
        3,
        "usage_deltas_sum_to_step_usage: each usage report is its own row"
    );

    let expected = json!({
        "input_tokens": 30,
        "output_tokens": 15,
        "cache_read_tokens": Value::Null,
        "cache_write_tokens": Value::Null,
        "cost_micros": 351,
    });
    assert_eq!(
        summary.usage, expected,
        "usage_deltas_sum_to_step_usage: the summary carries the sum"
    );
    let calls = spy.calls();
    assert_eq!(
        calls.last().map(|call| call.usage.clone()),
        Some(expected),
        "usage_deltas_sum_to_step_usage: run_step.usage is the sum of the step's usage rows"
    );

    let digest = format!("{:x}", Sha256::digest(PROMPT.as_bytes()));
    let carried: Vec<String> = calls
        .into_iter()
        .filter_map(|call| call.prompt_digest)
        .collect();
    assert_eq!(
        carried,
        vec![digest.clone()],
        "usage_deltas_sum_to_step_usage: the digest is written once and later writes pass None \
         (plan D15(b), X8)"
    );
    assert_eq!(
        str_at(&log[0], "digest"),
        Some(digest.as_str()),
        "usage_deltas_sum_to_step_usage: the prompt payload digest equals run_step.prompt_digest \
         (criterion 3)"
    );
}

/// A protocol update this event model does not map lands as kind `other` with its body verbatim,
/// which is what makes "every event" hold without a schema change per protocol revision
/// (`docs/ANA-4.md` §6.1).
async fn unknown_update_lands_in_other<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let body = json!({ "commands": ["compact", "review"], "n": 7 });
    let script = Script::one_turn(vec![
        ScriptEvent::Emit(DriverEvent::Other(OtherEvent {
            update: "available_commands_update".to_owned(),
            body: body.clone(),
        })),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("unknown_update_lands_in_other: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("unknown_update_lands_in_other: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("unknown_update_lands_in_other: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let row = log
        .iter()
        .find(|row| str_at(row, "update") == Some("available_commands_update"))
        .expect("unknown_update_lands_in_other: the unmapped update is persisted");
    assert_eq!(row.kind, EventKind::Other);
    assert_eq!(row.role, EventRole::Agent);
    assert_eq!(
        row.payload.get("body"),
        Some(&body),
        "unknown_update_lands_in_other: the body is stored verbatim"
    );
}

/// Edit proposals dedupe per `(tool_call_id, path)`: the buffered row is updated before the flush,
/// never written twice (plan D6, `docs/ANA-4.md` §4.3).
///
/// The rule asserted here is the **buffer-scoped** one: contiguous proposals collapse. ANA-4 §11
/// criterion 6's other half - a second write to the same path arriving *after* the row was already
/// flushed - needs an update path on the store seam that milestone 1 does not open, and is
/// milestone 3's.
async fn edit_proposal_deduped_per_call_and_path<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
) {
    let proposal = |path: &str, diff: &str, accepted: Option<bool>| {
        ScriptEvent::Emit(DriverEvent::EditProposal(EditProposalEvent {
            tool_call_id: Some("call-1".to_owned()),
            path: path.to_owned(),
            diff: diff.to_owned(),
            accepted,
        }))
    };
    let script = Script::one_turn(vec![
        proposal("src/a.rs", "@@ first", None),
        proposal("src/b.rs", "@@ other file", None),
        proposal("src/a.rs", "@@ second", Some(true)),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("edit_proposal_deduped_per_call_and_path: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("edit_proposal_deduped_per_call_and_path: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("edit_proposal_deduped_per_call_and_path: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let edits: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::EditProposal)
        .collect();
    assert_eq!(
        edits.len(),
        2,
        "edit_proposal_deduped_per_call_and_path: one row per (tool_call_id, path), not one per \
         proposal"
    );
    assert_eq!(str_at(edits[0], "path"), Some("src/a.rs"));
    assert_eq!(
        str_at(edits[0], "diff"),
        Some("@@ second"),
        "edit_proposal_deduped_per_call_and_path: the buffered row is updated in place"
    );
    assert_eq!(
        edits[0].payload.get("accepted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(str_at(edits[1], "path"), Some("src/b.rs"));
}

/// Flush trigger 4: a coalesced run is cut at [`CHUNK_FLUSH_BYTES`] rather than buffered
/// unboundedly, and the cut is a function of byte counts alone - no timer, so replay stays
/// deterministic (`docs/ANA-4.md` §4.1, §11 criterion 2).
async fn chunk_flush_at_16kib<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let piece = "x".repeat(1024);
    let mut events: Vec<ScriptEvent> = (0..17).map(|_| chunk(&piece, "m1")).collect();
    events.push(done(StopReason::EndTurn));
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, Script::one_turn(events), false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("chunk_flush_at_16kib: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("chunk_flush_at_16kib: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("chunk_flush_at_16kib: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let texts: Vec<usize> = log
        .iter()
        .filter(|row| row.kind == EventKind::AssistantText)
        .map(|row| text_of(row).len())
        .collect();
    assert_eq!(
        texts,
        vec![CHUNK_FLUSH_BYTES, 1024],
        "chunk_flush_at_16kib: 17 KiB of chunks is cut once at the bound and the remainder opens a \
         new run"
    );
}

// ---------------------------------------------------------------------------------------------
// The session banner and the scrubber
// ---------------------------------------------------------------------------------------------

/// The step's first `other` row is the `session_started` banner and it carries the agent-side
/// session id.
///
/// A downstream contract, not an internal detail: `docs/ANA-2.md` §4.8 resumes a promoted step by
/// querying exactly this row, because the id has no column in ANA-9 and MOD-2 chose a row over a
/// migration (`docs/ANA-4.md` §4.4). The payload nests the banner under `body` because
/// `OtherEvent` is `{ update, body }`; ANA-4 §4.4 writes the same keys flat.
async fn session_banner_is_first_other_row<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        chunk("hello", "m1"),
        ScriptEvent::Emit(DriverEvent::Other(OtherEvent {
            update: "config_option_update".to_owned(),
            body: json!({ "options": [] }),
        })),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let session_ref = session
        .session_ref()
        .cloned()
        .expect("session_banner_is_first_other_row: the transport reports a session id");
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("session_banner_is_first_other_row: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("session_banner_is_first_other_row: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("session_banner_is_first_other_row: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let banner = log
        .iter()
        .find(|row| row.kind == EventKind::Other)
        .expect("session_banner_is_first_other_row: the step has an `other` row");
    assert_eq!(
        str_at(banner, "update"),
        Some("session_started"),
        "session_banner_is_first_other_row: the first `other` row is the banner"
    );
    assert_eq!(
        banner.seq, 1,
        "session_banner_is_first_other_row: the banner is the session's first event, right after \
         the prompt"
    );
    assert_eq!(
        banner
            .payload
            .get("body")
            .and_then(|body| body.get("session_id"))
            .and_then(Value::as_str),
        Some(session_ref.as_str()),
        "session_banner_is_first_other_row: the row carries the id `session/load` needs \
         (ANA-2 §4.8)"
    );
    assert_eq!(
        banner
            .payload
            .get("body")
            .and_then(|body| body.get("protocol_version")),
        Some(&json!(1)),
        "session_banner_is_first_other_row: the banner reports the negotiated protocol version"
    );
}

/// A value from `SessionSpec.env` that reaches a payload or a `raw` blob is `[REDACTED]` in the
/// persisted row (`R-SEC-3`, `docs/ANA-4.md` §9: scrub before either write path).
async fn env_values_masked_in_rows<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        chunk(&format!("the token is {SECRET}"), "m1"),
        tool_call("call-1"),
        tool_result(
            "call-1",
            json!({ "text": format!("export {SECRET_ENV_KEY}={SECRET} && run") }),
        ),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    // `retain_raw`, so the `raw` column is under the same rule as the payload.
    let (chat, mut session) = open_case(harness, store, script, true).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, true, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("env_values_masked_in_rows: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("env_values_masked_in_rows: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("env_values_masked_in_rows: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let rendered = serde_json::to_string(&log).expect("the log serialises");
    assert!(
        !rendered.contains(SECRET),
        "env_values_masked_in_rows: no persisted row may carry an env value, payload or raw"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "env_values_masked_in_rows: the occurrence is masked, not dropped"
    );
    let text = log
        .iter()
        .find(|row| row.kind == EventKind::AssistantText)
        .expect("env_values_masked_in_rows: the assistant row is persisted");
    assert_eq!(text_of(text), "the token is [REDACTED]");
    let result = log
        .iter()
        .find(|row| row.kind == EventKind::ToolResult)
        .expect("env_values_masked_in_rows: the tool result is persisted");
    assert!(
        result.raw.is_some(),
        "env_values_masked_in_rows: the wire message is kept, and masked with the payload"
    );
}

/// Fail-closed (`R-SEC-3`, plan D6): a credential the scrubber could not mask blocks that row's
/// write entirely, leaves one `error { code: "scrub_residue" }` in its place with no gap in `seq`,
/// and makes `finish` report [`RecordError::Unmasked`].
async fn scrub_residue_refuses_write<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        chunk("before", "m1"),
        tool_call("call-1"),
        tool_result(
            "call-1",
            json!({ "output": format!("leaked {RESIDUE} here") }),
        ),
        chunk("after", "m2"),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("scrub_residue_refuses_write: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("scrub_residue_refuses_write: a refused row is not a recording failure");
    let outcome = recorder.finish().await;

    assert!(
        matches!(outcome, Err(RecordError::Unmasked(_))),
        "scrub_residue_refuses_write: finish reports the residue, got {outcome:?}"
    );

    let log = rows(store, chat.step_id).await;
    assert_eq!(
        log.iter()
            .filter(|row| row.kind == EventKind::ToolResult)
            .count(),
        0,
        "scrub_residue_refuses_write: the offending row is dropped entirely"
    );
    let errors: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Error)
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "scrub_residue_refuses_write: exactly one scrub_residue row in its place"
    );
    assert_eq!(errors[0].role, EventRole::Htui);
    assert_eq!(str_at(errors[0], "code"), Some("scrub_residue"));
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("sk-ant-"),
        "scrub_residue_refuses_write: the offending text never reaches the store, not even through \
         the error row"
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("the log is short")).collect::<Vec<_>>(),
        "scrub_residue_refuses_write: dropping a row does not open a gap in seq"
    );
}

#[cfg(test)]
mod tests {
    use super::{CASES, CaseHarness, Script, run_case};
    use crate::driver::AgentDriver;
    use crate::fake::FakeDriver;
    use htui_core::store::MemStore;

    /// The in-crate binding, so the guard below needs no integration test to exist.
    #[derive(Debug)]
    struct FakeHarness;

    impl CaseHarness for FakeHarness {
        fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
            Box::new(FakeDriver::scripted(script))
        }
    }

    #[test]
    fn case_names_are_unique() {
        let mut sorted = CASES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), CASES.len(), "case names are the suite's API");
    }

    #[tokio::test]
    async fn run_case_accepts_every_name_in_cases() {
        // `run_case` panics on a name it does not know, which is the whole point of its `match`:
        // running the list through it is what keeps `CASES` and the dispatcher in step.
        for name in CASES {
            run_case(name, &FakeHarness, &MemStore::demo()).await;
        }
    }
}
