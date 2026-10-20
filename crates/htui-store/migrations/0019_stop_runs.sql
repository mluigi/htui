-- 0019_stop_runs.sql - MOD-125 milestone 1 (plan D1, D3, D4). Forward-only (R-STO-5).
--
-- A run and a step can be 'stopped': halted by a workspace stop and resumable (R-ORCH-16). A
-- stopped run holds no slot (claim_run's slot counts 'running' only) but keeps its trees and its
-- branch, so it still refuses an overlapping scope (ANA-2 §4.7, invariant 6); it has no
-- finished_at and its item stays in_progress. 0001_init.sql's two status CHECKs are unnamed, so
-- Postgres named them run_status_check and run_step_status_check; both are dropped and re-added
-- under the same names with 'stopped' after 'awaiting_approval', the order RunStatus::ALL and
-- StepStatus::ALL follow.
--
-- run.workspace_id is the workspace a graph run was started from (PRD Q1): a workspace stop
-- stops only those runs. A graph run whose project belongs to exactly one workspace is backfilled
-- to it; a project in none or in several leaves its runs NULL, and no workspace stop ever touches
-- them. Chat runs stay NULL: a chat is driven by the person typing in it. ON DELETE SET NULL, so
-- a deleted workspace leaves its runs unscoped. The backfill fires trg_run_updated_at, which only
-- moves the cache cursor that the mirror rebuild below replaces anyway. queue_entry.workspace_id
-- is the workspace an item was queued from; the queue runner copies it onto the run it admits.
--
-- workspace_stop holds one row per stopped workspace (plan D4). Its presence is the gate that
-- claim_run and adopt_runs read; a resume deletes it. Not mirrored, like queue_batch: it is read
-- through the store. The mirror gains run.workspace_id in cache_migrations/0006_run_workspace.sql,
-- and schema_version becomes 19, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

ALTER TABLE run DROP CONSTRAINT run_status_check;
ALTER TABLE run ADD CONSTRAINT run_status_check CHECK (status IN
    ('queued','running','awaiting_approval','stopped','done','failed','cancelled'));

ALTER TABLE run_step DROP CONSTRAINT run_step_status_check;
ALTER TABLE run_step ADD CONSTRAINT run_step_status_check CHECK (status IN
    ('pending','running','awaiting_approval','stopped','done','failed','cancelled','superseded'));

ALTER TABLE run
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_run_workspace REFERENCES workspace(id) ON DELETE SET NULL;

CREATE INDEX idx_run_workspace_status ON run(workspace_id, status);

UPDATE run r
   SET workspace_id = wp.workspace_id
  FROM workspace_project wp
 WHERE wp.project_id = r.project_id
   AND r.kind = 'graph'
   AND NOT EXISTS (SELECT 1
                     FROM workspace_project other
                    WHERE other.project_id = r.project_id
                      AND other.workspace_id <> wp.workspace_id);

ALTER TABLE queue_entry
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_queue_entry_workspace REFERENCES workspace(id) ON DELETE SET NULL;

-- DV-6 (recommended): the same one-workspace rule for entries queued before 0019, so the run an
-- old entry admits is gated like a backfilled one. Drop this statement if the maintainer declines.
UPDATE queue_entry q
   SET workspace_id = wp.workspace_id
  FROM item i
  JOIN workspace_project wp ON wp.project_id = i.project_id
 WHERE i.id = q.item_id
   AND NOT EXISTS (SELECT 1
                     FROM workspace_project other
                    WHERE other.project_id = i.project_id
                      AND other.workspace_id <> wp.workspace_id);

CREATE TABLE workspace_stop (
    workspace_id UUID        PRIMARY KEY REFERENCES workspace(id) ON DELETE CASCADE,
    mode         TEXT        NOT NULL,
    requested_by UUID        NOT NULL REFERENCES app_user(id),
    requested_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT chk_workspace_stop_mode CHECK (mode IN ('graceful', 'forced'))
);

COMMENT ON COLUMN run.status IS
    'MOD-125 D1: stopped = halted by a workspace stop and resumable; live (not terminal), holds no '
    'slot, still overlaps, no finished_at.';
COMMENT ON COLUMN run_step.status IS
    'MOD-125 D1: stopped = interrupted by a workspace stop; resumes to running, or is superseded '
    'by a fresh rerun.';
COMMENT ON COLUMN run.workspace_id IS
    'MOD-125 D3: the workspace a graph run was started from; a workspace stop stops only these. '
    'NULL for chat runs and for runs whose project was in no or several workspaces at 0019.';
COMMENT ON COLUMN queue_entry.workspace_id IS
    'MOD-125 D3: the workspace the item was queued from; copied onto the run admit creates.';
COMMENT ON TABLE workspace_stop IS
    'MOD-125 D4: one row per stopped workspace; while it exists claim_run refuses and adopt_runs '
    'settles abandoned runs of the workspace as stopped. Resume deletes it. Not mirrored.';
COMMENT ON COLUMN workspace_stop.mode IS
    'MOD-125 D4: graceful (cancel the turn, keep the session) or forced (kill now); a forced stop '
    'is never downgraded by a later graceful one.';
COMMENT ON COLUMN workspace_stop.requested_by IS
    'MOD-125 D4: who asked for the stop (the escalating user, after an escalation).';
COMMENT ON COLUMN workspace_stop.requested_at IS
    'MOD-125 D4: the caller''s clock at the request (the escalation''s, after an escalation).';
