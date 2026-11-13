//! Whole-prompt goldens: the bytes `assemble()` hands to a driver, one accepted snapshot per role
//! (ANA-5 §4.7 `:1385-1398`, §12 criterion 4).
//!
//! `prompt_render.rs` pins each **section** of §4.2's closed vocabulary; this file pins what the
//! substitution, the collapse and the canonical form make of them together. The distinction is not
//! bookkeeping: every rule of §4.7 step 5 onwards — the blank line between two sections under one
//! placeholder, the collapse of any run of three or more newlines, the single trailing LF — is
//! invisible at section level and is the whole content of a whole-prompt golden.
//!
//! Landed by T67 rather than T64, which is finding **F-36**: T64's plan entry names this file and
//! T64 shipped only the per-section goldens beside it, so criterion 4 — "the all-empty fixture
//! renders no `documents:` / `upstream` / `excerpts` section and no run of three LFs" — was proved
//! at section level and nowhere else. The three greps below are that criterion at the level it is
//! written about, and they run **before** the snapshot so a re-accepted `.snap` cannot absorb a
//! regression in them (blueprint H-11).
#![cfg(feature = "test-support")]

use htui_core::prompt::{AssembledPrompt, PromptSpec, assemble, fixtures};
use htui_core::scrub::MinimalScrubber;

/// The scrubber a production caller with no session secrets hands in: still fail-closed on the
/// prefix rules (`scrub.rs:114-120`). The same one `prompt_digest.rs` uses, so the two suites
/// describe one pipeline.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error, which is what a failing golden should print.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the fixture must assemble: {error}"))
}

/// §4.7 step 5 and step 6, asserted on every golden before it is snapshotted.
///
/// Three properties no accepted snapshot may quietly lose: no run of three newlines survives the
/// collapse, the text ends in exactly one LF, and no `\r` reaches a rendered byte (P-2).
fn canonical_form_holds(prompt: &AssembledPrompt) {
    assert!(
        !prompt.text.contains("\n\n\n"),
        "§4.7 step 5 collapses any run of three or more newlines to two"
    );
    assert!(
        prompt.text.ends_with('\n') && !prompt.text.ends_with("\n\n"),
        "the canonical form ends in exactly one LF"
    );
    assert!(
        !prompt.text.contains('\r'),
        "P-2 normalises every content string to LF before anything measures it"
    );
    assert_eq!(prompt.digest.len(), 64, "lowercase sha256 hex, all 64");
}

/// §5.1's shape: an `implement` step on attempt 2 with every renderable section present.
#[test]
fn the_implement_prompt_renders_its_golden_bytes() {
    let prompt = ok(&fixtures::phase_implement_attempt2());
    canonical_form_holds(&prompt);
    insta::assert_snapshot!("prompt_implement_attempt2", prompt.text);
}

/// **Criterion 4**, whole-prompt: a position-0 phase with `input_kinds = []` on a root item with no
/// upstream links, no excerpts and attempt 1.
///
/// The four clauses of the criterion, in its own order: it assembles; the rendered text carries
/// none of the three optional sections; `sections[]` has entries only for sections that
/// contributed bytes; and no run of three or more newlines survives.
#[test]
fn the_all_empty_prompt_omits_every_optional_section() {
    let spec = fixtures::phase_all_empty();
    assert_eq!(spec.attempt, 1, "criterion 4 is an attempt-1 fixture");
    assert!(spec.documents.is_empty(), "`input_kinds = []`");
    assert!(spec.upstream.is_empty(), "a root item with no upstream");
    assert!(spec.excerpts.files.is_empty(), "no excerpts");

    let prompt = ok(&spec);

    // The three greps, verbatim from the criterion.
    assert!(
        !prompt.text.contains("<section name=\"documents:"),
        "no `documents:` section:\n{}",
        prompt.text
    );
    assert!(
        !prompt.text.contains("<section name=\"upstream\">"),
        "no `upstream` section:\n{}",
        prompt.text
    );
    assert!(
        !prompt.text.contains("<section name=\"excerpts\">"),
        "no `excerpts` section:\n{}",
        prompt.text
    );

    // `sections[]` has an entry only for what contributed bytes. `item` is §4.2 `:566-569`'s named
    // exception — "this item has no body" is information — and is the one zero-byte entry allowed.
    // `box` is here because a `BoxProfile` is never absent: §4.2 makes it a protected section and
    // this box has a hostname, an OS and an arch whatever else it lacks.
    let names: Vec<String> = prompt
        .payload_sections()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(
        names,
        vec!["template".to_owned(), "item".to_owned(), "box".to_owned()],
        "the frame, the (empty) item and the box are what contributed bytes"
    );
    for entry in prompt.payload_sections() {
        assert!(
            !entry.trimmed,
            "nothing was trimmed in a prompt this far under budget: {entry:?}"
        );
    }

    canonical_form_holds(&prompt);
    insta::assert_snapshot!("prompt_all_empty", prompt.text);
}

/// Criterion 17's bytes: `judge_task` once, then the three candidates in `fanout_index` order.
#[test]
fn the_judge_prompt_renders_its_golden_bytes() {
    let prompt = ok(&fixtures::judge_three_candidates());
    canonical_form_holds(&prompt);
    insta::assert_snapshot!("prompt_judge_three_candidates", prompt.text);
}

/// Criterion 18's assembler half: the handoff frame around a deterministic step summary.
#[test]
fn the_handoff_prompt_renders_its_golden_bytes() {
    let prompt = ok(&fixtures::handoff_basic());
    canonical_form_holds(&prompt);
    insta::assert_snapshot!("prompt_handoff_basic", prompt.text);
}
