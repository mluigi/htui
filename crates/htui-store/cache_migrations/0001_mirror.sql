-- ------------------------------------------------------------------------------------------------
-- The per-box read-only mirror of `docs/ANA-9.md` 4.4 (MOD-6 blueprint C.11).
--
-- The fifteen mirrored tables, same table and column names as Postgres, plus `cache_meta` and
-- `cache_cursor`. Type mapping, 4.4 verbatim:
--
--     UUID        -> TEXT      (hyphenated; decoded through `cache::read::uuid_col`, H.9)
--     TIMESTAMPTZ -> INTEGER   (microseconds since the epoch; `ts_col` / `ts_bind`, H.10)
--     TEXT[]      -> TEXT      (a JSON array)
--     JSONB       -> TEXT
--     BOOLEAN     -> INTEGER   (0/1)
--     INTEGER, SMALLINT, TEXT  unchanged
--
-- Deliberate differences from `migrations/0001_init.sql`, each one required:
--
--   * no `deleted_at` on `item_link`: a tombstone in Postgres is a *deletion* here (4.4, "the
--     mirror drops the row"), so the refresher deletes and the reads never filter;
--   * no CHECK constraints and no foreign keys: one writer feeds this file from a database that
--     already enforced them, and a partially mirrored parent must not refuse a child row;
--   * `item.key` is a plain column, not generated: it is copied from the server;
--   * the indexes are the ones `cache/read.rs` needs, prefixed `idx_cache_` so a grep can never
--     confuse one with a Postgres index.
--
-- Not mirrored, and deliberately without a table: revisions, skills, templates, graphs, phases,
-- agents, counters, settings, capability tags, command queue (4.4).
-- ------------------------------------------------------------------------------------------------

CREATE TABLE cache_meta   (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE cache_cursor (project_id TEXT NOT NULL, table_name TEXT NOT NULL,
                           high_water INTEGER NOT NULL,
                           PRIMARY KEY (project_id, table_name));

CREATE TABLE app_user (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, email TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE box (
    id TEXT PRIMARY KEY, user_id TEXT NOT NULL, hostname TEXT NOT NULL,
    os_family TEXT NOT NULL, os_version TEXT NOT NULL, arch TEXT NOT NULL,
    cpu TEXT NOT NULL DEFAULT '', ram_mb INTEGER, gpu_present INTEGER NOT NULL DEFAULT 0,
    gpu_vendor TEXT, htui_version TEXT NOT NULL,
    probed_tags TEXT NOT NULL DEFAULT '[]', declared_tags TEXT NOT NULL DEFAULT '[]',
    quirks TEXT NOT NULL DEFAULT '', settings TEXT NOT NULL DEFAULT '{}',
    registered_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
    last_probed_at INTEGER, updated_at INTEGER NOT NULL);

CREATE TABLE workspace (
    id TEXT PRIMARY KEY, slug TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE workspace_project (
    workspace_id TEXT NOT NULL, project_id TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, project_id));
CREATE INDEX idx_cache_workspace_project_project ON workspace_project(project_id);

CREATE TABLE project (
    id TEXT PRIMARY KEY, slug TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', secret_provider TEXT, secret_scope TEXT,
    settings TEXT NOT NULL DEFAULT '{}', created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE repo (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, remote_url TEXT,
    default_branch TEXT NOT NULL DEFAULT 'main', is_primary INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_repo_project ON repo(project_id);

CREATE TABLE item_kind (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, prefix TEXT NOT NULL, name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '', default_graph_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_item_kind_project ON item_kind(project_id);

CREATE TABLE item (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, kind_id TEXT NOT NULL,
    key_prefix TEXT NOT NULL, key_number INTEGER NOT NULL, key TEXT NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'open',
    priority INTEGER NOT NULL DEFAULT 0,
    required_tags TEXT NOT NULL DEFAULT '[]', touched_paths TEXT NOT NULL DEFAULT '[]',
    step_graph_id TEXT, version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, closed_at INTEGER);
CREATE INDEX idx_cache_item_project_status ON item(project_id, status);
CREATE INDEX idx_cache_item_order          ON item(project_id, key_prefix, key_number);

CREATE TABLE item_link (
    from_item_id TEXT NOT NULL, to_item_id TEXT NOT NULL, kind TEXT NOT NULL,
    proposed_by_step_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY (from_item_id, to_item_id, kind));
CREATE INDEX idx_cache_item_link_to ON item_link(to_item_id);

CREATE TABLE item_note (
    id TEXT PRIMARY KEY, item_id TEXT NOT NULL, body TEXT NOT NULL, created_by TEXT NOT NULL,
    box_id TEXT, via_step_id TEXT, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_item_note_item ON item_note(item_id, created_at);

CREATE TABLE document (
    id TEXT PRIMARY KEY, item_id TEXT NOT NULL, kind TEXT NOT NULL, version INTEGER NOT NULL,
    title TEXT NOT NULL, body TEXT NOT NULL, produced_by_step_id TEXT,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX idx_cache_document_item ON document(item_id, kind, version);

CREATE TABLE run (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, item_id TEXT, kind TEXT NOT NULL,
    mode TEXT NOT NULL, status TEXT NOT NULL, target_box_id TEXT NOT NULL,
    executing_box_id TEXT, graph_snapshot TEXT, started_by TEXT NOT NULL,
    queued_at INTEGER NOT NULL, started_at INTEGER, finished_at INTEGER, failure TEXT,
    updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_item    ON run(item_id, queued_at DESC);
CREATE INDEX idx_cache_run_project ON run(project_id, status);

CREATE TABLE run_step (
    id TEXT PRIMARY KEY, run_id TEXT NOT NULL, position INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1, fanout_index INTEGER NOT NULL DEFAULT 0,
    phase_name TEXT NOT NULL, agent_id TEXT, model TEXT, status TEXT NOT NULL,
    gate_outcome TEXT, gate_note TEXT, selected INTEGER, exit_code INTEGER,
    prompt_digest TEXT, trim_record TEXT, usage TEXT, isolation_path TEXT,
    started_at INTEGER, finished_at INTEGER, updated_at INTEGER NOT NULL);
CREATE INDEX idx_cache_run_step_run ON run_step(run_id, position, attempt, fanout_index);

CREATE TABLE run_step_commit (
    run_step_id TEXT NOT NULL, repo_id TEXT NOT NULL, before_hash TEXT NOT NULL,
    after_hash TEXT, PRIMARY KEY (run_step_id, repo_id));

CREATE TABLE session_event (
    run_step_id TEXT NOT NULL, seq INTEGER NOT NULL, turn INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL, role TEXT NOT NULL, tool_call_id TEXT, payload TEXT NOT NULL,
    raw TEXT, at INTEGER NOT NULL, PRIMARY KEY (run_step_id, seq));
