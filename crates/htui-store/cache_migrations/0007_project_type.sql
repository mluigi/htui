-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-146 (plan D6): project.type (migrations/0020_project_type.sql). TEXT, same
-- default, no foreign key: project_type is not mirrored. cache_meta.schema_version moves to 20
-- through PgStore::schema_version(), which forces a full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE project ADD COLUMN type TEXT NOT NULL DEFAULT 'software';
