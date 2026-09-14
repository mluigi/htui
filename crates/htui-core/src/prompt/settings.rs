//! The resolved prompt budget and where it came from (ANA-5 §4.4 step 1–2, §5.1; plan D101).
//!
//! T64 lands the two value types [`PromptSpec`](crate::prompt::PromptSpec) carries — a budget is an
//! **input** to the assembler, not something it resolves. T65 lands the resolution chain itself
//! (`DEFAULTS`, `resolve_budget`, `resolve_hops`, `resolve_excerpt_caps`) in this same file,
//! because the chain reads `app_setting` and `project.settings` and belongs beside the constants it
//! falls back to.

use serde::Serialize;

/// Which rung of `docs/ANA-2.md:284`'s chain produced the budget, so a surprising number is
/// traceable in one field (§5.1).
///
/// The fourth rung is plan D101's: `app_setting` may lack a key on a database migrated before
/// `0002_agent_probe.sql`, and "the compiled-in table answered" is a different fact from "the row
/// said so".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetSource {
    /// `ResolvedPhase.token_budget`.
    Phase,
    /// `project.settings.token_budget`.
    Project,
    /// `app_setting.token_budget`.
    AppSetting,
    /// The compiled-in §5.3 default, because no rung above it carried a usable value.
    AppSettingDefault,
}

/// A resolved token budget: the number, its provenance, and the reserve held back for the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The resolved `token_budget`.
    pub tokens: i64,
    /// Which rung produced [`tokens`](Self::tokens).
    pub source: BudgetSource,
    /// `app_setting.prompt_reserve_fraction` in basis points: `1_000` is 0.10.
    ///
    /// Basis points rather than an `f64` so [`Budget`] is `Eq` and the target is exact integer
    /// arithmetic on every box. A budget that compared with float tolerance would make the trim,
    /// and therefore the prompt bytes, a function of rounding.
    pub reserve_bp: u32,
}

impl Budget {
    /// `floor(tokens × (1 − reserve))`, in integer arithmetic (§4.4 step 2).
    ///
    /// The reserve is the space the response and any tool schemas need. Continue's config-load
    /// warning is what happens without one: "This leaves only -24576 tokens for input context and
    /// will likely result in your inputs being truncated"
    /// (<https://github.com/continuedev/continue/issues/5166>).
    #[must_use]
    pub const fn target(&self) -> i64 {
        let reserve = self.reserve_bp as i64;
        let kept = if reserve >= 10_000 {
            0
        } else {
            10_000 - reserve
        };
        self.tokens.saturating_mul(kept) / 10_000
    }

    /// The reserve as the fraction `trim_record.reserve` records (§5.1).
    #[must_use]
    pub fn reserve(&self) -> f64 {
        f64::from(self.reserve_bp) / 10_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(tokens: i64, reserve_bp: u32) -> Budget {
        Budget {
            tokens,
            source: BudgetSource::AppSettingDefault,
            reserve_bp,
        }
    }

    #[test]
    fn the_target_is_integer_arithmetic_and_floors() {
        // §5.1's worked example: 120 000 at a 10% reserve is 108 000.
        assert_eq!(budget(120_000, 1_000).target(), 108_000);
        assert!((budget(120_000, 1_000).reserve() - 0.10).abs() < f64::EPSILON);
        // B.3's oversize fixture: 44 000 at 10% is 39 600.
        assert_eq!(budget(44_000, 1_000).target(), 39_600);
        // Floors rather than rounds: 1 001 × 0.9 is 900.9.
        assert_eq!(budget(1_001, 1_000).target(), 900);
        // The degenerate rungs still produce a number rather than a panic.
        assert_eq!(budget(120_000, 0).target(), 120_000);
        assert_eq!(budget(120_000, 10_000).target(), 0);
        assert_eq!(budget(0, 1_000).target(), 0);
    }

    #[test]
    fn the_source_serialises_as_the_record_spells_it() {
        for (source, text) in [
            (BudgetSource::Phase, "phase"),
            (BudgetSource::Project, "project"),
            (BudgetSource::AppSetting, "app_setting"),
            (BudgetSource::AppSettingDefault, "app_setting_default"),
        ] {
            assert_eq!(
                serde_json::to_value(source).expect("a string"),
                serde_json::Value::String(text.to_owned())
            );
        }
    }
}
