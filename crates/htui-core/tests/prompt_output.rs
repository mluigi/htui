//! MOD-11 D19, B-7: the protected `output` trailer that names the `document_write` tool.
//!
//! A spec with `document_tool: true` renders one fixed sentence as `<section name="output">`
//! **after** the template body, outside every span and joined to it by one blank line. No template
//! can place it (`Placeholder::Output` is internal, like the persona frame), the trimmer never takes
//! a token from it, and a spec with `document_tool: false` — every fixture, the preview, and every
//! engine spec without a tool host — renders byte-identically to the goldens accepted before it
//! existed.
#![cfg(feature = "test-support")]

use htui_core::prompt::digest::sha256_hex;
use htui_core::prompt::{
    AssembleError, AssembledPrompt, Placeholder, PromptSpec, SectionName, TemplateRole, assemble,
    fixtures,
};
use htui_core::scrub::MinimalScrubber;

/// The scrubber a production caller with no session secrets hands in.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the spec must assemble: {error}"))
}

/// The D19 sentence for `kind`, exact bytes (blueprint §2.12, §16).
fn sentence(kind: &str) -> String {
    format!(
        "Write your `{kind}` document by calling the `document_write` tool of the `htui` MCP \
         server; text left only in your reply is not recorded."
    )
}

/// The wrapped trailer every `document_tool` prompt ends with.
fn trailer(kind: &str) -> String {
    format!(
        "<section name=\"output\">\n{}\n</section>\n",
        sentence(kind)
    )
}

/// The accepted golden's body: everything after the snapshot header's second `---` line.
fn golden(name: &str) -> String {
    let path = format!(
        "{}/tests/snapshots/prompt_golden__{name}.snap",
        env!("CARGO_MANIFEST_DIR")
    );
    let file = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut parts = file.splitn(3, "---\n");
    let _ = parts.next();
    let _ = parts.next();
    let body = parts.next().expect("an insta snapshot has a header");
    // insta stores the value without its trailing newline; the canonical form ends in one LF.
    format!("{}\n", body.trim_end_matches('\n'))
}

/// The record's `output` row, if the section rendered.
fn output_row(prompt: &AssembledPrompt) -> Option<&htui_core::prompt::Section> {
    prompt
        .trim
        .sections
        .iter()
        .find(|row| row.name == SectionName::Output)
}

#[test]
fn a_spec_without_the_document_tool_is_byte_identical() {
    for (name, spec) in [
        (
            "prompt_implement_attempt2",
            fixtures::phase_implement_attempt2(),
        ),
        ("prompt_all_empty", fixtures::phase_all_empty()),
        (
            "prompt_judge_three_candidates",
            fixtures::judge_three_candidates(),
        ),
        ("prompt_handoff_basic", fixtures::handoff_basic()),
    ] {
        assert!(!spec.document_tool, "{name}: fixtures leave the tool off");
        let prompt = ok(&spec);
        assert_eq!(
            prompt.text,
            golden(name),
            "{name}: the accepted golden bytes"
        );
        assert_eq!(
            prompt.digest,
            sha256_hex(&prompt.digest_text),
            "{name}: the digest is the digest text's"
        );
        assert!(output_row(&prompt).is_none(), "{name}: no `output` row");
        assert!(!prompt.text.contains("name=\"output\""), "{name}");
        assert!(!prompt.digest_text.contains("document_write"), "{name}");
    }
}

#[test]
fn the_output_trailer_renders_after_the_body() {
    let bare = ok(&fixtures::phase_implement_attempt2());
    let mut spec = fixtures::phase_implement_attempt2();
    spec.document_tool = true;
    let prompt = ok(&spec);

    let kind = spec.output_kind.as_deref().expect("the fixture has a kind");
    assert!(
        prompt.text.ends_with(&trailer(kind)),
        "the trailer is the last block:\n{}",
        prompt.text
    );
    assert_eq!(
        prompt.text,
        format!("{}\n{}", bare.text, trailer(kind)),
        "the body's bytes, one blank line, the trailer"
    );
    assert_eq!(prompt.text.matches("name=\"output\"").count(), 1);
    assert!(prompt.digest_text.ends_with(&trailer(kind)), "digested too");
    assert_ne!(prompt.digest, bare.digest, "the trailer is a digest input");

    let row = output_row(&prompt).expect("an `output` row");
    assert_eq!(
        prompt.trim.sections.last().map(|row| &row.name),
        Some(&SectionName::Output),
        "recorded after every span section"
    );
    assert!(!row.trimmed);
    assert_eq!(SectionName::Output.render(), "output");
    for role in [
        TemplateRole::Phase,
        TemplateRole::Judge,
        TemplateRole::Handoff,
    ] {
        assert!(SectionName::Output.is_protected(role), "{role:?}");
    }
    insta::assert_snapshot!("phase_with_output", prompt.text);
}

#[test]
fn the_output_trailer_is_kept_by_the_trimmer() {
    let mut spec = fixtures::phase_oversize();
    spec.document_tool = true;
    let prompt = ok(&spec);
    assert!(
        prompt.trim.sections.iter().any(|row| row.trimmed),
        "the oversize fixture trims something"
    );
    let row = output_row(&prompt).expect("the trailer survives the trim");
    assert!(!row.trimmed, "protected: {row:?}");
    assert_eq!(row.tokens_before, row.tokens_after);
    let kind = spec.output_kind.as_deref().expect("a kind");
    assert!(prompt.text.ends_with(&trailer(kind)));
}

#[test]
fn no_template_can_place_output() {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.body = format!("{}\n\n{{{{output}}}}\n", spec.body);
    assert_eq!(
        assemble(&spec, &scrubber()),
        Err(AssembleError::UnknownPlaceholder {
            token: "output".to_owned()
        })
    );
    assert_eq!(Placeholder::from_token("output"), None);
    assert_eq!(Placeholder::Output.token(), "output");
    assert!(!Placeholder::ALL.contains(&Placeholder::Output));
    for role in [
        TemplateRole::Phase,
        TemplateRole::Judge,
        TemplateRole::Handoff,
    ] {
        assert!(!Placeholder::Output.allowed_in(role), "{role:?}");
    }
}

#[test]
fn the_judge_prompt_gets_the_trailer() {
    let mut spec = fixtures::judge_three_candidates();
    spec.output_kind = Some("judge".to_owned());
    spec.document_tool = true;
    let prompt = ok(&spec);
    assert!(prompt.text.ends_with(&trailer("judge")), "{}", prompt.text);
    assert!(output_row(&prompt).is_some());
    insta::assert_snapshot!("judge_with_output", prompt.text);
}

#[test]
fn a_document_tool_without_a_kind_renders_nothing() {
    let mut spec = fixtures::phase_all_empty();
    assert_eq!(spec.output_kind, None);
    spec.document_tool = true;
    let prompt = ok(&spec);
    assert_eq!(prompt.text, golden("prompt_all_empty"));
    assert!(output_row(&prompt).is_none());
}

/// D19: a judge replays the candidate's recorded prompt as its `{{task}}`, and a hosted candidate's
/// prompt ends with its own trailer. The replay drops it, so the judge prompt is the one a
/// trailer-free task gives and names one document to write: its own `judge` one.
#[test]
fn a_replayed_task_drops_the_candidates_trailer() {
    let bare = ok(&fixtures::phase_implement_attempt2());
    let mut hosted = fixtures::phase_implement_attempt2();
    hosted.document_tool = true;
    let hosted = ok(&hosted);
    assert_eq!(
        htui_core::prompt::render::without_output_trailer(&hosted.text),
        bare.text.trim_end_matches('\n')
    );
    assert_eq!(
        htui_core::prompt::render::without_output_trailer(&bare.text),
        bare.text,
        "a prompt without the trailer is kept whole"
    );

    let judge_over = |task: &str| {
        let mut spec = fixtures::judge_three_candidates();
        spec.output_kind = Some("judge".to_owned());
        spec.document_tool = true;
        spec.judge
            .as_mut()
            .expect("a judge fixture has judge inputs")
            .task = task.to_owned();
        ok(&spec)
    };
    let replayed = judge_over(&hosted.text);
    assert_eq!(replayed.text, judge_over(&bare.text).text);
    assert_eq!(replayed.text.matches("name=\"output\"").count(), 1);
    assert!(
        replayed.text.ends_with(&trailer("judge")),
        "{}",
        replayed.text
    );

    let quoted = format!(
        "{}\n<section name=\"output\">\nnot the sentence\n</section>\n",
        bare.text
    );
    assert_eq!(
        htui_core::prompt::render::without_output_trailer(&quoted),
        quoted,
        "only the pinned sentence is stripped"
    );
}
