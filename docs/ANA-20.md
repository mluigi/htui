# ANA-20 - Research Qdrant features

> **Scope note:** Analysis of Qdrant features (Ops Optimization, FastEmbed, Quantization, Sparse Vectors) to determine optimal implementation patterns for `htui`'s vector search capabilities.
>
> **Requirements addressed:** `R-AGT-6`, `R-ID-7` (General context building and retrieval).
>
> **Status (2026-09-20): concluded.** Findings applied to update MOD-34.

---

## 1. Context and problem statement

MOD-34 aims to implement Qdrant for semantic search over workflow docs (`HANDOFF.md`, `docs/`). ANA-20 evaluates the specific features of Qdrant (specifically ops optimization tuning, FastEmbed integration, quantization, and sparse vectors) to determine which are strictly necessary or mathematically optimal for `htui`.

As a Senior Research Engineer, this analysis rejects hype and premature optimization, applying strict complexity bounds to architectural decisions.

---

## 2. Invariants

1. **Micro-Scale Dataset:** `htui` manages thousands of markdown items max ($N < 10^5$).
2. **Zero-Hallucination:** Search must reliably retrieve exact identifiers (e.g., "MOD-34") alongside semantic concepts.
3. **Rust Ecosystem:** `htui` is written in Rust; integrations must be native or accessible via stable IPC/RPC without Python dependencies.

---

## 3. Options and Verdicts

### 3.1 FastEmbed & Local Embeddings vs. External API
**Need:** Generate embeddings for documents. ANA-19 adopted an external LLM API. FastEmbed offers local ONNX-based execution (e.g., BAAI/bge-small).
**Critique:** Calling an external API for a local TUI introduces network latency ($O(100ms)$ per request), privacy concerns for proprietary code, and a hard dependency on an external service. FastEmbed enables sub-millisecond local embedding generation using CPU-optimized ONNX models. Using the `fastembed-rs` crate allows local, deterministic embedding generation without an external server.
**Verdict:** **Adopt FastEmbed (Rust binding).** Override the ANA-19 decision to use an external API. Use a lightweight local model (`BAAI/bge-small-en-v1.5` or `nomic-embed-text`) via `fastembed-rs` to eliminate network round-trips and external dependencies.

### 3.2 Ops Optimization & Read-Write Contention
**Need:** Tuning Qdrant's background optimizer for segment merging and indexing.
**Critique:** The documentation on ops optimization focuses on tuning `memmap_threshold`, `indexing_threshold`, and thread counts to avoid read-write contention under heavy continuous ingestion (thousands of vectors/sec). `htui` ingestion is triggered by human/agent doc edits (effectively $<1$ vector/sec). The default Qdrant settings are mathematically designed for datasets larger by at least 3 orders of magnitude. Adjusting them for this scale is a premature micro-optimization that adds configuration complexity for zero measurable gain.
**Verdict:** **Reject.** Do not customize optimizer settings. Default segment parameters are asymptotically optimal for $N < 10^5$.

### 3.3 Advanced Vector Features (Quantization, Sparse Vectors, ColBERT)
**Need:** Improve search accuracy or reduce memory footprint.
**Critique:**
- **Quantization (Scalar/Product/Binary):** Reduces memory footprint by compressing `f32` vectors. However, 10,000 vectors of 384 dimensions (bge-small) use $\approx 15$ MB of RAM. Quantization would save $\sim 10$ MB at the cost of recall and CPU overhead during distance calculations. Mathematically unjustified.
- **Sparse Vectors (BM25 / SPLADE):** Qdrant supports hybrid search (Dense + Sparse). For text documents containing code and architectural identifiers, exact keyword matching is critical. Dense embeddings suffer from semantic drift where exact ID references like "MOD-34" or specific variable names are lost. Qdrant's native BM25 support via sparse vectors is formally necessary to guarantee identifier retrieval.
- **ColBERT / Late Interaction:** Multi-vector representations improve precision but explode storage ($N \times \text{tokens}$ vectors). Computationally unacceptable for this domain.
**Verdict:**
- **Adopt Hybrid Search (Dense + Sparse/BM25).** Essential for precise retrieval of specific item IDs and code tokens.
- **Reject Quantization and ColBERT.** Unnecessary complexity for the current data scale.

### 3.4 Multitenancy Strategy
**Need:** Segregate different types of data (docs vs. items).
**Critique:** Creating multiple collections incurs a fixed memory overhead per collection due to HNSW graph management. Qdrant's payload filtering (using a single collection with a `type` metadata field and a payload index) offers $O(\log N)$ filtering without the collection overhead.
**Verdict:** **Adopt Single Collection with Payload Indexing.** Store all vectors in one collection and use payload filters to separate queries.

---

## 4. Verdict

Update MOD-34 to implement **local embeddings via FastEmbed (`fastembed-rs`)**, **Hybrid Search (Dense + BM25)**, and a **Single Collection with Payload Indexing**. Ignore ops optimization tuning and quantization as they are computationally irrelevant at our data scale.

---

## 5. Phasing (MOD spawn plan)

This analysis updates the existing **MOD-34**.
