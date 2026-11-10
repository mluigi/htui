# MOD-141 - One step-graph editor for Settings and the Toolbox

> HANDOFF item MOD-141 (from ANA-33, `docs/ANA-33.md` §7.10, §10). Routed **PRD** by `/handoff-run`
> (C2, C3, C4 fired; ultracode for implement and review accepted, 2026-11-10). Requirements: `R-TUI-8`,
> `R-TUI-7`, `R-TUI-10`, `R-ORCH-1`. Amends MOD-124 (its workflow editor is this component) and MOD-123
> (the `runtime_changes` default a graph may override).

## Problem

The maintainer configures how an item's work flows: which steps run, on which agents, behind which gates,
with which loops and branches. Since MOD-137 and MOD-138 a step graph has typed edges, `if` predicates,
per-edge loop limits, split modes (divide, compete), palette nodes, plan scope and three owner scopes
(project, workspace, global library), and the engine runs them. No UI writes any of it: `Settings > Kinds`
edits phases one form at a time, by position, as a straight line. A review loop limited to three rounds,
or an `if tests_failed` branch, can only be built through seeds or the store directly, and workspace and
library graphs cannot be edited at all. The v2 model is unreachable from the product.

## Evidence

- `R-TUI-8` ("with the shared graph editor"), `R-TUI-7` (the Toolbox's workflow editor) and `R-ORCH-1`
  (configurable loops and branches) require it.
- ANA-33 §7.10 and its ten §8 decisions (accepted 2026-10-18) specify the component; the UI redesign
  findings (`docs/ui-redesign-findings-2026-10-15.md` §4.6) specify MOD-124's editor behaviour.
- Observed in the tree: the v2 fields are stored (`replace_graph`, MOD-137) and validated and walked
  (`graph_body_walk_refusal`, MOD-138), but nothing in the TUI writes them. `Settings > Kinds` still labels
  the node fields "MOD-4 owns these".
- Owed from MOD-138 M1's review: two workspaces holding one project may each own a graph of one name, and
  the item and kind pickers then show two identical `name · workspace` entries and take the first typed
  match.

## Users

- **Primary**: the maintainer, shaping a project's, a workspace's or the library's workflows: adding a
  step, wiring a review loop, branching on a deterministic predicate, choosing candidate agents.
- **Not for**: agents changing a graph at run time (that is a `successors` plan, MOD-140); watching a run
  (the Runs pane, MOD-142 and MOD-120).

## Hypothesis

We believe **one rataflow step-graph editor that covers every node and edge field and is checked at save
by `resolve`'s own validator** will **let the maintainer build branching and looping workflows without
seeds or SQL** for **the maintainer configuring project, workspace and library graphs**.
We'll know we're right when **every `SnapshotNode` field and every edge field round-trips through the
editor, a graph the editor saves always resolves, and a review loop plus an `if` branch is built and saved
without leaving the TUI**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Node and edge field coverage | 100% of `SnapshotNode` fields and of edge relation, `if`, `loop_limit` | A round-trip test per field: edit in the editor, save, reload, compare |
| Saved graphs resolve | 100% | Every save path runs the shared validator; a test that a refused body never reaches `replace_graph` and an accepted one resolves |
| Out-of-TUI edits needed for the reference graph | 0 | Build "implement → review, review loops to implement at most 3, `if` branch to a fix step" in the editor alone (snapshot/e2e test) |
| Hardcoded keys in the editor | 0 | Every editor key is a catalogue action in `keys.toml` (`R-TUI-10`); the key-catalogue tests cover the new contexts |

## Scope

**MVP** — the editor component on a project graph, opened from `Settings > Kinds`: the graph drawn with
rataflow; add, delete and connect nodes; the base node form and the edge form; save through
`StoreRequest::ReplaceGraph` with the body from `GraphEntry`, refused by the shared validator with the
reason in words; in-memory undo and redo where each applied edit is one revision CAS; every key a
catalogue action.

**Decisions taken at framing (2026-11-10):**

- **Toolbox reach**: MOD-141 ships an entry point that opens the editor on a graph of any scope
  (project, workspace, global library), tested for all three. Only `Settings > Kinds` (project graphs)
  calls it from the UI; MOD-124 wires the Toolbox to it on the library and the workspace's graphs.
- **Duplicate picker entries**: qualify the label with the owning workspace's name; a typed name that
  still matches more than one graph is refused and asks for the qualified one.
- **Compete**: MOD-141 adds the later-tier compete toggle (`R-LATER-13`), off by default. Off: compete is
  not offered in the split picker; an existing compete node is shown read-only and kept unchanged on save.
- **Divide**: offered and saved now (storage and validator support it). While the divided-run runtime
  (MOD-139) is absent, a divide node carries a "runs once MOD-139 lands" note. The plan checks what
  `resolve` and the engine do with a divide node today.

**Out of scope**

- The Toolbox tab shell and its other kinds — MOD-124.
- `Settings > Workflows` and the global `runtime_changes` default — MOD-123 (this item only edits a
  graph's own override).
- Running divided steps: planner, parts, integrate — MOD-139.
- Planner tools and run-time `successors` plans — MOD-140.
- The Runs pane's parts tree and the run graph — MOD-142, MOD-120.
- Subgraph nodes — MOD-143.
- The Backlog item graph in flow mode — MOD-88, absorbed by MOD-120.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Editor core in `Settings > Kinds` | A project graph opens as nodes and edges; add, delete, connect; base node form (gate and hard flag, retry limit, isolation, command queue, verify command, token budget, template, persona, candidate agents) and edge form (relation, `if` predicate, loop limit); validated save through `replace_graph`; undo and redo; catalogue keys with per-context collisions resolved; the position-based phase forms retired; the store worker answers the written body so any scope can be edited | in-progress | `.claude/plans/mod-141-m1-step-graph-editor-core.plan.md` |
| 2 | Splits, palette and graph header | Split none, divide (planner and integrate persona and agent, default parts, `max_parts`) or compete behind the new later-tier toggle; palette flag and plan scope; the step **deadline** (moved from M1 at the plan fact-check: `GraphBody` lacks `deadline_seconds` and the judge columns, so M2 widens it); the header shows the project's bounds, tightens them and carries the graph's `runtime_changes` override | pending | — |
| 3 | Every scope, and the picker fix | The scope-generic entry point opens workspace and library graphs (MOD-124's hook); item and kind pickers qualify duplicate `name · workspace` labels and refuse an ambiguous typed match | pending | — |

## Open Questions

- [ ] Key collisions: `c`, `x`, `r` differ between Runs and the editor, `Ctrl-Z` meets pane zoom, and MOD-124's
  spec names `F9`-era keys; F9 was rejected as a reserved key. Which contexts and defaults? — settle in the M1
  plan against the key catalogue.
- [x] Undo across a CAS conflict: reload and clear undo and redo (M1 plan D4).
- [x] Layout: computed on every change from the forward edges, not stored; no node move in M1 (M1 plan D7).
- [ ] Graph-level `runtime_changes` override and bound tightening: does storage carry them on `step_graph`
  today, or does M2 need a migration? — TBD, verify in the M2 plan.
- [ ] Where the later-tier compete toggle lives (project settings rung or UI settings) and whether the engine
  honours it or only the editor does — TBD, M2 plan.
- [ ] What `resolve` and the engine do with a divide node before MOD-139 (refuse at run start, run it as a
  plain step, or park) — TBD, verify in the M2 plan; decides the note's wording.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| rataflow editing gaps (it is used read-only today: no edge creation by drag, side-handle loops) | Medium | High | Keyboard-first edits (connect by picker), mouse as a bonus; spike the connect gesture early in M1 |
| Key collisions with Runs and pane zoom regress existing bindings | Medium | Medium | New catalogue contexts, key-catalogue tests, insta snapshots of the hint lines |
| A wide UI change set breaks snapshots across tabs | High | Low | Full insta run per milestone, not a `.snap` grep (memory: snapshot impact) |
| Retiring the phase forms loses an edit path before the editor covers it | Low | High | Retire in M1 only once the base node form covers every field the old forms edited |
| Divide saved before MOD-139 confuses runs | Medium | Medium | The "runs once MOD-139 lands" note; the plan verifies today's behaviour first |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
