-- 0011_permission_relay.sql - MOD-42 (plan D1, D3, D4, D5, D12, D13).
-- Forward-only (R-STO-5).
--
-- The permission and control relay. step_permission holds one row per stage-3 permission request
-- an executor parked: any store client answers it with a compare-and-set, and the executor that
-- parked it applies the answer to its live session and records the permission_answer event
-- itself (session_event stays single-writer). run_command holds one row per requested command on
-- a run (today only cancel), applied by whichever process holds or adopts the run on its
-- executing box. Every time is clock_timestamp(), never a box clock. Both tables go with their
-- run (ON DELETE CASCADE), which delete_project relies on. Neither is mirrored (plan OQ-4), but
-- schema_version becomes 11, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

CREATE TABLE step_permission (
    id            UUID        PRIMARY KEY,
    run_id        UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    run_step_id   UUID        NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    session       UUID        NOT NULL,
    request_id    TEXT        NOT NULL,
    tool_call_id  TEXT,
    summary       TEXT,
    options       JSONB       NOT NULL,
    owner         UUID        NOT NULL,
    status        TEXT        NOT NULL DEFAULT 'pending',
    option_id     TEXT,
    answered_by   UUID        REFERENCES app_user(id),
    answered_box  UUID        REFERENCES box(id),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    answered_at   TIMESTAMPTZ,
    resolved_at   TIMESTAMPTZ,
    CONSTRAINT chk_step_permission_status
        CHECK (status IN ('pending', 'answered', 'applied', 'cancelled', 'stale')),
    CONSTRAINT chk_step_permission_options CHECK (jsonb_typeof(options) = 'array'),
    CONSTRAINT chk_step_permission_answer
        CHECK (status NOT IN ('answered', 'applied')
               OR (option_id IS NOT NULL AND answered_by IS NOT NULL
                   AND answered_box IS NOT NULL AND answered_at IS NOT NULL)),
    CONSTRAINT chk_step_permission_resolved
        CHECK ((status IN ('applied', 'cancelled', 'stale')) = (resolved_at IS NOT NULL)),
    CONSTRAINT uq_step_permission_request UNIQUE (session, request_id)
);
CREATE INDEX idx_step_permission_open ON step_permission (run_id)
    WHERE status IN ('pending', 'answered');
CREATE INDEX idx_step_permission_step ON step_permission (run_step_id)
    WHERE status IN ('pending', 'answered');

CREATE TABLE run_command (
    id           UUID        PRIMARY KEY,
    run_id       UUID        NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    kind         TEXT        NOT NULL,
    issued_by    UUID        NOT NULL REFERENCES app_user(id),
    issued_box   UUID        NOT NULL REFERENCES box(id),
    status       TEXT        NOT NULL DEFAULT 'pending',
    resolution   TEXT,
    issued_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    resolved_at  TIMESTAMPTZ,
    CONSTRAINT chk_run_command_kind CHECK (kind IN ('cancel')),
    CONSTRAINT chk_run_command_status CHECK (status IN ('pending', 'applied', 'refused')),
    CONSTRAINT chk_run_command_resolved CHECK ((status = 'pending') = (resolved_at IS NULL))
);
CREATE UNIQUE INDEX uq_run_command_pending ON run_command (run_id, kind)
    WHERE status = 'pending';
