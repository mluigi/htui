-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-138 milestone 2 (plan D5): run_step.branch, the readiness walker's `if`
-- decision (migrations/0025_run_step_branch.sql). TEXT holding the JSON, as trim_record and usage
-- are mirrored; no CHECK.
--
-- cache_meta.schema_version moves to 25 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN branch TEXT;
