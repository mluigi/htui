# Blueprint: MOD-34 — Qdrant related concepts search

**Plan**: `.claude/plans/mod-34-qdrant-search.plan.md`
**Design authority**: `docs/ANA-19.md`, `docs/ANA-20.md`
**Scope**: T1 (Infrastructure), T2 (Dependencies), T3 (VectorStore/Qdrant integration & Embeddings), T4 (Background Sync & Search Backend). All tasks run serially due to dependency and module registration intersections.

---

## 1. Build order and validation, at a glance

Serial: **T1 → T2 → T3 → T4**.

| Task | Crate(s) | Validation |
|---|---|---|
| T1 | root | `docker compose up -d` verifies Qdrant starts. |
| T2 | workspace, `htui-store` | `cargo check -p htui-store` |
| T3 | `htui-store` | `cargo test -p htui-store --all-features` (TDD: tests first for `VectorStore`, `QdrantStore`, and embedding generation). |
| T4 | `htui-store` | `cargo test -p htui-store --all-features` (TDD: tests first for sync logic and search logic). |

---

## 2. Tasks and Files

### 2.1 T1 - Infrastructure Update
| File | Action | What |
|---|---|---|
| `compose.yaml` | edit | Add `qdrant/qdrant:latest` service |

**Details**:
```yaml
  qdrant:
    image: qdrant/qdrant:latest
    container_name: htui-qdrant
    restart: unless-stopped
    ports:
      - "6334:6334"
      - "6333:6333"
    volumes:
      - htui-qdrant-data:/qdrant/storage
```
(Include `htui-qdrant-data:` in `volumes:`).

### 2.2 T2 - Dependencies
| File | Action | What |
|---|---|---|
| `Cargo.toml` | edit | Add `qdrant-client` and `fastembed` to `[workspace.dependencies]` |
| `crates/htui-store/Cargo.toml` | edit | Add `qdrant-client` and `fastembed` to `[dependencies]` |

### 2.3 T3 - VectorStore, QdrantStore, Local Embeddings
| File | Action | What |
|---|---|---|
| `crates/htui-store/src/vector.rs` | create | `VectorStore` trait, `QdrantStore` implementation, payload models |
| `crates/htui-store/src/embed.rs` | create | Local embedding wrapper using `fastembed-rs` (dense + BM25) |
| `crates/htui-store/src/lib.rs` | edit | Register `vector` and `embed` modules |

**Details**:
`VectorStore` trait should include:
- `upsert_document(doc_id, text, metadata)`
- `upsert_item(item_id, text, metadata)`
- `search_concepts(query, limit)`

`QdrantStore` struct will hold the gRPC client, calling out to the `embed.rs` module to generate embeddings (both dense and sparse).

### 2.4 T4 - Background Sync & Search Backend
| File | Action | What |
|---|---|---|
| `crates/htui-store/src/vector_sync.rs` | create | Background worker/sync hook for pushing embeddings to `QdrantStore` from PgStore |
| `crates/htui-store/src/lib.rs` | edit | Register `vector_sync` module |

**Details**:
The sync backend queries Postgres for changed items and documents, generating embeddings and uploading them via the `VectorStore` trait methods. The search backend exposes the `search_concepts` function.
