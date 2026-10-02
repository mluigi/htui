-- 0012_persona.sql - MOD-26 milestone 1 (plan D1, D5; PRD Q-storage, Q-binding).
-- Forward-only (R-STO-5).
--
-- persona holds the agent persona registry: one global row per name, like skill, carrying the
-- role text stage 3 renders ahead of the phase template (body), a narrowing of the agent row's
-- tool exposure (tools) and deny-only permission rules (permission). A persona never carries or
-- influences a model (R-AGT-8). step_graph_phase.persona_id binds at most one persona to a
-- phase; ON DELETE RESTRICT keeps a bound persona from vanishing under a graph (milestone 1 has
-- no delete). A started run reads personas only from its graph_snapshot, so editing a row never
-- changes a started run's steps. Neither is mirrored, but schema_version becomes 12, so each box
-- rebuilds its mirror once on first start. A headless worker never migrates: migrate from a TUI
-- first.

CREATE TABLE persona (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL CONSTRAINT uq_persona_name UNIQUE,
    description TEXT        NOT NULL DEFAULT '',
    body        TEXT        NOT NULL,
    tools       JSONB       NOT NULL DEFAULT '{}',
    permission  JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 0001's set_updated_at, BEFORE UPDATE only (0006_requirements.sql:135-142's shape).
CREATE TRIGGER trg_persona_updated_at BEFORE UPDATE ON persona
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

ALTER TABLE step_graph_phase
    ADD COLUMN persona_id UUID NULL
        CONSTRAINT fk_step_graph_phase_persona REFERENCES persona(id) ON DELETE RESTRICT;

COMMENT ON COLUMN persona.id IS
    'MOD-26 D1: client-minted UUIDv7; what step_graph_phase.persona_id references.';
COMMENT ON COLUMN persona.name IS
    'MOD-26 D1, D3: the registry name, unique; 1-64 of a-z, 0-9 and single inner hyphens, '
    'checked by the writers. A run snapshot freezes a persona under this name.';
COMMENT ON COLUMN persona.description IS
    'MOD-26 D1: the one-line summary a picker shows; never rendered into a prompt.';
COMMENT ON COLUMN persona.body IS
    'MOD-26 D1, D13: the role text stage 3 renders as the protected persona section ahead of '
    'the phase template. Never blank.';
COMMENT ON COLUMN persona.tools IS
    'MOD-26 D2, D10: {allow, deny, deny_kinds, command_run}, narrow-only against the agent row. '
    'allow keeps built-in tool names (empty keeps all), deny removes tool names, deny_kinds denies '
    'ACP tool kinds (read, edit, delete, move, search, execute, fetch), command_run false '
    'withdraws the command queue. Unknown keys are refused.';
COMMENT ON COLUMN persona.permission IS
    'MOD-26 D2, D10: {default, rules[]}, deny-only. default is null, ask or deny; every rule '
    'answers reject_once or reject_always and is evaluated before the agent row rules and '
    'remembered choices. Unknown keys are refused.';
COMMENT ON COLUMN persona.created_at IS
    'MOD-26 D1: when the row was inserted.';
COMMENT ON COLUMN persona.updated_at IS
    'MOD-26 D1: the update_persona compare-and-set token, stamped by trg_persona_updated_at.';
COMMENT ON COLUMN step_graph_phase.persona_id IS
    'MOD-26 D5: the persona this phase runs under; NULL for none. StartRun freezes it by name '
    'and content into run.graph_snapshot (phases[].persona, personas[]); ON DELETE RESTRICT.';
