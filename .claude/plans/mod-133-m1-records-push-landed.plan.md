# Plan: MOD-133 M1 — integration records, `push` and the landed check

**Source PRD**: `.claude/prds/mod-133-integration-modes.prd.md`
**Selected Milestone**: 1 — Records, `push` and the landed check
**Complexity**: Large
**Routing**: PRD path (C2, C4), ultracode for implement (lane script, one serial lane). No architect
(the seams are decided here, in the shared decisions; C2 fired, but ANA-34 §7 and this plan carry the
design, and the maintainer did not ask for one).

## Summary

A repo whose `integrate` mode resolves to `push` gets one `integration` record per finished run,
written in `finish_run(Done)`'s transaction; the executing box lands it (verify the run's commits are on
the local target branch, then a fast-forward `git push` with the box's own credentials), once, under a
record lease; a refusal parks the record with a typed reason and an `item_note`. Every readiness site
treats a blocker as released only when its finished run's records are all landed. Close-out's summary
gains an integration section. `local` projects are byte-for-byte unchanged. Settings, the waiting row,
Unblock retry and the display state are M2; the `pull_request` run branch is M3.

## Shared decisions

- **D1 — names.** The record is an **integration** (`integration` table, `Integration` type); the act
  is **landing** (`Isolator::land`, `Engine::land`, the sweep's land arm), because MOD-139 already owns
  `StepKind::Integrate` and the `premerge` vocabulary. Modes: `IntegrateMode { Local, Push,
  PullRequest }` (wire `local`, `push`, `pull_request`). States: `IntegrationState { Pending, Landed,
  PrOpen, PrMerged, PrClosed, Refused }` (wire `pending`, `landed`, `pr_open`, `pr_merged`, `pr_closed`,
  `refused`; the PR states are written by MOD-134, declared now so the CHECK never moves). `is_landed()`
  is `Landed | PrMerged`.
- **D2 — configuration.** Per repo: a nullable `repo.integrate` column (`NULL` = the project default).
  Project default: the `integrate` key of `project.settings`, read **strictly and separately** by
  `IntegrateMode::project_default(settings: &Value) -> Result<IntegrateMode, String>` (absent or `null` →
  `Local`; `"local"`/`"push"`/`"pull_request"` → that mode; anything else → `Err` naming the value), the
  way `ProjectCaps::from_settings` reads its keys. It is **not** a `ProjectSettings` field: a bad value
  there would fail the whole blob's decode and reset every key, `default_isolation` included
  (`graph.rs:1578-1605`). No `SettingKey` yet (M2 adds the registry key and the forms). `IntegrateMode`
  and `IntegrationState` are declared with `str_enum!` (`model/mod.rs:25`), which gives sqlx `Type`
  and the serde wire names. Remote and target are `repo.remote_url` and `repo.default_branch`; no
  override fields in M1 (ANA-34 §7.4's forge fields are MOD-134's). `NewRepo` is **not** changed (a repo
  is created `NULL`); `Repo` gains `integrate: Option<IntegrateMode>` and `RepoPatch` gains
  `integrate: Option<Option<IntegrateMode>>` (the `remote_url` double-option shape). Both new fields are
  `#[serde(default, skip_serializing_if = "Option::is_none")]` so existing JSON and snapshots keep their
  bytes.
- **D3 — snapshot.** `GraphSnapshot` gains `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  integrate: BTreeMap<RepoId, IntegrateSnapshot>` with `IntegrateSnapshot { mode: IntegrateMode,
  remote: String, target_branch: String }`. Only non-`local` repos are entries, so a `local` run's
  snapshot JSON is unchanged; `V` is not bumped (the rule at `run.rs:687-692`, per field at
  `:717-739`; no digest covers the whole snapshot). `resolve_or_refused` writes
  `integrate: BTreeMap::new()` (`graph.rs:693`); the map is filled in **`graph::resolve`'s
  `Resolution::Resolved` branch** (`graph.rs:448-455`, where memory is filled), so a v1 resume, which
  calls `resolve_or_refused` (`engine.rs:4919`), never meets the new refusals. Mode per repo of the run's
  scope = `repo.integrate` else `IntegrateMode::project_default(&project.settings)`. Refusals at run
  start are one new `ResolveError::IntegrateRefused { repo: String, reason: String }` (`Clone + Eq`,
  `graph.rs:190`'s style), whose `#[error]` renders these sentences:
  an unreadable project default → `` "project setting `integrate` is {value}, not one of local, push, pull_request" ``;
  a non-`local` mode with no `remote_url` → `` "`{repo}` integrates by `{mode}` but has no remote URL: set one, or set it to `local`" ``;
  `pull_request` → `` "`{repo}` integrates by `pull_request`, which lands with MOD-133 milestone 3: set it to `local` or `push`" ``.
- **D4 — record write.** In `finish_run_on` (Pg) and `State::finish_run` (Mem), when `to == Done`,
  in the same transaction: one `integration` row per `graph_snapshot.integrate` entry **whose repo has at
  least one `run_step_commit` with a non-null `after_hash` in this run** (a cheap "the run touched it";
  which of those commits landed is the engine's question, D8), state `pending`, carrying
  mode/remote/target from the snapshot and `project_id` from the run. A repo the run did not change gets
  no record (nothing to land). No other status writes records; `close_batch` finishes `Cancelled` only
  (`pg/write.rs:12829`), so it never writes one. Re-finishing is impossible (`legal_move`), and the
  insert is `ON CONFLICT DO NOTHING` anyway. `finish_chat_run` and `transition_run` never write records
  (chat runs carry no snapshot; no production caller moves a graph run to `done` through
  `transition_run`). Mem: `State` gains `integrations: BTreeMap<(RunId, RepoId), Integration>`; the
  snapshot is decoded from `Run.graph_snapshot` (`Option<Value>`), commits joined through `steps` by
  `run_id` (`mem.rs:342-351`).
- **D5 — table** (`0039_integration.sql`, mirrored by cache `0013_integration.sql`):
  `integration(run_id UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE, repo_id UUID NOT NULL
  REFERENCES repo(id), project_id UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE, mode TEXT NOT
  NULL CHECK (mode IN ('push','pull_request')), remote TEXT NOT NULL, target_branch TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN (…6…)), refusal TEXT, landed_hash TEXT,
  pr_ref TEXT, pr_url TEXT, lease_box_id UUID REFERENCES box(id) ON DELETE SET NULL, lease_owner UUID,
  lease_expires_at TIMESTAMPTZ, created_at, updated_at, PRIMARY KEY (run_id, repo_id),
  CHECK ((state = 'refused') = (refusal IS NOT NULL)))`, the `set_updated_at` trigger,
  `idx_integration_updated_at`, a partial `idx_integration_pending ON integration(run_id) WHERE state =
  'pending'`. Plus `ALTER TABLE repo ADD COLUMN integrate TEXT CHECK (integrate IN
  ('local','push','pull_request'))`. Header comment in `0034`'s style. Cache: the table in
  `item_artifact_link`'s mirror style (TEXT/INTEGER, no CHECK, no FK) without the lease columns,
  refreshed with `WHERE project_id = $1` and `upsert_sql(…, 2)` (composite key, `refresh.rs:1391`).
  `repo.integrate` is **not** mirrored: no cache reader builds a `Repo`.
- **D6 — store API**, all three on **`WriteStore`** (where `repos` already lives, `traits.rs:937`) and
  mirrored on `WorkerStore`, so the cache and `Backend` need nothing. Implementors to touch: `MemStore`
  (`mem.rs:11245`), `PgStore` (`pg/write.rs:2665`), `Writer` (`writer.rs:384`), the test spies
  `UsageSpy` (`htui-agent/src/conformance.rs:784`) and `SpyStore` (`htui-agent/tests/it/recorder.rs:472`),
  and the three `WorkerStore` impls (`htui-core/src/store/worker.rs:796`, `htui-store/src/worker.rs:120`,
  `:689`). Trait docs carry the contracts.
  - `WriteStore::integrations(&self, run: RunId) -> Result<Vec<Integration>>`, ordered by repo id.
  - `WriteStore::take_integrations(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta, run:
    Option<RunId>) -> Result<Vec<Integration>>`: leases and returns every `pending` record whose run's
    `executing_box_id = box_id` (and `run_id = run` when given) and whose lease is free, expired or
    `owner`'s; expiry is the store clock + `ttl` (the `claim_run` rule, `traits.rs:1587-1594`).
  - `WriteStore::settle_integration(&self, run: RunId, repo: RepoId, owner: Uuid, outcome:
    &LandOutcome) -> Result<bool>`: `pending` and `lease_owner = owner` → `landed` (sets `landed_hash`)
    or `refused` (sets `refusal`), clearing the lease; `false` when the row is no longer this owner's
    pending row (lost lease), never an error.
  - `LandOutcome { Landed { hash: String }, Refused { reason: String } }`.
  - `WorkerStore` gains the three, forwarding (`worker.rs` in both crates).
- **D7 — the landed check (ANA-35 §7.1), six sites.** A `blocked_by` target is **released** when it is `closed`,
  or its latest graph run (newest `kind = 'graph'`, not `cancelled`, by `(queued_at, id)`) is `done`
  **and no `integration` row of that run has a state outside (`landed`, `pr_merged`)**. No records
  counts as landed, so `local` is today's rule. Applied at: Pg `items(ready)`
  (`pg/read.rs:156-166`), `ready_items` (`:2406-2415`), `queue_rows`' open-blocker list (`:2693-2704`);
  Mem `is_released` (`mem.rs:6503-6506`, which serves `is_ready` and `open_blocker_keys`); cache
  `ready_case` (`cache/read.rs:248-255`, `LATEST_OF_T` at `:241`). `prune_finished_entries` (Pg and Mem)
  **stays on `done`**: it concerns the entry's own item, which `ready_items` already rejects once it has
  a graph run, so keeping an unlanded entry would only make it linger. The close-out law (`closes_from`)
  and `ItemState` are **not** changed in M1 (MOD-135 / M2).
- **D8 — landing (`Isolator::land`).** New trait method
  `fn land<'a>(&'a self, request: &'a LandRequest) -> IsolatorFuture<'a, LandOutcome>` with
  `LandRequest { repo: RepoId, remote: String, target_branch: String, commits: Vec<String> }`.
  `commits` are the run's **landed** commits for the repo, computed by the engine with a pure
  `landed_commits(steps: &[RunStep], commits: &[(StepId, Vec<RunStepCommit>)], repo: RepoId) -> Vec<String>`
  in htui-orch: the `after_hash` of exactly the steps reconcile merged into the primary — derive the
  predicate from `reconcile_done_step`'s callers (`engine.rs:8880-8940` and its eight call sites), which
  excludes at least fan-out losers (`selected == Some(false)`), judge rows (`fanout_index == -1`),
  divided-run parts and stage rows (`role`), and steps not `done`; pin it with unit cases for a fan-out
  run and a divided run. A wrong predicate costs a visible `not_on_target` refusal, never a lost push.
  Reads: `run_steps(run)` then `step_commits(step)` (as `close_out_reads` does, `engine.rs:3634-3651`).
  An empty set is fine: push mode pushes the target branch. The `GixIsolator` does:
  no checkout on this box → `Refused("repo_unavailable: …")`;
  local `refs/heads/<target>` missing → `Refused("no_target_branch: <target>")`; any commit not an
  ancestor of it → `Refused("not_on_target: <short hash> is not on <target>")` (these local reads under
  the repo's `admin_guard`, which is **released before** any network verb: its invariant is "held for the
  one child and nothing else", `real.rs:315-323`; the sync gix reads `branch_target`/`is_ancestor` are
  wrapped in `blocking(…)` as `head_of` does, `real.rs:461-469`) (this is ANA-34 §7.3's
  "primary must be on the target branch" check, made at landing instead of at every step so the walk is
  untouched); `git ls-remote <remote> refs/heads/<target>` equal to the local tip → `Landed` (the
  idempotence check: a crash after a push re-lands nothing); otherwise
  `git push --porcelain <remote> <tip>:refs/heads/<target>` — **never** `--force` or a `+` refspec —
  exit 0 → `Landed { hash: tip }`; a `[rejected]` non-fast-forward/fetch-first line →
  `Refused("non_fast_forward: <target> on the remote has moved; merge it locally, then retry")`; any
  other non-zero exit → `Refused("push_failed: <scrubbed stderr tail>")`. An `IsolateError` (git
  missing, the 120 s budget) is returned as `Err` and leaves the record `pending` for the next sweep,
  never `refused`. Remote URLs in every reason pass through a new display redactor
  `redact_remote(url: &str) -> String` beside `normalise_remote` in `infer.rs` (drops userinfo, reusing
  the "last `@`" rule at `infer.rs:139-142`; `normalise_remote` itself is a comparison key, not a URL). Both verbs go through `Cli::run` (scrubbed env,
  `GIT_TERMINAL_PROMPT=0`, the 120 s budget) plus `GIT_SSH_COMMAND` = the inherited value (else `ssh`)
  with ` -o BatchMode=yes` appended (PRD Q2: the box's own credentials, never a prompt; `SCRUBBED_ENV`
  keeps `SSH_AUTH_SOCK` and `GIT_SSH_COMMAND`). `FakeIsolator` follows its FIFO pattern
  (`fake.rs:97-226`): a `Mutex<VecDeque<Result<LandOutcome, IsolateError>>>` of scripted answers (empty
  → `Landed { hash: last commit, or "0" * 40 }`), a `land_requests` log and an `IsolatorCall::Land`
  entry; `StallAfterReconcile` (`tests/it/gix_isolator.rs:2017`) delegates.
- **D9 — the land pass.** `pub async fn land(&self, run: Option<RunId>) -> Result<usize, EngineError>`
  takes (`take_integrations(self.parts.box_id, self.parts.owner, self.lease_times().ttl, run)`) and
  calls `pub async fn land_taken(&self, records: Vec<Integration>) -> Result<usize, EngineError>`, which
  per record read the run's
  commits for the repo, call `isolator.land`, `settle_integration`; on `Refused` (and only when settle
  answered `true`) write an `item_note`: `` "integration refused for repo `{repo}` ({mode} to {target}): {reason}; the run's work has not landed" ``;
  on `Landed` a note `` "integration landed: repo `{repo}` {target} at {short hash}" ``. Returns the count
  settled. An `Err` from `isolator.land` leaves that record pending (logged, the loop continues). Called (a) by the walk
  loop right after `end_run(Done)` and `cleanup_walk` (`engine.rs:4427-4436`), for that run, its error
  logged at `warn` and swallowed (the sweep retries; a land failure must not turn a finished walk into
  an error), and (b) by the worker/TUI sweep for the whole box (D10). A `refused` record is **not**
  retried by either (Unblock retry is M2).
- **D10 — the sweep arm.** In `sweep_once` (`crates/htui-worker/src/runtime.rs:2226`),
  **unconditionally** between the `adopt` `if` block and `if claims` (`:2275-2276`; `adopt` and `admit`
  are both conditional): first the cheap step, `take_integrations(box_id, owner, ttl, None)` through the
  sweep's writer with the same owner the engine is built with (T5 confirms they are one `Uuid`); only
  when it returns records, `Kit::read(&ctx.shared, host, false)` + `kit.engine(&driver)` (as `adopt`
  does, `:2605-2613`) and `engine.land_taken(records)`. Errors logged, never fatal to the sweep. Same arm
  for both roles (I-1 is checked above it, `:2246-2250`).
- **D11 — close-out.** `closeout::summary` gains `integrations: &[Integration]` (last parameter; its
  8th, so it carries `#[expect(clippy::too_many_arguments, reason = "…")]`; callers `engine.rs:2812` and
  the `write` test helper `closeout.rs:406`); when
  non-empty it appends, after the run lines, `\n## Integration\n\n| repo | run | mode | target | outcome |\n|---|---|---|---|---|\n`
  and one row per record: repo name, run id, mode, `{remote without userinfo} {target}`, and
  `landed <short hash>` / `refused: <reason>` / `pending` / the PR states' wire names. Empty → today's
  bytes exactly. `Engine::close_out` reads `integrations(run)` for each of the item's runs. `preview` is
  unchanged.
- **D12 — requirement edits (PRD Q3).** Done in `bfb2fd17` (ANA-34 §10.2 → `R-ORCH-14`, §10.9 →
  `R-ORCH-11`).
- **D13 — pins that move.** Migration count 38 → 39: `htui-store/tests/it/connect.rs:140, 155, 241`;
  `tests/it/migrations.rs:136` (version list), `:4303`, `:4404` (`Pending(38)`), `:4413`, `:4436`
  (`MigrationsPending(38)`), `:4588` (`_sqlx_migrations` count), and the "thirty-eight / through 0038"
  prose at `connect.rs:141, 157, 243` and `migrations.rs:154, 185, 200, 4304, 4405, 4414, 4437, 4589`.
  `TABLES` 58 → 59 (`migrations.rs:38-114`, asserted `:172`, prose `:191`). `MIRRORED_TABLES` 23 → 24
  (`cache/mod.rs:51`; pin `tests/it/cache.rs:3150-3151`; `crates/htui/tests/it/snapshots/it__connection__confirm.snap:11`,
  "Goes: the 23 mirrored tables"). Conformance `CASES` 325 + the new cases
  (`htui-core/tests/it/mem_store.rs:57-58`, `htui-store/tests/it/pg_conformance.rs:71`). `.sqlx` regenerated
  with `--all-targets --all-features` against a migrated scratch DB (`docs/hr-sandbox.md`).

## Tasks (one serial lane)

Every `cargo` command in this sandbox is prefixed `env -u HTUI_TEST_QDRANT_URL`; no live Qdrant test is
run (a new live test is only compiled). No agent runs the workspace gate.

### T1 (foundation) — model, traits, MemStore, conformance
- **Files**: CREATE `crates/htui-core/src/model/integration.rs`; UPDATE `crates/htui-core/src/model/mod.rs`,
  `model/hierarchy.rs` (Repo, RepoPatch), `model/kind.rs` (ProjectSettings), `model/run.rs`
  (GraphSnapshot, IntegrateSnapshot), `store/traits.rs`, `store/worker.rs`, `store/mem.rs`,
  `store/conformance.rs`, `htui-agent/src/conformance.rs` (`UsageSpy`), `htui-agent/tests/it/recorder.rs`
  (`SpyStore`). Compile breaks to fix with the new fields: the 6 `Repo {` literals
  (`htui-agent/tests/it/excerpt.rs:1060`, `htui-core/src/model/item_spec.rs:509`, `store/mem.rs:3679`,
  `htui-orch/src/closeout.rs:367`, `infer.rs:402`, `overlap.rs:138`); the one full `RepoPatch` literal
  (`htui/src/ui/tabs/settings/hierarchy.rs:1433`); the 11 `GraphSnapshot {` literals
  (`htui-agent/tests/it/relay.rs:97`; `htui-core/src/fixtures.rs:1579`, `model/run.rs:2283`,
  `store/conformance.rs:8051`, `store/mem.rs:16787`; `htui-mcp/tests/it/tools_backlog.rs:40`;
  `htui-orch/src/graph.rs:693`; `htui-store/tests/it/cache.rs:683, 997`, `tests/it/pg_criteria.rs:1258`;
  `htui/tests/it/backlog.rs:1201`). Pg's four `query_as!(Repo, …)` break here too; T2 owns them, so T1's
  check is crate-scoped and T1 notes the break.
- **Do**: D1, D2, D3 (types only; resolution is T4), D4 (Mem), D6 (traits + Mem + `WorkerStore for MemStore`),
  D7 (Mem's site). Conformance cases (each runs over Mem here and Pg in T2): finish Done writes one
  pending record per changed non-local repo and none for `local`, for an unchanged repo, or for
  Failed/Cancelled; `take_integrations` leases only this box's pending records, respects a live foreign
  lease, takes an expired one, filters by run; `settle_integration` lands/refuses once and answers
  `false` to a stale owner; readiness: a dependent of a `done` run with a pending record is not ready,
  with a refused record is not ready, with all landed is ready, with none (local) is ready;
  `update_repo` sets and clears `integrate`; `IntegrateMode::project_default` unit cases.
- **Check**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-core --all-features`; clippy and
  rustfmt on the crates touched; `env -u HTUI_TEST_QDRANT_URL cargo check -p htui-core -p htui-agent -p htui-orch --all-features --tests`.
  `htui-store` (and so `htui-mcp`, `htui-worker`, `htui`) stops compiling at the four `query_as!(Repo, …)`
  until T2: expected; T1 still fixes the literals it lists in those crates, T2's workspace check confirms.

### T2 — Postgres
- **Files**: CREATE `crates/htui-store/migrations/0039_integration.sql`; UPDATE `crates/htui-store/src/pg/read.rs`,
  `pg/write.rs`, `src/worker.rs` (Pg and Writer WorkerStore), `src/writer.rs`, the four
  `query_as!(Repo, …)` queries (`repo_row` `pg/read.rs:3079`, `repo_rows` `:3163`, `create_repo`
  `pg/write.rs:4336`, `update_repo` `:4403` and its SET clause),
  `crates/htui-store/.sqlx/**`, `tests/it/migrations.rs`, `tests/it/connect.rs`, `tests/it/pg_conformance.rs`.
- **Do**: D5 (Pg side), D4/D6/D7 for Pg (three readiness sites), `repo.integrate` read and `update_repo`
  write, D13's Pg pins, `.sqlx` regeneration. Then `cargo check --workspace --all-features --tests`.
- **Check**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and (test(/^migrations::/) | test(/^connect::/) | test(/^pg_conformance::/))'`;
  `cargo sqlx prepare --check` from `crates/htui-store` against the scratch DB.

### T3 — cache mirror
- **Files**: CREATE `crates/htui-store/cache_migrations/0013_integration.sql`; UPDATE `src/cache/mod.rs`
  (`MIRRORED_TABLES`), `src/cache/refresh.rs` (the cursor list `:257-275` and the match `:281-315`, an
  `integration` arm on `refresh_run_plan`'s pattern), `src/cache/read.rs` (`ready_case`),
  `tests/it/cache.rs` and `crates/htui/tests/it/snapshots/it__connection__confirm.snap`.
- **Do**: D5 (cache side), D7's cache site; a cache test: a mirrored refused record holds a dependent
  out of `items(ready)` offline.
- **Check**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^cache/)'`;
  `env -u HTUI_TEST_QDRANT_URL cargo insta test -p htui --features testkit --test it -- connection::`
  (accept only the count line).

### T4 — landing in htui-orch
- **Files**: UPDATE `crates/htui-orch/src/isolate.rs` (trait), `isolate/git.rs` (`ls_remote`, `push`
  verbs, `GIT_SSH_COMMAND`), `isolate/real.rs` (`GixIsolator::land`), `src/fake.rs` (`FakeIsolator::land`),
  `tests/it/gix_isolator.rs` (`StallAfterReconcile` delegate + landing tests), `src/infer.rs`
  (`redact_remote`), `src/graph.rs` (D3 resolution and refusals), `src/engine.rs` (D9, the Finished hook
  between `engine.rs:4430` and its `return`, `close_out` reads), `src/closeout.rs` (D11), the new
  `landed_commits` (in `src/engine.rs` or a small `src/land.rs` registered in `lib.rs`).
- **Do**: D3 (resolution), D8, D9, D11. Tests: git-level against a local bare remote (`file://` or path
  remote, no network): lands once and a second `land` is idempotent; non-fast-forward refuses and the
  remote is untouched; `not_on_target` when the primary merged onto another branch; no `--force` ever
  (assert the argv); `GIT_SSH_COMMAND` carries `BatchMode=yes`. Engine (fake store + `FakeIsolator`):
  a push-mode run's Finished pass lands it and notes it; a refusal parks the record and notes it; a
  `local` run writes and lands nothing; a mode changed between start and finish lands per the snapshot;
  snapshot refusals (no remote, `pull_request`). Close-out: summary with records; byte-identical without.
- **Check**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-orch --all-features`.

### T5 — the sweep arm
- **Files**: UPDATE `crates/htui-worker/src/runtime.rs`; a test in `crates/htui-worker/tests/it/` (an
  existing runtime test module, or a new `landing.rs` registered in `tests/it/main.rs`).
- **Do**: D10; confirm the sweep's owner is the engine's `parts.owner`. Test: a pending record of a run executed on this box is landed by one `sweep_once`; a
  record of another box's run is left alone; a sweep on a box whose executor role does not match lands
  nothing.
- **Check**: `env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-worker --all-features`.

### T6 — worker doc (main thread, after T5)
- **Files**: `docs/htui-worker.md` (the sweep bullet at `:44`, under the heading at `:32`).
- **Do**: name the land arm in that bullet. (D12 already done.)

## Validation

`env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh` on the main thread after the lane; "PASSED (1 skipped:
qdrant)" is green in this sandbox. `cargo clippy --workspace -- -D warnings` (featureless) too.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Seven readiness spellings drift | high | one conformance case per state over Mem and Pg; the cache case in T3 |
| A snapshot or JSON pin moves for `local` runs | medium | D2/D3 `skip_serializing_if`; any moved snapshot is a defect, not an accept |
| `GIT_SSH_COMMAND` overrides a user's `core.sshCommand` | low | documented in D8; MOD-134 replaces ambient credentials |
| The Finished hook makes the walk loop fail | low | D9 swallows and logs; the sweep is the recovery path |
| Migration renumber at collect | high | HANDOFF live coordinates; pins listed in D13 |

## Verified claims

Fact-check 2026-11-14 (one agent, 25 claims against the tree; 12 true as written, 13 amended above).

| claim | verdict | evidence |
|---|---|---|
| `close_batch` finishes only `Cancelled`; the only engine `Done` is the walk's Finished arm | true | `pg/write.rs:12829`, `mem.rs:1411`, `engine.rs:4429`; `finish_chat_run`/`transition_run` write no graph `done` |
| A new snapshot field needs no `V` bump and moves no digest | true (cite fixed) | `run.rs:687-692, 717-739`; `topology` hashes phases/edges only (`graph.rs:392, 964`) |
| About 21 `Repo {` literals | **false → 6** | amended in T1; `Repo` has no `Default`; 1 full `RepoPatch` literal |
| `ProjectSettings` can take `integrate` like `default_isolation` | partly → **amended D2** | hand-written `Default` (`kind.rs:672`); a bad value resets the whole blob (`graph.rs:1578-1605`) |
| `resolve_or_refused` is the place for the refusals | **trap → amended D3** | v1 resume calls it (`engine.rs:4919`); unlisted `ResolveError`s fail the resume (`:4976`) |
| Engine holds box, owner, ttl | partly | `parts.box_id`/`parts.owner` (`engine.rs:559, 561`); ttl = `self.lease_times().ttl` (`:1514`) |
| Finished arm: `end_run(Done)`, `cleanup_walk`, `?` | true | `engine.rs:4427-4436` |
| `closeout::summary` call sites | true: 2 | `engine.rs:2812`, `closeout.rs:406`; 8 params trips clippy → `#[expect]` |
| `Cli::run` takes extra env; `admin_guard`, `is_ancestor`, `branch_target` exist; git has `push --porcelain` | true | `git.rs:360, 1536, 1882`; `real.rs:576`; git 2.43.0; admin lock invariant `real.rs:315-323` → amended D8 |
| `infer.rs` has a reusable URL redactor | partly → **amended D8** | `normalise_remote` is a key, not a URL (`infer.rs:111`) |
| `FakeIsolator` scripts per call; `StallAfterReconcile` delegates all | true | `fake.rs:97-226`; `gix_isolator.rs:2017-2132` |
| `Isolator` has no default methods; 3 impls | true | `isolate.rs:166-341` |
| A per-run commit read exists | partly | none per run: `run_steps` + `step_commits` (`engine.rs:3634-3651`) |
| D6's implementor list is complete | **false → amended D6** | `WriteStore` also on `UsageSpy`, `SpyStore`; methods moved off `ReadStore` to spare `Backend`/`CacheStore`/`Unlisted` |
| Mem `finish_run` sees snapshot and commits | true | `mem.rs:342-351` |
| `prune_finished_entries` is a readiness site | **false → amended D7** | own-item queue pruning (`pg/write.rs:12918-12942`, `runtime.rs:2329`); stays `done` |
| Test paths and pins | true (list) | exact lines in D13 |
| `MIRRORED_TABLES` 23, explicit refresh arms | true | `cache/mod.rs:51`; `refresh.rs:257-315` |
| `repo` mirror must gain `integrate` | partly → **amended D5** | no cache reader builds `Repo`; not mirrored |
| Pg repo reads are sqlx macros | true | 4 `query_as!(Repo, …)` (amended T2) |
| Sweep arm "after adopt, before admit" | partly → **amended D10** | both conditional (`runtime.rs:2271-2281`); arm placed unconditionally with a cheap take first |
| Worker doc lists the sweep | partly | one bullet `docs/htui-worker.md:44` |
| Next migrations `0039` / cache `0013`; `set_updated_at()` | true | `0001_init.sql:21` |
| `executing_box_id` stays set after `finish_run` | true | set only in `claim_run` (`pg/write.rs:7456`, `mem.rs:6963`) |
| `after_hash` of every step is on the target | **false → amended D8** | fan-out losers and parts keep unmerged hashes (`isolate.rs:249-253`) |
| D12 requirement edits pending | done | `bfb2fd17` |

## Acceptance
- [ ] T1-T6 complete; the gate is green in the sandbox (qdrant skipped)
- [ ] `local` behaviour and every existing snapshot unchanged
- [ ] Reviewer (`rust-reviewer`) findings applied or deferred with the maintainer
