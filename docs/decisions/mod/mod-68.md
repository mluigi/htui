# MOD-68 - Replace `fastembed`/`ort` with an `rten` embedder and a pinned weight fetch (done, 2026-10-01)

**Requirements:** `R-STO-8`, `R-NF-1`, `R-NF-2`, `R-NF-3`.
**Origin:** ANA-23 (`docs/ANA-23.md` §7-§9, `docs/decisions/ana/ana-23.md`).
**Artifacts:** plan with its verified-claims table and the blueprint amendments A-1..A-10:
[`.claude/plans/mod-68-rten-embedder.plan.md`](../../../.claude/plans/mod-68-rten-embedder.plan.md);
blueprint (hazards H-1..H-23):
[`.claude/plans/mod-68-rten-embedder.blueprint.md`](../../../.claude/plans/mod-68-rten-embedder.blueprint.md).
There is no PRD. The item was routed as a plan on 2026-10-01 with 0 of C1-C4 fired (C4 borderline;
the final change set is 15 files), no ultracode. It ran in a TOOL-7 sandbox (`hr/MOD-68`).
**Commits:**
- Plan and blueprint: `1ce5015` plan (25 claims checked, 2 amended), `add93eb` confirmed with the
  OQ-1/OQ-2 defaults, `88c37f3` blueprint and amendments.
- T1, goldens recorded from fastembed: `b37a4b2`.
- T2, `RtenEmbedder`: `7c4197e` (dependencies and dev-profile optimisation), `ebae287`.
- T3, fetch and construction sites: `0aeff27`, `442a832`.
- T4, model identity: `85faaa8`.
- T5, fastembed removed: `a57f650`, `98cec2a`.
- Review fixes: `47a7b5c` (M1, L5), `1dd7f94` (L2), `d44d3b7` (L3), `644a370` (L1, NITs),
  `6500377` (L4, L6).
- Then this write-up.

## What shipped

- **Embedder.** `htui_store::embed::RtenEmbedder` replaces `FastEmbedder` behind the same
  `DenseEmbedder` seam and the same `local-embed` feature. It uses `rten` 0.26
  (`onnx_format` only) and `tokenizers` 0.23.2 on `fancy-regex`, running the same
  `Xenova/bge-small-en-v1.5` `onnx/model.onnx` file. It follows fastembed's contract exactly:
  - truncate at 512 tokens;
  - re-add the five special tokens;
  - feed `i32` inputs, look the output up by the name `last_hidden_state`;
  - take the CLS row and normalise with `v / (‖v‖ + 1e-12)`.

  Each forward pass is capped at 8,192 padded tokens (`MAX_BATCH_TOKENS`). Padding is applied per
  sub-batch by hand, so one long text no longer pads a whole 256-text requirement chunk. It runs on
  an explicit `ThreadPool` sized by `available_parallelism()`. The `rten*` crates and `sha2` build
  at `opt-level = 3` in dev builds: unoptimised rten is about 118 times slower (H-1).
- **Goldens.** fastembed's vectors for six fixed texts were recorded before fastembed was removed,
  in `crates/htui-store/tests/fixtures/bge_small_goldens.json`. rten matches them with a worst
  element difference of 2.98e-7 against a 1e-5 limit, and a worst cosine of 1 − 4.1e-13. It does
  so both when texts share padded passes and when each text gets a pass of its own. The ignored
  tests `rten_matches_*` hold this check permanently. **The stored Qdrant vectors stay valid;
  nothing is re-embedded.**
- **Weights** (`htui_store::model`). The model is fetched on first use, not at build time. The
  source is pinned to `Xenova/bge-small-en-v1.5` at commit `ea104dac`, over the workspace `reqwest`:
  - `model.onnx`: sha256 `828e1496…cf35`, 133,093,490 B.
  - `tokenizer.json`: sha256 `d241a60d…5c66`.

  `ensure_model()` keeps verified files in `<cache>/htui/model/bge-small-en-v1.5-ea104dac/`. If they
  are missing, it adopts a matching fastembed snapshot from `<cache>/htui/fastembed/...`: hashed
  first, then copied, never moved, with symlinked blobs followed. Only after that does it download,
  in this order:
  - stream to a locked, uniquely named `.part` while hashing, refusing anything larger than the
    pin's size;
  - `sync_all`;
  - rename only on a hash match;
  - treat a failed rename onto an already-verified file as success.

  A stale `.part` is swept only when it is older than 10 minutes **and** the sweeper can lock it.
  A live download keeps its lock, so Windows' lazy directory timestamps cannot get it deleted
  (review M1). Every failure is a `StoreError` naming the file and URL, and a mismatch names both
  hashes. Nothing panics, which fixes ANA-23 §2.4: a fresh install could not load the model through
  `hf-hub` 0.3.2. Only `ensure_model` can produce a `ModelFiles` (review L2), so the identity below
  is always that of verified bytes. The CLI prints one line before a real download starts (review
  L4).
- **Construction sites.** All three are off the UI thread and off runtime workers (`R-NF-3`): the
  CLI `concepts::open`, the index job `open_index` and the overlay's `concepts_worker::load_model`.
  Each one runs the fetch on its task, so aborting the task cancels the download, then loads on a
  blocking or `apart` thread. `open_index` keeps the loaded model across Qdrant retries.
- **Model identity.** The collection's `metadata.embedder` records repo, commit, `model.onnx`
  sha256, dimension, pooling and normalisation, through the new `DenseEmbedder::identity()`.
  `HashEmbedder` reports `hash/<dim>`. On connect the checks run in this order:
  - **Dense width first.** A wrong width is refused and never stamped.
  - **Then the identity.** A collection from before MOD-68 is stamped, which is safe because
    fastembed's identity equals rten's. Qdrant merges metadata keys on update; a live test pins
    this.
  - **A different identity is refused.** The message names both identities and the remedy: delete
    `htui_concepts_v2`, then run `htui --index-items`.

  `vector::is_embedder_mismatch` lets the three sites drop the "cannot reach Qdrant" prefix for
  this case. The index job logs the mismatch once at `error`, then at `debug` each interval
  (review L3). A server too old for collection metadata gets one `warn` saying the check is off.
- **Removed.** `fastembed`, `ort`, `ort-sys`, `hf-hub` 0.3.2, `onig`/`onig_sys`, `tokenizers`
  0.19.1, ureq 2.12.1 and the unused workspace `ureq` entry: 33 lock packages, none added. The
  README now describes the first-use fetch and an offline install (place the two pinned files in the
  model directory). The `ORT_LIB_LOCATION` section is gone.

## Decisions

The plan's D1-D10 and the blueprint's A-1..A-10 hold, as recorded in the plan. The ones that will
matter later:

- **The identity lives in Qdrant collection metadata, not Postgres** (D8). It describes these
  vectors and disappears with the collection.
- **OQ-1 (maintainer, 2026-10-01): the Windows/macOS gate was a cross `cargo check`.** A scratch
  crate with the workspace's exact `rten`/`tokenizers` specs is green for
  `x86_64-pc-windows-msvc`, `aarch64-apple-darwin` and `x86_64-apple-darwin`, and no `cc`, `cmake`,
  `onig_sys` or `bindgen` appears for any target. The real run on those platforms belongs to MOD-16
  (note added there).
- **OQ-2 (maintainer): a mismatch is refused with the delete-and-reindex remedy.** There is no
  `--reindex` flag.
- **`openssl-sys` stays.** `sentry`'s default transport enables reqwest's `native-tls`. ANA-23
  §7.4 expected it to go.

## Verification

- Baseline before the change: 3,352 passed, 0 failed.
- Final gate on `6500377`:
  - `cargo fmt --check` clean.
  - `cargo clippy --workspace --all-targets --all-features -D warnings` clean.
  - `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`: **3,401 passed,
    0 failed**, with no Qdrant skip lines.
- The MOD-68 ignored tests all pass: both golden layouts, empty input and
  `real_download_from_huggingface` (21.5 s, both files verified against their pins). The only
  ignored failure is `secret::tests::the_real_backend_round_trips_too`, which needs a real OS
  keyring the sandbox lacks. It is untouched by this item.
- Adoption of a fastembed snapshot made zero HTTP requests. This was checked with every proxy
  variable pointed at a dead port.
- Review: `rust-reviewer` found no CRITICAL or HIGH issues, one MEDIUM (M1, Windows `.part`
  sweep), six LOW and a set of NITs. All were applied except the three below.

## Not done here / accepted

- **Accepted at review (maintainer, 2026-10-01).** Three NITs are left as they are:
  - `htui worker`'s exit can wait on a blocking-pool model hash or copy (seconds, bounded).
  - `an_unreachable_host_is_an_error_not_a_panic` binds and drops a port, a negligible reuse race.
  - `a_stored_qdrant_url_starts_the_job` stays offline only because its test runtime is
    current-thread.
- **Windows behaviour of the `.part` lock and rename was exercised on Linux only.** It is MOD-16's.
- **Not changed (outside the findings).** A mismatch from `--index-items` exits 1 and is still
  reported to Sentry.
- **The real `htui` binary's size change was not measured** (ANA-23 §9).
