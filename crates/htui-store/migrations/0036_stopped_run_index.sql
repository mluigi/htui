-- 0036_stopped_run_index.sql - MOD-125 M4 (plan D9). Forward-only (R-STO-5).
--
-- Indexes the stopped runs by the box that ran them: the column and predicate of
-- pending_resumes, which every box reads at each command poll to find the runs it resumes once
-- their workspace is resumed (MOD-125 M3). Partial, so it holds the few stopped runs, not the
-- run history. The query is unchanged, so no .sqlx entry moves.
--
-- Why it ships now (M2 review H1): htui_target_version rises only in apply_migrations, which the
-- TUI runs only over a pending schema, so the 0.2.0 version bump alone refused no 0.1.0 worker.
-- 0033 to 0035 landed on main after that bump and raised the target wherever a 0.2.0 build
-- applied them; 0036 is the migration M4 owes, raising it on any database still below.
--
-- No table, column or COMMENT ON (TABLES stays 56). Not mirrored: no cache migration. The
-- schema_version becomes 36, so each box rebuilds its mirror once.

CREATE INDEX idx_run_stopped_box ON run (executing_box_id) WHERE status = 'stopped';
