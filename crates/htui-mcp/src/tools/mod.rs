//! The tool table (MOD-11 blueprint §2.10): eight tools, one file each, fixed here once.
//!
//! [`ALL`]'s order is `tools/list`'s order and a pin. Each file exports a [`ToolDef`] named `DEF`
//! and an `async fn call`; a tool's owning task replaces both in its own file and never edits this
//! one. A tool whose `advertised` answers `false` for a scope is absent from that session's
//! `tools/list` and refused by `tools/call` (I-7).

use htui_core::scrub::Scrubber;
use htui_core::store::StoreError;
use htui_orch::tools::ToolScope;
use serde_json::Value;

use crate::host::Session;
use crate::protocol::Progress;

mod box_profile;
mod command;
mod document;
mod link;
mod note;
mod permission;
mod search;
mod status;

/// One tool: its name, its one-sentence description, its argument schema and when it is offered.
pub(crate) struct ToolDef {
    /// The name the agent calls (`mcp__htui__<name>`).
    pub(crate) name: &'static str,
    /// One sentence the agent reads.
    pub(crate) description: &'static str,
    /// The argument object's JSON Schema (schemars, `$schema` key removed).
    pub(crate) schema: fn() -> Value,
    /// Whether a session with this scope on this host is offered the tool.
    pub(crate) advertised: fn(&ToolScope, &HostCaps) -> bool,
}

/// What the host can do beyond the store, for `advertised`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HostCaps {
    /// A concept index is attached (D12): `search_concepts` can be offered.
    pub(crate) search: bool,
}

/// Table order is `tools/list` order and a pin.
pub(crate) const ALL: [&ToolDef; 8] = [
    &box_profile::DEF,
    &document::DEF,
    &note::DEF,
    &status::DEF,
    &link::DEF,
    &search::DEF,
    &command::DEF,
    &permission::DEF,
];

/// What a call sees: its session (scope, store, fence), the host it reads through, and the
/// client's progress token when it sent one.
pub(crate) struct Ctx<'a, H: htui_core::store::WorkerHost> {
    /// The session the token named.
    pub(crate) session: &'a Session<H::Store>,
    /// The host the session was opened on: reads (`box_profile`) go through it.
    pub(crate) host: H,
    /// `Some` when the client asked for progress (B-11).
    pub(crate) progress: Option<Progress>,
}

/// One-line reason; becomes `isError: true` (D2, "Tools").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolError(pub(crate) String);

/// `Ok` becomes the result's text as compact JSON.
pub(crate) type ToolResult = Result<Value, ToolError>;

/// Runs the tool `name`. The caller has already checked that it is advertised.
pub(crate) async fn dispatch<H: htui_core::store::WorkerHost>(
    name: &str,
    ctx: Ctx<'_, H>,
    args: Value,
) -> ToolResult {
    match name {
        "box_profile" => box_profile::call(ctx, args).await,
        "document_write" => document::call(ctx, args).await,
        "note_add" => note::call(ctx, args).await,
        "item_status" => status::call(ctx, args).await,
        "item_link" => link::call(ctx, args).await,
        "search_concepts" => search::call(ctx, args).await,
        "command_run" => command::call(ctx, args).await,
        "permission_prompt" => permission::call(ctx, args).await,
        other => Err(ToolError(format!("unknown tool: {other}"))),
    }
}

/// Decodes a tool's arguments; a failure is `invalid arguments: <serde message>`.
pub(crate) fn args<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ToolError> {
    serde_json::from_value(v).map_err(|err| ToolError(format!("invalid arguments: {err}")))
}

/// The JSON Schema of `T`, without the `$schema` key the agent does not need.
pub(crate) fn schema_of<T: schemars::JsonSchema>() -> Value {
    let mut schema = schemars::schema_for!(T).to_value();
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
    }
    schema
}

/// `text` masked by the session's scrubber, or refused when a credential survives (I-5, fail
/// closed). The refusal names the rule, never the text.
pub(crate) fn scrubbed(s: &dyn Scrubber, text: String) -> Result<String, ToolError> {
    let mut value = Value::String(text);
    s.scrub(&mut value).map_err(|unmasked| {
        ToolError(format!(
            "refused: the text matched credential rule {}; nothing was written",
            unmasked.rule
        ))
    })?;
    match value {
        Value::String(text) => Ok(text),
        _ => Err(ToolError(
            "refused: the scrubber changed the text's type; nothing was written".to_owned(),
        )),
    }
}

/// A store failure as the agent reads it.
pub(crate) fn store_error(err: StoreError) -> ToolError {
    ToolError(match err {
        StoreError::Fenced { .. } => "fenced: lease lost".to_owned(),
        StoreError::NotFound { entity, id } => format!("not found: {entity} {id}"),
        StoreError::Constraint(sentence) => format!("refused: {sentence}"),
        StoreError::Unreachable(message) | StoreError::Backend(message) => {
            format!("store unavailable: {message}")
        }
        other @ (StoreError::ReadOnly(_) | StoreError::ParseEnum(_)) => {
            format!("store unavailable: {other}")
        }
    })
}

#[cfg(test)]
mod tests {
    use htui_core::model::StepId;
    use htui_core::scrub::MinimalScrubber;
    use htui_core::store::StoreError;

    use super::{ALL, ToolError, scrubbed, store_error};

    #[test]
    fn the_table_is_the_pinned_order() {
        let names: Vec<&str> = ALL.iter().map(|def| def.name).collect();
        assert_eq!(
            names,
            [
                "box_profile",
                "document_write",
                "note_add",
                "item_status",
                "item_link",
                "search_concepts",
                "command_run",
                "permission_prompt",
            ]
        );
    }

    #[test]
    fn store_errors_read_as_one_line_reasons() {
        assert_eq!(
            store_error(StoreError::Fenced {
                step: StepId::new()
            }),
            ToolError("fenced: lease lost".to_owned())
        );
        assert_eq!(
            store_error(StoreError::NotFound {
                entity: "box",
                id: "b-1".to_owned()
            }),
            ToolError("not found: box b-1".to_owned())
        );
        assert_eq!(
            store_error(StoreError::Constraint("no".to_owned())),
            ToolError("refused: no".to_owned())
        );
        assert_eq!(
            store_error(StoreError::Unreachable("down".to_owned())),
            ToolError("store unavailable: down".to_owned())
        );
    }

    #[test]
    fn a_surviving_credential_is_refused_by_rule_never_by_text() {
        let scrubber = MinimalScrubber::new(["hunter2-secret".to_owned()]);
        assert_eq!(
            scrubbed(&scrubber, "the hunter2-secret value".to_owned()),
            Ok("the [REDACTED] value".to_owned())
        );
        let leaked = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let refused = scrubbed(&scrubber, format!("key {leaked}")).expect_err("fails closed");
        assert!(
            refused
                .0
                .starts_with("refused: the text matched credential rule "),
            "{refused:?}"
        );
        assert!(!refused.0.contains(leaked));
    }
}
