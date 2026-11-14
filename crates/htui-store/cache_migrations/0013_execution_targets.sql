-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-130 M1 (migrations/0037_execution_targets.sql; D15, plan P7): target is
-- mirrored as a full replace, like workspace, and project, item and run gain the level columns.
-- Type mapping as 0001_mirror.sql: UUID -> TEXT, JSONB -> TEXT, TIMESTAMPTZ -> INTEGER
-- microseconds through ts_bind / ts_col; no CHECKs and no foreign keys. CACHE_MIGRATOR runs before
-- the schema_version check, so this must apply on a populated 0012 file; schema_version moves to
-- 37 through PgStore::schema_version() and forces a full rebuild, so there is no backfill. Only
-- this box is mirrored and it is a machine, so box is unchanged; step_graph is not mirrored.
-- ------------------------------------------------------------------------------------------------
CREATE TABLE target (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, locator TEXT NOT NULL DEFAULT '{}',
    box_id TEXT, probed_at INTEGER, probe_error TEXT, retired_at INTEGER,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
ALTER TABLE project ADD COLUMN target_id TEXT;
ALTER TABLE item    ADD COLUMN target_id TEXT;
ALTER TABLE run     ADD COLUMN target_id TEXT;
ALTER TABLE run     ADD COLUMN target_source TEXT;
