# ANA-14 - Research whether using Redis could be beneficial

> **Scope note:** Design authority for evaluating whether introducing Redis into the `htui` architecture would provide benefits that justify its cost. This covers its potential use for Pub/Sub (cross-box synchronization), task queueing (auto mode remote dispatch), and caching. Governed by `CONCEPTS.md` and `docs/REQUIREMENTS.md`.
>
> **Requirements addressed:** `R-NF-2`, `R-ID-3`, `R-ORCH-12`.
> 
> **Status (2026-09-19): concluded, rejecting Redis.**

---

## 1. Context and problem statement

`htui` currently operates with Postgres as the single source of truth (`R-ID-3`) and a per-box SQLite read-only cache/mirror. The orchestrator runs locally, and multi-box synchronization relies on the Postgres database. 
As the architecture grows to support remote dispatch (queueing an item from one box to run on another, `R-ORCH-12`) and potential real-time UI updates across different boxes used by the same developer (`R-USR-1`), the question arises: **Would introducing Redis as an external daemon be beneficial for `htui`?**

Redis is commonly used in similar architectures for:
1. **Pub/Sub**: Real-time events to tell connected boxes that a run step finished or an item was updated.
2. **Task Queueing**: A reliable queue for auto mode (`R-ORCH-6`) and remote dispatch (`R-ORCH-12`).
3. **Session/Cache State**: Ephemeral state for the TUI or agents.

## 2. Invariants

Three requirements strictly constrain this decision:

1. **R-NF-2 (must):** "No dependency on any external daemon other than Postgres and the agents."
2. **R-ID-3 (must):** "Postgres is the single source of truth. Everything `htui` knows lives there..."
3. **R-ORCH-12 (later):** "Remote dispatch: queue an item from one box to run on another. Requires a headless `htui` worker per box **polling Postgres**..."

## 3. Options and Evaluation

### Option A: Use Redis for Pub/Sub (Real-time sync)
- **Benefit:** Instant updates across boxes without polling.
- **Drawback:** Violates `R-NF-2`. Postgres already supports `LISTEN` and `NOTIFY`, which `sqlx` natively supports. We can achieve real-time cross-box synchronization entirely within Postgres without adding a new daemon.

### Option B: Use Redis for the Auto Mode Queue (`R-ORCH-12`)
- **Benefit:** Redis Streams or Lists are excellent for distributed task queues.
- **Drawback:** Violates `R-NF-2` and contradicts `R-ORCH-12` which explicitly dictates polling Postgres. Furthermore, Postgres can act as a highly robust queue using `SELECT ... FOR UPDATE SKIP LOCKED`, which allows multiple `htui` workers to claim runs without contention, keeping all state in the SSOT (`R-ID-3`).

### Option C: Use Redis for Ephemeral Caching
- **Benefit:** Fast reads for transcripts or agent states.
- **Drawback:** We already have a local SQLite mirror (`cache.sqlite`) on each box that fulfills `R-STO-3` (startup with a warm cache under one second). Adding Redis would replace a zero-config, embedded, and fast local cache with a network daemon, increasing operational burden for zero UX gain.

## 4. Verdict

**Rejected.** Redis will not be introduced into the `htui` stack.

**Deciding reason:**
Introducing Redis explicitly violates `R-NF-2` ("No dependency on any external daemon other than Postgres") and dilutes `R-ID-3`. The specific problems Redis solves are already solvable using the existing infrastructure:
- Pub/Sub can be handled by Postgres `LISTEN`/`NOTIFY`.
- Queueing can be handled by Postgres `SKIP LOCKED` (or standard polling as required by `R-ORCH-12`).
- Caching is already handled optimally by the local SQLite mirror.

Adding Redis would increase the operational complexity of a "local-first, developer-guided harness" (`R-ID-2`) without enabling any features that Postgres and SQLite cannot already support.

## 5. Phasing

No implementation phases or `MOD-N` items are spawned from this verdict since the outcome is to maintain the current architectural constraints.
