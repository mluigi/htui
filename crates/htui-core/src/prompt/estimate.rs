//! The `chars-v2` token estimator (ANA-5 §4.4 `:920-950`, amended by plan D108).
//!
//! Every trim decision is measured with this, and nothing else. It is a pure function of the bytes
//! — same string, same number, on any box — because ANA-5 invariant 2 makes the assembled prompt
//! reproducible and a budget guard that consulted a tokenizer service, a clock or a cache would end
//! that. It is a guard rail and never a billing statement; `run_step.usage` is where real token
//! counts live.
//!
//! The constants are **measured**, not read off a published range: see
//! [`TokenEstimator::DEFAULT`]. The id travels with them into `trim_record.estimator`, so a stored
//! record says which arithmetic produced it — which is why a constants change is a new id (D108)
//! rather than new numbers under the old one.

/// A characters-per-token heuristic with a stable id recorded in `trim_record.estimator`.
///
/// Both rates are ×10 so the constants are integers: `prose_cpt: 25` is 2.5 characters per token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEstimator {
    /// The stable id. One id never carries two arithmetics.
    pub id: &'static str,
    /// Characters per token for prose, ×10.
    pub prose_cpt: u16,
    /// Characters per token for code, ×10.
    pub code_cpt: u16,
}

impl TokenEstimator {
    /// `chars-v2`: 2.5 prose / 2.4 code — **measured** (plan D108) on this box against `claude`
    /// 2.1.267 (`claude-opus-5[1m]`), by summing `input_tokens + cache_creation_input_tokens +
    /// cache_read_input_tokens` on the `result` event and differencing against a minimal-prompt
    /// baseline: 40 000 characters of prose → 15 958 tokens (2.507), 20 000 → 8 018 (2.494, linear
    /// to 0.5%), 40 000 characters of fenced code → 16 393 (2.440).
    ///
    /// ANA-5 §4.4's 3.5 / 3.0 came from a published range and was 28.4% off in the overflow
    /// direction, outside D99's ±25% threshold: a prompt the assembler believed was 108 000 tokens
    /// would really be 151 000, against a 10% reserve sized for a rounding error. Code moved with
    /// prose even though 18.7% is inside the threshold, because one estimator with one row measured
    /// and one row guessed is worse than either.
    ///
    /// This is also the default for an unknown model, as the more conservative of the two rows:
    /// over-estimating trims early rather than overflowing.
    pub const DEFAULT: Self = Self {
        id: "chars-v2",
        prose_cpt: 25,
        code_cpt: 24,
    };

    /// `chars-v1-gpt`: 4.0 / 3.3 for the GPT and Gemini families — **unverified**, and kept at
    /// ANA-5's published figures. The registry row that would serve those families emits no
    /// `usage_update` (MOD-2 milestone 7), so there is nothing to difference. Named apart from
    /// [`TokenEstimator::DEFAULT`] so a stored `trim_record` says which arithmetic produced it
    /// (D108).
    pub const WIDE: Self = Self {
        id: "chars-v1-gpt",
        prose_cpt: 40,
        code_cpt: 33,
    };

    /// Keyed on a **model id**, never on an agent's name (`R-AGT-5`): `gpt-*`, `o<digit>*` and
    /// `gemini-*` take [`TokenEstimator::WIDE`]; everything else, `None` included, takes
    /// [`TokenEstimator::DEFAULT`].
    #[must_use]
    pub fn for_model(model: Option<&str>) -> Self {
        let Some(model) = model else {
            return Self::DEFAULT;
        };
        let id = model.to_ascii_lowercase();
        let bytes = id.as_bytes();
        // `o3`, `o1-preview` — but not `opus-4`: the second character must be a digit.
        let openai_o_series =
            bytes.first() == Some(&b'o') && bytes.get(1).is_some_and(u8::is_ascii_digit);
        if id.starts_with("gpt-") || id.starts_with("gemini-") || openai_o_series {
            Self::WIDE
        } else {
            Self::DEFAULT
        }
    }

    /// Splits `s` into prose and code spans and sums `ceil(chars × 10 / cpt)` per span.
    ///
    /// **Characters, not bytes**: a multi-byte character is one unit, not two or four. Counting
    /// bytes would double the estimate of accented prose and quadruple it for CJK, and would do so
    /// silently, because every internal trim assertion is satisfied by a wrong constant.
    ///
    /// Two toggles, each a whole line:
    ///
    /// * a line whose trimmed start is three or more backticks flips prose↔code — a markdown
    ///   fence. The fence line itself counts as code, so a block and its delimiters are one span;
    /// * a line starting with `<file ` opens code until the `</file>` line. The excerpt render is
    ///   deliberately not fenced (ANA-5 §4.5 `:1115-1117`) — the `N | ` line-number prefix already
    ///   keeps a content fence from being read as a delimiter — so without this toggle every
    ///   excerpt would be counted at the prose rate.
    ///
    /// The toggles are exact and stateless across calls, so the estimate is a pure function of the
    /// bytes. `estimate("")` is `0`.
    #[must_use]
    pub fn estimate(self, s: &str) -> i64 {
        let mut total = 0i64;
        let mut span_chars = 0i64;
        let mut span_is_code = false;
        let mut fenced = false;
        let mut in_file = false;

        for line in s.split_inclusive('\n') {
            let line_is_code = if in_file {
                if line.starts_with("</file>") {
                    in_file = false;
                }
                true
            } else if line.starts_with("<file ") {
                in_file = true;
                true
            } else if line.trim_start().starts_with("```") {
                fenced = !fenced;
                true
            } else {
                fenced
            };

            if line_is_code != span_is_code && span_chars > 0 {
                total += self.span_tokens(span_chars, span_is_code);
                span_chars = 0;
            }
            span_is_code = line_is_code;
            span_chars += i64::try_from(line.chars().count()).unwrap_or(i64::MAX);
        }
        if span_chars > 0 {
            total += self.span_tokens(span_chars, span_is_code);
        }
        total
    }

    /// `ceil(chars × 10 / cpt)` for one span, in integer arithmetic.
    fn span_tokens(self, chars: i64, code: bool) -> i64 {
        let cpt = i64::from(if code { self.code_cpt } else { self.prose_cpt });
        (chars * 10 + cpt - 1) / cpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_are_the_measured_ones() {
        // D108, measured 2026-09-11 on this box against `claude` 2.1.267 (`claude-opus-5[1m]`) by
        // differencing `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` on
        // `result` against a minimal-prompt baseline: 40 000 chars of prose → 15 958 tokens
        // (2.507), 20 000 → 8 018 (2.494, linear to 0.5%), 40 000 chars of fenced code → 16 393
        // (2.440). These are the *measurement*, not a published range: ANA-5 §4.4's 3.5 / 3.0 was
        // 28.4% off in the overflow direction. A change here has to argue with those numbers.
        assert_eq!(
            TokenEstimator::DEFAULT,
            TokenEstimator {
                id: "chars-v2",
                prose_cpt: 25,
                code_cpt: 24,
            }
        );
        // Unverified, and named apart so `trim_record.estimator` says which arithmetic ran:
        // `agy_acp_server` emits no `usage_update`, so there is nothing to difference (D108).
        assert_eq!(
            TokenEstimator::WIDE,
            TokenEstimator {
                id: "chars-v1-gpt",
                prose_cpt: 40,
                code_cpt: 33,
            }
        );
        assert_ne!(
            TokenEstimator::DEFAULT.id,
            TokenEstimator::WIDE.id,
            "one id may never carry two arithmetics"
        );
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(TokenEstimator::DEFAULT.estimate(""), 0);
        assert_eq!(TokenEstimator::WIDE.estimate(""), 0);
    }

    #[test]
    fn chars_not_bytes() {
        let s = "é".repeat(100);
        assert_eq!(s.len(), 200, "the trap: 200 bytes");
        assert_eq!(s.chars().count(), 100, "but 100 characters");
        // ceil(100 × 10 / 25) = 40. Counting bytes would give ceil(200 × 10 / 25) = 80.
        assert_eq!(TokenEstimator::DEFAULT.estimate(&s), 40);
        assert_ne!(TokenEstimator::DEFAULT.estimate(&s), 80);
    }

    #[test]
    fn fenced_spans_use_the_code_rate() {
        let body = "let x = 1;\n".repeat(20);
        // 220 prose characters: ceil(2200 / 25) = 88.
        assert_eq!(TokenEstimator::DEFAULT.estimate(&body), 88);
        // The same lines fenced: 8 + 220 + 4 = 232 characters, all code, ceil(2320 / 24) = 97.
        let fenced = format!("```rust\n{body}```\n");
        assert_eq!(fenced.chars().count(), 232);
        assert_eq!(TokenEstimator::DEFAULT.estimate(&fenced), 97);
    }

    #[test]
    fn file_blocks_use_the_code_rate() {
        // The excerpt render is not fenced (ANA-5 §4.5 `:1115-1117`), so `<file ` opens code and
        // `</file>` closes it, or every excerpt would be counted at the prose rate.
        let body = concat!(
            "<file path=\"htui:a.rs\" lines=\"1-2\" reason=\"mentioned\">\n",
            "1 | fn main() {}\n",
            "2 | // end\n",
            "</file>\n",
        );
        let chars = i64::try_from(body.chars().count()).expect("small");
        let as_code = (chars * 10 + 23) / 24;
        let as_prose = (chars * 10 + 24) / 25;
        assert_ne!(as_code, as_prose, "the two rates must differ at this size");
        assert_eq!(TokenEstimator::DEFAULT.estimate(body), as_code);
        // Without the wrapper the same middle lines are prose.
        let bare = "1 | fn main() {}\n2 | // end\n";
        let bare_chars = i64::try_from(bare.chars().count()).expect("small");
        assert_eq!(
            TokenEstimator::DEFAULT.estimate(bare),
            (bare_chars * 10 + 24) / 25
        );
    }

    #[test]
    fn ceil_per_span_not_per_total() {
        // prose "ab\n" → ceil(30/25) = 2; code "```\n```\n" → ceil(80/24) = 4; prose "cd\n" → 2.
        // Summing the two prose runs first would give ceil(60/25) = 3 and a total of 7.
        let body = "ab\n```\n```\ncd\n";
        assert_eq!(TokenEstimator::DEFAULT.estimate(body), 8);
        assert_ne!(TokenEstimator::DEFAULT.estimate(body), 7);
    }

    #[test]
    fn for_model_keys_on_the_id_only() {
        for id in ["gpt-4o", "gpt-5.2", "o3", "o1-preview", "gemini-3-pro"] {
            assert_eq!(
                TokenEstimator::for_model(Some(id)),
                TokenEstimator::WIDE,
                "`{id}` is a GPT/Gemini-family model id"
            );
        }
        for id in [
            "claude-opus-5",
            "claude-sonnet-4-5",
            "opus-4",
            "codex",
            "gemini",
        ] {
            assert_eq!(
                TokenEstimator::for_model(Some(id)),
                TokenEstimator::DEFAULT,
                "`{id}` is not a GPT/Gemini-family model id"
            );
        }
        // `R-AGT-5`: an agent's *name* never keys the estimator, and an unknown model takes the
        // more conservative row.
        assert_eq!(TokenEstimator::for_model(None), TokenEstimator::DEFAULT);
        assert_eq!(TokenEstimator::for_model(Some("")), TokenEstimator::DEFAULT);
    }
}
