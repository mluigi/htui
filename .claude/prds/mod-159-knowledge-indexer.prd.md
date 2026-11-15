# MOD-159 — One knowledge indexer per collection: lease, freshness, coverage

> Origin: `HANDOFF.md` MOD-159 (from ANA-37, `docs/ANA-37.md` §7.2, §7.3, §8 Q3-Q5; amended 2026-10-20 by the
> "Reconciled with ANA-38" section of `docs/decisions/ana/ana-37.md`: one Qdrant, a global collection, one lease per
> collection).
> Requirements: `R-STO-8`, `R-ENT-12`, `R-ENT-13`, `R-ENT-16`; touched: `R-NF-2` (fail open).
> Routed PRD by `/handoff-run` (C2 new lease table and collection seam, C4 breadth across store, vector sync, worker,
> TUI, CLI and MCP search); ultracode recommended for implement. Sandbox run `hr/MOD-159`.

## Problem

Every box that has a Qdrant URL indexes every project into one shared collection, `htui_concepts_v2`: the worker's
index job every `concepts_sync_minutes` (15), `--index-items` and the TUI's `Ctrl+R`. On a box where the TUI is the
executor (the default), nothing indexes on its own, so the index goes stale until someone reindexes by hand. With two
indexers at once, one lists items before it scrolls points, so a point the other wrote in between is deleted as an
orphan (`docs/ANA-37.md` §2.1). And `R-STO-8` (amended 2026-10-19/20) now wants one collection per workspace plus one
global collection, each kept current by exactly one lease holder, with rows indexed within a short cycle. Until this
lands, MOD-112 (4) (notes, transcripts, agents' workspace scope), MOD-161 (related items at draft time) and MOD-167
(memory points) have no indexer to plug into.

## Evidence

- `docs/ANA-37.md` §2.1, §2.5 (code read): no write path triggers indexing; every worker with a URL re-syncs every
  project every 15 minutes, redundantly; the two-indexer orphan race is inferred from code order (`Indexer::sync`
  lists items, then scrolls points) and not tested.
- `R-STO-8` text (`docs/REQUIREMENTS.md`): per-workspace collections, a global memory collection, one lease holder per
  collection, short cycle plus full reconcile — decided, not asked again.
- Tree facts at plan time (checked in the plan's fact-check): `run.workspace_id` already exists (`0019_stop_runs`,
  MOD-125 M1) and scope-row runs carry their workspace (MOD-147 M2), so "the run records the workspace it was queued
  from" is already true. Owner documents (MOD-147 M4, `sync_owner_documents`) and scope-row requirements with `W-`
  keys (MOD-149) are already indexed, because `Indexer::sync` walks each project of `workspace_search_scope`, scope row
  included. So the item's "coverage" part is to carry them into the new layout, not to build new point types.
- No usage data: htui is in development with one maintainer. Assumption: a 60 s cycle is fresh enough for search
  (ANA-37 Q4) — validated if no lag is reported; NOTIFY is revisited with MOD-46.

## Users

- **Primary**: the maintainer, searching a workspace's knowledge from the TUI (`Ctrl+F`) or `--search-items`, who
  expects a just-filed item or document to be findable within about a minute without pressing `Ctrl+R`, on any box.
- **Secondary**: coding agents through `search_concepts` (today project-only; MOD-112 (4) widens it) and the later
  consumers MOD-161 and MOD-167, which need a single indexer to plug a point type into.
- **Not for**: multi-Qdrant deployments (one Qdrant per database, MOD-158); code or checkout markdown (`R-LATER-7`).

## Hypothesis

We believe **one lease-holding indexer per collection, with a 60 s incremental cycle and the full reconcile under the
lease,** will **keep every workspace's index current without duplicate work or the orphan race** for **the maintainer
and agents searching a workspace**.
We'll know we're right when **an item written on box A is found by a search on box B within two cycles while only one
process indexes its workspace, and a deletion by a holder whose lease lapsed never removes a point the new holder
wrote**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Indexers per collection at a time | exactly 1 (lease holder) | store conformance cases over MemStore and PgStore: take, renew, expire, steal-after-expiry, release, zero-row renewal = abandon |
| Freshness | a new or edited item, requirement or document is in its workspace's collection after one incremental cycle (≤ 60 s + overlap) | `vector_sync` test over `MemVectorStore`; live-Qdrant case (owed on host) |
| Orphan race | a lapsed holder's delete batch removes nothing another holder wrote | test: holder A loses the lease between listing and delete; its batch is abandoned |
| Layout | each workspace's points in its own collection; a project in two workspaces in both; global collection exists and is searched beside the workspace; `htui_concepts_v2` dropped | `vector_sync`/`concepts_worker` tests over `MemVectorStore`; live case owed |
| Holder visibility | `Ctrl+R` and `--index-items` with the lease held elsewhere request a pass and name the holder's box | TUI snapshot / CLI output test |
| Coverage carried | owner documents and scope-row requirements still found by a workspace search | existing MOD-147/MOD-149 tests re-pinned on the new layout |

## Scope

**MVP** — the three milestones below.

Decisions taken in this PRD (maintainer, 2026-11-14, at `/handoff-run`):

- **Every process competes for every collection.** Any TUI or worker that resolves Qdrant settings tries for the lease
  of every workspace in the database plus the global collection; first come. No workspace goes unindexed because no
  one has it open; in practice one process holds all leases.
- **The global collection's seam lands now.** It is named, has a lease, and every workspace search also queries it.
  It holds no points until MOD-167/MOD-168 add the memory point type; a missing global collection answers no hits,
  never "unavailable".

Decided upstream, not asked again: lease row per collection (holder, box, expiry) on the run lease's TTLs
(`lease_ttl_seconds` 120, `lease_refresh_seconds` 60), renewed before each delete batch with a zero-row renewal meaning
abandon, orphans confirmed absent in Postgres in the same pass; not a session advisory lock (ANA-37 Q4). Incremental
cycle 60 s, reading by `updated_at`/`created_at` pulled back by the cache refresh's overlap window
(`cache_overlap_seconds`, 300 s); full reconcile every `concepts_sync_minutes`. Collection names carry no version: a
payload change drops and reindexes (maintainer, 2026-10-20). `Ctrl+R`/`--index-items` take the lease only when free,
otherwise record a request for a pass on the lease row and show the holder's box (ANA-37 Q5). `--search-items` over
several workspaces names those whose search is unavailable.

**Out of scope**
- Notes, scrubbed transcripts, workspace-owned-item specifics and agents' workspace scope for `search_concepts` —
  MOD-112 (4), which plugs into this indexer. The transcript cursor by `(run_step_id, seq)` lands with the transcript
  point type there; MOD-159 leaves the per-source cursor seam it needs.
- Memory point type, near-duplicate candidates — MOD-167; global memory scope — MOD-168.
- Related items at draft time — MOD-161.
- NOTIFY/outbox triggers — revisit with MOD-46.
- A lease preference rule (worker over TUI) — not now (ANA-37 Q5).
- Live-Qdrant runs in the sandbox — owed on the host (sandbox Qdrant times out creating collections).

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Lease and collections | A lease row per collection in Postgres (migration `0036`, mirrored or not as the plan decides); one collection per workspace plus a global collection replace `htui_concepts_v2` (dropped once); the worker's index job and a TUI index job run the full reconcile only under each collection's lease, for every collection; delete batches renew first and abandon on a zero-row renewal; TUI, CLI and MCP searches read the workspace's collection(s) plus the global one; `--search-items` names unavailable workspaces | complete | `.claude/plans/mod-159-m1-lease-and-collections.plan.md` |
| 2 | Freshness | The holder runs a 60 s incremental cycle (rows written since its last pass minus the overlap window) between full reconciles; a new or edited item, requirement, item document or owner document is searchable within a cycle | complete | `.claude/plans/mod-159-m2-freshness.plan.md` |
| 3 | Requests and holder visibility | `Ctrl+R` and `--index-items` take a free lease and index, otherwise record a pass request the holder serves on its next beat and show the holder's box; clean exit releases leases | complete | `.claude/plans/mod-159-m3-requests.plan.md` |

## Open Questions

- [ ] Collection naming (workspace UUID vs slug; slugs can be renamed) — TBD, settle in `/plan` M1 (UUID expected).
- [ ] Whether the lease table is mirrored into the SQLite cache (it is live coordination state; likely not, as the run
  lease columns' use is online-only) — TBD, settle in `/plan` M1 fact-check.
- [ ] Where the TUI's index job lives (store worker vs a dedicated task beside the concepts runtime) and how it loads
  the model lazily so a TUI without Qdrant downloads nothing (`R-NF-2`, MOD-160's ordering) — TBD, `/plan` M1.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Migration number collides with a parallel branch (MOD-143, MOD-171 owe one) | High | Low | Take `0036`; renumber at `scripts/hr collect`, repin `migrations.rs` and `TABLES` (56) |
| Every TUI loading the 133 MB model to contend for leases | Medium | Medium | Take the lease before loading the model; a process that cannot resolve settings never contends |
| Two versions of htui on one Qdrant (old binary still writing `htui_concepts_v2`) | Low | Low | Development only; the new holder drops the legacy collection, an old binary recreating it is harmless |
| Incremental read misses a late commit | Low | Medium | Overlap window as the cache refresh; full reconcile backstop |
| Live-Qdrant tests cannot run in the sandbox | High | Low | Logic tested over `MemVectorStore`; live cases compiled, run on the host before close-out |
| Store conformance and `fake_conformance` count pins move | High | Low | Repin `CASES` (324) with the new lease cases |

---
*Status: COMPLETE — all three milestones landed (M1 `cd65d461..a61f879b`, M2 `b4509a9f..d9020fcf`, M3 `c9edd91a..cfa8d817`); item closed 2026-11-15 (`docs/decisions/mod/mod-159.md`).*
