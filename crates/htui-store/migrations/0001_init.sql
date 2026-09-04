-- htui schema v1: the 32 tables of `docs/ANA-9.md` section 5, in foreign-key order.
--
-- Thirty-two, not the "thirty" ANA-9 section 3's prose says: the prose count omits `app_setting`
-- and `capability_tag`, both of which are drawn in the same entity map (MOD-6 blueprint H.1). The
-- order below is the blueprint's B.1 table; the DDL bodies are section 5 column for column, CHECK
-- list for CHECK list, index for index, plus `item_key_counter` from 4.1 and `session_event`
-- from 4.3.
--
-- Forward-only (R-STO-5): later ANAs add `000N_*.sql`, they never edit this file. Editing it
-- changes its checksum and `PgStore::connect` then refuses to run against a database that already
-- applied the old text.
--
-- Layout: the `set_updated_at` trigger function first (the loop at the end references it), then
-- statements 1-32, then the trigger loop over the twenty tables that have an `updated_at` column,
-- then the three ALTER TABLEs that add the forward references to `run_step`.

-- --------------------------------------------------------------------------------------------
-- 5.1 Common function
-- --------------------------------------------------------------------------------------------

CREATE FUNCTION set_updated_at() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := clock_timestamp();
    RETURN NEW;
END $$;

-- --------------------------------------------------------------------------------------------
-- 1. app_user (5.2)
-- --------------------------------------------------------------------------------------------

CREATE TABLE app_user (                          -- "user" is reserved in SQL
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    email       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 2. capability_tag (5.2)
-- --------------------------------------------------------------------------------------------

CREATE TABLE capability_tag (
    tag         TEXT PRIMARY KEY,
    description TEXT NOT NULL DEFAULT '',
    seeded      BOOLEAN NOT NULL DEFAULT false
);

-- --------------------------------------------------------------------------------------------
-- 3. box (5.2)
-- --------------------------------------------------------------------------------------------

CREATE TABLE box (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),   -- minted on first launch, kept in box.toml
    user_id         UUID NOT NULL REFERENCES app_user(id),
    hostname        TEXT NOT NULL,
    os_family       TEXT NOT NULL CHECK (os_family IN ('windows','linux','macos')),
    os_version      TEXT NOT NULL,
    arch            TEXT NOT NULL,
    cpu             TEXT NOT NULL DEFAULT '',
    ram_mb          INTEGER,
    gpu_present     BOOLEAN NOT NULL DEFAULT false,
    gpu_vendor      TEXT,
    htui_version    TEXT NOT NULL,               -- re-probe trigger (R-BOX-2)
    probed_tags     TEXT[] NOT NULL DEFAULT '{}',
    declared_tags   TEXT[] NOT NULL DEFAULT '{}',
    quirks          TEXT NOT NULL DEFAULT '',
    settings        JSONB NOT NULL DEFAULT '{}', -- command_limits {class: n} (R-MCP-3), max_concurrent_items (R-ORCH-9)
    registered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_probed_at  TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, hostname)
);
CREATE INDEX idx_box_tags ON box USING GIN ((probed_tags || declared_tags));

-- --------------------------------------------------------------------------------------------
-- 4. box_tool (5.2)
-- --------------------------------------------------------------------------------------------

CREATE TABLE box_tool (                          -- compilers, build tools, shells, container runtime (R-BOX-2)
    box_id      UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    version     TEXT NOT NULL,
    path        TEXT NOT NULL,
    probed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (box_id, name)
);

-- --------------------------------------------------------------------------------------------
-- 5. agent (5.7) -- before phase_agent (15), which references it
-- --------------------------------------------------------------------------------------------

CREATE TABLE agent (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL UNIQUE,         -- 'claude','agy' seeded
    transport      TEXT NOT NULL CHECK (transport IN ('acp','cli')),
    launch         JSONB NOT NULL,               -- {argv: [...], env: {...}}; ANA-4 fixes the shape
    models         TEXT[] NOT NULL DEFAULT '{}',
    default_model  TEXT,
    billing        TEXT NOT NULL CHECK (billing IN ('subscription','per_token')),
    enabled        BOOLEAN NOT NULL DEFAULT true,
    settings       JSONB NOT NULL DEFAULT '{}',  -- adapter-specific, ANA-4
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 6. agent_box (5.7)
-- --------------------------------------------------------------------------------------------

CREATE TABLE agent_box (                         -- per-box enablement, discovery and quota snapshot
    agent_id       UUID NOT NULL REFERENCES agent(id) ON DELETE CASCADE,
    box_id         UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    enabled        BOOLEAN NOT NULL DEFAULT true,
    version        TEXT,
    path           TEXT,
    probed_at      TIMESTAMPTZ,
    quota          JSONB,                        -- {remaining, reset_at, ...} as reported (R-AGT-7)
    quota_at       TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent_id, box_id)
);

-- --------------------------------------------------------------------------------------------
-- 7. workspace (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE workspace (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 8. project (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE project (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug            TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    secret_provider TEXT,                        -- 'infisical' | NULL; ANA-7 may add columns
    secret_scope    TEXT,                        -- provider-specific project/env reference
    settings        JSONB NOT NULL DEFAULT '{}', -- token_budget, retention_days, cached_transcript_steps,
                                                 -- keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch
    created_by      UUID NOT NULL REFERENCES app_user(id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 9. workspace_project (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE workspace_project (
    workspace_id UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id   UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, project_id)
);
CREATE INDEX idx_workspace_project_project ON workspace_project(project_id);

-- --------------------------------------------------------------------------------------------
-- 10. workspace_box_path (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE workspace_box_path (
    workspace_id UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    box_id       UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    root_path    TEXT NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, box_id)
);

-- --------------------------------------------------------------------------------------------
-- 11. repo (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE repo (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id      UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    remote_url      TEXT,
    default_branch  TEXT NOT NULL DEFAULT 'main',
    is_primary      BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);
CREATE UNIQUE INDEX uq_repo_primary ON repo(project_id) WHERE is_primary;   -- exactly one primary per project

-- --------------------------------------------------------------------------------------------
-- 12. repo_box_path (5.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE repo_box_path (
    repo_id     UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    box_id      UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    local_path  TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, box_id)
);

-- --------------------------------------------------------------------------------------------
-- 13. step_graph (5.4) -- before item_kind (17), whose default_graph_id is NOT NULL
-- --------------------------------------------------------------------------------------------

CREATE TABLE step_graph (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name)
);

-- --------------------------------------------------------------------------------------------
-- 14. step_graph_phase (5.4)
-- --------------------------------------------------------------------------------------------

CREATE TABLE step_graph_phase (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    graph_id         UUID NOT NULL REFERENCES step_graph(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    name             TEXT NOT NULL,              -- 'prd','plan','implement','review','research','verdict','reproduce','fix',...
    fan_out          INTEGER NOT NULL DEFAULT 1 CHECK (fan_out >= 1),
    gate             TEXT NOT NULL DEFAULT 'always' CHECK (gate IN ('always','on_failure','never')),
    gate_hard        BOOLEAN NOT NULL DEFAULT false,
    retry_limit      INTEGER NOT NULL DEFAULT 1 CHECK (retry_limit >= 0),
    input_kinds      TEXT[] NOT NULL DEFAULT '{}',
    output_kind      TEXT NOT NULL,              -- document.kind this phase produces; defaults to name
    isolation        TEXT CHECK (isolation IN ('worktree','copy','shared_serialized','local')),  -- NULL = project default
    command_queue    TEXT NOT NULL DEFAULT 'fan_out_only' CHECK (command_queue IN ('off','fan_out_only','always')),
    verify_command   TEXT,
    template_name    TEXT NOT NULL,              -- prompt_template.name, defaults to phase name
    template_version INTEGER,                    -- NULL = follow latest
    token_budget     INTEGER,                    -- NULL = project.settings.token_budget
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (graph_id, position),
    UNIQUE (graph_id, name)
);

-- --------------------------------------------------------------------------------------------
-- 15. phase_agent (5.4)
-- --------------------------------------------------------------------------------------------

CREATE TABLE phase_agent (                       -- candidate agents in priority order (R-ORCH-1, R-AGT-8)
    phase_id    UUID NOT NULL REFERENCES step_graph_phase(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    agent_id    UUID NOT NULL REFERENCES agent(id),
    model       TEXT NOT NULL,
    PRIMARY KEY (phase_id, position)
);

-- --------------------------------------------------------------------------------------------
-- 16. prompt_template (5.4)
-- --------------------------------------------------------------------------------------------

CREATE TABLE prompt_template (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,                   -- phase name; defaults are copied into each new project
    version     INTEGER NOT NULL CHECK (version >= 1),
    body        TEXT NOT NULL,                   -- placeholder contract per ANA-5
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, name, version)
);

-- --------------------------------------------------------------------------------------------
-- 17. item_kind (5.5)
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_kind (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix           TEXT NOT NULL CHECK (prefix ~ '^[A-Z][A-Z0-9]{1,15}$'),
    name             TEXT NOT NULL,              -- 'analysis','feature','bug','refactor','tooling' seeded
    description      TEXT NOT NULL DEFAULT '',
    default_graph_id UUID NOT NULL REFERENCES step_graph(id),
    position         INTEGER NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, prefix),
    UNIQUE (project_id, name)
);

-- --------------------------------------------------------------------------------------------
-- 18. item_key_counter (4.1)
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_key_counter (
    project_id  UUID    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    prefix      TEXT    NOT NULL,
    last_value  INTEGER NOT NULL CHECK (last_value >= 0),
    PRIMARY KEY (project_id, prefix)
);

-- --------------------------------------------------------------------------------------------
-- 19. item (5.5)
-- --------------------------------------------------------------------------------------------

CREATE TABLE item (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id     UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    kind_id        UUID NOT NULL REFERENCES item_kind(id),
    key_prefix     TEXT NOT NULL,
    key_number     INTEGER NOT NULL CHECK (key_number >= 1),
    key            TEXT GENERATED ALWAYS AS (key_prefix || '-' || key_number::text) STORED,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'open' CHECK (status IN
                     ('open','queued','in_progress','awaiting_approval','blocked','done','failed','closed')),
    priority       SMALLINT NOT NULL DEFAULT 0,  -- R-ORCH-6 ordering; higher first
    required_tags  TEXT[] NOT NULL DEFAULT '{}', -- R-ORCH-10
    touched_paths  TEXT[] NOT NULL DEFAULT '{}', -- R-ORCH-9 declared overlap set, repo-relative globs
    step_graph_id  UUID REFERENCES step_graph(id),  -- NULL = kind default
    version        INTEGER NOT NULL DEFAULT 1,   -- 4.2
    created_by     UUID NOT NULL REFERENCES app_user(id),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at      TIMESTAMPTZ,
    UNIQUE (project_id, key_prefix, key_number)
);
CREATE INDEX idx_item_project_status ON item(project_id, status);
CREATE INDEX idx_item_required_tags ON item USING GIN (required_tags);
CREATE INDEX idx_item_updated_at ON item(project_id, updated_at);   -- cache cursor

-- --------------------------------------------------------------------------------------------
-- 20. item_revision (5.5)
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_revision (
    item_id        UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL,
    required_tags  TEXT[] NOT NULL,
    author_id      UUID NOT NULL REFERENCES app_user(id),
    box_id         UUID REFERENCES box(id) ON DELETE SET NULL,
    reason         TEXT NOT NULL DEFAULT '',     -- 'created','edited','divergence_resolution','imported',...
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, version)
);

-- --------------------------------------------------------------------------------------------
-- 21. item_link (5.5) -- proposed_by_step_id gains its REFERENCES at the end of this file
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_link (                         -- from --kind--> to; 'blocked_by': from is blocked by to
    from_item_id  UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    to_item_id    UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL CHECK (kind IN ('blocked_by','origin','relates','supersedes')),
    proposed_by_step_id UUID,                    -- NULL = importer; FK added below (forward reference)
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at    TIMESTAMPTZ,                   -- tombstone; live edges have NULL
    PRIMARY KEY (from_item_id, to_item_id, kind),
    CHECK (from_item_id <> to_item_id)
);
CREATE INDEX idx_item_link_to ON item_link(to_item_id) WHERE deleted_at IS NULL;

-- --------------------------------------------------------------------------------------------
-- 22. item_note (5.5) -- via_step_id gains its REFERENCES at the end of this file
-- --------------------------------------------------------------------------------------------

CREATE TABLE item_note (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id     UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    created_by  UUID NOT NULL REFERENCES app_user(id),
    box_id      UUID REFERENCES box(id) ON DELETE SET NULL,
    via_step_id UUID,                            -- set when added through MCP note_add; FK added below
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_item_note_item ON item_note(item_id, created_at);

-- --------------------------------------------------------------------------------------------
-- 23. document (5.5) -- produced_by_step_id gains its REFERENCES at the end of this file
-- --------------------------------------------------------------------------------------------

CREATE TABLE document (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL,           -- open set: phase output kinds plus 'summary'
    version             INTEGER NOT NULL CHECK (version >= 1),
    title               TEXT NOT NULL,
    body                TEXT NOT NULL,
    produced_by_step_id UUID,                    -- NULL = written by hand; FK added below
    created_by          UUID NOT NULL REFERENCES app_user(id),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (item_id, kind, version)
);

-- --------------------------------------------------------------------------------------------
-- 24. skill (5.6)
-- --------------------------------------------------------------------------------------------

CREATE TABLE skill (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,            -- global library; scoping is by binding
    description TEXT NOT NULL DEFAULT '',
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 25. skill_version (5.6)
-- --------------------------------------------------------------------------------------------

CREATE TABLE skill_version (
    skill_id    UUID NOT NULL REFERENCES skill(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL CHECK (version >= 1),
    body        TEXT NOT NULL,
    created_by  UUID NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (skill_id, version)
);

-- --------------------------------------------------------------------------------------------
-- 26. skill_binding (5.6) -- UNIQUE NULLS NOT DISTINCT is why Postgres 16 is the floor
-- --------------------------------------------------------------------------------------------

CREATE TABLE skill_binding (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    skill_id       UUID NOT NULL REFERENCES skill(id) ON DELETE CASCADE,
    project_id     UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    phase_id       UUID REFERENCES step_graph_phase(id) ON DELETE CASCADE,   -- NULL = project level
    pinned_version INTEGER,                      -- NULL = follow latest
    position       INTEGER NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)
);

-- --------------------------------------------------------------------------------------------
-- 27. run (5.8)
-- --------------------------------------------------------------------------------------------

CREATE TABLE run (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id       UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    item_id          UUID REFERENCES item(id) ON DELETE CASCADE,      -- NULL for free-standing chat
    kind             TEXT NOT NULL CHECK (kind IN ('graph','chat')),
    mode             TEXT NOT NULL CHECK (mode IN ('manual','auto')),
    status           TEXT NOT NULL DEFAULT 'queued' CHECK (status IN
                       ('queued','running','awaiting_approval','done','failed','cancelled')),
    target_box_id    UUID NOT NULL REFERENCES box(id),               -- R-ORCH-12 reserved: executes only when local
    executing_box_id UUID REFERENCES box(id),
    graph_snapshot   JSONB,                      -- step_graph + phases + phase_agent at start (R-ORCH-11)
    started_by       UUID NOT NULL REFERENCES app_user(id),
    queued_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at       TIMESTAMPTZ,
    finished_at      TIMESTAMPTZ,
    failure          TEXT,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_run_item ON run(item_id, queued_at DESC);
CREATE INDEX idx_run_project_status ON run(project_id, status);

-- --------------------------------------------------------------------------------------------
-- 28. run_step (5.8)
-- --------------------------------------------------------------------------------------------

CREATE TABLE run_step (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id         UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,             -- phase index in the snapshot
    attempt        INTEGER NOT NULL DEFAULT 1,   -- retry / review loop counter
    fanout_index   INTEGER NOT NULL DEFAULT 0,   -- 0..fan_out-1
    phase_name     TEXT NOT NULL,                -- 'chat' for run.kind = 'chat'
    agent_id       UUID REFERENCES agent(id),
    model          TEXT,
    status         TEXT NOT NULL DEFAULT 'pending' CHECK (status IN
                     ('pending','running','awaiting_approval','done','failed','cancelled','superseded')),
    gate_outcome   TEXT CHECK (gate_outcome IN ('approved','rejected','retried','skipped')),
    gate_note      TEXT,
    selected       BOOLEAN,                      -- fan-out winner; NULL when fan_out = 1
    exit_code      INTEGER,
    prompt_digest  TEXT,                         -- sha256 of session_event seq 0
    trim_record    JSONB,                        -- R-PRM-3: what was trimmed and by how much
    usage          JSONB,                        -- summed from usage events; kept for the Runs tab
    isolation_path TEXT,                         -- worktree / copy location on the executing box
    started_at     TIMESTAMPTZ,
    finished_at    TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_id, position, attempt, fanout_index)
);

-- --------------------------------------------------------------------------------------------
-- 29. run_step_commit (5.8)
-- --------------------------------------------------------------------------------------------

CREATE TABLE run_step_commit (                   -- per repo, since a project may have several
    run_step_id  UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    repo_id      UUID NOT NULL REFERENCES repo(id),
    before_hash  TEXT NOT NULL,
    after_hash   TEXT,
    PRIMARY KEY (run_step_id, repo_id)
);

-- --------------------------------------------------------------------------------------------
-- 30. session_event (4.3)
-- --------------------------------------------------------------------------------------------

CREATE TABLE session_event (
    run_step_id   UUID        NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    seq           INTEGER     NOT NULL,             -- 0-based, assigned by htui at capture
    turn          INTEGER     NOT NULL DEFAULT 0,   -- increments on each prompt/follow_up
    kind          TEXT        NOT NULL,
    role          TEXT        NOT NULL,             -- 'user' | 'agent' | 'htui'
    tool_call_id  TEXT,                             -- pairs tool_call / tool_result / edit_proposal / permission_*
    payload       JSONB       NOT NULL,
    raw           JSONB,                            -- wire message, only when project.settings.keep_raw_events
    at            TIMESTAMPTZ NOT NULL,             -- capture time on the executing box
    PRIMARY KEY (run_step_id, seq),
    CONSTRAINT chk_event_kind CHECK (kind IN (
        'prompt', 'follow_up', 'assistant_text', 'thought', 'tool_call', 'tool_result',
        'edit_proposal', 'permission_request', 'permission_answer', 'plan', 'usage',
        'error', 'done', 'other')),
    CONSTRAINT chk_event_role CHECK (role IN ('user', 'agent', 'htui'))
);
CREATE INDEX idx_session_event_tool ON session_event (run_step_id, tool_call_id)
    WHERE tool_call_id IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 31. command_run (5.8)
-- --------------------------------------------------------------------------------------------

CREATE TABLE command_run (                       -- R-MCP-3 queue; runtime state lives in Postgres like everything else
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_step_id  UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    box_id       UUID NOT NULL REFERENCES box(id),
    class        TEXT NOT NULL,                  -- 'build','test','run',... limits in box.settings.command_limits
    command      TEXT NOT NULL,
    cwd          TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','running','done','failed','cancelled')),
    exit_code    INTEGER,
    output       TEXT,                           -- scrubbed
    queued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at   TIMESTAMPTZ,
    finished_at  TIMESTAMPTZ
);
CREATE INDEX idx_command_run_queue ON command_run(box_id, class, status, queued_at);

-- --------------------------------------------------------------------------------------------
-- 32. app_setting (5.9)
-- --------------------------------------------------------------------------------------------

CREATE TABLE app_setting (
    key        TEXT PRIMARY KEY,                 -- 'cache_refresh_seconds','cache_overlap_seconds','per_token_cap_run',...
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- --------------------------------------------------------------------------------------------
-- 5.1 trigger loop: exactly the twenty tables that have an updated_at column.
--
-- BEFORE UPDATE only, so an INSERT that supplies an explicit updated_at (the demo loader) keeps
-- it while every UPDATE gets clock_timestamp() -- which is what the cache cursor of 4.4 rides on,
-- and why no write path may set updated_at by hand. The append-only and timestamp-free tables
-- (item_note, document, item_revision, session_event, run_step_commit, command_run, box_tool,
-- phase_agent, workspace_project, item_key_counter, capability_tag, skill_version) are
-- deliberately absent.
-- --------------------------------------------------------------------------------------------

DO $$ DECLARE t text; BEGIN
  FOREACH t IN ARRAY ARRAY['app_user','box','workspace','project','repo','repo_box_path',
    'workspace_box_path','item_kind','item','item_link','step_graph','step_graph_phase',
    'prompt_template','skill','skill_binding','agent','agent_box','run','run_step','app_setting']
  LOOP EXECUTE format('CREATE TRIGGER trg_%s_updated_at BEFORE UPDATE ON %I
                       FOR EACH ROW EXECUTE FUNCTION set_updated_at()', t, t);
  END LOOP; END $$;

-- --------------------------------------------------------------------------------------------
-- The three forward references to run_step (5.0), declared as plain UUID above.
-- --------------------------------------------------------------------------------------------

ALTER TABLE item_link
    ADD CONSTRAINT fk_item_link_step FOREIGN KEY (proposed_by_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;

ALTER TABLE item_note
    ADD CONSTRAINT fk_item_note_step FOREIGN KEY (via_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;

ALTER TABLE document
    ADD CONSTRAINT fk_document_step FOREIGN KEY (produced_by_step_id)
        REFERENCES run_step(id) ON DELETE SET NULL;
