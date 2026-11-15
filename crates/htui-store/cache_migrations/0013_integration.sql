-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-133 milestone 1 (plan D5, D7; migrations/0039_integration.sql): one record per
-- finished run and non-local repo the run changed, which the mirror's readiness read consults (a
-- `done` blocker run releases its dependents only when every record of it is landed | pr_merged).
-- item_artifact_link's style (0011_item_artifact_link.sql): TEXT and INTEGER, no CHECK, no foreign
-- key. The lease columns are not mirrored: another process's lease is no use to a reader, as for
-- run.lease_owner. repo.integrate is not mirrored either: no cache reader builds a Repo.
-- The primary key's run_id prefix backs ready_case's lookup of a blocker run's records.
--
-- A new table, so this applies over a populated 0012 file: CACHE_MIGRATOR runs before the
-- schema_version check (cache/mod.rs). cache_meta.schema_version moves to 39 through
-- PgStore::schema_version(), which forces a full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
CREATE TABLE integration (
    run_id TEXT NOT NULL, repo_id TEXT NOT NULL, project_id TEXT NOT NULL, mode TEXT NOT NULL,
    remote TEXT NOT NULL, target_branch TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending',
    refusal TEXT, landed_hash TEXT, pr_ref TEXT, pr_url TEXT, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, repo_id));
