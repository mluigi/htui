-- ------------------------------------------------------------------------------------------------
-- Mirror side of ANA-11 (MOD-38, plan D12; `docs/ANA-11.md` 5.2 with one deviation).
--
-- `item` gains `resolution`. Four tables join the mirror, same table and column names as
-- Postgres, 4.4's type mapping (`0001_mirror.sql`): `requirement_spec` (the deviation from 5.2 -
-- one small row per project, so `requirement_spec` never has to answer "none or not cached"),
-- `requirement_area`, `requirement` and `item_requirement`. `item_requirement` holds live rows
-- only: a tombstone is a delete, as `item_link`'s is, so the column `deleted_at` is not here.
-- `requirement.key` is a plain column: the generated expression is Postgres's, the mirror
-- stores its value. `requirement_revision` and `requirement_key_counter` are not mirrored;
-- `requirement_revisions` answers `None` offline, as `step_events` does for an uncached step.
--
-- `cache_meta.schema_version` moves to 5 through `PgStore::schema_version()`, which forces a
-- full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE item ADD COLUMN resolution TEXT;

CREATE TABLE requirement_spec (
    project_id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, preamble TEXT NOT NULL DEFAULT '',
    version INTEGER NOT NULL DEFAULT 1, updated_at INTEGER NOT NULL);

CREATE TABLE requirement_area (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, code TEXT NOT NULL, title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_requirement_area_project ON requirement_area(project_id, position, code);

CREATE TABLE requirement (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, area_id TEXT NOT NULL,
    area_code TEXT NOT NULL, number INTEGER NOT NULL, key TEXT NOT NULL,
    body TEXT NOT NULL, rationale TEXT NOT NULL DEFAULT '', priority TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active', version INTEGER NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_requirement_order ON requirement(project_id, area_code, number);

CREATE TABLE item_requirement (
    item_id TEXT NOT NULL, requirement_id TEXT NOT NULL, kind TEXT NOT NULL,
    requirement_version INTEGER NOT NULL, proposed_by_step_id TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY (item_id, requirement_id, kind));
CREATE INDEX idx_cache_item_requirement_req ON item_requirement(requirement_id);
