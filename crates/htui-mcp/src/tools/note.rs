//! `note_add` (MOD-11 M2, plan "Tools", blueprint §7.2): a note on the step's item.
//!
//! The item is the run's (I-1) and the note names the step that wrote it; the write goes through
//! the session's fence (I-3, [`add_step_note`]). The body is scrubbed first, fail closed (I-5),
//! then capped at [`NOTE_LIMIT`] bytes. `item_status` writes its request through [`write`] too.
//!
//! [`add_step_note`]: htui_core::store::WorkerStore::add_step_note

use htui_core::model::{NewNote, NoteId};
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, scrubbed, store_error};
use crate::host::Session;

/// The longest note body an agent may write, in bytes, counted after scrubbing (plan "Tools").
pub(crate) const NOTE_LIMIT: usize = 16 * 1024;

// The item is the scope's (I-1): a `run_id` or an `item_id` is refused. A plain comment, not a
// doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoteArgs {
    /// The note, at most 16384 bytes.
    body: String,
}

/// Advertised when the scope has an item.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "note_add",
    description: "Adds a note to this step's item.",
    schema: schema_of::<NoteArgs>,
    advertised: |scope, _| scope.item_id.is_some(),
};

/// `{"note_id": "<uuid>"}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let NoteArgs { body } = args(arguments)?;
    write(ctx.session, body).await
}

/// Scrubs `body`, caps it and writes it as a note of the session's step on the session's item,
/// under the session's fence. Answers `{"note_id": "<uuid>"}`.
pub(crate) async fn write<S: htui_core::store::WorkerStore>(
    session: &Session<S>,
    body: String,
) -> ToolResult {
    let scope = &session.scope;
    let Some(item) = scope.item_id else {
        return Err(ToolError(
            "out of scope: this session has no item".to_owned(),
        ));
    };
    let body = scrubbed(session.scrubber.as_ref(), body)?;
    if body.len() > NOTE_LIMIT {
        return Err(ToolError(format!(
            "refused: note is {} bytes, the limit is {NOTE_LIMIT}",
            body.len()
        )));
    }
    let note = NewNote {
        id: NoteId::new(),
        item_id: item,
        body,
        created_by: scope.user,
        box_id: Some(scope.box_id),
        via_step_id: Some(scope.step_id),
        created_at: session.clock.now(),
    };
    let written = htui_core::store::WorkerStore::add_step_note(&session.store, scope.fence, note)
        .await
        .map_err(store_error)?;
    Ok(json!({"note_id": written.id}))
}
