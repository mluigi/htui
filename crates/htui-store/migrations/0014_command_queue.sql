-- 0014_command_queue.sql - MOD-11 (plan D14, OQ-3). Forward-only (R-STO-5).
--
-- MOD-11 OQ-3 (plan D14): liveness for R-MCP-3's queue. A `running` row whose host crashed holds a
-- class slot until a claim of the same (box, class) sees its heartbeat older than three beats and
-- fails it. NULL on every row written before this migration and on every verify row (MOD-4 writes
-- only `done`/`failed`), so nothing existing is reaped.
ALTER TABLE command_run
    ADD COLUMN claimed_by   UUID,
    ADD COLUMN heartbeat_at TIMESTAMPTZ;
COMMENT ON COLUMN command_run.claimed_by   IS 'the claiming host''s owner id (MOD-11 OQ-3)';
COMMENT ON COLUMN command_run.heartbeat_at IS 'last beat of the claiming host; stale after 3 beats';
