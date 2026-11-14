-- 0034_item_artifact_link.sql - MOD-150 milestone 3 (plan D1; ANA-35 §7.9 P7): links from an item
-- to a document, one version of it pinned. Forward-only (R-STO-5). Depends on 0001_init.sql (item,
-- document, run_step, set_updated_at) and 0026_workspace_scope.sql (document.project_id).
--
-- ANA-35 §7.9's DDL as written, with its constraints named. A link targets exactly one of a
-- document and an attachment (chk_item_artifact_link_target); to_attachment_id has no foreign key
-- yet: MOD-114 (3)'s migration adds the attachment table and the key, and nothing writes the
-- column before then. The unique is NULLS NOT DISTINCT, so the NULL attachment of a document link
-- does not let the same link land twice; PgStore::propose_document_link upserts on it. The kinds
-- are the three of LinkKind a link to a document may take, in ArtifactLinkKind::ALL's order.
--
-- A link goes with its item and with its document (ON DELETE CASCADE on both), and loses a
-- proposing step that goes (SET NULL). Removal by the store is a tombstone (deleted_at), so the
-- cache cursor on updated_at sees it; trg_item_artifact_link_updated_at moves updated_at, as
-- 0006_requirements.sql's triggers do. idx_item_artifact_link_document backs the read of a
-- document's links and is not partial: the document foreign key's cascade and a delete's reach
-- count read tombstones too. idx_item_artifact_link_step backs the proposing step's SET NULL.
--
-- Mirrored from MOD-150 M3 T2 (cache_migrations/0011_item_artifact_link.sql). cache_meta.
-- schema_version moves to 34 through PgStore::schema_version(), so each box rebuilds its mirror
-- once on first start; a headless worker never migrates: migrate from a TUI first.

CREATE TABLE item_artifact_link (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_item_id        UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL
        CONSTRAINT chk_item_artifact_link_kind CHECK (kind IN ('origin', 'relates', 'implements')),
    to_document_id      UUID REFERENCES document(id) ON DELETE CASCADE,
    to_attachment_id    UUID,          -- FK added by MOD-114 (3)'s migration
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ,
    CONSTRAINT chk_item_artifact_link_target
        CHECK (num_nonnulls(to_document_id, to_attachment_id) = 1),
    CONSTRAINT uq_item_artifact_link
        UNIQUE NULLS NOT DISTINCT (from_item_id, kind, to_document_id, to_attachment_id)
);

CREATE TRIGGER trg_item_artifact_link_updated_at BEFORE UPDATE ON item_artifact_link
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX idx_item_artifact_link_updated_at ON item_artifact_link (updated_at);
CREATE INDEX idx_item_artifact_link_document ON item_artifact_link (to_document_id);
CREATE INDEX idx_item_artifact_link_step ON item_artifact_link (proposed_by_step_id);
