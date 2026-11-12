-- 0030_link_kinds.sql - MOD-150 milestone 1 (plan D5, D8; ANA-35 §7.6, §7.9 P7): item_link takes
-- three more kinds. Forward-only (R-STO-5). Depends on 0001_init.sql (item_link).
--
-- parent (child -> parent, "is a part of"), duplicates (duplicate -> canonical, "is the same as")
-- and implements (item -> item, "carries out that decision"). 0001 wrote the kind CHECK inline and
-- unnamed, so Postgres named it item_link_kind_check (verified on a scratch database built from
-- 0001_init.sql). It is replaced by the named chk_item_link_kind: the four old kinds first, in
-- their order, then the three new ones, which is LinkKind::ALL's order. Every existing row passes.
--
-- A child has at most one live parent and an item at most one live canonical. The two partial
-- unique indexes back the store's own refusals (PgStore::propose_link's second_parent and
-- second_canonical) against a writer that skips them; their violation surfaces as map_sqlx's
-- generic constraint error. A tombstoned row (deleted_at set) is outside both, so a withdrawn
-- parent may sit beside a live one. No constraint expresses acyclicity or the depth of 8: the store
-- checks both, under an advisory lock (plan D3).
--
-- Not mirrored: the cache's item_link has no CHECK and no column changes, so there is no cache
-- migration. cache_meta.schema_version moves to 30 through PgStore::schema_version(), so each box
-- rebuilds its mirror once on first start; a headless worker never migrates: migrate from a TUI
-- first.

ALTER TABLE item_link DROP CONSTRAINT item_link_kind_check;
ALTER TABLE item_link ADD CONSTRAINT chk_item_link_kind CHECK (kind IN
    ('blocked_by', 'origin', 'relates', 'supersedes', 'parent', 'duplicates', 'implements'));

CREATE UNIQUE INDEX uq_item_link_one_parent ON item_link (from_item_id)
    WHERE kind = 'parent' AND deleted_at IS NULL;
CREATE UNIQUE INDEX uq_item_link_one_canonical ON item_link (from_item_id)
    WHERE kind = 'duplicates' AND deleted_at IS NULL;
