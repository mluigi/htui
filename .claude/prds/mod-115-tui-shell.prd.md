# MOD-115 — TUI shell: header, panes, layouts, history, widget kit

## Problem
Every htui tab repeats the same chrome (status line, tab strip, a box restating the tab name, two hint rows) and
draws its own lists, forms, empty states and footers inside very large files (`backlog/mod.rs` 4.5k lines,
`settings/agents.rs` 3k). Hints list keys instead of guiding, forms render far from what they edit, raw internal
status names reach the screen, and no tab looks different from another. Leaving it unsolved means each redesign
item (MOD-116..125) would re-implement the same pieces and restyling one tab would not carry to the others.

## Evidence
- `docs/ui-redesign-findings-2026-10-15.md` §1: audit of the 199 insta snapshots and `crates/htui/src/ui/`
  (eight findings), and §2: the maintainer's direction ("btop mostly", "like an IDE", a toggleable hint on what a tab does).
- A runnable throwaway skeleton (`spike/ui-skeleton`, 12 review rounds) in which the maintainer tried the shell and
  settled the decisions in §3; the code is not kept, the decisions are.
- Filed requirements: `R-TUI-1`, `R-TUI-10`, `R-TUI-12`, `R-TUI-13`, `R-TUI-18`.

## Users
- **Primary**: the maintainer, who drives htui all day from a terminal (often under tmux, on an Italian keyboard
  where `[ ]` needs AltGr) and runs several workspaces and items at once.
- **Secondary**: new users, served by the toggleable "about this tab" panel and by hints that say what to do next.
- **Not for**: mouse-only users (the mouse works but is not the primary path); the new tabs (Workspaces, Toolbox,
  Logs) are other items.

## Hypothesis
We believe a **shared btop-style shell and widget kit** will **end the per-tab chrome and drawing duplication and
make every tab guide the user** for **people who live in htui's terminal UI**.
We'll know we're right when **no current tab draws its own pane frame, list, form row, empty state, modal frame or
footer outside the kit, all 199 snapshots are rewritten, and the maintainer signs off the result on a real session.**

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Tabs ported to the kit | all current tabs | review of each tab's render code against the kit's parts |
| Repeated chrome | none (no per-tab hint rows, no tab-name box) | snapshot review |
| Layout survives restart | preset, split sizes, visibility, zoom restored per tab; written the moment they change | test: change, drop the process, reopen |
| Status legibility | every status shows glyph + human word, never colour alone | snapshot review; no raw `awaiting_approval`-style names on screen |
| Snapshots | all rewritten, tab by tab, suite green | `cargo insta` full run, gate |
| Absorbed items | MOD-81, MOD-82, MOD-83, MOD-85 closed | HANDOFF bookkeeping at close-out |

## Scope
**MVP** — the whole item (maintainer answer: "every tab ported"):
- One-row header: workspace, tabs with accents, history chip, running/waiting counts, trimmed when narrow.
- Rounded panes: title and sub-tab chips in the top border, the focused pane's hints in the bottom one, faint
  unfocused borders, selection band, ellipsis on long rows, scrollbar that stops at the end.
- Toggleable "about this tab" panel (`F1`, on at first launch, choice persists).
- Per-tab layout presets (IDE, IDE + activity, three columns, focus), resizable splits, show/hide, zoom, all saved
  the moment they change.
- Navigation defaults: arrows stay inside a pane, `Tab` / `Ctrl ←→` between panes, `Shift ←→` sub-tabs, digits tabs,
  no AltGr-only default. New defaults go in through the rebindable keymap; keys the user already rebound are kept.
- Browser-style history over jumps, tab, sub-tab and pane changes (50 entries; back / forward).
- Leave dialog listing live work with leave / stop-all / stay. "Leave" behaves as today until background running lands.
- Shared widget kit: pane, tree list with groups, filter and selection band, form rows with inline help, key chips,
  empty state, modal frame, legend. `backlog/mod.rs` and `settings/agents.rs` are split along these lines.
- Tab accents, kind colours, glyph + human word for every status.
- All current tabs ported; 199 snapshots rewritten tab by tab.

**Out of scope**
- Workspaces, Toolbox and Logs tabs — MOD-116 and later; the header shows only tabs that exist.
- Workspace lifecycle (open / background / stopping / stopped), stop and resume, and the "+N in background" chip's
  data — MOD-116 / MOD-125; the chip stays hidden until then.
- Runs surviving the TUI exiting — MOD-116 / MOD-125; the leave dialog must not promise it.
- Graphs and the workflow editor, the new-item modal, requirements views — other redesign items.
- Mouse-wheel-under-tmux docs and hint — separate (findings §5).

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Widget kit + shell frame | Header, rounded panes, bottom-border hints, `F9` about panel (F1 stays Help); kit parts exist and two pilot overlays use them | complete | `.claude/plans/mod-115-tui-shell-m1.plan.md` |
| 2 | Layouts and navigation | Per-tab presets, resize, show/hide, zoom, saved at once; pane keys, sub-tab keys; history; leave dialog; every key configurable (maintainer, 2026-10-18) | complete | `.claude/plans/mod-115-tui-shell-m2.plan.md` |
| 3 | Tabs ported, group A | First group of existing tabs on the kit, snapshots rewritten | pending | — |
| 4 | Tabs ported, group B | Remaining tabs, `backlog/mod.rs` and `settings/agents.rs` split; MOD-81/82/83/85 closed | pending | — |

Grouping of tabs into milestones 3 and 4: TBD — `/plan` decides after counting each tab's size and snapshots.

## Open Questions
- [ ] Where per-tab layout and history state live (UI-state file vs database) — TBD, `/plan` decides against `R-TUI-13`.
- [ ] Migration of users' existing key overrides when `Tab` changes meaning — TBD, depends on how MOD-67's keymap stores overrides.
- [ ] Wording of the leave dialog while "keep running" is not yet available — TBD with the maintainer at review.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Snapshot churn hides real regressions | High | Medium | Rewrite tab by tab; review each tab's snapshots before the next; a full insta run (not grep) measures impact |
| Kit built against the skeleton's mock data fits poorly with real tab state | Medium | High | Milestone 1 ports a real pilot screen before the kit is frozen |
| `Tab` meaning change surprises current users | Medium | Low | Defaults only, overrides kept, release note |
| Very large files (`backlog/mod.rs`, `settings/agents.rs`) make the split risky | Medium | Medium | Split along kit parts under existing tests; one tab per milestone step |
| Many tabs ported in parallel conflict over the kit's API | Medium | Medium | Freeze the kit API at the end of milestone 1; fact-check file sets before fan-out |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
