# MOD-28 - rataflow execution view (done, 2026-10-02)

**Requirements:** `R-TUI-4` (Runs sub-tab steps and actions). ANA-12's invariants hold as
constraints: the graph is a projection of the store's rows and never a source of state, every run
action works from it, it renders inside the frame budget, and it can be driven by keyboard alone.
**Origin:** ANA-12 (`docs/ANA-12.md` §4, concluded 2026-09-14).
**Artifacts:**
- plan [`.claude/plans/mod-28-rataflow-execution-view.plan.md`](../../../.claude/plans/mod-28-rataflow-execution-view.plan.md): D1-D15, with its verified-claims table;
- blueprint `.claude/plans/mod-28-rataflow-execution-view.blueprint.md`: deviations B-1-B-10, decisions E1-E16.

Decision numbers are local to MOD-28 (the MOD-31 convention), because parallel sandbox runs share
the global sequence.

Routed as **plan** (C2 fired; C4 borderline, low confidence). Run in a TOOL-7 sandbox (`hr/MOD-28`).

**Decisions (maintainer, 2026-10-02):**
- route accepted, no ultracode;
- before the plan was drafted, two scope cuts:
  - **keyboard only**: mouse support is still wanted, so it becomes **MOD-71**;
  - **no tool-call chips**: these become **MOD-72**;
- plan confirmed as fact-checked;
- review: every finding applied.

**Commits:**
- plan and blueprint: `40abe2e9`, `f1674e6b`;
- T1 dependency and projection: `b277dd0f`, `49d450dc`;
- T2 `ExecutionGraph`: `424ff0a3`, `5710c273`;
- T3 Runs flow view: `0f072825`, `a7cabd39`;
- review fixes: `58ce5bc6`, `f3c9719d` (H1, L3, L4, N1), `3a9225d1`, `6da19ae3` (M1, L1, L2, L5, N2-N5).

---

## What was built

In the Backlog detail **Runs** sub-tab, `v` switches between the step list and a **flow view**. The
flow view draws the run under the cursor as a `rataflow` node graph:

- **Nodes:** one per step, showing slot and status, then the phase. A winner is marked `✓`, a
  promoted step `*`.
- **Fan-outs:** candidates sit side by side and the judge goes in the layer below them. Only the
  winner feeds the next phase.
- **Retries:** a retried attempt is the next layer, and the first edge into it is labelled `retry`.
- **Head:** above the canvas the view draws the run's header lines (mode, failure,
  `cancel requested`, waiting for the walk), the cursor step's gate note, and its pending
  permission request when there is one.

The list and the flow view share **one cursor**:

- `J`/`K` move it in the list's order and cross runs. Moving into another run swaps the graph.
- `Enter`, every action key (`a x r p s A o c T u R C`) and the permission digits work the same in
  both views, with no new action code.
- `+`/`-` zoom (0.5-2.0) and `=` fits the run. The cursor node is always revealed.

The view is rebuilt from every `Runs` reply, so it updates live through the existing `RunStream`
re-read. A re-read of the same run keeps the canvas still.

**Code:**
- `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` (new):
  - `project`: a pure projection of a `RunSummary` into layers, edges and positions;
  - `StepNode`: a `NodeContent` that draws its own border and text in htui `Theme` styles, clipped
    by display width;
  - `ExecutionGraph`: owns the `Flow`; sync, zoom, fit, and the scratch-pass reveal render.
- `runs.rs`: the `View` toggle, the keys, the flow head and the re-sync points.
- `rataflow = { version = "0.1", default-features = false }`: `Cargo.lock` gains only `rataflow`.

## Decisions as built (plan D1-D15)

- **D1, naming.** The UI calls it the **flow** view, because a **Graph** sub-tab (linked items,
  MOD-14) already exists. The widget is still `ExecutionGraph`, as the item text names it.
- **D2/D3, scope.** Keyboard only. Tool-call chips are deferred; the Runs tab has no `SessionEvent`
  data. Mouse is MOD-71, chips are MOD-72.
- **D4, dependency.** No `sugiyama`, no `crossterm` feature, and our own layered layout.
  `apply_layout` stacks disconnected components at one origin and leaves edgeless nodes unplaced.
  The feature would also have added `rust-sugiyama`, `petgraph` and duplicate `hashbrown`/`foldhash`.
- **D5, one run at a time.** The view draws the run that holds the cursor.
- **D6, layers.** Layers come from `(position, attempt, fanout_index)`; no parent link reaches the
  TUI, and `parent_step_id` is MOD-27's.
  - The judge (`-1`) sorts before its candidates, so it is moved after them.
  - The winner is `selected == Some(true)`.
  - A retry is a later `attempt` at the same `position`.
- **D7, shared cursor and reveal.**
  - The cursor node is built `with_selected`, because `set_nodes` clears selection.
  - A reveal is deferred until a canvas exists. Before the first render the canvas size is 0, so
    a reveal would be a no-op.
- **D8, keys.** `v`, `+`, `-` and `=` are free on Runs. Flow never consumes Tab or the digits,
  except for a digit answering a pending request, as in the list.
- **D9, read-only.** Nodes are not draggable, connectable or deletable. Their handles are hidden
  Bottom/Top, baked into the builder because `set_nodes` replaces nodes. Edges are neither
  selectable nor reconnectable. `locked` is not used, because it also blocks selection.
  rataflow's own key handlers are never called; their defaults include Del to delete and `i` to
  lock.
- **D10, nodes.** Nodes are 20×4 with gaps of 1 column and 3 rows.
  - At 20 columns two candidates fit the 43-column pane with the reveal's 1-cell margin.
  - The 3-row gap keeps the `retry` label off the arrowhead.
- **D11-D15:**
  - the modal footers and the artifact view are unchanged in flow;
  - every reply rebuilds the graph;
  - the graph sits in a `RefCell`, because `DetailTab::render` takes `&self`;
  - the view state is per tab and starts as list;
  - there is no animation and no clock.

**Deviations from the plan (blueprint B-1-B-10, review):**
- **B-1, centring.** A new run centres the widest layer, not the cursor node. Centring on the
  cursor cut the second candidate off at column 43.
- **B-2, scratch pass.** The draw-reveal-draw sequence draws its first pass into a scratch buffer,
  because rataflow never clears its area and a second draw into the frame left ghost borders.
- **B-3, ordered reveal.** The pending reveal is ordered (`None < Cursor < Reset`), so a later
  re-sync cannot downgrade a reset.
- **B-4, edge colours.** Edges take htui `dim`/`accent` instead of rataflow's indexed palette.
- **B-5, repeated ids.** A repeated step id keeps the first occurrence, so `set_nodes` cannot fail.
  An impossible error clears the flow; nothing panics.
- **Review H1 and L3, flow head.** The plan's one-line head (D11) became the full head described
  above. H1: the list draws a pending request under its step, so a digit in flow could answer a
  request the screen never showed.
- **Review M1, fit.** At the 0.5 zoom floor a tall run does not fit, so `=` also reveals the
  cursor.
- **Review L2, wider re-read.** When a re-read adds a wider layer, the viewport moves by the
  centring change, so the existing nodes stay put.
- **Review L5, retry label.** `retry` labels only the first edge into each target.

## Gate

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | clean |
| `cargo test -p htui --all-features -- --test-threads=1` | 1775 passed |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 106 suites, 3693 passed, 0 failed, 30 ignored; no SIGABRT (on `6da19ae3`) |

**Snapshots:** `backlog__runs_flow_fanout` and `backlog__runs_flow_reject_note` are new, so there
are 133 tracked snapshots. The HANDOFF pin said 129, but the tree already had 131 before MOD-28
(blueprint B-9).

## Carried

- **MOD-71, mouse support.**
  - An htui-wide mouse-capture policy. Capture disables the terminal's own text selection, so the
    proposal is capture only while a view wants it.
  - `Event::Mouse` routed through `Tab`/`DetailTab`.
  - Click to select, drag to pan and scroll to zoom in the flow view (`rataflow`'s `crossterm`
    feature, `handle_mouse_event`, `FlowEvent::NodeClicked`).
  - The restore, panic-hook and editor-suspend paths.
- **MOD-72, tool-call chips.** A per-step tool-call count read: Postgres, `MemStore`, `.sqlx`, and
  a `StoreRequest`/`StoreReply` pair. The counts are drawn as chips in `StepNode` (ANA-12 §3.2).
- **Hierarchy edges.** Swarm `parent_step_id` edges wait for MOD-27.
- **Fan-out width.** A fan-out of three or more candidates is wider than the 43-column pane at
  zoom 1. The reveal and `=` cover it until MOD-71 adds free panning.
