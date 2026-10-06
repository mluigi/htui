//! The scrubber seam: mask every known secret before a payload is persisted, refuse the write
//! when something credential-shaped survives.
//!
//! `R-SEC-3` is fail-closed, so [`Scrubber::scrub`] returning [`Unmasked`] means the caller must
//! not persist that value at all (plan MOD-2 D5, `docs/ANA-4.md` §9: "a `Scrubber` … that the
//! recorder calls on every payload and on every `raw` blob before either write path").
//! [`MinimalScrubber`] is the built-in implementation behind this trait (the name predates
//! MOD-10 and is kept): exact-match masking of known secrets, then whole-token pattern rules for
//! known credential formats (MOD-10 D1/D2) plus a PEM private-key marker.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use serde_json::Value;
use zeroize::Zeroize;

/// What every masked occurrence is replaced with.
const REDACTED: &str = "[REDACTED]";

/// ANA-7 §3.4's floor: a resolved value shorter than this many characters is injected but not
/// masked, because masking a 3-character value would shred every transcript (MOD-10 D3).
///
/// Applies only to [`MinimalScrubber::from_resolved`]; [`MinimalScrubber::new`] masks every
/// non-empty value it is given.
pub const MIN_MASKED_LEN: usize = 6;

/// Whole-token credential rules, as `(rule name, pattern body)`, in reporting order (MOD-10 D1,
/// D2).
///
/// Each body is a prefix plus a charset plus a minimum length, so a bare prefix in prose
/// (`sk-learn`, `ghp_short`, `AKIA` alone) is not a credential. It is anchored at an ASCII token
/// start by [`TOKEN_START`] when compiled. Names are persisted in `scrub_residue` rows, so an
/// existing name never changes. The order is the reporting order: when one string trips two rules
/// the first in this table is named, so the eight pre-MOD-10 names keep their old order.
const PATTERN_RULES: &[(&str, &str)] = &[
    ("anthropic_api_key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
    ("github_pat", r"github_pat_[A-Za-z0-9_]{20,}"),
    ("github_token", r"gh[pousr]_[A-Za-z0-9]{30,}"),
    ("aws_access_key_id", r"(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}"),
    ("slack_bot_token", r"xoxb-[A-Za-z0-9-]{10,}"),
    ("slack_user_token", r"xoxp-[A-Za-z0-9-]{10,}"),
    ("google_api_key", r"AIza[0-9A-Za-z_-]{35}"),
    // A wide gate only: any `sk-` token with a 20+ character URL-safe body. The set match alone
    // never refuses; [`residue_rule`] confirms it with [`openai_key_in`], which refuses on
    // [`OPENAI_STRICT`] or on any gated body that is neither an `ant-` body (left to
    // `anthropic_api_key`) nor word-shaped prose ([`SK_PROSE`], e.g.
    // `sk-learn-preprocessing-pipeline-v2`). So LiteLLM virtual keys (`token_urlsafe` bodies)
    // and `sk-<uuid4>` keys stay refused under this name, as the bare prefix refused them.
    ("openai_api_key", r"sk-[A-Za-z0-9_-]{20,}"),
    // New in MOD-10 (D2).
    ("gitlab_pat", r"glpat-[A-Za-z0-9_-]{20,}"),
    ("slack_token", r"xox[ars]-[A-Za-z0-9-]{10,}"),
    ("stripe_secret_key", r"[rs]k_live_[A-Za-z0-9]{20,}"),
    ("npm_token", r"npm_[A-Za-z0-9]{36}"),
    ("pypi_token", r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}"),
    (
        "sendgrid_api_key",
        r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}",
    ),
    (
        "jwt",
        r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
    ),
];

/// A token start (MOD-10 D1, D17): the string start, one character that is not
/// `[A-Za-z0-9_]`, or a JSON / percent escape whose last character is a letter or digit: `\n`,
/// `\r`, `\t`, `\b`, `\f`, `\"`, `\/`, `\\`, `\uXXXX` and `%XX`. A key serialised inside an
/// escaped string (`…\nsk-ant-…`, `%22ghp_…`) is therefore a whole token, while `subtask-…` and
/// `x%2Fsk-learn` stay prose (the latter is too short for any rule). It widens what fails closed;
/// `scripts/scrub-audit.sql` mirrors it (OQ-D: the host audit runs before merge).
///
/// `Bearer sk-…`, `--sk-…` and `[REDACTED]sk-…` are token starts too. A non-ASCII letter before a
/// key (`éAKIA…`) also counts, which errs towards failing closed. The `\"`, `\/` and `\\`
/// alternatives are redundant with `[^A-Za-z0-9_]` (the backslash before them already is one) and
/// are kept so the constant reads as the escape list D17 names.
const TOKEN_START: &str = r#"(?:^|[^A-Za-z0-9_]|\\[nrtbf"/\\]|\\u[0-9A-Fa-f]{4}|%[0-9A-Fa-f]{2})"#;

/// [`PATTERN_RULES`] compiled once per process, index for index.
static PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new(
        PATTERN_RULES
            .iter()
            .map(|(_, body)| format!("{TOKEN_START}(?:{body})")),
    )
    .expect("the pattern rules are literals and compile")
});

/// Index of `openai_api_key` in [`PATTERN_RULES`]: the one gate that needs [`openai_key_in`].
const OPENAI_RULE: usize = 7;

/// `sk-` keys that always refuse: the hyphen-free legacy form, plus the named hyphenated vendor
/// segments the bare prefix used to refuse under `openai_api_key` (OpenAI project,
/// service-account, admin and user-scoped `None` keys, OpenRouter `or-v1`, Langfuse `lf`).
static OPENAI_STRICT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{TOKEN_START}sk-(?:(?:proj|svcacct|admin|None|or-v[0-9]+|lf)-[A-Za-z0-9_-]{{20,}}|[A-Za-z0-9]{{20,}})"
    ))
    .expect("a literal pattern compiles")
});

/// Every `sk-` token the `openai_api_key` gate admits, with its body captured.
static SK_CANDIDATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"{TOKEN_START}sk-([A-Za-z0-9_-]{{20,}})"))
        .expect("a literal pattern compiles")
});

/// An `sk-` body that reads as prose: `-`/`_`-separated segments, each either words (an optional
/// capital, then lowercase, repeated: `learn`, `StandardScaler`) with optional trailing digits
/// (`py311`, `x86`), or digits alone (`2024`). Random key bodies almost never fit.
static SK_PROSE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(?:[A-Z]?[a-z]+)+[0-9]*|[0-9]+)(?:[-_](?:(?:[A-Z]?[a-z]+)+[0-9]*|[0-9]+))*$")
        .expect("a literal pattern compiles")
});

/// Whether `text` holds an `openai_api_key`: a strict hit, or a gated `sk-` body that is neither
/// an `ant-` body nor prose.
fn openai_key_in(text: &str) -> bool {
    OPENAI_STRICT.is_match(text)
        || SK_CANDIDATE.captures_iter(text).any(|caps| {
            let body = &caps[1];
            !body.starts_with("ant-") && !SK_PROSE.is_match(body)
        })
}

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

/// The longest not-yet-matching prefix any pattern rule can leave at the end of a text, plus the
/// longest token start in front of it (MOD-10 D18, blueprint A-4): `pypi_token`'s minimum match
/// is 70 bytes (`pypi-AgEIcHlwaS5vcmc` + 50) and `\uXXXX` is 6. A text cut at least
/// `PATTERN_HOLD_BACK - 1` bytes before its end therefore never splits a credential the rules
/// would have caught whole. A rule whose minimum grows must grow this.
pub const PATTERN_HOLD_BACK: usize = 76;

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

    /// MOD-10 D18: how many trailing bytes of an open text run a size-triggered cut must keep
    /// open, so that a secret or a credential still arriving is never split across two rows.
    /// `0` (the default) keeps the recorder's cut at the bound exactly, as before MOD-10 M3.
    fn hold_back(&self) -> usize {
        0
    }
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

/// The built-in [`Scrubber`]: exact-match masking plus whole-token pattern rules.
///
/// Two passes over the document, in this order (plan D5):
///
/// 1. every occurrence of every non-empty secret in every string leaf *and every object key*
///    becomes `[REDACTED]`, longest secret first so a secret that is a prefix of another cannot
///    leave residue;
/// 2. every string leaf and every object key is scanned for the whole-token pattern rules and for
///    a PEM private-key marker; the first survivor is returned as [`Unmasked`], which blocks the
///    write.
///
/// A pattern rule is a known prefix plus a charset plus a minimum length, and counts only at a
/// token start (string start, after a character that is not `[A-Za-z0-9_]`, or after a JSON or
/// percent escape such as `\n` or `%22`: [`TOKEN_START`]), so `subtask-list` and `sk-learn` are
/// not credentials while `Bearer sk-ant-api03-…` and `…\nsk-ant-api03-…` are.
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
    /// list is legal and still fails closed on the pattern rules. A duplicate is zeroized as it
    /// is dropped, like the whole list when the scrubber goes (MOD-10 D17).
    #[must_use]
    pub fn new(secrets: impl IntoIterator<Item = String>) -> Self {
        let mut secrets: Vec<String> = secrets.into_iter().filter(|s| !s.is_empty()).collect();
        secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        // `dedup_by` hands the candidate for removal first.
        secrets.dedup_by(|dropped, kept| {
            if dropped == kept {
                dropped.zeroize();
                true
            } else {
                false
            }
        });
        Self { secrets }
    }

    /// A scrubber over a resolved `key → value` map (MOD-10 D3).
    ///
    /// Masks every value of at least [`MIN_MASKED_LEN`] characters (`chars().count()`). Returns
    /// the **key names** of the values below the floor, in map order; those values are still
    /// injected by the caller, just not masked. An empty value is below the floor and is listed.
    /// No value is ever returned or logged. An empty map is legal and still fails closed on the
    /// pattern rules.
    ///
    /// A value that ends in `\n` / `\r\n` (a secret stored with its line end) is masked both as
    /// given and with its trailing line ends trimmed, so the bare token an agent echoes is masked
    /// too (MOD-10 D17). The trimmed form is masked only when it is itself at the floor; whether a
    /// key is listed as short is decided on its full value. Injection is untouched: the caller
    /// injects `resolved` byte for byte.
    #[must_use]
    pub fn from_resolved(resolved: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut masked = Vec::with_capacity(resolved.len());
        let mut short = Vec::new();
        for (key, value) in resolved {
            if value.chars().count() >= MIN_MASKED_LEN {
                masked.push(value.clone());
                let trimmed = value.trim_end_matches(['\r', '\n']);
                if trimmed.len() != value.len() && trimmed.chars().count() >= MIN_MASKED_LEN {
                    masked.push(trimmed.to_owned());
                }
            } else {
                short.push(key.clone());
            }
        }
        (Self::new(masked), short)
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

impl Drop for MinimalScrubber {
    /// MOD-10 D17: the secret list is wiped when the scrubber goes (a walk's, a chat's).
    fn drop(&mut self) {
        for secret in &mut self.secrets {
            secret.zeroize();
        }
    }
}

impl Scrubber for MinimalScrubber {
    fn scrub(&self, value: &mut Value) -> Result<(), Unmasked> {
        self.mask_value(value);
        self.find_residue(value, &mut String::new())
    }

    /// `max(longest secret in bytes, PATTERN_HOLD_BACK) - 1`: 75 with no secret.
    fn hold_back(&self) -> usize {
        self.secrets
            .first() // sorted longest first by byte length (`new`)
            .map_or(0, String::len)
            .max(PATTERN_HOLD_BACK)
            - 1
    }
}

/// The rule a string still matches after masking, if any.
///
/// `is_match` is the cheap gate (clean leaves dominate); only a hit pays for `matches`, whose
/// indices come back in ascending order, so the first one that holds is the first rule in table
/// order. Every set hit holds except `openai_api_key`, a wide gate confirmed by [`openai_key_in`].
fn residue_rule(text: &str) -> Option<&'static str> {
    if PATTERNS.is_match(text)
        && let Some(index) = PATTERNS
            .matches(text)
            .iter()
            .find(|&index| index != OPENAI_RULE || openai_key_in(text))
    {
        return Some(PATTERN_RULES[index].0);
    }
    text.contains(PEM_MARKER).then_some(PEM_RULE)
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

    /// No masking at all, so a fixture's digits never collide with `scrubber()`'s `1234` and only
    /// the pattern rules decide.
    fn rules_only() -> MinimalScrubber {
        MinimalScrubber::new(Vec::<String>::new())
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
            ("sk-ant-api03-aaaaaaaaaaaaaaaaaaaa", "anthropic_api_key"),
            ("sk-proj-aaaaaaaaaaaaaaaaaaaa", "openai_api_key"),
            ("ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "github_token"),
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
        let mut value =
            json!({ "alpha-secret": { "output": "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } });
        let err = scrubber()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");
        assert_eq!(err.path, "/[REDACTED]/output");
        assert!(!format!("{err:?}").contains("alpha-secret"));
    }

    #[test]
    fn a_credential_shaped_object_key_is_refused() {
        let mut value = json!({ "payload": { "sk-ant-api03-AAAAAAAAAAAAAAAAAAAA": "harmless" } });
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
            .expect_err("pattern rules run with an empty mask list");
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
    fn every_pattern_rule_compiles_alone() {
        for (rule, body) in PATTERN_RULES {
            Regex::new(&format!("{TOKEN_START}(?:{body})"))
                .unwrap_or_else(|err| panic!("rule {rule} does not compile: {err}"));
        }
        assert_eq!(PATTERNS.len(), PATTERN_RULES.len());
    }

    #[test]
    fn the_openai_rule_index_names_the_openai_rule() {
        assert_eq!(PATTERN_RULES[OPENAI_RULE].0, "openai_api_key");
    }

    #[test]
    fn every_pattern_rule_has_a_real_shaped_fixture() {
        let names: Vec<&str> = PATTERN_RULES.iter().map(|(rule, _)| *rule).collect();
        let covered: Vec<&str> = REAL_SHAPED.iter().map(|(rule, _)| *rule).collect();
        assert_eq!(names, covered, "one fixture per rule, in table order");
    }

    /// One real-shaped value per pattern rule (MOD-10 D2), each well past its rule's minimum.
    const REAL_SHAPED: &[(&str, &str)] = &[
        (
            "anthropic_api_key",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123",
        ),
        ("github_pat", "github_pat_11ABCDEFG0abcdefghijklmnopqrstuv"),
        ("github_token", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
        ("aws_access_key_id", "AKIAIOSFODNN7EXAMPLE"),
        ("slack_bot_token", "xoxb-0000000000-aaaaaaaaaaaa"),
        ("slack_user_token", "xoxp-0000000000-aaaaaaaaaaaa"),
        ("google_api_key", "AIzaSyA0123456789abcdefghijklmnopqrstuv"),
        ("openai_api_key", "sk-proj-abcdefghijklmnopqrstuvwxyz"),
        ("gitlab_pat", "glpat-abcdefghijklmnopqrst"),
        ("slack_token", "xoxa-0000000000-aaaaaaaaaaaa"),
        ("stripe_secret_key", "sk_live_abcdefghijklmnopqrstuvwx"),
        ("npm_token", "npm_abcdefghijklmnopqrstuvwxyz0123456789"),
        (
            "pypi_token",
            "pypi-AgEIcHlwaS5vcmcabcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz",
        ),
        (
            "sendgrid_api_key",
            "SG.abcdefghijklmnopqrstuv.abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
        ),
        (
            "jwt",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnopqrstuvwxyz",
        ),
    ];

    #[test]
    fn each_rule_refuses_a_real_shaped_key_in_four_positions() {
        let scrubber = rules_only();
        for (rule, key) in REAL_SHAPED {
            for text in [
                (*key).to_owned(),
                format!("the key {key} here"),
                format!("Authorization: Bearer {key}"),
            ] {
                let mut value = json!({ "payload": { "output": text } });
                let err = scrubber
                    .scrub(&mut value)
                    .err()
                    .unwrap_or_else(|| panic!("a real-shaped {rule} key must refuse the write"));
                assert_eq!(err.rule, *rule, "rule for {rule} in a value");
                assert_eq!(err.path, "/payload/output", "pointer for {rule}");
            }
            let mut value = json!({ "payload": { *key: "harmless" } });
            let err = scrubber
                .scrub(&mut value)
                .err()
                .unwrap_or_else(|| panic!("a real-shaped {rule} object key must refuse the write"));
            assert_eq!(err.rule, *rule, "rule for {rule} as a key");
            assert_eq!(err.path, "/payload", "the parent pointer for {rule}");
        }
    }

    /// One value per quantifier minimum, exactly one character short of it (sendgrid and jwt once
    /// per segment), under its rule's name.
    fn one_short() -> Vec<(&'static str, String)> {
        let a = |n: usize| "a".repeat(n);
        vec![
            ("anthropic_api_key", format!("sk-ant-{}", a(19))),
            ("github_pat", format!("github_pat_{}", a(19))),
            ("github_token", format!("ghp_{}", a(29))),
            ("aws_access_key_id", format!("AKIA{}", "A".repeat(15))),
            ("slack_bot_token", format!("xoxb-{}", "0".repeat(9))),
            ("slack_user_token", format!("xoxp-{}", "0".repeat(9))),
            ("google_api_key", format!("AIza{}", a(34))),
            ("openai_api_key", format!("sk-{}", a(19))),
            ("openai_api_key", format!("sk-proj-{}", a(19))),
            ("gitlab_pat", format!("glpat-{}", a(19))),
            ("slack_token", format!("xoxa-{}", "0".repeat(9))),
            ("stripe_secret_key", format!("sk_live_{}", a(19))),
            ("npm_token", format!("npm_{}", a(35))),
            ("pypi_token", format!("pypi-AgEIcHlwaS5vcmc{}", a(49))),
            ("sendgrid_api_key", format!("SG.{}.{}", a(21), a(43))),
            ("sendgrid_api_key", format!("SG.{}.{}", a(22), a(42))),
            ("jwt", format!("eyJ{}.eyJ{}.{}", a(9), a(10), a(10))),
            ("jwt", format!("eyJ{}.eyJ{}.{}", a(10), a(9), a(10))),
            ("jwt", format!("eyJ{}.eyJ{}.{}", a(10), a(10), a(9))),
        ]
    }

    #[test]
    fn one_character_short_of_every_minimum_is_clean_under_all_rules() {
        let scrubber = rules_only();
        for (rule, short) in one_short() {
            for text in [
                short.clone(),
                format!("the key {short} here"),
                format!("Authorization: Bearer {short}"),
            ] {
                let mut value = json!({ "t": text });
                scrubber.scrub(&mut value).unwrap_or_else(|err| {
                    panic!(
                        "one short of {rule} must be clean, got {}: {text}",
                        err.rule
                    )
                });
            }
        }
    }

    #[test]
    fn every_pattern_rule_has_a_one_short_fixture() {
        let fixtures = one_short();
        for (rule, _) in PATTERN_RULES {
            assert!(
                fixtures.iter().any(|(name, _)| name == rule),
                "no one-short fixture for {rule}"
            );
        }
    }

    /// Stripe test-mode keys are not refused (maintainer decision, MOD-10 M1 review M1): they cannot
    /// move money, and Stripe's documentation sample key appears in code and READMEs an agent reads.
    #[test]
    fn stripe_test_mode_keys_are_not_refused() {
        let scrubber = rules_only();
        for text in [
            "sk_test_abcdefghijklmnopqrstuvwx",
            "rk_test_abcdefghijklmnopqrstuvwx",
            "Stripe.api_key = \"sk_test_4eC39HqLyjWDarjtT1zdp7dc\"",
        ] {
            let mut value = json!({ "t": text });
            scrubber
                .scrub(&mut value)
                .unwrap_or_else(|err| panic!("{text:?} must stay clean, got {err}"));
        }
    }

    #[test]
    fn every_slack_token_kind_and_stripe_key_kind_is_refused() {
        let scrubber = rules_only();
        for (text, rule) in [
            ("xoxr-0000000000-aaaaaaaaaaaa", "slack_token"),
            ("xoxs-0000000000-aaaaaaaaaaaa", "slack_token"),
            ("rk_live_abcdefghijklmnopqrstuvwx", "stripe_secret_key"),
            ("gho_abcdefghijklmnopqrstuvwxyz0123456789", "github_token"),
            ("ghs_abcdefghijklmnopqrstuvwxyz0123456789", "github_token"),
            ("ASIAIOSFODNN7EXAMPLE", "aws_access_key_id"),
            ("sk-svcacct-abcdefghijklmnopqrstuvwxyz", "openai_api_key"),
            ("sk-abcdefghijklmnopqrstuvwxyz0123", "openai_api_key"),
        ] {
            let mut value = json!({ "t": text });
            let err = scrubber
                .scrub(&mut value)
                .expect_err("a real-shaped key must refuse the write");
            assert_eq!(err.rule, rule, "rule for {rule}");
        }
    }

    /// `sk-` keys whose body opens with a short hyphenated vendor segment (OpenRouter, OpenAI
    /// user-scoped, Langfuse), plus a LiteLLM virtual key and an `sk-<uuid4>` key. The bare-prefix
    /// rule refused all of them as `openai_api_key`; the whole-token rule must keep doing so, under
    /// the same name.
    #[test]
    fn hyphenated_vendor_sk_keys_are_still_refused_as_openai() {
        let scrubber = rules_only();
        for key in [
            "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "sk-None-abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRST0123",
            "sk-lf-1234abcd-12ab-34cd-56ef-1234567890ab",
            // A LiteLLM virtual key (`sk-` + `secrets.token_urlsafe(16)`) and an `sk-<uuid4>` key.
            "sk-Ab3_xY9-kLmN0pQrStUvWx",
            "sk-1b4e28ba-2fa1-11d2-883f-0016d3cca427",
        ] {
            for text in [
                key.to_owned(),
                format!("the key {key} here"),
                format!("Authorization: Bearer {key}"),
            ] {
                let mut value = json!({ "t": text });
                let err = scrubber
                    .scrub(&mut value)
                    .expect_err("a hyphenated vendor sk- key must refuse the write");
                assert_eq!(err.rule, "openai_api_key", "rule for {key}");
            }
        }
    }

    /// LiteLLM virtual keys: `sk-` + Python `secrets.token_urlsafe(16)`, 22 URL-safe base64
    /// characters. Generated once with that call, so roughly half carry a `-` or `_` the
    /// hyphen-free legacy form cannot see.
    const LITELLM_KEYS: &[&str] = &[
        "sk-azlCm5Hgj1tql0YuyTTtEQ",
        "sk-WboMMTvsTp7-LGJtbJppkQ",
        "sk-UVDOuCJwC1Y_9hZPjvyYKQ",
        "sk-AdDm9_ti3KL1WuYgGJ_VmQ",
        "sk-qQ4ZHehxkVBhRl_7-o05Hw",
        "sk-e2wyYk2B4jNkKm31wSjE7A",
        "sk-FsRN_fUN_6yeQLX5C0tDww",
        "sk-CHEPrtIa8Shs-lWbzuwlGQ",
        "sk-IyDFVTtNpKytzcit9Ui55g",
        "sk-ZOpihlALQ0-poFS2uaF8Yw",
        "sk-fXIVi4rxZCClaoYQnKmMuQ",
        "sk-gdvxQAgQEK0AJA516oyPeA",
        "sk-urdnfAKhq8YQqa4XSaLELQ",
        "sk-l7yOopBiQy5GX9d4kUbGwQ",
        "sk-hBMqJXpQm6ajCebTVgKiig",
        "sk-65HN9oWR3PwhZaiQEFr-BQ",
        "sk-9LD212pxNuHTETFZtnDusA",
        "sk-Eh-us4w8J9PYjRg57KtmGw",
        "sk-1lN5jOPBWIfdHa25xKwYcA",
        "sk-fq0y51fvCwkU0yVyOX-J9g",
        "sk-kf2SOIYvkGyTtEFrQupbPg",
        "sk-tuDCyKvJm8FaNVXfhZsunQ",
        "sk-C875ktqnml9WsKhUrfizQg",
        "sk-tTVV1vcFFuo4ATYCk5ZqRQ",
        "sk-g-XVa2U_d1gX-r-FqLdLlw",
        "sk-02UWGoUEKjGg3AnRgxjcHA",
        "sk-T1yhtfSyC760PRmmLBR1nA",
        "sk-xhJyi-PbETXQ90WfUptfPw",
        "sk-LjvBnymJeIXRCphZC_0CVQ",
        "sk-pTmPnZuhMgFCVZOfCnbZag",
    ];

    #[test]
    fn litellm_virtual_keys_are_refused_as_openai() {
        let scrubber = rules_only();
        for key in LITELLM_KEYS {
            for text in [
                (*key).to_owned(),
                format!("the key {key} here"),
                format!("Authorization: Bearer {key}"),
            ] {
                let mut value = json!({ "t": text });
                let err = scrubber
                    .scrub(&mut value)
                    .err()
                    .unwrap_or_else(|| panic!("a LiteLLM key must refuse the write: {key}"));
                assert_eq!(err.rule, "openai_api_key", "rule for {key}");
            }
        }
    }

    #[test]
    fn prose_that_shares_a_prefix_is_not_a_credential() {
        let scrubber = rules_only();
        for text in [
            "sk-learn",
            "pip install sk-learn-preprocessing-pipeline-v2",
            "sk-learn-preprocessing-pipeline-v2",
            "sk-lf-config",
            "sk-or-v1-docs",
            "sk-None-yet",
            "sk-learn-2024-release-notes-final",
            "sk-learn-py311-wheels-linux",
            "sk-learn_preprocessing_utils_v2",
            "sk-learn-StandardScaler-notes",
            "sk-learn-x86_64-manylinux2014",
            "sk-ant-abcdefghijklm",
            "AKIA",
            "the AKIA prefix marks a long-term key",
            "subtask-x",
            "ghp_short",
            "src/sk-live.rs",
            "task-list, subtask-42, whisk-broom",
            "xoxb-short",
            "eyJhbGciOiJIUzI1NiJ9",
            "npm_token",
        ] {
            let mut value = json!({ "t": text, text: "key" });
            scrubber
                .scrub(&mut value)
                .unwrap_or_else(|err| panic!("prose must not be residue: {err} for {text}"));
        }
    }

    #[test]
    fn a_short_sk_ant_key_is_never_reported_as_openai() {
        let scrubber = rules_only();
        for text in [
            "sk-ant-abcdefghijklmnopqrst",
            "sk-ant-api03-abcdefghijklmnopqrstuvwx",
        ] {
            let mut value = json!({ "t": text });
            let err = scrubber
                .scrub(&mut value)
                .expect_err("an Anthropic key must refuse the write");
            assert_eq!(err.rule, "anthropic_api_key", "rule for {text}");
        }
        let mut short = json!({ "t": "sk-ant-short" });
        scrubber
            .scrub(&mut short)
            .expect("a short sk-ant- word is not a credential");
    }

    #[test]
    fn the_first_rule_in_table_order_is_reported() {
        let mut value =
            json!({ "t": "AKIAIOSFODNN7EXAMPLE then sk-ant-api03-abcdefghijklmnopqrstuvwx" });
        let err = rules_only()
            .scrub(&mut value)
            .expect_err("residue must refuse the write");
        assert_eq!(err.rule, "anthropic_api_key");
    }

    #[test]
    fn a_non_ascii_letter_before_a_key_is_a_token_start() {
        let mut value = json!({ "t": "éAKIA0123456789ABCDEF" });
        let err = rules_only()
            .scrub(&mut value)
            .expect_err("a non-ASCII letter is a token start (fails closed)");
        assert_eq!(err.rule, "aws_access_key_id");
    }

    #[test]
    fn a_credential_mid_sentence_is_caught_and_the_root_pointer_is_empty() {
        let mut value = json!("the key is sk-ant-api03-zzzzzzzzzzzzzzzzzzzz, keep it");
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

    fn resolved(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn from_resolved_masks_values_at_the_floor() {
        let (scrubber, short) = MinimalScrubber::from_resolved(&resolved(&[("K", "abcdef")]));
        assert!(short.is_empty(), "{short:?}");
        let mut value = json!({ "t": "value abcdef here" });
        scrubber.scrub(&mut value).expect("clean");
        assert_eq!(value["t"], json!("value [REDACTED] here"));
    }

    #[test]
    fn from_resolved_lists_the_keys_below_the_floor() {
        let (scrubber, short) =
            MinimalScrubber::from_resolved(&resolved(&[("A", "abcde"), ("B", "longvalue")]));
        assert_eq!(short, vec!["A".to_owned()]);
        let mut value = json!({ "t": "abcde and longvalue" });
        scrubber.scrub(&mut value).expect("clean");
        assert_eq!(value["t"], json!("abcde and [REDACTED]"));
    }

    #[test]
    fn from_resolved_returns_key_names_never_values() {
        let (scrubber, short) = MinimalScrubber::from_resolved(&resolved(&[
            ("SHORT_KEY", "tiny"),
            ("EMPTY_KEY", ""),
            ("LONG_KEY", "a-long-resolved-value"),
        ]));
        assert_eq!(short, vec!["EMPTY_KEY".to_owned(), "SHORT_KEY".to_owned()]);
        let rendered = format!("{short:?} {scrubber:?}");
        for value in ["tiny", "a-long-resolved-value"] {
            assert!(!rendered.contains(value), "leaked: {rendered}");
        }
    }

    #[test]
    fn from_resolved_counts_only_in_debug() {
        let (scrubber, _) = MinimalScrubber::from_resolved(&resolved(&[
            ("TOKEN", "resolved-token-value"),
            ("PIN", "123"),
        ]));
        let rendered = format!("{scrubber:?}");
        assert_eq!(rendered, "MinimalScrubber { secrets: 1 }");
        assert!(!rendered.contains("TOKEN"), "{rendered}");
    }

    #[test]
    fn from_resolved_accepts_an_empty_map_and_still_fails_closed() {
        let (scrubber, short) = MinimalScrubber::from_resolved(&BTreeMap::new());
        assert!(short.is_empty());
        let mut dirty = json!({ "text": "AKIAIOSFODNN7EXAMPLE" });
        let err = scrubber
            .scrub(&mut dirty)
            .expect_err("pattern rules run with an empty resolved map");
        assert_eq!(err.rule, "aws_access_key_id");
    }

    #[test]
    fn the_floor_counts_characters_not_bytes() {
        assert_eq!(MIN_MASKED_LEN, 6);
        let (scrubber, short) =
            MinimalScrubber::from_resolved(&resolved(&[("FIVE", "ééééé"), ("SIX", "éééééé")]));
        assert_eq!(short, vec!["FIVE".to_owned()]);
        assert_eq!(format!("{scrubber:?}"), "MinimalScrubber { secrets: 1 }");
    }

    /// The dual-valid M1 Anthropic fixture: refused as `anthropic_api_key` at any token start.
    const ANT_KEY: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwx";

    #[test]
    fn an_escaped_token_start_is_a_token_start() {
        // The backslash is built at run time: a `\u0022` in a Rust string literal is an
        // escape error (H-21).
        let b = '\\';
        let mut cases = vec![
            (format!("x{b}n{ANT_KEY}"), "anthropic_api_key"),
            (format!("{b}u0022AKIAIOSFODNN7EXAMPLE"), "aws_access_key_id"),
            (
                "%22ghp_abcdefghijklmnopqrstuvwxyz0123456789".to_owned(),
                "github_token",
            ),
        ];
        for escape in ['r', 't', 'b', 'f', '/', '\\', '"'] {
            cases.push((format!("x{b}{escape}{ANT_KEY}"), "anthropic_api_key"));
        }
        let scrubber = rules_only();
        for (text, rule) in cases {
            let mut value = json!({ "t": text });
            let err = scrubber
                .scrub(&mut value)
                .err()
                .unwrap_or_else(|| panic!("an escaped token start must refuse: {text}"));
            assert_eq!(err.rule, rule, "rule for {text}");
            assert_eq!(err.path, "/t", "pointer for {text}");
        }
    }

    #[test]
    fn an_escaped_start_reaches_the_openai_confirmation() {
        let b = '\\';
        // A LiteLLM key after a JSON `\n`: `SK_CANDIDATE` must capture its body.
        let mut value = json!({ "t": format!("x{b}nsk-Ab3_xY9-kLmN0pQrStUvWx") });
        let err = rules_only()
            .scrub(&mut value)
            .expect_err("an escaped LiteLLM key must refuse the write");
        assert_eq!(err.rule, "openai_api_key");
        assert_eq!(err.path, "/t");
    }

    #[test]
    fn an_escape_lookalike_is_not_a_token_start() {
        let b = '\\';
        let scrubber = rules_only();
        for text in [
            "subtask-abcdefghijklmnopqrstuvwxyz".to_owned(),
            "x%2Fsk-learn".to_owned(),
            format!("n{ANT_KEY}"),
            // One hex digit, three hex digits, no hex digits.
            format!("%2{ANT_KEY}"),
            format!("{b}u002{ANT_KEY}"),
            format!("%ZZ{ANT_KEY}"),
            "src/sk-live.rs".to_owned(),
        ] {
            let mut value = json!({ "t": text });
            scrubber
                .scrub(&mut value)
                .unwrap_or_else(|err| panic!("{text:?} must stay clean, got {err}"));
        }
    }

    #[test]
    fn from_resolved_masks_a_value_and_its_newline_free_form() {
        for raw in ["abcdefgh\n", "abcdefgh\r\n"] {
            let (scrubber, short) = MinimalScrubber::from_resolved(&resolved(&[("K", raw)]));
            assert!(short.is_empty(), "{short:?}");
            assert_eq!(format!("{scrubber:?}"), "MinimalScrubber { secrets: 2 }");
            for text in [format!("x {raw} y"), "x abcdefgh y".to_owned()] {
                let mut value = json!({ "t": text });
                scrubber.scrub(&mut value).expect("clean");
                assert_eq!(value["t"], json!("x [REDACTED] y"), "for {raw:?}");
            }
        }
    }

    #[test]
    fn a_newline_free_form_below_the_floor_is_not_masked() {
        let (scrubber, short) = MinimalScrubber::from_resolved(&resolved(&[("K", "abcde\n")]));
        assert!(
            short.is_empty(),
            "the full value is at the floor: {short:?}"
        );
        assert_eq!(format!("{scrubber:?}"), "MinimalScrubber { secrets: 1 }");
        let mut value = json!({ "a": "x abcde\n y", "b": "x abcde y" });
        scrubber.scrub(&mut value).expect("clean");
        assert_eq!(value["a"], json!("x [REDACTED] y"));
        assert_eq!(value["b"], json!("x abcde y"));
    }

    #[test]
    fn hold_back_is_zero_for_a_scrubber_that_does_not_say() {
        #[derive(Debug)]
        struct Plain;
        impl Scrubber for Plain {
            fn scrub(&self, _: &mut Value) -> Result<(), Unmasked> {
                Ok(())
            }
        }
        assert_eq!(Plain.hold_back(), 0);
        let boxed: Box<dyn Scrubber> = Box::new(Plain);
        assert_eq!(boxed.hold_back(), 0);
    }

    #[test]
    fn hold_back_without_secrets_is_the_pattern_bound() {
        assert_eq!(rules_only().hold_back(), PATTERN_HOLD_BACK - 1);
        let (empty, _) = MinimalScrubber::from_resolved(&BTreeMap::new());
        assert_eq!(empty.hold_back(), PATTERN_HOLD_BACK - 1);
    }

    #[test]
    fn hold_back_covers_the_longest_secret() {
        let long = MinimalScrubber::new(["x".repeat(120), "y".repeat(10)]);
        assert_eq!(long.hold_back(), 119);
        let short = MinimalScrubber::new(["y".repeat(10)]);
        assert_eq!(short.hold_back(), PATTERN_HOLD_BACK - 1);
        // Bytes, not characters: 60 two-byte characters are 120 bytes.
        let wide = MinimalScrubber::new(["é".repeat(60)]);
        assert_eq!(wide.hold_back(), 119);
        let as_dyn: &dyn Scrubber = &wide;
        assert_eq!(as_dyn.hold_back(), 119);
    }

    #[test]
    fn pattern_hold_back_covers_every_rule_minimum_and_the_longest_token_start() {
        // `\\u[0-9A-Fa-f]{4}` in `TOKEN_START`: a JSON `\uXXXX` escape, 6 bytes.
        const LONGEST_TOKEN_START: usize = 6;
        for (rule, short) in one_short() {
            // One more byte reaches the rule's minimum match.
            assert!(
                short.len() + 1 + LONGEST_TOKEN_START <= PATTERN_HOLD_BACK,
                "{rule}: a {}-byte minimum after a {LONGEST_TOKEN_START}-byte token start \
                 exceeds PATTERN_HOLD_BACK = {PATTERN_HOLD_BACK}",
                short.len() + 1
            );
        }
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
