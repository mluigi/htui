-- 0031_memory_capture.sql - MOD-164 (plan D1, D3): automatic memory capture at run end. Forward-only
-- (R-STO-5). Depends on 0022_memory.sql (memory_entry), 0001_init.sql (run, run_step, item,
-- agent) and 0029_divided_fanout.sql (chk_run_step_stage, run_step.node_key's comment).
--
-- memory_entry gains who wrote it (source: 'person' on every existing row, through the default)
-- and, on a run's entry, the run's provenance: the run, the capture step, the item and the agent,
-- each a nullable foreign key that ON DELETE SET NULL clears, so a run's entry outlives the rows
-- it came from; the step row's model; the newest commit the run landed (stamped by htui, never by
-- the model); and the evidence JSONB, {citations: [{step_id, quote}]}. A person's entry carries
-- none of those (chk_memory_entry_person_has_no_run); item_id is outside that CHECK, kept free
-- for a later person-side link. Three usage counters, written once per run by finish_run (D5),
-- start at 0. idx_memory_entry_step backs the per-step write cap's count and step_id's SET NULL.
--
-- run_step: chk_run_step_stage reserves -4..-1 and takes 'capture' as a fourth stage key, the
-- capture stage's row at -4. As in 0029 the CHECK does not bind a key to its position. Every
-- existing row passes: no writer before 0031 produced -4 or 'capture'. 0022's and 0029's SQL is
-- not edited; 0031 restates memory_entry's table comment and run_step.node_key's column comment.
--
-- Not mirrored: memory is not mirrored (MOD-163 D7), the mirror's run_step has no CHECKs, and
-- 0031 adds no run_step column, so no cache migration. cache_meta.schema_version moves to 31
-- through PgStore::schema_version(), so each box rebuilds its mirror once on first start; a
-- headless worker never migrates: migrate from the TUI first.
-- --------------------------------------------------------------------------------------------
-- 1. memory_entry: source, run provenance, evidence, usage counters
-- --------------------------------------------------------------------------------------------
ALTER TABLE memory_entry
    ADD COLUMN source TEXT NOT NULL DEFAULT 'person'
        CONSTRAINT chk_memory_entry_source CHECK (source IN ('person','run')),
    ADD COLUMN run_id UUID REFERENCES run(id) ON DELETE SET NULL,
    ADD COLUMN step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,
    ADD COLUMN item_id UUID REFERENCES item(id) ON DELETE SET NULL,
    ADD COLUMN agent_id UUID REFERENCES agent(id) ON DELETE SET NULL,
    ADD COLUMN model TEXT,
    ADD COLUMN commit_hash TEXT,
    ADD COLUMN evidence JSONB,
    ADD COLUMN shown_steps INTEGER NOT NULL DEFAULT 0
        CONSTRAINT chk_memory_entry_shown_steps CHECK (shown_steps >= 0),
    ADD COLUMN shown_done INTEGER NOT NULL DEFAULT 0
        CONSTRAINT chk_memory_entry_shown_done CHECK (shown_done >= 0),
    ADD COLUMN shown_failed INTEGER NOT NULL DEFAULT 0
        CONSTRAINT chk_memory_entry_shown_failed CHECK (shown_failed >= 0),
    ADD CONSTRAINT chk_memory_entry_person_has_no_run
        CHECK (source = 'run'
               OR (run_id IS NULL AND step_id IS NULL AND agent_id IS NULL AND model IS NULL
                   AND commit_hash IS NULL AND evidence IS NULL));

-- The write cap's count of a capture step's entries, and step_id's ON DELETE SET NULL lookup.
CREATE INDEX idx_memory_entry_step ON memory_entry(step_id) WHERE step_id IS NOT NULL;

-- --------------------------------------------------------------------------------------------
-- 2. run_step: the capture stage at -4
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step DROP CONSTRAINT chk_run_step_stage;
ALTER TABLE run_step ADD CONSTRAINT chk_run_step_stage
    CHECK ((role = 'stage') = (position < 0)
           AND (role <> 'stage'
                OR (position BETWEEN -4 AND -1
                    AND node_key IS NOT NULL
                    AND node_key IN ('capture','pick','prepare','collect'))));

-- --------------------------------------------------------------------------------------------
-- 3. Comments (pinned verbatim by tests/migrations.rs's MOD164_COLUMN_COMMENTS, and the restated
-- two by MOD163_TABLE_COMMENTS and MOD139_COLUMN_COMMENTS)
-- --------------------------------------------------------------------------------------------
COMMENT ON TABLE memory_entry IS
    'MOD-163 D1, MOD-164 D1: project memory entries (R-ENT-13), written by a person or by a '
    'run''s capture step (source). Not mirrored (MOD-163 D7).';
COMMENT ON COLUMN run_step.node_key IS
    'MOD-139 D1 (ANA-33 7.4, 7.11), MOD-164 D3: a part''s <divided>/<part_key>, or a stage''s '
    'capture, pick, prepare or collect; NULL on a node of the frozen snapshot, which position '
    'names. Written once.';
COMMENT ON COLUMN memory_entry.source IS
    'MOD-164 D1: who wrote the entry, person or run (a run''s capture step). A person''s entry '
    'carries no run provenance and no evidence (chk_memory_entry_person_has_no_run).';
COMMENT ON COLUMN memory_entry.run_id IS
    'MOD-164 D1: the run whose capture step wrote the entry; NULL on a person''s entry, and '
    'NULL again if that run is deleted.';
COMMENT ON COLUMN memory_entry.step_id IS
    'MOD-164 D1: the capture step that wrote the entry; NULL on a person''s entry, and NULL '
    'again if that step is deleted. The write cap counts entries per step.';
COMMENT ON COLUMN memory_entry.item_id IS
    'MOD-164 D1: the item the writing run worked on; NULL on a person''s entry, and NULL again '
    'if that item is deleted.';
COMMENT ON COLUMN memory_entry.agent_id IS
    'MOD-164 D1: the agent that ran the capture step; NULL on a person''s entry, and NULL '
    'again if that agent is deleted.';
COMMENT ON COLUMN memory_entry.model IS
    'MOD-164 D1: the model the capture step ran on, as the step row records it; NULL on a '
    'person''s entry.';
COMMENT ON COLUMN memory_entry.commit_hash IS
    'MOD-164 D1: the newest commit the writing run landed, stamped by htui, never by the '
    'model; NULL on a person''s entry or when the run landed none.';
COMMENT ON COLUMN memory_entry.evidence IS
    'MOD-164 D1: {citations: [{step_id, quote}]}, the verbatim quotes from the run''s steps the '
    'entry rests on; NULL on a person''s entry.';
COMMENT ON COLUMN memory_entry.shown_steps IS
    'MOD-164 D5: how many finished steps'' prompts showed the entry, counted once when the run '
    'finishes.';
COMMENT ON COLUMN memory_entry.shown_done IS
    'MOD-164 D5: how many of the steps counted in shown_steps ended done.';
COMMENT ON COLUMN memory_entry.shown_failed IS
    'MOD-164 D5: how many of the steps counted in shown_steps ended failed.';
