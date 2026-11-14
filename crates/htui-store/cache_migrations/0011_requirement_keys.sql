-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-149 (migrations/0034_requirement_keys.sql): requirement.number becomes nullable
-- (a nested child has a child_number instead), and the row gains refines_id and child_number. key
-- stays the value Postgres stored. requirement is dropped and recreated: SQLite cannot drop NOT NULL
-- in place (0009_workspace_scope.sql's pattern). CACHE_MIGRATOR runs before the schema_version check,
-- so this must apply on a populated 0010 file; the rows it drops come back on the full rebuild
-- schema_version 34 forces. requirement_child_counter is not mirrored.
-- ------------------------------------------------------------------------------------------------
DROP TABLE requirement;
CREATE TABLE requirement (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, area_id TEXT NOT NULL,
    area_code TEXT NOT NULL, number INTEGER, key TEXT NOT NULL,
    refines_id TEXT, child_number INTEGER,
    body TEXT NOT NULL, rationale TEXT NOT NULL DEFAULT '', priority TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active', version INTEGER NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_requirement_order   ON requirement(project_id, area_code, number);
CREATE INDEX idx_cache_requirement_refines ON requirement(refines_id);
