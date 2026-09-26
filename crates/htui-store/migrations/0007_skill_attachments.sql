-- 0007_skill_attachments.sql - MOD-9 milestone 2 (docs/ANA-22.md 7.1; plan D38, D42).
-- Forward-only (R-STO-5).
--
-- 1. ANA-22 7.1 verbatim. skill_version keeps its import provenance, and skill_binding becomes the
--    attachment at three levels: project_id NULL is global (every project), phase_id NULL is the
--    project level, both set is one phase. The existing UNIQUE NULLS NOT DISTINCT (skill_id,
--    project_id, phase_id) already allows exactly one global row per skill, and project_id keeps
--    ON DELETE CASCADE, which never fires for a NULL key. Existing rows read activation 'always',
--    globs '{}' and languages '{}': every binding keeps today's behaviour.
-- 2. run_step.trim_record's contract is restated for record v 2 (plan D42): skill_choices[].

-- --------------------------------------------------------------------------------------------
-- 1. ANA-22 7.1: skill attachments
-- --------------------------------------------------------------------------------------------

ALTER TABLE skill_version
    ADD COLUMN source JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE skill_binding
    ALTER COLUMN project_id DROP NOT NULL,
    ADD COLUMN activation TEXT   NOT NULL DEFAULT 'always'
        CHECK (activation IN ('always', 'glob', 'off')),
    ADD COLUMN globs      TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN languages  TEXT[] NOT NULL DEFAULT '{}',
    ADD CONSTRAINT skill_binding_phase_needs_project
        CHECK (phase_id IS NULL OR project_id IS NOT NULL),
    ADD CONSTRAINT skill_binding_glob_needs_globs
        CHECK (activation <> 'glob' OR cardinality(globs) > 0);

COMMENT ON COLUMN skill_version.source IS
    'import provenance and raw frontmatter; prefills an attachment, never read by the prompt builder';
COMMENT ON COLUMN skill_binding.project_id IS 'NULL = global attachment (every project)';
COMMENT ON COLUMN skill_binding.activation IS
    'always | glob | off; the most specific attachment of a skill wins (ANA-22)';
COMMENT ON COLUMN skill_binding.globs IS
    'effective globs: typed plus languages expanded at save; <repo>:<glob> only on project or phase rows';
COMMENT ON COLUMN skill_binding.languages IS 'languages as authored; display only';

-- --------------------------------------------------------------------------------------------
-- 2. MOD-9 D42: the trim record's v 2
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42: {v, template, budget, budget_source, reserve, target, '
  'estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, notes}, '
  'v 2. skill_choices[] is every candidate skill, ordered by position then name, each {skill, '
  'name, version, level, activation, active, reason} with reason always, off, no_path, '
  'missing_version or not_placed (ANA-22 6 item 8). A v 1 record, written before 0007, has no '
  'skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. '
  'Written at stage 3 by set_step_prompt, before the session starts.';
