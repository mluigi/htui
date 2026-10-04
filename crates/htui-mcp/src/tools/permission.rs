//! `permission_prompt` (MOD-11 D18, blueprint §2.10, §12): the tool the CLI's
//! `--permission-prompt-tool` calls before every gated tool use.
//!
//! The call is forwarded over the session's [`PromptAsk`](htui_agent::prompt_bridge::PromptAsk) to
//! the CLI session, which announces it as an ordinary `permission_request` and completes it from
//! `answer_permission`. The answer is the CLI's own contract — `{"behavior":"allow","updatedInput":
//! <input>}` or `{"behavior":"deny","message":…}` — and never `isError`: a deny is an answer. The
//! CLI hides this tool from the model, so its description is one line.

use htui_agent::prompt_bridge::{PromptCall, PromptClosed, PromptVerdict};
use htui_core::model::Transport;
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolResult};

/// Advertised to a `Transport::Cli` scope, the one whose lease carries a prompt port (B-21).
pub(crate) const DEF: ToolDef = ToolDef {
    name: "permission_prompt",
    description: "Asks the person driving this session whether a tool may run.",
    schema,
    advertised: |scope, _| scope.transport == Transport::Cli,
};

/// [`PromptCall`]'s shape, by hand: `htui-agent` derives no schema, and the object stays open
/// because the CLI may add keys.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tool_name": {
                "type": "string",
                "description": "The tool the agent wants to run."
            },
            "input": {
                "description": "The tool's input, verbatim."
            },
            "tool_use_id": {
                "type": ["string", "null"],
                "description": "The CLI's tool-use id."
            }
        },
        "required": ["tool_name", "input"]
    })
}

/// Asks the session and answers the CLI's JSON; a session that is gone answers `deny` with
/// [`PromptClosed`]'s sentence.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    args: Value,
) -> ToolResult {
    let call: PromptCall = super::args(args)?;
    let input = call.input.clone();
    let verdict = match ctx.session.ask.as_ref() {
        Some(ask) => ask.ask(call).await,
        None => Err(PromptClosed),
    };
    Ok(match verdict {
        Ok(PromptVerdict::Allow) => json!({"behavior": "allow", "updatedInput": input}),
        Ok(PromptVerdict::Deny { message }) => json!({"behavior": "deny", "message": message}),
        Err(closed) => json!({"behavior": "deny", "message": closed.to_string()}),
    })
}
