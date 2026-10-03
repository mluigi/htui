//! §4.8's promotion, the pure half (plan D163, blueprint D192, D193): which opening a promoted
//! step gets, on either transport (MOD-37 M5), and the handoff role's `PromptSpec`.
//!
//! The engine reads the step's rows, its agent's caps and its trees; this module decides from
//! them and builds nothing it would have to read for itself.

use htui_agent::driver::{AgentSessionRef, DriverCaps};
use htui_agent::event::{DriverEvent, SESSION_STARTED};
use htui_agent::replay::envelope_from_row;
use htui_core::model::{EventKind, PromptTemplate, SessionEvent};
use htui_core::prompt::excerpt::RepoRoot;
use htui_core::prompt::{
    DiffBlock, HandoffInputs, PromptSpec, StepSummary, TemplateRef, TemplateRole,
};

/// The follow-up `htui` sends when it resumes a step's own agent session.
pub const RESUME_OPENING: &str = "This step was promoted to an interactive chat. Say where you \
    stopped and what is left, then wait for the maintainer's instructions.";

/// MOD-37 M5 (ANA-27 T5): what a chat that opened with the handoff prompt did not carry. The Runs
/// pane, the `resume_failed` row and the Chat tab all say it in these words.
pub const CONTEXT_NOT_CARRIED: &str = "context not carried; handoff prompt only";

/// How a promoted step's chat opens (blueprint D192).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningKind {
    /// The step's own agent session, resumed from its banner's id on either transport; the first
    /// message is [`RESUME_OPENING`].
    Resume(AgentSessionRef),
    /// A fresh session opened with the `handoff` role's prompt ([`handoff_spec`]).
    Handoff,
}

/// The step's latest `other` row whose update is `session_started`, as ANA-4 §6 records it
/// (`body.session_id`, `agent_worker.rs:2832-2839`), decoded through `replay::envelope_from_row`
/// (MOD-37 M5 A-7, amending D192: after a resume that failed and fell back, the latest session is
/// the handoff's, which holds the chat).
///
/// "Latest" is by `seq`. A row that does not decode, or a banner without a string `session_id`,
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
        .max_by_key(|(seq, _)| *seq)
        .and_then(|(_, body)| {
            body.get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(|id| AgentSessionRef(id.to_owned()))
        })
}

/// Blueprint D192: resume whenever the agent's caps say `resume` and [`banner`] finds the
/// session's id: the CLI through `--resume`, ACP through `session/resume` or `session/load`
/// (MOD-37 M5, R-48). The handoff text is built either way, as the fallback a failed resume
/// opens with.
///
/// So: [`OpeningKind::Resume`] when `caps.resume` and [`banner`] finds the session's id, and
/// [`OpeningKind::Handoff`] otherwise.
#[must_use]
pub fn opening_kind(caps: DriverCaps, events: &[SessionEvent]) -> OpeningKind {
    if !caps.resume {
        return OpeningKind::Handoff;
    }
    banner(events).map_or(OpeningKind::Handoff, OpeningKind::Resume)
}

/// §4.6(c)'s handoff spec built from the phase's own spec: role `Handoff`, the pinned `handoff`
/// template's body and ref, `handoff: Some(HandoffInputs { StepSummary::from_events(events,
/// roots), diff_so_far, failure_reason })`, `verify_failure`/`previous_diff`/`judge` cleared, and
/// `skills` emptied (MOD-9 D44: a handoff body cannot place `{{skills}}`, so a candidate would only
/// be scrubbed — failing the handoff on a skill body it never shows — and recorded `not_placed`).
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
        skills: Vec::new(),
        // MOD-26 OQ-5: a handoff runs under no persona, whatever the abandoned phase named.
        persona: None,
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
    use htui_core::prompt::{PersonaBlock, assemble, body_of, parse};
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

    /// `registry::caps_for`'s ACP profile spelled out, with `acp.session.resume` on.
    fn acp_caps() -> DriverCaps {
        DriverCaps {
            permission_requests: true,
            edit_proposals: true,
            plans: true,
            thoughts: true,
            follow_up_in_session: true,
            resume: true,
            usage: true,
            usage_mid_turn: true,
            authenticate: true,
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
            Some(AgentSessionRef("sess_2".to_owned())),
            "the latest `session_started` row, never another `other` row's `session_id`"
        );
        assert_eq!(
            opening_kind(cli_caps(), &banner_events()),
            OpeningKind::Resume(AgentSessionRef("sess_2".to_owned()))
        );
        // "Latest" is by `seq`, whatever order the rows were handed over in.
        let mut reversed = banner_events();
        reversed.reverse();
        assert_eq!(
            opening_kind(cli_caps(), &reversed),
            OpeningKind::Resume(AgentSessionRef("sess_2".to_owned()))
        );
    }

    /// MOD-37 M5 A-7 (amending D192): after a resume that failed and fell back to the handoff,
    /// the step's log holds two banners, and the latest one names the session that holds the
    /// chat. A re-promotion resumes that one instead of retrying the dead id.
    #[test]
    fn the_latest_banner_wins() {
        let events = vec![
            other(5, SESSION_STARTED, json!({ "session_id": "sess_new" })),
            event(2, 0, EventKind::AssistantText, json!({ "text": "on it" })),
            other(1, SESSION_STARTED, json!({ "session_id": "sess_old" })),
        ];
        assert_eq!(
            banner(&events),
            Some(AgentSessionRef("sess_new".to_owned())),
            "the banner with the highest `seq`, not the first one handed over"
        );
        assert_eq!(
            opening_kind(cli_caps(), &events),
            OpeningKind::Resume(AgentSessionRef("sess_new".to_owned()))
        );
    }

    #[test]
    fn no_banner_means_handoff() {
        let events: Vec<SessionEvent> = banner_events()
            .into_iter()
            .filter(|row| row.payload.get("update") != Some(&json!(SESSION_STARTED)))
            .collect();
        assert_eq!(banner(&events), None);
        assert_eq!(opening_kind(cli_caps(), &events), OpeningKind::Handoff);
        assert_eq!(
            opening_kind(cli_caps(), &[]),
            OpeningKind::Handoff,
            "a step that never started a session"
        );
        // A banner that names no session is no banner.
        let nameless = [other(0, SESSION_STARTED, json!({ "session_id": null }))];
        assert_eq!(banner(&nameless), None);
        assert_eq!(opening_kind(cli_caps(), &nameless), OpeningKind::Handoff);
    }

    #[test]
    fn a_non_resumable_agent_means_handoff() {
        assert_eq!(
            opening_kind(DriverCaps::default(), &banner_events()),
            OpeningKind::Handoff
        );
    }

    /// MOD-37 M5 (R-48): the ACP driver restores a session through `session/resume` or
    /// `session/load`, so an ACP step whose caps say `resume` resumes like a CLI one.
    #[test]
    fn an_acp_agent_with_resume_caps_and_a_banner_resumes() {
        assert_eq!(
            opening_kind(acp_caps(), &banner_events()),
            OpeningKind::Resume(AgentSessionRef("sess_2".to_owned()))
        );
    }

    /// An ACP row with `acp.session` turned off hands off, banner or not.
    #[test]
    fn an_acp_agent_without_resume_caps_hands_off() {
        let caps = DriverCaps {
            resume: false,
            ..acp_caps()
        };
        assert_eq!(opening_kind(caps, &banner_events()), OpeningKind::Handoff);
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
            // Blueprint F-C, D63: the fixture phase carries one skill, and a handoff carries none.
            skills: Vec::new(),
            persona: None,
            ..phase.clone()
        };
        assert_eq!(
            spec, expected,
            "every other field is the phase's; D44: a handoff carries no skills"
        );
        assert_eq!(spec.item_key, phase.item_key);
        assert_eq!(spec.budget, phase.budget);
        assert_eq!(spec.attempt, phase.attempt);
    }

    /// MOD-26 OQ-5: the handoff spec never carries the abandoned phase's persona frame.
    #[test]
    fn the_handoff_spec_drops_the_persona() {
        let phase = PromptSpec {
            persona: Some(PersonaBlock {
                name: "reviewer".to_owned(),
                body: "You are the reviewer for this step.\n".to_owned(),
            }),
            ..phase_implement_attempt2()
        };
        let spec = handoff_spec(
            phase,
            &handoff_template(),
            &handoff_events(),
            &[root()],
            Some(diff()),
            "gave up".to_owned(),
        );
        assert_eq!(spec.persona, None);
    }

    /// MOD-9 D44 (blueprint F-D, D63): the handoff body cannot place `{{skills}}`, so a phase's
    /// candidates would only be scrubbed and recorded `not_placed`. The spec drops them, and the
    /// assembled record has no choice to show. Tested here because the engine's only handoff path
    /// (`open_chat`) keeps the text and digest and stores no record.
    #[test]
    fn the_handoff_carries_no_skills() {
        let phase = phase_implement_attempt2();
        assert!(
            !phase.skills.is_empty(),
            "the fixture phase carries a skill, which is what makes this test able to fail"
        );
        let spec = handoff_spec(
            phase,
            &handoff_template(),
            &handoff_events(),
            &[root()],
            None,
            "x".to_owned(),
        );
        assert!(spec.skills.is_empty(), "{:?}", spec.skills);
        let assembled = assemble(&spec, &MinimalScrubber::new([])).expect("the handoff assembles");
        assert_eq!(assembled.trim.skill_choices, Vec::new());
    }
}
