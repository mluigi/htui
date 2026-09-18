-- 0003_orchestration.sql - MOD-4 milestone 1: the ANA-2 (orchestration) amendments to the ANA-9
-- schema, docs/ANA-2.md 9 verbatim. Forward-only (R-STO-5): 0001_init.sql and
-- 0002_agent_probe.sql are never edited. Depends on 0002_agent_probe.sql (ANA-4 9, MOD-2).
-- The cache-mirror companion is cache_migrations/0003_orchestration.sql (plan D7).

-- --------------------------------------------------------------------------------------------
-- 1. step_graph: item override graphs are hidden from the project graph list (ANA-2 §4.1)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph ADD COLUMN is_override BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_step_graph_listed ON step_graph(project_id) WHERE NOT is_override;
COMMENT ON COLUMN step_graph.is_override IS
  'true = an <item.key>-override clone (R-ORCH-1, ANA-2 §4.1); hidden from the R-TUI-8 graph list';

-- --------------------------------------------------------------------------------------------
-- 2. step_graph_phase: the R-ORCH-7 judge and the step deadline (ANA-2 §4.5, §4.2)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph_phase ADD COLUMN judge_agent_id   UUID REFERENCES agent(id);
ALTER TABLE step_graph_phase ADD COLUMN judge_model      TEXT;
ALTER TABLE step_graph_phase ADD COLUMN deadline_seconds INTEGER
  CHECK (deadline_seconds IS NULL OR deadline_seconds > 0);
ALTER TABLE step_graph_phase ADD CONSTRAINT ck_phase_judge_model
  CHECK (judge_model IS NULL OR judge_agent_id IS NOT NULL);

COMMENT ON COLUMN step_graph_phase.judge_agent_id IS
  'R-ORCH-7 judge; NULL = human selection is required whenever fan_out > 1 (ANA-2 §4.5)';
COMMENT ON COLUMN step_graph_phase.judge_model IS
  'model for the judge; overrides agent.default_model (ANA-2 §7)';
COMMENT ON COLUMN step_graph_phase.deadline_seconds IS
  'wall clock for one step attempt; NULL = project.settings.step_deadline_seconds (ANA-2 §4.1)';
COMMENT ON COLUMN step_graph_phase.verify_command IS
  'ANA-2 §4.2: runs after the session and before the gate, in the primary repo tree, through '
  'command_run when the phase advertises it; outcome lands in run_step.verify_outcome';
COMMENT ON COLUMN step_graph_phase.input_kinds IS
  'ANA-2 §4.2: each kind resolves to the latest document version on this item whose producing '
  'step is not a fan-out loser, preferring this run''s own output; a missing kind fails the step';

-- --------------------------------------------------------------------------------------------
-- 3. run: repo scope (R-ORCH-9), the recovery lease, the R-ORCH-11 snapshot guarantee
-- --------------------------------------------------------------------------------------------
ALTER TABLE run ADD COLUMN repo_scope       UUID[] NOT NULL DEFAULT '{}';
ALTER TABLE run ADD COLUMN lease_box_id     UUID REFERENCES box(id);
ALTER TABLE run ADD COLUMN lease_owner      UUID;
ALTER TABLE run ADD COLUMN lease_expires_at TIMESTAMPTZ;

-- NOT VALID: demo and fixture rows predate ANA-2 and carry a NULL snapshot on a graph run.
-- New rows are checked; the backfill is a separate, optional VALIDATE CONSTRAINT.
ALTER TABLE run ADD CONSTRAINT ck_run_graph_snapshot
  CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID;

CREATE INDEX idx_run_lease ON run(executing_box_id, lease_expires_at)
  WHERE status IN ('queued','running');
CREATE INDEX idx_run_repo_scope ON run USING GIN (repo_scope);

COMMENT ON COLUMN run.repo_scope IS
  'repos this run may touch, resolved at queue time from item.touched_paths (ANA-2 §4.7)';
COMMENT ON COLUMN run.lease_owner IS
  'per-process id of the orchestrator holding this run; a zero-row lease refresh means abandon '
  '(ANA-2 §4.9)';
COMMENT ON COLUMN run.graph_snapshot IS
  'ANA-2 §5.1: {v, graph, topology, mode, phases[], settings}; carries both gate and '
  'gate_effective so the R-ORCH-6 downgrade is auditable';

-- --------------------------------------------------------------------------------------------
-- 4. run_step: verification outcome (ANA-2 §4.2) and chat promotion (ANA-2 §4.8)
--    No CHECK on fanout_index, ever: -1 is the judge step (ANA-2 risk 12).
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN verify_outcome   TEXT
  CHECK (verify_outcome IN ('pass','fail','unavailable'));
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at      TIMESTAMPTZ;

COMMENT ON COLUMN run_step.verify_outcome IS
  'ANA-2 §4.2: pass | fail | unavailable; unavailable never fails a step';
COMMENT ON COLUMN run_step.exit_code IS
  'the agent process exit code (R-ORCH-11); verification has its own verify_exit_code';
COMMENT ON COLUMN run_step.fanout_index IS
  '0..fan_out-1; -1 = the R-ORCH-7 judge step for this position and attempt (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.selected IS
  'fan-out winner; NULL when fan_out = 1; false on a loser, which is also superseded (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.attempt IS
  '1-based; retry_limit is the number of additional attempts, so attempt <= retry_limit + 1 '
  '(ANA-2 §4.2)';
COMMENT ON COLUMN run_step.isolation_path IS
  'the primary repo tree; every repo in scope has a run_step_tree row (ANA-2 §4.6)';
COMMENT ON COLUMN run_step.promoted_at IS
  'set when the step was promoted to an interactive chat (R-ORCH-5, ANA-2 §4.8)';

-- --------------------------------------------------------------------------------------------
-- 5. run_step_tree: one row per (step, repo), because a project has one or more repos (R-ENT-3)
--    No updated_at and no trigger: like run_step_commit it rides its parent step (ANA-9 §6.2).
-- --------------------------------------------------------------------------------------------
CREATE TABLE run_step_tree (
    run_step_id UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    repo_id     UUID NOT NULL REFERENCES repo(id),
    mode        TEXT NOT NULL CHECK (mode IN ('worktree','copy','shared_serialized','local')),
    path        TEXT NOT NULL,                 -- absolute, on the executing box, outside every repo
    base_ref    TEXT NOT NULL,                 -- the commit the tree was created at (ANA-2 §4.6)
    dirty       BOOLEAN NOT NULL DEFAULT false,-- the tree had uncommitted work at step start
    PRIMARY KEY (run_step_id, repo_id)
);
COMMENT ON TABLE run_step_tree IS
  'R-ORCH-8 isolation, per repo; ANA-2 §4.6. A dirty local or shared_serialized tree is never '
  'reset by the recovery sweep (ANA-2 §4.9).';

-- --------------------------------------------------------------------------------------------
-- 6. item: touched_paths become repo-qualified (ANA-2 §4.7). Existing bare globs keep meaning
--    the primary repo, so no data migration is required.
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN item.touched_paths IS
  'R-ORCH-9 declared overlap set: "<repo_name>:<glob>" entries, or a bare glob meaning the '
  'primary repo. Empty means unknown, which overlaps the whole primary repo (ANA-2 §4.7).';

-- --------------------------------------------------------------------------------------------
-- 7. settings contracts (ANA-2 §4.7, §5.2)
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN box.settings IS
  'ANA-2 §4.7 BoxSettings: max_concurrent_items (R-ORCH-9), command_limits {class: n} (R-MCP-3). '
  'Every field optional; defaults come from app_setting.';
COMMENT ON COLUMN project.settings IS
  'ANA-2 §4.7 ProjectSettings: token_budget, retention_days, cached_transcript_steps, '
  'keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch, '
  'step_deadline_seconds, default_agent_id, judge_agent_id, copy_exclude. Every field optional.';

-- --------------------------------------------------------------------------------------------
-- 8. app_setting defaults (ANA-2 §5.4). Idempotent, so a re-run and the MOD-6 seed agree.
-- --------------------------------------------------------------------------------------------
INSERT INTO app_setting (key, value) VALUES
  ('max_concurrent_items',  '2'::jsonb),
  ('command_limits',        '{"build":1,"test":4,"verify":1}'::jsonb),
  ('default_isolation',     '"worktree"'::jsonb),
  ('step_deadline_seconds', '7200'::jsonb),
  ('max_fan_out',           '4'::jsonb),
  ('max_agents_per_run',    '6'::jsonb),
  ('copy_max_total_bytes',  '21474836480'::jsonb),
  ('lease_ttl_seconds',     '120'::jsonb),
  ('lease_refresh_seconds', '60'::jsonb),
  ('per_token_cap_run',     'null'::jsonb),
  ('per_token_cap_batch',   'null'::jsonb),
  ('scheduler_window',      'null'::jsonb)
ON CONFLICT (key) DO NOTHING;
