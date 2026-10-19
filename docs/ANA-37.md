# ANA-37 - Is Qdrant worth requiring

> **Scope note:** Decide whether Qdrant-backed semantic search (`R-STO-8`) gives htui enough real benefit,
> to agents, triage, duplicate detection and memory, to make the Qdrant daemon and its embedding model a
> required dependency (`R-NF-2`). Narrowed by the maintainer on 2026-10-18 (§1): a measurement of benefit,
> not a survey of alternative backends. Reframed by the maintainer the same day, after the first draft
> (§1, D1-D5): the subject is a knowledge search for the workspace on one shared Qdrant. Governed by
> `.claude/rules/workflow-docs.md`, `docs/REQUIREMENTS.md` and `docs/requirements-review-2026-10-15.md`
> (the item's origin).
>
> **Requirements addressed:** `R-NF-2`, `R-STO-8`, `R-ENT-13`. Touched at their seams: `R-STO-1` (where the
> Qdrant URL and API key live), `R-MCP-1`/`R-MCP-2` (agent read scope, read and search tools), `R-SEC-2`
> (htui's own credentials kept from sessions), `R-TUI-8` (Settings), `R-ENT-12`/`R-ENT-16` (documents and
> items owned by a project or a workspace), `R-ID-6` (drafts a person accepts), `R-NF-1` (Windows),
> `R-PRM-1`/`R-PRM-3` (how context reaches a prompt), `R-LATER-7` (code stays with the code-context tools).
>
> **Status (2026-10-19): concluded; the maintainer accepted all eleven §8 answers as recommended** (D1-D5
> decided 2026-10-18). Decision write-up: `docs/decisions/ana/ana-37.md`. No implementation code is written here. Verdict
> (§7): **knowledge search is a required workspace service: one shared Qdrant per workspace, required to be
> configured, failing open on outage.** The first draft kept Qdrant optional on every box. The maintainer then
> reframed the item (§1, D1-D5) as one search over everything a workspace knows, across its projects: items,
> requirements, decisions, documents, notes, scrubbed transcripts and, once it exists, memory. Code is out: it
> is per worker checkout and stays with the code-context tools. A workspace is expected to have its Qdrant
> configured. When it is missing or down, htui starts and works, knowledge search says "knowledge search
> unavailable", and nothing else is affected. One htui process per workspace runs the indexer, proposed here
> as the holder of a workspace lease in Postgres; each searching process embeds its own queries. The
> measurements stand. On htui's own corpus the shipped hybrid search puts a reworded duplicate in the top 5
> for 22 of 24 queries, against 9 of 24 for the best baseline without Qdrant (paired p = 0.0002) and 16 for
> htui's BM25 leg alone (p = 0.031). Natural-language questions show no significant gain (10 of 20 against 7,
> p = 0.58), and an agent with grep and read matched an agent with semantic search on all 12 trial tasks. That
> trial covered one project's backlog, small enough to read whole; how the gain behaves at workspace scale is
> **unmeasured** (§5.10). The "very beneficial" bar is met by the maintainer's decision on the
> workspace-knowledge framing, supported by the duplicate result; it is not proven for agents at today's size.
> Filed: **MOD-158** (P1), **MOD-159** (P2), **MOD-160** (P3), **MOD-161** (P4), **MOD-162** (P5); the §10
> requirement edits are applied to `docs/REQUIREMENTS.md`.

---

## 1. Context and problem statement

Semantic search exists today and is optional. `R-NF-2` lists Qdrant with its embedding model as one of two
optional services that fail open, "unless ANA-37 finds it worth requiring" (`docs/REQUIREMENTS.md:707-713`).
`R-STO-8` carries the same caveat (`:290-302`). The fail-open shape was built by MOD-34 and kept by MOD-50
and MOD-68; it traces back to ANA-19's invariant 3, "The orchestrator must not hard-fail if the embedding
provider or vector DB is temporarily unavailable" (`docs/ANA-19.md:25`).

The question comes from the 2026-10-15 requirements review. The scribe's record of the maintainer's answer:
"Qdrant: required **if it proves very beneficial**, which an **ANA decides** by measuring what search adds
(agent search tool, triage, dedup, the R-ENT-13 memory). Until then it stays optional and fails open"
(`docs/requirements-review-2026-10-15.md:272-276`). `HANDOFF.md:143` words it "if it is very beneficial".
Neither is a verbatim transcript. Both set the bar at *very* beneficial.

**Maintainer scoping, 2026-10-18.** The maintainer narrowed this item, and the scoping overrides the item
text where they differ. The goal is to establish whether Qdrant-backed semantic search gives **real
benefit** to htui: to agents, triage, duplicate detection and memory. It is **not** a survey of
alternatives: pgvector, embedded indexes and other vector databases are not researched or compared. The
cost side is kept only as far as needed to judge whether the benefit is worth requiring the daemon. The
item text's "against Postgres-only alternatives (`pgvector`, full-text search)" (`HANDOFF.md:146-147`) is
therefore dropped. Lexical search appears below only as the baseline that semantic search must beat.

**Maintainer direction, 2026-10-18 (after the first draft).** The first draft's verdict was "Qdrant stays
optional, the daemon and the embedding model alike, on every box". Having read it, the maintainer
reframed the item as a knowledge search for the workspace on one shared Qdrant. These are decisions, not
proposals, and they override the first draft where they differ:

- **D1. A knowledge search for the workspace.** One search across everything the workspace knows, across
  its projects. Example: an agent in project B asks "have we decided anything about retry limits?" and
  gets the decision taken in project A.
- **D2. One shared Qdrant per workspace,** like the shared Postgres. Never a Qdrant per worker or per box.
- **D3. What counts as knowledge:** items, requirements, decisions, documents (item documents and
  documents owned by a project or the workspace), notes, scrubbed run transcripts, and ANA-38's memory
  once it exists. "Docs for now": htui's documents stored in Postgres. **Not code:** code is per worker
  checkout and stays with Gortex and the other code-context tools (`R-LATER-7`). Markdown files inside a
  repo checkout count as code (per worker) unless imported into Postgres (MOD-8).
- **D4. Required, failing open on outage.** A workspace is expected to have its shared Qdrant configured.
  When it is unconfigured or unreachable, htui starts and keeps working; knowledge search says "knowledge
  search unavailable" and nothing else is affected. No run, prompt or close-out waits on it. htui does not
  refuse to start.
- **D5. Indexing runs in one place per workspace.** Query embedding stays in each process that searches,
  so each searching box keeps the 133 MB model. A central search endpoint is not adopted. How the one
  indexer is chosen is this doc's proposal (§7.3): whichever htui process holds a workspace-wide lease in
  Postgres.

The measurements of §5 stand and are not re-run. §5.10 reads them under D1; §6 to §11 are rewritten for
the new verdict.

Method: three readers in parallel (the code surface and its cost; decision history and the consumers;
published evidence), then two measurements. A retrieval benchmark links htui's own search code against
htui's own corpus (§5.1-5.5). An agent A/B trial gives the same tasks to an agent with grep and read and to
one that also has semantic search (§5.6-5.7). Caveats: `docs/` is not in the Gortex index, so docs were read
from copies byte-identical to `d15e9fea`; the benchmark and trial artefacts live in `/tmp` and are not in
the repo; agents' real use of `search_concepts` could not be counted, because the sandbox cannot reach the
host database.

## 2. Surface as read

### 2.1 What is indexed, by whom, and when

- **One collection for every project.** `htui_concepts_v2`, tenant key `project_id`, every search scoped
  by project (`crates/htui-store/src/vector.rs:1-9, 39, 642`). A layout change means a new collection, not
  a migration (`vector.rs:34-39`).
- **Three kinds of point** (`crates/htui-store/src/vector_sync.rs`): an item as `"{key} {title}\n\n{body}"`
  (`item_point`, `:192`); its latest documents, split at `#`/`##` headings and at 2000 characters
  (`MAX_CHUNK_CHARS`, `:27`; `document_points`, `:237`); and each requirement as `"{key}
  {body}\n\n{rationale}"`, withdrawn ones included (`requirement_point`, `:215`). Payload carries type, key,
  project, status, resolution and a snippet of at most 280 characters.
- **Not indexed, although `R-STO-8` requires it:** notes, scrubbed run transcripts, workspace-owned
  items and, for agents, workspace scope. These are MOD-112 part (4) (`HANDOFF.md:689-696`). Comments,
  links, tags and labels are not indexed either.
- **No write path triggers indexing.** The callers of `Indexer::sync` are: `htui worker`, at start and every
  `concepts_sync_minutes` (default 15), **only if the keyring held a Qdrant URL at worker start**
  (`crates/htui/src/concepts.rs:224-227, 300, 332, 361`); the CLI `htui --index-items` (`concepts.rs:215`);
  and the TUI overlay's `Ctrl+R`, current scope only (`ui/overlay/concepts_search.rs:200`). The TUI never
  syncs on its own. On a box where the TUI is the executor, which is the default, the index goes stale
  until someone reindexes by hand.
- **Consistency with Postgres.** Postgres is the source of truth and the index is rebuildable
  (`vector.rs:6-9`). Each sync is a full, idempotent reconcile per project, with N+1 document reads per
  cycle (`vector_sync.rs:67-129, 90`). There is no transactional coupling: the first error aborts the
  remaining scopes and the next cycle retries. Inferred from code order and not tested: with two indexers,
  A lists items (`:78`) before it scrolls points (`:81`), so an item B indexes in between is deleted by A
  as an orphan until B's next cycle.
- **Embedder identity is enforced** (MOD-68 D8). A collection stamped with another model or width is
  refused with `qdrant: embedder mismatch …` (`check_existing`, `vector.rs:456`). The remedy is manual:
  `curl -X DELETE` the collection, then `--index-items` (`README.md:549-552`).

- **Indexed today, against D3's list.** Search already spans a workspace, but only as a list of its
  projects: the TUI overlay's workspace scope passes each member project's ID to one filter
  (`search_filter`, `vector.rs:642`). The CLI passes `--cwd`'s project, `--project`'s, or by default every
  workspace's projects (`search_projects`, `crates/htui/src/concepts.rs:589-612`). Nothing is owned by a
  workspace yet.

| D3 knowledge | In Postgres today | Indexed today | Who closes the gap |
|---|---|---|---|
| Items (key, title, body) | yes | yes | — |
| Decisions (items closed `done`, `concluded`, `rejected`) | yes, as items | yes, with a decisions filter | — |
| Requirements | project-owned only (`requirement.project_id NOT NULL`, `migrations/0006_requirements.sql:70-72`) | yes | workspace requirements: ANA-35 (`R-ENT-14`, `R-ENT-16`) |
| Item documents | yes (`document.item_id NOT NULL`, `migrations/0001_init.sql:389-391`) | latest version, chunked | — |
| Documents owned by a project or the workspace | no; `document` has only an item owner | no | ANA-35 (`R-ENT-12`), then indexing |
| Notes | yes (`item_note`, `0001_init.sql:374`) | no | MOD-112 (4) |
| Scrubbed run transcripts | yes (`session_event`, `0001_init.sql:513`; scrubbed before a row persists, `R-SEC-3`) | no | MOD-112 (4) |
| Workspace-owned items | no | no | ANA-35, then MOD-112 (4) |
| Memory | no | no | ANA-38 |
| htui's own markdown backlog and decisions | no; files in the checkout | no; MOD-34 dropped them | MOD-8 imports them; until then they count as code (D3) |
| Code | — | no, and stays out (D3) | the code-context tools (`R-LATER-7`) |

### 2.2 What runs in htui and what needs the daemon

The dense model is `Xenova/bge-small-en-v1.5`, pinned and sha256-checked, 133,093,490 bytes, 384
dimensions, English only, run in-process on `rten` (`crates/htui-store/src/model.rs:23-55`;
`embed.rs:1-7, 17`). It is downloaded on first use into the user cache, never at build time
(`model.rs:152, 212-213`). The sparse half is BM25 computed in htui with no vocabulary file; `mod-34`,
`mod` and `34` are all terms; IDF is applied by Qdrant through `Modifier::Idf` (`bm25.rs:1-7`;
`vector.rs:887-910`). Fusion is server-side RRF over a dense and a sparse prefetch of `max(4×limit, 20)`
each (`vector.rs:1059-1110`).

| Needs only htui's own process (the embedder) | Needs the Qdrant daemon |
|---|---|
| Model fetch and load, embedding documents and queries, BM25 terms and weights, chunking, change detection on the Postgres side | Storage, dense nearest-neighbour search, sparse search **and its IDF**, RRF fusion, payload filters, the scroll change detection reads, the embedder stamp |

There is no fallback search in htui. `MemVectorStore` and `HashEmbedder` are test-only
(`vector.rs:1124-1130`; `embed.rs`). Without Qdrant the embedder is useless; without the embedder Qdrant
cannot be queried, because query vectors are built in htui. Requiring one means requiring both.

### 2.3 Every consumer today

| Consumer | Path | Scope | Available when |
|---|---|---|---|
| TUI overlay "Search concepts" (`Ctrl+F`) | `concepts_search.rs:224` → `QdrantIndex::search` (`concepts_worker.rs:233`) | workspace or one project; decisions toggle; 10 hits | a URL is in the TUI box's keyring and an index exists |
| CLI `htui --search-items` | `concepts.rs:452, 623`; read-only, missing collection answers `empty` | every workspace, or one project | same |
| MCP `search_concepts` for agents (MOD-11) | `crates/htui-mcp/src/tools/search.rs:63-111`; adapter `crates/htui/src/mcp_search.rs:21-56` | **the session's project only**; no decisions filter | TUI: always advertised. Worker: only if the keyring held a URL at start (`worker_cmd.rs` ~98-131) |
| Duplicate detection | **no code**; `Resolution::Duplicate` is a manual close reason | — | — |
| Triage | **no code** | — | — |
| Prompt seeding, the coordinator | **none**; no search call in `htui-orch`, `htui-worker`, `htui-agent` or `htui-core/src` | — | — |
| Memory (ANA-38) | not built (`HANDOFF.md:149-157`; `R-ENT-13`, `docs/REQUIREMENTS.md:221-225`) | — | — |

A hit gives an agent a key, a point type, a score and a snippet of at most 280 characters
(`tools/search.rs:1-8`). **The MCP server has no tool to open a hit.** Its eight tools are `box_profile`,
`document_write`, `note_add`, `item_status`, `item_link`, `search_concepts`, `command_run` and
`permission_prompt` (`ALL`, `crates/htui-mcp/src/tools/mod.rs:45-54`); read tools are MOD-112 (1). No prompt
tells an agent the tool exists; the only signal is its description, "Searches this project's items, documents
and requirements by meaning."

Without Qdrant, the lexical baseline htui has today is narrow: items match by case-insensitive substring
on **key and title only** (`crates/htui-store/src/pg/read.rs:105-110`), requirements on key and body
(`pg/read.rs:948`), documents, notes and transcripts not at all. There is no Postgres full-text search in
the code.

**Cross-project search today.** People can already search across projects: the overlay's workspace
scope passes the member projects' IDs to one filter, and the CLI's default passes every workspace's
projects (§2.1). Agents cannot: `search_concepts` is fixed to the session's project (`McpSearch::search`,
`projects: vec![query.project]`, `crates/htui/src/mcp_search.rs:44`),
although `R-MCP-1` lets an agent read workspace items, documents and requirements. So D1's example, an
agent in project B finding project A's decision, fails today on scope alone, before any question of
meaning.

### 2.4 Fail-open paths

- **TUI overlay.** With no URL it says so within microseconds and downloads nothing (`concepts.rs:97`).
  With a URL set, `connect` loads the model **before** contacting Qdrant, so a first search against a dead
  Qdrant can download 133 MB and then fail. It uses `connect`, not `open_existing`, so a TUI search
  **creates** the collection on an empty Qdrant; the CLI does not (`concepts_worker.rs:199-215`). Errors
  stay inside the overlay box (`concepts_worker.rs:35-48`).
- **MCP `search_concepts`.** Not advertised when no index is attached
  (`crates/htui-mcp/tests/tools_search.rs:345-358`); otherwise any failure is `isError` with
  `search unavailable: <cause>` (`tools/search.rs:97-110`). **The TUI always attaches an index:**
  `production()` is "Always `Some` in this build; a box without Qdrant gets `search unavailable`"
  (`crates/htui/src/mcp_search.rs:87-92`), and the TUI attaches it unconditionally
  (`crates/htui/src/lib.rs:207-211`). So in a TUI-hosted session on a box without Qdrant the tool is
  offered and every call errors, and `Ctrl+F` opens with the "no Qdrant URL" message. A worker attaches
  the index only if its keyring held a URL at start. The run is unaffected either way.
- **Worker index job.** No URL: one `info` line, no job, no `search_concepts`; a URL stored later needs a
  restart. Qdrant down: `warn` and retry every interval. Embedder mismatch: one `error`, sent to GlitchTip
  (`concepts.rs:332-417`; `docs/htui-worker.md` "Error reports"). Runs never wait on it
  (`docs/htui-worker.md:408-422`).
- **CLI.** `--search-items` exits 0 on hits or `empty`, 3 offline, 2 refused, 1 on a model failure
  (`concepts.rs:503, 623-660`; `state_cmd.rs:583-598`). `--index-items` runs before any terminal work.
- Qdrant errors are always `StoreError::Backend`, never `Unreachable`, so they cannot pass for a Postgres
  outage (`vector.rs:7-9`). Failing open holds, and it holds trivially: nothing else depends on Qdrant.

### 2.5 Configuration and deployment

- **One shared Qdrant can serve every box.** The URL is any `http(s)://` address with an optional API key
  (`qdrant_settings.rs:25-33`) and the collection is multi-tenant. Each box then needs network access,
  the URL in its keyring, and its own 133 MB model cache. Every worker with a URL re-syncs every project
  every 15 minutes: redundant, idempotent, with the race of §2.1.
- **The URL lives only in the OS keyring** (`secret.rs:43-45, 304-314`). Settings already has a Qdrant section
  that edits this box's URL and a masked API key in the keyring (`crates/htui/src/ui/tabs/settings/qdrant.rs`;
  "set one in Settings > Qdrant", `concepts.rs:97`). There is no env, stdin, file or systemd-credential
  source. The DSN also has `--dsn-stdin`, a systemd credential and, for development and CI, an environment
  variable (`docs/REQUIREMENTS.md:257-269`; `docs/htui-worker.md:380-399`); it never has a file source. So a
  systemd or provisioned worker **never** indexes and never offers `search_concepts`
  (`docs/htui-worker.md:240-241, 415-417`), and an hr sandbox, which has no keyring, searches only in tests.
  Requiring Qdrant on a headless worker first needs code that does not exist.
- **Dev and test.** `compose.yaml:53-62` runs `qdrant/qdrant:v1.19.1` pinned to the client, and
  `README.md:129` calls it "only needed for Search". `scripts/hr` gives each run its own Qdrant
  (`docker/hr/compose.hr.yaml:35-60`). `qdrant_live` and `qdrant_worker` print a skip line and pass when
  `HTUI_TEST_QDRANT_URL` is unset, so CI without Qdrant passes them vacuously.
- **Windows.** Qdrant v1.19.1 ships `qdrant-x86_64-pc-windows-msvc.zip`, about 29.7 MB
  (https://api.github.com/repos/qdrant/qdrant/releases/tags/v1.19.1). htui documents only
  `docker compose up -d` (`README.md:120-129`), which on Windows means Docker Desktop. `rten` was measured
  on Linux x86-64 only (ANA-23 §9); MOD-16 is open. The headless worker is Linux-only.
- **No place in Postgres names a workspace's Qdrant.** The URL is per box, in each keyring. The
  `workspace` table has no settings column (`migrations/0001_init.sql:129-137`); `project.settings` is per
  project (`:150`) and `app_setting` covers the whole database (`:557-561`). One Postgres holds several
  workspaces, and a project can belong to more than one (`workspace_project`, primary key
  `(workspace_id, project_id)`, `:161-166`). The Infisical base URL is kept the same per-box way
  (`INFISICAL_URL_USER`, `secret.rs:47`).

## 3. Constraints from sibling items

1. **MOD-112** (`HANDOFF.md:689-696`). Part (1) adds read tools with workspace read scope; part (2) "a
   search tool over R-STO-8's index" already exists as `search_concepts`; part (3) is `item_create` drafts;
   part (4) indexes notes and transcripts and adds workspace scope. MOD-112 names no lexical search tool, so
   without Qdrant an agent would still have no way to find by words.
2. **ANA-38** (`HANDOFF.md:149-157`) owns memory, including its "integration with Qdrant (indexed,
   searched, deduplicated)". ANA-1's scored facts were injected by score, not by search, and dropped on
   2026-09-03 as over-engineered (`docs/decisions/ana/ana-1.md:18-19`). ANA-37 can constrain ANA-38; it
   does not design it.
3. **ANA-34 (concluded).** Coordinator turns are seeded from store rows; memory joins the seed when it
   lands; the P4 coordinator tools include no search (`docs/ANA-34.md:489-491, 556-560, 671`). Its P3
   follow-up drafts (MOD-135, `:670`) are a natural place for a duplicate check.
4. **ANA-30** rules out automatic triage (X5, `docs/ANA-30.md:366`) and model-proposed duplicate links,
   which conflict with `R-ENT-9`; "display-only 'possible duplicates' from Qdrant needs no model" (X4,
   `:365`). Model-ranked prompt excerpts are ruled out (X2, `:363`).
5. **`R-ID-6`** allows a suggested link or a proposed item only as a draft a person accepts
   (`docs/REQUIREMENTS.md:113-118`).
6. **`R-PRM-1`/`R-PRM-3`.** Prompt assembly is deterministic, built from edges and excerpts under a
   budget, with no search step (`docs/REQUIREMENTS.md:463-468`). Semantic search reaches a prompt only
   through an agent's own tool calls.
7. **MOD-8** imports the markdown backlog and decisions into Postgres; it is open and "this repository is
   its first user" (`HANDOFF.md:716-723`). Until it lands, htui's own history is not in the shipped index.
   MOD-34 dropped indexing the repo's markdown (`docs/decisions/mod/mod-34.md:28-30`).
8. **`R-STO-1`** keeps the connection string and provider identities in the keyring, with a
   systemd-credential or `--dsn-stdin` exception for a headless worker's (R-ORCH-12) DSN on a Linux host
   with no secret service, and an environment variable for development and CI
   (`docs/REQUIREMENTS.md:257-269`). The Qdrant URL and API key are not in its list.
9. **`R-NF-1`** requires Windows 10+ (`docs/REQUIREMENTS.md:705-706`); MOD-16 (Windows verification) is
   open.
10. **ANA-35** (`HANDOFF.md:123-135`) settles workspaces that own items, documents and requirements, and
    documents owned by a project or a workspace. D3's workspace-owned knowledge cannot be indexed before it
    exists.
11. **`R-MCP-1`** lets an agent read beyond its run, workspace items, documents and requirements included
    (`docs/REQUIREMENTS.md:511-513`). Search at workspace scope stays inside it for those; notes and other
    runs' transcripts fall outside it (Q11). **`R-SEC-2`**
    keeps `htui`'s own credentials from every session (`:494-496`): a Qdrant API key, like the DSN, must
    never reach an agent's environment.

## 4. Prior art and evidence

### 4.1 htui's own record: the benefit was claimed, never measured

- **ANA-19 (2026-09-19)** claimed the problem: "it becomes harder for an agent (or human) to find related
  concepts using exact-match text search alone" (`docs/ANA-19.md:13-17`). The corpus it had in mind was
  htui's own markdown (`:13, 47`). Its "Beneficial: Yes" (`:60`) rests on engine latency and filtering;
  retrieval quality was not measured or estimated.
- **ANA-20 (2026-09-20)** added that search "must reliably retrieve exact identifiers (e.g. 'MOD-34')" and
  made BM25 "formally necessary" because dense embeddings lose exact IDs (`docs/ANA-20.md:21-22, 43-46`).
  Its own design treats lexical matching as the part that must not fail.
- **MOD-34 (2026-09-25)** chose "Postgres items only" and dropped the markdown corpus ANA-19 argued from
  (`docs/decisions/mod/mod-34.md:28-30`).
- **No quality measurement anywhere.** ANA-19/20/23 and the decision records for MOD-34/50/64/68/11 hold no
  recall, precision or relevance figure. The live tests embed with `HashEmbedder`, "whose dense arm is
  noise" (`docs/decisions/mod/mod-50.md:57-59`); MOD-68's goldens pin vector equality, not usefulness
  (`docs/decisions/mod/mod-68.md:38-43`).
- **No observed use.** From MOD-34 (09-25) to MOD-68 (10-01) "a fresh install cannot load the model"
  (`docs/ANA-23.md:92-105`). An analysis found that on the way; no user reported it. That is indirect
  evidence that nobody depended on search for six days.

### 4.2 Published evidence

Labels: **[MEASURED]** reproducible or peer-reviewed numbers, **[VENDOR]** a company about its own
product, **[OPINION]** statements without numbers.

| Use | Evidence | Source |
|---|---|---|
| Duplicate bug reports | [MEASURED] BM25F-based REP beat deep-learning methods by 22.3% RR@10 on average; lost only on the two largest datasets | https://arxiv.org/html/2212.00548v1 |
| | [MEASURED] REP plus an LLM reranker: +5 to 8% over REP | https://arxiv.org/abs/2308.10022 |
| | [MEASURED] off-the-shelf SBERT recall@5 26-36% against TF-IDF 13-24%; BM25 not tested | https://arxiv.org/html/2308.09193 |
| | [MEASURED] BM25 recall@100 is 10.2-18.5 points lower on textually dissimilar duplicates; term expansion recovers much of it | https://arxiv.org/pdf/2212.09976 ; https://dl.acm.org/doi/full/10.1145/3756681.3756978 |
| | [VENDOR] Sentry: lexical fingerprint first, ML fallback; v2 prevents 20% more duplicates | https://blog.sentry.io/enhancing-issue-grouping |
| Coding-agent retrieval | [VENDOR] Cursor: semantic plus grep +12.5% offline QA; online retention +0.3% overall, +2.6% at 1,000+ files | https://cursor.com/blog/semsearch |
| | [VENDOR, staff] Cursor then removed semantic indexing (Jul-Sep 2026): models "have gotten really good at using grep"; no measured comparison published | https://forum.cursor.com/t/what-do-you-think-about-cursor-removing-the-codebase-indexing-settings/165899 |
| | [OPINION] Claude Code: "agentic search generally works better"; Sourcegraph Cody dropped embeddings for operational reasons, no quality comparison | https://x.com/bcherny/status/2017824286489383315 ; https://sourcegraph.com/blog/how-cody-understands-your-codebase |
| Agent memory | [MEASURED] Mem0 paper: full context 72.9% against Mem0 66.9% on LoCoMo; Mem0's case is cost | https://arxiv.org/abs/2504.19413 |
| | [MEASURED] independent: LoCoMo 57.7 against 92.9 for long context; LongMemEval 49.0 against 82.4 | https://arxiv.org/html/2603.04814v1 |
| | [MEASURED] LongMemEval: long-context models drop 30-60% at ~115k-token histories; retrieval helps there | https://arxiv.org/abs/2410.10813 |
| | [VENDOR] Anthropic: under 200,000 tokens, "include the entire knowledge base in the prompt… no need for RAG" | https://www.anthropic.com/engineering/contextual-retrieval |
| Identifiers | [MEASURED] rare entities: DPR 49.7% against BM25 71.2% top-20 | https://arxiv.org/abs/2109.08535 |
| Hybrid | [MEASURED] fusion gains shrink as the dense model gets stronger | https://arxiv.org/pdf/2604.03676 |

Lessons that bind:

- **Lexical search is a strong baseline and is mandatory for identifiers.** Dense-only retrieval fails on
  exact IDs, and htui's text is full of them (`MOD-34`, `R-NF-2`).
- **Dense retrieval's structural advantage is vocabulary mismatch:** a duplicate worded differently from
  the original. The gains reported are moderate, a few to about 15 recall points, and come from corpora of
  10k to 500k reports. No study covers a corpus of htui's size and shape.
- **For agents, the measured case was withdrawn.** Cursor's gain was largest on large corpora and was
  removed once models grew good at iterative grep. This is evidence against requiring a semantic backend
  for agent search alone; how far it transfers to htui's prose (notes, transcripts) is open.
- **For memory at small size, injection beats retrieval on accuracy.** Retrieval pays off past roughly
  100-200k tokens of history, or on cost at many queries per context. The maintainer's own Claude Code
  memory for this repo is 40 files, about 12k words, loaded whole with no search, and it works at that
  size.

## 5. Measurement

### 5.1 Retrieval benchmark: setup

**Harness.** A scratch crate (`/tmp/ana37-benchmark/harness/`) links `crates/htui-store` (feature
`local-embed`) and `crates/htui-core` as path dependencies, with the repo's `Cargo.lock`, toolchain 1.98.1,
release build. Reused unchanged: the pinned model fetch (`model::ensure_model`, `model.rs:152`), the `rten`
embedder (`embed.rs:153`), BM25 vectors (`bm25.rs:71, 90`), collection creation (`vector.rs:721, 891-896`),
upsert (`vector.rs:987`), **the production search** (`QdrantStore::search`, `vector.rs:1059`, prefetch depth
`:1075`, `Fusion::Rrf` `:1084`), document chunking (`vector_sync::chunk_markdown`) and the point text
formats (`vector_sync.rs:199, 228, 238`). Reimplemented: point construction, because there are no Postgres
item rows (each decision is a closed item plus a `summary` document holding its decision file).

**Backends.**

| Name | What it is |
|---|---|
| **S** | htui's hybrid search at limit 40, collapsed to one row per ID |
| S10 | the same at htui's default limit of 10 (`DEFAULT_LIMIT`, `crates/htui/src/concepts.rs:65`) |
| S-dense, S-sparse | the two prefetch arms of `vector.rs:1059`, run alone with the same vectors and filter |
| L-and, L-or | Postgres `websearch_to_tsquery('english')` with `ts_rank_cd(…, 32)`; L-or replaces `&` by `\|` |
| L-*-len | the same, length-normalised (rank normalisation 2\|32), added as a fairness check |
| G, G-dens | a grep-style count of matched non-stopword keywords; ties broken by occurrences, or by density |

L and G are lexical baselines, not candidate backends: htui ships neither (§2.3), and per the scoping they
are not evaluated as designs. They stand for "what word matching finds".

**Corpus.** 322 documents, 1.04 M characters: 71 open items from `HANDOFF.md` (77,058 characters), 121
requirements from `docs/REQUIREMENTS.md` (51,133) and 130 decision files from `docs/decisions/**`
(847,144). In Qdrant this is 1,081 points: 201 item, 121 requirement, 759 document.

**Queries.** 72, frozen in `queries.json` (sha256 `56679bac…`) before any backend ran: (a) 24
paraphrased descriptions of an existing item, the duplicate case; (b) 20 natural-language questions with
vocabulary mismatch; (c) 16 identifier lookups; (d) 12 with no correct target.

### 5.2 Results

Run 2 is the reference. Each cell is recall@1 / recall@5 / MRR@10.

| Backend | a duplicates (24) | b questions (20) | c identifiers (16) | a+b+c (60) |
|---|---|---|---|---|
| **S** | 0.42 / **0.92** / 0.59 | 0.30 / 0.50 / 0.38 | 0.88 / 0.94 / 0.90 | 0.50 / **0.78** / 0.60 |
| S10 | 0.46 / 0.92 / 0.61 | 0.30 / 0.40 / 0.36 | 0.81 / 0.94 / 0.86 | 0.50 / 0.75 / 0.59 |
| S-dense | 0.38 / 0.71 / 0.55 | 0.30 / **0.60** / 0.42 | 0.56 / 0.81 / 0.67 | 0.40 / 0.70 / 0.54 |
| S-sparse | 0.25 / 0.67 / 0.39 | 0.15 / 0.40 / 0.25 | 0.88 / 0.88 / 0.89 | 0.38 / 0.63 / 0.47 |
| L-and | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 | 0.81 / 0.81 / 0.81 | 0.22 / 0.22 / 0.22 |
| L-or | 0.08 / 0.08 / 0.10 | 0.05 / 0.25 / 0.13 | 0.69 / 0.81 / 0.73 | 0.23 / 0.33 / 0.28 |
| L-and-len | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 | 0.69 / 0.81 / 0.75 | 0.18 / 0.22 / 0.20 |
| L-or-len | 0.08 / 0.38 / 0.21 | 0.05 / 0.35 / 0.15 | 0.69 / 0.88 / 0.77 | 0.23 / 0.50 / 0.34 |
| G | 0.08 / 0.17 / 0.12 | 0.20 / 0.20 / 0.23 | 0.69 / 0.81 / 0.76 | 0.28 / 0.35 / 0.33 |
| G-dens | 0.08 / 0.29 / 0.17 | 0.20 / 0.35 / 0.26 | 0.69 / 0.88 / 0.78 | 0.28 / 0.47 / 0.36 |

L-and returns nothing for 23 of 24 duplicate queries: it ANDs every term, so a long description matches no
document. L-or and G are dominated by long decision files.

Recall@5 with Wilson 95% intervals: duplicates S 22/24 (0.74-0.98) against L-or-len 9/24 (0.21-0.57) and
G-dens 7/24 (0.15-0.49); questions S 10/20 (0.30-0.70), S-dense 12/20, L-or-len 7/20; all 60, S 47/60
(0.66-0.87) against L-or-len 30/60.

Paired exact McNemar tests on recall@5:

| Comparison | S only | other only | p |
|---|---|---|---|
| Duplicates: S vs L-or-len | 13 | 0 | 0.0002 |
| Duplicates: S vs G-dens | 15 | 0 | 0.0001 |
| Duplicates: S vs S-sparse | 6 | 0 | 0.031 |
| Duplicates: S vs S-dense | 5 | 0 | 0.063 |
| Questions: S vs L-or-len | 8 | 5 | 0.58 |
| All: S vs L-or-len | 22 | 5 | 0.0015 |
| All: S vs S-dense | 8 | 3 | 0.23 |

Where the gain comes from:

- **Duplicates.** htui's BM25 leg alone, still lexical but with Qdrant's IDF, reaches 0.67, well above
  the best Postgres variant (0.38). The dense leg adds the rest: fusion reaches 0.92.
- **Questions.** Fusion hurts. Dense alone gets 0.60, the hybrid 0.50. On b03, b06, b11 and b12 one leg
  finds the target and the fused list loses it. On questions the dense leg alone finds 3 targets the
  hybrid misses (b03, b06, b12) and the hybrid finds 1 the dense leg misses (b01): p ≈ 0.63, not
  significant, and run-to-run tie reordering moves this class by up to one query (§5.9). Over all 60
  queries the hybrid beats dense 8 to 3 (p = 0.23).
- **Identifiers.** The sparse leg carries them (0.88). Dense alone drops to 0.56 at recall@1 and misses
  c09 (ConPTY) and c11 (Plackett-Luce) from the top 5 entirely.

### 5.3 Concrete differences

Ranks are S / S-dense / S-sparse / L-or-len / G-dens; "–" is outside the top 10.

**S finds it in the top 5 and all six lexical variants miss (13 queries):**

| Query | Target | Ranks |
|---|---|---|
| a01 "crash telemetry address is hard-wired…" | MOD-99 | 1/–/1/–/– |
| a03 "save the process return status of every agent invocation" | MOD-101 | 3/2/–/6/– |
| a04 "flat-rate plan… provider credentials from my shell" | MOD-97 | 5/7/5/–/– |
| a08 "docs checker should fail when a write-up does not mention requirements" | TOOL-13 | 1/1/1/6/8 |
| a09 "never two repositories flagged as the main one" | MOD-106 | 1/1/3/6/– |
| a11 "choose among assistants with odds proportional to strength" | MOD-36 | 4/2/–/–/– |
| a15 "bring markdown backlog and decision log into the database" | MOD-8 | 3/2/3/7/– |
| a18 "pause work in progress and pick it up later" | MOD-125 | 3/2/–/7/– |
| a20 "check out a repository onto a machine… keep the git login" | MOD-108 | 1/1/3/–/– |
| b01 "where do we decide which machine runs a step" | ANA-42 / R-ORCH-15 / MOD-130 | 5/–/3/–/– |
| b07 "which decision chose the protocol we use to talk to coding assistants" | DEC:ANA-4 | 1/1/4/–/– |
| b15 "what happens to running work when I close the app" | R-TUI-18 / DEC:MOD-41 | 1/1/6/8/– |
| b20 "how do agents split one task into pieces that run at the same time" | R-ORCH-7 / ANA-33 | 1/1/2/8/9 |

htui's sparse leg alone also finds 9 of these 13. Only a03, a11, a18 and b15 need the dense leg.

**A lexical variant finds it and S misses (7 queries, all questions):** b03 "keep passwords out of the
logs"; b04 "can two people work on the same backlog" (L-or-len rank 1; S, dense and sparse all miss);
b05 "decide against an in-memory cache server"; b09 "how are crash reports collected"; b11 "engine for
finding similar tickets by meaning"; b12 "finished work merged back into the main branch"; b14 "big
compile jobs queued so the computer is not overloaded" (G rank 1; S misses).

**Every backend misses the top 5:** a05 (MOD-94), a10 (MOD-98), b08 (R-ID-4), b10 (R-NF-3), and c16
"R-STO-8", whose own requirement ranks 7th behind items that cite it.

### 5.4 Can a score flag a duplicate on its own?

No. Separability of each query's top-1 score, true duplicates (a, n = 24) against no-target queries (d,
n = 12):

| Score | AUC | Recall with zero false alarms |
|---|---|---|
| S hybrid RRF | 0.67 | 0.25 |
| S-dense cosine | 0.58 | 0.17 |
| S-sparse per query term | 0.47 | — |
| L-or-len | 0.59 | 0 |
| G / G-dens fraction matched | 0.73 | 0.08 |

Raw BM25 shows an AUC of 0.91, but that is a query-length artefact: class a queries are longer (median 17
terms against 11); per query term it falls to 0.47. The RRF score has no absolute meaning; htui's own doc
says it is "comparable only within one search" (`vector.rs:352`). A dense cosine of 0.75 or more fired on
2 of 24 duplicates and 0 of 12 no-target queries; at 0.70 it fired on 12 (7 correct) and 4 false. The
usable form is "show the top 5 related items" for a person or an agent to judge, not an automatic flag.

### 5.5 Measured cost

- **Query latency** on a loaded shared host (load average 11-15, 4-CPU quota), p50/p95 in ms over three
  runs: S 8.8-16.6 / 11.8-27.0; query embedding alone 6.0-13.4 / 8.5-22.7; S-sparse 0.82-1.49 / 1.36-3.0;
  L-or 2.9-4.0 / 7.6-11.1. Relative, not absolute.
- **Indexing.** 1,081 points embedded in 130.5 s, 120.7 ms per point at 4 threads; BM25 vectors for all of
  them 0.026 s. First model load 15.8 s including the 133 MB download.
- **Index size.** The Qdrant snapshot is 8.54 MB; Qdrant's resident memory with only this collection is
  83.7 MB. An empty Qdrant 1.19.1 measured 67 MB resident (`/telemetry`, 2026-10-18).
- **Embedder in a process.** The benchmark's `semsearch` CLI peaks at 154 MB RSS, cold call 3.8 s, warm
  0.35-0.5 s. ANA-23's spike measured +8.1 MiB of binary and about 148-182 MB peak RSS (`docs/ANA-23.md`
  §5.3-5.4); the real htui binary was not measured.

### 5.6 Agent A/B trial: setup

Twelve tasks, each phrased as a person's request to an agent, with ground truth written before the runs
(`/tmp/ana37-trial/tasks.json`):

- **Duplicate check (5):** "is this already filed? give its ID". Targets MOD-114, MOD-107, MOD-102,
  MOD-135, and the shipped DEC:MOD-89. Paraphrases avoid title words; traps are planted (R-BOX-3 for
  "tags", R-LATER-12 for notifications).
- **Vocabulary mismatch (4):** a question whose answer uses other words. Targets R-STO-5 with DEC:MOD-40,
  DEC:MOD-79, DEC:MOD-62, and MOD-108 (trap: MOD-134, which is blocked on MOD-108).
- **None exists (3):** screen-reader support, dependency-advisory scanning, personal favourites. Each has
  near-miss distractors.

Arm A had grep and read only. Arm B had the benchmark's `semsearch` CLI (htui's hybrid search over the
§5.1 collection) plus grep and read. Corpus: `HANDOFF.md`, `DECISIONS.md`, `docs/REQUIREMENTS.md` and
`docs/decisions/` on `hr/ANA-37` at `d15e9fea`. One run per arm, same model. Every cited ground-truth line
was re-checked by the grader and found accurate. Grading: lenient (the primary expected ID present, no
wrong ID asserted with confidence above 0.5) is the headline; strict (every expected ID) is a secondary
view, harsher than the prompts, which ask for one item's ID.

### 5.7 Trial results

| Class (n) | A lenient | B lenient | A strict | B strict | B strict, prose credit |
|---|---|---|---|---|---|
| Duplicate check (5) | 5/5 | 5/5 | 3/5 (+2 partial) | 3/5 (+2 partial) | 4/5 (+1 partial) |
| Vocabulary mismatch (4) | 4/4 | 4/4 | 3/4 (+1 partial) | 3/4 (+1 partial) | 3/4 (+1 partial) |
| None exists (3) | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| **All (12)** | **12/12** | **12/12** | **9/12** | **9/12** | **10/12** |

| Tool calls | A | B |
|---|---|---|
| Total | 23 | 28 |
| Mean per task | 1.92 | 2.33 |
| Mean excluding hook-blocked calls | 1.58 | 2.00 |

- **No accuracy difference.** The final ID never differs between the arms under either rule. Neither arm
  fell into a trap or asserted a wrong ID.
- **Semantic search rescued nothing.** Arm A solved every vocabulary-mismatch task with grep and read.
- **B spent about 0.4 more calls per task** and its answers carry slightly more detail and secondary
  references: R-TUI-11 for t01, `.envrc` and `allow_env_files` for t05, the 0600/0700 fix for t07, three
  rejected near-misses for t12.
- **A read `HANDOFF.md` once and reused it** for t01, t02, t03 and t09. With the backlog in context, later
  tasks cost about one call. At this size an agent reads the whole backlog.

### 5.8 How often, from htui's own history

About 200 items were minted between 2026-09-03 and 2026-10-18. Of 130 decision records, none closed as
`duplicate`. Reading the history for overlaps found 16 instances: about 9 overlapping work items, roughly
4-5% of items. Only two plausibly needed recall across different wording: MOD-61 folded into MOD-10
("scrubber masks nothing" against "Secret provider", `DECISIONS.md:82`; `docs/decisions/mod/mod-61.md:1-4`)
and MOD-27's swarm overlapping divided fan-out (`HANDOFF.md:215-217`). So at least about 1% (2 of ~200)
of items had an overlap that was noticed and needed recall across different wording. This is a lower
bound, not a rate: the history counts only overlaps someone noticed, and the noticed ones were found by
reading the whole backlog. The benchmark points the other way: the best no-Qdrant baseline misses 15 of
24 reworded duplicates, 13 of which the hybrid finds (§5.2), which is what undetected duplicates would
look like. The rest of the 16 instances were caught by IDs in text or code, by file overlap, or by
reading the whole backlog in a review. No duplicated
code shipped; the misses cost a re-scope or a fold. One instance is this item's own text: it re-asked
pgvector, which ANA-19 §3.1 had already rejected (`docs/ANA-19.md:36`), and the word "pgvector" matches
lexically; nobody searched. These counts are over markdown the shipped index does not hold until MOD-8.
They are evidence about the need, not about the shipped index.

### 5.9 Threats to validity

Benchmark:

- **The query author knew the targets.** Class a avoids title words, which biases against lexical
  backends; class c uses distinctive terms, which favours them.
- **Small samples.** 24/20/16/12 queries give wide intervals. Only the duplicate result and the a+b+c
  result against L and G are statistically clear.
- **Small, skewed corpus.** 322 documents, 87% of characters in long decision files, which crowd both
  lexical rankings and S's top hits (759 chunk points).
- **Not htui's real data shape.** Items and decisions are not yet rows (MOD-8 is open); the point
  construction is a model. `docs/ANA-*.md`, notes and transcripts, which `R-STO-8` also covers, are not in
  the corpus.
- **The baselines are not htui's baseline.** htui without Qdrant matches items on key and title only
  (§2.3); that was not run. L and G read bodies and so are stronger than what htui has today.
- **L and G carry no IDF weighting and are untuned.** The only IDF-weighted lexical ranker measured is
  S-sparse, htui's BM25 leg, whose IDF Qdrant computes. So "9 of 24 without Qdrant" is what untuned
  word matching finds, and the dense leg's own share of the duplicate gain is 16 to 22 of 24 (hybrid
  against S-sparse), not 9 to 22.
- **G is one grep pass.** A real agent iterates, rephrases and greps the definition pattern (`**R-NF-2`).
  G understates agent grep, especially for identifiers.
- **Hybrid results are not deterministic.** Tied RRF scores are ordered arbitrarily: 34 of 72 queries had a
  different top 10 between runs 2 and 3. Metrics moved by at most one query per class. The single legs are
  deterministic.
- **Latency** was measured on a loaded, shared host.

Trial:

- **n = 12, one run per arm, ceiling effect.** A 12/12 tie cannot separate the arms and gives no variance.
- **The corpus is small enough to read whole,** which favours grep and read. The advantage of semantic
  search should grow with corpus size; the trial does not test that.
- **Arm A's grep is not an htui agent's grep.** Arm A grepped markdown files. An agent in htui works over
  Postgres rows through MCP and has no grep there; today it has no read tool either (§2.3). The trial shows
  what a lexical, read-capable tool gives, not what htui's agents have now. §7.4's outage fallback
  assumes MOD-112's text filter performs about as well as arm A's grep and whole-file read. An MCP filter
  over rows, with no bulk read, is likely weaker, and arm B also had grep and read, which htui's agents
  lack; so the trial's tie may overstate how little semantic search adds for an htui agent.
- **The task author knew the targets;** paraphrases may leave grep-findable residue ("terminal",
  "keyring", ".env.local", "/proc").
- **Learning across tasks** and **self-reported call counts,** with shared reads attributed
  inconsistently; 4 hook-blocked calls per arm add noise to the call metric.
- **The none class is vocabulary-checkable:** "screen reader", "advisory" and "favourite" have small
  synonym sets, so it does not stress what semantic search adds.

### 5.10 Reading the results under D1

The measurements answer a smaller question than D1 asks.

- **The trial measured one project's backlog, which fits one read.** Arm A read `HANDOFF.md` once and
  reused it (§5.7). A workspace knowledge base, several projects plus notes, transcripts and later memory,
  does not fit one read. Transcripts are not greppable for an agent: they are Postgres rows, the MCP
  server has no read tool (§2.3), and no read tool will page through every run's events.
- **So the single-search numbers are the relevant ones at workspace scale.** When nobody can read
  everything, what one search returns is what an agent or a person sees. On duplicates that is 22 of 24 in
  the top 5 with htui's hybrid search, against 9 of 24 for the best baseline without Qdrant (§5.2). On
  questions the gain is not significant (10 against 7 of 20, p = 0.58). D1's own example, "have we
  decided anything about retry limits?", is a question: the class with no significant gain, where the
  dense leg alone did best (12 of 20).
- **The extrapolation is unmeasured.** No corpus spanning several projects, notes or transcripts was
  built, and no trial ran on one. Whether the duplicate gain holds, grows or shrinks there, and whether
  questions start to separate, is not known. P5 measures it.

## 6. Analysis

### 6.1 Benefit per consumer at workspace scale

The benchmark measures one query, one ranked list: what a single search returns. The trial measures an
agent that may search, read and retry. At one project's size the agent reads everything, and search adds
nothing the trial could see (§5.7). Under D1 the corpus is a workspace: several projects, their notes and
transcripts, later memory. Nobody reads that whole, so the benefit moves to what one search returns, which
is where the measured gain is (§5.10).

| Consumer | Today | Under D1, Qdrant up | Qdrant down or unconfigured (D4) | Evidence |
|---|---|---|---|---|
| Agents finding knowledge | `search_concepts`, own project only, no read tool, no prompt mentions it | `search_concepts` at workspace scope over D3's corpus | unconfigured: the tool is not offered; down: "knowledge search unavailable"; either way MOD-112's read tools with a text filter (Q10) | duplicates 22 vs 9 of 24 (single search); questions not significant; trial: none at one project's size; workspace scale unmeasured |
| Duplicate check at draft time | no code | display-only related items, across the workspace's projects | the list says knowledge search is unavailable; filing unaffected | 22 of 24 in the top 5; no score works as an automatic flag (§5.4) |
| People: `Ctrl+F`, CLI | workspace or one project; items, documents, requirements | the same over D3's corpus | "knowledge search unavailable" | as the benchmark |
| Memory (ANA-38) | not built | indexed and searched with the rest | still reaches a prompt, by injection under `R-PRM-3`'s budget | published evidence favours injection below ~100-200k tokens (§4.2) |
| Triage | no code; automatic triage ruled out (ANA-30 X5) | same retrieval as duplicates | nothing | not measured separately |
| Coordinator, prompt assembly | no search | unchanged; no search step (`R-PRM-1`) | unaffected | — |

- **Agents.** D1's case is the cross-project one: an agent in project B looking for project A's
  decision. Today it fails on scope before meaning (§2.3). At workspace scope, with notes and transcripts
  added, an agent can no longer read everything, so it depends on what a search returns. The duplicate
  gain applies there. The question class, which D1's example belongs to, showed no significant gain, and
  the dense leg alone beat the shipped fusion on it (12 against 10 of 20, 3 to 1, p ≈ 0.63). The fusion
  may lose question hits; the data cannot tell.
- **Duplicates.** Still the one clear, measured gain: 13 more of 24 in the top 5 than the best baseline
  without Qdrant, and 6 more than htui's BM25 leg alone (§5.2). Across projects, two people may file the
  same work without ever reading each other's backlog, which is where reading everything stops working.
  That is reasoning: htui has one project, and no cross-project duplicate was counted.
- **Memory.** Indexing memory in the shared Qdrant makes it searchable with the rest of the workspace.
  Reaching a prompt is a separate path. At the sizes in view, injection works and beats retrieval on
  accuracy (§4.2), so memory must not depend on Qdrant to reach a prompt.

### 6.2 Cost of a required shared Qdrant

D2 and D4 change the cost the first draft weighed. The daemon is one per workspace, not one per box, and
"required" means "expected to be configured", not "refuse to start".

- **One install and upgrade path per workspace, like Postgres.** Someone runs one Qdrant, secures it and
  upgrades it. Qdrant binds 0.0.0.0 without authentication by default, guarantees upgrades only between
  consecutive minor versions, never downgrades and has no memory cap
  (https://qdrant.tech/documentation/faq/qdrant-fundamentals/ ;
  https://qdrant.tech/articles/memory-consumption/ ; https://computingforgeeks.com/install-qdrant-ubuntu/).
  Its memory at htui's size is small: 67 MB empty, 83.7 MB with the benchmark collection (§5.5). The dev
  image is pinned to the client's version (`compose.yaml:53-62`), so a server upgrade moves with htui's.
- **Per searching box: the model.** Each box that searches downloads the 133 MB model on first use and
  holds about 150-180 MB of RSS while it embeds (§5.5). That is every TUI that searches and every worker
  that hosts sessions offered `search_concepts`. D5 keeps it there: no central search endpoint.
- **The indexer's box** carries the embedding load of the whole workspace: 120.7 ms per point at 4
  threads, so the 1,081-point benchmark corpus took 130.5 s to embed (§5.5). Transcripts will multiply
  the point count; by how much is not measured.
- **Windows, assumed a client.** Assumed, not decided: the shared Qdrant runs on Linux or under Docker,
  so a Windows box only searches. D2 does not say where the Qdrant runs. A Windows-only workspace
  (`R-NF-1`) would use Docker Desktop or Qdrant's native Windows zip (§2.5); running that zip as a service
  is **UNVERIFIED** (§11). A Windows TUI that searches still embeds on `rten`, which is unmeasured there
  (MOD-16); if it fails, search fails open on that box.
- **Headless workers need a URL and a key source.** Today the URL is only in the keyring, which a
  headless worker lacks (§2.5). With the URL in Postgres (§7.5) a worker reads it from the database it
  already reaches; the API key still needs a non-keyring source, as the DSN has.
- **Failing open is cheap only if it is built cheap.** Today a TUI search against a dead Qdrant can
  download 133 MB first, a TUI search creates the collection, and the TUI advertises `search_concepts` on
  boxes where every call errors (§2.4). D4 needs these fixed.
- **Operational record.** The live tests time out under load (`docs/decisions/mod/mod-90.md:78-79`); a
  fresh install could not search for six days (§4.1); an embedder or collection change means a manual
  delete and reindex. With one indexer per workspace, a reindex is one process's job, not every box's.

### 6.3 Options considered

| Option | What it means | Status |
|---|---|---|
| A. Required on every box, refusing to start | every TUI, worker and CLI needs a reachable Qdrant and the model, or does not start | **rejected by D4**: an outage of a derived index would stop work that does not need it |
| B. Required shared service, failing open | each workspace is expected to configure one shared Qdrant; Settings warns when it has none; when it is missing or down, only knowledge search is affected, and it says "knowledge search unavailable" | **chosen** (D2, D4) |
| C. Optional everywhere | Qdrant stays an optional extra per box, with fixes to the optional path | the first draft's verdict; **superseded** by the maintainer's direction |

B keeps what C had right, that nothing but search depends on Qdrant, and adds what D1 needs: each
workspace has one place where its knowledge is indexed, by one process, reachable from every box. The
"very beneficial" bar is met by the maintainer's decision on the workspace-knowledge framing, and the
duplicate result supports it. It is not proven for agents at today's size: the trial tied, and the gain at
workspace scale is unmeasured (§5.10).

## 7. Verdict

**7.1 Knowledge search is a required workspace service: one shared Qdrant per workspace, required to be
configured, failing open on outage (D2, D4).** Each workspace is expected to name one Qdrant, as it has one
Postgres. Every box and process of a workspace uses that workspace's one Qdrant; none runs or names a Qdrant
of its own, except a development or CI override (Q1, `R-STO-1`). When the workspace has none configured, or
its Qdrant is unreachable, htui starts and works: knowledge search in the TUI and the CLI says "knowledge
search unavailable"; for agents `search_concepts` is not offered while the workspace has no Qdrant URL, and
answers "knowledge search unavailable" during an outage (Q6). No run, prompt or close-out waits on it. The
embedding model is needed wherever a process searches or indexes, and its failure fails open the same way.
Postgres stays the source of truth; the index is derived and rebuildable.

**7.2 Scope of knowledge (D3).** Indexed: items; requirements, withdrawn ones included; decisions (items
closed `done`, `concluded` or `rejected`, with their documents); documents owned by an item, a project or
the workspace; notes; scrubbed run transcripts; workspace-owned items; and ANA-38's memory once it exists.
Not indexed: code, and markdown files in a repo checkout, which belong to the code-context tools
(`R-LATER-7`) until MOD-8 imports them into Postgres. Of these, items, requirements, decisions and item
documents are indexed today; notes, transcripts and, once ANA-35 lands, workspace-owned items are MOD-112
(4); documents owned by a project or the workspace and workspace requirements wait on ANA-35 and are P2's;
memory waits on ANA-38 (§2.1). From transcripts, the prompt, follow-up, assistant-text and plan events are
indexed; tool calls, tool results and edit proposals, which carry code, are not (D3;
`chk_event_kind`, `0001_init.sql:524-527`). MOD-112 (4) takes this as a constraint.

**7.3 One indexer per workspace, kept fresh, with workspace scope.**

- **One indexer (D5).** Whichever htui process holds the workspace's indexer lease in Postgres runs the
  index cycle; no other process indexes. The lease is a row with holder, box and expiry, taken only when
  free or expired and renewed on each beat, on the pattern of the run lease (`lease_box_id`,
  `lease_owner`, `lease_expires_at`, `migrations/0003_orchestration.sql:41-43`). Worker or TUI may hold it,
  one lease per workspace it serves (Q5). This removes the two-indexer race of §2.1, provided a holder
  renews its lease immediately before each delete batch and confirms each orphan's absence in Postgres in
  the same pass (Q4).
- **One collection per workspace (Q3).** Each workspace's collection holds every member project. A
  project in two workspaces (§2.5) is indexed in each; the index is derived, so this costs space, not
  consistency. A run records no workspace today: `run` has no `workspace_id`, and only
  `workspace_project` and `workspace_box_path` carry one (`migrations/0001_init.sql:162, 174`). So "the
  session's workspace" needs a rule (Q3).
- **Freshness (Q4).** The holder indexes rows written since its last pass on a short cycle, and runs
  today's full reconcile on a longer one to catch deletions. The TUI's `Ctrl+R` and `--index-items` take
  the lease only if it is free; otherwise they ask the holder for a pass and show which box holds the
  lease (Q5).
- **Workspace scope for people and agents.** People already search a workspace as a list of its projects
  (§2.3). Agents get the same: `search_concepts` searches the session's workspace (Q3) by default, with
  its project as a narrower option. `R-MCP-1`'s read scope beyond the run covers workspace items,
  documents and requirements only (`docs/REQUIREMENTS.md:511-513`); snippets of other runs' transcripts,
  and arguably notes, fall outside it, so the scope is widened by Q11 or agent searches exclude those point
  types. The agent tool (`search_concepts`, MOD-11) is project-only today; widening it is MOD-112 (4).

**7.4 Per-consumer paths.**

- **Agents:** `search_concepts` at workspace scope over D3's corpus, advertised when the workspace has a
  Qdrant configured (Q6). During an outage a call answers "knowledge search unavailable", and the agent
  falls back to MOD-112 (1)'s read tools with a text filter over keys, titles and bodies, which works
  without Qdrant (Q10).
- **Duplicate check and triage:** a display-only "related items" list at draft time, the top 5 across
  the workspace, open and closed items, decisions included (Q7). During an outage the list says knowledge
  search is unavailable; filing is never blocked. No link, resolution or flag is written from a score
  (`R-ID-6`, §5.4).
- **Memory (ANA-38):** entries are indexed in the shared Qdrant and searched with the rest. Memory still
  reaches a prompt without Qdrant, by injection under `R-PRM-3`'s budget, so an outage never empties a
  prompt (Q8). Suggesting near-duplicate entries on write is a display-only use of the same search.
- **People:** `Ctrl+F` over D3's corpus, the current workspace by default as today. `--search-items`
  keeps its scopes, `--cwd`'s project, `--project`'s, or by default every workspace's (§2.1). A search
  over several workspaces queries each workspace's collection in that workspace's Qdrant and names the
  workspaces whose search is unavailable (P2).
- **Coordinator and prompt assembly:** unchanged; no search step (`R-PRM-1`).

**7.5 Configuration.**

- **The Qdrant URL is a workspace setting stored in Postgres (Q1).** Every box of the workspace reads it
  from the database it already uses, so a headless worker needs no extra URL source. It holds no secret: a
  URL that carries credentials is refused. While Postgres is unreachable (`R-STO-4`) the URL cannot be
  read, and knowledge search says it is unavailable.
- **The API key is a secret delivered like the DSN (Q2):** the OS keyring; for a headless worker a
  systemd credential or stdin; for development and CI an environment variable that htui removes from every
  agent subprocess environment (`R-SEC-2`). Never Postgres, `argv` or a plaintext file.
- **Settings shows the state (Q6).** The existing Settings > Qdrant section (§2.5) keeps its masked
  API-key row in this box's keyring; P1 moves its URL row to the workspace setting. Settings and first run
  show when the workspace has no Qdrant configured, when it is unreachable, and when this box lacks its
  key: a warning, not a block. A worker logs the same warning once at start.

**7.6 Phasing.**

| Phase | Content | Depends on |
|---|---|---|
| 0 | Requirement edits (§10), the `R-NF-2` text merged with MOD-130's pending amendment; MOD-112, ANA-35 and ANA-38 take their constraints | maintainer acceptance |
| 1 | P1 (MOD-158) workspace Qdrant setting and API key; P3 (MOD-160) cheap fail-open and wording | none |
| 2 | P2 (MOD-159) single indexer, freshness, one collection per workspace; MOD-112 (4) notes, transcripts and agents' workspace scope on top of it | P1 |
| 3 | P2's (MOD-159) coverage of documents owned by a project or the workspace, workspace requirements and memory; MOD-112 (4)'s workspace-owned items | ANA-35, ANA-38 |
| 4 | P4 (MOD-161) related items at draft time | P2; MOD-112 (3), MOD-135 and MOD-8 for the later parts |
| any | P5 (MOD-162) benchmark in the repo; rerun at workspace scale once notes and transcripts are indexed | none; the rerun needs MOD-112 (4) and MOD-8 |

## 8. Questions for the maintainer

D1-D5 (§1) are decided and not asked again. What stays open:

1. **Where the URL lives (7.5).** The Qdrant URL becomes a workspace setting in Postgres, read by every
   box of the workspace. `workspace` has no settings column today (§2.5), so this needs a migration. A URL
   that carries credentials is refused. The per-box keyring entry `qdrant-url` (`secret.rs:43`) is
   retired: a box that finds it set while the workspace has none offers it once as the workspace setting,
   and the existing Settings > Qdrant section (§2.5) moves its URL row to the workspace setting. For
   development and CI an environment variable may override the URL per process, so an hr sandbox can
   point at its own Qdrant. The migration coordinates with ANA-35, which owns the workspace schema, and
   with MOD-130's `target_id` on the workspace. **Recommended: accept.**
2. **How the API key is delivered (7.5).** As a secret like the DSN, under `R-STO-1`: in the OS keyring, one
   entry per workspace; for a headless worker (`R-ORCH-12`) a systemd credential, or stdin read beside
   `--dsn-stdin` and held only in memory; for development and CI an environment variable removed from every
   agent subprocess (`R-SEC-2`). A worker may serve several workspaces, so stdin or the credential carries one
   key per workspace, keyed by workspace slug (`workspace.slug`, `0001_init.sql:131`). Not in Postgres:
   although Qdrant holds only data derived from Postgres, a key there would let any database reader rewrite or
   delete the index, and it would be the only htui credential kept in the database. A workspace whose Qdrant
   needs no key is allowed, and Settings warns about it. **Recommended: accept, with the `R-STO-1` edit of
   §10.5.**
3. **One collection per workspace (7.3).** Each workspace gets its own collection in its Qdrant, holding
   every member project; a project in two workspaces is indexed in each; two workspaces that name the
   same Qdrant still keep separate collections and leases. Today's single `htui_concepts_v2` for every
   project (§2.1) is replaced, which is a new collection, not a migration (`vector.rs:34-39`). A session
   searches the workspace its run was queued from, which P2 records on the run (`run` has no
   `workspace_id` today, §7.3); a run with none recorded searches every workspace of its project.
   **Recommended: accept.**
4. **Lease mechanism and freshness trigger (7.3).** The lease: a row per workspace (holder, box, expiry),
   taken when free or expired and renewed each beat, on the run lease's pattern, reusing `lease_ttl_seconds`
   (120) and `lease_refresh_seconds` (60) (`migrations/0003_orchestration.sql:135-136`). The holder renews the
   lease immediately before each delete batch, and a renewal that touches zero rows means abandon, as with the
   run lease (`0003_orchestration.sql:56-58`); in the same pass it confirms each orphan's absence in Postgres.
   So a lapsed holder cannot delete what the next one indexed. Not a session advisory lock: that binds the
   lease to one connection, which `PgStore`'s pool does not hold still (`pg/mod.rs:103`); htui's only advisory
   lock is transaction-scoped (`pg/write.rs:7361`). The trigger: a short cycle, 60 s recommended. Items,
   requirements, notes and documents are read by `updated_at` or `created_at` (`item_note` and `document` are
   append-only and have only `created_at`, `migrations/0001_init.sql:564-571`), pulled back by the cache
   refresh's overlap window (`cache_overlap_seconds`, 300 s, `cache/refresh.rs:220, 338`): a timestamp is
   taken before its transaction commits, so a cursor set at the last pass would miss a late commit. Reading a
   row twice is harmless; the sync is idempotent (§2.1). Transcripts are read by `(run_step_id, seq)` past the
   last indexed `seq` per step, as the cache refresh reads them (`refresh_transcripts`,
   `cache/refresh.rs:1687`), never by `session_event.at`, which is the executing box's clock
   (`0001_init.sql:522`). Today's full reconcile runs every `concepts_sync_minutes` for deletions. Not NOTIFY
   or an outbox for now: htui has neither in its code (MOD-46 proposes `NOTIFY` for live transcripts; revisit
   with it), a missed notification would still need the cycle, and a minute's lag is acceptable for a search.
   **Recommended: short cycle; revisit NOTIFY with MOD-46 or if the lag is reported.**
5. **Who may hold the lease (7.3).** Any htui process of the workspace, worker or TUI, that can reach
   its Qdrant and load the model; first come, released on a clean exit, expired otherwise. A process may
   hold one lease per workspace it serves. A worker-only rule would leave a workspace whose TUI is the
   executor, the default (§2.1), with no indexer. The TUI's `Ctrl+R` and `--index-items` take the lease
   only if it is free; otherwise they record a request for a pass on the lease row, which the holder
   serves on its next beat, and show which box holds the lease. **Recommended: worker or TUI, no
   preference rule for now.**
6. **What "configured" warns about, and where (7.4, 7.5).** Three states: the workspace has no Qdrant
   URL; it has one that is unreachable or refuses the collection (embedder mismatch); this box has no API
   key for a Qdrant that needs one. Shown as a warning in the existing Settings > Qdrant section (§2.5),
   beside Postgres (`R-TUI-8`), at first run when a workspace is created or opened, and as one log line at
   worker start; never a modal, never a block. The section keeps this box's masked API key in the
   keyring, and its URL row now edits the workspace setting in Postgres. That row could instead sit in the
   Workspaces tab's overview (`R-TUI-14`), since the URL belongs to the workspace; Settings is
   recommended, beside the key. Search surfaces say "knowledge search unavailable" with the cause one
   key away. `search_concepts` is advertised to a session when the workspace has a URL configured,
   checked as the session starts; during an outage a call answers "knowledge search unavailable" as a
   tool error. **Recommended: accept.**
7. **Related items at draft time, display-only (7.4, P4):** the top 5 nearest items across the
   workspace, open and closed; no automatic duplicate flag, because no score separated duplicates from
   no-target queries (§5.4). **Recommended: accept.**
8. **Memory and Qdrant (7.4):** ANA-38 indexes memory in the shared Qdrant and gets memory into a prompt
   without it. **Recommended: accept, as a constraint on ANA-38.**
9. **Requirement edits (§10).** Each edit stands or falls with its question: the bracketed sentences of
   §10.2 with Q3, Q4, Q5 and Q7, §10.3's text-filter clause with Q10, §10.4 with Q8, §10.5 with Q1 and
   Q2, §10.6 with Q6, §10.7 with Q11. **Recommended: accept as written, the wording open to the
   maintainer's edits.**
10. **Agents' outage fallback (7.4).** MOD-112 (1)'s read tools gain a text filter over keys, titles and
    bodies that works without Qdrant, so an agent can still find by words while knowledge search is
    unavailable or unconfigured. MOD-112 names no lexical tool today (§3, item 1). It is likely weaker than
    the trial's grep and whole-file read (§5.9). **Recommended: accept.**
11. **Agents' read scope for search results (7.3).** `R-MCP-1` lets an agent read workspace items,
    documents and requirements beyond its run. Knowledge search at workspace scope also returns snippets
    of notes and of other runs' scrubbed transcripts. Either `R-MCP-1` widens to cover them (§10.7), or
    agent searches exclude notes and transcripts and only people search them. Transcripts are scrubbed
    before a row persists (`R-SEC-3`), and a hit carries at most 280 characters (§2.3). **Recommended:
    widen, with the edit of §10.7.**

## 9. Proposed spawns

Filed 2026-10-19 as P1 → MOD-158, P2 → MOD-159, P3 → MOD-160, P4 → MOD-161, P5 → MOD-162; ordering is by
dependency.

| P | Filed | Subject | Scope | Blocked on | Requirements |
|---|---|---|---|---|---|
| P1 | MOD-158 | Workspace Qdrant: URL setting and API key | The Qdrant URL as a workspace setting in Postgres (migration, coordinated with ANA-35's workspace schema and MOD-130's `target_id` on the workspace; set from the existing Settings > Qdrant section, whose URL row moves to it, and from the CLI), a URL with credentials refused; the API key in the keyring per workspace, from a systemd credential or stdin, one key per workspace slug, for a headless worker (R-ORCH-12), and for development and CI from an environment variable htui removes from every agent subprocess (R-SEC-2); a development and CI override for the URL; the per-box keyring URL offered once as the workspace setting, then retired; a setting changed after start picked up without a restart. | none | R-STO-1, R-STO-8, R-SEC-2, R-TUI-8, R-NF-2 |
| P2 | MOD-159 | One indexer per workspace; freshness; coverage | The workspace indexer lease in Postgres (holder, box, expiry, renewal, one lease per workspace a process serves; renewed before each delete batch, orphans confirmed absent in the same pass); one collection per workspace; the run records the workspace it was queued from; the short incremental cycle (timestamps with the cache refresh's overlap; transcripts by step and `seq`) plus the full reconcile; the worker's index job runs under the lease, and the TUI's `Ctrl+R` and `--index-items` take it only when free, otherwise request a pass from the holder and show which box holds it; `--search-items` over several workspaces queries each one's Qdrant and names those unavailable. Coverage of the parts of D3 that MOD-112 (4) does not own: documents owned by a project or the workspace and workspace requirements once ANA-35 lands, and memory as ANA-38 designs it. **MOD-112 (4) keeps** notes, scrubbed transcripts (prompt, follow-up, assistant-text and plan events only), workspace-owned items once ANA-35 lands, and agents' workspace scope; P2 provides the indexer they plug into and does not duplicate them. | P1; ANA-35 and ANA-38 for those parts | R-STO-8, R-ENT-12, R-ENT-13, R-ENT-16 |
| P3 | MOD-160 | Fail open cheaply, and say so | The TUI search checks Qdrant and the collection before loading the model and never creates a collection, as the CLI already does; every search surface answers "knowledge search unavailable" with its cause; `search_concepts` advertised when the workspace has a URL configured, answering with the same words during an outage (today it is always advertised, §2.4); Settings, first run and worker start warn when the workspace has no Qdrant, when it is unreachable, and when this box lacks the key; CI runs `qdrant_live` and `qdrant_worker` against a Qdrant instead of skipping. May be folded into P1. | none | R-NF-2, R-NF-3, R-STO-8, R-MCP-2, R-TUI-8 |
| P4 | MOD-161 | Related items at draft time | The top 5 nearest items across the workspace (open and closed, decisions included) beside item creation in the TUI, `item_create` drafts, the coordinator's follow-up drafts and the MOD-8 importer, for a person to judge; display-only, nothing written from a score; "knowledge search unavailable" during an outage, filing unaffected. | P2; MOD-112 (3), MOD-135 and MOD-8 for those parts | R-STO-8, R-ID-6, R-ENT-9, R-MCP-2 |
| P5 | MOD-162 | Retrieval benchmark in the repo; rerun at workspace scale | Move the ANA-37 harness (corpus builder, frozen queries with their hash, metrics, McNemar) from `/tmp` into the repo as a dev tool against a scratch Qdrant; build a workspace corpus from Postgres (several projects, notes, transcripts) once MOD-112 (4) and MOD-8 land; add cross-project duplicate and question queries; report dense-only beside the hybrid; rerun the agent trial on a corpus that does not fit one read. Could be filed as TOOL instead. | none; the workspace rerun needs MOD-112 (4) and MOD-8 | R-STO-8 |

Effects on existing items, applied at acceptance: **MOD-112** part (2) is recorded as existing
(`search_concepts`); part (4) keeps notes, transcripts (without tool calls, tool results or edit
proposals), workspace-owned items and agents' workspace scope, plugs into P2's indexer and becomes blocked
on P2; part (1)'s read tools carry the text filter that is agents' fallback during an outage (Q10).
**ANA-35** gains a note that documents owned by a project or the workspace and workspace requirements are
knowledge P2 indexes once they exist, and that P1's `workspace` migration coordinates with its workspace
schema. **MOD-130** gains a note that P1's migration and its `target_id` on the workspace touch the same
table, and that its `R-NF-2` amendment merges with §10.1. **ANA-38** gains "memory is indexed in the
shared Qdrant and reaches a prompt without it" as a constraint; how memory is indexed is ANA-38's design.
**MOD-135** gains P4 as a later consumer of its follow-up drafts.

## 10. Requirement edits owed

All maintainer-gated; `docs/REQUIREMENTS.md` is not edited by this item. `<date>` is the acceptance date.
Accepted 2026-10-19 and applied to `docs/REQUIREMENTS.md` in the conclusion commit (`docs/decisions/ana/ana-37.md`).
A bracketed condition marks a sentence that depends on a §8 question other than Q9; it is removed on
acceptance, and the sentence is dropped if its question is rejected.

**10.1 `R-NF-2`, proposed text:**

> **R-NF-2 (must).** Required services: Postgres, and one shared Qdrant per workspace with its embedding
> model, which runs in each process that searches or indexes (R-STO-8). Qdrant fails open: when a workspace's
> Qdrant is not configured or not reachable, `htui` starts and works, knowledge search says "knowledge search
> unavailable", and nothing else is affected; no run, prompt or close-out waits on it. While a workspace names
> no Qdrant, agents are not offered the search tool (R-MCP-2). Every box and process of a workspace uses that
> workspace's one Qdrant; none runs or names a Qdrant of its own, except a development or CI override
> (R-STO-1). Beyond Postgres, Qdrant and the agents, no required dependency on any external daemon, except the
> code-context tools in coding sessions (R-LATER-7). An error-reporting endpoint the user configures in
> Settings stays optional: it is off unless set, receives only scrubbed events (R-ID-7, MOD-99), and fails
> open with nothing else affected. Amended by maintainer decision, 2026-10-14, 2026-10-15 (context tools;
> Qdrant to ANA-37), and on ANA-37, `<date>` (one shared Qdrant per workspace, required, failing open).

Composes with MOD-130's pending amendment (`dockerd` and `sshd` opt-in per target, `HANDOFF.md:395`)
and with MOD-44's open `dockerd` question (`HANDOFF.md:347`); whichever lands second merges the other's
sentence instead of replacing the whole text.

**10.2 `R-STO-8`, proposed text:**

> **R-STO-8 (must).** Knowledge search for the workspace. `htui` indexes what a workspace knows, across its
> projects: each item's key, title and body, its latest documents, its notes and the scrubbed transcripts of
> its runs (prompts, agent text and plans, not tool calls, tool output or edits); documents owned by a project
> or the workspace (R-ENT-12); workspace-owned items (R-ENT-16); each requirement's key, body and rationale
> (withdrawn ones included); and memory (R-ENT-13). Code and markdown files in a repo checkout are not
> indexed: they belong to the code-context tools (R-LATER-7) unless imported into Postgres (R-LATER-3). The
> index lives in the workspace's shared Qdrant (R-NF-2), with local dense embeddings and BM25 sparse vectors
> ranked together. [Only if Q3 is accepted:] Each workspace has its own collection, holding every member
> project. Searches cover the workspace by default and can be narrowed to projects; agents search through an
> MCP tool at workspace scope, within their read scope (R-MCP-1, R-MCP-2). A search can be narrowed to
> decisions: items closed as `done`, `concluded` or `rejected` (R-ENT-8), and their documents. The index is
> derived from Postgres and rebuildable from it (R-STO-1). One `htui` process per workspace indexes it and
> keeps it current. [Only if Q4 and Q5 are accepted:] It is the holder of a workspace lease in Postgres,
> worker or TUI. [Only if Q4 is accepted:] Rows written are indexed within a short cycle, and a full reconcile
> catches deletions. When the workspace's Qdrant or the embedding model is unavailable, search answers
> "knowledge search unavailable" and nothing else is affected (R-NF-2). [Only if Q7 is accepted:] Drafting an
> item shows the nearest existing items across the workspace for a person to judge; nothing is written from a
> search score (R-ID-6). Added by maintainer decision on MOD-34, 2026-09-25 (`docs/ANA-19.md`,
> `docs/ANA-20.md`); amended by maintainer decision on MOD-50, 2026-09-29 (requirements indexed, decisions by
> resolution), on the requirements review, 2026-10-15 (notes, transcripts, workspace scope, agent tool), and
> on ANA-37, `<date>` (workspace knowledge search, shared Qdrant, one indexer; and, with Q3, Q4, Q5 and Q7, a
> collection per workspace, the lease, freshness and related items).

**10.3 `R-MCP-2`** (the text-filter clause with Q10; if Q10 is rejected, the clause is dropped), replace "read
tools for items, documents and requirements, and semantic search (R-STO-8)" with "read tools for items,
documents and requirements, with a text filter over their keys, titles and bodies that works without Qdrant,
and knowledge search at workspace scope (R-STO-8), which says it is unavailable when the workspace's Qdrant is
not available", and replace the final "Amended 2026-10-15." with "Amended 2026-10-15 and `<date>` (ANA-37)."

**10.4 `R-ENT-13`** (with Q8), append: "Memory entries are indexed in the workspace's knowledge search
(R-STO-8), and memory reaches a prompt without it, so an outage of the workspace's Qdrant never empties a
prompt (R-NF-2, ANA-37)."

**10.5 `R-STO-1`** (with Q1 and Q2), three exact edits to `docs/REQUIREMENTS.md:257-269`:

1. Replace "Connection string and provider identities live in the OS keyring" with "Connection string,
   provider identities and the API key of each workspace's Qdrant live in the OS keyring".
2. After "Never in `argv` or a plaintext file.", insert: "The same exception, and the development and CI
   environment variable, apply to a workspace's Qdrant API key (R-STO-8). The Qdrant URL is not a secret:
   it is a workspace setting in Postgres, carries no credentials, and may be overridden per process by a
   development or CI environment variable."
3. Append to the amendment trail, before its final full stop: ", and on ANA-37, `<date>` (Qdrant API key
   and URL)". The trail then ends "…(environment variable; key bindings moved to Postgres), and on ANA-37,
   `<date>` (Qdrant API key and URL)."

**10.6 `R-TUI-8`** (with Q6), two exact edits to `docs/REQUIREMENTS.md:588-600`:

1. Replace "the default workflow) and the component catalogue (R-ENT-17);" with "the default workflow),
   the component catalogue (R-ENT-17), and the workspace's Qdrant (its URL, stored in Postgres; a masked
   field for this box's API key, stored in the keyring; the state of the last attempt; a warning when the
   workspace has none; R-STO-8, R-NF-2);". The earlier "What is typed goes to the OS keyring and nowhere
   else" stays with the Postgres connection it follows; the Qdrant URL is not a secret (§10.5).
2. Replace the trail "Amended 2026-10-15 and 2026-10-17 (UI review,
   `docs/ui-redesign-findings-2026-10-15.md`)." with "Amended 2026-10-15, 2026-10-17 (UI review,
   `docs/ui-redesign-findings-2026-10-15.md`) and `<date>` (ANA-37)."

**10.7 `R-MCP-1`** (with Q11), replace "and may read beyond it — workspace items, documents and
requirements." with "and may read beyond it — workspace items with their notes, documents and
requirements, and, through knowledge search (R-STO-8), snippets of the scrubbed transcripts of the
workspace's runs.", and replace the final "Amended 2026-10-15 (wider read scope)." with "Amended
2026-10-15 (wider read scope) and `<date>` (ANA-37, notes and transcripts through search)."

## 11. Not verified

- **Alternative backends were deliberately not surveyed,** per the maintainer's 2026-10-18 scoping:
  pgvector, embedded indexes (Qdrant Edge included, noted only as existing in beta) and other vector
  databases. Whether a lexical index outside Qdrant could carry the share of the duplicate gain that
  htui's BM25 leg delivers (9 of 13 rescues) was therefore not examined.
- **The workspace-scale extrapolation is UNMEASURED.** §5.10 and §6.1 argue that the single-search
  numbers matter more once a workspace's knowledge does not fit one read. No corpus spanning several
  projects, notes or transcripts was built, no cross-project duplicate was counted (htui has one
  project), and no trial ran on such a corpus. P5 is owed for that.
- **Real agent use of `search_concepts`:** how often agents call it is **UNVERIFIED**; transcripts are in
  the host database, which the sandbox cannot reach.
- **htui's own no-Qdrant baseline** (substring on key and title) was not run in the benchmark; L and G are
  stronger stand-ins.
- **Collapsing, resolved.** The harness collapses chunk hits to one row per ID; htui does not.
  `QdrantStore::search` (`vector.rs:1059-1107`) returns points as Qdrant ranks them, and no collapsing was
  found in `mcp_search.rs`, `concepts.rs`, `concepts_worker.rs` or `tools/search.rs`. Recomputed from the
  uncollapsed points (`S_points`, top 5 points): duplicates 22/24, questions 9/20, identifiers 15/16. The
  headline holds; questions lose one hit.
- **The outage fallback against arm A.** §7.4 relies on MOD-112's text filter over Postgres rows during
  an outage. Whether it performs about as well as arm A's grep and whole-file read was not measured; an
  MCP filter with no bulk read is likely weaker (§5.9).
- **The real corpus shape:** items and decisions are not yet Postgres rows (MOD-8); notes, transcripts and
  `docs/ANA-*.md` were not in the corpus. Results may change on the real index.
- **The lease and freshness design** (§7.3, Q4, Q5) is a proposal checked against the schema only; no
  code for it was written or tested.
- **Trial limits:** one run per arm, call counts self-reported and not cross-checked against transcripts,
  the model and its version not recorded in this doc.
- **Resource figures:** Qdrant memory and CPU with a populated index under real query load, sync duration
  on a real backlog, and the point count transcripts add were not measured beyond the 83.7 MB figure. The
  embedder figures are ANA-23's spike, not the real binary. Qdrant's official idle RSS is **UNVERIFIED**.
- **Windows:** `rten` on Windows is unmeasured (MOD-16 open). That the shared Qdrant runs on Linux or
  under Docker, leaving Windows boxes as clients, is an assumption, not a decision (§6.2). For a
  Windows-only workspace, running the native `qdrant-x86_64-pc-windows-msvc.zip` as a Windows service is
  **UNVERIFIED**.
- **The two-indexer transient deletion** (§2.1) is inferred from code order and not tested; the lease of
  §7.3 removes it once built.
- **Web evidence marked UNVERIFIED by its reader:** the headline numbers of the "When Lexical Beats Neural"
  preprint (PDF returned 403); the 2026 Stack Overflow Recall@20 figures (search snippet only); the "vibes"
  quote attributed to the Latent Space podcast; whether embeddings supply Linear Triage Intelligence's
  candidates; the Cline/Roo memory-bank design.
- **Whether the UI redesign keeps `Ctrl+F`:** `docs/ui-redesign-findings-2026-10-15.md` does not mention
  it; **UNVERIFIED**.
- **The maintainer's bar** exists in two wordings, "proves very beneficial" and "is very beneficial";
  neither is a verbatim transcript. Under D1-D5 it is met by the maintainer's decision, not by a
  measurement (§6.3).
- **Artefacts:** the benchmark (`/tmp/ana37-benchmark/`), the trial (`/tmp/ana37-trial/`), the Qdrant
  collection `ana37_concepts` and the Postgres database `ana37` are outside the repo. An hr sandbox
  restart wipes `/tmp` and `~/.cache`; P5 exists so the numbers can be reproduced.
