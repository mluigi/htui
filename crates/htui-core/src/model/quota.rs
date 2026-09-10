//! `agent_box.quota` (`docs/ANA-4.md` §7, `:1110-1135`): the one allowance document three readers
//! share, and the per-run caps that bound it (MOD-2 plan D66, D70, D71).
//!
//! The §7 blob is written by the recorder's passive latch, rendered by the Settings tab's quota
//! column and read by MOD-4's selection loop. It lives here rather than in the driver crate for
//! the reason `usage.rs` gives for [`UsageTotals`](crate::model::UsageTotals) (`usage.rs:8-12`):
//! two crates that may not depend on each other both need one definition of it, and a second
//! hand-written normalizer would drift from the first. [`Quota::to_value`] is hand-built and
//! pinned to the derived serde form by `to_value_matches_the_serde_form_and_round_trips`, as
//! `usage.rs:73-85` does and says why — the document is infallible to build, and
//! `serde_json::to_value` is not.
//!
//! **Nothing here is keyed on an agent's name** (`R-AGT-5`). [`normalize`] selects on
//! [`QuotaSource`], which is a value the `agent` row declares
//! (`agent.settings.quota.source`, §5.2), and it reads every vendor key of the raw blob through
//! `get` / `as_*`, so a partial or foreign-shaped report costs a missing window and never a panic
//! (blueprint H-1). A vendor field this milestone does not understand is not lost either: the raw
//! blob stays on the `usage` row that carried it, which is D66's reason for keeping it verbatim.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::Billing;

str_enum!(
    /// `agent.settings.quota.source` (`docs/ANA-4.md` §5.2, §7): where a row's allowance is read
    /// from. Declared on the row, never derived from its name (`R-AGT-5`).
    ///
    /// Lives here rather than in the driver crate since MOD-2 milestone 7 because [`normalize`]
    /// selects on it and this crate cannot see `htui-agent`; `htui_agent::launch` re-exports it
    /// unchanged, so the driver-side spelling of the type is the same type.
    #[derive(Default)]
    QuotaSource {
        /// ACP's `_meta` rate-limit blob on a `usage_update` (§7's transport table).
        AcpMetaRateLimit => "acp_meta_rate_limit",
        /// A rate-limit event in the CLI's JSON stream (milestone 8).
        CliRateLimitEvent => "cli_rate_limit_event",
        /// The CLI's status line (milestone 8).
        CliStatusLine => "cli_status_line",
        /// No quota is reported. The default: a row that says nothing promises nothing.
        #[default]
        None => "none",
    }
);

/// The currency a `spend` figure is in whenever it has one: micros are USD by definition
/// (`UsageEvent.cost_micros`, ANA-4 §7).
const USD: &str = "USD";

/// One allowance window of the §7 document: `{ id, utilization, resets_at }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    /// The vendor's window id, e.g. `five_hour`, `seven_day`.
    pub id: String,
    /// The 0..1 fraction consumed; `>= 1.0` is a full window (§7's second skip rule).
    pub utilization: f64,
    /// The vendor's epoch converted to a timestamp; `None` when the window named none, or named
    /// one no `DateTime` can hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
}

/// `quota.spend`: what this session has cost so far, in USD micros.
///
/// The figure is the recorder's running `cost_micros` sum, which for an ACP session equals the
/// last `cost_micros_total` (§11 criterion 7) — so the latched document and a cap verdict come
/// from one number by construction, and a transport that reports deltas with no cumulative total
/// gets a spend figure too (blueprint P-5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spend {
    /// `None` when no USD cost has been reported this session: a context-only transport, or a
    /// non-USD one. A cap in micros has nothing to compare against either way (blueprint H-5), and
    /// "no USD spend" is the honest answer rather than a wrong number.
    pub session_micros: Option<i64>,
    /// `"USD"` whenever `session_micros` is `Some`, `None` otherwise: micros are USD by definition
    /// (`UsageEvent.cost_micros`, ANA-4 §7).
    pub currency: Option<String>,
}

/// The `agent_box.quota` document of `docs/ANA-4.md` §7 (`:1112-1125`).
///
/// Seven keys, and the serde form **is** the stored document: the fields are declared in the
/// order §7 prints them and [`Quota::to_value`] writes the same seven. Key *order* in the stored
/// bytes is deliberately not a promise — `JSONB` does not keep it, and `serde_json`'s
/// `preserve_order` is enabled by a dependency of the driver crate and not by this one, so the
/// order is a property of the build. Nothing compares these documents as text: `Value` equality
/// over an object is order-insensitive under either map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    /// Which source produced this document, i.e. how the raw blob was read.
    pub source: QuotaSource,
    /// `agent.billing` of the row this document belongs to: a `per_token` agent has no windows and
    /// carries `spend` alone (§7).
    pub billing: Billing,
    /// The vendor's status string verbatim (`allowed`, `allowed_warning`, `rejected`, …); `None`
    /// when the source reports none. Only a present, non-`allowed` value is a skip (plan D72).
    #[serde(default)]
    pub status: Option<String>,
    /// Derived, never reported: `status == "rejected"`, or any window at `utilization >= 1.0`.
    #[serde(default)]
    pub exhausted: bool,
    /// Sorted by `id`, so two latches of one blob are byte-identical.
    #[serde(default)]
    pub windows: Vec<QuotaWindow>,
    /// The session's spend so far.
    #[serde(default)]
    pub spend: Spend,
    /// When this document was observed; `agent_box.quota_at` mirrors it (§7).
    pub observed_at: DateTime<Utc>,
}

impl Quota {
    /// The seven keys in declaration order, hand-built.
    ///
    /// Infallible where `serde_json::to_value` is fallible, and pinned to the derived form by
    /// `to_value_matches_the_serde_form_and_round_trips` so the stored document and the wire form
    /// can never drift apart (the rule `usage.rs:70-85` states). Timestamps are written as
    /// chrono's serde writes them: `to_rfc3339_opts(SecondsFormat::AutoSi, true)`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "source": self.source.as_str(),
            "billing": self.billing.as_str(),
            "status": self.status,
            "exhausted": self.exhausted,
            "windows": self.windows.iter().map(window_value).collect::<Vec<_>>(),
            "spend": {
                "session_micros": self.spend.session_micros,
                "currency": self.spend.currency,
            },
            "observed_at": rfc3339(self.observed_at),
        })
    }

    /// A stored document back into the type; `None` for anything that does not parse.
    ///
    /// A pre-milestone-7 hand-written blob — the fixtures' `{ "remaining": 100 }` — is exactly
    /// that case, and it is *not* an error: the column has held whatever an earlier build put
    /// there, and a reader that cannot parse it knows only that it does not know (`R-AGT-8`'s
    /// "unknown is available").
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    /// The window with the highest `utilization`, first by `id` on a tie: what Settings shows.
    ///
    /// `windows` is sorted by `id`, so "first on a tie" is the earlier id.
    #[must_use]
    pub fn tightest_window(&self) -> Option<&QuotaWindow> {
        self.windows.iter().reduce(|tightest, window| {
            if window.utilization > tightest.utilization {
                window
            } else {
                tightest
            }
        })
    }
}

/// §7's document, from what one `usage` row and its row-side facts say (plan D66, blueprint P-5).
///
/// `raw` is the vendor blob **as scrubbed and persisted** on the `usage` row — never the wire
/// message — so the latched document can carry nothing the row does not.
///
/// For [`QuotaSource::AcpMetaRateLimit`]: `status` is `raw.status` as a string; `windows` is every
/// entry of `raw.unifiedWindows` whose `utilization` is a number, `resets_at` from that entry's
/// `resetsAt` read as epoch seconds (`None` when it is absent, not an integer, or out of range),
/// sorted by id. Any other source, and `raw == None`, give no status, no windows and
/// `exhausted: false` — a source with no reader yet reports nothing rather than guessing at a
/// shape (milestone 8 owns the two CLI ones). `spend` is `session_micros` with `"USD"` iff `Some`.
///
/// Every vendor key is read through `get` / `as_*`: a report with no `cost`, a `cost` in another
/// currency, a missing `unifiedWindows` or a window without a numeric `utilization` each cost that
/// one field and nothing else (blueprint H-1).
#[must_use]
pub fn normalize(
    source: QuotaSource,
    billing: Billing,
    raw: Option<&Value>,
    session_micros: Option<i64>,
    observed_at: DateTime<Utc>,
) -> Quota {
    // The selection D66 asks for: on the row's declared source, never on the agent's name.
    let blob = match source {
        QuotaSource::AcpMetaRateLimit => raw,
        QuotaSource::CliRateLimitEvent | QuotaSource::CliStatusLine | QuotaSource::None => None,
    };
    let status = blob
        .and_then(|raw| raw.get("status"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut windows: Vec<QuotaWindow> = blob
        .and_then(|raw| raw.get("unifiedWindows"))
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(id, body)| window(id, body))
                .collect()
        })
        .unwrap_or_default();
    // The sort is what makes the window list a function of the blob's content rather than of the
    // order its keys happened to arrive in — `serde_json::Map` iterates in insertion order or in
    // key order depending on a feature a dependency turns on — so two latches of one blob agree
    // (§7's "sorted by id").
    windows.sort_by(|left, right| left.id.cmp(&right.id));
    let exhausted = status.as_deref() == Some(REJECTED)
        || windows.iter().any(|window| window.utilization >= 1.0);
    Quota {
        source,
        billing,
        status,
        exhausted,
        windows,
        spend: Spend {
            session_micros,
            currency: session_micros.map(|_| USD.to_owned()),
        },
        observed_at,
    }
}

/// The one status string that is a *derived* exhaustion rather than a reported one (§7's first
/// skip rule). Not an agent's word for itself: it is the value the source's own vocabulary uses,
/// carried through as data like every other status.
const REJECTED: &str = "rejected";

/// One `unifiedWindows` entry, or `None` when it carries no numeric `utilization`.
///
/// The `utilization` is the only required key: a window without one says nothing about an
/// allowance, and defaulting it to `0` would report a full window as an empty one.
fn window(id: &str, body: &Value) -> Option<QuotaWindow> {
    let utilization = body.get("utilization").and_then(Value::as_f64)?;
    Some(QuotaWindow {
        id: id.to_owned(),
        utilization,
        resets_at: body
            .get("resetsAt")
            .and_then(Value::as_i64)
            .and_then(|secs| DateTime::from_timestamp(secs, 0)),
    })
}

/// One window as the document holds it, skipping `resets_at` when there is none — the
/// `skip_serializing_if` of [`QuotaWindow::resets_at`], by hand.
fn window_value(window: &QuotaWindow) -> Value {
    let mut object = Map::new();
    object.insert("id".to_owned(), Value::String(window.id.clone()));
    object.insert("utilization".to_owned(), json!(window.utilization));
    if let Some(resets_at) = window.resets_at {
        object.insert("resets_at".to_owned(), Value::String(rfc3339(resets_at)));
    }
    Value::Object(object)
}

/// A timestamp exactly as chrono's `Serialize` writes it.
fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// `project.settings.per_token_cap_run` (plan D70): the per-chat-run cap, in USD micros.
pub const PER_TOKEN_CAP_RUN: &str = "per_token_cap_run";

/// `project.settings.per_token_cap_batch` (plan D71): the batch cap, in USD micros. Read and
/// reported here, enforced by MOD-12 (`docs/ANA-4.md:1283`).
pub const PER_TOKEN_CAP_BATCH: &str = "per_token_cap_batch";

/// The two `project.settings` cap keys, in USD micros (plan D70, D71).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectCaps {
    /// Enforced by the recorder per chat run (plan D70). `None` = unbounded.
    pub run_micros: Option<i64>,
    /// Read and reported, **not** enforced here: MOD-12's (plan D71).
    pub batch_micros: Option<i64>,
}

/// A cap key that is present and is not a non-negative integer.
///
/// A plain struct with a hand-written [`core::fmt::Display`], as
/// [`ParseEnumError`](crate::model::ParseEnumError) is: `htui-core`'s one `thiserror` type is
/// [`StoreError`](crate::store::StoreError), and a settings value is not a store failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapError {
    /// Which cap key was rejected: [`PER_TOKEN_CAP_RUN`] or [`PER_TOKEN_CAP_BATCH`].
    pub key: &'static str,
    /// The offending value as JSON text, so the message names what the operator actually wrote.
    pub found: String,
}

impl core::fmt::Display for CapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "project.settings.{} must be a non-negative integer of USD micros, got {}",
            self.key, self.found
        )
    }
}

impl ProjectCaps {
    /// Reads both cap keys out of a `project.settings` document.
    ///
    /// Absent or `null` is `None` — unbounded. An integer `>= 0` is the cap. Anything else
    /// (negative, float, string, bool, object) is an **error**, because a cap the operator wrote
    /// and `htui` silently ignored is the risk table's "wrong by a factor of a million" in the
    /// other direction. `0` is a real cap: it cancels on the first row that reports any USD cost.
    ///
    /// # Errors
    ///
    /// [`CapError`] naming the first offending key, in declaration order.
    pub fn from_settings(settings: &Value) -> Result<Self, CapError> {
        Ok(Self {
            run_micros: cap_at(settings, PER_TOKEN_CAP_RUN)?,
            batch_micros: cap_at(settings, PER_TOKEN_CAP_BATCH)?,
        })
    }
}

/// One cap key: absent or `null` → `None`, a non-negative integer → `Some`, anything else → the
/// error that names the key and what was found.
fn cap_at(settings: &Value, key: &'static str) -> Result<Option<i64>, CapError> {
    match settings.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_i64() {
            Some(micros) if micros >= 0 => Ok(Some(micros)),
            _ => Err(CapError {
                key,
                found: value.to_string(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module's fixed observation instant: `2026-09-03T00:00:00Z`.
    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid instant")
    }

    /// An epoch second as a timestamp, so no test copies an RFC 3339 string by hand.
    fn epoch(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).expect("a valid instant")
    }

    /// Line 5 of `crates/htui-agent/tests/fixtures/claude_acp_turn.jsonl`, verbatim: the real
    /// `_meta` rate-limit value this milestone was designed against (blueprint C).
    fn blob() -> Value {
        json!({
            "status": "allowed",
            "resetsAt": 1_788_801_600_i64,
            "rateLimitType": "five_hour",
            "overageStatus": "rejected",
            "overageDisabledReason": "org_level_disabled",
            "isUsingOverage": false,
            "unifiedWindows": {
                "five_hour": { "utilization": 0.11, "resetsAt": 1_788_801_600_i64 },
                "seven_day": { "utilization": 0.62, "resetsAt": 1_788_854_400_i64 },
            },
        })
    }

    /// Blueprint C's worked example, end to end: the real blob and a spend figure become §7's
    /// seven-key document, windows sorted by id and each epoch converted to a timestamp.
    ///
    /// `overageStatus`, `rateLimitType` and `isUsingOverage` are deliberately **not** normalized:
    /// they stay on the `usage` row's `quota` key, which is D66's reason for recording the raw
    /// blob. `overageStatus: "rejected"` in particular must not make the document `exhausted` —
    /// that rule reads `status`, and this document's `status` is `allowed`.
    #[test]
    fn normalize_lifts_status_windows_and_spend_from_the_claude_blob() {
        let quota = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&blob()),
            Some(351),
            at(),
        );

        assert_eq!(
            quota.to_value(),
            json!({
                "source": "acp_meta_rate_limit",
                "billing": "subscription",
                "status": "allowed",
                "exhausted": false,
                "windows": [
                    {
                        "id": "five_hour",
                        "utilization": 0.11,
                        "resets_at": epoch(1_788_801_600),
                    },
                    {
                        "id": "seven_day",
                        "utilization": 0.62,
                        "resets_at": epoch(1_788_854_400),
                    },
                ],
                "spend": { "session_micros": 351, "currency": "USD" },
                "observed_at": at(),
            }),
            "the §7 document of ANA-4:1112-1125"
        );
        assert_eq!(
            quota.tightest_window().map(|window| window.id.as_str()),
            Some("seven_day"),
            "the highest utilization, which is what Settings renders"
        );
    }

    /// A row that declares no source normalizes to spend and nothing else, whatever a blob it was
    /// handed happens to say: the source is the row's declaration and the only selector (D66).
    ///
    /// This is the seeded `agy` shape (`source: "none"`, `billing: subscription`), and it is why
    /// its quota column reads a spend figure or nothing at all (plan D65).
    #[test]
    fn a_source_that_reports_nothing_normalizes_to_spend_only() {
        for source in [
            QuotaSource::None,
            QuotaSource::CliRateLimitEvent,
            QuotaSource::CliStatusLine,
        ] {
            let quota = normalize(
                source,
                Billing::Subscription,
                Some(&blob()),
                Some(394_692),
                at(),
            );
            assert_eq!(quota.status, None, "{source}: no status is reported");
            assert!(
                quota.windows.is_empty(),
                "{source}: no windows are reported"
            );
            assert!(!quota.exhausted, "{source}: nothing to be exhausted from");
            assert_eq!(
                quota.spend,
                Spend {
                    session_micros: Some(394_692),
                    currency: Some("USD".to_owned()),
                },
                "{source}: the spend figure is transport-neutral"
            );
        }
    }

    /// `exhausted` is derived and never reported: a `rejected` status, or any window at or past
    /// `1.0`. A window at `0.999` with an `allowed` status is not exhausted.
    #[test]
    fn exhausted_is_derived_from_rejected_or_a_full_window() {
        let rejected = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({ "status": "rejected" })),
            None,
            at(),
        );
        assert!(rejected.exhausted, "a rejected status is exhausted");
        assert!(rejected.windows.is_empty());

        let full = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({
                "status": "allowed",
                "unifiedWindows": {
                    "five_hour": { "utilization": 0.2 },
                    "seven_day": { "utilization": 1.0 },
                },
            })),
            None,
            at(),
        );
        assert!(full.exhausted, "a window at 1.0 is a full window");

        let nearly = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({
                "status": "allowed",
                "unifiedWindows": { "five_hour": { "utilization": 0.999 } },
            })),
            None,
            at(),
        );
        assert!(!nearly.exhausted, "0.999 is not 1.0");
    }

    /// Blueprint H-1, one shape per clause: a report with no `cost`, a `cost` in another currency,
    /// a blob with no `unifiedWindows` at all, a window carrying no numeric `utilization`, a
    /// window with no `resetsAt`, and one whose epoch no timestamp can hold.
    ///
    /// Every vendor key is read through `get` / `as_*`, so each of these costs exactly the field
    /// it concerns. A window without a `utilization` is **dropped** rather than reported at `0`:
    /// reporting it as empty is the one wrong answer, since `R-AGT-8` would then select a row
    /// whose allowance is unknown as if it were fresh.
    #[test]
    fn a_partial_or_foreign_blob_costs_one_field_and_never_a_panic() {
        // No `cost` on the report, and a `cost` the mapper could not convert: both arrive here as
        // `session_micros: None`, because `cost_micros` stays `None` on a non-USD currency
        // (`acp/map.rs`). The blob says "no USD spend" rather than a number in the wrong unit.
        let context_only = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::PerToken,
            Some(&blob()),
            None,
            at(),
        );
        assert_eq!(
            context_only.spend,
            Spend {
                session_micros: None,
                currency: None,
            },
            "no USD figure means no currency either"
        );
        assert_eq!(
            context_only.windows.len(),
            2,
            "a missing cost costs the spend key and nothing else"
        );

        // `_meta` present, `unifiedWindows` absent: the status still lands.
        let no_windows = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({ "status": "allowed_warning", "resetsAt": 1_788_801_600_i64 })),
            Some(10),
            at(),
        );
        assert_eq!(no_windows.status.as_deref(), Some("allowed_warning"));
        assert!(no_windows.windows.is_empty());
        assert_eq!(no_windows.tightest_window(), None);

        // A window without a numeric `utilization`, one without a `resetsAt`, and one whose epoch
        // is out of range for a timestamp.
        let partial = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({
                "unifiedWindows": {
                    "five_hour": { "resetsAt": 1_788_801_600_i64 },
                    "seven_day": { "utilization": 0.62 },
                    "monthly": { "utilization": "most of it" },
                    "yearly": { "utilization": 0.5, "resetsAt": i64::MAX },
                },
                "status": Value::Null,
            })),
            Some(10),
            at(),
        );
        assert_eq!(
            partial
                .windows
                .iter()
                .map(|window| (window.id.as_str(), window.utilization, window.resets_at))
                .collect::<Vec<_>>(),
            vec![("seven_day", 0.62, None), ("yearly", 0.5, None)],
            "only a window with a numeric utilization is reported, sorted by id"
        );
        assert_eq!(
            partial.status, None,
            "an explicit null status is no status, not the string `null`"
        );
        assert!(!partial.exhausted);
        assert_eq!(
            partial.to_value()["windows"][0],
            json!({ "id": "seven_day", "utilization": 0.62 }),
            "a window with no reset is two keys, as the serde form skips the third"
        );

        // A blob that is not an object at all: the recorder filters these out, and nothing here
        // depends on that filter having run.
        let scalar = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!("rate limited")),
            None,
            at(),
        );
        assert_eq!(scalar.status, None);
        assert!(scalar.windows.is_empty());
    }

    /// The hand-built document and the derived serde form are one shape, and the document parses
    /// back into the type it came from. This is what stops [`Quota::to_value`] from drifting.
    #[test]
    fn to_value_matches_the_serde_form_and_round_trips() {
        for quota in [
            normalize(
                QuotaSource::AcpMetaRateLimit,
                Billing::Subscription,
                Some(&blob()),
                Some(351),
                at(),
            ),
            normalize(QuotaSource::None, Billing::PerToken, None, None, at()),
            normalize(
                QuotaSource::AcpMetaRateLimit,
                Billing::Subscription,
                Some(&json!({
                    "status": "rejected",
                    "unifiedWindows": { "five_hour": { "utilization": 1.0 } },
                })),
                Some(0),
                at(),
            ),
        ] {
            assert_eq!(
                quota.to_value(),
                serde_json::to_value(&quota).expect("the document serializes"),
                "hand-built document == derived serde form"
            );
            assert_eq!(
                Quota::from_value(&quota.to_value()),
                Some(quota.clone()),
                "the stored document parses back into the type that wrote it"
            );
            // Sorted, because key *order* is a property of the build (see [`Quota`]'s doc) while
            // the key *set* is §7's contract.
            let document = quota.to_value();
            let mut keys: Vec<&str> = document
                .as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                [
                    "billing",
                    "exhausted",
                    "observed_at",
                    "source",
                    "spend",
                    "status",
                    "windows"
                ],
                "the seven keys of §7 and nothing else"
            );
        }
    }

    /// A document from before milestone 7 — the fixtures' hand-written `{ "remaining": 100 }` —
    /// does not parse, and that is a `None` rather than a panic or an empty `Quota`: an empty one
    /// would claim "no windows, no spend" about a row nobody has ever latched.
    #[test]
    fn from_value_refuses_a_pre_milestone_document() {
        assert_eq!(Quota::from_value(&json!({ "remaining": 100 })), None);
        assert_eq!(Quota::from_value(&Value::Null), None);
        assert_eq!(Quota::from_value(&json!("allowed")), None);
        // The three required keys: a document missing any one of them is not a §7 document.
        assert_eq!(
            Quota::from_value(&json!({ "source": "none", "billing": "per_token" })),
            None,
            "no observed_at"
        );
        assert_eq!(
            Quota::from_value(&json!({ "source": "none", "observed_at": at() })),
            None,
            "no billing"
        );
        // The four optional keys default, so a document written by a leaner writer still parses.
        assert_eq!(
            Quota::from_value(&json!({
                "source": "none",
                "billing": "per_token",
                "observed_at": at(),
            })),
            Some(Quota {
                source: QuotaSource::None,
                billing: Billing::PerToken,
                status: None,
                exhausted: false,
                windows: Vec::new(),
                spend: Spend::default(),
                observed_at: at(),
            })
        );
    }

    /// The caps: absent is unbounded, `0` is a real cap, and anything that is not a non-negative
    /// integer is an error naming the key — a cap the operator wrote and `htui` ignored is the
    /// failure this refuses to have.
    #[test]
    fn project_caps_read_integers_reject_the_rest_and_treat_absent_as_unbounded() {
        assert_eq!(
            ProjectCaps::from_settings(&json!({})).expect("an empty document is unbounded"),
            ProjectCaps::default()
        );
        assert_eq!(
            ProjectCaps::from_settings(&json!({
                "per_token_cap_run": Value::Null,
                "keep_raw_events": true,
            }))
            .expect("a null cap is unbounded"),
            ProjectCaps::default()
        );
        assert_eq!(
            ProjectCaps::from_settings(&json!({
                "per_token_cap_run": 0,
                "per_token_cap_batch": 5_000_000_i64,
            }))
            .expect("two integers"),
            ProjectCaps {
                run_micros: Some(0),
                batch_micros: Some(5_000_000),
            },
            "0 is a cap, not an absence"
        );

        for bad in [json!(-1), json!(1.5), json!("300"), json!(true), json!({})] {
            let error = ProjectCaps::from_settings(&json!({ "per_token_cap_run": bad.clone() }))
                .expect_err("a cap that is not a non-negative integer is refused");
            assert_eq!(error.key, PER_TOKEN_CAP_RUN);
            assert_eq!(error.found, bad.to_string());
            assert_eq!(
                error.to_string(),
                format!(
                    "project.settings.per_token_cap_run must be a non-negative integer of USD \
                     micros, got {bad}"
                )
            );
        }
        // The batch key is refused on its own terms, and the message names *it*.
        let error = ProjectCaps::from_settings(&json!({ "per_token_cap_batch": -1 }))
            .expect_err("a negative batch cap is refused");
        assert_eq!(error.key, PER_TOKEN_CAP_BATCH);
    }

    /// `tightest_window` takes the highest utilization and, on a tie, the first by id — which is
    /// the first in `windows`, because [`normalize`] sorts it.
    #[test]
    fn the_tightest_window_breaks_a_tie_on_the_lower_id() {
        let quota = normalize(
            QuotaSource::AcpMetaRateLimit,
            Billing::Subscription,
            Some(&json!({
                "unifiedWindows": {
                    "seven_day": { "utilization": 0.4 },
                    "five_hour": { "utilization": 0.4 },
                },
            })),
            None,
            at(),
        );
        assert_eq!(
            quota
                .windows
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["five_hour", "seven_day"],
            "sorted by id, whatever order the blob listed them in"
        );
        assert_eq!(
            quota.tightest_window().map(|window| window.id.as_str()),
            Some("five_hour")
        );
        assert_eq!(
            normalize(QuotaSource::None, Billing::PerToken, None, None, at()).tightest_window(),
            None
        );
    }

    /// The source strings are `agent.settings.quota.source`'s vocabulary (§5.2), and the default
    /// is the one that promises nothing.
    #[test]
    fn the_source_vocabulary_is_the_four_strings_of_the_settings_document() {
        assert_eq!(
            QuotaSource::ALL
                .iter()
                .map(|source| source.as_str())
                .collect::<Vec<_>>(),
            [
                "acp_meta_rate_limit",
                "cli_rate_limit_event",
                "cli_status_line",
                "none"
            ]
        );
        assert_eq!(QuotaSource::default(), QuotaSource::None);
        assert_eq!(
            serde_json::to_value(QuotaSource::AcpMetaRateLimit).expect("a string"),
            json!("acp_meta_rate_limit")
        );
    }
}
