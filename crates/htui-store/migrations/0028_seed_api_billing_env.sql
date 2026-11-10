-- 0028_seed_api_billing_env.sql - MOD-97 (plan D3, D4).
-- Forward-only (R-STO-5).
--
-- The claude (ACP) and claude-cli (CLI) seeds now name the variables that move Claude Code onto
-- API billing, settings.api_billing_env; a subscription-billed session strips them from the
-- child's environment. Seeds are a name-keyed insert-once (seed_if_empty_as: ON CONFLICT (name)
-- DO NOTHING), so a box seeded earlier keeps the hazard. This adds the list to a seeded claude or
-- claude-cli row only while its settings have no api_billing_env key; an explicit value (null and
-- [] mean none), a settings value that is not an object, another name or the other transport is
-- left alone, and a second run matches nothing. Runs before the seed pass
-- (apply_migrations: migrate, then bootstrap), so a fresh database is a no-op here and gets the
-- new seeds.
--
-- agy is not touched: its GEMINI_API_KEY is a login tier (discovery.credential.env), not a
-- billing switch.
--
-- updated_at moves through trg_agent_updated_at (0001_init.sql); it is never set here. No cache
-- migration: the mirror's agent table is replaced whole on every refresh pass
-- (cache_migrations/0002_agent_mirror.sql, cache/refresh.rs replace_agent).

UPDATE agent
   SET settings = settings || '{"api_billing_env": ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]}'::jsonb
 WHERE ((name = 'claude' AND transport = 'acp') OR (name = 'claude-cli' AND transport = 'cli'))
   AND jsonb_typeof(settings) = 'object'
   AND NOT (settings ? 'api_billing_env');
