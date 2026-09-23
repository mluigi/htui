# Blueprint: MOD-4 milestone 4, "three candidates, one winner"

**Plan**: `.claude/plans/mod-4-orch-fanout.plan.md`, which the maintainer confirmed on 2026-09-23. It covers D48–D71, with OQ-1..OQ-8 answered and D71 added when the plan was confirmed. **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`. Where D1–D8 disagree with this blueprint, D1–D8 win. **Design authority**: the ANA-2 and ANA-5 passages the plan's header cites.

**Verified at**: HEAD `adcfe8b` on branch `mod-4-m4`. The plan's ledger was taken at `01feaff`. The two commits since then touch only `.claude/plans/**` and `HANDOFF.md`, so every `crates/` citation in the plan still resolves, and I re-read each one. **Line numbers are pre-edit.** A citation into a file a task edits moves after that task's first commit.

**Graphify**: not used. Every `htui-core`, `htui-agent` and `htui-orch` fact below was read directly from the tree.

**Scope**: Wave A is T1 ∥ T2 ∥ T3 ∥ T4 ∥ T5, on one shared tree. After that, T6 → T7 → T8 run in order. The milestone adds two new modules (`select.rs`, `fanout.rs`) and one new dependency edge (`futures` into `htui-orch`). It adds no seam method, no migration, no `.sqlx` change and no `.snap` change. The `git` binary gets one new verb, `diff`.

**House style (carried)**: one named free function per refusal sentence. `Display`-exact failure vocabularies. Every instant comes from `Clock`. No `std` guard is held across an `.await`. No `gix::Repository` crosses an `.await`. The only `Command::new` calls under `crates/htui-orch/src/` are in `isolate/git.rs` and `verify.rs`. `#![warn(missing_docs)]`. Implementers commit incrementally, with `git add <own paths>` only (never `-A`/`-a`, never `stash`). Every gate is verified with `--test-threads=1`.

---

## 0. Flags

These are gaps between the plan and the tree, plus things I added. Each row has a resolution the implementer follows. **A-** rows depart from the plan's letter and go to the maintainer before dispatch. **F-** rows are mechanical corrections.

| # | Plan says | Tree at HEAD | Resolution |
|---|---|---|---|
| **A-1** | D56: with a slot, `shared_serialized` prepare does "HEAD == base → proceed; clean and HEAD != base → reset; dirty → Refused". | Read in that order, sibling 0 of a group proceeds on a **dirty** checkout at base: it records `dirty = true`, as M3's `prepare_in_place` does (`isolate/real.rs:372-387`). Only siblings 1..n are refused. T8's `a_dirty_shared_checkout_fails_every_sibling_and_parks` cannot pass under that reading, and sibling 0 would be judged on the user's uncommitted work while its siblings are not. | **With a slot, check dirtiness first.** `is_dirty(local_path)` → `Refused(dirty_tree_not_reset(path))` for **every** sibling. Otherwise HEAD == base → proceed; otherwise `reset --hard <base>`. This tightens D56's first bullet to "clean and HEAD == base", which is ANA-2 `:916`'s "dirty at step start and the user has not accepted the reset". A no-slot `prepare` is unchanged (M3's `dirty = true` recording stands). |
| **A-2** | D70 serialises `git worktree add` per repository. | The fact-check probed only concurrent **adds**. Under a concurrent drive, `git worktree remove` also mutates `.git/worktrees/`. That happens at D27's capture-time removal (`real.rs:716-724`), at `worktree_at`'s remake (`:499`, `:518`) and at cleanup (`:743-748`). It overlaps a sibling's `add` on the same repository, and that race was not probed. | The per-repository lock (`adds`) also wraps every `remove_worktree` call. It is the same lock, held for the one child, and has the same deadlock argument as D70. It is named `admin` in code, and its test counter counts both verbs. If the maintainer declines this, only adds are wrapped, as the plan says. |
| **A-3** | D55: `diff` runs "in the tree's own repository for `copy`". | After reconcile, a `copy` winner's `after_hash` is the **merge commit**, and that commit exists only in the primary checkout. `copy_range` copies copy → primary, never back (`real.rs:817-833`). D67's `previous_diff` over `before..M` would therefore fail with `bad revision` in the copy. | `diff` picks, per row, the repository that holds `after`. For `copy` it tries the copy first and falls back to the checkout. For every other mode it uses the checkout. The check is a new **`gix` read**, `git::has_commit(path, hex) -> Result<bool, IsolateError>`, not a verb. |
| **A-4** | D55's argv, "with M3's environment". | `COLUMNS` is inherited (it is not in `SCRUBBED_ENV`), and `git diff --stat` honours it even when stdout is piped. I confirmed this on this box: `COLUMNS=40 git diff --stat` truncates. The judge prompt's bytes, and so its digest, would depend on whatever launched `htui`. | `Cli::diff` passes `extra_env = [("COLUMNS", "80")]`. The argv stays exactly D55's. |
| **A-5** | D58 assembles the prompt once per group. D48 defines how a candidate fails once it is live. | Neither says what happens when stage 3 finds a **missing required input** (`engine.rs:1300-1302`) for a group. Assembly happens before any candidate is live, so M2's `fail_before_a_token` (which needs a `running` step, `:1250-1268`) has no step to fail. | `drive_group` assembles **before** the first `pending → running`. If an input is missing, every pending candidate moves `pending → running → failed` (two legal compare-and-sets, `model/run.rs:113-117`) with an `item_note`. Then `finish_run(Failed, RunFailure::MissingInput(kind))` and `cleanup_run`. This is the same run outcome as `fan_out = 1`. |
| **A-6** | D49(4) writes `select_fanout(winner, "the only candidate whose verify_command did not fail")`. D65 writes `select_fanout(…, "selected by a human")`. | `select_fanout` writes `reason` **only to the judge row**, and only when that row is `pending \| running \| awaiting_approval` (`mem.rs:3574-3586`, `traits.rs:803-813`). The auto-win creates no judge. A human pick after a judge failure leaves the failed judge alone, and a human pick with no judge has nowhere to write. So both reasons would be lost silently, which breaks invariant 7. | Both paths also write one `item_note` carrying the reason, with `via_step_id = winner`. The reason is still passed to `select_fanout`, where it does no harm. No seam change. |
| **A-7** | T8's case list. | A shared-serialized candidate that fails between `prepare` and `capture` keeps its `(box, repo)` guard. Guards are released only at `capture` (`real.rs:932-941`), on a `prepare` error (`:911-916`) and at `cleanup(run)` (`:1006`). Its siblings in the same `join_all` would wait on `lock_owned` forever (`real.rs:309`). M3's H-15 relied on `fail_hard → finish_run → cleanup_run`, and D48 removes that path for candidates. | `run_candidate`'s failure path calls `isolator.capture(step, &trees)` **best-effort** whenever `prepare` succeeded and `capture` has not run yet. The call is logged, its `Ok` value goes to `record_commits`, and an `Err` is ignored. `GixIsolator::capture` always releases (`real.rs:938-940`). This needs no new seam method. T8 gains one case: `a_failed_shared_sibling_releases_the_checkout_for_the_next`. |
| **F-A** | D48 and D53: a candidate's settle reads "its output document", and the judge's verdict is "the newest `judge`-kind document the judge step produced". | **`documents_of_kinds` answers only the latest version of each kind for the whole item** (`traits.rs:90-101`; `mem.rs:959-993` via `latest_document`). `Engine::output_of` filters that single row by `produced_by_step_id` (`engine.rs:1640-1656`). With three candidates each writing a `research` document, candidates 0 and 1 find nothing and settle `MissingOutput`. **Fan-out cannot work without this fix.** | T7's first commit rewrites `output_of`: `ReadStore::documents(item)` returns heads for every version (`traits.rs:74`, `mem.rs:1012-1025`), filtered by `kind` and `produced_by_step_id`, highest `version`, then `ReadStore::document(id)` for the body (`traits.rs:88`). For `fan_out = 1` the behaviour is unchanged. Both methods already exist, so there is no seam change. |
| **F-B** | Plan Risks row: "the review-body half still fires on a repeated review". | `gate::reviews_are_identical` (`gate.rs:801-839`) reads through the same latest-only `documents_of_kinds`. It can never hold two documents, so `LoopStop::NoProgressReview` has been **unreachable since M2**. No test reaches it; `gate.rs:1274` only tests its `Display`. | **Not fixed here.** Fixing it could change which stop reason existing loop cases hit, and it is outside the plan. Carried as **R-9** (§11). The Risks-row sentence is false at HEAD, and the main thread should correct it. |
| **F-C** | D49 "(1) … `never` retries the whole group"; D65 "RetryStep … retire every member". | T7's file set does not include `gate.rs`, and `gate::retire` is private (`gate.rs:854`). | **T5** exposes `pub(crate) async fn retire_slot(ctx, steps, position, now)`. `retire` becomes a loop over it. T7 calls `retire_slot` for both the group retry and D65's `RetryStep`. |
| **F-D** | D65: "`RetryStep` on any member of a parked fan-out slot retries the group". | `command::retry_enabled` accepts only `awaiting_approval \| failed` (`command.rs:319-338`), but a parked group's candidates are `done`. | T7 adds `command::retry_group_enabled(run, slot, phase)`. `retry_step` branches on `phase.fan_out > 1` before calling `retry_enabled`. |
| **F-E** | T7 criterion-8 case: "`HTUI_ANA_2` on the `analysis` graph via `repoint` with `research` `fan_out 3`". | Every seeded phase is `gate: Always` (`seed.rs:221-222`). D49(2) sends an `always` group straight to a human, so no prefilter and no judge would run. | The criteria 8/9/10 cases' `repoint` also sets `gate = Gate::Never`. The 9a/9b cases also set `verify_command = Some("true")`. `a_gated_fan_out_parks_for_human_selection` keeps `Always`. |
| **F-F** | T7's `the_review_loop_reruns_a_fanned_out_implement` on the feature graph with a judge. | D63 planned count at `fan_out 3`: `1 + 1 + 3 + 1 + 1 (judged) = 7 > 6`. That is exactly T4's `AgentCap { planned: 7, max: 6 }`, so the case would be refused at `StartRun`. | The case uses `implement` at `fan_out = 2` (`1 + 1 + 2 + 1 + 1 = 6`), with `implement` and `review` at `gate = Never`. |
| **F-G** | D53/T2: `judge_phase` is "named `<phase>:judge`". | The seeded judge body opens "You are judging `{{phase}}` candidates" (`prompt/defaults.rs:176`). The engine fills `spec.phase` from the phase it is handed (`engine.rs:1351`). | The judge's `PromptSpec.phase` is the **judged** phase's name. `judge_phase(...).name` (`research:judge`) is used only for `run_step.phase_name`, and so for `SessionKey.phase`. `judge_phase` also sets `isolation: Local` and `command_queue: Off`. |
| **F-H** | T2 builds its test fixtures freely; T4 adds `SnapshotJudge.template` in parallel. | A `SnapshotJudge { agent_id, agent_name, model }` literal in T2's tests stops compiling the moment T4's field lands. This is a hidden Wave-A coupling. | T2 builds `SnapshotJudge` only through `serde_json::from_value(json!({…}))`, where `template` is `#[serde(default)]`. The only construction site in `src/` is `graph.rs:485`, which T4 owns. |
| **F-I** | D51: when the walk skips the judge's agent, the judge is "created, moved pending → running, and failed". | `create_step` refuses an `agent_id` with no `agent` row (`mem.rs:3331-3337`). | When the agent row is missing, the judge row is created with `agent_id: None, model: None`. The failure path never opens a session, so `candidate_of` is never called on it. |
| **F-J** | D55: several repos are concatenated "in scope order". | `diff` receives `step_trees` rows, which are in `repo_id` order (M3 H-13), not the scope. | Use the order of the `commits` slice it is handed, which is also `repo_id` order. The repo name comes from `IsolatorConfig.repos[repo_id].name`. |
| **F-K** | D71 calls the selector per candidate. | The plan does not say what happens when the selector answers `None` for index `i > 0` after index 0 was chosen. | Select for **every** index first; this is pure and writes nothing. Any `None` means the phase is refused through D62's path before any row is created. No partial group is ever written. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum, each compiles) | Gate |
|---|---|---|---|
| T1 `allowed_warning` | htui-core | 1 | `cargo test -p htui-core --all-features model::quota`; `cargo test -p htui --all-features --test settings -- --test-threads=1`; `cargo clippy -p htui-core --all-targets --all-features -- -D warnings` |
| T2 `select.rs`, `fanout.rs` | htui-orch | 2: (a) `select.rs` + `lib.rs` mod line; (b) `fanout.rs` + `lib.rs` mod line and doc | `cargo test -p htui-orch --all-features --lib -- select:: fanout::`; clippy `-p htui-orch` |
| T3 the `Isolator` seam | htui-orch | 5: (a) `Cli::diff` + head buffer + `has_commit` + doc edits; (b) `copy.rs` `copies`; (c) trait change with no-op-shaped impls and every call site; (d) `real.rs` per-mode slot/base/diff/siblings; (e) D70/A-2 admin lock and fake behaviours | `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; C-10 |
| T4 snapshot, caps, writer | htui-core, htui-orch | 2: (a) `SnapshotJudge.template` + `graph.rs:485` gets `template: None` + `set_project_settings`; (b) `graph.rs` pin + three `ResolveError` variants and their checks | `cargo test -p htui-core --all-features`; `cargo test -p htui-orch --all-features graph::`; clippy on both |
| T5 groups in status/gate | htui-orch | 2: (a) `status.rs` variants and helpers; (b) `gate.rs` `retire_slot` and `winner_at` in `no_progress` | `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy |
| T6 stage-1 walk, fallback, D67, D68, D71 | htui-orch | 4: (a) `agent_boxes` + rung 3 + `resolve(box_id)`; (b) `SessionKey` / `DriverFor` / `after_done` / `select(…, index)`; (c) the engine's stage-1 walk, D62, D67; (d) five `CASES` and both pins at 23 | as T5, plus C-12 |
| T7 drive, judge, selection | htui-orch, root lock | 4: (a) `futures` + lock, F-A `output_of`, `session` split; (b) `command.rs` surface + `SelectFanout` handler + group `RetryStep`; (c) `Cursor::{Fan, Select}` + `admit` per index + `drive_group`/`run_candidate`/`select_stage`/`run_judge`/`park_selection`; (d) twelve `CASES`, pins at 35, `lib.rs` re-exports | as T5, then the workspace gate |
| T8 real git | htui-orch tests | 2: (a) fan-out cases; (b) D70 50-round case + criterion 7 over worktrees | `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`, then the workspace gate |

Every task writes its tests first. The first failing test is named in each section.

---

## 2. Wave A: the compile-coupling contract

**File sets.** I intersected the task file lists against the tree; they are pairwise disjoint.

| Task | Owns | May **not** |
|---|---|---|
| T1 | `crates/htui-core/src/model/quota.rs` | change `available`'s signature (`quota.rs:423`), `SkipReason`'s shape (`:356-378`), or any rule other than rule 3 |
| T2 | `crates/htui-orch/src/{select.rs, fanout.rs}` (new), `crates/htui-orch/src/lib.rs` | add any re-export (T6/T7 do); touch `engine.rs`; build a `SnapshotJudge` by literal (F-H); pin `allowed_warning` either way in a test (that is T1's) |
| T3 | `isolate.rs`, `isolate/{real,git,copy}.rs`, `fake.rs`, `engine.rs` | change `engine.rs` beyond the two call sites (`:871` `prepare(…, None)`, `:1157` `reconcile(…, &[])`); re-export `FanoutSlot` (`lib.rs` belongs to T2 in this wave; T7 adds it); touch `FakeOrchestrator` or `FakeGraphSource` (T6) |
| T4 | `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/mem.rs`, `crates/htui-orch/src/graph.rs` | change `graph::resolve`'s **signature** (callers: `engine.rs:322`, `:681`, `:3658`; `fake.rs:1120`; `tests/fixtures.rs:14-21`, `:106-114`; `graph.rs` tests); touch `GraphSource` or `candidates` (T6); bump `GraphSnapshot::V` |
| T5 | `crates/htui-orch/src/{status.rs, gate.rs}` | add a `Cursor` variant, which would break `engine.rs:626-656` and `:1534-1541` (T7 adds them); change `cursor`, `latest_at` or `next_attempt`; change `review_loop`'s signature (`tests/review_loop.rs:146`, `:204`, `:256` call it) |

**Hidden coupling, re-checked:**
- **`.sqlx`, migrations, seeds**: no task edits a query. `0003_orchestration.sql:65` already states that `fanout_index = -1` has no CHECK.
- **`.snap`**: the assembler is untouched, so the `htui-core` judge prompt snapshots cannot move. `allowed_warning` appears only in `htui-agent`'s `cli_map` snapshots (`tests/snapshots/cli_map__*.snap`), which are ACP stream fixtures that nothing in this milestone reads.
- **`Cargo.lock`**: only T7 changes it (one line added to `htui-orch`'s `dependencies`; `futures` is already a locked package, `Cargo.lock:3080`).
- **`CASES` pins**: only T6 and T7 touch them, in that order.
- **`lib.rs`**: in Wave A, only T2 touches it.
- **`tests/fixtures/*.snapshot.json`**: both hold `"judge": null` on every phase (9 occurrences). T4's field is only added when there is a judge, and T6's rung 3 finds nothing on the demo fixture, which has no `agent_box` rows (`mem.rs:109`, `:213`). The files must stay byte-identical (C-12).
- **Out-of-crate implementors**: I grepped the whole tree. `Isolator` is implemented only by `real.rs:896` and `fake.rs:205`. `GraphSource` is implemented by `fake.rs:506`, `:610` and `graph.rs:634`. `SessionSink` by `engine.rs:140`, `:1898` and `tests/gix_isolator.rs:358`. `AgentSelector` by `engine.rs:98`. No file under `tests/` calls an `Isolator` method directly. `crates/htui` does not depend on `htui-orch`.
- **Behaviour coupling**: T5's `retire` rewrite must behave identically for `fan_out = 1`, because all 18 `CASES` run it. T1 changes nothing that `htui-orch` tests at HEAD reach, since `available` has no caller in `htui-orch` yet.

**Shared-tree rule.** A Wave-A gate that fails in **another** task's file is not yours to fix. Re-run it after that task's next commit. Keep every edit-to-compile window short, and never commit a non-compiling state. Hidden coupling of this kind is recorded in memory: `parallel-fanout-hidden-file-coupling.md`.

---

## 3. T1: `allowed_warning` is selectable (D61)

**First failing test**: `available_selects_an_allowed_warning_status`. It is the inversion of `available_skips_an_allowed_warning_status` (`quota.rs:1022-1040`), keeps the same document, and expects `Availability::Available`.

- `quota.rs:237-239`: `const ALLOWED: &str = "allowed";` becomes `const SELECTABLE: [&str; 2] = ["allowed", "allowed_warning"];` with a doc citing PRD D3.
- Rule 3 (`:439-443`): `quota.status.filter(|status| !SELECTABLE.contains(&status.as_str()))`.
- Rewrite docs: `# allowed_warning` (`:409-414`) names PRD D3 and this milestone. `# No caller yet` (`:416-421`) becomes `# Caller`: `htui_orch::select::walk` (MOD-4 M4 D60). `SkipReason::Status` (`:368-369`) becomes "a present `status` outside `{allowed, allowed_warning}`".
- New tests:
  - `available_still_skips_every_other_non_allowed_status`: `"rejected"`, `"allowed_critical"` and `""` each give `Skip(Status(verbatim))`.
  - `an_allowed_warning_with_a_full_window_still_skips`: `allowed_warning` plus `("seven_day", 1.0)` gives `Skip(WindowFull { id: "seven_day" })`.

---

## 4. T2: `select.rs` and `fanout.rs`, pure (D49, D52, D53, D60)

### 4.1 `crates/htui-orch/src/select.rs`

```rust
//! `R-AGT-8`'s walk (ANA-2 §7 `:1603-1615`, plan D60): pure over rows, no store, no engine.
use std::collections::BTreeMap;
use htui_agent::probe::{ProbeSnapshot, ProbeStatus};
use htui_agent::registry::caps_for;
use htui_core::model::{Agent, AgentBox, AgentId, Gate, RunStep, SnapshotCandidate};
use htui_core::model::quota::{self, Availability, SkipReason};

#[derive(Debug, Clone, Copy)]
pub struct SelectInput<'a> {
    pub candidates: &'a [SnapshotCandidate],        // phase_agent order
    pub agents: &'a BTreeMap<AgentId, Agent>,       // rows found; absence = NoAgentRow
    pub boxes: &'a BTreeMap<AgentId, AgentBox>,     // this box's rows; absence = unknown
    pub gate_effective: Gate,
    pub spent_micros: Option<i64>,                  // select::run_spend
    pub cap_micros: Option<i64>,                    // snapshot.settings.per_token_cap_run
    pub min_budget_micros: i64,                     // app_setting.min_budget_for_new_attempt, else 0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipCause {
    NoAgentRow,
    Quota(SkipReason),
    InlineApproval,
    NotReady(String),
    Budget { remaining: i64, min: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped { pub agent_id: AgentId, pub agent_name: String, pub cause: SkipCause }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Walk { pub eligible: Vec<SnapshotCandidate>, pub skipped: Vec<Skipped> }

impl Walk {
    /// `!skipped.is_empty() && eligible.is_empty() && every cause is InlineApproval`.
    #[must_use] pub fn only_inline_approval(&self) -> bool;
    /// `"<name> (<cause>), <name> (<cause>)"` in candidate order; `""` when nothing was skipped.
    #[must_use] pub fn summary(&self) -> String;
}

#[must_use] pub fn walk(input: &SelectInput<'_>) -> Walk;
/// Σ `usage["cost_micros"].as_i64()` over the steps (`model/run.rs:247`, `model/usage.rs:37`);
/// `None` when no step reports one.
#[must_use] pub fn run_spend(steps: &[RunStep]) -> Option<i64>;
```

**The rules, first match wins, per candidate:**
1. `agents.get(id)` is `None` → `NoAgentRow`.
2. `quota::available(boxes.get(id).and_then(|b| b.quota.as_ref()), spent, cap)` gives `Skip(r)` → `Quota(r)`. This call covers ANA-4's four rules and runs even when the box row is absent, so the cap rule still applies.
3. `gate_effective != Never` and `!(caps.permission_requests || caps.edit_proposals)` over `caps_for(agent)` (`registry.rs:152`) → `InlineApproval`.
4. Not ready:
   - `!agent.enabled` → `NotReady("agent disabled")`;
   - a box row with `!enabled` → `NotReady("agent_box disabled")`;
   - `ProbeSnapshot::from_row(row)` (`probe.rs:1123`) gives `Some(p)` with `p.status != ProbeStatus::Ready` → `NotReady(format!("probe {}", p.status.as_str()))`.
   - An absent or unparseable probe counts as unknown and does not skip.
5. `(Some(spent), Some(cap))` with `cap - spent < min_budget` → `Budget { remaining: cap - spent, min }`.

**`SkipCause` `Display`, byte-exact.** `SkipReason` has no `Display` (`quota.rs:356-378`), so this module owns the wording:

| Cause | Bytes |
|---|---|
| `NoAgentRow` | `no agent row` |
| `Quota(Exhausted)` | `quota: exhausted` |
| `Quota(WindowFull { id })` | `quota: window {id} full` |
| `Quota(Status(s))` | `quota: status {s}` |
| `Quota(CapReached { spent_micros, cap_micros })` | `quota: cap reached ({spent_micros} of {cap_micros} micros)` |
| `InlineApproval` | `inline_approval` |
| `NotReady(why)` | `not ready: {why}` |
| `Budget { remaining, min }` | `budget: {remaining} micros left, {min} required` |

**Tests** (the plan's ten; first failing: `the_first_unskipped_candidate_is_eligible_first`):
- `the_first_unskipped_candidate_is_eligible_first`
- `an_exhausted_quota_skips_and_names_the_rule`: build the document as `Quota { … }.to_value()`, the shape of `quota.rs:940-958`.
- `a_missing_agent_box_row_is_unknown_and_selected`
- `a_probe_that_is_not_ready_skips`: `status: failed`.
- `a_disabled_agent_or_agent_box_skips`
- `a_cli_row_is_skipped_only_at_a_gated_phase`: `claude-cli` is `transport: cli` (`seeds/agent_claude_cli.json`).
- `the_per_run_cap_is_the_callers_spend`: gives `Quota(CapReached { spent, cap })`.
- `min_budget_skips_only_when_cap_and_spend_are_known`
- `every_skip_being_inline_approval_is_reported_as_such`
- `run_spend_sums_cost_micros`: rows with and without `usage`; all `None` gives `None`.

### 4.2 `crates/htui-orch/src/fanout.rs`

```rust
pub const JUDGE_KIND: &str = "judge";
/// D49(4) and D65's reasons: the text of A-6's item notes and `select_fanout`'s `reason`.
pub const AUTO_WIN_REASON: &str = "the only candidate whose verify_command did not fail";
pub const HUMAN_PICK_REASON: &str = "selected by a human";

#[must_use] pub fn judge_phase_name(phase: &str) -> String;           // "<phase>:judge"

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeVerdict { pub winner: i32, pub reasons: BTreeMap<String, String> }

/// The **last** fenced ```` ```json ```` block of `body` (ANA-5 `:1347-1348`, `defaults.rs:189-190`).
pub fn parse_judge_verdict(body: &str) -> Result<JudgeVerdict, JudgeFailure>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeFailure {
    Unparseable(String),
    OutOfRange { winner: i32, survivors: Vec<i32> },
    Disagreement { forward: i32, reversed: i32 },
    MissingDocument { call: u32 },
    SessionFailed(String),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateView { pub step: StepId, pub fanout_index: i32, pub status: StepStatus, pub verify_outcome: Option<VerifyOutcome> }
impl CandidateView { #[must_use] pub fn of(step: &RunStep) -> Self; }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefilter { pub pool: Vec<CandidateView>, pub passing: Vec<CandidateView> }
/// pool = `done`; passing = pool whose `verify_outcome != Some(Fail)`; both in `fanout_index` order.
#[must_use] pub fn prefilter(candidates: &[CandidateView]) -> Prefilter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HumanReason { Gated, NoJudge, NoSurvivingCandidate, JudgeFailed(JudgeFailure), CandidateDropped(i32) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route { Human(HumanReason), AutoWin(StepId), Judge(Vec<StepId>), GroupFailed }

/// D49 in order:
/// (1) pool or passing empty → `gate == Never ? GroupFailed : Human(NoSurvivingCandidate)`;
/// (2) `Always` → `Human(Gated)`; (3) no judge → `Human(NoJudge)`;
/// (4) one passing → `AutoWin`; (5) `Judge(passing)`.
#[must_use] pub fn route(gate: Gate, has_judge: bool, pre: &Prefilter) -> Route;

/// Struct update over the judged phase: `name = judge_phase_name`, `gate`/`gate_effective = Never`,
/// `fan_out = 1`, `output_kind = JUDGE_KIND`, `input_kinds = []`, `verify_command = None`,
/// `isolation = Local`, `command_queue = Off`, `template`, `candidates = [judge]`, `judge = None` (F-G).
#[must_use] pub fn judge_phase(phase: &SnapshotPhase, template: SnapshotTemplate, judge: &SnapshotCandidate) -> SnapshotPhase;
/// Model: `judge.model` → `agent.default_model` → `agent.models[0]`; `None` when there is none.
#[must_use] pub fn judge_candidate(judge: &SnapshotJudge, agent: &Agent) -> Option<SnapshotCandidate>;
#[must_use] pub fn judge_inputs(task: String, candidates: Vec<JudgeCandidate>, reverse: bool) -> JudgeInputs;
```

**`Display`, byte-exact.** For `JudgeFailure`, `survivors` are joined with `", "`:

| Value | Bytes |
|---|---|
| `JudgeFailure::Unparseable(d)` | `judge_unparseable: {d}` |
| `JudgeFailure::OutOfRange { winner, survivors }` | `judge_out_of_range: {winner} not in [{a, b}]` |
| `JudgeFailure::Disagreement { forward, reversed }` | `judge_disagreement: forward {forward}, reversed {reversed}` |
| `JudgeFailure::MissingDocument { call }` | `judge_missing_document: call {call}` |
| `JudgeFailure::SessionFailed(e)` | `judge_session_failed: {e}` |
| `JudgeFailure::Unavailable(r)` | `judge_unavailable: {r}` |
| `HumanReason::Gated` | `gated` |
| `HumanReason::NoJudge` | `no judge configured` |
| `HumanReason::NoSurvivingCandidate` | `no_surviving_candidate` |
| `HumanReason::JudgeFailed(f)` | `{f}` |
| `HumanReason::CandidateDropped(i)` | `judge_candidate_dropped: {i}` |

**Parser.** Scan the body's lines (CRLF folded). A line whose trim equals ```` ```json ```` opens a block, and the next line whose trim equals ```` ``` ```` closes it. Keep the last complete block.
- No block → `Unparseable("no fenced json block")`.
- `serde_json::from_str::<Value>` error → `Unparseable(err)`.
- `winner` missing or not an integer that fits `i32` → `Unparseable("winner is not an integer")`.
- A missing `reasons` is an empty map. Non-string values in `reasons` are dropped.
- Range checking is the engine's job, because it holds the survivors.

**Tests** (first failing: `parse_judge_verdict_reads_the_last_fenced_json_block`): the plan's ten, including every row of the `Display` table and all rows of `route` (the three gates × judge present or absent × pool/passing sizes 0, 1, 2). `SnapshotJudge` is built via `serde_json` (F-H).

### 4.3 `lib.rs`

Add `pub mod fanout;` and `pub mod select;` in alphabetical order. Rewrite the doc paragraph at `lib.rs:8-14`: milestone 4 adds `select` and `fanout`; `overlap.rs`, `recover.rs` and `queue.rs` remain uncreated (milestones 5 and MOD-12). No re-exports.

---

## 5. T3: the `Isolator` seam for fan-out (D54, D55, D56, D57, D70, A-1..A-4)

### 5.1 `isolate.rs`

```rust
/// Plan D54(a): a candidate of a fan-out group. `base` is the group's `HEAD` per repo, read once
/// by the engine through [`Isolator::base`] or re-derived from rows (M2 D16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutSlot<'a> { pub index: i32, pub width: i32, pub base: &'a BTreeMap<RepoId, String> }

pub trait Isolator: Send + Sync + fmt::Debug {
    fn prepare<'a>(&'a self, run: RunId, step: StepId, scope: &'a [RepoId], isolation: Isolation,
                   slot: Option<FanoutSlot<'a>>) -> IsolatorFuture<'a, Prepared>;
    fn capture<'a>(&'a self, step: StepId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Vec<RunStepCommit>>;  // unchanged
    /// D54(b): `HEAD` of every repo in `scope`; empty scope → empty map.
    fn base<'a>(&'a self, scope: &'a [RepoId]) -> IsolatorFuture<'a, BTreeMap<RepoId, String>>;
    /// D54(c)/D55: `None` when no row committed, or when `git` is unusable.
    fn diff<'a>(&'a self, trees: &'a [RunStepTree], commits: &'a [RunStepCommit]) -> IsolatorFuture<'a, Option<DiffBlock>>;
    /// D54(d): `siblings` = the other candidates of the winner's slot; `&[]` for `fan_out = 1`.
    fn reconcile<'a>(&'a self, winner: StepId, trees: &'a [RunStepTree], siblings: &'a [StepId]) -> IsolatorFuture<'a, Vec<RunStepCommit>>;
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()>;  // unchanged
}
```

The `reconcile` doc (`:135-142`) is rewritten, replacing "Fan-out is milestone 4's": for a fan-out winner, `worktree`/`copy` merge only the winner and the losers' labels stay; `shared_serialized` moves the checked-out branch to the winner's label (D56). The `prepare` doc gains "a slot fixes `before_hash` to `slot.base[repo]` (D54(a))".

### 5.2 `isolate/git.rs`

- **Docs.** `:11-15` and `:156` say "the five verbs": change to **six**, adding `diff`, and say `diff` is the one verb whose stdout *is* the product. `:31-32` (`MIN_GIT`'s "five verbs") changes to six. `Exited::stdout` (`:789-790`) changes from "Never parsed" to "never parsed, except by [`Cli::diff`], whose stdout is the product".
- **Head buffer.** `pub struct HeadBuffer { kept: Vec<u8>, cap: usize, overflowed: bool }` with `new`/`push`/`into_string`. `into_string` appends `"\n[diff truncated at 64 KiB]"` when `overflowed`. `pub const DIFF_CAP: usize = 64 * 1024;`.
- **Capture mode.** `Cli::run` (`:325-383`) becomes a thin wrapper over `async fn run_capturing(&self, verb, cwd, args, extra_env, stdout: Capture) -> Result<Exited, IsolateError>`, with `enum Capture { Tail, Head }`. Stderr is always tail-captured. Existing verbs keep `Tail`.
- **The verb:**
  ```rust
  /// D55 + A-4: `git diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/
  /// [--stat] <before> <after> --`, `COLUMNS=80`, head-capped stdout. Not retried (M3 D39).
  pub async fn diff(&self, repo: &Path, before: &str, after: &str, stat: bool) -> Result<String, IsolateError>;
  ```
  A non-zero exit gives `exited.failure("diff")`. I verified the argv on git 2.43.0 on this box.
- **A-3's read:** `pub fn has_commit(path: &Path, hex: &str) -> Result<bool, IsolateError>`. It uses `gix::open` and `try_find_object`, calls no subprocess, and is reached through `blocking`.

**Tests**, each git-backed one opening with `let Some(git) = skip_without_git!() else { return; };`:
- `diff_and_diff_stat_of_a_range_match_git_s_own_output`: **first failing**. The oracle spawns the same argv with `COLUMNS=80`.
- `diff_ignores_an_external_diff_driver_and_noprefix`
- `diff_of_an_empty_range_is_empty`
- `a_diff_over_the_cap_keeps_its_head_and_says_it_was_truncated`: the output starts with `diff --git a/` and ends with the marker.
- `head_buffer_keeps_the_first_64_kib` (no git)

### 5.3 `isolate/copy.rs`

- `pub fn measure_within_cap(src: &Path, excludes: &[Exclude], cap: u64, copies: u64) -> Result<u64, IsolateError>`: `need.saturating_mul(copies) > cap` refuses.
- New sentence, `pub fn copy_over_cap_copies(need: u64, copies: u64, cap: u64) -> String` = `copy would need {need} bytes × {copies} copies = {total}; cap is {cap}`. When `copies <= 1`, the existing `copy_over_cap(need, cap)` is kept so M3's `copy_refuses_over_the_cap` keeps its bytes.
- Callers updated: `copy.rs:546`, `:549`, `:844` (tests) and `real.rs:609`.
- Test: `measure_refuses_when_size_times_copies_exceeds_the_cap`.

### 5.4 `isolate/real.rs`

- **Admin lock (D70 + A-2).**
  - Field `admin: Mutex<BTreeMap<RepoId, Arc<tokio::sync::Mutex<()>>>>`.
  - `async fn admin_guard(&self, repo: RepoId) -> OwnedMutexGuard<()>`: clone the `Arc` under the `std` lock, drop that lock, then `lock_owned().await`. This mirrors `acquire` at `:297-317`.
  - Test-only instrumentation: `#[cfg(test)] in_flight: AtomicU32, max_in_flight: AtomicU32`, incremented after the guard is taken and decremented before it drops. Accessor `#[cfg(test)] fn max_admin_in_flight(&self) -> u32`.
  - Wrapped call sites: both `add_worktree*` calls in `worktree_at` (`:502-505`, `:524-527`), both `remove_worktree` calls in `worktree_at` (`:499`, `:518`), `capture_worktree`'s removal (`:723`) and `remove_worktree_of` (`:743-748`). Each takes `repo: RepoId`.
  - The guard is held around the `with_retry(…)` call and nothing else.
- **`worktree_at`** gains `repo: RepoId`. Its `source_head` comes from the caller: `slot.base[repo]`, or `head_of` when there is no slot. A missing base entry is `Refused(no_slot_base(&checkout.name))`, with the sentence `no fan-out base for {name}`.
- **`prepare_copy` / `copy_at`**: same `source_head` rule. `measure_within_cap(…, cap, copies = slot.map_or(1, |s| s.width as u64))`.
- **`prepare_in_place(…, slot)`**:
  - `Local` with `Some(slot)` → `Refused(local_cannot_fan_out())`, sentence `local isolation cannot fan out`. Unreachable past `graph.rs:425-429`.
  - `SharedSerialized` with a slot, after `acquire` (A-1): per repo,
    - `is_dirty(local)` → `Refused(dirty_tree_not_reset(&local))`, sentence `dirty_tree_not_reset: {path}`;
    - otherwise `head != base` → `with_retry(reset_hard(local, base))`.
    - Then the existing body runs, with `before = base` and `dirty = false`.
  - The existing error path releases the guard (`:911-916`).
- **`base(scope)`**: `resolve_scope` then `head_of` for each repo, as a `BTreeMap`.
- **`diff(trees, commits)`**: `git` is `Err` → `Ok(None)`. For each commit row with `after_hash: Some(a)` and `a != before`:
  - choose the repo per A-3 (copy: `has_commit(tree.path, a)`, else the checkout; other modes: the checkout);
  - run `diff(…, stat = true)` and `diff(…, stat = false)`.
  - No such row → `Ok(None)`.
  - One row → `DiffBlock { range: "{b}..{a}", stat, diff }`.
  - Several rows → `range: "{name}:{b}..{a}, …"`, with `stat`/`diff` concatenated under a `# repo {name}\n` line each, in slice order (F-J).
- **`reconcile_in_place(step, tree, checkout, siblings)`** for `shared_serialized` (D56):
  - `label(winner) == head` → existing identity.
  - Otherwise `is_dirty` → `Refused(dirty_primary_tree())`.
  - Otherwise `head == base_ref` or `head == label(s)` for some `s` in `siblings` → `reset_hard(local, label(winner).unwrap_or(base_ref))`; `after = label(winner).filter(|l| l != base_ref)`.
  - Otherwise `Refused(primary_moved(head))`.
  - `worktree`/`copy` ignore `siblings`.
- **~44 test call sites** in `real.rs` gain `None` / `&[]`.

**New tests:**
- `base_reads_each_repo_s_head`: **first failing**.
- `worktree_candidates_all_start_from_the_slot_base_even_after_the_primary_moved`
- `copy_candidates_reset_to_the_slot_base`
- `shared_serialized_siblings_reset_a_clean_checkout_to_base`
- `shared_serialized_refuses_a_dirty_checkout_with_a_slot`: includes HEAD == base (A-1).
- `shared_serialized_reconcile_moves_the_branch_to_the_winner`
- `shared_serialized_reconcile_refuses_a_head_no_sibling_left`
- `diff_spans_every_repo_under_a_header_line`
- `diff_is_none_without_git`
- `copy_diff_after_reconcile_reads_the_checkout` (A-3)
- `concurrent_worktree_prepares_on_one_repo_never_overlap_their_adds`: `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]`, 24 `tokio::spawn`s over an `Arc<GixIsolator>`, then `max_admin_in_flight() <= 1`.
- `adds_on_two_repositories_do_not_wait_for_each_other`: two repositories; each repo's lock is held by a test-held guard in turn.
- `a_no_slot_prepare_is_unchanged`

### 5.5 `fake.rs` (`FakeIsolator` only)

- `base(scope)` → `repo → format!("fake:group-base:{repo}")`. It **does not `tick()`**, because M2's hash assertions (`fake.rs:1178-1179`, `:1226`, `:1240`) depend on the counter.
- `prepare` with a slot uses `slot.base.get(repo)` for `base_ref` and `before_hash`, falling back to `tick()`. It still counts `prepares`.
- New field `diffs: Mutex<VecDeque<Option<DiffBlock>>>` and method `pub fn script_diff(&self, block: Option<DiffBlock>)`. `diff` pops the queue; unscripted answers `Ok(None)`.
- `reconcile` ignores `siblings`.
- The test call sites `:1165`, `:1190` and `:1205` gain `None`.
- Tests: `the_fake_honours_the_slot_base`, `the_fake_diff_is_scripted_and_none_by_default`.

### 5.6 `engine.rs` (T3's two lines only)

- `:868-872`: `.prepare(run.id, step.id, &run.repo_scope, phase.isolation, None)`
- `:1157`: `.reconcile(step.id, &trees, &[])`

---

## 6. T4: snapshot, caps, template pin, tests-only writer (D53, D63, D64, D69)

**First failing test**: `a_snapshot_judge_without_a_template_still_decodes` in `run.rs`.

- **`model/run.rs:545-553`.** Add `#[serde(default)] pub template: Option<SnapshotTemplate>` to `SnapshotJudge`. Doc: "`graph::resolve` pins the latest `judge` template at run creation (D53); `None` in a snapshot written before M4 means latest at judge time." `GraphSnapshot::V` stays 1.
- **`store/mem.rs`**, beside `set_app_setting` (`:434-443`):
  ```rust
  /// Replaces one project's `settings` blob without validation. **Tests only** — same reason as
  /// [`set_app_setting`](Self::set_app_setting): no seam writer reaches `project.settings`.
  pub fn set_project_settings(&self, project: ProjectId, settings: Value)
  ```
  It sets `settings` and stamps `updated_at = Utc::now()`, as `set_app_setting` does. Test: `set_project_settings_is_read_back_by_project`.
- **`graph.rs`:**
  - `:485` literal: add `template: judge_template(source, project).await?`. The helper returns `prompt_template(project, "judge", None)` mapped to `SnapshotTemplate`, or `None` when absent (no refusal).
  - `snapshot_phase` (`:424-430`), next to `LocalFanOut`: `phase.name == "review" && phase.fan_out > 1` → `ReviewFanOut { phase, fan_out }`.
  - In `resolve`, after `phases` and the caps are known (`:282-300`), in this order:
    - any phase with `fan_out as u32 > max_fan_out` → `FanOutCap { phase, fan_out, max }`;
    - `planned = Σ fan_out + #(fan_out > 1 && judge.is_some())`, and `planned > max_agents_per_run` → `AgentCap { planned, max }`.
  - New variants:
    ```rust
    #[error("phase `{phase}` fans out {fan_out}; max_fan_out is {max} (ANA-2 :868)")]
    FanOutCap { phase: String, fan_out: i32, max: u32 },
    #[error("run plans {planned} agents; max_agents_per_run is {max} (ANA-2 :870-875)")]
    AgentCap { planned: u32, max: u32 },
    #[error("phase `{phase}` is a review with fan_out {fan_out}; a review cannot fan out (plan D64)")]
    ReviewFanOut { phase: String, fan_out: i32 },
    ```
- **Tests**: the plan's list, plus `feature_snapshot_topology_is_pinned` **unchanged** (`:724`). `the_project_rung_answers_when_the_phase_does_not` also asserts `judge.template == Some(SnapshotTemplate { name: "judge", version: 1 })`.

---

## 7. T5: groups in `status.rs` and `gate.rs` (D66; D59 helpers)

**`status.rs`:**
```rust
/// Stage 1 left nothing (D62). `detail` empty → no "; " suffix (the StartRun rung-4 note).
NoCandidateAgent { phase: String, detail: String },   // "no_candidate_agent: phase `{phase}`; {detail}"
NoSurvivingCandidate { phase: String },               // "no_surviving_candidate: {phase}"

/// The candidates (`fanout_index >= 0`) of `(position, attempt)`, in `fanout_index` order.
#[must_use] pub fn group_at(steps: &[RunStep], position: i32, attempt: i32) -> Vec<&RunStep>;
/// The judge row (`fanout_index = -1`) of the slot.
#[must_use] pub fn judge_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep>;
/// D66: the `selected = true` candidate, else `fanout_index 0`.
#[must_use] pub fn winner_at(steps: &[RunStep], position: i32, attempt: i32) -> Option<&RunStep>;
```

`run_failure_display_is_ana2s_bytes` gains three rows:
- `no_candidate_agent: phase `implement`; claude (quota: status rejected)`
- `no_candidate_agent: phase `prd`` (empty detail)
- `no_surviving_candidate: implement`

Other tests: `winner_at_prefers_the_selected_candidate` (**first failing**), `winner_at_falls_back_to_index_zero_when_nothing_is_selected`, `group_at_excludes_the_judge`. **No `Cursor` change.**

**`gate.rs`:**
```rust
/// D66 / F-C: every row at `(position, max attempt over all rows at position)`, judge included,
/// by M2 D5's status split (`:866-886` today). `Pending|AwaitingApproval|Done → supersede_step`;
/// `Running|Failed → cancelled`; `Superseded|Cancelled` untouched.
pub(crate) async fn retire_slot<S: WriteStore, C: Clock + ?Sized>(
    ctx: &GateContext<'_, S, C>, steps: &[RunStep], position: i32, now: DateTime<Utc>,
) -> Result<(), EngineError>;
```

- `retire` (`:854-890`) becomes `for position in from..=to { retire_slot(…).await?; }`.
- `at()` (`:842-851`) is deleted. `commits_are_identical` uses `winner_at(steps, target, attempt).map(|s| s.id)`.
- `commits_are_identical`'s doc gains D66's R-8 ancestry paragraph.
- `WALKED_FANOUT_INDEX` (`:26`) goes if nothing else in the file reads it.

**Tests over `MemStore::demo()`.** `RUN_3` holds index 0, `selected = true`, `done`, and index 1, `selected = false`, `superseded` (`fixtures.rs:1474-1491`). Each test inserts its own judge or third candidate with `create_step` + `transition_step`.
- `retire_supersedes_every_candidate_and_the_judge_of_the_slot`
- `retire_cancels_a_failed_candidate_and_a_failed_judge`
- `no_progress_compares_the_two_winners_not_index_zero`: attempt 2's winner is at index 1.

---

## 8. T6: stage 1 is `R-AGT-8`'s walk; rung 3/4; D67; D68; D71

### 8.1 `graph.rs`

- `GraphSource` (`:53-91`) gains `async fn agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>>;`. It is implemented by:
  - `impl GraphSource for MemStore` (`fake.rs:506`): delegates to the inherent `agent_boxes` (`mem.rs:521`); the inherent method wins resolution;
  - `FakeGraphSource` (`fake.rs:610`): `GraphSource::agent_boxes(self.store, box_id)`;
  - `TestSource` (`graph.rs:634`): `self.store.agent_boxes(box_id)`.
- `resolve(store, source, item, mode, app, requested_scope, box_id: BoxId)`. `box_id` is the **last** parameter. It is threaded into `snapshot_phase` and `candidates`.
- **Rung 3** in `candidates` (`:529`), only when `default_agent_id` is `None`, so rung-2 semantics are unchanged:
  - rows = `source.agent_boxes(box_id)` where the box row is `enabled` and `source.agent(id)` is `Some` and `enabled`;
  - exactly one row → a `SnapshotCandidate` with model `default_model` → `models[0]`, else `NoCandidate`;
  - otherwise `NoCandidate`.
  - The doc at `:494-501` is rewritten.
- Callers updated: `engine.rs:322`, `:681`, `:3658` (with `self.parts.box_id` or `orch.box_id()`), `fake.rs:1120`, `graph.rs` tests, `tests/fixtures.rs:14-21`, `:106-114`.
- Tests: `rung_three_is_the_single_enabled_agent_on_this_box`, `two_enabled_agents_are_not_a_rung_three`. The rows come from `upsert_agent_box` on `MemStore::demo()`.

### 8.2 D68 and D71 surfaces (`engine.rs`)

```rust
/// Plan D68: what one session is, for the driver factory and the sink. `phase` is
/// `run_step.phase_name` (`<phase>:judge` for a judge); `call` is the judge's 0/1, else 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionKey<'a> { pub phase: &'a str, pub attempt: i32, pub fanout_index: i32, pub call: u32 }

pub type DriverFor<'a> = &'a (dyn Fn(&SnapshotCandidate, &SessionKey<'_>) -> Box<dyn AgentDriver> + Sync);

// SessionSink::after_done(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase,
//                         key: &SessionKey<'_>, done: &DoneEvent) -> Result<(), StoreError>;
// AgentSelector::select<'c>(&self, phase: &SnapshotPhase, eligible: &'c [SnapshotCandidate],
//                           fanout_index: i32) -> Option<&'c SnapshotCandidate>;   // FirstCandidate ignores it
```

- `session` (`:1406-1481`) builds `SessionKey { phase: &step.phase_name, attempt: step.attempt, fanout_index: step.fanout_index, call: 0 }` and passes it to both the driver factory (`:1424`) and `after_done` (`:916`).
- **Closures must annotate both parameter types** (H-18), for example `|_c: &SnapshotCandidate, key: &SessionKey<'_>| orch.driver_for_key(key)`. This applies to `dispatch_fake`/`resume_fake` (`:1816-1817`, `:1833-1834`), the engine test at `:3583`, and `tests/gix_isolator.rs:143`.
- `lib.rs` re-exports `engine::SessionKey`, `select::{walk, SkipCause, Walk}`.

**`fake.rs` (`FakeOrchestrator`)**:
- `scripts` key becomes `(String, i32, Option<(i32, u32)>)`.
- `script(phase, attempt, step)` stores `None` and is unchanged for callers.
- New `pub fn script_candidate(&self, phase: &str, attempt: i32, fanout_index: i32, call: u32, step: ScriptedStep)`.
- `script_for_key(&SessionKey)`: exact key, else `(phase, attempt, None)`, else the default.
- `driver_for_key(&SessionKey)`. `driver_for(phase, attempt)` is kept, delegating with `(0, 0)`.
- `after_done(item, step, phase, key)` reads `script_for_key(key).output`.
- `CommittingSink::after_done` (`tests/gix_isolator.rs:358-380`) takes the key and forwards it.

### 8.3 Stage 1 (`engine.rs`)

```rust
/// D60: the walk's inputs, read fresh every call (the recorder latches quota mid-run).
async fn stage_one(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase) -> Result<Walk, EngineError>;
fn min_budget(app: &BTreeMap<String, Value>) -> i64;   // "min_budget_for_new_attempt", positive i64 else 0 (H-19)
/// D62 in blueprint H-16's order; replaces `refuse_capability` (`:771-808`).
async fn refuse_no_candidate(&self, run: &Run, phase: &SnapshotPhase, walk: &Walk) -> Result<Rest, EngineError>;
/// D60 "never substitute silently": skipped rows ranked above `chosen` in `phase.candidates`.
async fn note_substitution(&self, run: &Run, phase: &SnapshotPhase, attempt: i32, fanout_index: i32,
                           chosen: &SnapshotCandidate, walk: &Walk) -> Result<(), EngineError>;
async fn admit(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase, attempt: i32) -> Result<Option<Rest>, EngineError>;
```

- `admit` (`:726-768`) runs `stage_one`, then `select(phase, &walk.eligible, 0)`. `None` goes to `refuse_no_candidate`. Otherwise it writes `note_substitution` and then `create_step { fanout_index: 0, … }`.
- Call sites gain `snapshot`: `:643`, `:979`, `:536`.
- Failure selection: `walk.only_inline_approval()` → `RunFailure::MissingCapability`, and the note keeps the shipped bytes `"{failure} (phase `{p}`)"` (`:788`). Otherwise `RunFailure::NoCandidateAgent { phase, detail }`, where detail is `walk.summary()` or `"the selector declined every eligible candidate"` when it is empty; the note is its `Display`.
- Substitution note bytes: `` stage 1 at `{phase}` attempt {a} (candidate {i}): skipped {name} ({cause}); chose {chosen_name}/{model} ``, with `via_step_id: None`.
- **`start_run` (`:315-331`)**: on `Err(ResolveError::NoCandidate { phase })` it calls `transition(item, Open, Blocked)`, where `Ok(false)` for a `failed` item is fine, and writes `add_note(RunFailure::NoCandidateAgent { phase, detail: "" })`. Then it returns the error.
- **D67 in `assemble_prompt` (`:1275-1393`)**, when the role is `Phase` and `step.attempt > 1`:
  - `prev = status::winner_at(&run_steps, step.position, step.attempt - 1)`;
  - `verify_failure = (prev.verify_outcome == Some(Fail)).then(...)` built from `command_runs(prev.id).last()` as `VerifyFailure { exit_code: prev.verify_exit_code.or(row.exit_code)?, output: row.output.unwrap_or_default() }`; a missing code writes a note instead;
  - `previous_diff = isolator.diff(&step_trees(prev), &step_commits(prev))`: `Ok(d)` → `d`, `Err(e)` → `None` plus `notes.push(format!("previous_diff unavailable: {e}"))`.
  - The comment at `:1376-1380` is replaced.

**Engine unit test**: `a_selector_is_asked_with_the_fanout_index`. An `IndexSelector` picks `eligible[index % len]`. The step's `agent_id` is the index-0 choice. `EngineParts` is built by literal, as at `:3590`.

### 8.4 Cases (`CASES` 18 → 23; first failing: `allowed_warning_candidate_is_selected`)

| Case | Setup | Asserts |
|---|---|---|
| `allowed_warning_candidate_is_selected` | `free_feat_3`; `upsert_agent_box(claude, BOX)` + `set_agent_box_quota(Quota { status: Some("allowed_warning"), … }.to_value())` | the prd step runs on `claude` and parks |
| `a_skipped_candidate_falls_through_to_the_next` | `with_candidates("prd", [(claude, "sonnet"), (agy, "gemini-3.7-flash-high")])`; claude's quota `rejected` | the step's `agent_id == agy`; exactly one note naming `claude (quota: status rejected)` and `agy` |
| `no_candidate_agent_blocks_the_item` | `without_candidates("prd")` | `Err(Resolve(NoCandidate))`; no new run; item `blocked`; note `` no_candidate_agent: phase `prd` `` |
| `every_candidate_skipped_refuses_the_run` | claude is the only candidate, quota `rejected` | run `failed`, `failure` starts `no_candidate_agent: phase `prd`; claude (quota: status rejected)`; item `blocked` |
| `a_second_attempt_carries_verify_failure_and_previous_diff` | `repoint` prd `gate = Never`, `verify_command = Some("cargo test")`, `retry_limit = 1`; `verifier.script_report(fail(1))`; `isolator.script_diff(Some(block))` | the seq-0 `prompt` payload of attempt 2 has `verify_failure` and `previous_diff` sections |

Both pins move in commit (d):
- `conformance.rs:1961-1975` is renamed `cases_are_unique_and_twenty_three`, and its message enumerates the five additions.
- `tests/fake_conformance.rs:14-17` becomes `cases_len_is_twenty_three`.

---

## 9. T7: the fan-out drive, the judge, selection, `SelectFanout`

### 9.1 Commit (a): plumbing with no behaviour change

- `crates/htui-orch/Cargo.toml`: `futures = { workspace = true }`, with a comment citing D58. `Cargo.lock` gains one line.
- **F-A**: `output_of(item, phase, step)` is rewritten over `documents(item)` heads, then `document(id)`.
- The session is split so the judge can pump twice through one recorder:
  ```rust
  async fn open_recorder(&self, run: &Run, step: &RunStep, prompt: &AssembledPrompt) -> Result<Recorder<'a, S>, EngineError>; // new + cap + record_prompt
  async fn drive_once(&self, run: &Run, step: &RunStep, phase: &SnapshotPhase, key: &SessionKey<'_>,
                      text: &str, cwd: PathBuf, extra_dirs: Vec<PathBuf>, recorder: &mut Recorder<'a, S>)
      -> Result<Result<DoneEvent, DriverError>, EngineError>;   // driver(key) → start → pump; spawn refusal → Err(Driver)
  ```
  `session` becomes `open_recorder` + `drive_once` + `recorder.finish()`, with the same behaviour.

### 9.2 Commit (b): the command surface (`command.rs`)

```rust
/// §6.2 `select` (`docs/ANA-2.md:1566`, criterion 10). Moves `SelectFanout` out of `:28-30`'s list.
SelectFanout { run: RunId, position: i32, attempt: i32, winner: StepId },
// CommandOutcome
Selected { rest: Rest },
// EngineError
#[error("run {run} position {position} attempt {attempt} holds one candidate; there is nothing to select")]
NotAFanout { run: RunId, position: i32, attempt: i32 },
#[error("run {run} position {position} attempt {attempt} already selected step {selected}")]
AlreadySelected { run: RunId, position: i32, attempt: i32, selected: StepId },
#[error("step {step} is not a candidate of run {run} position {position} attempt {attempt}")]
NotACandidate { step: StepId, run: RunId, position: i32, attempt: i32 },

/// D65. `slot` = `status::group_at` of the slot. Not enabled: run not `awaiting_approval`
/// (`RunStatus`), `slot.len() < 2` (`NotAFanout`), any `selected == Some(true)` (`AlreadySelected`),
/// winner not in slot (`NotACandidate`), winner not `done|awaiting_approval`
/// (`NotGated { expected: "done | awaiting_approval" }`). R-7 is stated in the doc.
pub fn select_enabled(run: &Run, slot: &[&RunStep], winner: StepId, position: i32, attempt: i32) -> Result<(), EngineError>;
/// F-D. Run `awaiting_approval`; no `selected == Some(true)`; `may_attempt(attempt + 1, retry_limit)`.
pub fn retry_group_enabled(run: &Run, slot: &[&RunStep], phase: &SnapshotPhase) -> Result<(), EngineError>;
```

Tests: `select_enabled` refuses (run not parked, single candidate, already selected, winner not a candidate, winner `failed`). `retry_group_enabled` refuses (spent budget, selected slot).

**Engine**, in the `dispatch` arm:

```rust
async fn select_fanout(&self, run: RunId, position: i32, attempt: i32, winner: StepId) -> Result<CommandOutcome, EngineError>
```
1. Read the run, snapshot, phase and steps; build the `group_at` slot; call `select_enabled`.
2. `now`; `store.select_fanout(run, position, attempt, winner, Some(HUMAN_PICK_REASON))`.
3. A-6 note: `` fan-out `{phase}` attempt {a}: candidate {i} selected by a human ``, with `via_step_id = winner`.
4. `unpark`.
5. Re-read the winner and call `reconcile_done_step(&run, &winner, &siblings)`. `Some(rest)` → `Selected { rest }`.
6. Otherwise `run_to_rest` → `Selected { rest }`.

`reconcile_done_step(run, step, siblings: &[StepId])` passes `siblings` to `isolator.reconcile`. Its other call sites (`:410`, `:975`) pass `&[]`.

**`retry_step`**: when `phase.fan_out > 1` (checked before `retry_enabled`, `:506`):
1. build the slot at `(row.position, row.attempt)`;
2. `retry_group_enabled`;
3. `gate::retire_slot(&ctx, &steps, position, now)`;
4. `unpark`;
5. `admit(run, snapshot, phase, attempt + 1)`, then `run_to_rest`, returning `Retried { step: row.id, rest }`.

### 9.3 Commit (c): the group-aware cursor and the drive

**`status.rs`:**

```rust
/// D59: fewer than `fan_out` candidates exist, or one is `pending`.
Fan { position: i32, attempt: i32 },
/// D59: every candidate settled, none selected.
Select { position: i32, attempt: i32 },
```

`cursor` keeps its current body for `phase.fan_out <= 1`, reading index 0 as `latest_at` does. For `fan_out > 1`, the order is load-bearing:
1. `latest_at` is `None` → `Create { position, 1 }`.
2. Let `a` be its attempt and `g = group_at(p, a)`.
3. Any `g` row `selected == Some(true) && Done` → the position is complete; continue.
4. `g` is non-empty and every row is `Superseded | Cancelled` → `Create { p, a + 1 }`.
5. Any `g` row or `judge_at(p, a)` is `Running` → `Rest { step, Running }`.
6. `g.len() < fan_out` or any row is `Pending` → `Fan { p, a }`.
7. Otherwise → `Select { p, a }`.

Tests:
- `cursor_drives_an_incomplete_group` (**first failing**)
- `cursor_selects_a_settled_group`
- `cursor_passes_a_selected_group`
- `cursor_creates_the_next_attempt_after_a_retired_group`
- `cursor_reads_the_zeroth_fanout_index_only`, rewritten for a `fan_out = 1` phase
- `a_running_judge_rests` (added)

**`engine.rs`**: `run_to_rest` gets two new arms, and `resting` maps `Fan { position, .. } | Select { position, .. } → Some(position)` (`:1534-1541`):

```rust
Cursor::Fan { position, attempt } => { let phase = Self::phase_at(run, &snapshot, position)?;
    if let Some(rest) = self.drive_group(&row, &snapshot, &phase, attempt).await? { return Ok(rest); } }
Cursor::Select { position, attempt } => { let phase = Self::phase_at(run, &snapshot, position)?;
    if let Some(rest) = self.select_stage(&row, &snapshot, &phase, attempt).await? { return Ok(rest); } }
```

`WALKED_FANOUT_INDEX` (`:50`) is removed. `admit` covers `fan_out = 1` as the case of a single index.

**`admit`, generalised (D71, F-K).** It becomes `admit_indices(run, snapshot, phase, attempt, indices: &[i32])`, and `admit` calls it with `0..phase.fan_out`:
1. Run `stage_one` once.
2. `picks = indices.map(|i| selector.select(phase, &walk.eligible, i).cloned())`. Any `None` → `refuse_no_candidate`, with nothing written.
3. For each `(i, chosen)`: `note_substitution(…, i, &chosen, &walk)`, then `create_step { fanout_index: i, agent_id: Some(chosen.agent_id), model: Some(chosen.model) }`.

Test: `a_fanout_keyed_selector_may_choose_differently_per_index`.

**`drive_group`**:

```rust
async fn drive_group(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase, attempt: i32) -> Result<Option<Rest>, EngineError> {
    // 1. missing = (0..fan_out) − group_at indices → admit_indices(missing); a refusal returns.
    // 2. slot = group_at (re-read); pending = its `Pending` rows.
    // 3. base = self.group_base(run, &slot)  // any candidate's step_commits → before_hash map; else isolator.base(&run.repo_scope)
    //    (an error here propagates: nothing is live, the next call re-derives — H-22)
    // 4. prompt = assemble_prompt(run, snapshot, pending[0], phase, item)  // once (D58)
    //    Err(missing) → A-5's fail_group_before_a_token(run, phase, &pending, missing)
    // 5. let slot_ref = FanoutSlot { index, width: phase.fan_out, base: &base } per candidate;
    //    futures::future::join_all(pending.iter().map(|s| self.run_candidate(CandidateStage { run, snapshot, phase, step: s, prompt: &prompt, base: &base })))
    //    → first Err propagates (only a settle write can Err: run_candidate owns every other failure)
    // 6. Ok(None) — the next pass sees `Select`.
}
```

**Borrows.** Every future borrows `&self`, whose `EngineParts` are `&'a`, and locals of `drive_group`'s frame: `prompt`, `base` and `pending`. `join_all` is awaited in that same frame, so no `'static` bound applies, which is D58's reason for not using `JoinSet`. The futures do not need to be `Send` today. The existing rule still holds: no `std` guard across an `.await`.

**`run_candidate`** (D48, A-7). It never propagates an error except when the failure write itself fails.

```rust
struct CandidateStage<'s> { run: &'s Run, snapshot: &'s GraphSnapshot, phase: &'s SnapshotPhase, step: &'s RunStep,
                            prompt: &'s AssembledPrompt, base: &'s BTreeMap<RepoId, String> }
async fn run_candidate(&self, c: CandidateStage<'_>) -> Result<(), EngineError>;
```

1. `transition_step(Pending → Running)`. `false` → `Ok(())`.
2. `let mut trees = None; let mut captured = false;` then run `candidate_live(&c, started_at, &mut trees, &mut captured)`:
   - `prepare(…, Some(FanoutSlot { index: step.fanout_index, width: phase.fan_out, base }))`, then `upsert_step_tree`, then `record_commits(before)`;
   - `set_step_prompt(step, &prompt.digest, trim)`;
   - `open_recorder`, `drive_once(key { call: 0 })`, `finish`;
   - on `Ok(done)` → `sink.after_done(…, &key, &done)`;
   - `verify(…)`;
   - `capture`, recording `captured = true`, then `record_commits(after)`;
   - `output_of`;
   - `settle(SettleInput { verify_outcome: None, … })` (D48);
   - `finish_step` with the **real** `verify_outcome`/`verify_exit_code`;
   - `Settle::Ok { note }` → `transition_step(Running → Done)` plus a note if `Some`;
   - `Settle::Failed(f)` → `transition_step(Running → Failed)` plus the note `` fan-out candidate {i} of `{phase}` attempt {a}: {f} `` with `via_step_id`;
   - `Settle::Rejected` cannot happen (D64) and is mapped to `Failed`.
3. On `Err(err)`: if `trees.is_some() && !captured`, call `isolator.capture(step, trees)` best-effort; `Ok(c)` goes to `record_commits`, `Err` is `warn`ed (A-7). Then `transition_step(Running → Failed)` (`Ok(false)` tolerated) and the same note with `err`. **No `finish_run`, no `cleanup_run`** (D48).

**`select_stage`** (D49):

```rust
async fn select_stage(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase, attempt: i32) -> Result<Option<Rest>, EngineError>
```
1. Read `steps`, `slot = group_at`, `judge = judge_at`.
2. `judge.status == Failed` → a crash between D51's fail and the park. `park_selection(…, judge.gate_note or "judge failed")`. There is no re-judge (D59).
3. `judge.status == Pending` → `run_judge(…, Some(judge))` with the passing set recomputed.
4. Otherwise `route(phase.gate_effective, phase.judge.is_some(), &prefilter(views))`:
   - `GroupFailed`:
     - if `may_attempt(attempt + 1, retry_limit)`: `retire_slot`, then `Ok(None)`, and the cursor gives `Create { attempt + 1 }`;
     - otherwise `finish_run(Failed, NoSurvivingCandidate { phase })` + `cleanup_run` → `Rest { Failed, Some(p), Some(failure) }`.
   - `Human(r)` → `park_selection(run, phase, attempt, &slot, &r.to_string())`.
   - `AutoWin(w)` → `select_fanout(…, w, Some(AUTO_WIN_REASON))`, the A-6 note, then `reconcile_done_step(run, winner, siblings)`, where `Some(rest)` returns and `None` → `Ok(None)`.
   - `Judge(passing)` → `run_judge(…, passing, None)`.

**`park_selection`** (D50). It is `park_run`'s shape (`:1188-1221`) with a different note:

```rust
async fn park_selection(&self, run: &Run, phase: &SnapshotPhase, attempt: i32, slot: &[&RunStep], reason: &str) -> Result<Rest, EngineError>;
```
- Writes: `transition_run(Running → AwaitingApproval)`, `transition(item, InProgress → AwaitingApproval)`, then the note.
- Note body: `` fan-out `{phase}` attempt {a} awaits selection: {reason}; candidates: {i} {status} (verify {outcome|none}), … ``.
- For `SharedSerialized` it adds `; the shared checkout stays at the last sibling's commit (base {repo}@{hash}; labels htui/{step}, …)`, as the plan's Risks row requires.
- Returns `Rest { AwaitingApproval, Some(position), None }`. `run.failure` stays NULL (R-3).

**`run_judge`** (D51–D53, F-G, F-I, H-8):

```rust
async fn run_judge(&self, run: &Run, snapshot: &GraphSnapshot, phase: &SnapshotPhase, attempt: i32,
                   passing: Vec<StepId>, existing: Option<RunStep>) -> Result<Option<Rest>, EngineError>
```
1. **Inputs, before any judge row** (H-8):
   - `task` = `step_events(lowest-index passing)` → the `seq == 0 && kind == Prompt` row → `payload["text"]`;
   - for each passing candidate, a `JudgeCandidate` with:
     - `verify`: `Pass` → `Some(true)`, `Fail` → `Some(false)`, otherwise `None`;
     - `exit_code` = `verify_exit_code`;
     - `document` from `output_of`;
     - `diff` from `isolator.diff(step_trees, step_commits)` (`Err` → `None` plus a note);
     - `verification_tail` from `command_runs().last().output`.
   - The template is `phase.judge.template`, else `graphs.prompt_template(project, "judge", None)`.
   - Build `judge_spec(reverse)`: role `Judge`, **`phase: phase.name`** (F-G), `output_kind: Some(JUDGE_KIND)`, budget from `settings::resolve_budget(phase.token_budget, …)`, `judge: Some(judge_inputs(task, candidates, reverse))`, and every other section empty.
   - Assemble `forward` and `reversed`.
   - Failures at this point (no prompt row, no template, `AssembleError`) become a `JudgeFailure` handled at step 3: `SessionFailed(err)` or `Unavailable("no `judge` template")`.
2. **Dropped candidate**: `forward.trim.sections` has `name == SectionName::JudgeCandidate(i) && strategy == TrimStrategy::Dropped` → `park_selection(HumanReason::CandidateDropped(i))`. **No judge row is created.**
3. **The judge row**:
   - `existing`, or `create_step { fanout_index: -1, phase_name: judge_phase_name(&phase.name), agent_id, model }`, where:
     - `agent = graphs.agent(judge.agent_id)`; `None` → the row is created with `agent_id: None`, and `Unavailable("no agent row")` (F-I);
     - `judge_candidate(judge, &agent)`; `None` → `Unavailable("no model")`;
     - `select::walk` over `[candidate]` with `gate_effective: Never`, skipped → `Unavailable(cause)`.
   - `transition_step(Pending → Running)`; `false` → `Ok(None)`.
   - Any failure so far goes to `fail_judge`.
4. **Two calls** (D52). Every `EngineError` here is caught and converted into `SessionFailed`:
   - `prepare(run, judge.id, &[], Isolation::Local, None)`, which yields a scratch `cwd` and no rows;
   - `open_recorder(forward)` (`record_prompt` at seq 0) and `set_step_prompt(judge, forward.digest, trim)`;
   - `jp = judge_phase(phase, template, &candidate)`;
   - call 0: `drive_once(key { phase: &judge.phase_name, attempt, fanout_index: -1, call: 0 }, forward.text)` → `after_done(…, &jp, &key0, &done)` → `doc0 = output_of(item, &jp, judge.id)`, and `None` → `MissingDocument { call: 0 }`;
   - `recorder.record_follow_up(&reversed.text, now)` (turn 1);
   - call 1: the same with `call: 1` and `reversed.text` → `doc1`, where `None` or `doc1.id == doc0.id` → `MissingDocument { call: 1 }`;
   - `recorder.finish()`. A cap breach or a pump `Err` is a `SessionFailed`.
   - Parse both documents. Each winner must be in `passing_indices`, else `OutOfRange`. If they differ, `Disagreement`.
5. **Success**:
   - `finish_step(judge, { finished_at: now, … None })`;
   - `reason = forward.reasons[winner] or "judge: {winner}"`;
   - `select_fanout(run, p, a, winner_step, Some(reason))`, which moves the judge `Running → Done` with `gate_note` (`mem.rs:3574-3586`);
   - `reconcile_done_step(run, winner, siblings)`.
6. **`fail_judge(judge, failure)`**:
   - `transition_step(Running → AwaitingApproval)`, then `answer_gate(Rejected, Some(failure.to_string()))`, the `gate.rs:403-423` pattern;
   - then `park_selection(…, &failure.to_string())`.

**A-5**: `fail_group_before_a_token(run, phase, pending, kind)`. For each pending candidate: `Pending → Running → Failed` plus a note. Then `finish_run(Failed, MissingInput(kind))`, then `cleanup_run`, returning `Rest { Failed, Some(p), Some(MissingInput) }`.

### 9.4 Commit (d): cases, pins, re-exports

- **Conformance harness**: `Orchestrate` gains `fn script_candidate(&self, phase: &str, attempt: i32, fanout_index: i32, call: u32, step: ScriptedStep)`.
- **`fake.rs`**: `ScriptedStep::judge(winner: i32, reasons: &[(i32, &str)]) -> Self`. Its body is a paragraph followed by exactly one fenced `json` block, with nothing after it.
- **Judge setup in the cases**: read `project.settings`, insert `judge_agent_id = ids::AGENT_AGY`, and write it back through `set_project_settings`. `agy` has `default_model` (`seeds/agent_agy.json`), while `claude` has neither a default model nor models, so it would be `judge_unavailable`.
- **Pins**: `CASES` goes to 35. `cases_are_unique_and_thirty_five`, `cases_len_is_thirty_five`.
- **`lib.rs`**: re-exports `fanout::{JudgeFailure, Route}` and `isolate::FanoutSlot`, which join the `isolate::` list.

| Case | Setup, per F-E / F-F | Asserts |
|---|---|---|
| `fan_out_three_with_a_judge_selects_one_winner` (crit. 8) | `HTUI_ANA_2`; `research`: `fan_out 3`, `gate Never`; judge `agy`; judge calls 0 and 1 both answer winner 1 | 3 candidates at 0–2 plus a judge at −1; the winner is `selected = true`, `done`; the losers are `false`, `superseded`; 3 `research` documents; `verdict`'s seq-0 prompt holds only the winner's document; the judge has `prompt_digest`, a `prompt` at seq 0 and a `follow_up` at turn 1, and its `gate_note` is the reason |
| `a_failing_verify_is_eliminated_before_the_judge` (9a) | as above, plus `verify_command`; the verifier scripts pass, fail, pass | the judge prompt contains `judge_candidate:0` and `judge_candidate:2`, not `:1` |
| `one_passing_candidate_wins_without_a_judge` (9b) | fail, pass, fail | no `fanout_index = -1` row; index 1 selected; an A-6 note |
| `judge_orderings_that_disagree_park_for_selection` (10a) | call 0 → 0, call 1 → 2 | every candidate `done`, `selected` NULL; judge `failed`, `gate_note == "judge_disagreement: forward 0, reversed 2"`; run and item `awaiting_approval` |
| `an_unparseable_verdict_parks_and_select_fanout_completes` (10b) | call 0 body is prose | park; then `SelectFanout(winner 2)` → `Selected`; winner `done`/`selected`; judge still `failed` with its note; the run walks on to `verdict` |
| `a_gated_fan_out_parks_for_human_selection` | `gate Always` | park reason `gated`; no judge row |
| `no_judge_means_human_selection` | `gate Never`, no judge | park reason `no judge configured` |
| `a_failed_candidate_does_not_fail_its_siblings` | `script_candidate("research", 1, 1, 0, failing(Refusal))` | candidate 1 `failed` with a note; 0 and 2 `done`; the run continues |
| `a_group_with_no_survivor_retries_then_fails` | `gate Never`, `retry_limit 1`, attempts 1 and 2 `failing` | attempt 1 is retired (cancelled); attempt 2 is a group; run `failed`, `failure == "no_surviving_candidate: research"` |
| `the_review_loop_reruns_a_fanned_out_implement` | FEAT-3; `implement` `fan_out 2`, `Never`; `review` `Never` + `request-changes` then `approve` | slot 1 fully retired, judge included; group at attempt 2; a second judge |
| `fan_out_above_max_fan_out_is_refused_at_start` | `fan_out 5` | `Err(Resolve(FanOutCap { max: 4, … }))`; no run row |
| `max_agents_per_run_is_refused_at_start` | FEAT-3 `implement` `fan_out 3` + judge | `AgentCap { planned: 7, max: 6 }` |

---

## 10. T8: real git (`tests/gix_isolator.rs`)

- `CommittingSink::after_done` writes `agent-{key.fanout_index}.txt`.
- The judge document is scripted through `FakeOrchestrator::script_candidate`.
- Each case opens with `skip_without_git!()`.

**Cases:**
- `a_three_way_worktree_fan_out_is_deterministic_over_many_rounds` (D70): 50 rounds of `join_all` over three slotted `prepare`s on a fresh repository. Every one succeeds, with no retry relied on.
- `a_three_way_worktree_fan_out_merges_only_the_winner`
- `shared_serialized_siblings_run_in_turn_and_the_branch_ends_on_the_winner`
- `a_dirty_shared_checkout_fails_every_sibling_and_parks` (A-1)
- **`a_failed_shared_sibling_releases_the_checkout_for_the_next`** (A-7, added): sibling 0 `refusing_to_start`, siblings 1 and 2 complete.
- `copy_refuses_n_copies_above_the_cap`
- `two_no_commit_implement_attempts_stop_the_loop_in_worktree_mode` (criterion 7, R-8)

---

## 11. Data flow: one 3-way `worktree` group with a judge

`W` = winner, `P` = the primary checkout.

| # | Call | Rows / effect |
|---|---|---|
| 1 | `cursor` → `Create{2,1}`; `admit`: `stage_one`, then `select(…, i)` for i = 0, 1, 2 | three `pending` rows at `(2,1,i)` |
| 2 | `cursor` → `Fan{2,1}`; `drive_group`: `isolator.base(scope)` = `{core: H}`; `assemble_prompt` once | — |
| 3 | `join_all`: for each i, `Pending → Running`; `prepare(…, Some(slot i, base))` under the admin lock (`git worktree add -b htui/<s_i> … H`) | three trees from `H` |
| 4 | per i: `upsert_step_tree`, `record_commits(before = H)`, `set_step_prompt` (same digest), session (key i), `after_done` (the document of candidate i), `verify`, `capture`, `record_commits(after)`, `settle(verify: None)`, `finish_step(real verify)`, `Running → Done` | three `done` rows, `selected` NULL |
| 5 | `cursor` → `Select{2,1}`; `prefilter`/`route` → `Judge([s0, s2])` (s1 verify `fail`) | — |
| 6 | `run_judge`: inputs (task = seq-0 of s0; diffs via `git diff`); forward and reversed assembled; no drop | — |
| 7 | `create_step(-1, "implement:judge")`, walk, `Pending → Running`; `prepare(Local, [])`; `record_prompt(forward)`; call 0, doc; `record_follow_up(reversed)`; call 1, doc; `finish` | judge events: seq 0 `prompt`, turn 1 `follow_up` |
| 8 | verdicts agree on 2 → `finish_step(judge)`; `select_fanout(W = s2, reason)` | W `done`/`true`; s0 and s1 `superseded`/`false`; judge `done` + `gate_note` |
| 9 | `reconcile_done_step(W, siblings [s0, s1])` → `git merge --no-ff htui/s2` into `P` → `record_commits(after = M)` | `P` at `M`; `htui/s0` and `htui/s1` remain |
| 10 | `cursor`: W selected and `done` → next position | — |

Failure branches: at 8, a disagreement leads to `fail_judge` (judge `failed` + note) and then `park_selection`. `SelectFanout` then resumes at step 8 with the human's pick.

---

## 12. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | **F-A**: `output_of` is latest-only, so fan-out settles `MissingOutput` for all but one candidate. | T7(a) rewrite; criterion 8 asserts three `done` candidates. |
| H-2 | **A-7**: a failed shared sibling leaks its guard, and the siblings deadlock inside `join_all`. | Best-effort `capture` on the failure path; T8 case. |
| H-3 | **A-6**: the auto-win and human-pick reasons are silently dropped by `select_fanout`. | Item notes; 9b and 10b assert them. |
| H-4 | **A-1**: D56's order would let sibling 0 run on dirty user work. | Dirty is checked first with a slot; T3 and T8 tests. |
| H-5 | **A-2**: `worktree remove` concurrent with `add` on one repository was never probed. | One admin lock per repository covers both verbs. |
| H-6 | **A-3**: the copy-mode `previous_diff` names a merge commit that is absent from the copy. | `has_commit` repo choice; T3 test. |
| H-7 | **A-4**: `COLUMNS` leaks into `--stat` and the judge digest. | `COLUMNS=80` in `Cli::diff`. |
| H-8 | Judge assembly ordering vs D51's lazy creation vs D53's "does not run the judge". | §9.3 `run_judge` step order: inputs, then drop check, then row. |
| H-9 | D63/D64 inside `resolve` also fire in `Engine::resume` (`engine.rs:681-689`). A lowered `max_fan_out` under a live run makes resume `Err(Resolve)` instead of a topology verdict. | Kept per the plan: the tests are named `…_at_resolution` and there is no production resume before M5. **Recorded for milestone 5.** |
| H-10 | **F-H**: a T2 test with a `SnapshotJudge` literal breaks when T4 lands. | `serde_json` construction only. |
| H-11 | **A-5**: group missing input. | `fail_group_before_a_token`. |
| H-12 | **F-C / F-D**: T7 cannot edit `gate.rs`; `retry_enabled` refuses `done`. | T5 `retire_slot`; `retry_group_enabled`. |
| H-13 | `FakeIsolator::base` must not `tick()`, or M2's `fake:base:N` assertions shift. | Stable synthetic base. |
| H-14 | D59 does not order a `running` candidate against a `pending` one. | §9.3 cursor order: running rests first. |
| H-15 | **F-G**: the judge body's `{{phase}}`. | Spec `phase` = judged name. |
| H-16 | **F-I**: `create_step` refuses an unknown `agent_id`. | `agent_id: None` on the failure path. |
| H-17 | **F-K**: partial group creation. | Select every index before any write. |
| H-18 | HRTB inference on `DriverFor` closures. | Annotate both parameter types. |
| H-19 | `min_budget_for_new_attempt` is unseeded; a stray `0`, negative or string value. | `app_positive` rule: positive `i64` else `0`. |
| H-20 | Lock ordering under `join_all`: `(box, repo)` guard, then `verify` permit (`ShellVerifier`), then admin lock. | No path holds a permit while awaiting a guard, or the admin lock while awaiting anything; D70's lock wraps one child. No cycle. |
| H-21 | FIFO fakes (`script_after`, `script_report`, `script_diff`) rely on `join_all` finishing in index order (D58). | Cases script one value per candidate and assert by `fanout_index`; everything else is keyed by `SessionKey`. |
| H-22 | `isolator.base` failing before any candidate is live propagates, and the run stays `running` at `Fan`. | Acceptable: nothing is live, and the next call re-derives. Stated in `drive_group`'s doc. |
| H-23 | D60 substitution notes are written once per candidate (D71), so three per group. | Accepted per D71; MOD-36 may collapse them. |
| H-24 | Wave A shares a working tree: another task's mid-edit can fail your gate. | §2 shared-tree rule; `git add <own paths>`. |
| H-25 | `SkipReason` has no `Display`. | `select.rs` owns the table (§4.1); T5's literal matches it. |
| H-26 | Two `Recorder` pumps after a `Done` (plan Risk, D52). | Read: `record_follow_up` flushes, then `turn += 1` (`record.rs:623-645`); `Done` only triggers a flush (`:898-907`). Criterion 8 asserts the log. On failure, stop and report; do not widen into `htui-agent`. |
| H-27 | **F-B**: `NoProgressReview` has been unreachable since M2. | Carried as R-9; not fixed here. |

---

## 13. What this milestone does NOT do

- **No re-attempt of a single failed candidate.** That is D48's recorded deviation; the milestone 5 sweep owns it.
- **No weighted agent spread.** That is MOD-36/ANA-21. `FirstCandidate` stays.
- **No phase-level judge columns** (OQ-8), **no `Unblock`**, **no R-7 resume**, **no Runs-tab wiring.**
- **No `WriteStore`/`ReadStore` method, no migration, no `.sqlx`, no `.snap`, no `crates/htui-store/**`, no `crates/htui-agent/**`, no `crates/htui-core/src/prompt/**`.**
- **No fix to `reviews_are_identical`** (R-9).

## 14. Gate checks that are not tests (continuing C-9)

- **C-10** `grep -rn 'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` and `verify.rs` only. `git.rs`'s verbs are `worktree add`, `worktree remove`, `merge --no-ff`, `merge --abort`, `reset --hard` and `diff`.
- **C-11** These are all empty:
  - `git diff --stat main -- crates/htui-store crates/htui-agent 'crates/**/*.snap' crates/htui-core/src/prompt`
  - `git diff main -- crates/htui-core/src/store/traits.rs`
- **C-12** `git diff --stat main -- crates/htui-orch/tests/fixtures/` is empty.
- **C-13** `cargo doc -p htui-orch --no-deps --all-features` exits 0. `cargo doc --workspace --no-deps` shows only the two baseline errors.
- **C-14** Both `CASES` pins read 35. `mem_store.rs:36` and `pg_conformance.rs:19` still read 49.

**Carried out of this milestone**:
- R-3, R-4, R-5, R-6 and R-7 (unchanged);
- **R-8 closed** (D66);
- **R-9 (new)**: `reviews_are_identical` reads latest-only `documents_of_kinds` (`gate.rs:823-837`), so `NoProgressReview` never fires. Fix it through `documents()` heads when a case can pin which stop reason each shipped loop case reaches;
- **H-9** (caps in `resolve` vs resume) goes to milestone 5.
