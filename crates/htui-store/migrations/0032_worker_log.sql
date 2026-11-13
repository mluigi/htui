-- 0032_worker_log.sql - MOD-122 milestone 2 (plan D1, D4, D6): worker_log holds the log lines a
-- headless htui worker ships, so the TUI reads them in its Logs tab. Forward-only (R-STO-5).
-- Depends on 0001_init.sql (app_user, box).
--
-- One row per shipped line. logged_at is the worker's clock and only displayed; received_at is the
-- database's (clock_timestamp(), never a box clock) and retention keys on it. box_name is the
-- box's hostname when the line was written, so a row stays attributable after its box goes.
-- level is the five tracing levels, in severity order; the text is capped at 8 KiB by the
-- appender, not by a CHECK.
--
-- Deleting a box (no product path does, only test helpers) keeps only its error lines: the
-- BEFORE DELETE trigger deletes the others first, so ON DELETE SET NULL only ever touches error
-- rows, which keep box_name and expire at retention (plan D6). The per-box cap of the sweep counts
-- box_id IS NOT NULL rows only, so a removed box's errors are not capped together.
--
-- Not mirrored: the cache has no worker_log and MIRRORED_TABLES is unchanged. cache_meta's
-- schema_version moves to 31 through PgStore::schema_version(), so each box rebuilds its mirror
-- once on first start; a headless worker never migrates: migrate from a TUI first.
CREATE TABLE worker_log (
    id          BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
    box_id      UUID        REFERENCES box(id) ON DELETE SET NULL,
    box_name    TEXT        NOT NULL,
    logged_at   TIMESTAMPTZ NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    level       TEXT        NOT NULL,
    target      TEXT        NOT NULL,
    text        TEXT        NOT NULL,
    CONSTRAINT chk_worker_log_level CHECK (level IN ('error', 'warn', 'info', 'debug', 'trace'))
);

CREATE INDEX idx_worker_log_box ON worker_log (box_id, id);
CREATE INDEX idx_worker_log_received ON worker_log (received_at);

CREATE FUNCTION worker_log_box_delete() RETURNS trigger AS $$
BEGIN
    DELETE FROM worker_log WHERE box_id = OLD.id AND level <> 'error';
    RETURN OLD;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_box_worker_log_delete
    BEFORE DELETE ON box
    FOR EACH ROW EXECUTE FUNCTION worker_log_box_delete();
