-- 0042_mcp_server.sql - MOD-127 milestone 2 (D2, D3, D4, D10). Forward-only (R-STO-5).
--
-- mcp_server is the registry of MCP servers a step's agent may be given beyond htui's own: one
-- global row per name, like skill but with no versions (an edit is a compare-and-set on updated_at).
-- A secret is held by reference: an env or header value is {"literal": ..} or {"secret": <name>},
-- where the name is that of a project secret R-SEC-2's provider supplies at launch; the value is
-- never stored. A started run reads its servers only from its graph_snapshot, so editing or deleting
-- a row never changes a started run.
--
-- toolbox_binding takes the second kind. Its ref_id has no foreign key (0041), so the write path
-- checks the server exists and delete_mcp_server removes its attachments in the same transaction.
-- An mcp_server attachment has no versions and no globs: pinned_version stays NULL, activation is
-- always or off, and globs is empty (toolbox_binding_mcp_shape).
--
-- run_step.trim_record is restated for record v 6, which gains mcp_choices[]. Neither table is
-- mirrored (skills are not). One forward step, run before the new binary uses the tables.

CREATE TABLE mcp_server (
    id          UUID        PRIMARY KEY,
    name        TEXT        NOT NULL CONSTRAINT mcp_server_name_key UNIQUE,
    description TEXT        NOT NULL DEFAULT '',
    transport   TEXT        NOT NULL CONSTRAINT mcp_server_transport_check
                            CHECK (transport IN ('stdio', 'http')),
    command     TEXT        NOT NULL DEFAULT '',
    args        TEXT[]      NOT NULL DEFAULT '{}',
    url         TEXT,
    headers     JSONB       NOT NULL DEFAULT '[]',
    env         JSONB       NOT NULL DEFAULT '[]',
    allow       TEXT[]      NOT NULL DEFAULT '{}',
    deny        TEXT[]      NOT NULL DEFAULT '{}',
    created_by  UUID        NOT NULL REFERENCES app_user(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT mcp_server_shape CHECK (
        (transport = 'stdio' AND command <> '' AND url IS NULL)
        OR (transport = 'http' AND url IS NOT NULL AND command = '' AND cardinality(args) = 0)
    )
);

-- 0001's set_updated_at, BEFORE UPDATE only (0012_persona.sql's shape).
CREATE TRIGGER trg_mcp_server_updated_at BEFORE UPDATE ON mcp_server
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

ALTER TABLE toolbox_binding
    DROP CONSTRAINT toolbox_binding_kind_check,
    ADD CONSTRAINT toolbox_binding_kind_check CHECK (kind IN ('skill', 'mcp_server')),
    ADD CONSTRAINT toolbox_binding_mcp_shape CHECK (
        kind <> 'mcp_server'
        OR (pinned_version IS NULL
            AND activation IN ('always', 'off')
            AND cardinality(globs) = 0)
    );

COMMENT ON TABLE mcp_server IS
    'MOD-127 M2 D2: an MCP server a step may be given beyond htui''s own, attached by toolbox_binding '
    'rows of kind mcp_server. One row, no versions. Not mirrored.';
COMMENT ON COLUMN mcp_server.name IS
    'a-z start, then a-z, 0-9, _ or -, at most 32 bytes, never htui; the agent sees mcp__<name>__<tool>; '
    'checked on write';
COMMENT ON COLUMN mcp_server.description IS 'the picker''s one-liner; may be empty';
COMMENT ON COLUMN mcp_server.transport IS 'stdio (command, args, env) or http (url, headers)';
COMMENT ON COLUMN mcp_server.command IS 'the program to start; empty for http';
COMMENT ON COLUMN mcp_server.args IS 'the program''s arguments; empty for http';
COMMENT ON COLUMN mcp_server.url IS 'the endpoint of an http server; NULL for stdio';
COMMENT ON COLUMN mcp_server.headers IS
    'ordered [[name, {"literal": text} | {"secret": project-secret name}], ..]; a literal under a '
    'name that looks secret is refused on write';
COMMENT ON COLUMN mcp_server.env IS
    'ordered [[name, {"literal": text} | {"secret": project-secret name}], ..]; the value of a secret '
    'is substituted only when a launch spec is built, never stored';
COMMENT ON COLUMN mcp_server.allow IS
    'tool names, without the mcp__<name>__ prefix, pre-approved for the server; empty leaves the '
    'existing policy to decide';
COMMENT ON COLUMN mcp_server.deny IS 'tool names removed from the agent';
COMMENT ON COLUMN mcp_server.updated_at IS 'the compare-and-set token of an edit';
COMMENT ON COLUMN toolbox_binding.kind IS
    'the entry kind: skill or mcp_server; an mcp_server row has no pin, no globs and is always or off';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118, MOD-33, MOD-163 and MOD-127: {v, template, budget, '
  'budget_source, reserve, target, estimator, estimated_before, estimated_after, sections[], '
  'skill_choices[], undigested[], excerpts, notes}, plus memory when a phase step''s run froze '
  'project memory (v 5) and mcp_choices[] when the step''s frozen toolbox bound an MCP server, v 6. '
  'memory is {shown[], total}: shown[] is each memory entry that reached the prompt after trimming, '
  'as {id, version} in render order, empty when the cap or the trim left none, and total is how many '
  'active entries the project had when the run started. mcp_choices[] is every server the frozen '
  'toolbox bound, ordered by server name bytes, each {server, name, level, activation, active, '
  'outcome} with outcome rendered, or {skipped: off | agent_unsupported | transport_unsupported | '
  'reserved_name | {secret_missing: <name of the project secret>}}; names only, never a value. '
  'undigested[] names every span rendered into the prompt but excluded from prompt_digest: '
  'box.hostname, or empty. skill_choices[] is every candidate skill, ordered by position then '
  'name, each {skill, name, version, level, activation, active, reason} with reason always, '
  'matched, no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched '
  'choice adds path, <repo>:<path>, the first matching file in repo then path byte order. A v 5 '
  'record, written before 0042, has no mcp_choices; a v 4 record, written before 0026, has no '
  'memory and its prompt carried none; a v 3 record, written before 0010, has no undigested and '
  'digested the hostname; a v 2 record, written before 0008, has no matched or no_match; a v 1 '
  'record, written before 0007, has no skill_choices. Canonical; the prompt payload sections[] '
  'array is its abridged projection. Written at stage 3 by set_step_prompt, before the session '
  'starts.';
