//! `item_status` (MOD-11 M2, plan "Tools", blueprint §7.2): a status *request*, never a status
//! move (I-4).
//!
//! The agent names the status it believes the item has reached and why; the request lands as a
//! note on the run's item, `status request: <status>[ (<resolution>)] — <reason>`, through the
//! same fenced, scrubbed and capped path as `note_add`. `item.status` is moved by its own
//! compare-and-set elsewhere, never here (R-ENT-8, ANA-2 risk 11).

use htui_core::model::{Resolution, Status};
use serde_json::Value;

use super::{Ctx, ToolDef, ToolError, ToolResult, args, note, schema_of};

// The item is the scope's (I-1): a `run_id` or an `item_id` is refused. `Status` and
// `Resolution` decode through their `str_enum!` serde names, so an unknown one is
// `invalid arguments: unknown variant …`. A plain comment, not a doc: schemars would hand a doc
// to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    /// The status the item should move to.
    #[schemars(schema_with = "status_schema")]
    status: Status,
    /// Why: one line a human reads before deciding.
    reason: String,
    /// Why the item closed; only with status `closed`.
    #[serde(default)]
    #[schemars(schema_with = "resolution_schema")]
    resolution: Option<Resolution>,
}

/// `{"type": "string", "enum": [<every status>]}`.
fn status_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let names: Vec<&str> = Status::ALL.iter().map(|status| status.as_str()).collect();
    schemars::json_schema!({"type": "string", "enum": names})
}

/// `{"type": ["string", "null"], "enum": [<every resolution>, null]}`: optional, `null` when
/// absent (the `default` schemars adds for `#[serde(default)]`).
fn resolution_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut names: Vec<Value> = Resolution::ALL
        .iter()
        .map(|resolution| Value::from(resolution.as_str()))
        .collect();
    names.push(Value::Null);
    schemars::json_schema!({"type": ["string", "null"], "enum": names})
}

/// Advertised when the scope has an item.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "item_status",
    description: "Asks a human to move this step's item to a status; writes a note, never the \
                  status itself.",
    schema: schema_of::<StatusArgs>,
    advertised: |scope, _| scope.item_id.is_some(),
};

/// `{"note_id": "<uuid>"}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let StatusArgs {
        status,
        reason,
        resolution,
    } = args(arguments)?;
    let res = match resolution {
        Some(_) if status != Status::Closed => {
            return Err(ToolError(
                "refused: a resolution goes with closed only".to_owned(),
            ));
        }
        Some(resolution) => format!(" ({resolution})"),
        None => String::new(),
    };
    note::write(
        ctx.session,
        format!("status request: {status}{res} \u{2014} {reason}"),
    )
    .await
}
