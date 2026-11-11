# Plan: MOD-139 M2 — Planner admission and parts

**Source PRD**: `.claude/prds/mod-139-divided-fan-out.prd.md`
**Selected Milestone**: 2 — Planner admission and parts
**Complexity**: Large

## Summary
The engine learns to run the first half of a divided node. When the planner node `<name>:plan` becomes
ready, the engine admits the node's default parts as plan revision 0 in one fenced store transaction
(plan `admitted` with `base_refs`, part rows `pending`, planner `done`). Each part then runs in its own
tree branched from `base_refs`, with its own prompt (an internal part section) and its own agent. A failed
part retries alone; an exhausted part fails the divided step. Part documents never reach generic inputs,
and `command_run`'s `fan_out_only` follows live siblings. Every engine path that maps a `run_step` row to a
node does it by role, and recovery handles planner and part rows. Integrate is M3: in M2 the integrate
node becomes ready once every part is done and stops there with the existing divide refusal, which no real
run can reach, because resolve still refuses divided graphs (PRD M2; lifted in M3). Resolve also gains
PRD Q1's refusal of a configured planner agent.

Grounding: ANA-33 §7.3, §7.4, §7.6 (admission paragraph), §7.7, §7.8; M1 (`0028`, `run_plans`,
`insert_run_plan`, `StepRole`); MOD-138's walker (`walk.rs`, `schedule.rs`, `run_to_rest`). The fact-check
(§3.5, below) amended D1, D4, D5, D6, D7, D8, D9, D11, D12, D13, D14, D15, D16 and added D17.

## Decisions

**D1 — Rows map to nodes by role, through one helper.** New `htui-orch` helper `node_of(snapshot, steps,
step) -> Option<&SnapshotPhase>`: `step`/`planner`/`integrate` rows map by `position` as today; a `part`
row maps to the planner node, found through its `parent_step_id` row's position; a `stage` row maps to
none. It replaces `phase_at(step.position)` wherever a row the engine did not just create is mapped:
`recover_run` (`engine.rs:3366`), `land_recovered` (`:3601`), `settle_failed` (`:3738`), `drive_cursor`
`Run` (`:4050`), `walk_live_step`'s retry landing (`:5037`). Of the five operator verbs dispatched at
`engine.rs:1155-1175`, `answer_gate`, `retry_step`, `promote` and `accept_artifact` refuse a part or stage
row with a typed `EngineError` ("a part is driven by its divided node") instead of `EngineError::Snapshot`;
`select_fanout` takes an operator-supplied position, not a row, and refuses a position that is not a
compete node as it does today. `decide`/`loop_back` are walker-driven at node positions and need no
change. Degrading sites that need only a skip or an arm: `htui-worker/src/views.rs` (`verdicts` `:369`,
`selectable_slots` `:454`; skip part and stage rows so the operator does not see "no phase at position N"),
`cancel_unsettled` (`engine.rs:4150-4157`), `schedule::candidate` (`:185-219`) and `status::position_of`
(`:487-502`, an arm for D4's cursor), `htui/src/agent_worker.rs:2699` (already degrades). Readiness
(`walk.rs`) and `status.rs` read rows per node position with `fanout_index == 0`, so part rows (past the
frozen list) and stage rows (negative) are invisible to them (verified C3); D3 adds what they must see.

**D2 — The planner node runs admission, not a session.** `Cursor::Create` creates the planner row
(`role = planner`, `node_key = <name>:plan`); `Cursor::Run(planner)` moves it `pending -> running` (the
existing CAS; `pending -> done` is illegal, `run.rs:194-219`) and calls `admit_default_parts`:
1. reads the node's `divide.default_parts` (`SnapshotDivide`, `run.rs:1161`);
2. validates them (D8 caps, D7 isolation, D10 owned paths and part persona, an empty list is the planner
   failure `empty_plan`, ANA §7.6);
3. picks each part's agent (D5) before any write, as `admit_indices` does (`engine.rs:4525-4570`);
4. reads `base_refs` = `Isolator::base(&run.repo_scope)` (`isolate.rs:190`, `BTreeMap<RepoId, String>`,
   serialised to the JSONB);
5. calls the new store verb `admit_plan` (D9) with revision = the run's highest revision + 1 (0 on a first
   admission), `base_position` = one past the highest position any frozen node or earlier plan uses, and
   one `NewRunStep` per part.
A validation failure fails the planner step and so the divided step (D6) with a typed `RunFailure`. A
planner *agent* is refused at resolve (D17), so M2 never opens a planner session.

**D3 — The divided node's state is derived from its parts.** In `walk::node_state` (`walk.rs:66-100`), a
planner node's state is the planner row's own state while it is not `done`; once `done`, it is `Done` when
every part of the latest planner attempt (rows whose `parent_step_id` is that row) has a `done` latest
attempt, `Failed` when a part's latest attempt is `failed`/`cancelled` with no attempt left (`may_attempt`,
`status.rs:30`, against the node's `retry_limit`), otherwise in flight. The integrate node's only in-edge is
the synthetic `planner -> name` edge (`expand_divide`, verified C8), so it becomes ready exactly when every
part is done. Rows only, no `run_plans` read in the walker: part rows exist only through admission (D9). A
`Failed` rest carries the **planner row**, never a part row (D6).

**D4 — Parts are driven by one node future.** New `Cursor::Parts { position }` for a planner node whose
planner row is `done` and whose parts are not all settled. It joins every exhaustive match (`drive_cursor`
`engine.rs:4026`, `resting` `:8182`, `schedule::candidate` `:196`, `status::position_of` `:488`) and the
two `matches!` lists that would otherwise drop it silently (`schedule.rs:158-161` input waits — it starts
sessions; `status.rs:433-441` `resumable_park` — it is a resumable write); its flight is `touches = true`,
`in_place` by isolation (`schedule.rs:47` doc). `drive_parts`, boxed per the stack rule (ANA §9), creates
any missing retry rows (D6) and runs the pending parts with `buffer_unordered(width)`, `width = min(live
parts, max_parallel_sessions)` (1 under `shared_serialized`); `schedule::candidate` weighs the cursor at
`width`, so the run's session cap sees every part. Each part runs through `run_part`, modelled on
`run_candidate` (`engine.rs:5570-5623`: the `pending -> running` move, best-effort tree release,
the failure path) plus `candidate_live` (`:5647-5834`), with these differences:
- per-part stage 3: the part's own prompt (D10) is assembled and recorded per part (`drive_group` builds
  one prompt per group, `:5311`, `:5715-5723`); the pre-session refusals (`MissingInput`, `PromptRefused`,
  `ScrubRefused`, `SecretsRefused`, no persona) are deterministic, so they fail the divided step with the
  same `RunFailure` a plain step gets, not a part retry;
- `prepare` with `FanoutSlot { index: i, width: parts, base: &base_refs }`, so the tree branches from
  `base_refs` (`start_of`, `real.rs:441`); under `local` (one part only, D7) `slot: None`, since
  `GixIsolator` refuses any slot under `local` (`real.rs:666-668`; the `FakeIsolator` does not, so an engine
  case asserts the `None`);
- its own session: the part row's `phase_name` is `<divided>/<part_key>` for revision 0 and
  `<divided>/<part_key>@<revision>` for later revisions, mirroring the judge's `<phase>:judge` rows, so
  `SessionKey::of` (`engine.rs:253-274`) separates parts across revisions with no key change (the fake
  harness refuses a second start per key, `fake.rs:1790-1794`);
- no gate, no `verify_command` (`candidate_live`'s verify stage `:5761-5772` is dropped), no landing: a
  part never reconciles into the primary tree (ANA §7.4). It is `done` when its session ends well and the
  plain-step artefact test holds (output document written, capture landed). `verify_command` gates
  integrate in M3 (ANA §7.5);
- a prepare refusal (`IsolateError::Refused`, e.g. the copy disk cap that `width` drives, `copy.rs:175`)
  is deterministic and fails the divided step, not a part retry;
- the failure note names a part, not a "fan-out candidate" (`fail_candidate`, `:5843`).
Under `shared_serialized`, each part resets the primary checkout to `base_refs` before it starts
(`reset_to_slot_base`, `real.rs:461-489`), and `drive_parts` resets it to `base_refs` once more after the
last part, so no later node starts from a part's commit (parts never reconcile; the part commits stay on
`htui/<step>`).

**D5 — Each part's agent; no per-part persona yet.** The part's `agent_id` (and `model`) when that agent
is among the node's `candidates`, else `AgentSelector::select` over `walk_candidates(node.candidates)`
(`FirstCandidate` today, `engine.rs:154-190`). An `agent_id` not among the candidates fails admission (D2
step 2), never a silent fallback. ANA's "frozen `bounds.agents`" does not exist (`SnapshotSettings`,
`run.rs:979-1005`), so the node's candidates are the bound. **Per-part personas are deferred**: freezing
them needs `ResolvedPhase`, both stores' `resolve_graph`, the `GraphSource` impls and a frozen name on
`SnapshotPart` (fact-check C11), which is MOD-140's planner surface. In M2 every part runs under the node's
persona, and a default part whose `persona_id` is set fails admission (`part_persona_unsupported`) rather
than being silently ignored.

**D6 — Retry per part, failure per divided step.** A failed part (session error, missing output, capture
failure) moves `running -> failed` and never fails its siblings. `drive_parts` then creates the part's
`attempt + 1` row at the same `position` (same parent, `node_key`, agent) while `may_attempt(attempt + 1,
node.retry_limit)`; the new attempt branches from `base_refs` again. This is ANA §7.4's order — parts
retry against `retry_limit` first, and only an exhausted part reaches the node's `fails` edge — which
differs on purpose from a plain node, where a `fails` edge replaces the retry. When a part has no attempt
left the divided step fails: the cursor rests `Failed` on the planner row, and `settle_failed`
(`engine.rs:3732-3760`) and the gate's `Settle::Failed` arm (`gate.rs:449-452`) gain a planner branch that
looks the `fails` edge up on the **integrate** node's name (every authored out-edge stays there after
`expand_divide`, `graph.rs:935-943`; `loop_edge("<name>:plan", Fails)` finds nothing) and passes the
planner row as `edge_loop`'s source (`gate.rs:1070-1083` builds the region from `source.phase_name` and
`source.position`). With no `fails` edge the run fails with `RunFailure::PartExhausted { node, part,
attempts }`; the planner node is never re-admitted by `settle_failed`'s retry branch. A missing part is
never routed around (PRD metric).

**D7 — Isolation per part.** `worktree`/`copy`: one tree per part. `shared_serialized`: width 1, reset to
`base_refs` around each part (D4); a dirty checkout refuses as today. `local` with more than one part fails
admission (`local_cannot_divide`); one part under `local` runs in place with `slot: None` (D4). M3's resolve
lift adds the same check at resolve for default parts.

**D8 — Caps at every admission, and resolve counts parts.** Admission refuses when `parts > max_parts`
(the node's, when set) or `parts > bounds.max_fan_out`, or when the sessions admitted so far in the run
plus the new parts exceed `bounds.max_agents_per_run` (ANA §7.8: distinct planned sessions, not retries).
The refusal fails the planner step with `RunFailure::PlanOverCaps`. `check_fan_out_caps`
(`graph.rs:1428-1458`) is corrected for divide nodes: a planner node with no planner agent counts 0
sessions (today its copied `fan_out`, always 1, since a divide node cannot be compete), the integrate node
1, and the node's default parts their count. In M2 only the D16 seam reaches it (production resolve refuses
at `not_yet_runnable`, `graph.rs:587`, before `check_fan_out_caps`, `:623`). Pinned by unit tests.

**D9 — One store verb for admission, one for abandonment.** `WriteStore::admit_plan(fence: StepFence,
admission: PlanAdmission) -> Result<RunPlan>`, also on `WorkerStore`:
```
PlanAdmission { plan: PlanToAdmit, base_refs: Value, parts: Vec<NewRunStep> }
PlanToAdmit { New(NewRunPlan) /* the engine's revision, inserted admitted */, Proposed(RunPlanId) }
```
One transaction, refusals before any write, in order: shape (`run_plan_refusal`; `run_step_refusal` per
part; every part `role = part`, `parent_step_id` = the planner, `run_id` = its run; `base_refs` a non-empty
object), `NotFound` run, `Fenced { step: planner }`, planner not in run, planner not `running`, a
`Proposed` plan that is not `proposed` or not this planner's, an `admitted` plan already on this planner
row, slot taken (the `(run_id, position, attempt, fanout_index)` tuple named), plan id or revision taken;
then the plan is inserted `admitted` with `base_refs` in one statement (`New`; 0028's
`chk_run_plan_admitted_base_refs` forbids two steps) or updated `proposed -> admitted` (`Proposed`), every
part row is inserted `pending`, and the planner moves `running -> done`. Pg locks the planner with
`step_fence(StepLock::Update)` (`FOR NO KEY UPDATE OF s FOR SHARE OF r`, `pass_step`'s shape; the parts'
FK key-share on the planner is compatible) and is pinned by both `assert_takes_the_step_first` and
`assert_locks_the_step_for_update_up_front` (`pg_criteria.rs:7498`, `:7532`). MemStore validates every row
before mutating (`State` has no rollback, `mem.rs:1482`). Timestamps: Pg's `trg_run_plan_updated_at`
stamps `updated_at`, so conformance compares plan timestamps the way M1's helper does, never exactly
across stores. New pure `PlanStatus::can_move_to` (`proposed -> admitted | refused | abandoned`, `admitted
-> abandoned`, others terminal), used by both stores. `abandon_plans(fence, planner_step) -> Result<u32>`
moves that planner row's `admitted` and `proposed` plans to `abandoned`, same lock shape; D11 uses it.
`PlanToAdmit::Proposed` is what PRD Q1 keeps general for MOD-140; M2 exercises it only through a direct
`insert_run_plan` in conformance. Forwarders: `Writer`, the two `htui-store` `WorkerStore` impls,
MemStore's `WorkerStore`, `UsageSpy`, `SpyStore` (verified complete, C23); `Backend` and `CacheStore` have
no write side. `WorkerStore`'s module doc count is corrected to the true number (68 today, 70 after).

**D10 — The part section.** A new internal prompt section, rendered for part sessions only and never
placeable in a template, like persona and memory (`Placeholder`, `template.rs:76`; internal ones are not in
`ALL`): key, title, brief, owned paths, acceptance. Internal, not a `{{part}}` token, so every divided
template gets the part brief without an edit and no template can drop it. `PromptSpec` gains `part:
Option<PartBlock>`: the seven full literals set `None` (`prompt/fixtures.rs` ×4, `engine.rs:6411`,
`:7272`, `htui/src/preview.rs:297`), and `promote.rs`'s `..phase` spread clears it as it clears persona and
memory. Placement: after persona and memory, before the template body, recorded as the next section in
`sections[]` (a new record-order contract, pinned by a test). Hand edits the compiler will not force:
`is_protected` (`mod.rs:378`, the section is protected, so an oversized brief refuses the part prompt like
`phase_protected_too_big` rather than being trimmed), `Placeholder::allowed_in` (`template.rs:210-220`),
`scrubbed_inputs` (`mod.rs:968`, the brief and acceptance are masked), and the explicit pushes in
`assemble` and `substitute` (like `mod.rs:556`, `:744`). `owned_paths` are validated at admission as
`repo:glob` with a repo-relative glob, so no absolute path reaches a digested byte (`render.rs:995`). No
edit-help entry (internal sections have none). `PromptSpec`'s rule 8 doc (`mod.rs:72-79`), `mod.rs:247` and
`render.rs:746` are narrowed to compete candidates.

**D11 — Loops retire parts and plans.** When `edge_loop` retires a region (`gate.rs:1041`, `retire_slot`
`:1317-1361`) that holds a planner node, it also retires that planner's part rows with `retire_slot`'s own
rule — `pending`/`awaiting_approval`/`done` superseded, `running`/`stopped`/`failed` cancelled (`failed ->
superseded` is illegal, `run.rs:217`) — for the latest attempt at each part position (older failed attempts
stay `failed`, which latest-attempt readers ignore), and calls `abandon_plans`. The planner node then
re-runs at `attempt + 1` and admits the next revision (D2) at fresh positions, with revision-suffixed part
`phase_name`s (D4). ANA §7.3.

**D12 — Part documents leave generic inputs.** `resolve_inputs` skips any document produced by a `role =
part` step, on all three implementations (verified complete, C27): MemStore (`State.resolve_input`,
`mem.rs:6169`, inside the existing `is_none_or` closure), PgStore (`pg/read.rs:844-930`: the join condition
becomes `(s.id IS NULL OR (s.selected IS NOT FALSE AND s.role <> 'part'))` — a bare `AND s.role <> 'part'`
would drop hand-written documents, whose `s.role` is NULL; `.sqlx` regenerated on a migrated scratch DB),
CacheStore (`cache/read.rs:1111-1179`, the same disjunction in its runtime query). The cache's mid-pass
lag (a part document whose step row has not arrived yet passes `s.id IS NULL`) is accepted, as it already is
for fan-out losers, and added to the doc paragraph at `cache/read.rs:1100-1110`; the parity test seeds both
tables first. Parts still write the divided node's `output_kind` through `write_step_document` unchanged.
`documents_of_kinds` and `upstream_summaries` are not input resolution and stay as they are. M3's
`{{parts}}` reads part documents by `parent_step_id`.

**D13 — `fan_out_only` follows live siblings.** `CommandQueue::exposed` (`kind.rs:61`) and its
one-resolver wrapper `command_queue_exposed` (`kind.rs:73`) gain `live_siblings: bool`: `FanOutOnly =>
fan_out > 1 || live_siblings || heavy tag`. Keeping `fan_out > 1` leaves compete exactly as today (its
siblings' prompts stay byte-identical whatever the timing). `live_siblings` = another row of the run, not a
stage or judge row, `pending` or `running`, other than this step — so, as ANA §7.4 says, a `fan_out_only`
step of a **parallel** branch gains `command_run` while another branch is live; that change is deliberate.
It is computed once per session before the prompt is assembled and threaded into both `phase_spec`
(`engine.rs:7158ff`, section) and `drive_once` (`~:7800`, `ToolExposure`), so the two cannot disagree
(`drive_once` has no step list in scope today).

**D14 — Recovery per role.** `recover_run` dispatches on the row's role before any node lookup:
- planner `running`: admission never ran (it moves the planner to `done` in its own transaction), so
  recovery re-runs admission on the same `running` row — deterministic, no new row, no retry spent; no
  `running -> stopped` move and no `interrupt_step` (which would spend the node's retry budget and park a
  `retry_limit 0` run);
- part `running`: a new `StepKind::Part` (`recover.rs:186-204`) applies the plain artefact test
  (`recover::classify`): finished → finished without `land_recovered`/`gate::apply` (parts have no gate or
  landing, and `land_recovered`'s `Landing::Retry` would hit `phase_at` on a part position); resettable
  tree → `interrupt_step` and the part's own retry (D6); a dirty `local`/`shared_serialized` tree → park.
  Siblings are untouched;
- stage rows are skipped.
`recover::frontiers` needs no change: after admission the `done` planner row is a `winner_at` frontier,
which under `worktree`/`copy` reconciles zero trees (harmless, `real.rs:1598-1626`) and under
`local`/`shared_serialized` is dropped by the landing rule once a part started; a conformance case pins
both shapes.

**D15 — Integrate waits for M3.** `drive_cursor` on an integrate node returns
`EngineError::Resolve(ResolveError::NotYetRunnable { node: <integrate name>, reason: NotRunnable::Divide
})`, the existing sentence ("a `divide` node runs once divided fan-out lands (MOD-139)"), no new type.
Unreachable in production; the M2 cases assert the walker offers integrate as the next cursor and stop.

**D16 — Tests reach divided runs through a seam, not through `Engine`.** `graph.rs` gains a `pub` resolve
entry under `cfg(any(test, feature = "test-support"))` that runs the real resolve (expansion, caps) minus
**only** the `NotRunnable::Divide` branch — it keeps the palette refusal (`graph.rs:724-726`),
`local_concurrent_refusal` (`:596`) and D17's refusal. Engine cases build the snapshot through it, create
the run directly on the `MemStore`, then claim it. No `Engine` path is gated on `test-support`: dev-
dependencies turn that feature on in workspace test builds, which would silently lift the refusal for every
crate's tests. M3 deletes the seam with the refusal.

**D17 — PRD Q1's refusal, which does not exist yet (fact-check C14).** `NotRunnable::PlannerAgent`
("a divided node's planner agent runs from MOD-140 on"), checked in `not_yet_runnable` **before** the
divide branch (else it is unobservable in M2) for a planner-role node whose `divide.planner.agent_id` is
set, naming the divide node (strip `PLAN_SUFFIX` as `:719` does). Pinned by a `graph.rs` unit test beside
`a_divide_node_is_expanded_then_refused` and a seam test. Nothing outside `graph.rs` matches `NotRunnable::`.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Fan-out rows | `htui-orch/src/engine.rs:4525-4570` `admit_indices` | select every agent before any write |
| Group driving | `engine.rs:5260-5411` `drive_group`, `:5570-5623` `run_candidate`, `:5647-5834` `candidate_live` | one boxed node future, per-slot prepare with a base |
| Failed sibling | `engine.rs:5843` `fail_candidate` | `running -> failed`, never fails siblings |
| Retry budget | `gate.rs:808-838` `retry_or_fail`, `status.rs:30` `may_attempt` | `attempt + 1` within `retry_limit` |
| Retirement | `gate.rs:1317-1361` `retire_slot` | supersede pending/done, cancel running/failed |
| Fenced one-tx write | `pg/write.rs:8244` `pass_step` (lock modes), `:8287` `park_step` (refusals before writes) | `step_fence(StepLock::Update)`, step then run |
| Lock-order pins | `htui-store/tests/pg_criteria.rs:7498`, `:7532` | `assert_takes_the_step_first`, `assert_locks_the_step_for_update_up_front` |
| Shared refusals | `store/traits.rs:2803-2887` `run_step_refusal`, `run_plan_refusal`, `planner_not_in_run` | pure helpers, one sentence on both stores |
| Conformance | `store/conformance.rs` `CASES` (258), boxed `run_case` arms; `htui-orch/src/conformance.rs` `CASES` (186) | pins in `mem_store.rs`, `pg_conformance.rs`, `fake_conformance.rs`, `cases_are_unique_and_counted` |
| Internal section | `htui-core/src/prompt/template.rs:76` `Placeholder` (persona, memory); `mod.rs:556`, `:744` | rendered by `assemble`, never placeable |
| Recovery kinds | `htui-orch/src/recover.rs:186-204` `StepKind::of` | one kind per row shape |
| Errors | `RunFailure` (`htui-orch/src/status.rs:42-145`, Display `:159-210`, stored as text through `finish_run`); `EngineError` (`command.rs:344ff`); `NotRunnable` (`graph.rs:310-326`) | typed variants, one sentence each |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/run.rs` | UPDATE | `PlanAdmission`, `PlanToAdmit`, `PlanStatus::can_move_to` |
| `crates/htui-core/src/store/{traits,worker,mem,conformance}.rs` | UPDATE | `admit_plan`, `abandon_plans`, refusal helpers, MemStore bodies, resolve exclusion, cases |
| `crates/htui-store/src/pg/{read,write}.rs`, `src/cache/read.rs`, `.sqlx` | UPDATE | Pg verbs, resolve exclusion on Pg and the cache |
| `crates/htui-store/src/{writer,worker}.rs` | UPDATE | forwarders |
| `crates/htui-agent/src/conformance.rs`, `tests/recorder.rs` | UPDATE | `UsageSpy`, `SpyStore` forwarders |
| `crates/htui-core/tests/mem_store.rs`, `crates/htui-store/tests/{pg_conformance,pg_criteria,cache}.rs` | UPDATE | case-count pins, lock-order pins, mirror ranking |
| `crates/htui-core/src/prompt/{template,mod,render,trim,fixtures}.rs` | UPDATE | the part section (D10) |
| `crates/htui-core/src/model/kind.rs` | UPDATE | `exposed` / `command_queue_exposed` with `live_siblings` |
| `crates/htui-orch/src/{engine,walk,schedule,status,recover,gate,graph,command,conformance,promote}.rs` | UPDATE | D1-D8, D11, D13-D17; `RunFailure` variants live in `status.rs` |
| `crates/htui-orch/tests/fake_conformance.rs` | UPDATE | engine case-count pin |
| `crates/htui-worker/src/views.rs` | UPDATE | skip part and stage rows |
| `crates/htui/src/preview.rs` | UPDATE | `PromptSpec` literal |

## Tasks
### Task 1: Store — admission, abandonment, input exclusion (parallel with T2)
- **Action**: D9 and D12 end to end on MemStore, PgStore and the cache: model types and `can_move_to`; the
  two verbs on every store and forwarder; the resolve exclusion; conformance cases first (admission with
  N parts; the refusal order; a `Proposed` revision ≥ 1 admitted after a direct `insert_run_plan`; planner
  not running; slot taken; a part with another parent or run; a second admission on one planner row;
  `abandon_plans`; resolve skips a part's document but keeps a plain step's and a hand-written one); the
  two `pg_criteria` lock pins; `.sqlx` on a migrated scratch DB; the cache parity test.
- **Files**: model `run.rs`; store `traits.rs`, `worker.rs`, `mem.rs`, `conformance.rs`; htui-store
  `pg/read.rs`, `pg/write.rs`, `cache/read.rs`, `writer.rs`, `worker.rs`, `.sqlx`; htui-agent
  `conformance.rs`, `tests/recorder.rs`; tests `mem_store.rs`, `pg_conformance.rs`, `pg_criteria.rs`,
  `cache.rs`.
- **Mirror**: M1's `insert_run_plan` on both stores; `pass_step`'s lock modes.
- **Validate**: `cargo test -p htui-core --features test-support --lib store:: --test mem_store`;
  `cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria --test cache
  --no-fail-fast -- --test-threads=1`; `sqlx prepare --check`.

### Task 2: Prompt part section and `fan_out_only` (parallel with T1)
- **Action**: D10 and the pure half of D13: the internal section, `PartBlock`, renderer, protected trim
  placement, masking, record order and its pin, the seven `PromptSpec` literals (`part: None`), promote's
  clear, the narrowed rule-8 docs; `exposed` and `command_queue_exposed` with `live_siblings`, unit tests,
  every caller passing `false` (T3 wires the real fact).
- **Files**: htui-core `prompt/{template,mod,render,trim,fixtures}.rs`, `model/kind.rs`; htui-orch
  `engine.rs` (the two literals and `command_run_exposed`'s call only), `promote.rs`; htui `src/preview.rs`.
- **Mirror**: the persona and memory sections; `render.rs` `command_queue()`.
- **Validate**: `cargo test -p htui-core --features test-support --lib prompt:: model::kind`;
  `cargo test -p htui-orch --features test-support --lib`; clippy both ways.

### Task 3: Engine — rows by role, admission, running parts (serial, after T1 and T2)
- **Action**: D1, D2, D3, D4, D5, D7, D8, D13's wiring, D15, D16, D17, test-first in
  `htui-orch/src/conformance.rs` and `graph.rs` unit tests: resolve refuses a divided graph and, first, a
  configured planner agent; a two-part divided node admits revision 0 with `base_refs` and runs both parts
  from them, each with its own branch, part section and agent; part documents are not the next node's
  inputs; integrate is the next cursor only when both parts are done; `max_parallel_sessions` 1 runs the
  parts one at a time; `shared_serialized` resets to `base_refs` around each part; a single `local` part
  prepares with no slot; an agent outside the candidates, a part persona, an over-cap plan and `local` with
  two parts each fail admission with their own failure; `command_run` is exposed to a part with a live
  sibling and not to a lone plain step; the four row-taking operator verbs refuse a part row; stage rows
  are ignored by the walk; the corrected cap count.
- **Files**: htui-orch `engine.rs`, `walk.rs`, `schedule.rs`, `status.rs`, `graph.rs`, `command.rs`,
  `conformance.rs`, `tests/fake_conformance.rs`; htui-worker `src/views.rs`.
- **Validate**: `cargo test -p htui-orch --features test-support --no-fail-fast -- --test-threads=1`, then
  grep the output for SIGABRT.

### Task 4: Engine — retries, failure, loops and recovery (serial, after T3)
- **Action**: D6, D11, D14, test-first: a failed part retries alone from `base_refs` and its sibling is
  untouched; an exhausted part fails the divided step, follows the divided node's `fails` edge (found on the
  integrate node) with the planner row as source, else fails the run with `PartExhausted`; a loop over the
  divided node retires its parts by `retire_slot`'s rule, abandons its plan and admits revision 1 with
  distinct session keys; crash with the planner `running` before admission, after admission, mid-part
  (resettable and dirty-local) each resume with no twin session and no second revision; the frontier
  shapes after admission under `worktree` and `local`.
- **Files**: htui-orch `engine.rs`, `gate.rs`, `recover.rs`, `walk.rs`, `conformance.rs`,
  `tests/fake_conformance.rs`.
- **Validate**: as T3.

### Task 5: Integration gate (serial)
- **Action**: the workspace gate on the merged tree (below), then the reviewer gate.

T1 and T2 file sets are disjoint (verified C34; with per-part personas deferred, T2 alone touches
`model/kind.rs`). T3 and T4 share `engine.rs`, `walk.rs`, `conformance.rs` and run in sequence.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # then grep SIGABRT
(cd crates/htui-store && DATABASE_URL="$BASE/mod139_m2_prepare" cargo sqlx prepare --check -- --all-targets --all-features)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Qdrant-live tests time out in the sandbox and are owed on the host.

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| Concurrent part futures overflow the 2 MiB debug stack | Medium | box `drive_parts` and each `run_part`; gate `--no-fail-fast`, grep SIGABRT |
| A row-to-node site missed by D1 errors on a part row | Medium | D1's list is the fact-checked call-site set; a case drives each row-taking operator verb at a part row |
| The `FakeIsolator` accepts what `GixIsolator` refuses (a slot under `local`) | Medium | an engine case asserts `slot: None` for a `local` part; a real-git isolator test in M3 |
| `resolve_inputs`'s disjunction changes ranking for hand-written documents | Medium | conformance keeps a hand-written and a plain step document beside a part's; the cache parity test |
| `live_siblings` changes parallel-branch prompts (deliberate, ANA §7.4) | Medium | compete keeps `fan_out > 1`; a case pins the parallel-branch exposure |
| `shared_serialized` parts move the primary checkout | Low | reset to `base_refs` around every part (D4) |
| Engine `CASES` grow past what one task keeps green | Low | T3 and T4 split, each with verify rounds |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes (Qdrant-live owed on the host)
- [ ] Patterns mirrored, not reinvented
- [ ] Divided graphs still refused at resolve; linear and compete runs unchanged; parallel branches gain
  `command_run` only for `fan_out_only` steps while a sibling is live (D13)

## Verified claims
Five read-only checkers, 2026-11-11, workflow `wf_5d0fefc9-5f7`; corrections folded into the decisions.

| Claim | Verdict | Evidence |
|---|---|---|
| C1 The row-to-node sites are the five named plus the operator verbs | PARTLY → D1 | `land_recovered` `engine.rs:3601` added; `decide`/`loop_back` are walker-driven; `select_fanout` takes a position; degrading sites listed |
| C2 `views.rs` degrades on a miss | TRUE | `views.rs:369` `refused_step`, `:454` filter |
| C3 Readiness never reads a part or stage row | TRUE | `walk.rs:66-100`, `status.rs:277-282` (`fanout_index == 0`), `run_steps` order `(position, attempt, fanout_index)` |
| C4 `pending -> done` illegal; `running -> done/failed/stopped`, `stopped -> running` legal | TRUE | `run.rs:194-219`; also `failed -> superseded` illegal (`:217`) → D11 |
| C5 `may_attempt` is `next <= retry_limit + 1` | TRUE | `status.rs:30-32` |
| C6 `RunFailure` takes new variants | PARTLY → files | lives in `htui-orch/src/status.rs:42-145`, Display `:159-210`, stored as text; no migration |
| C7 `EngineError` has a not-yet-runnable variant | FALSE → D15 | reuse `EngineError::Resolve(ResolveError::NotYetRunnable { .. })` |
| C8 `expand_divide` shape; integrate's only in-edge is the synthetic one | TRUE | `graph.rs:886-953`, `:914-943` |
| C9 `not_yet_runnable` refuses divide, pinned | TRUE | `graph.rs:710-728`, test `:5143-5179` |
| C10 `SnapshotDivide`/`SnapshotPart` fields; `divide_block` | TRUE | `run.rs:1161-1174`, `graph.rs:1650-1680` |
| C11 Part personas can be frozen in `divide_block` | PARTLY → D5 | needs `ResolvedPhase`, both `resolve_graph`s, `GraphSource` impls, a name on `SnapshotPart`; deferred |
| C12 `check_fan_out_caps` ignores parts; `SnapshotSettings` has no agents | PARTLY → D8 | `graph.rs:1428-1458`; `run.rs:979-1005`; only the seam reaches it in M2 |
| C13 A test-support resolve entry is reachable by engine cases | PARTLY → D16 | create the run on the MemStore; never gate `Engine` on the feature |
| C14 The planner-agent refusal exists | FALSE → D17 | `git grep MOD-140 graph.rs` finds only doc and palette text |
| C15 `admit_indices` selects before writing | TRUE | `engine.rs:4525-4570` |
| C16 `drive_group`/`candidate_live`/`fail_candidate` shapes | PARTLY → D4 | one prompt per group; group-level refusals; `run_candidate` is half the model |
| C17 `FanoutSlot`, `start_of`, `slot_base`, `reset_to_slot_base` | TRUE | `isolate.rs:103-110`, `real.rs:174-185`, `:441`, `:461`; `local` + slot refused `real.rs:666-668` → D4/D7 |
| C18 `Isolator::base` returns the per-repo HEAD map | TRUE | `isolate.rs:190`, `real.rs:1486-1494` |
| C19 `SessionKey::of` = `(phase_name, attempt, fanout_index, 0)`; `phase_name` free text | PARTLY → D4/D6 | `engine.rs:253-274`; `edge_loop` reads `source.phase_name` `gate.rs:1078` |
| C20 A new cursor fits `plan_pass` | PARTLY → D4 | exhaustive matches and two `matches!` lists named |
| C21 `AgentSelector`, `walk_candidates`, `candidate_of` | TRUE | `engine.rs:154-190`, `~4600`, `8249-8275` |
| C22 `WorkerStore` doc says 65 | PARTLY → D9 | true count is 68 |
| C23 The forwarder list is complete | TRUE | every `impl WriteStore`/`WorkerStore` in the workspace |
| C24 Lock doc, `step_fence`, `park_step`, `pass_step`, lock pins | PARTLY → D9 | lock modes are `pass_step`'s; second pin `:7532` |
| C25 MemStore mutates in place, no rollback | TRUE | `mem.rs:1482` |
| C26 `PlanStatus` has no transition law | TRUE | `run.rs:1086-1099`; 0028 has no guard |
| C27 Three `resolve_inputs` implementations | PARTLY → D12 | a bare `s.role <> 'part'` drops hand-written documents |
| C28 Case counts 258 and 186 and their pins | TRUE | `conformance.rs:92`; `mem_store.rs:35-37`; `pg_conformance.rs:55`; orch `fake_conformance.rs:17` |
| C29 `insert_run_plan` inserts `proposed` only | TRUE | `run.rs:610-631`; `traits.rs:1869-1887` |
| C30 Persona and memory are internal sections | TRUE | `template.rs:76`, `:121`, `:126`, `:131` |
| C31 `PromptSpec` literal sites | PARTLY → D10 | seven full literals; `promote.rs` spreads `..phase` |
| C32 `trim.rs` and `SectionName` force every arm | PARTLY → D10 | `is_protected`, `allowed_in`, `scrubbed_inputs` are not exhaustive |
| C33 `exposed`'s callers | PARTLY → D13 | `command_queue_exposed` `kind.rs:73`; `drive_once` has no steps in scope |
| C34 T1 and T2 file sets are disjoint | TRUE | no PromptSpec literal or `exposed` caller in T1's files, and no T1 type in T2's |
