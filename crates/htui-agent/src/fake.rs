//! The scripted transport (feature `test-support`, plan MOD-2 D7).
//!
//! [`FakeDriver`] is a real implementation of the [`AgentDriver`] / [`AgentSession`] seam whose
//! wire is a [`Script`]. It exists so [`crate::conformance`]'s one `CASES` list has something to
//! run against in milestone 1, and so milestone 3's ACP transport inherits a *behavioural*
//! specification rather than a prose one: everything the fake does here that is not a plain
//! "replay the next scripted event" is a rule the real transports owe too.
//!
//! Those rules, in one place:
//!
//! 1. **A session banner.** The first event of every session is
//!    `other { update: "session_started" }` carrying the agent-side session id (`docs/ANA-4.md`
//!    §4.4, `docs/ANA-2.md` §4.8: resuming a promoted step is a query for that row).
//! 2. **Permission requests park.** After a [`ScriptEvent::ParkPermission`] the session emits
//!    nothing further until [`AgentSession::answer_permission`] or [`AgentSession::cancel`]. A
//!    real transport *awaits* at that point; the fake returns [`DriverError::Transport`] instead,
//!    because a test that pulls while parked would otherwise hang rather than fail.
//! 3. **A denied or cancelled call gets a synthesized result.** ACP emits no `tool_result` for a
//!    rejected call, so the driver invents one - `failed` plus a `terminal_reason` - and the
//!    scripted result for that call, if any, is dropped (`docs/ANA-4.md` §4.3 "Tool-call terminal
//!    states", §11 criterion 5).
//! 4. **`cancel` answers everything.** Every parked request is answered `cancelled`, every tool
//!    call still open gets its synthesized `failed` result, and the turn closes with
//!    `done { stop_reason: "cancelled" }`.
//! 5. **One `done` per turn precedes the next follow-up.** [`AgentSession::send_follow_up`]
//!    before the turn's `done` is a protocol error, not a queued prompt.
//!
//! **Deterministic by construction.** The capture time of the *n*-th envelope is
//! [`crate::conformance::epoch`] plus *n* milliseconds and the agent-side session id is derived
//! from `SessionSpec.step_id`, so replaying one script twice produces one byte-identical row set -
//! the property `docs/ANA-4.md` §11 criterion 2 asks for. A wall-clock reading anywhere in here
//! would silently cost that criterion.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::TimeDelta;
use htui_core::model::{Agent, AgentBox, EventKind};
use serde_json::{Value, json};

use crate::conformance::{Script, ScriptEvent, Turn, epoch};
use crate::driver::{
    AgentDriver, AgentSession, AgentSessionRef, DriverCaps, DriverFuture, PermissionAnswer,
    PermissionRequestId, SessionSpec,
};
use crate::error::DriverError;
use crate::event::{
    DoneEvent, DriverEnvelope, DriverEvent, OtherEvent, PermissionOptionKind,
    PermissionRequestEvent, StopReason, TerminalReason, ToolResultEvent, ToolResultStatus,
};
use crate::registry::{DriverFactory, TransportBuilder};

// The banner's `other.update`, published here too: the fake writes the same banner every transport
// writes, and it is one string with one definition (`crate::event`).
pub use crate::event::SESSION_STARTED;

/// The name [`FakeDriver::scripted`] gives its driver, and the `cli.stream` value milestone 2
/// registers it under (adapter id `cli/fake`, plan D12 / assumption A2).
pub const FAKE_AGENT_NAME: &str = "fake";

/// The version the banner reports, so a snapshot of the banner never moves with the crate.
const FAKE_AGENT_VERSION: &str = "0.0.0-fake";

// ---------------------------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------------------------

/// A transport whose wire is a [`Script`].
///
/// The script sits in a slot rather than in a field because milestone 2's `FakeAdapter` loads it
/// through an `Arc<Mutex<Option<Script>>>` and [`AgentDriver::start`] drains it (plan D12): one
/// driver plays one script exactly once, which is what a real process does.
#[derive(Debug)]
pub struct FakeDriver {
    name: String,
    caps: DriverCaps,
    script: Mutex<Option<Script>>,
}

impl FakeDriver {
    /// A driver named `fake` that advertises every capability, for the conformance suite.
    #[must_use]
    pub fn scripted(script: Script) -> Self {
        Self::new(FAKE_AGENT_NAME, Self::full_caps(), script)
    }

    /// A driver with a caller-chosen name and capability profile.
    ///
    /// Milestone 2's registry proof builds one of these from an `agent` row, so the caps are the
    /// row's and not the fake's (plan D12).
    #[must_use]
    pub fn new(name: impl Into<String>, caps: DriverCaps, script: Script) -> Self {
        Self {
            name: name.into(),
            caps,
            script: Mutex::new(Some(script)),
        }
    }

    /// Every session predicate true: the fake is the reference transport, so a case is never
    /// skipped for want of a capability.
    #[must_use]
    pub const fn full_caps() -> DriverCaps {
        DriverCaps {
            permission_requests: true,
            edit_proposals: true,
            plans: true,
            thoughts: true,
            follow_up_in_session: true,
            resume: true,
            usage: true,
            // A script puts a `usage` event wherever it likes, including mid-turn, so no case is
            // skipped for want of this one (plan D91).
            usage_mid_turn: true,
            // The one exception: the fake is the reference transport for *sessions*, and
            // `authenticate` is proven through the refusing default body instead (plan MOD-21
            // D10). A fake that claimed a login it does not perform would make the contract
            // case that compares predicate against operation pass for the wrong reason.
            authenticate: false,
        }
    }
}

impl AgentDriver for FakeDriver {
    fn name(&self) -> &str {
        &self.name
    }

    fn caps(&self) -> DriverCaps {
        self.caps
    }

    fn start<'a>(
        &'a self,
        spec: SessionSpec,
        prompt: String,
    ) -> DriverFuture<'a, Box<dyn AgentSession>> {
        // The guard is taken and dropped before the future is built: a `MutexGuard` is not `Send`
        // and `DriverFuture` is, so holding one across the `async move` would not compile. The
        // "never hold a lock across an await" rule is enforced here by the type system.
        let taken =
            self.script.lock().map(|mut slot| slot.take()).map_err(|_| {
                DriverError::Transport("the fake's script slot is poisoned".to_owned())
            });
        let name = self.name.clone();
        Box::pin(async move {
            if prompt.is_empty() {
                return Err(DriverError::Transport(
                    "a session starts with a prompt".to_owned(),
                ));
            }
            let script = taken?.ok_or_else(|| {
                DriverError::Transport("the fake's script has already been played".to_owned())
            })?;
            Ok(Box::new(FakeSession::open(&name, &spec, script)) as Box<dyn AgentSession>)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------------------------

/// One playthrough of one [`Script`]: a [`Turn`] per prompt or follow-up.
#[derive(Debug)]
pub struct FakeSession {
    session_ref: AgentSessionRef,
    retain_raw: bool,
    /// Turns not yet opened; [`AgentSession::send_follow_up`] opens the next one.
    turns: VecDeque<Turn>,
    /// The open turn's remaining script.
    queue: VecDeque<ScriptEvent>,
    /// Events the session owes the caller before it reads the script again: the banner, and every
    /// result and `done` the transport synthesized.
    pending: VecDeque<DriverEvent>,
    /// Requests emitted and not yet answered. Non-empty means the session is stalled.
    parked: Vec<PermissionRequestEvent>,
    /// Tool calls emitted with no result yet, in emission order.
    open_calls: Vec<String>,
    /// Tool calls that already have a result, scripted or synthesized. A second one is dropped.
    settled_calls: BTreeSet<String>,
    /// Envelopes handed out so far: the deterministic clock, and the `raw` sequence number.
    clock: i64,
    /// Whether the open turn is still short of its `done`.
    turn_open: bool,
    /// Whether the session is finished: no further event, no further accepted call.
    ended: bool,
}

impl FakeSession {
    /// Opens a session on turn 0 of the script, with the banner already queued.
    fn open(name: &str, spec: &SessionSpec, script: Script) -> Self {
        // Derived from the step, never minted: two replays of one script must agree on every
        // persisted byte, and the banner carries this id into a row (§11 criterion 2).
        let session_ref = AgentSessionRef::new(format!("fake-{}", spec.step_id));
        let mut turns: VecDeque<Turn> = script.turns.into();
        let queue: VecDeque<ScriptEvent> = turns
            .pop_front()
            .map(|turn| turn.events.into())
            .unwrap_or_default();
        let banner = DriverEvent::Other(OtherEvent {
            update: SESSION_STARTED.to_owned(),
            body: json!({
                "session_id": session_ref.as_str(),
                "protocol_version": 1,
                "agent_name": name,
                "agent_version": FAKE_AGENT_VERSION,
                "models": [],
            }),
        });
        Self {
            session_ref,
            retain_raw: spec.retain_raw,
            turns,
            queue,
            pending: VecDeque::from(vec![banner]),
            parked: Vec::new(),
            open_calls: Vec::new(),
            settled_calls: BTreeSet::new(),
            clock: 0,
            turn_open: true,
            ended: false,
        }
    }

    /// Wraps one event as the envelope the recorder consumes, stamping the deterministic clock and
    /// synthesizing a wire message when the spec asked for one.
    fn envelope(&mut self, event: DriverEvent) -> DriverEnvelope {
        let n = self.clock;
        self.clock += 1;
        self.note(&event);
        let raw = self.retain_raw.then(|| {
            json!({
                "fake_wire": EventKind::from(&event).as_str(),
                "n": n,
                "body": payload_of(&event),
            })
        });
        DriverEnvelope {
            event,
            raw,
            at: epoch() + TimeDelta::milliseconds(n),
        }
    }

    /// Updates the call and turn bookkeeping an emitted event implies.
    fn note(&mut self, event: &DriverEvent) {
        match event {
            DriverEvent::ToolCall(call) => {
                if !self.open_calls.contains(&call.tool_call_id)
                    && !self.settled_calls.contains(&call.tool_call_id)
                {
                    self.open_calls.push(call.tool_call_id.clone());
                }
            }
            DriverEvent::ToolResult(result) => self.settle_bookkeeping(&result.tool_call_id),
            DriverEvent::Done(_) => self.turn_open = false,
            _ => {}
        }
    }

    /// Marks a call answered without emitting anything.
    fn settle_bookkeeping(&mut self, call: &str) {
        self.settled_calls.insert(call.to_owned());
        self.open_calls.retain(|open| open != call);
    }

    /// Queues the `failed` result `htui` invents for a call the protocol will never answer
    /// (`docs/ANA-4.md` §4.3). A call that already has a result is left alone.
    fn synthesize_result(&mut self, call: &str, reason: TerminalReason) {
        if self.settled_calls.contains(call) {
            return;
        }
        self.settle_bookkeeping(call);
        self.pending
            .push_back(DriverEvent::ToolResult(ToolResultEvent {
                tool_call_id: call.to_owned(),
                status: ToolResultStatus::Failed,
                output: None,
                locations: Vec::new(),
                terminal_reason: Some(reason),
            }));
    }
}

impl AgentSession for FakeSession {
    fn session_ref(&self) -> Option<&AgentSessionRef> {
        Some(&self.session_ref)
    }

    fn next_event<'a>(&'a mut self) -> DriverFuture<'a, Option<DriverEnvelope>> {
        Box::pin(async move {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(self.envelope(event)));
            }
            if self.ended {
                return Ok(None);
            }
            if let Some(parked) = self.parked.first() {
                return Err(DriverError::Transport(format!(
                    "permission request `{}` is parked: answer or cancel it before pulling again",
                    parked.request_id
                )));
            }
            if !self.turn_open {
                return Ok(None);
            }
            loop {
                let Some(step) = self.queue.pop_front() else {
                    return Err(DriverError::Transport(
                        "the script's turn ran out before its `done`".to_owned(),
                    ));
                };
                match step {
                    ScriptEvent::Emit(DriverEvent::ToolResult(result))
                        if self.settled_calls.contains(&result.tool_call_id) =>
                    {
                        // The call was already answered by a rejection or a cancel, and a
                        // protocol that then sent its own result would be sending a second one.
                    }
                    ScriptEvent::Emit(event) => return Ok(Some(self.envelope(event))),
                    ScriptEvent::ParkPermission(request) => {
                        self.parked.push(request.clone());
                        return Ok(Some(self.envelope(DriverEvent::PermissionRequest(request))));
                    }
                    ScriptEvent::ExpectCancel => {
                        return Err(DriverError::Transport(
                            "script marker `expect_cancel`: this turn ends only by `cancel`"
                                .to_owned(),
                        ));
                    }
                }
            }
        })
    }

    fn send_follow_up<'a>(&'a mut self, text: String) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            if text.is_empty() {
                return Err(DriverError::Transport(
                    "a follow-up carries text".to_owned(),
                ));
            }
            if self.turn_open {
                return Err(DriverError::Transport(
                    "a follow-up before the turn's `done`: exactly one `done` per turn precedes \
                     the next accepted follow-up"
                        .to_owned(),
                ));
            }
            let Some(turn) = self.turns.pop_front() else {
                self.ended = true;
                return Err(DriverError::Closed);
            };
            self.queue = turn.events.into();
            self.turn_open = true;
            Ok(())
        })
    }

    fn answer_permission<'a>(
        &'a mut self,
        request_id: PermissionRequestId,
        answer: PermissionAnswer,
    ) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            if self.ended {
                return Err(DriverError::Closed);
            }
            let Some(index) = self
                .parked
                .iter()
                .position(|parked| parked.request_id == request_id)
            else {
                return Err(DriverError::Transport(format!(
                    "no parked permission request `{request_id}`"
                )));
            };
            let request = self.parked.remove(index);
            let reason = match &answer {
                PermissionAnswer::Cancelled => Some(TerminalReason::Cancelled),
                PermissionAnswer::Selected(option_id) => {
                    let Some(option) = request.options.iter().find(|o| &o.id == option_id) else {
                        return Err(DriverError::Transport(format!(
                            "`{request_id}` offers no option `{option_id}`"
                        )));
                    };
                    matches!(
                        option.kind,
                        PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                    )
                    .then_some(TerminalReason::Rejected)
                }
            };
            if let Some(reason) = reason
                && let Some(call) = request.tool_call_id.clone()
            {
                self.synthesize_result(&call, reason);
            }
            Ok(())
        })
    }

    fn cancel<'a>(&'a mut self, grace: Duration) -> DriverFuture<'a, ()> {
        Box::pin(async move {
            // The fake owns no child process, so the grace window has nothing to wait for: a real
            // transport sends `session/cancel` (or SIGINT) and kills the tree once it elapses.
            let _ = grace;
            if self.ended {
                return Ok(());
            }
            for request in core::mem::take(&mut self.parked) {
                if let Some(call) = request.tool_call_id {
                    self.synthesize_result(&call, TerminalReason::Cancelled);
                }
            }
            for call in core::mem::take(&mut self.open_calls) {
                self.synthesize_result(&call, TerminalReason::Cancelled);
            }
            self.queue.clear();
            self.turns.clear();
            // Only a turn that is **open** is owed a `done`. Cancelling between turns ends the
            // session, not a turn, and a second `done` would be a row for a turn that never
            // started — `docs/ANA-4.md` §4.1 says exactly one per turn. (Milestone 3: the ACP
            // transport guards this the same way, and the two transports must not differ about
            // what the rows say.)
            if self.turn_open {
                self.pending.push_back(DriverEvent::Done(DoneEvent {
                    stop_reason: StopReason::Cancelled,
                }));
                self.turn_open = false;
            }
            self.ended = true;
            Ok(())
        })
    }
}

/// The payload document of one event, for the synthesized `raw` blob.
///
/// A real transport carries the message it actually received; the fake carries the payload it is
/// about to produce, which is enough for `docs/ANA-4.md` §11 criterion 4 ("the same rows with
/// `raw` populated") and gives the scrubber something in `raw` to mask.
fn payload_of(event: &DriverEvent) -> Value {
    let encoded = match event {
        DriverEvent::AssistantChunk(inner) | DriverEvent::ThoughtChunk(inner) => {
            serde_json::to_value(inner)
        }
        DriverEvent::ToolCall(inner) => serde_json::to_value(inner),
        DriverEvent::ToolResult(inner) => serde_json::to_value(inner),
        DriverEvent::EditProposal(inner) => serde_json::to_value(inner),
        DriverEvent::PermissionRequest(inner) => serde_json::to_value(inner),
        DriverEvent::Plan(inner) => serde_json::to_value(inner),
        DriverEvent::Usage(inner) => serde_json::to_value(inner),
        DriverEvent::Error(inner) => serde_json::to_value(inner),
        DriverEvent::Done(inner) => serde_json::to_value(inner),
        DriverEvent::Other(inner) => serde_json::to_value(inner),
    };
    encoded.unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// The registry adapter (plan D12, assumption A2)
// ---------------------------------------------------------------------------------------------

/// The fake reached through the **production** factory, as adapter id `cli/fake`.
///
/// A registry row asks for it the same way it would ask for a real stream adapter — by naming
/// `settings.cli.stream` — which is what makes milestone 2's `R-AGT-5` proof structural rather
/// than decorative: the unknown agent's row travels the production path, and only the last hop
/// differs. `fake` extends `docs/ANA-4.md` §5.2's `cli.stream` vocabulary under `test-support`
/// alone, so no shipped build can be pointed at it (assumption A2).
///
/// The script lives in a shared slot: the harness loads one, [`TransportBuilder::build`] hands it
/// to a fresh [`FakeDriver`], and the driver drains it on `start`.
#[derive(Debug, Clone, Default)]
pub struct FakeAdapter {
    script: Arc<Mutex<Option<Script>>>,
}

impl FakeAdapter {
    /// An adapter with no script loaded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the script the next [`TransportBuilder::build`] will hand to its driver.
    ///
    /// # Panics
    /// If the slot's lock is poisoned, which in a test means an earlier case panicked while
    /// holding it — reporting that here beats reporting an empty script three assertions later.
    pub fn load(&self, script: Script) {
        *self.script.lock().expect("the script slot is not poisoned") = Some(script);
    }
}

impl TransportBuilder for FakeAdapter {
    fn build(
        &self,
        agent: &Agent,
        _on_box: Option<&AgentBox>,
        caps: DriverCaps,
    ) -> Result<Box<dyn AgentDriver>, DriverError> {
        let script = self
            .script
            .lock()
            .map_err(|_| DriverError::Transport("the fake's script slot is poisoned".to_owned()))?
            .take()
            .ok_or_else(|| {
                DriverError::Transport("no script was loaded for this session".to_owned())
            })?;
        // The driver takes its name and capabilities from the **row**, not from the fake, so a
        // test asserting on `driver.name()` is asserting about the registry rather than about a
        // constant in this file. (This comment deliberately does not name the agent that test
        // uses: `tests/extensibility.rs` sweeps the tree for it, and a mention here would make the
        // proof circular.)
        Ok(Box::new(FakeDriver::new(agent.name.clone(), caps, script)))
    }
}

impl DriverFactory {
    /// A factory with the test-only adapters registered: `cli/fake`, and nothing else.
    ///
    /// The production registrations are milestone 3's `acp` and milestone 8's
    /// `cli/claude_stream_json`.
    ///
    /// Its adapter is unreachable from outside, so this is the factory for asserting *routing* —
    /// which ids exist, which row is refused. A caller that needs to drive a session registers its
    /// own [`FakeAdapter`] so it can [`load`](FakeAdapter::load) a script into it.
    #[must_use]
    pub fn with_test_support() -> Self {
        let mut factory = Self::new();
        factory.register("cli/fake", Box::new(FakeAdapter::new()));
        factory
    }
}
