//! §4.8's promotion, the pure half (plan D163, blueprint D192, D193): which opening a promoted
//! step gets, and the handoff role's `PromptSpec`.
//!
//! The engine reads the step's rows, its agent's caps and its trees; this module decides from
//! them and builds nothing it would have to read for itself.

use htui_agent::driver::{AgentSessionRef, DriverCaps};
use htui_agent::event::{DriverEvent, SESSION_STARTED};
use htui_agent::replay::envelope_from_row;
use htui_core::model::{EventKind, PromptTemplate, SessionEvent, Transport};
use htui_core::prompt::excerpt::RepoRoot;
use htui_core::prompt::{
    DiffBlock, HandoffInputs, PromptSpec, StepSummary, TemplateRef, TemplateRole,
};

/// The follow-up `htui` sends when it resumes a step's own agent session.
pub const RESUME_OPENING: &str = "This step was promoted to an interactive chat. Say where you \
    stopped and what is left, then wait for the maintainer's instructions.";

/// How a promoted step's chat opens (blueprint D192).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningKind {
    /// The step's own agent session, resumed from its banner's id; the first message is
    /// [`RESUME_OPENING`].
    Resume(AgentSessionRef),
    /// A fresh session opened with the `handoff` role's prompt ([`handoff_spec`]).
    Handoff,
}

/// The step's first `other` row whose update is `session_started`, as ANA-4 §6 records it
/// (`body.session_id`, `agent_worker.rs:2832-2839`), decoded through `replay::envelope_from_row`.
///
/// "First" is by `seq`. A row that does not decode, or a banner without a string `session_id`,
/// answers `None`.
#[must_use]
pub fn banner(events: &[SessionEvent]) -> Option<AgentSessionRef> {
    events
        .iter()
        .filter(|row| row.kind == EventKind::Other)
        .filter_map(|row| match envelope_from_row(row) {
            Ok(envelope) => match envelope.event {
                DriverEvent::Other(other) if other.update == SESSION_STARTED => {
                    Some((row.seq, other.body))
                }
                _ => None,
            },
            Err(_) => None,
        })
        .min_by_key(|(seq, _)| *seq)
        .and_then(|(_, body)| {
            body.get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(|id| AgentSessionRef(id.to_owned()))
        })
}

/// Blueprint D192: resume only where the transport honours `SessionSpec.resume` today. That is
/// the CLI (`cli/mod.rs:145`, `:405`); the ACP driver never reads it (R-48).
///
/// So: [`OpeningKind::Resume`] when `caps.resume`, the transport is `cli` and [`banner`] finds
/// the session's id, and [`OpeningKind::Handoff`] otherwise.
#[must_use]
pub fn opening_kind(
    caps: DriverCaps,
    transport: Transport,
    events: &[SessionEvent],
) -> OpeningKind {
    if !caps.resume || transport != Transport::Cli {
        return OpeningKind::Handoff;
    }
    banner(events).map_or(OpeningKind::Handoff, OpeningKind::Resume)
}

/// §4.6(c)'s handoff spec built from the phase's own spec: role `Handoff`, the pinned `handoff`
/// template's body and ref, `handoff: Some(HandoffInputs { StepSummary::from_events(events,
/// roots), diff_so_far, failure_reason })`, and `verify_failure`/`previous_diff`/`judge` cleared.
/// Every other field is the phase's.
#[must_use]
pub fn handoff_spec(
    phase: PromptSpec,
    template: &PromptTemplate,
    events: &[SessionEvent],
    roots: &[RepoRoot],
    diff_so_far: Option<DiffBlock>,
    failure_reason: String,
) -> PromptSpec {
    PromptSpec {
        role: TemplateRole::Handoff,
        template: TemplateRef {
            name: template.name.clone(),
            version: template.version,
        },
        body: template.body.clone(),
        verify_failure: None,
        previous_diff: None,
        judge: None,
        handoff: Some(HandoffInputs {
            step_summary: StepSummary::from_events(events, roots),
            diff_so_far,
            failure_reason,
        }),
        ..phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use htui_core::model::{EventRole, ProjectId, PromptTemplateId, StepId, UserId};
    use htui_core::prompt::excerpt::RootSource;
    use htui_core::prompt::fixtures::{handoff_basic, phase_implement_attempt2};
    use htui_core::prompt::{assemble, body_of, parse};
    use htui_core::scrub::MinimalScrubber;
    use serde_json::{Value, json};
    use uuid::Uuid;

    /// The step id of `htui_core::prompt::fixtures`' abandoned step (`HANDOFF_STEP_ID`).
    const STEP: &str = "01a06490-eea0-7000-8000-0000000000bb";
    /// Its run id (`HANDOFF_RUN_ID`).
    const RUN: &str = "01a06490-eea0-7000-8000-0000000000aa";

    fn fixed_at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid fixed timestamp")
    }

    fn event(seq: i32, turn: i32, kind: EventKind, payload: Value) -> SessionEvent {
        SessionEvent {
            run_step_id: StepId::from_uuid(Uuid::parse_str(STEP).expect("a literal")),
            seq,
            turn,
            kind,
            role: EventRole::Agent,
            tool_call_id: None,
            payload,
            raw: None,
            at: fixed_at(),
        }
    }

    fn other(seq: i32, update: &str, body: Value) -> SessionEvent {
        event(
            seq,
            0,
            EventKind::Other,
            json!({ "update": update, "body": body }),
        )
    }

    fn cli_caps() -> DriverCaps {
        DriverCaps {
            resume: true,
            ..DriverCaps::default()
        }
    }

    /// A CLI step's rows: the `htui`-authored prompt, the banner, a hook update, and a reply.
    fn banner_events() -> Vec<SessionEvent> {
        vec![
            event(0, 0, EventKind::Prompt, json!({ "text": "go" })),
            other(
                1,
                "system/hook_started",
                json!({ "session_id": "not_the_banner" }),
            ),
            other(2, SESSION_STARTED, json!({ "session_id": "sess_1" })),
            other(3, SESSION_STARTED, json!({ "session_id": "sess_2" })),
            event(4, 0, EventKind::AssistantText, json!({ "text": "on it" })),
        ]
    }

    /// The abandoned step's tree, `fixtures::handoff_root` spelled out (it is private there).
    fn root() -> RepoRoot {
        RepoRoot {
            repo: "htui".to_owned(),
            root: std::path::PathBuf::from(format!(
                "/home/htui/.local/share/htui/trees/{RUN}/{STEP}"
            )),
            source: RootSource::RunStepTree,
        }
    }

    /// `fixtures::handoff_events` spelled out row for row (it is private there); the test pins
    /// that the copy summarises to the fixture's own `handoff_basic` summary, so it cannot drift.
    fn handoff_events() -> Vec<SessionEvent> {
        let tree = format!("/home/htui/.local/share/htui/trees/{RUN}/{STEP}");
        let mut events = vec![event(
            0,
            0,
            EventKind::Prompt,
            json!({ "text": "restarting the implement phase" }),
        )];
        for (kind, count, turn) in [("read", 6, 0), ("edit", 9, 1), ("execute", 6, 2)] {
            for _ in 0..count {
                let seq = i32::try_from(events.len()).expect("a short fixture");
                events.push(event(
                    seq,
                    turn,
                    EventKind::ToolCall,
                    json!({ "tool_kind": kind }),
                ));
            }
        }
        for path in [
            "crates/htui-core/src/prompt/mod.rs",
            "crates/htui-core/src/prompt/trim.rs",
        ] {
            let seq = i32::try_from(events.len()).expect("a short fixture");
            events.push(event(
                seq,
                1,
                EventKind::EditProposal,
                json!({ "path": format!("{tree}/{path}") }),
            ));
        }
        let seq = i32::try_from(events.len()).expect("a short fixture");
        events.push(event(
            seq,
            2,
            EventKind::Error,
            json!({ "code": "command_failed", "message": "`cargo test` exited 101" }),
        ));
        let seq = i32::try_from(events.len()).expect("a short fixture");
        events.push(event(
            seq,
            2,
            EventKind::AssistantText,
            json!({ "text": "I cannot get the borrow checker past the trim loop.\n" }),
        ));
        events
    }

    fn handoff_template() -> PromptTemplate {
        PromptTemplate {
            id: PromptTemplateId::from_uuid(Uuid::from_u128(0x70)),
            project_id: ProjectId::from_uuid(Uuid::from_u128(0x71)),
            name: "handoff".to_owned(),
            version: 4,
            body: body_of("handoff")
                .expect("`handoff` is one of the two reserved bodies")
                .to_owned(),
            created_by: UserId::from_uuid(Uuid::from_u128(0x72)),
            created_at: fixed_at(),
            updated_at: fixed_at(),
        }
    }

    fn diff() -> DiffBlock {
        DiffBlock {
            range: "abc1234..HEAD".to_owned(),
            stat: " 2 files changed, 210 insertions(+)".to_owned(),
            diff: "--- a/trim.rs\n+++ b/trim.rs\n+fn run() {}\n".to_owned(),
        }
    }

    #[test]
    fn a_resumable_cli_agent_with_a_banner_resumes() {
        assert_eq!(
            banner(&banner_events()),
            Some(AgentSessionRef("sess_1".to_owned())),
            "the first `session_started` row, not the first `other` row"
        );
        assert_eq!(
            opening_kind(cli_caps(), Transport::Cli, &banner_events()),
            OpeningKind::Resume(AgentSessionRef("sess_1".to_owned()))
        );
        // "First" is by `seq`, whatever order the rows were handed over in.
        let mut reversed = banner_events();
        reversed.reverse();
        assert_eq!(
            opening_kind(cli_caps(), Transport::Cli, &reversed),
            OpeningKind::Resume(AgentSessionRef("sess_1".to_owned()))
        );
    }

    #[test]
    fn no_banner_means_handoff() {
        let events: Vec<SessionEvent> = banner_events()
            .into_iter()
            .filter(|row| row.payload.get("update") != Some(&json!(SESSION_STARTED)))
            .collect();
        assert_eq!(banner(&events), None);
        assert_eq!(
            opening_kind(cli_caps(), Transport::Cli, &events),
            OpeningKind::Handoff
        );
        assert_eq!(
            opening_kind(cli_caps(), Transport::Cli, &[]),
            OpeningKind::Handoff,
            "a step that never started a session"
        );
        // A banner that names no session is no banner.
        let nameless = [other(0, SESSION_STARTED, json!({ "session_id": null }))];
        assert_eq!(banner(&nameless), None);
        assert_eq!(
            opening_kind(cli_caps(), Transport::Cli, &nameless),
            OpeningKind::Handoff
        );
    }

    #[test]
    fn a_non_resumable_agent_means_handoff() {
        assert_eq!(
            opening_kind(DriverCaps::default(), Transport::Cli, &banner_events()),
            OpeningKind::Handoff
        );
    }

    #[test]
    fn an_acp_agent_hands_off_even_when_its_caps_say_resume() {
        // `acp.session.resume` defaults to `true`, but the ACP driver never reads
        // `SessionSpec.resume` (R-48): resuming would open a fresh session with no context.
        assert_eq!(
            opening_kind(cli_caps(), Transport::Acp, &banner_events()),
            OpeningKind::Handoff
        );
    }

    #[test]
    fn the_handoff_spec_carries_the_windowed_tail_and_the_failure() {
        let events = handoff_events();
        let spec = handoff_spec(
            phase_implement_attempt2(),
            &handoff_template(),
            &events,
            &[root()],
            Some(diff()),
            "The step exhausted its turn budget without a passing build.".to_owned(),
        );

        assert_eq!(spec.role, TemplateRole::Handoff);
        assert_eq!(
            spec.template,
            TemplateRef {
                name: "handoff".to_owned(),
                version: 4,
            }
        );
        assert_eq!(spec.body, handoff_template().body);
        let inputs = spec
            .handoff
            .clone()
            .expect("the handoff role carries its inputs");
        assert_eq!(
            inputs,
            HandoffInputs {
                step_summary: StepSummary::from_events(&events, &[root()]),
                diff_so_far: Some(diff()),
                failure_reason: "The step exhausted its turn budget without a passing build."
                    .to_owned(),
            }
        );
        assert_eq!(
            Some(inputs),
            handoff_basic().handoff,
            "the same events summarise to the golden handoff fixture's inputs"
        );
        assert!(spec.verify_failure.is_none(), "the phase's is cleared");
        assert!(spec.previous_diff.is_none(), "the phase's is cleared");
        assert!(spec.judge.is_none());
        parse(spec.role, &spec.body).expect("the handoff body parses in the handoff role");
        assemble(&spec, &MinimalScrubber::new([])).expect("the handoff spec assembles");

        // The tail is windowed, not the whole last reply.
        let long: String = (0..400)
            .map(|n| format!("line {n} of a long reply\n"))
            .collect();
        let mut events = handoff_events();
        let seq = i32::try_from(events.len()).expect("a short fixture");
        events.push(event(
            seq,
            2,
            EventKind::AssistantText,
            json!({ "text": long }),
        ));
        let spec = handoff_spec(
            phase_implement_attempt2(),
            &handoff_template(),
            &events,
            &[root()],
            None,
            "cancelled by the maintainer".to_owned(),
        );
        let inputs = spec.handoff.expect("the handoff role carries its inputs");
        assert_eq!(
            inputs.step_summary,
            StepSummary::from_events(&events, &[root()])
        );
        assert!(inputs.step_summary.last_assistant_tail.len() < long.len());
        assert!(
            inputs
                .step_summary
                .last_assistant_tail
                .contains("line 399 of a long reply"),
            "the tail keeps the end of the reply"
        );
        assert_eq!(inputs.diff_so_far, None);
        assert_eq!(inputs.failure_reason, "cancelled by the maintainer");
    }

    #[test]
    fn the_handoff_spec_keeps_the_phase_s_item_and_budget() {
        let phase = phase_implement_attempt2();
        let spec = handoff_spec(
            phase.clone(),
            &handoff_template(),
            &handoff_events(),
            &[root()],
            Some(diff()),
            "gave up".to_owned(),
        );

        let expected = PromptSpec {
            role: TemplateRole::Handoff,
            template: TemplateRef {
                name: "handoff".to_owned(),
                version: 4,
            },
            body: handoff_template().body,
            verify_failure: None,
            previous_diff: None,
            judge: None,
            handoff: spec.handoff.clone(),
            ..phase.clone()
        };
        assert_eq!(spec, expected, "every other field is the phase's");
        assert_eq!(spec.item_key, phase.item_key);
        assert_eq!(spec.budget, phase.budget);
        assert_eq!(spec.attempt, phase.attempt);
    }
}
