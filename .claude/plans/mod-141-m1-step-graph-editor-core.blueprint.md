# Blueprint: MOD-141 M1 - Step-graph editor core in Settings > Kinds

**Plan**: `.claude/plans/mod-141-m1-step-graph-editor-core.plan.md` (D1-D14, tasks T0-T8; settled).
**PRD**: `.claude/prds/mod-141-step-graph-editor.prd.md`.
**Branch**: `hr/MOD-141`. Wave 1 worktrees branch from the T0 commit.

Line numbers below are `hr/MOD-141` at `281cc89d`.

---

## Plan corrections

Each one was checked against the code. The blueprint below builds the corrected version.

| # | Plan claim | What the code says | Correction |
|---|---|---|---|
| C1 | D14/T7: "`DeletePhase` from Kinds go(es)" | `kinds.rs` never sends `StoreRequest::DeletePhase`. The only senders are `catalogue.rs:385`, `store_worker.rs:788/1414/2410` and `tests/kinds.rs:159,1103`. `d` on a graph or phase row answers `NOT_DELETED_HERE` (`kinds.rs:140,739`). | There is nothing to retire. T7 changes the phase-row refusal into a pointer to the editor. `StoreRequest::DeletePhase`, `CreatePhase`, `UpdatePhase` and `SetPhaseBudget` stay in the worker, unused by the UI. |
| C2 | T1 file list covers what `REQUEST_NAMES` 12 moves | `kinds.rs:2228` `a_write_in_flight_reads_in_words` loops over `REQUEST_NAMES` and requires `doing(name)` to have its own phrase. | T1 adds `"graph_body" => "reading the graph"` to `kinds.rs::doing` (`kinds.rs:2049`). |
| C3 | D6: "the confirm question for delete says so when the node has bindings" | The editor cannot know about bindings. `CatalogueSnapshot`, `GraphEntry` and `GraphBody` carry no `skill_binding` rows (`catalogue.rs:36-73`, `kind.rs:414-435`). | The delete question always states the limit (wording in T6). |
| C4 | T6 "remove the `dead_code` allow" | `GraphEditor`'s only caller is Kinds, which T7 writes. A `pub(crate)` type with no caller fails `cargo clippy --workspace -- -D warnings` (`scripts/gate.sh:84`). | `GraphEditor` and `EditorKey` are **`pub`** with documented `pub` methods. This is D1's Toolbox entry point, and a `pub` item of a `pub mod` is never dead. The wave-1 modules stay `pub(crate)`. T6 removes the allow. Every `pub` signature names only `pub` types (`private_interfaces`). |
| C5 | D11 mouse: plan files list has no Settings shell change | `SettingsSection` has no mouse methods (`settings/mod.rs:142-206`). `SettingsTab` implements neither `Tab::wants_mouse` nor `Tab::on_mouse` (defaults `false`/`Pass`, `registry.rs:135-149`). The shell hands the mouse only to a tab whose `wants_mouse` is true (`app/state.rs:1305`). | T7 adds defaulted `SettingsSection::{wants_mouse, on_mouse, on_mouse_lost}` plus `SettingsTab` forwarding. `crates/htui/src/ui/tabs/settings/mod.rs` joins T7's file set. The keyboard path is complete without it. |
| C6 | T2: "a delete whose rewire the walk refuses" | Rewiring every forward in-edge of X onto every forward out-target of X, keeping its kind, preserves forward reachability between the remaining nodes. Every W2/W3/W5 failure the rewire could cause needs a forward cycle in the body before the delete. That cycle is already a W1 refusal. We found no valid body whose delete the walk refuses. | T2 tests the property instead: deleting any node of each seeded graph and of the reference graph gives a body `validate` accepts. The refusal path ("refused body: sentence shown, nothing sent") is tested in T6 with a body that is already refused. |
| C7 | D12: `i` links in context `graph_editor` | `no_act_name_is_a_nested_table` (`catalogue.rs:3427`): an act named `links` in `[graph_editor]` would also be the table `[graph_editor.links]`. | The act is named `open_links`. |
| C8 | Plan's T6 key files | `every_act_is_offered_by_a_stack_and_handled_by_a_view` and `every_view_stack_is_named_by_a_view` (`views.rs:2124-2230`) scan the production source of the files in `VIEWS` (`views.rs:1173`). Each stack must be named `views::X` there, and each act it offers must be named `Act::X` in the same file. | `graph_editor/mod.rs` joins `VIEWS`. All `Act::` handling sits in `mod.rs`, so the wave-1 modules stay key-agnostic. The capturing-stack pin (`views.rs:1905-1933`) gains two names. |
| C9 | T6/T7 both add acts; T6 lands all the keys | `Act::KindsEditor` must be named in `kinds.rs` production code (C8), and T6 does not touch `kinds.rs`. | T6 adds 22 acts and T7 adds `settings.kinds.editor`. Pins: CATALOGUE 222 → 244 (T6) → 245 (T7). |
| C10 | T0 "skeleton only" | Wave-1 modules must not depend on each other: each worktree compiles alone. | T0's `mod.rs` holds the shared types and helpers (`Lookups`, `EdgeSpec`, `RELATIONS`, `relation_word`, `forward_edges`, `step_name`) as full code. Every wave-1 task imports only these and `htui-core`. |

Pins verified at `281cc89d`:

- `Context::ALL.len() == 52` (`catalogue.rs:2488`)
- `CATALOGUE.len() == 222` (`catalogue.rs:3011`)
- view-led stacks `== 84` (`views.rs:2012`)
- `DECLARED.len() == 91` (`stack.rs:439`)
- last act position `MemoryClashReplace => 221`

---

## Shared conventions (every task)

- **Module**: `crates/htui/src/ui/graph_editor/`. Submodules are `pub(crate)`. Only `GraphEditor` and `EditorKey` (T6) are `pub`.
- **Errors in words**: every refusal is a `String` sentence, shown on the editor's notice row. No panic and no `unwrap` on data, the `kinds.rs:66-147` habit.
- **Debug**:
  - Hand-written `Debug` wherever a struct holds a `TextField` or typed text. It prints labels and ids, never typed text (H-7, `kinds.rs:280-300`).
  - `missing_debug_implementations` is warn on `pub` types (`Cargo.toml:195`).
- **Docs**:
  - `#![warn(missing_docs)]` in `lib.rs:10`: every `pub` item gets a doc comment.
  - `rustdoc::private_intra_doc_links` is deny: doc links from a `pub` item may point only at `pub` items.
- **No inter-module calls in wave 1**:
  - `model`, `history`, `canvas`, `node_form`, `edge_form` and `links` each use only `super::{Lookups, EdgeSpec, RELATIONS, relation_word, forward_edges, step_name}`, `htui_core`, `crate::catalogue::PersonaSummary` and `crate::ui::{kit, cells, TextField, FieldOutcome, Theme}`.
  - They never name `Act`, `Keys`, `Stack`, `Ctx` or `StoreRequest`. That is T6's job.
- **Snapshots**: unit snapshots land under `crates/htui/src/ui/graph_editor/snapshots/` as `htui__ui__graph_editor__<module>__tests__<name>.snap`. Names are disjoint per task.
- **Tests**:
  - TDD: write the listed tests first and watch them fail, then implement.
  - Every command runs with `--test-threads=1` (memory: suite green is scheduling-dependent).
  - `tests/*.rs` needs `--features testkit`, or it runs 0 tests and reports ok.
- **Per-task lint gate** (run before every commit):
  ```bash
  cargo fmt --all -- --check
  cargo clippy -p htui -- -D warnings
  cargo clippy -p htui --all-features -- -D warnings
  ```
- **Worktrees** (memory: workflow worktree implementers):
  - After T0 is committed on `hr/MOD-141`, run `git worktree add ../htui-m141-tN -b mod-141-tN hr/MOD-141` for N = 1..5.
  - Commit incrementally. Never stash.
  - Merge serially into `hr/MOD-141` in the order T1, T2, T3, T4, T5, and re-run that task's validation on the merged tree after each merge.
  - Remove each worktree before `git branch -d`.
  - Disk at blueprint time: 2.5 TB free on the repo disk.

---

## T0 - Skeleton (serial, first)

**Files**: `crates/htui/src/ui/mod.rs` (one line) plus seven new files.

**`crates/htui/src/ui/mod.rs`**: add `pub mod graph_editor;` between `pub mod editor_pane;` and `pub mod header;`.

**`crates/htui/src/ui/graph_editor/mod.rs`** (literal):

```rust
//! MOD-141: the one step-graph editor (PRD `mod-141-step-graph-editor`, M1 plan D1-D14).
//!
//! It edits one [`GraphBody`] at a time, of any scope (project, workspace, library): nodes and
//! typed edges drawn with rataflow, a node form, an edge form, a links list, the store's own graph
//! rules run before every write (D5), and in-memory undo and redo where every applied edit is one
//! `StoreRequest::ReplaceGraph` compare-and-set on `step_graph.revision` (D4, D6). Nothing in here
//! names `Settings > Kinds`: the host hands it a body, the persona registry and every reply.
//!
//! - `model`: pure body edits and the client-side validator (D4, D5, D10).
//! - `history`: undo, redo and the one write in flight (D4, D6).
//! - `canvas`: the rataflow projection, layout, selection and mouse (D7, D11).
//! - `node_form`, `edge_form`, `links`: the forms and the links list (D8, D9, D10).
// Wave 1 (T1-T5) lands the submodules before anything calls them; T6 removes this allow.
#![allow(dead_code)]

pub(crate) mod canvas;
pub(crate) mod edge_form;
pub(crate) mod history;
pub(crate) mod links;
pub(crate) mod model;
pub(crate) mod node_form;

use std::collections::HashMap;

use htui_core::model::{
    AgentSummary, EdgeKind, EdgePredicate, GraphBody, PhaseId, PromptTemplate,
};
use htui_core::store::{WalkEdge, WalkNode, forward_mask};

use crate::catalogue::PersonaSummary;

/// What the host hands the editor beside the body (D1): the persona registry, always known (the
/// catalogue carries it), and the agents and the templates once their reads answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Lookups {
    /// The persona registry by name: the node form's persona choice and the validator's persona
    /// references (D5, D8).
    pub(crate) personas: Vec<PersonaSummary>,
    /// `StoreReply::Agents`, by `agent.name`; `None` until it arrived. While `None` the validator
    /// leaves the agent references to the store, and the candidate list cannot be edited.
    pub(crate) agents: Option<Vec<AgentSummary>>,
    /// The templates the graph's scope may use, every version; `None` until
    /// `StoreReply::Templates` arrived.
    pub(crate) templates: Option<Vec<PromptTemplate>>,
}

/// One link's editable part (D9): what the edge form yields and what `model` writes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdgeSpec {
    /// `step_graph_edge.kind`.
    pub(crate) kind: EdgeKind,
    /// `step_graph_edge.predicate`: `Some` exactly when `kind` is `if`.
    pub(crate) predicate: Option<EdgePredicate>,
    /// `step_graph_edge.loop_limit`: required on `changes_requested` and `fails`, optional on
    /// `asks_you`, `None` otherwise.
    pub(crate) loop_limit: Option<i32>,
}

/// The relations in picker order (D9).
pub(crate) const RELATIONS: [EdgeKind; 5] = [
    EdgeKind::Next,
    EdgeKind::If,
    EdgeKind::AsksYou,
    EdgeKind::ChangesRequested,
    EdgeKind::Fails,
];

/// A relation in words, never its wire name (MOD-83).
pub(crate) const fn relation_word(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Next => "next",
        EdgeKind::If => "if",
        EdgeKind::AsksYou => "asks you",
        EdgeKind::ChangesRequested => "changes requested",
        EdgeKind::Fails => "fails",
    }
}

/// Per edge of `body`, in `body.edges` order, whether it is forward: `forward_mask` over the body
/// exactly as `graph_body_walk_refusal` builds its walk, so the editor and the store cannot
/// disagree (D7). An edge naming a step the body lacks is `false`.
pub(crate) fn forward_edges(body: &GraphBody) -> Vec<bool> {
    let index: HashMap<PhaseId, usize> = body
        .nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.phase.id, at))
        .collect();
    let nodes: Vec<WalkNode<'_>> = body
        .nodes
        .iter()
        .map(|node| WalkNode {
            name: &node.phase.name,
            palette: node.phase.palette,
        })
        .collect();
    // Out of range rather than skipped, so the mask stays aligned with `body.edges`.
    let at = |id: PhaseId| index.get(&id).copied().unwrap_or(usize::MAX);
    let edges: Vec<WalkEdge> = body
        .edges
        .iter()
        .map(|edge| WalkEdge {
            from: at(edge.from),
            to: at(edge.to),
            kind: edge.kind,
        })
        .collect();
    forward_mask(&nodes, &edges)
}

/// The name of step `id` of `body`, or `None` when the body has no such step.
pub(crate) fn step_name(body: &GraphBody, id: PhaseId) -> Option<&str> {
    body.nodes
        .iter()
        .find(|node| node.phase.id == id)
        .map(|node| node.phase.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mask is the store's: a `next` is forward, a `changes_requested` never is (seeded bug
    /// graph: reproduce -> fix -> review, review -changes_requested-> fix).
    #[tokio::test]
    async fn forward_edges_is_the_stores_mask() {
        use htui_core::fixtures::ids;
        use htui_core::store::{MemStore, WriteStore};
        let body = MemStore::demo()
            .graph_body(ids::GRAPH_VULKAN_FIX)
            .await
            .expect("a memory read")
            .expect("the bug graph");
        let mask = forward_edges(&body);
        for (edge, forward) in body.edges.iter().zip(&mask) {
            assert_eq!(*forward, edge.kind == EdgeKind::Next, "{:?}", edge.kind);
        }
    }
}
```

(If `htui_core::fixtures` is not visible to `htui` lib tests in a featureless build, take the first graph of `crate::catalogue::snapshot(&MemStore::demo(), &scope)` the way `kinds.rs:2248` does.)

**Six submodule files** (literal; each one is only its module doc, and the wave task fills it):

`canvas.rs`:
```rust
//! MOD-141 D7, D11: a `GraphBody` drawn as a rataflow `Flow`. Forward edges (`forward_edges`) are
//! layered by longest path; back edges leave and enter by the sides (`changes_requested` and a
//! back `asks_you` on the right, `fails` on the left); palette nodes sit in a band below. The flow
//! keeps no selection of its own and never sees a key; the mouse reaches it only through
//! `Canvas::on_mouse`, and `FlowAction::Delete` is never routed to it.
```
`edge_form.rs`:
```rust
//! MOD-141 D9: the edge form - relation, the `if` predicate as one line of JSON parsed live with
//! `IfPredicate::parse`, and the loop limit where the relation takes one. Key-agnostic: the
//! editor maps its acts onto these methods.
```
`history.rs`:
```rust
//! MOD-141 D4, D6: undo and redo of whole labelled bodies (100 deep, redo cleared by a new edit)
//! and the one write in flight, with what `Applied`, `Stale` and `Failed` do to them. Pure: no
//! I/O, no keys.
```
`links.rs`:
```rust
//! MOD-141 D10: the links list - the selected step's edges out and in, one row each, with a
//! cursor. Key-agnostic.
```
`model.rs`:
```rust
//! MOD-141 D4, D5, D10: pure edits of a `GraphBody` (add after, delete with rewire, connect, edit
//! and remove a link, replace a node) and the client-side validator, the store's own rules in the
//! store's order. Every function takes a body and answers a new one or a sentence.
```
`node_form.rs`:
```rust
//! MOD-141 D8: the node form - every M1 node field, the ordered candidate agents, and the M2
//! fields shown read-only. Key-agnostic: the editor maps its acts onto these methods.
```

**Validate**:
```bash
cargo check -p htui --all-features
cargo clippy -p htui -- -D warnings
cargo test -p htui --lib ui::graph_editor -- --test-threads=1
```

**Commit**:
`chore(MOD-141): T0 graph_editor skeleton and shared types`

---

## T1 - Store worker body replies (D13)

**File set**:
- `crates/htui/src/store_worker.rs`
- `crates/htui/src/catalogue.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs` (reply routing and `doing` only)
- `crates/htui/tests/kinds.rs`

### Types

`store_worker.rs`, `StoreRequest`. Add it right after `DeletePhase` (`:788-793`), in `REQUEST_NAMES` order:
```rust
/// MOD-141 D13: one graph's whole body, any scope (project, workspace, library): how the graph
/// editor opens a graph by id (MOD-124's Toolbox hook). A read. Answers
/// [`StoreReply::GraphBody`].
GraphBody(StepGraphId),
```

`StoreReply`. Add these right after `KindDeleted` (`:1654-1659`):
```rust
/// MOD-141 D13: the answer to [`StoreRequest::ReplaceGraph`]. `outcome` is `replace_graph`'s
/// own: `Applied` with the body as stored, `Stale` with the body as it is now. `catalogue` is the
/// scope re-read after it, which `Settings > Kinds` keeps drawing from. A refusal is still
/// `Failed { request: "replace_graph" }`.
GraphReplaced {
    /// The write's outcome, the body boxed (clippy `large_enum_variant`).
    outcome: CasOutcome<Box<GraphBody>>,
    /// The scope's catalogue after the write, or after the miss.
    catalogue: Box<CatalogueSnapshot>,
},
/// MOD-141 D13: the answer to [`StoreRequest::GraphBody`]; `None` when no graph has the id.
GraphBody(Option<Box<GraphBody>>),
```
Variant and type share a name; that is legal, as `enum E { String(String) }` is. Import `CasOutcome` from `htui_core::store` if `store_worker.rs` does not already.

### Routing

- `StoreRequest::name` (`:1403-1414`): add `Self::GraphBody(..) => "graph_body",` after `delete_phase`, and change the comment "The eleven" to "The twelve".
- `try_serve` (`:2397-2410`): add `| StoreRequest::GraphBody(..)` to the catalogue or-arm and change the comment "eleven" to "twelve". This `match` has no wildcard, so it fails to compile until the arm exists.
- Doc comments to update:
  - `:656-659` ("the eleven catalogue requests ... MOD-137 D12's two" → "twelve ... then MOD-141 D13's `GraphBody`").
  - `:767-768` on `ReplaceGraph`: "`Stale` answers `CatalogueStale`" becomes "answers [`StoreReply::GraphReplaced`] (`Applied` or `Stale`), a refusal `Failed { request: "replace_graph" }`".

`catalogue.rs::serve`:
```rust
StoreRequest::ReplaceGraph { scope, graph, expected_revision, body } => {
    let outcome = writer
        .replace_graph(*graph, *expected_revision, (**body).clone())
        .await?;
    let catalogue = Box::new(snapshot(&writer, scope).await?);
    let outcome = match outcome {
        CasOutcome::Applied(body) => CasOutcome::Applied(Box::new(body)),
        CasOutcome::Stale(body) => CasOutcome::Stale(Box::new(body)),
    };
    Ok(StoreReply::GraphReplaced { outcome, catalogue })
}
// MOD-141 D13: any scope's graph by id, for the editor's open-by-id (the Toolbox hook).
StoreRequest::GraphBody(graph) => Ok(StoreReply::GraphBody(
    writer.graph_body(*graph).await?.map(Box::new),
)),
```
- `REQUEST_NAMES`: `[&str; 12]`, with `"graph_body"` appended after `"delete_phase"`. Update the doc ("The twelve request names ... MOD-15 M4's nine, MOD-137 D12's two, then MOD-141 D13's one").
- Fix the "eleven" wording in `serve`'s docs (`:207`, `:389-391`).
- Offline: `backend.writer()` is `None`, so `GraphBody` answers `Unreachable` like every catalogue request does. No code for that.

`kinds.rs`:
- `doing` (`:2049`): add `"graph_body" => "reading the graph",` (C2).
- `on_reply` (`:1597`): add after the `CatalogueStale` arm:
  ```rust
  // MOD-141 D13: `ReplaceGraph`'s answer carries the catalogue; read it as `Catalogue` /
  // `CatalogueStale` were. T7 routes it to an open graph editor first.
  StoreReply::GraphReplaced { outcome, catalogue } => match outcome {
      CasOutcome::Applied(_) => self.on_catalogue(catalogue, ctx),
      CasOutcome::Stale(_) => self.on_stale(catalogue, ctx.keys()),
  },
  ```
  (`use htui_core::store::CasOutcome;`.)
- `Failed { request: "graph_body" }` falls into the existing `REQUEST_NAMES.contains` arm, which is harmless: Kinds never sends it.

`StoreReply` has no other exhaustive match in the crate: `StoreReply::KindDeleted` appears only in `catalogue.rs:262` and `kinds.rs:1601`. Every other `on_reply` has a `_` arm.

### Tests in `tests/kinds.rs` (write first)

Changes:

| Test | Line | Change |
|---|---|---|
| `catalogue_requests` | `:85` | Append `StoreRequest::GraphBody(StepGraphId::default())`, and fix the doc ("twelve"). |
| `catalogue_names_are_stable` | `:169` | `REQUEST_NAMES.len() == 12`; `REQUEST_NAMES[9..] == ["replace_graph", "delete_phase", "graph_body"]`. |
| `offline_refuses_every_catalogue_request_by_name` | `:297` | Unchanged. It now covers `graph_body` by iterating. |
| `replace_graph_round_trips_through_the_catalogue` | `:915` | Uses a new helper `replaced(reply) -> (CasOutcome<Box<GraphBody>>, CatalogueSnapshot)` that panics on any other reply. Assert `Applied(body)`, `body.graph.revision == 1`, and `*body == *graph_entry(&catalogue, GRAPH_VULKAN_ANA)` (the body is the stored one). The existing name and edge assertions run on `catalogue`. |
| `replace_graph_from_the_catalogue_keeps_candidates_and_parts` | `:997` | Takes the catalogue through `replaced(..).1`. |
| `a_spent_revision_answers_catalogue_stale` | `:1031` | Renamed `a_spent_revision_answers_graph_replaced_stale`. First reply `Applied`. Second reply `GraphReplaced { outcome: Stale(current), catalogue }`, `current.graph.revision == 1`, and `catalogue == first's catalogue`. |
| `a_refused_replace_graph_answers_failed_by_name` | `:1052` | Unchanged (still `Failed`). |

New tests:

- `graph_body_reads_one_graph_of_each_scope`
  - Project: `GRAPH_VULKAN_ANA`.
  - Workspace: `writer.create_step_graph(NewStepGraph { id, scope: GraphScope::Workspace(ids::WORKSPACE_GRAPHICS), name: "ws-flow", description: "", is_override: false })`.
  - Library: the same with `GraphScope::Library`.
  - Each must answer `StoreReply::GraphBody(Some(body))` with `body.graph.id` and `.scope` as created.
  - `StepGraphId::new()` must answer `GraphBody(None)`.
- `replace_graph_on_a_library_graph_answers_its_body`:
  - Create a library graph and add one node through `ReplaceGraph` (body from `GraphBody`), with scope `vulkan_scope()`.
  - Expect `Applied(body)` with `body.graph.scope == GraphScope::Library` and `revision == 1`.
  - The catalogue does not list it. That is fine, because the editor reads the body.
- `a_graph_replaced_reply_redraws_the_tree_like_a_catalogue` (section, `bench_with_demo`):
  - Feed `GraphReplaced { Applied(..), catalogue }` where the catalogue has `verdict` renamed `decide`.
  - `render_section` shows `decide`, and no notice is set.
  - Then feed the `Stale` twin with no editor open. The notice must be `CHANGED_ELSEWHERE_CLOSED`, the twin of `a_stale_reply_with_no_editor_open_says_nothing_was_written`, `:2118`.
- The `kinds.rs` unit test `a_write_in_flight_reads_in_words` must pass, which needs C2.

**Validate**:
```bash
cargo test -p htui --features testkit --test kinds -- --test-threads=1
cargo test -p htui --lib ui::tabs::settings::kinds -- --test-threads=1
cargo test -p htui --lib store_worker -- --test-threads=1
```
Then the lint gate.

**Commits**:
1. `feat(MOD-141): T1 GraphReplaced and GraphBody replies, REQUEST_NAMES 12`: worker, catalogue, kinds routing and `doing`, plus the changed tests.
2. `test(MOD-141): T1 GraphBody of every scope, library replace, section reads GraphReplaced`.

---

## T2 - Body model and validator (D4, D5, D10)

**File set**: `crates/htui/src/ui/graph_editor/model.rs`.

### Interface (to T6)

```rust
use chrono::{DateTime, Utc};
use htui_core::model::{EdgeId, GraphBody, NodeBody, PhaseId, StepGraphEdge, Gate};
use super::{EdgeSpec, Lookups, forward_edges};

pub(crate) const NO_SUCH_STEP: &str = "no such step in this graph";
pub(crate) const NO_SUCH_LINK: &str = "no such link in this graph";
pub(crate) const SELF_LINK: &str = "a step cannot link to itself";

/// D5: the store's checks in the store's order (`traits.rs:1091-1112`, `mem.rs:4208-4262`):
/// `graph_body_shape_refusal(body.graph.id, body)`; then each `body_persona_refs` id against
/// `lookups.personas` -> `references_no_row(column, id, "persona")`; then, only when
/// `lookups.agents` is `Some`, each `body_agent_refs` id -> `references_no_row(column, id,
/// "agent")`; then `graph_body_topology_refusal`; then `graph_body_walk_refusal` (W1-W5, then
/// the `if` closed set). Steps 4-5 of the store (name taken in scope, ids of other graphs) need
/// store rows and stay the store's. `None`: the body may be sent.
pub(crate) fn validate(body: &GraphBody, lookups: &Lookups) -> Option<String>;

/// D10: a fresh node for "add after `after`": `htui_core::seed::phase_row(PhaseId::new(),
/// body.graph.id, max position + 1 (0 for an empty graph), &PhaseSeed { name: "",
/// input_kinds: &[], gate_hard: false }, now)`, then `gate = Gate::Never`, `retry_limit = 0`,
/// `template_name` = `after`'s node's (empty when `after` is `None` or unknown), `output_kind`
/// empty (the form fills the name). No agents, no parts.
pub(crate) fn draft_node(body: &GraphBody, after: Option<PhaseId>, now: DateTime<Utc>) -> NodeBody;

/// D10 add: `node` appended to `body.nodes` (its `graph_id` set to the body's, agents re-tagged
/// and renumbered 0..n). With `after` a flow node (not palette): every forward edge out of
/// `after` (`forward_edges`) is re-pointed to leave from `node` (same id, kind, predicate,
/// limit), then a `next` `after -> node` is added with `EdgeId::new()`. Back edges of `after`
/// stay. With `after` `None`, unknown (the selection can lag a reload) or a palette node: `node`
/// is a root with no edges. Err `STEP_EXISTS` only when `node.phase.id` is already in the body.
pub(crate) fn add_after(body: &GraphBody, after: Option<PhaseId>, node: NodeBody) -> Result<GraphBody, String>;

/// D8: `node` replaces the node of the same phase id; position, edges and the graph row are
/// kept; agents re-tagged (`phase_id`) and renumbered 0..n in order. Err `NO_SUCH_STEP`.
pub(crate) fn replace_node(body: &GraphBody, node: NodeBody) -> Result<GraphBody, String>;

/// D10 delete: the node goes with every edge touching it. For each forward edge `u -> id`
/// (in body order) and each forward edge `id -> v` (in body order): an edge `u -> v` with the
/// in-edge's kind, predicate and loop limit; the first target reuses the in-edge's id, later
/// ones get `EdgeId::new()`. A rewired edge whose `(from, to, kind)` already exists, or with
/// `u == v`, is dropped. Back edges into or out of `id` are dropped. Positions of the other
/// nodes are left alone (gaps are legal). Err `NO_SUCH_STEP`.
pub(crate) fn delete_node(body: &GraphBody, id: PhaseId) -> Result<GraphBody, String>;

/// D10 connect: a new edge `from -> to` (`EdgeId::new()`, the body's graph id) with `spec`.
/// Err `SELF_LINK` when `from == to`, `NO_SUCH_STEP` for an unknown end. A duplicate
/// `(from, to, kind)` is left to `validate` (topology rule c, the store's sentence).
pub(crate) fn connect(body: &GraphBody, from: PhaseId, to: PhaseId, spec: EdgeSpec) -> Result<GraphBody, String>;

/// D9: the edge `id` takes `spec`'s kind, predicate and limit; ends kept. Err `NO_SUCH_LINK`.
pub(crate) fn edit_edge(body: &GraphBody, id: EdgeId, spec: EdgeSpec) -> Result<GraphBody, String>;

/// D10 links list: the edge `id` removed. Err `NO_SUCH_LINK`.
pub(crate) fn remove_edge(body: &GraphBody, id: EdgeId) -> Result<GraphBody, String>;
```
Plus `pub(crate) const STEP_EXISTS: &str = "a step with this id already exists";`. No op touches `body.graph` (name, description, revision, timestamps).

### Tests (write first; `#[cfg(test)] mod tests` in `model.rs`)

Fixtures:
- Local helpers `node(graph, position, name)`, `edge(graph, from, to, kind)` and `body(names)`, mirroring `graph_rules.rs:926-1006`.
- `MemStore::demo()` with `WriteStore::graph_body` for the five seeded graphs (tokio tests).
- The **reference graph**:
  - `implement -next-> review`
  - `review -changes_requested(3)-> implement`
  - `implement -if {"verify":"failed"}-> fix`

Tests:
1. `validate_accepts_every_seeded_graph_and_the_reference_graph`.
2. `validate_runs_the_store_rules_in_order`. These cases must each give exactly the store function's sentence, and a body failing two stages gives the earlier one:
   - a duplicate node name (shape: `phase_name_is_taken`)
   - an unknown persona (`references_no_row(.., "persona")`)
   - an unknown agent with `agents: Some(vec![])`
   - a `fails` edge with no limit (`edge_needs_loop_limit`)
   - a forward `fails` (`loop_edge_runs_forward`)
   - `if {"bogus":1}` (`if_predicate_outside_the_closed_set`)
3. `validate_leaves_agents_to_the_store_until_they_are_read`: an unknown agent with `agents: None` gives `None`.
4. `draft_node_takes_the_selected_template_gate_never_retry_0_and_the_next_position`.
5. `add_after_takes_over_the_forward_out_edges_and_links_from_the_selected`: feature graph, add after `plan`. `plan -> new` (next) and `new -> implement` reuse the old edge id; `review -> implement` (changes_requested) is untouched.
6. `add_after_leaves_back_edges_where_they_were` (add after `review` in the feature graph): the loop still leaves `review`.
7. `add_with_no_selection_or_a_palette_selection_adds_a_root`.
8. `replace_node_retags_and_renumbers_candidates_and_keeps_edges`.
9. `delete_rewires_each_forward_in_edge_to_each_forward_out_target_keeping_its_kind`: a diamond `a -> x`, `b -if-> x`, `x -> c`, `x -> d` gives `a->c`, `a->d` (next) and `b->c`, `b->d` (if, same predicate).
10. `delete_drops_duplicates_self_links_and_back_edges_on_the_node`.
11. `deleting_any_step_of_the_seeded_and_reference_graphs_validates` (C6, the property).
12. `connect_mints_an_edge_and_refuses_a_self_link`.
13. `edit_edge_and_remove_edge_by_id_refuse_an_unknown_id`.
14. `no_op_touches_the_graph_row`.

**Validate**:
```bash
cargo test -p htui --lib ui::graph_editor::model -- --test-threads=1
```
Then the lint gate.

**Commits**:
1. `feat(MOD-141): T2 validate in the store's order`
2. `feat(MOD-141): T2 add, delete with rewire, connect, edit/remove link, replace node`

---

## T3 - History and save state (D4, D6)

**File set**: `crates/htui/src/ui/graph_editor/history.rs`.

### Interface (to T6)

```rust
use htui_core::model::GraphBody;

pub(crate) const UNDO_DEPTH: usize = 100;
pub(crate) const NOTHING_TO_UNDO: &str = "nothing to undo";
pub(crate) const NOTHING_TO_REDO: &str = "nothing to redo";
pub(crate) const IN_FLIGHT: &str = "still saving \u{2014} wait for the store's answer";

/// One undoable step: the body to go back (or forward) to, and what it was ("adding review").
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry { pub(crate) label: String, pub(crate) body: GraphBody }

/// The write in flight, by what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Write { Edit(String), Undo(String), Redo(String) }

/// Why a write may not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal { Busy, NothingToUndo, NothingToRedo }
impl Refusal { pub(crate) const fn sentence(self) -> &'static str; } // IN_FLIGHT / NOTHING_TO_*

#[derive(Debug, Default)]
pub(crate) struct History {
    undo: Vec<Entry>,             // oldest first
    redo: Vec<Entry>,             // most recently undone last
    pending: Option<Pending>,
}
#[derive(Debug)]
struct Pending { write: Write, before: GraphBody, popped: Option<Entry> }

impl History {
    pub(crate) fn busy(&self) -> Option<&Write>;
    pub(crate) fn undo_label(&self) -> Option<&str>;     // top of undo
    pub(crate) fn redo_label(&self) -> Option<&str>;
    pub(crate) fn depths(&self) -> (usize, usize);
    /// A new edit about to be sent from `current`. Err(Busy).
    pub(crate) fn begin_edit(&mut self, label: impl Into<String>, current: &GraphBody) -> Result<(), Refusal>;
    /// Pops the undo top; answers the body to send (CAS on `current.graph.revision`, which the
    /// caller reads). Err(Busy | NothingToUndo).
    pub(crate) fn begin_undo(&mut self, current: &GraphBody) -> Result<GraphBody, Refusal>;
    pub(crate) fn begin_redo(&mut self, current: &GraphBody) -> Result<GraphBody, Refusal>;
    /// `Applied`: Edit -> push Entry{label, before} on undo (drop the oldest past 100), clear
    /// redo. Undo -> push Entry{label, before} on redo. Redo -> push Entry{label, before} on undo
    /// (cap), redo kept. Answers the write that landed; `None` with nothing pending.
    pub(crate) fn applied(&mut self) -> Option<Write>;
    /// `Stale` (D4): both stacks and the pending write cleared. Answers the write that missed.
    pub(crate) fn stale(&mut self) -> Option<Write>;
    /// `Failed`: a popped entry goes back where it came from; nothing pushed.
    pub(crate) fn failed(&mut self) -> Option<Write>;
    pub(crate) fn clear(&mut self);
}
```
Write a hand-written `Debug` only if a field could carry typed text. Bodies carry stored names, so `derive(Debug)` is fine.

### Tests (write first)

1. `an_applied_edit_pushes_the_previous_body_with_its_label_and_clears_redo`
2. `undo_sends_the_previous_body_and_applied_moves_it_to_redo`
3. `redo_after_undo_round_trips_the_body`
4. `a_new_edit_after_an_undo_clears_redo`
5. `undo_is_capped_at_100_dropping_the_oldest`
6. `a_second_write_is_refused_while_one_is_in_flight`: `Busy` for edit, undo and redo.
7. `stale_clears_both_stacks_and_the_pending_write`
8. `failed_restores_a_popped_entry_and_pushes_nothing` (undo, redo and edit cases)
9. `nothing_to_undo_and_nothing_to_redo_are_refusals_with_their_sentences`
10. `an_answer_with_nothing_pending_changes_nothing`

Model the tests on the spike's `wf_checkpoint` (`spikes/ui-skeleton/src/app.rs:667-688`).

**Validate**:
```bash
cargo test -p htui --lib ui::graph_editor::history -- --test-threads=1
```
Then the lint gate.

**Commit**:
`feat(MOD-141): T3 undo/redo history and the one write in flight`

---

## T4 - Canvas (D7, D11)

**File set**: `crates/htui/src/ui/graph_editor/canvas.rs` and its snapshots.

### Interface (to T6)

```rust
use rataflow::{Direction, Flow, StepEdge, ...};
use htui_core::model::{AgentSummary, EdgeId, EdgeKind, GraphBody, PhaseId, StepGraphEdge, StepGraphId};

pub(crate) const NODE_W: f64 = 24.0;   // border + 22 cells: name/title, template, gate, agents
pub(crate) const NODE_H: f64 = 5.0;    // border, 3 lines, border
pub(crate) const H_GAP: f64 = 6.0;     // room for a side loop between two nodes of a layer
pub(crate) const V_GAP: f64 = 3.0;     // a label off the arrowhead (execution_graph R7)

/// Layout of one body, rataflow-free (unit-tested alone).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    /// Every node's top-left corner in world units: flow layers top to bottom, left to right,
    /// then the palette band.
    pub(crate) positions: Vec<(PhaseId, (f64, f64))>,
    /// Flow layers (palette band not counted).
    pub(crate) layers: usize,
    /// The widest row in world units (0.0 for an empty body).
    pub(crate) width: f64,
}
pub(crate) fn layout(body: &GraphBody) -> Layout;

/// Handle ids by edge (source, target).
pub(crate) const fn edge_handles(kind: EdgeKind, forward: bool) -> (&'static str, &'static str);
/// The label an edge carries, `None` for `next`.
pub(crate) fn edge_label(edge: &StepGraphEdge) -> Option<String>;

/// What a mouse event did on the canvas (D11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanvasEvent {
    /// Left click on a node.
    Select(PhaseId),
    /// Right click on a node: edit it.
    EditNode(PhaseId),
    /// Click on an edge: the links list on it.
    OpenLinks(EdgeId),
    /// A drag from a node's bottom handle dropped on another node.
    Connect { from: PhaseId, to: PhaseId },
}

#[derive(Debug)]
pub(crate) struct Canvas { flow: Flow<StepNode, StepEdge>, graph: Option<StepGraphId>,
    selected: Option<PhaseId>, width: f64, reveal: Reveal, drawn: Option<(u16, u16)>, pan: Option<(u16, u16)> }

impl Canvas {
    pub(crate) fn new() -> Self; // Flow::new().with_edges_reconnectable(false)
                                 //   .with_deselect_on_pane_click(false)
                                 //   .with_connection_radius(NODE_W / 2.0 + 2.0)
    /// Rebuilds the flow from `body` with `selected` selected (baked into the node builder:
    /// `set_nodes` clears selection and handles, execution_graph R2). Same graph: the viewport
    /// shifts by the selected (else first common) node's move (execution_graph `anchor`, :573),
    /// a selection change queues `Reveal::Cursor`. New graph id: `Reveal::Reset`.
    pub(crate) fn sync(&mut self, body: &GraphBody, selected: Option<PhaseId>, agents: &[AgentSummary], theme: &Theme);
    /// `select_node_in_direction`, then the flow's first selected node; `None` leaves the
    /// selection where it was.
    pub(crate) fn select_toward(&mut self, direction: Direction) -> Option<PhaseId>;
    pub(crate) fn zoom_in(&mut self); pub(crate) fn zoom_out(&mut self); pub(crate) fn fit(&mut self);
    /// Left/right press, drag, release, wheel only (execution_graph :629). After every event the
    /// flow's selection is put back on `self.selected`. Never applies a `FlowAction`.
    pub(crate) fn on_mouse(&mut self, mouse: MouseEvent) -> Option<CanvasEvent>;
    pub(crate) fn end_gesture(&mut self);
    /// execution_graph `render` (:703): drawable check, size-change reveal, scratch-buffer
    /// reveal, Reset viewport (zoom 1, centred, MARGIN), pan re-anchor.
    pub(crate) fn render(&mut self, frame: &mut Frame<'_>, area: Rect);
}
```
`StepNode` is a private `NodeContent`. It holds `title: String`, `lines: [String; 3]`, `palette: bool` and a `theme: Theme` copy. Its `render` draws a kit `Pane` titled by the node's name: border `theme.cursor` when `ctx.selected`, `theme.dim` for palette, otherwise `theme.base`. Then up to 3 lines clipped with `cells::clip`, as in `execution_graph.rs:283-313`.

**Layering algorithm** (`layout`):
1. `mask = super::forward_edges(body)`. Flow nodes are the non-palette nodes in `body.nodes` order, which is position order. Palette nodes are kept apart.
2. Consider forward edges whose ends are both flow nodes. Compute their indegree.
3. Kahn's algorithm:
   - Start the queue with the indegree-0 flow nodes in body order.
   - Pop the front.
   - For each forward successor, in edge order: set `layer[v] = max(layer[v], layer[u] + 1)`, decrement its indegree, and push it when the indegree reaches 0.
4. Flow nodes the queue never reached (only in a forward cycle, which the store refuses) take `max_layer + 1, + 2, ...` in body order. The function never panics.
5. Order within a layer:
   - Sort by (smallest in-layer index of a forward predecessor in any earlier layer, else `usize::MAX`), then by `position`.
   - Process layers top-down so predecessor indices are known.
6. `span(n) = n*NODE_W + (n-1)*H_GAP`. `width = max span` over the rows, the palette row included. `x = floor((width - span(row)) / 2) + i*X_STEP`, `y = layer*Y_STEP`, as in `execution_graph.rs:136-152`.
7. Palette band:
   - If there are flow layers, palette nodes go at `y = (layers + 1) * Y_STEP`, one empty band below the last layer. Otherwise at `y = 0`.
   - They are left-aligned at `x = i*X_STEP` in body order.

**Handles per node**. Bake them into every builder:

| Id | Type | Side | Offset | Hidden | Connectable |
|---|---|---|---|---|---|
| `top` | target | Top | 0.5 | no | end only |
| `bottom` | source | Bottom | 0.5 | no | start only |
| `r-out` | source | Right | 0.75 | yes | no |
| `r-in` | target | Right | 0.25 | yes | no |
| `l-out` | source | Left | 0.75 | yes | no |
| `l-in` | target | Left | 0.25 | yes | no |

- Palette nodes: every handle hidden; the node is `with_connectable(false)`.
- Flow nodes: `with_connectable(true)`, `with_draggable(false)`, `with_deletable(false)`, `with_selected(id == selected)`.
- The layout follows the spike (`spikes/ui-skeleton/src/flow.rs:479-523`).

**`edge_handles`**:

| Edge | Handles |
|---|---|
| forward (any kind) | `("bottom", "top")` |
| back `ChangesRequested` | `("r-out", "r-in")` |
| back `AsksYou` | `("r-out", "r-in")` |
| `Fails` | `("l-out", "l-in")` |
| back `Next`/`If` (impossible, defensive) | `("bottom", "top")` |

Edges are `Edge::new(edge_id_string, from_string, to_string)`, built with:
- `.with_source_handle(Some(..)).with_target_handle(Some(..))`
- `.with_deletable(false).with_selectable(true)`
- `StepEdge` content: stroke `theme.dim`, label `theme.base` (`execution_graph.rs:393-417`).

**`edge_label`** (text in words; `×` is `\u{d7}`):

| Kind | Label |
|---|---|
| `next` | none |
| `if` | `if {words}` |
| `asks_you` | `asks you`, plus ` ×{n}` when `loop_limit` is `Some(n)` |
| `changes_requested` | `changes requested ×{n}` |
| `fails` | `fails ×{n}` |

For an `if` edge, `words` is the predicate in words:

| Predicate | Words |
|---|---|
| `{"verify":"failed"}` | `verify failed` |
| `{"verify":"passed"}` | `verify passed` |
| `{"item_kind":"FIX"}` | `kind FIX` |
| `{"item_tag":"x"}` | `tag x` |
| `{"any_changed_path_matches":[g, ...]}` | `any path {g}` (+ ` +{k}`) |
| `{"all_changed_paths_match":[g, ...]}` | `all paths {g}` (+ ` +{k}`) |
| `{"not": p}` | `not ({p words})` |
| `{"all": [...]}` | `all of {n}` |
| `{"any": [...]}` | `any of {n}` |
| anything else | compact JSON |

Clip the label to 24 cells with `…`.

**Node lines**:
- Title (border): the node's name.
- Line 1: `template {name}`, with ` v{n}` when a version is pinned.
- Line 2: `gate {label::gate}`, with ` · hard` when `gate_hard`, and ` · retries {n}` when `retry_limit > 0`.
- Line 3, the first that applies:
  1. `split == Divide`: `divide · runs once MOD-139 lands`.
  2. `split == Compete`: `compete ×{fan_out}`.
  3. `palette`: `palette`.
  4. Otherwise the first candidate, `{agent name} · {model}` (` +{k}` when there are more). Name from `agents`, else the first 8 characters of the id. No candidates: `any agent`.

### Tests (write first)

Layout and rataflow-free:
1. `the_feature_seed_layers_one_step_per_layer`: prd 0, plan 1, implement 2, review 3. The `review -> implement` loop moves nothing.
2. `back_edges_do_not_move_layers`.
3. `an_if_branch_puts_both_targets_in_one_layer_centred`: the reference graph has review and fix in layer 1.
4. `palette_nodes_sit_in_their_own_band_below`.
5. `layout_is_deterministic_and_ignores_edge_order`: shuffle `body.edges`, same `Layout`.
6. `a_forward_cycle_still_lays_out`: a hand-built invalid body, no panic, every node placed.
7. `edge_handles_by_kind_and_direction`.
8. `edge_labels_read_in_words`: every row of the table above.

Flow:

9. `sync_bakes_selection_and_handles_and_keeps_the_viewport_on_a_re_sync` (mirror `execution_graph.rs:1313`).
10. `select_toward_moves_to_the_neighbour`: from implement, `Down` reaches review in the feature seed.
11. `zoom_and_fit_stay_in_range`.
12. `a_click_on_a_node_selects_it`.
13. `a_right_click_on_a_node_edits_it`.
14. `a_click_on_an_edge_opens_its_links`.
15. `a_drag_from_a_bottom_handle_onto_a_node_connects`: press on the `bottom` handle cell, drag, release on another node, and expect `Connect`.
16. `no_mouse_event_removes_a_node_or_an_edge`.

Snapshots: draw into a 100x28 `Buffer` with a local `rows(buf)` helper (as in `execution_graph.rs:1175`), then `insta::assert_snapshot!`:

| Snapshot | Body | Selected |
|---|---|---|
| `linear` | demo analysis graph | `research` |
| `review_loop` | demo feature graph | `implement` |
| `if_branch` | hand-built: implement, review, fix (if) | none |
| `palette` | demo bug graph plus one palette node `triage` | none |
| `divide_badge` | analysis graph with verdict `split = Divide` | none |
| `reference` | the reference graph | `review` |

**Validate**:
```bash
cargo test -p htui --lib ui::graph_editor::canvas -- --test-threads=1
```
Review each new snapshot. Then the lint gate.

**Commits**:
1. `feat(MOD-141): T4 layout, handles and labels` (rataflow-free tests).
2. `feat(MOD-141): T4 canvas flow, selection, zoom and mouse` (with snapshots).

---

## T5 - Forms and links (D8, D9, D10)

**File set**:
- `crates/htui/src/ui/graph_editor/node_form.rs`
- `crates/htui/src/ui/graph_editor/edge_form.rs`
- `crates/htui/src/ui/graph_editor/links.rs`
- their snapshots

Lookups come in through `super::Lookups`:
- The host fills `personas` from `CatalogueSnapshot::personas`.
- T6 fills `agents` from `StoreReply::Agents` (`Vec<AgentSummary>`, by name).
- T6 fills `templates` from `StoreReply::Templates`, filtered by the graph's scope:
  - `Project(p)`: `snapshot.projects` entry `p`, every version.
  - `Workspace` or `Library`: every scope project's rows, deduplicated by `(name, version)`. The Toolbox may refine this.

### `node_form.rs`

```rust
pub(crate) const NAME_REQUIRED: &str = "`name` is required";
pub(crate) const RETRY_IS_A_NUMBER: &str = "`retry limit` is a whole number, 0 or more";
pub(crate) const BUDGET_IS_A_NUMBER: &str = "`token budget` is a whole number of at least 1, or empty";
pub(crate) const NO_TEMPLATE: &str = "no template to choose; add one in Skills \u{203a} Templates";
pub(crate) const AGENTS_NOT_READ: &str = "agents not read yet; the candidates can be edited once they are";
pub(crate) const NO_AGENTS: &str = "no agents to choose from; add one in Settings \u{203a} Agents";
pub(crate) const NO_CANDIDATE: &str = "no candidate agent is selected";
pub(crate) const DIVIDE_NOTE: &str = "runs once MOD-139 lands";

/// Which node the form writes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeTarget { New { after: Option<PhaseId> }, Edit(PhaseId) }

/// The focused row's kind, for the editor's key mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRow { Text, Choice, Toggle, Candidate(usize), NoCandidates }

pub(crate) struct NodeForm {               // hand-written Debug: target, focus, no text
    target: NodeTarget,
    name: TextField, retry_limit: TextField, verify_command: TextField,
    token_budget: TextField, input_kinds: TextField, output_kind: TextField,
    template_name: String, template_version: Option<i32>, persona: Option<PersonaId>,
    gate: Gate, gate_hard: bool, isolation: Option<Isolation>, command_queue: CommandQueue,
    candidates: Vec<(AgentId, String)>,
    focus: usize,
}
impl NodeForm {
    /// Prefilled from `node` (D8); `target` Edit or New.
    pub(crate) fn new(node: &NodeBody, target: NodeTarget) -> Self;
    pub(crate) const fn target(&self) -> NodeTarget;
    pub(crate) fn focused(&self) -> NodeRow;
    pub(crate) fn next_field(&mut self);           // wraps; skips nothing editable
    pub(crate) fn prev_field(&mut self);
    /// The focused text row takes the key; `FieldOutcome::Pass` off a text row.
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome;
    pub(crate) fn on_paste(&mut self, text: &str); // focused text row only
    /// The focused choice, toggle or candidate row moves one option (wraps). Template change
    /// resets the version to latest. Err: AGENTS_NOT_READ / NO_AGENTS on a candidate row.
    pub(crate) fn cycle(&mut self, forward: bool, lookups: &Lookups) -> Result<(), String>;
    /// Appends the first (agent, model) pair; focus moves to it. Err AGENTS_NOT_READ / NO_AGENTS.
    pub(crate) fn candidate_add(&mut self, lookups: &Lookups) -> Result<(), String>;
    pub(crate) fn candidate_remove(&mut self) -> Result<(), String>;      // Err NO_CANDIDATE
    pub(crate) fn candidate_move(&mut self, up: bool) -> Result<(), String>;
    /// Parses and assembles. `base` supplies every column the form does not edit (id, graph_id,
    /// position, fan_out, split, palette, plan_scope, divide columns, parts, updated_at).
    pub(crate) fn build(&self, base: &NodeBody) -> Result<NodeBody, String>;
    pub(crate) fn title(&self, body: &GraphBody) -> String;  // "step research" / "new step after
                                                             // research" / "new step"
    pub(crate) fn lines(&self, base: &NodeBody, lookups: &Lookups, width: u16, accent: Style,
                        theme: &Theme) -> Vec<Line<'static>>; // kit Form built locally, editing(true)
}
/// Every (agent, model) pair, agents in reply order, models in `agent.models` order; an agent
/// with no models gives `(agent, default_model or "")`.
pub(crate) fn agent_pairs(agents: &[AgentSummary]) -> Vec<(AgentId, String)>;
```

**Fields in focus order** (each kit row label in quotes):

| # | Label | Value | Parse / refusal |
|---|---|---|---|
| 0 | `name` | Text | trimmed; empty → `NAME_REQUIRED` |
| 1 | `template` | Choice over sorted unique template names, plus the current name even when the list lacks it | empty and no names → `NO_TEMPLATE` |
| 2 | `version` | Choice: `latest`, then `v{n}` descending for the chosen name | `Option<i32>` |
| 3 | `persona` | Choice: `none`, then `lookups.personas` order; an unknown current id shows the id | `Option<PersonaId>` |
| 4 | `gate` | Choice: Always, OnFailure, Never (`label::gate`) | — |
| 5 | `hard gate` | Toggle; `cycle` flips it | — |
| 6 | `retry limit` | Text | `i32 >= 0`, else `RETRY_IS_A_NUMBER` |
| 7 | `isolation` | Choice: `project default`, then Worktree, Copy, SharedSerialized, Local (`label::isolation`) | `Option<Isolation>` |
| 8 | `commands` | Choice: Off, FanOutOnly, Always (`label::command_queue`) | — |
| 9 | `verify` | Text | trimmed; empty → `None` |
| 10 | `token budget` | Text | empty → `None`; `i32 >= 1`, else `BUDGET_IS_A_NUMBER` |
| 11 | `inputs` | Text | comma split, trim, drop empties, keep order (the `kinds.rs:1843` rule) |
| 12 | `output` | Text | trimmed; empty → the name |
| 13.. | `agent 1..n` | Choice over `agent_pairs`, label `{agent name} · {model}`; unknown agent shows its id | → `PhaseAgent { phase_id: base id, position: i, agent_id, model }` |
| — | `agents` | Placeholder row (`NodeRow::NoCandidates`) shown when there is no candidate, text `none (ctrl-n adds one)` | the editor substitutes the key label |

Read-only rows (`Value::ReadOnly`, never focused) come after the candidates:
- `split`: `none`, `compete`, or `divide · ` + `DIVIDE_NOTE`.
- `fan-out`
- `palette`
- `plan scope`
- `divide`: planner and integrate persona/agent/model and `max parts`, only when divide.
- `parts`: count.
- `deadline`: `edited in M2`.

Edit target: any existing candidates are kept as they are while `agents` is `None`.

### `edge_form.rs`

```rust
pub(crate) const PREDICATE_REQUIRED: &str = "`if` needs a predicate, e.g. {\"verify\":\"failed\"}";
pub(crate) fn predicate_not_json(error: &serde_json::Error) -> String;   // "`if` is not JSON: {error}"
pub(crate) fn predicate_refused(error: &PredicateError) -> String;      // "`if`: {error}"
pub(crate) fn loop_limit_required(kind: EdgeKind) -> String;            // "`loop limit` is required for {relation_word}"
pub(crate) const LOOP_IS_A_NUMBER: &str = "`loop limit` is a whole number, 0 or more";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeTarget { New { from: PhaseId, to: PhaseId }, Edit(EdgeId) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeRow { Relation, Predicate, LoopLimit }

pub(crate) struct EdgeForm { target: EdgeTarget, kind: EdgeKind, predicate: TextField,
                             loop_limit: TextField, focus: EdgeRow }   // hand-written Debug
impl EdgeForm {
    pub(crate) fn new(from: PhaseId, to: PhaseId) -> Self;              // kind Next
    pub(crate) fn edit(edge: &StepGraphEdge) -> Self;                   // predicate as compact JSON
    #[must_use] pub(crate) fn with_kind(self, kind: EdgeKind) -> Self;  // links-list cycling
    pub(crate) const fn target(&self) -> EdgeTarget;
    pub(crate) fn focused(&self) -> EdgeRow;
    /// The rows shown: Relation always; Predicate only for `if`; LoopLimit only for asks_you,
    /// changes_requested, fails.
    pub(crate) fn rows(&self) -> Vec<EdgeRow>;
    pub(crate) fn next_field(&mut self); pub(crate) fn prev_field(&mut self);
    pub(crate) fn cycle(&mut self, forward: bool);       // Relation row only; RELATIONS order, wraps
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome;
    pub(crate) fn on_paste(&mut self, text: &str);
    /// D9 live parse, shown under the predicate row: `None` when empty or parsing.
    pub(crate) fn predicate_error(&self) -> Option<String>;
    /// `if`: predicate required, JSON, `IfPredicate::parse` -> `EdgePredicate(value)`.
    /// changes_requested / fails: limit required. asks_you: limit optional. next / if: limit None
    /// whatever was typed. Limit: `i32 >= 0` else LOOP_IS_A_NUMBER.
    pub(crate) fn build(&self) -> Result<EdgeSpec, String>;
    pub(crate) fn title(&self, body: &GraphBody) -> String;   // "link implement → review"
    pub(crate) fn lines(&self, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>>;
}
/// One step through `RELATIONS`, wrapping.
pub(crate) fn cycle_kind(kind: EdgeKind, forward: bool) -> EdgeKind;
```
Relation labels come from `relation_word`. The predicate row puts `predicate_error()` in kit `Field::error`.

### `links.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkRow { pub(crate) edge: EdgeId, pub(crate) outgoing: bool, pub(crate) text: String }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Links { node: PhaseId, cursor: usize }
impl Links {
    pub(crate) const fn new(node: PhaseId) -> Self;
    /// The list of `edge`'s source with the cursor on `edge` (a click on an edge); `None` when
    /// the body has no such edge.
    pub(crate) fn on_edge(body: &GraphBody, edge: EdgeId) -> Option<Self>;
    pub(crate) const fn node(&self) -> PhaseId;
    /// Out-edges in body order, then in-edges in body order. Text: `→ {to} · {relation}` /
    /// `← {from} · {relation}`, plus ` · {predicate words or compact JSON}` for `if` and
    /// ` · limit {n}` when `loop_limit` is `Some`.
    pub(crate) fn rows(&self, body: &GraphBody) -> Vec<LinkRow>;
    pub(crate) fn down(&mut self, body: &GraphBody); pub(crate) fn up(&mut self);
    pub(crate) fn clamp(&mut self, body: &GraphBody);
    pub(crate) fn selected(&self, body: &GraphBody) -> Option<EdgeId>;
    pub(crate) fn lines(&self, body: &GraphBody, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>>;
    // kit list rows; "no links" empty state
}
```

### Tests (write first)

Node form:
1. `the_form_prefills_every_m1_field_from_the_node`
2. `build_writes_every_field_back_and_keeps_the_m2_columns_of_the_base`
3. `an_empty_name_is_refused`
4. `retry_limit_is_a_whole_number_0_or_more`
5. `token_budget_empty_is_none_and_below_1_is_refused`
6. `verify_empty_is_none`
7. `inputs_split_on_commas_trimmed_in_order`
8. `output_empty_is_the_name`
9. `template_cycles_through_the_scope_templates_and_keeps_an_unknown_current`
10. `version_resets_to_latest_when_the_template_changes`
11. `persona_cycles_none_then_the_registry_and_shows_an_unknown_id`
12. `gate_cycles_and_hard_gate_toggles`
13. `isolation_cycles_project_default_then_the_four`
14. `commands_cycles_the_three`
15. `candidates_add_remove_and_move_renumber_positions`
16. `a_candidate_cycles_through_every_agent_model_pair`
17. `candidates_need_the_agents_read_and_some_agent`
18. `next_field_skips_the_read_only_rows`
19. `debug_never_prints_typed_text`

Edge form:
1. `a_new_link_is_next_with_no_predicate_or_limit`
2. `cycle_kind_runs_next_if_asks_you_changes_requested_fails_and_wraps`
3. `if_needs_a_predicate`
4. `a_predicate_must_be_json`
5. `a_predicate_outside_the_closed_set_is_refused_live`: `predicate_error` must give the `PredicateError` text.
6. `loop_limit_is_required_on_changes_requested_and_fails`
7. `loop_limit_is_optional_on_asks_you_and_dropped_on_next_and_if`
8. `loop_limit_is_0_or_more`
9. `edit_prefills_kind_predicate_and_limit`
10. `rows_follow_the_relation`

Links:
1. `rows_list_out_then_in_edges_in_body_order` (feature graph's `review`)
2. `the_cursor_moves_and_clamps_after_a_removal`
3. `on_edge_selects_that_edge`

Snapshots (`lines(..)` joined, width 56):
- `node_form_edit`: research with persona `reviewer` and 2 candidates.
- `node_form_new`
- `node_form_divide`
- `edge_form_if_error`
- `edge_form_changes_requested`
- `links_review`

**Validate**:
```bash
cargo test -p htui --lib ui::graph_editor::node_form -- --test-threads=1
cargo test -p htui --lib ui::graph_editor::edge_form -- --test-threads=1
cargo test -p htui --lib ui::graph_editor::links -- --test-threads=1
```
Then the lint gate.

**Commits**:
1. `feat(MOD-141): T5 edge form and links list`
2. `feat(MOD-141): T5 node form with candidates and read-only M2 rows`

---

## T6 - `GraphEditor` and its keys (D1, D4, D11, D12)

**File set**:
- `crates/htui/src/ui/graph_editor/mod.rs`
- `crates/htui/src/keys/catalogue.rs`
- `crates/htui/src/keys/views.rs`
- `crates/htui/src/keys/stack.rs`
- `crates/htui/src/keys/load.rs`
- `crates/htui/tests/keys_file.rs`
- `docs/htui-keys.md`
- `graph_editor` mod snapshots
- small fixes to wave-1 modules if integration needs them

### Contexts (catalogue.rs)

1. Add these variants, after `SettingsMemory` in the enum and in `Context::ALL`:
   - `GraphEditor`: doc `[graph_editor]`, the graph editor's canvas, its connect mode and its forms (MOD-141 D12).
   - `GraphEditorLinks`: doc `[graph_editor.links]`, the graph editor's links list.
2. `table()`: `"graph_editor"`, `"graph_editor.links"`.
3. `heading()`: `"Graph editor"`, `"Links"`.
4. Add both to `is_view()`. Neither is shared.
5. Add `GraphEditor, GraphEditorLinks` to the `use Context::{...}` list (`:45`).
6. `context_position`: `SettingsMemory => 21` is unchanged. Add `GraphEditor => 22` and `GraphEditorLinks => 23`. Every later context moves up by 2: `Concepts => 24`, ... `Answer => 53`.
7. `every_context_is_listed_once_in_all`: 52 → **54**.
8. `context_tables_and_headings`: add the two tuples after `settings.memory`, and `assert!(Context::GraphEditor.is_view() && !Context::GraphEditor.is_shared())`, the same for `GraphEditorLinks`.

### Acts

Append to `enum Act` after `MemoryClashReplace`, each with a doc line ``/// `graph_editor.<name>`: ... (MOD-141 D12).``. Catalogue rows go in a block after `[settings.memory]` and before `[concepts]`, in context order (`catalogue_blocks_follow_context_all`). All are `row(..)`, so `since: 1`:

| Act | Context | name | defaults | help | position |
|---|---|---|---|---|---|
| `GraphEditorAdd` | GraphEditor | `add` | `a` | add step | 222 |
| `GraphEditorEdit` | GraphEditor | `edit` | `e`, `enter` | edit | 223 |
| `GraphEditorDelete` | GraphEditor | `delete` | `x` | delete step | 224 |
| `GraphEditorConnect` | GraphEditor | `connect` | `c` | connect | 225 |
| `GraphEditorOpenLinks` | GraphEditor | `open_links` | `i` | links | 226 |
| `GraphEditorUndo` | GraphEditor | `undo` | `u`, `ctrl-z` | undo | 227 |
| `GraphEditorRedo` | GraphEditor | `redo` | `U`, `ctrl-y` | redo | 228 |
| `GraphEditorZoomIn` | GraphEditor | `zoom_in` | `+` | zoom in | 229 |
| `GraphEditorZoomOut` | GraphEditor | `zoom_out` | `-` | zoom out | 230 |
| `GraphEditorFit` | GraphEditor | `zoom_fit` | `=` | fit | 231 |
| `GraphEditorLeft` | GraphEditor | `left` | `left`, `h` | select left | 232 |
| `GraphEditorDown` | GraphEditor | `down` | `down`, `j` | select down | 233 |
| `GraphEditorUp` | GraphEditor | `up` | `up`, `k` | select up | 234 |
| `GraphEditorRight` | GraphEditor | `right` | `right`, `l` | select right | 235 |
| `GraphEditorClose` | GraphEditor | `close` | `esc` | close | 236 |
| `GraphEditorCandidateAdd` | GraphEditor | `candidate_add` | `ctrl-n` | add agent | 237 |
| `GraphEditorCandidateRemove` | GraphEditor | `candidate_remove` | `ctrl-d` | remove agent | 238 |
| `GraphEditorCandidateUp` | GraphEditor | `candidate_up` | `alt-up` | agent up | 239 |
| `GraphEditorCandidateDown` | GraphEditor | `candidate_down` | `alt-down` | agent down | 240 |
| `LinksEdit` | GraphEditorLinks | `edit` | `e`, `enter` | edit link | 241 |
| `LinksDelete` | GraphEditorLinks | `delete` | `x`, `d` | delete link | 242 |
| `LinksClose` | GraphEditorLinks | `close` | `esc` | back | 243 |

Test pins in `catalogue.rs`:
- Append all 22 to `const ALL: &[Act]` (`:2527`) and to `position()`.
- `every_act_has_exactly_one_row`: 222 → **244**.
- `every_m2_row_is_since_2`: unchanged, since the new rows are `since: 1` and outside M2's block.

Why these chords cannot collide:
- `no_two_actions_in_one_context_share_a_default_chord` holds: the chords in `[graph_editor]` are all distinct, and the two `edit`/`delete`/`close` names live in different contexts.
- `no_act_name_is_a_nested_table` holds: no act is named `links` (C7).
- `no_view_name_hides_a_shared_name_it_inherits` holds: the graph_editor view layers inherit only `form.next_field`/`prev_field` and `confirm.yes`/`no`, and the links layer inherits only `list.down`/`up`. No own name matches those.
- Esc: `close` defaults to `esc` like `chat.end` (`catalogue.rs:2055`). It is kept by the loader's MD7 rule.

`VIEW_DEFAULTS`: add after the `SettingsMemory` rows:
```rust
// MOD-141 D12: the graph editor's forms move on `Down`/`Up`, as every Settings form does.
(GraphEditor, Act::FormNextField, &["down"]),
(GraphEditor, Act::FormPrevField, &["up"]),
```
These rows never meet `graph_editor.down`/`up`: the canvas stacks offer no `form` layer, and the form stacks do not offer the direction acts.

### Stacks (views.rs)

Declare these after `KINDS_CONFIRM`, each with a doc comment:
```rust
/// The graph editor's canvas (MOD-141 D12): its own acts and the modal global layer only - no
/// `settings`, `common` or `layout`, so `c` is not `common.clear` and `h`/`l` select.
pub const GRAPH_EDITOR: Stack<'static> = Stack::new(&[
    Layer::view(Context::GraphEditor, &[
        Act::GraphEditorAdd, Act::GraphEditorEdit, Act::GraphEditorDelete, Act::GraphEditorConnect,
        Act::GraphEditorOpenLinks, Act::GraphEditorUndo, Act::GraphEditorRedo, Act::GraphEditorZoomIn,
        Act::GraphEditorZoomOut, Act::GraphEditorFit, Act::GraphEditorLeft, Act::GraphEditorDown,
        Act::GraphEditorUp, Act::GraphEditorRight, Act::GraphEditorClose,
    ]),
    MODAL,
]);
/// Connecting: the directions pick the target, `edit` (Enter) connects here, `close` cancels.
pub const GRAPH_EDITOR_CONNECT: Stack<'static> = Stack::new(&[
    Layer::view(Context::GraphEditor, &[
        Act::GraphEditorLeft, Act::GraphEditorDown, Act::GraphEditorUp, Act::GraphEditorRight,
        Act::GraphEditorZoomIn, Act::GraphEditorZoomOut, Act::GraphEditorFit,
        Act::GraphEditorEdit, Act::GraphEditorClose,
    ]),
    MODAL,
]);
/// The node form: text fields, choices, the field moves and the candidate list.
pub const GRAPH_EDITOR_NODE_FORM: Stack<'static> = Stack::new(&[
    Layer::view(Context::GraphEditor, &[
        Act::GraphEditorCandidateAdd, Act::GraphEditorCandidateRemove,
        Act::GraphEditorCandidateUp, Act::GraphEditorCandidateDown,
    ]),
    FIELD, PICK, FORM_FIELDS, MODAL,
]);
/// The edge form: relation (a choice), predicate and loop limit (text fields).
pub const GRAPH_EDITOR_EDGE_FORM: Stack<'static> =
    Stack::new(&[Layer::view(Context::GraphEditor, &[]), FIELD, PICK, FORM_FIELDS, MODAL]);
/// The links list: rows, relation cycling (it opens the edge form), edit, delete, back.
pub const GRAPH_EDITOR_LINKS: Stack<'static> = Stack::new(&[
    Layer::view(Context::GraphEditorLinks, &[Act::LinksEdit, Act::LinksDelete, Act::LinksClose]),
    PICK, LIST, MODAL,
]);
/// The delete question.
pub const GRAPH_EDITOR_CONFIRM: Stack<'static> =
    Stack::new(&[Layer::view(Context::GraphEditor, &[]), CONFIRM, MODAL]);
```

Tests in `views.rs`:
- `VIEWS` (`:1173`): add `source!("ui/graph_editor/mod.rs")` (C8).
- `VIEW_STACKS` (`:1274`): add the 6 names after `KINDS_CONFIRM`.
- `the_m2_layers_sit_where_the_blueprint_puts_them` (`:1905`): insert `"GRAPH_EDITOR_NODE_FORM", "GRAPH_EDITOR_EDGE_FORM"` after `"KINDS_EDITOR"`.
- `every_view_layer_comes_first...` (`:2012`): 84 → **90**, and extend the comment ("MOD-141 D12: the graph editor's six, 90").
- `the_view_stacks_resolve_as_designed`: add these asserts:

  | Stack | Chord | Resolves to |
  |---|---|---|
  | `GRAPH_EDITOR` | `"c"` | `[GraphEditorConnect]` |
  | `GRAPH_EDITOR` | `"ctrl-z"` | `[GraphEditorUndo]` |
  | `GRAPH_EDITOR` | `"z"` | `[]` |
  | `GRAPH_EDITOR` | `"q"` | `[]` |
  | `GRAPH_EDITOR` | `"ctrl-c"` | `[Quit]` |
  | `GRAPH_EDITOR` | `"l"` | `[GraphEditorRight]` |
  | `GRAPH_EDITOR_CONNECT` | `"enter"` | `[GraphEditorEdit]` |
  | `GRAPH_EDITOR_NODE_FORM` | `"ctrl-n"` | `[GraphEditorCandidateAdd]` |
  | `GRAPH_EDITOR_NODE_FORM` | `"tab"` | `[FormNextField]` |
  | `GRAPH_EDITOR_LINKS` | `"h"` | `[ChoicePrev]` |
  | `GRAPH_EDITOR_LINKS` | `"d"` | `[LinksDelete]` |
  | `GRAPH_EDITOR_LINKS` | `"j"` | `[ListDown]` |
  | `GRAPH_EDITOR_CONFIRM` | `"y"` | `[ConfirmYes]` |

`DELEGATES` needs no entry: `mod.rs` names every act.

### DECLARED (stack.rs)

Append after the memory entries:
```rust
// MOD-141 D12: the graph editor, appended so every first-reported phrase is unchanged.
("in the graph editor", views::GRAPH_EDITOR),
("while the graph editor connects", views::GRAPH_EDITOR_CONNECT),
("in the graph editor's step form", views::GRAPH_EDITOR_NODE_FORM),
("in the graph editor's link form", views::GRAPH_EDITOR_EDGE_FORM),
("in the graph editor's links list", views::GRAPH_EDITOR_LINKS),
("in the graph editor's delete question", views::GRAPH_EDITOR_CONFIRM),
```
`declared_starts_with_base...` (`:439`): 91 → **97**. `DECLARED[83]` and `[84]` are unchanged.

### Table lists

- `load.rs` `TABLES` (`:539-549`): insert `graph_editor, graph_editor.links,` after `settings.memory,`.
- `tests/keys_file.rs` binary-module table string (`:245-252`): the same insertion.
- `docs/htui-keys.md`: regenerate the toml block from `cargo run -p htui -- --default-keys --print-keys`. The rule is `the_key_reference_page_is_the_default_print`, `keys_file.rs:724-745`.

### `GraphEditor` (mod.rs; replaces the T0 allow)

```rust
/// What one key did: handled (or passed) inside the editor, or the editor asks its host to close
/// it (`graph_editor.close` with nothing in flight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKey {
    /// The editor's answer, as a section's.
    Handled(Handled),
    /// The host goes back to its own view.
    Close,
}

/// MOD-141 D1: the step-graph editor on one graph of any scope. Holds no store handle
/// (`R-NF-3`): every write leaves through `Ctx::request`, every reply comes back through
/// `on_reply`. Hand-written `Debug` (graph id, revision, mode name, selection, busy).
pub struct GraphEditor {
    body: GraphBody,
    selected: Option<PhaseId>,
    mode: Mode,
    history: history::History,
    lookups: Lookups,
    canvas: RefCell<canvas::Canvas>,   // rataflow renders through `&mut` (runs.rs:354 pattern)
    resync: Cell<bool>,                // body/selection/lookups changed since the last sync
    after_save: Option<PhaseId>,       // the node to select when the write lands (an add)
    notice: Option<Notice>,            // Info / Error, as kinds.rs:384
}
enum Mode {
    Canvas,
    Connecting { from: PhaseId },
    NodeForm { form: node_form::NodeForm, base: NodeBody },
    EdgeForm(edge_form::EdgeForm),
    Links(links::Links),
    ConfirmDelete { node: PhaseId },
}

impl GraphEditor {
    /// Opens on `body` with the host's persona registry, and answers the reads to send now:
    /// `[StoreRequest::Agents, StoreRequest::Templates(scope.clone())]`. Selects the first flow
    /// node (position order), else none.
    pub fn open(body: GraphBody, personas: Vec<PersonaSummary>, scope: &Scope) -> (Self, Vec<StoreRequest>);
    /// The graph on screen.
    pub fn graph(&self) -> StepGraphId;
    /// Selects step `node` (the host opens on a phase row); unknown ids are ignored.
    pub fn select(&mut self, node: PhaseId);
    /// Opens the node form on the selected step (`e` on a phase row, D14).
    pub fn open_node_form(&mut self);
    /// The reads the host re-sends on activation: Agents, Templates(scope).
    pub fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>;
    /// The persona registry changed (every catalogue the host adopts).
    pub fn set_personas(&mut self, personas: &[PersonaSummary]);
    /// Whether a write is in flight.
    pub fn busy(&self) -> bool;
    /// The current mode's stack (Canvas GRAPH_EDITOR, Connecting GRAPH_EDITOR_CONNECT, NodeForm
    /// GRAPH_EDITOR_NODE_FORM, EdgeForm GRAPH_EDITOR_EDGE_FORM, Links GRAPH_EDITOR_LINKS,
    /// ConfirmDelete GRAPH_EDITOR_CONFIRM).
    pub fn stack(&self) -> Stack<'static>;
    /// The mode's keys through its stack, plus ` · saving…` while busy.
    pub fn hint_text(&self, keys: &Keys) -> String;
    /// Whether a paste lands (a form's focused text row).
    pub fn takes_paste(&self) -> bool;
    /// One key (the host hands every key while the editor is open).
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> EditorKey;
    /// A bracketed paste into the focused text row.
    pub fn on_paste(&mut self, text: &str) -> Handled;
    /// One mouse event on the canvas (D11); `Pass` off it or outside Canvas mode.
    pub fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled;
    /// The mouse went away mid-gesture.
    pub fn on_mouse_lost(&mut self);
    /// Every reply the host receives; the editor takes its own.
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>);
    /// Draws over the whole `area` (D2).
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);
}
```

**Sentences** (consts in `mod.rs`):
- `STALE_RELOADED`: `"changed elsewhere, reloaded; undo history cleared"` (D4, exact).
- `saved(label)`: `"saved: {label}"`.
- `undid(label)`: `"undid {label}"`.
- `redid(label)`: `"redid {label}"`.
- `NO_STEP_SELECTED`: `"no step is selected"`.
- `NOT_CONNECTED`: `"not connected"`.
- `PALETTE_HAS_NO_LINKS`: `"a palette step has no links"`.
- `history::IN_FLIGHT`, `NOTHING_TO_UNDO` and `NOTHING_TO_REDO` from T3.
- The delete question (C3; `{answers}` is `keys.hint(GRAPH_EDITOR_CONFIRM, HINT_CONFIRM)`):
  `"delete step {name}? its links go with it, and what led into it now leads to what came after; undo brings the step back but not its skill bindings, deadline or judge. {answers}"`

**History labels**:

| Edit | Label |
|---|---|
| add | `adding {name}` |
| node edit | `editing {name}` |
| delete | `deleting {name}` |
| connect | `linking {from} → {to}` |
| edge edit | `changing the link {from} → {to}` |
| link removal | `removing the link {from} → {to}` |

**Header** (one line, `theme.title`):
- `{graph name} · {project graph|workspace graph|library graph} · revision {n}`.
- Then, when `history.undo_label()` is some: ` · {undo key} undoes: {label}`. The undo key is `keys.label(GRAPH_EDITOR, GraphEditorUndo)`, so `u` under the defaults.

**Render layout** (`render`):
- Row 0 is the header. The last 1-2 rows are the notice, or the delete question, wrapped (kit `notice`, tone Error for refusals and stale).
- In between:
  - In Canvas, Connecting and ConfirmDelete modes, the canvas takes the whole middle.
  - In NodeForm, EdgeForm and Links modes, the middle splits horizontally: the canvas `Min(30)` on the left, the panel `Length(min(56, width/2))` on the right.
  - The panel is the form's title in `theme.title`, then `form.lines(..)` (or `links.lines`).
- The canvas draws through `self.canvas.borrow_mut()`. Before it draws, if `resync` is set, call `canvas.sync(&self.body, self.selected, agents_or_empty, ctx.theme)`.
- `on_key` and `on_mouse` run the same resync first, so a key that arrives before the first frame still finds a synced flow.

**Hints** (`HintSpec` consts):

| Spec | Entries |
|---|---|
| `HINT_CANVAS` | Pair(Down, Up, "select"), One(Add, "add"), One(Edit, "edit"), One(Delete, "delete"), One(Connect, "connect"), One(OpenLinks, "links"), One(Undo, "undo"), One(Redo, "redo"), One(Fit, "fit"), One(Close, "close") |
| `HINT_CONNECT` | Pair(Down, Up, "pick"), One(Edit, "connect here"), One(Close, "cancel") |
| `HINT_NODE_FORM` | Pair(FormNextField, FormPrevField, "field"), Pair(ChoicePrev, ChoiceNext, "value"), One(CandidateAdd, "add agent"), One(CandidateRemove, "remove agent"), Pair(CandidateUp, CandidateDown, "order"), One(FieldSubmit, "save"), One(FieldCancel, "cancel") |
| `HINT_EDGE_FORM` | Pair(FormNextField, FormPrevField, "field"), Pair(ChoicePrev, ChoiceNext, "relation"), One(FieldSubmit, "save"), One(FieldCancel, "cancel") |
| `HINT_LINKS` | Pair(ListDown, ListUp, "select"), Pair(ChoicePrev, ChoiceNext, "relation"), One(LinksEdit, "edit"), One(LinksDelete, "delete"), One(LinksClose, "back") |
| `HINT_CONFIRM` | One(ConfirmYes, "delete"), All(ConfirmNo, "keep") |

**Mode machine (`on_key`)**: resolve with `ctx.pressed(&key)` and `ctx.keys().actions(self.stack(), chord)`. A chord nothing takes falls to `modal_rest(stack, chord)`.

**Canvas** mode:

| Act | Effect |
|---|---|
| `GraphEditorLeft`/`Right`/`Up`/`Down` | `canvas.select_toward(Direction::..)`; a `Some` becomes `self.selected` and sets `resync`. |
| `GraphEditorZoomIn`/`ZoomOut`/`Fit` | Applied to the canvas. |
| `GraphEditorAdd` | Busy → `IN_FLIGHT`. Otherwise `base = model::draft_node(&body, selected, Utc::now())` and `Mode::NodeForm { form: NodeForm::new(&base, New { after: selected }), base }`. |
| `GraphEditorEdit` | No selection → `NO_STEP_SELECTED`. Otherwise `NodeForm::new(node, Edit(id))` with `base = node.clone()`. |
| `GraphEditorDelete` | Busy → refuse. No selection → refuse. Otherwise `ConfirmDelete`. |
| `GraphEditorConnect` | Busy, no selection, or a palette step → refuse. Otherwise `Connecting { from }`. |
| `GraphEditorOpenLinks` | No selection → refuse. Otherwise `Links(Links::new(id))`. |
| `GraphEditorUndo` | `history.begin_undo(&body)`. Err → its sentence. Ok(prev) → `send(prev, ctx)`. |
| `GraphEditorRedo` | `history.begin_redo(&body)`, as undo. |
| `GraphEditorClose` | Busy → `IN_FLIGHT`. Otherwise `EditorKey::Close`. |

**Connecting** mode:
- The directions move the selection.
- `GraphEditorEdit`: the target is `selected`. If it equals `from` → `model::SELF_LINK`. Otherwise `Mode::EdgeForm(EdgeForm::new(from, target))`.
- `GraphEditorClose` → Canvas with notice `NOT_CONNECTED`.

**NodeForm** mode:
1. If the focused row is `Text`, offer the key to `form.on_text_key(key)` first:
   - `Consumed` → done.
   - `Submit` → save.
   - `Cancel` → Canvas.
   - `Pass` → continue to step 2.
2. Resolve acts:
   - `FormNextField`/`FormPrevField` → move the focus.
   - `ChoicePrev`/`ChoiceNext` → `form.cycle(..)`; an Err becomes the notice.
   - `GraphEditorCandidateAdd`/`Remove`/`Up`/`Down` → the candidate list.
   - `FieldSubmit` → save.
   - `FieldCancel` → Canvas.
3. Save:
   1. Busy → `IN_FLIGHT`.
   2. `form.build(&base)`.
   3. Build the next body: target `New` → `model::add_after(&body, after, node)`; target `Edit` → `model::replace_node`.
   4. `model::validate(&next, &lookups)`. A `Some(sentence)` becomes the notice; nothing is sent, and the form stays open.
   5. `history.begin_edit(label, &body)`.
   6. `send(next, ctx)`. Remember `after_save = Some(new id)` for an add.

**EdgeForm** mode:
- Same text-row-first shape as NodeForm. `ChoicePrev`/`ChoiceNext` call `form.cycle`.
- Save: `form.build()`, then target `New` → `model::connect` or target `Edit` → `model::edit_edge`, then validate, `begin_edit` and send.

**Links** mode:
- `ListDown`/`ListUp` move the cursor.
- `ChoicePrev`/`ChoiceNext` → `EdgeForm::edit(edge).with_kind(cycle_kind(kind, forward))` as `Mode::EdgeForm`. Nothing is sent until Enter: one write per edit.
- `LinksEdit` → `EdgeForm::edit(edge)`.
- `LinksDelete` → busy check, `model::remove_edge`, validate, `begin_edit`, send. The mode stays `Links`.
- `LinksClose` → Canvas.

**ConfirmDelete** mode:
- `ConfirmYes` → `model::delete_node`, validate (a refusal: notice, back to Canvas, nothing sent), `begin_edit`, send, then Canvas.
- `ConfirmNo` → Canvas.

**`send(next, ctx)`**:
```rust
ctx.request(StoreRequest::ReplaceGraph { scope: ctx.scope.clone(), graph: self.body.graph.id, expected_revision: self.body.graph.revision, body: Box::new(next) })
```
The editor keeps showing the current body until the reply. That is D4: busy until the reply.

**`on_reply`**:

- `GraphReplaced { outcome: Applied(body), .. }` where `body.graph.id == self.graph()` and `history.busy()` is some:
  1. `self.body = *body`.
  2. `history.applied()` gives the write. The notice is Info `saved`/`undid`/`redid` with its label.
  3. The mode goes back to Canvas, except after a links delete, where it stays `Links` (clamped).
  4. `selected = after_save.take()` when one is set. Otherwise keep the selection if the node still exists, else the first node.
  5. Set `resync`.
- `GraphReplaced { outcome: Stale(current), .. }` for this graph:
  1. `self.body = *current`.
  2. `history.stale()`.
  3. Mode Canvas.
  4. Notice Error `STALE_RELOADED`.
  5. Re-check the selection, and set `resync`.
- A `GraphReplaced` for another graph, or one with nothing pending, is ignored.
- `Failed { request: "replace_graph", message }` while busy:
  - `history.failed()`.
  - Notice Error `message`.
  - The mode stays as it was, so a form keeps its text and Enter retries it.
- `Agents(list)` → `lookups.agents = Some(list.clone())`, set `resync`.
- `Failed { request: "agents", message }` → notice Info `"agents unavailable: {message}"`.
- `Templates(snapshot)` → `lookups.templates = Some(filter(snapshot, body.graph.scope))`, using T5's scope rule.
- `Failed { request: "templates" }` → the same kind of notice as for agents.
- Everything else is ignored.

**Mouse** (`on_mouse`, Canvas mode only):

| `CanvasEvent` | Effect |
|---|---|
| `Select(id)` | Selects. |
| `EditNode(id)` | Selects, then the NodeForm on it. |
| `OpenLinks(edge)` | `Links::on_edge`. |
| `Connect { from, to }` | `Mode::EdgeForm(EdgeForm::new(from, to))`, refused while busy. |

`on_mouse` returns `Consumed` when an event fired or the viewport moved.

### T6 tests (write first; `mod.rs` `#[cfg(test)]`)

Bench: a local `Bench` mirroring `testkit.rs:747-881`:
- Fields: `Scope` (demo Graphics), `Emit`, `Keys::compiled()`, `Theme`, `TopBarState`.
- `ctx()` builds `Ctx::new(.., Origin::Tab(SettingsTab::ID), ..)`.
- `key(&mut GraphEditor, "a")`, `reply(..)`, `drained()`.
- `served(&Backend, request)` serves a request through `crate::store_worker::serve(&Backend::memory(MemStore::demo()), ..)` for real outcomes.

Tests:
1. `opening_asks_for_agents_and_templates`
2. `each_scope_saves_its_own_graph_and_revision`: literal Project (rev 4), Workspace (rev 7) and Library (rev 9) bodies. Rename a node through `e`, type, Enter. Each emits `ReplaceGraph { graph: own id, expected_revision: own rev, .. }` (the PRD's scope decision, T6 per plan).
3. `a_refused_body_never_sends_a_request`:
   - Rename to `judge`; the notice is `reserved_phase_name("judge")`.
   - Connect a `fails` edge forward; the notice is `loop_edge_runs_forward`.
   - `drained()` holds no `Action::Store`.
4. `a_second_edit_is_refused_while_one_is_in_flight`
5. `applied_adopts_the_body_and_the_header_names_the_undo`: the header holds `u undoes: editing research`.
6. `undo_sends_the_previous_body_on_the_current_revision_and_redo_returns`
7. `stale_adopts_the_current_body_and_clears_undo_and_redo`: the notice is exactly `STALE_RELOADED`, and `u` then answers `nothing to undo`.
8. `failed_keeps_the_body_and_the_form_with_the_refusal`
9. `close_is_refused_while_busy_and_answers_close_otherwise`
10. `connect_picks_a_target_then_the_edge_form_then_one_write`
11. `delete_asks_then_rewires_in_one_write`: question text checked, C3.
12. `cycling_a_relation_in_the_links_list_opens_the_edge_form_unsaved`
13. `a_rebound_key_acts_and_the_hint_follows`: `load_str("[graph_editor]\nadd = [\"A\"]\n")`, then `A` adds and the hint reads `A add`.
14. `every_mode_names_its_stack`
15. `debug_never_prints_typed_text`
16. Snapshots via `render` into 100x30: `editor_canvas`, `editor_node_form`, `editor_links`, `editor_confirm`, `editor_connecting`.

Keys tests are the changed pins above.

**Validate**:
```bash
cargo test -p htui --lib keys:: -- --test-threads=1
cargo test -p htui --lib ui::graph_editor -- --test-threads=1
cargo test -p htui --features testkit --test keys_file --test no_hardcoded_keys -- --test-threads=1
```
Then the lint gate. The bare clippy run proves the allow is gone (C4).

**Commits**:
1. `feat(MOD-141): T6 key contexts graph_editor and graph_editor.links, 22 acts, six stacks` (catalogue, views, stack, load, keys_file, docs). The pins move here. `views::` names are satisfied in commit 2, so this commit may land together with commit 2 if `every_view_stack_is_named_by_a_view` cannot pass alone.
2. `feat(MOD-141): T6 GraphEditor: modes, writes, replies, render, mouse; remove dead_code allow`.

---

## T7 - Kinds hosting and retirement (D2, D3, D14)

**File set**:
- `crates/htui/src/ui/tabs/settings/kinds.rs`
- `crates/htui/src/ui/tabs/settings/mod.rs` (mouse, C5)
- `crates/htui/src/keys/catalogue.rs`
- `crates/htui/src/keys/views.rs`
- `docs/htui-keys.md`
- `crates/htui/tests/kinds.rs`
- `crates/htui/tests/snapshots/kinds__*.snap`

### Key

Row `row(Act::KindsEditor, SettingsKinds, "editor", &["enter"], "graph editor")`:
- Place it after `KindsGraph` in the `[settings.kinds]` block.
- `Act::KindsEditor` doc: ``/// `settings.kinds.editor`: open the graph editor on the row's graph (MOD-141 D3).``. Append it after `LinksClose`, at position **244**.
- `CATALOGUE.len()`: 244 → **245**.
- `KINDS_BROWSE` view own acts: add `Act::KindsEditor`.
- `the_view_stacks_resolve_as_designed`: add `assert_eq!(resolve(KINDS_BROWSE, "enter"), [Act::KindsEditor])`.
- `enter` is free in `[settings.kinds]` (N, g) and in `KINDS_BROWSE`: the `LIST` layer there is only `down`/`up` (`views.rs:70`), so `list.fold` is absent.
- Regenerate `docs/htui-keys.md`.

### Mode and host wiring (kinds.rs)

- `enum Mode` gains `Graph { editor: Box<GraphEditor> }`. Its `Debug` arm prints `f.debug_struct("Graph").field("editor", editor)` (the editor's Debug is text-free).
- Every exhaustive match on `Mode` gains the arm:
  - `on_editor_key` `:814`
  - `on_deleted` `:1187`
  - `on_stale` `:1212`
  - `stack` `:1385`: `Mode::Graph { editor } => editor.stack()`
  - `hint_text` `:1353`: `editor.hint_text(keys)`, returned early as `hierarchy.rs:554` does
  - `pane` `:1467`: empty
  - `Debug` `:344`
- `captures_input`: already `!matches!(.., Browse)`, which covers Graph.
- `on_key`: first, `if let Mode::Graph { editor } = &mut self.mode { match editor.on_key(key, ctx) { EditorKey::Handled(h) => h, EditorKey::Close => { self.close_editor(); Consumed } } }`. This is the `hierarchy.rs:1513-1522` shape.
- `on_paste`: `Mode::Graph` → `editor.on_paste(text)`.
- `takes_paste`: Graph → `editor.takes_paste()`.
- `render`: Graph → `editor.render(frame, area, ctx); return;` (whole section, D2).
- `scroll`: Graph → `None`.
- `key_hint`: through `hint_text`.
- `wants_requests`: `Catalogue(scope)`, plus `editor.wants_requests(scope)` when open.
- `on_scope_change`: the mode goes back to Browse (the editor drops with the scope, `kinds.rs:1554` rule).
- `absorbs_failure(request)`: `matches!(self.mode, Mode::Graph { .. }) && request == "replace_graph"`. The editor's notice says it; the status line does not repeat it.
- `on_reply` order when `Mode::Graph`:
  - `GraphReplaced { catalogue, .. }`: adopt the snapshot quietly (`self.snapshot = Some((**catalogue).clone()); self.unavailable = None; self.clamp_cursor();`), then `editor.set_personas(&catalogue.personas)`, then `editor.on_reply(reply, ctx)`.
  - `Catalogue(s)`: `self.on_catalogue(s, ctx)`. `busy` is `None` while the editor writes, so the mode is untouched. Then `editor.set_personas`.
  - `Failed { request: "replace_graph" | "agents" | "templates" }`, `Agents(_)`, `Templates(_)`: editor only.
  - Everything else: the existing arms.
  - With no editor open, `GraphReplaced` keeps T1's arm.
- New field-free helper `open_editor(&mut self, row: Row, form: bool, ctx: &Ctx<'_>)`. It is refused when `blocked()`.

  | Row | Effect |
  |---|---|
  | `Graph { p, g }` | Opens on that entry. |
  | `Phase { p, g, i }` | Opens on the graph and `editor.select(phase id)`; with `form` also `open_node_form()`. |
  | `Kind { p, k }` | Opens the kind's default graph when it is a project graph (`locate_graph`). Otherwise refuse with the `open_graph` sentences (`kinds.rs:701-711`): `GRAPH_MISSING` / `graph {label} is not edited here`. |
  | `Project` | Refuse `PICK_A_GRAPH = "select a graph or a step: {Enter} opens the graph editor"` (the key label from `KINDS_BROWSE`). |

  Then `GraphEditor::open(entry.clone(), snapshot.personas.clone(), ctx.scope)`, `ctx.request` each returned request, `self.notice = None`, and `self.mode = Mode::Graph { editor: Box::new(editor) }`.
- `close_editor`: Browse. The cursor goes to the first row showing that graph: the first `Kind` whose `default_graph_id` is it, else its `Graph` row. `clamp_cursor`.
- Browse keys:
  - `Act::KindsEditor` → `open_editor(row, false, ctx)`.
  - `Act::Edit` on a `Phase` row → `open_editor(row, true, ctx)` (D14, `e` opens it on the node). On every other row it is unchanged.
  - `Act::New` on `Graph` or `Phase` rows → refuse `STEPS_IN_THE_EDITOR = "steps are added and deleted in the graph editor: {Enter} opens it"`. Project and Kind rows keep `NewKind`.
  - `Act::Delete` on a `Phase` row → the same `STEPS_IN_THE_EDITOR`. On a `Graph` row → `NOT_DELETED_HERE = "graphs are not deleted here"` (the constant is reworded). Kind rows are unchanged.
- `HINT_BROWSE`: `n "kind"` (was "kind/phase"); insert `Hint::One(Act::KindsEditor, "graph editor")` after `KindsGraph`.
- `detail_lines`: drop ` \u{2014} {MOD_4_OWNS}`, and delete `MOD_4_OWNS`.

### Retired from kinds.rs (dead after the phase forms go; bare clippy refuses leftovers)

- `EditorKind::NewPhase`, `EditorKind::EditPhase`, and their arms in `build` (`:1021-1034`), `reload` (`:1994-2006`), `open_new_child` (`:601-608`) and `open_edit` (`:662-680`).
- `FollowUp` and `Built`'s second element: `type Built` becomes `StoreRequest`, or keep the tuple with a unit-free type. Also remove `Editor.follow_up`, `follow_up()` (`:1142-1177`) and the `on_catalogue` follow-up branch (`:1529-1531`).
- `Stored.budget`, `Stored.persona`, `Editor.stored_budget`, `Editor.stored_persona`, and `open_with`'s handling of them.
- `build_phase_edit`, `new_phase_fields`, `edit_phase_fields`, `budget_text`, `input_kinds`, `budget`, `persona_patch`, `persona_text`, `no_persona_named`, `NO_PERSONAS`, `BUDGET_IS_A_NUMBER`, `GATE_IS_Y_OR_N`, and `phase_by_id` if unused.
- Kept: `persona_label` and `budget_label` (both used by `phase_line`), `personas()`, `doing`'s phrases (`REQUEST_NAMES` test), and `super::yes_or_no` (Hierarchy uses it).
- The unit test `a_confirm_prefix_never_prints_the_typed_prefix` (`:2173`): drop the removed fields from its `Editor` literal.

### Mouse (settings/mod.rs, C5)

`SettingsSection` gains:
```rust
/// Whether this section's own gesture wants the mouse now (MOD-141 D11). Defaulted to `false`.
fn wants_mouse(&self) -> bool { false }
/// A mouse event, only while `wants_mouse`. Defaulted to `Pass`.
fn on_mouse(&mut self, _mouse: MouseEvent, _ctx: &mut Ctx<'_>) -> Handled { Handled::Pass }
/// The mouse went away mid-gesture. Defaulted to nothing.
fn on_mouse_lost(&mut self) {}
```
`impl Tab for SettingsTab` gains:
- `wants_mouse`: the active section's.
- `on_mouse`: the active section's.
- `on_mouse_lost`: every section's.

Kinds implements the three for `Mode::Graph` by forwarding to the editor.

### Tests (tests/kinds.rs)

**Retire** these phase-form section tests, which test behaviour that no longer exists:
- `e_on_a_phase_opens_seven_fields_and_enter_sends_update_phase` (`:2267`)
- `a_budget_only_change_sends_set_phase_budget` (`:2348`)
- `clearing_the_budget_sends_none` (`:2381`)
- `a_non_number_budget_is_refused` (`:2426`)
- `a_patch_and_budget_change_is_two_writes_in_order` (`:2449`)
- `a_stale_budget_follow_up_is_retried_by_enter` (`:2526`)
- `a_gate_hard_that_is_not_y_or_n_is_refused` (`:2675`)
- `a_typed_persona_name_binds_the_phase` (`:2775`)
- `a_persona_and_budget_change_sends_the_patch_then_the_budget` (`:2791`)
- `blanking_a_bound_persona_sends_some_none` (`:2821`)
- `an_untouched_bound_persona_sends_none` (`:2843`)
- `an_unknown_persona_name_is_refused_before_sending` (`:2858`)
- `with_no_personas_the_refusal_says_add_one` (`:2877`)
- the editor half of `an_id_the_list_does_not_know_prints_as_the_id_and_is_left_alone` (`:2902`); keep its phase-line assertion
- helpers left unused: `edit_research`, `focus_persona`, `persona_field`, `the_update_phase`, `research`, if unused

Keep these worker tests (the requests still exist):
- `create_phase_is_the_seeders_row_plus_five_columns`
- `update_phase_replaces_input_kinds_whole`
- `set_phase_budget_sets_then_clears`
- `delete_phase_rereads_the_catalogue`

**Rewrite**:
- `n_on_a_phase_creates_in_its_graph` (`:2625`) → `n_on_a_graph_or_phase_points_at_the_graph_editor` (`STEPS_IN_THE_EDITOR` with `Enter`; nothing emitted).
- `d_on_a_phase_or_graph_is_refused` (`:3082`): the phase row gives `STEPS_IN_THE_EDITOR`, the graph row gives `"graphs are not deleted here"`.
- `the_selected_phase_shows_the_read_only_line` (`:1281`) and `the_read_only_line_names_its_columns_in_words` (`:3382`): no `MOD-4 owns these` tail.

**New** (write first):
1. `enter_on_a_graph_row_opens_the_editor_and_asks_for_agents_and_templates`: requests `[Agents, Templates(scope)]`, `key_stack == GRAPH_EDITOR`.
2. `enter_on_a_phase_row_opens_the_editor_on_that_step`
3. `e_on_a_phase_row_opens_its_step_form_in_the_editor`
4. `enter_on_a_kind_opens_its_project_graph_and_refuses_a_shared_one`
5. `while_the_editor_is_open_h_and_l_do_not_cycle_sections`: drive `SettingsTab` and check `captures_input`.
6. `closing_the_editor_returns_to_browse_with_the_cursor_on_the_graph`
7. `a_graph_replaced_reply_updates_the_tree_and_the_editor`
8. `a_scope_change_closes_the_editor`
9. `the_hint_row_is_the_editors_while_it_is_open`
10. `the_settings_tab_hands_the_mouse_to_the_open_editor_only` (`wants_mouse` false in Browse)
11. Snapshots: `kinds__graph_editor` (the analysis graph open, 100x30), `kinds__graph_editor_step` (opened on `verdict`).

### Snapshot impact

The full insta run is the authority (memory: snapshot impact needs a full insta run). Expected:

| Snapshot | Change |
|---|---|
| `kinds__demo.snap` | hint row: `n kind`, `Enter graph editor` |
| `kinds__demo_80.snap` | hint row, likely cut |
| `kinds__phase_detail.snap` | tail gone |
| `kinds__editor_phase.snap` | deleted with its test; remove the file, or `cargo insta test --unreferenced=delete` |
| `kinds__graph_editor.snap` | new |
| `kinds__graph_editor_step.snap` | new |
| `kinds__phase_persona.snap`, `kinds__stale.snap`, `kinds__delete_ask.snap`, `kinds__prefix_warn.snap`, `kinds__no_workspace.snap`, `kinds__offline.snap` | expected unchanged; confirm |

Any Harness snapshot elsewhere that shows Settings › Kinds' hint row moves too.

**Validate**:
```bash
cargo test -p htui --features testkit --test kinds -- --test-threads=1
cargo test -p htui --lib keys:: -- --test-threads=1
cargo test -p htui --lib ui::tabs::settings -- --test-threads=1
cargo insta test -p htui --features testkit --unreferenced=reject -- --test-threads=1
```
If `cargo-insta` is absent, run `INSTA_UPDATE=new cargo test -p htui --features testkit -- --test-threads=1` and review every `.snap.new`. Review every changed snapshot. Then the lint gate.

**Commits**:
1. `feat(MOD-141): T7 settings.kinds.editor and Mode::Graph hosting the editor`
2. `refactor(MOD-141): T7 retire the position-based phase forms; pointers to the editor; detail line`
3. `feat(MOD-141): T7 Settings forwards the mouse to a section (graph editor canvas)`
4. `test(MOD-141): T7 kinds snapshots`

---

## T8 - End-to-end acceptance (PRD metrics)

**File set**: `crates/htui/tests/graph_editor.rs` (new, `#![cfg(feature = "testkit")]`) and its snapshots.

**Drivers**:
- `harness()`: `Harness::over(store.clone())` with the Settings tab and its sections, as in `tests/kinds.rs::kinds_over` (`:1128`), cycled to Kinds. It routes real replies through the shell's fan-out. `MemStore` clones share state (`mem.rs:128-130`), so assertions read `store.graph_body(..)`.
- `bench_round(bench, section, backend)`: drains `Action::Store` requests, serves each with `store_worker::serve(&backend, &req)` and feeds the reply back. Repeat until nothing is emitted. This is the `tests/kinds.rs` worker+section pattern without a shell.

**Scenarios**:
1. `the_reference_graph_is_built_in_the_tui_and_resolves` (Harness):
   1. `N` creates graph `reference` (type the name, Enter), then move the cursor to its Graph row and press Enter: the editor opens (empty).
   2. `a`, name `implement`, choose a template, Enter.
   3. `a` (implement selected), `review`, Enter: `implement -next-> review`.
   4. `a` (review selected), `fix`, Enter: `review -next-> fix`.
   5. `i` on fix, select `← review · next`, `x`.
   6. Select implement, `c`, pick fix, Enter. In the edge form, `l` to `if`, Tab, type `{"verify":"failed"}`, Enter.
   7. Select review, `c`, pick implement, Enter. Cycle to `changes requested`, Tab, type `3`, Enter.
   8. `Esc` closes the editor. `e` on kind ANA sets its graph to `reference` (existing kind editor).
   9. Assert the stored body: 3 nodes and exactly the three edges (kinds, predicate, limit).
   10. Assert `store.resolve_graph(<a demo Vulkan ANA item>)` is `Some` with phases implement, review, fix.
   11. Snapshot the final editor frame.
2. `every_m1_node_field_round_trips` (table-driven, bench): for each of name, template, version, persona, gate, hard gate, retry limit, isolation, commands, verify, token budget, inputs, output, and candidates (add, reorder, remove): edit in the form, save, re-read `graph_body`, and compare that column. The other columns are unchanged.
3. `every_edge_field_round_trips`: relation (every kind valid on a suitable pair), `if` predicate, loop limit (set, change, clear on `asks_you`).
4. `undo_and_redo_across_three_edits`: three edits, `u` x3 (body equals the seed, revision +3 more), `U` x3 (body equals after-edit-3).
5. `a_stale_reply_clears_the_stacks`: two edits, then a foreign `writer.replace_graph` bumps the revision, then a third edit. The notice must be `changed elsewhere, reloaded; undo history cleared`, `u` must answer `nothing to undo`, and the body must equal the foreign write.
6. `a_refused_body_never_sends_a_request`: a reserved name, a forward `fails`, and `if {"bogus":1}`. Each shows the store's sentence; no `ReplaceGraph` is drained; the revision is unchanged.
7. `the_editor_keys_are_catalogue_keys`: a rebound key file (`[graph_editor] add = ["A"]`, `[graph_editor.links] delete = ["D"]`) drives the same add and link delete. Together with `no_hardcoded_keys`, this covers the PRD's "0 hardcoded keys" metric.

**Validate**:
```bash
cargo test -p htui --features testkit --test graph_editor -- --test-threads=1
```
Then the milestone validation below.

**Commits**:
1. `test(MOD-141): T8 reference graph built in the TUI and resolved`
2. `test(MOD-141): T8 field round trips, undo/redo, stale, refusals, rebound keys`

---

## Milestone validation (after T8, on `hr/MOD-141`)

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui --features testkit --test kinds --test graph_editor --test keys_file --test no_hardcoded_keys -- --test-threads=1
bash scripts/gate.sh        # Qdrant/Postgres-live failures in the sandbox are owed on the host
```

---

## Open risks

1. **Inserted steps sort last.** D10's `position = max + 1` puts an inserted middle step last in the Kinds tree. Any position-ordered view also shows it last, for example the Runs flow's layers, which `execution_graph.rs:88-127` groups by `run_step.position`. Edges still define the walk. Decide in review whether M1 renumbers positions to flow order on write; that would be a D10 change, not made here.
2. **rataflow side loops.** The side-handle loops and the `H_GAP = 6` room are taken from the spike, untested in this crate. The T4 snapshots are the check, and the constants are T4's to tune.
3. **Mouse drag from a handle.** The `bottom` handle drag and the `connection_radius` follow the spike. If rataflow ignores a drag on a connectable node, the keyboard connect path stays complete (PRD risk).
4. **Undo of a delete restores less than was deleted.** It restores the node and its edges, but not skill bindings, deadline or judge (D6, C3). The question says so; M2 widens `GraphBody`.
5. **Templates for workspace and library graphs** come from every scope project. MOD-124 may want the library's own list.
6. **Key-pin churn between T6 and T7.** They run serially. T7 re-runs `keys::` and regenerates `docs/htui-keys.md`.
7. **Worktree builds.** Five wave-1 worktrees build concurrently. The sandbox Postgres crash loop is disk-related: check `df -h .` before fan-out. Merge serially and re-validate each on the merged tree, because `.snap` files under one directory and one crate couple the builds.
