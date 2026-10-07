# Plan: MOD-12 M3 — Queue overlay

**Source PRD**: `.claude/prds/mod-12-auto-mode-queue-runner.prd.md`
**Selected Milestone**: 3 — Queue overlay
**Complexity**: Large
**Status**: confirmed by the maintainer 2026-10-07 (L4: close a stalled batch); implementation pending
**Routing**: plan path (milestone of an existing PRD; C2, C3, C4 fired), ultracode for the
implementers only (`/handoff-run` verdict 2026-10-07, accepted). Waves below are decided by the file
sets in "Files to Change", not by prose.

## Summary

One modal overlay, opened from anywhere with `ctrl-q`, shows this box's queue in queue order. Each
row says what that entry is doing: running here, next to run, waiting and why, held by the batch cap,
or escalated to a human and why. Escalation reasons are review loop exhausted, judge undecided, hard
gate parked, blocked, failed, missing tags, and live on another box. The overlay also pauses and
resumes the queue, dequeues an entry, and reorders entries by writing `queue_entry.position`. That
column already exists (`0016`), so M3 needs **no migration**. The maintainer's L4 answer also changes
the runner: once a batch has stalled (no live run, nothing admissible), it closes, so a later `Q`
never starts spending on its own.

## Grounding (read on `hr/MOD-12` at `95a48f49` = `main`; see Verified claims)

- **Schema.** `queue_entry.position INTEGER NULL` exists. Its comment reads "milestone 3 reorder;
  NULL sorts last" (`0016_auto_queue.sql`). Nothing writes a non-`NULL` value: `pg/write.rs:7893`
  inserts `NULL`, and `mem.rs:946` sets `None`. Ordering already honours it:
  - `admission_order` (`htui-core/src/model/queue.rs:60-79`) sorts by
    `(position.is_none(), position, ready_index)`, where `ready_index` is `ready_items`' order
    (`priority DESC, created_at, id`), so admission is PRD D4 exactly;
  - `queue_entries` (`pg/read.rs:2242`, `mem.rs:977`) sorts `position NULLS LAST, queued_at,
    item_id`, which is **not** D4. Only the runner reads it, and it re-sorts.

  Migrations end at `0017_follow_up.sql` here and on `/host/htui`.
- **Queue surface.** `MemStore`, `PgStore` and `Backend` have `queue_item`, `dequeue_item`,
  `queue_entries`, `open_batch`, `open_batch_of`, `close_batch`, `prune_finished_entries`,
  `close_drained_batch`, `batch_cancelled_items`, `batch_spend`, `batch_runs`, `run_batch_spend`,
  `queued_runs_on_box`, `ready_items` and `running_runs_on_box`. `Backend` refuses every one offline.
  There is no reorder write.
- **TUI queue requests.** `QueueState`, `QueueItem`, `DequeueItem`, `ResumeQueue` and `PauseQueue`
  (`store_worker.rs:368-382`) are served by `serve_queue` (`:2341-2410`). They answer
  `StoreReply::Queue(QueueView)` or `QueueWritten { write, view }`. `QueueView { entries: Vec<ItemId>,
  open_batch, demo }` (`:1705`) is IDs only. `QUEUE_REQUEST_NAMES: [&str; 5]` (`:96`) is matched by
  the Backlog (`backlog/mod.rs:801`, `:1195`).
- **Drain.** `PgStore::close_drained_batch` (`pg/write.rs:8050`) closes `drained` only while the batch
  is open, **the box has no entry**, and no run of the batch is `queued|running|awaiting_approval`.
  `admit()` (`htui-worker/src/runtime.rs:2169`) calls the drain only when `entries` is empty. When
  `admission_order` is empty, or every entry is stopped by the batch rule, it returns with the batch
  left open. This is review L4: a stalled batch stays open, so a later `Q` spends without `P`.
- **Why an entry is not admitted is never stored.**
  - The runner drops not-ready, missing-tags, `blocked_by`-open and cancelled-in-batch entries
    silently.
  - Batch stops and malformed caps are logged once per batch (`note_batch_stop` `:333`,
    `note_bad_cap` `:345`).
  - The persisted signals per escalation are:

    | Escalation | Item status | Run status | Other signal |
    |---|---|---|---|
    | Review loop exhausted | `blocked` | `awaiting_approval` | note `review loop exhausted after N attempts…` (`gate.rs:1050`) |
    | Judge undecided | `awaiting_approval` | `awaiting_approval` | no step parked; note ``fan-out `p` attempt N awaits selection: …`` (`engine.rs:4670`) |
    | Hard gate parked | `awaiting_approval` | `awaiting_approval` | a step `awaiting_approval`; any gate park in an auto run is hard, since soft gates are downgraded at snapshot time |
    | Blocked (walk refusal, incl. batch cap/budget in the walk) | `blocked` | — | note = `run.failure` text (`engine.rs:3421`, `:5643`, `:838`) |
    | Failed | `failed` | — | `run.failure` |
    | Missing tags (normal case), `blocked_by` open | `open` | — | **nothing written**: compute live with `missing_tags(item, box)` (`backend.rs:556`) and the item's blocker links |
    | On another box | — | `queued`/`running` | the item's live run has `target_box_id ≠` this box; no read lists it today |
- **Overlay framework.** `trait Overlay` (`ui/overlay/registry.rs:36`) has `wants_requests`,
  `on_key`, `on_reply` and `render`. Factories are registered and offered in `app/mod.rs:101-107`.
  `is_offerable` (`app/state.rs:339`) is `Workspaces | Find | Waiting`. Catalogue rows are at
  `keys/catalogue.rs:320-321` (`ctrl-f`, `ctrl-w`). The `print-keys` template is `keys/print.rs:92`,
  and the end-to-end key table `tests/keys.rs:143`. **No overlay is ticked:**
  `App::on_tick` (`app/update.rs:126`) re-reads `StoreState`, `Waiting` and the active tab every
  `TICKS_PER_REFRESH = 4` ticks. The closest analogue is `WaitingList`
  (`ui/overlay/waiting_list.rs`), which is modal and has a cursor anchored on row identity, `j`/`k`,
  `Enter` → `Action::Reveal(RevealTarget::Step{..})` (`:115-126`), a dim hint line, and a `Bench` with
  `TestBackend` snapshots. `RevealTarget::Item { id, key }` exists (`app/action.rs:103`).
- **Keys.** `ctrl-q` and `ctrl-u` are unbound (`ctrl-q` appears only in a chord-parsing test,
  `keys/chord.rs:940`). Ctrl chords pass through text fields and the Backlog/Runs panes, which is why
  Ctrl+F and Ctrl+W are chords.
- **Live budget inputs.** `min_budget_micros(app)` (`queue.rs:114`), `batch_budget` / `BatchStop`
  (`queue.rs`), `ProjectCaps::from_settings` (`quota.rs:322`), `Backend::project_settings` (`:319`)
  and `Backend::app_settings` (`:394`), and `admission_limit` / `free_slots` (`queue.rs:86-96`).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Pure queue logic | `admission_order`, `batch_budget` (`htui-core/src/model/queue.rs`, table tests at `:170+`) | Pure function in `model/queue.rs` that the runtime and the TUI only feed |
| Inherent unmirrored read | `batch_runs` / `queue_entries` (Mem `mem.rs:969`, Pg `pg/read.rs:2230`, `Backend` `backend.rs:633`) | Inherent on Pg/Mem; `Backend` dispatches and refuses offline (`orchestration_offline()`) |
| Queue write | `queue_item` / `dequeue_item` (`pg/write.rs:7872`, `:7932`) | Box-scoped write, typed reply, no panic on a lost race |
| Pg/Mem parity | `inherent_orchestration_reads_answer_the_fixture` (`htui-store/tests/pg_criteria.rs`) | Same fixture through both stores, equal answers |
| TUI request + serve module | `queue_settings.rs` (M2) and `serve_queue` (`store_worker.rs:2341`) | `StoreRequest` variant + `name()` arm + route, served by a module that composes `Backend` reads |
| List overlay | `WaitingList` (`ui/overlay/waiting_list.rs`, Bench `:349-411`, `snapshots/`) | Modal, row-anchored cursor, hint line, `Reveal` on `Enter`, insta over `TestBackend(100, 30)` |
| Data on open | `WorkspaceSwitcher::wants_requests` (`workspace_switcher.rs:156`) | Request on open, replace state on reply |
| Global offered act | Ctrl+W: `Act::Waiting`, catalogue `:321`, `is_offerable`, `app/mod.rs:101-107`, `print.rs:92`, `tests/keys.rs:143` | New `Act` + catalogue row + offerable arm + register/offer + print template + key table |
| Runner behaviour test | `crates/htui-worker/tests/auto_queue.rs` (M1/M2) | `Backend::Memory` with the claim scan on and a fake driver; Pg variant under `testkit` |

## Decisions (proposed; CONFIRM accepts or overrides)

- **D1 — No migration.** Reorder writes `queue_entry.position`; L4 reuses `closed_reason =
  'drained'` (maintainer, 2026-10-07). `0018` stays free.
- **D2 — One queue order everywhere: `position NULLS LAST, priority DESC, created_at, id`** (PRD D4).
  `queue_entries` changes from `queued_at` to this order, joining `item` in Pg and sorting on the
  `Item` in Mem, so the overlay, the reorder write and `admission_order` all agree. `admission_order`
  is unchanged: it already yields this order over the ready subset. A parity test pins it.
- **D3 — Reorder = "move one entry one step" in the store, atomically.**
  `move_queue_entry(box, item, QueueMove::{Up, Down}) -> Result<bool>` on Mem, Pg and `Backend`
  (offline refusal).
  - In one transaction, Pg locks the box's entries (`SELECT … FOR UPDATE` in D2 order), writes
    `position = 1..n` in the current order, then swaps the item with its neighbour.
  - The first move therefore makes every current entry explicit, and entries queued later (`NULL`)
    go after them, ordered by priority.
  - Moving the first entry up, the last down, or an item not in the box's queue is `Ok(false)`, not
    an error.
  - It never touches `item.priority`.
- **D4 — L4: a stalled batch closes `drained`.** The predicate in `close_drained_batch` (Pg and Mem)
  loses its "box has no entry" clause. It closes the named batch iff the batch is open and no run of
  it is `queued|running|awaiting_approval`. The re-check stays in the UPDATE's `WHERE`.
  - The runner calls it when **no entry is admissible this tick**:
    - `entries` is empty (today's drain);
    - `admission_order` is empty: everything not ready, cancelled in this batch, or escalated;
    - or the tick had free slots and **every** ordered entry was stopped by `batch_budget`, a
      malformed cap, or an absent project.
  - Free slots = 0 never closes, because a slot may be held by a manual run. An enqueue refusal
    never closes either; it is treated as transient.
  - A parked run (`awaiting_approval`) keeps the batch open, because it is live.
  - The accepted costs:
    - an entry that becomes ready after the close waits for `P`;
    - the next `P` opens a fresh batch with a fresh cap (the PRD's accepted consequence).
  - The Backlog `Q`/`P` status lines are unchanged; they already read the batch state after the write.
- **D5 — The overlay's model is pure and lives in `htui-core`.** In `model/queue.rs`:
  - **Types.** `QueueRow` holds the entry plus the item key, title, status, project, priority and
    `created_at`. It also carries `latest_run: Option<QueueRunFact>`, `latest_note: Option<String>`
    and `open_blockers: Vec<String>` (keys). `QueueRunFact` holds the run id, status, mode,
    `target_box_id`, the target box's hostname, `failure`, and whether a step is parked with its id.
  - **`classify_entry(row, here: BoxId, live: &LiveFacts) -> EntryState`.** `LiveFacts` holds the
    ready set, cancelled-in-batch, missing tags per item, and the batch-stop result per project.
    `EntryState` is one of:
    - `Running { run, status }`, a live run here;
    - `Elsewhere { hostname, status }`;
    - `Next`, ready and admissible;
    - `Held(BatchStop)`;
    - `Waiting(Wait::{BlockedBy(keys), CancelledInBatch, Paused})`;
    - `Escalated(Escalation)`.
  - **`Escalation`** is one of `HardGateParked { run, step }`, `JudgeUndecided { run, note }`,
    `ReviewLoopExhausted { run, note }`, `Blocked { note }`, `Failed { failure }` and
    `MissingTags(tags)`. Each has a `Display` sentence. Note text is shown verbatim, never parsed,
    so a walk-time batch-cap block reads as its own note.
  - **Precedence** (first match wins): a live run (here or elsewhere), item `failed`, item `blocked`
    (review loop when its run is `awaiting_approval`), item `awaiting_approval` (gate when a step is
    parked, else judge), missing tags, `blocked_by` open, cancelled in batch, held, next.
  - **`QueueOverview`** holds the box, the open batch (`opened_at`, spend), slots used and the limit,
    the rows in D2 order with their `EntryState`, and `demo`.
- **D6 — One joined read, `queue_rows(box) -> Vec<QueueRow>`**, inherent on Pg, Mem and `Backend`
  (offline refusal), in D2 order. In Pg it is one statement:
  - `queue_entry ⨝ item`;
  - `LATERAL` for the latest run with its target box's hostname, plus whether a step is
    `awaiting_approval`;
  - `LATERAL` for the latest note;
  - an array of open `blocked_by` keys.

  Missing tags and the budget stop stay **live** in the serve module (D7); they are per-box and
  per-batch facts that the existing reads answer. An N+1 over a queue of tens of entries is
  accepted for `missing_tags`, called only for `open` items not in the ready set.
- **D7 — The serve module `crates/htui/src/queue_overview.rs`.** It handles `StoreRequest::QueueOverview`
  (reply `StoreReply::QueueOverview(QueueOverview)`). It composes:
  1. `queue_rows`, `open_batch_of`, `batch_cancelled_items` and `ready_items`;
  2. `missing_tags` for non-ready `open` rows;
  3. `batch_spend`, `project_settings` per project, `app_settings`, `min_budget_micros` and
     `batch_budget`;
  4. `running_runs_on_box` and `admission_limit`.

  It then calls `classify_entry`. A memory or demo build answers with `demo: true`, as `QueueView`
  does. Offline answers `Failed` through the normal path. `StoreRequest::MoveQueueEntry { item, to }`
  is served in `serve_queue` and answers `QueueWritten`. A new `QueueWrite::Moved { moved: bool }`
  arm keeps the Backlog's sentences intact. Both names join `QUEUE_REQUEST_NAMES` (5 → 7).
- **D8 — The overlay `QueueOverlay`** (`ui/overlay/queue.rs`, `OverlayId("queue")`) is modal.
  - **Opening.** It opens on global `ctrl-q` (`Act::Queue`, offered like Ctrl+W). `wants_requests`
    is `[QueueOverview]`.
  - **Header.** `queue: running · batch since 14:02 · $1.20 spent · 1/2 slots`, `paused`, or
    `demo: nothing is admitted`.
  - **Rows.** Rows are in queue order, each `KEY  title  <state sentence>`, with escalations in the
    theme's warning style. A footer hint carries the keys.
  - **Keys:**

    | Key | Action |
    |---|---|
    | `j` / `k` | Move the cursor (anchored on the item) |
    | `J` / `K` | Move the entry down / up (`MoveQueueEntry`) |
    | `P` | Pause or resume by the header state (`PauseQueue` / `ResumeQueue`) |
    | `Q` | Dequeue the entry under the cursor (`DequeueItem`) |
    | `Enter` | Close and reveal: `RevealTarget::Step` for a run or escalation with a run, otherwise `RevealTarget::Item` |
    | `Esc` | Close (the existing wildcard binding) |

  - **Refresh.** After any `QueueWritten` or `Failed` for a queue request it re-requests
    `QueueOverview`. While it is open it also re-reads on the shell's refresh tick (D9).
  - **Size.** On a narrow terminal the reason column truncates with an ellipsis. No new width rules;
    MOD-81 owns those.
- **D9 — An overlay refresh hook.** `trait Overlay` gains `fn refresh(&mut self, ctx: &mut Ctx<'_>)
  {}` (default no-op). `App::on_tick`'s refresh branch calls it on the **top** overlay, next to
  `refresh_active_tab`. Existing overlays are unchanged.
- **D10 — Not in M3:**
  - target-box selection (MOD-43): `Elsewhere` only reports;
  - enforcing the scheduler window;
  - any change to the Backlog `Q`/`P` keys or sentences;
  - CLEAN-9's residuals. The overlay shows the runner's budget rule (`ProjectCaps::from_settings`),
    so a malformed project key reads `Held` there while CLEAN-9's engine path is still lax. This is
    noted, not fixed.

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/queue.rs` | UPDATE (`QueueRow`, `QueueRunFact`, `EntryState`, `Escalation`, `Wait`, `LiveFacts`, `QueueOverview`, `classify_entry`, `QueueMove` + tests) | T1 |
| `crates/htui-core/src/model/mod.rs` | UPDATE (exports, if not glob) | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE (T2: `close_drained_batch` predicate + test; T3: `queue_entries` order, `queue_rows`, `move_queue_entry` + tests) | T2, T3 |
| `crates/htui-store/src/pg/write.rs` | UPDATE (T2: drain predicate; T3: `move_queue_entry`) | T2, T3 |
| `crates/htui-store/src/pg/read.rs` | UPDATE (`queue_entries` order, `queue_rows`) | T3 |
| `crates/htui-store/src/backend.rs` | UPDATE (`queue_rows`, `move_queue_entry` dispatch) | T3 |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE (T2: stalled-close parity; T3: order, `queue_rows`, move parity) | T2, T3 |
| `crates/htui-store/.sqlx/*` | CREATE/DELETE | T2, T3 |
| `crates/htui-worker/src/runtime.rs` | UPDATE (`admit` stall detection → close) | T2 |
| `crates/htui-worker/tests/auto_queue.rs` | UPDATE (L4 tests) | T2 |
| `crates/htui/src/queue_overview.rs` | CREATE (serve module) | T4 |
| `crates/htui/src/lib.rs` | UPDATE (module line) | T4 |
| `crates/htui/src/store_worker.rs` | UPDATE (`QueueOverview`, `MoveQueueEntry`, replies, `QueueWrite::Moved`, names 5→7, routes) | T4 |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE (exhaustive `QueueWrite` match, if any) | T4 |
| `crates/htui/src/ui/overlay/queue.rs` + `src/ui/overlay/snapshots/*queue*` | CREATE | T5 |
| `crates/htui/src/ui/overlay/mod.rs`, `ui/overlay/registry.rs` (refresh hook) | UPDATE | T5 |
| `crates/htui/src/app/update.rs` (tick calls `refresh`), `app/state.rs` (`is_offerable`), `app/mod.rs` (register/offer), `app/action.rs` or `keys/` (`Act::Queue`), `keys/catalogue.rs`, `keys/print.rs` | UPDATE | T5 |
| `crates/htui/tests/keys.rs` (key table), `crates/htui/tests/queue_overlay.rs` + snapshots | UPDATE / CREATE | T5 |
| `docs/ANA-2.md` (§4.10 as-built: overlay, reorder, L4 close), PRD row + L4 note, HANDOFF (P2 close-out: MOD-12 done) | UPDATE | T6 |

**Intersections that decide the waves:**
- T1 ∩ T2 = ∅ → **T1 ∥ T2** (Wave 1). T2 does not need T1's types.
- T2 ∩ T3 = {`mem.rs`, `pg/write.rs`, `pg_criteria.rs`, `.sqlx`} → serial: T3 after T2. T3 also
  needs T1's `QueueRow` / `QueueMove`.
- T4 needs T1 and T3 (types, `Backend` methods); T4 ∩ T3 = ∅ but the dependency makes it serial.
- T5 ∩ T4 = ∅ (`store_worker.rs` is T4's only), but T5 needs T4's requests → serial.
- T6 last.

## Tasks

### Wave 1 (parallel: T1 ∥ T2)

#### T1: The pure queue model (TDD)
- **Action**: Table tests first in `queue.rs` for `classify_entry`: one per `EntryState` and per
  `Escalation`, the precedence order of D5 (e.g. a failed item with an open blocker reads `Failed`; a
  blocked item with an `awaiting_approval` run reads review-loop; an `awaiting_approval` run without a
  parked step reads judge), `Elsewhere` beats everything, `Held` only for ready entries, plus
  `Display` sentences. Then the types and the function.
- **Mirror**: `admission_order` / `batch_budget` tests.
- **Validate**: `cargo test -p htui-core --all-features queue`.

#### T2: L4 — a stalled batch closes (TDD)
- **Action**: Red tests in `auto_queue.rs` over `Backend::Memory`:
  - (a) only a `blocked` entry and no live run → the batch closes `drained`, and a later `Q` of a
    ready item admits nothing until `P`;
  - (b) a parked (`awaiting_approval`) auto run keeps the batch open;
  - (c) every ready entry held by `batch_budget` with free slots → closes;
  - (d) free slots 0 held by a manual run with admissible entries → stays open;
  - (e) the empty-queue drain still closes (regression).

  A Mem unit test and a Pg parity case show the store close ignores entries but honours live runs.
  Then the predicate change (Pg SQL + Mem), the `.sqlx` entry against a migrated scratch DB, the doc
  comments, and the `admit` stall detection of D4.
- **Mirror**: M1/M2 `auto_queue.rs` tests; `close_drained_batch_closes_only_the_drained_batch_it_names`.
- **Validate**: `cargo test -p htui-worker --all-features -- --test-threads=1`; `cargo test -p htui-core --all-features batch`; `cargo test -p htui-store --all-features --test pg_criteria -- --test-threads=1`.

### Wave 2

#### T3: Store — order, `queue_rows`, `move_queue_entry` (TDD)
- **Action**: Tests first:
  - Mem units: `queue_entries` in D2 order;
  - `queue_rows` facts: latest run with hostname, parked step, latest note, open blockers only;
  - `move_queue_entry`: materialises positions, swaps, ends are `false`, a later `Q` goes last, and
    priority is untouched;
  - Pg/Mem parity in `pg_criteria.rs` on one fixture for all three.

  Then the Pg SQL (one statement for `queue_rows`, one transaction for the move), `Backend` dispatch
  with the offline refusal, and `.sqlx`.
- **Mirror**: `queue_entries` / `batch_runs` end to end; `inherent_orchestration_reads_answer_the_fixture`.
- **Validate**: `cargo test -p htui-core --all-features`; `cargo test -p htui-store --all-features -- --test-threads=1`; `cargo build --workspace --all-features --all-targets`.

### Wave 3

#### T4: Requests and the serve module (TDD)
- **Action**: Tests first in `queue_overview.rs` over `Backend::Memory`. A fixture with one entry per
  state answers a `QueueOverview` whose rows and states match. Further cases:
  - a batch over its cap marks ready rows `Held`;
  - demo answers `demo: true`;
  - `MoveQueueEntry` answers `QueueWritten { Moved }` with the new order;
  - an offline `Backend` answers `Failed` with `QUEUE_REQUEST_NAMES` containing the name.

  Then the variants, `name()` arms, routes and the module.
- **Mirror**: `queue_settings.rs` serve module; `serve_queue`.
- **Validate**: `cargo test -p htui --all-features queue -- --test-threads=1`.

### Wave 4

#### T5: The overlay (TDD)
- **Action**: Red Bench tests and snapshots in `ui/overlay/queue.rs`:
  - each state row;
  - the header in its three forms;
  - the offline line;
  - cursor anchoring across a reorder;
  - `J`/`K`/`P`/`Q` emitting the right requests;
  - `Enter` emitting `Reveal`.

  An end-to-end `tests/queue_overlay.rs` (testkit `Harness`) covers `ctrl-q` opening it over the
  Memory backend, a reorder round trip, and refresh on tick. `tests/keys.rs` gains the `ctrl-q` row.
  Then the refresh hook (D9), `Act::Queue` wiring, the catalogue row, the `print.rs` template, and
  `is_offerable` with its test.
- **Mirror**: `WaitingList` and its tests; Ctrl+W's wiring.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo insta test -p htui --all-features` — new snapshots only; `print-keys` output check.

### Wave 5

#### T6: Documents, close-out, gates
- **Action**:
  - `docs/ANA-2.md` §4.10 as-built note (overlay, reorder semantics, L4 stall close);
  - PRD row 3 `complete`, plus the L4 decision under Open Questions;
  - HANDOFF P2 close-out of MOD-12 (all milestones), as `docs/decisions/mod/mod-12.md` per
    `workflow-docs.md`;
  - full gate run below.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless: catches test-support-only code
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee /tmp/mod12-m3.log
grep -n -E 'SIGABRT|test result: FAILED' /tmp/mod12-m3.log
cargo insta test -p htui --all-features            # only the new queue-overlay snapshots appear
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The stall close fires while a slot is held by a manual run with admissible entries waiting | Medium | D4: free slots 0 never closes; test T2(d) |
| A parked hard gate counted as "not live" closes the batch under a running review | Low | Live = `queued|running|awaiting_approval` (unchanged SQL clause); test T2(b) |
| Classification precedence disagrees with what the runner actually does | Medium | One pure function, table-tested; the serve module uses the runner's own `ready_items`, `admission_order` and `batch_budget` |
| The latest note is not the cause of a block | Medium | Shown verbatim as "last note"; the Runs pane holds the full history (`Enter` reveals it) |
| `queue_entries` reorder changes runner behaviour | Low | `admission_order` already re-sorts by position then ready order; an M1/M2 suite rerun proves it |
| `ctrl-q` swallowed by terminal flow control | Low | Raw mode clears `IXON` (crossterm); the e2e harness test drives the chord; `keys.toml` lets the maintainer rebind (MOD-67) |
| Overlay refresh spams requests | Low | Refresh only on the shell's refresh tick, and only when none is in flight |
| `.sqlx` drift | Medium | Prepare against a migrated scratch DB (`docs/hr-sandbox.md`) |
| Snapshot churn outside the new overlay | Low | No change to existing render paths; `cargo insta test` must show new files only |

## Acceptance

- [ ] `ctrl-q` opens the queue overlay. It lists the box's queue in D2 order with every row's state.
      Each escalation kind of the PRD has a named test and a snapshot row.
- [ ] `J`/`K` reorder by position; backlog priority is untouched; the runner admits in the new order
- [ ] `P` pauses or resumes from the overlay; `Q` dequeues
- [ ] A stalled batch closes, and a later `Q` admits nothing until `P` (L4)
- [ ] Validation passes; existing snapshots unchanged
- [ ] Patterns mirrored, not reinvented

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `queue_entry.position` exists, nullable, always written `NULL` | TRUE | `0016_auto_queue.sql`; `pg/write.rs:7893-7894` `VALUES ($1, $2, NULL, …)`; `mem.rs:946`; `queue.rs:35` doc |
| `admission_order` sorts `(position.is_none(), position, ready_index)` | TRUE | `queue.rs:74-77`; test `admission_order_puts_positions_first_then_ready_order` `:225` |
| `queue_entries` orders by `queued_at`, not D4 | TRUE | `pg/read.rs:2242` `ORDER BY e.position NULLS LAST, e.queued_at, e.item_id`; `mem.rs:977-984` |
| No reorder write exists | TRUE | Backend queue methods `backend.rs:599-761`; grounding agent's full listing |
| `close_drained_batch` requires the box to have no entry | TRUE | `pg/write.rs:8063` `NOT EXISTS (SELECT 1 FROM queue_entry e WHERE e.box_id = b.box_id)` |
| `admit` drains only on empty `entries`; empty `order` returns with the batch open | TRUE | `runtime.rs:2192-2195`, `:2233-2237` |
| Next migration `0018` free here and on host | TRUE (unused by this plan) | `ls` both trees end `0017_follow_up.sql` |
| No overlay is ticked; `on_tick` refreshes state, waiting, active tab | TRUE | `app/update.rs:126-137`, `TICKS_PER_REFRESH = 4` (`:19`) |
| `is_offerable` = `Workspaces \| Find \| Waiting` | TRUE | `app/state.rs:339-341` |
| `ctrl-q` unbound | TRUE | catalogue global rows `:243-321`; only hit `keys/chord.rs:940` (parse test) |
| Ctrl+W pinned in `print.rs` and `tests/keys.rs` | TRUE | `keys/print.rs:92`, `tests/keys.rs:143` |
| `WaitingList` `Enter` reveals a step; `RevealTarget::Item` exists | TRUE | `waiting_list.rs:115-126`; `app/action.rs:101-108` |
| `QUEUE_REQUEST_NAMES` is `[&str; 5]` matched by the Backlog | TRUE | `store_worker.rs:96`; `backlog/mod.rs:801`, `:1195` |
| `missing_tags`, `project_settings`, `app_settings` on `Backend`; `min_budget_micros` pure | TRUE | `backend.rs:556`, `:319`, `:394`; `queue.rs:114` |
| Escalation signals per kind (table in Grounding) | TRUE (grounding agent, file:line each) | `gate.rs:1050`, `engine.rs:4670`, `:3421`, `:5643`, `:838`, `:707-717` |
| Missing tags / `blocked_by` leave no persisted reason for a queued item | TRUE | `ready_items` filters them; runner drop is silent (`runtime.rs:2233`) |
| Task independence T1 ∥ T2 | TRUE | file sets disjoint (T1: `queue.rs`, `model/mod.rs`; T2: `mem.rs`, `pg/write.rs`, `pg_criteria.rs`, `.sqlx`, `runtime.rs`, `auto_queue.rs`) |
| T2/T3 intersect | TRUE → serial | {`mem.rs`, `pg/write.rs`, `pg_criteria.rs`, `.sqlx`} |
| T2 does not need T1's types | TRUE | T2 changes a predicate and `admit`'s control flow only |
