-- 0016_follow_up.sql - MOD-70 (plan D1, D3, D4, D5).
-- Forward-only (R-STO-5).
--
-- Follow-ups for engine steps. A run_command row of kind follow_up carries one user follow-up for
-- one running step's live engine session: any store client queues it, the process walking the
-- step takes it at the session's next turn end and records the scrubbed follow_up event itself
-- (session_event stays single-writer). Its text is plaintext only while the row is pending and is
-- nulled when it resolves (chk_run_command_follow_up). follow_up_window holds one row per step whose
-- engine session takes follow-ups; closed_at is set when that session ends, and the enqueue's
-- FOR SHARE on the window row against the close's UPDATE is what makes a follow-up that misses the
-- last turn end a refusal rather than a stranded row. A step's status is unchanged: there is no new
-- state. Every time is clock_timestamp(). Both tables go with their run and step (ON DELETE
-- CASCADE), which delete_project relies on. Neither is mirrored, but schema_version becomes 16, so
-- each box rebuilds its mirror once. A headless worker never migrates: migrate from a TUI first, and
-- upgrade every box together - a pre-0016 binary's cancel no longer infers an index here.

ALTER TABLE run_command
    ADD COLUMN run_step_id UUID REFERENCES run_step(id) ON DELETE CASCADE,
    ADD COLUMN text        TEXT;

ALTER TABLE run_command DROP CONSTRAINT chk_run_command_kind;
ALTER TABLE run_command
    ADD CONSTRAINT chk_run_command_kind CHECK (kind IN ('cancel', 'follow_up'));
ALTER TABLE run_command
    ADD CONSTRAINT chk_run_command_follow_up CHECK (
        (kind = 'cancel' AND run_step_id IS NULL AND text IS NULL)
        OR (kind = 'follow_up' AND run_step_id IS NOT NULL
            AND (status = 'pending') = (text IS NOT NULL)));

-- One pending cancel per run, as before; one pending follow-up per step (PRD Q4). Two plain partial
-- indexes so each ON CONFLICT target infers a plain column list.
DROP INDEX uq_run_command_pending;
CREATE UNIQUE INDEX uq_run_command_pending_cancel ON run_command (run_id)
    WHERE status = 'pending' AND kind = 'cancel';
CREATE UNIQUE INDEX uq_run_command_pending_follow_up ON run_command (run_step_id)
    WHERE status = 'pending' AND kind = 'follow_up';

CREATE TABLE follow_up_window (
    run_step_id UUID        PRIMARY KEY REFERENCES run_step(id) ON DELETE CASCADE,
    run_id      UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    session     UUID        NOT NULL,
    owner       UUID        NOT NULL,  -- the lease owner the window opened under (B-4)
    opened_at   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    closed_at   TIMESTAMPTZ
);
CREATE INDEX idx_follow_up_window_open ON follow_up_window (run_id) WHERE closed_at IS NULL;
