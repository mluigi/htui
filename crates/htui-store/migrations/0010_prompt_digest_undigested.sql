-- 0010_prompt_digest_undigested.sql - MOD-33 (plan D263, D270, D271, D273).
-- Forward-only (R-STO-5).
--
-- run_step.prompt_digest is restated: the digest is over the digest text, the sent text with each
-- undigested span's value replaced by its fixed stand-in. run_step.trim_record is restated for
-- record v 4, which gains undigested[]. Comment only: no table, column, constraint or index
-- moves, and the SQLite mirror's schema is untouched. schema_version still becomes 10, so each
-- box rebuilds its mirror once on first start (0008's R-56).

COMMENT ON COLUMN run_step.prompt_digest IS
  'ANA-5 4.7 as amended by MOD-33: sha256, lowercase hex, over the canonical assembled prompt '
  'TEXT as sent with each undigested span replaced by its fixed stand-in - today only the box '
  'section''s hostname value, as [hostname] - LF normalised, BOM stripped, one trailing LF, '
  'scrubbed before hashing. trim_record.undigested lists the spans. Not over the payload and not '
  'over sections[]. An audit field, never a replay key (ANA-2 4.9).';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1 as amended by MOD-9 D42 and D118 and MOD-33: {v, template, budget, budget_source, '
  'reserve, target, estimator, estimated_before, estimated_after, sections[], skill_choices[], '
  'undigested[], excerpts, notes}, v 4. undigested[] names every span rendered into the prompt '
  'but excluded from prompt_digest: box.hostname, or empty. skill_choices[] is every candidate '
  'skill, ordered by position then name, each {skill, name, version, level, activation, active, '
  'reason} with reason always, matched, no_match, off, no_path, missing_version or not_placed '
  '(ANA-22 6 item 8); a matched choice adds path, <repo>:<path>, the first matching file in repo '
  'then path byte order. A v 3 record, written before 0010, has no undigested and digested the '
  'hostname; a v 2 record, written before 0008, has no matched or no_match; a v 1 record, written '
  'before 0007, has no skill_choices. Canonical; the prompt payload sections[] array is its '
  'abridged projection. Written at stage 3 by set_step_prompt, before the session starts.';
