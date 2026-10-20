# MOD-163 — Memory: entry store and prompt section

> Routed as **PRD** by `/handoff-run MOD-163` (criteria C2, C3 and C4 fired). Accepted by the maintainer
> 2026-10-20, sandbox run `hr/MOD-163`. Ultracode accepted for the implement phase and for the review
> verify pass. Origin: ANA-38 (`docs/ANA-38.md` §7.2 P1, §4.6; `docs/decisions/ana/ana-38.md`, including
> "Reconciled with ANA-37" and "Owed edits decided"). Requirements: `R-ENT-13`, `R-PRM-1`, `R-PRM-3`,
> `R-ENT-12`, `R-ENT-10`, `R-TUI-8`, `R-TUI-14`. PRD gate: maintainer accepted the recommended answers to
> Q1-Q4, 2026-10-20.

## Problem

Knowledge that outlives one item (a build gotcha, a decision, a recurring error and its fix) has no home
in htui. An agent step sees its item, input documents, upstream summaries of linked items, excerpts and
skills (`R-PRM-1`); a gotcha learned on an unlinked item never reaches it, so every run relearns it.
`R-ENT-13` (a "must" as a whole since 2026-10-20) asks for memory entries read by agents and the
coordinator. MOD-163 is the base every other memory item stands on (MOD-164..169): without an entry
store and a prompt section, automatic capture, freshness, Qdrant indexing and measurement have nothing to
write to or read from.

## Evidence

Read at `a801f477` (branch `hr/MOD-163`). Paths are relative to the repo root.

- **Maintainer requirement, not an incident.** `R-ENT-13` was reopened by the 2026-10-15 requirements
  review and settled by ANA-38 (concluded 2026-10-20). The benefit to htui's own runs is an
  **assumption — needs validation via MOD-169's before/after measurement on real htui steps**.
- **The need is visible in the maintainer's own workflow.** The development workflow keeps about 45
  hand-indexed auto-memory gotchas, several of them re-learned by agents more than once, and some already
  stale (`docs/ANA-38.md` §2.6). That memory is per user and per box; htui runs cannot see it.
- **No home exists today.** `document.item_id` is `NOT NULL` (`crates/htui-store/migrations/0001_init.sql:391`,
  unchanged through `0019`); no owner documents exist; MOD-144..150 (ANA-35's owner work) are all open.
  `item_note` is per item. Hence the maintainer's Q1 answer: a dedicated store.
- **The prompt seam fits a new section.** `SectionName` is a closed enum and `PromptSpec` carries one
  field per section (`crates/htui-core/src/prompt/mod.rs:76, 282`); phase trim order is Excerpts,
  Upstream, PreviousDiff, VerifyFailure, Documents, Item (`crates/htui-core/src/prompt/trim.rs:358-366`);
  `RECORD_VERSION` is 4 (`trim.rs:59`); `phase_spec` builds the spec (`crates/htui-orch/src/engine.rs:5919`).
  `assemble` is pure (no store, no clock), so selection must happen before it, in `phase_spec`.
- **No Workspaces tab yet.** MOD-116 is open (blocked on MOD-115); the app registers Backlog, Skills,
  Requirements, Settings and Chat (`crates/htui/src/app/mod.rs:66-88`); projects are shown in Settings'
  hierarchy section (`crates/htui/src/ui/tabs/settings/hierarchy.rs`).
- **Settings cascade has no workspace rung.** `SettingRung { App, Project(_), Phase(_) }`
  (`crates/htui-core/src/store/traits.rs:3134-3148`); `workspace` has no settings column
  (`0001_init.sql:129-137`).

## Users

- **Primary**: agents running steps in a project, who need the project's standing facts, decisions,
  gotchas and lessons in their prompt without being linked to the item that learned them; and the
  maintainer, who writes, corrects and retires entries and must be able to see what every step was given.
- **Not for**: the coordinator seed (MOD-136); global or workspace-owned memory (MOD-168, MOD-147);
  agents searching memory beyond the prompt (MOD-167); the development workflow's own memory (separate by
  `R-ENT-13`; a one-time hand seed is a later manual act, not a sync).

## Hypothesis

We believe **a project-scoped store of verbatim memory entries, with a fixed-order, capped prompt section
filled from Postgres and a TUI list to write, edit and retire them**, will **let knowledge learned on one
item reach every later step of the project, deterministically and visibly**, for **agents and the
maintainer**.
We'll know we're right when **an entry written in the TUI appears in the next step prompt of every item in
its project, identical across fan-out siblings and retries of one run, recorded by id and version in the
step's trim record, and a project with no entries gets byte-identical prompts to today's**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Reach | an active entry appears in every phase step prompt of its project; none from another project | engine tests over the fake driver |
| Run-start snapshot | siblings and retries of one run get a byte-identical memory section even when an entry is edited or retired mid-run | engine tests |
| Only live entries | `suspect`, `superseded` and `retired` entries never appear | store conformance + engine tests |
| No-memory parity | a project with zero entries produces the same prompt bytes as before MOD-163 | prompt golden tests |
| Cap and left-out line | section stays under its token cap (starting default ~1,500); a left-out count line names no tool | prompt unit tests |
| Trim record | ids and versions of shown entries recorded; `RECORD_VERSION` 5; v4 records still read | trim tests |
| Trim rank | memory is cut after Excerpts and before Upstream, never protected | trim tests |
| Safe writes | text scrubbed before persist, failing closed (`R-ID-7`); concurrent edits refused by compare-and-set (`R-ENT-10`) | store conformance (MemStore, PgStore) |
| TUI | list, manual write, edit, retire from Settings under the project; capture switch at app and project level | TUI tests + insta snapshots |

## Scope

**MVP**: a dedicated memory store for project-owned entries (kind `fact` | `decision` | `gotcha` |
`lesson`; subject; prose; state `active` | `suspect` | `superseded` | `retired`; freshness stamp; version
history), on every store backend the trait requires; a person's manual write, edit and retire; a new
internal prompt section filled in `phase_spec` from a run-start snapshot of active non-suspect entries, in
fixed order, under a hard token cap, rendered as quoted data labelled with source and trust, unprotected,
trimmed after Excerpts and before Upstream, recorded in `trim_record` (version bump); a self-contained TUI
memory pane mounted under the project in Settings; the "automatic capture" switch at the app and project
rungs, default on, read by MOD-164.

Decisions taken at the PRD gate (maintainer, 2026-10-20):

- **Q1 — storage: a dedicated table**, not an ANA-35 owner document. Owner documents do not exist yet,
  and state, pin, counters and a three-level scope do not fit document columns. Settles ANA-38 Q3.
- **Q2 — TUI host before MOD-116: interim in Settings.** The list is a self-contained pane mounted under
  the project in Settings' hierarchy section; MOD-116 moves it into the Workspaces tab's memory sub-tab
  (`R-TUI-14`). Every new key goes through `keys.toml`.
- **Q3 — capture switch ships here**, at the app and project rungs (workspace after MOD-147), labelled
  as taking effect with automatic capture; MOD-164 reads it.
- **Q4 — left-out line: count only** ("6 of 14 project memory entries shown"), naming no tool until
  MOD-167 makes memory searchable; MOD-167 adds the `search_concepts` pointer under MOD-160's rule.

**Out of scope**

- Automatic capture, write caps, usage counters, automatic supersede. **Why:** MOD-164.
- The lesson detector. **Why:** MOD-165.
- Freshness sweep, pins, pin-as-trusted, re-validation, auto-retire, the "why is this here" view.
  **Why:** MOD-166. (MOD-163 stores the state and stamp; nothing here sets `suspect`.)
- Qdrant point type, `search_concepts` over memory, near-duplicate candidates, Qdrant-ranked order.
  **Why:** MOD-167, MOD-169.
- Workspace and global scope, the global prompt section, promotion. **Why:** MOD-147, MOD-168.
- The Workspaces tab sub-tab. **Why:** MOD-116 (it re-mounts this item's pane).
- The coordinator seed reading entries. **Why:** MOD-136.
- Seeding from the development workflow's memory. **Why:** a one-time hand-reviewed act after this lands
  (ANA-38 Q10), not code.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Entry store | Project memory entries can be written, edited, retired and listed with version history, scrubbed and compare-and-set, on every store backend | in-progress | `.claude/plans/mod-163-m1-entry-store.plan.md` |
| 2 | Prompt section | Every phase step prompt of a project carries its active entries from a run-start snapshot, capped, trimmed in rank, recorded in `trim_record` v5; no-memory prompts unchanged | pending | — |
| 3 | TUI pane and switch | The maintainer lists, writes, edits and retires a project's entries from Settings, and sets the capture switch at app and project level | pending | — |

## Open Questions

- [ ] Column set now versus later: provenance (run, agent, item, commit), pin, pinned-as-trusted and usage
  counters belong to MOD-164/166. Add the columns now (one migration) or let each item migrate? TBD —
  settle in `/plan`.
- [ ] What the "freshness stamp" is before MOD-166 (a confirmed-at time and the entry version are the
  candidates). TBD — `/plan`, against ANA-38 §4.6 S2.
- [ ] A manual write whose (project, subject) matches an active entry: supersede the old one, or refuse
  and point to edit? TBD — `/plan` (MOD-164's automatic supersede uses the same key).
- [ ] Where the run-start snapshot lives (a per-run record of entry ids and versions, or a frozen copy of
  the text) so retries and resumed runs (MOD-125) see the same set. TBD — `/plan`.
- [ ] Does the SQLite cache mirror carry memory (`R-STO-3` lists items, notes, documents, requirements and
  citations, not memory)? TBD — `/plan`; offline browsing of the pane is the deciding use.
- [ ] Rendering: the exact source and trust label for a person-written entry versus a later automatic one,
  and the per-entry and section format under the cap. TBD — `/plan`, against ANA-5 invariants.
- [ ] Which docs need dated notes (ANA-5 §4.4 trim order and section vocabulary, CONCEPTS "Memory")
  versus already amended on 2026-10-20. TBD — `/plan` fact-check.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A new store-trait method fans out to every backend (PgStore, MemStore, CacheStore, Writer, WorkerStore, Backend, UsageSpy) plus conformance and `.sqlx` | High | Wide, shallow edit set; parallel tasks collide | File-set intersection in the plan fact-check; `.sqlx` regenerated against a migrated scratch DB |
| The `RECORD_VERSION` bump and a new section ripple into prompt goldens and TUI snapshots | High | Many snapshot updates | Full insta run; no-memory parity test pins that only memory changes prompt bytes |
| Settings is still moving (MOD-115 M3/M4, MOD-123 redesign) | Medium | Merge conflicts in the interim pane mount | Keep the pane self-contained; the mount is a thin seam |
| An unprotected section sitting between Excerpts and Upstream changes which inputs survive a tight budget | Medium | Upstream summaries cut later than before when memory is large | Cap below the budget; trim tests on the new order |
| Stored text becomes a standing instruction without review once MOD-164 writes automatically | Medium (later) | Prompt poisoning | Quoted-data rendering with source and trust labels from day one; scrub before persist |
| Engine walks near the 2 MiB debug stack | Low | SIGABRT in tests | Gate `--no-fail-fast`, grep for SIGABRT |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
