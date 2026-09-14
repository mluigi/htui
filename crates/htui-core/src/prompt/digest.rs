//! The canonical form of an assembled prompt and its `sha256` (ANA-5 §4.7 `:1354-1455`).
//!
//! `R-ORCH-11` records a prompt digest per step and `docs/ANA-4.md:338-340` fixes what is digested:
//! "the assembled prompt text once ... **ANA-5 supplies the text**". ANA-5 chose the smallest thing
//! that answers the audit question — the text alone, canonicalised, **as sent** — over `text` plus
//! `sections[]`, which would have made the digest a function of the estimator version, and over
//! canonicalising for the hash while sending the original bytes, which would make the digest
//! describe a string nobody was given.
//!
//! This module owns steps 5–7 of §4.7's pipeline and nothing else. Steps 1–4 (render, scrub, trim,
//! substitute) are `assemble()`'s in [`crate::prompt`], and step 2's scrub is the ANA-7 scrubber's;
//! putting the per-section scrub here, as the plan's file table did, would put the pipeline in two
//! modules (blueprint E-11 iv).
//!
//! **The order is normalise-then-collapse, not §4.7's collapse-then-normalise** (blueprint P-2).
//! On CRLF input `\r\n\r\n\r\n` holds no run of LFs at all, so collapsing first leaves three line
//! endings where two are wanted, and ANA-5 §12 criteria 4 and 6 fail together. Normalising first is
//! the same bytes for LF input and the correct bytes for CRLF input, which is the whole point of a
//! rule `R-NF-1` put Windows first for.

use sha2::{Digest, Sha256};

/// §4.7 steps 5 and 6, in the order [P-2](self) fixes.
///
/// 1. a leading U+FEFF is dropped;
/// 2. `\r\n` and lone `\r` become `\n`;
/// 3. any run of three or more consecutive LFs becomes exactly two — this is what lets an absent
///    section take its surrounding blank lines with it, deterministically, so a body may place
///    every placeholder unconditionally;
/// 4. trailing whitespace at the end of the string is trimmed and exactly one `\n` is appended.
///
/// Step 4 is over the whole string, never per line: §4.2 rule 2 forbids per-line stripping because
/// it would change meaning in whitespace-sensitive content and make the digest a function of a
/// cosmetic rule. `canonical("")` is therefore `"\n"` — the appended LF is unconditional.
#[must_use]
pub fn canonical(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = String::with_capacity(text.len() + 1);
    let mut newline_run = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let c = if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            '\n'
        } else {
            c
        };
        if c == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                out.push('\n');
            }
        } else {
            newline_run = 0;
            out.push(c);
        }
    }
    let trimmed = out.trim_end();
    out.truncate(trimmed.len());
    out.push('\n');
    out
}

/// §4.7 step 7: `sha256` over the UTF-8 bytes, lowercase hex, all 64 characters.
///
/// The same `sha2` call `htui_agent::record::Recorder::record_prompt` makes over the scrubbed
/// payload text, so the recorder's recomputation is an identity rather than a second opinion. That
/// is what makes ANA-5 criterion 11 checkable at all: the text sent, the text digested and the text
/// stored are one string.
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_collapses_three_lfs_to_two() {
        assert_eq!(canonical("a\n\n\n\n\nb"), "a\n\nb\n");
        assert_eq!(canonical("a\n\nb"), "a\n\nb\n", "two are already canonical");
        assert_eq!(canonical("a\nb"), "a\nb\n", "one is left alone");
        // The absent-section case: a body's blank lines around an empty slot collapse to one gap.
        assert_eq!(canonical("head\n\n\n\ntail"), "head\n\ntail\n");
    }

    #[test]
    fn canonical_normalises_crlf_before_collapsing() {
        // P-2: collapse-then-normalise leaves three line endings here, because CRLF input holds no
        // run of LFs to collapse.
        assert_eq!(canonical("a\r\n\r\n\r\nb"), "a\n\nb\n");
        assert_eq!(
            canonical("a\r\rb"),
            "a\n\nb\n",
            "lone CR is a line ending too"
        );
        assert_eq!(canonical("a\r\n\r\rb"), "a\n\nb\n", "and the two mix");
        assert!(!canonical("a\r\nb").contains('\r'), "no CR survives");
        // Criterion 6: the same content in either encoding is one string.
        assert_eq!(canonical("x\r\ny\r\n\r\n\r\nz"), canonical("x\ny\n\n\nz"));
    }

    #[test]
    fn canonical_strips_a_bom_and_ends_with_one_lf() {
        assert_eq!(canonical("\u{feff}a"), "a\n");
        assert_eq!(
            canonical("a\u{feff}b"),
            "a\u{feff}b\n",
            "only a *leading* BOM is a byte-order mark"
        );
        assert_eq!(
            canonical("a\n\n\n"),
            "a\n",
            "trailing runs go with the trim"
        );
        assert_eq!(canonical("a   \t\n  "), "a\n");
        assert_eq!(canonical("a"), "a\n");
        assert_eq!(canonical(""), "\n", "the appended LF is unconditional");
        // Idempotent: canonicalising a canonical string changes nothing.
        let once = canonical("a\r\n\r\n\r\nb   \n\n");
        assert_eq!(canonical(&once), once);
    }

    #[test]
    fn sha256_matches_the_recorder_call() {
        // The empty-string vector, so a re-implementation elsewhere is caught by a known constant.
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let digest = sha256_hex("htui");
        assert_eq!(digest.len(), 64, "all 64 characters are stored");
        assert!(
            digest
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
            "lowercase hex"
        );
        // `format!("{:x}", Sha256::digest(..))` is exactly `record.rs`'s call over the same bytes.
        assert_eq!(
            digest,
            format!("{:x}", Sha256::digest("htui".as_bytes())),
            "the recorder's recomputation must be an identity"
        );
    }
}
