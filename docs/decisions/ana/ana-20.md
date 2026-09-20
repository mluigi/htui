# ANA-20 - Research Qdrant features (done, 2026-09-20)

Research how to implement, if useful, all the features of Qdrant (specifically ops optimization and FastEmbed) to update MOD-34.

Analysis concludes that `htui`'s dataset ($N < 10^5$) does not warrant custom ops optimization or vector quantization. However, relying on an external LLM API (as previously decided in ANA-19) introduces unnecessary latency and dependencies. We adopt local ONNX embeddings via `fastembed-rs`, hybrid search (Dense + BM25 sparse vectors) for exact identifier matching, and a single-collection multitenancy strategy using payload indexing.

Verdict is beneficial for architectural refinement. MOD-34 is updated to reflect these findings.

See `docs/ANA-20.md` for the full analysis.
