//! MOD-26 milestone 1, T3: the persona frame (plan D13, I-7; blueprint §2.10, §6.3, B-7, B-17,
//! B-20).
//!
//! A spec that carries a persona renders it as the protected `persona` section **ahead of** the
//! template body, outside every span: `template` keeps `sections[0]` (P-9) and the persona is
//! `sections[1]`. The section is scrubbed like every other input, never trimmed, counted in the
//! digest, and joined to the body by [`PERSONA_SEPARATOR`], whose two bytes the frame's estimate
//! pays for. `{{persona}}` is internal: no template can place it. A persona-less spec renders
//! exactly as before — the goldens of `prompt_render.rs` and `prompt_digest.rs` are the proof, and
//! this file only asserts that nothing named `persona` appears.
//!
//! Every test starts from `fixtures::phase_implement_attempt2()`, the fixture in which every phase
//! section renders, with the `reviewer` persona below.
#![cfg(feature = "test-support")]

use htui_core::prompt::render::template_text;
use htui_core::prompt::{
    AssembleError, AssembledPrompt, Budget, BudgetSource, PERSONA_SEPARATOR, PersonaBlock,
    Placeholder, PromptSpec, SectionName, TemplateRole, TrimStrategy, assemble, fixtures, parse,
};
use htui_core::scrub::MinimalScrubber;

/// The persona every test binds unless it says otherwise.
const BODY: &str = "You are the reviewer.\n";

/// The frame `BODY` renders to, separator included: the first bytes of every persona prompt.
const FRAME: &str =
    "<section name=\"persona\" persona=\"reviewer\">\nYou are the reviewer.\n</section>\n\n";

/// The scrubber a production caller with no session secrets hands in, still fail-closed on the
/// prefix rules.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// The fixture with the `reviewer` persona whose body is `body`.
fn with_persona(body: &str) -> PromptSpec {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.persona = Some(PersonaBlock {
        name: "reviewer".to_owned(),
        body: body.to_owned(),
    });
    spec
}

/// The fixture with the `reviewer` persona.
fn spec() -> PromptSpec {
    with_persona(BODY)
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the spec must assemble: {error}"))
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

/// The record's `persona` row, if the section rendered.
fn persona_row(prompt: &AssembledPrompt) -> Option<&htui_core::prompt::Section> {
    prompt
        .trim
        .sections
        .iter()
        .find(|row| row.name == SectionName::Persona)
}

#[test]
fn a_persona_renders_first_and_is_sections_one() {
    let prompt = ok(&spec());
    assert!(
        prompt.text.starts_with(FRAME),
        "the persona frame comes first, then one blank line, then the body:\n{}",
        prompt.text
    );
    assert_eq!(
        prompt.text.matches("name=\"persona\"").count(),
        1,
        "rendered once"
    );
    assert_eq!(prompt.sections[0].name, SectionName::Template, "P-9");
    assert_eq!(prompt.sections[1].name, SectionName::Persona);
    assert_eq!(prompt.trim.sections, prompt.sections);
    assert_eq!(prompt.payload_sections()[1].name, "persona");
    assert!(
        prompt.digest_text.starts_with(FRAME),
        "both forms carry the frame"
    );
}

#[test]
fn the_persona_section_survives_any_trim() {
    let spec = spec();
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let persona = persona_row(&full)
        .expect("the persona section has a row")
        .clone();
    assert!(persona.tokens_before > 0);
    let floor = protected_floor(&full);

    // At the protected floor every trimmable section is driven to the end of its ladder — the
    // item to its 80-line floor, which this three-line body already is under, and every other one
    // dropped — and the persona loses nothing.
    let prompt = ok(&at_target(&spec, floor));
    for row in &prompt.trim.sections {
        if row.name.is_protected(TemplateRole::Phase) {
            assert_eq!(
                row.strategy,
                TrimStrategy::None,
                "`{}` is protected",
                row.name
            );
            assert!(!row.trimmed, "`{}` is protected", row.name);
            assert_eq!(row.tokens_before, row.tokens_after, "`{}`", row.name);
        } else if row.name != SectionName::Item {
            assert_eq!(
                row.strategy,
                TrimStrategy::Dropped,
                "`{}` is trimmable and goes before the persona does",
                row.name
            );
        }
    }
    assert_eq!(
        persona_row(&prompt),
        Some(&persona),
        "the persona row is untouched"
    );
    assert!(
        prompt.text.starts_with(FRAME),
        "and the frame is still first"
    );

    // One token less and the protected set alone is over target: the refusal counts the persona.
    let error = assemble(&at_target(&spec, floor - 1), &scrubber())
        .expect_err("the protected set alone is over target");
    let AssembleError::BudgetTooSmall { needed, target } = error else {
        panic!("expected BudgetTooSmall, got {error:?}");
    };
    assert_eq!(target, floor - 1);
    assert_eq!(needed, floor);
    let bare = ok(&at_target(
        &fixtures::phase_implement_attempt2(),
        i64::MAX / 4,
    ));
    assert_eq!(
        needed,
        protected_floor(&bare) + persona.tokens_before + full.sections[0].tokens_before
            - bare.sections[0].tokens_before,
        "the persona's own tokens and its separator's are both in `needed`"
    );
}

#[test]
fn a_secret_in_the_persona_body_is_masked() {
    const SECRET: &str = "hunter2";
    let spec = with_persona(&format!("You are the reviewer. The key is {SECRET}.\n"));
    let prompt = assemble(&spec, &MinimalScrubber::new([SECRET.to_owned()]))
        .expect("a maskable secret does not refuse the prompt");
    for (form, text) in [("text", &prompt.text), ("digest_text", &prompt.digest_text)] {
        assert!(!text.contains(SECRET), "the secret survived in `{form}`");
        assert!(
            text.contains("You are the reviewer. The key is [REDACTED]."),
            "the mask took its place in `{form}`"
        );
    }
}

#[test]
fn an_unmaskable_persona_body_refuses_the_prompt() {
    let spec = with_persona("Use the key sk-ant-api03-DEADBEEF.\n");
    let error = assemble(&spec, &scrubber()).expect_err("a prefix-rule secret cannot be masked");
    let AssembleError::Unmasked { ref section, .. } = error else {
        panic!("expected Unmasked, got {error:?}");
    };
    assert_eq!(section, "persona");
    assert!(!error.to_string().contains("sk-ant-api03-DEADBEEF"));
}

#[test]
fn a_template_cannot_place_the_persona_placeholder() {
    let mut spec = spec();
    spec.body = format!("{{{{persona}}}}\n\n{}", spec.body);
    assert_eq!(
        assemble(&spec, &scrubber()),
        Err(AssembleError::UnknownPlaceholder {
            token: "persona".to_owned()
        })
    );
    assert_eq!(Placeholder::from_token("persona"), None);
    assert_eq!(Placeholder::Persona.token(), "persona");
    assert_eq!(
        Placeholder::ALL.len(),
        20,
        "the internal placeholder is not listed"
    );
    assert!(!Placeholder::ALL.contains(&Placeholder::Persona));
    for role in [
        TemplateRole::Phase,
        TemplateRole::Judge,
        TemplateRole::Handoff,
    ] {
        assert!(
            !Placeholder::Persona.allowed_in(role),
            "no {role:?} body may place `{{{{persona}}}}`"
        );
    }
}

#[test]
fn the_digest_changes_with_the_persona_body() {
    let reviewer = ok(&spec());
    let other = ok(&with_persona("You are the architect.\n"));
    assert_ne!(reviewer.digest, other.digest, "the body is digested");
    let bare = ok(&fixtures::phase_implement_attempt2());
    assert_ne!(
        bare.digest, reviewer.digest,
        "binding a persona changes the digest"
    );
    assert_eq!(reviewer.digest, ok(&spec()).digest, "and reproducibly so");
}

#[test]
fn the_separator_is_counted_in_the_frame() {
    assert_eq!(PERSONA_SEPARATOR, "\n\n");
    let spec = spec();
    let est = spec.estimator;
    let template = template_text(&parse(spec.role, &spec.body).expect("the fixture body parses"));

    let with = ok(&spec);
    assert_eq!(with.sections[0].name, SectionName::Template);
    assert_eq!(
        with.sections[0].tokens_before,
        est.estimate(&format!("{template}{PERSONA_SEPARATOR}")),
        "B-17: the separator is the frame's"
    );

    let without = ok(&fixtures::phase_implement_attempt2());
    assert_eq!(
        without.sections[0].tokens_before,
        est.estimate(&template),
        "I-7: persona-less, the frame's estimate is today's"
    );
}

#[test]
fn a_persona_less_spec_is_unchanged() {
    let spec = fixtures::phase_implement_attempt2();
    assert_eq!(spec.persona, None);
    let prompt = ok(&spec);
    assert!(
        prompt
            .trim
            .sections
            .iter()
            .all(|row| row.name != SectionName::Persona),
        "no section is named `persona`"
    );
    assert!(
        prompt
            .payload_sections()
            .iter()
            .all(|row| row.name != "persona")
    );
    assert!(!prompt.text.contains("name=\"persona\""));
    assert!(!prompt.digest_text.contains("name=\"persona\""));
}
