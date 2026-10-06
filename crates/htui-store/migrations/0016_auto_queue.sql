-- 0016_auto_queue.sql - MOD-12 milestone 1 (plan D1, D2, D3, D4, D7). Forward-only (R-STO-5).
--
-- The auto-mode queue (MOD-12 PRD D1-D4, docs/ANA-2.md §4.10). queue_entry is one item's opt-in
-- membership of one box's queue: run.status 'queued' already means "a run exists" (create_run moves
-- the item open|failed -> queued), so membership needs a row of its own. An item is queued on at most
-- one box, and the entry goes with its item (ON DELETE CASCADE), which delete_project relies on.
-- queue_batch is one queue activation: a box's queue runs iff it has an open batch (plan D2). Pause
-- closes it 'paused', drain closes it 'drained' (plan D3), and resume opens a new one. The partial
-- unique index keeps at most one open batch per box. run.batch_id is the batch an auto run was
-- admitted under, NULL for manual and chat runs. Milestone 2 sums run_step.usage over it, and no
-- total is stored (ANA-2 §4.10). Nothing here is mirrored, but schema_version becomes 16, so each
-- box rebuilds its mirror once on first start. A headless worker never migrates: migrate from a
-- TUI first.

CREATE TABLE queue_batch (
    id            UUID        PRIMARY KEY,
    box_id        UUID        NOT NULL REFERENCES box(id),
    opened_at     TIMESTAMPTZ NOT NULL,
    opened_by     UUID        NOT NULL REFERENCES app_user(id),
    closed_at     TIMESTAMPTZ,
    closed_reason TEXT,
    CONSTRAINT chk_queue_batch_closed_reason CHECK (closed_reason IN ('paused', 'drained')),
    CONSTRAINT chk_queue_batch_closed CHECK ((closed_at IS NULL) = (closed_reason IS NULL))
);

CREATE UNIQUE INDEX uq_queue_batch_open ON queue_batch(box_id) WHERE closed_at IS NULL;

CREATE TABLE queue_entry (
    item_id   UUID        PRIMARY KEY REFERENCES item(id) ON DELETE CASCADE,
    box_id    UUID        NOT NULL REFERENCES box(id),
    position  INTEGER,
    queued_at TIMESTAMPTZ NOT NULL,
    queued_by UUID        NOT NULL REFERENCES app_user(id)
);

CREATE INDEX idx_queue_entry_box ON queue_entry(box_id);

ALTER TABLE run
    ADD COLUMN batch_id UUID NULL CONSTRAINT fk_run_batch REFERENCES queue_batch(id);

CREATE INDEX idx_run_batch ON run(batch_id) WHERE batch_id IS NOT NULL;

COMMENT ON COLUMN queue_batch.id IS
    'MOD-12 D1: client-minted UUIDv7; what run.batch_id references.';
COMMENT ON COLUMN queue_batch.box_id IS
    'MOD-12 D2: the box whose queue this activation is; at most one open batch per box.';
COMMENT ON COLUMN queue_batch.opened_at IS
    'MOD-12 D2: when the queue was resumed.';
COMMENT ON COLUMN queue_batch.opened_by IS
    'MOD-12 D2: who resumed it.';
COMMENT ON COLUMN queue_batch.closed_at IS
    'MOD-12 D2, D3: when the batch closed; NULL while the queue runs.';
COMMENT ON COLUMN queue_batch.closed_reason IS
    'MOD-12 D2, D3: paused, or drained (no entry left and no live auto run); NULL while open.';
COMMENT ON COLUMN queue_entry.item_id IS
    'MOD-12 D1: the queued item; queued on at most one box, and gone with the item.';
COMMENT ON COLUMN queue_entry.box_id IS
    'MOD-12 D1: the box whose queue holds the item; the local box at queue time (R-ORCH-12).';
COMMENT ON COLUMN queue_entry.position IS
    'MOD-12 D4: an explicit queue position (milestone 3 reorder); NULL sorts last.';
COMMENT ON COLUMN queue_entry.queued_at IS
    'MOD-12 D1: when the item was queued.';
COMMENT ON COLUMN queue_entry.queued_by IS
    'MOD-12 D1: who queued it.';
COMMENT ON COLUMN run.batch_id IS
    'MOD-12 D7: the queue_batch an auto run was admitted under; NULL for manual and chat runs.';
