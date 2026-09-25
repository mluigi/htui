# Plan: MOD-34 - Qdrant related concepts search

## Context

Design authority: `docs/ANA-19.md` (Qdrant, `VectorStore` seam) and `docs/ANA-20.md` (local
embeddings via `fastembed-rs`, hybrid Dense + BM25, one collection with a `type` payload index,
no optimizer tuning, no quantization).

Revision 2 (2026-09-25). The first revision of this plan was written before any code existed and
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
| Tasks T1 and T2 are independent | **True** | T1 touches `embed.rs`, `Cargo.toml` (both), `crates/htui/Cargo.toml`; T2 touches `bm25.rs` and the `lib.rs` module list. Intersection empty except `lib.rs`, which T1 does not touch |

## Decisions (for CONFIRM)

- **D1 - BM25 is computed in htui, IDF by Qdrant.** Drop the SPLADE model. A new `bm25.rs`
  tokenises text (lowercase; keeps item IDs such as `MOD-34` and `R-STO-1` whole as well as their
  parts), maps each term to a stable `u32` (first four bytes of its SHA-256) and weights it by
  BM25 term frequency (k1 = 1.2, b = 0.75). The collection's sparse vector gets
  `Modifier::Idf`, so Qdrant supplies the corpus statistics. This is ANA-20's BM25 without a
  second model download, and it is what makes exact identifiers retrievable.
- **D2 - `fastembed`/`ort` move behind an optional `local-embed` feature** on `htui-store`,
  enabled by the `htui` crate. Today every workspace build fetches onnxruntime from
  `parcel.pyke.io`; with the feature off, `cargo test -p htui-store` builds offline and uses a
  test embedder. The binary still ships local embeddings.
- **D3 - Entry point is a CLI pair until MOD-11**: `htui --index-concepts [PATH]` and
  `htui --search-concepts <QUERY>`, in the style of `--set-dsn`. The `search_concepts` MCP tool
  moves to MOD-11 as a cross-link note, because it needs MOD-11's server.
- **D4 - Sources are the repo's files for now**: each HANDOFF.md checklist item becomes one point
  keyed by its ID; each `docs/**/*.md` file is split at `##` headings. Pg items through
  `ReadStore` are left for a follow-up (ANA-19 §3.2 names Postgres as the source of truth, and the
  rev-1 plan already called the filesystem sync a bootstrap).
- **D5 - Pin the compose image** to `qdrant/qdrant:v1.19.1`, matching the client, as Postgres is
  pinned to 16.

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
  across runs (golden values); repeated terms saturate per k1; longer documents weigh a term less
  per b.

### T3 - QdrantStore corrections
- **Files:** `crates/htui-store/src/vector.rs`
- `QdrantStore::connect(&QdrantSettings, embedder)` (settings passed in, not read from the
  keyring inside, so tests can point it anywhere); collection name versioned
  (`htui_concepts_v1`) so an existing SPLADE-shaped collection is never reused.
- Hybrid query: `query_points` with two prefetches (`dense`, `sparse`) fused by RRF; optional
  `type` filter (`doc` / `item`) per ANA-20 §3.4.
- Point ID = UUID built from SHA-256 of the source ID. Payload carries `source` (path or item ID),
  `chunk`, `hash`, `type`.
- `search_concepts` returns hits (`source`, `chunk`, `score`, `type`), not bare strings.
- Offline tests cover ID stability and payload shape; live tests run when
  `HTUI_TEST_QDRANT_URL` is set (skip otherwise, as `HTUI_TEST_DATABASE_URL` does).

### T4 - Sync correctness
- **Files:** `crates/htui-store/src/vector_sync.rs`, `crates/htui-store/src/vector.rs` (trait
  gains `delete_source` and `source_hash`)
- HANDOFF split per checklist item; docs split per `##` section.
- Unchanged `hash` for a source → skip; changed → delete that source's points, then upsert;
  source gone → delete.
- Tests first against an in-memory `VectorStore` fake: skip, replace, delete, per-item split.

### T5 - CLI entry point and graceful failure
- **Files:** `crates/htui/src/cli.rs`, `crates/htui/src/lib.rs`
- `--index-concepts [PATH]` (default: current directory) and `--search-concepts <QUERY>`.
  No Qdrant URL stored, or server unreachable → one clear line on stderr and a non-zero exit;
  the TUI start path is untouched (ANA-19 §2 invariant 3).
- Tests: argument parsing; the no-URL message.

### T6 - Compose pin and bookkeeping
- **Files:** `compose.yaml`, `HANDOFF.md`
- Image pinned per D5. At close-out: MOD-11 gains the `search_concepts` note (D3); the Pg-items
  follow-up is recorded per D4.

## Independence

T1 and T2 are independent (verified above) and may run in parallel. T3 needs both. T4 needs T3.
T5 needs T3 and T4. T6 is last. No ultracode: six tasks, mostly serial.

## Overlap with MOD-7 (running in parallel)

MOD-7 works in `htui-orch`, `htui-agent`, the Settings tab and the store's box rows. This plan does
not touch the Settings tab, `store_worker.rs` or any SQL. Shared files are the module list in
`crates/htui-store/src/lib.rs`, `Cargo.toml`/`Cargo.lock`, `crates/htui/src/cli.rs` and `lib.rs` if
MOD-7 adds flags, and HANDOFF.md at close-out. All are additive hunks.

## Validation

`cargo fmt --check`, `cargo clippy -p htui-store -p htui --all-targets -- -D warnings`,
`cargo test -p htui-store --features test-support` (offline), `cargo test -p htui`. Live Qdrant and
model-download tests run on the maintainer's box (`docker compose up -d qdrant`,
`HTUI_TEST_QDRANT_URL=http://localhost:6334`). In the cloud box, builds need
`ORT_LIB_LOCATION` (see Verified claims) only when `local-embed` is on.

## Status

Awaiting maintainer CONFIRM (revision 2).
