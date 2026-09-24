# Blueprint: MOD-4 milestone 6, "the maintainer drives it"

**Status**: **accepted** (2026-09-24). The maintainer accepted findings F-A..F-S (§0) and decisions D181–D209 (§20) as written. Where a finding says **Blocker**, the plan read literally does not compile, cannot pass its own case, or leaves a state nobody can recover; the fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-4-orch-drive.plan.md`, confirmed 2026-09-24 with every OQ default. It covers D153–D180 and R-38–R-47. The fact-check's "(amended at fact-check)" notes and its Verified-claims table take precedence over the plan's original prose, and this blueprint follows them. **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 6 (`:312`). PRD D1–D8 win over this blueprint where they disagree. **Design authority**: ANA-2 §4.3, §4.8, §4.10, §6.2 and §8 (`docs/ANA-2.md:520-687`, `:1160-1240`, `:1380-1392`, `:1551-1580`, `:1681-1697`).

**Verified at**: HEAD `8b2e39f` on branch `mod-4-m6`. `git diff --stat ba68682 HEAD -- crates/ Cargo.toml Cargo.lock` is empty, so the fact-check's line numbers still hold. Every signature this blueprint prescribes against was opened through the Gortex index at HEAD. **Line numbers are pre-edit.** A citation into a file that a task edits moves after that task's first commit.

**Graphify**: `graphify-out/` is deleted in the working tree, so nothing here comes from it.

**Scope**:
- Order: T1 alone. Then Wave A (T2 ∥ T3 ∥ T5), each in its own git worktree, merged T2, then T5, then T3. Then T4. Then T6. Then Wave B (T7 ∥ T8), each in its own worktree, merged T7 then T8. Then T9.
- New modules: `htui-orch/src/closeout.rs`, `htui-orch/src/promote.rs`, `htui/src/run_worker.rs`.
- New public engine surface (T4, all in `engine.rs`/`command.rs`/`status.rs`):
  - `Engine::enqueue`, `Engine::abandoned`, `Engine::sweep_fenced` plus the `RunFence` trait;
  - `Engine::unblock_case`, `Engine::close_out_preview`;
  - `DeadWalks::mark`;
  - four `Command` variants and one admission function per verb (D184);
  - `command::snapshot_of` and `command::phase_at` made public;
  - `status::resumable_park`.
- One new recorder constructor (T2) and one new item-law edge (T1).
- **No migration, no `WriteStore`/`ReadStore` method, no `.sqlx` change.** `StoreRequest` grows 58 → 62 (D182). `htui` gains its first `htui-orch` dependency.

**House style (carried)**:
- One named free function per refusal sentence, and `Display`-exact vocabularies.
- Every instant the engine writes comes from `Clock`.
- No `std` guard is held across an `.await`.
- The only `Command::new` calls under `crates/htui-orch/src/` are in `isolate/git.rs` and `verify.rs`.
- `#![warn(missing_docs)]` in `htui-orch`.
- Implementers commit incrementally, staging their own paths only (never `-A`/`-a`, never `stash`). Every commit compiles.
- Every gate is verified with `--test-threads=1` on the real tree after the merge (project memory).

---

## 0. Findings against the plan

The fact-check missed each of these. Each is fixed by a blueprint decision.

| # | Blocker? | Plan says | Tree at `8b2e39f` | Fix |
|---|---|---|---|---|
| **F-A** | **Blocker** (layout) | D169: step line 1 is cursor (2), slot (5), status (9), phase (12), usage (7), duration (5), "single spaces between". | Those widths sum to 40, and the five separating spaces make **45 > 43**. Status 9 cannot hold `awaiting_approval` (17) or `superseded` (10). Today's pane already clips both: `step.status.as_str()` goes into `Constraint::Length(9)` (`detail/runs.rs:291-294`, `:307-313`). | D197's grid: cursor 1, slot 5, status 10 (short form `awaiting`), phase 11, usage 6, duration 5, which is exactly 43. Line 2 is indent 8, gate 10, then a 24-column tail. §9.4. |
| **F-B** | **Blocker** (the pane cannot call the guards) | D166/D168: each key is "greyed by the same guard the engine refuses with" (`command.rs:1-8`), and the pane calls `retry_enabled`, `select_enabled` and the rest. | The pane holds `RunSummary`/`RunStepSummary` only (`run.rs:718-764`, `:797-827`, `detail/runs.rs:47-57`). Those carry no `gate_note` (the A-8 exemption), no snapshot (`retry_limit`, `output_kind`, `fan_out`) and no `Run` row. The guards take `&Run`, `&RunStep` and `&SnapshotPhase` (`command.rs:405-420`, `:452-491`, `:522-565`, `:662`). | D182: a new `StoreRequest::RunActions(ItemId)`. The worker evaluates the real guards over real rows and answers `ItemActions`, and the pane greys keys from that. D184 gives one admission function per verb, called by both the engine and `RunActions`. |
| **F-C** | **Blocker** (every Backlog snapshot) | T8's pane sends `RunStream` (D172). The plan leaves a runtime-less `try_serve` refusing those requests "like the chat requests". | A `StoreReply::Failed` passes the freshness gate and is written to the status line (`app/update.rs:147-149`). The harness's `settle()` serves every request through `store_worker::serve` with no runtime (`testkit.rs:371`). So every Backlog harness test would render `run_stream: no run runtime in this build` on its status line. | D183: `try_serve` answers `RunStream` with a `Subscribed` acknowledgement and `RunActions` with no live chats, and serves `Document` as a read. Only `Orch` is refused. No existing snapshot moves because of T6/T8 plumbing. |
| **F-D** | **Blocker** (correctness, R-27) | D157: a dropped walk's "guards are released by `walk_leased`'s own path, and its lease is released (D127/D139)". | `walk_leased` releases only in its own `Left(Err)` and `Right(Abandoned\|Expired)` arms (`engine.rs:1117-1165`). A preempting drop of the **outer** command future runs neither. The `shared_serialized` guards stay held in `GixIsolator.held`, and the lease stays live under this owner until the TTL. | D188: a new `pub async fn Engine::abandoned(&self, run)` that calls `isolator.release(run)` and `release_lease(run)`, both best-effort. The worker calls it after every token-cancelled walk. |
| **F-E** | **Blocker** (R-27 left open) | D157/D158: "every sweep-driven walk of run *r* holds `RunLocks[r]`". | `Engine::sweep` releases every `DeadWalks` run, adopts, and recovers each adopted run inside one call (`engine.rs:1325-1390`). No caller can take a per-run lock between adopting and recovering. The race the `DeadWalks` doc names (`engine.rs:262-275`) survives. | D189: `pub trait RunFence` and `Engine::sweep_fenced(&F)`, with `sweep()` delegating to a no-op fence. `RunLocks` implements the fence with `try_lock_owned`. A held run is skipped in both the release pre-pass and the recovery. |
| **F-F** | **Blocker** (R-NF-3) | D165: `RunRuntime` answers `RunServed::Attach` after the engine's writes, and the loop hands it to `attach_promoted`. | A promotion of a `running` step has to cancel the walk and then **wait for the run's lock** (D157). Awaiting that inside `RunRuntime::serve` blocks the store loop for as long as the walk takes to drop. `AgentRuntime` lives on the loop and cannot be reached from a task (`store_worker.rs:1351-1359`). | D181 and D191: a promotion is always a task. Its `RunServed::Attach` reaches the loop through a runtime-owned event channel, which is one more arm in the **store worker's** `select!` (not `event_loop.rs`). §8.3. |
| **F-G** | Non-blocker (context loss) | D163: `Opening::Resume` when `DriverCaps.resume` allows (CLI `true`, "ACP from settings"). | The ACP driver never reads `SessionSpec.resume`. The only readers are `cli/mod.rs:145` and `:405`. `acp.session.resume` defaults to `true` (`htui-agent/tests/launch.rs:334`, `registry.rs:165`), so every ACP row would take the resume path and open a **fresh** session with no context. | D192: resume only when `caps.resume && transport == Cli && banner.is_some()`, otherwise handoff. Recorded as **R-48**. |
| **F-H** | Non-blocker | D166: the worker supplies `chat_live` "from `AgentRuntime::steps`, `agent_worker.rs:438`". | `steps()` is history: "Every step this runtime has started, oldest first" (`agent_worker.rs:436-441`). `caps(step)` (`:503`) answers `Some` for a chat whose task ended until the next `serve` sweeps it (`:531`). | D206: `LiveChats`, handed to `RunRuntime::serve`. T6 derives it from `caps`, and T7 adds `AgentRuntime::live_steps()`, which checks `!commands.is_closed()`. |
| **F-I** | Non-blocker | D157: "`StartRun` holds an item lock until `create_run` returns the run id". | `Engine::start_run` resolves, creates, claims and walks in one call (`engine.rs:511-560`), so no caller can take a run lock between create and claim. `create_run` already admits one run per item, because the item compare-and-set is `open\|failed → queued` (`engine.rs:540-543`). | D186: T4 factors `pub async fn Engine::enqueue`, and `start_run` becomes `enqueue` followed by `claim`. The worker takes `RunLocks[run]` between the two. There is no item lock. |
| **F-J** | Non-blocker (wording) | D162: "item → blocked, then `finish_run(Failed)` and `cleanup_run`, then an `item_note` — the order `refuse_no_candidate` uses". | `refuse_no_candidate` writes the item move, **then the note**, then `finish_run`, then `cleanup_run` (`engine.rs:2224-2240`). | D195 follows the tree: step `failed`, item `blocked`, note, `finish_run(Failed)`, `cleanup_run`. |
| **F-K** | Non-blocker | T8: `backlog__empty_runs.snap` is re-recorded. | That snapshot renders only `No runs for this item.` (`backlog__empty_runs.snap:9`). D169 does not change it. | Listed in T8's set for a re-check only. It must stay byte-identical, and a diff there is a regression. |
| **F-L** | Non-blocker | T8's test list does not mention the existing step-row tests. | `a_step_without_one_stays_one_line` (`detail/runs.rs:531-555`) pins one-line steps. D169 makes every step two lines. | T8 rewrites it as `every_step_takes_two_lines` (§9.6). |
| **F-M** | Non-blocker (UX regression) | D171/D172: re-read `Runs` on every frame and on every `Orch` reply. | `RunsTab::on_reply` resets the scroll and puts the cursor on the **first** step on every `Runs` reply (`detail/runs.rs:215-221`, `reselect` `:103-105`). After every action the cursor would jump. | D198: a `Runs` reply for the same item keeps the cursor on the same entry id. |
| **F-N** | Non-blocker | D172: the pane subscribes "when a `Runs` reply for a new item arrives". | A `Runs` reply is a bare `Vec<RunSummary>`, and for an item with no run it names no item. | The pane subscribes for its own `self.item` (D199). |
| **F-O** | Non-blocker | T6/T8 tests over `Backend::Memory` walk the demo items. | `MemStore::phase_agents` answers empty unconditionally, and the demo seeds no `agent_box` (`fake.rs:870-880`, `mem.rs:128-130`). `BackendGraphs` over the demo therefore resolves **no candidate**. | Every `htui` walk test seeds one candidate: one enabled scripted `agent` row plus one `agent_box` row for the demo box (rung 3), the way `tests/chat.rs:88-99` seeds its row (§8.9). |
| **F-P** | Non-blocker | D154: `Document(DocumentId)` is served ahead of `try_serve`. | It is a plain `ReadStore::document` read (`traits.rs:88`) and needs no runtime. | D183: `try_serve` serves it. |
| **F-Q** | Non-blocker | Plan pin: `StoreRequest` 58 → 61. | D182 adds `RunActions`. | 58 → **62**. |
| **F-R** | Non-blocker (production reach) | Criterion 21: every action is "reachable from `runs.rs::on_key`". | Production's `SessionSink` is `NoSink` (`engine.rs:193-208`), so no step ever produces an `output_kind` document outside a test. `approve` and `accept` are therefore always refused with `MissingOutputForApproval` in production until MOD-11. | They are reachable and greyed with the guard's sentence. Recorded as **R-50**. Tests reach them through D203's `StepAuthor`. |
| **F-S** | Non-blocker | D156: `IsolatorConfig` is built "from `Backend::repo_paths(box)` and the repos of the scope's projects". | An `Orch` request carries no scope, and the seam has no `repo(id)` read (`store_worker.rs:326-328`). | D202: every project of every workspace (`Backend::workspaces()` → `WriteStore::repos(project)`), joined on `repo_paths(box)`. |

### 0a. The three points a previous attempt raised, resolved against the tree

1. **`RunServed::Attach` everywhere.**
   - T6 defines `run_worker::RunServed { Reply, Deferred, Attach { addr, promoted } }`. `agent_worker::Served` is **not** touched (`agent_worker.rs:258-271`).
   - T7's Action text should read "the loop hands `RunServed::Attach` to `AgentRuntime::attach_promoted`". §10 uses only that name.
   - `Attach` reaches the loop through the runtime's event receiver (D181), in both the store loop and the harness. T6 leaves one stub, in the helper `on_run_served` in each of the two places. T7 replaces both stubs.
2. **Does a failed `Orch` reply reach the Runs pane? Yes. T8 needs no `app/*` file.** The path, followed at HEAD:
   - `App::on_reply` (`app/update.rs:139`) runs `observe_reply` and then `is_fresh`. A fresh `Failed` is written to the status line (`:147-149`).
   - The **same** reply then goes to `Origin::Tab(id)`: `tab.on_reply(&reply, &mut ctx)` (`:170-173`).
   - `BacklogTab::on_reply` hands every non-`Items` reply to `self.detail.on_reply` (`backlog/mod.rs:231-240`).
   - `DetailRegistry::on_reply` gives it to every sub-tab (`detail/mod.rs:176-180`), so `RunsTab::on_reply` sees `StoreReply::Failed { request, .. }`.
   - The request names are D209's per-verb names, which the pane matches with `run_worker::ORCH_NAMES.contains(request)`.

   Wave B stays disjoint:
   - T7 = {`agent_worker.rs`, `store_worker.rs`, `testkit.rs`, `chat/mod.rs`, `tests/chat.rs`, `chat__*.snap`}.
   - T8 = {`detail/runs.rs`, `detail/mod.rs`, `backlog/mod.rs`, `tests/backlog.rs`, `backlog__*.snap`, `replay__runs_step_selected.snap`}.
   - The intersection is ∅. Neither task edits `app/*`, `run_worker.rs` or `lib.rs`.
3. **Every `RunStream` frame carries the subscribing request's `(origin, seq)`.**
   - `RunRuntime` keeps a `Publisher` with `subs: HashMap<Origin, Subscription { seq, item }>`.
   - Serving `StoreRequest::RunStream { item }` inserts or **replaces** `subs[envelope.origin] = { envelope.seq, item }`, because an older `seq` from that origin is already stale under `App::latest` (`app/state.rs:283-296`). It is answered once, at that `seq`, with `RunFrame { kind: Subscribed }`.
   - Every later frame for `item` is sent as `ReplyEnvelope { seq: sub.seq, origin: sub.origin.clone(), reply: RunStream(frame) }`. That is the only `seq` `App::is_fresh` (`app/state.rs:304-308`) passes for `(origin, RunStream)`. It is the chat stream's rule (`agent_worker.rs:9-12`).
   - Pinned by T6's `run_stream_frames_carry_the_subscription_seq` (§8.10).

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum; each compiles) | Gate |
|---|---|---|---|
| T1 item law | htui-core | 2: (a) `SANCTIONED` row (red); (b) `can_move_to` + doc (green) | `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo build -p htui-orch --all-features`; store gate on both stores (plan T1) |
| T2 continuing recorder | htui-agent (worktree) | 2: (a) `continuing` with `todo!()` + six tests (red); (b) body + doc items 2 and 3 (green) | `cargo test -p htui-agent --all-features -- --test-threads=1`; clippy |
| T5 detached git | htui-orch (worktree) | 2: (a) dropped-merge test (red); (b) `detached` + three public verbs (green) | `cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1`; C-1 |
| T3 pure modules | htui-orch (worktree) | 2: (a) both modules with `todo!()` + `lib.rs` lines + tests (red); (b) bodies | `cargo test -p htui-orch --all-features --lib -- closeout:: promote::` |
| merge | — | T2, then T5, then T3 | after **each** merge: the `htui-agent` and `htui-orch` gates on the real tree |
| T4 commands | htui-orch | 8 (§7.10) | `cargo test -p htui-orch --all-features -- --test-threads=1`; clippy; `cargo doc -p htui-orch --no-deps --all-features` |
| T6 run worker | htui | 9 (§8.12) | `cargo test -p htui --all-features -- --test-threads=1`; workspace clippy; `cargo doc --workspace --no-deps --keep-going` (baseline two) |
| T7 chat binding | htui (worktree) | 4 (§10.6) | `htui` gate; `cargo insta test -p htui --all-features` shows exactly one new `.snap` |
| T8 Runs pane | htui (worktree) | 6 (§9.8) | `htui` gate; review every `.snap` with `cargo insta review` |
| merge | — | T7, then T8 | after **each**: the `htui` gate on the real tree |
| T9 Postgres | htui tests | 1–2 | the workspace gate (§15) |

Tests come first everywhere. A red commit must compile: unimplemented bodies are `todo!()` (`clippy::todo` is not in `clippy::all`).

---

## 2. Wave shape and the compile-coupling contract

**Disk first.** Two worktrees at a time means two `target/` directories. `df -h /` read 156 GB free (65 % used) at HEAD. Re-check before each wave (project memory: `dev-postgres-crash-is-disk-pressure.md`).

| Task | Owns | May **not** |
|---|---|---|
| T2 | `htui-agent/src/record.rs`, `htui-agent/tests/recorder.rs` | change `Recorder::new`'s signature or any existing method's; touch `htui-orch` |
| T5 | `htui-orch/src/isolate/git.rs`, `htui-orch/tests/gix_isolator.rs` | change any `pub` signature in `git.rs` (§6); touch `real.rs` |
| T3 | `htui-orch/src/closeout.rs` (new), `htui-orch/src/promote.rs` (new), `htui-orch/src/lib.rs` (the two `pub mod` lines and one doc sentence only) | name `Engine`, `Command` or any store; add a crate dependency |
| T7 | `htui/src/agent_worker.rs`, `htui/src/store_worker.rs`, `htui/src/testkit.rs`, `htui/src/ui/tabs/chat/mod.rs`, `htui/tests/chat.rs`, `htui/tests/snapshots/chat__*.snap` | touch `run_worker.rs`, `app/*`, any Backlog file |
| T8 | `htui/src/ui/tabs/backlog/detail/runs.rs`, `detail/mod.rs`, `backlog/mod.rs`, `htui/tests/backlog.rs`, `htui/tests/snapshots/backlog__*.snap`, `htui/tests/snapshots/replay__runs_step_selected.snap` | touch `run_worker.rs`, `store_worker.rs`, `testkit.rs`, `app/*` |

**Build coupling, re-checked at HEAD:**
- **T1**: `Status::can_move_to` is a `const fn` (`item.rs:46`). The edge adds a match alternative, and nothing changes signature.
- **T2**: adds one associated function. `Recorder::new` (`record.rs:455`) is unchanged.
- **T3**: two new modules that nothing calls. Their imports exist at HEAD:
  - `htui_core::model::{Item, RunSummary, RunStepCommit, Repo, DocumentHead, NewDocument, SessionEvent, EventKind, Transport}`;
  - `htui_core::prompt::{PromptSpec, TemplateRole, TemplateRef, HandoffInputs, StepSummary, DiffBlock, excerpt::RepoRoot}` (`prompt/mod.rs:70-110`, `:196-222`, `render.rs:745`, `excerpt.rs:115`);
  - `htui_agent::driver::{DriverCaps, AgentSessionRef}` (`driver.rs:49`, `:325`);
  - `htui_agent::replay::envelope_from_row` (`replay.rs:73`), `htui_agent::event::SESSION_STARTED` (`event.rs:163`).
- **T5**: private bodies change behind unchanged public signatures. `Cli: Clone` (`git.rs:179`), and `merge_no_ff_reading`'s `fn` pointer argument is `'static + Send`.
- **T7/T8**: each builds on T6's public surface only (`RunRuntime`, `RunServed`, `LiveChats`, `ItemActions`, `OrchRequest`, `OrchReply`, `RunFrame`, `Action::Promote`), so each red commit compiles on its own tree.

**Hidden coupling:**
- `Cargo.lock` moves in T6 only.
- `chat__*.snap` moves in T7 only. `backlog__*.snap` and `replay__*.snap` move in T8 only.
- `CASES` moves in T4 only. `StoreRequest` moves in T6 only.
- No task touches `.sqlx`, a seed, a migration, or `RunStepSummary` and its three builders.

---

## 3. T1: `htui-core`, the escalated item follows its run back (D161, OQ-4)

**First failing test**: `the_item_status_table_sanctions_exactly_the_ana_2_pairs` (`item.rs:296`), after `(Status::Blocked, Status::AwaitingApproval)` joins `SANCTIONED` (`:262-285`).

- `item.rs:55`: `Self::Blocked => matches!(to, Self::Open | Self::AwaitingApproval | Self::Closed)`.
- The `can_move_to` doc (`:38-45`) gains one sentence: "`blocked → awaiting_approval` is MOD-4 plan D161's one deviation from `docs/ANA-2.md:583-584`. `Unblock` uses it to let an escalated item follow its parked run back (R-4)."
- The table test's doc names D161.
- Gate as in plan T1. The store transition case stays green and does **not** prove the edge (plan amendment). T4 proves it on `MemStore`, and T9 proves it on Postgres.

---

## 4. T2: `htui-agent`, a recorder that continues a step's log (D164)

**First failing test**: `a_continued_recorder_keeps_the_step_s_earlier_usage`.

### 4.1 Surface (`record.rs`, after `new` at `:455`)

```rust
/// A recorder over a step whose log already has rows (MOD-4 plan D164, ANA-5 criterion 18): a
/// promoted graph step, continued by a chat. `tail` is the step's persisted rows
/// (`ReadStore::step_events`), in any order.
///
/// - `seq` continues at `max(tail.seq) + 1` (item 2: gapless, one writer).
/// - `turn` is the highest `turn` of `tail`; `turns = turn + 1`. `record_follow_up` then opens
///   `turn + 1` (`:623-640`).
/// - `prompt_digest` and the owed digest are `None`, so every `set_step_usage` passes `None` and
///   `run_step.prompt_digest` is left as the original prompt wrote it (item 3).
/// - `usage` is seeded with `UsageTotals::from_rows(tail)`: the recorder writes the whole
///   `run_step.usage` document from its own running total (`:1071-1073`), so a continuation that
///   started from zero would erase the step's pre-promotion spend.
///
/// Never call `record_prompt` on it: that is what would rewrite the digest.
pub fn continuing(
    store: &'a S,
    scrubber: &'a dyn Scrubber,
    step: StepId,
    retain_raw: bool,
    ui: Option<mpsc::Sender<DriverEnvelope>>,
    tail: &[SessionEvent],
) -> Self;
```

The body is `Self { next_seq, turn, turns, usage, ..Self::new(store, scrubber, step, retain_raw, ui) }`, with:
- `next_seq = tail.iter().map(|r| r.seq).max().map_or(0, |m| m + 1)`;
- `turn = tail.iter().map(|r| r.turn).max().unwrap_or(0)`;
- `turns = if tail.is_empty() { 0 } else { turn + 1 }`.

`usage_dirty` stays `false`, so a continuation that reports no usage writes no usage.

The module doc (`record.rs:14-25`) gains one sentence in item 2 ("`Recorder::continuing` starts past a log's last row") and one in item 3 ("a continuing recorder owes no digest").

### 4.2 Tests

`record.rs`, unit tests over `MemStore`:

| Test | Asserts |
|---|---|
| `a_continued_recorder_starts_after_the_last_seq_and_turn` | A tail of rows at seq 0..=5 with max turn 1 gives first row `seq == 6` and `turn == 1`. |
| `a_continued_follow_up_opens_the_next_turn` | `record_follow_up` writes `kind = follow_up`, `turn = 2` and `seq = 6`, and a later chunk row is at turn 2. |
| `a_continued_recorder_never_writes_prompt_digest` | Seed `set_step_prompt(step, "d0", …)`, continue, write a usage row, `finish`: the digest is still `d0`. |
| `continuing_an_empty_log_is_seq_zero_turn_zero` | An empty tail gives the first row at seq 0, turn 0. |
| `a_continued_recorder_keeps_the_step_s_earlier_usage` | A tail with two usage rows (in 30, out 12, cost 100) plus one new usage row (in 5) gives `run_step.usage.input_tokens == 35` and `cost_micros == 100`. |

`tests/recorder.rs`:

| Test | Asserts |
|---|---|
| `a_handoff_is_a_follow_up_at_the_next_turn_not_a_second_prompt` | After a `new` recorder writes a prompt and one turn, and a `continuing` one writes the handoff text as a follow-up: exactly one `prompt` row, at seq 0; one `follow_up` row at `turn = last + 1`; `prompt_digest` unchanged. |

---

## 5. T3: `htui-orch`, the two pure modules (D163, D167)

**First failing test**: `one_row_per_repo_and_step_with_an_after_hash`.

### 5.1 `closeout.rs`

```rust
//! ANA-2 §4.10's close-out summary (`docs/ANA-2.md:1380-1392`), pure: rows in, one `NewDocument`
//! out. The engine reads and writes; this module formats (plan D167, blueprint D208).

/// What the first confirmation shows (plan D167).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// `item.key`: the text the second confirmation asks to be typed back.
    pub key: String,
    /// `item.title`.
    pub title: String,
    /// `item.status` now.
    pub status: Status,
    /// Runs of the item the summary lists.
    pub runs: usize,
    /// `(repo, step)` rows with an `after_hash`: the commit table's length.
    pub rows: usize,
    /// The `summary` document's version once written: `max(existing) + 1`, or 1.
    pub version: i32,
}

#[must_use]
pub fn preview(item: &Item, runs: &[RunSummary], commits: &[(StepId, Vec<RunStepCommit>)],
               heads: &[DocumentHead]) -> Preview;

#[must_use]
pub fn summary(item: &Item, runs: &[RunSummary], commits: &[(StepId, Vec<RunStepCommit>)],
               repos: &[Repo], id: DocumentId, user: UserId, at: DateTime<Utc>) -> NewDocument;
```

`summary` returns `kind = "summary"`, `title = "Close-out <key>"`, `produced_by_step_id = None`, `created_by = user`, `created_at = at`. The body is D208:

```
# Close-out <key> — <title>

| repo | position | phase | attempt | commits |
|---|---|---|---|---|
| htui | 2 | implement | 1 | 0a1b2c3..9f8e7d6 |

- graph run 01J…: done, finished 2026-09-02 12:00 UTC
```

Rows cover every `(step, repo)` with `after_hash: Some`. They are ordered by run `queued_at`, then `(position, attempt, fanout_index)` of the step (looked up in the runs' `RunStepSummary`), then the repo name. Hashes are the stored strings, not abbreviated. A repo missing from `repos` renders its id. The run lines are oldest first: `<kind> run <id>: <status>[, finished <%Y-%m-%d %H:%M UTC>]`.

### 5.2 `promote.rs`

```rust
//! §4.8's promotion, the pure half (plan D163, blueprint D192, D193): which opening a promoted
//! step gets, and the handoff role's `PromptSpec`.

/// The follow-up `htui` sends when it resumes a step's own agent session.
pub const RESUME_OPENING: &str = "This step was promoted to an interactive chat. Say where you \
    stopped and what is left, then wait for the maintainer's instructions.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningKind { Resume(AgentSessionRef), Handoff }

/// The step's first `other` row whose update is `session_started`, as ANA-4 §6 records it
/// (`body.session_id`, `agent_worker.rs:2832-2839`), decoded through `replay::envelope_from_row`.
#[must_use] pub fn banner(events: &[SessionEvent]) -> Option<AgentSessionRef>;

/// Blueprint D192: resume only where the transport honours `SessionSpec.resume` today. That is
/// the CLI (`cli/mod.rs:145`, `:405`); the ACP driver never reads it (R-48).
#[must_use] pub fn opening_kind(caps: DriverCaps, transport: Transport, events: &[SessionEvent]) -> OpeningKind;

/// §4.6(c)'s handoff spec built from the phase's own spec: role `Handoff`, the pinned `handoff`
/// template's body and ref, `handoff: Some(HandoffInputs { StepSummary::from_events(events,
/// roots), diff_so_far, failure_reason })`, and `verify_failure`/`previous_diff`/`judge` cleared.
/// Every other field is the phase's.
#[must_use]
pub fn handoff_spec(phase: PromptSpec, template: &PromptTemplate, events: &[SessionEvent],
                    roots: &[RepoRoot], diff_so_far: Option<DiffBlock>, failure_reason: String) -> PromptSpec;
```

### 5.3 `lib.rs`

Add `pub mod closeout;` and `pub mod promote;` in alphabetical order. The module doc gains "Milestone 6 adds `closeout` and `promote`, the pure halves of close-out and promotion."

### 5.4 Tests

`closeout.rs`:
- `one_row_per_repo_and_step_with_an_after_hash`
- `a_step_that_committed_nothing_has_no_row`
- `rows_are_in_position_attempt_repo_order`
- `the_summary_is_kind_summary_produced_by_nobody`
- `the_preview_counts_what_the_summary_holds`: `preview.rows` equals the table's row count; `version` is 1, then 3 given heads v1 and v2.

`promote.rs`:
- `a_resumable_cli_agent_with_a_banner_resumes`
- `no_banner_means_handoff`
- `a_non_resumable_agent_means_handoff`
- `an_acp_agent_hands_off_even_when_its_caps_say_resume` (F-G)
- `the_handoff_spec_carries_the_windowed_tail_and_the_failure`: over `htui_core::prompt::fixtures::handoff_events` (`fixtures.rs:439`).
- `the_handoff_spec_keeps_the_phase_s_item_and_budget`

Plan T3's `the_opening_uses_the_step_s_own_trees_as_cwd` moves to T4, because cwd comes from `run_step_tree` rows the engine reads.

---

## 6. T5: `htui-orch`, primary writes survive a dropped walk (D160, R-26)

**First failing test**: `a_merge_dropped_mid_hook_still_lands_and_leaves_no_merge_head`, in `tests/gix_isolator.rs` with `skip_without_git!()`. The setup:
- A primary checkout with a `pre-merge-commit` hook `sleep 2`.
- `tokio::time::timeout(Duration::from_millis(300), cli.merge_no_ff(primary, step, before, after))` must answer `Err(Elapsed)`, which drops the future.
- Then `sleep(3 s)`.

It asserts no `.git/index.lock`, no `.git/MERGE_HEAD`, and `HEAD`'s parents `[before, after]`. Before the fix, `kill_on_drop(true)` (`git.rs:321`) SIGKILLs git mid-hook and leaves `MERGE_HEAD`.

In `git.rs`:

```rust
/// D160: a verb that changes the primary runs on its own task and is awaited through the handle,
/// so dropping the caller's future (a preempted or abandoned walk) lets the child finish.
async fn detached<T: Send + 'static>(
    verb: &'static str,
    work: impl Future<Output = Result<T, IsolateError>> + Send + 'static,
) -> Result<T, IsolateError> {
    tokio::spawn(work).await.map_err(|join| IsolateError::Git(format!("git {verb}: its task failed: {join}")))?
}
```

- `pub async fn reset_hard(&self, tree, target)` becomes `detached("reset --hard", { let cli = self.clone(); let tree = tree.to_path_buf(); let target = target.to_owned(); async move { cli.reset_hard_attached(&tree, &target).await } })`. The current body moves to a private `reset_hard_attached`.
- `merge_no_ff` does the same over `merge_no_ff_reading`. The whole merge, including its own abort, runs on the task.
- `abort_merge` does the same, over `abort_merge_attached`. `merge_no_ff_reading` calls the `_attached` variant, because it is already on a task.
- Signatures, `with_retry` and the `worktree add/remove` verbs are unchanged. C-1 still holds, because `tokio::spawn` is not `Command::new`.

---

## 7. T4: `htui-orch`, the four commands and the carried engine fixes

### 7.1 `command.rs` (commit a)

`Command` gains four variants. The module doc (`:1-3`) and the `Command` doc (`:27-38`) list nine verbs, and name `CancelStep` and `OpenArtifact` as not built (D173, D178).

```rust
/// §4.8 / D163. `chat_open` is a fact only the caller can know (blueprint D185): a chat of this
/// process is live. The worker overwrites whatever a view sent; a headless caller passes `false`.
PromoteStep { run: RunId, step: StepId, chat_open: bool },
/// §4.8 accept artifact (D166). `chat_live`: a chat of this process is live **on this step**.
AcceptArtifact { run: RunId, step: StepId, chat_live: bool },
/// §4.3 verdict 1 (D161).
Unblock { item: ItemId },
/// `R-TUI-9` (D167). The engine builds the summary; the caller names the item.
CloseOut { item: ItemId },
```

`CommandOutcome` (still `Debug, Clone, PartialEq, Eq`) gains:
- `Promoted { step, rest, opening: Box<Opening> }`
- `Accepted { rest }`
- `Unblocked { item, case: UnblockCase, rest: Option<Rest> }`
- `ClosedOut { item, summary: DocumentId, version: i32 }`

```rust
/// D163, blueprint D192: how the promoted step's chat opens. No `AssembledPrompt` (not `Eq`, and
/// its trim record must not be written over the step's): the text and its digest only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub agent_id: AgentId, pub agent_name: String, pub model: Option<String>,
    pub phase: String, pub cwd: PathBuf, pub extra_dirs: Vec<PathBuf>,
    pub path: OpeningPath,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpeningPath {
    Resume { session_ref: AgentSessionRef, text: String },   // text = promote::RESUME_OPENING
    Handoff { text: String, digest: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnblockCase { Reopen, FollowRun(RunId), Resume(RunId) }
```

New `EngineError` arms and their `Display` (byte-exact, one test each):

| Arm | Bytes |
|---|---|
| `NotBlocked { item, status, why: &'static str }` | ``item {item} is `{status}`; {why}`` |
| `NotPromoted { step }` | ``step {step} was not promoted to chat; accept artifact needs a promoted step (ANA-2 §4.8)`` |
| `PromoteCandidate { step }` | ``step {step} is a fan-out candidate; select a winner instead of promoting one`` |
| `ChatLive { step: Option<StepId> }` | `None`: ``end the open chat first (Chat tab, Esc Esc)``; `Some(s)`: ``step {s} is being chatted with; end that chat first (Chat tab, Esc Esc)`` |
| `AcceptVerifyFailed { step, exit_code: Option<i32> }` | ``step {step}: the verify command failed (exit {code\|none}); fix it in the chat, then accept again`` |
| `NotClosable { item, status }` | ``item {item} is `{status}`; close-out needs `done`, `failed` or `blocked` (ANA-2 §4.10)`` |
| `NotTerminal { run, status }` | ``run {run} is `{status}`; a cleanup retry is for a finished run (R-25)`` |

The `ItemBlocked` bytes (`command.rs:229-236`) become ``item {item} is blocked; `Unblock` (`u`) clears it (ANA-2 §4.3)``. Any test that pins the old bytes is updated in the same commit.

**Admission functions (D184).** Each is pure. The engine calls it after its reads and before `take_lease`, and `run_worker::actions` calls the same function.

```rust
pub fn snapshot_of(run: &Run) -> Result<GraphSnapshot, EngineError>;             // moved from engine.rs:4505
pub fn phase_at(run: RunId, s: &GraphSnapshot, position: i32) -> Result<SnapshotPhase, EngineError>;
/// retry_step's whole pre-dispatch order (engine.rs:~740-775): run status, ItemBlocked, the
/// fan-out route (latest slot → StaleSlot, group → retry_group_enabled), else retry_enabled.
pub fn retry_admitted(run: &Run, item: Status, steps: &[RunStep], step: &RunStep, phase: &SnapshotPhase) -> Result<(), EngineError>;
/// Step running|awaiting_approval|failed (§6.2 `:1563`), run non-terminal, item not blocked
/// (ItemBlocked), not a fan-out candidate (phase.fan_out > 1 → PromoteCandidate), chat_open false.
pub fn promote_enabled(run: &Run, item: Status, steps: &[RunStep], step: &RunStep, phase: &SnapshotPhase, chat_open: bool) -> Result<(), EngineError>; // steps added by T4 repair b6cb08f: refuses a non-latest attempt with StaleSlot
/// Step awaiting_approval, promoted_at set (NotPromoted), has_output (MissingOutputForApproval),
/// chat_live false (ChatLive(Some)).
pub fn accept_enabled(steps: &[RunStep], step: &RunStep, phase: &SnapshotPhase, has_output: bool, chat_live: bool) -> Result<(), EngineError>; // steps added by T4 repair b6cb08f
/// D161's three cases in order over (run, its cursor) pairs; resumable_park decides case 3.
pub fn unblock_enabled(item: &Item, runs: &[(Run, Cursor)]) -> Result<UnblockCase, EngineError>;
/// No active run (RunStatus naming the first live one), item done|failed|blocked (NotClosable).
pub fn close_out_enabled(item: &Item, runs: &[Run]) -> Result<(), EngineError>;
/// run.status.is_terminal() else NotTerminal.
pub fn cleanup_enabled(run: &Run) -> Result<(), EngineError>;
/// Row-level mirror of `create_run`'s item compare-and-set: Status::can_move_to(Queued). The
/// engine does NOT call it (legal_move stays the authority); `start_enabled_is_the_law` pins them.
pub fn start_enabled(item: &Item) -> Result<(), EngineError>;
```

`unblock_enabled`, case by case:
1. The item is `blocked` and no run is active → `Reopen`.
2. The item is `blocked` and an active run is `awaiting_approval` → `FollowRun(run)`.
3. The item is `awaiting_approval`, and a run that is `awaiting_approval` has `status::resumable_park(&cursor)` → `Resume(run)`.
4. Anything else is `NotBlocked`, with a `why` naming what holds the item: "nothing is blocked", "run R is `running`", or "run R is parked at a gate; answer it".

The R-7 sentence at `:505-521` is rewritten to name `Unblock` (`u`, D161 case 3).

### 7.2 `status.rs` (commit a)

- `RunFailure::PromptRefused { phase: String, reason: String }` with `Display` ``prompt refused at `{phase}`: {reason}``. `run_failure_display_is_ana2s_bytes` gains its row.
- `pub fn resumable_park(cursor: &Cursor) -> bool { matches!(cursor, Cursor::Create { .. } | Cursor::Run(_) | Cursor::Finished) }`. That is `walk_resumed`'s own predicate (`engine.rs:2046-2050`), so D132 and `Unblock` case 3 cannot drift.

### 7.3 Engine refactors, behaviour-preserving (commit b)

| Change | Where | Note |
|---|---|---|
| `pub async fn enqueue(&self, item, mode, repo_scope) -> Result<RunId, EngineError>` | from `start_run` `:511-560` | Resolve, rung-4 refusal, `create_run`. `start_run` becomes `let id = self.enqueue(..).await?; self.claim(id).await` (D186). |
| `retry_step` calls `command::retry_admitted` | `:~740-775` | Existing cases pin the refactor. |
| `snapshot_of` / `phase_at` call the `command::` versions | `:4505`, `:4538` | — |
| `walk_resumed` uses `status::resumable_park` | `:2046` | — |
| `pub fn DeadWalks::mark(&self, run)` | `:276-316` | Doc: "a walk task of this process died (a panic), so the next sweep releases and adopts it (plan D158, R-12)". |
| `pub trait RunFence: Sync { type Guard: Send; fn hold(&self, run: RunId) -> Option<Self::Guard>; }` and `pub async fn sweep_fenced<F: RunFence>(&self, fence: &F)` | `sweep` `:1325` | `sweep()` becomes `self.sweep_fenced(&NoFence).await`. In the dead-walk pre-pass, a run whose `hold` answers `None` is **not** released. In recovery, `hold` is asked after `adopt_runs`; `None` is skipped with a `debug!` (the adopted lease is this owner's, and the holder's own `take_lease` renews it); `Some(guard)` is held through `walk_leased(recover_run)`. D189. |
| `pub async fn abandoned(&self, run: RunId)` | beside `release_lease` `:1260` | `isolator.release(run)` (warn on error) then `release_lease(run)`. Never raises. D188. |
| `pub async fn unblock_case(&self, item) -> Result<UnblockCase, EngineError>` | new | Read-only: item, `runs(item)`, each `run` plus `run_steps` plus `cursor`, then `unblock_enabled`. |
| `pub async fn close_out_preview(&self, item) -> Result<closeout::Preview, EngineError>` | new | Read-only: `close_out_enabled`, then `closeout::preview`. |
| `assemble_prompt` answers `Result<Result<AssembledPrompt, StageThree>, EngineError>` | `:4058` | `enum StageThree { MissingInput(String), Refused(AssembleError) }`. Only `assemble(&spec, ..)` maps to `Refused`. Store and `NoTemplate` errors stay outer. Both call sites (`:2387`, `:2704`) match the old `MissingInput` arm unchanged. D195. |
| Compile test `a_dispatch_future_is_send` (engine unit) | tests | `fn assert_send<F: Send>(_: &F) {}` over `engine.dispatch(cmd)`, `engine.resume(run)` and `engine.sweep_fenced(&f)` futures with `MemStore`, `dyn Isolator`, `dyn Verifier` and `dyn Clock`. It surfaces any `!Send` before T6 spawns one (D204, R-52). |

### 7.4 D162 at both call sites, D159, D179, D180 (commit c)

**`refuse_prompt` (D162, D195)**, reached from `walk_live_step` on `Err(StageThree::Refused(err))`:
1. `transition_step(step, Running → Failed, now)`. `Ok(false)` → `stale_step(..)`, as `fail_before_a_token` does (`:4031-4050`). No `gate_note` is written (plan amendment).
2. If `run.item_id` is set: `transition(item, InProgress → Blocked)`, then `note(item, failure.to_string(), Some(step.id), now)`.
3. `finish_run(run, Failed, Some(&failure.to_string()), now)`. The item is already `blocked`, so the mirror leaves it (`traits.rs:1281-1290`).
4. `cleanup_run(run)`, then `Ok(Some(Rest { Failed, Some(position), Some(failure) }))`.

`failure = RunFailure::PromptRefused { phase, reason: err.to_string() }`. No recorder is opened, because stage 4 is never reached (`:4294`).

**`drive_group`'s escape** (`:2704-2706`) takes the same path, generalised as `fail_group_before_a_token(run, phase, &pending, failure: RunFailure, block: bool)` (`:2820`). Each pending candidate goes `pending → running → failed` through `fail_candidate` (unchanged). When `block` is set, the item moves `in_progress → blocked` plus the note, then `finish_run` and `cleanup_run`. The missing-input path calls it with `block = false`, so its behaviour is identical.

**D159**: in `resume` (`:1950-1962`), when `resume_window` answers `Some(TopologyChanged)` and the row read in the window was `running`:
- `move_run(Running → AwaitingApproval)`, then `transition(item, InProgress → AwaitingApproval)`, inside `leased_window`.
- The existing release follows.
- `TopologyChanged.rest.run` is then `AwaitingApproval`.

**D179**: `cancel_run` (`:~1057`) takes the lease through `take_lease` for a `running` or `awaiting_approval` run after `cancel_enabled` and before its first write. `LeaseHeld` means nothing was written. The writes and `cleanup_run` run in `leased_window`, then `release_lease(run)`. A `queued` run takes no lease.

**D180**: in `walk_resumed`, `if !self.unpark(&row, now).await? { return Err(stale_run(run, AwaitingApproval, Running)); }`.

**D176**: the comment at `:4358` becomes: "`R-SEC-2`: no secret provider exists yet; MOD-10 (secret provider, from ANA-7) owns wiring one. The walk invents none."

### 7.5 `promote` (commit d, D163, D191, D192)

`dispatch` arm: `PromoteStep { run, step, chat_open } => self.promote(run, step, chat_open)`.

1. Reads: `run`, `snapshot_of`, `step`, `phase_at`, the item's status, then `promote_enabled(..)`.
2. `take_lease(run)`, then `leased_window`:
   - if `step.status == Running`: `move_step(Running → AwaitingApproval)` (legal, `run.rs:115-118`);
   - `store.promote_step(step, now)` (`traits.rs:983`), which parks the run and the item in one transaction;
   - `note(item, "step <id> (`<phase>` attempt <a>) promoted to chat", Some(step), now)`.
3. `release_lease(run)`. The run is parked, and a parked run holds no lease (D87).
4. Opening:
   - `agent = graphs.agent(step.agent_id)`, then `caps = htui_agent::registry::caps_for(&agent)`;
   - `events = store.step_events(step).await?.unwrap_or_default()`;
   - `trees = store.step_trees(step)`; `repos = store.repos(project)`. `cwd` is the primary repo's tree path, else the first tree's, else `EngineError::Snapshot` ("the step has no tree to chat in"). `extra_dirs` are the others.
   - `promote::opening_kind(caps, agent.transport, &events)` decides:
     - `Resume(ref)` gives `OpeningPath::Resume { session_ref: ref, text: RESUME_OPENING }`.
     - `Handoff` rebuilds the phase spec (the `assemble_prompt` inputs, factored as `phase_spec(..)`), with `template = graphs.prompt_template(project, "handoff", None)`, `roots` from the trees (repo name, tree path), `diff_so_far` from the step's commits through `isolator.diff`, and `failure_reason = run.failure.or(step.gate_note).unwrap_or("promoted by the maintainer at a gate")`. Then `assemble(handoff_spec)`, and `OpeningPath::Handoff { text, digest }`. A `StageThree::Refused` here is raised as `EngineError::Prompt`. The promotion has already been written, and the chat is simply not opened; the worker reports the sentence.
5. Answer `Promoted { step, rest: resting(run), opening }`.

No `run(kind='chat')` row is written, and neither `set_step_prompt` nor `record_prompt` is called.

### 7.6 `accept_artifact` (commit e, D166, D194)

1. Reads: `run`, snapshot, step, phase, `has_output = output_of(..).is_some()` (`:4597`).
2. `accept_enabled(&step, &phase, has_output, chat_live)`.
3. `until = take_lease(run)`. Then, in `leased_window`:
   - `trees = step_trees(step)`;
   - `verify = self.verify(VerifyStage { result: &Ok(DoneEvent { stop_reason: EndTurn }), started_at: now, session_cwd: &cwd, .. })` (`:2515`). **D211 (review round, §21):** `started_at` is the accept, not `step.started_at`, so the verify runs on a fresh copy of the phase deadline; measured from the step's start it was `Unavailable` (zero remainder) on every accept more than `deadline_seconds` after the agent began;
   - `after = isolator.capture(step, &trees)`, then `record_commits(step, &after)`;
   - `finish_step(StepOutcome { verify_outcome, verify_exit_code, finished_at: now, usage: None, trim_record: None, exit_code: None })`.
4. `verify_outcome == Some(Fail)` → note, `release_lease`, and `Err(AcceptVerifyFailed { step, exit_code })`. The step stays promoted and `awaiting_approval`. `Some(Unavailable)` is not refused (D30: `unavailable` never fails a step): it is recorded, and the note `accept: verify unavailable: <reason>` is written in the same window (D211).
5. Otherwise `self.answer_guarded(&run, &snapshot, &step, &phase, GateAnswer::Approved)` (`:633`). It re-takes the lease (a renewal), answers `approved`, unparks, reconciles, and walks from `position + 1` under the heartbeat. The outcome is mapped to `Accepted { rest }`.

`GateAnswer::Skipped` stays unexposed (plan disagreement 4).

### 7.7 `unblock` (commit f, D161)

`unblock_case(item)`, then:
- `Reopen`: `transition(item, Blocked → Open)`, then the note ``unblocked: back to `open` ``.
- `FollowRun(r)`: `transition(item, Blocked → AwaitingApproval)` (T1's edge), then the note ``unblocked: follows run {r}, parked at `awaiting_approval` ``.
- `Resume(r)`: the note ``unblocked: resuming run {r}``, then `self.resume(r)` (lease, `walk_resumed`, walk).

A compare-and-set that answers `false` is `StaleWrite`. The answer is `Unblocked { item, case, rest }`, where `rest` is the walk's for `Resume`.

### 7.8 `close_out` (commit g, D167)

1. Reads: `item`, `runs(item)` (summaries), each `run` row, `close_out_enabled`, `step_commits` for every step of every run, `repos(project)`.
2. `doc = closeout::summary(.., DocumentId::new(), user, now)`.
3. `store.close_out(item, doc, &[])` (`traits.rs:1045`). The store re-checks both refusals before writing (`:1034-1043`).
4. Answer `ClosedOut { item, summary: written.id, version: written.version }`.

No run lease is involved. The store refuses a live run in the same transaction.

### 7.9 Harness and cases (commit h)

**`fake.rs`**: `FakeOrchestrator::refuse_prompt(&self, phase: &str)` makes `FakeGraphSource::prompt_template` answer, for that phase's pinned name, a template whose body holds `{{no_such_placeholder}}`. That is `TemplateError::UnknownPlaceholder`, criterion 3's literal case. **`conformance.rs`**: `Orchestrate` gains `fn refuse_prompt(&self, phase: &str)`.

**Sixteen `CASES`, 52 → 68**, all over the fakes. Setup lines name only what differs from the usual `primary_repo` + `StartRun(FEAT-3)` shape.

| Case | Asserts |
|---|---|
| `promote_keeps_the_step_and_writes_no_chat_run` (17a) | `prd` parked at its gate, then `PromoteStep` → `Promoted`: the same step id, `promoted_at` set, run and item `awaiting_approval`, zero `kind = chat` runs, `prompt_digest` unchanged, and the opening `Handoff` (the fake agent row is not resumable). |
| `promote_a_failed_step_of_a_parked_run` | A failed step under a parked run → `Promoted`, and the step is `awaiting_approval`. |
| `promote_refuses_a_terminal_run` | → `RunStatus`; nothing is written. |
| `promote_moves_a_dropped_running_step_to_awaiting` (D163's engine half) | `stall_after_done` then `until_stalled` leaves `prd` `running`. `PromoteStep` from the same owner → `Promoted`, and the step is `awaiting_approval` with `promoted_at`. |
| `accept_artifact_verifies_captures_and_resumes_at_the_next_position` (17b) | Promote, then `AcceptArtifact` → `Accepted`. The step is `done` with `approved`, `verify_outcome` is recorded, there is one `after_hash`, and `plan` (position + 1) exists. |
| `accept_artifact_needs_the_document_and_the_promotion` | Without promotion → `NotPromoted`; without a document → `MissingOutputForApproval`; with `chat_live` → `ChatLive(Some)`. Each writes nothing. |
| `accept_artifact_refuses_a_failed_verify` (D194) | With `FakeVerifier` scripted to `fail`: `AcceptVerifyFailed`, the step still `awaiting_approval`, and `verify_outcome = fail` recorded. |
| `unblock_opens_a_blocked_item_with_no_run` (criterion 14) | Rung four's `no_candidate_agent` (`:594-611`) leaves the item `blocked`; `Unblock` → `Reopen` and the item is `open`. |
| `unblock_lets_an_escalated_run_be_promoted_and_approved` (R-4) | The review loop escalates (item `blocked`, run `awaiting_approval`); `Unblock` → `FollowRun`; `PromoteStep` → `Promoted`; `AnswerGate(Approved)` → `Answered`. |
| `unblock_resumes_a_reconcile_refused_park` (R-7) | `FakeIsolator::refuse_reconcile` parks the run on a `done` step; `Unblock` → `Resume`, and the walk goes on. |
| `close_out_writes_one_summary_and_closes_the_item` (criterion 20) | One `summary` document at version 1, with a body row per `(repo, step)`; the item is `closed` with `closed_at` set. |
| `close_out_is_refused_while_a_run_is_live` (criterion 20) | → `RunStatus`; no document and no move. |
| `a_prompt_refusal_blocks_the_item_and_starts_no_session` (ANA-5 criterion 3, `walk_step`) | `refuse_prompt("prd")`, then `StartRun` → the step is `failed`, the item `blocked`, the run `failed` with ``prompt refused at `prd`: …``, zero `session_event` rows, and one note. |
| `a_prompt_refusal_in_a_fan_out_blocks_the_item` (ANA-5 criterion 3, `drive_group`) | A fan-out 2 phase: both candidates `failed`, the item `blocked`, the run `failed`, no session. |
| `a_topology_mismatch_parks_a_running_run` (criterion 3, D159) | A stall, `restarted()`, a graph edit, a sweep (`Walk`), then `resume` → `TopologyChanged`; the run and the item are `awaiting_approval`. It replaces the engine unit test `a_topology_mismatch_on_a_running_run_leaves_it_adoptable_by_the_same_process` (`engine.rs:9540`, deleted). |
| `cancel_meets_a_live_lease_and_writes_nothing` (R-28, D179) | A foreign live lease, then `CancelRun` → `LeaseHeld`; every step and the run are unchanged. |

**Engine unit tests** (not `CASES`):
- `a_marked_dead_walk_is_adopted_by_the_next_sweep` (D158)
- `a_fenced_sweep_skips_a_held_run_and_keeps_its_lease` (D189)
- `abandoned_releases_the_guards_and_the_lease` (D188)
- `walk_resumed_stops_on_a_stale_unpark` (D180)
- One per untested D132 crash path (D180): `a_crash_between_a_retry_and_the_unpark_is_resumed`, `a_crash_between_a_group_retry_and_the_unpark_is_resumed`, `a_crash_between_a_selection_and_the_unpark_is_resumed`. If the group retry's cursor reads `Fan` (not `resumable_park`), that case asserts the run stays parked and records it under R-31's remainder rather than widening D132.
- `enqueue_then_claim_is_start_run`
- `a_dispatch_future_is_send`

**`command.rs` guard tests**: one per new admission function, each asserting the named refusal. `start_enabled_is_the_law` iterates `Status::ALL` and checks `start_enabled(item).is_ok() == status.can_move_to(Queued)`.

**Pins**:
- `conformance.rs:292` (`CASES`) and `:4307` become `cases_are_unique_and_sixty_eight`, with the message recounted: 18 + 5 + 13 + 6 + 10 + 16, the last group enumerated as §7.9 lists it.
- `tests/fake_conformance.rs:15-16` becomes `cases_len_is_sixty_eight` / `68`.

**`lib.rs`**:
- `pub use command::{Opening, OpeningPath, UnblockCase, start_enabled, retry_admitted, promote_enabled, accept_enabled, unblock_enabled, close_out_enabled, cleanup_enabled, snapshot_of, phase_at};`
- `engine::RunFence` joins the `engine::` list.
- `status::resumable_park` joins the `status::` list.

### 7.10 Commit boundaries (T4)

(a) `command.rs` + `status.rs` types, admission functions and their tests. The engine does not call them yet. (b) The §7.3 refactors, green, with no behaviour change. (c) D162 at both sites + D159 + D179 + D180, each red then green. (d) promote + `fake.rs` harness. (e) accept. (f) unblock. (g) close-out. (h) pins, `lib.rs` re-exports, docs (`engine.rs:442-448`, `:560-561`, `:1941-1944` now name `run_worker` as the caller).

---

## 8. T6: `htui`, `run_worker.rs` and the request plumbing

### 8.1 Manifests (commit a)

- `Cargo.toml` `[workspace.dependencies]`: `htui-orch = { path = "crates/htui-orch" }` beside the other three (`:23-26`).
- `crates/htui/Cargo.toml` `[dependencies]`: `htui-orch = { workspace = true }`.
- `[dev-dependencies]`: `htui-orch = { workspace = true, features = ["test-support"] }`, which also reaches `htui-core/test-support` for `MemFault`.

### 8.2 Request and reply shapes (commit b, D154, D182, D183, D209)

`store_worker.rs`, after `ClearQdrantSettings`. `StoreRequest` goes 58 → 62:

```rust
/// Plan D154: one orchestrator command or read, served by `run_worker::RunRuntime` on its own
/// task (`R-NF-3`). Answered once at this `seq`, possibly hours later (R-41).
Orch(crate::run_worker::OrchRequest),
/// Plan D172: a subscription. Answered at once with `RunFrame { kind: Subscribed }`, then with a
/// frame at **this** `seq` per change to the item's runs (blueprint §0a point 3).
RunStream { item: ItemId },
/// Plan D173: one document with its body.
Document(DocumentId),
/// Blueprint D182: every action's enabling verdict for the item, from the engine's own guards.
RunActions(ItemId),
```

`name()` arms (still a `const fn`):
- `Orch` maps through a nested match to D209's per-verb names: `start_run`, `answer_gate`, `retry_step`, `cancel_run`, `select_fanout`, `promote_step`, `accept_artifact`, `unblock`, `close_out`, `close_out_preview` and `cleanup_run`. They are `run_worker::ORCH_NAMES: [&str; 11]`, in that order.
- The others are `run_stream`, `document` and `run_actions`.

`StoreReply` adds:
- `Orch(crate::run_worker::OrchReply)`
- `RunStream(crate::run_worker::RunFrame)`
- `Document(Box<Option<Document>>)`
- `RunActions(Box<crate::run_worker::ItemActions>)`

`try_serve` (`:911-1012`, no wildcard):

```rust
StoreRequest::Document(id) => StoreReply::Document(Box::new(backend.document(*id).await?)),
StoreRequest::RunStream { item } => StoreReply::RunStream(RunFrame::subscribed(*item)),
StoreRequest::RunActions(item) => StoreReply::RunActions(Box::new(
    crate::run_worker::actions(backend, *item, &LiveChats::default()).await?)),
StoreRequest::Orch(_) => StoreReply::Failed { request: request.name(), message: "no run runtime in this build".to_owned() },
```

`run_worker.rs` types:

```rust
#[derive(Debug, Clone)]
pub enum OrchRequest { Command(htui_orch::Command), CloseOutPreview { item: ItemId }, Cleanup { run: RunId } }

#[derive(Debug, Clone)]
pub enum OrchReply {
    Done(Box<htui_orch::CommandOutcome>),
    /// To the Chat tab only (D165, D191): the promoted step and how its chat opens.
    Promoted { step: StepId, run: RunId, phase: String, agent: String, model: Option<String>, via: Via },
    CloseOutPreview(Box<htui_orch::closeout::Preview>),
    CleanedUp { run: RunId },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Via { Resumed, Handoff }

#[derive(Debug, Clone)]
pub struct RunFrame { pub item: ItemId, pub run: Option<RunId>, pub kind: FrameKind }
#[derive(Debug, Clone)]
pub enum FrameKind { Subscribed, Started, SessionDone { step: StepId }, Rested(htui_orch::Rest), Adopted, Error(String) }
// T6 repair 8548711 added a seventh variant, `Changed`: a command whose outcome has no `Rest` (ClosedOut, a Reopen Unblock, a successful cleanup) publishes it so the pane re-reads. T8 must handle all seven.

pub type Enabled = Result<(), String>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemActions {
    pub item: ItemId, pub key: String,
    pub run: Enabled, pub unblock: Enabled, pub close_out: Enabled,
    pub runs: BTreeMap<RunId, RunActions>, pub steps: BTreeMap<StepId, StepActions>,
}
#[derive(Debug, Clone, PartialEq, Eq)] pub struct RunActions { pub cancel: Enabled, pub cleanup: Enabled }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepActions { pub approve: Enabled, pub reject: Enabled, pub retry: Enabled, pub promote: Enabled,
                         pub accept: Enabled, pub select: Enabled, pub open: Result<DocumentId, String> }

/// Blueprint D206: the steps a chat of this process is live on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveChats(BTreeSet<StepId>);
impl LiveChats { pub fn of(steps: impl IntoIterator<Item = StepId>) -> Self; pub fn is_empty(&self) -> bool; pub fn contains(&self, s: StepId) -> bool; }
```

None of these carries a secret (`store_worker.rs:71-78`). A rejection note is the user's own text.

**`actions(backend, item, live) -> StoreResult<ItemActions>` (D182, D184)** reads `item`, `runs(item)` (summaries), `documents(item)`, and per run `run(id)` + `run_steps(id)`. It answers:
- `writer()` is `None` → every command verdict is `Err(DATABASE_UNREACHABLE)` (`writer.rs`, re-exported `htui_store::DATABASE_UNREACHABLE`).
- A snapshot that does not decode → every step verdict of that run is `Err(sentence)`.
- Per step:
  - `approve = answer_gate_enabled(step, phase, has_output, &Approved)`;
  - `reject` = the same with `Rejected { note: String::new() }`;
  - `retry = retry_admitted(..)`;
  - `promote = promote_enabled(run, item.status, steps, step, phase, !live.is_empty())`;
  - `accept = accept_enabled(steps, step, phase, has_output, live.contains(step.id))`;
  - `select = select_enabled(run, &group_at(steps, p, a), step.id, p, a)` for `fanout_index >= 0 && phase.fan_out > 1`, else `Err("step X is not a fan-out candidate")`;
  - `open` = the newest head of `phase.output_kind` produced by the step, else ``Err("step X produced no `<kind>` document")``.
- Item level: `unblock_enabled(..).map(drop)`, `close_out_enabled`, `start_enabled`.
- Per run: `cancel_enabled`, `cleanup_enabled`.
- **D212 (review round, §21):** per run, `chatting = chat_free(steps, live)` comes first: while a chat of this process is live on a step of the run, `approve`, `reject`, `retry`, `select` of every step of the run and the run's `cancel` are its `ChatLive { step: Some(s) }` sentence — the refusal the worker answers them with (D184's one admission function).
- Every `Err` is its `EngineError`'s `Display`.

### 8.3 `RunRuntime` inside the store worker's `select!` (commit c)

```rust
/// What `RunRuntime::serve` (and the runtime's event channel) decided about one request.
pub enum RunServed {
    Reply(StoreReply),
    /// A task this runtime owns answers the request, exactly once.
    Deferred,
    /// D165/D181: a promotion's engine writes are done; the loop hands `promoted` to
    /// `AgentRuntime::attach_promoted` (T7) and answers at `addr`.
    Attach { addr: crate::agent_worker::ReplyAddr, promoted: Box<Promoted> },
}
pub struct Promoted { pub run: RunId, pub step: StepId, pub project: ProjectId, pub opening: htui_orch::Opening }

impl RunRuntime {
    pub fn production() -> Self;
    pub fn with_parts(isolator: Arc<dyn Isolator>, verifier: Arc<dyn Verifier>, drivers: DriverFactory) -> Self;
    pub fn with_clock(self, clock: Arc<dyn Clock>) -> Self;              // tests: a tokio-time clock
    pub fn with_author(self, author: Arc<dyn StepAuthor>) -> Self;      // D203
    pub fn with_sweep_every(self, every: Duration) -> Self;             // D190
    pub fn with_scratch_root(self, root: PathBuf) -> Self;              // D202, production isolator
    /// Called once, before the loop: the receiver the loop owns (D181).
    pub fn take_events(&mut self) -> mpsc::UnboundedReceiver<RunServed>;
    pub async fn serve(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>,
                       envelope: &RequestEnvelope, live: &LiveChats) -> RunServed;
    pub fn sweep(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>);
    pub fn sweep_every(&self) -> Duration;
    /// Harness only: await every task, each under `limit`; answers the runs still running.
    pub async fn settle(&mut self, limit: Duration) -> Vec<RunId>;
    pub async fn shutdown(&mut self, grace: Duration);
    pub fn isolator_builds(&self) -> usize;                              // D156 test hook
}
```

**`store_worker.rs` wiring.** `spawn(started, rx, tx)` keeps its signature and calls `spawn_with_runtimes(started, rx, tx, AgentRuntime::production(), RunRuntime::production())`. `spawn_with(started, rx, tx, agent)` keeps its four arguments (called at `:1057`, `:2212`, `:2289`, `:2417`) and calls `spawn_with_runtimes(.., agent, RunRuntime::production())`. Inside `spawn_with_runtimes`:

```text
let mut run_events = runs.take_events();
let mut sweep_ticker = interval_at(now + runs.sweep_every(), runs.sweep_every()); Delay behaviour
if backend.writer().is_some() { runs.sweep(&backend, &tx); }          // Memory, or started Online (D190)
loop { select! {
  envelope = rx.recv() => { … existing arms …
     StoreRequest::Orch(_) | StoreRequest::RunStream { .. } | StoreRequest::RunActions(_) => {
         let live = live_chats(&runtime);          // T6: steps().filter(caps.is_some()); T7: runtime.live_steps()
         match runs.serve(&backend, &tx, &envelope, &live).await {
             RunServed::Reply(reply) => reply,
             RunServed::Deferred => continue,
             attach @ RunServed::Attach { .. } => { on_run_served(attach, &mut runtime, &backend, &tx).await; continue }
         }
     }
     other => try_serve … (unchanged; `Document` lands here)
  }
  Some(served) = run_events.recv() => on_run_served(served, &mut runtime, &backend, &tx).await,
  _ = sweep_ticker.tick(), if backend.writer().is_some() => runs.sweep(&backend, &tx),
  … ConnEvent::Online(pg) => { go_online(..); runs.sweep(&backend, &tx); }   // first and every Online
}}
runs.shutdown(CANCEL_GRACE).await;                // before runtime.shutdown (:1449)
```

`on_run_served` is private to `store_worker.rs`.
- **T6 stub**: `Attach { addr, .. }` sends `ReplyEnvelope { seq: addr.seq, origin: addr.origin, reply: Failed { request: "promote_step", message: "promotion needs the chat runtime".into() } }`.
- The event channel carries only `Attach`. A `Reply` or `Deferred` arriving there is a bug: `debug_assert!` plus a `tracing::error!`, and it is dropped.
- **T7** replaces the `Attach` stub (§10.3).

**Server switch (T6 repair).** `SetDsn` step (6) calls `runs.forget_server()`. A walk belongs to the server it was claimed on, so every walk of this process is preempted and gives its lease back to that server through `abandoned` (that server's next sweep, in any process, adopts the run); the production isolator and verifier and the claim queue are forgotten, so the new server's first command builds its own parts and is never refused with R-39's sentence for the old server's walks. Shutdown cancels walks and chats side by side (`tokio::join!`), each runtime inside one `2 × CANCEL_GRACE` window.

The `select!` futures are dropped before any handler runs, so handlers may borrow `runs` and `runtime`. The receiver and the ticker are locals, like `ticker` and `events` (`:1086-1115`). **`event_loop.rs` is untouched** (ANA-2 `:1687`).

### 8.4 `RunLocks`, tokens and preemption (commit d, D157, D187)

```rust
/// R-27: one async mutex per run, minted on first use. Held by every command, resume, claim and
/// sweep-driven recovery of that run for its whole duration. Pruned at a sweep tick once nobody
/// holds or waits for it (D214).
#[derive(Debug, Clone, Default)]
pub struct RunLocks(Arc<std::sync::Mutex<HashMap<RunId, Arc<tokio::sync::Mutex<()>>>>>);
impl RunLocks {
    pub async fn lock(&self, run: RunId) -> tokio::sync::OwnedMutexGuard<()>;
    pub fn try_lock(&self, run: RunId) -> Option<tokio::sync::OwnedMutexGuard<()>>;
}
impl htui_orch::RunFence for RunLocks { type Guard = OwnedMutexGuard<()>; fn hold(&self, run: RunId) -> Option<Self::Guard> { self.try_lock(run) } }

/// D187: one parent token per run; every task of the run works under a child.
#[derive(Debug, Clone, Default)]
struct Walks(Arc<std::sync::Mutex<HashMap<RunId, CancellationToken>>>);
impl Walks { fn child(&self, run: RunId) -> CancellationToken; fn preempt(&self, run: RunId) -> bool; fn is_live(&self, run: RunId) -> bool; }
```

The std mutex is never held across an `.await`. Every task over run *r* has this shape:

```text
let token = walks.child(r);
let guard = select! { g = locks.lock(r) => g, _ = token.cancelled() => return };   // cancelled while queued: walk nothing
let out = select! { out = work => Some(out), _ = token.cancelled() => None };     // `work` boxed; dropped on cancel
if out.is_none() { engine.abandoned(r).await }                                       // D188 (F-D)
drop(guard);
```

`CancelRun{r}` and `PromoteStep{r, ..}` call `walks.preempt(r)` first. That removes and cancels the parent, so the preempting task's own child comes from a fresh parent. Then they await the lock. Dropping the walk kills the agent through `ChildGuard::drop` (M5 D86). T5 lets a primary `merge`/`reset` child finish.

### 8.5 Serving (commit c, then e)

`serve` sweeps finished handles first (`AgentRuntime::serve`'s shape, `agent_worker.rs:531-548`). Then:

| Request | Answer |
|---|---|
| `RunStream { item }` | `publisher.subscribe(origin, seq, item)`; `Reply(RunStream(RunFrame::subscribed(item)))` |
| `RunActions(item)` | **D215 (review round, §21):** spawn a task tracked through `shared.track`, over clones of the backend, the reply sender and `live`: `actions(backend, item, live).await ⇒ RunActions \| Failed`, answered at `(origin, seq)` → `Deferred`. `try_serve`'s runtime-less path (D183) stays inline. (Was `Reply(actions(..).await ⇒ ..)`, inline on the loop.) |
| `Orch(_)` with `backend.writer()` `None` | `Reply(Failed { request: name, message: DATABASE_UNREACHABLE })`. Nothing is spawned (D174). |
| `Orch(Command(c))` | overwrite `chat_open`/`chat_live` from `live` (D185), then spawn the §8.6 task → `Deferred` |
| `Orch(CloseOutPreview { item })` | spawn: `engine.close_out_preview` → `Orch(CloseOutPreview)` or `Failed` → `Deferred` |
| `Orch(Cleanup { run })` | spawn under `RunLocks[run]`: `cleanup_enabled`, then `engine.cleanup_run(run)` → `Orch(CleanedUp)` → `Deferred` |

**Per-task parts (D156, D202).** Each task owns clones and builds one `Engine` per step of work:
- `Writer` (`backend.writer()`);
- `BackendGraphs(backend.clone())`;
- `Arc<dyn Isolator>` and `Arc<dyn Verifier>` from `Shared::singletons().await`. That is a `tokio::sync::Mutex<Option<Production>>` built at the first command. For `Gix` it reads `box_info`, `workspaces()` → `writer.repos(p)`, `repo_paths(box)`, `app_settings` and `identity::config_root()?/trees` (or the injected root). It builds `GixIsolator::new(IsolatorConfig { repos, scratch_root, copy_exclude: vec![], copy_max_total_bytes: app["copy_max_total_bytes"] or 20 GiB, box_id })` inside `tokio::task::spawn_blocking` (D213: the `git --version` probe and the scratch root's creation are blocking; a `JoinError` maps to the `String` refusal), and `ShellVerifier::new(&limits, Arc::new(MinimalScrubber::new(std::iter::empty())), clock)`, where `limits` is the box row's `command_limits`, falling back to `{"verify":1}` for a missing row or key (and, warned, a value that does not parse). A failed `box_row` read is passed up, so nothing is cached and the next command reads again (D216).
- **Rebuild**: at each `StartRun`, when the repo map read now differs from the one the isolator was built from:
  - no run of this process is live → rebuild, and `isolator_builds += 1`;
  - otherwise → refuse `StartRun` with ``a repo was added or moved since the first run; wait for the live runs to rest (R-39)``.
- Identity: `box_id` from `backend.box_info()` (not `agent_worker.rs:1733`, which is private to T7's file), `user = backend.this_user()`, `app = backend.app_settings()`, `box_profile = backend.box_profile(box_id)` (`None` → "this box has no profile row"), `owner` (one `Uuid::now_v7()` per runtime), `dead_walks: Arc<DeadWalks>`, `clock`, `FirstCandidate`, `scrubber`.
- `ProgressSink { publisher, writer, author }` (D172, D203).
- The driver closure: `agents = backend.agents()` is read once per task into a map. The closure is `|c: &SnapshotCandidate, _k: &SessionKey<'_>| -> Box<dyn AgentDriver>`. It answers the map row's `drivers.driver_for(&agent, on_box.as_ref())`, and `RefusedDriver(err)` for a refusal or an unknown agent. Both parameter types are annotated (blueprint H-18, `engine.rs:247-250`).

`type WorkerEngine<'a> = Engine<'a, Writer, BackendGraphs, dyn Isolator, dyn Verifier, dyn Clock, FirstCandidate, ProgressSink>;`

```rust
/// Plan D155: `GraphSource` is `htui-orch`'s and `Backend` is `htui-store`'s, so `impl GraphSource
/// for Backend` here is E0117 (proven: plan Verified claims). A local newtype is the answer, and it
/// keeps invariant 10 (the orchestrator never names `htui-store`).
#[derive(Debug, Clone)]
pub struct BackendGraphs(pub Backend);
impl GraphSource for BackendGraphs {
    // resolve_graph, phase_agents, prompt_template, agent_boxes → the Backend-inherent read of the
    // same name (backend.rs:454, :468, :487, :501); agent → Backend::agents() filtered, as the
    // MemStore impl does (fake.rs:854-861).
}

/// A driver the registry refused: `start` answers that refusal, which the walk fails as a spawn
/// failure under every gate (engine.rs:4259-4265). `DriverError` is `Clone` (error.rs:14-17).
#[derive(Debug)] struct RefusedDriver(DriverError);

/// D172, D203: `after_done` publishes `SessionDone` for the item and, in a test, writes the step's
/// output document. Production `author` is `None` (MOD-11 writes documents; R-50).
pub struct ProgressSink { publisher: Publisher, writer: Writer, author: Option<Arc<dyn StepAuthor>> }
pub trait StepAuthor: Send + Sync + core::fmt::Debug {
    fn document(&self, item: ItemId, step: &RunStep, phase: &SnapshotPhase) -> Option<NewDocument>;
}
```

### 8.6 Command tasks (commit c: StartRun/answers; d: cancel, promote)

Every task answers its request once at `(origin, seq)` and publishes one `Rested(rest)` or `Error(sentence)` frame for its item, **whatever origin asked** (D200).

| Command | Task |
|---|---|
| `StartRun` | If `singletons` refuse (R-39) → `Failed`. `run = engine.enqueue(..)`, then `Started` frame, then lock `run` + child token, then `engine.claim(run)`. `ClaimRefused` → `queued.insert((queued_at, run))` + `Failed(sentence)`. Otherwise `Orch(Done(Started))`. |
| `AnswerGate`, `RetryStep`, `SelectFanout`, `AcceptArtifact` | lock `run`, then `engine.dispatch(c)`. **D212 (review round, §21):** first, right after `tag(run)`, `AnswerGate`, `RetryStep` and `SelectFanout` are refused through `ctx.refuse` with `chat_free(run_steps(run), live)`'s `ChatLive { step: Some(s) }` while a chat of this process is live on a step `s` of the run. |
| `CancelRun{r}` | D212's refusal first (a cancel does not end the chat: "end that chat first (Chat tab, Esc Esc)"), then `walks.preempt(r)`, lock, dispatch. D179's lease take follows `abandoned`'s release. |
| `PromoteStep{r, s, ..}` | `walks.preempt(r)` when the run has a live walk here, then lock, then dispatch. `Ok(Promoted { opening, .. })` sends `Orch(Promoted{..})` to `(origin, seq)` and then `events.send(RunServed::Attach { addr, promoted })`. `Err` → `Failed`. |
| `Unblock{item}` | `case = engine.unblock_case(item)`. For `FollowRun(r)`/`Resume(r)`: lock `r`, re-check that `unblock_case` still names `r` (else ``the item changed; press `u` again``), then dispatch. |
| `CloseOut{item}` | dispatch (the store refuses a live run in-transaction) |

**Supervision (D158).** Every task is spawned inside a supervisor: `let inner = tokio::spawn(work); tokio::spawn(async move { if let Err(e) = inner.await && e.is_panic() { dead_walks.mark(run); publish(Error("the walk task panicked; the next sweep adopts it")) } })`. A `StartRun` task writes its run id into a `OnceLock<RunId>` the supervisor reads. **Claim retry (M5 D84)**: when any task ends with its run no longer `running`, this process's `queued` runs are re-claimed in `queued_at` order, one task each, under their locks. `Admitted` or a terminal run leaves the set; `ClaimRefused` stays.

### 8.7 Sweep (commit e, D158, D189, D190)

`runs.sweep(backend, tx)` first prunes, in one lazy pass (D214, review round §21): finished task handles, `Walks` parents with no task under them, and `RunLocks` entries whose `Arc` only the map holds and whose mutex is free. Then it returns at once:
- if a sweep task is still running, the tick is skipped;
- if `backend.writer()` is `None`, nothing happens.

Otherwise it spawns one task:
1. `adopted = engine.sweep_fenced(&locks).await`. Its dead-walk pre-pass, under the fence, calls `isolator.release(run)` (warn on error) before `release_lease(run)`, so a panicked walk's `shared_serialized` guards do not block its own adoption (D210). A run that leaves `DeadWalks` through `renew_lease` (a `Resume` before the sweep) has its guards released there.
2. Each `Adopted`:
   - `Walk` → a resume task: lock, child token, `engine.resume(run)` → `Rested`/`Error` frame, then the claim retry;
   - `Parked(rest)` / `Finished(rest)` → a `Rested` frame;
   - `Error(s)` → an `Error` frame.
3. Frames read `run.item_id` through `writer.run(id)`.
4. `sweep_every` is `LeaseTimes::from_app(app).ttl`, read at construction as the default 120 s and replaced by the first sweep's `app` read. The ticker is re-armed on change.

### 8.8 `Action::Promote` (commit h, D165)

- `app/action.rs`: `Promote { run: RunId, step: StepId }`, documented "Emitted by the Runs pane; the shell focuses the tab that drives chats (`App::replay_tab`) and asks for the promotion on its behalf".
- `app/update.rs:28` gains the arm `Action::Promote { run, step } => self.promote(run, step)`, which mirrors `replay` (`:129-136`):
  - `replay_tab` is `None` → status ``no tab can drive a promoted step``;
  - otherwise `update_tab(Focus(tab))` and `dispatch(Origin::Tab(tab), StoreRequest::Orch(OrchRequest::Command(Command::PromoteStep { run, step, chat_open: false })))`.

### 8.9 Harness hook (commit i)

`testkit.rs` gains the fields `runs: Option<RunRuntime>` and `run_events: Option<UnboundedReceiver<RunServed>>`, and `pub fn with_run_runtime(mut self, runtime: RunRuntime) -> Self`. That builder calls `take_events`.

`drive()` (`:180-254`):
- The tuple match gains `(StoreRequest::Orch(_) | StoreRequest::RunStream { .. } | StoreRequest::RunActions(_), _)`.
  - With a runtime: `runs.serve(&backend, &replies.0, &envelope, &live_chats(runtime))`. `Deferred` → `continue`; `Attach` → `on_run_served` (the T6 stub, as in the loop).
  - Without one: `store_worker::serve` (D183).
- After serving, each round runs `runs.settle(CHAT_END)`, panicking with the run ids that did not finish, then `while let Ok(s) = run_events.try_recv() { on_run_served(s) }`, then the chat poll and the reply drain as today.
- `settle()` is unchanged: no runtime, so `store_worker::serve`.

`Served` (`:216-219`) is untouched in T6.

**Walk-test fixture (F-O)**: `run_fixture() -> (MemStore, Arc<FakeAdapter>)` in `run_worker.rs`'s test module:
- `MemStore::demo()`, with every fixture agent disabled;
- one scripted agent row (`transport = cli`, `settings.cli.stream = "fake"`, `tests/chat.rs:40-54`'s shape);
- one `agent_box` row for `store.box_info()`'s box, status ready (rung 3);
- a `DriverFactory` with `cli/fake` → `SharedAdapter`;
- `RunRuntime::with_parts(Arc::new(FakeIsolator::new()), Arc::new(FakeVerifier::default()), factory).with_clock(Arc::new(TokioClock::at(demo_at(..)))).with_author(Arc::new(OutputAuthor))`.

`TokioClock` answers `base + (tokio::time::Instant::now() - start)`, so `start_paused` tests move the lease fence. `OutputAuthor` writes one document of `phase.output_kind` per step. The item under test is `ids::HTUI_ANA_2` (open, no run).

### 8.10 Tests (first failing: `backend_graphs_delegates_each_read`)

`run_worker.rs` unit tests through `spawn_with_runtimes`, unless marked:

| Test | Asserts |
|---|---|
| `backend_graphs_delegates_each_read` (D155) | The five trait reads equal the inherent reads over the seeded memory backend. |
| `an_orch_request_is_served_off_the_loop` (`R-NF-3`) | `StartRun` with a `StallAdapter` (a session whose `next_event` awaits a `Notify`), then `Workspaces`: the `Workspaces` reply arrives first (the shape of `the_loop_answers_other_requests_while_a_probe_is_in_flight`, `store_worker.rs:2208`). |
| `two_commands_on_one_run_are_serialised` (R-27) | A run parked, `RetryStep` whose attempt-2 session stalls, then `AnswerGate(Approved)` on the same run: no reply to the second until the stall is released. It then answers `NotGated`, because the step moved. |
| `cancel_preempts_a_live_walk` (D157, D188) | A stalled `StartRun`, then `CancelRun` → `Done(Cancelled)` promptly. The stalled session was dropped (the adapter's drop flag). The run is `cancelled`, the item `open`, `lease_expires_at <= now`, and `FakeIsolator::releases() >= 1`. |
| `the_sweep_resumes_each_adopted_run_on_its_own_task` | Two runs `running` under a foreign expired lease: the startup sweep adopts both, each is resumed on its own task, and both rest. |
| `a_sweep_skips_a_run_a_command_holds` (D189) | Hold `RunLocks[r]`, sweep: `r` is not recovered and its lease is this owner's. |
| `a_panicked_walk_is_adopted_by_the_next_sweep` (R-12) | A panicking session gives `DeadWalks` the run; the next sweep releases and adopts it, and a healthy script rests it. |
| `a_refused_claim_is_retried_when_a_walk_rests` (M5 D84) | Two items with overlapping `touched_paths`: run 2 gets `Failed("… overlaps run …")`; `CancelRun(run1)` makes run 2 walk. |
| `an_offline_backend_refuses_every_orch_request_with_one_sentence` (D174, PRD `:197`) | `Backend::Offline` over a throwaway `CacheStore`. Each of the 11 `ORCH_NAMES` → `Failed { request: name, message: DATABASE_UNREACHABLE }`, and no task is spawned. `RunStream` → `Subscribed`. `RunActions` answers every command `Err(DATABASE_UNREACHABLE)`. |
| `a_store_outage_fences_the_walk_and_the_sweep_adopts_it_after` (D175, criterion 19 re-scoped) | `start_paused`, `TokioClock`, `lease_ttl_seconds = 30`. A stalled walk; `MemFault::RefreshLease` on; advance: an `Error(LeaseLost…)` frame and `DeadWalks` holds the run. Fault off; the next sweep tick adopts it; attempt 2 exists and rests. |
| `run_stream_frames_carry_the_subscription_seq` (§0a point 3) | Subscriptions `(Tab(backlog), 7, ANA-2)` and `(Tab(chat), 9, FEAT-1)`; a `StartRun(ANA-2)` from `(App, 10)`: every frame is at `(Tab(backlog), 7)` and none at `Tab(chat)`. Re-subscribing at seq 11: later frames carry 11. |
| `a_promotion_reaches_the_attach_hand_off` (T6 stub; in **`store_worker.rs`'s** test module, through `spawn_with_runtimes`) | Parked, then `PromoteStep` from `Tab(chat)`: `Orch(Promoted{..})`, then `Failed { "promote_step", "promotion needs the chat runtime" }`, both at the request's seq. T7 changes the expectation (§10.5). |
| `run_actions_grey_by_the_engine_guards` (D182) | Parked with no document: `approve` is `Err` with `MissingOutputForApproval`'s bytes. After the author writes it: `Ok`. With `LiveChats::of([other])`: `promote` is `Err(ChatLive(None))`. |
| `the_isolator_is_built_once_per_process` (D156) | `RunRuntime::production().with_scratch_root(tmp)` over the fixture: two `StartRun`s give `isolator_builds() == 1`. |
| `cleanup_retries_a_terminal_run` (D177) | A cancelled run: `Cleanup` → `CleanedUp` and `FakeIsolator::cleanups()` +1. On a live run: `Failed(NotTerminal bytes)`. |
| `a_try_serve_without_a_runtime_answers_the_stream_and_the_actions` (D183, unit over `serve`) | `RunStream` → `Subscribed`; `RunActions` → `RunActions`; `Orch` → ``Failed "no run runtime in this build"``. |

`app/update.rs` tests:
- `promote_focuses_the_chat_tab_and_addresses_it`: the recorder tab is the replay tab, and the dispatched envelope is `(Tab(chat), Orch(Command(PromoteStep{..})))`.
- `promote_with_no_chat_tab_says_so`.

### 8.11 T6's snapshot impact

**None.** No view sends the new requests until T8, and `try_serve`'s new arms answer without a status-line error (D183). `cargo insta test -p htui --all-features` must report zero changes.

### 8.12 Commit boundaries (T6)

- (a) The manifests, `lib.rs` `pub mod run_worker;`, `BackendGraphs` and its test.
- (b) The four request variants, the replies, `name()` arms, `try_serve` arms, the types and `actions()`, with D183's test. It compiles and the loop still refuses.
- (c) `RunRuntime::serve`, the tasks for `StartRun` and the answer verbs, and the loop arms. This is where `on_run_served`'s stub lands.
- (d) `RunLocks`, `Walks`, preemption, `abandoned` use, and the promote task.
- (e) Sweep, supervision and claim retry.
- (f) `Publisher` and frames.
- (g) The offline and outage cases.
- (h) `Action::Promote` and `App::promote`.
- (i) The harness hook.

---

## 9. T8: `htui`, the Runs pane (D166–D173, D197–D201)

### 9.1 `DetailTab::captures_input` (OQ-7)

`detail/mod.rs`, in the trait (`:59-75`), mirrors `SettingsSection::captures_input` (`settings/mod.rs:144-149`):

```rust
/// Whether this sub-tab is taking typed text or a modal answer right now, so the Backlog tab must
/// hand it every key before its own navigation (MOD-4 plan OQ-7). Derived from a mode, never a flag.
fn captures_input(&self) -> bool { false }
```

`DetailRegistry` gets `pub fn captures_input(&self) -> bool { self.active().is_some_and(DetailTab::captures_input) }`.

`backlog/mod.rs:203`: the first statement of `BacklogTab::on_key` becomes `if self.detail.captures_input() { return self.detail.on_key(key, ctx); }`. It names no action, which is ANA-2 `:1694-1697`. A capturing pane returns `Pass` for a `CONTROL` chord (hierarchy's carve-out, `settings/hierarchy.rs:674-676`) and `Consumed` for everything else, so `q`, digits and `Tab` cannot leave a typed field.

### 9.2 Pane state

```rust
pub struct RunsTab {
    runs: Vec<RunSummary>, item: Option<ItemId>, scroll: Scroll,
    selected: Option<usize>,              // index into entries()
    actions: Option<ItemActions>,         // D182; None until the first RunActions reply
    subscribed: Option<ItemId>,           // D199
    mode: Mode,
}
enum Entry { Step { run: usize, step: StepId }, Run { run: usize } }   // D198: a stepless run has one Run entry
enum Mode {
    Browse,
    RejectNote { run: RunId, step: StepId, field: TextField },
    ConfirmCancel { run: RunId },
    CloseOut(CloseOutStage),
    Artifact { id: DocumentId, doc: Option<Document>, scroll: Scroll },
}
enum CloseOutStage { Counting, Warn(Preview), Typed { preview: Preview, field: TextField }, InFlight }
```

`captures_input() = !matches!(self.mode, Mode::Browse)` (D201). `selected_step()` keeps its meaning: the step of a `Step` entry, else `None`, so `Enter`'s replay is unchanged.

### 9.3 Keys (D168), all in `Browse`

Each key reads its verdict from `self.actions`:
- `None` → `ctx.emit(Action::Error("the run actions have not loaded yet"))`.
- `Err(s)` → `ctx.emit(Action::Error(s))`, and nothing is sent.
- `Ok` → the emit below. Every emitted `Orch` request goes through `ctx.request`, so it is stamped with the Backlog tab's origin.

| Key | Entry | Verdict | Emits |
|---|---|---|---|
| `a` | step | `steps[s].approve` | `Orch(Command(AnswerGate { run, step, Approved }))` |
| `x` | step | `reject` | `RejectNote`. `Enter` with a non-empty trimmed note → `AnswerGate { Rejected { note } }`, back to `Browse`. Empty → notice `a rejection needs a note`. `Esc` → `Browse`. |
| `r` | step | `retry` | `RetryStep { run, step }` |
| `p` | step | `promote` | `ctx.emit(Action::Promote { run, step })` |
| `c` | any | `runs[r].cancel` | `ConfirmCancel`. `y` → `CancelRun { run }`; `n`/`Esc` → `Browse`; anything else is swallowed. |
| `o` | step | `open` (`Ok(id)`) | `Artifact { id, doc: None }` + `Document(id)` |
| `s` | step | `select` | `SelectFanout { run, position, attempt, winner: step }` |
| `u` | — | `actions.unblock` | `Unblock { item }` |
| `A` | step | `accept` | `AcceptArtifact { run, step, chat_live: false }` (the worker overwrites it, D185) |
| `R` | — | `actions.run` | `StartRun { item, mode: Manual, repo_scope: None }` |
| `C` | — | `actions.close_out` | `CloseOut(Counting)` + `Orch(CloseOutPreview { item })` |
| `T` | any | `runs[r].cleanup` | `Orch(Cleanup { run })` |
| `J`/`K`, `PageUp`/`PageDown`, `Enter` | | | unchanged (`detail/runs.rs:200-213`) |

**Close-out (D167, MOD-15's shape, `settings/hierarchy.rs:666-732`)**:
- `Counting`: `Esc`/`n` → `Browse`.
- On `Orch(CloseOutPreview(p))` → `Warn(p)`. The pane shows `close <key> · <runs> runs · <rows> commit rows · summary v<version>`, then `y continue · n cancel`.
- `Warn`: `y` → `Typed { field: TextField::new() }`; `n`/`Esc` → `Browse`; anything else is swallowed.
- `Typed`: the pane shows `type <key> to close it: <field>`.
  - `Submit` with `field.text() == Some(&preview.key)` → `InFlight` + `Orch(Command(CloseOut { item }))`.
  - `Submit` with any other text → `field.clear()` + notice `that is not the item's key`.
  - `Cancel` → `Browse`. `Consumed`/`Pass` → nothing (`n` is a letter here).
- `InFlight`: every key is swallowed until `Orch(Done(ClosedOut))` or `Failed { "close_out" }` → `Browse`.

### 9.4 Step rows at 43 columns (D169, D170, D197)

The pane renders one `Paragraph` of pre-formatted `Line`s, not a `Table`, because run rows and step rows need two different grids:

```rust
fn header_lines(theme) -> [Line; 2];                       // "kind   status    box          started" / "mode … finished"
fn run_lines(run: &RunSummary, theme) -> Vec<Line>;        // 2, +1 when `failure` (fit to 43 with …)
fn step_lines(step: &RunStepSummary, siblings: &[RunStepSummary], on_cursor: bool, theme) -> [Line; 2];
fn fit(text: &str, width: usize) -> String;                // char-counted, `…` on a cut, space-padded
```

- **Run header lines stay byte-identical** to today's `6 / 9 / 12 / 11` grid with single spacing: `format!("{:<6} {:<9} {:<12} {:<11}", …)` (`detail/runs.rs:258-278`, `:318-333`).
- **Step line 1**: `{cursor}{sp}{slot:<5}{sp}{status:<10}{sp}{phase:<11}{sp}{usage:>6}{sp}{duration:>5}` = 1+1+5+1+10+1+11+1+6+1+5 = **43**.
  - `cursor` is `▸` or a space.
  - `slot` is `p.a`, plus `/i` when any sibling at `(position, attempt)` has `fanout_index != 0` (`/j` for the judge, `-1`). It is `fit` to 5.
  - `status` is `StepStatus::as_str()` except `awaiting_approval`, which renders `awaiting`.
  - `phase` is `fit(phase_name, 11)`.
- **Step line 2**: `{8 spaces}{gate:<10}{sp}{tail:<24}` = **43**.
  - `gate` is `approved`/`rejected`/`retried`/`skipped`/`—`, followed by `*` when `promoted_at` is set and `✓` when `selected == Some(true)`.
  - `tail` is `fit([indicator(step) + " "] + agent/model, 24)`, where agent/model is `agent_name` or `—`, then `/`, then `model` or `—`.
  - The tail starts at column 19, which is the phase column. `a_step_with_a_trim_record_takes_two_lines`'s same-column assertion (`:492-497`) still holds.
- `PHASE_WIDTH` becomes 11. `the_figure_is_thousands_with_a_bang_when_trimmed`'s bound holds over `i32::MIN` (`~-2147.5M !` is 11).
- **Usage (D170)**, at most 6 characters, right-aligned:
  - the document is parsed with `serde_json::from_value::<UsageTotals>` (`usage.rs:24-38`);
  - `cost_micros > 0` → `$0.42` / `$12.34` below $100, `$123` below $100 000, else `>$99k`;
  - otherwise `input_tokens + output_tokens > 0` → `812` / `12k` / `1.2M` / `>999M`;
  - otherwise `—`.
- **Duration**, at most 5 characters, right-aligned:
  - `finished_at - started_at` → `45s`, `12m`, `1h04`, up to `99h59`, else `>99h`;
  - a negative span renders `0s`; a missing `finished_at` renders `…`; a missing `started_at` renders `—`.
  - No clock is read in `render` (`testkit.rs:3-7`).

Sample for the fixture's `FEAT-1` (usage and agent illustrative; the re-record is the authority):

```
kind   status    box          started
mode                          finished
graph  done      DESKTOP-HTUI 09-02 08:00
manual                        09-02 12:00
▸ 0.1   done       prd              —  1h00
        —          claude/sonnet
  1.1   done       plan             —  1h00
        —          claude/sonnet
  2.1   done       implement        —  1h00
        —          ~36k ! claude/sonnet
```

### 9.5 Replies (D171, D172, D198, D199)

`on_reply`:

| Reply | Effect |
|---|---|
| `Runs(runs)` | Replace the rows; keep `selected` on the same entry id when it is still there, else the first entry (the first reply still lands on the first step, which keeps the existing test green). Then, if `self.item != self.subscribed`, request `RunStream { item }` and set `subscribed`. Always request `RunActions(item)`. |
| `RunActions(a)` where `a.item == self.item` | Store it. |
| `RunStream(f)` where `f.item == self.item` and the kind is not `Subscribed` | Request `Runs(item)`. |
| `Orch(_)`, or `Failed { request }` with `ORCH_NAMES.contains(request)` | Request `Runs(item)` (D171). The close-out and cancel mode transitions of §9.3 also happen here. |
| `Document(doc)` in `Artifact` | Set `doc`. `None` → notice `the document is not on this box`. |
| `Failed { request: "document" }` | `Browse`. |

`on_item_change` resets `mode` to `Browse` and clears `actions`. It leaves `subscribed` alone: the next `Runs` reply re-subscribes.

**Artifact view** (read-only, D173): a line `<kind> v<version> · <title>`, then the body wrapped, with `J`/`K`/`j`/`k`/`PageUp`/`PageDown` scrolling and `Esc` closing. Everything else is swallowed.

### 9.6 Tests (first failing: `every_step_row_fits_forty_three_columns`)

`detail/runs.rs` unit tests. They use the existing `Shell` ctx with hand-built `ItemActions` over the fixture rows. Every binding test asserts the emitted action for an enabled verdict, and `Action::Error(sentence)` with nothing else for a disabled one:

- `a_approves_a_parked_step`, `x_asks_for_a_note_then_rejects`, `r_retries`, `p_emits_promote`, `c_asks_then_cancels`, `o_opens_the_artifact_read_only`, `s_selects_a_candidate`, `u_unblocks`, `shift_a_accepts_the_artifact`, `shift_r_starts_a_run`, `shift_c_counts_then_asks_for_the_key`, `t_retries_the_cleanup` (criterion 21's reachability, `docs/ANA-2.md:2144`).
- `a_refusal_re_reads_the_runs` (D171): a `Failed { "retry_step", .. }` reply leads to a `Runs(item)` request.
- `a_run_stream_frame_re_reads_the_runs` (D172); `the_subscription_ack_does_not_re_read`.
- `the_first_runs_reply_subscribes_once_and_asks_for_the_actions` (D199).
- `a_re_read_keeps_the_cursor_on_its_step` (D198).
- `every_step_row_fits_forty_three_columns` (D169). Every fixture step, plus synthetic extremes, is rendered through `step_lines`:
  - `research:judge` at position 123, attempt 45, fanout -1;
  - `awaiting_approval` with `promoted_at` and `selected`;
  - `prompt_tokens = i32::MIN` with a trim;
  - a 40-character agent name and a 60-character model;
  - `cost_micros = i64::MAX`, tokens `i64::MAX`, and a 1000 h span.
  - Every line is **exactly 43** wide (`Line::width()`). Status starts at column 8, phase at 19, usage at 31 and duration at 38.
- `a_run_with_a_failure_gets_a_third_line_that_fits`.
- `a_running_step_shows_no_duration` (D170): `…`.
- `usage_and_duration_cells_never_exceed_their_width` (domain sweep).
- `every_step_takes_two_lines` replaces `a_step_without_one_stays_one_line` (F-L).
- `a_parked_fanout_shows_its_candidates_and_s_picks_one` (M4 OQ-4's human path).
- `the_close_out_key_must_match_and_other_keys_are_swallowed` (D167).
- `captures_input_follows_the_mode` (D201).

`backlog/mod.rs` test: `a_capturing_sub_tab_gets_the_navigation_keys`. A `CapturingProbe` `DetailTab` (the `tests/settings.rs:868` precedent), placed in a `BacklogTab { detail, .. }` built in-module, receives `j`, `G`, `l`, `[` and `Enter`, and the list cursor does not move.

`tests/backlog.rs`, harness with `with_run_runtime` where a runtime is needed:
- `x_note_letters_do_not_move_the_list`
- `the_runs_pane_greys_a_key_with_the_guard_s_sentence`: the status line shows the sentence, and no `Orch` is sent.

### 9.7 Snapshots (T8)

| Snapshot | Change |
|---|---|
| `backlog__detail_runs.snap` | **re-record**: two lines per step (§9.4) |
| `replay__runs_step_selected.snap` | **re-record**: the same rows, with the cursor on `plan` (`tests/replay.rs:251`) |
| `backlog__empty_runs.snap` | **must not change** (F-K) |
| `backlog__runs_closeout_warn.snap` (new) | `C` on a `done` item with no live run (`ANA-1`): the counts line |
| `backlog__runs_closeout_typed.snap` (new) | `y`, then `ANA` typed |
| `backlog__runs_artifact.snap` (new) | `o` on a step with an output document (written in the test) |
| `backlog__runs_reject_note.snap` (new) | `x` on a parked step (a parked run seeded through the runtime), then `needs work` typed |

Every other `backlog__*`, `replay__*`, `chat__*`, `shell__*`, `prompt_preview__*` and `settings__*` snapshot must be unchanged. `cargo insta review` must show exactly the six changes above.

### 9.8 Commit boundaries (T8)

- (a) `captures_input` seam + the Backlog guard + its test.
- (b) The row renderer (`fit`, `step_lines`, `run_lines`) + the width pins + the rewritten one-line test. The two snapshots are re-recorded here.
- (c) `ItemActions`-driven keys `a`/`r`/`p`/`s`/`u`/`A`/`R`/`T`/`o`, with their tests.
- (d) The modes: `x`, `c`, `C` and the artifact view, with the modal tests.
- (e) Subscription and re-reads (D171, D172, D198, D199).
- (f) The harness tests and the four new snapshots, reviewed.

---

## 10. T7: `htui`, the Chat tab drives a promoted step (D164, D165, D205, D206)

### 10.1 `agent_worker.rs`

```rust
/// D205: what a chat session records against.
enum ChatBinding {
    /// MOD-2's own chat: a `run(kind='chat')` minted at start, closed at the end.
    Fresh(ChatRunSpec),
    /// D165: a promoted graph step. No run is minted and none is closed: the step's status is the
    /// engine's (`awaiting_approval`, promoted) and stays so when the session ends.
    Promoted { step_id: StepId, tail: Vec<SessionEvent> },
}

impl AgentRuntime {
    /// D206: the steps whose chat task is still running (its command channel open).
    pub fn live_steps(&self) -> Vec<StepId>;

    /// D165: `RunServed::Attach`'s other half. `start`'s checks (`:1145-1335`) minus the mint,
    /// plus the tail: writer (refused offline), box, user, the opening agent's row (enabled),
    /// `driver_for`, settings, project caps, quota latch, then `writer.step_events(step)` (`None`
    /// → "the step's log is not on this box"). The spec is the promoted step's: `cwd` and
    /// `extra_dirs` from the opening, `resume: Some(ref)` on `OpeningPath::Resume`. Answers
    /// `Served::Start { step_id, task: run_chat(args) }` or `Served::Reply(Failed { "promote_step", … })`.
    pub async fn attach_promoted(&mut self, backend: &Backend, replies: &mpsc::UnboundedSender<ReplyEnvelope>,
                                 addr: ReplyAddr, promoted: crate::run_worker::Promoted) -> Served;
}
```

`ChatArgs.chat: ChatRunSpec` becomes `binding: ChatBinding`. The changes in `run_chat` (`:2310`):
- `step_id = binding.step_id()`.
- On a spawn failure: `Fresh` → `close_run(Failed)` as today; `Promoted` → the frames only.
- After `ChatAccepted`:
  - `Fresh` → `Recorder::new` + `record_prompt` (today);
  - `Promoted` → `Recorder::continuing(&writer, &scrubber, step_id, retain_raw, Some(ui_tx), &tail)` (T2) + `record_follow_up(&opening_text, now)` + `frames.local("follow_up", ..)`.
- At the end: `Fresh` → `close_run(status)`; `Promoted` → nothing.

`finish_chat_run` is never called for a graph step.

### 10.2 `chat/mod.rs`

- A new field `promoted: Option<PromotedHeader { step, phase, agent, model, via }>`.
- `on_reply`:
  - `Orch(OrchReply::Promoted{..})` → set `promoted`, reset `transcript`, set `session = None` and `pending_start = true`. No live chat exists, because the promotion's `chat_open` guard refused otherwise (D185).
  - The existing refusal arm widens to `request.starts_with("chat_") || request == "promote_step"` (`:490-493`).
- `header()`: when `promoted` is set, it renders `promoted · <phase> · <resumed|handoff> · <agent> · <model> · session <ref>`. Otherwise it is **unchanged**, so no existing `chat__*` snapshot moves.
- `submit` is unchanged: a live promoted session takes `ChatSend { step_id }` like any chat.
- `Esc Esc` ends it through `ChatCancel`, and the step stays `awaiting_approval`.

### 10.3 `store_worker.rs` and `testkit.rs`

- `on_run_served`'s `Attach` arm becomes `match runtime.attach_promoted(backend, tx, addr.clone(), *promoted).await { Served::Start { step_id, task } => runtime.attach(step_id, tokio::spawn(task)), Served::Reply(r) => send(addr, r), Served::Deferred => {} }`.
- `live_chats(&runtime)` becomes `LiveChats::of(runtime.live_steps())`.
- The harness does the same, pushing `Start`'s task into `chats` instead of spawning (`testkit.rs:204-212`'s shape).

### 10.4 The promotion data flow, end to end

| # | Where | What |
|---|---|---|
| 1 | Runs pane | `p` → `Action::Promote { run, step }` |
| 2 | `App::promote` | focus the Chat tab; `dispatch(Tab(chat), Orch(Command(PromoteStep { chat_open: false })))`, seq *n* |
| 3 | loop → `RunRuntime::serve` | `chat_open := !live.is_empty()`; spawn the promote task → `Deferred` |
| 4 | task | `walks.preempt(run)` if a walk is live here → the walk task drops its walk (agent killed) → `engine.abandoned(run)` releases the guards and the lease |
| 5 | task | lock `run` → `engine.dispatch(PromoteStep)`: take lease → `running → awaiting_approval` (if so) → `promote_step` → note → release → opening |
| 6 | task | `tx: (Tab(chat), n, Orch(Promoted{..}))`; `publish(item, Rested(parked))` to the Runs pane's `(Tab(backlog), m)`; `events: RunServed::Attach { addr: (Tab(chat), n), promoted }` |
| 7 | loop `run_events` arm → `on_run_served` | `runtime.attach_promoted(..)` → `Served::Start` → spawn + `attach(step_id)` |
| 8 | session task | `driver.start(spec{cwd, resume?}, opening_text)` → `ChatAccepted` at `(Tab(chat), n)` → continuing recorder → `follow_up` at `turn + 1` → turns |
| 9 | Chat tab | header `promoted · …`; `ChatSend`/`ChatAnswer`/`ChatCancel` keyed by `step_id` drive the step unchanged |
| 10 | later | `Esc Esc` ends the session, and the step stays promoted. On the Runs pane, `A` runs `AcceptArtifact` (with `chat_live` now false): stage 5, then `approved`, then the walk resumes at `position + 1` |

### 10.5 Tests (first failing: `promotion_opens_the_chat_on_the_same_step`)

`tests/chat.rs`, over a harness with both runtimes and `RunRuntime::with_parts(..).with_author(..)`. `StartRun(ANA-2)` is dispatched as `Action::Store` and parks at `prd`, then `Action::Promote` is applied directly, as the replay cases do (`tests/chat.rs:561`):

| Test | Asserts |
|---|---|
| `promotion_opens_the_chat_on_the_same_step` | `ChatTab::session().step_id == prd`, and the header says `promoted · prd · handoff`. |
| `a_follow_up_lands_at_the_next_turn_of_the_step` (criterion 17) | After a composed message, the `session_event` rows of `prd` have strictly increasing `turn`, all on one `run_step_id`. |
| `promotion_writes_no_chat_run` | No `run` of kind `chat` in the store. |
| `a_promoted_session_end_leaves_the_step_awaiting` | After `Esc Esc`, the step is `awaiting_approval` with `promoted_at` set and the run is `awaiting_approval`. |
| `the_handoff_opening_is_one_follow_up_row` (ANA-5 criterion 18) | One `prompt` row, at seq 0; one `follow_up` at `turn = last + 1` carrying the handoff text; `prompt_digest` unchanged. |
| `accept_is_refused_while_the_promoted_chat_is_live` | `AcceptArtifact` gives the `ChatLive(Some)` sentence; after `Esc Esc` it gives `Done(Accepted)` and `plan` exists. |
| `a_second_promotion_is_refused_while_a_chat_is_open` (D185) | `ChatLive(None)`'s sentence on the status line. |
| `promoting_a_running_step_preempts_its_walk` | A `StallAdapter` session live: promote. The stalled session is dropped, and the chat opens on the same step. |

`agent_worker.rs` unit tests:
- `attach_promoted_resumes_a_cli_step_with_its_banner`: `SpecSpy` (`:3291`) sees `spec.resume == Some(banner ref)`.
- `a_promoted_chat_never_closes_a_run`.
- `live_steps_drops_an_ended_chat`.

`store_worker.rs` test `a_promotion_reaches_the_attach_hand_off` (written by T6, §8.10) changes its expectation from the stub's `Failed` to `ChatAccepted` at the request's seq. It lives in `store_worker.rs`, which T6 and T7 both own, serially. Nothing in `run_worker.rs` asserts the stub, because that file is not in T7's set.

**Snapshot**: one new `chat__chat_promoted.snap`, named `"chat_promoted"` in `insta::assert_snapshot!`, which is the file's existing naming. No existing `chat__*` changes.

### 10.6 Commit boundaries (T7)

- (a) `ChatBinding` + `live_steps` + the unit tests (green; `Fresh` only).
- (b) `attach_promoted` + the `Promoted` binding in `run_chat`.
- (c) Loop and harness: stub → attach, `live_chats` swap.
- (d) Chat tab header + refusal arm + `tests/chat.rs` cases + snapshot.

---

## 11. T9: Postgres end to end (criteria 17 and 20, the new edge)

`crates/htui/tests/runs_pg.rs` (new, `#![cfg(feature = "testkit")]`):
1. `let Some(db) = testkit::demo_db().await else { println!("{}", testkit::SKIP); return; }` (`htui-store/src/testkit.rs:32`, `:170`).
2. `CacheStore::open(tmp, "runs-pg", 1)`, then `Backend::Online { pg: db.store.clone(), cache }` (`htui-store/tests/connect.rs:167-174`'s shape).
3. `Harness::over_backend(..).with_run_runtime(..).with_agent_runtime(..)`, with F-O's candidate seed written through `db.store`.
4. `db.drop_db().await` at the end.

| Test | Asserts |
|---|---|
| `a_promoted_step_continues_its_own_log_on_postgres` | Criterion 17 end to end: one `run_step` id, `follow_up` rows at an increasing `turn`, `promoted_at` set, no chat run, `prompt_digest` unchanged, and `usage` equal to the pre-promotion sum plus the chat's (raw `SELECT usage FROM run_step WHERE id = $1`, `chat_usage_pg.rs:118-126`'s runtime-checked form). |
| `close_out_is_one_transaction_on_postgres` | Criterion 20: a refused close-out (live run) writes no document; a closed one writes one `summary` and sets `closed_at`. |
| `unblock_follows_an_escalated_run_on_postgres` | T1's edge on `PgStore`: `blocked → awaiting_approval` through `Unblock`. |

---

## 12. Count pins that move

| Pin | Now | After | Where | Task |
|---|---|---|---|---|
| `htui-orch` `CASES` | 52 | **68** | `conformance.rs:292`; `cases_are_unique_and_fifty_two` `:4307` (name, literal, message); `tests/fake_conformance.rs:15-16` | T4 |
| `RunFailure` `Display` rows | 11 variants | 12 | `status.rs`, `run_failure_display_is_ana2s_bytes` | T4 |
| `StoreRequest` variants | 58 | **62** | `store_worker.rs:79-499` (unpinned) | T6 |
| `ORCH_NAMES` | — | 11 | `run_worker.rs` (pinned by a test that each is a distinct `name()`) | T6 |
| store `CASES` / `READ_CASES` / `.sqlx` | 53 / 9 / 227 | unchanged | plan table | — |

---

## 13. Data flow

### 13.1 Criterion 3 at both call sites (D162, D195)

| # | `walk_step` (`fan_out = 1`) | `drive_group` (`fan_out > 1`) |
|---|---|---|
| 1 | `pending → running` (`move_step`); prepare; trees; `before` commits | admit missing indices; base read; nothing is live |
| 2 | `assemble_prompt` → `Ok(Err(StageThree::Refused(e)))` | the same, once for the group |
| 3 | `running → failed` (no `gate_note`) | each pending: `pending → running → failed` (`fail_candidate`) |
| 4 | item `in_progress → blocked`; note ``prompt refused at `prd`: …`` | the same |
| 5 | `finish_run(Failed, PromptRefused)`; the item stays `blocked` | the same |
| 6 | `cleanup_run`; `Some(Rest { Failed, .. })`; no recorder, no session | the same |
| 7 | `Unblock` → `Reopen` → `open`; `R` starts a new run | the same |

### 13.2 `Unblock`'s three cases (D161)

| Case | Rows before | Writes | After |
|---|---|---|---|
| `Reopen` | item `blocked`, no active run (rung 4, D162) | `transition(Blocked → Open)`, note | `R` enabled |
| `FollowRun(r)` | item `blocked`, run `awaiting_approval` (escalation, `gate.rs:996-997`) | `transition(Blocked → AwaitingApproval)` (T1), note | `p`/`a`/`r`/`c` reach the run (R-4) |
| `Resume(r)` | item `awaiting_approval`, `resumable_park(cursor(r))` (reconcile refusal R-7, D132 leftovers) | note, `resume(r)`: lease → `walk_resumed` → unpark (D180 honours `false`) → reconcile the frontier → walk | run walks on, or parks again with the refusal note |

---

## 14. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | Two worktrees in each wave means two `target/` directories, and disk pressure crashes the dev Postgres (memory). | `df -h /` before each wave. |
| H-2 | `start_paused` tests with `SystemClock`: the heartbeat's fence reads a wall clock that paused time never moves, so it spins. | Every T6/T7 test that moves time uses `TokioClock` (§8.9). |
| H-3 | The first `tokio::spawn` of an `Engine` future. | T4's `a_dispatch_future_is_send` (D204). |
| H-4 | A `select!` branch future borrowing `runs` while a handler uses `runs`. | The event receiver and the sweep ticker are loop locals (§8.3), and branch futures are dropped before handlers. |
| H-5 | The harness's `drive` returning while a walk task is still running photographs a half-walk. | `runs.settle(CHAT_END)` every round, panicking with the run ids. |
| H-6 | A token cancelled while a task waits for the lock. | `select!` on lock versus token, so the task walks nothing (§8.4). |
| H-7 | A preempted walk's `shared_serialized` guard and lease. | `abandoned` (D188), never `walk_leased`'s arms (F-D). |
| H-8 | The pane's typed field and global keys (`q`, digits, `Tab`). | `captures_input` + `Consumed` for all but `CONTROL` (§9.1). |
| H-9 | `Recorder::continuing` is given a tail read from the mirror window. | `attach_promoted` reads the tail through the **writer** (online: Postgres; memory: the store), never the `Backend` read path. |
| H-10 | `CommandOutcome: Eq` and `AssembledPrompt: !Eq` (`prompt/mod.rs:317`). | `OpeningPath::Handoff { text, digest }` (D192). |
| H-11 | The `ItemBlocked` byte change breaks a test that pins the old sentence. | Updated in the same commit (§7.1). |

---

## 15. Workspace gate (after T4, T6, each Wave B merge, and T9)

The plan's Validation block, unchanged. `cargo sqlx prepare --check` stays at 227 files. `cargo doc --workspace --no-deps --keep-going` shows exactly the two baseline errors (`traits.rs:954`, `pg/write.rs:3476`).

---

## 16. What this milestone does NOT do

- No grace window and no parked-permission answers before a preempting kill (R-38). No `follow_up_in_session` promotion path (D163).
- No ACP `session/load` (R-48). No secret wiring (D176). No `CancelStep` (D178), no `GateAnswer::Skipped` verb (D166), no `OpenArtifact` command (D173).
- No `run_step.gate_note` in `RunStepSummary` (R-3). No projection, mirror or `.sqlx` change. No `event_loop.rs` change. No `refresh_lease` status clause (D179). No `queue.rs`.
- No key-help line in the pane (43 columns cannot hold twelve keys). The verbs are in the module doc and the `?` box is unchanged. This is a later refinement.

---

## 17. Gate checks that are not tests

- **C-1**: `grep -rn 'Command::new' crates/htui-orch/src/` hits `isolate/git.rs` (3) and `verify.rs` (1) only.
- **C-2**: `git diff --stat main -- crates/htui-store crates/htui-core/src/store 'crates/**/migrations' .sqlx crates/htui/src/event_loop.rs` is empty.
- **C-3**: `grep -rn 'impl GraphSource for Backend' crates/` is empty. T4 corrects `fake.rs:831`'s sentence to name `run_worker::BackendGraphs`, since `fake.rs` is in T4's set. `graph.rs:45` carries the same wrong sentence but is **outside every task's file set**, so it is recorded for the close-out's doc sweep rather than widening T4.
- **C-4**: `cargo insta test -p htui --all-features` after T6 reports zero changes; after T7, exactly `chat__chat_promoted.snap`; after T8, exactly §9.7's six.
- **C-5**: the backlog guard is one `if` naming no action: `grep -n "captures_input" crates/htui/src/ui/tabs/backlog/mod.rs` gives 1 hit.
- **C-6**: `StoreRequest` has 62 variants, and `name()` returns 11 distinct `ORCH_NAMES`.

---

## 18. Risks (continuing after R-47)

| Risk | Likelihood | Mitigation |
|---|---|---|
| **R-48** The ACP driver ignores `SessionSpec.resume` (only `cli/mod.rs:145`, `:405` read it), so a promoted ACP step always gets the handoff prompt, a fresh model context (F-G). | Certain for ACP rows | D192 routes by transport; the Chat header says `handoff`. Owner: a MOD-2 follow-up for ACP `session/load`. |
| **R-49** A promoted chat works in the step's tree without the `shared_serialized` `(box, repo)` guard. It was released at `capture` (`engine.rs:2435`) or by `abandoned`, so another run may `prepare` the same checkout meanwhile. | Low (needs `shared_serialized` + a second run on the repo) | Recorded. A later refinement re-takes the guard in `attach_promoted`. |
| **R-50** Production `approve`/`accept` are greyed until MOD-11, because `NoSink` writes no `output_kind` document (F-R). | Certain until MOD-11 | The sentence names the missing kind. Tests use D203's author. |
| **R-51** A command on a run with a live walk waits for the whole walk (D157), and the pane shows nothing for the wait. | Low (the pane greys most such keys) | The answer lands when the walk rests. A "waiting" frame is a later refinement. |
| **R-52** An `Engine` future has never been spawned; `!Send` would surface in T6. | Low | D204's compile test in T4. |
| **R-53** `ItemActions` is as of the last `Runs` reply. A verdict can flip before the key. | Medium | The engine re-checks (the same function, D184), and D171 re-reads. |
| **R-54** Between the T6 and T7 merges, `LiveChats` counts an ended-but-unswept chat as live (`caps`, F-H). | Low, and only on the branch | T7's `live_steps`. |

---

## 19. Where the plan and the tree disagree (beyond the plan's own list)

F-A through F-S (§0). The ones the main thread records against ANA-2 or the PRD:
- F-G (ANA-2 `:1199-1206` presumes ACP resume);
- F-R (criterion 21's production reach before MOD-11);
- D192's resume sentence (`RESUME_OPENING`, an `htui`-authored follow-up);
- D194 (`accept` refused on a failed verify, where ANA-2 `:1223-1233` is silent).

---

## 20. Decisions settled in this blueprint (continuing after D180)

| # | Decision |
|---|---|
| **D181** | `RunServed { Reply, Deferred, Attach { addr, promoted } }` in `run_worker.rs`. `Attach` reaches the loop through `RunRuntime::take_events()`'s receiver, a loop-local arm in the **store worker's** `select!`. The loop and the harness route every `RunServed` through one `on_run_served` helper. T6's `Attach` arm answers `Failed { "promote_step", "promotion needs the chat runtime" }`, and T7 replaces it. `agent_worker::Served` is unchanged. |
| **D182** | `StoreRequest::RunActions(ItemId)` → `StoreReply::RunActions(Box<ItemActions>)`. It has its own discriminant, so a long-lived `Orch` reply is not orphaned by greying reads. `StoreRequest` goes 58 → 62. |
| **D183** | `try_serve` answers `RunStream` (a `Subscribed` acknowledgement), `RunActions` (no live chats) and `Document` (a read) without a runtime, and refuses only `Orch`. A runtime-less shell shows no status-line error. |
| **D184** | One pure admission function per verb in `command.rs`, called by the engine after its reads and by `run_worker::actions`: `retry_admitted` (a refactor of `retry_step`'s order), `promote_enabled`, `accept_enabled`, `unblock_enabled`, `close_out_enabled`, `cleanup_enabled`. `start_enabled` is the row-level mirror of `create_run`'s law and is not called by the engine. `snapshot_of` and `phase_at` become public. |
| **D185** | `PromoteStep { chat_open }` and `AcceptArtifact { chat_live }` carry the one fact only the process can know. The worker overwrites them from `LiveChats` before dispatch. A promotion is refused while any chat of this process is live, because the tab has one session. |
| **D186** | `Engine::enqueue` is factored out of `start_run`. The worker enqueues, takes `RunLocks[run]`, registers the token, then claims. There is no item lock: `create_run`'s item compare-and-set admits one. |
| **D187** | One parent `CancellationToken` per run, and a child per task. A preempting command removes and cancels the parent, then awaits the lock. A task cancelled while queued for the lock walks nothing. |
| **D188** | `pub async fn Engine::abandoned(run)` = `isolator.release(run)` + `release_lease(run)`, best-effort. The worker calls it after every token-cancelled walk. |
| **D189** | `pub trait RunFence` + `Engine::sweep_fenced`, implemented by `RunLocks` (`try_lock_owned`). A held run is neither released in the dead-walk pre-pass nor recovered, and its adopted lease serves the holder's own renewal. |
| **D190** | The sweep runs at loop start when the backend has a writer, at each `ConnEvent::Online`, and on a ticker of `LeaseTimes::from_app(app).ttl` (default 120 s), guarded on `writer().is_some()`. One sweep at a time. |
| **D191** | A promotion is always a task: preempt a local walk, lock, dispatch. Then `Orch(Promoted{..})` to the requester, a `Rested` frame to subscribers, and `RunServed::Attach`. A fan-out candidate is not promotable (`PromoteCandidate`). |
| **D192** | `Opening { agent_id, agent_name, model, phase, cwd, extra_dirs, path }`, with `OpeningPath::Resume { session_ref, text }` or `Handoff { text, digest }`. There is no `AssembledPrompt`, and the step's `trim_record`/`prompt_digest` are never written. Resume only when `caps.resume && transport == Cli && banner` (R-48). |
| **D193** | Both openings start the session at once. Resume sends `promote::RESUME_OPENING`, an `htui`-authored sentence recorded as the `follow_up`, so `run_chat` keeps its start → accepted → turns shape. |
| **D194** | `AcceptArtifact`: guard → lease → {verify with a synthetic `Done`, capture, `record_commits`, `finish_step`} in `leased_window`. A `fail` verify refuses with `AcceptVerifyFailed` (the outcome is recorded and the step stays promoted). Otherwise `answer_guarded(Approved)`. |
| **D195** | `assemble_prompt` answers `Result<Result<AssembledPrompt, StageThree>, EngineError>` with `StageThree::{MissingInput, Refused}`. Only `assemble`'s own refusal blocks the item. Both call sites follow `refuse_no_candidate`'s actual order (F-J). `fail_group_before_a_token` gains `(failure, block)`. |
| **D196** | `status::resumable_park` is `walk_resumed`'s predicate, shared with `Unblock` case 3. |
| **D197** | The Runs pane renders pre-formatted `Line`s. Run lines keep today's grid byte for byte. Step line 1 is 1/5/10/11/6/5 with single spaces (43). Line 2 is indent 8 + gate 10 + tail 24 (43). `awaiting_approval` renders `awaiting`. |
| **D198** | Cursor entries are the steps, plus the header of any run with no step. Run-level keys act on the entry's run. A `Runs` reply for the same item keeps the cursor on the same entry id. |
| **D199** | The pane subscribes (`RunStream`) for `self.item` on the first `Runs` reply after an item change, and asks `RunActions` on every `Runs` reply. |
| **D200** | Every command's end publishes a frame for its item whatever origin asked, so the Runs pane sees a promotion issued from the Chat tab. |
| **D201** | `captures_input = mode != Browse`. `CONTROL` chords pass and everything else is consumed. |
| **D202** | The production isolator config is the repos of every project of every workspace, joined on `repo_paths(box)`. The scratch root is `identity::config_root()/trees` (injectable). It is rebuilt only when the set changed and no walk is live. Otherwise `StartRun` is refused (R-39). |
| **D203** | The `StepAuthor` seam in `ProgressSink` writes a step's output document in tests. Production passes `None`. |
| **D204** | T4 pins `Send` for `dispatch`, `resume` and `sweep_fenced` futures with a compile test. |
| **D205** | `ChatBinding { Fresh(ChatRunSpec), Promoted { step_id, tail } }` in `agent_worker.rs`. A promoted session never mints or closes a run. |
| **D206** | `LiveChats` is passed into `RunRuntime::serve`. T6 derives it from `caps` over `steps()`, and T7 from the new `AgentRuntime::live_steps()`. |
| **D207** | Harness: `with_run_runtime`. `drive` settles the runtime's tasks each round and drains its event receiver. `settle()` stays runtime-free. |
| **D208** | The close-out body format and ordering (§5.1). Title `Close-out <key>`. |
| **D209** | `StoreRequest::name` maps `Orch` to eleven per-verb names (`run_worker::ORCH_NAMES`). The status line reads `retry_step: …`, and the pane and Chat tab match on those names. |

---

## 21. Review round (rust-reviewer over `main..001ddef`, 2026-09-24)

`rust-reviewer` returned **request-changes** with seven findings. One adversarial verifier per finding tried to refute it and reproduced what it could: findings 1–5 and 7 are real (three HIGH, three LOW once re-graded), and finding 6 is not a defect as designed. The maintainer answered every design question with the **recommended option** on 2026-09-24. Each behaviour change is test-first: every red commit compiles, and the first failing test is named in §21.2. Pins: orch `CASES` 68 → 70; store `CASES` and `.sqlx` unchanged.

### 21.1 Decisions (D210–D216)

| # | Finding | Decision | Commits (red, green) |
|---|---|---|---|
| **H1** (1) | A panicked walk's `shared_serialized` `(box, repo)` guard lives in the shared isolator's `held` map, not on the walk's stack, so unwinding never drops it. The sweep's dead-walk pre-pass only released the lease, and recovery never releases guards, so the adopted run's resumed walk (and any other run's `shared_serialized` step on that repository) waited on the lock forever. | **D210** `sweep_fenced`'s dead-walk pre-pass, under the fence, calls `isolator.release(run)` (warn on error) before `release_lease(run)`. `DeadWalks::remove` returns `bool`, and `renew_lease` releases the run's guards when the run was in the set, which covers a `Resume` that takes the lease before the next sweep. Every `renew_lease` caller holds the run's lock or the fence. One private helper, `release_guards`, serves both. | `223995e`, `073432a` |
| **H2** (2) | `accept_artifact`'s verify measured the phase deadline from `step.started_at`, so every accept more than `deadline_seconds` (default 7200 s) after the agent began got `remaining = 0`, an `Unavailable` verify that never ran, and merged unverified. | **D211** The accept's `VerifyStage` uses `started_at: now`: a fresh copy of the phase deadline, measured from the accept (a hung command still cannot hold the lease forever). `Unavailable` is not refused (D30; some causes are permanent): it is recorded, and an `item_note` `accept: verify unavailable: <reason>` is written in the same window as the `Fail` note. §7.6 amended. | `869d96e`, `e14941a` |
| **H3** (3) | Only `PromoteStep` and `AcceptArtifact` read the live-chat fact. Approve, reject, retry, select and cancel went through on a run whose promoted step was still chatted with: the next phase ran under the chat, a reject or cancel removed the worktree under the live agent, and the chat kept writing to a closed or superseded step. | **D212** (option a, `htui` only) `run_worker` refuses `AnswerGate` (approve and reject), `RetryStep`, `SelectFanout` and `CancelRun` with `EngineError::ChatLive { step: Some(live_step) }` while any step of the **same run** has a live chat (D185 allows one per process). The refusal is inside the task, after `tag(run)` and before any preemption, through `ctx.refuse`, so its D200 `Error` frame is published. Cancel refuses ("end that chat first (Chat tab, Esc Esc)"); it does not end the chat. One pure helper, `chat_free(steps, live)`, is used by the task and by `verdicts`, so the Runs pane greys `a`/`x`/`r`/`s`/`c` with the same sentence (D182, D184). §8.2 and §8.6 amended. | `91a0865`, `567e4e9` |
| **L4** (4, re-graded from MEDIUM) | `Shared::singletons` ran `GixIsolator::new` — a `git --version` probe busy-polled for up to `PROBE_TIMEOUT` (10 s), `create_dir_all` and path canonicalisation — on a runtime worker thread. | **D213** `GixIsolator::new` runs inside `tokio::task::spawn_blocking`; a `JoinError` maps to the `String` refusal. Nothing is assigned until it answers. The docs on `GixIsolator::new` (`real.rs`), `PROBE_TIMEOUT` and the hung-probe test (`git.rs`) no longer say "at worker start". | `59b1734` (no red: a test needs a slow `git` on `PATH`) |
| **L5** (5, re-graded from MEDIUM) | `Shared.tasks` kept one finished sweep handle (≈17 KB) per idle tick until the next `serve`; `Walks.parents` and `RunLocks` entries were never removed. | **D214** Lazy pruning: one pass per sweep tick (`Shared::prune`, at the top of `RunRuntime::sweep`) over finished task handles, `Walks` parents with `live == 0`, and `RunLocks` entries with `Arc::strong_count == 1` whose mutex `try_lock` succeeds, each under its map's own std mutex (the one `child()` and `entry()` mint under). `tasks_len`'s doc names the sweep. §8.4 and §8.7 amended. | `9233fdb`, `1c92c2e` |
| **(6)** | `RunActions` is served inline on the store loop with 4 + 2N reads (`PgStore::runs` is two statements), on every Runs reply and Backlog cursor move. | **Not a defect as designed** (§8.2 and §8.5 chose the read shape and inline serving; R-NF-3 is about the UI thread); **changed by maintainer choice (D215)**. **D215** `serve` answers `RunActions` from a task tracked through `shared.track`, like `Orch` commands, returning `RunServed::Deferred`; the task answers at `envelope.seq` through the reply channel, over clones of the backend, the sender and `LiveChats`. `App::is_fresh` drops an overtaken reply. `try_serve`'s runtime-less path stays inline (D183). The offline unit case reads the verdicts from the channel; §8.5's table amended. | `6c066f4`, `8c0c1e4` |
| **L7** (7) | `command_limits` turned a `box_row` store error (and a missing or unparseable value) into `{"verify":1}` silently, and `singletons` cached the verifier built from it for the life of the process. | **D216** `command_limits` returns `Result`: a `box_row` error propagates in `singletons` like the reads beside it, so nothing is cached and the next command retries. The `{"verify":1}` fallback stays for a missing row or key (D156); a stored value that does not parse falls back too, with a `tracing::warn!` (the only warning). | `1489e2d`, `9bdf218` |

### 21.2 Tests (first failing test per decision)

- **D210** (`htui-orch` engine unit, `graph.rs` untouched: FEAT-3 is re-pointed at `SharedSerialized` through `Harness::repoint`, and the walk is dropped mid-session with its guard held): `a_dead_shared_serialized_walk_is_adopted_and_walks_on` (the sweep adopts, `releases() == 1`, the resume parks at `prd` within 5 s) and `a_resume_before_the_sweep_releases_a_dead_walks_guards` (after `take_lease` the run leaves `DeadWalks` with `releases() == 1`; another run's `SharedSerialized` prepare finishes within 5 s; the resume completes). With the count assertions removed, red timed out instead, so the hang itself is covered.
- **D211** (conformance, `CASES` 68 → 70; `cases_are_unique_and_seventy`, `fake_conformance::cases_len_is_seventy`): `accept_artifact_verifies_on_a_deadline_from_the_accept` (3 h past the deadline the verify gets the full 7200 s window; red 0 ns) and `accept_artifact_notes_an_unavailable_verify`.
- **D212**: `tests/chat.rs` `the_run_s_verbs_are_refused_while_the_promoted_chat_is_live` (approve, reject, retry, select and cancel each refused with `<name>: step X is being chatted with; end that chat first (Chat tab, Esc Esc)`; nothing moves; the chat stays live; after `Esc Esc` the approve goes through); `run_worker` units `a_verb_on_a_run_being_chatted_with_is_refused_and_published` (the D200 `Error` frame reaches the item's subscriber) and `run_actions_grey_the_run_s_verbs_while_a_chat_is_live_on_it` (a live chat on a step of no run of the item greys nothing, `select` included; the fixture holds one run of the item, so a chat on another run of the same item is not pinned).
- **D214**: `a_sweep_prunes_finished_tasks_idle_parents_and_free_locks` (a live parent and a held lock stay).
- **D215**: `run_actions_are_answered_from_a_tracked_task` (one tracked task, answered once at origin and `seq`, a refusal included); `an_offline_backend_refuses_every_orch_request_with_one_sentence` reads the verdicts from the channel.
- **D216**: `command_limits_fail_with_the_store_and_default_a_missing_value` (offline `box_row` is an error; no row, no key and `"many"` give `{"verify":1}`; a stored map is read). The red commit changed the signature first, with the body still swallowing the error, so the test compiles.

### 21.3 Risks

| Risk | Likelihood | Handling |
|---|---|---|
| **R-55** (D216) `box.settings.command_limits` is read once per process (per server): an edit to it does not reach a running process's verifier until a restart or a server switch. Nothing edits it today. | Very low (no code path writes it) | Recorded. Whoever adds an editor re-reads the limits or rebuilds the verifier when no walk is live. |

### 21.4 As landed

- **D210**: the Resume-before-sweep case is tested through `engine.take_lease` (Resume's first step) followed by a full `harness.resume`: resuming a still-running step prepares the same `StepId` again, and the fake drops that step's own guard, so another run's prepare shows the repository is free. `223995e` fails `cargo fmt --check` on its own (one `assert_eq!`); `073432a` fixes it. Left unsquashed.
- **D212**: the Runs pane needed no change (`allowed()` reads the verdicts), but its verdicts had to be refreshed when the chat ends (§21.5, V2). `cancel_enabled`'s doc (`command.rs`) names the new refusal. The window accepted for `accept_enabled` remains: a verb served just before a queued `PromoteStep` attaches its chat is not caught. A failed `run_steps` read in the check refuses the verb with the store's sentence.
- **D216**: the `singletons` propagation has no store-level test (`MemStore` has no `box_row` fault, and `htui-core` was left untouched); the unit test pins `command_limits` itself.

**Gate** at `9bdf218` (§15):
- `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` (Postgres): 1838 passed, 8 failed, 26 ignored. All 8 were dev Postgres in recovery mode (`57P03` at `testkit.rs:118`) under load; `chat_usage_pg` (2), `connection` (50), `prompt_settings` (42) and `runs_pg` (3) each pass re-run alone. `htui` lib 240, `htui-orch` lib 411, `chat` 26.
- `clippy --workspace --all-features --all-targets -D warnings` and `fmt --check` clean.
- Workspace rustdoc shows only the two baseline errors (`htui-core` `MIRRORED_TABLES`, `htui-store` `step_exists`).
- `cargo insta test -p htui --all-features`: 566 passed, nothing pending.
