//! The prompt assembler: everything pure (ANA-5 §4.8 `:1458-1477`).
//!
//! ANA-5 §4.8 split the prompt builder along the dependency test ANA-4 §8 applied to the JSON-RPC
//! SDK. The pure half lives here, in `htui-core`, because that is the only crate both MOD-2 and
//! MOD-9 reach without a new edge: MOD-9's template editor lives in `crates/htui`, which depends on
//! `htui-core` and `htui-store` and on neither `htui-agent` nor `htui-orch`, while MOD-4 needs the
//! same one assembler for the judge and handoff prompts. The I/O half — the repository walk and the
//! per-file read behind [`crate::prompt`]'s excerpt section — is deliberately **not** here: this
//! crate contains no `std::fs` at all, and `htui_agent::excerpt` owns that side.
//!
//! One assembler, three roles (ANA-5 §7): a phase step's prompt, the judge's, and the handoff's.
//! `assemble()` is pure by contract (ANA-5 invariant 2 `:1975-1976`) — no store handle, no I/O, no
//! clock, and no map iteration order reaches the rendered bytes — which is what makes a prompt
//! digest reproducible on any box.
//!
//! Milestone 9 builds this module in pieces. T59 landed the two leaves that depend on nothing:
//! [`template`], the `{{name}}` scanner and its closed per-role placeholder sets, and [`estimate`],
//! the `chars-v2` token estimator every budget decision is measured with. T60 landed [`defaults`].
//! T64 lands the input surface — [`PromptSpec`] and everything it owns — plus [`render`], the
//! per-section renderers, and [`digest`], the canonical form and its hash. `assemble()`, the trim
//! and the settings chain are T65's; the excerpt ranker is T66's.

pub mod defaults;
pub mod digest;
pub mod estimate;
pub mod excerpt;
pub mod render;
pub mod settings;
pub mod template;

#[cfg(feature = "test-support")]
pub mod fixtures;

pub use defaults::{COMMAND_QUEUE_TEXT, DEFAULT_TEMPLATES, body_of};
pub use estimate::TokenEstimator;
pub use excerpt::ExcerptSet;
pub use settings::{Budget, BudgetSource};
pub use template::{ParsedTemplate, Placeholder, Span, TemplateError, TemplateRole, parse};

use serde::{Serialize, Serializer};

use crate::model::box_::BoxProfile;
use crate::model::link::UpstreamEntry;
use crate::model::skill::BoundSkill;

/// Everything the assembler needs, and nothing it must not have (ANA-5 §8 `:1917-1942`).
///
/// Note what is **absent**: no store handle, no `RunId`, no `StepId`, no `Vec<SessionEvent>`, no
/// clock. ANA-5 §4.7 rule 8 makes "two fan-out siblings assemble the same bytes" a property of the
/// type rather than of the code, so a run id cannot reach a rendered byte because there is nowhere
/// to put one. The handoff role takes a [`StepSummary`] the caller built from the step's own
/// events, which is the one place transcript-derived text enters, and it enters already summarised
/// (§4.2 `:587-589`).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptSpec {
    /// Which closed placeholder set [`body`](Self::body) was parsed against.
    pub role: TemplateRole,
    /// The pinned `prompt_template` row, recorded in `trim_record.template` (§4.7 rule 9).
    pub template: TemplateRef,
    /// `prompt_template.body` of that version. The frame every section is substituted into.
    pub body: String,
    /// `<project.slug>:<item.key>`.
    pub item_key: String,
    /// `item.title`, raw; the render collapses and truncates it.
    pub item_title: String,
    /// `item_kind.name`.
    pub item_kind: String,
    /// `item.body`, verbatim.
    pub item_body: String,
    /// `ResolvedPhase.name`.
    pub phase: String,
    /// `ResolvedPhase.output_kind`; `None` substitutes the empty string.
    pub output_kind: Option<String>,
    /// `run_step.attempt`, 1-based.
    pub attempt: i32,
    /// The `input_kinds` resolution: one winner per kind, in `input_kinds` order (§4.7 rule 3).
    pub documents: Vec<InputDocument>,
    /// §4.3's walk, already in canonical order (§4.7 rule 2); `assemble()` re-sorts anyway.
    pub upstream: Vec<UpstreamEntry>,
    /// The §4.2 box projection. Carries no path, by type (`BoxProfile` drops `box_tool.path`).
    pub box_profile: BoxProfile,
    /// The `R-SKL-2` resolution, already collapsed and ordered; `assemble()` re-sorts anyway.
    pub skills: Vec<BoundSkill>,
    /// §4.5's read and windowed excerpts, with the audit half the ranker filled.
    pub excerpts: ExcerptSet,
    /// Whether `R-MCP-4`'s `command_run` exposure is on for this phase (§4.2 `:484`).
    pub command_queue: bool,
    /// The previous attempt's verification output; `None` on attempt 1.
    pub verify_failure: Option<VerifyFailure>,
    /// The previous attempt's diff; `None` on attempt 1.
    pub previous_diff: Option<DiffBlock>,
    /// `Some` if and only if `role == Judge`.
    pub judge: Option<JudgeInputs>,
    /// `Some` if and only if `role == Handoff`.
    pub handoff: Option<HandoffInputs>,
    /// The resolved budget and its provenance (§4.4 step 1).
    pub budget: Budget,
    /// `app_setting.max_skill_tokens`, resolved by the caller (§4.2's aggregate cap).
    pub max_skill_tokens: i64,
    /// The estimator every figure in the record is by, and no other (§5.1).
    pub estimator: TokenEstimator,
    /// Caller notes that are not errors — a clamped `hops`, a stand-in the preview substituted.
    /// Copied into `trim_record.notes` first, before the assembler's own.
    pub notes: Vec<String>,
}

/// The pinned template row, recorded so a digest can be traced back to the bytes that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateRef {
    /// `prompt_template.name`.
    pub name: String,
    /// `prompt_template.version`. Never `latest`: invariant 10 pins it into the snapshot.
    pub version: i32,
}

/// One resolved input document: the winner of its kind (§4.7 rule 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDocument {
    /// `document.kind`; also the `documents:<kind>` section-name suffix.
    pub kind: String,
    /// `document.version` of the winning row.
    pub version: i32,
    /// `document.body`, verbatim.
    pub body: String,
}

/// The previous attempt's verification output (§4.2). Command output, not a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyFailure {
    /// The verification command's exit code.
    pub exit_code: i32,
    /// Its captured output.
    pub output: String,
}

/// A diff at three sizes: the range attribute, the stat, the unified text.
///
/// Three fields rather than one because the trim ladder of §4.4 degrades a diff to its stat, and a
/// stat that had to be re-derived from the unified text at trim time would be a parser in the
/// assembler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffBlock {
    /// `<before_hash>..<after_hash>`, the `range` attribute.
    pub range: String,
    /// The `--stat` summary.
    pub stat: String,
    /// The unified diff.
    pub diff: String,
}

/// The judge role's inputs (§4.6a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeInputs {
    /// The judged step's stored `prompt` text, **replayed** rather than re-assembled (`:1256-1263`).
    pub task: String,
    /// One per surviving candidate.
    pub candidates: Vec<JudgeCandidate>,
    /// Whether this is the second judge call, whose candidate order is reversed (§4.7 rule 6).
    pub reverse: bool,
}

/// One fan-out candidate the judge compares (§4.6a, `docs/ANA-2.md:828`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeCandidate {
    /// `run_step.fan_out_index`. The one place §4.7 rule 8 lets an index be rendered.
    pub fanout_index: i32,
    /// `Some(true)` renders `verify="pass"`, `Some(false)` `verify="fail"`, `None` omits it.
    pub verify: Option<bool>,
    /// The verification exit code; `None` omits the attribute.
    pub exit_code: Option<i32>,
    /// The candidate's output document.
    pub document: Option<InputDocument>,
    /// The candidate's diff.
    pub diff: Option<DiffBlock>,
    /// The tail of its verification output.
    pub verification_tail: Option<String>,
}

/// The handoff role's inputs (§4.6c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffInputs {
    /// The abandoned step's own events, already summarised by [`StepSummary::from_events`].
    pub step_summary: StepSummary,
    /// `before_hash..HEAD` of the abandoned step.
    pub diff_so_far: Option<DiffBlock>,
    /// `run.failure` or the gate note: the reason the handoff prompt exists.
    pub failure_reason: String,
}

/// §4.6(c)'s deterministic summary of one step's own events (`:1301-1318`).
///
/// Built by [`StepSummary::from_events`]; the assembler itself never sees an event, which is what
/// keeps `R-PRM-1`'s "never raw transcripts" true even for the one prompt built from a transcript.
/// `R-ID-6` is the other half: no model summarises a step in order to restart it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StepSummary {
    /// How many turns the step ran, the `turns` attribute.
    pub turns: u32,
    /// How many `session_event` rows it produced, the `events` attribute.
    pub events: u32,
    /// `(tool_kind, count)` in first-seen order.
    pub tool_calls: Vec<(String, u32)>,
    /// `edit_proposal` payload paths, first-seen order, deduped.
    pub files_edited: Vec<String>,
    /// One rendered line per `error` row.
    pub errors: Vec<String>,
    /// The last `assistant_text` payload's `text`, head+tail windowed.
    pub last_assistant_tail: String,
}

/// The closed section vocabulary of §4.2 as a type.
///
/// `Box` is a variant here and `Box<T>` is in the Rust prelude: never glob-import this enum.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SectionName {
    /// The frame's literal spans. Always the first `sections[]` entry (P-9).
    Template,
    /// `item.body`.
    Item,
    /// One per resolved input kind; the payload is `document.kind`.
    Documents(String),
    /// §4.3's walk.
    Upstream,
    /// The §4.2 box projection.
    Box,
    /// The `R-SKL-2` resolution.
    Skills,
    /// §4.5's file excerpts.
    Excerpts,
    /// The previous attempt's verification output.
    VerifyFailure,
    /// The previous attempt's diff.
    PreviousDiff,
    /// `R-MCP-4`'s two sentences.
    CommandQueue,
    /// Judge only: the replayed task.
    JudgeTask,
    /// Judge only, one per candidate; the payload is the `fan_out_index`.
    JudgeCandidate(i32),
    /// Handoff only: §4.6(c)'s summary.
    StepSummary,
    /// Handoff only: this step's diff so far.
    DiffSoFar,
    /// Handoff only: why the step was abandoned.
    FailureReason,
}

impl SectionName {
    /// The one spelling: the `name="…"` attribute and the `sections[].name` key (§4.2 `:554-576`).
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Template => "template".to_owned(),
            Self::Item => "item".to_owned(),
            Self::Documents(kind) => format!("documents:{kind}"),
            Self::Upstream => "upstream".to_owned(),
            Self::Box => "box".to_owned(),
            Self::Skills => "skills".to_owned(),
            Self::Excerpts => "excerpts".to_owned(),
            Self::VerifyFailure => "verify_failure".to_owned(),
            Self::PreviousDiff => "previous_diff".to_owned(),
            Self::CommandQueue => "command_queue".to_owned(),
            Self::JudgeTask => "judge_task".to_owned(),
            Self::JudgeCandidate(index) => format!("judge_candidate:{index}"),
            Self::StepSummary => "step_summary".to_owned(),
            Self::DiffSoFar => "diff_so_far".to_owned(),
            Self::FailureReason => "failure_reason".to_owned(),
        }
    }

    /// Whether the trimmer may never take a token from this section (§4.4 `:824-830`).
    ///
    /// `failure_reason` is protected in a handoff prompt and is not renderable anywhere else, so
    /// the role argument is what keeps the set honest rather than a comment.
    #[must_use]
    pub const fn is_protected(&self, role: TemplateRole) -> bool {
        match self {
            Self::Template | Self::Box | Self::Skills | Self::CommandQueue => true,
            Self::FailureReason => matches!(role, TemplateRole::Handoff),
            _ => false,
        }
    }
}

impl core::fmt::Display for SectionName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

impl Serialize for SectionName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.render())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_names_are_one_vocabulary_with_two_uses() {
        assert_eq!(
            SectionName::Documents("plan".to_owned()).render(),
            "documents:plan"
        );
        assert_eq!(SectionName::JudgeCandidate(2).render(), "judge_candidate:2");
        assert_eq!(SectionName::Box.to_string(), "box");
        assert_eq!(
            serde_json::to_value(SectionName::VerifyFailure).expect("a string"),
            serde_json::Value::String("verify_failure".to_owned()),
            "the `name=` attribute and the `sections[].name` key are one function"
        );
    }

    #[test]
    fn the_protected_set_is_role_dependent_only_for_failure_reason() {
        for role in [
            TemplateRole::Phase,
            TemplateRole::Judge,
            TemplateRole::Handoff,
        ] {
            for name in [
                SectionName::Template,
                SectionName::Box,
                SectionName::Skills,
                SectionName::CommandQueue,
            ] {
                assert!(name.is_protected(role), "{name} is protected in {role:?}");
            }
            for name in [
                SectionName::Item,
                SectionName::Documents("plan".to_owned()),
                SectionName::Upstream,
                SectionName::Excerpts,
                SectionName::VerifyFailure,
                SectionName::PreviousDiff,
                SectionName::JudgeTask,
                SectionName::JudgeCandidate(0),
                SectionName::StepSummary,
                SectionName::DiffSoFar,
            ] {
                assert!(!name.is_protected(role), "{name} is trimmable in {role:?}");
            }
        }
        assert!(SectionName::FailureReason.is_protected(TemplateRole::Handoff));
        assert!(!SectionName::FailureReason.is_protected(TemplateRole::Phase));
    }
}
