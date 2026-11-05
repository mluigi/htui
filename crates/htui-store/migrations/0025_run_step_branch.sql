-- 0025_run_step_branch.sql - MOD-138 milestone 2 (plan D5, Q-A). Forward-only (R-STO-5).
--
-- run_step.branch records, once, the readiness walker's decision on the step's `if` edges:
-- {"<target node key>": true | false} for each `if` edge leaving the step's node. record_branch
-- writes it only while the step is done and the column is NULL, so the first decision wins and
-- every reader, offline ones included, reads the same answer. NULL on every step whose node has
-- no `if` edge and on every row written before MOD-138 M2; nothing is backfilled. No COMMENT, as
-- 0014. The mirror gains the column in cache_migrations/0008_run_step_branch.sql; schema_version
-- becomes 25, so each box rebuilds its mirror once on first start. A headless worker never
-- migrates: migrate from a TUI first.
--
-- The CHECK admits NULL or an object whose every member is a JSON boolean (jsonb_path_exists in a
-- CHECK was probed on PostgreSQL 16.15 for T1: accepted).

ALTER TABLE run_step ADD COLUMN branch JSONB
  CONSTRAINT chk_run_step_branch
  CHECK (branch IS NULL
         OR (jsonb_typeof(branch) = 'object'
             AND NOT jsonb_path_exists(branch, '$.* ? (@.type() != "boolean")')));
