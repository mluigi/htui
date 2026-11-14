-- ------------------------------------------------------------------------------------------------
-- Mirror side of MOD-150 milestone 3 (plan D9; migrations/0034_item_artifact_link.sql): links from
-- an item to one pinned version of a document. item_link's style (0001_mirror.sql): TEXT and
-- INTEGER, no CHECK, no foreign key, and no deleted_at: the mirror holds live rows only, a
-- tombstone is a delete by id. to_attachment_id is mirrored though nothing writes it before
-- MOD-114 (3). idx_cache_item_artifact_link_from backs CacheStore::links' read of the reached
-- items' document links.
--
-- A new table, so this applies over a populated 0010 file: CACHE_MIGRATOR runs before the
-- schema_version check (cache/mod.rs). cache_meta.schema_version moves to 34 through
-- PgStore::schema_version(), which forces a full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
CREATE TABLE item_artifact_link (
    id TEXT PRIMARY KEY, from_item_id TEXT NOT NULL, kind TEXT NOT NULL, to_document_id TEXT,
    to_attachment_id TEXT, proposed_by_step_id TEXT, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_item_artifact_link_from ON item_artifact_link(from_item_id);
