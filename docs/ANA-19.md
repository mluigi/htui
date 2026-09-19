# ANA-19 - Vector DB for related concepts search

> **Scope note:** Analysis of whether to implement a vector database for searching related concepts in `htui`, specifically evaluating the update of the existing Postgres instance to include pgvector.
>
> **Requirements addressed:** `R-AGT-6`, `R-ID-7` (General context building and retrieval).
>
> **Status (2026-09-19): concluded.** Implementation spawned as MOD-34 (pgvector related concepts search).

---

## 1. Context and problem statement

`htui` maintains a rich history of architectural decisions (ANA, MOD) and workflows in markdown files (`docs/`, `DECISIONS.md`, `HANDOFF.md`). As the number of items and codebase complexity grows, it becomes harder for an agent (or human) to find related concepts using exact-match text search alone.

Agents building context for a new task need semantic search to surface related historical decisions, architectural invariants, or similar past work items. Adding a dedicated vector database adds a significant operational overhead. Since `htui` already relies on PostgreSQL (`PgStore`), adding the `pgvector` extension allows unified relational and vector storage without introducing new external infrastructure.

This document determines if and how `pgvector` should be integrated into `htui` to enable related concept searches.

---

## 2. Invariants

1. **Unified Architecture:** Vector data must live alongside relational data to simplify deployment and allow hybrid search (e.g., semantic search filtered by project ID or status).
2. **Online Dependency:** Per MOD-25, `htui` is online-only. We can rely on external embedding APIs rather than bundling local embedding models.
3. **Graceful Degradation:** The orchestrator must not hard-fail if the embedding provider is temporarily unavailable; it should fall back to standard keyword search or alert the user.

---

## 3. Options and Verdicts

### 3.1 Infrastructure
**Need:** Where do we store vector embeddings?

| Option | Verdict |
|---|---|
| Separate Vector DB (Qdrant, Pinecone) | Rejected. Adds infrastructure complexity, requires managing separate data sync processes, and violates the desire for a simple local stack. |
| `pgvector` in existing PostgreSQL | **Adopted.** The `compose.yaml` dev Postgres can easily use a pgvector-enabled image (e.g., `pgvector/pgvector:pg16`). This allows hybrid search in a single SQL query. |

### 3.2 Embedding Generation and Sync
**Need:** How and when are embeddings generated for concepts (documents, items)?

| Option | Verdict |
|---|---|
| On-the-fly via LLM API | **Adopted.** Since `htui` is explicitly online-only (MOD-25), it can rely on an external embedding model (e.g., OpenAI `text-embedding-3-small` or equivalent). |
| Sync mechanism | **Adopted.** A background worker or a save hook updates the embeddings whenever a markdown file in `docs/` or an item in `HANDOFF.md` changes. |

### 3.3 Store Integration
**Need:** How does this affect `PgStore`?

| Option | Verdict |
|---|---|
| Schema & Traits | **Adopted.** Requires a new migration (e.g., `0004_pgvector.sql`) that runs `CREATE EXTENSION IF NOT EXISTS vector;` and adds a `vector` column (or a new table `document_embeddings`) with an HNSW index. `ReadStore` needs a new method like `search_related_concepts(query_embedding, limit)`. |

---

## 4. Verdict

**Beneficial:** Yes. Implementing `pgvector` provides significant value for agent context building by enabling semantic search over the project's knowledge base without adding new database infrastructure. 

---

## 5. Phasing (MOD spawn plan)

This analysis spawns one implementation item to be opened per `lifecycle.md`:

1. **MOD-34 - pgvector related concepts search:**
   - Update `compose.yaml` to use a `pgvector`-enabled image.
   - Add schema migration `0004_pgvector.sql` to enable the extension and add vector tables/columns.
   - Implement embedding generation logic (calling an embedding model).
   - Implement background sync to keep embeddings of `docs/` and items updated.
   - Add `search_related_concepts` to `ReadStore` and wire it to an MCP tool (`search_concepts`) so agents can query it.
