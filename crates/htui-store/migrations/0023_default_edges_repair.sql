-- 0023_default_edges_repair.sql - MOD-138 milestone 1 (plan D7, D8). Forward-only (R-STO-5).
--
-- Settings > Kinds adds, moves, renames and deletes phases one row at a time (create_phase,
-- update_phase, delete_phase), and before MOD-138 none of those writers touched step_graph_edge.
-- A graph edited that way since 0020_step_graph_v2.sql has edges out of step with its positions
-- and names; the position walk hid it. From MOD-138 on, resolve reads the edges, so this
-- migration repairs them once, and the per-row writers keep a default-edged graph default-edged
-- from here on (plan D7).
--
-- The repair (plan D8): for every graph with no 'if', 'asks_you' or 'fails' edge (no editor can
-- author one before MOD-141, so such a graph's edges are all derived), its 'next' and
-- 'changes_requested' edges are deleted and 0020's backfill is applied again: a 'next' edge
-- between each pair of phases adjacent in position order, and for each phase named 'review' a
-- 'changes_requested' edge to the greatest earlier phase named 'implement', else to the phase just
-- before it in position order, with loop_limit = the target's retry_limit. seed::default_edges is
-- the same rule in Rust, pinned equal by a migration test. A graph that carries an authored edge
-- is left exactly as it is. revision is not bumped (0020 did not either): nothing holds a
-- replace_graph token across a migration.
--
-- Idempotent: a second application deletes and re-inserts the same (from, to, kind, loop_limit)
-- rows. No table or column changes, step_graph_edge is not mirrored, so there is no cache
-- migration; schema_version becomes 23, so each box rebuilds its mirror once on first start.

CREATE TEMPORARY TABLE mod138_repaired ON COMMIT DROP AS
SELECT g.id AS graph_id
  FROM step_graph g
 WHERE NOT EXISTS (SELECT 1 FROM step_graph_edge e
                    WHERE e.graph_id = g.id AND e.kind IN ('if', 'asks_you', 'fails'));

DELETE FROM step_graph_edge e
 USING mod138_repaired r
 WHERE e.graph_id = r.graph_id;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind)
SELECT ordered.graph_id, ordered.id, ordered.next_id, 'next'
  FROM (SELECT p.graph_id, p.id,
               LEAD(p.id) OVER (PARTITION BY p.graph_id ORDER BY p.position) AS next_id
          FROM step_graph_phase p
          JOIN mod138_repaired r ON r.graph_id = p.graph_id) ordered
 WHERE ordered.next_id IS NOT NULL;

INSERT INTO step_graph_edge (graph_id, from_phase_id, to_phase_id, kind, loop_limit)
SELECT rv.graph_id, rv.id, t.id, 'changes_requested', t.retry_limit
  FROM step_graph_phase rv
  JOIN mod138_repaired r ON r.graph_id = rv.graph_id
 CROSS JOIN LATERAL (SELECT c.id, c.retry_limit
                       FROM step_graph_phase c
                      WHERE c.graph_id = rv.graph_id
                        AND c.position < rv.position
                      ORDER BY (c.name = 'implement') DESC, c.position DESC
                      LIMIT 1) t
 WHERE rv.name = 'review';
