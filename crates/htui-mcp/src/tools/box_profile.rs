//! `box_profile` (blueprint §2.10): a stub until MOD-11 T2 commit 4 replaces this file — never advertised, and
//! refused if called anyway.

use serde_json::Value;

use super::{Ctx, ToolDef, ToolError, ToolResult, no_arguments};

/// Never advertised before MOD-11 T2 commit 4 lands.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "box_profile",
    description: "Describes the machine this session runs on: its OS, CPU, RAM, GPU and the tools found on it.",
    schema: no_arguments,
    advertised: |_, _| false,
};

/// Refused: the tool is not in this build.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(_: Ctx<'_, H>, _: Value) -> ToolResult {
    Err(ToolError("not available in this build".into()))
}
