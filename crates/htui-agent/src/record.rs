//! The session recorder of `docs/ANA-4.md` §4.1 (plan MOD-2 D6): the one writer of a step's
//! `session_event` log.
//!
//! A [`Recorder`] turns the driver's per-chunk event stream into the row set ANA-9 §4.3 asks for.
//! It owns six things no one else may own:
//!
//! 1. **Coalescing.** A contiguous run of [`DriverEvent::AssistantChunk`] /
//!    [`DriverEvent::ThoughtChunk`] becomes one `assistant_text` / `thought` row. The flush
//!    triggers are §4.1's, in evaluation order: a different event variant, a different
//!    [`TextChunk::message_id`](crate::event::TextChunk::message_id), [`DriverEvent::Done`], [`CHUNK_FLUSH_BYTES`] of accumulated text,
//!    and the session end ([`Recorder::finish`]). There is deliberately **no idle-time flush**: a
//!    timer would make the persisted row set a function of scheduling, and §11 criterion 2 asks
//!    that the same fixture replayed twice yield identical rows.
//! 2. **`seq` and `turn`.** `seq` is gapless from 0 with exactly one writer, so
//!    `PRIMARY KEY (run_step_id, seq)` is a backstop and not the allocator. `turn` is 0 at the
//!    `prompt` row and increments once per [`Recorder::record_follow_up`] - refused or not - and
//!    every row of a turn carries that turn's number, including the `done` that closes it. A
//!    flush the store refuses **commits nothing**: `seq` does not advance, and the rows it
//!    numbered stay owed inside the recorder until a later flush writes them at exactly those
//!    numbers. Gaplessness is therefore a property of the log, not merely of the counter.
//! 3. **The prompt digest.** `sha256` over the assembled prompt text, computed once, written both
//!    as the `digest` key of the `prompt` payload and through
//!    [`WriteStore::set_step_usage`]`(step, usage, Some(digest))`. Later usage writes for the same
//!    step pass `None`, as that method's contract says. Passing `Some` rather than ANA-5 §4.4's
//!    `None` is the plan's X8 ruling, and it stands until milestone 9.
//! 4. **Scrub, then persist, then the UI.** Every payload and every `raw` blob goes through the
//!    [`Scrubber`] at capture, and every payload goes through it a **second** time over the
//!    assembled row at the flush - the pass that catches a secret no single chunk carried
//!    (`R-SEC-3`, ANA-4 §9). The two passes are not the same guarantee, and the difference is
//!    visible at the render channel, which is offered the *capture-time* envelope, one frame per
//!    chunk:
//!
//!    - **The store's guarantee.** No persisted `payload` or `raw` carries a known secret, and a
//!      row whose assembled text still looks credential-shaped is not written at all.
//!    - **The render channel's guarantee, which is weaker.** No secret *contained in one chunk*
//!      is offered to the chat tab, and nothing the scrubber refused is offered at all. A secret
//!      **split across two chunks** is masked in the persisted row and appears in cleartext in
//!      the two frames that already went out. That is deliberate: handing the tab the flushed row
//!      instead would batch streaming text to flush granularity and cost the chat tab its
//!      streaming feel, to close a local-only, same-operator, non-persistent exposure. Milestone
//!      3 owns the chat tab, and any stronger render-path rule is its decision.
//!
//!    The channel is bounded and written with `try_send`, so a paused chat tab can never stall
//!    the agent; drops are counted ([`Recorder::dropped`]) rather than waited on, because a
//!    dropped render frame is recoverable from the store and a dropped row is not.
//! 5. **The passive quota latch** (`docs/ANA-4.md` §7 `:1131-1135`, plan D66-D68). The recorder is
//!    the only place that sees every `usage` row, so it is where `agent_box.quota` is refreshed:
//!    it normalizes the row's vendor rate-limit blob into §7's document and writes two columns
//!    through [`WriteStore::set_agent_box_quota`]. Opt-in ([`Recorder::with_quota_latch`]),
//!    best-effort — a refused allowance write never fails a turn — and silent when a row has
//!    nothing to say, which is what keeps a turn's first, blob-less report from erasing the last
//!    one.
//! 6. **The per-run cap** (`docs/ANA-4.md` §7 `:1143-1150`, §11 criterion 8, plan D69-D70). The
//!    same fact that makes the recorder the latch's home makes it the cap's: it is the only place
//!    that sees every `usage` row. What it does with the cap is **detect** — [`Recorder::record`]
//!    returns a [`CapBreach`] on the one row whose running spend reached
//!    [`RunCap::micros`] — and the cancel is [`enforce_breach`]'s, called by a layer that holds the
//!    session. §7 as written puts the cancel here too; it cannot be here, because a `Recorder` with
//!    a session in it is a recorder the conformance suite could not drive with no transport at all,
//!    and plan D69 is the maintainer's record of that amendment.
//!
//! **Fail-closed.** When the scrubber returns [`Unmasked`] the recorder drops that row entirely,
//! writes `error { code: "scrub_residue", message: "<rule> at <path>" }` with role `htui` in its
//! place - so `seq` keeps no gap - and reports [`RecordError::Unmasked`] from
//! [`Recorder::finish`]. It does **not** mark the step failed: that is the step owner's status
//! write (`finish_chat_run` in milestone 3, MOD-4 for graph steps), and milestone 1 has no session
//! to attach it to.
//!
//! Deliberately absent: the offline path. The recorder writes through a [`WriteStore`] and
//! nothing else; choosing `append_pending` over the store is milestone 4 (plan D8, D16).

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use htui_core::model::{
    AgentId, Billing, BoxId, EventKind, EventRole, PER_TOKEN_CAP_RUN, QuotaSource, SessionEvent,
    StepId, UsageTotals, quota::normalize,
};
use htui_core::scrub::{Scrubber, Unmasked};
use htui_core::store::{StoreError, WriteStore};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::driver::{AgentSession, PermissionRequestId};
use crate::error::DriverError;
use crate::event::{DoneEvent, DriverEnvelope, DriverEvent, ErrorEvent, StopReason};

/// Flush trigger 4: the accumulated text a coalesced run may reach before it is cut.
///
/// A bound, not a target - one pathological turn cannot buffer unboundedly - and a byte count
/// rather than a duration, which is what keeps replay deterministic.
pub const CHUNK_FLUSH_BYTES: usize = 16 * 1024;

/// `error.code` of the row the recorder writes in place of a payload the scrubber refused.
const SCRUB_RESIDUE: &str = "scrub_residue";

/// `error.code` of the row the recorder writes when the per-run cap is reached
/// (`docs/ANA-4.md` §7 `:1143-1150`, §11 criterion 8).
///
/// Public because it is what a reader of the log matches on: the chat tab renders the row, and
/// `crates/htui/src/agent_worker.rs`'s own cases assert the code rather than the message, which is
/// prose and may be reworded.
pub const CAP_EXCEEDED: &str = "cap_exceeded";

/// What the passive latch of `docs/ANA-4.md` §7 (`:1131-1135`) needs (plan D66-D68): which
/// `agent_box` row to write, and the two row-side facts the document carries.
///
/// `source` is `agent.settings.quota.source` and `billing` is `agent.billing` — both read off the
/// `agent` row, never derived from its name (`R-AGT-5`). The recorder holds the latch rather than
/// looking either up, because it has no registry read and must not grow one: it is what the
/// conformance suite drives with no transport and no registry at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLatch {
    /// `agent_box.agent_id` of the row to latch into.
    pub agent_id: AgentId,
    /// `agent_box.box_id` of the row to latch into: the box this chat runs on.
    pub box_id: BoxId,
    /// How this row's transport reports an allowance, as the row itself declares.
    pub source: QuotaSource,
    /// `agent.billing`, which the §7 document carries so a reader knows what `spend` means.
    pub billing: Billing,
}

/// The per-run cap (plan D70) and the grace the cancel it triggers may take (plan D69).
///
/// `micros` is `project.settings.per_token_cap_run`, in **USD micros** — the unit
/// `UsageTotals::cost_micros` already sums, so the comparison is integer arithmetic and not a
/// rounding argument. A cap of `0` is a real cap and cancels on the first row that reports any USD
/// cost; an *absent* cap is [`Recorder::with_run_cap`] never being called.
///
/// The grace rides here rather than on [`pump`] so that seam keeps its signature (blueprint H-9):
/// the worker passes its own `CANCEL_GRACE`, the conformance suite passes zero, and eighteen call
/// sites do not move to carry a value only one of them has an opinion about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCap {
    /// The cap in USD micros.
    pub micros: i64,
    /// How long the cancel this cap triggers may wait before the process tree is killed.
    pub grace: Duration,
}

/// The verdict [`Recorder::record`] returns **once** per session: the row that took the running USD
/// spend to or past the cap.
///
/// `spent >= cap` is the same comparison `R-AGT-8`'s "the per-token cap is already reached" makes
/// (`docs/ANA-4.md` §7), so a cap and a spend that are equal is a breach and not a near miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapBreach {
    /// The cap that was reached, in USD micros.
    pub cap_micros: i64,
    /// The session's running spend at the breaching row, in USD micros.
    pub spent_micros: i64,
    /// The breaching row's capture time, which the `error` row that follows it carries.
    pub at: DateTime<Utc>,
}

/// `TIMESTAMPTZ` keeps microseconds, so a capture time is truncated to microseconds before it is
/// written; otherwise a `MemStore` row and its `PgStore` twin would differ in a column neither
/// backend changed (the rule `crates/htui-core/src/model/run.rs` follows for `ChatRunSpec::mint`).
const TIMESTAMPTZ_DIGITS: u16 = 6;

/// Capture time for a row `htui` authors itself, at the precision the column keeps.
///
/// The rows a driver produces take their `at` from [`DriverEnvelope::at`]. A prompt, a follow-up
/// and a permission answer have **no envelope**, so their caller supplies the instant instead: an
/// implicit clock reading here would make `session_event.at` wall-clock dependent and break
/// `docs/ANA-4.md` §11 criterion 2 ("the same fixture replayed twice yields identical rows") for
/// every script that records a prompt - which, by plan D7, is every conformance case. The
/// truncation stays inside the recorder so a caller cannot forget it.
fn stamp(at: DateTime<Utc>) -> DateTime<Utc> {
    at.trunc_subsecs(TIMESTAMPTZ_DIGITS)
}

wire_enum!(
    /// `permission_answer.by` (ANA-9 §4.3): who chose the option.
    ///
    /// Also decides the row's `session_event.role`: a user's answer is the user's row, a policy
    /// rule's answer is `htui`'s.
    #[derive(Default)]
    AnsweredBy {
        /// A human picked the option in the chat tab (`R-TUI-6`).
        #[default]
        User => "user",
        /// An `agent.settings.permission` rule or a cancellation answered it (§4.3).
        Policy => "policy",
    }
);

impl AnsweredBy {
    /// The `session_event.role` an answer by this author is written under.
    #[must_use]
    pub const fn role(self) -> EventRole {
        match self {
            Self::User => EventRole::User,
            Self::Policy => EventRole::Htui,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Errors and the summary
// ---------------------------------------------------------------------------------------------

/// Why a recording call failed.
///
/// Separate from [`DriverError`] because the recorder is not a transport: it can fail on a store
/// write, on scrub residue and on a payload it cannot encode, and on nothing else. `From` in the
/// driver's direction exists so [`pump`] can return one error type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// The store refused the write. **Nothing was committed**: `seq` did not advance, and the
    /// rows that flush had numbered stay owed inside the recorder, to be written at those same
    /// numbers by the next flush - a retry, `finish`, or milestone 4's offline sink. The caller
    /// decides whether to retry, go offline (milestone 4) or fail the step; dropping the recorder
    /// is what loses the rows.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Something credential-shaped survived scrubbing. Reported once, from
    /// [`Recorder::finish`]; the offending row was already dropped and replaced by a
    /// `scrub_residue` error row when it happened.
    #[error(transparent)]
    Unmasked(#[from] Unmasked),
    /// A payload could not be turned into JSON. Only a transport that produced a
    /// non-representable value (a `NaN` cost, say) can cause this.
    ///
    /// A payload masking made unreadable is deliberately **not** this error: see
    /// [`Recorder::record`].
    #[error("recorder could not encode a payload: {0}")]
    Encode(String),
}

impl From<RecordError> for DriverError {
    fn from(error: RecordError) -> Self {
        match error {
            RecordError::Store(store) => Self::Store(store),
            RecordError::Unmasked(unmasked) => Self::Scrub(unmasked),
            RecordError::Encode(message) => Self::Transport(message),
        }
    }
}

/// What a finished recorder wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecorderSummary {
    /// Rows the store accepted. Lower than `seq` only if a replay found rows already stored.
    pub rows: usize,
    /// The next unused `seq`, i.e. the number of rows the recorder authored.
    pub seq: i32,
    /// Turns recorded: 0 with no prompt, otherwise the last `turn` plus one.
    pub turns: i32,
    /// The `sha256` of the prompt text, if a prompt was recorded.
    pub prompt_digest: Option<String>,
    /// The document written to `run_step.usage`: the sum of the step's `usage` deltas.
    pub usage: Value,
    /// Render frames the bounded UI channel could not take.
    pub dropped: usize,
    /// The one per-run cap breach of this session, if it had one (plan D70).
    ///
    /// A property of the *session*, not of the last row: it is set once, by the row that reached
    /// the cap, and a later row never replaces it.
    pub cap_breach: Option<CapBreach>,
}

// ---------------------------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------------------------

/// A row on its way to the store: everything but the `seq`, which is stamped at flush time so a
/// dropped row leaves no gap.
#[derive(Debug, Clone)]
struct PendingRow {
    kind: EventKind,
    role: EventRole,
    tool_call_id: Option<String>,
    payload: Value,
    /// The verbatim wire messages behind this row; empty unless `retain_raw`. A coalesced row has
    /// as many as it has chunks.
    raw: Vec<Value>,
    at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------------------------

/// The one writer of a step's `session_event` log (`docs/ANA-4.md` §4.1, plan D6).
///
/// Borrows its store and its scrubber: one recorder lives for one `run_step`, inside the task that
/// pumps that step's session, and it holds no lock across an `.await` because it holds no lock at
/// all.
pub struct Recorder<'a, S: WriteStore> {
    store: &'a S,
    scrubber: &'a dyn Scrubber,
    step: StepId,
    retain_raw: bool,
    ui: Option<mpsc::Sender<DriverEnvelope>>,

    /// Rows written but not yet flushed. Holds rows of exactly one [`EventKind`], because any
    /// variant change flushes first.
    buffer: Vec<PendingRow>,
    /// The kind currently buffered, or `None` when the buffer is empty.
    buffer_kind: Option<EventKind>,
    /// The open text run's grouping key, meaningful only while `buffer_kind` is a chunk kind.
    open_message_id: Option<String>,
    /// `(tool_call_id, path)` to its index in `buffer`, for the edit-proposal dedup rule.
    edits: BTreeMap<(Option<String>, String), usize>,
    /// Rows a flush numbered and the store then refused. They keep their `seq` and go out ahead
    /// of the buffer on the next flush, which is what makes a failed append cost a retry rather
    /// than a hole in the log.
    unflushed: Vec<SessionEvent>,

    next_seq: i32,
    turn: i32,
    turns: i32,
    prompt_digest: Option<String>,
    /// The digest still owed to `run_step.prompt_digest`; taken by the first `set_step_usage`.
    digest_pending: Option<String>,
    usage: UsageTotals,
    usage_dirty: bool,
    /// Plan D66-D68: `None` records no quota — a test, an offline chat, or a backend that refused
    /// the latch once and will not be asked again this session.
    quota_latch: Option<QuotaLatch>,
    /// The last vendor blob a `usage` row of this session carried, as scrubbed and persisted.
    ///
    /// Kept so a later row without one refreshes the spend figure without erasing the windows: a
    /// turn's first report carries no `_meta` (blueprint H-3), and a document assembled from that
    /// row alone would say "no windows" about an allowance nobody re-reported.
    last_quota_raw: Option<Value>,
    /// Plan D70: the per-run cap this session is bounded by, or `None` for an unbounded run.
    run_cap: Option<RunCap>,
    /// Set by the first breach and never cleared: a later `usage` row reports no second verdict,
    /// because the session it would cancel is already being closed.
    cap_breached: Option<CapBreach>,
    residue: Option<Unmasked>,
    dropped: usize,
    rows: usize,
}

impl<S: WriteStore> core::fmt::Debug for Recorder<'_, S> {
    /// Prints the recorder's own state. The store is not printed: `WriteStore` does not require
    /// `Debug`, and a store's contents are not what a recorder log is about.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Recorder")
            .field("step", &self.step)
            .field("retain_raw", &self.retain_raw)
            .field("ui", &self.ui.is_some())
            .field("buffered", &self.buffer.len())
            .field("buffer_kind", &self.buffer_kind)
            .field("unflushed", &self.unflushed.len())
            .field("next_seq", &self.next_seq)
            .field("turn", &self.turn)
            .field("dropped", &self.dropped)
            .field("residue", &self.residue)
            // Whether a latch is configured, not which row it names: the identity of the row is
            // not what a recorder log is about either.
            .field("quota_latch", &self.quota_latch.is_some())
            .field("run_cap", &self.run_cap)
            .field("cap_breached", &self.cap_breached)
            .finish()
    }
}

impl<'a, S: WriteStore> Recorder<'a, S> {
    /// Opens a recorder over one `run_step`.
    ///
    /// `retain_raw` is `SessionSpec.retain_raw`, itself `project.settings.keep_raw_events`. `ui`
    /// is the chat tab's bounded channel; `None` records with no live rendering, which is what
    /// every test and every headless run does.
    pub fn new(
        store: &'a S,
        scrubber: &'a dyn Scrubber,
        step: StepId,
        retain_raw: bool,
        ui: Option<mpsc::Sender<DriverEnvelope>>,
    ) -> Self {
        Self {
            store,
            scrubber,
            step,
            retain_raw,
            ui,
            buffer: Vec::new(),
            buffer_kind: None,
            open_message_id: None,
            edits: BTreeMap::new(),
            unflushed: Vec::new(),
            next_seq: 0,
            turn: 0,
            turns: 0,
            prompt_digest: None,
            digest_pending: None,
            usage: UsageTotals::default(),
            usage_dirty: false,
            quota_latch: None,
            last_quota_raw: None,
            run_cap: None,
            cap_breached: None,
            residue: None,
            dropped: 0,
            rows: 0,
        }
    }

    /// Records `agent_box.quota` for one row as well as the step's log (plan D66-D68).
    ///
    /// A builder rather than a sixth parameter to [`Recorder::new`] because most recorders have no
    /// latch: every conformance case that is not about quota, every offline chat (a buffered writer
    /// refuses registry writes, so the worker answers `None` before the first row rather than
    /// discovering it on one), and every test of the other twelve rules.
    #[must_use]
    pub fn with_quota_latch(mut self, latch: QuotaLatch) -> Self {
        self.quota_latch = Some(latch);
        self
    }

    /// Bounds this run by a per-run cap (`docs/ANA-4.md` §7 `:1143-1150`, plan D69-D70).
    ///
    /// A builder for [`Recorder::with_quota_latch`]'s reason and one more: an absent cap has to
    /// mean *unbounded* rather than zero (plan D70), and a parameter would make every caller state
    /// an opinion about a setting most projects do not have.
    ///
    /// What the recorder does with it is **detect**, and nothing else: [`Recorder::record`] returns
    /// the verdict and the layer holding the session performs the cancel
    /// ([`enforce_breach`]). That is plan D69's amendment to §7, and the reason for it is that a
    /// recorder with a session in it would be a recorder the conformance suite could not drive with
    /// no transport at all.
    #[must_use]
    pub fn with_run_cap(mut self, cap: RunCap) -> Self {
        self.run_cap = Some(cap);
        self
    }

    /// The per-run cap this recorder was configured with, if any.
    ///
    /// [`enforce_breach`] reads it for the grace: the cancel is performed by a layer that holds the
    /// session and not the configuration, and this is how the two meet without [`pump`] growing a
    /// parameter (blueprint H-9).
    #[must_use]
    pub const fn run_cap(&self) -> Option<RunCap> {
        self.run_cap
    }

    /// Render frames the bounded UI channel could not take, so far.
    #[must_use]
    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    /// The step this recorder writes.
    #[must_use]
    pub const fn step(&self) -> StepId {
        self.step
    }

    /// Records the assembled initial prompt: `seq = 0`, `turn = 0`, role `htui` (ANA-9 §4.3).
    ///
    /// The digest is `sha256` over the **scrubbed** text (ANA-5 §4.7's "scrub before digest"), so
    /// two prompts that differ only in a masked secret share a digest, and it is written straight
    /// through to `run_step.prompt_digest` as well as into the payload.
    ///
    /// Call once, before any other row: it is what opens turn 0.
    ///
    /// `at` is the row's `session_event.at`, truncated here to the column's microseconds. It is a
    /// parameter and not an internal clock reading because a prompt has no envelope to take an
    /// instant from, and a wall-clock stamp would make the row set a function of when it ran
    /// (`docs/ANA-4.md` §11 criterion 2).
    ///
    /// # Errors
    /// [`RecordError::Store`] when the append or the digest write fails,
    /// [`RecordError::Encode`] when `sections` cannot be scrubbed as JSON.
    pub async fn record_prompt(
        &mut self,
        text: &str,
        sections: Value,
        at: DateTime<Utc>,
    ) -> Result<(), RecordError> {
        let at = stamp(at);
        self.turn = 0;
        self.turns = 1;
        let mut payload = json!({ "text": text, "sections": sections });
        match self.scrubber.scrub(&mut payload) {
            Ok(()) => {}
            Err(unmasked) => return self.refuse(unmasked, at).await,
        }
        let scrubbed = payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let digest = format!("{:x}", Sha256::digest(scrubbed.as_bytes()));
        payload["digest"] = Value::String(digest.clone());
        self.prompt_digest = Some(digest.clone());
        self.digest_pending = Some(digest);

        self.flush().await?;
        self.push(PendingRow {
            kind: EventKind::Prompt,
            role: EventRole::Htui,
            tool_call_id: None,
            payload,
            raw: Vec::new(),
            at,
        });
        self.flush().await?;
        self.sync_step().await
    }

    /// Records a user follow-up and opens the next turn (ANA-9 §4.3: role `user`, payload `text`).
    ///
    /// `at` is the row's `session_event.at`, for the reason [`Recorder::record_prompt`] gives.
    ///
    /// The turn opens **whether or not the text survives scrubbing**. A refused follow-up is
    /// still a follow-up: the agent was asked something new, and the `scrub_residue` row that
    /// stands in for it - and every agent row that answers it - belongs to the new turn. Numbering
    /// them under the turn that just ended would make `turn` a function of the scrubber.
    ///
    /// The order is flush, then increment: the previous turn's buffered rows have to be stamped
    /// with the number they were recorded under, and a buffered row takes its `turn` at flush
    /// time.
    ///
    /// # Errors
    /// [`RecordError::Store`] when the append fails.
    pub async fn record_follow_up(
        &mut self,
        text: &str,
        at: DateTime<Utc>,
    ) -> Result<(), RecordError> {
        let at = stamp(at);
        let mut payload = json!({ "text": text });
        self.flush().await?;
        self.turn += 1;
        self.turns = self.turn + 1;
        match self.scrubber.scrub(&mut payload) {
            Ok(()) => {}
            Err(unmasked) => return self.refuse(unmasked, at).await,
        }
        self.push(PendingRow {
            kind: EventKind::FollowUp,
            role: EventRole::User,
            tool_call_id: None,
            payload,
            raw: Vec::new(),
            at,
        });
        self.flush().await
    }

    /// Records the answer to a parked permission request (ANA-9 §4.3: `request_id`, `option_id`,
    /// `by`).
    ///
    /// `cancelled` is an added key, not a renamed one: a cancellation answers every outstanding
    /// request with no option at all (`docs/ANA-4.md` §4.3), and the row has to say so.
    ///
    /// `at` is the row's `session_event.at`, for the reason [`Recorder::record_prompt`] gives.
    ///
    /// # Errors
    /// [`RecordError::Store`] when the append fails.
    pub async fn record_permission_answer(
        &mut self,
        request_id: &PermissionRequestId,
        option_id: Option<&str>,
        by: AnsweredBy,
        cancelled: bool,
        at: DateTime<Utc>,
    ) -> Result<(), RecordError> {
        let at = stamp(at);
        let mut payload = json!({
            "request_id": request_id.as_str(),
            "option_id": option_id,
            "by": by.as_str(),
            "cancelled": cancelled,
        });
        match self.scrubber.scrub(&mut payload) {
            Ok(()) => {}
            Err(unmasked) => return self.refuse(unmasked, at).await,
        }
        self.flush().await?;
        self.push(PendingRow {
            kind: EventKind::PermissionAnswer,
            role: by.role(),
            tool_call_id: None,
            payload,
            raw: Vec::new(),
            at,
        });
        self.flush().await
    }

    /// Records one driver envelope: scrub, then persist, then `try_send` to the UI.
    ///
    /// "Persist" is the buffering step, which writes to the store as soon as one of the §4.1
    /// triggers fires. The UI never sees a payload the scrubber refused, and a full UI channel
    /// never costs a row - only a render frame, which [`Recorder::dropped`] counts.
    ///
    /// A payload the scrubber refuses is **not** an error here: the row is dropped, a
    /// `scrub_residue` row takes its place, and [`Recorder::finish`] is what reports it.
    ///
    /// A payload masking made *unreadable* is not an error either, and not a residue: nothing
    /// survived unmasked, so the masked document is persisted as it stands, under the kind the
    /// unscrubbed event decided, standing alone rather than coalesced, and no frame goes to the
    /// UI. A `SessionSpec.env` value equal to a closed-vocabulary wire string (`read`,
    /// `end_turn`, `user`, …) or to a field name is the way that happens; assumption A7 accepts
    /// those false positives in masking, and this is where the recorder keeps one from becoming a
    /// failed turn.
    ///
    /// **The return value is the cap verdict** (plan D69-D70): `Some` on the one `usage` row whose
    /// running spend reached [`RunCap::micros`], and `None` on every other row of every session,
    /// including every later row of a session that already breached. A caller that holds the
    /// session answers it with [`enforce_breach`]; a caller that does not — a replay, an uploader,
    /// a test asserting on rows — may ignore it, which is why it is not `#[must_use]`.
    ///
    /// # Errors
    /// [`RecordError::Store`] when a flush fails, [`RecordError::Encode`] when the event cannot
    /// be represented as JSON at all.
    pub async fn record(
        &mut self,
        envelope: DriverEnvelope,
    ) -> Result<Option<CapBreach>, RecordError> {
        let kind = EventKind::from(&envelope.event);
        let (event, payload, raw) = match self.scrub_envelope(&envelope) {
            Ok(triple) => triple,
            Err(ScrubRefusal::Unmasked(unmasked)) => {
                // A row that was never persisted spent nothing: a refused payload is not a `usage`
                // report, whatever its event said it was.
                return self.refuse(unmasked, envelope.at).await.map(|()| None);
            }
            Err(ScrubRefusal::Encode(message)) => return Err(RecordError::Encode(message)),
        };
        let Some(event) = event else {
            return self
                .record_unreadable(kind, payload, raw, envelope.at)
                .await;
        };
        let mut scrubbed = DriverEnvelope {
            event,
            raw,
            at: envelope.at,
        };

        // Triggers 1 and 2: a different variant, or a different grouping key inside the same one.
        let chunk = match &scrubbed.event {
            DriverEvent::AssistantChunk(text) | DriverEvent::ThoughtChunk(text) => Some(text),
            _ => None,
        };
        let stale_group = chunk.is_some_and(|text| self.open_message_id != text.message_id);
        if self
            .buffer_kind
            .is_some_and(|open| open != kind || stale_group)
        {
            self.flush().await?;
        }

        let mut reached_bound = false;
        // The cap verdict, set by the one `usage` row that reaches the cap (plan D70).
        let mut breach = None;
        // Which buffered row this envelope became, so its `raw` lands on *that* row rather than
        // on whichever row happens to be last (the dedup arm updates a row further back).
        let target = match (&scrubbed.event, chunk) {
            (_, Some(text)) => {
                if self.buffer_kind.is_none() {
                    self.open_message_id.clone_from(&text.message_id);
                    self.push(PendingRow {
                        kind,
                        role: EventRole::Agent,
                        tool_call_id: None,
                        payload: json!({ "text": text.text }),
                        raw: Vec::new(),
                        at: scrubbed.at,
                    });
                } else if let Some(open) = self.buffer.last_mut()
                    && let Some(Value::String(open_text)) = open.payload.get_mut("text")
                {
                    open_text.push_str(&text.text);
                }
                reached_bound = self
                    .buffer
                    .last()
                    .and_then(|open| open.payload.get("text"))
                    .and_then(Value::as_str)
                    .is_some_and(|open_text| open_text.len() >= CHUNK_FLUSH_BYTES);
                self.buffer.len() - 1
            }
            (DriverEvent::EditProposal(proposal), _) => {
                let key = (proposal.tool_call_id.clone(), proposal.path.clone());
                // Dedup per `(tool_call_id, path)`: the buffered row is updated before the flush,
                // never written twice (ANA-4 §4.3, plan D6). The row *is* the update once it is
                // updated - it carries the update's `diff` and `accepted` - so it carries the
                // update's capture time as well, and (below) the update's `raw`.
                if let Some(&index) = self.edits.get(&key) {
                    let open = &mut self.buffer[index];
                    open.payload = payload;
                    open.at = scrubbed.at;
                    index
                } else {
                    self.edits.insert(key, self.buffer.len());
                    self.push(PendingRow {
                        kind,
                        role: EventRole::Agent,
                        tool_call_id: proposal.tool_call_id.clone(),
                        payload,
                        raw: Vec::new(),
                        at: scrubbed.at,
                    });
                    self.buffer.len() - 1
                }
            }
            (event, _) => {
                if matches!(event, DriverEvent::Usage(_)) {
                    // The scrubbed document that is about to be persisted, not the typed event:
                    // the uploader (`htui-store`) has only these bytes, and summing them on both
                    // sides is what makes an uploaded step's `run_step.usage` indistinguishable
                    // from an online one (plan D36).
                    self.usage.add_payload(&payload);
                    self.usage_dirty = true;
                    self.latch_quota(&payload, scrubbed.at).await;
                    // After the latch, so a breached run still publishes the allowance the row it
                    // died on reported: the cancel below is what ends the session, and the column
                    // three readers render should not lose the last thing the vendor said.
                    breach = self.check_cap(scrubbed.at);
                }
                self.push(PendingRow {
                    kind,
                    role: EventRole::Agent,
                    tool_call_id: tool_call_id_of(event),
                    payload,
                    raw: Vec::new(),
                    at: scrubbed.at,
                });
                self.buffer.len() - 1
            }
        };

        // The row's own copy of `raw`. Cloned only when a live chat tab is about to be handed the
        // envelope; with no UI - every headless run and every test - the blob is moved, because a
        // `raw` blob is the largest thing a row carries.
        let raw = if self.ui.is_some() {
            scrubbed.raw.clone()
        } else {
            scrubbed.raw.take()
        };
        if let Some(raw) = raw
            && let Some(open) = self.buffer.get_mut(target)
        {
            open.raw.push(raw);
        }

        // Triggers 3 and 4: the turn ended, or the open run reached the byte bound.
        if matches!(scrubbed.event, DriverEvent::Done(_)) || reached_bound {
            self.flush().await?;
            self.sync_step().await?;
        }

        self.send_ui(scrubbed);
        Ok(breach)
    }

    /// Trigger 5: closes the session's log.
    ///
    /// Flushes what is buffered, writes `run_step.usage` (and the digest, if no usage report has
    /// carried it yet), and reports the summary.
    ///
    /// # Errors
    /// [`RecordError::Unmasked`] when any payload of this session failed to scrub - the rows are
    /// still written, minus the refused ones - and [`RecordError::Store`] when the final writes
    /// fail.
    pub async fn finish(mut self) -> Result<RecorderSummary, RecordError> {
        self.flush().await?;
        self.sync_step().await?;
        if let Some(unmasked) = self.residue.clone() {
            return Err(RecordError::Unmasked(unmasked));
        }
        Ok(RecorderSummary {
            rows: self.rows,
            seq: self.next_seq,
            turns: self.turns,
            prompt_digest: self.prompt_digest.clone(),
            usage: self.usage.to_value(),
            dropped: self.dropped,
            cap_breach: self.cap_breached,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------------------------

    /// Buffers one row, remembering the kind the buffer now holds.
    fn push(&mut self, row: PendingRow) {
        self.buffer_kind = Some(row.kind);
        self.buffer.push(row);
    }

    /// Writes what is buffered and closes the open run.
    ///
    /// The final scrub happens here rather than at capture, because a secret split across two
    /// chunks only exists once the run is assembled; masking is idempotent, so scrubbing the
    /// already-scrubbed pieces again costs a pass and changes nothing.
    ///
    /// **Failure-atomic.** The batch is numbered into a local vector, and `next_seq` / `rows`
    /// move only once the store has said `Ok`. A store that refuses keeps the numbered batch in
    /// `unflushed`, so the next flush offers those rows again, ahead of anything buffered since
    /// and at the very `seq` they were given: with one writer, a failed append costs a retry and
    /// never a hole. Re-offering is safe because `append_events` skips a `(run_step_id, seq)` it
    /// already holds, so a partially applied batch cannot be written twice.
    async fn flush(&mut self) -> Result<(), RecordError> {
        self.buffer_kind = None;
        self.open_message_id = None;
        self.edits.clear();
        if self.buffer.is_empty() && self.unflushed.is_empty() {
            return Ok(());
        }

        let pending = core::mem::take(&mut self.buffer);
        let mut rows = core::mem::take(&mut self.unflushed);
        rows.reserve(pending.len());
        for mut row in pending {
            let outcome = self.scrubber.scrub(&mut row.payload);
            let row = match outcome {
                Ok(()) => row,
                Err(unmasked) => {
                    self.note_residue(&unmasked);
                    residue_row(&unmasked, row.at)
                }
            };
            rows.push(self.event_row(row));
        }
        let mut seq = self.next_seq;
        for row in &mut rows {
            row.seq = seq;
            seq += 1;
        }
        match self.store.append_events(&rows).await {
            Ok(written) => {
                self.next_seq = seq;
                self.rows += written;
                Ok(())
            }
            Err(error) => {
                self.unflushed = rows;
                Err(RecordError::Store(error))
            }
        }
    }

    /// Writes `run_step.usage`, carrying the prompt digest on the first call only (the contract of
    /// [`WriteStore::set_step_usage`], plan D15(b)).
    async fn sync_step(&mut self) -> Result<(), RecordError> {
        if !self.usage_dirty && self.digest_pending.is_none() {
            return Ok(());
        }
        let usage = self.usage.to_value();
        let digest = self.digest_pending.take();
        self.store.set_step_usage(self.step, usage, digest).await?;
        self.usage_dirty = false;
        Ok(())
    }

    /// The passive latch of `docs/ANA-4.md` §7 (`:1131-1135`, plan D66-D68): after every `usage`
    /// row, publish `agent_box.quota` when the row has something to say — and never fail the turn.
    ///
    /// **Passive** means there is no query path: `Settings > Refresh agents` re-runs the probes and
    /// a handshake reports no allowance, so the only time `htui` learns anything about an allowance
    /// is while a run is in flight. That is what `R-AGT-7`'s "refreshed per run" asks for, and it
    /// costs nothing beyond the rows the session was writing anyway.
    ///
    /// **Nothing to say** is the rule blueprint H-3 turns on, and [`Recorder::nothing_to_say`] is
    /// where it is stated: a row that has learned nothing worth publishing leaves the standing
    /// document alone instead of replacing it with an emptier one. Once a blob has been seen it is
    /// remembered (`last_quota_raw`), so a later report with no `_meta` refreshes the spend and
    /// keeps the windows.
    ///
    /// **Best-effort** is plan D68: a failed allowance write is logged and dropped. `R-HIS-1`'s
    /// durability is carried by the `usage` rows, which are buffered offline and re-derive the
    /// figure after upload; failing a turn because an advisory number could not be stored would
    /// trade the requirement for the courtesy. Two of the four outcomes also switch the latch off
    /// for the session, because they will not change: an unreachable registry
    /// (`REGISTRY_ON_SERVER_ONLY` — the offline refusal, which the worker normally answers before
    /// the first row) and a missing `agent_box` row, which no later report of this session probes
    /// into existence.
    ///
    /// The document is assembled from the **scrubbed payload** that is about to be persisted, not
    /// from the typed event, for `UsageTotals::add_payload`'s reason: the row's bytes are what a
    /// second reader would see, and a latch that published more than the row carries would put a
    /// value in a column that no log explains.
    ///
    /// Blueprint H-17, accepted rather than fixed: this awaits a store write inside
    /// [`Recorder::record`], before the row is buffered, so a slow server makes every `usage` row
    /// cost a round trip. It is not on the UI task (`R-NF-3` holds), a `usage` row is rare — one
    /// per report — and the write is one indexed two-column `UPDATE`. If it ever matters the latch
    /// moves to [`Recorder::sync_step`]'s cadence: the same document, written less often.
    async fn latch_quota(&mut self, payload: &Value, at: DateTime<Utc>) {
        let Some(latch) = self.quota_latch else {
            return;
        };
        if let Some(raw) = payload.get("quota").filter(|raw| raw.is_object()) {
            self.last_quota_raw = Some(raw.clone());
        }
        if self.nothing_to_say(latch.source) {
            return;
        }
        let quota = normalize(
            latch.source,
            latch.billing,
            self.last_quota_raw.as_ref(),
            self.usage.cost_micros,
            at,
        );
        match self
            .store
            .set_agent_box_quota(latch.agent_id, latch.box_id, quota.to_value(), at)
            .await
        {
            Ok(()) => {}
            Err(StoreError::Unreachable(reason)) => {
                tracing::info!(
                    %reason,
                    "quota is not latched on this backend; the usage rows are (plan D68)"
                );
                self.quota_latch = None;
            }
            Err(StoreError::NotFound { .. }) => {
                tracing::debug!(
                    "no agent_box row for this agent on this box; nothing to latch into until it \
                     is probed"
                );
                self.quota_latch = None;
            }
            Err(err) => {
                tracing::warn!(%err, "the quota latch failed; the turn continues (plan D68)");
            }
        }
    }

    /// Whether this session has learned nothing worth publishing yet — the rule that makes
    /// blueprint H-3 impossible rather than merely unlikely.
    ///
    /// A row whose declared source **reports an allowance** has said nothing until that source's
    /// first blob arrives, whatever it has spent. A turn's first `usage_update` carries no `_meta`
    /// (`tests/acp_map.rs`), so a latch that fired on it would publish `windows: []` over the
    /// windows the column already holds — the empty document erasing the last session's, which is
    /// the failure H-3 names. A spend figure is not worth that: it is on the `usage` rows either
    /// way, and the next report of this turn carries the blob.
    ///
    /// A row whose source **reports nothing** ([`QuotaSource::None`] — the seeded live-ACP row,
    /// and every transport milestone 8 has yet to teach) has no allowance to wait for, so its
    /// spend is the whole document: something to say once anything has been spent, and nothing
    /// before that. That is why such a row's quota column reads `—` until it costs something
    /// (plan D65) rather than reading a document full of nulls.
    const fn nothing_to_say(&self, source: QuotaSource) -> bool {
        match source {
            QuotaSource::None => self.usage.cost_micros.is_none(),
            QuotaSource::AcpMetaRateLimit
            | QuotaSource::CliRateLimitEvent
            | QuotaSource::CliStatusLine => self.last_quota_raw.is_none(),
        }
    }

    /// The cap comparison of `docs/ANA-4.md` §7 (`:1143-1150`, plan D70): `Some` exactly once per
    /// session, on the row that took the running spend to or past the cap.
    ///
    /// Three `?`s and an `if`, and each of them is a rule:
    ///
    /// - **no cap** is unbounded, not a cap of zero (plan D70's own sentence about an absent key);
    /// - **no USD spend** cannot reach a cap denominated in USD micros, so a context-only report —
    ///   `agy`'s whole shape and the first rows of every `claude` turn — never breaches, and a cap
    ///   of `0` waits for the first *costed* row rather than firing on the first row (blueprint
    ///   H-5). `usage.cost_micros` is `None` until a report carries a USD cost, which is also what
    ///   a non-USD cost leaves it as (`acp::map`), so a cap has nothing to compare against there
    ///   either and says so instead of guessing a conversion;
    /// - **once**: a session that has breached is already being cancelled, and a second verdict
    ///   would send a second cancel and write a second pair of closing rows.
    ///
    /// `>=` and not `>`: `R-AGT-8` reads "the per-token cap is **already reached**", so a spend
    /// equal to the cap is a breach.
    fn check_cap(&mut self, at: DateTime<Utc>) -> Option<CapBreach> {
        let cap = self.run_cap?;
        let spent = self.usage.cost_micros?;
        if self.cap_breached.is_some() || spent < cap.micros {
            return None;
        }
        let breach = CapBreach {
            cap_micros: cap.micros,
            spent_micros: spent,
            at,
        };
        self.cap_breached = Some(breach);
        Some(breach)
    }

    /// The two closing rows of `docs/ANA-4.md` §7 and §11 criterion 8, in this order and last:
    /// `error { code: "cap_exceeded", message }` with role `htui` — as every row the recorder
    /// authors itself — then `done { stop_reason: "cancelled" }`.
    ///
    /// `transport_done` is the transport's **own** turn end, taken off the stream by
    /// [`enforce_breach`] and deliberately not recorded as it stood: its `stop_reason` may say
    /// `end_turn` when the agent finished inside the grace window (blueprint H-4), and criterion 8
    /// says the last row says `cancelled`. Its `at` and its `raw` are kept on the row written in
    /// its place, so `keep_raw_events` still explains where that row came from; a `raw` the
    /// scrubber refuses is **dropped** rather than refusing the row, because the row itself is
    /// `htui`'s prose and dropping a debugging blob is cheaper than losing the log's last two rows.
    /// Exactly one `done` per turn (§4.1) therefore still holds.
    ///
    /// Both rows are buffered **before** the flush that writes them, which is what makes a store
    /// that refuses cost a retry rather than criterion 8 (blueprint H-2): the breaching `usage`
    /// row, the `error` and the `done` are owed together, in that order, at the `seq` they were
    /// given, and [`Recorder::finish`] writes them.
    ///
    /// Public because [`enforce_breach`] is the shared sequence and lives outside the type; a
    /// caller that reaches for this directly is writing its own cancel, and plan D69 says there
    /// should be exactly one.
    ///
    /// # Errors
    /// [`RecordError::Store`] when the flush or the `run_step.usage` write fails,
    /// [`RecordError::Encode`] when the two rows cannot be represented as JSON — which is
    /// unreachable for two structs of `String`s, and is not worth a `panic` to prove.
    pub async fn record_cap_breach(
        &mut self,
        breach: CapBreach,
        transport_done: Option<DriverEnvelope>,
    ) -> Result<(), RecordError> {
        let error = ErrorEvent {
            code: CAP_EXCEEDED.to_owned(),
            // "Estimated", because it is: ANA-4 §7 frames a client-side cap as a guard rail and
            // never as a billing statement, and the risk table asks that the row say so.
            //
            // **Both figures carry their micros**, not only the cap's. Four decimal places is the
            // `usage_line` precedent (`chat/transcript.rs`), and at sub-cent amounts it renders a
            // spend of 350 and a cap of 300 as `$0.0003` twice — a row that says the cap was
            // reached and cannot say by how much. The micros are the number an operator would go
            // and edit, so they belong in the row that told them to.
            message: format!(
                "per-run cap reached: an estimated ${:.4} ({} micros) spent against a cap of \
                 ${:.4} (project.settings.{PER_TOKEN_CAP_RUN} = {} micros); the session was \
                 cancelled",
                breach.spent_micros as f64 / 1e6,
                breach.spent_micros,
                breach.cap_micros as f64 / 1e6,
                breach.cap_micros,
            ),
        };
        let done = DoneEvent {
            stop_reason: StopReason::Cancelled,
        };
        let error_payload = encode(&error)?;
        let done_payload = encode(&done)?;

        let (done_at, done_raw) = match transport_done {
            Some(envelope) => {
                let raw = match (self.retain_raw, envelope.raw) {
                    (true, Some(mut raw)) => self.scrubber.scrub(&mut raw).ok().map(|()| raw),
                    _ => None,
                };
                (envelope.at, raw)
            }
            None => (breach.at, None),
        };

        self.push(PendingRow {
            kind: EventKind::Error,
            role: EventRole::Htui,
            tool_call_id: None,
            payload: error_payload,
            raw: Vec::new(),
            at: breach.at,
        });
        self.push(PendingRow {
            kind: EventKind::Done,
            role: EventRole::Htui,
            tool_call_id: None,
            payload: done_payload,
            raw: done_raw.into_iter().collect(),
            at: done_at,
        });
        self.flush().await?;
        self.sync_step().await?;

        // Both rows reach the chat tab, which is where a breach becomes visible to the user (plan
        // D73: the `error` row *is* the visibility, and the tab renders it today). `raw: None` -
        // the render path has never carried one.
        self.send_ui(DriverEnvelope {
            event: DriverEvent::Error(error),
            raw: None,
            at: breach.at,
        });
        self.send_ui(DriverEnvelope {
            event: DriverEvent::Done(done),
            raw: None,
            at: done_at,
        });
        Ok(())
    }

    /// Persists a row whose masked payload no longer reads back as its own event type.
    ///
    /// Masking is what broke it, so **nothing leaked and nothing is dropped**. An
    /// `agent.settings.env` value that happens to equal - or to be a substring of - a
    /// closed-vocabulary wire string (`read`, `end_turn`, `allow_once`, `user`, …) masks that
    /// value like any other occurrence, and a value that equals a field *name* masks the key
    /// itself; either way the document stops being a `T` while staying a perfectly safe, fully
    /// masked JSON row. Assumption A7 accepts those short-value false positives in masking, and
    /// this is where the recorder refuses to let one become anything more than a false positive:
    /// the row keeps the [`EventKind`] the **unscrubbed** event already decided, and carries the
    /// masked document verbatim.
    ///
    /// What the row loses is the two things that read the typed event: it cannot be coalesced into
    /// an open run, nor deduped against an earlier proposal, so the open run is flushed first and
    /// the row stands alone. `tool_call_id` is taken from the masked document, which is why the
    /// column can never carry an unmasked id. No frame reaches the chat tab, which has no scrubbed
    /// event to render; the row is there when it reloads.
    ///
    /// It is **not** latched either. [`Recorder::latch_quota`] assembles the §7 allowance document
    /// out of the payload, and a masked payload is not a document to publish; the row keeps the
    /// vendor blob it carried, so a later readable report of the same session publishes it.
    ///
    /// It is **not** exempt from `run_step.usage`. A persisted `usage` row is summed from its
    /// document, not from its typed event ([`UsageTotals::add_payload`]), so masking costs it only
    /// the keys masking made unreadable. Skipping it here would be a real divergence rather than a
    /// tidiness: `upload_pending` sums *every* persisted `usage` row of a chat that happened
    /// offline (MOD-2 plan D36), so an online step and the same step uploaded from a buffer would
    /// disagree for exactly this one row shape.
    async fn record_unreadable(
        &mut self,
        kind: EventKind,
        payload: Value,
        raw: Option<Value>,
        at: DateTime<Utc>,
    ) -> Result<Option<CapBreach>, RecordError> {
        self.flush().await?;
        let mut breach = None;
        if kind == EventKind::Usage {
            self.usage.add_payload(&payload);
            self.usage_dirty = true;
            // And it is **not** exempt from the cap either, for the same reason it is not exempt
            // from the sum: the spend is real, the row records it, and a run that could dodge its
            // cap by having one payload key masked would be a guard rail with a hole in it.
            breach = self.check_cap(at);
        }
        let tool_call_id = payload
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.push(PendingRow {
            kind,
            role: EventRole::Agent,
            tool_call_id,
            payload,
            raw: raw.into_iter().collect(),
            at,
        });
        self.flush().await?;
        Ok(breach)
    }

    /// The fail-closed path (`R-SEC-3`): drop the row, write a `scrub_residue` row in its place,
    /// remember the residue for [`Recorder::finish`], and tell the UI nothing.
    async fn refuse(&mut self, unmasked: Unmasked, at: DateTime<Utc>) -> Result<(), RecordError> {
        self.flush().await?;
        self.note_residue(&unmasked);
        self.push(residue_row(&unmasked, at));
        self.flush().await
    }

    /// Keeps the first residue of the session; the rest are already visible as their own rows.
    fn note_residue(&mut self, unmasked: &Unmasked) {
        if self.residue.is_none() {
            self.residue = Some(unmasked.clone());
        }
    }

    /// Stamps a buffered row with its step and turn. `seq` is assigned by the caller.
    fn event_row(&self, row: PendingRow) -> SessionEvent {
        let raw = match row.raw.len() {
            0 => None,
            // One row, one message. A coalesced row is many messages, so it carries the array
            // that produced it rather than an arbitrary one of them.
            1 => row.raw.into_iter().next(),
            _ => Some(Value::Array(row.raw)),
        };
        SessionEvent {
            run_step_id: self.step,
            seq: 0,
            turn: self.turn,
            kind: row.kind,
            role: row.role,
            tool_call_id: row.tool_call_id,
            payload: row.payload,
            raw,
            at: row.at,
        }
    }

    /// Scrubs an envelope's payload and its `raw` blob, returning the scrubbed event - `None`
    /// when masking made the document unreadable as its own type - the scrubbed payload document
    /// and the `raw` to persist.
    fn scrub_envelope(
        &self,
        envelope: &DriverEnvelope,
    ) -> Result<(Option<DriverEvent>, Value, Option<Value>), ScrubRefusal> {
        let (event, payload) = scrub_event(self.scrubber, &envelope.event)?;
        let raw = match (self.retain_raw, envelope.raw.clone()) {
            (true, Some(mut raw)) => {
                self.scrubber
                    .scrub(&mut raw)
                    .map_err(ScrubRefusal::Unmasked)?;
                Some(raw)
            }
            _ => None,
        };
        Ok((event, payload, raw))
    }

    /// Offers one scrubbed envelope to the chat tab, counting the frame as dropped when the
    /// bounded channel is full or gone. Never awaits: a paused tab must not stall the agent.
    fn send_ui(&mut self, envelope: DriverEnvelope) {
        let Some(ui) = self.ui.as_ref() else {
            return;
        };
        match ui.try_send(envelope) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.dropped += 1,
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.dropped += 1;
                // The tab is gone; stop paying for the attempt on every later event.
                self.ui = None;
            }
        }
    }
}

/// The `error` row that stands in for a payload the scrubber refused.
///
/// Its two fields are the rule name and the JSON pointer [`Unmasked`] carries, never the offending
/// text. The row is **not** exempt from the flush's scrub - it is buffered like any other row and
/// scrubbed with the rest - but the pass finds nothing: a rule name is one of a handful of
/// constants, and the pointer's tokens have already passed the scrubber's own residue check. A
/// residue row that did trip a rule would be dropped and replaced by a second residue row, which
/// is the fail-closed direction and not a loop.
fn residue_row(unmasked: &Unmasked, at: DateTime<Utc>) -> PendingRow {
    PendingRow {
        kind: EventKind::Error,
        role: EventRole::Htui,
        tool_call_id: None,
        payload: json!({
            "code": SCRUB_RESIDUE,
            "message": format!("{} at {}", unmasked.rule, unmasked.path),
        }),
        raw: Vec::new(),
        at,
    }
}

/// One of the recorder's own events as the payload document it persists.
///
/// Pinned to the serde form rather than hand-built, unlike [`residue_row`]'s two constant strings:
/// a hand-written `{"stop_reason": "cancelled"}` that drifted from [`DoneEvent`]'s serialisation
/// would be a `done` row the chat tab and the replay path could no longer deserialise.
fn encode<T: Serialize>(event: &T) -> Result<Value, RecordError> {
    serde_json::to_value(event).map_err(|error| RecordError::Encode(error.to_string()))
}

/// `session_event.tool_call_id` for the events that name a call.
fn tool_call_id_of(event: &DriverEvent) -> Option<String> {
    match event {
        DriverEvent::ToolCall(call) => Some(call.tool_call_id.clone()),
        DriverEvent::ToolResult(result) => Some(result.tool_call_id.clone()),
        DriverEvent::EditProposal(proposal) => proposal.tool_call_id.clone(),
        DriverEvent::PermissionRequest(request) => request.tool_call_id.clone(),
        _ => None,
    }
}

/// Why an envelope could not be handed back as a scrubbed event.
///
/// Private, because only two of the three reach the caller as a [`RecordError`]: the third is
/// handled inside [`Recorder::record`] and is not a failure at all.
#[derive(Debug)]
enum ScrubRefusal {
    /// Something credential-shaped survived masking. Fail closed (`R-SEC-3`).
    Unmasked(Unmasked),
    /// The event could not be serialised at all - a transport produced a non-representable value.
    Encode(String),
}

/// Scrubs one event's payload, returning the event rebuilt from the scrubbed document - `None`
/// when the masked document no longer reads back as its own type - and the document itself.
///
/// The round trip is what keeps one definition of a payload: the JSON that goes to
/// `session_event.payload` is the JSON the rebuilt event carries, so the chat tab and the store
/// see the *same* capture-time masking and a frame can never be less masked than its row was at
/// capture. It is **not** a promise that the screen shows exactly what the row shows: the flush
/// scrubs the assembled run a second time, and a secret split across two chunks is masked there,
/// after both frames have gone out. The module doc's item 4 states the render channel's real,
/// weaker guarantee; milestone 3 owns the chat tab and any stronger render-path rule.
fn scrub_event(
    scrubber: &dyn Scrubber,
    event: &DriverEvent,
) -> Result<(Option<DriverEvent>, Value), ScrubRefusal> {
    Ok(match event {
        DriverEvent::AssistantChunk(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::AssistantChunk), payload)
        }
        DriverEvent::ThoughtChunk(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::ThoughtChunk), payload)
        }
        DriverEvent::ToolCall(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::ToolCall), payload)
        }
        DriverEvent::ToolResult(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::ToolResult), payload)
        }
        DriverEvent::EditProposal(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::EditProposal), payload)
        }
        DriverEvent::PermissionRequest(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::PermissionRequest), payload)
        }
        DriverEvent::Plan(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::Plan), payload)
        }
        DriverEvent::Usage(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::Usage), payload)
        }
        DriverEvent::Error(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::Error), payload)
        }
        DriverEvent::Done(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::Done), payload)
        }
        DriverEvent::Other(inner) => {
            let (inner, payload) = round_trip(scrubber, inner)?;
            (inner.map(DriverEvent::Other), payload)
        }
    })
}

/// Serialise, scrub, deserialise.
///
/// The scrubbed **document** comes back whatever happens: it is masked, so it is safe to persist.
/// The value comes back only when the masked document still reads as a `T`. Masking alone can
/// make it not read as one, with nothing having leaked - a secret equal to a closed-vocabulary
/// wire string masks that value, and a secret equal to a field name masks the key - so a failure
/// here is `None` rather than an error, and the caller decides what a row with no typed event
/// means (see [`Recorder::record_unreadable`]). Only a value serde cannot serialise **at all** is
/// an error, and that is a transport bug, not a masking artefact.
fn round_trip<T>(scrubber: &dyn Scrubber, inner: &T) -> Result<(Option<T>, Value), ScrubRefusal>
where
    T: Serialize + DeserializeOwned,
{
    let mut payload =
        serde_json::to_value(inner).map_err(|error| ScrubRefusal::Encode(error.to_string()))?;
    scrubber
        .scrub(&mut payload)
        .map_err(ScrubRefusal::Unmasked)?;
    Ok((serde_json::from_value(payload.clone()).ok(), payload))
}

// ---------------------------------------------------------------------------------------------
// The pump
// ---------------------------------------------------------------------------------------------

/// Drives one turn of a session into a recorder and returns the [`DoneEvent`] that closed it.
///
/// Exactly one `done` per turn precedes the next accepted follow-up (`docs/ANA-4.md` §4.1), so a
/// caller runs `pump` once per prompt or follow-up. Envelopes after the `done` are left in the
/// transport for the next turn.
///
/// A turn that reaches its per-run cap ends `cancelled` rather than reaching the transport's own
/// `done`: the breach verdict [`Recorder::record`] hands back is answered here, by
/// [`enforce_breach`], because this is a layer that holds both the session and the recorder.
///
/// # Errors
/// [`DriverError::Closed`] when the transport ends the stream without a `done`, whatever
/// [`AgentSession::next_event`] returned otherwise, and the recorder's own failures mapped through
/// [`RecordError`].
pub async fn pump<S: WriteStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
) -> Result<DoneEvent, DriverError> {
    loop {
        let Some(envelope) = session.next_event().await? else {
            return Err(DriverError::Closed);
        };
        let done = match &envelope.event {
            DriverEvent::Done(done) => Some(*done),
            _ => None,
        };
        if let Some(breach) = recorder.record(envelope).await? {
            return enforce_breach(session, recorder, breach).await;
        }
        if let Some(done) = done {
            return Ok(done);
        }
    }
}

/// The per-run cap's cancel-and-close sequence, performed by the layer that holds the session
/// (plan D69 as amended by the blueprint's P-1).
///
/// **Why it is shared and not [`pump`]'s.** `Recorder` holds no session and must not: it is the
/// type the conformance suite drives with no transport at all, which is exactly what makes the
/// suite transport-neutral. So the cancel belongs to a loop that holds both — and there are **two**
/// of those. `pump` is the one the suites and `tests/recorder.rs` drive; production drives
/// `htui::agent_worker::run_turn`, which cannot be `pump` because `next_event` refuses while a
/// permission request is parked, so the pull and the command channel have to be served by one
/// loop. A sequence written into `pump` alone would be a cap the binary never enforced.
///
/// **The order, and why.** `cancel(grace)`; then pull what the cancel itself produced — the
/// synthesized `tool_result` for every call it terminated (`docs/ANA-4.md` §4.3 makes those a
/// MUST), any `error` the transport reports on its way out — into the log, **up to** the
/// transport's own `done`, which is withheld; then the two closing rows
/// ([`Recorder::record_cap_breach`]). The `done` is withheld rather than recorded because its
/// `stop_reason` may say `end_turn` if the agent finished inside the grace window, and criterion 8
/// says the last row says `cancelled`.
///
/// The grace comes off [`Recorder::run_cap`] rather than a parameter, which is what keeps `pump`'s
/// signature and its eighteen call sites unchanged (blueprint H-9).
///
/// A **failed cancel is logged, not returned**: the kill path has already ended the session by
/// then, and the rows still have to be written. Returning here would leave a log whose last row is
/// the `usage` report that breached, and nothing saying why the conversation stopped.
///
/// # Errors
/// The recorder's own, mapped through [`RecordError`]: a store that refuses the closing flush is
/// reported to the caller, and the rows it numbered stay owed (blueprint H-2).
pub async fn enforce_breach<S: WriteStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    breach: CapBreach,
) -> Result<DoneEvent, DriverError> {
    let grace = recorder.run_cap().map_or(Duration::ZERO, |cap| cap.grace);
    if let Err(err) = session.cancel(grace).await {
        tracing::warn!(%err, "the cap's cancel took the kill path; the closing rows are written anyway");
    }
    let mut transport_done = None;
    while let Ok(Some(envelope)) = session.next_event().await {
        if matches!(envelope.event, DriverEvent::Done(_)) {
            transport_done = Some(envelope);
            break;
        }
        // The verdict is spent, so this cannot answer `Some` again (`Recorder::check_cap`) - which
        // is what keeps a second cancel out of a session that is already closing.
        recorder.record(envelope).await?;
    }
    recorder.record_cap_breach(breach, transport_done).await?;
    Ok(DoneEvent {
        stop_reason: StopReason::Cancelled,
    })
}
