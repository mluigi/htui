//! MOD-139 milestone 2, T2: the internal `part` section (plan D10; blueprint §1.9, §T2.4).
//!
//! A divided node's part session carries its brief as the internal `part` section: rendered after
//! the persona and the memory and before the template body, recorded as the next `sections[]`
//! entry, protected (an oversized brief refuses the prompt rather than being trimmed), scrubbed
//! like every other input, and joined to the body by [`PART_SEPARATOR`], whose two bytes the
//! frame's estimate pays for. `{{part}}` is internal: no template can place it, so every divided
//! template carries the brief without an edit and none can drop it. A judge or handoff spec never
//! renders one, and a part-less spec renders exactly as before — the `prompt_golden__*` snapshots
//! are the byte proof.
#![cfg(feature = "test-support")]

use htui_core::prompt::render::{self, template_text};
use htui_core::prompt::{
    AssembleError, AssembledPrompt, Budget, BudgetSource, PART_SEPARATOR, PartBlock, PersonaBlock,
    Placeholder, PromptSpec, Section, SectionName, TemplateRole, TrimStrategy, assemble, fixtures,
    parse,
};
use htui_core::scrub::MinimalScrubber;

/// The scrubber a production caller with no session secrets hands in, still fail-closed on the
/// prefix rules.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the spec must assemble: {error}"))
}

/// The phase fixture in which every phase section renders, working the `api` part.
fn spec() -> PromptSpec {
    with_part(fixtures::part_api())
}

/// The phase fixture working `part`.
fn with_part(part: PartBlock) -> PromptSpec {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.part = Some(part);
    spec
}

/// [`spec`] behind the `reviewer` persona and the worked memory example.
fn with_persona_and_memory() -> PromptSpec {
    let mut spec = spec();
    spec.persona = Some(PersonaBlock {
        name: "reviewer".to_owned(),
        body: "You are the reviewer.\n".to_owned(),
    });
    spec.memory = Some(fixtures::memory_two_of_three());
    spec
}

/// `spec` at `target` tokens with no reserve, so the test's arithmetic is the target's.
fn at_target(spec: &PromptSpec, target: i64) -> PromptSpec {
    let mut out = spec.clone();
    out.budget = Budget {
        tokens: target,
        source: BudgetSource::Project,
        reserve_bp: 0,
    };
    out
}

/// What the frame and the protected sections cost together: the floor no trim goes under.
fn protected_floor(prompt: &AssembledPrompt) -> i64 {
    prompt
        .trim
        .sections
        .iter()
        .filter(|row| row.name.is_protected(TemplateRole::Phase))
        .map(|row| row.tokens_before)
        .sum()
}

/// The record's `part` row, if the section rendered.
fn part_row(prompt: &AssembledPrompt) -> Option<&Section> {
    prompt
        .trim
        .sections
        .iter()
        .find(|row| row.name == SectionName::Part)
}

/// The `part` span as `substitute` writes it: the wrapped section, then its separator.
fn part_span(part: &PartBlock) -> String {
    format!("{}{PART_SEPARATOR}", render::wrap(&render::part(part)))
}

/// The byte offset of `needle` in `text`, or a panic naming it.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is not in the prompt:\n{text}"))
}

#[test]
fn a_part_section_renders_after_persona_and_memory() {
    let prompt = ok(&with_persona_and_memory());
    let span = part_span(&fixtures::part_api());
    for (form, text) in [("text", &prompt.text), ("digest_text", &prompt.digest_text)] {
        let persona = at(text, "<section name=\"persona\"");
        let memory = at(text, "<section name=\"memory\"");
        let part = at(text, &span);
        let body = at(text, "You are running the `implement` phase");
        assert_eq!(
            persona, 0,
            "the persona frame still comes first in `{form}`"
        );
        assert!(persona < memory, "persona, then memory in `{form}`");
        assert!(memory < part, "memory, then the part in `{form}`");
        assert_eq!(
            part + span.len(),
            body,
            "the part and its blank line, then the body in `{form}`"
        );
        assert_eq!(
            text.matches("<section name=\"part\"").count(),
            1,
            "rendered once in `{form}`"
        );
    }
}

#[test]
fn the_part_section_is_recorded_after_persona_and_memory() {
    let names = |prompt: &AssembledPrompt| -> Vec<SectionName> {
        prompt.sections.iter().map(|row| row.name.clone()).collect()
    };

    let prompt = ok(&with_persona_and_memory());
    assert_eq!(
        names(&prompt)[..4],
        [
            SectionName::Template,
            SectionName::Persona,
            SectionName::Memory,
            SectionName::Part,
        ],
        "D10: the part is recorded after the persona and the memory"
    );
    assert_eq!(prompt.trim.sections, prompt.sections);
    assert_eq!(prompt.payload_sections()[3].name, "part");

    let prompt = ok(&spec());
    assert_eq!(
        names(&prompt)[..2],
        [SectionName::Template, SectionName::Part],
        "with neither, the part is `sections[1]`"
    );
    assert_eq!(
        prompt.sections[2].name,
        SectionName::Item,
        "the body's spans follow"
    );
    assert!(
        prompt.sections[2..]
            .iter()
            .all(|row| row.name != SectionName::Part),
        "recorded once"
    );
}

#[test]
fn the_part_placeholder_is_internal() {
    assert_eq!(Placeholder::Part.token(), "part");
    assert_eq!(Placeholder::from_token("part"), None);
    assert!(!Placeholder::ALL.contains(&Placeholder::Part));
    assert_eq!(
        Placeholder::ALL.len(),
        20,
        "the internal placeholder is not listed"
    );
    assert!(Placeholder::Part.is_section());
    for role in [
        TemplateRole::Phase,
        TemplateRole::Judge,
        TemplateRole::Handoff,
    ] {
        assert!(
            !Placeholder::Part.allowed_in(role),
            "no {role:?} body may place `{{{{part}}}}`"
        );
    }

    let mut spec = spec();
    spec.body = "{{part}}".to_owned();
    assert_eq!(
        assemble(&spec, &scrubber()),
        Err(AssembleError::UnknownPlaceholder {
            token: "part".to_owned()
        })
    );
}

#[test]
fn an_oversized_brief_refuses_the_prompt() {
    let brief = "Keep every handler thin and every error typed.\n".repeat(400);
    let mut part = fixtures::part_api();
    part.brief = brief;
    let spec = with_part(part);

    let full = ok(&at_target(&spec, i64::MAX / 4));
    let row = part_row(&full).expect("the part section has a row").clone();
    assert!(row.tokens_before > 1_000, "the brief is large");
    let floor = protected_floor(&full);

    // At the protected floor everything trimmable goes first; the part loses nothing.
    let prompt = ok(&at_target(&spec, floor));
    assert_eq!(part_row(&prompt), Some(&row), "the part row is untouched");
    assert_eq!(row.strategy, TrimStrategy::None);
    assert!(!row.trimmed);

    // A target the brief alone is over: refused, never trimmed.
    for target in [floor - 1, row.tokens_before] {
        let error = assemble(&at_target(&spec, target), &scrubber())
            .expect_err("the protected set alone is over target");
        let AssembleError::BudgetTooSmall { needed, target: t } = error else {
            panic!("expected BudgetTooSmall, got {error:?}");
        };
        assert_eq!(t, target);
        assert_eq!(needed, floor, "the part's tokens are in `needed`");
    }
}

#[test]
fn a_part_secret_is_masked() {
    const SECRET: &str = "hunter2";
    let part = PartBlock {
        key: "api".to_owned(),
        title: format!("The HTTP API {SECRET}"),
        brief: format!("Add the endpoint; the key is {SECRET}."),
        owned_paths: vec!["htui:crates/api/**".to_owned(), format!("docs/{SECRET}.md")],
        acceptance: Some(format!("`{SECRET}` never reaches a log.")),
    };
    let prompt = assemble(&with_part(part), &MinimalScrubber::new([SECRET.to_owned()]))
        .expect("a maskable secret does not refuse the prompt");
    for (form, text) in [("text", &prompt.text), ("digest_text", &prompt.digest_text)] {
        assert!(!text.contains(SECRET), "the secret survived in `{form}`");
        for masked in [
            "title=\"The HTTP API [REDACTED]\"",
            "Add the endpoint; the key is [REDACTED].",
            "- docs/[REDACTED].md",
            "`[REDACTED]` never reaches a log.",
        ] {
            assert!(text.contains(masked), "`{masked}` in `{form}`");
        }
    }
}

#[test]
fn an_unmaskable_part_brief_refuses_the_prompt() {
    let mut part = fixtures::part_api();
    part.brief = "Use the key sk-ant-api03-DEADBEEFDEADBEEFDEADBEEF.\n".to_owned();
    let error =
        assemble(&with_part(part), &scrubber()).expect_err("a pattern-rule secret is refused");
    let AssembleError::Unmasked { ref section, .. } = error else {
        panic!("expected Unmasked, got {error:?}");
    };
    assert_eq!(section, "part");
    let rendered = error.to_string();
    assert!(!rendered.contains("DEADBEEF"), "{rendered}");
}

#[test]
fn only_a_phase_spec_renders_a_part() {
    for mut spec in [
        fixtures::judge_three_candidates(),
        fixtures::handoff_basic(),
    ] {
        let before = ok(&spec);
        spec.part = Some(fixtures::part_api());
        let with = ok(&spec);
        assert!(
            !with.text.contains("name=\"part\""),
            "a {:?} spec renders no part",
            spec.role
        );
        assert!(part_row(&with).is_none());
        assert_eq!(with, before, "a {:?} spec ignores its part", spec.role);
    }

    let empty = fixtures::phase_all_empty();
    assert_eq!(empty.part, None);
    let prompt = ok(&empty);
    assert!(part_row(&prompt).is_none());
    assert!(!prompt.text.contains("name=\"part\""));
    assert!(!prompt.digest_text.contains("name=\"part\""));
    assert_eq!(
        prompt.sections[0].tokens_before,
        empty.estimator.estimate(&template_text(
            &parse(empty.role, &empty.body).expect("parses")
        )),
        "I-7: part-less, the frame's estimate is today's"
    );
}

#[test]
fn the_separator_is_counted_in_the_frame() {
    assert_eq!(PART_SEPARATOR, "\n\n");
    let spec = spec();
    let est = spec.estimator;
    let template = template_text(&parse(spec.role, &spec.body).expect("the fixture body parses"));
    let with = ok(&spec);
    assert_eq!(
        with.sections[0].tokens_before,
        est.estimate(&format!("{template}{PART_SEPARATOR}")),
        "the separator is the frame's, as the persona's is (B-17)"
    );
}

#[test]
fn two_parts_differ_in_their_part_section_only() {
    let mut other = fixtures::part_api();
    other.key = "ui".to_owned();
    other.title = "The Runs pane".to_owned();
    other.brief = "Show the endpoint's runs.".to_owned();
    other.owned_paths = vec!["htui:crates/htui/src/tabs/**".to_owned()];
    other.acceptance = None;

    let api = ok(&spec());
    let ui = ok(&with_part(other.clone()));
    assert_ne!(api.digest, ui.digest, "the part is digested");
    assert_eq!(
        api.text.replacen(&part_span(&fixtures::part_api()), "", 1),
        ui.text.replacen(&part_span(&other), "", 1),
        "D10: every other byte is the node's"
    );
}
