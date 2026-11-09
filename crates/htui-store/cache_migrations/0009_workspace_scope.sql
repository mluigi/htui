-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-147 milestone 1 (migrations/0026_workspace_scope.sql): project.owner_workspace_id
-- (the scope row, looked up by ReadStore::scope_project) and document's new shape, project_id on every
-- row with a nullable item_id. document is dropped and recreated: SQLite cannot drop NOT NULL in
-- place. CACHE_MIGRATOR runs before the schema_version check (cache/mod.rs:138), so this must apply on
-- a populated 0008 file; the rows it drops come back on the full rebuild schema_version 26 forces.
-- workspace_box_path is not mirrored, so its rename has no mirror side.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE project ADD COLUMN owner_workspace_id TEXT;
CREATE INDEX idx_cache_project_owner ON project(owner_workspace_id);

DROP TABLE document;
CREATE TABLE document (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, item_id TEXT, kind TEXT NOT NULL,
    version INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL, produced_by_step_id TEXT,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_document_item    ON document(item_id, kind, version);
CREATE INDEX idx_cache_document_project ON document(project_id, kind, version);
