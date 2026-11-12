# MOD-150 — Link kinds and links to documents and attachments

> Origin: `HANDOFF.md` MOD-150 (from ANA-35, `docs/ANA-35.md` §7.6, §7.9 P7, §7.10 order 7, §8 Q18).
> Requirements: `R-ENT-9`, `R-TUI-5`, `R-MCP-2`.
> Routed PRD by `/handoff-run` (C2 new link kinds, a new `item_artifact_link` table and an MCP argument;
> C4 breadth across store, cache, orch, prompt, MCP and TUI); ultracode accepted for implement and review.

## Problem

Item links can say "blocked by", "spawned from", "relates to" and "replaces", and can only point at
another item. So a workspace item (`@shop:WS-4` "Add SSO") cannot own its member-project parts
(`api:FEAT-9`, `frontend:FEAT-14`); closing `FIX-20` as `duplicate` names no canonical item; `FEAT-9`
cannot say it carries out `ANA-35`'s verdict, nor which version of a spec it was built against. The cost
is decomposition and provenance that live only in prose, and five open items waiting on the kinds:
MOD-155 (pending links), MOD-143 (subgraph nodes read `parent`), MOD-114 (3) (attachment targets),
MOD-8 (the importer maps "absorbs" to `parent`) and MOD-135 (flags a parent closed with open children).

## Evidence

- `R-ENT-9` (amended 2026-10-15 and 2026-10-18 by ANA-35/ANA-36) already requires the kinds `parent`,
  `duplicates`, `implements`, document and attachment targets, a single acyclic parent, version-pinned
  document links, and close-out recording the duplicate/supersede link; `R-TUI-5` requires linked
  documents and attachments as Graph nodes and the link kinds as a legend.
- Tree facts (ANA-35 §2.5): `item_link.kind` CHECK lists four kinds; `LinkKind` has four variants; the
  upstream prompt walk follows `blocked_by` and `origin` only; links target items only.
- No usage data: htui is in development with one maintainer. Assumption — the kinds and the separate
  artifact table suffice for MOD-155, MOD-143, MOD-114 (3) and MOD-8; validated by those items' plans
  reading this one's store API.

## Users

- **Primary**: the maintainer, who decomposes workspace work into member-project items, closes
  duplicates and superseded items naming the canonical one, and reads the result in the Graph tab.
- **Secondary**: coding agents, which propose `parent`, `duplicates` and `implements` links and links to
  documents through MCP `item_link`, and whose prompts see a parent's summary through the upstream walk.
- **Not for**: a manual "add link" action in the TUI (`R-ENT-9`: none, except close-out); requirement
  relations ("refines", "covered by" stay citations, MOD-149/`R-ENT-15`).

## Hypothesis

We believe **three new link kinds, a separate version-pinned link table for document targets, and
close-out recording the duplicate/supersede link** will **let decomposition, duplication, implementation
and spec provenance be stored and shown instead of written in prose** for **the maintainer and the agents
working an item**.
We'll know we're right when **an agent links `api:FEAT-9` to `@shop:WS-4` as `parent` and to `ANA-35`'s
`verdict` (pinned v2) as `implements`; the child's prompt shows the parent; a second parent or a cycle is
refused; closing `FIX-20` as `duplicate` of `FIX-17` records the link; and the Graph tab draws all of it
with the legend words** — with readiness, the item-link mirror key and every existing link reader unchanged.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Single parent, single canonical, no parent cycle, parent depth ≤ 8 | every violation refused, same refusal on both stores | conformance cases over MemStore and PgStore (concurrent second-parent insert included on Pg) |
| Parent in the upstream prompt walk | a child's prompt names its parent and the parent's summary | prompt render test + store walk conformance |
| Readiness unaffected by the new kinds | `ready_items`/`queue_rows` results identical with and without `parent`/`duplicates`/`implements` links | conformance case |
| Close-out records the link | closing as `duplicate`/`superseded` with no live link, naming an item, writes the link in the same transaction; without a link and without a named item it is refused | store conformance + orch + TUI e2e test |
| Document links pin a version | `to_document` by owner+kind pins the latest version at write (or the named one), the reply names it, the Graph shows "v2 (latest v4)" | MCP tool tests + Graph snapshot |
| Existing links untouched | every pre-existing link test and snapshot passes unchanged, except snapshots whose legend grows | full gate + full insta run |

## Scope

**MVP** — the three milestones below.

Decisions taken in this PRD (maintainer, 2026-11-12):

- **Agents propose all three new kinds live** through MCP `item_link` (`parent`, `duplicates`,
  `implements`), as `blocked_by` is live today. `R-ENT-9`'s "pending until a person accepts" for
  `parent`/`duplicates`/`supersedes`/`blocked_by` is MOD-155's, and arrives there with no API change.
- **`to_document` names a document by owner and kind**, with an optional version: the owner is a
  (qualified) item key, or a project/workspace slug for an owner document (MOD-147); no version pins the
  latest at write time; the reply names the canonical owner, kind and pinned version. No UUID form: agents
  never see document ids today (`search_concepts` hits carry key and `document_kind`, MOD-112's read tools
  are not built). Reach is `propose_link`'s (the step's project, its workspaces' scope rows and members).
- **Three milestones**, kinds → close-out → document links.

Settled by ANA-35 §7.6 (accepted 2026-10-18, §8 Q18) and not reopened here: `parent` at most one live per
child, acyclic (walk up before insert), depth at most 8, joins the upstream walk, never affects readiness;
`duplicates` at most one live canonical per item; `implements` item → item or document; document and
attachment targets live in `item_artifact_link` (kinds `origin`, `relates`, `implements`), pinning a
document version, `to_attachment_id` without its foreign key; Graph legend words blocks, from, relates
to, replaces, part of, duplicate of, implements.

**Out of scope**
- Pending state for agent-proposed scheduling/status kinds, accept/reject UI — MOD-155.
- The `attachment` table, the `to_attachment_id` foreign key, attachment nodes in the Graph — MOD-114 (3).
- Flagging a parent closed with open children on the close-out checklist — MOD-135.
- Subgraph nodes reading `parent` — MOD-143. Importer mappings — MOD-8.
- A manual link action in the TUI — not wanted (`R-ENT-9`).
- MCP read tools exposing document ids — MOD-112.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Link kinds in the store | `parent`, `duplicates` and `implements` are stored link kinds (widened CHECK, `LinkKind`); a second live parent or canonical, a parent cycle and a parent chain deeper than 8 are refused on both stores; `parent` joins the upstream prompt walk (a child sees its parent's summary) without touching readiness; MCP `item_link` advertises and accepts the three kinds live; the Graph tab draws them with the legend words (part of, duplicate of, implements) | in-progress | `.claude/plans/mod-150-m1-link-kinds.plan.md` |
| 2 | Close-out records duplicates and supersedes | Closing an item as `duplicate` or `superseded` requires a live `duplicates` link from it (or `supersedes` link to it), or records one in the same transaction from the item the person names in the close-out flow; refused with neither | pending | — |
| 3 | Links to documents | An item links to a document version (`item_artifact_link`, kinds origin/relates/implements, `to_attachment_id` reserved without FK), mirrored in the cache; MCP `item_link` takes `to_document` by owner + kind (+ version) and answers the pinned version; the Graph tab shows linked documents as nodes, "v2 (latest v4)" | pending | — |

## Open Questions

- [ ] Concurrency of the cycle and depth check on Pg: the walk-up is a read before insert, so two
  concurrent `parent` inserts could close a cycle. TBD — settle the lock (per-project advisory lock, or
  locking the ancestor chain) in `/plan` M1's fact-check against `propose_link`'s transaction.
- [ ] How the upstream walk bounds itself with `parent` added (hops, de-duplication of an item reached as
  both parent and blocker, depth 8 vs the walk's own hop limit). TBD — read the walk in `/plan` M1.
- [ ] Which close-out surfaces exist besides the Backlog runs pane (CLI, waiting-row actions, headless
  worker) and must offer "name the canonical item". TBD — enumerate in `/plan` M2's fact-check.
- [ ] Whether a withdrawn link (`deleted_at`) may be re-proposed with the same kind (today's PK
  `(from, to, kind)`) and how that meets the single-parent partial unique. TBD — read `propose_link` and
  `withdraw_link` in `/plan` M1.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Auto-generated constraint name `item_link_kind_check` differs on a live database | Low | Medium | Check `\d item_link` on the sandbox Postgres before writing `DROP CONSTRAINT` (ANA-35 §11) |
| Concurrent parent writes close a cycle on Pg | Medium | Medium | Lock chosen in M1's plan; a concurrent conformance case on Pg |
| Migration number collides with a parallel branch (latest is `0029`) | High | Low | Take `0030`; renumber at `scripts/hr collect` and repin count pins (`CASES`, `TABLES`, `MIRRORED_TABLES`, the `connection__confirm` snapshot) |
| A new kind leaks into readiness or the queue | Low | High | Readiness SQL filters `kind = 'blocked_by'` explicitly; a conformance case pins it |
| Legend and Graph snapshot churn | High | Low | Full insta run, every moved snapshot reviewed |
| MCP `item_link` schema change moves snapshots and prompt digests | Medium | Low | Accept and review the moved snapshots; note digest moves |
| Live-Qdrant / Postgres gates unreachable in the hr sandbox | High | Low | Sandbox gate on the rest; host gate owed before close-out |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
