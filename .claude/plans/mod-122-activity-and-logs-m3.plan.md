# Plan: MOD-122 M3 — Activity panel

**Source PRD**: `.claude/prds/mod-122-activity-and-logs.prd.md`
**Selected Milestone**: 3 — Activity panel
**Complexity**: Large (seven tasks: new table with status triggers, a narrow store trait, a MemStore write hook, a shell-owned panel with its own key context, a Backlog default change with snapshot churn, one App setting)
**Status**: complete 2026-11-14 (hr/MOD-122 4c032f88): T1–T7 + T5b landed, review fix round R1 applied (both instances; nothing deferred)
**Routing**: PRD path by `/handoff-run MOD-122` (C2, C4 fired; resumed at `plan` for M3); ultracode accepted for the implement phase.
**Grounded at**: `e99acd97` (branch `hr/MOD-122`). Requirements: `R-TUI-16` (activity panel); touches `R-TUI-12` (layout presets), `R-TUI-13` (history), `R-TUI-11` (the waiting reasons), `R-TUI-8` (one retention field). PRD milestone 3.
**Blueprint**: .claude/plans/mod-122-activity-and-logs-m3.blueprint.md (F1–F13 folded into the task sections on 2026-11-14)

## Summary

Every status change that matters to the maintainer is stored as one row of a new Postgres table
`activity_event`, written **by the store at the moment the status changes**: Postgres triggers on
`run`, `run_step`, `item` and `step_permission` (so the TUI's in-process runtime, every `htui worker`
and the chat path all write through the same point), and, for the memory backend, one diff hook in
`MemStore::write`. Kinds follow the DECIDE gate: *needs you* (gate, judge failed, selection, unblock,
interrupted, permission), *failures* (step failed, run failed), *plain* (run started, run finished,
ready for close-out). A shell-owned activity panel fills the activity slot of the layout presets:
events newest first, needs-you events still waiting pinned on top with their count in the title,
`f` cycles all workspaces · needs you · failures · this workspace, Enter jumps to the item's Runs
sub-tab (switching workspace and saying so when needed) or replays a chat, recorded in back/forward
history. It polls once a second like the rest of the shell. Events expire with the logs' retention
(`log_retention_days`, default 14, now editable in Settings), and an item's or run's events go with
it except failure events, which stay until retention. Backlog defaults to the preset with the panel.

## Decisions

PRD open questions M3 touches are answered here (D4, D6, D7, D11). Maintainer checks are marked *check*.

- **D1 — one narrow trait, reads only.** `ActivityStore` (`htui-core/src/store/activity.rs`) has two
  methods and two implementors, `MemStore` and `PgStore`, plus `Writer` forwarding, exactly the
  shape of M2's `WorkerLogStore` (`htui-core/src/store/worker_log.rs:21-82`, `Writer` impl
  `htui-store/src/worker.rs:529`, inherent `Backend::worker_logs` `htui-store/src/backend.rs:597-611`).
  No append method: events are written inside the transitions (D2), never by a caller. A
  `#[cfg(feature = "test-support")] MemStore::push_activity` seeds rows for UI tests only.
  ```rust
  pub trait ActivityStore: Send + Sync {
      /// My events with `id > after`, oldest first, at most `limit`; `None` = the newest `limit`,
      /// still oldest first. `pinned` is every needs-you event of mine still waiting (D4), newest
      /// first, at most `PINNED_LIMIT` (200), whatever `after` says.
      fn activity(&self, after: Option<i64>, limit: u32) -> impl Future<Output = Result<ActivityPage>> + Send;
      /// Deletes events with `at < older_than`, then beyond `per_user_cap` per user (oldest first);
      /// returns the count. Idempotent: two sweepers at once are harmless.
      fn prune_activity(&self, older_than: DateTime<Utc>, per_user_cap: u32) -> impl Future<Output = Result<u64>> + Send;
  }
  ```
- **D2 — one writer per transition, in the store.** The transitions have many call sites: a step
  fails through `transition_step` (`pg/write.rs:8036`) from about ten engine sites plus
  `interrupt_step` (`:8155`) and `answer_gate` (`:8191`); a run parks through `park_step`
  (`:9290`), `promote_step` (`:9155`) and `transition_run` (`:7971`); chat runs bypass the engine
  (`start_chat_run` `:3334`, `finish_chat_run` `:3388`, `fail_run` `:9361`, called from
  `htui/src/agent_worker.rs`). Both runtimes reach the same store: the TUI's
  `RunRuntime<Backend, …>` (`htui/src/run_worker.rs:39`) and the worker's `RunRuntime<PgStore, …>`
  (`htui/src/worker_cmd.rs:147`). So the one writer is the store:
  - **Postgres:** `CREATE CONSTRAINT TRIGGER … DEFERRABLE INITIALLY DEFERRED FOR EACH ROW`
    *(blueprint F2)*: one per table on `run`, `run_step` and `item`, `AFTER UPDATE OF status … WHEN
    (OLD.status IS DISTINCT FROM NEW.status AND NEW.status IN (…))`, listing only the statuses that
    can write (`run_step`: `awaiting_approval`, `failed`; `run`: `awaiting_approval`, `failed`,
    `running`, `done`; `item`: `blocked`); and on `step_permission` the only insert trigger, `AFTER
    INSERT … WHEN (NEW.status = 'pending')`, with no update trigger (no permission update writes an
    event). No product insert creates a row that should write an event (`run` is inserted `queued`
    or chat `running`, `run_step` `pending` or chat `running`, items `open`), while the fixtures'
    parked/failed/blocked rows that `PgStore::load_demo` inserts into every `demo_db` must write
    none (D13). Cancels, supersedes and `settle_permissions` queue no deferred call at all.
    Postgres refuses `OLD` in the `WHEN` of a trigger that covers `INSERT` (fact-check probe), so
    insert and update are never one trigger. Deferred, so each fires at commit and **re-reads the
    row's current state**: the order inside `park_step`'s transaction (step then run, or run then
    step) cannot turn a gate into an "interrupted" event, and a status that left the waiting state
    before commit writes no needs-you event. Because every queued call re-reads the same final
    row, a row whose status moves twice in one transaction (A→B→C, or park → unpark → park) queues
    two calls that would write two identical events (fact-check probe), so each function writes
    only when its `NEW.status` equals the row's current status, and at most once per transaction,
    kind and subject *(blueprint F3)*: the table carries `txid xid8 NOT NULL DEFAULT
    pg_current_xact_id()` and `subject UUID NOT NULL` (a copy, not an FK), a unique index
    `uq_activity_event_once (txid, kind, subject)`, and each function inserts with `ON CONFLICT DO
    NOTHING`. `pg_current_xact_id()` is the top-level xid even inside a subtransaction; the `xmin`
    guard first planned is the subtransaction's xid and failed the probe inside an `EXCEPTION`
    block. No function has an `EXCEPTION` block: a raise is a bug, caught by the conformance cases
    and the full store suite. A single-statement writer (`transition_step`, `transition_run`,
    `fail_run`) commits at once and is covered unchanged. `replace_graph`'s `SET CONSTRAINTS ALL
    IMMEDIATE` (`pg/write.rs:5129`) would fire queued calls early, but it writes no status.
    Precedent for triggers on these tables: the `updated_at` triggers (`0001_init.sql:578`) and
    M2's `box` trigger (`0032_worker_log.sql`).
  - **MemStore:** every mutation runs through `MemStore::write` (`htui-core/src/store/mem.rs:1537`,
    the one `state.write()` in the file). It snapshots the statuses of runs, steps, items and
    permissions before `f`, diffs after, and appends events under the same lock with D3's rules:
    the same "final state of the write" semantics as the deferred trigger. *(Blueprint F5:)* the
    snapshot is status maps only (`StatusSnapshot`, below in T1); a changed status passes F2's
    status filter, then D3; a row that appeared during the write is an event source only for
    `permissions` (a new `pending` id writes `permission`), as the triggers (F2); a removed id
    writes nothing (deletion is D7's, in `remove_project`). Accepted divergence: Postgres writes an
    event for a row that moves B→C→B inside one transaction, the diff sees B→B; no store method
    does that today.
  - Re-parks write nothing: a second `park_step` on a parked step returns `StepMoved` before any
    write (`pg/write.rs:9318-9320`), and the engine's recovery re-park (`engine.rs:3810`) moves the
    run `running → awaiting_approval` while its step is already `awaiting_approval`, so the run
    rule ("no step of the run `awaiting_approval`") writes no run-level event.
- **D3 — kinds and the transition table.** `ActivityKind` (stored as text, `chk_activity_event_kind`):

  | Transition (final state at commit) | Kind | Class | Subject / jump ids |
  |---|---|---|---|
  | `run_step.status` → `awaiting_approval` | `gate` | needs you | step, run, item |
  | `run.status` → `awaiting_approval`, no step of the run `awaiting_approval` | `judge_failed` / `selection` / `interrupted` (rule below) | needs you | run (+ the rule's step) |
  | `item.status` → `blocked` | `unblock` | needs you | item (+ its newest active run) |
  | `step_permission` inserted with `pending` | `permission` | needs you | permission, step, run |
  | `run_step.status` → `failed` | `step_failed` | failure | step, run |
  | `run.status` → `failed` | `run_failed` | failure | run (+ its newest step) |
  | `run.status` `queued` → `running`, `kind = 'graph'` | `run_started` | plain | run |
  | `run.status` → `done`, `kind = 'graph'`, item present and `item.status <> 'closed'` | `close_out` | plain | run, item |
  | `run.status` → `done`, `kind = 'graph'`, otherwise | `run_finished` | plain | run |

  The run-level park rule mirrors `htui_worker::views::waiting` (`htui-worker/src/views.rs:616-839`,
  re-exported as `htui_worker::waiting`; the run rules are `:666-781`),
  which needs the snapshot cursor in Rust and cannot run in SQL, so the store approximates it,
  first match wins: **`judge_failed`** when the run has a judge step (`fanout_index = -1`,
  `htui-orch/src/status.rs:491` `judge_at`) with status `failed` and a `gate_note` (`views.rs`
  takes a failed judge with no note as a selection, `:696-703`) in a slot `(position, attempt)`
  with no `selected = true` candidate (event step = the judge); **`selection`** when the newest slot
  holding two or more candidates (`fanout_index >= 0`) has none `selected = true` and none
  `running`/`pending` (event step = the slot's first candidate); else **`interrupted`** (the engine's
  `park_interrupted`, `engine.rs:4349`; event step = the run's most recently updated `failed` step,
  or none). One pure Rust function `park_kind(&[RunStep]) -> (ActivityKind, Option<StepId>)` in
  `htui-core/src/model/activity.rs` implements it for MemStore; the plpgsql function implements the
  same rule. *(Blueprint F1:)* parity is two links: T1's pure test in `views.rs`'s test module
  pins `park_kind` == `waiting` (with a named divergence list), and the conformance case
  `run_park_kinds`, run on both stores, pins plpgsql == `park_kind`.
  `cancelled` and `superseded` write nothing (a person or a retry did it). `interrupt_step` sets
  `failed`, so a step interrupted by a restart is a `step_failed` and its run's park an
  `interrupted`: two events, both true.
  Text per kind: the step's `gate_note` (gate, judge failed, step failed), the permission's
  `summary` (permission), `run.failure` (run failed), else empty; cut to 500 chars at a char
  boundary. These strings are already stored and shown by the Runs pane and the waiting overlay;
  no new text is formed.
- **D4 — how a needs-you event stops being pinned: derived at read time.** *check* `pinned` is
  computed by `activity` from the subject's current state, never stored: a `gate` while its step is
  `awaiting_approval`; `permission` while its row is `pending`; `unblock` while its item is
  `blocked`; the three run kinds while the run is `awaiting_approval` with no step parked; and only
  the newest needs-you event per subject (on Postgres `DISTINCT ON (subject) … ORDER BY subject,
  id DESC`, blueprint F3). Rejected: a `resolved_at` set by the transition that ends
  the wait. Waits end on about eight paths (`answer_gate`, `select_fanout`, `supersede_step`,
  `transition_run` back to `running` from at least six engine sites, `finish_run`, cancel, item
  `blocked → open`, `resume_run`, `answer_permission`/`apply_permission`/`settle_permissions` and
  the stale sweep, `pg/relay.rs:190-347`); one missed path leaves a stale pin forever, while a
  derived pin cannot go stale. It also matches `R-TUI-11` ("computed from the store at read time").
  Cost: one indexed query per poll over a partial index. The history row stays in the feed after
  its pin goes.
- **D5 — workspaces.** Items, runs and documents carry `project_id`, never a workspace; a project
  can sit in several workspaces (`workspace_project` PK, `0001_init.sql:161-167`). The trigger stores
  `workspace_id` = `run.workspace_id` (`0019_stop_runs.sql:34`, the workspace a graph run started
  from), else the project's `owner_workspace_id` (`0026_workspace_scope.sql`, a workspace's own
  scope row), else the only `workspace_project` row of the project, else `NULL`. Filters:
  - *this workspace*: `event.workspace == Some(scope.workspace_id) || event.project` is in
    `scope.project_ids` (`Scope::contains`, `htui-core/src/model/scope.rs:32`);
  - *needs you*: pinned rows only; *failures*: `step_failed`/`run_failed`; *all workspaces*:
    everything. Filtering is in the panel over the rows it holds, not in SQL.
  - A row outside the current workspace shows its workspace's slug (`@slug`); `NULL` resolves to
    the first workspace, by name, whose projects contain the event's project; none → `@?`.
  - Reads are the user's own rows (`user_id` = `run.started_by`, or `item.created_by` for an item
    event), as M2's reads are.
- **D6 — retention and sweeping.** *check* Events share M2's key `app_setting`
  `log_retention_days` (1–365, default **14**; `htui/src/worker_log/ship.rs:23-25`, `retention_days`
  `:64-69`) and add a cap of **100 000 events per user**. One retention number covers stored logs
  and events, as M2 D4 promised. Sweeps (each idempotent, no lock):
  - the TUI: the shell sends `StoreRequest::SweepRetention` on the first refresh tick and then
    every `SWEEP_TICKS` (one hour of ticks, blueprint F10); the store worker prunes
    `activity_event` **and** `worker_log` with the same cut through `backend.writer()` (Offline →
    `RetentionSwept(0)`, blueprint F6) (today only a running worker sweeps, `ship.rs:111-129`);
  - each worker: its shipper's hourly sweep also calls `prune_activity`.
  It does not wait on MOD-100's sweeper (open, no code). *(PRD open question "default retention and
  sweeper" answered for events.)* The Settings surface is T7: the key joins the App rung of the
  typed settings registry (`SettingKey`, `htui-core/src/prompt/settings.rs:189`), so it is edited in
  Settings › Prompt beside `memory_capture` and `light_model` until MOD-123 regroups Settings.
- **D7 — deletion with the item, errors excepted.** *(PRD open question "errors" answered: for
  events, the failure kinds `step_failed` and `run_failed`; M2 D5 settled log lines.)* No product
  path deletes an item (`pg/write.rs:16`, conformance `no_delete_path`): items go with
  `delete_project`/`delete_workspace` (`pg/write.rs:6599`, `:6570`; the deletes `:11734-11788`) by `ON DELETE CASCADE`.
  So, as M2 D6 did for boxes, it is a schema guarantee:
  - `item_id`, `run_id`, `step_id`, `permission_id`, `project_id`, `workspace_id` are
    `ON DELETE SET NULL`; the row keeps copies `project_slug`, `item_key`, `step_name`.
  - `BEFORE DELETE` triggers on `item` and on `run` delete that item's / run's events whose kind is
    not a failure, so the FK clauses only ever touch failures, which stay attributable by the copies
    and expire at retention. The run trigger covers a free chat (no item).
  - `DeleteReach` (`htui-core/src/store/traits.rs:4402-4483`, struct `:4414`) is **not** extended: the feed is not
    data a person made, and what survives is exactly the failures. The twin test
    `every_cascade_table_loses_exactly_what_the_report_names` (`htui-store/tests/it/pg_criteria.rs:3198`)
    is left as is; T2's own test pins the delete.
  - MemStore *(blueprint F5)*: rows are only ever removed in `State::remove_project`
    (`mem.rs:6013-6127`, reached by `delete_project` and `delete_workspace`), which already mirrors
    every `ON DELETE` with its `gone` reach; the rule goes there, beside its sibling cascades, not
    in the `write` diff: drop the non-failure events whose `item`/`run` is in
    `gone.items`/`gone.runs`; on the survivors set `item`/`run`/`step`/`permission` to `None` when
    the referenced row is gone and `project` to `None` for the project. `delete_workspace` sets
    `workspace` to `None` on **every** surviving event of that workspace, whatever its kind.
- **D8 — the panel.** Shell-owned, one per `App` (events are global, not a tab's):
  `crates/htui/src/ui/shell/activity.rs`, `ActivityPanel`. Drawn by `App::paint_panes`
  (`htui/src/app/state.rs:792-827`) in place of the placeholder (`ACTIVITY_*` constants
  `state.rs:1769-1772`), in the slot `ui/shell/layout.rs:53-54` already lays out.
  - Rows: pinned first (newest first) under a `needs you` divider, then the rest newest first, a
    pinned event not repeated below. A row: glyph + word (`⚠ needs you`, `✗ failed`, `◐ started`,
    `● finished`, `✓ ready to close`, never colour alone), local `HH:MM`, `@slug` when not the
    current workspace, `KEY` or `chat`, the step name, the kind's label and the text, cut with `…`.
  - Title: `Activity · ⚠ N needs you · <filter>`, N = pinned rows under the current filter; no
    `· ⚠ …` part when N = 0.
  - Holds at most **500** events (`CAPACITY`, defined from T1's `ACTIVITY_PAGE`); the first read takes the newest 500 (`after: None`),
    later reads re-read from `max_id − 200` and de-duplicate by id (M2 D10's late-commit overlap,
    `htui/src/ui/tabs/logs/workers.rs` `merge`). `pinned` is replaced wholesale each reply.
  - Poll: in `App::on_tick` every `TICKS_PER_REFRESH` (`htui/src/app/update.rs:28`, ~1 s), only while
    the active tab shows the activity pane, one request in flight at most, and at once when it
    becomes visible. Reply handled in `App::on_app_reply` (`update.rs:621`), not `observe_reply`
    (that is the top bar's read-only look before the freshness gate; blueprint F9): `Activity` is
    only sent as `Origin::App`, which is never `forget`-ed, so every reply passes the gate. A failed
    read is absorbed in `App::absorbed` (`update.rs:485-497`, its `Origin::App` arm gains
    `"activity"` and `"sweep_retention"`, free of side effects) and `on_app_reply` clears in-flight
    on `Failed { request: "activity" }`, so a down database does not fill the status line once a
    second; the last rows stay. Offline: empty page.
  - Filter: session state, default *all workspaces*; not saved (R-TUI-12's saved "panel state" is the
    about panel's).
  - Empty: `empty::render` with a next step ("events appear as runs start, wait and fail"); with a
    filter that hides everything, it names `f`.
- **D9 — Enter and history.** Enter on a row:
  - a chat event (`chat = true`) with a step → `Action::Replay { step_id }` (`app/action.rs:39`),
    the existing way to open a chat's transcript, workspace-independent (the Chat tab has no
    per-session reveal yet; MOD-121);
  - an item event whose project is in the current scope → `Action::Reveal(RevealTarget::Step { item,
    key, run, step })` (`action.rs:167-195`), the Ctrl+W overlay's jump (`ui/overlay/waiting_list.rs:130-141`);
  - an item event in another workspace → the shell sets `SetScope` to that workspace (D5's
    resolution; `App::set_scope` `update.rs:349-366`) and then reveals, in that order (the Backlog's
    `on_scope_change` clears a pending reveal, `ui/tabs/backlog/mod.rs:337-343`), and sets the
    status line `opened workspace <name> for <KEY>`; when `set_scope` is refused (an in-pane editor
    is open, `update.rs:52-55`) nothing moves and the status line says why;
  - a row whose item or run is gone (refs `NULL`) → status `<KEY> was deleted`, nothing moves.
  Recording: keys already run inside `App::recorded` (`state.rs:705-720`, `:1406`), so a jump from
  the activity pane to the Backlog's detail is one history entry and back returns to the activity
  pane (`App::here` labels it ` · activity`, `state.rs:679-698`). *check* `Loc` holds no workspace
  (`ui/shell/history.rs:20-29`): back after a cross-workspace jump returns to the place but stays in
  the opened workspace, as the `w` switcher does today. The workspaces list rides the activity reply
  (`StoreReply::Activity { page, workspaces }`), so Enter needs no extra round trip.
- **D10 — keys.** Every key in `keys.toml`, none reserved. A new `Context::Activity` (name
  `activity`, title `Activity`), mirroring `Context::Logs` (`keys/catalogue.rs:162-163`, `:236`,
  `:297`, `:360`, `:416`) with two acts:
  `row(Act::ActivityOpen, Activity, "open", &["enter"], "open the item or chat")` and
  `row(Act::ActivityFilter, Activity, "filter", &["f"], "all workspaces · needs you · failures · this workspace")`.
  Stack `ACTIVITY_BROWSE` in `keys/views.rs`, mirroring `LOGS_BROWSE` (`:1218-1230`):
  `[Layer::view(Context::Activity, &[ActivityOpen, ActivityFilter]), Layer::only(Context::List,
  &[ListDown, ListUp, ListTop, ListBottom]), LAYOUT, HISTORY, GLOBAL]` (`ListBottom` per blueprint
  F8: Logs leaves it out only because of `LogsBottom`, and no layer of this stack binds `G`).
  Leaving out the `List` fold and the
  Backlog context frees `enter` and `f` there (`f` is only `BacklogOpenFilter`, `catalogue.rs:1999`).
  Routing *(blueprint F8)*: today `App::dispatch_key` (`state.rs:1410-1502`) runs 1. the editor,
  2. `translate` through `active_stack()`, 3. the overlay block, 4. `tab.on_key`, 5.
  `apply_keys(tab stack)`, even with the activity pane focused, so a `j` moves the hidden Backlog
  list. M3 adds a shell step between 3 and 4, guarded by `fn activity_has_keys(&self) -> bool`:
  no overlay, the active tab's `focus_of(tab) == PaneId::Activity`, the tab's layout shows the
  pane, and **not** `tab.key_stack().and_then(|s| s.global()).is_some_and(|(_, l)| l.is_modal())`
  (the "T7 repair" in `paint_panes`, `:798-806`, draws an open modal form over a zoomed activity
  pane, and the form keeps the keys). The step runs `for act in self.keys.actions(ACTIVITY_BROWSE,
  chord)`: the panel's acts (`ActivityOpen`, `ActivityFilter`, `ListDown`/`ListUp`/`ListTop`/
  `ListBottom`) go to `self.activity.on_act(act, &self.scope)` and then the D9 jump; otherwise
  `self.apply_keys(ACTIVITY_BROWSE, chord)`; it then **returns either way**, so the Backlog never
  sees a key while the activity pane holds them. No literal chord: it resolves only through
  `Keys::actions`. `App::active_stack` (`state.rs:979-984`) returns `ACTIVITY_BROWSE` under the
  same predicate (so `translate`, the `?` box, the shell chips and `keys_line` follow), and the
  activity pane's bottom-border hint (`state.rs:820-823`, today the tab's `key_hint`) uses it too.
  Pane focus keeps using `global.next_pane`/`prev_pane`.
- **D11 — default presets.** *check* Backlog's default becomes `Preset::IdeActivity`
  (`BACKLOG_LAYOUT`, `ui/tabs/backlog/mod.rs:79`), as the UI review's round 7 decided ("Backlog with
  the activity panel", `docs/ui-redesign-findings-2026-10-15.md` §3.4 line 102). Logs, Requirements
  and Skills stay `Ide`; Chat and Settings are single-pane (`ui/tabs/registry.rs:155`). A layout
  already saved in `ui-state.toml` wins over the default (`state.rs:152-155`). *(PRD open question
  "which tabs default to the panel" answered.)*
- **D12 — chat runs.** *check* A chat run writes needs-you (`permission`) and failure events, never
  `run_started`/`run_finished`: every chat turn would otherwise add two plain rows and bury the feed.
  The D3 table's `kind = 'graph'` conditions encode it.
- **D13 — what is not in M3.** No notifications or bell (MOD-114 consumes these events later). No
  `LISTEN`/`NOTIFY` (MOD-46). No background-workspace lifecycle (MOD-116); the feed reads every
  workspace regardless. No mouse row selection in the panel (a click focuses the pane, as today). No
  demo-data events (the demo's store writes events as soon as anything moves). No search or export.
  `htui waiting` and the Ctrl+W overlay are unchanged.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Narrow store trait | `htui-core/src/store/worker_log.rs:21-82`; `MemStore` impl `mem.rs:11907`, state `:397-401`; `pg/worker_log.rs:25`; `Writer` `htui-store/src/worker.rs:529`; `Backend::worker_logs` `backend.rs:597-611` | Feature trait, two stores, `Writer` forwards per arm, an inherent `Backend` read (Offline → empty) |
| Conformance | `WORKER_LOG_CASES` `htui-core/src/store/conformance.rs:1582-1588`, `run_worker_log_case` `:1596`, `run_all_worker_logs` `:1631`, the name-uniqueness/count unit test `:34303-34326`; bound `htui-core/tests/it/mem_store.rs:30` (count `:110`), `htui-store/tests/it/pg_conformance.rs:168-188` | One case body, run for MemStore and Postgres |
| Migration with triggers | `crates/htui-store/migrations/0032_worker_log.sql` (table, `chk_`, `BEFORE DELETE` trigger), `0011_permission_relay.sql` header | Header comment, forward-only, `clock_timestamp()`, "not mirrored" |
| Waiting classifier | `htui-worker/src/views.rs:616-839` `waiting` (public as `htui_worker::waiting`); tests `an_interrupted_park_is_a_row` `:1936`; private fixtures `slot`, `interrupted_park`, `resolved_selection`, `undecodable` `:1588-1645`; `every_parked_run_owns_a_row` `:2148-2177` | What each reason means; parity fixtures (F1) |
| Cross-store park shapes | `select_fanout_is_one_transaction` helpers in `conformance.rs` (`gated_step`, `new_run_step(run, pos, att, idx)`, `answer_gate(.., Rejected, Some(note))`, `:14242`) | Build fan-out shapes through store writes on both stores (F1) |
| Store request | `htui/src/store_worker.rs` `WorkerLogs` (`:478`, name `:1431`, reply `:1902`, serve `:2660`), `Workspaces` (serve `:2498`, name `:1384`) | Variant + `name` arm + reply + serve arm |
| Shell-owned poll | `App::on_tick` `update.rs:294-311` (`Waiting`), `on_app_reply` `:621` (not `observe_reply` `:500-525`, blueprint F9), `absorbed` `:485-497` | App-origin request, reply after the gate in `on_app_reply`, absorbed failure |
| Cursor merge | `htui/src/ui/tabs/logs/workers.rs` `after`, `merge` | 200-row overlap, de-dup by id |
| Jump | `ui/overlay/waiting_list.rs:130-141` `WaitingList::enter`; `App::reveal` `update.rs:413-434` | `Reveal(Step{…})`; registration `app/mod.rs:150-153` |
| Keys | `Context::Logs` and `Act::Logs*` (`keys/catalogue.rs:162`, `:876-881`, `:2332-2334`, test tables `:2754`, `:3022`, `:3278`, `:3691`, `:3750`); `LOGS_BROWSE` `keys/views.rs:1218-1230`, registries `:1350`, `:1407`, `:1501` | New context, acts, rows, stack, the bookkeeping the invariant tests demand |
| Pane drawing | `App::paint_panes` `state.rs:792-827`; kit `Pane`, `empty::render` | Pane title, focus accent, hints in the bottom border |
| App setting key | `SettingKey` `htui-core/src/prompt/settings.rs:189-250` (`MemoryCapture`, `LightModel`), `htui/src/prompt_settings.rs:153-156`, `ui/tabs/settings/prompt.rs:1202` | Typed App-rung key in byte order, validator, row in Settings › Prompt |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/activity.rs` | CREATE | T1 | `ActivityKind`, `ActivityClass`, `ActivityEvent`, `ActivityPage`, `park_kind`, `PINNED_LIMIT`, `ACTIVITY_PAGE` (F7) |
| `crates/htui-core/src/model/mod.rs` | UPDATE | T1 | export |
| `crates/htui-core/src/store/activity.rs` | CREATE | T1 | `ActivityStore` (D1), `PER_USER_CAP` (F7) |
| `crates/htui-core/src/store/mod.rs` | UPDATE | T1 | export |
| `crates/htui-core/src/store/mem.rs` | UPDATE | T1 | state, `write` status-snapshot hook (D2, F5), D7 in `remove_project`/`delete_workspace` (F5), impl, `push_activity` |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | T1 | `ACTIVITY_CASES` (cross-store park shapes, F1) |
| `crates/htui-core/tests/it/mem_store.rs` | UPDATE | T1 | bind the cases |
| `crates/htui-worker/src/views.rs` (test module only) | UPDATE | T1 | `park_kind_agrees_with_waiting` parity test (F1) |
| `crates/htui-store/migrations/0034_activity_event.sql` | CREATE | T2 | table, `txid`/`subject` guard (F3), indexes (F4), status triggers (F2), delete triggers |
| `crates/htui-store/src/pg/activity.rs`, `src/pg/mod.rs` | CREATE / UPDATE | T2 | the two statements |
| `crates/htui-store/src/worker.rs` | UPDATE | T2 | `impl ActivityStore for Writer` (T3's sweep uses it, F6) |
| `crates/htui-store/src/backend.rs` | UPDATE | T2 | `Backend::activity` (no `prune_retention`, F6) |
| `crates/htui-store/.sqlx/query-*.json` | CREATE | T2 | offline entries |
| `crates/htui-store/tests/it/activity_pg.rs`, `tests/it/main.rs` | CREATE / UPDATE | T2 | triggers, deletes, pins on Postgres |
| `crates/htui-store/tests/it/pg_conformance.rs` | UPDATE | T2 | bind `ACTIVITY_CASES` |
| `crates/htui-store/tests/it/connect.rs`, `tests/it/migrations.rs` | UPDATE | T2 | count pins 33 → 34, `TABLES` 54 → 55 |
| `crates/htui/src/store_worker.rs` | UPDATE | T3 | `Activity`, `SweepRetention` |
| `crates/htui/src/worker_log/ship.rs` (+ its test fake) | UPDATE | T3 | worker sweep prunes events too |
| `docs/htui-worker.md` | UPDATE | T3 | retention covers activity events |
| `crates/htui/src/ui/shell/activity.rs`, `ui/shell/mod.rs` | CREATE / UPDATE | T4 | `ActivityPanel` |
| `crates/htui/src/keys/catalogue.rs`, `keys/views.rs`, `docs/htui-keys.md` | UPDATE | T4 | D10 |
| `crates/htui/src/app/state.rs`, `app/update.rs` | UPDATE | T5 | slot, key routing, poll, reply, absorb, Enter, sweep timer |
| `crates/htui/tests/it/activity.rs`, `tests/it/main.rs`, `tests/it/snapshots/it__activity__*.snap` | CREATE / UPDATE | T5 | end-to-end panel tests |
| `README.md` | UPDATE | T5 | the activity panel, one paragraph |
| `crates/htui/src/ui/tabs/backlog/mod.rs` + every moved `.snap` | UPDATE | T6 | D11 |
| `crates/htui-core/src/prompt/settings.rs`, `crates/htui/src/prompt_settings.rs`, `crates/htui/tests/it/prompt_settings.rs`, `ui/tabs/settings/prompt.rs` + its snapshots | UPDATE | T7 | `SettingKey::LogRetentionDays`; key-parity test with `ship.rs` (F13) |

## Tasks

**Order and lanes.** **T1 first, alone (foundation, one verifier).** Then two lanes in separate
worktrees: **lane A: T2 → T3** (T3 needs T2's `Backend::activity`), **lane B: T4** (needs only T1's
model types), and **lane C: T7** (independent of everything; may start at once, even beside T1).
Then **T5** after lanes A and B merge (it uses T3's requests and T4's panel), then **T6** last
(snapshot churn, after every other UI change). File sets: T1 ∩ T2 = ∅ (htui-core plus
`htui-worker/src/views.rs`'s test module vs htui-store; T1 no longer touches
`htui-worker/tests/it/main.rs`, F1); T2 ∩ T3 = ∅; T3 ∩ T4 = ∅ (`store_worker.rs`, `ship.rs`, `docs/htui-worker.md` vs
`ui/shell/*`, `keys/*`, `docs/htui-keys.md`); T7 ∩ every other = ∅ (`prompt/settings.rs`,
`prompt_settings.rs`, `settings/prompt.rs`). T5 and T6 are serial by design.
The generated `.sqlx` entries are T2's only. The shared constants (`ACTIVITY_PAGE`,
`PER_USER_CAP`) are T1's (F7), so no lane needs a type or constant from another lane: T4 uses
`ActivityPage`, `ActivityEvent`, `WorkspaceSummary`, `RevealTarget`, `Scope` and its own `Act`s, no
`StoreReply`; T3 uses T2's `Backend::activity` and `Writer` impl (same lane) plus T1's types. T7's
key-parity test (F13) reads `ship.rs` (T3's) without changing it.

### Task 1: Model, trait, MemStore writer, conformance (foundation; serial, first; one verifier)

Owns: `crates/htui-core/src/model/activity.rs` (new), `crates/htui-core/src/model/mod.rs`,
`crates/htui-core/src/store/activity.rs` (new), `crates/htui-core/src/store/mod.rs`,
`crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/it/mem_store.rs`, `crates/htui-worker/src/views.rs` (its `mod tests`
only; no other task touches the file; F1).

Types (`model/activity.rs`; `str_enum!` as `QueueSetting` uses it, `model/queue_settings.rs:9`):
```rust
str_enum!(
    /// Plan D3: one activity event kind, by its stored name.
    ActivityKind {
        Gate => "gate", JudgeFailed => "judge_failed", Selection => "selection",
        Unblock => "unblock", Interrupted => "interrupted", Permission => "permission",
        StepFailed => "step_failed", RunFailed => "run_failed",
        RunStarted => "run_started", RunFinished => "run_finished", CloseOut => "close_out",
    }
);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityClass { NeedsYou, Failure, Plain }
impl ActivityKind {
    pub const fn class(self) -> ActivityClass;
    /// The panel's words: "gate", "judge failed", … , "ready to close".
    pub const fn label(self) -> &'static str;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityEvent {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub kind: ActivityKind,
    pub workspace: Option<WorkspaceId>,
    pub project: Option<ProjectId>,
    pub project_slug: String,
    pub item: Option<ItemId>,
    pub item_key: Option<String>,
    pub run: Option<RunId>,
    /// `run.kind = 'chat'`.
    pub chat: bool,
    pub step: Option<StepId>,
    /// The step's `phase_name`.
    pub step_name: Option<String>,
    pub permission: Option<PermissionId>,
    pub text: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityPage { pub events: Vec<ActivityEvent>, pub pinned: Vec<ActivityEvent> }
pub const PINNED_LIMIT: u32 = 200;
/// F7: one page of events: T3's `ACTIVITY_LIMIT` and T4's `CAPACITY` are defined from it.
pub const ACTIVITY_PAGE: u32 = 500;
pub const TEXT_MAX_CHARS: usize = 500;
/// D3's run-level park rule over the run's steps.
pub fn park_kind(steps: &[RunStep]) -> (ActivityKind, Option<StepId>);
```
`ActivityEvent` does not expose Postgres's `txid` or `subject` columns (F3); MemStore needs
neither. `ActivityStore` as D1, and beside it in `store/activity.rs` (F7):
```rust
/// D6: events kept per user; used by T2's tests, T3's TUI sweep and the shipper's sweep.
pub const PER_USER_CAP: u32 = 100_000;
```
`#[cfg(feature = "test-support")] pub fn push_activity(&self, event: ActivityEvent) -> i64` on `MemStore` (assigns the id and `at`).
MemStore hook (F5):
```rust
struct StatusSnapshot {
    runs: HashMap<RunId, RunStatus>,
    steps: HashMap<StepId, StepStatus>,
    items: HashMap<ItemId, Status>,
    permissions: HashSet<PermissionId>,
}
```
All `Copy` values, built with `with_capacity`. `MemStore::write` becomes `let now = self.now();`
(MemClock, no lock) **before** taking the guard, then `let before = guard.status_snapshot(); let r
= f(&mut guard); guard.record_activity(&before, now); r`. `record_activity` walks the *after* maps:
a changed status passes F2's status filter (D2), then D3; a new permission id with `pending`
writes `permission`; a removed id writes nothing. It is a `State` method, so it reaches no
`self.read` and no `this_user()` (deadlock, claim 48). Its doc comment records the accepted
divergence (B→C→B in one transaction: Postgres writes, the diff sees B→B; if a conformance case
finds such a write, flag it rather than paper over it). Rejected: version counters (140
`self.write(` sites write through `&mut State` fields, no setter to hook); a scratch snapshot held
in `State`, unless T1's before/after timing of `cargo test -p htui-core --lib` shows more than
5 % (report the number either way).

- **Action**: tests first.
  - `park_kind` unit tests: a failed judge in an unselected slot → `JudgeFailed` + the judge's id; an unselected slot of two done candidates → `Selection` + the first candidate; a run whose newest step `failed` with no slot → `Interrupted` + that step; no steps → `Interrupted, None`; a slot with a `selected = true` candidate is never `Selection`.
  - Conformance cases `ACTIVITY_CASES` (generic over `S: WriteStore + ActivityStore`, registered like `WORKER_LOG_CASES`), each driving real store writes:
    - `gate_is_pinned_until_answered`: `park_step` → one `gate` event, pinned; `answer_gate` → still in `events`, no longer pinned.
    - `run_park_kinds` (F1): the three D3 shapes through `transition_run(Running → AwaitingApproval)` give `judge_failed`, `selection`, `interrupted`; back to `Running` unpins. Build the shapes with the `select_fanout_is_one_transaction` helpers (`gated_step`, `new_run_step(run, pos, att, idx)`, `answer_gate(.., Rejected, Some(note))` for a failed judge with a note, `conformance.rs:14242`). Read the steps back and assert the written kind and step equal **both** the literal expectation and `park_kind(&steps)` (on Postgres this pins plpgsql == `park_kind`). Also cover: a failed judge with no note → `selection` (`views.rs:696-703`); a slot with a `running` candidate → `interrupted`; two slots where only the newer one is unselected → `selection` on the newer slot. The fixture's own parks write `gate` events, so assert on the run-subject event only. Gotcha (M2 F2): `every_cross_referenced_test_name_exists` panics if a `conformance.rs` doc comment names a `views.rs::…` test; put that cross-reference in `model/activity.rs`.
    - `blocked_item_is_unblock`: item → `blocked` pinned; → `open` unpinned.
    - `permission_pending_is_pinned`: `open_permission` pinned; `answer_permission` unpins.
    - `failures`: `transition_step(Running → Failed)` → `step_failed` with the gate note as text; `finish_run(Failed)` → `run_failed` with `run.failure`.
    - `plain`: `claim_run` → `run_started`; `finish_run(Done)` with an open item → `close_out`; with a closed item → `run_finished`; a chat run's start and finish write nothing, its failure writes `run_failed` with `chat = true`.
    - `same_status_writes_nothing`: a second `park_step` on a parked step adds no event.
    - `newest_pin_per_subject`: park, unpark, park the same run → two events, one pinned.
    - `paging`: ids rise; `after` exclusive; `None` = newest `limit` oldest-first; another user's events are never read.
    - `prune`: older than the cut go; the per-user cap keeps the newest.
    - `delete_project_keeps_failures`: after `delete_project`, plain and needs-you events of its items and runs are gone; failure events remain with `item`/`run`/`step`/`project` `None`, `item_key`/`project_slug` intact.
    - Text over 500 chars is cut at a char boundary.
  - Extend `delete_project_leaves_no_row_in_any_map` (`mem.rs:14651`, F5): every activity row that survives is a failure with every reference `None`.
  - `park_kind_agrees_with_waiting` in `crates/htui-worker/src/views.rs`'s `mod tests` (F1; pure, no store: `waiting`'s run rules call the private `selectable_slots` (`views.rs:455`) over a decoded snapshot, which store writes cannot build cheaply, and the fixtures `slot`, `interrupted_park`, `resolved_selection`, `undecodable` (`:1588-1645`) are private to that module). Walk the fixture list of `every_parked_run_owns_a_row` (`:2148-2177`); for each run that is `AwaitingApproval` with no step parked and that owns a `JudgeFailed`/`Selection`/`Interrupted` row, assert `park_kind(steps) == (row.reason as kind, row.step)`. The same test pins a named **divergence list**, so any drift fails it:
    - `undecodable(...)` shapes: `waiting` says `Interrupted` with no rest step; `park_kind` reads no snapshot;
    - a slot whose candidates are all failed, with a failed judge: `waiting` says `Interrupted`, `park_kind` says `JudgeFailed`;
    - the interrupted step: `waiting` uses the cursor's rest step, `park_kind` the newest `failed` step; assert that they agree on `interrupted_park`;
    - a run that already owns an `Unblock` row (`crashed_rejection`, `escalation`): `waiting` adds no run row, while the store writes `interrupted` beside `unblock`.
- Then implement: `State` gains `activity: Vec<ActivityRow>` and `activity_last_id: i64` (as `worker_logs` at `mem.rs:397-401`, initialised `:535`); `MemStore::write` (`:1537`) wraps `f` as the hook above (snapshot → `f` → `State::record_activity(&before, now)`, F5); each event's user is its row's (`run.started_by`, `item.created_by`, D5), never `this_user()`, which takes the read lock (`mem.rs:11912-11913`) and would deadlock inside `write`; D7's MemStore half in `State::remove_project` (`mem.rs:6013-6127`) and `delete_workspace` as D7 states (F5); `impl ActivityStore for MemStore` beside the `WorkerLogStore` impl (`:11907`).
- **Mirror**: `WorkerLogStore` and its MemStore impl; `run_all_worker_logs`.
- **Validate** (crate-scoped):
  `cargo test -p htui-core --all-features --lib -- model::activity store::mem`;
  `cargo test -p htui-core --all-features --test it mem_store::`;
  `cargo test -p htui-worker --all-features --lib -- views::tests::park_kind` (F1);
  `cargo test -p htui-core --lib` timed before and after (F5; report the numbers);
  `cargo clippy -p htui-core -p htui-worker --all-targets --all-features -- -D warnings`; `cargo clippy -p htui-core -- -D warnings` (featureless); `cargo fmt -p htui-core -p htui-worker`.

### Task 2: Postgres table, triggers, store, pins (lane A, after T1; worktree)

Owns: `crates/htui-store/migrations/0034_activity_event.sql` (new), `crates/htui-store/src/pg/activity.rs` (new), `crates/htui-store/src/pg/mod.rs`, `crates/htui-store/src/worker.rs`, `crates/htui-store/src/backend.rs`, `crates/htui-store/.sqlx/query-*.json` (new entries), `crates/htui-store/tests/it/activity_pg.rs` (new), `crates/htui-store/tests/it/main.rs`, `crates/htui-store/tests/it/pg_conformance.rs`, `crates/htui-store/tests/it/connect.rs`, `crates/htui-store/tests/it/migrations.rs`.

Signatures:
```rust
// pg/activity.rs
impl htui_core::store::ActivityStore for PgStore { /* D1 */ }
// worker.rs, beside `impl WorkerLogStore for Writer` (:529)
impl htui_core::store::ActivityStore for Writer { /* per arm */ }
// backend.rs, beside `worker_logs` (:597)
impl Backend {
    /// Online → Postgres, Memory → MemStore, Offline → an empty page.
    pub async fn activity(&self, after: Option<i64>, limit: u32) -> Result<ActivityPage>;
}
```
No `Backend::prune_retention` (F6): `PER_BOX_CAP` lives in `htui/src/worker_log/ship.rs:27`, which
htui-store cannot import, and there is no `Backend` prune today (only `Writer` forwards
`prune_worker_logs`, `worker.rs:558`). T3 sweeps through `backend.writer()` and this task's
`impl ActivityStore for Writer`, whose only consumer is T3. `PER_USER_CAP` is T1's
(`htui_core::store`, F7); worker logs keep `PER_BOX_CAP` (`ship.rs:27`).

- **Action**: tests first.
  - Bind `ACTIVITY_CASES` in `pg_conformance.rs` as `WORKER_LOG_CASES` is (`:168-188`). On Postgres `run_park_kinds` is what pins the plpgsql park rule against `park_kind` (F1).
  - `activity_pg.rs` (Postgres only, `HTUI_TEST_DATABASE_URL`, `testkit::fresh_db`):
    - in one explicit transaction, update the run to `awaiting_approval` *before* the step: one `gate` event, no `interrupted` (the deferred trigger);
    - a transaction that parks and unparks a run before commit writes no needs-you event;
    - park → unpark → park inside one explicit transaction writes exactly one `gate` (F3);
    - raw `DELETE FROM item` keeps only failure events, refs `NULL`, copies intact; raw `DELETE FROM run` of a free chat the same;
    - `workspace_id` attribution: from `run.workspace_id`; a scope-row item → the owner workspace; a project in two workspaces with a chat run → `NULL`;
    - the kind CHECK refuses an unknown kind;
    - the pinned query uses the partial index (`EXPLAIN` names it) — informative, not a pin on plan shape.
  - Pins: `connect.rs` `:140`, `:155`, `:241` (`Pending(33)` → 34) and their "thirty-three … 0033_memory_capture.sql" sentences `:141`, `:156-157`, `:242`; `migrations.rs` doc `:25-34`, `TABLES` `:35` (add `activity_event` in the list's order), applied list `:125`, count `:159` (54 → 55) and its messages `:160-180`, `:4256`, `:4357`, `:4366`, `:4389` (`Pending`/`MigrationsPending(33)` → 34) and their sentences `:4257`, `:4358`, `:4367`, `:4390`, `:4541` (`_sqlx_migrations` count 33) and its "thirty-three" sentence `:4542`. `MIRRORED_TABLES` (`cache/mod.rs:47`) and `it__connection__confirm.snap` do **not** move (not mirrored); re-run both tests once to find any pin this list misses.
- Then the migration (header as `0032`: forward-only, not mirrored, `schema_version` moves to 34 so each box rebuilds its mirror once, a headless worker never migrates):
  ```sql
  CREATE TABLE activity_event (
      id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
      user_id       UUID        NOT NULL REFERENCES app_user(id) ON DELETE CASCADE,
      at            TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
      kind          TEXT        NOT NULL,
      workspace_id  UUID        REFERENCES workspace(id)       ON DELETE SET NULL,
      project_id    UUID        REFERENCES project(id)         ON DELETE SET NULL,
      project_slug  TEXT        NOT NULL,
      item_id       UUID        REFERENCES item(id)            ON DELETE SET NULL,
      item_key      TEXT,
      run_id        UUID        REFERENCES run(id)             ON DELETE SET NULL,
      chat          BOOLEAN     NOT NULL DEFAULT false,
      step_id       UUID        REFERENCES run_step(id)        ON DELETE SET NULL,
      step_name     TEXT,
      permission_id UUID        REFERENCES step_permission(id) ON DELETE SET NULL,
      text          TEXT        NOT NULL DEFAULT '',
      -- F3: the once-per-transaction guard; neither is exposed by `ActivityEvent`.
      txid          XID8        NOT NULL DEFAULT pg_current_xact_id(),
      -- F3: a copy, not an FK: the step for gate/step_failed, the run for the five run kinds,
      -- the item for unblock, the permission for permission. Never NULL.
      subject       UUID        NOT NULL,
      CONSTRAINT chk_activity_event_kind CHECK (kind IN ('gate','judge_failed','selection','unblock',
          'interrupted','permission','step_failed','run_failed','run_started','run_finished','close_out'))
  );
  CREATE INDEX idx_activity_event_user ON activity_event (user_id, id);
  CREATE INDEX idx_activity_event_at ON activity_event (at);
  CREATE INDEX idx_activity_event_needs_you ON activity_event (user_id, id)
      WHERE kind IN ('gate','judge_failed','selection','unblock','interrupted','permission');
  CREATE INDEX idx_activity_event_item ON activity_event (item_id);
  CREATE INDEX idx_activity_event_run ON activity_event (run_id);
  -- F4: the FK columns `ON DELETE SET NULL` searches once per deleted step/permission
  -- (`delete_project` cascades over every `run_step` and `step_permission`, `:11734-11788`).
  CREATE INDEX idx_activity_event_step ON activity_event (step_id) WHERE step_id IS NOT NULL;
  CREATE INDEX idx_activity_event_permission ON activity_event (permission_id) WHERE permission_id IS NOT NULL;
  -- F3: at most one event per transaction, kind and subject.
  CREATE UNIQUE INDEX uq_activity_event_once ON activity_event (txid, kind, subject);
  ```
  `project_id` and `workspace_id` fire once per delete and need no index (F4). Keying the unique index on the nullable FK columns under `NULLS NOT DISTINCT` would collide once `ON DELETE SET NULL` clears them; `subject` never does (F3). `xid8` is 64-bit and no `::xid` cast remains, so no wraparound question.
  Plus: one plpgsql function per source table writing D3's rows (re-reading the row's current state, D2), the run-park rule as D3 states it, D5's `workspace_id`, `left(text, 500)`; the constraint triggers (F2): `CREATE CONSTRAINT TRIGGER trg_<table>_activity_update AFTER UPDATE OF status ON <table> DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (OLD.status IS DISTINCT FROM NEW.status AND NEW.status IN (…)) EXECUTE FUNCTION …` on `run_step` (`'awaiting_approval','failed'`), `run` (`'awaiting_approval','failed','running','done'`) and `item` (`'blocked'`), and on `step_permission` only `trg_step_permission_activity_insert AFTER INSERT … DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.status = 'pending') EXECUTE FUNCTION …`, no update trigger (Postgres refuses `OLD` in an INSERT trigger's `WHEN`, and no product insert on the other three tables should write); each function returns early when the row is gone or its current status differs from `NEW.status`, and inserts with `INSERT … ON CONFLICT DO NOTHING` against `uq_activity_event_once` (F3, D2); the item key is `key_prefix || '-' || key_number` (no `key` column); **no `EXCEPTION` block** (F3: it opens a subtransaction per call, and a transaction touching more than 64 rows then overflows the subxid cache; nothing in the trigger can raise on valid data: `started_by`/`created_by` are `NOT NULL` FKs and every `project_id` is an FK; a raise is a bug the conformance cases and the full store suite catch); `BEFORE DELETE` triggers on `item` and `run` deleting the non-failure events (D7). The pinned query is D4's, newest per subject by `DISTINCT ON (subject) … ORDER BY subject, id DESC` (F3).
  Regenerate `.sqlx` against a migrated scratch database (`cargo sqlx prepare` in `crates/htui-store`; in the hr sandbox Postgres is `psql -h localhost -p 5439 -U postgres`, no docker; the compose `htui` database is empty, migrate a scratch one first).
- **Mirror**: `0032_worker_log.sql`; `pg/worker_log.rs`; `Backend::worker_logs`; M2 T1 commit `8d6e046f` for the pin set.
- **Validate**: `cargo test -p htui-store --all-features --test it activity_pg:: pg_conformance:: connect:: migrations::` (read the names that run: the filter is a substring match); then the whole store suite against the test database, `cargo nextest run -p htui-store --all-features -E 'binary(it)'` (F12: existing raw-SQL tests that `UPDATE run_step SET status = …` now write events, and a failure there shows a trigger bug), including `every_cascade_table_loses_exactly_what_the_report_names` (its hand list `pg_criteria.rs:3345` stays unchanged, D7); `cargo clippy -p htui-store --all-targets --all-features -- -D warnings`; `cargo clippy -p htui-store -- -D warnings`; `cargo fmt -p htui-store`; `cargo sqlx prepare --check` in `crates/htui-store` against the scratch database. Prefix with `env -u HTUI_TEST_QDRANT_URL` in the sandbox (no live Qdrant tests here).

### Task 3: Store requests and the sweeps (lane A, after T2; same worktree)

Owns: `crates/htui/src/store_worker.rs`, `crates/htui/src/worker_log/ship.rs` (and the fake `ShipSource` in its tests), `docs/htui-worker.md`.

Signatures (in `store_worker.rs`, beside `WorkerLogs` `:478`, reply `:1902`, serve `:2660`):
```rust
StoreRequest::Activity { after: Option<i64> },              // name "activity"
StoreRequest::SweepRetention,                              // name "sweep_retention"
StoreReply::Activity { page: ActivityPage, workspaces: Vec<WorkspaceSummary> },
StoreReply::RetentionSwept(u64),
pub const ACTIVITY_LIMIT: u32 = htui_core::model::activity::ACTIVITY_PAGE; // F7: 500
```
`Activity` serves `backend.activity(after, ACTIVITY_LIMIT)` and the same workspaces read `Workspaces` serves (`:2498`). `SweepRetention` (F6): `let Some(writer) = backend.writer() else { RetentionSwept(0) }` (`backend.rs:161`, Offline → `None`); then `cut = now − retention_days(&backend.app_settings().await?)` (`backend.rs:406`; `retention_days` `ship.rs:64-69` is `pub`); then `writer.prune_worker_logs(cut, PER_BOX_CAP) + writer.prune_activity(cut, PER_USER_CAP)` (`PER_BOX_CAP` `ship.rs:27`; `PER_USER_CAP` T1's, `htui_core::store`). In `ship.rs`, `ShipSource: WorkerLogStore + ActivityStore` (`:39`) and `sweep` (`:111-129`) also calls `prune_activity(older_than, PER_USER_CAP)`; a failure of either makes the next sweep due after `SWEEP_RETRY` as today.
Request and reply shape checked (F11): no match on `StoreRequest`/`StoreReply` outside `store_worker.rs` is exhaustive (`WorkerLogs` appears elsewhere only in the Logs tab's own arms), so the new variants break no other lane; `ActivityPage` derives `Debug, Clone` and `WorkspaceSummary` is `Clone`, as `StoreReply` (`Debug, Clone`) needs. Offline: `Backend::activity` gives an empty page, `backend.workspaces()` reads the mirror, `SweepRetention` gives 0. Memory serves through MemStore.

- **Action**: tests first.
  - Store worker over a `MemStore` backend: `Activity { after: None }` answers pushed events and the pinned set with the workspaces; `after` exclusive; offline answers an empty page; a refusal is `Failed { request: "activity" }`; `SweepRetention` prunes an old event and keeps a new one, with `log_retention_days` 3 and with it absent (14); an Offline backend answers `SweepRetention` with `RetentionSwept(0)` (F6); the `name` test (`name_arms_are_stable`, `:6447`) gains both.
  - Shipper: the sweep calls `prune_activity` with the same cut it gives `prune_worker_logs` (the fake records both); a failing `prune_activity` schedules `SWEEP_RETRY`.
- Then implement; add one paragraph to `docs/htui-worker.md` (retention section, `:525`): the hourly sweep also removes activity events older than the retention.
- **Mirror**: the `WorkerLogs` request; the shipper's sweep.
- **Validate**: `cargo test -p htui --all-features --lib -- store_worker worker_log`; `cargo clippy -p htui --all-targets --all-features -- -D warnings`; `cargo clippy -p htui -- -D warnings`; `cargo fmt -p htui`.

### Task 4: The panel and its keys (lane B, after T1; worktree)

Owns: `crates/htui/src/ui/shell/activity.rs` (new), `crates/htui/src/ui/shell/mod.rs` (`pub mod activity;`), `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs`, `docs/htui-keys.md`, unit snapshots `crates/htui/src/ui/shell/snapshots/htui__ui__shell__activity__*.snap` (new).

Signatures:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivityFilter { #[default] All, NeedsYou, Failures, ThisWorkspace }
impl ActivityFilter { pub const fn next(self) -> Self; pub const fn label(self) -> &'static str; }

pub struct ActivityPanel { /* events (≤ CAPACITY, by id), pinned, filter, cursor, in_flight, workspaces */ }
pub const CAPACITY: usize = htui_core::model::activity::ACTIVITY_PAGE as usize; // F7: 500
pub const OVERLAP: i64 = 200;

/// What Enter asks the shell to do (D9).
pub enum ActivityJump {
    Replay { step: StepId },
    Reveal { workspace: Option<WorkspaceSummary>, target: RevealTarget },
    Gone { key: String },
}

impl ActivityPanel {
    pub fn new() -> Self;
    /// `None` until the first reply; then `max_id − OVERLAP`.
    pub fn after(&self) -> Option<i64>;
    pub fn begin_request(&mut self) -> bool;   // false while one is in flight
    pub fn merge(&mut self, page: ActivityPage, workspaces: Vec<WorkspaceSummary>);
    pub fn failed(&mut self);                  // clears in_flight, keeps rows
    /// The rows under the filter: pinned first, then the rest, newest first.
    pub fn rows(&self, scope: &Scope) -> Vec<&ActivityEvent>;
    pub fn pinned_count(&self, scope: &Scope) -> usize;
    pub fn on_act(&mut self, act: Act, scope: &Scope) -> Option<ActivityJump>;
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>, focused: bool);
    pub fn key_hint(&self, keys: &Keys) -> Line<'static>;
}
```
Keys: D10 (`Context::Activity`, `Act::ActivityOpen`, `Act::ActivityFilter`, `ACTIVITY_BROWSE` with `Act::ListBottom` in its `List` layer, F8), with every bookkeeping site the invariant tests need: `Context::ALL`, name, title, `is_view`, the context index table, the act list and index, `context_tables_and_headings`, the view-context list (`catalogue.rs:236`, `:297`, `:360`, `:416`, `:2754`, `:3022`, `:3278`, `:3691`, `:3750`); in `views.rs` `VIEWS` gains `source!("ui/shell/activity.rs")` (`:1350`), `VIEW_STACKS` gains `ACTIVITY_BROWSE` (`:1407`/`:1501`). Regenerate `docs/htui-keys.md` with `cargo run -q -p htui -- --default-keys --print-keys`.
From other tasks the panel uses only T1's `ActivityPage`, `ActivityEvent`, `ACTIVITY_PAGE` and the existing `WorkspaceSummary`, `RevealTarget`, `Scope`, plus its own `Act`s; no `StoreReply` (F7).

- **Action**: tests first (unit, in `activity.rs`).
  - `merge`: ring cap 500 drops the oldest; a repeated id is not doubled; a late lower id is inserted in order; `pinned` replaced wholesale; `after()` is `None` then `max_id − 200`; `failed` keeps rows.
  - `rows`: pinned first newest-first, a pinned event not repeated below; each filter (all, needs you = pinned only, failures = `step_failed`/`run_failed`, this workspace = D5's rule including a scope-row project via `workspace_id`).
  - `on_act`: `ActivityFilter` cycles four and wraps; `ActivityOpen` on an in-scope item → `Reveal { workspace: None, Step{…} }`; out of scope → `Reveal { workspace: Some(…) }` resolved by `workspace_id`, then by first-by-name containing the project; a chat row → `Replay`; refs gone → `Gone`; on an empty list → `None`; cursor moves with `ListDown`/`ListUp`/`ListTop`/`ListBottom` and stays on its event when new rows arrive above.
  - Snapshots (`insta`, a `TestBackend`): empty; empty under a filter (names `f`); pinned plus history with the title count; another workspace's `@slug`; a removed item's row; narrow width with `…`; focused vs unfocused border.
  - Key invariants: `every_act_is_listed_and_has_a_row`, `every_act_has_exactly_one_row`, `names_are_unique_per_context`, `context_tables_and_headings`, `every_view_stack_is_named_by_a_view`, `every_act_is_offered_by_a_stack_and_handled_by_a_view`, `every_view_layer_comes_first_and_every_modal_stack_ends_modal`; `keys_file` reference page.
- **Mirror**: Logs tab `workers.rs` merge; `WaitingList::enter`; the Logs key rows and stack.
- **Validate**: `cargo test -p htui --all-features --lib -- ui::shell::activity keys::`; `cargo test -p htui --all-features --test it keys_file::`; `cargo insta test -p htui --all-features --lib` limited to the new snapshots, reviewed one by one; clippy (both forms) and fmt as T3.

### Task 5: Shell wiring (serial, after T3 and T4 merge)

Owns: `crates/htui/src/app/state.rs`, `crates/htui/src/app/update.rs`, `crates/htui/tests/it/activity.rs` (new), `crates/htui/tests/it/main.rs`, `crates/htui/tests/it/snapshots/it__activity__*.snap` (new), `README.md`.

Changes:
- `App` gains `activity: ActivityPanel` and `sweep_due: u64` (the tick count of the next sweep, F10).
- `paint_panes` (`state.rs:792-827`) draws `self.activity.render(...)` in place of the placeholder; delete `ACTIVITY_WHAT`/`ACTIVITY_NEXT*` (`:1769-1772`); rewrite `the_activity_pane_is_a_placeholder` (`:2478`) and `the_activity_placeholder_is_still_the_shells` (`:2674`) into "the shell draws the panel"; adjust the hint test (`:2556-2585`) and grow/hide test (`:2896-2906`).
- `dispatch_key` (`:1410-1502`), F8: add `fn activity_has_keys(&self) -> bool` (D10's predicate: no overlay, the active tab's `focus_of(tab) == PaneId::Activity`, the layout shows the pane, and the tab's key stack's global layer is not modal). A shell step between the overlay block (3) and `tab.on_key` (4) runs `for act in self.keys.actions(ACTIVITY_BROWSE, chord)`: the panel's acts (`ActivityOpen`, `ActivityFilter`, `ListDown`/`ListUp`/`ListTop`/`ListBottom`) → `self.activity.on_act(act, &self.scope)` and then `ActivityJump` → `Replay` / `SetScope` then `Reveal` / status sentence (D9); otherwise `self.apply_keys(ACTIVITY_BROWSE, chord)`; then it returns either way. No literal chord. `active_stack` (`:979-984`) returns `ACTIVITY_BROWSE` under `activity_has_keys` (so `translate`, the `?` box, the shell chips and `keys_line` follow without further edits), and the pane's bottom-border hint (`:820-823`) uses the same predicate.
- `on_tick` (`update.rs:294-311`): `StoreRequest::Activity { after }` with `Origin::App` while the active tab's layout shows the activity pane and `begin_request()` allows; also when a layout change makes it visible. `SweepRetention` on the first refresh tick and every `SWEEP_TICKS = 3600 * 4` ticks after it (`event_loop.rs:23`, 250 ms), counted in `App::sweep_due` (F10; a reconnect is caught by the next hourly sweep; Offline answers `RetentionSwept(0)`, F6).
- `on_app_reply` (`update.rs:621`), F9, not `observe_reply` (unchanged: it runs before the freshness gate and is the top bar's read-only look): `StoreReply::Activity { page, workspaces }` → `activity.merge`; `RetentionSwept` → nothing; `Failed { request: "activity" }` → `activity.failed()` (clears in-flight). `absorbed` (`:485-497`): its `Origin::App` arm gains the names `"activity"` and `"sweep_retention"` and stays free of side effects.

- **Action**: tests first, `testkit::Harness` (`crates/htui/src/testkit.rs:53`) `over(MemStore)` with `with_ui_store` setting Backlog to `ide_activity` (T6 has not landed):
  - an event pushed into the store appears within one refresh (`drive` four ticks), newest first;
  - a parked step (a real `park_step` on the MemStore) is pinned and counted in the title; answering it unpins it within one refresh;
  - Tab to the activity pane; `enter` on an in-workspace row opens the Backlog's Runs sub-tab on the step; `backspace` returns to the activity pane;
  - a row of another workspace: Enter switches scope, reveals, and the status line names the workspace;
  - `f` cycles the filter and the title says which;
  - a chat row opens the replay;
  - a failed poll sets no status line and keeps the rows; no request while one is in flight;
  - with the pane hidden (`ide` preset) no `Activity` request is sent;
  - the sweep request is sent at the first refresh, not again before `SWEEP_TICKS` (F10);
  - with a Backlog form open and Activity focused, a typed key reaches the form (F8);
  - events are seeded with `MemStore::push_activity` called from `tests/it/activity.rs`, never from `src/testkit.rs` (F12: `testkit` does not turn on `htui-core/test-support`, `htui/Cargo.toml:26`; only the dev-deps, `:103`, do).
  - Snapshots: the Backlog with the panel (pinned + history), the panel focused with its hints.
- Then implement; README: one paragraph under the shell/layout text naming the panel, its filter key and Enter.
- **Mirror**: the `Waiting` poll and reply; `App::reveal`; `WaitingList::enter`.
- **Validate**: `cargo test -p htui --all-features --lib -- app::`; `cargo nextest run -p htui --all-features -E 'binary(it) and test(/^activity::/)'`; `cargo insta test -p htui --all-features` reviewing every `.snap.new` (expect only `activity` and the state tests); clippy (both forms) and fmt.

### Task 6: Backlog defaults to the activity preset (serial, last)

Owns: `crates/htui/src/ui/tabs/backlog/mod.rs` (`BACKLOG_LAYOUT` `:79`) and every snapshot the change moves (`crates/htui/tests/it/snapshots/*.snap`, `crates/htui/src/**/snapshots/*.snap`), plus any test whose expected visible row count the shorter list pane changes.

- **Action**: set `preset: Preset::IdeActivity` in `BACKLOG_LAYOUT` (D11). First count: run the full insta suite once and record how many snapshots move (memory: grepping `.snap` undercounts). Review each `.snap.new` — the only change must be the list/detail pair shortened and the activity pane below (empty state in a fresh store); no other diff is accepted. A test that asserts row counts or cursor-visible rows is fixed by pinning its own preset through `with_ui_store` only when the test is about something else; otherwise its expectation moves.
  - New test: a fresh UI state opens the Backlog with the activity pane; a saved `ide` layout in `ui-state.toml` keeps `ide`.
- **Validate**: `cargo insta test -p htui --all-features` (full; review all); `cargo nextest run -p htui --all-features -E 'binary(it)'`; `cargo test -p htui --all-features --lib`.

### Task 7: The retention setting (lane C; independent, may start at once)

Owns: `crates/htui-core/src/prompt/settings.rs`, `crates/htui/src/prompt_settings.rs`, `crates/htui/tests/it/prompt_settings.rs`, `crates/htui/src/ui/tabs/settings/prompt.rs` and its snapshots, any `SettingKey` count pin found by the tests (e.g. `settings.rs:1517-1524` `app_keys().count() == 13`).

- **Action**: tests first. `SettingKey::LogRetentionDays` (`log_retention_days`), App rung only, a whole number **1–365**, default 14, inserted in key byte order (after `LightModel`, before `MaxSkillTokens`, so later discriminants and `SPECS` move; `ALL: [Self; 15]` → 16). `SettingSpec` already bounds it (`min`/`max`, `settings.rs:379-382`): `min: 1, max: 365`. Settings must store it as a JSON number (`retention_days` reads `Value::as_u64`, `ship.rs:64-69`). Tests: the spec refuses 0, 366 and a string with the validator's sentence; the App rung accepts 30; Settings › Prompt lists the row with its help ("days stored logs and activity events are kept; all boxes") and saves through `set_setting(SettingRung::App, …)`; the count pins move. The worker's and the TUI's `retention_days` (`ship.rs:64-69`) read the same key unchanged. F13: add `log_retention_days_is_the_shippers_key` to `crates/htui/src/prompt_settings.rs`'s tests: `SettingKey::LogRetentionDays`'s name equals `worker_log::ship::RETENTION_KEY`, its spec's `min..=max` is `1..=365`, and its default equals `DEFAULT_RETENTION_DAYS` (both `pub`, `ship.rs:23-25`). It reads `ship.rs` (T3's file) without changing it.
- **Validate**: `cargo test -p htui-core --all-features --lib -- prompt::settings`; `cargo nextest run -p htui --all-features -E 'binary(it) and test(/^prompt_settings::/)'`; `cargo test -p htui --all-features --lib -- ui::tabs::settings::prompt prompt_settings::` (F13's test); snapshots reviewed; clippy (both forms) and fmt for both crates.

## Open questions for the maintainer

**Answered by the maintainer 2026-11-14: the recommended answer on all three** (chat runs write
waits and failures only; back stays in the opened workspace; retention edited in Settings › Prompt,
T7 kept). The plan text below already follows them.

1. **Chat runs in the feed (D12).** Recommended: only permission requests and failures, never
   started/finished. Alternative: every chat run's start and finish too.
2. **Back after a cross-workspace jump (D9).** Recommended: back returns to the place but not the
   workspace (as the `w` switcher behaves), until MOD-116's workspace lifecycle. Alternative: add a
   workspace to history entries now.
3. **Where the retention number is edited (D6, T7).** Recommended: Settings › Prompt's App rows
   (the typed registry that already holds `memory_capture` and `light_model`) until MOD-123.
   Alternative: drop T7 and leave the key file-only (`app_setting`), as M2 did.

## Validation

```bash
bash scripts/gate.sh                                    # the repo gate, by the main thread once per lane merge
cargo clippy --workspace -- -D warnings                 # featureless
cargo insta test -p htui --all-features                 # full snapshot run, review every .snap.new
```
Manual check (a PTY in the sandbox against its Postgres, or the host):
1. Two workspaces; start an item's run in each (one from the TUI, one through a worker if available).
2. The Backlog shows the panel: `run started` rows appear within a couple of seconds.
3. Let a gate park: it is pinned, the title counts it; answer it: the pin goes, the row stays.
4. `f` through the four filters; Enter on the other workspace's row: the scope switches and the status line says so; `⌫` returns to the activity pane.
5. Fail a step: a `✗ failed` row; delete the project: its failure row stays with its key, the others go.
Owed on the host: live-Qdrant tests, the real-keyring smoke, the Postgres gate on the merged tree, the Windows build, the migration renumber at collect if a sibling run adds one.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Duplicate or missed events (TUI runtime and workers) | Medium | Medium | One writer: store triggers and the MemStore write hook (D2); conformance case per kind on both stores; same-status writes nothing |
| The run-park rule disagrees with `views::waiting` | Medium | Low | D3 rule; `views.rs` parity test with a named divergence list, `run_park_kinds` on both stores (T1, F1); the jump still lands on the run |
| Deferred triggers slow the hot status writes | Low | Medium | One indexed insert per status change, `WHEN`-style guard; T2 notes timing of `claim_run`'s transaction before/after in the close-out |
| MemStore snapshot per write slows large in-memory tests | Low | Low | Status maps only (`StatusSnapshot`, F5); measure the `htui-core` suite time before/after T1 and say so; a scratch snapshot only above 5 % |
| Stale pins | Low (derived) | Medium | D4: computed at read time from the subject's state |
| Snapshot churn from the Backlog default | High | Low | T6 alone, last, counted first, every diff reviewed |
| New migration collides with a sibling run's | High | Low | Renumber at collect; move the pins again (memory) |
| `.sqlx` drift between lanes | Medium | Low | T2 owns every `.sqlx` entry; T3 adds no `query!` |
| Key conflict on `enter`/`f` | Low | Low | `ACTIVITY_BROWSE` holds no `List` fold or Backlog layer (D10); invariant tests |
| Event text carries a secret | Low | High | Text is copied only from strings already stored and displayed (D3); no new formatting; reviewer checks the trigger functions |

## Verified claims

Checked at `e99acd97` (plan fact-check, 2026-11-14). Postgres probes on the sandbox's PostgreSQL 18.7 in a scratch database; every probed behaviour (no `OLD` in an INSERT trigger's `WHEN`, deferred constraint triggers queued per row event with the event's `NEW`, user row triggers firing on cascaded deletes, `pg_current_xact_id()` since 13) is documented the same for 16.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Latest migration is `0033_memory_capture`; new one is `0034` | verified | `ls crates/htui-store/migrations` ends at `0033_memory_capture.sql` |
| 2 | 54 → 55 tables; 33 → 34 pending | verified | all migrations applied to a scratch DB: 54 public tables; `migrations.rs:159` `54`, `connect.rs:140/155/241` `33` |
| 3 | `WorkerLogStore` `worker_log.rs:21-82` (`impl Future` methods), `Writer` impl `worker.rs:529`, `Backend::worker_logs` `backend.rs:597-611`, `app_settings` `:406` | verified | read at HEAD; `impl Future … + Send` trait methods with `async fn` impls are M2's precedent, no compile probe needed |
| 4 | `#[cfg(feature = "test-support")]` `MemStore` method shape | verified | `mem.rs:1545` (`set_fault`) precedent; htui-worker dev-deps enable `htui-core/test-support` (`Cargo.toml:32`) |
| 5 | `pg/write.rs` lines: `transition_step` 8036, `interrupt_step` 8155, `answer_gate` 8191, `park_step` 9290, `promote_step` 9155, `transition_run` 7971, `start_chat_run` 3334, `finish_chat_run` 3388, `fail_run` 9361 | verified | `git grep -n 'fn <name>\b'` |
| 6 | "about ten engine sites" fail a step via `transition_step` | verified | `engine.rs` `move_step`/`transition_step` to `Failed`: ~10 sites (6310, 7065, 7907, `move_step` callers) |
| 7 | Chat paths called from `htui/src/agent_worker.rs` | verified | `:2464` `start_chat_run`, `:2784` `fail_run` |
| 8 | TUI `RunRuntime<Backend,…>` `run_worker.rs:39`; worker `RunRuntime<PgStore,…>` `worker_cmd.rs:147` | verified | read at HEAD |
| 9 | D2 `CREATE CONSTRAINT TRIGGER … AFTER INSERT OR UPDATE OF status … WHEN (OLD.status IS DISTINCT FROM NEW.status)` | amended | PG 18.7 probe: `ERROR: INSERT trigger's WHEN condition cannot reference OLD values`. D2 and T2 now say two triggers per table (UPDATE with `WHEN`, INSERT without); the two-trigger shape created cleanly |
| 10 | Deferred trigger "re-reads the row's current state" writes one event per transition | amended | probe: A→B→C in one txn queues two calls, both re-read `C` (`NEW` was `B` then `C`); park→unpark→park wrote two extra `gate` rows. D2/T2 now guard: write only when `NEW.status` = current status and no event of that kind/subject has `xmin = pg_current_xact_id()::xid`; re-probed: one event per txn, a later txn writes again. Superseded by blueprint F3: inside an `EXCEPTION` block `xmin` is the subtransaction's xid and the guard wrote 2 rows; now `txid xid8` + `subject` + unique index + `ON CONFLICT DO NOTHING`, no `EXCEPTION` |
| 11 | `NEW` in a deferred row trigger is the row as of that update, not the final state | verified | probe: `newstatus=B, cur=C` for the first call |
| 12 | `park_step` order (step then run, or run then step) gives exactly one `gate`, no `interrupted` | verified | `park_step` is step then run (`write.rs:9326`, `:9334`); probe of both orders: one `gate` row each, no run-level row |
| 13 | Same-status rewrite writes nothing | verified | probe `C→C`: 0 calls (`WHEN` filters at queue time) |
| 14 | Recovery re-park `engine.rs:3810` "writes the same status" | amended | it moves the run `running → awaiting_approval` with the step already parked; the run rule writes nothing. Second `park_step` returns `StepMoved` (`write.rs:9318-9320`). Wording fixed |
| 15 | Trigger precedent: `updated_at` triggers `0001_init.sql:578`, box trigger `0032_worker_log.sql` | verified | `:574-580` loop over `run`, `run_step`, `item`; `0032:42` `BEFORE DELETE ON box` |
| 16 | `MemStore::write` `mem.rs:1537` is the one `state.write()`; state `:397-401`, init `:535`, impl `:11907` | verified | `git grep 'state\.write()'` → only `:1538` |
| 17 | `views::waiting` `views.rs:616-785` | amended | function is `:616-839` (run rules `:666-781`); public as `htui_worker::waiting` (`lib.rs:56-60`, `views` is private) |
| 18 | `judge_failed` = failed judge in an unselected slot | amended | `views.rs:696-703` takes a failed judge only with a `gate_note`; rule now says so |
| 19 | `judge_at` `status.rs:491`, `fanout_index = -1`; `park_interrupted` `engine.rs:4349`; `interrupt_step` sets `failed` | verified | read at HEAD (`write.rs:8158`) |
| 20 | Columns `selected`, `gate_note`, `run.failure`, `step_permission.summary`, `phase_name` exist | verified | `0001_init.sql:462,484,485`; scratch DB `\d step_permission` |
| 21 | R-TUI-11 "computed from the store at read time" | verified | `docs/REQUIREMENTS.md:806-807` |
| 22 | Unpin paths incl. stale sweep `pg/relay.rs:190-347` | verified | `open_permission` 190 (stale at 215-218) … `answer_permission` 347 |
| 23 | `workspace_project` PK `0001_init.sql:161-167`; `run.workspace_id` `0019_stop_runs.sql:34`; `owner_workspace_id` `0026`; `Scope::contains` `scope.rs:32` | verified | read at HEAD; `WorkspaceSummary` has `slug`, `name`, `projects` |
| 24 | `log_retention_days` default 14 `ship.rs:23-25`, `retention_days` `:64-69` (pub), `sweep` `:111-129`, `PER_BOX_CAP` `:27`, `ShipSource` `:39` | verified | read at HEAD (`ShipSource` at `:38`) |
| 25 | No product path deletes an item (`write.rs:16`, `no_delete_path`); `delete_project`/`delete_workspace` `:6583`, `:11734-11788` | amended | fns at `:6599`/`:6570` (6583 is the doc comment); deletes `:11734`, `:11743`, `:11763` |
| 26 | `BEFORE DELETE` on `item`/`run` fire on cascades from `project`; `ON DELETE SET NULL` then clears only failures | verified | probe (FK graph copied from the real schema): `DELETE FROM project` fired both triggers at depth 2, kept `step_failed`/`run_failed` with all refs `NULL`, no "already modified" error |
| 27 | `DeleteReach` `traits.rs:4400-4440`; twin test `pg_criteria.rs:3198` | amended | struct `:4414` (doc `:4402`, end `:4483`); test name and line verified; it uses a hand list, no FK scan |
| 28 | `paint_panes` `state.rs:793-827`, `ACTIVITY_*` `:1769-1772`, layout slot `layout.rs:53-54` | amended | `paint_panes` at `:792`; others verified |
| 29 | `TICKS_PER_REFRESH` `update.rs:28` ~1 s; `on_tick` `:294-311`; `observe_reply` `:500-525`; `absorbed` `:485-497` | verified | 4 × 250 ms (`event_loop.rs:23`) |
| 30 | `Action::Replay { step_id }` `action.rs:39`; `RevealTarget::Step {item,key,run,step}` `:167-195`; `WaitingList::enter` `waiting_list.rs:130-141` | verified | read at HEAD |
| 31 | `set_scope` `update.rs:349-366`, refused when an editor is open `:52-55`; `on_scope_change` clears `pending_reveal` `backlog/mod.rs:337-343` | verified | `set_scope` at `:351`; refusal is in the `Action::SetScope` arm |
| 32 | `recorded` `state.rs:705-720`, `on_key` `:1406`; `here` ` · activity` `:679-698`; `Loc` holds no workspace `history.rs:20-29` | verified | read at HEAD |
| 33 | `Context::Logs` sites `catalogue.rs:162-163, 236, 297, 360, 416`; acts `:876-881`; rows `:2330-2334`; tables `:2754, :3022, :3278, :3691, :3750` | amended | rows at `:2332-2334`; rest verified |
| 34 | `f` is bound only to `BacklogOpenFilter` (`catalogue.rs:1999`); `enter` in the List context only as `ListFold` | verified | `git grep '"f"'` → one row; Global/Layout/History bind neither |
| 35 | `LOGS_BROWSE` `views.rs:1218-1227`; `VIEWS` `:1350`; `VIEW_STACKS` `:1407`/`:1501` | amended | stack ends `:1230`; rest verified |
| 36 | `dispatch_key` `state.rs:1410-1495`; `active_stack` `:979-984`; hint `:820-823` | amended | `dispatch_key` ends `:1502` |
| 37 | `BACKLOG_LAYOUT` `backlog/mod.rs:79`; findings §3.4 line 102; `registry.rs:155`; saved layout wins `state.rs:152-155` | verified | read at HEAD; Logs/Requirements/Skills have their own `ListDetail` layouts |
| 38 | Patterns: `WORKER_LOG_CASES` `:1587-1593`, `run_all_worker_logs` `:1630`, `mem_store.rs:31`, `pg_conformance.rs:168-188`; `an_interrupted_park_is_a_row` `:1947` | amended | `:1582-1588`, `:1631`, `mem_store.rs:30`, test `:1936`; pg binding verified. Added the case-count unit test `conformance.rs:34303-34326` (T1-owned file) |
| 39 | `store_worker.rs`: `WorkerLogs` `:478`, name `:1431`, reply `:1902`, serve `:2660`; `Workspaces` serve `:2498`, name `:1384`; name test `:6450` | verified | name test is `name_arms_are_stable` `:6447`; `StoreReply` derives `Debug, Clone` only |
| 40 | Pins `connect.rs:140,155,241`, `migrations.rs:25-35,125,159-180,4256,4357,4366,4389,4541` | amended | added the "thirty-three" sentences (`connect.rs:141,156-157,242`, `migrations.rs:4257,4358,4367,4390`); count at `:4541`, sentence `:4542` |
| 41 | `connection__confirm` does not move: `activity_event` is not mirrored | verified | mirroring is the explicit `MIRRORED_TABLES: [&str; 22]` (`cache/mod.rs:47`); the snapshot prints "22 mirrored tables" from its `len()` |
| 42 | `.sqlx` offline entries per crate under `crates/htui-store/.sqlx` | verified | directory present; `SET CONSTRAINTS` entries live there |
| 43 | `SettingKey` `settings.rs:189`, `ALL: [Self; 15]`, `app_keys().count() == 13` `:1524`, byte order `light_model` < `log_retention_days` < `max_skill_tokens` | verified | read at HEAD; only `settings.rs`, `settings/prompt.rs`, `tests/it/prompt_settings.rs` name variants |
| 44 | "If `SettingSpec` cannot bound a maximum" | amended | `SettingSpec` has `min`/`max` (`settings.rs:379-382`); T7 text now says `min: 1, max: 365` and that the value is stored as a JSON number (`retention_days` reads `as_u64`) |
| 45 | `str_enum!` precedent `queue_settings.rs:9`; `Harness` `testkit.rs:53`, `over`, `with_ui_store` | verified | macro at `model/mod.rs:25`; `over` `:121`, `with_ui_store` `:210` |
| 46 | Key doc regeneration `cargo run -q -p htui -- --default-keys --print-keys` | verified | `docs/htui-keys.md:24-25`; `cli.rs:566` accepts the pair |
| 47 | M2 T1 commit `8d6e046f` | verified | `git log -1 8d6e046f` |
| 48 | MemStore write hook takes `user` | amended | `this_user()` takes the read lock (`mem.rs:11912-11913`); the hook now takes each row's own user (D5) |
| 49 | Lanes: T1 ∥ T7; A (T2→T3) ∥ B (T4) ∥ C (T7) | verified | file sets intersect nowhere: `tests/it/main.rs` is per crate (htui-store T2, htui T5; after F1 T1 touches no `tests/it/main.rs`, only `htui-worker/src/views.rs`'s tests); `.sqlx` T2 only; `docs/htui-keys.md`/`catalogue.rs`/`views.rs` T4 only; `store_worker.rs` T3 only (T4's panel takes pages, not requests); unit snapshots (T4) vs `it__prompt_settings__*` (T7) disjoint; no `Cargo.toml` change needed |
| 50 | T5, T6 serial; T6 after every UI change | verified | T6's moved snapshots are Backlog ones; T7 merges before T6 under "after every other UI change" |

## Acceptance
- [ ] All tasks complete, tests first; T1 verified; lanes merged and the gate green on the merged tree
- [ ] Every D3 kind written exactly once per transition on both stores (conformance), the run-park rule matches `views::waiting` (parity)
- [ ] Pinned needs-you events counted in the title and unpinned within one poll of the answer
- [ ] Enter jumps to the item's step or the chat, across workspaces with the status sentence, and back returns to the panel
- [ ] `f` filters all workspaces · needs you · failures · this workspace; every key in `keys.toml`, `docs/htui-keys.md` regenerated
- [ ] Retention prunes events from the TUI and workers; deleting a project keeps only its failure events
- [ ] Featureless clippy clean; full insta run reviewed; Backlog snapshot churn limited to the panel
- [ ] Owed on the host and said so: live-Qdrant tests, the Postgres gate on the merged tree, the Windows build, the migration renumber at collect
