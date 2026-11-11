//! MOD-139 M2 (ANA-33 §7.4, §7.8): divided fan-out's pure rules — names, positions, the
//! admission refusal, part widths and live siblings. No store, no clock.
//!
//! A divided node `N` is frozen as two nodes (`graph::expand_divide`): the planner `N:plan` and
//! the integrate node `N`. Its parts are rows past the frozen node list, under the planner row
//! (`parent_step_id`), one per default part of an admitted plan revision. Everything here reads
//! the snapshot and the rows the caller already holds.

use htui_core::model::{
    GraphSnapshot, Isolation, NodeRole, PLAN_SUFFIX, RunPlan, RunStep, SkillGlob, SnapshotPart,
    SnapshotPhase, StepId, StepRole, StepStatus,
};
use serde_json::{Value, json};

use crate::status::{PlanCap, RunFailure, latest_at, may_attempt};

/// The divided node's own name: a planner node's `name` less `PLAN_SUFFIX`, any other node's name.
#[must_use]
pub fn divided_name(phase: &SnapshotPhase) -> &str {
    match phase.role {
        NodeRole::Planner => phase.name.strip_suffix(PLAN_SUFFIX).unwrap_or(&phase.name),
        NodeRole::Step | NodeRole::Integrate => &phase.name,
    }
}

/// The row role a node's `Cursor::Create` writes: Step→Step, Planner→Planner, Integrate→Integrate.
#[must_use]
pub const fn row_role(node: NodeRole) -> StepRole {
    match node {
        NodeRole::Step => StepRole::Step,
        NodeRole::Planner => StepRole::Planner,
        NodeRole::Integrate => StepRole::Integrate,
    }
}

/// `<divided>/<key>` at revision 0, `<divided>/<key>@<revision>` from revision 1 (D4): the part
/// row's `phase_name`, so `SessionKey::of` separates parts across revisions.
#[must_use]
pub fn part_phase_name(divided: &str, key: &str, revision: i32) -> String {
    if revision == 0 {
        part_node_key(divided, key)
    } else {
        format!("{divided}/{key}@{revision}")
    }
}

/// `<divided>/<key>`, every revision (ANA-33 §7.4; blueprint P-18): the part row's `node_key`.
#[must_use]
pub fn part_node_key(divided: &str, key: &str) -> String {
    format!("{divided}/{key}")
}

/// The part key of a `node_key`: the text after its last `/`.
#[must_use]
pub fn part_key(node_key: &str) -> Option<&str> {
    node_key.rsplit_once('/').map(|(_, key)| key)
}

/// D2 step 5: 1 + max(the snapshot's highest position, the highest non-stage row position, every
/// plan's `base_position + parts - 1`); `parts` is `body["parts"]`'s length.
#[must_use]
pub fn next_base_position(snapshot: &GraphSnapshot, steps: &[RunStep], plans: &[RunPlan]) -> i32 {
    let nodes = snapshot.phases.iter().map(|phase| phase.position);
    let rows = steps
        .iter()
        .filter(|step| step.role != StepRole::Stage)
        .map(|step| step.position);
    let planned = plans.iter().map(|plan| {
        let parts = plan
            .body
            .get("parts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        plan.base_position
            .saturating_add(i32::try_from(parts).unwrap_or(i32::MAX))
            .saturating_sub(1)
    });
    nodes
        .chain(rows)
        .chain(planned)
        .max()
        .unwrap_or(-1)
        .saturating_add(1)
}

/// `{"scope": "parts", "parts": [<each SnapshotPart, serde>]}`: an admitted revision's body.
#[must_use]
pub fn plan_body(parts: &[SnapshotPart]) -> Value {
    json!({ "scope": "parts", "parts": parts })
}

/// `"sha256:" + sha256_hex` over `body` serialised with its object keys in byte order at every
/// depth, so the digest does not depend on `serde_json`'s `preserve_order` (which a workspace
/// build turns on and a crate build does not, `graph::topology`'s trap) or on the key order a
/// store hands the body back in.
#[must_use]
pub fn plan_digest(body: &Value) -> String {
    let mut text = String::new();
    canonical(body, &mut text);
    format!("sha256:{}", htui_core::prompt::digest::sha256_hex(&text))
}

/// `value` as JSON with every object's keys in byte order.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push('{');
            for (i, (key, value)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(value, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// D8 (ANA-33 §7.8): planned sessions. Step: `fan_out` + 1 for a judged `fan_out > 1`;
/// Planner: 1 if `divide.planner.agent_id` is set, else 0, plus its parts — `admitting.1` when
/// `admitting == Some((planner position, n))`, else `divide.default_parts.len()`;
/// Integrate: 1. Retries and rounds never count.
#[must_use]
pub fn planned_sessions(phases: &[SnapshotPhase], admitting: Option<(i32, u32)>) -> u32 {
    phases
        .iter()
        .map(|phase| match phase.role {
            NodeRole::Step => {
                // A non-positive `fan_out` plans nothing; the store's CHECK keeps it at one or more.
                let own = u32::try_from(phase.fan_out).unwrap_or(0);
                let judge = u32::from(phase.fan_out > 1 && phase.judge.is_some());
                own.saturating_add(judge)
            }
            NodeRole::Planner => {
                let divide = phase.divide.as_ref();
                let planner =
                    u32::from(divide.is_some_and(|divide| divide.planner.agent_id.is_some()));
                let parts = match admitting {
                    Some((position, parts)) if position == phase.position => parts,
                    _ => divide.map_or(0, |divide| {
                        u32::try_from(divide.default_parts.len()).unwrap_or(u32::MAX)
                    }),
                };
                planner.saturating_add(parts)
            }
            NodeRole::Integrate => 1,
        })
        .fold(0, u32::saturating_add)
}

/// D2 step 2: the first refusal of admitting `parts` on planner node `node`, or `None`.
///
/// In order: no part (`EmptyPlan`); more than one part under `local` (`LocalCannotDivide`); the
/// node's `max_parts`, then `max_fan_out`, then `max_agents_per_run` over [`planned_sessions`]
/// (`PlanOverCaps`); then per part in `position` order a persona (`PartPersonaUnsupported`), each
/// owned path that is not a repo-relative glob (`PartPathRefused`), and an agent that is not one of
/// the node's candidates (`PartAgentNotCandidate`).
#[must_use]
pub fn admission_refusal(
    snapshot: &GraphSnapshot,
    node: &SnapshotPhase,
    parts: &[SnapshotPart],
) -> Option<RunFailure> {
    let name = divided_name(node).to_owned();
    let count = u32::try_from(parts.len()).unwrap_or(u32::MAX);
    if parts.is_empty() {
        return Some(RunFailure::EmptyPlan { node: name });
    }
    if node.isolation == Isolation::Local && parts.len() > 1 {
        return Some(RunFailure::LocalCannotDivide {
            node: name,
            parts: count,
        });
    }
    let over = |cap, planned, max| RunFailure::PlanOverCaps {
        node: name.clone(),
        cap,
        planned,
        max,
    };
    if let Some(max) = node.divide.as_ref().and_then(|divide| divide.max_parts) {
        let max = u32::try_from(max).unwrap_or(0);
        if count > max {
            return Some(over(PlanCap::MaxParts, count, max));
        }
    }
    let settings = &snapshot.settings;
    if count > settings.max_fan_out {
        return Some(over(PlanCap::MaxFanOut, count, settings.max_fan_out));
    }
    let planned = planned_sessions(&snapshot.phases, Some((node.position, count)));
    if planned > settings.max_agents_per_run {
        return Some(over(
            PlanCap::MaxAgentsPerRun,
            planned,
            settings.max_agents_per_run,
        ));
    }
    let mut ordered: Vec<&SnapshotPart> = parts.iter().collect();
    ordered.sort_by_key(|part| part.position);
    for part in ordered {
        if part.persona_id.is_some() {
            return Some(RunFailure::PartPersonaUnsupported {
                node: name,
                part: part.key.clone(),
            });
        }
        for path in &part.owned_paths {
            if let Some(reason) = owned_path_refusal(path.trim()) {
                return Some(RunFailure::PartPathRefused {
                    node: name,
                    part: part.key.clone(),
                    path: path.trim().to_owned(),
                    reason,
                });
            }
        }
        if let Some(agent) = part.agent_id
            && !node
                .candidates
                .iter()
                .any(|candidate| candidate.agent_id == agent)
        {
            return Some(RunFailure::PartAgentNotCandidate {
                node: name,
                part: part.key.clone(),
                agent,
            });
        }
    }
    None
}

/// D10 (blueprint P-16): why `path` (trimmed) is not a repo-relative `repo:glob`, or `None`.
fn owned_path_refusal(path: &str) -> Option<String> {
    let glob = match SkillGlob::parse(path) {
        Ok(glob) => glob,
        Err(err) => return Some(format!("is not a glob: {err}")),
    };
    if glob.glob.starts_with('/') || glob.glob.starts_with('\\') {
        return Some("is an absolute path".to_owned());
    }
    if glob.glob.split('/').any(|segment| segment == "..") {
        return Some("leaves its repository through `..`".to_owned());
    }
    None
}

/// The latest attempt of each part position under `planner` (rows whose `parent_step_id` is it),
/// in position order.
#[must_use]
pub fn latest_parts(steps: &[RunStep], planner: StepId) -> Vec<&RunStep> {
    let mut latest: Vec<&RunStep> = Vec::new();
    for step in steps
        .iter()
        .filter(|step| step.role == StepRole::Part && step.parent_step_id == Some(planner))
    {
        match latest
            .iter_mut()
            .find(|held| held.position == step.position)
        {
            Some(held) if held.attempt < step.attempt => *held = step,
            Some(_) => {}
            None => latest.push(step),
        }
    }
    latest.sort_by_key(|step| step.position);
    latest
}

/// Whether `status` is a part's settled failure: `failed`, `cancelled` or `superseded`.
const fn failed_part(status: StepStatus) -> bool {
    matches!(
        status,
        StepStatus::Failed | StepStatus::Cancelled | StepStatus::Superseded
    )
}

/// Whether `part` (a latest attempt) is out of attempts: `failed | cancelled | superseded` and
/// `!may_attempt(attempt + 1, retry_limit)`.
#[must_use]
pub const fn exhausted(part: &RunStep, retry_limit: i32) -> bool {
    failed_part(part.status) && !may_attempt(part.attempt + 1, retry_limit)
}

/// D4: the parts a round may run at once: 1 under `shared_serialized` or `local`, else
/// `min(live, max_parallel_sessions.max(1)).max(1)`, `live` = the latest-attempt parts under the
/// node's latest planner row that are `pending`, or `failed | cancelled | superseded` with an
/// attempt left.
#[must_use]
pub fn parts_width(snapshot: &GraphSnapshot, steps: &[RunStep], node: &SnapshotPhase) -> u32 {
    if matches!(
        node.isolation,
        Isolation::SharedSerialized | Isolation::Local
    ) {
        return 1;
    }
    let live = latest_at(steps, node.position)
        .filter(|planner| planner.role == StepRole::Planner)
        .map_or(0, |planner| {
            latest_parts(steps, planner.id)
                .into_iter()
                .filter(|part| {
                    part.status == StepStatus::Pending
                        || (failed_part(part.status) && !exhausted(part, node.retry_limit))
                })
                .count()
        });
    let live = u32::try_from(live).unwrap_or(u32::MAX);
    live.min(snapshot.settings.max_parallel_sessions.max(1))
        .max(1)
}

/// D13: another row of the run than `step`, not a stage row (`role == Stage`) and not a judge
/// (`fanout_index == -1`), at `pending` or `running`.
#[must_use]
pub fn live_siblings(steps: &[RunStep], step: StepId) -> bool {
    steps.iter().any(|row| {
        row.id != step
            && row.role != StepRole::Stage
            && row.fanout_index != -1
            && matches!(row.status, StepStatus::Pending | StepStatus::Running)
    })
}

#[cfg(test)]
mod tests {
    use htui_core::fixtures::ids;
    use htui_core::model::{
        GraphSnapshot, Isolation, NodeRole, PersonaId, PlanScope, PlanStatus, RunPlan, RunPlanId,
        RunStep, SnapshotCandidate, SnapshotDivide, SnapshotPart, SnapshotPhase, StepId, StepRole,
        StepStatus,
    };
    use serde_json::json;

    use super::{
        admission_refusal, divided_name, exhausted, latest_parts, live_siblings,
        next_base_position, part_key, part_node_key, part_phase_name, parts_width, plan_body,
        plan_digest, planned_sessions, row_role,
    };
    use crate::status::{PlanCap, RunFailure};
    use crate::walk::tests::{fixture, node, row};

    fn part(key: &str, position: i32) -> SnapshotPart {
        SnapshotPart {
            position,
            key: key.to_owned(),
            title: format!("The {key}"),
            brief: String::new(),
            owned_paths: vec![format!("src/{key}/**")],
            acceptance: None,
            agent_id: None,
            model: None,
            persona_id: None,
        }
    }

    /// FEAT-3 divided at `implement`: `prd 0, plan 1, implement:plan 2, implement 3, review 4`,
    /// the planner holding `parts`.
    fn divided(parts: Vec<SnapshotPart>) -> GraphSnapshot {
        let (mut snapshot, _) = fixture();
        let template = snapshot.phases[0].clone();
        let divide = SnapshotDivide {
            default_parts: parts,
            ..SnapshotDivide::default()
        };
        let mut planner = node(&template, "implement:plan", 2);
        planner.role = NodeRole::Planner;
        planner.isolation = Isolation::Worktree;
        planner.divide = Some(divide.clone());
        planner.candidates = vec![SnapshotCandidate {
            agent_id: ids::AGENT_CLAUDE,
            agent_name: "claude".to_owned(),
            model: "sonnet".to_owned(),
        }];
        let mut integrate = node(&template, "implement", 3);
        integrate.role = NodeRole::Integrate;
        integrate.divide = Some(divide);
        snapshot.phases = vec![
            node(&template, "prd", 0),
            node(&template, "plan", 1),
            planner,
            integrate,
            node(&template, "review", 4),
        ];
        snapshot.settings.max_fan_out = 8;
        snapshot.settings.max_agents_per_run = 16;
        snapshot.settings.max_parallel_sessions = 4;
        snapshot
    }

    fn planner_of(snapshot: &GraphSnapshot) -> SnapshotPhase {
        snapshot.phases[2].clone()
    }

    /// A row of the run at `position`, `role`, under `parent`.
    fn step_at(
        position: i32,
        role: StepRole,
        parent: Option<StepId>,
        status: StepStatus,
    ) -> RunStep {
        let (_, mut step) = fixture();
        step.id = StepId::new();
        step.position = position;
        step.attempt = 1;
        step.fanout_index = 0;
        step.role = role;
        step.parent_step_id = parent;
        step.status = status;
        step
    }

    fn plan_at(base_position: i32, parts: &[SnapshotPart]) -> RunPlan {
        let (_, step) = fixture();
        let body = plan_body(parts);
        RunPlan {
            id: RunPlanId::new(),
            run_id: step.run_id,
            revision: 0,
            planner_step_id: step.id,
            scope: PlanScope::Parts,
            status: PlanStatus::Admitted,
            base_position,
            base_refs: Some(json!({})),
            digest: plan_digest(&body),
            body,
            created_at: step.updated_at,
            updated_at: step.updated_at,
        }
    }

    /// D4, P-18: `phase_name` carries the revision from 1 on; `node_key` never does.
    #[test]
    fn part_names_carry_the_revision_from_one_on() {
        assert_eq!(part_phase_name("implement", "api", 0), "implement/api");
        assert_eq!(part_phase_name("implement", "api", 1), "implement/api@1");
        assert_eq!(part_node_key("implement", "api"), "implement/api");
        assert_eq!(part_key("implement/api"), Some("api"));
        assert_eq!(part_key("api"), None);
        let snapshot = divided(vec![part("api", 0)]);
        assert_eq!(divided_name(&snapshot.phases[2]), "implement");
        assert_eq!(divided_name(&snapshot.phases[3]), "implement");
        assert_eq!(divided_name(&snapshot.phases[0]), "prd");
        assert_eq!(row_role(NodeRole::Planner), StepRole::Planner);
        assert_eq!(row_role(NodeRole::Integrate), StepRole::Integrate);
        assert_eq!(row_role(NodeRole::Step), StepRole::Step);
    }

    /// D2 step 5: one past the highest node, row and earlier plan position; stages ignored.
    #[test]
    fn the_base_position_follows_nodes_rows_and_plans() {
        let snapshot = divided(vec![part("api", 0), part("ui", 1)]);
        assert_eq!(next_base_position(&snapshot, &[], &[]), 5);
        let stage = step_at(-1, StepRole::Stage, None, StepStatus::Pending);
        assert_eq!(
            next_base_position(&snapshot, std::slice::from_ref(&stage), &[]),
            5
        );
        let planner = step_at(2, StepRole::Planner, None, StepStatus::Done);
        let rows = [
            stage,
            step_at(5, StepRole::Part, Some(planner.id), StepStatus::Done),
            step_at(6, StepRole::Part, Some(planner.id), StepStatus::Done),
            planner,
        ];
        assert_eq!(next_base_position(&snapshot, &rows, &[]), 7);
        let plan = plan_at(5, &[part("api", 0), part("ui", 1)]);
        assert_eq!(
            next_base_position(&snapshot, &[], std::slice::from_ref(&plan)),
            7
        );
        assert_eq!(next_base_position(&snapshot, &rows, &[plan]), 7);
    }

    /// The body is the parts under a `parts` scope; the digest ignores key order.
    #[test]
    fn the_plan_body_and_digest_are_canonical() {
        let body = plan_body(&[part("api", 0)]);
        assert_eq!(body["scope"], "parts");
        assert_eq!(body["parts"][0]["key"], "api");
        let reordered: serde_json::Value =
            serde_json::from_str(&body.to_string()).expect("the body round-trips");
        assert_eq!(plan_digest(&body), plan_digest(&reordered));
        assert!(plan_digest(&body).starts_with("sha256:"));
        assert_ne!(
            plan_digest(&body),
            plan_digest(&plan_body(&[part("ui", 0)]))
        );
    }

    /// D2 step 2's order: each rule with every earlier one satisfied.
    #[test]
    fn admission_refusals_come_in_order() {
        let snapshot = divided(vec![part("api", 0), part("ui", 1)]);
        let node = planner_of(&snapshot);
        let two = [part("api", 0), part("ui", 1)];
        assert_eq!(admission_refusal(&snapshot, &node, &two), None);
        assert_eq!(
            admission_refusal(&snapshot, &node, &[]),
            Some(RunFailure::EmptyPlan {
                node: "implement".to_owned()
            })
        );

        let mut local = node.clone();
        local.isolation = Isolation::Local;
        assert_eq!(
            admission_refusal(&snapshot, &local, &two),
            Some(RunFailure::LocalCannotDivide {
                node: "implement".to_owned(),
                parts: 2
            })
        );
        assert_eq!(admission_refusal(&snapshot, &local, &two[..1]), None);
        let mut persona = two.clone();
        persona[0].persona_id = Some(PersonaId::new());
        assert!(
            matches!(
                admission_refusal(&snapshot, &local, &persona),
                Some(RunFailure::LocalCannotDivide { .. })
            ),
            "rule 2 before a part persona"
        );

        let mut capped = node.clone();
        if let Some(divide) = capped.divide.as_mut() {
            divide.max_parts = Some(1);
        }
        assert_eq!(
            admission_refusal(&snapshot, &capped, &two),
            Some(RunFailure::PlanOverCaps {
                node: "implement".to_owned(),
                cap: PlanCap::MaxParts,
                planned: 2,
                max: 1
            })
        );
        let mut fan = snapshot.clone();
        fan.settings.max_fan_out = 1;
        assert_eq!(
            admission_refusal(&fan, &node, &two),
            Some(RunFailure::PlanOverCaps {
                node: "implement".to_owned(),
                cap: PlanCap::MaxFanOut,
                planned: 2,
                max: 1
            })
        );
        assert!(
            matches!(
                admission_refusal(&fan, &capped, &two),
                Some(RunFailure::PlanOverCaps {
                    cap: PlanCap::MaxParts,
                    ..
                })
            ),
            "rules 3 and 4 at once answer `max_parts`"
        );
        let mut agents = snapshot.clone();
        // prd 1 + plan 1 + two parts + integrate 1 + review 1 = 6.
        agents.settings.max_agents_per_run = 5;
        assert_eq!(
            admission_refusal(&agents, &node, &two),
            Some(RunFailure::PlanOverCaps {
                node: "implement".to_owned(),
                cap: PlanCap::MaxAgentsPerRun,
                planned: 6,
                max: 5
            })
        );

        assert_eq!(
            admission_refusal(&snapshot, &node, &persona),
            Some(RunFailure::PartPersonaUnsupported {
                node: "implement".to_owned(),
                part: "api".to_owned()
            })
        );
        let mut path = two.clone();
        path[1].owned_paths = vec!["/etc/x".to_owned()];
        assert_eq!(
            admission_refusal(&snapshot, &node, &path),
            Some(RunFailure::PartPathRefused {
                node: "implement".to_owned(),
                part: "ui".to_owned(),
                path: "/etc/x".to_owned(),
                reason: "is an absolute path".to_owned()
            })
        );
        let mut agent = two.clone();
        agent[1].agent_id = Some(ids::AGENT_AGY);
        assert_eq!(
            admission_refusal(&snapshot, &node, &agent),
            Some(RunFailure::PartAgentNotCandidate {
                node: "implement".to_owned(),
                part: "ui".to_owned(),
                agent: ids::AGENT_AGY
            })
        );
        agent[1].agent_id = Some(ids::AGENT_CLAUDE);
        assert_eq!(admission_refusal(&snapshot, &node, &agent), None);
        let mut both = two;
        both[1].agent_id = Some(ids::AGENT_AGY);
        both[1].owned_paths = vec!["src/../x".to_owned()];
        assert!(
            matches!(
                admission_refusal(&snapshot, &node, &both),
                Some(RunFailure::PartPathRefused { .. })
            ),
            "a part's paths before its agent"
        );
    }

    /// D10 (P-16): an owned path is a repo-relative `repo:glob`.
    #[test]
    fn owned_paths_must_be_repo_relative_globs() {
        let snapshot = divided(vec![part("api", 0)]);
        let node = planner_of(&snapshot);
        let with = |path: &str| {
            let mut one = part("api", 0);
            one.owned_paths = vec![path.to_owned()];
            admission_refusal(&snapshot, &node, &[one])
        };
        assert_eq!(with("src/**"), None);
        assert_eq!(with("htui:src/**"), None);
        let reason = |path: &str| match with(path) {
            Some(RunFailure::PartPathRefused { reason, .. }) => reason,
            other => panic!("{path}: {other:?}"),
        };
        assert_eq!(reason("/etc/x"), "is an absolute path");
        assert_eq!(reason("htui:/x"), "is an absolute path");
        assert_eq!(reason("src/../x"), "leaves its repository through `..`");
        assert!(
            reason("[").starts_with("is not a glob: "),
            "{}",
            reason("[")
        );
    }

    /// D8: parts count, a planner without an agent does not, integrate is one; retries never.
    #[test]
    fn planned_sessions_count_parts_not_retries() {
        let snapshot = divided(vec![part("api", 0), part("ui", 1), part("db", 2)]);
        // prd 1 + plan 1 + three parts + integrate 1 + review 1.
        assert_eq!(planned_sessions(&snapshot.phases, None), 7);
        assert_eq!(planned_sessions(&snapshot.phases, Some((2, 1))), 5);
        assert_eq!(
            planned_sessions(&snapshot.phases, Some((0, 1))),
            7,
            "admitting names the planner's position"
        );
        let mut agent = snapshot.clone();
        if let Some(divide) = agent.phases[2].divide.as_mut() {
            divide.planner.agent_id = Some(ids::AGENT_CLAUDE);
        }
        assert_eq!(planned_sessions(&agent.phases, None), 8);
    }

    /// D13: a pending or running row other than this one, not a stage or a judge.
    #[test]
    fn live_siblings_skip_stages_judges_and_this_step() {
        let this = step_at(5, StepRole::Part, None, StepStatus::Running);
        assert!(!live_siblings(std::slice::from_ref(&this), this.id));
        let stage = step_at(-1, StepRole::Stage, None, StepStatus::Pending);
        let mut judge = step_at(1, StepRole::Step, None, StepStatus::Running);
        judge.fanout_index = -1;
        let done = step_at(0, StepRole::Step, None, StepStatus::Done);
        assert!(!live_siblings(&[this.clone(), stage, judge, done], this.id));
        let pending = step_at(6, StepRole::Part, None, StepStatus::Pending);
        assert!(live_siblings(&[this.clone(), pending], this.id));
        let running = step_at(3, StepRole::Step, None, StepStatus::Running);
        assert!(live_siblings(&[this.clone(), running], this.id));
    }

    /// D4: in place one at a time; else the live parts under the session cap.
    #[test]
    fn parts_width_is_one_in_place_else_capped() {
        let mut snapshot = divided(vec![part("api", 0), part("ui", 1), part("db", 2)]);
        snapshot.settings.max_parallel_sessions = 2;
        let node = planner_of(&snapshot);
        let planner = row(&snapshot, "implement:plan", 1, StepStatus::Done);
        let mut planner = planner;
        planner.role = StepRole::Planner;
        let parts: Vec<RunStep> = (5..8)
            .map(|position| {
                step_at(
                    position,
                    StepRole::Part,
                    Some(planner.id),
                    StepStatus::Pending,
                )
            })
            .collect();
        let mut steps = parts.clone();
        steps.push(planner.clone());
        assert_eq!(parts_width(&snapshot, &steps, &node), 2);
        let mut wide = snapshot.clone();
        wide.settings.max_parallel_sessions = 8;
        assert_eq!(parts_width(&wide, &steps, &node), 3);
        let mut serial = node.clone();
        serial.isolation = Isolation::SharedSerialized;
        assert_eq!(parts_width(&snapshot, &steps, &serial), 1);
        let mut local = node.clone();
        local.isolation = Isolation::Local;
        assert_eq!(parts_width(&snapshot, &steps, &local), 1);
        let mut one = steps.clone();
        one[0].status = StepStatus::Done;
        one[1].status = StepStatus::Running;
        assert_eq!(parts_width(&wide, &one, &node), 1, "one part pending");
        assert_eq!(parts_width(&wide, &[planner], &node), 1, "never zero");
    }

    /// The latest attempt per position, in position order; `exhausted` reads the budget.
    #[test]
    fn latest_parts_and_exhaustion() {
        let planner = step_at(2, StepRole::Planner, None, StepStatus::Done);
        let mut first = step_at(6, StepRole::Part, Some(planner.id), StepStatus::Failed);
        let mut second = first.clone();
        second.id = StepId::new();
        second.attempt = 2;
        second.status = StepStatus::Pending;
        let other = step_at(5, StepRole::Part, Some(planner.id), StepStatus::Done);
        let stranger = step_at(7, StepRole::Part, Some(StepId::new()), StepStatus::Done);
        let steps = [
            second.clone(),
            first.clone(),
            other.clone(),
            stranger,
            planner.clone(),
        ];
        let latest: Vec<(i32, i32)> = latest_parts(&steps, planner.id)
            .into_iter()
            .map(|step| (step.position, step.attempt))
            .collect();
        assert_eq!(latest, [(5, 1), (6, 2)]);
        assert!(!exhausted(&first, 1), "attempt 2 is left");
        assert!(exhausted(&first, 0));
        first.status = StepStatus::Cancelled;
        assert!(exhausted(&first, 0));
        second.status = StepStatus::Failed;
        assert!(exhausted(&second, 1));
        assert!(!exhausted(&other, 0), "a done part is not exhausted");
    }
}
