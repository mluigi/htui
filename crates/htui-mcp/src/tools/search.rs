//! `search_concepts` (blueprint §2.10): a stub until MOD-11 T7 replaces this file — never advertised, and
//! refused if called anyway.

use serde_json::Value;

use super::{Ctx, ToolDef, ToolError, ToolResult, no_arguments};

/// Never advertised before MOD-11 T7 lands.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "search_concepts",
    description: "Searches this project's items, documents and requirements by meaning.",
    schema: no_arguments,
    advertised: |_, _| false,
};

/// Refused: the tool is not in this build.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(_: Ctx<'_, H>, _: Value) -> ToolResult {
    Err(ToolError("not available in this build".into()))
}
