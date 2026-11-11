# Blueprint: MOD-139 M2 — Planner admission and parts (T1–T5)

> Produced by `code-architect` 2026-11-11 from the confirmed, fact-checked plan
> `.claude/plans/mod-139-m2-planner-and-parts.plan.md` (D1–D17, T1–T5, claims C1–C34), the PRD
> `.claude/prds/mod-139-divided-fan-out.prd.md` (M2 row) and `docs/ANA-33.md` §7.3, §7.4, §7.6
> (admission), §7.7, §7.8, §9. Tree read at `904839d5` (branch `hr/MOD-139`). Format: M1's
> `.claude/plans/mod-139-m1-records.blueprint.md`.
>
> The plan stays the spec. §0 lists where the tree or a probe disagrees with it, or where it leaves
> a choice open; every row is **adopted** (an implementation detail or the only reading that
> works) unless it is under "Flagged". None reopens a maintainer ruling (internal part section, no
> verify on parts, part personas deferred, `fan_out_only` = live siblings incl. parallel branches,
> parts retry before the `fails` edge).
>
> **Probe.** The admission and abandonment SQL of §1.5 ran on a scratch database
> (`mod139_m2_bp_probe` on the sandbox's `localhost:5439`, PostgreSQL 16.15, `0001..0028` applied
> with `sqlx migrate run`, since dropped): the planner `FOR NO KEY UPDATE`, the run `FOR SHARE`, the
> slot test over `UNNEST`, the plan inserted `admitted` with `base_refs` in one statement
> (`created_at = updated_at`), two `role = 'part'` rows under a `role = 'planner'` parent, the
> planner `running -> done`, and the abandon `UPDATE` (the trigger moved `updated_at` past
> `created_at`) all succeeded; no CHECK fired.

## 0. Plan deltas

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | D9's `PlanAdmission { plan, base_refs, parts }` cannot keep D9's own refusal order for `PlanToAdmit::Proposed`: a proposed plan id names neither run nor planner until a read, yet `NotFound` run and `Fenced { step: planner }` come **before** the plan is read. The planner's `finished_at` also needs an instant, and the store has no clock the engine controls (every engine write passes `at`, e.g. `transition_step`). | D9 order; `traits.rs:1731` (`transition_step(.., at)`); `mem.rs:6789-6815` stamps `finished_at` from `at` | **adopted**: `PlanAdmission` carries `run_id`, `planner_step_id` and `at` beside `plan`, `base_refs`, `parts` (§1.2). A `New` plan must name the same run and planner (shape refusal `admission_plan_mismatch`). |
| P-2 | D9's "`base_refs` a non-empty object" would refuse every run whose `repo_scope` is empty: `Isolator::base` of an empty scope is `{}`, and an empty scope is the demo fixture's own shape (every engine case without `primary_repo`). | `isolate.rs:183-189` (`base`: "an empty scope is an empty map"), `isolate.rs:155-157` (`prepare`: "An empty `scope` is not an error — it is the demo fixture's own shape"); `fake.rs:832-838` | **adopted**: `base_refs` must be a JSON **object whose every member is a string** (empty allowed), with no NUL (§1.3). |
| P-3 | D9's Pg lock "`step_fence(StepLock::Update)`" answers `NotFound { entity: "run_step" }` for an unknown planner and checks the **planner's** run's lease (`JOIN run r ON r.id = s.run_id`), so it cannot give D9's order (`NotFound` run, then `Fenced`, then `planner_not_in_run`). | `pg/write.rs:258-300` | **adopted**: two statements in `insert_run_plan`'s shape (`pg/write.rs:7886-7915`): the planner `SELECT … FOR NO KEY UPDATE` alone, then the run `FOR SHARE` (byte-identical to `insert_run_plan`'s run read at `pg/write.rs:7899`, so its `.sqlx` entry is reused). Step → run order and the update lock up front both hold, so `assert_takes_the_step_first` and `assert_locks_the_step_for_update_up_front` (`pg_criteria.rs:7498`, `:7532`) pin it unchanged (§1.5). `abandon_plans` keeps `step_fence(StepLock::Update)`: its order is `step_fence`'s. |
| P-4 | "an `admitted` plan already on this planner row" is unreachable through the API: admission moves the planner `running -> done` in the same transaction and `done -> running` is illegal. | `run.rs:194-219` (`can_move_to`) | **adopted**: the check stays (defence in depth, both stores, sentence `planner_already_admitted`) and is pinned by a `mem.rs` unit test that rewinds the row by hand. The conformance "second admission on one planner row" case asserts the refusal that actually fires: `planner_not_running`. |
| P-5 | Files the plan's Files to Change misses. (a) `htui-core/src/model/mod.rs` and `store/mod.rs` re-export the new types and refusal helpers (M1 P-5a). (b) A new pure module `htui-orch/src/divide.rs` (admission rules, names, positions, widths) and `lib.rs` to register it and re-export `node_of`/`operator_row`, so T3 and T4 do not both grow `engine.rs` for pure code. (c) A new `htui-core/tests/prompt_part.rs` beside `prompt_persona.rs`/`prompt_memory.rs`. (d) `MemFault::AdmitPlan` in `mem.rs` (T1), the only way an engine case can stop the walk with the planner `running` before admission (D14's first crash). | `model/mod.rs:195`, `store/mod.rs:22-52`; `lib.rs:31-86`; `mem.rs:150-166`, `:1491` (`set_fault`) | **adopted**: (a) T1, (b) T3, (c) T2, (d) T1. §5 keeps T1 ∩ T2 empty. |
| P-6 | D1 lists `land_recovered` (`engine.rs:3601`) and `walk_live_step`'s retry landing (`:5037`) as row-to-node sites. Both map a `Landing::Retry { position }`, which `gate::retry_or_fail` builds from `phase.position` (a node's), never from a row; parts never reach either (D4: no gate; D14: no `land_recovered`) and a planner never reaches `walk_live_step` (D2). | `gate.rs:819-824`; D4, D14 | **adopted**: both keep `phase_at`; T3 adds a one-line comment at each ("a `Landing::Retry` position is a node's, by construction"). The sites that map a **row** use `node_at` (`recover_run :3366`, `settle_failed :3738`, `drive_cursor` `Run :4050`); the four operator verbs first refuse a part or stage row with `operator_row` and then keep `phase_at` (§T3.7). |
| P-7 | D6's "the gate's `Settle::Failed` arm (`gate.rs:449-452`) gains a planner branch": no M2 row reaches `gate::apply` with a planner or part phase (parts have no gate, D4; an admission refusal fails the run directly, P-8). | `gate.rs:407-452`; `engine.rs:3590`, `:5026` (only `land_recovered` and `walk_live_step` call `gate::apply`) | **adopted**: the arm's `loop_edge(&phase.name, Fails)` becomes `loop_edge(divided_name(phase), Fails)` (identical for every non-planner phase), so a future planner caller looks on the integrate node. The real planner branch is `settle_failed`'s (T4). |
| P-8 | D2 says an admission validation failure "fails the planner step and so the divided step with a typed `RunFailure`" but names no write path. | `engine.rs:6982-7014` `refuse_prompt` | **adopted**: `refuse_admission` = `refuse_prompt`'s shape: planner `running -> failed`, `block_and_fail` (item `open -> blocked`, the sentence as its note, `finish_run(Failed)`, cleanup), `Rest { Failed, position: planner, failure }`. No `fails` edge and no retry: every D2 refusal is a deterministic configuration fault. A stage-1 refusal at admission (no eligible agent) first moves the planner `running -> failed`, then calls `refuse_no_candidate` unchanged. |
| P-9 | A part's prompt and skills read through the planner node, whose name is `<divided>:plan`: `{{phase}}` would render `implement:plan`, and `phase_skills` looks the live phase up by name and would drop every phase-level skill with a note. | `engine.rs:7345-7370` (`phase_skills` by `row.name == phase.name`); `graph.rs:914-919` (the planner keeps the copied phase under a new name) | **adopted**: a part's spec sets `spec.phase = divided_name(node)`; `phase_skills` looks up `divided_name(phase)`. Graph reads keyed by node (`input_needs`, `reads_verdict`) keep the planner node, which is right for them. `reads_verdict` is `false` for a part's settle. |
| P-10 | D16's seam cannot be reached by an engine case through `MemStore`'s own `GraphSource`: on the demo fixture it answers rung 4 (`NoCandidate`) for every phase; only `FakeOrchestrator::graphs()` (a `FakeGraphSource`) answers candidates. `Orchestrate` exposes no graph source. | `fake.rs:1143-1186` (MemStore source), `:1188-1206` (rung-4 note), `:2570-2590` (`graphs()`) | **adopted**: `engine::enqueue_divided_fake(orch, item)` (`#[cfg(feature = "test-support")]`, beside `claim_fake`, **not** an `Engine` method) runs the seam over `orch.graphs()` and `create_run`s the run; `Orchestrate` gains `enqueue_divided`, implemented in `conformance.rs` by calling it. No `Engine` path is gated (D16 holds). |
| P-11 | D4 lists the deterministic pre-session refusals as `MissingInput`, `PromptRefused`, `ScrubRefused`, `SecretsRefused`, no persona and a prepare refusal. MOD-89's env-file refusal is the same kind (deterministic until a human removes the file; `drive_group` already treats it as a run failure). | `engine.rs:5378-5405` | **adopted**: `EnvFilesRefused` joins the list. A round lets every started part settle, then the **first refusal in part position order** ends the run (§1.10). |
| P-12 | D2's validation failures are named only by code (`empty_plan`, `local_cannot_divide`, `part_persona_unsupported`); D10's owned-path rule and D5's "agent not among the candidates" name none. | plan D2, D5, D7, D10 | **adopted**: seven `RunFailure` variants with exact bytes (§1.7), in the house `code: detail` style (`status.rs:159-210`). |
| P-13 | D10 masks "the brief and acceptance"; the key, the title and each owned path render too. | `mod.rs:968-1080` (every rendered input is masked under its section) | **adopted**: `scrubbed_inputs` masks all five (key, title, brief, each owned path, acceptance), phase specs only (as memory, `mod.rs:1037-1046`). |
| P-14 | D3 leaves open what a `running`/`stopped` part reads as once the planner is `done`. | `schedule.rs:84-99` (rule 1 blocks on a `running` rest) | **adopted**: `NodeState::Live(Cursor::Rest { step: <part>, status })` — "a session this walk does not own", which blocks like any (§T3.4). `status::position_of` maps a part row to its planner's position, and `Engine::resting` reuses `position_of` instead of its own copy (`engine.rs:8182-8195`). |
| P-15 | The plan splits D6 (retries) into T4 but D3 (whose `Failed`/retry readings drive D6) and D4 (`drive_parts`) into T3. | plan T3/T4 | **adopted**: T3 lands D3's whole pure derivation (unit-tested in `walk.rs`) and `drive_parts` with **one round per call** and no retry rows; T4 adds the retry rows and `settle_failed`'s planner branch. No T3 case makes a part fail on its own terms. |
| P-16 | D10's "owned paths validated as `repo:glob` with a repo-relative glob" names no parser. | `model/skill_glob.rs:29-71` (`SkillGlob::parse`, the house `repo:glob`) | **adopted**: §1.8 rule 6. |
| P-17 | D5 does not say what happens when a part names an agent that **is** a candidate but stage 1's walk skipped (quota, box). | `engine.rs:4525-4570` | **adopted**: the selector over the walk's eligible list, as a plain node substitutes (with `note_substitution`); `None` is `refuse_no_candidate` (P-8). |
| P-18 | D4 says a part's `phase_name` carries the revision suffix; ANA §7.4 fixes `node_key = '<divided>/<part_key>'`. | ANA-33 §7.4 | **adopted**: `node_key` never carries `@<revision>`; `phase_name` does from revision 1 (§1.8). |
| P-19 | `cancel_unsettled` (`engine.rs:4150-4157`) names the ender by `phase.name`, which for a planner reads `implement:plan`. | `engine.rs:4151-4161` | **adopted**: the ender is `divided_name(phase)`; stage rows are skipped (`role == Stage → continue`). |
| P-20 | `views.rs` `selectable_slots` (`:454`) needs no edit: it keeps a row only when `phase_at(..).is_ok_and(..)`, so a part (no node at its position) or a stage (negative) is already never selectable. | `views.rs:446-458` | **adopted**: only `verdicts` (`:369`) changes: a part or stage row gets `refused_step` with `operator_row`'s sentence instead of "no phase at position N". |
| P-21 | D14's "under `local`/`shared_serialized` [the planner frontier] is dropped by the landing rule once a part started" needs time to move: the in-place rule is `started > finished` (strict), and the fake's `TestClock` never advances on a read, so a part's `started_at` equals the planner's `finished_at` (both stamped from one frozen instant) and the planner stays a frontier on the fake. | `recover.rs:421-427`; `htui-core/src/clock.rs:72-128` (`TestClock` moves only on `advance`/`set`) | **adopted**: no engine change; a planner has no `run_step_tree` row, so reconciling it lands nothing under any isolation. T4 case 9 pins the property that matters — no part is ever reconciled, and the planner at most with no sibling — rather than the drop, which only a moving clock shows. |

**Flagged for the coordinator (no M2 change beyond what is written):**
- **F-1.** A part whose crash recovery interrupts it (D14 "resettable") spends a part attempt; with
  `retry_limit 0` the divided step then fails `part_exhausted`, where a plain step out of budget
  parks (D94, `engine.rs:3690-3730`). No operator verb can retry a part (D1 refuses one), so a park
  would strand the run. Kept as D6 reads; MOD-140/MOD-142 may want a "retry part" verb.
- **F-2.** M2 divided runs (engine cases only) end `Err(Resolve(NotYetRunnable { Divide }))` at the
  integrate node with the run left `running` and its lease released (`walk_leased`,
  `engine.rs:2717-2735`). Intended by D15; M3 removes it.
- **F-3.** `plan_admission_refusal` refuses an admission with no part
  (`"an admission inserts at least one part step"`) although D9 does not list it: the engine never
  sends one (`empty_plan` first), and a part-less admission would leave a `done` planner whose node
  reads `Done` vacuously.

---

## Main-thread rulings (2026-11-11, binding)

- **F-1 adopted.** An interrupted part spends a part attempt, and with `retry_limit 0` the divided step
  fails `part_exhausted` instead of parking: no operator verb retries a part, so a park would strand the
  run. A "retry part" verb is MOD-142's to propose (Runs pane), not M2's.
- **F-2 adopted.** M2 divided runs exist only in engine cases and end at the integrate node as D15 says.
- **F-3 adopted.** The store refuses an admission with no part, with the sentence above; the engine's
  `empty_plan` comes first.
- **F-4 (T3, adopted).** `plan_digest` hashes a canonical form of the body (object keys sorted by
  byte order at every depth), not `serde_json::to_string(body)` as §1.8 wrote: serde_json's
  `preserve_order` feature is on in every build, so §1.8's formula would hash insertion order, and a
  body read back from `jsonb` would not match the digest stored at admission. Pinned by
  `the_plan_body_and_digest_are_canonical` (`7a9e5e36`).
- **F-5 (T3, adopted).** Two planners never admit at once (`schedule::plan_pass` rule (f),
  `72a9114d`): each reads the run's revision and part positions before its own `admit_plan`, so two
  in flight would collide.
- **F-6 (T4, adopted).** A part failed `interrupted, tree not reset` keeps its divided node parked
  on that part whatever its attempt budget says (`walk::parts_state` rule 2, `4557edb2`), so no
  parts round retries over a tree recovery refused to touch. `recover::NOT_RESET` is public for
  it, and the park reports the planner's position and the part's own name (`implement/api`).
  T4 touched `walk.rs` for this, outside §5's file set.
- **T3 case 15** (two parallel `fan_out_only` branches) depends on the first-poll order of concurrent
  futures. An implementer who sees it flake reports it as a remaining finding; it is never weakened or
  `#[ignore]`d without a coordinator ruling.

## 1. Shared decisions for every task

### 1.1 Names and placement

| Name | Where | Task |
|---|---|---|
| `PlanAdmission`, `PlanToAdmit`, `PlanStatus::can_move_to` | `htui-core/src/model/run.rs`, re-exported in `model/mod.rs` | T1 |
| `WriteStore::admit_plan`, `WriteStore::abandon_plans` (+ `WorkerStore` twins) | `store/traits.rs`, `store/worker.rs` | T1 |
| `plan_admission_refusal`, `admission_plan_mismatch`, `step_not_a_part`, `part_of_another_run`, `part_of_another_planner`, `planner_not_running`, `plan_not_this_planners`, `plan_not_proposed`, `planner_already_admitted` | `store/traits.rs` beside `planner_not_in_run` (`:2881`), re-exported in `store/mod.rs` | T1 |
| `MemFault::AdmitPlan` | `store/mem.rs:150-166` | T1 |
| `PartBlock`, `PART_SEPARATOR`, `Placeholder::Part`, `SectionName::Part`, `render::part` | `prompt/{mod,template,render}.rs` | T2 |
| `CommandQueue::exposed(.., live_siblings, ..)`, `command_queue_exposed(.., live_siblings, ..)` | `model/kind.rs:61`, `:73` | T2 |
| `crate::divide` (pure) | `htui-orch/src/divide.rs` (new) | T3 |
| `Cursor::Parts`, `RunFailure` × 7, `PlanCap` | `htui-orch/src/status.rs` | T3 |
| `NotRunnable::PlannerAgent`, `resolve_divided` (seam) | `htui-orch/src/graph.rs` | T3 |
| `EngineError::RoleRow`, `node_of`, `operator_row` | `htui-orch/src/command.rs` (beside `phase_at`, `:1046`) | T3 |
| `drive_parts`, `run_part`, `admit_planner`, `admit_parts`, `refuse_admission`, `node_at`, `enqueue_divided_fake` | `htui-orch/src/engine.rs` | T3 |
| `StepKind::Part`, `settle_failed`'s planner branch, `retire_parts` | `recover.rs`, `engine.rs`, `gate.rs` | T4 |

Every new helper name has at most three underscores (the P-9 rule of `store/conformance.rs`,
§6 G-4).

### 1.2 Model types (`htui-core/src/model/run.rs`, after `NewRunPlan` at `:614-631`)

```rust
impl PlanStatus {
    /// MOD-139 M2 D9: `run_plan.status`'s law, read by both stores before a status write.
    /// `proposed -> admitted | refused | abandoned`; `admitted -> abandoned`; `refused` and
    /// `abandoned` are terminal. 0028 holds no trigger for it (C26), so the stores are the law.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Proposed => matches!(to, Self::Admitted | Self::Refused | Self::Abandoned),
            Self::Admitted => matches!(to, Self::Abandoned),
            Self::Refused | Self::Abandoned => false,
        }
    }
}

/// MOD-139 M2 D9: which revision [`crate::store::WriteStore::admit_plan`] admits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanToAdmit {
    /// The engine's own revision (default parts), inserted straight at `admitted` with its
    /// `base_refs` in one statement (`chk_run_plan_admitted_base_refs` forbids two).
    New(NewRunPlan),
    /// A revision already written `proposed` (MOD-140's `plan_submit`; M2 reaches it only through
    /// a direct `insert_run_plan` in conformance), moved `proposed -> admitted`.
    Proposed(RunPlanId),
}

/// MOD-139 M2 D9 (blueprint P-1): one admission, decided and written in one transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanAdmission {
    /// The run admitted into; a `New` plan names it too.
    pub run_id: RunId,
    /// The planner step: `running` before, `done` after. A `New` plan names it too, and every
    /// part's `parent_step_id` is it.
    pub planner_step_id: StepId,
    /// The revision admitted.
    pub plan: PlanToAdmit,
    /// The primary HEAD per repo (`Isolator::base`): a JSON object of repo id to commit string,
    /// possibly empty (an empty scope).
    pub base_refs: Value,
    /// Every part row, inserted `pending`; `role = part`, same run, parent = the planner.
    pub parts: Vec<NewRunStep>,
    /// The caller's instant: the planner's `finished_at` (`COALESCE`d).
    pub at: DateTime<Utc>,
}
```

`model/mod.rs:195`'s `pub use run::{…}` gains `PlanAdmission, PlanToAdmit`. Unit test in
`model/mod.rs` beside `plan_status_matches_check_list`: `plan_status_moves_by_its_law` (all 16
pairs: exactly the four `true` cells above).

### 1.3 Store verbs and the refusal sentences (`store/traits.rs`)

```rust
// WriteStore, after insert_run_plan (:1887):
async fn admit_plan(&self, fence: StepFence, admission: PlanAdmission) -> Result<RunPlan>;
async fn abandon_plans(&self, fence: StepFence, planner_step: StepId) -> Result<u32>;
// WorkerStore (store/worker.rs, after insert_run_plan at :361):
fn admit_plan(&self, fence: StepFence, admission: PlanAdmission)
    -> impl Future<Output = Result<RunPlan>> + Send;
fn abandon_plans(&self, fence: StepFence, planner_step: StepId)
    -> impl Future<Output = Result<u32>> + Send;
```

Every sentence is a `pub fn` (`#[must_use]`) in `traits.rs`, called by **both** stores, so the two
refuse in one sentence. Verbatim:

| Helper | Returns |
|---|---|
| `plan_admission_refusal(admission: &PlanAdmission) -> Option<String>` | the shape refusal, judged before any read, first that applies, in this order: (1) `New(plan)`: `run_plan_refusal(plan)`; (2) `New(plan)` with `plan.run_id != admission.run_id \|\| plan.planner_step_id != admission.planner_step_id`: `admission_plan_mismatch(plan.id)`; (3) `parts.is_empty()`: `"an admission inserts at least one part step"`; (4) `base_refs` not an object, or a member not a string: `"an admitted plan's base_refs is an object of repo ids to commit strings"`; (5) `json_has_nul(&base_refs)`: `has_nul("run_plan.base_refs")`; (6) per part in order: `run_step_refusal(part)`, then `role != Part`: `step_not_a_part(part.id, part.role)`, then `run_id != admission.run_id`: `part_of_another_run(part.id, admission.run_id)`, then `parent_step_id != Some(admission.planner_step_id)`: `part_of_another_planner(part.id, admission.planner_step_id)` |
| `admission_plan_mismatch(plan: RunPlanId) -> String` | `format!("plan {plan} names another run or planner step than its admission")` |
| `step_not_a_part(step: StepId, role: StepRole) -> String` | `format!("step {step} is a {role} step; an admission inserts part steps only")` |
| `part_of_another_run(step: StepId, run: RunId) -> String` | `format!("part step {step} is not a step of run {run}")` |
| `part_of_another_planner(step: StepId, planner: StepId) -> String` | `format!("part step {step} is not under planner step {planner}")` |
| `planner_not_running(step: StepId, status: StepStatus) -> String` | `format!("planner step {step} is {status}; an admission needs it running")` |
| `plan_not_this_planners(plan: RunPlanId, planner: StepId) -> String` | `format!("plan {plan} is not a plan of planner step {planner}")` |
| `plan_not_proposed(plan: RunPlanId, status: PlanStatus) -> String` | `format!("plan {plan} is {status}; only a proposed plan is admitted")` |
| `planner_already_admitted(planner: StepId, revision: i32) -> String` | `format!("planner step {planner} already admitted plan revision {revision}")` |

Reused unchanged: `run_plan_refusal`, `run_step_refusal`, `planner_not_in_run`,
`plan_revision_taken`, `already_exists`, `step_slot_is_taken` (`:3257`), `has_nul`,
`json_has_nul` (`traits.rs:2867`, private; `has_nul` `:2983`, pub; the new helper lives in the same file).
`store/mod.rs`'s `pub use traits::{…}` gains the nine names.

### 1.4 `admit_plan`: the decision order and the writes (both stores)

1. `Constraint(plan_admission_refusal(&admission))`, before any read.
2. `NotFound { entity: "run", id: admission.run_id }`.
3. `Fenced { step: admission.planner_step_id }` when the run's lease owner `!= fence.owner()`.
4. `Constraint(planner_not_in_run(planner, run))`: planner unknown, or a step of another run.
5. `Constraint(planner_not_running(planner, status))`: the planner is not `running`.
6. `Proposed(id)` only: `Constraint(plan_not_this_planners(id, planner))` when the plan is unknown,
   of another run, or of another planner; then `Constraint(plan_not_proposed(id, status))` when
   `!status.can_move_to(PlanStatus::Admitted)`.
7. `Constraint(planner_already_admitted(planner, revision))`: some plan of this planner is
   `admitted` (the lowest such revision is named). Unreachable via the API (P-4).
8. `Constraint(step_slot_is_taken(run, position, attempt, fanout_index))` for the **first part in
   admission order** whose `(run_id, position, attempt, fanout_index)` already exists.
9. `New(plan)` only: `Constraint(already_exists("run_plan", id))` / `Constraint(plan_revision_taken(run,
   revision))`, not ordered between themselves (M1 §1). A part id already taken is
   `Constraint(_)` (MemStore `already_exists("run_step", id)`, Pg the raw `23505`); no case.

Writes, in this order, all-or-nothing: the plan (`New`: inserted `status = admitted`, `base_refs`;
`Proposed`: `status = admitted`, `base_refs` set, `updated_at` moves), every part row `pending`
(`started_at`/`finished_at` NULL), the planner `running -> done` with
`finished_at = COALESCE(finished_at, at)`. Answer: the admitted `RunPlan` row. Timestamps:
`New` answers `created_at == updated_at` (MemStore one `now` truncated with `TIMESTAMPTZ_DIGITS`,
Pg both `DEFAULT now()`); `Proposed` keeps `created_at` and moves `updated_at` (Pg:
`trg_run_plan_updated_at`, `clock_timestamp()`; MemStore `now`, which may **equal** `created_at`
under a fixed clock). Conformance compares `updated_at` only as `>= created_at`, never exactly
across stores.

**MemStore** (`State::admit_plan(&mut self, fence, admission, now) -> Result<RunPlan>`, beside
`State::insert_run_plan` at `mem.rs:6937`): decides 1–9 on reads only, **then** mutates
(`State` has no rollback). Fence: `self.lease_owners.get(&run).copied() != fence.owner()`, as
`insert_run_plan` reads it (`:6956`). Wrapper (`MemStore`'s `WriteStore`, beside `:10510`):
`#[cfg(feature = "test-support")] self.check_fault(MemFault::AdmitPlan)?;` first, then
`let now = self.now().trunc_subsecs(TIMESTAMPTZ_DIGITS); self.write(|s| s.admit_plan(fence, admission, now))`.

### 1.5 `admit_plan` on Postgres: the statement sequence (one transaction; probed)

```
tx = pool.begin()
S1  SELECT run_id AS "run_id: RunId", status AS "status: StepStatus"
      FROM run_step WHERE id = $1 FOR NO KEY UPDATE                      -- planner; fetch_optional
S2  SELECT lease_owner AS "lease_owner?" FROM run WHERE id = $1 FOR SHARE  -- byte-identical to
                                                                         -- insert_run_plan's (:7899)
    -> NotFound run / Fenced / planner_not_in_run (S1 None or run_id != run) / planner_not_running
S3  (Proposed) SELECT run_id AS "run_id: RunId", planner_step_id AS "planner_step_id: StepId",
           status AS "status: PlanStatus" FROM run_plan WHERE id = $1 FOR UPDATE
S4  SELECT MIN(revision) AS "revision" FROM run_plan
     WHERE planner_step_id = $1 AND status = 'admitted'                   -- Option<i32>
S5  per part: SELECT EXISTS (SELECT 1 FROM run_step WHERE run_id = $1 AND position = $2
                AND attempt = $3 AND fanout_index = $4) AS "taken!"
W1  New:      INSERT INTO run_plan (id, run_id, revision, planner_step_id, scope, status,
                                    base_position, base_refs, body, digest)
              VALUES ($1, $2, $3, $4, $5, 'admitted', $6, $7, $8, $9)
              RETURNING <insert_run_plan's twelve columns, :7926-7937>
              .map_err(|err| run_plan_insert_refused(err, &plan))
    Proposed: UPDATE run_plan SET status = 'admitted', base_refs = $2 WHERE id = $1
              RETURNING <the same twelve columns>
W2  per part: create_step's query_as! text **verbatim** (pg/write.rs:7166-7196), on &mut *tx,
              .map_err(map_sqlx) — the .sqlx entry is create_step's own
W3  UPDATE run_step SET status = 'done', finished_at = COALESCE(finished_at, $2)
     WHERE id = $1 AND status = 'running'                                -- rows_affected == 1
tx.commit()
```

Why it holds both lock pins: S1 is the first statement and locks only the step, with the update
mode up front; S2 locks the run after. A holder's `FOR UPDATE` on the step makes S1 wait holding
nothing on the run (`assert_takes_the_step_first`); a holder's `FOR SHARE` makes S1 queue holding
nothing on the step (`assert_locks_the_step_for_update_up_front`). W2's foreign key takes
`FOR KEY SHARE` on the planner in the same transaction, which conflicts with nothing it holds.

`abandon_plans` on Postgres: `step_fence(&mut tx, planner, fence, StepLock::Update)` (NotFound
`run_step`, then `Fenced`), then `UPDATE run_plan SET status = 'abandoned' WHERE planner_step_id =
$1 AND status IN ('proposed', 'admitted')`, `rows_affected` as `u32`, commit.

### 1.6 `abandon_plans` contract (both stores)

`NotFound { entity: "run_step" }` for an unknown planner, then `Fenced { step }`; then every plan
of that planner whose `status.can_move_to(Abandoned)` (i.e. `proposed`, `admitted`) moves to
`abandoned` (`updated_at` moves); `Ok(count)`. A second call answers `Ok(0)`. MemStore:
`State::abandon_plans(&mut self, fence, planner, now) -> Result<u32>` with
`Self::fence_holds(&self.lease_owners, row, fence)?` (`mem.rs:2392`), as `pass_step` (`:6877-6900`).

### 1.7 Orchestrator types (exact)

**`Cursor::Parts`** (`status.rs:217`, after `Select`):
```rust
/// MOD-139 M2 D4: a divided node whose planner row is `done` and whose parts (of that planner
/// attempt) are not all `done`, none is live and none is out of attempts: create the retry rows
/// owed (T4) and run every `pending` part.
Parts {
    /// The planner node's `position`.
    position: i32,
},
```

**`RunFailure`** (`status.rs:42-145`), new variants after `LoopExhausted`, and **`PlanCap`**:
```rust
EmptyPlan { node: String },
LocalCannotDivide { node: String, parts: u32 },
PlanOverCaps { node: String, cap: PlanCap, planned: u32, max: u32 },
PartPersonaUnsupported { node: String, part: String },
PartAgentNotCandidate { node: String, part: String, agent: AgentId },
PartPathRefused { node: String, part: String, path: String, reason: String },
PartExhausted { node: String, part: String, attempts: i32 },

/// MOD-139 M2 D8: which cap an admission broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanCap { MaxParts, MaxFanOut, MaxAgentsPerRun }
impl PlanCap { pub const fn as_str(self) -> &'static str { /* "max_parts" | "max_fan_out" | "max_agents_per_run" */ } }
```
`node` is always the **divided** name (`divided_name`, `implement`), `part` the part key (`api`).
Display bytes (in the `Display` match, `status.rs:159-210`):

| Variant | Exact `Display` |
|---|---|
| `EmptyPlan` | `empty_plan: divided node `{node}` has no parts` |
| `LocalCannotDivide` | `local_cannot_divide: divided node `{node}` plans {parts} parts under `local` isolation, which runs one` |
| `PlanOverCaps` (`MaxParts`/`MaxFanOut`) | `plan_over_caps: divided node `{node}` plans {planned} parts; {cap} is {max}` |
| `PlanOverCaps` (`MaxAgentsPerRun`) | `plan_over_caps: divided node `{node}` brings the run to {planned} planned sessions; {cap} is {max}` |
| `PartPersonaUnsupported` | `part_persona_unsupported: part `{part}` of `{node}` names a persona; per-part personas run from MOD-140 on` |
| `PartAgentNotCandidate` | `part_agent_not_candidate: part `{part}` of `{node}` names agent {agent}, which is not one of the node's candidates` |
| `PartPathRefused` | `part_path_refused: part `{part}` of `{node}`: owned path `{path}` {reason}` |
| `PartExhausted` | `part_exhausted: part `{part}` of `{node}` failed after {attempts} attempts` |

(`{cap}` is `cap.as_str()`. Backticks above are literal bytes of the sentence.)

**`NotRunnable::PlannerAgent`** (`graph.rs:312-326`):
```rust
/// MOD-139 M2 D17 (PRD Q1): a divided node with a planner agent configured; MOD-140 runs it.
#[error("a divided node's planner agent runs from MOD-140 on")]
PlannerAgent,
```

**`EngineError::RoleRow`** (`command.rs:344ff`, after `PromoteCandidate`):
```rust
/// MOD-139 M2 D1: an operator verb named a part or a stage row, which no node maps to.
#[error("{}", role_row(.step, .role))]
RoleRow { step: StepId, role: StepRole },
fn role_row(step: &StepId, role: &StepRole) -> String  // private, beside chat_is_live
// Part  -> format!("step {step} is a part; a part is driven by its divided node")
// other -> format!("step {step} is a `{role}` row; htui drives it, not an operator verb")
```

**`command.rs` (beside `phase_at`, `:1046`)**:
```rust
/// MOD-139 M2 D1: the node a row belongs to. `step`/`planner`/`integrate` (and every other
/// non-part, non-stage role) by `position`; a `part` by its `parent_step_id` row's position (its
/// planner node); a `stage` row by none. `None` too when a part's parent is not in `steps`.
#[must_use]
pub fn node_of<'s>(snapshot: &'s GraphSnapshot, steps: &[RunStep], step: &RunStep)
    -> Option<&'s SnapshotPhase>;

/// MOD-139 M2 D1: `Err(EngineError::RoleRow)` for a `part` or `stage` row, `Ok(())` otherwise.
/// # Errors
pub fn operator_row(step: &RunStep) -> Result<(), EngineError>;
```
`lib.rs:55-59` re-exports both. Engine wrapper (`engine.rs`, beside `phase_at` at `:8237`):
`fn node_at(run: RunId, snapshot: &GraphSnapshot, steps: &[RunStep], step: &RunStep) ->
Result<SnapshotPhase, EngineError>` → `EngineError::Snapshot { run, reason: format!("no node for
step {} (a {} row at position {})", step.id, step.role, step.position) }` on `None`.

**`StepKind::Part`** (T4, `recover.rs:186-204`): a fourth variant; `StepKind::of` answers `Part`
first when `matches!(step.role, StepRole::Part)`, before the judge test.

### 1.8 `htui-orch/src/divide.rs` (new, pure; T3) — the rules every task reads

Module doc: `//! MOD-139 M2 (ANA-33 §7.4, §7.8): divided fan-out's pure rules — names, positions, the
admission refusal, part widths and live siblings. No store, no clock.` Registered in `lib.rs` as
`pub mod divide;` (between `command` and `engine`, alphabetical).

```rust
/// The divided node's own name: a planner node's `name` less `PLAN_SUFFIX`, any other node's name.
#[must_use] pub fn divided_name(phase: &SnapshotPhase) -> &str;
/// The row role a node's `Cursor::Create` writes: Step→Step, Planner→Planner, Integrate→Integrate.
#[must_use] pub const fn row_role(node: NodeRole) -> StepRole;
/// `<divided>/<key>` at revision 0, `<divided>/<key>@<revision>` from revision 1 (D4).
#[must_use] pub fn part_phase_name(divided: &str, key: &str, revision: i32) -> String;
/// `<divided>/<key>`, every revision (ANA-33 §7.4; P-18).
#[must_use] pub fn part_node_key(divided: &str, key: &str) -> String;
/// The part key of a `node_key`: the text after its last `/`.
#[must_use] pub fn part_key(node_key: &str) -> Option<&str>;
/// D2 step 5: 1 + max(the snapshot's highest position, the highest non-stage row position,
/// every plan's `base_position + parts - 1`); `parts` is `body["parts"]`'s length.
#[must_use] pub fn next_base_position(snapshot: &GraphSnapshot, steps: &[RunStep], plans: &[RunPlan]) -> i32;
/// `{"scope": "parts", "parts": [<each SnapshotPart, serde>]}`.
#[must_use] pub fn plan_body(parts: &[SnapshotPart]) -> Value;
/// `"sha256:" + sha256_hex(serde_json::to_string(body))` (htui_core::prompt::digest::sha256_hex).
#[must_use] pub fn plan_digest(body: &Value) -> String;
/// D8 (ANA-33 §7.8): planned sessions. Step: `fan_out` + 1 for a judged `fan_out > 1`;
/// Planner: 1 if `divide.planner.agent_id` is set, else 0, plus its parts — `admitting.1` when
/// `admitting == Some((planner position, n))`, else `divide.default_parts.len()`;
/// Integrate: 1. Retries and rounds never count.
#[must_use] pub fn planned_sessions(phases: &[SnapshotPhase], admitting: Option<(i32, u32)>) -> u32;
/// D2 step 2: the first refusal of admitting `parts` on planner node `node`, or `None` (order below).
#[must_use] pub fn admission_refusal(snapshot: &GraphSnapshot, node: &SnapshotPhase, parts: &[SnapshotPart]) -> Option<RunFailure>;
/// The latest attempt of each part position under `planner` (rows whose `parent_step_id` is it),
/// in position order.
#[must_use] pub fn latest_parts(steps: &[RunStep], planner: StepId) -> Vec<&RunStep>;
/// Whether `part` (a latest attempt) is out of attempts: `failed | cancelled | superseded` and
/// `!may_attempt(attempt + 1, retry_limit)`.
#[must_use] pub fn exhausted(part: &RunStep, retry_limit: i32) -> bool;
/// D4: the parts a round may run at once: 1 under `shared_serialized` or `local`, else
/// `min(live, max_parallel_sessions.max(1)).max(1)`, `live` = the latest-attempt parts under the
/// node's latest planner row that are `pending`, or `failed | cancelled | superseded` with an
/// attempt left.
#[must_use] pub fn parts_width(snapshot: &GraphSnapshot, steps: &[RunStep], node: &SnapshotPhase) -> u32;
/// D13: another row of the run than `step`, not a stage row (`role == Stage`) and not a judge
/// (`fanout_index == -1`), at `pending` or `running`.
#[must_use] pub fn live_siblings(steps: &[RunStep], step: StepId) -> bool;
```

**The admission refusal order** (`admission_refusal`, first wins; `node_name = divided_name(node)`):
1. `parts.is_empty()` → `EmptyPlan`.
2. `node.isolation == Isolation::Local && parts.len() > 1` → `LocalCannotDivide { parts }`.
3. `divide.max_parts == Some(m) && parts > m` → `PlanOverCaps { cap: MaxParts, planned: parts, max: m }`.
4. `parts > settings.max_fan_out` → `PlanOverCaps { cap: MaxFanOut, .. }`.
5. `planned_sessions(&snapshot.phases, Some((node.position, parts))) > settings.max_agents_per_run`
   → `PlanOverCaps { cap: MaxAgentsPerRun, planned, max }`.
6. Per part, in `SnapshotPart.position` order: `persona_id.is_some()` → `PartPersonaUnsupported`;
   then each `owned_paths` entry, trimmed: `SkillGlob::parse` refuses → `PartPathRefused { reason:
   format!("is not a glob: {err}") }`; the glob proper starts with `/` or `\` →
   `reason: "is an absolute path".into()`; a `/`-separated segment equals `..` →
   `reason: "leaves its repository through `..`".into()`; then `agent_id == Some(a)` with no
   `node.candidates[i].agent_id == a` → `PartAgentNotCandidate { agent: a }`.

**Formats and shapes (pinned by `divide.rs` unit tests).**
- Planner row: `role = planner`, `node_key = Some(node.name)` (`implement:plan`), `phase_name =
  node.name`, created by `admit` at the planner node's position (D2).
- Part row: `position = base_position + i` (i = index in `default_parts` position order),
  `attempt = 1`, `fanout_index = 0`, `role = part`, `parent_step_id = planner row`, `node_key =
  part_node_key`, `phase_name = part_phase_name` (`implement/api`, `implement/api@1`), `agent_id`
  and `model` from §1.10's pick. A retry row (T4) copies all of these with `attempt + 1`.
- `revision` = the run's highest `run_plans` revision + 1, else 0.
- `base_refs` = `serde_json::to_value(&BTreeMap<RepoId, String>)` of `Isolator::base(&run.repo_scope)`:
  `{"<repo uuid>": "<commit>"}`, `{}` for an empty scope; `drive_parts` decodes it back with
  `serde_json::from_value::<BTreeMap<RepoId, String>>`.
- `scope` = `node.plan_scope.unwrap_or(PlanScope::Parts)`.
- FEAT-3 divided at `implement`: `prd 0, plan 1, implement:plan 2, implement 3, review 4`;
  revision 0's parts at 5, 6; after a loop, revision 1's at 7, 8.

### 1.9 The part prompt section (T2; ANA-5 shapes)

`prompt/mod.rs`:
```rust
/// MOD-139 M2 D10: what the internal `part` section renders — one divided part's brief.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartBlock { pub key: String, pub title: String, pub brief: String,
                       pub owned_paths: Vec<String>, pub acceptance: Option<String> }
impl From<&SnapshotPart> for PartBlock { /* field by field */ }
/// MOD-139 M2 D10: the blank line after the part section, counted in the frame (persona's rule,
/// B-17): the section is protected and never dropped.
pub const PART_SEPARATOR: &str = "\n\n";
// PromptSpec, after `memory` (:125):
/// MOD-139 M2 D10: the divided part this session works, rendered as the internal `part`
/// section after persona and memory and before the body. `None` everywhere but a part's
/// session; rendered for a Phase spec only.
pub part: Option<PartBlock>,
```
- `template.rs`: `Placeholder::Part` declared **after `Memory`, before `Output`**; `token()` →
  `"part"`; **not** in `ALL`; `allowed_in(Phase)`'s excluded list gains `Self::Part`
  (`template.rs:208-221`); `is_section()` is `true` with no edit. `ALL`'s doc and the variant doc
  say "Internal, like `Placeholder::Persona`".
- `mod.rs`: `SectionName::Part` after `Memory`, `render()` → `"part"`; `is_protected`'s `true` arm
  gains `Self::Part` (`:369-378`, `_ => false` would otherwise trim it).
- `render.rs`, after `memory` (`:692-713`):
  ```rust
  /// MOD-139 M2 D10: `<section name="part" key="…" title="…">`, then the brief, then
  /// `Owned paths:` with one `- <path>` line each, then `Acceptance:` and its text; the three
  /// blocks that are present are joined by one blank line. No run, step or fan-out index.
  #[must_use] pub fn part(block: &PartBlock) -> Rendered
  // attrs: vec![("key", attr(&block.key)), ("title", title_attr(&block.title))]
  // blocks: content_of(&brief) if non-empty;
  //         format!("Owned paths:\n{}", paths.map(|p| format!("- {p}")).join("\n")) if any;
  //         format!("Acceptance:\n{}", content_of(acceptance)) if Some and non-empty;
  // content: blocks.join("\n\n")
  ```
  Pinned bytes for `fixtures::part_api()` (`key "api"`, `title "The HTTP API"`, `brief "Add the
  \`/runs\` endpoint.\nKeep the handlers thin."`, `owned_paths ["htui:crates/api/**",
  "docs/api.md"]`, `acceptance Some("\`cargo test -p api\` passes.")`):
  ```
  <section name="part" key="api" title="The HTTP API">
  Add the `/runs` endpoint.
  Keep the handlers thin.

  Owned paths:
  - htui:crates/api/**
  - docs/api.md

  Acceptance:
  `cargo test -p api` passes.
  </section>
  ```
- `assemble` (`mod.rs:556-568`): after the memory push,
  `for section in render_sections(Placeholder::Part, ..) { rendered.push((Placeholder::Part, section, 1)); }`;
  the frame (`:610-616`) gains `if part_of(spec).is_some() { frame.push_str(PART_SEPARATOR); }`.
- `substitute` (`:744-754`): after the memory block, `if let Some(part) =
  blocks.get(&Placeholder::Part) { text.push_str(&part.join("\n\n")); text.push_str(PART_SEPARATOR); }`.
- `render_sections` (`:829`, its arms at `:894-899`): `Placeholder::Part => part_of(spec).map(render::part).into_iter().collect()`.
- `fn part_of(spec: &PromptSpec) -> Option<&PartBlock>`: `spec.part.as_ref().filter(|_| matches!(spec.role, TemplateRole::Phase))` (memory_of's shape, `:783-788`).
- `scrubbed_inputs` (`:968`): under `if matches!(spec.role, TemplateRole::Phase) && let Some(part) =
  &mut spec.part`, mask `key`, `title`, `brief`, every `owned_paths` entry and `acceptance` under
  `SectionName::Part.render()` (P-13).
- `trim.rs`: the two exhaustive matches gain `SectionName::Part` — `Source::Fixed` (`:588-594`) and
  the `None` list of `trim_diff` (`:939`, list `:953-968`). Nothing else: a protected section is never trimmed,
  so an oversized brief is `AssembleError::BudgetTooSmall` at `refusals()` (`:641-657`).
- Record order: `sections[0] = template`, then `persona` and `memory` when present, then `part`,
  then the body's spans, `output` last (the `rendered` push order). Pinned in §T2.4.
- Rule-8 docs narrowed to compete candidates: `PromptSpec`'s doc (`mod.rs:72-79`), `JudgeCandidate.
  fanout_index` (`mod.rs:247`), `render::judge_candidate` (`render.rs:746`): "two **compete**
  candidates assemble the same bytes; a divided node's parts differ in their `part` section only
  (MOD-139 D10)".

### 1.10 The engine's new paths (T3, T4)

```rust
/// One part's inputs to [`Engine::run_part`] (beside `CandidateStage`, `engine.rs:8401`).
#[derive(Debug, Clone, Copy)]
struct PartStage<'s> {
    run: &'s Run,
    snapshot: &'s GraphSnapshot,
    /// The planner node (`node_of` the part).
    node: &'s SnapshotPhase,
    /// The part row, still `pending`.
    step: &'s RunStep,
    /// Its default part, by key (`part_key(step.node_key)`).
    part: &'s SnapshotPart,
    /// `None` under `local` (D7: `GixIsolator` refuses any slot there, `real.rs:666-668`);
    /// else `FanoutSlot { index: position - plan.base_position, width: <parts in the plan>, base: &base_refs }`.
    slot: Option<FanoutSlot<'s>>,
}

/// What one part's round answered (D4).
#[derive(Debug)]
enum PartOutcome {
    /// `done`, or `failed` on its own terms (retried by T4's next round).
    Settled,
    /// A deterministic pre-session refusal: the part row is already `failed` with `text` as its
    /// note; the round ends the run (P-11). `block` blocks the item (PromptRefused, no persona).
    Refused { text: String, failure: Option<RunFailure>, block: bool },
}

async fn admit_planner(&self, run: &Run, snapshot: &GraphSnapshot, node: &SnapshotPhase, planner: RunStep) -> Result<Option<Rest>, EngineError>;
async fn admit_parts(&self, run: &Run, snapshot: &GraphSnapshot, node: &SnapshotPhase, planner: &RunStep) -> Result<Option<Rest>, EngineError>;
async fn refuse_admission(&self, run: &Run, planner: &RunStep, failure: RunFailure) -> Result<Rest, EngineError>;
async fn drive_parts(&self, run: &Run, snapshot: &GraphSnapshot, node: &SnapshotPhase) -> Result<Option<Rest>, EngineError>;
async fn run_part(&self, stage: PartStage<'_>) -> Result<PartOutcome, EngineError>;
async fn part_live(&self, stage: &PartStage<'_>, trees: &mut Option<Vec<RunStepTree>>, captured: &mut bool) -> Result<PartOutcome, EngineError>;
async fn fail_part(&self, run: &Run, node: &SnapshotPhase, step: &RunStep, reason: &str) -> Result<bool, EngineError>;
// T4:
async fn settle_exhausted_part(&self, run: &Run, snapshot: &GraphSnapshot, node: &SnapshotPhase, planner: &RunStep) -> Result<Option<Rest>, EngineError>;
async fn reset_part(&self, run: &Run, node: &SnapshotPhase, step: &RunStep, trees: &[RunStepTree]) -> Result<Option<Next>, EngineError>;
#[cfg(feature = "test-support")]
pub async fn enqueue_divided_fake(orch: &crate::fake::FakeOrchestrator, item: ItemId) -> Result<RunId, EngineError>;
```

**`admit_planner`** (D2): `move_step(run, planner, Pending, Running, now)` (StaleWrite on `false`),
then `Box::pin(self.admit_parts(..))`. **`admit_parts`** (also D14's recovery entry, on a `running`
planner), in order: `divide` = `node.divide` (else `EngineError::Snapshot { reason: "planner node
`<name>` carries no divide block" }`); `steps`, `plans` read; `revision`; `admission_refusal` →
`refuse_admission`; `walk = stage_one(run, snapshot, node)`; per part i (P-17): a named agent in
`walk.eligible` → `(agent, part.model.clone().unwrap_or(candidate.model))`, else
`selector.select(node, &walk.eligible, i)` with `note_substitution(run, node, 1, i, &chosen, &walk)`,
`None` → planner `running -> failed` then `refuse_no_candidate(run, node, &walk)`; **every pick
before any write** (`admit_indices`' rule); `base = isolator.base(&run.repo_scope)`;
`base_position`; the part rows; `store.admit_plan(StepFence::Lease(owner), PlanAdmission { ..,
plan: PlanToAdmit::New(NewRunPlan { id: RunPlanId::new(), run_id, revision, planner_step_id,
scope, base_position, body, digest }), at: self.now() })`; `Ok(None)`. A store or isolator error
propagates with the planner `running` (that is D14's "admission never ran").
**`refuse_admission`**: P-8.

**`drive_parts`** (D4; one round per call, P-15): the latest planner row at `node.position`
(`role == Planner`, `done`, else `Ok(None)`); [T4: the retry rows, §T4.2]; `pending` = the
latest-attempt parts at `pending`, position order; none → `Ok(None)`; the planner's `admitted` plan
(highest revision whose `planner_step_id` is the row; else `EngineError::Snapshot { reason:
"planner step <id> is done with no admitted plan" }`) and its decoded `base_refs`;
`Box::pin(self.secrets_ready(run))` once (`drive_group`'s `:5296-5307`) — a refusal moves each
pending part `pending -> running` and then through `fail_part` with the sentence, then
`finish_run(Failed)` + `cleanup_walk` (`fail_group_before_a_token`'s order, `:5493-5532`, with
`fail_part` in place of `fail_candidate`, whose note names a fan-out candidate), and answers
`Rest { Failed, Some(node.position), Some(SecretsRefused { cause }) }`; `width = parts_width(..)`;
`futures::stream::iter(pending.iter().map(|step| Box::pin(self.run_part(PartStage { .. }))))
.buffer_unordered(width as usize).collect::<Vec<_>>().await`; errors fold as `drive_group`'s
(`group_error` at `:8621`, called `:5382`: a cancel or a fenced write is raised once every part settled); the
first `Refused` in part position order: `block` → `block_and_fail(run, &failure, Some(part), now)`,
else `finish_run(Failed, Some(&text))` + `cleanup_walk`; `Ok(Some(Rest { Failed, Some(node.position),
failure }))`. Under `shared_serialized` (no refusal): `isolator.reset(last.id, &last_trees)` on the
round's last captured part (its tree's `base_ref` is `base_refs`), a non-empty `refused` is
`EngineError::Isolate(IsolateError::Refused(<reasons joined by ", ">))`. `Ok(None)`.

**`run_part`** (`run_candidate`'s shape, `:5570-5623`): `move_step(pending -> running)` (StaleWrite)
+ `step_started`; `part_live`; on `Err` **and** on `Ok(Refused { .. })`: `release_trees` if
`trees` is set and not captured (plan D78's note: a `shared_serialized` guard is released only at
`capture`, and the round's next part would wait on it for ever); on `Err`, a cancel or fenced error
is raised, otherwise `fail_part(run, node, step, &failure_text(&part_name, &err))` and
`Ok(Settled)`. (`part_name` is the part row's `phase_name`.) **`part_live`** (`candidate_live`'s shape, `:5647-5834`, minus verify, gate and
landing): `prepare(run.id, step.id, &run.repo_scope, node.isolation, stage.slot)` — an
`IsolateError::Refused` is `Refused { text: failure_text(&part_name, &err), failure: None, block:
false }` after `fail_part`; trees and `before` commits recorded; `live = divide::live_siblings(
&run_steps, step.id)` read now; stage 3 `assemble_prompt(run, snapshot, step, node, item,
Some(stage.part), live)` — `MissingInput` → `RunFailure::MissingInput`, `Refused` →
`PromptRefused { phase: <part phase_name>, reason }` (`block: true`), `NoPersona` → same
(`block: true`); `set_step_prompt` (a scrub refusal → `RunFailure::scrub_refused(<part phase_name>,
..)`, `block: false`); `drive_once(run, step, node, persona, &SessionKey::of(step), .., live, ..)`
(an `EngineError::EnvFiles` → `EnvFilesRefused`, `block: false`); `after_done`; `capture`,
`record_commits`; `output_of(item, node, step.id)`; `gate::settle(&SettleInput { verify_outcome:
None, reads_verdict: false, deadline_seconds: node.deadline_seconds, .. })`; `finish_step`
(`verify_outcome: None, verify_exit_code: None`); `Ok` → `running -> done` (+ its note);
`Failed`/`Rejected` → `fail_part`. **`fail_part`**: `transition_step(running -> failed)`; on `true`
the note `format!("part `{key}` of `{divided}` attempt {attempt}: {reason}")` (`via_step_id`
the part).

**Threading `live_siblings` (D13).** `command_run_exposed(&self, phase, required_tags,
live_siblings: bool)`; `phase_spec(.., strict, live_siblings: bool)`; `assemble_prompt(&self, run,
snapshot, step, phase, item, part: Option<&SnapshotPart>, live_siblings: bool)` (sets `spec.part` and
`spec.phase = divided_name(phase)` when `part` is `Some`, P-9); `session(.., live_siblings)`;
`drive_once(.., live_siblings: bool, ..)` (its `exposed` computation, `:7804-7809`). Callers:
`walk_live_step` computes `live_siblings(&run_steps, step.id)` once before stage 3 and passes it to
both; `drive_group` computes it once for `first` and carries it in `CandidateStage` (new field
`live_siblings: bool`); the judge's `drive_once` (`:6661`) and the handoff's `phase_spec` (`:2231`)
pass `false`.

### 1.11 `CommandQueue::exposed` (T2)

```rust
// model/kind.rs:61
pub fn exposed(self, fan_out: i32, live_siblings: bool, item_tags: &[String]) -> bool
// FanOutOnly => fan_out > 1 || live_siblings || item_tags.iter().any(|tag| tag == HEAVY_BUILD_TAG)
// model/kind.rs:73
pub fn command_queue_exposed(mode: CommandQueue, fan_out: i32, live_siblings: bool, item_tags: &[String]) -> bool
```
Doc: "`fan_out_only` → when the phase fans out to more than one agent, **when the session has a live
sibling in the run (MOD-139 D13: another `pending` or `running` row, not a stage or a judge —
divided parts and parallel branches)**, or the item carries `HEAVY_BUILD_TAG`".

### 1.12 The stack rule (ANA-33 §9; memory: htui-orch-test-stack-headroom)

Box at every new await of a node-sized future: `drive_cursor`'s `Parts` arm
(`Box::pin(self.drive_parts(..))`), the `Run(planner)` arm (`Box::pin(self.admit_planner(..))`),
each `run_part` in the stream (`Box::pin`), `admit_parts` inside `admit_planner` and in
`recover_run`, `settle_exhausted_part` in `settle_failed`, `loop_over` (already boxed),
`gate::retire_parts`' `abandon_plans` (Pg's future is small; no box needed), and every new
`case()` frame arm in both conformance files. Gate runs `--no-fail-fast` and greps `SIGABRT`.

---

## T1 — Store: admission, abandonment, input exclusion (parallel with T2)

Test-first: §T1.4's cases and §T1.7's pins are written against stubs (commit 2), go red, then
MemStore (commit 4) and PgStore (commit 5) turn them green.

### T1.1 Model (`model/run.rs`, `model/mod.rs`)

§1.2 verbatim. `use chrono::{DateTime, Utc}` is already in `run.rs`. Test
`plan_status_moves_by_its_law` in `model/mod.rs`'s tests (beside `plan_status_matches_check_list`).

### T1.2 Trait, helpers, forwarders

- `traits.rs`: the two `WriteStore` verbs after `insert_run_plan` (`:1887`), each with an
  `# Errors` section that lists §1.4 / §1.6's order **as written there**. The nine helpers of §1.3
  after `plan_revision_taken` (`:2887-2890`), with unit tests in `traits.rs`'s tests:
  `plan_admission_refusal_judges_the_shape_in_order` (one assertion per rule of §1.3, in order:
  a `New` with a negative revision answers `run_plan_refusal`'s sentence even when its run also
  mismatches; a mismatched run; no parts; `base_refs` `[]`, `{"r": 1}`, `{"r": "a\u0000"}`; a
  stage part answers `run_step_refusal`'s sentence first; a `Step` role; another run; another
  parent) and `the_admission_sentences_are_pinned` (each helper's exact bytes once).
- The `StepFence` doc's list of fenced writers (`traits.rs:3845-3853`) gains
  "[`WriteStore::admit_plan`] and [`WriteStore::abandon_plans`] (MOD-139 M2 D9, fenced on the
  planner step)".
- `store/worker.rs`: the two `WorkerStore` declarations after `insert_run_plan` (`:361`), and the
  `MemStore` impl bodies (UFCS, after `:926-928`). Doc counts (`:15-23`, `:233`): "`WorkerStore` is
  **70** methods" (it is 68 at `904839d5`: `git show HEAD:crates/htui-core/src/store/worker.rs |
  awk '/^pub trait WorkerStore/,/^}/' | grep -c '^    fn '`), the writes clause gains "MOD-139 M2
  `admit_plan` and `abandon_plans`", and the section comment `// -- 27 writes` becomes `// -- 29
  writes`. Recount after the edit (expect 70).
- `store/mod.rs`: re-export the nine helpers.
- Forwarders (every impl, C23): `htui-store/src/writer.rs:375` `impl WriteStore for Writer` (match
  `Memory`/`Online`, as `:1271-1280`); `htui-store/src/worker.rs:116` (PgStore) and `:526` (Writer)
  `WorkerStore` impls (`WriteStore::admit_plan(self, ..)`, as `:322-328`, `:732-738`);
  `htui-agent/src/conformance.rs:781` `UsageSpy` and `htui-agent/tests/recorder.rs:469` `SpyStore`
  (`self.inner.admit_plan(..)`, as `:1347-1353`, `:1073-1079`).
- Commit-2 stubs: `MemStore` and `PgStore` bodies `Err(StoreError::Backend("MOD-139 M2: not
  yet".into()))` with `_`-prefixed parameters; commits 4 and 5 replace them.

### T1.3 MemStore (`store/mem.rs`)

- `MemFault::AdmitPlan` (`:150-166`, doc "[`WriteStore::admit_plan`] (MOD-139 M2 D14): the
  admission a crash cut off before its commit").
- `State::admit_plan` (§1.4; beside `State::insert_run_plan` `:6937`) and `State::abandon_plans`
  (§1.6). Reuse in `admit_plan`: `step_slot_is_taken` for rule 8 by scanning `self.steps`, and the
  rows are built as `State::create_step` builds them (`:6670-6748`: the `RunStep` literal with
  `status: Pending` and `updated_at: now`); call a shared private `fn step_row(new: NewRunStep, now)
  -> RunStep` extracted from `create_step`'s literal rather than duplicating it.
- `MemStore`'s `WriteStore` wrappers beside `:10510-10513` (§1.4's fault and clock; `abandon_plans`
  with `self.now().trunc_subsecs(TIMESTAMPTZ_DIGITS)`).
- D12: `State::resolve_input` (`:6169`), the filter at `:6176-6181` becomes
  `.is_none_or(|step| step.selected != Some(false) && step.role != StepRole::Part)`, and its doc
  (`:6160-6168`) gains "and not a divided part's (MOD-139 D12: part documents are their integrate
  node's, through `{{parts}}`)".
- Unit test (mem.rs tests): `admit_plan_refuses_a_planner_already_admitted` — admit on a running
  planner, then rewind the planner to `running` inside `store.write(|state| …)` and admit a fresh
  `New` revision: `Constraint(planner_already_admitted(planner, 0))`, exactly, and nothing written
  (P-4).

### T1.4 Conformance cases (`store/conformance.rs`)

Append to `CASES` (`:92-364`) under `// MOD-139 M2: admission, abandonment and part documents
(plan D9, D12)`; one **boxed** arm each in `run_case_inline` (`:384`; stack note `:376-380`); bodies after
the M1 block (`:28203ff`) under `// ---- MOD-139 M2: …`. Fixtures to reuse: `leased_step(case,
store, a, at)` (`:8398`; its step is `pending` at 0 — move it `pending -> running` with
`transition_step` to make it the planner), `role_step(run, position, role, parent, key)`
(`:28221`), `new_run_plan(run, planner, revision)` (`:28207`), `constraint_exactly`,
`seam_clock()`. New local helpers: `fn admission(run, planner, plan: PlanToAdmit, parts:
Vec<NewRunStep>) -> PlanAdmission` (`base_refs: json!({"repo": "abc"})`, `at: seam_clock()`);
`fn part(run, planner, position, key) -> NewRunStep` (`role_step(run, position, Part,
Some(planner), Some(&format!("implement/{key}")))` with `phase_name` the same text);
`fn admitted_row(plan: &NewRunPlan, base_refs: &Value, created_at, updated_at) -> RunPlan`.

| # | Case | Asserts |
|---|---|---|
| 1 | `admit_plan_admits_a_new_revision` | Planner running; `New(new_run_plan(run, planner, 0))` with parts `api` at 5 and `ui` at 6. The answer equals `admitted_row(plan, base_refs, row.created_at, row.updated_at)` with `created_at == updated_at`; `run_plans(run) == [answer]`. `run_steps`: the planner `done`, `finished_at == Some(seam_clock())`; the two parts `pending`, role `Part`, parent the planner, `node_key`/`phase_name` as given, `attempt 1`, `fanout_index 0`. |
| 2 | `admit_plan_admits_a_proposed_revision` | `insert_run_plan(rev 1)`; admit `Proposed(id)` with one part at 6. The answer: same `id`, `revision 1`, `status Admitted`, `base_refs Some(base_refs)`, `body`/`digest`/`base_position` unchanged, `created_at` the insert's, `updated_at >= created_at`. `run_plans(run) == [answer]`. Planner `done`, part `pending`. |
| 3 | `admit_plan_refusal_order` | Each refused exactly and nothing written (`run_plans` empty, `run_steps` unchanged, planner still `running`): (a) `base_refs: json!([])` on an **unknown** run is the shape sentence, not `NotFound`; (b) unknown run → `NotFound { entity: "run", .. }`; (c) `Lease(b)` and `Unleased` on `a`'s run → `Fenced { step }` with `step == planner`, also with a planner of another run, so the fence comes first; (d) a planner of another run (a second `create_run` + `create_step`, unclaimed) → `planner_not_in_run(planner, run)`, and an unknown planner the same; (e) the planner still `pending` (a second `leased_step`) → `planner_not_running(step, StepStatus::Pending)`. |
| 4 | `admit_plan_refuses_a_foreign_part` | Exact sentences, nothing written: a part whose `run_id` is another run (`part_of_another_run`), whose parent is another step of the run (`part_of_another_planner`), and a `Step`-role row (`step_not_a_part`). |
| 5 | `admit_plan_refuses_a_taken_slot` | The planner is a fresh step at 1 moved to `running`; parts `api` at 5 (free) and `ui` at 0, the `leased_step` row's own slot `(0, 1, 0)`: `step_slot_is_taken(run, 0, 1, 0)`, exactly; `api` was not written either, the planner is still `running`, no plan. |
| 6 | `admit_plan_refuses_a_done_planner` | Case 1's admission (repeated here: every case has a fresh store), then a second admission on the same planner (a fresh `New` rev 1, parts at 7): `planner_not_running(planner, StepStatus::Done)`; `run_plans` still one row, no row at 7 (P-4). |
| 7 | `admit_plan_refuses_a_plan_not_proposed_or_not_its_own` | Two running planners `p` and `q` of one run; `insert_run_plan(rev 1 under p)`: admitting it under `q` → `plan_not_this_planners(id, q)`; an unknown id → the same sentence; after `abandon_plans(p)`, admitting it under `p` → `plan_not_proposed(id, PlanStatus::Abandoned)`. Nothing written. |
| 8 | `abandon_plans_abandons_admitted_and_proposed` | Under one planner: rev 0 admitted (case 1's admission, repeated) and rev 1 proposed (`insert_run_plan` after it). `abandon_plans(Lease(a), planner) == Ok(2)`; both read `Abandoned`, each `updated_at >= created_at`; a second call `Ok(0)`; `Lease(b)` → `Fenced { step: planner }`; `StepId::new()` → `NotFound { entity: "run_step", .. }`. |
| 9 | `resolve_inputs_skips_part_documents` | On a `leased_step` run of `HTUI_ANA_2`, through `write_document`: kind `implement` written three ways — hand-written v1 (`produced_by_step_id: None`), a plain step's v2 (`role Step` at 1), a part's v3 (`role Part` at 5 under a planner at 2); kind `notes` written twice — hand-written v1, the part's v2. `resolve_inputs(HTUI_ANA_2, run, ["implement", "notes"])` answers `implement` **v2** (the part's newer v3 is skipped, the plain step's kept) and `notes` **v1** (the hand-written one, `s.role` NULL, kept). |

Count: `CASES.len()` 258 → **267**. P-9 (§6 G-4): every backticked snake_case name with four or
more underscores in this file's text must be a fn here or in `mem.rs` — the nine case names are;
never backtick a constraint, index or trigger name (`chk_run_plan_admitted_base_refs`,
`trg_run_plan_updated_at`, `idx_run_step_parent_step`), and write "the planner's update lock"
rather than any lock-mode identifier with underscores.

### T1.5 PgStore (`pg/write.rs`, `pg/read.rs`) and the mirror (`cache/read.rs`)

- `pg/write.rs`: `admit_plan` and `abandon_plans` in `impl WriteStore for PgStore` (`:1795`),
  after `insert_run_plan` (`:7886`), exactly §1.5. Imports: `PlanAdmission, PlanToAdmit` and the
  helpers. `rows_affected() != 1` at W3 is `StoreError::Backend(format!("planner step {} moved
  under its admission lock", planner))` (unreachable under S1's lock; no case).
- `pg/read.rs:907` (`resolve_inputs`): `AND (s.id IS NULL OR s.selected IS NOT FALSE)` becomes
  `AND (s.id IS NULL OR (s.selected IS NOT FALSE AND s.role <> 'part'))` (D12; a bare `AND s.role
  <> 'part'` would drop hand-written documents, whose `s.role` is NULL). Doc (`:827-843`) gains the
  part sentence.
- `cache/read.rs` (`:1153`): `AND (s.id IS NULL OR COALESCE(s.selected, 1) <> 0)` becomes
  `AND (s.id IS NULL OR (COALESCE(s.selected, 1) <> 0 AND s.role <> 'part'))`; the mid-pass
  paragraph (`:1100-1110`) gains: "A **part's** document can be answered offline the same way while
  its `run_step` row has not arrived (MOD-139 D12, accepted like the loser's)."

### T1.6 `.sqlx` regeneration (M1 §T3.6's recipe)

New entries: S1, S3, S4, S5, W1 (`New`), W1 (`Proposed`), W3, the abandon `UPDATE`; replaced:
`resolve_inputs`. Reused byte-identical (no new file): S2 (`insert_run_plan`'s run read) and W2
(`create_step`'s insert), and `step_fence`'s `Update` statement.

```bash
cd /home/mluigi/projects/htui/crates/htui-store
BASE="${HTUI_TEST_DATABASE_URL%/*}"          # postgres://postgres@localhost:5439
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE IF EXISTS mod139_m2_prepare' -c 'CREATE DATABASE mod139_m2_prepare'
DATABASE_URL="$BASE/mod139_m2_prepare" sqlx migrate run --source migrations
DATABASE_URL="$BASE/mod139_m2_prepare" cargo sqlx prepare -- --all-targets --all-features
DATABASE_URL="$BASE/mod139_m2_prepare" cargo sqlx prepare --check -- --all-targets --all-features
psql "$HTUI_TEST_DATABASE_URL" -c 'DROP DATABASE mod139_m2_prepare'
```
Keep `DATABASE_URL` exported to the migrated scratch DB while writing queries (the macros check
online). No migration in M2.

### T1.7 Pins and Postgres-only tests

- `htui-core/tests/mem_store.rs:37`: `258` → `267`; append to the message "…, and MOD-139 M2's
  nine admission, abandonment and part-input cases (plan D9, D12)".
- `htui-store/tests/pg_conformance.rs:53-62`: `EXPECTED_CASES = 267`, doc line "and MOD-139 M2's
  nine admission, abandonment and part-input cases (plan D9, D12) make it 267"; extend the message.
- `htui-store/tests/pg_criteria.rs`, beside `pass_step_locks_the_step_for_update_up_front`
  (`:8089`): `admit_plan_takes_the_step_first` and `admit_plan_locks_the_step_for_update_up_front`.
  Each: `let (run, step, owner) = leased_running_step(&db).await;` (`:7354`; its step is the
  planner, F-2 of M1: no role check), one part at position 50 (`role Part`, parent `step`,
  `node_key "implement/api"`), `base_refs: json!({})`; the write is
  `store.admit_plan(StepFence::Lease(owner), admission)`; the answer is `Ok` with `status ==
  Admitted`, and `run_step` reads the planner `done`.
- `htui-store/tests/cache.rs`, beside `the_mirror_ranks_resolve_inputs_the_way_postgres_does`
  (`:1881`): `the_mirror_skips_part_documents_the_way_postgres_does` — on `ids::RUN_1`: a planner
  (`create_step`, role `Planner`, position 60) and a part (61, parent the planner); then **both**
  tables seeded before the pass: a hand-written and a part document of one kind on the run's item;
  `run_pass`; `cache.resolve_inputs(item, RUN_1, [kind]) == db.store.resolve_inputs(..)`, and the
  answer is the hand-written one.

### T1.8 Commits (each compiles: `cargo check --workspace --all-targets --all-features` first)

1. `feat(model): MOD-139 M2 T1, PlanAdmission, PlanToAdmit and the plan status law` — §T1.1.
2. `feat(store): MOD-139 M2 T1, admit_plan and abandon_plans on every store and forwarder` —
   §T1.2 (stubs), `MemFault::AdmitPlan`, the helper unit tests.
3. `test(store): MOD-139 M2 T1, nine admission, abandonment and part-input conformance cases (red)`
   — §T1.4 and the two count pins.
4. `feat(store): MOD-139 M2 T1, MemStore admission, abandonment and part-document exclusion` —
   §T1.3.
5. `feat(store): MOD-139 M2 T1, PgStore admission and abandonment, part documents out of input
   resolution on Postgres and the mirror` — §T1.5, `.sqlx`, the `pg_criteria` and `cache.rs` tests.

### T1.9 Validate (`--features test-support` for htui-core, P-10 of M1; `--all-features` for htui-store)

```bash
cargo test -p htui-core --features test-support --lib -- model:: store::traits store::mem
cargo test -p htui-core --features test-support --test mem_store -- --test-threads=1
cargo test -p htui-core --features test-support --lib conformance::
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_m2_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
cargo test -p htui-store --all-features --no-fail-fast --test pg_conformance --test pg_criteria --test cache -- --test-threads=1
cargo test -p htui-agent --all-features --no-run
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
```
`conformance::tests::every_cross_referenced_test_name_exists` and
`run_case_accepts_every_name_in_cases` must pass.

---

## T2 — Prompt part section and `fan_out_only` (parallel with T1)

### T2.1 Prompt (`htui-core/src/prompt/`)

§1.9 in full: `PartBlock` + `From<&SnapshotPart>`, `PART_SEPARATOR`, `PromptSpec.part`,
`Placeholder::Part` (token, `allowed_in`, docs of `ALL`/`is_section`), `SectionName::Part`
(`render`, `is_protected`), `render::part`, `assemble`/`substitute`/`render_sections`/`part_of`,
`scrubbed_inputs`, both `trim.rs` matches, the rule-8 doc narrowing.

### T2.2 The seven full `PromptSpec` literals and the spread

`part: None,` at `prompt/fixtures.rs:167`, `:270`, `:377`, `:520`; `htui-orch/src/engine.rs:6411`
(judge spec) and `:7272` (`phase_spec`); `htui/src/preview.rs:297`. `promote.rs:85-115`
(`handoff_spec`): `part: None,` beside `persona`/`memory`, its doc (`:78-84`) gains "and `part`
(MOD-139 D10)"; the test literal at `promote.rs:464-480` gains `part: None,` too. The `..spec.clone()`
spreads (`mod.rs:713`, `:1669`, `htui-agent/src/excerpt.rs:1147`) need nothing. New fixture
`pub fn part_api() -> PartBlock` in `prompt/fixtures.rs` (§1.9's values).

### T2.3 `fan_out_only` (`model/kind.rs`)

§1.11. The one engine caller (`engine.rs:7143`) becomes `phase.command_queue.exposed(phase.fan_out,
false, required_tags)`; T3 wires the real fact. Tests in `kind.rs` (`:757-805`): the existing
`exposed`/`command_queue_exposed` calls gain `false`; new `fan_out_only_follows_live_siblings`:
`FanOutOnly.exposed(1, true, &[])`, `!FanOutOnly.exposed(1, false, &[])`, `!Off.exposed(2, true, ..)`,
`Always.exposed(1, false, &[])`, and `command_queue_exposed(FanOutOnly, 1, true, &[])`.

### T2.4 Tests to write first

| Test | File | Asserts |
|---|---|---|
| `section_names_are_one_vocabulary_with_two_uses` (extended) | `prompt/mod.rs:1471` | `SectionName::Part.render() == "part"` |
| `the_protected_set_is_role_dependent_only_for_failure_reason` (extended) | `prompt/mod.rs:1488` | `Part` in the protected list for all three roles |
| `part_renders_brief_paths_and_acceptance` | `prompt/render.rs` tests | `wrap(&part(&fixtures::part_api()))` is §1.9's block, byte for byte; an all-empty `PartBlock` is `<section name="part" key="" title="">\n</section>` |
| `a_part_section_renders_after_persona_and_memory` | `tests/prompt_part.rs` (new, `#![cfg(feature = "test-support")]`) | `phase_implement_attempt2()` + persona + memory + part: the text holds the persona span, the memory span, then `wrap(part) + PART_SEPARATOR`, then the body, in that order (one `find` each, increasing) |
| `the_part_section_is_recorded_after_persona_and_memory` | `tests/prompt_part.rs` | `sections` names: with persona and memory `[template, persona, memory, part, …]`; with neither `[template, part, …]` |
| `the_part_placeholder_is_internal` | `tests/prompt_part.rs` | `Placeholder::Part.token() == "part"`, not in `ALL`, `!allowed_in(role)` for all three roles, a body `"{{part}}"` is `AssembleError::UnknownPlaceholder { token: "part" }` |
| `an_oversized_brief_refuses_the_prompt` | `tests/prompt_part.rs` | a brief past the target (as `prompt_persona.rs`'s `at_target`) → `AssembleError::BudgetTooSmall { .. }`; nothing trims it |
| `a_part_secret_is_masked` | `tests/prompt_part.rs` | `MinimalScrubber::new([secret])` with the secret in brief, title and one path: the text holds `[REDACTED]` and not the secret (the shape `prompt_persona.rs` uses) |
| `only_a_phase_spec_renders_a_part` | `tests/prompt_part.rs` | judge and handoff fixtures with `part: Some(..)` render no `part` section; `fixtures::phase_all_empty()` with `part: None` is byte-identical to before (the `prompt_golden` snapshots stay untouched) |
| `fan_out_only_follows_live_siblings` | `model/kind.rs` tests | §T2.3 |

### T2.5 Commits

1. `feat(prompt): MOD-139 M2 T2, the internal part section` — §T2.1, §T2.2, its tests.
2. `feat(model): MOD-139 M2 T2, fan_out_only follows live siblings` — §T2.3.

### T2.6 Validate

```bash
cargo test -p htui-core --features test-support --lib -- prompt:: model::kind
cargo test -p htui-core --features test-support --test prompt_part --test prompt_persona --test prompt_memory --test prompt_golden --test prompt_render --test prompt_output --test prompt_digest -- --test-threads=1
cargo test -p htui-orch --features test-support --lib -- promote::
cargo check -p htui --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
```
No `.snap` may change (`git status crates/htui-core/tests/snapshots` clean).

---

## T3 — Engine: rows by role, admission, running parts (serial, after T1 and T2 merge)

### T3.1 `graph.rs`: the seam (D16), D17, the corrected count (D8)

- `NotRunnable::PlannerAgent` (§1.7), declared first in the enum.
- `not_yet_runnable(phases, lift_divide: bool)` (`:710-728`), in this order: (1) **new**: the first
  phase in position order with `role == NodeRole::Planner` whose `divide.planner.agent_id` is
  `Some` → `refuse(divided_name(phase), NotRunnable::PlannerAgent)`; (2) unless `lift_divide`, the
  existing first non-step node → `Divide` (its `:718-721` name strip becomes `divided_name`);
  (3) the palette branch (`:724-726`) unchanged.
- Refactor `resolve_or_refused` (`:457`) into a private `resolve_or_refused_with(.., lift_divide:
  bool)` called by `resolve_or_refused` with `false`; `resolve` (`:385`) into a private
  `resolve_with(.., lift_divide)` called with `false`. The flag is a plain `bool` (an enum variant
  built only under `test-support` would be dead code to the featureless clippy).
- The seam, after `resolve`:
  ```rust
  /// MOD-139 M2 D16: [`resolve`] with **only** the `NotRunnable::Divide` refusal lifted — the
  /// palette, the planner-agent (D17) and the `local` refusals, the validator and the caps all
  /// stand. Engine conformance builds divided runs through it; no `Engine` path calls it. M3
  /// deletes it with the refusal.
  /// # Errors
  /// As [`resolve`].
  #[cfg(any(test, feature = "test-support"))]
  pub async fn resolve_divided<S: htui_core::store::WorkerStore, G: GraphSource>(
      store: &S, source: &G, item: &Item, mode: RunMode, app: &BTreeMap<String, Value>,
      requested_scope: Option<&[RepoId]>, box_id: BoxId,
  ) -> std::result::Result<Resolved, ResolveError>
  ```
- `check_fan_out_caps` (`:1428-1458`): the planned count is
  `crate::divide::planned_sessions(phases, None)`; its doc says how a divide node counts (D8).
- Unit tests beside `a_divide_node_is_expanded_then_refused` (`:5143`):
  `a_planner_agent_is_refused_before_the_divide` (`replace_feature` with `planner_agent_id =
  Some(ids::AGENT_CLAUDE)` → `NotYetRunnable { node: "implement", reason: PlannerAgent }`, its text
  contains "MOD-140"); `the_seam_lifts_only_the_divide_refusal` (`resolve_divided` over the same
  divide-without-agent graph resolves, `phases[2]` is `implement:plan`/`Planner` and `phases[3]`
  `implement`/`Integrate`; with the planner agent it is `PlannerAgent`; with a palette node it is
  `Palette`); `fan_out_caps_count_parts_not_the_planner` (pure: phases `prd`, a planner with three
  default parts and no planner agent, its integrate, `review`; planned = 1 (`prd`) + 3 (the
  parts; no planner session) + 1 (integrate) + 1 (`review`) = 6, so `max_agents_per_run` 6 →
  `Ok` and 5 → `AgentCap { planned: 6, max: 5 }`; the old count, `fan_out` per node, said 4).

### T3.2 `divide.rs` (new) and `lib.rs`

§1.8. `lib.rs`: `pub mod divide;`; `pub use command::{…}` gains `node_of, operator_row`;
`#[cfg(feature = "test-support")] pub use engine::{…}` (`:65-66`) gains `enqueue_divided_fake`.
Unit tests in `divide.rs`: `part_names_carry_the_revision_from_one_on` (`implement/api`,
`implement/api@1`, `node_key` `implement/api` both), `the_base_position_follows_nodes_rows_and_plans`
(FEAT-3 divided: 5; with rows to 6 and a plan at 5 with two parts: 7; a stage row at −1 ignored),
`admission_refusals_come_in_order` (one assertion per rule of §1.8, each with the earlier rules
satisfied, plus a fixture breaking rules 3 and 4 at once answering `MaxParts`, and one breaking rule 2 and a part persona answering `LocalCannotDivide`), `owned_paths_must_be_repo_relative_globs`
(`src/**` and `htui:src/**` pass; `/etc/x`, `htui:/x`, `src/../x`, `[` refuse with their reasons),
`planned_sessions_count_parts_not_retries`, `live_siblings_skip_stages_judges_and_this_step`,
`parts_width_is_one_in_place_else_capped` (`shared_serialized` 1, `local` 1, `worktree` 3 parts cap 2 → 2).

### T3.3 `status.rs`

- `Cursor::Parts` (§1.7), `RunFailure` × 7 and their `Display` arms, `PlanCap` (§1.7).
- `position_of` (`:487-502`): `Cursor::Parts { position } => return Some(*position)`; the row
  lookup maps a `role == Part` row to its parent's position (P-14).
- `resumable_park` (`:433-441`): add `| Cursor::Parts { .. }` and the doc line "`Parts` (MOD-139
  M2 D4) is a write the engine owes, as `Decide`".
- Tests: `run_failure_display_is_ana2s_bytes` (`:744`) gains the seven exact strings of §1.7;
  `position_of` gains `Parts` and a part-row `Rest` (answers the planner's position);
  `resumable_park(&Cursor::Parts { position: 2 })`.

### T3.4 `walk.rs` (D3)

In `node_state` (`:66-100`), after `let Some(step) = latest_at(steps, position)`:
`if phase.role == NodeRole::Planner && step.status == StepStatus::Done { return parts_state(phase,
step, steps); }`. New private `parts_state` in this precedence (P-14, P-15):
1. a latest part `running | stopped | awaiting_approval` → `Live(Cursor::Rest { step: part.id,
   status })`;
2. a latest part `exhausted(part, phase.retry_limit)` → `Live(Cursor::Rest { step: planner.id,
   status: StepStatus::Failed })` (the planner row is `done`; the rest's status is the node's);
3. every latest part `done` (vacuously with none, F-3) → `Done(planner)`;
4. otherwise → `Live(Cursor::Parts { position })`.
The `NodeState::Live` doc names `Cursor::Parts`. Unit tests (with `tests::row`, `graph`, `node`
at `:881-983`; build a planner/integrate pair by hand): `a_pending_planner_runs_and_a_done_one_reads_its_parts`,
`a_divided_node_is_done_only_when_every_part_is`, `a_live_part_rests_the_node`,
`an_exhausted_part_rests_the_planner_failed`, `a_failed_part_with_an_attempt_left_is_owed_parts`,
`stage_and_part_rows_are_invisible_to_other_nodes` (a stage row at −1 and part rows at 5/6 leave
`ready` over a linear snapshot unchanged), `integrate_is_offered_once_the_parts_are_done`
(`ready == [Create { position: 3, attempt: 1 }]`).

### T3.5 `schedule.rs`

`candidate` (`:185-219`): `Cursor::Parts { .. } => (crate::divide::parts_width(snapshot, steps,
phase), true)`; rule (e) (`:158-161`): `Cursor::Create { .. } | Cursor::Run(_) | Cursor::Fan { .. } |
Cursor::Parts { .. }`. Tests: `a_parts_cursor_weighs_its_width_and_touches` and
`a_parts_cursor_waits_on_its_inputs_like_a_run`.

### T3.6 `command.rs`

`EngineError::RoleRow` + `role_row` (§1.7), `node_of`, `operator_row` (§1.7). Unit tests:
`node_of_maps_parts_to_their_planner_and_stages_to_none`, `operator_row_refuses_parts_and_stages`
(the two sentences exactly).

### T3.7 `engine.rs` (anchors at `904839d5`)

| Site | Change |
|---|---|
| `answer_gate` `:1424`, `retry_step` `:1806`, `promote` `:2078`, `accept_artifact` `:2327` | right after the row read: `crate::command::operator_row(&step)?;` (`&row` in the last three), before `phase_at` |
| `admit_indices` `:4552-4565` | `role: crate::divide::row_role(phase.role)`, `node_key: (phase.role == NodeRole::Planner).then(\|\| phase.name.clone())` |
| `drive_cursor` `:4026-4070` | `Create`: an `Integrate` node → `Err(EngineError::Resolve(ResolveError::NotYetRunnable { node: phase.name.clone(), reason: NotRunnable::Divide }))` (D15) before `admit`; `Run`: `node_at`, then `Planner` → `Box::pin(self.admit_planner(..))`, `Integrate` → the D15 error, `Step` → `walk_step`; new `Cursor::Parts { position }` → `phase_at` then `Box::pin(self.drive_parts(row, snapshot, &phase))` |
| `recover_run` `:3366` | stage rows `continue`; `node_at` instead of `phase_at` (D14's role dispatch is T4) |
| `settle_failed` `:3738` | `node_at` instead of `phase_at` (planner branch is T4) |
| `land_recovered` `:3601`, `walk_live_step` `:5037` | unchanged, one comment each (P-6) |
| `cancel_unsettled` `:4150-4170` | ender via `divided_name`; `if step.role == StepRole::Stage { continue; }` (P-19) |
| `resting` `:8179-8199` | `position: crate::status::position_of(&cursor(&snapshot, &steps), &steps)` (P-14) |
| `walk_live_step` `:4871` | `live` computed before stage 3; `assemble_prompt(.., None, live)`, `session(.., live)` |
| `drive_group` `:5311`, `CandidateStage` `:8401` | `live_siblings` once for `first`; new field; `candidate_live`'s `drive_once` passes it |
| `phase_spec` `:7158`, `command_run_exposed` `:7142`, `assemble_prompt` `:7047`, `session` `:7498`, `drive_once` `:7752`, judge `:6661`, handoff `:2231` | §1.10's threading |
| `phase_skills` `:7345-7370` | `row.name == crate::divide::divided_name(phase)` (P-9) |
| new | `PartStage`, `PartOutcome`, `admit_planner`, `admit_parts`, `refuse_admission`, `drive_parts`, `run_part`, `part_live`, `fail_part`, `node_at` (§1.10, §1.7) |
| new, after `claim_fake` `:8924-8935` | `enqueue_divided_fake` (P-10): `item = orch.store.item(item)?` (NotFound as `enqueue_with` reads it), `graphs = orch.graphs()`, `app = orch.store.app_settings().await?`, `resolved = graph::resolve_divided(&orch.store, &graphs, &item, RunMode::Manual, &app, None, orch.box_id()).await?`, `orch.store.create_run(NewRun { workspace_id: None, id, project_id, item_id, mode: Manual, target_box_id: orch.box_id(), started_by: orch.user(), graph_snapshot: resolved.snapshot, repo_scope: resolved.repo_scope, queued_at: orch.clock.now(), batch_id: None })` (`enqueue_with`'s literal, `:1277-1292`) |

`gate.rs`'s apply arm (P-7) is T4's file; T3 does not touch `gate.rs`.

### T3.8 `htui-worker/src/views.rs`

In `verdicts`' step loop (`:369`), before `phase_at`: `if let Err(err) = operator_row(step) {
actions.steps.insert(step.id, refused_step(&err.to_string())); continue; }` (import
`htui_orch::operator_row`). `selectable_slots` unchanged (P-20). Test (views.rs tests): a run
holding a part row reports its verdicts refused with the part sentence.

### T3.9 The harness (`htui-orch/src/conformance.rs`)

- `Orchestrate` (`:86-256`) gains, with a doc naming D16 and P-10:
  `async fn enqueue_divided(&self, item: ItemId) -> Result<RunId, EngineError>;`, implemented for
  `FakeOrchestrator` (`:258ff`) as `crate::engine::enqueue_divided_fake(self, item).await`.
- Helpers, in a `// -- MOD-139 M2 T3: divided nodes (plan D1-D5, D7, D8, D10, D13, D15-D17) --`
  block:
  - `async fn divided_feat_3<O: Orchestrate>(orch: &O, parts: &[(&str, Option<AgentId>)], mutate:
    impl Fn(&mut StepGraphPhase))` — `feat_3_gated(orch, Gate::Never, |phase| {
    phase.input_kinds.clear(); })` (`:5438`), then `edit_graph(orch, ids::HTUI_FEAT_3, |body| …)`
    (`:9667`): on the `implement` node `split = Split::Divide`, `fan_out = 1`, `mutate(&mut
    node.phase)`, and `node.parts = parts.enumerate().map(|(i, (key, agent))| PhasePart {
    phase_id: node.phase.id, position: i, key, title: format!("The {key}"), brief:
    format!("Build the {key} part."), owned_paths: vec![format!("src/{key}/**")], acceptance:
    Some(format!("the {key} tests pass")), agent_id: agent, model: None, persona_id: None })`.
  - `async fn start_divided<O: Orchestrate>(orch: &O) -> (RunId, Result<CommandOutcome,
    EngineError>)` — `orch.enqueue_divided(ids::HTUI_FEAT_3)` then `orch.claim(run)`.
  - `fn stops_at_integrate(result: &Result<CommandOutcome, EngineError>)` — asserts
    `matches!(result, Err(EngineError::Resolve(ResolveError::NotYetRunnable { node, reason:
    NotRunnable::Divide })) if node == "implement")` (D15).
  - `fn parts_of(steps: &[RunStep], planner: StepId) -> Vec<RunStep>` (rows under `planner`,
    `(position, attempt)` order) and `fn planner_of(steps: &[RunStep]) -> RunStep` (the latest
    `role == Planner` row).
  - Sessions are scripted and held by the part's `phase_name`: `orch.script("implement/ui", 1,
    ScriptedStep::done_without_output())`, `orch.hold_sessions(&[("implement/api", 1),
    ("implement/ui", 1)])`, `spec_of(&orch, &session("implement/api", 0))` (`:8559-8575`) —
    `SessionKey::of` a part is `(phase_name, attempt, 0, 0)` (`engine.rs:267-274`).
  - Trees and bases: `primary_repo(&orch)` (`:1929`) before `enqueue_divided` gives a one-repo
    scope; `FakeIsolator::base` answers `fake:group-base:<repo>` (`fake.rs:832-838`), so a part
    prepared with a slot records `base_ref == "fake:group-base:<repo>"` (`fake.rs:704-706`), and one
    prepared with `slot: None` a fresh `fake:base:<n>` — that is how a case asserts D4's `None`
    (the fake records no slot; `GixIsolator` would refuse one under `local`, §6 G-2).
    `call_log_of(&orch, run)` (`:11224`) names steps `position/attempt/fanout_index`.

### T3.10 Cases (`htui-orch/src/conformance.rs`)

Append to `CASES` (`:567-911`) under `// MOD-139 M2 T3 (plan D1-D5, D7, D8, D10, D13, D15-D17):
divided nodes, admission and parts.`, a new frame `fn divided_case` (shape of `concurrency_case`,
`:11154-11222`) tried in `case()` (`:932-951`) before `earlier_case`, each arm `Box::pin`. The `CASES`
doc (`:520-566`) gains a "**Eighteen for MOD-139 M2 T3**" paragraph. Every case starts
`harness.fresh()` and (unless stated) `divided_feat_3(&orch, &[("api", None), ("ui", None)], |_| {})`.

| # | Case | Asserts |
|---|---|---|
| 1 | `a_divided_node_admits_its_default_parts_as_revision_zero` | With `primary_repo`: `stops_at_integrate`. `run_plans(run)`: one row, `revision 0`, `Admitted`, `base_position 5`, `base_refs == {"<repo>": "fake:group-base:<repo>"}`, `planner_step_id` = the planner row, `body == plan_body(default_parts)`, `digest == plan_digest(&body)`. Planner row: position 2, `role Planner`, `node_key "implement:plan"`, `Done`, `finished_at` set. Parts: `(5, "implement/api")`, `(6, "implement/ui")`, `role Part`, parent the planner, `node_key` = `phase_name`, `attempt 1`, `fanout 0`, `Done`. No row at position 3. `walk::ready(&snapshot, &steps) == [Cursor::Create { position: 3, attempt: 1 }]`. |
| 2 | `each_part_runs_in_its_own_tree_with_its_own_prompt_and_agent` | `with_candidates("implement", [(AGENT_CLAUDE, "sonnet"), (AGENT_AGY, "gemini-3.7-flash-high")])`, part `ui` names `AGENT_AGY`; `primary_repo`. Each part's `step_trees` holds one row whose `base_ref` is the plan's `base_refs[repo]` and whose paths differ; `call_log_of` holds `prepare 5/1/0`, `capture 5/1/0`, `prepare 6/1/0`, `capture 6/1/0` and no `reconcile 5/…` or `reconcile 6/…`; `api`'s row `agent_id == AGENT_CLAUDE`, `ui`'s `AGENT_AGY` with model `gemini-3.7-flash-high`; `prompt_sections(part)` has `"part"` at index 1; `prompt_text` of `api` contains `key="api"` and "Build the api part." and not "Build the ui part."; the two `prompt_digest`s differ; both `spec_of(session("implement/api", 0))` and `("implement/ui", 0)` exist. |
| 3 | `part_documents_stay_out_of_input_resolution` | After the run stops, both parts wrote an `implement` document (`documents(item)` holds two with `produced_by_step_id` the parts); `store().resolve_inputs(HTUI_FEAT_3, run, ["implement"])` answers `None` for the kind (no hand-written one exists). |
| 4 | `integrate_is_offered_only_once_every_part_is_done` | `hold_sessions([("implement/ui", 1)])`; join the walk with: `until_steps` api `Done`, then `walk::ready(&snapshot, &steps) == [Cursor::Rest { step: ui.id, status: StepStatus::Running }]` (a live part rests its node, §T3.4 rule 1; no `Create` at 3) and no row at 3, then `release`. After: `stops_at_integrate`, `ready == [Create { 3, 1 }]`. |
| 5 | `two_parts_are_live_at_once_under_the_cap` | `hold_sessions` both; `within(hold.live(2))`; release; `live_sessions_high_water() == 2`; both `Done`. |
| 6 | `max_parallel_sessions_one_runs_the_parts_one_at_a_time` | `set_app_setting("max_parallel_sessions", json!(1))` **before** `enqueue_divided`; hold both, `hold.live(1)`, 64 yields, `hold.reached() == 1`, release; `live_sessions_high_water() == 1`; both `Done`. |
| 7 | `shared_serialized_parts_reset_to_the_base_refs` | `mutate`: `isolation = Some(SharedSerialized)`; `primary_repo`. Both parts' `base_ref` is `base_refs[repo]`; the log's part entries are `prepare 5/1/0, capture 5/1/0, prepare 6/1/0, capture 6/1/0, reset 6/1/0` in that order; `in_place_overlaps() == 0`; `live_sessions_high_water() == 1`. |
| 8 | `a_single_local_part_prepares_with_no_slot` | parts `[("api", None)]`, `isolation = Some(Local)`, `primary_repo`. The part `Done`; its tree's `base_ref` starts with `fake:base:` and differs from `base_refs[repo]` (no slot, D4/D7). |
| 9 | `local_with_two_parts_fails_admission` | `isolation = Some(Local)`. `claim` → `Started { rest }` with `rest.run == Failed`, `rest.failure == Some(LocalCannotDivide { node: "implement", parts: 2 })`; `run.failure` is its `Display`; planner `Failed`; `run_plans` empty; no part row; the item reads `blocked` (`item_of(..).status == Status::Blocked`); its last note is the sentence. |
| 10 | `a_part_agent_outside_the_candidates_fails_admission` | part `ui` names `AGENT_AGY` (the fake's fallback candidate list is `AGENT_CLAUDE` only): `PartAgentNotCandidate { node: "implement", part: "ui", agent: AGENT_AGY }`, as case 9's shape. |
| 11 | `a_part_persona_fails_admission` | `edit_graph` sets `ui`'s `persona_id = Some(ids::PERSONA_ARCHITECT)`: `PartPersonaUnsupported { node: "implement", part: "ui" }`, as case 9. |
| 12 | `an_over_cap_plan_fails_admission` | `mutate`: `max_parts = Some(1)`: `PlanOverCaps { node: "implement", cap: PlanCap::MaxParts, planned: 2, max: 1 }`, as case 9. |
| 13 | `an_empty_divided_node_fails_admission` | `divided_feat_3(&orch, &[], ..)`: `EmptyPlan { node: "implement" }`, as case 9. |
| 14 | `a_part_with_a_live_sibling_gets_command_run` | `harness.fresh().hosted()`; `mutate`: `command_queue = CommandQueue::FanOutOnly`. `spec_of(session("implement/api", 0)).tools.command_run` and `("implement/ui", 0)` both `true`, each with `has_denials`; each part prompt has a `command_queue` section; `prd`'s spec (`session("prd", 0)`) has neither (a lone plain step, `fan_out_only`, no tag). |
| 15 | `a_fan_out_only_branch_beside_a_live_sibling_gets_command_run` | `hosted()`; `feat_3_graph(&orch, DIAMOND, \|phase\| phase.command_queue = CommandQueue::FanOutOnly)` (`:11310`, `:11324`); `hold_sessions([("plan", 1), ("implement", 1)])`, `within(hold.live(2))`, release. Both `spec_of(session("plan", 0))` and `("implement", 0)` have `command_run` and the denials (D13's deliberate change, §6 G-9); `prd` has none; the run ends `Done`. |
| 16 | `the_operator_verbs_refuse_a_part_row` | After a case-1 run (repeated here; every case has a fresh orchestrator): `AnswerGate { answer: Approved }`, `RetryStep`, `PromoteStep { chat_open: false }`, `AcceptArtifact { chat_live: false }` on the `api` part each `Err(EngineError::RoleRow { step, role: StepRole::Part })` with `step == api.id`; its text is `operator_row`'s; no row moved. |
| 17 | `stage_rows_are_ignored_by_the_walk` | Plain FEAT-3 (`feat_3_gated(Gate::Never, ..)` with `prd.gate = Gate::Always`); `start` parks at `prd`; `create_step` three stage rows (`role Stage`, −3 `pick`, −2 `prepare`, −1 `collect`, `phase_name "stage"`); `answer_at(.., 0, Approved)` → `Done`; the stage rows are still `pending`; `walk::ready == [Cursor::Finished]`. |
| 18 | `start_run_refuses_a_divided_graph_and_first_a_planner_agent` | `StartRun` (production resolve) on the divided graph → `Err(Resolve(NotYetRunnable { node: "implement", reason: Divide }))`; after `edit_graph` sets `planner_agent_id = Some(AGENT_CLAUDE)` on `implement`: `StartRun` → `PlannerAgent`, and `orch.enqueue_divided(HTUI_FEAT_3)` → `PlannerAgent` too (the seam keeps D17). No run created by any of the three. |

### T3.11 Pins

- `conformance.rs` `cases_are_unique_and_counted` (`:13513-13522`): `186` → `204`; the arithmetic
  string gains ` + 18` and the prose "…, and MOD-139 M2 T3's eighteen divided-node cases (plan
  D1-D5, D7, D8, D10, D13, D15-D17)".
- `tests/fake_conformance.rs:17-61`: `186` → `204`; message appends ", and MOD-139 M2 T3's eighteen
  divided-node cases (admission of revision 0 from base refs, parts in their own trees with their
  own prompt and agent, part documents out of input resolution, integrate only after every part,
  the session cap, `shared_serialized` resets, a slotless `local` part, five admission refusals,
  `command_run` beside live siblings and on a parallel branch, the operator verbs and stage rows,
  and the resolve refusals) 204".

### T3.12 Commits

1. `feat(orch): MOD-139 M2 T3, the divided-graph resolve seam, the planner-agent refusal and the
   divide rules` — §T3.1, §T3.2 (with `divide.rs`'s tests).
2. `feat(orch): MOD-139 M2 T3, rows map to nodes by role and the walker derives a divided node from
   its parts` — §T3.3–§T3.6, §T3.8, `operator_row` at the four verbs, `cancel_unsettled`, `resting`.
3. `test(orch): MOD-139 M2 T3, eighteen divided-node cases and the case-count pins (red)` —
   §T3.9–§T3.11, with `enqueue_divided_fake` real (commit 1 landed the seam); the cases fail on
   the missing admission and parts.
4. `feat(orch): MOD-139 M2 T3, admission and running parts` — the rest of §T3.7.

### T3.13 Validate

```bash
cargo test -p htui-orch --features test-support --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m2-t3.log
grep -c SIGABRT /tmp/mod139-m2-t3.log      # expect 0
cargo test -p htui-worker --all-features --lib -- views
cargo test -p htui-core --features test-support --test mem_store -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
```
Never wait on the suite with a `pgrep -f "cargo test…"` loop (it matches itself; memory
agent-pgrep-wait-loops-self-match).

---

## T4 — Engine: retries, failure, loops and recovery (serial, after T3)

Test-first: §T4.6's nine cases (commit 1) are red on T3's tree; commits 2 and 3 turn them green.

### T4.1 `gate.rs` (D6's lookup, D11)

- `apply`'s `(Gate::Never, Settle::Failed)` arm (`:447-458`, the lookup at `:450-452`): `loop_edge(crate::divide::divided_name(phase),
  EdgeKind::Fails)` (P-7).
- `retire_slot` (`:1317-1361`): after the slot's loop, for the slot's row with `role ==
  StepRole::Planner` (the slot is the planner node's latest attempt), call
  `retire_parts(ctx, steps, planner, now)` and then `ctx.store.abandon_plans(ctx.fence,
  planner.id).await?`. New `async fn retire_parts(ctx, steps: &[RunStep], planner: &RunStep, now)`:
  for each `crate::divide::latest_parts(steps, planner.id)` row, `retire_slot`'s status rule —
  `pending | awaiting_approval | done` → `supersede_step`; `running | stopped | failed` →
  `move_step(.., Cancelled)`; `superseded | cancelled` untouched. Older attempts stay as they are
  (D11). `retire_slot`'s doc gains the D11 paragraph (ANA-33 §7.3: "That includes their parts and
  plans").
- Unit test (gate.rs tests, beside `retire_cancels_a_failed_candidate_and_a_failed_judge`
  `:2887`): `retire_takes_a_planners_parts_and_abandons_its_plans` — a `done` planner with a
  `done` part, a `failed` part and an older `failed` attempt of it, and an admitted plan: after
  `retire_slot`, the planner `superseded`, the done part `superseded`, the latest failed part
  `cancelled`, the older attempt still `failed`, the plan `abandoned`.

### T4.2 `drive_parts`' retry rows (D6)

At the top of a round, after the planner read: unless some latest part is `exhausted`, every latest
part `failed | cancelled | superseded` with `may_attempt(attempt + 1, node.retry_limit)` gets
`store.create_step(NewRunStep { id: StepId::new(), attempt: part.attempt + 1, .. })` copying
`run_id`, `position`, `fanout_index 0`, `phase_name`, `agent_id`, `model`, `role Part`,
`parent_step_id`, `node_key` (§1.8); then re-read `steps`. The retry prepares from the same
`base_refs` slot. No note (the failed attempt carries `fail_part`'s).

### T4.3 `settle_failed`'s planner branch (D6)

At the top of `settle_failed` (`:3732`), after `node_at`:
- `step.role == StepRole::Planner && step.status == StepStatus::Done` →
  `return Box::pin(self.settle_exhausted_part(run, snapshot, &node, step)).await;`
- `step.role == StepRole::Planner` (a `failed` planner: an admission refusal whose
  `block_and_fail` a crash cut off) skips the `fails` and retry branches and ends on
  `RetryBudgetSpent` (the D131 rule: the row carries no failure).

`settle_exhausted_part`: re-read `steps`; the first `latest_parts(steps, planner.id)` row in
position order that is `exhausted(part, node.retry_limit)` (none → `Ok(None)`: rows moved);
`edge = Graph::of(snapshot).loop_edge(divided_name(node), EdgeKind::Fails).cloned()`;
`Some(edge)` → `Box::pin(self.loop_over(run.id, snapshot, planner, &edge)).await` (the planner row
is `edge_loop`'s source: its `phase_name`/`position` build the region, `gate.rs:1078`);
`None` → `failure = PartExhausted { node: divided_name(node), part: part_key(node_key), attempts:
part.attempt }`, `finish_run(Failed, Some(&failure.to_string()))`, `cleanup_walk`, `Ok(Some(Rest {
run: Failed, position: Some(planner.position), failure: Some(failure) }))`. The planner is never
re-admitted here.

### T4.4 Recovery per role (D14)

- `recover.rs`: `StepKind::Part` (§1.7) — `of` answers it first when the row is a part; `classify`'s
  `captured` match (`:253-259`) gains `StepKind::Part` beside `Plain | Judge` (the plain artefact
  test). Unit test `a_part_is_classified_by_the_plain_artefact_test`.
- `engine.rs` `recover_run` (`:3343-3396`), in the `running` loop, before `node_at`:
  `StepRole::Stage → continue`; `StepRole::Planner` → `let node = Self::node_at(..)?;
  Box::pin(self.admit_parts(run, &snapshot, &node, step)).await?.map(Self::landed)` fed into the
  loop's existing `None | Some(Finished) | Some(next)` handling (no new row, no `interrupt_step`,
  no retry spent: admission never ran, D14).
- `recover_step` (`:3465-3549`), new arms **before** the `(_, …)` arms (they would otherwise catch a
  part silently, §6 G-6):
  - `(StepKind::Part, Adjudication::Finished { .. })` → `finish_recovered(step, None, None, now)`,
    `move_step(running -> done)`, `Ok(None)` (no `land_recovered`, no `gate::apply`);
  - `(StepKind::Part, Adjudication::Reset)` → `reset_part(run, phase, step, &trees)`:
    `isolator.reset`; a refusal or error is `never_reset`'s park (as `reset_interrupted`,
    `:3629-3650`); otherwise `interrupt_step(step.id, INTERRUPTED, now)` and `Ok(None)` — the next
    round's retry row is T4.2's, not an `admit`;
  - `(StepKind::Part, Adjudication::NeverReset { .. })` → `never_reset(run, phase, step, &trees,
    &[], DIRTY_AT_START)` (the park).
- `recover::frontiers` unchanged (D14): the `done` planner is a frontier; under `worktree`/`copy`
  it reconciles zero trees, under `local`/`shared_serialized` the in-place rule drops it once a
  part started later (`recover.rs:421-441`, `started_after`, strict `>`). A planner has no tree,
  so either reading lands nothing (P-21).

### T4.5 Harness

`async fn crash_divided<O: Orchestrate>(orch: &O, stalled: &[Arc<Notify>]) -> RunId` —
`crash_on_all`'s shape (`:12335-12364`) with `orch.enqueue_divided(HTUI_FEAT_3)` then
`orch.claim(run)` as the walk. Recovery reads through `orch.restarted()` and `adopted_next`
(`:12382`); a walk that ended `Err` released its lease, and `adopt_runs` adopts a `running` run
whose lease is gone (`mem.rs:6499-6509`).

### T4.6 Cases

A frame `fn divided_recovery_case` after `divided_case` in `case()`; `CASES` under `// MOD-139 M2
T4 (plan D6, D11, D14): part retries, the exhausted divided step, loops over a divided node, and
recovery per role.`; the doc gains a "**Nine for MOD-139 M2 T4**" paragraph.

| # | Case | Asserts |
|---|---|---|
| 1 | `a_failed_part_retries_alone_from_the_base_refs` | `mutate`: `retry_limit = 1`; `primary_repo`; `script("implement/ui", 1, done_without_output())`. `stops_at_integrate`. Rows: `api` attempt 1 `Done` only; `ui` attempt 1 `Failed` (its note names `part `ui` of `implement` attempt 1: missing_output`), attempt 2 `Done` at the **same** position 6, same `node_key`/`phase_name`/agent; attempt 2's tree `base_ref == base_refs[repo]`; `spec_for(SessionKey { phase: "implement/ui", attempt: 2, .. })` exists and `api`'s attempt 2 does not. |
| 2 | `an_exhausted_part_fails_the_run_with_part_exhausted` | `retry_limit = 0`; `ui` attempt 1 `done_without_output()`. `claim` → `Started { rest }`: `rest.run == Failed`, `rest.position == Some(2)`, `rest.failure == Some(PartExhausted { node: "implement", part: "ui", attempts: 1 })`; `run.failure` is its `Display`; `api` `Done`; planner `Done`; no row at 3; `run_plans` one `Admitted`. |
| 3 | `an_exhausted_part_takes_the_fails_edge_and_re_plans` | `retry_limit = 0`; `add_edge(.., "implement", "plan", EdgeKind::Fails, None, Some(1))` (`:9712`; after expansion the edge stays on the integrate node, the planner is in its region); `ui` attempt 1 fails. Then: `plan` attempt 1 `Superseded`, attempt 2 `Done`; planner attempt 1 `Superseded`, attempt 2 `Done`; revision 0's `api` (5) `Superseded`, `ui` (6) `Cancelled`; `run_plans`: rev 0 `Abandoned`, rev 1 `Admitted` with `base_position 7` and `planner_step_id` = planner attempt 2; revision 1's parts at 7 and 8 named `implement/api@1`, `implement/ui@1`, both `Done`; `spec_for` holds `("implement/ui", 1)` and `("implement/ui@1", 1)` as two sessions; `stops_at_integrate`. |
| 4 | `a_crash_before_admission_re_admits_on_the_same_planner_row` | `store().set_fault(MemFault::AdmitPlan, true)`; `start_divided` → `Err(Store(Unreachable(_)))`; the planner `running`, attempt 1, no plan, no part. Fault off; `adopted_next(&orch.restarted(), run) == Next::Walk`; now one plan rev 0 `Admitted` under **that** planner row, which is `Done`; no planner attempt 2; parts `pending`. `restarted.resume(run)` → `stops_at_integrate`-shaped `Err`; parts `Done`, each with one session. |
| 5 | `a_crash_after_admission_re_plans_nothing_and_lands_no_part` | `primary_repo`; `start_divided` stops at integrate (lease released). `adopted_next(&other, run) == Next::Walk`; `other.isolator().reconciles() == [(planner.id, Vec::new())]` (the `worktree` planner frontier reconciles no tree; no part id appears); after `other.resume(run)` (stops at integrate): still one plan, no planner attempt 2, no part attempt 2. |
| 6 | `a_crashed_part_with_its_output_is_done_on_recovery` | `max_parallel_sessions 1`; `stall_after_done("implement/api", 1, None, true)`; `crash_divided` → `api` `running`, `ui` `pending`. `adopted_next == Next::Walk`; `api` `Done` with `finished_at` set and **no** `reconcile` of it; `resume` runs `ui` and stops at integrate; `api` has no attempt 2. |
| 7 | `a_crashed_part_without_output_retries_alone` | `retry_limit = 1`, `max_parallel_sessions 1`; `stall_after_done("implement/api", 1, None, false)`; crash; `adopted_next == Next::Walk`; `api` attempt 1 `Failed` with `gate_note` `interrupted`; `ui` still `pending`. `resume`: `api` attempt 2 and `ui` `Done`, stops at integrate. |
| 8 | `a_crashed_part_in_a_dirty_local_tree_parks` | parts `[("api", None)]`, `isolation = Some(Local)`, `primary_repo`; stall `api` without output; crash; mark its tree rows `dirty = true` through `upsert_step_tree(StepFence::Lease(orch.owner()), ..)` (the shape of `a_dirty_tree_is_never_reset`, `:5692`). `sweep` → one adopted `Next::Parked(Rest { run: AwaitingApproval, failure: Some(Interrupted { reset: false, .. }), .. })`; `api` `Failed` with `gate_note` `interrupted, tree not reset`; `resets() == 0`. |
| 9 | `a_local_divided_node_lands_no_part_on_recovery` | parts `[("api", None)]`, `isolation = Some(Local)`, `primary_repo`; `start_divided` stops at integrate; `adopted_next(&other, run) == Next::Walk`; `other.isolator().reconciles()` names no part, and at most the planner with no sibling (`[(planner.id, vec![])]` on the fake's frozen clock, P-21); `call_log_of` holds no `reconcile 5/…`; no part attempt 2, one plan. |

### T4.7 Pins

`cases_are_unique_and_counted`: `204` → `213`, ` + 9`, "…, and MOD-139 M2 T4's nine (a part
retried alone, the exhausted divided step, its `fails` loop and re-plan, and recovery per role)";
`tests/fake_conformance.rs`: `204` → `213`, the same clause.

### T4.8 Commits

1. `test(orch): MOD-139 M2 T4, nine retry, loop and recovery cases and the case-count pins (red)`.
2. `feat(orch): MOD-139 M2 T4, part retries and the exhausted divided step` — §T4.2, §T4.3, the
   gate arm's lookup.
3. `feat(orch): MOD-139 M2 T4, loops retire parts and abandon plans; recovery per role` — §T4.1's
   `retire_slot`, §T4.4.

### T4.9 Validate

As §T3.13 (`/tmp/mod139-m2-t4.log`), plus
`cargo test -p htui-orch --features test-support --lib -- gate:: recover:: walk:: status::`.

---

## T5 — Integration gate (serial)

Merge T3+T4 onto T1+T2. Re-run `cargo sqlx prepare --check` on a fresh `mod139_m2_prepare`
(§T1.6). Then the plan's Validation block:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod139-m2-gate.log
grep -c SIGABRT /tmp/mod139-m2-gate.log    # expect 0
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_m2_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Confirm in the gate:
- `pg_store_conformance` runs the nine M2 cases; `EXPECTED_CASES == 267 == CASES.len()`;
  `mem_store`'s pin is 267; `every_cross_referenced_test_name_exists` passes.
- htui-orch: `CASES.len() == 213` in both pins; `fake_orchestrator_passes_every_case` green.
- `admit_plan_takes_the_step_first` and `admit_plan_locks_the_step_for_update_up_front` green;
  `the_mirror_skips_part_documents_the_way_postgres_does` green.
- Linear and compete suites green, **no `.snap` drift** (`git status --short '*.snap'` empty): a
  spec with `part: None` and a session with no live sibling render byte-identically.
- `htui-agent`'s extensibility test is green (no `zeta` literal anywhere; memory
  htui-zeta-is-a-forbidden-literal).
- Sandbox: the live-Qdrant suites time out here and are owed on the host; a `pg_conformance`
  stack overflow appears only in the workspace build (memory htui-orch-test-stack-headroom) —
  grep the log for `SIGABRT` and box the offending arm.
- Then the reviewer gate (rust-reviewer, unpinned model; memory htui-opus-402-blocks-pinned-agents).

---

## 5. Per-task file sets (C34 re-checked with this blueprint's additions)

| Task | Files |
|---|---|
| T1 | `htui-core/src/model/{run,mod}.rs`; `htui-core/src/store/{traits,mod,worker,mem,conformance}.rs`; `htui-core/tests/mem_store.rs`; `htui-store/src/{pg/read,pg/write,cache/read,writer,worker}.rs`; `htui-store/.sqlx/*`; `htui-store/tests/{pg_conformance,pg_criteria,cache}.rs`; `htui-agent/src/conformance.rs`; `htui-agent/tests/recorder.rs` |
| T2 | `htui-core/src/prompt/{template,mod,render,trim,fixtures}.rs`; `htui-core/src/model/kind.rs`; `htui-core/tests/prompt_part.rs` (new); `htui-orch/src/{engine,promote}.rs` (the two literals and `command_run_exposed`'s call only); `htui/src/preview.rs` |
| T3 | `htui-orch/src/{divide (new),lib,graph,status,walk,schedule,command,engine,conformance}.rs`; `htui-orch/tests/fake_conformance.rs`; `htui-worker/src/views.rs` |
| T4 | `htui-orch/src/{engine,gate,recover,conformance}.rs`; `htui-orch/tests/fake_conformance.rs` |

**T1 ∩ T2 = ∅.** T1 touches `model/run.rs` and `model/mod.rs`, T2 `model/kind.rs`; T1 touches
`store/*`, T2 `prompt/*`; no `PromptSpec` literal, `exposed` caller or prompt type is in a T1 file,
and no T1 type (`PlanAdmission`, `PlanToAdmit`, the store helpers) is in a T2 file (`PartBlock`
takes the M1 `SnapshotPart`, already in the tree). T2 touches no `htui-store` file and no `.sqlx`.
T3 and T4 share `engine.rs`, `conformance.rs` and `fake_conformance.rs` and run in sequence; T4
adds no file T3 does not have except `gate.rs` and `recover.rs`. `htui-orch/src/fake.rs` is touched
by no task (P-10).

## 6. Risks and gotchas for implementers

- **G-1 Stack.** §1.12. Every new node future and case arm boxed; a part round under
  `buffer_unordered` holds `width` part futures, each a `candidate_live`-sized frame — box each
  `run_part` at the `map`. If a SIGABRT appears, box the rare branch first (`refuse_admission`,
  `settle_exhausted_part`).
- **G-2 `FakeIsolator` is not `GixIsolator`.** The fake accepts a slot under `local` (the real one
  refuses, `real.rs:666-668`), records no slot (assert through `base_ref`, §T3.9), never refuses a
  dirty shared checkout at `prepare`, logs `reset` without labelling, answers `base` as
  `fake:group-base:<repo>`, and its `reconcile` is the identity. A green fake case proves the
  engine's calls, not git's behaviour; M3's real-git test owns the latter.
- **G-3 Exhaustive vs non-exhaustive matches.** The compiler forces: `drive_cursor`'s `match
  cursor`, `schedule::candidate`, `status::position_of`, `RunFailure`'s `Display`,
  `SectionName::render`, `Placeholder::token`, `render_sections`, `trim.rs` `:588-594` and
  `:953-968`, `recover::classify`'s `captured` match, `StepKind` equality uses. The compiler does
  **not** force (hand edits, each listed above): `is_protected` (`_ => false`),
  `Placeholder::allowed_in` (`!matches!`), `scrubbed_inputs`, `assemble`'s pushes and
  `substitute`'s blocks, `recover_step`'s `(_, …)` arms (a part would silently land through
  `land_recovered`), `settle_failed`'s generic path (a `done` planner would be re-admitted), and
  **the two `matches!` lists**: `schedule.rs:158-161` (rule (e): `Parts` starts sessions) and
  `status.rs:433-441` (`resumable_park`: `Parts` is a resumable write). Missing either drops
  `Parts` silently: no input wait, or a parked divided run that `Unblock` never resumes.
- **G-4 The P-9 self-scan** (`store/conformance.rs` `every_cross_referenced_test_name_exists`,
  `:28821`): in that file's text (doc comments **and** string literals, every backticked span), a
  snake_case token with four or more underscores must be a fn of `conformance.rs` or `mem.rs`, and
  `<a>::<b_c_d_e_f>` is a failure. The nine case names qualify; the store helpers have at most three
  underscores; never backtick `chk_run_plan_admitted_base_refs`, `trg_run_plan_updated_at`,
  `idx_run_step_parent_step` or a lock-mode spelling with underscores.
- **G-5 Pg timestamps.** `trg_run_plan_updated_at` stamps `clock_timestamp()`, so a `Proposed`
  admission's and every abandonment's `updated_at` is later than `created_at` on Postgres and
  possibly equal on MemStore's fixed seam clock: compare `>=`. `New` admits with both columns
  `DEFAULT now()` (equal). The planner's `finished_at` is the caller's `at` on both stores.
- **G-6 `recover_step`'s catch-all arms.** Put the three `StepKind::Part` arms first (§T4.4).
- **G-7 Cache mid-pass lag.** `run_pass` commits `document` before `run_step`; between the two, a
  part's document reads `s.id IS NULL` on the mirror and may be answered as an input (accepted, as
  for fan-out losers, documented at `cache/read.rs:1100-1110`). The parity test seeds both tables
  before its pass, or it would flake on this window.
- **G-8 Seam isolation.** `resolve_divided` is `cfg(any(test, feature = "test-support"))` and is
  called only by `enqueue_divided_fake` and graph.rs tests; workspace test builds turn
  `test-support` on through dev-dependencies, so any `Engine` path that called it would lift the
  divide refusal for every crate's tests (D16). `StartRun` keeps refusing (case 18).
- **G-9 D13's deliberate change and its timing.** `fan_out_only` steps of parallel branches gain
  `command_run` while a sibling is live, and compete keeps `fan_out > 1`. In case 15 both
  branches' rows exist before either computes its prompt because `run_to_rest` admits both
  `Create`s in one pass and `FuturesUnordered`'s FIFO ready queue polls the second `Create` before
  the first branch's `Run` (`engine.rs:3846-3900`); this ordering is read from the code, not run.
  If the case flakes on that order, take it to the coordinator rather than weakening it to one
  branch. A serial part round (`max_parallel_sessions 1`, `shared_serialized`) gives the **last**
  part no live sibling: correct by D13's definition.
- **G-10 Integrate stops every M2 divided case.** `claim`/`resume` answer `Err(Resolve(..Divide))`
  with the run `running` and its lease released (F-2). Assert through `stops_at_integrate`; do not
  expect `Done`.
- **G-11 Test-support everywhere.** `-p htui-core` tests need `--features test-support` (or they
  run zero, memory htui-integration-tests-need-testkit); htui-store needs `--all-features`; put
  `--no-fail-fast` **before** `--`, `--test-threads=1` after (the keyring fake is process-wide).
- **G-12 `.sqlx` and the compose DB.** The compose `htui` database is empty; prepare only against
  the migrated scratch `mod139_m2_prepare` (§T1.6). Byte-identical SQL reuses an entry: keep S2
  and W2 exactly as `insert_run_plan`'s and `create_step`'s text.
- **G-13 `MemStore` has no rollback.** `State::admit_plan` decides all nine refusals on reads, then
  writes; a refusal after a write leaves a half-admitted plan no Postgres run can produce.
- **G-14 Argument counts.** `drive_once` and `session` already carry
  `#[allow(clippy::too_many_arguments)]` (`engine.rs:7746`, `:7492`); `phase_spec` and
  `assemble_prompt` reach seven parameters with `live_siblings` (and `part`), clippy's default
  limit. If the featureless clippy flags one, add the same `allow` with a reason, as those two do.
- **G-15 The fake clock is frozen.** `TestClock` moves only on `advance`/`set`, so every instant
  a walk stamps between two `advance`s is equal: a part's `started_at` equals the planner's
  `finished_at`, and strict-`>` rules (`recover::frontiers`' in-place drop) read "not later" (P-21).
  `>=` rules (`proves`) read "later". Assert recovery shapes with that in mind, or `advance` the
  clock where the case has a hook (`advance_after_done`).
