//! `item_link` (MOD-11 M2, plan "Tools", blueprint §7.2): proposes or withdraws a link from the
//! step's item.
//!
//! `from` is always the run's item and `to` an item **key** resolved inside the scope's project
//! only ([`item_by_key`]; a key the project lacks is `out of scope`, I-1). `add` upserts through
//! [`propose_link`], `remove` tombstones through [`withdraw_link`], both under the session's fence
//! (I-3). A run withdraws only what one of its steps proposed (PRD OQ-4, B-6): anything else
//! reads `not yours`.
//!
//! [`item_by_key`]: htui_core::store::WorkerStore::item_by_key
//! [`propose_link`]: htui_core::store::WorkerStore::propose_link
//! [`withdraw_link`]: htui_core::store::WorkerStore::withdraw_link

use htui_core::model::LinkKind;
use htui_core::model::link::{ProposeLink, WithdrawLink};
use htui_core::store::StoreError;
use htui_core::store::traits::{link_key, link_not_proposed_by_run};
use serde_json::{Value, json};

use super::{Ctx, ToolDef, ToolError, ToolResult, args, schema_of, store_error};

/// What `item_link` does with the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    /// Propose the link (or revive a tombstoned one).
    Add,
    /// Withdraw a link this run proposed.
    Remove,
}

// `from` is the scope's item and `to` a key of the scope's project (I-1): a `run_id`, an
// `item_id` or a `from` is refused. A plain comment, not a doc: schemars would hand a doc to the
// agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkArgs {
    /// `add` proposes the link, `remove` withdraws one this run proposed.
    #[schemars(schema_with = "op_schema")]
    op: Op,
    /// The other item's key in this project, e.g. `FEAT-12`.
    to: String,
    /// Read `<this item> <kind> <to>`: `blocked_by`, `origin`, `relates` or `supersedes`.
    #[schemars(schema_with = "kind_schema")]
    kind: LinkKind,
}

/// `{"type": "string", "enum": ["add", "remove"]}`.
fn op_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({"type": "string", "enum": ["add", "remove"]})
}

/// `{"type": "string", "enum": [<every link kind>]}`.
fn kind_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let names: Vec<&str> = LinkKind::ALL.iter().map(|kind| kind.as_str()).collect();
    schemars::json_schema!({"type": "string", "enum": names})
}

/// Advertised when the scope has an item.
pub(crate) const DEF: ToolDef = ToolDef {
    name: "item_link",
    description: "Proposes or withdraws a link from this step's item to another item of its \
                  project, named by key.",
    schema: schema_of::<LinkArgs>,
    advertised: |scope, _| scope.item_id.is_some(),
};

/// `{"from": "<key>", "to": "<key>", "kind": "<kind>", "live": bool}`.
pub(crate) async fn call<H: htui_core::store::WorkerHost>(
    ctx: Ctx<'_, H>,
    arguments: Value,
) -> ToolResult {
    let LinkArgs { op, to, kind } = args(arguments)?;
    let session = ctx.session;
    let scope = &session.scope;
    let store = &session.store;
    let Some(from) = scope.item_id else {
        return Err(ToolError(
            "out of scope: this session has no item".to_owned(),
        ));
    };
    let target = htui_core::store::WorkerStore::item_by_key(store, scope.project_id, &to)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ToolError(format!("out of scope: {to} is not an item of this project")))?;
    let from_key = htui_core::store::WorkerStore::item(store, from)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ToolError(format!("not found: item {from}")))?
        .key;
    let written = match op {
        Op::Add => {
            let link = ProposeLink {
                from,
                to: target,
                kind,
                step: scope.step_id,
            };
            htui_core::store::WorkerStore::propose_link(store, scope.fence, link).await
        }
        Op::Remove => {
            let link = WithdrawLink {
                from,
                to: target,
                kind,
                step: scope.step_id,
            };
            htui_core::store::WorkerStore::withdraw_link(store, scope.fence, link).await
        }
    };
    let link = written.map_err(|err| match err {
        StoreError::Constraint(sentence)
            if sentence == link_not_proposed_by_run(&link_key(from, target, kind)) =>
        {
            ToolError(format!(
                "not yours: {from_key} {kind} {to} was not proposed by this run"
            ))
        }
        other => store_error(other),
    })?;
    Ok(json!({
        "from": from_key,
        "to": to,
        "kind": link.kind,
        "live": link.deleted_at.is_none(),
    }))
}
