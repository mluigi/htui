-- 0036_indexer_lease.sql - MOD-159 milestone 1 (plan D2). Forward-only (R-STO-5).
--
-- The knowledge index moves from one shared Qdrant collection to one per workspace
-- (htui_ws_<workspace uuid, simple form>) plus a global one (htui_global), and exactly one htui
-- process indexes each: the holder of that collection's indexer_lease row. Every TUI and worker
-- competes for every collection; the first take of a collection creates its row
-- (INSERT ... ON CONFLICT). The collection CHECK is IndexedCollection::name's shape, so the
-- legacy htui_concepts_v2 can never hold a lease.
--
-- The lease columns are the run lease's (0003): lease_owner is a per-process token, NULL when
-- free; lease_expires_at is stamped and judged by clock_timestamp() plus a TTL, never a box
-- clock (MOD-40 plan D10). A release clears the owner, expires the row now and keeps
-- lease_box_id for display. pass_requested_at is MOD-159 M3's (Ctrl+R / --index-items ask the
-- holder for a pass); M1 never writes it.
--
-- No foreign keys: a deleted workspace's row is inert (cleaning its Qdrant collection is
-- deferred), and lease_box_id only names the holder for display (LEFT JOIN box), so a box delete
-- reaches nothing new.
--
-- Not mirrored, like workspace_stop: live coordination state, read through the store. The cache
-- has no indexer_lease, MIRRORED_TABLES is unchanged and there is no cache migration;
-- cache_meta.schema_version moves to 36 through PgStore::schema_version(), so each box rebuilds
-- its mirror once on first start. A headless worker never migrates: migrate from a TUI first.

CREATE TABLE indexer_lease (
    collection        TEXT PRIMARY KEY
                      CONSTRAINT chk_indexer_lease_collection
                      CHECK (collection ~ '^htui_(global|ws_[0-9a-f]{32})$'),
    lease_owner       UUID,
    lease_box_id      UUID,
    lease_expires_at  TIMESTAMPTZ,
    pass_requested_at TIMESTAMPTZ
);

COMMENT ON TABLE indexer_lease IS
    'MOD-159 D2: one row per Qdrant collection (htui_ws_<workspace uuid> or htui_global); its '
    'holder alone indexes that collection. Created by the first take. Not mirrored.';
COMMENT ON COLUMN indexer_lease.lease_owner IS
    'MOD-159 D2: the holder''s per-process token; NULL = free. Never read back by the store.';
COMMENT ON COLUMN indexer_lease.lease_box_id IS
    'MOD-159 D2: the holder''s box, kept after a release; no FK (display only, LEFT JOIN box).';
COMMENT ON COLUMN indexer_lease.lease_expires_at IS
    'MOD-159 D2: clock_timestamp() plus the TTL at the last take or renewal; now at a release.';
COMMENT ON COLUMN indexer_lease.pass_requested_at IS
    'MOD-159 D2: when Ctrl+R or --index-items asked the holder for a pass (M3); unused in M1.';
