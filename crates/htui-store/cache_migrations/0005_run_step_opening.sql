-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-37 milestone 5 (R-48): run_step.opening, the way a promoted step's chat opened
-- (migrations/0014_run_step_opening.sql). TEXT with no CHECK, as 0003 mirrors verify_outcome.
--
-- cache_meta.schema_version moves to 14 through PgStore::schema_version(), which forces a full
-- rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN opening TEXT;
