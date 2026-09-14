# ANA-12 - Analyze rataflow implementation (concluded, 2026-09-14)

This item required analyzing how the `rataflow` terminal node-graph library could be implemented to visualize the execution workflow of an item in `htui`, using `zoetrope` as prior art.

**Findings and Verdict:**
We analyzed `rataflow`'s capabilities and `zoetrope`'s implementation of real-time transcript visualization. Given that `htui` is moving from strictly linear step lists toward dynamic execution trees (via review loops, fan-outs, and MOD-27 swarm spawns), a flat list (currently in `R-TUI-4`) is no longer sufficient.

To implement this without violating `htui`'s invariants (non-blocking UI, orchestrator state ownership), we decided to:
1. Augment the `Runs` tab (`R-TUI-4`) with an Execution Graph view powered by `rataflow`, rather than replacing the list view entirely.
2. Map `RunStep` rows to graph nodes, `parent_step_id` to edges, and `SessionEvent` (tool calls) to internal node chips.
3. Pipe the TUI's mouse events into `rataflow` when the graph is focused, translating `FlowEvent::NodeClicked` into node selection for standard run actions (approve, reject, retry).
4. Animate the graph live as `StoreReply::RunStep` and `StoreReply::SessionEvent` events arrive from the orchestrator.

The full design and mapping is recorded in `docs/ANA-12.md`.

**Spawned Items:**
- **MOD-28**: rataflow execution view
