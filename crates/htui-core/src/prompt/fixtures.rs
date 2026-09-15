//! Deterministic [`PromptSpec`] corpora for the golden prompts and the digest tests
//! (blueprint B.10; `test-support`).
//!
//! One constructor per case, every string a literal: no clock, no `Uuid::new_v4`, no store read.
//! That is what lets a golden `.snap` be a contract rather than a recording — a fixture that
//! reached for the environment would make an accepted snapshot a statement about the box that
//! accepted it, which is precisely the property ANA-5 invariant 2 exists to deny.
//!
//! T64 landed the four shapes the section renders are proved against. T65 adds the three budget
//! cases — [`phase_oversize`], [`phase_protected_too_big`], [`phase_skills_over_cap`] — and
//! [`demo_trim_record`], none of which could exist before `assemble()` did.

use uuid::Uuid;

use crate::model::OsFamily;
use crate::model::box_::BoxProfile;
use crate::model::ids::{ItemId, SkillId};
use crate::model::item::Status;
use crate::model::link::UpstreamEntry;
use crate::model::skill::BoundSkill;
use crate::prompt::excerpt::{
    Excerpt, ExcerptAudit, ExcerptCaps, ExcerptReason, ExcerptSet, FileRecord, RootRecord,
    RootSource,
};
use crate::prompt::settings::{Budget, BudgetSource};
use crate::prompt::{
    DiffBlock, HandoffInputs, InputDocument, JudgeCandidate, JudgeInputs, PromptSpec, StepSummary,
    TemplateRef, TemplateRole, TokenEstimator, VerifyFailure,
};

/// The §5.3 default budget at the §5.3 default reserve, sourced from the project rung so the
/// fixture exercises a rung that is neither the top nor the compiled-in floor.
fn demo_budget() -> Budget {
    Budget {
        tokens: 120_000,
        source: BudgetSource::Project,
        reserve_bp: 1_000,
    }
}

/// The §4.2 box projection of `:516-526`, verbatim, so the golden prompt and the document agree.
fn demo_box() -> BoxProfile {
    BoxProfile {
        hostname: "dev-win-01".to_owned(),
        os_family: OsFamily::Windows,
        os_version: "10.0.26200".to_owned(),
        arch: "x86_64".to_owned(),
        cpu: "AMD Ryzen 9 7950X, 32 threads".to_owned(),
        ram_mb: Some(65_536),
        gpu_vendor: Some("nvidia".to_owned()),
        htui_version: "0.4.1".to_owned(),
        tools: vec![
            ("cargo".to_owned(), "1.98.1".to_owned()),
            ("git".to_owned(), "2.47.0".to_owned()),
            ("node".to_owned(), "22.12.0".to_owned()),
            ("python".to_owned(), "3.13.1".to_owned()),
            ("rustc".to_owned(), "1.98.1".to_owned()),
        ],
        more_tools: 0,
        quirks: "MSVC toolchain only\nno WSL\nlong paths enabled.".to_owned(),
    }
}

/// One upstream entry with a fixed id, so the canonical sort is reproducible.
fn upstream_entry(
    id: u128,
    key: &str,
    title: &str,
    status: Status,
    depth: u8,
    in_scope: bool,
    summary: Option<&str>,
) -> UpstreamEntry {
    UpstreamEntry {
        item_id: ItemId::from_uuid(Uuid::from_u128(id)),
        qualified_key: key.to_owned(),
        title: title.to_owned(),
        status,
        depth,
        in_scope,
        summary: summary.map(str::to_owned),
    }
}

/// The §5.3 caps, so the audit half of the fixture matches the shipped defaults.
fn demo_caps() -> ExcerptCaps {
    ExcerptCaps {
        max_files: 12,
        file_line_cap: 400,
        head_lines: 200,
        max_file_bytes: 524_288,
    }
}

/// §5.1's shape: an `implement` step on attempt 2, with two documents, a previous diff, one
/// upstream summary at depth 1 and two one-liners, one skill and two excerpts.
///
/// The one section §5.1's example does not carry is `verify_failure` — the previous attempt
/// produced no verification output — and this fixture carries one anyway, because criterion 4's
/// opposite case (`phase_all_empty`) needs a sibling in which every renderable section is present.
#[must_use]
pub fn phase_implement_attempt2() -> PromptSpec {
    let excerpts = vec![
        Excerpt {
            repo: "htui".to_owned(),
            path: "crates/htui-core/src/prompt/mod.rs".to_owned(),
            first_line: 1,
            last_line: 4,
            truncated: false,
            elided_lines: 0,
            elided_bytes: 0,
            rank: 1,
            weight: 100,
            reason: ExcerptReason::TouchedPath,
            provider: None,
            content: "//! Prompt assembly (ANA-5).\n\npub mod render;\npub mod trim;\n".to_owned(),
        },
        Excerpt {
            repo: "htui".to_owned(),
            path: "crates/htui-core/src/store/traits.rs".to_owned(),
            first_line: 40,
            last_line: 43,
            truncated: true,
            elided_lines: 412,
            elided_bytes: 18_903,
            rank: 2,
            weight: 60,
            reason: ExcerptReason::Mentioned,
            provider: None,
            content:
                "pub trait ReadStore: Send + Sync {\n    async fn item(&self) -> Result<()>;\n}\n"
                    .to_owned(),
        },
    ];
    let audit = ExcerptAudit {
        provider_set: vec!["builtin@1".to_owned()],
        roots: vec![RootRecord {
            repo: "htui".to_owned(),
            source: RootSource::RunStepTree,
            scan_truncated: false,
        }],
        considered: 143,
        selected: 2,
        caps: demo_caps(),
        // §5.1's example row verbatim, `"5f0a..."` included. It is a **shape**, not a hash, and
        // nothing reads it: `assemble()` rebuilds `files[]` from the surviving excerpts and hashes
        // each one's rendered block itself (plan F-39), so a fixture cannot pin a wrong digest.
        files: vec![FileRecord {
            repo: "htui".to_owned(),
            path: "crates/htui-core/src/prompt/mod.rs".to_owned(),
            lines: "1-4".to_owned(),
            rank: 1,
            weight: 100,
            reason: ExcerptReason::TouchedPath,
            truncated: false,
            bytes: 61,
            sha256: "5f0a".to_owned(),
        }],
    };

    PromptSpec {
        role: TemplateRole::Phase,
        template: TemplateRef {
            name: "implement".to_owned(),
            version: 3,
        },
        body: crate::prompt::body_of("implement")
            .expect("`implement` is one of the ten default bodies")
            .to_owned(),
        item_key: "htui:MOD-2".to_owned(),
        item_title: "Agent driver and chat".to_owned(),
        item_kind: "feature".to_owned(),
        item_body: "Drive one agent session per run step, and record every event.\n\nThe seam is `AgentDriver`.\n"
            .to_owned(),
        phase: "implement".to_owned(),
        output_kind: Some("impl".to_owned()),
        attempt: 2,
        documents: vec![
            InputDocument {
                kind: "plan".to_owned(),
                version: 3,
                body: "1. Land the driver seam.\n2. Land the recorder.\n".to_owned(),
            },
            InputDocument {
                kind: "review".to_owned(),
                version: 1,
                body: "---\nverdict: request-changes\n---\n\nThe recorder never flushes.\n"
                    .to_owned(),
            },
        ],
        upstream: vec![
            upstream_entry(
                0x9,
                "htui:ANA-9",
                "Postgres schema",
                Status::Closed,
                1,
                true,
                Some("The schema is one database, six domains, and no soft deletes outside `item`.\n"),
            ),
            upstream_entry(
                0x7,
                "htui:MOD-7",
                "Box registry and capabilities",
                Status::InProgress,
                1,
                true,
                None,
            ),
            upstream_entry(
                0x1,
                "auth-service:MOD-1",
                "Token rotation",
                Status::Done,
                2,
                false,
                None,
            ),
        ],
        box_profile: demo_box(),
        skills: vec![BoundSkill {
            skill_id: SkillId::from_uuid(Uuid::from_u128(0x5111)),
            name: "rust-style".to_owned(),
            version: 2,
            position: 0,
            body: "Prefer `expect` with a reason over `unwrap`.\nNo `unsafe`.\n".to_owned(),
        }],
        excerpts: ExcerptSet {
            files: excerpts,
            audit,
            notes: Vec::new(),
        },
        command_queue: true,
        verify_failure: Some(VerifyFailure {
            exit_code: 101,
            output: "running 3 tests\ntest recorder::flushes ... FAILED\n".to_owned(),
        }),
        previous_diff: Some(DiffBlock {
            range: "abc1234..def5678".to_owned(),
            stat: " 2 files changed, 41 insertions(+), 8 deletions(-)".to_owned(),
            diff: "--- a/record.rs\n+++ b/record.rs\n@@ -1,2 +1,3 @@\n+// flush\n".to_owned(),
        }),
        judge: None,
        handoff: None,
        budget: demo_budget(),
        max_skill_tokens: 20_000,
        estimator: TokenEstimator::DEFAULT,
        notes: Vec::new(),
    }
}

/// Criterion 4: every optional section absent, so the assembled prompt must carry no empty
/// wrapper, no doubled blank line and no `sections[]` entry for anything that never had data.
#[must_use]
pub fn phase_all_empty() -> PromptSpec {
    PromptSpec {
        role: TemplateRole::Phase,
        template: TemplateRef {
            name: "prd".to_owned(),
            version: 1,
        },
        body: crate::prompt::body_of("prd")
            .expect("`prd` is one of the ten default bodies")
            .to_owned(),
        item_key: "htui:FEAT-3".to_owned(),
        item_title: "A bare item".to_owned(),
        item_kind: "feature".to_owned(),
        item_body: String::new(),
        phase: "prd".to_owned(),
        output_kind: None,
        attempt: 1,
        documents: Vec::new(),
        upstream: Vec::new(),
        box_profile: BoxProfile {
            hostname: "ci-linux-01".to_owned(),
            os_family: OsFamily::Linux,
            os_version: "6.8.0".to_owned(),
            arch: "x86_64".to_owned(),
            cpu: String::new(),
            ram_mb: None,
            gpu_vendor: None,
            htui_version: "0.4.1".to_owned(),
            tools: Vec::new(),
            more_tools: 0,
            quirks: String::new(),
        },
        skills: Vec::new(),
        excerpts: ExcerptSet::default(),
        command_queue: false,
        verify_failure: None,
        previous_diff: None,
        judge: None,
        handoff: None,
        budget: Budget {
            tokens: 120_000,
            source: BudgetSource::AppSettingDefault,
            reserve_bp: 1_000,
        },
        max_skill_tokens: 20_000,
        estimator: TokenEstimator::DEFAULT,
        notes: vec!["no readable repo root on this box".to_owned()],
    }
}

/// Criterion 17: three candidates, the middle one failing verification, the last one carrying a
/// diff with no document.
#[must_use]
pub fn judge_three_candidates() -> PromptSpec {
    let candidates = vec![
        JudgeCandidate {
            fanout_index: 0,
            verify: Some(true),
            exit_code: Some(0),
            document: Some(InputDocument {
                kind: "impl".to_owned(),
                version: 1,
                body: "Added the flush on turn end.\n".to_owned(),
            }),
            diff: Some(DiffBlock {
                range: "aaa1111..bbb2222".to_owned(),
                stat: " 1 file changed, 9 insertions(+)".to_owned(),
                diff: "--- a/record.rs\n+++ b/record.rs\n+self.flush();\n".to_owned(),
            }),
            verification_tail: Some("test result: ok. 41 passed; 0 failed\n".to_owned()),
        },
        JudgeCandidate {
            fanout_index: 1,
            verify: Some(false),
            exit_code: Some(101),
            document: Some(InputDocument {
                kind: "impl".to_owned(),
                version: 1,
                body: "Flushed from the drop handler.\n".to_owned(),
            }),
            diff: Some(DiffBlock {
                range: "aaa1111..ccc3333".to_owned(),
                stat: " 1 file changed, 22 insertions(+), 1 deletion(-)".to_owned(),
                diff: "--- a/record.rs\n+++ b/record.rs\n+impl Drop for Recorder {}\n".to_owned(),
            }),
            verification_tail: Some(
                "test recorder::flushes ... FAILED\ntest result: FAILED. 40 passed; 1 failed\n"
                    .to_owned(),
            ),
        },
        JudgeCandidate {
            fanout_index: 2,
            verify: None,
            exit_code: None,
            document: None,
            diff: Some(DiffBlock {
                range: "aaa1111..ddd4444".to_owned(),
                stat: " 3 files changed, 140 insertions(+), 6 deletions(-)".to_owned(),
                diff: "--- a/record.rs\n+++ b/record.rs\n+// a rewrite\n".to_owned(),
            }),
            verification_tail: None,
        },
    ];
    PromptSpec {
        role: TemplateRole::Judge,
        template: TemplateRef {
            name: "judge".to_owned(),
            version: 1,
        },
        body: crate::prompt::body_of("judge")
            .expect("`judge` is one of the two reserved bodies")
            .to_owned(),
        item_key: "htui:MOD-2".to_owned(),
        item_title: "Agent driver and chat".to_owned(),
        item_kind: "feature".to_owned(),
        item_body: String::new(),
        phase: "implement".to_owned(),
        output_kind: Some("impl".to_owned()),
        attempt: 1,
        documents: Vec::new(),
        upstream: Vec::new(),
        box_profile: demo_box(),
        skills: Vec::new(),
        excerpts: ExcerptSet::default(),
        command_queue: false,
        verify_failure: None,
        previous_diff: None,
        judge: Some(JudgeInputs {
            task: "You are running the `implement` phase for htui:MOD-2, attempt 1.\n".to_owned(),
            candidates,
            reverse: false,
        }),
        handoff: None,
        budget: demo_budget(),
        max_skill_tokens: 20_000,
        estimator: TokenEstimator::DEFAULT,
        notes: Vec::new(),
    }
}

/// Criterion 18's assembler half: a handoff after a step that edited two files and hit one error.
#[must_use]
pub fn handoff_basic() -> PromptSpec {
    PromptSpec {
        role: TemplateRole::Handoff,
        template: TemplateRef {
            name: "handoff".to_owned(),
            version: 1,
        },
        body: crate::prompt::body_of("handoff")
            .expect("`handoff` is one of the two reserved bodies")
            .to_owned(),
        item_key: "htui:MOD-2".to_owned(),
        item_title: "Agent driver and chat".to_owned(),
        item_kind: "feature".to_owned(),
        item_body: String::new(),
        phase: "implement".to_owned(),
        output_kind: Some("impl".to_owned()),
        attempt: 1,
        documents: vec![InputDocument {
            kind: "plan".to_owned(),
            version: 3,
            body: "1. Land the driver seam.\n2. Land the recorder.\n".to_owned(),
        }],
        upstream: Vec::new(),
        box_profile: demo_box(),
        skills: Vec::new(),
        excerpts: ExcerptSet::default(),
        command_queue: false,
        verify_failure: None,
        previous_diff: None,
        judge: None,
        handoff: Some(HandoffInputs {
            step_summary: StepSummary {
                turns: 3,
                events: 217,
                tool_calls: vec![
                    ("read".to_owned(), 6),
                    ("edit".to_owned(), 9),
                    ("execute".to_owned(), 6),
                ],
                files_edited: vec![
                    "crates/htui-core/src/prompt/mod.rs".to_owned(),
                    "crates/htui-core/src/prompt/trim.rs".to_owned(),
                ],
                errors: vec!["`command_failed` — `cargo test` exited 101 at turn 2".to_owned()],
                last_assistant_tail: "I cannot get the borrow checker past the trim loop.\n"
                    .to_owned(),
            },
            diff_so_far: Some(DiffBlock {
                range: "abc1234..HEAD".to_owned(),
                stat: " 2 files changed, 210 insertions(+)".to_owned(),
                diff: "--- a/trim.rs\n+++ b/trim.rs\n+fn run() {}\n".to_owned(),
            }),
            failure_reason: "The step exhausted its turn budget without a passing build."
                .to_owned(),
        }),
        budget: demo_budget(),
        max_skill_tokens: 20_000,
        estimator: TokenEstimator::DEFAULT,
        notes: Vec::new(),
    }
}

/// `lines` numbered lines of the same fixed sentence, LF, with a trailing LF.
///
/// Deterministic and parameterised rather than one 85 000-character literal: the oversize fixture
/// needs section sizes that stand in a stated ratio to the budget, and a literal would hide that
/// ratio in a wall of text nobody re-reads. Nothing here reads the environment, so the bytes are
/// the same on every box, which is all a golden fixture owes.
fn prose_block(topic: &str, lines: usize) -> String {
    let mut out = String::with_capacity(lines * 72);
    for n in 0..lines {
        out.push_str(&format!(
            "{topic} line {n}: the recorder buffers rows and flushes them on the turn boundary.\n"
        ));
    }
    out
}

/// `lines` of unified-diff body, which the estimator bills at the code rate inside its fence.
fn diff_block(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 72);
    out.push_str("--- a/crates/htui-agent/src/record.rs\n+++ b/crates/htui-agent/src/record.rs\n");
    for n in 0..lines {
        out.push_str(&format!(
            "+    self.rows.push(PendingRow::new({n})); // flush on the turn boundary\n"
        ));
    }
    out
}

/// Criterion 9's fixture: an `implement` step half again over its budget.
///
/// Sized so the deficit is cleared **inside** the document group, which is the interesting case:
/// §4.4's four lower-ranked sections are exhausted in order — `excerpts`, `upstream`,
/// `previous_diff`, `verify_failure` — and `documents:plan` then gives up exactly the residual, so
/// `documents:review` and `item` are never reached and the four protected sections carry
/// `strategy: "none"` throughout. A fixture that cleared at the first rung would prove none of that.
///
/// "Exhausted" is §4.4 step 7 as written: a section reaching its floor without clearing the deficit
/// **moves to the drop**, and the pass moves on only after it. The rungs a section passes through
/// on the way are what `trim_record` records — the upstream row here keeps its `stubbed` and
/// `dropped` counters even though the section itself went — and the intermediate states are proved
/// by the three rung tests in `prompt_digest.rs`, which clear the deficit inside one ladder.
///
/// The budget is 40 000 rather than §5.3's 120 000 so the corpus stays a few hundred kilobytes;
/// the ratio to `target` is what the trim reads, never the absolute number.
#[must_use]
pub fn phase_oversize() -> PromptSpec {
    let mut spec = phase_implement_attempt2();
    spec.budget = Budget {
        tokens: 40_000,
        source: BudgetSource::Project,
        reserve_bp: 1_000,
    };

    spec.item_body = prose_block("item", 110);
    spec.documents = vec![
        InputDocument {
            kind: "plan".to_owned(),
            version: 3,
            body: prose_block("plan", 950),
        },
        InputDocument {
            kind: "review".to_owned(),
            version: 1,
            body: prose_block("review", 72),
        },
    ];
    // Ten lines, well under the 200-line tail-cut floor: it cannot reclaim anything, so it reaches
    // its floor immediately and the ladder moves to the drop. That is §4.4 step 7 as written.
    spec.verify_failure = Some(VerifyFailure {
        exit_code: 101,
        output: prose_block("test recorder::flushes", 10),
    });
    spec.previous_diff = Some(DiffBlock {
        range: "abc1234..def5678".to_owned(),
        stat: " 2 files changed, 210 insertions(+), 8 deletions(-)".to_owned(),
        diff: diff_block(210),
    });
    spec.upstream = vec![
        upstream_entry(
            0x9,
            "htui:ANA-9",
            "Postgres schema",
            Status::Closed,
            1,
            true,
            Some(&prose_block("ana-9 summary", 60)),
        ),
        upstream_entry(
            0x7,
            "htui:MOD-7",
            "Box registry and capabilities",
            Status::InProgress,
            1,
            true,
            Some(&prose_block("mod-7 summary", 40)),
        ),
        upstream_entry(
            0x4,
            "htui:MOD-4",
            "Orchestrator",
            Status::Queued,
            2,
            true,
            Some(&prose_block("mod-4 summary", 40)),
        ),
        upstream_entry(
            0x1,
            "auth-service:MOD-1",
            "Token rotation",
            Status::Done,
            2,
            false,
            None,
        ),
    ];
    for (index, excerpt) in spec.excerpts.files.iter_mut().enumerate() {
        excerpt.first_line = 1;
        excerpt.content = diff_block(170 + index * 10);
        excerpt.last_line = u32::try_from(excerpt.content.lines().count()).unwrap_or(u32::MAX);
    }
    spec
}

/// Criterion 10's first half: a protected set that alone exceeds `target`.
///
/// The budget is a plausible typo — 1 000 tokens, an order of magnitude below anything a model
/// takes — rather than an impossible one, because the refusal exists for a misconfigured row and
/// its message is what a maintainer acts on.
#[must_use]
pub fn phase_protected_too_big() -> PromptSpec {
    let mut spec = phase_implement_attempt2();
    spec.budget = Budget {
        tokens: 1_000,
        source: BudgetSource::Project,
        reserve_bp: 1_000,
    };
    spec.skills = vec![BoundSkill {
        skill_id: SkillId::from_uuid(Uuid::from_u128(0x5111)),
        name: "rust-style".to_owned(),
        version: 2,
        position: 0,
        body: prose_block("rust-style", 60),
    }];
    spec
}

/// Criterion 10's second half: skills over `max_skill_tokens`.
///
/// The budget stays ample, so the only thing wrong with this spec is the aggregate skill text —
/// §4.2 refuses the step rather than dropping a binding, because a silently dropped skill breaks
/// `R-ID-5`'s identical-behaviour promise.
#[must_use]
pub fn phase_skills_over_cap() -> PromptSpec {
    let mut spec = phase_implement_attempt2();
    spec.max_skill_tokens = 100;
    spec.skills = vec![
        BoundSkill {
            skill_id: SkillId::from_uuid(Uuid::from_u128(0x5111)),
            name: "rust-style".to_owned(),
            version: 2,
            position: 0,
            body: prose_block("rust-style", 30),
        },
        BoundSkill {
            skill_id: SkillId::from_uuid(Uuid::from_u128(0x5222)),
            name: "command-queue".to_owned(),
            version: 1,
            position: 1,
            body: prose_block("command-queue", 30),
        },
    ];
    spec
}

/// A real [`TrimRecord`](crate::prompt::TrimRecord) for the demo store's `implement` step (T68).
///
/// Built from [`phase_oversize`] rather than from [`phase_implement_attempt2`] because the Runs
/// pane's two indicators are `~34k` and `!`: a record with nothing trimmed would render half of
/// what T68 has to show. `budget_source` is `project` either way, which is the field D106's
/// projection reads.
///
/// # Panics
///
/// If the oversize fixture stops assembling, which is a bug in this module rather than in a caller.
#[must_use]
pub fn demo_trim_record() -> crate::prompt::TrimRecord {
    crate::prompt::assemble(&phase_oversize(), &crate::scrub::MinimalScrubber::new([]))
        .expect("the oversize fixture assembles; it is over budget, not unassemblable")
        .trim
}

/// Criterion 6: the same spec with every body re-encoded as CRLF.
///
/// The template body is converted too, because a `prompt_template` row edited on Windows is
/// exactly how CRLF reaches the frame, and the frame's own line endings are what P-2's
/// normalise-first ordering exists for.
#[must_use]
pub fn with_crlf(spec: &PromptSpec) -> PromptSpec {
    fn crlf(s: &str) -> String {
        s.replace("\r\n", "\n").replace('\n', "\r\n")
    }
    let mut out = spec.clone();
    out.body = crlf(&out.body);
    out.item_body = crlf(&out.item_body);
    for doc in &mut out.documents {
        doc.body = crlf(&doc.body);
    }
    for entry in &mut out.upstream {
        entry.summary = entry.summary.as_deref().map(crlf);
    }
    for skill in &mut out.skills {
        skill.body = crlf(&skill.body);
    }
    for excerpt in &mut out.excerpts.files {
        excerpt.content = crlf(&excerpt.content);
    }
    out.box_profile.quirks = crlf(&out.box_profile.quirks);
    if let Some(verify) = &mut out.verify_failure {
        verify.output = crlf(&verify.output);
    }
    if let Some(diff) = &mut out.previous_diff {
        diff.stat = crlf(&diff.stat);
        diff.diff = crlf(&diff.diff);
    }
    if let Some(judge) = &mut out.judge {
        judge.task = crlf(&judge.task);
        for candidate in &mut judge.candidates {
            if let Some(doc) = &mut candidate.document {
                doc.body = crlf(&doc.body);
            }
            if let Some(diff) = &mut candidate.diff {
                diff.stat = crlf(&diff.stat);
                diff.diff = crlf(&diff.diff);
            }
            candidate.verification_tail = candidate.verification_tail.as_deref().map(crlf);
        }
    }
    if let Some(handoff) = &mut out.handoff {
        handoff.step_summary.last_assistant_tail = crlf(&handoff.step_summary.last_assistant_tail);
        if let Some(diff) = &mut handoff.diff_so_far {
            diff.stat = crlf(&diff.stat);
            diff.diff = crlf(&diff.diff);
        }
        handoff.failure_reason = crlf(&handoff.failure_reason);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::parse;

    #[test]
    fn every_fixture_body_parses_in_its_own_role() {
        for spec in [
            phase_implement_attempt2(),
            phase_all_empty(),
            judge_three_candidates(),
            handoff_basic(),
        ] {
            parse(spec.role, &spec.body)
                .unwrap_or_else(|e| panic!("`{}` does not parse: {e}", spec.template.name));
            assert_eq!(
                TemplateRole::of_name(&spec.template.name),
                spec.role,
                "`{}`'s role is derived from its name",
                spec.template.name
            );
            assert_eq!(
                spec.judge.is_some(),
                spec.role == TemplateRole::Judge,
                "judge inputs iff the judge role"
            );
            assert_eq!(
                spec.handoff.is_some(),
                spec.role == TemplateRole::Handoff,
                "handoff inputs iff the handoff role"
            );
        }
    }

    #[test]
    fn with_crlf_touches_every_body_and_nothing_else() {
        let plain = phase_implement_attempt2();
        let crlf = with_crlf(&plain);
        assert_ne!(crlf, plain);
        assert!(crlf.body.contains("\r\n"));
        assert!(crlf.item_body.contains("\r\n"));
        assert!(crlf.documents[0].body.contains("\r\n"));
        assert!(
            crlf.upstream[0]
                .summary
                .as_deref()
                .expect("a summary")
                .contains("\r\n")
        );
        assert!(crlf.skills[0].body.contains("\r\n"));
        assert!(crlf.excerpts.files[0].content.contains("\r\n"));
        assert!(crlf.box_profile.quirks.contains("\r\n"));
        // Idempotent, so a fixture that already carried CRLF does not gain `\r\r\n`.
        assert_eq!(with_crlf(&crlf), crlf);
        // The structural fields are untouched.
        assert_eq!(crlf.template, plain.template);
        assert_eq!(crlf.budget, plain.budget);
        assert_eq!(crlf.item_key, plain.item_key);
    }

    #[test]
    fn the_fixtures_carry_no_absolute_path_and_no_clock() {
        for spec in [
            phase_implement_attempt2(),
            phase_all_empty(),
            judge_three_candidates(),
            handoff_basic(),
        ] {
            for excerpt in &spec.excerpts.files {
                assert!(
                    !excerpt.path.starts_with('/') && !excerpt.path.contains(':'),
                    "§4.2 rule 5: `{}` must be repo-relative",
                    excerpt.path
                );
            }
            assert!(!spec.box_profile.quirks.contains("/home/"));
        }
    }
}
