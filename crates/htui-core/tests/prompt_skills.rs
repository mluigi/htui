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
use htui_core::prompt::excerpt::RepoPath;
use htui_core::prompt::{
    AssembleError, AssembledPrompt, PromptSpec, SectionName, assemble, fixtures, glob,
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
            matched: None,
        }
    );
    assert!(prompt.trim.skill_choices[0].active);
    assert_eq!(prompt.trim.skill_choices[0].reason, ChoiceReason::Always);
}

/// MOD-9 D73/D74: a `glob` winner reads the spec's file set, and the record distinguishes the
/// three answers — matched, missed, and there-was-no-file-set. The matcher runs here so the test
/// is the wiring and not a hand-written map: `Engine::with_excerpts` and `preview::build` do
/// exactly these two lines.
fn glob_spec(files: &[(&str, &str)]) -> PromptSpec {
    let mut spec = base();
    spec.skills[1].activation = Activation::Glob;
    spec.skills[1].globs = vec!["**/*.rs".to_owned()];
    spec.skill_files = files
        .iter()
        .map(|(repo, path)| RepoPath {
            repo: (*repo).to_owned(),
            path: (*path).to_owned(),
        })
        .collect();
    spec.skill_matches = Some(glob::matched_skills(&spec.skills, &spec.skill_files));
    spec
}

#[test]
fn a_glob_skill_that_matches_renders_and_records_the_path() {
    let spec = glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]);
    let prompt = ok(&spec);
    assert!(
        prompt.text.contains("name=\"command-queue\" version=\"1\""),
        "D74: a `glob` winner the file set named is active, so it renders like an `always` one"
    );
    let choice = &prompt.trim.skill_choices[1];
    assert_eq!(choice.reason, ChoiceReason::Matched);
    assert_eq!(choice.activation, Activation::Glob);
    assert!(choice.active);
    assert_eq!(
        choice.matched.as_deref(),
        Some("htui:crates/htui-core/src/lib.rs"),
        "D74: the record names the `<repo>:<path>` that fired, so the firing is auditable"
    );
}

#[test]
fn a_glob_skill_the_file_set_missed_records_no_match_and_renders_nothing() {
    let spec = glob_spec(&[("htui", "docs/ANA-5.md")]);
    let skill = spec.skills[1].skill_id;
    let prompt = ok(&spec);

    assert!(
        !prompt.text.contains("name=\"command-queue\""),
        "a `glob` nothing matched is inactive"
    );
    let choice = &prompt.trim.skill_choices[1];
    assert_eq!(choice.reason, ChoiceReason::NoMatch);
    assert!(!choice.active);
    assert_eq!(choice.matched, None);
    assert_eq!(
        spec.skill_matches
            .as_ref()
            .expect("a file set ran")
            .get(&skill),
        None,
        "the walk ran and named nothing, which is the empty map behind `no_match`"
    );
}

#[test]
fn a_glob_skill_with_no_file_set_records_no_path() {
    let mut spec = glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]);
    spec.skill_matches = None;
    let prompt = ok(&spec);

    assert!(
        !prompt.text.contains("name=\"command-queue\""),
        "no file set, no render — the judge and the handoff both record this"
    );
    let choice = &prompt.trim.skill_choices[1];
    assert_eq!(choice.reason, ChoiceReason::NoPath);
    assert!(!choice.active);
    assert_eq!(choice.matched, None);
}

#[test]
fn the_matched_key_is_null_for_every_reason_but_the_one_that_fires() {
    let matched = ok(&glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]));
    let always = ok(&base());
    let mut off_spec = base();
    off_spec.skills[1].activation = Activation::Off;
    let off = ok(&off_spec);
    let missed = ok(&glob_spec(&[("htui", "docs/ANA-5.md")]));
    let mut no_path = glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]);
    no_path.skill_matches = None;
    let no_path = ok(&no_path);

    let fired: Vec<serde_json::Value> = [&matched, &always, &off, &missed, &no_path]
        .iter()
        .map(|prompt| prompt.trim.to_value()["skill_choices"][1]["matched"].clone())
        .collect();
    assert_eq!(
        fired[0],
        serde_json::json!("htui:crates/htui-core/src/lib.rs"),
        "the matched choice carries its path"
    );
    for (label, value) in [("no_match", &fired[3]), ("no_path", &fired[4])] {
        assert_eq!(
            value,
            &serde_json::Value::Null,
            "`{label}` records `matched: null`"
        );
    }
    assert_eq!(
        fired[2],
        serde_json::Value::Null,
        "`off` records `matched: null`"
    );
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
        "`off` and a `glob` the file set did not name both render nothing, so the bytes agree"
    );
    assert_ne!(
        glob.trim.skill_choices, off.trim.skill_choices,
        "the record says why: `off` against `no_path`"
    );
    assert_ne!(
        off.digest, always.digest,
        "switching a skill on changes what renders, so the digest moves"
    );
}

/// MOD-9 D74, the glob half of the digest rule: a `glob` skill is a rendered section, so whether
/// it fires is a change to the active set and therefore to the digest. Toggling a skill's globs
/// so it stops matching moves it; toggling one that never matched does not — the second half is
/// what keeps a matched path out of the bytes.
#[test]
fn a_glob_skill_digest_moves_when_it_stops_matching_and_not_when_it_never_matched() {
    let fired = ok(&glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]));
    let stops_matching = ok(&with_globs(
        glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]),
        "**/*.md",
    ));
    // The same walk, and two globs that both miss it: the second half of the rule, which is what
    // keeps a `glob` attachment from churning a digest no reader can audit.
    let never = with_globs(
        glob_spec(&[("htui", "crates/htui-core/src/lib.rs")]),
        "**/*.md",
    );
    let never_matched = ok(&never);
    let still_misses = ok(&with_globs(never, "**/*.txt"));

    assert_eq!(
        fired.trim.skill_choices[1].reason,
        ChoiceReason::Matched,
        "the file set named the glob, so it renders"
    );
    assert_ne!(
        fired.digest, stops_matching.digest,
        "a matched glob that stops matching leaves the active set, so the digest moves"
    );
    assert_eq!(
        stops_matching.trim.skill_choices[1].reason,
        ChoiceReason::NoMatch
    );
    assert_eq!(
        never_matched.digest, still_misses.digest,
        "toggling a glob that never matched changes no rendered byte"
    );
    assert_eq!(
        never_matched.trim.skill_choices[1].reason,
        ChoiceReason::NoMatch
    );
    assert_eq!(never_matched.trim, still_misses.trim);
}

/// Repoints a spec's `glob` skill at one pattern and re-runs the matcher, which is what a caller
/// does after an edit. Kept out of the test bodies so the assertions read as the rule, not as the
/// wiring.
fn with_globs(mut spec: PromptSpec, pattern: &str) -> PromptSpec {
    spec.skills[1].globs = vec![pattern.to_owned()];
    spec.skill_matches = Some(glob::matched_skills(&spec.skills, &spec.skill_files));
    spec
}
