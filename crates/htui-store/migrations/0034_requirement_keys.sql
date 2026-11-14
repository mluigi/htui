-- 0034_requirement_keys.sql - MOD-149 (ANA-35 §11; plan D1, D2, D5). Forward-only (R-STO-5).
--
-- requirement.key stops being generated: it is written at mint, so a workspace's scope row can
-- mint W- keys and a requirement can nest under another as {parent key}.{n} at any depth
-- (R-ENT-1.1.1). DROP EXPRESSION keeps every stored value; the scope rows' existing rows are then
-- rewritten to W-. refines_id points a requirement at the one it refines: nested (same project,
-- same area, child_number set and number NULL) or across owners (a member project's top-level
-- requirement refining its workspace's, keeping its own key and number). child_number comes from
-- requirement_child_counter, one row per parent, numbers never reused.
--
-- Two departures from ANA-35's sketch:
--   1. key SET NOT NULL. The generated column was declared without NOT NULL (0006), so after DROP
--      EXPRESSION an insert that omitted key would store NULL, and the shape CHECK passes NULL.
--   2. refines_id ... ON DELETE SET NULL. A workspace delete takes its scope row's requirements;
--      a member project's refinement of one survives as a plain top-level requirement with its key.
--      A nested child is always in its parent's project, so it is gone with it.
--
-- The area-code CHECK (^[A-Z][A-Z0-9]{1,15}$) and number >= 1 (0006) make every existing key match
-- chk_requirement_key_shape. UNIQUE (project_id, area_code, number) stays: NULL numbers are
-- distinct, so nested children pass it. uq_requirement_child leads with refines_id, so it serves
-- the FK and the refinements read. No COMMENT ON; one new table (TABLES 54 -> 55).
--
-- The mirror recreates requirement with the new columns in
-- cache_migrations/0011_requirement_keys.sql; schema_version becomes 34, so each box rebuilds its
-- mirror once. requirement_child_counter is not mirrored, as requirement_key_counter is not.

ALTER TABLE requirement ALTER COLUMN key DROP EXPRESSION;     -- values kept, stored from now on
ALTER TABLE requirement ALTER COLUMN key SET NOT NULL;
UPDATE requirement r SET key = 'W' || substr(r.key, 2)        -- scope-row rows become W-
  FROM project p WHERE p.id = r.project_id AND p.owner_workspace_id IS NOT NULL;
ALTER TABLE requirement ADD COLUMN refines_id UUID
    CONSTRAINT fk_requirement_refines REFERENCES requirement(id) ON DELETE SET NULL;
ALTER TABLE requirement ADD COLUMN child_number INTEGER
    CONSTRAINT chk_requirement_child_number CHECK (child_number >= 1);
ALTER TABLE requirement ALTER COLUMN number DROP NOT NULL;
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_number_or_child
    CHECK (num_nonnulls(number, child_number) = 1);
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_child_refines
    CHECK (child_number IS NULL OR refines_id IS NOT NULL);
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_key_shape
    CHECK (key ~ '^[RW]-[A-Z][A-Z0-9]*-[1-9][0-9]*(\.[1-9][0-9]*)*$');
ALTER TABLE requirement ADD CONSTRAINT uq_requirement_key UNIQUE (project_id, key);
ALTER TABLE requirement ADD CONSTRAINT uq_requirement_child UNIQUE (refines_id, child_number);
-- UNIQUE (project_id, area_code, number) stays: NULL numbers are distinct, children pass it
CREATE TABLE requirement_child_counter (
    parent_id  UUID PRIMARY KEY REFERENCES requirement(id) ON DELETE CASCADE,
    last_value INTEGER NOT NULL CHECK (last_value >= 0)
);
