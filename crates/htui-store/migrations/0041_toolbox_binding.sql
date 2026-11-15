-- 0041_toolbox_binding.sql - MOD-127 milestone 1 (D3). Forward-only (R-STO-5).
--
-- skill_binding becomes toolbox_binding: the attachment of a toolbox entry to a scope. Five levels
-- now (global, workspace, project, item kind, phase), derived from four nullable keys, never stored.
-- The row carries a kind ('skill' is the only one in M1) and a ref_id naming the entry of that kind;
-- ref_id has no foreign key because the referent's table depends on the kind, so integrity is the
-- write path's existence check and delete-with-referent. Existing rows keep their ids, positions,
-- activation, pins and globs. One forward step, run before the new binary uses the table: there is
-- no dual-read window.

ALTER TABLE skill_binding RENAME TO toolbox_binding;
ALTER TABLE toolbox_binding RENAME COLUMN skill_id TO ref_id;
ALTER TABLE toolbox_binding DROP CONSTRAINT skill_binding_skill_id_fkey;

ALTER TRIGGER trg_skill_binding_updated_at ON toolbox_binding RENAME TO trg_toolbox_binding_updated_at;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_pkey TO toolbox_binding_pkey;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_project_id_fkey TO toolbox_binding_project_id_fkey;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_phase_id_fkey TO toolbox_binding_phase_id_fkey;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_activation_check TO toolbox_binding_activation_check;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_phase_needs_project TO toolbox_binding_phase_needs_project;
ALTER TABLE toolbox_binding RENAME CONSTRAINT skill_binding_glob_needs_globs TO toolbox_binding_glob_needs_globs;

ALTER TABLE toolbox_binding
    DROP CONSTRAINT skill_binding_skill_id_project_id_phase_id_key,
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill'
        CONSTRAINT toolbox_binding_kind_check CHECK (kind IN ('skill')),
    ADD COLUMN workspace_id UUID
        CONSTRAINT toolbox_binding_workspace_id_fkey REFERENCES workspace(id) ON DELETE CASCADE,
    ADD COLUMN item_kind_id UUID
        CONSTRAINT toolbox_binding_item_kind_id_fkey REFERENCES item_kind(id) ON DELETE CASCADE,
    ADD CONSTRAINT toolbox_binding_scope_shape CHECK (
        (workspace_id IS NULL OR (project_id IS NULL AND item_kind_id IS NULL AND phase_id IS NULL))
        AND (item_kind_id IS NULL OR (project_id IS NOT NULL AND phase_id IS NULL))
    ),
    ADD CONSTRAINT toolbox_binding_key
        UNIQUE NULLS NOT DISTINCT (kind, ref_id, workspace_id, project_id, item_kind_id, phase_id);

COMMENT ON TABLE toolbox_binding IS
    'attaches a toolbox entry (kind, ref_id) to a scope; the level is derived from the four scope keys';
COMMENT ON COLUMN toolbox_binding.kind IS 'the entry kind; skill is the only one so far';
COMMENT ON COLUMN toolbox_binding.ref_id IS
    'the entry of that kind; no foreign key (the referent table depends on kind), checked on write';
COMMENT ON COLUMN toolbox_binding.workspace_id IS 'set alone = workspace attachment (every project of it)';
COMMENT ON COLUMN toolbox_binding.project_id IS
    'NULL with every other key NULL = global attachment (every project); alone = project level';
COMMENT ON COLUMN toolbox_binding.item_kind_id IS 'with project_id = one item kind of that project';
COMMENT ON COLUMN toolbox_binding.phase_id IS 'with project_id = one phase of that project';
COMMENT ON COLUMN toolbox_binding.activation IS
    'always | glob | off; the most specific attachment of an entry wins (ANA-22)';
COMMENT ON COLUMN toolbox_binding.globs IS
    'effective globs: typed plus languages expanded at save; <repo>:<glob> only on project, item kind or phase rows';
