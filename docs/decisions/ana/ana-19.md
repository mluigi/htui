# ANA-19 - Vector DB for related concepts search (done, 2026-09-19)

Research whether it would be beneficial to implement a vector DB for searching related concepts, updating the Postgres to pgvector.

Adding a vector database adds significant operational overhead. Since `htui` already relies on PostgreSQL (`PgStore`), adding the `pgvector` extension allows unified relational and vector storage without introducing new external infrastructure.

Verdict is beneficial. Spawned MOD-34 to implement pgvector integration.

See `docs/ANA-19.md` for the full analysis.
