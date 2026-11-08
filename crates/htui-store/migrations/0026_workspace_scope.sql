-- 0026_workspace_scope.sql - MOD-147 milestone 1 (ANA-35 7.5, 7.9 P4, 7.10 row 4; plan D2, D3, D8,
-- D9). Forward-only (R-STO-5).
--
-- Every workspace gets one hidden scope row in project (owner_workspace_id set, type 'workspace', slug
-- '@<workspace slug>'), created by create_workspace in its own transaction and, for workspaces that
-- exist before this migration, by the bootstrap (PgStore::seed_if_empty_as), not here: the seed is
-- code (htui_core::seed). The row never joins workspace_project. Three facts are tied by two CHECKs:
-- owner_workspace_id set <=> type 'workspace' <=> slug starting with '@'.
--
-- Documents gain project_id on every row and a nullable item_id (ANA-35 D2): an item document names
-- both (fk_document_item_project keeps the copy honest), a project's or a workspace's names only the
-- owner. Versions are per (owner, item, kind): uq_document_owner_kind_version, NULLS NOT DISTINCT
-- (PostgreSQL 15+). The old (item_id, kind, version) unique was the only index leading with item_id,
-- so idx_document_item replaces it; the new unique leads with project_id and serves owner reads.
-- uq_document_id_project is the target MOD-114's attachments reference.
--
-- workspace_box_path.root_path becomes folder_path; an absent row means ~/workspaces/<slug>
-- (htui_core::model::workspace_folder). No COMMENT ON (the column-comment pins stay), no new table
-- (TABLES stays 51). The mirror gains project.owner_workspace_id and the new document shape in
-- cache_migrations/0009_workspace_scope.sql; schema_version becomes 26, so each box rebuilds its mirror
-- once. A headless worker never migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. Pre-check: '@' slugs and the 'workspace' type are reserved for scope rows
-- --------------------------------------------------------------------------------------------

DO $$
DECLARE
    offenders TEXT;
BEGIN
    SELECT string_agg(p.slug, ', ' ORDER BY p.slug)
      INTO offenders
      FROM project p
     WHERE p.slug LIKE '@%' OR p.type = 'workspace';
    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION 'MOD-147 (0026_workspace_scope): slugs starting with @ and the type workspace are reserved for workspace scope rows, and these projects use them: %. Rename them and change their type with the previous htui, then start this one again.', offenders
            USING ERRCODE = 'check_violation';
    END IF;
END
$$;

-- --------------------------------------------------------------------------------------------
-- 2. project: the scope row
-- --------------------------------------------------------------------------------------------

ALTER TABLE project
    ADD COLUMN owner_workspace_id UUID
        CONSTRAINT uq_project_owner_workspace UNIQUE
        CONSTRAINT fk_project_owner_workspace REFERENCES workspace(id) ON DELETE CASCADE;
ALTER TABLE project ADD CONSTRAINT chk_project_scope_row
    CHECK ((owner_workspace_id IS NULL) = (type <> 'workspace'));
ALTER TABLE project ADD CONSTRAINT chk_project_scope_slug
    CHECK ((owner_workspace_id IS NULL) = (slug NOT LIKE '@%'));

-- --------------------------------------------------------------------------------------------
-- 3. document: project_id on every row, item_id nullable
-- --------------------------------------------------------------------------------------------

ALTER TABLE item ADD CONSTRAINT uq_item_id_project UNIQUE (id, project_id);

ALTER TABLE document
    ADD COLUMN project_id UUID
        CONSTRAINT fk_document_project REFERENCES project(id) ON DELETE CASCADE;
UPDATE document d SET project_id = i.project_id FROM item i WHERE i.id = d.item_id;
ALTER TABLE document ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE document ALTER COLUMN item_id DROP NOT NULL;
ALTER TABLE document ADD CONSTRAINT fk_document_item_project
    FOREIGN KEY (item_id, project_id) REFERENCES item(id, project_id) ON DELETE CASCADE;
ALTER TABLE document ADD CONSTRAINT uq_document_id_project UNIQUE (id, project_id);
ALTER TABLE document DROP CONSTRAINT document_item_id_kind_version_key;     -- 0001's unnamed UNIQUE
ALTER TABLE document ADD CONSTRAINT uq_document_owner_kind_version
    UNIQUE NULLS NOT DISTINCT (project_id, item_id, kind, version);
CREATE INDEX idx_document_item ON document (item_id, kind, version);

-- --------------------------------------------------------------------------------------------
-- 4. workspace_box_path: the folder
-- --------------------------------------------------------------------------------------------

ALTER TABLE workspace_box_path RENAME COLUMN root_path TO folder_path;
