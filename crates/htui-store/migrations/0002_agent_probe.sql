-- 0002_agent_probe.sql - MOD-2 milestone 5 (plan D43): the ANA-4 §4.6 probe column, and ANA-5 §9's
-- column contracts and app_setting defaults folded in - "MOD-2 authors 0002 and MOD-2 is this
-- document's consumer, so there is zero sequencing risk" (docs/ANA-5.md 9). The file name stops
-- being a complete description of its contents; this header is the discoverability half of that.
--
-- Forward-only (R-STO-5): 0001_init.sql is never edited. ANA-2's 0003_orchestration.sql depends on
-- this file (docs/ANA-2.md 9); its cache-mirror migration is 0003 too, because
-- cache_migrations/0002_agent_mirror.sql is MOD-2 milestone 4's.

-- --------------------------------------------------------------------------------------------
-- 1. ANA-4 §9 (docs/ANA-4.md:1267-1273), as amended by MOD-2 plan D43.
--
-- agent_box.probe holds the §4.6 snapshot: {transport, resolved {command, args, env}, tools
-- {name: version}, handshake {at, protocol_version, agent_name, agent_version, capabilities,
-- auth_methods}, status (ready | unauthenticated | missing | failed), stderr_tail, source
-- (probe | manual)}. Typed by htui_agent::probe::ProbeSnapshot; htui-core and htui-store hold
-- it as JSON. MOD-4 reads probe->>'status' (docs/ANA-2.md 7).
--
-- ANA-4 wrote `COMMENT ON COLUMN agent.name IS NULL` to "fix the stale inline comment in 0001".
-- 0001 carries no database comment, so that statement clears nothing; the stale text is a
-- source-file comment (0001_init.sql:96) the forward-only rule forbids editing. This is the
-- comment ANA-4 meant, for the reader of `\d+ agent`.
-- --------------------------------------------------------------------------------------------

ALTER TABLE agent_box ADD COLUMN probe JSONB;

COMMENT ON COLUMN agent.name IS
  'unique registry name, e.g. claude or agy. Rows are seeded by htui, not by 0001_init.sql: '
  'htui_core::model::agent::seed_rows (crates/htui-core/seeds/*.json) inserted by '
  'PgStore::seed_if_empty_as when the table is empty. The inline comment in 0001_init.sql '
  'predates that and is stale (ANA-4 9 as amended by MOD-2 plan D43).';

-- --------------------------------------------------------------------------------------------
-- 2. ANA-5 §9: the five column contracts, verbatim (docs/ANA-5.md:2153-2181).
--
-- ANA-5 (prompt assembly). No DDL: run_step.prompt_digest and run_step.trim_record already
-- exist in 0001_init.sql. These are the documented contracts and the app_setting defaults.
-- --------------------------------------------------------------------------------------------

COMMENT ON COLUMN prompt_template.body IS
  'ANA-5 4.1: {{name}} placeholders over a closed per-role set; {{{{ escapes a literal {{; no '
  'conditionals and no loops, because a section whose data is absent renders empty. Role is '
  'derived from name: judge and handoff are reserved, everything else is a phase template.';

COMMENT ON COLUMN prompt_template.name IS
  'phase name, or one of the reserved names judge and handoff (ANA-5 4.6); defaults are copied '
  'into each new project by the ANA-9 5.10 seed as amended by ANA-5';

COMMENT ON COLUMN run_step.prompt_digest IS
  'ANA-5 4.7: sha256, lowercase hex, over the canonical assembled prompt TEXT as sent - LF '
  'normalised, BOM stripped, one trailing LF, scrubbed before hashing. Not over the payload and '
  'not over sections[]. An audit field, never a replay key (ANA-2 4.9).';

COMMENT ON COLUMN run_step.trim_record IS
  'ANA-5 5.1: {v, template, budget, budget_source, reserve, target, estimator, estimated_before, '
  'estimated_after, sections[], excerpts, notes}. Canonical; the prompt payload sections[] array '
  'is its abridged projection. Written at stage 3 by set_step_prompt, before the session starts.';

COMMENT ON COLUMN step_graph_phase.token_budget IS
  'ANA-5 4.4: phase, then project.settings.token_budget, then app_setting.token_budget; the '
  'assembler targets budget * (1 - app_setting.prompt_reserve_fraction)';

-- --------------------------------------------------------------------------------------------
-- 3. ANA-5 §9: the ten 5.3 defaults, verbatim (docs/ANA-5.md:2183-2189). Idempotent, so a re-run
--    and the MOD-6 seed agree.
-- --------------------------------------------------------------------------------------------

INSERT INTO app_setting (key, value) VALUES
  ('token_budget',                 '120000'::jsonb),
  ('prompt_reserve_fraction',      '0.10'::jsonb),
  ('prompt_upstream_hops',         '2'::jsonb),
  ('max_skill_tokens',             '20000'::jsonb),
  ('excerpt_max_files',            '12'::jsonb),
  ('excerpt_file_line_cap',        '400'::jsonb),
  ('excerpt_head_lines',           '200'::jsonb),
  ('excerpt_max_file_bytes',       '524288'::jsonb),
  ('excerpt_max_scan_files',       '20000'::jsonb),
  ('excerpt_provider_deadline_ms', '1500'::jsonb)
ON CONFLICT (key) DO NOTHING;
