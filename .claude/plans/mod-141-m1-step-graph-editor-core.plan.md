# Plan: MOD-141 M1 - Step-graph editor core in Settings > Kinds

**Source PRD**: `.claude/prds/mod-141-step-graph-editor.prd.md`
**Selected Milestone**: 1 - Editor core in `Settings > Kinds`
**Complexity**: Large
**Routing**: PRD path, ultracode for implement and review (accepted 2026-11-10).

## Summary

A scope-generic, rataflow-drawn step-graph editor (`crates/htui/src/ui/graph_editor/`) that edits one
`GraphBody` at a time: nodes and typed edges on a canvas, a node form and an edge form, a links list,
client-side validation with the store's own rule functions, and in-memory undo and redo where every
applied edit is one `StoreRequest::ReplaceGraph` CAS on `revision`. `Settings > Kinds` hosts it as a
full-section sub-view (the memory-pane pattern) on the project's graphs and retires its position-based
phase forms. The store worker gains a body-carrying reply and a one-graph read, so the same entry point
opens workspace and library graphs (MOD-124's hook, tested, not wired to any UI in M1).

## Decisions (D)

- **D1 Location and entry.** `crates/htui/src/ui/graph_editor/` (a sibling of `path_picker.rs`, not under
  `tabs/settings`, because the Toolbox will host it too). `GraphEditor::open(body: GraphBody)` takes any
  scope; the host supplies agents, templates and personas. Nothing in the component names Kinds.
- **D2 Hosting.** `KindsSection` gains `Mode::Graph { editor: Box<GraphEditor> }`, the memory pane's shape
  (`hierarchy.rs:1513-1522`, `settings/memory.rs`): the editor takes every key, paste and reply while open,
  its stack carries no `SETTINGS` layer (sections do not cycle under it), it renders over the whole section
  area, and its close act returns to Browse with the cursor on the graph.
- **D3 Opening.** New act `settings.kinds.editor` ("open graph editor") on a Graph row opens it on that
  graph; on a Phase row it opens with that node selected. Default chord settled in T6 against
  `no_two_actions_in_one_context_share_a_default_chord` (first choice `Enter`, else `o`). `g` keeps the
  name and description form.
- **D4 One write per applied edit.** An edit builds the next body, runs the client-side validator (D5),
  and only then sends `ReplaceGraph { graph, expected_revision, body }`. The editor is busy until the
  reply: `Applied` adopts the returned body and revision and pushes the previous body on the undo stack;
  `Stale` adopts the current body, **clears undo and redo**, and says so ("changed elsewhere, reloaded;
  undo history cleared"); `Failed { request: "replace_graph" }` keeps the body and shows the refusal.
  Rebasing undo across a foreign write is out (answers PRD open question 2).
- **D5 Validator.** Client-side, the same `pub` functions the store runs in its order:
  `graph_body_shape_refusal`, persona and agent references against the host's lists
  (`body_persona_refs`/`body_agent_refs`), `graph_body_topology_refusal`, `graph_body_walk_refusal` (which
  includes the `if` closed set). A refused body never reaches the worker. The store still re-validates.
- **D6 Undo and redo.** A stack of whole `GraphBody` values with a label each ("adding security review"),
  100 deep, redo cleared by a new edit. Undo and redo are themselves one `ReplaceGraph` each, CAS on the
  current revision. Known limit: undoing a node delete re-inserts the phase id, but its `skill_binding`
  rows, `deadline_seconds` and judge columns are not restored (the store drops bindings on delete and
  writes the columns `GraphBody` lacks as NULL on a new id, `traits.rs:1085-1088`); the confirm question for
  delete says so when the node has bindings.
- **D7 Layout.** Not stored (answers PRD open question 3). Forward edges (`forward_mask`, `pub`,
  `graph_rules.rs:409`) are layered by longest path, mirroring `execution_graph.rs` `layers`/`project`
  (:88, :136), deterministic for snapshots and independent of rust-sugiyama's unknown cycle handling.
  Back edges leave and enter by the sides: `changes_requested` on the right, `fails` on the left, a back
  `asks_you` on the right (findings §4.6). Palette nodes sit in a separate band below the graph (they have
  no edges). The viewport is kept across re-layouts. No node move (`M`/drag) in M1: positions are not stored.
- **D8 Node form (M1 fields).** name, template name and version (from `StoreRequest::Templates`), persona
  (catalogue personas, or none), gate and gate_hard, retry_limit, isolation (project default or a value),
  command_queue, verify_command (empty = none), token_budget (empty = none, `>= 1`), input_kinds,
  output_kind, and the candidate agents: an ordered `phase_agent` list (add, remove, move up and down; agent
  and model from `StoreRequest::Agents`'s `AgentSummary.models`). Written through the body:
  `replace_graph` already writes `token_budget`, and the Phase rung of `set_setting` bumps `revision`
  (`mem.rs:5003`), so the CAS catches either writer. Shown read-only in M1, edited in M2: split
  (`none`/`compete`/`divide`, with "runs once MOD-139 lands" on divide, `graph.rs:312-315`), fan_out,
  palette, plan_scope, divide columns and parts, **deadline**.
- **D9 Edge form.** Relation picker over `EdgeKind` (`next`, `if`, `asks_you`, `changes_requested`,
  `fails`); `if` takes its predicate as one line of JSON, parsed live with `IfPredicate::parse`
  (`predicate.rs`) and its error shown under the field; `loop_limit` required on `changes_requested` and
  `fails`, optional on `asks_you`, absent otherwise (`EdgeKind::needs_loop_limit`). `retries` is not an
  edge: it is the node's `retry_limit` (ANA-33 §10, `R-ORCH-1` text edit). Self-loops cannot exist (the
  model and rataflow both refuse them).
- **D10 Node operations.** `add`: a new node after the selected one, which takes over the selected node's
  outgoing forward edges and gets a `next` from it; the form opens first and the add is one write on save
  (defaults: the selected node's template, `gate never`, retry 0, fresh UUIDv7 ids, position max + 1).
  `delete` (confirmed): the node goes with its edges; every forward edge into it is re-pointed at each
  target of its outgoing forward edges, keeping its kind, duplicates dropped; if the result is refused,
  the refusal is shown and nothing is written. `connect`: arrows pick the target, then the edge form. The
  links list shows the selected node's edges in and out: change the relation, edit, delete.
- **D11 Mouse.** Click selects a node; a drag from a handle (`FlowEvent::ConnectionCompleted`,
  `actions.rs:188`) opens the edge form; a click on an edge opens the links list on it; right-click edits
  the node. `FlowAction::Delete` is never routed to rataflow (it deletes at once); rataflow gets no keys.
- **D12 Keys (R-TUI-10).** New contexts `graph_editor` (canvas) and `graph_editor.links`; the forms reuse
  `form`/`field`/`choice` acts where one exists. Defaults from findings §4.6: `a` add, `e`/`Enter` edit,
  `x` delete, `c` connect, `i` links, `u`/`ctrl-z` undo, `U`/`ctrl-y` redo, `+ - =` zoom and fit, arrows and
  `h j k l` select in a direction, plus a close act. Collisions resolve per context because the editor's
  stacks hold only its contexts plus `global` (no `common`, `settings`, `layout`): `c` is not
  `common.clear` there, `x`/`r` never meet Runs, `ctrl-z` never meets pane zoom (`[layout] zoom` is `z` and
  Settings stacks carry no layout layer). No F-keys.
- **D13 Store worker.** `ReplaceGraph` answers `StoreReply::GraphReplaced { outcome:
  CasOutcome<Box<GraphBody>>, catalogue: Box<CatalogueSnapshot> }` (Kinds keeps reading the catalogue, the
  editor reads the body). New `StoreRequest::GraphBody(StepGraphId)` answers
  `StoreReply::GraphBody(Option<Box<GraphBody>>)` for opening a graph by id (the Toolbox hook).
  `REQUEST_NAMES` goes from 11 to 12.
- **D14 Retiring the phase forms.** `EditorKind::NewPhase`/`EditPhase` and `DeletePhase` from Kinds go;
  `n` on a graph row and `e`/`d` on a phase row answer with a pointer to the editor (or `e` opens it on the
  node). The phase tree stays as read-only rows; the `MOD_4_OWNS` detail line is replaced by the node's
  summary without the "MOD-4 owns these" tail. Retired only once T5 covers every field those forms edited
  (name, position by order, template, gate_hard, input_kinds, persona, budget).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Full-section sub-view | `crates/htui/src/ui/tabs/settings/hierarchy.rs:1513-1522`, `settings/memory.rs:1-19` | A `Mode` variant holding a self-contained pane that takes keys, paste, replies, `wants_requests`, stack, scroll and render |
| rataflow projection | `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs:88,136,283-313,385-393,467,492,629,703` | Own layering, `NodeContent` drawing a kit `Pane`, hidden handles, `set_nodes`/`set_edges` sync with selection rebuilt, viewport kept |
| Editor prototype | `spikes/ui-skeleton/src/flow.rs:463-526`, `main.rs:541-660`, `app.rs:397-398,667` | Side handles for loops, connect flow, links modal, snapshot undo capped at 100 |
| Key context | commit `9c267f92` (MOD-163 memory pane) | Every file a new context touches (catalogue, views `VIEWS`, stack `DECLARED`, pins, `docs/htui-keys.md`) |
| Errors in words | `kinds.rs:66-147` constants, `refuse`/`say` | A refusal is a sentence shown in the notice line, never a panic |
| CAS replies | `catalogue.rs:413-419` `cas` | Applied/Stale mapped to replies, Stale reloads by hand |
| Tests: section | `crates/htui/tests/kinds.rs` `SectionBench` (`testkit.rs:747`), `bench_with_demo` :1152 | Keys by chord, requests asserted as `Action::Store(..)`, no worker (`R-NF-3`) |
| Tests: worker | `crates/htui/tests/kinds.rs:39,70,813` | `store_worker::serve(&Backend::memory(MemStore::demo()), &request)` |
| Snapshots | `crates/htui/tests/snapshots/kinds__*.snap` | `insta::assert_snapshot!` on `render_section` text |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui/src/ui/mod.rs` | UPDATE | `pub mod graph_editor;` | T0 |
| `crates/htui/src/ui/graph_editor/mod.rs` | CREATE | module root (T0 skeleton), then `GraphEditor` (T6) | T0, T6 |
| `crates/htui/src/ui/graph_editor/model.rs` | CREATE | pure body edits and the validator chain (D4, D5, D10) | T2 |
| `crates/htui/src/ui/graph_editor/history.rs` | CREATE | undo/redo stack and save state machine (D4, D6) | T3 |
| `crates/htui/src/ui/graph_editor/canvas.rs` | CREATE | rataflow projection, layout, selection, mouse, render (D7, D11) | T4 |
| `crates/htui/src/ui/graph_editor/node_form.rs` | CREATE | node form and candidate list (D8) | T5 |
| `crates/htui/src/ui/graph_editor/edge_form.rs` | CREATE | edge form (D9) | T5 |
| `crates/htui/src/ui/graph_editor/links.rs` | CREATE | links list (D10) | T5 |
| `crates/htui/src/store_worker.rs` | UPDATE | `GraphReplaced`, `GraphBody` request and reply (D13) | T1 |
| `crates/htui/src/catalogue.rs` | UPDATE | serve both, `REQUEST_NAMES` 12 (D13) | T1 |
| `crates/htui/src/keys/catalogue.rs`, `keys/views.rs`, `keys/stack.rs`, `keys/load.rs` | UPDATE | contexts, acts, stacks, pins (D12, D3) | T6, T7 |
| `crates/htui/tests/keys_file.rs`, `docs/htui-keys.md` | UPDATE | table order, regenerated key doc | T6, T7 |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | UPDATE | `Mode::Graph`, open act, retire phase forms, `GraphReplaced` routing (D2, D3, D14) | T1 (reply routing only), T7 |
| `crates/htui/tests/kinds.rs` | UPDATE | worker tests for D13 (T1); section tests for hosting and retirement (T7) | T1, T7 |
| `crates/htui/tests/snapshots/kinds__*.snap` | UPDATE | phase detail line and retired editor frames | T7 |
| `crates/htui/tests/graph_editor.rs` | CREATE | end-to-end: reference graph, field round trips, scopes | T8 |

## Tasks

TDD throughout: each task writes its failing tests first. Wave 1 modules are unused until T6, so
`graph_editor/mod.rs` carries `#![allow(dead_code)]` from T0 to T6, where it is removed (the bare clippy run
in `scripts/gate.sh:83-84` would refuse dead code otherwise).

### Task 0: Skeleton (serial, first)
- **Action**: `pub mod graph_editor;` and the seven files with module docs and the submodule lines.
- **Validate**: `cargo check -p htui --all-features`.

### Wave 1 (parallel; file sets disjoint, see the check below)

#### Task 1: Store worker body replies (D13)
- **Action**: `StoreReply::GraphReplaced` for `ReplaceGraph`; `StoreRequest::GraphBody` and
  `StoreReply::GraphBody`; `try_serve` routing; `REQUEST_NAMES` 12; Kinds' `on_reply` reads
  `GraphReplaced.catalogue` exactly as it read `Catalogue`/`CatalogueStale`.
- **Mirror**: `catalogue.rs` `cas`; the existing `ReplaceGraph` worker tests (`tests/kinds.rs:813-1100`).
- **Validate**: `cargo test -p htui --features testkit --test kinds -- --test-threads=1`.

#### Task 2: Body model and validator (D4, D5, D10)
- **Action**: pure functions over `GraphBody`: add after (taking over forward out-edges), delete with
  rewire, connect, edit edge, remove edge, replace node; `validate(body, personas, agents) -> Option<String>`
  in the store's order. Unit tests per operation, including a delete whose rewire the walk refuses.
- **Mirror**: `graph_rules.rs` test fixtures (`node`, `edge`, `body`, :926-1006).
- **Validate**: `cargo test -p htui --lib ui::graph_editor::model`.

#### Task 3: History and save state (D4, D6)
- **Action**: undo/redo of labelled bodies (cap 100, redo cleared on a new edit), the busy state, and the
  transitions on Applied, Stale (stacks cleared) and Failed. Pure, no I/O.
- **Mirror**: spike `app.rs:667` `wf_checkpoint`.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::history`.

#### Task 4: Canvas (D7, D11)
- **Action**: `GraphBody` to a rataflow `Flow`: longest-path layers over forward edges, side handles for
  back edges, edge labels (`if`, loop limit), palette band, read-only split badge, selection in a
  direction, zoom and fit, mouse to events. Snapshot tests of rendered frames (linear seed graph, review
  loop, `if` branch, palette, divide badge).
- **Mirror**: `execution_graph.rs`; spike `flow.rs:463-526`.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::canvas`.

#### Task 5: Forms and links (D8, D9, D10)
- **Action**: node form (every M1 field, candidate list, inline refusals), edge form (relation, live
  predicate parse, loop-limit rules), links list. Key-agnostic methods (`next_field`, `cycle_choice`,
  `insert_char`, ...); acts are mapped to them in T6. Each form yields a `NodeBody` or `StepGraphEdge`.
  Snapshot tests of each form.
- **Mirror**: `kinds.rs` `Editor`/`Field` and kit `Form`.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::{node_form,edge_form,links}`.

**Independence check (step 3.5):**
- T1 = {`store_worker.rs`, `catalogue.rs`, `kinds.rs`, `tests/kinds.rs`}
- T2 = {`graph_editor/model.rs`}
- T3 = {`graph_editor/history.rs`}
- T4 = {`graph_editor/canvas.rs`, its snapshots}
- T5 = {`node_form.rs`, `edge_form.rs`, `links.rs`, their snapshots}

All pairwise intersections are empty. The shared root `graph_editor/mod.rs` is written by T0 before the
wave and not touched again until T6. Insta names its unit snapshot files by module path, so the
snapshot files don't collide. There are no migrations and no `.sqlx` changes. Coupling at build time:
all five tasks compile one crate, so a broken file in one task breaks the others' builds. Each
implementer therefore works in its own worktree on a named branch, and the branches are merged serially
(memory: workflow worktree implementers).

### Wave 2 (serial)

#### Task 6: `GraphEditor` and its keys (D1, D4, D11, D12)
- **Action**: the orchestrator. It has modes (canvas, node form, edge form, links, confirm delete, connecting), maps acts to the wave 1 methods, sends requests through the host's `Ctx`, and handles `GraphReplaced`/`Failed`. It asks for `Agents` and `Templates` while open. The header shows name, scope, revision and "u undoes: …".
- **Keys**: the new contexts and acts, wired across every file commit `9c267f92` touched. Regenerate `docs/htui-keys.md` and remove the `dead_code` allow.
- **Tests**: unit tests open a Project, a Workspace and a Library body and assert that each save carries its own graph id and revision.
- **Validate**: `cargo test -p htui --lib keys:: -- --test-threads=1`, `--test keys_file`,
  `--test no_hardcoded_keys`, `--lib ui::graph_editor`.

#### Task 7: Kinds hosting and retirement (D2, D3, D14)
- **Action**: `Mode::Graph`, the open act, retirement of the phase forms with refusal pointers, the new phase detail line, and the updated insta snapshots (full insta run, not a `.snap` grep).
- **Validate**: `cargo test -p htui --features testkit --test kinds -- --test-threads=1`; `cargo insta test -p htui --features testkit` (review every changed snapshot).

#### Task 8: End-to-end acceptance (PRD metrics)
- **Action**: `tests/graph_editor.rs` over `Harness::over(MemStore::demo())`:
  - build the reference graph: implement → review, review `changes_requested` back to implement limit 3, `if {"verify":"failed"}` to a fix step;
  - check it saves and resolves;
  - round-trip each M1 node field and each edge field;
  - undo and redo across three edits;
  - a Stale reply clears the stacks;
  - a refused body never sends a request.
- **Validate**: `cargo test -p htui --features testkit --test graph_editor -- --test-threads=1`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings                  # featureless (memory: featureless clippy gate)
cargo clippy --workspace --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui --features testkit --test kinds --test graph_editor --test keys_file --test no_hardcoded_keys -- --test-threads=1
bash scripts/gate.sh                                     # full gate; Qdrant timeouts in the sandbox are owed on the host
```

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `replace_graph` is one CAS on `revision` and re-validates shape, topology, walk and `if` | true | `traits.rs:1076-1112`; `mem.rs:4187-4314` |
| The validator functions are `pub` and usable client-side | true | `graph_rules.rs:86,186,515` (`pub fn`), re-exported `store/mod.rs:16` |
| `GraphBody` carries every M1 node field | **partly false** | `StepGraphPhase` lacks `deadline_seconds`, `judge_agent_id` and `judge_model` (`traits.rs:1085-1088`). **Plan amended: deadline moves to M2** (PRD rows updated) |
| `ReplaceGraph` answers the written body | **false** | It answers the scope's `Catalogue`/`CatalogueStale` (`catalogue.rs:383-391,413`), and shared graphs carry no body (`catalogue.rs:62-66`). **Plan amended: D13** |
| `token_budget` written through the body is safe | true | `replace_graph` writes the phase row whole; the Phase rung bumps `revision` (`mem.rs:5003`), so the CAS sees either writer |
| Divide is refused at run start before MOD-139 | true | `htui-orch/src/graph.rs:587,710-728,312-315`; test `a_divide_node_is_expanded_then_refused` :5146 |
| rataflow mutation, `select_node_in_direction`, `select_next_edge`, `ConnectionCompleted`, edge labels | true | `state/graph.rs:376,418`, `state/selection.rs:141,284`, `actions.rs:188`, `types/edge.rs:118` (rataflow 0.1.0 source) |
| rataflow refuses self-loops; the model has none either | true | `state/graph.rs:170-175` (per research agent); `StepGraphEdge.to` "never `from`" (`kind.rs`) |
| Pane zoom is not `ctrl-z` and is not reachable in Settings | true | `docs/htui-keys.md:344` `[layout] zoom = ["z"]`; `KINDS_BROWSE` has no layout layer (`keys/views.rs:358`) |
| No undo precedent in `crates/htui` | true | Only the spike (`app.rs:397-398`); `text_area.rs:10` |
| `forward_mask` is `pub` | true | `graph_rules.rs:409` |
| Agents and templates are not in the catalogue; separate requests exist | true | `store_worker.rs:823,1534,1669`; `catalogue.rs:36-73` |
| Wave 1 tasks are file-disjoint | true | Intersection check above |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Longest-path layering draws wide or crossing graphs badly | Medium | Snapshots on the five seeded graphs plus the reference graph; rust-sugiyama is a later swap behind the canvas module |
| Key pins (`context_position`, counts 52/222/84/91) drift when T6 and T7 both add acts | High | T6 and T7 run serially; T7 re-runs `keys::` tests |
| Undoing a delete loses skill bindings and judge/deadline columns | Medium | D6: the confirm question says so; M2's body widening restores deadline and judge |
| Snapshot churn across Kinds frames | High | Full `cargo insta test` run in T7 and in review |
| Worktree disk use (~10 GB target each, 5 in wave 1) | Medium | `df -h .` before fan-out; fall back to 3 worktrees (T2+T3 together) if space is short |
| Mouse drag on rataflow conflicts with the canvas selection | Low | Keyboard path is complete on its own; the mouse is tested separately |

## Acceptance

- [ ] All tasks complete, TDD (tests first)
- [ ] Validation passes (the sandbox's Qdrant/Postgres-live failures are recorded and owed on the host)
- [ ] Patterns mirrored, not reinvented
- [ ] The PRD's M1 metrics hold: field round trips, saved graphs resolve, the reference graph built in the TUI, zero hardcoded keys
