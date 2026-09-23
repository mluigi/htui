//! Fan-out's pure half (ANA-2 §4.5, plan D49, D52, D53): the prefilter, the group's route, the
//! judge's phase and inputs, and the judge's verdict.
//!
//! Nothing here reads a store or drives a session. The engine settles a group's candidates, hands
//! them to [`prefilter`] and [`route`], and — on the judge route — builds the judge step from
//! [`judge_phase`], [`judge_candidate`] and [`judge_inputs`], then reads each call's document back
//! through [`parse_judge_verdict`]. Every failure the judge can have is a [`JudgeFailure`] whose
//! `Display` is the byte-exact reason a park note and a `gate_note` carry.

use core::fmt;
use std::collections::BTreeMap;

use htui_core::model::{
    Agent, CommandQueue, Gate, Isolation, RunStep, SnapshotCandidate, SnapshotJudge, SnapshotPhase,
    SnapshotTemplate, StepId, StepStatus, VerifyOutcome,
};
use htui_core::prompt::{JudgeCandidate, JudgeInputs};
use serde_json::Value;

/// The judge step's `output_kind`, and so the `document.kind` its verdict is read from (D52).
pub const JUDGE_KIND: &str = "judge";

/// D49(4)'s reason: the text of the auto-win's item note (D77) and `select_fanout`'s `reason`.
pub const AUTO_WIN_REASON: &str = "the only candidate whose verify_command did not fail";

/// D65's reason: the text of a human pick's item note (D77) and `select_fanout`'s `reason`.
pub const HUMAN_PICK_REASON: &str = "selected by a human";

/// The judge step's `run_step.phase_name`, and so its `SessionKey.phase`: `<phase>:judge` (D51).
///
/// Only the step row carries this name. The judge's `PromptSpec.phase` stays the judged phase's
/// own name, because the seeded judge body renders it as the phase being judged (F-G).
#[must_use]
pub fn judge_phase_name(phase: &str) -> String {
    format!("{phase}:{JUDGE_KIND}")
}

/// A judge call's verdict: the winning `fanout_index` and a reason per candidate (D52).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeVerdict {
    /// The winning candidate's `fanout_index`, not yet range-checked (the engine holds the
    /// survivors).
    pub winner: i32,
    /// `"<fanout_index>"` → reason; entries whose value was not a string are dropped.
    pub reasons: BTreeMap<String, String>,
}

/// Why the judge could not pick a winner (D52): every variant ends in D50's human park.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeFailure {
    /// The document held no complete fenced `json` block, or its block did not parse, or its
    /// `winner` was not an integer.
    Unparseable(String),
    /// The verdict named a `fanout_index` that is not one of the survivors.
    OutOfRange {
        /// The verdict's `winner`.
        winner: i32,
        /// The `fanout_index` of every candidate the judge was shown.
        survivors: Vec<i32>,
    },
    /// The forward and reversed calls named different winners (position bias, §4.7 rule 6).
    Disagreement {
        /// Call 0's winner.
        forward: i32,
        /// Call 1's winner.
        reversed: i32,
    },
    /// A call finished without writing a new `judge` document.
    MissingDocument {
        /// Which call: `0` forward, `1` reversed.
        call: u32,
    },
    /// The judge's session failed.
    SessionFailed(String),
    /// The judge's agent was skipped by the walk, or has no model to run.
    Unavailable(String),
}

impl fmt::Display for JudgeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparseable(detail) => write!(f, "judge_unparseable: {detail}"),
            Self::OutOfRange { winner, survivors } => {
                let survivors = survivors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "judge_out_of_range: {winner} not in [{survivors}]")
            }
            Self::Disagreement { forward, reversed } => write!(
                f,
                "judge_disagreement: forward {forward}, reversed {reversed}"
            ),
            Self::MissingDocument { call } => write!(f, "judge_missing_document: call {call}"),
            Self::SessionFailed(error) => write!(f, "judge_session_failed: {error}"),
            Self::Unavailable(reason) => write!(f, "judge_unavailable: {reason}"),
        }
    }
}

impl std::error::Error for JudgeFailure {}

/// The one fenced block the judge is told to end with (ANA-5 `:1347-1348`,
/// `prompt/defaults.rs:189-190`).
const OPEN_FENCE: &str = "```json";
/// The line that closes [`OPEN_FENCE`].
const CLOSE_FENCE: &str = "```";

/// The **last** complete fenced `json` block of `body`, parsed into a [`JudgeVerdict`] (D52).
///
/// A line whose trim is exactly `` ```json `` opens a block and the next line whose trim is
/// exactly `` ``` `` closes it; an unclosed block is not a block. A missing `reasons` is an empty
/// map and non-string reasons are dropped, because §4.5 does not count them as a failure. Range
/// checking is the caller's, since only the caller holds the survivors.
///
/// # Errors
/// [`JudgeFailure::Unparseable`] when there is no block, the block is not JSON, or `winner` is
/// missing or not an integer that fits `i32`.
pub fn parse_judge_verdict(body: &str) -> Result<JudgeVerdict, JudgeFailure> {
    let block = last_json_block(body)
        .ok_or_else(|| JudgeFailure::Unparseable("no fenced json block".to_owned()))?;
    let value: Value = serde_json::from_str(&block)
        .map_err(|error| JudgeFailure::Unparseable(error.to_string()))?;
    let winner = value
        .get("winner")
        .and_then(Value::as_i64)
        .and_then(|winner| i32::try_from(winner).ok())
        .ok_or_else(|| JudgeFailure::Unparseable("winner is not an integer".to_owned()))?;
    let reasons = value
        .get("reasons")
        .and_then(Value::as_object)
        .map(|reasons| {
            reasons
                .iter()
                .filter_map(|(index, reason)| Some((index.clone(), reason.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    Ok(JudgeVerdict { winner, reasons })
}

/// The text between the last [`OPEN_FENCE`] line and the [`CLOSE_FENCE`] line after it, lines
/// joined with `\n` (`str::lines` folds CRLF); `None` when no block was ever closed.
fn last_json_block(body: &str) -> Option<String> {
    let mut last = None;
    let mut open: Option<Vec<&str>> = None;
    for line in body.lines() {
        match open.as_mut() {
            None if line.trim() == OPEN_FENCE => open = Some(Vec::new()),
            None => {}
            Some(_) if line.trim() == CLOSE_FENCE => {
                last = open.take().map(|lines| lines.join("\n"));
            }
            Some(lines) => lines.push(line),
        }
    }
    last
}

/// What the prefilter and the router read of one candidate step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateView {
    /// `run_step.id`.
    pub step: StepId,
    /// `run_step.fanout_index`, `0..fan_out`.
    pub fanout_index: i32,
    /// `run_step.status` as the candidate settled (D48).
    pub status: StepStatus,
    /// `run_step.verify_outcome`.
    pub verify_outcome: Option<VerifyOutcome>,
}

impl CandidateView {
    /// The view of a candidate row.
    #[must_use]
    pub fn of(step: &RunStep) -> Self {
        Self {
            step: step.id,
            fanout_index: step.fanout_index,
            status: step.status,
            verify_outcome: step.verify_outcome,
        }
    }
}

/// D49's two sets, both in `fanout_index` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefilter {
    /// The candidates that settled `done`.
    pub pool: Vec<CandidateView>,
    /// The pool whose `verify_outcome` is not `fail` (`pass`, `unavailable` or none).
    pub passing: Vec<CandidateView>,
}

/// D49's prefilter: `pool` is every `done` candidate, `passing` the pool less the verify failures.
#[must_use]
pub fn prefilter(candidates: &[CandidateView]) -> Prefilter {
    let mut pool: Vec<CandidateView> = candidates
        .iter()
        .filter(|candidate| candidate.status == StepStatus::Done)
        .copied()
        .collect();
    pool.sort_by_key(|candidate| candidate.fanout_index);
    let passing = pool
        .iter()
        .filter(|candidate| candidate.verify_outcome != Some(VerifyOutcome::Fail))
        .copied()
        .collect();
    Prefilter { pool, passing }
}

/// Why a group parks for a human (D50): the reason its park note names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HumanReason {
    /// The phase's `gate_effective` is `always`.
    Gated,
    /// The phase has no judge.
    NoJudge,
    /// No candidate is `done` and passing, and the gate is not `never`.
    NoSurvivingCandidate,
    /// The judge failed.
    JudgeFailed(JudgeFailure),
    /// The judge prompt's trimmer dropped the candidate at this `fanout_index` (D53).
    CandidateDropped(i32),
}

impl fmt::Display for HumanReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gated => f.write_str("gated"),
            Self::NoJudge => f.write_str("no judge configured"),
            Self::NoSurvivingCandidate => f.write_str("no_surviving_candidate"),
            Self::JudgeFailed(failure) => write!(f, "{failure}"),
            Self::CandidateDropped(index) => write!(f, "judge_candidate_dropped: {index}"),
        }
    }
}

/// Where a settled group goes next (D49).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// D50's park for a human selection.
    Human(HumanReason),
    /// The one passing candidate wins; no judge step is created.
    AutoWin(StepId),
    /// The judge compares these candidates, in `fanout_index` order.
    Judge(Vec<StepId>),
    /// A `never`-gated group with no survivor: §4.2's `failed` cell for the whole group.
    GroupFailed,
}

/// D49, in order:
/// (1) pool or passing empty → `GroupFailed` at `never`, else `Human(NoSurvivingCandidate)`;
/// (2) `always` → `Human(Gated)`; (3) no judge → `Human(NoJudge)`;
/// (4) one passing → `AutoWin`; (5) `Judge(passing)`.
#[must_use]
pub fn route(gate: Gate, has_judge: bool, pre: &Prefilter) -> Route {
    if pre.pool.is_empty() || pre.passing.is_empty() {
        return if gate == Gate::Never {
            Route::GroupFailed
        } else {
            Route::Human(HumanReason::NoSurvivingCandidate)
        };
    }
    if gate == Gate::Always {
        return Route::Human(HumanReason::Gated);
    }
    if !has_judge {
        return Route::Human(HumanReason::NoJudge);
    }
    match pre.passing.as_slice() {
        [only] => Route::AutoWin(only.step),
        passing => Route::Judge(passing.iter().map(|candidate| candidate.step).collect()),
    }
}

/// The judge step's phase: a struct update over the judged `phase` (D51, D53, F-G).
///
/// `name` is [`judge_phase_name`]; the judge is ungated (`gate`/`gate_effective` `never`), single
/// (`fan_out` 1), writes a [`JUDGE_KIND`] document from no inputs, verifies nothing, runs in no
/// tree (`isolation` `local`) with no command queue, on `template`, with `judge` as its only
/// candidate, and has no judge of its own.
#[must_use]
pub fn judge_phase(
    phase: &SnapshotPhase,
    template: SnapshotTemplate,
    judge: &SnapshotCandidate,
) -> SnapshotPhase {
    SnapshotPhase {
        name: judge_phase_name(&phase.name),
        gate: Gate::Never,
        gate_effective: Gate::Never,
        fan_out: 1,
        output_kind: JUDGE_KIND.to_owned(),
        input_kinds: Vec::new(),
        verify_command: None,
        isolation: Isolation::Local,
        command_queue: CommandQueue::Off,
        template,
        candidates: vec![judge.clone()],
        judge: None,
        ..phase.clone()
    }
}

/// The judge as a candidate (D51): the model is `judge.model`, else `agent.default_model`, else
/// `agent.models[0]`; `None` when there is none, which the engine reports as `judge_unavailable`.
#[must_use]
pub fn judge_candidate(judge: &SnapshotJudge, agent: &Agent) -> Option<SnapshotCandidate> {
    let model = judge
        .model
        .clone()
        .or_else(|| agent.default_model.clone())
        .or_else(|| agent.models.first().cloned())?;
    Some(SnapshotCandidate {
        agent_id: judge.agent_id,
        agent_name: judge.agent_name.clone(),
        model,
    })
}

/// The judge's prompt inputs for one call (D53): `candidates` in `fanout_index` order, and
/// `reverse` set for the second call — the assembler does the reversing (`prompt/mod.rs:573-580`),
/// so both calls hand it the same list.
#[must_use]
pub fn judge_inputs(task: String, candidates: Vec<JudgeCandidate>, reverse: bool) -> JudgeInputs {
    let mut candidates = candidates;
    candidates.sort_by_key(|candidate| candidate.fanout_index);
    JudgeInputs {
        task,
        candidates,
        reverse,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        Agent, CommandQueue, Gate, GraphSnapshot, Isolation, SnapshotCandidate, SnapshotJudge,
        SnapshotPhase, SnapshotTemplate, StepId, StepStatus, VerifyOutcome,
    };
    use htui_core::prompt::JudgeCandidate;
    use serde_json::json;

    use super::{
        AUTO_WIN_REASON, CandidateView, HUMAN_PICK_REASON, HumanReason, JUDGE_KIND, JudgeFailure,
        JudgeVerdict, Prefilter, Route, judge_candidate, judge_inputs, judge_phase,
        judge_phase_name, parse_judge_verdict, prefilter, route,
    };

    /// One candidate view at `index`, settled `status` with `verify`.
    fn view(index: i32, status: StepStatus, verify: Option<VerifyOutcome>) -> CandidateView {
        CandidateView {
            step: StepId::new(),
            fanout_index: index,
            status,
            verify_outcome: verify,
        }
    }

    fn done(index: i32, verify: Option<VerifyOutcome>) -> CandidateView {
        view(index, StepStatus::Done, verify)
    }

    fn steps(views: &[CandidateView]) -> Vec<StepId> {
        views.iter().map(|view| view.step).collect()
    }

    /// The `feature` graph's first phase, as the fixture's `RUN_1` snapshot carries it.
    fn phase() -> SnapshotPhase {
        let run = demo_data()
            .runs
            .into_iter()
            .find(|row| row.id == ids::RUN_1)
            .expect("the fixture holds RUN_1");
        let snapshot: GraphSnapshot =
            serde_json::from_value(run.graph_snapshot.expect("RUN_1 carries a snapshot"))
                .expect("the fixture snapshot is a `GraphSnapshot`");
        snapshot
            .phases
            .into_iter()
            .next()
            .expect("the snapshot has a phase")
    }

    /// A judge built through `serde_json` only, so `SnapshotJudge` can gain a field without
    /// breaking this module (blueprint F-H).
    fn snapshot_judge(model: Option<&str>) -> SnapshotJudge {
        serde_json::from_value(json!({
            "agent_id": ids::AGENT_AGY,
            "agent_name": "agy",
            "model": model,
        }))
        .expect("a judge decodes")
    }

    fn agent(id: htui_core::model::AgentId) -> Agent {
        demo_data()
            .agents
            .into_iter()
            .find(|agent| agent.id == id)
            .expect("the fixture seeds the agent")
    }

    fn judge_candidate_at(index: i32) -> JudgeCandidate {
        JudgeCandidate {
            fanout_index: index,
            verify: Some(index % 2 == 0),
            exit_code: Some(index),
            document: None,
            diff: None,
            verification_tail: None,
        }
    }

    /// D52: the verdict is the **last** fenced `json` block; an earlier one, and an unclosed one
    /// after it, are not the verdict.
    #[test]
    fn parse_judge_verdict_reads_the_last_fenced_json_block() {
        let body = "# Comparison\r\n\
                    An example of the shape:\r\n\
                    ```json\r\n\
                    { \"winner\": 9 }\r\n\
                    ```\r\n\
                    The verdict:\r\n\
                    \x20 ```json \r\n\
                    { \"winner\": 2, \"reasons\": { \"0\": \"slower\", \"2\": \"cleaner\", \"1\": 7 } }\r\n\
                    ```\r\n\
                    ```json\r\n\
                    { \"winner\": 5 }\r\n";
        assert_eq!(
            parse_judge_verdict(body),
            Ok(JudgeVerdict {
                winner: 2,
                reasons: BTreeMap::from([
                    ("0".to_owned(), "slower".to_owned()),
                    ("2".to_owned(), "cleaner".to_owned()),
                ]),
            }),
            "the last *complete* block, CRLF folded, the non-string reason dropped"
        );
        assert_eq!(
            parse_judge_verdict("```json\n{ \"winner\": 0 }\n```"),
            Ok(JudgeVerdict {
                winner: 0,
                reasons: BTreeMap::new(),
            }),
            "a missing `reasons` is an empty map, not a failure"
        );
    }

    #[test]
    fn a_prose_only_body_is_unparseable() {
        for body in [
            "Candidate 1 is better.",
            "",
            "```\n{ \"winner\": 1 }\n```",
            "```json\n{ \"winner\": 1 }\n",
        ] {
            assert_eq!(
                parse_judge_verdict(body),
                Err(JudgeFailure::Unparseable("no fenced json block".to_owned())),
                "{body:?}"
            );
        }
        let Err(JudgeFailure::Unparseable(detail)) =
            parse_judge_verdict("```json\n{ winner: 1 }\n```")
        else {
            panic!("a block that is not JSON is unparseable");
        };
        assert!(!detail.is_empty(), "the serde error is the detail");
    }

    #[test]
    fn a_non_integer_winner_is_unparseable() {
        for block in [
            "{ \"winner\": \"1\" }",
            "{ \"winner\": 1.5 }",
            "{ \"winner\": null }",
            "{ \"reasons\": {} }",
            "{ \"winner\": 4294967296 }",
            "[1]",
        ] {
            assert_eq!(
                parse_judge_verdict(&format!("```json\n{block}\n```")),
                Err(JudgeFailure::Unparseable(
                    "winner is not an integer".to_owned()
                )),
                "{block}"
            );
        }
        assert_eq!(
            parse_judge_verdict("```json\n{ \"winner\": -1 }\n```").map(|verdict| verdict.winner),
            Ok(-1),
            "range checking is the engine's"
        );
    }

    /// D52's six failures and D50's five park reasons, byte for byte.
    #[test]
    fn judge_failure_display_is_byte_exact() {
        let cases = [
            (
                JudgeFailure::Unparseable("no fenced json block".to_owned()),
                "judge_unparseable: no fenced json block",
            ),
            (
                JudgeFailure::OutOfRange {
                    winner: 7,
                    survivors: vec![0, 2],
                },
                "judge_out_of_range: 7 not in [0, 2]",
            ),
            (
                JudgeFailure::Disagreement {
                    forward: 0,
                    reversed: 2,
                },
                "judge_disagreement: forward 0, reversed 2",
            ),
            (
                JudgeFailure::MissingDocument { call: 1 },
                "judge_missing_document: call 1",
            ),
            (
                JudgeFailure::SessionFailed("driver exited".to_owned()),
                "judge_session_failed: driver exited",
            ),
            (
                JudgeFailure::Unavailable("agy (quota: exhausted)".to_owned()),
                "judge_unavailable: agy (quota: exhausted)",
            ),
        ];
        for (failure, bytes) in &cases {
            assert_eq!(failure.to_string(), *bytes);
            assert_eq!(
                HumanReason::JudgeFailed(failure.clone()).to_string(),
                *bytes,
                "a judge failure parks under its own bytes"
            );
        }
        assert_eq!(HumanReason::Gated.to_string(), "gated");
        assert_eq!(HumanReason::NoJudge.to_string(), "no judge configured");
        assert_eq!(
            HumanReason::NoSurvivingCandidate.to_string(),
            "no_surviving_candidate"
        );
        assert_eq!(
            HumanReason::CandidateDropped(2).to_string(),
            "judge_candidate_dropped: 2"
        );
        assert_eq!(
            AUTO_WIN_REASON,
            "the only candidate whose verify_command did not fail"
        );
        assert_eq!(HUMAN_PICK_REASON, "selected by a human");
    }

    /// D49: the prefilter drops only `fail`; the judge is reached only when two or more pass, and
    /// then compares the passing ones alone.
    #[test]
    fn prefilter_eliminates_fail_only_when_two_pass() {
        let candidates = [
            done(2, Some(VerifyOutcome::Pass)),
            done(0, Some(VerifyOutcome::Fail)),
            view(3, StepStatus::Failed, None),
            done(1, Some(VerifyOutcome::Unavailable)),
            done(4, None),
        ];
        let row = demo_data()
            .steps
            .into_iter()
            .next()
            .expect("the fixture holds a step");
        assert_eq!(
            CandidateView::of(&row),
            CandidateView {
                step: row.id,
                fanout_index: row.fanout_index,
                status: row.status,
                verify_outcome: row.verify_outcome,
            }
        );
        let pre = prefilter(&candidates);
        assert_eq!(
            pre.pool
                .iter()
                .map(|view| view.fanout_index)
                .collect::<Vec<_>>(),
            [0, 1, 2, 4],
            "every `done`, in `fanout_index` order"
        );
        assert_eq!(
            pre.passing
                .iter()
                .map(|view| view.fanout_index)
                .collect::<Vec<_>>(),
            [1, 2, 4],
            "`unavailable` and no verify both pass the filter"
        );
        assert_eq!(
            route(Gate::Never, true, &pre),
            Route::Judge(steps(&pre.passing)),
            "the failing candidate is not shown to the judge"
        );

        let one = prefilter(&[
            done(0, Some(VerifyOutcome::Fail)),
            done(1, Some(VerifyOutcome::Pass)),
        ]);
        assert_eq!(one.pool.len(), 2, "the failing candidate stays selectable");
        assert_eq!(
            route(Gate::OnFailure, false, &one),
            Route::Human(HumanReason::NoJudge),
            "with no judge a human sees both"
        );
    }

    #[test]
    fn one_passing_candidate_routes_to_auto_win() {
        let winner = done(1, Some(VerifyOutcome::Pass));
        let pre = prefilter(&[
            done(0, Some(VerifyOutcome::Fail)),
            winner,
            done(2, Some(VerifyOutcome::Fail)),
        ]);
        for gate in [Gate::Never, Gate::OnFailure] {
            assert_eq!(
                route(gate, true, &pre),
                Route::AutoWin(winner.step),
                "{gate:?}"
            );
        }
        assert_eq!(
            route(Gate::Always, true, &pre),
            Route::Human(HumanReason::Gated),
            "a gated group still goes to a human"
        );
    }

    #[test]
    fn zero_passing_is_a_failed_group() {
        let all_fail = prefilter(&[
            done(0, Some(VerifyOutcome::Fail)),
            done(1, Some(VerifyOutcome::Fail)),
        ]);
        let none_done = prefilter(&[
            view(0, StepStatus::Failed, None),
            view(1, StepStatus::Failed, None),
        ]);
        for pre in [&all_fail, &none_done] {
            for has_judge in [true, false] {
                assert_eq!(route(Gate::Never, has_judge, pre), Route::GroupFailed);
                for gate in [Gate::Always, Gate::OnFailure] {
                    assert_eq!(
                        route(gate, has_judge, pre),
                        Route::Human(HumanReason::NoSurvivingCandidate),
                        "{gate:?} parks a failed group"
                    );
                }
            }
        }
    }

    /// Every row of ANA-2 `:807-812` × gate × judge presence, in D49's order.
    #[test]
    fn route_follows_ana2_s_table() {
        let fail = || done(0, Some(VerifyOutcome::Fail));
        let pass = |index| done(index, Some(VerifyOutcome::Pass));
        // (pool size, passing size) = (0,0), (1,0), (1,1), (2,0), (2,1), (2,2).
        let groups: Vec<Prefilter> = vec![
            prefilter(&[view(0, StepStatus::Failed, None)]),
            prefilter(&[fail()]),
            prefilter(&[pass(0)]),
            prefilter(&[fail(), done(1, Some(VerifyOutcome::Fail))]),
            prefilter(&[fail(), pass(1)]),
            prefilter(&[pass(0), pass(1)]),
        ];
        for pre in &groups {
            let sizes = (pre.pool.len(), pre.passing.len());
            for gate in [Gate::Never, Gate::OnFailure, Gate::Always] {
                for has_judge in [false, true] {
                    let expected = match (sizes.1, gate, has_judge) {
                        (0, Gate::Never, _) => Route::GroupFailed,
                        (0, _, _) => Route::Human(HumanReason::NoSurvivingCandidate),
                        (_, Gate::Always, _) => Route::Human(HumanReason::Gated),
                        (_, _, false) => Route::Human(HumanReason::NoJudge),
                        (1, _, true) => Route::AutoWin(pre.passing[0].step),
                        (_, _, true) => Route::Judge(steps(&pre.passing)),
                    };
                    assert_eq!(
                        route(gate, has_judge, pre),
                        expected,
                        "pool/passing {sizes:?}, {gate:?}, judge {has_judge}"
                    );
                }
            }
        }
    }

    /// D53: both calls hand the assembler the same list in `fanout_index` order; only the flag
    /// differs, and the assembler does the reversing.
    #[test]
    fn judge_inputs_reverse_only_the_order() {
        let shuffled = vec![
            judge_candidate_at(2),
            judge_candidate_at(0),
            judge_candidate_at(1),
        ];
        let forward = judge_inputs("the task".to_owned(), shuffled.clone(), false);
        let reversed = judge_inputs("the task".to_owned(), shuffled, true);
        assert_eq!(
            forward
                .candidates
                .iter()
                .map(|candidate| candidate.fanout_index)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(forward.candidates, reversed.candidates);
        assert_eq!(forward.task, reversed.task);
        assert!(!forward.reverse);
        assert!(reversed.reverse);
    }

    /// D51, D53, F-G: the judge's phase is the judged one with the judge's own shape.
    #[test]
    fn judge_phase_is_named_phase_colon_judge_and_outputs_judge() {
        let judged = SnapshotPhase {
            fan_out: 3,
            gate: Gate::Always,
            gate_effective: Gate::OnFailure,
            verify_command: Some("cargo test".to_owned()),
            isolation: Isolation::Worktree,
            command_queue: CommandQueue::FanOutOnly,
            judge: Some(snapshot_judge(None)),
            ..phase()
        };
        let template = SnapshotTemplate {
            name: "judge".to_owned(),
            version: 3,
        };
        let judge = SnapshotCandidate {
            agent_id: ids::AGENT_AGY,
            agent_name: "agy".to_owned(),
            model: "gemini".to_owned(),
        };
        let phase = judge_phase(&judged, template.clone(), &judge);
        assert_eq!(judge_phase_name("research"), "research:judge");
        assert_eq!(phase.name, format!("{}:judge", judged.name));
        assert_eq!(phase.output_kind, JUDGE_KIND);
        assert_eq!(JUDGE_KIND, "judge");
        assert_eq!(
            (phase.gate, phase.gate_effective),
            (Gate::Never, Gate::Never)
        );
        assert_eq!(phase.fan_out, 1);
        assert!(phase.input_kinds.is_empty());
        assert_eq!(phase.verify_command, None);
        assert_eq!(phase.isolation, Isolation::Local);
        assert_eq!(phase.command_queue, CommandQueue::Off);
        assert_eq!(phase.template, template);
        assert_eq!(phase.candidates, [judge]);
        assert_eq!(phase.judge, None);
        assert_eq!(
            (phase.position, phase.retry_limit, phase.token_budget),
            (judged.position, judged.retry_limit, judged.token_budget),
            "the rest is the judged phase's"
        );
    }

    /// D51: the judge's model is `judge.model`, else the agent's default, else its first model.
    #[test]
    fn judge_candidate_resolves_the_model_down_the_ladder() {
        let agy = agent(ids::AGENT_AGY);
        let default = agy
            .default_model
            .clone()
            .expect("agy seeds a default model");
        let pinned = judge_candidate(&snapshot_judge(Some("pinned")), &agy)
            .expect("a pinned model resolves");
        assert_eq!(
            (
                pinned.agent_id,
                pinned.agent_name.as_str(),
                pinned.model.as_str()
            ),
            (ids::AGENT_AGY, "agy", "pinned")
        );
        assert_eq!(
            judge_candidate(&snapshot_judge(None), &agy).map(|candidate| candidate.model),
            Some(default)
        );
        let listed = Agent {
            default_model: None,
            models: vec!["first".to_owned(), "second".to_owned()],
            ..agy.clone()
        };
        assert_eq!(
            judge_candidate(&snapshot_judge(None), &listed).map(|candidate| candidate.model),
            Some("first".to_owned())
        );
        let bare = Agent {
            default_model: None,
            models: Vec::new(),
            ..agy
        };
        assert_eq!(
            judge_candidate(&snapshot_judge(None), &bare),
            None,
            "no model: judge_unavailable"
        );
    }
}
