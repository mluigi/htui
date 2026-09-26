-- 0006_requirements.sql - MOD-38: ANA-11 (requirements) docs/ANA-11.md §5, and the §4.2 close-out
-- resolution. Forward-only (R-STO-5): 0001-0004 are never edited. Depends on 0001_init.sql
-- (project, app_user, box, item, run_step, set_updated_at()).
-- The cache-mirror companion is cache_migrations/0004_requirements.sql (plan D12).
--
-- Deltas from §5 (plan D8): the per-area key counter is minted by mint_item's
-- INSERT ... ON CONFLICT DO UPDATE ... RETURNING CTE, not by UPDATE ... RETURNING; the four
-- mutable tables get their set_updated_at() triggers by explicit CREATE TRIGGER (0001's loop is
-- not re-run); closed rows are backfilled to 'done' before chk_item_resolution_iff_closed. No
-- COMMENT ON COLUMN: ANA_COLUMN_COMMENTS stays at 25.

-- --------------------------------------------------------------------------------------------
-- 1. item.resolution (ANA-11 §4.2): set by close_out only, NULL until the item is closed
-- --------------------------------------------------------------------------------------------

ALTER TABLE item ADD COLUMN resolution TEXT CHECK (resolution IN
    ('done','concluded','rejected','withdrawn','superseded','duplicate'));

-- Every item closed before this migration closed through the old blocked/failed/done -> closed
-- edges, which only a finished item took. trg_item_updated_at is off for the backfill, so every
-- closed item keeps the updated_at of its real last change (the mirror rebuilds on
-- schema_version 5 regardless).
ALTER TABLE item DISABLE TRIGGER trg_item_updated_at;
UPDATE item SET resolution = 'done' WHERE status = 'closed';
ALTER TABLE item ENABLE TRIGGER trg_item_updated_at;

ALTER TABLE item ADD CONSTRAINT chk_item_resolution_iff_closed
    CHECK ((status = 'closed') = (resolution IS NOT NULL));

-- --------------------------------------------------------------------------------------------
-- 2. requirement_spec (§4.4): one header per project
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_spec (
    project_id  UUID PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    owner_id    UUID NOT NULL REFERENCES app_user(id),
    preamble    TEXT NOT NULL DEFAULT '',
    version     INTEGER NOT NULL DEFAULT 1,      -- CAS token, no revision history (plan D9)
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 3. requirement_area
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_area (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    code        TEXT NOT NULL CHECK (code ~ '^[A-Z][A-Z0-9]{1,15}$'),   -- 'ENT','STO','NF'
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, code)
);

-- --------------------------------------------------------------------------------------------
-- 4. requirement_key_counter (ANA-9 §4.1 semantics, one per area)
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_key_counter (
    area_id     UUID PRIMARY KEY REFERENCES requirement_area(id) ON DELETE CASCADE,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0)
);

-- --------------------------------------------------------------------------------------------
-- 5. requirement -- area_id has no cascade, the shape of item.kind_id
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id   UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    area_id      UUID NOT NULL REFERENCES requirement_area(id),
    area_code    TEXT NOT NULL,                  -- copied at mint, like item.key_prefix
    number       INTEGER NOT NULL CHECK (number >= 1),
    key          TEXT GENERATED ALWAYS AS ('R-' || area_code || '-' || number::text) STORED,
    body         TEXT NOT NULL,
    rationale    TEXT NOT NULL DEFAULT '',
    priority     TEXT NOT NULL CHECK (priority IN ('must','later')),
    state        TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','withdrawn')),
    version      INTEGER NOT NULL DEFAULT 1,     -- CAS, ANA-9 §4.2
    created_by   UUID NOT NULL REFERENCES app_user(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, area_code, number)
);
CREATE INDEX idx_requirement_updated_at ON requirement(project_id, updated_at);   -- cache cursor
CREATE INDEX idx_requirement_area ON requirement(area_id);   -- FK check on area delete

-- --------------------------------------------------------------------------------------------
-- 6. requirement_revision: append-only, no updated_at, no trigger
-- --------------------------------------------------------------------------------------------

CREATE TABLE requirement_revision (
    requirement_id     UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    version            INTEGER NOT NULL,
    body               TEXT NOT NULL,
    rationale          TEXT NOT NULL,
    priority           TEXT NOT NULL,
    state              TEXT NOT NULL,
    author_id          UUID NOT NULL REFERENCES app_user(id),
    box_id             UUID REFERENCES box(id) ON DELETE SET NULL,
    reason             TEXT NOT NULL,            -- 'created','amended','withdrawn','imported','divergence_resolution'
    amended_by_item_id UUID REFERENCES item(id) ON DELETE SET NULL,   -- the deciding item
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (requirement_id, version)
);
-- ON DELETE SET NULL's lookup when an item is deleted
CREATE INDEX idx_requirement_revision_amended_by ON requirement_revision(amended_by_item_id);

-- --------------------------------------------------------------------------------------------
-- 7. item_requirement: citations; tombstoned like item_link
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_requirement (
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    requirement_id      UUID NOT NULL REFERENCES requirement(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL CHECK (kind IN ('addresses','amends','withdraws','reserves')),
    requirement_version INTEGER NOT NULL,        -- stamp; suspect when requirement.version is newer
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,   -- NULL = human or importer
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ,             -- tombstone; live citations have NULL
    PRIMARY KEY (item_id, requirement_id, kind)
);
CREATE INDEX idx_item_requirement_req ON item_requirement(requirement_id) WHERE deleted_at IS NULL;
CREATE INDEX idx_item_requirement_updated_at ON item_requirement(updated_at);   -- cache cursor

-- --------------------------------------------------------------------------------------------
-- 8. updated_at triggers, 0001 §5.1's shape: BEFORE UPDATE only, so an INSERT that supplies
-- updated_at (the demo loader) keeps it. requirement_key_counter and requirement_revision have
-- no updated_at and are deliberately absent.
-- --------------------------------------------------------------------------------------------

CREATE TRIGGER trg_requirement_spec_updated_at BEFORE UPDATE ON requirement_spec
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_requirement_area_updated_at BEFORE UPDATE ON requirement_area
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_requirement_updated_at BEFORE UPDATE ON requirement
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
CREATE TRIGGER trg_item_requirement_updated_at BEFORE UPDATE ON item_requirement
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
