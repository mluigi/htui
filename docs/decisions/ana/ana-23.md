# ANA-23 - Pure-Rust local embedder to replace `ort`/`fastembed` (concluded, 2026-10-01)

Opened 2026-09-26 during MOD-9 milestone 3. The maintainer wants no native runtime fetched at
build time, and `htui-store`'s `local-embed` feature (`embed::FastEmbedder`, MOD-34,
`docs/decisions/mod/mod-34.md`) pulls `ort`, whose `ort-sys` build script downloads ONNX Runtime
from `parcel.pyke.io`. Addresses `R-STO-8`, `R-NF-1`, `R-NF-2`. Analysis: `docs/ANA-23.md`.

**Verdict (`docs/ANA-23.md` §7).** Replace `fastembed`/`ort` with **`rten`** 0.26, a pure-Rust ONNX
runtime that runs the **same** `Xenova/bge-small-en-v1.5` `onnx/model.onnx`, with `tokenizers`
0.23 on `fancy-regex`. The weights are fetched at run time over htui's existing `reqwest`, pinned
to commit `ea104dac`, sha256-checked per file, and an existing fastembed cache is reused. The
stored Qdrant vectors stay: the candidates match them to 1 − 1e-12 cosine with identical top-10
rankings, so **nothing is re-embedded**. From now on, the embedder's identity is recorded so that a
later model change cannot mix vectors silently. tract-onnx is the fallback; candle is rejected.

**Evidence.** Three scratch spikes and the fastembed baseline were run on a 95-text corpus: htui's
21 demo concept points plus 74 decision-doc paragraphs. All four were timed one after another in
the same window, on htui's own per-item call shape:

| | build fetch | embedder C | stripped delta | per-item | query | batch95 RSS | vectors |
|---|---|---|---|---|---|---|---|
| fastembed/ort | yes | C, OpenSSL, libstdc++ | +24.1 MiB | 1.0× | 2.3 ms | 5.9 GB | baseline |
| **rten** | no | **none** | **+8.1 MiB** | **1.29×** | 5.6 ms | **1.6 GB** | 1 − 3.9e-13 |
| tract | no | C + asm | +33.5 MiB | 3.63× | 6.0 ms | 2.7 GB | 1 − 9.6e-13 |
| candle 0.9.2 | no | none (0.10+: `onig`) | +8.6 MiB | 4.32× (patched) | ~8 ms | 4.3 GB | 1 − 3.3e-13 |

rten was not one of the two named candidates. The web survey surfaced it, and it was measured on
the same terms.

**Defect found.** A fresh install cannot load the model today. `hf-hub` 0.3.2, pinned by
`fastembed` 3.14.1, cannot follow HuggingFace's new relative `307` redirects for `tokenizer.json`
and the other small files, so `FastEmbedder::new()` fails on any machine without a populated
cache. With no network and no cache it panics instead (`docs/ANA-23.md` §2.4). MOD-68 fixes both.

**Decisions (maintainer, 2026-10-01).** rten as the engine, and a pinned fetch over `reqwest` with
sha256 checks and fastembed cache reuse rather than `hf-hub`.

**Spawned.** **MOD-68**: replace fastembed/ort with an rten embedder and a pinned weight fetch.
One milestone: record goldens with fastembed first, then `RtenEmbedder` behind `DenseEmbedder`, the
fetch, the model identity record, and finally the removal of fastembed (`docs/ANA-23.md` §8).

Commits: analysis and close-out in the commit that added this file.
