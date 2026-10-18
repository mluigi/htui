# ANA-37 - Is Qdrant worth requiring

> **Scope note:** Decide whether Qdrant-backed semantic search (`R-STO-8`) gives htui enough real benefit,
> to agents, triage, duplicate detection and memory, to make the Qdrant daemon and its embedding model a
> required dependency (`R-NF-2`). Narrowed by the maintainer on 2026-10-18 (§1): a measurement of benefit,
> not a survey of alternative backends. Governed by `.claude/rules/workflow-docs.md`, `docs/REQUIREMENTS.md`
> and `docs/requirements-review-2026-10-15.md` (the item's origin).
>
> **Requirements addressed:** `R-NF-2`, `R-STO-8`, `R-ENT-13`. Touched at their seams: `R-MCP-2` (agent
> read and search tools), `R-STO-1` (where a headless worker finds its Qdrant URL), `R-ID-6` (drafts a
> person accepts), `R-NF-1` (Windows), `R-PRM-1`/`R-PRM-3` (how context reaches a prompt).
>
> **Status (2026-10-18): draft; §8 awaits the maintainer.** No implementation code is written here.
> Verdict (§7): **Qdrant stays optional, the daemon and the embedding model alike, on every box.** On
> htui's own corpus the shipped hybrid search puts a reworded duplicate in the top 5 for 22 of 24 queries.
> Without Qdrant, the best baseline (Postgres full-text, length-normalised) gets 9 of 24 (paired
> p = 0.0002). htui's own BM25 leg, which is lexical but runs in Qdrant, gets 16 of 24; the dense leg adds
> the remaining 6 (p = 0.031). The gain does not show for natural-language questions (10 of 20 against 7
> of 20, p = 0.58) or for identifiers (13-15 of 16 for every backend), and in an agent A/B trial an agent
> with grep and read matched an agent with semantic search on all 12 tasks (n = 12, one run per arm, both
> at the ceiling). The one consumer that clearly gains, a duplicate check at draft time, is unbuilt and
> display-only; without Qdrant it would be absent and filing unaffected. In htui's own history at least
> about 1% of items (2 of ~200) had a noticed overlap that needed recall across different wording;
> overlaps nobody noticed are not counted. Requiring Qdrant would make a reachable Qdrant (one can be
> shared) and a 133 MB model per searching box a hard dependency; headless workers would first need a
> non-keyring URL source (P1). Proposed spawns P1..P4 (§9), not filed.

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
  (`MAX_CHUNK_CHARS`, `:27`; `document_points`, `:237`); and each requirement as `"{key} {body}\n\n{rationale}"`,
  withdrawn ones included (`requirement_point`, `:215`). Payload carries type, key, project, status,
  resolution and a snippet of at most 280 characters.
- **Not indexed, although `R-STO-8` requires it:** notes, scrubbed run transcripts, workspace scope and
  workspace-owned items. These are MOD-112 part (4) (`HANDOFF.md:689-696`). Comments, links, tags and labels
  are not indexed either.
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
`permission_prompt` (`ALL`, `crates/htui-mcp/src/tools/mod.rs:45-54`); read tools are MOD-112 (1). No prompt tells
an agent the tool exists; the only signal is its description, "Searches this project's items, documents
and requirements by meaning."

Without Qdrant, the lexical baseline htui has today is narrow: items match by case-insensitive substring
on **key and title only** (`crates/htui-store/src/pg/read.rs:105-110`), requirements on key and body
(`pg/read.rs:948`), documents, notes and transcripts not at all. There is no Postgres full-text search in
the code.

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
- **The URL lives only in the OS keyring** (`secret.rs:43-45, 304-314`). There is no env, stdin, file or
  systemd-credential source. The DSN also has `--dsn-stdin`, a systemd credential and, for development
  and CI, an environment variable (`docs/REQUIREMENTS.md:257-269`; `docs/htui-worker.md:380-399`); it
  never has a file source. So a
  systemd or provisioned worker **never** indexes and never offers `search_concepts`
  (`docs/htui-worker.md:240-241, 415-417`), and an hr sandbox, which has no keyring, searches only in
  tests. Requiring Qdrant on a headless worker first needs code that does not exist.
- **Dev and test.** `compose.yaml:53-62` runs `qdrant/qdrant:v1.19.1` pinned to the client, and
  `README.md:129` calls it "only needed for Search". `scripts/hr` gives each run its own Qdrant
  (`docker/hr/compose.hr.yaml:35-60`). `qdrant_live` and `qdrant_worker` print a skip line and pass when
  `HTUI_TEST_QDRANT_URL` is unset, so CI without Qdrant passes them vacuously.
- **Windows.** Qdrant v1.19.1 ships `qdrant-x86_64-pc-windows-msvc.zip`, about 29.7 MB
  (https://api.github.com/repos/qdrant/qdrant/releases/tags/v1.19.1). htui documents only
  `docker compose up -d` (`README.md:120-129`), which on Windows means Docker Desktop. `rten` was measured
  on Linux x86-64 only (ANA-23 §9); MOD-16 is open. The headless worker is Linux-only.

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
  what a lexical, read-capable tool gives, not what htui's agents have now. Q2 assumes MOD-112's text
  filter performs about as well as arm A's grep and whole-file read. An MCP filter over rows, with no
  bulk read, is likely weaker, and arm B also had grep and read, which htui's agents lack; so the
  trial's tie may overstate how little semantic search adds for an htui agent.
- **The task author knew the targets;** paraphrases may leave grep-findable residue ("terminal",
  "keyring", ".env.local", "/proc").
- **Learning across tasks** and **self-reported call counts,** with shared reads attributed
  inconsistently; 4 hook-blocked calls per arm add noise to the call metric.
- **The none class is vocabulary-checkable:** "screen reader", "advisory" and "favourite" have small
  synonym sets, so it does not stress what semantic search adds.

## 6. Analysis

### 6.1 Benefit per consumer

The benchmark and the trial answer different questions, and both are needed. The benchmark measures one
query, one ranked list: what a single search returns. The trial measures an agent that may search, read
and retry. On duplicates the single search gains a lot (22 of 24, against 9 without Qdrant and 16 for
htui's BM25 leg alone). The agent with grep and read
gains nothing from it at this size, because it reads the whole backlog. So the benefit is largest exactly
where nobody reads everything: a person at the item form, an importer, a large backlog.

| Consumer | Today | Without Qdrant | Broken or degraded | How often | How large |
|---|---|---|---|---|---|
| Agents finding related work | `search_concepts`, own project only, no read tool, no prompt mentions it | no discovery tool at all | **broken for discovery** today, run unaffected; **degraded** once MOD-112 gives read tools with a text filter | unmeasured | trial: none (12/12 both); benchmark questions +3 of 20, not significant |
| Duplicate check at draft time | no code | the person or agent reads the backlog; key/title substring | neither today (unbuilt). Under P3 it is absent without Qdrant and filing is unaffected | at least about 1% of items (2 of ~200) had a noticed overlap; unnoticed ones are not counted | benchmark: 22 of 24 in the top 5, against 9 without Qdrant (p = 0.0002) and 16 for htui's BM25 leg alone (p = 0.031); no score works as an automatic flag |
| Triage | no code; automatic triage ruled out (ANA-30 X5) | nothing | neither; no consumer | — | same retrieval as duplicates; not measured separately |
| Memory (ANA-38) | not built | injection by scope, kind or recency under `R-PRM-3`'s budget | **degraded only past the budget** | — | published evidence favours injection below ~100-200k tokens |
| Coordinator (ANA-34) | no search | unaffected | neither | — | none |
| People: `Ctrl+F`, CLI | built | key/title substring | degraded | unmeasured | as the benchmark |

- **Agents.** The only hard break today is that an agent without Qdrant has no discovery tool. That break
  is not about meaning: it exists because the MCP server has no read or list tool at all. MOD-112's read
  tools fix it if they carry a text filter over bodies, which the trial's arm A stands in for (an
  assumption, §5.9: a row filter with no bulk read is likely weaker than grep and read). Once they
  do, what semantic search adds for an agent at htui's size is not measurable in the trial and not
  significant in the benchmark's question class. The dense leg alone beat the shipped fusion on
  questions (0.60 against 0.50, 3 to 1, p ≈ 0.63), so the fusion may lose some question hits; the data
  cannot tell.
- **Duplicates.** This is the one clear, measured gain, and it splits in two. Against the best no-Qdrant
  baseline the hybrid gains 13 queries (22 against 9 of 24, p = 0.0002). But htui's BM25 leg alone,
  which is lexical and carries the IDF the Postgres and grep baselines lack, already reaches 16 of 24,
  and it finds 9 of the 13 rescues. What meaning adds is the step from 16 to 22: the hybrid beats the
  sparse leg 6 to 0 (p = 0.031). In a03, a11 and a18 the dense leg ranks the target second, the sparse leg
  misses the top 10 and no other lexical variant reaches the top 5, and the hybrid ranks it 3rd, 4th and
  3rd. Both legs, as built, need the daemon. A second caution: the consumer is a suggestion list a person
  judges (`R-ID-6`, ANA-30 X4), and no score supports an automatic flag (§5.4). Without Qdrant the list is
  absent; the item is still filed, the duplicate is still caught later or not, and history says a noticed
  miss costs a fold.
- **Memory.** Nothing exists to measure. At the scale of the one analogue on record (12k words) memory
  fits in a prompt, and the published results favour injection there. Semantic retrieval matters for
  memory only once a scope's memory outgrows `R-PRM-3`'s budget. Dedup of entries on write is the
  duplicate case again and is display-only by `R-ID-6`.

### 6.2 Benefit against the cost of a required daemon

Requiring Qdrant means requiring two things on every box that searches or indexes: a reachable daemon and
the in-process embedder with its model.

- **The daemon** can be one shared instance, like Postgres (§2.5), so "a second daemon on every box"
  overstates the install. It still adds a service to install, secure and upgrade: Qdrant binds 0.0.0.0
  without authentication by default, guarantees upgrades only between consecutive minor versions, never
  downgrades, has no memory cap, and ships no Windows ARM64 binary
  (https://qdrant.tech/documentation/faq/qdrant-fundamentals/ ;
  https://qdrant.tech/articles/memory-consumption/ ;
  https://github.com/qdrant/qdrant/issues/7686 ; https://computingforgeeks.com/install-qdrant-ubuntu/).
  Its memory is small at htui's size: 67 MB empty, 83.7 MB with the 1,081-point benchmark collection.
- **The embedder** costs every searching box a 133 MB download on first use, 150-180 MB of RSS while it
  embeds, and, on Windows, an `rten` path no one has measured.
- **Headless workers** cannot be given a Qdrant URL at all today (§2.5). Requiring Qdrant there needs new
  code first, and until then a provisioned worker would refuse to start or violate the requirement. This
  is a one-off cost: P1 closes the gap whichever way the verdict goes.
- **Failing open would become failing closed,** if "required" means refusing to start without it. Every
  path in §2.4 that now shows an error in a box would turn into a refusal to start, and the tests that
  skip without Qdrant would have to stop skipping. A weaker reading, "required where a feature is
  enabled", is option B of §6.3.
- **Operational record.** The live tests time out under load (`docs/decisions/mod/mod-90.md:78-79`); a
  fresh install could not search for six days (§4.1); an embedder or collection change means a manual
  delete and reindex.

On the other side of the scale: one measured, significant gain, for one consumer that is unbuilt and
display-only, absent without Qdrant while filing is unaffected. Of that gain, the share only meaning
delivers is 6 of 24 duplicate queries (16 to 22, p = 0.031); the rest is htui's BM25 leg, lexical but
run in Qdrant. Add no measured gain for agents; no consumer at all in triage, the coordinator or prompts;
and memory that would not need retrieval at the sizes in view. That does not meet "very beneficial". The
benefit is real enough to keep and improve the optional path, not to make every box depend on it.

### 6.3 Options considered

| Option | What it means | Against it |
|---|---|---|
| A. Required on every box | every TUI, worker and CLI needs a reachable Qdrant and the model, and refuses to start without them | pays the whole of §6.2 for a gain the trial could not see at htui's size and one unbuilt, display-only consumer; headless workers cannot be given a URL until P1 |
| B. Required where a feature is enabled | a box that turns on a Qdrant-backed feature (say P3's related items) must have Qdrant; other boxes need not | no such feature exists yet, so today it requires nothing; it is the expected shape if a §7.4 trigger fires |
| C. Optional, plus fixes to the optional path | Qdrant stays fail-open everywhere; P1 makes it configurable on headless workers and keeps the index fresh, P2 makes failure cheap and honest, P3 builds the one consumer that gains | the duplicate gain reaches only boxes where someone configures Qdrant |

C wins. It delivers the measured benefit wherever Qdrant is set up, which one shared instance makes cheap
(§2.5), and costs nothing elsewhere. It fixes the gaps that today keep the benefit from showing even where
Qdrant is configured (§7.3). And it leaves B open for when a consumer exists whose absence hurts (§7.4).
A's cost is certain; its benefit is one display-only list.

## 7. Verdict

**7.1 Qdrant stays optional: the daemon and the embedding model alike, on every box.** No box, process or
deployment kind (TUI, worker, headless, Windows) requires Qdrant or the model. `R-NF-2` keeps it in its
fail-open list and drops the "unless ANA-37" caveat (§10.1). The embedder is useless without the daemon and
the daemon cannot be queried without the embedder (§2.2), so they are optional together; there is no
"embedder required, daemon optional" split worth making.

**7.2 Every consumer has a path that does not need Qdrant, or is hidden without it.**

- **Agents:** MOD-112's read tools carry a text filter over keys, titles, bodies, documents and
  requirements that works without Qdrant (§10.3). `search_concepts` stays, additive. Today the TUI always
  advertises it and on a box without Qdrant every call errors (§2.4); under P2 it is advertised only where
  a Qdrant URL is configured. Agent discovery then degrades without Qdrant and never breaks.
- **Duplicate check and triage:** a display-only "related items" list at draft time, top 5, open and
  closed items, decisions included; shown when Qdrant is configured and hidden when it is not. No link,
  resolution or flag is ever written from a score (`R-ID-6`, §5.4). This is P3.
- **Memory (ANA-38):** memory reaches a prompt without Qdrant, by injection under `R-PRM-3`'s budget.
  Indexing memory entries in Qdrant and suggesting near-duplicate entries on write are allowed as
  optional extras. ANA-38 takes this as a constraint (§10.4).
- **Coordinator and prompt assembly:** unchanged; no search step.

**7.3 Make the optional path work where it is configured.** The benefit measured in §5 is only real where
the index exists and is fresh. Today it is often neither: a headless worker cannot be given the URL, and a
box whose TUI is the executor never syncs (§2.1, §2.5). So: the Qdrant URL and API key get the DSN's
non-keyring sources for a headless worker, and whichever process executes runs on a box keeps the index
fresh (P1). A search against a dead Qdrant fails before the 133 MB model loads, a search never creates a
collection, and agents are offered `search_concepts` only where a Qdrant URL is configured (P2). The
recommended deployment is one shared Qdrant per team, as with Postgres.

**7.4 What would reopen the question.** Any one of these, measured with the same harness (P4):

1. The real corpus is in Postgres (MOD-8, MOD-112 (4)) and a rerun shows a significant gain for the
   question class, or an agent trial on it shows an accuracy gap between the arms.
2. ANA-38's memory for one scope outgrows `R-PRM-3`'s budget, so retrieval is needed to reach a prompt.
3. P3's related-items list is in use and its absence on boxes without Qdrant is reported as a problem.
4. A project's open backlog no longer fits one agent read, or its open items pass a size set with P4
   (today's 71 open items fit one read), and an agent trial on it separates the arms.

Even then the expected answer is "required where that feature is enabled", not "required on every box".

**7.5 Phasing.**

| Phase | Content | Depends on |
|---|---|---|
| 0 | Requirement edits (§10); MOD-112 and ANA-38 take their constraints | maintainer acceptance |
| 1 | P1 headless configuration and index freshness; P2 cheap fail-open | none |
| 2 | P3 related items at draft time: the TUI's item creation first, then `item_create` drafts, coordinator follow-ups and the importer | MOD-112 (3), MOD-135, MOD-8 for the later parts |
| any | P4 benchmark in tree, rerun when MOD-8 lands | none |

## 8. Questions for the maintainer

1. **Qdrant optional, daemon and model together, on every box (7.1).** **Recommended: accept.**
2. **Agent discovery without Qdrant (7.2):** MOD-112's read tools gain a text filter over bodies,
   documents and requirements that works without Qdrant; `search_concepts` stays additive. Without this an
   agent on a box without Qdrant has no way to find by words. **Recommended: accept.**
3. **Related items at draft time, display-only (7.2, P3):** top 5 nearest items when Qdrant is
   configured, hidden otherwise; no automatic duplicate flag, because no score separated duplicates from
   no-target queries (§5.4). **Recommended: accept.**
4. **Memory must reach a prompt without Qdrant (7.2):** ANA-38 takes this as a constraint; Qdrant indexing
   and dedup of memory entries stay optional. **Recommended: accept.**
5. **Make the optional path work (7.3, P1, P2):** a non-keyring Qdrant URL source for a headless worker,
   on the DSN's pattern; index freshness owned by the executing process; a search fails before the model
   loads and never creates a collection; agents are offered `search_concepts` only where a Qdrant URL is
   configured. **Recommended: accept, including the `R-STO-1` edit in §10.5.**
6. **Reopen triggers (7.4)** and keeping the benchmark harness in the repo to measure them (P4).
   **Recommended: accept.**
7. **Requirement edits (§10).** Each edit stands or falls with its question: the bracketed sentences of
   §10.2 with Q3 and Q5, §10.3 with Q2, §10.4 with Q4, §10.5 with Q5. **Recommended: accept as written,
   the wording open to the maintainer's edits.**

## 9. Proposed spawns

Not filed; no IDs are minted. Ordering is by dependency.

| P | Subject | Scope | Blocked on | Requirements |
|---|---|---|---|---|
| P1 | Qdrant for headless workers; index freshness | For a headless worker (R-ORCH-12), the Qdrant URL and API key read from stdin and from a systemd credential beside the keyring, as the DSN is; for development and CI, from an environment variable that htui removes from every agent subprocess environment (R-SEC-2), as `R-STO-1` allows for the DSN, so an hr sandbox, which has no keyring, can search outside tests; a URL stored after start is picked up without a restart; the process that executes runs on a box (worker or TUI) runs the index cycle; the two-indexer transient deletion of §2.1 fixed or shown harmless; README: one shared Qdrant per team. | none | R-STO-8, R-STO-1, R-SEC-2, R-NF-2 |
| P2 | Search fails open cheaply | The TUI search checks Qdrant and the collection before loading the model, and opens the existing collection instead of creating it, as the CLI does; a dead Qdrant never triggers a 133 MB download. The TUI advertises `search_concepts` to a session only when a Qdrant URL is configured, checked as the session starts, so a URL stored mid-session reaches new sessions (today it is always advertised, §2.4). `Ctrl+F` stays offered and keeps saying how to configure Qdrant. May be folded into P1. | none | R-NF-2, R-STO-8, R-NF-3, R-MCP-2 |
| P3 | Related items at draft time | When Qdrant is configured: the top 5 nearest existing items (open and closed, decisions included) beside item creation in the TUI, `item_create` drafts, the coordinator's follow-up drafts and the MOD-8 importer, for a person to judge; display-only, nothing written from a score; hidden without Qdrant. | MOD-112 (3), MOD-135 and MOD-8 for those parts; the TUI part is not blocked | R-STO-8, R-ID-6, R-ENT-9, R-MCP-2 |
| P4 | Retrieval benchmark in the repo | Move the ANA-37 harness (corpus builder, frozen queries with their hash, metrics, McNemar) from `/tmp` into the repo as a dev tool against a scratch Qdrant; add a corpus read from Postgres once MOD-8 lands; report dense-only beside the hybrid. Could be filed as TOOL instead. | none | R-STO-8 |

Effects on existing items, applied at acceptance: **MOD-112** part (2) is recorded as existing
(`search_concepts`); what remains is workspace scope, and part (1) gains the text filter of Q2. **ANA-38**
gains "memory reaches a prompt without Qdrant" as a constraint. **MOD-135** gains P3 as a later consumer of
its follow-up drafts.

## 10. Requirement edits owed

All maintainer-gated; `docs/REQUIREMENTS.md` is not edited by this item. `<date>` is the acceptance date.
A bracketed condition marks a sentence that depends on a §8 question other than Q7; it is removed on
acceptance, and the sentence is dropped if its question is rejected.

**10.1 `R-NF-2`, proposed text:**

> **R-NF-2 (must).** No required dependency on any external daemon other than Postgres, the agents, and in
> coding sessions the code-context tools (R-LATER-7). Two services are optional, and each fails open with
> nothing else affected when absent: Qdrant with its embedding model for semantic search (R-STO-8), and an
> error-reporting endpoint the user configures in Settings, which is off unless set and receives only
> scrubbed events (R-ID-7, MOD-99). When semantic search is absent, a feature that uses it either has a
> path that works without it, or is not offered, or says plainly that search is unavailable; no run,
> prompt or close-out needs it. ANA-37 measured what semantic search adds and kept Qdrant optional; its
> §7.4 names what reopens the question. Amended by maintainer decision, 2026-10-14, 2026-10-15 (context
> tools; Qdrant to ANA-37), and `<date>` (ANA-37).

**10.2 `R-STO-8`, proposed text:**

> **R-STO-8 (must).** Semantic search over items and requirements. `htui` indexes each item's key, title
> and body, its latest documents, its notes, the scrubbed transcripts of its runs, and each requirement's
> key, body and rationale (withdrawn ones included), into a Qdrant collection (local dense embeddings plus
> BM25 sparse vectors, ranked together) and answers searches scoped to projects or a workspace,
> workspace-owned items, documents and requirements included (R-ENT-16). Agents search through an MCP tool
> (R-MCP-2). A search can be narrowed to decisions: items closed as `done`, `concluded` or `rejected`
> (R-ENT-8), and their documents. The index is derived from Postgres and rebuildable from it (R-STO-1).
> [Only if Q5 is accepted:] The process that executes runs on a box keeps it current. Qdrant and the
> embedding model are optional (R-NF-2): where they are not configured or are unavailable, a search or
> tool that needs them is either not offered or answers with a clear error, and nothing else is
> affected. [Only if Q5 is accepted:] A headless worker (R-ORCH-12) takes the Qdrant URL and API key from
> the same sources as its DSN (R-STO-1). [Only if Q3 is accepted:] Where Qdrant is configured, drafting an
> item shows the nearest existing items for a person to judge; nothing is written from a search score
> (R-ID-6). Added by maintainer decision on MOD-34, 2026-09-25
> (`docs/ANA-19.md`, `docs/ANA-20.md`); amended by maintainer decision on MOD-50, 2026-09-29
> (requirements indexed, decisions by resolution), on the requirements review, 2026-10-15 (notes,
> transcripts, workspace scope, agent tool), and on ANA-37, `<date>` (optional; and, with Q3 and Q5,
> freshness, headless configuration, related items).

**10.3 `R-MCP-2`**, replace "read tools for items, documents and requirements, and semantic search
(R-STO-8)" with "read tools for items, documents and requirements, with a text filter over their keys,
titles and bodies that works without Qdrant, and semantic search (R-STO-8) where Qdrant is configured".

**10.4 `R-ENT-13`**, append: "Memory reaches a prompt without semantic search, which is optional (R-NF-2);
indexing entries in Qdrant and suggesting near-duplicate entries are optional extras (ANA-37)."

**10.5 `R-STO-1`** (with Q5), three exact edits to `docs/REQUIREMENTS.md:257-269`:

1. Replace "Connection string and provider identities live in the OS keyring" with "Connection string,
   provider identities and the Qdrant URL and API key live in the OS keyring".
2. After "Never in `argv` or a plaintext file.", insert: "The same exception, and the development and CI
   environment variable, apply to the Qdrant URL and API key (R-STO-8)."
3. Append to the amendment trail, before its final full stop: ", and on ANA-37, `<date>` (Qdrant URL and
   API key)". The trail then ends "…(environment variable; key bindings moved to Postgres), and on
   ANA-37, `<date>` (Qdrant URL and API key)."

## 11. Not verified

- **Alternative backends were deliberately not surveyed,** per the maintainer's 2026-10-18 scoping:
  pgvector, embedded indexes (Qdrant Edge included, noted only as existing in beta) and other vector
  databases. Whether a lexical index outside Qdrant could carry the share of the duplicate gain that
  htui's BM25 leg delivers (9 of 13 rescues) was therefore not examined.
- **Real agent use of `search_concepts`:** how often agents call it is **UNVERIFIED**; transcripts are in
  the host database, which the sandbox cannot reach.
- **htui's own no-Qdrant baseline** (substring on key and title) was not run in the benchmark; L and G are
  stronger stand-ins.
- **Collapsing, resolved.** The harness collapses chunk hits to one row per ID; htui does not.
  `QdrantStore::search` (`vector.rs:1059-1107`) returns points as Qdrant ranks them, and no collapsing was
  found in `mcp_search.rs`, `concepts.rs`, `concepts_worker.rs` or `tools/search.rs`. Recomputed from the
  uncollapsed points (`S_points`, top 5 points): duplicates 22/24, questions 9/20, identifiers 15/16. The
  headline holds; questions lose one hit.
- **MOD-112's text filter against arm A.** Q2 and §6.1 assume a text filter over Postgres rows performs
  about as well as arm A's grep and whole-file read. That was not measured; an MCP filter with no bulk
  read is likely weaker (§5.9).
- **The real corpus shape:** items and decisions are not yet Postgres rows (MOD-8); notes, transcripts and
  `docs/ANA-*.md` were not in the corpus. Results may change on the real index.
- **Trial limits:** one run per arm, call counts self-reported and not cross-checked against transcripts,
  the model and its version not recorded in this doc.
- **Resource figures:** Qdrant memory and CPU with a populated index under real query load, and sync
  duration on a real backlog, were not measured beyond the 83.7 MB figure. The embedder figures are
  ANA-23's spike, not the real binary. Qdrant's official idle RSS is **UNVERIFIED**.
- **Windows:** running Qdrant as a Windows service is **UNVERIFIED**; `rten` on Windows is unmeasured
  (MOD-16 open).
- **The two-indexer transient deletion** (§2.1) is inferred from code order and not tested.
- **Web evidence marked UNVERIFIED by its reader:** the headline numbers of the "When Lexical Beats Neural"
  preprint (PDF returned 403); the 2026 Stack Overflow Recall@20 figures (search snippet only); the "vibes"
  quote attributed to the Latent Space podcast; whether embeddings supply Linear Triage Intelligence's
  candidates; the Cline/Roo memory-bank design.
- **Whether the UI redesign keeps `Ctrl+F`:** `docs/ui-redesign-findings-2026-10-15.md` does not mention
  it; **UNVERIFIED**.
- **The maintainer's bar** exists in two wordings, "proves very beneficial" and "is very beneficial";
  neither is a verbatim transcript.
- **Artefacts:** the benchmark (`/tmp/ana37-benchmark/`), the trial (`/tmp/ana37-trial/`), the Qdrant
  collection `ana37_concepts` and the Postgres database `ana37` are outside the repo. An hr sandbox
  restart wipes `/tmp` and `~/.cache`; P4 exists so the numbers can be reproduced.
