# ANA-14 - Research whether using Redis could be beneficial (rejected, 2026-09-19)

**What was decided:** Redis will not be introduced into the `htui` stack.

**Why:**
The introduction of Redis explicitly violates `R-NF-2` ("No dependency on any external daemon other than Postgres and the agents") and dilutes `R-ID-3` (Postgres as the single source of truth). The capabilities Redis would provide—Pub/Sub for cross-box synchronization, task queueing for remote dispatch (`R-ORCH-12`), and ephemeral caching—can be effectively handled by Postgres (`LISTEN`/`NOTIFY`, `SKIP LOCKED`) and the existing local SQLite cache mirror. Introducing Redis would increase operational complexity without corresponding feature justification.

**Commit hashes:** (N/A, documentation only)
