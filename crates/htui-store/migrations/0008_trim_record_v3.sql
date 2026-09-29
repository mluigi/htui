-- 0008_trim_record_v3.sql - MOD-9 milestone 5 (docs/ANA-22.md 6 item 8; plan D109, D118, OQ-27).
-- Forward-only (R-STO-5).
--
-- run_step.trim_record's contract is restated for record v 3: skill_choices[].reason gains
-- matched and no_match, and a matched choice carries path. Comment only: no table, column,
-- constraint or index moves, and the SQLite mirror's schema is untouched. schema_version still
-- becomes 8, so each box rebuilds its mirror once on first start (plan R-56).

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118: {v, template, budget, budget_source, reserve, '
  'target, estimator, estimated_before, estimated_after, sections[], skill_choices[], excerpts, '
  'notes}, v 3. skill_choices[] is every candidate skill, ordered by position then name, each '
  '{skill, name, version, level, activation, active, reason} with reason always, matched, '
  'no_match, off, no_path, missing_version or not_placed (ANA-22 6 item 8); a matched choice '
  'adds path, <repo>:<path>, the first matching file in repo then path byte order. A v 2 '
  'record, written before 0008, has no matched or no_match; a v 1 record, written before 0007, '
  'has no skill_choices. Canonical; the prompt payload sections[] array is its abridged '
  'projection. Written at stage 3 by set_step_prompt, before the session starts.';
