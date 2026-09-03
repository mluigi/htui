# MOD-1 - TUI scaffold

> HANDOFF item: `MOD-1` (routed **PRD** by `/handoff-run`, 2026-09-03; C2 + C4 fired; ultracode
> accepted for implement and review). Requirements: `R-TUI-1..3`, `R-NF-1`, `R-NF-3`.
> Design authority: `docs/ANA-9.md` takes priority over every other document for data shapes and
> the store seam; `docs/REQUIREMENTS.md` and `CONCEPTS.md` frame the rest.

## Problem
`htui` has a full product contract (`docs/REQUIREMENTS.md`), a concluded data model
(`docs/ANA-9.md`) and zero code. Every downstream module (MOD-2 driver, MOD-4 orchestrator, MOD-6
store, MOD-7 box, MOD-13..15 spawned by this item) needs a running terminal application to land in
and a store seam to land behind. Without the scaffold, each of those items would invent its own
shell and the seam would be settled by whichever ships first instead of by ANA-9.

## Evidence
- Assumption — needs validation via prototype: run the scaffold with seeded in-memory data on
  Windows, Linux and macOS and check every `R-TUI-1..3` surface is reachable by keyboard.
- `HANDOFF.md` status line (2026-09-03): "No code yet ... MOD-1 and MOD-6 can start now."
- `docs/ANA-9.md` §9: "MOD-1 builds against `MemStore`; the trait in §6.1 is the seam."

## Users
- **Primary**: the single developer of `htui` (`R-USR-1`), opening the tool on any of their boxes
  to browse the backlog of a workspace. Trigger: a session start on any box.
- **Not for**: teams (`R-USR-3`, later), agents (no MCP surface in this item), anyone expecting to
  create or run items (deferred to MOD-4, MOD-12, MOD-13).

## Hypothesis
We believe **a keyboard-driven shell whose every view reads through the ANA-9 §6.1 store seam**
will **let MOD-2, MOD-4, MOD-6, MOD-7 and MOD-13..15 land as additive modules** for **the
developer**.
We'll know we're right when **MOD-6 replaces the in-memory store with the Postgres and cache
backends without touching a single view, and every later tab or action slots into an existing
extension point instead of reshaping the shell**.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Cross-platform build (`R-NF-1`) | Builds and runs on Windows 10+, Linux, macOS | CI matrix or manual run on the three OSes |
| Surface coverage (`R-TUI-1..3`) | Top bar, tab strip, workspace switcher overlay, Backlog list grouped by project, five detail sub-tabs, Skills and Settings stubs all reachable by keyboard | Manual walkthrough with seeded data; one snapshot test per surface |
| UI never blocks (`R-NF-3`) | No store call on the render path; store calls run on the runtime and deliver results by message | Code review plus a test that a slow store leaves input responsive |
| Seam fidelity (ANA-9 §6.1) | `ReadStore` / `WriteStore` / `Backend` shape matches §6.1; domain types mirror §5 columns | Review checklist against ANA-9 sections |
| Extensibility | Adding a tab, a detail sub-tab, an overlay or an action requires no edit to the event loop or the store traits | Reviewer confirms against the blueprint; MOD-13/14/15 item text maps to named extension points |

## Scope
**MVP** — a skeleton. The binary starts, restores the terminal on every exit path, shows the top
bar (workspace, box, Postgres state, active run count) and the tab strip (Backlog, Skills,
Settings; Chat reserved). Backlog tab: left pane lists items of the current workspace grouped by
project; right pane shows the selected item under Body, Runs, Graph, Documents and Notes sub-tabs
as **read-only** views of what the store returns. Skills and Settings tabs are placeholders.
Workspace switcher overlay lists workspaces and switches scope. All data comes from `MemStore`
loaded with demo fixtures. Everything visible is keyboard-driven.

Maintainer decisions (2026-09-03) that bound the MVP:
- ANA-9 has priority over anything; where the item text and ANA-9 differ, ANA-9 wins.
- Skeleton only: no filters, no item actions, no editing, no graph traversal, no run actions.
  Those are tracked as MOD-13 (filters and editing), MOD-14 (Graph tab), MOD-15 (hierarchy
  management), MOD-4 (run, close, Runs tab actions), MOD-12 (queue) — all opened or amended in
  `HANDOFF.md` on 2026-09-03.
- The TUI scope is always a workspace; the user creates a workspace even for a single project.
  The `R-ENT-2` no-workspace fallback stays a store-level concept and is not surfaced.
- Code must stay extensible: every tab, sub-tab, overlay and action is a registration point, not
  a match arm in the shell.

**Out of scope**
- Postgres, SQLite cache, keyring, offline mode — MOD-6.
- Chat tab, agent driver, agent registry Settings section — MOD-2.
- Runs tab actions, close-out, run records — MOD-4.
- Queue overlay, caps and scheduler Settings section — MOD-12.
- Backlog filters, new/edit item, divergence view, notes append, hand-written documents — MOD-13.
- Graph tab traversal and re-rooting — MOD-14.
- Workspace, project, repo and kind management, Settings sections for kinds and graphs — MOD-15.
- Box probe and box profile Settings section — MOD-7.
- Secret provider Settings section — MOD-10.
- Skills tab content — MOD-9.
- Mouse support beyond what the terminal backend gives for free (`R-TUI-1`: mouse optional).
- Persisting UI state (last workspace, last tab) — no store for it until MOD-6.
- Legacy markdown import — MOD-8.

## Delivery Milestones
<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Shell boots | Binary starts on the three OSes, draws top bar and tab strip, handles quit and terminal restore, and has the off-thread work pattern in place (`R-NF-1`, `R-NF-3`) | in-progress | `.claude/plans/mod-1-tui-scaffold.plan.md` |
| 2 | Store seam | ANA-9 §6.1 traits and `Backend`, domain types mirroring §5, `MemStore` with demo fixtures and a trait-level test suite MOD-6 will reuse | in-progress | `.claude/plans/mod-1-tui-scaffold.plan.md` |
| 3 | Backlog tab | Grouped item list per project of the current workspace; Body, Runs, Graph, Documents, Notes read-only sub-tabs fed through `ReadStore` (`R-TUI-2..3` skeleton) | in-progress | `.claude/plans/mod-1-tui-scaffold.plan.md` |
| 4 | Scope and stubs | Workspace switcher overlay changes scope and re-queries; top bar fields fed from app state; Skills and Settings placeholder tabs (`R-TUI-1` skeleton) | in-progress | `.claude/plans/mod-1-tui-scaffold.plan.md` |

## Open Questions
Resolved by the maintainer on 2026-09-03:
- [x] `R-ENT-2` stays as written (store-level rule); `docs/REQUIREMENTS.md` is not amended. The
      TUI never offers the no-workspace fallback.
- [x] Demo fixtures ship behind a `--demo` flag; without it `MemStore` starts empty.
- [x] Windows Terminal is the target; legacy conhost is best-effort.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Seam drifts from ANA-9 §6.1 while the doc is only prose | Medium | MOD-6 has to re-cut the trait | Plan fact-check verifies trait signatures against §6.1 line by line; reviewer checklist |
| Views reach into the store synchronously "just for now" | Medium | `R-NF-3` violated, later refactor | Store handle lives only on the runtime side; views receive snapshots by message |
| Shell hard-codes the tab and sub-tab set | High if unchecked | MOD-2/13/14/15 each edit the event loop | Registration-based tabs, sub-tabs, overlays and actions; reviewer verifies |
| Domain types copy §5 loosely and diverge from the DDL | Medium | Mapping pain in MOD-6 | Types carry the §5 column names; `MemStore` enforces the §4.1 and §4.2 rules even in memory |
| Terminal left raw on panic | Low | Bad first impression on every crash | Panic hook plus drop guard restore the terminal on every exit path |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
