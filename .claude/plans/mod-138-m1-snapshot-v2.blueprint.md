# Blueprint: MOD-138 M1 — Snapshot v2, the shared validator and the v1 lift (T1–T6)

> Produced by `code-architect` 2026-10-28 from the confirmed plan
> `.claude/plans/mod-138-m1-snapshot-v2.plan.md` (D1–D13), the PRD
> `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md` (Q1–Q4) and `docs/ANA-33.md` §7.1–§7.3,
> §7.7, §10. Tree read at `4cd56ee0` (branch `hr/MOD-138`). The plan stays the spec. §0 lists where
> the tree disagrees with it or where it is silent; rows marked **adopted** are implementation
> detail or a corrected plan claim. Four rows need the maintainer: they are collected, with the
> smallest amendment each, in **"Plan amendments needed"** at the end (A1–A4; A5 was added during
> T3, when the default-edge rule had to leave palette phases out). Every SQL statement
> below was probed on the sandbox Postgres 16.15 (`localhost:5439`, scratch DB `bp138` built from
> `0001`..`0022`, since dropped); §T3.2 and §T6.3 give the results.

## 0. Plan claims contradicted or extended by the tree

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | D1's "`split` skipped when `split = Split::for_fan_out(fan_out)`" cannot be written as a serde attribute: `skip_serializing_if` receives only `&field`, never the sibling `fan_out`. Under `ck_phase_split_fan_out` (`compete` ⇔ `fan_out > 1`) and `phase_row_refusal` (`graph_rules.rs:26-31`), a phase's split is fully determined by `fan_out` plus "is it divide", and "is it divide" is exactly `divide.is_some()`. | `kind.rs:280-306` (`Split`, `for_fan_out`); serde attribute semantics | **amendment A1**: `SnapshotPhase::split()` is a derived accessor, never serialised (§1.2). Byte-identity then holds trivially for `split`. |
| F2 | The orchestrator conformance suite runs on **one** binding, the `FakeOrchestrator` over `MemStore` (`Orchestrate::store(&self) -> &MemStore`, `conformance.rs:78-94`; the only binding is `tests/fake_conformance.rs`). The plan's "new cases run on the fake and PgStore orchestrators" is wrong: there is no PgStore orchestrator. | `conformance.rs:64-94`, `tests/fake_conformance.rs:1-38` | **adopted**: T5's cases are fake-only, as every resume case today. PgStore parity of the read side is T4's `pg_criteria` test. |
| F3 | **Seven** tests pin v1 resume, not six. The seventh is `conformance.rs` `mod tests` `a_lowered_cap_does_not_strand_a_run_on_resume` (`:9296-9318`), which asserts the `live graph not comparable` note. Under v2 pinning no live resolve runs, so that note is never written. | `conformance.rs:9296-9318` | **adopted**: it moves to a v1 fixture with the other six (§T5.4). |
| F4 | The demo fixture's runs carry `v: GraphSnapshot::V` with `topology: "sha256:demo"` (`fixtures.rs:1437-1456`). With `V = 2`, a v2 resume would recompute the digest and park every fixture run as tampered. | `fixtures.rs:1437-1456`, `:2509-2516` (asserts `v == V`) | **adopted**: the fixture writes `v: 1` (it stands for runs written before MOD-138) and `:2511` asserts `1`. Readers get it through the lift. |
| F5 | `EdgePredicate` is an opaque `serde_json::Value` (`kind.rs:350`). Finding 4's `preserve_order` trap applies to any `Value` inside the v2 digest. | `graph.rs:265-299` doc; `kind.rs:346-350` | **adopted**: `SnapshotEdge.predicate` serialises through a key-sorting wrapper (`SortedJson`, §1.2). M1 never freezes a predicate (D6 refuses non-default edges), but the digest is correct by construction for M2. |
| F6 | **T4 depends on T3, not only on T2.** The test helpers that repoint an item at an edited clone build it with `create_step_graph` plus one `create_phase` per row and **no edges**: `conformance.rs:1202` `repoint` (5 callers), `:1267` `insert_verify_phase`, `engine.rs:7851` `Harness::repoint`, `tests/gix_isolator.rs:336`, `tests/fixtures.rs:36-103`. Without D7 (T3), every such clone is refused by D6(c) at resolve (its stored edges, none, are not its default edges), so the fake suite cannot be green on T4 alone. With D7, a graph built row by row is default-edged by induction (the empty graph is default-edged). At file level T3 ∩ T4 is **`mem.rs` only**: T3 changes `pg/write.rs`, T4 changes `pg/read.rs`. | the helpers above; `mem.rs:3649-3712` | **amendment A2**: T3 → T4 serial on one tree (§2). |
| F7 | PgStore `resolve_graph` is several statements with no shared snapshot (`pg/read.rs:1989-2037`). Once resolve trusts edges, a Settings edit that commits between the phase read and the edge read is misreported: `NotYetRunnable` at `StartRun`, and a **false park** on a v1 resume. | `pg/read.rs:1977-2037` | **adopted**: the reads run in one `REPEATABLE READ, READ ONLY` transaction, the `graph_body` precedent (`pg/write.rs:4492-4501`, MOD-137 R1 A-L2). The two `&self` reads it needs move to `_on` free functions with byte-identical literals, so `.sqlx` does not change (§T4.3). D10's "moved while it was being resolved" refusal stays as a defensive check. |
| F8 | No reader lists workspace or library graphs. `step_graphs(project)` filters `project_id = $1` (`pg/read.rs:2992-3019`, `mem.rs` `step_graph_rows`), and `graph_body` needs an id. D12's "offer workspace and library graphs" therefore needs one new trait read. | `traits.rs:940-944`; `item_writes.rs:318-334`; `catalogue.rs:115` | **amendment A4**: `WriteStore::usable_graphs(project)` (§T6.1). That is one new `query_as!`, so `cargo sqlx prepare`, plus impls on `Writer` and on both test spies. |
| F9 | `delete_workspace` cascades its graphs (`fk_step_graph_workspace … ON DELETE CASCADE`), but `item_kind.default_graph_id` and `item.step_graph_id` reference `step_graph(id)` **without** a cascade. Probed: deleting a workspace whose graph a kind names fails with `23503 item_kind_default_graph_id_fkey`. MemStore (`mem.rs:5306-5330`) deletes silently and leaves the kind dangling. D12 makes this reachable for the first time. | probe §T6.3; `mem.rs:5306-5330`; `pg/write.rs:9696-9717` | **amendment A3**: both stores refuse the delete before writing, naming a holder (§T6.2). |
| F10 | Neither store checks an **item's** graph: `mint_item` and `update_item` bind `step_graph_id` as given (`pg/write.rs:1709-1866`; `mem.rs:2101`, `:2189`). The only item rule is `item_spec::check_graph` over `ItemFormContext.graphs` (`item_spec.rs:319-331`, `item_writes.rs:318-334`). `htui-mcp` has no graph surface (`git grep -i graph crates/htui-mcp/src`: no hit). | as cited | **adopted**: D12's "any item-side store check" is the empty set. The item rule widens through the context's list. |
| F11 | `FakeGraphSource` (`fake.rs:1086-1098`), `TestSource` (`graph.rs:1374-1400`), `HostGraphs` (`htui-worker/src/graphs.rs:21`), `Backend` and the `worker.rs` forwarders all pass the store's `ResolvedGraph` through whole. D10 changes only its two constructors: `mem.rs:7305-7318` and `pg/read.rs:2013-2036`. | as cited | **adopted**: `fake.rs` and `graphs.rs` drop out of T4's file set. |
| F12 | `retry_limit` is written only by `replace_graph` (`update_kept_phase`, `pg/write.rs:1439`). `PhasePatch` carries no `retry_limit` (`kind.rs:571-596`). | as cited | **adopted**: D7 in `update_phase` fires only when the patch sets `name` or `position`. It is skipped otherwise, with the same outcome either way (§1.5). |
| F13 | Two edge orders exist. `seed::default_edges` emits every `next`, then the loops (`seed.rs:298-321`); `graph_edges` sorts by (from position, to position, kind bytes) (`pg/read.rs:3474`, `mem.rs` `graph_edge_rows`). A digest over "the" edges needs one order. | as cited | **adopted**: one canonical snapshot edge order (`sort_snapshot_edges`, §1.2). Resolve and the lift both sort, and the v2 digest sorts its borrowed view too, so the order cannot make two equal graphs differ. |
| F14 | `FEATURE_TOPOLOGY` (`graph.rs:1317-1318`) pins the **v1** digest. Under `V = 2`, `snapshot.topology` is the v2 digest. | `graph.rs:1317-1330`, `:1505-1516`, `:1731`, `:2306`, `:3894`, `:3905` | **adopted**: `FEATURE_TOPOLOGY` stays and is asserted as `topology(&snapshot.phases)`. That is the cross-build byte-identity proof D1 needs, in crate and workspace runs. A new `FEATURE_TOPOLOGY_V2` pins `snapshot.topology` (§T4.6). |
| F15 | `tests/fixtures.rs` `feature_snapshot_matches` compares the whole resolved snapshot to the **v1** JSON fixture. `feature_with_verify_snapshot_matches` compares `topology`. | `tests/fixtures.rs:11-33`, `:125` | **adopted**: `feature.snapshot.json` becomes T1's v1 decode fixture. T4 adds `feature.v2.snapshot.json` for the whole-snapshot compare. The verify test compares `graph::topology(&phases)` against the v1 fixture's digest. |
| F16 | `PgStore::schema_version()` is `MIGRATOR`'s max version (`pg/mod.rs:724-726`); nothing pins it. | as cited | no action |
| F17 | After T3, MOD-137's scratch graphs carry default edges, because `scratch_graph` creates phases through `create_phase` (`store/conformance.rs:21665-21691`). Every MOD-137 case read here that cares sets `body.edges` explicitly or deletes the edges (`:22234-22268`, `:22342-22488`, `:22493-22578`, `:22584-22670`, `:22975-22999`). | as cited | **adopted**: T3 re-runs every MOD-137 case. An expectation that assumed "no edges" is amended, with a note. None was found. |
| F18 | D9's "every `asks_you` that is not a back edge" is circular if "back" is defined over forward edges that include forward `asks_you`. | D9 text | **adopted** (precision, not a change): an `asks_you` edge is a **back** edge iff its target reaches its source over `next` and `if` edges alone (§1.4, rule W2). |
| F19 | The demo `DemoData`-mutating resolve tests (`graph.rs:1524`, `:1961`, `:1985`, `:2005`, `:3559`, `:3613`, `:3741`, `:3829`; `runtime.rs:5867`; `run_worker.rs:1052`) edit `position`, `fan_out`, `isolation`, templates and personas. None edits `name` or `retry_limit`, so the seeded edges stay default and D6(c) passes. `positions_are_dense` multiplies positions by 10, which keeps both the order and the default edges. | as cited | no action. A future fixture edit of `name`/`retry_limit` in `DemoData` must also rewrite `data.edges`. |

---

## 1. Shared decisions for every task

### 1.1 Where things live

| Item | File |
|---|---|
| `NodeRole`, `SourceHow`, `RuntimeChanges`, `SnapshotSource`, `SnapshotEdge`, `SnapshotDivide`, `SnapshotSeat`, `SnapshotPart`, `SortedJson`, `sort_snapshot_edges`, `default_snapshot_edges`, `GraphSnapshot::lift_v1`, `SnapshotPhase::split` | `crates/htui-core/src/model/run.rs`, after `SnapshotSettings` (`:671-684`); re-exported in `model/mod.rs`'s `run` re-export |
| `seed::DefaultEdge`, `seed::default_edge_plan` | `crates/htui-core/src/seed.rs`, right above `default_edges` (`:277`), which is rewritten over it |
| `walk_refusal`, `graph_body_walk_refusal`, `WalkNode`, `WalkEdge`, `default_edges_follow`, `graph_usable_by` | `crates/htui-core/src/store/graph_rules.rs`, after `graph_body_topology_refusal` (`:183-230`); `pub use` in `store/mod.rs` beside the MOD-137 ones |
| Refusal sentences (§1.6) | `crates/htui-core/src/store/traits.rs`, a new `// ---- MOD-138 ----` block after MOD-137's (`:2967-3115`) |
| `topology_v2`, `expand_divide`, `snapshot_walk_refusal`, `NotRunnable`, new `ResolveError` variants | `crates/htui-orch/src/graph.rs` (it stays below 4 300 lines; no `snapshot.rs`) |
| `Resume::SnapshotTampered`, `park_on_resume` | `crates/htui-orch/src/engine.rs` |
| `fake::rewrite_as_v1` (test-support) | `crates/htui-orch/src/fake.rs` |
| `MemStore::set_run_snapshot` (test-support) | `crates/htui-core/src/store/mem.rs`, beside `set_box_setting` (`:1476-1492`) |

### 1.2 Rust types (exact)

```rust
// run.rs; imports gain: use crate::model::kind::{EdgeKind, EdgePredicate, GraphScope, PlanScope, Split};
//                       use crate::model::ids::{AgentId, PersonaId};   (AgentId is already imported)

str_enum!(
    /// MOD-138 D1 (ANA-33 §7.2): what a frozen node is. `subgraph` is MOD-143's, added there.
    #[derive(Default)]
    NodeRole {
        /// An ordinary step; every v1 node.
        #[default]
        Step => "step",
        /// `<name>:plan` of an expanded `divide` node (D11).
        Planner => "planner",
        /// `<name>` of an expanded `divide` node (D11).
        Integrate => "integrate",
    }
);

impl NodeRole {
    /// D1's skip predicate: a `step` node serialises without `role`.
    #[must_use]
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes a reference")]
    pub const fn is_step(&self) -> bool { matches!(self, Self::Step) }
}

str_enum!(
    /// MOD-138 D1: how the run's graph was chosen. `pick` is MOD-126's, added there.
    SourceHow {
        /// `item.step_graph_id` named the graph (an override included).
        Pin => "pin",
        /// The item named none: its kind's `default_graph_id`.
        KindDefault => "kind_default",
    }
);

str_enum!(
    /// MOD-138 D1, D13 (ANA-33 §7.8): run-time changes, frozen here and read by MOD-140.
    #[derive(Default)]
    RuntimeChanges {
        Off => "off",
        #[default]
        Ask => "ask",
        Automatic => "automatic",
    }
);

/// `GraphSnapshot.source` (D1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSource {
    pub how: SourceHow,
}

/// One edge of a v2 snapshot, by node key (D2): the phase `name` (`<name>:plan` for an expanded
/// planner). `predicate` stays MOD-137's opaque JSON, serialised with sorted keys (F5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "sorted_predicate")]
    pub predicate: Option<EdgePredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_limit: Option<i32>,
}

/// D1, D11: a `divide` node's frozen block. **M1 never stores one** (resolve refuses a divide
/// node after expanding it, D6(a)), so MOD-139 may reshape it without a compatibility concern.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDivide {
    #[serde(default)]
    pub planner: SnapshotSeat,
    #[serde(default)]
    pub integrate: SnapshotSeat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parts: Option<i32>,
    /// `phase_part` rows by `position` (D11).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_parts: Vec<SnapshotPart>,
}

/// The planner or integrate columns of a divide row, as stored (ids, not frozen names: see
/// `SnapshotDivide`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSeat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<PersonaId>,
}

/// One `phase_part` row minus `phase_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPart {
    pub position: i32,
    pub key: String,
    pub title: String,
    #[serde(default)] pub brief: String,
    #[serde(default)] pub owned_paths: Vec<String>,
    #[serde(default)] pub acceptance: Option<String>,
    #[serde(default)] pub agent_id: Option<AgentId>,
    #[serde(default)] pub model: Option<String>,
    #[serde(default)] pub persona_id: Option<PersonaId>,
}
```

`GraphSnapshot` gains two fields. The order below is the declaration order; neither digest reads
the envelope, D3:

```rust
pub struct GraphSnapshot {
    pub v: u32,
    pub graph: SnapshotGraph,
    /// MOD-138 D1; `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SnapshotSource>,
    pub topology: String,     // doc: v1 = topology(phases); v2 = topology_v2(phases, edges) (D3)
    pub mode: RunMode,
    pub phases: Vec<SnapshotPhase>,   // JSON key stays `phases` (D1)
    /// MOD-138 D2: by node key, `sort_snapshot_edges` order. Empty in a v1 blob; `snapshot_of`
    /// lifts it (D4).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<SnapshotEdge>,
    pub settings: SnapshotSettings,
    pub scope: …,      // unchanged
    pub personas: …,   // unchanged
}

impl GraphSnapshot {
    /// The `v` this crate writes (MOD-138 D1).
    pub const V: u32 = 2;
    /// The oldest `v` it reads; lifted by [`Self::lift_v1`] (D4).
    pub const V1: u32 = 1;

    /// D4: a v1 snapshot's edges are `seed::default_edges`' rule over its own phases. Keeps
    /// `v == 1`, so resume can tell a v1 run from a v2 one. A no-op on v2.
    pub fn lift_v1(&mut self) {
        if self.v == Self::V1 && self.edges.is_empty() {
            self.edges = default_snapshot_edges(&self.phases);
        }
    }
}
```

`SnapshotGraph` gains, after `is_override`:

```rust
    /// MOD-138 D1: the graph's owner (`{"owner":"project","id":…}` / `workspace` / `library`);
    /// `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<GraphScope>,
    /// MOD-138 D1: `step_graph.revision` at resolve; `None` on a v1 blob.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
```

`SnapshotSettings` gains, after `max_agents_per_run`. They are always serialised: they are frozen
bounds and are not hashed:

```rust
    /// MOD-138 D1, D13 (`app_setting.max_parallel_sessions`, default 4; read by M3).
    #[serde(default = "SnapshotSettings::default_max_parallel_sessions")]
    pub max_parallel_sessions: u32,
    /// (`max_plan_revisions`, default 3; MOD-139/MOD-140).
    #[serde(default = "SnapshotSettings::default_max_plan_revisions")]
    pub max_plan_revisions: u32,
    /// (`max_child_runs`, default 8; MOD-143).
    #[serde(default = "SnapshotSettings::default_max_child_runs")]
    pub max_child_runs: u32,
    /// (`runtime_changes`, default `ask`; MOD-140).
    #[serde(default)]
    pub runtime_changes: RuntimeChanges,
```

with `impl SnapshotSettings { pub const DEFAULT_MAX_PARALLEL_SESSIONS: u32 = 4; pub const
DEFAULT_MAX_PLAN_REVISIONS: u32 = 3; pub const DEFAULT_MAX_CHILD_RUNS: u32 = 8; const fn
default_…() -> u32 { Self::DEFAULT_… } }`. `bounds.agents` is not added (D1).

`SnapshotPhase` gains four fields, **appended after `persona`** (`run.rs:629`), in this order:

```rust
    /// MOD-138 D1: absent from the JSON, and so from both digests, for a `step` node.
    #[serde(default, skip_serializing_if = "NodeRole::is_step")]
    pub role: NodeRole,
    /// `step_graph_phase.palette`; absent when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub palette: bool,
    /// `step_graph_phase.plan_scope`; absent when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_scope: Option<PlanScope>,
    /// D11: `Some` exactly on a node whose stored `split` is `divide`; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub divide: Option<SnapshotDivide>,
}

impl SnapshotPhase {
    /// A1 (F1): `divide` when the node carries a divide block, else what `fan_out` implies
    /// (`ck_phase_split_fan_out`). Never serialised.
    #[must_use]
    pub const fn split(&self) -> Split {
        if self.divide.is_some() { Split::Divide } else { Split::for_fan_out(self.fan_out) }
    }
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes a reference")]
fn is_false(value: &bool) -> bool { !*value }
```

Helpers, all pure, in `run.rs`:

```rust
/// F5: `predicate` with object keys sorted by bytes at every depth, so the bytes never depend on
/// `serde_json`'s `preserve_order` (finding 4). Only called on `Some` (the field is skipped on `None`).
fn sorted_predicate<S: serde::Serializer>(p: &Option<EdgePredicate>, s: S) -> Result<S::Ok, S::Error>;
/// `Serialize` over a `&Value`: objects as maps in key-byte order, everything else as itself.
struct SortedJson<'a>(&'a Value);

/// F13: the one edge order: (position of `from`, position of `to`, `kind.as_str()` bytes), positions
/// by key in `phases` (an unknown key sorts last at `i32::MAX`), ties by (`from`, `to`) bytes.
pub fn sort_snapshot_edges(edges: &mut [SnapshotEdge], phases: &[SnapshotPhase]);

/// D4: `seed::default_edge_plan` over `(position, name, retry_limit)` of `phases`, by key, sorted.
#[must_use]
pub fn default_snapshot_edges(phases: &[SnapshotPhase]) -> Vec<SnapshotEdge>;
```

`seed.rs` (D4, "that function over a phase projection"):

```rust
/// One edge `default_edges` derives, by index into the projection it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultEdge { pub from: usize, pub to: usize, pub kind: EdgeKind, pub loop_limit: Option<i32> }

/// MOD-138 D4: MOD-137 D9's rule over any projection `(position, name, retry_limit)`; output
/// order as `default_edges`' doc says (every `next` in position order, then the loops in review
/// position order). Indices are into `phases` as given, not sorted.
#[must_use]
pub fn default_edge_plan(phases: &[(i32, &str, i32)]) -> Vec<DefaultEdge>;
```

`default_edges(phases, mint)` keeps its signature. Its body becomes
`default_edge_plan(&projection)` mapped to `StepGraphEdge`, with `mint()` called in output order,
so the demo's deterministic ids (`fixtures.rs:804`) do not move.

**Byte-identity proof (D1).** `serde_derive`'s `Serialize` for a struct calls
`serialize_struct`. It then calls `serialize_field` per field in declaration order, and
`skip_field` for a field whose `skip_serializing_if` holds. `serde_json` ignores the length hint
and writes nothing for `skip_field`. A linear phase (role `step`, palette `false`, plan_scope and
divide `None`) therefore emits exactly the v1 token stream: `position` … `judge`, then `persona`
when bound, then nothing. The four new fields come **after** `persona`, so
`a_bound_phase_carries_its_persona_name`'s `ends_with(",\"persona\":\"reviewer\"}")`
(`run.rs:1290-1298`) still holds. `topology(&phases)` (`graph.rs:287`) serialises the typed slice,
so the v1 digest of such a phase is unchanged. Three tests pin it: T1's fixture test (the v1 JSON
re-serialises to its own stored digest), T4's `FEATURE_TOPOLOGY` asserted as
`topology(&snapshot.phases)` in both the crate and the workspace run, and the T5 v1 resume of an
unedited graph, which walks.

### 1.3 The digests (D3)

```rust
// graph.rs, beside `topology` (:287), which is unchanged and stays the v1 digest.
/// MOD-138 D3: `"sha256:"` + hex over `serde_json::to_string` of the typed borrowed
/// `{ "phases": [...], "edges": [...] }`, edges in `sort_snapshot_edges` order whatever the input
/// order. Never through `Value` (finding 4); a predicate is key-sorted (F5). The envelope (`v`,
/// `graph`, `source`, `mode`, `settings`, `scope`, `personas`) is outside it, as for v1.
///
/// # Errors
/// [`ResolveError::Store`] carrying the serialiser's sentence (the `topology` precedent).
pub fn topology_v2(phases: &[SnapshotPhase], edges: &[SnapshotEdge]) -> std::result::Result<String, ResolveError>;

#[derive(Serialize)]
struct TopologyV2<'a> { phases: &'a [SnapshotPhase], edges: Vec<&'a SnapshotEdge> }
```

The order is sorted by references: `let mut edges: Vec<&SnapshotEdge> = …; edges.sort_by(…)`,
using the same key function as `sort_snapshot_edges`. Factor that key into a
`pub(crate) fn edge_sort_key` in `run.rs`, so the two orders cannot drift.

### 1.4 The walk validator (D9), exact rules

Core, over node indices, so that both adapters (GraphBody by `PhaseId`, the snapshot by key)
share one function:

```rust
/// One node the walk validator sees.
pub struct WalkNode<'a> { pub name: &'a str, pub palette: bool }
/// One edge, by index into the node slice.
pub struct WalkEdge { pub from: usize, pub to: usize, pub kind: EdgeKind }

/// MOD-138 D9: the first refusal, in this order, else `None`:
/// - W1/W2: let N = the `next` and `if` edges. An `asks_you` edge is a **back** edge iff its
///   target reaches its source over N (F18), else forward. The forward edges (N plus the forward
///   `asks_you`) must be acyclic: [`forward_edges_form_a_cycle`] naming the cycle (DFS from nodes
///   in slice order, edges in slice order; the path from the re-entered node round to it again).
/// - W3: per edge in slice order, a `changes_requested` or `fails` edge must be a back edge: its
///   target reaches its source over the forward edges, else [`loop_edge_runs_forward`].
/// "Every cycle is closed by a `changes_requested`, `fails` or `asks_you` edge" is a consequence:
/// with the forward edges acyclic, every cycle holds a non-forward edge, and the non-forward
/// edges are exactly those kinds. Palette nodes are excluded from the node set. MOD-137's rule
/// (e) already refuses any edge on a palette node, so this is a no-op kept for M2's reachability.
#[must_use]
pub fn walk_refusal(graph: StepGraphId, nodes: &[WalkNode<'_>], edges: &[WalkEdge]) -> Option<String>;

/// D9 over a body: nodes in body order, edges in body order; an edge naming a phase the body
/// lacks is skipped (rule (a) of `graph_body_topology_refusal` refuses it first).
#[must_use]
pub fn graph_body_walk_refusal(graph: StepGraphId, body: &GraphBody) -> Option<String>;
```

V9 re-checked against these rules: every MOD-137 `replace_graph` body cited there passes. The
`build → plan` `asks_you` (`store/conformance.rs:22379`) is a back edge over `plan → build`
`next`. The `plan → review` `asks_you` (`:22502`) is forward, and the forward set stays acyclic.
The `docs → review` `fails` (`graph.rs:2784`) loops back over `review → docs`. The seeded graphs
pass: their only loop is `review → implement|fix` over the `next` chain.

### 1.5 D7 — the default-edge rule

```rust
/// MOD-138 D7: after a per-row write (`create_phase`, `update_phase`, `delete_phase`), the edges
/// the graph must now hold. `Some(edges)` exactly when the graph was default-edged before the
/// write (`before_edges` equal `seed::default_edges(before_phases)` as a multiset of `(from, to,
/// kind, loop_limit, predicate is none)`) **and** `default_edges(after_phases)` differs from
/// `before_edges` by that shape. The new edges are minted by `mint` in `default_edges` order.
/// `None`: authored edges (left alone; a new phase is an unlinked root, which D9 accepts), or
/// nothing moved. A cascade (`delete_phase`) needs no special case: an edge on the deleted phase is
/// in `before_edges` but cannot be in `default_edges(after)`, so the shapes differ.
/// (A5, adopted in T3:) palette phases are left out of both sides (`graph_rules::linear`): a
/// palette node has no edges (MOD-137 rule e), so `default_edges` here always reads the
/// non-palette phases; creating or deleting a palette phase moves no edge, and the chain runs
/// past one.
#[must_use]
pub fn default_edges_follow(
    before_phases: &[StepGraphPhase],
    before_edges: &[StepGraphEdge],
    after_phases: &[StepGraphPhase],
    mint: impl FnMut() -> EdgeId,
) -> Option<Vec<StepGraphEdge>>;
```

`after_phases` is computed purely from `before_phases` plus the writer's own result. That is the
inserted row, the returned updated row, or `before` minus the deleted id. It is never re-read.

### 1.6 Refusal sentences (new `traits.rs` block, `#[must_use] pub fn … -> String`)

```rust
pub fn forward_edges_form_a_cycle(path: &[&str], graph: StepGraphId) -> String
    // "phases `plan` → `build` → `plan` form a cycle of forward edges in graph `{graph}`; only a
    //  `changes_requested`, `fails` or back `asks_you` edge may close a loop"
pub fn loop_edge_runs_forward(kind: EdgeKind, from: &str, to: &str, graph: StepGraphId) -> String
    // "the `{kind}` edge from `{from}` to `{to}` in graph `{graph}` runs forward: `{to}` does not
    //  reach `{from}` over forward edges, and a `{kind}` edge always loops back"
pub fn graph_not_usable(graph: StepGraphId, project: ProjectId) -> String       // replaces graph_not_in_project
    // "step_graph {graph} is not usable in project {project}: a kind or item takes a graph of its
    //  project, of a workspace holding its project, or of the library"
pub fn workspace_graph_is_held(graph: &str, holder: &str, project: &str) -> String   // A3
    // "step_graph `{graph}` of this workspace is held by {holder} of project `{project}`; point it
    //  at another graph before deleting the workspace"
```

`graph_not_in_project` (`traits.rs:2557-2561`) is deleted, with its re-export at
`store/mod.rs:31` and its imports at `mem.rs:74` and `pg/write.rs:67`. Its only uses are the three
kind sites (`git grep graph_not_in_project`). The orchestrator's two sentences are inline
`format!`s in `graph.rs`, the `override_graph` precedent (`graph.rs:651-653`):
`"graph `{name}` moved while it was being resolved; start the run again"` and
`"phase name `{name}:plan` is taken by the expansion of divide phase `{name}`"`.

---

## T1 — v2 snapshot types and the literal sweep (serial; first)

### T1.1 Changes

- `run.rs`: §1.2 in full: `V = 2`, `V1 = 1`, `lift_v1`, the helpers.
- `model/mod.rs`: re-export `NodeRole, RuntimeChanges, SnapshotDivide, SnapshotEdge, SnapshotPart,
  SnapshotSeat, SnapshotSource, SourceHow, default_snapshot_edges, sort_snapshot_edges` with the
  existing `run` items.
- `seed.rs`: `DefaultEdge`, `default_edge_plan`; `default_edges` over it.
- `command.rs` `snapshot_of` (`:1008-1029`). Moved here from T5 so every commit stays green (the
  demo fixture becomes v1, F4):
  ```rust
  if !(GraphSnapshot::V1..=GraphSnapshot::V).contains(&snapshot.v) {
      return Err(EngineError::SnapshotVersion { run: run.id, v: snapshot.v });
  }
  snapshot.lift_v1();
  Ok(snapshot)
  ```
  Its doc says it reads 1 and 2 and lifts 1 (D4).
- `fixtures.rs:1437-1456`: `v: 1` (literal, with a comment naming F4). `:2509-2516` asserts `1`
  ("the fixture's runs predate MOD-138").
- `mem.rs`, beside `set_box_setting`: the test seam
  ```rust
  /// MOD-138 T5: replaces `run.graph_snapshot` as no writer does (a run's snapshot is frozen), on
  /// this store and every clone; `updated_at` does not move. Whether the run exists.
  #[cfg(feature = "test-support")]
  pub fn set_run_snapshot(&self, run: RunId, snapshot: Value) -> bool
  ```
- The literal sweep. Every **full** literal gets the new fields at their defaults; spreads
  (`..x`) and serde-built values need nothing.

| Type | Full-literal sites |
|---|---|
| `SnapshotPhase` (+`role: NodeRole::Step, palette: false, plan_scope: None, divide: None`) | `fixtures.rs:1402`, `run.rs:1207`, `graph.rs:1021` (production; T4 fills real values), `engine.rs:8015`, `engine.rs:8943` |
| `GraphSnapshot` (+`source: None, edges: Vec::new()`) | `htui-agent/tests/relay.rs:97`, `fixtures.rs:1437`, `run.rs:1238`, `store/conformance.rs:5173`, `mem.rs:13436`, `htui-mcp/tests/tools_backlog.rs:38`, `graph.rs:394` (production), `htui-store/tests/cache.rs:660`, `pg_criteria.rs:1093`, `htui/tests/backlog.rs:952` |
| `SnapshotGraph` (+`scope: None, revision: None`) | the same ten sites (`:99`, `:1439`, `:1240`, `:5175`, `:13438`, `:40`, `graph.rs:396`, `:662`, `:1095`, `:954`) |
| `SnapshotSettings` (+ the four bounds at their `DEFAULT_*`, `runtime_changes: RuntimeChanges::Ask`) | `relay.rs:107`, `fixtures.rs:1447`, `run.rs:1248`, `store/conformance.rs:5183`, `mem.rs:13446`, `tools_backlog.rs:48`, `graph.rs:379` (production; T4 reads `app`), `cache.rs:670`, `pg_criteria.rs:1103`, `backlog.rs:962` |

The other files in the plan's list (`fanout.rs:290,735,754`, `overlap.rs:178`, `recover.rs`,
`gate.rs`, `status.rs`, `review_loop.rs`, `queue_overview.rs`, `run_worker.rs:1391`, the spreads at
`store/conformance.rs:5801,5997` and `backlog.rs:3356`) spread an existing value or decode JSON.
Leave them alone; the workspace check proves it.

### T1 tests (first)

| Test | Asserts |
|---|---|
| `run.rs` `a_step_phase_serialises_without_the_v2_keys` | `to_string(snapshot_phase(None))` contains none of `"role"`, `"palette"`, `"plan_scope"`, `"divide"`, `"split"`, and still ends `"judge":null}` |
| `run.rs` `a_planner_phase_carries_its_role_and_divide` | role `planner`, palette `true`, `plan_scope`, divide: all present; the round trip is the identity |
| `run.rs` `split_is_divide_then_fan_out` | `split()` is `Divide` with a block, `Compete` at `fan_out 2`, `None` at 1 |
| `run.rs` `a_v2_snapshot_round_trips_edges_source_scope_revision_and_bounds` | the encode/decode identity; `"edges"` and `"source"` are written; a library scope encodes `{"owner":"library"}` |
| `run.rs` `a_v1_blob_decodes_with_v2_defaults` | the v1 JSON shape (no `edges`, `source`, bounds or graph scope) decodes: edges empty, source `None`, bounds `4/3/8/ask` |
| `run.rs` `a_predicate_serialises_with_sorted_keys` | `{"z":1,"a":{"y":2,"b":3}}` built in that insertion order serialises as `{"a":{"b":3,"y":2},"z":1}` (sorts both with and without `preserve_order`) |
| `run.rs` `edges_sort_by_position_then_kind` | an unsorted list ends in (from pos, to pos, kind bytes) order; an unknown key sorts last |
| `run.rs` `default_snapshot_edges_is_seed_default_edges` | for each `seed::KINDS` graph, plus `gappy` (0/5/9), a leading `review` and two reviews: the `(from name, to name, kind, loop_limit)` set from `default_snapshot_edges(projection)` equals `seed::default_edges(rows)` mapped to names. **The "one rule, pinned equal" of D4.** |
| `run.rs` `lift_v1_fills_default_edges_and_keeps_v1` | v1 with no edges → the default edges, `v` still 1; v2 untouched (also with empty edges) |
| `seed.rs` `default_edge_plan_matches_default_edges_order` | same edges and order as MOD-137's `default_edges` on the seed and gap shapes (the existing `default_edges` tests stay green) |
| `htui-orch/tests/fixtures.rs` `the_v1_fixture_decodes_and_its_phases_hash_to_its_own_digest` | `feature.snapshot.json` decodes, `v == 1`, every phase role `step` / palette `false` / divide `None`; **`graph::topology(&decoded.phases) == decoded.topology`** (the byte-identity proof on a recorded v1 blob) |
| `command.rs` `snapshot_of_reads_v1_and_v2_and_refuses_v3` | v1 lifted, v2 as stored, `v: 3` is `SnapshotVersion { v: 3 }` |

### T1 commits

1. `MOD-138 T1: snapshot v2 types, default_edge_plan, the v1 lift` (`run.rs`, `model/mod.rs`,
   `seed.rs`, `command.rs`, its tests).
2. `MOD-138 T1: literal sweep, demo fixture as v1, set_run_snapshot seam` (every site above).
   The workspace check runs before this commit; `cargo test -p htui-core --all-features model::run`.

---

## T2 — The shared validator and the default-edge rule (after T1)

### T2.1 Pure rules (`graph_rules.rs`)

§1.4's `walk_refusal`, `graph_body_walk_refusal` and §1.5's `default_edges_follow`. The
`default_edges_follow` doc names its three callers.

### T2.2 Wiring into `replace_graph`, both stores

- MemStore `State::replace_graph` (`mem.rs:3966-3968`), straight after the topology refusal:
  ```rust
  if let Some(refusal) = graph_body_walk_refusal(graph, &body) {
      return Err(StoreError::Constraint(refusal));
  }
  ```
- PgStore (`pg/write.rs:4241-4243`): the same, still before step 7's first write.
- The trait doc (`traits.rs:1036-1041`) gains step **8**: "`Constraint` with
  [`graph_body_walk_refusal`]'s sentence (forward cycles, a `changes_requested` or `fails` edge
  that does not loop back)". Delete the sentence "acyclicity, back-edge kinds and reachability are
  MOD-138's shared validator, which this writer will call once it exists" (`:1022-1023`).
- `override_graph` (`graph.rs:694-698`) pre-checks the clone's body: append
  `.or_else(|| graph_body_walk_refusal(target_id, &body))`. A clone of a valid graph passes, so no
  test moves.

No SQL changes, so no `.sqlx` change.

### T2 tests (first)

`graph_rules.rs` unit tests (the MOD-137 `body()` fixture, `:576-600`, is
plan → implement → review with the review loop):

| Test | Asserts |
|---|---|
| `walk_the_valid_body_passes` | `body()` → `None`; every `seed::KINDS` graph through `seed::default_edges` → `None` |
| `walk_a_forward_next_or_if_cycle_is_refused` | add `review → plan` `next` → `forward_edges_form_a_cycle(["plan","implement","review","plan"], graph)`; the same with `if` (predicate set) |
| `walk_an_asks_you_back_edge_is_accepted` | `review → plan` `asks_you` → `None` |
| `walk_an_asks_you_with_no_next_path_back_is_forward` | `a → b` `asks_you`, `b → a` `asks_you`, no `next` → refused as a forward cycle (F18) |
| `walk_changes_requested_must_loop_back` | `plan → review` `changes_requested` → `loop_edge_runs_forward(ChangesRequested, "plan", "review", graph)` |
| `walk_fails_must_loop_back` | the same with `fails` |
| `walk_a_back_edge_may_close_over_a_forward_asks_you` | `a → b` `asks_you` (forward), `b → a` `changes_requested` → `None` |
| `the_walk_rules_fire_in_order` | a body with both a forward cycle and a forward `fails` → the cycle; mended → the `fails`; mended → `None` |
| `follow_rederives_a_default_graph` | after appending, moving, renaming `review`, and deleting `implement`: `Some(default_edges(after))` |
| `follow_leaves_an_authored_graph_alone` | before edges plus an `asks_you` → `None` for each write |
| `follow_is_none_when_nothing_moves` | renaming `implement` → `build` with `review` still looping to that same phase id → `None` (F12, §1.5) |

Store conformance (`CASES` 206 → **210**; each a `CASES` entry and a boxed `run_case` arm):

| Case | Asserts |
|---|---|
| `replace_graph_refuses_a_forward_cycle` | scratch `plan, build, review`; edges `plan→build`, `build→review`, `review→plan` `next` → `Constraint` = `forward_edges_form_a_cycle`; nothing written, revision unchanged |
| `replace_graph_refuses_a_forward_changes_requested` | `plan→review` `changes_requested` with no path back → `loop_edge_runs_forward` |
| `replace_graph_refuses_a_forward_fails` | the same with `fails` |
| `replace_graph_accepts_an_asks_you_back_edge` | default edges plus `review→plan` `asks_you` → `Applied`; edges stored as given |
| `replace_graph_refusal_order` (extended, `:22342`) | the initial body also gets `graph_edge(graph, build, plan, Next)` (W1) and `graph_edge(graph, plan, review, Fails)` (W3), appended to `body.edges`. Two `(sentence, mend)` rows go **after** `parts_off_divide`: `forward_edges_form_a_cycle(&["plan","build","plan"], graph)` → drop the cycle edge, then `loop_edge_runs_forward(Fails, "plan", "review", graph)` → drop it. The earlier rows still fire first: `into_palette` precedes the new edges in body order, so rule (e) names it. |

The pins: `htui-core/tests/mem_store.rs:36` 206 → 210, prose "…, MOD-138 M1's four for the walk
validator (plan D9)". `htui-store/tests/pg_conformance.rs` `EXPECTED_CASES` 206 → 210, with the
doc line and message to match.

### T2 commits

1. `MOD-138 T2: walk validator and default_edges_follow (pure, unit-tested)`.
2. `MOD-138 T2: replace_graph calls the walk validator in both stores; four cases, refusal order`.
   Validate with `cargo test -p htui-core --all-features graph_rules`, then `--test mem_store` and
   `--test pg_conformance`, each with `--test-threads=1`.

---

## T3 — Per-row writers keep default edges; migration `0023` (after T2; store lane)

### T3.1 The three writers (D7)

**MemStore.** One helper:

```rust
/// MOD-138 D7: `default_edges_follow` applied in place (no rollback in `write()`, so it runs after
/// the writer's last refusal; it cannot fail).
fn follow_default_edges(&mut self, graph: StepGraphId, before_phases: &[StepGraphPhase],
                        before_edges: &[StepGraphEdge], after_phases: &[StepGraphPhase]) {
    if let Some(edges) = default_edges_follow(before_phases, before_edges, after_phases, EdgeId::new) {
        self.edges.retain(|_, edge| edge.graph_id != graph);
        self.edges.extend(edges.into_iter().map(|edge| (edge.id, edge)));
    }
}
```

- `create_phase` (`mem.rs:3649-3712`): after the reference checks, read
  `before = self.phase_rows(graph)` and `before_edges = self.graph_edge_rows(graph)`. Then push the
  row and bump; `after = before + row`, sorted by position; then `follow_default_edges`.
- `update_phase` (`:3777-3840`): only when `patch.name.is_some() || patch.position.is_some()`.
  `before`/`before_edges` are read after the last check (`require_persona`) and before
  `phase_mut`. `after` is `before` with the updated row replacing it by id.
- `delete_phase` (`:4023-4038`): `before`/`before_edges` before the retains; `after` is `before`
  minus `id`.

**PgStore.** Graph row first, then phase rows (MOD-137 F7, §1.4 there). Every writer already
bumps as the **first** statement, which takes the graph row lock. D7's reads come **after** the
bump, inside the same transaction. In READ COMMITTED each later statement takes a fresh snapshot,
which sees every writer that committed before the lock was granted. No other graph-content writer
can interleave, because every one bumps the same row first. No new lock is taken, so there is no
new deadlock order. Helpers in `pg/write.rs`:

```rust
/// MOD-138 D7, inside the writer's transaction after its bump.
async fn graph_shape_on(conn: &mut PgConnection, graph: StepGraphId)
    -> Result<(Vec<StepGraphPhase>, Vec<StepGraphEdge>)>
    // phase_rows_on(&mut *conn, graph) then graph_edges_on(&mut *conn, graph): existing literals
    // (`pg/read.rs:3377`, `:3455`), so no new `.sqlx` entry

/// Replaces `graph`'s edges whole: the `DELETE FROM step_graph_edge WHERE graph_id = $1` literal
/// `replace_graph` step 9 already prepares (`pg/write.rs` ~4283; the offline hash is the literal),
/// then `Box::pin(insert_edge(...))` per edge (`:1407`). `replace_graph` switches to it too.
async fn replace_edges(conn: &mut PgConnection, graph: StepGraphId, edges: &[StepGraphEdge]) -> Result<()>
```

- `create_phase` (`:3796-3833`): after `phase_reference_refusal`, `let (before, before_edges) =
  graph_shape_on(&mut tx, phase.graph_id)`. Then `insert_phase`, and with
  `after = before + row`, `default_edges_follow` → `replace_edges` if `Some`. Then commit.
- `update_phase` (`:3896-4058`): keep the graph id. `bump_graph_of_phase` already returns it, but
  `current.graph_id` (after the `FOR UPDATE` read) is the authoritative one, per R1 A-M1. Only
  when the patch sets `name` or `position`, read `graph_shape_on` after the `Stale` and clash
  checks, before the `UPDATE`. Then, on `Some(row)` (Applied), `after = before` with `row`, follow,
  and commit. A `None` from the `UPDATE` rolls back as today.
- `delete_phase` (`:4369-4389`): `let graph = bump_graph_of_phase(...)`; on `Some(graph)`, read
  `graph_shape_on` before the `DELETE`. When `rows_affected == 1`, `after = before − id`, follow,
  and commit. The existing zero-row rollback is unchanged.

**New `sqlx::query!`: none.** Run `cargo sqlx prepare --check` from `crates/htui-store` (MOD-137 F3)
to prove it. The trait docs of `create_phase` / `update_phase` / `delete_phase`
(`traits.rs:946-963`, `:977-990`, `:1049-1055`) each gain one sentence: "MOD-138 D7: on a
default-edged graph the edges are re-derived in the same write; authored edges are left alone."
`create_phase`'s doc also says a `palette` phase is left unlinked, so creating one moves no edge (A5).

### T3.2 Migration `crates/htui-store/migrations/0023_default_edges_repair.sql` (probed verbatim)

The file as shipped (A5: the `NOT p.palette`, `NOT c.palette` and `NOT rv.palette` filters were
added in T3; the probe below predates them and its eight graphs carry no palette phase, so its
rows are unchanged by them):

```sql
-- 0023_default_edges_repair.sql - MOD-138 milestone 1 (plan D7, D8). Forward-only (R-STO-5).
--
-- Settings > Kinds adds, moves, renames and deletes phases one row at a time (create_phase,
-- update_phase, delete_phase), and before MOD-138 none of those writers touched step_graph_edge.
-- A graph edited that way since 0020_step_graph_v2.sql has edges out of step with its positions
-- and names; the position walk hid it. From MOD-138 on, resolve reads the edges, so this
-- migration repairs them once, and the per-row writers keep a default-edged graph default-edged
-- from here on (plan D7).
--
-- The repair (plan D8): for every graph with no 'if', 'asks_you' or 'fails' edge (no editor can
-- author one before MOD-141, so such a graph's edges are all derived), its 'next' and
-- 'changes_requested' edges are deleted and 0020's backfill is applied again: a 'next' edge
-- between each pair of phases adjacent in position order, and for each phase named 'review' a
-- 'changes_requested' edge to the greatest earlier phase named 'implement', else to the phase just
-- before it in position order, with loop_limit = the target's retry_limit. A palette phase is
-- left out of both (a palette node has no edges, MOD-137's store rule): the chain runs past it and
-- no loop starts or ends on it. seed::default_edges over the non-palette phases is the same rule
-- in Rust (htui_core::store::default_edges_follow), pinned equal by a migration test. A graph that
-- carries an authored edge is left exactly as it is. revision is not bumped (0020 did not
-- either): nothing holds a replace_graph token across a migration.
--
-- Idempotent: a second application deletes and re-inserts the same (from, to, kind, loop_limit)
-- rows. No table or column changes, step_graph_edge is not mirrored, so there is no cache
-- migration; schema_version becomes 23, so each box rebuilds its mirror once on first start.

CREATE TEMPORARY TABLE mod138_repaired ON COMMIT DROP AS
SELECT g.id AS graph_id
  FROM step_graph g
 WHERE NOT EXISTS (SELECT 1 FROM step_graph_edge e
                    WHERE e.graph_id = g.id AND e.kind IN ('if', 'asks_you', 'fails'));

DELETE FROM step_graph_edge e
 USING mod138_repaired r
 WHERE e.graph_id = r.graph_id;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind)
SELECT ordered.graph_id, ordered.id, ordered.next_id, 'next'
  FROM (SELECT p.graph_id, p.id,
               LEAD(p.id) OVER (PARTITION BY p.graph_id ORDER BY p.position) AS next_id
          FROM step_graph_phase p
          JOIN mod138_repaired r ON r.graph_id = p.graph_id
         WHERE NOT p.palette) ordered
 WHERE ordered.next_id IS NOT NULL;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind, loop_limit)
SELECT rv.graph_id, rv.id, t.id, 'changes_requested', t.retry_limit
  FROM step_graph_phase rv
  JOIN mod138_repaired r ON r.graph_id = rv.graph_id
 CROSS JOIN LATERAL (SELECT c.id, c.retry_limit
                       FROM step_graph_phase c
                      WHERE c.graph_id = rv.graph_id
                        AND c.position < rv.position
                        AND NOT c.palette
                      ORDER BY (c.name = 'implement') DESC, c.position DESC
                      LIMIT 1) t
 WHERE rv.name = 'review'
   AND NOT rv.palette;
```

The temporary table relies on sqlx running each migration in one transaction, which is its
default. The file carries no `-- no-transaction`.

**Probe (2026-10-28, `bp138` = `0001`..`0022` applied with `psql -1 -f`).** Graphs seeded raw:

- `feature`: already default;
- `appended`: `docs@2` added with no edge;
- `renamed`: a stale loop `review → plan` where the rule now says `review → implement`;
- `gappy`: positions 0/5/9, no edges;
- `authored`: an odd `x → z` `next`, plus `if` and `fails`;
- `asks`: `asks_you`;
- `solo`: one `review`;
- `empty`: no phases.

After applying the file (`-1`):

```
  graph   | fpos | from_phase | tpos | to_phase  |       kind        | loop_limit |      predicate
----------+------+------------+------+-----------+-------------------+------------+----------------------
 appended |    0 | implement  |    1 | review    | next              |            |
 appended |    1 | review     |    0 | implement | changes_requested |          2 |
 appended |    1 | review     |    2 | docs      | next              |            |
 asks     |    0 | p          |    1 | q         | next              |            |
 asks     |    1 | q          |    0 | p         | asks_you          |            |
 authored |    0 | x          |    2 | z         | next              |            |   <- authored: untouched
 authored |    1 | y          |    2 | z         | if                |            | {"verify": "passed"}
 authored |    2 | z          |    0 | x         | fails             |          1 |
 feature  |    0 | prd        |    1 | plan      | next              |            |
 feature  |    1 | plan       |    2 | implement | next              |            |
 feature  |    2 | implement  |    3 | review    | next              |            |
 feature  |    3 | review     |    2 | implement | changes_requested |          3 |
 gappy    |    0 | implement  |    5 | docs      | next              |            |
 gappy    |    5 | docs       |    9 | review    | next              |            |
 gappy    |    9 | review     |    0 | implement | changes_requested |          4 |
 renamed  |    0 | plan       |    1 | implement | next              |            |
 renamed  |    1 | implement  |    2 | review    | next              |            |
 renamed  |    2 | review     |    1 | implement | changes_requested |          2 |   <- repaired
(18 rows)
```

A second application gave the same 18 rows (`diff` empty). Every `revision` stayed 0. `solo` and
`empty` have no edges.

T3 added two palette probe graphs to the migration test (A5): `palette` (`plan`, palette `spare`,
`review`) gets `plan → review` `next` and `review → plan` `changes_requested` (the chain runs past
`spare`, the fallback loop skips it); `palette-review` (`implement`, palette `review`) gets no
edge. The pinned table is the 18 rows above plus those 2: 20 rows.

### T3.3 Pins that move (`0023`; the "new migration moves count pins" memory)

- `tests/connect.rs:140`, `:242`: `Pending(22)` → `Pending(23)`. The prose at `:141-142`,
  `:157-158` and `:243-244` becomes "twenty-three embedded migrations, through MOD-138's
  0023_default_edges_repair.sql".
- `tests/migrations.rs`:
  - versions `:115`: append `23`; the prose at `:117-127` adds "and MOD-138's
    0023_default_edges_repair.sql".
  - `:3561`, `:3662`: `Pending(23)`. `:3671`, `:3694`: `MigrationsPending(23)`. The prose at
    `:3562`, `:3663`, `:3672`, `:3695` and `:3847-3848` says twenty-three through `0023`.
  - `TABLES` (51) and the comment lists are unchanged: no table, no `COMMENT ON`.
- The `migration_0020_*` and `migration_0021_*` tests apply through the latest migration with
  `MIGRATOR.run`, so `0023` runs in them too. Their graphs are default-shaped, so they keep the same
  `(from, to, kind, loop_limit)`; only the edge ids change, and no 0020/0021 test compares edge ids.

### T3 tests (first)

| Test | Asserts |
|---|---|
| `migrations.rs` `migration_0023_repairs_drifted_graphs_and_leaves_authored_ones` | `run_to(22)`; raw-seed the eight probe graphs, plus `palette` and `palette-review` (A5) (with `insert_graph` / `insert_phase`, `:2003-2030`, and a raw edge helper like `try_edge`); `run`; `edge_names(pool)` equals the table above; `authored` and `asks` keep their **edge ids**; `revision` all 0 |
| `migrations.rs` `migration_0023_equals_default_edges` | per repaired graph, the stored `(from, to, kind, loop_limit)` set equals `seed::default_edges(rows)` over the non-palette rows (A5) (the `migration_0020_backfill_equals_default_edges` shape, `:2271-2370`) |
| conformance `create_phase_keeps_a_default_graph_default_edged` | scratch `plan, implement, review` (now default by induction) → `edge_shape` = default; append `docs@3` → still default; revision +1 per write |
| conformance `update_phase_keeps_a_default_graph_default_edged` | move `review` to position 9 → default; rename `implement` → `build` (the loop now targets the previous phase, the same id: shape unchanged); rename `review` → `check` → the loop edge goes; one bump each |
| conformance `delete_phase_keeps_a_default_graph_default_edged` | delete the middle phase → `next` re-chains around it, the loop re-targets |
| conformance `per_row_writers_leave_authored_edges_alone` | `replace_graph` adds `review → plan` `asks_you`; then create, move, rename and delete → the edges are exactly the authored set minus the cascade |
| conformance `create_phase_leaves_a_palette_phase_unlinked` (A5) | creating a `palette` phase on a default-edged graph moves no edge; the graph's read-back body saves through `replace_graph` unchanged |

`CASES` 210 → **215** (both pins, prose "…, and five for the per-row writers' default edges (plan
D7)"; the fifth is A5's palette case). Re-run every MOD-137 case (F17).

### T3 commits

1. `MOD-138 T3: per-row writers keep a default-edged graph default-edged (both stores)`.
2. `MOD-138 T3: 0023_default_edges_repair.sql and its tests; migration pins`.
   Validate: both conformance suites `--test-threads=1`; `cargo test -p htui-store --all-features
   --test migrations --test connect`; `(cd crates/htui-store && cargo sqlx prepare --check)`.

---

## T4 — Resolve builds v2 (after T3; A2)

### T4.1 Model (`kind.rs:671-690`, D10)

```rust
pub struct ResolvedGraph {
    pub graph: StepGraph,
    pub phases: Vec<ResolvedPhase>,
    /// MOD-138 D10: `graph_edges` order (blueprint MOD-137 §1.3).
    #[serde(default)]
    pub edges: Vec<StepGraphEdge>,
}
pub struct ResolvedPhase {
    pub phase: StepGraphPhase,
    pub agents: Vec<PhaseAgent>,
    pub persona: Option<Persona>,
    /// MOD-138 D10: `phase_part` by `position`; empty off a divide node.
    #[serde(default)]
    pub parts: Vec<PhasePart>,
}
```

Update the doc "in one round trip" (`kind.rs:668`): it is one transaction on Postgres (F7) and one
lock in MemStore.

### T4.2 MemStore (`mem.rs:7291-7319`)

`edges: self.graph_edge_rows(graph_id)` and `parts: self.phase_parts_of(phase.id)`.

### T4.3 PgStore (`pg/read.rs:1989-2037`, F7)

```rust
pub async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>> {
    let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx).await.map_err(map_sqlx)?;
    let resolved = Box::pin(resolve_graph_on(&mut tx, item)).await?;   // boxed: the engine awaits it
    tx.commit().await.map_err(map_sqlx)?;
    Ok(resolved)
}
```

`resolve_graph_on(conn, item)` holds today's body, with each read on `&mut *conn`:
`step_graph_row_on` (`:3346`), `phase_rows_on` (`:3377`), the new `phase_agents_on` (the body of
`phase_agents`, `:1916`, moved verbatim to a free `PgExecutor` fn, with `phase_agents` calling it),
`phase_parts_on` per phase (`:3488`), `graph_edges_on` (`:3455`), and the new
`persona_rows_by_id_on` (the body of `:3096`, moved the same way). Every literal is byte-identical,
so **no `.sqlx` change**. `sqlx::raw_sql` matches `graph_body`'s use.

### T4.4 `graph::resolve` (`graph.rs:315-412`), new order

1. `Box::pin(source.resolve_graph(item.id))` → `NoGraph`. Boxed (§4).
2. Project, then `ProjectCaps` (unchanged).
3. The body: `GraphBody { graph, nodes: phases → NodeBody { phase, agents, parts }, edges }`.
   - **moved** (D10): an edge whose `from` or `to` is not a phase of the read →
     `ResolveError::Store(StoreError::Constraint("graph `{name}` moved while it was being resolved; start the run again"))`.
   - **InvalidGraph** (D9):
     `graph_body_topology_refusal(id, &body).or_else(|| graph_body_walk_refusal(id, &body))` →
     `ResolveError::InvalidGraph { graph: name, reason }`.
4. The per-phase loop (unchanged refusals and order). `snapshot_phase` fills `role: Step,
   palette: phase.palette, plan_scope: phase.plan_scope, divide: (phase.split == Split::Divide)
   .then(|| divide_block(phase, &row.parts))`, with `divide_block` mapping the seven columns and
   the parts.
5. Edges by key: id → name over the rows; `SnapshotEdge { from, to, kind, predicate:
   edge_predicate(edge).cloned(), loop_limit }`; then `sort_snapshot_edges(&mut edges, &phases)`.
6. `let (phases, edges) = expand_divide(phases, edges)?` (D11), then
   `snapshot_walk_refusal(graph.id, &phases, &edges)` → `InvalidGraph` (defensive; it covers PRD
   Q1's "the validator checks the expansion").
7. **D6**, first refusal wins:
   - (a) the first node, in position order, whose role is not `step` →
     `NotYetRunnable { node: <the divide phase's name>, reason: NotRunnable::Divide }`;
   - (b) the first `palette` node → `NotRunnable::Palette`;
   - (c) `resolved.edges` versus `seed::default_edges` of the non-palette rows (A5: the rule D7
     and `0023` keep; moot in M1 while (b) refuses any palette node first) as multisets of
     `(from id, to id, kind, loop_limit, predicate none)`. On a difference, `node` is the `from`
     name of the first stored edge (stored order) absent from the defaults, else the `from` name of
     the first default edge absent from the store → `NotRunnable::NonDefaultEdges`.
8. Settings plus D13's bounds:
   `app_u32(app, "max_parallel_sessions").unwrap_or(SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS)`,
   the same for `max_plan_revisions` and `max_child_runs`, and
   `app.get("runtime_changes").and_then(Value::as_str).and_then(|s| s.parse().ok()).unwrap_or_default()`.
   Then `check_fan_out_caps` and `overlap::resolve` (unchanged).
9. The literal:
   - `v: GraphSnapshot::V`;
   - `graph: SnapshotGraph { id, name, is_override, scope: Some(graph.scope), revision: Some(graph.revision) }`;
   - `source: Some(SnapshotSource { how: if item.step_graph_id.is_some() { SourceHow::Pin } else { SourceHow::KindDefault } })`;
   - `topology: topology_v2(&phases, &edges)?`;
   - `edges`.

```rust
// graph.rs
/// MOD-138 D6: why a graph this version cannot walk is refused before its run exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NotRunnable {
    #[error("a `divide` node runs once divided fan-out lands (MOD-139)")]
    Divide,
    #[error("a palette node is placed by a planner, which runs from MOD-140 on")]
    Palette,
    #[error("its edges are not the default ones its phases imply (`next` in position order and the \
             review loop); other edges run once the readiness walker lands (MOD-138 M2)")]
    NonDefaultEdges,
}

// ResolveError gains (after `UnknownTouchedRepo`, before `Store`):
    /// MOD-138 D9: the shared validator refused the live graph (or its frozen expansion).
    #[error("graph `{graph}` is not a valid step graph: {reason}")]
    InvalidGraph { graph: String, reason: String },
    /// MOD-138 D6: a graph M1 freezes but cannot walk; lifted by MOD-139, MOD-140 and M2.
    #[error("phase `{node}` cannot run yet: {reason}")]
    NotYetRunnable { node: String, reason: NotRunnable },

/// MOD-138 D11: each `divide` node N at position p becomes `N:plan` (role `planner`, position p,
/// N's `plan_scope`) and `N` (role `integrate`, p + 1, `plan_scope: None`), both carrying N's
/// `divide` block; every later position shifts by one; edges into N move to `N:plan`, edges out of
/// N stay on N, and a synthetic `N:plan → N` `next` joins them; edges re-sorted.
///
/// # Errors
/// `InvalidGraph` when `N:plan` is already a node's key.
pub fn expand_divide(phases: Vec<SnapshotPhase>, edges: Vec<SnapshotEdge>)
    -> std::result::Result<(Vec<SnapshotPhase>, Vec<SnapshotEdge>), ResolveError>;

/// D9 over a snapshot (by key): `walk_refusal` with `palette` from the node.
fn snapshot_walk_refusal(graph: StepGraphId, phases: &[SnapshotPhase], edges: &[SnapshotEdge])
    -> Option<String>;
```

Resolve's call sites: `StartRun` (`engine.rs:734-750`) needs no change, because the new variants
are pre-run refusals raised as `EngineError::Resolve`. The resume path is T5's. `runtime.rs:138`
`backs_off` already treats every non-`Store` resolve error as one to back off on.

### T4 tests (first)

| Test (`graph.rs` `mod tests` unless named) | Asserts |
|---|---|
| `feature_snapshot_topology_is_pinned` (changed, `:1505-1516`) | `topology(&snapshot.phases) == FEATURE_TOPOLOGY` (v1 digest unchanged: F14); `snapshot.topology == FEATURE_TOPOLOGY_V2` (compute once, paste; extend `TOPOLOGY_MOVED` to name both); `v == 2` |
| `:1731`, `:2306`, `:3894`, `:3905` | compare `snapshot.topology` with `FEATURE_TOPOLOGY_V2`. The override clone has the same keys and edges, so the same v2 digest. |
| `every_seeded_kind_resolves_to_its_default_edges` | for each kind's item: `snapshot.edges == default_snapshot_edges(&snapshot.phases)`; `source`, `graph.scope == Project`, `graph.revision == 0` |
| `a_snapshot_freezes_the_bounds_and_the_source` | default bounds `4/3/8/ask`; the `app_setting` values `7/2/3/"off"` are frozen; a pinned item → `Pin`, else `KindDefault` |
| `a_divide_node_is_expanded_then_refused` | via `store_with`: `implement` set `split: Divide`, `max_parts: Some(2)`, one `phase_part` → `NotYetRunnable { node: "implement", reason: Divide }` |
| `expand_divide_splits_reroutes_and_shifts` (pure) | `prd, plan, implement(divide), review` → keys `prd, plan, implement:plan, implement, review`, positions 0..4; `plan → implement:plan`, `implement:plan → implement` `next`, `implement → review`, `review → implement:plan` (the loop is re-pointed); roles `planner`/`integrate` |
| `expand_divide_refuses_a_taken_plan_key` | a node already named `x:plan` → `InvalidGraph` |
| `a_palette_node_is_refused` | `NotYetRunnable { node, reason: Palette }` (the palette node has no edges, and its default `next` edges are removed in `DemoData`) |
| `non_default_edges_are_refused` | `replace_graph` adds `review → plan` `asks_you` → `NotYetRunnable { node: "review", reason: NonDefaultEdges }` |
| `an_invalid_live_graph_is_refused_with_the_validators_sentence` | a forward cycle planted through `DemoData.edges` (it bypasses `replace_graph`) → `InvalidGraph { reason == forward_edges_form_a_cycle(...) }`: the same sentence the store gives at save |
| `an_edge_outside_the_read_is_refused_as_moved` | a `TestSource` that drops a phase from `resolve_graph`'s answer → the moved sentence |
| `topology_v2_ignores_edge_order` (pure) | shuffled edges, same digest; any field change, different digest; a predicate with keys built in two orders, same digest |
| `mem.rs` `resolve_graph_carries_edges_and_parts` | beside `resolve_graph_fills_the_bound_persona` (`:15634`): `edges == graph_edge_rows`, `parts == phase_parts_of` |
| `pg_criteria.rs` `resolve_graph_carries_edges_and_parts_as_mem_store_does` | beside `:6914`: the same on PgStore |
| `tests/fixtures.rs` `feature_snapshot_matches` (changed) | compares with the new `tests/fixtures/feature.v2.snapshot.json` (recorded once from a resolve; `"v": 2`) |
| `tests/fixtures.rs` `feature_with_verify_snapshot_matches` (changed) | `graph::topology(&resolved.snapshot.phases) == expected.topology` (the v1 fixture) |

### T4 commits

1. `MOD-138 T4: ResolvedGraph carries edges and parts (MemStore; PgStore in one read-only snapshot)`.
2. `MOD-138 T4: resolve writes snapshot v2 (expansion, bounds, source, v2 digest, D6/D9 refusals)`.
   Validate: `cargo test -p htui-orch --all-features graph::`; `--test fixtures`;
   `--test fake_conformance`; `pg_criteria` `--test-threads=1`.

---

## T5 — Pinned resume (after T4)

### T5.1 `Resume` (`engine.rs:547-564`)

```rust
pub enum Resume {
    /// The snapshot was walked: a v2 run whose snapshot recomputes to its own `topology` (D5), or
    /// a v1 run whose live graph still hashes to the lifted snapshot's v2 digest (or does not
    /// resolve under a start-time rule: the `live graph not comparable` note says which).
    Walked(Rest),
    /// A v1 run (D5): `snapshot` is the lifted snapshot's v2 digest; `live` is the live graph's v2
    /// digest, or `not runnable: <refusal>` when M1 refuses the live graph (D6, D9). Nothing advanced.
    TopologyChanged { snapshot: String, live: String, rest: Rest },
    /// MOD-138 D5: a v2 run whose snapshot no longer hashes to its own `topology`. Nothing
    /// advanced; parked like `TopologyChanged`.
    SnapshotTampered { stored: String, recomputed: String, rest: Rest },
}
```

Every match site gets the new arm. That is the plan's two plus the conformance helper:

- `engine.rs:1784`: `Resume::Walked(rest) | Resume::TopologyChanged { rest, .. } | Resume::SnapshotTampered { rest, .. } => rest`.
- `htui-worker/src/runtime.rs:2629`: the same three-way pattern.
- `conformance.rs:8501-8504` `resumed`: `Resume::SnapshotTampered { .. } => panic!("nothing edited the snapshot")`.

`git grep "Resume::"` shows no other match.

### T5.2 `resume_window` (`engine.rs:3161-3245`)

```rust
let row = self.run(run).await?;
let snapshot = Self::snapshot_of(&row)?;              // a v1 blob arrives lifted (T1)
let item = self.item(Self::item_of(&row)?).await?;
if snapshot.v >= GraphSnapshot::V {
    // D5 v2: pinned; the live graph is never read.
    let recomputed = graph::topology_v2(&snapshot.phases, &snapshot.edges)?;
    if recomputed == snapshot.topology { return Ok(None); }
    let body = format!(
        "snapshot tampered: run {run} carries topology `{}`, but its own phases and edges hash \
         to `{recomputed}`; the walk is not advanced (ANA-33 §7.2, invariant 2)", snapshot.topology);
    let rest = Box::pin(self.park_on_resume(row, item.id, body)).await?;
    return Ok(Some(Resume::SnapshotTampered { stored: snapshot.topology, recomputed, rest }));
}
// D5 v1: today's live compare, edge-aware.
let lifted = graph::topology_v2(&snapshot.phases, &snapshot.edges)?;
let live = match graph::resolve(...).await {         // the existing call
    Ok(live) => live.snapshot.topology,
    Err(err @ (ResolveError::NotYetRunnable { .. } | ResolveError::InvalidGraph { .. })) =>
        format!("not runnable: {err}"),              // D5: its topology did change
    Err(err @ ResolveError::ProjectCaps { .. }) => { /* unchanged */ }
    Err(err @ (FanOutCap | AgentCap | ReviewFanOut | LocalFanOut | NoCandidate | UnknownTouchedRepo)) => {
        /* unchanged "live graph not comparable" note, naming `lifted` */ }
    Err(err) => return Err(err.into()),
};
if live != lifted {
    let body = format!("topology mismatch: run {run} walks `{lifted}`, the live graph is `{live}`; \
                        the walk is not advanced (ANA-2 §4.9, invariant 2)");
    let rest = Box::pin(self.park_on_resume(row, item.id, body)).await?;
    return Ok(Some(Resume::TopologyChanged { snapshot: lifted, live, rest }));
}
Ok(None)
```

`park_on_resume(&self, row: Run, item: ItemId, body: String) -> Result<Rest, EngineError>` is
`:3208-3237` extracted verbatim. It writes the `add_note`, moves a `running` run to
`awaiting_approval` (run, then item: MOD-4 D159), and returns `self.resting(&row)`. The note keeps
starting with `"topology mismatch"`, which is asserted with `starts_with` at `conformance.rs:7302`.

**Why an unedited v1 graph still walks.** The lifted phases are the stored v1 phases. The live
phases resolve to the same bytes (§1.2's proof), as they do today. The lifted edges are
`default_snapshot_edges(stored phases)`. The live edges are the stored edges, which D6(c) has just
checked equal `default_edges(live rows)`. `0023` and D7 make that true for every graph. The test in
§T5.4 pins it.

### T5.3 The v1 test helper (`fake.rs`, test-support)

```rust
/// MOD-138 T5: rewrites `run`'s stored snapshot as the v1 blob a pre-MOD-138 binary wrote: `v: 1`,
/// no `edges`, `source`, `graph.scope`/`revision`, the four bound keys removed from `settings`, and
/// `topology = graph::topology(&phases)` (the v1 digest). Through `MemStore::set_run_snapshot`.
///
/// # Panics
/// When the run has no decodable snapshot, or a node that is not a linear `step` (v1 had none).
pub fn rewrite_as_v1(store: &MemStore, run: RunId);
```

### T5.4 Tests

**Moved to v1** (F3; each calls `rewrite_as_v1` right after the run exists, before the edit;
assertions unchanged in substance):

| Test | Where to rewrite |
|---|---|
| `conformance.rs` `topology_mismatch_parks_on_resume` (`:1734`) | after `start`, in both halves (the unedited half proves the lift equals live) |
| `conformance.rs` `a_topology_mismatch_parks_a_running_run` (`:5608`) | after `crash`, before `set_input_kinds` |
| `conformance.rs` `a_handed_back_run_on_a_changed_graph_parks_with_the_topology_note` (`:7268`) | after `parked_at_prd(&a)` |
| `conformance.rs` `mod tests` `a_lowered_cap_does_not_strand_a_run_on_resume` (`:9296`) | after `start` (F3) |
| `engine.rs` `parked_run_with_a_moved_graph` (`:12848`), so `a_topology_mismatch_parks_on_resume` (`:12879`); its second half also rewrites | after `started` |
| `engine.rs` `rebinding_a_phase_persona_parks_a_resumed_run` (`:13180`), first half | after `started` |
| `engine.rs` `resume_walks_a_run_whose_touched_paths_name_an_unknown_repo` (`:12224`) | after `started` |

**New `CASES`** (orchestrator 109 → **114**; pins at `conformance.rs:8389` and
`tests/fake_conformance.rs:17`, prose "…, MOD-138 M1's five pinned-resume cases (plan D5) 114"):

| Case | Asserts |
|---|---|
| `a_v2_run_walks_on_after_a_live_edit` | twin of `:1734`: `set_input_kinds` under a v2 run → `Walked`, position 1 created, no `topology mismatch` note |
| `a_running_v2_run_walks_on_after_a_live_edit` | twin of `:5608`: the sweep adopts, `resume` → `Walked`, the run is not parked |
| `a_handed_back_v2_run_on_a_changed_graph_walks_on` | twin of `:7268`: the frontier merge, then `Walked`; no topology note |
| `a_tampered_v2_snapshot_parks_with_both_digests` | after `start` + `approve_without_walking`, `set_run_snapshot` with `phases[0].input_kinds` edited (or `topology` replaced) → `SnapshotTampered { stored != recomputed }`; run and item `awaiting_approval`; lease given back; one `snapshot tampered` note naming both; the next sweep adopts nothing |
| `a_v1_run_whose_live_graph_gained_a_non_default_edge_parks` | v1 fixture; `replace_graph` adds `review → plan` `asks_you` → `TopologyChanged { live starts_with "not runnable: " }`, parked, `topology mismatch` note |

**Engine unit tests** (not counted): the v2 twins of the three engine tests
(`a_v2_run_on_a_moved_graph_walks_on`, `rebinding_a_phase_persona_under_a_v2_run_walks_on`,
`a_v2_run_whose_touched_paths_name_an_unknown_repo_walks_without_a_note`), and
`a_v1_run_on_an_unchanged_graph_walks`, which drives a v1 run through to its next gate.

### T5 commits

1. `MOD-138 T5: Resume::SnapshotTampered; resume_window splits on v (pinned v2, edge-aware v1)`,
   with the match arms (`engine.rs`, `runtime.rs`, `conformance.rs` `resumed`).
2. `MOD-138 T5: v1 fixtures for the seven v1 resume tests; five v2 cases; CASES 114`.
   Validate: `cargo test -p htui-orch --all-features -- --test-threads=1`;
   `cargo test -p htui-worker --all-features`.

---

## T6 — Workspace and library graphs for items and kinds (after T4; ∥ T5 allowed, §2)

### T6.1 The rule and the reader (D12, A4)

```rust
// graph_rules.rs
/// MOD-138 D12: may a kind or item of `project` name a graph of `scope`? Its project's (an
/// override included: the item check refuses overrides itself), a workspace's that holds
/// `project` (`workspaces` = the `workspace_id`s of its `workspace_project` rows), or the library's.
#[must_use]
pub fn graph_usable_by(scope: GraphScope, project: ProjectId, workspaces: &[WorkspaceId]) -> bool;

// traits.rs, WriteStore, after `step_graphs` (:944)
/// MOD-138 D12: every graph a kind or item of `project` may name ([`graph_usable_by`]): the
/// project's, then each workspace's that holds it, then the library's; by name bytes within each
/// scope, then id. Overrides included (callers filter `is_override`, as for `step_graphs`).
///
/// # Errors
/// The backend's own failures only.
async fn usable_graphs(&self, project: ProjectId) -> Result<Vec<StepGraph>>;
```

Implementors:

- MemStore: `graphs.values().filter(graph_usable_by(..))`, sorted
  `(scope rank, name bytes, id)`, with the workspace ids from `workspace_projects`.
- PgStore (`pg/read.rs`, beside `step_graph_rows`; the **new** `query_as!(StepGraphRow, …)`).
  Probed: it lists the project's graphs, `wsflow` of the containing workspace and `libflow`,
  excludes `otherws`, and orders them as stated:
  ```sql
  SELECT id AS "id: StepGraphId", project_id AS "project_id: ProjectId",
         workspace_id AS "workspace_id: WorkspaceId", name, description, is_override, revision,
         created_at, updated_at
    FROM step_graph g
   WHERE g.project_id = $1
      OR (g.project_id IS NULL AND g.workspace_id IS NULL)
      OR g.workspace_id IN (SELECT wp.workspace_id FROM workspace_project wp WHERE wp.project_id = $1)
   ORDER BY CASE WHEN g.project_id IS NOT NULL THEN 0 WHEN g.workspace_id IS NOT NULL THEN 1 ELSE 2 END,
            g.name COLLATE "C", g.id
  ```
  The column order is `StepGraphRow`'s (`pg/read.rs:2996-3006`); note its positional-binding
  comment.
- `Writer` (`writer.rs`, beside `:718`), forwarding.
- The two spies: `htui-agent/src/conformance.rs` (beside `:997`) and `htui-agent/tests/recorder.rs`
  (beside `:724`), forwarding.

### T6.2 The checks

- **Kinds, MemStore** `check_item_kind` (`mem.rs:3366-3372`): the scope test becomes
  `!graph_usable_by(row.scope, project, &self.workspaces_of(project))`, and the sentence
  `graph_not_usable`. Doc `:3347-3349` updated.
- **Kinds, PgStore**:
  - `create_item_kind` guard (`pg/write.rs:3427`) becomes
    `WHERE EXISTS (SELECT 1 FROM step_graph g WHERE g.id = $6 AND (g.project_id = $2 OR (g.project_id IS NULL AND g.workspace_id IS NULL) OR g.workspace_id IN (SELECT wp.workspace_id FROM workspace_project wp WHERE wp.project_id = $2)))`;
  - `update_item_kind` guard (`:3511-3513`): the same predicate over `item_kind.project_id` and
    `COALESCE($6, item_kind.default_graph_id)`;
  - the sentences at `:3551` and in `kind_guard_refusal` (`:9755`) become `graph_not_usable`.
  - Probed: `feature`, `wsflow` and `libflow` usable, `otherws` not; a kind on `wsflow` inserts.
  - The two changed literals need `cargo sqlx prepare`.
- **Items.** No store check exists (F10). `item_writes.rs::catalogue` (`:318-323`) reads
  `writer.usable_graphs(project)` instead of `step_graphs`, still minus `is_override`. The doc at
  `:76` becomes "`usable_graphs(project)` minus `is_override` (A3, MOD-138 D12)".
  `item_spec::check_graph` keeps its code; its doc (`:319-320`) and `SpecContext.graphs`' doc
  (`:57-60`) say "the graphs the project may use". `SpecError::Graph`'s text (`:153`) becomes "that
  step graph is not one this project can use". An item still never names another item's override:
  overrides are filtered out of the list, and only the item's own unchanged override bypasses the
  check, as today.
- **A3, `delete_workspace`** in both stores, before any write:
  - Mem `State::delete_workspace` (`mem.rs:5306`), after `NotFound`: the first holder in
    `(graph name bytes, holder bytes)` order, among the kinds whose `default_graph_id` and the items
    whose `step_graph_id` is in `workspace_graphs(id)`. A holder → `Constraint(
    workspace_graph_is_held(graph, "item_kind PREFIX" | "item KEY", project slug))`.
  - Pg `delete_workspace_once` (`pg/write.rs:9696`), after `workspace_reach`, in the same
    repeatable-read transaction. The **new** `query!`, probed:
    ```sql
    SELECT g.name AS "graph!", pr.slug AS "project!", 'item_kind ' || k.prefix AS "holder!"
      FROM item_kind k JOIN step_graph g ON g.id = k.default_graph_id JOIN project pr ON pr.id = k.project_id
     WHERE g.workspace_id = $1
    UNION ALL
    SELECT g.name, pr.slug, 'item ' || i.key
      FROM item i JOIN step_graph g ON g.id = i.step_graph_id JOIN project pr ON pr.id = i.project_id
     WHERE g.workspace_id = $1
     ORDER BY 1 COLLATE "C", 3 COLLATE "C" LIMIT 1
    ```
    The FK's `23503` stays the backstop for a holder that commits after this read.
  - The trait doc (`traits.rs:1362-1367`) gains that `Constraint`.
- **Removing** a project from a workspace (`remove_workspace_project`) is not refused. The rule is
  checked when a kind or item is written. The kind keeps its graph and still resolves; its next
  edit is refused by the guard until it names a usable graph. This is documented on
  `graph_usable_by`.
- **UI lists only.**
  - `catalogue.rs`: `ProjectCatalogue` gains `shared: Vec<StepGraph>` (the workspace and library
    rows of `usable_graphs`, in its order). `snapshot` (`:107-121`) reads `usable_graphs` once: the
    project rows get a `graph_body` as today, the others go to `shared`. A
    `ProjectCatalogue::graph_name(id) -> Option<(&str, GraphScope)>` searches both.
  - `kinds.rs`: `graph_named` (`:1855-1862`) tries the project's graphs first, then `shared` by
    name. `NO_GRAPH_NAMED` (`:118`) becomes "`graph` names no graph this project can use", and
    `htui/tests/kinds.rs:1576` moves with it.
  - `kind_line` (`:1304-1316`) and `edit_kind_fields` (`:1760-1764`) label through `graph_name`; a
    shared graph is drawn as `graph {name} · workspace` / `· library`, with no phase rows, because
    `rows()` (`:442-477`) only expands project graphs.
  - `item_form.rs` `picker_label` (`:986-994`) appends the same `· workspace` / `· library` suffix
    for a non-project graph.
  - The demo fixture holds no workspace or library graph, so no insta snapshot moves. Confirm with
    `cargo insta test --check`.
- **`override_graph`** needs no code change. `graph_body` reads any scope (`traits.rs:1077-1082`),
  the clone is created in `GraphScope::Project(item.project_id)` (`graph.rs:643-645`, `:704-712`),
  and a workspace or library phase carries no phase binding (MOD-137 F9), so `copies` is empty.
  Update the docs at `graph.rs:466-470` and `orphaned_override`'s `:819-821` ("only an item of the
  graph's own project can name it" still holds for overrides). Templates resolve in the run's
  project (`snapshot_phase`'s `prompt_template(project, …)`, `graph.rs:1003-1006`).

### T6 tests (first)

| Test | Asserts |
|---|---|
| `graph_rules.rs` `graph_usable_by_project_workspace_library` | the four scopes against a project in one workspace; an override of the project → `true` |
| conformance `item_kind_takes_usable_graphs_only` | create a workspace with the project linked; a kind on a library graph, then on that workspace's graph → `Ok`; another workspace's graph → `Constraint` = `graph_not_usable`; `update_item_kind` to each, the same |
| conformance `usable_graphs_lists_project_workspace_library` | exact ids and order; another workspace's graph absent; after `remove_workspace_project`, its graphs absent |
| conformance `delete_workspace_refuses_a_held_graph` | a kind on the workspace graph → `Constraint` = `workspace_graph_is_held`, nothing deleted (`delete_reach` unchanged); kind repointed → the delete applies |
| `item_spec.rs` `a_workspace_or_library_graph_in_the_context_is_accepted` | `check_graph(Some(lib))` with `lib` in `ctx.graphs` → `Ok`; the existing `:868`/`:893` tests keep their meaning, text updated |
| `item_writes.rs` `the_form_offers_library_and_workspace_graphs` | after creating a library graph, `ItemForm`'s `graphs` lists it after the project's; a mint naming it lands |
| `htui/tests/kinds.rs` `a_kind_can_take_a_library_graph` | typing a library graph's name creates the kind; its line says `· library` |
| `graph.rs` `an_items_run_resolves_a_library_graph_with_the_projects_templates` | kind default = a library graph with a `review` phase; resolve → `graph.scope == Some(Library)`, template version from the item's project |
| `graph.rs` `an_override_of_a_library_graph_clones_into_the_project` | `override_graph` → the clone's scope is `Project(item.project_id)`, edges copied; a re-override re-clones from the library graph |

Store `CASES` 215 → **218** (both pins; prose "…, and three for D12's usable graphs (plan D12)").
`READ_CASES` stays 15 (`usable_graphs` is on `WriteStore`).

### T6 commits

1. `MOD-138 T6: graph_usable_by, usable_graphs, widened kind guards, delete_workspace holder refusal`
   (core, both stores, `Writer`, spies, `.sqlx`, conformance, pins).
2. `MOD-138 T6: item and kind forms list workspace and library graphs`.
   Validate: both conformance suites `--test-threads=1`; `cargo test -p htui --all-features
   --features testkit` (memory: integration tests need testkit); `cargo sqlx prepare --check`.

---

## 2. File sets and independence

| Task | Files |
|---|---|
| T1 | `htui-core`: `model/run.rs`, `model/mod.rs`, `seed.rs`, `fixtures.rs`, `store/mem.rs` (literal `:13436`, `set_run_snapshot`), `store/conformance.rs` (literal `:5173`). `htui-orch`: `command.rs`, `graph.rs` (literals), `engine.rs` (literals), `tests/fixtures.rs`. Plus `htui-agent/tests/relay.rs`, `htui-mcp/tests/tools_backlog.rs`, `htui-store/tests/{cache,pg_criteria}.rs`, `htui/tests/backlog.rs` |
| T2 | `htui-core`: `store/graph_rules.rs`, `store/traits.rs`, `store/mod.rs`, `store/mem.rs` (`replace_graph`), `store/conformance.rs`, `tests/mem_store.rs`. `htui-store`: `pg/write.rs` (`replace_graph`), `tests/pg_conformance.rs`. `htui-orch/src/graph.rs` (`override_graph` pre-check) |
| T3 | `store/mem.rs` (3 writers + helper), `store/traits.rs` (docs), `store/graph_rules.rs` (`linear` and its test, A5), `store/conformance.rs`, `tests/mem_store.rs`; `pg/write.rs` (3 writers + helpers), `migrations/0023_default_edges_repair.sql`, `tests/{migrations,connect,pg_conformance}.rs` |
| T4 | `model/kind.rs`, `store/mem.rs` (`resolve_graph` + test), `pg/read.rs`, `htui-store/tests/pg_criteria.rs`, `htui-orch/src/graph.rs`, `htui-orch/tests/fixtures.rs`, `tests/fixtures/feature.v2.snapshot.json` |
| T5 | `htui-orch/src/{engine.rs,conformance.rs,fake.rs}`, `htui-orch/tests/fake_conformance.rs`, `htui-worker/src/runtime.rs` |
| T6 | `store/{graph_rules,traits,mod,mem,conformance}.rs`, `model/item_spec.rs`, `tests/mem_store.rs`; `pg/{read,write}.rs`, `writer.rs`, `tests/pg_conformance.rs`, `.sqlx/`; `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`; `htui/src/{item_writes,catalogue}.rs`, `htui/src/ui/tabs/settings/kinds.rs`, `htui/src/ui/tabs/backlog/item_form.rs`, `htui/tests/kinds.rs`; `htui-orch/src/graph.rs` (tests, docs) |

| ∩ | T2 | T3 | T4 | T5 | T6 |
|---|---|---|---|---|---|
| **T1** | mem, conformance, graph | mem, conformance | mem, graph, fixtures.rs, pg_criteria | engine | mem, conformance, graph |
| **T2** | | mem, traits, conformance, mem_store, write, pg_conformance | mem, graph | — | graph_rules, traits, mod, mem, conformance, mem_store, write, pg_conformance, graph |
| **T3** | | | **mem** (+ F6 behavioural) | — | mem, traits, conformance, mem_store, write, pg_conformance |
| **T4** | | | | — (but T5 needs `topology_v2` and v2 snapshots) | mem, read, graph |
| **T5** | | | | | **∅** |

**Correction to the plan's T3/T4 row (A2).** The file intersection is `mem.rs` only, not "mem.rs
and the PgStore files": T3 changes `pg/write.rs`, T4 changes `pg/read.rs`. But F6 makes T4
untestable without T3: the fake suite's repoint clones are refused by D6(c) until D7 lands. A T4
worktree branched before T3 merges cannot validate. The order is therefore **serial**:
T1 → T2 → T3 → T4 → {T5 ∥ T6} → close-out.

T5 ∥ T6 is genuinely disjoint, with no hidden coupling found:

- T5 touches no store file (`set_run_snapshot` lands in T1);
- T6 touches no orchestrator walk file;
- the demo data is unchanged by both;
- the pins are in different files.

If the orchestrator wants them parallel, use worktrees (memory: workflow worktree implementers;
about 10 GB of `target/` each), merging T5 then T6. Serial is equally fine.

## 3. Gate commands per task

```bash
cargo check --workspace --all-targets --all-features                      # every commit
cargo test -p htui-core --all-features -- --test-threads=1                # T1-T3, T6
cargo test -p htui-store --all-features --test pg_conformance --test migrations --test connect --test pg_criteria -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1                # T1, T4, T5
(cd crates/htui-store && cargo sqlx prepare --check)                      # T3 (expect clean), T6 (after prepare)
# T6 prepare: a migrated scratch DB, not the compose `htui` DB (memory)
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE prep138"; \
  (cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/prep138 sqlx migrate run && \
   DATABASE_URL=postgres://postgres@localhost:5439/prep138 cargo sqlx prepare -- --all-targets --all-features)
# close-out: the plan's Validation block, plus `grep -c SIGABRT` = 0 and the featureless clippy
```

## 4. What implementers must know (not in the plan)

1. **Stack (ANA-33 §9, memory "htui-orch test stack headroom").** M1 adds no per-node future, and
   three awaits are boxed deliberately:
   - `Box::pin(source.resolve_graph(..))` in `graph::resolve` (PgStore's future now holds a
     transaction);
   - `Box::pin(resolve_graph_on(..))` inside PgStore;
   - `Box::pin(self.park_on_resume(..))` at both of `resume_window`'s park sites.

   Every new conformance case is a boxed `run_case` arm (MOD-137's rule). The PgStore D7 path
   reuses `Box::pin(insert_edge(..))`. Run the workspace gate `--no-fail-fast` and grep `SIGABRT`:
   `pg_conformance` overflows only in workspace builds.
2. **Never route either digest through `Value`** (finding 4, F5). `topology_v2` serialises a
   borrowed typed struct; the predicate goes through `SortedJson`.
3. **Field order is a contract.** The four new `SnapshotPhase` fields go after `persona`. A field
   inserted earlier, or one without a skip predicate, moves every v1 digest and parks every
   in-flight v1 run.
4. **`default_edges` is used by four rules and must not drift**: the T1 equality test, the
   0020/0023 equality tests, D6(c) and D7. Change it only with all of them.
5. **Edge ids are minted fresh** by D7 and `0023` (UUIDv7 in the stores, `gen_random_uuid` in SQL).
   Compare edges by shape (`edge_shape`, `store/conformance.rs:21772`), never by id, except for the
   untouched authored graphs.
6. **`:` in phase names.** Expansion keys are `<name>:plan`, and resolve refuses a collision.
   MOD-139 should reserve the suffix at save time. That is out of scope here, so record it as a
   MOD-139 note at close-out.
7. **Close-out docs**:
   - `docs/ANA-2.md` invariant 2 (v2 runs pinned; v1 keeps the compare);
   - `HANDOFF.md` migration coordinates (`0023` taken);
   - `docs/decisions/mod/mod-138.md` M1 section;
   - the plan's Validation block (MOD-137 F3: run `sqlx prepare` from inside `crates/htui-store`).

---

## Plan amendments needed

- **A1 — D1's `split` field (F1).** D1 asks for `SnapshotPhase.split` "skipped when
  `split = Split::for_fan_out(fan_out)`". serde's `skip_serializing_if` sees only the field
  itself, never `fan_out`, so that predicate cannot be written. **Smallest amendment:** `split` is
  not a stored field but a derived accessor, `SnapshotPhase::split()`: `Divide` when the node
  carries a `divide` block, else `Split::for_fan_out(fan_out)`. `ck_phase_split_fan_out` makes the
  two equivalent. `divide: Option<SnapshotDivide>` is `Some` exactly on a divide node. Nothing
  serialises `split`, so byte-identity holds trivially.
- **A2 — T3/T4 run serially, not in parallel worktrees (F6).** The plan's intersection is
  overstated: the files T3 and T4 share are `mem.rs` alone, since T3 changes `pg/write.rs` and T4
  changes `pg/read.rs`. But T4's D6(c) refusal fails the fake conformance suite unless T3's D7 is
  present. The repoint helpers build clones through `create_phase` with no edges
  (`conformance.rs:1202`, `:1267`; `engine.rs:7851`; `tests/gix_isolator.rs:336`;
  `tests/fixtures.rs:36-103`). **Smallest amendment:** T4 runs after T3 on the same tree. In
  exchange, T5 and T6 are fully disjoint and may run in parallel worktrees after T4. The plan
  serialised T6 after T5. Two small moves keep every commit green and T5 ∩ T6 empty:
  - `snapshot_of`'s 1|2 read and the lift move from T5 to T1;
  - the `MemStore::set_run_snapshot` test seam lands in T1.
- **A3 — `delete_workspace` must refuse a held graph (F9).** D12 lets a kind or item name a
  workspace graph, and `delete_workspace` cascades that graph. `item_kind.default_graph_id` and
  `item.step_graph_id` have no cascade, so PgStore fails mid-delete with `23503
  item_kind_default_graph_id_fkey` (probed) and MemStore leaves a dangling reference: a parity
  break. **Smallest amendment:** both stores refuse the delete before writing, with
  `workspace_graph_is_held(graph, holder, project)`. That is one new Pg query and one
  store-conformance case. Removing a project from a workspace is not refused: the rule is checked
  when a kind or item is written.
- **A4 — D12 needs one new store read (F8).** No reader lists workspace or library graphs:
  `step_graphs(project)` filters `project_id`, and `graph_body` needs an id. So "offer workspace
  and library graphs (lists only)" cannot be built from existing reads. **Smallest amendment:**
  `WriteStore::usable_graphs(project) -> Vec<StepGraph>`, ordered project, then workspace, then
  library, by name. It has five implementors (MemStore, PgStore, `Writer`, and the two
  `htui-agent` spies) and one new `query_as!`, so `cargo sqlx prepare` is needed. T6's file set
  grows by `writer.rs`, the two spy files and `.sqlx/`.
- **A5 — the default-edge rule leaves palette phases out (found in T3).** D7 and D8 as written
  apply `seed::default_edges` to every phase. A palette node may carry no edge (MOD-137 rule e), so
  `create_phase` of a palette row on a default-edged graph would chain a `next` onto it, and
  `replace_graph` would then refuse the graph's own read-back body. **Adopted amendment:** the rule
  reads only the non-palette phases. `default_edges_follow` filters both sides through
  `graph_rules::linear` (an edit to T2's `graph_rules.rs`, outside T3's file set), and `0023` adds
  `NOT p.palette` / `NOT c.palette` / `NOT rv.palette` (§T3.2 shows the shipped file). The chain
  runs past a palette phase and no loop starts or ends on one. One more store-conformance case,
  `create_phase_leaves_a_palette_phase_unlinked`, moves `CASES` to 215 at T3 and T6's pin to
  215 → 218. Downstream: D6(c) compares against the same palette-excluding rule (moot in M1, since
  (b) refuses a palette node first); the v1 lift (`default_snapshot_edges`) is unaffected, as
  every v1 phase has `palette = false`.
