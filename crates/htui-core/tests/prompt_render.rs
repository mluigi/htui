//! Golden bytes for every section of ANA-5 §4.2's closed vocabulary (`docs/ANA-5.md:435-590`).
//!
//! One accepted snapshot per section kind, plus the two upstream render states and a truncated
//! excerpt. They are **per section** rather than per assembled prompt because `assemble()` is T65's
//! and a whole-prompt golden cannot exist before it does; T65 adds `prompt_golden.rs` beside this
//! file for that, and these stay as the finer-grained contract. A diff in one of these files is a
//! change to what a model is shown and to every `prompt_digest` taken after it, so every hunk is
//! read before it is accepted.
//!
//! What the non-snapshot tests hold that a snapshot cannot: that CRLF input renders the same bytes
//! as LF input (criterion 6, at the section level), that criterion 4's three greps hold on a spec
//! with nothing to say, and that no absolute path and no `\r` reaches a rendered byte
//! (§4.2 rule 5, §4.7 rule 8).
#![cfg(feature = "test-support")]

use htui_core::prompt::render::{self, UpstreamState};
use htui_core::prompt::{SectionName, fixtures};

/// Every section a `phase_implement_attempt2()` spec renders, wrapped, in §4.2's table order.
///
/// A single function so the fixture is built once and so a reviewer reading the accepted snapshots
/// sees them in the order `R-PRM-1` lists them.
fn phase_sections() -> Vec<(&'static str, String)> {
    let spec = fixtures::phase_implement_attempt2();
    let mut out = vec![("section_item", render::wrap(&render::item(&spec)))];
    for doc in &spec.documents {
        let name: &'static str = match doc.kind.as_str() {
            "plan" => "section_documents_plan",
            "review" => "section_documents_review",
            other => unreachable!("the fixture carries `plan` and `review`, not `{other}`"),
        };
        out.push((name, render::wrap(&render::document(doc))));
    }
    out.push((
        "section_upstream",
        render::wrap(&render::upstream(&spec.upstream, &[]).expect("three entries")),
    ));
    out.push((
        "section_box",
        render::wrap(&render::box_profile(&spec.box_profile)),
    ));
    out.push((
        "section_skills",
        render::wrap(&render::skills(&spec.skills).expect("one binding")),
    ));
    out.push((
        "section_excerpts",
        render::wrap(&render::excerpts(&spec.excerpts.files).expect("two excerpts")),
    ));
    out.push((
        "section_verify_failure",
        render::wrap(&render::verify_failure(
            spec.verify_failure.as_ref().expect("attempt 2"),
        )),
    ));
    let diff = spec.previous_diff.as_ref().expect("attempt 2");
    out.push((
        "section_previous_diff",
        render::wrap(&render::diff(SectionName::PreviousDiff, diff, false)),
    ));
    out.push((
        "section_previous_diff_stat_only",
        render::wrap(&render::diff(SectionName::PreviousDiff, diff, true)),
    ));
    out.push((
        "section_command_queue",
        render::wrap(&render::command_queue()),
    ));
    out
}

#[test]
fn the_phase_sections_render_their_golden_bytes() {
    for (name, rendered) in phase_sections() {
        insta::assert_snapshot!(name, rendered);
    }
}

/// The frame itself: §4.2's first table row is "inline; no wrapper. It is the frame."
#[test]
fn the_template_frame_is_its_literal_spans() {
    let spec = fixtures::phase_implement_attempt2();
    let parsed = htui_core::prompt::parse(spec.role, &spec.body).expect("a seeded body parses");
    insta::assert_snapshot!("template_frame", render::template_text(&parsed));
}

/// §4.3's two render groups, each alone, because the joined case cannot show that neither one
/// leaves a stray separator behind when the other is absent.
#[test]
fn the_two_upstream_states_render_alone() {
    let spec = fixtures::phase_implement_attempt2();
    let summaries_only: Vec<UpstreamState> = spec
        .upstream
        .iter()
        .map(|e| {
            if e.is_summary() {
                UpstreamState::Summary
            } else {
                UpstreamState::Dropped
            }
        })
        .collect();
    insta::assert_snapshot!(
        "section_upstream_summaries_only",
        render::wrap(&render::upstream(&spec.upstream, &summaries_only).expect("one summary"))
    );

    // The ladder's degrade: every row as a one-liner, including the one that has a summary.
    let all_one_liners: Vec<UpstreamState> = spec
        .upstream
        .iter()
        .map(|e| {
            if e.in_scope {
                UpstreamState::Pending
            } else {
                UpstreamState::Stub
            }
        })
        .collect();
    insta::assert_snapshot!(
        "section_upstream_one_liners_only",
        render::wrap(&render::upstream(&spec.upstream, &all_one_liners).expect("three rows"))
    );
}

#[test]
fn the_judge_sections_render_their_golden_bytes() {
    let spec = fixtures::judge_three_candidates();
    let judge = spec.judge.as_ref().expect("the judge role carries inputs");
    insta::assert_snapshot!(
        "section_judge_task",
        render::wrap(&render::judge_task(&judge.task))
    );
    for candidate in &judge.candidates {
        let name = match candidate.fanout_index {
            0 => "section_judge_candidate_0",
            1 => "section_judge_candidate_1",
            _ => "section_judge_candidate_2",
        };
        insta::assert_snapshot!(
            name,
            render::wrap(&render::judge_candidate(candidate, false))
        );
    }
    // §4.6a's isolate cap: a candidate whose diff exceeds its share renders as a stat only
    // (criterion 17's last clause, at the render level).
    insta::assert_snapshot!(
        "section_judge_candidate_2_stat_only",
        render::wrap(&render::judge_candidate(&judge.candidates[2], true))
    );
}

#[test]
fn the_handoff_sections_render_their_golden_bytes() {
    let spec = fixtures::handoff_basic();
    let handoff = spec
        .handoff
        .as_ref()
        .expect("the handoff role carries inputs");
    insta::assert_snapshot!(
        "section_step_summary",
        render::wrap(&render::step_summary(&handoff.step_summary))
    );
    insta::assert_snapshot!(
        "section_diff_so_far",
        render::wrap(&render::diff(
            SectionName::DiffSoFar,
            handoff.diff_so_far.as_ref().expect("a diff so far"),
            false,
        ))
    );
    insta::assert_snapshot!(
        "section_failure_reason",
        render::wrap(&render::failure_reason(&handoff.failure_reason))
    );
}

/// Criterion 4's three greps, at the level this task can hold them: a spec with nothing to say
/// renders **no section at all** for documents, upstream and excerpts — not an empty one.
///
/// The fourth clause, "no run of three or more consecutive newlines survives", is
/// `digest::canonical`'s and is asserted in its own unit tests; the whole-prompt form is T65's,
/// because it needs the substitution step.
#[test]
fn a_spec_with_nothing_to_say_renders_no_empty_wrappers() {
    let spec = fixtures::phase_all_empty();
    assert!(spec.documents.is_empty(), "no input kinds resolved");
    assert!(
        render::upstream(&spec.upstream, &[]).is_none(),
        "no `<section name=\"upstream\">`"
    );
    assert!(
        render::skills(&spec.skills).is_none(),
        "no `<section name=\"skills\">`"
    );
    assert!(
        render::excerpts(&spec.excerpts.files).is_none(),
        "no `<section name=\"excerpts\">`"
    );
    assert!(spec.verify_failure.is_none() && spec.previous_diff.is_none());

    // The two sections that always exist still render, and the empty item body takes no blank
    // line with it.
    insta::assert_snapshot!("section_item_empty", render::wrap(&render::item(&spec)));
    insta::assert_snapshot!(
        "section_box_minimal",
        render::wrap(&render::box_profile(&spec.box_profile))
    );
}

/// Criterion 6 at the section level: the same spec re-encoded as CRLF renders byte-identically.
///
/// This is the property P-2 exists for. Normalising at render time rather than after the collapse
/// is what makes it true — collapse-then-normalise would leave a CRLF fixture with line endings the
/// LF fixture does not have, and the whole-prompt digest would differ by exactly that.
#[test]
fn crlf_inputs_render_identical_bytes() {
    let lf = fixtures::phase_implement_attempt2();
    let crlf = fixtures::with_crlf(&lf);
    assert_ne!(crlf, lf, "the fixture really is re-encoded");

    let lf_sections = phase_sections();
    let crlf_sections = {
        let spec = crlf;
        let mut out = vec![render::wrap(&render::item(&spec))];
        for doc in &spec.documents {
            out.push(render::wrap(&render::document(doc)));
        }
        out.push(render::wrap(
            &render::upstream(&spec.upstream, &[]).expect("three entries"),
        ));
        out.push(render::wrap(&render::box_profile(&spec.box_profile)));
        out.push(render::wrap(
            &render::skills(&spec.skills).expect("one binding"),
        ));
        out.push(render::wrap(
            &render::excerpts(&spec.excerpts.files).expect("two excerpts"),
        ));
        out.push(render::wrap(&render::verify_failure(
            spec.verify_failure.as_ref().expect("attempt 2"),
        )));
        let diff = spec.previous_diff.as_ref().expect("attempt 2");
        out.push(render::wrap(&render::diff(
            SectionName::PreviousDiff,
            diff,
            false,
        )));
        out.push(render::wrap(&render::diff(
            SectionName::PreviousDiff,
            diff,
            true,
        )));
        out.push(render::wrap(&render::command_queue()));
        out
    };

    assert_eq!(lf_sections.len(), crlf_sections.len());
    for ((name, lf_bytes), crlf_bytes) in lf_sections.iter().zip(&crlf_sections) {
        assert_eq!(lf_bytes, crlf_bytes, "`{name}` differs under CRLF input");
    }
}

/// §4.2 rule 5 and §4.7 rules 7 and 8, over every rendered byte this task can produce.
///
/// Rule 8's fan-out clause is what makes this worth asserting rather than assuming: two siblings
/// differ only by `<config_dir>/trees/<run_id>/<step_id>/`, so a single absolute path anywhere in
/// the render would make ANA-5 invariant 3 false and the check would still pass every other test.
#[test]
fn no_rendered_byte_carries_a_path_a_clock_or_a_carriage_return() {
    let mut every: Vec<String> = phase_sections()
        .into_iter()
        .map(|(_, rendered)| rendered)
        .collect();

    let judge = fixtures::judge_three_candidates();
    let inputs = judge.judge.as_ref().expect("judge inputs");
    every.push(render::wrap(&render::judge_task(&inputs.task)));
    for candidate in &inputs.candidates {
        every.push(render::wrap(&render::judge_candidate(candidate, false)));
    }

    let handoff = fixtures::handoff_basic();
    let inputs = handoff.handoff.as_ref().expect("handoff inputs");
    every.push(render::wrap(&render::step_summary(&inputs.step_summary)));
    every.push(render::wrap(&render::failure_reason(
        &inputs.failure_reason,
    )));

    for rendered in &every {
        assert!(!rendered.contains('\r'), "a CR survived into:\n{rendered}");
        for line in rendered.lines() {
            for needle in ["/home/", "/tmp/", "/scratch/", "C:\\", "/Users/"] {
                assert!(
                    !line.contains(needle),
                    "`{needle}` is an absolute path and reached:\n{line}"
                );
            }
        }
        // Rule 7: no wall-clock value. The fixtures carry no timestamp, so the cheap proof is that
        // nothing ISO-8601-shaped appears — a `T..Z` stamp is what a careless render would add.
        assert!(
            !rendered.contains("Z\n") && !rendered.contains("+00:00"),
            "a timestamp reached:\n{rendered}"
        );
    }
}
