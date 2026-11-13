-- 0031_command_log.sql - MOD-152 milestone 1 (plan D1, D2): the log of a command_run row.
-- Forward-only (R-STO-5). Depends on 0001_init.sql (command_run).
--
-- command_run.output stays the 64 KiB scrubbed tail the verify gate reads. The log is the longer
-- tail of the same merged output (at most 2 MiB, the store accepts COMMAND_LOG_STORE_MAX bytes),
-- scrubbed as one text by the runner before it is stored (R-ID-7, R-SEC-3). It lives in a table of
-- its own so CommandRun and every command_runs() read stay unchanged and cheap. One log per row,
-- written once by the runner before the row ends (upserted by command_run_id), and removed with its
-- row: the key is the foreign key.
--
-- text is TEXT, so a NUL cannot be stored: the runner turns NULs into U+FFFD first. truncated says
-- the text is a tail; total_bytes is the size of the whole output before any cut.
--
-- Not mirrored: command_run itself is not, so there is no cache migration. cache_meta.schema_version
-- moves to 31 through PgStore::schema_version(), so each box rebuilds its mirror once on first
-- start; a headless worker never migrates: migrate from a TUI first.

CREATE TABLE command_log (
    command_run_id UUID        PRIMARY KEY REFERENCES command_run (id) ON DELETE CASCADE,
    text           TEXT        NOT NULL,
    truncated      BOOLEAN     NOT NULL DEFAULT FALSE,
    total_bytes    BIGINT      NOT NULL DEFAULT 0,
    written_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_command_log_total_bytes CHECK (total_bytes >= 0)
);
