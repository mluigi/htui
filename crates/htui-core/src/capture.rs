//! MOD-164 D9, D10: the capture core, pure. When a finished item run earns a capture step
//! ([`capture_trigger`]), what evidence the light model is shown ([`EvidencePack`]), the fixed
//! prompt it is given ([`capture_prompt`]), and how its answer is checked before anything reaches
//! the store ([`parse_capture`]).
//!
//! Nothing here touches a store, a clock or a session: the engine (`htui-orch`'s
//! `engine/capture.rs`) reads the steps and their command output, hands them in, and writes what
//! [`parse_capture`] keeps through `WriteStore::write_run_memory`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::model::memory::{
    MEMORY_BODY_MAX, MEMORY_RUN_WRITE_CAP, MEMORY_SUBJECT_MAX, MemoryBody, MemoryCitation,
    MemoryEvidence, MemoryKind, MemorySubject, MemoryTextError, NewRunMemory,
};
use crate::model::run::{GateOutcome, RunStep, StepRole, StepStatus, VerifyOutcome};
use crate::model::{MemoryEntryId, StepId};
use crate::prompt::digest::{canonical, sha256_hex};
use crate::prompt::excerpt::ExcerptAudit;
use crate::prompt::trim::TemplateRecord;
use crate::prompt::{
    AssembledPrompt, Budget, BudgetSource, Section, SectionName, TemplateRole, TokenEstimator,
    TrimRecord, TrimStrategy,
};
use crate::scrub::Scrubber;

/// D10: the cap, in `chars-v2` tokens, on the evidence pack's rendered text. Output tails are cut
/// first, then whole oldest blocks.
pub const CAPTURE_INPUT_TOKEN_CAP: i64 = 6_000;

/// D10: the cap, in `chars-v2` tokens, on the capture step's answer. A longer answer is refused
/// whole.
pub const CAPTURE_OUTPUT_TOKEN_CAP: i64 = 2_000;

/// D10: the shortest quote, in characters after trimming, a citation may carry.
pub const CAPTURE_QUOTE_MIN: usize = 12;

/// `trim_record.template.name` of a capture prompt. Not a `prompt_template` row: the text is
/// internal and fixed.
pub const CAPTURE_TEMPLATE: &str = "capture";

/// `trim_record.v` as `assemble` writes it (`prompt/trim.rs` `RECORD_VERSION`, private there).
/// `tests/capture.rs` pins the two equal.
const TRIM_RECORD_VERSION: u8 = 5;

/// A tail longer than this many characters cannot fit the pack at any `chars-v2` rate, so the
/// search starts here rather than at a megabyte of build log.
const TAIL_CEILING: usize = 3 * 6_000;

/// Why a run earns a capture step (D9): the first signal in walk order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureReason {
    /// A step ended `failed`.
    StepFailed,
    /// A step ran a second or later attempt.
    Retried,
    /// A step's verify command failed.
    VerifyFailed,
    /// A gate answered `rejected` or `retried`.
    GateRejected,
}

impl CaptureReason {
    /// The one spelling, for notes and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StepFailed => "step_failed",
            Self::Retried => "retried",
            Self::VerifyFailed => "verify_failed",
            Self::GateRejected => "gate_rejected",
        }
    }
}

/// D9: whether a run's steps earn a capture step, and why. Only rows that are not stage rows and
/// have `fanout_index >= 0` count, so a judge or a stage (`capture` included) never triggers one.
/// The rows are read in walk order (position, attempt), and the first signal is the answer.
#[must_use]
pub fn capture_trigger(steps: &[RunStep]) -> Option<CaptureReason> {
    walk_order(steps).into_iter().find_map(signal)
}

/// The steps D9 reads, in walk order: position, attempt, then fan-out index and id so the order
/// is total. Walk order is by position, not by time: the "oldest" blocks the pack drops first are
/// the ones at the lowest walk positions, which on a branching graph need not be the first to run.
fn walk_order(steps: &[RunStep]) -> Vec<&RunStep> {
    let mut rows: Vec<&RunStep> = steps.iter().filter(|step| counts(step)).collect();
    rows.sort_by_key(|step| {
        (
            step.position,
            step.attempt,
            step.fanout_index,
            step.id.as_uuid(),
        )
    });
    rows
}

/// D9's row filter: not a stage row, not a judge.
fn counts(step: &RunStep) -> bool {
    step.role != StepRole::Stage && step.fanout_index >= 0
}

/// D9's per-row signal.
fn signal(step: &RunStep) -> Option<CaptureReason> {
    if step.status == StepStatus::Failed {
        Some(CaptureReason::StepFailed)
    } else if step.verify_outcome == Some(VerifyOutcome::Fail) {
        Some(CaptureReason::VerifyFailed)
    } else if matches!(
        step.gate_outcome,
        Some(GateOutcome::Rejected | GateOutcome::Retried)
    ) {
        Some(CaptureReason::GateRejected)
    } else if step.attempt > 1 {
        Some(CaptureReason::Retried)
    } else {
        None
    }
}

/// A failed attempt, in the sense the win is measured against: D9's signals less the retry
/// itself.
fn failed_attempt(step: &RunStep) -> bool {
    matches!(
        signal(step),
        Some(CaptureReason::StepFailed | CaptureReason::VerifyFailed | CaptureReason::GateRejected)
    )
}

/// One step's evidence, as the capture prompt shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBlock {
    /// `S1..Sn`, in walk order: the name the model cites.
    pub label: String,
    /// The step the block describes.
    pub step_id: StepId,
    /// The rendered block, `<block label="…">` to `</block>`: htui's fixed label lines around
    /// the step's [`spans`](Self::spans).
    pub text: String,
    /// The step's own evidence as the block shows it: its `gate_note` and its output tail (cut
    /// as rendered). A citation's quote must be a substring of one of these, never of htui's
    /// fixed wording, so quoting a label line (`passed after a failed attempt`, `phase: …`) cites
    /// nothing.
    pub spans: Vec<String>,
}

/// D10: the evidence a capture step is shown, one [`EvidenceBlock`] per notable step, within
/// [`CAPTURE_INPUT_TOKEN_CAP`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePack {
    blocks: Vec<EvidenceBlock>,
    dropped: usize,
}

/// A notable step before rendering: the row, whether it is the win, and its output.
struct Notable<'a> {
    step: &'a RunStep,
    win: bool,
    output: Option<&'a str>,
}

impl EvidencePack {
    /// Builds the pack from a run's steps and, per step, its latest `command_run.output`.
    ///
    /// A step is notable if it meets D9's row signal, or if it is the passing attempt right after a
    /// failed attempt at the same position (the win, which a retry always is, and which its block
    /// says). Blocks are labelled `S1..Sn` in walk order. Over [`CAPTURE_INPUT_TOKEN_CAP`]
    /// (`chars-v2`), the output tails are cut first, every block to the same length and keeping
    /// the end, where a failure prints; when even no tail at all does not fit, the oldest blocks
    /// are dropped whole and the rest relabelled from `S1`. "Oldest" is the lowest walk position
    /// (position, then attempt), not the earliest to run.
    #[must_use]
    pub fn build(steps: &[RunStep], outputs: &BTreeMap<StepId, String>) -> Self {
        let rows = walk_order(steps);
        let notable: Vec<Notable<'_>> = rows
            .iter()
            .filter_map(|step| {
                let win = step.status == StepStatus::Done
                    && rows.iter().any(|other| {
                        other.position == step.position
                            && other.attempt == step.attempt - 1
                            && failed_attempt(other)
                    });
                (win || signal(step).is_some()).then(|| Notable {
                    step,
                    win,
                    output: outputs.get(&step.id).map(|out| out.trim_end()),
                })
            })
            .collect();

        for dropped in 0..notable.len() {
            let kept = &notable[dropped..];
            if let Some(blocks) = fit(kept) {
                return Self { blocks, dropped };
            }
        }
        Self {
            blocks: Vec::new(),
            dropped: notable.len(),
        }
    }

    /// The blocks, in label order.
    #[must_use]
    pub fn blocks(&self) -> &[EvidenceBlock] {
        &self.blocks
    }

    /// The block a label names; labels are exact (`S1`, never `s1`).
    #[must_use]
    pub fn block(&self, label: &str) -> Option<&EvidenceBlock> {
        self.blocks.iter().find(|block| block.label == label)
    }

    /// The step a label names.
    #[must_use]
    pub fn step_of(&self, label: &str) -> Option<StepId> {
        self.block(label).map(|block| block.step_id)
    }

    /// The rendered pack: every block, in order.
    #[must_use]
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect()
    }

    /// [`text`](Self::text)'s `chars-v2` estimate; at most [`CAPTURE_INPUT_TOKEN_CAP`].
    #[must_use]
    pub fn tokens(&self) -> i64 {
        TokenEstimator::DEFAULT.estimate(&self.text())
    }

    /// Whether no step was notable, or none fitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// How many of the oldest notable steps were dropped to meet the cap.
    #[must_use]
    pub const fn dropped_blocks(&self) -> usize {
        self.dropped
    }
}

/// The longest common tail length at which `kept` fits the cap, rendered; `None` when not even
/// tail-less blocks fit.
fn fit(kept: &[Notable<'_>]) -> Option<Vec<EvidenceBlock>> {
    let longest = kept
        .iter()
        .filter_map(|notable| notable.output.map(|out| out.chars().count()))
        .max()
        .unwrap_or(0);
    let render = |limit: usize| -> (Vec<EvidenceBlock>, bool) {
        let blocks: Vec<EvidenceBlock> = kept
            .iter()
            .enumerate()
            .map(|(index, notable)| render_block(index + 1, notable, limit))
            .collect();
        let text: String = blocks.iter().map(|block| block.text.as_str()).collect();
        let fits = TokenEstimator::DEFAULT.estimate(&text) <= CAPTURE_INPUT_TOKEN_CAP;
        (blocks, fits)
    };

    let ceiling = longest.min(TAIL_CEILING);
    let (blocks, fits) = render(ceiling);
    if fits {
        return Some(blocks);
    }
    let (mut best, fits) = render(0);
    if !fits {
        return None;
    }
    // The cost is monotone in the limit, near enough: the search records only limits it measured
    // as fitting, so the answer fits whatever the estimator's span toggles do.
    let (mut low, mut high) = (1, ceiling.saturating_sub(1));
    while low <= high {
        let mid = low + (high - low) / 2;
        let (blocks, fits) = render(mid);
        if fits {
            best = blocks;
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    Some(best)
}

/// One block, its output cut to the last `limit` characters.
fn render_block(number: usize, notable: &Notable<'_>, limit: usize) -> EvidenceBlock {
    let step = notable.step;
    let label = format!("S{number}");
    let mut spans = Vec::new();
    let mut text = format!(
        "<block label=\"{label}\">\nphase: {}\nattempt: {}\nstatus: {}\n",
        step.phase_name,
        step.attempt,
        step.status.as_str()
    );
    if let Some(code) = step.exit_code {
        text.push_str(&format!("exit_code: {code}\n"));
    }
    if let Some(verify) = step.verify_outcome {
        text.push_str(&format!("verify: {}\n", verify.as_str()));
    }
    if let Some(gate) = step.gate_outcome {
        text.push_str(&format!("gate: {}\n", gate.as_str()));
    }
    if let Some(note) = &step.gate_note {
        text.push_str(&format!("gate_note: {note}\n"));
        spans.push(note.clone());
    }
    if notable.win {
        text.push_str("result: passed after a failed attempt at this phase\n");
    }
    if let Some(output) = notable.output.filter(|out| !out.is_empty()) {
        text.push_str("output (tail):\n");
        let total = output.chars().count();
        if total > limit {
            let cut = total - limit;
            let start = output
                .char_indices()
                .nth(cut)
                .map_or(output.len(), |(at, _)| at);
            text.push_str(&format!("[… {cut} earlier characters cut]\n"));
            text.push_str(&output[start..]);
            spans.push(output[start..].to_owned());
        } else {
            text.push_str(output);
            spans.push(output.to_owned());
        }
        text.push('\n');
    }
    text.push_str("</block>\n");
    EvidenceBlock {
        label,
        step_id: step.id,
        text,
        spans,
    }
}

/// D10: the capture step's prompt, an internal fixed text around the pack.
///
/// Built in `assemble`'s shape, because the engine's recorder takes an [`AssembledPrompt`]: the
/// text is canonical, nothing is undigested (so `digest_text` is the text), the digest is its
/// `sha256`, and the record carries one `template` section holding the whole estimate. The record
/// names the template [`CAPTURE_TEMPLATE`] at version 1, and its budget is the two caps together.
#[must_use]
pub fn capture_prompt(pack: &EvidencePack) -> AssembledPrompt {
    let text = canonical(&format!(
        "You are the capture step of a finished htui run. The run hit trouble: a failure, a \
         retry, a failed verify or a rejected gate. Below is the evidence from its notable steps, \
         one block per step, {labels}.\n\n\
         Write down what this run teaches about the project, as memory entries that later runs \
         on this project will be shown.\n\n\
         Rules:\n\
         - Write at most {cap} entries.\n\
         - Each entry's kind is one of: fact, decision, gotcha, lesson.\n\
         - A subject is at most {subject} characters; a body is at most {body} characters.\n\
         - Every entry cites at least one block, with a verbatim quote of at least {quote} \
         characters copied from that block.\n\
         - Write lasting project knowledge, not a story of the run.\n\
         - Zero entries is a valid answer: write none when the run teaches nothing lasting.\n\
         - Call `document_write` once, with this JSON as the document body and nothing else:\n\
         {{\"entries\":[{{\"kind\":\"gotcha\",\"subject\":\"...\",\"body\":\"...\",\
         \"evidence\":[{{\"block\":\"S1\",\"quote\":\"...\"}}]}}]}}\n\n\
         <evidence>\n{pack}</evidence>\n",
        labels = match pack.blocks.len() {
            0 => "but no block fitted".to_owned(),
            1 => "labelled S1".to_owned(),
            count => format!("labelled S1 to S{count}"),
        },
        cap = MEMORY_RUN_WRITE_CAP,
        subject = MEMORY_SUBJECT_MAX,
        body = MEMORY_BODY_MAX,
        quote = CAPTURE_QUOTE_MIN,
        pack = pack.text(),
    ));
    let estimator = TokenEstimator::DEFAULT;
    let tokens = estimator.estimate(&text);
    let budget = Budget {
        tokens: CAPTURE_INPUT_TOKEN_CAP + CAPTURE_OUTPUT_TOKEN_CAP,
        source: BudgetSource::AppSettingDefault,
        reserve_bp: 0,
    };
    let sections = vec![Section {
        name: SectionName::Template,
        tokens_before: tokens,
        tokens_after: tokens,
        strategy: TrimStrategy::None,
        trimmed: false,
        elided_lines: None,
        elided_bytes: None,
        stubbed: None,
        dropped: None,
    }];
    let notes = if pack.dropped == 0 {
        Vec::new()
    } else {
        vec![format!(
            "capture: dropped the {} oldest evidence blocks to meet the {CAPTURE_INPUT_TOKEN_CAP}-token cap",
            pack.dropped
        )]
    };
    let trim = TrimRecord {
        v: TRIM_RECORD_VERSION,
        template: TemplateRecord {
            name: CAPTURE_TEMPLATE.to_owned(),
            version: 1,
            role: TemplateRole::Phase,
        },
        budget: budget.tokens,
        budget_source: budget.source,
        reserve: budget.reserve(),
        target: budget.target(),
        estimator: estimator.id,
        estimated_before: tokens,
        estimated_after: tokens,
        sections: sections.clone(),
        skill_choices: Vec::new(),
        undigested: Vec::new(),
        memory: None,
        excerpts: ExcerptAudit::default(),
        notes,
    };
    AssembledPrompt {
        digest: sha256_hex(&text),
        digest_text: text.clone(),
        text,
        sections,
        trim,
    }
}

/// One entry [`parse_capture`] kept: everything of a [`NewRunMemory`] but its id and commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedEntry {
    /// The kind.
    pub kind: MemoryKind,
    /// The checked, scrubbed subject.
    pub subject: MemorySubject,
    /// The checked, scrubbed body.
    pub body: MemoryBody,
    /// The citations, labels mapped to steps and quotes scrubbed.
    pub evidence: MemoryEvidence,
}

impl CapturedEntry {
    /// The write argument, with the caller's id and the engine's commit (never the model's).
    #[must_use]
    pub fn into_new(self, id: MemoryEntryId, commit: Option<String>) -> NewRunMemory {
        NewRunMemory {
            id,
            kind: self.kind,
            subject: self.subject,
            body: self.body,
            evidence: self.evidence,
            commit,
        }
    }
}

/// Why [`parse_capture`] dropped one entry. Never carries the entry's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    /// The kind is not `fact`, `decision`, `gotcha` or `lesson`.
    BadKind,
    /// [`MemorySubject::new`] refused the subject.
    Subject(MemoryTextError),
    /// [`MemoryBody::new`] refused the body.
    Body(MemoryTextError),
    /// The entry cites nothing.
    NoEvidence,
    /// A citation names a block the pack does not hold.
    UnknownBlock,
    /// A trimmed quote is shorter than [`CAPTURE_QUOTE_MIN`] characters.
    ShortQuote,
    /// A trimmed quote is not a substring of the block it cites.
    QuoteNotFound,
    /// The scrubber refused a quote.
    UnscrubbableQuote {
        /// The scrubber's rule.
        rule: &'static str,
    },
    /// The entry came after [`MEMORY_RUN_WRITE_CAP`] entries were already kept.
    OverCap,
}

impl DropReason {
    /// The one spelling, for the item note.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BadKind => "bad_kind",
            Self::Subject(_) => "bad_subject",
            Self::Body(_) => "bad_body",
            Self::NoEvidence => "no_evidence",
            Self::UnknownBlock => "unknown_block",
            Self::ShortQuote => "short_quote",
            Self::QuoteNotFound => "quote_not_found",
            Self::UnscrubbableQuote { .. } => "unscrubbable_quote",
            Self::OverCap => "over_cap",
        }
    }
}

/// What [`parse_capture`] made of an answer it accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOutput {
    /// The entries to write, in answer order.
    pub kept: Vec<CapturedEntry>,
    /// Each dropped entry's index in the answer, and why.
    pub dropped: Vec<(usize, DropReason)>,
}

/// Why [`parse_capture`] refused an answer whole. Never carries the answer's text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureRefusal {
    /// The answer is estimated over [`CAPTURE_OUTPUT_TOKEN_CAP`].
    #[error("the capture answer is {tokens} tokens, over the {cap}-token cap")]
    TooLarge {
        /// The answer's `chars-v2` estimate.
        tokens: i64,
        /// [`CAPTURE_OUTPUT_TOKEN_CAP`].
        cap: i64,
    },
    /// The answer is not JSON, not the expected shape, or names a field the shape lacks.
    #[error("the capture answer is not the expected JSON (line {line}, column {column})")]
    Malformed {
        /// serde's line, 1-based.
        line: usize,
        /// serde's column, 1-based.
        column: usize,
    },
}

/// The answer's shape: `{"entries":[…]}` and nothing else.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnswer {
    entries: Vec<RawEntry>,
}

/// One entry as written. `kind` is a string so a bad kind drops the entry rather than refusing
/// the answer; a missing `evidence` reads as empty and drops it the same way.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    kind: String,
    subject: String,
    body: String,
    #[serde(default)]
    evidence: Vec<RawCitation>,
}

/// One citation as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCitation {
    block: String,
    quote: String,
}

/// D10: checks the capture step's answer against the pack it was shown.
///
/// The answer is refused whole when it is estimated over [`CAPTURE_OUTPUT_TOKEN_CAP`], or when it
/// does not parse as `{"entries":[{"kind","subject","body","evidence":[{"block","quote"}]}]}`
/// (unknown fields included). Otherwise each entry is kept or dropped with a [`DropReason`]:
/// once [`MEMORY_RUN_WRITE_CAP`] entries are kept, every later one is [`DropReason::OverCap`] (an
/// invalid entry uses up no slot); a kept entry has a known kind, a subject and body
/// [`MemorySubject::new`] / [`MemoryBody::new`] accept (scrubbed, fail-closed), and at least one
/// citation, each naming a block of the pack with a trimmed quote of at least
/// [`CAPTURE_QUOTE_MIN`] characters found in one of that block's evidence
/// [`spans`](EvidenceBlock::spans). Quotes are scrubbed and labels mapped to their [`StepId`].
///
/// # Errors
/// [`CaptureRefusal`] when the answer is refused whole.
pub fn parse_capture(
    body: &str,
    pack: &EvidencePack,
    scrubber: &dyn Scrubber,
) -> Result<CaptureOutput, CaptureRefusal> {
    let tokens = TokenEstimator::DEFAULT.estimate(body);
    if tokens > CAPTURE_OUTPUT_TOKEN_CAP {
        return Err(CaptureRefusal::TooLarge {
            tokens,
            cap: CAPTURE_OUTPUT_TOKEN_CAP,
        });
    }
    let answer: RawAnswer =
        serde_json::from_str(body).map_err(|error| CaptureRefusal::Malformed {
            line: error.line(),
            column: error.column(),
        })?;
    let cap = usize::try_from(MEMORY_RUN_WRITE_CAP).unwrap_or(usize::MAX);
    let mut output = CaptureOutput {
        kept: Vec::new(),
        dropped: Vec::new(),
    };
    for (index, entry) in answer.entries.into_iter().enumerate() {
        let checked = if output.kept.len() >= cap {
            Err(DropReason::OverCap)
        } else {
            check_entry(entry, pack, scrubber)
        };
        match checked {
            Ok(kept) => output.kept.push(kept),
            Err(reason) => output.dropped.push((index, reason)),
        }
    }
    Ok(output)
}

/// One entry's checks, in order: kind, subject, body, evidence.
fn check_entry(
    entry: RawEntry,
    pack: &EvidencePack,
    scrubber: &dyn Scrubber,
) -> Result<CapturedEntry, DropReason> {
    let kind: MemoryKind = entry.kind.parse().map_err(|_| DropReason::BadKind)?;
    let subject = MemorySubject::new(&entry.subject, scrubber).map_err(DropReason::Subject)?;
    let body = MemoryBody::new(&entry.body, scrubber).map_err(DropReason::Body)?;
    if entry.evidence.is_empty() {
        return Err(DropReason::NoEvidence);
    }
    let citations = entry
        .evidence
        .iter()
        .map(|citation| check_citation(citation, pack, scrubber))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CapturedEntry {
        kind,
        subject,
        body,
        evidence: MemoryEvidence { citations },
    })
}

/// One citation: a known block, a long enough quote found in one of its evidence spans (never in
/// its label lines), scrubbed.
fn check_citation(
    citation: &RawCitation,
    pack: &EvidencePack,
    scrubber: &dyn Scrubber,
) -> Result<MemoryCitation, DropReason> {
    let block = pack
        .block(&citation.block)
        .ok_or(DropReason::UnknownBlock)?;
    let quote = citation.quote.trim();
    if quote.chars().count() < CAPTURE_QUOTE_MIN {
        return Err(DropReason::ShortQuote);
    }
    if !block.spans.iter().any(|span| span.contains(quote)) {
        return Err(DropReason::QuoteNotFound);
    }
    let mut value = Value::String(quote.to_owned());
    scrubber
        .scrub(&mut value)
        .map_err(|unmasked| DropReason::UnscrubbableQuote {
            rule: unmasked.rule,
        })?;
    let Value::String(quote) = value else {
        return Err(DropReason::UnscrubbableQuote {
            rule: "type_changed",
        });
    };
    Ok(MemoryCitation {
        step_id: block.step_id,
        quote,
    })
}
