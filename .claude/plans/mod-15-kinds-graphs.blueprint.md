# Blueprint: MOD-15 milestone 4 — kinds and graphs are editable

**Plan**: `.claude/plans/mod-15-kinds-graphs.plan.md` (D1–D18, D17b, F-1…F-22 are binding; this
file does not re-open any of them). **Shape**: `.claude/plans/mod-15-hierarchy-section.blueprint.md`
(milestone 3), one section across. **HEAD**: `9be844b`, branch `main`.
**Read notes**: graphify (`graphify-out/`) was queried first for "where is the catalogue read /
who serves a section's request"; the BFS came back too broad to be useful (361 nodes for one
question), so every coordinate below was taken by reading the file at HEAD. Bare filenames follow
the plan's convention (`crates/htui/src/…`, `crates/htui-core/src/…`).

**Constraints carried verbatim from the plan**: no `WriteStore` method added (conformance `CASES`
stays 36); no migration; no `query!`/`.sqlx` change; a section holds no store handle and no
`UserId`/`BoxId`; nothing sets `updated_at` by hand; `Debug` never prints a field's text;
`unsafe_code = "forbid"`, MSRV 1.98; tests first (TDD). Where the plan and PRD D2/D6/D10/D12,
M1 D3/D8 or M3 D5/D7 disagree, those win.

Every code block below is the shape to write, not text to paste; names, field order, strings and
signatures are load-bearing, comments and doc sentences may be reworded to taste.

---

## 0. Choices this blueprint settles inside the plan's decisions

These are not new decisions; each is the one concrete reading of a plan decision that the code
needs. An implementer who disagrees with one of these raises it before writing, not after.

| # | Choice | Under | Why this reading |
|---|---|---|---|
| B-1 | `SetPhaseBudget.budget: Option<i64>` (plan D6) is passed to `set_setting` as `serde_json::Value::from(budget)`. The **store** refuses an out-of-range number with its own sentence (`mem.rs:2318-2460`: `i32::try_from` on the Phase rung → `Constraint("… does not fit `step_graph_phase.token_budget`, which is INTEGER")`, and `validate` refuses `< 1`). The section parses the field with `str::parse::<i64>()` and refuses only non-numbers. | D6, D16, F-5 | The seam is the one validator (M1 D7). The section never re-implements min/max. |
| B-2 | `set_setting`'s `expected` is `Option` and `clear_setting`'s is plain (F-5): the worker passes `Some(expected)` to set and `expected` to clear. `Some` is always sent — the Phase rung refuses `None` with `expected_on_row(key, "step_graph_phase")` (`mem.rs`), and the section always has a row token for an existing phase. | D6, D8 | The asymmetry lives in the worker arm, never in the request type. |
| B-3 | **`token_budget` is an edit-only field.** The new-phase editor has the five columns D9 names (`name`, `position`, `template_name`, `gate_hard`, `input_kinds`); a phase is born with `token_budget = None` (`inherit`, the seeder's default) and the sixth column is set by `e` afterwards. The edit-phase editor has six. | D9, D16 | D9 lists exactly five request columns; a create followed by a budget write would need the new row's id, which only the reply knows. |
| B-4 | **A phase edit that changes both the patch columns and the budget is two writes on one `Enter`, chained by the section**: `UpdatePhase` first; when its `Catalogue` reply lands with `busy == Some("update_phase")` and the editor carries a `FollowUp::Budget`, the section re-takes the phase's `updated_at` from the fresh snapshot and sends `SetPhaseBudget`; the editor closes on *that* reply. A budget-only change sends `SetPhaseBudget` alone; a patch-only change sends `UpdatePhase` alone. | D6, D8, M3 D7 (one write in flight) | `busy` is one slot (M3 review H-1); two requests of different variants on the same tick are both delivered (F-1) but the second's CAS token is stale by construction, so it cannot be sent before the first's reply. |
| B-5 | **Whole patch on edit**, as M3 does (`settings/hierarchy.rs:874-878`, `:903-906`): every editable column goes out as `Some(text)` whether or not it changed. The only comparison the section makes is the one D10 needs (`prefix` field vs stored prefix) and the one B-4 needs (budget field vs stored budget). | D8, D10 | One habit across both sections; CAS makes it safe. |
| B-6 | `Row` has four variants: `Project`, `Kind`, `Graph` (a graph no kind points at) and `Phase { p, g, i }`. A phase under a kind and a phase under an unreferenced graph are the **same** variant — both carry the graph's index, and every key on a phase row acts on the graph. Indentation is the only difference and it is decided at line time. | D4 | The key tables need "which graph" and never "which parent". |
| B-7 | The kind editor's `graph` field is the **graph's name**, resolved among the project's graphs at submit; no match is refused with `` `graph` names no graph in this project ``. New kind: prefilled with the first graph by name when the project has one, else empty. | D4, D5 | `CreateKind.graph: StepGraphId` — the user types a name; the store checks `graph_not_in_project` again (`mem.rs:1875`). |
| B-8 | `position` prefills: new kind → `max(position) + 1` over the project's kinds (`0` when none); new phase → `max(position) + 1` over the graph's phases. Parsed with `str::parse::<i32>()`; a non-number is refused with `` `position` is a whole number ``. | D5 | Positions are unique per graph (`mem.rs:2118-2120`); the prefill is the one value that cannot collide. |
| B-9 | D13 parse of `input_kinds` is `text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()`; an empty field is `vec![]` and is sent (replaced whole). Rendered back as `join(",")`. | D13 | Verbatim from the decision. |
| B-10 | `gate_hard` field label is `gate_hard (y/n)`, parsed by the promoted `yes_or_no`; a non-answer is refused with `` `gate_hard (y/n)` is y or n `` (M3's sentence shape, `settings/hierarchy.rs:883`). | D14 | One convention. |
| B-11 | The **detail line** (D15) is not a `Row`: `lines()` appends one dimmed line after the selected phase row only. The cursor never lands on it, so `cursor == line index` for every row up to and including the selection. | D15 | Keeps `Row` and `cursor` simple; the pane's `Min(3)` already scrolls. |
| B-12 | `on_scope_change` drops the snapshot, the cursor, the mode and `busy`, keeps the notice — M3's rule word for word. `wants_requests` answers `vec![StoreRequest::Catalogue(scope.clone())]`. | D2, D7 | Same reason as M3 D5: a CAS token from another scope must not survive a switch. |
| B-13 | Project rows are read-only here: `e`/`d` on a project row set the notice `projects are edited in Hierarchy`; `n` on one opens a new-kind editor for that project; `N` on any row opens a new-graph editor for the row's project. | D4, D5 | The hierarchy section owns projects; this one owns what is inside them. |
| B-14 | Every request the section sends carries `ctx.scope.clone()` (`Ctx.scope: &Scope`, `app/state.rs:70`). | D7 | The reply must re-read what the section renders. |

---

## 1. `crates/htui/src/catalogue.rs` (new, T1) — the worker half

Mirrors `hierarchy.rs` one concept across: types → `snapshot` → `serve` → `reread`/`cas` →
`REQUEST_NAMES` → tests. No `UserId`, no `BoxId`, no `Backend::this_user`/`box_info` call anywhere
in this file (D1).

### 1.1 Module doc (intent)

`//! The catalogue behind Settings > Kinds: every project of the scope with its kinds, its graphs
and each graph's phases, read whole per event (M3 D5's trade) and written through milestone 1's
compare-and-set seam. One request in, one reply out; the section renders from the snapshot and
never patches a row into it (D3).`

### 1.2 Types (D3)

```rust
/// One read of the scope: one entry per `scope.project_ids`, in scope order; a project id that
/// names no row is skipped (a torn read, not a state to render).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueSnapshot {
    pub projects: Vec<ProjectCatalogue>,
}

/// One project with everything the Kinds section shows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCatalogue {
    pub project: Project,
    /// `item_kinds(project)`: by `position`, then `prefix`.
    pub kinds: Vec<ItemKind>,
    /// `step_graphs(project)`: by `name`.
    pub graphs: Vec<GraphEntry>,
}

/// One graph and its phases by `position`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEntry {
    pub graph: StepGraph,
    pub phases: Vec<StepGraphPhase>,
}
```

`Project`, `ItemKind`, `StepGraph`, `StepGraphPhase` all derive `PartialEq, Eq` today (check
`model/hierarchy.rs`, `model/kind.rs` before deriving; if one does not, drop `PartialEq, Eq` from
the three snapshot types and compare fields in tests — do not add derives to `htui-core` for a
test's convenience).

`impl ProjectCatalogue { pub fn graph(&self, id: StepGraphId) -> Option<&GraphEntry> }` — linear
search by `graph.id`; used by the section's row tree (D4) and by tests.

### 1.3 `snapshot`

```rust
/// Assembles the scope's catalogue. N+1 by design: per event, never per keystroke.
pub async fn snapshot<S: ReadStore + ?Sized>(store: &S, scope: &Scope) -> Result<CatalogueSnapshot>
```

Body: `for id in &scope.project_ids { let Some(project) = store.project(*id).await? else { continue }; let kinds = store.item_kinds(*id).await?; let mut graphs = Vec::new(); for graph in store.step_graphs(*id).await? { let phases = store.phases(graph.id).await?; graphs.push(GraphEntry { graph, phases }); } projects.push(ProjectCatalogue { project, kinds, graphs }); }`.
Bound is `ReadStore` only (all four reads are on `ReadStore`; check `traits.rs:472`, `:508`,
`:537` and `project(id)` — if `item_kinds`/`step_graphs`/`phases` turn out to be on `WriteStore`'s
side of the seam, widen to `ReadStore + WriteStore + ?Sized` as `hierarchy::snapshot` does).

### 1.4 `serve`

```rust
/// Serves one catalogue request. Every arm ends in a fresh `snapshot` of the request's scope.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>
```

First line, as `hierarchy::serve`: `let writer = backend.writer().ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?;` — `Catalogue` (a read) goes through the same writer handle so `Offline` refuses it identically (the section's `unavailable` path).

| Arm | Seam call | Then |
|---|---|---|
| `Catalogue(scope)` | — | `reread(writer, scope)` |
| `CreateKind { scope, project, prefix, name, description, graph, position }` | `writer.create_item_kind(NewItemKind { id: ItemKindId::new(), project_id: *project, prefix, name, description, default_graph_id: *graph, position })` | `reread` |
| `UpdateKind { scope, id, expected, patch }` | `writer.update_item_kind(*id, *expected, patch.clone())` | `cas(writer, scope, &outcome)` |
| `DeleteKind { scope, id }` | `writer.delete_item_kind(*id)?` | mirror rebuild (§1.6) then `Ok(StoreReply::KindDeleted { mirror, catalogue: Box::new(snapshot(writer, scope).await?) })` |
| `CreateGraph { scope, project, name, description }` | `writer.create_step_graph(NewStepGraph { id: StepGraphId::new(), project_id: *project, name, description })` | `reread` |
| `UpdateGraph { scope, id, expected, patch }` | `writer.update_step_graph(*id, *expected, patch.clone())` | `cas` |
| `CreatePhase { scope, graph, name, position, template_name, gate_hard, input_kinds }` | §1.5 row, then `writer.create_phase(&row)` | `reread` |
| `UpdatePhase { scope, id, expected, patch }` | `writer.update_phase(*id, *expected, patch.clone())` | `cas` |
| `SetPhaseBudget { scope, phase, expected, budget: Some(n) }` | `writer.set_setting(SettingRung::Phase(*phase), SettingKey::TokenBudget, Value::from(*n), Some(*expected))` | `cas` |
| `SetPhaseBudget { scope, phase, expected, budget: None }` | `writer.clear_setting(SettingRung::Phase(*phase), SettingKey::TokenBudget, *expected)` | `cas` |
| `other` | — | `Err(StoreError::Backend(format!("not a catalogue request: {}", other.name())))` |

`SettingKey` is `htui_core::prompt::SettingKey` (re-exported at `prompt/mod.rs:42`); `Value` is
`serde_json::Value` (`htui-core` already depends on it; `htui` may need `serde_json` in
`[dependencies]` — check `crates/htui/Cargo.toml` first; it is a workspace dep).

### 1.5 `CreatePhase` row construction (D9, F-6)

```rust
const BLANK: PhaseSeed = PhaseSeed { name: "", input_kinds: &[], gate_hard: false };
let mut row = seed::phase_row(PhaseId::new(), *graph, *position, &PhaseSeed { gate_hard: *gate_hard, ..BLANK }, Utc::now());
row.name = name.clone();
row.output_kind = name.clone();          // the seeder's rule (D17b)
row.template_name = template_name.clone();
row.input_kinds = input_kinds.clone();
```

Exactly four text columns overwritten (`name`, `output_kind`, `template_name`, `input_kinds`) plus
`gate_hard` through the seed; `fan_out`, `gate`, `retry_limit`, `isolation`, `command_queue`,
`verify_command`, `template_version`, `token_budget` are whatever `phase_row` wrote and are never
named in `catalogue.rs`. `create_phase` ignores the row's `updated_at` (`traits.rs:517` doc;
`mem.rs:2145`), so the `now` handed to `phase_row` is not "setting `updated_at` by hand". If
`PhaseSeed` has private fields or no `..` construction, build the whole literal — the three fields
are `pub` at `seed.rs:30-38`.

### 1.6 `DeleteKind` mirror (D12)

Copy of the `DeleteProject` arm (`hierarchy.rs:333-350`): after the delete,
`let mirror = match backend.cache() { Some(cache) => match cache.rebuild().await { Ok(()) => MirrorAfterDelete::Rebuilt, Err(e) => MirrorAfterDelete::Failed(e.to_string()) }, None => MirrorAfterDelete::NoMirror };` —
read the `DeleteProject` arm for the exact `NotNeeded` condition and reproduce it; `MirrorAfterDelete` is
`use crate::hierarchy::MirrorAfterDelete;`, not re-declared.

### 1.7 `reread`, `cas`

```rust
async fn reread(writer: &Writer, scope: &Scope) -> Result<StoreReply> {
    Ok(StoreReply::Catalogue(Box::new(snapshot(writer, scope).await?)))
}

async fn cas<T>(writer: &Writer, scope: &Scope, outcome: &CasOutcome<T>) -> Result<StoreReply> {
    let fresh = Box::new(snapshot(writer, scope).await?);
    Ok(match outcome {
        CasOutcome::Applied(_) => StoreReply::Catalogue(fresh),
        CasOutcome::Stale(_) => StoreReply::CatalogueStale(fresh),
    })
}
```

(`Writer` is whatever type `backend.writer()` returns in `hierarchy.rs:169`; use the same import.)

### 1.8 `REQUEST_NAMES`

```rust
/// The nine names `StoreRequest::name()` answers for this module's variants, in variant order.
pub const REQUEST_NAMES: [&str; 9] = [
    "catalogue", "create_kind", "update_kind", "delete_kind", "create_graph",
    "update_graph", "create_phase", "update_phase", "set_phase_budget",
];
```

### 1.9 In-module tests (`#[cfg(test)]`)

Small and store-free, as `hierarchy.rs`'s are: `snapshot_skips_a_project_that_names_no_row` can
live here over `MemStore::demo()` if `hierarchy.rs` does the same; otherwise every test is in
`tests/kinds.rs` (§3). Do not duplicate.

### 1.10 `crates/htui/src/lib.rs`

Insert `pub mod catalogue;` in alphabetical position among `lib.rs:12-24` (after `app`, before
`cli`), with a one-line doc comment matching its neighbours' style.

---

## 2. `crates/htui/src/store_worker.rs` edits (T1)

### 2.1 Imports (`:14-33`)

`use crate::catalogue::{self, CatalogueSnapshot};` beside the `hierarchy` import. `Scope`,
`ItemKindId`, `StepGraphId`, `PhaseId`, `ItemKindPatch`, `StepGraphPatch`, `PhasePatch` from
`htui_core::model` (all re-exported, `model/mod.rs:105-125`).

### 2.2 `StoreRequest` variants — insert after `DeleteProject` (`:329`), in this order (D5)

Doc-comment style: one sentence of intent, then the CAS sentence where there is a token, as
`:241-329` do.

```rust
    /// The catalogue of every project in the scope (MOD-15 M4 D2): kinds, graphs, phases. One
    /// request per event, never one per project — the staleness index keeps only the newest of a
    /// variant.
    Catalogue(Scope),
    /// Creates a kind in `project` with `graph` as its default graph. Answers [`StoreReply::Catalogue`].
    CreateKind {
        scope: Scope,
        project: ProjectId,
        prefix: String,
        name: String,
        description: String,
        graph: StepGraphId,
        position: i32,
    },
    /// CAS on `item_kind.updated_at` (M1 D3): `Stale` answers [`StoreReply::CatalogueStale`]. A
    /// prefix change touches no `item` and no counter (PRD D12).
    UpdateKind {
        scope: Scope,
        id: ItemKindId,
        expected: DateTime<Utc>,
        patch: ItemKindPatch,
    },
    /// Deletes a kind no item holds; a held kind is refused by the seam with its own sentence
    /// (PRD D6/D10). Answers [`StoreReply::KindDeleted`] with the mirror rebuilt (M4 D12).
    DeleteKind { scope: Scope, id: ItemKindId },
    /// Creates an empty graph in `project`. Answers [`StoreReply::Catalogue`].
    CreateGraph {
        scope: Scope,
        project: ProjectId,
        name: String,
        description: String,
    },
    /// CAS on `step_graph.updated_at` (M1 D3): `Stale` answers [`StoreReply::CatalogueStale`].
    UpdateGraph {
        scope: Scope,
        id: StepGraphId,
        expected: DateTime<Utc>,
        patch: StepGraphPatch,
    },
    /// Creates a phase from `seed::phase_row`'s frozen defaults plus these five columns (M4 D9).
    CreatePhase {
        scope: Scope,
        graph: StepGraphId,
        name: String,
        position: i32,
        template_name: String,
        gate_hard: bool,
        input_kinds: Vec<String>,
    },
    /// CAS on `step_graph_phase.updated_at` (M1 D3) over `PhasePatch`'s five columns.
    UpdatePhase {
        scope: Scope,
        id: PhaseId,
        expected: DateTime<Utc>,
        patch: PhasePatch,
    },
    /// `token_budget` on the `Phase` rung (M1 D8, M4 D6): `Some` is `set_setting`, `None` is
    /// `clear_setting` so the project/app rung answers. CAS on the phase's `updated_at`.
    SetPhaseBudget {
        scope: Scope,
        phase: PhaseId,
        expected: DateTime<Utc>,
        budget: Option<i64>,
    },
```

The enum's "no secret as a plain `String`" rule (`:60-66`) holds: every string here is a prefix, a
name or a description (F-18).

### 2.3 `name()` arms — insert after `:376`

```rust
            Self::Catalogue(_) => "catalogue",
            Self::CreateKind { .. } => "create_kind",
            Self::UpdateKind { .. } => "update_kind",
            Self::DeleteKind { .. } => "delete_kind",
            Self::CreateGraph { .. } => "create_graph",
            Self::UpdateGraph { .. } => "update_graph",
            Self::CreatePhase { .. } => "create_phase",
            Self::UpdatePhase { .. } => "update_phase",
            Self::SetPhaseBudget { .. } => "set_phase_budget",
```

### 2.4 `StoreReply` variants — insert before `Failed` (`:503`)

```rust
    /// The scope's catalogue, freshly read (MOD-15 M4 D2/D7).
    Catalogue(Box<CatalogueSnapshot>),
    /// A catalogue write missed its CAS token: the tree as it is now; the editor keeps its text and
    /// retries only on `Enter` (M4 D8, PRD D8).
    CatalogueStale(Box<CatalogueSnapshot>),
    /// A kind is gone and the mirror was rebuilt (or could not be) (M4 D12).
    KindDeleted {
        mirror: MirrorAfterDelete,
        catalogue: Box<CatalogueSnapshot>,
    },
```

F-11: every `StoreReply` match outside the worker carries `_ => {}`, so nothing else changes.

### 2.5 `try_serve` arm — insert after the hierarchy arm (`:752`), before `StoreState`

```rust
        // One arm of nine or-ed patterns, no guard: `try_serve`'s match has no wildcard, so a
        // guarded arm is E0004 (M3 F-12). `catalogue::serve` refuses anything else by name.
        StoreRequest::Catalogue(_)
        | StoreRequest::CreateKind { .. }
        | StoreRequest::UpdateKind { .. }
        | StoreRequest::DeleteKind { .. }
        | StoreRequest::CreateGraph { .. }
        | StoreRequest::UpdateGraph { .. }
        | StoreRequest::CreatePhase { .. }
        | StoreRequest::UpdatePhase { .. }
        | StoreRequest::SetPhaseBudget { .. } => catalogue::serve(backend, request).await,
```

`serve` (`:679`) wraps `try_serve`'s `Err` into `failed(request.name(), err)` already; nothing to
add there.

---

## 3. `crates/htui/tests/kinds.rs` — worker half (T1)

Header and helpers copied from `tests/hierarchy.rs:1-120`: `#![cfg(feature = "testkit")]`,
`fn demo() -> Backend { Backend::memory(MemStore::demo()) }`, plus

```rust
fn catalogue(reply: StoreReply) -> CatalogueSnapshot   // unwraps `Catalogue`, panics with the reply otherwise
fn stale(reply: StoreReply) -> CatalogueSnapshot       // unwraps `CatalogueStale`
fn refusal(reply: StoreReply) -> (&'static str, String) // unwraps `Failed { request, message }`
fn vulkan_scope() -> Scope { Scope { workspace_id: ids::WORKSPACE_GRAPHICS, project_ids: vec![ids::PROJECT_VULKAN] } }
async fn demo_catalogue(backend: &Backend) -> CatalogueSnapshot { catalogue(serve(backend, &StoreRequest::Catalogue(vulkan_scope())).await) }
fn kind<'a>(snapshot: &'a CatalogueSnapshot, id: ItemKindId) -> &'a ItemKind
fn phase<'a>(snapshot: &'a CatalogueSnapshot, id: PhaseId) -> &'a StepGraphPhase
```

| Test | Plan | Arrange | Assert |
|---|---|---|---|
| `catalogue_names_are_stable` | D5 | one of each of the nine variants, built with dummy values, in `REQUEST_NAMES` order | `name()` of each equals `REQUEST_NAMES[i]`; `REQUEST_NAMES.len() == 9` |
| `the_demo_catalogue_reads_back_the_seed` | D3, F-9 | `demo_catalogue` | one project, `slug == "vulkan-tutorials"`; `kinds.len() == 5` with prefixes `["ANA","FEAT","FIX","CLEAN","TOOL"]` in that (position) order; `graphs.len() == 5`; phases across graphs sum to `seed::PHASES_PER_PROJECT` (15); every kind's `default_graph_id` resolves through `ProjectCatalogue::graph` |
| `a_two_project_scope_answers_both_in_scope_order` | D2, F-22 | `let store = MemStore::demo(); let second = store.create_project(NewProject { id: ProjectId::new(), slug: "second".into(), name: "Second".into(), description: String::new(), created_by: ids::USER }).await?;` then `Backend::memory(store)`; scope `[second.id, PROJECT_VULKAN]` | two entries, slugs `["second", "vulkan-tutorials"]` (scope order, not position order); the second project's `kinds.len() == 5` (M2's seed) |
| `an_unknown_project_id_is_skipped` | D3 | scope `[ProjectId::new(), PROJECT_VULKAN]` | one entry, vulkan; no `Failed` |
| `create_kind_lands_and_rereads` | D5, D7 | `CreateKind { scope, project: PROJECT_VULKAN, prefix: "DOC", name: "docs", description: "", graph: GRAPH_VULKAN_TOOL, position: 5 }` | reply is `Catalogue`; `kinds.len() == 6`; the last kind is `DOC`/`docs` with `default_graph_id == GRAPH_VULKAN_TOOL` |
| `update_kind_applies_against_the_current_token` | D8 | read; `UpdateKind { id: KIND_VULKAN_FEAT, expected: kind.updated_at, patch: ItemKindPatch { name: Some("features".into()), ..Default::default() } }` | `Catalogue`; the kind's `name == "features"`; its `updated_at != expected` |
| `a_prefix_rename_leaves_existing_keys_alone` | D10, F-17, risk table row 2 | read; `UpdateKind` with `patch.prefix = Some("FT")` on `KIND_VULKAN_FEAT` | `Catalogue`; kind prefix `"FT"`; `backend` `item(ITEM …FEAT-1 id)`'s `key_prefix` (or `key`) still `"FEAT"` — locate the item through the fixture's id constant (`fixtures.rs`, the vulkan `FEAT-1 "Chapter 12 parity"` row) |
| `update_kind_with_a_stale_token_answers_stale` | D8 | `expected: kind.updated_at - Duration::seconds(1)` | reply is `CatalogueStale`; the snapshot inside still names the kind unchanged |
| `delete_kind_of_a_held_kind_is_refused_with_the_seam_sentence` | D11, F-7 | `DeleteKind { id: KIND_VULKAN_FEAT }` | `refusal` → `("delete_kind", msg)` with `msg == item_kind_is_held("FEAT", 1)` (vulkan has one `FEAT-1` item); a following `Catalogue` still has 5 kinds |
| `delete_kind_of_an_unreferenced_kind_reports_the_mirror` | D12 | `DeleteKind { id: KIND_VULKAN_ANA }` | `KindDeleted { mirror: MirrorAfterDelete::NoMirror, catalogue }` (a memory backend has no cache — confirm against what `tests/hierarchy.rs` asserts for `DeleteProject` on `demo()` and use the same variant); `catalogue.kinds.len() == 4`, no `ANA` |
| `create_graph_and_update_graph` | D5, D8 | `CreateGraph { name: "docs", description: "" }` then `UpdateGraph` on it with `StepGraphPatch { name: Some("documentation".into()), description: None }` | 6 graphs; the renamed graph present; a second `UpdateGraph` with the first token → `CatalogueStale` |
| `create_phase_is_the_seeders_row_plus_five_columns` | D9, D17b | `CreatePhase { graph: GRAPH_VULKAN_ANA, name: "triage", position: 7, template_name: "triage", gate_hard: true, input_kinds: vec!["verdict".into()] }` | the graph now has 3 phases; the new one has `name == output_kind == "triage"`, `template_name "triage"`, `gate_hard`, `input_kinds == ["verdict"]`, and the frozen eight equal a `seed::phase_row(.., &PhaseSeed { name: "", input_kinds: &[], gate_hard: true }, now)` row's (`fan_out`, `gate`, `retry_limit`, `isolation`, `command_queue`, `verify_command`, `template_version`, `token_budget`) |
| `create_phase_with_a_reserved_name_is_refused` | F-8 | `name: "judge"` | `refusal` → `("create_phase", reserved_phase_name("judge"))` |
| `update_phase_replaces_input_kinds_whole` | D13 | `UpdatePhase { patch: PhasePatch { input_kinds: Some(vec![]), ..Default::default() } }` on the analysis graph's second phase | `Catalogue`; `input_kinds.is_empty()`; `updated_at` moved |
| `set_phase_budget_sets_then_clears` | D6, D16, B-1, B-2 | `SetPhaseBudget { phase, expected: p.updated_at, budget: Some(60_000) }` → read → `SetPhaseBudget { expected: fresh, budget: None }` | after set: `token_budget == Some(60_000)`; after clear: `None`; a `Some(i64::MAX)` answers `Failed` whose message contains `does not fit`; a stale token answers `CatalogueStale` |
| `offline_refuses_every_catalogue_request_by_name` | D7 | `CacheStore::open` + `Backend::Offline { cache, since }` as `tests/hierarchy.rs` does | for each of the nine, `refusal` → `(name, DATABASE_UNREACHABLE)` |
| `serve_refuses_a_foreign_request_by_name` | §1.4 last arm | `catalogue::serve(&demo(), &StoreRequest::Hierarchy(WORKSPACE_GRAPHICS))` | `Err(StoreError::Backend(msg))`, `msg == "not a catalogue request: hierarchy"` |

`ItemKindPatch`/`StepGraphPatch`/`PhasePatch` derive `Default`? Check `model/kind.rs`; if not,
write all fields out (`None`) — do not add a derive in `htui-core`.

---

## 4. `crates/htui/src/ui/tabs/settings/kinds.rs` (new, T2) — the section

### 4.1 Constants (every user-visible string)

```rust
const NO_WORKSPACE: &str = "no workspace: nothing to list";
const NO_PROJECT: &str = "no project in scope";
const UNAVAILABLE: &str = "catalogue unavailable";
const GRAPH_MISSING: &str = "graph missing";                              // D4
const HINT_BROWSE: &str =
    "j/k · n kind/phase · N graph · e edit · g graph · d delete kind · r reload";
const HINT_NO_WORKSPACE: &str = "r reload";
const HINT_UNAVAILABLE: &str = "r reload";
const HINT_EDITING: &str = "Tab/Shift+Tab field · Enter save · Esc cancel";
const HINT_CONFIRM_PREFIX: &str = "y write · n/Esc back to the editor";   // D10
const HINT_DELETING: &str = "y delete · n/Esc stop";                        // D11
const CHANGED_ELSEWHERE: &str = "changed elsewhere since you opened it — reloaded; Enter retries against the current row";
const DELETED_ELSEWHERE: &str = "deleted elsewhere — the editor was closed";
const RELOADED: &str = "reloaded";
const NOT_DELETED_HERE: &str = "graphs and phases are not deleted here";   // D5
const PROJECTS_ELSEWHERE: &str = "projects are edited in Hierarchy";        // B-13
const MOD_4_OWNS: &str = "MOD-4 owns these";                                // D15
const NO_GRAPH_NAMED: &str = "`graph` names no graph in this project";     // B-7
const POSITION_IS_A_NUMBER: &str = "`position` is a whole number";         // B-8
const BUDGET_IS_A_NUMBER: &str = "`token_budget` is a whole number or empty"; // B-1
const GATE_IS_Y_OR_N: &str = "`gate_hard (y/n)` is y or n";                // B-10
```

Format helpers (exact output):

- `fn prefix_warning(old: &str, new: &str) -> String` → `format!("items keyed {old}-* keep their keys and their counter; the next item minted under this kind is {new}-1.")` (D10, verbatim).
- `fn delete_question(name: &str, prefix: &str) -> String` → `format!("delete kind {name} ({prefix})? a kind any item uses is refused. y delete · n/Esc stop")` (D11, verbatim; the hint line repeats `HINT_DELETING`).
- `fn deleted_notice(prefix: &str, mirror: &MirrorAfterDelete) -> String` → `format!("deleted kind `{prefix}`; {mirror}")` where `mirror` is M3's four strings verbatim (`settings/hierarchy.rs:741-744`): `mirror rebuilt` / `no mirror` / `no rebuild needed` / `mirror not rebuilt: {err}`.
- `fn in_flight(busy: &str) -> String` → `` format!("`{busy}` is still in flight") `` (M3's).
- `fn required(label: &str) -> String` → `` format!("`{label}` is required") `` (M3's).
- `fn budget_text(b: Option<i32>) -> String` → `b.map_or_else(String::new, |n| n.to_string())`; `fn budget_label(b: Option<i32>) -> String` → `inherit` or the number (D16).

### 4.2 Types

```rust
/// One line of the tree, by index into the snapshot; rebuilt from the snapshot on demand (D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Project { p: usize },
    Kind { p: usize, k: usize },
    /// A graph no kind points at (D4).
    Graph { p: usize, g: usize },
    /// `projects[p].graphs[g].phases[i]`, whichever parent it is drawn under (B-6).
    Phase { p: usize, g: usize, i: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    NewKind(ProjectId),
    EditKind(ItemKindId),
    NewGraph(ProjectId),
    EditGraph(StepGraphId),
    NewPhase(StepGraphId),
    EditPhase(PhaseId),
}

/// The second write of a phase edit that changed both patch columns and the budget (B-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowUp {
    Budget(Option<i64>),
}

struct Field { label: &'static str, input: TextField, required: bool }   // as M3, with `required`/`optional`/`text`

struct Editor {
    kind: EditorKind,
    fields: Vec<Field>,
    focus: usize,
    /// The row's CAS token; `None` for a create.
    expected: Option<DateTime<Utc>>,
    /// Kind edit: the stored prefix, for D10's comparison. Phase edit: unused.
    stored_prefix: Option<String>,
    /// Phase edit: the stored budget as text, for B-4's comparison.
    stored_budget: Option<String>,
    follow_up: Option<FollowUp>,
}
// `impl fmt::Debug for Editor` by hand: kind, focus, expected, follow_up, field labels — never a field's text.

#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    Editing(Editor),
    /// D10: the editor is kept whole so `n`/`Esc` return to it with its text.
    ConfirmPrefix { editor: Editor, old: String, new: String },
    /// D11: one confirmation, then the write.
    Deleting { id: ItemKindId, name: String, prefix: String, stage: DeleteStage },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteStage { Asking, InFlight }

pub struct KindsSection {
    snapshot: Option<CatalogueSnapshot>,
    unavailable: Option<String>,
    cursor: usize,
    mode: Mode,
    busy: Option<&'static str>,
    notice: Option<String>,
}
impl KindsSection { pub const ID: SectionId = SectionId("kinds"); pub fn new() -> Self; }
impl Default for KindsSection
```

`Editor.expected` is `Option` for the same reason M3's is (a create has none); the `EditKind` /
`EditGraph` / `EditPhase` submit paths treat `None` as `DELETED_ELSEWHERE`, as
`settings/hierarchy.rs:867-869` does.

### 4.3 Field layouts (order and required-ness)

| `EditorKind` | Fields (label → prefill) |
|---|---|
| `NewKind(p)` | `prefix`* → `""`; `name`* → `""`; `description` → `""`; `graph`* → first graph name or `""` (B-7); `position`* → B-8 |
| `EditKind(id)` | same five, prefilled from the row; `graph` prefilled with the default graph's name (or `""` when missing); `stored_prefix = Some(row.prefix)` |
| `NewGraph(p)` | `name`* → `""`; `description` → `""` |
| `EditGraph(id)` | same two from the row |
| `NewPhase(g)` | `name`* → `""`; `position`* → B-8; `template_name`* → `""`; `gate_hard (y/n)`* → `"n"`; `input_kinds` → `""` |
| `EditPhase(id)` | same five from the row (`input_kinds` joined with `,`, `gate_hard` as `y`/`n`), then `token_budget` → `budget_text(row.token_budget)`; `stored_budget = Some(that text)` |

`*` = required (empty text refused with `required(label)` before any request). Focus starts at 0.

### 4.4 Rows and lines (D4)

`fn rows(&self) -> Vec<Row>`: for each `(p, project)`: `Project { p }`; for each `(k, kind)` in
`project.kinds` (already by position): `Kind { p, k }`, then if `project.graphs` has an index `g`
with `graph.id == kind.default_graph_id`, `Phase { p, g, i }` for each phase; then for each `(g,
entry)` where no kind's `default_graph_id == entry.graph.id`: `Graph { p, g }` then its
`Phase { p, g, i }` rows.

`fn lines(&self, ctx) -> Vec<Line>` — one line per row, plus B-11's detail line:

| Row | Text | Style |
|---|---|---|
| `Project` | `{slug}  {name}` | `theme.base`; selected → `theme.selected` |
| `Kind` (graph found) | `  {prefix}  {name} · graph {graph.name}` | as above |
| `Kind` (graph missing) | `  {prefix}  {name} · graph missing` | the tail `graph missing` in `theme.error` |
| `Graph` | `  {name} · graph, no kind` | `theme.dim` unless selected |
| `Phase` | `    {position}. {name} · template {template_name} · gate {hard\|soft} · in {a,b\|none} · budget {inherit\|N}` | `theme.base` |
| detail (after the selected `Phase` only) | `      fan_out {n} · gate {gate.as_str()} · isolation {none\|as_str} · command_queue {as_str} · verify_command {none\|text} · retry_limit {n} · output_kind {text} · template_version {none\|n} — MOD-4 owns these` | `theme.dim`, never selectable |

`gate hard|soft` is `gate_hard`; the detail line's `gate` is the `Gate` enum. `none` for `None`
options. Description is not drawn on the tree (it is in the editor); if the reviewer wants it,
append ` — {description}` to the kind line when non-empty — not in the snapshots below.

`fn selected(&self) -> Option<Row>`, `move_cursor(delta)`, `clamp_cursor()` as M3.

### 4.5 `pane(width, theme) -> Vec<Line>` (below the rows)

| Mode | Lines |
|---|---|
| `Browse` | notice only (wrapped) |
| `Editing(e)` | `e.lines(width, theme)` (label-padded `label: [input]`, M3's `Editor::lines`), then notice |
| `ConfirmPrefix { old, new, .. }` | `prefix_warning(old, new)` wrapped, in `theme.error`, then notice |
| `Deleting { name, prefix, stage, .. }` | `delete_question(name, prefix)` in `theme.error`; `InFlight` appends ` · delete_kind in flight` in `theme.dim`; then notice |

### 4.6 Hint line

| State | Text |
|---|---|
| no scope projects (`snapshot.projects.is_empty()`) | `HINT_NO_WORKSPACE` with message `NO_PROJECT`; no snapshot and no `unavailable` → `NO_WORKSPACE` |
| `unavailable = Some(m)` | message `format!("{UNAVAILABLE}: {m}")` in error; hint `HINT_UNAVAILABLE` |
| `Browse` | `HINT_BROWSE` + ` · {busy} in flight` when `busy` is `Some` |
| `Editing` | `HINT_EDITING` |
| `ConfirmPrefix` | `HINT_CONFIRM_PREFIX` |
| `Deleting` | `HINT_DELETING` |

Notice wins over the hint when the pane cannot fit it (M3's `hint`), `is_error(notice)` → `theme.error`.

### 4.7 Key tables

**Browse** (`on_key`, after `captures_input == false` so the tab's cycle ran first):

| Key | Row | Effect |
|---|---|---|
| `j` / `Down`, `k` / `Up` | any | `move_cursor(±1)`; `Consumed` |
| `n` | `Project{p}` / `Kind{p,..}` | `open(NewKind(project.id))` |
| `n` | `Graph{p,g}` / `Phase{p,g,..}` | `open(NewPhase(graph.id))` |
| `N` | any | `open(NewGraph(project.id))` of the row's `p` |
| `e` | `Kind` | `open(EditKind(id))` |
| `e` | `Graph` | `open(EditGraph(id))` |
| `g` | `Kind` | `open(EditGraph(kind.default_graph_id))` — **D19**, the answer to H-6: a graph every kind points at has no `Graph` row, and `g` is how its name and description are reached. Notice `GRAPH_MISSING` when `default_graph_id` names no graph of the project |
| `g` | `Phase` | `open(EditGraph(graph.id))` of the graph the phase belongs to (D19) |
| `g` | `Graph` | `open(EditGraph(id))` — same editor `e` opens, so the key means one thing everywhere |
| `g` | `Project` | notice `PROJECTS_ELSEWHERE` |
| `e` | `Phase` | `open(EditPhase(id))` |
| `e` / `d` | `Project` | notice `PROJECTS_ELSEWHERE` |
| `d` | `Kind` | `begin_delete(id, name, prefix)` → `Mode::Deleting { stage: Asking }` (refuses with `in_flight` when busy) |
| `d` | `Graph` / `Phase` | notice `NOT_DELETED_HERE` |
| `r` | any | `ctx.request(Catalogue(ctx.scope.clone()))`; `busy` untouched (a read) |
| `Esc` | any | `notice = None` |
| anything else | | `Pass` |

`open` refuses with `in_flight(busy)` when `busy.is_some()` (M3 H-1 rule), and with nothing when
there is no snapshot.

**Editing** (`on_editor_key`): identical to M3 `:772-810` — `Tab`/`Down` next field, `BackTab`/`Up`
previous, `Enter` → `submit`, `Esc` → `Mode::Browse` (text dropped), `CONTROL` chords `Pass`,
everything else to `field.input.on_key` and `Consumed`.

**ConfirmPrefix**: `y` → `send(UpdateKind { … })` built from the kept editor, `mode = Editing(editor)` stays open until the reply (so `CatalogueStale` can re-take the token); `n` / `Esc` → `mode = Editing(editor)` with its text intact, no request; `CONTROL` chords `Pass`; every other key swallowed (`Consumed`).

**Deleting** (`on_deleting_key`, mirrors M3 `:679-735`): `Asking` + `y` → `send(DeleteKind { scope, id })`, `stage = InFlight`; `n` / `Esc` while `Asking` → `Browse`; while `InFlight` every key but `CONTROL` is swallowed; `CONTROL` → `Pass`.

### 4.8 `submit(ctx)` per editor kind

Common prelude: `if let Some(b) = self.busy { notice = in_flight(b); return }`; then the required
check over `fields` in order (first empty required → `required(label)`, return).

| Kind | Parse | Request |
|---|---|---|
| `NewKind(p)` | `graph` by name (B-7), `position` (B-8) | `CreateKind { scope, project: p, prefix: t(0), name: t(1), description: t(2), graph, position }` |
| `EditKind(id)` | `expected` else `DELETED_ELSEWHERE`; graph; position; build `patch = ItemKindPatch { prefix: Some(t(0)), name: Some(t(1)), description: Some(t(2)), default_graph_id: Some(graph), position: Some(position) }` (B-5) | if `t(0) != stored_prefix` → `mode = ConfirmPrefix { editor, old, new }` and **no request**; else `UpdateKind { scope, id, expected, patch }` |
| `NewGraph(p)` | — | `CreateGraph { scope, project: p, name: t(0), description: t(1) }` |
| `EditGraph(id)` | `expected` | `UpdateGraph { scope, id, expected, patch: StepGraphPatch { name: Some(t(0)), description: Some(t(1)) } }` |
| `NewPhase(g)` | position; `gate_hard` via `yes_or_no` (B-10); `input_kinds` via B-9 | `CreatePhase { scope, graph: g, name: t(0), position, template_name: t(2), gate_hard, input_kinds }` |
| `EditPhase(id)` | `expected`; position; gate; kinds; `budget = match t(5).trim() { "" => None, s => Some(s.parse::<i64>() else BUDGET_IS_A_NUMBER) }`; `budget_changed = t(5).trim() != stored_budget` | `patch = PhasePatch { name: Some(t(0)), position: Some(position), template_name: Some(t(2)), gate_hard: Some(gate), input_kinds: Some(kinds) }`; then: `budget_changed` only → `SetPhaseBudget { scope, phase: id, expected, budget }`; else → `UpdatePhase { scope, id, expected, patch }` with `editor.follow_up = budget_changed.then_some(FollowUp::Budget(budget))` (B-4) |

`send(request, ctx)`: `self.busy = Some(request.name()); ctx.request(request)` (M3 `:762-765`).

### 4.9 `on_reply` table

| Reply | Condition | Effect |
|---|---|---|
| `Catalogue(s)` | `busy == Some("update_phase")` and the open editor is `EditPhase(id)` with `follow_up = Some(Budget(b))` | replace snapshot, `clamp_cursor`; find phase `id` in `s`; `Some(row)` → `editor.expected = Some(row.updated_at)`, `follow_up = None`, `send(SetPhaseBudget { scope, phase: id, expected: row.updated_at, budget: b })`, editor stays open; `None` → `DELETED_ELSEWHERE`, `Browse`, `busy = None` |
| `Catalogue(s)` | otherwise | replace snapshot; `unavailable = None`; `clamp_cursor`; if `busy` was a write (`Some` and not `"catalogue"`) → `notice = None`, `mode = Browse`; if `busy` was none and the mode is `Browse` → `notice = RELOADED` only when the reply answered an `r` (track with a `reloading: bool` set by `r`, cleared here — or leave the notice untouched; M3's behaviour is the tie-break: read `on_tree` `:940-985` and copy it); `busy = None` |
| `CatalogueStale(s)` | `mode` is `Editing(e)` or `ConfirmPrefix { editor: e, .. }` | replace snapshot; `reload(e, s)`: locate the row by `e.kind`'s id → `Gone` → `DELETED_ELSEWHERE`, `Browse`; `Token(t)` → `e.expected = Some(t)`, `stored_prefix`/`stored_budget` refreshed from the current row, editor text kept, notice `CHANGED_ELSEWHERE`, `mode = Editing(e)` (a `ConfirmPrefix` collapses back to the editor so the user re-reads the warning against the current row); `busy = None` |
| `CatalogueStale(s)` | any other mode | replace snapshot; notice `CHANGED_ELSEWHERE`; `busy = None` |
| `KindDeleted { mirror, catalogue }` | — | replace snapshot; `notice = deleted_notice(prefix, mirror)` using the `Deleting` mode's prefix; `mode = Browse`; `busy = None`; `clamp_cursor` |
| `Failed { request, message }` | `REQUEST_NAMES.contains(request)` | `request == "catalogue"` → `unavailable = Some(message)`, snapshot kept; else → `notice = Some(message)` in error, `busy = None`, mode kept (`Deleting { InFlight }` → back to `Asking`? No: → `Browse`, as M3 does for a refused delete — the question was answered by the seam) |
| anything else | | ignored |

### 4.10 `impl SettingsSection`

`id` → `Self::ID`; `title` → `"Kinds"`; `wants_requests(scope)` → `vec![Catalogue(scope.clone())]`;
`on_scope_change` → B-12; `captures_input` → `!matches!(self.mode, Mode::Browse)`; `on_key` →
dispatch on mode (`Editing` → `on_editor_key`, `ConfirmPrefix` → `on_confirm_key`, `Deleting` →
`on_deleting_key`, `Browse` → table 4.7); `on_reply` → 4.9; `render` → 4.11.

### 4.11 `render`

Copy of `HierarchySection::render` (`:1220-1260`): `Layout::vertical([Min(3), Length(pane.len() as u16), Length(1)])`; rows drawn as a `Paragraph` of `lines()` scrolled so the cursor is visible (reuse M3's scroll arithmetic verbatim); pane below; hint last via `message(frame, hint_area, text, theme)` from `settings/mod.rs`. When there is no snapshot: one `message` with `NO_WORKSPACE`/`UNAVAILABLE` and the matching hint, as M3's `render` does.

---

## 5. `settings/mod.rs` and `settings/hierarchy.rs` edits (T2, D14)

- `settings/mod.rs`: `pub mod kinds;` after `pub mod hierarchy;` (`:12`); `pub use kinds::KindsSection;` after `:28`; move the two helpers in verbatim:

```rust
/// `""` is `None`, anything else is the text (M3).
pub(crate) fn some_text(text: String) -> Option<String>
/// `y`/`yes`/`n`/`no`, case-insensitive, trimmed (M3's `primary (y/n)` convention).
pub(crate) fn yes_or_no(text: &str) -> Option<bool>
```

- `settings/hierarchy.rs`: delete the private fns at `:1453-1462`; the call sites (`:882`, `:905` and any other) become `super::some_text` / `super::yes_or_no` (or a `use super::{some_text, yes_or_no};` at the top). `is_error` (`:1448`) stays private in `hierarchy.rs`; `kinds.rs` gets its own one-liner (or promote it too if the reviewer prefers — either is inside D14's spirit; the blueprint picks **promote all three** for one less duplicate: `pub(crate) fn is_error(notice: &str) -> bool`). Keep the doc comments.

---

## 6. Registration and the strip pin (T2, D17)

- `app/mod.rs:14`: `use crate::ui::tabs::settings::{AgentsSection, HierarchySection, KindsSection};`
- `app/mod.rs:47-50`: `vec![Box::new(AgentsSection::new()), Box::new(HierarchySection::new()), Box::new(KindsSection::new())]`.
- `tests/settings.rs:942-963` `the_section_strip_fits_the_frame`: the `vec!` gains `Box::new(KindsSection::new())`; the assertion (strip width `<= SECTION_WIDE`) is unchanged. Import `KindsSection`.

Snapshots outside `kinds__*` that show the strip line ` Agents  Hierarchy ` will now show
` Agents  Hierarchy  Kinds `: expected to move are `hierarchy__demo.snap`,
`hierarchy__no_workspace.snap`, `hierarchy__offline.snap` and every `settings__agents_*.snap`
that renders the full tab (the plan's Validation allows exactly this: "except where the section
strip gained `Kinds`"). `hierarchy__editor_repo`, `__stale`, `__delete_*` render through
`render_section` (no strip) and must be byte-identical.

---

## 7. `crates/htui/tests/kinds.rs` — section half (T2)

Helpers copied from `tests/hierarchy.rs:783-840`: `type_into`, `type_at`, `drawn`, `error_text`;
plus `async fn kinds_over(store: MemStore) -> Harness` (with_tab Settings, `settle`, `l`, `l`,
`settle` — two `l` because Kinds is third) and `async fn demo_snapshot(backend) -> CatalogueSnapshot`
(`§3`'s `demo_catalogue`). `bench_with_demo()` → `(SectionBench, KindsSection)` after
`bench.reply(&mut section, &StoreReply::Catalogue(Box::new(snapshot)))` and `bench.drained()`.

| Test | Plan | Steps | Assert / snapshot |
|---|---|---|---|
| `the_demo_catalogue_renders_the_tree` | D4, D17 | `Harness::demo()` → `kinds_over` → `render()` | `insta::assert_snapshot!("demo", …)` → **`kinds__demo.snap`**: strip ` Agents  Hierarchy  Kinds `, project line, five kind lines each with its phases beneath, no `Graph` rows (seed is 1:1), the first phase's detail line is not shown (cursor on the project row), hint `HINT_BROWSE` |
| `no_workspace_says_so` | — | `Harness::empty()` → Settings → `l` `l` | **`kinds__no_workspace.snap`**: `NO_WORKSPACE` + `HINT_NO_WORKSPACE` |
| `offline_is_unavailable_with_the_worker_sentence` | D7 | `Harness::over_backend(Offline)` as `tests/hierarchy.rs` does | **`kinds__offline.snap`**: `catalogue unavailable: {DATABASE_UNREACHABLE}` in error (`error_text` non-empty), hint `r reload` |
| `the_selected_phase_shows_the_read_only_line` | D15 | bench; `j` ×2 (project → ANA kind → its first phase) | **`kinds__phase_detail.snap`** via `render_section(&section, 100)`: the detail line under the phase, ending `— MOD-4 owns these`; `error_text` empty (it is dim, not error) |
| `a_kind_whose_graph_is_missing_says_so` | D4 | hand-built snapshot: demo project catalogue with one kind's `default_graph_id = StepGraphId::new()`; `bench.reply` | that kind's line ends `graph missing`, `error_text` contains `graph missing`, and no phase rows follow it (rows count = 1 + 5 kinds + 12 phases) |
| `an_unreferenced_graph_is_listed_after_the_kinds` | D4 | snapshot with an extra `GraphEntry { graph: { name: "orphan" }, phases: [] }` | the last row is `Graph`; its line is `  orphan · graph, no kind` |
| `l_is_a_letter_while_editing` | M3 D2 | bench; `n` on the project row; `type_at "l"` | `captures_input()` true; `Handled::Consumed`; field 0 text is `l`; `drained()` empty |
| `n_on_a_kind_opens_the_kind_editor_and_enter_creates` | D5, B-7, B-8 | bench; `j` (ANA kind); `n`; type prefix `DOC`, Tab, `docs`, Tab, Tab (description skipped; `graph` prefilled `analysis`), Tab (position prefilled `5`); `enter` | one `Action::Store(CreateKind { project: PROJECT_VULKAN, prefix: "DOC", name: "docs", description: "", graph: GRAPH_VULKAN_ANA, position: 5 })`; `busy == Some("create_kind")` observable as ` · create_kind in flight` is **not** on the hint while editing — assert via a second `enter` being refused with `` `create_kind` is still in flight `` |
| `a_required_field_is_refused_before_any_request` | M3 rule | `n`; `enter` with an empty prefix | `drained()` empty; notice `` `prefix` is required `` in `error_text` |
| `an_unknown_graph_name_is_refused` | B-7 | `n`; fill; set `graph` to `nope`; `enter` | no request; notice `NO_GRAPH_NAMED` |
| `e_on_a_kind_then_enter_sends_the_whole_patch_with_the_token` | D8, B-5 | `j`; `e`; Tab to `name`; type ` 2`; `enter` | `UpdateKind { id: KIND_VULKAN_ANA, expected: kind.updated_at, patch }` with all five `Some`, `patch.prefix == Some("ANA")`, `name == Some("analysis 2")` |
| `a_prefix_change_asks_before_it_writes` | D10 | `j`; `e`; clear prefix (`backspace` ×3), type `AN`; `enter` | `drained()` empty; **`kinds__prefix_warn.snap`** via `render_section`: `items keyed ANA-* keep their keys and their counter; the next item minted under this kind is AN-1.` in error; hint `y write · n/Esc back to the editor`; `captures_input()` true |
| `n_on_the_prefix_warning_returns_to_the_editor_with_its_text` | D10 | as above, then `n` | mode is editing again (hint `HINT_EDITING`); field 0 reads `AN`; `drained()` empty; then `y` after a second `enter` → exactly one `UpdateKind` whose `patch.prefix == Some("AN")` |
| `y_on_the_prefix_warning_writes_once` | D10 | as above, then `y` | one `UpdateKind`; a second `y` (or `enter`) → no second request, notice `` `update_kind` is still in flight `` |
| `d_on_a_kind_asks_once` | D11 | `j` ×2… position on `FEAT` kind (`j` past ANA's two phases: project, ANA, p, p, FEAT = `j` ×4); `d` | **`kinds__delete_ask.snap`** via `render_section`: `delete kind feature (FEAT)? a kind any item uses is refused. y delete · n/Esc stop` in error; `drained()` empty |
| `y_sends_delete_kind_and_a_refusal_is_the_seams_sentence` | D11, F-7 | as above, `y` → `DeleteKind { id: KIND_VULKAN_FEAT }`; then `bench.reply(Failed { request: "delete_kind", message: item_kind_is_held("FEAT", 1) })` | one request; after the reply: mode Browse, `error_text` contains `item_kind FEAT is held by 1 items` |
| `a_kind_delete_reports_the_mirror` | D12 | `bench.reply(KindDeleted { mirror: Rebuilt, catalogue })` while `Deleting { InFlight }` for `ANA` | notice `` deleted kind `ANA`; mirror rebuilt ``; `Browse`; tree has 4 kinds |
| `d_on_a_phase_or_graph_is_refused` | D5 | `j` ×2 (a phase); `d` | no request; notice `graphs and phases are not deleted here` |
| `e_on_a_phase_opens_six_fields_and_enter_sends_update_phase` | D13, D15 | `j` ×2; `e`; Tab ×4 to `input_kinds`; clear; type `plan, review,,`; `enter` | **`kinds__editor_phase.snap`** via `render_section` before `enter` (six labels: `name`, `position`, `template_name`, `gate_hard (y/n)`, `input_kinds`, `token_budget`; `token_budget` empty = inherit); then one `UpdatePhase { id, expected: phase.updated_at, patch }` with `patch.input_kinds == Some(vec!["plan","review"])`, `gate_hard == Some(false)` and the other three `Some(unchanged)`; no `SetPhaseBudget` |
| `a_budget_only_change_sends_set_phase_budget` | D6, D16 | `e` on a phase; Tab ×5; type `60000`; `enter` | one `SetPhaseBudget { phase, expected: phase.updated_at, budget: Some(60000) }`, no `UpdatePhase` |
| `clearing_the_budget_sends_none` | D16 | snapshot with `token_budget = Some(60000)` on that phase; `e`; Tab ×5; `backspace` ×5; `enter` | `SetPhaseBudget { budget: None }` |
| `a_non_number_budget_is_refused` | B-1 | type `lots`; `enter` | no request; notice `` `token_budget` is a whole number or empty `` |
| `a_patch_and_budget_change_is_two_writes_in_order` | B-4 | `e`; rename; Tab ×5; `60000`; `enter` → `UpdatePhase`; then `bench.reply(Catalogue(fresh))` where `fresh` has the phase with a new `updated_at` | after the reply: exactly one more action, `SetPhaseBudget { expected: fresh_updated_at, budget: Some(60000) }`; editor still open (`captures_input()`); then `bench.reply(Catalogue(…))` → Browse |
| `a_stale_reply_keeps_the_text_and_retakes_the_token` | D8 | `e` on ANA; type `x`; `enter`; `bench.reply(CatalogueStale(snapshot with ANA.updated_at + 1s))` | **`kinds__stale.snap`** via `render_section`: editor open, `name` field reads `analysisx`, notice `CHANGED_ELSEWHERE`; then `enter` → `UpdateKind { expected: the new token }` |
| `a_stale_reply_with_the_row_gone_closes_the_editor` | D8 | `CatalogueStale(snapshot without ANA)` | mode Browse; notice `DELETED_ELSEWHERE` |
| `r_reloads_and_esc_clears_the_notice` | T2 keys | `r` → one `Catalogue(scope)` with `scope == bench.scope` (the demo Graphics scope); set a notice via a refusal; `esc` | notice gone |
| `a_scope_change_drops_the_editor_and_keeps_the_notice` | B-12 | `e`; `section.on_scope_change(&other_scope)` | `captures_input()` false; `wants_requests(&other)` == `[Catalogue(other)]`; notice survives |
| `the_section_holds_no_handle` | R-NF-3 | compile-time by construction; assert in a doc test or skip | (M3 skipped; skip) |

Snapshot names (all under `crates/htui/tests/snapshots/`): `kinds__demo.snap`,
`kinds__no_workspace.snap`, `kinds__offline.snap`, `kinds__phase_detail.snap`,
`kinds__prefix_warn.snap`, `kinds__delete_ask.snap`, `kinds__editor_phase.snap`,
`kinds__stale.snap` — eight. Bare `insta::assert_snapshot!("name", …)` in `tests/kinds.rs`
produces `kinds__name.snap`. Read every `.snap.new` before renaming (plan T2 gate).

---

## 8. Data flow (three passes)

1. **Read.** Tab activation / scope change → `wants_requests` → `Catalogue(scope)` → worker
   `try_serve` → `catalogue::serve` → `snapshot` (N+1 over `project`/`item_kinds`/`step_graphs`/
   `phases`) → `StoreReply::Catalogue` → `on_reply` replaces the snapshot → `rows()`/`lines()` at
   render. Nothing is cached beside the snapshot (D3).
2. **Write.** `Enter` → `submit` → one `StoreRequest` carrying `scope` and the row's token → worker
   seam call → `cas`/`reread` → `Catalogue` or `CatalogueStale` → `on_reply`: close the editor or
   keep it with the new token. B-4's chain is the only case of two writes for one `Enter`, and they
   are serial through `busy`.
3. **Delete.** `d` → `Deleting { Asking }` → `y` → `DeleteKind` → seam delete → mirror rebuild →
   `KindDeleted` → notice with the mirror outcome; or `Failed` → the seam's sentence, Browse.

---

## 9. Build order (TDD; each step's tests red before its code) and commit plan

| # | Step | Files | Implements | Commit |
|---|---|---|---|---|
| 1 | `catalogue_names_are_stable` + the nine `StoreRequest` variants, `name()` arms, three replies, `try_serve` arm (compile probe of the or-ed arm first) | `store_worker.rs`, `lib.rs` (`pub mod catalogue;` with an empty `REQUEST_NAMES` stub), `tests/kinds.rs` | D5, F-2 | `feat(htui): nine catalogue requests are named and routed (M4 D5)` |
| 2 | Types + `snapshot` + `Catalogue` arm; tests `the_demo_catalogue_reads_back_the_seed`, `a_two_project_scope_…`, `an_unknown_project_id_is_skipped`, `offline_refuses_…`, `serve_refuses_a_foreign_request_by_name` | `catalogue.rs`, `tests/kinds.rs` | D1, D2, D3, D7 | `feat(htui): the catalogue is read whole per scope (M4 D2, D3)` |
| 3 | Kind and graph writes (`Create*`, `Update*`) with `reread`/`cas`; their tests incl. `a_prefix_rename_leaves_existing_keys_alone` and the stale twin | `catalogue.rs`, `tests/kinds.rs` | D5, D8, D10 (seam side) | `feat(htui): kinds and graphs write through CAS (M4 D8)` |
| 4 | `CreatePhase` (D9 row) + `UpdatePhase` + `SetPhaseBudget`; their tests | `catalogue.rs`, `tests/kinds.rs` | D6, D9, D13, D16, D17b, B-1, B-2 | `feat(htui): a phase is the seeder's row plus five columns; budget rides the Phase rung (M4 D6, D9)` |
| 5 | `DeleteKind` + mirror; tests | `catalogue.rs`, `tests/kinds.rs` | D11 (seam side), D12 | `feat(htui): a kind delete rebuilds the mirror and says so (M4 D12)` |
| — | **T1 gate**: `cargo test -p htui --all-features`, clippy `-D warnings`, `cargo doc -p htui --no-deps`, `CASES`/`EXPECTED_CASES` 36, `git status` clean of `.sqlx/`/`migrations/` | | | |
| 6 | D14 promotion; hierarchy call sites; `pub mod kinds;` with an empty `KindsSection` that renders `NO_WORKSPACE`; registration; strip pin; accept the strip-line snapshot moves after reading each | `settings/mod.rs`, `settings/hierarchy.rs`, `kinds.rs`, `app/mod.rs`, `tests/settings.rs`, moved snapshots | D14, D17 | `feat(htui): Settings > Kinds is registered after Hierarchy (M4 D14, D17)` |
| 7 | Rows, lines, detail line, browse keys `j`/`k`/`r`/`Esc`, `on_reply` for `Catalogue`/`Failed(catalogue)`; tests `the_demo_catalogue_renders_the_tree`, `no_workspace_says_so`, `offline_…`, `the_selected_phase_shows_…`, `a_kind_whose_graph_is_missing_…`, `an_unreferenced_graph_…`, `r_reloads_…`, `a_scope_change_…` | `kinds.rs`, `tests/kinds.rs`, 4 snapshots | D4, D15, B-6, B-11, B-12 | `feat(htui): the kinds tree renders kinds, their phases and the read-only columns (M4 D4, D15)` |
| 8 | Editors: `Field`/`Editor`/`Mode::Editing`, `open`, `on_editor_key`, `submit` for kinds and graphs, `CatalogueStale` handling; tests `l_is_a_letter_…`, `n_on_a_kind_…`, `a_required_field_…`, `an_unknown_graph_name_…`, `e_on_a_kind_…`, `a_stale_reply_…` ×2 | `kinds.rs`, `tests/kinds.rs`, `kinds__stale.snap` | D5, D8, B-5, B-7, B-8 | `feat(htui): kinds and graphs are created and edited from the section (M4 D8)` |
| 9 | `Mode::ConfirmPrefix`; tests `a_prefix_change_asks_…`, `n_on_the_prefix_warning_…`, `y_on_the_prefix_warning_…` | `kinds.rs`, `tests/kinds.rs`, `kinds__prefix_warn.snap` | D10 | `feat(htui): a prefix change is read before it is written (M4 D10)` |
| 10 | Phase editor (six fields), B-9/B-10 parsing, `SetPhaseBudget`, B-4 chain; tests `e_on_a_phase_…`, `a_budget_only_…`, `clearing_the_budget_…`, `a_non_number_budget_…`, `a_patch_and_budget_change_…` | `kinds.rs`, `tests/kinds.rs`, `kinds__editor_phase.snap` | D6, D13, D16, B-3, B-4 | `feat(htui): a phase's six columns are editable; empty budget means inherit (M4 D13, D16)` |
| 11 | `Mode::Deleting`, `on_deleting_key`, `KindDeleted`/`Failed(delete_kind)`; tests `d_on_a_kind_asks_once`, `y_sends_delete_kind_…`, `a_kind_delete_reports_…`, `d_on_a_phase_or_graph_…` | `kinds.rs`, `tests/kinds.rs`, `kinds__delete_ask.snap` | D5, D11, D12 | `feat(htui): a kind delete asks once and reports the mirror (M4 D11)` |
| — | **T2 gate**: full `cargo test -p htui --all-features`; every `kinds__*.snap` read; no snapshot outside `kinds__*` changed except by the strip line; clippy; `--demo` smoke only where a TTY exists, otherwise say so | | | |
| 12 | HANDOFF "Milestone 4 landed" paragraph; PRD row 4 → complete with the plan link | `HANDOFF.md`, PRD | T3 | `docs(mod-15): milestone 4 phase note` |

Each step commits before the next starts (memory: uncommitted subagent work dies with the session).

---

## 10. Hazards (numbered for the implementer's prompt)

| # | Hazard | Guard |
|---|---|---|
| H-1 | A guarded `try_serve` arm → `E0004` (F-2). | Or-ed patterns, no guard; compile probe first (step 1). |
| H-2 | `PhaseSeed`'s `&'static` fields (F-6). | §1.5: empty static seed, overwrite four columns after `phase_row`. Never a `Box::leak`. |
| H-3 | Two writes in flight from one `Enter`. | B-4: the second is sent only from `on_reply` with the fresh token; `busy` is one slot and `submit` refuses while it is `Some`. |
| H-4 | `set_setting` with `expected: None` on the Phase rung is a `Constraint`, not a CAS miss. | B-2: always `Some(expected)`. |
| H-5 | `Value::from(i64)` for a number the column cannot hold. | B-1: the store's sentence reaches the notice untouched; test `set_phase_budget_sets_then_clears` covers `i64::MAX`. |
| H-6 | A graph every kind points at has no `Graph` row, so `e` cannot reach `UpdateGraph` for a referenced graph. | **Closed by D19**: `g` on a kind or phase row opens the owning graph's editor. No fifth row variant, no doubled tree — one key, the editor `e` already opens. O-2 below is resolved. |
| H-7 | `Debug` on `Editor`/`Mode` printing typed text. | Hand-written `Debug` for `Editor` (labels, focus, kind, token only); `Mode` derives nothing that reaches `Editor`'s fields except through that impl. `TextField`'s own `Debug` never prints the buffer (M3). |
| H-8 | `clamp_cursor` after a delete leaves the cursor on the detail line's old index. | The detail line is not a row; clamp against `rows().len()`. |
| H-9 | A `Catalogue` reply from an `r` while an editor is open must not close the editor. | Only a reply matching a **write** `busy` closes it; `r` never sets `busy`. Copy M3's `on_tree` condition. |
| H-10 | `ConfirmPrefix` swallowing `q`. | `captures_input` is true there; the tab does not see `q`; `Esc` returns to the editor, `Esc` again to Browse. Documented in the hint. |
| H-11 | Kind `position` uniqueness: `check_item_kind` (`mem.rs:1855-1890`) does **not** check position, so duplicates are allowed; B-8's prefill is a courtesy, not a guard. | Nothing to do; state it in the kind editor's doc. |
| H-12 | `hierarchy__*.snap` that render the whole tab move by one strip token. | §6 lists which; read each diff; nothing else may move. |
| H-13 | `serde_json` not in `crates/htui`'s dependencies. | Check `Cargo.toml`; add the workspace dep if absent (`serde_json = { workspace = true }`), no new version. |
| H-14 | `item_kinds` ordering ties (`position` then `prefix`) differ from the seed order if positions collide. | Tests assert the seed's positions 0..4, which do not. |
| H-15 | Reading `Project` for a scope id that is a widowed link. | `project(id)?` → `None` → `continue` (D3), tested. |
| H-16 | `KindDeleted` arriving with no `Deleting` mode (e.g. after a scope change dropped it). | Notice uses the prefix from the mode when present, else `deleted kind; {mirror}`; never panics. |
| H-17 | `Failed { request: "catalogue" }` while an editor is open. | `unavailable` is set, snapshot kept, editor kept; `submit` still refuses nothing new — the next write's `Failed` will say `DATABASE_UNREACHABLE`. |

---

## 11. What must not change

`crates/htui-core/src/store/**`, `crates/htui-store/**` (`.sqlx/`, `migrations/`, `query!`),
`crates/htui-agent/**`, `seed.rs`, `ui/text_field.rs`, `chat/composer.rs`, `keymap.rs`,
`app/action.rs`; `CASES == 36`, `EXPECTED_CASES == 36`; every `hierarchy__*.snap` rendered through
`render_section`; the `HierarchySection` behaviour (only `super::` call sites move); the
`SettingsSection` trait; `Harness`/`SectionBench` APIs (add nothing to `testkit.rs` — the helpers
this file needs live in `tests/kinds.rs`).

---

## 12. Open items (recorded, not decided here)

- **O-1** (plan D17b): whether a phase rename should follow `output_kind` — MOD-4's.
- ~~**O-2** (H-6)~~ — **resolved by D19**: `g` on a kind or phase row opens the owning graph's
  editor, so every graph is reachable whether or not a kind points at it. A `Row::Graph` under
  every kind would have doubled the tree and is still not done.
- **O-3**: `RELOADED` after `r` — M3's exact behaviour decides whether the notice is set (copy
  `on_tree`); the blueprint does not pin a snapshot on it.
- **O-4**: whether `is_error` joins D14's promotion (this file says yes; a reviewer may keep it
  private — either is inside the plan).
