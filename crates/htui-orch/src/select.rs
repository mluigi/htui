//! `R-AGT-8`'s walk (ANA-2 §7 `docs/ANA-2.md:1603-1615`, plan D60): pure over rows, no store, no
//! engine.
//!
//! Stage 1 hands this module the phase's `phase_agent` candidates in priority order and the rows
//! it read for them; the walk answers which candidates are eligible, in the same order, and why
//! each of the others was skipped. The engine then hands `eligible` to its `AgentSelector`, so
//! "the first eligible candidate" is `FirstCandidate`'s answer and a skipped higher-priority row
//! is named in an `item_note` rather than substituted silently (`R-ORCH-10`).

use core::fmt;
use std::collections::BTreeMap;

use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_core::model::quota::{self, Availability, SkipReason};
use htui_core::model::{Agent, AgentBox, AgentId, Gate, RunStep, SnapshotCandidate};

/// Everything [`walk`] reads, borrowed from the caller (plan D60).
#[derive(Debug, Clone, Copy)]
pub struct SelectInput<'a> {
    /// The phase's candidates, in `phase_agent` order.
    pub candidates: &'a [SnapshotCandidate],
    /// The `agent` rows found for them; an absent key is [`SkipCause::NoAgentRow`].
    pub agents: &'a BTreeMap<AgentId, Agent>,
    /// This box's `agent_box` rows; an absent key is *unknown*, and unknown does not skip.
    pub boxes: &'a BTreeMap<AgentId, AgentBox>,
    /// The phase's `gate_effective`: any gate but `never` needs an inline-approval transport.
    pub gate_effective: Gate,
    /// What the run has spent so far, in USD micros ([`run_spend`]); `None` when unknown.
    pub spent_micros: Option<i64>,
    /// `snapshot.settings.per_token_cap_run`, in USD micros; `None` is unbounded.
    pub cap_micros: Option<i64>,
    /// `app_setting.min_budget_for_new_attempt`, in USD micros, else `0` (OQ-6).
    pub min_budget_micros: i64,
}

/// Why [`walk`] skipped a candidate: the first of D60's five rules that fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipCause {
    /// Rule 1: the candidate's `agent_id` has no `agent` row.
    NoAgentRow,
    /// Rule 2: ANA-4 §7's quota predicate (`quota::available`) skipped the row.
    Quota(SkipReason),
    /// Rule 3: the phase is gated and the transport can answer neither a permission request nor
    /// an edit proposal inline (`docs/ANA-2.md:482`).
    InlineApproval,
    /// Rule 4: the agent or its `agent_box` row is disabled, or its probe is not `ready`.
    NotReady(String),
    /// Rule 5: the run's remaining budget is below `min_budget_for_new_attempt`.
    Budget {
        /// `cap - spent`, in USD micros.
        remaining: i64,
        /// The minimum a new attempt needs, in USD micros.
        min: i64,
    },
}

impl fmt::Display for SkipCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAgentRow => f.write_str("no agent row"),
            Self::Quota(SkipReason::Exhausted) => f.write_str("quota: exhausted"),
            Self::Quota(SkipReason::WindowFull { id }) => write!(f, "quota: window {id} full"),
            Self::Quota(SkipReason::Status(status)) => write!(f, "quota: status {status}"),
            Self::Quota(SkipReason::CapReached {
                spent_micros,
                cap_micros,
            }) => write!(
                f,
                "quota: cap reached ({spent_micros} of {cap_micros} micros)"
            ),
            Self::InlineApproval => f.write_str("inline_approval"),
            Self::NotReady(why) => write!(f, "not ready: {why}"),
            Self::Budget { remaining, min } => {
                write!(f, "budget: {remaining} micros left, {min} required")
            }
        }
    }
}

/// One candidate [`walk`] skipped, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// `phase_agent.agent_id`.
    pub agent_id: AgentId,
    /// The snapshot's denormalised `agent.name`, so a note needs no registry.
    pub agent_name: String,
    /// The first rule that fired.
    pub cause: SkipCause,
}

/// [`walk`]'s answer: the eligible candidates and the skipped ones, both in candidate order.
///
/// `PartialEq` without `Eq`: `SnapshotCandidate` derives no `Eq`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Walk {
    /// Candidates no rule skipped, in `phase_agent` order.
    pub eligible: Vec<SnapshotCandidate>,
    /// Candidates a rule skipped, in `phase_agent` order.
    pub skipped: Vec<Skipped>,
}

impl Walk {
    /// Nothing is eligible and every skip was [`SkipCause::InlineApproval`]: the shipped
    /// `missing_capability: inline_approval` refusal rather than `no_candidate_agent` (D62).
    #[must_use]
    pub fn only_inline_approval(&self) -> bool {
        !self.skipped.is_empty()
            && self.eligible.is_empty()
            && self
                .skipped
                .iter()
                .all(|skipped| skipped.cause == SkipCause::InlineApproval)
    }

    /// `"<name> (<cause>), <name> (<cause>)"` in candidate order; `""` when nothing was skipped.
    #[must_use]
    pub fn summary(&self) -> String {
        self.skipped
            .iter()
            .map(|skipped| format!("{} ({})", skipped.agent_name, skipped.cause))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// D60's walk: each candidate is kept or skipped by the first of five rules that fires.
///
/// 1. no `agent` row → [`SkipCause::NoAgentRow`];
/// 2. `quota::available` over the box row's quota and the caller's spend and cap skips →
///    [`SkipCause::Quota`] (it runs with an absent box row too, so the cap rule still applies);
/// 3. a gate other than `never` and a transport with neither `permission_requests` nor
///    `edit_proposals` → [`SkipCause::InlineApproval`];
/// 4. a disabled agent, a disabled box row, or a probe that parses with a status other than
///    `ready` → [`SkipCause::NotReady`]; an absent or unparseable probe is unknown and does not
///    skip;
/// 5. spend and cap both known with `cap - spent < min_budget` → [`SkipCause::Budget`].
#[must_use]
pub fn walk(input: &SelectInput<'_>) -> Walk {
    let mut walked = Walk::default();
    for candidate in input.candidates {
        match skip_cause(input, candidate) {
            None => walked.eligible.push(candidate.clone()),
            Some(cause) => walked.skipped.push(Skipped {
                agent_id: candidate.agent_id,
                agent_name: candidate.agent_name.clone(),
                cause,
            }),
        }
    }
    walked
}

/// The first of D60's five rules that skips `candidate`, or `None` when it is eligible.
fn skip_cause(input: &SelectInput<'_>, candidate: &SnapshotCandidate) -> Option<SkipCause> {
    let Some(agent) = input.agents.get(&candidate.agent_id) else {
        return Some(SkipCause::NoAgentRow);
    };
    let row = input.boxes.get(&candidate.agent_id);
    if let Availability::Skip(reason) = quota::available(
        row.and_then(|row| row.quota.as_ref()),
        input.spent_micros,
        input.cap_micros,
    ) {
        return Some(SkipCause::Quota(reason));
    }
    if input.gate_effective != Gate::Never {
        let caps = caps_for(agent);
        if !(caps.permission_requests || caps.edit_proposals) {
            return Some(SkipCause::InlineApproval);
        }
    }
    if let Some(why) = not_ready(agent, row) {
        return Some(SkipCause::NotReady(why));
    }
    if let (Some(spent), Some(cap)) = (input.spent_micros, input.cap_micros) {
        let remaining = cap.saturating_sub(spent);
        if remaining < input.min_budget_micros {
            return Some(SkipCause::Budget {
                remaining,
                min: input.min_budget_micros,
            });
        }
    }
    None
}

/// Rule 4's reason, or `None`: an absent box row and an absent or unparseable probe are unknown,
/// and unknown is ready (ANA-2 §7's "unknown is available", extended to rows never probed).
fn not_ready(agent: &Agent, row: Option<&AgentBox>) -> Option<String> {
    if !agent.enabled {
        return Some("agent disabled".to_owned());
    }
    let row = row?;
    if !row.enabled {
        return Some("agent_box disabled".to_owned());
    }
    ProbeSnapshot::from_row(row)
        .filter(|probe| probe.status != ProbeStatus::Ready)
        .map(|probe| format!("probe {}", probe.status.as_str()))
}

/// `Σ usage["cost_micros"]` over `steps` (`model/run.rs:247`, `model/usage.rs:37`); `None` when no
/// step reports one.
#[must_use]
pub fn run_spend(steps: &[RunStep]) -> Option<i64> {
    steps
        .iter()
        .filter_map(|step| step.usage.as_ref()?.get("cost_micros")?.as_i64())
        .fold(None, |total, cost| {
            Some(total.unwrap_or(0).saturating_add(cost))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, Utc};
    use htui_agent::probe::ProbeSnapshot;
    use htui_core::fixtures::{demo_data, ids};
    use htui_core::model::{
        Agent, AgentBox, AgentId, Billing, BoxId, Gate, Quota, QuotaSource, QuotaWindow, RunStep,
        SkipReason, SnapshotCandidate, Spend,
    };
    use serde_json::{Value, json};

    use super::{SelectInput, SkipCause, Skipped, Walk, run_spend, walk};

    /// The module's fixed instant: `2026-09-03T00:00:00Z`.
    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    /// The demo fixture's three seeded agents, keyed by id.
    fn agents() -> BTreeMap<AgentId, Agent> {
        demo_data()
            .agents
            .into_iter()
            .map(|agent| (agent.id, agent))
            .collect()
    }

    fn candidate(agent_id: AgentId, name: &str) -> SnapshotCandidate {
        SnapshotCandidate {
            agent_id,
            agent_name: name.to_owned(),
            model: "default".to_owned(),
        }
    }

    /// `claude` then `agy`, both `acp`: two candidates no rule skips on a bare fixture.
    fn two_acp() -> Vec<SnapshotCandidate> {
        vec![
            candidate(ids::AGENT_CLAUDE, "claude"),
            candidate(ids::AGENT_AGY, "agy"),
        ]
    }

    /// An enabled, never-probed, never-latched `agent_box` row.
    fn box_row(agent_id: AgentId) -> AgentBox {
        AgentBox {
            agent_id,
            box_id: BoxId::new(),
            enabled: true,
            version: None,
            path: None,
            probed_at: None,
            quota: None,
            quota_at: None,
            updated_at: at(),
            probe: None,
        }
    }

    /// A stored ANA-4 §7 document, built through the type that writes it (`quota.rs:940-958`).
    fn quota_document(status: Option<&str>, exhausted: bool, windows: &[(&str, f64)]) -> Value {
        Quota {
            source: QuotaSource::AcpMetaRateLimit,
            billing: Billing::Subscription,
            status: status.map(str::to_owned),
            exhausted,
            windows: windows
                .iter()
                .map(|(id, utilization)| QuotaWindow {
                    id: (*id).to_owned(),
                    utilization: *utilization,
                    resets_at: None,
                })
                .collect(),
            spend: Spend::default(),
            observed_at: at(),
        }
        .to_value()
    }

    /// A probe document with `status`, checked to parse so a test cannot pass on an unparseable
    /// (and therefore unknown) probe by accident.
    fn probe(status: &str) -> Value {
        let document = json!({
            "transport": "acp",
            "resolved": null,
            "tools": {},
            "handshake": null,
            "status": status,
            "stderr_tail": null,
        });
        let mut row = box_row(ids::AGENT_CLAUDE);
        row.probe = Some(document.clone());
        assert!(
            ProbeSnapshot::from_row(&row).is_some(),
            "the test's probe document parses"
        );
        document
    }

    fn boxes_of(rows: impl IntoIterator<Item = AgentBox>) -> BTreeMap<AgentId, AgentBox> {
        rows.into_iter().map(|row| (row.agent_id, row)).collect()
    }

    fn input<'a>(
        candidates: &'a [SnapshotCandidate],
        agents: &'a BTreeMap<AgentId, Agent>,
        boxes: &'a BTreeMap<AgentId, AgentBox>,
    ) -> SelectInput<'a> {
        SelectInput {
            candidates,
            agents,
            boxes,
            gate_effective: Gate::Never,
            spent_micros: None,
            cap_micros: None,
            min_budget_micros: 0,
        }
    }

    /// The one cause `walk` gave the first candidate, which must be skipped.
    fn first_cause(walked: &Walk) -> &SkipCause {
        &walked
            .skipped
            .first()
            .expect("a candidate was skipped")
            .cause
    }

    /// D60: the walk keeps `phase_agent` order, so the first unskipped candidate is the first
    /// eligible one — which is what `FirstCandidate` then picks.
    #[test]
    fn the_first_unskipped_candidate_is_eligible_first() {
        let candidates = [
            candidate(AgentId::new(), "ghost"),
            candidate(ids::AGENT_CLAUDE, "claude"),
            candidate(ids::AGENT_AGY, "agy"),
        ];
        let agents = agents();
        let boxes = BTreeMap::new();
        let walked = walk(&input(&candidates, &agents, &boxes));
        assert_eq!(
            walked.eligible,
            candidates[1..].to_vec(),
            "the unskipped candidates, in order"
        );
        assert_eq!(
            walked.skipped,
            [Skipped {
                agent_id: candidates[0].agent_id,
                agent_name: "ghost".to_owned(),
                cause: SkipCause::NoAgentRow,
            }]
        );
        assert_eq!(walked.summary(), "ghost (no agent row)");
        assert_eq!(
            Walk::default().summary(),
            "",
            "nothing skipped, nothing said"
        );
    }

    /// Rule 2 is ANA-4 §7's predicate, and the cause carries its reason verbatim.
    #[test]
    fn an_exhausted_quota_skips_and_names_the_rule() {
        let candidates = two_acp();
        let agents = agents();
        let cases = [
            (
                quota_document(Some("allowed"), true, &[]),
                SkipCause::Quota(SkipReason::Exhausted),
                "quota: exhausted",
            ),
            (
                quota_document(Some("allowed"), false, &[("seven_day", 1.0)]),
                SkipCause::Quota(SkipReason::WindowFull {
                    id: "seven_day".to_owned(),
                }),
                "quota: window seven_day full",
            ),
            (
                quota_document(Some("rejected"), false, &[]),
                SkipCause::Quota(SkipReason::Status("rejected".to_owned())),
                "quota: status rejected",
            ),
        ];
        for (document, cause, bytes) in cases {
            let mut row = box_row(ids::AGENT_CLAUDE);
            row.quota = Some(document);
            let boxes = boxes_of([row]);
            let walked = walk(&input(&candidates, &agents, &boxes));
            assert_eq!(first_cause(&walked), &cause);
            assert_eq!(cause.to_string(), bytes, "byte-exact");
            assert_eq!(walked.summary(), format!("claude ({bytes})"));
            assert_eq!(walked.eligible, candidates[1..].to_vec(), "agy still runs");
        }
    }

    /// Unknown is available: no box row, a box row with no quota, and an unparseable probe or
    /// quota blob all select.
    #[test]
    fn a_missing_agent_box_row_is_unknown_and_selected() {
        let candidates = two_acp();
        let agents = agents();
        let none = BTreeMap::new();
        let walked = walk(&input(&candidates, &agents, &none));
        assert_eq!(walked.eligible, candidates, "no box rows at all");
        assert!(walked.skipped.is_empty());

        let mut garbled = box_row(ids::AGENT_AGY);
        garbled.probe = Some(json!({ "not": "a probe" }));
        garbled.quota = Some(json!({ "remaining": 100 }));
        let boxes = boxes_of([box_row(ids::AGENT_CLAUDE), garbled]);
        let walked = walk(&input(&candidates, &agents, &boxes));
        assert_eq!(
            walked.eligible, candidates,
            "an unparseable probe or quota is unknown"
        );
    }

    /// Rule 4's probe half: a parsed status other than `ready` skips; `ready` does not.
    #[test]
    fn a_probe_that_is_not_ready_skips() {
        let candidates = two_acp();
        let agents = agents();
        for (status, skipped) in [
            ("failed", Some("not ready: probe failed")),
            ("missing", Some("not ready: probe missing")),
            ("unauthenticated", Some("not ready: probe unauthenticated")),
            ("ready", None),
        ] {
            let mut row = box_row(ids::AGENT_CLAUDE);
            row.probe = Some(probe(status));
            let boxes = boxes_of([row]);
            let walked = walk(&input(&candidates, &agents, &boxes));
            match skipped {
                Some(bytes) => {
                    assert_eq!(
                        first_cause(&walked),
                        &SkipCause::NotReady(format!("probe {status}"))
                    );
                    assert_eq!(first_cause(&walked).to_string(), bytes);
                }
                None => assert_eq!(walked.eligible, candidates, "`ready` is ready"),
            }
        }
    }

    #[test]
    fn a_disabled_agent_or_agent_box_skips() {
        let candidates = two_acp();
        let mut agents = agents();
        agents
            .get_mut(&ids::AGENT_CLAUDE)
            .expect("the fixture seeds claude")
            .enabled = false;
        let none = BTreeMap::new();
        let walked = walk(&input(&candidates, &agents, &none));
        assert_eq!(
            first_cause(&walked),
            &SkipCause::NotReady("agent disabled".to_owned())
        );
        assert_eq!(walked.summary(), "claude (not ready: agent disabled)");

        let agents = self::agents();
        let mut row = box_row(ids::AGENT_CLAUDE);
        row.enabled = false;
        let boxes = boxes_of([row]);
        let walked = walk(&input(&candidates, &agents, &boxes));
        assert_eq!(
            first_cause(&walked),
            &SkipCause::NotReady("agent_box disabled".to_owned())
        );
        assert_eq!(walked.eligible, candidates[1..].to_vec());
    }

    /// Rule 3: `claude-cli` is `transport: cli` (`seeds/agent_claude_cli.json`), which answers no
    /// permission and proposes no edit, so it is skipped at `always` and `on_failure` only.
    #[test]
    fn a_cli_row_is_skipped_only_at_a_gated_phase() {
        let candidates = [candidate(ids::AGENT_CLAUDE_CLI, "claude-cli")];
        let agents = agents();
        let boxes = BTreeMap::new();
        for gate in [Gate::Always, Gate::OnFailure] {
            let walked = walk(&SelectInput {
                gate_effective: gate,
                ..input(&candidates, &agents, &boxes)
            });
            assert_eq!(first_cause(&walked), &SkipCause::InlineApproval, "{gate:?}");
            assert_eq!(walked.summary(), "claude-cli (inline_approval)");
        }
        let walked = walk(&input(&candidates, &agents, &boxes));
        assert_eq!(walked.eligible, candidates, "`never` needs no interlock");

        let acp = two_acp();
        let walked = walk(&SelectInput {
            gate_effective: Gate::Always,
            ..input(&acp, &agents, &boxes)
        });
        assert_eq!(walked.eligible, acp, "an acp row answers inline");
    }

    /// The cap rule reads the caller's run spend, and applies with or without a box row.
    #[test]
    fn the_per_run_cap_is_the_callers_spend() {
        let candidates = two_acp();
        let agents = agents();
        let none = BTreeMap::new();
        let reached = walk(&SelectInput {
            spent_micros: Some(500),
            cap_micros: Some(500),
            ..input(&candidates, &agents, &none)
        });
        let cause = SkipCause::Quota(SkipReason::CapReached {
            spent_micros: 500,
            cap_micros: 500,
        });
        assert!(reached.eligible.is_empty(), "the cap binds every candidate");
        assert!(reached.skipped.iter().all(|skipped| skipped.cause == cause));
        assert_eq!(cause.to_string(), "quota: cap reached (500 of 500 micros)");

        let boxes = boxes_of([box_row(ids::AGENT_CLAUDE)]);
        let under = walk(&SelectInput {
            spent_micros: Some(499),
            cap_micros: Some(500),
            ..input(&candidates, &agents, &boxes)
        });
        assert_eq!(under.eligible, candidates, "below the cap");
    }

    /// OQ-6: `min_budget_for_new_attempt` needs both figures; either unknown is unbounded.
    #[test]
    fn min_budget_skips_only_when_cap_and_spend_are_known() {
        let candidates = two_acp();
        let agents = agents();
        let boxes = BTreeMap::new();
        let base = SelectInput {
            min_budget_micros: 200,
            ..input(&candidates, &agents, &boxes)
        };
        let short = walk(&SelectInput {
            spent_micros: Some(900),
            cap_micros: Some(1_000),
            ..base
        });
        let cause = SkipCause::Budget {
            remaining: 100,
            min: 200,
        };
        assert_eq!(first_cause(&short), &cause);
        assert_eq!(cause.to_string(), "budget: 100 micros left, 200 required");
        assert!(short.eligible.is_empty());

        let enough = walk(&SelectInput {
            spent_micros: Some(800),
            cap_micros: Some(1_000),
            ..base
        });
        assert_eq!(enough.eligible, candidates, "exactly the minimum is enough");
        for (spent, cap) in [(None, Some(1_000)), (Some(900), None), (None, None)] {
            let walked = walk(&SelectInput {
                spent_micros: spent,
                cap_micros: cap,
                ..base
            });
            assert_eq!(walked.eligible, candidates, "{spent:?} of {cap:?}");
        }
    }

    /// D62: only a walk whose every skip is the interlock is `missing_capability`.
    #[test]
    fn every_skip_being_inline_approval_is_reported_as_such() {
        let agents = agents();
        let boxes = BTreeMap::new();
        let gated = |candidates: &[SnapshotCandidate]| {
            walk(&SelectInput {
                gate_effective: Gate::Always,
                ..input(candidates, &agents, &boxes)
            })
        };
        let cli = [candidate(ids::AGENT_CLAUDE_CLI, "claude-cli")];
        assert!(gated(&cli).only_inline_approval());

        let mixed = [cli[0].clone(), candidate(AgentId::new(), "ghost")];
        let walked = gated(&mixed);
        assert!(walked.eligible.is_empty());
        assert!(!walked.only_inline_approval(), "one skip is `no agent row`");
        assert_eq!(
            walked.summary(),
            "claude-cli (inline_approval), ghost (no agent row)"
        );

        let with_acp = [cli[0].clone(), candidate(ids::AGENT_CLAUDE, "claude")];
        assert!(
            !gated(&with_acp).only_inline_approval(),
            "something is eligible"
        );
        assert!(
            !Walk::default().only_inline_approval(),
            "nothing was skipped"
        );
    }

    /// D60's rules are "first match wins": where two rules would skip one candidate, the earlier
    /// one is the recorded cause, and that choice decides D62's `only_inline_approval`.
    #[test]
    fn the_first_matching_rule_is_the_cause() {
        let agents = agents();
        let exhausted = || {
            let mut row = box_row(ids::AGENT_CLAUDE_CLI);
            row.quota = Some(quota_document(Some("allowed"), true, &[]));
            row
        };

        // Rule 1 before rule 2: a ghost with an exhausted box row has no agent row.
        let ghost = AgentId::new();
        let ghost_candidates = [candidate(ghost, "ghost")];
        let mut ghost_row = exhausted();
        ghost_row.agent_id = ghost;
        let boxes = boxes_of([ghost_row]);
        let walked = walk(&input(&ghost_candidates, &agents, &boxes));
        assert_eq!(
            first_cause(&walked),
            &SkipCause::NoAgentRow,
            "rule 1 before 2"
        );

        // Rule 2 before rule 3: a gated cli row with an exhausted quota is a quota skip.
        let cli = [candidate(ids::AGENT_CLAUDE_CLI, "claude-cli")];
        let boxes = boxes_of([exhausted()]);
        let walked = walk(&SelectInput {
            gate_effective: Gate::Always,
            ..input(&cli, &agents, &boxes)
        });
        assert_eq!(
            first_cause(&walked),
            &SkipCause::Quota(SkipReason::Exhausted),
            "rule 2 before 3"
        );
        assert!(!walked.only_inline_approval(), "a quota skip is no refusal");

        // Rule 3 before rule 4: a gated, disabled cli row is the interlock.
        let mut disabled = agents.clone();
        disabled
            .get_mut(&ids::AGENT_CLAUDE_CLI)
            .expect("the fixture seeds claude-cli")
            .enabled = false;
        let none = BTreeMap::new();
        let walked = walk(&SelectInput {
            gate_effective: Gate::Always,
            ..input(&cli, &disabled, &none)
        });
        assert_eq!(
            first_cause(&walked),
            &SkipCause::InlineApproval,
            "rule 3 before 4"
        );
        assert!(walked.only_inline_approval());

        // Rule 4 before rule 5: a failed probe on a short budget is not ready.
        let acp = [candidate(ids::AGENT_CLAUDE, "claude")];
        let mut failed = box_row(ids::AGENT_CLAUDE);
        failed.probe = Some(probe("failed"));
        let boxes = boxes_of([failed]);
        let walked = walk(&SelectInput {
            spent_micros: Some(900),
            cap_micros: Some(1_000),
            min_budget_micros: 200,
            ..input(&acp, &agents, &boxes)
        });
        assert_eq!(
            first_cause(&walked),
            &SkipCause::NotReady("probe failed".to_owned()),
            "rule 4 before 5"
        );
    }

    #[test]
    fn run_spend_sums_cost_micros() {
        let template = demo_data()
            .steps
            .into_iter()
            .next()
            .expect("the fixture holds a step");
        let step = |usage: Option<Value>| RunStep {
            usage,
            ..template.clone()
        };
        let steps = [
            step(Some(json!({ "cost_micros": 1_500, "input_tokens": 10 }))),
            step(None),
            step(Some(json!({ "input_tokens": 3 }))),
            step(Some(json!({ "cost_micros": 250 }))),
        ];
        assert_eq!(run_spend(&steps), Some(1_750));
        assert_eq!(run_spend(&steps[1..3]), None, "no step reports a cost");
        assert_eq!(run_spend(&[]), None);
    }
}
