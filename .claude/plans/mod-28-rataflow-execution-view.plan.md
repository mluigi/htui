# Plan: MOD-28 rataflow execution view in the Runs sub-tab

**Source**: HANDOFF `MOD-28` (from ANA-12, `docs/ANA-12.md`; `R-TUI-4`)
**Routed**: plan path via `/handoff-run` (C2 fired; C4 borderline, low confidence), accepted by the maintainer 2026-10-02.
**Scope cuts (maintainer, 2026-10-02, before drafting)**: keyboard only, with **mouse deferred** to a follow-up MOD
(mouse support is still wanted, just not in this item). **Tool-call chips deferred** to a follow-up MOD. Both are
minted at close-out (T4).
**Complexity**: Medium
**Status**: done 2026-10-02 (`docs/decisions/mod/mod-28.md`)

## Summary

Today the Runs sub-tab draws every run as a flat list of step lines. MOD-28 adds a second view of the same data:
`v` switches the pane to a **flow view**, which draws the run under the cursor as a `rataflow` node graph. There is
one node per `RunStepSummary`. The graph shows fan-out candidates side by side with their judge below them, a retry
as the next layer with a `retry` edge label, and the selected winner as the one exit of a fan-out. The view is a pure
projection of the `Runs` reply (ANA-12 invariant 1). It is rebuilt from every reply, so live updates come free with
the existing `RunStream` re-read.

The list and the flow view share one cursor. The flow view's selected node *is* the list cursor, so `J`/`K`, `Enter`
and every `R-TUI-4` action key (`a x r p s A o c T u R C 1-9`) behave the same in both views (ANA-12 invariant 2,
action parity, with no new action code). Selection also moves the canvas to keep the node visible, which takes care
of ANA-12 invariant 4 (keyboard navigation). `+`/`-` zoom the canvas and `=` fits the whole run in it.

## Design decisions (proposed, maintainer may amend at CONFIRM)

| # | Decision | Why |
|---|---|---|
| D1 | **The view is called "flow" in all UI text** (`v` toggles list ↔ flow). The widget type is `ExecutionGraph`, as the item text names it. | A **Graph** sub-tab already exists (linked items, MOD-14, `detail/graph.rs`; README line 194). A second "graph" one strip away would be ambiguous. |
| D2 | **Keyboard only.** No mouse capture, no `Event::Mouse` routing. Follow-up MOD (minted at T4) owns htui-wide mouse capture plus graph click/drag/scroll. | Maintainer scope cut. htui never enables mouse capture (`terminal.rs` `init` enables raw mode, alternate screen and bracketed paste only). `App::on_terminal_event` drops everything but Key/Paste/Resize (`app/state.rs:413-423`). |
| D3 | **Tool-call chips deferred.** Nodes show only what `RunStepSummary` already carries. Follow-up MOD (minted at T4) adds a per-step tool-call count read. | Maintainer scope cut. The Runs tab has no `SessionEvent` data. `StepEvents` only serves Replay (`store_worker.rs`). |
| D4 | **`rataflow = { version = "0.1", default-features = false }`**, with **no `sugiyama`** and no `crossterm` feature. Our own layered layout sets node positions. | Without `sugiyama` the only new crate is `rataflow`. With it we would get `rust-sugiyama`, `petgraph 0.8.3`, and duplicate `hashbrown 0.15`/`foldhash 0.1`. `apply_layout` also stacks disconnected components at one origin and leaves edgeless nodes unpositioned (`rataflow/src/layout.rs:253-283`). Our layers are known: `(position, attempt)` and then judge. The mouse MOD adds the `crossterm` feature if it needs `From<crossterm::MouseEvent>`. |
| D5 | **One run at a time:** the flow view draws the run that holds the cursor entry. `J`/`K` walk the same `entries()` order as the list (slot by slot, candidates left to right), and crossing into another run swaps the graph. A run with no steps shows its run line plus "No steps yet." | Keeps the shared cursor (D7) and the existing `EntryKey` cursor survival unchanged. A multi-run canvas would need run-level layout and inter-run edges that ANA-12 never asked for. |
| D6 | **Layers and edges, inferred from `(position, attempt, fanout_index)`** (no parent link reaches the TUI; `parent_step_id` is MOD-27, still open). Steps arrive sorted `(position, attempt, fanout_index)` (`run.rs:812`). That puts a judge (`fanout_index == -1`, same `(position, attempt)` as its candidates, `engine.rs:4877-4879`) **before** its candidates, so the projection moves it explicitly. Each `(position, attempt)` gets one layer for its candidates (`fanout_index >= 0`, or the single step), plus a judge layer **after** it when there is a judge. Edges go from layer L to every node of layer L+1. The sources are **all of L** when L+1 is L's judge layer. Otherwise they are **L's winner** (`selected == Some(true)`; the field is `Option<bool>`, `run.rs:739`) when one is set, else all of L. An edge into a layer with the same `position` and a higher `attempt` is labelled `retry`. | The judge reads every candidate. After the judge (or a manual `s`) only the winner feeds the next phase. A retry is the next attempt of the same phase. The rules give a single weakly-connected DAG for every run, so nothing overlaps (D4's gotcha cannot arise). |
| D7 | **Shared cursor:** the flow view never keeps its own selection. Each sync builds the cursor step's node `with_selected(true)`, because `set_nodes` clears selection (fact-check R2). The reveal (`ensure_node_visible`) is **deferred**: sync sets a `reveal` flag, and `render` draws, applies the reveal, and draws again when the flag is set. Before the first render the canvas size is 0, so a reveal is a no-op and `center_on_selected` puts the node at the top-left corner (fact-check R6). `selected_step()`, `entry_step()` and `entry_run()` stay the single source, which gives action parity for free. | ANA-12 invariant 2 with no duplicated action code. rataflow's own key bindings (Tab = next node, arrows, hjkl = pan, **Del = delete**, `i` = toggle lock) are **never fed keys**: Tab/BackTab and the digits are global, and arrows/hjkl belong to Backlog. |
| D8 | **Flow-view keys** (only in `Mode::Browse` with the flow view shown): `v` toggles back to the list. `+`/`-` zoom in and out (`ControlsAction::ZoomIn`/`ZoomOut`, step 1.2, clamped 0.5–2.0). `=` fits the run (`ControlsAction::FitView`, applied on the next render). All other keys are the list's. In the list, `v` switches to flow. `PageUp`/`PageDown` stay the list scroll and are a no-op in flow. Flow keys never consume Tab or digits, because Runs sees them before the global keymap. | `v` is free on Runs (its only binding is in Reqs' cite picker, `detail/requirements.rs:283`). `+`/`-` are bound only by the Graph sub-tab (`graph.rs:713-714`), which is never active at the same time. `=` is bound nowhere. |
| D9 | **Read-only projection:** every node is `with_draggable(false)`, `with_connectable(false)`, `with_deletable(false)`. Its handles are baked into the `Node` builder as source Bottom / target Top, both hidden. `set_nodes` replaces nodes, so a separate `set_handles_hidden` call would be undone on every rebuild (fact-check R2). The default handles are Right/Left (R1), and the layout is vertical. Edges are not deletable, and the flow is `with_edges_reconnectable(false)`. Do **not** use `locked`, because it also blocks selection (`rataflow/src/state/event_handlers.rs:251-260`). | ANA-12 invariant 1: the graph is never a source of state. Also guards the later mouse MOD, where a click on a connectable node starts a connection instead of a click. |
| D10 | **Node content is our own `NodeContent` impl, `StepNode`.** The Flow draws no border, so `StepNode` draws its own (fact-check R4). The node is a fixed `NODE_W × NODE_H` = **20 × 4**, with `H_GAP` 1 and `V_GAP` **3**. Line 1 is the list's `super::slot()` text plus the status (e.g. `0.1/0 done`), then `✓` for the winner and `*` if promoted. The list draws those marks in its line-2 gate cell via `super::gate()`; a 2-line node has no gate cell, so they move up. Line 2 is the phase name. Text is clipped by display width with a private `clip` helper built on `cells::cell_width` + `cells::graphemes`; there is no shared clip helper, and the list's `fit` counts chars. Border style comes from htui's `Theme`: `selected` for the cursor node (`ctx.selected`), `error` for `Failed`, `accent` for `Running`/`AwaitingApproval`, `dim` for `Superseded`/`Cancelled` (a fan-out loser or a retried attempt), `base` otherwise. | The list already owns the slot text (`slot`, `runs.rs:1027`), so both views read the same. A width of 20 lets two candidates plus the gap fit 43 columns with `ensure_node_visible`'s 1-cell margin; at 21 a reveal cuts a border (R8). A 3-row gap keeps the `retry` label off the arrowhead (R7). htui's `Theme` keeps snapshots free of rataflow's default `Color::Indexed` palette. |
| D11 | **Flow layout inside the pane:** row 1 is the run's first `run_lines` line (kind · status · box · started; `run_lines(run, cancel_requested, theme)` is a free function, `runs.rs:996`). The rest is the canvas. The modal footer and `Mode::Artifact` draw exactly as in the list view. | Same chrome as the list, so every modal (reject note, cancel confirm, close-out, artifact) works unchanged in flow. |
| D12 | **Rebuild on every `Runs` reply** (`set_nodes` + `set_edges` on the kept `Flow`, with selection and hidden handles baked into the builders per D7/D9). `set_nodes` keeps zoom and offset (fact-check R2), so they stay put while the same run is shown. Switching to another run resets the view: zoom 1.0, centred on the cursor node. The initial zoom is 1.0, **not** fit, because zooming out below 1 makes node text unreadable. | Projection, never source (invariant 1). A run has a few dozen steps at most, so a full rebuild costs nothing. Keeping the viewport means a live update does not jump the canvas. |
| D13 | **Interior mutability:** `RunsTab` holds the graph as `RefCell<ExecutionGraph>`, because `DetailTab::render` takes `&self` and rataflow renders through `impl Widget for &mut Flow`. | Same pattern as the tab's existing `Cell` (`runs.rs` imports `core::cell::Cell`). No trait signature change. |
| D14 | **View state is per `RunsTab`, not per item.** It survives `on_item_change` and is not persisted, and it starts as list. | Smallest change. If the maintainer wants it remembered, that belongs with MOD-67 (configurable hotkeys/prefs). |
| D15 | **No animation, no clock:** `tick_animation` / `tick_auto_pan` are never called and no edge is animated. | Keeps renders deterministic for snapshots. rataflow reads no clock itself (`content.rs:87-92`, phase 0 until ticked). |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / module doc | `crates/htui/src/ui/tabs/backlog/detail/runs.rs:1-36` | `//!` doc with purpose, decision IDs and a key table; every constant documented |
| Sibling pure-projection sub-tab | `crates/htui/src/ui/tabs/backlog/detail/graph.rs` (MOD-14) | Pure row building from a reply, rendered with theme styles; plan-decision IDs cited inline (`plan D2`) |
| Cursor / key dispatch | `runs.rs` `on_key` `:1298-1341`, `move_cursor`, `entries()` `:299-315`, `EntryKey` | Browse-only keys; modals take every key; `Handled::Pass` for keys it cannot act on |
| Theme | `crates/htui/src/ui/theme.rs:11-23` | `base dim title accent selected error`, only via `ctx.theme` |
| Display width | `crates/htui/src/ui/cells.rs` (`cell_width`, `graphemes`) | Clip by terminal cells |
| Empty / error states | `detail/mod.rs:306-311` `message()` | Centered dim message |
| Unit tests | `runs.rs:1571+` (`Shell`, `pane()`, `lines()` on `TestBackend(43,16)`, `extremes()`), e.g. `the_step_cursor_moves_and_clamps_at_both_ends` `:1656` | Seed from `MemStore::demo()` fixtures, drive keys, assert lines |
| Snapshot tests | `crates/htui/tests/backlog.rs` (`#![cfg(feature = "testkit")]`, `Harness`), e.g. `the_reject_note_renders_under_the_parked_run` `:626`; snapshots `tests/snapshots/backlog__*.snap` | Drive the app through the harness, `insta::assert_snapshot!` |
| Fixtures | `crates/htui-core/src/fixtures.rs:1545-1585` (`RUN_3` steps: two candidates at position 0, winner `Done`+`Some(true)`, loser `Superseded`+`Some(false)`, no next layer) | Snapshot data. Projection tests build **synthetic** steps with distinct ids: no fixture has a judge or a retry (`fixtures.rs:2400` asserts attempt 1), and `extremes()` reuses one id, so it fails `set_nodes` with `DuplicateNodeId` |
| Dependencies | root `Cargo.toml:154-159` (newer entries) | A `# MOD-x Dn:` comment block above the entry; members use `{ workspace = true }` (`crates/htui/Cargo.toml:44`) |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `Cargo.toml` (workspace deps) | UPDATE | `rataflow = { version = "0.1", default-features = false }`, with a comment citing MOD-28 D4 | T1 |
| `crates/htui/Cargo.toml` | UPDATE | `rataflow = { workspace = true }` | T1 |
| `Cargo.lock` | UPDATE | `rataflow 0.1.0` only (D4) | T1 |
| `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs` | CREATE | T1: `layers()`/`project()`, a pure projection `RunSummary → (nodes, edges, positions)` with unit tests. T2: `StepNode: NodeContent`, `ExecutionGraph` (owns `Flow<StepNode>`; `sync(run, cursor_step)`, `zoom_in/out`, `fit`, `render`), unit tests on `TestBackend` | T1, T2 |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T1: `mod execution_graph;` only. T3: `View {List, Flow}`, `graph: RefCell<ExecutionGraph>`, `v`/`+`/`-`/`=` keys, the flow branch in `render`, re-sync on `on_runs` and cursor moves, module doc key table, unit tests | T1, T3 |
| `crates/htui/tests/backlog.rs` + `crates/htui/tests/snapshots/backlog__runs_flow*.snap` | UPDATE / CREATE | Flow view through the harness: toggle, a fan-out run snapshot, cursor sync across runs, an action key in flow sends the same request as in the list | T3 |
| `README.md` (Runs paragraph, ~line 204) | UPDATE | `v` flow view, `+`/`-`/`=` | T4 |
| `docs/decisions/mod/mod-28.md`, `DECISIONS.md` index, `HANDOFF.md` (item close, pins, MOD count, 2 new follow-up MODs), `docs/ANA-12.md` status cross-link | CREATE / UPDATE | Close-out bookkeeping per `lifecycle.md` P2; follow-ups minted per P0 via `scripts/hr-mint` | T4 |

## Tasks

**Order: serial, T1 → T2 → T3 → T4.** T1 and T2 share `execution_graph.rs`. T1 and T3 share `runs.rs`. T3 depends
on T2's `ExecutionGraph`. **No parallel fan-out.** One implementer runs the chain and commits after each task
(memory: implementer agents commit incrementally).

### Task 1: dependency + pure projection (TDD)
- **Action**: Add the dependency (D4). In `execution_graph.rs`, `pub(super) fn project(run: &RunSummary) -> Projection`
  produces `layers: Vec<Vec<StepId>>`, `edges: Vec<(StepId, StepId, Option<&'static str>)>` and
  `positions: Vec<(StepId, (f64, f64))>` per D6. Each layer is centred horizontally on the widest layer.
  The y step is `NODE_H + V_GAP` and the x step is `NODE_W + H_GAP`.
- **Tests first** (synthetic steps, distinct ids):
  - a linear run is a chain;
  - a 2-candidate fan-out puts its candidates in one layer, and the next layer is fed by the winner only;
  - a judge that sorts before its candidates still lands in the layer after them;
  - a fan-out with a judge sends every candidate's edge into the judge;
  - a retry layer's edge is labelled `retry`;
  - no winner yet means all candidates feed the next layer;
  - an empty run has no layers;
  - node ids are the step ids, and no two positions overlap.
- **Validate**: `cargo test -p htui --lib execution_graph`

### Task 2: `ExecutionGraph` widget (TDD)
- **Action**: `StepNode` per D10. `ExecutionGraph::sync(&mut self, run: Option<&RunSummary>, cursor: Option<StepId>, theme)`
  rebuilds per D12, with the read-only flags and hidden Bottom/Top handles of D9 and `with_selected` of D7 baked into
  the node builders, and sets the `reveal` flag. It also has `zoom_in`, `zoom_out`, `fit` and
  `render(&mut self, frame, area)`. `render` draws, applies a pending reveal, and draws again (D7). It keeps the run id
  it last drew so it can tell a new run apart (D12).
- **Tests first** (render to `TestBackend` at 43 cols):
  - the cursor node is drawn with `selected` style;
  - a failed step is drawn with `error` style;
  - the winner carries `✓` and the loser is dim;
  - text is clipped by display width (a wide-glyph phase name);
  - a re-`sync` of the same run keeps zoom and the selection, and handles stay hidden; a new run resets it;
  - a cursor node below the fold is visible on the **first** render;
  - two renders of the same state are identical (D15);
  - the flow never reports a node as draggable, connectable or deletable.
- **Validate**: `cargo test -p htui --lib execution_graph`

### Task 3: Runs tab integration (TDD)
- **Action**: `View` enum and the `RefCell<ExecutionGraph>` field (D13, D14). `v` toggles (Browse only, D8).
  `+`, `-` and `=` work in flow only. Re-sync on `on_runs`, `move_cursor` and the `v` switch into flow. Add the flow
  branch in `render` (D11), and update the module doc key table.
- **Tests first** (unit, in `runs.rs`):
  - `v` toggles and is refused in modal modes;
  - `J`/`K` in flow moves the same cursor and crosses runs;
  - `a`/`s`/`Enter` in flow send exactly what the list sends;
  - `+`/`-`/`=` are passed through in list view;
  - a `Runs` re-read keeps the cursor node selected.
- **Tests first** (integration, `tests/backlog.rs`):
  - snapshot `backlog__runs_flow_fanout` (`RUN_3`; frames are 100×30);
  - snapshot `backlog__runs_flow_reject_note`, where a modal footer is drawn under the flow;
  - a no-step run shows "No steps yet.".
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`

### Task 4: Close-out docs + follow-ups
- Mint the two follow-up MODs through `scripts/hr-mint` (sandbox, lifecycle P0). Check its sibling listing for
  duplicates first:
  1. **Mouse support:** htui-wide mouse capture policy, `Event::Mouse` routing through `Tab`/`DetailTab`, and
     click-to-select, drag-pan and scroll-zoom in the flow view. Includes the restore, panic and editor-suspend
     paths.
  2. **Tool-call chips:** a per-step tool-call count read (Postgres + MemStore + `.sqlx` + `StoreRequest`/`StoreReply`),
     drawn as chips in `StepNode`.
- Write `docs/decisions/mod/mod-28.md` (D1–D15 as decided, deviations, pins moved) and add it to the `DECISIONS.md`
  index. Update the README Runs paragraph and the `ANA-12.md` status line. Close the item in HANDOFF per
  `lifecycle.md` P2. Re-count the pins (snapshot count) instead of incrementing them.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1      # testkit, else tests/*.rs run 0 tests
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
cargo tree -p htui -i rataflow                              # one rataflow, no rust-sugiyama/petgraph
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| rataflow 0.1.0 is young (one release). API churn, or a rendering bug in narrow panes | Medium | Pin `0.1`. The projection (T1) is rataflow-free, so a swap costs only T2. T2's snapshot tests at 43 cols catch rendering regressions. |
| 43-column pane: a fan-out wider than 2 candidates doesn't fit at zoom 1 | High (by design) | Selection reveals the node (D7). `=` fits. Losers are dimmed, not hidden. Accepted for v1. |
| Zoomed-out text unreadable | Medium | Initial zoom 1.0 (D12). `fit` is opt-in. |
| `set_nodes` on a live re-read | Resolved | Fact-check R2: viewport kept, selection cleared, handles un-hidden. D7/D9 bake both into the builders; a T2 test pins it. |
| Reveal before the first render is a silent no-op | Resolved | Fact-check R6. D7's deferred reveal (draw, reveal, draw); a T2 test pins the first render. |
| A rataflow default binding leaks (Del deletes, `i` locks) | Low | D7: rataflow's key handlers are never called. Only `apply_controls_action` zoom/fit actions are used. D9 flags make deletion impossible anyway. |
| Keyboard pan missing (arrows/hjkl taken by Backlog) | Low | Selection reveal + fit cover every node. Free pan arrives with the mouse MOD. |
| Disk (`target/` growth) | Low | Check `df -h .` before the full gate (memory: dev-postgres crash = disk pressure). |

## Verified claims (step 3.5)

Probe crate: `/tmp/rfcheck`, toolchain 1.98.1, `rataflow = { version = "0.1", default-features = false }`, ratatui
`=0.30.2`. It builds with 0 warnings and pulls in only `ratatui` + `thiserror`.

| Claim | Verdict | Evidence |
|---|---|---|
| `v` is unbound on Runs, Backlog and global; its only binding is the Reqs cite picker | ✅ true | `keymap.rs:205-253`, `app/mod.rs`, `backlog/mod.rs:582-616`; only `detail/requirements.rs:283` |
| `+ - =` unbound and delivered to `RunsTab::on_key` in Browse | ✅ true | Only `graph.rs:713-714` binds `+`/`-`. Path `App::on_key` (`app/state.rs:504`) → Backlog `_ => detail.on_key` → `DetailRegistry::on_key` (`detail/mod.rs:237`) → `runs.rs:1298` |
| Steps sorted `(position, attempt, fanout_index)`; judge `-1` | ✅ true | `run.rs:812`; `mem.rs:1270`, `pg/read.rs:406`, `cache/read.rs:572` |
| The judge comes after its candidates | ❌ **false**: same `(position, attempt)`, so `-1` sorts **first** | `engine.rs:4877-4879`. D6 amended: the projection moves it |
| Winner is `selected: bool` | ❌ **false**: `Option<bool>` | `run.rs:739`. D6 amended: `== Some(true)` |
| Status variants for styling | ✅ (names fixed) | `run.rs:82-97`: `Pending Running AwaitingApproval Done Failed Cancelled Superseded`. D10 amended |
| `RUN_3` fixture at `fixtures.rs:295-302` has winner + loser and a next layer | ❌ **partly false**: ids at `:295-302`, steps at `:1545-1585`, no next layer; no judge/retry fixture anywhere (`:2400`); `extremes()` reuses one id | T1 uses synthetic steps; Patterns row amended |
| `slot()` / `step_lines` are reusable, and `✓` is on line 1 | ⚠️ partly: `slot(step, siblings) -> String` (`runs.rs:1027`) is private but reachable as `super::slot`; `✓`/`*` come from `gate()` (`:1042`) in **line 2** | D10 amended: the node puts the marks on line 1 by design |
| `cells` has a display-width clip helper | ❌ **false**: only `cell_width` (`cells.rs:45`) and `graphemes` (`:58`) | D10 amended: private `clip` in `execution_graph.rs` |
| Theme fields `base dim title accent selected error` | ✅ true | `theme.rs:11-24` |
| `run_lines` gives the run header line | ✅ true | `runs.rs:996`, `run_lines(run, cancel_requested, theme)`; line 1 = kind · status · box · started |
| `runs.rs` can own `runs/execution_graph.rs` | ✅ true | no `runs/` dir, no `#[path]` in `crates/htui/src`; `detail/mod.rs:16` `pub mod runs;` |
| `tests/backlog.rs` is testkit-gated, snapshots `backlog__<name>` | ✅ true | `:6`; `insta::assert_snapshot!("runs_reject_note", …)` `:631`; frames 100×30 |
| Every dependency carries a MOD comment | ⚠️ partly: only newer entries do | `Cargo.toml:53-68` uncommented; `:154-159` commented. The new entry follows the newer form |
| `DetailTab::render` takes `&self` | ✅ true | `detail/mod.rs` trait `fn render(&self, frame, area, ctx)` |
| `runs.rs` already uses `core::cell::Cell` | ✅ true | `runs.rs` imports |
| htui never enables mouse capture; mouse events are dropped | ✅ true | `terminal.rs` `init` (raw, alt screen, bracketed paste); `app/state.rs:413-423` |
| A "Graph" sub-tab already exists | ✅ true | `detail/graph.rs:1` (MOD-14); README:194 |
| rataflow API available with no features (Flow, `Node::new` + custom `NodeContent`, `Edge::new`, `set_nodes`/`set_edges`, `select_node`, `ensure_node_visible`, controls zoom/fit, `set_edge_label`, node flags, `with_edges_reconnectable`) | ✅ true (naming: `add_node`/`add_edge`, no `add`) | `/tmp/rfcheck` compiles. `ControlsAction::{ZoomIn, ZoomOut, ResetZoom, FitView, ToggleLock}` `actions.rs:111` |
| Default handles suit a vertical layout | ❌ **false**: source Right / target Left | D9 amended: Bottom/Top, hidden, in the builder |
| `set_nodes` keeps viewport and selection | ⚠️ partly: viewport ✅ kept (x −6.1, y −3.6, zoom 1.2 before and after); selection ❌ cleared; hidden handles ❌ reset | D7/D9/D12 amended |
| `render` needs `&mut Flow` | ✅ true | `canvas.rs:20` `impl Widget for &mut Flow`, no `&Flow` impl. D13 stands |
| The Flow draws node borders | ❌ **false**: content draws everything; `ctx.selected` available | `content.rs:79`. D10 amended |
| Default zoom 1.0; positions/sizes `f64` | ✅ true | probe |
| `ensure_node_visible` works before the first render | ❌ **false**: silent no-op at canvas 0; `center_on_selected` lands top-left | `viewport.rs:288-291`, probe R6. D7 amended: deferred reveal |
| `retry` label fits a 2-row gap | ❌ **false**: it covers the arrowhead; 3 rows show both | probe R7. D10 `V_GAP` = 3 |
| 21-wide nodes, two side by side, fit 43 columns | ⚠️ at rest yes, but a reveal (1-cell margin) cuts a border; 20 fits | probe R8. D10 `NODE_W` = 20 |
| Renders are deterministic | ✅ true | probe R9: identical buffers and styles, in-process and across fresh flows |
| Only `rataflow` is added to the lock without `sugiyama` | ✅ true | `/tmp/rfprobe2`, `/tmp/rfcheck`: deps `ratatui` + `thiserror`, both already in `Cargo.lock` |
| rataflow MSRV 1.88 ≤ toolchain | ✅ true | `rust-toolchain.toml` `1.98.1` |
| Task independence | ✅ none claimed | T1∩T2 = `execution_graph.rs`, T1∩T3 = `runs.rs`, T3 needs T2's type → serial chain, one implementer |
