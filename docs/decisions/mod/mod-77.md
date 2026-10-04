# MOD-77 - Pg step writers lock the step before the run (done, 2026-10-04)

**Requirements:** `R-HIS-1`, `R-ORCH-11`.
**Origin:** MOD-11 (`docs/decisions/mod/mod-11.md`, follow-ups). `park_step` locks `FOR UPDATE OF s, r`
(step → run), but the older fenced step writers locked the run first and then the step. A stale walk
racing the walk that adopted its run could therefore deadlock against a park (`40P01`).
**Artifacts:**
- plan [`.claude/plans/mod-77-step-lock-order.plan.md`](../../../.claude/plans/mod-77-step-lock-order.plan.md): D1-D7, with its verified-claims table;
- blueprint `.claude/plans/mod-77-step-lock-order.blueprint.md`: exact SQL, test helpers, commit plan.

Decision numbers are local to MOD-77 (the MOD-31 convention).

Routed as **plan** (0 criteria fired). Run in a TOOL-7 sandbox (`hr/MOD-77`). One serial implementer,
because every task touched `pg/write.rs` or `tests/pg_criteria.rs`.

**Decisions (maintainer, 2026-10-04):**
- route accepted, no ultracode;
- plan confirmed as written and fact-checked (21 claims, 2 amended: `finish_chat_run` added as D5,
  and `step_fence` given a lock mode, D1);
- fix and `.sqlx` regeneration in one commit, so every commit builds offline (`SQLX_OFFLINE=true`);
- review: M1, L2 and L3 applied; L1 applied as `FOR KEY SHARE` (the maintainer's choice); L4 and L5
  accepted and recorded below.

**Commits:**
- plan and blueprint: `fc7913c4`, `0ae52181`, `9347e0eb`;
- lock-order tests (red): `50917fa4`;
- fix + `.sqlx`: `f1b517df`;
- review round 1: `c25fccdc` (M1 tests), `7c6398cc` (L2, L3);
- review round 2: `bd815b59` (L1, `FOR KEY SHARE`, + `.sqlx`).

---

## What was decided and built

Every fenced Postgres step writer now locks the **step before the run**, which is `park_step`'s and
`promote_step`'s order, so none of them can close a cycle with a park. Postgres takes a joined row's
locks in the order the locking clauses are written. That was probed on 16.15: the control
`FOR SHARE OF r FOR NO KEY UPDATE OF s` holds the run while it waits on the step. So in every
statement below, the step clause comes first, and the order matters.

| Writer | Before | After |
|---|---|---|
| `set_step_usage`, `set_step_prompt`, `finish_step`, `pass_step` | `UPDATE run_step … WHERE EXISTS (… run … FOR SHARE)`: run, then step | one statement with a locking CTE, `FOR NO KEY UPDATE OF s FOR SHARE OF r`, then `UPDATE … FROM locked` (D2) |
| `upsert_step_tree` | `step_fence` (`FOR SHARE OF r`), then an update of `isolation_path` | `step_fence(…, StepLock::Update)`: `FOR NO KEY UPDATE OF s FOR SHARE OF r` (D1) |
| `record_commits`, `fenced_miss` | `step_fence` (`FOR SHARE OF r`) | `step_fence(…, StepLock::KeyShare)`: `FOR KEY SHARE OF s FOR SHARE OF r` (D1, L1) |
| `append_events` | `lease` CTE `FOR SHARE OF r` | `FOR KEY SHARE OF s FOR SHARE OF r` (D3, L1) |
| relay `open_permission` | `FOR SHARE OF r` | `FOR KEY SHARE OF s FOR SHARE OF r` (D4, L1) |
| `finish_chat_run` | `UPDATE run`, then `UPDATE run_step` | `SELECT … FROM run_step … FOR NO KEY UPDATE` first, then the run, then the step (D5) |

- **D1/D2: writers that update the step lock it `FOR NO KEY UPDATE` up front.** A shared step lock
  that a later `UPDATE run_step` upgraded would deadlock two writers of one step against each other.
  `run_step`'s key columns (`id`, the `(run_id, position, attempt, fanout_index)` unique) are never in
  a `SET`, so each `UPDATE` stays at `NO KEY UPDATE` and upgrades nothing.
- **D2's CTE re-checks under `READ COMMITTED`.** Probed: a lease take committed mid-write leaves 0
  rows (`Fenced`); a park committed mid-pass leaves 0 rows (`pass_step`'s `Ok(false)`); an unrelated
  commit leaves 1. `EXPLAIN` shows `CTE locked -> LockRows` under the `Update`, so it is materialized.
  Do not add `NOT MATERIALIZED`. Each writer stays one round trip, and the miss paths are unchanged.
- **L1: reader-inserters take `FOR KEY SHARE` on the step.** That is the lock their foreign-key insert
  already took. It still conflicts with a park's `FOR UPDATE OF s`, so they queue at the step before
  touching the run. It does not conflict with `NO KEY UPDATE`, so `append_events`, the busiest writer,
  does not serialise behind the step's own usage, settle and transition writes. MOD-11's `step_scope`
  keeps `FOR SHARE OF s, r`: it is correct as is and out of this item's scope.
- **D5: `finish_chat_run` is the one run → step path the item did not list.** Left alone, it would
  have formed a new cycle with a chat's usage write. The error order is unchanged: `NotFound{run}`
  before `NotFound{run_step}` (`conformance.rs` `start_chat_run_mints_chat_rows`, `MemStore`).
  Its `closed_step == 0` guard is now unreachable and kept as a defensive check (L2).
- No migration, no trait or signature change, and `MemStore` is untouched. `.sqlx`: 7 + 3 entries
  replaced, plus `finish_chat_run`'s new step lock and the second `step_fence` arm.

## Tests

Deterministic lock-order tests, not races (`crates/htui-store/tests/pg_criteria.rs`). A raw transaction
holds the step, the writer is spawned, and the helper waits until `pg_blocking_pids` shows the writer
blocked by that holder. Then:
- **`*_takes_the_step_first`** (nine writers: the six above plus `append_events`, `record_commits`,
  `open_permission`): the holder holds the step `FOR UPDATE`, then `SELECT … FROM run … FOR UPDATE
  NOWAIT` must succeed, so the writer holds nothing on the run. All nine failed with `55P03` before the
  fix (`50917fa4`).
- **`*_locks_the_step_for_update_up_front`** (the six step updaters, review M1): the holder holds the
  step `FOR SHARE`, then upgrades with `FOR NO KEY UPDATE NOWAIT`, which must succeed, so the writer
  holds no share lock it would later upgrade. Proved by a temporary regression: `upsert_step_tree`
  with the share lock fails with `55P03`.

The existing fence races (`a_lease_take_committed_mid_write_fences_it`, `…_mid_settle_fences_it`,
`a_step_document_racing_a_park_never_deadlocks`) stay green.

Gates on the final tree: fmt and both clippy gates (`--all-targets --all-features` and featureless)
clean; `htui-store` 334 passed (`pg_criteria` 84, `--test-threads=1`); `htui-core` 644; `htui-agent`
596; `htui` `runs_pg` 9 and `chat_usage_pg` 2, with no stack overflow; `cargo sqlx prepare --check`
clean.

## Accepted, not fixed

- **L5 / D7: a multi-step `append_events` batch.** One batch naming two steps of one run can still
  hold the run (taken for the first step's row) while it waits on a parked second step. The trait
  permits such a batch. The only production caller, `Recorder::flush`, writes one step per batch
  (`Recorder` holds one `step`). Documented at `append_events`.
- **L4: a failed lock-order test can leak its database.** If the bounded wait assertion fires, the
  spawned writer is left running and `drop_db` is not reached, as with the existing
  `wait_for_the_take`.
- **Out of scope by construction (D7):** the unfenced single-row step writers, `select_fanout` (steps
  only, in id order), the command queue (foreign-key `KEY SHARE` only), the lease writers (run only)
  and `close_out` (it refuses a live run before it touches a step).
