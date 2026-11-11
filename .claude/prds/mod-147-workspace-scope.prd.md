# MOD-147 — Workspace scope: items, kinds, documents, folder

> Origin: `HANDOFF.md` MOD-147 (from ANA-35, `docs/ANA-35.md` §7.5, §7.9 P4, §7.10 order 4).
> Requirements: `R-ENT-2`, `R-ENT-5`, `R-ENT-7`, `R-ENT-12`, `R-ENT-16`, `R-STO-3`, `R-STO-8`, `R-MCP-1`.
> Routed PRD by `/handoff-run` (C2 new owner model and qualified keys, C4 breadth across store, cache,
> orch, search, MCP and TUI); ultracode accepted for implement and review.

## Problem

A workspace owns nothing but its member list today. Work that spans its projects ("which auth library do
`api` and `frontend` both use?"), the workspace's goals, and soon its coordinator runs (MOD-136) and
requirements (MOD-149) have no owner: every owned table, the key counter, the cache cursor and the search
tenant require a `project_id`, and every document requires an `item_id`. The cost is that cross-project
work is filed under an arbitrary member project, and eight open items stay blocked on this ownership
model (MOD-136, MOD-149, MOD-116, MOD-8, MOD-112 (4), MOD-159's coverage, the workspace memory scope,
MOD-150's document targets).

## Evidence

- The 2026-10-15 requirements review amended `R-ENT-2` ("[a workspace] owns its own items, documents and
  requirements"), `R-ENT-16` (own key space and kinds) and `R-STO-8` (workspace-scoped search including
  workspace-owned documents) (`docs/requirements-review-2026-10-15.md:86-88`).
- ANA-35 §2.4-§2.5 tree facts: `project_id NOT NULL` on every owned table; `document.item_id NOT NULL`;
  the Qdrant `Owner` is `Item | Requirement`, so a document with no item is never indexed.
- No usage data: htui is in development with one maintainer. Assumption: the scope-row model (ANA-35 K2)
  suffices for MOD-136/MOD-149/MOD-116 — validated by those items' plans reading this one's store API.

## Users

- **Primary**: the maintainer running several related projects as one workspace (e.g. `shop` with members
  `api` and `frontend`), who files cross-project work, keeps workspace goals, and reads it in the Backlog.
- **Secondary**: coding agents, which address workspace items by qualified key over MCP and propose links
  to them from member-project steps.
- **Not for**: lone projects outside any workspace (no folder, no scope row); multi-tenant or multi-user
  separation (the scope row is an ownership device, not an access-control one).

## Hypothesis

We believe **one hidden scope row per workspace in `project`** will **give workspaces their own items,
kinds, key space, graphs, documents and runs without a second owner column** for **the maintainer and the
agents working a workspace**.
We'll know we're right when **a `WS` item mints as `@shop:WS-1`, runs plan → review in its scope row,
links to member items, its workspace's goals document is searchable, and a conformance case proves the
scope row appears in no project listing** — with no per-project read path changed to learn a second owner.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Scope-row leakage into listings (project lists, Settings hierarchy, cwd mapping, `htui status`, membership) | 0 | conformance case(s) over MemStore and PgStore |
| Workspace item lifecycle | create → enqueue → plan → review → close-out passes | orch fake-conformance / integration test on a workspace item |
| Qualified key resolution | `@shop:WS-1` and `api:FEAT-9` resolve in MCP and CLI, including when a project is also named `shop` | MCP and CLI tests |
| Owner documents searchable | a workspace `goals` document is returned by a workspace-scoped search | live-Qdrant test (owed on host) |
| Existing workspaces | every existing workspace has exactly one scope row after migrate + bootstrap | migration test `run_to(25)` → plant → `run_to(26)` + bootstrap |

## Scope

**MVP** — the four milestones below: the scope row and owner documents in the store, keys and links
across a workspace, the TUI surfaces, and search.

Decisions taken in this PRD (maintainer, 2026-11-08):

- **Search lands in the current collection.** `Owner::Document` points and the owner-document indexer
  pass go into today's `htui_concepts_v2`; the payload-schema stamp change drops and reindexes it (no
  collection rename while htui is in development). MOD-159 later moves everything into its per-workspace
  collections.
- **Workspace search scope** ("member projects plus the scope row") ships as a resolver used by the TUI
  and CLI search. Widening the MCP `search_concepts` tool for agents stays MOD-112's.
- **Owner documents in the TUI** live in Settings' project and workspace entries: a documents list, view,
  new version through `$EDITOR`, with `goals` preset for a workspace. MOD-116 later moves it into the
  Workspaces tab.

**Out of scope**
- Workspace requirements, `W-` keys, nesting — MOD-149.
- Link kinds `parent`/`duplicates`/`implements`, links to documents — MOD-150.
- Coordinator runs and the item-less `ToolScope`/`WaitingRow` — MOD-136 (it consumes the scope row).
- The Workspaces tab and its folder/components UI — MOD-116, MOD-117.
- Per-type seeds, the type picker, new template bodies (including the workspace `plan` body) — MOD-148.
- `needs_repo` and repo-less run refusals — MOD-145 (the workspace seed's phases need no repo).
- Per-workspace Qdrant collections, the indexer lease — MOD-159. MCP search widening — MOD-112.
- Workspace-level skill and target bindings — MOD-113 (2), MOD-130.
- Attachments — MOD-114 (3).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Scope row and owner documents in the store | Creating a workspace creates its scope row (`@<slug>`, type `workspace`) in the same transaction, and the bootstrap creates one for every existing workspace; the workspace seed gives it the `WS` kind (plan → review); items mint `WS-N` from its own counter; documents carry `project_id` with a nullable `item_id`, so a project or workspace can own one; `workspace_box_path.folder_path` with its `~/workspaces/<slug>` default; the scope row never appears in any listing; the cache mirrors the scope row and owner documents (the folder stays online-only, as `workspace_box_path` always has) | complete | `.claude/plans/mod-147-m1-scope-row.plan.md` |
| 2 | Keys and links across a workspace | A workspace kind's prefix equal to a member's is refused, a colliding project may join with a warning; keys are qualified `slug:KEY` wherever ambiguous (MCP, CLI), `@shop:WS-1` for workspace items; `propose_link` reaches the step's workspaces and their member projects | complete | `.claude/plans/mod-147-m2-keys-and-links.plan.md` |
| 3 | TUI surfaces | The Backlog shows each workspace's own items as the workspace's group; Settings' project and workspace entries list, show and add owner documents (goals preset for a workspace); the workspace form sets the scope row's secret binding and shows the folder path | pending | — |
| 4 | Search | Owner documents (no item) are indexed as `Owner::Document` points by an owner-document pass beside the item loop, in today's collection (payload stamp bump → drop and reindex); a workspace-scoped search in the TUI/CLI covers member projects plus the scope row | pending | — |

## Open Questions

- [ ] Workspace deletion: ANA-35's DDL cascades the scope row (and with it the workspace's items,
  documents and runs) from `workspace` (`ON DELETE CASCADE`). Is cascading acceptable, or must deleting a
  workspace that owns items refuse? TBD — needs validation by reading today's workspace-delete path in `/plan`.
- [ ] The workspace seed's `plan` phase: which prompt template body it uses until MOD-148 adds the
  workspace `plan` body (likely today's software `plan` template). TBD — settle in `/plan` M1.
- [ ] Which existing listing sites need the visible-projects predicate (lists, Settings hierarchy, cwd
  mapping, `htui status`, membership pickers, fixtures, demo loader). TBD — enumerate in `/plan` M1's
  fact-check.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A listing site misses the visible-projects predicate and shows `@shop` as a project | Medium | Medium | One predicate, one conformance case per listing surface; grep audit in the plan fact-check |
| Migration number collides with a parallel branch (MOD-139, MOD-143, MOD-159 also owe one) | High | Low | Take `0026`; renumber at `scripts/hr collect` and repin `migrations.rs` and the count pins (`CASES` 233, `TABLES` 51) |
| Document unique-constraint change and `project_id` backfill break `.sqlx` entries and the cache mirror | Medium | Medium | `cargo sqlx prepare` against a migrated scratch DB; cache migration `0009`; mirror rebuild by schema version |
| Live-Qdrant tests time out in the hr sandbox | High | Low | Run them on the host before close-out; sandbox gate covers the rest |
| Snapshot churn in the Backlog and Settings | Medium | Low | Full insta run, every moved snapshot reviewed |
| MOD-159 lands concurrently and rewrites the indexer | Low | Medium | Keep the owner-document pass a separate function beside the item loop, easy to move into MOD-159's indexer |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
