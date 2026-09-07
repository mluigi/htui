# Plan: MOD-2 durable history and replay (milestone 4)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 4 (Durable history and replay). Milestones 1–2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`). This plan continues their
decision (`D31`+) and task (`T15`+) numbering so a cross-reference never means two things.
Milestones 5–9 are out of scope; where one of them owns a seam this plan touches, the plan says
where it stops.

**Design authority**: `docs/ANA-4.md` §4.1 (recorder rules, the store seam, the offline JSON-lines
buffer, retention posture), §4.4 ("Session load and resume" — `htui` replays from its own store, not
through `session/load`), §6.1 (the mapping table the replay decoder inverts), §8 (module layout,
`store_worker` variants, test strategy), §9 step 4 (`StepEvents` replay), §11 criteria 2, 3, 4, 7 and
12. `docs/ANA-9.md` §4.3 (`session_event` columns, the pending buffer), §4.4 (the mirror and its
cursors). `docs/REQUIREMENTS.md` `R-HIS-1..2`, `R-SEC-3`, `R-TUI-6`, `R-NF-3`.

**Requirements**: `R-HIS-1` (nothing about a session exists only on one box: every event persists,
scrubbed, online or offline), `R-HIS-2` (the chat view reopens a past step read-only and replays it),
`R-SEC-3` (the offline buffer holds scrubbed rows only — the scrubber stays in the recorder),
`R-TUI-6` (the replay renders through the same transcript the live session renders through),
`R-NF-3` (no store handle on the render side; replay arrives as a reply like every other read).

**Complexity**: Large

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-07 (the PRD exists and lists
milestone 4 as its own row; C2/C4 borderline at route time). Grounding raised the breadth above the
route-time estimate — see "Scope finding" below — so the CONFIRM gate is offered over the wider
scope, and the ultracode recommendation is re-stated there rather than carried over from routing.

## Summary

Milestone 3 left one honest hole: a chat that cannot reach Postgres is **refused**
(`agent_worker.rs:333`, "Milestone 4 owns the offline session path"). This milestone closes it from
both ends. Offline, a chat starts against the mirror, records into
`<cache_dir>/pending/<project>.<run>.jsonl` through a new `Writer` arm, and the refresher's existing
`upload_pending` lands it idempotently on the next connection. Online or offline, any step that has
rows can be reopened: a new `StepEvents` request answers with the persisted rows, a new decoder in
`htui-agent` turns each row back into the `DriverEnvelope` the live path already renders, and the
Chat tab shows it in a read-only replay mode that issues no session command at all.

### Scope finding (decided here, surfaced at CONFIRM)

An offline chat needs two facts the mirror does not hold today:

1. **The registry row.** `MIRRORED_TABLES` is 15 tables and `agent` is not among them
   (`crates/htui-store/src/cache/mod.rs:35`), so `Backend::agents()` answers
   `StoreError::Unreachable` while offline by construction, not by omission
   (`crates/htui-store/src/backend.rs:267`). An offline chat cannot resolve a driver without it.
2. **The user.** `Backend::this_user()` answers `Unreachable` offline (`backend.rs:133`), and
   `run.started_by` is `NOT NULL`.

Both are fixed by mirroring one more table and reading the mirrored `app_user`, which costs a **cache
migration**. That migration takes the number `docs/ANA-2.md` §9 reserved for MOD-4
(`cache_migrations/0002_orchestration.sql`); MOD-4's becomes `0003_orchestration.sql`. The Postgres
side is untouched — `0002_agent_probe.sql` is still milestone 5's, and nothing here writes it.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming | `crates/htui-store/src/cache/pending.rs:54` | `append_pending`/`upload_pending` — one owner per side of a file-format contract, stated in the module doc |
| Store seam | `crates/htui-store/src/writer.rs:31` | `Writer` as a plain-delegation enum over backends; a new arm adds arms, never a branch in a caller |
| Errors | `crates/htui-store/src/backend.rs:140` | `StoreError::Unreachable(<what needs the server>)` for a capability the offline backend genuinely lacks |
| Requests | `crates/htui/src/store_worker.rs:48` | one `StoreRequest` variant per read, served in `serve`, with a `name()` arm for the failure line |
| Cross-tab | `crates/htui/src/app/update.rs:22` | `App::dispatch(Origin, StoreRequest)` — the shell stamps origin, so a reply can be addressed to a tab that did not emit the key |
| UI state | `crates/htui/src/ui/tabs/chat/transcript.rs:190` | `Transcript::apply(&DriverEnvelope)`, with the three `htui`-authored kinds arriving as `Other` (`transcript.rs:275`) |
| Tests | `crates/htui-store/tests/cache.rs:999`, `crates/htui-agent/tests/` | Postgres-gated suites behind `HTUI_TEST_DATABASE_URL`; `insta` snapshots for rendered output |

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D31 | **The mirror gains `agent`, and only `agent`.** `cache_migrations/0002_agent_mirror.sql` creates the table, `MIRRORED_TABLES` becomes 16, and `CacheStore::agents()` answers `AgentSummary { agent, on_box: None }`. `agent_box` is **not** mirrored. | The registry row is what `DriverFactory::driver_for` needs; `on_box` is a probe snapshot whose columns milestone 5's `0002_agent_probe.sql` is about to change, and mirroring a table one milestone before its shape moves buys two migrations for one fact. Offline, "which box" is this box, and enablement is `agent.enabled`. |
| D32 | **`agent` is unscoped, so it is a full replace with no cursor**, pulled in the refresher's step 1 beside `app_user`/`workspace`/`workspace_project` (`refresh.rs:216`, `replace_app_user` at `refresh.rs:399`). No `cache_cursor` row exists for it. | The mirror already has this shape for exactly this problem: `cache_cursor.project_id` is `NOT NULL` and every cursor-driven table is project-scoped, so the unscoped tables were made small full replaces instead. The registry is a handful of rows and follows the precedent rather than inventing a sentinel project id for one table. |
| D33 | **`CacheStore::this_user()` resolves the OS user against the mirrored `app_user`,** by the same name `PgStore::seed_if_empty_as` derives (`USERNAME`/`USER`), and answers `StoreError::NotFound` when this box has never synced a row for that name. | `run.started_by` is `NOT NULL` and an offline chat has to name an author. Deriving it the same way online does is what makes the uploaded run indistinguishable from one recorded online; inventing a local user id would make `upload_pending` insert a stranger. |
| D34 | **The offline sink is a third `Writer` arm, not a branch in the recorder**: `Writer::Buffered(BufferedWriter)`, where `BufferedWriter` holds the `CacheStore` (for reads), the cache dir, and `Arc<Mutex<HashMap<StepId, (ProjectId, RunId)>>>` filled by `start_chat_run`. `append_events` groups by `run_step_id`, looks the pair up and calls `append_pending`. `Backend::writer()` answers `Some` for `Offline`. | ANA-4 §4.1 says the recorder "writes the same rows as JSON lines" offline — same rows, different sink, which is exactly what a `WriteStore` arm is. The recorder stays generic over `S: WriteStore` and gains no offline knowledge, so every conformance case that passes online passes offline unchanged. `backend.rs`'s invariant is amended honestly in the same commit: there is now an offline write path, and it reaches a file, never the server. |
| D35 | **The buffered writer refuses what the buffer cannot hold.** `mint_item`/`update_item`/`transition` answer `StoreError::Unreachable` as before (offline item editing is MOD-13's question, not this milestone's); `upsert_agent`/`upsert_agent_box` likewise. `set_step_usage` and `finish_chat_run` are **no-ops that log at debug**. | The buffer's line format holds `session_event` columns only (`pending.rs:14`), so a `run_step` column written offline has nowhere to go. Silently dropping the usage would break §11 criterion 7 after upload, which is why D36 recomputes it server-side instead of pretending here. |
| D36 | **`upload_pending` fills `run_step.usage` and `run_step.prompt_digest` from the buffered rows**: the digest from the `prompt` row's `payload.digest`, the usage from **the same summing rule the recorder uses**. That rule moves to `htui-core`: `UsageTotals` leaves `htui-agent::record` for `htui_core::model::usage`, gaining `from_rows(&[SessionEvent])`, and both the recorder and the uploader call it. Both writes stay inside the existing transaction and idempotency is unchanged (`ON CONFLICT DO NOTHING`; a second pass inserts nothing). | §11 criteria 3 and 7 are stated over persisted rows and must hold for a chat that happened to be offline. The token fields of a `usage` row are **deltas** (`event.rs:339`, summed with `saturating_add` in `UsageTotals`) and only `cost_micros_total` is cumulative, so "take the last row" would be wrong and a second hand-written sum in `htui-store` would drift from the first. `htui-store` cannot depend on `htui-agent`, and `UsageTotals` is an ANA-9 §4.3 payload shape, so `htui-core` is where it belongs anyway. |
| D37 | **Replay decodes rows, it does not re-render them**: new `htui-agent::replay::envelope_from_row(&SessionEvent) -> Result<DriverEnvelope, ReplayError>`, the inverse of `record.rs`'s `round_trip`. The three `htui`-authored kinds (`prompt`, `follow_up`, `permission_answer`) decode to `DriverEvent::Other` with the row's payload as its body — the exact shape the live UI already receives (`transcript.rs:285`). An unknown `kind` or an undecodable payload becomes `Other { update: "<kind>" }`, never an error that costs the transcript. | One transcript code path for live and replay is the property `R-HIS-2` is worth: a second renderer drifts, and the first thing it drifts on is the kind an adapter added last week. `record.rs` writes each payload as the serde form of the event's inner struct, so the inverse is serde, not a hand-written parser. |
| D38 | **`StoreRequest::StepEvents(StepId)` → `StoreReply::StepEvents { step_id, events: Option<Vec<SessionEvent>> }`.** `None` is `ReadStore::step_events`'s "not cached" and renders as "this step is not on this box", distinct from `Some(vec![])` ("this step recorded nothing"). | `step_events` already exists on all three backends (`traits.rs:44`, `backend.rs:330`), so this is a worker variant and nothing deeper. Collapsing `None` into an empty transcript would make an unsynced step look like an empty conversation, which is the one reading `R-HIS-1` forbids. |
| D39 | **Replay is entered by `Action::Replay { step_id }`, handled in `App::update`**: focus the Chat tab, then `dispatch(Origin::Tab(ChatTab::ID), StoreRequest::StepEvents(step_id))`. The Runs pane gains step selection (`j`/`k` inside the selected run, `Enter` replays) and emits that action. | `TabRegistry::by_id_mut` exists but calling a Chat-specific method through `dyn Tab` would put a tab's vocabulary in the shared trait. Stamping the request with the Chat tab's origin uses the addressing the shell already has (`update.rs:22`), so the reply lands in `ChatTab::on_reply` like every other read and no trait changes. `RunSummary.steps` is already on the reply (`run.rs:319`), so selecting a step costs no extra request. |
| D40 | **Replay is a mode, and the mode is what makes it read-only**: `ChatTab` holds `replay: Option<ReplayState { step_id, transcript, missing: bool }>`. While it is `Some`, the composer refuses to open, digits answer nothing, `Esc` leaves replay, and the tab issues **no** `Chat*` request. A live session underneath is untouched and is restored on leave. | `R-HIS-2`'s "read-only" is a property of what the tab can *send*, not of what it draws greyed out. Making replay a state with no command path is the version a reviewer can check by grepping for `ctx.request` inside the branch. |
| D41 | **The replayed permission rows render resolved.** A `permission_request` row followed by its `permission_answer` row resolves through `Transcript::resolve_permission` exactly as it did live; a request with no answer (a cancelled session) stays parked-looking but **is not** answerable, because D40 consumes no digits in replay. | The parked row is history: it says the agent asked and nobody answered, which is what the log holds. Hiding it would edit the record; answering it would write to a finished step. |
| D42 | **The chat header states the buffer.** An offline chat renders `buffered · uploads when the store returns` beside the session id, from the writer's own label (`Writer::label()` gains `"buffered"`). | The maintainer must be able to tell a recorded conversation from one that is only on this disk; milestone 3's banner precedent is that a capability the transport lacks is stated, not inferred. |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-store/cache_migrations/0002_agent_mirror.sql` | CREATE | the mirrored `agent` table (D31) |
| `crates/htui-store/src/cache/mod.rs` | UPDATE | `MIRRORED_TABLES` 15 → 16, the nil-project cursor constant (D32) |
| `crates/htui-store/src/cache/read.rs` | UPDATE | `agents()`, `this_user()` over the mirror (D31, D33) |
| `crates/htui-store/src/cache/refresh.rs` | UPDATE | full-replace pull for the unscoped `agent` table (D32) |
| `crates/htui-core/src/model/usage.rs` | CREATE | `UsageTotals` + `from_rows`, moved out of the recorder (D36) |
| `crates/htui-agent/src/record.rs` | UPDATE | use the moved `UsageTotals` (D36) |
| `crates/htui-store/.sqlx/` | UPDATE | regenerated after the `run_step` insert changes (D36) |
| `crates/htui-store/src/backend.rs` | UPDATE | `Offline` arms of `agents`/`this_user`; `writer()` answers `Some` offline; module-doc amendment (D31, D33, D34) |
| `crates/htui-store/src/writer.rs` | UPDATE | `Writer::Buffered`, `BufferedWriter`, its `ReadStore`/`WriteStore` impls, `label()` (D34, D35, D42) |
| `crates/htui-store/src/cache/pending.rs` | UPDATE | `upload_pending` fills `run_step.usage`/`prompt_digest` (D36) |
| `crates/htui-store/tests/cache.rs` | UPDATE | mirror pull + offline registry/user cases |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | §11 criterion 12 end to end, criteria 3 and 7 after upload |
| `crates/htui-store/tests/writer_buffered.rs` | CREATE | the buffered writer against a temp cache dir |
| `crates/htui-agent/src/replay.rs` | CREATE | `envelope_from_row`, `ReplayError` (D37) |
| `crates/htui-agent/src/lib.rs` | UPDATE | re-export `replay` |
| `crates/htui-agent/tests/replay.rs` | CREATE | round-trip over the milestone-3 recorded fixtures |
| `crates/htui/src/store_worker.rs` | UPDATE | `StepEvents` request/reply, `serve` arm, `name()` arm (D38) |
| `crates/htui/src/app/action.rs` | UPDATE | `Action::Replay { step_id }` (D39) |
| `crates/htui/src/app/update.rs` | UPDATE | focus + dispatch for that action (D39) |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | step selection and the replay key (D39) |
| `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE | replay mode, header/banner, reply arm (D40, D42) |
| `crates/htui/src/ui/tabs/chat/transcript.rs` | UPDATE | `from_rows` constructor over decoded envelopes (D37) |
| `crates/htui/src/agent_worker.rs` | UPDATE | drop the milestone-3 refusal; run against the buffered writer (D34) |
| `crates/htui/tests/` (snapshots) | UPDATE | replay-mode snapshot, offline-chat harness case |
| `crates/htui/src/keymap.rs` | UPDATE | the Runs-pane replay binding and its help line |

## Tasks

Independence below is by **file set**, and the sets are listed for exactly that reason. Two tasks
whose sets intersect run serially even when their subjects look unrelated.

### T15: Mirror the agent registry and the offline user
- **Files**: `cache_migrations/0002_agent_mirror.sql`, `src/cache/mod.rs`, `src/cache/read.rs`,
  `src/cache/refresh.rs`, `src/backend.rs`, `tests/cache.rs`
- **Action**: create the mirrored table; add the full-replace pull beside `replace_app_user`; implement
  `CacheStore::agents()` (`on_box: None`) and `CacheStore::this_user()`; point the `Offline` arms of
  `Backend::agents`/`this_user` at them. Tests first: a refresh pulls a seeded `agent`; an offline
  backend lists it; `this_user` resolves the OS name and answers `NotFound` for an unsynced one.
- **Mirror**: the existing per-table pull arms in `refresh.rs`; `read.rs`'s column-by-column decode
  with `uuid_col`/`json_col` error context.
- **Validate**: `cargo test -p htui-store --features demo`;
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`

### T16: `Writer::Buffered`, the offline sink (serial after T15 — both touch `backend.rs`)
- **Files**: `src/writer.rs`, `src/backend.rs`, `tests/writer_buffered.rs`
- **Action**: TDD the arm: `start_chat_run` registers `(step → project, run)`; `append_events`
  groups by step and appends through `append_pending`; an event for an unregistered step is a
  `NotFound`, not a silent drop; `set_step_usage`/`finish_chat_run` no-op; item writes stay
  `Unreachable`; reads delegate to the cache. Then `Backend::writer()` answers `Some` for `Offline`,
  and the module doc says what changed and what did not.
- **Mirror**: `writer.rs`'s plain-delegation style; `pending.rs`'s "trusts its input, the scrubber
  lives upstream" doc rule.
- **Validate**: `cargo test -p htui-store --features demo`; clippy over the crate.

### T17: `upload_pending` fills the step columns
- **Files**: `crates/htui-core/src/model/usage.rs`, `crates/htui-core/src/model/mod.rs`,
  `crates/htui-agent/src/record.rs`, `crates/htui-store/src/cache/pending.rs`,
  `crates/htui-store/tests/pg_criteria.rs`, `crates/htui-store/.sqlx/`
- **Action**: move `UsageTotals` into `htui-core` with a `from_rows` constructor (the recorder's
  existing tests are the proof the move changed nothing), then TDD against Postgres: a buffer holding
  a `prompt` row and three `usage` rows uploads a `run_step` whose `prompt_digest` equals the payload
  digest and whose `usage` equals `UsageTotals::from_rows(...)`; a second upload changes nothing and
  still deletes the file. Regenerate `.sqlx` from inside `crates/htui-store` per README (the insert
  is a `query!`).
- **Mirror**: the existing `upload_one` transaction and its `ON CONFLICT DO NOTHING` clauses.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`;
  `cargo test -p htui-agent --features test-support`;
  `cd crates/htui-store && DATABASE_URL=…/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features`
- **Note**: this task touches `htui-agent` (`record.rs`) and T18 touches `htui-agent`
  (`replay.rs`, `lib.rs`) — the file sets are disjoint, so the two stay parallel; only `lib.rs` would
  collide, and the `usage` move exports from `htui-core`, not from `htui-agent`.

### T18: The replay decoder
- **Files**: `crates/htui-agent/src/replay.rs`, `crates/htui-agent/src/lib.rs`,
  `crates/htui-agent/tests/replay.rs`
- **Action**: TDD `envelope_from_row`: every `EventKind` decodes to the `DriverEvent` the recorder
  wrote; the three `htui`-authored kinds and the session banner decode to `Other` with the row's
  payload; an unknown kind degrades to `Other` rather than erroring. Regression net: feed the
  milestone-3 fixtures through record → rows → decode and assert the decoded envelopes equal the
  recorded ones (`insta`).
- **Mirror**: `record.rs`'s `round_trip`, inverted; `map.rs`'s "unknown lands in `other`" rule.
- **Validate**: `cargo test -p htui-agent --features test-support`; clippy over the crate.

### T19: `StepEvents` and the way into replay (needs T18's types only at the seam; may start once T18 lands)
- **Files**: `crates/htui/src/store_worker.rs`, `src/app/action.rs`, `src/app/update.rs`,
  `src/ui/tabs/backlog/detail/runs.rs`, `src/keymap.rs`
- **Action**: add the request/reply pair and its `serve`/`name` arms; add `Action::Replay`, handled
  as focus-then-dispatch under the Chat tab's origin; give the Runs pane step selection and the
  replay key with its help line. Harness test: the action focuses Chat and the reply is addressed to
  it.
- **Mirror**: `store_worker.rs`'s existing read arms; `update.rs`'s `dispatch(Origin, …)`.
- **Validate**: `cargo test -p htui --features testkit`

### T20: Chat tab replay mode (serial after T19)
- **Files**: `src/ui/tabs/chat/mod.rs`, `src/ui/tabs/chat/transcript.rs`, `crates/htui/tests/`
  snapshots
- **Action**: `ReplayState`; `Transcript::from_rows` over decoded envelopes; the read-only key
  handling of D40; the header line; the "not on this box" body for a `None` reply. Snapshot a
  replayed fixture conversation and assert it matches the live snapshot of the same fixture modulo
  the header.
- **Mirror**: `chat/mod.rs`'s existing mode-free key handling and its `hint()` table.
- **Validate**: `cargo test -p htui --features testkit`; review the new snapshots.

### T21: The offline chat itself (serial after T16 and T20 — touches `chat/mod.rs`)
- **Files**: `crates/htui/src/agent_worker.rs`, `src/ui/tabs/chat/mod.rs`, `crates/htui/tests/`
- **Action**: delete the milestone-3 refusal at `agent_worker.rs:333`; the offline path resolves box,
  user and registry row from the mirror and records through `Writer::Buffered`; the header states the
  buffer (D42). End-to-end test with an offline `Backend`: a chat against the fake driver writes
  `<dir>/pending/<project>.<run>.jsonl` in `seq` order, and `upload_pending` then lands it
  (§11 criterion 12).
- **Mirror**: `agent_worker.rs`'s existing `start` flow, minus the refusal.
- **Validate**: `cargo test -p htui --features testkit`;
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features`

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings

# after T17, from inside the crate (README "Postgres queries are checked at compile time")
cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features
```

`USERNAME=htui-ci` is TOOL-2's standing workaround; a Postgres suite run without
`HTUI_TEST_DATABASE_URL` reports `ok` while proving nothing, so the Postgres line above is the one
that counts for T15, T17 and T21.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The cache migration takes MOD-4's reserved `0002` number | Certain | Stated at CONFIRM; MOD-4's cache migration becomes `0003_orchestration.sql`, recorded in the HANDOFF MOD-4 line at close-out |
| `agent` mirrored now, `agent_box` shape moving in milestone 5 | Medium | D31 mirrors `agent` only; the probe snapshot stays server-side until its columns settle |
| A global table in a per-project cursor table | Medium | D32's nil-project sentinel, as a named constant with a test that a full refresh pulls `agent` exactly once |
| Offline `run.started_by` names a user the server does not have | Low | D33 derives the same name the online seed does and refuses (`NotFound`) rather than inventing one; `upload_pending` already parks a buffer the server refuses instead of deleting it |
| Replay decode drifts from record encode | Medium | T18's round-trip test over the milestone-3 recorded fixtures is the regression net, and both sides are serde over the same structs |
| Two transcript code paths | Low by construction | D37 gives replay the live renderer; the only new UI code is the mode, not the rendering |
| A replay could send a command to a finished step | Low | D40 makes replay a state with no `ctx.request` call at all — checkable by grep |

## Verified claims

Checked against the tree on 2026-09-07, before the CONFIRM gate. Two claims were **falsified** and the
plan was amended before the maintainer saw it (D32, D36).

| Claim | Verdict | Evidence |
|---|---|---|
| `agent` is not mirrored; `MIRRORED_TABLES` holds 15 tables | true | `crates/htui-store/src/cache/mod.rs:35` |
| `Backend::agents()` is `Unreachable` offline | true | `crates/htui-store/src/backend.rs:267`–`271` |
| `Backend::this_user()` is `Unreachable` offline | true | `crates/htui-store/src/backend.rs:133`, `:140` |
| `Backend::writer()` answers `None` offline | true | `crates/htui-store/src/backend.rs:115`, `:119` |
| The milestone-3 chat refuses an unwritable backend, naming milestone 4 | true | `crates/htui/src/agent_worker.rs:333`–`337` |
| `ReadStore::step_events` exists on every backend, `None` = not cached | true | `crates/htui-core/src/store/traits.rs:44`; `backend.rs:330`; `cache/read.rs:550` |
| `RunSummary` already carries its steps, so replay needs no extra read | true | `crates/htui-core/src/model/run.rs:319` |
| `App::dispatch(Origin, StoreRequest)` exists, so a reply can be addressed to another tab | true | `crates/htui/src/app/update.rs:22`, `:119`–`:129` |
| `TabRegistry` has `by_id_mut`/`focus` but no downcast, so a Chat-specific call would need a trait method | true | `crates/htui/src/ui/tabs/registry.rs:112`, `:135` |
| The three `htui`-authored kinds already reach the live transcript as `Other` | true | `crates/htui/src/ui/tabs/chat/transcript.rs:275`–`287` |
| A recorded row's `payload` is the serde form of the event's inner struct (so decode is serde, not a parser) | true | `record.rs:790`–`808` (`event_row`), `record.rs:965` (`round_trip`); `prompt` adds only `digest` (`record.rs:389`) |
| `EventKind` has an `other` member for an unknown kind | true | `crates/htui-core/src/model/event.rs:40` |
| `upload_pending` runs on reconnect already | true | `crates/htui-store/src/cache/refresh.rs:298` |
| `run_step` has `prompt_digest` and `usage` columns, and the uploader sets neither | true | `crates/htui-store/migrations/0001_init.sql:487`, `:489`; `cache/pending.rs:302`–`305` |
| `app_user` is mirrored, and the online seed derives the name from `USERNAME`/`USER`/`htui` | true | `cache/mod.rs:36`; `crates/htui-store/src/pg/mod.rs:472`–`474` |
| `CacheStore` is `Clone`, so a buffered writer can own one | true | `crates/htui-store/src/cache/mod.rs:74` |
| `insta` is a dev-dependency of both `htui-agent` and `htui` | true | `crates/htui-agent/Cargo.toml:47`, `crates/htui/Cargo.toml:44` |
| Postgres queries are compile-checked against committed `.sqlx` (85 files), regenerated from inside the crate | true | `README.md:285`–`296`; `crates/htui-store/.sqlx/` |
| **An unscoped mirrored table needs a cursor row with a sentinel project id** | **false** | Unscoped tables are full replaces with **no** cursor: `refresh.rs:216` ("unscoped full replace `app_user`, `workspace`, `workspace_project`"), `replace_app_user` at `refresh.rs:399`. D32 rewritten to follow that precedent. |
| **`run_step.usage` is the last `usage` row's cumulative payload** | **false** | The token fields are per-row **deltas** summed with `saturating_add` (`event.rs:339`, `record.rs:236`–`239`); only `cost_micros_total` is cumulative. D36 rewritten: the summing rule moves to `htui-core` and both writers call it. |
| Task independence: T15∩T16 = `backend.rs` (declared serial), T20∩T21 = `chat/mod.rs` (declared serial), T17∩T18 disjoint inside `htui-agent` | true | file sets listed per task above |

## Acceptance

- [x] All tasks complete — T15–T21, landed `81d247b`
- [x] Validation passes, Postgres line included — 410 tests, 0 failed, `cargo sqlx prepare --check` clean
- [x] `docs/ANA-4.md` §11 criteria 2, 3, 4, 7 and 12 demonstrated by tests, criterion 12 end to end
      (`crates/htui/tests/chat_offline.rs::a_buffered_chat_lands_in_postgres_on_the_next_connection`)
- [x] `R-HIS-1` and `R-HIS-2` hold for a chat recorded offline and replayed after upload
- [x] `rust-reviewer` gate run over the whole change set — no CRITICAL, no HIGH; two MEDIUM
      (`seal_one`'s append fallback, `seal_orphaned` failing `CacheStore::open`) and three of five LOW
      fixed in the same commit; L-3 (`&Ctx` is not a read-only capability) recorded as an observation,
      L-4 (`.serena/` untracked) kept out of the commit
- [x] Patterns mirrored, not reinvented

## Close-out

Landed `81d247b` on 2026-09-08, one commit, bookkeeping in the next.

**Amendments made during implementation**, each with its reason:

| # | Amendment | Why |
|---|---|---|
| A1 | `[H-1]`: the live buffer is `<project>.<run>.jsonl.open`, sealed by `finish_chat_run`, with `seal_orphaned` at `CacheStore::open`. `finish_chat_run` is therefore **not** the pure no-op D35 specified. | The refresher would otherwise upload a chat mid-flight, set `run.status = done` and freeze `run_step.usage` at a partial sum — the criterion-7 property this milestone is for. Tagged at every site so a veto is mechanical. |
| A2 | A seal onto a taken name renames to `<project>.<run>.<n>.jsonl`; it never appends. `parse` accepts the numbered stem and dedupes `(run_step_id, seq)`. `list` sorts by stem so the head uploads first. | Reviewer M-1: the append fallback duplicated rows across a crash between write and unlink, and lost a tail when two processes raced. Rename is atomic; both windows close. |
| A3 | `seal_orphaned` warns per unsealable file instead of propagating. | Reviewer M-2: one bad file in `pending/` failed `CacheStore::open`, i.e. application start, over a best-effort buffer. `upload_pending` already warns and continues. |
| A4 | `record_unreadable` sums a masked `usage` payload. | The uploader sums every persisted `usage` row; the recorder skipped the ones scrubbing made undecodable, so an online step and the same step uploaded from a buffer could disagree. Landed with the test that was missing when this was first declined. |
| A5 | Replay keys are `J`/`K` + `Enter`, and the binding lives in `register_all`, not `Keymap::default_global`. | `j`/`k` belong to the Backlog list cursor and `Enter` never reaches the detail pane; the key table is not allowed to name a concrete view. |
| A6 | `htui-store` grew a `test-support` feature (`tests/common/mod.rs` → `src/testkit.rs`, self dev-dep) and `Harness::drive_to_end()`. | Criterion 12 needs one test holding both a driver and a throwaway database; a poll-once harness cannot drive a `spawn_blocking` write without a sleep, which the byte-stable-snapshot rule forbids. Reviewer confirmed neither leaks into `cargo build`. |
| A7 | `run_step.usage` is written on every uploaded step, as five nulls where there were no usage rows. | Matches what the online path writes at the prompt, so an uploaded step is indistinguishable from one recorded online. |

**Known and accepted:** an uploaded offline chat has `run_step.agent_id` and `model` NULL (the line
format carries `session_event` columns only, H-3); a run split across two sealed files keeps the
first file's partial `usage`, because the second insert is `ON CONFLICT DO NOTHING` and changing that
would take the column's ownership away from the recorder; `Some(vec![])` from `StepEvents` is
currently unproduced, the arm kept total deliberately.
