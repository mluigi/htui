# Blueprint: MOD-68 - `rten` embedder, pinned weight fetch, collection identity

**Plan**: `.claude/plans/mod-68-rten-embedder.plan.md` (CONFIRMED 2026-10-01). This blueprint is the
plan's second fact-check and its implementation detail. Where it departs from the plan, the
departure is listed in §7 (Amendments) and marked **[A-n]** where it applies.
**Branch**: `hr/MOD-68` (sandbox, `HR_SANDBOX=1`). Serial tasks T1-T5, no fan-out, no extra worktree
(disk at 90%).
**Toolchain**: 1.98.1 (`rust-toolchain.toml`), clippy `-D warnings` over `[workspace.lints]`.

## 0. Ground truth this blueprint was checked against

| Fact | Where | Consequence |
|---|---|---|
| `[workspace.lints.rust]`: `unsafe_code = forbid`, `missing_debug_implementations = warn`, `unused_qualifications = warn`; clippy `all = warn` (no pedantic) | `Cargo.toml` | Every `pub` type needs `Debug`; no `std::sync::Arc` spelled out once `Arc` is imported; `as i32` casts are fine (pedantic is off) |
| `htui-store` has `#![warn(missing_docs)]` | `crates/htui-store/src/lib.rs:11` | Every `pub` item, field and const in `model.rs`/`embed.rs` needs a doc comment (`pub(crate)` does not) |
| Root `clippy.toml` holds only `msrv` (htui-agent has its own) | `clippy.toml` | No disallowed-methods list applies to `htui-store` or `htui`: `std::thread::spawn` in a test stub is allowed |
| `htui` always enables `htui-store/local-embed`; `htui-worker` never does | `crates/htui/Cargo.toml:37`, `crates/htui-worker/Cargo.toml` | No `cfg` needed in `concepts.rs`/`concepts_worker.rs`; `htui-worker` is unaffected (its `tests/deps.rs` only bans ratatui/crossterm) |
| `qdrant-client` 1.19.0's **default** features turn on `reqwest/rustls`, which is `__rustls-aws-lc-rs` | `qdrant-client-1.19.0/Cargo.toml`, `reqwest-0.13.5/Cargo.toml` | One `reqwest` 0.13.5 in the lock; in `htui-store` it is already built with rustls + aws-lc-rs. `ClientBuilder::build` falls back to aws-lc-rs when no default provider is installed (`reqwest async_impl/client.rs:721`, `:2510`): **no panic** in `htui-store` even without an install. We still install `ring` once (mirrors `install/http.rs`, keeps one provider process-wide, survives a future trim of qdrant's features) |
| `MetadataWrapper: From<HashMap<String, serde_json::Value>>` only; `impl From<qdrant::Value> for serde_json::Value` exists (feature `serde`, on by default) | `qdrant-client-1.19.0/src/grpc_conversions/metadata.rs:9`, `extensions.rs:192` | Write metadata as `HashMap<String, serde_json::Value>`; read back `CollectionConfig.metadata` and convert each `Value` to JSON |
| `UpdateCollectionBuilder::new(name)`; its `metadata` is **merged** with what is stored | `builder_ext.rs:79`, `update_collection_builder.rs:102` | Stamping writes only the `embedder` key and touches nothing else |
| `collection_info(impl Into<GetCollectionInfoRequest>)` (any `Into<String>`) → `GetCollectionInfoResponse { result: Option<CollectionInfo> }`; dense size at `config.params.vectors_config.config` = `vectors_config::Config::ParamsMap(VectorParamsMap { map })` → `map["dense"].size: u64` | `qdrant.rs:415-440`, `:513`, `:1179` | §2.6 `dense_size` |
| rten 0.26: `Model::load_file` picks the loader by **extension** (`FileType::from_path`); `NodeId: Copy`; `.view()`/`.shape()` need `use rten_tensor::prelude::*` | `rten-0.26.0/src/model.rs:753`, `graph/node_id.rs:8`; compile error seen without the prelude | Final file names must end in `.onnx`; `.part` files are never loaded |
| **Debug-profile speed** (measured 2026-10-01 on this box, 4 threads): one 512-token text = **10.6 s**, six = **66 s** at `opt-level 0`; with `opt-level = 3` on the `rten*` crates: **90 ms** / **608 ms**. `sha256` of the 133 MB `model.onnx`: **2.5 s** at `opt-level 0`, **69 ms** with `sha2` at `opt-level 3` | probe, `/tmp/mod68probe/examples/long.rs`, `hash.rs` (gone since the sandbox was recreated) | Per-package `[profile.dev.package.*] opt-level = 3` is part of T2 **[A-1]**. Without it the ignored golden gate takes minutes and a `cargo run` index is ~100x slower than with fastembed (ORT is prebuilt and optimised) |
| tokenizers 0.23.2 `AddedToken::default()` has `normalized: true`; fastembed 3.14.1 re-adds the five special tokens from `special_tokens_map.json` with `special: true, ..Default::default()` | `tokenizers-0.23.2/src/tokenizer/added_vocabulary.rs:78`; `fastembed-3.14.1/src/common.rs:108-127` | `RtenEmbedder::load` repeats that call from constants so tokenisation matches fastembed even for a text that contains a literal `[SEP]` |
| fastembed pads `BatchLongest` and truncates at `min(512, model_max_length)` with default `TruncationParams`; normalises `v / (norm + 1e-12)` with an f32 sequential sum | `fastembed-3.14.1/src/common.rs:94-137` | §2.4 mirrors it exactly, padding per sub-batch by hand **[A-2]** |
| Seeded fastembed cache: `refs/main` = `ea104dac…`, `snapshots/ea104dac…/{onnx/model.onnx (133,093,490 B), tokenizer.json (711,396 B), config.json, special_tokens_map.json, tokenizer_config.json}`, regular files (a real fastembed cache has symlinks into `blobs/`) | `ls ~/.cache/htui/fastembed/...` | T1 records from it; T3's adoption test must also cover a symlinked file |
| Only `x86_64-unknown-linux-gnu` is installed in rustup after the sandbox was recreated | `rustup target list --installed` | T5's D10 cross-check first runs `rustup target add` for three targets (network) |
| `DenseEmbedder` implementors: `FastEmbedder` (`embed.rs:62`), `HashEmbedder` (`:118`). Construction / type sites: `concepts.rs:22,90,99,313-317,563`; `concepts_worker.rs:18,58-66,81,137,157-160,178,189-205`. `HashEmbedder` is also used by `htui-store/tests/qdrant_live.rs:31` and `htui/tests/qdrant_worker.rs:30`, on throwaway collections only | graph text search | §2.6 and §2.7 enumerate every edit |
| Loopback HTTP stubs already exist (`htui-agent/tests/install.rs` `Fixture`, tokio `TcpListener`); `htui-store`'s dev `tokio` has no `net` feature | `crates/htui-store/Cargo.toml` | T3's stub uses `std::net::TcpListener` on a `std::thread`: **no new dev-dependency and no feature change** |
| "cannot reach Qdrant" is the prefix all three sites put on a `QdrantStore::connect` error; the overlay tests assert only fake strings | `concepts.rs:102,319`, `concepts_worker.rs:205`, `ui/overlay/concepts_search.rs:890-1049` | Prefixes stay as they are (§2.7); the identity error carries its own remedy after the prefix |

## 1. Design decisions (resolved here, binding for the implementer)

- **B1 - `model.rs` is born in T2, not T3.** T2 needs `ModelFiles` and the tokenizer constants;
  T3 adds the fetch to the same file. `pub mod model` is `#[cfg(feature = "local-embed")]` from T2 on.
- **B2 - Golden texts live in the fixture.** The recorder builds them once; every later test reads
  `texts` from the fixture, so there is one source. The fixture is read at run time from
  `concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bge_small_goldens.json")`, which resolves
  in a lib unit test (`CARGO_MANIFEST_DIR` is set for every target of the package). Not
  `include_str!`: T1's shape test would not compile before the recorder has run.
- **B3 - The golden test finds the model through `model::ensure_model()`** from T3 on. In T2 (no
  fetch yet) a test-only helper points at the seeded fastembed snapshot; T3 swaps that helper's
  body for `ensure_model().await`, so the gate also proves adoption end to end, with zero downloads
  on a box that has the fastembed cache.
- **B4 - Padding is done per sub-batch by hand** (pad id 0, mask 0, type 0, right side), which is
  byte-for-byte what `PaddingStrategy::BatchLongest` produces for that sub-batch. The tokenizer
  keeps truncation only. Tokenising once with padding on would pad every sub-batch to the call's
  global longest and defeat D5 **[A-2]**.
- **B5 - The splitter is a pure function** over token lengths, `#[cfg(any(test, feature =
  "local-embed"))]`, so its tests run in the default suite with no model and no feature, and a
  build without the feature has no dead code.
- **B6 - Fetch is per file.** For each pinned file: keep it if it verifies, else adopt it from a
  fastembed snapshot, else download it. A missing or corrupt `tokenizer.json` does not re-download
  133 MB.
- **B7 - Unique `.part` names plus a stale sweep [A-3].** `<local>.<pid>.<seq>.part`. A `.part`
  whose mtime is older than 10 minutes is removed before downloading; a live download writes at
  least every 60 s (the read timeout), so a fresh `.part` belongs to another live process and is
  left alone. This replaces the plan's "a `.part` from a killed run is replaced", which unique
  names cannot do on their own.
- **B8 - The identity check reads the dense size first.** A collection whose `dense` size differs
  from `dim()` is refused even when it carries no `embedder` key, so it is never stamped.
- **B9 - `open_index` keeps a loaded model across retries [A-4].** The embedder is `Clone`; a retry
  after an unreachable Qdrant reconnects without re-hashing and re-loading 133 MB. The stale
  comment D9 names is rewritten to say so.
- **B10 - One HTTP client, two timeouts** (`connect_timeout` 15 s, `read_timeout` 60 s, no total
  timeout), built lazily on the first download only, after the `ring` install. A run that finds
  its files never builds a client or installs a provider.
- **B11 - Hashing and copying run on `spawn_blocking`.** Only the HTTP body is async. A 133 MB hash
  is ~70 ms with the `sha2` profile override, so `htui worker`'s exit is not held in any way that
  matters (the reason `apart()` exists is an unbounded wait, not this).

## 2. Component designs

### 2.1 Module layout and gating

```
crates/htui-store/src/lib.rs     #[cfg(feature = "local-embed")] pub mod model;   (T2)
crates/htui-store/src/model.rs   pins, tokenizer constants, ModelFiles, identity(),   (T2)
                                 ModelSource + ensure_model() + the fetch           (T3)
crates/htui-store/src/embed.rs   DenseEmbedder (+ identity, T4), EmbedderIdentity (T4),
                                 sub_batches (T2), RtenEmbedder (T2, cfg local-embed),
                                 FastEmbedder (until T5), HashEmbedder
crates/htui-store/tests/fixtures/bge_small_goldens.json   (T1)
```

`lib.rs` doc line for the module: `/// The pinned BGE-small model files: where they are cached and how they are fetched (MOD-68).`

### 2.2 Golden fixture and recorder (T1; D1, D2)

File `crates/htui-store/tests/fixtures/bge_small_goldens.json`, written with
`serde_json::to_string_pretty` plus a trailing newline:

```json
{
  "provenance": {
    "recorder": "fastembed 3.14.1 (ONNX Runtime via ort), FastEmbedder::embed, one call",
    "model": "Xenova/bge-small-en-v1.5",
    "revision": "ea104dacec62c0de699686887e3f920caeb4f3e3",
    "onnx_sha256": "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35",
    "tokenizer_sha256": "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    "recorded": "2026-10-01",
    "tolerance": "1e-5 per element, cosine >= 1 - 1e-9 (ANA-23 §8.1)"
  },
  "texts": ["...6 strings..."],
  "vectors": [[384 f32], "...6 rows..."]
}
```

f32 values go through serde_json's shortest round-trip form and are read back into `Vec<f32>`, so
the fixture is exact. `recorded` is `chrono::Utc::now().date_naive().to_string()` at record time.

Types, in `embed::tests` (test-only, so no docs are required):

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Goldens { provenance: Provenance, texts: Vec<String>, vectors: Vec<Vec<f32>> }
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Provenance { recorder: String, model: String, revision: String, onnx_sha256: String,
                    tokenizer_sha256: String, recorded: String, tolerance: String }
fn goldens_path() -> std::path::PathBuf   // env!("CARGO_MANIFEST_DIR")/tests/fixtures/bge_small_goldens.json
fn read_goldens() -> Goldens              // std::fs::read_to_string + serde_json::from_str, expect()
```

The six texts (recorder only; `fn golden_texts() -> Vec<String>`, deleted in T5):

1. `"hello world"` (short)
2. `"MOD-68 Replace fastembed/ort with an rten embedder and a pinned weight fetch. The dense embedder moves to rten plus tokenizers, so a fresh install can load the model and the stored Qdrant vectors stay valid."` (typical item point)
3. `"The store worker serialises every request and answers on its own task. ".repeat(60)` (> 512 tokens; the probe measured it truncating to exactly 512)
4. `"検索とベクトル: Qdrant 🦀 naïve café — 数据库 ✅"` (CJK, emoji, accents, dash)
5. `""` (empty)
6. `"MOD-34"` (the exact key)

Recorder:

```rust
#[cfg(feature = "local-embed")]
#[tokio::test]
#[ignore = "records the goldens from fastembed; needs the BGE model and HTUI_RECORD_GOLDENS=1"]
async fn record_fastembed_goldens()
```
Returns after an `eprintln!` unless `std::env::var("HTUI_RECORD_GOLDENS").as_deref() == Ok("1")`.
Else `FastEmbedder::new()`, one `embed(golden_texts())`, asserts 6 × `DENSE_DIM`, writes the file
(`create_dir_all` of `tests/fixtures` first).

Shape test (no feature, never ignored, runs in the default suite):

```rust
#[test]
fn goldens_fixture_is_six_unit_vectors_of_384()
```
asserts: `texts.len() == 6 == vectors.len()`; every row has `DENSE_DIM` elements; every row's
f64 norm is within `1e-6` of 1; `texts[4] == ""`, `texts[5] == "MOD-34"`; `provenance.revision`
is the pinned commit; `provenance.onnx_sha256` is the pinned hash.

### 2.3 Sub-batch splitter (T2; D5)

```rust
/// Most padded tokens (rows × longest row) one forward pass may hold: 16 rows of 512.
#[cfg(any(test, feature = "local-embed"))]
pub(crate) const MAX_BATCH_TOKENS: usize = 16 * 512;

/// Splits texts of token lengths `lens`, in input order, into consecutive ranges whose
/// `rows × longest` stays within `cap`. A single text longer than `cap` is a range of its own.
#[cfg(any(test, feature = "local-embed"))]
pub(crate) fn sub_batches(lens: &[usize], cap: usize) -> Vec<std::ops::Range<usize>>
```

Greedy: keep `start` and `longest`; for each `i`, `widest = longest.max(lens[i])`; if `i > start`
and `(i - start + 1) * widest > cap`, close `start..i` and restart at `i` with `longest =
lens[i]`; else `longest = widest`. Close the last range. `lens` empty → `vec![]`. `cap == 0` is
treated as 1 (`cap.max(1)`) so the loop cannot misbehave.

Tests (in `embed::tests`, no feature):

| Test | Asserts |
|---|---|
| `sub_batches_of_nothing_is_nothing` | `sub_batches(&[], 8192)` is empty |
| `sub_batches_that_fit_are_one_range` | `[3, 9, 4]`, cap 100 → `[0..3]` |
| `sub_batches_honour_the_cap` | for several fixed inputs (incl. `vec![20; 255]` + `[512]`, ANA-23 §9's shape): every range has `len * max ≤ cap` **or** `len == 1` |
| `sub_batches_keep_input_order` | ranges are contiguous, start at 0, end at `lens.len()`, none empty |
| `an_oversize_text_is_alone` | `[10, 600, 10]`, cap 512 → `[0..1, 1..2, 2..3]` |
| `a_long_text_does_not_pad_the_whole_chunk` | `[20; 255] ++ [512]`, cap 8192 → the range holding index 255 has `len ≤ 16` |

### 2.4 `RtenEmbedder` (T2; D3, D4)

In `embed.rs`, all `#[cfg(feature = "local-embed")]`:

```rust
/// BGE-small-en-v1.5 on `rten`, run on the blocking pool. Cheap to clone: the model, the
/// tokenizer and the thread pool are shared (MOD-64 D238).
#[derive(Clone)]
pub struct RtenEmbedder { inner: Arc<Rten> }

struct Rten {                       // private: no Debug/doc lint applies
    model: rten::Model,
    tokenizer: tokenizers::Tokenizer,
    pool: Arc<rten::ThreadPool>,
    input_ids: rten::NodeId,
    attention_mask: rten::NodeId,
    token_type_ids: rten::NodeId,
    output: rten::NodeId,
}

impl fmt::Debug for RtenEmbedder { /* debug_struct("RtenEmbedder").finish_non_exhaustive() */ }

impl RtenEmbedder {
    /// Loads the model and tokenizer from `files`. CPU only; never touches the network.
    /// # Errors
    /// `StoreError::Backend` naming the file that would not load.
    pub fn load(files: &ModelFiles) -> Result<Self, StoreError>;

    /// One vector per text, in input order, at most `cap` padded tokens per forward pass.
    fn embed_blocking(&self, texts: &[String], cap: usize) -> Result<Vec<Vec<f32>>, StoreError>;
}

impl DenseEmbedder for RtenEmbedder {
    fn dim(&self) -> usize { DENSE_DIM }
    // T4: fn identity(&self) -> EmbedderIdentity { crate::model::identity() }
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.embed_blocking(&texts, MAX_BATCH_TOKENS))
            .await
            .map_err(|e| StoreError::Backend(format!("embedding task failed: {e}")))?
    }
}
```

`load`, in order (tokenizer first: cheap, and it fails fast on a wrong directory):

1. `Tokenizer::from_file(&files.tokenizer)` → on error
   `embedding model: cannot load {path}: {e}` (path = `files.tokenizer.display()`).
2. `tokenizer.with_truncation(Some(TruncationParams { max_length: model::MAX_TOKENS, ..Default::default() }))`
   → same error shape. **No `with_padding`** (B4).
3. `tokenizer.add_special_tokens(model::SPECIAL_TOKENS.map(|t| AddedToken { content: t.into(), special: true, ..Default::default() }))`
   — fastembed's own call, from constants. Check the 0.23.2 signature when writing it (ANA-23 §7.1:
   takes an iterator and returns `Result`); map an `Err` to the same error shape.
4. `Model::load_file(&files.onnx)` → `embedding model: cannot load {path}: {e}`.
5. Resolve `node_id("input_ids")`, `"attention_mask"`, `"token_type_ids"`; `output_ids().first()`
   (empty → `embedding model: {path} has no output`).
6. `ThreadPool::with_num_threads(std::thread::available_parallelism().map_or(1, NonZero::get))`
   into an `Arc`, created once here.

`embed_blocking`:

1. `texts.is_empty()` → `Ok(vec![])` before touching the model.
2. `encode_batch(texts.iter().map(String::as_str).collect::<Vec<_>>(), true)` → error
   `embedding failed: {e}`.
3. `lens = encodings.iter().map(|e| e.get_ids().len())`; for each range of `sub_batches(&lens, cap)`:
   `rows = range.len()`, `longest = max lens`; three zeroed `Vec<i32>` of `rows * longest`; copy
   each encoding's `get_ids()`, `get_attention_mask()`, `get_type_ids()` (`as i32`) into its row
   prefix; `NdTensor::from_data([rows, longest], v)`.
4. `let mut opts = RunOptions::default(); opts.thread_pool = Some(Arc::clone(&self.inner.pool));`
   `model.run(vec![(input_ids, ids.view().into()), (attention_mask, ...), (token_type_ids, ...)], &[output], Some(opts))`
   → `embedding failed: {e}`.
5. Take the single output, `try_into::<NdTensor<f32, 3>>()` → `embedding failed: {e}`. Check
   `shape()[2] == DENSE_DIM` else `embedding failed: the model answered {n}-wide vectors, expected 384`.
6. Row `i`: `h.slice((i, 0))` (the CLS token), collected to `Vec<f32>`, then fastembed's
   normalisation exactly: `let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt(); v / (norm + 1e-12)`.
7. Push in range order (ranges are in input order, so the output is too).

Imports: `use rten_tensor::prelude::*;` and `use rten_tensor::NdTensor;` inside the gated block
(without the prelude `.view()` and `.shape()` do not resolve).

T2 tests (in `embed::tests`):

| Test | Gate | Asserts |
|---|---|---|
| `rten_matches_fastembed_goldens` | `local-embed`, `#[ignore = "needs the BGE model (133 MB)"]`, `#[tokio::test]` | one `embed(texts)` call; per element `\|a-b\| ≤ 1e-5`; f64 cosine `≥ 1 - 1e-9`; message names the text index and the worst element |
| `rten_matches_goldens_one_text_per_pass` | same | `embed_blocking(&texts, model::MAX_TOKENS)` (cap 512 forces one text per pass) on `spawn_blocking`; same tolerances. Proves the splitter and the per-sub-batch padding on the real model |
| `rten_embeds_nothing_as_nothing` | same | `embed(vec![])` is `Ok(vec![])` |
| `rten_load_names_a_missing_file` | `local-embed`, **not** ignored | `RtenEmbedder::load(&ModelFiles { onnx: tmp/"absent.onnx", tokenizer: tmp/"absent.json" })` is `Err`, text contains `absent.json` |

Helper (T2 body, replaced in T3):

```rust
#[cfg(feature = "local-embed")]
async fn golden_model() -> crate::model::ModelFiles {
    // T2: the seeded fastembed snapshot. T3: crate::model::ensure_model().await.expect("the model is fetched or adopted")
    let snap = dirs::cache_dir().expect("a cache dir").join("htui/fastembed/models--Xenova--bge-small-en-v1.5/snapshots")
        .join(crate::model::REVISION);
    crate::model::ModelFiles { onnx: snap.join("onnx/model.onnx"), tokenizer: snap.join("tokenizer.json") }
}
```

### 2.5 `model.rs` (T2 constants, T3 fetch; D6)

Public surface (all documented, `missing_docs`):

```rust
pub const REPO: &str = "Xenova/bge-small-en-v1.5";
pub const REVISION: &str = "ea104dacec62c0de699686887e3f920caeb4f3e3";
pub const ONNX_SHA256: &str = "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35";
pub const ONNX_BYTES: u64 = 133_093_490;
pub const TOKENIZER_SHA256: &str = "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";
pub const TOKENIZER_BYTES: u64 = 711_396;
/// fastembed read these from config.json, tokenizer_config.json and special_tokens_map.json.
pub const MAX_TOKENS: usize = 512;
pub const PAD_ID: u32 = 0;                       // "[PAD]"; used by embed_blocking's zero fill (document it there)
pub const SPECIAL_TOKENS: [&str; 5] = ["[CLS]", "[MASK]", "[PAD]", "[SEP]", "[UNK]"];
pub const POOLING: &str = "cls";
pub const NORMALISATION: &str = "l2";

/// The two files the embedder loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles { pub onnx: PathBuf, pub tokenizer: PathBuf }
impl ModelFiles { pub fn in_dir(dir: &Path) -> Self }   // dir/model.onnx, dir/tokenizer.json

/// The identity recorded in the Qdrant collection (T4; D8).
pub fn identity() -> EmbedderIdentity;                  // REPO, REVISION, ONNX_SHA256, DENSE_DIM, POOLING, NORMALISATION

/// The model files, verified: cached, adopted from fastembed's cache, or downloaded (T3).
/// # Errors
/// `StoreError::Backend` naming the file and URL; never panics.
pub async fn ensure_model() -> Result<ModelFiles, StoreError> { ModelSource::production()?.ensure().await }
```

Test seam (`pub(crate)`, T3):

```rust
#[derive(Debug, Clone)]
pub(crate) struct Pin { pub(crate) remote: &'static str, pub(crate) local: &'static str,
                        pub(crate) sha256: String, pub(crate) bytes: u64 }
pub(crate) fn pins() -> Vec<Pin>;  // [("onnx/model.onnx","model.onnx",ONNX_SHA256,ONNX_BYTES), ("tokenizer.json","tokenizer.json",TOKENIZER_SHA256,TOKENIZER_BYTES)]

#[derive(Debug, Clone)]
pub(crate) struct ModelSource { base_url: String, cache_root: PathBuf, pins: Vec<Pin> }
impl ModelSource {
    pub(crate) fn production() -> Result<Self, StoreError>;   // "https://huggingface.co", dirs::cache_dir()/"htui", pins()
    pub(crate) fn new(base_url: impl Into<String>, cache_root: impl Into<PathBuf>, pins: Vec<Pin>) -> Self;
    pub(crate) fn model_dir(&self) -> PathBuf;                // cache_root/model/bge-small-en-v1.5-ea104dac
    pub(crate) fn url(&self, pin: &Pin) -> String;            // {base}/{REPO}/resolve/{REVISION}/{pin.remote}
    pub(crate) async fn ensure(&self) -> Result<ModelFiles, StoreError>;
}
```

Tests keep the real `remote`/`local` names and replace only `sha256`/`bytes` with those of small
fake bodies, so `ModelFiles::in_dir` and the URLs are the production ones.

`ensure`, in order (blocking steps on `spawn_blocking`; a `JoinError` is
`embedding model: the file check stopped: {e}`):

1. `create_dir_all(model_dir)` → `embedding model: cannot create {dir}: {e}`.
2. Sweep: every `*.part` in `model_dir` with mtime older than `STALE_PART = 10 min` is removed
   (errors ignored).
3. For each pin, `dest = model_dir/pin.local`:
   a. `verify(dest, pin)`: missing → false; read in 64 KiB chunks through `Sha256`; equal → keep,
      next pin. Unequal → `tracing::warn!` and fall through.
   b. Adopt: snapshot dirs = `cache_root/fastembed/models--Xenova--bge-small-en-v1.5/snapshots/*`,
      the one named `REVISION` first, the rest sorted. For each with `snap/pin.remote` present:
      copy it **through the hasher** into a fresh `.part` (`std::fs::File::open` follows symlinks;
      never `rename` or `hard_link` the source), `sync_all`, compare, `rename(part, dest)` on a
      match, else remove the `.part` and `warn!`. First match wins. The source is never modified.
   c. Download (async), below.
4. `Ok(ModelFiles::in_dir(&model_dir))`.

Download of one pin (the client is built once per `ensure`, on first need, B10):

```
install_crypto_provider();                       // static Once + rustls::crypto::ring::default_provider().install_default(), Err ignored
client = reqwest::Client::builder().user_agent(concat!("htui/", env!("CARGO_PKG_VERSION")))
         .connect_timeout(15 s).read_timeout(60 s).build()       // Err → "embedding model: cannot build the HTTP client: {e}"
resp   = client.get(&url).send().await                           // Err → E2
!resp.status().is_success()                                      // → E1 (redirects, incl. HF's relative 307, are followed by reqwest's default policy)
file   = tokio::fs::File::create(&part)                          // Err → E5
loop resp.chunk().await: done + len > pin.bytes → E4; hasher.update; file.write_all   // chunk Err → E2, write Err → E5
file.sync_all(); drop(file)                                      // drop BEFORE remove/rename (Windows)
any failure above → remove_file(part), return the error
hex(hasher) != pin.sha256 → remove_file(part), E3
rename(part, dest): Err and verify(dest) now true → remove part, Ok (another process won; Windows "in use")
                    Err otherwise → E5 with dest
```

`.part` name: `dest.with_file_name(format!("{}.{}.{}.part", pin.local, std::process::id(), PART_SEQ.fetch_add(1, Relaxed)))`
with `static PART_SEQ: AtomicU64`.

Exact error texts (all `StoreError::Backend`):

| Id | Text |
|---|---|
| E1 | `embedding model: cannot download {local}: {url} answered HTTP {status_u16}` |
| E2 | `embedding model: cannot download {local} from {url}: {e}` |
| E3 | `embedding model: {local} from {url} has sha256 {computed}, expected {pinned}` |
| E4 | `embedding model: {local} from {url} is larger than the expected {bytes} bytes` |
| E5 | `embedding model: cannot write {path}: {e}` |
| E6 | `embedding model: no user cache directory on this system` (production, `dirs::cache_dir()` is `None`) |

`hex()` is a private copy of `install/fetch.rs:184` (`{byte:02x}`); there is none in `htui-store`.

T3 test stub (in `model::tests`, std only):

```rust
/// A loopback HTTP/1.1 responder on a std thread: scripted routes, every request path recorded.
struct Stub { addr: SocketAddr, seen: Arc<Mutex<Vec<String>>> }
enum Answer { Body(u16, Vec<u8>), Redirect(u16, &'static str) }
impl Stub {
    fn start(routes: Vec<(String, Answer)>) -> Self;   // TcpListener::bind("127.0.0.1:0"); detached std::thread::spawn over incoming()
    fn base(&self) -> String;                          // "http://127.0.0.1:<port>"
    fn seen(&self) -> Vec<String>;
}
```
Per connection: read header lines with a `BufReader` until the empty line; record the path of
`GET <path> HTTP/1.1`; answer `HTTP/1.1 {code} X\r\nContent-Length: {n}\r\nConnection: close\r\n`
(+ `Location: {to}\r\n` for a redirect) `\r\n{body}`; an unknown path answers 404 with an empty
body. Never hold the `Mutex` while writing. `#[tokio::test]` (current-thread) is safe because the
stub runs on its own thread. Helpers: `fn fake_pins(onnx: &[u8], tok: &[u8]) -> Vec<Pin>` (sha256
and length of each body), `fn source(stub: &Stub, root: &Path, pins) -> ModelSource`,
`fn path_of(local) -> String` = `/Xenova/bge-small-en-v1.5/resolve/<REVISION>/<remote>`.

T3 `model::tests` (run with `--features local-embed`; each case owns a `tempfile::tempdir()` as
`cache_root`, so the user's cache is never touched):

| Test | Asserts |
|---|---|
| `downloads_both_files_into_the_model_dir` | `ensure()` → `ModelFiles::in_dir(root/model/bge-small-en-v1.5-ea104dac)`; both files hold the stub's bytes; `seen()` is exactly the two production paths, model first; no `*.part` left |
| `follows_a_relative_redirect` | `tokenizer.json` answers `307 Location: /api/resolve-cache/tok`, that path answers 200; the file is written (pins HF's 2026 redirect shape, ANA-23 §2.4) |
| `a_hash_mismatch_names_both_hashes_and_leaves_no_file` | stub serves other bytes of the pinned length; `Err` text contains the URL, the computed and the pinned hex; `dest` absent; no `*.part` |
| `a_body_longer_than_the_pin_is_refused` | body one byte longer than `pin.bytes`; text contains `larger than the expected`; no file, no `*.part` |
| `http_404_names_the_url` | no route; text contains the URL and `HTTP 404` |
| `an_unreachable_host_is_an_error_not_a_panic` | base = a port that was bound then dropped; `Err` containing the URL |
| `present_files_make_no_request` | both files pre-written with the right bytes; `Ok`; `seen()` empty |
| `a_corrupt_present_file_is_replaced` | `model.onnx` pre-written with wrong bytes; after `ensure()` it holds the right ones; `seen()` names only the model path |
| `a_fastembed_snapshot_is_adopted_without_a_request` | `root/fastembed/models--Xenova--bge-small-en-v1.5/snapshots/<REVISION>/{onnx/model.onnx, tokenizer.json}`, on unix the model one a **symlink** into `../../blobs/x` (`std::os::unix::fs::symlink`, safe API); `seen()` empty; `dest` is a regular file (`symlink_metadata().file_type().is_file()`); the snapshot files still exist with their bytes |
| `a_snapshot_with_wrong_hashes_is_ignored` | snapshot holds wrong bytes; files are downloaded; snapshot unchanged |
| `a_stale_part_is_swept_and_a_fresh_one_left` | plant `model.onnx.1.0.part` with `File::set_modified(now - 1 h)` and `tokenizer.json.2.0.part` fresh; after `ensure()` the first is gone, the second is still there |
| `an_unwritable_cache_root_is_an_error` | `cache_root` is a **regular file**; text contains `cannot create`. (Not `chmod`: the sandbox may run as root, which ignores modes) |
| `ensure_model_futures_are_send` | `fn send<T: Send>(_: T) {}` applied to `ensure_model()` and to `ModelSource::new(..).ensure()` (never awaited). Guards `BoxFuture` in `concepts_worker` and `tokio::spawn` in `spawn_index_job` |
| `production_source_is_the_pinned_commit` | `pins()` carry the four constants; `ModelSource::new("https://huggingface.co", root, pins()).url(&pins()[1])` equals `https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer.json` |

### 2.6 Model identity on connect (T4; D8)

`embed.rs` (not gated):

```rust
/// Which model made a collection's dense vectors (MOD-68 D8), as stored under the `embedder` key
/// of the Qdrant collection's metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EmbedderIdentity {
    pub model: String,          // "Xenova/bge-small-en-v1.5" | "hash"
    pub revision: String,       // full commit | ""
    pub onnx_sha256: String,    // pinned model.onnx sha256 | ""
    pub dim: usize,
    pub pooling: String,        // "cls" | "none"
    pub normalisation: String,  // "l2"
}
impl EmbedderIdentity {
    /// `HashEmbedder`'s: `hash/<dim>`.
    #[must_use] pub fn hash(dim: usize) -> Self;   // model "hash", revision "", onnx_sha256 "", pooling "none", normalisation "l2"
}
impl fmt::Display for EmbedderIdentity {
    // onnx_sha256 empty  -> "{model}/{dim}"                                   e.g. "hash/384"
    // otherwise          -> "{model}@{rev8} (model.onnx {sha12}, {dim}-d, {pooling} pooling, {normalisation})"
    //   rev8/sha12 via .get(..8)/.get(..12).unwrap_or(&full): stored metadata is untrusted, never slice-panic
}

pub trait DenseEmbedder {
    fn dim(&self) -> usize;
    /// The model these vectors come from, recorded in and checked against the collection (D8).
    fn identity(&self) -> EmbedderIdentity;
    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, StoreError>;
}
```

JSON shape stored in the collection metadata (`serde_json::to_value(identity)`):

```json
{"embedder": {"model": "Xenova/bge-small-en-v1.5",
              "revision": "ea104dacec62c0de699686887e3f920caeb4f3e3",
              "onnx_sha256": "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35",
              "dim": 384, "pooling": "cls", "normalisation": "l2"}}
```

Every implementor and call site (signature change; run `change verify` first):

| Site | Change |
|---|---|
| `embed.rs` `impl DenseEmbedder for HashEmbedder` | `fn identity(&self) -> EmbedderIdentity { EmbedderIdentity::hash(self.dim) }` |
| `embed.rs` `impl DenseEmbedder for RtenEmbedder` | `crate::model::identity()` |
| `embed.rs` `impl DenseEmbedder for FastEmbedder` (alive until T5) | `crate::model::identity()` (same files, ANA-23 §5.6) |
| `tests/qdrant_live.rs` new `struct Renamed { inner: HashEmbedder, identity: EmbedderIdentity }` | test-only implementor delegating `dim`/`embed` |
| `vector.rs` `create_collection`, `ensure_collection` | the only callers of `identity()` |
| `htui/tests/qdrant_worker.rs`, `htui-store/tests/qdrant_live.rs` `throwaway()` | unchanged: a new throwaway collection is created with `hash/384` |

`vector.rs`:

```rust
/// The collection metadata key the embedder's identity is stored under (D8).
pub const EMBEDDER_KEY: &str = "embedder";

/// What `ensure_collection` does with a collection that already exists.
#[derive(Debug, PartialEq, Eq)]
enum Existing { Matches, Stamp }

/// Pure, unit-tested: size first (B8), then identity.
fn check_existing(collection: &str, dense_size: Option<u64>, stored: Option<serde_json::Value>,
                  mine: &EmbedderIdentity, dim: usize) -> Result<Existing, StoreError>;

/// `config.params.vectors_config` → `ParamsMap` → `map[DENSE].size`; `None` for anything else.
fn dense_size(info: &CollectionInfo) -> Option<u64>;

/// `{EMBEDDER_KEY: identity}` for `CreateCollectionBuilder::metadata` / `UpdateCollectionBuilder::metadata`.
fn metadata(&self) -> Result<HashMap<String, serde_json::Value>, StoreError>;   // to_value Err → backend("embedder identity", e)
```

`ensure_collection`: `!exists` → `create_collection()` (now `.metadata(self.metadata()?)`). Else
`collection_info(&self.collection)` (`backend("collection info", e)`; `result: None` →
`backend("collection info", "no result")`), then `check_existing(...)`: `Matches` → nothing;
`Stamp` → `update_collection(UpdateCollectionBuilder::new(&self.collection).metadata(self.metadata()?))`
(`backend("stamp embedder", e)`) and `tracing::info!(collection, identity = %mine, "stamped the
collection's embedder")`. The payload-index loop that follows is unchanged. Stored metadata is read
as `info.config.as_ref().and_then(|c| c.metadata.get(EMBEDDER_KEY)).cloned().map(serde_json::Value::from)`.

Exact refusals (built with the existing `backend(ctx, e)` → `qdrant: {ctx}: {e}`, ctx = `embedder mismatch`;
`{remedy}` = ``delete collection `{c}` and run `htui --index-items` to rebuild it`` (OQ-2 default)):

| Case | Text |
|---|---|
| other identity | ``qdrant: embedder mismatch: collection `{c}` holds vectors from {stored}, but this htui embeds with {mine}; {remedy}`` |
| unreadable `embedder` value | same, with `{stored}` = the raw JSON (`Value::to_string()`) |
| other dense size | ``qdrant: embedder mismatch: collection `{c}` holds {found}-wide `dense` vectors, but this htui embeds {dim}-wide ones; {remedy}`` |
| no `dense` vector / no params | ``qdrant: embedder mismatch: collection `{c}` has no `dense` vector; {remedy}`` |

T4 tests. Pure, in `vector::tests` (default suite, no Qdrant):

| Test | Asserts |
|---|---|
| `an_equal_identity_matches` | `Ok(Existing::Matches)` |
| `a_collection_without_an_identity_is_stamped` | `stored = None`, right size → `Ok(Existing::Stamp)` |
| `another_identity_is_refused_naming_both_and_the_remedy` | text contains both `Display`s, the collection name, `htui --index-items` |
| `another_width_is_refused_before_any_stamp` | `stored = None`, size 8, dim 384 → `Err` containing `8-wide` and `384-wide` |
| `a_collection_without_a_dense_vector_is_refused` | `dense_size = None` → `Err` containing ``no `dense` vector`` |
| `an_unreadable_identity_is_refused` | `stored = json!("garbage")` → `Err` containing `"garbage"` |
| `the_identity_round_trips_through_qdrant_metadata` | `serde_json::to_value(id)` → `qdrant_client::qdrant::Value::from` → `serde_json::Value::from` → `from_value` equals `id` (pins `dim` as an integer) |

In `embed::tests`: `the_hash_identity_reads_hash_slash_dim` (`EmbedderIdentity::hash(384).to_string() == "hash/384"`, `HashEmbedder::new(8).identity() == EmbedderIdentity::hash(8)`); under `local-embed`:
`the_model_identity_is_the_pinned_bge` (fields equal the constants; `Display` starts with `Xenova/bge-small-en-v1.5@ea104dac`).

Live, in `tests/qdrant_live.rs` (`--features test-support`, skip line without `HTUI_TEST_QDRANT_URL`):
helper `fn raw(url) -> Qdrant` (`from_url(..).skip_compatibility_check().build()`), helper
`async fn stored_identity(&Qdrant, &str) -> Option<EmbedderIdentity>`, helper
`async fn create_bare(&Qdrant, name, dense_size)` (dense named `DENSE` + sparse `SPARSE`, **no metadata**,
the layout `create_collection` uses). Every case drops its collection through `raw` before its
final assert, so a failure leaves nothing behind.

| Test | Asserts |
|---|---|
| `a_new_collection_records_the_embedder` | `throwaway()`; `stored_identity == Some(EmbedderIdentity::hash(DENSE_DIM))` |
| `an_unrecorded_collection_is_stamped_once` | `create_bare(384)`; `connect_to` with `HashEmbedder` is `Ok`; identity now stored; a second `connect_to` is `Ok` |
| `another_identity_is_refused_naming_both` | `throwaway()` (stamps `hash/384`); `connect_to` the same name with `Renamed { identity: model "test/other" }` → `Err` naming `hash/384`, `test/other/384`, the collection |
| `another_dense_width_is_refused_and_not_stamped` | `create_bare(8)`; `connect_to` with `HashEmbedder::new(384)` → `Err` with `8-wide`; `stored_identity == None` afterwards |

### 2.7 The three construction sites (T3; D7, R-NF-3)

`crates/htui/src/concepts.rs`:

```rust
use htui_store::embed::RtenEmbedder;          // replaces FastEmbedder
use htui_store::model;

async fn open() -> anyhow::Result<(PgStore, QdrantStore<RtenEmbedder>)> {
    // settings, dsn, identity, pg: unchanged
    let files = model::ensure_model().await.context("cannot load the embedding model")?;
    let embedder = tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
        .await
        .context("the embedding model's loader stopped")?
        .context("cannot load the embedding model")?;
    let store = QdrantStore::connect(&settings, embedder).await.context("cannot reach Qdrant")?;
    Ok((pg, store))
}
```
(`open` runs on the CLI's runtime before any terminal work; `spawn_blocking` there is fine.)

`open_index` **[A-4]** keeps its loop, its `warn!` and its sleep; the model is loaded once:

```rust
/// The model, then the collection; a failure is a `warn` and another attempt after the interval,
/// forever (plan D19). The model is fetched on this task and loaded on a thread of its own
/// (`apart`), once: `RtenEmbedder` is `Clone`, so a retry after an unreachable Qdrant reuses it.
async fn open_index(pg: &PgStore, settings: &QdrantSettings) -> QdrantStore<RtenEmbedder> {
    let mut loaded: Option<RtenEmbedder> = None;
    loop {
        let opened = match index_model(&mut loaded).await {
            Ok(embedder) => QdrantStore::connect(settings, embedder)
                .await
                .map_err(|err| format!("cannot reach Qdrant: {err}")),
            Err(err) => Err(err),
        };
        // match opened / warn / sleep: unchanged
    }
}

/// The loaded model, or a fetch (on this task: `apart` threads have no runtime) and a load (on a
/// thread of its own).
async fn index_model(loaded: &mut Option<RtenEmbedder>) -> Result<RtenEmbedder, String> {
    if let Some(embedder) = loaded { return Ok(embedder.clone()); }
    let files = model::ensure_model().await.map_err(|err| err.to_string())?;
    let embedder = match apart("htui-index-model", move || RtenEmbedder::load(&files)).await {
        Some(Ok(embedder)) => embedder,
        Some(Err(err)) => return Err(err.to_string()),
        None => return Err("the model loader stopped".to_owned()),
    };
    *loaded = Some(embedder.clone());
    Ok(embedder)
}
```

The test `fast_embedder_is_clone` (`concepts.rs:563`) becomes:

```rust
/// The search runtime shares one loaded model between its tasks (MOD-64 D238), the index job
/// keeps one across retries, and both move it across tasks; `htui-store`'s own tests build
/// without `local-embed`, so the check lives here (D258).
#[test]
fn rten_embedder_is_clone_send_and_sync() {
    fn shared<T: Clone + Send + Sync + 'static>() {}
    shared::<RtenEmbedder>();
}
```

`crates/htui/src/concepts_worker.rs` (the `Loader`/`Shared`/`OnceCell`-free shape of D238/D245 is unchanged):

| Line(s) today | Change |
|---|---|
| `:18` `use htui_store::embed::FastEmbedder;` | `use htui_store::embed::RtenEmbedder;` + `use htui_store::model;` |
| `:58` `type Loader = Arc<dyn Fn() -> BoxFuture<'static, Result<FastEmbedder, String>> + Send + Sync>;` | `RtenEmbedder` |
| `:60-67` `load_model` | body below; doc: "The pinned files (fetched or adopted, on this task), then `RtenEmbedder::load` on the blocking pool (D238, MOD-68 D7)." |
| `:81` `connections: Connections<QdrantStore<FastEmbedder>>` | `RtenEmbedder` |
| `:137` `type Load = Shared<BoxFuture<'static, Result<FastEmbedder, String>>>;` | `RtenEmbedder` (`Shared` needs `Output: Clone`: `RtenEmbedder: Clone` ✓) |
| `:157-160` `with_loader(loader: impl Fn() -> BoxFuture<'static, Result<FastEmbedder, String>> + …)` | `RtenEmbedder` |
| `:178` `async fn embedder(&self) -> Result<FastEmbedder, String>` | `RtenEmbedder` |
| `:189-191` `connect(...) -> Result<Arc<QdrantStore<FastEmbedder>>, String>` | `RtenEmbedder` |

```rust
fn load_model() -> BoxFuture<'static, Result<RtenEmbedder, String>> {
    Box::pin(async {
        let files = model::ensure_model().await.map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || RtenEmbedder::load(&files))
            .await
            .map_err(|e| format!("the embedding model's loader stopped: {e}"))?
            .map_err(|e| e.to_string())
    })
}
```

The load-counting tests need **no edit beyond the type**: `qdrant_index_without_a_stored_url_names_where_to_set_it`
wraps `load_model()` and asserts it is never called (settings are read first, so no fetch happens
either); `superseded_searches_share_one_failing_load_and_the_next_search_retries_once` builds its
own failing `Box::pin(async { … Err(..) })`, whose type is inferred from `with_loader`. They need
`ensure_model()`'s future to be `Send` (`BoxFuture`), which `ensure_model_futures_are_send` pins.

### 2.8 Manifests

Workspace `Cargo.toml` (T2): in `[workspace.dependencies]`, after `qdrant-client`:

```toml
# MOD-68 (ANA-23 §7.1): the embedder. `onnx_format` only: the `.rten` loader and the contrib ops
# are not needed. Pure Rust, no build script needing a C compiler.
rten                  = { version = "0.26.0", default-features = false, features = ["onnx_format"] }
rten-tensor           = "0.26.0"
# `fancy-regex`, not the default `onig`: no C build (ANA-23 §5.2).
tokenizers            = { version = "0.23.2", default-features = false, features = ["fancy-regex"] }
```

and, after `[profile.dev]` **[A-1]**:

```toml
# MOD-68: the embedder's numeric crates at full optimisation in dev and test builds. Measured on
# 2026-10-01: one 512-token text takes 10.6 s at opt-level 0 and 90 ms at 3; hashing the 133 MB
# model takes 2.5 s against 69 ms. Only these crates are affected; everything else keeps the fast
# debug build.
[profile.dev.package.rten]
opt-level = 3
[profile.dev.package.rten-base]
opt-level = 3
[profile.dev.package.rten-gemm]
opt-level = 3
[profile.dev.package.rten-onnx]
opt-level = 3
[profile.dev.package.rten-parallel]
opt-level = 3
[profile.dev.package.rten-shape-inference]
opt-level = 3
[profile.dev.package.rten-simd]
opt-level = 3
[profile.dev.package.rten-tensor]
opt-level = 3
[profile.dev.package.rten-vecmath]
opt-level = 3
[profile.dev.package.sha2]
opt-level = 3
```
(Check `cargo metadata`/`Cargo.lock` after T2 for the exact `rten-*` set; cargo only *warns* on a
spec that matches nothing, but a missed crate silently stays slow.)

`crates/htui-store/Cargo.toml`:

| Task | `[features] local-embed` | `[dependencies]` |
|---|---|---|
| T2 | `["dep:fastembed", "dep:rten", "dep:rten-tensor", "dep:tokenizers"]` | `rten = { workspace = true, optional = true }`, `rten-tensor = {…}`, `tokenizers = {…}` |
| T3 | `+ "dep:reqwest", "dep:rustls"` | `reqwest = { workspace = true, optional = true }`, `rustls = { workspace = true, optional = true }` (stays in `[dev-dependencies]` too; cargo allows both) |
| T5 | drop `"dep:fastembed"`; comment rewritten: "MOD-68: the production embedder (`embed::RtenEmbedder`) and its first-use model fetch (`model`). Off by default so the store's own tests build without it; the `htui` binary turns it on. Nothing is downloaded at build time." | drop `fastembed` |

Workspace `Cargo.toml` T5: drop `fastembed` and `ureq`. Update the comment above `reqwest` to
say `htui-store`'s `model` fetch uses it too.

## 3. Tasks (serial; tests first in each)

Every commit message ends with the attribution line from the session reminder
(`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`). Stage explicit paths only; never
`git add -A`. Commit as soon as each step is green (memory: uncommitted subagent work dies with the
session). Gortex workflow per edit: `read(editing_context)` → `change(impact)` → `edit` →
`change(detect)`; `change(verify)` before T4's trait change.

### T1 - fastembed goldens (fastembed still builds)

Precondition: `~/.cache/htui/fastembed/models--Xenova--bge-small-en-v1.5/{refs/main, snapshots/ea104dac…/}`
exists (re-seeded 2026-10-01). `ort-sys` downloads onnxruntime while building if `target/` lost it
(the sandbox was recreated): network is needed for this one build.

1. **Test first**: add to `embed::tests` the `Goldens`/`Provenance` types, `goldens_path`,
   `read_goldens`, and `goldens_fixture_is_six_unit_vectors_of_384` (§2.2). It fails: no fixture.
2. Add `golden_texts()` and `record_fastembed_goldens` (§2.2, cfg `local-embed`, ignored, env-gated).
3. Record:
   `HTUI_RECORD_GOLDENS=1 cargo test -p htui-store --features local-embed --lib embed::tests::record_fastembed_goldens -- --ignored --exact`
4. Check the fixture by eye: 6 texts, 6 × 384, provenance filled; text 3 is the long one.

Validate:
```bash
cargo test -p htui-store --lib embed                                   # shape test green, no feature
cargo clippy -p htui-store --all-targets --features local-embed -- -D warnings
cargo fmt --all -- --check
```
Commit (1): `test(mod-68): record fastembed's BGE vectors as goldens before the swap` —
`crates/htui-store/src/embed.rs crates/htui-store/tests/fixtures/bge_small_goldens.json`.

### T2 - `RtenEmbedder` (tests first)

1. **Manifests**: workspace `rten`, `rten-tensor`, `tokenizers` and the profile overrides (§2.8);
   `htui-store` T2 row (§2.8). `cargo check -p htui-store --features local-embed` updates the lock.
2. **`model.rs` skeleton** (B1): constants, `ModelFiles`, `in_dir`; `lib.rs`
   `#[cfg(feature = "local-embed")] pub mod model;`. (No `identity()` yet: `EmbedderIdentity` is T4.)
3. **Tests first** in `embed::tests`: the six splitter tests (§2.3, no feature), the four rten tests
   and the T2 `golden_model()` helper (§2.4). The splitter tests fail to compile until step 4: write
   `sub_batches` as `todo!()` first if a red run is wanted, then fill it.
4. `MAX_BATCH_TOKENS`, `sub_batches`; then `RtenEmbedder`, `Rten`, `load`, `embed_blocking`,
   `impl DenseEmbedder` (§2.4).

Validate:
```bash
cargo test -p htui-store --lib embed                                                 # splitter, shape
cargo test -p htui-store --features local-embed --lib embed -- --include-ignored     # goldens on rten (seeded snapshot); < 10 s with [A-1]
cargo clippy -p htui-store --all-targets --features local-embed -- -D warnings
cargo build -p htui                                                                  # fastembed and rten side by side still link
cargo fmt --all -- --check
```
If a golden misses: compare token ids against fastembed's first (`tokenizers` on the same file,
`encode_batch` of the six texts), then the special-token step, then truncation (default
`TruncationParams` is `LongestFirst`, right side, stride 0 — fastembed's).

Commits (2): `build(mod-68): add rten and tokenizers to the workspace, optimised in dev builds` —
`Cargo.toml crates/htui-store/Cargo.toml Cargo.lock`; then `feat(mod-68): RtenEmbedder behind
DenseEmbedder, with a padded-token cap per pass` — `crates/htui-store/src/embed.rs
crates/htui-store/src/model.rs crates/htui-store/src/lib.rs`. (Manifests alone do not build
anything new without code, so the first commit is green on its own.)

### T3 - model fetch and the three construction sites

1. **Manifests**: `htui-store` T3 row (`reqwest`, `rustls` optional under `local-embed`).
2. **Tests first** in `model::tests`: the stub and the 14 cases of §2.5, plus one more:
   `real_download_from_huggingface` — `#[ignore = "downloads 133 MB from huggingface.co"]`,
   `ModelSource::new("https://huggingface.co", tempdir, pins())`, `ensure()` is `Ok` and both files
   verify. It is the only test that exercises TLS and HF's real redirects.
3. `model.rs`: `Pin`, `pins()`, `ModelSource`, `ensure`, `ensure_model`, the provider install,
   `hex`, the `.part` sequence, the sweep (§2.5).
4. Swap `golden_model()`'s body to `crate::model::ensure_model().await.expect("the model is fetched or adopted")` (B3).
5. `concepts.rs` (`open`, `open_index`, `index_model`, the Clone test) and `concepts_worker.rs`
   (§2.7).

Validate:
```bash
cargo test -p htui-store --features local-embed --lib model
cargo test -p htui-store --features local-embed --lib embed -- --include-ignored     # adopts the fastembed snapshot into ~/.cache/htui/model (133 MB), zero downloads
cargo test -p htui-store --features local-embed --lib model::tests::real_download_from_huggingface -- --ignored --exact   # once; network
cargo test -p htui --features testkit --lib concepts -- --test-threads=1             # concepts + concepts_worker
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Commits (2): `feat(mod-68): pinned, verified model fetch with fastembed cache adoption` —
`crates/htui-store/Cargo.toml Cargo.lock crates/htui-store/src/model.rs crates/htui-store/src/embed.rs`;
then `feat(mod-68): build RtenEmbedder at the three sites, fetch on the task, load off it` —
`crates/htui/src/concepts.rs crates/htui/src/concepts_worker.rs`.

### T4 - model identity on connect

Qdrant: `docker compose up -d qdrant` (compose.yaml; gRPC on 6334), then
`export HTUI_TEST_QDRANT_URL=http://localhost:6334`. If no Docker in the sandbox, the live cases
print the skip line; the pure `check_existing` tests still gate the logic, and the live run moves
to the host (say so in the hand-off).

1. `change(verify)` on `DenseEmbedder` with the new method.
2. **Tests first**: the seven `vector::tests`, the two `embed::tests` and the four live cases plus
   `Renamed`, `raw`, `stored_identity`, `create_bare` in `tests/qdrant_live.rs` (§2.6).
3. `EmbedderIdentity` (+ `hash`, `Display`), the trait method, the three implementors,
   `model::identity()`; `vector.rs`: `EMBEDDER_KEY`, `Existing`, `check_existing`, `dense_size`,
   `metadata`, `create_collection` and `ensure_collection` (§2.6).

Validate:
```bash
cargo test -p htui-store --lib -- vector:: embed::                     # two filters go after --
cargo test -p htui-store --features local-embed --lib embed
cargo test -p htui-store --features test-support --test qdrant_live -- --test-threads=1
HTUI_TEST_DATABASE_URL=… cargo test -p htui --features testkit --test qdrant_worker      # if Postgres is up; HashEmbedder throwaway still works
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
Commit (1): `feat(mod-68): record the embedder in the Qdrant collection and refuse a mismatch` —
`crates/htui-store/src/embed.rs crates/htui-store/src/model.rs crates/htui-store/src/vector.rs crates/htui-store/tests/qdrant_live.rs`.

### T5 - remove `fastembed` (after the D10 cross-check is green)

1. **D10** (OQ-1 default), in a scratch crate, **not** `htui-store` (its `ring`, `aws-lc-sys`,
   `libsqlite3-sys` build scripts need C cross toolchains this box lacks):
   ```bash
   rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin
   cargo new --lib /tmp/mod68cross && cd /tmp/mod68cross
   # [dependencies] exactly the workspace's: rten 0.26.0 (no default, onnx_format), rten-tensor 0.26.0, tokenizers 0.23.2 (no default, fancy-regex)
   # src/lib.rs: one fn that calls Model::load_file, Tokenizer::from_file, ThreadPool::with_num_threads, so the code paths are monomorphised
   for t in x86_64-pc-windows-msvc aarch64-apple-darwin x86_64-apple-darwin; do cargo check --release --target "$t" || exit 1; done
   rm -rf /tmp/mod68cross
   ```
   Record the three green lines in the commit body.
2. Remove: `FastEmbedder` (struct, `Debug`, `new`, `DenseEmbedder` impl), `fast_embedder_returns_384_dims`,
   `record_fastembed_goldens`, `golden_texts`; workspace `fastembed`, `ureq`; `htui-store`
   `fastembed` and `"dep:fastembed"`. The `Goldens`/`Provenance` types and the fixture **stay**.
3. Docs in code: `embed.rs` module doc (production is `RtenEmbedder`; the model is fetched on first
   use by `crate::model`, nothing at build time); the `local-embed` comment (§2.8); the workspace
   `reqwest` comment; `docker/hr/Dockerfile:15` comment to "libssl-dev: native-tls (sentry → reqwest)"
   **[A-5]**.
4. README: line 54-56 bullet → "**Network access on first use of the concepts index.** The
   embedding model (133 MB) is downloaded into your user cache directory the first time you index
   or search, and checked against a pinned hash. An existing download from an earlier htui is
   reused." Line 385 → "The first run downloads the embedding model (133 MB) into your user cache
   directory and checks it." Lines 496-499: delete the "Building without network access" section
   and the anchor link at 56. After line 400 add: "If a later htui embeds with another model, it
   refuses the old index and says so; delete it with `curl -X DELETE
   http://localhost:6333/collections/htui_concepts_v2` and run `htui --index-items` again." Do
   **not** claim the OpenSSL headers are gone.
5. Prove the removal:
   ```bash
   cargo tree -i ort; cargo tree -i onig_sys; cargo tree -i hf-hub; cargo tree -i ureq   # each: "did not match any packages"
   ```

Validate: the full gate (§3.1). Commits (2): `build(mod-68): drop fastembed and ureq; the embedder is
rten` — `Cargo.toml Cargo.lock crates/htui-store/Cargo.toml crates/htui-store/src/embed.rs`; then
`docs(mod-68): README and comments for the first-use model fetch` — `README.md docker/hr/Dockerfile`.

### 3.1 Full gate (after T5)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod68-gate.log
grep -E "SIGABRT|FAILED|panicked" /tmp/mod68-gate.log                     # memory: htui-orch stack headroom; expect nothing
cargo test -p htui-store --features local-embed --lib embed -- --ignored  # goldens on rten (D2)
cargo test -p htui-store --features test-support --test qdrant_live -- --test-threads=1   # with HTUI_TEST_QDRANT_URL
cargo tree -i ort; cargo tree -i onig_sys; cargo tree -i hf-hub           # each: no match
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## 4. Hazards (second fact-check of the plan against the code)

| # | Hazard | Plan says / misses | Evidence | Resolution in this blueprint |
|---|---|---|---|---|
| H-1 | **Debug builds run rten ~118x slower** (10.6 s per 512-token text, 66 s for the six goldens) and hash the model ~36x slower (2.5 s) | Missed; the probe and ANA-23 timed release only | measured 2026-10-01, §0 | `[profile.dev.package.<rten*, sha2>] opt-level = 3` in T2 **[A-1]**: 90 ms / 608 ms / 69 ms |
| H-2 | Tokenising once with tokenizer padding pads every sub-batch to the call's global longest, so D5 would cap nothing | D4 "pad `BatchLongest`" + D5 "tokenised once, then split" contradict | `PaddingStrategy::BatchLongest` pads one `encode_batch` call | Truncation only in the tokenizer; zero-fill per sub-batch by hand (B4) **[A-2]** |
| H-3 | Unique per-process `.part` names never "replace" a killed run's `.part`; 133 MB of debris per killed run | T3 test "a `.part` from a killed run is replaced" | D6 + Risks row "per-process unique `.part` names" | 10-minute mtime sweep; test `a_stale_part_is_swept_and_a_fresh_one_left` **[A-3]** |
| H-4 | `load_model` returns `BoxFuture` (`Send`) and `spawn_index_job` `tokio::spawn`s `open_index`: `ensure_model()`'s future must be `Send` | Not stated | `concepts_worker.rs:58-67`, `concepts.rs:285-288` | No `std` lock or non-`Send` error held across an `.await` in `model.rs`; blocking steps in `spawn_blocking`; `ensure_model_futures_are_send` |
| H-5 | `RtenEmbedder` must be `Clone` (`Shared` output, D238), `Send + Sync` (`ConceptIndex: Send + Sync` holds `QdrantStore<RtenEmbedder>`), and `Debug` (`missing_debug_implementations`, `-D warnings`) | D3 says `Clone` only | `concepts_worker.rs:50,137`; workspace lints | `Arc<Rten>`, manual `Debug`; `rten_embedder_is_clone_send_and_sync` in `htui` (store tests build without the feature) |
| H-6 | TLS backend differs by build: in the `htui` binary `sentry` turns on reqwest `native-tls`, so reqwest's **default backend is NativeTls** (`reqwest tls.rs:621-635`); in `htui-store` tests it is rustls with the **aws-lc-rs** fallback (qdrant-client's default `rustls` feature) | Plan: "`rustls-no-provider` client needs the ring install" | `cargo tree -e features -i native-tls`; `reqwest client.rs:721,2510` | Neither path can panic; the `ring` install is kept to mirror `install/http.rs` and fix the provider process-wide. No new crate in the lock (reqwest, rustls already in `htui-store`'s graph). TLS is exercised only by the ignored `real_download_from_huggingface` **[A-8]** |
| H-7 | `openssl-sys` stays because of **sentry** alone; `qdrant-client` uses rustls | T5: "pulled by `qdrant-client` and `sentry`"; Dockerfile comment names hf-hub/fastembed | `cargo tree -e features -i native-tls` | T5 updates `docker/hr/Dockerfile:15` **[A-5]**; README still makes no OpenSSL claim |
| H-8 | D10 cannot `cargo check` `htui-store` for Windows/macOS (`ring`, `aws-lc-sys`, `libsqlite3-sys` need C cross toolchains) and the three targets are no longer installed | "a cross `cargo check` of the embedder stack" | `rustup target list --installed` | Scratch crate with exactly the workspace's rten/tokenizers specs, `rustup target add` first (network) **[A-6]** |
| H-9 | Windows: `rename` onto a `model.onnx` another process has open fails; `remove_file` of an open `.part` fails | Risks row assumes the loser's rename overwrites | `install/fetch.rs:107-112` (drop before unlink) | Drop the handle before rename/remove; a failed rename whose `dest` now verifies is success |
| H-10 | `#[cfg(feature = "local-embed")]` gating: `model.rs` and its tests compile only with the feature; the default `cargo test -p htui-store` does not run them | Not stated | `lib.rs`, feature list | Splitter `cfg(any(test, feature))`; fixture shape and identity tests ungated; per-task commands name `--features local-embed`; the workspace gate uses `--all-features` |
| H-11 | `htui-store` has `#![warn(missing_docs)]`: every `pub` const, field and fn of `model.rs` and `EmbedderIdentity` needs a doc | Not stated | `lib.rs:11` | §2.5/§2.6 list them; `pub(crate)` for the seam |
| H-12 | A lib unit test reaches `tests/fixtures` only by a run-time path; `include_str!` would not compile before T1 records | Not stated | — | `env!("CARGO_MANIFEST_DIR")` + `read_to_string` (B2) |
| H-13 | An "unwritable cache dir" test by `chmod` passes under root (sandbox) | T3 case list | — | Make `cache_root` a regular file |
| H-14 | reqwest `system-proxy` reads `HTTP_PROXY`/`ALL_PROXY`; a set proxy would route the loopback stub | Not stated | workspace `reqwest` features | None set in this sandbox; `htui-agent`'s install tests share the exposure. If one is set, run with `NO_PROXY=127.0.0.1` |
| H-15 | An identity refusal surfaces as `cannot reach Qdrant: qdrant: embedder mismatch: …` | Not stated | `concepts.rs:102,319`, `concepts_worker.rs:205` | Accepted: prefixes unchanged (overlay tests pin the shape); the refusal names both identities and the remedy |
| H-16 | Every process start hashes 133 MB to "use the dir if every file hashes" | D6 step 1 | measured 69 ms (opt) | Accepted with [A-1]; a size+mtime marker is a follow-up only if it shows |
| H-17 | A size mismatch on an unrecorded collection would be stamped and then fail every upsert | D8 lists the cases but not their order | — | Size checked first (B8); `another_dense_width_is_refused_and_not_stamped` |
| H-18 | Two processes stamping one unrecorded collection | — | `UpdateCollection` metadata merges | Same payload, harmless |
| H-19 | `a_stored_qdrant_url_starts_the_job` spawns `open_index`, which now starts with `ensure_model()` against the developer's real cache | — | `concepts.rs:777-791` | Current-thread test runtime: the job is aborted before it is polled; at worst a sweep or a hash, no write completes. Accepted |
| H-20 | `tokenizers` 0.23 `AddedToken::default()` is `normalized: true`; fastembed re-adds the five specials with it | ANA-23: "special tokens become constants" | `added_vocabulary.rs:78`, `fastembed common.rs:108-127` | `load` repeats the call from `SPECIAL_TOKENS` |
| H-21 | T3's golden run adopts into `~/.cache/htui/model` (133 MB) and `real_download_from_huggingface` writes 133 MB to a tempdir; disk is at 90% (44 G free) | — | `df -h /` | Fine; no extra target dirs (no `--release` gate thanks to [A-1]) |
| H-22 | `concepts_worker` reads settings before the model, but `open` and `open_index` fetch the model before connecting: a bad Qdrant URL still costs a first fetch | D7 | `concepts.rs:90-104` | Today's order, unchanged |
| H-23 | `ort-sys` downloads onnxruntime at build time; the sandbox was recreated, so T1's build needs network | T1 precondition covers the cache only | `local-embed` comment | Stated in T1 |

## 5. Recommended plan amendments

- **A-1** T2 adds per-package `opt-level = 3` for the `rten*` crates and `sha2` in `[profile.dev]` (H-1).
- **A-2** D4/D5: the tokenizer truncates only; padding is per sub-batch, by hand, equal to `BatchLongest` (H-2).
- **A-3** D6/T3: `.part` names are unique per process **and** stale ones (mtime > 10 min) are swept;
  the T3 case becomes "a stale `.part` is swept, a fresh one is left" (H-3).
- **A-4** D7: `open_index` keeps the loaded model across retries (the "unchanged loop" now reconnects
  without re-loading); this is what replaces the stale "not `Clone`" comment.
- **A-5** T5: `openssl-sys` stays because of `sentry` only; update `docker/hr/Dockerfile:15` too
  (one more file, 14 with the lock).
- **A-6** D10: the cross-check runs in a scratch crate after `rustup target add`, not on `htui-store`.
- **A-7** Files table: `model.rs` is created in T2 (constants, `ModelFiles`) and grows the fetch in T3.
- **A-8** T3 adds an ignored `real_download_from_huggingface` test, run once in the gate: the only
  check of TLS and HF's real redirects, and the "fresh cache + network" acceptance line.
- **A-9** D6: each pin carries its byte size; a body larger than it is refused before it fills the disk.
- **A-10** D8: the dense size is checked before the identity, so a wrong-width collection is never stamped.

## 6. Architecture summary

### Files to create

| File | Purpose | Task |
|---|---|---|
| `crates/htui-store/tests/fixtures/bge_small_goldens.json` | fastembed's six vectors + provenance; the pin for ANA-23 §5.6 | T1 |
| `crates/htui-store/src/model.rs` | pins, tokenizer constants, `ModelFiles`, `identity()`, `ModelSource`, `ensure_model()` | T2 (constants), T3 (fetch), T4 (`identity`) |

### Files to modify

| File | Changes | Task |
|---|---|---|
| `crates/htui-store/src/embed.rs` | goldens tests + recorder; `sub_batches`, `RtenEmbedder`; `EmbedderIdentity`, `DenseEmbedder::identity`; drop `FastEmbedder` + recorder | T1, T2, T3 (helper body), T4, T5 |
| `crates/htui-store/src/lib.rs` | `#[cfg(feature = "local-embed")] pub mod model;` | T2 |
| `crates/htui-store/src/vector.rs` | `EMBEDDER_KEY`, `check_existing`, `dense_size`, `metadata`; create/ensure write and check the identity | T4 |
| `crates/htui-store/tests/qdrant_live.rs` | four identity cases, `Renamed`, raw-client helpers | T4 |
| `crates/htui-store/Cargo.toml` | `local-embed` = rten, rten-tensor, tokenizers, reqwest, rustls; drop fastembed | T2, T3, T5 |
| `Cargo.toml` | workspace rten/rten-tensor/tokenizers; dev profile overrides; drop fastembed, ureq | T2, T5 |
| `Cargo.lock` | follows the manifests | T2, T3, T5 |
| `crates/htui/src/concepts.rs` | `open`, `open_index` + `index_model`, Clone/Send/Sync test | T3 |
| `crates/htui/src/concepts_worker.rs` | `Loader`, `Load`, `load_model`, `connections`, `with_loader`, `embedder`, `connect` types | T3 |
| `README.md` | first-use fetch, no build-time download, mismatch remedy | T5 |
| `docker/hr/Dockerfile` | `libssl-dev` comment: sentry → reqwest native-tls | T5 [A-5] |

### Data flow

`--index-items` / `--search-items` (`concepts::open`), the worker's index job
(`concepts::open_index`) and the TUI's search (`QdrantIndex::load` → `load_model`) all do the same
three steps off the UI thread: (1) `model::ensure_model()` on the caller's tokio task: for each
pinned file, verify the cached copy, else copy-and-hash from a fastembed snapshot, else stream it
from `huggingface.co/<repo>/resolve/<commit>/<file>` into a unique `.part`, hash as it arrives,
`sync_all`, rename on a match; (2) `RtenEmbedder::load(&ModelFiles)` on `spawn_blocking` (CLI,
TUI) or `apart` (worker); (3) `QdrantStore::connect(settings, embedder)`, whose
`ensure_collection` creates the collection with the embedder's identity in its metadata, or
reads an existing one's `dense` size and `embedder` metadata and matches, stamps, or refuses.
After that, `embed()` runs `embed_blocking` on the blocking pool: tokenise once (truncate 512),
split into sub-batches of at most 8,192 padded tokens, pad each by hand, run the graph with the
explicit thread pool, take CLS, L2-normalise, return in input order.

### Build sequence

1. T1: goldens fixture + shape test (fastembed still present). 1 commit.
2. T2: manifests + profile overrides; `model.rs` constants; splitter; `RtenEmbedder`; golden gate green. 2 commits.
3. T3: `ensure_model` + stub tests; three construction sites; golden gate via `ensure_model`. 2 commits.
4. T4: `EmbedderIdentity`, trait method, `vector.rs` check/stamp; pure + live tests. 1 commit.
5. T5: D10 cross-check (scratch crate); remove fastembed/ureq; README, comments, Dockerfile; full gate. 2 commits.
6. Reviewer (`rust-reviewer`, unpinned per memory) over `main..hr/MOD-68`; then the workflow's close-out (HANDOFF, DECISIONS) - not part of T1-T5.
