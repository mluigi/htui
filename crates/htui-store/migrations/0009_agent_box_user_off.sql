-- 0009_agent_box_user_off.sql - MOD-23 (plan D242, OQ-1 answer B; blueprint D249).
-- Forward-only (R-STO-5).
--
-- agent_box.enabled keeps its one meaning, "this box may run this agent", and gains a second
-- author: the probe proposes it (MOD-2 D50: status = ready) and the human vetoes it from
-- Settings > Agents. The veto is its own column so that no re-probe can undo it:
-- upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT agent_box.user_off on conflict, and
-- WriteStore::set_agent_box_enabled is the only writer of user_off (MOD-2 D74's single-writer
-- shape). Every reader of agent_box.enabled is right without a change. agent_box is not mirrored
-- (cache_migrations/0002_agent_mirror.sql), so there is no cache migration.

ALTER TABLE agent_box
    ADD COLUMN user_off BOOLEAN NOT NULL DEFAULT false;

COMMENT ON COLUMN agent_box.user_off IS
    'MOD-23 D242: the per-box switch, written only by set_agent_box_enabled. true keeps enabled '
    'false through every probe: upsert_agent_box writes enabled = EXCLUDED.enabled AND NOT '
    'user_off on conflict. Switching on re-derives enabled from the stored probe status.';
