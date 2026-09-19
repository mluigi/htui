# ANA-19 - Vector DB for related concepts search (done, 2026-09-19)

Research whether it would be beneficial to implement a vector DB for searching related concepts, overriding constraints on deployment simplicity to prioritize performance and scalability.

While `pgvector` offers simplicity by reusing Postgres, dedicated vector engines offer better raw latency and filtering. Qdrant emerged as the best choice because it offers top-tier performance (~3-4ms latency), exceptional metadata filtering, and is a lightweight single-binary Rust engine that can easily be added to `compose.yaml`, unlike heavier distributed systems like Milvus or managed-only services like Pinecone.

Verdict is beneficial. Spawned MOD-34 to implement Qdrant integration.

See `docs/ANA-19.md` for the full analysis.
