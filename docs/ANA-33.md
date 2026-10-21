# ANA-33 - Step graphs v2 and divided fan-out

> **Scope note:** This ANA settles six questions:
>
> 1. the step-graph model, its storage, and the snapshot a run freezes (`R-ORCH-11`);
> 2. the planner's MCP tools;
> 3. how a part maps onto `run_step` (`UNIQUE (run_id, position, attempt, fanout_index)`) and onto
>    crash recovery (ANA-2 §4.9);
> 4. how the integrate step relates to ANA-2 §4.6's reconciliation, where the orchestrator never
>    resolves a conflict;
> 5. the Settings graph editor;
> 6. what becomes of MOD-27 (swarm, `spawn_subagent`), MOD-36 and ANA-25 (now later tier,
>    `R-LATER-13`), and what the verdict means for MOD-98 and ANA-30.
>
> Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md`,
> `docs/requirements-review-2026-10-15.md` (§6 `R-ORCH-1`/`-3`/`-7`, §10 `R-MCP-2`, §11 `R-TUI-4`/`-8`)
> and `docs/ui-redesign-findings-2026-10-15.md` (F) §4.6 and §5.2. It came out of the 2026-10-15
> requirements review (`HANDOFF.md:105-120`). Read it beside ANA-34 (the coordinator, "best read beside
> ANA-33", `HANDOFF.md:121-133`) and ANA-40 (workflows, accepted).
>
> **Requirements addressed:** `R-ORCH-1`, `R-ORCH-3`, `R-ORCH-7`, `R-ORCH-8`, `R-ORCH-11`, `R-MCP-2`,
> `R-ID-6`, `R-TUI-4`, `R-TUI-8`. Touched at their seams:
> - `R-ORCH-2` (gates, hard flag) and `R-ORCH-4` (step control);
> - `R-ORCH-9` (overlap) and `R-ORCH-14` (coordinator, ANA-34);
> - `R-ORCH-15` (targets, ANA-42) and `R-ORCH-16` (stop and resume);
> - `R-MCP-1` (tool scope) and `R-MCP-3` (`command_run` exposure);
> - `R-AGT-8` (candidate order), `R-TUI-7` (Toolbox editor) and `R-TUI-10` (configurable keys);
> - `R-LATER-13` (compete fan-out) and `R-ENT-8` (status moves to the run, ANA-35).
>
> **Status (2026-10-18): concluded; the maintainer accepted all ten §8 decisions.** No implementation
> code is written here. Write-up: `docs/decisions/ana/ana-33.md`. Verdict (§7): **one graph model, phase rows as nodes plus typed edge rows, owned by a
> project, a workspace or the global workflow library; a run freezes a v2 snapshot and is pinned to it;
> a divided step expands into a planner node, the parts it plans, and an integrate node; parts are
> planned nodes at their own appended positions, recorded in an append-only `run_plan` before any of
> them starts; the integrate agent merges the parts in its own tree after a deterministic pre-merge,
> and only its branch reaches §4.6's unchanged reconciliation.** Spawned: **MOD-137** to
> **MOD-143** (minted 2026-10-18 through `scripts/hr-mint`).

---

## 1. Context and problem statement

ANA-2 (concluded 2026-09-06) gave `htui` a linear step graph. `position` is dense, and "the only way to
revisit a position is `attempt + 1`". Fan-out is rival: N candidates run one prompt, and a judge or a
person keeps one result. ANA-2 §4.1 explicitly rejected a DAG with edges and "a per-task child table so
N parallel sessions can run N *different* prompts" (`docs/ANA-2.md:256-259`). It rejected them because
the requirements then said "ordered list" and "same prompt". It recorded task fan-out as Open for the
maintainer 1, with the default "rival fan-out only in v1" (`docs/ANA-2.md:2222`).

The 2026-10-15 review withdrew both premises:

- `R-ORCH-1` now lets graphs branch, run phases in parallel and loop. A graph may be owned by a
  workspace, may run several items as subgraphs, and may be planned at run time by the agent that
  starts it, within the project's bounds, with hard gates still enforced (`docs/REQUIREMENTS.md:365-382`).
- `R-ORCH-7` now means *division of work*: a planner, parallel parts each in its own tree, then an
  integrate step that merges branches and artefacts and resolves conflicts, before review
  (`:404-409`). Best-of-N moved to `R-LATER-13` and stays in code.
- `R-ORCH-3` makes the loop-back target a per-phase declaration (`:388-390`).
- The UI review then added a global library of workflows with typed links (`next`, `approved`,
  `asks you`, `if`, `changes requested`, `fails`, `retries`). ANA-40 (accepted) split the work: a
  workflow is the authored template and ANA-33 owns the run graph. ANA-40 §8 P7 left the migration of
  `step_graph` into the workflow model to this ANA.

Method: six readers worked in parallel (orchestrator, store, MCP, TUI, documents, web prior art), and
each returned facts with `path:line` evidence. This document re-read the load-bearing ones:

- `status.rs:316`, `gate.rs:807-823`, `engine.rs:2676-2695`, `engine.rs:3183-3221`, `engine.rs:4081-4141`;
- `0001_init.sql:214-260, 447-495`;
- `docs/ANA-2.md:100-139, 1032-1046`;
- `docs/REQUIREMENTS.md:365-424`.

Citations are against `41bf5c90` on `hr/ANA-33`. The web rows in §4 are summaries fetched by the
survey reader, so treat them as indicative.

## 2. Surface as read

### 2.1 Graph model and storage

- **Graph and phase tables.** `step_graph(id, project_id NOT NULL, name, description, UNIQUE (project_id,
  name))` is at `0001_init.sql:214-222`. `0003_orchestration.sql:9-10` adds `is_override`.
  `step_graph_phase` (`0001_init.sql:228-248`) holds:
  - the phase columns `R-ORCH-1` enumerates;
  - `UNIQUE (graph_id, position)` and `UNIQUE (graph_id, name)`;
  - `judge_agent_id`, `judge_model` and `deadline_seconds`, added by `0003:17-22`;
  - `persona_id`, added by `0012`.

  `phase_agent(phase_id, position, agent_id, model)` holds the candidates and is insert-only
  (`0001_init.sql:254-260`).
- **No topology storage.** There is no edge, condition, loop target, parent, subgraph or owner other
  than a project. Order is `position` alone.
- **Ownership.** Nothing is owned by a workspace. The tables that reference one are `workspace`,
  `workspace_project` and `workspace_box_path`. Graphs, kinds, templates, skill bindings and documents
  key on a project or an item.
- **Editing seam.** `WriteStore` has create, update and list methods for graphs, phases and agents
  (`htui-core/src/store/traits.rs:890-952`). It has **no deleter**, and every row is CAS'd on its own
  `updated_at`, so there is no atomic multi-row edit. `graph.rs:455-461` already notes that re-override
  "needs a phase deleter `WriteStore` does not carry".
- **Seeded graphs.** `KINDS` in `htui-core/src/seed.rs:54` holds the seeded graphs, all linear. The
  `implement` phase lists `review` in its `input_kinds` as the back-edge's input.

### 2.2 The snapshot (R-ORCH-11)

- **Shape.** `GraphSnapshot { v, graph, topology, mode, phases, settings, scope, personas }`
  (`htui-core/src/model/run.rs:476-500`), with `V = 1` (`:504`). `phases` is a flat
  `Vec<SnapshotPhase>` in position order (`:546-590`).
- **Built by `graph::resolve`.** It renumbers phases to dense `0..n-1`, checks caps
  (`check_fan_out_caps`, `graph.rs:641-671`), and computes `topology` as sha256 over the serialised
  `phases[]` (`graph.rs:274-296`). Any new `SnapshotPhase` field changes every digest (`graph.rs:271-272`).
- **Readers.** `command::snapshot_of` refuses `v != 1` (`command.rs:1003-1024`).
- **Resume.** Resume re-resolves the **live** graph and parks the run on a topology mismatch
  (`engine.rs:3183-3221`), which is ANA-2 invariant 2 (`docs/ANA-2.md:109-113`).
- **Storage.** The snapshot is mirrored as `run.graph_snapshot TEXT` (`cache_migrations/0001_mirror.sql:106`).
  It is the only offline route to a graph.

### 2.3 The walk, gates and the loop

- **One cursor per run.** `status::cursor` walks `snapshot.phases` in order, and "the first position
  whose latest step is not `done` decides" (`status.rs:316-349`). At most one position is live, so
  execution is strictly sequential. Only a fan-out group runs sessions concurrently.
- **Required inputs.** `required_inputs` treats an input kind as optional only if a *later* position
  produces it (`engine.rs:7401-7412`). That is the linear trick that makes `review` legal as an
  `implement` input on attempt 1.
- **Retry.** A retry stays at the same position (`may_attempt`, `status.rs:30-32`).
- **Review loop.** The loop is name-keyed. `loop_target` is "the greatest position below `review`
  named `implement`, else the one before it" (`gate.rs:807-823`). `retire` covers `from..=to`
  positionally (`gate.rs:971-982`). No phase declares a target.
- **Hard gates.** `gate_hard` survives the auto-mode downgrade at snapshot time (`effective_gate`,
  `graph.rs:753-759`).

### 2.4 Fan-out today (rival, ANA-2 §4.5)

- **Rows.** Candidates are `run_step` rows at one `(position, attempt)` with `fanout_index`
  `0..fan_out`. The judge is `-1`. "No CHECK on fanout_index, ever" (`0003:64-65`).
- **One prompt for the group.** `drive_group` assembles **one prompt from the first pending candidate**
  and runs every candidate on it under `join_all` (`engine.rs:4081-4141`). All candidates start from one
  `group_base` per repo, passed through `FanoutSlot { index, width, base }` (`isolate.rs:103-110`).
- **Selection.** Selection keeps one result: `select_fanout` marks one row `selected = true` and the
  rest superseded. Inputs exclude losers with `s.selected IS NOT FALSE` (invariant 8,
  `docs/ANA-2.md:135-138`).
- **Retry and agents.** `RetryStep` on a member retries the whole group (`command.rs:1056-1057`).
  `AgentSelector` is asked once per `fanout_index`, and its only implementation is `FirstCandidate`
  (`engine.rs:149-183`).
- **Caps.** Caps are checked once, at resolve: `max_fan_out` (4) and `max_agents_per_run` (8), the
  latter as `Σ fan_out` plus one per judged phase, with retries uncounted (`graph.rs:35-38, 641-671`).
  ANA-2 §4.5's prose says retries count. The code does not.

### 2.5 Isolation and reconciliation (ANA-2 §4.6 as built)

- **Trees.** Every step gets a tree per repo (`run_step_tree`). A worktree branches `htui/<step_id>` from
  "the primary repo's current HEAD at step start".
- **Reconcile.** `Isolator::reconcile` takes **one winner** (`isolate.rs:237`). `Cli::merge_no_ff`
  reads the conflicted paths, runs `merge --abort`, and returns `merge_conflict: <paths>`
  (`isolate/git.rs:712-796`).
- **Conflicts park the run.** `reconcile_done_step` parks the run on that error (`engine.rs:5565-5642`).
  §4.6 step 3 states the rule: "The orchestrator never resolves a conflict ... a trade `R-ID-6` does not
  permit here" (`docs/ANA-2.md:1039-1043`).
- **Invariant 3.** "The judge is the single exception `R-ORCH-7` grants" to "no agent in a bookkeeping
  path" (`docs/ANA-2.md:114-118`).

### 2.6 Crash recovery (ANA-2 §4.9 as built)

- **Order of the sweep.** `recover_run` first re-reconciles a done winner through `recover::frontier`,
  which looks for "the highest position p ... provided every row at a position > p is retired"
  (`recover.rs:365-376`). It then classifies each `running` row as `Plain`, `Candidate` or `Judge`
  (`recover.rs:186-278`).
- **Candidates.** An interrupted **candidate is not retried**: "D95: no reset and no park; the group
  routes over its survivors" (`engine.rs:2688-2694`).
- **Plain steps.** A plain step follows the artefact test: `after_hash` for every repo plus the output
  document means done; a resettable tree means reset and retry; a dirty `local` or `shared_serialized`
  tree means park.

### 2.7 The MCP surface

- **Tools.** Eight tools are dispatched (`htui-mcp/src/tools/mod.rs:45-54, 81-88`). There is no plan,
  graph, part or `spawn_subagent` tool.
- **Scope.** `ToolScope` comes from the session token, never from arguments. It carries one run, one
  step and a `StepFence`, but **no step role** (`htui-orch/src/tools.rs:22-49`). Every item write is
  fenced. `create_step` is on `WorkerStore` (`worker.rs:263-264`, delegating to
  `WriteStore::create_step`, `traits.rs:1377-1383`) but takes no fence.
- **No live channel.** A session cannot call back into the engine. The only in-session round trip is
  `prompt_bridge`, which is permission-shaped and CLI-only.
- **Judge precedent.** The judge writes a `judge` document, and deterministic code parses the last fenced
  JSON block after the session ends (`fanout.rs:120-142`).

### 2.8 The TUI

- **Settings editor.** `Settings > Kinds` is a list tree, project → kind → phases
  (`htui/src/ui/tabs/settings/kinds.rs:1-24, 136-169`). It edits six phase columns plus the persona
  (`kinds.rs:1766-1777`). `fan_out`, `gate`, `isolation`, `command_queue`, `verify_command`,
  `retry_limit`, `output_kind` and `template_version` are shown read-only, dimmed, ending "MOD-4 owns
  these" (`kinds.rs:129, 2013-2034`). It cannot delete a graph or phase (`kinds.rs:120-122`) and has no
  `phase_agent` editor.
- **Runs flow view.** The flow view lays out one layer per `(position, attempt)`, with a judge layer and
  winner-only edges (`runs/execution_graph.rs:86-178`). `s` sends `SelectFanout`. `RunSummary` carries
  no snapshot (`run.rs:865-896`).
- **What the redesign decided** (F §4.6, §5.2; MOD-120, MOD-124):
  - "a fan-out is a step property (its parts), not a link";
  - "workflows are templates; runs are their actuation";
  - "runs already going keep the version they started with";
  - the run graph shows `prepare`, `pick workflow`, rounds, parts and `collect`.

  The spike's `WfLink` has no condition payload, and `actuate` is linear
  (`spikes/ui-skeleton/src/data.rs:85-139`, `flow.rs:315-378`).

### 2.9 Conflicts among the sources, resolved

1. **ANA-2 §4.1 rejected edges and per-task rows.** Both reasons are withdrawn by the amended
   `R-ORCH-1` and `R-ORCH-7`. The rejection is superseded here, and ANA-2's Open call 1 is answered by the
   review: divide is the must.
2. **ANA-2 invariant 2 ("a resumed run refuses ... topology hash changed") against MOD-124 and F ("runs
   keep the version they started with").** Today an edited graph parks every run in flight on resume. A
   graph planned at run time would park on every resume, because it has no live counterpart. Resolved in
   7.2: a v2 run is pinned to its snapshot.
3. **ANA-2 §4.5 counts retries in `max_agents_per_run`; the code does not.** The code is the rule
   (7.8), and the prose is corrected.
4. **§4.6 "never resolves" against `R-ORCH-7` "resolving conflicts".** Both hold once the resolver is an
   agent working in its own tree as item work, and the orchestrator still never resolves one (7.5).
5. **`R-TUI-8` (a graph editor in Settings) against `R-TUI-7` and MOD-124 (the workflow editor in the
   Toolbox).** Resolved with one model and one component (7.10).
6. **ANA-13 §3.3 (`RunKind::Swarm`, "a single shared context window"), ANA-27's typed exits in MOD-27,
   and the new planner.** Three answers to "an agent chooses what runs next". Resolved in 7.12.
7. **ANA-40 §2.6 and §11 cite `HANDOFF.md:111-139` for ANA-33 and ANA-34.** That range is stale; the
   items are now at `:105-133`.

## 3. What is open

1. Storage for edges, conditions, loops, parallel phases, joins, owners and subgraphs, and whether
   workflows and kinds' graphs are one model (ANA-40 P7).
2. What a run freezes when part of its graph is planned during the run, and how resume treats it.
3. The planner's tools: where validation happens, and what bounds it checks.
4. Where a planner, a part and an integrate step sit in `run_step`, given that every reader of
   `fanout_index >= 0` assumes rivals.
5. How recovery treats a planner that crashed mid-plan, an unfinished part, and an integrate step
   crashed mid-merge.
6. Who merges the parts, and how that squares with §4.6 and invariant 3.
7. One editor or two, and which phase fields become editable.
8. The fate of MOD-27, MOD-36, ANA-25 and MOD-98, and the homes the neighbours wait for:
   - MOD-126's `pick workflow` step;
   - ANA-42's `prepare` and `collect`;
   - MOD-111's step control;
   - MOD-125's resume.

## 4. Prior art

| System | Definition per run | Dynamic fan-out recorded as | Recovery | Caps | Integration |
|---|---|---|---|---|---|
| Claude Code Workflow / ultracode | script written to a file per run; nondeterminism throws | agents journalled in start order | replay by ordinal and prompt: a failed agent reruns it and every later one | 16 concurrent, 4,096 per call, 1,000 per run (hard); size warning advisory | none built in; worktree branches left for a later agent |
| LangGraph | code; checkpoints only | `Send` packets inside a checkpointed superstep | completed branch writes kept; failing branches retried | `recursion_limit`, `max_concurrency` | reducer node; a skipped branch silently blocks a list-form join |
| Temporal | Pinned worker version | `StartChildWorkflowExecution` in the parent's history before the child runs | deterministic replay of history | guidance of at most 1,000 children per parent | parent awaits children; close policy explicit |
| Argo | template copied into `status.storedWorkflowTemplateSpec` | `withParam` JSON output | retry in place; fan-out plus retry is a known bug class (#7617, #17134, PR #16938) | `parallelism` | fan-in task |
| GitHub Actions and merge queue | commit SHA; re-runs keep it and the setup job's matrix | matrix JSON from a setup job | re-run failed jobs on the frozen matrix | `max-parallel`, 256 jobs | merge queue applies PRs in order onto base + the ones ahead |
| Airflow 3 | `bundle_version` per DagRun; rerun asks latest or original | mapped task rows `(dag_run, task_id, map_index)` + `try_number` | per-instance retry | `max_map_length` | downstream task reads the mapped list |
| Magentic-One, OpenAI handoffs, CrewAI | plan in the manager's context | none durable | none | none | the manager |
| Devin, OpenHands, Factory, Cursor | per session | coordinator splits into slices with schemas | per child | small teams (3-10) | coordinator or an integration branch; Factory serialises writers; Cursor keeps best-of-N a separate command |

Five lessons bind this design:

1. **Freeze the definition by copying it into the run** (Argo, Airflow, GitHub), and never resolve a run
   through a live reference.
2. **Record a dynamic plan as data before any child starts** (Temporal's await-spawn, Argo's `withParam`,
   Airflow's mapped rows). Recovery reads the record and never re-asks the planner.
3. **Key children by a stable identity, so one failed child reruns alone** (Airflow's `map_index`,
   Temporal's workflow id). Claude Code's ordinal-plus-prompt cache is too coarse for side-effecting
   coding parts.
4. **The runtime enforces the caps and refuses loudly.** The planner's own size is advice under them.
5. **Integration of divided code work is a named stage with explicit ownership per part.** It is done by
   an agent or a person on an integration branch, never by the engine silently. Applying parts in a fixed
   order, merge-queue style, makes the deterministic part deterministic.

## 5. Constraints

1. **`R-ID-6` and ANA-2 invariant 3.** Agents propose and code records. Plan admission, branch
   predicates, status moves, gate application and the merge into the primary tree stay deterministic.
2. **`R-ORCH-11` and invariant 2.** A run walks only its frozen record. Anything planned is appended
   beside it and never edits it.
3. **Invariant 9.** Resume is decided from rows, never from memory, so a plan is a row before any part
   runs.
4. **`R-MCP-1` and MOD-11's I-1 to I-8.** Tool scope comes from the token, writes are fenced and
   run-scoped, and text is scrubbed fail-closed.
5. **`R-ID-4`.** Trees live under the scratch root, and nothing is written into a managed repository.
6. **Hard gates are never downgraded, removed or bypassed.** This holds by a plan as much as by auto mode.
7. **`R-STO-5`: migrations are forward-only.** Every migration rebuilds every box's mirror
   (`pg/mod.rs:724` against `cache_meta.schema_version`, `cache/mod.rs:141-145`). `v = 1` snapshots of runs in flight, parked runs and adopted runs must keep walking.
8. **Compete fan-out keeps working (`R-LATER-13`).** `fanout_index`, the `-1` judge and `select_fanout`
   keep their meaning, and "no CHECK on `fanout_index`, ever" survives.
9. **`R-AGT-5`.** Adding an agent changes no orchestrator or prompt code, so no code path is keyed on
   an agent name. Parts get agents through `AgentSelector`.
10. **`R-TUI-10` and the every-key-configurable rule.** Every editor key is a catalogue action.

## 6. Options

### 6.1 Graph model and storage

| Option | Verdict | Reason |
|---|---|---|
| A. Keep the list and add columns (a loop target, a parallel rank) | Rejected | It cannot express `if`, `asks you`, joins or planned nodes. Ranks collide with `UNIQUE (graph_id, position)`. |
| B. Phase rows as nodes plus a `step_graph_edge` table, an owner scope, a graph `revision`, and an atomic replace writer | **Adopted** | It keeps every FK that made ANA-2 keep rows: `persona_id`, `phase_agent.agent_id`, `judge_agent_id`, and `skill_binding.phase_id`, which keys on the phase row. A linear graph is a graph with `next` edges, so today's data migrates by a deterministic backfill. |
| C. Append-only JSON definitions (`step_graph_version`, as `prompt_template` does) | Rejected | Atomic edits and history come free, but the FKs above are lost and `skill_binding` has nothing to key on. History is not a requirement, since the run snapshot already freezes what ran. It can be reopened if edit history is wanted. |
| D. A separate `workflow` table family beside `step_graph` | Rejected | That means two models, two resolvers, two editors and two snapshot paths for one concept. ANA-40 C already calls a kind's graph "a degenerate workflow". |

Under B, the authored relations of `R-ORCH-1` map onto stored state as follows:

| Template relation (F §4.6) | Stored as | Taken when |
|---|---|---|
| `next` | edge `next` | the source is done (after its gate, if any) |
| `approved` | edge `next` from a node whose `gate` is not `never` | the source's gate is approved. The relation picker sets the source gate, so `R-ORCH-2` keeps one home. |
| `asks you` | edge `asks_you` | a person takes it. It is never downgraded in auto mode; declining it skips the target. |
| `if` | edge `if` with a predicate (7.3) | the source is done and the predicate holds |
| `changes requested` (loop) | edge `changes_requested` with a `limit` | the source's verdict is `Rejected` |
| `fails` (loop) | edge `fails` with a `limit` | the source settles failed |
| `retries` (loop) | node `retry_limit`, drawn as a self-loop | the same node runs again |

### 6.2 What a run freezes, and planned graphs

| Option | Verdict | Reason |
|---|---|---|
| A. Rewrite `run.graph_snapshot` when a plan is admitted | Rejected | It breaks `R-ORCH-11` immutability and churns the digest, so the record no longer says what the run started with. |
| B. A v2 snapshot freezes the graph and its bounds; each admitted plan is an append-only `run_plan` row with its own digest, and planned nodes take positions appended after the frozen ones | **Adopted** | This is the Argo, Temporal and Airflow shape. The frozen part and every revision stay addressable and replayable. |
| C. The plan as an item `document` (`kind = plan`) | Rejected as storage | Documents are versioned per item and kind, not per run and revision, and readers resolve "latest per kind". The plan is still *shown* to the person, as the approval view in 7.6. |

Resume:
- **A, keep the live-topology comparison for v2.** Rejected: it parks every planned run and contradicts
  "runs keep the version they started with".
- **B, pin a v2 run to its snapshot.** **Adopted.** Integrity is checked by recomputing the snapshot's
  own digest and each plan's digest from the rows.

### 6.3 The planner's tools

| Option | Verdict | Reason |
|---|---|---|
| A. The tool inserts `run_step` rows (`create_step`) | Rejected | It makes the MCP host a second writer of `run_step` beside the walk, and it is unfenced, which breaks MOD-11 I-3. |
| B. The planner writes a structured document that the engine parses after the session (the judge pattern) | Rejected as the sole path | It is deterministic, but the planner learns of a cap breach only after its session ends, so every refusal costs a full retry. |
| C. `plan_submit` validates against the frozen bounds in-session and writes a fenced **proposed** `run_plan` revision; after the session the engine re-validates the latest proposal and **admits** it in one transaction | **Adopted** | The agent gets immediate, typed refusals and the code still records. It mirrors `command_run`'s double check of `lease_holds`. |
| D. Turn-by-turn spawn and handoff tools (Swarm, OpenAI handoffs) | Rejected | The plan lives in a context window and has no recovery story. |

### 6.4 A part on `run_step`

| Option | Verdict | Reason |
|---|---|---|
| A. A part is a `fanout_index >= 0` row at the divided node's position, plus a role column | Rejected | It fits the key, but every reader of `fanout_index >= 0` assumes rivals: `group_at`, `winner_at`, `group_cursor`'s `Select`, `select_fanout`, `recover::frontier`, the Candidate recovery arm, `selected IS NOT FALSE`, and `pg/relay.rs:787`. A retry is group-wide (`command.rs:1056`), so a per-part retry collides with the shared `attempt`. It also cannot express a plan whose nodes have their own successors. |
| B. A part is a **planned node** at its own appended position: `fanout_index = 0`, a per-part `attempt`, plus `role`, `parent_step_id` and `node_key` columns | **Adopted** | The key is unchanged, and each part retries alone (Airflow's `map_index` plus `try_number`). Parts reuse the same machinery as dynamic graphs. `fanout_index` keeps its compete-only meaning. |
| C. A child table `run_part` | Rejected | Every satellite table keys on `run_step_id` (trees, commits, events, permissions, commands, follow-ups, documents) and would need duplicating. |

### 6.5 Crash recovery of the new roles

| Option | Verdict | Reason |
|---|---|---|
| A. Treat parts as candidates ("route over survivors") | Rejected | A lost part is lost work, and the rule would drop it silently. |
| B. Each part is a plain step under §4.9's artefact test. A planner is done if and only if its plan was admitted, since admission is the journal event. Integrate is a plain step plus an ancestry post-check. | **Adopted** | It reuses §4.9 unchanged per row and never re-asks a planner once its plan is admitted. |
| C. Replay by prompt digest (Claude Code) | Rejected | ANA-2 §4.9 already rejected it, and it reruns every later sibling. |

### 6.6 The integrate step and §4.6

| Option | Verdict | Reason |
|---|---|---|
| A. The orchestrator merges every part into the primary tree and parks on a conflict; no integrate agent | Rejected | `R-ORCH-7` requires an integrate *agent* that merges branches *and artefacts* and resolves conflicts. |
| B. The integrate agent does all the merging in its own tree, and only its branch reconciles | Partly adopted | It keeps §4.6 intact, but a clean case still costs agent git work, and no row records which parts merged cleanly. |
| C. B plus a **deterministic pre-merge** of the parts in plan order inside the integrate tree, stopping before the first conflict, and a **post-check** that every part's commits are ancestors of the integrate result | **Adopted** | Mechanical merging is done by code, judgement by the agent, and inclusion is proved by code (merge-queue order, Argo #7617's "one terminal attempt per part"). |
| D. The coordinator (ANA-34) integrates the parts | Rejected | `R-ORCH-14`'s coordinator "does no item work". Parts are one run's work. |

### 6.7 The editor

| Option | Verdict | Reason |
|---|---|---|
| A. Extend the list editor in `Settings > Kinds` | Rejected | A list cannot show branches, joins or loops. |
| B. Two editors, Settings for kinds' graphs and the Toolbox for workflows | Rejected | That is two components over one model. |
| C. One rataflow graph-editor component over the one model. Settings opens a project's graphs and picks each kind's default; the Toolbox lists the global and workspace library. One validator is shared with `resolve`. | **Adopted** | It meets `R-TUI-7` and `R-TUI-8` with one piece of code. |

### 6.8 MOD-27, MOD-36, ANA-25

- **Swarm.**
  - A, keep `RunKind::Swarm` (ANA-13 §3.3's proposed kind, never built: `RunKind` is `graph` and
    `chat`, `htui-core/src/model/mod.rs:297`): rejected, because a third run kind duplicates planned
    graphs.
  - B, **subsume it into planned graphs**: **adopted**. An agent choosing its successor is a
    `successors`-scope plan revision (7.6), bounded by `max_plan_revisions`, which is the hop cap.
- **`spawn_subagent`.**
  - A, drop it: rejected, because `R-MCP-2` lists it.
  - B, **keep consult mode only**: **adopted**. This is ANA-27's read-only child, which has no tree,
    returns its answer as the tool result, and is its own `run_step` (`role = consult`) with
    `parent_step_id`.
  - C, a tree-writing child: subsumed, since that is a part.
- **MOD-36 and ANA-25.** Both stay later tier and unchanged. They depend only on compete groups and
  judge verdicts, which divide mode never produces.

### 6.9 One graph, several items (subgraphs)

| Option | Verdict | Reason |
|---|---|---|
| A. Inline the subgraph in one run with a per-step `item_id` | Rejected | It breaks one item per run, which overlap, the lease, close-out and `R-ORCH-11`'s record all assume. |
| B. **Child runs:** a subgraph node creates one run per item, with `parent_run_id` and `parent_step_id`, each run getting a copy of the subgraph snapshot frozen in the parent | **Adopted** | Every child is an ordinary run, and the parent waits deterministically. |
| C. Leave it to the coordinator or ANA-41 | Rejected | `R-ORCH-1` puts it in the graph. Integrating a child's work is still the coordinator's (7.13). |

## 7. Verdict

**7.1 One graph model (6.1 B, answering ANA-40 P7).**

- **Nodes and edges.** `step_graph_phase` rows stay the nodes. A new
  `step_graph_edge(id, graph_id, from_phase_id, to_phase_id, kind, predicate JSONB, limit)` holds the
  links, with `kind IN ('next','if','asks_you','changes_requested','fails')`.
  *Amended by MOD-137 (2026-10-21):* the column is `loop_limit`, since `limit` is reserved in
  Postgres; the migration is `0020`, since MOD-125 took `0019`.
- **Owner scope.** `step_graph` gains a nullable `workspace_id`, and `project_id` becomes nullable,
  under a CHECK that at most one is set. Neither set means the **global workflow library**. Uniqueness
  becomes one partial unique index per scope.
- **Workflows are `step_graph` rows.** ANA-40's global workflows are `step_graph` rows in the global
  scope, and a kind's default graph is a project-scoped row. There is no second table.
- **Who may use which graph.** An item may use a graph of its project, of its workspace, or of the
  library. `item_kind.default_graph_id` keeps its FK and may name any of them.
- **Templates and bindings.** A global or workspace graph's `template_name` resolves at snapshot time in
  the run's project (`R-PRM-4`'s library plus override). `skill_binding` keeps keying on the phase row
  with the binding's own `project_id`.
- **Revisions and edits.** `step_graph.revision` is bumped by every edit and is the whole-graph CAS
  token. A `replace_graph` writer applies an editor's nodes and edges in one transaction, and new
  deleters cover phases, edges and agents. This also pays the re-override debt at `graph.rs:455-461`.
- **New node properties:**
  - `split IN ('none','compete','divide')`. `compete` is today's `fan_out > 1`. `divide` has planner
    and integrate persona and agent columns, plus optional default parts (`phase_part` rows).
  - `palette BOOLEAN`, for a node a planner may instantiate but the walk never reaches on its own.
  - `plan_scope` on a planner node.
- **Validation.** Forward edges must be acyclic, and a back edge is only `changes_requested`, `fails`
  or `asks_you`. The findings' own `asks you` example is a back edge (`merge → code` in research
  fan-out, `docs/ui-redesign-findings-2026-10-15.md:389-390`): a person picks whether the loop is
  taken, and the edge carries a `limit` like the other loop links, which still bounds it (7.3). A
  palette node has no edges.
- **Backfill.** The migration derives `next` edges between consecutive positions. For each `review`
  phase it adds the `changes_requested` edge today's rule implies: to the greatest earlier `implement`,
  else to the previous phase, with `limit` taken from the target's `retry_limit`. That makes
  `R-ORCH-3`'s default explicit data.
- **What `position` means now.** It stays as a display and order hint, and `UNIQUE (graph_id,
  position)` is kept. `UNIQUE (graph_id, name)` is kept too, because the name is the node key.

**7.2 Snapshot v2, and the run is pinned (6.2 B).** Design notation, not code:

```
GraphSnapshot v2 {
  v: 2,
  graph:    { id, name, scope: project|workspace|global, owner_id, revision, is_override },
  source:   { how: pin|pick|kind_default, pick_step: Option<StepId> },          // MOD-126
  topology: "sha256:…" over canonical { nodes, edges },
  mode,
  nodes:    [SnapshotNode { position, key, role: step|planner|integrate|subgraph,
                            palette, split, …every SnapshotPhase v1 field…,
                            divide: Option<{ planner, integrate, max_parts, default_parts }>,
                            plan_scope: Option<parts|successors|graph>,
                            subgraph: Option<{ snapshot: GraphSnapshot, items: ItemSource }> }],
  edges:    [SnapshotEdge { from, to, kind, predicate, limit }],
  bounds:   SnapshotSettings v1 + { max_plan_revisions, max_parallel_sessions,
                                    runtime_changes: off|ask|automatic, max_child_runs,
                                    agents: [{ name, id, models }] },
  scope, personas
}
```

- **Expansion.** A `divide` node expands at resolve into two frozen nodes, `<name>:plan` (role
  `planner`) and `<name>` (role `integrate`), joined by a synthetic edge. Its parts come later from the
  plan.
- **Plans.** Each plan revision is a `run_plan` row: `(id, run_id, revision, planner_step_id, scope,
  status IN ('proposed','admitted','refused','abandoned'), base_position, base_refs JSONB, body JSONB,
  digest, created_at)`, with `UNIQUE (run_id, revision)`. It is append-only and mirrored.
- **Positions.** Planned nodes take positions `base_position..` after every frozen node and every
  earlier revision. So `position` stays "the node index in the run graph", and the `run_step` key keeps
  working unchanged.
- **Resume.** A v2 run never compares itself against the live graph. Integrity means the snapshot's
  `topology` and each admitted plan's `digest` recompute from their own content. "Re-run on this
  version" and "re-run on the latest" are two distinct verbs of a new run (Airflow, GitHub).
- **v1 compatibility.** v1 stays readable forever. `snapshot_of` accepts `v` of 1 or 2 and lifts a v1
  snapshot into the v2 model: linear `next` edges, the name-rule `changes_requested` edge, and compete
  fan-out as is. v1 runs keep today's live-topology park, so their behaviour does not change mid-flight.
  New runs write only v2. An older binary refuses v2, and the migration bump already gates headless
  workers.

**7.3 The walk becomes a readiness set.**

- **The ready set.** `status::cursor` is replaced by a pure `ready(snapshot, plans, steps) ->
  Vec<Cursor>`, and `run_to_rest` drives every ready node concurrently under the run's lease, up to
  `max_parallel_sessions`. A linear graph yields one cursor at a time, which is today's behaviour.
- **Joins and skips.** A node becomes ready when every forward in-edge is *resolved*: its source is done
  or skipped, and the edge's condition has been decided. At least one of those edges must be *taken*.
  A node whose resolved in-edges are all not taken is **skipped**. That is derived and has no row, and
  it propagates. This avoids LangGraph's silently blocked join. v1 has only "all" joins.
- **`if` predicates.** These are a closed, deterministic set evaluated over the source step's
  artefacts: `all_changed_paths_match` and `any_changed_path_matches` (globs in §4.7's `repo:glob`
  form), `verify` (passed or failed), and `item_kind` and `item_tag`. They combine with `all`, `any` and
  `not`. Each source's `next` edges are always taken. A branch chosen by an agent goes through a
  planner, never through `if`.
- **`asks_you`.** The run parks at `awaiting_approval` on the edge: its source is done and its target
  is not yet created. Take creates the target, and decline skips it. It is never downgraded. On an
  `asks_you` back edge, take is a loop (below) within the edge's `limit`; once the limit is spent the
  run escalates as for any loop instead of asking again.
- **Loops.** `changes_requested` replaces `is_review`'s name rule: any node with such an edge has its
  verdict read the way `gate.rs` reads `review` today. On a loop from S to T, the walker retires every
  node on a forward path from T to S. That includes their parts and plans: plans become `abandoned`,
  and parts are superseded. T is re-created at `attempt + 1`.
  - **Budgets.** The budget is the edge's `limit`. The no-progress test (`commits_are_identical`,
    `reviews_are_identical`) applies to the region. After that, the run escalates as today.
  - **Retry versus loop.** A node with a `fails` edge loops on failure. A node without one retries
    itself up to `retry_limit`, which is today's behaviour.
- **Required inputs.** `required_inputs` becomes "optional if a node reachable *from* this one produces
  it", which is the same trick in graph form.
- **Parallel nodes reconcile on their own.** Each reconciles on done through §4.6 as today, serialised
  per repo by a reconcile lock. A conflict parks the run. An author who wants an agent to merge
  parallel work uses a divided node. `local` isolation on two nodes that can be live together is
  refused at resolve, as `LocalFanOut` is today.

**7.4 A divided step: planner, parts, integrate (6.4 B).**

- **Planner.** The planner node is position p in the frozen list, with `role = planner`.
  - Like the judge, it has no tree of its own (the judge runs in a scratch `cwd` with no
    `run_step_tree` row, `engine.rs:5341`). Unlike the judge, it reads the run's primary checkout on its
    target at its current HEAD, with edit tool kinds denied through `ToolExposure.deny_kinds`
    (`htui-agent/src/driver.rs:226`).
  - Its output is a plan (7.6), and its gate (`R-ORCH-2`) is the plan's approval: the planned parts are
    shown before they start (Claude Code, Devin). The gate defaults to `always`, and auto mode downgrades
    it unless it is hard.
  - With no planner agent configured and with `default_parts` set, the engine admits the default parts
    as revision 0 itself. The spike's static `(part, agent)` pairs become exactly that.
- **Admission is one transaction.** It sets the plan to `admitted`, records `base_refs` (the primary
  HEAD per repo at admission), inserts every part row as `pending`, and moves the planner step to
  `done`.
- **Part rows.** Each part row has:
  - `position = base_position + i`, `attempt = 1`, `fanout_index = 0`;
  - `role = 'part'`, `parent_step_id` = the planner step, and `node_key = '<divided>/<part_key>'`.

  New `run_step` columns: `role TEXT NOT NULL DEFAULT 'step' CHECK (role IN
  ('step','planner','part','integrate','subgraph','consult','stage'))`, `parent_step_id UUID NULL
  REFERENCES run_step`, and `node_key TEXT NULL`. All three are written once, never updated, and
  mirrored. Compete rows stay `role = 'step'` and keep their `fanout_index` meaning, so no backfill is
  needed.
- **How a part runs.**
  - Every part branches from the plan's `base_refs`, not from "HEAD at step start". That generalises
    `FanoutSlot.base` to a part slot.
  - Each part has its own prompt: the divided node's template rendered with a `{{part}}` section (key,
    title, brief, owned paths, acceptance).
  - Each part has its own agent: the plan's named agent if it is in the frozen `bounds.agents` and in the
    node's candidates, else `AgentSelector`, which is `FirstCandidate` today and MOD-36's weighted
    selector later.
  - Isolation is the divided node's (`R-ORCH-8`, per part). `shared_serialized` runs parts one at a
    time, and `local` with more than one part is refused.
- **Parts never reconcile into the primary tree.**
- **Part documents.** Generic `input_kinds` resolution excludes documents produced by `role = 'part'`
  steps. That extends invariant 8: part documents are visible only to their integrate node, through a
  `{{parts}}` section, and to a re-planning planner. N parts writing one kind therefore never shadow each
  other downstream.
- **Retries.** A failed part retries alone, at `attempt + 1` at its own position, within the divided
  node's `retry_limit`. A part exhausted fails the divided step, which follows its `fails` edge or
  escalates. A missing part is incomplete work, never "routing over survivors".
- **`command_run` exposure.** `R-MCP-3`'s `fan_out_only` now means "exposed when the session has live
  siblings in the run": compete candidates, divided parts and parallel nodes.

**7.5 Integrate and §4.6 (6.6 C).**

- **Preparing the integrate tree.** The integrate node becomes ready when every part of the admitted
  revision is `done`. Its tree branches from the same `base_refs`. For `copy` mode, `prepare` first
  fetches each part's `htui/<part_step_id>` into the copy. Worktrees share the object store already.
- **Pre-merge.** Before the session, a new `Isolator` verb `premerge(tree, parts)` applies the parts in
  plan order with the existing `merge_no_ff`. It stops before the first conflicting part, aborts that
  merge, and returns a report: merged cleanly, conflicted (the part and its paths), not attempted. The
  report is stored on the step and given to the agent.
- **The agent's job.** It completes the remaining merges, resolving conflicts, and reconciles the parts'
  documents into the divided node's `output_kind`.
- **Post-check.** After the session, deterministic code checks per repo that every done part's
  `after_hash` is an ancestor of the integrate `after_hash` (`git merge-base --is-ancestor`); a part with
  no commits is exempt. Then `verify_command` runs. A failure is `integrate_incomplete: <part keys>`.
- **Landing.** Only the integrate branch then lands, through §4.6's **unchanged** single-branch
  `reconcile`, which still parks on `merge_conflict` against a primary tree that moved.
- **Why this keeps §4.6.** A conflict resolved inside a step's own tree is item work done by an agent
  (`R-ID-6`, amended). The *orchestrator* still never resolves a conflict.
- **Text amendments to ANA-2.** §4.6 step 3 gains "an integrate step, an agent, may resolve conflicts
  between parts inside its own tree". Invariant 3's "the judge is the single exception" gains "the
  planner's plan and the integrate agent's merge are item work, admitted and checked by code, not
  exceptions".
- **Review.** `review` follows integrate and stays single (`ReviewFanOut` is kept).

**7.6 The planner's MCP tools (6.3 C).**

- **Three new tools.** Each is one file under `crates/htui-mcp/src/tools/`, with an `ALL` entry and a
  `dispatch` arm. `ToolScope` gains `role: StepRole` and `plan_scope: Option<PlanScope>`, set only by
  `open_tools`.
- **The tools.** None of them accepts a run or step id (I-1):

  | Tool | Advertised to | Does |
  |---|---|---|
  | `plan_bounds` (read) | a planner, or any step when `runtime_changes != off` | the frozen bounds: the palette, allowed agents and models, `max_parts`, the remaining agent budget, remaining revisions, hard-gated and `asks_you` nodes that must stay reachable |
  | `plan_submit` (fenced write) | the same | validates the body below against the frozen bounds and writes a `proposed` `run_plan` revision. A later call in the same session supersedes it. Answers `{revision, accepted, refusals[]}` and is never `isError` for a refusal. |
  | `plan_status` (read) | planner, integrate | the run's plans and each part's state, branch, `after_hash`, diffstat and document id |

- **Plan body.** Design notation:
  ```
  Plan { scope: parts|successors|graph,
         parts: [{ key, title, brief, owned_paths: [repo:glob], acceptance?, agent?: {name, model}, persona? }],
         nodes: [{ key, from_palette, brief, after: [key] }],
         into: Option<frozen node key>, notes }
  ```
- **Validation (pure, shared by the tool and the engine):**
  - keys are unique slugs, and `1 <= parts <= max_parts`. An empty plan is a planner failure,
    `empty_plan`, never an empty fan-out (Argo PR #16938);
  - agents and personas come from the bounds;
  - the sessions admitted so far plus the new ones are at most `max_agents_per_run`;
  - revisions are at most `max_plan_revisions`;
  - planned forward edges are acyclic;
  - a `parts` plan contains no planner, so nesting is at most two levels: a graph planner may plan a
    divided node;
  - every hard-gated or `asks_you` frozen node stays reachable and is never superseded.

  Overlapping `owned_paths` between parts (by §4.7's prefix rule) are reported in the approval view, not
  refused. Changes a part makes outside its owned paths are recorded on the part and shown to integrate
  and review.
- **Admission.** After the planner's session ends, the engine re-validates the latest `proposed`
  revision and admits it (7.4). Older proposals become `refused` or `abandoned`. A session that ends with
  no valid proposal fails as `missing_output`.
- **Run-time changes (`R-ORCH-1`, `R-TUI-8`).** `bounds.runtime_changes` is frozen from Settings >
  Workflows (default **`ask`**) and may be overridden per graph. When it is not `off`, any agent step may
  submit a `successors` plan. Such a plan replaces not-yet-created downstream nodes with palette nodes
  that rejoin at `into`. Under `ask`, admission parks the run with the plan shown. Under `automatic`, it
  is admitted at once. A graph planned at run time is a graph whose first node is a planner with
  `plan_scope = graph` over its palette.
- **Permissions.** The tools are pre-approved by `pre_approve`, and none joins `ASKING_TOOLS`.
  Personas may still narrow them. ANA-36 and MOD-112 keep the wider tool survey. These three are the
  "planner's tools" `R-MCP-2` names.

**7.7 Crash recovery (6.5 B, ANA-2 §4.9).**

- **Planner.** A planner with an admitted revision is `done`, because admission is the record. Without
  one, it is reset and retried like a plain step. Proposals from the dead session become `abandoned`.
  The planner is never re-asked once its plan is admitted.
- **Parts.** A part follows the plain-step artefact test. If done, it is done. If its tree is resettable,
  it is reset and that part alone retries. If the tree is a dirty `local` or `shared_serialized` tree,
  the run parks. Siblings are untouched, and part rows are idempotent through the unchanged `UNIQUE` key.
- **Integrate.**
  - Interrupted mid-pre-merge: the tree resets to base and the pre-merge reruns, since it is
    deterministic.
  - Finished: the plain test plus the ancestry post-check.
- **Frontier.** `recover::frontier` becomes the set of done, landing nodes (plain or integrate) whose
  reconcile is not yet recorded. MOD-138's plan settles whether a `reconciled_at` column makes that
  explicit.
- **No twins.** The no-twin rule is the existing lease and fence (`StepFence::Lease`), unchanged.

**7.8 Bounds and caps.**

- **Where bounds come from.** They are the project settings rung (`ProjectSettings`), frozen into
  `bounds`. A graph or a divided node may tighten them, never loosen them.
- **Defaults:**
  - `max_fan_out` (4) is also the parts cap;
  - `max_agents_per_run` (8);
  - `max_plan_revisions` (3);
  - `max_parallel_sessions` (4);
  - `max_child_runs` (8);
  - `runtime_changes` (`ask`).
- **What `max_agents_per_run` counts.** Distinct planned sessions: frozen nodes times `fan_out`, plus
  judges, plus admitted parts and planned nodes, plus consult children. It does not count retries or
  rounds, matching `check_fan_out_caps`. It is checked at resolve and again at every admission. ANA-2
  §4.5's text is corrected to the code.

**7.9 Subgraphs (6.9 B).**

- **Child runs.** A `subgraph` node carries a frozen child snapshot and an item source. The source is
  either the items a planner names (existing items only; drafts through `item_create` need acceptance)
  or the item's children through ANA-35's `parent` link. The node creates one child run per item.
- **Linking.** New `run.parent_run_id` and `run.parent_step_id` columns link a child to its parent, with
  `UNIQUE (parent_step_id, item_id)`. Re-creation after a crash is therefore idempotent.
- **What a child is.** Each child is an ordinary run: its own lease, its own overlap admission through
  `claim_run`, and its own close-out. It is capped by `max_child_runs`.
- **Completion.** The node is `done` when every child is `done`. A failed child fails the node, which
  then follows its edges.
- **Waiting.** While the parent waits only on children, it releases its per-box slot, as invariant 6
  does for a gate. ANA-35, which moves statuses onto the run, names that state (recommended:
  `waiting`).
- **Depth.** Depth is 1 in v1: a child graph holds no subgraph node.

**7.10 The editor and the run display (6.7 C).**

- **One component.** One rataflow graph editor edits every `step_graph`, whatever its scope. MOD-124's
  keys apply.
- **Where it opens.** `Settings > Kinds` keeps the kind-to-default-graph picker and opens the editor on
  the project's graphs. The Toolbox (MOD-124) opens it on the library and the workspace's graphs.
- **The node form.** It exposes every `SnapshotNode` field, which ends "MOD-4 owns these":
  - gate and hard flag, retry limit, isolation, command queue, verify command, deadline, token budget,
    template, persona;
  - the candidate agents (a `phase_agent` editor on the new deleters);
  - split: none, divide (planner and integrate persona and agent, default parts, `max_parts`), or
    compete. Compete is shown only when the later-tier compete setting is on.
  - the palette flag and the plan scope.
- **The edge form.** It has the relation picker, the predicate editor for `if`, and the loop `limit`.
- **Bounds.** They are edited in the project's settings, and the graph header shows and tightens them.
- **Validation and undo.** The validator is `resolve`'s own, run at save, so a graph that saves always
  resolves. Undo and redo stay in the editor's memory. Every applied edit is one `replace_graph` CAS on
  `revision`.
- **Keys.** Every key is a catalogue action under new contexts. The known collisions are `c`, `x` and
  `r`, which differ between Runs and the editor, `Ctrl-Z` against pane zoom, and `F9`. They resolve per
  context, and none is hardcoded.
- **The run display (`R-TUI-4`).** The Runs step list groups a divided node as planner, then its parts
  (key, agent, state), then integrate. It is derived from the mirrored `role`, `parent_step_id` and
  `node_key`, so it works offline. MOD-120's run graph is built from the snapshot's nodes and edges, the
  run's `run_plan` rows and the step rows. `RunSummary` gains the decoded graph and plans.
- **Best-of-N affordances.** The judge layer, the `selected` mark and `s` / `SelectFanout` appear only
  on a compete node.

**7.11 htui's own stages have rows.**

- **What they are.** `pick workflow` (MOD-126), `prepare · <target>` and `collect branch` (ANA-42,
  MOD-132) are `run_step` rows with `role = 'stage'`. Their `node_key` is `pick`, `prepare` or `collect`.
  They sit at the reserved positions -3, -2 and -1, outside the snapshot's nodes. `position` has no
  CHECK, and the key is unchanged.
- **How the pick is recorded.** The pick runs before `create_run`, since its answer chooses the graph to
  snapshot. Its row is inserted in the `create_run` transaction, with agent, model and usage, and its
  record is a `workflow_pick` document. A crash before insert loses only a cheap pick, which is re-run.
  This is the "pick's home in the run graph" MOD-126 waits for.

**7.12 MOD-27, MOD-36, ANA-25, MOD-98 and ANA-30.**

- **MOD-27 is narrowed.**
  - `RunKind::Swarm` is dropped, and ANA-13 §3.3 is superseded. The swarm's typed exits and hop cap
    become a `successors` plan and `max_plan_revisions`.
  - A tree-writing `spawn_subagent` is a part.
  - What stays is **consult-mode `spawn_subagent`**: a read-only child, `role = 'consult'` with
    `parent_step_id`, and a session the MCP host asks the engine to run. That needs the first
    tool-to-engine channel, which is the item's real work. It is counted in `max_agents_per_run`.
  - Recommended: once it exists, graph steps deny the harness's own subagent tool.
- **MOD-36 and ANA-25 stay deferred** under `R-LATER-13`, unchanged. Compete code (`fanout.rs`,
  `select_stage`, `run_judge`, `SelectFanout`) stays and is tested through the v1 lift.
- **MOD-98's open choice is settled.** HANDOFF already lists it as not blocked (`HANDOFF.md:528`) and
  leaves "a seeded disabled phase or a documented recipe" to its plan (`:522-523`); MOD-126, which is
  blocked on ANA-33, amends its wording (`:811`). "Opt-in" means a seeded alternative graph per project (`feature-summary`,
  `bug-summary`) and a library workflow, each with a `summary` node after `review`. It does not mean a
  disabled node: the v2 model has no disabled-node concept, and an unwanted node is simply not in the
  graph. MOD-98 may land on today's linear model, and the edge backfill carries it.
- **ANA-30 amendment.** §4 U5's "lengthens every graph by one step" becomes "adds one node to the graphs
  that opt in; other graphs are unchanged".

**7.13 The boundary with ANA-34 (and ANA-35, ANA-41).**

| Concern | ANA-33 | ANA-34 (coordinator) |
|---|---|---|
| Splitting a step into parts and running them | owns | none |
| Merging parts' branches and documents into one branch inside a run | owns (integrate step, an agent) | none |
| Landing a run's result: merge into the primary tree, push, PR, and conflicts there | keeps §4.6's reconcile as the default landing hook, unchanged | owns; may replace the hook's target (a run branch, a PR) without touching ANA-33's model |
| Close-out and its checklist | unchanged | owns |
| Creating and awaiting a subgraph's child runs | owns (deterministic) | integrates each child's work when it completes (`R-ORCH-14`) |
| The planner's plan | owns; per run, item work | may read it, never edits it |
| Which items run next across a project | none | coordinator, and ANA-41's project workflows |
| What the coordinator changes in `R-ORCH-7` (`R-ORCH-14`: ANA-34 "also settles what it changes in R-ID-6, R-ORCH-7, R-ENT-8 and R-TUI-9", `docs/REQUIREMENTS.md:430-431`) | owns the parts and the integrate step inside a run | owns what the coordinator does with the integrated result (landing, push, PR) |

ANA-35 owns workspace-owned items, item and run statuses (the `blocked` writes of §4.4 and §4.5, and
the waiting state in 7.9), and the `parent` link kind. Workspace-*owned graphs* need only 7.1's owner
column and land before ANA-35.

**7.14 Phasing.** Every phase ships in version one.

1. **The graph model** (MOD-137, MOD-138). Edges, scope and revision, the v2 snapshot, the
   readiness walk, and per-node loop targets. Linear graphs keep working through the backfill and the v1
   lift, and `R-ORCH-3` ships here.
2. **Divided fan-out** (MOD-139, MOD-140, MOD-142). `R-ORCH-7` and `R-TUI-4`'s parts tree, plus the
   stage rows MOD-126 and MOD-132 need.
3. **The editor** (MOD-141), with MOD-115, MOD-123 and MOD-124.
4. **Subgraphs** (MOD-143), after ANA-35 and beside ANA-34.
5. **Narrowed MOD-27** (consult-mode `spawn_subagent`), after MOD-139.

## 8. Maintainer decisions requested (all accepted 2026-10-18)

**Accepted (2026-10-18):** the maintainer accepted all ten decisions below as recommended.

1. **One model: workflows are `step_graph` rows in the global scope, with no separate workflow table
   (7.1, answering ANA-40 P7).** Accept, or keep two models (6.1 D)?
2. **Parts are planned nodes at appended positions, and `fanout_index` stays compete-only (7.4).**
   Accept, or reuse `fanout_index` for parts (6.4 A)?
3. **Integrate: a deterministic pre-merge, the agent completing the merge, and an ancestry post-check,
   with only the integrate branch landing through §4.6 (7.5).** Accept, including the ANA-2 §4.6 and
   invariant 3 text amendments?
4. **A v2 run is pinned to its snapshot, and resume no longer compares against the live graph (7.2).**
   This supersedes the second clause of ANA-2 invariant 2 for v2 runs. Accept?
5. **Run-time changes: any step may submit a `successors` plan when `runtime_changes` is not `off`,
   default `ask`, and hard-gated and `asks you` nodes can never be bypassed (7.6).** Accept, or limit
   plan tools to planner nodes only?
6. **`if` takes a closed set of deterministic predicates, and agent-chosen branches go through a planner
   (7.3).** Accept?
7. **MOD-27 is narrowed to consult-mode `spawn_subagent`, `RunKind::Swarm` is dropped, and graph steps
   deny the harness's own subagent tool once it lands (7.12).** Accept?
8. **MOD-98's "opt-in" is a seeded alternative graph plus a library workflow, with no disabled-node flag
   (7.12).** Accept? This settles MOD-98's open choice now.
9. **`max_agents_per_run` counts planned sessions, not retries or rounds, matching the code (7.8).**
   Accept the correction to ANA-2 §4.5's text?
10. **A divided step with one failed part fails as a whole, and integrate never runs on a partial set
    (7.4).** Accept, or allow "integrate what finished" behind a per-node flag?

## 9. Risks and unknowns for the spawned items

- **Stack depth.** Engine walks already sat at the 2 MiB debug stack until MOD-10 M3 boxed the dispatch
  arms. Driving several nodes concurrently multiplies the futures in flight, so box every per-node
  future and gate the suite `--no-fail-fast` with a grep for SIGABRT.
- **Concurrency is counted per run, not per session.** `max_concurrent_items` counts runs, so a run with
  four parts draws four sessions and four quotas at once. A box-level session cap may be needed (open,
  MOD-139's plan). `R-ORCH-6`'s quota wait must apply per part.
- **Ancestry proves inclusion of commits, not of their changes.** An integrate agent could, for example,
  merge with `-s ours` and pass. The mitigations are the per-part diffstat in the review prompt and
  review after integrate. A stronger check, such as a per-part diff against `owned_paths`, is an
  unknown.
- **Parallel landings.** Parallel plain nodes landing into the primary tree need a per-repo reconcile
  lock, and the primary-tree-clean check of §4.6 step 1 must hold under it.
- **Known bug classes elsewhere.** Argo has stop-then-retry on fanned-out DAGs, and LangGraph has
  recovery inside nested subgraphs. MOD-125's test matrix must include a divided step and a subgraph
  node.
- **`if` predicates.** The set may prove too small. Extension is additive: a new predicate kind
  serialised with `skip_serializing_if`, as the persona field did, without a `V` bump.
- **`deny_kinds` coverage.** It is unverified that `deny_kinds` covers every adapter's edit tools for a
  read-only planner. ACP reports some tools as `other`. Verify in MOD-140, and otherwise give the
  planner a throwaway tree whose commits fail admission (`planner_wrote`).
- **Migration cost.** Each migration rebuilds every box's mirror and needs `cargo sqlx prepare` against a
  migrated scratch database. The `tests/migrations.rs` table and comment lists, plus the MemStore and
  PgStore conformance suites, grow with every new column.
- **Not verified here:**
  - the web rows of §4, which are survey summaries;
  - whether `merge_no_ff`'s two-parent post-condition (`isolate/git.rs:1451`) holds unchanged in a
    `copy` tree after fetch.

## 10. Spawned items

- **MOD-137 - Step graph v2 storage: edges, owner scope, revision, atomic edits.**
  - **Migration `0019`** (landed as `0020_step_graph_v2.sql`; MOD-125 took `0019`):
    - `step_graph_edge`;
    - `step_graph.workspace_id`, a nullable `project_id`, the scope CHECK and partial unique indexes;
    - `step_graph.revision`;
    - the `split`, `palette` and `plan_scope` columns, and the divide planner and integrate persona and
      agent columns;
    - `phase_part`;
    - the deterministic edge backfill from positions plus the explicit review loop edge.
  - **Code:**
    - `replace_graph` and deleters for phases, edges and `phase_agent` on `WriteStore`, the MemStore and
      the PgStore;
    - seeds write edges;
    - `StoreRequest` verbs on `catalogue::serve`;
    - the re-override owed at `graph.rs:455-461`.
  - **Dependencies:** not blocked.
  - **Requirements:** `R-ORCH-1`, `R-ORCH-3`, `R-TUI-8` (storage half).
  - **Answers:** ANA-40 P7.
- **MOD-138 - Snapshot v2 and the graph walker.**
  - **Snapshot:** `GraphSnapshot` v2 and its resolve, with divide expansion and bounds; the v1 lift in
    `snapshot_of`; pinned resume for v2.
  - **Walker:** `ready()` replacing `cursor`, with concurrent driving under `max_parallel_sessions`,
    all-joins and skip propagation, `if` predicates, `asks_you` parks, and per-edge loop targets with
    region retirement, limits and no-progress; graph-aware `required_inputs` and frontier; a per-repo
    reconcile lock.
  - **Validator:** a shared pure validator used by save and resolve.
  - **Dependencies:** MOD-137.
  - **Requirements:** `R-ORCH-1`, `R-ORCH-2`, `R-ORCH-3`, `R-ORCH-11`, and the `R-ORCH-4` seam
    (MOD-111 addresses nodes by position).
- **MOD-139 - Divided fan-out runtime: planner, parts, integrate.**
  - **Migration:** `run_step.role`, `parent_step_id` and `node_key`; `run_plan`; cache mirror `0006`
    for both.
  - **Admission:** planner expansion and the admission transaction; default-parts revision 0.
  - **Parts:** the shared `base_refs` part slot; the `{{part}}` and `{{parts}}` prompt sections; the
    resolver exclusion of part documents (invariant 8 extended); per-part retry; whole-step failure on an
    exhausted part.
  - **Integrate:** `Isolator::premerge`, the copy-mode fetch, the ancestry post-check, and landing only
    the integrate branch.
  - **Recovery and caps:** §4.9 recovery per role; caps at every admission; the new meaning of
    `command_queue` `fan_out_only`.
  - **Stage rows:** the `role = 'stage'` rows at -3, -2 and -1 for MOD-126 and MOD-132.
  - **Dependencies:** MOD-138.
  - **Requirements:** `R-ORCH-7`, `R-ORCH-8`, `R-ORCH-11`, `R-MCP-3`, and ANA-2 §4.6 and §4.9.
- **MOD-140 - The planner's MCP tools and run-time changes.**
  - **Tools:** `plan_bounds`, `plan_submit` and `plan_status` in `htui-mcp`, with a fenced `run_plan`
    proposal writer in the `write_step_document` style.
  - **Scope and validation:** the `ToolScope` role and plan scope set in `open_tools` (and in the chat
    constructors and test scopes); the shared plan validator; engine re-validation and admission after
    the session; refusal texts; `missing_output` for no proposal.
  - **Run-time changes:** the `runtime_changes` bound (`off`, `ask`, `automatic`) frozen from Settings >
    Workflows; the plan-approval park; the read-only planner exposure (`deny_kinds`).
  - **Dependencies:** MOD-139; MOD-11 is done. Coordinate with ANA-36 and MOD-112.
  - **Requirements:** `R-MCP-2`, `R-MCP-1`, `R-ID-6`, `R-ORCH-1`.
- **MOD-141 - One step-graph editor for Settings and the Toolbox.**
  - **Component:** a rataflow editor over `replace_graph`, with the node form (every field, including
    split, divide, candidates, palette and plan scope) and the edge form (relation, `if` predicate,
    limit).
  - **Behaviour:** the save-time validator; in-memory undo and redo over revision CAS; catalogue key
    actions with no hardcoded keys.
  - **Surfaces:** `Settings > Kinds` opens it on the project's graphs; the Toolbox opens it on the
    library and the workspace.
  - **Dependencies:** MOD-137, MOD-138 (validator), MOD-115.
  - **Amends:** MOD-124, whose workflow editor *is* this component (MOD-124 keeps the Toolbox shell), and
    MOD-123, whose Workflows section carries the `runtime_changes` default.
  - **Requirements:** `R-TUI-8`, `R-TUI-7`, `R-TUI-10`, `R-ORCH-1`.
- **MOD-142 - Runs pane for divided and planned runs.**
  - **Data:** `RunSummary` carries the decoded v2 graph and `run_plan` rows.
  - **Step list:** groups planner, parts and integrate as a tree with each part's state, works offline
    from the mirrored columns, and offers the plan-approval view at a planner gate.
  - **Compete affordances:** the judge layer, the `selected` mark and `s` appear only on a compete node.
  - **Dependencies:** MOD-139.
  - **Amends:** MOD-120, whose run graph reads the same graph, plans and stage rows. It does not need
    MOD-115 for the list view.
  - **Requirements:** `R-TUI-4`, `R-ORCH-11`.
- **MOD-143 - Subgraph nodes: one graph runs several items.**
  - **Migration:** `run.parent_run_id`, `parent_step_id` and `UNIQUE (parent_step_id, item_id)`.
  - **Node:** the `subgraph` node with a frozen child snapshot and an item source (planner-named items,
    or ANA-35's `parent` children); child-run creation through the normal claim; `max_child_runs`;
    completion and failure rules.
  - **Waiting and recovery:** slot release while waiting; idempotent recovery.
  - **Dependencies:** MOD-138; ANA-35 (the run waiting state and the `parent` link); ANA-34 (child
    integration).
  - **Requirements:** `R-ORCH-1`, `R-ORCH-9`, and the `R-ORCH-14` seam.

**Existing items affected:**

- **MOD-27:** narrowed to consult-mode `spawn_subagent`, and blocked on MOD-139.
- **MOD-36 and ANA-25:** unchanged, later tier.
- **MOD-98:** still not blocked; its open choice is settled, with opt-in meaning a seeded alternative
  graph.
- **MOD-75:** unchanged in scope. Its question park now meets concurrent nodes; whether a question
  parks only the asking part or node, or the whole run, is left open for MOD-75's plan.
- **ANA-30:** the §4 U5 amendment in 7.12.
- **MOD-120 and MOD-124:** amended as above.
- **MOD-126:** unblocked once MOD-139's stage rows land; the pick home is 7.11.
- **MOD-132 (ANA-42):** gains the `prepare` and `collect` stage rows.
- **MOD-111:** step control addresses nodes by position and `node_key`.
- **MOD-125:** its test matrix gains divided steps.
- **ANA-41:** reads 7.6 and 7.9 for "ANA-33's planned graphs".

**Text edits for the session's bookkeeping:**

- **ANA-2:** §4.1 rows 256-259 are superseded; invariant 2 (v2 pinning); invariant 3 and §4.6 step 3
  (7.5); invariant 8 (part documents); §4.5's cap text.
- **ANA-13:** §3.2 and §3.3 are superseded by 7.12.
- **`R-MCP-3`:** the meaning of `fan_out_only`.
- **`R-TUI-8`:** "with the shared graph editor".
- **`R-ORCH-1`:** `retries` is a self-loop on the same node (its own `retry_limit`, 7.3), not a loop
  back to an earlier step; 7.1 has no `retries` edge kind.
- **`CONCEPTS.md:42-43`:** graphs are owned by a project, a workspace or the global library.

## 11. Sources

- **`HANDOFF.md`:**
  - ANA-33 and ANA-34 at `:105-133`;
  - MOD-27 at `:203-223`;
  - ANA-25 at `:68-84`;
  - MOD-75 at `:467-480`;
  - MOD-98 at `:518-528`;
  - MOD-111 at `:656-664`;
  - MOD-120 to MOD-126 at `:759-812`;
  - MOD-36 at `:834-864`;
  - MOD-126's "blocked on ANA-33 for the pick's home" at `:811-812`.
- **`docs/REQUIREMENTS.md`:** `R-ORCH-1` to `R-ORCH-12` (`:365-424`), `R-ID-6` (`:113-123`),
  `R-MCP-1` to `R-MCP-3` (`:511-524`), `R-TUI-4` (`:551-561`), `R-TUI-7` and `R-TUI-8` (`:580-600`),
  `R-LATER-13` (`:699-701`).
- **Review and findings:** `docs/requirements-review-2026-10-15.md` §6 (`:127-162`; `R-ORCH-1` at `:129-137`, `R-ORCH-7` at `:147-155`), §10 and §11;
  `docs/ui-redesign-findings-2026-10-15.md` §4.5, §4.6, §5.2 and §5.4.
- **ANA documents:**
  - `docs/ANA-2.md` §2 (`:100-139`), §4.1 (`:239-259`), §4.4 to §4.6 (`:744-1065`), §4.9
    (`:1348-1447`), §10 (`:2222`);
  - `docs/ANA-13.md` §3.2 and §3.3;
  - `docs/ANA-21.md` §2.2;
  - `docs/ANA-30.md` §4 U5;
  - `docs/ANA-40.md` §4.4 and §8 P7;
  - `docs/ANA-42.md` §7.5 and §7.6.
- **Code:**
  - `crates/htui-store/migrations/0001_init.sql`, `0003_orchestration.sql` and `0012_persona.sql`;
    `cache_migrations/0001_mirror.sql`;
  - `crates/htui-core/src/model/kind.rs` and `run.rs`; `seed.rs`; `store/traits.rs` and `worker.rs`;
  - `crates/htui-orch/src/graph.rs`, `status.rs`, `gate.rs`, `engine.rs`, `fanout.rs`, `recover.rs`,
    `isolate.rs`, `isolate/git.rs`, `command.rs` and `tools.rs`;
  - `crates/htui-mcp/src/tools/mod.rs` and `host.rs`;
  - `crates/htui/src/ui/tabs/settings/kinds.rs`, `catalogue.rs` and
    `ui/tabs/backlog/detail/runs/execution_graph.rs`;
  - `spikes/ui-skeleton/src/data.rs` and `flow.rs`.
- **Web** (via the survey reader):
  - code.claude.com/docs/en/workflows and /worktrees;
  - docs.langchain.com (graph API, subgraphs) and langgraph issues #9106 and #9276;
  - docs.temporal.io (child workflows, worker versioning);
  - argo-workflows field reference, issues #7617 and #17134, PR #16938;
  - docs.github.com (re-runs, merge queue);
  - airflow.apache.org (dynamic task mapping, bundles);
  - docs.prefect.io;
  - openai.github.io/openai-agents-python;
  - microsoft.github.io/autogen (Magentic-One);
  - cognition.com (Devin manages Devins);
  - openhands.dev (parallel refactors);
  - factory.com (Missions);
  - forum.cursor.com (worktrees, best-of-n);
  - addyosmani.com (code agent orchestra).
