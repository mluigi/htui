# Plan: MOD-37 milestone 5 - ACP resume

**Source PRD**: `.claude/prds/mod-37-orchestrator-hardening.prd.md`
**Selected Milestone**: 5 - ACP resume (R-48, ANA-27 §5.1 T5)
**Complexity**: Medium
**Routing**: PRD path, M5 planned on its own (maintainer-confirmed 2026-10-03); ultracode not needed.
**Status**: confirmed 2026-10-03 (fact-checked: 16 claims, 8 ✓, 8 amended); blueprint `.claude/plans/mod-37-acp-resume.blueprint.md` (amendments A-1..A-11, A-7 decided: latest banner); implementing

## Summary
A promoted ACP step resumes its own agent session instead of always getting the handoff prompt.
The ACP driver finally reads `SessionSpec.resume`: it calls `session/resume` when the agent
advertises it, otherwise `session/load` with the history replay discarded, and refuses if the agent
advertises neither. A resume that fails, on either transport, is reported and then falls back to
the handoff prompt in the same bind, never silently. Every promoted chat records how it opened in a
new `run_step.opening` column (`resumed` | `handoff` | `resume_failed`). The Runs pane shows
"context not carried; handoff prompt only" for the latter two, and the Chat tab header follows the
real outcome (ANA-27 T5).

## Maintainer decisions (2026-10-03)
| Question | Decision |
|---|---|
| R-48 scope | **Full ACP resume plus the T5 note.** R-48 closes; nothing is re-deferred. |
| ACP call and replay | **`session/resume` first, `session/load` fallback.** Resume when `agentCapabilities.sessionCapabilities.resume` is advertised and `settings.acp.session.resume` allows it. Otherwise load when `loadSession` is advertised and `settings.acp.session.load` allows it, discarding every update queued before the load response, so the step log gains no duplicate rows. |
| A requested resume that fails | **Report, then a labelled handoff in the same bind.** The failure sentence is recorded as an `other` row (`resume_failed`) in the step's own log, which the chat shows as it arrives, and the chat then opens with the handoff prompt, marked "context not carried; handoff prompt only". No silent fresh start and no dead end. A failed resume used to leave the step promoted with no chat, and a second promotion chose `Resume` again. *(Fact-check amendment: a step-log row instead of an item note. `ChatArgs` and `Promoted` carry no item or user for `NewNote`, and the row is durable, scoped to the step, and reaches the tab through the existing `ChatFrame::Event`.)* |
| Where the note lives | **A new `run_step` column**, R-3's shape: both stores, the cache mirror and `RunStepSummary`. |
| Which banner a promotion resumes (blueprint A-7, decided after the blueprint) | **The latest `session_started` row**, not the first (amends MOD-4 D192). A re-promotion after a fallback resumes the handoff session instead of retrying the dead one. |

## Grounding (facts this plan rests on)
- SDK `agent-client-protocol` 2.1.0 (`Cargo.lock`) has `ConnectionTo::load_session(id, cwd)`
  and `resume_session(id, cwd)`. Both return a `RestoreSessionBuilder`, and
  `.block_task().start_session()` on it yields a `RestoredSession`, whose `into_session()` is the
  same `ActiveSession` that `session_main` drives today (`session.rs:88-131`, `:456-530`). For
  `session/load`, "history updates sent before the load response remain queued on the returned
  session" (`concepts/sessions.rs:86-88`). `LoadSessionRequest` and `ResumeSessionRequest` both
  carry `additional_directories` and are stable v1 with no feature gate (schema 1.7.0
  `v1/agent.rs:958`, `:1289`).
- The live claude ACP adapter advertises `loadSession: true` and
  `sessionCapabilities { …, resume }` (`docs/ANA-4.md:594-596`).
- `settings.acp.session.{load,resume}` already exist and both default to `true`
  (`htui-agent/src/launch.rs` `SessionSettings`). `caps_from` reads only `.resume` for ACP
  (`registry.rs:165`).
- `promote::opening_kind` resumes only `Transport::Cli` (`promote.rs:63-72`), and the test
  `an_acp_agent_hands_off_even_when_its_caps_say_resume` pins that today.
- `session_main` writes the `session_started` banner from `session_ref` after `session/new`
  (`acp/mod.rs:1101-1145`). The same banner after a restore keeps `promote::banner` working on a
  re-promoted step.
- CLI: `claude` 2.1.287 with an unknown `--resume` id prints
  `No conversation found with session ID: …`, emits a `result` error line and exits 1 before
  `system/init` (probed 2026-10-03). The sentence goes to **stderr**; the stdout `result` line goes
  into `pre_init` and is dropped. The driver turns that into a start `Err`
  (`DriverError::Transport`: "the agent ended before sending `system/init`" plus the stderr tail,
  `cli/mod.rs:948-962`, `:1411-1418`), so the reason reaches the error text, and `run_chat` fails
  the chat (`agent_worker.rs:3949-3976`).
- The Chat tab header already prints `resumed`/`handoff` from `Via`
  (`ui/tabs/chat/mod.rs:325-328`), derived from `OpeningPath` in `htui-worker/src/runtime.rs:2386-2389`.
  The Runs pane shows nothing about the opening.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Summary field across the three builders | commit `19ca2292` (R-3 `gate_note`) | `RunStepSummary` field, `MemStore`, `pg/read.rs` + `pg/rows.rs`, `cache/read.rs`, one `.sqlx` entry, a conformance case, `closeout.rs` literal |
| New write op across both stores | commit `2ece53cc` (R-5 `pass_step`/`park_step`) | `traits.rs` → `mem.rs` → `store/worker.rs` → `pg/write.rs` → `htui-store` `writer.rs`/`worker.rs`, conformance first, `.sqlx` via the sandbox `prepare` recipe |
| Migration header | `crates/htui-store/migrations/0011_permission_relay.sql:1-12` | forward-only, cites the plan, states the mirror rebuild |
| Enum-as-text column | `migrations/0003_orchestration.sql:67-68` (`verify_outcome`) | `TEXT CHECK (… IN (…))`, nullable |
| Mirror column | `cache_migrations/0003_orchestration.sql:16-18`, `cache/refresh.rs:1195-1252` | `ALTER TABLE run_step ADD COLUMN`, extend the column list and the `sqlx::query!` |
| ACP transport tests | `htui-agent/tests/acp_driver.rs:212-290` (`refuse_session_new`), `:492-540` (`fs_agent`) | raw newline-delimited JSON-RPC fake over `tokio::io::duplex`, asserting events and messages |
| Driver errors | `acp/mod.rs:1701` `handshake_error(step, err, child)` | `"<step> failed: <err>"` plus the stderr tail |
| Promote pure tests | `htui-orch/src/promote.rs:262-322` | `opening_kind` over `cli_caps()` + `banner_events()` |
| Worker promoted-bind tests | `agent_worker.rs:5126-5190` (`attach_promoted_resumes_a_cli_step_with_its_banner`, `fixture_with_spec_spy`) | spec spy asserting `seen.resume`, opening text recorded as `follow_up` |
| Runs pane extra line | `ui/tabs/backlog/detail/runs.rs:1137-1148` (`note_line`) + its tests `:2000-2050` | an optional fitted line under the step, `PANE` wide, width test |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-store/migrations/0014_run_step_opening.sql` | CREATE | `run_step.opening TEXT CHECK (opening IN ('resumed','handoff','resume_failed'))`, nullable |
| `crates/htui-store/cache_migrations/0005_run_step_opening.sql` | CREATE | the mirror column |
| `crates/htui-core/src/model/{run,mod}.rs` | UPDATE | `StepOpening` via `str_enum!` (`run.rs:139-163`), re-exported; `RunStepSummary.opening: Option<StepOpening>` |
| `crates/htui-core/src/store/{traits,mem,mod,conformance}.rs` | UPDATE | `WriteStore::record_opening(step, StepOpening)`; MemStore keeps a `step → opening` map (the field is not on `RunStep`); re-export; conformance cases |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | case count |
| `crates/htui-store/src/pg/{write,read,rows}.rs` | UPDATE | Postgres op + summary projection |
| `crates/htui-store/src/cache/{refresh,read}.rs` | UPDATE | mirror the column (`RUN_STEP_COLUMNS`, select, bind); read it into the summary |
| `crates/htui-store/src/{writer,worker}.rs` | UPDATE | route the new op (`Writer` is Memory or Online only; no offline writer) |
| `crates/htui-store/.sqlx/*.json` | UPDATE | several entries (the write, the summary read, the mirror refresh); `cargo sqlx prepare` against a migrated scratch DB |
| `crates/htui-store/tests/{pg_conformance,cache,connect,migrations}.rs` | UPDATE | `EXPECTED_CASES`; mirror round-trip; the migration-count pins 13 → 14 (`connect.rs:139-159,243-245`, `migrations.rs:96-103,1017-1141,1291`, the 0013 commit `7c7e78c2`'s pattern) |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | UPDATE | `UsageSpy` / `SpyStore` implement `WriteStore` |
| `crates/htui-orch/src/closeout.rs` | UPDATE | `RunStepSummary` literal gains `opening` |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | `session_main` restores instead of `session/new` when `spec.resume` is set |
| `crates/htui-agent/src/registry.rs` | UPDATE | ACP `caps.resume = session.resume \|\| session.load` |
| `crates/htui-agent/tests/acp_driver.rs` | UPDATE | resume, load-with-replay, neither, refused-load cases |
| `crates/htui-orch/src/promote.rs` | UPDATE | `opening_kind` drops the CLI-only rule; tests flip |
| `crates/htui-orch/src/command.rs` | UPDATE | `OpeningPath::Resume` carries the handoff fallback (`handoff: String`, `digest: String`) |
| `crates/htui-orch/src/engine.rs` | UPDATE | `opening` builds the handoff text for both paths (helper extracted from the `Handoff` arm) |
| `crates/htui/src/agent_worker.rs` | UPDATE | fallback on a failed resume; `record_opening`; the `resume_failed` step-log row |
| `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE | a `resume_failed` `other` event flips the header's `via` to `handoff` and shows the sentence |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` (+ `runs/execution_graph.rs` literal at `:693`) | UPDATE | "context not carried; handoff prompt only" line at both `note_line` call sites (`list_lines`, `flow_head`); literals at `runs.rs:2365`, `execution_graph.rs:693` gain `opening: None` |
| `HANDOFF.md`, PRD, `docs/ANA-2.md` amendment line, `docs/decisions/mod/mod-37.md` | UPDATE | close-out |

## Tasks

**Sequencing (fact-check amendment).** The three lanes share no file, but they share crates:
T1 edits `htui-agent` (`conformance.rs`, `tests/recorder.rs`) and `htui-orch` (`closeout.rs`),
and `htui-orch` depends on `htui-agent`. Running them at once on one tree would let a half-done lane
break another lane's build. So **T1 runs first** on the main tree and commits. Then **T2 and T3 run in
parallel**: T2 on the main tree, and T3 in one worktree on a named branch cut from T1's head,
merged back with `--no-ff` before T4. T4, T5 and T6 follow in order. T3 keeps the `htui` crate
compiling by adding the new `Resume` fields as `..` at `agent_worker.rs:1034` and `:5140`, which
are its only cross-crate touches.

### Task 1: `run_step.opening`, store to summary
- **Files**: both migrations; `htui-core` `model/{run,mod}.rs`, `store/{traits,mem,mod,conformance}.rs`,
  `tests/mem_store.rs`; `htui-store` `pg/{write,read,rows}.rs`, `cache/{refresh,read}.rs`,
  `writer.rs`, `worker.rs`, `.sqlx/`, `tests/{pg_conformance,cache,connect,migrations}.rs`;
  `htui-agent` `src/conformance.rs` and `tests/recorder.rs` (spy `WriteStore` impls);
  `htui-orch/src/closeout.rs` (literal only); `htui/src/ui/tabs/backlog/detail/runs.rs:2365` and
  `runs/execution_graph.rs:693` (test literals only, `opening: None`; the rest use `..base`).
- **Action**: tests first. Conformance cases: `record_opening` round-trips through `run_steps` and
  the item's run summary in both stores, a second write replaces the first, an unknown step is
  refused with `NotFound`, and the write bumps `updated_at` so the mirror picks it up. Then the
  column, the op and the three builders. The field lives on `RunStepSummary` only, not on `RunStep`,
  so the engine's `RunStep` literals stay untouched. `MemStore` therefore keeps its own
  `HashMap<StepId, StepOpening>`, dropped wherever its steps are dropped. The migration-count pins
  move from 13 to 14.
- **Mirror**: `19ca2292`, `2ece53cc`; migration header `0011`.
- **Validate**: `cargo test -p htui-core --all-features`, `cargo test -p htui-store --all-features`
  (Postgres conformance against `HTUI_TEST_DATABASE_URL`), `cargo sqlx prepare --check` per
  `docs/hr-sandbox.md`.

### Task 2 (parallel with T3, after T1): the ACP driver restores a session
- **Files**: `htui-agent/src/acp/mod.rs`, `htui-agent/src/registry.rs`, `htui-agent/tests/acp_driver.rs`
  (and `tests/launch.rs` only if a default-caps pin moves).
- **Action**: tests first, against a scripted fake in `refuse_session_new`'s style:
  (a) `spec.resume = Some(id)` plus a `sessionCapabilities.resume` handshake: `session/resume` is
  sent with the id, cwd and extra dirs, `session/new` is never sent, the banner carries the id;
  (b) `loadSession: true` only: `session/load` is sent, the fake replays an
  `agent_message_chunk` and a `tool_call` before answering, and no event from the replay reaches
  `next_event`, so the first event after the banner belongs to the new prompt's turn;
  (c) neither advertised, or both disabled in settings: `start` fails with a `Transport` error
  naming the missing capability, the child is reaped, and no `session/new` is sent;
  (d) a refused `session/load`: the error is `session/load failed: <agent's message>`, the D61
  shape.
  Then `session_main` step 2 branches: with no `spec.resume`, `session/new` as today; with it,
  resume, then load, then refuse. The load path drains what is already queued before the model
  step and the first prompt. `read_update` is a `futures` mpsc `next()`, so one poll with
  `std::task::Waker::noop()` returns `Ready` while something is queued and `Pending` once the queue
  is empty. The SDK queues replay ahead of the ordered response (`session.rs:1223`,
  `jsonrpc.rs:6011`). No Cargo change: `futures` is not a dependency of `htui-agent`. `at_step` names the request actually
  sent. `caps_from`: ACP `resume = session.resume || session.load`.
- **Mirror**: `acp_driver.rs` D61 cases; `handshake_error`.
- **Validate**: `cargo test -p htui-agent --all-features --test acp_driver`, then
  `cargo test -p htui-agent --all-features`.

### Task 3 (parallel with T2, worktree, after T1): the engine resumes ACP steps and always carries the handoff
- **Files**: `htui-orch/src/promote.rs`, `htui-orch/src/command.rs`, `htui-orch/src/engine.rs`;
  `htui/src/agent_worker.rs:1034` and `:5140` (only adding `..` / the new fields, so the crate keeps
  compiling). `htui-worker/src/runtime.rs:2387` matches `{ .. }` and is unaffected.
- **Action**: tests first. `opening_kind` resumes for any transport when `caps.resume` holds and a
  banner exists. The pinned test becomes `an_acp_agent_with_resume_caps_and_a_banner_resumes`, and
  ACP with `resume: false` still hands off. `OpeningPath::Resume` gains `handoff: String` and
  `digest: String`. The engine's `opening` builds the handoff spec in a helper for both paths, so
  the worker can fall back without the engine. An engine test asserts that a Resume opening carries
  the same handoff text a Handoff opening would. Doc comments that say "CLI only" / "R-48" are
  rewritten.
- **Note**: the `Resume` path now also needs the project's `handoff` template. A project without
  one already fails every non-resumable promotion, so a promotion now fails the same way,
  with the step promoted and no chat (the existing "opening cannot be built" path).
- **Mirror**: `promote.rs` tests; `engine.rs` `a_handoff_spec_carries_no_excerpts_and_runs_no_pass`.
- **Validate**: `cargo test -p htui-orch --all-features --no-fail-fast` (grep for SIGABRT: stack
  headroom).

### Task 4: the worker falls back on a failed resume and records the opening (after T1, T3)
- **Files**: `htui/src/agent_worker.rs`.
- **Action**: tests first, with the spec-spy fixture and a fake driver whose first `start` fails:
  (a) Handoff opening → `opening = handoff`, one `start` with `resume: None`;
  (b) Resume that starts → `opening = resumed`, `RESUME_OPENING` recorded as the `follow_up`;
  (c) Resume whose `start` fails with anything but `DriverError::Spawn` → `opening = resume_failed`,
  and an `other` row `resume_failed` in the step's log with body
  `{ session_id, reason, note: "context not carried; handoff prompt only" }`. The row is recorded
  through the continuing recorder and sent to the tab as a `ChatFrame::Event`, the way the
  `follow_up` row is. Then a second `start` runs with `resume: None` and the handoff text, recorded
  as the `follow_up`. (`SessionSpec` is `Clone`, and `start` takes `&self`.)
  (d) the fallback start also failing → the chat fails as today, and `opening` stays
  `resume_failed`;
  (e) `Spawn` → no fallback (the adapter is gone, so a handoff would fail the same way), and the
  reprobe runs as today.
  `ChatArgs` gains the fallback text. `opening` is written through the bind's `htui_store::Writer`,
  and a failed write is logged, never fatal to the chat. A CLI `No conversation found…` arrives in
  the error's stderr tail, so the row quotes it.
- **Mirror**: `attach_promoted_resumes_a_cli_step_with_its_banner`, `fixture_with_spec_spy`;
  `run_chat`'s start-error arm.
- **Validate**: `cargo test -p htui --all-features agent_worker -- --test-threads=1`.

### Task 5: Runs pane and Chat tab (after T1, T4)
- **Files**: `htui/src/ui/tabs/backlog/detail/runs.rs`, `htui/src/ui/tabs/chat/mod.rs`.
- **Action**: tests first (render and width tests, snapshots where the pane already has them).
  The Runs pane shows a fitted line under a step whose `opening` is `handoff`
  ("context not carried; handoff prompt only") or `resume_failed`
  ("resume failed; context not carried; handoff prompt only"), after R-3's gate-note line when
  both exist, at the same indent, at **both** `note_line` call sites (`list_lines` `:1648`,
  `flow_head` `:1616`). Scroll and height come from `lines.len()`, so they need no change. In the
  Chat tab, a `resume_failed` `other` event turns the header's `via` (set by `OrchReply::Promoted`
  before the bind) to `handoff`, and the transcript shows the sentence once. No new reply or frame
  type: `ChatFrame::Event` already carries it.
- **Mirror**: `note_line` and its tests; the header tests at `chat/mod.rs:1060-1210`.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`, `cargo insta test` if
  snapshots move.

### Task 6: close-out
- HANDOFF: R-48 struck ("closed by MOD-37 phase 5"), a phase 5 note, and MOD-37 closes if nothing
  is left open (R-32's D131 half, R-44, R-53 and R-55 are re-deferred or documented with reasons:
  confirm at close-out against the PRD's success metric).
- Add an amendment line to ANA-2 §4.8 (`docs/ANA-2.md:1239`): ACP now resumes. Write-up
  `docs/decisions/mod/mod-37.md`, DECISIONS index, and PRD row 5 `complete`.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast 2>&1 | tee /tmp/m5-gate.log; grep -c SIGABRT /tmp/m5-gate.log
cargo test -p htui --all-features -- --test-threads=1          # scheduling-dependent suite
cd crates/htui-store && cargo sqlx prepare --check -- --all-features   # migrated scratch DB, docs/hr-sandbox.md
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| The `session/load` replay drain misses an update that arrives after the response (an agent that streams replay late) | L | The SDK queues replay ahead of the response by ordered dispatch. Prefer `session/resume`. A late replay row would be a visible duplicate, never a lost row. Case (b) pins the drain. |
| A restored ACP session silently has no context (the agent persisted nothing) | M | Out of `htui`'s sight. The `resumed` label is a claim about the protocol, not the model's memory. Mention it in the write-up. |
| `Resume` now depends on the `handoff` template | L | Every seeded project has it, and the failure path is the existing "opening cannot be built" sentence (T3 note). |
| New migration means one mirror rebuild per box | L | Same as 0011, stated in the migration header. |
| `htui-orch` test stack headroom | M | Box large futures; gate with `--no-fail-fast` and grep for SIGABRT. |
| sqlx prepare against the empty compose DB | M | Sandbox scratch-DB recipe (`docs/hr-sandbox.md`). |
| Fallback double-start leaves a child behind | L | The failed `start` already reaps (D61). Case (c) asserts one live child at a time through the fake. |
| The handoff `follow_up` after a `resume_failed` row lands at the next turn of a log the failed start never wrote to | L | The failed start emits nothing (it fails before the banner), so the continuing recorder sees the same tail; case (c) asserts the seq/turn order: `resume_failed`, then `follow_up`. |

## Acceptance
- [ ] An ACP promotion with a banner and resume caps resumes (`session/resume`, else `session/load` with no replay rows)
- [ ] A failed resume, ACP or CLI, is reported and opens a labelled handoff, never silently
- [ ] `run_step.opening` is written for every bound promoted chat and shown in the Runs pane; the Chat tab header matches it
- [ ] Each behaviour has a test that fails on the old code
- [ ] Validation passes; validator green
- [ ] Patterns mirrored, not reinvented

## Verified claims
Step 3.5 fact-check, 2026-10-03: one agent plus two of the session's own probes. Falsified or
incomplete claims were amended above.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | `agent-client-protocol` 2.1.0 / schema 1.7.0, no extra features | ✓ | `Cargo.lock:178-179,212-213`; `Cargo.toml:94` `=2.1.0`; `htui-agent/Cargo.toml:29` |
| 2 | `load_session`/`resume_session` → `RestoredSession::into_session()` is the same `ActiveSession` `session_main` drives | ✓ | SDK `session.rs:88,114,466,483,530`; **compile probe** (offline, 13 s): new/resume/load routes assigned to one `ActiveSession<'static, Agent>` binding, then `send_prompt` |
| 3 | Replay before the `session/load` response stays queued; a non-blocking drain is possible | amended | `concepts/sessions.rs:87-89`; `read_update` = `futures` mpsc `next()` (`session.rs:1054`); replay pushed at `:1223` before the ordered response (`jsonrpc.rs:6011`). `futures` is **not** an `htui-agent` dependency, so T2 polls with `Waker::noop()` (compiled in the probe) |
| 4 | Both requests carry `additional_directories`; capability fields public; no feature gate | ✓ | schema `v1/agent.rs:958/974/1004`, `:1289/1303/1336`, `:3794`, `:3809`, `~4065` |
| 5 | `SessionSettings.{load,resume}` default true; `caps_from` reads only `.resume`; no test pins ACP `caps.resume` | ✓ | `launch.rs:358-372`; `registry.rs:165`; `tests/launch.rs:333-334` pins settings only; `driver_contract.rs:390,521` |
| 6 | `opening_kind` resumes CLI only; the pinning test exists | ✓ | `promote.rs:57-72`, `:315` |
| 7 | `OpeningPath::Resume` sites | amended | built `engine.rs:1373`, `agent_worker.rs:5140` (test); destructured `agent_worker.rs:1034`; `runtime.rs:2387` is `{ .. }`. T3 now touches `agent_worker.rs` at those two lines; `runtime.rs` was dropped from the plan |
| 8 | `RunStepSummary` literals; `Default`? | amended | no `Default` (`run.rs:737`); full literals `mem.rs:1287`, `pg/rows.rs:151`, `cache/read.rs:585`, `closeout.rs:272`, `runs.rs:2365`, `execution_graph.rs:693`; MemStore builds from `RunStep`, so it needs a side map |
| 9 | New migration: mirror rebuild, no constant | amended | `pg/mod.rs:690`, `cache/mod.rs:141-152`; the count pins in `tests/connect.rs` and `tests/migrations.rs` move from 13 to 14 (commit `7c7e78c2`'s pattern) |
| 10 | The mirror needs a column list entry, a `query!` and an `ALTER` | ✓ | `cache/refresh.rs:~1203,1212-1251`; `cache_migrations/0003_orchestration.sql:16-18` |
| 11 | Files touched by a new `WriteStore` op | amended | per `2ece53cc`: also `store/mod.rs`, `tests/mem_store.rs`, `htui-agent/src/conformance.rs` (`UsageSpy`), `htui-agent/tests/recorder.rs` (`SpyStore`); `WorkerStore` optional; no offline writer |
| 12 | Start-error arm; can the writer `add_note`? | ✓, gap | `agent_worker.rs:3949-3976`; `Writer::add_note` (`writer.rs:1145`), but `NewNote` needs item, user and box, which `ChatArgs` and `Promoted` lack, so the failure record became a step-log `other` row |
| 13 | CLI: an unknown `--resume` id gives a start `Err` that carries the reason | ✓ | `cli/mod.rs:948-962,1411-1418`; session probe: `No conversation found…` is on **stderr** (stdout carries only the `result` line), so the stderr tail carries it |
| 14 | How the tab learns `via` | amended | only `OrchReply::Promoted` (`views.rs:88-100`, sent before the bind, `runtime.rs:2386-2397`); `ChatAccepted`/`ChatFrame` have no slot, so the signal is a `resume_failed` `other` event over `ChatFrame::Event` |
| 15 | Runs pane: where step lines are assembled; scroll math | ✓ | `note_line` used at `runs.rs:1616` (`flow_head`) and `:1648` (`list_lines`); height and scroll come from `lines.len()` (`:1570,1589,1655`) |
| 16 | The handoff build can be extracted to a private helper; no other exhaustive matches | ✓ | `engine.rs:1337-1465`; the only other matches are `runtime.rs:2386` (`{ .. }`) and `agent_worker.rs:1034` |
| — | Task independence | amended | the file sets are disjoint, but the crates overlap (T1↔T2 in `htui-agent`, T1↔T3 in `htui-orch`, and `htui-orch` → `htui-agent`), so T1 runs first, then T2 ∥ T3 with T3 in a worktree |
