-- 0023_item_status.sql - MOD-144 (ANA-35 §7.1, §7.10 row 1). Forward-only (R-STO-5).
--
-- item.status shrinks to open | blocked | closed. Progress (queued, running, needs you, stopped,
-- failed, done) is derived from the item's latest graph run: newest kind = 'graph' run by
-- (queued_at, id), cancelled skipped. "One live graph run per item" moves from the item-status
-- lock to create_run's explicit check plus uq_run_item_live; live includes 'stopped' (MOD-125).
--
-- Order: (1) refuse readably when an item already has two live graph runs, naming the items, so the
-- unique index never fails with a bare 23505; (2) uq_run_item_live; (3) the §7.1 mapping, with the
-- updated_at trigger off as 0006 does (the mirror rebuilds on schema_version 23 regardless): a done
-- or failed item whose latest graph run does not say so is "unexplained" -- done -> closed/done with
-- a note, failed -> blocked with a note; every other removed value -> open; closed_at is cleared on
-- every row that is not closed (only close_out stamps it from now on); (4) the CHECK swap.
-- Notes are written as the item's created_by (item_note.created_by is NOT NULL). No COMMENT ON (the
-- ANA column comment pins stay), no new table (TABLES stays 51), no cache migration (cache
-- item.status has no CHECK; the schema_version bump rebuilds every mirror). A headless worker never
-- migrates: migrate from a TUI first.

-- --------------------------------------------------------------------------------------------
-- 1. Pre-check: at most one live graph run per item
-- --------------------------------------------------------------------------------------------

DO $$
DECLARE
    offenders TEXT;
BEGIN
    SELECT string_agg(p.slug || ':' || i.key, ', ' ORDER BY p.slug, i.key_prefix, i.key_number)
      INTO offenders
      FROM item i
      JOIN project p ON p.id = i.project_id
     WHERE (SELECT count(*)
              FROM run r
             WHERE r.item_id = i.id
               AND r.kind = 'graph'
               AND r.status IN ('queued','running','awaiting_approval','stopped')) > 1;
    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION 'MOD-144 (0023_item_status): an item may have one live graph run, and these have more: %. Cancel all but one live run of each with the previous htui, then start this one again.', offenders
            USING ERRCODE = 'check_violation';
    END IF;
END
$$;

-- --------------------------------------------------------------------------------------------
-- 2. Expand: the declarative guard
-- --------------------------------------------------------------------------------------------

CREATE UNIQUE INDEX uq_run_item_live ON run (item_id)
    WHERE kind = 'graph' AND status IN ('queued','running','awaiting_approval','stopped');

-- --------------------------------------------------------------------------------------------
-- 3. Migrate: the §7.1 mapping
-- --------------------------------------------------------------------------------------------

CREATE TEMP TABLE mod144_unexplained AS
SELECT i.id, i.status, i.created_by
  FROM item i
  LEFT JOIN LATERAL (
        SELECT r.status
          FROM run r
         WHERE r.item_id = i.id
           AND r.kind = 'graph'
           AND r.status <> 'cancelled'
         ORDER BY r.queued_at DESC, r.id DESC
         LIMIT 1
       ) lr ON true
 WHERE i.status IN ('done','failed')
   AND lr.status IS DISTINCT FROM i.status;

INSERT INTO item_note (item_id, body, created_by)
SELECT u.id,
       CASE u.status
           WHEN 'done' THEN 'closed by the status migration: marked done with no finished run'
           ELSE 'failed outside a run before the status migration; clear to run it again'
       END,
       u.created_by
  FROM mod144_unexplained u;

ALTER TABLE item DISABLE TRIGGER trg_item_updated_at;

UPDATE item i
   SET status = 'closed', resolution = 'done', closed_at = COALESCE(i.closed_at, now())
  FROM mod144_unexplained u
 WHERE u.id = i.id AND u.status = 'done';

UPDATE item i
   SET status = 'blocked', closed_at = NULL
  FROM mod144_unexplained u
 WHERE u.id = i.id AND u.status = 'failed';

UPDATE item
   SET status = 'open', closed_at = NULL
 WHERE status IN ('queued','in_progress','awaiting_approval','done','failed');

UPDATE item
   SET closed_at = NULL
 WHERE status IN ('open','blocked') AND closed_at IS NOT NULL;

ALTER TABLE item ENABLE TRIGGER trg_item_updated_at;

DROP TABLE mod144_unexplained;

-- --------------------------------------------------------------------------------------------
-- 4. Contract: the three-value CHECK (0001's unnamed CHECK is item_status_check)
-- --------------------------------------------------------------------------------------------

ALTER TABLE item DROP CONSTRAINT item_status_check;
ALTER TABLE item ADD CONSTRAINT chk_item_status
    CHECK (status IN ('open','blocked','closed')) NOT VALID;
ALTER TABLE item VALIDATE CONSTRAINT chk_item_status;
