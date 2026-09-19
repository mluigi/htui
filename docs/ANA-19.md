# ANA-19 - Vector DB for related concepts search

> **Scope note:** Analysis of whether to implement a vector database for searching related concepts in `htui`, explicitly prioritizing performance, scalability, and advanced vector capabilities over deployment simplicity.
>
> **Requirements addressed:** `R-AGT-6`, `R-ID-7` (General context building and retrieval).
>
> **Status (2026-09-19): concluded.** Implementation spawned as MOD-34 (Qdrant related concepts search).

---

## 1. Context and problem statement

`htui` maintains a rich history of architectural decisions (ANA, MOD) and workflows in markdown files (`docs/`, `DECISIONS.md`, `HANDOFF.md`). As the number of items and codebase complexity grows, it becomes harder for an agent (or human) to find related concepts using exact-match text search alone.

Agents building context for a new task need semantic search to surface related historical decisions, architectural invariants, or similar past work items. While an initial analysis favored `pgvector` for simplicity, a deeper review of vector database capabilities reveals that purpose-built engines provide better raw query latency, advanced metadata filtering, and scaling characteristics. 

This document determines which vector database should be integrated into `htui` to enable related concept searches, overriding the constraint for absolute deployment simplicity in favor of the best-performing solution.

---

## 2. Invariants

1. **Performance over Simplicity:** The chosen solution must offer superior latency, scalable filtering, and state-of-the-art vector performance even if it introduces an additional container to the stack.
2. **Online Dependency:** Per MOD-25, `htui` is online-only. We can rely on external embedding APIs rather than bundling local embedding models.
3. **Graceful Degradation:** The orchestrator must not hard-fail if the embedding provider or vector DB is temporarily unavailable; it should fall back to standard keyword search or alert the user.

---

## 3. Options and Verdicts

### 3.1 Infrastructure
**Need:** Where do we store vector embeddings?

| Option | Verdict |
|---|---|
| `pgvector` in PostgreSQL | Rejected. While simple (reusing `PgStore`), it struggles with index build times at massive scales and its query latency (~5ms) and filtering capabilities are outclassed by dedicated engines. |
| Pinecone | Rejected. While highly performant and scalable, it is a fully managed cloud service. This would force users to bring a Pinecone API key and rely on external infrastructure for a core feature, which violates the self-hostability of the orchestrator. |
| Milvus | Rejected. Excellent for billion-scale deployments but architecturally heavy for local/TUI deployments, requiring multiple services (etcd, MinIO) even in standalone mode. |
| **Qdrant** | **Adopted.** A single-binary engine written in Rust. It offers top-tier raw performance (~3-4ms latency), exceptional metadata filtering capabilities, and is lightweight enough to be easily added as a single container in `compose.yaml`. It hits the perfect middle ground between extreme performance and local deployability. |

### 3.2 Embedding Generation and Sync
**Need:** How and when are embeddings generated for concepts (documents, items)?

| Option | Verdict |
|---|---|
| On-the-fly via LLM API | **Adopted.** Since `htui` is explicitly online-only (MOD-25), it can rely on an external embedding model (e.g., OpenAI `text-embedding-3-small` or equivalent). |
| Sync mechanism | **Adopted.** A background worker or a save hook updates the embeddings in Qdrant whenever a markdown file in `docs/` or an item in `HANDOFF.md` changes. `PgStore` remains the source of truth for the raw text; Qdrant stores the embeddings and references the Postgres IDs. |

### 3.3 Store Integration
**Need:** How does this affect `htui`'s architecture?

| Option | Verdict |
|---|---|
| New `VectorStore` trait | **Adopted.** Introduce a `VectorStore` trait with a `QdrantStore` implementation. `PgStore` continues to handle relational data. The orchestrator queries `QdrantStore` for `search_related_concepts(query_embedding, limit)` which returns item/document IDs, then retrieves the full text from `PgStore`. |

---

## 4. Verdict

**Beneficial:** Yes. Implementing **Qdrant** provides the highest performance and most robust filtering for agent context building, successfully bringing semantic search over the project's knowledge base without the excessive overhead of distributed systems like Milvus.

---

## 5. Phasing (MOD spawn plan)

This analysis spawns one implementation item to be opened per `lifecycle.md`:

1. **MOD-34 - Qdrant related concepts search:**
   - Update `compose.yaml` to include the `qdrant/qdrant` Docker image.
   - Introduce a `VectorStore` trait and implement `QdrantStore` using the official Rust client.
   - Implement embedding generation logic (calling an embedding model).
   - Implement background sync to keep embeddings of `docs/` and items updated in Qdrant, keyed by their Postgres/file identifiers.
   - Wire the semantic search to an MCP tool (`search_concepts`) so agents can query it.
