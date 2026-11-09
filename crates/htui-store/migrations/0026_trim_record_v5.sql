-- 0026_trim_record_v5.sql - MOD-163 milestone 2 (plan D17).
-- Forward-only (R-STO-5).
--
-- run_step.trim_record is restated for record v 5, which gains memory {shown[], total} when the
-- step's prompt carried project memory. Comment only: no table, column, constraint or index
-- moves, and the SQLite mirror's schema is untouched. schema_version still becomes 26, so each
-- box rebuilds its mirror once on first start (0008's R-56). MOD-147's sandbox also writes a
-- 0026: whichever merges second renumbers at collect, with the count pins.

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118, MOD-33 and MOD-163: {v, template, budget, '
  'budget_source, reserve, target, estimator, estimated_before, estimated_after, sections[], '
  'skill_choices[], undigested[], excerpts, notes}, plus memory when the step''s prompt carried '
  'project memory, v 5. memory is {shown[], total}: shown[] is each memory entry that reached '
  'the prompt after trimming, as {id, version} in render order, and total is how many active '
  'entries the project had when the run started. undigested[] names every span rendered into '
  'the prompt but excluded from prompt_digest: box.hostname, or empty. skill_choices[] is every '
  'candidate skill, ordered by position then name, each {skill, name, version, level, '
  'activation, active, reason} with reason always, matched, no_match, off, no_path, '
  'missing_version or not_placed (ANA-22 6 item 8); a matched choice adds path, <repo>:<path>, '
  'the first matching file in repo then path byte order. A v 4 record, written before 0026, has '
  'no memory and its prompt carried none; a v 3 record, written before 0010, has no undigested '
  'and digested the hostname; a v 2 record, written before 0008, has no matched or no_match; a '
  'v 1 record, written before 0007, has no skill_choices. Canonical; the prompt payload '
  'sections[] array is its abridged projection. Written at stage 3 by set_step_prompt, before '
  'the session starts.';
