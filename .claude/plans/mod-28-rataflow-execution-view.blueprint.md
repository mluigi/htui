# Blueprint: MOD-28, rataflow flow view in the Runs sub-tab

**Status**: **proposed** (2026-10-02). Plan deviations B-1 to B-10 (§0) and blueprint decisions
E1–E16 (§6) belong to this blueprint. A deviation marked **Blocker** means the plan, read literally,
produces a defect its own tests or snapshots would show. The Fix column is what the implementer
builds. The plan's D1–D15 are binding and are not reopened. Where a fix narrows one of them, the
row cites the evidence.

**Plan**: `.claude/plans/mod-28-rataflow-execution-view.plan.md` at `40abe2e9`, **confirmed**
2026-10-02, fact-checked (step 3.5, probe `/tmp/rfcheck`). Design source: `docs/ANA-12.md`
(invariants 1, 2, 4). Tasks are cited as "MOD-28 T*n*" outside this file.

**Verified at**: HEAD `40abe2e9`, branch `hr/MOD-28`, clean tree. Every `htui` anchor was read
through Gortex (`read`, `search`). The rataflow source isn't indexed, so it was read at
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rataflow-0.1.0/src`. **Line numbers are
pre-edit**: once a task commits to a file, a citation into that file moves. A second probe,
`/tmp/rfcheck28` (rataflow 0.1.0, `default-features = false`, ratatui `=0.30.2`, target dir shared
with `/tmp/rfcheck`), built and ran this blueprint's exact calls. It confirmed the following. Node
handles baked into the builder survive `set_nodes`. Selection baked with `with_selected` survives
too. Viewport `x=-4.30 y=-14.30 z=1.20` is unchanged across a rebuild. The scratch-buffer reveal
shows a below-the-fold cursor on the first frame. The `retry` label sits above the arrowhead. The
`Frame::render_widget(&mut flow, area)` path works. Two renders of one state are equal, styles
included. Counted at this HEAD: `crates/htui/tests/snapshots` has **131** tracked files
(`HANDOFF.md:48-49` still says 129, B-9). `df -h .` shows 2.7 T free (82 %).

**Graphify**: `graphify-out/` doesn't exist in this checkout, so nothing here comes from it.

**Coupling verdict.** The plan's order stands: **serial T1 → T2 → T3 → T4, one implementer, no
fan-out**. T1 and T2 share `runs/execution_graph.rs`. T1 and T3 share `runs.rs` (T1 adds only the
`mod` line). T3 needs T2's `ExecutionGraph`. No file joins a task's list beyond the plan's, except
the one new probe-free helper in `tests/backlog.rs` (E14), which T3 already owns.

**Scope**:
- **No migration, no `.sqlx`, no store or seam change, no new `StoreRequest`/`StoreReply`
  variant.** The flow view only projects the existing `Runs` reply.
- **New dependency**: `rataflow 0.1.0`, no default features, which brings `ratatui` and
  `thiserror` (both already locked). No `rust-sugiyama`, no `petgraph` (D4).
- **New code**: `runs/execution_graph.rs` (`project`, `StepNode`, `ExecutionGraph`, all
  `pub(super)`), plus `View` and two fields on `RunsTab`.
- **Pins that move** (T4 re-counts, and does not add to the HANDOFF numbers):
  `crates/htui/tests/snapshots` 131 → **133** (`backlog__runs_flow_fanout.snap`,
  `backlog__runs_flow_reject_note.snap`; no existing snapshot changes, because the list view
  is untouched). MOD count +2 for the two follow-ups minted at T4. Unmoved: `StoreRequest` 96,
  `StoreReply` 55, store `CASES` 119, `.sqlx` 307.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_debug_implementations` warns (`Cargo.toml:170-173`),
  `clippy::all` warns, and the gate runs `cargo clippy --workspace --all-targets --all-features
  -- -D warnings`. rustdoc denies broken and private intra-doc links, so a `pub(super)` item's doc
  can't link a private fn of another module. A doc written before its target exists uses plain
  backticks.
- Implementers commit after each step and stage only their own paths: never `-A`, never `stash`,
  never `--amend`. Every commit compiles. A red commit may put a `todo!()` body **only** in an
  item that no existing product path calls (H-6).
- Integration tests need `--features testkit` or `--all-features`. Without them, `tests/*.rs` runs
  0 tests and still reports ok. Every `htui` gate uses `--test-threads=1` (the keyring fake is
  process-wide).
- UI code never panics in a render path and never logs (no `tracing` in `ui/`). A malformed reply
  renders as something rather than aborting (`figure`'s doc, `runs.rs:883-884`).

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree / rataflow at `40abe2e9` | Fix |
|---|---|---|---|---|
| **B-1** | **Blocker** (the T3 fan-out snapshot would show a cut node) | D12: "Switching to another run resets the view: zoom 1.0, centred on the cursor node." | `center_on` sets `viewport.x = canvas_w/2 − node_center_x·zoom` (`types/viewport.rs:142-145`, called by `center_on_selected`, `state/viewport.rs:77-89`). For `RUN_3` the cursor starts on candidate 0 (world x 0..20, centre 10) in a 43-column canvas, so `x = 21.5 − 10 = 11.5`. Candidate 1 (world 21..41) then lands at columns 32.5..52.5 and is cut at 43. D10's own R8 geometry ("20 fits 43 with the 1-cell margin") was measured from the **origin + `ensure_node_visible`** (`/tmp/rfcheck/src/main.rs:104-112`), not from a centre. | E5: a **Reset** reveal. After the first (scratch) pass, the viewport is set to `Viewport::new(max(1, ⌊(canvas_w − run_w)/2⌋), 1, 1.0)`, which centres the run's widest layer horizontally with a 1-row top margin at zoom 1. Then the cursor is revealed with `ensure_node_visible`. `RUN_3`: `x = max(1, ⌊(43−41)/2⌋) = 1`, which is exactly the R8 geometry (probe frame in §3.6). A one-node run sits centred at column 11. D12's "zoom 1.0, not fit" and "reset on another run" are kept. |
| **B-2** | **Blocker** (ghost borders after any reveal that pans) | D7: "`render` draws, applies the reveal, and draws again." | Drawing twice into the same `Frame` buffer leaves the first pass's cells wherever the second pass doesn't overwrite them. The canvas never clears its area. Edges are composited onto the frame cell by cell, non-empty cells only (`ui/canvas.rs:20-63`, `composite_cells(&edge_buf, …, false)` at `:57`). Nodes overwrite only their own new rectangle (`:340`). A reveal that pans by `dy` leaves the old borders behind. | E6: the first pass draws into a throwaway `Buffer::empty(area)`. Its only job is to set the canvas size (`render_context.set_canvas_area`, `canvas.rs:37`) and run a pending fit (`:40`). Then the reveal applies, and only the second pass draws into the frame. The probe ran this exact sequence (§3.6). |
| **B-3** | Non-blocker (a lost reset) | D7: "sync sets a `reveal` flag." | A `bool` flag can't tell "new run, reset the viewport" from "same run, keep it on screen". When `J` crosses into another run and a `Runs` re-read lands before the next frame, the second (same-run) sync would downgrade the pending reset. The new run would then be drawn at the old run's offset. | E4: `enum Reveal { None, Cursor, Reset }`, derived `Ord`. `sync` does `self.reveal = self.reveal.max(new)`, and `render` `take`s it. |
| **B-4** | Non-blocker (palette leak) | D10: "htui's `Theme` keeps snapshots free of rataflow's default `Color::Indexed` palette." | That holds for nodes only. `StepEdge::render` falls back to `EdgeStyle::default()` (`ui/builtins.rs:435-444`), and `render_path` resolves its stroke and label to `palette.muted`/`palette.text` (`content.rs`, `EdgeRenderContext::render_path`), which are `Color::Indexed(237)`/`(231)` (`theme.rs`, `Palette::DARK`). | E8: every edge carries `StepEdge::default().with_style(EdgeStyle::default().with_stroke_style(theme.dim).with_label_style(theme.accent))` (`edge_render.rs:267`, `:285`). Handles are hidden, so they never draw. No rataflow palette colour reaches the buffer. |
| **B-5** | Non-blocker (totality) | T2: `sync` calls `set_nodes` + `set_edges`. | Both return `Result<(), rataflow::Error>` (`state/graph.rs:376`, `:418`): `DuplicateNodeId`, `DuplicateEdgeId`, `SelfReferentialEdge`, `InvalidEdgeReference`. The store never repeats a step id, but a reply is data, and the existing unit fixture `extremes()` (`runs.rs:1928-1964`) reuses one id 6 times. | E3: `layers()` keeps the **first** step of a repeated id. Edges only go from layer L to layer L+1 (distinct ids), and an edge id is `"{src}>{dst}"` (unique per pair). So neither call can fail on a projection. `sync` still handles an `Err` totally: it clears the flow and forgets the run, so the canvas is drawn empty, with no panic and no log. T1 pins this with `a_repeated_step_id_is_kept_once`. |
| **B-6** | Non-blocker (clippy in per-task gates) | T1 adds `mod execution_graph;` with `project`. T2 adds `ExecutionGraph`. T3 wires it. | Between commits, `project` (T1) and `ExecutionGraph` (T2) have no non-test caller. `dead_code` fires, and `clippy -D warnings` fails the task gate. | E15: one **module-level** inner attribute at the top of `execution_graph.rs`, added in T1: `#![cfg_attr(not(test), expect(dead_code, reason = "MOD-28 T3 wires the flow view into RunsTab"))]`. It covers every not-yet-called item (`layers`, `project`, and in T2 `StepNode`, `Reveal`, `ExecutionGraph`, `handles`, `edge`, `clip`). T3's green commit removes it, when `sync`/`render` gain callers. `expect` warns when the expectation goes unmet, so a stale one can't survive the T3 gate. |
| **B-7** | Non-blocker (one field) | T1: `Projection { layers, edges, positions }`. | B-1's reset needs the run's width in world units, and only the projection knows the widest layer. | `Projection` gains `width: f64` (`max_layer_len·NODE_W + (max_layer_len−1)·H_GAP`, 0 for an empty run). |
| **B-8** | Non-blocker (mark order) | D10: line 1 is "slot plus status, then `✓` for the winner and `*` if promoted." | `gate()` (`runs.rs:1041-1053`) writes `*` **before** `✓` (`rejected*✓`, pinned by `every_step_row_fits_forty_three_columns`, `runs.rs:2020-2025`). D10's reason for reusing `slot()` is that "both views read the same". | E10: marks follow `gate()`'s order: a space, then `*` if promoted, then `✓` if the winner (`0.1/0 done ✓`, `0.1/0 done *✓`). |
| **B-9** | Non-blocker (bookkeeping) | Plan: T3 adds 2 snapshots. HANDOFF pins 129. | `git ls-files crates/htui/tests/snapshots \| wc -l` = **131** at this HEAD. `HANDOFF.md:48-49` is stale by 2. | T4 writes **133** after re-counting, not 129 + 2. |
| **B-10** | Non-blocker (test placement) | Files table: the integration tests cover "cursor sync across runs". | No demo item has two runs (`fixtures.rs:1404-1467`: `RUN_1` FEAT-1, `RUN_2` FEAT-3, `RUN_3` ANA-1), so a harness test can't cross runs without seeding a second run. The plan's T3 integration list itself doesn't name it. | Crossing runs is a **unit** test (`j_and_k_in_flow_move_the_same_cursor_across_runs`, §4.5) on two synthetic runs. The plan's three integration tests stay, and E14 adds a fourth (action parity through the harness). |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | A rataflow default binding leaks: `Del` deletes, `i` locks, `Tab` cycles, arrows/`hjkl` pan (`state/event_handlers.rs`). | `RunsTab` never calls `handle_key_event`, `handle_controls_key_event`, `apply` or `handle_mouse_event`. The flow's only inputs are `set_nodes`, `set_edges`, `zoom_in`, `zoom_out`, `request_fit_view`, `viewport =` and `ensure_node_visible`. D9 flags make deletion impossible anyway. T2 `no_node_is_draggable_connectable_or_deletable` pins the flags. |
| **H-2** | The reveal is a silent no-op before the first draw (`state/viewport.rs:288-291`: canvas 0 → return). | B-2/E6: every reveal runs after a scratch pass in the same `render`. T2 `a_cursor_below_the_fold_is_visible_on_the_first_render`. |
| **H-3** | `set_nodes` clears selection and un-hides handles (fact-check R2). | D7/D9: `with_selected(id == cursor)` and `with_handles([source Bottom hidden, target Top hidden])` are baked into every `Node` builder (`types/node.rs:186`, `:257`; `types/handle.rs:211`, `:216`, `:271`). T2 `a_re_sync_of_the_same_run_keeps_zoom_selection_and_hidden_handles`. |
| **H-4** | The input order of `RunSummary.steps` isn't guaranteed in tests. `a_parked_fanout_shows_its_candidates_and_s_picks_one` (`runs.rs:2674-2724`) feeds `[0, 1, -1]`, with the judge **last**. The store sorts the judge **first** (`-1`). | `layers()` groups by `(position, attempt)` in a `BTreeMap` and sorts each group by `(fanout_index, id)`, so both orders yield the same layers. T1 `the_projection_does_not_depend_on_the_input_order`. |
| **H-5** | `RefCell` double borrow. | `render(&self)` takes the only runtime `borrow_mut`. Every `&mut self` path also goes through `borrow_mut()` (E12), and none of them is reachable while `render` holds its borrow (single-threaded, no re-entry). |
| **H-6** | A red commit routes a live path to a `todo!()`. | T1's and T2's red bodies are reached only by the new tests. T3's red commit adds only the `View`/`graph` fields (behaviour-neutral, nothing toggles) and test-only accessors. |
| **H-7** | Review loops that go back to an earlier position (R-ORCH-3) draw in `(position, attempt)` order, not chronologically. `(2,1) → (3,1) → (2,2)` shows as `(2,1) →retry (2,2) → (3,1)`. | Accepted and inherent to D6: no parent link reaches the TUI (`parent_step_id` is MOD-27). T4 notes it in `mod-28.md` as a MOD-27 follow-up remark. |
| **H-8** | Zooming in around the canvas centre (`state/viewport.rs:29-42`) can push the cursor node off screen. | E9: `zoom_in` and `zoom_out` also raise `Reveal::Cursor`. `fit` doesn't, because a fit shows every node. |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles) | Gate (`--test-threads=1` on every `htui` test run) |
|---|---|---|---|
| T1 dependency + projection | `Cargo.toml`, `crates/htui/Cargo.toml`, `Cargo.lock`, `runs/execution_graph.rs` (new), `runs.rs` (one `mod` line) | 2 (§2.6) | `cargo test -p htui --lib execution_graph -- --test-threads=1`; `cargo clippy -p htui --all-targets --all-features -- -D warnings`; `cargo tree -p htui -i rataflow` |
| T2 `ExecutionGraph` widget | `runs/execution_graph.rs` | 2 (§3.8) | `cargo test -p htui --lib execution_graph -- --test-threads=1`; clippy `-p htui` |
| T3 Runs integration | `runs.rs`, `tests/backlog.rs`, 2 new `.snap` | 2 (§4.7) | `cargo test -p htui --lib runs -- --test-threads=1`; `cargo test -p htui --features testkit --test backlog -- --test-threads=1`; `cargo insta pending-snapshots` empty after accept; snapshots = **133**; then `cargo test -p htui --all-features -- --test-threads=1` |
| T4 close-out | `README.md`, `docs/decisions/mod/mod-28.md`, `DECISIONS.md`, `HANDOFF.md`, `docs/ANA-12.md` | 1 (+ the mint leases) (§5) | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | — | the plan's whole Validation block (§7), on the real tree |

---

## 2. T1: dependency and pure projection (D4, D6)

### 2.1 Dependency

`Cargo.toml`: append after `url = "2.5"` (`:168`), before `[workspace.lints.rust]` (`:170`). Use
the newer commented form (`:155-168`):

```toml
# MOD-28 D4: the Runs pane's flow view (ANA-12). No default features: `sugiyama` would pull in
# rust-sugiyama, petgraph and second copies of hashbrown/foldhash, and our layered layout sets
# every position itself. `crossterm` is for the mouse follow-up. Adds one crate; its two
# dependencies (`ratatui`, `thiserror`) are already locked.
rataflow               = { version = "0.1", default-features = false }
```

`crates/htui/Cargo.toml`: after `sha2 = { workspace = true }` (`:72`), before `[dev-dependencies]`:

```toml
# MOD-28 D4: the flow view of the Runs pane (`runs/execution_graph.rs`).
rataflow               = { workspace = true }
```

`Cargo.lock`: let cargo add `rataflow 0.1.0` only. Check: `cargo tree -p htui -i rataflow` shows one
`rataflow v0.1.0`, and `cargo tree -p htui | grep -E 'sugiyama|petgraph'` is empty.

### 2.2 Module wiring (`runs.rs`, T1 part)

Insert at `runs.rs:38` (the blank line after the module doc, before `use core::cell::Cell;` at
`:39`): `mod execution_graph;` followed by a blank line. That's the whole T1 change to `runs.rs`.
The file is `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs`. There's no
`#[path]` anywhere in `crates/htui/src`, and `detail/mod.rs:16` declares `pub mod runs;`.

### 2.3 Module header, imports, constants

```rust
//! MOD-28: the Runs pane's flow view, the run under the cursor drawn as a `rataflow` graph.
//!
//! A pure projection of one `RunSummary` (ANA-12 invariant 1). [`project`] lays the steps out in
//! layers (plan D6), and `ExecutionGraph` owns the `Flow` that draws them. The flow keeps no
//! selection of its own: the cursor step is rebuilt selected on every sync (plan D7), and no key
//! ever reaches rataflow's own bindings (blueprint H-1).

use std::collections::{BTreeMap, BTreeSet};

use htui_core::model::{RunId, RunStepSummary, RunSummary, StepId, StepStatus};
use rataflow::{
    Edge, EdgeStyle, Flow, Handle, HandlePosition, Node, NodeContent, NodeRenderContext,
    StepEdge, Viewport,
};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget as _};

use crate::ui::Theme;
use crate::ui::cells::{cell_width, graphemes};

/// Plan D10: a node's width in cells at zoom 1. Two candidates and the gap are 41 columns, which a
/// reveal's 1-cell margins keep inside the 43-column pane (fact-check R8; 21 cuts a border).
pub(super) const NODE_W: f64 = 20.0;
/// Plan D10: a node's height, a border, two lines and a border.
pub(super) const NODE_H: f64 = 4.0;
/// Plan D10: the columns between two nodes of one layer.
pub(super) const H_GAP: f64 = 1.0;
/// Plan D10: the rows between two layers. Three keep a `retry` label off the arrowhead (R7).
pub(super) const V_GAP: f64 = 3.0;
/// The x distance from one node of a layer to the next.
const X_STEP: f64 = NODE_W + H_GAP;
/// The y distance from one layer to the next.
const Y_STEP: f64 = NODE_H + V_GAP;
/// Plan D6: the label of an edge into the next attempt of the same position.
pub(super) const RETRY: &str = "retry";
/// The margin `ensure_node_visible` keeps (`rataflow` `state/viewport.rs:299`), reused as the
/// reset viewport's left floor and top offset (blueprint E5).
const MARGIN: f64 = 1.0;
```

(Imports used only by T2 items arrive in T2: `Frame`, `Buffer`, `Rect`, `Style`, `Block`, `Widget`,
`Viewport`, `Edge`, `EdgeStyle`, `Flow`, `Handle`, `HandlePosition`, `Node`, `NodeContent`,
`NodeRenderContext`, `StepEdge`, `cells`, plus **`ControlsAction`** for §3.3's zoom and fit. T1
imports only what T1 compiles against: `BTreeMap`, `BTreeSet`, and the `htui_core::model` types.)

### 2.4 Types and functions

```rust
/// One row of the flow (plan D6): the candidates of a `(position, attempt)`, or its judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layer {
    /// `run_step.position` of every step in it.
    pub(super) position: i32,
    /// `run_step.attempt` of every step in it.
    pub(super) attempt: i32,
    /// Whether this is the judge layer (`fanout_index < 0`) of its `(position, attempt)`.
    pub(super) judge: bool,
    /// Its steps, left to right: by `fanout_index`, then id.
    pub(super) steps: Vec<StepId>,
    /// The step marked `selected == Some(true)`, if any (`run.rs:739` is `Option<bool>`).
    pub(super) winner: Option<StepId>,
}

/// What [`project`] makes of a run.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Projection {
    /// The layers, top to bottom.
    pub(super) layers: Vec<Layer>,
    /// `(source, target, label)`, in layer order and then left to right.
    pub(super) edges: Vec<(StepId, StepId, Option<&'static str>)>,
    /// Every node's top-left corner in world units, in layer order and then left to right.
    pub(super) positions: Vec<(StepId, (f64, f64))>,
    /// The widest layer's width in world units, `0.0` for a run with no step (blueprint B-7).
    pub(super) width: f64,
}

/// Plan D6: a run's steps grouped into layers, each id once (blueprint B-5, H-4).
pub(super) fn layers(steps: &[RunStepSummary]) -> Vec<Layer>

/// Plan D6: a run as layers, edges and positions. Rataflow-free, so T1 tests it alone.
pub(super) fn project(run: &RunSummary) -> Projection
```

The module's first line after the `//!` doc is B-6's bridge:
`#![cfg_attr(not(test), expect(dead_code, reason = "MOD-28 T3 wires the flow view into RunsTab"))]`.

### 2.5 The algorithm (D6), as pseudo-code

```text
layers(steps):
    seen   := BTreeSet<StepId>
    groups := BTreeMap<(position, attempt), (candidates: Vec<&Step>, judges: Vec<&Step>)>
    for step in steps:                          # input order is irrelevant (H-4)
        if !seen.insert(step.id): continue      # B-5: the first of a repeated id wins
        g := groups.entry((step.position, step.attempt)).or_default()
        if step.fanout_index < 0: g.judges.push(step)      # -1 is the judge (engine.rs:4877-4879)
        else:                     g.candidates.push(step)  # >= 0: candidates, or the single step
    out := []
    for ((p, a), (cands, judges)) in groups:    # BTreeMap: ascending (position, attempt)
        sort cands  by (fanout_index, id)
        sort judges by (fanout_index, id)
        if cands  non-empty: out.push(Layer{p, a, judge: false, steps: ids(cands),
                                          winner: first id in cands  with selected == Some(true)})
        if judges non-empty: out.push(Layer{p, a, judge: true,  steps: ids(judges),
                                          winner: first id in judges with selected == Some(true)})
        # the judge layer comes AFTER its candidates even though -1 sorts first (D6)
    return out

project(run):
    L := layers(&run.steps)
    if L empty: return Projection{ layers: [], edges: [], positions: [], width: 0.0 }
    span(n) := n·NODE_W + (n−1)·H_GAP           # n >= 1
    width   := max over L of span(len(layer.steps))
    positions := []
    for (i, layer) in L.enumerate():
        x0 := ((width − span(len(layer.steps))) / 2).floor()   # centred on the widest, whole cells
        y  := i · Y_STEP
        for (k, id) in layer.steps.enumerate():
            positions.push((id, (x0 + k·X_STEP, y)))
    edges := []
    for (from, to) in L.windows(2):
        sources := if to.judge && (to.position, to.attempt) == (from.position, from.attempt):
                       from.steps                          # the judge reads every candidate
                   else if let Some(w) = from.winner:
                       [w]                                 # after a judge or `s`, only the winner feeds on
                   else:
                       from.steps                          # no winner yet: all of L
        label   := if to.position == from.position && to.attempt > from.attempt:
                       Some(RETRY)
                   else: None
        for s in sources: for t in to.steps: edges.push((s, t, label))
    return Projection{ layers: L, edges, positions, width }
```

Properties T1 pins: ids in `positions` are exactly the distinct step ids. No two node rectangles
`[x, x+NODE_W) × [y, y+NODE_H)` intersect. Every edge goes from layer i to layer i+1, so there are
no self-loops, and every endpoint is in `positions`. The result is one weakly-connected DAG.

### 2.6 Tests (first) and commits (T1)

All tests go in `#[cfg(test)] mod tests` in `execution_graph.rs`. Fixtures are synthetic, with
distinct deterministic ids (not `extremes()`, B-5):

```rust
use uuid::Uuid;
use htui_core::fixtures::demo_at;
use htui_core::model::{BoxId, ProjectId, RunKind, RunMode, RunStatus};

/// A settled step at `(position, attempt, fanout_index)`, id `0x100 + n`
/// (the `htui-orch` `closeout.rs:253` idiom).
fn step(n: u128, position: i32, attempt: i32, fanout_index: i32) -> RunStepSummary {
    RunStepSummary {
        id: StepId::from_uuid(Uuid::from_u128(0x100 + n)),
        position, attempt, fanout_index,
        phase_name: "plan".to_owned(),
        agent_id: None, model: None,
        status: StepStatus::Done,
        gate_outcome: None, started_at: None, finished_at: None,
        prompt_tokens: None, trimmed: false, usage: None,
        selected: None, exit_code: None, verify_outcome: None,
        promoted_at: None, agent_name: None, gate_note: None,
    }
}
/// `step` with `selected: Some(won)` and `status` Done / Superseded.
fn candidate(n: u128, position: i32, attempt: i32, index: i32, won: Option<bool>) -> RunStepSummary
/// A running graph run `0x900 + n` holding `steps` as given (order not normalised).
fn run(n: u128, steps: Vec<RunStepSummary>) -> RunSummary {
    RunSummary {
        id: RunId::from_uuid(Uuid::from_u128(0x900 + n)),
        item_id: None, project_id: ProjectId::default(),
        kind: RunKind::Graph, mode: RunMode::Manual, status: RunStatus::Running,
        target_box_id: BoxId::default(), executing_box_id: None,
        box_hostname: "box".to_owned(), queued_at: demo_at(0, 0),
        started_at: None, finished_at: None, failure: None, steps,
    }
}
fn id(n: u128) -> StepId
```

(`RunStepSummary`/`RunSummary` fields: `crates/htui-core/src/model/run.rs:703-751`, `:784-814`.
`htui-core` is a dependency with feature `demo`, so `fixtures::demo_at` is reachable, as in
`runs.rs:1580`.)

| Test | Fixture | Pins |
|---|---|---|
| `a_linear_run_is_a_chain` | steps at positions 0, 1, 2, attempt 1, index 0 | 3 layers of 1. Edges `[(0→1, None), (1→2, None)]`. Positions `(0,0)`, `(0,7)`, `(0,14)`. `width == 20.0` |
| `a_fanout_puts_its_candidates_in_one_layer_and_only_the_winner_feeds_the_next` | p0 candidates index 0 (`Some(false)`) and 1 (`Some(true)`), then p1 single | layer 0 = `[c0, c1]`, winner `c1`. Edges into p1 = `[(c1→p1)]` only. p1 at x `⌊(41−20)/2⌋ = 10` |
| `a_judge_sorted_before_its_candidates_lands_in_the_layer_after_them` | store order `[judge(-1), c0, c1]` at `(0,1)` | layers = `[[c0, c1] judge:false, [j] judge:true]` |
| `every_candidate_feeds_the_judge` | same plus `c0` marked winner | edges `c0→j`, `c1→j` (all of L, despite the winner), then `j→next` |
| `a_retry_layer_is_fed_by_an_edge_labelled_retry` | `(1,1)` Failed, `(1,2)` Running | one edge `(a1→a2, Some("retry"))`. Also a fan-out with judge at `(1,1)` followed by `(1,2)`: the edge from the judge is labelled `retry` |
| `with_no_winner_yet_every_candidate_feeds_the_next_layer` | candidates `selected: None` (and one `Some(false)`), then a next layer | edges from both candidates |
| `an_empty_run_has_no_layers` | `run(…, vec![])` | all four fields empty, `width == 0.0` |
| `node_ids_are_step_ids_and_no_two_nodes_overlap` | a 3-wide fan-out + judge + retry + single, 8 steps | `positions` ids == the step id set. Pairwise rectangle intersection is empty. Every edge endpoint is in `positions`, and no edge has `source == target` |
| `the_projection_does_not_depend_on_the_input_order` | the same 4 steps in store order (`-1` first) and in `runs.rs:2679`'s order (`-1` last), and reversed | `project` is equal for all three |
| `a_repeated_step_id_is_kept_once` | two steps with id `n = 1` (different positions) | one node. The first step's position wins |
| `layers_are_centred_on_the_widest` | 3 candidates, then a single | single at x `⌊(62−20)/2⌋ = 21`. 2-wide under 3-wide at x 10 and 31 |

Commits:
1. `test(mod-28): rataflow dependency and projection tests (red)`: §2.1, §2.2, and §2.3–§2.4 with
   `layers`/`project` bodies `todo!("MOD-28 T1")`, plus the tests. Red, and nothing outside the
   tests calls them (H-6).
2. `feat(mod-28): project a run into flow layers, edges and positions`: §2.5. Gate green (§1).

---

## 3. T2: the `ExecutionGraph` widget (D7, D9, D10, D12, D13, D15)

**File**: `runs/execution_graph.rs` (only).

### 3.1 `StepNode` (D10)

```rust
/// Plan D10: one step's node. The flow draws no border (`rataflow` `content.rs:79`), so this does.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StepNode {
    /// Line 1: `super::slot()`, the status, then `*`/`✓` (blueprint B-8).
    head: String,
    /// Line 2: the phase name.
    phase: String,
    /// What colours the border.
    status: StepStatus,
    /// The pane's theme, copied: `NodeContent::render` sees only rataflow's context.
    theme: Theme,
}

impl StepNode {
    /// The node of `step`, one of `siblings` (its run's steps, for the slot's `/i` and `/j`).
    pub(super) fn new(step: &RunStepSummary, siblings: &[RunStepSummary], theme: &Theme) -> Self
    /// The border: `selected` on the cursor, else by status.
    fn border(&self, selected: bool) -> Style
    /// The text: `dim` for a node whose border is `dim`, `base` otherwise.
    fn text(&self) -> Style
}

impl NodeContent for StepNode {
    fn render(&self, ctx: &NodeRenderContext, buf: &mut Buffer)
}

/// `text` in at most `width` terminal cells, by grapheme, cut with `…` (blueprint E11).
fn clip(text: &str, width: usize) -> String
```

- `head` is `format!("{} {}{}", super::slot(step, siblings), super::step_status(step.status),
  marks)`, where `marks` is `""`, or a space followed by `*` (if `promoted_at.is_some()`) and then
  `✓` (if `selected == Some(true)`). Both `slot` (`runs.rs:1026`) and `step_status` (`:981`) are
  private to `runs.rs`, and a child module may call them as `super::…`.
- `phase` is `step.phase_name.clone()`.
- `border(selected)`: if `selected`, `theme.selected` (REVERSED). Otherwise by `status`: `Failed`
  gives `theme.error`. `Running | AwaitingApproval` gives `theme.accent`. `Superseded | Cancelled`
  gives `theme.dim`. `Pending | Done` gives `theme.base`. Write the match out, with no wildcard,
  so a new `StepStatus` has to be placed (`run.rs:82-97`).
- `render`: the probe-verified shape.
  ```rust
  let block = Block::bordered().border_style(self.border(ctx.selected));
  let inner = block.inner(ctx.area);
  block.render(ctx.area, buf);
  for (row, line) in (0u16..).zip([&self.head, &self.phase]) {
      if row >= inner.height { break; }
      buf.set_string(inner.x, inner.y + row, clip(line, usize::from(inner.width)), self.text());
  }
  ```
  `ctx.area` is local `(0,0)`-based and is the node's full size at the current zoom
  (`canvas.rs:320-332`). Zoom 0.5 gives a 10×2 node with no interior, and the loop draws
  nothing; the probe shows a bare box. A zoom-2.0 node is 40×8 and draws 2 lines.
- `clip` (no shared helper exists; `runs.rs`'s `fit` counts `char`s):
  ```text
  flat := text with every control char mapped to ' '      # one line is one line (fit's rule)
  if cell_width(flat) <= width: return flat
  if width == 0: return ""
  out := ""; used := 0
  for g in graphemes(flat):
      w := cell_width(g)
      if used + w > width − 1: break                      # leave one cell for the cut mark
      out += g; used += w
  return out + super::CUT                                  # '…' (runs.rs:139), 1 cell
  ```
  A wide glyph that would straddle the last cell is dropped, so the result can be `width − 1`
  cells. It is never `width + 1`.

### 3.2 Handles and edges (D9, B-4)

```rust
/// Plan D9: one source on the bottom and one target on the top, both hidden. Baked into every
/// builder, because `set_nodes` resets anything set after it (fact-check R2).
fn handles() -> Vec<Handle> {
    vec![
        Handle::source(HandlePosition::Bottom).with_hidden(true),
        Handle::target(HandlePosition::Top).with_hidden(true),
    ]
}

/// One projected edge, drawn in the pane's own styles (blueprint B-4).
fn edge(source: StepId, target: StepId, label: Option<&'static str>, theme: &Theme)
    -> Edge<StepEdge>
{
    let content = StepEdge::default().with_style(
        EdgeStyle::default()
            .with_stroke_style(theme.dim)
            .with_label_style(theme.accent),
    );
    let edge = Edge::new(format!("{source}>{target}"), source.to_string(), target.to_string())
        .with_content(content)
        .with_deletable(false)
        .with_selectable(false);
    match label { Some(label) => edge.with_label(label), None => edge }
}
```

(`Edge::new` `types/edge.rs:130`, `with_content` `:155`, `with_deletable` `:199`,
`with_selectable` `:205`, `with_label` `:223`. `StepEdge::with_style` `ui/builtins.rs:399`.
`StepId: Display` is the hyphenated UUID, from the `ids.rs` `id_newtype!` macro.)

### 3.3 `ExecutionGraph` (D7, D12, D13)

```rust
/// Blueprint E4: what the next `render` does to the viewport before it draws.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Reveal {
    /// Nothing.
    #[default]
    None,
    /// Keep the cursor node on screen (`ensure_node_visible`, plan D7).
    Cursor,
    /// A new run: zoom 1, the run centred, then the cursor kept on screen (plan D12, B-1).
    Reset,
}

/// Plan D7/D12/D13: the flow view of one run, rebuilt from every sync.
#[derive(Debug)]
pub(super) struct ExecutionGraph {
    /// The canvas. Never fed a key or a mouse event (blueprint H-1).
    flow: Flow<StepNode, StepEdge>,
    /// The run last synced, which tells a new run apart (plan D12).
    run: Option<RunId>,
    /// The cursor step last synced, the node a reveal keeps on screen.
    cursor: Option<StepId>,
    /// The run's widest layer in world units (B-7), for the reset's centring.
    width: f64,
    /// The pending viewport move (B-3).
    reveal: Reveal,
}

impl Default for ExecutionGraph {
    fn default() -> Self {
        Self {
            flow: Flow::new().with_edges_reconnectable(false),   // D9 (state/mod.rs:703)
            run: None, cursor: None, width: 0.0, reveal: Reveal::None,
        }
    }
}

impl ExecutionGraph {
    /// Plan D7/D12: rebuilds the flow from `run`, `cursor` selected. `None` clears it.
    pub(super) fn sync(&mut self, run: Option<&RunSummary>, cursor: Option<StepId>, theme: &Theme)
    /// Plan D8: `+`, one `ZOOM_STEP` (1.2) in, clamped to 2.0. The cursor stays on screen (E9).
    pub(super) fn zoom_in(&mut self)
    /// Plan D8: `-`, one step out, clamped to 0.5 (E9).
    pub(super) fn zoom_out(&mut self)
    /// Plan D8: `=`, the whole run fitted on the next render (`request_fit_view`).
    pub(super) fn fit(&mut self)
    /// Plan D7, blueprint B-2/E6: draws `area`, revealing first when a reveal is pending.
    pub(super) fn render(&mut self, frame: &mut Frame<'_>, area: Rect)
    /// Test-only views for `runs.rs`'s tests (the fields are private to this module).
    #[cfg(test)] pub(super) fn shown_run(&self) -> Option<RunId> { self.run }
    #[cfg(test)] pub(super) fn selected(&self) -> Option<String> { self.flow.first_selected_node_id() }
    #[cfg(test)] pub(super) fn zoom(&self) -> f64 { self.flow.viewport.zoom }
}
```

**`sync` body**:

```text
let Some(run) = run else { clear(); return }      # clear: set_nodes(vec![]) (never fails),
                                                   #        set_edges(vec![]), run = None,
                                                   #        cursor = None, reveal = None
let projection = project(run)
let nodes = projection.positions.iter().filter_map(|(id, (x, y))| {
    let step = run.steps.iter().find(|s| s.id == *id)?        # always Some
    Some(Node::new(id.to_string(), (*x, *y), (NODE_W, NODE_H), StepNode::new(step, &run.steps, theme))
        .with_selected(Some(*id) == cursor)                   # D7 (types/node.rs:186)
        .with_draggable(false)                                # D9 (:204)
        .with_connectable(false)                              # D9 (:221)
        .with_deletable(false)                                # D9 (:198)
        .with_handles(handles()))                             # D9 (:257); source/target pos unused
}).collect()
let edges = projection.edges.iter().map(|(s, t, l)| edge(*s, *t, *l, theme)).collect()
if self.flow.set_nodes(nodes).and_then(|()| self.flow.set_edges(edges)).is_err() {
    clear(); return                                           # unreachable by construction (B-5)
}
let next = if self.run == Some(run.id) { Reveal::Cursor } else { Reveal::Reset }
self.run = Some(run.id); self.cursor = cursor; self.width = projection.width
self.reveal = self.reveal.max(next)                           # B-3
```

Never call `locked` (`with_locked`/`toggle_lock`): it blocks selection
(`state/event_handlers.rs:251-260`, D9). Never call `tick_animation`/`tick_auto_pan` (D15).

**`zoom_in`/`zoom_out`/`fit`**:
`self.flow.apply_controls_action(ControlsAction::ZoomIn / ZoomOut / FitView)`
(`state/event_handlers.rs:576-600`; `ControlsAction` is `actions.rs:111`, non-exhaustive, so
match nothing on it). Ignore the returned `EventResponse` (`let _ =`). `ZoomIn` is
`zoom_in()`, which zooms around the canvas centre by `ZOOM_STEP = 1.2` clamped to
`[0.5, 2.0]` (`state/viewport.rs:8-14`, `:29-42`). `FitView` is `request_fit_view()`
(`:171`), which is applied inside the next draw (`canvas.rs:40`) and re-applied until the canvas
size is stable (`:189-201`). After a zoom: `self.reveal = self.reveal.max(Reveal::Cursor)` (E9).

**`render` body** (the D7 deferred reveal, B-2):

```rust
let reveal = core::mem::take(&mut self.reveal);
if reveal != Reveal::None {
    // The canvas size is known only after a draw (state/viewport.rs:288-291), so the first pass
    // goes to a throwaway buffer (blueprint B-2): drawing twice into the frame leaves ghosts.
    let mut scratch = Buffer::empty(area);
    (&mut self.flow).render(area, &mut scratch);           // canvas.rs:20 / :37 / :40
    if reveal == Reveal::Reset {
        let x = ((f64::from(area.width) - self.width) / 2.0).floor().max(MARGIN);
        self.flow.viewport = Viewport::new(x, MARGIN, 1.0); // zoom 1, plan D12 (B-1)
    }
    if let Some(cursor) = self.cursor {
        self.flow.ensure_node_visible(&cursor.to_string()); // state/viewport.rs:287-322
    }
}
frame.render_widget(&mut self.flow, area);                   // impl Widget for &mut Flow
```

### 3.4 Data flow (T2 alone)

`RunSummary` → `project` → `(Node<StepNode>, Edge<StepEdge>)` builders → `set_nodes`/`set_edges`
on the kept `Flow`, which keeps its viewport (R2, probe `x=-4.30 y=-14.30 z=1.20` before and
after) → `render` → scratch pass → reveal → frame pass.

### 3.5 Tests (first), `TestBackend` at 43 columns

Helpers, in the same `mod tests`:

```rust
/// One frame of `graph` at 43×23 (the 100×30 frame's Runs canvas: 24 rows less the header row).
fn draw(graph: &mut ExecutionGraph) -> Buffer {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(43, 23))
        .expect("the test backend is constructible");
    term.draw(|frame| graph.render(frame, frame.area())).expect("the graph draws");
    term.backend().buffer().clone()
}
/// Each row as a `String`, trailing blanks trimmed (as `runs.rs`'s `lines`, `:1729-1743`).
fn rows(buf: &Buffer) -> Vec<String>
/// The `(x, y)` of the first `┌` on the row holding `needle`'s node, for style checks.
fn corner_of(buf: &Buffer, needle: &str) -> (u16, u16)
```

| Test | Pins |
|---|---|
| `the_cursor_node_is_drawn_selected` | Linear 2-step run, cursor on step 1. The cursor node's `┌` has `Modifier::REVERSED` (`theme.selected`), and the other node's doesn't |
| `a_failed_step_is_drawn_with_the_error_style` | A `Failed` step, not the cursor: its corner's `fg == Color::Red` (`Theme::default().error`) |
| `the_winner_carries_a_check_and_the_loser_is_dim` | The `RUN_3` shape, synthetic (winner index 0 `Done` `Some(true)`, loser index 1 `Superseded` `Some(false)`, phase `research`), cursor on the winner. Rows contain `0.1/0 done ✓` and `0.1/1 superseded`. The loser's corner and text are `fg DarkGray` (`theme.dim`) |
| `a_promoted_winner_reads_star_then_check` | B-8: `promoted_at: Some(..)` + `Some(true)` gives `0.1 done *✓` |
| `text_is_clipped_by_display_width` | Phase `"調査フェーズの長い名前です"` (wide). The interior row's text has `cell_width ≤ 18` and ends in `…`, and the node's right `│` is at the same column as on its border rows. Plus `clip` unit cases: `("ab", 5) → "ab"`, `("abcdef", 4) → "abc…"`, `("日本語", 4) → "日…"`, `("a\nb", 3) → "a b"`, `(_, 0) → ""` |
| `a_re_sync_of_the_same_run_keeps_zoom_selection_and_hidden_handles` | sync, draw, `zoom_in`, draw. Record `viewport`. Re-sync the same run with one status changed: `viewport` is equal, `first_selected_node_id() == Some(cursor.to_string())`, and every node's `handles.iter().all(\|h\| h.hidden)`. Then sync another run and draw: `viewport.zoom == 1.0` |
| `a_new_run_is_centred_at_zoom_one` | B-1: a one-node run in 43 columns has its `┌` at column 11, row 1. The `RUN_3` shape has corners at columns 1 and 22, both nodes whole |
| `a_cursor_below_the_fold_is_visible_on_the_first_render` | A fresh graph, 8-step linear run, cursor on the last step: the **first** `draw` contains `7.1 done` (H-2) |
| `a_retry_edge_is_drawn_with_its_label` | `(1,1)` Failed → `(1,2)` Running: some row contains `retry`, and a row below it contains `▼` |
| `two_renders_of_the_same_state_are_identical` | D15: `draw(g) == draw(g)`, styles included (`Buffer: PartialEq`) |
| `no_node_is_draggable_connectable_or_deletable` | Every `flow.nodes()` has `!draggable && !connectable && !deletable`, every `flow.edges()` has `!deletable`, and `!flow.edges_reconnectable` |
| `a_sync_of_no_run_clears_the_canvas` | `sync(None, …)`, then `flow.nodes().count() == 0`, and `draw` is blank |

### 3.6 Probe frames (`/tmp/rfcheck28`, what the T2/T3 tests will see)

`RUN_3` after the Reset (`x=1 y=1 z=1`), 43×23, first rows:
```
|                                           |
| ┌──────────────────┐ ┌──────────────────┐ |
| │0.1/0 done ✓      │ │0.1/1 superseded  │ |
| │research          │ │research          │ |
| └──────────────────┘ └──────────────────┘ |
```
Fan-out + judge + retry, cursor on the last node, first render (it reveals by `y=-10`):
```
|          ┌──────────────────┐             |
|          │2.1 failed        │             |
|          │implement         │             |
|          └──────────────────┘             |
|                    │                      |
|                  retry                    |
|                    ▼                      |
|          ┌──────────────────┐             |
|          │2.2 running       │             |
```

### 3.7 Clippy notes

- `(0u16..).zip([..])` avoids a `usize → u16` cast.
- `f64::from(area.width)` is lossless.
- `#[derive(PartialEq)]` on `StepNode` needs `Theme: PartialEq`, which holds (`theme.rs:10`).

### 3.8 Commits (T2)

1. `test(mod-28): ExecutionGraph and StepNode tests (red)`: §3.1–§3.3 with `todo!("MOD-28 T2")` in
   `sync`, `render`, `StepNode::render` and `clip`, and the tests. The module-level `expect`
   (B-6) stays. Red, and nothing in the product calls them.
2. `feat(mod-28): ExecutionGraph draws a run as a read-only flow`: the bodies. Gate green.

---

## 4. T3: Runs tab integration (D5, D7, D8, D11, D13, D14)

**Files**: `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/tests/backlog.rs`,
`crates/htui/tests/snapshots/backlog__runs_flow_{fanout,reject_note}.snap`.

### 4.1 Imports and constants (`runs.rs`)

- `:39` `use core::cell::Cell;` becomes `use core::cell::{Cell, RefCell};`.
- After `:63` (`use crate::ui::text_field::…`): `use self::execution_graph::ExecutionGraph;`.
- After `WAITING_LINE` (`:101-102`):
  ```rust
  /// MOD-28 D5: what the flow view says under a run with no step yet.
  const NO_STEPS_YET: &str = "No steps yet.";
  ```
- Module doc key table: add after `:28`:
  ```text
  //! | `v` | pane | the list or the flow view of the cursor's run (MOD-28 D1, D8) |
  //! | `+` / `-` / `=` | flow | zoom in, zoom out, fit the run (MOD-28 D8) |
  ```
  Then add one paragraph after the MOD-42 paragraph (`:30-33`): "MOD-28: `v` draws the run under
  the cursor as a flow (`execution_graph.rs`). The flow's selected node *is* the cursor, so every
  key above acts the same in both views (ANA-12 invariant 2). `PageUp`/`PageDown` do nothing there."

### 4.2 Types and fields

After `EntryKey` (`:213-220`), before `Mode` (`:222`):

```rust
/// MOD-28 D1, D14: which view of the runs the pane draws. Per pane, not per item, never persisted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum View {
    /// The run list (MOD-4 D197).
    #[default]
    List,
    /// The cursor's run as a `rataflow` graph (MOD-28).
    Flow,
}
```

`RunsTab` (`:161-194`), after `waiting` (`:193`):

```rust
    /// MOD-28 D14: list or flow. Starts as list, and an item change leaves it alone.
    view: View,
    /// MOD-28 D13: the flow, behind a `RefCell` because `DetailTab::render` takes `&self` and
    /// rataflow draws through `impl Widget for &mut Flow` only.
    graph: RefCell<ExecutionGraph>,
```

`#[derive(Debug, Default)]` on `RunsTab` (`:161`) still holds, because `ExecutionGraph:
Default + Debug` (`Flow` has a manual `Debug`, `state/mod.rs:446`). `on_item_change`
(`:1280-1289`) is **not** changed (D14). The next `Runs` reply re-syncs, and the new run id
triggers a Reset.

### 4.3 Functions

In the first `impl RunsTab` (`:277-623`), after `move_cursor` (`:338-346`):

```rust
/// MOD-28 D7, D12: rebuilds the flow from the cursor's run, the cursor's step selected. Only
/// while the flow is shown: the list never pays for it, and `v` syncs on the way in.
fn sync_graph(&self, theme: &Theme) {
    if self.view == View::Flow {
        self.graph.borrow_mut().sync(self.entry_run(), self.selected_step(), theme);
    }
}
```

`&self` plus `borrow_mut` (E12) lets `entry_run()`'s borrow of `self.runs` sit next to the
graph's borrow. `on_key` and `on_runs` call it from their `&mut self` with no borrowck conflict.

In the second `impl RunsTab` (`:1489-1569`), after `footer`:

```rust
/// MOD-28 D11: the flow branch of `render`. Row 1 is the run's first `run_lines` line, and the
/// rest is the canvas, or "No steps yet." under a run with none.
fn render_flow(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let Some(run) = self.entry_run() else {
        return;
    };
    let [head, canvas] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let line = run_lines(run, self.cancel_requested(run.id), theme)
        .into_iter()
        .next()
        .unwrap_or_default();
    frame.render_widget(Paragraph::new(line), head);
    if run.steps.is_empty() {
        message(frame, canvas, NO_STEPS_YET, theme);
        return;
    }
    self.graph.borrow_mut().render(frame, canvas);
}
```

### 4.4 Re-sync points, key arms, render branch (pre-edit lines)

- **`on_runs`** (`:368-388`): insert `self.sync_graph(ctx.theme);` right after the `self.selected =
  …;` statement (`:372-378`), **before** the `let Some(item) = self.item else { return; };` early
  return at `:379`. A reply with no item still re-syncs.
- **`on_key`** `match key.code` (`:1328-1341`). The `J`/`K` arms (`:1329-1330`) become:
  ```rust
  KeyCode::Char('J') => {
      self.move_cursor(1);
      self.sync_graph(ctx.theme);
  }
  KeyCode::Char('K') => {
      self.move_cursor(-1);
      self.sync_graph(ctx.theme);
  }
  ```
  Insert before the action arm (`:1337`):
  ```rust
  KeyCode::Char('v') => {
      self.view = match self.view {
          View::List => View::Flow,
          View::Flow => View::List,
      };
      self.sync_graph(ctx.theme);
  }
  KeyCode::Char('+') if self.view == View::Flow => self.graph.get_mut().zoom_in(),
  KeyCode::Char('-') if self.view == View::Flow => self.graph.get_mut().zoom_out(),
  KeyCode::Char('=') if self.view == View::Flow => self.graph.get_mut().fit(),
  // MOD-28 D8: the list's page keys do nothing in the flow, and leave the list's scroll alone.
  KeyCode::PageUp | KeyCode::PageDown if self.view == View::Flow => {}
  ```
  `get_mut` needs no runtime borrow (`&mut self`). Every arm falls through to
  `Handled::Consumed` (`:1342`). In the list view, `+ - =` reach `_ => self.scroll.on_key`
  (`:1340`), which returns `Pass` (`detail/mod.rs` `Scroll::on_key`), so they still pass on (D8).
  The modal check (`:1299-1301`) and the `CONTROL | ALT` pass (`:1302-1307`) come first,
  unchanged. In a modal `v` is typed or swallowed. The digit block (`:1311-1327`) is unchanged.
  `Tab`/`BackTab` were never matched and still pass.
- **`render`** (`:1445-1486`): insert after the `if self.runs.is_empty() { … return; }` block
  (`:1469-1472`), before the list header (`:1474`):
  ```rust
  if self.view == View::Flow {
      self.render_flow(frame, list, ctx.theme);
      return;
  }
  ```
  The footer (`:1458-1468`) and `Mode::Artifact` (`:1450-1456`) are drawn as in the list (D11).

### 4.5 Tests (first): unit tests in `runs.rs` `mod tests` (after `a_new_item_takes_the_cursor_with_it`, `:3505-3521`)

The fixtures reuse `Shell`, `pane()`, `driven()`, `verdicts()`, `allowed_emit()`-style driving,
and `lines()` (43×16) from `:1589-1743`, `:2328-2422`. Synthetic second runs follow
`a_re_read_keeps_the_cursor_on_its_step` (`:3474-3478`): `feat_1_runs().await.remove(0)`, then
`id = RunId::new()` and every step `id = StepId::new()`.

| Test | Pins |
|---|---|
| `v_toggles_the_flow_view_and_is_typed_in_a_modal` | `v` gives `Consumed`, then `view == Flow`, then `v` gives `List`. In `x`'s note `v` is typed (the footer shows `v`) and `view` is unchanged |
| `j_and_k_in_flow_move_the_same_cursor_across_runs` | Two runs (FEAT-1's, plus a copy with fresh ids). In flow, `J`×4 crosses into run 2: `selected_step()` is run 2's first step, `graph.borrow().shown_run() == Some(run2.id)`, and `graph.borrow().selected() == Some(step.to_string())`. `K` comes back |
| `action_keys_in_flow_send_what_the_list_sends` | For `(a, 1)`, `(s, 3)`, `(Enter, 1)`, `(r, 0)`: the one action emitted in list view and in flow view (`v` first, drained) have equal `format!("{:?}")` (D7, ANA-12 inv. 2) |
| `plus_minus_and_equals_pass_in_the_list_view` | each gives `Handled::Pass`. In flow, each gives `Consumed`, and `+` raises `graph.borrow().zoom()` above 1.0 |
| `page_keys_are_a_no_op_in_flow` | in flow `PageDown` gives `Consumed`, and `scroll` is unchanged (`first_visible()` the same) |
| `a_runs_re_read_keeps_the_cursor_node_selected` | `driven`, `v`, `J J` (on `implement`). Re-read with a stepless newer run first (as `:3474-3479`): `graph.borrow().selected() == Some(implement.to_string())` |
| `the_flow_view_draws_the_run_line_then_the_canvas` | `lines()` in flow: row 0 starts with `graph` (the run's kind cell), and some row contains `0.1 done` (the PRD step's slot and status) |
| `a_run_with_no_steps_says_so_in_flow` | runs = `[stepless, FEAT-1's]`, cursor on the stepless run, `v`: row 1 is `No steps yet.` |
| `the_flow_view_survives_an_item_change` | D14: `v`, `on_item_change(ANA_2)`, then `view == Flow` |

### 4.6 Tests (first): integration in `tests/backlog.rs`

New section after `the_runs_pane_greys_a_key_with_the_guard_s_sentence` (`:740-761`), before
the Graph section (`:763`):

```rust
// ---------------------------------------------------------------------------------------------
// The Runs pane's flow view (MOD-28 plan D1-D15).
// ---------------------------------------------------------------------------------------------
```

| Test | Setup | Pins / snapshot |
|---|---|---|
| `v_draws_the_fan_out_run_as_a_flow` | `backlog()`, `sub_tab(1)` (the arrival row is `ANA-1`, which owns only `RUN_3`, `fixtures.rs:1446-1465`), `key("v")` | frame contains `0.1/0 done ✓` and `0.1/1 superseded`. **`insta::assert_snapshot!("runs_flow_fanout", frame)`** → `backlog__runs_flow_fanout.snap`. Then `v`: frame contains `kind   status` again (list header) |
| `the_reject_note_renders_under_the_flow` | `parked()` (`:577-595`), `key("v")`, `key("x")`, `type_text("needs work")`, `drive` | frame contains `reject with a note:` and `research`. **`insta::assert_snapshot!("runs_flow_reject_note", …)`** → `backlog__runs_flow_reject_note.snap` |
| `a_run_with_no_steps_says_so_in_the_flow` | `MemStore::demo()` plus `create_run(NewRun { id: RunId::new(), project_id: ids::PROJECT_HTUI, item_id: ids::HTUI_ANA_2, mode: RunMode::Manual, target_box_id: ids::BOX, started_by: ids::USER, graph_snapshot: bare_snapshot(), repo_scope: Vec::new(), queued_at: demo_at(2, 8) })`, then `polled(store)` (no run runtime, so the queued run never gets a step), `down(TO_ANA_2)`, `sub_tab(1)`, `v` | frame contains `No steps yet.` and `queued`. No snapshot |
| `an_action_key_in_flow_answers_as_in_the_list` | as `the_runs_pane_greys_a_key_with_the_guard_s_sentence`, with `key("v")` before `a` | `harness.app().status == Some(sentence)` (D7 parity through the real worker, E14) |

`bare_snapshot()` copies `crates/htui-agent/tests/relay.rs:80-100` (`GraphSnapshot { v:
GraphSnapshot::V, graph: SnapshotGraph { id: ids::GRAPH_HTUI_FEAT, … }, phases: Vec::new(), … }`).
The imports added to `:24-29` are `GraphSnapshot, Isolation, NewRun, RunId, RunMode,
SnapshotGraph, SnapshotSettings` (all `htui_core::model`).

### 4.7 Commits (T3)

1. `test(mod-28): Runs flow view tests (red)`: §4.2 (`View`, the two fields; nothing toggles, so
   it's behaviour-neutral) and the §4.5/§4.6 tests. The module-level `expect(dead_code)` stays,
   because `sync`/`render` still have no caller. Red: `v` isn't bound, and the
   snapshots are pending.
2. `feat(mod-28): v draws the cursor's run as a flow in the Runs pane`: §4.1, §4.3, §4.4, the
   module-level `expect(dead_code)` removed from `execution_graph.rs` (B-6), plus the two
   accepted snapshots, reviewed in `cargo insta review`. Check that the `fanout` frame shows
   the run line, two whole boxes at columns 1 and 22 of the pane and no `kind status` header.
   Check that `reject_note` shows the centred `research` node above the three footer rows. Gate:
   the `backlog` suite, `runs` unit tests, then `cargo test -p htui --all-features --
   --test-threads=1`. `ls crates/htui/tests/snapshots | wc -l` = **133**, and `git status
   --porcelain crates/htui/tests/snapshots` shows exactly two `??`.

### 4.8 Data flow (whole feature)

`StoreReply::Runs` → `RunsTab::on_runs` (cursor kept by `EntryKey`, D198) → `sync_graph`
(flow view only) → `ExecutionGraph::sync(entry_run, selected_step, theme)` → `project` → rebuilt
nodes and edges (selected and hidden handles baked in) + `Reveal` raised. Keys: `J`/`K` move
`selected` → `sync_graph`. Action keys read `entry_step()`/`entry_run()` exactly as in the list,
because the graph is never consulted for state (ANA-12 inv. 1, 2). `RunsTab::render` (`&self`) →
`render_flow` → `graph.borrow_mut().render` → scratch pass → reveal → frame pass. Live updates
come for free: `RunStream` frames re-read `Runs` (`:1384-1404`), and every reply re-syncs.

---

## 5. T4: close-out docs and follow-ups

1. Mint the two follow-ups through `scripts/hr-mint --prefix MOD --title "…"` (lifecycle P0).
   Read its stderr sibling list for duplicates first.
   (a) **Mouse support**: htui-wide mouse capture (`terminal.rs` `init` + restore/panic/editor
   paths), `Event::Mouse` routing through `Tab`/`DetailTab` (`app/state.rs:413-423` drops it
   today), and click-to-select, drag-pan and scroll-zoom in the flow view. That MOD adds rataflow's
   `crossterm` feature if it needs it (D4).
   (b) **Tool-call chips**: a per-step tool-call count read (Postgres + MemStore + `.sqlx` +
   `StoreRequest`/`StoreReply`), drawn as chips in `StepNode`.
   Add both to `HANDOFF.md` as open items in the existing style.
2. `docs/decisions/mod/mod-28.md`: D1–D15 as decided, B-1..B-10, E1–E16, H-7 (no chronology
   without MOD-27's parent link), pins moved.
3. `DECISIONS.md`: one index line at the top (`:5` style), `- **[MOD-28](docs/decisions/mod/mod-28.md)**
   - Rataflow flow view in the Runs sub-tab (done, <date>)`.
4. `README.md:204` Runs paragraph: add "`v` switches to a flow view of the run under the cursor
   (`+`/`-` zoom, `=` fits the run); every Runs key works the same there."
5. `docs/ANA-12.md:7` status line: append "MOD-28 done (keyboard, list ↔ flow); mouse and
   tool-call chips are MOD-<a>/MOD-<b>."
6. `HANDOFF.md`: tick `:238` per `lifecycle.md` P2. Re-count pins at `:46-49` (`snapshots` **133**,
   B-9; add "MOD-28" to the "Pins after" list). Update the MOD-N row at `:614`: drop
   "MOD-28 rataflow", add the two new MODs, and re-count the total.
7. Commit: `docs(mod-28): close-out - write-up, DECISIONS index, HANDOFF (status, pins, MOD count),
   README, ANA-12 status`. Run §7 on the real tree before this commit.

---

## 6. Blueprint decisions

| # | Decision | Why |
|---|---|---|
| **E1** | `execution_graph.rs` is a child of `runs.rs`, and all its API is `pub(super)` | The plan's file. Being a child gives it `super::slot`, `super::step_status` and `super::CUT` without widening their visibility |
| **E2** | `layers()` uses a `BTreeMap<(position, attempt), _>`, groups sorted by `(fanout_index, id)`, and any `fanout_index < 0` counts as a judge | H-4: independent of input order. `< 0` is total over `i32` (only `-1` occurs, `engine.rs:4877-4879`) |
| **E3** | The first step of a repeated id wins. `sync` clears the flow on an impossible `Err` | B-5: no panic and no log in a render path |
| **E4** | `Reveal { None < Cursor < Reset }`, combined with `max`, taken by `render` | B-3 |
| **E5** | Reset = zoom 1, the widest layer centred (`x = max(1, ⌊(w−width)/2⌋)`), `y = 1`, then the cursor revealed | B-1: keeps D12's intent (zoom 1, a fresh view per run) and D10's R8 geometry |
| **E6** | The first pass goes to a scratch `Buffer`, and only the second reaches the frame | B-2 |
| **E7** | Projection positions are whole cells (`floor` of the centring offset) | A cell boundary never falls inside a node at zoom 1. Snapshot borders are integral |
| **E8** | Edges are `StepEdge` styled `theme.dim` (stroke) and `theme.accent` (label), not deletable, not selectable | B-4. A dim stroke keeps the nodes the focus. The accent label makes `retry` legible |
| **E9** | `+`/`-` raise `Reveal::Cursor`, and `=` doesn't | H-8: D7's "selection keeps the node visible" survives a zoom around the centre |
| **E10** | Line 1 marks follow `gate()`: ` *✓` | B-8 |
| **E11** | Private `clip` by grapheme and cell width, cut with `super::CUT` | D10. The list's `fit` counts `char`s, and `cells` has no clip (fact-check) |
| **E12** | `sync_graph(&self)` uses `RefCell::borrow_mut`. The key arms use `get_mut` | Disjoint borrows without restructuring `entry_run()`. `render` is the only other runtime borrow (H-5) |
| **E13** | The flow syncs only while it is shown, and `v` into flow syncs | The list path stays byte-identical, so no existing snapshot moves |
| **E14** | A fourth integration test, `an_action_key_in_flow_answers_as_in_the_list` | Action parity through the real worker, cheaply, by reusing `:740-761`'s setup |
| **E15** | One module-level `#![cfg_attr(not(test), expect(dead_code, reason))]` bridges T1→T3 | B-6. One attribute instead of one per item. `expect` forces its own removal |
| **E16** | Two commits per code task (red, then green) and one docs commit | Memory: implementers commit incrementally. H-6 keeps red `todo!()`s off live paths |

## 7. Close-out gate (plan § Validation, on the real tree)

```bash
df -h .                                                     # target/ growth (memory: disk pressure)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui --all-features -- --test-threads=1      # testkit, else tests/*.rs run 0 tests
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
cargo tree -p htui -i rataflow                              # one rataflow, no rust-sugiyama/petgraph
ls crates/htui/tests/snapshots | wc -l                      # 133
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
