-- ------------------------------------------------------------------------------------------------
-- The `agent` registry row, mirrored (MOD-2 milestone 4, plan D31 / D32; `docs/ANA-9.md` 4.4
-- amended).
--
-- Same columns as `migrations/0001_init.sql` `agent`; the `0001_mirror.sql` type mapping. Unscoped:
-- a full replace on every pass beside `app_user`, `workspace` and `workspace_project`, so no
-- `cache_cursor` row ever names it.
--
-- `agent_box` is deliberately absent: it is a probe snapshot whose columns milestone 5's
-- `0002_agent_probe.sql` changes, and offline "which box" is this box (D31). The `0001_mirror.sql`
-- header's "not mirrored: agents" is superseded here, not edited there (forward-only).
-- ------------------------------------------------------------------------------------------------

CREATE TABLE agent (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, transport TEXT NOT NULL,
    launch TEXT NOT NULL, models TEXT NOT NULL DEFAULT '[]', default_model TEXT,
    billing TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
    settings TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
