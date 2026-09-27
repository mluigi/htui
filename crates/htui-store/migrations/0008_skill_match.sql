-- 0008_skill_match.sql - MOD-9 milestone 3 (docs/ANA-22.md 6 item 8; plan D75, D87, D89).
-- Forward-only (R-STO-5).
--
-- No column and no constraint changes. 0007 wrote run_step.trim_record's comment with the reason
-- vocabulary of milestone 2, which had no `matched` and no `no_match`; a comment cannot be edited
-- in place, because schema_state refuses a database that applied an older text of an existing
-- migration (pg/mod.rs:584-627, error.rs checksum_drift), so the contract is re-issued here. The
-- commented-column total does not move: this writes one comment over a column 0007 already
-- commented, so the thirty-four of migrations.rs is still thirty-four.

-- --------------------------------------------------------------------------------------------
-- 1. run_step.trim_record: MOD-9 D75's reason vocabulary
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42: {v, template, budget, budget_source, reserve, target, '
  'estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, notes}, '
  'v 2. skill_choices[] is every candidate skill, ordered by position then name, each {skill, '
  'name, version, level, activation, active, reason, matched} with reason always, matched, '
  'no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); matched is the '
  'repo:path the glob fired on, or null. A v 1 record, written before 0007, has no '
  'skill_choices. Canonical; the prompt payload sections[] array is its abridged projection. '
  'Written at stage 3 by set_step_prompt, before the session starts.';
