# Blueprint: MOD-125 M2 — Stop applied by the executor (T1–T5)

> Produced by `code-architect` 2026-11-12 from the confirmed plan
> `.claude/plans/mod-125-m2-stop-applied.plan.md` (D1–D12, T1–T5), PRD
> `.claude/prds/mod-125-stop-and-resume-runs.prd.md`, M1 blueprint
> `.claude/plans/mod-125-m1-stopped-state.blueprint.md`. Tree read at `bd0556a0` (branch `hr/MOD-125`).
> The plan stays the spec. §0 lists where the tree disagrees with it or the plan leaves a gap: rows
> marked **adopted** are implementation detail and need no call; rows marked **DV-n** change or
> contradict a plan decision, or add a file to a task's set, and wait for the maintainer (§7). Until a
> DV is answered, implement the **recommended** branch; every DV names what to flip if overruled.
> No migration in M2. TDD: every commit group below writes its tests first, then the code, and
> commits both once `cargo check --workspace --all-targets --all-features` is green.

## 0. Plan claims contradicted or extended by the tree

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| F1 | D9's "Pg locking the `workspace_stop` row `FOR UPDATE` first" inverts M1's lock order. `delete_workspace_once` later runs `DELETE FROM workspace`, whose `fk_run_workspace ON DELETE SET NULL` cascade locks every `run` row of the workspace. `claim_run` (run `FOR UPDATE`, then `workspace_stop FOR SHARE`, M1 T3.3) and `adopt_runs` (runs `SKIP LOCKED`, then `workspace_stop FOR SHARE`) take the opposite order. Delete holding the stop row and waiting on a run that a claim holds while the claim waits `FOR SHARE` on the stop row is a `40P01`, which `is_serialization_failure` does not retry. | `pg/write.rs:10628-10728` (RR tx, `Ok(None)` = retry only for `40001`); M1 blueprint §6 R1 ("run/step, then `workspace_stop`") | **DV-1** |
| F2 | D9 counts only `stopped` member runs. A stop in flight (runs still `running`, being preempted for up to grace + slack ≈ 7 s) is not refused: the delete cascades the stop row and NULLs `run.workspace_id`, then `settle_stop` still stops the run, leaving a `stopped` run no workspace resume can reach. That is M-3's orphan, reached through the "stopping" window rather than closed. | `stop_run` checks owner + status only (`pg/write.rs:7186-7247`); `pending_stops` was read before the delete | **DV-2** (maintainer call) |
| F3 | The demo fixture's `RUN_8` is a `stopped` run of member project `vulkan-tutorials` with `workspace_id = WORKSPACE_GRAPHICS` (`fixtures.rs` `runs()`, `graph_run(ids::RUN_8, ids::WORKSPACE_GRAPHICS, …, RunStatus::Stopped, …)`). After D9 the demo's `graphics` workspace cannot be deleted, so two existing tests flip: `crates/htui/tests/hierarchy.rs::delete_workspace_keeps_its_projects` (`:877`, in T1) and `crates/htui/tests/hierarchy_pg.rs::a_workspace_delete_rebuilds_the_mirror_its_scope_row_was_in` (`:276-320`, **not** in T1's set). Every conformance delete case uses a fresh workspace and is unaffected. | search of `.delete_workspace(` / `DeleteWorkspace(` | **DV-3** (adds `hierarchy_pg.rs` to T1) |
| F4 | The new type, the helpers and two constants need re-exports in files outside the plan's sets: `crates/htui-core/src/model/mod.rs` (`pub use stop::{PendingStop, …}`), `crates/htui-core/src/store/mod.rs` (the sentence helpers, as `workspace_has_live_run` is at `:55`), `crates/htui-worker/src/lib.rs` (`STOP_SLACK`, `workspace_is_stopped`, as `CANCEL_GRACE` is at `:52`; `worker_pg.rs` imports from the crate root). | — | **DV-4** (file-set completions, no behaviour) |
| F5 | D3 step 3 ("clears the run's `dead_walks` entry and guards if it had no live walk") is already what `take_lease` does: `Engine::take_lease` → `renew_lease` (`engine.rs:2919-2935`) removes the run from `DeadWalks`, releases its guards and stales its dropped requests on a successful take. `DeadWalks` is the engine's type (`engine.rs:353`), owned by the runtime's `Shared.dead_walks: Arc<DeadWalks>` and lent through `EngineParts`; nothing of it moves to T3. | `engine.rs:2919-2935`, `runtime.rs` `Shared` | **adopted**: `settle_stop` adds no `dead_walks` code; it releases guards once more only after a successful `stop_run` (idempotent, `Isolator::release` touches no tree, `isolate.rs:314-319`). |
| F6 | A preempted walk can end two ways, and both release the lease before `settle_stop` runs: a graceful walk returns `Err(Cancelled)` (`settle_walk`, `engine.rs:4360-4366`: "I-6 writes nothing") and `walk_leased`'s error arm releases the lease (`:2755-2762`); a dropped walk runs `abandoned` (`:2980-3001`). The runtime's walk paths treat `None` and `Some(Err(Cancelled))` alike (`abandoned` + `PREEMPTED`, `runtime.rs` `on_run_unless_claimed`, `unblock`). | — | **adopted**: `settle_stop` always retakes through `take_lease` (D3 as planned). |
| F7 | `take_lease` takes an `awaiting_approval` run too. A run that parked between the poll and the settle is taken, `stop_run` answers `false`, and the lease taken must go back. | `mem.rs` `State::take_lease` (`Running \| AwaitingApproval`), `engine.rs:2881-2905` | **adopted**: `settle_stop` releases its lease when `stop_run` answers `false`. |
| F8 | D3's `info!` line names the mode, which `settle_stop(run)` does not receive. | — | **adopted**: the line is the runtime's (`stop_walk` knows the mode); `settle_stop` keeps the plan's signature. |
| F9 | D4's "a pending cancel of the same run wins: `pending_commands` runs first" is racy: `poll_once` **spawns** each `cancel_run` (`runtime.rs` `poll_once`, `spawn_supervised`), so a stop task spawned after it can take the run lock first; `cancel_enabled` then refuses the `stopped` run and the cancel row stays pending until M3 (M1 R4). | `runtime.rs:3161-3195`, M1 blueprint row 30 | **adopted**: `poll_once` skips every pending stop whose run has a pending cancel row in the same poll's listing. The plan's outcome (cancel wins) then holds deterministically. |
| F10 | The escalation path `Walks::preempt(run)` would also cancel a stop task's own child token if the stop task minted one for its lock wait (`lock_announcing` answers `None` on a cancelled token), dropping the settle. | `runtime.rs:573-590,812-818` | **adopted**: `stop_walk` mints no child; it waits on `Shared.locks.lock(run)` directly, so `preempt` only ever reaches the walk. |
| F11 | `Unblock` does not go through `on_run_unless_claimed`: `unblock()` (`runtime.rs:3205-3250`) locks the run itself. | — | **adopted**: D7's check for `Unblock` sits in `unblock()`, under the lock, after the case re-check, for `UnblockCase::Resume` only. |
| F12 | D8: "the TUI queue header's `slots_used` (`queue_overview.rs:97`) reads through `admit`'s count". It calls `backend.queued_runs_on_box` directly. | `queue_overview.rs:97` | **adopted**: same effect (the store filter drops them); no edit to `queue_overview.rs`. |
| F13 | D10 names `workspace_gone(ws)`; the sentence the plan fixes carries no id. | — | **adopted**: `workspace_gone() -> String`, no argument. |
| F14 | D1 places the Pg body beside `pending_commands` in `pg/relay.rs`. The M1 stop verbs live in `pg/write.rs` (`workspace_stop` `~:7117`, `stop_run` `:7186`). | — | **adopted**: the body is a method in `pg/write.rs`'s `impl WriteStore for PgStore`, right after `stop_run`; `pg/relay.rs` is untouched. |
| F15 | `delete_workspace_live_literal_matches_run_status` (`pg/read.rs:4025`) scans `delete_workspace_once`'s body for the live filter literal and its negative control `replacen`s the **first** occurrence. A second copy of `r.status IN ('queued', 'running', 'awaiting_approval', 'stopped')` in the body would break the control. | `pg/read.rs:4007-4045` | **adopted**: D9's SQL spells `r.status = 'stopped'` (and DV-2's `r.status = 'running'`), never the `IN (...)` list. |
| F16 | The PRD question "Mapping onto ACP and the CLI adapter" (`prd.md:132`) has two halves; M2 settles the cancel half (D2), the `--resume` redo is M3's. | `prd.md:132` | **DV-5**: annotate, do not tick. |
| F17 | T4: `.claude/skills/htui-dev/tests/cargo.test.ts` mentions `htui@0.1.0` / `htui-core@0.1.0` only as `-p` spellings the hook parses (it never compares them with `Cargo.toml`). No snapshot, test or doc compares a literal `0.1.0` with the build. The emitted Claude plugin's version is `claude_emit::VERSION = env!("CARGO_PKG_VERSION")` (`claude_emit.rs:28`), tests use the constant. | `cargo.test.ts:166-191`; `git grep 0.1.0` | **adopted**: no edit beyond the nine manifests and `Cargo.lock`. Operational: an installed relay **and** an emitted plugin from the old build read as stale (`htui 0.2.0 ≠ plugin 0.1.0`, `docs/htui-claude-plugin.md:222`): reinstall / `htui claude emit --force` (§8). |
| F18 | D7 lists the walking set exactly as `Kit::refusal` does (`runtime.rs:1077-1086`, "`c`, `p`, `C` and `T` walk nothing"). `PromoteStep` is therefore not refused, but on a run in a stopping workspace it parks the run `awaiting_approval` (out of `pending_stops`' reach) and opens a live chat session. | `command.rs` `PromoteStep` doc | **Question Q1** (§8); the plan's set is implemented. |

---

## 1. Shared decisions for every task

- **`PendingStop` order**: `(run.queued_at, run.id)`, both stores (`RunId`'s `Ord` is uuid byte order, Postgres' order; `mem.rs:1053-1054`).
- **What `pending_stops(owner)` lists**: runs with `status = 'running'`, `lease_owner = owner` (live or lapsed, as `pending_commands`' owner arm), and a `workspace_stop` row for `run.workspace_id`. Not `queued`, not `awaiting_approval`, not `stopped`, not a `workspace_id IS NULL` run, never another owner's. The mode is the stop row's current mode, so an escalation shows on the next poll.
- **Stop grace** (runtime): `Graceful` → `Shared.cancel_grace` (`CANCEL_GRACE` = 2 s in production, `with_cancel_grace` in tests), `Forced` → `Duration::ZERO`; then the walk's token drops at grace + `STOP_SLACK` (5 s). Cancel keeps grace + 1 s.
- **Error taxonomy** unchanged: new refusals are `StoreError::Constraint(<sentence helper>)`; Pg maps the two workspace FKs by constraint name, as `parent_fk_refused` does (`pg/write.rs:~1257`).
- **Sentences** (all in `crates/htui-core/src/store/traits.rs` beside `workspace_has_live_run`, `:3674-3691`, re-exported from `store/mod.rs`):
  ```rust
  /// MOD-125 M2 D9 (M1 review M-3): `delete_workspace` while `runs` (≥ 1) of its member projects'
  /// runs that recorded this workspace are stopped (DV-2: or still running under its stop). A
  /// scope-row run is counted by [`workspace_has_live_run`] alone. The advice names resume: a
  /// stopped run is cancelled or finished only once resumed (M3).
  #[must_use]
  pub fn workspace_has_stopped_run(workspace: &str, runs: u64) -> String {
      let (noun, pronoun) = if runs == 1 { ("run", "it") } else { ("runs", "them") };
      format!(
          "workspace `{workspace}` has {runs} stopped {noun}; resume the workspace and finish or \
           cancel {pronoun} before deleting the workspace"
      )
  }

  /// MOD-125 M2 D10 (M1 review L-3): a run or a queue entry names a workspace that is gone, in
  /// both stores' words (Pg maps `fk_run_workspace` / `fk_queue_entry_workspace` to it).
  #[must_use]
  pub fn workspace_gone() -> String {
      "this workspace no longer exists: switch workspace".to_owned()
  }
  ```
  Runtime (D7), in `crates/htui-worker/src/runtime.rs` beside `CANCEL_REQUESTED`, re-exported from `lib.rs`:
  ```rust
  /// MOD-125 M2 D7: a walking command on a run whose workspace is stopped.
  #[must_use]
  pub fn workspace_is_stopped(slug: &str) -> String {
      format!("workspace `{slug}` is stopped: resume it first")
  }
  ```
  (The plan's text with the slug in backticks, as every other slug sentence; F13-style detail.)

---

## T1 — Store: `pending_stops`, the admission count, M-3, L-3

Files: `crates/htui-core/src/{model/stop.rs, model/mod.rs (DV-4), store/{traits,worker,mem,conformance,mod (DV-4)}.rs}`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/{pg/read.rs, pg/write.rs, worker.rs, writer.rs}`,
`crates/htui-store/.sqlx/`, `crates/htui-store/tests/{pg_conformance,pg_criteria}.rs`,
`crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs`,
`crates/htui/tests/hierarchy.rs`, `crates/htui/tests/hierarchy_pg.rs` (DV-3). `pg/relay.rs` untouched (F14).

### T1.1 `PendingStop` (`crates/htui-core/src/model/stop.rs`, appended)

```rust
use crate::model::ids::RunId;   // add to the existing `use crate::model::ids::{…}`

/// One stop this process applies (MOD-125 M2 D1): a `running` run it leases whose workspace has a
/// `workspace_stop` row, with that row's mode. Read by the command poll
/// ([`WriteStore::pending_stops`](crate::store::WriteStore::pending_stops)); never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingStop {
    /// `run.id`.
    pub run: RunId,
    /// `workspace_stop.mode` of the run's workspace, as read now: a forced upsert over a graceful
    /// stop shows on the next poll.
    pub mode: StopMode,
}
```
`model/mod.rs`: `pub use stop::{PendingStop, StopMode, WorkspaceStop};` (DV-4). The module doc's
first line gains "and what the executor's poll reads of it (M2)".

### T1.2 `WriteStore::pending_stops` (`traits.rs`, right after `pending_commands`, `:2294`)

```rust
    /// MOD-125 M2 D1: the stops this process applies: every `running` run whose
    /// `lease_owner = owner` (live or lapsed, as [`pending_commands`](Self::pending_commands)'
    /// owner arm) and whose `workspace_id` has a `workspace_stop` row, with that row's mode, in
    /// `(queued_at, id)` order. A run without a workspace, a parked, queued or stopped run, and
    /// another owner's run are never listed: a lapsed lease is the sweep's (`adopt_runs` settles
    /// it, M1 D6), and a run leased elsewhere is that process's poll. No row is written: the
    /// request is the `workspace_stop` row itself.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn pending_stops(&self, owner: Uuid) -> Result<Vec<PendingStop>>;
```
Import `PendingStop` in `traits.rs`' model `use`.

**`WorkerStore`** (`crates/htui-core/src/store/worker.rs`, after `pending_commands`, `:436-441`):
```rust
    /// [`WriteStore::pending_stops`].
    fn pending_stops(&self, owner: Uuid) -> impl Future<Output = Result<Vec<PendingStop>>> + Send;
```
`impl WorkerStore for MemStore` (`:998`): `async fn pending_stops(&self, owner: Uuid) -> Result<Vec<PendingStop>> { WriteStore::pending_stops(self, owner).await }`.
Module doc (`:15-25`): "`WorkerStore` is **71** methods: 16 [`ReadStore`] reads …, **8** [`WriteStore`] reads, 29 writes …; MOD-125 M1 adds `workspace_stop` and `stop_run`, **MOD-125 M2 the read `pending_stops`**; …".
Add `PendingStop` to the model `use` list.

**Forwards** (each one line, beside the implementor's `pending_commands`):
- `crates/htui-store/src/worker.rs` `impl WorkerStore for PgStore` (`:389`) and `for Writer` (`:809`): `WriteStore::pending_stops(self, owner).await` (by path, as their neighbours).
- `crates/htui-store/src/writer.rs` `impl WriteStore for Writer` (`:1673`): `match self { Self::Memory(store) => store.pending_stops(owner).await, Self::Online(pg) => pg.pending_stops(owner).await }`.
- `crates/htui-agent/src/conformance.rs` `UsageSpy` (`:1596`) and `crates/htui-agent/tests/recorder.rs` `SpyStore` (`:1322`): `self.inner.pending_stops(owner).await` (return type `StoreResult<Vec<PendingStop>>`).

### T1.3 MemStore (`crates/htui-core/src/store/mem.rs`)

`State`, after `pending_commands` (`:9616-9641`):
```rust
    /// MOD-125 M2 D1: [`WriteStore::pending_stops`]: the owner's running runs whose workspace has
    /// a stop record, with its mode, by `(queued_at, id)`.
    fn pending_stops(&self, owner: Uuid) -> Vec<PendingStop> {
        let mut rows: Vec<(DateTime<Utc>, PendingStop)> = self
            .runs
            .values()
            .filter(|row| {
                row.status == RunStatus::Running && self.lease_owners.get(&row.id) == Some(&owner)
            })
            .filter_map(|row| {
                let stop = self.workspace_stops.get(&row.workspace_id?)?;
                Some((row.queued_at, PendingStop { run: row.id, mode: stop.mode }))
            })
            .collect();
        rows.sort_unstable_by_key(|(at, stop)| (*at, stop.run));
        rows.into_iter().map(|(_, stop)| stop).collect()
    }
```
`impl WriteStore for MemStore`, after `pending_commands` (`:11129`):
`async fn pending_stops(&self, owner: Uuid) -> Result<Vec<PendingStop>> { Ok(self.read(|state| state.pending_stops(owner))) }`.

**D8 `queued_runs_on_box`** (`:1058-1069`): filter becomes
```rust
.filter(|row| {
    row.target_box_id == box_id
        && row.status == RunStatus::Queued
        // MOD-125 M2 D8: a stopped workspace's queued run takes no free slot and is not
        // re-claimed every sweep; it reappears at the resume.
        && row.workspace_id.is_none_or(|ws| !state.workspace_stops.contains_key(&ws))
})
```
Doc: append "MOD-125 M2 D8: less the queued runs of a stopped workspace (they hold no slot and
cannot be claimed until the resume)."

**D9 `State::delete_workspace`** (`:5783`), right after the MOD-147 A1 block (`:5798-5815`), before `workspace_reach_of`:
```rust
        // MOD-125 M2 D9 (M-3): a stopped run of a member project that recorded this workspace
        // would lose the only resume that reaches it (`ON DELETE SET NULL`). Scope-row runs are
        // A1's above. DV-2: a run still `running` under this workspace's stop is about to be.
        let stopping = self.workspace_stops.contains_key(&id);
        let held = self
            .runs
            .values()
            .filter(|run| {
                run.workspace_id == Some(id)
                    && Some(run.project_id) != scope
                    && (run.status == RunStatus::Stopped
                        || (stopping && run.status == RunStatus::Running))
            })
            .count();
        if held > 0 {
            let slug = &self.workspaces[&id].slug;
            return Err(StoreError::Constraint(workspace_has_stopped_run(slug, rows(held))));
        }
```
(If DV-2 is declined: drop `stopping` and the `||` arm.) Doc of the fn: add the paragraph.

**D10**: `queue_item` (`:1101-1111`) and `State::create_run` (`:6377-6385`): replace
`references_no_row("…workspace_id", workspace, "workspace")` with `workspace_gone()` (the `if let`
guards unchanged). Keep the `// MOD-125 D3: fk_run_workspace.` comment, add "(M2 D10: in the
readable sentence)".

### T1.4 PgStore

**`pending_stops`** (`crates/htui-store/src/pg/write.rs`, `impl WriteStore for PgStore`, right after `stop_run`, `:7247`):
```rust
    /// MOD-125 M2 D1: starts from `workspace_stop` (one row per stopped workspace) and joins
    /// `run` on `idx_run_workspace_status` (`0019_stop_runs.sql:37`); no index covers
    /// `lease_owner`, so the stop rows are the narrow side. Read on the pool.
    ///
    /// # Errors
    ///
    /// The backend's own failures only.
    async fn pending_stops(&self, owner: Uuid) -> Result<Vec<PendingStop>> {
        sqlx::query_as!(
            PendingStop,
            r#"
            SELECT r.id    AS "run: RunId",
                   ws.mode AS "mode: StopMode"
              FROM workspace_stop ws
              JOIN run r ON r.workspace_id = ws.workspace_id
             WHERE r.status = 'running' AND r.lease_owner = $1
             ORDER BY r.queued_at, r.id
            "#,
            owner,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }
```

**D8 `queued_runs_on_box`** (`pg/read.rs:2472`): the query becomes
```sql
SELECT id AS "id: RunId", queued_at
  FROM run
 WHERE target_box_id = $1 AND status = 'queued'
   AND NOT EXISTS (SELECT 1 FROM workspace_stop ws WHERE ws.workspace_id = run.workspace_id)
 ORDER BY queued_at, id
```
(`run.workspace_id IS NULL` makes the subquery empty, so unscoped runs stay.) Doc as Mem's.
`WorkerHost::queued_runs_on_box`'s doc (`core worker.rs:603`) gains the same clause.

**D9** (`delete_workspace_once`, `pg/write.rs:10628`), immediately after the A1 `live > 0` block
(`:10680-10701`), before `scope_row_id`, **no `workspace_stop` lock** (DV-1):
```rust
        // MOD-125 M2 D9 (M-3): a member project's run that recorded this workspace and is
        // stopped (DV-2: or running under the workspace's stop) refuses. Scope-row runs are A1's
        // (no double count). No row is locked here (DV-1): `workspace_stop` before `run` would
        // invert `claim_run`'s and `adopt_runs`' order against the `SET NULL` cascade below. A run
        // stopped after this snapshot conflicts with that cascade, the `40001` retries the
        // attempt, and the recount sees it (`pg_criteria.rs::delete_workspace_recounts_a_run_stopped_under_it`).
        let held = sqlx::query_scalar!(
            r#"
            SELECT count(*) AS "held!"
              FROM run r JOIN project p ON p.id = r.project_id
             WHERE r.workspace_id = $1
               AND p.owner_workspace_id IS DISTINCT FROM $1
               AND (r.status = 'stopped'
                    OR (r.status = 'running'
                        AND EXISTS (SELECT 1 FROM workspace_stop ws WHERE ws.workspace_id = $1)))
            "#,
            id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if held > 0 {
            let slug = sqlx::query_scalar!("SELECT slug FROM workspace WHERE id = $1", id.as_uuid())
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx)?;
            return Err(StoreError::Constraint(workspace_has_stopped_run(&slug, rows(held))));
        }
```
The `SELECT slug …` literal is byte-identical to A1's, so it reuses its `.sqlx` entry. If DV-2 is
declined, drop the `OR (…)` arm. If DV-1 is overruled, insert
`SELECT 1 AS "one!" FROM workspace_stop WHERE workspace_id = $1 FOR UPDATE` (mapping a `40001`
to `Ok(None)`) before this count, and accept the `40P01` risk F1 names.

**D10** helper (beside `parent_fk_refused`, `~:1257`):
```rust
/// MOD-125 M2 D10 (M1 review L-3): `constraint`'s `23503` (`fk_run_workspace`,
/// `fk_queue_entry_workspace`) in [`workspace_gone`]'s sentence, `MemStore`'s; anything else
/// through [`map_sqlx`].
fn workspace_fk_refused(err: sqlx::Error, constraint: &str) -> StoreError {
    if let sqlx::Error::Database(db) = &err
        && db.constraint() == Some(constraint)
    {
        return StoreError::Constraint(workspace_gone());
    }
    map_sqlx(err)
}
```
Sites: `create_run`'s `INSERT INTO run … RETURNING` (`:6342-6382`): `.fetch_one(&mut *tx).await.map_err(|err| workspace_fk_refused(err, "fk_run_workspace"))?`;
`queue_item`'s `INSERT INTO queue_entry … ON CONFLICT (item_id) DO NOTHING` (`:10911-10925`):
`.execute(&mut *tx).await.map_err(|err| workspace_fk_refused(err, "fk_queue_entry_workspace"))?`.
Both docs' `# Errors` say "an unknown workspace is [`workspace_gone`]'s sentence".

**Trait doc** `WriteStore::delete_workspace` (`traits.rs:1458-1474`), `# Errors` gains:
"[`StoreError::Constraint`] ([`workspace_has_stopped_run`], MOD-125 M2 D9) while a run of a member
project that recorded this workspace is `stopped` (or `running` under the workspace's stop, DV-2),
checked after A1's; nothing is deleted then." `create_run`'s and `queue_item`'s Mem docs likewise.

### T1.5 Conformance (`crates/htui-core/src/store/conformance.rs`)

Five `CASES` entries appended after `"a_deleted_workspace_unscopes_its_runs_and_drops_its_stop"`
(`:274`) under a `// MOD-125 M2: the executor's read, M-3 and L-3` comment, and five arms after
`:915-920` (each `Box::pin(case(store)).await`). Imports: `PendingStop`, `workspace_gone`,
`workspace_has_stopped_run`. Helpers used: `create_scoped_run` (`:8076`), `mint_tagged`
(`:6903`, fresh open FEAT items), `new_workspace`, `seam_clock`, `LEASE`, `run_row`, `step_row`.
The fixture box takes **2** running claims (`max_concurrent_items`), so each case keeps at most two
runs `running` at a time; `awaiting_approval` and `stopped` hold no slot.

| Case | Body / asserts |
|---|---|
| `pending_stops_lists_the_owners_running_runs_of_a_stopped_workspace` | `W1 = ids::WORKSPACE_PLATFORM`, `W2 = create_workspace(new_workspace("second"))`. A (`HTUI_ANA_2`, W1) and B (`HTUI_CLEAN_1`, W2) claimed by `owner`; `stop_workspace(W1, Graceful)`, `stop_workspace(W2, Forced)`. `pending_stops(owner) == [A Graceful, B Forced]` in `(queued_at, id)` order (sort the expected pair by `(queued_at, id)` read back); `pending_stops(stranger) == []`. **Not stopped**: `stop_run(B, owner)` → `[A]`. **Not parked**: `transition_run(A, Running, AwaitingApproval)` → `[]`. **Not another owner's**: C (minted item, W1) claimed by `stranger` → `pending_stops(stranger) == [C Graceful]`, `pending_stops(owner) == []`. **No workspace**: G (minted, `None`) claimed by `owner` → still `[]`. **Not queued**: D (minted, W1) created only → never listed. The fixture's `RUN_4` (`running`, W1, no lease) is never listed (the owner is fresh). |
| `pending_stops_shows_an_escalation` | A (W1) claimed by `owner`; graceful stop → `[A Graceful]`; forced stop → `[A Forced]`; a later graceful request → still `[A Forced]` (M1's never-downgrade, read through the new verb). |
| `pending_stops_is_empty_after_resume` | A (W1) claimed; stop → `[A …]`; `resume_workspace(W1)` → `true`; `pending_stops(owner) == []`; the run is still `running` and leased (`refresh_lease(A, owner)` true). |
| `delete_workspace_refuses_while_a_member_run_of_it_is_stopped` | `W = create_workspace(new_workspace("held"))`; A (`HTUI_ANA_2`, `Some(W)`) claimed, `stop_run(A, owner)` (no workspace stop needed). `delete_workspace(W)` → `Err(Constraint(workspace_has_stopped_run("held", 1)))`; `workspace(W)` still exists, `run_row(A).workspace_id == Some(W)`. DV-2 half: B (`HTUI_CLEAN_1`, `Some(W)`) claimed and `running`; `stop_workspace(W, Graceful)` → the delete now says `…has 2 stopped runs…`. Then `transition_run(A, Stopped, Cancelled)` and `transition_run(B, Running, Cancelled)` (both sanctioned) → `delete_workspace(W)` is `Ok`, `run_row(A).workspace_id == None`. (Declined DV-2: drop B.) |
| `create_run_naming_a_gone_workspace_reads_switch_workspace` | `W = create_workspace(new_workspace("gone"))`, `delete_workspace(W)` ok; `create_run(NewRun { workspace_id: Some(W), ..new_run(PROJECT_HTUI, HTUI_ANA_2, vec![]) })` → `Err(StoreError::Constraint(m))` with `m == workspace_gone()`; the item has no new run (`runs(HTUI_ANA_2)` unchanged). |

**Pins**: `crates/htui-core/tests/mem_store.rs:37` and `crates/htui-store/tests/pg_conformance.rs:59`
`277 → 282`; each message/doc gains "and MOD-125 M2's five (`pending_stops` ×3, M-3, L-3) make it
282". Intermediate pins per commit group are in "T1 commits".

### T1.6 Off-trait tests (Mem unit + `pg_criteria` twin)

| Test | Where | Asserts |
|---|---|---|
| `queued_runs_on_box_skips_a_stopped_workspaces_runs` | `mem.rs` tests (beside `:15008`) + `pg_criteria.rs` (beside `:6232`) | Fresh `W` (`create_workspace`), two minted items: X queued with `Some(W)`, Y queued with `None` (both `target_box_id = BOX`). The fixture's own queued run (`RUN_2`, PLATFORM) leads. Before: list = fixture + `[X, Y]` by `(queued_at, id)`. `stop_workspace(W)` → fixture + `[Y]`. `resume_workspace(W)` → fixture + `[X, Y]` again. (Do not stop PLATFORM: it would drop `RUN_2` too.) |
| `queue_item_naming_a_gone_workspace_reads_switch_workspace` | `mem.rs` tests (beside `queue_item_records_its_workspace`) + `pg_criteria.rs` (beside `:12087`) | `W` created then deleted; `queue_item(item, BOX, Some(W), USER, at)` → `Err(Constraint(workspace_gone()))`; `queue_entries(BOX)` has no entry for the item. |
| `delete_workspace_recounts_a_run_stopped_under_it` | `pg_criteria.rs` only (DV-1) | Member run A of `W` claimed (`running`). Connection T: `BEGIN; UPDATE run SET status = 'stopped', lease_owner = NULL WHERE id = A` (uncommitted). Spawn `delete_workspace(W)` on the store; sleep 200 ms; `T.commit()`. The delete answers `Err(Constraint(workspace_has_stopped_run(slug, 1)))` within 5 s (the cascade's `40001` retried, the recount saw it); the workspace exists. No `40P01` anywhere. |

### T1.7 `crates/htui/tests/hierarchy.rs` (and DV-3's fixes)

- New `a_workspace_with_a_stopped_run_is_refused_on_the_status_line`: `hierarchy_over(MemStore::demo())`
  (`:1580`; the section opens on `graphics`, whose member run `RUN_8` is `stopped`), `size(200, 30)`;
  `d` on the workspace row, settle, `y`, type `graphics` (one `harness.key` per char unless the
  harness has a text helper), `enter`, settle. The frame contains
  `"delete_workspace: constraint violated"` and `"workspace `graphics` has 1 stopped run"`, and the
  tree still shows `graphics` (nothing was deleted). Pattern: `:3289`.
- `delete_workspace_keeps_its_projects` (`:877`): before the `DeleteWorkspace` request, settle
  `RUN_8`: `store.transition_run(ids::RUN_8, RunStatus::Stopped, RunStatus::Cancelled, Utc::now())`
  (`expect` true), with a comment "MOD-125 M2 D9: the demo's stopped `RUN_8` would refuse the delete".
  The reach assertions are unchanged (runs are not counted in `DeleteReach`).
- DV-3: `crates/htui/tests/hierarchy_pg.rs::a_workspace_delete_rebuilds_the_mirror_its_scope_row_was_in`
  (`:276-320`): the same `transition_run` on the Pg demo store before the request.

### T1 tests first, then code — and `.sqlx`

`cargo sqlx prepare` against a migrated scratch DB after commit groups 1, 2, 3 and 4 (each changes or
adds a query): see §5.

### T1 commits (each compiles; tests and code together)

1. `MOD-125 M2 T1: PendingStop and WriteStore::pending_stops` — T1.1, T1.2 (trait, WorkerStore + 3 impls, Writer, spies), Mem/Pg bodies, the three `pending_stops_*` cases, pins `277 → 280`, `.sqlx`.
2. `MOD-125 M2 T1: queued_runs_on_box skips a stopped workspace's queued runs` — D8 both stores, docs, the Mem/Pg twin, `.sqlx`.
3. `MOD-125 M2 T1: delete_workspace refuses while a member run is stopped` — sentence helper + re-export, Mem, Pg, trait doc, the conformance case (pins `→ 281`), `delete_workspace_recounts_a_run_stopped_under_it`, T1.7 (hierarchy.rs test + fix, hierarchy_pg.rs fix), `.sqlx`.
4. `MOD-125 M2 T1: a gone workspace reads "switch workspace"` — `workspace_gone` + re-export, Mem 2 sites, Pg helper + 2 sites, the conformance case (pins `→ 282`), the queue_item twins.

Validate: `cargo test -p htui-core --all-features --test mem_store`;
`cargo test -p htui-core --all-features --lib store::mem`;
`cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1`;
`cargo test -p htui --all-features --test hierarchy --test hierarchy_pg -- --test-threads=1`.

---

## T2 — Engine: `settle_stop`, the verifier's group, the divided-step matrix

Files: `crates/htui-orch/src/{engine.rs, conformance.rs, verify.rs}`, `crates/htui-orch/tests/fake_conformance.rs`. Needs only M1's `stop_run`.

### T2.1 `Engine::settle_stop` (`engine.rs`, right after `cancel_leased`, `:2712`)

```rust
    /// MOD-125 M2 D3: settles `run` as `stopped` once this process's walk of it is quiesced —
    /// preempted and abandoned, ended on the run's cancel signal, or never live here (a sweep
    /// adoption before the walk began, a [`DeadWalks`] entry). `cancel_leased`'s shape minus
    /// everything terminal: no `finish_run`, no `cleanup_walk`, so the trees, the branch and the
    /// session banner stay and the item stays `in_progress` (M1 D2).
    ///
    /// 1. [`Self::take_lease`]: the preempted walk released the lease (`walk_leased`'s error arm,
    ///    or [`Self::abandoned`]); a take also drops a dead walk's entry and guards (plan D140,
    ///    D210). A run that is not `running`/parked, runs on another box, or whose lease a live
    ///    stranger holds answers `Ok(false)` with nothing written.
    /// 2. Under the lease: every follow-up window closes and a pending follow-up is refused with
    ///    [`FOLLOW_UP_SESSION_ENDED`] (D12; idempotent, best-effort), then the store's
    ///    [`stop_run`](htui_core::store::WorkerStore::stop_run): `running → stopped`, the running
    ///    steps `stopped`, the lease released in the same transaction.
    /// 3. `true`: the run's guards are released once more (touching no tree). `false` (it parked
    ///    meanwhile, or another writer settled it): the lease this call took goes back.
    ///
    /// The caller holds the run's lock (MOD-4 plan D157). Writes no activity row, as
    /// `cancel_leased` writes none; the runtime logs the stop with its mode.
    ///
    /// # Errors
    /// The store's own failures and an unknown run (`NotFound`); every "not this stop's to write"
    /// answer is `Ok(false)`.
    pub async fn settle_stop(&self, run: RunId) -> Result<bool, EngineError> {
        match self.take_lease(run).await {
            Ok(_) => {}
            Err(EngineError::RunStatus { .. } | EngineError::LeaseHeld { .. }) => return Ok(false),
            Err(err) => return Err(err),
        }
        let stopped = self
            .leased_window(run, async {
                self.close_finished_follow_ups(run, FOLLOW_UP_SESSION_ENDED).await;
                Ok(self.parts.store.stop_run(run, self.parts.owner).await?)
            })
            .await?;
        if stopped {
            self.release_guards(run, "releasing a stopped run's guards failed").await;
        } else {
            self.release_lease(run).await;
        }
        Ok(stopped)
    }
```
Fake wiring (`engine.rs`, beside `claim_fake`, `:10481`), so the conformance `impl Orchestrate for
FakeOrchestrator` needs no `fake.rs` edit:
```rust
/// [`Engine::settle_stop`] over a `FakeOrchestrator`'s parts (MOD-125 M2 conformance).
///
/// # Errors
/// As [`Engine::settle_stop`].
#[cfg(feature = "test-support")]
pub async fn settle_stop_fake(
    orch: &crate::fake::FakeOrchestrator,
    run: RunId,
) -> Result<bool, EngineError> {
    let graphs = orch.graphs();
    let driver = |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key);
    let scrubber = RunSecrets::new(orch.secret_source());
    let engine = Engine::new(fake_parts(orch, &graphs, &driver, &scrubber).await?);
    engine.settle_stop(run).await
}
```

**Engine unit tests** (`engine.rs` test module; the `Harness` / `leased_run` helpers, `:12565`):

| Test | Asserts |
|---|---|
| `a_stop_retakes_the_released_lease_and_settles_run_and_step_stopped_keeping_the_trees` | A walk held in its session (the module's held/stalled session double), dropped; `engine.abandoned(run)` (lease released); `settle_stop(run)` → `Ok(true)`; run `stopped`, `finished_at` `None`; the running step `stopped`, done steps untouched; `step_trees` non-empty and the fake isolator recorded **no** `cleanup` (and one more `release`, if it counts them); `refresh_lease(run, owner)` false; item state `Stopped`. |
| `a_stop_records_the_turns_cancelled_done` | Scripted turn with N events; the run's control sends `Cancel { grace }` mid-turn; the walk answers `Err(Cancelled)`; `abandoned`; `settle_stop` → true. `step_events` hold the N events then exactly one `Done(Cancelled)` as the last row (every pre-`Done` event has a row). |
| `a_stop_of_a_run_that_finished_or_was_cancelled_writes_nothing` | Three runs: `done` (walked to its end), `cancelled` (`cancel_run`), already `stopped` (`settle_stop` twice): each `settle_stop` → `Ok(false)`; `run_row` equal before/after; no lease taken (`lease_expires_at` unchanged). |
| `a_stop_while_another_owner_holds_the_lease_writes_nothing` | `store.take_lease(run, BOX, stranger, 1 day)`; `settle_stop` → `Ok(false)`; run still `running`, the stranger's `refresh_lease` true. Plus a run executing on another box → `Ok(false)`. |
| `a_stopped_steps_pending_follow_up_is_refused_session_cancelled` (D12) | Mirror `an_abandoned_walks_follow_up_is_refused_before_the_lease_goes_back` (`:22951`): a follow-up queued on the live step, then `Cancel { grace }`; after `settle_stop` the follow-up row is `refused` with `FOLLOW_UP_SESSION_CANCELLED` and its text nulled. (The dropped path's `FOLLOW_UP_SESSION_ENDED` stays pinned by the existing test.) |
| `a_stop_of_a_run_that_parked_meanwhile_gives_the_lease_back` (F7) | A parked (`awaiting_approval`) run of this owner with a released lease: `settle_stop` → `Ok(false)`; status unchanged; `refresh_lease(run, owner)` false (the lease went back). |

### T2.2 The divided-step matrix (D6; `crates/htui-orch/src/conformance.rs`)

`Orchestrate` (`:86`) gains, after `cancel_walks` (`:263`):
```rust
    /// MOD-125 M2 D3: `Engine::settle_stop`, the executor's settlement after its walk of `run`
    /// was preempted (a case drops or cancels the walk first).
    ///
    /// # Errors
    /// As `Engine::settle_stop`.
    async fn settle_stop(&self, run: RunId) -> Result<bool, EngineError>;
```
and `impl Orchestrate for FakeOrchestrator` (`:272`): `crate::engine::settle_stop_fake(self, run).await`.

Four cases in a new frame `stop_case` (MOD-125 M2), chained in `case` (`:1056-1075`) before
`.unwrap_or_else(|| earlier_case(…))`; four `CASES` entries under `// MOD-125 M2: a stop of a divided step`.
Common shape: `divided_width_one(&orch)` (`:15288`: `api`, `ui`, one session at a time) or
`divided_feat_3`; after the settle assert `orch.settle_stop(run) == Ok(true)`, run `stopped`,
`finished_at` `None`, `orch.isolator().cleanups() == 0`, and a second `settle_stop` → `Ok(false)`.
Assert the held row's `role` before stopping (the planner and integrate sessions share the
`implement` key; the existing cases tell them apart by `planner_of` / `integrate_of`).

| Case | How the walk is stopped | Asserts |
|---|---|---|
| `a_stop_during_the_planner_stops_the_planner` | `hold_sessions(&[("implement", 1)])`; `join!(orch.claim(run), async { hold.live(1); orch.cancel_walks(10 ms) })` → `Err(Cancelled)` (graceful shape) | `planner_of(&steps)` `stopped`, attempt 1; no part row; no plan admitted |
| `a_stop_during_a_part_stops_it_and_leaves_the_queued_part_pending` | as `a_cancel_during_a_part_leaves_the_queued_part_pending` (`:15317`) | `implement/api` `stopped`; `assert_ui_never_pulled` (`ui` `pending`, never prepared, no session); planner `done` |
| `a_stop_during_the_pre_merge_stops_the_integrate_step` | `let stall = orch.isolator().stall_nth_premerge(1); let run = crash_divided(&orch, &[stall]).await;` (the walk sits in an isolator call, so only the token drop reaches it: the forced/escalated shape; the lease stays this owner's) | `integrate_of` `stopped`; both parts `done`; `premerge_requests().len() == 1` |
| `a_stop_during_integrate_stops_the_integrate_step` | `crash_divided(&orch, &[orch.stall_after_done("implement", 1, None, false)])` (integrate `running`, as `a_crash_after_an_unmerged_session_fails_rather_than_lands`, `:16222`, asserts) | `integrate_of` `stopped`; parts `done`; no `cleanup` |

Pins: `conformance.rs:16929` and `tests/fake_conformance.rs:18` `230 → 234`; append "+ 4" to the
sum and ", and MOD-125 M2's four divided-step stops (plan D6) 234" to each message.

### T2.3 The verifier's group (D5; `crates/htui-orch/src/verify.rs`)

In `ShellVerifier::spawn_and_wait` (`:361`):
```rust
        let mut child = match spawn_supervised(build) {
            // MOD-125 M2 D5: a check dropped mid-run (a stop's or a cancel's token) kills its whole
            // group, as `run_shell`'s does (H-18): `kill_on_drop` reaches only the shell.
            Ok(child) => GroupGuard { child, ended: false },
            Err(err) => { /* unchanged */ }
        };
```
Every `child.` use becomes `child.child.` (`stdout()`, `stderr()`, `wait()`, `as_mut()` in the
elapsed arm). The final `match ended { … }` is bound (`let report = match ended { … };`), then
`child.ended = true; report` — the guard fires only when the future is dropped before an ending was
decided; the elapsed arm's `kill_within_grace` already killed the group. Update `GroupGuard`'s doc
(`~:669`): "[`run_shell`]'s and the verifier's child (MOD-125 M2 D5) …".

Test (in `mod shell_tests`, beside `a_dropped_run_shell_kills_its_group`, `~:909`, `#[cfg(unix)]`):
`a_dropped_check_kills_its_process_group` — `PATTERN = "sleep 30.4307"`; a `ShellVerifier::new(&BTreeMap::new(), Arc::new(MinimalScrubber::new([])), Arc::new(SystemClock))`; `verifier.run(VerifyRequest { command: Some(format!("{PATTERN} & {PATTERN}")), cwd: Some(dir), remaining: None, step: StepId::new() })` under `tokio::time::timeout(500 ms)` → `Err` (still running when dropped); then `until_gone(PATTERN, "the check's group outlived the dropped verify")`. Fails on today's tree (the backgrounded `sleep` survives), passes after.

### T2 commits

1. `MOD-125 M2 T2: Engine::settle_stop` — T2.1 + `settle_stop_fake` + the six engine tests.
2. `MOD-125 M2 T2: the divided-step stop matrix` — `Orchestrate::settle_stop`, impl, `stop_case`, four cases, pins `→ 234`.
3. `MOD-125 M2 T2: a dropped check kills its process group` — T2.3 + its test.

Validate: `cargo test -p htui-orch --all-features --no-fail-fast 2>&1 | tee /tmp/orch.log; grep -c SIGABRT /tmp/orch.log` (expect 0; memory: stack headroom — box any new large future you await on the walk's path).

---

## T3 — Runtime: apply stops, refuse walking commands, admission (after T1, T2 merged)

Files: `crates/htui-worker/src/{runtime.rs, worker.rs (no edit expected: `worker.rs:71` already reaches `poll_once`), lib.rs (DV-4)}`,
`crates/htui-worker/tests/auto_queue.rs`, `crates/htui/src/run_worker.rs`, `crates/htui/tests/worker_pg.rs`.

### T3.1 Constants and `preempt_gracefully`'s slack (`runtime.rs`)

Beside `CANCEL_GRACE` (`:119`):
```rust
/// MOD-42 plan D11: how long past its grace a cancel's preempt waits for the walk to end.
const CANCEL_SLACK: Duration = Duration::from_secs(1);

/// MOD-125 M2 D2: how long past its grace a stop's preempt waits before the token drops the
/// walk. After the driver's grace the relay still kills and reaps the agent, settles its
/// permission rows, flushes its recorder, syncs the step and finishes; a drop mid-flush loses rows
/// whose `seq` was taken (`relay.rs:758-760`). The wait ends as soon as the walk does, so a quick
/// stop costs nothing.
pub const STOP_SLACK: Duration = Duration::from_secs(5);
```
`Walks::preempt_gracefully(&self, run, grace, slack: Duration) -> bool` (`:783`): the deadline
becomes `tokio::time::Instant::now() + grace + slack`; doc "wait until none is live or
`grace + slack`". Callers `:2869` and `:3095` pass `CANCEL_SLACK` (behaviour unchanged).
`lib.rs` re-exports `STOP_SLACK` and `workspace_is_stopped` beside `CANCEL_GRACE` (`:52`, DV-4).

### T3.2 The `Stopping` map (D4.1)

`Shared` (`:189`) gains, after `applying`:
```rust
    /// MOD-125 M2 D4: the runs a stop task of this process is settling, with the strongest mode
    /// asked so far. Not on `Walks::Parent`, which a preempt removes. Same or weaker mode: the
    /// poll does nothing; `forced` over `graceful`: the walk is dropped at once (escalation).
    stopping: StdMutex<HashMap<RunId, StopMode>>,
```
(initialised empty wherever `Shared` is built). Helper types:
```rust
/// What a polled stop of a run does (MOD-125 M2 D4).
enum StopTurn { First, Escalate, InFlight }

impl<P: ReplySink> Shared<P> {
    /// MOD-125 M2 D4: records `mode` for `run` and says what the poll does with it.
    fn stop_turn(&self, run: RunId, mode: StopMode) -> StopTurn {
        let mut map = self.stopping.lock().unwrap_or_else(PoisonError::into_inner);
        match map.get(&run).copied() {
            None => { map.insert(run, mode); StopTurn::First }
            Some(StopMode::Graceful) if mode == StopMode::Forced => {
                map.insert(run, StopMode::Forced);
                StopTurn::Escalate
            }
            Some(_) => StopTurn::InFlight,
        }
    }
}

/// Removes `run` from [`Shared::stopping`] however its stop task ends.
struct StopInFlight<P: ReplySink> { shared: Arc<Shared<P>>, run: RunId }
impl<P: ReplySink> Drop for StopInFlight<P> {
    fn drop(&mut self) {
        self.shared.stopping.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.run);
    }
}
```

### T3.3 `poll_once` (`:3161-3195`) and `stop_walk`

After the `for row in rows { … }` loop (collect `let cancelling: HashSet<RunId> = rows.iter().map(|row| row.run_id).collect();` before it):
```rust
    // MOD-125 M2 D4: the stops this process applies, after the cancels. A run with a pending
    // cancel in this listing is left to it (F9): the cancel's task applies first, and the next
    // poll no longer lists a cancelled run.
    let stops = match htui_core::store::WorkerStore::pending_stops(&writer, ctx.shared.owner).await {
        Ok(stops) => stops,
        Err(err) => {
            tracing::warn!(%err, "reading the pending workspace stops failed");
            return;
        }
    };
    for stop in stops {
        if cancelling.contains(&stop.run) {
            continue;
        }
        match ctx.shared.stop_turn(stop.run, stop.mode) {
            StopTurn::InFlight => {}
            // D2: the relay does not re-read its control inside `cancel`, so a forced stop over a
            // graceful one in flight is the token drop, now; the first task then settles.
            StopTurn::Escalate => {
                ctx.shared.walks.preempt(stop.run);
            }
            StopTurn::First => {
                let task = ctx.unaddressed("stop_run");
                let in_flight = StopInFlight { shared: Arc::clone(&ctx.shared), run: stop.run };
                spawn_supervised(task.clone(), async move {
                    stop_walk(task, stop.run, stop.mode, in_flight).await;
                });
            }
        }
    }
```
Update `poll_once`'s doc: "…and every pending workspace stop (`pending_stops(owner)`, MOD-125 M2 D4) runs as [`stop_walk`] on a task of its own."

```rust
/// MOD-125 M2 D4: one run's workspace stop. A live walk of the run is preempted through the
/// cancel seam — `Cancel { grace }`, graceful = the runtime's cancel grace, forced = zero — and its
/// token drops at grace + [`STOP_SLACK`]; the walk publishes its own `PREEMPTED` and runs
/// `abandoned`. Then, under the run's lock (taken directly, not through a walk token, so an
/// escalation's `preempt` reaches only the walk, F10), `Engine::settle_stop`. A run leased here
/// with no live walk goes straight to the settle. Every failure is logged: the stop row stays, so
/// the next poll retries.
async fn stop_walk<H: htui_core::store::WorkerHost, P: ReplySink>(
    ctx: TaskCtx<H, P>,
    run: RunId,
    mode: StopMode,
    _in_flight: StopInFlight<P>,
) {
    let _ = ctx.tag.run.set(run);
    if ctx.shared.walks.is_live(run) {
        let grace = match mode {
            StopMode::Graceful => ctx.shared.cancel_grace,
            StopMode::Forced => Duration::ZERO,
        };
        ctx.shared.walks.preempt_gracefully(run, grace, STOP_SLACK).await;
    }
    let guard = ctx.shared.locks.lock(run).await;
    if ctx.shared.walks.closed() {
        return;
    }
    let kit = match Kit::read(&ctx.shared, &ctx.host, false).await {
        Ok(kit) => kit,
        Err(message) => {
            tracing::warn!(%run, %message, "a stop could not build its engine; the next poll retries");
            return;
        }
    };
    let driver = |candidate: &SnapshotCandidate, _key: &SessionKey<'_>| kit.driver(candidate);
    let engine = kit.engine(&driver);
    match engine.settle_stop(run).await {
        Ok(true) => tracing::info!(%run, %mode, "run {run} stopped ({mode})"),
        Ok(false) => tracing::debug!(%run, "the stop found the run no longer running under this process"),
        Err(err) => tracing::warn!(%run, %err, "settling a stop failed; the next poll retries"),
    }
    drop(guard);
}
```
(`Box::pin` the `settle_stop` future if clippy's `large_futures` or the stack headroom asks.)

### T3.4 D7 — walking commands refused in a stopped workspace

Helper (runtime.rs):
```rust
/// MOD-125 M2 D7: the refusal of a walking command on `row`, when its workspace is stopped. A
/// store failure refuses too (fail closed), in its own words.
async fn stopped_workspace_refusal<H: htui_core::store::WorkerHost>(
    host: &H,
    writer: &H::Store,
    row: Option<&Run>,
) -> Option<String> {
    let workspace = row?.workspace_id?;
    match htui_core::store::WorkerStore::workspace_stop(writer, workspace).await {
        Ok(None) => None,
        Ok(Some(_)) => {
            let slug = host.workspaces().await.ok().and_then(|all| {
                all.into_iter().find(|ws| ws.workspace_id == workspace).map(|ws| ws.slug)
            });
            Some(workspace_is_stopped(slug.as_deref().unwrap_or(&workspace.to_string())))
        }
        Err(err) => Some(err.to_string()),
    }
}
```
- `on_run_unless_claimed` (`:2847`), right after `let row = ctx.tag_run(&kit.writer, run).await.or(early);`:
  ```rust
  if matches!(
      command,
      Command::AnswerGate { .. } | Command::RetryStep { .. } | Command::SelectFanout { .. } | Command::AcceptArtifact { .. }
          | Command::PromoteStep { .. }
  ) && let Some(refusal) = stopped_workspace_refusal(&ctx.host, &kit.writer, row.as_ref()).await
  {
      ctx.refuse(refusal);
      return None;
  }
  ```
  (`row` is moved later into the `Promoted` arm; borrow it here.) `StartRun`, `CancelRun`,
  `Cleanup`, `CloseOut` are not refused. `PromoteStep` IS refused (Q1, maintainer 2026-11-12); T3.6 adds
  it to `walking_commands_are_refused_in_a_stopped_workspace`.
- `unblock()` (`:3205-3250`), after the `unblock_case` re-check under the lock and before `walked(…)`:
  if `case` is `UnblockCase::Resume(_)`, read `ctx.tag_run(&kit.writer, run).await` and refuse with
  `stopped_workspace_refusal` the same way.

### T3.5 D8 — `admit` (`:2174-2368`)

After `workspace_of` is built (`~:2290`) and before the per-item loop:
```rust
    // MOD-125 M2 D8: one stop read per distinct workspace of the ordered entries. A stopped
    // workspace's entries stay queued and untouched, and do not count toward the `Stalled` drain:
    // a stop never closes a batch.
    let mut stopped: HashSet<WorkspaceId> = HashSet::new();
    if let Some(writer) = host.writer() {
        let mut seen: HashSet<WorkspaceId> = HashSet::new();
        for workspace in order.iter().filter_map(|item| workspace_of.get(item).copied().flatten()) {
            if !seen.insert(workspace) { continue; }
            match htui_core::store::WorkerStore::workspace_stop(&writer, workspace).await {
                Ok(Some(_)) => { stopped.insert(workspace); }
                Ok(None) => {}
                Err(err) => {
                    shared.queue_read_failed("reading a workspace's stop", &err);
                    return;
                }
            }
        }
    }
    let mut held = 0_usize;
```
(No writer: `Kit::read` below fails as today.) In the loop, first statement:
```rust
        if workspace_of.get(&item).copied().flatten().is_some_and(|ws| stopped.contains(&ws)) {
            held += 1;
            tracing::debug!(batch = %batch.id, %item, "the entry's workspace is stopped; it stays queued");
            continue;
        }
```
and the stall check becomes
`if admissible.is_empty() { if held > 0 { return; } drain(ctx, &batch, Drain::Stalled, &seen).await; return; }`.
Doc of `admit`: add "MOD-125 M2 D8: an entry of a stopped workspace is skipped, and a batch whose
only admissible-looking entries are such is not drained."

### T3.6 Tests (first)

**`crates/htui/src/run_worker.rs`** (test module; `Fixture`, `Park`/`Graced` `:486-552`, `Worker::spawn`,
`polled` `~:3560`, `stranded`, `requested`, `rests_at`, `commands_of`, `assert_answered_cancelled_once`).
New helper `start_run_in(item, workspace) -> StoreRequest` (`start_run` `:817` with `workspace: Some(workspace)`),
and a new double `Play::Deaf(Deaf)`: a session whose `next_event` never resolves and whose `cancel`
records each grace it receives (`Arc<StdMutex<Vec<Duration>>>`) and returns `Ok(())` without ending the
turn, so only the token drop ends its walk. All runs below start in `ids::WORKSPACE_PLATFORM`.

| Test | Setup → asserts |
|---|---|
| `the_command_poll_stops_a_live_walk_gracefully` | `Play::Park`, `with_cancel_grace(3 s)`, `Worker::spawn`; `start_run_in(HTUI_ANA_2, PLATFORM)`; `a_pending_request`; `store.stop_workspace(PLATFORM, Graceful, USER, now)`. `rests_at(run, Stopped)`; the start's reply is `Failed { message == PREEMPTED }`; `assert_answered_cancelled_once`; `park.grace() == Some(3 s)`; the parked step `stopped`; `finished_at` `None`; `commands_of(run)` empty (no cancel row); item state `Stopped`. |
| `a_forced_stop_cancels_with_zero_grace` | as above with `Forced` → `park.grace() == Some(Duration::ZERO)`; run and step `stopped`. |
| `a_forced_poll_escalates_a_graceful_stop` | `Play::Deaf`, `with_cancel_grace(60 s)`; graceful stop; wait until `deaf.graces() == [60 s]`; then `stop_workspace(PLATFORM, Forced)`; `rests_at(run, Stopped)` within `PATIENCE` (20 s ≪ 65 s); no assertion on a `Done(Cancelled)` row (D2). |
| `a_second_poll_does_not_restop` | `Play::Deaf`, `with_cancel_grace(60 s)`, graceful stop; after `deaf.graces().len() == 1`, sleep `2 × COMMAND_POLL + 200 ms`; still exactly one cancel received; then escalate as above to end the walk. |
| `a_pending_cancel_beats_a_stop` | `stranded` + `take_lease(owner)` + `requested(cancel)` + `stop_workspace(PLATFORM, Graceful)`; `polled()` → run `Cancelled`, the command `Applied`. |
| `a_leased_run_with_no_walk_is_settled_directly` | `stranded` + `take_lease(owner, 1 day)` + `stop_workspace(Forced)`; `polled()` → run `stopped`, `refresh_lease(run, owner)` false. |
| `the_store_loop_applies_a_workspace_stop_on_its_own` | as the previous with `Worker::spawn` and no `polled()` (twin of `the_store_loop_applies_a_pending_cancel_on_its_own`, `~:3757`); `rests_at(run, Stopped)`. |
| `walking_commands_are_refused_in_a_stopped_workspace` | A run of `HTUI_ANA_2` parked at `research`'s gate (`start_run_in` → `AwaitingApproval`, as `an_orch_request_is_served_off_the_loop`); `stop_workspace(PLATFORM)`. `AnswerGate`, `RetryStep`, `SelectFanout`, `AcceptArtifact` on it each answer `Failed { message == workspace_is_stopped("platform") }` and the run row is unchanged. `Unblock`: seed the resume case as the existing `Unblock` resume test does (search `UnblockCase::Resume` in this module) → the same refusal. |
| `a_cancel_is_not_refused_in_a_stopped_workspace` | the parked run above; `cancel(run)` → `CommandOutcome::Cancelled`. |

**`crates/htui-worker/tests/auto_queue.rs`** — `admit_skips_a_stopped_workspaces_entries_without_draining_the_batch`:
queue item X with `Some(PLATFORM)` and item Y with `Some(W2)` (fresh workspace), open the batch, stop
PLATFORM, sweep: Y admitted (a run with `workspace_id == Some(W2)`), X has no run, and the batch is
still open; with Y's run finished and only X left, sweep again: still open (no `Stalled` drain), X
still not admitted; `resume_workspace(PLATFORM)`, sweep: X admitted. Use the file's own queue/batch/
sweep helpers and `Held` (`:290-322`) to keep Y's walk from finishing until the assertion wants it.

**`crates/htui/tests/worker_pg.rs`** (real time, `#[tokio::test(flavor = "multi_thread")]`):
```rust
/// MOD-125 M2: a stop ends a live worker walk within the grace, the slack, one poll and a margin.
const STOP_BOUND: Duration =
    Duration::from_secs(CANCEL_GRACE.as_secs() + STOP_SLACK.as_secs() + COMMAND_POLL.as_secs() + 2);
```
New `Stack::start_in(item, workspace)` (as `start`, `:398`, with `workspace: Some(workspace)`).
- `a_stop_from_another_box_settles_a_live_worker_walk_stopped`: as
  `a_cancel_from_another_box_ends_a_live_worker_walk` (`:967`) with `stack.start_in(HTUI_FEAT_3, PLATFORM)`;
  the other box calls `client.stop_workspace(PLATFORM, Graceful, client.this_user(), now)`; the run
  reaches `stopped` within `STOP_BOUND`; the parked request `cancelled` and its I-7 answer recorded
  once; the step `stopped`; no `run_command` row; `stack.released(run)`.
- `a_forced_stop_from_another_box_settles_a_live_worker_walk_stopped`: the same with `Forced`.

### T3 commits

1. `MOD-125 M2 T3: STOP_SLACK and preempt_gracefully's slack` (T3.1 incl. `lib.rs`; no behaviour change).
2. `MOD-125 M2 T3: the command poll applies workspace stops` (T3.2, T3.3, the run_worker stop tests + `Deaf`, the two worker_pg tests).
3. `MOD-125 M2 T3: walking commands are refused in a stopped workspace` (T3.4, `workspace_is_stopped`, its two run_worker tests).
4. `MOD-125 M2 T3: admit skips a stopped workspace's entries` (T3.5, the auto_queue test).

Validate: `cargo test -p htui-worker --all-features --no-fail-fast`;
`cargo test -p htui --all-features -- --test-threads=1` (scheduling-dependent suite; never wait on a
`pgrep -f "cargo test…"` loop, it matches itself).

---

## T4 — Minimum version (D11 (a): every crate `0.1.0 → 0.2.0`)

Files: `version = "0.1.0"` → `"0.2.0"` on line 3 of each of the nine members:
`crates/{htui,htui-agent,htui-bench,htui-core,htui-mcp,htui-orch,htui-secrets,htui-store,htui-worker}/Cargo.toml`
(path dependencies carry no version, so nothing else moves). Then `cargo update --workspace` (alias
`-w`), which rewrites only the nine workspace entries of `Cargo.lock`; never `--locked` until then.
`spikes/ui-skeleton` is not a member and is untouched. No test, snapshot or doc compares a literal
`0.1.0` with the build (F17): the seven `box_settings__*.snap` render the demo box's fixture value,
`box_probe.rs:103,407`, `provision/verify.rs:199`, `boxes.rs:1407`, `state_cmd.rs:1599` and
`channel.rs:646` are passed-in values; `cargo.test.ts` needs no edit.

Commit: `MOD-125 M2 T4: every crate 0.2.0, raising the minimum version (D11 a)`.

Validate: `cargo test -p htui-store --all-features --test migrations --test read_only_pg -- --test-threads=1`;
`cargo test -p htui --all-features --bin htui cli`; `cargo test -p htui --all-features --lib claude_emit`;
`cargo test -p htui-mcp --all-features`.

---

## T5 — Docs (after T3)

- `docs/ANA-2.md`: under the run table (after the `stopped | resume impossible …` row, `~:712`) and
  under the step table (after `stopped | run failed …`, `~:764`), one paragraph each:
  > *Amended by MOD-125 M2, 2026-11-12 (plan D1–D4, `.claude/plans/mod-125-m2-stop-applied.plan.md`):*
  > the "workspace stop applied" row is written by the process that leases the run. Its command
  > poll (every `COMMAND_POLL`, TUI and worker alike) lists `pending_stops(owner)` — its `running`
  > runs whose workspace has a `workspace_stop` row — interrupts the walk through the cancel seam
  > (graceful: `Cancel { CANCEL_GRACE }`; forced: `Cancel { 0 }`; the token drops at grace +
  > `STOP_SLACK` = 5 s, at once on an escalation), then retakes the lease and calls `stop_run`
  > (`Engine::settle_stop`). Trees, branch and the session banner stay; no `finished_at`. A pending
  > cancel of the run wins; a run whose lease lapsed is the sweep's row below.
  In the step table's paragraph, add: "`pending` parts of a divided step stay `pending` (D6)."
  Change both rows' who-column from `orchestrator (M2)` to `orchestrator (executor's command poll, M2)`.
- `.claude/prds/mod-125-stop-and-resume-runs.prd.md`: `:132` — append "**Cancel half settled by M2
  D2** (graceful = `Cancel{CANCEL_GRACE}`, forced = `Cancel{0}` + token drop at `STOP_SLACK`); the
  `--resume` redo stays M3's." and leave the box unticked (DV-5). `:135` — append "M2 landed M-3
  (D9), L-3 (D10), L-7's `admit` half (D8) and L-5 (D11 a); L-7's `select_fanout` half (M3) and L-6
  remain."
- `CONCEPTS.md` unchanged (`:67` still holds).

Commit: `docs(MOD-125): ANA-2 and the PRD record M2's executor stop`. Validate:
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## 5. Validation (gate on the merged tree, `--test-threads=1`)

```bash
# sqlx data after each T1 commit group that changes a query (pending_stops, queued_runs_on_box, D9):
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx_mod125m2"
DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod125m2 cargo sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx_mod125m2 cargo sqlx prepare -- --all-targets --all-features)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
bash scripts/gate.sh 2>&1 | tee /tmp/gate.log; grep -c SIGABRT /tmp/gate.log
cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=1
cargo sqlx prepare --check --workspace     # with DATABASE_URL on the scratch DB
cargo insta test --workspace --all-features --check
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE htui_sqlx_mod125m2"
```
`HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres` drives the Pg suites.

Independence (plan table, re-checked): T1 = core store/model + htui-store + agent spies +
`htui/tests/{hierarchy,hierarchy_pg}.rs`; T2 = `htui-orch/src/{engine,conformance,verify}.rs`,
`tests/fake_conformance.rs`; T4 = nine `Cargo.toml` + `Cargo.lock`. Disjoint. T4's `Cargo.lock`
is the only shared-looking file and only T4 writes it; T1/T2 add no dependency.

---

## 6. Open risks (no plan decision involved)

| # | Risk | Mitigation |
|---|---|---|
| R1 | A stop row inserted after `delete_workspace`'s snapshot (DV-2 counts by snapshot): the cascade deletes it, the runs keep running unscoped. | The stop is lost with its workspace, as the user deleted it; no orphan. |
| R2 | `pending_stops` read before a resume, settled after: the run is stopped although its workspace is no longer. | M3's resume handles a `stopped` run of a resumed workspace; window ≤ one settle. |
| R3 | A parked (`awaiting_approval`) run in a stopped workspace stays parked; D7 refuses answering its gate until resume. | As M1 D1. |
| R4 | `stop_walk` waits on the run lock without a token: a lock holder that never ends blocks it. | Every holder is a walk the preempt just dropped, or a short command; runtime shutdown cancels them. |
| R5 | The verifier's group kill also kills a backgrounded process a check started on purpose. | Same contract as `run_shell` (H-18) and as a cancel already implies. |

---

## 7. Deviations needing a maintainer call

**Maintainer calls, 2026-11-12:** DV-1 no extra lock (recommended); DV-2 refuse while stopping
(recommended); DV-3, DV-4, DV-5 as recommended; Q1 refuse `PromoteStep` (recommended). Every DV row's
Recommendation column is the binding text.

| DV | Plan says | Tree / problem | Recommendation | If overruled |
|---|---|---|---|---|
| **DV-1** | D9: Pg locks the `workspace_stop` row `FOR UPDATE` first. | F1: inverts M1's run → `workspace_stop` order against the delete's own `SET NULL` cascade; `40P01` vs `claim_run`/`adopt_runs`, not retried. | No explicit lock: the count runs in the existing REPEATABLE READ attempt; a run changed after the snapshot conflicts with the cascade, the `40001` retries, the recount refuses. Pinned by `delete_workspace_recounts_a_run_stopped_under_it`. | Add the `FOR UPDATE` (T1.4 names the statement) and accept the deadlock risk. |
| **DV-2** | D9 counts `stopped` member runs. | F2: a delete during the stop's grace/slack window passes and leaves the in-flight runs to be settled `stopped` with no workspace: M-3's orphan through another door. | Also count member runs `running` while the workspace has a stop row ("stopping"), same sentence. | Drop the `OR (…)` arm in Pg, the `stopping` arm in Mem, and B in the conformance case. |
| **DV-3** | T1's files: `crates/htui/tests/hierarchy.rs`. | F3: the demo's `stopped` `RUN_8` makes `graphics` undeletable; `hierarchy_pg.rs::a_workspace_delete_rebuilds_the_mirror_its_scope_row_was_in` breaks too. | Add `crates/htui/tests/hierarchy_pg.rs` to T1; both tests cancel `RUN_8` (`Stopped → Cancelled`) first. | Change the fixture's `RUN_8` workspace to `None` instead (moves the fixture, `pg/demo.rs` round-trips, and whichever snapshot shows it: a wider change). |
| **DV-4** | T1/T3 file sets. | F4: re-exports live in `htui-core/src/model/mod.rs`, `htui-core/src/store/mod.rs`, `htui-worker/src/lib.rs`. | Add them to T1 (two) and T3 (one); re-exports only. | — (the code does not compile without them). |
| **DV-5** | T5: the PRD's "mapping onto ACP and the CLI adapter" question is ticked. | F16: its `--resume` half is M3's. | Annotate the cancel half as settled, leave the box open for M3. | Tick it and move the `--resume` half into M3's plan. |

---

## 8. Questions for the maintainer

- **Q1 (F18)**: `PromoteStep` on a run of a stopping workspace parks the run and opens a live chat,
  escaping the stop. The plan's D7 set (= `Kit::refusal`'s walking set) leaves it allowed. Refuse it
  too (one more arm in T3.4's `matches!`)? Recommended: yes, but it is outside the plan, so it is
  not implemented unless you say so.
- **DV-2** above is the one with product meaning (a stopping workspace cannot be deleted for ~7 s).
- **Operational (D11 a, F17)**: after T4 the installed MCP relay and the emitted Claude plugin from
  the 0.1.0 build read as stale: reinstall the relay and run `htui claude emit --force` on each box.
