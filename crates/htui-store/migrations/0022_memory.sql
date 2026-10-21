-- 0022_memory.sql - MOD-163 milestone 1 (plan D1, D3, D4, D6-D9): the project memory entry store
-- (R-ENT-13, ANA-38 §4.6). Forward-only (R-STO-5). Depends on 0001_init.sql (project, app_user,
-- box, set_updated_at()). Mirrors 0006_requirements.sql's requirement / requirement_revision pair.
--
-- memory_entry is one project-owned entry: kind, subject, prose, state, a compare-and-set version
-- and confirmed_at, the freshness stamp (D3). Text reaches it only through htui-core's MemoryText,
-- trimmed, scrubbed and capped in characters before the write (R-ID-7, D5); the two char_length
-- CHECKs are the backstop. One live entry per subject (D4): uq_memory_entry_live_subject folds
-- the subject with lower(btrim()) among active and suspect entries of a project; it is named so
-- that PgStore can tell its 23505 from the primary key's. superseded_by is DEFERRABLE INITIALLY
-- DEFERRED because a supersede moves the old entry first (freeing its subject) and inserts the
-- new one second, in one transaction (D6); it is set only on a superseded entry, and ON DELETE
-- SET NULL may clear it again, so the CHECK is one-way. Provenance, usage counters and pins are
-- later items' columns (D9).
--
-- Not mirrored (D7): there is no cache migration. cache_meta.schema_version moves to 20 through
-- PgStore::schema_version(), so each box rebuilds its mirror once on first start; a headless
-- worker never migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. memory_entry
-- --------------------------------------------------------------------------------------------

CREATE TABLE memory_entry (
    id            UUID        PRIMARY KEY,                   -- client-minted UUIDv7
    project_id    UUID        NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind          TEXT        NOT NULL CHECK (kind IN ('fact','decision','gotcha','lesson')),
    subject       TEXT        NOT NULL CHECK (char_length(subject) BETWEEN 1 AND 120),
    body          TEXT        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 2000),
    state         TEXT        NOT NULL DEFAULT 'active'
                              CHECK (state IN ('active','suspect','superseded','retired')),
    version       INTEGER     NOT NULL DEFAULT 1,            -- CAS, ANA-9 §4.2
    confirmed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    superseded_by UUID
        CONSTRAINT fk_memory_entry_superseded_by REFERENCES memory_entry(id)
        ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,
    created_by    UUID        NOT NULL REFERENCES app_user(id),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_memory_entry_superseded_by
        CHECK (superseded_by IS NULL OR (state = 'superseded' AND superseded_by <> id))
);
CREATE INDEX idx_memory_entry_updated_at ON memory_entry(project_id, updated_at);   -- list order
CREATE UNIQUE INDEX uq_memory_entry_live_subject
    ON memory_entry(project_id, lower(btrim(subject)))
    WHERE state IN ('active','suspect');
-- ON DELETE SET NULL's lookup when an entry is deleted (as 0006's idx_requirement_revision_amended_by)
CREATE INDEX idx_memory_entry_superseded_by ON memory_entry(superseded_by)
    WHERE superseded_by IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 2. memory_entry_revision: append-only, no updated_at, no trigger
-- --------------------------------------------------------------------------------------------

CREATE TABLE memory_entry_revision (
    entry_id    UUID        NOT NULL REFERENCES memory_entry(id) ON DELETE CASCADE,
    version     INTEGER     NOT NULL,
    kind        TEXT        NOT NULL,
    subject     TEXT        NOT NULL,
    body        TEXT        NOT NULL,
    state       TEXT        NOT NULL,
    author_id   UUID        NOT NULL REFERENCES app_user(id),
    box_id      UUID        REFERENCES box(id) ON DELETE SET NULL,
    reason      TEXT        NOT NULL,                        -- 'created','amended','retired','superseded'
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (entry_id, version)
);

-- --------------------------------------------------------------------------------------------
-- 3. updated_at trigger, 0001 §5.1's shape (BEFORE UPDATE only). memory_entry_revision has no
-- updated_at and is deliberately absent.
-- --------------------------------------------------------------------------------------------

CREATE TRIGGER trg_memory_entry_updated_at BEFORE UPDATE ON memory_entry
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- --------------------------------------------------------------------------------------------
-- 4. Comments (pinned verbatim by tests/migrations.rs's MOD163_* constants)
-- --------------------------------------------------------------------------------------------

COMMENT ON TABLE memory_entry IS
    'MOD-163 D1: project memory entries (R-ENT-13), written by a person in M1. Not mirrored (D7).';
COMMENT ON TABLE memory_entry_revision IS
    'MOD-163 D1: append-only history of memory_entry, one row per version.';
COMMENT ON COLUMN memory_entry.state IS
    'MOD-163 D4: active and suspect are live, one per folded subject per project; superseded and '
    'retired are closed. Nothing in M1 sets suspect.';
COMMENT ON COLUMN memory_entry.version IS
    'MOD-163 D3: the compare-and-set token; every write moves it by one and writes a revision.';
COMMENT ON COLUMN memory_entry.confirmed_at IS
    'MOD-163 D3: when the text was last written or confirmed; create and amend set it, retire and '
    'supersede leave it.';
COMMENT ON COLUMN memory_entry.superseded_by IS
    'MOD-163 D4, D6: the entry that replaced this one, set only on a superseded entry; NULL again '
    'if that entry is deleted.';
COMMENT ON COLUMN memory_entry_revision.reason IS
    'MOD-163 D1: created, amended, retired or superseded (no CHECK, as requirement_revision.reason).';
