# Plan: MOD-34 - Qdrant related concepts search

## Context
Per ANA-19 and ANA-20, we need to implement a vector database for semantic search over historical decisions (`docs/`, `HANDOFF.md`) to help agents build context. 
We will use Qdrant for storage and hybrid search (Dense + BM25), and `fastembed-rs` for local embedding generation. The functionality will eventually back an MCP tool `search_concepts` (to be exposed via the `htui MCP server` in MOD-11).

## Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| `search_concepts` MCP tool exists | False | `rg -l 'MCP tool'` and `HANDOFF.md` confirm `MOD-11` (htui MCP server) is not built yet. We will build the backend logic for `search_concepts` but defer transport wiring to MOD-11. |
| `qdrant-client` and `fastembed` are in workspace | False | `grep -E 'fastembed|qdrant' Cargo.toml` yielded no results. They must be added to `htui-store/Cargo.toml`. |
| T2 and T3 file sets are independent | False | Both require editing `crates/htui-store/Cargo.toml` and `crates/htui-store/src/lib.rs`. They will be executed sequentially. |

## Tasks

### 1. Infrastructure Update
- **Files touched:** `compose.yaml`
- **Action:** Add the `qdrant/qdrant:latest` image to `compose.yaml` exposing port 6334 (gRPC) and 6333 (HTTP). This allows local development.

### 2. Dependencies
- **Files touched:** `crates/htui-store/Cargo.toml`, `Cargo.toml` (workspace)
- **Action:** Add `qdrant-client` and `fastembed` to the workspace `Cargo.toml` and to `htui-store` dependencies.

### 3. VectorStore Trait & Qdrant Implementation
- **Files touched:** `crates/htui-store/src/vector.rs`, `crates/htui-store/src/lib.rs`
- **Action:** 
  - Introduce `VectorStore` trait with methods `upsert_document`, `upsert_item`, and `search_concepts`.
  - Implement `QdrantStore` using `qdrant-client`.
  - Initialize a single Qdrant collection configured for both dense and sparse (BM25) vectors.
  - Implement payload indexing (e.g. `type: "doc" | "item"`) per ANA-20.

### 4. Local Embeddings via FastEmbed
- **Files touched:** `crates/htui-store/src/embed.rs`, `crates/htui-store/src/lib.rs`
- **Action:**
  - Create a helper module wrapping `fastembed` (e.g., using `bge-small-en-v1.5` for dense embeddings).
  - Include BM25 sparse vector generation if supported by the wrapper, or fallback to Qdrant's sparse vector tooling.

### 5. Background Sync & Search Backend
- **Files touched:** `crates/htui-store/src/vector_sync.rs`, `crates/htui-store/src/lib.rs`
- **Action:**
  - Implement the background worker that monitors or iterates `HANDOFF.md` and `docs/` to hash and upsert documents into `QdrantStore`.
    - *Note:* This filesystem sync is a temporary bootstrap mechanism for `htui`'s own development. Once legacy markdown data is imported to Postgres (`R-LATER-3`), the permanent architecture will sync embeddings directly from `PgStore` inserts/updates.
  - Implement the `search_concepts` backend logic that queries `QdrantStore` (Hybrid search) and returns matching metadata/identifiers. (The MCP tool exposure is deferred to MOD-11, but the API will be ready).

## Independence
Tasks 1 and 2 must run first. Tasks 3, 4, 5 are sequential because they share module registration (`lib.rs`) and sequentially build upon each other (Embeddings -> Store -> Sync).

## Status
Awaiting maintainer CONFIRM.
