-- 0018_seed_permission_mode.sql - MOD-96 (plan D5, D6; Q2, Q3 = b).
-- Forward-only (R-STO-5).
--
-- The seeds now state the permission mode: claude-cli runs in 'default' and allows edits through
-- one agent rule instead of acceptEdits, and claude (ACP) pins 'default' with session/set_mode.
-- Seeds are a name-keyed insert-once (seed_if_empty_as: ON CONFLICT (name) DO NOTHING), so a box
-- seeded earlier keeps the old values. This rewrites a seeded row only while it still holds them;
-- an operator's edit, a missing key or another name is left alone, and a second run matches
-- nothing. Runs before the seed pass (apply_migrations: migrate, then bootstrap), so a fresh
-- database is a no-op here and gets the new seeds.
--
-- updated_at moves through trg_agent_updated_at (0001_init.sql); it is never set here. No cache
-- migration: the mirror's agent table is replaced whole on every refresh pass
-- (cache_migrations/0002_agent_mirror.sql, cache/refresh.rs replace_agent).

UPDATE agent
   SET settings = jsonb_set(
           jsonb_set(settings, '{cli,permission_mode}', '"default"'),
           '{permission,rules}',
           '[{"match": {"tool_kind": "edit"}, "answer": "allow_once", "reason": "edits are allowed and recorded by htui (MOD-96); a persona rule can refuse them"}]')
 WHERE name = 'claude-cli'
   AND transport = 'cli'
   AND settings #>> '{cli,permission_mode}' = 'acceptEdits'
   AND settings #> '{permission,rules}' = '[]'::jsonb;

UPDATE agent
   SET settings = jsonb_set(settings, '{acp,permission_mode}', '"default"')
 WHERE name = 'claude'
   AND transport = 'acp'
   AND jsonb_typeof(settings -> 'acp') = 'object'
   AND settings #> '{acp,permission_mode}' IS NULL;
