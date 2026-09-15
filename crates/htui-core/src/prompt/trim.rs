//! `R-PRM-3`'s fixed priority order, read as a **keep** order, and the record it writes
//! (ANA-5 §4.4 `:791-988`, §5.1 `:1483-1554`).
//!
//! `R-PRM-3` lists "template and skills first, then item body, then prior documents, then upstream
//! summaries, then excerpts". ANA-5 settles the reading that makes it coherent: the list is a
//! keep-priority, the head of it is **protected**, and trimming walks the tail backwards. The
//! literal reading — template and skills lose tokens first — would delete the phase instructions
//! before deleting a file excerpt, and would make `R-ID-5`'s "behaviour identical on every box"
//! false the moment a skill shrank under budget pressure.
//!
//! Three properties of this module are load-bearing beyond its own tests:
//!
//! * **Re-estimation after every rung** (§4.4 step 8). Every strategy is written as "change,
//!   re-wrap, re-estimate", never as arithmetic on the previous count, so the elision marker's own
//!   tokens and the wrapper's are always included. Codex had to patch exactly that class of bug:
//!   its truncation fix "accounts for 3-line marker overhead when calculating head/tail line
//!   budgets" (<https://github.com/openai/codex/pull/6476>).
//! * **One map, and only one.** [`SectionEntry`] — the `prompt` event payload's `sections[]` — is
//!   constructed by [`TrimRecord::section_entries`] and by nothing else (ANA-5 risk 12,
//!   `:1551-1554`). `trim_record` is canonical; the payload array is its abridged projection;
//!   nothing derives the other way, so the two cannot drift.
//! * **The floor and then the drop** (§4.4 step 7). A section that reached its floor without
//!   clearing the deficit still records the strategy it used and its elision counts; only the drop
//!   rewrites the row to `{ tokens_after: 0, strategy: dropped, trimmed: true }`. "This was
//!   dropped" is the fact a reader needs, so a dropped section keeps an entry.
//!
//! The two refusals live here too, and both happen **before** any session starts: a protected set
//! that alone exceeds the target, and skills over `max_skill_tokens`. Neither degrades anything —
//! §4.4 makes the first "the one budget condition that is an error rather than a trim", and §4.2
//! refuses rather than dropping a binding because a silently dropped skill breaks `R-ID-5`.

use serde::Serialize;
use serde_json::Value;

use crate::model::link::UpstreamEntry;
use crate::prompt::estimate::TokenEstimator;
use crate::prompt::excerpt::{Excerpt, ExcerptAudit};
use crate::prompt::render::{self, Rendered, UpstreamState};
use crate::prompt::template::{Placeholder, TemplateRole};
use crate::prompt::{AssembleError, JudgeCandidate, PromptSpec, SectionName, TemplateRef};

/// §4.4's floor for `item` and `judge_task`: half the body, and never fewer than this many lines.
const ITEM_FLOOR_LINES: usize = 80;
/// The same floor as a percentage of the body's own length.
const ITEM_FLOOR_PERCENT: usize = 50;
/// §4.4's floor for a surviving input document.
const DOCUMENT_FLOOR_LINES: usize = 40;
/// The same floor as a percentage.
const DOCUMENT_FLOOR_PERCENT: usize = 25;
/// §4.4's floor for `verify_failure`: the last 200 lines carry the failure.
const VERIFY_FLOOR_LINES: usize = 200;
/// §4.6c's floor for a handoff `step_summary`.
const STEP_SUMMARY_FLOOR_LINES: usize = 20;
/// `trim_record.v`: version 1, first, so a later reader can branch (§5.1).
const RECORD_VERSION: u8 = 1;

/// How a section lost content. Closed vocabulary (§5.1 `:1545`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrimStrategy {
    /// Nothing was taken. Every protected section carries this throughout.
    None,
    /// Head and tail kept, the middle elided with one marker.
    HeadTail,
    /// The tail kept, the head elided with one marker.
    TailCut,
    /// A diff degraded to its stat.
    StatOnly,
    /// §4.3's depth ladder: summaries degraded to one-liners, depth-2 rows dropped.
    StubLadder,
    /// The section, or the members of it counted in `dropped`, were removed entirely.
    Dropped,
}

/// One row of `trim_record.sections` (§5.1 `:1505-1517`).
///
/// `tokens_before` and `tokens_after` are what the section costs **in this prompt**: a placeholder
/// placed twice by its body costs its section twice, which is why the rows sum to
/// `estimated_before` and `estimated_after` with no correction term.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// The closed name, rendered the one way [`SectionName::render`] renders it.
    pub name: SectionName,
    /// Estimated tokens at full size.
    pub tokens_before: i64,
    /// Estimated tokens as sent; `0` when the section was dropped.
    pub tokens_after: i64,
    /// Which strategy took the tokens.
    pub strategy: TrimStrategy,
    /// Whether the section lost content, including when it was dropped whole.
    pub trimmed: bool,
    /// The elision marker's first number, when there is a marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elided_lines: Option<u32>,
    /// The elision marker's second number, when there is a marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elided_bytes: Option<u64>,
    /// Upstream rows degraded to a one-liner by the ladder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stubbed: Option<u32>,
    /// Members removed: upstream rows, excerpt files, or documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u32>,
}

impl Section {
    /// A section that has not been touched yet.
    fn untouched(name: SectionName, tokens: i64) -> Self {
        Self {
            name,
            tokens_before: tokens,
            tokens_after: tokens,
            strategy: TrimStrategy::None,
            trimmed: false,
            elided_lines: None,
            elided_bytes: None,
            stubbed: None,
            dropped: None,
        }
    }
}

/// §5.2's abridged projection of one [`Section`].
///
/// Constructed by [`TrimRecord::section_entries`] and by nothing else; `prompt_digest.rs` greps the
/// tree to keep it that way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionEntry {
    /// The closed name.
    pub name: String,
    /// `tokens_after`, by the estimator named in the record.
    pub tokens: i64,
    /// Whether the section lost content.
    pub trimmed: bool,
}

/// `trim_record.template`: the row actually rendered (§5.1 `:1542`).
///
/// `role` is here because a digest alone cannot tell an auditor whether a phase template or one of
/// the two reserved names produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TemplateRecord {
    /// `prompt_template.name`.
    pub name: String,
    /// `prompt_template.version`, pinned, never `latest` (§4.7 rule 9).
    pub version: i32,
    /// Which closed placeholder set the body was parsed against.
    pub role: TemplateRole,
}

/// `run_step.trim_record`, version 1 (ANA-5 §5.1).
///
/// Byte-stable without an extra rule: `serde_json::Map` is a `BTreeMap` in this workspace, so
/// [`TrimRecord::to_value`]'s object keys serialise sorted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrimRecord {
    /// Schema version, first.
    pub v: u8,
    /// The pinned template row.
    pub template: TemplateRecord,
    /// The resolved budget.
    pub budget: i64,
    /// Which rung of the chain resolved it.
    pub budget_source: crate::prompt::settings::BudgetSource,
    /// The reserve as a fraction, the form §5.1 records.
    pub reserve: f64,
    /// `floor(budget × (1 − reserve))`.
    pub target: i64,
    /// The estimator id. Every token figure in this record is by this estimator and no other.
    pub estimator: &'static str,
    /// The sum of every section's `tokens_before`, plus the template's.
    pub estimated_before: i64,
    /// The sum of every section's `tokens_after`.
    pub estimated_after: i64,
    /// One row per section that had data, in render order, `template` first.
    pub sections: Vec<Section>,
    /// §4.5's audit, verbatim.
    pub excerpts: ExcerptAudit,
    /// Conditions that are not errors: a clamped `hops`, a skipped repo, a declared stand-in.
    pub notes: Vec<String>,
}

impl TrimRecord {
    /// The record as JSON, for `run_step.trim_record`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// **The** `sections[]` map (ANA-5 `:1551-1554`): `{name, tokens_after, trimmed}`, same order.
    #[must_use]
    pub fn section_entries(&self) -> Vec<SectionEntry> {
        self.sections
            .iter()
            .map(|section| SectionEntry {
                name: section.name.render(),
                tokens: section.tokens_after,
                trimmed: section.trimmed,
            })
            .collect()
    }
}

/// One entry in a role's trim order. A group (`Documents`, `Candidates`) is one entry because
/// §4.4 trims its members against one shared floor rather than one by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrimStep {
    /// Rank 1: budget-derived rather than budget-trimmed, so it goes first.
    Excerpts,
    /// Rank 2: §4.3's depth ladder.
    Upstream,
    /// Rank 2: full diff, then the stat, then the section.
    PreviousDiff,
    /// Rank 3: command output, tail-cut.
    VerifyFailure,
    /// Rank 3: the document group.
    Documents,
    /// Rank 4: head+tail, never dropped.
    Item,
    /// Judge rank 1: the per-candidate isolate cap.
    Candidates,
    /// Judge rank 2.
    JudgeTask,
    /// Handoff rank 1.
    DiffSoFar,
    /// Handoff rank 2.
    StepSummary,
}

/// The order sections lose tokens in, first to lose first (§4.4 `:830-851`).
///
/// The phase order is `R-PRM-3`'s list read backwards, with `verify_failure` placed before the
/// document group and the group before `item` (blueprint P-3, following §5.1's worked example
/// rather than §4.4's row text — the two disagree and the example is the one with arithmetic in
/// it). The two role orders are §4.4's own two-entry orders.
pub(crate) const fn trim_order(role: TemplateRole) -> &'static [TrimStep] {
    match role {
        TemplateRole::Phase => &[
            TrimStep::Excerpts,
            TrimStep::Upstream,
            TrimStep::PreviousDiff,
            TrimStep::VerifyFailure,
            TrimStep::Documents,
            TrimStep::Item,
        ],
        TemplateRole::Judge => &[TrimStep::Candidates, TrimStep::JudgeTask],
        TemplateRole::Handoff => &[
            TrimStep::DiffSoFar,
            TrimStep::StepSummary,
            TrimStep::Documents,
        ],
    }
}

/// What a section can be re-rendered from, and the state the ladder has put it in.
///
/// The trimmer re-renders rather than editing rendered bytes wherever the section has structure —
/// a fenced block, a set of blocks, an attribute that moves with the content — because editing the
/// bytes would mean parsing them back, and nothing in this crate parses a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    /// Content is the body verbatim, so the trimmer edits it in place.
    PlainText,
    /// `spec.documents[i]`, content edited in place.
    Document(usize),
    /// One state per canonical upstream row.
    Upstream(Vec<UpstreamState>),
    /// The surviving excerpt indices, in the ranker's own order.
    Excerpts(Vec<usize>),
    /// The previous attempt's verification output, re-rendered when tail-cut.
    VerifyFailure,
    /// A diff; `true` once it has been degraded to its stat.
    Diff(bool),
    /// `candidates[i]`; `true` once its diff has been degraded to its stat.
    Candidate(usize, bool),
    /// Nothing to trim: a protected section, or one with no ladder.
    Fixed,
}

/// One section under the trimmer's control.
#[derive(Debug, Clone)]
struct Entry {
    /// Which slot substitutes this section.
    placeholder: Placeholder,
    /// The rendered content, as it currently stands.
    rendered: Rendered,
    /// How many times the body places [`Entry::placeholder`]: a section placed twice costs twice.
    weight: i64,
    /// What it can be re-rendered from.
    source: Source,
    /// The record row, kept up to date as the ladder runs.
    record: Section,
    /// `false` once the section has been dropped whole.
    alive: bool,
}

/// The canonical inputs the assembler resolved, which the trimmer re-renders from.
///
/// Separate from [`PromptSpec`] because both of these are **re-sorted** copies: the assembler does
/// not trust its caller's order (§4.7 rules 2 and 6), so trimming from `spec.upstream` directly
/// would re-render in the caller's order and change the bytes mid-trim.
pub(crate) struct Inputs<'a> {
    /// The spec, for the sections that carry no order of their own.
    pub(crate) spec: &'a PromptSpec,
    /// `spec.upstream` in `(depth, key bytes, item_id)` order.
    pub(crate) upstream: &'a [UpstreamEntry],
    /// The judge's candidates in the order this call renders them.
    pub(crate) candidates: &'a [JudgeCandidate],
}

/// §4.4's numbered steps as one greedy linear pass.
///
/// Linear and not a priority search, deliberately: Priompt is the closest prior art and its own
/// authors disown the generality — "adding priorities to everything is sort of an anti-pattern" —
/// and its binary search over cutoffs is "not always guaranteed to produce the perfect
/// p_opt-cutoff". A linear pass over a five-entry order fixed by a requirement is exact,
/// explainable in one table, and auditable.
pub(crate) struct Trimmer<'a> {
    inputs: Inputs<'a>,
    est: TokenEstimator,
    target: i64,
    template_tokens: i64,
    entries: Vec<Entry>,
}

impl<'a> Trimmer<'a> {
    /// Takes the rendered sections in render order, already scrubbed, and estimates each one.
    pub(crate) fn new(
        inputs: Inputs<'a>,
        est: TokenEstimator,
        target: i64,
        template_tokens: i64,
        rendered: Vec<(Placeholder, Rendered, i64)>,
    ) -> Self {
        let entries = rendered
            .into_iter()
            .map(|(placeholder, rendered, weight)| {
                let tokens = est.estimate(&render::wrap(&rendered)) * weight;
                let source = Self::source_of(&inputs, &rendered.name);
                Entry {
                    record: Section::untouched(rendered.name.clone(), tokens),
                    placeholder,
                    rendered,
                    weight,
                    source,
                    alive: true,
                }
            })
            .collect();
        Self {
            inputs,
            est,
            target,
            template_tokens,
            entries,
        }
    }

    /// Which ladder a section has, decided once at construction from its name.
    fn source_of(inputs: &Inputs<'a>, name: &SectionName) -> Source {
        match name {
            SectionName::Item | SectionName::JudgeTask | SectionName::StepSummary => {
                Source::PlainText
            }
            SectionName::Documents(kind) => inputs
                .spec
                .documents
                .iter()
                .position(|doc| &doc.kind == kind)
                .map_or(Source::Fixed, Source::Document),
            SectionName::Upstream => Source::Upstream(
                inputs
                    .upstream
                    .iter()
                    .map(UpstreamState::of)
                    .collect::<Vec<_>>(),
            ),
            SectionName::Excerpts => {
                Source::Excerpts((0..inputs.spec.excerpts.files.len()).collect())
            }
            SectionName::VerifyFailure => Source::VerifyFailure,
            SectionName::PreviousDiff | SectionName::DiffSoFar => Source::Diff(false),
            SectionName::JudgeCandidate(index) => inputs
                .candidates
                .iter()
                .position(|c| c.fanout_index == *index)
                .map_or(Source::Fixed, |i| Source::Candidate(i, false)),
            SectionName::Template
            | SectionName::Box
            | SectionName::Skills
            | SectionName::CommandQueue
            | SectionName::FailureReason => Source::Fixed,
        }
    }

    /// What the whole prompt currently estimates at: the frame plus every live section.
    fn total(&self) -> i64 {
        self.template_tokens
            + self
                .entries
                .iter()
                .filter(|entry| entry.alive)
                .map(|entry| entry.record.tokens_after)
                .sum::<i64>()
    }

    /// How many tokens must still be reclaimed. `<= 0` means stop.
    fn deficit(&self) -> i64 {
        self.total() - self.target
    }

    /// The `skills` section's cost, which §4.2 caps on its own.
    fn skills_tokens(&self) -> i64 {
        self.entries
            .iter()
            .find(|entry| entry.record.name == SectionName::Skills)
            .map_or(0, |entry| entry.record.tokens_before)
    }

    /// The frame plus every section the trimmer may never touch.
    fn protected_tokens(&self) -> i64 {
        let role = self.inputs.spec.role;
        self.template_tokens
            + self
                .entries
                .iter()
                .filter(|entry| entry.record.name.is_protected(role))
                .map(|entry| entry.record.tokens_before)
                .sum::<i64>()
    }

    /// §4.4 step 4 and §4.2's cap, in the order D.1 fixes: skills first, because a step whose
    /// skills alone are over the cap is misconfigured in one place rather than two.
    ///
    /// # Errors
    ///
    /// [`AssembleError::SkillsExceedCap`] or [`AssembleError::BudgetTooSmall`]. Both happen before
    /// any session starts and before anything is digested, which is what criterion 10 asserts.
    pub(crate) fn refusals(&self) -> Result<(), AssembleError> {
        let skills = self.skills_tokens();
        let cap = self.inputs.spec.max_skill_tokens;
        if skills > cap {
            return Err(AssembleError::SkillsExceedCap {
                tokens: skills,
                cap,
            });
        }
        let needed = self.protected_tokens();
        if needed > self.target {
            return Err(AssembleError::BudgetTooSmall {
                needed,
                target: self.target,
            });
        }
        Ok(())
    }

    /// §4.4 steps 5 to 8: walk the order, stop the moment the deficit reaches zero.
    pub(crate) fn run(&mut self) {
        for step in trim_order(self.inputs.spec.role) {
            if self.deficit() <= 0 {
                return;
            }
            match step {
                TrimStep::Excerpts => self.trim_excerpts(),
                TrimStep::Upstream => self.trim_upstream(),
                TrimStep::PreviousDiff => self.trim_diff(&SectionName::PreviousDiff),
                TrimStep::DiffSoFar => self.trim_diff(&SectionName::DiffSoFar),
                TrimStep::VerifyFailure => self.trim_verify_failure(),
                TrimStep::Documents => self.trim_documents(),
                TrimStep::Item => {
                    self.head_tail_in_place(
                        &SectionName::Item,
                        ITEM_FLOOR_LINES,
                        ITEM_FLOOR_PERCENT,
                    );
                }
                TrimStep::JudgeTask => {
                    self.head_tail_in_place(
                        &SectionName::JudgeTask,
                        ITEM_FLOOR_LINES,
                        ITEM_FLOOR_PERCENT,
                    );
                }
                TrimStep::StepSummary => {
                    self.head_tail_in_place(&SectionName::StepSummary, STEP_SUMMARY_FLOOR_LINES, 0);
                }
                TrimStep::Candidates => self.trim_candidates(),
            }
        }
    }

    /// The record's rows, the surviving sections in render order, and which excerpts survived.
    pub(crate) fn finish(self) -> (Vec<Section>, Vec<(Placeholder, Rendered)>, Vec<usize>) {
        let kept = self
            .entries
            .iter()
            .find(|entry| entry.record.name == SectionName::Excerpts)
            .map_or_else(
                || (0..self.inputs.spec.excerpts.files.len()).collect(),
                |entry| match (&entry.source, entry.alive) {
                    (Source::Excerpts(kept), true) => kept.clone(),
                    _ => Vec::new(),
                },
            );
        let mut sections = Vec::with_capacity(self.entries.len());
        let mut live = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            sections.push(entry.record);
            if entry.alive {
                live.push((entry.placeholder, entry.rendered));
            }
        }
        (sections, live, kept)
    }

    /// The index of a section by name, if it was rendered and is still alive.
    fn live_index(&self, name: &SectionName) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.alive && &entry.record.name == name)
    }

    /// Re-measures one section after its content changed (§4.4 step 8).
    fn re_estimate(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.record.tokens_after =
            self.est.estimate(&render::wrap(&entry.rendered)) * entry.weight;
    }

    /// §4.4 step 7's drop: the section leaves the prompt and keeps an entry saying so.
    fn drop_section(&mut self, index: usize) {
        let entry = &mut self.entries[index];
        entry.alive = false;
        entry.record.tokens_after = 0;
        entry.record.strategy = TrimStrategy::Dropped;
        entry.record.trimmed = true;
    }

    /// Records that a strategy took content from a section.
    fn mark(&mut self, index: usize, strategy: TrimStrategy) {
        let record = &mut self.entries[index].record;
        record.strategy = strategy;
        record.trimmed = true;
    }

    /// How many tokens this section must give up, in its own unweighted terms.
    fn reclaim_for(&self, index: usize) -> i64 {
        let weight = self.entries[index].weight.max(1);
        self.deficit().div_euclid(weight) + i64::from(self.deficit().rem_euclid(weight) > 0)
    }

    // -- the per-section strategies ------------------------------------------------------------

    /// Drop whole files, highest rank number first; no file left means the section goes (§4.2).
    fn trim_excerpts(&mut self) {
        let Some(index) = self.live_index(&SectionName::Excerpts) else {
            return;
        };
        let mut removed = self.entries[index].record.dropped.unwrap_or(0);
        loop {
            let Source::Excerpts(kept) = &self.entries[index].source else {
                return;
            };
            if kept.is_empty() {
                break;
            }
            // Highest rank number is the worst file; `(repo, path)` breaks a tie, so two files the
            // ranker scored equally leave in one fixed order rather than in `Vec` order.
            let files = &self.inputs.spec.excerpts.files;
            let worst = kept
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let (a, b) = (&files[**a], &files[**b]);
                    a.rank
                        .cmp(&b.rank)
                        .then_with(|| a.repo.as_bytes().cmp(b.repo.as_bytes()))
                        .then_with(|| a.path.as_bytes().cmp(b.path.as_bytes()))
                })
                .map(|(position, _)| position)
                .unwrap_or_default();
            let Source::Excerpts(kept) = &mut self.entries[index].source else {
                return;
            };
            kept.remove(worst);
            removed += 1;
            let surviving: Vec<Excerpt> = kept
                .iter()
                .map(|file| self.inputs.spec.excerpts.files[*file].clone())
                .collect();
            let Some(rendered) = render::excerpts(&surviving) else {
                break;
            };
            self.entries[index].rendered = rendered;
            self.entries[index].record.dropped = Some(removed);
            self.mark(index, TrimStrategy::Dropped);
            self.re_estimate(index);
            if self.deficit() <= 0 {
                return;
            }
        }
        if let Source::Excerpts(kept) = &mut self.entries[index].source {
            kept.clear();
        }
        self.entries[index].record.dropped = Some(
            removed.max(u32::try_from(self.inputs.spec.excerpts.files.len()).unwrap_or(u32::MAX)),
        );
        self.drop_section(index);
    }

    /// §4.3's depth ladder, one rung at a time with re-estimation after each (§4.4's `upstream`).
    fn trim_upstream(&mut self) {
        let Some(index) = self.live_index(&SectionName::Upstream) else {
            return;
        };
        let rows = self.inputs.upstream.len();
        let mut stubbed = 0u32;
        let mut dropped = 0u32;
        // (a) depth-2 summaries degrade, (b) depth-2 rows go, (c) depth-1 summaries degrade.
        for rung in 0..3u8 {
            for row in 0..rows {
                if self.deficit() <= 0 {
                    return;
                }
                let entry = &self.inputs.upstream[row];
                let Source::Upstream(states) = &mut self.entries[index].source else {
                    return;
                };
                let state = states[row];
                let next = match rung {
                    0 if entry.depth >= 2 && state == UpstreamState::Summary => {
                        stubbed += 1;
                        UpstreamState::Pending
                    }
                    1 if entry.depth >= 2 && state != UpstreamState::Dropped => {
                        dropped += 1;
                        UpstreamState::Dropped
                    }
                    2 if entry.depth <= 1 && state == UpstreamState::Summary => {
                        stubbed += 1;
                        UpstreamState::Pending
                    }
                    _ => continue,
                };
                states[row] = next;
                let states = states.clone();
                let Some(rendered) = render::upstream(self.inputs.upstream, &states) else {
                    break;
                };
                self.entries[index].rendered = rendered;
                self.entries[index].record.stubbed = (stubbed > 0).then_some(stubbed);
                self.entries[index].record.dropped = (dropped > 0).then_some(dropped);
                self.mark(index, TrimStrategy::StubLadder);
                self.re_estimate(index);
            }
        }
        // The floor is every surviving row as a one-liner; still over, and the section goes.
        if self.deficit() > 0 {
            self.drop_section(index);
        }
    }

    /// Full diff, then the stat alone, then the section (§4.2's `previous_diff` row).
    fn trim_diff(&mut self, name: &SectionName) {
        let Some(index) = self.live_index(name) else {
            return;
        };
        let block = match name {
            SectionName::PreviousDiff => self.inputs.spec.previous_diff.as_ref(),
            _ => self
                .inputs
                .spec
                .handoff
                .as_ref()
                .and_then(|handoff| handoff.diff_so_far.as_ref()),
        };
        if let (Some(block), Source::Diff(false)) = (block, &self.entries[index].source) {
            self.entries[index].rendered = render::diff(name.clone(), block, true);
            self.entries[index].source = Source::Diff(true);
            self.mark(index, TrimStrategy::StatOnly);
            self.re_estimate(index);
        }
        if self.deficit() > 0 {
            self.drop_section(index);
        }
    }

    /// Tail-cut to 200 lines, keeping the tail, then the section (§4.4's `verify_failure` row).
    fn trim_verify_failure(&mut self) {
        let Some(index) = self.live_index(&SectionName::VerifyFailure) else {
            return;
        };
        if let Some(failure) = self.inputs.spec.verify_failure.as_ref() {
            let reclaim = self.reclaim_for(index);
            let normalised = render::normalise_newlines(&failure.output);
            if let Some((cut, lines, bytes)) = tail_cut(
                normalised.trim_end_matches('\n'),
                VERIFY_FLOOR_LINES,
                reclaim,
                self.est,
            ) {
                let mut trimmed = failure.clone();
                trimmed.output = cut;
                self.entries[index].rendered = render::verify_failure(&trimmed);
                self.entries[index].record.elided_lines = Some(lines);
                self.entries[index].record.elided_bytes = Some(bytes);
                self.mark(index, TrimStrategy::TailCut);
                self.re_estimate(index);
            }
        }
        if self.deficit() > 0 {
            self.drop_section(index);
        }
    }

    /// Head+tail every document to its floor in `input_kinds` order, then drop from the end of the
    /// order — never index 0, which §4.4 makes a stage-3 failure rather than a trim.
    fn trim_documents(&mut self) {
        let documents: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry.source, Source::Document(_)) && entry.alive)
            .map(|(index, _)| index)
            .collect();
        for index in &documents {
            if self.deficit() <= 0 {
                return;
            }
            self.head_tail_at(*index, DOCUMENT_FLOOR_LINES, DOCUMENT_FLOOR_PERCENT);
        }
        for index in documents.iter().rev() {
            if self.deficit() <= 0 {
                return;
            }
            if *index == documents[0] {
                // The first resolved kind is what the phase is working from.
                return;
            }
            self.drop_section(*index);
        }
    }

    /// §4.6a's isolate cap: each candidate gets an equal share of what the frame and the task left.
    fn trim_candidates(&mut self) {
        let candidates: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry.source, Source::Candidate(_, _)) && entry.alive)
            .map(|(index, _)| index)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let task = self
            .entries
            .iter()
            .find(|entry| entry.record.name == SectionName::JudgeTask)
            .map_or(0, |entry| entry.record.tokens_after);
        let share = (self.target - self.template_tokens - task).max(0)
            / i64::try_from(candidates.len()).unwrap_or(1);
        for index in candidates {
            if self.entries[index].record.tokens_after <= share {
                continue;
            }
            let Source::Candidate(candidate, false) = self.entries[index].source else {
                continue;
            };
            self.entries[index].rendered =
                render::judge_candidate(&self.inputs.candidates[candidate], true);
            self.entries[index].source = Source::Candidate(candidate, true);
            self.mark(index, TrimStrategy::StatOnly);
            self.re_estimate(index);
            if self.entries[index].record.tokens_after > share {
                // MOD-4 reads a dropped candidate as "escalate to human selection" (`:891`), which
                // is a better outcome than judging a candidate the judge cannot see.
                self.drop_section(index);
            }
        }
    }

    /// Head+tail a section whose content is its body verbatim, by name.
    fn head_tail_in_place(&mut self, name: &SectionName, floor_lines: usize, percent: usize) {
        if let Some(index) = self.live_index(name) {
            self.head_tail_at(index, floor_lines, percent);
        }
    }

    /// Head+tail the section at `index`, recording the floor state even when it did not clear the
    /// deficit (§4.4 step 7).
    fn head_tail_at(&mut self, index: usize, floor_lines: usize, percent: usize) {
        let reclaim = self.reclaim_for(index);
        let content = self.entries[index].rendered.content.clone();
        let floor = floor_for(&content, floor_lines, percent);
        let Some((cut, lines, bytes)) = head_tail(&content, floor, reclaim, self.est) else {
            return;
        };
        self.entries[index].rendered.content = cut;
        self.entries[index].record.elided_lines = Some(lines);
        self.entries[index].record.elided_bytes = Some(bytes);
        self.mark(index, TrimStrategy::HeadTail);
        self.re_estimate(index);
    }
}

/// The floor in lines: `percent` of the body, never fewer than `floor_lines`. `percent == 0` is a
/// flat floor.
fn floor_for(content: &str, floor_lines: usize, percent: usize) -> usize {
    let lines = content.split('\n').count();
    floor_lines.max(lines.saturating_mul(percent) / 100)
}

/// Keeps `⌈k/2⌉` lines of head and `⌊k/2⌋` of tail with one marker between, for the largest `k` in
/// `[floor_lines, n)` that reclaims `reclaim` tokens — or the floor itself when nothing does.
///
/// Head **and** tail rather than head alone, on the clearest published before-and-after in the
/// field: Codex's own PR describes the behaviour it replaced as "First 128 lines kept entirely,
/// regardless of size ... TAIL IS COMPLETELY LOST - error messages invisible to model"
/// (<https://github.com/openai/codex/pull/6476>).
///
/// The search is binary because the cost is monotone in `k`: a 4 000-line document costs about
/// twelve estimates rather than 4 000. Returns `None` when the content is already at or under its
/// floor, or when the marker would cost more than the lines it replaces.
pub(crate) fn head_tail(
    content: &str,
    floor_lines: usize,
    reclaim: i64,
    est: TokenEstimator,
) -> Option<(String, u32, u64)> {
    window(content, floor_lines, reclaim, est, false)
}

/// The same search keeping the **tail**, with the marker first: command output puts the failure at
/// the end (§4.4's `verify_failure` row).
pub(crate) fn tail_cut(
    content: &str,
    floor_lines: usize,
    reclaim: i64,
    est: TokenEstimator,
) -> Option<(String, u32, u64)> {
    window(content, floor_lines, reclaim, est, true)
}

/// The shared search. `tail_only` picks between the two shapes.
fn window(
    content: &str,
    floor_lines: usize,
    reclaim: i64,
    est: TokenEstimator,
    tail_only: bool,
) -> Option<(String, u32, u64)> {
    let lines: Vec<&str> = content.split('\n').collect();
    let total = lines.len();
    if total <= floor_lines || floor_lines == 0 && total < 2 {
        return None;
    }
    let full = est.estimate(content);
    let budget = full - reclaim.max(0);
    let mut best: Option<usize> = None;
    let (mut low, mut high) = (floor_lines, total - 1);
    while low <= high {
        let mid = low + (high - low) / 2;
        if est.estimate(&rebuild(&lines, mid, tail_only)) <= budget {
            best = Some(mid);
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    let kept = best.unwrap_or(floor_lines);
    let text = rebuild(&lines, kept, tail_only);
    if est.estimate(&text) >= full {
        // The marker costs more than the lines it replaced: leave the content alone rather than
        // record a trim that made the prompt bigger.
        return None;
    }
    let elided: Vec<&&str> = elided_lines(&lines, kept, tail_only);
    let bytes: u64 = elided
        .iter()
        .map(|line| line.len() as u64 + 1)
        .sum::<u64>()
        .saturating_sub(u64::from(kept == 0));
    Some((text, u32::try_from(elided.len()).unwrap_or(u32::MAX), bytes))
}

/// The kept lines with the marker in place, for a given `k`.
fn rebuild(lines: &[&str], kept: usize, tail_only: bool) -> String {
    let total = lines.len();
    let dropped = total.saturating_sub(kept);
    let bytes: u64 = if tail_only {
        lines[..dropped].iter().map(|l| l.len() as u64 + 1).sum()
    } else {
        let head = kept.div_ceil(2);
        lines[head..total - (kept - head)]
            .iter()
            .map(|l| l.len() as u64 + 1)
            .sum()
    };
    let marker = render::elision_marker(u32::try_from(dropped).unwrap_or(u32::MAX), bytes);
    if tail_only {
        let mut out = marker;
        out.push('\n');
        out.push_str(&lines[dropped..].join("\n"));
        out
    } else {
        let head = kept.div_ceil(2);
        let tail = kept - head;
        let mut out = lines[..head].join("\n");
        out.push('\n');
        out.push_str(&marker);
        if tail > 0 {
            out.push('\n');
            out.push_str(&lines[total - tail..].join("\n"));
        }
        out
    }
}

/// Which lines `rebuild` left out, for the record's two numbers.
fn elided_lines<'a>(lines: &'a [&'a str], kept: usize, tail_only: bool) -> Vec<&'a &'a str> {
    let total = lines.len();
    let dropped = total.saturating_sub(kept);
    if tail_only {
        lines[..dropped].iter().collect()
    } else {
        let head = kept.div_ceil(2);
        lines[head..total - (kept - head)].iter().collect()
    }
}

/// Builds the record once the trim has run. One function, so `sections[0]` is `template` in every
/// role and every path (P-9).
pub(crate) fn record(
    spec: &PromptSpec,
    template: &TemplateRef,
    template_tokens: i64,
    sections: Vec<Section>,
    excerpts: ExcerptAudit,
    notes: Vec<String>,
) -> TrimRecord {
    let mut rows = Vec::with_capacity(sections.len() + 1);
    rows.push(Section::untouched(SectionName::Template, template_tokens));
    rows.extend(sections);
    TrimRecord {
        v: RECORD_VERSION,
        template: TemplateRecord {
            name: template.name.clone(),
            version: template.version,
            role: spec.role,
        },
        budget: spec.budget.tokens,
        budget_source: spec.budget.source,
        reserve: spec.budget.reserve(),
        target: spec.budget.target(),
        estimator: spec.estimator.id,
        estimated_before: rows.iter().map(|row| row.tokens_before).sum(),
        estimated_after: rows.iter().map(|row| row.tokens_after).sum(),
        sections: rows,
        excerpts,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(lines: usize) -> String {
        (0..lines)
            .map(|n| format!("line {n}: the recorder buffers rows and flushes on the boundary."))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_three_orders_are_the_documented_ones() {
        assert_eq!(
            trim_order(TemplateRole::Phase),
            &[
                TrimStep::Excerpts,
                TrimStep::Upstream,
                TrimStep::PreviousDiff,
                TrimStep::VerifyFailure,
                TrimStep::Documents,
                TrimStep::Item,
            ]
        );
        assert_eq!(
            trim_order(TemplateRole::Judge),
            &[TrimStep::Candidates, TrimStep::JudgeTask]
        );
        assert_eq!(
            trim_order(TemplateRole::Handoff),
            &[
                TrimStep::DiffSoFar,
                TrimStep::StepSummary,
                TrimStep::Documents,
            ]
        );
    }

    #[test]
    fn head_tail_keeps_both_ends_and_counts_what_it_took() {
        let content = body(400);
        let est = TokenEstimator::DEFAULT;
        let (cut, lines, bytes) = head_tail(&content, 40, 2_000, est).expect("400 lines can cut");
        assert!(cut.starts_with("line 0:"), "the head survives");
        assert!(cut.ends_with("line 399: the recorder buffers rows and flushes on the boundary."));
        assert!(cut.contains(&render::elision_marker(lines, bytes)));
        assert!(est.estimate(&cut) <= est.estimate(&content) - 2_000);
        // The marker's two numbers are the two the record stores: one string, never a parse.
        let elided: Vec<&str> = content
            .split('\n')
            .filter(|line| !cut.contains(line))
            .collect();
        assert_eq!(elided.len(), lines as usize);
        assert_eq!(
            bytes,
            elided.iter().map(|l| l.len() as u64 + 1).sum::<u64>()
        );
    }

    #[test]
    fn head_tail_stops_at_its_floor() {
        let content = body(400);
        let est = TokenEstimator::DEFAULT;
        // Ask for more than the section can ever give: it returns the floor, not `None`.
        let (cut, lines, _) = head_tail(&content, 100, 1_000_000, est).expect("the floor cut");
        assert_eq!(lines, 300, "400 lines down to the 100-line floor");
        assert_eq!(cut.split('\n').count(), 101, "100 lines plus one marker");
        // And a section already at or under its floor is left alone rather than marked.
        assert!(head_tail(&body(40), 40, 10, est).is_none());
        assert!(head_tail("", 0, 10, est).is_none());
    }

    #[test]
    fn tail_cut_keeps_the_tail_and_puts_the_marker_first() {
        let content = body(400);
        let est = TokenEstimator::DEFAULT;
        let (cut, lines, _) = tail_cut(&content, 200, 1_000_000, est).expect("the floor cut");
        assert_eq!(lines, 200);
        assert!(cut.starts_with("[... htui elided 200 lines /"));
        assert!(cut.ends_with("line 399: the recorder buffers rows and flushes on the boundary."));
        assert!(!cut.contains("line 0:"), "the head is what went");
    }

    #[test]
    fn the_floor_is_a_percentage_with_a_minimum() {
        assert_eq!(floor_for(&body(400), 40, 25), 100);
        assert_eq!(floor_for(&body(40), 40, 25), 40, "the minimum wins");
        assert_eq!(floor_for(&body(400), 20, 0), 20, "a flat floor");
    }
}
