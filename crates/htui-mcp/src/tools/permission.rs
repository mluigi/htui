//! `permission_prompt` (blueprint §2.10): a stub until MOD-11 T9 replaces this file — never advertised, and
//! refused if called anyway.

use serde_json::Value;

use super::{Ctx, ToolDef, ToolError, ToolResult, no_arguments};

/// Never advertised before MOD-11 T9 lands.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "permission_prompt",
    description: "Asks the person driving this session whether a tool may run.",
    schema: no_arguments,
    advertised: |_, _| false,
};

/// Refused: the tool is not in this build.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(_: Ctx<'_, H>, _: Value) -> ToolResult {
    Err(ToolError("not available in this build".into()))
}
