# ANA-23 - Pure-Rust local embedder to replace `ort`/`fastembed`

> **Scope note:** "`htui-store`'s `local-embed` feature (`embed::FastEmbedder`, BGE-small-en-v1.5,
> MOD-34, `docs/decisions/mod/mod-34.md`) pulls `ort`, whose `ort-sys` build script downloads ONNX
> Runtime from `parcel.pyke.io` or needs `ORT_LIB_LOCATION` pointing at a native `libonnxruntime`
> [...]. The maintainer wants no native runtime fetched at build time. Compare **`candle`** (with
> `tokenizers` on its `fancy-regex` feature, no `onig`) and **`tract-onnx`** (runs the same ONNX
> file): offline build, binary size, embedding speed on the demo corpus, how and when model weights
> are fetched at run time, and whether the vectors equal the ones already stored in qdrant within a
> tolerance or force a re-embed. Deliver a verdict and the `MOD-N` that implements it."
> (`HANDOFF.md`, ANA-23; opened 2026-09-26 during MOD-9 milestone 3.)
>
> **Requirements addressed:** `R-STO-8`, `R-NF-1`, `R-NF-2`.
>
> **Status (2026-10-01): concluded.**
> Verdict: **replace `fastembed`/`ort` with `rten`** (0.26, a pure-Rust ONNX runtime) running the
> **same** `Xenova/bge-small-en-v1.5` `onnx/model.onnx` file, with `tokenizers` 0.23 on
> `fancy-regex`. The weights are fetched at run time over htui's existing `reqwest`, **pinned to
> one repo commit and checked against a sha256 per file**, and an existing fastembed cache is
> reused. rten was not one of the two named candidates; the web survey surfaced it and it was
> measured on the same terms (§5). It beats both on every axis but one: the embedder's own
> dependency tree has **no C compiler at all**, it adds **8.1 MiB** to a stripped binary
> (fastembed: 24.1 MiB), it runs htui's real call shape at **1.29×** fastembed's time with
> about a quarter of fastembed's peak memory, and its vectors equal the stored ones to
> **1 − 4e-13 cosine**, so **nothing is re-embedded**. The one axis it loses is query latency
> (5.6 ms against 2.3 ms), which an interactive search does not notice. **tract-onnx** is the
> fallback (same file, 3.6× slower, needs an assembler); **candle** is rejected (4.3× slower even
> after patching its attention, a second 134 MB download, and an `onig` C dependency in every
> release after 0.9.2). Implemented by **MOD-68**.

---

## 1. Context and problem statement

MOD-34 gave htui a concepts index (`R-STO-8`): Qdrant holds a dense BGE-small-en-v1.5 vector and a
BM25 sparse vector per item, document section and requirement, fused with RRF. The dense half is
`embed::FastEmbedder` (`crates/htui-store/src/embed.rs:27`), a thin wrapper over `fastembed` 3.14.1.
`fastembed` runs ONNX through `ort` 2.0.0-rc.4, and `ort-sys`'s build script links a static ONNX
Runtime 1.18.1 that it **downloads at build time** (an 18.6 MB archive from `parcel.pyke.io`,
unpacked to an 87 MB `libonnxruntime.a`). A sandbox that blocks that host builds `htui` only with a
hand-fetched library behind `ORT_LIB_LOCATION`. The maintainer wants no native runtime fetched at
build time.

The item asks five questions of each candidate: offline build, binary size, speed on the demo
corpus, run-time weight fetching, and whether the vectors already in Qdrant stay valid.

## 2. Current state in htui

### 2.1 Wiring

- `fastembed = "3.14.1"` (workspace `Cargo.toml`), default features `ort-download-binaries` and
  `online`. `htui-store`'s `local-embed = ["dep:fastembed"]` is off by default; the `htui` binary
  turns it on (`crates/htui/Cargo.toml:36`), so every shipped binary carries it.
- `FastEmbedder::new` (`embed.rs:42`) loads `EmbeddingModel::BGESmallENV15` into
  `<dirs::cache_dir()>/htui/fastembed`. `DenseEmbedder::embed` (`embed.rs:62`) runs
  `model.embed(texts, None)` in `spawn_blocking`.
- Two embed calls: `QdrantStore::upsert` (`vector.rs:661`, documents) and `QdrantStore::search`
  (`vector.rs:734`, one query as a batch of one).
- Construction: `concepts.rs` `open()` (CLI `--index-items`/`--search-items`) and `open_index`
  (the background index job, on its own thread); `concepts_worker.rs` `load_model` (the TUI search
  overlay, `spawn_blocking`).

### 2.2 The embedding contract

Read from `fastembed-3.14.1`'s source and confirmed against the downloaded files:

| Aspect | Value |
|---|---|
| Model file | `Xenova/bge-small-en-v1.5` `onnx/model.onnx`, 133,093,490 B, blob `828e1496…cf35` |
| Revision | none: hf-hub follows `main` (commit `ea104dacec62c0de699686887e3f920caeb4f3e3` on 2026-10-01) |
| Tokenizer | BERT WordPiece, `BertNormalizer` (lowercase, clean text, CJK), `BertPreTokenizer`, `[CLS] $A [SEP]` |
| Length | truncate at 512 tokens from the right, pad `BatchLongest` with `[PAD]`/0 |
| Pooling | CLS (`[:, 0, :]` of the single output), not mean |
| Normalisation | `v / (‖v‖ + 1e-12)` |
| Prefix | none, for documents and queries alike |
| Batch | 256 (`embed(texts, None)`), `rayon` over chunks |
| Threads | ONNX Runtime intra-op = `available_parallelism()` |

htui's real call shape (`vector_sync.rs:120`, `:168`): **one embed call per item**, holding the
item point and all its document sections; requirements in chunks of up to 256; a query alone.

### 2.3 What Qdrant knows about the model

Collection `htui_concepts_v2` (`vector.rs:36`): named `dense` vector, size `DENSE_DIM` 384,
`Distance::Cosine`; named `sparse` vector with `Modifier::Idf`. **Nothing records which model
produced the stored vectors**: no model id in the collection name, the payload or any metadata, and
`ensure_collection` does not compare an existing collection's size or distance to the embedder. The
indexer's staleness test reads only `updated_at`, status, the latest document ids and the
requirement version. Today a swap to another 384-wide model would silently mix vectors, and a
different width would make Qdrant reject every upsert and query.

### 2.4 A defect found on the way: a fresh install cannot load the model

HuggingFace now answers the non-LFS files (`tokenizer.json`, `config.json`,
`special_tokens_map.json`, `tokenizer_config.json`) with `307` and a **relative** `Location`
(`/api/resolve-cache/models/...`). `hf-hub` 0.3.2, which `fastembed` 3.14.1 pins, passes that
header straight to `ureq::get` (`hf-hub-0.3.2/src/api/sync.rs:292-295`), which cannot parse a
relative URL. Reproduced on 2026-10-01: with an empty cache `FastEmbedder::new()` downloads the
133 MB `model.onnx`, builds the session, then fails with
`request error: Bad URL: failed to parse URL: RelativeUrlWithoutBase`, and every retry fails the
same way. Only a machine whose `~/.cache/htui/fastembed` was populated before HuggingFace changed
its redirects can search. With no network and no cache `fastembed` **panics** rather than erroring
(`text_embedding.rs:131`, `unwrap_or_else(panic!)`); htui's index loop and overlay catch it on
their own threads, but `concepts::open()` calls `FastEmbedder::new()` inline. Every candidate
below fixes this; MOD-68 is therefore also a bug fix.

## 3. Constraints

- **No native runtime fetched at build time** (the maintainer's ask). A C compiler is not
  forbidden: htui already compiles `ring` (rustls), `aws-lc-sys` and `openssl-sys` elsewhere in
  the tree.
- **`R-STO-8`**: when the embedding model is unavailable, search fails with a clear error and
  nothing else is affected. A panic on load is outside that contract.
- **`R-NF-1`**: Windows, Linux, macOS. **`R-NF-2`**: no daemon beyond Postgres and the agents (an
  in-process runtime satisfies it).
- Toolchain pinned at Rust 1.98.1 (`rust-version = "1.98"`).
- The stored vectors should stay valid, or the cost of a re-embed must be stated.

## 4. Survey (2026-10-01)

| Option | Version | What it is | Build-time native | Verdict |
|---|---|---|---|---|
| **rten** | 0.26.0 (2026-08-29), MIT/Apache-2.0, MSRV 1.94 | pure-Rust ONNX runtime, loads `.onnx` directly since 0.23 | none (no `build.rs` in any `rten-*` crate) | measured; **chosen** |
| **tract-onnx** | 0.23.8 (2026-09-21), MIT/Apache-2.0, MSRV 1.91 | pure-Rust ONNX runtime | `cc` + assembler for `tract-linalg`'s 42 `.S` kernels (no feature turns it off) | measured; fallback |
| **candle** | 0.9.2 (2026-01-24) / 0.11.0 (2026-06-26), MIT/Apache-2.0 | pure-Rust tensor library, `candle-transformers` `BertModel` over safetensors | 0.9.2: none; **0.10.0–0.11.0: `onig_sys` C** via a non-optional `tokenizers ^0.22` with `onig` (`candle-core-0.11.0/Cargo.toml:273`). Fixed on `main` by huggingface/candle#3952 (merged 2026-09-03), unreleased | measured; rejected |
| fastembed | 7.1.0 (2026-09-22) | current crate's newer line | `ort` =2.0.0-rc.13 is non-optional; still downloads or dynamically loads ONNX Runtime | ruled out: no pure-Rust mode |
| burn | 0.21 / 0.22.0-pre.4 | framework; `burn-onnx` generates Rust from ONNX at build time | — | too heavy and moving for one 33M-parameter encoder |
| model2vec-rs | 0.3.0 | static distilled embeddings | `esaxx` C++ even on `fancy-regex` | a different, weaker model class; would force a re-embed |
| embed_anything, gllm, edgebert, bge, candle_embed | — | wrappers | inherit candle's `onig`, or abandoned | skip |

**Tokenizer.** `tokenizers` 0.23.2 with `default-features = false, features = ["fancy-regex"]`
compiles no C (its `esaxx-rs` dependency builds C++ only under `esaxx_fast`, which is off). BGE's
pipeline (`BertNormalizer`, `BertPreTokenizer`, WordPiece) never reaches `SysRegex`, so the regex
backend cannot change a token. Measured anyway: `fancy-regex` and `onig` builds encoded a
50,011-line corpus (htui sources, CJK, emoji, control characters, 3,000-char lines) to
byte-identical ids, type ids, masks and offsets, at the same speed. BGE's `tokenizer.json` carries
`"truncation": null`; the caller must set 512. `tokenizers` 1.0.0-rc.2 drops truncation and
padding from its API and is not usable yet.

**Weight sources.** `Xenova/bge-small-en-v1.5` carries only ONNX files (`model.onnx` 133 MB, plus
fp16, int8 and q4 variants) and no licence field of its own; it is an export of
`BAAI/bge-small-en-v1.5` (MIT), whose `tokenizer.json` it matches key for key. `BAAI` carries the
safetensors candle needs (133,466,304 B). `hf-hub` 1.0 is async-only on `reqwest` 0.13 plus
`hf-xet`, and pulls `aws-lc-sys` through `cmake`; 0.5 (`ureq` 3, rustls+ring) works and follows the
relative redirect. Plain `https://huggingface.co/<repo>/resolve/<commit>/<file>` URLs return the
bytes over plain HTTPS, Xet-backed or not.

## 5. Measurements

### 5.1 Method

Everything ran in scratch crates under `/tmp/ana23` in the ANA-23 sandbox (4-CPU cgroup quota on a
12-thread Ryzen 7600X, Rust 1.98.1, default release profile), and nothing touched the repository.
The scratch tree was not kept; the numbers, commands and findings are recorded here.

- **Corpus.** 95 texts. 21 are htui's own: a harness linked to `htui-core[demo]` and
  `htui-store[test-support]` ran `vector_sync::Indexer::sync` over `MemStore::demo()` into
  `MemVectorStore` and dumped every `ConceptPoint.text`. That is 13 item points, 5 document
  sections and 3 requirements, 26 to 1,372 characters. The demo is small and mostly titles, so 74
  texts were added in the item-point shape (`"<H1>\n\n<first paragraph>"`) from
  `docs/decisions/*/*.md`, including one 23,986-character paragraph as a truncation case. Token
  lengths: median 123, eight texts at the 512 cap, 16,568 tokens in all. 12 queries, 11 in natural
  language and the exact key `MOD-34`.
- **Baseline.** A scratch crate on `fastembed` 3.14.1 with htui's lockfile versions and the same
  `InitOptions`. Its vectors stand in for the ones in Qdrant: the sandbox's Qdrant was empty, and
  fastembed's output is deterministic (bit-identical across processes, batch shapes and two ORT
  link modes). §8 lists the gaps this leaves.
- **Candidates.** One binary each that loads the model, embeds the corpus and the queries the way
  htui does, and writes the vectors. Each copies fastembed's tokenizer setup, CLS pooling and
  normalisation line for line. rten and tract load the same `model.onnx`; candle loads `BAAI`'s
  safetensors at commit `5c38ec7c`.
- **Comparison.** A stdlib-only script computes, in float64, each text's cosine, element-wise
  difference and L2 distance from the baseline, and each query's top 10 over the corpus. The top-10
  check ran twice: candidate queries against **baseline** documents (the real state after a
  switch: new queries, stored vectors) and candidate queries against candidate documents.
- **Timing.** One agent ran all four binaries in sequence, never concurrently, once the 1-minute
  load average was at or below 6, each mode in its own process: 4 threads, warm cache, medians of
  3 to 5 runs. The batch-of-one and single-thread rows were rerun in a second window and stayed
  within about 10%.

### 5.2 Offline build

| | Fetches at build time | Native compilation the embedder needs |
|---|---|---|
| fastembed/ort | **yes**: `ort-sys` downloads the 18.6 MB archive whenever its cache (`~/.cache/ort.pyke.io`, or `$XDG_CACHE_HOME/dfbin`) lacks it; `cargo --offline` does not stop a build script (measured: with the network open it downloaded under `--offline`; with it blocked the build panicked). `ORT_LIB_LOCATION` is the only hermetic route | C for `onig_sys` and `ring`; `pkg-config` and OpenSSL headers for `openssl-sys` (through `hf-hub`'s `native-tls`, dead at run time); `libstdc++` at link |
| **rten** | no | **none** (`cargo tree -i cc` empty without `hf-hub`; 94 crates) |
| tract | no | C compiler and assembler for `tract-linalg` (a build with `CC=/nonexistent/cc` fails) |
| candle 0.9.2 | no | none for the model; 0.10+ adds `onig_sys` |

All three candidates built with `cargo build --offline` under an `LD_PRELOAD` shim that fails
`connect` and `getaddrinfo`. With a weight fetcher on `hf-hub` 0.5, `ring` joins every tree. htui
already compiles `ring` for `rustls`, so that costs nothing; the chosen design fetches over htui's
existing `reqwest` and adds no crate at all. Every candidate binary links only `libc`, `libm` and
`libgcc_s` (`ldd`). The baseline also needs `libstdc++` (GLIBCXX_3.4.29).

### 5.3 Binary size

Stripped release binaries, measured as the delta over a tokio hello-world built in the same target
dir with htui's tokio features:

| Backend | Stripped total | Embedder delta |
|---|---|---|
| fastembed/ort (incl. `hf-hub` 0.3 + `ureq` 2 + TLS) | 25,871,112 B | **+24.1 MiB** |
| **rten**, no fetcher | 9,073,768 B | **+8.1 MiB** |
| rten with `hf-hub` 0.5 | 11,533,448 B | +10.4 MiB |
| candle with `hf-hub` 0.5 | 9,641,824 B | +8.6 MiB |
| tract with `hf-hub` 0.5 | 35,721,584 B | +33.5 MiB |

The size change of the real `htui` binary was not measured. It shares tokio, TLS and serde with the
rest of htui, so the spike deltas overstate it.

### 5.4 Speed and memory

Medians, 4 threads, same window (§5.1). **per-item** is htui's real shape (88 calls, at most 4
texts each); **batch95** is the whole corpus in one call, padded to 95×512:

| Backend | load | **per-item** | batch of one | sorted, chunks of 32 | batch95 | query median / p90 | per-item, 1 thread |
|---|---|---|---|---|---|---|---|
| fastembed/ort | 247 ms | **1,885 ms** (50 texts/s) | 2,064 ms | 3,966 ms | 8,798 ms | **2.3 / 2.7 ms** | 6,921 ms |
| **rten** | **73 ms** | 2,431 ms (**1.29×**) | 2,480 ms (1.20×) | 4,349 ms (1.10×) | **8,525 ms (0.97×)** | 5.6 / 6.8 ms | 8,830 ms (1.28×) |
| tract (symbolic shapes) | 308 ms | 6,850 ms (3.63×) | 6,801 ms | 14,632 ms | 26,824 ms (3.05×) | 6.0 / 7.9 ms | 11,429 ms |
| candle, patched | 92 ms | 8,140 ms (4.32×) | 8,400 ms | 24,600 ms | 54,955 ms (6.25×) | ~8 ms | 13,154 ms |

| Peak RSS | load | per-item | sorted | batch95 |
|---|---|---|---|---|
| fastembed/ort | 258 MB | 309 MB | 1,834 MB | **5,879 MB** |
| **rten** | 148 MB | 182 MB | 631 MB | **1,584 MB** |
| tract | 283 MB | 283 MB | 989 MB | 2,676 MB |
| candle | 268 MB | 268 MB | 1,524 MB | 4,289 MB |

Notes:

- **Padding is the whole story at batch95.** One 512-token text pads every row to 512, and the
  attention scores for 95×12 heads×512×512 in f32 are 1.2 GB per layer. On CPU, small calls win
  for every backend; a batch of one is within 10% of per-item. htui's per-item shape is already
  good. Its requirement chunks of up to 256 (`vector_sync.rs:168`) are not: one long rationale
  pads the chunk, and with fastembed that chunk would reach the 5.9 GB row above.
- **candle's numbers are after two fixes.** Stock 0.9.2 `bert.rs` used about 1.4 of 4 cores: the
  generic softmax, the mask `broadcast_add` and `gelu_erf` run on one thread, and at L=512 they
  take about 80% of a layer. Vendoring `bert.rs` to use `softmax_last_dim` and to skip the mask add
  on unpadded batches was bit-exact and made it 1.7–2× faster. It is still the slowest.
- **tract**: symbolic `B`/`S` dimensions, optimised once (about 60 ms), beat per-shape
  concretisation (50–93 ms per new shape, no throughput gain). tract is single-threaded unless
  `tract-linalg/multithread-mm` is on and an executor is set (55.8 s against 31.1 s at batch95).
- **rten**: its default thread pool sizes by physical cores (6 here) and ignores the cgroup quota
  (from rten's source, not measured), so the spike passes an explicit pool sized by
  `available_parallelism()`.
- Interactive cost: a search embeds one query. 5.6 ms against 2.3 ms is invisible next to the
  Qdrant round-trip. Re-indexing the 95-text corpus takes 2.4 s against 1.9 s.

### 5.5 Weights at run time

| | Files fetched | From | Pinned | Cache | Empty cache, no network |
|---|---|---|---|---|---|
| fastembed | `model.onnx` + 4 JSONs (133.8 MB) | `Xenova/…` `main` via `hf-hub` 0.3.2 | no | `~/.cache/htui/fastembed`, HF layout | **panic**; and with the network up, the relative-redirect failure (§2.4) |
| rten / tract | same 5 files | same repo | spike: no; MOD-68: yes | spike: HF layout; **reads fastembed's cache as-is** | clean `Err` |
| candle | `model.safetensors` + 3 JSONs (134 MB) | `BAAI/…` | spike: commit `5c38ec7c` | separate; **a second download** for every existing user | clean `Err` |

Measured: `hf-hub` 0.5 resolved the relative `307`s and filled an empty cache, byte-identical to
fastembed's. A tract binary pointed at a copy of fastembed's cache, with the network blocked,
loaded from `refs/main` → `snapshots/ea104dac…` in 0.1 ms and wrote nothing. A cold download took
19–21 s on this link.

### 5.6 Vectors against the stored ones

| Backend | 1 − min cosine (107 texts) | max element diff | top 10, mixed (new query vs stored docs) | top 10, all-backend |
|---|---|---|---|---|
| candle | 3.3e-13 | 3.3e-7 | 12/12 identical order, 0 rank moves | 12/12 |
| **rten** | **3.9e-13** | **3.6e-7** | 12/12, 0 moves, largest score change 2.1e-7 | 12/12 |
| tract | 9.6e-13 | 4.8e-7 | 12/12, 0 moves | 12/12 |

The closest pair of adjacent baseline scores in any top 11 is 6.56e-6 apart (the query `MOD-34`),
about 30 times the largest score change. No rank can flip. Each backend reproduces its own vectors
bit for bit across runs and batch shapes, and per-item, sorted and batch-of-one vectors equal
batch95's (rten: bit-identical). The differences are f32 accumulation order in the matmul, GELU and
layer-norm kernels, not tokenisation: a token mismatch would push cosine to about 0.99 or below.
Norms stay within [0.9999996, 1.0000003].

**Answer: no re-embed.** Vectors stored by fastembed and vectors produced by rten are
interchangeable at any tolerance down to 1e-6 per element (or 1 − 1e-12 cosine), and a mixed index
ranks exactly as a pure one.

## 6. Options

| | Offline build | Native toolchain | Size | Speed (per-item) | Memory | Weights | Re-embed |
|---|---|---|---|---|---|---|---|
| Keep fastembed, pin `ORT_LIB_LOCATION` | only with a hand-fetched library | C, OpenSSL, libstdc++ | +24.1 MiB | 1.0× | worst | broken on fresh installs until `hf-hub` is bumped, which `fastembed` 3.14 pins | — |
| Upgrade to fastembed 7.x | no (`ort` rc.13 still downloads or dlopens) | same | ~same | ~1.0× | same | fixed upstream | likely none |
| **rten** | **yes** | **none** | **+8.1 MiB** | **1.29×** | **best** | same file, same cache | **none** |
| tract | yes | C + asm | +33.5 MiB | 3.63× | middle | same file, same cache | none |
| candle 0.9.2 | yes | none | +8.6 MiB | 4.32× (patched) | poor at big batches | different repo, second download | none |
| candle `main` (git dep) | yes | none | ~same | ~same | — | — | none; and a git dependency |

The first two keep a native runtime and fail the maintainer's ask. tract meets it but is the
largest and second slowest. candle meets it only on an old release or a git dependency, needs its
attention patched to reach 4.3×, and would make every existing user download the weights again.

## 7. Verdict

### 7.1 Engine: rten

`rten` 0.26 (`default-features = false, features = ["onnx_format"]`; the `.rten` flatbuffers
loader and `contrib` ops are not needed) runs `Xenova/bge-small-en-v1.5` `onnx/model.onnx`
unmodified. Tokenisation is `tokenizers` 0.23.2 on `fancy-regex`, configured exactly as fastembed
does (§2.2). It is the only candidate whose embedder needs no C compiler, and it is the smallest
and lightest. It is close to ONNX Runtime on htui's real shape, beats it on large batches, and
loads in a third of the time. Query latency is its one cost and does not matter for a single
interactive query.

Implementation facts the spike established:

- rten tensors carry no `i64`: feed `input_ids`, `attention_mask` and `token_type_ids` as
  `NdTensor<i32, 2>` (ids < 30,522, so no loss; rten casts the graph's `int64` initialisers
  itself).
- `RunOptions` is `#[non_exhaustive]`: build it with `RunOptions::default()` and assign
  `thread_pool`.
- Pass an explicit `rten::ThreadPool` sized by `std::thread::available_parallelism()`; rten's
  default ignores cgroup quotas.
- Take the graph's single output as `last_hidden_state`. Slice `[:, 0, :]`, then
  `v / (‖v‖ + 1e-12)`.
- `tokenizers` 0.19 → 0.23 API changes: `add_special_tokens` takes an iterator and returns
  `Result`; `with_padding`/`with_truncation` return the impl type.

### 7.2 Weights: pinned fetch over `reqwest`, with cache reuse

The model files are fetched at run time, on first use, from
`https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/<file>`.
That is htui's existing `reqwest` 0.13 (rustls + ring, with the explicit provider install that
`htui-agent`'s `install/http.rs` does). `htui-store` gains the workspace dependency, and no crate is
added to the lock. **Each file is checked against a sha256 compiled into htui**, written to a
temporary name and renamed into place only when it verifies. Only `model.onnx` and
`tokenizer.json` are needed: the tokenizer settings fastembed read from the three small JSONs
(`max_length` 512, `[PAD]`/0, the special tokens) become constants beside the hashes. Before
downloading, an existing fastembed snapshot under
`<cache>/htui/fastembed/models--Xenova--bge-small-en-v1.5/snapshots/*/` is used when its files
hash to the pinned values, so no existing user downloads 133 MB again.

This answers "how and when": once, lazily, on the first index or search, from a single pinned
commit, verified. An unreachable host, a hash mismatch or a missing file is a `StoreError` naming
the file and the URL, never a panic (`R-STO-8`). `hf-hub` was rejected because its revision pin is
untested against the existing cache, and it would add `ureq` 3 to the build for three GETs.

### 7.3 Stored vectors: keep them, and record the model from now on

No re-embed and no collection rename: §5.6 shows the stored vectors are interchangeable. What §2.3
found missing is the record of which model made them. MOD-68 records the embedder's identity (repo,
commit, `model.onnx` sha256, dim, pooling) and checks it whenever it connects. Today's identity
equals fastembed's. Any later model change then shows up as a deliberate re-index, not a silent
mix. MOD-68's plan decides where the identity is stored (a Postgres `app_setting` row, or the
collection).

### 7.4 What the swap also fixes

- The fresh-install failure (§2.4) and the load-time panic.
- `ort-sys`'s build-time download, and with `fastembed` gone: `ort`, `ort-sys`, `onig`/`onig_sys`,
  `esaxx`'s C++ path, `hf-hub` 0.3.2, `ureq` 2 and the OpenSSL headers `native-tls` needed
  (unless something else in the tree still pulls them). The `local-embed` feature comment that
  points at `ORT_LIB_LOCATION` goes too.

### 7.5 Fallback

If rten fails on a platform that `R-NF-1` requires (§8), tract-onnx is the fallback. It loads the
same file and cache and passes the same vector check, at 3.6× the indexing time and 33.5 MiB, and
it needs an assembler. Swapping between them is local to the `DenseEmbedder` implementation.

## 8. MOD-68 and its phasing

One implementing item, **MOD-68** (from ANA-23), one milestone. The tasks, in order:

1. **Goldens first, while fastembed still builds.** Record fastembed's vectors for a handful of
   fixed texts (short, long, truncated, Unicode) as a test fixture. The new embedder must match
   them within 1e-5 per element. This is the test that pins §5.6 for good.
2. **`RtenEmbedder`** behind the existing `DenseEmbedder` seam (`embed.rs`). It takes a model
   directory and has no network code. Per-call batching caps padded tokens (the batch-of-one shape
   is within 10% of the best on CPU; §5.4). The explicit thread pool and the `i32` inputs as in §7.1.
3. **Model fetch** (§7.2): the pinned URLs, sha256s and tokenizer constants in one place, fastembed
   cache reuse, temporary file and rename, typed errors. The three construction sites
   (`concepts.rs` `open`/`open_index`, `concepts_worker.rs` `load_model`) call it off the UI thread
   (`R-NF-3`).
4. **Model identity** recorded and checked on connect (§7.3).
5. **Remove `fastembed`** from the workspace and the `local-embed` feature's dependency (the
   feature can stay as the switch for the rten stack). Check whether the workspace `ureq` entry
   still has a user. Update `docs/decisions` cross-references and the stale
   "`FastEmbedder` is not Clone" comment (`concepts.rs:313`).

It touches about 8 files: `embed.rs`, a new fetch module, `concepts.rs`, `concepts_worker.rs`,
`vector.rs` (identity), the two `Cargo.toml`s, `Cargo.lock`, and the test fixture. Not blocked.

## 9. Risks and open questions

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| rten fails to build or run on Windows/macOS (`R-NF-1`); only Linux x86-64 was measured | Low (pure Rust, runtime SIMD dispatch, no `build.rs`) | High | MOD-68 builds the embedder on all three in CI or by hand before removing fastembed; tract is the fallback (§7.5); MOD-16 (Windows verification) covers the binary |
| Vectors drift between CPU paths (AVX2 against AVX-512) | Low | Low | Measured at 1e-7 within one machine; the golden test (§8.1) runs on every machine that runs the suite |
| HuggingFace changes its URL scheme again | Medium | Medium (fresh installs only) | A pinned commit URL plus a hash check fails loudly and names the file. A mirror base URL (setting or env) is a cheap follow-up if it bites |
| Users' stored vectors came from an older Xenova revision than `ea104dac` | Low | Low | Not checked. Any older ONNX export of the same BAAI weights differs by float noise at most, and the identity record makes the next change explicit |
| rten is a single-maintainer project | Medium | Medium | The seam is one trait; tract runs the same file |
| A requirements chunk of 256 with one long text pads the whole forward pass | Medium | Medium (memory) | The per-call token cap of §8.2 |

Not measured: the real `htui` binary's size change, `cargo audit` over the candidate trees (htui has
no `deny.toml`), the int8 and fp16 ONNX variants (they would change the vectors and force a
re-embed, so they are out of scope), and a real user's Qdrant collection.

## 10. Decisions taken (maintainer, 2026-10-01)

1. **Engine: rten**, accepted over the two named candidates on the §5 evidence.
2. **Weights: pinned fetch over htui's `reqwest`** with per-file sha256 and fastembed cache reuse,
   not `hf-hub`.
