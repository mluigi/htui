//! `search_concepts` (MOD-11 M3, plan "Tools", blueprint §2.10, §10): the concept index, scoped to
//! the session's project.
//!
//! The project is the scope's and never an argument (I-1): `projects` is always
//! `[scope.project_id]`. `types` are parsed here with [`ConceptType::parse`] (`htui-store`'s
//! `PointType::parse` is private, B-3); `limit` is 1..=20, ten when omitted. Advertised only when
//! the host has a concept index (D12); an index that cannot answer is `isError` with its cause
//! (R-STO-8). A hit is what the index carries, with no title or status lookups.

use htui_core::model::Status;
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of};
use crate::search::{ConceptQuery, ConceptType};

/// The most hits one call may ask for (plan "Tools": `limit? ≤ 20`).
const MAX_LIMIT: u64 = 20;

/// The hits a call gets when it names no limit.
const DEFAULT_LIMIT: u64 = 10;

// The project is the scope's (I-1): a `project` or a `run_id` is refused. `types` are strings,
// parsed by hand so an unknown one names itself; `statuses` decode through their `str_enum!` serde
// names. A plain comment, not a doc: schemars would hand a doc to the agent as the schema's
// description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// What to look for, in words; an exact key such as `HTUI-12` matches too.
    query: String,
    /// Point types to keep; every type when omitted.
    #[serde(default)]
    #[schemars(schema_with = "types_schema")]
    types: Option<Vec<String>>,
    /// Item statuses to keep; every status when omitted; any status leaves requirements out.
    #[serde(default)]
    #[schemars(schema_with = "statuses_schema")]
    statuses: Option<Vec<Status>>,
    /// At most this many hits, 1 to 20; 10 when omitted.
    #[serde(default)]
    #[schemars(range(min = 1, max = 20))]
    limit: Option<u64>,
}

/// `{"type": ["array", "null"], "items": {"type": "string", "enum": [<every type>]}}`.
fn types_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let names: Vec<&str> = ConceptType::ALL.iter().map(|t| t.as_str()).collect();
    schemars::json_schema!({
        "type": ["array", "null"],
        "items": {"type": "string", "enum": names}
    })
}

/// `{"type": ["array", "null"], "items": {"type": "string", "enum": [<every status>]}}`.
fn statuses_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let names: Vec<&str> = Status::ALL.iter().map(|status| status.as_str()).collect();
    schemars::json_schema!({
        "type": ["array", "null"],
        "items": {"type": "string", "enum": names}
    })
}

/// Advertised when the host has a concept index (D12).
pub(crate) const DEF: ToolDef = ToolDef {
    name: "search_concepts",
    description: "Searches this project's items, documents and requirements by meaning.",
    schema: schema_of::<SearchArgs>,
    advertised: |_, caps| caps.search,
};

/// `{"hits": [ConceptHit…]}`, best first.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let SearchArgs {
        query,
        types,
        statuses,
        limit,
    } = args(arguments)?;
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ToolError(format!(
            "invalid arguments: limit {limit} is outside 1..={MAX_LIMIT}"
        )));
    }
    let types = types
        .unwrap_or_default()
        .iter()
        .map(|name| {
            ConceptType::parse(name)
                .ok_or_else(|| ToolError(format!("invalid arguments: unknown type {name}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let Some(index) = ctx.session.search.as_ref() else {
        return Err(ToolError(
            "search unavailable: no concept index is attached".to_owned(),
        ));
    };
    let hits = index
        .search(ConceptQuery {
            text: query,
            project: ctx.session.scope.project_id,
            types,
            statuses: statuses.unwrap_or_default(),
            limit,
        })
        .await
        .map_err(|cause| ToolError(format!("search unavailable: {cause}")))?;
    Ok(json!({ "hits": hits }))
}
