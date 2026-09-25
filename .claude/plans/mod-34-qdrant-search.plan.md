# Plan: MOD-34 - Qdrant related concepts search

## Context

Design authority: `docs/ANA-19.md` (Qdrant, `VectorStore` seam) and `docs/ANA-20.md` (local
embeddings via `fastembed-rs`, hybrid Dense + BM25, one collection with a `type` payload index,
no optimizer tuning, no quantization).

Revision 3 (2026-09-25; revision 2 the same day planned a filesystem sync the maintainer then ruled out). The first revision of this plan was written before any code existed and
was never confirmed. A first-pass implementation has since landed in the tree (inside the import
commit `2698f0b`, not labelled as MOD-34): `compose.yaml` has a `qdrant` service, the workspace
and `htui-store` depend on `qdrant-client 1.19.0`, `fastembed 3.14.1` and `ort =2.0.0-rc.4`,
`htui-store` has `vector.rs` (`VectorStore`, `QdrantStore`), `embed.rs` (`Embedder`),
`vector_sync.rs` (`VectorSync`) and `qdrant_settings.rs`, and the Settings tab has a Qdrant
section that stores the URL and API key in the OS keyring. This revision fact-checks that code
against ANA-20 and plans only what is left.

Routed as **plan** (C2 fired: new `VectorStore` seam; C1, C3, C4 did not). Maintainer accepted the
route on 2026-09-25.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `qdrant-client` and `fastembed` are not in the workspace (rev 1) | **False** | `Cargo.toml` `[workspace.dependencies]` declares `qdrant-client = "1.19.0"`, `fastembed = "3.14.1"`, `ort = "=2.0.0-rc.4"`; `crates/htui-store/Cargo.toml` uses all three |
| `compose.yaml` needs a Qdrant service (rev 1 T1) | **False, done** | `compose.yaml:51` `qdrant:` service, `image: qdrant/qdrant:latest`, ports 6333/6334, volume `htui-qdrant-data` |
| The store does hybrid (Dense + BM25) search | **False** | `vector.rs` `search_concepts` computes a sparse query vector, binds it to `_sparse_embeddings` and never uses it; the query is `search_points` on `"dense"` only |
| The sparse vectors are BM25 | **False** | `embed.rs` builds `SparseTextEmbedding::try_new(Default::default())`; `fastembed-3.14.1/src/models/sparse.rs` lists one model, `SPLADEPPV1`. No BM25 exists in this fastembed version |
| Point IDs are stable | **False** | `vector.rs` `generate_point_id` hashes with `std::collections::hash_map::DefaultHasher`, whose algorithm std documents as unspecified across releases; the workspace pins rustc, so it holds until the next toolchain bump |
| Sync removes stale points | **False** | `vector_sync.rs` only upserts `"{path}-{i}"` chunks; a file that shrinks or is deleted leaves its old chunks searchable |
| Sync skips unchanged content | **False** | `sync_handoff` computes a SHA-256 and stores it as payload, but nothing reads it back; every run re-embeds everything |
| HANDOFF items are indexed as items | **False** | `sync_handoff` splits HANDOFF.md into 2000-character chunks named `HANDOFF-{i}`, so a hit on "MOD-34" returns a chunk, not the item |
| Anything constructs `QdrantStore` or runs `VectorSync` | **False** | No caller outside the three modules (`rg 'QdrantStore\|VectorSync\|Embedder' crates`) |
| The `search_concepts` MCP tool can be wired now | **False** | MOD-11 (htui MCP server) is open; no MCP server exists in the tree |
| Qdrant client 1.19 has what hybrid needs | **True** | `qdrant-client-1.19.0/src/builders/prefetch_query_builder.rs`, `query_points_builder.rs`; `qdrant.rs:1982` `Modifier::Idf` |
| `PointId` can be built from a UUID string | **True** | `grpc_conversions/primitives.rs:192` `impl From<String> for PointId` |
| A Qdrant server matching client 1.19 exists | **True** | Docker Hub `qdrant/qdrant` tags include `v1.19.0`, `v1.19.1` (registry tag list, 2026-09-25) |
| `htui-store` builds without network | **False** | `ort-sys 2.0.0-rc.4` `download-binaries` fetches `parcel.pyke.io/.../msort_static-v1.18.1` at build time; it failed here behind the proxy. It built with `ORT_LIB_LOCATION` pointing at `libonnxruntime.so.1.18.0` taken from the `onnxruntime-node@1.18.0` npm package |
| Pg items can be read without new SQL | **True** | `htui-core/src/store/traits.rs:68` `ReadStore::items`, `:70` `ReadStore::item` |
| ANA-19/ANA-20's requirement IDs cover this work | **False** | Both cite `R-AGT-6` (agent autodiscovery) and `R-ID-7` (secret scrubbing); neither is about search, and the MOD-34 line cites none |
| Tasks T1 and T2 are independent | **True** | T1 touches `embed.rs`, `Cargo.toml` (both), `crates/htui/Cargo.toml`; T2 touches `bm25.rs` and the `lib.rs` module list. Intersection empty |
| Items carry what an incremental sync needs | **True** | `htui-core/src/model/item.rs:131` `ItemSummary` has `id`, `project_id`, `key`, `title`, `updated_at`; `Item` adds `body` |
| The in-memory store can drive offline tests | **True** | `htui-core/src/store/mem.rs:4238` implements `ReadStore::items` / `item` |

## Maintainer answers (2026-09-25)

- **Sources:** index **Postgres items only**. That is the objective; local files would only serve
  htui's own development and there is no shared Qdrant for that yet. The filesystem sync
  (`HANDOFF.md`, `docs/`) is removed, not extended.
- **Requirement:** add one. Proposed **R-STO-8** (T6).
- **fastembed stays** for the dense vectors (BGE-small-en-v1.5). The question was only which
  sparse vector sits beside it: SPLADE or BM25 (D1).

## Decisions (for CONFIRM)

- **D1 - BM25 sparse vectors computed in htui, IDF applied by Qdrant; SPLADE dropped.**
  fastembed stays for dense. fastembed 3.14.1 offers no BM25 (its only sparse model is SPLADE++),
  so `bm25.rs` tokenises text (lowercase; keeps keys such as `MOD-34` whole as well as their parts),
  maps each term to a stable `u32` (first four bytes of its SHA-256) and weights it by BM25 term
  frequency (k1 = 1.2, b = 0.75). The collection's sparse vector gets `Modifier::Idf`. Reason:
  ANA-20 §3.3 wants the sparse side for exact identifiers; dense already covers meaning, SPLADE's
  learned expansion overlaps it, and SPLADE costs a second transformer pass per item and per query
  plus a second model download.
- **D2 - `fastembed`/`ort` behind an optional `local-embed` feature** on `htui-store`, enabled by
  the `htui` crate. Every workspace build today fetches onnxruntime from `parcel.pyke.io`; with the
  feature off, `cargo test -p htui-store` builds offline and uses a test embedder.
- **D3 - Entry point is a CLI pair**: `htui --index-items` (incremental sync of every project the
  connection sees) and `htui --search-items <QUERY> [--project <KEY>]`, in the style of
  `--set-dsn`. Automatic sync belongs with the headless worker's background jobs (MOD-41) and the
  `search_concepts` agent tool with MOD-11; both get a cross-link note at close-out.
- **D4 - One point per item, keyed by the item's UUID.** Text = key, title and body. Payload:
  `item_id`, `key`, `project_id`, `kind_id`, `status`, `updated_at`. Keyword payload index on
  `project_id` (ANA-20 §3.4's single collection, with the project as the tenant) so a search is
  always scoped to projects. Item documents are not indexed in this item.
- **D5 - Pin the compose image** to `qdrant/qdrant:v1.19.1`, matching the client.

## Tasks

### T1 - Embedder seam and feature gate
- **Files:** `crates/htui-store/src/embed.rs`, `crates/htui-store/Cargo.toml`, `Cargo.toml`,
  `crates/htui/Cargo.toml`
- `DenseEmbedder` trait (`embed(&self, texts) -> Vec<Vec<f32>>`, `dim()`); `FastEmbedder`
  (BGE-small-en-v1.5, 384 dims) under `cfg(feature = "local-embed")`; a deterministic
  `HashEmbedder` for tests under `test-support`. SPLADE removed.
- Tests first: `HashEmbedder` is deterministic and returns `dim()` values; the ignored
  model-download test stays ignored and moves under the feature.

### T2 - BM25 sparse vectors
- **Files:** `crates/htui-store/src/bm25.rs` (new), `crates/htui-store/src/lib.rs`
- Tests first: tokenizer keeps `MOD-34` whole and also yields `mod`, `34`; term index is stable
  (golden values); repeated terms saturate per k1; longer texts weigh a term less per b.

### T3 - Item vector store
- **Files:** `crates/htui-store/src/vector.rs`
- `VectorStore` becomes item-only: `upsert_items`, `delete_items`, `indexed(project) -> (ItemId,
  updated_at)` pairs, `search(query, projects, limit) -> Vec<ItemHit>` (`item_id`, `key`,
  `score`). `upsert_document` goes.
- `QdrantStore::connect(&QdrantSettings, embedder)`; collection `htui_items_v1` (never reuses the
  first pass's SPLADE-shaped collection); point ID = item UUID.
- Hybrid query: `query_points` with `dense` and `sparse` prefetches fused by RRF, filtered on
  `project_id`.
- Offline tests: point ID and payload shape. Live tests when `HTUI_TEST_QDRANT_URL` is set (skip
  otherwise, as `HTUI_TEST_DATABASE_URL` does), using `HashEmbedder` so no model download.

### T4 - Item indexer (replaces the filesystem sync)
- **Files:** `crates/htui-store/src/vector_sync.rs` (rewritten), `crates/htui-store/src/lib.rs`
  only if the module is renamed
- `ItemIndexer::sync(read: &impl ReadStore, projects, store: &impl VectorStore)`: list
  `ItemSummary` per project, compare `updated_at` with what is indexed, fetch bodies (`item()`)
  only for new or changed items, upsert them, delete indexed items no longer listed.
- Tests first against `htui-core`'s in-memory store and an in-memory `VectorStore` fake: first
  sync indexes all; a second sync with no change fetches nothing; an edited item is re-indexed; a
  removed item is deleted; other projects are untouched.

### T5 - CLI entry point and graceful failure
- **Files:** `crates/htui/src/cli.rs`, `crates/htui/src/lib.rs`
- `--index-items` and `--search-items <QUERY> [--project <KEY>]` per D3. No Qdrant URL stored,
  Qdrant unreachable, or Postgres unreachable → one clear line on stderr and a non-zero exit; the
  TUI start path is untouched (ANA-19 §2 invariant 3).
- Tests: argument parsing and conflicts; the no-URL message.

### T6 - Requirement, compose pin and bookkeeping
- **Files:** `docs/REQUIREMENTS.md`, `compose.yaml`, `HANDOFF.md`
- New requirement, section 4:
  > **R-STO-8 (must).** Semantic search over items. `htui` indexes each item's key, title and
  > body into a Qdrant collection (local dense embeddings plus BM25 sparse vectors, ranked
  > together) and answers searches scoped to projects. The index is derived from Postgres and
  > rebuildable from it (R-STO-1); when Qdrant or the embedding model is unavailable, search fails
  > with a clear error and nothing else is affected.
- MOD-34's line rescoped to Postgres items and citing `R-STO-8`; the file-sync wording goes.
- At close-out: MOD-11 gains the `search_concepts` note, MOD-41 the automatic-sync note (D3).
  ANA-19/ANA-20 keep their `R-AGT-6`/`R-ID-7` citations (ANA edits are maintainer-only); the
  write-up records that `R-STO-8` is the requirement they serve.

## Independence

T1 and T2 are independent (verified above) and may run in parallel. T3 needs both. T4 needs T3.
T5 needs T3 and T4. T6 is last. No ultracode: six tasks, mostly serial.

## Overlap with MOD-7 (running in parallel)

MOD-7 works in `htui-orch`, `htui-agent`, the Settings tab and the store's box rows. This plan does
not touch the Settings tab, `store_worker.rs` or any SQL, and reads items only through the existing
`ReadStore` trait. Shared files are the module list in `crates/htui-store/src/lib.rs`,
`Cargo.toml`/`Cargo.lock`, `crates/htui/src/cli.rs` and `lib.rs` if MOD-7 adds flags,
`docs/REQUIREMENTS.md` if MOD-7 amends a requirement, and HANDOFF.md at close-out. All are
additive hunks.

## Validation

`cargo fmt --check`, `cargo clippy -p htui-store -p htui --all-targets -- -D warnings`,
`cargo test -p htui-store --features test-support` (offline), `cargo test -p htui`, the handoff-run
validator. Live Qdrant and model-download tests run on the maintainer's box
(`docker compose up -d qdrant`, `HTUI_TEST_QDRANT_URL=http://localhost:6334`). In the cloud box,
builds with `local-embed` need `ORT_LIB_LOCATION` (see Verified claims).

## Status

Awaiting maintainer CONFIRM (revision 3).
