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
    Agent, AgentBox, AgentId, Billing, BindingChange, BoxEdit, BoxId, BoxProbe, BoxRecord, BoxRow,
    ChatRunSpec, CitationKind, Claim, CommandRun, CoverageRow, Document, DocumentHead, DocumentId,
    EventKind, EventRole, GateOutcome, Item, ItemCitation, ItemFilter, ItemId, ItemKind,
    ItemKindId, ItemKindPatch, ItemPatch, ItemRequirement, ItemSummary, LinkGraph, NewCommandRun,
    NewDocument, NewItem, NewItemKind, NewNote, NewProject, NewPromptTemplate, NewRepo,
    NewRequirement, NewRequirementArea, NewRun, NewRunStep, NewSkill, NewSkillVersion,
    NewStepGraph, NewWorkspace, Note, PER_TOKEN_CAP_RUN, PhaseId, PhasePatch, Project, ProjectId,
    ProjectPatch, PromptScope, PromptTemplate, Quota, QuotaSource, Repo, RepoBoxPath, RepoId,
    RepoPatch, Requirement, RequirementArea, RequirementAreaId, RequirementFilter, RequirementId,
    RequirementPatch, RequirementRevision, RequirementSpec, RequirementUpdate, Resolution,
    ResolvedInput, Run, RunId, RunStatus, RunStep, RunStepCommit, RunStepTree, RunSummary, Scope,
    SessionEvent, Skill, SkillBinding, SkillBindingKey, SkillId, SkillPatch, SkillVersion, Status,
    StepGraph, StepGraphId, StepGraphPatch, StepGraphPhase, StepId, StepOutcome, StepStatus,
    UpstreamEntry, UserId, Workspace, WorkspaceBoxPath, WorkspaceId, WorkspacePatch,
    WorkspaceProject, normalize,
};
use htui_core::prompt::settings::SettingKey;
use htui_core::scrub::MinimalScrubber;
use htui_core::store::{
    CasOutcome, DeleteReach, DeleteTarget, ReadStore, Result as StoreResult, SettingRung,
    StoredSetting, UpdateOutcome, WriteStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, PermissionAnswer, PermissionPolicy,
    PermissionRequestId, SessionSpec, ToolExposure,
};
use crate::error::DriverError;
use crate::event::{
    DoneEvent, DriverEvent, EditProposalEvent, OtherEvent, PermissionOption, PermissionOptionKind,
    PermissionRequestEvent, StopReason, TerminalReason, TextChunk, ToolCallEvent, ToolKind,
    ToolResultEvent, ToolResultStatus, UsageEvent,
};
use crate::record::{
    AnsweredBy, CAP_EXCEEDED, CHUNK_FLUSH_BYTES, CapBreach, QuotaLatch, RecordError, Recorder,
    RecorderSummary, RunCap, pump,
};

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
    /// The named tool call was refused by the **transport's own** policy, with no request `htui`
    /// could have answered: the CLI's `--permission-mode` declining a tool (`docs/ANA-4.md` §4.3,
    /// §6.2, plan D85).
    ///
    /// A transport that *has* a permission channel never sees this marker — the cases that use it
    /// take the no-capability arm of their `caps.permission_requests` gate — so a harness for one
    /// refuses it by name, the way it refuses a scripted `permission_request`.
    ///
    /// What a harness owes for it is one [`DriverEvent::PermissionAnswer`] naming the refused call
    /// (`by: policy`, `denied: true`, no option, not cancelled) followed by the synthesized
    /// `failed` result harness rule 3 owes any settled call — the refusal is the answer, so the
    /// call is closed and the protocol's own later result for it is dropped. The event is emitted
    /// **directly**: since plan D93 the answer is a typed event, so no `other` row carries the
    /// refusal and no string has to be recognized to turn one into the row.
    PolicyDenied(String),
}

/// How a transport is built from a [`Script`]. The only thing a binding has to supply.
///
/// `crates/htui-agent/tests/fake_conformance.rs` is milestone 1's implementation; milestone 3's
/// ACP binding is the second, and it adds no case.
pub trait CaseHarness {
    /// A driver that will put `script` on this transport's wire.
    fn driver(&self, script: Script) -> Box<dyn AgentDriver>;

    /// What this transport can do, **before** there is a script to put on its wire.
    ///
    /// [`AgentDriver::caps`] answers the same question and `open_case` returns it, which is what
    /// a case gates its *assertions* on. This method exists because three of the six
    /// capability-gated cases (plan D80, D91) gate the **script** as well, and a script is what
    /// [`driver`](Self::driver) takes: `cancel_answers_parked_permissions` cannot script a park a
    /// transport would refuse by name, and `run_cap_breach_cancels_within_one_event` cannot end a
    /// turn with `ExpectCancel` on a transport whose cost report *is* the end of the turn. Reading
    /// the caps off a throwaway driver would mean building a second transport per case — a second
    /// child process, for a binding that spawns one — to ask a question the registry row already
    /// answers.
    ///
    /// No default body on purpose: a binding states this from the same place its driver gets it
    /// (the `agent` row, or the fake's own profile), and `open_case` asserts the two agree, so a
    /// declaration that drifts from the driver fails the first case rather than silently choosing
    /// the wrong arm for the rest of the suite.
    fn caps(&self) -> DriverCaps;
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
    "quota_blob_latches_agent_box",
    "run_cap_breach_cancels_within_one_event",
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
        "quota_blob_latches_agent_box" => quota_blob_latches_agent_box(harness, store).await,
        "run_cap_breach_cancels_within_one_event" => {
            run_cap_breach_cancels_within_one_event(harness, store).await;
        }
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
        // No case scripts a server-side budget: the cap under test in this suite is the
        // recorder's own, and a second one would make a breach ambiguous (plan D70, D90).
        budget_micros: None,
    }
}

/// Mints a chat `run` / `run_step` pair and starts a session on it, and reports what the transport
/// said it can do.
///
/// Two calls in one case give two independent steps of one store, which is how a case replays a
/// fixture twice without a second store.
///
/// The [`DriverCaps`] are read off the driver **before** `start`, which is the only place a case
/// can see them: `run_case` never holds a driver (blueprint P-7). They are what the six
/// capability-gated cases of plan D80 and D91 branch their assertions on — a case without the
/// capability asserts the *other* side of the contract rather than skipping, so `CASES` stays at
/// fifteen names and a transport that started emitting a row it says it cannot produce fails.
async fn open_case<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
    script: Script,
    retain_raw: bool,
) -> (ChatRunSpec, Box<dyn AgentSession>, DriverCaps) {
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
    let caps = driver.caps();
    assert_eq!(
        caps,
        harness.caps(),
        "a transport's capabilities are a property of its row, not of the session it is about to \
         open: the harness declared one profile and its driver reports another, so the cases that \
         chose a script from the declaration scripted the wrong thing"
    );
    let session = driver
        .start(session_spec(chat.step_id, retain_raw), PROMPT.to_owned())
        .await
        .expect("the transport must start a session");
    (chat, session, caps)
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

/// The rows a *driver* authored, i.e. the log minus the two kinds only `htui` can write.
///
/// `permission_answer` used to be on that list and is not since plan D93: a transport whose own
/// policy refused a call **reports** the answer ([`DriverEvent::PermissionAnswer`], `docs/ANA-4.md`
/// §4.1 as amended), so a filter that drops the kind would hide the one row
/// `rejected_tool_gets_failed_result`'s no-capability arm exists to assert, and would let a
/// transport pass criterion 4 by writing an answer row with no wire message behind it.
/// `prompt` and `follow_up` stay: no [`DriverEvent`] maps to either, so no driver can author one.
fn driver_rows(log: &[SessionEvent]) -> Vec<&SessionEvent> {
    log.iter()
        .filter(|row| !matches!(row.kind, EventKind::Prompt | EventKind::FollowUp))
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

/// Pulls and records envelopes until one of `kind` has been recorded.
///
/// [`pump_to_permission`]'s sibling for the transports that have no permission request to pump to
/// (plan D80): a case that has to act *mid-turn* — cancel, answer, assert — has to stop the pull
/// somewhere, and the kind is the only coordinate every binding shares.
///
/// # Panics
///
/// When the stream ends, errors, or reaches its `done` without one.
async fn pump_to_kind<S: WriteStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    kind: EventKind,
) {
    loop {
        let envelope = session
            .next_event()
            .await
            .unwrap_or_else(|error| {
                panic!("the transport must not fail before its `{kind}`: {error}")
            })
            .unwrap_or_else(|| panic!("the transport must reach its `{kind}`"));
        let found = EventKind::from(&envelope.event) == kind;
        let ended = matches!(envelope.event, DriverEvent::Done(_));
        recorder
            .record(envelope)
            .await
            .expect("recording must land");
        if found {
            return;
        }
        assert!(!ended, "the turn ended before its `{kind}`");
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

/// One `set_agent_box_quota` call a recorder made (plan D66-D68).
#[derive(Debug, Clone, PartialEq)]
struct QuotaCall {
    agent_id: AgentId,
    box_id: BoxId,
    quota: Value,
    quota_at: DateTime<Utc>,
}

/// A [`WriteStore`] that delegates to the case's store and remembers its `set_step_usage` calls.
///
/// `run_step.usage` and `run_step.prompt_digest` are returned by **no** [`ReadStore`] method -
/// `RunStepSummary` carries neither column, and a chat run has `item_id NULL`, so `ReadStore::runs`
/// cannot reach a chat step at all. Adding a read method to make them observable is exactly what
/// plan D15(a) defers to milestone 9, so this suite uses the device T4's `tests/recorder.rs`
/// already uses: wrap the store and watch the write. Generic over `S`, so a milestone 3 binding
/// over `PgStore` gets the same case for free.
///
/// `agent_box.quota` is the second such column, and for the same reason: the store suite is
/// generic over [`WriteStore`] and has no `agent_box` read at all, so the latch of plan D66-D68 is
/// observed where it is made.
struct UsageSpy<'a, S: WriteStore> {
    inner: &'a S,
    calls: Mutex<Vec<UsageCall>>,
    quota_calls: Mutex<Vec<QuotaCall>>,
}

impl<'a, S: WriteStore> UsageSpy<'a, S> {
    fn new(inner: &'a S) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
            quota_calls: Mutex::new(Vec::new()),
        }
    }

    /// Every `set_step_usage` call so far, in order.
    fn calls(&self) -> Vec<UsageCall> {
        self.calls
            .lock()
            .expect("the spy log is never poisoned")
            .clone()
    }

    /// Every `set_agent_box_quota` call so far, in order.
    fn quota_calls(&self) -> Vec<QuotaCall> {
        self.quota_calls
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
    async fn document(&self, id: DocumentId) -> StoreResult<Option<Document>> {
        self.inner.document(id).await
    }
    async fn documents_of_kinds(
        &self,
        item: ItemId,
        kinds: &[String],
    ) -> StoreResult<Vec<Document>> {
        self.inner.documents_of_kinds(item, kinds).await
    }
    async fn upstream_summaries(
        &self,
        id: ItemId,
        hops: u8,
        scope: &PromptScope,
    ) -> StoreResult<Vec<UpstreamEntry>> {
        self.inner.upstream_summaries(id, hops, scope).await
    }
    async fn project(&self, id: ProjectId) -> StoreResult<Option<Project>> {
        self.inner.project(id).await
    }

    // ---- MOD-4 milestone 1: ANA-2 §8's five run reads ---------------------------------------
    //
    // Delegated, none of them logged, for the reason the hierarchy block below gives: a decorator
    // owes `ReadStore` every method, and a chat session reads no orchestrated run.

    async fn run(&self, id: RunId) -> StoreResult<Option<Run>> {
        self.inner.run(id).await
    }
    async fn run_steps(&self, run: RunId) -> StoreResult<Vec<RunStep>> {
        self.inner.run_steps(run).await
    }
    async fn step_trees(&self, step: StepId) -> StoreResult<Vec<RunStepTree>> {
        self.inner.step_trees(step).await
    }
    async fn step_commits(&self, step: StepId) -> StoreResult<Vec<RunStepCommit>> {
        self.inner.step_commits(step).await
    }
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> StoreResult<Vec<ResolvedInput>> {
        self.inner.resolve_inputs(item, run, kinds).await
    }
    // ---- ANA-11 §5.1: requirements (MOD-38) ----
    async fn requirement_spec(&self, project: ProjectId) -> StoreResult<Option<RequirementSpec>> {
        self.inner.requirement_spec(project).await
    }
    async fn requirement_areas(&self, project: ProjectId) -> StoreResult<Vec<RequirementArea>> {
        self.inner.requirement_areas(project).await
    }
    async fn requirements(
        &self,
        project: ProjectId,
        filter: &RequirementFilter,
    ) -> StoreResult<Vec<Requirement>> {
        self.inner.requirements(project, filter).await
    }
    async fn requirement(&self, id: RequirementId) -> StoreResult<Option<Requirement>> {
        self.inner.requirement(id).await
    }
    async fn requirement_revisions(
        &self,
        id: RequirementId,
    ) -> StoreResult<Option<Vec<RequirementRevision>>> {
        self.inner.requirement_revisions(id).await
    }
    async fn item_requirements(&self, item: ItemId) -> StoreResult<Vec<ItemCitation>> {
        self.inner.item_requirements(item).await
    }
    async fn requirement_coverage(
        &self,
        requirement: RequirementId,
    ) -> StoreResult<Vec<CoverageRow>> {
        self.inner.requirement_coverage(requirement).await
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
    async fn record_box_probe(&self, probe: &BoxProbe) -> StoreResult<()> {
        self.inner.record_box_probe(probe).await
    }
    async fn boxes(&self) -> StoreResult<Vec<BoxRecord>> {
        self.inner.boxes().await
    }
    async fn edit_box(
        &self,
        id: BoxId,
        expected: i32,
        edit: BoxEdit,
    ) -> StoreResult<CasOutcome<BoxRow>> {
        self.inner.edit_box(id, expected, edit).await
    }
    async fn set_agent_box_quota(
        &self,
        agent_id: AgentId,
        box_id: BoxId,
        quota: Value,
        quota_at: DateTime<Utc>,
    ) -> StoreResult<()> {
        // The write first, the log after: the `set_step_usage` rule above, for the same reason.
        self.inner
            .set_agent_box_quota(agent_id, box_id, quota.clone(), quota_at)
            .await?;
        self.quota_calls
            .lock()
            .expect("the spy log is never poisoned")
            .push(QuotaCall {
                agent_id,
                box_id,
                quota,
                quota_at,
            });
        Ok(())
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
    /// Delegated, and deliberately not logged: this spy watches `set_step_usage` and the quota
    /// latch, and the chat path this suite drives never assembles a prompt (MOD-2 plan D97). The
    /// recorder's own `SpyStore` keeps the prompt log.
    async fn set_step_prompt(&self, step: StepId, digest: &str, trim: &Value) -> StoreResult<()> {
        self.inner.set_step_prompt(step, digest, trim).await
    }

    // ---- MOD-15 milestone 1: the hierarchy -------------------------------------------------
    //
    // Delegated, none of them logged: this spy watches `set_step_usage` and the quota latch, and
    // a chat session writes no hierarchy row. They are here because `WriteStore` has no default
    // bodies - a decorator owes every method - and not because any case reaches them.

    async fn create_workspace(&self, new: NewWorkspace) -> StoreResult<Workspace> {
        self.inner.create_workspace(new).await
    }
    async fn update_workspace(
        &self,
        id: WorkspaceId,
        expected: DateTime<Utc>,
        patch: WorkspacePatch,
    ) -> StoreResult<CasOutcome<Workspace>> {
        self.inner.update_workspace(id, expected, patch).await
    }
    async fn workspace(&self, id: WorkspaceId) -> StoreResult<Option<Workspace>> {
        self.inner.workspace(id).await
    }
    async fn upsert_workspace_project(&self, link: &WorkspaceProject) -> StoreResult<()> {
        self.inner.upsert_workspace_project(link).await
    }
    async fn remove_workspace_project(
        &self,
        workspace: WorkspaceId,
        project: ProjectId,
    ) -> StoreResult<()> {
        self.inner
            .remove_workspace_project(workspace, project)
            .await
    }
    async fn workspace_projects(
        &self,
        workspace: WorkspaceId,
    ) -> StoreResult<Vec<WorkspaceProject>> {
        self.inner.workspace_projects(workspace).await
    }
    async fn upsert_workspace_box_path(&self, path: &WorkspaceBoxPath) -> StoreResult<()> {
        self.inner.upsert_workspace_box_path(path).await
    }
    async fn workspace_box_paths(
        &self,
        workspace: WorkspaceId,
    ) -> StoreResult<Vec<WorkspaceBoxPath>> {
        self.inner.workspace_box_paths(workspace).await
    }
    async fn create_project(&self, new: NewProject) -> StoreResult<Project> {
        self.inner.create_project(new).await
    }
    async fn update_project(
        &self,
        id: ProjectId,
        expected: DateTime<Utc>,
        patch: ProjectPatch,
    ) -> StoreResult<CasOutcome<Project>> {
        self.inner.update_project(id, expected, patch).await
    }
    async fn create_repo(&self, new: NewRepo) -> StoreResult<Repo> {
        self.inner.create_repo(new).await
    }
    async fn update_repo(
        &self,
        id: RepoId,
        expected: DateTime<Utc>,
        patch: RepoPatch,
    ) -> StoreResult<CasOutcome<Repo>> {
        self.inner.update_repo(id, expected, patch).await
    }
    async fn repos(&self, project: ProjectId) -> StoreResult<Vec<Repo>> {
        self.inner.repos(project).await
    }
    async fn upsert_repo_box_path(&self, path: &RepoBoxPath) -> StoreResult<()> {
        self.inner.upsert_repo_box_path(path).await
    }
    async fn repo_box_paths(&self, repo: RepoId) -> StoreResult<Vec<RepoBoxPath>> {
        self.inner.repo_box_paths(repo).await
    }
    async fn create_item_kind(&self, new: NewItemKind) -> StoreResult<ItemKind> {
        self.inner.create_item_kind(new).await
    }
    async fn update_item_kind(
        &self,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    ) -> StoreResult<CasOutcome<ItemKind>> {
        self.inner.update_item_kind(id, expected, patch).await
    }
    async fn item_kinds(&self, project: ProjectId) -> StoreResult<Vec<ItemKind>> {
        self.inner.item_kinds(project).await
    }
    async fn delete_item_kind(&self, id: ItemKindId) -> StoreResult<()> {
        self.inner.delete_item_kind(id).await
    }
    async fn create_step_graph(&self, new: NewStepGraph) -> StoreResult<StepGraph> {
        self.inner.create_step_graph(new).await
    }
    async fn update_step_graph(
        &self,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    ) -> StoreResult<CasOutcome<StepGraph>> {
        self.inner.update_step_graph(id, expected, patch).await
    }
    async fn step_graphs(&self, project: ProjectId) -> StoreResult<Vec<StepGraph>> {
        self.inner.step_graphs(project).await
    }
    async fn create_phase(&self, phase: &StepGraphPhase) -> StoreResult<StepGraphPhase> {
        self.inner.create_phase(phase).await
    }
    async fn update_phase(
        &self,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    ) -> StoreResult<CasOutcome<StepGraphPhase>> {
        self.inner.update_phase(id, expected, patch).await
    }
    async fn phases(&self, graph: StepGraphId) -> StoreResult<Vec<StepGraphPhase>> {
        self.inner.phases(graph).await
    }
    async fn append_prompt_template(
        &self,
        new: NewPromptTemplate,
        expected: Option<i32>,
    ) -> StoreResult<CasOutcome<PromptTemplate>> {
        self.inner.append_prompt_template(new, expected).await
    }
    async fn skills(&self) -> StoreResult<Vec<Skill>> {
        self.inner.skills().await
    }
    async fn skill_versions(&self, skill: SkillId) -> StoreResult<Vec<SkillVersion>> {
        self.inner.skill_versions(skill).await
    }
    async fn skill_bindings(&self, project: Option<ProjectId>) -> StoreResult<Vec<SkillBinding>> {
        self.inner.skill_bindings(project).await
    }
    async fn create_skill(&self, new: NewSkill) -> StoreResult<(Skill, SkillVersion)> {
        self.inner.create_skill(new).await
    }
    async fn update_skill(
        &self,
        id: SkillId,
        expected: DateTime<Utc>,
        patch: SkillPatch,
    ) -> StoreResult<CasOutcome<Skill>> {
        self.inner.update_skill(id, expected, patch).await
    }
    async fn add_skill_version(
        &self,
        skill: SkillId,
        expected: i32,
        new: NewSkillVersion,
    ) -> StoreResult<CasOutcome<SkillVersion>> {
        self.inner.add_skill_version(skill, expected, new).await
    }
    async fn set_skill_binding(
        &self,
        key: SkillBindingKey,
        expected: Option<DateTime<Utc>>,
        change: BindingChange,
    ) -> StoreResult<CasOutcome<Option<SkillBinding>>> {
        self.inner.set_skill_binding(key, expected, change).await
    }
    async fn set_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        value: Value,
        expected: Option<DateTime<Utc>>,
    ) -> StoreResult<CasOutcome<StoredSetting>> {
        self.inner.set_setting(rung, key, value, expected).await
    }
    async fn clear_setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
        expected: DateTime<Utc>,
    ) -> StoreResult<CasOutcome<StoredSetting>> {
        self.inner.clear_setting(rung, key, expected).await
    }
    async fn setting(
        &self,
        rung: SettingRung,
        key: SettingKey,
    ) -> StoreResult<Option<StoredSetting>> {
        self.inner.setting(rung, key).await
    }
    async fn delete_reach(&self, target: DeleteTarget) -> StoreResult<Option<DeleteReach>> {
        self.inner.delete_reach(target).await
    }
    async fn delete_workspace(&self, id: WorkspaceId) -> StoreResult<DeleteReach> {
        self.inner.delete_workspace(id).await
    }
    async fn delete_project(&self, id: ProjectId) -> StoreResult<DeleteReach> {
        self.inner.delete_project(id).await
    }

    // ---- MOD-4 milestone 1: ANA-2 §8's eighteen run writers ---------------------------------
    //
    // Delegated, none of them logged, and for the hierarchy block's reason: a decorator owes
    // `WriteStore` every method. This spy counts `set_step_usage` and the quota latch, and a chat
    // run is minted by `start_chat_run`, never by `create_run` - nothing in MOD-4 changes what
    // there is to count, so nothing here records.

    async fn create_run(&self, new: NewRun) -> StoreResult<Run> {
        self.inner.create_run(new).await
    }
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> StoreResult<Claim> {
        self.inner
            .claim_run(run, box_id, owner, at, lease_until)
            .await
    }
    async fn refresh_lease(
        &self,
        run: RunId,
        owner: Uuid,
        until: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.refresh_lease(run, owner, until).await
    }
    async fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> StoreResult<Vec<Run>> {
        self.inner.adopt_runs(box_id, owner, now, lease_until).await
    }
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.take_lease(run, box_id, owner, now, until).await
    }
    async fn release_lease(
        &self,
        run: RunId,
        owner: Uuid,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.release_lease(run, owner, now).await
    }
    async fn create_step(&self, new: NewRunStep) -> StoreResult<RunStep> {
        self.inner.create_step(new).await
    }
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.transition_run(run, from, to, at).await
    }
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.transition_step(step, from, to, at).await
    }
    async fn finish_step(&self, step: StepId, outcome: StepOutcome) -> StoreResult<()> {
        self.inner.finish_step(step, outcome).await
    }
    async fn interrupt_step(
        &self,
        step: StepId,
        note: &str,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.interrupt_step(step, note, at).await
    }
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        self.inner.answer_gate(step, outcome, note, at).await
    }
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> StoreResult<()> {
        self.inner
            .select_fanout(run, position, attempt, winner, reason)
            .await
    }
    async fn supersede_step(&self, step: StepId) -> StoreResult<()> {
        self.inner.supersede_step(step).await
    }
    async fn upsert_step_tree(&self, step: StepId, trees: &[RunStepTree]) -> StoreResult<()> {
        self.inner.upsert_step_tree(step, trees).await
    }
    async fn record_commits(&self, step: StepId, commits: &[RunStepCommit]) -> StoreResult<()> {
        self.inner.record_commits(step, commits).await
    }
    async fn record_command_run(&self, new: NewCommandRun) -> StoreResult<CommandRun> {
        self.inner.record_command_run(new).await
    }
    async fn command_runs(&self, step: StepId) -> StoreResult<Vec<CommandRun>> {
        self.inner.command_runs(step).await
    }
    async fn write_document(&self, new: NewDocument) -> StoreResult<Document> {
        self.inner.write_document(new).await
    }
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> StoreResult<()> {
        self.inner.promote_step(step, at).await
    }
    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> StoreResult<()> {
        self.inner.fail_run(run, failure, at).await
    }
    async fn finish_run(
        &self,
        run: RunId,
        to: RunStatus,
        failure: Option<&str>,
        at: DateTime<Utc>,
    ) -> StoreResult<()> {
        self.inner.finish_run(run, to, failure, at).await
    }
    async fn close_out(
        &self,
        item: ItemId,
        resolution: Resolution,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> StoreResult<Document> {
        self.inner
            .close_out(item, resolution, summary, commits)
            .await
    }
    async fn add_note(&self, note: NewNote) -> StoreResult<Note> {
        self.inner.add_note(note).await
    }
    // ---- ANA-11 §5.1: requirements and citations (MOD-38) ----
    async fn set_requirement_spec(
        &self,
        project: ProjectId,
        expected_version: Option<i32>,
        owner_id: UserId,
        preamble: String,
    ) -> StoreResult<CasOutcome<RequirementSpec>> {
        self.inner
            .set_requirement_spec(project, expected_version, owner_id, preamble)
            .await
    }
    async fn create_requirement_area(
        &self,
        new: NewRequirementArea,
    ) -> StoreResult<RequirementArea> {
        self.inner.create_requirement_area(new).await
    }
    async fn mint_requirement(
        &self,
        area: RequirementAreaId,
        new: NewRequirement,
    ) -> StoreResult<Requirement> {
        self.inner.mint_requirement(area, new).await
    }
    async fn amend_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        patch: RequirementPatch,
        amended_by: ItemId,
    ) -> StoreResult<RequirementUpdate> {
        self.inner
            .amend_requirement(id, expected_version, patch, amended_by)
            .await
    }
    async fn withdraw_requirement(
        &self,
        id: RequirementId,
        expected_version: i32,
        withdrawn_by: ItemId,
        author_id: UserId,
        box_id: Option<BoxId>,
    ) -> StoreResult<RequirementUpdate> {
        self.inner
            .withdraw_requirement(id, expected_version, withdrawn_by, author_id, box_id)
            .await
    }
    async fn cite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
        proposed_by: Option<StepId>,
    ) -> StoreResult<ItemRequirement> {
        self.inner.cite(item, requirement, kind, proposed_by).await
    }
    async fn uncite(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> StoreResult<()> {
        self.inner.uncite(item, requirement, kind).await
    }
    async fn reconfirm(
        &self,
        item: ItemId,
        requirement: RequirementId,
        kind: CitationKind,
    ) -> StoreResult<ItemRequirement> {
        self.inner.reconfirm(item, requirement, kind).await
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

    let (first, mut session, _caps) = open_case(harness, store, coalescing_script(), false).await;
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

    let (second, mut session, _caps) = open_case(harness, store, coalescing_script(), false).await;
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
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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
        let (chat, mut session, _caps) = open_case(harness, store, script(), retain_raw).await;
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
///
/// **Two arms, one case** (plan D80). The gate is `caps.permission_requests`, and the script is
/// gated with the assertions: a transport with no permission channel would refuse a scripted park
/// by name, and a harness that fabricated one would be proving the harness. Without the capability
/// the case asserts the *negative* — no permission row of either kind reaches the log — and then
/// every clause of criterion 5 that does not mention a permission, which is most of it. Skipping
/// instead would keep the name in `CASES` and drop the coverage; this way `CASES` stays at fifteen
/// and a transport that claims no permission channel but writes a permission row still fails.
async fn cancel_answers_parked_permissions<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    if !harness.caps().permission_requests {
        cancel_without_a_permission_channel(harness, store).await;
        return;
    }
    let script = Script::one_turn(vec![
        chunk("about to read ", "m1"),
        tool_call("call-1"),
        park("req-1", "call-1"),
        // The turn has no `done` of its own: the cancel is what ends it.
        ScriptEvent::ExpectCancel,
    ]);
    let scrubber = scrubber();
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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

/// `cancel_answers_parked_permissions` over a transport that reports
/// `caps.permission_requests: false` (plan D80, `docs/ANA-4.md` §4.3).
///
/// Nothing is parked, so nothing is answered — and that is the assertion, not the excuse: the log
/// must carry **no** `permission_request` row and **no** `permission_answer` row, which is what
/// turns the capability from a banner string into something the suite enforces. What survives of
/// criterion 5 is everything else it says: a cancel still closes every open tool call with a
/// synthesized `failed` result naming `cancelled`, and the turn still ends with exactly one
/// `done { stop_reason: "cancelled" }` as the log's last row.
async fn cancel_without_a_permission_channel<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
) {
    let script = Script::one_turn(vec![
        chunk("about to read ", "m1"),
        tool_call("call-1"),
        // The turn has no `done` of its own: the cancel is what ends it. No park — a transport
        // that cannot park has nothing to park, and scripting one would ask the harness to invent
        // a request the dialect has no shape for.
        ScriptEvent::ExpectCancel,
    ]);
    let scrubber = scrubber();
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("cancel_answers_parked_permissions: the prompt row must land");

    // Stopping on the `tool_call` is what leaves a call open for the cancel to close, which is the
    // clause of criterion 5 this arm still measures.
    pump_to_kind(session.as_mut(), &mut recorder, EventKind::ToolCall).await;
    session
        .cancel(Duration::from_millis(0))
        .await
        .expect("cancel_answers_parked_permissions: cancel must succeed");
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
    assert!(
        !log.iter().any(|row| matches!(
            row.kind,
            EventKind::PermissionRequest | EventKind::PermissionAnswer
        )),
        "cancel_answers_parked_permissions: a transport that reports no permission channel puts \
         no permission row in the log, cancel included: {:?}",
        kinds(&log)
    );

    let results: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolResult)
        .collect();
    assert_eq!(
        results
            .iter()
            .filter_map(|row| row.tool_call_id.as_deref())
            .collect::<Vec<_>>(),
        vec!["call-1"],
        "cancel_answers_parked_permissions: the open call gets exactly one synthesized result"
    );
    assert_eq!(
        str_at(results[0], "status"),
        Some(ToolResultStatus::Failed.as_str()),
        "cancel_answers_parked_permissions: a cancelled call fails"
    );
    assert_eq!(
        str_at(results[0], "terminal_reason"),
        Some(TerminalReason::Cancelled.as_str()),
        "cancel_answers_parked_permissions: the synthesized row says why"
    );

    assert_eq!(
        log.iter().filter(|row| row.kind == EventKind::Done).count(),
        1,
        "cancel_answers_parked_permissions: exactly one `done` per turn (§4.1): {:?}",
        kinds(&log)
    );
    assert_eq!(
        log.last().map(|row| row.kind),
        Some(EventKind::Done),
        "cancel_answers_parked_permissions: `done` closes the log"
    );
    assert_eq!(
        log.last().and_then(|row| str_at(row, "stop_reason")),
        Some(StopReason::Cancelled.as_str()),
        "cancel_answers_parked_permissions: and it says `cancelled`"
    );
}

/// A rejected permission answer terminates its tool call: the transport synthesizes one `failed`
/// `tool_result` with `terminal_reason = "rejected"`, and the protocol's own result for that call
/// never lands (`docs/ANA-4.md` §4.3 "Tool-call terminal states", §8 test strategy 2's "a tool call
/// whose result is a rejection").
///
/// **Two arms, one case** (plan D80), gated on `caps.permission_requests`. The rule under test is
/// about a *refusal* settling a call, and both transports have refusals — one asks `htui` and is
/// told no, the other is told no by its own `--permission-mode` and reports it. The second arm is
/// the same sentence with the refusal arriving already decided.
async fn rejected_tool_gets_failed_result<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    if !harness.caps().permission_requests {
        policy_denial_gets_failed_result(harness, store).await;
        return;
    }
    let script = Script::one_turn(vec![
        tool_call("call-9"),
        park("req-9", "call-9"),
        tool_result("call-9", json!({ "text": "should never be recorded" })),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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

/// `rejected_tool_gets_failed_result` over a transport that reports
/// `caps.permission_requests: false` (plan D80, D85, `docs/ANA-4.md` §4.3, §6.2).
///
/// The refusal arrives already decided: the transport's own policy declined the call and reports
/// the answer ([`ScriptEvent::PolicyDenied`], plan D93's typed event). Two negatives and two
/// positives:
///
/// - **no** `permission_request` row — nothing was ever asked, and a transport that wrote one
///   would be claiming a channel it says it does not have;
/// - the protocol's own late result for the settled call is still dropped (§4.3's rule survives
///   the change of who refused), which is what the `should never be recorded` string checks;
/// - exactly one `tool_result` for the call, `failed` — a refusal closes a call whoever made it;
/// - the denial is **in the log**, as `by: policy`, `role: htui`, no option, `denied: true`, and
///   joinable to the call from either side. That row is the whole reason the degradation is
///   visible in a transcript at all (D85), and `driver_rows` no longer filters the kind out.
async fn policy_denial_gets_failed_result<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let script = Script::one_turn(vec![
        tool_call("call-9"),
        ScriptEvent::PolicyDenied("call-9".to_owned()),
        tool_result("call-9", json!({ "text": "should never be recorded" })),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
    let mut recorder = Recorder::new(store, &scrubber, chat.step_id, false, None);
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("rejected_tool_gets_failed_result: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("rejected_tool_gets_failed_result: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("rejected_tool_gets_failed_result: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    assert!(
        !log.iter()
            .any(|row| row.kind == EventKind::PermissionRequest),
        "rejected_tool_gets_failed_result: a transport with no permission channel asks nothing: \
         {:?}",
        kinds(&log)
    );
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
    assert!(
        !serde_json::to_string(&log)
            .expect("the log serialises")
            .contains("should never be recorded"),
        "rejected_tool_gets_failed_result: the transport's own result for a settled call is dropped"
    );

    let answers: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::PermissionAnswer)
        .collect();
    assert_eq!(
        answers.len(),
        1,
        "rejected_tool_gets_failed_result: one refusal is one `permission_answer` row"
    );
    assert_eq!(str_at(answers[0], "by"), Some(AnsweredBy::Policy.as_str()));
    assert_eq!(
        answers[0].role,
        EventRole::Htui,
        "rejected_tool_gets_failed_result: a policy answer is `htui`'s row whoever reported it"
    );
    assert_eq!(
        answers[0].payload.get("option_id"),
        Some(&Value::Null),
        "rejected_tool_gets_failed_result: a denial picks no option"
    );
    assert_eq!(
        answers[0].payload.get("denied").and_then(Value::as_bool),
        Some(true),
        "rejected_tool_gets_failed_result: the key that tells a denial from an answer somebody gave"
    );
    assert_eq!(
        str_at(answers[0], "request_id"),
        Some("call-9"),
        "rejected_tool_gets_failed_result: with no request to name, the refused call names itself"
    );
    assert_eq!(
        answers[0].tool_call_id.as_deref(),
        Some("call-9"),
        "rejected_tool_gets_failed_result: and it reaches the column `idx_session_event_tool` joins \
         on"
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
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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
///
/// **Two arms, one case** (plan D91, blueprint E-4), gated on `caps.usage_mid_turn`, and the
/// **script is the same**: what changes is how many rows the transport needs to report the same
/// spend. A dialect whose cost exists only on the message that ends the turn (`docs/ANA-4.md` §7)
/// reports it once, so the arm asserts **one** `usage` row instead of three — and then asserts the
/// criterion in its strongest form, because with one row the sum has nowhere to hide: the row's
/// own `cost_micros` *is* the total, and `cost_micros_total` must agree with it. Everything about
/// `run_step.usage` and the digest is shared, which is the point: criterion 7 is one statement.
async fn usage_deltas_sum_to_step_usage<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    // **The script reports no token counts, on purpose** (amended in milestone 3). ANA-4 §7 is
    // explicit that ACP carries no per-turn token fields — `usage_update` has `used`, `size` and
    // `cost`, and nothing else — so a script asking for `input_tokens` would be asking one
    // transport to invent a number the protocol does not have, and criterion 1 ("adding a
    // transport adds no case") would be bought by making the case untrue. The recorder's token
    // summing is still covered, by `tests/recorder.rs`, where the events are authored directly.
    let usage = |used: i64, cost: i64| {
        ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
            cost_micros: Some(cost),
            context_used: Some(used),
            context_size: Some(200_000),
            ..UsageEvent::default()
        }))
    };
    let script = Script::one_turn(vec![
        usage(1_000, 100),
        usage(2_000, 250),
        usage(2_100, 1),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session, caps) = open_case(harness, store, script, false).await;
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
    let reports: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Usage)
        .collect();
    if caps.usage_mid_turn {
        assert_eq!(
            reports.len(),
            3,
            "usage_deltas_sum_to_step_usage: each usage report is its own row"
        );
    } else {
        assert_eq!(
            reports.len(),
            1,
            "usage_deltas_sum_to_step_usage: a transport that reports cost once, on the message \
             that ends the turn, writes one row for the turn (plan D91): {:?}",
            kinds(&log)
        );
        assert_eq!(
            reports[0].payload.get("cost_micros"),
            Some(&json!(351)),
            "usage_deltas_sum_to_step_usage: and that row's delta is the whole turn's spend"
        );
        assert_eq!(
            reports[0].payload.get("cost_micros_total"),
            Some(&json!(351)),
            "usage_deltas_sum_to_step_usage: on a first turn the delta and the running total are \
             the same number, and a transport that reported the vendor's cumulative figure as the \
             delta would fail here rather than in a second turn nobody scripted"
        );
    }

    let expected = json!({
        "input_tokens": Value::Null,
        "output_tokens": Value::Null,
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

/// The vendor rate-limit value of `crates/htui-agent/tests/fixtures/claude_acp_turn.jsonl` line 5,
/// verbatim: the one real blob milestone 7 was designed against.
///
/// A **payload** value and not a wire shape. `UsageEvent.quota` carries whatever a transport's own
/// vendor-extension slot held, and where that slot is belongs to the binding — over ACP it is
/// `_meta` of the `usage_update`, which `crate::acp::map` names once. This suite therefore names
/// no key, which is what keeps the case transport-neutral (criterion 1).
fn rate_limit_blob() -> Value {
    json!({
        "status": "allowed",
        "resetsAt": 1_788_801_600_i64,
        "rateLimitType": "five_hour",
        "overageStatus": "rejected",
        "overageDisabledReason": "org_level_disabled",
        "isUsingOverage": false,
        "unifiedWindows": {
            "five_hour": { "utilization": 0.11, "resetsAt": 1_788_801_600_i64 },
            "seven_day": { "utilization": 0.62, "resetsAt": 1_788_854_400_i64 },
        },
    })
}

/// A `usage` row's vendor rate-limit blob becomes `agent_box.quota`: the passive latch of
/// `docs/ANA-4.md` §7 (`:1110-1135`, plan D66-D68), one write per row that has something to say.
///
/// **This case reads a column its siblings cannot**, like `usage_deltas_sum_to_step_usage` above:
/// the store suite is generic over [`WriteStore`] and has no `agent_box` read, so the latch is
/// observed as the [`UsageSpy`] saw it made. `crates/htui/tests/chat_usage_pg.rs` is where it is
/// read back out of a real `agent_box` row.
///
/// Two scripts, because §7 has two shapes and the *row* decides which:
///
/// - a source that reports an allowance publishes status, windows and spend — and says nothing at
///   all until its first blob arrives, since a turn's first report carries none and an empty
///   window list would erase the standing document (blueprint H-3);
/// - a source that reports none publishes spend alone, on every costed row, which is the seeded
///   live-ACP row's shape (plan D65).
///
/// Both are selected by `agent.settings.quota.source`, which the latch carries. Nothing here
/// reads an agent's name (`R-AGT-5`), and the blob is a payload value both transports move
/// unchanged.
///
/// **Two arms, one case** (plan D91, blueprint E-4), gated on `caps.usage_mid_turn`, with both
/// scripts unchanged. A transport that reports cost once, at the end of the turn, has one row to
/// carry the blob and one latch to make — so H-3's "says nothing until a blob arrives" has no
/// second report to be visible in, and the first *costed* row is also the blob's row. What the arm
/// still asserts is everything that makes the latch a latch: the blob reaches the row verbatim,
/// the document carries the turn's whole spend, and `quota_at` is the capture time of the row that
/// produced it.
async fn quota_blob_latches_agent_box<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    // The row the latch writes into. `quota` is NULL on a fresh row and is the narrow setter's
    // alone (plan D74), so there is nothing to plant: what this case pins is that the latch puts
    // §7's document where there was none.
    let probe = json!({ "status": "ready", "source": "probe" });
    store
        .upsert_agent_box(&AgentBox {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            enabled: true,
            version: Some("1.2.3".to_owned()),
            path: Some("agent".to_owned()),
            probed_at: Some(epoch()),
            quota: None,
            quota_at: None,
            updated_at: epoch(),
            probe: Some(probe),
        })
        .await
        .expect("quota_blob_latches_agent_box: the probed row must land");

    // The same three reports `usage_deltas_sum_to_step_usage` uses — deltas 100, 250, 1, so the
    // session spend runs 100, 350, 351 — with a vendor blob on whichever row the script says.
    let usage = |used: i64, cost: i64, quota: Option<Value>| {
        ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
            cost_micros: Some(cost),
            context_used: Some(used),
            context_size: Some(200_000),
            quota,
            ..UsageEvent::default()
        }))
    };
    let scrubber = scrubber();

    // --- Script A: the blob arrives on the second report ---------------------------------------
    let script = Script::one_turn(vec![
        usage(1_000, 100, None),
        usage(2_000, 250, Some(rate_limit_blob())),
        usage(2_100, 1, None),
        done(StopReason::EndTurn),
    ]);
    let (chat, mut session, caps) = open_case(harness, store, script, false).await;
    let spy = UsageSpy::new(store);
    let mut recorder =
        Recorder::new(&spy, &scrubber, chat.step_id, false, None).with_quota_latch(QuotaLatch {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            source: QuotaSource::AcpMetaRateLimit,
            billing: Billing::Subscription,
        });
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("quota_blob_latches_agent_box: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("quota_blob_latches_agent_box: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("quota_blob_latches_agent_box: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let reports: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Usage)
        .collect();
    let calls = spy.quota_calls();
    if caps.usage_mid_turn {
        assert_eq!(
            reports.len(),
            3,
            "quota_blob_latches_agent_box: each usage report is its own row"
        );
        assert_eq!(
            reports
                .iter()
                .map(|row| row.payload.get("quota"))
                .collect::<Vec<_>>(),
            vec![None, Some(&rate_limit_blob()), None],
            "quota_blob_latches_agent_box: the persisted row carries the blob exactly as the \
             report did — verbatim where there was one, absent where there was not (plan D66)"
        );
        assert_eq!(
            calls.len(),
            2,
            "quota_blob_latches_agent_box: the first report has no allowance to publish and its \
             spend alone would empty the column, so only the two rows after it latch (blueprint \
             H-3), got {calls:?}"
        );
        assert_eq!(
            calls[0].quota,
            normalize(
                QuotaSource::AcpMetaRateLimit,
                Billing::Subscription,
                Some(&rate_limit_blob()),
                Some(350),
                reports[1].at,
            )
            .to_value(),
            "quota_blob_latches_agent_box: the blob's own row publishes it with the spend so far"
        );
        assert_eq!(
            calls[1].quota,
            normalize(
                QuotaSource::AcpMetaRateLimit,
                Billing::Subscription,
                Some(&rate_limit_blob()),
                Some(351),
                reports[2].at,
            )
            .to_value(),
            "quota_blob_latches_agent_box: and the report after it refreshes the spend while the \
             windows stand"
        );
        assert_eq!(
            calls[1].quota_at, reports[2].at,
            "quota_blob_latches_agent_box: `quota_at` is the capture time of the row that produced \
             the document, which its `observed_at` mirrors (§7)"
        );
    } else {
        // The turn-end form (plan D91): one report, so the blob's row and the turn's only costed
        // row are the same row and there is one latch to make. H-3's "publish nothing until a blob
        // arrives" has no second report to be visible in — what is left of it is that the one call
        // carries the blob, which a transport reporting spend with no allowance would fail.
        assert_eq!(
            reports.len(),
            1,
            "quota_blob_latches_agent_box: a transport that reports cost once writes one row for \
             the turn (plan D91): {:?}",
            kinds(&log)
        );
        assert_eq!(
            reports[0].payload.get("quota"),
            Some(&rate_limit_blob()),
            "quota_blob_latches_agent_box: the persisted row carries the blob verbatim (plan D66)"
        );
        assert_eq!(
            calls.len(),
            1,
            "quota_blob_latches_agent_box: one report that has something to say is one latch, got \
             {calls:?}"
        );
        assert_eq!(
            calls[0].quota,
            normalize(
                QuotaSource::AcpMetaRateLimit,
                Billing::Subscription,
                Some(&rate_limit_blob()),
                Some(351),
                reports[0].at,
            )
            .to_value(),
            "quota_blob_latches_agent_box: the turn's one row publishes the blob with the turn's \
             whole spend"
        );
        assert_eq!(
            calls[0].quota_at, reports[0].at,
            "quota_blob_latches_agent_box: `quota_at` is the capture time of the row that produced \
             the document, which its `observed_at` mirrors (§7)"
        );
    }
    assert!(
        calls
            .iter()
            .all(|call| call.agent_id == ids::AGENT_CLAUDE && call.box_id == ids::BOX),
        "quota_blob_latches_agent_box: every latch names the row the chat runs on"
    );
    // The document the *last* latch published, which is the one the Settings cell would read:
    // whether it took two calls or one, it says the same thing about the same turn.
    let last = calls
        .last()
        .expect("quota_blob_latches_agent_box: the blob's row latches");
    let document = Quota::from_value(&last.quota)
        .expect("quota_blob_latches_agent_box: the latched document parses back");
    assert_eq!(
        document
            .windows
            .iter()
            .map(|window| window.id.as_str())
            .collect::<Vec<_>>(),
        ["five_hour", "seven_day"],
        "quota_blob_latches_agent_box: both windows, sorted by id"
    );
    assert_eq!(document.status.as_deref(), Some("allowed"));
    assert!(!document.exhausted);
    assert_eq!(document.spend.session_micros, Some(351));

    // --- Script B: a source that reports no allowance, so spend is the whole document ----------
    let script = Script::one_turn(vec![
        usage(1_000, 100, None),
        usage(2_000, 250, None),
        usage(2_100, 1, None),
        done(StopReason::EndTurn),
    ]);
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
    let spy = UsageSpy::new(store);
    let mut recorder =
        Recorder::new(&spy, &scrubber, chat.step_id, false, None).with_quota_latch(QuotaLatch {
            agent_id: ids::AGENT_CLAUDE,
            box_id: ids::BOX,
            source: QuotaSource::None,
            billing: Billing::PerToken,
        });
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("quota_blob_latches_agent_box: the prompt row must land");
    pump(session.as_mut(), &mut recorder)
        .await
        .expect("quota_blob_latches_agent_box: the turn must reach done");
    recorder
        .finish()
        .await
        .expect("quota_blob_latches_agent_box: the recorder must close cleanly");

    let log = rows(store, chat.step_id).await;
    let reports: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::Usage)
        .collect();
    assert!(
        reports.iter().all(|row| row.payload.get("quota").is_none()),
        "quota_blob_latches_agent_box: a report that carried no blob persists none"
    );
    let calls = spy.quota_calls();
    // One latch per costed row, whichever arm this is: over a mid-turn transport that is the three
    // running spends, over a turn-end one the single report is the turn's whole spend. The
    // expected list is built from `reports` rather than from a literal, so the only thing the two
    // arms disagree about is how many rows carried the same money (plan D91).
    let spends: &[i64] = if caps.usage_mid_turn {
        &[100, 350, 351]
    } else {
        &[351]
    };
    assert_eq!(
        reports.len(),
        spends.len(),
        "quota_blob_latches_agent_box: one costed row per report this transport makes: {:?}",
        kinds(&log)
    );
    assert_eq!(
        calls
            .iter()
            .map(|call| call.quota.clone())
            .collect::<Vec<_>>(),
        spends
            .iter()
            .enumerate()
            .map(|(index, spend)| normalize(
                QuotaSource::None,
                Billing::PerToken,
                None,
                Some(*spend),
                reports[index].at,
            )
            .to_value())
            .collect::<Vec<_>>(),
        "quota_blob_latches_agent_box: a source that reports no allowance latches every costed \
         row, and each document is spend plus the two row-side facts"
    );
    for call in &calls {
        let document = Quota::from_value(&call.quota)
            .expect("quota_blob_latches_agent_box: the latched document parses back");
        assert_eq!(document.source, QuotaSource::None);
        assert_eq!(document.billing, Billing::PerToken);
        assert_eq!(document.status, None, "no status is reported");
        assert!(document.windows.is_empty(), "and no windows");
        assert!(!document.exhausted);
        assert_eq!(document.spend.currency.as_deref(), Some("USD"));
    }
}

/// One capped session played to its end, and everything a cap assertion needs from it.
///
/// A struct rather than a tuple because five of the six things this case reads about a run are
/// only observable in different places: the rows in the store, the verdict in the summary, the
/// turn's end in [`pump`]'s return, `run_step.usage` in the [`UsageSpy`], and the step's identity
/// in the log's own key.
struct Played {
    /// The persisted log, in `seq` order.
    log: Vec<SessionEvent>,
    /// What the recorder said it wrote, including [`RecorderSummary::cap_breach`].
    summary: RecorderSummary,
    /// The `done` the turn ended with, as the loop that drove it reported it.
    stop: DoneEvent,
    /// Every `set_step_usage` call, so the running spend the cap compared against is observable.
    usage_calls: Vec<UsageCall>,
    /// The step, for [`without_identity`].
    step: StepId,
    /// The transport's session id, for [`without_identity`].
    session: Option<AgentSessionRef>,
}

/// Plays one script through a recorder carrying `cap` (`None` = unbounded) and reports what it
/// wrote.
///
/// The grace is **zero**, which is what [`RunCap::grace`] carrying it rather than [`pump`] taking
/// it is for: a suite that waited out a real cancel window would pay for it once per case, and the
/// fake owns no child process to wait for (`cancel_answers_parked_permissions` passes zero for the
/// same reason).
async fn play_capped<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
    cap: Option<i64>,
    script: Script,
) -> Played {
    let scrubber = scrubber();
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
    let session_ref = session.session_ref().cloned();
    let spy = UsageSpy::new(store);
    let mut recorder = Recorder::new(&spy, &scrubber, chat.step_id, false, None);
    if let Some(micros) = cap {
        recorder = recorder.with_run_cap(RunCap {
            micros,
            grace: Duration::from_millis(0),
        });
    }
    recorder
        .record_prompt(PROMPT, prompt_sections(), epoch())
        .await
        .expect("run_cap_breach_cancels_within_one_event: the prompt row must land");
    let stop = pump(session.as_mut(), &mut recorder)
        .await
        .expect("run_cap_breach_cancels_within_one_event: the turn must reach an end");
    let summary = recorder
        .finish()
        .await
        .expect("run_cap_breach_cancels_within_one_event: the recorder must close cleanly");
    Played {
        log: rows(store, chat.step_id).await,
        summary,
        stop,
        usage_calls: spy.calls(),
        step: chat.step_id,
        session: session_ref,
    }
}

/// The per-run cap of `docs/ANA-4.md` §7 (`:1143-1150`, as amended by plan D69) and §11 criterion
/// 8: the session is cancelled **within one event** of the breach and the step's last two rows are
/// `error{code:"cap_exceeded"}` then `done{stop_reason:"cancelled"}`.
///
/// "Within one event" is asserted as adjacency — the row after the breaching `usage` row is the
/// `error` — which is the strongest form of the claim and the one this script can make: no tool
/// call is open, so the cancel synthesizes nothing to sit in between. The interleaving where it
/// does (an open call gets a `tool_result` between the two) is `tests/recorder.rs`'s, because the
/// row it inserts is a property of a transport's cancel and not of the cap.
///
/// Transport-neutral by construction (criterion 1): the *detection* is the recorder's, the
/// *cancel* is [`crate::record::enforce_breach`]'s, and neither knows which transport is on the
/// other side of the [`AgentSession`] it holds. Four runs, because a guard rail has to be provably
/// off as well as on.
///
/// **Two arms, one case** (plan D91, blueprint E-4), gated on `caps.usage_mid_turn`, and this is
/// the gate where the *script* changes and **not one assertion does**. Runs (a) and (d) end in
/// [`ScriptEvent::ExpectCancel`] over a transport that reports cost mid-turn — the marker is what
/// makes a build that detected the breach and forgot to cancel fail instead of hang. Over a
/// transport whose cost report *is* the message that ends the turn there is nothing after the
/// breach for the marker to guard: the report and the `done` arrive together, so the script ends
/// in `done { end_turn }` and the cap still has to answer it. That the verdict is unchanged —
/// `stop_reason: cancelled`, `error` then `done` as the last two rows, exactly one `done` — is the
/// whole point of the arm: the transport's own `end_turn` is **withheld** by
/// [`enforce_breach`](crate::record::enforce_breach) rather than recorded beside the cap's
/// (milestone 7 H-4), and this is where a real transport exercises it.
async fn run_cap_breach_cancels_within_one_event<H: CaseHarness, S: WriteStore>(
    harness: &H,
    store: &S,
) {
    // Runs (b) and (c) are under their cap and end in a `done` already, so only (a) and (d) move.
    let unfinished = || {
        if harness.caps().usage_mid_turn {
            ScriptEvent::ExpectCancel
        } else {
            done(StopReason::EndTurn)
        }
    };
    // The deltas `usage_deltas_sum_to_step_usage` uses, so the running spend is 100 then 350: a
    // cap of 300 is crossed by the second report and by no other number in the script.
    let usage = |used: i64, cost: i64| {
        ScriptEvent::Emit(DriverEvent::Usage(UsageEvent {
            cost_micros: Some(cost),
            context_used: Some(used),
            context_size: Some(200_000),
            ..UsageEvent::default()
        }))
    };

    // --- (a) the cap is reached by the second report -------------------------------------------
    // The turn has no `done` of its own: only the cap's cancel can end it, so a build that
    // detected the breach and forgot to cancel reaches `ExpectCancel` and fails as a transport
    // error rather than hanging (`ScriptEvent::ExpectCancel`).
    let breached = play_capped(
        harness,
        store,
        Some(300),
        Script::one_turn(vec![usage(1_000, 100), usage(2_000, 250), unfinished()]),
    )
    .await;

    assert_eq!(
        breached.stop.stop_reason,
        StopReason::Cancelled,
        "run_cap_breach_cancels_within_one_event: a capped turn ends `cancelled`, whatever the \
         transport's own `done` said"
    );
    let breaching = breached
        .log
        .iter()
        .rposition(|row| row.kind == EventKind::Usage)
        .expect("run_cap_breach_cancels_within_one_event: the breaching usage row is persisted");
    assert_eq!(
        breached.summary.cap_breach,
        Some(CapBreach {
            cap_micros: 300,
            spent_micros: 350,
            at: breached.log[breaching].at,
        }),
        "run_cap_breach_cancels_within_one_event: the verdict names the cap, the spend that \
         reached it and the row that did"
    );
    assert_eq!(
        breached.log[breaching + 1].kind,
        EventKind::Error,
        "run_cap_breach_cancels_within_one_event: `within one event of the breach` is the row \
         after the breaching one, with no open call for the cancel to synthesize a result for: \
         {:?}",
        kinds(&breached.log)
    );
    let error = &breached.log[breaching + 1];
    assert_eq!(str_at(error, "code"), Some(CAP_EXCEEDED));
    assert_eq!(
        error.role,
        EventRole::Htui,
        "run_cap_breach_cancels_within_one_event: `htui` authored this row, not the agent"
    );
    assert!(
        str_at(error, "message").is_some_and(|message| message.contains(PER_TOKEN_CAP_RUN)),
        "run_cap_breach_cancels_within_one_event: the message names the setting an operator \
         would go and change: {:?}",
        str_at(error, "message")
    );
    assert_eq!(
        kinds(&breached.log[breached.log.len() - 2..]),
        vec![EventKind::Error, EventKind::Done],
        "run_cap_breach_cancels_within_one_event: the step's last two rows (criterion 8)"
    );
    assert_eq!(
        breached
            .log
            .last()
            .and_then(|row| str_at(row, "stop_reason")),
        Some(StopReason::Cancelled.as_str()),
        "run_cap_breach_cancels_within_one_event: and the last of them says `cancelled`"
    );
    assert_eq!(
        breached
            .log
            .iter()
            .filter(|row| row.kind == EventKind::Done)
            .count(),
        1,
        "run_cap_breach_cancels_within_one_event: exactly one `done` per turn still holds (§4.1) \
         — the transport's own is withheld, not recorded beside this one"
    );
    assert_eq!(
        breached
            .usage_calls
            .last()
            .and_then(|call| call.usage.get("cost_micros").cloned()),
        Some(json!(350)),
        "run_cap_breach_cancels_within_one_event: the breaching row was summed exactly once, so \
         `run_step.usage` is the spend the verdict compared against"
    );

    // --- (b) the same script under a cap it never reaches --------------------------------------
    let under = play_capped(
        harness,
        store,
        Some(1_000),
        Script::one_turn(vec![
            usage(1_000, 100),
            usage(2_000, 250),
            done(StopReason::EndTurn),
        ]),
    )
    .await;
    assert_eq!(under.stop.stop_reason, StopReason::EndTurn);
    assert_eq!(
        under.summary.cap_breach, None,
        "run_cap_breach_cancels_within_one_event: a session under its cap reports no breach"
    );
    assert!(
        !under.log.iter().any(|row| row.kind == EventKind::Error),
        "run_cap_breach_cancels_within_one_event: and writes no `error` row: {:?}",
        kinds(&under.log)
    );

    // --- (c) the same script with no cap at all ------------------------------------------------
    let unbounded = play_capped(
        harness,
        store,
        None,
        Script::one_turn(vec![
            usage(1_000, 100),
            usage(2_000, 250),
            done(StopReason::EndTurn),
        ]),
    )
    .await;
    assert_eq!(unbounded.summary.cap_breach, None);
    assert_eq!(
        without_identity(&unbounded.log, unbounded.step, unbounded.session.as_ref()),
        without_identity(&under.log, under.step, under.session.as_ref()),
        "run_cap_breach_cancels_within_one_event: an absent cap is unbounded, not a cap of zero — \
         the rows are the ones a cap nobody reached produced, down to the `at`s (plan D70)"
    );

    // --- (d) a cap below the first report's own delta -------------------------------------------
    let immediate = play_capped(
        harness,
        store,
        Some(50),
        Script::one_turn(vec![usage(1_000, 100), unfinished()]),
    )
    .await;
    assert_eq!(immediate.stop.stop_reason, StopReason::Cancelled);
    assert_eq!(
        immediate
            .summary
            .cap_breach
            .map(|breach| breach.spent_micros),
        Some(100),
        "run_cap_breach_cancels_within_one_event: a cap under one report's delta is reached by \
         that report, not missed by it (`spent >= cap`)"
    );
    assert_eq!(
        immediate
            .log
            .iter()
            .filter(|row| row.kind == EventKind::Usage)
            .count(),
        1,
        "run_cap_breach_cancels_within_one_event: and nothing after it was pulled: {:?}",
        kinds(&immediate.log)
    );
    assert_eq!(
        kinds(&immediate.log[immediate.log.len() - 2..]),
        vec![EventKind::Error, EventKind::Done]
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
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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

/// Edit proposals dedupe per `(tool_call_id, path)` **per step**, across a flush: one row for the
/// key, updated in place, never written twice (`docs/ANA-4.md` §4.3, §11 criterion 6, plan D6/D77).
///
/// The case name is unchanged and its **script is not** (T46). It used to write the same path twice
/// with nothing in between, which only ever exercised the buffer-scoped half of the rule — and a
/// fake that never flushes mid-call is precisely why a transport-neutral suite reported criterion 6
/// as holding while `agy` was leaving three rows for one file write in production. The assistant
/// chunk between the two writes is the flush: it forces the open buffer out on the kind change,
/// which is what used to clear the dedup index and open a second row.
///
/// The two writes to `src/a.rs` **differ**, so what is proved is the general rule and not the
/// suppression of a byte-identical repeat: the surviving row has to carry the *second* write's
/// content, at the *first* write's `seq` (plan D77 reserves the `seq` at announcement and holds only
/// the row). `src/b.rs` is the control — a different path under the same call is a different key and
/// keeps its own row.
///
/// **Two arms, one case** (plan D80), gated on `caps.edit_proposals`, and the **script is the
/// same**: a transport that cannot propose an edit still sees the three writes, it just surfaces
/// them the only way it can. §4.3 (`:545-546`) says what that is — "surfaces them only as post-hoc
/// `Edit`/`Write` tool calls" — so the second arm asserts zero `edit_proposal` rows and three
/// `tool_call` rows in script order. **Calls do not dedupe**, and that is the point of the arm:
/// the same file written twice is two things that happened, where two *proposals* for one file are
/// one pending change. A transport that deduped its calls to look like it had passed the first arm
/// would be losing a row the user needs.
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
        // The flush the live defect needed. Any of §4.1's triggers would do — live it was the
        // permission park/answer — and a kind change is the one every binding can produce.
        chunk("thinking about it ", "m1"),
        proposal("src/a.rs", "@@ second", Some(true)),
        done(StopReason::EndTurn),
    ]);
    let scrubber = scrubber();
    let (chat, mut session, caps) = open_case(harness, store, script, false).await;
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
    if !caps.edit_proposals {
        edits_surface_as_post_hoc_tool_calls(&log);
        return;
    }
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
    // `contains`, not `==` (amended in milestone 3). ANA-4 §4.3: no ACP shape carries a diff, so a
    // transport that speaks the protocol synthesizes the unified text from `oldText`/`newText` and
    // the script's marker survives *inside* it. Demanding equality would demand that a transport
    // put the script's string on the wire verbatim, which is a fact about the fake and not about
    // the contract. What the case is for — one row per `(tool_call_id, path)`, updated in place,
    // carrying the last proposal's content and its `accepted` — is unchanged.
    assert!(
        str_at(edits[0], "diff").is_some_and(|diff| diff.contains("@@ second")),
        "edit_proposal_deduped_per_call_and_path: the buffered row is updated in place: {:?}",
        str_at(edits[0], "diff")
    );
    assert_eq!(
        edits[0].payload.get("accepted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(str_at(edits[1], "path"), Some("src/b.rs"));

    // The reservation, which is the half a row count cannot see: the surviving row sits where it
    // was **announced**, ahead of the row whose flush used to break the rule, even though it was
    // written after it. Replay reads by `seq`, so this is what keeps the transcript's order true.
    let flushed = log
        .iter()
        .find(|row| row.kind == EventKind::AssistantText)
        .expect("edit_proposal_deduped_per_call_and_path: the chunk between the writes is a row");
    assert!(
        edits[0].seq < flushed.seq && edits[1].seq < flushed.seq,
        "edit_proposal_deduped_per_call_and_path: an `edit_proposal` reserves its `seq` at the \
         announcement: {} and {} against {}",
        edits[0].seq,
        edits[1].seq,
        flushed.seq
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("a case's log is short")).collect::<Vec<_>>(),
        "edit_proposal_deduped_per_call_and_path: and `seq` is still gapless 0..n"
    );
}

/// `edit_proposal_deduped_per_call_and_path` over a transport that reports
/// `caps.edit_proposals: false` (plan D80, `docs/ANA-4.md` §4.3 `:545-546`).
///
/// The same three writes, surfaced the only way a transport with no proposal channel can surface
/// them: as post-hoc `Edit`/`Write` tool calls, after the fact rather than before it. So the
/// dedup rule has nothing to apply to — **zero** `edit_proposal` rows, the negative this arm owes
/// — and what the log must carry instead is one `edit`-kinded call per write, in script order,
/// each with its result. Three, not two: a write that already happened is not a pending change
/// waiting to be superseded, so the second write to `src/a.rs` is its own row and a transport that
/// collapsed it would be hiding an edit from the user.
fn edits_surface_as_post_hoc_tool_calls(log: &[SessionEvent]) {
    assert!(
        !log.iter().any(|row| row.kind == EventKind::EditProposal),
        "edit_proposal_deduped_per_call_and_path: a transport that reports no proposal channel \
         writes no `edit_proposal` row: {:?}",
        kinds(log)
    );

    let calls: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolCall)
        .collect();
    assert_eq!(
        calls.len(),
        3,
        "edit_proposal_deduped_per_call_and_path: one call per write, and calls do not dedupe: \
         {:?}",
        kinds(log)
    );
    assert!(
        calls
            .iter()
            .all(|row| str_at(row, "tool_kind") == Some(ToolKind::Edit.as_str())),
        "edit_proposal_deduped_per_call_and_path: a write is an `edit`-kinded call: {:?}",
        calls
            .iter()
            .map(|row| str_at(row, "tool_kind"))
            .collect::<Vec<_>>()
    );
    let paths: Vec<Option<&str>> = calls
        .iter()
        .map(|row| {
            row.payload
                .get("locations")
                .and_then(|locations| locations.get(0))
                .and_then(|location| location.get("path"))
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(
        paths,
        vec![Some("src/a.rs"), Some("src/b.rs"), Some("src/a.rs")],
        "edit_proposal_deduped_per_call_and_path: each call names the file it wrote, in script \
         order"
    );

    let results: Vec<&SessionEvent> = log
        .iter()
        .filter(|row| row.kind == EventKind::ToolResult)
        .collect();
    assert_eq!(
        results.len(),
        3,
        "edit_proposal_deduped_per_call_and_path: every call is closed by its own result"
    );
    assert!(
        calls.iter().all(|call| results
            .iter()
            .any(|result| result.tool_call_id == call.tool_call_id)),
        "edit_proposal_deduped_per_call_and_path: and each result names the call it closed"
    );

    let flushed = log
        .iter()
        .find(|row| row.kind == EventKind::AssistantText)
        .expect("edit_proposal_deduped_per_call_and_path: the chunk between the writes is a row");
    assert!(
        calls[1].seq < flushed.seq && flushed.seq < calls[2].seq,
        "edit_proposal_deduped_per_call_and_path: the text the agent wrote between the second and \
         third write sits between them: {} < {} < {}",
        calls[1].seq,
        flushed.seq,
        calls[2].seq
    );
    assert_eq!(
        log.iter().map(|row| row.seq).collect::<Vec<_>>(),
        (0..i32::try_from(log.len()).expect("a case's log is short")).collect::<Vec<_>>(),
        "edit_proposal_deduped_per_call_and_path: and `seq` is still gapless 0..n"
    );
    assert_eq!(
        log.last().map(|row| row.kind),
        Some(EventKind::Done),
        "edit_proposal_deduped_per_call_and_path: `done` closes the log"
    );
}

/// Flush trigger 4: a coalesced run is cut at [`CHUNK_FLUSH_BYTES`] rather than buffered
/// unboundedly, and the cut is a function of byte counts alone - no timer, so replay stays
/// deterministic (`docs/ANA-4.md` §4.1, §11 criterion 2).
async fn chunk_flush_at_16kib<H: CaseHarness, S: WriteStore>(harness: &H, store: &S) {
    let piece = "x".repeat(1024);
    let mut events: Vec<ScriptEvent> = (0..17).map(|_| chunk(&piece, "m1")).collect();
    events.push(done(StopReason::EndTurn));
    let scrubber = scrubber();
    let (chat, mut session, _caps) =
        open_case(harness, store, Script::one_turn(events), false).await;
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
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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
    // **Present, and `null` or a positive integer** (blueprint E-5). Not `== 1`, which is what this
    // asserted while ACP was the only protocol in the tree: a transport that negotiates no version
    // — a CLI dialect has none to negotiate (plan D84) — would have failed a case about the
    // *banner*, for a key that is honestly absent. `null` is that transport saying so, which is a
    // different fact from a transport that forgot the key, and the presence check is what still
    // separates them. Both existing bindings report `1` and are unchanged by this.
    let version = banner
        .payload
        .get("body")
        .and_then(|body| body.get("protocol_version"))
        .expect(
            "session_banner_is_first_other_row: the banner carries `protocol_version`, whether or \
             not this transport negotiated one",
        );
    assert!(
        version.is_null() || version.as_i64().is_some_and(|version| version > 0),
        "session_banner_is_first_other_row: the banner reports the negotiated protocol version, or \
         `null` where the transport negotiates none: {version:?}"
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
    let (chat, mut session, _caps) = open_case(harness, store, script, true).await;
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
    let (chat, mut session, _caps) = open_case(harness, store, script, false).await;
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
    use crate::driver::{AgentDriver, DriverCaps};
    use crate::fake::{FAKE_AGENT_NAME, FakeDriver};
    use htui_core::store::MemStore;

    /// The in-crate binding, so the guard below needs no integration test to exist.
    #[derive(Debug)]
    struct FakeHarness;

    impl CaseHarness for FakeHarness {
        fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
            Box::new(FakeDriver::scripted(script))
        }

        fn caps(&self) -> DriverCaps {
            FakeDriver::full_caps()
        }
    }

    /// The fake reporting the capability profile `docs/ANA-4.md` §4.3 gives the CLI transport, so
    /// the second arm of every capability gate is **run** and not merely compiled (plan D80, D91).
    ///
    /// Not a fourth binding and not a case: it plays [`CASES`] through the same [`run_case`], and
    /// its whole job is that the gate chooses the right column and the right column then holds.
    /// The fake honours its own [`DriverCaps`] (`fake.rs` rule 6), so what it puts on the wire for
    /// these three is the degradation the ANA describes rather than the script verbatim.
    ///
    /// `crates/htui-agent/tests/extensibility.rs` makes the same run from the other direction and
    /// is the stronger statement — its profile comes from a **registry row** it read back out of a
    /// store, so nothing in this file chose it.
    #[derive(Debug)]
    struct DegradedHarness;

    impl DegradedHarness {
        /// `full_caps` minus the three the CLI transport does not have. `permission_requests` and
        /// `edit_proposals` are §4.3's; `usage_mid_turn` is D91's.
        const fn profile() -> DriverCaps {
            DriverCaps {
                permission_requests: false,
                edit_proposals: false,
                usage_mid_turn: false,
                ..FakeDriver::full_caps()
            }
        }
    }

    impl CaseHarness for DegradedHarness {
        fn driver(&self, script: Script) -> Box<dyn AgentDriver> {
            Box::new(FakeDriver::new(FAKE_AGENT_NAME, Self::profile(), script))
        }

        fn caps(&self) -> DriverCaps {
            Self::profile()
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

    /// Every case again, over a transport missing three capabilities (plan D80, D91).
    ///
    /// Every name here is already in [`CASES`] and stays there: what this asserts is that the arm
    /// a missing capability selects is a *statement that holds*, not an escape hatch — a transport
    /// that says it has no permission channel writes no permission row, a refusal its own policy
    /// made still closes the call it settled, a write it can only report after the fact is still
    /// one row per write, and a per-run cap still cancels a turn whose cost arrives on the message
    /// that ends it.
    #[tokio::test]
    async fn a_transport_without_a_capability_takes_the_other_arm() {
        for name in CASES {
            run_case(name, &DegradedHarness, &MemStore::demo()).await;
        }
    }
}
