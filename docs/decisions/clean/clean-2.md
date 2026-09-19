# CLEAN-2 - Delete the disabled offline buffered-write path (done, 2026-09-19)

**What was done:**
Deleted the disabled offline buffered-write path across the tree. This path was previously disabled by MOD-25 but deliberately kept compiling for one release to make reversal cheap. Since the decision has settled, the dead code was fully removed.

Specifically, the following were deleted:
- `Writer::Buffered` arm and `BufferedWriter` in `crates/htui-store/src/writer.rs`.
- `cache::pending` module (`append_pending`, `seal_pending`, `upload_pending`, `seal_orphaned`).
- The `matches!(Writer::Buffered(_))` guards and offline latch logic in `crates/htui/src/agent_worker.rs`.
- The `ChatSessionState::buffered` and its UI rendering branches in `crates/htui/src/ui/tabs/chat/mod.rs`.
- All associated tests and snapshots (e.g., `writer_buffered.rs`, pending cases in `tests/cache.rs` and `tests/pg_criteria.rs`, and offline buffered chat cases).

**Commit:**
`92f3c48` - CLEAN-2: Delete the disabled offline buffered-write path
