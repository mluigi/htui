# MOD-13 - Backlog filters and item editing

> HANDOFF item: `MOD-13` (from MOD-1; routed **PRD** by `/handoff-run`, 2026-10-01; C2 + C4 fired,
> low confidence at threshold, maintainer accepted; ultracode for **implement**). Sandbox run on
> branch `hr/MOD-13`. Requirements: `R-TUI-2`, `R-ENT-5`, `R-ENT-10`, `R-ENT-11`, `R-ENT-12`.
> Design authority: `docs/ANA-9.md` §4.1 (keys), §4.2 (divergence), §7.1 (mint), §7.2
> (compare-and-set), §7.4 (readiness); `docs/ANA-2.md` §4.7 (`repo_name:glob`). MOD-25 (closed
> 2026-09-16) makes `htui` online-only.

## Problem
The Backlog is still read-only, as MOD-1 shipped it. The maintainer can browse items but cannot
create one, change its title, body, tags or `touched_paths`, add a note or write a document from
the TUI, and cannot narrow a long list to what matters, e.g. "open, ready for this box, project X".
The store already mints keys and rejects stale writes (`MemStore`, `PgStore`), but no UI sits over
those operations, and a rejected edit has nowhere to be shown. Auto mode (MOD-12), remote dispatch
(MOD-43) and the orchestrator's summary (MOD-4 R-43) all assume items a human can author and fix.

## Evidence
- `R-TUI-2`, `R-ENT-5`, `R-ENT-10..12` are **must** requirements with no UI today (MOD-1 write-up,
  `docs/decisions/mod/mod-1.md`: "read-only views").
- MOD-4 close-out: the generated `summary` document carries no human prose; "an editable summary is
  this item's editor's" (`HANDOFF.md`, MOD-13 entry; MOD-4 risk R-43).
- ANA-2 §4.7: an empty `touched_paths` overlaps the whole primary repo, so declaring paths is the only
  way to buy parallelism, which requires an edit path that validates `repo_name:glob`.
- Assumption — needs validation via prototype: the filter set (status, project, capability,
  readiness) is enough to find work in a multi-project backlog without a free-text search.

## Users
- **Primary**: the single maintainer (`R-USR-1`) at a box that is connected to the server. They open
  the Backlog to triage, author a new item, fix an item's spec before queueing it, or record a note.
  Trigger: planning a session, or a run exposing a wrong or missing spec.
- **Not for**: an offline box. It browses read-only and says so, and there is no local mint and no
  queued write (MOD-25). Also not for agents: they write documents through steps, not this editor.

## Hypothesis
We believe **Backlog filters plus compare-and-set item editing in the TUI and in `$EDITOR`** will
**let the maintainer run the whole item lifecycle's human half inside `htui` without ever silently
losing a concurrent edit** for **the maintainer**.
We'll know we're right when **every `R-TUI-2` filter and the `new`/`edit` actions work identically
against `MemStore` and `PgStore`; a concurrent edit on the same `version` always produces the
three-way view and never an overwrite; and an item can be created, edited, noted and given a
hand-written document end to end from the keyboard.**

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Lost concurrent edits | 0 | Conformance case: two writers on one `version`, loser gets `Diverged`, resolution revision has `reason = 'divergence_resolution'` |
| Store parity | same outcome on both backends | Every new store-facing case runs under the shared conformance suite against `MemStore` and `PgStore` |
| Offline write surface | 0 reachable edit actions | Test: backend `Offline` → `new`/`edit`/note/document actions absent or refused with a visible read-only notice |
| Filter correctness | readiness filter matches ANA-9 §7.4 | Fixture test: filtered set equals the §7.4 query result for the selected box |
| Invalid `touched_paths` accepted | 0 | Test: unknown `repo_name`, malformed glob rejected at edit time with a message |

## Scope
**MVP** — all five parts (maintainer decision, 2026-10-01):
1. **Filters**: status, project, capability (required tags), readiness. "Ready" means ANA-9 §7.4:
   `open`, required tags covered by this box's `probed_tags ∪ declared_tags`, and no unfinished
   `blocked_by` target.
2. **`new` and `edit`** actions over the `version`-covered spec columns (ANA-9 §4.2), including
   `touched_paths` with `repo_name:glob` validation (bare glob = primary repo). `new` mints through
   ANA-9 §7.1 only; `edit` is one compare-and-set per §7.2 that writes a revision.
3. **Three-way divergence view**: ancestor / theirs (head) / mine. The user takes theirs or mine
   as the base, or edits the merged text, and resubmits against `head.version`. The resolution is
   recorded with `reason = 'divergence_resolution'`.
4. **External `$EDITOR` round-trip** for body (and other long text), feeding the same
   compare-and-set path.
5. **Note thread append** (`R-ENT-11`) and **hand-written documents** (`R-ENT-12`) of any document
   kind, including an editable `summary`.

**Out of scope**
- Per-hunk merge UI in the divergence view — pick-one plus free edit is enough to never lose work;
  revisit on evidence.
- Any offline edit, local `mint_item`, local `item_key_counter`, or compare-and-set backed by
  anything but `PgStore`/`MemStore` — forbidden by MOD-25.
- Status transitions (`close`, reopen) — orchestrator-driven (`R-ENT-8`) with their own status
  CAS. `R-TUI-2`'s `run`/`queue`/`close`/`open graph` actions belong to the orchestrator and graph
  items.
- Link editing (`blocked_by`, `origin`), requirement editing (`R-ENT-14`), importer mint variant.
- Free-text search over titles and bodies — not in `R-TUI-2`.

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Filters | Backlog narrows by status, project, capability, readiness; filter state visible | complete | `.claude/plans/mod-13-filters.plan.md` |
| 2 | New and edit | Items minted and edited in-TUI through §7.1/§7.2, `touched_paths` validated, offline read-only notice | pending | — |
| 3 | Divergence | A stale edit opens the three-way view and resolves to a `divergence_resolution` revision | pending | — |
| 4 | `$EDITOR` round-trip | Long text edited externally, returns through the same compare-and-set | pending | — |
| 5 | Notes and documents | Note appended to the thread; hand-written document of any kind, incl. `summary`, saved as a new version | pending | — |

## Open Questions
All four were resolved by the maintainer on 2026-10-01, taking the defaults.
- [x] Which `version`-covered columns are editable in this item? Default: all of them (`title`, `body`,
  `kind_id`, `required_tags`, `priority`, `touched_paths`, `step_graph_id`). But ANA-9 §7.2's SQL
  updates only `title`/`body`/`required_tags`, and re-kinding must never rewrite the key (§4.1).
  To be settled in `/plan` against the actual store signature.
- [x] Readiness is box-relative (§7.4). Which box does the filter use: the current box only, or a
  selectable one? Default: the current box.
- [x] How is a `touched_paths` entry naming a repo outside the project treated: reject, or warn?
  Default: reject (ANA-2 §4.7 resolves `repo_name` against the project's repos).
- [x] Is a hand-written document an edit of the latest version (new version on save) or always a fresh
  document? Default: new version per save, with no compare-and-set (documents are append-versioned,
  `R-ENT-12`). TBD — needs validation against the MOD-6 document write semantics.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `$EDITOR` suspends the terminal and a store reply or tick lands mid-suspend | M | M | Keep the TUI event loop paused/restored around the child. Pin it with a test using a fake editor command |
| `MemStore` and `PgStore` diverge on the new write paths | M | H | Store-facing behavior lands as conformance cases run against both |
| Status CAS and body CAS confused, so a step completion invalidates an open edit | L | H | ANA-9 §4.2 split: status never bumps `version`. Add a conformance case for it |
| An edit action reachable while `Offline` | L | H | Gate at the action registry, plus a test per action |
| Snapshot churn from new Backlog chrome (filter bar) breaks unrelated tests | M | L | Keep filter chrome out of existing snapshots when no filter is set, or update them in one commit |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
