//! MOD-139 milestone 3, T2: integrate's internal `premerge` and `parts` sections (plan D4, D5;
//! blueprint §1.5, §T2.3).
//!
//! An integrate session carries the isolator's pre-merge report as the internal **protected**
//! `premerge` section and the done parts it merges as the internal **trimmable** `parts` section,
//! both rendered after the `part` section and before the template body, in that order (a byte
//! contract: persona, memory, part, premerge, parts, body spans, output). `premerge` mirrors
//! `part`: its separator is the frame's and an oversized report refuses the prompt; its conflicted
//! paths are capped at twenty, in byte order, in the section and in the item note alike, which
//! are one renderer. `parts` mirrors `memory`: its separator rides with its span, and its ladder
//! (last part first) cuts bodies, then drops them, then the section, between the document group
//! and `item`. Neither placeholder can be placed by a template, neither renders outside a phase
//! spec, and a spec carrying neither renders exactly as before.
#![cfg(feature = "test-support")]

use htui_core::model::{PartMerge, PartPremerge, PremergeReport, RepoId, RepoPremerge, StepId};
use htui_core::prompt::render::{self, PREMERGE_PATH_CAP, PREMERGE_PREAMBLE, PartState};
use htui_core::prompt::{
    AssembleError, AssembledPrompt, Budget, BudgetSource, ExcerptSet, InputDocument, IntegratePart,
    PARTS_SEPARATOR, PREMERGE_SEPARATOR, PersonaBlock, Placeholder, PromptSpec, Section,
    SectionName, TemplateError, TemplateRole, TrimStrategy, assemble, fixtures, parse,
    premerge_note,
};
use htui_core::scrub::MinimalScrubber;
use uuid::Uuid;

/// The scrubber a production caller with no session secrets hands in, still fail-closed on the
/// prefix rules.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|error| panic!("the spec must assemble: {error}"))
}

/// The phase fixture in which every phase section renders, as an integrate session: the `api`
/// part's brief, the one-conflict report and the two done parts.
fn spec() -> PromptSpec {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.part = Some(fixtures::part_api());
    spec.premerge = Some(fixtures::premerge_one_conflict());
    spec.parts = fixtures::integrate_parts();
    spec
}

/// [`spec`] behind a persona, with memory and the `output` trailer: every section the order pins.
fn with_everything() -> PromptSpec {
    let mut spec = spec();
    spec.persona = Some(PersonaBlock {
        name: "reviewer".to_owned(),
        body: "You are the reviewer.\n".to_owned(),
    });
    spec.memory = Some(fixtures::memory_two_of_three());
    spec.document_tool = true;
    spec
}

/// An integrate spec with nothing trimmable ahead of `parts` in the phase order — no excerpts,
/// memory, upstream, previous diff, verification output or documents — so the `parts` ladder is
/// the first to act and its rungs are what a target selects.
fn lean() -> PromptSpec {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.excerpts = ExcerptSet::default();
    spec.upstream = Vec::new();
    spec.previous_diff = None;
    spec.verify_failure = None;
    spec.documents = Vec::new();
    spec.premerge = Some(fixtures::premerge_one_conflict());
    spec.parts = fixtures::integrate_parts();
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

/// The record's row named `name`, if the section rendered.
fn row<'a>(prompt: &'a AssembledPrompt, name: &SectionName) -> Option<&'a Section> {
    prompt.trim.sections.iter().find(|row| &row.name == name)
}

/// The record's `parts` row, which every ladder test reads.
fn parts_row(prompt: &AssembledPrompt) -> &Section {
    row(prompt, &SectionName::Parts).expect("the parts section has a row")
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

/// The `premerge` span as `substitute` writes it: the wrapped section, then its separator.
fn premerge_span(report: &PremergeReport) -> String {
    let section = render::premerge(report).expect("a report with a repo renders");
    format!("{}{PREMERGE_SEPARATOR}", render::wrap(&section))
}

/// The untrimmed `parts` span: the wrapped section, then its separator.
fn parts_span(parts: &[IntegratePart]) -> String {
    let section = render::parts(parts, &[]).expect("parts render");
    format!("{}{PARTS_SEPARATOR}", render::wrap(&section))
}

/// The byte offset of `needle` in `text`, or a panic naming it.
fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is not in the prompt:\n{text}"))
}

/// The first bytes of the `implement` body, where the template's spans start.
const BODY: &str = "You are running the `implement` phase";

/// The frame's own estimate: the template's literal text, normalised.
fn template_tokens(spec: &PromptSpec, separators: &str) -> i64 {
    let parsed = parse(spec.role, &spec.body).expect("the fixture body parses");
    spec.estimator
        .estimate(&format!("{}{separators}", render::template_text(&parsed)))
}

/// `n` numbered prose lines about `topic`, LF-terminated.
fn prose(topic: &str, n: usize) -> String {
    (0..n)
        .map(|line| {
            format!("{topic} line {line}: the integrate step merges every part in order.\n")
        })
        .collect()
}

/// The largest target in `low..=high` at which `holds` is true of the assembled prompt, by
/// bisection: `holds` must be true at `low` and false at `high`, and monotone between.
fn largest_target(
    spec: &PromptSpec,
    (mut low, mut high): (i64, i64),
    holds: impl Fn(&AssembledPrompt) -> bool,
) -> i64 {
    assert!(holds(&ok(&at_target(spec, low))), "true at the low end");
    assert!(!holds(&ok(&at_target(spec, high))), "false at the high end");
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if holds(&ok(&at_target(spec, mid))) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

#[test]
fn the_premerge_and_parts_placeholders_are_internal() {
    for (placeholder, token) in [
        (Placeholder::Premerge, "premerge"),
        (Placeholder::Parts, "parts"),
    ] {
        assert_eq!(placeholder.token(), token);
        assert_eq!(Placeholder::from_token(token), None);
        assert!(!Placeholder::ALL.contains(&placeholder));
        assert!(placeholder.is_section());
        let body = format!("{{{{{token}}}}}");
        for role in [
            TemplateRole::Phase,
            TemplateRole::Judge,
            TemplateRole::Handoff,
        ] {
            assert!(
                !placeholder.allowed_in(role),
                "no {role:?} body may place `{body}`"
            );
            assert!(
                matches!(
                    parse(role, &body),
                    Err(TemplateError::UnknownPlaceholder { token: ref t, .. }) if t == token
                ),
                "`{body}` is unknown to a {role:?} body"
            );
        }
        let mut spec = spec();
        spec.body = body;
        assert_eq!(
            assemble(&spec, &scrubber()),
            Err(AssembleError::UnknownPlaceholder {
                token: token.to_owned()
            })
        );
    }
    assert_eq!(
        Placeholder::ALL.len(),
        20,
        "the internal placeholders are not listed"
    );
}

#[test]
fn integrate_sections_render_in_the_pinned_order() {
    let spec = with_everything();
    let prompt = ok(&spec);
    let premerge = premerge_span(&fixtures::premerge_one_conflict());
    let parts = parts_span(&fixtures::integrate_parts());
    for (form, text) in [("text", &prompt.text), ("digest_text", &prompt.digest_text)] {
        let persona = at(text, "<section name=\"persona\"");
        let memory = at(text, "<section name=\"memory\"");
        let part = at(text, "<section name=\"part\"");
        let premerge_at = at(text, &premerge);
        let parts_at = at(text, &parts);
        let body = at(text, BODY);
        let output = at(text, "<section name=\"output\"");
        assert_eq!(
            persona, 0,
            "the persona frame still comes first in `{form}`"
        );
        assert!(persona < memory, "persona, then memory in `{form}`");
        assert!(memory < part, "memory, then the part in `{form}`");
        assert!(
            part < premerge_at,
            "the part, then the premerge in `{form}`"
        );
        assert_eq!(
            premerge_at + premerge.len(),
            parts_at,
            "the premerge and its blank line, then the parts in `{form}`"
        );
        assert_eq!(
            parts_at + parts.len(),
            body,
            "the parts and their blank line, then the body in `{form}`"
        );
        assert!(
            body < output,
            "the body, then the output trailer in `{form}`"
        );
        for name in ["premerge", "parts"] {
            assert_eq!(
                text.matches(&format!("<section name=\"{name}\"")).count(),
                1,
                "`{name}` rendered once in `{form}`"
            );
        }
    }
}

#[test]
fn the_integrate_sections_are_recorded_in_render_order() {
    let prompt = ok(&with_everything());
    let names: Vec<SectionName> = prompt.sections.iter().map(|row| row.name.clone()).collect();
    assert_eq!(
        names[..6],
        [
            SectionName::Template,
            SectionName::Persona,
            SectionName::Memory,
            SectionName::Part,
            SectionName::Premerge,
            SectionName::Parts,
        ],
        "D5: premerge and parts are recorded after the part, in render order"
    );
    assert_eq!(
        names.last(),
        Some(&SectionName::Output),
        "the trailer is last"
    );
    for name in [SectionName::Premerge, SectionName::Parts] {
        assert_eq!(
            names.iter().filter(|recorded| **recorded == name).count(),
            1,
            "{name} is recorded once"
        );
    }
    assert_eq!(prompt.trim.sections, prompt.sections);
    let payload: Vec<String> = prompt
        .payload_sections()
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(payload[4..6], ["premerge".to_owned(), "parts".to_owned()]);
}

#[test]
fn the_premerge_separator_is_counted_in_the_frame() {
    assert_eq!(PREMERGE_SEPARATOR, "\n\n");
    let mut spec = fixtures::phase_implement_attempt2();
    spec.premerge = Some(fixtures::premerge_one_conflict());
    let with = ok(&spec);
    assert_eq!(
        with.sections[0].tokens_before,
        template_tokens(&spec, PREMERGE_SEPARATOR),
        "the separator is the frame's, as the part's is"
    );

    spec.premerge = None;
    spec.parts = fixtures::integrate_parts();
    let without = ok(&spec);
    assert_eq!(
        without.sections[0].tokens_before,
        template_tokens(&spec, ""),
        "no premerge, no frame separator: the parts' rides with its own span"
    );
}

#[test]
fn the_parts_separator_rides_with_its_section() {
    assert_eq!(PARTS_SEPARATOR, "\n\n");
    let spec = lean();
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let est = spec.estimator;
    let row = parts_row(&full).clone();
    assert_eq!(
        row.tokens_before,
        est.estimate(&parts_span(&spec.parts)),
        "the parts row pays for its wrapper and its blank line"
    );
    assert_eq!(
        full.sections[0].tokens_before,
        template_tokens(&spec, PREMERGE_SEPARATOR),
        "and the frame does not"
    );

    // Dropped whole, the separator goes with it: the prompt is the parts-less one, byte for byte.
    let dropped = ok(&at_target(
        &spec,
        full.trim.estimated_after - row.tokens_before,
    ));
    assert_eq!(parts_row(&dropped).tokens_after, 0);
    let mut none = spec.clone();
    none.parts = Vec::new();
    let without = ok(&at_target(&none, i64::MAX / 4));
    assert_eq!(dropped.text, without.text, "no blank line is left behind");
    let premerge = premerge_span(&fixtures::premerge_one_conflict());
    assert_eq!(
        at(&dropped.text, &premerge) + premerge.len(),
        at(&dropped.text, BODY),
        "the premerge's blank line, then the body"
    );
}

#[test]
fn an_oversized_premerge_refuses_the_prompt() {
    let parts = (0..400)
        .map(|n| PartPremerge {
            step: StepId::from_uuid(Uuid::from_u128(0x7000 + n)),
            key: format!("part-{n:03}"),
            tip: Some(format!("{:012x}", 0xa000_0000_u128 + n)),
            merge: PartMerge::Merged {
                commit: format!("{:012x}", 0xb000_0000_u128 + n),
            },
        })
        .collect();
    let mut spec = fixtures::phase_implement_attempt2();
    spec.premerge = Some(PremergeReport {
        repos: vec![RepoPremerge {
            repo_id: RepoId::from_uuid(Uuid::from_u128(0xc04e)),
            name: "core".to_owned(),
            head: "4e5f6a7b8c9d".to_owned(),
            parts,
        }],
    });

    let full = ok(&at_target(&spec, i64::MAX / 4));
    let premerge = row(&full, &SectionName::Premerge)
        .expect("the premerge section has a row")
        .clone();
    assert!(premerge.tokens_before > 1_000, "the report is large");
    let floor = protected_floor(&full);

    // At the protected floor everything trimmable goes first; the report loses nothing.
    let prompt = ok(&at_target(&spec, floor));
    assert_eq!(row(&prompt, &SectionName::Premerge), Some(&premerge));
    assert_eq!(premerge.strategy, TrimStrategy::None);
    assert!(!premerge.trimmed);

    // A target the protected set alone is over: refused, never trimmed.
    for target in [floor - 1, premerge.tokens_before] {
        let error = assemble(&at_target(&spec, target), &scrubber())
            .expect_err("the protected set alone is over target");
        let AssembleError::BudgetTooSmall { needed, target: t } = error else {
            panic!("expected BudgetTooSmall, got {error:?}");
        };
        assert_eq!(t, target);
        assert_eq!(needed, floor, "the report's tokens are in `needed`");
    }
}

#[test]
fn conflicted_paths_are_capped_at_twenty_in_byte_order() {
    let report = fixtures::premerge_one_conflict();
    let PartMerge::Conflicted { paths } = &report.repos[0].parts[1].merge else {
        panic!("the fixture's `ui` part conflicted");
    };
    assert_eq!(paths.len(), 22);
    let mut sorted = paths.clone();
    sorted.sort();
    assert_ne!(
        *paths, sorted,
        "the fixture hands the paths in out of order"
    );

    let mut listed: Vec<String> = sorted
        .iter()
        .take(PREMERGE_PATH_CAP)
        .map(|path| format!("  - {path}"))
        .collect();
    listed.push("  - and 2 more".to_owned());
    let block = format!(
        "- part `ui` d4e5f6a7b8c9: conflicted, merge aborted; conflicted paths:\n{}\n- part `db` \
         0f1e2d3c4b5a: not attempted, an earlier part conflicted",
        listed.join("\n")
    );

    let mut spec = fixtures::phase_implement_attempt2();
    spec.premerge = Some(report.clone());
    let prompt = ok(&spec);
    let note = premerge_note("implement", 1, &report).expect("a report with a repo has a note");
    for (form, text) in [
        ("text", prompt.text.as_str()),
        ("digest_text", prompt.digest_text.as_str()),
        ("note", note.as_str()),
    ] {
        assert!(
            text.contains(&block),
            "the capped block in `{form}`:\n{text}"
        );
        for past in &sorted[PREMERGE_PATH_CAP..] {
            assert!(
                !text.contains(past.as_str()),
                "`{past}` is past the cap in `{form}`"
            );
        }
    }
}

#[test]
fn the_parts_ladder_cuts_the_last_body_first() {
    let spec = lean();
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let prompt = ok(&at_target(&spec, full.trim.estimated_after - 50));

    let row = parts_row(&prompt);
    assert_eq!(row.strategy, TrimStrategy::HeadTail);
    assert!(row.trimmed);
    assert!(row.elided_lines.is_some_and(|lines| lines > 0), "{row:?}");
    assert!(row.elided_bytes.is_some_and(|bytes| bytes > 0), "{row:?}");
    assert_eq!(row.dropped, None);

    let text = &prompt.text;
    let api = fixtures::integrate_parts()[0]
        .body
        .clone()
        .expect("the api part has a document");
    assert!(
        text.contains(api.trim_end_matches('\n')),
        "the first part keeps its whole body"
    );
    let ui = at(text, "<part key=\"ui\"");
    let marker = at(text, "[... htui elided ");
    assert!(ui < marker, "the cut is in the last part's body");
    assert_eq!(text.matches("[... htui elided ").count(), 1, "one cut");
    assert!(text.contains("ui impl line 0:"), "the head survives");
    assert!(text.contains("ui impl line 299:"), "the tail survives");
    assert!(!text.contains("ui impl line 150:"), "the middle went");
    assert_eq!(
        row_strategy(&prompt, &SectionName::Item),
        TrimStrategy::None,
        "the item is never reached"
    );
}

/// The strategy of the row named `name`.
fn row_strategy(prompt: &AssembledPrompt, name: &SectionName) -> TrimStrategy {
    row(prompt, name)
        .unwrap_or_else(|| panic!("{name} has a row"))
        .strategy
}

#[test]
fn the_parts_ladder_drops_bodies_last_first_and_keeps_tips_and_stat() {
    let spec = lean();
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let total = full.trim.estimated_after;
    let section = parts_row(&full).tokens_before;
    // The largest target at which any body went: the first body dropped cleared it.
    let target = largest_target(&spec, (total - section, total), |prompt| {
        parts_row(prompt).dropped.is_some()
    });
    let prompt = ok(&at_target(&spec, target));

    let row = parts_row(&prompt);
    assert_eq!(row.dropped, Some(1), "one body went");
    assert_eq!(
        row.strategy,
        TrimStrategy::Dropped,
        "the last rung's strategy wins"
    );
    assert!(row.trimmed);
    assert!(row.tokens_after > 0, "the section stays");

    let text = &prompt.text;
    assert!(
        text.contains(
            "<part key=\"ui\" title=\"The Runs pane\">\ntip `core` d4e5f6a7b8c9\n 5 files \
             changed, 260 insertions(+), 31 deletions(-)\n</part>"
        ),
        "the last part is bare: key, title, tip and stat\n{text}"
    );
    assert!(!text.contains("ui impl line"), "its body went");
    assert!(
        text.contains("api impl line 0:"),
        "the first part keeps its (cut) body"
    );
}

#[test]
fn the_parts_ladder_drops_the_section_last() {
    let spec = lean();
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let section = parts_row(&full).tokens_before;
    let prompt = ok(&at_target(&spec, full.trim.estimated_after - section));

    let row = parts_row(&prompt);
    assert_eq!(row.tokens_after, 0);
    assert_eq!(row.strategy, TrimStrategy::Dropped);
    assert!(row.trimmed);
    assert_eq!(row.dropped, Some(2), "both bodies went before the section");
    assert!(!prompt.text.contains("<section name=\"parts\""));
    assert!(
        prompt.text.contains("<section name=\"premerge\">"),
        "the report is protected"
    );
    assert_eq!(
        row_strategy(&prompt, &SectionName::Item),
        TrimStrategy::None
    );
}

#[test]
fn parts_trim_after_documents_and_before_item() {
    let mut spec = lean();
    spec.item_body = prose("item", 400);
    spec.documents = vec![InputDocument {
        kind: "plan".to_owned(),
        version: 2,
        body: prose("plan", 300),
    }];
    let full = ok(&at_target(&spec, i64::MAX / 4));
    let total = full.trim.estimated_after;
    let plan = SectionName::Documents("plan".to_owned());

    // A small deficit: the document gives it, the parts and the item are untouched.
    let small = ok(&at_target(&spec, total - 50));
    assert_eq!(row_strategy(&small, &plan), TrimStrategy::HeadTail);
    assert_eq!(
        row_strategy(&small, &SectionName::Parts),
        TrimStrategy::None
    );
    assert_eq!(row_strategy(&small, &SectionName::Item), TrimStrategy::None);

    // A deficit the document cannot cover: it floors first, the parts give the rest, and the
    // item — last in the phase order — is never reached.
    let document = row(&full, &plan)
        .expect("the document has a row")
        .tokens_before;
    let large = ok(&at_target(&spec, total - document));
    let plan_row = row(&large, &plan).expect("the document has a row");
    assert_eq!(plan_row.strategy, TrimStrategy::HeadTail);
    assert_eq!(
        plan_row.elided_lines,
        Some(225),
        "300 lines down to the 75-line floor"
    );
    assert_eq!(
        row_strategy(&large, &SectionName::Parts),
        TrimStrategy::HeadTail
    );
    assert_eq!(row_strategy(&large, &SectionName::Item), TrimStrategy::None);
}

#[test]
fn premerge_and_parts_secrets_are_masked() {
    const SECRET: &str = "hunter2";
    let mut report = fixtures::premerge_one_conflict();
    if let PartMerge::Conflicted { paths } = &mut report.repos[0].parts[1].merge {
        paths[0] = format!("crates/{SECRET}.rs");
    }
    let mut parts = fixtures::integrate_parts();
    parts[1].title = format!("The Runs pane {SECRET}");
    parts[0].body = Some(format!("The key is {SECRET}.\n"));
    let mut spec = lean();
    spec.premerge = Some(report.clone());
    spec.parts = parts.clone();

    let prompt = assemble(&spec, &MinimalScrubber::new([SECRET.to_owned()]))
        .expect("a maskable secret does not refuse the prompt");
    for (form, text) in [("text", &prompt.text), ("digest_text", &prompt.digest_text)] {
        assert!(!text.contains(SECRET), "the secret survived in `{form}`");
        for masked in [
            "  - crates/[REDACTED].rs",
            "title=\"The Runs pane [REDACTED]\"",
            "The key is [REDACTED].",
        ] {
            assert!(text.contains(masked), "`{masked}` in `{form}`");
        }
    }

    // A pattern-rule secret is refused under the name of the section that would render it.
    const PATTERN: &str = "sk-ant-api03-DEADBEEFDEADBEEFDEADBEEF";
    let mut refused = spec.clone();
    if let Some(PartMerge::Conflicted { paths }) = refused
        .premerge
        .as_mut()
        .map(|report| &mut report.repos[0].parts[1].merge)
    {
        paths[0] = format!("crates/{PATTERN}.rs");
    }
    let mut body = spec.clone();
    body.parts[0].body = Some(format!("Use {PATTERN}.\n"));
    for (spec, section) in [(refused, "premerge"), (body, "parts")] {
        let error = assemble(&spec, &scrubber()).expect_err("a pattern-rule secret is refused");
        let AssembleError::Unmasked {
            section: ref name, ..
        } = error
        else {
            panic!("expected Unmasked, got {error:?}");
        };
        assert_eq!(name, section);
        assert!(!error.to_string().contains("DEADBEEF"), "{error}");
    }
}

#[test]
fn only_a_phase_spec_renders_premerge_and_parts() {
    for mut spec in [
        fixtures::judge_three_candidates(),
        fixtures::handoff_basic(),
    ] {
        let before = ok(&spec);
        // A pattern secret in each: neither is masked, scanned or rendered for this role.
        let mut report = fixtures::premerge_one_conflict();
        report.repos[0].name = "sk-ant-api03-DEADBEEFDEADBEEFDEADBEEF".to_owned();
        let mut parts = fixtures::integrate_parts();
        parts[0].body = Some("sk-ant-api03-DEADBEEFDEADBEEFDEADBEEF".to_owned());
        spec.premerge = Some(report);
        spec.parts = parts;
        let with = ok(&spec);
        for name in ["premerge", "parts"] {
            assert!(
                !with.text.contains(&format!("name=\"{name}\"")),
                "a {:?} spec renders no `{name}`",
                spec.role
            );
        }
        assert!(row(&with, &SectionName::Premerge).is_none());
        assert!(row(&with, &SectionName::Parts).is_none());
        assert_eq!(with, before, "a {:?} spec ignores both", spec.role);
    }

    let empty = fixtures::phase_all_empty();
    assert_eq!(empty.premerge, None);
    assert!(empty.parts.is_empty());
    let prompt = ok(&empty);
    assert!(row(&prompt, &SectionName::Premerge).is_none());
    assert!(row(&prompt, &SectionName::Parts).is_none());
    assert_eq!(
        prompt.sections[0].tokens_before,
        template_tokens(&empty, ""),
        "I-7: without either, the frame's estimate is today's"
    );
}

#[test]
fn a_part_with_no_document_renders_its_header_and_stat() {
    let mut parts = fixtures::integrate_parts();
    parts[0].body = None;
    parts.push(IntegratePart {
        key: "db".to_owned(),
        title: "The schema".to_owned(),
        tips: Vec::new(),
        diff: None,
        body: None,
    });
    let api = "<part key=\"api\" title=\"The HTTP API\">\ntip `core` a1b2c3d4e5f6\n 3 files \
               changed, 120 insertions(+), 4 deletions(-)\n</part>";
    let db = "<part key=\"db\" title=\"The schema\">\nno commits\n</part>";

    let section = render::parts(&parts, &[]).expect("parts render");
    assert!(section.content.contains(api), "{}", section.content);
    assert!(section.content.contains(db), "{}", section.content);

    let mut spec = lean();
    spec.parts = parts;
    let prompt = ok(&spec);
    assert!(prompt.text.contains(api), "{}", prompt.text);
    assert!(prompt.text.contains(db), "{}", prompt.text);
    assert!(!prompt.text.contains("api impl line"), "no body");
    assert!(
        prompt.text.contains("ui impl line 0:"),
        "the other part keeps its body"
    );
}

#[test]
fn a_report_with_no_repo_renders_no_premerge_section() {
    let report = PremergeReport::default();
    assert_eq!(render::premerge(&report), None);
    assert_eq!(premerge_note("implement", 1, &report), None);

    let mut spec = fixtures::phase_implement_attempt2();
    let before = ok(&spec);
    spec.premerge = Some(report);
    let prompt = ok(&spec);
    assert!(row(&prompt, &SectionName::Premerge).is_none());
    assert!(!prompt.text.contains("name=\"premerge\""));
    assert_eq!(
        prompt.sections[0].tokens_before,
        template_tokens(&spec, ""),
        "no section, no frame separator"
    );
    assert_eq!(prompt, before, "an empty report renders as no report");
}

#[test]
fn the_premerge_note_is_the_section_text() {
    let report = fixtures::premerge_one_conflict();
    let section = render::premerge(&report).expect("a report with a repo renders");
    assert_eq!(section.name, SectionName::Premerge);
    assert!(section.attrs.is_empty(), "no attributes");
    assert_eq!(section.content, render::premerge_content(&report));
    assert_eq!(
        premerge_note("implement", 1, &report),
        Some(format!(
            "premerge `implement` attempt 1:\n{}",
            section.content
        ))
    );

    // The bytes, pinned: the preamble, the repo line, one line per part; no step id anywhere.
    let paths: Vec<String> = (0..20)
        .map(|n| format!("  - crates/htui/src/tabs/runs/f{n:02}.rs"))
        .collect();
    let expected = format!(
        "{PREMERGE_PREAMBLE}\nrepo `core` at 4e5f6a7b8c9d:\n- part `api` a1b2c3d4e5f6: merged as \
         9a8b7c6d5e4f\n- part `ui` d4e5f6a7b8c9: conflicted, merge aborted; conflicted \
         paths:\n{}\n  - and 2 more\n- part `db` 0f1e2d3c4b5a: not attempted, an earlier part \
         conflicted",
        paths.join("\n")
    );
    assert_eq!(section.content, expected);
    for part in &report.repos[0].parts {
        assert!(
            !section.content.contains(&part.step.to_string()),
            "P-9: no step id is rendered"
        );
    }

    // The two outcomes the fixture does not carry.
    let other = PremergeReport {
        repos: vec![RepoPremerge {
            repo_id: RepoId::from_uuid(Uuid::from_u128(0xd0c5)),
            name: "docs".to_owned(),
            head: "123456789abc".to_owned(),
            parts: vec![
                PartPremerge {
                    step: StepId::from_uuid(Uuid::from_u128(0x5b01)),
                    key: "api".to_owned(),
                    tip: Some("a1b2c3d4e5f6".to_owned()),
                    merge: PartMerge::Contained,
                },
                PartPremerge {
                    step: StepId::from_uuid(Uuid::from_u128(0x5b02)),
                    key: "ui".to_owned(),
                    tip: None,
                    merge: PartMerge::NoCommits,
                },
            ],
        }],
    };
    assert_eq!(
        render::premerge_content(&other),
        format!(
            "{PREMERGE_PREAMBLE}\nrepo `docs` at 123456789abc:\n- part `api` a1b2c3d4e5f6: \
             already contained\n- part `ui`: no commits"
        )
    );
    assert_eq!(
        render::parts(
            &fixtures::integrate_parts(),
            &[PartState::Bare, PartState::Bare]
        )
        .map(|section| section.content.contains("impl line")),
        Some(false),
        "a bare block carries no body"
    );
}
