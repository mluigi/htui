//! Where the walk is, and whether it may take another step.
//!
//! Three things milestone 2 needs before anything can walk: ANA-2 §4.2's retry admission
//! predicate (plan D3), the typed failure vocabulary whose `Display` renders ANA-2's exact bytes
//! (plan D12), and the cursor the engine re-derives from `run_steps` on every call rather than
//! remembering across one (plan D16).

use core::fmt;

use htui_core::model::{
    EdgeKind, GateOutcome, GraphSnapshot, RunStep, StepId, StepRole, StepStatus,
    missing_tags_failure,
};
use htui_core::scrub::Unmasked;

/// The `run_step.fanout_index` a `fan_out = 1` phase walks, and the one every fan-out slot has.
///
/// A fanned-out phase holds candidates at `0..fan_out` and a judge at `-1` (`docs/ANA-2.md` §4.5);
/// [`cursor`] reads such a slot as a group (plan D59), while [`latest_at`], [`next_attempt`] and
/// [`winner_at`]'s fallback keep reading index 0, which a group created whole always holds.
const WALKED_FANOUT_INDEX: i32 = 0;

/// ANA-2 §4.2's retry admission, read prospectively (plan D3): may `next_attempt` be *created*?
///
/// ANA-2 writes the predicate as the bare `attempt <= retry_limit + 1` four times (`:487`,
/// `:579`, `:646`, `:1562`) and prospectively once (`:716`). Only the prospective reading yields
/// the two attempts `:487` says the shipped `retry_limit = 1` permits — the retrospective one
/// reads the *existing* step's attempt and admits a third. One helper, so the sites cannot drift
/// apart again.
#[must_use]
pub const fn may_attempt(next_attempt: i32, retry_limit: i32) -> bool {
    next_attempt <= retry_limit + 1
}

/// Why a run stopped: `run.failure` text and refusal wording in one vocabulary (plan D12).
///
/// ANA-2 mixes prose strings and identifier-shaped codes with no rule, and two of them are
/// asserted verbatim by its validation criteria 6 and 14 (`docs/ANA-2.md:2095`, `:2101`). A typed
/// enum with one [`fmt::Display`] is how the engine reasons over variants while the bytes stay
/// exact; the same reason milestone 1 gave the refusal sentences of
/// `crates/htui-core/src/store/traits.rs:1053-1130` one named function apiece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    /// Stage 3: an `input_kinds` entry resolved to no document (`docs/ANA-2.md:414`). Carries the
    /// `document.kind` that was missing.
    MissingInput(String),
    /// Stage 5: no document of the phase's `output_kind` was produced by this step (`:430`).
    MissingOutput,
    /// Stage 1: no candidate survives the inline-approval interlock (`:482`). The phase is gated
    /// and every candidate is CLI-only, so there is nothing to schedule it onto.
    MissingCapability,
    /// §4.4 escalation (`:746`): the review loop ran out of budget. Carries the attempt count.
    ReviewLoopExhausted(i32),
    /// §4.4 step 1 found no position to loop back to — a `review` at position 0, or one with no
    /// predecessor (plan D5). ANA-2 names the case and not the string.
    NoLoopTarget,
    /// A human rejected a gated step of a phase that cannot loop (`docs/ANA-2.md:578`, blueprint
    /// A-8). Carries the phase name.
    Rejected {
        /// `step_graph_phase.name` of the rejected step.
        phase: String,
    },
    /// Stage 1 left no eligible candidate (plan D62): the `R-AGT-8` walk skipped every agent for a
    /// reason other than the inline-approval interlock, or `graph::resolve` reached rung 4. An
    /// empty `detail` renders with no `; ` suffix, which is the `StartRun` rung-4 note.
    NoCandidateAgent {
        /// `step_graph_phase.name` of the phase that found nothing.
        phase: String,
        /// `<agent> (<reason>), …` for every skipped agent; empty when there was no walk.
        detail: String,
    },
    /// Every candidate of a fan-out group failed, so there is nothing to select (plan D48).
    NoSurvivingCandidate {
        /// `step_graph_phase.name` of the fanned-out phase.
        phase: String,
    },
    /// Plan D94: the sweep failed a step it found `running` after the lease expired, and parked
    /// the run (ANA-2 §4.9 `:1298-1299`). `run.failure` stays NULL (R-3): this is the typed
    /// reason of the sweep's park, not a column.
    Interrupted {
        /// `step_graph_phase.name` of the interrupted step.
        phase: String,
        /// Whether the step's trees were reset (D92), or left exactly as found (D93).
        reset: bool,
    },
    /// Plan D131: a `running` run whose latest step is `failed`, out of budget and not
    /// interrupted, is a settle that crashed before its `finish_run`. The step's own failure is
    /// not on any row, so the run ends on the budget that stopped it.
    RetryBudgetSpent {
        /// `step_graph_phase.name` of the failed step.
        phase: String,
        /// `run_step.attempt` of the failed step, the last the budget allowed.
        attempt: i32,
    },
    /// MOD-4 plan D162 (blueprint D195, ANA-5 criterion 3): stage 3's assembler refused the
    /// phase's own prompt, so the step failed before a token was spent and the item is `blocked`.
    /// A missing required input is [`RunFailure::MissingInput`], not this.
    PromptRefused {
        /// `step_graph_phase.name` of the phase whose prompt was refused.
        phase: String,
        /// The assembler's own sentence (`htui_core::prompt::AssembleError`'s `Display`).
        reason: String,
    },
    /// `R-ORCH-10` (MOD-7 milestone 3, D83): the item requires tags this box has neither probed
    /// nor declared. Carries them in byte order, deduplicated, as `GraphSource::missing_tags` and
    /// `Claim::MissingTags` answer. The note at enqueue and at claim, and `run.failure` at claim,
    /// read the same bytes (D78).
    MissingTags(Vec<String>),
    /// MOD-10 D5: a scrub refusal on a live step. It is either `TrimRecord::to_value` refusing the
    /// step's record, or the recorder's `finish` reporting a dropped row. Names the phase and the
    /// rule, **never** the text and never the JSON pointer.
    ScrubRefused {
        /// `step_graph_phase.name` of the step, `<phase>:judge` for a judge, `chat` for a chat.
        phase: String,
        /// The rule name `Unmasked::rule` carried, e.g. `anthropic_api_key`.
        rule: String,
    },
    /// MOD-10 D13: the run's secrets were refused before any agent started: no identity,
    /// unreachable, bad credentials, locked, cooling down, not found, permission, invalid or
    /// reserved key, or a configuration fault. Transient causes fail the run too (OQ-A).
    SecretsRefused {
        /// The provider's or the column check's error. Value-free by construction (M2 D4).
        cause: htui_core::secret::SecretError,
    },
    /// MOD-89 D3, D6: a provider project's session trees hold env files and `allow_env_files` is
    /// off, so the step failed before its driver was built. Names the files, never a value.
    EnvFilesRefused {
        /// `project.secret_provider`, as the column holds it.
        provider: String,
        /// What the scan found.
        files: crate::isolate::EnvFiles,
    },
    /// MOD-138 M2 D7: a `fails` or `asks_you` loop ran out of budget or, for `fails`, made no
    /// progress. A `changes_requested` loop keeps [`RunFailure::ReviewLoopExhausted`] and its
    /// bytes (`gate::loop_failure`). `run.failure` stays NULL, as for the review loop (R-3).
    LoopExhausted {
        /// The loop edge's kind: `fails` or `asks_you`.
        kind: EdgeKind,
        /// The edge's source node.
        from: String,
        /// The edge's target node, the one the loop would have re-run.
        to: String,
        /// The target's attempt just judged.
        attempts: i32,
    },
    /// MOD-139 M2 D2 (ANA-33 §7.6): a divided node's admitted plan has no part.
    EmptyPlan {
        /// The divided node's name (`implement`, never `implement:plan`).
        node: String,
    },
    /// MOD-139 M2 D7: more than one part under `local` isolation, which runs one.
    LocalCannotDivide {
        /// The divided node's name.
        node: String,
        /// The parts planned.
        parts: u32,
    },
    /// MOD-139 M2 D8 (ANA-33 §7.8): an admission over one of its caps.
    PlanOverCaps {
        /// The divided node's name.
        node: String,
        /// The cap broken.
        cap: PlanCap,
        /// Parts planned (`MaxParts`, `MaxFanOut`), or the run's planned sessions
        /// (`MaxAgentsPerRun`).
        planned: u32,
        /// The cap's value.
        max: u32,
    },
    /// MOD-139 M2 D5: a default part names a persona; per-part personas run from MOD-140 on.
    PartPersonaUnsupported {
        /// The divided node's name.
        node: String,
        /// The part's key.
        part: String,
    },
    /// MOD-139 M2 D5: a default part names an agent that is not one of the node's candidates.
    PartAgentNotCandidate {
        /// The divided node's name.
        node: String,
        /// The part's key.
        part: String,
        /// The agent the part names.
        agent: htui_core::model::AgentId,
    },
    /// MOD-139 M2 D10: a default part's owned path is not a repo-relative `repo:glob`.
    PartPathRefused {
        /// The divided node's name.
        node: String,
        /// The part's key.
        part: String,
        /// The owned path, trimmed.
        path: String,
        /// Why: not a glob, absolute, or leaving its repository.
        reason: String,
    },
    /// MOD-139 M2 D6: a part failed with no attempt left, so the divided step failed.
    PartExhausted {
        /// The divided node's name.
        node: String,
        /// The part's key.
        part: String,
        /// The part's last attempt.
        attempts: i32,
    },
    /// MOD-139 M3 D6: integrate's post-check found done parts its capture does not hold.
    IntegrateIncomplete {
        /// The divided node's name.
        node: String,
        /// The parts' keys, plan order.
        parts: Vec<String>,
    },
}

/// MOD-139 M3 D6: `IntegrateIncomplete`'s one sentence, shared by [`RunFailure`] and
/// `gate::StepFailure` (blueprint P-7: a singular and a plural form).
#[must_use]
pub fn integrate_incomplete(node: &str, parts: &[String]) -> String {
    let named = parts
        .iter()
        .map(|part| format!("`{part}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if parts.len() == 1 {
        format!("integrate_incomplete: part {named} of `{node}` is not merged")
    } else {
        format!("integrate_incomplete: parts {named} of `{node}` are not merged")
    }
}

/// MOD-139 M2 D8: which cap an admission broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanCap {
    /// The divided node's own `max_parts`.
    MaxParts,
    /// `app_setting.max_fan_out`, frozen in the snapshot.
    MaxFanOut,
    /// `app_setting.max_agents_per_run`, frozen in the snapshot.
    MaxAgentsPerRun,
}

impl PlanCap {
    /// The cap's setting name, as the failure sentence names it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxParts => "max_parts",
            Self::MaxFanOut => "max_fan_out",
            Self::MaxAgentsPerRun => "max_agents_per_run",
        }
    }
}

impl RunFailure {
    /// The one projection of an [`Unmasked`] into the run vocabulary (MOD-10 D5): the rule, never
    /// the path.
    #[must_use]
    pub fn scrub_refused(phase: impl Into<String>, unmasked: &Unmasked) -> Self {
        Self::ScrubRefused {
            phase: phase.into(),
            rule: unmasked.rule.to_owned(),
        }
    }
}

impl fmt::Display for RunFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput(kind) => write!(f, "missing input document: {kind}"),
            Self::MissingOutput => f.write_str("missing_output"),
            Self::MissingCapability => f.write_str("missing_capability: inline_approval"),
            Self::ReviewLoopExhausted(attempts) => {
                write!(f, "review loop exhausted after {attempts} attempts")
            }
            Self::NoLoopTarget => f.write_str("no_loop_target"),
            Self::Rejected { phase } => write!(f, "rejected: {phase}"),
            Self::NoCandidateAgent { phase, detail } if detail.is_empty() => {
                write!(f, "no_candidate_agent: phase `{phase}`")
            }
            Self::NoCandidateAgent { phase, detail } => {
                write!(f, "no_candidate_agent: phase `{phase}`; {detail}")
            }
            Self::NoSurvivingCandidate { phase } => write!(f, "no_surviving_candidate: {phase}"),
            Self::Interrupted { phase, reset: true } => write!(f, "interrupted: {phase}"),
            Self::Interrupted {
                phase,
                reset: false,
            } => write!(f, "interrupted, tree not reset: {phase}"),
            Self::RetryBudgetSpent { phase, attempt } => {
                write!(
                    f,
                    "retry budget spent: step `{phase}` attempt {attempt} failed"
                )
            }
            Self::PromptRefused { phase, reason } => {
                write!(f, "prompt refused at `{phase}`: {reason}")
            }
            Self::MissingTags(missing) => f.write_str(&missing_tags_failure(missing)),
            Self::ScrubRefused { phase, rule } => {
                write!(f, "scrub refused at `{phase}`: unmasked {rule}")
            }
            Self::SecretsRefused { cause } => f.write_str(&cause.refusal()),
            Self::EnvFilesRefused { provider, files } => {
                f.write_str(&crate::isolate::env_files::refused(provider, files))
            }
            Self::LoopExhausted {
                kind,
                from,
                to,
                attempts,
            } => write!(
                f,
                "{kind} loop exhausted after {attempts} attempts: `{from}` to `{to}`"
            ),
            Self::EmptyPlan { node } => {
                write!(f, "empty_plan: divided node `{node}` has no parts")
            }
            Self::LocalCannotDivide { node, parts } => write!(
                f,
                "local_cannot_divide: divided node `{node}` plans {parts} parts under `local` \
                 isolation, which runs one"
            ),
            Self::PlanOverCaps {
                node,
                cap: cap @ PlanCap::MaxAgentsPerRun,
                planned,
                max,
            } => write!(
                f,
                "plan_over_caps: divided node `{node}` brings the run to {planned} planned \
                 sessions; {} is {max}",
                cap.as_str()
            ),
            Self::PlanOverCaps {
                node,
                cap,
                planned,
                max,
            } => write!(
                f,
                "plan_over_caps: divided node `{node}` plans {planned} parts; {} is {max}",
                cap.as_str()
            ),
            Self::PartPersonaUnsupported { node, part } => write!(
                f,
                "part_persona_unsupported: part `{part}` of `{node}` names a persona; per-part \
                 personas run from MOD-140 on"
            ),
            Self::PartAgentNotCandidate { node, part, agent } => write!(
                f,
                "part_agent_not_candidate: part `{part}` of `{node}` names agent {agent}, which is \
                 not one of the node's candidates"
            ),
            Self::PartPathRefused {
                node,
                part,
                path,
                reason,
            } => write!(
                f,
                "part_path_refused: part `{part}` of `{node}`: owned path `{path}` {reason}"
            ),
            Self::PartExhausted {
                node,
                part,
                attempts,
            } => write!(
                f,
                "part_exhausted: part `{part}` of `{node}` failed after {attempts} attempts"
            ),
            Self::IntegrateIncomplete { node, parts } => {
                f.write_str(&integrate_incomplete(node, parts))
            }
        }
    }
}

/// Where the walk is, re-derived from `run_steps` on every call (plan D16).
///
/// The engine holds none of this across a call, which is what milestone 5's recovery sweep needs:
/// a sweep is only correct if there was no cross-restart state for it to have missed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    /// No live or finished step at `position`: create `(position, attempt, 0)` at `pending`.
    Create {
        /// `run_step.position`, a dense index into `snapshot.phases`.
        position: i32,
        /// `run_step.attempt`, 1-based, from [`next_attempt`].
        attempt: i32,
    },
    /// A `pending` step exists at the first unfinished position: run it.
    Run(StepId),
    /// MOD-125 M3 D4: a `stopped` step under a `running` run (the run was resumed): walk it on its
    /// own row, `stopped → running`, reopening its session or walking it again by what its log
    /// holds. Scheduled as [`Cursor::Run`] is; never a park.
    Resume(StepId),
    /// A `running`, `awaiting_approval` or `failed` step: the walk cannot advance on its own, and
    /// what happens next is a session, a human's gate answer or the review loop.
    Rest {
        /// The step the walk stopped on.
        step: StepId,
        /// Its status, so the caller need not read the row again.
        status: StepStatus,
    },
    /// Plan D59: a fan-out slot with fewer than `fan_out` candidates, or with one still
    /// `pending`: the engine creates the missing indices and drives the group together.
    Fan {
        /// `run_step.position` of the slot.
        position: i32,
        /// `run_step.attempt` of the slot.
        attempt: i32,
    },
    /// Plan D59: every candidate of a fan-out slot settled and none is `selected`: route the
    /// group (plan D49) — a winner, the judge, a human, or the group's failure.
    Select {
        /// `run_step.position` of the slot.
        position: i32,
        /// `run_step.attempt` of the slot.
        attempt: i32,
    },
    /// MOD-139 M2 D4: a divided node whose planner row is `done` and whose parts (of that planner
    /// attempt) are not all `done`, none is live and none is out of attempts: create the retry rows
    /// owed (T4) and run every `pending` part.
    Parts {
        /// The planner node's `position`.
        position: i32,
    },
    /// MOD-138 M2 D6: `step`, a node's winner, is `done` and its node's `if` edges are not
    /// decided yet (`run_step.branch` is NULL). A write the engine owes, so it sorts before every
    /// node cursor (blueprint A6).
    Decide {
        /// The winner whose `branch` is owed.
        step: StepId,
    },
    /// MOD-138 M2 D7: `step` is `done` + `approved` and its back `asks_you` edge to `to` still
    /// has budget, so the loop over that edge is owed. Sorts before every node cursor (A6).
    Loop {
        /// The approved source's winner.
        step: StepId,
        /// The edge's target key; the edge is `(step's node, to, asks_you)`, unique by MOD-137
        /// rule (c).
        to: String,
    },
    /// Every walked node of the snapshot is done at its latest attempt, or skipped (MOD-138 M2
    /// D2), and nothing is owed.
    Finished,
}

/// The latest-attempt step at `position`, or `None` when the walk has not reached it.
///
/// Only `fanout_index = 0` is considered: a `fan_out = 1` phase's one row, or any slot's first
/// candidate. [`cursor`] reads a fanned-out slot whole (plan D59).
#[must_use]
pub fn latest_at(steps: &[RunStep], position: i32) -> Option<&RunStep> {
    steps
        .iter()
        .filter(|step| step.position == position && step.fanout_index == WALKED_FANOUT_INDEX)
        .max_by_key(|step| step.attempt)
}

/// `max(attempt at position) + 1`, or `1` when nothing has run there.
///
/// Read on every call rather than carried, so `UNIQUE (run_id, position, attempt, fanout_index)`
/// cannot be tripped by a stale count (blueprint H-4).
#[must_use]
pub fn next_attempt(steps: &[RunStep], position: i32) -> i32 {
    latest_at(steps, position).map_or(1, |step| step.attempt + 1)
}

/// The `fanout_index` of a fan-out slot's judge row (`docs/ANA-2.md` §4.5, plan D53).
const JUDGE_FANOUT_INDEX: i32 = -1;

/// The candidates of the slot `(position, attempt)` — every row with `fanout_index >= 0` — in
/// `fanout_index` order (plan D59).
///
/// A single-candidate phase's slot is its one `fanout_index 0` row; the judge is never in the
/// group ([`judge_at`] answers it).
#[must_use]
pub fn group_at(steps: &[RunStep], position: i32, attempt: i32) -> Vec<&RunStep> {
    let mut group: Vec<&RunStep> = steps
        .iter()
        .filter(|step| step.position == position && step.attempt == attempt)
        .filter(|step| step.fanout_index >= 0)
        .collect();
    group.sort_by_key(|step| step.fanout_index);
    group
}

/// The judge row (`fanout_index = -1`) of the slot `(position, attempt)`, if one was created.
#[must_use]
pub fn judge_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep> {
    steps.iter().find(|step| {
        step.position == position
            && step.attempt == attempt
            && step.fanout_index == JUDGE_FANOUT_INDEX
    })
}

/// The slot's winner (plan D66): the candidate carrying `selected = true`, else its
/// `fanout_index 0` when nothing in the slot is selected.
///
/// The fallback is what makes a `fan_out = 1` phase read exactly as it did before fan-out: its one
/// row is never selected, and it is index 0. `None` only when the slot has no index 0 either.
#[must_use]
pub fn winner_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep> {
    let group = group_at(steps, position, attempt);
    group
        .iter()
        .find(|step| step.selected == Some(true))
        .or_else(|| {
            group
                .iter()
                .find(|step| step.fanout_index == WALKED_FANOUT_INDEX)
        })
        .copied()
}

/// Where the walk is: the first of [`crate::walk::ready`] (MOD-138 M2 D2).
///
/// MOD-138 M3: on a branching graph `ready` can hold several cursors and the concurrent walk
/// takes them all (through [`crate::walk::ready_masked`]); this first one stays the display and
/// recovery reading, and [`run_resumable`] reads every cursor where a resume is decided.
///
/// `ready` orders the owed writes ([`Cursor::Decide`], [`Cursor::Loop`]) first, then every reached
/// node's cursor in position order, so on a lifted linear graph this is exactly the pre-M2 scan
/// (blueprint §2.1): the first position whose latest step is not `done` decides.
///
/// `superseded` and `cancelled` at the latest attempt count as "no live step here" and yield
/// [`Cursor::Create`] at [`next_attempt`]: the lazy re-insertion plan D5's review loop relies on,
/// which supersedes the chain and leaves the creating to the walk.
///
/// A phase with `fan_out > 1` is read as a slot (plan D59, `group_cursor`); a `fan_out = 1`
/// phase is read exactly as it was before fan-out, at `fanout_index 0`.
///
/// Every status a row can hold is matched, including the ones §4.3 would not have produced: MOD-2
/// inserts `run_step` rows outside the status law on purpose, so the walk must never assume a row
/// passed through `can_move_to` (plan D17).
#[must_use]
pub fn cursor(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor {
    crate::walk::ready(snapshot, steps)
        .into_iter()
        .next()
        .unwrap_or(Cursor::Finished)
}

/// The pre-M2 cursor, verbatim, kept for the parity oracle (MOD-138 M2 D2, `walk.rs`'s tests):
/// walks `snapshot.phases` in position order; the first position whose latest step is not `done`
/// decides.
///
/// `superseded` and `cancelled` at the latest attempt count as "no live step here" and yield
/// [`Cursor::Create`] at [`next_attempt`] — the lazy re-insertion plan D5's review loop relies on,
/// which supersedes the chain and leaves the creating to the walk.
///
/// A phase with `fan_out > 1` is read as a slot (plan D59, `group_cursor`); a `fan_out = 1`
/// phase is read exactly as it was before fan-out, at `fanout_index 0`.
///
/// Every status a row can hold is matched, including the ones §4.3 would not have produced: MOD-2
/// inserts `run_step` rows outside the status law on purpose, so the walk must never assume a row
/// passed through `can_move_to` (plan D17).
#[cfg(test)]
#[must_use]
pub(crate) fn cursor_by_position(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor {
    for phase in &snapshot.phases {
        let position = phase.position;
        if phase.fan_out > 1 {
            match group_cursor(steps, position, phase.fan_out) {
                Some(stop) => return stop,
                None => continue,
            }
        }
        let Some(step) = latest_at(steps, position) else {
            return Cursor::Create {
                position,
                attempt: 1,
            };
        };
        match step.status {
            StepStatus::Done => {}
            StepStatus::Pending => return Cursor::Run(step.id),
            // MOD-125 M3 D4: as `walk::node_state` reads it, or the parity oracle diverges.
            StepStatus::Stopped => return Cursor::Resume(step.id),
            StepStatus::Running | StepStatus::AwaitingApproval | StepStatus::Failed => {
                return Cursor::Rest {
                    step: step.id,
                    status: step.status,
                };
            }
            StepStatus::Superseded | StepStatus::Cancelled => {
                return Cursor::Create {
                    position,
                    attempt: next_attempt(steps, position),
                };
            }
        }
    }
    Cursor::Finished
}

/// Whether a parked run's cursor is one a resume may walk on from (plan D132, blueprint D196).
///
/// `Create`, `Run` and `Finished` under an `awaiting_approval` run are a command that crashed
/// between its first write and its unpark: every step the run is parked over is `done`,
/// `superseded` or `selected`, and no gate is waiting on a human. `Decide` and `Loop` (MOD-138 M2
/// D2) are writes the engine still owes, which a crash, or a decision the isolator could not
/// read (D6), cut off. `Parts` (MOD-139 M2 D4) is a write the engine owes, as `Decide`. `Rest`,
/// `Fan` and `Select` are real parks a human has to answer. `Resume` (MOD-125 M3) is not one
/// either: a stopped step resumes only through `Engine::resume_stopped`, never a park's resume.
/// `Engine::resume`'s own walk and `Unblock`'s third case (plan D161) both read this one
/// predicate, so the two cannot drift.
#[must_use]
pub fn resumable_park(cursor: &Cursor) -> bool {
    matches!(
        cursor,
        Cursor::Create { .. }
            | Cursor::Run(_)
            | Cursor::Decide { .. }
            | Cursor::Loop { .. }
            | Cursor::Parts { .. }
            | Cursor::Finished
    )
}

/// MOD-37 R-31: the `failed` + `rejected` latest step of a parked run, which
/// `answer_gate(Rejected)` wrote and whose unpark a crash cut off.
///
/// `answer_gate` moves the step `awaiting_approval -> failed` and the unpark is the command's
/// second write, so a crash between them leaves the run `awaiting_approval` (the item reads `needs
/// you`) over a [`Cursor::Rest`] at `failed` whose `gate_outcome` is `rejected`. No gate waits on a
/// human there: the rejection was answered, and only its tail (the end of the run, or the review
/// loop) is owed. A `failed` step with no `gate_outcome` (a spent budget, an interrupt) is not one
/// of these.
///
/// A followed review-loop escalation looks exactly like this too (MOD-37 milestone 2 blueprint,
/// claim 2): the run parked over its `failed` + `rejected` review, the item moved back to `open`
/// by `Unblock`'s `FollowRun`. No row tells the two apart, so `Unblock` resumes
/// that one as well and the loop, with nothing changed, escalates again.
#[must_use]
pub fn crashed_rejection<'s>(cursor: &Cursor, steps: &'s [RunStep]) -> Option<&'s RunStep> {
    let Cursor::Rest {
        step,
        status: StepStatus::Failed,
    } = cursor
    else {
        return None;
    };
    steps
        .iter()
        .find(|row| row.id == *step)
        .filter(|row| row.gate_outcome == Some(GateOutcome::Rejected))
}

/// Blueprint D196, widened by MOD-37 R-31: the one predicate every resume reader reads.
///
/// [`resumable_park`]'s six cursors, or a [`crashed_rejection`]. `Unblock`'s third case
/// (through [`crate::command::unblock_enabled`]'s callers), `Engine::resume`'s walk and the
/// worker box's hand-back resume all read this, and none open-codes it.
#[must_use]
pub fn resumable(cursor: &Cursor, steps: &[RunStep]) -> bool {
    resumable_park(cursor) || crashed_rejection(cursor, steps).is_some()
}

/// MOD-138 M3 (A1): the position a cursor stands on, the key the concurrent walk holds a node
/// by. `Create`/`Fan`/`Select`/`Parts` carry it; `Run`/`Rest`/`Decide`/`Loop` read their step's
/// row, and a part row (MOD-139 M2, blueprint P-14) stands on its planner's position, the node it
/// belongs to. `None` for `Finished` and for a step `steps` does not hold.
#[must_use]
pub fn position_of(cursor: &Cursor, steps: &[RunStep]) -> Option<i32> {
    let step = match cursor {
        Cursor::Create { position, .. }
        | Cursor::Fan { position, .. }
        | Cursor::Select { position, .. }
        | Cursor::Parts { position } => return Some(*position),
        Cursor::Run(step)
        | Cursor::Resume(step)
        | Cursor::Rest { step, .. }
        | Cursor::Decide { step }
        | Cursor::Loop { step, .. } => step,
        Cursor::Finished => return None,
    };
    let row = steps.iter().find(|row| row.id == *step)?;
    match (row.role, row.parent_step_id) {
        (StepRole::Part, Some(parent)) => steps
            .iter()
            .find(|planner| planner.id == parent)
            .map(|planner| planner.position),
        _ => Some(row.position),
    }
}

/// MOD-138 M3 A6: whether a parked run is one a resume may walk on from: [`crate::walk::ready`]
/// is non-empty and **every** cursor of it is [`resumable`] (a [`resumable_park`] or a
/// [`crashed_rejection`]). A sibling's owed `Decide` sorts first, so the first cursor alone could
/// read resumable over a step still waiting on a human. On a linear graph `ready` holds one node
/// cursor, so this is `resumable(&cursor(..), steps)`.
#[must_use]
pub fn run_resumable(snapshot: &GraphSnapshot, steps: &[RunStep]) -> bool {
    let ready = crate::walk::ready(snapshot, steps);
    !ready.is_empty() && ready.iter().all(|cursor| resumable(cursor, steps))
}

/// Plan D59 for one fanned-out position; `None` when the slot is complete and the walk goes on.
///
/// The order is load-bearing (blueprint §9.3, H-14): a `selected` `done` candidate completes the
/// position before anything else is read; a slot retired whole is re-created at `attempt + 1`; a
/// `running` candidate or judge rests the walk before a `pending` sibling is driven; a short or
/// `pending` slot is driven; anything else is settled and goes to selection.
///
/// The slot's attempt is the highest over its candidates (every `fanout_index >= 0` row) rather
/// than [`latest_at`]'s index-0 reading, which agrees whenever a group was created whole (plan
/// D71, blueprint F-K) and cannot split a slot when one was not.
pub(crate) fn group_cursor(steps: &[RunStep], position: i32, fan_out: i32) -> Option<Cursor> {
    let Some(attempt) = steps
        .iter()
        .filter(|step| step.position == position && step.fanout_index >= 0)
        .map(|step| step.attempt)
        .max()
    else {
        return Some(Cursor::Create {
            position,
            attempt: 1,
        });
    };
    let group = group_at(steps, position, attempt);
    if group
        .iter()
        .any(|step| step.selected == Some(true) && step.status == StepStatus::Done)
    {
        return None;
    }
    if group
        .iter()
        .all(|step| matches!(step.status, StepStatus::Superseded | StepStatus::Cancelled))
    {
        return Some(Cursor::Create {
            position,
            attempt: attempt + 1,
        });
    }
    if let Some(live) = group
        .iter()
        .copied()
        .chain(judge_at(steps, position, attempt))
        .find(|step| step.status == StepStatus::Running)
    {
        return Some(Cursor::Rest {
            step: live.id,
            status: live.status,
        });
    }
    let short = i32::try_from(group.len()).is_ok_and(|len| len < fan_out);
    if short || group.iter().any(|step| step.status == StepStatus::Pending) {
        return Some(Cursor::Fan { position, attempt });
    }
    Some(Cursor::Select { position, attempt })
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{GateOutcome, GraphSnapshot, RunStep, StepId, StepStatus};

    use crate::status::{
        Cursor, PlanCap, RunFailure, crashed_rejection, cursor, group_at, judge_at, latest_at,
        may_attempt, next_attempt, position_of, resumable, resumable_park, run_resumable,
        winner_at,
    };
    use crate::walk::tests::{diamond, done, row};

    /// The four `RUN_1` steps, the one `RUN_2` step, or whatever the fixture holds for a run.
    fn steps_of(run: htui_core::model::RunId) -> Vec<RunStep> {
        demo_data()
            .steps
            .into_iter()
            .filter(|step| step.run_id == run)
            .collect()
    }

    /// The `feature` graph's snapshot, as every fixture `graph` run carries it.
    fn snapshot() -> GraphSnapshot {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
            .expect("the fixture snapshot is a `GraphSnapshot`")
    }

    /// MOD-138 M3 (A1): a cursor's position, carried by it or read from its step's row.
    #[test]
    fn position_of_reads_the_cursor_or_its_row() {
        let snapshot = diamond();
        let b = row(&snapshot, "b", 1, StepStatus::Pending);
        let c = done(&snapshot, "c");
        let steps = [b.clone(), c.clone()];
        let at = |cursor: Cursor| position_of(&cursor, &steps);
        assert_eq!(
            at(Cursor::Create {
                position: 3,
                attempt: 1
            }),
            Some(3)
        );
        assert_eq!(
            at(Cursor::Fan {
                position: 2,
                attempt: 1
            }),
            Some(2)
        );
        assert_eq!(
            at(Cursor::Select {
                position: 1,
                attempt: 2
            }),
            Some(1)
        );
        assert_eq!(at(Cursor::Run(b.id)), Some(1));
        assert_eq!(
            at(Cursor::Rest {
                step: b.id,
                status: StepStatus::Pending
            }),
            Some(1)
        );
        assert_eq!(at(Cursor::Decide { step: c.id }), Some(2));
        assert_eq!(
            at(Cursor::Loop {
                step: c.id,
                to: "a".to_owned()
            }),
            Some(2)
        );
        assert_eq!(at(Cursor::Finished), None);
        assert_eq!(at(Cursor::Run(StepId::new())), None, "a step the rows lack");
        assert_eq!(at(Cursor::Parts { position: 2 }), Some(2));
        assert!(
            resumable_park(&Cursor::Parts { position: 2 }),
            "an owed write"
        );

        // MOD-139 M2 (P-14): a part row stands on its planner's position.
        let mut planner = c.clone();
        planner.role = htui_core::model::StepRole::Planner;
        let mut part = row(&snapshot, "d", 1, StepStatus::Running);
        part.position = 7;
        part.role = htui_core::model::StepRole::Part;
        part.parent_step_id = Some(planner.id);
        let rows = [planner.clone(), part.clone()];
        assert_eq!(
            position_of(
                &Cursor::Rest {
                    step: part.id,
                    status: StepStatus::Running
                },
                &rows
            ),
            Some(planner.position)
        );
        assert_eq!(
            position_of(&Cursor::Run(part.id), std::slice::from_ref(&part)),
            None,
            "a part whose planner the rows lack"
        );
    }

    /// MOD-138 M3 A6: on a linear graph `run_resumable` is the first cursor's `resumable`.
    #[test]
    fn run_resumable_on_a_linear_graph_is_the_first_cursors() {
        let snapshot = snapshot();
        let steps = steps_of(ids::RUN_1);
        for len in 0..=steps.len() {
            for status in [
                StepStatus::Pending,
                StepStatus::Running,
                StepStatus::AwaitingApproval,
                StepStatus::Failed,
                StepStatus::Done,
                StepStatus::Superseded,
            ] {
                for outcome in [None, Some(GateOutcome::Rejected)] {
                    let mut rows = steps[..len].to_vec();
                    if let Some(last) = rows.last_mut() {
                        last.status = status;
                        last.gate_outcome = outcome;
                    }
                    assert_eq!(
                        run_resumable(&snapshot, &rows),
                        resumable(&cursor(&snapshot, &rows), &rows),
                        "{len} rows, the last {status} {outcome:?}"
                    );
                }
            }
        }
    }

    /// MOD-138 M3 A6: a run is resumable only when every cursor of `ready` is.
    #[test]
    fn run_resumable_needs_every_cursor_resumable() {
        let snapshot = diamond();
        let a = done(&snapshot, "a");
        let parked = row(&snapshot, "b", 1, StepStatus::AwaitingApproval);
        assert!(
            !run_resumable(&snapshot, &[a.clone(), parked.clone()]),
            "`b` waits on a human beside `c`'s `Create`"
        );
        assert!(
            resumable(
                &Cursor::Create {
                    position: 2,
                    attempt: 1
                },
                &[]
            ),
            "the other cursor alone would read resumable"
        );
        let b = done(&snapshot, "b");
        assert!(run_resumable(&snapshot, &[a.clone(), b]));
        let mut rejected = row(&snapshot, "b", 1, StepStatus::Failed);
        rejected.gate_outcome = Some(GateOutcome::Rejected);
        assert!(
            run_resumable(&snapshot, &[a.clone(), rejected.clone()]),
            "a crashed rejection beside a `Create`"
        );
        let running = row(&snapshot, "c", 1, StepStatus::Running);
        assert!(!run_resumable(&snapshot, &[a, rejected, running]));
    }

    /// MOD-138 M2 D7: a `fails` or `asks_you` loop's escalation names the kind, both ends and the
    /// attempt just judged; the note and the `Rest` read the same bytes.
    #[test]
    fn loop_exhausted_display() {
        assert_eq!(
            RunFailure::LoopExhausted {
                kind: htui_core::model::EdgeKind::Fails,
                from: "test".to_owned(),
                to: "implement".to_owned(),
                attempts: 2,
            }
            .to_string(),
            "fails loop exhausted after 2 attempts: `test` to `implement`"
        );
        assert_eq!(
            RunFailure::LoopExhausted {
                kind: htui_core::model::EdgeKind::AsksYou,
                from: "review".to_owned(),
                to: "plan".to_owned(),
                attempts: 1,
            }
            .to_string(),
            "asks_you loop exhausted after 1 attempts: `review` to `plan`"
        );
    }

    /// ANA-2 writes five of these as prose or as identifier-shaped codes and two of them are
    /// asserted verbatim by its own validation criteria 6 and 14 (`docs/ANA-2.md:2095`, `:2101`).
    /// A typed enum is only worth having if its `Display` is byte-exact, so this is the test the
    /// module was written against.
    #[test]
    fn run_failure_display_is_ana2s_bytes() {
        assert_eq!(
            RunFailure::MissingTags(vec!["a".into(), "b".into()]).to_string(),
            "missing tags: a, b",
            "MOD-7 milestone 3 D77: the tags joined with `, `, in the order given"
        );
        assert_eq!(
            RunFailure::MissingTags(vec!["docker".into(), "vulkan".into()]).to_string(),
            "missing tags: docker, vulkan",
            "criterion 14's note body, byte for byte"
        );
        assert_eq!(
            RunFailure::MissingInput("plan".to_owned()).to_string(),
            "missing input document: plan",
            "`docs/ANA-2.md:414`"
        );
        assert_eq!(
            RunFailure::MissingOutput.to_string(),
            "missing_output",
            "`docs/ANA-2.md:430`"
        );
        assert_eq!(
            RunFailure::MissingCapability.to_string(),
            "missing_capability: inline_approval",
            "`docs/ANA-2.md:482`"
        );
        assert_eq!(
            RunFailure::ReviewLoopExhausted(2).to_string(),
            "review loop exhausted after 2 attempts",
            "`docs/ANA-2.md:746`"
        );
        assert_eq!(
            RunFailure::NoLoopTarget.to_string(),
            "no_loop_target",
            "plan D5's terminal review; ANA-2 names the case and not the string"
        );
        assert_eq!(
            RunFailure::Rejected {
                phase: "implement".to_owned(),
            }
            .to_string(),
            "rejected: implement",
            "`docs/ANA-2.md:578`, blueprint A-8"
        );
        assert_eq!(
            RunFailure::NoCandidateAgent {
                phase: "implement".to_owned(),
                detail: "claude (quota: status rejected)".to_owned(),
            }
            .to_string(),
            "no_candidate_agent: phase `implement`; claude (quota: status rejected)",
            "plan D62: a stage-1 walk that left nothing eligible"
        );
        assert_eq!(
            RunFailure::NoCandidateAgent {
                phase: "prd".to_owned(),
                detail: String::new(),
            }
            .to_string(),
            "no_candidate_agent: phase `prd`",
            "plan D62's `StartRun` rung-4 note carries no detail, so no `; `"
        );
        assert_eq!(
            RunFailure::NoSurvivingCandidate {
                phase: "implement".to_owned(),
            }
            .to_string(),
            "no_surviving_candidate: implement",
            "plan D48: every candidate of the group failed"
        );
        assert_eq!(
            RunFailure::Interrupted {
                phase: "implement".to_owned(),
                reset: true,
            }
            .to_string(),
            "interrupted: implement",
            "plan D94: the sweep reset the step's trees and parked it out of budget"
        );
        assert_eq!(
            RunFailure::Interrupted {
                phase: "implement".to_owned(),
                reset: false,
            }
            .to_string(),
            "interrupted, tree not reset: implement",
            "plan D93/D94: a tree the sweep must not reset, left as found (`:1299`)"
        );
        assert_eq!(
            RunFailure::RetryBudgetSpent {
                phase: "plan".to_owned(),
                attempt: 2,
            }
            .to_string(),
            "retry budget spent: step `plan` attempt 2 failed",
            "plan D131: the sweep ends a run whose settle crashed before its `finish_run`"
        );
        assert_eq!(
            RunFailure::PromptRefused {
                phase: "prd".to_owned(),
                reason: "unknown prompt placeholder: {{no_such_placeholder}}".to_owned(),
            }
            .to_string(),
            "prompt refused at `prd`: unknown prompt placeholder: {{no_such_placeholder}}",
            "MOD-4 plan D162: ANA-5 criterion 3's stage-3 refusal"
        );
        assert_eq!(
            RunFailure::ScrubRefused {
                phase: "implement".to_owned(),
                rule: "anthropic_api_key".to_owned(),
            }
            .to_string(),
            "scrub refused at `implement`: unmasked anthropic_api_key",
            "MOD-10 D5: a scrub refusal names the phase and the rule, never the text or the pointer"
        );
        assert_eq!(
            RunFailure::SecretsRefused {
                cause: htui_core::secret::SecretError::NoIdentity,
            }
            .to_string(),
            "secrets_refused: no Infisical machine identity is stored in the OS keyring",
            "MOD-10 D13: a refused resolution names the cause, never a value"
        );
        assert_eq!(
            RunFailure::EnvFilesRefused {
                provider: "infisical".into(),
                files: crate::isolate::EnvFiles {
                    paths: vec![".env".into()],
                    truncated: None,
                },
            }
            .to_string(),
            "env_files_refused: `.env` in the session's trees; the project resolves its secrets \
             from infisical. Remove them, or set allow_env_files.",
            "MOD-89 D7: names the files and the provider, never a value"
        );
        let node = || "implement".to_owned();
        let part = || "api".to_owned();
        assert_eq!(
            RunFailure::EmptyPlan { node: node() }.to_string(),
            "empty_plan: divided node `implement` has no parts",
            "MOD-139 M2 D2"
        );
        assert_eq!(
            RunFailure::LocalCannotDivide {
                node: node(),
                parts: 2,
            }
            .to_string(),
            "local_cannot_divide: divided node `implement` plans 2 parts under `local` isolation, \
             which runs one",
            "MOD-139 M2 D7"
        );
        assert_eq!(
            RunFailure::PlanOverCaps {
                node: node(),
                cap: PlanCap::MaxParts,
                planned: 3,
                max: 2,
            }
            .to_string(),
            "plan_over_caps: divided node `implement` plans 3 parts; max_parts is 2",
            "MOD-139 M2 D8, a part count"
        );
        assert_eq!(
            RunFailure::PlanOverCaps {
                node: node(),
                cap: PlanCap::MaxFanOut,
                planned: 3,
                max: 2,
            }
            .to_string(),
            "plan_over_caps: divided node `implement` plans 3 parts; max_fan_out is 2",
            "MOD-139 M2 D8, a part count"
        );
        assert_eq!(
            RunFailure::PlanOverCaps {
                node: node(),
                cap: PlanCap::MaxAgentsPerRun,
                planned: 9,
                max: 8,
            }
            .to_string(),
            "plan_over_caps: divided node `implement` brings the run to 9 planned sessions; \
             max_agents_per_run is 8",
            "MOD-139 M2 D8, the run's sessions"
        );
        assert_eq!(
            RunFailure::PartPersonaUnsupported {
                node: node(),
                part: part(),
            }
            .to_string(),
            "part_persona_unsupported: part `api` of `implement` names a persona; per-part \
             personas run from MOD-140 on",
            "MOD-139 M2 D5"
        );
        assert_eq!(
            RunFailure::PartAgentNotCandidate {
                node: node(),
                part: part(),
                agent: ids::AGENT_AGY,
            }
            .to_string(),
            format!(
                "part_agent_not_candidate: part `api` of `implement` names agent {}, which is not \
                 one of the node's candidates",
                ids::AGENT_AGY
            ),
            "MOD-139 M2 D5"
        );
        assert_eq!(
            RunFailure::PartPathRefused {
                node: node(),
                part: part(),
                path: "/etc/x".to_owned(),
                reason: "is an absolute path".to_owned(),
            }
            .to_string(),
            "part_path_refused: part `api` of `implement`: owned path `/etc/x` is an absolute path",
            "MOD-139 M2 D10"
        );
        assert_eq!(
            RunFailure::PartExhausted {
                node: node(),
                part: part(),
                attempts: 2,
            }
            .to_string(),
            "part_exhausted: part `api` of `implement` failed after 2 attempts",
            "MOD-139 M2 D6"
        );
        assert_eq!(
            RunFailure::IntegrateIncomplete {
                node: node(),
                parts: vec!["ui".to_owned()],
            }
            .to_string(),
            "integrate_incomplete: part `ui` of `implement` is not merged",
            "MOD-139 M3 D6, one part"
        );
        assert_eq!(
            RunFailure::IntegrateIncomplete {
                node: node(),
                parts: vec![part(), "ui".to_owned()],
            }
            .to_string(),
            "integrate_incomplete: parts `api`, `ui` of `implement` are not merged",
            "MOD-139 M3 D6, several parts"
        );
    }

    /// MOD-10 D13: the run's sentence is the cause's own `refusal()`, for every cause the
    /// orchestrator can meet, so a chat, an engine error and a run read the same bytes.
    #[test]
    fn secrets_refused_is_the_cause_s_refusal() {
        use htui_core::secret::SecretError;
        let causes = [
            SecretError::NoIdentity,
            SecretError::Config(
                "the project names a secret provider but has no secret_scope".into(),
            ),
            SecretError::Unreachable {
                endpoint: "/api/v1/auth/universal-auth/login",
                cause: "connection refused".into(),
            },
            SecretError::BadCredentials,
            SecretError::IdentityLocked,
            SecretError::LoginRefusedEarlier,
            SecretError::LoginCoolingDown {
                retry_after_secs: 30,
            },
            SecretError::ProjectNotFound,
            SecretError::PathNotFound {
                environment: "dev".into(),
                path: "/app".into(),
            },
            SecretError::PermissionDenied {
                detail: "no access".into(),
            },
            SecretError::RateLimited {
                retry_after_secs: Some(5),
            },
            SecretError::UnsupportedServer {
                endpoint: "/api/v4/secrets",
            },
            SecretError::InvalidKey { key: "1BAD".into() },
            SecretError::InvalidValue {
                key: "API_KEY".into(),
            },
            SecretError::ReservedKey {
                key: "HTUI_LOG".into(),
            },
            SecretError::Protocol {
                endpoint: "/api/v4/secrets",
                detail: "not json".into(),
            },
        ];
        for cause in causes {
            let failure = RunFailure::SecretsRefused {
                cause: cause.clone(),
            };
            assert_eq!(failure.to_string(), cause.refusal());
            assert!(failure.to_string().starts_with("secrets_refused: "));
        }
    }

    /// MOD-10 D5: the one projection of an [`htui_core::scrub::Unmasked`] keeps the rule and drops
    /// the JSON pointer, so neither the `Display` nor the `Debug` can carry a path segment.
    #[test]
    fn scrub_refused_never_carries_the_pointer() {
        let unmasked = htui_core::scrub::Unmasked {
            path: "/payload/sk-secret".into(),
            rule: "aws_access_key_id",
        };
        let failure = RunFailure::scrub_refused("prd", &unmasked);
        assert_eq!(
            failure,
            RunFailure::ScrubRefused {
                phase: "prd".to_owned(),
                rule: "aws_access_key_id".to_owned(),
            }
        );
        for rendered in [failure.to_string(), format!("{failure:?}")] {
            assert!(!rendered.contains("/payload"), "no pointer: {rendered}");
            assert!(
                !rendered.contains("sk-secret"),
                "no path segment: {rendered}"
            );
        }
    }

    /// Blueprint D196: the three cursors a crashed command leaves under a parked run are
    /// resumable, and the three a human has to answer are not. MOD-138 M2 adds the two owed
    /// writes, `Decide` and `Loop` (next test).
    #[test]
    fn only_a_crashed_command_s_park_is_resumable() {
        let step = ids::STEP_R2_PRD;
        for cursor in [
            Cursor::Create {
                position: 1,
                attempt: 1,
            },
            Cursor::Run(step),
            Cursor::Finished,
        ] {
            assert!(resumable_park(&cursor), "{cursor:?} is a crashed command's");
        }
        for cursor in [
            Cursor::Rest {
                step,
                status: StepStatus::AwaitingApproval,
            },
            Cursor::Fan {
                position: 0,
                attempt: 1,
            },
            Cursor::Select {
                position: 0,
                attempt: 1,
            },
        ] {
            assert!(!resumable_park(&cursor), "{cursor:?} waits on a human");
        }
    }

    /// MOD-138 M2 D2: the two owed writes are a crashed command's, so a resume walks on.
    #[test]
    fn decide_and_loop_are_resumable_parks() {
        let step = ids::STEP_R2_PRD;
        for cursor in [
            Cursor::Decide { step },
            Cursor::Loop {
                step,
                to: "plan".to_owned(),
            },
        ] {
            assert!(resumable_park(&cursor), "{cursor:?} is owed by the engine");
            assert!(resumable(&cursor, &[]), "{cursor:?} resumes");
        }
    }

    /// MOD-37 R-31: a run parked over the `failed` + `rejected` step `answer_gate(Rejected)` wrote,
    /// whose unpark a crash cut off, is resumable, and [`crashed_rejection`] names the step. A
    /// `failed` step with no `gate_outcome` (a spent budget, an interrupt) is not, nor is a real
    /// gate. The rows are also exactly a followed review-loop escalation's (the MOD-37 milestone 2
    /// blueprint, claim 2), its twin: `Unblock` resumes that one too.
    #[test]
    fn a_rejection_a_crash_left_parked_is_resumable() {
        let mut steps = steps_of(ids::RUN_2);
        let step = steps[0].id;
        steps[0].status = StepStatus::Failed;
        steps[0].gate_outcome = Some(GateOutcome::Rejected);
        steps[0].gate_note = Some("not like this".to_owned());
        let failed = Cursor::Rest {
            step,
            status: StepStatus::Failed,
        };
        assert!(resumable(&failed, &steps), "the rejection's tail is owed");
        assert_eq!(
            crashed_rejection(&failed, &steps).map(|row| row.id),
            Some(step),
            "the predicate names the rejected step"
        );

        let mut spent = steps.clone();
        spent[0].gate_outcome = None;
        assert!(
            !resumable(&failed, &spent),
            "a budget's or an interrupt's `failed` step is no rejection"
        );
        assert!(crashed_rejection(&failed, &spent).is_none());

        let gate = Cursor::Rest {
            step,
            status: StepStatus::AwaitingApproval,
        };
        assert!(!resumable(&gate, &steps), "a real gate waits on a human");
        assert!(crashed_rejection(&gate, &steps).is_none());
    }

    /// `RUN_3`'s two `research` candidates with the selection moved to index 1: the winner is
    /// whichever row carries `selected = true`, not whichever sits at index 0 (plan D66).
    #[test]
    fn winner_at_prefers_the_selected_candidate() {
        let mut steps = steps_of(ids::RUN_3);
        for step in &mut steps {
            step.selected = Some(step.fanout_index == 1);
        }
        let winner = winner_at(&steps, 0, 1).expect("the slot has candidates");
        assert_eq!(winner.id, ids::STEP_R3_RESEARCH_B);
        assert_eq!(
            winner_at(&steps, 0, 2),
            None,
            "an attempt with no rows has no winner"
        );
    }

    /// A slot nothing has selected yet — a single-candidate phase, or a group still being driven —
    /// answers its `fanout_index 0`, which every slot has (plan D59, D66).
    #[test]
    fn winner_at_falls_back_to_index_zero_when_nothing_is_selected() {
        for selected in [None, Some(false)] {
            let mut steps = steps_of(ids::RUN_3);
            for step in &mut steps {
                step.selected = selected;
            }
            // Index 1 first, so the fallback cannot be "the first row in the slice".
            steps.reverse();
            let winner = winner_at(&steps, 0, 1).expect("the slot has candidates");
            assert_eq!(
                winner.id,
                ids::STEP_R3_RESEARCH_A,
                "`selected = {selected:?}` everywhere falls back to index 0"
            );
        }
    }

    /// The judge shares its slot with the candidates but is not one of them: `group_at` skips
    /// `fanout_index = -1`, orders by index, and `judge_at` answers the row it skipped.
    #[test]
    fn group_at_excludes_the_judge() {
        let mut steps = steps_of(ids::RUN_3);
        let mut judge = steps[0].clone();
        judge.id = StepId::new();
        judge.fanout_index = -1;
        judge.selected = None;
        let mut third = steps[0].clone();
        third.id = StepId::new();
        third.fanout_index = 2;
        third.selected = Some(false);
        let mut next = steps[0].clone();
        next.id = StepId::new();
        next.attempt = 2;
        // Out of index order on purpose, with a row of the next attempt mixed in.
        steps.insert(0, third.clone());
        steps.insert(1, judge.clone());
        steps.push(next);

        let group: Vec<_> = group_at(&steps, 0, 1).iter().map(|step| step.id).collect();
        assert_eq!(
            group,
            vec![ids::STEP_R3_RESEARCH_A, ids::STEP_R3_RESEARCH_B, third.id],
            "the candidates of `(0, 1)`, in `fanout_index` order, without the judge"
        );
        assert_eq!(judge_at(&steps, 0, 1).map(|step| step.id), Some(judge.id));
        assert_eq!(judge_at(&steps, 0, 2), None, "attempt 2 has no judge");
        assert!(group_at(&steps, 1, 1).is_empty());
    }

    /// Plan D3: the predicate is about the attempt that is *about to be created*, so the shipped
    /// `retry_limit = 1` permits two attempts and not three — which is what ANA-2 says in the same
    /// sentence it writes the bare form in (`docs/ANA-2.md:487`).
    #[test]
    fn may_attempt_is_prospective() {
        assert!(may_attempt(1, 1), "the first attempt is always admissible");
        assert!(may_attempt(2, 1), "one retry: the second attempt");
        assert!(
            !may_attempt(3, 1),
            "a third attempt against `retry_limit = 1` is the reading D3 refuses"
        );
        assert!(may_attempt(1, 0), "`retry_limit = 0` still permits one");
        assert!(!may_attempt(2, 0), "and only one");
    }

    #[test]
    fn latest_at_takes_the_highest_attempt_of_the_position() {
        let mut steps = steps_of(ids::RUN_1);
        let mut retry = steps[2].clone();
        retry.id = ids::STEP_R2_PRD; // any other id; the fixture mints no second attempt
        retry.attempt = 2;
        retry.status = StepStatus::Pending;
        steps.push(retry);

        let found = latest_at(&steps, 2).expect("position 2 has steps");
        assert_eq!(found.attempt, 2);
        assert_eq!(next_attempt(&steps, 2), 3);
        assert_eq!(
            next_attempt(&steps, 1),
            2,
            "one attempt exists at position 1"
        );
        assert_eq!(
            next_attempt(&steps, 9),
            1,
            "a position with no step starts at 1"
        );
        assert!(latest_at(&steps, 9).is_none());
    }

    /// `RUN_1` is the fixture's finished run: four `done` steps against the four-phase `feature`
    /// snapshot.
    #[test]
    fn cursor_on_four_done_steps_is_finished() {
        assert_eq!(cursor(&snapshot(), &steps_of(ids::RUN_1)), Cursor::Finished);
    }

    /// `RUN_2` holds one `pending` `prd` step and nothing else.
    #[test]
    fn cursor_on_a_pending_step_runs_it() {
        assert_eq!(
            cursor(&snapshot(), &steps_of(ids::RUN_2)),
            Cursor::Run(ids::STEP_R2_PRD)
        );
    }

    /// The position the walk has not reached yet is the one to create, at attempt 1.
    #[test]
    fn cursor_on_a_gap_creates_the_position() {
        let steps: Vec<RunStep> = steps_of(ids::RUN_1)
            .into_iter()
            .filter(|step| step.position < 2)
            .collect();
        assert_eq!(
            cursor(&snapshot(), &steps),
            Cursor::Create {
                position: 2,
                attempt: 1
            }
        );
    }

    /// Plan D5's lazy re-insertion: the review loop supersedes the implement step and leaves the
    /// creating to the walk, so a superseded latest attempt reads as "no live step here".
    #[test]
    fn cursor_after_a_supersede_creates_the_next_attempt() {
        let mut steps = steps_of(ids::RUN_1);
        steps[2].status = StepStatus::Superseded;
        assert_eq!(
            cursor(&snapshot(), &steps),
            Cursor::Create {
                position: 2,
                attempt: 2
            }
        );
    }

    /// A step the walk cannot advance past: `running`, `awaiting_approval` and `failed` all rest,
    /// and the engine's caller decides what happens next.
    #[test]
    fn cursor_rests_on_a_step_the_walk_cannot_advance() {
        for status in [
            StepStatus::Running,
            StepStatus::AwaitingApproval,
            StepStatus::Failed,
        ] {
            let mut steps = steps_of(ids::RUN_1);
            steps[3].status = status;
            assert_eq!(
                cursor(&snapshot(), &steps),
                Cursor::Rest {
                    step: steps[3].id,
                    status
                },
                "a `{status}` step at the last position rests"
            );
        }
    }

    /// Plan D59: a `fan_out = 1` phase is walked exactly as before fan-out — its cursor reads
    /// `fanout_index 0` only. `RUN_1`'s `review` has `fan_out = 1`, so a stray index-1 row at a
    /// later attempt (the shape of a loser or a judge) does not move the cursor off `Finished`.
    #[test]
    fn cursor_reads_the_zeroth_fanout_index_only() {
        let snapshot = snapshot();
        assert_eq!(
            snapshot.phases[3].fan_out, 1,
            "the fixture's `review` is single"
        );
        let mut steps = steps_of(ids::RUN_1);
        let mut loser = steps[3].clone();
        loser.id = ids::STEP_R3_RESEARCH_B;
        loser.fanout_index = 1;
        loser.attempt = 2;
        loser.status = StepStatus::Superseded;
        steps.push(loser);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Finished,
            "a non-zero fan-out index is not a `fan_out = 1` phase's cursor"
        );
    }

    /// `RUN_1`'s snapshot with `prd` (position 0) fanned out three ways, and `RUN_1`'s steps with
    /// its `prd` row replaced by `group`: one candidate per `(fanout_index, status, selected)`.
    fn fanned(group: &[(i32, StepStatus, Option<bool>)]) -> (GraphSnapshot, Vec<RunStep>) {
        let mut snapshot = snapshot();
        snapshot.phases[0].fan_out = 3;
        let mut steps = steps_of(ids::RUN_1);
        let prd = steps.remove(0);
        assert_eq!(prd.position, 0, "the first `RUN_1` row is `prd`");
        for (index, status, selected) in group {
            let mut candidate = prd.clone();
            candidate.id = StepId::new();
            candidate.fanout_index = *index;
            candidate.status = *status;
            candidate.selected = *selected;
            steps.push(candidate);
        }
        (snapshot, steps)
    }

    /// Plan D59's `Fan`: a group with a `pending` candidate, or with fewer than `fan_out`, is
    /// driven — the walk does not run index 0 alone and wait on the rest.
    #[test]
    fn cursor_drives_an_incomplete_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Pending, None),
            (2, StepStatus::Pending, None),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Fan {
                position: 0,
                attempt: 1
            },
            "a `pending` candidate is driven with its group"
        );

        let (snapshot, steps) = fanned(&[(0, StepStatus::Done, None), (1, StepStatus::Done, None)]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Fan {
                position: 0,
                attempt: 1
            },
            "two of three: the missing index is created by the drive"
        );
    }

    /// Plan D59's `Select`: every candidate settled — `done` or `failed` — and none selected.
    #[test]
    fn cursor_selects_a_settled_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Failed, None),
            (2, StepStatus::Done, None),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Select {
                position: 0,
                attempt: 1
            }
        );
    }

    /// A `selected` `done` candidate completes the position, wherever it sits in the group — here
    /// at index 2, with index 0 a superseded loser the old cursor would have re-created.
    #[test]
    fn cursor_passes_a_selected_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Superseded, Some(false)),
            (2, StepStatus::Done, Some(true)),
        ]);
        assert_eq!(cursor(&snapshot, &steps), Cursor::Finished);
    }

    /// A group retired whole — superseded by the review loop or cancelled by a retry — is "no
    /// live slot here", and the next attempt is created.
    #[test]
    fn cursor_creates_the_next_attempt_after_a_retired_group() {
        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Superseded, Some(false)),
            (1, StepStatus::Cancelled, Some(false)),
            (2, StepStatus::Superseded, Some(true)),
        ]);
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Create {
                position: 0,
                attempt: 2
            }
        );
    }

    /// Blueprint H-14: a `running` candidate or a `running` judge rests the walk, and a `running`
    /// candidate outranks a `pending` sibling — one call never drives half a group twice.
    #[test]
    fn a_running_judge_rests() {
        let (snapshot, mut steps) = fanned(&[
            (0, StepStatus::Done, None),
            (1, StepStatus::Done, None),
            (2, StepStatus::Done, None),
        ]);
        let mut judge = steps[steps.len() - 1].clone();
        judge.id = StepId::new();
        judge.fanout_index = -1;
        judge.status = StepStatus::Running;
        steps.push(judge.clone());
        assert_eq!(
            cursor(&snapshot, &steps),
            Cursor::Rest {
                step: judge.id,
                status: StepStatus::Running
            }
        );

        let (snapshot, steps) = fanned(&[
            (0, StepStatus::Running, None),
            (1, StepStatus::Pending, None),
            (2, StepStatus::Pending, None),
        ]);
        assert!(
            matches!(
                cursor(&snapshot, &steps),
                Cursor::Rest {
                    status: StepStatus::Running,
                    ..
                }
            ),
            "a live candidate rests the walk before its pending siblings are driven"
        );
    }
}
