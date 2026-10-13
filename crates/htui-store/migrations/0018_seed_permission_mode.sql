-- 0018_seed_permission_mode.sql - MOD-96 (plan D6; Q3 = b).
-- Forward-only (R-STO-5).
--
-- The claude (ACP) seed now pins its session mode to 'default' with session/set_mode.
-- Seeds are a name-keyed insert-once (seed_if_empty_as: ON CONFLICT (name) DO NOTHING), so a box
-- seeded earlier keeps the old values. This pins a seeded claude row only while it has no mode;
-- a row whose acp.permission_mode is already set (an explicit JSON null included), a missing acp
-- object or another name is left alone, and a second run matches nothing. Runs before the seed
-- pass (apply_migrations: migrate, then bootstrap), so a fresh database is a no-op here and gets
-- the new seeds.
--
-- The claude-cli row is not touched. Its acceptEdits allows an edit only inside the session's
-- directories and after the CLI's safety checks; an unscoped htui edit rule would widen approval
-- (MOD-96 review R1).
--
-- updated_at moves through trg_agent_updated_at (0001_init.sql); it is never set here. No cache
-- migration: the mirror's agent table is replaced whole on every refresh pass
-- (cache_migrations/0002_agent_mirror.sql, cache/refresh.rs replace_agent).

UPDATE agent
   SET settings = jsonb_set(settings, '{acp,permission_mode}', '"default"')
 WHERE name = 'claude'
   AND transport = 'acp'
   AND jsonb_typeof(settings -> 'acp') = 'object'
   AND settings #> '{acp,permission_mode}' IS NULL;
