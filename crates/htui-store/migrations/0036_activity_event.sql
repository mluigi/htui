-- 0036_activity_event.sql - MOD-122 milestone 3 (plan D2-D5, D7; blueprint F2-F4): activity_event
-- holds one row per transition a person should hear about, which the TUI's Activity panel reads.
-- Forward-only (R-STO-5). Depends on 0001_init.sql (app_user, workspace, project, item, run,
-- run_step), 0011_permission_relay.sql (step_permission), 0019_stop_runs.sql (run.workspace_id)
-- and 0026_workspace_scope.sql (project.owner_workspace_id).
--
-- The rows are written by the store inside the transition, never by a caller (plan D2): one
-- deferred constraint trigger per source table (run_step, run, item: AFTER UPDATE OF status;
-- step_permission: AFTER INSERT of a pending request; no insert on the other three writes, so the
-- demo fixture's parked and failed rows write nothing, plan D13). Deferred, each call runs at
-- commit and re-reads its row: it writes only when the row's current status is still the NEW.status
-- that queued it, so a park undone before commit writes nothing and the order of park_step's two
-- updates cannot turn a gate into an "interrupted". A row that moves twice in one transaction
-- queues two calls that would write the same event: txid (pg_current_xact_id(), the top-level
-- transaction even inside a subtransaction) and subject (a copy, never NULL) carry the unique
-- index uq_activity_event_once, and every insert is ON CONFLICT DO NOTHING (blueprint F3). No
-- function has an EXCEPTION block: it would open a subtransaction per call.
--
-- Plan D3's kinds and texts; the run-level park rule is htui_core::model::park_kind's, first match
-- wins: judge_failed, selection, interrupted (the conformance case run_park_kinds pins the two).
-- Plan D5: workspace_id is the run's recorded workspace, else the project's scope workspace, else
-- the project's only workspace, else NULL. The user is run.started_by, or item.created_by for an
-- unblock. Plan D4: nothing records that a wait ended; the read derives it from the subject.
--
-- Deleting an item or a run (no product path deletes either outside delete_project and
-- delete_workspace's cascades) keeps only its failures: the BEFORE DELETE triggers delete the
-- other kinds first, so ON DELETE SET NULL only ever touches step_failed and run_failed rows,
-- which keep project_slug, item_key and step_name and expire at retention (plan D7).
--
-- Not mirrored: the cache has no activity_event and MIRRORED_TABLES is unchanged. cache_meta's
-- schema_version moves to 34 through PgStore::schema_version(), so each box rebuilds its mirror
-- once on first start; a headless worker never migrates: migrate from a TUI first.
CREATE TABLE activity_event (
    id            BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id       UUID        NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    kind          TEXT        NOT NULL,
    workspace_id  UUID        REFERENCES workspace(id)       ON DELETE SET NULL,
    project_id    UUID        REFERENCES project(id)         ON DELETE SET NULL,
    project_slug  TEXT        NOT NULL,
    item_id       UUID        REFERENCES item(id)            ON DELETE SET NULL,
    item_key      TEXT,
    run_id        UUID        REFERENCES run(id)             ON DELETE SET NULL,
    chat          BOOLEAN     NOT NULL DEFAULT false,
    step_id       UUID        REFERENCES run_step(id)        ON DELETE SET NULL,
    step_name     TEXT,
    permission_id UUID        REFERENCES step_permission(id) ON DELETE SET NULL,
    text          TEXT        NOT NULL DEFAULT '',
    -- F3: the once-per-transaction guard; neither is exposed by `ActivityEvent`.
    txid          XID8        NOT NULL DEFAULT pg_current_xact_id(),
    -- F3: a copy, not an FK: the step for gate/step_failed, the run for the seven run kinds,
    -- the item for unblock, the permission for permission. Never NULL.
    subject       UUID        NOT NULL,
    CONSTRAINT chk_activity_event_kind CHECK (kind IN ('gate', 'judge_failed', 'selection',
        'unblock', 'interrupted', 'permission', 'step_failed', 'run_failed', 'run_started',
        'run_finished', 'close_out'))
);

CREATE INDEX idx_activity_event_user ON activity_event (user_id, id);
CREATE INDEX idx_activity_event_at ON activity_event (at);
CREATE INDEX idx_activity_event_needs_you ON activity_event (user_id, id)
    WHERE kind IN ('gate', 'judge_failed', 'selection', 'unblock', 'interrupted', 'permission');
CREATE INDEX idx_activity_event_item ON activity_event (item_id);
CREATE INDEX idx_activity_event_run ON activity_event (run_id);
-- F4: the FK columns ON DELETE SET NULL searches once per deleted step or permission
-- (delete_project cascades over every run_step and step_permission of the project).
CREATE INDEX idx_activity_event_step ON activity_event (step_id) WHERE step_id IS NOT NULL;
CREATE INDEX idx_activity_event_permission ON activity_event (permission_id)
    WHERE permission_id IS NOT NULL;
-- F3: at most one event per transaction, kind and subject.
CREATE UNIQUE INDEX uq_activity_event_once ON activity_event (txid, kind, subject);

-- One event of p_kind about p_subject, for p_user. The item defaults to the run's; the project
-- is the run's, else the item's; the copies are read now, so they outlive their rows (plan D7).
CREATE FUNCTION activity_event_write(
    p_kind TEXT,
    p_user UUID,
    p_subject UUID,
    p_run_id UUID,
    p_item_id UUID,
    p_step_id UUID,
    p_permission_id UUID,
    p_text TEXT
) RETURNS void AS $$
DECLARE
    v_run          run%ROWTYPE;
    v_item         item%ROWTYPE;
    v_project_id   UUID;
    v_workspace_id UUID;
    v_slug         TEXT;
    v_step_name    TEXT;
BEGIN
    IF p_run_id IS NOT NULL THEN
        SELECT * INTO v_run FROM run WHERE id = p_run_id;
    END IF;
    IF COALESCE(p_item_id, v_run.item_id) IS NOT NULL THEN
        SELECT * INTO v_item FROM item WHERE id = COALESCE(p_item_id, v_run.item_id);
    END IF;
    v_project_id := COALESCE(v_run.project_id, v_item.project_id);

    -- Plan D5: the run's recorded workspace, the scope row's owner, the only workspace, or NULL.
    v_workspace_id := v_run.workspace_id;
    IF v_workspace_id IS NULL THEN
        SELECT owner_workspace_id INTO v_workspace_id FROM project WHERE id = v_project_id;
    END IF;
    IF v_workspace_id IS NULL THEN
        SELECT (array_agg(wp.workspace_id))[1] INTO v_workspace_id
          FROM workspace_project wp
         WHERE wp.project_id = v_project_id
        HAVING count(*) = 1;
    END IF;

    SELECT slug INTO v_slug FROM project WHERE id = v_project_id;
    IF p_step_id IS NOT NULL THEN
        SELECT phase_name INTO v_step_name FROM run_step WHERE id = p_step_id;
    END IF;

    INSERT INTO activity_event (user_id, kind, workspace_id, project_id, project_slug, item_id,
                                item_key, run_id, chat, step_id, step_name, permission_id, text,
                                subject)
    VALUES (p_user, p_kind, v_workspace_id, v_project_id, COALESCE(v_slug, ''), v_item.id,
            CASE WHEN v_item.id IS NOT NULL THEN v_item.key_prefix || '-' || v_item.key_number END,
            v_run.id, COALESCE(v_run.kind = 'chat', false), p_step_id, v_step_name,
            p_permission_id, left(COALESCE(p_text, ''), 500), p_subject)
    ON CONFLICT (txid, kind, subject) DO NOTHING;
END
$$ LANGUAGE plpgsql;

-- run_step -> awaiting_approval is a gate, -> failed a step_failed; the text is the gate note.
CREATE FUNCTION activity_run_step_update() RETURNS trigger AS $$
DECLARE
    v_step run_step%ROWTYPE;
    v_user UUID;
BEGIN
    SELECT * INTO v_step FROM run_step WHERE id = NEW.id;
    IF NOT FOUND OR v_step.status <> NEW.status THEN
        RETURN NULL;
    END IF;
    SELECT started_by INTO v_user FROM run WHERE id = v_step.run_id;
    IF v_user IS NULL THEN
        RETURN NULL;
    END IF;
    PERFORM activity_event_write(
        CASE WHEN v_step.status = 'failed' THEN 'step_failed' ELSE 'gate' END,
        v_user, v_step.id, v_step.run_id, NULL, v_step.id, NULL, v_step.gate_note);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

-- Plan D3's run rows: a park with no step parked (judge_failed, selection or interrupted), a
-- failure, a graph run's start from queued, a graph run's finish (close_out over an item that is
-- not closed, else run_finished).
CREATE FUNCTION activity_run_update() RETURNS trigger AS $$
DECLARE
    v_run      run%ROWTYPE;
    v_kind     TEXT;
    v_step_id  UUID;
    v_text     TEXT;
    v_position INTEGER;
    v_attempt  INTEGER;
BEGIN
    SELECT * INTO v_run FROM run WHERE id = NEW.id;
    IF NOT FOUND OR v_run.status <> NEW.status THEN
        RETURN NULL;
    END IF;

    IF v_run.status = 'awaiting_approval' THEN
        IF EXISTS (SELECT 1 FROM run_step
                    WHERE run_id = v_run.id AND status = 'awaiting_approval') THEN
            RETURN NULL;
        END IF;
        -- 1. A failed judge with a note in a slot with no selected candidate; the newest slot.
        SELECT j.id, j.gate_note INTO v_step_id, v_text
          FROM run_step j
         WHERE j.run_id = v_run.id AND j.fanout_index = -1 AND j.status = 'failed'
           AND j.gate_note IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM run_step c
                            WHERE c.run_id = j.run_id AND c.position = j.position
                              AND c.attempt = j.attempt AND c.fanout_index >= 0
                              AND c.selected IS TRUE)
         ORDER BY j.position DESC, j.attempt DESC
         LIMIT 1;
        IF v_step_id IS NOT NULL THEN
            v_kind := 'judge_failed';
        ELSE
            v_text := NULL;
            -- 2. The newest slot of two or more candidates, none selected, none still to run.
            SELECT s.position, s.attempt INTO v_position, v_attempt
              FROM run_step s
             WHERE s.run_id = v_run.id AND s.fanout_index >= 0
             GROUP BY s.position, s.attempt
            HAVING count(*) >= 2
             ORDER BY s.position DESC, s.attempt DESC
             LIMIT 1;
            IF v_position IS NOT NULL AND NOT EXISTS (
                SELECT 1 FROM run_step c
                 WHERE c.run_id = v_run.id AND c.position = v_position
                   AND c.attempt = v_attempt AND c.fanout_index >= 0
                   AND (c.selected IS TRUE OR c.status IN ('running', 'pending'))) THEN
                v_kind := 'selection';
                SELECT c.id INTO v_step_id
                  FROM run_step c
                 WHERE c.run_id = v_run.id AND c.position = v_position
                   AND c.attempt = v_attempt AND c.fanout_index >= 0
                 ORDER BY c.fanout_index
                 LIMIT 1;
            ELSE
                -- 3. Interrupted, on the most recently updated failed step, if any.
                v_kind := 'interrupted';
                SELECT f.id INTO v_step_id
                  FROM run_step f
                 WHERE f.run_id = v_run.id AND f.status = 'failed'
                 ORDER BY f.updated_at DESC, f.id DESC
                 LIMIT 1;
            END IF;
        END IF;
        PERFORM activity_event_write(v_kind, v_run.started_by, v_run.id, v_run.id, NULL,
                                     v_step_id, NULL, v_text);
    ELSIF v_run.status = 'failed' THEN
        SELECT s.id INTO v_step_id
          FROM run_step s
         WHERE s.run_id = v_run.id
         ORDER BY s.updated_at DESC, s.id DESC
         LIMIT 1;
        PERFORM activity_event_write('run_failed', v_run.started_by, v_run.id, v_run.id, NULL,
                                     v_step_id, NULL, v_run.failure);
    ELSIF v_run.status = 'running' THEN
        IF v_run.kind = 'graph' AND OLD.status = 'queued' THEN
            PERFORM activity_event_write('run_started', v_run.started_by, v_run.id, v_run.id,
                                         NULL, NULL, NULL, NULL);
        END IF;
    ELSIF v_run.status = 'done' THEN
        IF v_run.kind = 'graph' THEN
            PERFORM activity_event_write(
                CASE WHEN EXISTS (SELECT 1 FROM item
                                   WHERE id = v_run.item_id AND status <> 'closed')
                     THEN 'close_out' ELSE 'run_finished' END,
                v_run.started_by, v_run.id, v_run.id, NULL, NULL, NULL, NULL);
        END IF;
    END IF;
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

-- item -> blocked is an unblock, with the item's newest live run.
CREATE FUNCTION activity_item_update() RETURNS trigger AS $$
DECLARE
    v_item   item%ROWTYPE;
    v_run_id UUID;
BEGIN
    SELECT * INTO v_item FROM item WHERE id = NEW.id;
    IF NOT FOUND OR v_item.status <> NEW.status THEN
        RETURN NULL;
    END IF;
    SELECT r.id INTO v_run_id
      FROM run r
     WHERE r.item_id = v_item.id
       AND r.status IN ('queued', 'running', 'awaiting_approval', 'stopped')
     ORDER BY r.queued_at DESC, r.id DESC
     LIMIT 1;
    PERFORM activity_event_write('unblock', v_item.created_by, v_item.id, v_run_id, v_item.id,
                                 NULL, NULL, NULL);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

-- A step_permission inserted pending, and still pending at commit, is a permission event.
CREATE FUNCTION activity_step_permission_insert() RETURNS trigger AS $$
DECLARE
    v_permission step_permission%ROWTYPE;
    v_user       UUID;
BEGIN
    SELECT * INTO v_permission FROM step_permission WHERE id = NEW.id;
    IF NOT FOUND OR v_permission.status <> NEW.status THEN
        RETURN NULL;
    END IF;
    SELECT started_by INTO v_user FROM run WHERE id = v_permission.run_id;
    IF v_user IS NULL THEN
        RETURN NULL;
    END IF;
    PERFORM activity_event_write('permission', v_user, v_permission.id, v_permission.run_id,
                                 NULL, v_permission.run_step_id, v_permission.id,
                                 v_permission.summary);
    RETURN NULL;
END
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER trg_run_step_activity_update
    AFTER UPDATE OF status ON run_step
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status
          AND NEW.status IN ('awaiting_approval', 'failed'))
    EXECUTE FUNCTION activity_run_step_update();

CREATE CONSTRAINT TRIGGER trg_run_activity_update
    AFTER UPDATE OF status ON run
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status
          AND NEW.status IN ('awaiting_approval', 'failed', 'running', 'done'))
    EXECUTE FUNCTION activity_run_update();

CREATE CONSTRAINT TRIGGER trg_item_activity_update
    AFTER UPDATE OF status ON item
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status AND NEW.status IN ('blocked'))
    EXECUTE FUNCTION activity_item_update();

CREATE CONSTRAINT TRIGGER trg_step_permission_activity_insert
    AFTER INSERT ON step_permission
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (NEW.status = 'pending')
    EXECUTE FUNCTION activity_step_permission_insert();

-- Plan D7: an item's or a run's events go with it, its failures excepted.
CREATE FUNCTION activity_item_delete() RETURNS trigger AS $$
BEGIN
    DELETE FROM activity_event
     WHERE item_id = OLD.id AND kind NOT IN ('step_failed', 'run_failed');
    RETURN OLD;
END
$$ LANGUAGE plpgsql;

CREATE FUNCTION activity_run_delete() RETURNS trigger AS $$
BEGIN
    DELETE FROM activity_event
     WHERE run_id = OLD.id AND kind NOT IN ('step_failed', 'run_failed');
    RETURN OLD;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_item_activity_delete
    BEFORE DELETE ON item
    FOR EACH ROW EXECUTE FUNCTION activity_item_delete();

CREATE TRIGGER trg_run_activity_delete
    BEFORE DELETE ON run
    FOR EACH ROW EXECUTE FUNCTION activity_run_delete();
