//! MOD-9 milestone 2, T2: which skill candidates render, and the record of every one of them
//! (plan D40, D42, D43; blueprint §3.5).
//!
//! The assembler collapses the step's candidates most-specific-wins, then `select` decides each:
//! only the **active** ones render, are estimated and meet `max_skill_tokens`, and every candidate
//! lands in `trim_record.skill_choices` with the reason it did or did not render (ANA-22 §6 item
//! 8). Every test starts from `fixtures::phase_skills_over_cap()` with the cap raised to 20 000,
//! so the base spec carries two `Always` project skills — `rust-style` v2 at position 0 and
//! `command-queue` v1 at position 1 — and assembles.
#![cfg(feature = "test-support")]

use htui_core::model::{Activation, ChoiceReason, SkillChoice, SkillLevel};
use htui_core::prompt::{
    AssembleError, AssembledPrompt, PromptSpec, SectionName, assemble, fixtures,
};
use htui_core::scrub::MinimalScrubber;

/// The scrubber a production caller with no session secrets hands in, as `prompt_golden.rs`.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the spec must assemble: {error}"))
}

/// Two `Always` project skills, the cap well above both.
fn base() -> PromptSpec {
    let mut spec = fixtures::phase_skills_over_cap();
    spec.max_skill_tokens = 20_000;
    spec
}

/// The `skills` row of the record's `sections[]`, if the section rendered.
fn skills_row(prompt: &AssembledPrompt) -> Option<&htui_core::prompt::Section> {
    prompt
        .trim
        .sections
        .iter()
        .find(|row| row.name == SectionName::Skills)
}

#[test]
fn the_base_spec_renders_both_skills_and_records_both_always() {
    let prompt = ok(&base());
    assert!(
        prompt
            .text
            .contains("<skill name=\"rust-style\" version=\"2\">")
    );
    assert!(
        prompt
            .text
            .contains("<skill name=\"command-queue\" version=\"1\">")
    );
    let reasons: Vec<ChoiceReason> = prompt
        .trim
        .skill_choices
        .iter()
        .map(|choice| choice.reason)
        .collect();
    assert_eq!(reasons, vec![ChoiceReason::Always, ChoiceReason::Always]);
    assert!(prompt.trim.skill_choices.iter().all(|choice| choice.active));
}

#[test]
fn an_off_skill_is_not_rendered_and_is_recorded_off() {
    let mut spec = base();
    spec.skills[1].activation = Activation::Off;
    let prompt = ok(&spec);

    assert!(prompt.text.contains("name=\"rust-style\""));
    assert!(
        !prompt.text.contains("name=\"command-queue\""),
        "D43: an `off` winner renders nothing"
    );
    assert_eq!(
        prompt.trim.skill_choices.len(),
        2,
        "every candidate is recorded"
    );
    assert_eq!(
        prompt.trim.skill_choices[1],
        SkillChoice {
            skill: spec.skills[1].skill_id,
            name: "command-queue".to_owned(),
            version: Some(1),
            level: SkillLevel::Project,
            activation: Activation::Off,
            active: false,
            reason: ChoiceReason::Off,
        }
    );
    assert!(prompt.trim.skill_choices[0].active);
    assert_eq!(prompt.trim.skill_choices[0].reason, ChoiceReason::Always);
}

#[test]
fn a_glob_skill_records_no_path_before_milestone_3() {
    let mut spec = base();
    spec.skills[1].activation = Activation::Glob;
    spec.skills[1].globs = vec!["**/*.rs".to_owned()];
    let prompt = ok(&spec);

    assert!(
        !prompt.text.contains("name=\"command-queue\""),
        "OQ-12: no step resolves a root before milestone 3, so a `glob` winner is inactive"
    );
    let choice = &prompt.trim.skill_choices[1];
    assert_eq!(choice.reason, ChoiceReason::NoPath);
    assert_eq!(choice.activation, Activation::Glob);
    assert!(!choice.active);
}

#[test]
fn a_missing_version_records_missing_version() {
    let mut spec = base();
    spec.skills[1].version = None;
    spec.skills[1].body = String::new();
    let prompt = ok(&spec);

    assert!(!prompt.text.contains("name=\"command-queue\""));
    let choice = &prompt.trim.skill_choices[1];
    assert_eq!(choice.reason, ChoiceReason::MissingVersion);
    assert!(!choice.active);
    assert_eq!(
        prompt.trim.to_value()["skill_choices"][1]["version"],
        serde_json::Value::Null,
        "D55: no version in force serialises `null`"
    );
}

#[test]
fn a_template_without_the_placeholder_records_not_placed_and_renders_nothing() {
    let mut spec = base();
    let body = spec.body.replace("{{skills}}\n", "");
    assert_ne!(
        body, spec.body,
        "the implement body places `{{{{skills}}}}` on its own line"
    );
    spec.body = body;
    let prompt = ok(&spec);

    assert!(!prompt.text.contains("<section name=\"skills\">"));
    assert!(
        skills_row(&prompt).is_none(),
        "no `skills` row when nothing rendered"
    );
    assert_eq!(prompt.trim.skill_choices.len(), 2);
    for choice in &prompt.trim.skill_choices {
        assert_eq!(choice.reason, ChoiceReason::NotPlaced, "{}", choice.name);
        assert!(!choice.active, "{}", choice.name);
    }
}

#[test]
fn an_off_skill_never_trips_the_cap() {
    // F-F: measure one skill's `skills` tokens, and make that exactly the cap. `phase_skills_over_
    // cap`'s own cap (100) is under either skill alone, so it cannot show the difference.
    let mut single_spec = base();
    single_spec.skills.truncate(1);
    single_spec.max_skill_tokens = i64::MAX;
    let single = skills_row(&ok(&single_spec))
        .expect("one `Always` skill renders the section")
        .tokens_before;
    assert!(single > 0);

    let mut both = base();
    both.max_skill_tokens = single;
    match assemble(&both, &scrubber()) {
        Err(AssembleError::SkillsExceedCap { tokens, cap }) => {
            assert!(tokens > single, "two skills cost more than one");
            assert_eq!(cap, single);
        }
        other => panic!("two active skills over a one-skill cap must refuse: {other:?}"),
    }

    let mut one_off = both.clone();
    one_off.skills[1].activation = Activation::Off;
    let prompt = assemble(&one_off, &scrubber())
        .unwrap_or_else(|error| panic!("D43: an `off` skill is not estimated: {error}"));
    assert_eq!(
        skills_row(&prompt)
            .expect("the active skill renders")
            .tokens_before,
        single,
        "only the active skill is estimated and meets the cap"
    );
}

#[test]
fn choices_follow_the_collapse_order_and_carry_masked_names() {
    let mut spec = base();
    spec.skills.reverse();
    // After the reverse, `skills[0]` is `command-queue` at position 1.
    spec.skills[0].name = "deploy-s3cr3t".to_owned();
    let prompt = assemble(&spec, &MinimalScrubber::new(["s3cr3t".to_owned()]))
        .unwrap_or_else(|error| panic!("the spec must assemble: {error}"));

    let names: Vec<&str> = prompt
        .trim
        .skill_choices
        .iter()
        .map(|choice| choice.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["rust-style", "deploy-[REDACTED]"],
        "choices are in collapse order `(position, name bytes)`, not input order, and masked"
    );
    let serialised = prompt.trim.to_value().to_string();
    assert!(!serialised.contains("s3cr3t"), "{serialised}");
}

#[test]
fn the_digest_moves_only_when_the_active_set_moves() {
    let mut off = base();
    off.skills[1].activation = Activation::Off;
    off.skills[1].globs = Vec::new();

    let mut off_edited = off.clone();
    off_edited.skills[1].globs = vec!["**/*.rs".to_owned()];
    off_edited.skills[1].body = "An edited body nothing renders.\n".to_owned();

    let mut glob = off.clone();
    glob.skills[1].activation = Activation::Glob;
    glob.skills[1].globs = vec!["**/*.rs".to_owned()];

    let mut always = off.clone();
    always.skills[1].activation = Activation::Always;

    let off = ok(&off);
    let off_edited = ok(&off_edited);
    let glob = ok(&glob);
    let always = ok(&always);

    assert_eq!(
        off.digest, off_edited.digest,
        "an inactive skill's globs and body are not rendered bytes"
    );
    assert_eq!(
        off.trim, off_edited.trim,
        "and not recorded either: nothing about them rendered"
    );
    assert_eq!(
        off.digest, glob.digest,
        "`off` and `glob` are both inactive before milestone 3"
    );
    assert_ne!(
        glob.trim.skill_choices, off.trim.skill_choices,
        "the record says why"
    );
    assert_ne!(
        off.digest, always.digest,
        "switching a skill on changes what renders, so the digest moves"
    );
}
