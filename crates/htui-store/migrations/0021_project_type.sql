-- 0021_project_type.sql - MOD-146 (ANA-35 7.4, 7.9 P3; plan D1). Forward-only (R-STO-5).
--
-- project_type is ANA-35 7.4's registry: a project's type is set at creation and its seed copied
-- in once, so editing or adding a type never re-seeds a project. The six built-ins carry no
-- recipe (code holds theirs, so ANA-33 can change the graph format); a user-defined type must
-- (chk_project_type_seed). coding marks the types ANA-39 and MOD-128 read; selectable is false
-- for workspace, which only MOD-147's scope rows take. The INSERT below is the only writer.
--
-- Every existing project takes project.type's default 'software', as does every project created
-- before MOD-148's picker. No index stands behind fk_project_type: projects are few and no type
-- is deleted. The mirror gains project.type in cache_migrations/0007_project_type.sql;
-- project_type is not mirrored, like persona. schema_version becomes 20, so each box rebuilds
-- its mirror once on first start. A headless worker never migrates: migrate from a TUI first.

CREATE TABLE project_type (
    slug         TEXT        PRIMARY KEY
                             CONSTRAINT chk_project_type_slug CHECK (slug ~ '^[a-z][a-z0-9-]{0,31}$'),
    name         TEXT        NOT NULL,
    description  TEXT        NOT NULL DEFAULT '',
    coding       BOOLEAN     NOT NULL DEFAULT false,
    builtin      BOOLEAN     NOT NULL DEFAULT false,
    selectable   BOOLEAN     NOT NULL DEFAULT true,
    seed         JSONB,
    seed_version INTEGER     NOT NULL DEFAULT 1,
    created_by   UUID        REFERENCES app_user(id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_project_type_seed CHECK (builtin OR seed IS NOT NULL)
);

-- 0001's set_updated_at, BEFORE UPDATE only (0012_persona.sql's shape).
CREATE TRIGGER trg_project_type_updated_at BEFORE UPDATE ON project_type
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The six built-ins, seed::PROJECT_TYPES verbatim (tests/migrations.rs pins the two equal).
INSERT INTO project_type (slug, name, description, coding, builtin, selectable) VALUES
    ('docs',      'Documents',  'Documents outlined, drafted and reviewed.',                           false, true, true),
    ('general',   'General',    'Tasks carried out and reviewed.',                                     false, true, true),
    ('ops',       'Operations', 'Operations planned, executed and verified; incidents investigated.', false, true, true),
    ('research',  'Research',   'Questions answered and research concluded with a verdict.',          false, true, true),
    ('software',  'Software',   'Code in repositories: analyses, features, fixes.',                   true,  true, true),
    ('workspace', 'Workspace',  'A workspace''s own items; not offered for new projects.',            false, true, false);

ALTER TABLE project
    ADD COLUMN type TEXT NOT NULL DEFAULT 'software'
        CONSTRAINT fk_project_type REFERENCES project_type(slug);

COMMENT ON TABLE project_type IS
    'MOD-146 D1 (ANA-35 7.4): the project type registry. A type seeds a project once, at '
    'creation; editing or adding a type never re-seeds one. Six built-in rows; not mirrored.';
COMMENT ON COLUMN project_type.slug IS
    'MOD-146 D1: the key project.type references; a letter, then up to 31 of a-z, 0-9 and '
    'hyphens (chk_project_type_slug).';
COMMENT ON COLUMN project_type.coding IS
    'MOD-146 D1: a software-like type (software alone among the built-ins); ANA-39 and MOD-128 '
    'read it.';
COMMENT ON COLUMN project_type.selectable IS
    'MOD-146 D1: false for workspace, which only scope rows take (MOD-147); a new-project picker '
    'offers the rest.';
COMMENT ON COLUMN project_type.seed IS
    'MOD-146 D1: the JSONB recipe of a user-defined type; NULL for a built-in, whose recipe code '
    'holds (chk_project_type_seed).';
COMMENT ON COLUMN project.type IS
    'MOD-146 D1: the project type slug, set at creation; every project before MOD-148 is '
    'software.';
