-- 0013_persona_phase_index.sql - MOD-26 milestone 2 (plan D14, D15; milestone 1 review N5).
-- Forward-only (R-STO-5).
--
-- step_graph_phase.persona_id gains an index. delete_persona refuses a persona still bound to a
-- phase by finding its phases by persona_id, and fk_step_graph_phase_persona's ON DELETE RESTRICT
-- check makes the same lookup; without the index both scan every phase. Index only: no table,
-- column or constraint moves, and the SQLite mirror's schema is untouched. schema_version still
-- becomes 13, so each box rebuilds its mirror once on first start (0008's R-56). A headless
-- worker never migrates: migrate from a TUI first.

CREATE INDEX idx_step_graph_phase_persona ON step_graph_phase(persona_id);   -- FK check on persona delete
