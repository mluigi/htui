# MOD-34 - Qdrant concepts search over items and their documents (done, 2026-09-25)

Satisfies **`R-STO-8`**, added for this item by maintainer decision on 2026-09-25. Design authority:
`docs/ANA-19.md` (Qdrant, a `VectorStore` seam) and `docs/ANA-20.md` (local dense embeddings,
hybrid Dense + BM25, one collection with payload indexes). Both ANA documents cite `R-AGT-6` and
`R-ID-7`, which are about agent autodiscovery and secret scrubbing; `R-STO-8` is the requirement they
actually serve. The ANA text is left as written, since ANA edits are maintainer-only.

Artifact: plan `.claude/plans/mod-34-qdrant-search.plan.md`, revision 4 (routed **plan**, C2 fired;
no PRD). The revision-1 blueprint next to it predates the code and is superseded by the plan.
Commits: `601e5f8` (the build, R-STO-8, close-out) and `3e616f0` (review fixes), on top of the plan
revisions `bfd0b9f`, `5c55de7` and `9c3969c`.

## Starting point

A first pass had landed unlabelled inside the import commit `2698f0b`: a `qdrant` compose service,
`qdrant-client`/`fastembed`/`ort` dependencies, and `htui-store` modules `vector.rs`, `embed.rs` and
`vector_sync.rs`. Nothing called them, and they fell short of ANA-20:

- search used only the dense vector (the sparse query vector was computed and dropped);
- the "BM25" vector was SPLADE++, the only sparse model `fastembed 3.14.1` ships;
- point IDs came from `std::hash::DefaultHasher`, which std does not promise stable across releases;
- the sync walked `HANDOFF.md` and `docs/` on disk, never deleted stale chunks and never skipped
  unchanged ones.

## Maintainer decisions (2026-09-25)

- **Postgres items only.** The index covers items and their documents from the store. Indexing the
  repo's own markdown was dropped: it would only serve htui's development, and there is no shared
  Qdrant for that.
- **Decisions and requirements are in scope.** A decision is a closed item plus its
  `summary`/`verdict` document (ANA-11 §4.2), so documents are indexed and every point carries the
  item's status. Requirements live in MOD-38's `requirement` table, which does not exist yet; the
  collection reserves `type = requirement` for them.
- **fastembed stays for dense vectors; BM25 is computed in htui.** SPLADE's learned expansion
  overlaps what the dense model already covers, costs a second transformer pass per point and per
  query plus a second download, and splits keys like `MOD-34` into word pieces. BM25 keeps exact
  identifiers exact, which is why ANA-20 wanted a sparse side.
- **A requirement ID was added** (`R-STO-8`).

## What was built

- **`htui_store::bm25`**: a tokenizer that keeps `MOD-34` whole and also as `mod`, `34`; term index
  = the first four bytes of the term's SHA-256; document weights = saturated, length-normalised term
  frequency (k1 1.2, b 0.75, average length fixed at 256 tokens). The collection's sparse vector
  uses `Modifier::Idf`, so Qdrant supplies IDF from the corpus.
- **`htui_store::embed`**: a `DenseEmbedder` seam. `FastEmbedder` (BGE-small-en-v1.5, 384
  dimensions, model cached under `<cache>/htui/fastembed`) sits behind the new **`local-embed`**
  feature, which the `htui` binary turns on. `HashEmbedder` (test-support) needs no model.
- **`htui_store::vector`**: `VectorStore` (`upsert`, `delete`, `indexed`, `search`), `QdrantStore`
  over collection **`htui_concepts_v1`** (`dense` cosine plus `sparse` IDF vectors, keyword payload
  indexes on `type`, `project_id`, `status`, `item_id`), and `MemVectorStore` (test-support). An
  item point's ID is the item's UUID; a document section's is a version-8 UUID from SHA-256 of the
  document ID and section number. Search is a `query_points` request with dense and sparse
  prefetches fused by RRF. It is always filtered to the given projects, and optionally to point
  types and statuses. A search with no projects returns nothing.
- **`htui_store::vector_sync::Indexer`**: an incremental, idempotent sync through `ReadStore`, so it
  runs against `PgStore`, the mirror or `MemStore` alike. An item is rebuilt when its `updated_at` or
  its `status` moved (a `transition` does not touch `updated_at`), or when its set of latest
  documents changed. A rebuild rewrites all of the item's points, because document points repeat the
  status. Documents are split at `#`/`##` headings and capped at 2000 characters per point, since
  BGE-small reads about 512 tokens. Items no longer listed lose their points.
- **CLI**: `htui --index-items [--project SLUG]` and
  `htui --search-items QUERY [--project SLUG] [--decisions] [--limit N]`. Both run before any
  terminal work and exit. `--decisions` keeps `done` and `closed` items until MOD-38 adds
  `item.resolution`. A missing Qdrant URL, DSN, unreachable server or pending migration is one line
  on stderr and a non-zero exit. The TUI start path is untouched.
- **`compose.yaml`** pins `qdrant/qdrant:v1.19.1`, matching the client.
- The workspace's direct `ort` pin and `htui-store`'s `walkdir` dependency are gone (`fastembed`
  already pins `ort`; the filesystem sync was the only `walkdir` user in the crate).

## Verification

- `cargo test -p htui-store --lib`: 46 passed offline, with no model and no onnxruntime. Coverage:
  BM25, `HashEmbedder`, point IDs, payload round trips, filters, and the indexer's six cases.
- `tests/qdrant_live.rs`: three cases against a real Qdrant **1.19.1** (sync, then resync writes
  nothing; exact-key search; project scoping; type and status filters; delete and re-sync). Gated on
  `HTUI_TEST_QDRANT_URL`, so they skip without it.
- `cargo clippy -p htui-store -p htui --all-targets -- -D warnings` is clean, with `local-embed`
  on.
- **Not run in the cloud box:** `FastEmbedder` against the real model (`#[ignore]`d test
  `fast_embedder_returns_384_dims`). The proxy blocks HuggingFace. It needs one run on a box with
  network.

## Review

The configured reviewer (`rust-reviewer`) found no blocking defects. Three fixes were applied:
- qdrant-client's compatibility check is skipped, because it printed to stdout and blocked on its
  own health probe;
- `--limit` is bounded to 1..=1000, and the prefetch depth uses a saturating multiply;
- control characters are stripped from snippets, keys and document kinds before printing.

Deferred:
- Points whose payload no longer parses are invisible to `indexed()`, so a sync never deletes them.
  This matters once MOD-38 writes `requirement` points.
- A change to the chunking rules needs a new collection name, because document freshness compares
  IDs, not the way the text was split.
- `FastEmbedder::new` loads the model synchronously. That is fine for the CLI, but it should move to
  the blocking pool when MOD-41 reuses it.
- Each sync makes one `documents()` query per item.

## Build note

`ort 2.0.0-rc.4` downloads onnxruntime from `parcel.pyke.io` at build time whenever `local-embed` is
on, so the `htui` binary does too. An offline build sets `ORT_LIB_LOCATION` to a directory holding
`libonnxruntime.so` 1.18 (for example from the `onnxruntime-node@1.18.0` npm package), plus
`LD_LIBRARY_PATH` to run. `cargo test -p htui-store` without `--all-features` needs neither.

## Left to other items

- **MOD-11**: the agent-facing `search_concepts` tool over `VectorStore::search`.
- **MOD-41**: automatic sync as a headless-worker background job; until then it is
  `htui --index-items`.
- **MOD-38**: index requirements (`type = requirement`) once the table exists, and add
  `item.resolution` to the payload so "decisions" stops meaning "done or closed".
