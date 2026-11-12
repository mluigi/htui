//! MOD-141 M2 D11 (M-4): what a run of one step would be refused with today, in
//! `htui_orch::graph::resolve`'s own sentences and order. A warning beside the form and a `!` on
//! the canvas, never a refused save. Pure: no keys, no store.

use htui_core::model::{Isolation, NodeBody, PhasePart, PlanScope, Split};
use htui_orch::divide::owned_path_refusal;
use htui_orch::graph::{NotRunnable, ResolveError};
use htui_orch::status::{PlanCap, RunFailure};

/// MOD-141 M2 D11 (M-4): every sentence `htui_orch::graph::resolve` would refuse a run of `node`
/// with today, each the `ResolveError` resolve raises, in resolve's order; empty when none.
/// `max_fan_out` is `RunBounds::max_fan_out` once the editor's settings read answered; `None`
/// skips the two checks that need it. Known limits: `max_agents_per_run` needs the whole graph,
/// the project's default isolation and candidate rungs are not visible here, and the graph-wide
/// checks (`FanOutEdge`, `LocalConcurrent`, `ReviewFanOut`) stay resolve's.
///
/// Resolve stops at its first refusal; this lists every check that fails (C11), each in the
/// sentence resolve would give were it the first.
#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]
pub(crate) fn notes(node: &NodeBody, max_fan_out: Option<u32>) -> Vec<String> {
    let phase = &node.phase;
    let name = &phase.name;
    let divide = phase.split == Split::Divide;
    // `not_yet_runnable` names a planner node by `divided_name`: the divided node's own name.
    let not_yet = |reason| {
        ResolveError::NotYetRunnable {
            node: name.clone(),
            reason,
        }
        .to_string()
    };
    let mut notes = Vec::new();
    if divide && phase.planner_agent_id.is_some() {
        notes.push(not_yet(NotRunnable::PlannerAgent));
    }
    if divide && node.parts.is_empty() {
        notes.push(not_yet(NotRunnable::NoParts));
    }
    if divide && !matches!(phase.plan_scope, None | Some(PlanScope::Parts)) {
        notes.push(not_yet(NotRunnable::PlanScope));
    }
    if phase.palette {
        notes.push(not_yet(NotRunnable::Palette));
    }
    // `check_fan_out_caps`' first cap; its second, `max_agents_per_run`, needs the whole graph.
    if let Some(max) = max_fan_out
        && u32::try_from(phase.fan_out).is_ok_and(|fan_out| fan_out > max)
    {
        notes.push(
            ResolveError::FanOutCap {
                phase: name.clone(),
                fan_out: phase.fan_out,
                max,
            }
            .to_string(),
        );
    }
    // `divide_refusal`: what admission would refuse of the default parts. No part is `NoParts`
    // above, so `EmptyPlan` never shows.
    if divide && !node.parts.is_empty() {
        notes.extend(part_refusals(node, max_fan_out).into_iter().map(|reason| {
            ResolveError::DivideRefused {
                node: name.clone(),
                reason,
            }
            .to_string()
        }));
    }
    notes
}

/// Every refusal `htui_orch::divide::admission_refusal` would answer for `node`'s default parts,
/// in its order: more than one part under the step's own `local`; `max_parts`, then
/// `max_fan_out` (`None` skips it); then per part by position a persona, each owned path that is
/// not a repo-relative glob, and an agent that is none of the listed candidates (none listed:
/// the candidates come from rungs not visible here, so resolve judges it).
fn part_refusals(node: &NodeBody, max_fan_out: Option<u32>) -> Vec<RunFailure> {
    let phase = &node.phase;
    let name = || phase.name.clone();
    let count = u32::try_from(node.parts.len()).unwrap_or(u32::MAX);
    let mut refusals = Vec::new();
    if phase.isolation == Some(Isolation::Local) && count > 1 {
        refusals.push(RunFailure::LocalCannotDivide {
            node: name(),
            parts: count,
        });
    }
    let over = |cap, max| RunFailure::PlanOverCaps {
        node: name(),
        cap,
        planned: count,
        max,
    };
    if let Some(max) = phase.max_parts {
        let max = u32::try_from(max).unwrap_or(0);
        if count > max {
            refusals.push(over(PlanCap::MaxParts, max));
        }
    }
    if let Some(max) = max_fan_out
        && count > max
    {
        refusals.push(over(PlanCap::MaxFanOut, max));
    }
    let mut ordered: Vec<&PhasePart> = node.parts.iter().collect();
    ordered.sort_by_key(|part| part.position);
    for part in ordered {
        if part.persona_id.is_some() {
            refusals.push(RunFailure::PartPersonaUnsupported {
                node: name(),
                part: part.key.clone(),
            });
        }
        for path in &part.owned_paths {
            let path = path.trim();
            if let Some(reason) = owned_path_refusal(path) {
                refusals.push(RunFailure::PartPathRefused {
                    node: name(),
                    part: part.key.clone(),
                    path: path.to_owned(),
                    reason,
                });
            }
        }
        if let Some(agent) = part.agent_id
            && !node.agents.is_empty()
            && !node
                .agents
                .iter()
                .any(|candidate| candidate.agent_id == agent)
        {
            refusals.push(RunFailure::PartAgentNotCandidate {
                node: name(),
                part: part.key.clone(),
                agent,
            });
        }
    }
    refusals
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use htui_core::model::{
        AgentId, Isolation, NodeBody, PersonaId, PhaseAgent, PhaseId, PhasePart, PlanScope, Split,
        StepGraphId,
    };
    use htui_core::seed::{PhaseSeed, phase_row};
    use htui_orch::graph::{NotRunnable, ResolveError};
    use htui_orch::status::{PlanCap, RunFailure};

    use super::notes;

    /// A plain step named `name`: the seed's row, its name everywhere.
    fn step(name: &str) -> NodeBody {
        let seed = PhaseSeed {
            name: "",
            input_kinds: &[],
            gate_hard: false,
        };
        let mut phase = phase_row(PhaseId::new(), StepGraphId::new(), 0, &seed, Utc::now());
        phase.name = name.to_owned();
        phase.output_kind = name.to_owned();
        phase.template_name = name.to_owned();
        NodeBody {
            phase,
            agents: Vec::new(),
            parts: Vec::new(),
        }
    }

    /// A default part `key` of `phase` at `position`, owning `src/<key>/**`: one admission takes.
    fn part(phase: PhaseId, position: i32, key: &str) -> PhasePart {
        PhasePart {
            phase_id: phase,
            position,
            key: key.to_owned(),
            title: key.to_owned(),
            brief: String::new(),
            owned_paths: vec![format!("src/{key}/**")],
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    /// A divide step named `name` with one clean default part per key, positions 0...
    fn divide(name: &str, keys: &[&str]) -> NodeBody {
        let mut node = step(name);
        node.phase.split = Split::Divide;
        let phase = node.phase.id;
        node.parts = keys
            .iter()
            .zip(0..)
            .map(|(key, position)| part(phase, position, key))
            .collect();
        node
    }

    /// `agent` as a candidate of `node`, after the ones it has.
    fn candidate(node: &mut NodeBody, agent: AgentId) {
        let position = i32::try_from(node.agents.len()).expect("a few candidates");
        node.agents.push(PhaseAgent {
            phase_id: node.phase.id,
            position,
            agent_id: agent,
            model: "m".to_owned(),
        });
    }

    /// Resolve's sentence for `reason` on `node`.
    fn not_yet(node: &str, reason: NotRunnable) -> String {
        ResolveError::NotYetRunnable {
            node: node.to_owned(),
            reason,
        }
        .to_string()
    }

    /// Resolve's sentence for an admission refusal of `node`'s default parts.
    fn refused(node: &str, reason: RunFailure) -> String {
        ResolveError::DivideRefused {
            node: node.to_owned(),
            reason,
        }
        .to_string()
    }

    #[test]
    fn a_clean_step_has_no_notes() {
        let plain = step("implement");
        let mut compete = step("draft");
        compete.phase.split = Split::Compete;
        compete.phase.fan_out = 3;
        let mut parts = divide("build", &["api", "ui"]);
        parts.phase.plan_scope = Some(PlanScope::Parts);
        candidate(&mut parts, AgentId::new());
        for node in [&plain, &compete, &parts] {
            for max_fan_out in [None, Some(4)] {
                assert_eq!(
                    notes(node, max_fan_out),
                    Vec::<String>::new(),
                    "{} under {max_fan_out:?}",
                    node.phase.name
                );
            }
        }
    }

    #[test]
    fn a_planner_agent_is_noted_first() {
        let mut node = divide("implement", &[]);
        node.phase.planner_agent_id = Some(AgentId::new());
        let notes = notes(&node, Some(4));
        assert_eq!(
            notes,
            vec![
                not_yet("implement", NotRunnable::PlannerAgent),
                not_yet("implement", NotRunnable::NoParts),
            ]
        );
        assert_eq!(
            notes[0],
            format!(
                "phase `implement` cannot run yet: {}",
                NotRunnable::PlannerAgent
            ),
            "resolve's sentence around the `NotRunnable` text"
        );

        // Off a divide node the planner column means nothing to resolve.
        let mut plain = step("implement");
        plain.phase.planner_agent_id = Some(AgentId::new());
        assert_eq!(notes_of(&plain), Vec::<String>::new());
    }

    #[test]
    fn a_part_less_divide_is_noted_no_parts() {
        let mut node = divide("implement", &[]);
        node.phase.max_parts = Some(1);
        node.phase.isolation = Some(Isolation::Local);
        assert_eq!(
            notes(&node, Some(1)),
            vec![not_yet("implement", NotRunnable::NoParts)],
            "no part checks run on no parts: never `EmptyPlan`"
        );
    }

    #[test]
    fn a_plan_scope_past_parts_is_noted() {
        let mut node = divide("implement", &["api"]);
        for scope in [PlanScope::Successors, PlanScope::Graph] {
            node.phase.plan_scope = Some(scope);
            assert_eq!(
                notes_of(&node),
                vec![not_yet("implement", NotRunnable::PlanScope)],
                "{scope}"
            );
        }
        for scope in [None, Some(PlanScope::Parts)] {
            node.phase.plan_scope = scope;
            assert_eq!(notes_of(&node), Vec::<String>::new(), "{scope:?}");
        }

        // A stored scope on a node that does not divide is kept, not noted.
        let mut plain = step("implement");
        plain.phase.plan_scope = Some(PlanScope::Graph);
        assert_eq!(notes_of(&plain), Vec::<String>::new());
    }

    #[test]
    fn a_palette_step_is_noted() {
        let mut node = step("security");
        node.phase.palette = true;
        assert_eq!(
            notes_of(&node),
            vec![not_yet("security", NotRunnable::Palette)]
        );
    }

    #[test]
    fn a_compete_fan_out_above_max_fan_out_is_noted() {
        let mut node = step("draft");
        node.phase.split = Split::Compete;
        node.phase.fan_out = 5;
        assert_eq!(
            notes(&node, Some(4)),
            vec![
                ResolveError::FanOutCap {
                    phase: "draft".to_owned(),
                    fan_out: 5,
                    max: 4,
                }
                .to_string()
            ]
        );
        assert_eq!(notes(&node, Some(5)), Vec::<String>::new(), "at the cap");
    }

    /// 6a-6d, each tripped once by one divide node, in admission's order: `local` with more
    /// than one part, `max_parts`, `max_fan_out`, then per part by position its persona, each
    /// owned path and its agent.
    #[test]
    fn the_default_parts_are_judged_as_admission_judges_them() {
        let mut node = divide("implement", &[]);
        let phase = node.phase.id;
        node.phase.isolation = Some(Isolation::Local);
        node.phase.max_parts = Some(2);
        let (listed, stranger) = (AgentId::new(), AgentId::new());
        candidate(&mut node, listed);
        // Body order is not position order: `api` is judged first.
        let mut ui = part(phase, 2, "ui");
        ui.agent_id = Some(stranger);
        let mut docs = part(phase, 1, "docs");
        docs.owned_paths = vec!["  /etc/x ".to_owned()];
        let mut api = part(phase, 0, "api");
        api.persona_id = Some(PersonaId::new());
        node.parts = vec![ui, docs, api];

        let name = || "implement".to_owned();
        let local = RunFailure::LocalCannotDivide {
            node: name(),
            parts: 3,
        };
        let notes = notes(&node, Some(2));
        assert_eq!(
            notes,
            vec![
                refused("implement", local.clone()),
                refused(
                    "implement",
                    RunFailure::PlanOverCaps {
                        node: name(),
                        cap: PlanCap::MaxParts,
                        planned: 3,
                        max: 2,
                    }
                ),
                refused(
                    "implement",
                    RunFailure::PlanOverCaps {
                        node: name(),
                        cap: PlanCap::MaxFanOut,
                        planned: 3,
                        max: 2,
                    }
                ),
                refused(
                    "implement",
                    RunFailure::PartPersonaUnsupported {
                        node: name(),
                        part: "api".to_owned(),
                    }
                ),
                refused(
                    "implement",
                    RunFailure::PartPathRefused {
                        node: name(),
                        part: "docs".to_owned(),
                        path: "/etc/x".to_owned(),
                        reason: "is an absolute path".to_owned(),
                    }
                ),
                refused(
                    "implement",
                    RunFailure::PartAgentNotCandidate {
                        node: name(),
                        part: "ui".to_owned(),
                        agent: stranger,
                    }
                ),
            ]
        );
        assert_eq!(
            notes[0],
            local.to_string(),
            "a divide refusal reads as the admission's own sentence"
        );

        // One part under `local` runs: only the stranger is left.
        node.parts.truncate(1);
        assert_eq!(
            notes_of(&node),
            vec![refused(
                "implement",
                RunFailure::PartAgentNotCandidate {
                    node: name(),
                    part: "ui".to_owned(),
                    agent: stranger,
                }
            )]
        );
    }

    /// C11: admission answers its first refusal; the editor lists them all.
    #[test]
    fn every_failing_check_is_listed_not_only_the_first() {
        let mut node = divide("implement", &["api", "ui"]);
        node.phase.planner_agent_id = Some(AgentId::new());
        node.phase.plan_scope = Some(PlanScope::Graph);
        node.phase.palette = true;
        node.parts[0].owned_paths = vec!["/etc/x".to_owned(), "src/../x".to_owned()];
        node.parts[0].persona_id = Some(PersonaId::new());
        node.parts[1].persona_id = Some(PersonaId::new());

        let name = || "implement".to_owned();
        let path = |path: &str, reason: &str| {
            refused(
                "implement",
                RunFailure::PartPathRefused {
                    node: name(),
                    part: "api".to_owned(),
                    path: path.to_owned(),
                    reason: reason.to_owned(),
                },
            )
        };
        let persona = |part: &str| {
            refused(
                "implement",
                RunFailure::PartPersonaUnsupported {
                    node: name(),
                    part: part.to_owned(),
                },
            )
        };
        assert_eq!(
            notes(&node, Some(4)),
            vec![
                not_yet("implement", NotRunnable::PlannerAgent),
                not_yet("implement", NotRunnable::PlanScope),
                not_yet("implement", NotRunnable::Palette),
                persona("api"),
                path("/etc/x", "is an absolute path"),
                path("src/../x", "leaves its repository through `..`"),
                persona("ui"),
            ]
        );
    }

    #[test]
    fn without_max_fan_out_the_two_cap_checks_are_skipped() {
        let mut compete = step("draft");
        compete.phase.split = Split::Compete;
        compete.phase.fan_out = 9;
        assert_eq!(notes(&compete, None), Vec::<String>::new());

        let mut node = divide("implement", &["a", "b", "c"]);
        assert_eq!(notes(&node, None), Vec::<String>::new());
        let over_fan_out = refused(
            "implement",
            RunFailure::PlanOverCaps {
                node: "implement".to_owned(),
                cap: PlanCap::MaxFanOut,
                planned: 3,
                max: 2,
            },
        );
        assert_eq!(notes(&node, Some(2)), vec![over_fan_out]);

        // `max_parts` is the node's own, so it is judged either way.
        node.phase.max_parts = Some(2);
        assert_eq!(
            notes(&node, None),
            vec![refused(
                "implement",
                RunFailure::PlanOverCaps {
                    node: "implement".to_owned(),
                    cap: PlanCap::MaxParts,
                    planned: 3,
                    max: 2,
                }
            )]
        );
    }

    /// With no listed candidate the node's candidates come from rungs the editor cannot see, so
    /// a part agent is left to resolve.
    #[test]
    fn a_part_agent_is_judged_only_against_listed_candidates() {
        let (listed, stranger) = (AgentId::new(), AgentId::new());
        let mut node = divide("implement", &["api"]);
        node.parts[0].agent_id = Some(stranger);
        assert_eq!(notes_of(&node), Vec::<String>::new(), "no listed candidate");

        candidate(&mut node, listed);
        assert_eq!(
            notes_of(&node),
            vec![refused(
                "implement",
                RunFailure::PartAgentNotCandidate {
                    node: "implement".to_owned(),
                    part: "api".to_owned(),
                    agent: stranger,
                }
            )]
        );

        candidate(&mut node, stranger);
        assert_eq!(notes_of(&node), Vec::<String>::new(), "now a candidate");
    }

    /// `notes` under a generous `max_fan_out`, so the caps stay quiet.
    fn notes_of(node: &NodeBody) -> Vec<String> {
        notes(node, Some(8))
    }
}
