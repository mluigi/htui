-- 0004_max_agents_per_run_default.sql - MOD-4 milestone 4 (maintainer decision): the default
-- app_setting.max_agents_per_run goes from 6 to 8. Forward-only (R-STO-5): 0003_orchestration.sql
-- seeded the 6 (docs/ANA-2.md 5.4) and is never edited; this file moves it.
--
-- Why: plan D63 counts sum(fan_out) plus one per judged phase against the cap (ANA-2 :870-875
-- read literally). The seeded `feature` graph with `implement` at fan_out 3 and a judge plans
-- prd 1 + plan 1 + implement 3 + judge 1 + review 1 = 7 agents, so under 6 it is refused at
-- StartRun. 8 lets that ordinary judged fan-out run by default.
--
-- Only the exact seeded value moves. A row that is not '6' was chosen by somebody, and a
-- migration cannot tell a deliberate value from a default, so it is left alone. A missing row
-- stays missing: htui-orch's built-in fallback (graph.rs DEFAULT_MAX_AGENTS_PER_RUN) is 8 too.

UPDATE app_setting SET value = '8'::jsonb
 WHERE key = 'max_agents_per_run' AND value = '6'::jsonb;
