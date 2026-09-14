# ANA-12 - rataflow execution workflow visualization

> **Scope note:** Analysis of how `rataflow` should be integrated into `htui` to visualize the execution workflow of an item (inspired by `zoetrope`), addressing the shift from linear step lists to dynamic execution trees (fan-outs, swarms, review loops).
>
> **Requirements addressed:** `R-TUI-4` (Runs tab step list and actions), `R-ORCH-7` (Fan-out selection), `R-ORCH-3` (Review loop).
>
> **Status (2026-09-14): concluded.** Implementation spawned as MOD-28 (rataflow execution view).

---

## 1. Context and problem statement

`htui` orchestrates agents using step graphs. Historically, these were strictly linear (`prd` -> `plan` -> `implement` -> `review`), making `R-TUI-4`'s "step list" sufficient for visualizing a run.

However, execution in `htui` is increasingly non-linear:
- **Review loops (`R-ORCH-3`):** Phases can retry, creating cycles or multiple attempts.
- **Fan-outs (`R-ORCH-7`):** Parallel agents run concurrently on isolated worktrees.
- **Dynamic Swarms (MOD-27):** Agents can dynamically spawn subagents hierarchically using the `task` MCP tool.

A flat list cannot effectively convey the concurrency, parent-child relationships, and real-time state of these executions. `rataflow` is a Rust TUI library for node-based graphs. `zoetrope`, an application built on `rataflow`, demonstrates how to effectively visualize complex, hierarchical agent execution transcripts in real-time, including tool calls and subagent trees.

This document determines how `htui` can adopt `rataflow` to visualize item execution workflows, what data models it will touch, and how it integrates into the TUI event loop.

---

## 2. Invariants

1. **State Machine Authority:** The orchestrator and the database own the source of truth (`run`, `run_step`, `run_step_tree`, `session_event`). The `rataflow` graph is a pure projection of these rows, never the source of state.
2. **Action Parity:** `R-TUI-4` mandates actions (approve, reject, retry, promote to chat, cancel, open artifact, select fan-out result). The graph view must support triggering these actions (e.g., via node selection and context menus).
3. **Non-blocking Rendering:** `rataflow` must render within `htui`'s existing `ratatui` frame budget without blocking `tokio` tasks or lagging the TUI.
4. **Degradation:** Terminal environments lacking full mouse support must still be able to navigate the execution history, meaning keyboard navigation between nodes must be implemented.

---

## 3. Options and Verdicts

### 3.1 Integration Surface
**Need:** Where should the `rataflow` execution graph live?

| Option | Verdict |
|---|---|
| Replace the `Runs` tab step list entirely | Rejected. A list is still denser and better for simple linear runs or accessibility/keyboard-heavy workflows. |
| Add as a view toggle in the `Runs` tab (`R-TUI-4`) | **Adopted.** Add a toggle (e.g., `v` for View mode) in the `Runs` pane to switch between "List" and "Graph" (`rataflow`). The Graph becomes the primary view for complex (fan-out/swarm) runs. |
| Use `rataflow` in `Settings` to design step graphs | Rejected for v1. Step graphs are mostly static templates configured via seeded kinds (`R-ENT-6`). The highest value is in visualizing *execution*, not templating. |

### 3.2 Mapping the Store to Graph Nodes
**Need:** How do `htui`'s database rows map to `rataflow` nodes and edges?

| Concept | Mapping Strategy |
|---|---|
| **Nodes** | Each `RunStep` becomes a node. The node displays the phase name, agent name, model, and execution status (`running`, `done`, `failed`, `awaiting_approval`). |
| **Edges (Sequence)** | Linear progression edges connect a step to the subsequent step based on `position` and `attempt`. |
| **Edges (Hierarchy)** | `parent_step_id` (from MOD-27 swarm spawns) or fan-out dispatchers create hierarchical child edges. |
| **Tool Calls** | Following the `zoetrope` model, `SessionEvent` rows of type `tool_call` are visualized as chips or indicators inside the `RunStep` node (e.g., `⚒ bash ×3`). |

### 3.3 Event Loop & Interaction
**Need:** `htui`'s standard event loop needs to interact with `rataflow`'s internal event system (`FlowEvent`).

| Option | Verdict |
|---|---|
| Isolate `rataflow` in a separate `tokio` task | Rejected. UI rendering must happen synchronously in the `ratatui` draw closure. |
| Route events conditionally based on focus | **Adopted.** When the Runs tab is focused and in Graph mode, mouse events (drag, click, scroll) are piped to `flow.handle_mouse_event()`. `FlowEvent::NodeClicked` translates to selecting the corresponding `RunStep` in `htui`'s state, enabling the standard `R-TUI-4` action shortcuts. |

### 3.4 Live Updates
**Need:** The graph must animate and update live as the orchestrator runs.

**Verdict:** The `Runs` tab state already listens to `StoreReply::RunStep` and `StoreReply::SessionEvent`. When a store event arrives, the UI state applies it to the `rataflow` graph model:
- New `RunStep` -> `flow.add_node()`
- Step completes -> Update node styling (e.g., border color green/red).
- `SessionEvent` (tool call) -> Update the node's internal widget state to tick the tool count.

---

## 4. Phasing (MOD spawn plan)

This analysis spawns one implementation item to be opened per `lifecycle.md`:

1. **MOD-28 - rataflow execution view:** 
   - Add `rataflow` as a dependency.
   - Implement `ExecutionGraph` widget mapping `RunStep` and `SessionEvent` lists to a node graph.
   - Add a view toggle to the Backlog `Runs` tab (`R-TUI-4`).
   - Wire mouse and keyboard events to support panning, zooming, and node selection for triggering run actions.
