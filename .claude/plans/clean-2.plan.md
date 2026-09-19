# Plan: CLEAN-2 — Delete the disabled offline buffered-write path

## Tasks

1. **Delete `cache::pending` and `upload_pending`**
   - Delete `crates/htui-store/src/cache/pending.rs`.
   - In `crates/htui-store/src/cache/mod.rs`, remove `pub mod pending;`.
   - In `crates/htui-store/src/cache/refresh.rs`, remove the `upload_pending` call and its import.
   - In `crates/htui-store/src/cache/mod.rs` (or `CacheStore::open`), remove `seal_orphaned` call.
   - *Rationale*: Enough releases have passed since MOD-25 that no unuploaded buffers exist. The `pending` path can be fully removed.

2. **Remove `Writer::Buffered` and `BufferedWriter`**
   - In `crates/htui-store/src/writer.rs`, delete `Writer::Buffered` variant and `pub struct BufferedWriter` with all its methods.
   - Delete `BUFFERED_LABEL` and `BUFFERED_NOTE` constants if they are in `ui/tabs/chat/mod.rs`.
   - *Rationale*: The buffered writer machinery was kept temporarily but is now dead code.

3. **Clean up `agent_worker.rs` and `Backend`**
   - In `crates/htui/src/agent_worker.rs`, remove `matches!(writer, Writer::Buffered(_))` guards from `project_caps_for`, `quota_latch_for`, `recording_writer`. Simplify the logic (offline backends already answer `None` for `.writer()`).
   - In `crates/htui-store/src/backend.rs`, clean up comments mentioning `Writer::Buffered` and `upload_pending`.

4. **Clean up Chat UI**
   - In `crates/htui/src/ui/tabs/chat/mod.rs`, remove `ChatSessionState::buffered`, `BUFFERED_NOTE`, and the D42 header branch for buffered mode.

5. **Clean up Tests**
   - Delete `crates/htui-store/tests/writer_buffered.rs`.
   - In `crates/htui/tests/chat_offline.rs`, remove ignored tests referencing `MOD-25` and `Writer::Buffered`.
   - Delete `crates/htui/tests/snapshots/chat_offline__chat_buffered.snap`.
   - In `crates/htui/src/agent_worker.rs`, remove the test `a_buffered_writer_never_re_probes` and clean up `a_buffered_writer_gets_no_latch` / `an_offline_backend_refuses_a_chat_with_the_unreachable_warning` if they construct `Writer::Buffered`.
   - In `crates/htui/src/testkit.rs`, remove `Writer::Buffered` mentions if they exist.
   - In `crates/htui-store/tests/cache.rs`, remove tests that test `pending/` (e.g., `seal_orphaned`, `upload_pending`).
   - In `crates/htui-store/tests/pg_criteria.rs`, remove tests that import `htui_store::cache::pending::*`.

## Fact Check

| Claim | Verdict | Evidence |
|---|---|---|
| "Enough releases have passed" | True | Maintainer explicitly confirmed "decision settled". Upload path can be deleted. |
| `Writer::Buffered` only constructed in tests | True | `grep` shows `Writer::Buffered` is only constructed in `agent_worker.rs` test blocks, never in production code. |
| Test deletions are safe | True | Tests explicitly test the offline buffer path which is being removed entirely. |
