-- 0040_execution_targets.sql - MOD-130 milestone 1 (ANA-42 7.1; PRD D1-D5, D7, D11-D15; plan P1-P5).
-- Forward-only (R-STO-5). Depends on 0001_init.sql (box, project, step_graph, item, run,
-- set_updated_at) and 0026_workspace_scope.sql (the scope project row that holds the workspace
-- level, D5).
--
-- A target is a record that points at a box: a kind, a locator (names and addresses, never
-- secrets) and, once bound, a box. Its status is derived at read time from the kind, the bound box
-- and the stored probe columns, never stored (D4). One local target is seeded here with the id and
-- name of htui_core::seed::LOCAL_TARGET_ID and LOCAL_TARGET_NAME (P2); box_id NULL means the box
-- that starts the run, and the store refuses to create, edit, retire or delete it (D2).
--
-- A box is a machine or a container; a container is a child of a machine (D12). os_family keeps
-- its three values, and only a container may hold NULL until its first probe (D13, P3): a plain
-- IN list passes on NULL, so chk_box_os_family spells the NULL case out.
--
-- project, step_graph and item gain a nullable target_id, NULL meaning inherit (D3); a scope row's
-- project.target_id is the workspace level (D5). run gains target_id and target_source, written by
-- milestone 2 (D7). Every reference is ON DELETE RESTRICT, and the store refuses a delete in use
-- before the statement runs (D14), so no SQLSTATE of the restrict ever reaches a caller.
--
-- target is mirrored (D15) through the cache migration of the same milestone; cache_meta's
-- schema_version moves through PgStore::schema_version(), so each box rebuilds its mirror once. A
-- headless worker never migrates: migrate from a TUI first.

CREATE TABLE target (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL CONSTRAINT uq_target_name UNIQUE,
    kind        TEXT NOT NULL CONSTRAINT chk_target_kind
                CHECK (kind IN ('local','docker','ssh','ssh_docker','enrolled')),
    locator     JSONB NOT NULL DEFAULT '{}'
                CONSTRAINT chk_target_locator_object CHECK (jsonb_typeof(locator) = 'object'),
    box_id      UUID CONSTRAINT fk_target_box REFERENCES box(id) ON DELETE RESTRICT,
    probed_at   TIMESTAMPTZ,
    probe_error TEXT,
    retired_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_target_local_shape CHECK (kind <> 'local'
        OR (box_id IS NULL AND locator = '{}'::jsonb AND retired_at IS NULL))
);
CREATE UNIQUE INDEX uq_target_one_local ON target (kind) WHERE kind = 'local';
CREATE TRIGGER trg_target_updated_at BEFORE UPDATE ON target
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
INSERT INTO target (id, name, kind)
    VALUES ('00000000-0000-4000-8000-000000000001', 'this box', 'local');

ALTER TABLE box ADD COLUMN kind TEXT NOT NULL DEFAULT 'machine'
    CONSTRAINT chk_box_kind CHECK (kind IN ('machine','container'));
ALTER TABLE box ADD COLUMN parent_box_id UUID
    CONSTRAINT fk_box_parent REFERENCES box(id) ON DELETE RESTRICT;
ALTER TABLE box ADD CONSTRAINT chk_box_kind_parent
    CHECK ((kind = 'container') = (parent_box_id IS NOT NULL));
ALTER TABLE box ADD CONSTRAINT chk_box_parent_not_self CHECK (parent_box_id <> id);
ALTER TABLE box DROP CONSTRAINT box_os_family_check;
ALTER TABLE box ALTER COLUMN os_family DROP NOT NULL;
ALTER TABLE box ADD CONSTRAINT chk_box_os_family CHECK (CASE WHEN os_family IS NULL
    THEN kind = 'container' ELSE os_family IN ('windows','linux','macos') END);
CREATE INDEX idx_box_parent ON box (parent_box_id) WHERE parent_box_id IS NOT NULL;

ALTER TABLE project    ADD COLUMN target_id UUID CONSTRAINT fk_project_target    REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE step_graph ADD COLUMN target_id UUID CONSTRAINT fk_step_graph_target REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE item       ADD COLUMN target_id UUID CONSTRAINT fk_item_target       REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE run        ADD COLUMN target_id UUID CONSTRAINT fk_run_target        REFERENCES target(id) ON DELETE RESTRICT;
ALTER TABLE run ADD COLUMN target_source TEXT CONSTRAINT chk_run_target_source
    CHECK (target_source IN ('item','workflow','project','workspace','settings','default'));
ALTER TABLE run ADD CONSTRAINT chk_run_target_pair CHECK ((target_id IS NULL) = (target_source IS NULL));
CREATE INDEX idx_project_target    ON project    (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_step_graph_target ON step_graph (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_item_target       ON item       (target_id) WHERE target_id IS NOT NULL;
CREATE INDEX idx_run_target        ON run        (target_id) WHERE target_id IS NOT NULL;

COMMENT ON TABLE target IS
    'MOD-130 M1 (ANA-42 7.1): where a run executes, a record that points at a box. Kind, locator '
    'and, once bound, box; the status is derived at read time and never stored (D4). One local '
    'row is seeded and built in (D2). Retired, never deleted while anything names it (D14). '
    'Mirrored.';
COMMENT ON COLUMN target.kind IS
    'MOD-130 M1 (D1): local, docker, ssh, ssh_docker or enrolled. All five are stored; a kind this '
    'build cannot run derives the status planned.';
COMMENT ON COLUMN target.locator IS
    'MOD-130 M1 (ANA-42 7.1): a JSON object of names and addresses only, never values: an ssh '
    'Host and user, a docker context, an image, a run root. The store refuses a key or a URL that '
    'looks like a secret.';
COMMENT ON COLUMN target.box_id IS
    'MOD-130 M1 (D2, P4): the box the target runs on once bound; NULL on the local target, which '
    'means the box that starts the run, and on a target not yet bound.';
COMMENT ON COLUMN target.probe_error IS
    'MOD-130 M1 (D4): why the last probe through this target failed; NULL after a good one. Set, '
    'the status is unreachable.';
COMMENT ON COLUMN target.retired_at IS
    'MOD-130 M1 (D14): when the target was retired. A retired target is hidden from pickers and '
    'keeps its history; never set on the local target.';
COMMENT ON COLUMN box.kind IS
    'MOD-130 M1 (D12): machine, or container, a child of a machine (parent_box_id). Only a '
    'container may hold a NULL os_family, until its first probe (D13).';
COMMENT ON COLUMN box.parent_box_id IS
    'MOD-130 M1 (D12): the machine a container box runs on; NULL exactly on a machine.';
COMMENT ON COLUMN project.target_id IS
    'MOD-130 M1 (D3): the project''s target; NULL = inherit; on a scope row, the workspace level '
    '(D5).';
COMMENT ON COLUMN step_graph.target_id IS
    'MOD-130 M1 (D3, D11): the workflow''s target; NULL = inherit. Setting it bumps revision.';
COMMENT ON COLUMN item.target_id IS
    'MOD-130 M1 (D3, D11): the item''s target, a spec field under version; NULL = inherit.';
COMMENT ON COLUMN run.target_id IS
    'MOD-130 M1 (D7): the target the run resolved at its start; NULL on a run started before '
    'targets. Set with target_source.';
COMMENT ON COLUMN run.target_source IS
    'MOD-130 M1 (D7): the level that named run.target_id: item, workflow, project, workspace, '
    'settings or default (no level and no Settings default: the local target).';
