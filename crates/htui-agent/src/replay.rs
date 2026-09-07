//! The inverse of [`record`](crate::record) (plan MOD-2 D37): a persisted `session_event` row
//! back into the [`DriverEnvelope`] the live path renders.
//!
//! The recorder writes each payload as the serde form of the event's inner struct
//! ([`round_trip`](crate::record), `record.rs:965`), so the inverse is `serde_json::from_value`
//! per kind - a table, not a parser. That is the whole point of D37: replay and a live session
//! reach the chat tab through **one** renderer, and a second decoder written by hand would drift
//! from the encoder on the first kind an adapter adds.
//!
//! Two forms, because "an unknown kind never costs the transcript" and a `Result` are both worth
//! having. [`envelope_from_row`] is strict and is what the tests and the round-trip regression net
//! assert over; [`envelope_or_other`] is D37's policy - a row that will not decode still reaches
//! the transcript as [`DriverEvent::Other`], where it renders as a dim `<kind>` line instead of
//! taking the rest of the conversation down with it. The UI calls the lossy form.
//!
//! Three kinds have no [`DriverEvent`] variant at all: `prompt`, `follow_up` and
//! `permission_answer` are authored by `htui`, not by a driver
//! (`event.rs:155`). They decode to `Other { update: "<kind>", body: <payload> }`, which is
//! exactly the shape the live tab already receives for them (`chat/transcript.rs:275`) - so a
//! replayed conversation renders like the one that was recorded, down to a resolved permission
//! row.
//!
//! Two adjustments make a decoded row set match what the live path saw, both stated in blueprint
//! section E:
//!
//! - **The column fills the payload.** ANA-9 §4.3's key list omits `tool_call_id` for the kinds
//!   that have the column, and a row written from that spec (the demo fixture, a hand-written
//!   row) carries the id only in `session_event.tool_call_id`. When the payload is an object
//!   without the key and the column has one, the column's value is inserted before decoding. A
//!   recorder row never needs it, and a payload that has the key wins.
//! - **The `seq` stamp.** `assistant_text` / `thought` rows carry no `message_id`, and the
//!   transcript opens one row per group: two adjacent rows decoded with `None` would be glued
//!   into one and the last line of the first would run into the first line of the second. Each
//!   row is stamped `message_id = Some("seq:<n>")` instead - one persisted row, one transcript
//!   row, which is the live structure. The one divergence left is a text run the recorder's
//!   [`CHUNK_FLUSH_BYTES`](crate::record::CHUNK_FLUSH_BYTES) bound split into two rows, which
//!   rendered live as one.

use htui_core::model::{EventKind, SessionEvent};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::event::{DriverEnvelope, DriverEvent, OtherEvent, TextChunk};

/// A row whose payload does not read as the struct [`record`](crate::record) wrote for its kind.
///
/// Carries the kind, the `seq` and serde's reason - never the payload, which may hold text the
/// scrubber masked and an error message is the one place a masked document tends to get copied
/// back out of.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("seq {seq}: a `{kind}` payload does not decode: {reason}")]
pub struct ReplayError {
    /// The row's `session_event.kind`, i.e. which struct was expected.
    pub kind: EventKind,
    /// The row's `session_event.seq`, which is what names the row inside its step.
    pub seq: i32,
    /// Serde's own reason, for the maintainer reading a log line.
    pub reason: String,
}

/// The strict decode: `Ok` for every row the recorder can have written, `Err` for one it cannot.
///
/// The three `htui`-authored kinds and `other` are always `Ok`, as [`DriverEvent::Other`]. Tests
/// and the round-trip regression net call this; the transcript calls [`envelope_or_other`],
/// because a decode failure there would cost the reader the rest of the step.
///
/// # Errors
/// [`ReplayError`] when the payload does not deserialise as the kind's struct.
pub fn envelope_from_row(row: &SessionEvent) -> Result<DriverEnvelope, ReplayError> {
    Ok(DriverEnvelope {
        event: event_from_row(row)?,
        // `role` and `turn` are not read: the transcript renders neither, and the row's own kind
        // already says who authored it.
        raw: row.raw.clone(),
        at: row.at,
    })
}

/// D37's policy: [`envelope_from_row`], and on `Err` the row as
/// `Other { update: <kind>, body: <payload> }`.
///
/// Nothing is dropped - the payload rides along in `body`, so a maintainer looking at the raw row
/// sees what the transcript saw.
#[must_use]
pub fn envelope_or_other(row: &SessionEvent) -> DriverEnvelope {
    envelope_from_row(row).unwrap_or_else(|_| DriverEnvelope {
        event: verbatim(row),
        raw: row.raw.clone(),
        at: row.at,
    })
}

/// [`envelope_or_other`] over rows in `seq` order.
///
/// The sort is defensive: every backend's `step_events` already orders by `seq`, but a slice a
/// caller assembled need not, and replay order is `seq` and never `at` (ANA-9 §4.3).
#[must_use]
pub fn envelopes(rows: &[SessionEvent]) -> Vec<DriverEnvelope> {
    let mut ordered: Vec<&SessionEvent> = rows.iter().collect();
    ordered.sort_by_key(|row| row.seq);
    ordered.into_iter().map(envelope_or_other).collect()
}

/// The decode table of blueprint section E, one arm per [`EventKind`].
fn event_from_row(row: &SessionEvent) -> Result<DriverEvent, ReplayError> {
    match row.kind {
        // No `DriverEvent` variant exists for the three kinds `htui` authors itself; the live tab
        // receives them as `other` too, so this *is* the shape, not a fallback.
        EventKind::Prompt | EventKind::FollowUp | EventKind::PermissionAnswer => Ok(verbatim(row)),
        EventKind::AssistantText => Ok(DriverEvent::AssistantChunk(chunk(row)?)),
        EventKind::Thought => Ok(DriverEvent::ThoughtChunk(chunk(row)?)),
        EventKind::ToolCall => Ok(DriverEvent::ToolCall(decode(row, filled(row))?)),
        EventKind::ToolResult => Ok(DriverEvent::ToolResult(decode(row, filled(row))?)),
        EventKind::EditProposal => Ok(DriverEvent::EditProposal(decode(row, filled(row))?)),
        EventKind::PermissionRequest => {
            Ok(DriverEvent::PermissionRequest(decode(row, filled(row))?))
        }
        EventKind::Plan => Ok(DriverEvent::Plan(decode(row, row.payload.clone())?)),
        EventKind::Usage => Ok(DriverEvent::Usage(decode(row, row.payload.clone())?)),
        EventKind::Error => Ok(DriverEvent::Error(decode(row, row.payload.clone())?)),
        EventKind::Done => Ok(DriverEvent::Done(decode(row, row.payload.clone())?)),
        // The persisted payload *is* an `OtherEvent` (`{ update, body }`), so this arm is the
        // identity - including for the `session_started` banner, from which the header lifts the
        // session id.
        EventKind::Other => Ok(DriverEvent::Other(decode(row, row.payload.clone())?)),
    }
}

/// The row as `Other { update: <kind>, body: <payload> }`: what the three `htui`-authored kinds
/// decode to, and what the lossy form falls back to for anything else.
fn verbatim(row: &SessionEvent) -> DriverEvent {
    DriverEvent::Other(OtherEvent {
        update: row.kind.as_str().to_owned(),
        body: row.payload.clone(),
    })
}

/// One coalesced text row, stamped with the grouping key the module doc explains.
fn chunk(row: &SessionEvent) -> Result<TextChunk, ReplayError> {
    let mut chunk: TextChunk = decode(row, row.payload.clone())?;
    chunk.message_id = Some(format!("seq:{}", row.seq));
    Ok(chunk)
}

/// The payload with `session_event.tool_call_id` inserted when the payload is an object that does
/// not already carry it (the "column fills the payload" rule).
fn filled(row: &SessionEvent) -> Value {
    let mut payload = row.payload.clone();
    if let Some(id) = row.tool_call_id.as_ref()
        && let Some(object) = payload.as_object_mut()
        && !object.contains_key("tool_call_id")
    {
        object.insert("tool_call_id".to_owned(), Value::String(id.clone()));
    }
    payload
}

/// `serde_json::from_value`, with the row's coordinates attached to the failure.
fn decode<T: DeserializeOwned>(row: &SessionEvent, payload: Value) -> Result<T, ReplayError> {
    serde_json::from_value(payload).map_err(|error| ReplayError {
        kind: row.kind,
        seq: row.seq,
        reason: error.to_string(),
    })
}
