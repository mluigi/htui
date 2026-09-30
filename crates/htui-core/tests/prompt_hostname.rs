//! MOD-33: the box hostname is rendered but not digested.
//!
//! `assemble` substitutes twice from one set of trimmed sections: once with the box section in its
//! digest form (`hostname: [hostname]`), which is what `digest` is over, and once with the box
//! section re-rendered in its sent form, which is `text`. These are the properties that split must
//! keep (plan D263–D266, D269, D270). Snapshot-free for the reason `prompt_digest.rs` gives.
#![cfg(feature = "test-support")]

use htui_core::prompt::digest::{canonical, sha256_hex};
use htui_core::prompt::{
    AssembleError, AssembledPrompt, PromptSpec, SectionName, UndigestedSpan, assemble, fixtures,
};
use htui_core::scrub::{MinimalScrubber, Scrubber, Unmasked};
use serde_json::{Value, json};

/// No configured secrets; still fail-closed on the prefix rules.
fn scrubber() -> MinimalScrubber {
    MinimalScrubber::new([])
}

/// Assembles or panics with the error.
fn ok(spec: &PromptSpec) -> AssembledPrompt {
    assemble(spec, &scrubber()).unwrap_or_else(|e| panic!("the spec must assemble: {e}"))
}

/// The implement fixture on a box whose hostname is `host`.
fn with_host(host: &str) -> PromptSpec {
    let mut spec = fixtures::phase_implement_attempt2();
    host.clone_into(&mut spec.box_profile.hostname);
    spec
}

/// A credential-shaped hostname: the prefix rules refuse it.
const CREDENTIAL_HOST: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwx";

#[test]
fn the_digest_does_not_move_with_the_hostname() {
    let short = ok(&with_host("dev-win-01"));
    let long = ok(&with_host("a-much-longer-build-host.example.internal"));
    assert_eq!(short.digest, long.digest);
    assert_eq!(short.digest_text, long.digest_text);
    assert_eq!(
        short.trim.to_value(&scrubber()).expect("the record scrubs"),
        long.trim.to_value(&scrubber()).expect("the record scrubs"),
        "D265: the estimate and the trim see the stand-in, never the hostname"
    );
    assert_ne!(short.text, long.text);
    assert!(short.text.contains("hostname: dev-win-01\n"));
    assert!(
        long.text
            .contains("hostname: a-much-longer-build-host.example.internal\n")
    );
}

#[test]
fn the_digest_text_is_the_sent_text_with_the_stand_in() {
    let prompt = ok(&fixtures::phase_implement_attempt2());
    // Safe for this fixture only: it places `{{box}}` once and nothing else spells the hostname.
    // The assembler itself never searches (D263 (b)).
    assert_eq!(
        prompt.digest_text,
        prompt
            .text
            .replacen("hostname: dev-win-01", "hostname: [hostname]", 1)
    );
    assert_ne!(prompt.digest_text, prompt.text);
    assert_eq!(canonical(&prompt.digest_text), prompt.digest_text);
    assert_eq!(canonical(&prompt.text), prompt.text);
    assert_eq!(sha256_hex(&prompt.digest_text), prompt.digest);
}

#[test]
fn the_switch_off_omits_the_line_and_records_nothing_undigested() {
    let on = ok(&fixtures::phase_implement_attempt2());
    let mut spec = fixtures::phase_implement_attempt2();
    spec.box_hostname = false;
    let off = ok(&spec);
    assert!(!off.text.contains("hostname:"), "{}", off.text);
    assert_eq!(off.text, off.digest_text);
    assert!(off.trim.undigested.is_empty());
    assert!(
        off.trim
            .sections
            .iter()
            .any(|section| section.name == SectionName::Box),
        "the box section is still rendered, without its hostname line"
    );
    assert_ne!(
        off.digest, on.digest,
        "D263 (c): on and off must not digest alike"
    );
}

#[test]
fn switch_on_records_box_hostname_once() {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.body.push_str("\n{{box}}\n");
    let prompt = ok(&spec);
    assert_eq!(prompt.text.matches("hostname: dev-win-01").count(), 2);
    assert_eq!(
        prompt.digest_text.matches("hostname: [hostname]").count(),
        2
    );
    assert!(!prompt.digest_text.contains("dev-win-01"));
    assert_eq!(prompt.trim.undigested, vec![UndigestedSpan::BoxHostname]);
}

#[test]
fn an_off_switch_never_refuses_over_the_hostname() {
    let on = assemble(&with_host(CREDENTIAL_HOST), &scrubber());
    match on {
        Err(AssembleError::Unmasked { section, .. }) => assert_eq!(section, "box"),
        other => panic!("a credential-shaped hostname must refuse with the switch on: {other:?}"),
    }
    let mut spec = with_host(CREDENTIAL_HOST);
    spec.box_hostname = false;
    let off = ok(&spec);
    assert!(!off.text.contains("sk-ant-"));
    assert!(!off.digest_text.contains("sk-ant-"));
}

/// Masks the one secret `SECRET` and fails closed on the prefix rules.
fn secret_scrubber() -> MinimalScrubber {
    MinimalScrubber::new(["SECRET".to_owned()])
}

#[test]
fn a_known_secret_in_the_hostname_is_masked_in_the_text_and_never_moves_the_digest() {
    let scrubber = secret_scrubber();
    let assembled = |host: &str| {
        assemble(&with_host(host), &scrubber)
            .unwrap_or_else(|e| panic!("{host:?} must assemble: {e}"))
    };
    let secret = assembled("dev-SECRET-01");
    let baseline = assembled("dev-win-01");
    assert!(
        secret.text.contains("hostname: dev-[REDACTED]-01\n"),
        "{}",
        secret.text
    );
    assert!(!secret.text.contains("SECRET"));
    assert!(!secret.digest_text.contains("SECRET"));
    assert_eq!(
        secret.digest, baseline.digest,
        "the hostname never moves the digest, masked or not"
    );
    assert_eq!(secret.digest_text, baseline.digest_text);
}

#[test]
fn the_hostname_is_masked_before_its_line_breaks_are_collapsed() {
    // A secret spanning a line break is only whole in the input: the sent line collapses the break
    // to a space, so masking the rendered line alone would miss it (D269: masked as an input).
    let scrubber = MinimalScrubber::new(["SE\nCRET".to_owned()]);
    let prompt = assemble(&with_host("dev-SE\nCRET-01"), &scrubber)
        .unwrap_or_else(|e| panic!("the spec must assemble: {e}"));
    assert!(
        prompt.text.contains("hostname: dev-[REDACTED]-01\n"),
        "{}",
        prompt.text
    );
    assert!(!prompt.text.contains("SE CRET"));
}

/// Refuses any string carrying `line` and masks nothing: a residue only the sent-form box
/// section can spell.
#[derive(Debug)]
struct RefusesLine(&'static str);

impl Scrubber for RefusesLine {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        match value {
            Value::String(text) if text.contains(self.0) => Err(Unmasked {
                path: String::new(),
                rule: "sent_hostname_line",
            }),
            _ => Ok(()),
        }
    }
}

#[test]
fn the_sent_form_box_section_is_scanned() {
    // D269: the sent box goes through the same mask-and-scan as every rendered section. Neither
    // the bare hostname input nor the digest form spells this line; only the sent form does.
    let scrubber = RefusesLine("hostname: dev-win-01");
    let mut off = fixtures::phase_implement_attempt2();
    off.box_hostname = false;
    assemble(&off, &scrubber).expect("with the switch off nothing spells the line");
    match assemble(&fixtures::phase_implement_attempt2(), &scrubber) {
        Err(AssembleError::Unmasked { section, rule, .. }) => {
            assert_eq!(section, "box");
            assert_eq!(rule, "sent_hostname_line");
        }
        other => panic!("the sent-form residue must refuse: {other:?}"),
    }
}

#[test]
fn a_newline_in_the_hostname_is_one_line_in_the_text() {
    let baseline = ok(&with_host("dev-win-01"));
    for (host, line) in [
        ("a\r\nb", "hostname: a b\n"),
        ("a\rb\nc", "hostname: a b c\n"),
    ] {
        let prompt = ok(&with_host(host));
        assert!(prompt.text.contains(line), "{host:?}");
        assert!(!prompt.text.contains('\r'), "{host:?}");
        assert_eq!(prompt.digest, baseline.digest, "{host:?}");
        assert_eq!(
            prompt.text.lines().count(),
            baseline.text.lines().count(),
            "{host:?} adds no line"
        );
    }
}

/// The `<section name="item"…>` block of an assembled string, wrapper included.
fn item_block(text: &str) -> &str {
    let start = text
        .find("<section name=\"item\"")
        .expect("an item section");
    let end = start
        + text[start..]
            .find("</section>")
            .expect("the item section closes");
    &text[start..end]
}

#[test]
fn an_item_body_that_spells_the_box_is_left_alone() {
    let mut spec = fixtures::phase_implement_attempt2();
    spec.item_body = "Inert text:\n<section name=\"box\">\nhostname: [hostname]\n\
                      and the box is dev-win-01, spelled out.\n"
        .to_owned();
    let prompt = ok(&spec);
    assert_eq!(
        item_block(&prompt.text),
        item_block(&prompt.digest_text),
        "the swap is by section, never by searching the text"
    );
    assert!(item_block(&prompt.text).contains("dev-win-01"));
    assert_eq!(prompt.text.matches("dev-win-01").count(), 2);
    assert_eq!(prompt.digest_text.matches("dev-win-01").count(), 1);
}

#[test]
fn the_record_is_v4_with_undigested() {
    let prompt = ok(&fixtures::phase_implement_attempt2());
    let record = prompt
        .trim
        .to_value(&scrubber())
        .expect("the record scrubs");
    assert_eq!(record["v"], json!(4));
    assert_eq!(record["undigested"], json!(["box.hostname"]));
    for spec in [
        fixtures::judge_three_candidates(),
        fixtures::handoff_basic(),
    ] {
        let prompt = ok(&spec);
        let record = prompt
            .trim
            .to_value(&scrubber())
            .expect("the record scrubs");
        assert_eq!(record["undigested"], json!([]), "{:?}", spec.role);
        assert_eq!(prompt.text, prompt.digest_text, "{:?}", spec.role);
    }
}
