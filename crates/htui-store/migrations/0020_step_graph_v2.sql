-- 0020_step_graph_v2.sql - MOD-137 (plan D1-D3, D5-D9). Forward-only (R-STO-5).
--
-- Step graphs get a topology of their own and an owner other than a project (ANA-33 §7.1).
--
-- Owner scope (D3): step_graph.project_id becomes nullable and workspace_id is added; at most one
-- is set (ck_step_graph_owner), and neither set is the global workflow library. The unnamed
-- UNIQUE (project_id, name) of 0001_init.sql (Postgres named it step_graph_project_id_name_key)
-- becomes one partial unique index per scope. An override stays a per-item clone, so it is
-- always project-scoped (ck_step_graph_override_in_project). A workspace delete cascades its
-- graphs; MOD-137 lets no kind or item name a non-project graph (plan D4), so nothing holds them.
--
-- revision (D5) is the whole-graph compare-and-set token: every writer of a graph's content bumps
-- it by one, explicitly, in its own statement or transaction (not a trigger), so MemStore and
-- PgStore follow one written rule. The bump fires trg_step_graph_updated_at too.
--
-- step_graph_phase gains the node properties a v2 snapshot will freeze (D7): split (backfilled
-- 'compete' where fan_out > 1, and kept equal to it by ck_phase_split_fan_out), palette,
-- plan_scope, and the divide planner/integrate columns, which only a 'divide' node may set. The
-- split backfill fires trg_step_graph_phase_updated_at on fanned-out phases, which only stales an
-- editor token. The position and name uniques of 0001_init.sql are re-created DEFERRABLE
-- INITIALLY IMMEDIATE under the names uq_step_graph_phase_position / uq_step_graph_phase_name, so
-- replace_graph can swap positions and names inside one transaction (SET CONSTRAINTS ...
-- DEFERRED); no statement targets them with ON CONFLICT. uq_step_graph_phase_graph_id is the
-- (non-deferrable) key the edge table's composite foreign keys reference, so an edge can never
-- point into another graph.
--
-- step_graph_edge (D6) holds the links; its bound is loop_limit, because LIMIT is a reserved word
-- (ANA-33 names it limit). phase_part (D8) holds a divided node's optional default parts. Both
-- are replaced, never edited, so neither has updated_at.
--
-- Backfill (D9): per graph, a 'next' edge between each pair of phases adjacent in position order,
-- and for each phase named 'review' a 'changes_requested' edge to the greatest earlier phase
-- named 'implement', else to the phase just before it in position order, with loop_limit = the
-- target's retry_limit. That is gate::loop_target over graph::resolve's dense renumbering, and
-- review_loop's bound; seed::default_edges states the same rule in Rust and a migration test pins
-- the two equal. Nothing reads edges at run time yet (MOD-138).
--
-- Neither table is mirrored and step_graph is not either, so there is no cache migration.
-- schema_version becomes 20, so each box rebuilds its mirror once on first start. A headless
-- worker never migrates: migrate from a TUI first.

-- D3, D5: owner scope and revision.
ALTER TABLE step_graph ALTER COLUMN project_id DROP NOT NULL;

ALTER TABLE step_graph
    ADD COLUMN workspace_id UUID NULL
        CONSTRAINT fk_step_graph_workspace REFERENCES workspace(id) ON DELETE CASCADE,
    ADD COLUMN revision BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT ck_step_graph_owner CHECK (project_id IS NULL OR workspace_id IS NULL),
    ADD CONSTRAINT ck_step_graph_override_in_project CHECK (NOT is_override OR project_id IS NOT NULL);

ALTER TABLE step_graph DROP CONSTRAINT step_graph_project_id_name_key;

CREATE UNIQUE INDEX uq_step_graph_project_name   ON step_graph (project_id, name)
    WHERE project_id IS NOT NULL;
CREATE UNIQUE INDEX uq_step_graph_workspace_name ON step_graph (workspace_id, name)
    WHERE workspace_id IS NOT NULL;
CREATE UNIQUE INDEX uq_step_graph_library_name   ON step_graph (name)
    WHERE project_id IS NULL AND workspace_id IS NULL;

-- D7: node properties.
ALTER TABLE step_graph_phase
    ADD COLUMN split                TEXT    NOT NULL DEFAULT 'none',
    ADD COLUMN palette              BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN plan_scope           TEXT    NULL,
    ADD COLUMN planner_persona_id   UUID    NULL
        CONSTRAINT fk_step_graph_phase_planner_persona REFERENCES persona(id) ON DELETE RESTRICT,
    ADD COLUMN planner_agent_id     UUID    NULL
        CONSTRAINT fk_step_graph_phase_planner_agent REFERENCES agent(id),
    ADD COLUMN planner_model        TEXT    NULL,
    ADD COLUMN integrate_persona_id UUID    NULL
        CONSTRAINT fk_step_graph_phase_integrate_persona REFERENCES persona(id) ON DELETE RESTRICT,
    ADD COLUMN integrate_agent_id   UUID    NULL
        CONSTRAINT fk_step_graph_phase_integrate_agent REFERENCES agent(id),
    ADD COLUMN integrate_model      TEXT    NULL,
    ADD COLUMN max_parts            INTEGER NULL;

UPDATE step_graph_phase SET split = 'compete' WHERE fan_out > 1;

ALTER TABLE step_graph_phase
    ADD CONSTRAINT ck_phase_split CHECK (split IN ('none', 'compete', 'divide')),
    ADD CONSTRAINT ck_phase_split_fan_out CHECK ((split = 'compete') = (fan_out > 1)),
    ADD CONSTRAINT ck_phase_plan_scope CHECK (plan_scope IN ('parts', 'successors', 'graph')),
    ADD CONSTRAINT ck_phase_max_parts CHECK (max_parts >= 1),
    ADD CONSTRAINT ck_phase_divide_columns CHECK (split = 'divide' OR (
            planner_persona_id   IS NULL AND planner_agent_id   IS NULL AND planner_model   IS NULL
        AND integrate_persona_id IS NULL AND integrate_agent_id IS NULL AND integrate_model IS NULL
        AND max_parts IS NULL));

CREATE INDEX idx_step_graph_phase_planner_persona   ON step_graph_phase (planner_persona_id);
CREATE INDEX idx_step_graph_phase_integrate_persona ON step_graph_phase (integrate_persona_id);

-- D10: deferrable position and name uniques; D6: the key the edges reference.
ALTER TABLE step_graph_phase
    DROP CONSTRAINT step_graph_phase_graph_id_position_key,
    DROP CONSTRAINT step_graph_phase_graph_id_name_key,
    ADD CONSTRAINT uq_step_graph_phase_position UNIQUE (graph_id, position)
        DEFERRABLE INITIALLY IMMEDIATE,
    ADD CONSTRAINT uq_step_graph_phase_name UNIQUE (graph_id, name)
        DEFERRABLE INITIALLY IMMEDIATE,
    ADD CONSTRAINT uq_step_graph_phase_graph_id UNIQUE (graph_id, id);

-- D6: edges.
CREATE TABLE step_graph_edge (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    graph_id      UUID        NOT NULL
        CONSTRAINT fk_step_graph_edge_graph REFERENCES step_graph(id) ON DELETE CASCADE,
    from_phase_id UUID        NOT NULL,
    to_phase_id   UUID        NOT NULL,
    kind          TEXT        NOT NULL,
    predicate     JSONB       NULL,
    loop_limit    INTEGER     NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT fk_step_graph_edge_from FOREIGN KEY (graph_id, from_phase_id)
        REFERENCES step_graph_phase (graph_id, id) ON DELETE CASCADE,
    CONSTRAINT fk_step_graph_edge_to FOREIGN KEY (graph_id, to_phase_id)
        REFERENCES step_graph_phase (graph_id, id) ON DELETE CASCADE,
    CONSTRAINT ck_step_graph_edge_kind
        CHECK (kind IN ('next', 'if', 'asks_you', 'changes_requested', 'fails')),
    CONSTRAINT ck_step_graph_edge_predicate CHECK ((kind = 'if') = (predicate IS NOT NULL)),
    CONSTRAINT ck_step_graph_edge_loop_limit CHECK (loop_limit IS NULL OR loop_limit >= 0),
    CONSTRAINT ck_step_graph_edge_loop_limit_required
        CHECK (kind NOT IN ('changes_requested', 'fails') OR loop_limit IS NOT NULL),
    CONSTRAINT ck_step_graph_edge_not_self CHECK (from_phase_id <> to_phase_id),
    CONSTRAINT uq_step_graph_edge UNIQUE (from_phase_id, to_phase_id, kind)
);

CREATE INDEX idx_step_graph_edge_graph ON step_graph_edge (graph_id);
CREATE INDEX idx_step_graph_edge_to    ON step_graph_edge (to_phase_id);

-- D8: a divided node's default parts.
CREATE TABLE phase_part (
    phase_id    UUID    NOT NULL
        CONSTRAINT fk_phase_part_phase REFERENCES step_graph_phase(id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    key         TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    brief       TEXT    NOT NULL DEFAULT '',
    owned_paths TEXT[]  NOT NULL DEFAULT '{}',
    acceptance  TEXT    NULL,
    agent_id    UUID    NULL CONSTRAINT fk_phase_part_agent REFERENCES agent(id),
    model       TEXT    NULL,
    persona_id  UUID    NULL
        CONSTRAINT fk_phase_part_persona REFERENCES persona(id) ON DELETE RESTRICT,
    CONSTRAINT pk_phase_part PRIMARY KEY (phase_id, position),
    CONSTRAINT uq_phase_part_key UNIQUE (phase_id, key)
);

CREATE INDEX idx_phase_part_persona ON phase_part (persona_id);

-- D9: today's implied topology, as data.
INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind)
SELECT graph_id, id, next_id, 'next'
  FROM (SELECT graph_id, id,
               LEAD(id) OVER (PARTITION BY graph_id ORDER BY position) AS next_id
          FROM step_graph_phase) ordered
 WHERE next_id IS NOT NULL;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind, loop_limit)
SELECT r.graph_id, r.id, t.id, 'changes_requested', t.retry_limit
  FROM step_graph_phase r
  CROSS JOIN LATERAL (SELECT c.id, c.retry_limit
                        FROM step_graph_phase c
                       WHERE c.graph_id = r.graph_id
                         AND c.position < r.position
                       ORDER BY (c.name = 'implement') DESC, c.position DESC
                       LIMIT 1) t
 WHERE r.name = 'review';

COMMENT ON COLUMN step_graph.project_id IS
    'MOD-137 D3: the owning project; NULL for a workspace graph or a library graph. At most one '
    'of project_id and workspace_id is set (ck_step_graph_owner).';
COMMENT ON COLUMN step_graph.workspace_id IS
    'MOD-137 D3: the owning workspace; NULL for a project graph or a library graph (neither set). '
    'ON DELETE CASCADE.';
COMMENT ON COLUMN step_graph.revision IS
    'MOD-137 D5: whole-graph compare-and-set token; every write to the graph, its phases, '
    'candidates, parts or edges bumps it by one; replace_graph compares it.';
COMMENT ON COLUMN step_graph_phase.split IS
    'MOD-137 D7: none, compete (fan_out > 1, rival candidates) or divide (planner, parts, '
    'integrate; ANA-33 7.4); compete iff fan_out > 1.';
COMMENT ON COLUMN step_graph_phase.palette IS
    'MOD-137 D7: a node a planner may instantiate but the walk never reaches on its own; has no '
    'edges.';
COMMENT ON COLUMN step_graph_phase.plan_scope IS
    'MOD-137 D7: what a planner node may plan: parts, successors or graph (ANA-33 7.5); NULL off a '
    'planner.';
COMMENT ON COLUMN step_graph_phase.max_parts IS
    'MOD-137 D7: the most parts a divide planner may propose; divide nodes only, like the six '
    'planner_* and integrate_* columns.';
COMMENT ON TABLE step_graph_edge IS
    'MOD-137 D6: one link between two phases of one graph; replaced, never edited. Acyclicity and '
    'reachability are the shared validator''s (MOD-138), not CHECKs.';
COMMENT ON COLUMN step_graph_edge.kind IS
    'MOD-137 D6: next, if (carries predicate), asks_you, changes_requested or fails (both carry '
    'loop_limit).';
COMMENT ON COLUMN step_graph_edge.predicate IS
    'MOD-137 D6: the if edge''s condition, NULL on every other kind; MOD-138 closes its shape '
    '(ANA-33 7.3).';
COMMENT ON COLUMN step_graph_edge.loop_limit IS
    'MOD-137 D2, D6: how often a loop edge may be taken (ANA-33 limit; LIMIT is reserved); '
    'required on changes_requested and fails.';
COMMENT ON TABLE phase_part IS
    'MOD-137 D8: a divide node''s default parts (ANA-33 7.4, 7.6); written only by replace_graph.';
