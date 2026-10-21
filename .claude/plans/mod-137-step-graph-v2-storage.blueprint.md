# Blueprint: MOD-137 — Step graph v2 storage (T1–T6)

> Produced by `code-architect` 2026-10-21 from the confirmed plan
> `.claude/plans/mod-137-step-graph-v2-storage.plan.md` (D1–D13), design `docs/ANA-33.md` §7.1, §10.
> Tree read at `294b9366` (branch `hr/MOD-137`). The plan stays the spec. §0 lists where the tree
> disagrees with it or where the plan is silent. Every row is **adopted**: implementation detail,
> or a plan claim corrected by evidence. No row reopens a decision, so there is no DV for the
> maintainer. Each row names the lines to flip if a reviewer objects.

## 0. Plan claims contradicted or extended by the tree

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | D5's writer list omits `clear_setting` on the `Phase` rung, which writes `token_budget = NULL`, a node field, exactly as `set_setting` does. | `pg/write.rs:4126-4140`, `mem.rs:4427` | **adopted**: `clear_setting`'s Phase arm bumps `revision` too (§1.4, row 7). |
| F2 | "22 `StepGraphPhase {` sites / 11 files" over-counts. There are **14** real literals, 2 of them full (`seed.rs:216`, core `conformance.rs:2752`) and 12 spread (`..x.clone()`). The rest of the grep hits are `-> StepGraphPhase {` return types and the definition. The 5 `StepGraph {` and 17 `NewStepGraph {` counts are right. | fork census (§T2.5) | **adopted**: §T2.5 is the exact list. |
| F3 | The Validation block's `cargo sqlx prepare --workspace -- --all-targets --all-features` writes a second `.sqlx` at the workspace root that nothing reads. The committed data is `crates/htui-store/.sqlx/`. | `docs/hr-sandbox.md:206-219`; repo memory "sqlx prepare needs a migrated scratch DB" | **adopted**: run from inside `crates/htui-store` (§3). T6 fixes the plan's block in passing. |
| F4 | D9's "else to the immediately previous position" means previous **in position order**, not `position - 1`. Live positions may have gaps (`UNIQUE (graph_id, position)` only). `gate::loop_target` runs on the snapshot, whose positions `graph::resolve` renumbers densely (`graph.rs:345-365`, the `dense` loop). On live rows, `review` at 4 after `draft` at 0 must loop to `draft`. | `gate.rs:807-823` + `graph.rs:345-365`; probe graph `gap-no-implement` (§T1.3) | **adopted**: the SQL uses `LEAD`/`ORDER BY position DESC LIMIT 1` and `default_edges` sorts. Both are pinned by the gap case. |
| F5 | `seed::default_edges(&[StepGraphPhase]) -> Vec<StepGraphEdge>` cannot mint edge ids. Mem and Pg want UUIDv7, the demo wants `demo_uuid(class::EDGE, n)`. | `fixtures.rs:93,804` (deterministic ids), `seed.rs` is id-agnostic for phases | **adopted**: `default_edges(phases, mint: impl FnMut() -> EdgeId)`. The D9 equality test compares edges without their ids. |
| F6 | D10 says to map the commit-time `23505` so that Pg's text equals Mem's. `map_sqlx` words a unique violation as `"<constraint>: <message>"` (`error.rs:36-46`), never as Mem's sentence. Identical wording therefore needs a decision **before** the write, not a mapping after it. | `error.rs:17-52`; Mem `check_phase` wording `mem.rs:3567-3599` | **adopted**: both stores call the shared pure checkers of `store/graph_rules.rs` (§T3.2). Pg pre-reads the FK targets inside the transaction. The deferred uniques stay only as a backstop: they are forced with `SET CONSTRAINTS ALL IMMEDIATE` before the re-read, so a missed clash surfaces at a statement, never at `COMMIT`. |
| F7 | D5's bump must take the **graph row lock first**. `replace_graph` locks `step_graph` (CAS) and then writes phase rows. A per-row writer that writes its phase row first and bumps afterwards inverts that order, and the two can deadlock (`40P01`). Pg's deferred-unique check at the end of `replace_graph` would even wait on the other transaction's uncommitted phase insert. | lock order reasoning. MOD-77 D1 is this repo's precedent (`write.rs:203-240`) | **adopted**: every Pg writer runs `bump` as its **first** statement in a transaction (§1.4). Single-statement writers become transactions. A CAS miss rolls the bump back. |
| F8 | `delete_persona`'s Pg holders read `JOIN project pr ON pr.id = g.project_id` (`write.rs:3867-3871`). A holder on a workspace or library graph is never named, so the capped loop ends in `"delete_persona: holders kept changing"` (`:3887`). The plan says only "also counts the two divide columns and `phase_part`". | `write.rs:3840-3889`; Mem twin `mem.rs:4020-4027` names `?` | **adopted**: `LEFT JOIN project` + `LEFT JOIN workspace`, owner label `COALESCE(pr.slug, 'workspace:' \|\| w.slug, 'library')`, the same label in Mem (§T2.7). |
| F9 | `set_skill_binding` derives a phase's project from its graph (`mem.rs:4141`, `pg/write.rs:3562`, `.map(\|graph\| graph.project_id)`). These two readers are not in the plan's "~9". | census §3 | **adopted**: `.and_then(\|graph\| graph.scope.project_id())`. A phase of a workspace or library graph then has no project, and rule 2 refuses its binding with `phase_not_in_project`. That is D4's "no use of a non-project graph yet". |
| F10 | The conformance suite is generic over `WriteStore`, which has no reader for one graph (`step_graph(id)` is inherent, `mem.rs:727`, `pg/read.rs:1826`). A library or workspace graph is invisible to `step_graphs(project)`, so `graph_scopes_are_unique_per_scope` and `delete_workspace_takes_its_graphs` could not observe their rows. Pg needs the same whole-body read for `Stale(current body)`. | `traits.rs:884-952` | **adopted**: one extra trait read, `graph_body(graph) -> Result<Option<GraphBody>>` (§T3.1). To drop it, the two cases probe with `replace_graph(.., 0, ..)` → `NotFound` instead. |
| F11 | The plan's T5 file list has `htui-orch/src/conformance.rs` "(the override cases that live there)". No override case lives there: `override_graph`'s tests are `graph.rs:1929,2078,2155,2247`, and `conformance.rs:1197` is a doc line. | text search `override_graph(` | **adopted**: T5 touches `graph.rs` only. The plan's T4 ∥ T5 disjointness holds a fortiori. |
| F12 | `create_phase` binds the whole row. After 0020, `ck_phase_split_fan_out` refuses a `fan_out > 1` row whose `split` is not `compete`. Four test helpers raise `fan_out` and then call `create_phase`: orch `conformance.rs:1201` `repoint` and `:1263` `insert_verify_phase`, `engine.rs:7851` `repoint`, `tests/gix_isolator.rs:336` `repoint`. That is 11 + 15 + 1 call sites. `from_demo` fixtures (`graph.rs:1749,2372,2500`, `run_worker.rs:1052`) bypass every store check and are untouched. | text search `\.fan_out = [2-9]` | **adopted**: each of the four helpers sets `split` from `fan_out` after `mutate` (§T2.5). No call site changes. |
| F13 | `StepGraph` and `StepGraphPhase` are bound by `query_as!` straight into the struct. `scope: GraphScope` cannot be decoded that way. | `pg/read.rs:2925-2945`, `write.rs:2967-2990` | **adopted**: a private `StepGraphRow` in `pg/read.rs`, the `PersonaRow` shape (`read.rs:2990`). The phase row stays flat (§1.2). |
| F14 | `StepGraphPhase` lacks three columns that `step_graph_phase` has: `judge_agent_id`, `judge_model`, `deadline_seconds`. `replace_graph` cannot carry them. | scratch DB `\d step_graph_phase`; `kind.rs:266-310` | **adopted**: a kept node keeps them (they are absent from the `UPDATE`), and a new node gets `NULL`. Documented on the trait method. |

---

## 1. Shared decisions for every task

### 1.1 Where things live

| Item | File |
|---|---|
| `EdgeId` | `crates/htui-core/src/model/ids.rs`, `id_newtype!` list right after `PhaseId` |
| `GraphScope`, `Split`, `PlanScope`, `EdgeKind`, `EdgePredicate`, `StepGraphEdge`, `PhasePart`, `NodeBody`, `GraphBody`; field changes on `StepGraph`, `NewStepGraph`, `StepGraphPhase` | `crates/htui-core/src/model/kind.rs`, exported through `model/mod.rs`'s existing `kind` re-export |
| Refusal sentences (§1.5) | `crates/htui-core/src/store/traits.rs`, a new `// ---- MOD-137 ----` block after the MOD-4 block (`:2656`) |
| Pure checkers `phase_row_refusal`, `graph_body_shape_refusal`, `graph_body_topology_refusal`, `body_persona_refs`, `body_agent_refs` | `crates/htui-core/src/store/graph_rules.rs` (new), `pub mod` + `pub use` in `store/mod.rs`. It is the seed of MOD-138's shared validator. |
| `seed::default_edges` | `crates/htui-core/src/seed.rs` after `phase_row` |
| `StepGraphRow`, `EdgeRow`, `graph_body_on`, `graph_edges_on`, `phase_parts_on` | `crates/htui-store/src/pg/read.rs`, next to `step_graph_rows` |

### 1.2 Rust types (exact)

```rust
// ids.rs, inside id_newtype!( … ), after PhaseId:
    /// `step_graph_edge.id` (MOD-137 D6).
    EdgeId,
```

```rust
// kind.rs (imports gain WorkspaceId and EdgeId; `core::fmt` for Display)

/// Who owns a `step_graph` row (MOD-137 D3): a project, a workspace, or neither — the global
/// workflow library. `ck_step_graph_owner` keeps at most one of the two columns set. Who may *use*
/// a non-project graph is MOD-138's (plan D4): today a kind or item still names only its own
/// project's graphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "owner", content = "id", rename_all = "snake_case")]
pub enum GraphScope {
    /// `step_graph.project_id` set.
    Project(ProjectId),
    /// `step_graph.workspace_id` set.
    Workspace(WorkspaceId),
    /// Neither set: the global workflow library (ANA-33 §7.1).
    Library,
}

impl GraphScope {
    /// `step_graph.project_id`.
    #[must_use]
    pub const fn project_id(self) -> Option<ProjectId> { match self { Self::Project(id) => Some(id), _ => None } }
    /// `step_graph.workspace_id`.
    #[must_use]
    pub const fn workspace_id(self) -> Option<WorkspaceId> { match self { Self::Workspace(id) => Some(id), _ => None } }
    /// The two columns back into a scope; `None` when both are set, which `ck_step_graph_owner`
    /// makes unreachable on Postgres (the Pg row decoder answers `StoreError::Backend` then).
    #[must_use]
    pub const fn from_columns(project: Option<ProjectId>, workspace: Option<WorkspaceId>) -> Option<Self> {
        match (project, workspace) {
            (Some(p), None) => Some(Self::Project(p)),
            (None, Some(w)) => Some(Self::Workspace(w)),
            (None, None) => Some(Self::Library),
            (Some(_), Some(_)) => None,
        }
    }
}

/// `project `<id>``, `workspace `<id>`` or `the library`: the tail of every scope sentence.
impl fmt::Display for GraphScope { /* exactly those three spellings */ }

str_enum!(
    /// `step_graph_phase.split` (MOD-137 D7, ANA-33 §7.1). `compete` is exactly `fan_out > 1`
    /// (`ck_phase_split_fan_out`); `divide` alone may carry the planner/integrate columns and
    /// `phase_part` rows. Always written qualified (`Split::None`), never glob-imported.
    #[derive(Default)]
    Split {
        /// One step.
        #[default]
        None => "none",
        /// Rival candidates, today's fan-out (ANA-2 §4.5).
        Compete => "compete",
        /// Planner, parts, integrate (ANA-33 §7.4).
        Divide => "divide",
    }
);

str_enum!(
    /// `step_graph_phase.plan_scope` (MOD-137 D7, ANA-33 §7.5): what a planner node may plan.
    PlanScope {
        /// Only the divided node's parts.
        Parts => "parts",
        /// The node's successors.
        Successors => "successors",
        /// The whole graph.
        Graph => "graph",
    }
);

str_enum!(
    /// `step_graph_edge.kind` (MOD-137 D6).
    EdgeKind {
        /// Unconditional successor.
        Next => "next",
        /// Taken when `predicate` holds (ANA-33 §7.3); the only kind with a predicate.
        If => "if",
        /// A person picks whether it is taken; forward or back, `loop_limit` optional here.
        AsksYou => "asks_you",
        /// The review loop (R-ORCH-3); carries `loop_limit`.
        ChangesRequested => "changes_requested",
        /// Taken when the source fails; carries `loop_limit`.
        Fails => "fails",
    }
);

impl EdgeKind {
    /// `ck_step_graph_edge_predicate`: `if` and only `if` carries one.
    #[must_use]
    pub const fn takes_predicate(self) -> bool { matches!(self, Self::If) }
    /// `ck_step_graph_edge_loop_limit_required`.
    #[must_use]
    pub const fn needs_loop_limit(self) -> bool { matches!(self, Self::ChangesRequested | Self::Fails) }
}

/// `step_graph_edge.predicate` (MOD-137 D6): opaque JSON in MOD-137; MOD-138 replaces the inside
/// with ANA-33 §7.3's closed predicate set without a column change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EdgePredicate(pub Value);

/// A row of `step_graph_edge` (MOD-137 D6). Replaced, never edited: no `updated_at`, and
/// `created_at` is not carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepGraphEdge {
    /// `step_graph_edge.id`, minted client-side as a UUIDv7.
    pub id: EdgeId,
    /// `step_graph_edge.graph_id`.
    pub graph_id: StepGraphId,
    /// `step_graph_edge.from_phase_id`; a phase of `graph_id`.
    pub from: PhaseId,
    /// `step_graph_edge.to_phase_id`; a phase of `graph_id`, never `from`.
    pub to: PhaseId,
    /// `step_graph_edge.kind`.
    pub kind: EdgeKind,
    /// `step_graph_edge.predicate`; `Some` iff `kind` is `if`.
    #[serde(default)]
    pub predicate: Option<EdgePredicate>,
    /// `step_graph_edge.loop_limit` (ANA-33's `limit`, plan D2); required on
    /// `changes_requested` and `fails`, `>= 0`.
    #[serde(default)]
    pub loop_limit: Option<i32>,
}

/// A row of `phase_part` (MOD-137 D8): one default part of a `divide` node, by `position`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhasePart {
    /// `phase_part.phase_id`.
    pub phase_id: PhaseId,
    /// `phase_part.position`.
    pub position: i32,
    /// `phase_part.key`, unique within the phase.
    pub key: String,
    /// `phase_part.title`.
    pub title: String,
    /// `phase_part.brief`.
    #[serde(default)]
    pub brief: String,
    /// `phase_part.owned_paths`.
    #[serde(default)]
    pub owned_paths: Vec<String>,
    /// `phase_part.acceptance`.
    #[serde(default)]
    pub acceptance: Option<String>,
    /// `phase_part.agent_id`.
    #[serde(default)]
    pub agent_id: Option<AgentId>,
    /// `phase_part.model`.
    #[serde(default)]
    pub model: Option<String>,
    /// `phase_part.persona_id` (`ON DELETE RESTRICT`).
    #[serde(default)]
    pub persona_id: Option<PersonaId>,
}

/// One node of a [`GraphBody`]: the phase row, its candidates and its parts, each by `position`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeBody {
    /// The `step_graph_phase` row; `updated_at` is ignored on write.
    pub phase: StepGraphPhase,
    /// Its `phase_agent` rows, replaced wholesale.
    #[serde(default)]
    pub agents: Vec<PhaseAgent>,
    /// Its `phase_part` rows, replaced wholesale; only a `divide` node has any.
    #[serde(default)]
    pub parts: Vec<PhasePart>,
}

/// A graph's whole editable content, the one write of MOD-141's editor
/// ([`crate::store::WriteStore::replace_graph`], MOD-137 D10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphBody {
    /// The graph row. `name` and `description` are written; `scope`, `is_override`, `revision`
    /// and the timestamps are read-only (ignored on write, the stored values on read).
    pub graph: StepGraph,
    /// By `position`.
    pub nodes: Vec<NodeBody>,
    /// By (from position, to position, kind bytes).
    pub edges: Vec<StepGraphEdge>,
}
```

`StepGraph` (field order **is** the Pg row order below):

```rust
pub struct StepGraph {
    pub id: StepGraphId,
    /// `step_graph.project_id` / `workspace_id` (MOD-137 D3). Replaces `project_id`.
    pub scope: GraphScope,
    pub name: String,            // doc: "unique within its scope"
    pub description: String,
    pub is_override: bool,
    /// `step_graph.revision` (MOD-137 D5): bumped by every write to the graph's content; the
    /// CAS token of `replace_graph`. 0 on a fresh graph.
    #[serde(default)]
    pub revision: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

`NewStepGraph.project_id: ProjectId` becomes `pub scope: GraphScope` with the doc "an override
(`is_override`) must be `Project` (`ck_step_graph_override_in_project`)". `StepGraphPatch` is unchanged.

**D7: flat columns, not a grouped `DivideSpec`.** Ten fields are appended to `StepGraphPhase`
**between `persona_id` and `updated_at`**, in this order, each with `#[serde(default)]`:

```rust
    /// `step_graph_phase.split` (MOD-137 D7).
    #[serde(default)] pub split: Split,
    /// `step_graph_phase.palette` (D7): a planner may instantiate it; the walk never reaches it;
    /// it has no edges.
    #[serde(default)] pub palette: bool,
    /// `step_graph_phase.plan_scope` (D7); `None` off a planner node.
    #[serde(default)] pub plan_scope: Option<PlanScope>,
    /// `step_graph_phase.planner_persona_id` (D7); `divide` only.
    #[serde(default)] pub planner_persona_id: Option<PersonaId>,
    /// `step_graph_phase.planner_agent_id` (D7); `divide` only.
    #[serde(default)] pub planner_agent_id: Option<AgentId>,
    /// `step_graph_phase.planner_model` (D7); `divide` only.
    #[serde(default)] pub planner_model: Option<String>,
    /// `step_graph_phase.integrate_persona_id` (D7); `divide` only.
    #[serde(default)] pub integrate_persona_id: Option<PersonaId>,
    /// `step_graph_phase.integrate_agent_id` (D7); `divide` only.
    #[serde(default)] pub integrate_agent_id: Option<AgentId>,
    /// `step_graph_phase.integrate_model` (D7); `divide` only.
    #[serde(default)] pub integrate_model: Option<String>,
    /// `step_graph_phase.max_parts` (D7); `divide` only, `>= 1`.
    #[serde(default)] pub max_parts: Option<i32>,
```

Why flat:

1. `query_as!(StepGraphPhase, …)` binds the struct straight from 4 SELECT/RETURNING lists (`read.rs:2809,2955`, `write.rs:3064,3193`). A nested `Option<DivideSpec>` would need a `PhaseRow` plus a conversion at each site, and a third decode path beside `seed_project` and `load_demo`.
2. The row maps 1:1, which is the repo's rule (`model/mod.rs:3-4`: "field names are §5 column names verbatim"). `persona_id` and the judge pair are flat for the same reason.
3. The invariant "divide columns only on a divide node" is enforced once, by `ck_phase_divide_columns` plus the shared `phase_row_refusal`, in both stores. A Rust group would be a second, weaker statement of the same rule.
4. Grouping belongs to the frozen v2 node (`SnapshotNode.divide: Option<{…}>`, ANA-33 §7.2), which MOD-138 builds from these columns at resolve time.
5. `#[serde(default)]` lets an older serialised phase decode.

### 1.3 Ordering conventions (both stores, conformance-pinned)

- Nodes by `position`. Candidates and parts by `position`.
- Edges by `(position(from), position(to), kind.as_str() bytes)`. Pg: `ORDER BY f.position, t.position, e.kind COLLATE "C"`.
- `graph_edges` and `phase_parts` of an unknown id answer an empty `Vec`, as `phases` does. `graph_body` of an unknown id answers `None`.

### 1.4 The revision-bump rule (D5) — every writer, both stores

The rule: one content write = `revision + 1` (and, through the trigger in Pg or by hand in Mem,
`step_graph.updated_at = now`). `create_step_graph` and the seeders write `revision = 0`. A refused
or stale write bumps nothing.

**Pg lock order (F7):** the bump is the **first** statement of the writer's transaction, so the
graph row is always locked before any phase, candidate, part or edge row. Two helpers go in
`pg/write.rs`:

```rust
/// MOD-137 D5, F7: `graph`'s revision + 1, as the first statement of the caller's transaction
/// (graph row first, then its phase rows). `false` when no such graph (the writer's own statement
/// then refuses as it always has).
async fn bump_graph(conn: &mut PgConnection, graph: StepGraphId) -> Result<bool>
//  UPDATE step_graph SET revision = revision + 1 WHERE id = $1

/// The same through a phase: `None` when the phase names no row.
async fn bump_graph_of_phase(conn: &mut PgConnection, phase: PhaseId) -> Result<Option<StepGraphId>>
//  UPDATE step_graph g SET revision = g.revision + 1
//    FROM step_graph_phase p WHERE p.id = $1 AND g.id = p.graph_id RETURNING g.id AS "id: StepGraphId"
```

Mem: a `State::bump_graph(&mut self, graph: StepGraphId, now)` helper does `revision += 1;
updated_at = now`. It is called after the writer's last refusal and before or after its mutation
(one lock, so the order does not matter).

| # | Writer | Mem site | Pg site | Change |
|---|---|---|---|---|
| 1 | `create_step_graph` | `mem.rs:3475` | `write.rs:2966` | `revision: 0` in the literal; Pg `RETURNING` gains `workspace_id, revision`. No bump |
| 2 | `update_step_graph` | `mem.rs:3512` | `write.rs:2998` | Mem `row.revision += 1` beside `updated_at = now`; Pg one statement: `SET name = …, description = …, revision = revision + 1` (the row is the graph, so the lock order holds) |
| 3 | `create_phase` | `mem.rs:3601` | `write.rs:3059` | Mem `bump_graph(phase.graph_id)` after the last check; Pg `pool.begin()` → `bump_graph(graph_id)` → the insert → commit |
| 4 | `create_phase_agents` | `mem.rs:3626` | `write.rs:3122` | empty slice: no bump (writes nothing, as today). Mem bump after the checks; Pg tx `bump_graph_of_phase` → `UNNEST` insert → commit (an unknown phase still fails on the insert's `23503`, as today) |
| 5 | `update_phase` | `mem.rs:3679` | `write.rs:3169` | bump only on `Applied`. Pg: the reserved-name early return stays before any tx; then tx `bump_graph_of_phase` (None → rollback, `cas_miss(None)` = `NotFound`) → CAS `UPDATE`; `None` → **rollback**, then `cas_miss(self.phase_row(id))` |
| 6 | `set_setting` `Phase` rung | `mem.rs:4321` | `write.rs:4012` | as row 5 (the `RETURNING updated_at` statement inside the tx; miss → rollback) |
| 7 | `clear_setting` `Phase` rung (F1) | `mem.rs:4427` | `write.rs:4126` | as row 6 |
| 8 | `delete_phase` (new) | T3 | T3 | tx `bump_graph_of_phase` (None → `NotFound "step_graph_phase"`) → `DELETE` |
| 9 | `delete_edge` (new) | T3 | T3 | tx `UPDATE step_graph g SET revision = g.revision + 1 FROM step_graph_edge e WHERE e.id = $1 AND g.id = e.graph_id RETURNING g.id` (None → `NotFound "step_graph_edge"`) → `DELETE` |
| 10 | `delete_phase_agents` (new) | T3 | T3 | tx `bump_graph_of_phase` (None → `NotFound "step_graph_phase"`) → `DELETE FROM phase_agent WHERE phase_id = $1`; bumps even when the phase had no candidates |
| 11 | `replace_graph` (new) | T3 | T3 | the CAS **is** the bump (§T3.4) |
| — | seeders `create_project` / `seed_project`, `from_demo` / `load_demo` | `mem.rs:3015`, `:370` | `write.rs:7966`, `demo.rs:47` | `revision = 0`; no bump |
| — | `delete_project` / `delete_workspace` | — | — | rows go; nothing to bump |

`override_graph` (T5) writes through `create_step_graph` (0) and then `replace_graph` (→ 1). A
re-override is one more `replace_graph`.

Consequence to expect: a phase write now moves `step_graph.updated_at`, so an `update_step_graph`
holding a token read before that phase write answers `Stale` (D5, accepted). The catalogue
re-reads after every write, so `Settings > Kinds` never holds such a token. Watch for a test that does.

### 1.5 Refusal sentences (new helpers in `traits.rs`, both stores, every one conformance-pinned)

All are `#[must_use] pub fn … -> String`, in the existing `references_no_row` style:

```rust
pub fn graph_name_is_taken(name: &str, scope: GraphScope) -> String
    // "step_graph.name `{name}` is taken in {scope}"   (project wording unchanged from mem.rs:3493)
pub fn override_outside_project(scope: GraphScope) -> String
    // "an override graph is a per-item clone and belongs to a project, not {scope}"
pub fn row_names_another_graph(column: &str, row: impl Display, graph: StepGraphId) -> String
    // "{column} `{row}` is not the graph being written (`{graph}`)"
pub fn phase_position_is_taken(position: i32, graph: StepGraphId) -> String
    // "step_graph_phase.position {position} is taken in graph `{graph}`"   (mem.rs:3589 verbatim)
pub fn phase_name_is_taken(name: &str, graph: StepGraphId) -> String
    // "step_graph_phase.name `{name}` is taken in graph `{graph}`"         (mem.rs:3594 verbatim)
pub fn phase_part_key_is_taken(key: &str, phase: PhaseId) -> String
    // "phase_part.key `{key}` is taken in phase `{phase}`"
pub fn edge_endpoint_outside_graph(column: &str, phase: PhaseId, graph: StepGraphId) -> String
    // "step_graph_edge.{column} `{phase}` references no step_graph_phase in graph `{graph}`"
pub fn edge_loops_to_itself(edge: EdgeId) -> String
    // "step_graph_edge `{edge}` runs from a phase to itself"
pub fn edge_predicate_mismatch(edge: EdgeId, kind: EdgeKind) -> String
    // If:    "step_graph_edge `{edge}` is `if` and carries no predicate"
    // other: "step_graph_edge `{edge}` is `{kind}` and carries a predicate; only `if` does"
pub fn edge_loop_limit_negative(edge: EdgeId, limit: i32) -> String
    // "step_graph_edge `{edge}` has loop_limit {limit}; a loop limit is 0 or more"
pub fn edge_needs_loop_limit(edge: EdgeId, kind: EdgeKind) -> String
    // "step_graph_edge `{edge}` is `{kind}` and carries no loop_limit"
pub fn edge_is_taken(from: PhaseId, to: PhaseId, kind: EdgeKind, graph: StepGraphId) -> String
    // "step_graph_edge ({from}, {to}, `{kind}`) is taken in graph `{graph}`"
pub fn palette_phase_has_edges(phase: &str, edge: EdgeId) -> String
    // "palette phase `{phase}` has step_graph_edge `{edge}`; a palette node has no edges"
pub fn split_fan_out_mismatch(phase: &str, split: Split, fan_out: i32) -> String
    // "phase `{phase}` is split `{split}` with fan_out {fan_out}; `compete` is exactly fan_out > 1"
pub fn divide_columns_off_divide(phase: &str, split: Split) -> String
    // "phase `{phase}` is split `{split}`; planner_*, integrate_* and max_parts belong to a `divide` phase"
pub fn max_parts_below_one(phase: &str, max_parts: i32) -> String
    // "phase `{phase}` has max_parts {max_parts}; a divide phase takes at least 1"
pub fn parts_off_divide(phase: &str, split: Split) -> String
    // "phase `{phase}` is split `{split}`; only a `divide` phase has parts"
```

Mem `check_phase` (`mem.rs:3567-3599`) switches to `phase_position_is_taken` / `phase_name_is_taken`
(byte-identical text). Mem `create_step_graph` / `update_step_graph` switch to `graph_name_is_taken`.
Reused unchanged: `references_no_row`, `already_exists`, `reserved_phase_name`, `row_names_another_phase`.

---

## T1 — Migration 0020 and its tests

### T1.1 `crates/htui-store/migrations/0020_step_graph_v2.sql` (probed verbatim, §T1.3)

```sql
-- 0020_step_graph_v2.sql - MOD-137 (plan D1-D3, D5-D9). Forward-only (R-STO-5).
--
-- Step graphs get a topology of their own and an owner other than a project (ANA-33 §7.1).
--
-- Owner scope (D3): step_graph.project_id becomes nullable and workspace_id is added; at most one
-- is set (ck_step_graph_owner), and neither set is the global workflow library. The unnamed
-- UNIQUE (project_id, name) of 0001_init.sql (Postgres named it step_graph_project_id_name_key)
-- becomes one partial unique index per scope. An override stays a per-item clone, so it is
-- always project-scoped (ck_step_graph_override_in_project). A workspace delete cascades its
-- graphs; MOD-137 lets no kind or item name a non-project graph (plan D4), so nothing holds them.
--
-- revision (D5) is the whole-graph compare-and-set token: every writer of a graph's content bumps
-- it by one, explicitly, in its own statement or transaction (not a trigger), so MemStore and
-- PgStore follow one written rule. The bump fires trg_step_graph_updated_at too.
--
-- step_graph_phase gains the node properties a v2 snapshot will freeze (D7): split (backfilled
-- 'compete' where fan_out > 1, and kept equal to it by ck_phase_split_fan_out), palette,
-- plan_scope, and the divide planner/integrate columns, which only a 'divide' node may set. The
-- split backfill fires trg_step_graph_phase_updated_at on fanned-out phases, which only stales an
-- editor token. The position and name uniques of 0001_init.sql are re-created DEFERRABLE
-- INITIALLY IMMEDIATE under the names uq_step_graph_phase_position / uq_step_graph_phase_name, so
-- replace_graph can swap positions and names inside one transaction (SET CONSTRAINTS ...
-- DEFERRED); no statement targets them with ON CONFLICT. uq_step_graph_phase_graph_id is the
-- (non-deferrable) key the edge table's composite foreign keys reference, so an edge can never
-- point into another graph.
--
-- step_graph_edge (D6) holds the links; its bound is loop_limit, because LIMIT is a reserved word
-- (ANA-33 names it limit). phase_part (D8) holds a divided node's optional default parts. Both
-- are replaced, never edited, so neither has updated_at.
--
-- Backfill (D9): per graph, a 'next' edge between each pair of phases adjacent in position order,
-- and for each phase named 'review' a 'changes_requested' edge to the greatest earlier phase
-- named 'implement', else to the phase just before it in position order, with loop_limit = the
-- target's retry_limit. That is gate::loop_target over graph::resolve's dense renumbering, and
-- review_loop's bound; seed::default_edges states the same rule in Rust and a migration test pins
-- the two equal. Nothing reads edges at run time yet (MOD-138).
--
-- Neither table is mirrored and step_graph is not either, so there is no cache migration.
-- schema_version becomes 20, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

-- D3, D5: owner scope and revision.
ALTER TABLE step_graph ALTER COLUMN project_id DROP NOT NULL;

ALTER TABLE step_graph
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_step_graph_workspace REFERENCES workspace(id) ON DELETE CASCADE,
    ADD COLUMN revision BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT ck_step_graph_owner CHECK (project_id IS NULL OR workspace_id IS NULL),
    ADD CONSTRAINT ck_step_graph_override_in_project CHECK (NOT is_override OR project_id IS NOT NULL);

ALTER TABLE step_graph DROP CONSTRAINT step_graph_project_id_name_key;

CREATE UNIQUE INDEX uq_step_graph_project_name   ON step_graph (project_id, name)
    WHERE project_id IS NOT NULL;
CREATE UNIQUE INDEX uq_step_graph_workspace_name ON step_graph (workspace_id, name)
    WHERE workspace_id IS NOT NULL;
CREATE UNIQUE INDEX uq_step_graph_library_name   ON step_graph (name)
    WHERE project_id IS NULL AND workspace_id IS NULL;

-- D7: node properties.
ALTER TABLE step_graph_phase
    ADD COLUMN split                TEXT    NOT NULL DEFAULT 'none',
    ADD COLUMN palette              BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN plan_scope           TEXT    NULL,
    ADD COLUMN planner_persona_id   UUID    NULL
        CONSTRAINT fk_step_graph_phase_planner_persona REFERENCES persona(id) ON DELETE RESTRICT,
    ADD COLUMN planner_agent_id     UUID    NULL
        CONSTRAINT fk_step_graph_phase_planner_agent REFERENCES agent(id),
    ADD COLUMN planner_model        TEXT    NULL,
    ADD COLUMN integrate_persona_id UUID    NULL
        CONSTRAINT fk_step_graph_phase_integrate_persona REFERENCES persona(id) ON DELETE RESTRICT,
    ADD COLUMN integrate_agent_id   UUID    NULL
        CONSTRAINT fk_step_graph_phase_integrate_agent REFERENCES agent(id),
    ADD COLUMN integrate_model      TEXT    NULL,
    ADD COLUMN max_parts            INTEGER NULL;

UPDATE step_graph_phase SET split = 'compete' WHERE fan_out > 1;

ALTER TABLE step_graph_phase
    ADD CONSTRAINT ck_phase_split CHECK (split IN ('none', 'compete', 'divide')),
    ADD CONSTRAINT ck_phase_split_fan_out CHECK ((split = 'compete') = (fan_out > 1)),
    ADD CONSTRAINT ck_phase_plan_scope CHECK (plan_scope IN ('parts', 'successors', 'graph')),
    ADD CONSTRAINT ck_phase_max_parts CHECK (max_parts >= 1),
    ADD CONSTRAINT ck_phase_divide_columns CHECK (split = 'divide' OR (
            planner_persona_id   IS NULL AND planner_agent_id   IS NULL AND planner_model   IS NULL
        AND integrate_persona_id IS NULL AND integrate_agent_id IS NULL AND integrate_model IS NULL
        AND max_parts IS NULL));

CREATE INDEX idx_step_graph_phase_planner_persona   ON step_graph_phase (planner_persona_id);
CREATE INDEX idx_step_graph_phase_integrate_persona ON step_graph_phase (integrate_persona_id);

-- D10: deferrable position and name uniques; D6: the key the edges reference.
ALTER TABLE step_graph_phase
    DROP CONSTRAINT step_graph_phase_graph_id_position_key,
    DROP CONSTRAINT step_graph_phase_graph_id_name_key,
    ADD CONSTRAINT uq_step_graph_phase_position UNIQUE (graph_id, position)
        DEFERRABLE INITIALLY IMMEDIATE,
    ADD CONSTRAINT uq_step_graph_phase_name UNIQUE (graph_id, name)
        DEFERRABLE INITIALLY IMMEDIATE,
    ADD CONSTRAINT uq_step_graph_phase_graph_id UNIQUE (graph_id, id);

-- D6: edges.
CREATE TABLE step_graph_edge (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    graph_id      UUID        NOT NULL
        CONSTRAINT fk_step_graph_edge_graph REFERENCES step_graph(id) ON DELETE CASCADE,
    from_phase_id UUID        NOT NULL,
    to_phase_id   UUID        NOT NULL,
    kind          TEXT        NOT NULL,
    predicate     JSONB       NULL,
    loop_limit    INTEGER     NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT fk_step_graph_edge_from FOREIGN KEY (graph_id, from_phase_id)
        REFERENCES step_graph_phase (graph_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_step_graph_edge_to FOREIGN KEY (graph_id, to_phase_id)
        REFERENCES step_graph_phase (graph_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_step_graph_edge_kind
        CHECK (kind IN ('next', 'if', 'asks_you', 'changes_requested', 'fails')),
    CONSTRAINT ck_step_graph_edge_predicate CHECK ((kind = 'if') = (predicate IS NOT NULL)),
    CONSTRAINT ck_step_graph_edge_loop_limit CHECK (loop_limit IS NULL OR loop_limit >= 0),
    CONSTRAINT ck_step_graph_edge_loop_limit_required
        CHECK (kind NOT IN ('changes_requested', 'fails') OR loop_limit IS NOT NULL),
    CONSTRAINT ck_step_graph_edge_not_self CHECK (from_phase_id <> to_phase_id),
    CONSTRAINT uq_step_graph_edge UNIQUE (from_phase_id, to_phase_id, kind)
);

CREATE INDEX idx_step_graph_edge_graph ON step_graph_edge (graph_id);
CREATE INDEX idx_step_graph_edge_to    ON step_graph_edge (to_phase_id);

-- D8: a divided node's default parts.
CREATE TABLE phase_part (
    phase_id    UUID    NOT NULL
        CONSTRAINT fk_phase_part_phase REFERENCES step_graph_phase(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    key         TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    brief       TEXT    NOT NULL DEFAULT '',
    owned_paths TEXT[]  NOT NULL DEFAULT '{}',
    acceptance  TEXT    NULL,
    agent_id    UUID    NULL CONSTRAINT fk_phase_part_agent REFERENCES agent(id),
    model       TEXT    NULL,
    persona_id  UUID    NULL
        CONSTRAINT fk_phase_part_persona REFERENCES persona(id) ON DELETE RESTRICT,
    CONSTRAINT pk_phase_part PRIMARY KEY (phase_id, position),
    CONSTRAINT uq_phase_part_key UNIQUE (phase_id, key)
);

CREATE INDEX idx_phase_part_persona ON phase_part (persona_id);

-- D9: today's implied topology, as data.
INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind)
SELECT graph_id, id, next_id, 'next'
  FROM (SELECT graph_id, id,
               LEAD(id) OVER (PARTITION BY graph_id ORDER BY position) AS next_id
          FROM step_graph_phase) ordered
 WHERE next_id IS NOT NULL;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind, loop_limit)
SELECT r.graph_id, r.id, t.id, 'changes_requested', t.retry_limit
  FROM step_graph_phase r
  CROSS JOIN LATERAL (SELECT c.id, c.retry_limit
                        FROM step_graph_phase c
                       WHERE c.graph_id = r.graph_id
                         AND c.position < r.position
                       ORDER BY (c.name = 'implement') DESC, c.position DESC
                       LIMIT 1) t
 WHERE r.name = 'review';

COMMENT ON COLUMN step_graph.project_id IS
    'MOD-137 D3: the owning project; NULL for a workspace graph or a library graph. At most one '
    'of project_id and workspace_id is set (ck_step_graph_owner).';
COMMENT ON COLUMN step_graph.workspace_id IS
    'MOD-137 D3: the owning workspace; NULL for a project graph or a library graph (neither set). '
    'ON DELETE CASCADE.';
COMMENT ON COLUMN step_graph.revision IS
    'MOD-137 D5: whole-graph compare-and-set token; every write to the graph, its phases, '
    'candidates, parts or edges bumps it by one; replace_graph compares it.';
COMMENT ON COLUMN step_graph_phase.split IS
    'MOD-137 D7: none, compete (fan_out > 1, rival candidates) or divide (planner, parts, '
    'integrate; ANA-33 7.4); compete iff fan_out > 1.';
COMMENT ON COLUMN step_graph_phase.palette IS
    'MOD-137 D7: a node a planner may instantiate but the walk never reaches on its own; has no '
    'edges.';
COMMENT ON COLUMN step_graph_phase.plan_scope IS
    'MOD-137 D7: what a planner node may plan: parts, successors or graph (ANA-33 7.5); NULL off a '
    'planner.';
COMMENT ON COLUMN step_graph_phase.max_parts IS
    'MOD-137 D7: the most parts a divide planner may propose; divide nodes only, like the six '
    'planner_* and integrate_* columns.';
COMMENT ON TABLE step_graph_edge IS
    'MOD-137 D6: one link between two phases of one graph; replaced, never edited. Acyclicity and '
    'reachability are the shared validator''s (MOD-138), not CHECKs.';
COMMENT ON COLUMN step_graph_edge.kind IS
    'MOD-137 D6: next, if (carries predicate), asks_you, changes_requested or fails (both carry '
    'loop_limit).';
COMMENT ON COLUMN step_graph_edge.predicate IS
    'MOD-137 D6: the if edge''s condition, NULL on every other kind; MOD-138 closes its shape '
    '(ANA-33 7.3).';
COMMENT ON COLUMN step_graph_edge.loop_limit IS
    'MOD-137 D2, D6: how often a loop edge may be taken (ANA-33 limit; LIMIT is reserved); '
    'required on changes_requested and fails.';
COMMENT ON TABLE phase_part IS
    'MOD-137 D8: a divide node''s default parts (ANA-33 7.4, 7.6); written only by replace_graph.';
```

Constraint names, so nobody has to re-derive them:

- `fk_step_graph_workspace`, `ck_step_graph_owner`, `ck_step_graph_override_in_project`, `uq_step_graph_{project,workspace,library}_name`.
- `ck_phase_split`, `ck_phase_split_fan_out`, `ck_phase_plan_scope`, `ck_phase_max_parts`, `ck_phase_divide_columns`, `fk_step_graph_phase_{planner,integrate}_{persona,agent}`, `uq_step_graph_phase_{position,name,graph_id}`.
- `fk_step_graph_edge_{graph,from,to}`, `ck_step_graph_edge_{kind,predicate,loop_limit,loop_limit_required,not_self}`, `uq_step_graph_edge`.
- `fk_phase_part_{phase,agent,persona}`, `pk_phase_part`, `uq_phase_part_key`.
- Indexes: `idx_step_graph_phase_{planner,integrate}_persona`, `idx_step_graph_edge_{graph,to}`, `idx_phase_part_persona`.

The new persona FKs are **NOT DEFERRABLE**, which `delete_persona`'s pass loop relies on (`write.rs:3829-3834`).

### T1.2 Count and pin updates (same commit)

- `tests/connect.rs:140,242`: `Pending(19)` → `Pending(20)`. `:141-142,156-158,243-244`: "nineteen … through MOD-125's 0019_stop_runs.sql" → "twenty … through MOD-137's 0020_step_graph_v2.sql".
- `tests/migrations.rs`:
  - `TABLES` (`:25-84`): append `"step_graph_edge", "phase_part"` after `workspace_stop`, under a `// 0020_step_graph_v2.sql (MOD-137)` comment. The doc says "The 48 tables …". The asserts at `:133`, `:144` become `48`, and the prose gains "MOD-137's 0020_step_graph_v2.sql adds step_graph_edge and phase_part".
  - Versions `:105-115`: `…, 19, 20]` plus the prose "and MOD-137's 0020_step_graph_v2.sql".
  - `:1903,:2004` `Pending(20)`. `:2013,:2036` `MigrationsPending(20)`. Prose at `:1904,:2005,:2014,:2037,:2189-2190`: "twenty … through MOD-137's 0020_step_graph_v2.sql".
  - New `MOD137_COLUMN_COMMENTS: &[(&str, &str, &str)]` holds the ten column texts above, verbatim and in migration order. Chain it after `MOD125_COLUMN_COMMENTS` at `:880`, `:941`, `:960`. New `MOD137_TABLE_COMMENTS` holds `step_graph_edge` and `phase_part`, added to the `for (table, expected) in [ANA_TABLE_COMMENT, MOD125_TABLE_COMMENT]` loop (`:904`, which becomes an iterator over the three plus the two).
  - `plant_project_kind` (`:1716`) and the trigger test (`:2347-2373`) stay unchanged: `INSERT INTO step_graph (id, project_id, name)` is still valid.

### T1.3 Probe result (2026-10-21, `CREATE DATABASE bp137 TEMPLATE scratch137` on `localhost:5439`, since dropped)

Before 0020 the copy (0001..0019) was seeded by hand with one project and six graphs:

- `feature` (prd, plan, implement `fan_out 2 retry 3`, review);
- `bug` (reproduce, fix `retry 2`, review);
- `analysis` (research, verdict);
- `gappy` (implement@0 `retry 4`, docs@5, review@9);
- `gap-no-implement` (draft@0 `retry 2`, review@4);
- `solo` (a lone review@0).

`psql -v ON_ERROR_STOP=1 -f 0020_step_graph_v2.sql` → applied. Result:

```
      graph       | fpos | from_phase | tpos | to_phase  |       kind        | loop_limit
------------------+------+------------+------+-----------+-------------------+------------
 analysis         |    0 | research   |    1 | verdict   | next              |
 bug              |    0 | reproduce  |    1 | fix       | next              |
 bug              |    1 | fix        |    2 | review    | next              |
 bug              |    2 | review     |    1 | fix       | changes_requested |          2
 feature          |    0 | prd        |    1 | plan      | next              |
 feature          |    1 | plan       |    2 | implement | next              |
 feature          |    2 | implement  |    3 | review    | next              |
 feature          |    3 | review     |    2 | implement | changes_requested |          3
 gap-no-implement |    0 | draft      |    4 | review    | next              |
 gap-no-implement |    4 | review     |    0 | draft     | changes_requested |          2
 gappy            |    0 | implement  |    5 | docs      | next              |
 gappy            |    5 | docs       |    9 | review    | next              |
 gappy            |    9 | review     |    0 | implement | changes_requested |          4
(13 rows)            -- `solo` has none: a review at position 0 has no target (loop_target None)
```

- `split`: only `feature/implement` (`fan_out 2`) became `compete`.
- Uniques: `uq_step_graph_phase_{name,position}` are `condeferrable = t, condeferred = f`; `uq_step_graph_phase_graph_id` is `f`.
- Every graph has `revision 0`.
- 12 `MOD-137` comments are present.

Refusal probes, each with its exact constraint:

- both owners → `ck_step_graph_owner`;
- a library duplicate → `uq_step_graph_library_name`;
- a workspace duplicate → `uq_step_graph_workspace_name`;
- a project duplicate → `uq_step_graph_project_name`;
- the same name across the three scopes → ok;
- `is_override` in the library → `ck_step_graph_override_in_project`;
- `split = 'compete'` with `fan_out 1`, and `fan_out 2` with `split = 'none'` → both `ck_phase_split_fan_out`;
- `max_parts` on a `none` node → `ck_phase_divide_columns`;
- a `divide` node with `max_parts 2, planner_model` → ok, and `max_parts 0` → `ck_phase_max_parts`;
- a cross-graph edge → `fk_step_graph_edge_to`;
- `if` without a predicate → `ck_step_graph_edge_predicate`;
- `fails` without a limit → `ck_step_graph_edge_loop_limit_required`;
- a self edge → `ck_step_graph_edge_not_self`;
- a duplicate edge → `uq_step_graph_edge`.

Transaction probes:

- A position+name swap under `SET CONSTRAINTS uq_step_graph_phase_position, uq_step_graph_phase_name DEFERRED` commits.
- A real clash under `DEFERRED` surfaces at `SET CONSTRAINTS ALL IMMEDIATE` as `uq_step_graph_phase_position` (the F6 backstop).
- `DELETE` of `bug/fix` → 0 edges left in `bug`.
- `phase_part` with the column `key` → ok (non-reserved).
- `UPDATE step_graph SET revision = revision + 1 WHERE id = $1 AND revision = 0 RETURNING revision` → `1`; the same again → no row (stale).

### T1 tests (first)

| Test (`tests/migrations.rs`, Pg) | Asserts |
|---|---|
| `migration_0020_backfills_next_and_review_edges` | `common::bare_db()`, `MIGRATOR.run_to(19, …)` (the `migration_0019_*` staging at `:1465`). Raw-insert all five `seed::KINDS` graphs through `seed::graph_row`/`phase_row` (bind columns by hand; `implement` and `fix` get `retry_limit 3` to tell bounds apart), plus `gappy` and `gap-no-implement` as in T1.3. Then plain `run`. Each graph has `n-1` `next` edges between neighbours in position order. `feature`/`refactor`/`tooling` have `review → implement`, `bug` has `review → fix`, `analysis` has no loop edge, `gappy` has `review@9 → implement@0`, `gap-no-implement` has `review@4 → draft@0`. Every `loop_limit` is the target's `retry_limit`; `next` edges have `NULL` predicate and limit |
| `migration_0020_marks_fanned_out_phases_compete` | v19 phase `fan_out 2` → `split = 'compete'`, the rest `none`; `palette` false; all seven divide columns `NULL` |
| `migration_0020_owner_scope_refuses_what_it_should` | both owners → `23514 ck_step_graph_owner`; duplicate per scope → `23505` naming the scope's index; the same name in project + workspace + library → ok; library `is_override` → `23514 ck_step_graph_override_in_project`; `revision` defaults to 0; deleting the workspace removes its graph |
| `migration_0020_edges_stay_inside_their_graph` | cross-graph → `23503 fk_step_graph_edge_to`; the `kind`/`predicate`/`loop_limit`/`not_self` CHECKs refuse by name; phase delete cascades its edges; graph delete cascades edges and parts |
| `migration_0020_phase_uniques_are_deferrable` | `pg_constraint`: the two `condeferrable` and not `condeferred`; a swap of positions and names in one tx under `SET CONSTRAINTS … DEFERRED` commits |
| existing comment tests (`:866`, `:913`) | with `MOD137_*` chained: exact texts and exactly these commented columns |
| existing `connect.rs` / `migrations.rs` count tests | 20 / 48 |

The D9 **equality** test needs `seed::default_edges`, so it lands with T3 (§T3.6).

Between T1 and T2 the Rust code still compiles. The Pg suites are only guaranteed green again at
T2, for two reasons. The existing `create_phase` does not bind `split`. And the D10 deferrable
uniques change the order in which Postgres reports violations (T1 repair 1): a deferrable unique is
checked by an `AFTER ROW` `Unique_ConstraintTrigger_*`, triggers fire in name order, so every
`RI_ConstraintTrigger_c_*` FK check runs first. A phase write that both clashes on position or name
and names an unknown persona is therefore refused by `fk_step_graph_phase_persona`, not by the
clash. At HEAD after T1, `pg_store_conformance` stops on `a_phase_naming_no_persona_is_refused`
("a create with a taken position naming no persona (D5: the clash first)") for this reason; that
case passing is a T2 exit criterion (§T2.2). `migration_0020_deferrable_uniques_report_after_foreign_keys`
pins the database order. T1's gate is therefore the two named test binaries.

### T1 commits

1. `MOD-137 T1: 0020_step_graph_v2.sql and its migration tests` (the SQL, `connect.rs`, `migrations.rs`).

---

## T2 — Model, scope and the compile fan-out

### T2.1 Model (`kind.rs`, `ids.rs`, `model/mod.rs`): §1.2 verbatim

`model/mod.rs` tests (`:369`) gain:

```rust
    #[test]
    fn graph_v2_enums_match_check_lists() {
        check_enum(Split::ALL, &["none", "compete", "divide"]);
        check_enum(PlanScope::ALL, &["parts", "successors", "graph"]);
        check_enum(EdgeKind::ALL, &["next", "if", "asks_you", "changes_requested", "fails"]);
    }
```

`kind.rs` unit tests:

- `graph_scope_round_trips_through_serde`: `{"owner":"project","id":…}`, `{"owner":"workspace","id":…}`, `{"owner":"library"}`.
- `graph_scope_from_columns_refuses_both`.
- `graph_scope_display_spells_the_three_owners`.
- `split_defaults_to_none`.
- `a_v1_phase_json_decodes_with_v2_defaults`: a `StepGraphPhase` serialised without the ten keys decodes with `Split::None`, `false`, `None`s.
- `edge_kind_rules_match_the_checks`: `takes_predicate` is only `If`, and `needs_loop_limit` is exactly `ChangesRequested | Fails`.

### T2.2 Postgres row types and lists (`pg/read.rs`, `pg/write.rs`, `pg/demo.rs`)

```rust
/// `step_graph` as Postgres hands it over (MOD-137 D3): the two owner columns, folded into
/// [`GraphScope`] by `TryFrom`. Field order is the SELECT order.
struct StepGraphRow {
    id: StepGraphId, project_id: Option<ProjectId>, workspace_id: Option<WorkspaceId>,
    name: String, description: String, is_override: bool, revision: i64,
    created_at: DateTime<Utc>, updated_at: DateTime<Utc>,
}
impl TryFrom<StepGraphRow> for StepGraph { type Error = StoreError; /* from_columns; None → Backend("step_graph `<id>` has both project_id and workspace_id (ck_step_graph_owner)") */ }
```

The one select list, used by `step_graph_row` (`read.rs:2781`), `step_graph_rows` (`:2925`), the
`RETURNING` of `create_step_graph` (`write.rs:2972`) and of `update_step_graph` (`:3011`):

```sql
id           AS "id: StepGraphId",
project_id   AS "project_id: ProjectId",
workspace_id AS "workspace_id: WorkspaceId",
name, description, is_override, revision, created_at, updated_at
```

`step_graph_rows` keeps `WHERE project_id = $1` (project scope only, D4). `create_step_graph`
binds `scope.project_id()`, `scope.workspace_id()`. Before the statement it refuses
`is_override && !matches!(scope, Project(_))` with `override_outside_project(scope)`, the same
sentence Mem uses.

Phase select list: the existing list plus, after `persona_id` and before `updated_at`:

```sql
split                AS "split: Split",
palette,
plan_scope           AS "plan_scope: PlanScope",
planner_persona_id   AS "planner_persona_id: PersonaId",
planner_agent_id     AS "planner_agent_id: AgentId",
planner_model,
integrate_persona_id AS "integrate_persona_id: PersonaId",
integrate_agent_id   AS "integrate_agent_id: AgentId",
integrate_model,
max_parts,
```

It goes at `read.rs:2809` (`phase_row`), `read.rs:2955` (`phase_rows`), the `RETURNING` of
`write.rs:3064` (`create_phase`) and of `write.rs:3193` (`update_phase`). `create_phase`'s
`INSERT` binds the ten new columns (`$18..$27`). Factor that `INSERT … RETURNING` into
`async fn insert_phase<'e, E: PgExecutor<'e>>(e: E, phase: &StepGraphPhase) -> Result<StepGraphPhase>`,
so `replace_graph` (T3) reuses the byte-identical statement and its `.sqlx` entry. `seed_project`
(`write.rs:7990`) and `load_demo` (`demo.rs:242`) keep their column lists: the defaults equal
`phase_row`'s. Their `step_graph` inserts (`write.rs:7977`, `demo.rs:226`) bind
`graph.scope.project_id()` (`Option<Uuid>`).

`create_phase` refusal order, both stores:

1. `reserved_phase_name`;
2. **`phase_row_refusal(&phase)`** (new, §T3.2; T2 adds the function's node part to `graph_rules.rs`);
3. the graph exists;
4. position/name clashes;
5. `persona_id`, `planner_persona_id`, `integrate_persona_id` → `references_no_row("step_graph_phase.<column>", id, "persona")`;
6. `planner_agent_id`, `integrate_agent_id` → `references_no_row("step_graph_phase.<column>", id, "agent")`.

Pg cannot take step 4 from the database any more (T1 repair 1): after 0020 the deferrable
`uq_step_graph_phase_{position,name}` are checked after every FK trigger, so the `INSERT`/`UPDATE`
reports an unknown persona or agent before a clash. Pg `create_phase` and `update_phase` therefore
decide step 4 themselves, inside the transaction §1.4 opens, after `bump_graph` has taken the
graph row lock (so no concurrent writer of the graph can slip a clashing row in between): one
pre-read of `(graph_id, position)` and `(graph_id, name)` excluding the row's own id, against the
row as it will be written. For `update_phase` that is the patch applied over the current row, and
Mem's order (`mem.rs` `update_phase`: not found, then `Stale`, then the clash, then the persona)
means the pre-read runs only once the row is read `FOR UPDATE` and its `updated_at` equals the
token; a stale token still answers `Stale` over a clash. A hit rolls
back and answers Mem's position or name sentence (`mem.rs:3589`, `:3594`), position first, so
both stores word a clash the same. `SET CONSTRAINTS … IMMEDIATE` does not help: an
immediate deferrable unique is still checked by its after-row trigger, after the FK triggers. The
unique stays as the backstop for anything the pre-read misses.

Pg reaches step 5 and 6 through `phase_persona_refused` (`write.rs:808`). It is generalised to
`phase_fk_refused(err, phase: &StepGraphPhase)` over a const table of `(constraint, column,
Option<id>, target table)` for the five FKs. Anything else goes through `map_sqlx`. This is sound
only because step 4 was decided before the write.

### T2.3 MemStore (`store/mem.rs`)

- `State` gains `edges: BTreeMap<EdgeId, StepGraphEdge>` and `phase_parts: BTreeMap<(PhaseId, i32), PhasePart>`. `from_demo` (`:370`) fills `edges` from `data.edges` once T3 adds the field (T2: `BTreeMap::new()`); `phase_parts: BTreeMap::new()`.
- `create_step_graph` (`:3475`), order:
  1. owner exists: `Project` → today's `"step_graph.project_id `{}` references no project"`; `Workspace` → `references_no_row("step_graph.workspace_id", w, "workspace")`; `Library` needs nothing;
  2. `override_outside_project`;
  3. `already_exists`;
  4. `graph_name_is_taken(name, scope)`, comparing `row.scope == new.scope`;
  5. the literal sets `revision: 0`.
- `update_step_graph` (`:3512`): the clash compares `row.scope == current.scope` and words it with `graph_name_is_taken`. It bumps (§1.4).
- `check_item_kind` (`:3341`): `graphs.get(&graph).is_none_or(|row| row.scope != GraphScope::Project(project))`, a behaviour-identical refusal (D4).
- `step_graph_rows` (`:3559`) and `project_reach` (`:4723`): `row.scope == GraphScope::Project(…)`.
- `set_skill_binding` (`:4141`): `.and_then(|graph| graph.scope.project_id())` (F9).
- `delete_project` (`:5008-5020`): also `edges.retain(|_, e| !gone.graphs.contains(&e.graph_id))` and `phase_parts.retain(|(p, _), _| !gone.phases.contains(p))`, before `phases`. The test `delete_project_leaves_no_row_in_any_map` (`:11911`) asserts both maps.
- `delete_workspace` (`:4898`) and `workspace_reach` (`:4666`): collect `gone = graphs where scope == Workspace(id)` and their phases. Remove their `phase_agents`, `phase_parts`, `edges`, `skill_bindings` with a `phase_id` among them (none can exist after F9, but the cascade is the FK's), then the phases and the graphs. The reach reports `step_graphs`, `phases` and `phase_agents`. No new `DeleteReach` field: that would mean ten more sites, `hierarchy.rs:730` among them, for counts nobody draws.
- `create_phase` (`:3601`): the order of §T2.2, then `bump_graph`. `create_phase_agents`, `update_phase`, `set_setting`/`clear_setting` Phase arms bump (§1.4).
- `delete_persona` (`:4012`): §T2.7.

### T2.4 Seeds and fixtures (`seed.rs`, `fixtures.rs`)

- `seed::graph_row` (`:193`): `scope: GraphScope::Project(project_id), revision: 0`. The signature is unchanged.
- `seed::phase_row` (`:216`): the ten new fields at their defaults (`split: Split::None`, `palette: false`, the rest `None`).
- Test `seed.rs:498` (`rows_carry_the_frozen_defaults`): `graph.scope == GraphScope::Project(p)` and the ten defaults.

### T2.5 Compile fan-out: every site

**`StepGraphPhase {` (14):**

| Site | Kind | Change |
|---|---|---|
| `htui-core/src/seed.rs:216` `phase_row` | prod, full | ten defaults |
| `htui-core/src/store/conformance.rs:2752` `new_phase` | test-support, full | ten defaults |
| `htui-core/src/store/conformance.rs:18619,18639,19055` | spread `..new_phase(..)` | none |
| `htui-orch/src/conformance.rs:1221` (`repoint`), `:1289`, `:1303` (`insert_verify_phase`) | spread | **F12**: after `mutate(&mut edited)` in `repoint` (and after the `fan_out` edit in `insert_verify_phase`) add `if edited.split != Split::Divide { edited.split = if edited.fan_out > 1 { Split::Compete } else { Split::None }; }` with the comment "ck_phase_split_fan_out (MOD-137 D7)" |
| `htui-orch/src/engine.rs:7883` (test `repoint` at `:7851`) | spread | F12, as above |
| `htui-orch/tests/gix_isolator.rs:361` (`repoint` at `:336`) | spread | F12, as above |
| `htui-orch/src/graph.rs:564` (override clone) | prod, spread | none in T2 (T5 rewrites it) |
| `htui-orch/tests/fixtures.rs:70,77` | spread | none |
| `htui-worker/tests/auto_queue.rs:845` | spread | none |

**`StepGraph {` (5):** `seed.rs:193` (prod), `mem.rs:3498` (prod), `model/item_spec.rs:533` (test),
`htui/src/skills.rs:619` (test) and `htui/tests/kinds.rs:997` (test, inside `GraphEntry`; T4
adds `edges`) all become `scope: GraphScope::Project(…)` + `revision: 0`.

**`NewStepGraph {` (17): `project_id: X` → `scope: GraphScope::Project(X)`.**
`htui-orch/src/graph.rs:547` (prod), `htui/src/catalogue.rs:213` (prod,
`StoreRequest::CreateGraph` keeps `project`), core `conformance.rs:4124,4135,4373,19045`, orch
`conformance.rs:1211,1272`, `engine.rs:7873`, `htui-orch/tests/fixtures.rs:54`,
`tests/gix_isolator.rs:351`, `htui-worker/tests/auto_queue.rs:829`,
`htui/src/item_writes.rs:509,743`, `htui/src/ui/tabs/backlog/item_form.rs:1736,2336`.

**`.project_id` on a `StepGraph` / `NewStepGraph`:**

| Site | Fn | Change |
|---|---|---|
| `mem.rs:3341` | `check_item_kind` | `row.scope != GraphScope::Project(project)` |
| `mem.rs:3476-3500` | `create_step_graph` | §T2.3 |
| `mem.rs:3532,3537` | `update_step_graph` | §T2.3 |
| `mem.rs:3559` | `step_graph_rows` | scope filter |
| `mem.rs:4024` | `delete_persona` | §T2.7 |
| `mem.rs:4141` | `set_skill_binding` | F9 |
| `mem.rs:4723` | `project_reach` | scope filter |
| `mem.rs:11911` | test | `row.scope != GraphScope::Project(gone)` |
| `pg/write.rs:2982` | `create_step_graph` | bind both columns |
| `pg/write.rs:3562` | `set_skill_binding` | F9 |
| `pg/write.rs:7979` | `seed_project` | `graph.scope.project_id().map(ProjectId::as_uuid)` |
| `pg/demo.rs:229` | `load_demo` | same |
| core `conformance.rs:3468` | `project_create_seeds_the_catalogue` | `graph.scope == GraphScope::Project(p)` |
| `seed.rs:498` | test | same |

SQL that keeps working unchanged under D4: `write.rs:518` (`project_reach` CTE),
`:2745`/`:2831` (`create/update_item_kind` same-project `EXISTS`), and the `pg_criteria.rs`
statements at `:3106`, `:3612`. A `NULL` `project_id` never equals `$1`.

**Pg selects/returnings in struct order:** `read.rs:2781,2809,2925,2955`; `write.rs:2972,3011,3072,3204`
(§T2.2). The entry points that delegate to them need no edit: `read.rs:1826` (`step_graph`) and
`:1912/1929` (`resolve_graph`).

### T2.6 Writers' bumps: §1.4 rows 1–7, both stores

### T2.7 `delete_persona` (D7, F8)

- Mem (`mem.rs:4012`). A phase **holds** persona `id` when `persona_id`, `planner_persona_id` or `integrate_persona_id == Some(id)`, or any `phase_parts` row of it has `persona_id == Some(id)`. The holder triple's first element is the graph owner label: the project slug for `Project`, `workspace:<slug>` for `Workspace`, `library` for `Library`, and `?` when the owner row is missing (unreachable). The wording is still `persona_is_bound`.
- Pg (`write.rs:3848-3850`) guard:
  ```sql
  DELETE FROM persona
   WHERE id = $1
     AND NOT EXISTS (SELECT 1 FROM step_graph_phase
                      WHERE persona_id = $1 OR planner_persona_id = $1 OR integrate_persona_id = $1)
     AND NOT EXISTS (SELECT 1 FROM phase_part WHERE persona_id = $1)
  RETURNING id
  ```
  Holders (`:3867`):
  ```sql
  SELECT COALESCE(pr.slug, 'workspace:' || w.slug, 'library') AS "owner!", g.name AS "graph!", p.name AS "phase!"
    FROM step_graph_phase p
    JOIN step_graph g ON g.id = p.graph_id
    LEFT JOIN project   pr ON pr.id = g.project_id
    LEFT JOIN workspace w  ON w.id  = g.workspace_id
   WHERE p.persona_id = $1 OR p.planner_persona_id = $1 OR p.integrate_persona_id = $1
      OR EXISTS (SELECT 1 FROM phase_part pp WHERE pp.phase_id = p.id AND pp.persona_id = $1)
  ```
  Update the comment at `:3829-3834` ("the join cannot name" now means only a missing parent).
- `persona_is_bound`'s doc (`traits.rs:2619-2624`) says "a graph name is unique only per project". It becomes "per scope; the owner is the project slug, `workspace:<slug>` or `library`".

### T2 tests (first)

| Test | Backend | Asserts |
|---|---|---|
| `model/mod.rs::graph_v2_enums_match_check_lists` | unit | §T2.1 |
| `kind.rs` six tests | unit | §T2.1 |
| `seed.rs::rows_carry_the_frozen_defaults` (extended) | unit | scope + ten defaults |
| `mem.rs::delete_project_leaves_no_row_in_any_map` (extended) | Mem | `edges`, `phase_parts` empty for the project |
| existing core conformance, orch, worker, htui suites | Mem + Pg | green: no behaviour change beyond the bump |
| existing `a_phase_naming_no_persona_is_refused` (core conformance) | Pg | green again (red since T1, §T1 tests): the clash is refused before the persona through §T2.2's pre-read, for `create_phase` and `update_phase` |

The cases that pin the bump, the cascade and the persona references are T3's (plan): §T3.6
cases 11, 14 and 16.

### T2 commits (each compiles)

1. `MOD-137 T2: graph v2 model types` (additive: `EdgeId`, `GraphScope`, `Split`, `PlanScope`, `EdgeKind`, `EdgePredicate`, `StepGraphEdge`, `PhasePart`, `NodeBody`, `GraphBody`, unit tests).
2. `MOD-137 T2: StepGraph.scope and revision, the D7 phase fields, and the fan-out`. This is the breaking field change plus every site of §T2.5, `StepGraphRow`, the Pg lists, `graph_rules::phase_row_refusal`, the `create_phase` FK mapping, the F12 helpers and `.sqlx` (prepared, §3).
3. `MOD-137 T2: every graph content write bumps revision` (§1.4 rows 1–7, both stores, `.sqlx`).
4. `MOD-137 T2: delete_persona counts divide and part personas; delete_workspace takes its graphs` (§T2.3, §T2.7, the Mem maps, `.sqlx`).

---

## T3 — Store verbs, edges in seeds

### T3.1 `WriteStore` (`store/traits.rs`, a new block after `phases` at `:952`)

```rust
    // MOD-137: graph v2 writers and reads (plan D10, D11)

    /// Replaces a graph's editable content in one write (D10). From `body.graph` only `name` and
    /// `description` are written; its `id` must be `graph`, and `scope`, `is_override`, `revision`
    /// and the timestamps are ignored. Nodes are diffed **by phase id**: a kept id is updated in
    /// place and keeps its `skill_binding` rows; a new id is inserted; a stored id missing from
    /// `body` is deleted with its candidates, parts, edges and phase bindings. Every node's
    /// candidates and parts, and the graph's edges, are replaced wholesale. `revision` + 1.
    ///
    /// Answers `Applied` with the body as stored (§1.3 order, every `updated_at` the store's
    /// clock). The phase columns [`StepGraphPhase`] does not carry (`judge_agent_id`,
    /// `judge_model`, `deadline_seconds`) keep their values on a kept node and are `NULL` on a new
    /// one. Store-level checks only: acyclicity, back-edge kinds and reachability are MOD-138's
    /// shared validator, which this writer will call once it exists.
    ///
    /// # Errors
    /// In this order, and nothing is written on any of them:
    /// 1. [`StoreError::NotFound`] `"step_graph"` for an unknown `graph`;
    /// 2. `Ok(CasOutcome::Stale(current body))` when `expected_revision` is not the stored one;
    /// 3. [`StoreError::Constraint`] with [`graph_body_shape_refusal`]'s sentence (ids of the body
    ///    against `graph` and their own node, reserved names, duplicate ids, position and name
    ///    clashes, candidate and part position/key clashes, duplicate edge ids);
    /// 4. `Constraint` [`graph_name_is_taken`] when the new name is taken in the graph's scope;
    /// 5. `Constraint` [`already_exists`] for a phase or edge id held by another graph;
    /// 6. `Constraint` [`references_no_row`] for the first persona, then the first agent, that names
    ///    no row, in [`body_persona_refs`] / [`body_agent_refs`] order;
    /// 7. `Constraint` with [`graph_body_topology_refusal`]'s sentence (edge endpoints, edge CHECKs,
    ///    duplicate edges, node CHECKs, palette edges, parts on a non-divide node).
    async fn replace_graph(
        &self,
        graph: StepGraphId,
        expected_revision: i64,
        body: GraphBody,
    ) -> Result<CasOutcome<GraphBody>>;

    /// Deletes a phase with its candidates, parts, edges and phase bindings (D11); the graph's
    /// last phase may go. `revision` + 1.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `"step_graph_phase"` for an unknown id.
    async fn delete_phase(&self, id: PhaseId) -> Result<()>;

    /// Deletes one edge (D11). `revision` + 1.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `"step_graph_edge"` for an unknown id.
    async fn delete_edge(&self, id: EdgeId) -> Result<()>;

    /// Deletes every candidate of `phase` (D11); the editor re-adds through
    /// [`create_phase_agents`](Self::create_phase_agents). `revision` + 1, also when the phase had none.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] `"step_graph_phase"` for an unknown phase.
    async fn delete_phase_agents(&self, phase: PhaseId) -> Result<()>;

    /// A graph's edges by (from position, to position, kind bytes); empty for an unknown graph.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn graph_edges(&self, graph: StepGraphId) -> Result<Vec<StepGraphEdge>>;

    /// A phase's parts by `position`; empty for an unknown phase.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn phase_parts(&self, phase: PhaseId) -> Result<Vec<PhasePart>>;

    /// The whole body [`replace_graph`](Self::replace_graph) writes, as stored (F10); `None` for an
    /// unknown graph. Any scope: the one reader of a workspace or library graph.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn graph_body(&self, graph: StepGraphId) -> Result<Option<GraphBody>>;
```

Implementors, one delegating arm each, in the same order:

- `MemStore` (wrappers after `mem.rs:8673`);
- `PgStore` (after `write.rs:3248`);
- `Writer` (after `writer.rs:731`, the `Self::Memory(store) / Self::Online(pg)` shape);
- `UsageSpy` (after `htui-agent/src/conformance.rs:997`);
- `SpyStore` (after `htui-agent/tests/recorder.rs:724`).

### T3.2 Pure rules (`store/graph_rules.rs`, both stores call them)

```rust
/// D7's node CHECKs on one row: ck_phase_split_fan_out, then ck_phase_divide_columns, then
/// ck_phase_max_parts. `None` when the row passes. `create_phase` and `replace_graph` call it.
pub fn phase_row_refusal(phase: &StepGraphPhase) -> Option<String>;

/// D10 step 3, first failure in this order:
///  a. `body.graph.id != graph` → row_names_another_graph("step_graph.id", body.graph.id, graph);
///  b. per node in body order: phase.graph_id → row_names_another_graph("step_graph_phase.graph_id", ..);
///     each agent → row_names_another_phase("phase_agent", agent.phase_id, node);
///     each part  → row_names_another_phase("phase_part", part.phase_id, node);
///     then each edge → row_names_another_graph("step_graph_edge.graph_id", edge.graph_id, graph);
///  c. per node: reserved_phase_name;
///  d. per node, the second occurrence: already_exists("step_graph_phase", id),
///     phase_position_is_taken, phase_name_is_taken;
///  e. per node: already_exists("phase_agent", "({phase}, {position})"),
///     already_exists("phase_part", "({phase}, {position})"), phase_part_key_is_taken;
///  f. edges, the second occurrence: already_exists("step_graph_edge", id).
pub fn graph_body_shape_refusal(graph: StepGraphId, body: &GraphBody) -> Option<String>;

/// D10 step 7, first failure in this order:
///  a. per edge: from, then to, not a node id → edge_endpoint_outside_graph("from_phase_id"|"to_phase_id", ..);
///  b. per edge: edge_loops_to_itself; edge_predicate_mismatch; edge_loop_limit_negative;
///     edge_needs_loop_limit;
///  c. edges, the second (from, to, kind): edge_is_taken;
///  d. per node: phase_row_refusal;
///  e. per edge: an endpoint whose node is `palette` → palette_phase_has_edges(that node's name, edge);
///  f. per node with parts and split != Divide → parts_off_divide.
pub fn graph_body_topology_refusal(graph: StepGraphId, body: &GraphBody) -> Option<String>;

/// D10 step 6's walk, personas first: per node, persona_id, planner_persona_id,
/// integrate_persona_id, then its parts' persona_id; each as (column, id) for references_no_row.
pub fn body_persona_refs(body: &GraphBody) -> Vec<(&'static str, PersonaId)>;
/// Then agents: per node, planner_agent_id, integrate_agent_id, its agents' agent_id
/// ("phase_agent.agent_id"), then its parts' agent_id ("phase_part.agent_id").
pub fn body_agent_refs(body: &GraphBody) -> Vec<(&'static str, AgentId)>;
```

The columns named are `"step_graph_phase.persona_id"`, `"step_graph_phase.planner_persona_id"`,
`"step_graph_phase.integrate_persona_id"`, `"phase_part.persona_id"` → table `"persona"`, and
`"step_graph_phase.planner_agent_id"`, `"step_graph_phase.integrate_agent_id"`,
`"phase_agent.agent_id"`, `"phase_part.agent_id"` → table `"agent"`.

Unit tests in the same file: one per lettered rule, plus `the_shape_rules_fire_in_order` and
`the_topology_rules_fire_in_order` (one body breaking all of them, mended one rule at a time).

### T3.3 MemStore `replace_graph` (decide, then mutate; `write()` has no rollback, `mem.rs:1355`)

1. `graph_row = graphs.get(graph)` else `NotFound "step_graph"`.
2. `graph_row.revision != expected_revision` → `Ok(Stale(self.graph_body(graph).expect(..)))`.
3. `graph_body_shape_refusal(graph, &body)` → `Constraint`.
4. Any other graph with `row.scope == graph_row.scope && row.name == body.graph.name` → `graph_name_is_taken`.
5. For each node id in body order: `self.phase(id)` is in another graph → `already_exists("step_graph_phase", id)`. For each edge id: `self.edges.get(id)` in another graph → `already_exists("step_graph_edge", id)`.
6. The first `body_persona_refs` entry not in `personas`, then the first `body_agent_refs` entry not in `agents` → `references_no_row(column, id, table)`.
7. `graph_body_topology_refusal` → `Constraint`.
8. **Mutate** (nothing below can fail):
   - `removed` = this graph's phase ids not in the body.
   - Drop `phase_agents` and `phase_parts` of every phase of the graph (removed and kept), every `edges` row with `graph_id == graph`, every `skill_bindings` row whose `phase_id` is in `removed`, and the `removed` phases.
   - For each node: replace the kept row in place, or push the new one, with `updated_at = now`.
   - Insert the agents, parts and edges.
   - Set the graph's `name` and `description`, then `bump_graph(graph, now)`.
9. `Ok(Applied(self.graph_body(graph).expect("written under the same lock")))`.

`State::graph_body(graph) -> Option<GraphBody>` assembles §1.3's order. The deleters:

- `delete_phase`: `NotFound` → remove the phase, its agents, parts, edges with `from` or `to` equal to it, and its skill bindings → bump.
- `delete_edge`: `NotFound` → remove → bump.
- `delete_phase_agents`: `NotFound` (unknown phase) → remove the range → bump.

### T3.4 PgStore `replace_graph` (one transaction)

```
tx = pool.begin()
SET CONSTRAINTS uq_step_graph_phase_position, uq_step_graph_phase_name DEFERRED
1. CAS + bump + lock:
   UPDATE step_graph SET revision = revision + 1
    WHERE id = $1 AND revision = $2
   RETURNING project_id AS "project_id: ProjectId", workspace_id AS "workspace_id: WorkspaceId"
   None → tx.rollback(); match self.graph_body(graph) { None → NotFound "step_graph",
          Some(b) → Ok(Stale(b)) }
2. graph_body_shape_refusal → rollback, Constraint
3. SELECT EXISTS (SELECT 1 FROM step_graph
                   WHERE id <> $1 AND name = $2
                     AND project_id IS NOT DISTINCT FROM $3
                     AND workspace_id IS NOT DISTINCT FROM $4) AS "taken!"
   → graph_name_is_taken(name, scope)
4. SELECT id AS "id: PhaseId" FROM step_graph_phase WHERE id = ANY($1) AND graph_id <> $2
   SELECT id AS "id: EdgeId"  FROM step_graph_edge  WHERE id = ANY($1) AND graph_id <> $2
   → the first in body order: already_exists(..)
5. SELECT id AS "id: PersonaId" FROM persona WHERE id = ANY($1) FOR KEY SHARE
   SELECT id AS "id: AgentId"   FROM agent   WHERE id = ANY($1) FOR KEY SHARE
   → the first body_*_refs entry missing: references_no_row(..)
   (FOR KEY SHARE: a concurrent delete_persona waits, so a checked persona cannot vanish)
6. graph_body_topology_refusal → rollback, Constraint
7. UPDATE step_graph SET name = $2, description = $3 WHERE id = $1
8. DELETE FROM step_graph_phase WHERE graph_id = $1 AND NOT (id = ANY($2))   -- $2 = kept ids;
   cascades phase_agent, phase_part, skill_binding, step_graph_edge
9. DELETE FROM step_graph_edge WHERE graph_id = $1
   DELETE FROM phase_agent WHERE phase_id = ANY($1)
   DELETE FROM phase_part  WHERE phase_id = ANY($1)
10. per kept node (same graph):
    UPDATE step_graph_phase SET position = $3, name = $4, fan_out = $5, gate = $6,
        gate_hard = $7, retry_limit = $8, input_kinds = $9, output_kind = $10, isolation = $11,
        command_queue = $12, verify_command = $13, template_name = $14,
        template_version = $15, token_budget = $16, persona_id = $17, split = $18,
        palette = $19, plan_scope = $20, planner_persona_id = $21, planner_agent_id = $22,
        planner_model = $23, integrate_persona_id = $24, integrate_agent_id = $25,
        integrate_model = $26, max_parts = $27
     WHERE id = $1 AND graph_id = $2
    per new node: insert_phase(&mut *tx, &node.phase)      -- §T2.2's shared statement
11. INSERT INTO phase_agent (phase_id, position, agent_id, model)
    SELECT t.phase_id, t.position, t.agent_id, t.model
      FROM UNNEST($1::uuid[], $2::int4[], $3::uuid[], $4::text[]) AS t(phase_id, position, agent_id, model)
    per part:  INSERT INTO phase_part (phase_id, position, key, title, brief, owned_paths,
               acceptance, agent_id, model, persona_id) VALUES ($1 .. $10)
    per edge:  insert_edge(&mut *tx, &edge)
               -- INSERT INTO step_graph_edge (id, graph_id, from_phase_id, to_phase_id, kind,
               --                              predicate, loop_limit) VALUES ($1 .. $7)
12. SET CONSTRAINTS ALL IMMEDIATE          -- F6 backstop: a missed clash is a statement error
    (23505 → map_sqlx → Constraint; the tx rolls back on drop)
13. body = graph_body_on(&mut *tx, graph) -- what this transaction wrote
14. tx.commit(); Ok(Applied(body))
```

Every statement maps through `map_sqlx`. Steps 2–6 decide every refusal the conformance suite
pins, so Pg's sentences are Mem's byte for byte. The per-row loops are deliberate: a graph is a
handful of rows, and `UNNEST` cannot carry a `text[]` per row (`input_kinds`, `owned_paths`).

`insert_edge` is also used by `seed_project` and `load_demo` (§T3.5), so the three share one
`.sqlx` entry. `graph_edges_on` / `phase_parts_on` / `graph_body_on` are generic over
`PgExecutor`, and `graph_edges` / `phase_parts` / `graph_body` call them on the pool:

```sql
-- graph_edges_on (EdgeRow: predicate as Option<serde_json::Value>, folded into EdgePredicate)
SELECT e.id AS "id: EdgeId", e.graph_id AS "graph_id: StepGraphId",
       e.from_phase_id AS "from_phase_id: PhaseId", e.to_phase_id AS "to_phase_id: PhaseId",
       e.kind AS "kind: EdgeKind", e.predicate, e.loop_limit
  FROM step_graph_edge e
  JOIN step_graph_phase f ON f.id = e.from_phase_id
  JOIN step_graph_phase t ON t.id = e.to_phase_id
 WHERE e.graph_id = $1
 ORDER BY f.position, t.position, e.kind COLLATE "C"

-- phase_parts_on (query_as! straight into PhasePart: field order = select order)
SELECT phase_id AS "phase_id: PhaseId", position, key, title, brief, owned_paths, acceptance,
       agent_id AS "agent_id: AgentId", model, persona_id AS "persona_id: PersonaId"
  FROM phase_part WHERE phase_id = $1 ORDER BY position
```

`graph_body_on` reads, in this order:

1. `step_graph_row`;
2. `phase_rows`;
3. the graph's candidates in one statement (`SELECT a.* … JOIN step_graph_phase p ON p.id = a.phase_id WHERE p.graph_id = $1 ORDER BY p.position, a.position`);
4. the parts the same way (`ORDER BY p.position, pp.position`);
5. `graph_edges_on`.

The candidates and parts are grouped by `phase_id` in Rust.

Pg deleters follow §1.4 rows 8–10: `pool.begin()`, the bump, `DELETE … WHERE id = $1`, commit.

### T3.5 Seeds: `default_edges` and its callers

```rust
/// D9: today's implied topology as edges, the rule 0020's backfill applies. Phases sorted by
/// `position` (any slice order): a `next` edge between each adjacent pair, then for each phase
/// named `review` a `changes_requested` edge to the greatest earlier phase named `implement`, else
/// to the phase just before it in position order (F4), with `loop_limit` = the target's
/// `retry_limit`; a `review` first in order gets none. All `phases` share one `graph_id`
/// (`debug_assert!`); empty in, empty out. `mint` names each edge (UUIDv7 in the stores,
/// `demo_uuid` in the fixture).
#[must_use]
pub fn default_edges(phases: &[StepGraphPhase], mut mint: impl FnMut() -> EdgeId) -> Vec<StepGraphEdge>
```

The output lists all `next` edges in position order, then the loop edges in review-position order.
The names `"implement"` and `"review"` are `seed`-local consts. The doc points at
`htui_orch::gate::loop_target` and `engine.rs:91` as the run-time twins: `htui-core` cannot
depend on `htui-orch`, and MOD-138 will retire the name rule.

Callers:

- **Mem `create_project`** (`mem.rs:3046-3051`): collect the graph's phase rows, then `for e in seed::default_edges(&rows, EdgeId::new) { self.edges.insert(e.id, e); }`.
- **Pg `seed_project`** (`write.rs:7966`): collect `row`s per graph, then `insert_edge(&mut *tx, &e)` for each `default_edges(&rows, EdgeId::new)`.
- **Fixture**: `fixtures.rs` gets `class::EDGE: u8 = 21`. `catalogue()` (`:764`) returns a fifth `Vec<StepGraphEdge>` built per graph with a running `edge_n`: `default_edges(&graph_phases, || { let id = EdgeId::from_uuid(demo_uuid(class::EDGE, edge_n)); edge_n += 1; id })`. That is 14 per project, 42 in total, below 256. `DemoData` gains `pub edges: Vec<StepGraphEdge>` after `phases`, and `demo_data()` (`:417-419`) unpacks it. `fixtures.rs:1353` becomes `let (_, graphs, phases, _, _) = catalogue();`.
- The two `DemoData { .., ..demo_data() }` literals (`htui/src/store_worker.rs:3965,4032`) need no change.
- **Pg `load_demo`** (`demo.rs:47`): after the phases, `insert_edge` for each of `data.edges`.

### T3.6 Tests (conformance first; each = `CASES` entry + `run_case` arm; 175 → **193**)

| # | Case | Asserts |
|---|---|---|
| 1 | `replace_graph_applies_nodes_edges_and_bumps_revision` | A fresh project graph with 3 phases (`create_phase`) and its `graph_body`. Replace it with: a renamed graph, one kept node edited, one node dropped, one new node, candidates on two nodes, edges `next ×2` + `asks_you` with `loop_limit 1`. The answer is `Applied(b)` with `b.graph.revision == r0 + 1`, nodes by position, the dropped id gone, the new id present, edges in §1.3 order. `graph_body == Some(b)`, `phases(graph)` and `graph_edges(graph)` equal `b`'s |
| 2 | `replace_graph_refuses_a_stale_revision_and_writes_nothing` | after case 1's shape, the same call with `r0` → `Stale(current)` where `current == graph_body` before the call; the body, revision, phases, agents, edges are unchanged. Unknown graph → `NotFound "step_graph"` |
| 3 | `replace_graph_keeps_bindings_of_kept_phases` | A phase binding (`set_skill_binding`) on kept phase K and on dropped phase D. After the replace, K's binding is still there and D's is gone (`skill_bindings(Some(project))`) |
| 4 | `replace_graph_swaps_positions_and_names` | A, B at 0/1 → A at 1 named `b`, B at 0 named `a`, in one call → `Applied`, and the ids keep their rows (this is the deferred-unique case on Pg) |
| 5 | `replace_graph_refuses_an_edge_into_another_graph` | an edge whose `to` is a phase of `GRAPH_HTUI_FEAT` → `Constraint(edge_endpoint_outside_graph("to_phase_id", p, graph))` exactly; nothing written |
| 6 | `replace_graph_refuses_edges_on_a_palette_node` | a `palette` node with a `next` edge → `Constraint(palette_phase_has_edges(name, edge))`; with no edges → `Applied` |
| 7 | `replace_graph_refuses_parts_on_a_non_divide_node` | parts on a `none` node → `Constraint(parts_off_divide(..))`. A `divide` node (`max_parts 3`, `planner_model`, a part with `owned_paths` and a persona) → `Applied`, and `phase_parts(node)` round-trips every field |
| 8 | `replace_graph_refusal_order` | One body breaking: a reserved name `judge`, a position clash, an unknown persona, an unknown agent, a dangling edge, `fan_out 2` with `Split::None`, a palette edge and parts on a `none` node. Mend one rule per call, expecting **exactly** the §T3.1/§T3.2 sentences in that order (the sentence functions are called in the assertions). Then a body valid but for a graph name taken in the project → `graph_name_is_taken`. Then a node id that is `ids::PHASE_HTUI_IMPLEMENT` → `already_exists("step_graph_phase", ..)`. Revision unchanged throughout |
| 9 | `delete_phase_cascades_edges_candidates_parts_and_bindings` | a `divide` phase with candidates, parts, a binding and in/out edges; `delete_phase` → `phases`, `graph_edges`, `phase_parts`, bindings all without it; revision + 1; again → `NotFound "step_graph_phase"`; the last phase of a graph may go (`phases` empty) |
| 10 | `delete_edge_and_delete_phase_agents_bump_revision` | each + 1 and removes exactly its rows; unknown ids → `NotFound` `"step_graph_edge"` / `"step_graph_phase"`; `delete_phase_agents` on a phase with none still + 1 |
| 11 | `every_graph_write_bumps_revision` | One graph, `revision` read through `graph_body` after each write, + 1 each time: `update_step_graph`, `create_phase`, `create_phase_agents` (non-empty), `update_phase`, `set_setting(Phase)`, `clear_setting(Phase)` (F1). `create_phase_agents(&[])` → + 0; a stale `update_phase` → + 0; a refused `create_phase` (reserved name) → + 0; `step_graph.updated_at` moved with each bump |
| 12 | `graph_scopes_are_unique_per_scope` | `create_step_graph` `Library "lib"` → a row with `scope: Library, revision 0`; a second `Library "lib"` → `Constraint(_)` (see the note below); `Workspace(WORKSPACE_PLATFORM) "lib"` and `Project(PROJECT_HTUI) "lib"` → ok; `Workspace(WorkspaceId::new())` → `Constraint(_)`; `graph_body(lib)` answers the library row; `step_graphs(PROJECT_HTUI)` lists only the project one; `replace_graph` renaming a second workspace graph to `"lib"` → exactly `graph_name_is_taken("lib", Workspace(WORKSPACE_PLATFORM))` |
| 13 | `an_override_is_project_scoped` | `is_override: true` with `Library` and with `Workspace(..)` → exactly `Constraint(override_outside_project(scope))`, no row; with `Project` → ok |
| 14 | `delete_workspace_takes_its_graphs` | a scratch `create_workspace` W, a graph of W with one phase + candidates; `delete_reach(Workspace(W))` reports `step_graphs 1, phases 1, phase_agents n`; `delete_workspace(W)` → `graph_body(g) == None`, project graphs untouched |
| 15 | `project_create_seeds_default_edges` | `create_project` → each of the five graphs' `graph_edges` equal `default_edges(&phases(g), ..)` compared without ids (from, to, kind, predicate, loop_limit); `bug` has `review → fix` with `loop_limit` = fix's `retry_limit`; 14 edges in all |
| 16 | `a_persona_held_by_divide_columns_or_parts_is_not_deleted` | A persona as `planner_persona_id` of a `divide` node in a **library** graph → `delete_persona` is refused with exactly `persona_is_bound(name, &[("library", g, phase)])`. As a part's persona in a project graph → refused, naming that phase. Clear both through `replace_graph` → deleted |
| 17 | `replace_graph_round_trips_every_node_field` | one node with every D7 field set (`divide`, `palette false`, `plan_scope Graph`, both personas, both agents, models, `max_parts 2`) and an `if` edge with a JSON predicate → `graph_body` equals the written body field for field (struct-order guard for the Pg lists) |
| 18 | `create_phase_keeps_split_in_step_with_fan_out` | `fan_out 2` + `Split::None` → exactly `split_fan_out_mismatch`; `fan_out 2` + `Compete` → ok; `Split::None` + `max_parts Some(2)` → `divide_columns_off_divide`; unknown `planner_persona_id` → `references_no_row("step_graph_phase.planner_persona_id", id, "persona")` |

Note on case 12: `create_step_graph`'s duplicate name stays `Constraint(_)` on Pg, as it is today
(`step_graph_and_phase_round_trip` asserts only the variant). A pre-check there would be scope
creep. The exact text is pinned on `replace_graph`, which pre-checks (§T3.4 step 3).

Counts: `htui-core/tests/mem_store.rs:36` `175` → `193` (extend the prose: "MOD-137's eighteen,
plan D3-D11"); `htui-store/tests/pg_conformance.rs:39` `EXPECTED_CASES = 193`.

Other T3 tests:

| Test | Backend | Asserts |
|---|---|---|
| `seed.rs::default_edges_link_adjacent_phases` | unit | 4 phases → 3 `next` in order |
| `seed.rs::default_edges_loop_review_to_the_greatest_earlier_implement` | unit | `implement@0, docs@5, review@9` → `review → implement`, `loop_limit` = its `retry_limit` |
| `seed.rs::default_edges_fall_back_to_the_previous_phase` | unit | `bug` from `KINDS` → `review → fix`; `draft@0, review@4` → `review → draft` (F4) |
| `seed.rs::default_edges_give_a_first_review_no_loop` | unit | lone `review@0` → `[]` |
| `seed.rs::default_edges_ignore_slice_order` | unit | a shuffled slice gives the same edges |
| `seed.rs::every_seeded_graph_has_its_default_edges` | unit | across `KINDS`: 10 `next` + 4 loops = 14 |
| `graph_rules.rs` tests | unit | §T3.2 |
| `migrations.rs::migration_0020_backfill_equals_default_edges` (D9) | Pg | Staged at v19 (as §T1): all five `KINDS` graphs + `gappy` + `gap-no-implement`, raw inserts, then `run`. For each graph, the SQL edges (from, to, kind, loop_limit) sorted equal `seed::default_edges(&phases, EdgeId::new)` sorted |
| `migrations.rs` demo-count test (`:2895-2960`) | Pg | `tables` and `expected` gain `("step_graph_edge", data.edges.len())`; the array is `[(&str, usize); 27]` |
| `pg_criteria.rs::replace_graph_waits_for_a_concurrent_phase_write` | Pg only | tx T: `bump_graph_of_phase`-shaped `UPDATE step_graph …` held. `replace_graph` from the pool blocks, and after T commits it answers `Stale` (the revision moved). No `40P01` (F7). 5 s timeout |

### T3 commits

1. `MOD-137 T3: graph_rules and the graph v2 refusal sentences` (additive, unit tests).
2. `MOD-137 T3: seed::default_edges; seeders and the demo write edges` (+ `DemoData.edges`, the D9 equality test, the demo count, `.sqlx`). The trait is not touched yet: Mem keeps `edges` in its state map, Pg writes through `insert_edge`.
3. `MOD-137 T3: replace_graph, the deleters and the graph reads on every WriteStore` (trait, Mem, Pg, `Writer`, `UsageSpy`, `SpyStore`, 18 conformance cases, counts, `pg_criteria` case, `.sqlx`).

---

## T4 — Catalogue verbs (`htui` crate)

### T4.1 `store_worker.rs`

Enum: after `SetPhaseBudget` (`:747-757`), update the block comment at `:648-650` ("the eleven
catalogue requests"):

```rust
    /// MOD-137 D12: the editor's one write. `replace_graph` under CAS on `step_graph.revision`;
    /// `Stale` answers [`StoreReply::CatalogueStale`], a refusal `Failed { request: "replace_graph" }`.
    ReplaceGraph {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The graph to replace; must equal `body.graph.id`.
        graph: StepGraphId,
        /// The `revision` the editor opened on.
        expected_revision: i64,
        /// Nodes, candidates, parts and edges as the editor holds them.
        body: Box<GraphBody>,
    },
    /// MOD-137 D11/D12: deletes a phase with its candidates, parts, edges and phase bindings.
    /// Answers [`StoreReply::Catalogue`].
    DeletePhase {
        /// The scope the reply re-reads.
        scope: Scope,
        /// The phase to delete.
        phase: PhaseId,
    },
```

The `Box` keeps `StoreRequest` from growing by a whole `StepGraph` (clippy `large_enum_variant`).

- `name()` (`:1251-1260`): `Self::ReplaceGraph { .. } => "replace_graph"`, `Self::DeletePhase { .. } => "delete_phase"`, after `set_phase_budget`. The comment says "The eleven of `catalogue::REQUEST_NAMES`".
- `try_serve` (`:2161-2172`): `| StoreRequest::ReplaceGraph { .. } | StoreRequest::DeletePhase { .. }` join the or-pattern. Fix the comments "nine" (`:2161`) and "twenty-three" (`:2174`) to the new totals.

### T4.2 `catalogue.rs`

- `GraphEntry` gains `/// `graph_edges(graph)`, §1.3 order (MOD-137 D12). pub edges: Vec<StepGraphEdge>,`. `snapshot` (`:113`) reads `store.graph_edges(graph.id).await?` per graph (one more N+1 read per event, D3's accepted trade). The graph's `revision` rides on `entry.graph.revision`.
- `serve`, after `SetPhaseBudget`:
  ```rust
  StoreRequest::ReplaceGraph { scope, graph, expected_revision, body } => {
      let outcome = writer.replace_graph(*graph, *expected_revision, (**body).clone()).await?;
      cas(&writer, scope, &outcome).await
  }
  StoreRequest::DeletePhase { scope, phase } => {
      writer.delete_phase(*phase).await?;
      reread(&writer, scope).await
  }
  ```
- `REQUEST_NAMES: [&str; 11]`: the nine, then `"replace_graph"`, `"delete_phase"`. Fix the doc and comments that say "nine" (`:142-145`, `:312-314`, `:342-344`).
- `ui/tabs/settings/kinds.rs:1574` (`REQUEST_NAMES.contains`) needs no change: refusals of both verbs surface through the existing `Failed` arm.

### T4 tests (first; `htui/tests/kinds.rs`, run with `--features testkit`)

| Test | Asserts |
|---|---|
| `catalogue_requests()` (`:82`) | gains `ReplaceGraph { scope: nil_scope(), graph: default, expected_revision: 0, body: Box::new(<a minimal body over StepGraph default fields>) }` and `DeletePhase { scope: nil_scope(), phase: default }`, in order |
| `catalogue_names_are_stable` (`:144`) | `REQUEST_NAMES.len() == 11`, names match |
| `offline_refuses_every_catalogue_request_by_name` (`:264`) | covers both new names unchanged (it iterates `catalogue_requests()`) |
| `the_demo_catalogue_carries_seeded_edges` (new) | each `GraphEntry.edges` of the demo scope: `analysis` 1 `next`; `bug` `review → fix` `changes_requested`; 14 in the project; `revision == 0` |
| `replace_graph_round_trips_through_the_catalogue` (new) | From `GRAPH_VULKAN_ANA`'s entry, build a body that renames `verdict` → `decide` and adds `asks_you decide → research` (`loop_limit 1`). The reply is `Catalogue`, the entry has `revision 1`, the new phase name and the edge |
| `a_spent_revision_answers_catalogue_stale` (new) | the same request twice → the second is `CatalogueStale` with the catalogue unchanged |
| `a_refused_replace_graph_answers_failed_by_name` (new) | a palette node with an edge → `Failed { request: "replace_graph", message }` with `message` containing `palette_phase_has_edges(..)` |
| `delete_phase_rereads_the_catalogue` (new) | `DeletePhase(verdict of GRAPH_VULKAN_ANA)` → `Catalogue`; that graph has 1 phase, 0 edges, `revision 1` |
| `kinds.rs:996` `GraphEntry` literal | gains `edges: Vec::new()` |

### T4 commits

1. `MOD-137 T4: ReplaceGraph and DeletePhase on catalogue::serve; GraphEntry carries edges`.

---

## T5 — Override copies edges; re-override (`htui-orch/src/graph.rs` only, F11)

### T5.1 Algorithm

```
override_graph(store, source, item):
  resolved = source.resolve_graph(item.id)?  .ok_or(NoGraph)
  reclone  = resolved.graph.is_override && item.step_graph_id == Some(resolved.graph.id)

  // the clone's source (D13): the kind default on a re-override, else what the item runs under
  (src_graph, src_rows): 
     if reclone:
        kind  = store.item_kinds(item.project_id).find(|k| k.id == item.kind_id) .ok_or(NoGraph)
        g     = kind.default_graph_id
        name  = store.step_graphs(item.project_id).find(id == g).name   (else NoGraph)
        rows  = for p in store.phases(g): (p, source.phase_agents(p.id), store.phase_parts(p.id))
     else:
        rows  = resolved.phases → (row.phase, row.agents, store.phase_parts(row.phase.id))
  src_edges = store.graph_edges(src_graph)

  D96 attachment check over the bindings of the source phase ids   (unchanged code, new `sources`)

  target = if reclone { resolved.graph (its id and revision) }
           else { store.create_step_graph(NewStepGraph { scope: Project(item.project_id),
                    name: "<key>-override", description: …, is_override: true }) }   // revision 0

  map: old phase id → PhaseId::new(), in source order
  body = GraphBody {
     graph: StepGraph { name: "<key>-override",
                        description: "Per-item override of `<src name>` for <key>", ..target },
     nodes: rows → NodeBody { phase: StepGraphPhase { id: map[old], graph_id: target.id, ..p },
                              agents: re-keyed phase_id, parts: re-keyed phase_id },
     edges: src_edges → StepGraphEdge { id: EdgeId::new(), graph_id: target.id,
                                        from: map[e.from], to: map[e.to], ..e },
  }
  match store.replace_graph(target.id, target.revision, body)? {
     Applied(b) => b,
     Stale(_)   => Err(Store(Constraint("override graph `<key>-override` moved while it was being re-cloned; override again")))
  }
  copy phase bindings onto map[old] (unchanged loop; outside the transaction, `graph.rs:609-610`)
  if !reclone: repoint the item (unchanged update_item); else leave item.step_graph_id alone
  Ok(applied.graph)
```

- The first override now writes its phases, candidates, parts and edges in **one** `replace_graph` (revision 0 → 1) instead of a `create_phase` + `create_phase_agents` loop. The orphan window shrinks to "graph row with no phases". `graph.rs:564`'s literal is replaced by the body build.
- On a re-override, `replace_graph` deletes the previous clone's phases (their candidates, parts, edges and bindings cascade) and writes fresh ids. No `update_item` runs, so `item.version` is unchanged.
- Docs: replace the "**Re-override is still owed**" paragraph (`graph.rs:458-461`) with "A second call on an item whose `step_graph_id` already names its override re-clones **from the kind's default graph** (the override is never a source) into the same graph through one `replace_graph`, with fresh phase and edge ids, and leaves `item.step_graph_id` alone (ANA-2 `:317-320`, MOD-137 D13)". Add "edges are copied, remapped" to the deep-clone list. The `# Errors` gains the `Stale` sentence.

### T5 tests (first; `graph.rs` `mod tests`, Mem, beside `:1929-2300`)

| Test | Asserts |
|---|---|
| `override_clone_copies_edges_remapped` | `feat_1`: clone's `graph_edges` (from name, to name, kind, loop_limit) equal the kind default's; every endpoint is a clone phase id; edge ids are fresh; `graph_id == clone.id`; clone `revision == 1` |
| `re_override_reclones_from_the_kind_default_into_the_same_graph` | override, then `replace_graph` the override (drop a phase), then `override_graph` again → the same graph id; `item.step_graph_id` and `item.version` unchanged; phase names = the default's; phase ids ≠ the first clone's; `revision == 3`; edges remapped |
| `re_override_picks_up_a_changed_kind_default` | override; then add `docs` + edges to the default via `replace_graph`; re-override → the override has `docs` and its edges |
| `re_override_moves_phase_attachments_to_the_fresh_clone` | binding on a default phase → after re-override it sits on the new clone phase; the old clone phases' bindings are gone |
| `re_override_refuses_a_stale_attachment_before_writing` | D96 parity: a bad glob on a default-phase binding → `Constraint`, override `revision` unchanged |
| existing `override_clone_carries_phase_attachments_and_marks_itself`, `override_clone_carries_phase_agents`, `override_clone_refuses_a_stale_repo_glob_before_writing`, `override_clone_refuses_a_negative_position_before_writing` | still green (the second asserts the agent rows re-keyed) |

### T5 commits

1. `MOD-137 T5: override_graph copies edges and re-overrides from the kind default`.

---

## T6 — Docs and full gate

- `docs/ANA-33.md` §7.1 and §10: one line each: the migration is `0020_step_graph_v2.sql` (0019 is MOD-125's) and the bound is `loop_limit` (`LIMIT` is reserved).
- `docs/ANA-2.md:317-320`: re-override is built (MOD-137 D13): it re-clones from the kind default through `replace_graph`.
- `HANDOFF.md` MOD-138 note: runs may use workspace and library graphs there (D4); `replace_graph` calls the shared validator once it exists; `graph_rules.rs` is its seed.
- The plan's Validation block line for `sqlx prepare` (F3), if the plan is kept as a record.
- Close-out per the workflow docs, then `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

### T6 commits

1. `MOD-137 T6: docs — ANA-33, ANA-2 and the MOD-138 hand-off note`.

---

## 2. Independence

- **T1 → T2 → T3** are serial. They share `mem.rs`, `pg/write.rs`, `conformance.rs` and `.sqlx/`.
- **T4 ∥ T5** are disjoint. T4 = `htui/src/{store_worker,catalogue}.rs`, `htui/tests/kinds.rs`. T5 = `htui-orch/src/graph.rs` (F11).
- `kinds.rs:996`'s `GraphEntry` literal is T4's. Its `StepGraph` part changes in T2.
- `.sqlx/` is touched only by T1–T3: T4 and T5 add no SQL.

## 3. Gate commands per task

Sandbox: `HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres` is already set.
Prepare DB, once after T1:

```bash
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE prep137 TEMPLATE scratch137"
psql -h localhost -p 5439 -U postgres -d prep137 -v ON_ERROR_STOP=1 -f crates/htui-store/migrations/0020_step_graph_v2.sql
# after any SQL change (T2, T3), from inside the crate (F3):
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/prep137 \
   cargo sqlx prepare -- --all-targets --all-features)
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/prep137 \
   cargo sqlx prepare --check -- --all-targets --all-features)   # "potentially unused" warning is expected
```

Never run a bare `cargo sqlx prepare` or `--workspace`. If `.sqlx` shrinks, run
`git checkout -- crates/htui-store/.sqlx && git clean -fq crates/htui-store/.sqlx` and re-run.

| Task | Gate |
|---|---|
| T1 | `psql … -d <copy> -v ON_ERROR_STOP=1 -f …/0020_step_graph_v2.sql`; `\d step_graph_edge`; `cargo test -p htui-store --all-features --test migrations --test connect -- --test-threads=1`; `cargo check --workspace --all-targets --all-features` |
| T2 | `cargo check --workspace --all-targets --all-features`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo clippy --workspace -- -D warnings`; `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features -- --test-threads=1`; `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \| tee /tmp/t2-orch.log; grep -c SIGABRT /tmp/t2-orch.log` (expect 0); `cargo test -p htui-worker --all-features -- --test-threads=1`; `cargo test -p htui --all-features -- --test-threads=1`; sqlx `--check` |
| T3 | `cargo test -p htui-core --all-features` (`mem_store` count 193); `cargo test -p htui-store --all-features --test pg_conformance --test migrations --test pg_criteria -- --test-threads=1`; `cargo test -p htui-agent --all-features`; clippy both; sqlx `--check` |
| T4 | `cargo test -p htui --features testkit --test kinds -- --test-threads=1` (testkit or it runs 0 tests and says ok, so check the count is non-zero); `cargo test -p htui --all-features --lib -- store_worker catalogue` |
| T5 | `cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \| tee /tmp/t5-orch.log; grep -c SIGABRT /tmp/t5-orch.log` |
| T6 | the plan's Validation block with F3's sqlx line; `cargo insta test --workspace --all-features --check` (expect no churn); `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |

Run every Pg gate with `--test-threads=1`: the workspace suite is scheduling-dependent (repo
memory). On a Postgres crash loop, check `df -h .` first.

## 4. What implementers must know (not in the plan)

1. **Lock order (F7).** Bump first, then write. Every Pg graph-content writer is now a transaction whose first statement is the bump.
2. **Wording is decided before the write (F6).** Pg never relies on a `23505`/`23503`/`23514` for a sentence a conformance case pins. The pre-reads in §T3.4 steps 3–5 and the shared pure checkers carry the text. The deferred uniques are a backstop.
3. **`ck_phase_split_fan_out` bites test helpers (F12).** Four helpers derive `split` from `fan_out`. `from_demo` fixtures are unchecked, so they may keep `fan_out 2` + `Split::None` while nothing reads `split` (MOD-138 must fix that when it starts to).
4. **Previous means previous in order (F4).** Use `LEAD` / sort, never `position - 1`.
5. **Pg `StepGraph` decodes through `StepGraphRow` (F13).** `StepGraphPhase` stays a direct `query_as!` with the ten columns between `persona_id` and `updated_at`, in every list.
6. **`judge_*` / `deadline_seconds` survive a replace only on kept nodes (F14).**
7. **A phase write now stales a graph-name token (D5).** Tests that hold an old `step_graph.updated_at` across a phase write will see `Stale`.
8. **Owner label in `persona_is_bound`** is the project slug, `workspace:<slug>` or `library` (F8).
9. **No new `DeleteReach` fields.** Workspace graphs count into the existing `step_graphs`, `phases` and `phase_agents`.
10. **`"zeta"` is a forbidden literal** anywhere in htui-agent-visible text (repo memory). Do not use it in test names or fixtures.
