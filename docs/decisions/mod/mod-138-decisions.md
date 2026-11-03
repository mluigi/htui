# MOD-138 - Snapshot v2 and the graph walker: the decisions so far, milestone 1 (in progress, 2026-11-03)

**Requirements:** `R-ORCH-1`, `R-ORCH-2`, `R-ORCH-3`, `R-ORCH-4` (seam), `R-ORCH-11`.
**Origin:** ANA-33 (`docs/ANA-33.md` §7.2, §7.3, §7.7, §10; `docs/decisions/ana/ana-33.md`).
**Artifacts:** PRD `.claude/prds/mod-138-snapshot-v2-graph-walker.prd.md` (PRD gate Q1-Q4); M1
`.claude/plans/mod-138-m1-snapshot-v2.plan.md` (decisions D1-D13, amendments A1-A7 in its Status block,
verified claims) and `.claude/plans/mod-138-m1-snapshot-v2.blueprint.md` (findings F1-F19).
Routed **PRD** by `/handoff-run` (C2 and C4, at the threshold), ultracode for implement and review, built on
`hr/MOD-138` in an hr sandbox.

MOD-138 is still open in `HANDOFF.md`, so this is not its close-out write-up: that one is written
under `docs/decisions/mod/` with the item's own name when M3 lands, and it folds this file in (the MOD-115
precedent: an index line for an open item would be a reused ID).

## PRD gate (maintainer, 2026-10-27)

- **Q1:** the v2 snapshot freezes a `divide` node's expansion (`<name>:plan`, role `planner`, then `<name>`,
  role `integrate`), and resolve refuses to start such a run until MOD-139. Same for subgraph nodes (MOD-143).
- **Q2:** an `asks_you` edge is answered through the existing gate answer (M2).
- **Q3:** validated by tests only; branching graphs get real use with MOD-141's editor.
- **Q4:** three milestones: M1 snapshot v2, validator and lift; M2 the readiness walker; M3 concurrent nodes.

## Milestone 1: what was built (2026-10-28 to 2026-11-03)

- **Snapshot v2** (`htui-core` `model/run.rs`). `GraphSnapshot::V` is 2, `V1` 1. New:
  - `edges` keyed by node name, `source` (`pin` / `kind_default`);
  - `SnapshotGraph.scope` and `revision`;
  - the bounds (`max_parallel_sessions` 4, `max_plan_revisions` 3, `max_child_runs` 8, `runtime_changes` `ask`),
    read from `app_setting` with code defaults (D13);
  - per node `role`, `palette`, `plan_scope`, `divide`, after `persona` and skipped at their defaults, so a
    linear phase serialises byte-for-byte as v1 and keeps its v1 digest. `split` is a derived accessor (A1).
  - The v2 `topology` hashes typed `{phases, edges}` (edges in one canonical order, predicates key-sorted);
    never through `serde_json::Value`.
- **The v1 lift.** `command::snapshot_of` reads v1 and v2 and lifts a v1 snapshot through the default-edge
  rule (`seed::default_edge_plan`, shared with `seed::default_edges`), keeping `v == 1`.
- **One graph validator** (`store/graph_rules.rs`, D9): forward edges (`next`, `if`, a forward `asks_you`)
  are acyclic, and a `changes_requested` or `fails` edge must loop back. Called by both stores'
  `replace_graph`, by the override clone and by resolve, with one sentence each.
- **Default edges kept in step** (D7, A5). `create_phase`, `update_phase` (name or position) and
  `delete_phase` re-derive a default-edged graph's edges in the same write, in both stores (PgStore after
  MOD-137's revision bump, so no new lock order). Authored edges are left alone; palette phases are outside
  the rule. **Migration `0023_default_edges_repair.sql`** repaired graphs Settings › Kinds had edited row by
  row since `0020` (D8).
- **Resolve writes v2** (T4). PgStore reads graph, phases, agents, parts, edges and personas in one
  `REPEATABLE READ, READ ONLY` transaction. A divide node is expanded and then refused; resolve refuses
  `InvalidGraph` (the validator's sentence) and `NotYetRunnable` (divide, palette, or edges other than the
  default ones: M1 still walks by position).
- **Pinned resume** (T5, D5, A6). A v2 run never reads the live graph; if its own phases and edges no
  longer hash to its digest it parks as `Resume::SnapshotTampered` with a note. A v1 run compares its lifted
  snapshot with the live graph, and a live graph M1 refuses is compared on the phases alone, as before
  MOD-138, so pre-upgrade runs on a palette or authored-edge graph are not stranded.
- **Workspace and library graphs** (T6, D12, A3, A4, A7). Kinds and items may name their project's graphs,
  a graph of a workspace holding the project, or a library graph (`graph_usable_by`, `usable_graphs`).
  `delete_workspace` refuses while a kind or item holds one of its graphs. A kind's graph is checked only
  when an edit changes it, as an item's is. The item and kind forms label shared graphs `· workspace` /
  `· library`.

## Plan amendments (all maintainer-accepted)

| # | When | What |
|---|---|---|
| A1 | blueprint, 2026-10-28 | `split` is derived, not a snapshot field (serde cannot skip on a sibling) |
| A2 | blueprint, 2026-10-28 | T1→T4 serial (T4's refusal needs T3's edges), then T5 ∥ T6 in worktrees |
| A3 | blueprint, 2026-10-28 | `delete_workspace` refuses a held graph (the FK would fail mid-delete) |
| A4 | blueprint, 2026-10-28 | new `WriteStore::usable_graphs` read |
| A5 | T3 verify, 2026-10-29 | palette phases outside the default-edge rule and `0023` |
| A6 | T5 verify, 2026-10-29 | a v1 run on a refused live graph compares on the phase-only digest |
| A7 | review R1, 2026-10-29 | a kind's graph is checked only when an update changes it |

## Review

`rust-reviewer` ran as three area instances (core; Postgres and migration; orchestrator and UI): approve
with fixes, no critical or high finding. Each finding was checked by an adversarial verifier. Confirmed and
fixed in round R1: the kind-edit lock-out (medium; A7), stale docs, the `0023` palette probe without drifted
edges, and the missing Postgres race tests. The race tests (`pg_criteria.rs`) were each shown to fail with
the guarded behaviour removed. Refuted: a workspace-delete race giving the raw FK error, which is the A3
backstop by design and still refuses; two test-strength claims already covered elsewhere; two resume-note
claims. Deferred to **MOD-141**: two workspaces holding a project may each own a graph of one name, and the
pickers then show two identical `name · workspace` entries and take the first typed match. Noted for
**MOD-139**: reserve the `:plan` suffix in phase names at save time (resolve refuses a collision today).

## Pins after M1

Store conformance `CASES` 218 (was 206); `READ_CASES` 15; `htui-orch` `CASES` 115 (was 109); migrations
through `0023_default_edges_repair` (`TABLES` 51 unchanged; no cache migration).

## Commits

On `hr/MOD-138` from `8a4a88c6`: PRD and plan `41f7b234`, `4cd56ee0`, `c3daa9c5`, `9231ac43`, `da737997`,
`99fdf5ca`; T1 `2cfb28a7`..`d4eb9b52`; T2 `723f1e8a`..`985b23a8`; T3 `dc7cce8c`..`f2953a2d`; T4 `c627229a`,
`2e645d54`; T5 `4211020e`..`056c7d48` (merge `494b7e03`); T6 `f5a68b54`..`f165f4de` (merge `25ac5c19`); R1
`9842e42a`..`d114f91f`.

## Gate

See the M1 plan's Status block for the final gate on the merged tree. Known failures outside MOD-138, all
reproduced on `main` or environmental: the sandbox Qdrant times out (`qdrant_live`, `qdrant_worker`,
`state_cli_pg`); `htui-store` `cache.rs` `the_spawned_refresher_passes_and_follows_the_scope` fails on `main`
(`8a4a88c6`) too. The host's Postgres gate on the merged tree is owed, as for every `hr` run.
