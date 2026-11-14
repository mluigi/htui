-- 0039_integration.sql - MOD-133 milestone 1 (plan D2, D5; ANA-34 §7). Forward-only (R-STO-5).
-- Depends on 0001_init.sql (run, repo, project, box, set_updated_at).
--
-- How a finished run's work reaches its repository's remote. repo.integrate is the repo's mode;
-- NULL is the project's default (the `integrate` key of project.settings, read by
-- IntegrateMode::project_default), which is `local` when absent: today's behaviour, where the work
-- stays on the box's local target branch. Every existing repo reads NULL, so nothing changes.
--
-- integration holds one record per finished run and non-local repo the run changed, written by
-- finish_run(done) in the run's own transaction (plan D4), keyed (run_id, repo_id). mode, remote
-- and target_branch are frozen from the run's graph_snapshot.integrate. A record is `pending`
-- until the executing box lands it (`landed`, landed_hash) or refuses it (`refused`, refusal:
-- chk_integration_refusal ties the two). The pr_* states and columns are MOD-134's, declared now so
-- the CHECK never moves; mode never holds `local`, which writes no record. A run whose records are
-- not all landed | pr_merged does not release the items it blocks (plan D7).
--
-- The lease columns are the run lease's (0003): lease_owner is a per-process token, NULL when
-- free; lease_expires_at is stamped and judged by clock_timestamp() plus a TTL, never a box clock.
-- A settle clears all three.
--
-- A record goes with its run and with its project (ON DELETE CASCADE on both). repo_id has no
-- cascade, as run_step_commit's has none; a project delete still takes its own records, since the
-- project_id cascade reaches them in the same statement (migration_0039_creates_integration pins
-- it). A deleted box frees its leases (SET NULL). trg_integration_updated_at moves
-- updated_at; idx_integration_updated_at backs the cache cursor and idx_integration_pending the
-- land pass's take.
--
-- Mirrored without the lease columns (cache_migrations/0013_integration.sql), for the cache's
-- readiness read; repo.integrate is not mirrored (no cache reader builds a Repo).
-- cache_meta.schema_version moves to 39 through PgStore::schema_version(), so each box rebuilds
-- its mirror once on first start; a headless worker never migrates: migrate from a TUI first.

ALTER TABLE repo ADD COLUMN integrate TEXT
    CONSTRAINT chk_repo_integrate CHECK (integrate IN ('local', 'push', 'pull_request'));

CREATE TABLE integration (
    run_id           UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    repo_id          UUID NOT NULL REFERENCES repo(id),
    project_id       UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    mode             TEXT NOT NULL
        CONSTRAINT chk_integration_mode CHECK (mode IN ('push', 'pull_request')),
    remote           TEXT NOT NULL,
    target_branch    TEXT NOT NULL,
    state            TEXT NOT NULL DEFAULT 'pending'
        CONSTRAINT chk_integration_state CHECK (state IN ('pending', 'landed', 'pr_open',
                                                          'pr_merged', 'pr_closed', 'refused')),
    refusal          TEXT,
    landed_hash      TEXT,
    pr_ref           TEXT,
    pr_url           TEXT,
    lease_box_id     UUID REFERENCES box(id) ON DELETE SET NULL,
    lease_owner      UUID,
    lease_expires_at TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, repo_id),
    CONSTRAINT chk_integration_refusal CHECK ((state = 'refused') = (refusal IS NOT NULL))
);

CREATE TRIGGER trg_integration_updated_at BEFORE UPDATE ON integration
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX idx_integration_updated_at ON integration (updated_at);
CREATE INDEX idx_integration_pending ON integration (run_id) WHERE state = 'pending';
