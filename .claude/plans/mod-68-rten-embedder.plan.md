# Plan: MOD-68 - Replace `fastembed`/`ort` with an `rten` embedder and a pinned weight fetch

**Source**: `HANDOFF.md` MOD-68 (from ANA-23; `docs/ANA-23.md` §7-§9, `docs/decisions/ana/ana-23.md`)
**Route**: plan (maintainer-accepted verdict, 2026-10-01; criteria 0 fired, C4 borderline)
**Selected Milestone**: the only one (ANA-23 §8)
**Requirements**: `R-STO-8`, `R-NF-1`, `R-NF-2`, `R-NF-3`
**Complexity**: Medium
**Status**: DONE 2026-10-01 (write-up `docs/decisions/mod/mod-68.md`); CONFIRMED by the maintainer 2026-10-01 (OQ-1 and OQ-2 defaults accepted)

## Summary

`htui-store`'s dense embedder moves from `fastembed` 3.14.1 (ONNX Runtime through `ort`, downloaded
at build time; `hf-hub` 0.3.2, which cannot load the model on a fresh install today) to `rten` 0.26
plus `tokenizers` 0.23 on `fancy-regex`. Both run the same `Xenova/bge-small-en-v1.5`
`onnx/model.onnx`, so the stored Qdrant vectors stay valid. The weights are fetched on first use over
the workspace `reqwest`, pinned to commit `ea104dac` and sha256-checked per file. An existing
fastembed cache is reused. The model's identity is written to the Qdrant collection's metadata and
checked on connect. fastembed's vectors are recorded as goldens before fastembed is removed.

## Decisions (this plan)

- **D1 - Goldens recorded from fastembed, committed as a fixture, before any rten code.** Six texts:
  short, a typical item point, a 512+-token text that truncates, Unicode with CJK and emoji, empty,
  and the exact key `MOD-34`. The recorder is an `#[ignore]` test behind `local-embed` that writes
  the fixture only when `HTUI_RECORD_GOLDENS=1`. It is deleted with fastembed in T5. The fixture
  stays and records its provenance (fastembed 3.14.1, commit `ea104dac`, date).
- **D2 - Golden check runs as an `#[ignore = "needs the BGE model (133 MB)"]` test** and is part of
  this item's gate (`-- --ignored`). Tolerance: 1e-5 per element (ANA-23 §8.1), plus cosine ≥ 1 - 1e-9.
  Ignored by default so the normal suite never downloads 133 MB.
- **D3 - `RtenEmbedder` replaces `FastEmbedder`.** Same seam (`DenseEmbedder`), `Clone`
  (`Arc` over model, tokenizer and pool), still behind `local-embed`, whose dependency set becomes
  `rten`, `tokenizers`, `reqwest`, `rustls`. `RtenEmbedder::load(dir: &ModelFiles)` is synchronous,
  CPU-only and never touches the network. `embed` runs on `spawn_blocking` as today.
- **D4 - Inference contract = fastembed's (ANA-23 §2.2).** Truncate at 512, pad `BatchLongest` with
  `[PAD]`/0, inputs `input_ids`/`attention_mask`/`token_type_ids` as `NdTensor<i32, 2>`, output
  `last_hidden_state[:, 0, :]`, `v / (‖v‖ + 1e-12)`. An explicit `rten::ThreadPool` sized by
  `available_parallelism()` goes through `RunOptions::default()` with `thread_pool` assigned.
  The tokenizer settings that fastembed read from `config.json` and the two token-map JSONs become
  constants. Only `model.onnx` and `tokenizer.json` are fetched.
- **D5 - Padded-token cap per forward pass.** A call's texts are tokenised once, then split in input
  order into sub-batches whose `rows × longest` stays ≤ `MAX_BATCH_TOKENS` (8,192, which is
  16 × 512). A single text is never split. Output order is input order. This fixes the
  256-requirement chunk that one long text pads (ANA-23 §9).
- **D6 - Fetch is async, load is blocking.** New `htui_store::model` module (behind `local-embed`)
  owns the repo, commit, file list, sha256 pins, URLs and tokenizer constants. It has one entry
  point, `async fn ensure_model() -> Result<ModelFiles, StoreError>`, which works in this order:
  1. Use `<cache>/htui/model/bge-small-en-v1.5-ea104dac/` if every file hashes to its pin.
  2. Else adopt from any `<cache>/htui/fastembed/models--Xenova--bge-small-en-v1.5/snapshots/*/`
     whose files hash to the pins. It copies (fastembed's blobs may be symlinks), never moves.
  3. Else download `https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/<commit>/<file>`,
     streaming and hashing to `<name>.part`, `sync_all`, then rename only on a hash match.

  Any failure is a `StoreError::Backend` naming the file and the URL; a hash mismatch also names
  both hashes. There is never a panic. The crypto-provider install and two-timeout client mirror
  `htui-agent/src/install/http.rs`. That client is `pub(crate)` and `htui-store` has no edge to
  `htui-agent`, so the pattern is mirrored, not imported (no new crate edge). `apart()` threads have no runtime, so the fetch runs on the caller's
  runtime and only the load goes off-thread.
- **D7 - Construction sites** (`R-NF-3`, all off the UI thread):
  - `concepts.rs` `open()` (CLI): `ensure_model().await`, then `spawn_blocking(load)`, no longer an
    inline blocking `FastEmbedder::new()`.
  - `open_index`: `ensure_model().await`, then `apart("htui-index-model", load)`. Its retry loop
    is unchanged.
  - `concepts_worker.rs` `load_model`: `ensure_model().await` inside the boxed future, then
    `spawn_blocking(load)`. The `Loader`/`OnceCell` shape (D238/D245) is unchanged.
- **D8 - Model identity in the Qdrant collection's `metadata`** (`embedder` key: repo, commit,
  `model.onnx` sha256, dim, pooling `cls`, normalisation `l2`). It is not a Postgres `app_setting`,
  because the identity describes these vectors and must vanish when the collection does. It
  becomes a new `DenseEmbedder::identity(&self) -> EmbedderIdentity` method; `HashEmbedder` answers
  `hash/<dim>`. `ensure_collection` handles three cases:
  - Collection created: the identity is written in `CreateCollection`.
  - Collection exists with no `embedder` key (every pre-MOD-68 index): the identity is stamped with
    `UpdateCollection`. This is safe because fastembed's identity equals rten's (ANA-23 §5.6).
  - Collection exists with a different identity, or its `dense` size differs from `dim()`:
    `StoreError::Backend` naming both identities and the remedy. Search and index both refuse
    (`R-STO-8`: a clear error, nothing else affected).
- **D9 - Remove `fastembed`** from the workspace and `htui-store`, plus the workspace `ureq` entry,
  which no crate names. Update the README build notes (lines 54, 385, 498), the `local-embed`
  feature comment and the stale "`FastEmbedder` is not `Clone`" comment (`concepts.rs:313`).
- **D10 - The Windows/macOS gate is a cross `cargo check`** of the embedder stack, run in the
  sandbox before T5. A real Windows/macOS run stays with MOD-16 and the host (see OQ-1).

## Blueprint amendments (code-architect, 2026-10-01)

`.claude/plans/mod-68-rten-embedder.blueprint.md` §0 records hazards H-1..H-23 and amends this plan.
Where the two disagree, the blueprint wins:
- **A-1** dev-profile `opt-level = 3` for `rten*` and `sha2` (debug rten is ~118x slower).
- **A-2** the tokenizer only truncates; padding is done per sub-batch by hand.
- **A-3** a `.part` older than 10 min is swept.
- **A-4** `open_index` keeps the loaded model across retries.
- **A-5** `openssl-sys` stays because of `sentry` alone, and `docker/hr/Dockerfile:15` is updated.
- **A-6** the D10 cross-check runs in a scratch crate after `rustup target add`.
- **A-7** `model.rs` is born in T2 (constants, `ModelFiles`).
- **A-8** an ignored `real_download_from_huggingface` test runs in the gate.
- **A-9** every pin carries its byte size, and an oversize download is refused.
- **A-10** the `dense` size is checked before the identity.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / seam | `crates/htui-store/src/embed.rs:13-21` | `DenseEmbedder` trait; feature-gated production impl; `HashEmbedder` for tests |
| Errors | `crates/htui-store/src/embed.rs:53-54`, `vector.rs` `backend(ctx, e)` | `StoreError::Backend(format!("<what failed>: {e}"))` |
| HTTP client | `crates/htui-agent/src/install/http.rs:51-90` | `Once`-guarded `ring` provider install; `user_agent`, `connect_timeout`, `read_timeout`; non-2xx is an error naming the URL |
| Streamed hash + write | `crates/htui-agent/src/install/fetch.rs:81` | `Sha256` updated per chunk while writing |
| Off-thread load | `crates/htui/src/concepts.rs:316` (`apart`), `concepts_worker.rs:63` (`spawn_blocking`) | blocking work never on a runtime worker or the UI thread |
| Live Qdrant tests | `crates/htui-store/tests/qdrant_live.rs:23` | `throwaway()` collection, skip line when `HTUI_TEST_QDRANT_URL` is unset |
| Tests | `crates/htui-store/src/embed.rs:131-176` | `#[tokio::test]` in-module; model-needing tests `#[ignore = "…"]` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-store/tests/fixtures/bge_small_goldens.json` | CREATE | T1 | fastembed vectors, the pin for §5.6 |
| `crates/htui-store/src/embed.rs` | UPDATE | T1, T2, T4, T5 | T1 recorder test; T2 `RtenEmbedder`; T4 `identity()`; T5 drop `FastEmbedder` and the recorder |
| `crates/htui-store/src/model.rs` | CREATE | T3 | pins, cache adoption, fetch |
| `crates/htui-store/src/lib.rs` | UPDATE | T3 | `pub mod model` (cfg `local-embed`) |
| `crates/htui-store/Cargo.toml` | UPDATE | T2, T3, T5 | `rten`, `tokenizers`, `reqwest`, `rustls` under `local-embed`; drop `fastembed` |
| `Cargo.toml` | UPDATE | T2, T5 | workspace `rten`, `tokenizers`; drop `fastembed`, `ureq` |
| `Cargo.lock` | UPDATE | T2, T5 | follows the manifests |
| `crates/htui-store/src/vector.rs` | UPDATE | T4 | identity in `create_collection`/`ensure_collection` |
| `crates/htui-store/tests/qdrant_live.rs` | UPDATE | T4 | stamp, match, mismatch and dim-mismatch cases |
| `crates/htui/src/concepts.rs` | UPDATE | T3 | `open`, `open_index` through `ensure_model` + load; `clone_of::<RtenEmbedder>` |
| `crates/htui/src/concepts_worker.rs` | UPDATE | T3 | `load_model`, `Loader`, `connect` types |
| `README.md` | UPDATE | T5 | no build-time download; the first-use fetch |

Thirteen files including the lock, so C4 fires after all. Routing is unchanged: one criterion is
below the PRD threshold.

## Tasks

Serial. Each task touches `embed.rs` or the manifests that the next one needs (see the
independence check below), so there is no fan-out.

### T1: fastembed goldens (while fastembed still builds)
- **Precondition**: fastembed cannot fill an empty cache (ANA-23 §2.4). In this sandbox the cache
  `~/.cache/htui/fastembed/models--Xenova--bge-small-en-v1.5/{refs/main,snapshots/ea104dac…/}`
  was filled by hand from the pinned URLs during the fact-check, and fastembed loads from it.
- **Action**: the `record_fastembed_goldens` ignored test (D1) writes
  `tests/fixtures/bge_small_goldens.json` (`{provenance, texts[], vectors[][]}`). Record the fixture and
  commit it. Add a non-ignored unit test that checks the fixture's shape: 6 vectors × 384, each
  with norm 1 ± 1e-6.
- **Validate**: `HTUI_RECORD_GOLDENS=1 cargo test -p htui-store --features local-embed --lib embed::tests::record_fastembed_goldens -- --ignored`; `cargo test -p htui-store --lib embed`

### T2: `RtenEmbedder` (tests first)
- **Action**: first, the failing ignored test `rten_matches_fastembed_goldens` (D2), plus unit tests
  for the sub-batch splitter (D5: cap honoured, input order kept, oversize single text alone).
  Then add `RtenEmbedder::load(&ModelFiles)` and `DenseEmbedder for RtenEmbedder` (D3, D4).
  `ModelFiles` here is only `{onnx, tokenizer}` paths, a plain struct with no network. Add
  `rten = { version = "0.26", default-features = false, features = ["onnx_format"] }`,
  `rten-tensor` and `tokenizers = { version = "0.23", default-features = false, features = ["fancy-regex"] }`
  to the workspace and to `local-embed`.
- **Validate**: `cargo test -p htui-store --features local-embed --lib embed -- --include-ignored`
  (the model comes from the seeded cache, path given by the test helper)

### T3: model fetch and the three construction sites
- **Action**: tests first, against a local `TcpListener` HTTP stub, with no network. Cases: download
  writes the files; bad hash → error naming both hashes and no file left behind; a `.part` from a
  killed run is replaced; fastembed snapshot adopted without a request; already-present files
  produce zero requests; HTTP 404 → error naming the URL; unwritable cache dir → error. The URL
  base and cache root are injectable for tests only (`pub(crate)` `ModelSource`). Then add
  `model.rs` (D6) and rewire `concepts.rs`/`concepts_worker.rs` (D7).
- **Validate**: `cargo test -p htui-store --features local-embed --lib model`; `cargo test -p htui --features testkit --lib concepts`

### T4: model identity on connect
- **Action**: tests first in `qdrant_live.rs`: a new collection carries the identity; a collection
  without one is stamped; a mismatched identity refuses connect with both named; a dense size ≠
  `dim()` refuses. Then add `DenseEmbedder::identity` (signature change; every implementor listed
  by `change verify`) and D8 in `vector.rs`.
- **Validate**: `cargo test -p htui-store --features test-support --test qdrant_live` (with `HTUI_TEST_QDRANT_URL`)

### T5: remove `fastembed` (after the D10 cross-check is green)
- **Action**: run the cross `cargo check` of the embedder stack for `x86_64-pc-windows-msvc`,
  `aarch64-apple-darwin` and `x86_64-apple-darwin`. Then remove `fastembed` and `ureq` (D9), the
  recorder test and `FastEmbedder`. Update the README and comments. Confirm `cargo tree -i ort`,
  `-i onig_sys` and `-i hf-hub` print nothing. `openssl-sys` **stays**: `reqwest`'s `native-tls`
  path is still pulled by `qdrant-client` and `sentry` (fact-check). The README must not claim
  the OpenSSL headers are gone.
- **Validate**: the full gate below

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # memory: suite green is scheduling-dependent
cargo test -p htui-store --features local-embed --lib embed -- --ignored   # goldens (D2)
cargo tree -i ort; cargo tree -i onig_sys; cargo tree -i hf-hub             # each: no match
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| rten fails at run time on Windows/macOS (only a type-check is possible here) | Low | D10 cross-check; MOD-16 host verification; tract-onnx fallback is local to `RtenEmbedder` (ANA-23 §7.5) |
| A Qdrant server too old for collection metadata ignores the field | Low | A missing key is stamped, never refused, so an old server degrades to "unchecked", not broken. The compose image is `qdrant/qdrant:latest` |
| Golden test flakes across CPU paths (AVX2/AVX-512) | Low | 1e-5 per element is about 30× the measured drift (3.6e-7) |
| Two processes fetch at once into one cache | Medium | Per-process unique `.part` names plus atomic rename. The loser's rename overwrites identical verified bytes |
| HuggingFace changes URLs again | Medium | The hash pin fails loudly and names the file. A mirror base URL is a follow-up (ANA-23 §9) |
| Disk at 91% in the sandbox | Medium | ~41 G free; no extra worktrees; `cargo clean -p` if needed (memory: dev Postgres crash = disk) |

## Acceptance

- [ ] Goldens recorded from fastembed and committed before fastembed is removed
- [ ] `RtenEmbedder` matches every golden within 1e-5 per element
- [ ] Fresh cache + network: first index/search fetches, verifies and loads; no panic on any failure
- [ ] Existing fastembed cache adopted with zero downloads
- [ ] Collection identity written, stamped on old collections, mismatch refused with a clear error
- [ ] `fastembed`, `ort`, `onig_sys`, `hf-hub` 0.3 gone from the lock; no build-time download
- [ ] Validation passes; reviewer (`rust-reviewer`) findings applied or deferred with the maintainer

## Open questions (maintainer) - resolved 2026-10-01: both defaults accepted

- **OQ-1 - Windows/macOS gate.** HANDOFF says "build the embedder on Windows and macOS before
  removing fastembed". This Linux sandbox can only cross `cargo check` (no linker or runtime for
  those targets). The cross-check of `rten` + `tokenizers` (fancy-regex) already passes on all
  three targets. **Default**: accept the cross-check as this item's gate, and leave the real
  run to MOD-16 or the host after `scripts/hr collect`. **Alternative**: keep T5 (the removal) on
  the branch but have the maintainer build it on a Windows/macOS host before merge.
- **OQ-2 - Identity mismatch remedy.** **Default**: refuse with a message that tells the user to
  delete collection `htui_concepts_v2` and re-run `htui --index-items`. **Alternative**: a
  `--reindex` CLI flag that drops and rebuilds. That adds scope and is a separate item if wanted.

## Verified claims (step 3.5)

Checked 2026-10-01 against the tree at `f3c5b8b` and the pinned toolchain (Rust 1.98.1). The probes
ran in `/tmp/mod68probe`, not kept.

| Claim | Verdict | Evidence |
|---|---|---|
| `FastEmbedder` and `HashEmbedder` are the only `DenseEmbedder` implementors | ✓ | graph `implementations` of `embed.rs::DenseEmbedder`: 2 (`embed.rs:62`, `:118`) |
| `FastEmbedder` is built at three sites: `concepts.rs` `open` and `open_index`, `concepts_worker.rs` `load_model` | ✓ | graph usages of `FastEmbedder`: `concepts.rs:99`, `:316`, `concepts_worker.rs:65`; type-only at `:158`, `:197` |
| `open()` calls `FastEmbedder::new()` inline on the runtime | ✓ | `concepts.rs:99` |
| `apart()` runs work on a plain `std::thread` with no runtime, so async fetch cannot run inside it | ✓ | `concepts.rs:338-348` |
| Stale "`FastEmbedder` is not `Clone`" comment | ✓ | `concepts.rs:313`; it derives `Clone` since MOD-64 (`embed.rs:26`) |
| The requirement chunk is 256 | ✓ | `vector_sync.rs:30` `UPSERT_BATCH = 256`, `:168` |
| `htui-store` cannot depend on `htui-agent` | ✗ amended | No edge either way; the reason to mirror is that the client is `pub(crate)` and an edge would be new (D6 reworded) |
| `reqwest` + `rustls` (ring) are workspace deps; `install/http.rs` installs the provider once | ✓ | `Cargo.toml:91-94`; `http.rs:51-55` |
| Workspace `ureq` entry has no direct user | ✓ | no `ureq` in any `crates/*/Cargo.toml`; `cargo tree -i ureq@2.12.1`: only `hf-hub` 0.3.2 ← `fastembed` |
| Removing fastembed drops `openssl-sys` | ✗ amended | `cargo tree -i openssl-sys`: also `native-tls` ← `hyper-tls`/`reqwest` ← `qdrant-client`, `sentry`. T5 now says it stays |
| README mentions the build-time ONNX Runtime download | ✓ | `README.md:54`, `:385`, `:498-499` |
| Pinned URL serves `tokenizer.json` via a relative `307` and `model.onnx` via `302` to a CDN | ✓ | `curl`: `307 /api/resolve-cache/…`; `302`, then `200`, `content-length 133093490`, `x-linked-etag 828e1496…cf35` |
| sha256 pins | ✓ | `model.onnx` `828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35`; `tokenizer.json` `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` (sha256sum of fresh downloads) |
| rten 0.26 loads the file; inputs `input_ids`/`attention_mask`/`token_type_ids` accept `NdTensor<i32,2>`; single output `last_hidden_state` `[B,L,384]` | ✓ | probe run: names printed, shape `[2, 15, 384]`, normalised 384-vectors |
| `RunOptions` is `#[non_exhaustive]` with a `pub thread_pool: Option<Arc<ThreadPool>>`; `ThreadPool::with_num_threads` | ✓ | `rten-0.26.0/src/graph.rs:109-134`; `threading.rs:30`; probe compiles |
| `rten::Model` and `tokenizers::Tokenizer` are `Send + Sync` (shareable behind `Arc` across `spawn_blocking`) | ✓ | probe static assert compiles |
| tokenizers 0.23 `with_truncation(&mut self)` returns `Result`, `with_padding` returns `&mut Self` | ✓ | `tokenizers-0.23.2/src/tokenizer/mod.rs:661`, `:687`; probe |
| The embedder stack needs no C compiler | ✓ | probe `cargo tree -i cc`: no match (`esaxx-rs` present without `cc`) |
| The embedder stack builds for Windows and macOS (D10, OQ-1) | ✓ (type-check) | `cargo check --release` green for `x86_64-pc-windows-msvc`, `aarch64-apple-darwin` and `x86_64-apple-darwin` |
| Qdrant 1.19.1 stores, returns and updates collection `metadata` | ✓ | REST probe on `localhost:6333`: create with `metadata`, GET `config.metadata`, PATCH replaced it |
| `qdrant-client` 1.19.0 exposes `metadata` on create, update and `CollectionConfig` | ✓ | `create_collection_builder.rs:146`, `update_collection_builder.rs:103`, `qdrant.rs:1200` (`CollectionConfig.metadata`); `collection_info` at `qdrant_client/collection.rs:94` |
| `ensure_collection` checks nothing about an existing collection's size or model | ✓ | `vector.rs:605-630` |
| A deliberate-rebuild drop already exists | ✓ | `vector.rs:596` (`delete_collection`, "for tests and for a deliberate rebuild") |
| fastembed 3.14.1 loads from a hand-filled HF cache (T1 precondition) | ✓ | seeded `refs/main` + `snapshots/ea104dac…/`; `cargo test -p htui-store --features local-embed --lib embed::tests::fast_embedder_returns_384_dims -- --ignored`: ok |
| T1-T5 independence | serial by design | `embed.rs` is touched by T1, T2, T4, T5; manifests by T2, T3, T5. Non-empty intersections, so no fan-out |
