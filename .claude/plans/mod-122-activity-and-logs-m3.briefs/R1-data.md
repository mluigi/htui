# Brief R1-data — MOD-122 M3 review fix round, data area

The review gate (`rust-reviewer`, data instance) over `e99acd97..HEAD` found no high or medium
defects; these findings are applied in this one fix round. **Confirm each against the code before
acting** (reviewers can be wrong); fix what you confirm, test-first where a test can pin it; list
what you reject, with the reason. Plan context: `.claude/plans/mod-122-activity-and-logs-m3.plan.md`
D2 (F3: "nothing in the trigger can raise on valid data"), D6 (sweeps), D8 (the last rows stay on a
failed read). Read only those sections if you need them.

## L1 — a deferred activity trigger deadlocks against a concurrent project/workspace delete

`crates/htui-store/migrations/0034_activity_event.sql:118` (the `INSERT` in `activity_event_write`;
its FK check on `project_id`/`workspace_id`) with `PgStore::delete_project_once`
(`crates/htui-store/src/pg/write.rs:~11788`) and `delete_workspace_once` (`:~11734`). The transition
holds its row lock on the item/run/step; the delete takes the `project` row lock and its cascade then
waits on that row; at commit the trigger's FK check needs `FOR KEY SHARE` on the project row →
deadlock. Reviewer's probe on PG 18 (scratch DB): session A `BEGIN; UPDATE item SET status='blocked' …;
pg_sleep(2); COMMIT`, session B `BEGIN ISOLATION LEVEL REPEATABLE READ; DELETE FROM project …` →
A fails at commit with `deadlock detected … PL/pgSQL function activity_event_write`. If B is the
victim instead it surfaces `40P01`, which `is_serialization_failure` (`pg/write.rs:~699`, matches
only `40001`) does not retry.

Fix: in both delete paths lock the children before deleting the parent (e.g. `SELECT 1 FROM item
WHERE project_id = $1 FOR UPDATE`, same on `run`, before `DELETE FROM project`; for a workspace, the
rows whose events point at it), so B waits on A, A commits, and B gets the `40001` it already retries.
Also treat `40P01` as retryable in `is_serialization_failure`. Pin it with a two-connection test of
the probe (htui-store `tests/it/activity_pg.rs`). New queries need `.sqlx` entries (migrated scratch
DB; byte-identical SQL reuses an entry).

## L2 — the TUI's `SweepRetention` stops at the first failed prune

`crates/htui/src/store_worker.rs:~2712`: `let logs = writer.prune_worker_logs(cut, PER_BOX_CAP).await?;`
returns before `prune_activity` runs. `ship.rs`'s sweep runs both on purpose. Fix: run both, then
combine (`let logs = …await; let events = …await; Ok(RetentionSwept(logs?.saturating_add(events?)))`
or report the first error after running both). Test: a writer whose worker-log prune fails still
prunes events.

## M1 (from the TUI instance; the fix is in this area) — offline empties the pinned list

`crates/htui-store/src/backend.rs:~634` answers the activity read with an empty page **as success**
while Offline/connecting; the panel's `merge` (`crates/htui/src/ui/shell/activity.rs:~164`) then
replaces `pinned` with nothing, so the "needs you" rows and the title count vanish for the whole
outage. D8: the last rows stay on a failed read. Fix: while the backend is offline or connecting,
the store worker answers `StoreRequest::Activity` with `StoreReply::Failed { request: "activity", .. }`
(already absorbed by the shell, which keeps the rows). Keep `Backend::activity`'s Offline arm if other
callers rely on it, or change it and its test — say which. Test in `store_worker.rs`: an offline
backend answers the activity read with `Failed`.

Files: `crates/htui-store/src/pg/write.rs`, `crates/htui-store/tests/it/activity_pg.rs`,
`crates/htui-store/.sqlx/*` (new entries only), `crates/htui/src/store_worker.rs`, and
`crates/htui-store/src/backend.rs` only if M1 needs it.
Check: `env -u HTUI_TEST_QDRANT_URL cargo test -p htui-store --all-features --test it activity_pg:: pg_criteria:: pg_conformance::`;
`env -u HTUI_TEST_QDRANT_URL cargo test -p htui --all-features --lib -- store_worker`; `cargo sqlx prepare --check` in
`crates/htui-store` against a migrated scratch DB; clippy (`--all-targets --all-features` and featureless) and fmt on
the two crates.
