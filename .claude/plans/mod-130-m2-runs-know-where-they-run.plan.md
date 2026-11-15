# Plan: MOD-130 M2 — Runs know where they run

**Source PRD**: `.claude/prds/mod-130-execution-targets.prd.md`
**Selected Milestone**: 2 — Runs know where they run
**Complexity**: Medium-Large
**Status**: complete (`c52e87be`..`8b222d18`, 2026-11-15); review fix round R1 applied; sandbox gate green but for `cache::the_spawned_refresher_passes_and_follows_the_scope`, the known load flake (alone, alternating: base `3e7b7d38` 2/4, HEAD 2/4).
**Routing**: PRD path, ultracode for implement (C2, C4) (`/handoff-run` in the `hr/MOD-130` sandbox,
2026-11-15). The plan runs without a confirm gate (handoff-run step 3). No architect: M2 adds no module
seam beyond what the PRD and M1 fixed (C2 for this milestone is the resolver and the run snapshot, both
specified below).

## Summary

After M2 every graph run records **where it runs and why**:

- **Resolver.** A pure function in `htui-core` resolves the cascade item → workflow → project →
  workspace → Settings → built-in local target, and names the level that answered (`TargetSource`).
- **Run snapshot.** `Engine::enqueue_with` resolves the target and writes `run.target_id`,
  `run.target_source` and `run.target_box_id` (where the work runs) through `NewRun.target`.
  A later cascade edit never moves a queued or running run (D7).
- **Run-start refusals.** A retired target, a kind this build cannot run (`docker` → MOD-44, `ssh`/`ssh_docker`
  → MOD-131, `enrolled` → MOD-47), a target bound to another box (D10) and a runnable non-local target that
  is not `ready` are refused at enqueue with a sentence. No run is written. The item gets the note and is
  blocked, the same as for missing tags.
- **Chat runs unchanged** (D17): they carry no target.

Only `local` runs in this build, so every run that is written still runs on this box, as today.

## Shared decisions

Every task's brief carries this section.

### Decisions from the PRD gate (2026-11-14) that M2 implements

| D | What M2 does with it |
|---|---|
| D1 | Every non-local kind resolves, but run start refuses it with a sentence naming the item that brings it (`TargetKind::owner`). |
| D2 | The seeded `LOCAL_TARGET_ID` (`crates/htui-core/src/seed.rs`) is the answer when no level names a target; its run's `target_box_id` is this box. |
| D3 | `NULL` = inherit at every level. The Settings level is the `app_setting` row `default_target` (`DEFAULT_TARGET_KEY`); absent → source `default`, target local. |
| D5 | The workspace level is the scope `project` row's `target_id`. For an item filed on a scope row (`project.owner_workspace_id = Some(ws)`), its project row **is** the workspace level: it answers as `workspace`, and there is no separate project level. |
| D6 | For any other item, the workspace level is the scope row of the run's `workspace_id` (the `workspace` argument of `enqueue_with`). `None` skips the level. |
| D7 | `run.target_id` and `run.target_source` are written at enqueue, never updated. Retry and resume keep the row, so they keep both. |
| D10 | A target bound to a box that is neither this box nor a child of this box is refused at run start, naming the box. |
| D11 | The workflow level is `step_graph.target_id` of the graph the run resolves (the item's override, else its kind's default: `ResolvedGraph.graph`). |
| D17 | `start_chat_run` is untouched; a chat run's `target_id`/`target_source` are `NULL`. |

D8 (the driver in `executing_box_id` at enqueue), D9 (tags and candidates against the target box) and the
claim/poll/queue re-keying are **M3**; see P1.

### Decisions this plan takes

- **P1 — `executing_box_id` is not written at enqueue in M2.** The PRD milestone row says M2 "names the
  driving box". But D8's write changes what every reader of `executing_box_id` sees for a queued run
  (`run_hostname`, the Runs pane, slot/overlap counts, the activity triggers). The PRD's own risk row puts
  that audit in M3. In M2 only `local` runs, so the driver is always this box and nothing is lost by
  waiting: M3 adds the write together with the claim/poll re-key and the audit. `create_run` keeps writing
  `executing_box_id = NULL`.
- **P2 — Order inside `enqueue_with`.** The order stays `start_enabled` → missing tags (against
  `self.parts.box_id`, unchanged) → `graph::resolve` (rung 4 unchanged). Then come **target resolution and
  its refusal**, then `create_run`. The workflow level needs the resolved graph, and `graph::resolve`
  already reads it. M3 decides whether to move target resolution before the tags check when tags switch to
  the target box (D9).
- **P3 — `Resolved` carries the workflow level.** `graph::Resolved` gains
  `pub workflow_target: Option<TargetId>`, filled from `ResolvedGraph.graph.target_id` in
  `resolve_or_refused`'s resolved arm. Every `Resolved { .. }` literal gets the field. The snapshot JSON is
  unchanged: the target is a run column, not a snapshot field.
- **P4 — Two new `WorkerStore` reads** (the engine's store bound is `WorkerStore`, which has neither):
  - `scope_project(&self, workspace: WorkspaceId) -> Result<Option<ProjectId>>`: the same answer as
    `ReadStore::scope_project`. Implement it by delegating to the existing inherent or `ReadStore` body on
    `MemStore` and `PgStore`, and add it to `Writer`'s `WorkerStore` dispatch.
  - `target_binding(&self, id: TargetId) -> Result<Option<TargetBinding>>`: the target row, the `box` row its
    `box_id` names (`bound`), and that box's parent row when it is a container, else the bound row itself
    (`host`). `None` when no target has the id. The Pg body is one statement for the target plus the
    existing box-row reader for the box ids; it takes no locks.
  - Implementors are `MemStore` (`crates/htui-core/src/store/worker.rs`), and `PgStore` and `Writer`
    (`crates/htui-store/src/worker.rs`). There are no others (verified claim C5).
- **P5 — The Settings level comes from `self.parts.app`**, the `app_setting` map the engine already
  carries (`PgStore::app_settings` returns every row; `Kit::read` re-reads it for every command and
  sweep, `htui-worker/src/runtime.rs:1177`). A new pure helper
  `default_target_in(app: &BTreeMap<String, Value>) -> Result<Option<TargetId>, StoreError>` repeats
  `DefaultTarget::parse`'s rule (`as_str` → `TargetId`, else `StoreError::Backend` with the same
  sentence naming the key). It cannot call `parse`, which needs an `updated_at` the map does not carry.
  Factor the shared sentence into one private fn so the two cannot drift.
- **P6 — Model, in `crates/htui-core/src/model/target.rs`** (re-exported from `model/mod.rs` like M1's
  names):

  ```rust
  str_enum!(/// `run.target_source` (MOD-130 M2, D7): the cascade level that answered.
      TargetSource { Item => "item", Workflow => "workflow", Project => "project",
                     Workspace => "workspace", Settings => "settings", Default => "default" });

  /// The cascade's levels, each `None` = inherit (MOD-130 M2, D3).
  #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
  pub struct TargetLevels { pub item: Option<TargetId>, pub workflow: Option<TargetId>,
      pub project: Option<TargetId>, pub workspace: Option<TargetId>, pub settings: Option<TargetId> }

  /// What [`resolve_target`] answers and a run records (MOD-130 M2, D7).
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
  pub struct ResolvedTarget { pub target: TargetId, pub source: TargetSource }

  /// First `Some` in item, workflow, project, workspace, settings order; none → `LOCAL_TARGET_ID`
  /// with `TargetSource::Default` (MOD-130 M2, D2, D3). Shaped like `resolve_budget`.
  #[must_use] pub fn resolve_target(levels: &TargetLevels) -> ResolvedTarget;

  /// The rows `target_binding` reads (P4).
  #[derive(Debug, Clone, PartialEq)]
  pub struct TargetBinding { pub target: Target, pub bound: Option<BoxRow>, pub host: Option<BoxRow> }

  /// Why a run cannot start on a target (MOD-130 M2, D1, D10, D14). `Display` is the sentence.
  #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
  pub enum TargetRefusal {
      #[error("target {name} is retired; choose another target or inherit")]
      Retired { name: String },
      #[error("target {name} is {kind}, which this build cannot run yet; {owner} brings it")]
      NotRunnable { name: String, kind: TargetKind, owner: &'static str },
      #[error("target {name} belongs to box {owner}; start the run there (MOD-43) or choose another target")]
      BoundElsewhere { name: String, owner: BoxId },
      #[error("target {name} is {status}; probe it before starting a run on it")]
      NotReady { name: String, status: TargetStatus },
  }

  /// First match wins: retired; kind not in `runnable` (owner from `TargetKind::owner`);
  /// `host` (or `bound` when it is a machine) is not `here`; `status_with(runnable, ..)` is not
  /// `Ready`. `None` → the run may start (MOD-130 M2).
  #[must_use] pub fn run_start_refusal(binding: &TargetBinding, here: BoxId, runnable: &[TargetKind])
      -> Option<TargetRefusal>;

  /// The box the work runs on: `binding.target.box_id`, else `here` (local) (MOD-130 M2).
  #[must_use] pub fn runs_on(binding: &TargetBinding, here: BoxId) -> BoxId;

  /// P5.
  pub fn default_target_in(app: &BTreeMap<String, Value>) -> Result<Option<TargetId>, StoreError>;
  ```

  `htui-core` depends on `thiserror` (C9), so use the derive. The sentences above are the contract, and
  tests pin them verbatim. `TargetKind::owner()` is `Option<&'static str>` and `None` only for `local`.
  `NotRunnable.owner` takes `kind.owner().unwrap_or("a later item")`; never `unwrap`.
- **P7 — `Run` and `NewRun`.**
  - `Run` gains `pub target_id: Option<TargetId>` and `pub target_source: Option<TargetSource>`, both
    `#[serde(default)]`, documented as MOD-130 M2 D7 ("`None` on a chat run and on every run queued before
    M2").
  - `NewRun` gains `#[serde(default)] pub target: Option<ResolvedTarget>`. The engine always writes
    `Some`; `None` is for tests and is stored as `NULL`/`NULL` (`chk_run_target_pair`).
  - `Run.target_box_id`'s doc changes to: "the box the work runs on: the resolved target's box, this box
    for `local` (MOD-130 M2)".
- **P8 — `create_run` validates `new.target`, in both stores, inside the write, before the insert.** The
  check sits after the repo-scope phantom check and before the batch check:
  1. a target naming no row → `StoreError::Constraint(references_no_row("run.target_id", t, "target"))`;
  2. a retired target → `StoreError::Constraint(target_retired(&name))`.

  On Postgres, call M1's `share_level_target(&mut tx, "run.target_id", new.target.map(|t| t.target))`
  (`pg/write.rs:2088`, used by the level setters at `:4170`/`:4220`/`:4272`). It reads the target `FOR SHARE` and
  answers exactly these two refusals, so a `retire_target` racing the enqueue serialises with it. Place
  it between the phantom-repo check (`:7948`) and the batch check (`:7965`). Never surface a raw SQLSTATE.
- **P9 — Mem `delete_target` refuses on run history** (M1 P8's deferred branch): with `Run.target_id`
  present, MemStore's `delete_target` step 4 counts runs naming the target and answers
  `target_has_runs(name, n)`, as Pg does. The trait doc's "(Postgres only until M2 …)" parenthesis is
  removed.
- **P10 — The engine refusal.** A new variant
  `EngineError::TargetRefused { item: ItemId, refusal: TargetRefusal }`,
  `#[error("item {item}: {refusal}")]`, is raised by `Engine::refuse_target(&item, refusal)`. That method
  mirrors `refuse_missing_tags`: one `block_if_idle(item.id, refusal.to_string())`, then the variant. It
  is permanent until the cascade or the target changes, so it is never `ClaimRefused`. Update the doc list
  of refusals on `Engine::enqueue`.
- **P11 — Engine resolution body** (in `enqueue_with`, after `graph::resolve`):

  ```text
  project   = store.project(item.project_id)?            (NotFound if gone)
  workspace = project.owner_workspace_id.or(workspace)    (D5, D6; the store fills it the same way)
  scope_row = if project.owner_workspace_id.is_some() { None } else { workspace → store.scope_project }
  levels = TargetLevels {
      item: item.target_id, workflow: resolved.workflow_target,
      project:   if scope row item { None } else { project.target_id },
      workspace: if scope row item { project.target_id } else { scope_row → store.project(..).target_id },
      settings:  default_target_in(&self.parts.app)?,
  }
  chosen  = resolve_target(&levels)
  binding = store.target_binding(chosen.target)? → None is StoreError::NotFound { entity: "target" }
  if let Some(r) = run_start_refusal(&binding, self.parts.box_id, RUNNABLE_KINDS) → refuse_target
  NewRun { target_box_id: runs_on(&binding, self.parts.box_id), target: Some(chosen), .. }
  ```

  `Item.target_id` already exists (M1 T3). Verify its field name (C3).
- **P12 — Cache.** `crates/htui-store/src/cache/read.rs`'s `RUN_SELECT` and `run_of` read the mirror's
  `target_id` and `target_source` (the columns exist since M1's cache `0014`; C7) into the new `Run`
  fields. No cache migration.

### Conventions

- **Conformance pattern.** Each case is `async fn <name><S: WriteStore>(store: &S)` and starts with
  `const CASE: &str = "<name>";` (`crates/htui-core/src/store/conformance.rs`). Register it twice: in
  `CASES` and as an arm of `run_case_inline`. Mirror M1's `retire_and_delete_refuse_a_target_in_use`:
  `CASES` at `conformance.rs:510`, its arm at `:1620`, its body at `:40299`.
- **The case count is pinned in three places, all reading 343 before M2** (C10):
  `crates/htui-store/tests/it/pg_conformance.rs:77` (`EXPECTED_CASES`), the doc text at `:76`, the assert
  message at `:84`; and `crates/htui-core/tests/it/mem_store.rs:65`. T2 adds 3 cases, so the count
  becomes 346.
- **`htui-orch`'s own conformance suite runs on MemStore only** (C11 false), and nothing tests at the
  `WorkerStore` level (C12). Engine tests therefore go in `engine.rs`'s MemStore test module, and the Pg
  bodies of the new worker reads get `pg_criteria` tests.
- **`.sqlx`.** Every new or changed `query!`/`query_as!` needs prepared data. In the hr sandbox (no docker;
  Postgres at `localhost:5439`, user `postgres`, trust auth):
  ```
  psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
  cd crates/htui-store
  export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations
  cargo sqlx prepare -- --all-targets --all-features     # keep both flags, or test-only queries are deleted
  cargo sqlx prepare --check -- --all-targets --all-features
  ```
  A "potentially unused queries" warning from `--check` is expected. Delete the stale entries the changed
  queries leave behind: `prepare` rewrites the directory, so commit what it leaves.
- **Tests.** Integration tests are modules of each crate's single `it` binary. Run one module as
  `cargo nextest run -p <crate> --all-features -E 'binary(it) and test(/^<module>::/)'`, and unit tests
  as `cargo nextest run -p <crate> --all-features -E 'not binary(it) and test(/<filter>/)'`.
  `HTUI_TEST_DATABASE_URL` and `HTUI_TEST_QDRANT_URL` are exported in the sandbox, and live-Qdrant cases
  run normally. TDD: write each task's tests first and watch them fail.
- **Lints.** `unsafe_code = "forbid"`, the workspace lints and the MSRV are unchanged. Every new pub item
  has a doc line citing `MOD-130 M2` and the D or P number. Run
  `cargo clippy -p <crate> --all-targets --all-features -- -D warnings` and
  `cargo clippy -p <crate> -- -D warnings` (featureless) on every crate you touched, and `cargo fmt`.
- **Never run the workspace gate.** Commit incrementally, one commit per green step, with the message
  `feat(<area>): MOD-130 M2 T<n> …` ending with the attribution lines. Never stash; never push.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Resolver with a named source | `resolve_budget` (`crates/htui-core/src/prompt/settings.rs:813-833`), its tests `:1920-2012` | first `Some` wins, the source enum names the rung, table-driven test |
| str_enum + CHECK pin | `TargetKind`/`TargetStatus` and `kind_and_status_match_the_check_list` (`crates/htui-core/src/model/target.rs`) | `str_enum!`, a test pinning the CHECK list order |
| Engine refusal that blocks the item | `refuse_missing_tags` (`crates/htui-orch/src/engine.rs:1604-1616`), `EngineError::MissingTags` (`crates/htui-orch/src/command.rs:435`) | one `block_if_idle`, then the variant |
| Engine refusal tests | `enqueue_refuses_missing_tags_before_resolving` (`engine.rs:19192`), `a_startrun_refusal_writes_nothing_over_a_live_run` (`:19356`) | MemStore engine, assert error, no run, note + blocked |
| Store validation before insert | `PgStore::create_run`'s phantom repo check (`crates/htui-store/src/pg/write.rs:7870-8032`) | `references_no_row` before the write, inside the tx |
| Target `FOR SHARE` in a writer | M1's item writers (`share_level_target`, `pg/write.rs`, used at `:4272`) | read the target row `FOR SHARE`, then `target_retired` |
| Worker-trait read | `WorkerStore::project` (`crates/htui-core/src/store/worker.rs:186`), the MemStore body `:843`, the Pg and `Writer` bodies in `crates/htui-store/src/worker.rs` | `impl Future + Send` in the trait, `async fn` in the impls |

## Files to Change

| File | Task |
|---|---|
| `crates/htui-core/src/model/target.rs`, `crates/htui-core/src/model/mod.rs` | T1 |
| `crates/htui-core/src/model/run.rs` (Run, NewRun, docs, test literals) | T2 |
| `crates/htui-core/src/store/{mem,conformance,traits}.rs`, `crates/htui-core/src/fixtures.rs` | T2 |
| `crates/htui-store/src/pg/{read,write}.rs`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/.sqlx/` | T2 |
| every `NewRun { … }` and `Run { … }` literal (about 60 and 10; list with `search text "NewRun {"` / `"lease_box_id: "`) in `htui-core`, `htui-store/tests`, `htui-orch`, `htui-agent/tests`, `htui-mcp/tests`, `htui-worker`, `htui` | T2 (literals in `htui-orch/src/engine.rs` other than `enqueue_with`'s too) |
| `crates/htui-store/tests/it/{pg_conformance,pg_criteria}.rs`, `crates/htui-core/tests/it/mem_store.rs` | T2 |
| `crates/htui-core/src/store/worker.rs`, `crates/htui-store/src/worker.rs` (+ `.sqlx`) | T3 |
| `crates/htui-orch/src/{engine,command,graph}.rs` (enqueue, refusal, `Resolved.workflow_target`, tests) | T3 |
| `crates/htui-store/tests/it/pg_criteria.rs` (Pg bodies of `scope_project`/`target_binding`) | T3 |

## Tasks

The tasks run as one serial lane, T1 → T2 → T3. Each one builds on the previous task's types, and T2 and T3
share `.sqlx`, the `NewRun` literal in `enqueue_with` and the store crates. **T2 is
`verify: foundation`** (T3 builds on its schema-facing changes); T1 and T3 go to the review unverified.

### Task 1: The resolver and the refusal, pure (htui-core)

**Depends on:** nothing. **Files:** `crates/htui-core/src/model/target.rs`, `crates/htui-core/src/model/mod.rs`.

Add P6's items to `target.rs` and re-export them from `model/mod.rs` beside M1's target names.

Tests first, in `target.rs`'s `mod tests`:
- `target_source_matches_the_check_list`: `TargetSource::ALL` as strings equals
  `["item","workflow","project","workspace","settings","default"]` (the order of
  `chk_run_target_source` in `0040_execution_targets.sql`), with a `parse` round trip and serde as the
  string.
- `resolve_target_takes_the_first_level_that_names_one`: table-driven over all 32 combinations of the five
  levels set or `None`, each to a distinct id. It expects the first set level in item, workflow, project,
  workspace, settings order with the matching source, and `(LOCAL_TARGET_ID, Default)` when all are `None`.
- `run_start_refusal_follows_its_order`, with explicit runnable sets:
  - a retired local target → `Retired`, before anything else;
  - a docker target under `RUNNABLE_KINDS` → `NotRunnable { owner: "MOD-44" }`; ssh and ssh_docker →
    `"MOD-131"`; enrolled → `"MOD-47"`;
  - with `runnable = TargetKind::ALL`, a docker target whose child box's parent (`host`) is another box →
    `BoundElsewhere { owner: host.id }`, and an ssh target bound to a machine that is not `here` → the same;
  - with `runnable = ALL`, docker on a child of `here` that is unprobed → `NotReady { status: Unprepared }`;
    probed and the host has the `docker` tag → `None`;
  - local → `None`.
- `refusal_sentences_are_pinned`: each variant's `to_string()`, verbatim.
- `runs_on_is_the_bound_box_else_here`.
- `default_target_in_reads_the_setting`: absent → `None`; a JSON string id → `Some`; `json!(3)` →
  `StoreError::Backend` naming `default_target`.

**Check:** `cargo nextest run -p htui-core --all-features -E 'not binary(it) and test(/model::target::/)'`,
then clippy (both forms) and fmt on `htui-core`.

### Task 2: Runs record their target (`verify: foundation`)

**Depends on:** T1. **Files:** the T2 rows of Files to Change.

1. **Model (P7).** Add the `Run` and `NewRun` fields and change the `target_box_id` doc. Update every
   literal: `NewRun { … }` gets `target: None` (the engine's `enqueue_with` literal too; T3 sets it), and
   `Run { … }` gets `target_id: None, target_source: None`. A `..spread` literal needs nothing.
2. **MemStore.**
   - `State::create_run` validates `new.target` (P8) and stores it on the `Run`.
   - `start_chat_run` writes `None`/`None`.
   - `delete_target` refuses on run history (P9).
3. **PgStore.**
   - `create_run` gains the P8 checks (`FOR SHARE` read) and inserts `target_id`/`target_source`.
   - Every `query_as!(Run, …)` selects the two columns as `target_id AS "target_id: TargetId"` and
     `target_source AS "target_source: TargetSource"`. That covers `run`, `waiting_candidates`,
     `overlapping_runs` (`pg/read.rs`), `create_run`'s `RETURNING`, `adopt_runs` and `workspace_live_runs`
     (`pg/write.rs`), and any other found by compiling.
   - The chat-run insert stays as it is, unless it returns a `Run`; if so, it selects the two columns as
     well.
4. **Cache (P12).** Update `RUN_SELECT` and `run_of`.
5. **Conformance cases** (+3, count 343 → 346; C10):
   - `create_run_records_the_target_and_source`: a `NewRun` with
     `Some(ResolvedTarget { target: LOCAL_TARGET_ID, source: Project })`. Read it back with `run(id)`;
     both fields match. A `NewRun` with `target: None` reads back `None`/`None`.
   - `create_run_refuses_an_unknown_or_retired_target`: an unknown id gives `references_no_row`'s
     sentence. Then create a target and retire it, and that gives `target_retired`. Both write no run
     (`runs(item)` is unchanged).
   - `delete_target_refuses_a_target_with_runs`: create a target and a run naming it → `target_has_runs`.
     Keep M1's Pg-only `pg_criteria` test `delete_target_refuses_one_with_run_history`
     (`pg_criteria.rs:~15372`).
6. **`.sqlx`**: prepare as in Conventions.

**Check:**
- `cargo nextest run -p htui-core --all-features -E 'binary(it) and test(/^mem_store::/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and (test(/^pg_conformance::/) or test(/^cache::/))'`
- `cargo check --workspace --all-targets --all-features`, since the literals span every crate
- the sqlx `--check`, then clippy (both forms) and fmt on the crates touched.

### Task 3: `enqueue` resolves, snapshots and refuses

**Depends on:** T2. **Files:** the T3 rows of Files to Change.

1. **Worker reads (P4).** Add `scope_project` and `target_binding` to `WorkerStore`, with bodies on
   `MemStore`, `PgStore` and `Writer`, plus `.sqlx` for the Pg target read. `Writer`'s body delegates
   to its existing `scope_project` (`crates/htui-store/src/writer.rs:360`). Add `pg_criteria` tests
   (there is no `WorkerStore` conformance path, C12): `target_binding` on Pg returns the target, the bound
   child box (made with `attach_child_box`) and its parent machine, and `None` for an unknown id; the
   worker `scope_project` answers as the `ReadStore` one.
2. **`Resolved.workflow_target` (P3)**, filled in `graph.rs`, with every `Resolved { .. }` literal updated.
3. **Engine (P10, P11).** Add `EngineError::TargetRefused`, `refuse_target`, and the resolution in
   `enqueue_with`. Extend `command.rs`'s `the_named_refusals_say_what_went_wrong` with the new variant's
   `Display`.
4. **Engine tests** (MemStore engine, `engine.rs`'s test module; mirror
   `enqueue_refuses_missing_tags_before_resolving` at `engine.rs:19164`):
   - `enqueue_snapshots_the_local_default`: no level set → the run has `LOCAL_TARGET_ID`, `Default`, and
     `target_box_id` = this box.
   - `enqueue_names_the_level_that_answered`: one case each, with the `local` target as the value (the
     only runnable one):
     - item (`set` through the item's spec/version path);
     - workflow (`set_graph_target`);
     - project (`set_project_target`);
     - workspace (the run's workspace scope row);
     - a scope-row item, which answers as `workspace`;
     - settings (`set_default_target`, then rebuild the engine's `app` map).

     Each run records the expected source.
   - `a_cascade_edit_moves_no_queued_run`: enqueue, change the project's target, and the queued run's
     target and source are unchanged.
   - `enqueue_refuses_a_target_this_build_cannot_run`: create a docker target and set the project level to
     it. `enqueue` → `EngineError::TargetRefused { refusal: NotRunnable { owner: "MOD-44", .. } }`, no run
     written, the item `blocked` with the sentence as its note.
   - `a_target_refusal_writes_nothing_over_a_live_run`: mirror `:19356`.
   - Chat runs: assert an existing chat-run test's run has `target_id == None` (a one-line addition).
5. **`Resolved` literal.** The only one is `graph.rs:852`. `graph.name` is moved into `SnapshotGraph`
   there, but `graph.target_id` is `Copy`, so read it at the literal.

**Check:**
- `cargo nextest run -p htui-orch --all-features -E 'test(/enqueue|target/)'`
- `cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^fake_conformance::/)'`
- `cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^pg_criteria::/)'`
- the sqlx `--check`, then clippy (both forms) and fmt on `htui-core`, `htui-store` and `htui-orch`.

## Validation (main thread, once for the lane)

```bash
bash scripts/gate.sh
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| About 70 struct literals across 7 crates break the build mid-task | High | T2 does them mechanically in one commit and runs `cargo check --workspace --all-targets --all-features` |
| A Run SELECT missed; `query_as!` fails at compile | Medium | The compiler names it; `.sqlx --check` runs |
| The engine's `app` map is stale relative to a Settings default edit | Low | It is as fresh as every app setting the engine reads (P5); MOD-123's Settings surface owns any refresh |
| Placing target refusal after rung 4 gives a docker-target item with no candidate the rung-4 note | Low | M2 only; M3 re-orders with D9 (P2) |
| Blocking the item on a target refusal surprises someone who expected a soft refusal | Low | Mirrors missing tags and rung 4 (invariant 7: a refusal a human can read); `u` unblocks |
| One shared level (Settings default, workspace, project, workflow) naming a non-runnable target blocks every item the queue runner tries in one tick (review M1, 2026-11-15) | Low now, Medium once setters exist | Maintainer decision 2026-11-15: keep blocking (the missing-tags precedent). The note names the level that chose the target (R1 F1). The setters of MOD-123, MOD-116 and MOD-124 should warn before saving a target that is not runnable; MOD-130's M2 phase note in HANDOFF records this. A `Retired` refusal is soft (R1 L2) |

## Claims to verify

| # | Claim |
|---|---|
| C1 | `WorkerStore` has no `scope_project` and no target read; the engine's store bound is `WorkerStore` (`EngineParts`, `engine.rs:517`). |
| C2 | `PgStore::app_settings` / `MemStore::app_settings` return every `app_setting` row (so `default_target` is in `EngineParts.app`), and `EngineParts.app` is filled from it. |
| C3 | `Item` has `target_id: Option<TargetId>` (M1 T3), and `StepGraph` has `target_id`. |
| C4 | `ResolvedGraph.graph` is the `StepGraph` row the run resolves (override else kind default), and `resolve_or_refused` has it in scope in its resolved arm. |
| C5 | `WorkerStore` implementors are exactly `MemStore`, `PgStore`, `Writer` (`crates/htui-store/src/worker.rs:121, :711`). |
| C6 | `query_as!(Run, …)` appears exactly in `run`, `waiting_candidates`, `overlapping_runs`, `create_run`, `adopt_runs`, `workspace_live_runs`; chat-run creation does not return a `Run` through `query_as!`. |
| C7 | The cache mirror's `run` table has `target_id` and `target_source` (cache `0014`), and `refresh_run` copies both. |
| C8 | `str_enum!` types decode through `query_as!` overrides (`AS "x: TargetKind"` used in M1's Pg code), so `TargetSource` can too. |
| C9 | Whether `htui-core` depends on `thiserror`. |
| C10 | The conformance count pins read 343 today (`pg_conformance.rs`, `mem_store.rs`). |
| C11 | `crates/htui-orch/src/conformance.rs` holds engine cases run over both MemStore and Pg (where and how pinned). |
| C12 | Whether a `WorkerStore`-level conformance path exists for Pg. |
| C13 | No path other than `Engine::enqueue_with` creates a production graph run with `NewRun` (the other `NewRun {` hits are tests or test helpers). |
| C14 | `chk_run_target_source`'s value order in `0040_execution_targets.sql` is item, workflow, project, workspace, settings, default. |
| C15 | `share_level_target` (or M1's equivalent helper) exists in `pg/write.rs` and reads a target `FOR SHARE`, answering `references_no_row`/`target_retired`. |
| C16 | No exhaustive `match` on `EngineError` outside `htui-orch` breaks on a new variant without a wildcard (else list the files for T3). |
| C17 | Task independence: T1 ⊂ {target.rs, model/mod.rs}; T2 and T3 share `.sqlx`, `engine.rs` (literal) and the store crates, so the lane is serial. |

## Verified claims (fact-check, 2026-11-15)

| Claim | Verdict | Evidence |
|---|---|---|
| C1 | true | `WorkerStore` (`htui-core/src/store/worker.rs:157-607`) has neither read; `EngineParts` `S: WorkerStore` (`engine.rs:519`) |
| C2 | true | `pg/read.rs:2152`, `mem.rs:841`; `Kit::read` (`htui-worker/src/runtime.rs:1177`) re-reads per command, `Kit::engine` clones into `app` (`:1274-1296`) |
| C3 | true | `Item.target_id` `model/item.rs:186`; `StepGraph.target_id` `model/kind.rs:467`; `Project.target_id` `hierarchy.rs:87` |
| C4 | true | `kind.rs:704-708`; `resolve_or_refused` (`graph.rs:650`) holds `graph`; `Resolved` literal `graph.rs:852` |
| C5 | true | `htui-core/src/store/worker.rs:816`; `htui-store/src/worker.rs:121`, `:711` |
| C6 | true | `pg/read.rs:769, 1612, 3030`; `pg/write.rs:7987, 8513, 8789`; `start_chat_run` returns `()`; cache `run_of` `cache/read.rs:310` |
| C7 | true | `cache_migrations/0014_execution_targets.sql`; `cache/refresh.rs:1260-1331` |
| C8 | true | `AS "kind: TargetKind"` at `pg/write.rs:2036` and others |
| C9 | true (has it) | `crates/htui-core/Cargo.toml:21`. P6 amended: derive |
| C10 | true | `pg_conformance.rs:76,77,84`; `mem_store.rs:65`. Conventions amended: three Pg spots |
| C11 | **false** | orch conformance is MemStore-only (`htui-orch/src/conformance.rs:83-85`; pin `fake_conformance.rs:18` = 265). Plan amended: engine tests in `engine.rs`, Pg reads in `pg_criteria` |
| C12 | **no** | no `WorkerStore` suite. Plan amended: `pg_criteria` tests |
| C13 | true | `engine.rs:1482` is the only production `NewRun`; the others are tests or test-support |
| C14 | true | `0040_execution_targets.sql:64-65` |
| C15 | **partly** | `share_level_target` `pg/write.rs:2088` is the level setters' helper; item writers fold `EXISTS … FOR SHARE` into a CTE (`:2943`, `:3077`). P8 amended to call `share_level_target` |
| C16 | true | every `EngineError` match has a catch-all (`runtime.rs`, `engine.rs:3041/6923/11137`) |
| C17 | true | `model/mod.rs:235` re-exports `target::{…}`; T2/T3 share `.sqlx`, `engine.rs:1482`, store crates → serial lane |

Further amendments from the check: P5 (`default_target_in` cannot call `DefaultTarget::parse`), P6 (`owner()`
is an `Option`), the conformance mirror case name (`retire_and_delete_refuse_a_target_in_use`), `Writer::scope_project`
exists (`writer.rs:360`), and line drift (`engine.rs:19164`).

## Acceptance

- [ ] T1–T3 complete, each commit green on its own check
- [ ] Gate passes on the lane
- [ ] Reviewer (`rust-reviewer`) findings applied or deferred with the maintainer
- [ ] PRD milestone 2 row updated; HANDOFF phase note written; validator green
