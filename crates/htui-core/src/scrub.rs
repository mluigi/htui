//! The scrubber seam: mask every known secret before a payload is persisted, refuse the write
//! when something credential-shaped survives.
//!
//! `R-SEC-3` is fail-closed, so [`Scrubber::scrub`] returning [`Unmasked`] means the caller must
//! not persist that value at all (plan MOD-2 D5, `docs/ANA-4.md` §9: "a `Scrubber` … that the
//! recorder calls on every payload and on every `raw` blob before either write path").
//! [`MinimalScrubber`] is the deliberately small built-in that stands in for the real ANA-7
//! scrubber until MOD-10 replaces the implementation behind this unchanged trait.

use serde_json::Value;

/// What every masked occurrence is replaced with.
const REDACTED: &str = "[REDACTED]";

/// Known credential prefixes, as `(rule name, prefix)` pairs, checked in this order.
///
/// The order is the reporting order: `sk-ant-` precedes `sk-` so an Anthropic key is named as one
/// rather than as the generic case. This is the PRD's "known credential prefixes" list and nothing
/// more; ANA-7's real rule set is MOD-10's.
const PREFIX_RULES: &[(&str, &str)] = &[
    ("anthropic_api_key", "sk-ant-"),
    ("github_pat", "github_pat_"),
    ("github_token", "ghp_"),
    ("aws_access_key_id", "AKIA"),
    ("slack_bot_token", "xoxb-"),
    ("slack_user_token", "xoxp-"),
    ("google_api_key", "AIza"),
    ("openai_api_key", "sk-"),
];

/// Rule name for a PEM private key block.
const PEM_RULE: &str = "private_key_pem";
/// The one marker a PEM private key cannot be split away from.
///
/// It appears in *both* the `-----BEGIN … PRIVATE KEY-----` header and the
/// `-----END … PRIVATE KEY-----` footer, so a key streamed across chunks trips the rule at either
/// end even when the dashes were cut by a chunk boundary. Requiring the header *and* the footer in
/// one leaf (the earlier pair check) let a footer-bearing tail persist the key body, so the pair is
/// gone. Matching the bare `-----BEGIN ` dashes was rejected instead: it adds no coverage (a chunk
/// that holds only a partial header holds no key material) while refusing every certificate,
/// CSR and public key, none of which are secrets. The residual false positive is a payload that
/// spells `PRIVATE KEY` in capitals in prose, which fails closed.
const PEM_MARKER: &str = "PRIVATE KEY";

/// Masks known secrets in a payload and reports anything credential-shaped that survived.
///
/// The seam MOD-10 replaces: the recorder holds a `&dyn Scrubber` and never names an
/// implementation.
pub trait Scrubber: Send + Sync + core::fmt::Debug {
    /// Masks `value` in place, then fails closed if a string leaf still matches a credential rule.
    ///
    /// Masking is idempotent: scrubbing an already-scrubbed value changes nothing.
    ///
    /// # Errors
    /// [`Unmasked`] when a string leaf still matches a rule after masking. The caller must drop
    /// the value rather than persist it: a scrubber that could not mask something never lets it
    /// reach a store (`R-SEC-3`).
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked>;
}

/// A string leaf still matched a credential rule after masking.
///
/// Carries a JSON pointer and a rule name and **never the offending text**. This error is logged
/// and is written into a persisted `error` event (plan D6), so its [`core::fmt::Display`] and
/// [`core::fmt::Debug`] output are part of the security contract, not a convenience.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unmasked {rule} at `{path}`")]
pub struct Unmasked {
    /// RFC 6901 JSON pointer to the offending string leaf; `""` is the whole document.
    ///
    /// Object keys in the pointer are masked with the same secret list, and a key that itself
    /// matches a credential rule is reported at its parent object's pointer instead of being
    /// spelled out, so neither a known secret nor a credential-shaped key can reach a log through
    /// this field.
    pub path: String,
    /// Name of the rule that matched, e.g. `"anthropic_api_key"` or `"private_key_pem"`.
    pub rule: &'static str,
}

/// The minimal built-in [`Scrubber`]: exact-match masking plus known credential prefixes.
///
/// Two passes over the document, in this order (plan D5):
///
/// 1. every occurrence of every non-empty secret in every string leaf *and every object key*
///    becomes `[REDACTED]`, longest secret first so a secret that is a prefix of another cannot
///    leave residue;
/// 2. every string leaf and every object key is scanned for the known credential prefixes and for
///    a PEM private-key marker; the first survivor is returned as [`Unmasked`], which blocks the
///    write.
///
/// A prefix counts only at a token start (string start, or after a character that is neither
/// alphanumeric nor `_`), so `subtask-list` is not an `sk-` credential while `Bearer sk-…` is.
/// Numbers and booleans are structural and are never rewritten; strings are masked and scanned
/// wherever they appear, as a value or as a key.
pub struct MinimalScrubber {
    /// Non-empty secrets, deduplicated and sorted longest first.
    secrets: Vec<String>,
}

impl MinimalScrubber {
    /// Builds a scrubber that masks every non-empty `secret`, longest first.
    ///
    /// Empty secrets are dropped: masking on an empty needle would match everywhere. An empty
    /// list is legal and still fails closed on the prefix rules.
    #[must_use]
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> = secrets.into_iter().filter(|s| !s.is_empty()).collect();
        secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        secrets.dedup();
        Self { secrets }
    }

    /// Replaces every occurrence of every secret in `text` with `[REDACTED]`.
    ///
    /// One left-to-right scan taking the longest secret that matches at each position, and
    /// stepping over an existing `[REDACTED]` marker rather than into it. That is what makes the
    /// pass idempotent even when a secret is a substring of the marker.
    fn mask(&self, text: &str) -> String {
        if self.secrets.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(tail) = rest.strip_prefix(REDACTED) {
                out.push_str(REDACTED);
                rest = tail;
                continue;
            }
            if let Some(secret) = self.secrets.iter().find(|s| rest.starts_with(s.as_str())) {
                out.push_str(REDACTED);
                rest = &rest[secret.len()..];
                continue;
            }
            let Some(ch) = rest.chars().next() else { break };
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
        out
    }

    /// Masks every string leaf *and every object key* of `value` in place.
    ///
    /// Keys are masked because a payload can carry a secret as a key just as easily as as a value
    /// (`{"<env secret>": "…"}`), and an unmasked key is persisted verbatim. Rebuilding the map is
    /// what makes that possible; two keys that mask to the same string collapse into one entry,
    /// which drops a value but never keeps a secret.
    fn mask_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.mask(text),
            Value::Array(items) => {
                for item in items {
                    self.mask_value(item);
                }
            }
            Value::Object(map) => {
                let mut masked = serde_json::Map::with_capacity(map.len());
                for (key, mut item) in core::mem::take(map) {
                    self.mask_value(&mut item);
                    masked.insert(self.mask(&key), item);
                }
                *map = masked;
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// Returns the first string leaf *or object key* that still matches a rule, as a JSON pointer
    /// plus rule name.
    ///
    /// `path` is the pointer of `value` itself; it is grown and truncated in place.
    ///
    /// A key that matches a rule is reported at the pointer of its **parent object**, not at its
    /// own pointer: [`Unmasked::path`] is logged and persisted, so interpolating a
    /// credential-shaped key into it would be the leak the type exists to prevent. Checking a key
    /// before descending through it also means every token already in `path` has passed
    /// `residue_rule`.
    fn find_residue(&self, value: &Value, path: &mut String) -> Result<(), Unmasked> {
        match value {
            Value::String(text) => {
                if let Some(rule) = residue_rule(text) {
                    return Err(Unmasked {
                        path: path.clone(),
                        rule,
                    });
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let base = path.len();
                    path.push('/');
                    path.push_str(&index.to_string());
                    self.find_residue(item, path)?;
                    path.truncate(base);
                }
            }
            Value::Object(map) => {
                for (key, item) in map {
                    if let Some(rule) = residue_rule(key) {
                        return Err(Unmasked {
                            path: path.clone(),
                            rule,
                        });
                    }
                    let base = path.len();
                    path.push('/');
                    path.push_str(&escape_token(&self.mask(key)));
                    self.find_residue(item, path)?;
                    path.truncate(base);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }
}

impl core::fmt::Debug for MinimalScrubber {
    /// Prints the secret *count*, never a secret: the type is held by types whose `Debug` reaches
    /// the log (invariant 4 of `docs/ANA-4.md` §4.1).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MinimalScrubber")
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

impl Scrubber for MinimalScrubber {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        self.mask_value(value);
        self.find_residue(value, &mut String::new())
    }
}

/// The rule a string still matches after masking, if any.
fn residue_rule(text: &str) -> Option<&'static str> {
    for (rule, prefix) in PREFIX_RULES {
        if starts_a_token_with(text, prefix) {
            return Some(rule);
        }
    }
    if text.contains(PEM_MARKER) {
        return Some(PEM_RULE);
    }
    None
}

/// Whether `prefix` occurs in `text` at a token start.
///
/// A token starts at the beginning of the string or after a character that is neither
/// alphanumeric nor `_`, so `subtask-42` does not read as an `sk-` credential while `--sk-…` and
/// `Bearer sk-…` do. Erring towards a match is the fail-closed direction.
fn starts_a_token_with(text: &str, prefix: &str) -> bool {
    text.match_indices(prefix).any(|(at, _)| {
        text[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric() && before != '_')
    })
}

/// Escapes one JSON pointer reference token (RFC 6901: `~` → `~0`, `/` → `~1`).
fn escape_token(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scrubber() -> MinimalScrubber {
        MinimalScrubber::new(["alpha-secret".to_string(), "1234".to_string()])
    }

    #[test]
    fn env_values_are_masked_in_every_string_leaf() {
        let mut value = json!({
            "payload": {
                "output": "the token is alpha-secret, use it",
                "nested": ["pin 1234", { "deep": "alpha-secret/1234" }],
                "count": 7,
                "flag": true,
                "nothing": null
            }
        });
        scrubber()
            .scrub(&mut value)
            .expect("masked payload is clean");
        assert_eq!(
            value["payload"]["output"],
            json!("the token is [REDACTED], use it")
        );
        assert_eq!(value["payload"]["nested"][0], json!("pin [REDACTED]"));
        assert_eq!(
            value["payload"]["nested"][1]["deep"],
            json!("[REDACTED]/[REDACTED]")
        );
        assert_eq!(value["payload"]["count"], json!(7));
        let rendered = value.to_string();
        assert!(!rendered.contains("alpha-secret"), "{rendered}");
        assert!(!rendered.contains("1234"), "{rendered}");
    }

    #[test]
    fn a_known_secret_that_looks_like_a_credential_is_masked_not_refused() {
        let scrubber = MinimalScrubber::new(["sk-ant-api03-known".to_string()]);
        let mut value = json!({ "payload": { "output": "using sk-ant-api03-known now" } });
        scrubber
            .scrub(&mut value)
            .expect("a masked secret is not residue");
        assert_eq!(value["payload"]["output"], json!("using [REDACTED] now"));
    }

    #[test]
    fn an_unknown_credential_prefix_fails_closed_with_a_json_pointer() {
        let scrubber = scrubber();
        for (text, rule) in [
            ("sk-ant-api03-aaaaaaaaaaaa", "anthropic_api_key"),
            ("sk-proj-aaaaaaaaaaaaaaaa", "openai_api_key"),
            ("ghp_aaaaaaaaaaaaaaaaaaaa", "github_token"),
            ("github_pat_11AAAAAAA0aaaaaaaaaaaa", "github_pat"),
            ("AKIAAAAAAAAAAAAAAAAA", "aws_access_key_id"),
            ("xoxb-0000000000-aaaaaaaaaaaa", "slack_bot_token"),
            ("xoxp-0000000000-aaaaaaaaaaaa", "slack_user_token"),
            (
                "AIzaSyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "google_api_key",
            ),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----",
                "private_key_pem",
            ),
        ] {
            let mut value = json!({ "payload": { "output": text } });
            let err = scrubber
                .scrub(&mut value)
                .expect_err("residue must refuse the write");
            assert_eq!(err.rule, rule, "rule for {text}");
            assert_eq!(err.path, "/payload/output", "pointer for {text}");
        }
    }

    #[test]
    fn unmasked_never_repeats_the_offending_text() {
        let secret = "sk-ant-api03-do-not-print-me";
        let mut value = json!({ "payload": { "output": format!("here it is: {secret}") } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");

        let display = err.to_string();
        let debug = format!("{err:?}");
        for rendered in [&display, &debug] {
            assert!(!rendered.contains(secret), "leaked: {rendered}");
            assert!(!rendered.contains("do-not-print-me"), "leaked: {rendered}");
            assert!(!rendered.contains("sk-ant-"), "leaked: {rendered}");
        }
        assert!(display.contains("/payload/output"), "{display}");
        assert!(display.contains("anthropic_api_key"), "{display}");
    }

    #[test]
    fn a_masked_secret_never_appears_in_the_error_path() {
        let mut value = json!({ "alpha-secret": { "output": "ghp_aaaaaaaaaaaaaaaaaaaa" } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");
        assert_eq!(err.path, "/[REDACTED]/output");
        assert!(!format!("{err:?}").contains("alpha-secret"));
    }

    #[test]
    fn a_credential_shaped_object_key_is_refused() {
        let mut value = json!({ "payload": { "sk-ant-api03-AAAA": "harmless" } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("a credential-shaped key must refuse the write");
        assert_eq!(err.rule, "anthropic_api_key");
        assert_eq!(err.path, "/payload", "the parent pointer, never the key");
    }

    #[test]
    fn unmasked_never_repeats_a_credential_shaped_key() {
        let key = "sk-ant-api03-do-not-print-me-either";
        let mut value = json!({ "payload": { key: "harmless" } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("a credential-shaped key must refuse the write");

        let display = err.to_string();
        let debug = format!("{err:?}");
        for rendered in [&display, &debug] {
            assert!(!rendered.contains(key), "leaked: {rendered}");
            assert!(!rendered.contains("do-not-print-me"), "leaked: {rendered}");
            assert!(!rendered.contains("sk-ant-"), "leaked: {rendered}");
        }
        assert!(display.contains("anthropic_api_key"), "{display}");
    }

    #[test]
    fn an_env_secret_used_as_an_object_key_is_masked() {
        let mut value = json!({ "output": { "alpha-secret": "harmless", "pin 1234": "also" } });
        scrubber()
            .scrub(&mut value)
            .expect("a masked key is not residue");
        assert_eq!(value["output"]["[REDACTED]"], json!("harmless"));
        assert_eq!(value["output"]["pin [REDACTED]"], json!("also"));
        let rendered = value.to_string();
        assert!(!rendered.contains("alpha-secret"), "{rendered}");
        assert!(!rendered.contains("1234"), "{rendered}");
    }

    #[test]
    fn an_env_secret_key_nested_inside_an_array_is_masked() {
        let mut value = json!({
            "items": [
                { "alpha-secret": ["1234", { "alpha-secret": "deep" }] },
                "plain"
            ]
        });
        scrubber()
            .scrub(&mut value)
            .expect("a masked key is not residue at any depth");
        assert_eq!(
            value["items"][0]["[REDACTED]"][1]["[REDACTED]"],
            json!("deep")
        );
        let rendered = value.to_string();
        assert!(!rendered.contains("alpha-secret"), "{rendered}");
        assert!(!rendered.contains("1234"), "{rendered}");
    }

    #[test]
    fn masking_object_keys_stays_idempotent() {
        let scrubber = scrubber();
        let mut once = json!({ "alpha-secret": { "pin 1234": ["alpha-secret"] } });
        scrubber.scrub(&mut once).expect("clean");
        let mut twice = once.clone();
        scrubber.scrub(&mut twice).expect("clean");
        assert_eq!(once, twice, "scrub twice must equal scrub once");
    }

    #[test]
    fn a_pem_footer_without_a_header_is_refused() {
        let mut value =
            json!({ "payload": { "output": "MIIabcdef\n-----END RSA PRIVATE KEY-----\n" } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("a footer-only PEM chunk must refuse the write");
        assert_eq!(err.rule, "private_key_pem");
        assert_eq!(err.path, "/payload/output");
    }

    #[test]
    fn a_pem_header_without_a_footer_is_refused() {
        let scrubber = scrubber();
        for text in [
            "-----BEGIN RSA PRIVATE KEY-----\nMIIabcdef",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNza",
            // the marker line itself cut mid-dashes by a chunk boundary
            "trailing prose\n-----BEGIN EC PRIVATE KEY",
        ] {
            let mut value = json!({ "payload": { "output": text } });
            let err = scrubber
                .scrub(&mut value)
                .expect_err("a header-only PEM chunk must refuse the write");
            assert_eq!(err.rule, "private_key_pem", "rule for {text}");
            assert_eq!(err.path, "/payload/output", "pointer for {text}");
        }
    }

    #[test]
    fn a_public_pem_block_is_not_a_private_key() {
        let scrubber = scrubber();
        for text in [
            "-----BEGIN CERTIFICATE-----\nMIIabc\n-----END CERTIFICATE-----",
            "-----BEGIN PUBLIC KEY-----\nMIIabc\n-----END PUBLIC KEY-----",
            "-----BEGIN CERTIFICATE REQUEST-----\nMIIabc",
        ] {
            let mut value = json!({ "payload": { "output": text } });
            scrubber
                .scrub(&mut value)
                .unwrap_or_else(|err| panic!("public PEM must not be residue: {err} for {text}"));
        }
    }

    #[test]
    fn the_scrubber_debug_hides_the_secret_list() {
        let rendered = format!("{:?}", scrubber());
        assert!(!rendered.contains("alpha-secret"), "{rendered}");
        assert!(!rendered.contains("1234"), "{rendered}");
        assert!(rendered.contains("MinimalScrubber"), "{rendered}");
    }

    #[test]
    fn a_payload_with_no_strings_is_ok() {
        let mut value = json!({ "a": 1, "b": [true, null, 2.5], "c": {} });
        let before = value.clone();
        scrubber()
            .scrub(&mut value)
            .expect("no strings, nothing to do");
        assert_eq!(value, before);
    }

    #[test]
    fn an_empty_secret_list_masks_nothing_and_still_fails_closed() {
        let scrubber = MinimalScrubber::new(Vec::<String>::new());

        let mut clean = json!({ "text": "nothing to see here" });
        let before = clean.clone();
        scrubber.scrub(&mut clean).expect("no secrets, no residue");
        assert_eq!(clean, before);

        let mut dirty = json!({ "text": "AKIAAAAAAAAAAAAAAAAA" });
        let err = scrubber
            .scrub(&mut dirty)
            .expect_err("prefix rules run with an empty mask list");
        assert_eq!(err.rule, "aws_access_key_id");
        assert_eq!(err.path, "/text");
    }

    #[test]
    fn empty_secrets_are_ignored() {
        let scrubber = MinimalScrubber::new([String::new(), "tok".to_string()]);
        let mut value = json!("a tok here");
        scrubber.scrub(&mut value).expect("clean");
        assert_eq!(value, json!("a [REDACTED] here"));
    }

    #[test]
    fn longer_secrets_mask_before_their_prefixes() {
        let scrubber = MinimalScrubber::new(["abc".to_string(), "abc123".to_string()]);
        let mut value = json!({ "t": "abc123 and abc" });
        scrubber.scrub(&mut value).expect("clean");
        assert_eq!(value["t"], json!("[REDACTED] and [REDACTED]"));
    }

    #[test]
    fn scrubbing_is_idempotent() {
        let scrubber = MinimalScrubber::new([
            "alpha-secret".to_string(),
            "RED".to_string(),
            "1234".to_string(),
        ]);
        let mut once = json!({
            "a": "alpha-secret RED 1234",
            "b": ["x RED", { "c": "plain text" }]
        });
        scrubber.scrub(&mut once).expect("clean");
        let mut twice = once.clone();
        scrubber.scrub(&mut twice).expect("clean");
        assert_eq!(once, twice, "scrub twice must equal scrub once");
    }

    #[test]
    fn a_prefix_inside_a_word_is_not_a_credential() {
        let mut value = json!({ "t": "task-list, subtask-42, whisk-broom" });
        scrubber()
            .scrub(&mut value)
            .expect("a prefix mid-word is not a token start");
    }

    #[test]
    fn a_credential_mid_sentence_is_caught_and_the_root_pointer_is_empty() {
        let mut value = json!("the key is sk-ant-api03-zzzzzzzzzzzz, keep it");
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");
        assert_eq!(err.path, "");
        assert_eq!(err.rule, "anthropic_api_key");
    }

    #[test]
    fn the_pointer_escapes_slashes_and_tildes_and_indexes_arrays() {
        let mut value = json!({ "a/b~c": ["ok", "xoxb-0000000000-aaaaaaaaaaaa"] });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");
        assert_eq!(err.path, "/a~1b~0c/1");
        assert_eq!(err.rule, "slack_bot_token");
    }

    #[test]
    fn the_seam_is_a_send_sync_trait_object() {
        fn takes_dyn(_: &dyn Scrubber) {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<MinimalScrubber>();
        assert_send_sync::<Unmasked>();
        takes_dyn(&scrubber());

        let boxed: Box<dyn Scrubber> = Box::new(scrubber());
        let mut value = json!("plain");
        boxed.scrub(&mut value).expect("clean");
    }
}
