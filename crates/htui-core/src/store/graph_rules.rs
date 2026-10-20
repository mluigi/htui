//! MOD-137's pure graph rules (blueprint §T3.2): the store checks both `MemStore` and `PgStore`
//! decide before any write, so they refuse the same input with the same sentence (F6). The seed of
//! MOD-138's shared graph validator.

use crate::model::{Split, StepGraphPhase};
use crate::store::traits::{
    divide_columns_off_divide, max_parts_below_one, split_fan_out_mismatch,
};

/// D7's node CHECKs on one row: `ck_phase_split_fan_out`, then `ck_phase_divide_columns`, then
/// `ck_phase_max_parts`. `None` when the row passes. `create_phase` and `replace_graph` call it.
#[must_use]
pub fn phase_row_refusal(phase: &StepGraphPhase) -> Option<String> {
    if (phase.split == Split::Compete) != (phase.fan_out > 1) {
        return Some(split_fan_out_mismatch(
            &phase.name,
            phase.split,
            phase.fan_out,
        ));
    }
    let divide_columns = phase.planner_persona_id.is_some()
        || phase.planner_agent_id.is_some()
        || phase.planner_model.is_some()
        || phase.integrate_persona_id.is_some()
        || phase.integrate_agent_id.is_some()
        || phase.integrate_model.is_some()
        || phase.max_parts.is_some();
    if phase.split != Split::Divide && divide_columns {
        return Some(divide_columns_off_divide(&phase.name, phase.split));
    }
    match phase.max_parts {
        Some(max_parts) if max_parts < 1 => Some(max_parts_below_one(&phase.name, max_parts)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::phase_row_refusal;
    use crate::model::{
        AgentId, CommandQueue, Gate, PersonaId, PhaseId, Split, StepGraphId, StepGraphPhase,
    };

    fn phase(split: Split, fan_out: i32) -> StepGraphPhase {
        StepGraphPhase {
            id: PhaseId::new(),
            graph_id: StepGraphId::new(),
            position: 0,
            name: "implement".to_owned(),
            fan_out,
            gate: Gate::Never,
            gate_hard: false,
            retry_limit: 0,
            input_kinds: Vec::new(),
            output_kind: "diff".to_owned(),
            isolation: None,
            command_queue: CommandQueue::Off,
            verify_command: None,
            template_name: "implement".to_owned(),
            template_version: None,
            token_budget: None,
            persona_id: None,
            split,
            palette: false,
            plan_scope: None,
            planner_persona_id: None,
            planner_agent_id: None,
            planner_model: None,
            integrate_persona_id: None,
            integrate_agent_id: None,
            integrate_model: None,
            max_parts: None,
            updated_at: Utc::now(),
        }
    }

    /// `ck_phase_split_fan_out`: `compete` is exactly `fan_out > 1`, both ways.
    #[test]
    fn split_must_follow_fan_out() {
        assert_eq!(phase_row_refusal(&phase(Split::None, 1)), None);
        assert_eq!(phase_row_refusal(&phase(Split::Compete, 2)), None);
        assert_eq!(phase_row_refusal(&phase(Split::Divide, 1)), None);
        assert_eq!(
            phase_row_refusal(&phase(Split::None, 2)).as_deref(),
            Some(
                "phase `implement` is split `none` with fan_out 2; `compete` is exactly \
                 fan_out > 1"
            )
        );
        assert_eq!(
            phase_row_refusal(&phase(Split::Compete, 1)).as_deref(),
            Some(
                "phase `implement` is split `compete` with fan_out 1; `compete` is exactly \
                 fan_out > 1"
            )
        );
        assert!(phase_row_refusal(&phase(Split::Divide, 3)).is_some());
    }

    /// `ck_phase_divide_columns`: each of the seven columns alone refuses off a `divide` node.
    #[test]
    fn divide_columns_belong_to_a_divide_phase() {
        let setters: [fn(&mut StepGraphPhase); 7] = [
            |p| p.planner_persona_id = Some(PersonaId::new()),
            |p| p.planner_agent_id = Some(AgentId::new()),
            |p| p.planner_model = Some("m".to_owned()),
            |p| p.integrate_persona_id = Some(PersonaId::new()),
            |p| p.integrate_agent_id = Some(AgentId::new()),
            |p| p.integrate_model = Some("m".to_owned()),
            |p| p.max_parts = Some(2),
        ];
        for set in setters {
            let mut row = phase(Split::None, 1);
            set(&mut row);
            assert_eq!(
                phase_row_refusal(&row).as_deref(),
                Some(
                    "phase `implement` is split `none`; planner_*, integrate_* and max_parts \
                     belong to a `divide` phase"
                )
            );
            let mut row = phase(Split::Divide, 1);
            set(&mut row);
            assert_eq!(phase_row_refusal(&row), None, "a divide phase takes it");
        }
    }

    /// `ck_phase_max_parts`: a divide phase takes at least one part.
    #[test]
    fn max_parts_is_at_least_one() {
        let mut row = phase(Split::Divide, 1);
        row.max_parts = Some(0);
        assert_eq!(
            phase_row_refusal(&row).as_deref(),
            Some("phase `implement` has max_parts 0; a divide phase takes at least 1")
        );
        row.max_parts = Some(1);
        assert_eq!(phase_row_refusal(&row), None);
    }

    /// The order: the split rule first, then the divide columns, then `max_parts`.
    #[test]
    fn the_node_rules_fire_in_order() {
        let mut row = phase(Split::None, 2);
        row.max_parts = Some(0);
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("fan_out 2")
        );
        row.fan_out = 1;
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("belong to a `divide` phase")
        );
        row.split = Split::Divide;
        assert!(
            phase_row_refusal(&row)
                .expect("refused")
                .contains("max_parts 0")
        );
    }
}
