//! What a golden snapshot cannot hold: the assembler's invariants (ANA-5 §12 criteria 3, 5, 6, 8,
//! 9, 10, 15, 17, 18).
//!
//! Deliberately snapshot-free (blueprint H-11). Every assertion here is a property — two runs agree,
//! a projection equals its source, an arithmetic identity holds, a refusal fires before anything
//! else happens — because an `insta` corpus can absorb a real change by being re-accepted, and
//! these are the facts that must not be re-acceptable. `prompt_render.rs` holds the bytes; this
//! file holds the rules.
#![cfg(feature = "test-support")]

use std::collections::BTreeSet;

use htui_core::prompt::render::elision_marker;
use htui_core::prompt::{
    AssembleError, AssembledPrompt, PromptSpec, SectionName, TrimStrategy, assemble, fixtures,
};
use htui_core::scrub::MinimalScrubber;

/// The scrubber every test passes: no configured secrets, and still fail-closed on the prefix
/// rules (`scrub.rs:114-120`), which is exactly what a production caller with no session secrets
/// hands in.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error, which is what a failing criterion should print.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|e| panic!("the fixture must assemble: {e}"))
}

/// The record's strategy for one section name, or `None` when it has no entry at all.
fn strategy_of(prompt: &AssembledPrompt, name: &SectionName) -> Option<TrimStrategy> {
    prompt
        .trim
        .sections
        .iter()
        .find(|section| &section.name == name)
        .map(|section| section.strategy)
}

// ---------------------------------------------------------------------------------------------
// Criterion 5 — purity and fan-out identity
// ---------------------------------------------------------------------------------------------

#[test]
fn same_spec_same_bytes_twice() {
    // Two independently constructed specs, so a `HashMap` iteration order anywhere in the render
    // path would show up as a digest difference rather than as a flake (blueprint H-3).
    let first = ok(&fixtures::phase_implement_attempt2());
    let second = ok(&fixtures::phase_implement_attempt2());
    assert_eq!(first.text, second.text);
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.trim, second.trim);
    assert_eq!(first.digest.len(), 64, "lowercase sha256 hex, all 64");
}

#[test]
fn fan_out_siblings_get_identical_bytes() {
    // Criterion 5. `PromptSpec` carries no `fanout_index`, no `RunId` and no `StepId`, so the three
    // siblings are the same call three times — that *is* the design (§4.7 rule 8: the property is
    // enforced by the type, not by the code). The greps are what make the claim checkable rather
    // than tautological: they look for the values a caller could have leaked through a body.
    let spec = fixtures::phase_implement_attempt2();
    let siblings: Vec<AssembledPrompt> = (0..3).map(|_| ok(&spec)).collect();
    assert_eq!(siblings[0].digest, siblings[1].digest);
    assert_eq!(siblings[1].digest, siblings[2].digest);
    assert_eq!(siblings[0].text, siblings[2].text);

    let text = &siblings[0].text;
    // The three values two siblings differ by, spelled the way a leak would spell them.
    let run_id = "01a06490-eea0-7000-8000-0000000000aa";
    let step_id = "01a06490-eea0-7000-8000-0000000000bb";
    let tree_root = format!("/home/htui/.local/share/htui/trees/{run_id}/{step_id}");
    for leak in [run_id, step_id, tree_root.as_str(), "fanout_index"] {
        assert!(
            !text.contains(leak),
            "§4.7 rule 8: `{leak}` must not reach a Phase prompt"
        );
    }
    // Absolute paths, POSIX and Windows, wherever they sit on a line.
    for prefix in ["/home/", "/tmp/", "/var/", "/Users/", "\\\\?\\"] {
        assert!(
            !text.contains(prefix),
            "§4.2 rule 5: no absolute path — found `{prefix}`"
        );
    }
    for line in text.lines() {
        let bytes = line.as_bytes();
        let drive_letter = bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(1) == Some(&b':')
            && bytes.get(2) == Some(&b'\\');
        assert!(!drive_letter, "a Windows absolute path on `{line}`");
        assert!(!line.starts_with('/'), "a POSIX absolute path on `{line}`");
    }
}

#[test]
fn the_prompt_module_reads_no_clock() {
    // §4.7 rule 7, as a source-level fact rather than a behavioural one: a clock read that only
    // fires on one branch would pass every behavioural test and still break reproducibility.
    const SOURCES: [(&str, &str); 9] = [
        ("mod.rs", include_str!("../src/prompt/mod.rs")),
        ("template.rs", include_str!("../src/prompt/template.rs")),
        ("estimate.rs", include_str!("../src/prompt/estimate.rs")),
        ("render.rs", include_str!("../src/prompt/render.rs")),
        ("digest.rs", include_str!("../src/prompt/digest.rs")),
        ("trim.rs", include_str!("../src/prompt/trim.rs")),
        ("settings.rs", include_str!("../src/prompt/settings.rs")),
        ("excerpt.rs", include_str!("../src/prompt/excerpt.rs")),
        ("defaults.rs", include_str!("../src/prompt/defaults.rs")),
    ];
    for (name, source) in SOURCES {
        // A clock read is forbidden everywhere, tests included: there is no legitimate `now()` in
        // a module whose output must be reproducible.
        for forbidden in ["Utc::now", "Local::now", "SystemTime", "Instant::now"] {
            assert!(
                !source.contains(forbidden),
                "`prompt/{name}` names `{forbidden}`: §4.7 rule 7 puts no wall-clock value in the \
                 digested text"
            );
        }
        // The rest bind the shipped path only. A `#[cfg(test)]` module may build a fixed
        // `DateTime` to feed a renderer — that is a literal, not a clock — and the cut is made at
        // the attribute rather than by exempting a file, so a new test module inherits the rule.
        let shipped = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(shipped, _)| shipped);
        for forbidden in ["use chrono", "std::env", "HashMap", "HashSet"] {
            assert!(
                !shipped.contains(forbidden),
                "`prompt/{name}` names `{forbidden}`: invariant 2 says the assembler reads no \
                 clock, no environment and no unordered map"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Criterion 6 — line endings
// ---------------------------------------------------------------------------------------------

#[test]
fn crlf_inputs_digest_identically() {
    for spec in [
        fixtures::phase_implement_attempt2(),
        fixtures::judge_three_candidates(),
        fixtures::handoff_basic(),
    ] {
        let lf = ok(&spec);
        let crlf = ok(&fixtures::with_crlf(&spec));
        assert_eq!(
            lf.digest, crlf.digest,
            "criterion 6: `{}` digests the same in either encoding",
            spec.template.name
        );
        assert_eq!(lf.text, crlf.text);
        assert!(
            !crlf.text.contains('\r'),
            "the text handed to `AgentDriver::start` carries no CR"
        );
        assert_eq!(
            lf.trim.sections, crlf.trim.sections,
            "and every token figure and elision count is over LF text (P-2)"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Criterion 8 — one map, and only one
// ---------------------------------------------------------------------------------------------

#[test]
fn payload_sections_are_the_records_projection() {
    for spec in [
        fixtures::phase_implement_attempt2(),
        fixtures::phase_all_empty(),
        fixtures::phase_oversize(),
        fixtures::judge_three_candidates(),
        fixtures::handoff_basic(),
    ] {
        let prompt = ok(&spec);
        let entries = prompt.payload_sections();
        assert_eq!(
            entries.len(),
            prompt.trim.sections.len(),
            "one entry per record row, for `{}`",
            spec.template.name
        );
        for (entry, section) in entries.iter().zip(&prompt.trim.sections) {
            assert_eq!(entry.name, section.name.render());
            assert_eq!(entry.tokens, section.tokens_after);
            assert_eq!(entry.trimmed, section.trimmed);
        }
        assert_eq!(
            prompt.sections, prompt.trim.sections,
            "`AssembledPrompt::sections` is the record's own vector, not a second copy"
        );
        // The payload form the recorder is handed is the same projection, serialised.
        let value = prompt.payload_sections_value();
        assert_eq!(
            value,
            serde_json::to_value(&entries).expect("plain data"),
            "§5.2's array and `section_entries()` are one function's output"
        );
        assert_eq!(
            prompt.trim.sections[0].name,
            SectionName::Template,
            "P-9: `template` is always the first entry"
        );
    }
}

#[test]
fn the_only_section_entry_constructor_is_the_records_projection() {
    // ANA-5 risk 12: the payload array and the record drift the moment a second place builds an
    // entry. `trim.rs` is the one place, and this is the grep that keeps it so.
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/htui-core has a parent")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![crates.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable crate directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).expect("a readable source file");
                if source.contains("SectionEntry {")
                    && !path.ends_with("prompt/trim.rs")
                    && !path.ends_with("tests/prompt_digest.rs")
                {
                    offenders.push(path.display().to_string());
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "`SectionEntry` is constructed outside `prompt/trim.rs`: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 9 — the trim, its arithmetic and its markers
// ---------------------------------------------------------------------------------------------

#[test]
fn the_oversize_fixture_lands_at_or_under_target() {
    let spec = fixtures::phase_oversize();
    let prompt = ok(&spec);
    let record = &prompt.trim;
    let target = spec.budget.target();

    let ratio = record.estimated_before as f64 / spec.budget.tokens as f64;
    assert!(
        (1.4..=1.6).contains(&ratio),
        "the fixture is meant to be half again over budget, measured {ratio:.2}× \
         ({} before, {} budget)",
        record.estimated_before,
        spec.budget.tokens
    );
    assert!(
        record.estimated_after <= target,
        "criterion 9: {} must land at or under {target}",
        record.estimated_after
    );
    assert_eq!(record.target, target);
    assert_eq!(record.budget, spec.budget.tokens);
    assert_eq!(record.estimator, spec.estimator.id);

    // §5.1's arithmetic invariant: the rows sum to the two totals.
    let before: i64 = record.sections.iter().map(|s| s.tokens_before).sum();
    let after: i64 = record.sections.iter().map(|s| s.tokens_after).sum();
    assert_eq!(before, record.estimated_before);
    assert_eq!(after, record.estimated_after);

    // The protected set never loses a token, whatever the deficit (§4.4, invariant 4).
    for name in [
        SectionName::Template,
        SectionName::Box,
        SectionName::Skills,
        SectionName::CommandQueue,
    ] {
        let section = record
            .sections
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("`{name}` has an entry"));
        assert_eq!(
            section.strategy,
            TrimStrategy::None,
            "`{name}` is protected"
        );
        assert!(!section.trimmed, "`{name}` is protected");
        assert_eq!(section.tokens_before, section.tokens_after);
    }

    // The deficit is cleared in §4.4's order and stops the moment it reaches zero, so the sections
    // at or before the clearing point are trimmed and the ones after it are untouched.
    assert_eq!(
        strategy_of(&prompt, &SectionName::Excerpts),
        Some(TrimStrategy::Dropped),
        "rank 1 goes first"
    );
    // §4.4 step 7: a section that reaches its floor without clearing the deficit moves to the drop,
    // and the pass moves on only after it. The rungs each one passed through are still recorded —
    // `upstream` kept its ladder counters — and the intermediate states are proved on their own,
    // below, by clearing the deficit inside one ladder.
    assert_eq!(
        strategy_of(&prompt, &SectionName::Upstream),
        Some(TrimStrategy::Dropped)
    );
    let upstream = record
        .sections
        .iter()
        .find(|s| s.name == SectionName::Upstream)
        .expect("an upstream entry");
    assert_eq!(
        upstream.stubbed,
        Some(3),
        "three summaries degraded on the way"
    );
    assert_eq!(upstream.dropped, Some(2), "and both depth-2 rows went");
    assert_eq!(
        strategy_of(&prompt, &SectionName::PreviousDiff),
        Some(TrimStrategy::Dropped)
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::VerifyFailure),
        Some(TrimStrategy::Dropped),
        "ten lines cannot reach a 200-line floor, so the ladder moves straight to the drop"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::Documents("plan".to_owned())),
        Some(TrimStrategy::HeadTail),
        "the residual is taken from the first document"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::Documents("review".to_owned())),
        Some(TrimStrategy::None),
        "and the deficit was cleared before the second one"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::Item),
        Some(TrimStrategy::None),
        "`item` is trimmed last of all and is never reached here"
    );

    // A dropped section keeps an entry saying so, and contributes no bytes.
    for name in [SectionName::Excerpts, SectionName::VerifyFailure] {
        let section = record
            .sections
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("`{name}` has an entry"));
        assert_eq!(section.tokens_after, 0);
        assert!(section.trimmed);
    }
    assert!(
        !prompt.text.contains("<section name=\"excerpts\">"),
        "a dropped section leaves no wrapper behind"
    );
    assert!(!prompt.text.contains("<section name=\"verify_failure\""));
}

/// The oversize corpus against a chosen `target`, so a test can put the deficit's clearing point
/// inside whichever ladder it means to exercise. `reserve_bp: 0` makes `target == tokens`, which
/// keeps the arithmetic of each case readable.
fn oversize_at(target: i64) -> PromptSpec {
    let mut spec = fixtures::phase_oversize();
    spec.budget = htui_core::prompt::Budget {
        tokens: target,
        source: htui_core::prompt::BudgetSource::Project,
        reserve_bp: 0,
    };
    spec
}

/// What the corpus estimates at before anything is trimmed.
fn untrimmed_total() -> i64 {
    ok(&oversize_at(i64::MAX / 4)).trim.estimated_before
}

#[test]
fn the_upstream_ladder_degrades_before_it_drops() {
    // §4.3's depth ladder, stopped mid-rung: the deficit clears while rows are still one-liners, so
    // the section survives with its `stubbed` count — which is what `R-PRM-2`'s stubs look like
    // under pressure.
    let spec = oversize_at(untrimmed_total() - 13_000);
    let prompt = ok(&spec);
    assert_eq!(
        strategy_of(&prompt, &SectionName::Excerpts),
        Some(TrimStrategy::Dropped),
        "the section ahead of it is exhausted first"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::Upstream),
        Some(TrimStrategy::StubLadder)
    );
    let upstream = prompt
        .trim
        .sections
        .iter()
        .find(|s| s.name == SectionName::Upstream)
        .expect("an upstream entry");
    assert!(upstream.tokens_after > 0, "the section survived");
    assert!(upstream.stubbed.unwrap_or(0) > 0, "and rows were degraded");
    assert!(
        prompt.text.contains("also upstream:"),
        "the one-liners are what the ladder leaves"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::PreviousDiff),
        Some(TrimStrategy::None),
        "and the pass stopped before the next section"
    );
}

#[test]
fn a_diff_falls_to_its_stat_before_it_is_dropped() {
    let spec = oversize_at(untrimmed_total() - 20_000);
    let prompt = ok(&spec);
    assert_eq!(
        strategy_of(&prompt, &SectionName::PreviousDiff),
        Some(TrimStrategy::StatOnly)
    );
    assert!(
        prompt
            .text
            .contains(" 2 files changed, 210 insertions(+), 8 deletions(-)"),
        "the stat carries most of the signal at a fraction of the cost"
    );
    assert!(
        !prompt
            .text
            .contains("+    self.rows.push(PendingRow::new(209));"),
        "and the unified diff is what went"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::VerifyFailure),
        Some(TrimStrategy::None)
    );
}

#[test]
fn verify_failure_keeps_its_tail() {
    // Head-only truncation is the failure Codex had to patch: "TAIL IS COMPLETELY LOST - error
    // messages invisible to model". A verification tail is exactly that case.
    let mut spec = fixtures::phase_oversize();
    spec.verify_failure = Some(htui_core::prompt::VerifyFailure {
        exit_code: 101,
        output: (0..400)
            .map(|n| format!("test recorder::case_{n} ... ok\n"))
            .collect(),
    });
    let before = {
        let mut probe = spec.clone();
        probe.budget = htui_core::prompt::Budget {
            tokens: i64::MAX / 4,
            source: htui_core::prompt::BudgetSource::Project,
            reserve_bp: 0,
        };
        ok(&probe).trim.estimated_before
    };
    spec.budget = htui_core::prompt::Budget {
        tokens: before - 24_000,
        source: htui_core::prompt::BudgetSource::Project,
        reserve_bp: 0,
    };

    let prompt = ok(&spec);
    let section = prompt
        .trim
        .sections
        .iter()
        .find(|s| s.name == SectionName::VerifyFailure)
        .expect("a verify_failure entry");
    assert_eq!(section.strategy, TrimStrategy::TailCut);
    assert!(
        prompt.text.contains("test recorder::case_399 ... ok"),
        "the tail survives"
    );
    assert!(
        !prompt.text.contains("test recorder::case_0 ... ok"),
        "the head is what went"
    );
    assert!(
        prompt.text.contains(&elision_marker(
            section.elided_lines.expect("a marker"),
            section.elided_bytes.expect("a marker"),
        )),
        "and the marker says how much"
    );
}

// ---------------------------------------------------------------------------------------------
// D100 under the trim ladder — review finding C-1
// ---------------------------------------------------------------------------------------------

/// The maskable secret the C-1 corpus plants in every trimmable input.
const SECRET: &str = "hunter2";

/// The oversize corpus with `SECRET` in every input a trim rung re-renders from.
///
/// One string in every field the ladder reaches: the excerpt bodies (whole-file drop), the upstream
/// titles and summaries (stub ladder), the diff's stat and body (stat-only), the verification output
/// (tail cut), both document bodies and the item body (head+tail). If any rung re-renders from
/// [`PromptSpec`] rather than from the scrubbed inputs, the secret comes back.
fn oversize_with_secrets() -> PromptSpec {
    let mut spec = fixtures::phase_oversize();
    spec.item_body = format!("the item body says {SECRET}\n{}", spec.item_body);
    for doc in &mut spec.documents {
        doc.body = format!("the {} body says {SECRET}\n{}", doc.kind, doc.body);
    }
    for entry in &mut spec.upstream {
        entry.title = format!("{} {SECRET}", entry.title);
        entry.summary = entry
            .summary
            .as_deref()
            .map(|summary| format!("the summary says {SECRET}\n{summary}"));
    }
    for excerpt in &mut spec.excerpts.files {
        excerpt.content = format!("// {SECRET}\n{}", excerpt.content);
    }
    if let Some(diff) = &mut spec.previous_diff {
        diff.stat = format!("{} {SECRET}", diff.stat);
        diff.diff = format!("+// {SECRET}\n{}", diff.diff);
    }
    // 400 lines, well over the 200-line floor, so the tail cut has something to reclaim and the
    // rung re-renders `verify_failure` from its own input rather than cutting the section in place.
    spec.verify_failure = Some(htui_core::prompt::VerifyFailure {
        exit_code: 101,
        output: (0..400)
            .map(|n| format!("test recorder::case_{n} says {SECRET} ... ok\n"))
            .collect(),
    });
    spec
}

/// `spec` at `target` tokens with no reserve, so the test's arithmetic is the target's.
fn at_target(spec: &PromptSpec, target: i64) -> PromptSpec {
    let mut out = spec.clone();
    out.budget = htui_core::prompt::Budget {
        tokens: target,
        source: htui_core::prompt::BudgetSource::Project,
        reserve_bp: 0,
    };
    out
}

#[test]
fn no_trim_rung_re_renders_an_unscrubbed_input() {
    // **C-1.** `assemble()` scrubs the *rendered* sections (D100) and then hands them to the
    // trimmer, whose rungs re-render from `PromptSpec` — so every masked byte the ladder touched
    // reappeared in the prompt while `trim_record` went on describing the scrubbed text. The fix is
    // to mask at the input layer, once, before anything renders: then the first render and every
    // re-render are over the same masked data and there is no rung left to forget.
    let base = oversize_with_secrets();
    let scrubber = MinimalScrubber::new([SECRET.to_owned()]);
    let untrimmed = assemble(&at_target(&base, i64::MAX / 4), &scrubber)
        .expect("the corpus assembles")
        .trim
        .estimated_before;

    let mut strategies: BTreeSet<&'static str> = BTreeSet::new();
    for cut in [
        0, 4_000, 8_000, 12_000, 16_000, 20_000, 24_000, 28_000, 32_000,
    ] {
        let spec = at_target(&base, untrimmed - cut);
        let prompt = assemble(&spec, &scrubber)
            .unwrap_or_else(|error| panic!("the corpus assembles at -{cut}: {error}"));
        assert!(
            !prompt.text.contains(SECRET),
            "the scrub survived neither the render nor the trim at -{cut}"
        );
        assert!(
            prompt.text.contains("[REDACTED]"),
            "and the mask is what took its place at -{cut}"
        );
        for section in &prompt.trim.sections {
            strategies.insert(section.strategy.as_str());
        }
    }
    // Every rung that re-renders is proved reached, or the sweep above proves nothing.
    for strategy in [
        TrimStrategy::HeadTail,
        TrimStrategy::TailCut,
        TrimStrategy::StatOnly,
        TrimStrategy::StubLadder,
        TrimStrategy::Dropped,
    ] {
        assert!(
            strategies.contains(strategy.as_str()),
            "the sweep never reached `{}`: {strategies:?}",
            strategy.as_str()
        );
    }
}

#[test]
fn the_judge_candidate_rung_re_renders_scrubbed_too() {
    // C-1's judge half: `trim_candidates` re-renders a candidate stat-only from
    // `inputs.candidates[i]`, which is the caller's struct.
    let mut spec = fixtures::judge_three_candidates();
    let Some(judge) = &mut spec.judge else {
        panic!("the judge fixture carries judge inputs");
    };
    for candidate in &mut judge.candidates {
        if let Some(diff) = &mut candidate.diff {
            diff.stat = format!("{} {SECRET}", diff.stat);
            diff.diff = format!("+// {SECRET}\n{}", diff.diff);
        }
        if let Some(doc) = &mut candidate.document {
            doc.body = format!("{SECRET}\n{}", doc.body);
        }
        candidate.verification_tail = candidate
            .verification_tail
            .as_deref()
            .map(|tail| format!("{SECRET}\n{tail}"));
    }
    let scrubber = MinimalScrubber::new([SECRET.to_owned()]);
    let untrimmed = assemble(&at_target(&spec, i64::MAX / 4), &scrubber)
        .expect("the judge corpus assembles")
        .trim
        .estimated_before;

    let mut saw_stat_only = false;
    for percent in [90, 75, 60, 50, 40] {
        // Floored above the protected set, which is what a judge frame plus its box and skills
        // cost: below it the call is a refusal rather than a trim and proves nothing.
        let target = (untrimmed * percent / 100).max(400);
        let prompt = assemble(&at_target(&spec, target), &scrubber)
            .unwrap_or_else(|error| panic!("the judge corpus assembles at {target}: {error}"));
        assert!(
            !prompt.text.contains(SECRET),
            "a candidate re-rendered stat-only from an unscrubbed input at {target}"
        );
        saw_stat_only |= prompt
            .trim
            .sections
            .iter()
            .any(|section| section.strategy == TrimStrategy::StatOnly);
    }
    assert!(saw_stat_only, "the isolate cap was never reached");
}

#[test]
fn the_frame_literals_are_scrubbed_like_every_other_digested_byte() {
    // **H-1.** Step 7 substituted `render::normalise_newlines(literal)` — the raw span — while only
    // the *estimate* ran over a scrubbed copy, so a secret written into a `prompt_template.body`
    // reached the model and the digest.
    let mut spec = fixtures::phase_implement_attempt2();
    spec.body = format!("Use the key {SECRET}.\n\n{}", spec.body);
    let prompt = assemble(&spec, &MinimalScrubber::new([SECRET.to_owned()]))
        .expect("a maskable secret in the frame is masked, not refused");
    assert!(
        !prompt.text.contains(SECRET),
        "§4.7 step 2: the frame's literals are digested bytes like any other"
    );
    assert!(prompt.text.contains("Use the key [REDACTED]."));

    // And a secret the scrubber cannot mask refuses, naming the frame rather than the text.
    let mut residue = fixtures::phase_implement_attempt2();
    residue.body = format!("Use the key sk-ant-api03-DEADBEEF.\n\n{}", residue.body);
    let error = assemble(&residue, &scrubber()).expect_err("`R-SEC-3` fails closed on the frame");
    let AssembleError::Unmasked { ref section, .. } = error else {
        panic!("expected an unmasked refusal, got {error:?}");
    };
    assert_eq!(section, "template");
    assert!(!error.to_string().contains("sk-ant-api03-DEADBEEF"));
}

#[test]
fn an_attribute_is_masked_before_it_is_escaped_and_cut() {
    // **M-2.** The scrub ran on values `render::attr` had already rewritten and `title_attr` had
    // already cut at 120 bytes, so an exact-match mask missed a secret carrying `&` or `"` and a
    // long secret shipped its first 120 bytes. Masking at the input layer puts the mask first.
    let long_secret = format!("SEKRIT-{}-TAIL", "x".repeat(200));
    let amp_secret = "MOD & 2";
    let mut spec = fixtures::phase_implement_attempt2();
    spec.item_title = format!("Ship it: {long_secret}");
    spec.item_key = format!("htui:{amp_secret}");
    let prompt = assemble(
        &spec,
        &MinimalScrubber::new([long_secret.clone(), amp_secret.to_owned()]),
    )
    .expect("both secrets are maskable");
    assert!(
        !prompt.text.contains("SEKRIT-xxxxxxxxxx"),
        "the 120-byte `title` cut must not ship the head of a masked secret:\n{}",
        prompt.text
    );
    assert!(
        !prompt.text.contains("MOD &amp; 2"),
        "a secret carrying `&` is masked before `attr` rewrites it:\n{}",
        prompt.text
    );
}

#[test]
fn every_marker_matches_its_record() {
    // Criterion 9's last clause, and blueprint rule 11: the prompt and the record never disagree.
    let prompt = ok(&fixtures::phase_oversize());
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for section in &prompt.trim.sections {
        match (section.elided_lines, section.elided_bytes) {
            (Some(lines), Some(bytes)) => {
                let marker = elision_marker(lines, bytes);
                assert!(
                    prompt.text.contains(&marker),
                    "`{}` records {lines} lines / {bytes} bytes and the prompt does not say so",
                    section.name
                );
                assert!(section.trimmed, "a marker means the section lost content");
                expected.insert(marker);
            }
            (None, None) => {}
            _ => panic!(
                "`{}` records one elision number and not the other",
                section.name
            ),
        }
    }
    assert!(
        !expected.is_empty(),
        "the oversize fixture must produce at least one marker"
    );
    // And the converse: no marker in the text is missing from the record.
    let found: BTreeSet<String> = prompt
        .text
        .lines()
        .filter(|line| line.starts_with("[... htui elided "))
        .map(str::to_owned)
        .collect();
    assert_eq!(found, expected, "every marker in the text is in the record");
}

#[test]
fn the_record_hashes_the_rendered_excerpt_block() {
    // ANA-5 §4.5 `:1136-1137`: "`sha256` is over the excerpt's rendered content bytes, not the
    // whole file, so a reader can prove which bytes the model saw without storing them."
    //
    // Plan F-39 recorded that T65 narrowed `files[]` from the ranker's own records instead,
    // because `render::file_block` was private. This is that hole closed: the record is rebuilt
    // from the surviving excerpts and hashed over the bytes the prompt actually carries.
    let prompt = ok(&fixtures::phase_implement_attempt2());
    let files = &prompt.trim.excerpts.files;
    assert_eq!(files.len(), 2, "both fixture excerpts survived: {files:?}");
    for file in files {
        let open = format!("<file path=\"{}:{}\" ", file.repo, file.path);
        let at = prompt
            .text
            .find(&open)
            .unwrap_or_else(|| panic!("`{open}` is not in the prompt:\n{}", prompt.text));
        let end = prompt.text[at..]
            .find("</file>")
            .expect("every block closes")
            + "</file>".len();
        let block = &prompt.text[at..at + end];
        assert_eq!(
            file.bytes,
            block.len() as u64,
            "`{}` records {} bytes and the rendered block is {}",
            file.path,
            file.bytes,
            block.len()
        );
        assert_eq!(
            file.sha256,
            htui_core::prompt::digest::sha256_hex(block),
            "`{}`'s recorded digest is not its rendered block's",
            file.path
        );
        assert_eq!(file.sha256.len(), 64, "lowercase sha256 hex, all 64");
        assert!(
            block.contains(&format!(" lines=\"{}\"", file.lines)),
            "the record's `lines` is the attribute's"
        );
    }
    // The audit's own `selected` is the ranker's and is allowed to differ from `files`.
    assert_eq!(prompt.trim.excerpts.selected, 2);
}

#[test]
fn a_dropped_excerpt_leaves_its_record_behind_and_nothing_else() {
    // §5.1 `:1536-1538`: `selected` is what the ranker paid for, `files` is what reached the
    // model, and the trimmer is allowed to make the two disagree.
    let prompt = ok(&fixtures::phase_oversize());
    let excerpts = &prompt.trim.excerpts;
    assert!(
        excerpts.selected > 0,
        "the oversize fixture selected excerpts"
    );
    assert!(
        excerpts.files.len() < excerpts.selected as usize,
        "the trim dropped at least one: {excerpts:?}"
    );
    for file in &excerpts.files {
        assert!(
            prompt
                .text
                .contains(&format!("<file path=\"{}:{}\" ", file.repo, file.path)),
            "`{}` is recorded and is not in the prompt",
            file.path
        );
    }
}

#[test]
fn reserve_target_is_integer_arithmetic() {
    let spec = fixtures::phase_implement_attempt2();
    let record = ok(&spec).trim;
    assert_eq!(record.budget, 120_000);
    assert_eq!(record.target, 108_000, "§5.1's worked example");
    assert!((record.reserve - 0.10).abs() < f64::EPSILON);
    assert_eq!(
        serde_json::to_value(&record).expect("plain data")["budget_source"],
        serde_json::json!("project")
    );
}

#[test]
fn to_value_is_byte_stable_and_carries_the_documented_keys() {
    // Blueprint H-19 asks for sorted keys, on the plan's verified claim that "`serde_json::Map` is
    // a `BTreeMap` here — no `preserve_order` feature anywhere". **That claim is false in a
    // workspace build** (finding F-35): `agent-client-protocol` turns on
    // `serde_json/preserve_order`, and Cargo unifies features across the workspace, so
    // `htui-core`'s own `Map` is an `IndexMap` whenever `htui-agent` is in the same build and a
    // `BTreeMap` when it is not. Key order is therefore sorted under `-p htui-core` and field
    // declaration order under `--workspace`, from one unchanged `#[derive(Serialize)]`.
    //
    // What §5.1 actually needs survives that, which is why this test asserts it instead: the
    // serialisation is **byte-stable** — deterministic under either map type — and the column is
    // `JSONB`, which reorders keys on the way in regardless. Nothing in the assembler reads a key
    // order, and `prompt_digest` is over the text and never over this record.
    let first = ok(&fixtures::phase_implement_attempt2()).trim.to_value();
    let second = ok(&fixtures::phase_implement_attempt2()).trim.to_value();
    assert_eq!(
        serde_json::to_string(&first).expect("plain data"),
        serde_json::to_string(&second).expect("plain data"),
        "two assemblies of one spec serialise to the same bytes"
    );

    let keys: BTreeSet<&str> = first
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "v",
            "template",
            "budget",
            "budget_source",
            "reserve",
            "target",
            "estimator",
            "estimated_before",
            "estimated_after",
            "sections",
            "excerpts",
            "notes",
        ]),
        "§5.1's twelve keys, no more and no fewer"
    );
    assert_eq!(first["v"], serde_json::json!(1));
    assert_eq!(first["template"]["role"], serde_json::json!("phase"));
    assert_eq!(first["template"]["version"], serde_json::json!(3));
    assert_eq!(first["estimator"], serde_json::json!("chars-v2"));
    // A section row omits the four optional counters rather than spelling them `null`.
    let template_row = &first["sections"][0];
    assert_eq!(template_row["name"], serde_json::json!("template"));
    assert_eq!(template_row["strategy"], serde_json::json!("none"));
    assert!(template_row.get("elided_lines").is_none());
    assert!(template_row.get("stubbed").is_none());
}

// ---------------------------------------------------------------------------------------------
// Criterion 10 — the two refusals, before anything happens
// ---------------------------------------------------------------------------------------------

#[test]
fn a_protected_set_over_target_refuses_before_anything() {
    let spec = fixtures::phase_protected_too_big();
    let error = assemble(&spec, &scrubber()).expect_err("the protected set alone is over target");
    let AssembleError::BudgetTooSmall { needed, target } = error else {
        panic!("expected a budget refusal, got {error:?}");
    };
    assert_eq!(target, spec.budget.target());
    assert!(needed > target, "{needed} must exceed {target}");
    assert_eq!(
        AssembleError::BudgetTooSmall { needed, target }.to_string(),
        format!(
            "prompt budget too small: protected sections need {needed} tokens, target is {target}"
        ),
        "criterion 10's exact text, which becomes `run.failure`"
    );
}

#[test]
fn skills_over_the_cap_refuse() {
    let spec = fixtures::phase_skills_over_cap();
    let error = assemble(&spec, &scrubber()).expect_err("the skills exceed their cap");
    let AssembleError::SkillsExceedCap { tokens, cap } = error else {
        panic!("expected a skills refusal, got {error:?}");
    };
    assert_eq!(cap, spec.max_skill_tokens);
    assert!(tokens > cap);
    assert!(
        error
            .to_string()
            .starts_with("skills exceed max_skill_tokens"),
        "criterion 10's exact text"
    );

    // The skills cap is checked **before** the budget, because it is the sharper message: a step
    // whose skills alone are over the cap is misconfigured in one place, not two (D.1 step 5).
    let mut both = spec;
    both.budget = htui_core::prompt::Budget {
        tokens: 1_000,
        source: htui_core::prompt::BudgetSource::Project,
        reserve_bp: 1_000,
    };
    assert!(
        matches!(
            assemble(&both, &scrubber()),
            Err(AssembleError::SkillsExceedCap { .. })
        ),
        "the skills refusal wins when both would fire"
    );
}

// ---------------------------------------------------------------------------------------------
// Criterion 3 — a row that bypassed MOD-9's validator
// ---------------------------------------------------------------------------------------------

#[test]
fn an_unknown_placeholder_refuses_with_the_stage_3_text() {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.body = "You are running {{phase}} for {{item_key}}.\n\n{{foo}}\n".to_owned();
    let error = assemble(&spec, &scrubber()).expect_err("`foo` is outside the closed set");
    assert_eq!(
        error.to_string(),
        "unknown prompt placeholder: {{foo}}",
        "criterion 3's exact text"
    );

    // A placeholder that exists but not in this role is the same failure to a caller: the template
    // named something it may not use (blueprint B.1).
    let mut wrong_role = fixtures::phase_implement_attempt2();
    wrong_role.body = "{{item}}\n{{candidates}}\n".to_owned();
    assert!(matches!(
        assemble(&wrong_role, &scrubber()),
        Err(AssembleError::UnknownPlaceholder { ref token }) if token == "candidates"
    ));
}

#[test]
fn scrub_residue_names_the_section_not_the_text() {
    // D100: the scrub runs per section, before the digest, so `Unmasked.section` names the fact a
    // maintainer can act on — and never the secret.
    let mut spec = fixtures::phase_implement_attempt2();
    spec.item_body = "The token is sk-ant-api03-DEADBEEF and it must never be stored.\n".to_owned();
    let error = assemble(&spec, &scrubber()).expect_err("`R-SEC-3` fails closed");
    let AssembleError::Unmasked {
        ref section,
        rule,
        ref path,
    } = error
    else {
        panic!("expected an unmasked refusal, got {error:?}");
    };
    assert_eq!(section, "item");
    assert_eq!(rule, "anthropic_api_key");
    assert_eq!(path, "", "the whole wrapped string is the leaf");
    let rendered = error.to_string();
    assert!(
        !rendered.contains("sk-ant-api03-DEADBEEF"),
        "the error must not repeat the secret: {rendered}"
    );

    // A secret the scrubber *can* mask is masked, and assembly continues.
    let mut masked = fixtures::phase_implement_attempt2();
    masked.item_body = "The token is hunter2 and it must never be stored.\n".to_owned();
    let prompt = assemble(&masked, &MinimalScrubber::new(["hunter2".to_owned()]))
        .expect("a maskable secret is masked, not refused");
    assert!(!prompt.text.contains("hunter2"));
    assert!(prompt.text.contains("[REDACTED]"));
}

// ---------------------------------------------------------------------------------------------
// §4.7's ordering rules the assembler enforces itself
// ---------------------------------------------------------------------------------------------

#[test]
fn upstream_order_is_canonical_whatever_the_caller_sent() {
    // Rule 2: the assembler re-sorts rather than trusting its caller, because three backends and
    // two collations produce the rows.
    let spec = fixtures::phase_implement_attempt2();
    let mut shuffled = spec.clone();
    shuffled.upstream.reverse();
    assert_ne!(
        shuffled.upstream, spec.upstream,
        "the fixture must actually be reordered"
    );
    assert_eq!(ok(&spec).digest, ok(&shuffled).digest);
}

#[test]
fn skill_order_is_unchanged_by_store_order() {
    // Criterion 15's third clause. The first two (the phase override and the `position` tie-break)
    // are `model::skill`'s own tests; this is the assembler half.
    let mut spec = fixtures::phase_skills_over_cap();
    spec.max_skill_tokens = 20_000;
    let mut reversed = spec.clone();
    reversed.skills.reverse();
    assert_eq!(ok(&spec).digest, ok(&reversed).digest);

    // And a skill bound twice appears once (`BoundSkill::collapse`'s dedup, applied here too).
    let mut doubled = spec.clone();
    doubled.skills.push(spec.skills[0].clone());
    assert_eq!(ok(&spec).text, ok(&doubled).text);
}

#[test]
fn a_reordered_body_reorders_sections_and_changes_the_digest() {
    // Rule 1: section order is the parsed span order, which is a property of the pinned template
    // version. A body that moves `{{upstream}}` above `{{documents}}` is a different prompt.
    let spec = fixtures::phase_implement_attempt2();
    let mut moved = spec.clone();
    moved.body = spec.body.replace(
        "{{documents}}\n{{verify_failure}}\n{{previous_diff}}\n{{upstream}}",
        "{{upstream}}\n{{documents}}\n{{verify_failure}}\n{{previous_diff}}",
    );
    assert_ne!(moved.body, spec.body, "the replacement must actually apply");

    let original = ok(&spec);
    let reordered = ok(&moved);
    assert_ne!(original.digest, reordered.digest);
    let names = |prompt: &AssembledPrompt| -> Vec<String> {
        prompt
            .trim
            .sections
            .iter()
            .map(|s| s.name.render())
            .collect()
    };
    let before = names(&original);
    let after = names(&reordered);
    assert_ne!(before, after, "`sections[]` follows the body");
    assert_eq!(
        before.iter().collect::<BTreeSet<_>>(),
        after.iter().collect::<BTreeSet<_>>(),
        "the same sections, in a different order"
    );
}

// ---------------------------------------------------------------------------------------------
// Criteria 17 and 18 — the two roles whose callers are MOD-4's (D107)
// ---------------------------------------------------------------------------------------------

#[test]
fn the_second_judge_call_differs_only_in_candidate_order() {
    let spec = fixtures::judge_three_candidates();
    let first = ok(&spec);
    let mut second_call = spec.clone();
    second_call
        .judge
        .as_mut()
        .expect("the judge role carries judge inputs")
        .reverse = true;
    let second = ok(&second_call);

    assert_ne!(
        first.digest, second.digest,
        "a different prompt with a different digest, by design (§4.7 rule 6)"
    );
    let candidates = |prompt: &AssembledPrompt| -> Vec<String> {
        prompt
            .trim
            .sections
            .iter()
            .map(|s| s.name.render())
            .filter(|name| name.starts_with("judge_candidate:"))
            .collect()
    };
    assert_eq!(
        candidates(&first),
        vec![
            "judge_candidate:0",
            "judge_candidate:1",
            "judge_candidate:2"
        ]
    );
    let mut reversed = candidates(&first);
    reversed.reverse();
    assert_eq!(candidates(&second), reversed);
    assert_eq!(
        first.text.len(),
        second.text.len(),
        "only the order moved; not one byte was added or removed"
    );
}

#[test]
fn a_candidate_over_its_share_renders_stat_only() {
    // Criterion 17's third clause: the isolate cap is per candidate, so one oversized diff does not
    // cost the other two their content.
    let mut spec = fixtures::judge_three_candidates();
    spec.budget = htui_core::prompt::Budget {
        tokens: 4_000,
        source: htui_core::prompt::BudgetSource::Project,
        reserve_bp: 1_000,
    };
    let bloated = spec.judge.as_mut().expect("judge inputs").candidates[1]
        .diff
        .as_mut()
        .expect("candidate 1 carries a diff");
    bloated.diff = (0..800)
        .map(|n| format!("+    self.rows.push(PendingRow::new({n}));\n"))
        .collect();

    let prompt = ok(&spec);
    assert_eq!(
        strategy_of(&prompt, &SectionName::JudgeCandidate(1)),
        Some(TrimStrategy::StatOnly)
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::JudgeCandidate(0)),
        Some(TrimStrategy::None),
        "a candidate inside its share keeps its diff"
    );
    assert!(
        prompt
            .text
            .contains(" 1 file changed, 22 insertions(+), 1 deletion(-)"),
        "the stat survives; it is the signal the diff carried"
    );
    assert!(
        !prompt
            .text
            .contains("+    self.rows.push(PendingRow::new(799));"),
        "and the unified diff does not"
    );
    assert!(prompt.trim.estimated_after <= prompt.trim.target);
}

#[test]
fn a_handoff_summary_carries_only_the_windowed_tail() {
    // Criterion 18's third clause, which is the half MOD-2 can own: no verbatim `assistant_text`
    // beyond the windowed tail the caller put in `StepSummary` (`R-PRM-1`, D107).
    let prompt = ok(&fixtures::handoff_basic());
    assert!(
        prompt
            .text
            .contains("Last assistant message (turn 3, tail):")
    );
    assert!(
        prompt
            .text
            .contains("I cannot get the borrow checker past the trim loop.")
    );
    assert!(
        prompt.text.contains("<section name=\"failure_reason\">"),
        "the reason the prompt exists is protected and present"
    );
    assert_eq!(
        strategy_of(&prompt, &SectionName::FailureReason),
        Some(TrimStrategy::None)
    );
    // The handoff role's own closed set: no `{{item}}`, so no item section reaches the record.
    assert!(
        !prompt.text.contains("<section name=\"item\""),
        "`item` is outside the handoff role's placeholder set"
    );
}

#[test]
fn a_role_without_its_inputs_refuses() {
    let mut spec = fixtures::judge_three_candidates();
    spec.judge = None;
    assert!(matches!(
        assemble(&spec, &scrubber()),
        Err(AssembleError::RoleInputs { .. })
    ));
}

// ---------------------------------------------------------------------------------------------
// One vocabulary, two spellings (T67)
// ---------------------------------------------------------------------------------------------

/// `as_str` and `Serialize` must produce the same word for every variant of the three closed
/// vocabularies `trim_record` carries.
///
/// The Prompt sub-tab renders `as_str` and a reader compares what it renders against the stored
/// row, which is the `Serialize` form. Two tables would be two spellings, and this is what keeps
/// them one.
#[test]
fn the_record_vocabularies_spell_themselves_once() {
    use htui_core::prompt::excerpt::RootSource;
    use htui_core::prompt::settings::BudgetSource;

    fn serialised(value: &impl serde::Serialize) -> String {
        match serde_json::to_value(value).expect("a unit variant serialises") {
            serde_json::Value::String(text) => text,
            other => panic!("a unit variant serialises as a string, not {other}"),
        }
    }

    for strategy in [
        TrimStrategy::None,
        TrimStrategy::HeadTail,
        TrimStrategy::TailCut,
        TrimStrategy::StatOnly,
        TrimStrategy::StubLadder,
        TrimStrategy::Dropped,
    ] {
        assert_eq!(strategy.as_str(), serialised(&strategy), "{strategy:?}");
    }
    for source in [
        BudgetSource::Phase,
        BudgetSource::Project,
        BudgetSource::AppSetting,
        BudgetSource::AppSettingDefault,
    ] {
        assert_eq!(source.as_str(), serialised(&source), "{source:?}");
    }
    for source in [
        RootSource::RunStepTree,
        RootSource::RepoBoxPath,
        RootSource::NoPath,
    ] {
        assert_eq!(source.as_str(), serialised(&source), "{source:?}");
    }
}
