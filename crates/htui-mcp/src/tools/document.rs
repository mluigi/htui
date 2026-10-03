//! `document_write` (MOD-11 M2, plan "Tools", blueprint §7.2): the step's output document.
//!
//! The kind is the phase's `output_kind` and the item the run's (I-1); the write goes through the
//! session's fence (I-3, [`write_step_document`]), so a step whose run another process took
//! writes nothing. Each call writes a new version: the newest wins. Title and body are scrubbed
//! first, fail closed (I-5).
//!
//! [`write_step_document`]: htui_core::store::WorkerStore::write_step_document

use htui_core::model::{DocumentId, NewDocument};
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, scrubbed, store_error};

// The item and the kind are the scope's (I-1): a `run_id`, an `item_id` or a `kind` is refused.
// A plain comment, not a doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocumentArgs {
    /// The document's title; the output kind when omitted.
    title: Option<String>,
    /// The document's body, in Markdown.
    body: String,
}

/// Advertised when the scope has an item and a phase output kind (PRD OQ-5).
pub(crate) const DEF: ToolDef = ToolDef {
    name: "document_write",
    description: "Records this step's output document; each call writes a new version.",
    schema: schema_of::<DocumentArgs>,
    advertised: |scope, _| scope.item_id.is_some() && scope.output_kind.is_some(),
};

/// `{"document_id": "<uuid>", "kind": "<kind>", "version": n}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let DocumentArgs { title, body } = args(arguments)?;
    let session = ctx.session;
    let scope = &session.scope;
    let (Some(item), Some(kind)) = (scope.item_id, scope.output_kind.clone()) else {
        return Err(ToolError(
            "out of scope: this session has no item or no output kind".to_owned(),
        ));
    };
    let scrubber = session.scrubber.as_ref();
    let title = scrubbed(scrubber, title.unwrap_or_else(|| kind.clone()))?;
    let body = scrubbed(scrubber, body)?;
    let new = NewDocument {
        id: DocumentId::new(),
        item_id: item,
        kind,
        title,
        body,
        produced_by_step_id: Some(scope.step_id),
        created_by: scope.user,
        created_at: session.clock.now(),
    };
    let written =
        htui_core::store::WorkerStore::write_step_document(&session.store, scope.fence, new)
            .await
            .map_err(store_error)?;
    Ok(json!({
        "document_id": written.id,
        "kind": written.kind,
        "version": written.version,
    }))
}
