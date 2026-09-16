//! The resolved prompt budget and where it came from (ANA-5 §4.4 step 1–2, §5.1; plan D101).
//!
//! T64 landed the two value types [`PromptSpec`](crate::prompt::PromptSpec) carries — a budget is
//! an **input** to the assembler, not something it resolves. T65 lands the resolution chain itself,
//! here rather than in `assemble()`, because [`crate::prompt::assemble`] is pure by contract
//! (ANA-5 invariant 2) and a settings read inside it would end that: the caller reads the rungs and
//! hands down a [`Budget`].
//!
//! [`DEFAULTS`] is plan D101's compiled-in table and is not a duplicate for its own sake.
//! `app_setting` has **no cache mirror** (`cache_migrations/0001_mirror.sql` declares no such
//! table) and **no generic reader** (`connect.rs:292-302` reads two hard-coded keys), so a box that
//! cannot reach its Postgres has no other way to learn `token_budget`; and `SEEDED_SETTINGS` is
//! typed `[(&str, i32); 2]`, which cannot express `prompt_reserve_fraction = 0.10` at all. The two
//! are pinned equal by `tests::the_defaults_are_migration_0002s_ten_rows_verbatim`, which reads the
//! migration file itself and so needs no database.
//!
//! Every rung follows one rule, `connect.rs:299-302`'s: a key that is absent, `null`, non-numeric
//! or non-positive is **not a value** and falls through to the next rung. That is why
//! [`BudgetSource`] is recorded — "the compiled-in table answered" and "the row said so" are
//! different facts, and a surprising number is then traceable in one field rather than by
//! re-deriving the chain.

use core::time::Duration;
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::prompt::excerpt::ExcerptCaps;

/// ANA-5 §5.3's ten reserved `app_setting` keys, with the defaults migration `0002` seeds
/// (`0002_agent_probe.sql:68-79`).
///
/// The reserve is held in **basis points** rather than as an `f64` so the whole table is `Eq` and
/// the target is exact integer arithmetic on every box (see [`Budget::target`]).
pub const DEFAULTS: Defaults = Defaults {
    token_budget: 120_000,
    prompt_reserve_fraction_bp: 1_000,
    prompt_upstream_hops: 2,
    max_skill_tokens: 20_000,
    excerpt_max_files: 12,
    excerpt_file_line_cap: 400,
    excerpt_head_lines: 200,
    excerpt_max_file_bytes: 524_288,
    excerpt_max_scan_files: 20_000,
    excerpt_provider_deadline_ms: 1_500,
};

/// §4.3's permitted upstream hop range; a stored value outside it is clamped and noted.
const HOPS_RANGE: (u8, u8) = (1, 2);
/// The largest reserve the resolver will honour: half the budget. A row claiming more would leave
/// the prompt less space than the response, which is a typo rather than a policy.
const MAX_RESERVE_BP: u32 = 5_000;

/// The compiled-in `app_setting` table of plan D101 (ANA-5 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// `token_budget`: the last rung of `docs/ANA-2.md:284`'s chain.
    pub token_budget: i64,
    /// `prompt_reserve_fraction`, ×10 000: `1_000` is the seeded `0.10`.
    pub prompt_reserve_fraction_bp: u32,
    /// `prompt_upstream_hops`, clamped to `1..=2`.
    pub prompt_upstream_hops: u8,
    /// `max_skill_tokens`: the aggregate skills cap that refuses rather than degrades.
    pub max_skill_tokens: i64,
    /// `excerpt_max_files`.
    pub excerpt_max_files: u32,
    /// `excerpt_file_line_cap`.
    pub excerpt_file_line_cap: u32,
    /// `excerpt_head_lines`.
    pub excerpt_head_lines: u32,
    /// `excerpt_max_file_bytes`.
    pub excerpt_max_file_bytes: u64,
    /// `excerpt_max_scan_files`.
    pub excerpt_max_scan_files: u32,
    /// `excerpt_provider_deadline_ms`.
    pub excerpt_provider_deadline_ms: u64,
}

impl Defaults {
    /// The ten rows as `(key, value)` in **key byte order**, which is the order migration `0002`'s
    /// `INSERT` is compared against and the order a seeder must write them in.
    ///
    /// Iterated out of [`SettingKey::ALL`] rather than spelled a second time (MOD-15 plan D7): the
    /// key list has one home, so a key added to the registry cannot be forgotten here and drift
    /// past `the_defaults_are_migration_0002s_ten_rows_verbatim`.
    #[must_use]
    pub fn as_rows(&self) -> Vec<(&'static str, Value)> {
        SettingKey::ALL
            .into_iter()
            .map(|key| (key.key(), self.value_of(key)))
            .collect()
    }

    /// One key's compiled-in value, as the JSON migration `0002` seeds.
    ///
    /// [`SettingKey::PromptReserveFraction`] is emitted through the same `f64` expression
    /// [`as_rows`](Self::as_rows) has always used, so the byte-for-byte `0002` pin is unaffected by
    /// the registry landing beside it.
    #[must_use]
    pub fn value_of(&self, key: SettingKey) -> Value {
        match key {
            SettingKey::ExcerptFileLineCap => Value::from(self.excerpt_file_line_cap),
            SettingKey::ExcerptHeadLines => Value::from(self.excerpt_head_lines),
            SettingKey::ExcerptMaxFileBytes => Value::from(self.excerpt_max_file_bytes),
            SettingKey::ExcerptMaxFiles => Value::from(self.excerpt_max_files),
            SettingKey::ExcerptMaxScanFiles => Value::from(self.excerpt_max_scan_files),
            SettingKey::ExcerptProviderDeadlineMs => Value::from(self.excerpt_provider_deadline_ms),
            SettingKey::MaxSkillTokens => Value::from(self.max_skill_tokens),
            SettingKey::PromptReserveFraction => {
                Value::from(f64::from(self.prompt_reserve_fraction_bp) / 10_000.0)
            }
            SettingKey::UpstreamHops => Value::from(self.prompt_upstream_hops),
            SettingKey::TokenBudget => Value::from(self.token_budget),
        }
    }

    /// One key's compiled-in value in the unit [`SettingSpec::min`] and `max` are written in, which
    /// for [`SettingKind::Fraction`] is basis points.
    ///
    /// [`validate`]'s `not_above` rule needs a number for the peer key even when no rung stores
    /// one, and "no row" means the table below answers — so the rule compares against the same
    /// value the reader would have resolved rather than against nothing.
    fn integer(&self, key: SettingKey) -> i64 {
        match key {
            SettingKey::ExcerptFileLineCap => i64::from(self.excerpt_file_line_cap),
            SettingKey::ExcerptHeadLines => i64::from(self.excerpt_head_lines),
            SettingKey::ExcerptMaxFileBytes => {
                i64::try_from(self.excerpt_max_file_bytes).unwrap_or(i64::MAX)
            }
            SettingKey::ExcerptMaxFiles => i64::from(self.excerpt_max_files),
            SettingKey::ExcerptMaxScanFiles => i64::from(self.excerpt_max_scan_files),
            SettingKey::ExcerptProviderDeadlineMs => {
                i64::try_from(self.excerpt_provider_deadline_ms).unwrap_or(i64::MAX)
            }
            SettingKey::MaxSkillTokens => self.max_skill_tokens,
            SettingKey::PromptReserveFraction => i64::from(self.prompt_reserve_fraction_bp),
            SettingKey::UpstreamHops => i64::from(self.prompt_upstream_hops),
            SettingKey::TokenBudget => self.token_budget,
        }
    }
}

/// The ten `app_setting` keys of ANA-5 §5.3, declared in **key byte order**: the discriminant
/// indexes [`SPECS`] and [`Self::ALL`] reproduces [`Defaults::as_rows`]'s order (MOD-15 plan D7).
///
/// A typed key rather than a `&str`, because the writer this exists for is the one place a typo
/// used to be invisible: `positive_i64` treats an unknown key exactly as it treats a malformed
/// one, so a misspelled write would show one number in the editor and use another in the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingKey {
    /// `excerpt_file_line_cap`.
    ExcerptFileLineCap = 0,
    /// `excerpt_head_lines`.
    ExcerptHeadLines = 1,
    /// `excerpt_max_file_bytes`.
    ExcerptMaxFileBytes = 2,
    /// `excerpt_max_files`.
    ExcerptMaxFiles = 3,
    /// `excerpt_max_scan_files`.
    ExcerptMaxScanFiles = 4,
    /// `excerpt_provider_deadline_ms`.
    ExcerptProviderDeadlineMs = 5,
    /// `max_skill_tokens`.
    MaxSkillTokens = 6,
    /// `prompt_reserve_fraction`, the one [`SettingKind::Fraction`] of the ten.
    PromptReserveFraction = 7,
    /// `prompt_upstream_hops` on the App rung, `upstream_hops` on a project (flag A).
    UpstreamHops = 8,
    /// `token_budget`, the only key all three rungs accept.
    TokenBudget = 9,
}

impl SettingKey {
    /// Every key, in the order [`Defaults::as_rows`] emits and migration `0002` inserts.
    pub const ALL: [Self; 10] = [
        Self::ExcerptFileLineCap,
        Self::ExcerptHeadLines,
        Self::ExcerptMaxFileBytes,
        Self::ExcerptMaxFiles,
        Self::ExcerptMaxScanFiles,
        Self::ExcerptProviderDeadlineMs,
        Self::MaxSkillTokens,
        Self::PromptReserveFraction,
        Self::UpstreamHops,
        Self::TokenBudget,
    ];

    /// This key's row of the registry.
    #[must_use]
    pub const fn spec(self) -> &'static SettingSpec {
        &SPECS[self as usize]
    }

    /// The `app_setting.key` column value.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.spec().key
    }

    /// The reverse of [`Self::key`], `None` for a string outside the ten.
    ///
    /// The two MOD-6 cache keys (`cache_refresh_seconds`, `cache_overlap_seconds`) are deliberately
    /// outside: their reader is `connect.rs`'s own `> 0` rule and the `0002` pin asserts exactly
    /// ten rows. Adding one is a variant, when MOD-12 or the connection section asks (plan D7).
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.key() == key)
    }
}

impl core::fmt::Display for SettingKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.key())
    }
}

/// Which rungs accept a key (plan D7): a three-flag bitset, and no `bitflags` dependency for three
/// bits that never grow — `docs/ANA-2.md:284`'s chain has exactly these three writable rungs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rungs(u8);

impl Rungs {
    /// An `app_setting` row.
    pub const APP: Self = Self(1);
    /// One key of `project.settings`.
    pub const PROJECT: Self = Self(2);
    /// `step_graph_phase.token_budget`.
    pub const PHASE: Self = Self(4);

    /// Whether every flag of `other` is set here.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union, spelled as a `const fn` so [`SPECS`] can build one.
    #[must_use]
    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for Rungs {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.or(rhs)
    }
}

impl core::fmt::Display for Rungs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut written = false;
        for (flag, name) in [
            (Self::APP, "app"),
            (Self::PROJECT, "project"),
            (Self::PHASE, "phase"),
        ] {
            if self.contains(flag) {
                if written {
                    f.write_str("|")?;
                }
                f.write_str(name)?;
                written = true;
            }
        }
        if written { Ok(()) } else { f.write_str("no") }
    }
}

/// How a key's JSON is read (PRD D7's `kind`): every key but one is an integer, and
/// `prompt_reserve_fraction` is a float the reader rounds to basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// Read through `positive_i64` or a narrowing of it.
    Integer,
    /// Read through `as_f64` and rounded once, to basis points.
    Fraction,
}

/// One row of the registry (plan D7): the single source `DEFAULTS`, the migration pin, the
/// validator and milestone 5's editor all read.
///
/// `min` and `max` are **the reader's own clamps** in `unit`; for [`SettingKind::Fraction`] they
/// are basis points of the stored float. That is what makes "no value can be written that the read
/// half would clamp" checkable rather than aspirational — `ranges_are_the_readers_clamps` asserts
/// them against the resolver's constants, so a clamp that moves fails a unit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingSpec {
    /// The `app_setting.key` column value.
    pub key: &'static str,
    /// The name under which `project.settings` holds it; `Some` exactly when `rungs` has
    /// [`Rungs::PROJECT`]. Two of the ten spell it differently there (flag A), and writing the App
    /// spelling into a project would store a key `resolve_hops` never looks at.
    pub project_key: Option<&'static str>,
    /// How the JSON is read.
    pub kind: SettingKind,
    /// The smallest value the reader honours, in `unit`.
    pub min: i64,
    /// The largest value the reader honours, in `unit`. Narrowed to `i32::MAX` on
    /// [`Rungs::PHASE`], whose column is `INTEGER` (flag C).
    pub max: i64,
    /// Which rungs accept the key.
    pub rungs: Rungs,
    /// The peer key this one may not exceed, the one cross-key rule the reader applies.
    pub not_above: Option<SettingKey>,
    /// What `min` and `max` count.
    pub unit: &'static str,
    /// One line for the editor milestone 5 builds.
    pub doc: &'static str,
}

/// `u32::MAX` as the `i64` the four `positive_u32` keys cap at.
const U32_MAX: i64 = u32::MAX as i64;

/// The registry, indexed by [`SettingKey`]'s discriminant.
pub const SPECS: [SettingSpec; 10] = [
    SettingSpec {
        key: "excerpt_file_line_cap",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: U32_MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "lines",
        doc: "Lines of one file the excerpt scanner reads.",
    },
    SettingSpec {
        key: "excerpt_head_lines",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: U32_MAX,
        rungs: Rungs::APP,
        not_above: Some(SettingKey::ExcerptFileLineCap),
        unit: "lines",
        doc: "Lines quoted from the top of a file; the reader clamps it to excerpt_file_line_cap.",
    },
    SettingSpec {
        key: "excerpt_max_file_bytes",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: i64::MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "bytes",
        doc: "Largest file the scanner opens; read as an i64, so i64::MAX is the ceiling (F2).",
    },
    SettingSpec {
        key: "excerpt_max_files",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: U32_MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "files",
        doc: "Files quoted per prompt.",
    },
    SettingSpec {
        key: "excerpt_max_scan_files",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: U32_MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "files",
        doc: "Files the scanner visits before it stops.",
    },
    SettingSpec {
        key: "excerpt_provider_deadline_ms",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: i64::MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "ms",
        doc: "Wall clock the excerpt provider gets; read as an i64 (F2).",
    },
    SettingSpec {
        key: "max_skill_tokens",
        project_key: None,
        kind: SettingKind::Integer,
        min: 1,
        max: i64::MAX,
        rungs: Rungs::APP,
        not_above: None,
        unit: "tokens",
        doc: "Tokens the skill section may take before the step is refused.",
    },
    SettingSpec {
        key: "prompt_reserve_fraction",
        project_key: None,
        kind: SettingKind::Fraction,
        min: 0,
        max: 5_000,
        rungs: Rungs::APP,
        not_above: None,
        unit: "bp",
        doc: "Share of the budget held back for the answer, 0.0 to 0.5.",
    },
    SettingSpec {
        key: "prompt_upstream_hops",
        project_key: Some("upstream_hops"),
        kind: SettingKind::Integer,
        min: 1,
        max: 2,
        rungs: Rungs::APP.or(Rungs::PROJECT),
        not_above: None,
        unit: "hops",
        doc: "Upstream items followed when quoting context.",
    },
    SettingSpec {
        key: "token_budget",
        project_key: Some("token_budget"),
        kind: SettingKind::Integer,
        min: 1,
        max: i64::MAX,
        rungs: Rungs::APP.or(Rungs::PROJECT).or(Rungs::PHASE),
        not_above: None,
        unit: "tokens",
        doc: "Prompt budget; the phase column is INTEGER, so that rung caps at i32::MAX.",
    },
];

/// [`validate`]'s first check on its own: `Some` is the sentence to refuse with, `None` is "this
/// rung accepts this key".
///
/// Separate because `clear_setting` and `setting` have the same rule and no value to validate
/// (plan D8), and a store that spelled the refusal itself would be a second sentence for a fact
/// this one already states — the drift the four text helpers in
/// [`store::traits`](crate::store::traits) exist to prevent.
#[must_use]
pub fn rung_refusal(key: SettingKey, at: Rungs) -> Option<String> {
    (!key.spec().rungs.contains(at)).then(|| format!("`{key}` is not accepted on the {at} rung"))
}

/// Refuses what the reader would clamp or ignore (plan D7, D8), in order: rung, kind, range,
/// `not_above`.
///
/// `at` is the single rung being written; `peer` is that same rung's current value of
/// `not_above`'s key, or `None` for "this rung stores none", in which case the compiled-in default
/// stands in — the number [`resolve_excerpt_caps`] would itself have resolved.
///
/// # The `not_above` rule is one-directional, by decision
///
/// It is checked when the **child** is written and never when the **peer** is, so "nothing this
/// accepts is a value the reader would clamp" is true of the value in front of it and of no other.
/// Concretely: `excerpt_head_lines` may not be set above `excerpt_file_line_cap`, but
/// `excerpt_file_line_cap` may be *lowered below* a stored `excerpt_head_lines` and that write is
/// accepted — after which [`resolve_excerpt_caps`]'s `.min(file_line_cap)` clamps the head the
/// caller never touched, and the store's own `excerpt_head_lines` no longer describes what the
/// reader uses.
///
/// Blueprint flag J, accepted by the maintainer rather than overlooked: the plan is authority and
/// this is recorded so the settings editor can show the consequence instead of implying the pair is
/// symmetric. A `not_below` twin would be one more field on [`SettingSpec`] if it is ever wanted.
///
/// The `String` is the whole sentence to show; the store wraps it in
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) unchanged, so the same words
/// reach the editor from `MemStore` and from `PgStore`.
///
/// # Errors
///
/// One of, in the order they are checked:
/// - `` `{key}` is not accepted on the {rung} rung ``
/// - `` `{key}` must be a JSON integer, got {value} `` / `` `{key}` must be a finite JSON number,
///   got {value} ``
/// - `` `{key}` = {n} is outside {min}..={max} {unit} `` — for a fraction, `` `{key}` = {value}
///   rounds to {n} bp, outside {min}..={max} bp ``
/// - `` `{key}` = {n} is above `{other}` = {m}; the reader would clamp it ``
pub fn validate(
    key: SettingKey,
    at: Rungs,
    value: &Value,
    peer: Option<&Value>,
) -> Result<(), String> {
    let spec = key.spec();
    if let Some(refusal) = rung_refusal(key, at) {
        return Err(refusal);
    }

    // The phase rung is one `INTEGER` column, so it narrows the spec's own ceiling rather than
    // letting Postgres answer `22003` to a value validation had already accepted (flag C).
    let ceiling = if at == Rungs::PHASE {
        spec.max.min(i64::from(i32::MAX))
    } else {
        spec.max
    };

    let number = match spec.kind {
        SettingKind::Integer => {
            let Some(number) = value.as_i64() else {
                return Err(format!("`{key}` must be a JSON integer, got {value}"));
            };
            if number < spec.min || number > ceiling {
                return Err(format!(
                    "`{key}` = {number} is outside {}..={ceiling} {}",
                    spec.min, spec.unit
                ));
            }
            number
        }
        SettingKind::Fraction => {
            let Some(fraction) = value.as_f64().filter(|f| f.is_finite()) else {
                return Err(format!("`{key}` must be a finite JSON number, got {value}"));
            };
            // Saturating, as every float-to-integer cast has been since Rust 1.45: a fraction too
            // large to name lands on `i64::MAX` and is refused by the range rather than wrapping
            // into it.
            let number = (fraction * 10_000.0).round() as i64;
            if number < spec.min || number > ceiling {
                return Err(format!(
                    "`{key}` = {value} rounds to {number} bp, outside {}..={ceiling} bp",
                    spec.min
                ));
            }
            number
        }
    };

    if let Some(other) = spec.not_above {
        let limit = peer
            .and_then(Value::as_i64)
            .unwrap_or_else(|| DEFAULTS.integer(other));
        if number > limit {
            return Err(format!(
                "`{key}` = {number} is above `{other}` = {limit}; the reader would clamp it"
            ));
        }
    }
    Ok(())
}

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

impl BudgetSource {
    /// The one spelling: the `budget_source` value `trim_record` serialises.
    ///
    /// See [`TrimStrategy::as_str`](crate::prompt::TrimStrategy::as_str) for why a view gets this
    /// rather than a table of its own.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Project => "project",
            Self::AppSetting => "app_setting",
            Self::AppSettingDefault => "app_setting_default",
        }
    }
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

/// A rung's value when it is a usable positive integer, `None` when it is anything else.
///
/// One rule for every integer key, taken from the only generic-ish reader the tree already has
/// (`connect.rs:299-302`, which "skips any non-positive or non-numeric value"): absent, `null`,
/// a string, a bool, zero and negative all mean "this rung did not answer".
fn positive_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|n| *n > 0)
}

/// One key out of a `project.settings` object, which may be absent or may not be an object at all.
fn project_key<'a>(project: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    project?.get(key)
}

/// `docs/ANA-2.md:284`'s chain with plan D101's fourth rung: phase, project, `app_setting`, table.
///
/// The reserve is resolved independently of the budget and always from `app_setting`: ANA-5 §4.4
/// step 2 gives it no per-phase or per-project rung, and inventing one would make two boxes
/// disagree about how much of the same budget is spendable.
#[must_use]
pub fn resolve_budget(
    phase: Option<i32>,
    project: Option<&Value>,
    app: &BTreeMap<String, Value>,
) -> Budget {
    let reserve_bp = resolve_reserve_bp(app);
    let (tokens, source) = if let Some(tokens) = phase.filter(|n| *n > 0) {
        (i64::from(tokens), BudgetSource::Phase)
    } else if let Some(tokens) = positive_i64(project_key(project, "token_budget")) {
        (tokens, BudgetSource::Project)
    } else if let Some(tokens) = positive_i64(app.get("token_budget")) {
        (tokens, BudgetSource::AppSetting)
    } else {
        (DEFAULTS.token_budget, BudgetSource::AppSettingDefault)
    };
    Budget {
        tokens,
        source,
        reserve_bp,
    }
}

/// `app_setting.prompt_reserve_fraction` as basis points, clamped to `0..=5_000`.
///
/// The one fractional key of §5.3, and the reason §9's seed is SQL rather than an extension of
/// `SEEDED_SETTINGS`. Read as `f64` and rounded once, here, so no other code ever holds the float:
/// a budget compared with float tolerance would make the trim — and therefore the prompt bytes — a
/// function of rounding.
fn resolve_reserve_bp(app: &BTreeMap<String, Value>) -> u32 {
    let Some(fraction) = app
        .get("prompt_reserve_fraction")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite())
    else {
        return DEFAULTS.prompt_reserve_fraction_bp;
    };
    let bp = (fraction * 10_000.0).round();
    if bp <= 0.0 {
        0
    } else if bp >= f64::from(MAX_RESERVE_BP) {
        MAX_RESERVE_BP
    } else {
        // Exact: `bp` is a rounded float in `0 .. 5_000`, so the cast cannot lose a digit.
        bp as u32
    }
}

/// §4.3 step 2: `project.settings.upstream_hops`, then `app_setting.prompt_upstream_hops`, then 2.
///
/// A resolved value outside `1..=2` is **clamped and noted**, never an error: `R-PRM-1` permits one
/// or two hops, and a project that stored seven has a stale row rather than a broken step. The note
/// lands in `trim_record.notes`, which is where ANA-5 §5.1 puts conditions that are not errors.
#[must_use]
pub fn resolve_hops(
    project: Option<&Value>,
    app: &BTreeMap<String, Value>,
    notes: &mut Vec<String>,
) -> u8 {
    let raw = positive_i64(project_key(project, "upstream_hops"))
        .or_else(|| positive_i64(app.get("prompt_upstream_hops")))
        .unwrap_or_else(|| i64::from(DEFAULTS.prompt_upstream_hops));
    let (low, high) = HOPS_RANGE;
    let clamped = raw.clamp(i64::from(low), i64::from(high));
    if clamped != raw {
        notes.push(format!(
            "upstream hops clamped from {raw} to {clamped} (ANA-5 §4.3 allows {low}..={high})"
        ));
    }
    u8::try_from(clamped).unwrap_or(high)
}

/// §4.2's aggregate skills cap: `app_setting.max_skill_tokens`, then the table.
///
/// Resolved by the caller and carried on [`PromptSpec`](crate::prompt::PromptSpec) because
/// exceeding it **refuses the step** ([`AssembleError::SkillsExceedCap`](crate::prompt::AssembleError))
/// rather than dropping a binding — a silently dropped skill would break `R-ID-5`'s
/// identical-behaviour promise.
#[must_use]
pub fn resolve_max_skill_tokens(app: &BTreeMap<String, Value>) -> i64 {
    positive_i64(app.get("max_skill_tokens")).unwrap_or(DEFAULTS.max_skill_tokens)
}

/// §5.3's six `excerpt_*` keys: the four caps the audit records, the walk's scan cap, and the
/// provider deadline (blueprint B.8; T65 deferred this to T66 by name).
///
/// Three returns rather than one struct because the three go to three different places: the caps
/// are recorded verbatim in `trim_record.excerpts.caps`, the scan cap is the walk's and never
/// appears in the record, and the deadline belongs to the provider runner in `htui-agent`. Folding
/// them together would put two fields into the audit that ANA-5 `:1127` does not have.
///
/// Every key resolves independently under the one rung rule (`connect.rs:299-302`): absent, `null`,
/// non-numeric or non-positive falls through to [`DEFAULTS`]. `excerpt_head_lines` is then clamped
/// to `excerpt_file_line_cap` — a head above the cap that made a file window at all is a stored
/// typo, and honouring it would render more lines than the cap permits.
#[must_use]
pub fn resolve_excerpt_caps(app: &BTreeMap<String, Value>) -> (ExcerptCaps, u32, Duration) {
    let file_line_cap =
        positive_u32(app.get("excerpt_file_line_cap")).unwrap_or(DEFAULTS.excerpt_file_line_cap);
    let caps = ExcerptCaps {
        max_files: positive_u32(app.get("excerpt_max_files")).unwrap_or(DEFAULTS.excerpt_max_files),
        file_line_cap,
        head_lines: positive_u32(app.get("excerpt_head_lines"))
            .unwrap_or(DEFAULTS.excerpt_head_lines)
            .min(file_line_cap),
        max_file_bytes: positive_i64(app.get("excerpt_max_file_bytes"))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .unwrap_or(DEFAULTS.excerpt_max_file_bytes),
    };
    let scan =
        positive_u32(app.get("excerpt_max_scan_files")).unwrap_or(DEFAULTS.excerpt_max_scan_files);
    let deadline = positive_i64(app.get("excerpt_provider_deadline_ms"))
        .and_then(|ms| u64::try_from(ms).ok())
        .unwrap_or(DEFAULTS.excerpt_provider_deadline_ms);
    (caps, scan, Duration::from_millis(deadline))
}

/// [`positive_i64`] narrowed, for the four `u32` caps. A row too large for a `u32` is a typo and
/// falls through rather than saturating into a cap nobody wrote.
fn positive_u32(value: Option<&Value>) -> Option<u32> {
    positive_i64(value).and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Migration `0002_agent_probe.sql`, read at compile time so the assertion below needs no
    /// database. The Postgres-side twin is
    /// `htui-store/tests/migrations.rs::the_ten_ana5_defaults_land_with_their_values`.
    const MIGRATION_0002: &str =
        include_str!("../../../htui-store/migrations/0002_agent_probe.sql");

    /// The `('key', 'value'::jsonb)` pairs of `0002`'s `app_setting` INSERT, in file order.
    ///
    /// A five-line parser rather than a regex, because `htui-core` takes no regex dependency and
    /// the shape is fixed by the file this test exists to pin.
    fn migration_rows() -> Vec<(String, Value)> {
        let insert = MIGRATION_0002
            .split_once("INSERT INTO app_setting (key, value) VALUES")
            .expect("0002 seeds app_setting")
            .1;
        let insert = insert
            .split_once("ON CONFLICT")
            .expect("the INSERT is idempotent")
            .0;
        let mut rows = Vec::new();
        for line in insert.lines() {
            let Some(rest) = line.trim().strip_prefix('(') else {
                continue;
            };
            let (key, rest) = rest
                .trim_start_matches('\'')
                .split_once('\'')
                .expect("a quoted key");
            let value = rest
                .split_once('\'')
                .expect("a quoted value")
                .1
                .split_once('\'')
                .expect("a quoted value")
                .0;
            rows.push((
                key.to_owned(),
                serde_json::from_str(value).expect("a JSON literal"),
            ));
        }
        rows
    }

    #[test]
    fn the_defaults_are_migration_0002s_ten_rows_verbatim() {
        // D101: `app_setting` has no cache mirror and no generic reader, so the compiled-in table
        // is the only thing an offline box can fall back to. This is what stops the two drifting.
        let rows = migration_rows();
        assert_eq!(rows.len(), 10, "ANA-5 §5.3 reserves ten keys, got {rows:?}");
        let mut expected = rows.clone();
        expected.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        assert_eq!(
            DEFAULTS.as_rows(),
            expected
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect::<Vec<_>>(),
            "`DEFAULTS` and migration 0002 carry the same ten keys and the same ten values"
        );
    }

    #[test]
    fn as_rows_is_key_byte_order() {
        let keys: Vec<&str> = DEFAULTS.as_rows().into_iter().map(|(key, _)| key).collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(keys, sorted, "byte order, never a map's iteration order");
    }

    /// Every compiled-in default is a value its own spec accepts.
    ///
    /// The registry exists to refuse what the reader would ignore; a table whose own seeded value
    /// the validator rejects would make migration `0002` unwritable through the editor it feeds.
    #[test]
    fn every_default_validates_under_its_own_spec() {
        for key in SettingKey::ALL {
            let value = DEFAULTS.value_of(key);
            assert_eq!(
                validate(key, Rungs::APP, &value, None),
                Ok(()),
                "the seeded default of {key} is a value its own spec accepts"
            );
        }
    }

    /// `min`/`max` are the reader's clamps, asserted against the reader's own constants rather
    /// than against copies of them, so a later edit to one fails here instead of drifting.
    #[test]
    fn ranges_are_the_readers_clamps() {
        let (low, high) = HOPS_RANGE;
        let hops = SettingKey::UpstreamHops.spec();
        assert_eq!(
            (hops.min, hops.max),
            (i64::from(low), i64::from(high)),
            "`resolve_hops` clamps to HOPS_RANGE, so the writer refuses outside it"
        );
        assert_eq!(
            SettingKey::PromptReserveFraction.spec().max,
            i64::from(MAX_RESERVE_BP),
            "`resolve_reserve_bp` clamps at MAX_RESERVE_BP basis points"
        );

        for key in [
            SettingKey::ExcerptFileLineCap,
            SettingKey::ExcerptHeadLines,
            SettingKey::ExcerptMaxFiles,
            SettingKey::ExcerptMaxScanFiles,
        ] {
            assert_eq!(
                key.spec().max,
                U32_MAX,
                "{key} is read through `positive_u32`, which drops anything wider"
            );
        }
        for key in [
            // F2: read as `positive_i64(..).and_then(|v| u64::try_from(v).ok())`, so the ceiling
            // is `i64::MAX` and not `u64::MAX` — above it the reader silently drops the row.
            SettingKey::ExcerptMaxFileBytes,
            SettingKey::ExcerptProviderDeadlineMs,
            SettingKey::MaxSkillTokens,
            SettingKey::TokenBudget,
        ] {
            assert_eq!(key.spec().max, i64::MAX, "{key} is read as an i64");
        }
        for key in SettingKey::ALL {
            assert!(
                key.spec().min >= 0,
                "{key} is read through a `positive_*` helper, so zero is not a value"
            );
        }
        assert_eq!(
            SettingKey::ExcerptHeadLines.spec().not_above,
            Some(SettingKey::ExcerptFileLineCap),
            "`resolve_excerpt_caps` clamps the head to the line cap, so the writer refuses above it"
        );
    }

    /// Flag A: the App key and the `project.settings` key are different strings for two of the
    /// ten, so a spec that accepted the project rung without naming its key would write something
    /// `resolve_hops` never reads.
    #[test]
    fn project_key_is_some_exactly_where_project_is_accepted() {
        for key in SettingKey::ALL {
            let spec = key.spec();
            assert_eq!(
                spec.project_key.is_some(),
                spec.rungs.contains(Rungs::PROJECT),
                "{key} declares a project key exactly when it accepts the project rung"
            );
        }
        assert_eq!(
            SettingKey::UpstreamHops.spec().project_key,
            Some("upstream_hops"),
            "`resolve_hops` reads `upstream_hops` out of the project, not the App spelling"
        );
        assert_eq!(
            SettingKey::UpstreamHops.key(),
            "prompt_upstream_hops",
            "…while the App row is the one migration 0002 seeds"
        );
    }

    /// The discriminant is the index and the keys ascend as bytes, which is what makes
    /// `SettingKey::ALL` reproduce `as_rows`'s order without a second list.
    #[test]
    fn specs_are_indexed_by_discriminant() {
        for key in SettingKey::ALL {
            assert_eq!(
                SPECS[key as usize].key,
                key.key(),
                "{key} indexes its own spec"
            );
            assert_eq!(
                SettingKey::from_key(key.key()),
                Some(key),
                "{key} round-trips through its column value"
            );
        }
        let keys: Vec<&str> = SettingKey::ALL.into_iter().map(SettingKey::key).collect();
        let mut sorted = keys.clone();
        sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        sorted.dedup();
        assert_eq!(
            keys, sorted,
            "ten distinct keys, strictly ascending as bytes"
        );
        assert!(
            SettingKey::from_key("cache_refresh_seconds").is_none(),
            "D7"
        );
    }

    /// The PRD's "stored equals effective": a value the writer accepts comes back out of the
    /// reader unchanged, rather than clamped into a number nobody wrote.
    #[test]
    fn an_accepted_value_survives_its_resolver() {
        for hops in 1_u8..=2 {
            let value = json!(hops);
            assert_eq!(
                validate(SettingKey::UpstreamHops, Rungs::APP, &value, None),
                Ok(())
            );
            let mut notes = Vec::new();
            assert_eq!(
                resolve_hops(None, &app(&[("prompt_upstream_hops", value)]), &mut notes),
                hops,
                "an accepted hop count is the hop count the walk uses"
            );
            assert!(notes.is_empty(), "…and nothing was clamped on the way");
        }

        let fraction = json!(0.25);
        assert_eq!(
            validate(
                SettingKey::PromptReserveFraction,
                Rungs::APP,
                &fraction,
                None
            ),
            Ok(())
        );
        assert_eq!(
            resolve_reserve_bp(&app(&[("prompt_reserve_fraction", fraction)])),
            2_500,
            "0.25 is 2500 basis points, held exactly"
        );

        let head = json!(DEFAULTS.excerpt_file_line_cap);
        let cap = DEFAULTS.value_of(SettingKey::ExcerptFileLineCap);
        assert_eq!(
            validate(SettingKey::ExcerptHeadLines, Rungs::APP, &head, Some(&cap)),
            Ok(()),
            "a head equal to the cap is accepted; `not_above` is not `not_equal`"
        );
        let (caps, _, _) = resolve_excerpt_caps(&app(&[
            ("excerpt_head_lines", head),
            ("excerpt_file_line_cap", cap),
        ]));
        assert_eq!(
            caps.head_lines, DEFAULTS.excerpt_file_line_cap,
            "…and it survives the clamp that refuses one line more"
        );
    }

    /// Flag C: `step_graph_phase.token_budget` is `INTEGER`, so the phase rung narrows the spec's
    /// own ceiling. Without this a write Postgres answers `22003` to would pass validation.
    #[test]
    fn phase_rung_caps_token_budget_at_i32() {
        let too_wide = json!(i64::from(i32::MAX) + 1);
        assert!(
            validate(SettingKey::TokenBudget, Rungs::PHASE, &too_wide, None).is_err(),
            "the phase column cannot hold it"
        );
        assert_eq!(
            validate(SettingKey::TokenBudget, Rungs::APP, &too_wide, None),
            Ok(()),
            "…while `app_setting.value` is JSONB and the reader is an i64"
        );
        assert_eq!(
            validate(
                SettingKey::TokenBudget,
                Rungs::PHASE,
                &json!(i32::MAX),
                None
            ),
            Ok(()),
            "the ceiling itself is accepted"
        );
    }

    /// The rung flag is checked before anything else, so a key on the wrong rung is refused by
    /// name rather than by range.
    #[test]
    fn a_key_is_refused_on_a_rung_that_does_not_accept_it() {
        let refused = validate(SettingKey::ExcerptMaxFiles, Rungs::PROJECT, &json!(3), None)
            .expect_err("`excerpt_max_files` has no project rung");
        assert!(
            refused.contains("excerpt_max_files") && refused.contains("project"),
            "the refusal names the key and the rung, got `{refused}`"
        );
        assert!(
            validate(SettingKey::UpstreamHops, Rungs::PHASE, &json!(1), None).is_err(),
            "`token_budget` is the phase rung's only key"
        );
    }

    /// Every refusal carries the key, the value and the rule, because `positive_i64` fails
    /// silently and a bare "invalid" would be indistinguishable from the silence it replaces.
    #[test]
    fn a_refusal_names_the_key_the_value_and_the_rule() {
        for (label, key, value) in [
            ("a string", SettingKey::TokenBudget, json!("120000")),
            ("a float", SettingKey::TokenBudget, json!(1.5)),
            ("null", SettingKey::TokenBudget, Value::Null),
            ("zero", SettingKey::TokenBudget, json!(0)),
            ("three hops", SettingKey::UpstreamHops, json!(3)),
            (
                "a string fraction",
                SettingKey::PromptReserveFraction,
                json!("0.1"),
            ),
            (
                "more than half",
                SettingKey::PromptReserveFraction,
                json!(0.5001),
            ),
            (
                "a negative fraction",
                SettingKey::PromptReserveFraction,
                json!(-0.1),
            ),
        ] {
            let refused = validate(key, Rungs::APP, &value, None)
                .expect_err("{label} is refused, never clamped");
            assert!(
                refused.contains(key.key()),
                "the refusal of {label} names `{key}`, got `{refused}`"
            );
        }

        let clamped = validate(
            SettingKey::ExcerptHeadLines,
            Rungs::APP,
            &json!(500),
            Some(&json!(400)),
        )
        .expect_err("a head above the cap would be clamped by the reader");
        assert!(
            clamped.contains("excerpt_file_line_cap"),
            "the `not_above` refusal names the peer it lost to, got `{clamped}`"
        );
        assert_eq!(
            validate(
                SettingKey::ExcerptHeadLines,
                Rungs::APP,
                &json!(500),
                Some(&json!(600))
            ),
            Ok(()),
            "…and accepts the same value once the peer allows it"
        );
        assert!(
            validate(
                SettingKey::ExcerptHeadLines,
                Rungs::APP,
                &json!(DEFAULTS.excerpt_file_line_cap + 1),
                None
            )
            .is_err(),
            "an absent peer is its compiled-in default, not `no rule`"
        );
    }

    fn app(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    #[test]
    fn chain_phase_project_app_default() {
        let app_rows = app(&[("token_budget", json!(90_000))]);
        let project = json!({ "token_budget": 60_000 });

        let phase = resolve_budget(Some(30_000), Some(&project), &app_rows);
        assert_eq!((phase.tokens, phase.source), (30_000, BudgetSource::Phase));

        let project_rung = resolve_budget(None, Some(&project), &app_rows);
        assert_eq!(
            (project_rung.tokens, project_rung.source),
            (60_000, BudgetSource::Project)
        );

        let app_rung = resolve_budget(None, None, &app_rows);
        assert_eq!(
            (app_rung.tokens, app_rung.source),
            (90_000, BudgetSource::AppSetting)
        );

        let floor = resolve_budget(None, None, &BTreeMap::new());
        assert_eq!(
            (floor.tokens, floor.source),
            (DEFAULTS.token_budget, BudgetSource::AppSettingDefault),
            "the compiled-in table answers, and says so"
        );
    }

    #[test]
    fn a_non_positive_rung_falls_through() {
        // The `connect.rs:299-302` rule: absent, null, non-numeric or non-positive is not a value.
        let app_rows = app(&[("token_budget", json!(90_000))]);
        for dud in [json!(0), json!(-1), json!("120000"), json!(null)] {
            let project = json!({ "token_budget": dud });
            let resolved = resolve_budget(None, Some(&project), &app_rows);
            assert_eq!(
                (resolved.tokens, resolved.source),
                (90_000, BudgetSource::AppSetting),
                "a project rung of {dud} is not a budget"
            );
        }
        for dud in [json!(0), json!(-5), json!(true)] {
            let resolved = resolve_budget(None, None, &app(&[("token_budget", dud.clone())]));
            assert_eq!(
                (resolved.tokens, resolved.source),
                (DEFAULTS.token_budget, BudgetSource::AppSettingDefault),
                "an app rung of {dud} is not a budget"
            );
        }
        assert_eq!(
            resolve_budget(Some(0), None, &app_rows).source,
            BudgetSource::AppSetting,
            "a phase budget of zero is not a budget either"
        );
    }

    #[test]
    fn reserve_is_basis_points() {
        assert_eq!(
            resolve_budget(None, None, &BTreeMap::new()).reserve_bp,
            DEFAULTS.prompt_reserve_fraction_bp
        );
        for (value, bp) in [
            (json!(0.10), 1_000),
            (json!(0.0), 0),
            (json!(0.125), 1_250),
            (json!(0.333_33), 3_333),
            (json!(0.9), 5_000),
            (json!(-0.5), 0),
        ] {
            assert_eq!(
                resolve_budget(
                    None,
                    None,
                    &app(&[("prompt_reserve_fraction", value.clone())])
                )
                .reserve_bp,
                bp,
                "{value} is {bp} basis points, clamped to 0..=5000"
            );
        }
        for dud in [json!("0.10"), json!(null)] {
            assert_eq!(
                resolve_budget(
                    None,
                    None,
                    &app(&[("prompt_reserve_fraction", dud.clone())])
                )
                .reserve_bp,
                DEFAULTS.prompt_reserve_fraction_bp,
                "{dud} is not a fraction"
            );
        }
    }

    #[test]
    fn hops_clamp_notes() {
        let mut notes = Vec::new();
        assert_eq!(resolve_hops(None, &BTreeMap::new(), &mut notes), 2);
        assert!(notes.is_empty(), "the default is not a clamp");

        let project = json!({ "upstream_hops": 1 });
        assert_eq!(
            resolve_hops(
                Some(&project),
                &app(&[("prompt_upstream_hops", json!(2))]),
                &mut notes
            ),
            1,
            "the project rung wins"
        );
        assert!(notes.is_empty());

        let dense = json!({ "upstream_hops": 7 });
        assert_eq!(resolve_hops(Some(&dense), &BTreeMap::new(), &mut notes), 2);
        assert_eq!(
            notes,
            vec!["upstream hops clamped from 7 to 2 (ANA-5 §4.3 allows 1..=2)".to_owned()],
            "§4.3 step 2: clamped and noted, never an error"
        );

        notes.clear();
        assert_eq!(
            resolve_hops(
                None,
                &app(&[("prompt_upstream_hops", json!(0))]),
                &mut notes
            ),
            2,
            "a non-positive rung falls through rather than clamping up"
        );
        assert!(notes.is_empty(), "falling through is not a clamp");
    }

    #[test]
    fn max_skill_tokens_falls_through_to_the_table() {
        assert_eq!(
            resolve_max_skill_tokens(&BTreeMap::new()),
            DEFAULTS.max_skill_tokens
        );
        assert_eq!(
            resolve_max_skill_tokens(&app(&[("max_skill_tokens", json!(500))])),
            500
        );
        assert_eq!(
            resolve_max_skill_tokens(&app(&[("max_skill_tokens", json!(-1))])),
            DEFAULTS.max_skill_tokens
        );
    }

    #[test]
    fn excerpt_caps_fall_through_key_by_key() {
        // §5.3's six `excerpt_*` keys, each independent: a project that configured two of them
        // must not lose the other four to the fall-through.
        let (caps, scan, deadline) = resolve_excerpt_caps(&BTreeMap::new());
        assert_eq!(
            caps,
            ExcerptCaps {
                max_files: DEFAULTS.excerpt_max_files,
                file_line_cap: DEFAULTS.excerpt_file_line_cap,
                head_lines: DEFAULTS.excerpt_head_lines,
                max_file_bytes: DEFAULTS.excerpt_max_file_bytes,
            }
        );
        assert_eq!(scan, DEFAULTS.excerpt_max_scan_files);
        assert_eq!(
            deadline,
            Duration::from_millis(DEFAULTS.excerpt_provider_deadline_ms)
        );

        let rows = app(&[
            ("excerpt_max_files", json!(3)),
            ("excerpt_head_lines", json!(40)),
            ("excerpt_max_scan_files", json!(500)),
            ("excerpt_provider_deadline_ms", json!(250)),
            // The `connect.rs:299-302` rule: neither of these is a value.
            ("excerpt_file_line_cap", json!(0)),
            ("excerpt_max_file_bytes", json!("524288")),
        ]);
        let (caps, scan, deadline) = resolve_excerpt_caps(&rows);
        assert_eq!(caps.max_files, 3);
        assert_eq!(caps.head_lines, 40);
        assert_eq!(caps.file_line_cap, DEFAULTS.excerpt_file_line_cap);
        assert_eq!(caps.max_file_bytes, DEFAULTS.excerpt_max_file_bytes);
        assert_eq!(scan, 500);
        assert_eq!(deadline, Duration::from_millis(250));
    }

    #[test]
    fn the_head_never_exceeds_the_line_cap() {
        // A head above the line cap is a stored typo, not a policy: `select` would then window a
        // file to more lines than the cap that made it window at all.
        let rows = app(&[
            ("excerpt_file_line_cap", json!(50)),
            ("excerpt_head_lines", json!(400)),
        ]);
        let (caps, _, _) = resolve_excerpt_caps(&rows);
        assert_eq!(caps.file_line_cap, 50);
        assert_eq!(caps.head_lines, 50, "clamped to the cap it sits under");
    }

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
                Value::String(text.to_owned())
            );
        }
    }
}
