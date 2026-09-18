-- ------------------------------------------------------------------------------------------------
-- Mirror side of ANA-2 (MOD-4 milestone 1, plan D7 / D9; `docs/ANA-2.md` 9's sketch corrected).
--
-- `run` and `run_step` gain the columns `migrations/0003_orchestration.sql` adds to them, minus
-- `run.lease_owner`, a liveness token for a process that is by definition not running while the
-- mirror is read. `run_step_tree` is the seventeenth mirrored table: no `updated_at` and no cursor
-- of its own, it rides its parent step's `updated_at` exactly as `run_step_commit` does.
-- `box.probed_tags`, `box.declared_tags` and `box.settings` are already in `0001_mirror.sql`.
--
-- `cache_meta.schema_version` moves to 3 through `PgStore::schema_version()`, which forces a
-- full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run      ADD COLUMN repo_scope       TEXT NOT NULL DEFAULT '[]';  -- JSON array of uuids
ALTER TABLE run      ADD COLUMN lease_box_id     TEXT;
ALTER TABLE run      ADD COLUMN lease_expires_at INTEGER;
ALTER TABLE run_step ADD COLUMN verify_outcome   TEXT;
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at      INTEGER;

CREATE TABLE run_step_tree (
    run_step_id TEXT NOT NULL, repo_id TEXT NOT NULL, mode TEXT NOT NULL,
    path TEXT NOT NULL, base_ref TEXT NOT NULL, dirty INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_step_id, repo_id));
