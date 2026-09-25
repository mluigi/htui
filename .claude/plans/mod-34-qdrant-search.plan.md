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
- **Decisions and requirements:** index decisions (a closed item plus its `summary`/`verdict`
  document per ANA-11) and requirements once they are in Postgres (D4, D6). Plan confirmed with
  these folded in, 2026-09-25.
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
- **D4 - Items and their documents, typed points.** Each item is one point keyed by its UUID
  (text: key, title, body). Each item's **latest document of every kind** is split at `##`
  headings into points keyed by UUID v5-style `sha256(document_id, chunk)` (text: item key,
  document kind and title, section). Separate points because BGE-small reads about 512 tokens, so
  an appended write-up would be cut off. Payload on every point: `type` (`item` | `document`; later
  `requirement`), `item_id`, `key`, `project_id`, `kind_id`, `status`, `updated_at`; documents add
  `document_id`, `doc_kind`, `version`, `chunk`. Keyword payload indexes on `type`, `project_id`
  and `status`, so a search is always scoped to projects and can ask for decisions only (closed
  items and their `summary`/`verdict` documents, ANA-11 §4.2). `resolution` joins the payload
  when MOD-38 adds `item.resolution`.
- **D5 - Pin the compose image** to `qdrant/qdrant:v1.19.1`, matching the client.
- **D6 - Requirements are indexed after MOD-38.** ANA-11 puts them in their own `requirement`
  table, which does not exist yet. The collection reserves `type = requirement`; MOD-38 gains a
  cross-link note at close-out.

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

### T3 - Vector store over items and documents
- **Files:** `crates/htui-store/src/vector.rs`
- `VectorStore` trait: `upsert(points)`, `delete(point_ids)`, `indexed(project) ->
  Vec<IndexedPoint>` (point ID, `type`, `item_id`, `updated_at`, `document_id`),
  `search(&SearchQuery) -> Vec<Hit>`. `SearchQuery` = text, projects, optional `types`, optional
  `statuses`, limit. `Hit` = `type`, `item_id`, `key`, `document_id`, `doc_kind`, `score`, and a
  short snippet. `upsert_document`/`upsert_item` go.
- `QdrantStore::connect(&QdrantSettings, embedder)`; collection `htui_concepts_v1` (never reuses
  the first pass's SPLADE-shaped collection) with `dense` (384, cosine) and `sparse`
  (`Modifier::Idf`) vectors and the D4 payload indexes.
- Hybrid query: `query_points` with `dense` and `sparse` prefetches fused by RRF, filtered on
  `project_id` and the optional `type`/`status` sets.
- Offline tests: point IDs, payload shape, filter building. Live tests when
  `HTUI_TEST_QDRANT_URL` is set (skip otherwise, as `HTUI_TEST_DATABASE_URL` does), using
  `HashEmbedder` so no model download.

### T4 - Indexer (replaces the filesystem sync)
- **Files:** `crates/htui-store/src/vector_sync.rs` (rewritten)
- `Indexer::sync(read: &impl ReadStore, projects, store: &impl VectorStore) -> SyncReport`:
  per project, list `ItemSummary`; an item whose `updated_at` differs from the indexed one is
  re-fetched (`item()`) and re-upserted; for every item, `documents()` heads give the latest
  `DocumentId` per kind, and a document ID not yet indexed is fetched (`document()`), split and
  upserted, while points of superseded or vanished documents are deleted; items no longer listed
  lose their points. Documents are immutable per version, so a document ID never needs
  re-embedding.
- Tests first against `htui-core`'s in-memory store and an in-memory `VectorStore` fake: first
  sync indexes items and latest documents; a second sync with no change upserts nothing; an edited
  item is re-indexed; a new document version replaces the old one's points; a removed item loses
  item and document points; other projects are untouched.

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
  > body, and its latest documents, into a Qdrant collection (local dense embeddings plus BM25 sparse vectors, ranked
  > together) and answers searches scoped to projects. The index is derived from Postgres and
  > rebuildable from it (R-STO-1); when Qdrant or the embedding model is unavailable, search fails
  > with a clear error and nothing else is affected.
- MOD-34's line rescoped to Postgres items and citing `R-STO-8`; the file-sync wording goes.
- At close-out: MOD-11 gains the `search_concepts` note, MOD-41 the automatic-sync note (D3),
  MOD-38 the requirement-indexing and `resolution` payload note (D4, D6).
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

Confirmed by the maintainer 2026-09-25 (revision 4: documents and typed points folded in). Implementation in progress.
