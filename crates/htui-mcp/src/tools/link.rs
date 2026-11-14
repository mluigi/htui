//! `item_link` (MOD-11 M2, plan "Tools", blueprint §7.2): proposes or withdraws a link from the
//! step's item.
//!
//! `from` is always the run's item and `to` an item **key** ([`item_by_key`], I-1): `KEY` in the
//! scope's project, `slug:KEY` in another project of its workspaces, or `@workspace:KEY` for a
//! workspace's own item (MOD-147 D2). A key no project has, or an item outside the reach of D3
//! (this project, its workspaces and their members), is `out of scope`. The answer names `to`
//! canonically: bare in this project, `slug:KEY` elsewhere. `add` upserts through
//! [`propose_link`], `remove` tombstones through [`withdraw_link`], both under the session's fence
//! (I-3). A run withdraws only what one of its steps proposed (PRD OQ-4, B-6): anything else
//! reads `not yours`, and so does a `remove` of another project's item with no live link (edge 9:
//! the store's `not found` would print an id outside the reach), as `is no live link this run
//! proposed`, which stays true when the link was this run's and is already withdrawn.
//!
//! MOD-150 (plan D9): `kind` is any [`LinkKind`], `parent`, `duplicates` and `implements`
//! included, and each goes live at once (pending links are MOD-155's). The store refuses a second
//! live `parent` or `duplicates`, a `parent` that closes a cycle and one that makes a chain deeper
//! than eight. Those refusals reach the agent unchanged as `refused: <sentence>` through
//! `store_error`, which names the item's current parent or canonical.
//!
//! MOD-150 M3 (plan D11): instead of `to`, a call may name `to_document`, a document by owner (an
//! item key, else a project slug), kind and optional version (the latest at write when omitted).
//! Exactly one of the two is named. A document link is `origin`, `relates` or `implements`; it
//! goes through [`propose_document_link`] and [`withdraw_document_link`], and the answer names
//! the pinned version and the latest. The owner reads canonically, as `to` does: bare in this
//! project, `slug:KEY` elsewhere, the project slug for a project's own document.
//!
//! [`item_by_key`]: WorkerStore::item_by_key
//! [`propose_link`]: WorkerStore::propose_link
//! [`withdraw_link`]: WorkerStore::withdraw_link
//! [`propose_document_link`]: WorkerStore::propose_document_link
//! [`withdraw_document_link`]: WorkerStore::withdraw_document_link

use htui_core::model::link::{ProposeLink, WithdrawLink};
use htui_core::model::{
    ArtifactLinkKind, DocumentRef, ItemId, LinkKind, LinkedDocument, ProjectId,
    ProposeDocumentLink, WithdrawDocumentLink, split_qualified,
};
use htui_core::store::traits::{
    document_already_pinned, document_outside_reach, document_ref_label, link_key,
    link_not_proposed_by_run, link_outside_reach,
};
use htui_core::store::{StoreError, WorkerHost, WorkerStore};
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

// `from` is the scope's item and `to` a key (I-1): `KEY` of the scope's project, `slug:KEY` of
// another project of its workspaces or `@workspace:KEY` of a workspace's own item (MOD-147 D2); a
// `run_id`, an `item_id` or a `from` is refused. MOD-150 M3 D11: `to_document` names a document
// instead; exactly one of the two is named. A plain comment, not a doc: schemars would hand a doc
// to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkArgs {
    /// `add` proposes the link, `remove` withdraws one this run proposed.
    #[schemars(schema_with = "op_schema")]
    op: Op,
    // An attribute, not a doc: a doc wrapped at 100 columns reaches the agent with a line break.
    #[schemars(
        description = "The other item's key: `FEAT-12` in this project, `api:FEAT-12` in project \
                       `api`, or `@shop:WS-1` for workspace `shop`'s own item (or use \
                       `to_document`)."
    )]
    #[serde(default)]
    to: Option<String>,
    // An attribute, not a doc: a doc wrapped at 100 columns reaches the agent with a line break.
    #[schemars(
        description = "Instead of `to`, a document: its owner, its kind and optionally the \
                       version; `kind` is then `origin`, `relates` or `implements`."
    )]
    #[serde(default)]
    to_document: Option<DocumentArg>,
    // An attribute, not a doc: a doc wrapped at 100 columns reaches the agent with a line break.
    #[schemars(
        description = "How this item relates to `to`, read `<this item> <kind> <to>`: \
                       `blocked_by`: this item cannot start until `to` is finished; `origin`: \
                       this item was spawned by `to`; `relates`: see also; `supersedes`: this \
                       item replaces `to`; `parent`: this item is a part of `to`; `duplicates`: \
                       this item is the same as `to`; `implements`: this item carries out `to`'s \
                       decision. An item has at most one `parent` and one `duplicates`."
    )]
    #[schemars(schema_with = "kind_schema")]
    kind: LinkKind,
}

// MOD-150 M3 D11: how the agent names a document, inlined so the schema carries no `$ref`. A
// plain comment, not a doc: schemars would hand a doc to the agent as the schema's description.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(inline)]
struct DocumentArg {
    #[schemars(
        description = "The document's owner: an item key (`ANA-35`, `api:FEAT-12`, \
                       `@shop:WS-1`) for an item's document, else a project slug (`api`, `@shop` \
                       for workspace `shop`) for a project's own document."
    )]
    owner: String,
    #[schemars(description = "The document's kind, such as `plan` or `verdict`.")]
    kind: String,
    #[schemars(description = "The version to link; the latest when omitted.")]
    #[serde(default)]
    version: Option<i32>,
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
                  project or its workspaces, named by key: FEAT-12 here, api:FEAT-12 in project \
                  api, @shop:WS-1 for workspace shop's own items.",
    schema: schema_of::<LinkArgs>,
    advertised: |scope, _| scope.item_id.is_some(),
};

/// `to` names no item this session reaches (MOD-147 D3), in the agent's own text.
fn out_of_scope(to: &str) -> ToolError {
    ToolError(format!(
        "out of scope: {to} is not an item of this project or its workspaces"
    ))
}

/// `{"from": "<key>", "to": "<key>", "kind": "<kind>", "live": bool}`; `to` is canonical: bare in
/// this project, `slug:KEY` elsewhere. With `to_document` (MOD-150 M3 D11): `{"from": "<key>",
/// "to_document": {"owner", "kind", "version", "latest"}, "kind", "live"}`.
pub(crate) async fn call<H: WorkerHost>(ctx: Ctx<'_, H>, arguments: Value) -> ToolResult {
    let LinkArgs {
        op,
        to,
        to_document,
        kind,
    } = args(arguments)?;
    let Some(from) = ctx.session.scope.item_id else {
        return Err(ToolError(
            "out of scope: this session has no item".to_owned(),
        ));
    };
    match (to, to_document) {
        (Some(to), None) => item_call(ctx, from, op, to, kind).await,
        (None, Some(document)) => {
            let kind = ArtifactLinkKind::from_link_kind(kind).ok_or_else(|| {
                ToolError(
                    "refused: a link to a document is origin, relates or implements".to_owned(),
                )
            })?;
            let to = DocumentRef {
                owner: document.owner,
                kind: document.kind,
                version: document.version,
            };
            document_call(ctx, from, op, to, kind).await
        }
        _ => Err(ToolError(
            "invalid arguments: name `to` or `to_document`".to_owned(),
        )),
    }
}

/// `from`'s bare key.
async fn item_key<S: WorkerStore>(store: &S, from: ItemId) -> Result<String, ToolError> {
    Ok(WorkerStore::item(store, from)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ToolError(format!("not found: item {from}")))?
        .key)
}

/// The link to the item keyed `to`.
async fn item_call<H: WorkerHost>(
    ctx: Ctx<'_, H>,
    from: ItemId,
    op: Op,
    to: String,
    kind: LinkKind,
) -> ToolResult {
    let session = ctx.session;
    let scope = &session.scope;
    let store = &session.store;
    let target = WorkerStore::item_by_key(store, scope.project_id, &to)
        .await
        .map_err(store_error)?
        .ok_or_else(|| out_of_scope(&to))?;
    let target_item = WorkerStore::item(store, target)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ToolError(format!("not found: item {target}")))?;
    let elsewhere = target_item.project_id != scope.project_id;
    let canonical = if elsewhere {
        let project = WorkerStore::project(store, target_item.project_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| ToolError(format!("not found: project {}", target_item.project_id)))?;
        format!("{}:{}", project.slug, target_item.key)
    } else {
        target_item.key
    };
    let from_key = item_key(store, from).await?;
    let written = match op {
        Op::Add => {
            let link = ProposeLink {
                from,
                to: target,
                kind,
                step: scope.step_id,
            };
            WorkerStore::propose_link(store, scope.fence, link).await
        }
        Op::Remove => {
            let link = WithdrawLink {
                from,
                to: target,
                kind,
                step: scope.step_id,
            };
            WorkerStore::withdraw_link(store, scope.fence, link).await
        }
    };
    let not_yours = || {
        ToolError(format!(
            "not yours: {from_key} {kind} {canonical} was not proposed by this run"
        ))
    };
    let link = written.map_err(|err| match err {
        StoreError::Constraint(sentence)
            if sentence == link_not_proposed_by_run(&link_key(from, target, kind)) =>
        {
            not_yours()
        }
        // MOD-147 edge 9: `withdraw_link` has no reach check, so another project's item with no
        // live link reads `not yours` (this run has nothing live there to withdraw). The store's
        // sentence would print both ids and name an item outside the reach. `WorkerStore` has no
        // workspace read to tell an out-of-reach target from a co-member's, so this covers both,
        // worded to hold when the link was this run's and is already withdrawn.
        StoreError::NotFound {
            entity: "item_link",
            ..
        } if elsewhere => ToolError(format!(
            "not yours: {from_key} {kind} {canonical} is no live link this run proposed"
        )),
        StoreError::Constraint(sentence) if sentence == link_outside_reach(target) => {
            out_of_scope(&to)
        }
        other => store_error(other),
    })?;
    Ok(json!({
        "from": from_key,
        "to": canonical,
        "kind": link.kind,
        "live": link.deleted_at.is_none(),
    }))
}

/// The document's owner as the agent reads it (D11): the bare key of an item of `project`,
/// `slug:KEY` of an item elsewhere, the project slug for a project's own document.
fn canonical_owner(document: &LinkedDocument, project: ProjectId) -> String {
    match &document.owner_key {
        Some(key) if document.project_id == project => key.clone(),
        Some(key) => format!("{}:{key}", document.project_slug),
        None => document.project_slug.clone(),
    }
}

/// Whether the agent's `owner` text names `document`'s owner, by D3's rules: a bare key in
/// `project`, a qualified key in its slug's project, else a project slug.
fn names_owner(owner: &str, document: &LinkedDocument, project: ProjectId) -> bool {
    let (slug, key) = split_qualified(owner);
    match (&document.owner_key, slug) {
        (Some(owner_key), None) => owner_key == key && document.project_id == project,
        (Some(owner_key), Some(slug)) => owner_key == key && document.project_slug == slug,
        (None, _) => document.project_slug == owner,
    }
}

/// The link to the document `to` (MOD-150 M3 D11).
async fn document_call<H: WorkerHost>(
    ctx: Ctx<'_, H>,
    from: ItemId,
    op: Op,
    to: DocumentRef,
    kind: ArtifactLinkKind,
) -> ToolResult {
    let session = ctx.session;
    let scope = &session.scope;
    let store = &session.store;
    let from_key = item_key(store, from).await?;
    let label = document_ref_label(&to);
    let written = match op {
        Op::Add => {
            let link = ProposeDocumentLink {
                from,
                kind,
                to: to.clone(),
                step: scope.step_id,
            };
            WorkerStore::propose_document_link(store, scope.fence, link).await
        }
        Op::Remove => {
            let link = WithdrawDocumentLink {
                from,
                kind,
                to: to.clone(),
                step: scope.step_id,
            };
            WorkerStore::withdraw_document_link(store, scope.fence, link).await
        }
    };
    let written = match written {
        Ok(written) => written,
        Err(StoreError::Constraint(sentence)) if sentence == document_outside_reach(&label) => {
            return Err(ToolError(format!(
                "out of scope: document {label} is outside this project and its workspaces"
            )));
        }
        Err(StoreError::Constraint(sentence)) if sentence == link_not_proposed_by_run(&label) => {
            return Err(ToolError(format!(
                "not yours: {from_key} {kind} {label} was not proposed by this run"
            )));
        }
        Err(StoreError::NotFound {
            entity: "document", ..
        }) => return Err(ToolError(format!("not found: document {label}"))),
        Err(StoreError::Constraint(sentence)) => {
            // D4: the pin names the version only; the live link it names is read back for the
            // owner and kind the agent knows.
            let pinned = WorkerStore::links(store, from, 0)
                .await
                .map_err(store_error)?
                .documents
                .into_iter()
                .find(|doc| {
                    doc.from_item_id == from
                        && doc.kind == kind
                        && doc.doc_kind == to.kind
                        && names_owner(&to.owner, doc, scope.project_id)
                        && sentence == document_already_pinned(from, kind, doc.version)
                });
            return Err(match pinned {
                Some(doc) => ToolError(format!(
                    "refused: {from_key} already {kind} {} {} v{}; remove it before linking \
                     another version",
                    canonical_owner(&doc, scope.project_id),
                    doc.doc_kind,
                    doc.version
                )),
                None => store_error(StoreError::Constraint(sentence)),
            });
        }
        Err(other) => return Err(store_error(other)),
    };
    let document = &written.document;
    Ok(json!({
        "from": from_key,
        "to_document": {
            "owner": canonical_owner(document, scope.project_id),
            "kind": document.doc_kind,
            "version": document.version,
            "latest": document.latest_version,
        },
        "kind": written.link.kind,
        "live": written.link.deleted_at.is_none(),
    }))
}
