//! MOD-164 T4: the capture core (plan D9, D10). Pure: the trigger over a run's steps, the
//! evidence pack built from them, the fixed capture prompt, and the parser for the light model's
//! answer.
#![cfg(feature = "test-support")]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use htui_core::capture::{
    CAPTURE_INPUT_TOKEN_CAP, CAPTURE_OUTPUT_TOKEN_CAP, CaptureReason, CaptureRefusal, DropReason,
    EvidencePack, capture_prompt, capture_trigger, parse_capture,
};
use htui_core::model::memory::{MEMORY_RUN_WRITE_CAP, MemoryKind};
use htui_core::model::run::{GateOutcome, RunStep, StepRole, StepStatus, VerifyOutcome};
use htui_core::model::{MemoryEntryId, RunId, StepId};
use htui_core::prompt::digest::{canonical, sha256_hex};
use htui_core::prompt::{SectionName, TokenEstimator, fixtures};
use htui_core::scrub::MinimalScrubber;
use serde_json::{Value, json};
use uuid::Uuid;

fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-13T10:00:00Z")
        .expect("a fixed instant")
        .with_timezone(&Utc)
}

/// A clean step: done on attempt 1, verify passed, gate approved. Ids are fixed so a pack is
/// reproducible.
fn step(n: u128, position: i32, attempt: i32) -> RunStep {
    RunStep {
        id: StepId::from_uuid(Uuid::from_u128(0x1000 + n)),
        run_id: RunId::from_uuid(Uuid::from_u128(0x99)),
        position,
        attempt,
        fanout_index: 0,
        phase_name: format!("phase{position}"),
        agent_id: None,
        model: None,
        status: StepStatus::Done,
        gate_outcome: Some(GateOutcome::Approved),
        gate_note: None,
        selected: None,
        exit_code: Some(0),
        prompt_digest: None,
        trim_record: None,
        usage: None,
        isolation_path: None,
        started_at: Some(at()),
        finished_at: Some(at()),
        verify_outcome: Some(VerifyOutcome::Pass),
        verify_exit_code: Some(0),
        promoted_at: None,
        branch: None,
        role: StepRole::Step,
        parent_step_id: None,
        node_key: None,
        updated_at: at(),
    }
}

fn failed(n: u128, position: i32, attempt: i32) -> RunStep {
    RunStep {
        status: StepStatus::Failed,
        exit_code: Some(1),
        verify_outcome: Some(VerifyOutcome::Fail),
        verify_exit_code: Some(1),
        gate_outcome: None,
        ..step(n, position, attempt)
    }
}

fn plain() -> MinimalScrubber {
    MinimalScrubber::new(Vec::<String>::new())
}

// ---- Trigger (D9) ------------------------------------------------------------------------------

#[test]
fn a_clean_run_does_not_trigger() {
    let steps = [step(1, 0, 1), step(2, 1, 1), step(3, 2, 1)];
    assert_eq!(capture_trigger(&steps), None);
    assert_eq!(capture_trigger(&[]), None);
}

#[test]
fn each_of_the_four_signals_triggers() {
    let base = || vec![step(1, 0, 1), step(2, 1, 1)];

    let mut steps = base();
    steps[1].status = StepStatus::Failed;
    assert_eq!(capture_trigger(&steps), Some(CaptureReason::StepFailed));

    let mut steps = base();
    steps.push(step(3, 1, 2));
    steps[1].status = StepStatus::Superseded;
    assert_eq!(capture_trigger(&steps), Some(CaptureReason::Retried));

    let mut steps = base();
    steps[0].verify_outcome = Some(VerifyOutcome::Fail);
    assert_eq!(capture_trigger(&steps), Some(CaptureReason::VerifyFailed));

    for outcome in [GateOutcome::Rejected, GateOutcome::Retried] {
        let mut steps = base();
        steps[1].gate_outcome = Some(outcome);
        assert_eq!(
            capture_trigger(&steps),
            Some(CaptureReason::GateRejected),
            "{outcome:?}"
        );
    }

    // Approved, skipped and an unavailable verify are not signals.
    let mut steps = base();
    steps[0].gate_outcome = Some(GateOutcome::Skipped);
    steps[1].verify_outcome = Some(VerifyOutcome::Unavailable);
    assert_eq!(capture_trigger(&steps), None);
}

#[test]
fn a_judge_or_a_stage_row_alone_does_not_trigger() {
    let mut judge = failed(9, 1, 1);
    judge.fanout_index = -1;
    judge.attempt = 2;
    judge.gate_outcome = Some(GateOutcome::Rejected);
    let steps = [step(1, 0, 1), step(2, 1, 1), judge];
    assert_eq!(capture_trigger(&steps), None);

    let mut stage = failed(8, -1, 1);
    stage.role = StepRole::Stage;
    stage.node_key = Some("collect".to_owned());
    assert_eq!(capture_trigger(&[step(1, 0, 1), stage]), None);
}

#[test]
fn every_reason_has_a_stable_spelling() {
    let all = [
        CaptureReason::StepFailed,
        CaptureReason::Retried,
        CaptureReason::VerifyFailed,
        CaptureReason::GateRejected,
    ];
    let names: Vec<&str> = all.iter().map(|reason| reason.as_str()).collect();
    assert_eq!(
        names,
        ["step_failed", "retried", "verify_failed", "gate_rejected"]
    );
}

// ---- Evidence pack (D10) -----------------------------------------------------------------------

#[test]
fn blocks_are_labelled_in_walk_order() {
    // Given out of order: (2,1), (0,1), (1,2), (1,1). Walk order is position, then attempt.
    let mut late = failed(4, 2, 1);
    late.gate_note = Some("the reviewer asked for a smaller diff".to_owned());
    let first = failed(1, 0, 1);
    let retry = step(3, 1, 2);
    let mut early = failed(2, 1, 1);
    early.status = StepStatus::Superseded;
    let clean = step(5, 3, 1);
    let steps = vec![
        late.clone(),
        first.clone(),
        retry.clone(),
        early.clone(),
        clean,
    ];

    let pack = EvidencePack::build(&steps, &BTreeMap::new());
    let labels: Vec<(&str, StepId)> = pack
        .blocks()
        .iter()
        .map(|block| (block.label.as_str(), block.step_id))
        .collect();
    assert_eq!(
        labels,
        [
            ("S1", first.id),
            ("S2", early.id),
            ("S3", retry.id),
            ("S4", late.id),
        ],
        "the clean step at position 3 is not notable"
    );
    assert_eq!(pack.step_of("S1"), Some(first.id));
    assert_eq!(pack.step_of("S4"), Some(late.id));
    assert_eq!(pack.step_of("S5"), None);
    assert_eq!(pack.step_of("s1"), None);

    let s4 = &pack.blocks()[3].text;
    assert!(s4.contains("phase: phase2"), "{s4}");
    assert!(s4.contains("attempt: 1"), "{s4}");
    assert!(s4.contains("status: failed"), "{s4}");
    assert!(s4.contains("exit_code: 1"), "{s4}");
    assert!(s4.contains("verify: fail"), "{s4}");
    assert!(
        s4.contains("gate_note: the reviewer asked for a smaller diff"),
        "{s4}"
    );
    let text = pack.text();
    let order: Vec<usize> = [
        "label=\"S1\"",
        "label=\"S2\"",
        "label=\"S3\"",
        "label=\"S4\"",
    ]
    .iter()
    .map(|label| text.find(label).expect("every label is rendered"))
    .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

#[test]
fn the_winning_attempt_is_included_with_its_output() {
    let lost = failed(1, 0, 1);
    let won = step(2, 0, 2);
    let mut outputs = BTreeMap::new();
    outputs.insert(
        lost.id,
        "error[E0599]: no method named `frob` found".to_owned(),
    );
    outputs.insert(won.id, "test result: ok. 12 passed; 0 failed".to_owned());

    let pack = EvidencePack::build(&[won.clone(), lost.clone()], &outputs);
    assert_eq!(pack.blocks().len(), 2);
    assert_eq!(pack.step_of("S1"), Some(lost.id));
    assert_eq!(pack.step_of("S2"), Some(won.id));
    let win = &pack.blocks()[1].text;
    assert!(win.contains("status: done"), "{win}");
    assert!(
        win.contains("test result: ok. 12 passed; 0 failed"),
        "{win}"
    );
    assert!(win.contains("passed after a failed attempt"), "{win}");
    assert!(
        pack.blocks()[0]
            .text
            .contains("error[E0599]: no method named `frob` found")
    );
}

#[test]
fn judges_and_stage_rows_are_never_blocks() {
    let mut judge = failed(9, 0, 1);
    judge.fanout_index = -1;
    let mut stage = failed(8, -2, 1);
    stage.role = StepRole::Stage;
    stage.node_key = Some("prepare".to_owned());
    let pack = EvidencePack::build(&[failed(1, 0, 1), judge, stage], &BTreeMap::new());
    assert_eq!(pack.blocks().len(), 1);
}

fn output_of(n: usize, marker: &str) -> String {
    let mut out = String::new();
    for line in 0..n {
        out.push_str(&format!(
            "compiling crate number {line} of the workspace graph\n"
        ));
    }
    out.push_str(marker);
    out
}

#[test]
fn a_large_pack_cuts_output_tails_first() {
    let steps: Vec<RunStep> = (0..4)
        .map(|n| failed(n, i32::try_from(n).unwrap(), 1))
        .collect();
    let outputs: BTreeMap<StepId, String> = steps
        .iter()
        .enumerate()
        .map(|(n, step)| {
            (
                step.id,
                output_of(2_000, &format!("FINAL LINE OF STEP {n}")),
            )
        })
        .collect();

    let pack = EvidencePack::build(&steps, &outputs);
    assert!(
        pack.tokens() <= CAPTURE_INPUT_TOKEN_CAP,
        "{} > {CAPTURE_INPUT_TOKEN_CAP}",
        pack.tokens()
    );
    assert_eq!(
        pack.tokens(),
        TokenEstimator::DEFAULT.estimate(&pack.text())
    );
    assert_eq!(
        pack.blocks().len(),
        4,
        "no block is dropped while a tail can be cut"
    );
    for (n, block) in pack.blocks().iter().enumerate() {
        assert!(
            block.text.contains(&format!("FINAL LINE OF STEP {n}")),
            "the tail is kept: {}",
            block.text
        );
        assert!(
            block.text.contains("earlier characters cut"),
            "{}",
            block.text
        );
        assert!(!block.text.contains("crate number 0 of"), "{}", block.text);
    }
}

#[test]
fn a_pack_still_too_large_drops_the_oldest_blocks() {
    // Each gate note costs about 400 tokens and is never cut, so forty blocks cannot fit.
    let steps: Vec<RunStep> = (0..40)
        .map(|n| {
            let mut step = failed(n, i32::try_from(n).unwrap(), 1);
            step.gate_note = Some(format!(
                "note {n}: {}",
                "the reviewer explains. ".repeat(45)
            ));
            step
        })
        .collect();
    let pack = EvidencePack::build(&steps, &BTreeMap::new());
    assert!(pack.tokens() <= CAPTURE_INPUT_TOKEN_CAP);
    let kept = pack.blocks().len();
    assert!(kept > 1 && kept < 40, "kept {kept}");
    assert_eq!(pack.dropped_blocks(), 40 - kept);
    // The newest survive, relabelled from S1.
    assert_eq!(pack.blocks()[0].label, "S1");
    assert_eq!(pack.blocks()[0].step_id, steps[40 - kept].id);
    assert_eq!(pack.blocks()[kept - 1].step_id, steps[39].id);
}

#[test]
fn an_untriggered_run_has_an_empty_pack() {
    let pack = EvidencePack::build(&[step(1, 0, 1)], &BTreeMap::new());
    assert!(pack.is_empty());
    assert_eq!(pack.dropped_blocks(), 0);
}

// ---- Prompt (D10) ------------------------------------------------------------------------------

fn sample_pack() -> EvidencePack {
    let lost = failed(1, 0, 1);
    let won = step(2, 0, 2);
    let mut outputs = BTreeMap::new();
    outputs.insert(
        lost.id,
        "error: the sqlx offline data is stale for query 3f2a".to_owned(),
    );
    outputs.insert(won.id, "test result: ok. 4 passed".to_owned());
    EvidencePack::build(&[lost, won], &outputs)
}

#[test]
fn the_prompt_digest_is_stable() {
    let pack = sample_pack();
    let one = capture_prompt(&pack);
    let two = capture_prompt(&sample_pack());
    assert_eq!(one, two);
    assert_eq!(one.text, canonical(&one.text), "the text is canonical");
    assert_eq!(one.digest_text, one.text, "nothing is undigested");
    assert_eq!(one.digest, sha256_hex(&one.digest_text));
    assert_eq!(one.digest.len(), 64);
    // Pinned: a change to the fixed text is a change to every capture step's digest.
    assert_eq!(
        one.digest,
        "afef8f3ab01c595a6787ffb9e0b11604af353d85c1fbeba99ea6908b4ee982b9"
    );

    // A different pack is a different prompt.
    let other = EvidencePack::build(&[failed(7, 0, 1)], &BTreeMap::new());
    assert_ne!(capture_prompt(&other).digest, one.digest);
}

#[test]
fn the_prompt_has_assembles_record_shape() {
    let prompt = capture_prompt(&sample_pack());
    assert_eq!(prompt.sections.len(), 1);
    assert_eq!(prompt.sections[0].name, SectionName::Template);
    assert_eq!(prompt.sections, prompt.trim.sections);
    assert_eq!(
        prompt.trim.sections[0].tokens_after,
        TokenEstimator::DEFAULT.estimate(&prompt.text)
    );
    assert_eq!(
        prompt.trim.estimated_after,
        prompt.trim.sections[0].tokens_after
    );
    assert_eq!(prompt.trim.template.name, "capture");
    assert_eq!(prompt.trim.estimator, "chars-v2");
    assert!(prompt.trim.target >= prompt.trim.estimated_after);
    assert_eq!(prompt.trim.v, fixtures::demo_trim_record().v);
    let sections = prompt.payload_sections_value();
    assert_eq!(sections[0]["name"], json!("template"));
    let record = prompt.trim.to_value(&plain()).expect("a clean record");
    assert_eq!(record["template"]["name"], json!("capture"));
}

#[test]
fn the_prompt_carries_the_pack_and_the_rules() {
    let pack = sample_pack();
    let text = capture_prompt(&pack).text;
    assert!(text.contains(pack.text().trim_end()), "{text}");
    for needle in [
        "at most 3 entries",
        "fact, decision, gotcha, lesson",
        "120 characters",
        "2000 characters",
        "at least 12 characters",
        "not a story of the run",
        "Zero entries is a valid answer",
        "document_write",
        r#"{"entries":[{"kind":"#,
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
}

// ---- Parser (D10) ------------------------------------------------------------------------------

const QUOTE: &str = "the sqlx offline data is stale";

fn entry(subject: &str) -> Value {
    json!({
        "kind": "gotcha",
        "subject": subject,
        "body": "Run cargo sqlx prepare against a migrated scratch database after any SQL change.",
        "evidence": [{"block": "S1", "quote": QUOTE}],
    })
}

fn body(entries: &[Value]) -> String {
    json!({ "entries": entries }).to_string()
}

#[test]
fn a_valid_entry_is_kept_with_its_labels_mapped() {
    let pack = sample_pack();
    let mut two = entry("Offline sqlx data");
    two["kind"] = json!("lesson");
    two["evidence"] = json!([
        {"block": "S2", "quote": "  test result: ok. 4 passed "},
        {"block": "S1", "quote": QUOTE},
    ]);
    let out = parse_capture(&body(&[entry("Stale sqlx data"), two]), &pack, &plain())
        .expect("a valid body");
    assert!(out.dropped.is_empty(), "{:?}", out.dropped);
    assert_eq!(out.kept.len(), 2);

    let first = &out.kept[0];
    assert_eq!(first.kind, MemoryKind::Gotcha);
    assert_eq!(first.subject.as_str(), "Stale sqlx data");
    assert!(first.body.as_str().starts_with("Run cargo sqlx prepare"));
    assert_eq!(first.evidence.citations.len(), 1);
    assert_eq!(
        first.evidence.citations[0].step_id,
        pack.step_of("S1").unwrap()
    );
    assert_eq!(first.evidence.citations[0].quote, QUOTE);

    let second = &out.kept[1];
    assert_eq!(second.kind, MemoryKind::Lesson);
    let steps: Vec<StepId> = second
        .evidence
        .citations
        .iter()
        .map(|c| c.step_id)
        .collect();
    assert_eq!(
        steps,
        [pack.step_of("S2").unwrap(), pack.step_of("S1").unwrap()]
    );
    assert_eq!(
        second.evidence.citations[0].quote, "test result: ok. 4 passed",
        "quotes are trimmed"
    );

    let id = MemoryEntryId::new();
    let new = out.kept[0].clone().into_new(id, Some("abc123".to_owned()));
    assert_eq!(new.id, id);
    assert_eq!(new.kind, MemoryKind::Gotcha);
    assert_eq!(new.subject.as_str(), "Stale sqlx data");
    assert_eq!(new.commit.as_deref(), Some("abc123"));
    assert_eq!(new.evidence, out.kept[0].evidence);
}

#[test]
fn zero_entries_is_a_valid_answer() {
    let out = parse_capture(r#"{"entries":[]}"#, &sample_pack(), &plain()).expect("valid");
    assert!(out.kept.is_empty() && out.dropped.is_empty());
}

#[test]
fn a_body_with_unknown_fields_is_refused_whole() {
    let pack = sample_pack();
    let top = json!({"entries": [entry("Stale sqlx data")], "note": "extra"}).to_string();
    assert!(matches!(
        parse_capture(&top, &pack, &plain()),
        Err(CaptureRefusal::Malformed { .. })
    ));
    let mut inner = entry("Stale sqlx data");
    inner["confidence"] = json!(0.9);
    assert!(matches!(
        parse_capture(&body(&[inner]), &pack, &plain()),
        Err(CaptureRefusal::Malformed { .. })
    ));
    let mut cite = entry("Stale sqlx data");
    cite["evidence"] = json!([{"block": "S1", "quote": QUOTE, "line": 3}]);
    assert!(matches!(
        parse_capture(&body(&[cite]), &pack, &plain()),
        Err(CaptureRefusal::Malformed { .. })
    ));
}

#[test]
fn a_body_that_is_not_json_is_refused_whole() {
    let pack = sample_pack();
    for text in ["", "entries: none", "[1, 2", r#"{"entries": "S1"}"#] {
        assert!(
            matches!(
                parse_capture(text, &pack, &plain()),
                Err(CaptureRefusal::Malformed { .. })
            ),
            "{text:?}"
        );
    }
}

#[test]
fn a_body_over_the_cap_is_refused_whole() {
    let pack = sample_pack();
    let mut big = entry("Stale sqlx data");
    big["body"] = json!("word ".repeat(1_100));
    let text = body(&[big]);
    assert!(TokenEstimator::DEFAULT.estimate(&text) > CAPTURE_OUTPUT_TOKEN_CAP);
    assert_eq!(
        parse_capture(&text, &pack, &plain()),
        Err(CaptureRefusal::TooLarge {
            tokens: TokenEstimator::DEFAULT.estimate(&text),
            cap: CAPTURE_OUTPUT_TOKEN_CAP,
        })
    );
}

fn dropped(entry: Value) -> DropReason {
    let out = parse_capture(&body(&[entry]), &sample_pack(), &plain()).expect("a valid body");
    assert!(out.kept.is_empty(), "kept {:?}", out.kept);
    assert_eq!(out.dropped.len(), 1);
    assert_eq!(out.dropped[0].0, 0);
    out.dropped[0].1.clone()
}

#[test]
fn bad_entries_are_dropped_with_a_reason() {
    let mut kind = entry("Stale sqlx data");
    kind["kind"] = json!("rumour");
    assert_eq!(dropped(kind), DropReason::BadKind);

    let mut none = entry("Stale sqlx data");
    none["evidence"] = json!([]);
    assert_eq!(dropped(none), DropReason::NoEvidence);
    let mut absent = entry("Stale sqlx data");
    absent.as_object_mut().unwrap().remove("evidence");
    assert_eq!(dropped(absent), DropReason::NoEvidence);

    let mut block = entry("Stale sqlx data");
    block["evidence"] = json!([{"block": "S9", "quote": QUOTE}]);
    assert_eq!(dropped(block), DropReason::UnknownBlock);

    let mut short = entry("Stale sqlx data");
    short["evidence"] = json!([{"block": "S1", "quote": "  sqlx data  "}]);
    assert_eq!(dropped(short), DropReason::ShortQuote);

    // Present in S2 only, cited against S1.
    let mut elsewhere = entry("Stale sqlx data");
    elsewhere["evidence"] = json!([{"block": "S1", "quote": "test result: ok. 4 passed"}]);
    assert_eq!(dropped(elsewhere), DropReason::QuoteNotFound);
    let mut invented = entry("Stale sqlx data");
    invented["evidence"] = json!([
        {"block": "S1", "quote": QUOTE},
        {"block": "S1", "quote": "a sentence the run never printed"},
    ]);
    assert_eq!(dropped(invented), DropReason::QuoteNotFound);

    let secret = format!("ghp_{}", "a".repeat(36));
    assert!(matches!(
        dropped(entry(&format!("Token {secret}"))),
        DropReason::Subject(_)
    ));
    let mut blank = entry("   ");
    blank["kind"] = json!("fact");
    assert!(matches!(dropped(blank), DropReason::Subject(_)));
    assert!(matches!(
        dropped(entry(&"s".repeat(121))),
        DropReason::Subject(_)
    ));
    let mut long = entry("Stale sqlx data");
    long["body"] = json!("b".repeat(2_001));
    assert!(matches!(dropped(long), DropReason::Body(_)));
}

#[test]
fn a_quote_is_scrubbed_with_the_known_secrets() {
    let lost = failed(1, 0, 1);
    let mut outputs = BTreeMap::new();
    outputs.insert(
        lost.id,
        "login failed with hunter2-password for db".to_owned(),
    );
    let pack = EvidencePack::build(&[lost], &outputs);
    let scrubber = MinimalScrubber::new(["hunter2-password".to_owned()]);
    let mut e = entry("Database login");
    e["evidence"] = json!([{"block": "S1", "quote": "login failed with hunter2-password"}]);
    let out = parse_capture(&body(&[e]), &pack, &scrubber).expect("valid");
    assert_eq!(out.kept.len(), 1, "{:?}", out.dropped);
    let quote = &out.kept[0].evidence.citations[0].quote;
    assert!(!quote.contains("hunter2"), "{quote}");
    assert!(quote.starts_with("login failed with "), "{quote}");
}

#[test]
fn a_quote_the_scrubber_refuses_drops_the_entry() {
    let lost = failed(1, 0, 1);
    let token = format!("ghp_{}", "b".repeat(36));
    let mut outputs = BTreeMap::new();
    outputs.insert(lost.id, format!("push refused for token {token} here"));
    let pack = EvidencePack::build(&[lost], &outputs);
    let mut e = entry("Push token");
    e["evidence"] = json!([{"block": "S1", "quote": format!("refused for token {token}")}]);
    let out = parse_capture(&body(&[e]), &pack, &plain()).expect("valid");
    assert!(out.kept.is_empty());
    assert!(
        matches!(out.dropped[0].1, DropReason::UnscrubbableQuote { .. }),
        "{:?}",
        out.dropped
    );
    assert_eq!(out.dropped[0].1.as_str(), "unscrubbable_quote");
}

#[test]
fn the_fourth_entry_is_dropped_over_the_cap() {
    let entries: Vec<Value> = (0..5).map(|n| entry(&format!("Subject {n}"))).collect();
    let out = parse_capture(&body(&entries), &sample_pack(), &plain()).expect("valid");
    assert_eq!(out.kept.len(), MEMORY_RUN_WRITE_CAP as usize);
    let subjects: Vec<&str> = out.kept.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["Subject 0", "Subject 1", "Subject 2"]);
    assert_eq!(
        out.dropped,
        [(3, DropReason::OverCap), (4, DropReason::OverCap)]
    );
}

#[test]
fn every_drop_reason_has_a_stable_spelling() {
    assert_eq!(DropReason::BadKind.as_str(), "bad_kind");
    assert_eq!(DropReason::NoEvidence.as_str(), "no_evidence");
    assert_eq!(DropReason::UnknownBlock.as_str(), "unknown_block");
    assert_eq!(DropReason::ShortQuote.as_str(), "short_quote");
    assert_eq!(DropReason::QuoteNotFound.as_str(), "quote_not_found");
    assert_eq!(DropReason::OverCap.as_str(), "over_cap");
}
