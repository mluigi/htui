-- 0014_run_step_opening.sql - MOD-37 milestone 5 (R-48, ANA-27 §5.1 T5).
-- Forward-only (R-STO-5).
--
-- run_step.opening records how a promoted step's chat opened: 'resumed' (the step's own agent
-- session, restored through session/resume, session/load or the CLI's --resume), 'handoff' (a fresh
-- session opened with the handoff prompt) or 'resume_failed' (a requested resume failed, and the chat
-- fell back to the handoff prompt in the same bind). NULL on every step never bound to a promoted chat;
-- no older row knows how its chat opened, so nothing is backfilled. A second promotion overwrites it.
-- The mirror gains the column in cache_migrations/0005_run_step_opening.sql, and schema_version becomes
-- 14, so each box rebuilds its mirror once on first start. A headless worker never migrates: migrate
-- from a TUI first.

ALTER TABLE run_step ADD COLUMN opening TEXT
  CHECK (opening IN ('resumed','handoff','resume_failed'));
