-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-125 milestone 1 (plan D3): run.workspace_id, the workspace a graph run was
-- started from (migrations/0019_stop_runs.sql). TEXT, no foreign key, as 0001 mirrors every UUID.
-- The 'stopped' status needs nothing here: the mirror has no CHECKs (0001_mirror.sql). Neither
-- queue_entry.workspace_id nor workspace_stop is mirrored.
--
-- cache_meta.schema_version moves to 19 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run ADD COLUMN workspace_id TEXT;
