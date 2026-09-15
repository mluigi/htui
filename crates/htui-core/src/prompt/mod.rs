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
pub mod trim;

#[cfg(feature = "test-support")]
pub mod fixtures;

pub use defaults::{COMMAND_QUEUE_TEXT, DEFAULT_TEMPLATES, body_of};
pub use estimate::TokenEstimator;
pub use excerpt::ExcerptSet;
pub use settings::{Budget, BudgetSource, DEFAULTS, Defaults};
pub use template::{ParsedTemplate, Placeholder, Span, TemplateError, TemplateRole, parse};
pub use trim::{Section, SectionEntry, TrimRecord, TrimStrategy};

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::model::box_::BoxProfile;
use crate::model::link::UpstreamEntry;
use crate::model::skill::BoundSkill;
use crate::prompt::render::Rendered;
use crate::prompt::trim::{Inputs, Trimmer};
use crate::scrub::Scrubber;

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

/// The assembled prompt: one `String`, three uses (§4.7 step 8).
///
/// `text` is canonical and is what is sent, what is digested and what is persisted. Canonicalising
/// for the hash while sending the original bytes was considered and rejected: the digest would then
/// describe a string nobody was given.
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledPrompt {
    /// The canonical assembled prompt.
    pub text: String,
    /// `sha256` over [`text`](Self::text)'s UTF-8 bytes, lowercase hex, 64 characters.
    pub digest: String,
    /// The record's own rows, kept here so a reader need not open the record.
    pub sections: Vec<Section>,
    /// `run_step.trim_record`.
    pub trim: TrimRecord,
}

impl AssembledPrompt {
    /// The `prompt` payload's `sections[]` — **the** one map (ANA-5 risk 12).
    ///
    /// Delegates to [`TrimRecord::section_entries`]; nothing else constructs a [`SectionEntry`].
    #[must_use]
    pub fn payload_sections(&self) -> Vec<SectionEntry> {
        self.trim.section_entries()
    }

    /// The same projection as JSON, ready for `Recorder::record_prompt`'s `sections` argument
    /// (`record.rs:571-575` takes the text and the array separately; this is the array).
    #[must_use]
    pub fn payload_sections_value(&self) -> Value {
        serde_json::to_value(self.payload_sections()).unwrap_or(Value::Null)
    }
}

/// Why a prompt could not be assembled. Every variant happens **before** a session starts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssembleError {
    /// Criterion 3's exact text: a `prompt_template` row that bypassed MOD-9's validator.
    ///
    /// Covers [`TemplateError::UnknownPlaceholder`] and [`TemplateError::WrongRole`] alike — to a
    /// caller both are "this template named a placeholder it may not use", and the fix is the same
    /// edit.
    #[error("unknown prompt placeholder: {{{{{token}}}}}")]
    UnknownPlaceholder {
        /// The offending token, without its braces.
        token: String,
    },
    /// [`TemplateError::Unterminated`] and [`TemplateError::MissingRequired`], with the scanner's
    /// own text and byte offset.
    #[error("prompt template does not parse: {0}")]
    Template(TemplateError),
    /// §4.4 step 4: the one budget condition that is an error rather than a trim.
    #[error("prompt budget too small: protected sections need {needed} tokens, target is {target}")]
    BudgetTooSmall {
        /// What the frame and the protected sections cost together.
        needed: i64,
        /// `floor(budget × (1 − reserve))`.
        target: i64,
    },
    /// §4.2's aggregate cap, which refuses rather than dropping a binding (`R-ID-5`).
    #[error("skills exceed max_skill_tokens ({tokens} > {cap})")]
    SkillsExceedCap {
        /// The rendered skills section's estimated tokens.
        tokens: i64,
        /// `app_setting.max_skill_tokens`, as the caller resolved it.
        cap: i64,
    },
    /// Plan D100: the scrubber found residue in one section after masking (`R-SEC-3`).
    ///
    /// Names the section and never the text, which is why the scrub runs per section rather than
    /// over the assembled whole: the section is the fact a maintainer can act on.
    #[error("unmasked {rule} in section `{section}` at `{path}`")]
    Unmasked {
        /// The section's rendered name, or the scalar placeholder's token.
        section: String,
        /// The scrubber's rule name.
        rule: &'static str,
        /// The scrubber's JSON pointer into the wrapped value; `""` is the whole string.
        path: String,
    },
    /// A role's inputs are missing, which is a caller bug rather than a template one.
    #[error("a {role:?} template needs {what}, which the spec does not carry")]
    RoleInputs {
        /// The role the body was parsed against.
        role: TemplateRole,
        /// What was missing.
        what: &'static str,
    },
}

impl From<TemplateError> for AssembleError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::UnknownPlaceholder { token, .. }
            | TemplateError::WrongRole { token, .. } => Self::UnknownPlaceholder { token },
            other => Self::Template(other),
        }
    }
}

/// The one entry point: `PromptSpec` in, canonical bytes and their audit out (§4.7 `:1385-1398`).
///
/// **Pure by contract** (ANA-5 invariant 2): the same spec and the same scrubber rules produce the
/// same bytes on any box. There is no store handle, no I/O, no clock and no unordered map in the
/// path, and the two inputs whose order a caller could get wrong — the upstream walk and the skill
/// bindings — are re-sorted here rather than trusted, because three backends and two collations
/// produce them.
///
/// `scrubber` is an argument rather than a field of [`PromptSpec`] because `PromptSpec` is
/// `Clone + PartialEq` and `dyn Scrubber` is neither. `MinimalScrubber::new([])` is what a caller
/// with no session secrets passes, and it still fails closed on the prefix rules.
///
/// # Errors
///
/// Every variant of [`AssembleError`], all of them before a session starts and before anything is
/// digested — which is the half of criteria 3 and 10 that matters.
pub fn assemble(
    spec: &PromptSpec,
    scrubber: &dyn Scrubber,
) -> Result<AssembledPrompt, AssembleError> {
    // 1. Parse the pinned body against its role's closed set.
    let parsed = parse(spec.role, &spec.body)?;
    match spec.role {
        TemplateRole::Judge if spec.judge.is_none() => {
            return Err(AssembleError::RoleInputs {
                role: spec.role,
                what: "judge inputs",
            });
        }
        TemplateRole::Handoff if spec.handoff.is_none() => {
            return Err(AssembleError::RoleInputs {
                role: spec.role,
                what: "handoff inputs",
            });
        }
        _ => {}
    }

    // 2. Render every section at full size, from inputs in canonical order (§4.7 rules 2, 4, 6).
    let mut upstream = spec.upstream.clone();
    UpstreamEntry::sort_canonical(&mut upstream);
    let skills = BoundSkill::collapse(spec.skills.clone(), Vec::new());
    let candidates = judge_candidates(spec);

    let mut rendered: Vec<(Placeholder, Rendered, i64)> = Vec::new();
    for placeholder in &parsed.used {
        if !placeholder.is_section() {
            continue;
        }
        let weight = weight_of(&parsed, *placeholder);
        for section in render_sections(*placeholder, spec, &upstream, &skills, &candidates) {
            rendered.push((*placeholder, section, weight));
        }
    }

    // 3. Scrub per section, before anything is measured or hashed (plan D100, §4.7 step 2).
    for (_, section, _) in &mut rendered {
        let name = section.name.render();
        section.content = scrub_text(scrubber, &section.content, &name)?;
        for (_, value) in &mut section.attrs {
            *value = scrub_text(scrubber, value, &name)?;
        }
    }
    let template_text = scrub_text(scrubber, &render::template_text(&parsed), "template")?;
    let scalars = scalar_substitutions(spec, scrubber)?;

    // 4. Estimate, 5. refuse, 6. trim — all three inside the trimmer, which owns the arithmetic.
    let est = spec.estimator;
    let template_tokens = est.estimate(&template_text);
    let inputs = Inputs {
        spec,
        upstream: &upstream,
        candidates: &candidates,
    };
    let mut trimmer = Trimmer::new(inputs, est, spec.budget.target(), template_tokens, rendered);
    trimmer.refusals()?;
    trimmer.run();
    let (sections, live, kept_excerpts) = trimmer.finish();

    // 7. Substitute in span order: the frame's literals verbatim, each slot's sections wrapped.
    let mut blocks: BTreeMap<Placeholder, Vec<String>> = BTreeMap::new();
    for (placeholder, section) in &live {
        blocks
            .entry(*placeholder)
            .or_default()
            .push(render::wrap(section));
    }
    let mut text = String::with_capacity(spec.body.len() * 4);
    for span in &parsed.spans {
        match span {
            Span::Literal(literal) => text.push_str(&render::normalise_newlines(literal)),
            Span::Slot(placeholder) if placeholder.is_section() => {
                if let Some(rendered) = blocks.get(placeholder) {
                    // A blank line between two sections under one placeholder: `{{documents}}`
                    // stands for a list of blocks, and §4.7 step 5 collapses any run this leaves.
                    text.push_str(&rendered.join("\n\n"));
                }
            }
            Span::Slot(placeholder) => {
                if let Some(scalar) = scalars.get(placeholder) {
                    text.push_str(scalar);
                }
            }
        }
    }

    // 8. Canonicalise and digest. 9. Record.
    let text = digest::canonical(&text);
    let digest = digest::sha256_hex(&text);
    let record = trim::record(
        spec,
        &spec.template,
        template_tokens,
        sections,
        surviving_audit(spec, &kept_excerpts),
        notes(spec),
    );
    Ok(AssembledPrompt {
        text,
        digest,
        sections: record.sections.clone(),
        trim: record,
    })
}

/// How many times the body places this placeholder. Duplicates are legal (§4.1) and each
/// occurrence substitutes, so a section placed twice costs its tokens twice (`:404-405`).
fn weight_of(parsed: &ParsedTemplate, placeholder: Placeholder) -> i64 {
    let uses = parsed
        .spans
        .iter()
        .filter(|span| matches!(span, Span::Slot(slot) if *slot == placeholder))
        .count();
    i64::try_from(uses).unwrap_or(1).max(1)
}

/// The judge's candidates in the order **this** call renders them (§4.7 rule 6).
///
/// Sorted by `fanout_index`, and reversed only for the second judge call — which is a different
/// prompt with a different digest by design, and is how ANA-2 tests a judge for order bias.
fn judge_candidates(spec: &PromptSpec) -> Vec<JudgeCandidate> {
    let Some(judge) = &spec.judge else {
        return Vec::new();
    };
    let mut candidates = judge.candidates.clone();
    candidates.sort_by_key(|candidate| candidate.fanout_index);
    if judge.reverse {
        candidates.reverse();
    }
    candidates
}

/// Every section one placeholder stands for, in the order it renders them.
///
/// A placeholder with no data renders nothing and gets no `sections[]` entry (§4.2 `:566-569`);
/// `item` is the exception named there, because "this item has no body" is information.
fn render_sections(
    placeholder: Placeholder,
    spec: &PromptSpec,
    upstream: &[UpstreamEntry],
    skills: &[BoundSkill],
    candidates: &[JudgeCandidate],
) -> Vec<Rendered> {
    match placeholder {
        Placeholder::Item => vec![render::item(spec)],
        Placeholder::Documents => spec.documents.iter().map(render::document).collect(),
        Placeholder::Upstream => render::upstream(upstream, &[]).into_iter().collect(),
        Placeholder::Box => vec![render::box_profile(&spec.box_profile)],
        Placeholder::Skills => render::skills(skills).into_iter().collect(),
        Placeholder::Excerpts => render::excerpts(&spec.excerpts.files).into_iter().collect(),
        Placeholder::VerifyFailure => spec
            .verify_failure
            .iter()
            .map(render::verify_failure)
            .collect(),
        Placeholder::PreviousDiff => spec
            .previous_diff
            .iter()
            .map(|diff| render::diff(SectionName::PreviousDiff, diff, false))
            .collect(),
        Placeholder::CommandQueue => {
            if spec.command_queue {
                vec![render::command_queue()]
            } else {
                Vec::new()
            }
        }
        Placeholder::Task => spec
            .judge
            .iter()
            .map(|judge| render::judge_task(&judge.task))
            .collect(),
        Placeholder::Candidates => candidates
            .iter()
            .map(|candidate| render::judge_candidate(candidate, false))
            .collect(),
        Placeholder::StepSummary => spec
            .handoff
            .iter()
            .map(|handoff| render::step_summary(&handoff.step_summary))
            .collect(),
        Placeholder::DiffSoFar => spec
            .handoff
            .iter()
            .filter_map(|handoff| handoff.diff_so_far.as_ref())
            .map(|diff| render::diff(SectionName::DiffSoFar, diff, false))
            .collect(),
        Placeholder::FailureReason => spec
            .handoff
            .iter()
            .map(|handoff| render::failure_reason(&handoff.failure_reason))
            .collect(),
        Placeholder::ItemKey
        | Placeholder::ItemTitle
        | Placeholder::ItemKind
        | Placeholder::Phase
        | Placeholder::OutputKind
        | Placeholder::Attempt => Vec::new(),
    }
}

/// The six scalars of §4.1 `:362`, substituted directly rather than as sections.
///
/// `item_title` is collapsed and truncated by [`render::one_line_title`] rather than
/// attribute-escaped: a scalar lands in a line of the frame, so a title carrying a newline would
/// break the frame's own structure, while `&amp;` in running prose would be an escape for a syntax
/// that is not there. Every one of them is scrubbed, because every one of them is a digested byte.
fn scalar_substitutions(
    spec: &PromptSpec,
    scrubber: &dyn Scrubber,
) -> Result<BTreeMap<Placeholder, String>, AssembleError> {
    let raw = [
        (Placeholder::ItemKey, spec.item_key.clone()),
        (
            Placeholder::ItemTitle,
            render::one_line_title(&spec.item_title),
        ),
        (Placeholder::ItemKind, spec.item_kind.clone()),
        (Placeholder::Phase, spec.phase.clone()),
        (
            Placeholder::OutputKind,
            spec.output_kind.clone().unwrap_or_default(),
        ),
        (Placeholder::Attempt, spec.attempt.to_string()),
    ];
    let mut out = BTreeMap::new();
    for (placeholder, value) in raw {
        let scrubbed = scrub_text(scrubber, &value, placeholder.token())?;
        out.insert(placeholder, scrubbed);
    }
    Ok(out)
}

/// Plan D100's two lines: wrap the content in a `Value::String`, scrub it, unwrap it.
///
/// No trait change and no second scrubber, so `R-SEC-3` keeps one fail-closed implementation and
/// MOD-10 still has exactly one surface to replace.
fn scrub_text(
    scrubber: &dyn Scrubber,
    content: &str,
    section: &str,
) -> Result<String, AssembleError> {
    let mut value = Value::String(content.to_owned());
    scrubber
        .scrub(&mut value)
        .map_err(|unmasked| AssembleError::Unmasked {
            section: section.to_owned(),
            rule: unmasked.rule,
            path: unmasked.path,
        })?;
    Ok(match value {
        Value::String(masked) => masked,
        // `mask_value` rewrites a string leaf in place and cannot change its type; this arm exists
        // so a future scrubber that did would lose the content loudly rather than silently.
        other => other.to_string(),
    })
}

/// §4.5's audit with `files[]` narrowed to what survived the trim (§5.1 `:1536-1538`).
///
/// `selected` and `files` are deliberately allowed to disagree: `selected` is what the ranker chose
/// and paid for, `files` is what reached the model. A dropped section therefore leaves `selected:
/// 3` beside an empty `files`, and a reader can see the difference.
fn surviving_audit(spec: &PromptSpec, kept: &[usize]) -> excerpt::ExcerptAudit {
    let surviving: Vec<(&str, &str)> = kept
        .iter()
        .filter_map(|index| spec.excerpts.files.get(*index))
        .map(|file| (file.repo.as_str(), file.path.as_str()))
        .collect();
    let mut audit = spec.excerpts.audit.clone();
    audit
        .files
        .retain(|file| surviving.contains(&(file.repo.as_str(), file.path.as_str())));
    audit
}

/// `trim_record.notes`: the caller's first (plan D103's declared stand-ins), then the excerpt
/// pass's. Both are conditions that are not errors (§5.1 `:1549`).
fn notes(spec: &PromptSpec) -> Vec<String> {
    let mut notes = spec.notes.clone();
    notes.extend(spec.excerpts.notes.iter().cloned());
    notes
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
            Value::String("verify_failure".to_owned()),
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
