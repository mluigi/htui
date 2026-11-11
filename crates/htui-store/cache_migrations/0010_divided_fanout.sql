-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-139 milestone 1 (plan D2; migrations/0029_divided_fanout.sql): run_step's
-- role, parent_step_id and node_key, and run_plan. No CHECK and no foreign key, as everywhere in
-- the mirror (0001_mirror.sql); JSONB as TEXT, as trim_record and usage are mirrored. run_plan is
-- a cursor table on its own updated_at, refreshed after run_step, so an offline reader answers
-- ReadStore::run_plans from the last refresh (MOD-142's parts tree). role's DEFAULT keeps this
-- applicable over a populated 0009 file: CACHE_MIGRATOR runs before the schema_version check
-- (cache/mod.rs), and the rows it keeps go on the full rebuild below.
--
-- cache_meta.schema_version moves to 28 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN role TEXT NOT NULL DEFAULT 'step';
ALTER TABLE run_step ADD COLUMN parent_step_id TEXT;
ALTER TABLE run_step ADD COLUMN node_key TEXT;

CREATE TABLE run_plan (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL, revision INTEGER NOT NULL,
  planner_step_id TEXT NOT NULL, scope TEXT NOT NULL, status TEXT NOT NULL,
  base_position INTEGER NOT NULL, base_refs TEXT, body TEXT NOT NULL, digest TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_plan_run ON run_plan(run_id, revision);
