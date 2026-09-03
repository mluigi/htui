# ANA-1 - Data model, box registry and sync topology (done, 2026-09-03)

## Summary

Concluded design for `htui`'s persistent storage, box registry, and synchronization architecture, documented in [`docs/ANA-1.md`](file:///D:/projects/htui/docs/ANA-1.md).

## What was decided

1. **Storage Canonicity & Decoupled Trackers:**
   - **Postgres** is canonical for work items, revision history (`item_revision`), box profiles, execution pipelines and steps (`run`, `run_step`), phase documents (`item_document`), atomic repository facts (`artifact`), and the fact knowledge graph (`artifact_link`).
   - **`items.json`** in managed repos is a derived, merge-friendly offline snapshot (UUID, key, title, status only; no bodies, discussions, or facts).
   - **External Issue Tracker** is canonical for human-facing story, comments, and discussion threads, pluggable and decoupled pending `ANA-6`.
   - **Zero-agent sync invariant:** sync is executed strictly by deterministic API clients without LLM token burn.

2. **Schema, Multi-Agent Steps & Living Knowledge Graph:**
   - Complete PostgreSQL DDL specified in `docs/ANA-1.md` covering `repo`, `box`, `item`, `item_revision`, `item_link`, `run`, `run_step`, `item_document`, `artifact`, `artifact_link`, `sync_state`, and `external_issue_mapping`.
   - Execution split into pipeline container (`run`) and per-phase agent step (`run_step`), capturing specific agent models, commit hashes, prompt digests, and exit statuses.
   - Separation of macro phase documents (`item_document`: PRD, Plan, Review, Summary) from atomic repository facts (`artifact` with 1–10 `importance` and `commit_hash`).
   - Directed edge table `item_link` uses explicit `blocked_by` semantics; 1–2 hop upstream prerequisites are queried via recursive SQL CTE to inject active summaries and high-importance facts (`importance >= 7`).

3. **Box Registry & Prompt Injection:**
   - Host machine toolchains, compilers (Rust, GCC/MinGW, Clang, MSVC), build tools (CMake, Ninja, vcpkg), shells, and OS quirks are probed on startup and injected as `<HOST_ENVIRONMENT>` into agent prompts, eliminating cross-box build discrepancies.

4. **Transcript Split & Local Secret Scrubbing:**
   - Two destinations: distilled step summaries are posted as issue comments; full raw execution streams are stored locally and uploaded as attachments (generic URI `transcript_ref`).
   - All transcripts are scrubbed of API keys, tokens, and environment secrets locally in Rust before leaving the machine.

5. **Concurrency, Divergence & Application-Managed Revisions:**
   - Optimistic locking via `version` on `item`. Full mutation history recorded in `item_revision` by the application layer (`htui`) with machine ID and reason.
   - Concurrent conflicting edits trigger explicit divergence detection, powered by a 3-way diff against common ancestor revisions.

## Downstream items

Implementation is tracked across three spawned modules:
- **MOD-6:** Item store: `items.json` + Postgres
- **MOD-7:** Box registry + prompt injection
- **MOD-5:** Issue tracker sync (pending `ANA-6`)

