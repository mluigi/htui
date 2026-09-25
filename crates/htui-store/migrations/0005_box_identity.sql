-- --------------------------------------------------------------------------------------------
-- 0005_box_identity.sql - MOD-7 milestone 1: a box is keyed on its box.toml id, not its hostname.
-- Forward-only (R-STO-5): 0001_init.sql is never edited; this file moves what it has to.
--
-- ANA-16 C4 and PRD D1: UNIQUE (user_id, hostname) made a renamed box a duplicate primary key
-- (box.toml keeps the id across a rename) and a cloned hostname a merge into another box's row.
-- Registration now looks the row up by id and checks a keyed machine fingerprint instead.
--
-- Three columns, none mirrored (MIRRORED_TABLES unchanged, no cache migration):
--   machine_fingerprint  the keyed hash only; the CHECK makes "no raw identity" a schema fact.
--   edit_version         milestone 2's compare-and-set token; nothing reads or writes it yet.
--   probe_spec_digest    re-probes when app_setting.box_probe_spec changes (plan D18).
-- --------------------------------------------------------------------------------------------

ALTER TABLE box DROP CONSTRAINT box_user_id_hostname_key;

ALTER TABLE box ADD COLUMN machine_fingerprint TEXT
    CHECK (machine_fingerprint IS NULL OR machine_fingerprint ~ '^[0-9a-f]{64}$');
ALTER TABLE box ADD COLUMN edit_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE box ADD COLUMN probe_spec_digest TEXT
    CHECK (probe_spec_digest IS NULL OR probe_spec_digest ~ '^[0-9a-f]{64}$');

COMMENT ON COLUMN box.machine_fingerprint IS
    'MOD-7 D1: HMAC-SHA256 keyed by the OS machine identity (/etc/machine-id, IOPlatformUUID, MachineGuid) over the constant htui/box-fingerprint/v1, lowercase hex. NULL means no identity was readable. It checks a box.toml id and never keys the row: a mismatch means a copied box.toml, and a new box is minted.';
COMMENT ON COLUMN box.edit_version IS
    'MOD-7 D14: compare-and-set token of the declared_tags, quirks and settings editors, bumped by them only. Registration and the probe never write it, so a reconnect cannot stale an open editor.';
COMMENT ON COLUMN box.probe_spec_digest IS
    'MOD-7 D18: sha256 hex of the effective box probe spec (the compiled seed overlaid by app_setting.box_probe_spec) at the last successful probe. Another digest re-probes at the next connect. NULL means never probed since 0005.';
COMMENT ON COLUMN box.htui_version IS
    'MOD-7 D5: the htui version at the last successful probe, and the re-probe trigger (R-BOX-2). Inserted at first registration; rewritten only by the probe writer, never by a reconnect.';
