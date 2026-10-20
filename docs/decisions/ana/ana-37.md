# ANA-37 - Is Qdrant worth requiring (concluded, 2026-10-19)

Full analysis: `docs/ANA-37.md` (surface as read, measurement, options, what was not verified). Origin:
the 2026-10-15 requirements review, `docs/requirements-review-2026-10-15.md`. Addresses `R-NF-2`,
`R-STO-8`, `R-ENT-13`; touches `R-STO-1`, `R-MCP-1`, `R-MCP-2`, `R-SEC-2`, `R-TUI-8`, `R-ENT-12`,
`R-ENT-16`, `R-ID-6` and `R-LATER-7` at their seams.

## Verdict

Knowledge search is a **required workspace service: one shared Qdrant per workspace, required to be
configured, failing open on outage**. Postgres stays the source of truth; the index is derived.

Decided by the maintainer on 2026-10-18, after the first draft (which kept Qdrant optional everywhere):

- **D1.** One knowledge search for the workspace, across its projects.
- **D2.** One shared Qdrant per workspace, like the shared Postgres; never one per worker or box.
- **D3.** Knowledge is items, requirements, decisions, documents, notes, scrubbed transcripts and
  ANA-38's memory; not code, which stays with the code-context tools (`R-LATER-7`).
- **D4.** Required, failing open: htui starts and works, search says "knowledge search unavailable",
  nothing else waits on it.
- **D5.** One indexer per workspace; each searching process embeds its own queries.

The maintainer accepted all eleven answers of `docs/ANA-37.md` §8 on 2026-10-19 ("all as recommended"):

- **Q1.** The Qdrant URL is a workspace setting in Postgres, credentials in a URL refused; the per-box
  keyring `qdrant-url` is offered once, then retired; a development/CI override per process.
- **Q2.** The API key is a secret like the DSN: OS keyring per workspace, systemd credential or stdin
  keyed by workspace slug for a headless worker, a scrubbed environment variable for development/CI.
- **Q3.** One collection per workspace; a run records the workspace it was queued from.
- **Q4.** A Postgres lease row per workspace on the run lease's pattern, renewed before each delete
  batch; a 60 s incremental cycle plus the full reconcile; NOTIFY revisited with MOD-46.
- **Q5.** Worker or TUI may hold the lease, no preference rule; `Ctrl+R`/`--index-items` request a pass
  from a holder.
- **Q6.** Three warned states (no URL, unreachable, no key here), never a block; `search_concepts`
  offered only when the workspace has a URL.
- **Q7.** Related items at draft time: top 5 across the workspace, display-only.
- **Q8.** ANA-38 indexes memory in the shared Qdrant and gets memory into a prompt without it.
- **Q9.** The §10 requirement edits, as written.
- **Q10.** MOD-112 (1)'s read tools gain a text filter that works without Qdrant (agents' fallback).
- **Q11.** `R-MCP-1` widens to notes and, through search, transcript snippets.

## Why

Measured on htui's own corpus (`docs/ANA-37.md` §5): the shipped hybrid search puts a reworded duplicate
in the top 5 for 22 of 24 queries, against 9 for the best lexical baseline without Qdrant (paired
p = 0.0002) and 16 for htui's BM25 leg alone (p = 0.031). Natural-language questions show no
significant gain (10 of 20 against 7, p = 0.58). In the agent trial, grep and read matched semantic
search on all 12 tasks (12/12 both, at ceiling): one project's backlog fits one read. No score
separates duplicates from no-target queries, so nothing is written from a score. The gain at
workspace scale is **unmeasured**; the bar is met by the maintainer's decision on the
workspace-knowledge framing, supported by the duplicate result, and MOD-162 owes the measurement.

## Spawned items

| Filed | From | Subject | Blocked on |
|---|---|---|---|
| MOD-158 | P1 | Workspace Qdrant: URL setting and API key | none |
| MOD-159 | P2 | One knowledge indexer per workspace: lease, freshness, coverage | MOD-158; coverage parts on ANA-35 (MOD-147, MOD-149 on main) and ANA-38 |
| MOD-160 | P3 | Knowledge search fails open cheaply and says so | none |
| MOD-161 | P4 | Related items at draft time | MOD-159; parts on MOD-112 (3), MOD-135, MOD-8 |
| MOD-162 | P5 | Retrieval benchmark in the repo; rerun at workspace scale | none; the rerun needs MOD-112 (4) and MOD-8 |

Effects on open items: MOD-112 part (2) is recorded as existing (`search_concepts`), part (4) keeps
notes, transcripts (no tool calls, tool results or edit proposals), workspace-owned items and agents'
workspace scope and is blocked on MOD-159, part (1) carries the outage text filter; ANA-35 gains the
documents and workspace requirements MOD-159 indexes and MOD-158's `workspace` migration to coordinate
with; MOD-130 shares the `workspace` table with MOD-158 and merges its pending `R-NF-2` sentence into the
new text; ANA-38 takes Q8 as a constraint; MOD-135 gains MOD-161 as a later consumer of its drafts.

## Requirement edits applied

Accepted with Q9 and applied in the conclusion commit (`docs/REQUIREMENTS.md` header line of
2026-10-19): `R-NF-2` and `R-STO-8` replaced (§10.1, §10.2, the bracketed conditions dropped);
`R-MCP-2` (§10.3), `R-ENT-13` (§10.4), `R-STO-1` (§10.5, three edits), `R-TUI-8` (§10.6) and
`R-MCP-1` (§10.7) amended in place. MOD-130's pending `dockerd`/`sshd` sentence was not in this tree's
`R-NF-2`, so it was not merged; MOD-130 carries the note.

## Reconciled with ANA-38 (2026-10-20)

Decided by the maintainer after reading both concluded analyses side by side; it amends D2, Q1, Q2 and
Q3 above.

- **One Qdrant for the whole database**, run like Postgres as its own service (amends D2's "per
  workspace"). Its URL is one database-wide setting in Postgres (Q1); there is one API key (Q2), with
  the same keyring, systemd-credential, stdin and environment-variable sources.
- **Collections (Q3):** one per workspace, plus one global collection for ANA-38's global memory
  entries, never copied into a workspace's. One indexer lease per collection (Q4, Q5 otherwise unchanged).
- **Memory (Q8):** indexed by the lease holder like the rest; a near-duplicate score records a candidate
  and writes nothing, as for items (§5.4, Q7); agents find memory only through knowledge search.
- **Requirements amended:** `R-NF-2`, `R-STO-1`, `R-STO-8`, `R-TUI-8`, `R-ENT-13`, `R-MCP-1`, `R-MCP-2`.
  **Items amended:** MOD-130, MOD-158, MOD-159, MOD-160, MOD-162. ANA-38's side is in
  `docs/decisions/ana/ana-38.md`.

## Commits

`ce55c61f` (first draft), `ac0707d6` (reframed under D1-D5), and the conclusion commit that adds this
file.

## Not verified

See `docs/ANA-37.md` §11: alternative backends deliberately not surveyed; the workspace-scale gain
(MOD-162); real agent use of `search_concepts`; the outage text filter against grep and read; the lease
and freshness design, checked against the schema only; Qdrant resource figures under load; `rten` and a
native Qdrant on Windows.
