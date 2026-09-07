//! `run_step.usage` (`docs/ANA-9.md` §4.3, `docs/ANA-4.md` §7): the one summing rule (plan D36).
//!
//! The token fields of a `usage` row are per-row **deltas** summed with saturating adds, so "take
//! the last row" is not the answer; only `cost_micros_total` is cumulative, and it is not one of
//! the five keys this type carries. A key no row ever reported stays `null`, so "the agent reports
//! no token counts" and "the agent reported zero" stay distinguishable.
//!
//! The rule lives here rather than in the recorder because two writers need it and neither may
//! depend on the other: `htui-agent`'s recorder sums the deltas as they arrive, and `htui-store`'s
//! `upload_pending` sums the same rows again when a chat that happened offline is uploaded. A
//! second, hand-written sum would drift from the first, and a step uploaded later must be
//! indistinguishable from one recorded online.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::event::{EventKind, SessionEvent};

/// The five §4.3 `usage` keys, each nullable.
///
/// The serde form **is** the `run_step.usage` document: the fields are declared in the order the
/// document has always carried them, and none is skipped when `None`, so a totals value and the
/// persisted JSONB are the same five keys either way round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageTotals {
    /// `usage.input_tokens` summed over the step.
    pub input_tokens: Option<i64>,
    /// `usage.output_tokens` summed over the step.
    pub output_tokens: Option<i64>,
    /// `usage.cache_read_tokens` summed over the step.
    pub cache_read_tokens: Option<i64>,
    /// `usage.cache_write_tokens` summed over the step.
    pub cache_write_tokens: Option<i64>,
    /// `usage.cost_micros` summed over the step. A delta per row (ANA-4 §7 derives it from the
    /// agent's cumulative `cost_micros_total`), which is what makes this a plain sum.
    pub cost_micros: Option<i64>,
}

impl UsageTotals {
    /// Adds one `usage` **payload**'s five keys; a key that is absent, `null` or not an integer
    /// adds nothing and leaves its total as it was.
    ///
    /// Takes the JSON document rather than a typed event because the typed event lives in
    /// `htui-agent` and the uploader in `htui-store`, and the payload is the one thing both hold —
    /// the recorder passes the scrubbed document it is about to persist, so both sides sum exactly
    /// the bytes the row carries.
    pub fn add_payload(&mut self, payload: &Value) {
        add_delta(&mut self.input_tokens, payload, "input_tokens");
        add_delta(&mut self.output_tokens, payload, "output_tokens");
        add_delta(&mut self.cache_read_tokens, payload, "cache_read_tokens");
        add_delta(&mut self.cache_write_tokens, payload, "cache_write_tokens");
        add_delta(&mut self.cost_micros, payload, "cost_micros");
    }

    /// [`UsageTotals::add_payload`] over every row whose kind is [`EventKind::Usage`], in slice
    /// order; rows of any other kind contribute nothing.
    ///
    /// The uploader's entry point: it holds the step's persisted rows and nothing else, and this
    /// is what turns them back into the document the online recorder would have written.
    #[must_use]
    pub fn from_rows(rows: &[SessionEvent]) -> Self {
        let mut totals = Self::default();
        for row in rows.iter().filter(|row| row.kind == EventKind::Usage) {
            totals.add_payload(&row.payload);
        }
        totals
    }

    /// The `run_step.usage` document: the five keys, each nullable, that `set_step_usage` and the
    /// uploader's `run_step` insert write.
    ///
    /// Built by hand rather than through `serde_json::to_value`, whose signature is fallible while
    /// five nullable integers cannot fail to serialize; `to_value_matches_the_serde_form` pins the
    /// two together so the document and the wire form can never drift apart.
    #[must_use]
    pub fn to_value(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_tokens": self.cache_read_tokens,
            "cache_write_tokens": self.cache_write_tokens,
            "cost_micros": self.cost_micros,
        })
    }
}

/// `slot += payload[key]`, saturating, leaving `slot` untouched when the row carried no integer
/// there — the difference between a total of `0` and a total nobody ever reported.
fn add_delta(slot: &mut Option<i64>, payload: &Value, key: &str) {
    if let Some(delta) = payload.get(key).and_then(Value::as_i64) {
        *slot = Some(slot.unwrap_or(0).saturating_add(delta));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::event::EventRole;
    use crate::model::ids::StepId;
    use chrono::{DateTime, Utc};

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    /// One `session_event` row of the given kind, as the persisted log holds it.
    fn row(step: StepId, seq: i32, kind: EventKind, payload: Value) -> SessionEvent {
        SessionEvent {
            run_step_id: step,
            seq,
            turn: 0,
            kind,
            role: EventRole::Agent,
            tool_call_id: None,
            payload,
            raw: None,
            at: at(),
        }
    }

    fn usage_row(step: StepId, seq: i32, payload: Value) -> SessionEvent {
        row(step, seq, EventKind::Usage, payload)
    }

    /// The recorder's `usage_deltas_sum_into_step_usage` case, reached from the persisted rows
    /// instead of from the live events: the uploader must land on the same document.
    #[test]
    fn from_rows_sums_the_deltas() {
        let step = StepId::new();
        let rows = vec![
            usage_row(
                step,
                0,
                json!({"input_tokens": 10, "output_tokens": 5, "cost_micros": 100}),
            ),
            usage_row(
                step,
                1,
                json!({"input_tokens": 20, "output_tokens": 7, "cost_micros": 250}),
            ),
            usage_row(
                step,
                2,
                json!({"input_tokens": 0, "output_tokens": 3, "cost_micros": 1}),
            ),
        ];

        assert_eq!(
            UsageTotals::from_rows(&rows),
            UsageTotals {
                input_tokens: Some(30),
                output_tokens: Some(15),
                cache_read_tokens: None,
                cache_write_tokens: None,
                cost_micros: Some(351),
            }
        );
    }

    /// `cost_micros_total` is the agent's cumulative figure and `cost_micros` the delta derived
    /// from it (ANA-4 §7). Summing the deltas is the rule; the cumulative key is not one of the
    /// five and must not leak into the document, or an uploaded step would double-count.
    #[test]
    fn the_cumulative_key_is_not_summed_and_never_reaches_the_document() {
        let step = StepId::new();
        let rows = vec![
            usage_row(
                step,
                0,
                json!({"cost_micros": 100, "cost_micros_total": 100}),
            ),
            usage_row(
                step,
                1,
                json!({"cost_micros": 250, "cost_micros_total": 350}),
            ),
            usage_row(step, 2, json!({"cost_micros": 1, "cost_micros_total": 351})),
        ];

        let totals = UsageTotals::from_rows(&rows);
        assert_eq!(totals.cost_micros, Some(351), "the deltas, summed");
        assert_eq!(
            totals.to_value().get("cost_micros_total"),
            None,
            "the document carries the five keys and nothing else"
        );
    }

    /// A step's log is mostly not `usage`; every other kind passes through untouched, including a
    /// row that happens to carry a `usage`-shaped payload under a different kind.
    #[test]
    fn from_rows_ignores_rows_of_other_kinds() {
        let step = StepId::new();
        let rows = vec![
            row(
                step,
                0,
                EventKind::Prompt,
                json!({"text": "hi", "digest": "abc", "input_tokens": 9_000}),
            ),
            usage_row(step, 1, json!({"input_tokens": 12})),
            row(
                step,
                2,
                EventKind::AssistantText,
                json!({"text": "hello", "output_tokens": 9_000}),
            ),
            row(step, 3, EventKind::Done, json!({"reason": "end_turn"})),
        ];

        assert_eq!(
            UsageTotals::from_rows(&rows),
            UsageTotals {
                input_tokens: Some(12),
                ..UsageTotals::default()
            }
        );
    }

    /// A step with no rows at all, and a step whose log holds no `usage` row, both report nothing
    /// rather than zero — the distinction the nullable keys exist for.
    #[test]
    fn a_step_that_reported_nothing_totals_to_five_nulls() {
        let step = StepId::new();
        assert_eq!(UsageTotals::from_rows(&[]), UsageTotals::default());
        assert_eq!(
            UsageTotals::from_rows(&[row(step, 0, EventKind::Done, json!({"reason": "end_turn"}))]),
            UsageTotals::default()
        );
        assert_eq!(
            UsageTotals::default().to_value(),
            json!({
                "input_tokens": Value::Null,
                "output_tokens": Value::Null,
                "cache_read_tokens": Value::Null,
                "cache_write_tokens": Value::Null,
                "cost_micros": Value::Null,
            })
        );
    }

    /// A key that is absent, explicitly `null`, or carries something that is not an integer adds
    /// nothing: a driver that reports three of the five keys must not make the other two `0`.
    #[test]
    fn a_key_without_an_integer_adds_nothing() {
        let mut totals = UsageTotals::default();
        totals.add_payload(&json!({
            "input_tokens": 7,
            "output_tokens": Value::Null,
            "cache_read_tokens": "many",
            "cache_write_tokens": 1.5,
        }));

        assert_eq!(
            totals,
            UsageTotals {
                input_tokens: Some(7),
                ..UsageTotals::default()
            }
        );

        totals.add_payload(&json!({}));
        assert_eq!(
            totals,
            UsageTotals {
                input_tokens: Some(7),
                ..UsageTotals::default()
            },
            "an empty payload leaves every total as it was"
        );

        totals.add_payload(&json!({"output_tokens": 0}));
        assert_eq!(
            totals.output_tokens,
            Some(0),
            "a reported zero is a report, not an absence"
        );
    }

    /// A driver that reports an absurd delta saturates rather than panicking in debug or wrapping
    /// in release: a bad number costs an inaccurate total, never the step's log.
    #[test]
    fn deltas_saturate() {
        let mut totals = UsageTotals::default();
        totals.add_payload(&json!({"cost_micros": i64::MAX}));
        totals.add_payload(&json!({"cost_micros": i64::MAX}));
        assert_eq!(totals.cost_micros, Some(i64::MAX));
    }

    /// The document and the serde form are one shape: `to_value` is hand-built for infallibility,
    /// so this is what stops the two from drifting.
    #[test]
    fn to_value_matches_the_serde_form() {
        let totals = UsageTotals {
            input_tokens: Some(30),
            output_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost_micros: Some(351),
        };

        assert_eq!(
            totals.to_value(),
            json!({
                "input_tokens": 30,
                "output_tokens": 15,
                "cache_read_tokens": Value::Null,
                "cache_write_tokens": Value::Null,
                "cost_micros": 351,
            }),
            "the document the recorder has always written"
        );
        assert_eq!(
            totals.to_value(),
            serde_json::to_value(totals).expect("five nullable integers serialize"),
            "hand-built document == derived serde form"
        );
        assert_eq!(
            serde_json::from_value::<UsageTotals>(totals.to_value()).expect("round trip"),
            totals
        );
    }
}
