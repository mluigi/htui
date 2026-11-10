-- 0028_divided_fanout.sql - MOD-139 milestone 1 (plan D1; ANA-33 §7.2, §7.4, §7.11): the records
-- of divided fan-out. Forward-only (R-STO-5). Depends on 0001_init.sql (run, run_step,
-- set_updated_at()) and on 0020_step_graph_v2.sql's plan_scope vocabulary.
--
-- run_step gains ANA-33 §7.4's three columns, each written once by create_step and never updated:
-- role ('step' on every row written before 0028 and on every compete candidate, so nothing is
-- backfilled), parent_step_id (a part's planner step) and node_key (a part's
-- '<divided>/<part_key>', a stage's 'pick' | 'prepare' | 'collect'). chk_run_step_part makes a part
-- name both. chk_run_step_stage reserves positions -3..-1 for htui's own stage rows (§7.11): a
-- negative position is a stage row's and only a stage row's, and a stage row names one of the
-- three stages. No writer before 0028 produced a negative position, so every existing row passes.
-- node_key IS NOT NULL is spelled out: a NULL key would make the IN list NULL, and a CHECK passes
-- on NULL (probed for the M1 blueprint). M1 writes only role = 'step'; MOD-139 M2 writes planner,
-- part and integrate rows, MOD-126 and MOD-132 the stage rows.
--
-- run_plan is one revision of a run's plan (§7.2), append-only in revisions: UNIQUE (run_id,
-- revision), inserted as 'proposed'; afterwards only status moves, and base_refs is set by the
-- admission. updated_at is an addition to §7.2's columns, for that status move and the mirror's
-- cursor, under 0001's set_updated_at() trigger. An admitted revision carries its base_refs.
--
-- Mirrored: cache_migrations/0010_divided_fanout.sql adds the three columns and run_plan.
-- cache_meta.schema_version moves to 28 through PgStore::schema_version(), so each box rebuilds its
-- mirror once on first start; a headless worker never migrates: migrate from a TUI first. MOD-97's
-- sandbox may also write 0028: whichever merges second renumbers at collect, with its count pins.

-- --------------------------------------------------------------------------------------------
-- 1. run_step: role, parent_step_id, node_key
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step
  ADD COLUMN role TEXT NOT NULL DEFAULT 'step'
    CONSTRAINT chk_run_step_role
    CHECK (role IN ('step','planner','part','integrate','subgraph','consult','stage')),
  ADD COLUMN parent_step_id UUID
    CONSTRAINT fk_run_step_parent_step REFERENCES run_step(id) ON DELETE CASCADE,
  ADD COLUMN node_key TEXT,
  ADD CONSTRAINT chk_run_step_part
    CHECK (role <> 'part' OR (parent_step_id IS NOT NULL AND node_key IS NOT NULL)),
  ADD CONSTRAINT chk_run_step_stage
    CHECK ((role = 'stage') = (position < 0)
           AND (role <> 'stage'
                OR (position BETWEEN -3 AND -1
                    AND node_key IS NOT NULL
                    AND node_key IN ('pick','prepare','collect'))));
-- ON DELETE CASCADE's lookup when a step is deleted, and M2's "the parts of this planner" read.
CREATE INDEX idx_run_step_parent_step ON run_step(parent_step_id)
  WHERE parent_step_id IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 2. run_plan
-- --------------------------------------------------------------------------------------------
CREATE TABLE run_plan (
  id              UUID PRIMARY KEY,                                 -- client-minted UUIDv7
  run_id          UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CONSTRAINT chk_run_plan_revision CHECK (revision >= 0),
  planner_step_id UUID NOT NULL
                  CONSTRAINT fk_run_plan_planner_step REFERENCES run_step(id) ON DELETE CASCADE,
  scope           TEXT NOT NULL
                  CONSTRAINT chk_run_plan_scope CHECK (scope IN ('parts','successors','graph')),
  status          TEXT NOT NULL DEFAULT 'proposed'
                  CONSTRAINT chk_run_plan_status
                  CHECK (status IN ('proposed','admitted','refused','abandoned')),
  base_position   INTEGER NOT NULL
                  CONSTRAINT chk_run_plan_base_position CHECK (base_position >= 0),
  base_refs       JSONB,
  body            JSONB NOT NULL,
  digest          TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT uq_run_plan_revision UNIQUE (run_id, revision),
  CONSTRAINT chk_run_plan_admitted_base_refs
    CHECK (status <> 'admitted' OR base_refs IS NOT NULL)
);
CREATE INDEX idx_run_plan_updated_at ON run_plan(updated_at);              -- the mirror's cursor
-- ON DELETE CASCADE's lookup when a run_step is deleted (as 0022's idx_memory_entry_superseded_by)
CREATE INDEX idx_run_plan_planner_step ON run_plan(planner_step_id);

-- --------------------------------------------------------------------------------------------
-- 3. updated_at trigger, 0001 §5.1's shape (BEFORE UPDATE only)
-- --------------------------------------------------------------------------------------------
CREATE TRIGGER trg_run_plan_updated_at BEFORE UPDATE ON run_plan
  FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- --------------------------------------------------------------------------------------------
-- 4. Comments (pinned verbatim by tests/migrations.rs's MOD139_* constants)
-- --------------------------------------------------------------------------------------------
COMMENT ON TABLE run_plan IS
  'MOD-139 D1 (ANA-33 7.2): one revision of a run''s plan, append-only in revisions. Inserted as '
  'proposed; admission sets status and base_refs and writes the part rows in one transaction '
  '(MOD-139 M2). Mirrored.';
COMMENT ON COLUMN run_step.role IS
  'MOD-139 D1 (ANA-33 7.4, 7.11): what the step is. step on every row written before 0028 and on '
  'every compete candidate; planner, part and integrate rows belong to a divided node; stage rows '
  'are htui''s own pick, prepare and collect. Written once.';
COMMENT ON COLUMN run_step.parent_step_id IS
  'MOD-139 D1 (ANA-33 7.4): the step this one was spawned under, a part''s planner step; NULL on '
  'every other row today. Written once.';
COMMENT ON COLUMN run_step.node_key IS
  'MOD-139 D1 (ANA-33 7.4, 7.11): a part''s <divided>/<part_key>, or a stage''s pick, prepare or '
  'collect; NULL on a node of the frozen snapshot, which position names. Written once.';
COMMENT ON COLUMN run_plan.revision IS
  'MOD-139 D1: 0 for default parts the engine admits itself, then one per planner submission; '
  'unique per run.';
COMMENT ON COLUMN run_plan.planner_step_id IS
  'MOD-139 D1: the step of the same run whose planner produced this revision; insert_run_plan '
  'refuses a step of another run.';
COMMENT ON COLUMN run_plan.status IS
  'MOD-139 D1: proposed when inserted; admitted, refused or abandoned once decided. Only an '
  'admitted revision''s parts run.';
COMMENT ON COLUMN run_plan.base_position IS
  'MOD-139 D1 (ANA-33 7.2): the run_step.position of this revision''s first planned node; the '
  'i-th takes base_position + i, after every frozen node and every earlier revision.';
COMMENT ON COLUMN run_plan.base_refs IS
  'MOD-139 D1 (ANA-33 7.4): the primary HEAD per repo at admission, which every part and the '
  'integrate step branch from. NULL until admitted; an admitted revision has it.';
COMMENT ON COLUMN run_plan.digest IS
  'MOD-139 D1 (ANA-33 7.2): sha256 over the canonical body; resume recomputes it to check an '
  'admitted plan.';
COMMENT ON COLUMN run_plan.updated_at IS
  'MOD-139 D1: an addition to ANA-33 7.2''s columns. Moves with status (set_updated_at) and is '
  'the mirror''s cursor.';
