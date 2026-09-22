# Blueprint: MOD-4 milestone 3 — work happens in a real tree

**Plan**: `.claude/plans/mod-4-orch-tree.plan.md` (APPROVED; D22–D47 settled; OQ-1 resolved to the `git` CLI by the maintainer on 2026-09-22, OQ-2 to OQ-5 at their defaults). **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md` D1–D8 win on conflict. **Design authority**: `docs/ANA-2.md` §2 invariants 4, 6, 7 and 9, §4.2 (verification, `:491-517`), §4.6 (`:885-1007`), §4.7 (what this milestone leaves alone, `:1009-1158`), §4.9 (what milestone 5 reads out of this milestone's rows, `:1298-1300`), §8 (`:1667-1670`, `:1777` as amended), §10.7 row 7 (`:2055` as amended), §12 criteria 11, 12 and 13 (`:2115-2121`).

**Verified at**: HEAD `b807f65` on branch `mod-4-m2-close-out`. Every line cited below was re-read from that tree on 2026-09-22; the plan's own ledger was taken at `e66455e` and the four commits since touch only `.claude/plans/**`, `docs/ANA-2.md` and `crates/htui-orch/src/gate.rs` (the verdict parameter, `1cbddc5`), so its `engine.rs` and store citations still hold. **Line numbers are pre-edit**: T1 inserts into `run.rs`, `traits.rs`, `mem.rs`, `pg/write.rs`, `writer.rs` and the two spies; T2 inserts into `isolate.rs` and `fake.rs`; T6 edits `engine.rs`, `gate.rs`, `command.rs`, `fake.rs` and `conformance.rs`. A citation into a file a task edits moves after that task's first commit.

**Scope**: T1 ∥ T2, then T3 ∥ T4, then T5, then T6. Six tasks; one new dependency (`gix` 0.87.1) and four crates promoted from workspace to `htui-orch` (`walkdir`, `process-wrap`, `which`, `dirs`, plus `windows` under `cfg(windows)`); three new modules under `isolate/` and one at the crate root; two seam methods on three `WriteStore` implementations plus two spies; one `.sqlx` regeneration; three of ANA-2's criteria over a real repository. No migration, no `.snap`, no `crates/htui/**`.

**House style**: one named free function per refusal sentence (`traits.rs:1053-1160`), `IsolateError::Refused`'s payload is the sentence and `Display` prepends `isolation refused: ` (`isolate.rs:44`); every instant a writer stamps comes from the caller and is `trunc_subsecs(TIMESTAMPTZ_DIGITS)`-truncated (`model/run.rs:272`); `MemStore` writes are one non-async `write` closure (`mem.rs:657-660`); Postgres transactions check the batch whole before the first statement (`pg/write.rs:3170-3185`); doc comments cite the ANA line or the `gix-0.87.1/src/...` line or the `git worktree --help` synopsis they implement; `#![warn(missing_docs)]`; `[lints] workspace = true`; **the only `Command::new` under `crates/htui-orch/src/` is `isolate/git.rs`'s `Cli` and `verify.rs`'s shell** (today the grep is empty).

---

## 0. Flags

Plan-vs-tree discrepancies and architect additions. Each has a resolution the implementer follows; the **A-** rows are surfaced to the maintainer before dispatch and are the only places this blueprint departs from the plan's letter.

| # | Plan says | Tree at HEAD | Resolution |
|---|---|---|---|
| F-A | T1: "`Writer`/`BufferedWriter`; both `BufferedWriter` arms refuse with the one sentence"; Files table lists `crates/htui-store/tests/writer_buffered.rs` | `BufferedWriter` and `writer_buffered.rs` were deleted by CLEAN-2 (`92f3c48`); `Writer` is `Memory(MemStore) \| Online(PgStore)` (`writer.rs:57-62`) and every arm is a two-way `match` (`:788-830`) | T1 adds two two-arm delegations to `Writer` and no refusal anywhere. The "refusing `BufferedWriter` arm" sentence in the plan's Summary, T1's Action and its Files table is stale; nothing is owed. |
| F-B | Files table (`:272`): `git.rs` exports `prune_worktrees`; D23 and the Risks row still describe prune "as a backstop" | D46 (`:191`) says `git worktree prune` is **never** run and the mode table's cleanup cell (`:208`) agrees | No `prune_worktrees` function exists. Cleanup is `remove --force --force` per tree, then one `worktrees()` read; a surviving entry under the scratch root is **reported** in the cleanup error (§6.5). D46 wins over D23's prune paragraph. |
| F-C | D35 and T2's Action: `checkout_tree_over`, "D35's full-index reset only", the `worktree-mutation` feature; the Risks row on the populated-directory checkout | D47 (`:192`) makes `git reset --hard` the fifth shelled verb and deletes the composition | T2 lands `reset_hard(repo)` and **no** `checkout_tree_over`; `gix::worktree::state` is never named. The feature list is recomputed in §3.1 (F-D). The Risks row is retired with it. |
| F-D | D22: "six features"; Acceptance: "exactly D22's six-feature set" | After D47 nothing names `worktree-mutation` | **Five**: `["sha1", "max-performance-safe", "parallel", "index", "status"]` — §3.1 names the call that needs each and what `cargo tree` must and must not show (C-6). |
| F-E | D43 and the mode table: the `shared_serialized` mutex is "held from `prepare` to `cleanup`"; T5's test says a second `prepare` "does not resolve until the first `cleanup`" | Every phase of a run resolves to the **same** isolation unless a phase overrides it (`graph.rs`'s chain ends at `ProjectSettings.default_isolation`, `kind.rs:256`), so a four-phase `shared_serialized` run has step 2's `prepare` waiting on a guard step 1 holds until run end — the walk deadlocks against itself on `MemStore::demo()` the moment a project sets the mode | **A-1.** The guard is held from `prepare` to **`capture` of the same step** — ANA-2 `:916`'s "held for the whole step", read literally — and `cleanup(run)` releases any guard still held by a step of that run (the crash path, where `capture` never ran). A second `prepare` on the same `(box, repo)` therefore resolves at the first step's `capture`, and T5's test is worded that way (§6.6). Nothing changes for milestone 4's siblings: their serialisation is exactly prepare-to-capture. |
| F-F | D38: an idempotent `prepare` for `worktree` **and `copy`** "reports `before_hash` = the target of `refs/heads/htui/<step_id>`" | The mode table gives `copy` no branch: its tree is "a filesystem copy … reset to `HEAD`", `before_hash` "source's `HEAD`". A reused copy whose agent has committed has a moved `HEAD` and no other durable record of its base until `upsert_step_tree` ran — which is exactly the window D38 exists for | **A-2.** `copy` labels `htui/<step_id>` **inside the copy** at `before_hash`, written with `gix` right after `reset_hard`. The copy is scratch under the box root, so `R-ID-4` is untouched; the label is what a reused copy and OQ-5's `copy_range` both read as the base. D26's "`local` creates no branch at all" is unchanged (the managed tree is not scratch). |
| F-G | D25: a reconcile refusal "parks the **run** (step stays `done`) through `park_run`" | No §6.2 verb unparks a run whose every step is `done`: `AnswerGate` needs a step at `awaiting_approval` (`command.rs:241-247`), `RetryStep` needs `awaiting_approval \| failed` (`:274-283`), and `Engine::resume` returns `resting` for an `AwaitingApproval` run (`engine.rs:541-546`) | Implemented as the plan says; the dead end is **carried as R-7** (§11): the run is readable (`item_note` names the refusal and the `before_hash`), nothing is lost, and milestone 5's sweep or milestone 6's `Unblock`-shaped verb is where a retry of `reconcile` belongs. `park_run`'s doc says so. |
| F-H | T4 Mirror: "`crates/htui-store/src/cache/**`'s use of `walkdir`" | The one `walkdir` consumer is `crates/htui-store/src/vector_sync.rs` (`grep -rln walkdir crates/*/src`) | Mirror that file's bounded walk. |
| F-I | T2: `CreationFlags(CREATE_NO_WINDOW)` "exactly as `launch.rs:1110-1160` does" | `launch::CREATE_NO_WINDOW` is `pub(crate)` (`launch.rs:54`) | `git.rs` and `verify.rs` share one `pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;` in `isolate/git.rs` with the same doc sentence. H-16. |
| F-J | The implementer-prompt paragraph (`:321-324`) reads "… `merge --abort` and `reset --hard` and `merge --abort`" | A duplicated clause | Five argv shapes: `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort`, `reset --hard`. The prompt is corrected when dispatched. |
| F-K | T6: "the fake's cleanup counter is 1" | `FakeIsolator::cleanup` calls `tick()`, the **hash** counter, and exposes no cleanup count (`fake.rs:207-215`) | T6 adds `cleanups: Mutex<u32>` and `pub fn cleanups(&self) -> u32` beside `prepares` (§7.3), and `cleanup` stops touching `tick()`. |
| F-L | Stage 5: "runs verify after the session and before the settle" | The plan never says what happens when the session produced no `Done` — `pump` returned `Err` or the driver refused to start | **A-3.** `verify_command` runs **only when `result.is_ok()`**. A crashed session is already `Failed` by settle's first rule (`gate.rs:236-240`); spending a `cargo test` on a tree the agent never finished editing records nothing a human wants and delays the failure. `verify_outcome` stays `None` and no `command_run` row is written on that path; the `command_run` row's absence is distinguishable from `unavailable` by `run_step.verify_outcome IS NULL`. |
| F-M | The mode table's refusal column names no sentence for a repo with no commit | `head_id()` on an unborn `HEAD` is an error (`gix-0.87.1/src/repository/reference.rs:211`, `head_id::Error::Unborn`), and `gix::init` produces exactly that repo | **A-4.** One more refusal, any mode: `unborn HEAD: <name> has no commit to record as before_hash`. ANA-2 `:962-963` makes `before_hash` `NOT NULL`, so there is no row to write. |
| F-N | T1 Files table: `pg/rows.rs` gains "the `CommandRun` row, fields appended (M1 D10)" | `rows.rs` holds only results "whose shape is not a table row" (`rows.rs:1-25`); `RunStepTree` and `RunStepCommit` decode straight into the model through `query_as!` with column overrides (`pg/read.rs:709-745`) | `command_runs` is `query_as!(CommandRun, …)` with `"status: CommandRunStatus"`; `rows.rs` is not touched. |
| F-O | Plan: the ANA-2 amendment at `:1777` and `:2055` "is the main thread's to write" | Both lines already carry the 2026-09-22 amendment at HEAD (`b807f65`) | Nothing owed. `git.rs`'s module doc cites `docs/ANA-2.md:1777` as amended. |
| F-P | D41: `FakeVerifier` and "the harness field" | `Orchestrate` (`conformance.rs:52-90`) exposes `isolator()` and `clock()`; a case that scripts a verify outcome needs the verifier the same way | `Orchestrate` gains `fn verifier(&self) -> &FakeVerifier`, the milestone-2 precedent for out-of-list accessors (`conformance.rs:66-84`). |
| F-Q | The mode table: `worktree` cleanup is `remove --force --force` per tree; `copy` is "delete the directory" | D28's `<root>/<run_id>/<step_id>/` common parent is created by `prepare` and named by nobody's cleanup | `cleanup` ends with `remove_dir_all(<root>/<run_id>)` after the per-tree verbs; a `NotFound` is not an error. |
| F-R | T6 Action: "`EngineParts.verifier: &'a V` (one more generic, defaulted nowhere)" | `EngineParts` has six type parameters and `dispatch_fake`/`fake_parts` name every one (`engine.rs:169-206`, `:1490-1500`) | `V: Verifier + ?Sized` is the seventh; `fake_parts` names `crate::fake::FakeVerifier`; milestone 6 names `ShellVerifier`. H-14. |
| F-S | Root manifest comment: `similar = "3.2.0"   # milestone 3 diff synthesis; no crate consumes it yet (A3)` (`Cargo.toml:65`) | `htui-agent` consumes it (`crates/htui-agent/Cargo.toml:35`); OQ-4 moved the diff renderer to milestone 4 | T2's first commit owns `Cargo.toml` and rewrites the comment: `# ACP edit_proposal.diff (htui-agent); milestone 4's previous_diff renderer (MOD-4 M3 plan OQ-4)`. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum) | Validation |
|---|---|---|---|
| T1 `command_run` seam + `isolation_path` | htui-core, htui-store, htui-agent (spies) | 3: (a) types, trait, `MemStore`, `Writer`, spies, case, pins, `PENDING`; (b) Pg writers, `.sqlx`, `pg_criteria` twin, `PENDING` removed; (c) `isolation_path` on both stores, case extension, `.sqlx` | `cargo test -p htui-core --all-features && cargo test -p htui-agent --all-features && USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features -- --test-threads=1 && cargo clippy -p htui-core -p htui-store -p htui-agent --all-targets --all-features -- -D warnings && (cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)` |
| T2 manifests + `isolate/git.rs` | root manifest, htui-orch | 4: (a) manifests, `pub mod git` stub, `Prepared.extra_dirs`, trait doc, `similar` comment, C-6; (b) `Cli`; (c) the `gix` half; (d) the five verbs, `with_retry`, test support | `cargo test -p htui-orch --all-features && cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`, then C-6 and C-7 once |
| T3 `verify.rs` | htui-orch | 2: (a) types, `Verifier`, `ShellVerifier::run` over `sh -c`; (b) semaphore, deadline remainder, tail cap, scrub | as T2 |
| T4 `isolate/copy.rs` | htui-orch | 2: (a) `DEFAULT_COPY_EXCLUDE`, `Exclude`, `measure`; (b) `copy_tree`, the two `.git` refusals | as T2 |
| T5 `isolate/real.rs` | htui-orch | 4: (a) config, validation, `local` + `shared_serialized`, the lock; (b) `worktree` prepare/capture/cleanup; (c) `copy`; (d) `reconcile`, exports, the trait-doc correction | as T2 |
| T6 engine, `CancelRun`, criteria | htui-orch | 4: (a) `Verifier` into the walk, `FakeVerifier`, two cases, pins 17; (b) `extra_dirs`, `cleanup_run`, `reconcile_done_step`, `park_run`; (c) `CancelRun`, third case, pins 18, `cleanups`; (d) `tests/gix_isolator.rs` | as T2, then the plan's full workspace gate (`:514-523`) including `cargo tree -p htui-orch -i gix --edges features` and C-6 to C-9 |

Tests first in every task; the first failing test is named in each section. Every git-backed test opens with the skip of §3.6 and **passes** on a box without `git` ≥ 2.33.0.

---

## 2. T1 — `command_run`'s writer and reader, `isolation_path`

### 2.1 Model (`crates/htui-core/src/model/run.rs`)

After `RunStepCommit` (`:572-583`):

```rust
str_enum!(
    /// `command_run.status` (`0001_init.sql:544`): the `R-MCP-3` queue's own vocabulary. This
    /// milestone writes `done` for a verify that ran and `failed` for one that could not
    /// (ANA-2 §4.2's `unavailable`, `:515`); `queued`, `running` and `cancelled` are MOD-11's.
    CommandRunStatus {
        Queued => "queued", Running => "running", Done => "done",
        Failed => "failed", Cancelled => "cancelled",
    }
);

/// A row of `command_run` (`0001_init.sql:537-551`), field for field (plan D31, OQ-2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRun {
    pub id: CommandRunId,
    pub run_step_id: StepId,
    pub box_id: BoxId,
    /// `'verify'` for every row this milestone writes (ANA-2 `:501`).
    pub class: String,
    pub command: String,
    /// The tree the command ran in; the session `cwd` when there was no primary tree (§7.2).
    pub cwd: String,
    pub status: CommandRunStatus,
    pub exit_code: Option<i32>,
    /// Scrubbed before it reaches a store (`0001_init.sql:546`, `R-SEC-3`).
    pub output: Option<String>,
    pub queued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Arguments of [`crate::store::WriteStore::record_command_run`]: the same twelve fields, every
/// instant the caller's (plan D8). `NewX`/`X` is the seam's shape (`NewRunStep`/`RunStep`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewCommandRun { /* the twelve fields above */ }
```

`model/mod.rs:130-134`: add `CommandRun, CommandRunStatus, NewCommandRun` to the `run::` re-export.

### 2.2 Trait (`crates/htui-core/src/store/traits.rs`)

Insert after `record_commits` (`:857`) and before `write_document`, inside the ANA-2 §8 block:

```rust
/// Records one `command_run` row (ANA-2 §4.2 `:501-506`; plan D31): this milestone's
/// `verify_command` runs, later MOD-11's queue. Every column is the caller's, `queued_at`
/// included, so the row is the durable home of `verify_failure`'s input (`docs/ANA-5.md:335`).
///
/// # Errors
/// [`StoreError::NotFound`] `{ entity: "run_step" }`;
/// [`StoreError::Constraint`] with [`references_no_row`] for an unknown `box_id` and with
/// [`already_exists`] for a duplicate `id`.
async fn record_command_run(&self, new: NewCommandRun) -> Result<CommandRun>;

/// A step's `command_run` rows in `(queued_at, id)` order. On `WriteStore` beside its writer by
/// milestone 1's `repos`/`phases` precedent (`:489`): the table is unmirrored
/// (`MIRRORED_TABLES` does not list it), so a `ReadStore` placement would be unreachable from
/// the conformance suite.
///
/// # Errors
/// The backend's own failures only; an unknown step reads as empty.
async fn command_runs(&self, step: StepId) -> Result<Vec<CommandRun>>;
```

No new refusal function: both sentences exist (`references_no_row` `:1098`, `already_exists` `:1104`). `WriteStore` 61 → 63; the M2 banner (`:670-675`) counts transactions, not methods, and is unchanged — neither writer is composite.

`upsert_step_tree`'s doc (`:841-848`) gains one paragraph: *Also writes `run_step.isolation_path` (ANA-2 `:903`, plan D33): the `path` of the batch's row whose repo `is_primary`, else of its lowest `repo_id` — the order `step_trees` answers in — and leaves the column alone for an empty batch.*

### 2.3 `MemStore` (`crates/htui-core/src/store/mem.rs`)

- `State.command_runs: Vec<CommandRun>` after `step_commits` (`:131`), initialised `Vec::new()` beside `:217-218`.
- `State::record_command_run(&mut self, new: NewCommandRun) -> Result<CommandRun>`: `require_step(new.run_step_id)?` (`:2954`) → `boxes.contains_key(&new.box_id)` else `Constraint(references_no_row("command_run.box_id", new.box_id, "box"))` → duplicate id → `Constraint(already_exists("command_run", new.id))` → push, return the row.
- `State::command_runs(&self, step) -> Vec<CommandRun>`: filter, sort by `(queued_at, id)`.
- `State::upsert_step_tree` (`:3617-3627`) gains a `now: DateTime<Utc>` parameter (the `impl WriteStore` arm at `:4439` passes `Utc::now()` as `promote_step`'s does at `:4455`) and, after the inserts, when `trees` is non-empty: `let primary = trees.iter().find(|t| self.repos.get(&t.repo_id).is_some_and(|r| r.is_primary)).or_else(|| trees.iter().min_by_key(|t| t.repo_id)); step.isolation_path = Some(primary.path.clone()); step.updated_at = now;`.
- `counts`'s `command_runs: 0` (`:2836`) becomes the filtered count over `steps`, the shape of `run_step_trees` two lines above; the doc at `:2726` drops `command_runs` from its "constant `0`" sentence.
- Two `impl WriteStore` arms after `record_commits` (`:4443`), `self.write(|state| …)` each.

### 2.4 `PgStore` (`crates/htui-store/src/pg/write.rs`)

- `record_command_run`: one transaction; `step_exists(&mut tx, new.run_step_id)` (`:155`); `INSERT INTO command_run (id, run_step_id, box_id, class, command, cwd, status, exit_code, output, queued_at, started_at, finished_at) VALUES ($1 … $12)` with `status` bound as `new.status.as_str()`; `23503` → `Constraint` and `23505` → `Constraint` through `map_sqlx`; commit; return `CommandRun` built from `new` (no `RETURNING`: every column is the caller's, and `DEFAULT now()` on `queued_at` is never exercised).
- `command_runs`: `query_as!(CommandRun, r#"SELECT id AS "id: CommandRunId", run_step_id AS "run_step_id: StepId", box_id AS "box_id: BoxId", class, command, cwd, status AS "status: CommandRunStatus", exit_code, output, queued_at, started_at, finished_at FROM command_run WHERE run_step_id = $1 ORDER BY queued_at, id"#)` — the `step_trees` shape (`pg/read.rs:709-727`), but in `write.rs` because the method is `WriteStore`'s.
- `upsert_step_tree` (`:3187-3235`): after the per-row loop and before `commit`, when `trees` is non-empty: `SELECT id FROM repo WHERE id = ANY($1) AND is_primary` over the batch's repo ids → the chosen row (primary, else the lowest `repo_id` computed in Rust) → `UPDATE run_step SET isolation_path = $2 WHERE id = $1`. `updated_at` is the `set_updated_at` trigger's (`0001_init.sql:21`, applied by the loop at `:575-579`); nothing sets it by hand.

Three new `sqlx::query!`/`query_as!` sites → regenerate `.sqlx` (227 files today) and commit the new `query-*.json` with the queries.

### 2.5 `Writer` (`crates/htui-store/src/writer.rs`) and the spies

- `Writer`: two arms after `record_commits` (`:795-800`), `match self { Self::Memory(store) => …, Self::Online(pg) => … }`. No `BufferedWriter` (F-A).
- `crates/htui-agent/src/conformance.rs` after the `record_commits` arm (`:1008-1010`): `async fn record_command_run(&self, new: NewCommandRun) -> StoreResult<CommandRun> { self.inner.record_command_run(new).await }` and `async fn command_runs(&self, step: StepId) -> StoreResult<Vec<CommandRun>> { self.inner.command_runs(step).await }`; `CommandRun`, `NewCommandRun` join the file's `htui_core::model` import.
- `crates/htui-agent/tests/recorder.rs` after `:701-703`: the same two arms on `SpyStore`.

### 2.6 Conformance (`crates/htui-core/src/store/conformance.rs`)

**First failing test**: `verify_run_is_recorded` — `E0599 no method named record_command_run` until §2.2 lands. Name appended to `CASES` after `"trees_and_commits_round_trip"` (`:80`); `run_case` arm after `:159`; count 48 → 49 in `crates/htui-core/tests/mem_store.rs:36-42` (append `, and MOD-4 milestone 3's one for \`record_command_run\` (plan D31)` to the ledger string) and `crates/htui-store/tests/pg_conformance.rs:19`.

Legs, on `ids::STEP_R2_PRD` and `ids::BOX`:

1. `record_command_run(NewCommandRun { id: CommandRunId::new(), run_step_id: STEP_R2_PRD, box_id: BOX, class: "verify", command: "cargo test", cwd: "/srv/trees/prd/core", status: Done, exit_code: Some(0), output: Some("ok"), queued_at: t0, started_at: Some(t0), finished_at: Some(t0 + 1s) })` → the returned row equals the input field for field.
2. A second row at `queued_at: t0 - 1s` with `status: Failed, exit_code: None, output: Some("no `sh` on PATH")` → `command_runs(STEP_R2_PRD)` is `[second, first]` (queued order, not insertion order).
3. `command_runs(ids::STEP_PLAN)` is empty; `command_runs(StepId::new())` is `Ok(vec![])`.
4. An unknown `run_step_id` → `NotFound { entity: "run_step" }`, checked before the box (pass an unknown box too, to prove precedence).
5. A known step and `BoxId::new()` → `Constraint` containing `command_run.box_id`.
6. The first row's `id` again → `Constraint` containing `already exists`.
7. `counts` for the fixture project reports `command_runs == 2` on a store that started at `0` (the `Counts` doc at `traits.rs:1322-1327` is rewritten: MOD-4 milestone 3 is the first writer).

`trees_and_commits_round_trip` (`:4995`) extension: the `tree` closure's `path` becomes `format!("/srv/trees/prd/{}", if repo == core { "core" } else { "docs" })`; after the first two-row upsert, the step row read through `run_steps(<the run at fixtures.rs:1450>)` has `isolation_path == Some("/srv/trees/prd/core")` (`core` is the primary); after the one-row upsert of `first`, it equals that row's path; `upsert_step_tree(step, &[])` leaves it.

Doc comment names the Pg twin `pg_criteria.rs::command_run_round_trips_and_orders_by_queued_at`; add it to `PENDING` (`:6605-6608` region) in T1's first commit and remove it in the second (`every_cross_referenced_test_name_exists` enforces both directions, H-9).

### 2.7 `pg_criteria` twin (`crates/htui-store/tests/pg_criteria.rs`)

`command_run_round_trips_and_orders_by_queued_at`: the `common::demo_db()` skip prologue; legs 1, 2 and 5 of §2.6 against Postgres; plus a 70 KiB `output` (the 64 KiB tail cap plus slack) round-trips byte for byte, and `SELECT isolation_path FROM run_step WHERE id = $1` after a two-row `upsert_step_tree` is the primary's path.

---

## 3. T2 — the manifests and `isolate/git.rs`

### 3.1 Manifests

Root `Cargo.toml`, `[workspace.dependencies]` after `walkdir` (`:101`):

```toml
# MOD-4 milestone 3 (plan D22, D47): every read and every ref write of the isolator. No default
# features: `sha1` is the only one of them we need and is named explicitly.
gix = { version = "0.87.1", default-features = false, features = [
    "sha1", "max-performance-safe", "parallel", "index", "status"] }
```

`crates/htui-orch/Cargo.toml` `[dependencies]`: `gix`, `walkdir`, `process-wrap`, `which`, `dirs` as `{ workspace = true }`; `tokio` features `["sync", "rt", "process", "io-util", "time", "fs"]`; under `[target.'cfg(windows)'.dependencies]` `windows = { workspace = true }` (the `htui-agent` shape). `[dev-dependencies]`: `tempfile = "3"` (per-crate, as the other four crates declare it).

**The five features, each tied to a call** (F-D):

| Feature | The call | Where the gate is |
|---|---|---|
| `sha1` | every `ObjectId`; without it `gix-hash` fails to compile (`E0004`) | `gix-0.87.1/Cargo.toml:228-233`, dropped by `default-features = false` (`:132-139`) |
| `parallel` | `Repository: Send`, which `spawn_blocking` needs | `src/types.rs:155-158` |
| `status` | `is_dirty()` (D24, D25, D29) | `src/lib.rs:492`; `src/status/mod.rs:168`; its closure `[gix-status, dirwalk, index, blob-diff, gix-diff/index]` (`Cargo.toml:241-247`) pulls `attributes`, which gates `submodules()` (`src/repository/mod.rs:62-63`) |
| `index` | `open_index()` for D25's conflict path list | `src/repository/mod.rs:43-44`; `src/repository/index.rs:25`. Already in `status`'s closure; named because a call needs it |
| `max-performance-safe` | OQ-5's `copy_range` reads every object of a step's range under the default pack cache | `Cargo.toml:183` (`= max-control`); the one entry not tied to a call, kept for the reason D22 gives |

Ungated and used: `head_id` (`reference.rs:211`), `reference(...)`/`find_reference` (`:79`, `:323`), `worktrees()` (`worktree.rs:46`), `rev_walk` (`revision.rs:174`), `find_object`/`write_object`/`commit_as` (`object.rs:55`, `:250`, `:361`). Gone with D47: `worktree-mutation` (`gix-worktree-state`), `index`'s write half. Never named: `merge`, `blob-diff` (transitive via `status`), `dirwalk` (transitive), `revision`.

**C-6** (run once in T2's first commit and again at the workspace gate): `cargo tree -p htui-orch -i gix --edges features` shows exactly those five under `htui-orch`; `cargo tree -p htui-orch -e normal | grep -E 'gix-(merge|worktree-state|transport|protocol|credentials|negotiate)'` prints nothing; `grep -c 'name = "git2\|name = "libgit2' Cargo.lock` is `0` (it is `0` at HEAD).

### 3.2 `isolate.rs` (T2's three edits)

- `pub mod git;` after the `use` block (`:24`), with `pub mod copy;` (T4) and `pub mod real;` (T5) to follow.
- `Prepared` gains `pub extra_dirs: Vec<PathBuf>` after `cwd` (`:79`): *`SessionSpec.extra_dirs` (`driver.rs:257-258`): every tree that is not under `cwd` — the second repo of a `shared_serialized`/`local` run (plan D28).* `FakeIsolator::prepare` (`fake.rs:182-185`) sets `extra_dirs: Vec::new()`.
- `Isolator::prepare`'s doc (`:93-97`) gains: *A real isolator makes this idempotent for `worktree` and `copy` (plan D38): a second call for the same `(run, step)` finds the tree it made and reports the same `before_hash`. The trait does not promise it — the fake mints a fresh one per call — and the engine calls it once (`engine.rs:1069-1073`).*

### 3.3 `isolate/git.rs` — the `Cli` contract

```rust
/// The `git` binary this process spawns for the five verbs `gix` 0.87.1 lacks (plan OQ-1, D23,
/// D25, D47; `docs/ANA-2.md:1777` as amended). Located once, version-checked once, cloned freely.
#[derive(Debug, Clone)]
pub(crate) struct Cli {
    binary: PathBuf,
    version: (u32, u32, u32),
}

/// The oldest `git` whose `worktree add --lock --reason` and `worktree remove --force --force`
/// exist (plan Risks, the ledger's tag walk): 2.33.0, August 2021.
pub(crate) const MIN_GIT: (u32, u32, u32) = (2, 33, 0);
/// One verb's wall-clock budget; on expiry the process group is killed.
pub(crate) const VERB_TIMEOUT: Duration = Duration::from_secs(120);
/// Bytes kept of each of stdout and stderr: the **last** 64 KiB (D30's figure).
pub(crate) const CAPTURE_TAIL: usize = 64 * 1024;
```

**Locating and the version floor** — `Cli::locate() -> Result<Cli, IsolateError>`, synchronous, run once by `GixIsolator::new` and once by the test support (§3.6):

1. `which::which("git")` (`PATHEXT`-aware on Windows, `probe.rs:339-347`'s reason) → `Err` → `Refused("git not on PATH")`.
2. `std::process::Command::new(&binary).arg("--version").env("LC_ALL", "C").output()` with the same scrub as below → spawn error → `Refused(format!("git --version could not run: {err}"))`.
3. Parse the first line as `git version <a>.<b>.<c>[.<suffix>]` (`2.43.0`, `2.47.1.windows.1`, `2.39.5 (Apple Git-154)`): three leading unsigned integers split on `.`, everything after the third ignored. No match → `Refused(format!("git --version output is not understood: {line}"))`.
4. `version < MIN_GIT` → `Refused(format!("git {a}.{b}.{c} is older than 2.33.0"))`.

The result is cached on the `Cli` and on the isolator that holds it (§6.1); no verb re-probes.

**Environment** — `Cli::command(&self, verb: &str, cwd: &Path) -> tokio::process::Command`, the one place the child environment is shaped:

| Action | Variables | Why |
|---|---|---|
| inherit | everything else (`PATH`, `HOME`, `XDG_CONFIG_HOME`, `GIT_CONFIG_*`, `GIT_CONFIG_NOSYSTEM`) | the user's `~/.gitconfig` and the repo's config apply by design (plan Risks, "inherited, not controlled") |
| `env_remove` | `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES`, `GIT_PREFIX`, `GIT_DISCOVERY_ACROSS_FILESYSTEM` | every variable that redirects **repository discovery or the location of the index, refs or objects** — `GIT_DIR=/nonexistent` was verified to break `worktree add` (ledger `:711`); the others are the same class, and a parent `htui` launched from inside a `git` hook or a `git rebase -x` inherits them |
| set | `LC_ALL=C` | D39's classifier and D25's conflict branch match English text |
| set | `GIT_TERMINAL_PROMPT=0` | no verb may block on a tty |
| set | `GIT_OPTIONAL_LOCKS=0` | `git` may otherwise refresh the index opportunistically and take `index.lock` under a concurrent `gix` read (`git-config(1)`, `GIT_OPTIONAL_LOCKS`) |
| set | `GIT_ADVICE=0` | 2.42+ suppresses `hint:` lines so the last stderr line is the `fatal:`/`error:` one; ignored by older `git`, harmless |
| set, `merge_no_ff` only | `GIT_AUTHOR_NAME=htui`, `GIT_AUTHOR_EMAIL=htui@localhost`, `GIT_COMMITTER_NAME=htui`, `GIT_COMMITTER_EMAIL=htui@localhost` | D25: a managed repo may carry no `user.*`; verified to commit with `HOME=/nonexistent` |

`current_dir(cwd)`, `stdin(Stdio::null())`, `stdout(Stdio::piped())`, `stderr(Stdio::piped())`, `kill_on_drop(true)`.

**Spawning** — `Cli::run(&self, verb: &'static str, cwd: &Path, args: &[&OsStr], extra_env: &[(&str, &str)]) -> Result<Exited, IsolateError>`:

- Wrapped exactly as `launch.rs:1110-1160`: `CommandWrap::from(command)`, `.wrap(ProcessGroup::leader())` on Unix; `.wrap(CreationFlags(PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW)))` + `.wrap(JobObject)` on Windows, with the refused-job-object fallback. Spawn error → `Git(format!("git {verb}: cannot spawn {}: {err}", binary.display()))`.
- stdout and stderr are read **concurrently** (`tokio::join!` over two readers) into `TailBuffer`s that keep the last `CAPTURE_TAIL` bytes each (a `VecDeque<u8>` drained from the front past the cap) — not `launch.rs`'s `take(LIMIT + 1)` head cap (`launch.rs:736-740`), because the line that matters is the last one. Neither pipe can fill and stall the child.
- The whole of read-both-then-`wait` runs under `tokio::time::timeout(VERB_TIMEOUT)`; on expiry `child.kill()` (the group/job) is awaited and the error is `Git(format!("git {verb} timed out after 120s"))`.
- `Exited { code: Option<i32>, stdout: String, stderr: String }` is returned for **every** exit status; `String::from_utf8_lossy`. `code` is `None` when a signal killed the child (Unix).

**A non-zero exit becomes a typed error** through `Exited::failure(&self, verb) -> IsolateError`: `Git(format!("git {verb}: {}", last non-empty stderr line, or "exit status {code} with no stderr", or "killed by signal"))`. Classification is the verb's: each verb matches on `code` and `stderr` before calling `failure`, so the sentences below are fixed here:

| Class | Detected by | Produces |
|---|---|---|
| not on `PATH` | `Cli::locate` step 1 | `Refused("git not on PATH")` |
| too old | step 4 | `Refused("git 2.32.1 is older than 2.33.0")` |
| cannot spawn | `spawn()` error | `Git("git worktree add: cannot spawn /usr/bin/git: <io error>")` |
| timed out | `timeout` elapsed | `Git("git merge timed out after 120s")`, never retried |
| killed by signal | `code == None` | `Git("git worktree add: killed by signal")` |
| lock held | `lock_signature(stderr)`: any of `Unable to create '` … `.lock': File exists`, `cannot lock ref`, `Unable to write index` | the verb's `Git("git <verb>: <last line>")`, **retried** by `with_retry` |
| merge conflict | `merge --no-ff` exit `1` without a lock signature | `Refused("merge_conflict: <path list>")` after `abort_merge` (§3.5) |
| already removed | `worktree remove` exit `128` with `is not a working tree` | `Ok(())` |
| no merge to abort | `merge --abort` exit `128` with `There is no merge to abort` | `Ok(())` |
| anything else | non-zero | `Git("git <verb>: <last non-empty stderr line>")` |

`stdout` is never parsed on success; every success has a `gix` post-condition (§3.5).

### 3.4 `isolate/git.rs` — the `gix` half

Synchronous functions over `&Path` returning owned values, each called by T5 under `tokio::task::spawn_blocking` with an owned `PathBuf`; no `gix::Repository` crosses an `.await` (`Repository` is `Send` and not `Sync`, `types.rs:148`). Each doc comment names the `gix-0.87.1/src/...` line it wraps.

```rust
pub(crate) fn open(path: &Path) -> Result<gix::Repository, IsolateError>;       // gix::open, src/lib.rs:418; error → Git("cannot open <path>: …")
pub(crate) fn head(path: &Path) -> Result<String, IsolateError>;               // head_id()?.detach().to_hex(); Unborn → Refused(A-4's sentence)
pub(crate) fn is_dirty(path: &Path) -> Result<bool, IsolateError>;             // is_dirty(), src/status/mod.rs:168 (D24)
pub(crate) fn has_submodules(path: &Path) -> Result<bool, IsolateError>;       // submodules()?.is_some_and(|mut it| it.next().is_some()), src/repository/submodule.rs:93
pub(crate) fn create_branch(path: &Path, name: &str, target: &str) -> Result<(), IsolateError>;
    // reference(format!("refs/heads/{name}"), id, PreviousValue::MustNotExist, "htui: label"), reference.rs:79 (D26, A-2)
pub(crate) fn branch_target(path: &Path, name: &str) -> Result<Option<String>, IsolateError>;
    // find_reference, reference.rs:323; NotFound → Ok(None) (D38)
pub(crate) fn worktree_by_path(main: &Path, path: &Path) -> Result<Option<WorktreeEntry>, IsolateError>;
    // worktrees()?, worktree.rs:46; Proxy::{base, is_locked, lock_reason}, proxy.rs:48-103; compared canonicalised (H-11)
pub(crate) fn worktrees_under(main: &Path, root: &Path) -> Result<Vec<PathBuf>, IsolateError>;
    // every Proxy whose base() starts with root — D46's report (F-B)
pub(crate) fn conflicted_paths(path: &Path) -> Result<Vec<String>, IsolateError>;
    // open_index()?, index.rs:25; entries whose stage() != Stage::Unconflicted, gix-index-0.55.0/src/entry/mod.rs:3-11; sorted, deduplicated
pub(crate) fn head_parents(path: &Path) -> Result<Vec<String>, IsolateError>;  // head_commit()?.parent_ids(), object/commit.rs:154 (D25's post-condition)
pub(crate) fn copy_range(from: &Path, to: &Path, base: &str, tip: &str) -> Result<u32, IsolateError>;
    // rev_walk([tip]).with_hidden([base]).all()?, revision.rs:174; per commit: find_object + write_object of the commit, its tree, every tree entry
    // reachable that `to` lacks (OQ-5); returns the object count. Runs before `merge_no_ff` in copy mode.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeEntry { pub path: PathBuf, pub locked: bool, pub lock_reason: Option<String> }
```

`copy_range` writes objects with `gix`'s `write_object`, which takes `objects/` loose-object locks — D39's first source.

### 3.5 `isolate/git.rs` — the five verbs and `with_retry`

```rust
impl Cli {
    /// D23: `git worktree add --lock --reason "htui run <run>" -b htui/<step> <path> <before>`,
    /// in `repo`. Post-condition: `head(path) == before` and `worktree_by_path(repo, path)` is
    /// `Some(locked)`; either failing → `Git("git worktree add: created a tree that does not
    /// check out")`.
    pub(crate) async fn add_worktree(&self, repo: &Path, path: &Path, step: StepId, run: RunId, before: &str) -> Result<(), IsolateError>;
    /// D23/D46: `git worktree remove --force --force <path>` in `repo`; "is not a working tree"
    /// and a vanished directory are both `Ok`.
    pub(crate) async fn remove_worktree(&self, repo: &Path, path: &Path) -> Result<(), IsolateError>;
    /// D47: `git reset --hard <target>` in `tree`. Post-condition: `head(tree) == target` and
    /// `!is_dirty(tree)`.
    pub(crate) async fn reset_hard(&self, tree: &Path, target: &str) -> Result<(), IsolateError>;
    /// D25: `git merge --no-ff --no-edit -m "htui: reconcile <step>" <after>` in `primary` under
    /// the four identity variables. Exit 0 → post-condition `head_parents(primary) == [before,
    /// after]` → `Ok(Merged { commit })`. Exit 1 with a lock signature → `abort_merge` then the
    /// lock error (so `with_retry` sleeps and retries). Exit 1 otherwise → `conflicted_paths`,
    /// `abort_merge`, `Err(Refused("merge_conflict: a, b/c"))`. Any other → `abort_merge`
    /// (swallowing "no merge to abort"), then `Git`.
    pub(crate) async fn merge_no_ff(&self, primary: &Path, step: StepId, before: &str, after: &str) -> Result<Merged, IsolateError>;
    /// `git merge --abort`; exit 128 "There is no merge to abort" is `Ok`.
    pub(crate) async fn abort_merge(&self, primary: &Path) -> Result<(), IsolateError>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Merged { pub commit: String }

/// D39: three retries at 200, 400 and 800 ms when `is_lock_error` says so; the schedule is
/// entirely ours (`gix` acquires with `Fail::Immediately`, fact-check). Every attempt's error is
/// logged at `warn` with its classification, so a rewording is a one-line fix.
pub(crate) async fn with_retry<T, F, Fut>(verb: &'static str, mut op: F) -> Result<T, IsolateError>
where F: FnMut() -> Fut, Fut: Future<Output = Result<T, IsolateError>>;
/// `Display` contains `.lock`, `cannot lock`, `Unable to create '`…`.lock': File exists`,
/// `cannot lock ref` or `Unable to write index`; a `timed out` text is never a lock error.
pub(crate) fn is_lock_error(err: &IsolateError) -> bool;
```

Every write in this file — the five verbs, `create_branch`, `copy_range` — is called through `with_retry`; reads are not.

### 3.6 Test support and the skip

```rust
/// Printed byte for byte by every git-backed test on a box without a usable `git` (plan D40; the
/// `htui_store::testkit::SKIP` convention, `testkit.rs:35`).
pub const SKIP_GIT: &str = "skipped: git not on PATH";

#[cfg(any(test, feature = "test-support"))]
pub mod testkit {
    /// `Ok(cli)` when `Cli::locate()` succeeds, `Err(sentence)` with `SKIP_GIT` or
    /// `skipped: git <v> is older than 2.33.0` otherwise. Decided once per process (`OnceLock`).
    pub fn usable_git() -> Result<Cli, String>;
    /// `let Some(git) = skip_without_git!() else { return };` — prints the sentence and returns.
    #[macro_export] macro_rules! skip_without_git { … }
    /// `git worktree list --porcelain` in `repo`, parsed into `(path, head, branch, locked reason)`
    /// rows — the criterion-11/13 oracle over the same binary.
    pub async fn worktree_list(cli: &Cli, repo: &Path) -> Vec<PorcelainWorktree>;
    /// `gix::init` + `commit_as` under a `TempDir`: one file `f`, one commit, returns the hash.
    pub fn repo_with_one_commit(dir: &Path) -> String;
    pub fn commit_file(repo: &Path, name: &str, body: &str, message: &str) -> String;
}
```

`tests/gix_isolator.rs` reaches these through `htui_orch::isolate::git::testkit` under `--all-features`, which the gate already passes for `fake_conformance.rs`'s sake.

### 3.7 Tests (in `git.rs`'s `mod tests`, TDD order)

No `git` needed: `parses_git_version_and_refuses_below_2_33` (**first failing test** — `Cli::parse_version("git version 2.43.0")`, `"git version 2.47.1.windows.1"`, `"git version 2.39.5 (Apple Git-154)"` parse; `"2.32.1"` → the exact refusal sentence; garbage → the "not understood" sentence); `init_commit_and_head_round_trip`; `is_dirty_ignores_untracked_and_sees_a_modified_tracked_file` (D24); `head_of_an_unborn_repo_is_refused` (A-4); `branch_label_is_created_once_and_read_back` (`create_branch` twice → the second is a lock/exists error; `branch_target`); `copy_range_moves_every_object_between_odbs` (OQ-5: two commits in `a`, none in `b`; after `copy_range` every object of the range `find_object`s in `b`); `with_retry_retries_three_times_on_a_lock_error_and_not_on_others` (a closure counting attempts: a `Git("… cannot lock ref …")` is tried four times over ~1.4 s, a `Git("… timed out …")` once, a `Refused` once); `tail_buffer_keeps_the_last_64_kib`; `env_scrub_list_is_the_documented_one` (the `Command`'s env map, read back through `as_std().get_envs()`).

`git` needed (each opens with `skip_without_git!()`): `add_worktree_is_listed_by_gix_and_by_git` (D23's post-conditions plus the porcelain oracle's `locked htui run <run>` line and `branch refs/heads/htui/<step>`); `add_worktree_on_an_existing_branch_is_a_git_error_naming_the_branch` (`create_branch` first → `Git("git worktree add: fatal: a branch named 'htui/<step>' already exists")`); `remove_clears_a_locked_dirty_worktree_and_a_vanished_one`; `a_stale_entry_that_survives_remove_is_reported_and_never_pruned` (unlock by hand, `rm -rf` the directory, simulate `remove` failing by making the entry a plain file — then `worktrees_under` lists it and the porcelain oracle still does afterwards); `reset_hard_restores_a_deleted_and_a_modified_tracked_file` (D47); `merge_no_ff_of_a_descendant_makes_a_two_parent_commit` (parents `[before, after]`, `%an` is `htui`, the file present, no `MERGE_HEAD`); `a_conflicting_merge_is_refused_with_the_path_and_aborted` (`Refused("merge_conflict: f")`, primary clean at `before`, no `MERGE_HEAD`); `a_held_index_lock_inside_merge_is_aborted_and_retried` (plant `.git/index.lock`, remove it from a `tokio::spawn` after 100 ms; the merge commits on attempt 2 and `MERGE_HEAD` is absent between attempts); `git_env_is_scrubbed` (the test sets `GIT_DIR=/nonexistent` on its own process; `add_worktree` still succeeds); `a_verb_that_hangs_times_out` (`#[cfg(unix)]`: a `Cli` whose `binary` is a shell script `sleep 30` behind a 200 ms `VERB_TIMEOUT` injected through `Cli::with_timeout` under `cfg(test)` → the `timed out` sentence and no orphan, checked by `kill -0` on the recorded pid).

**C-7**: `grep -rn 'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` (T2) and, after T3, `verify.rs`; nothing else.

---

## 4. T3 — `verify.rs`

### 4.1 Surface

```rust
/// The boxed future every [`Verifier`] returns — `IsolatorFuture`'s shape (`isolate.rs:32`),
/// infallible: a verify that cannot run is a report, not an error (ANA-2 `:515`).
pub type VerifierFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What stage 5 hands the verifier (plan D30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyRequest {
    /// `SnapshotPhase::verify_command`; `None` yields no report.
    pub command: Option<String>,
    /// The primary repo's tree, or `None` → `unavailable` with `no primary tree`.
    pub cwd: Option<PathBuf>,
    /// The step deadline's remainder; `None` is no deadline; `Some(0)` is already elapsed.
    pub remaining: Option<Duration>,
    pub step: StepId,
}

/// One run of the command, in `command_run`'s shape (plan D31's row is built from this).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub outcome: VerifyOutcome,
    pub exit_code: Option<i32>,
    /// Scrubbed, tail-capped output (stdout and stderr merged), or the `unavailable` reason.
    pub output: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

/// M2 D6's pattern applied to the walk's other side effect: a seam now, implementations by kind.
pub trait Verifier: Send + Sync + fmt::Debug {
    fn run<'a>(&'a self, request: VerifyRequest) -> VerifierFuture<'a, Option<VerifyReport>>;
}

/// D30: `sh -c` / `cmd /C` in the primary tree, the process environment unchanged, one
/// permit of the `verify` class, the deadline's remainder as the timeout.
#[derive(Debug)]
pub struct ShellVerifier {
    permits: Arc<tokio::sync::Semaphore>,
    scrubber: Arc<dyn Scrubber>,
    clock: Arc<dyn Clock>,
}
impl ShellVerifier {
    /// `limits` is the resolved `command_limits` map (`BoxSettings.command_limits`, `box_.rs:116`,
    /// falling through to `app_setting.command_limits`, `0003:129`); `verify` defaults to `1`.
    pub fn new(limits: &BTreeMap<String, u32>, scrubber: Arc<dyn Scrubber>, clock: Arc<dyn Clock>) -> Self;
}
```

`Clock` is the isolator module's trait (`isolate.rs:140`); the verifier stamps `started_at`/`finished_at` from it so a test can pin them. `lib.rs` gains `pub mod verify;` and `pub use verify::{ShellVerifier, Verifier, VerifierFuture, VerifyReport, VerifyRequest};`.

### 4.2 `ShellVerifier::run`

1. `command: None` → `None`. `cwd: None` → `Some(unavailable("no primary tree"))`. `remaining == Some(Duration::ZERO)` → `unavailable("deadline elapsed")` without spawning.
2. `permits.acquire()` — the class semaphore (`:504-506`); the wait counts against `remaining`.
3. Spawn `sh -c <command>` (Unix) / `cmd /C <command>` (Windows) with `current_dir(cwd)`, `stdin(null)`, stdout and stderr both piped, wrapped as `launch.rs:1110-1160` (F-I's constant), `kill_on_drop(true)`. Spawn error → `unavailable(format!("cannot spawn sh: {err}"))`.
4. Read both pipes concurrently into one merged `TailBuffer` (64 KiB), under `tokio::time::timeout(remaining)` when `Some`; expiry → kill the group → `unavailable("deadline elapsed")` with the output tail kept.
5. Exit: `code 0` → `Pass`, `exit_code Some(0)`; non-zero → `Fail`, the code; no code (signal) → `unavailable("killed by signal")`.
6. Scrub: `serde_json::Value::String(output)` through `scrubber.scrub` (`scrub.rs:49-58`); `Err(Unmasked)` → the output is **dropped** and replaced by `"<scrub refused: N bytes withheld>"`, the outcome unchanged (the exit code is a fact; the text is not persistable, `R-SEC-3`).

`unavailable(reason)` is `VerifyReport { outcome: Unavailable, exit_code: None, output: reason, … }`; the engine writes `command_run.status = failed` for it and `done` for pass/fail (§7.2).

### 4.3 Tests (`verify.rs` `mod tests`, all `#[cfg(unix)]` except the first)

**First failing test**: `no_verify_command_yields_no_report`. Then `exit_zero_is_pass_with_code_zero` (`true`); `nonzero_exit_is_fail_with_the_code` (`exit 3`); `a_missing_binary_is_unavailable_with_a_reason` — **not** unavailable: `sh -c definitely-not-a-binary` exits 127 and is `Fail(127)` with `command not found` in the output, which is the plan's `:229-231` reading ("the shell reports it"); the genuinely `unavailable` cases are `no_primary_tree_is_unavailable` and `a_verifier_without_sh_is_unavailable` (`PATH=""` through a `cfg(test)` shell-path override); `a_timeout_is_unavailable_and_kills_the_group` (`sleep 30` behind 200 ms; the child pid is gone); `output_is_tail_capped_and_masked` (a 100 KiB `yes | head -c` keeps the last 64 KiB; a `MinimalScrubber::new(["s3cret"])` masks it); `the_verify_semaphore_admits_one` (two `sleep 0.2` requests under `verify = 1` have non-overlapping `[started_at, finished_at]`); `a_zero_remainder_never_spawns`.

---

## 5. T4 — `isolate/copy.rs`

```rust
/// ANA-2 `:923-924`'s seven entries, applied when `ProjectSettings.copy_exclude` is empty (D35).
pub const DEFAULT_COPY_EXCLUDE: [&str; 7] = ["target/", "build/", "cmake-build-*/", "node_modules/", ".venv/", "out/", "dist/"];

/// One entry: a path **component** with at most one trailing `*` ("no glob crate", ANA-2 `:1783`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclude { prefix: String, wildcard: bool }
impl Exclude {
    pub fn parse(entry: &str) -> Option<Self>;              // strips a trailing `/`; refuses `/` inside, `*` anywhere but last
    pub fn matches(&self, component: &str) -> bool;
}
pub fn excludes(project: &[String]) -> Vec<Exclude>;        // project list, else the default; unparseable entries are skipped with a `warn`

/// Bytes of every file `copy_tree` would copy: a `walkdir` over `src`, pruning a directory whose
/// name any exclude matches (`vector_sync.rs`'s bounded walk, F-H). `.git/` is counted.
pub fn measure(src: &Path, excludes: &[Exclude]) -> Result<u64, IsolateError>;

/// `src` → `dst` minus excludes: files, directories, symlinks re-created as symlinks (target
/// verbatim), Unix permissions preserved. `dst` must not exist. Refuses before the first byte:
/// `Refused("not a git checkout")` when `src/.git` is absent, `Refused("source is a linked
/// worktree")` when it is a file (D35).
pub fn copy_tree(src: &Path, dst: &Path, excludes: &[Exclude]) -> Result<(), IsolateError>;
```

All synchronous; T5 calls them under `spawn_blocking`. `isolate.rs` gains `pub mod copy;`.

Tests, TDD order: `default_excludes_match_ana2s_seven_entries` (**first failing**), `an_empty_project_list_falls_back_to_the_default`, `cmake_build_star_matches_by_prefix` (`cmake-build-debug` yes, `cmake-buildx/y` — a nested path — no, `xcmake-build-` no), `measure_counts_only_what_would_be_copied` (1 MiB under `target/` uncounted), `copy_tree_reproduces_the_layout_minus_excludes_and_keeps_dot_git`, `a_source_with_a_dot_git_file_is_refused`, `a_source_without_dot_git_is_refused`, `a_symlink_is_copied_as_a_symlink` (`#[cfg(unix)]`).

---

## 6. T5 — `isolate/real.rs`, `GixIsolator`

### 6.1 Surface

```rust
/// One repo's checkout on this box, resolved by the caller (plan D34).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCheckout { pub name: String, pub local_path: PathBuf, pub is_primary: bool }

#[derive(Debug, Clone)]
pub struct IsolatorConfig {
    pub repos: BTreeMap<RepoId, RepoCheckout>,
    /// `identity::config_root()/trees` (`identity.rs:45-52`; ANA-2 `:905`). Passed, not read:
    /// `htui-orch` never depends on `htui-store`.
    pub scratch_root: PathBuf,
    /// `ProjectSettings.copy_exclude`; empty → `copy::DEFAULT_COPY_EXCLUDE`.
    pub copy_exclude: Vec<String>,
    /// `app_setting.copy_max_total_bytes` (`0003:134`).
    pub copy_max_total_bytes: u64,
    pub box_id: BoxId,
}

/// The production [`Isolator`]: four modes over `gix` reads and five `git` verbs (plan D22–D47).
#[derive(Debug)]
pub struct GixIsolator {
    config: IsolatorConfig,
    /// `Cli::locate()`'s answer, taken once; `Err` is the refusal sentence `worktree` and
    /// `reconcile` answer with (D40: "a test can never pass on a box where production would refuse").
    git: Result<Cli, String>,
    /// D43 / A-1: one guard per `(box, repo)`, held by the step that took it.
    locks: Mutex<BTreeMap<(BoxId, RepoId), Arc<tokio::sync::Mutex<()>>>>,
    held: Mutex<BTreeMap<StepId, Vec<tokio::sync::OwnedMutexGuard<()>>>>,
}
impl GixIsolator {
    /// Validates the root against every checkout (invariant 4) and probes `git` once.
    /// Synchronous: it spawns `git --version`; milestone 6 calls it at worker start.
    pub fn new(config: IsolatorConfig) -> Result<Self, IsolateError>;
    /// A test seam: the same, with `git` decided by the caller.
    #[cfg(feature = "test-support")]
    pub fn with_git(config: IsolatorConfig, git: Result<Cli, String>) -> Result<Self, IsolateError>;
}
```

`isolate.rs`: `pub mod real; pub use real::{GixIsolator, IsolatorConfig, RepoCheckout};`; `lib.rs`: the same three in the `isolate::` re-export. `Isolator::reconcile`'s doc (`isolate.rs:116-120`) is rewritten: *One winner this milestone (`fanout_index = 0`); for `worktree` and `copy` it is a real `--no-ff` merge into the primary tree and the returned `after_hash` is the merge commit (plan D25); for `shared_serialized` and `local` it is the identity.*

Refusal sentences, as free functions in `real.rs` (house style): `no_checkout_for_repo()` = `no checkout for this repo on this box` (`engine.rs:2606`'s pinned text); `root_inside_repo(root, repo)` = `scratch root <root> is inside a managed repository (<repo>)`; `repo_inside_root(repo, root)` = `managed repository <repo> is inside the scratch root <root>`; `duplicate_repo_name(name)` = `duplicate repo name <name>`; `submodules_refused()` = `submodules: worktree isolation is not supported`; `not_a_git_checkout()`, `linked_worktree_source()` (T4's); `copy_over_cap(need, cap)` = `copy would need <need> bytes; cap is <cap>`; `dirty_primary_tree()`, `primary_moved(hash)` = `primary_moved: <hash>`, `merge_conflict(paths)` = `merge_conflict: a, b/c`; `unborn_head(name)` (A-4). Paths are canonicalised (`std::fs::canonicalize`) before every prefix comparison, at `new` and at every `prepare` (H-11).

### 6.2 Paths and the session directory (D28)

Tree path `<root>/<run_id>/<step_id>/<name>/` (`RunStepTree.path` is `to_string_lossy` of it; H-11). `cwd`: `worktree`/`copy` → `<root>/<run_id>/<step_id>/` (created with `create_dir_all`, also for an empty scope); `shared_serialized`/`local` → the checkout of the scope's `is_primary` repo, else the first repo in scope order; every other tree of those two modes → `extra_dirs`. `Prepared.trees` is in scope order; rows read back through `step_trees` are in `repo_id` order (H-13).

### 6.3 `prepare` per mode

Common prologue: every repo in scope must be in `config.repos` (else `no_checkout_for_repo`) and its name unique (else `duplicate_repo_name`); `before = head(local_path)` (A-4).

- **`local`**: `dirty = is_dirty(local_path)`; row `{ mode, path: local_path, base_ref: before, dirty }`; no branch, nothing held.
- **`shared_serialized`**: `held[step].push(locks[(box, repo)].clone().lock_owned().await)` **before** the reads (A-1); then as `local`.
- **`worktree`**: `has_submodules` → `submodules_refused`; `git` is `Err` → `Refused(sentence)`; `tree = <root>/<run>/<step>/<name>`. **Idempotence (D38)**: if `tree/.git` exists, `branch_target(local_path, "htui/<step>")` is `Some(b)`, `head(tree) == b` and `!is_dirty(tree)` → reuse with `before = b`; if `tree/.git` exists and any of those fails → `remove_worktree` then fall through (H-5). Else `with_retry(add_worktree(local_path, tree, step, run, before))`. Row `{ mode, path: tree, base_ref: before, dirty: false }`.
- **`copy`**: `local_path/.git` absent → `not_a_git_checkout`; a file → `linked_worktree_source`; `tree/.git` exists and `branch_target(tree, "htui/<step>")` is `Some(b)` → reuse with `before = b` (A-2); a `tree.partial` directory exists → `remove_dir_all` it (H-6). Else `need = measure(local_path, &excludes)`; `need > cap` → `copy_over_cap`; `copy_tree(local_path, tree.partial, &excludes)`; `git` needed from here: `with_retry(reset_hard(tree.partial, before))`; `with_retry(create_branch(tree.partial, "htui/<step>", before))`; `rename(tree.partial, tree)`. Row `{ mode, path: tree, base_ref: before, dirty: false }` — the source's dirtiness is erased by the reset (mode table).

### 6.4 `capture`

Per row: `after = head(row.path)`; `after == row.base_ref` → `after_hash: None`. **`shared_serialized`**: `create_branch(local_path, "htui/<step>", after)` when `after` is `Some` (D26; an already-existing label with the same target is not an error), then **release** `held[step]` (A-1). **`worktree`**: `after_hash: None && !is_dirty(row.path)` → `remove_worktree` now (D27); the branch survives and is what `reconcile` reads. A row whose `path` no longer exists (a D27 removal on a re-run of `capture`, H-4) → `after = branch_target(local_path, "htui/<step>")`, `None` when equal to `base_ref`.

### 6.5 `cleanup` (D36, D46, F-Q)

For every row: `worktree` → `with_retry(remove_worktree(local_path, path))`; `copy` → `remove_dir_all(path)` (NotFound is fine); `shared_serialized` → release `held[row.run_step_id]` if still held; `local` → nothing. Then `remove_dir_all(<root>/<run>)`, NotFound fine. Then, for every distinct `worktree`-mode repo, `worktrees_under(local_path, <root>/<run>)`: a non-empty list → the whole call ends `Err(Git(format!("stale worktree entry {path}; run `git worktree prune` yourself")))` **after** every other row was processed. Errors are collected, not short-circuited; the first is returned and the rest `warn`ed. The engine `warn`s the returned one too (§7.4).

### 6.6 `reconcile` (D25)

Per row, with `before = row.base_ref`:

- `local` → identity: the row's current `step_commits` value is re-derived as `after = head(local_path)`, `None` when equal.
- `shared_serialized` → identity when `head(local_path) == branch_target(local_path, "htui/<step>")` (the only milestone-3 case); otherwise `Refused(primary_moved(head))`.
- `worktree` → `after = branch_target(local_path, "htui/<step>")`; `None` or `== before` → identity, `after_hash: None`. Else: `git` `Err` → `Refused`; `is_dirty(local_path)` → `Refused(dirty_primary_tree())`; `head(local_path) != before` → **unless** `head_parents(local_path) == [before, after]`, in which case the merge already happened and the row's `after_hash` is `head` (H-3's idempotence) → else `Refused(primary_moved(head))`; then `with_retry(merge_no_ff(local_path, step, before, after))` → `after_hash: Some(merged.commit)`.
- `copy` → `after = head(row.path)` (the copy still exists: cleanup is run-terminal); as `worktree` but with `with_retry(copy_range(row.path, local_path, before, after))` before the merge so `<after>` resolves in the primary (OQ-5).

### 6.7 Tests (`real.rs` `mod tests`, each over a `TempDir` with one or two `repo_with_one_commit`s, a config map, and `scratch_root = tmp/trees`)

No `git`: `a_scratch_root_inside_a_repo_is_refused` (**first failing test**), `a_repo_inside_the_scratch_root_is_refused`, `a_repo_missing_from_the_map_is_refused_with_the_pinned_sentence`, `local_records_dirty_true_and_head_as_before_hash` (criterion 12's record half), `local_cwd_is_the_primary_and_the_other_repo_is_an_extra_dir` (D28), `shared_serialized_labels_after_hash_and_holds_the_lock_until_capture` (A-1: a second `prepare` on the same `(box, repo)` from another step is pending — `tokio::time::timeout(50 ms)` elapses — until the first step's `capture`, after which it resolves; the label `htui/<step>` targets `after`), `cleanup_releases_a_lock_a_crashed_step_never_captured`, `worktree_prepare_is_refused_without_git` (`with_git(config, Err(SKIP_GIT-shaped sentence))`: `worktree` → `isolation refused: git not on PATH`; `local` on the same isolator prepares), `an_unborn_repo_is_refused` (A-4), `an_empty_scope_prepares_a_cwd_and_no_trees`.

`git` needed: `worktree_prepare_writes_one_tree_per_repo_outside_every_repo` (criterion 11's isolator half: two repos → two rows under the root, both `before_hash`es the repos' `HEAD`s, `cwd` the common parent, `extra_dirs` empty, both branches `htui/<step>` at `before`, the porcelain oracle lists both as locked); `worktree_capture_reports_the_new_head_or_none`; `a_no_commit_clean_worktree_is_removed_at_capture_and_a_dirty_one_is_kept` (D27); `prepare_is_idempotent_for_worktree_and_copy` (D38/A-2: two calls, one tree, the same `before_hash` even after the source moved by one commit between the calls); `a_half_made_worktree_is_remade` (H-5: delete a tracked file in the tree between the calls → the second call removes and re-adds; `before` unchanged); `a_repo_with_submodules_is_refused_for_worktree` (`.gitmodules` + a `gitlink` entry committed with `commit_as`); `copy_resets_a_dirty_source_and_labels_the_base` (D35/D47/A-2); `copy_refuses_over_the_cap`; `cleanup_removes_every_tree_and_leaves_the_repo_untouched` (criterion 13's isolator half: no entry under the root by `worktrees()` and by the porcelain oracle; a user's own linked worktree made by the test survives; each repo's `head` and `is_dirty` unchanged; `htui/<step>` branches still exist and none is checked out anywhere — every entry's `branch` line is read); `a_stale_entry_is_reported_not_pruned` (D46); `reconcile_merges_the_winner_no_ff_and_updates_the_primary_tree`; `reconcile_is_the_identity_for_a_no_commit_step`; `reconcile_refuses_a_dirty_primary`, `reconcile_refuses_a_moved_primary` (the exact sentences); `reconcile_after_a_crash_between_merge_and_the_row_is_idempotent` (H-3); `copy_reconcile_copies_the_range_then_merges` (OQ-5).

---

## 7. T6 — the engine's three missing call sites, verify, `CancelRun`, criteria

### 7.1 Surface changes

- `EngineParts`/`Engine` gain `V: Verifier + ?Sized` and `pub verifier: &'a V` after `isolator` (`engine.rs:183`); `Debug` prints it (`:222`); `fake_parts` (`:1490-1500`) names `crate::fake::FakeVerifier`. `Engine` gains `pub async fn cleanup_run(&self, run: RunId) -> Result<(), EngineError>` (D36, `pub` for milestone 6) and `pub async fn cancel_run(&self, run: RunId) -> Result<CommandOutcome, EngineError>` reached through `dispatch`.
- `command.rs`: `Command::CancelRun { run: RunId }`; `CommandOutcome::Cancelled { rest: Rest }`; `pub fn cancel_enabled(run: &Run) -> Result<(), EngineError>` — `Queued | Running | AwaitingApproval` else `EngineError::RunStatus { expected: "queued | running | awaiting_approval" }`. The module doc's "belong to milestones 3 to 6" list (`:28-30`) drops `CancelRun`.
- `gate.rs`: `SettleInput.verify_outcome`'s doc (`:219`) → *`run_step.verify_outcome`, from `verify.rs` (milestone 3, plan D30); `None` when the phase has no `verify_command` or the session did not finish (A-3).* `StepFailure::VerifyFailed`'s doc (`:131-133`) → *Produced since milestone 3.* No logic change.
- `fake.rs`: `FakeIsolator.cleanups` and `cleanups()` (F-K); `FakeVerifier { reports: Mutex<VecDeque<Option<VerifyReport>>> }` with `script_report(Option<VerifyReport>)` and helpers `FakeVerifier::pass()`, `::fail(code)`, `::unavailable(reason)`; an unscripted call answers `None` (no `verify_command`, the seeded shape). `FakeOrchestrator.verifier: FakeVerifier` public; `Orchestrate::verifier()` (F-P).
- `engine.rs:1045-1048`'s comment (D32): *`verify_failure` and `previous_diff` are milestone 4's prompt sections (MOD-4 M3 plan OQ-4/D32); their inputs — `command_run.output` and `before_hash..after_hash` — are durable since milestone 3.*

### 7.2 Stage 5, rewritten (`walk_live_step`, `:841-874`)

```
let verify = if result.is_ok() {                                     // A-3
    let primary = trees.iter().find(|t| is_primary(t.repo_id)).map(|t| PathBuf::from(&t.path));
    let remaining = phase.deadline_seconds.map(|d| (started_at + d) - self.now()).map(clamp_to_zero);
    self.parts.verifier.run(VerifyRequest { command: phase.verify_command.clone(), cwd: primary, remaining, step: step.id }).await
} else { None };
if let Some(report) = &verify {
    self.parts.store.record_command_run(NewCommandRun {
        id: CommandRunId::new(), run_step_id: step.id, box_id: self.parts.box_id,
        class: "verify".into(), command: phase.verify_command.clone().unwrap_or_default(),
        cwd: primary_or_session_cwd, status: if report.outcome == Unavailable { Failed } else { Done },
        exit_code: report.exit_code, output: Some(report.output.clone()),
        queued_at: report.started_at, started_at: Some(report.started_at), finished_at: Some(report.finished_at),
    }).await?;
}
let after = self.parts.isolator.capture(step.id, &trees).await?;     // unchanged, after verify (ANA-2 :387)
self.parts.store.record_commits(step.id, &after).await?;
… settle(SettleInput { verify_outcome: verify.as_ref().map(|r| r.outcome), … })
… finish_step(StepOutcome { verify_outcome: verify.as_ref().map(|r| r.outcome), verify_exit_code: verify.as_ref().and_then(|r| r.exit_code), … })
```

`is_primary(repo)` needs the repo rows: `WriteStore::repos(project)` (`traits.rs:489`), read once per step beside `project`. The `cwd` of the row falls back to `prepared.cwd` when there is no primary tree (§2.1).

### 7.3 Reconcile and park (D25)

```rust
/// After a step reached `done` and before the walk advances: the winner into the primary tree.
/// `Ok(None)` — carry on; `Ok(Some(rest))` — the run is parked (F-G, R-7).
async fn reconcile_done_step(&self, run: &Run, step: &RunStep) -> Result<Option<Rest>, EngineError> {
    let trees = self.parts.store.step_trees(step.id).await?;          // D16: from rows, not from memory
    match self.parts.isolator.reconcile(step.id, &trees).await {
        Ok(after) => { self.parts.store.record_commits(step.id, &after).await?; Ok(None) }
        Err(IsolateError::Refused(reason)) => Ok(Some(self.park_run(run, step, &reason).await?)),
        // `Git`/`Io` park too: the step is `done` and `fail_hard` cannot move it (H-2).
        Err(other) => Ok(Some(self.park_run(run, step, &other.to_string()).await?)),
    }
}
/// run `running -> awaiting_approval`, item `in_progress -> awaiting_approval`, then the note —
/// blueprint H-10's step → run → item order with the step already `done`.
async fn park_run(&self, run: &Run, step: &RunStep, reason: &str) -> Result<Rest, EngineError>;
```

Two call sites: `walk_live_step` on `Landing::Advance` (`:880`) → `reconcile_done_step` → `Some(rest)` returns it; `answer_gate` after `unpark` (`:389`) for `Approved | Skipped` → the same, returning `CommandOutcome::Answered { rest }`. A `Git`/`Io` error from `reconcile` (not a refusal) is **also** a park, with `reason = err.to_string()` (H-2): the step is `done`, `fail_hard` cannot move it, and a run failed for a transient `index.lock` after three retries is worse than one parked with the sentence.

### 7.4 `cleanup_run` and the terminal sites (D36)

```rust
pub async fn cleanup_run(&self, run: RunId) -> Result<(), EngineError> {
    let steps = self.parts.store.run_steps(run).await?;
    let mut trees = Vec::new();
    for step in &steps { trees.extend(self.parts.store.step_trees(step.id).await?); }
    if let Err(err) = self.parts.isolator.cleanup(run, &trees).await {
        tracing::warn!(%run, %err, "run-terminal cleanup failed; milestone 5's sweep is the retry");
    }
    Ok(())
}
```

Called after every terminal `finish_run` reachable from the engine, without touching `gate.rs`'s `GateContext` (which holds no isolator): (1) the four engine sites — `after_rejection`'s non-loopable branch (`:415`), `run_to_rest`'s `Finished` (`:556`), `refuse_capability` (`:724`), `fail_hard` (`:912`, inside `walk_step`'s `Err` arm after `fail_hard` returned `true`); (2) `walk_live_step` when `gate::apply` returns `Landing::Rest(rest)` with `rest.run.is_terminal()` — `retry_or_fail`'s exhausted budget (`gate.rs:542`); (3) `after_rejection` when `review_loop` returns `NoTarget` (`gate.rs:651`, `:675`, `:688`). `RunStatus::is_terminal` = `!is_active()` (`run.rs:53`). Never on `resting()` (a rest this call did not write).

### 7.5 `CancelRun` (D45)

`cancel_run`: read the run → `cancel_enabled` → `now` → for every step in `run_steps` whose status is not terminal (`StepStatus::is_terminal`, `run.rs`), `transition_step(step, status, Cancelled, now)` (every non-terminal step can move there: `run.rs:105-125`; `Ok(false)` ignored, D17) → `finish_run(run, Cancelled, None, now)` (item `queued | in_progress | awaiting_approval → open`, `traits.rs:898-905`) → `cleanup_run` → `Rest { run: Cancelled, position: <cursor's position before the moves>, failure: None }`. A `queued` run has no steps and was never claimed; `finish_run` admits `queued → cancelled` (`run.rs:60-68`). "Only if no session is live" is true by construction this milestone and is stated on `cancel_run`'s doc for milestone 6.

### 7.6 Conformance (`src/conformance.rs`, `CASES` 15 → 18)

`verify_fail_settles_failed` (**first failing test** — `E0599 verifier`): `update_phase(prd, PhasePatch { gate: Never, verify_command: Some("cargo test"), retry_limit: 0 })`, `verifier().script_report(FakeVerifier::fail(1))` → run `failed` with `failure == "verify_outcome: fail"` (`StepFailure::run_failure_text`), step `(0,1)` `failed`, `verify_outcome == Some(Fail)`, `verify_exit_code == Some(1)`, `command_runs(step)` has one row with `class == "verify"`, `status == Done`, `exit_code == Some(1)`. `verify_unavailable_never_fails`: `script_report(FakeVerifier::unavailable("no `sh` on PATH"))` on an `always`-gated prd → parks with the step `awaiting_approval` (settle `Ok`), `verify_outcome == Some(Unavailable)`, `verify_exit_code == None`, the row's `status == Failed` and `output == "no `sh` on PATH"`. `cancel_cleans_up_once`: start (parks at prd) → `CancelRun` → run `cancelled`, every step `cancelled`, item `open`, `isolator().cleanups() == 1`; a second `CancelRun` → `EngineError::RunStatus`. Pins: `cases_are_unique_and_eighteen` (`conformance.rs:1749`), `cases_len_is_eighteen` (`tests/fake_conformance.rs:14-17`). The `CASES` doc paragraph (`:126-136`) gains one sentence for the three.

Engine unit tests: `a_finished_run_is_cleaned_up_once` (criterion 1's walk ends with `cleanups() == 1`), `a_reconcile_refusal_parks_the_run_with_the_step_done` (a `FakeIsolator::refuse_reconcile(reason)` FIFO, the `prepare` shape: run `awaiting_approval`, item `awaiting_approval`, step `done`, note contains the reason), `a_hard_failure_still_cleans_up`.

### 7.7 `tests/gix_isolator.rs` (criteria 11, 12, 13 end to end)

Harness: `MemStore::demo()` + two `create_repo`s on `ids::PROJECT_HTUI` (`core` primary, `docs`) + `upsert_repo_box_path` per repo for `ids::BOX` + `update_project` setting `default_isolation` per case + `GixIsolator::new(IsolatorConfig { repos: from the rows, scratch_root: tmp/trees, … })` + `ShellVerifier::new(&BTreeMap::from([("verify", 1)]), …)` + `FakeGraphSource` + `FakeDriver` through `FakeOrchestrator::driver_for` + `TestClock` + a `SessionSink` that writes the scripted document (the fake's `after_done`) — assembled as `EngineParts` directly (`engine.rs:169-206`'s fields are public). Each git-backed case opens with `skip_without_git!()`.

- `criterion_11_a_worktree_step_on_two_repos` (`:2115`): `StartRun { HTUI_FEAT_3, Manual, Some([core, docs]) }` → after the prd step parks: `step_trees(step)` has two rows with `mode == Worktree`, both paths under `tmp/trees` and under neither checkout, `step_commits(step)` two rows with `before_hash == HEAD` of each repo, `run_steps` shows `isolation_path == Some(core's tree path)` (D33); the porcelain oracle in each repo lists the tree with `branch refs/heads/htui/<step_id>` and `locked htui run <run_id>`; `Repository::worktrees()` agrees.
- `criterion_12_a_local_step_on_a_dirty_tree` (`:2118`, record half; **no git**): `default_isolation: Local`, a modified tracked file in `core` → the tree row has `dirty == true`, `base_ref == HEAD`, the commit row `before_hash == HEAD`. The sweep's refusal is milestone 5's and is not asserted.
- `criterion_13_cancel_removes_every_tree` (`:2120`): as criterion 11 with a scripted commit in the worktree between prepare and capture (a `SessionSink` that `commit_file`s into `prepared.cwd/core`), approve prd so `reconcile` merges, park at plan → `CancelRun` → no entry under the root by either oracle, `tmp/trees/<run>` gone, each checkout's `head` is what it was (`core`'s is the merge commit made by the approved step, asserted explicitly) and `is_dirty` unchanged, and no repo has an `htui/` branch checked out (every `worktree list` entry's `branch` line and each main tree's `HEAD` symbolic target).
- `a_verify_command_runs_in_the_primary_tree`: `verify_command: Some("test -f f")` on a `worktree` prd → `verify_outcome == Some(Pass)`, the `command_run.cwd` is the `core` tree.

---

## 8. Data flow — one `worktree` step over two repos, then its reconcile

Statuses after each call; `R` run, `S` step, `P` the primary checkout's `HEAD`. `t` the test clock.

| # | Call | Effect |
|---|---|---|
| 1 | `transition_step(Pending → Running)` | S running |
| 2 | `GixIsolator::prepare(run, step, [core, docs], Worktree)`: validate root; per repo `head` (`gix`), `has_submodules` (`gix`), `git worktree add --lock --reason "htui run R" -b htui/S <root>/R/S/core P` (with_retry), post-condition `head(tree) == P` + `worktree_by_path` locked; same for `docs` | two trees, two locked entries, two branches at their bases; `cwd = <root>/R/S/` |
| 3 | `upsert_step_tree(S, [core row, docs row])` | two rows; `run_step.isolation_path = <root>/R/S/core` |
| 4 | `record_commits(S, [(core, P, None), (docs, D, None)])` | two rows |
| 5 | stage 3, stage 4 with `SessionSpec { cwd: <root>/R/S/, extra_dirs: [] }` | the agent commits `c1` in `<root>/R/S/core` on `htui/S` |
| 6 | `verifier.run({ command, cwd: <root>/R/S/core, remaining })` → `Pass`; `record_command_run(status: Done, exit_code: 0)` | one `command_run` row |
| 7 | `capture(S, rows)`: `head(core tree) = c1 ≠ P` → `Some(c1)`; `head(docs tree) = D` → `None`, `!is_dirty` → `git worktree remove --force --force <docs tree>` (D27) | docs tree gone, its branch stays |
| 8 | `record_commits(S, [(core, P, Some(c1)), (docs, D, None)])`; `settle` → `Ok`; `finish_step(verify_outcome: Pass, verify_exit_code: 0)` | |
| 9 | gate `never` → `transition_step(Running → Done)`, `Landing::Advance` | S done |
| 10 | `reconcile_done_step`: `step_trees(S)`; core: `branch_target(htui/S) = c1`, `!is_dirty(P's checkout)`, `head == P`, `git merge --no-ff --no-edit -m "htui: reconcile S" c1` (with_retry), `head_parents == [P, c1]` → `M`; docs: `branch_target = D == base` → identity | P's checkout at `M` |
| 11 | `record_commits(S, [(core, P, Some(M)), (docs, D, None)])` | ANA-2 `:987-988` |
| 12 | next position's `prepare` branches `htui/S2` from `M` | ANA-2 `:948-953` |
| … | run reaches `Finished` → `finish_run(Done)` → `cleanup_run`: `worktree remove` for core's tree, `remove_dir_all(<root>/R)`, `worktrees_under` empty | criterion 13's shape for `done` |

Rejection at 10 (`dirty_primary_tree`): `park_run` → R awaiting_approval, item awaiting_approval, note `isolation refused: dirty_primary_tree`; S stays done; R-7.

---

## 9. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | **Crash between `git worktree add` and `upsert_step_tree`** (`engine.rs:791-801`): a locked entry and `htui/<step>` exist, no row does; a naive second `prepare` hits `fatal: a branch named … already exists` (exit 255). | D38's reuse (§6.3): `.git` present + branch target = tree `HEAD` + clean → reuse, `before` from the branch. `prepare_is_idempotent_for_worktree_and_copy`. |
| H-2 | **A `reconcile` failure lands on a `done` step**: `fail_hard` moves `running → failed` only, and `done → failed` is illegal (`run.rs:121`). An escaped `?` would leave the run `running` with every step `done` — `cursor` says `Finished` and the next pass would `finish_run(Done)` over an unmerged primary. | §7.3: every `reconcile` error parks through `park_run`; `record_commits` runs only after `Ok`. |
| H-3 | **Crash between `git merge` and `record_commits`**: the primary is at merge commit `M`, the row says `c1`; a re-run of `reconcile` would refuse `primary_moved: M`. | §6.6: `head_parents == [before, after]` is read as "already merged" and `M` is returned. `reconcile_after_a_crash_between_merge_and_the_row_is_idempotent`. |
| H-4 | **Crash between `capture` and `record_commits(after)`** after D27 removed a clean no-commit worktree: the tree path is gone, the row still names it. | §6.4: a missing `worktree` path reads `branch_target` instead of `head(path)`; a missing `copy` path is `Refused` (a copy is never removed at capture). Milestone 5's reset of such a step needs the branch, which survives (`ledger :705`). |
| H-5 | **A `worktree add` killed mid-checkout** (timeout, `SIGKILL`) leaves an admin entry, a branch and a partial working tree that `gix::open` accepts. | §6.3's reuse test includes `!is_dirty(tree)`; a dirty half-tree is removed and re-added. `a_half_made_worktree_is_remade`. |
| H-6 | **A `copy_tree` interrupted before `reset_hard`/`create_branch`** leaves a directory with `.git` that D38's "exists with a `.git` entry" would reuse. | §6.3: copies are made into `<name>.partial` and renamed last; a `.partial` on entry is deleted. |
| H-7 | **`merge --abort` itself fails** (a second `index.lock`, a hook): the primary is left with `MERGE_HEAD` and stage entries; the next step's `worktree add` still succeeds and the user's checkout is mid-merge. | `abort_merge` is inside `with_retry`; a final failure is `Git("git merge --abort: …")`, which parks the run with that sentence (H-2), and the note tells the human the primary is mid-merge. Never silent. |
| H-8 | **Exit 1 is both "conflict" and "`index.lock` held" for `git merge`** (ledger `:707-708`), and the lock case leaves `MERGE_HEAD` too. | §3.5: the lock signature is consulted first, `abort_merge` runs before the sleep, and the conflict branch is the fallback. `a_held_index_lock_inside_merge_is_aborted_and_retried`. |
| H-9 | **Count-pin and cross-reference drift**: `CASES` 49 at `mem_store.rs:36` and `pg_conformance.rs:19`; orch `CASES` 18 at `conformance.rs:1749` and `fake_conformance.rs:14`; `every_cross_referenced_test_name_exists` (`conformance.rs:6608`) fails both when a named `pg_criteria` fn is missing and when a `PENDING` entry becomes defined. | T1 (a) adds the `PENDING` entry, T1 (b) removes it; T6 (a) moves both orch pins to 17 and T6 (c) to 18. |
| H-10 | **`UsageSpy` and `SpyStore` are exhaustive `WriteStore` impls** (`conformance.rs:673-1039`, `recorder.rs:353-733`); a missing arm breaks `-p htui-agent --all-features`. | T1 (a) carries both arms; T1's gate builds `htui-agent`. |
| H-11 | **Paths are compared as strings and stored as `String`** (`RunStepTree.path`): a symlinked checkout, a relative `local_path`, a `\\?\` prefix on Windows, or `git`'s own spelling of a worktree base (`gitdir` file) all differ from ours byte-wise. | `real.rs` canonicalises every path at `new` and at `prepare`; `worktree_by_path` canonicalises both sides; rows store the canonical form. `git_env_is_scrubbed` runs the test repo through a symlinked `TempDir` on macOS-shaped boxes (`/var` → `/private/var`). |
| H-12 | **`default_isolation` is `Worktree` for every phase** (`kind.rs:256`, `:283`): `prd` and `plan` get a full checkout each, at `git`'s cost, on every run of every demo project with a repo. | Not changed (the plan's mode table stands). D27 removes the clean no-commit trees at capture, so no litter; the cost is noted for milestone 6's settings UI. |
| H-13 | **Row order**: `Prepared.trees` is scope order, `step_trees` is `repo_id` order (`pg/read.rs:716`, `mem.rs:128`). `reconcile` and `cleanup` receive the latter. | Nothing in `real.rs` indexes by position; every per-row action keys on `repo_id`. `cwd` for `local`/`shared_serialized` uses `is_primary`, not "first". |
| H-14 | **A seventh generic on `EngineParts`** breaks every constructor: `fake_parts`, the engine's own `Harness`, `tests/review_loop.rs`, `tests/fixtures.rs`. | T6 (a) is one commit across all of them; `cargo test -p htui-orch --all-features` is the check. |
| H-15 | **`shared_serialized`'s guard leaks on a step that never reaches `capture`** (a `fail_hard` path skips stage 5) and would block the next step of the same run. | A-1: `cleanup(run)` releases every guard held by that run's steps, and `fail_hard` → `finish_run` → `cleanup_run` always follows (§7.4). `cleanup_releases_a_lock_a_crashed_step_never_captured`. |
| H-16 | **`CREATE_NO_WINDOW` is `pub(crate)` in `htui-agent`** (`launch.rs:54`). | F-I: one constant in `git.rs`, shared with `verify.rs`. |
| H-17 | **`gix::Repository` is not `Sync`** (`types.rs:148`); a helper that returns one to an `async fn` makes the `IsolatorFuture` non-`Send` and fails at the trait bound with an error far from the cause. | §3.4: every `gix` function takes `&Path` and returns owned values; the review greps `real.rs` for `gix::Repository` (must be zero). |
| H-18 | **`is_dirty()` is `O(tree)`** and runs at every `local`/`shared_serialized` `prepare`, every `worktree` capture and every non-identity `reconcile` on the maintainer's own checkout. | Excludes are honoured (`status` pulls `excludes`); T5's close-out records the measured time on this repo (plan Risks). |
| H-19 | **`after_hash` becomes the merge commit after reconcile** (ANA-2 `:987`), so criterion 7's "identical `after_hash`" across implement attempts can only fire for two no-commit attempts (`None == None`) in `worktree`/`copy` mode; a real repeat of the same diff yields distinct merge commits. | Not this milestone's to change (the fake pins criterion 7). Recorded for milestone 4's no-progress predicate, which may prefer the pre-merge hash — R-8. |
| H-20 | **`git`'s inherited config bites**: `safe.directory` refuses a checkout owned by another user (`fatal: detected dubious ownership`), `core.hooksPath` runs hooks with our scrubbed env, `merge.conflictstyle`/`merge.renames` change the conflict set. | Every case degrades to a `Git("git <verb>: <fatal line>")` the operator reads; hooks are the repo owner's (plan Risks). Nothing is overridden with `-c`. |
| H-21 | **`GIT_OPTIONAL_LOCKS`, `GIT_ADVICE`** are set on a `git` that may predate them (2.33 knows the first, not the second). | Both are ignored by an older `git`; neither changes an exit status. |
| H-22 | **The `verify` semaphore is per `ShellVerifier`**; a harness that built one per dispatch would serialise nothing. | `FakeOrchestrator` owns one `FakeVerifier`; milestone 6 owns one `ShellVerifier` per process and the doc on `ShellVerifier::new` says so. |
| H-23 | **`command_run.cwd` and `command` are `NOT NULL`** (`0001:542-543`); an `unavailable` report for `no primary tree` has no tree to name. | §7.2: `cwd` falls back to `prepared.cwd`; `command` is the phase's text (present whenever a report exists). |
| H-24 | **`cargo doc --workspace --no-deps` exits 101 at HEAD** (CLEAN-3's six `htui-store` errors); a new `htui-orch` doc error is invisible in the exit code. | C-9 greps the output for `htui_orch`/`htui-orch`; zero allowed. |
| H-25 | **`.sqlx` regeneration needs the `htui_sqlx` database** and `SQLX_OFFLINE=true` in `.cargo/config.toml` hides a stale file until `--check` runs. | T1's gate ends with `cargo sqlx prepare --check`; the three new files are committed with their queries. |
| H-26 | **A `TempDir` under `/tmp` and a repo under `$HOME`** may be on different filesystems; `rename(tree.partial, tree)` is same-directory and fine, but a test that points `scratch_root` at a different mount from the repo is a valid production shape and `copy_tree` must not assume reflinks. | `copy_tree` is byte copying; `a_symlink_is_copied_as_a_symlink` and the cross-mount case are the same code path. |

---

## 10. Windows notes for MOD-16

Every place this milestone's behaviour is platform-dependent, for the MOD-16 verifier (PRD `:296`). None is guarded here beyond what is stated.

| Surface | Unix here | What MOD-16 must verify |
|---|---|---|
| Locating the binary | `which::which("git")` → `/usr/bin/git` | `git.exe` through `PATHEXT` (`which` handles it, `probe.rs:339-347`); Git for Windows' `cmd/git.exe` versus `bin/git.exe` both answer `--version`; the version string is `2.47.1.windows.1` (§3.3 step 3 parses it) |
| Child creation | `ProcessGroup::leader()`; a timeout kills the group | `CreationFlags(CREATE_NO_WINDOW)` + `JobObject` (F-I); a timeout kills the job; the refused-job-object fallback logs and continues (`launch.rs:1146-1158`) |
| Paths | `/`; `RunStepTree.path` is the canonical path | `\` and drive letters; `std::fs::canonicalize` yields `\\?\C:\…` — the same prefix must be applied to both sides of every `starts_with` (H-11), and `git` writes its `gitdir`/`worktree list` paths **without** the prefix and with `/` separators, so `worktree_by_path` must compare after normalising both; `MAX_PATH` (260) is reachable under `%APPDATA%\htui\trees\<run uuid>\<step uuid>\<name>\…` — `core.longpaths` or the `\\?\` form is the fix |
| Scratch root | `~/.config/htui/trees` | `%APPDATA%\htui\trees` (`identity.rs:36-39`); on a roaming profile this is synced — a worktree in a roaming directory is a MOD-16 question |
| `worktree remove --force --force` | removes a dirty locked tree | fails with `Permission denied` while any process holds a file open in the tree (an agent, an AV scanner, a `cargo` build); the entry survives and §6.5 reports it as stale; milestone 5's sweep is the retry |
| `core.autocrlf` / filters | not set | with `autocrlf=true`, `git`'s checkout writes CRLF while `gix`'s `is_dirty()` compares through its own filter pipeline; a false `dirty = true` on a fresh worktree would make D27 keep every no-commit tree — verify `is_dirty()` is `false` right after `worktree add` on an `autocrlf=true` repo |
| Hooks | `post-checkout` runs on `worktree add` (verified) | it runs through `sh.exe` from Git for Windows; a hook that needs `bash` on `PATH` fails the verb with a `Git(...)` sentence |
| `LC_ALL=C` | pins English | honoured by Git for Windows' gettext; verify the three D39 phrases and the conflict lines are byte-identical |
| `verify_command` | `sh -c` | `cmd /C`; quoting rules differ (a `&&` chain works, a `'` does not); the `verify` templates seeded by MOD-15 are Unix-shaped |
| Symlinks in `copy_tree` | re-created | `symlink_file`/`symlink_dir` need Developer Mode or elevation; on refusal `copy_tree` should copy the target's content instead and `warn` — decided by MOD-16, not here |
| Reflink / `copy` cost | none on ext4 either | full byte copy on NTFS (ANA-2 `:928-933`); the cap and the measured size are the guard |
| Signals | `code() == None` means a signal | never `None` on Windows; the "killed by signal" class is unreachable there |
| Case-insensitive filesystem | n/a | `htui/<uuid>` branch names are lowercase and unique; two repos whose `name`s differ only by case collide as directories under `<root>/<run>/<step>/` — `duplicate_repo_name` should compare case-insensitively on Windows |

---

## 11. What this milestone does NOT do

An implementer who finds themself writing any of these has drifted into milestone 4 or 5:

- **Fan-out**: no `fanout.rs`, no `select.rs`, no judge step, no `fanout_index != 0`, no per-candidate trees; `Prepared.trees` is one row per repo (plan `:212-213`). ANA-2 §4.5 is read only for what it forwards to milestone 4 (H-19).
- **Overlap and admission (R-2)**: no `overlap.rs`, no `RunScope`, no rule L for `local` (ANA-2 `:917`, `:1082-1087`), no advisory lock, no change to `claim_run` (D44). The `(box, repo)` mutex of D43 serialises siblings inside one process and nothing else.
- **The recovery sweep**: no `recover.rs`, no lease refresh, no artefact test, no tree reset of an interrupted step (ANA-2 §4.9, criterion 12's second half, criterion 18); `dirty = true` is recorded and never acted on.
- **Resume of a reconcile-parked run** (R-7) and **`Unblock`**, **`CancelStep`**, **`PromoteStep`**, **`SelectFanout`**, **`AcceptArtifact`**, **`CloseOut`**: `Command` gains `CancelRun` only.
- **The prompt sections** `verify_failure` and `previous_diff` (D32): both stay `None`; no diff renderer; `similar` stays `htui-agent`'s.
- **A second `shared_serialized` sibling's reset** (D29): a dirty tree is recorded, never reset, never refused.
- **`git worktree prune`**, `git gc`, `git fetch`, any network `git`, `git2`: never (D46; OQ-5; ANA-2 `:940`, `:2055`).
- **`command_run` as a queue**: no `queued`/`running` rows, no MOD-11 enqueue path; `record_command_run` writes finished rows only.
- **TUI wiring**: nothing under `crates/htui/**`; `GixIsolator`, `ShellVerifier`, `cleanup_run` and `CancelRun` have no caller outside `htui-orch` and its tests until milestone 6's `run_worker.rs`.
- **A migration or a column**: `0004` does not exist (PRD `:272`); `isolation_path` and `command_run` are `0001`'s.

---

## 12. File sets

**T1** — gate per §1. `crates/htui-core/src/model/{run.rs, mod.rs}`; `crates/htui-core/src/store/{traits.rs, mem.rs, conformance.rs}`; `crates/htui-core/tests/mem_store.rs`; `crates/htui-store/src/pg/write.rs`; `crates/htui-store/.sqlx/` (three new files); `crates/htui-store/src/writer.rs`; `crates/htui-store/tests/{pg_conformance.rs, pg_criteria.rs}`; `crates/htui-agent/src/conformance.rs`; `crates/htui-agent/tests/recorder.rs`. **Not** `pg/rows.rs` (F-N), **not** any `writer_buffered.rs` (F-A).

**T2** — `Cargo.toml` (`:65` comment, after `:101`); `crates/htui-orch/Cargo.toml`; `crates/htui-orch/src/isolate.rs` (`:24`, `:79`, `:93-97`); `crates/htui-orch/src/isolate/git.rs` (create); `crates/htui-orch/src/fake.rs` (`:182-185`).

**T3** — `crates/htui-orch/src/verify.rs` (create); `crates/htui-orch/src/lib.rs`.

**T4** — `crates/htui-orch/src/isolate/copy.rs` (create); `crates/htui-orch/src/isolate.rs` (one `pub mod copy;` line).

**T5** — `crates/htui-orch/src/isolate/real.rs` (create); `crates/htui-orch/src/isolate.rs` (`pub mod real;`, the re-export, the `reconcile` doc); `crates/htui-orch/src/lib.rs` (the three names).

**T6** — `crates/htui-orch/src/{engine.rs, command.rs, gate.rs, fake.rs, conformance.rs}`; `crates/htui-orch/tests/{fake_conformance.rs, gix_isolator.rs (create), review_loop.rs, fixtures.rs}` (the last two only where `EngineParts` is built, H-14). Then the full workspace gate and `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

**Not touched, on purpose**: every `.snap`; `crates/htui/**`; `crates/htui-core/Cargo.toml` and `crates/htui-store/Cargo.toml` (invariant 10: `gix` enters `htui-orch` only — a zero diff on both is part of C-6); `claim_run`; `docs/ANA-2.md` (already amended, F-O); `crates/htui-core/src/model/kind.rs`; `crates/htui-agent/src/**` except the spy arm.

**Gate checks that are not tests (`C-n`, continuing milestone 1's series; C-5 stays carried — a fixture `repo` in `htui-core` is still not added, and `tests/gix_isolator.rs` creates its repos through the seam at run time)**:

- **C-6** `cargo tree -p htui-orch -i gix --edges features` shows the five features of §3.1 and no `gix-merge`, `gix-worktree-state`, `gix-transport`, `gix-protocol`, `gix-credentials`, `gix-negotiate`; `Cargo.lock` has no `git2`/`libgit2`; `git diff --stat -- crates/htui-core/Cargo.toml crates/htui-store/Cargo.toml` is empty.
- **C-7** `grep -rn 'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` and `verify.rs` only; `grep -rn '"prune"' crates/htui-orch/src/` is empty; `grep -rn 'gix::Repository' crates/htui-orch/src/isolate/real.rs` is empty (H-17).
- **C-8** `git --version` on the gate box is 2.43.0, so `cargo test -p htui-orch --all-features -- --nocapture 2>&1 | grep -c 'skipped: git'` is `0` there; on a box with `PATH` stripped of `git` the same count equals the number of git-backed tests and the suite is green.
- **C-9** `cargo doc --workspace --no-deps 2>&1 | grep -c 'htui[-_]orch'` is `0` (H-24).

**Carried out of this milestone**: R-3, R-4, R-5, R-6 (unchanged from milestone 2); **R-7** — a run parked by `park_run` has no resume verb (F-G); **R-8** — milestone 4's no-progress predicate reads `after_hash` after reconcile rewrote it to the merge commit (H-19); C-5.
