# ANA-33 - Step graphs v2 and divided fan-out (concluded, 2026-10-18)

Full analysis: `docs/ANA-33.md` (surface as read, prior art, options, risks, sources). Origin: the
2026-10-15 requirements review (`docs/requirements-review-2026-10-15.md`), with the UI redesign's
workflows (`docs/ui-redesign-findings-2026-10-15.md` §4.6, §5.2) and ANA-40 P7. Addresses `R-ORCH-1`,
`R-ORCH-3`, `R-ORCH-7`, `R-ORCH-8`, `R-ORCH-11`, `R-MCP-2`, `R-ID-6`, `R-TUI-4`, `R-TUI-8`.

## Verdict

The maintainer accepted all ten decisions of `docs/ANA-33.md` §8 as recommended (2026-10-18).

1. **One graph model.** Workflows are `step_graph` rows in the global scope; there is no second
   workflow table. Phase rows stay the nodes, a new `step_graph_edge` holds typed links (`next`, `if`,
   `asks_you`, `changes_requested`, `fails`), and a graph is owned by a project, a workspace or the
   global library. `step_graph.revision` is the whole-graph CAS token for one-transaction edits.
   Back edges are `changes_requested`, `fails` or `asks_you`, each bounded by its `limit`; `retries`
   is a node's own `retry_limit`, a self-loop (§7.1, answering ANA-40 P7).
2. **Parts are planned nodes at appended positions**, recorded in an append-only `run_plan` before
   any starts; `fanout_index` stays compete-only, and the `run_step` key is unchanged (§7.4).
3. **Integrate** is a deterministic pre-merge, an agent completing the merge in its own tree, and an
   ancestry post-check; only the integrate branch lands, through ANA-2 §4.6's unchanged reconcile
   (§7.5). The orchestrator still never resolves a conflict.
4. **A v2 run is pinned to its snapshot**; resume checks the snapshot's own digests and no longer
   compares against the live graph. v1 snapshots stay readable through a lift (§7.2).
5. **Run-time changes:** any step may submit a `successors` plan when `runtime_changes` is not `off`
   (default `ask`); hard-gated and `asks_you` nodes can never be bypassed (§7.6).
6. **`if` takes a closed set of deterministic predicates**; a branch chosen by an agent goes through
   a planner (§7.3).
7. **MOD-27 is narrowed** to consult-mode `spawn_subagent`; `RunKind::Swarm` is dropped, and graph
   steps deny the harness's own subagent tool once it lands (§7.12).
8. **MOD-98's "opt-in"** is a seeded alternative graph plus a library workflow, with no disabled-node
   flag (§7.12).
9. **`max_agents_per_run` counts planned sessions**, not retries or rounds, matching the code; ANA-2
   §4.5's text is corrected (§7.8).
10. **A divided step with a failed part fails as a whole**; integrate never runs on a partial set
    (§7.4).

The walk becomes a readiness set (all-joins, derived skips, per-edge loop targets with region
retirement), the planner gets three MCP tools (`plan_bounds`, `plan_submit`, `plan_status`), htui's
own stages (`pick`, `prepare`, `collect`) get `role = 'stage'` rows at positions -3..-1, and one
rataflow editor edits every `step_graph`. The ANA-34 boundary is §7.13: ANA-33 owns parts and
integrate inside a run, ANA-34 owns what the coordinator does with the integrated result.

## Why

The 2026-10-15 review withdrew both premises of ANA-2 §4.1's linear graph and rival fan-out:
`R-ORCH-1` now branches, runs in parallel and loops, and `R-ORCH-7` means divided work. Extending the
phase rows with edges (rather than a second workflow model) keeps one validator, one snapshot and
one editor; appended positions keep the shipped `UNIQUE (run_id, position, attempt, fanout_index)`
key and crash recovery's artefact test; and confining conflict resolution to an agent in its own
tree keeps `R-ID-6` and §4.6 intact. Options and prior art are `docs/ANA-33.md` §4 and §6.

## Spawned items

| Filed | Subject | Blocked on |
|---|---|---|
| MOD-137 | Step graph v2 storage: edges, owner scope, revision, atomic edits | not blocked |
| MOD-138 | Snapshot v2 and the graph walker | MOD-137 |
| MOD-139 | Divided fan-out runtime: planner, parts, integrate | MOD-138 |
| MOD-140 | The planner's MCP tools and run-time changes | MOD-139 |
| MOD-141 | One step-graph editor for Settings and the Toolbox | MOD-137, MOD-138, MOD-115 |
| MOD-142 | Runs pane for divided and planned runs | MOD-139 |
| MOD-143 | Subgraph nodes: one graph runs several items | MOD-138, ANA-35, ANA-34 |

## Items amended

- **MOD-27:** narrowed to consult-mode `spawn_subagent`; blocked on MOD-139.
- **MOD-98:** opt-in settled (seeded alternative graph and a library workflow); still not blocked.
- **MOD-75:** its question park meets concurrent nodes; part-versus-run parking is left to its plan.
- **MOD-111:** step control addresses nodes by position and `node_key`.
- **MOD-120:** its run graph reads the v2 graph, plans and stage rows (MOD-142).
- **MOD-123:** its Workflows section carries the `runtime_changes` default.
- **MOD-124:** its workflow editor is MOD-141's component; MOD-124 keeps the Toolbox shell.
- **MOD-125:** its test matrix gains divided steps and subgraph nodes.
- **MOD-126:** the pick's home is §7.11; it waits on MOD-139's stage rows.
- **MOD-132:** gains the `prepare` and `collect` stage rows from MOD-139.
- **ANA-34, ANA-36, ANA-41, MOD-36, MOD-112:** pointers moved from the open ANA-33 to this write-up.
- **MOD-36 and ANA-25:** unchanged, later tier (`R-LATER-13`).

## Text edits applied (2026-10-18)

Dated amendment notes, not rewrites:

- `docs/ANA-2.md`: §4.1's DAG and per-task rows superseded; invariant 2 (v2 runs pinned to their
  snapshot); invariant 3 and §4.6 step 3 (integrate pre-merge, agent and ancestry check, only the
  integrate branch lands); invariant 8 (part documents excluded from later phases); §4.5's cap text
  (`max_agents_per_run` counts planned sessions, not retries).
- `docs/ANA-13.md`: §3.2 and §3.3 superseded by ANA-33 §7.12.
- `docs/ANA-30.md`: §4 U5 "adds one node to the graphs that opt in; other graphs are unchanged".
- `docs/REQUIREMENTS.md`: `R-MCP-3` (meaning of `fan_out_only`), `R-TUI-8` ("with the shared graph
  editor"), `R-ORCH-1` (`retries` is a self-loop), recorded in the header.
- `CONCEPTS.md`: step graphs are owned by a project, a workspace or the global library.

## Not verified

See `docs/ANA-33.md` §9: the web rows of §4 (survey summaries); whether `merge_no_ff`'s two-parent
post-condition holds in a `copy` tree after fetch; `deny_kinds` coverage of every adapter's edit
tools for a read-only planner (MOD-140); whether ancestry is a strong enough integrate check.

## Commits

- `cf41f323` - the draft (`docs(ana): ANA-33 draft, step graphs v2 and divided fan-out`).
- The close-out commit follows it on `hr/ANA-33` (`docs(handoff): ANA-33 concluded; spawn
  MOD-137..143, ...`).
