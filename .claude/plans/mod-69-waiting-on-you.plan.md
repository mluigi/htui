# Plan: MOD-69 — Waiting-on-you list across items

**Source PRD**: `.claude/prds/mod-69-waiting-on-you.prd.md`
**Selected Milestone**: M1 "See what waits" and M2 "Jump to it", planned together. M2 is one
reveal path plus close-out, and splitting would cost a second plan, confirm and review cycle.
**Complexity**: Large (store read across three backends, plus TUI wiring; 109 snapshot files carry
the top bar)
**Routing**: PRD path, ultracode for implement (maintainer, 2026-10-03). Sandbox `hr/MOD-69`.

## Summary

One tick-driven store request, `Waiting{scope}`, replaces today's `ActiveRuns` on the shell's
refresh tick and on a scope change. It is served by one candidate read over the active workspace:
items that are `blocked` or `awaiting_approval`, or that own a parked run, each with its active
runs and their steps. Online only, it adds
one read of open permission requests. Then a pure classifier in `htui-worker::views` runs over the
result, reusing the same engine guards the Runs pane uses (`verdicts`, `unblock_enabled`,
`select_enabled`). The reply, a `WaitingView`, lives in `TopBarState`:

- the top bar renders "N working · M waiting";
- a new global overlay (`Ctrl+W`) renders the rows straight from `ctx.top_bar`, so it holds no
  state but its cursor;
- `Enter` emits a new `RevealTarget::Step`, which the Backlog routes to the item's Runs sub-tab with
  the step selected.

## Decisions

- **D1 — The derivation is Rust over one candidate read, not SQL.** Unblock's Resume case needs
  `status::cursor` over the run's snapshot (`crates/htui-orch/src/status.rs:272,315`), which SQL
  cannot evaluate. The store returns candidates; `htui-worker::views` classifies them. "One store
  query" (HANDOFF) means one request with no per-item fan-out: one `ReadStore` method plus, online,
  one `WriteStore` method, never N reads per item.
- **D2 — Candidate set.** An item in scope that is `blocked` or `awaiting_approval`, **or** owns an
  active run at `RunStatus::AwaitingApproval`, with its active runs and each such run's steps. Run
  status is the anchor because parks move the item only when it is `in_progress`:
  - `park_step` (traits.rs:1502-1505) and `park_selection` (engine.rs:4470 ignores the
    `transition` bool) leave an item that a person moved to `open` under a parked run where it was;
  - every `UnblockCase` needs `blocked` or `awaiting_approval` (`command.rs:1182-1191`);
  - `escalate` parks no step: run → `AwaitingApproval`, item `in_progress → blocked`
    (`gate.rs:1050-1051`).

  Chat runs (no item) are never candidates. Permissions come separately (D4).
- **D3 — Classification rules (pure, over candidate rows).** One row per reason instance:
  - **Gate**: a step at `StepStatus::AwaitingApproval` in an active run. Reason text: `gate_note`,
    or "gate" when there is none.
  - **Judge failure**: the run is at `AwaitingApproval` with no step parked, and some fan-out slot
    has a candidate whose `select` verdict is `Ok`. The slot's judge step, found with
    `status::judge_at(steps, position, attempt)` (status.rs:231; `group_at` excludes the judge), is
    `failed` with a `gate_note`, which is how `fail_judge` leaves it (`engine.rs:5163-5174`). The
    recovery sweep's `gate_note = "interrupted"` judge (htui-orch conformance.rs:5013-5045) also
    shows here, which is deliberate: to a person, both are judge failures. Reason text: that
    `gate_note`. Step: the judge step.
  - **Selection**: the same as judge failure, but no failed judge exists in the slot. Reason text:
    "awaits selection". Step: the slot's first candidate.
  - **Unblock**: `unblock_enabled(item, &[(Run, bool)])` returns `Ok(case)`, called directly with
    `resumable(&cursor(..), steps)` per active run. `ItemActions.unblock` cannot be used here: it
    drops the case (`.map(drop)`, views.rs:441). A run whose snapshot does not decode is handled as
    `verdicts` handles it: no unblock row, and its gate rows still show. Reopen has no run. FollowRun
    and Resume carry the run. Reason text names the case.
  - **Permission**: each open permission (D4). Reason text: `summary` (the tool), or "permission"
    when there is none. Step: `run_step_id`.

  Selection verdicts come from the same `verdicts` function the Runs pane uses
  (`crates/htui-worker/src/views.rs:327`), called with empty heads and no live chats, so the list
  and the pane cannot disagree on whether a run awaits a selection. A gate is told by step status,
  not by the `approve` verdict, because a parked gate without its output still waits on a person:
  it greys `approve` but not `reject`.
- **D4 — Open permissions.** A new `WriteStore::open_permissions(scope)`. It applies the
  relay's own "open" predicate scoped by project: `pending`, `r.lease_owner = p.owner`, and
  `r.lease_expires_at > clock_timestamp()` (`crates/htui-store/src/pg/relay.rs:501-524`, Mem
  `State::relay_view` + `live_owner`, mem.rs:6133-6140, 6329), plus `r.item_id IS NOT NULL`. A chat
  run's permission is answered in its Chat tab and has no item to reveal. Each row carries the item
  id and key. It sits on `WriteStore` because the relay tables are not mirrored
  (`docs/decisions/mod/mod-42.md` OQ-4). Offline (`Backend::writer()` is `None`), the view sets
  `permissions_known = false` and the overlay says "permissions unavailable offline" (PRD OQ,
  approved 2026-10-03).
- **D5 — Counts.**
  - `waiting` is the number of rows, the same length as the overlay list.
  - `working` is the active runs in scope (the existing `active_runs`) minus the distinct active
    runs that own at least one row.
  - A run is never counted in both.
  - "working + waiting = active" does **not** hold: Reopen rows have no run, and one run can own
    several rows. The PRD's metric is amended accordingly (verified-claims row 14).
- **D6 — The view lives in `TopBarState`.** `StoreReply::Waiting(WaitingView)` updates
  `top_bar.waiting`. The overlay reads `ctx.top_bar.waiting` (`Ctx` already carries
  `top_bar`, `crates/htui/src/app/state.rs:79`). So the list refreshes every tick (4 × 250 ms,
  `event_loop.rs:19`, `update.rs:19`) with no request of its own: `observe_reply` runs on every
  reply whatever the origin (`update.rs:278`), and a reply sets `dirty` (`update.rs:44`). It holds
  no state beyond the cursor (PRD "no own state").
  - `StoreRequest::ActiveRuns` has no other users: only the tick (update.rs:132) and `set_scope`
    (update.rs:188) send it. Both switch to `Waiting`. The request and reply variants are
    **removed**, and their pinning tests (update.rs:668, :739; store_worker.rs:3162) become
    `Waiting` tests. `Backend::active_runs` stays, now called by the `Waiting` serve arm.
  - `TopBarState` derives `Debug, Clone, Default, PartialEq, Eq` (state.rs:33), so `WaitingView` and
    `WaitingRow` derive all five and hold only ids, strings and counts, never `Run`, `RunStep`,
    `Item` or `StepPermission` (those derive no `Eq`).
- **D7 — Key.** `Ctrl+W`, a `KeyScope::Global` binding to
  `Action::Overlay(OverlayAction::Open(WaitingList::ID))`, help text "waiting". It is a chord so
  no text field takes it, as with `Ctrl+F` (`crates/htui/src/app/mod.rs:86-96`). Every text widget
  passes chords (`text_field.rs:146-154`, `text_area.rs:191`, `chat/mod.rs:449-455`).
  - **One shadow to fix:** Settings › Boxes matches `KeyCode::Char('w')` without a modifier check
    (`settings/boxes.rs:711`), so today Ctrl+W opens a box's executor. T3 adds the modifier guard.
  - MOD-67 registers the binding as a named action when it lands.
- **D8 — Reveal to step.** A new `RevealTarget::Step { item, key, run: Option<RunId>, step:
  Option<StepId> }` whose `kind()` and `key()` cover it (action.rs, the only exhaustive matches).
  `kind()` is `RevealKind::Item`, so `reveal_tabs` routes it to the Backlog; Backlog and Requirements
  use let-else and need no edits.
  - **Typed path.** The Backlog has no typed path to the pane: sub-tabs are `Box<dyn DetailTab>`
    with no downcast (detail/mod.rs:67, 122-127). Add `DetailTab::focus(&mut self, run, step)`
    with a no-op default, and `DetailRegistry::focus(id, run, step)` to pass it through.
    `RunsTab` (`RunsTab::ID`, runs.rs:322) implements it as a pending focus, applied in `on_runs`
    after the cursor is set (runs.rs:446-472, arm :1453).
  - **Ordering.** `RunsTab::on_item_change` clears pane state (runs.rs:1270), so the focus is
    armed **after** `select_item`. An unloaded item carries the focus inside `pending_reveal`, and
    the `Items` reply arms it after its `select_item`.
  - **Already-selected item.** `go` returns early for the item already selected
    (backlog/mod.rs:191), so no `Runs` re-read fires. The pane applies the focus to its loaded runs
    at once when they are present; otherwise it re-requests `Runs`.
  - The Backlog then `select_id(RunsTab::ID)`. `on_item_change` does not reset the active sub-tab
    (detail/mod.rs:220-224), so the selection survives.
  - The cursor moves to the step's entry, or to the run's first entry when `step` is `None`.
    `select_step` searches only the cursor's run, so the focus search covers every run's entries.
    A target the reply does not hold leaves the cursor where `on_runs` put it and says nothing.
- **D9 — Ordering.** Rows sort by project position, then item key, then run creation, then step
  position, then reason in D3 order. This is deterministic, so snapshots are stable.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / scoped read | `crates/htui-store/src/pg/read.rs:1269` (`active_runs`) | `scope.is_empty()` short-circuit, `project_uuids(scope)`, `= ANY($1)` |
| Relay read | `crates/htui-store/src/pg/relay.rs:500-525` (`relay_view`) | `query_as!(PermissionRecord, …)` with the open-lease predicate |
| Cache read | `crates/htui-store/src/cache/read.rs:1380` | dynamic SQL via `placeholders(n)` + `AssertSqlSafe`, bind per project |
| Mem read | `crates/htui-core/src/store/mem.rs:410` | `self.read(\|state\| …)` filter on `scope.contains(project_id)` |
| New trait read end to end | MOD-72 `tool_call_counts` (`traits.rs:281`, mem.rs:6565, pg/read.rs:1125, cache/read.rs:1249, backend.rs:836, writer.rs:312, agent `UsageSpy` :741, `SpyStore` :429) | one method threaded through every implementor |
| Errors | `map_sqlx`, `StoreError::{NotFound, …}` | store errors surface as `StoreReply::Failed` through the worker |
| Engine guards over rows | `crates/htui-worker/src/views.rs:327` (`verdicts`) | pure function over `(Item, [(Run, Vec<RunStep>)])` |
| Store request | `crates/htui/src/store_worker.rs:133,1014,1152,1709` (`ActiveRuns`) | variant + `name()` + reply + `serve` arm |
| Global overlay | `crates/htui/src/app/mod.rs:86-96` + `ui/overlay/concepts_search.rs` | factory register + global chord binding; `Overlay` trait |
| Reveal | `crates/htui/src/ui/tabs/backlog/mod.rs:786-812`, `app/update.rs:233-272` | kind-routed reveal; `pending_reveal` for unloaded rows |
| Tests | conformance `tool_call_counts_read_back` (`crates/htui-core/src/store/conformance.rs:14218`), `CASES`/`READ_CASES` (:53, :515); insta snapshots under `crates/htui/src/ui/**/snapshots` | one case per reason plus negatives; Mem + Pg harness |
| Logging | `tracing::debug!` in `app/update.rs:241,270` | debug for silent no-ops |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/waiting.rs` | CREATE | `WaitingCandidate { item, runs: Vec<(Run, Vec<RunStep>)> }`, `OpenPermission { item, item_key, permission: StepPermission }` |
| `crates/htui-core/src/model/mod.rs` | UPDATE | module + re-exports |
| `crates/htui-core/src/store/traits.rs` | UPDATE | `ReadStore::waiting_candidates`, `WriteStore::open_permissions` |
| `crates/htui-core/src/store/mem.rs` | UPDATE | both impls |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | cases per D2/D4 in `CASES` (Mem + Pg; READ_CASES may not write and its fixture has no parked runs) + `run_case` dispatcher arms |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | case-count pins (mem_store.rs:36, :66) |
| `crates/htui-store/src/pg/read.rs` | UPDATE | `waiting_candidates` (items, runs, steps by `ANY`) |
| `crates/htui-store/src/pg/relay.rs`, `pg/write.rs` | UPDATE | `open_permissions` + trait wiring |
| `crates/htui-store/src/cache/read.rs` | UPDATE | mirror `waiting_candidates` |
| `crates/htui-store/src/backend.rs`, `writer.rs` | UPDATE | `ReadStore` arm / forward |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | UPDATE | spy forwards |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | `EXPECTED_CASES` pin (:28) |
| `crates/htui-store/tests/cache.rs` | UPDATE | the cache's own `waiting_candidates` test (mirror `tool_call_counts`, cache.rs:550-622) |
| `crates/htui-store/.sqlx/query-*.json` | CREATE | offline query data for each new `query!` (docs/hr-sandbox.md:194-209 recipe, `--all-targets --all-features`) |
| `crates/htui-worker/src/views.rs` | UPDATE | `WaitingReason`, `WaitingRow`, `WaitingView`, `fn waiting(...)` + unit tests |
| `crates/htui-worker/src/lib.rs` | UPDATE | `pub use views::{WaitingReason, WaitingRow, WaitingView, waiting}` (`mod views` is private, lib.rs:43, 56) |
| `crates/htui/src/store_worker.rs` | UPDATE | `StoreRequest::Waiting{scope}`, `StoreReply::Waiting`, `serve` arm; `ActiveRuns` variants removed |
| `crates/htui/src/app/state.rs` | UPDATE | `TopBarState.waiting: WaitingView` |
| `crates/htui/src/app/update.rs` | UPDATE | tick dispatches `Waiting`; reply sets `top_bar.waiting` |
| `crates/htui/src/ui/top_bar.rs` | UPDATE | "N working · M waiting", waiting styled when > 0 |
| `crates/htui/{tests,src}/snapshots/*.snap` | UPDATE | 109 of 146 carry the top bar's run text; 104 carry the global help line that gains `Ctrl+w waiting` |
| `crates/htui/src/ui/tabs/settings/boxes.rs` | UPDATE | modifier guard on `Char('w')` (D7) |
| `crates/htui/tests/backlog.rs` | UPDATE | comment at :2366-2379 names the `ActiveRuns` tick |
| `crates/htui/src/ui/overlay/waiting_list.rs` | CREATE | the overlay |
| `crates/htui/src/ui/overlay/mod.rs`, `crates/htui/src/app/mod.rs` | UPDATE | module; factory + `Ctrl+W` binding |
| `crates/htui/src/app/action.rs` | UPDATE | `RevealTarget::Step` |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | reveal arm for `Step` |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | `DetailTab::focus` default + `DetailRegistry::focus` |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | `RunsTab` pending focus applied in `on_runs` |
| close-out docs | UPDATE/CREATE | `docs/decisions/mod/mod-69.md`, `DECISIONS.md`, `HANDOFF.md`, PRD rows |

## Tasks

File sets are exact; the step-3.5 intersection check decides independence (verified-claims row 25).
**Wave order: T0 → {T1 ∥ T2 ∥ T5} → T3 → T6.** The first draft's T4 (the overlay) is folded into
T3. Its global binding adds `Ctrl+w waiting` to the help line that 104 existing snapshots carry,
so the top-bar text and the binding share one re-accept pass.

### Task 0: Model types (serial, first)
- **Files**: `crates/htui-core/src/model/waiting.rs` (new), `crates/htui-core/src/model/mod.rs`
- **Action**: the two row types of D2/D4:
  - `WaitingCandidate { item: Item, runs: Vec<(Run, Vec<RunStep>)> }`;
  - `OpenPermission { item: ItemId, item_key: String, permission: StepPermission }`.

  Both derive `Debug, Clone, PartialEq, Serialize, Deserialize`, **not `Eq`**, because the model
  rows they hold derive no `Eq` (run.rs:167, 212; item.rs:117; relay.rs:74). No logic.
- **Validate**: `cargo check -p htui-core`

### Task 1: Store read surface (parallel with T2, T5)
- **Files**: `crates/htui-core/src/store/{traits.rs,mem.rs,conformance.rs}`,
  `crates/htui-core/tests/mem_store.rs`,
  `crates/htui-store/src/{pg/read.rs,pg/relay.rs,pg/write.rs,cache/read.rs,backend.rs,writer.rs}`,
  `crates/htui-store/tests/{pg_conformance.rs,cache.rs}`, `crates/htui-agent/src/conformance.rs`,
  `crates/htui-agent/tests/recorder.rs`, `crates/htui-store/.sqlx/*` (new files only)
- **Action**: TDD.
  1. **Conformance cases first, in `CASES`** (Mem + Pg), with `run_case` arms and the count pins
     bumped (mem_store.rs:36, :66; pg_conformance.rs:28):
     - a candidate per state: gate park, selection park, failed judge, escalation (run parked,
       item blocked), Reopen (blocked, no run), an `open` item under a parked run;
     - negatives: an `in_progress` item with a running run; a finished run; an item out of scope;
       a chat run;
     - open permission vs answered, expired-lease, foreign-owner and chat-run rows.
  2. The cache's `waiting_candidates` gets its own test in `tests/cache.rs` (READ_CASES may not
     write).
  3. Then the trait methods, with explicit forwarding in every implementor and no default bodies
     (a default would let `UsageSpy` skip silently).
  4. `.sqlx` into `crates/htui-store/.sqlx` via the docs/hr-sandbox.md:194-209 recipe (scratch DB
     `htui_sqlx`, `--all-targets --all-features`).
- **Mirror**: `active_runs`, `relay_view`, `tool_call_counts`.
- **Validate**: `cargo test -p htui-core --all-features`;
  `cargo test -p htui-store --all-features -- --test-threads=1` (Pg via `HTUI_TEST_DATABASE_URL`);
  `cargo test -p htui-agent --all-features`

### Task 2: Classifier (parallel with T1, T5)
- **Files**: `crates/htui-worker/src/views.rs`, `crates/htui-worker/src/lib.rs` (re-export,
  required)
- **Action**: TDD. Unit tests first, from row fixtures with no store, covering each D3 reason, the
  D5 counts and the D9 order. Fixtures:
  - a parked gate without output (counts);
  - a judge-failed slot (judge row, not selection) and an interrupted judge (judge row);
  - a selection slot already resolved (no row);
  - an escalation (unblock FollowRun row, no gate row);
  - a person-blocked item over a parked gate (gate row plus unblock row);
  - an `open` item under a parked run;
  - a run whose snapshot does not decode (no selection or unblock row; its gate row still shows);
  - permissions present vs `None` (offline: `permissions_known = false`).

  Then implement `waiting(active: usize, candidates: &[WaitingCandidate], permissions:
  Option<&[OpenPermission]>) -> WaitingView` on `verdicts` (select) plus a direct
  `unblock_enabled(item, &[(Run, bool)])`, using `status::judge_at` for the judge.
  `WaitingView`/`WaitingRow` derive `Debug, Clone, Default, PartialEq, Eq`.
- **Mirror**: `verdicts` tests in the same file.
- **Validate**: `cargo test -p htui-worker --all-features`

### Task 5: Reveal to step (parallel with T1, T2) — M2
- **Files**: `crates/htui/src/app/action.rs`, `crates/htui/src/ui/tabs/backlog/mod.rs`,
  `crates/htui/src/ui/tabs/backlog/detail/mod.rs`, `crates/htui/src/ui/tabs/backlog/detail/runs.rs`
- **Action**: TDD. Implement D8, with tests for:
  - an item already loaded vs unloaded (`pending_reveal` carries the focus);
  - the item already selected (`go` short-circuits);
  - a step present vs absent in the `Runs` reply;
  - a step in a run other than the cursor's;
  - `step: None` focusing the run;
  - the half-typed-field refusal still applies.

  New snapshot files only; existing snapshots must not change.
- **Validate**: `cargo test -p htui --all-features backlog`

### Task 3: TUI wiring, top bar, overlay, one snapshot pass (serial, after T1/T2/T5 merge)
- **Files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/app/{state.rs,update.rs,mod.rs}`,
  `crates/htui/src/ui/top_bar.rs`, `crates/htui/src/ui/overlay/{waiting_list.rs (new),mod.rs}`,
  `crates/htui/src/ui/tabs/settings/boxes.rs`, `crates/htui/tests/backlog.rs` (comment),
  `crates/htui/{tests,src}/snapshots/*.snap`
- **Action**: TDD, in this order:
  1. **Store request.** `StoreRequest::Waiting{scope}` / `StoreReply::Waiting(WaitingView)`. The
     serve arm reads `active_runs` + `waiting_candidates`, then `writer()?.open_permissions` when
     online, then `htui_worker::waiting`. Remove the `ActiveRuns` variants (D6) and port their
     tests.
  2. **Shell.** The tick and `set_scope` send `Waiting`; the reply sets `top_bar.waiting`.
  3. **Top bar.** Renders "N working · M waiting", with the waiting part in `theme.accent` when
     non-zero (`Theme` has no warning style, theme.rs:11-24).
  4. **Overlay.** `WaitingList`, modal:
     - cursor on `j`/`k` plus Up/Down (the overlay convention, workspace_switcher.rs:161-170);
     - `Enter` emits `OverlayAction::Close`, then `Action::Reveal(RevealTarget::Step{..})`, the
       order `ConceptsSearch` uses (concepts_search.rs:215-216); `Esc` closes;
     - rows show the item key, the step, the reason label and the reason text;
     - empty state "nothing is waiting on you"; "permissions unavailable offline" when
       `!permissions_known`;
     - the cursor clamps when rows shrink between ticks.
  5. **Key.** Register the factory and the `Ctrl+W` global binding (help "waiting"), and add the
     Boxes modifier guard (D7).
  6. **Snapshots.** Re-accept once, alone: `INSTA_UPDATE=always cargo test --workspace
     --all-features`, then re-run without the variable (README.md:489-494). Review the diff:
     only the top-bar text and the help line may change in existing snapshots. Add new snapshots:
     overlay empty, mixed reasons, offline.

  Tests: `serve_waiting_*` (mirror store_worker.rs:3159); an update-loop test that a park shows on
  the next refresh tick; a test that the overlay renders fresh rows after a reply without
  re-opening.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`

### Task 6: Close-out (after the review gate)
- Write-up `docs/decisions/mod/mod-69.md`, `DECISIONS.md` index line, HANDOFF line removed and
  summary/status updated, PRD milestones complete, validator green.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # Pg + Qdrant via HTUI_TEST_* (set in sandbox)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The classifier drifts from the Runs pane | Medium | Reuse `verdicts` / `unblock_enabled`; T2 fixtures mirror the pane's own enabled actions |
| The per-tick candidate read is heavy (run snapshots) | Low | Candidates are only `blocked` / `awaiting_approval` items' active runs, a small set; measure in T3 |
| Snapshot churn collides across tasks | High if parallel | T3 alone owns the 109 top-bar and 104 help-line snapshots, in one pass after the parallel wave; T5 adds new snapshot files only |
| `.sqlx` drift from parallel tasks | Medium | Only T1 adds `query!` calls; byte-identical SQL reuses entries (memory) |
| The orch-test stack is near 2 MiB | Low | No engine futures are touched; gate with `--no-fail-fast`, grep SIGABRT |
| Transient gate row during `fail_judge` (step briefly `AwaitingApproval`) | Low | A single walk transaction window; the next tick corrects it |

## Verified claims

Fact-checked 2026-10-03 by two parallel read-only agents (store/engine S1–S12, TUI U1–U10) against
`187ca50e`+docs. Falsified or partial claims were amended above before CONFIRM.

| # | claim | verdict | evidence |
|---|---|---|---|
| 1 | Every non-permission reason leaves the item `blocked`/`awaiting_approval` | PARTLY → D2 amended | parks move the item only from `in_progress` (traits.rs:1502-1505, engine.rs:4470); an `open` item under a parked run is now a candidate via run status |
| 2 | `escalate` parks a step | FALSE → fixture amended | run → `AwaitingApproval`, item → `blocked`, no step parked (gate.rs:1050-1051) |
| 3 | `fail_judge` leaves the judge `failed` with `gate_note`; nothing else does | PARTLY → D3 amended | engine.rs:5163-5174; the recovery sweep's "interrupted" judge also does (orch conformance.rs:5013-5045), shown as a judge row |
| 4 | `group_at` finds the judge | FALSE → `judge_at` | `group_at` keeps `fanout_index >= 0` (status.rs:219); `judge_at` (status.rs:231) |
| 5 | `verdicts` is callable in views.rs; empty heads/live leave `select` intact | TRUE | views.rs:325, 385-398; heads only feed approve/accept/open |
| 6 | `ItemActions.unblock` gives the case | FALSE → D3 amended | `.map(drop)` (views.rs:441); call `unblock_enabled` (command.rs:1173) directly |
| 7 | A gate without output: Rejected Ok, Approved refused | TRUE | command.rs:682-703 |
| 8 | Relay open predicate; Mem equivalent | TRUE | pg/relay.rs:501-524; mem.rs:6133-6140, 6329 |
| 9 | Permission scoping by project covers only item runs | FALSE → D4 amended | chat runs have `item_id` NULL; filter `r.item_id IS NOT NULL` |
| 10 | `writer()` is None only offline; relay tables not mirrored | TRUE | backend.rs:147-153; no relay tables in `cache_migrations/`; mod-42.md:17-19 |
| 11 | Implementor list (Mem, Pg, Cache, Backend, Writer, UsageSpy, SpyStore); no default bodies | TRUE | mem.rs:6444/6570, pg/read.rs:66, pg/write.rs:783, cache/read.rs:268, backend.rs:625, writer.rs:125/320, agent conformance.rs:640/746, recorder.rs:327/434 |
| 12 | New conformance cases are picked up automatically | PARTLY → T1 amended | lists loop, but counts are pinned (mem_store.rs:36, :66; pg_conformance.rs:28) and `run_case` needs arms; READ_CASES may not write (fixtures have no parked runs) |
| 13 | `.sqlx` lives at the repo root | FALSE → T1 amended | `crates/htui-store/.sqlx` (322 files); recipe docs/hr-sandbox.md:194-209 |
| 14 | Model rows derive `Eq`; "working + waiting = active" | FALSE → T0, D5, PRD amended | run.rs:167, 212; item.rs:117; relay.rs:74 derive no `Eq`; Reopen rows and multi-row runs break the sum |
| 15 | A new ReadStore method forces edits in htui-worker/htui | FALSE (good) | `WorkerHost` has no ReadStore supertrait (worker.rs:360) |
| 16 | `ActiveRuns` has other users | FALSE → D6 amended | only update.rs:132, :188; tests update.rs:668, :739, store_worker.rs:3162 |
| 17 | Overlays get `Ctx.top_bar` and redraw on every reply/tick | TRUE | state.rs:79; update.rs:44, :135, :278 |
| 18 | Tick is 4 × 250 ms | TRUE | update.rs:19; event_loop.rs:19 |
| 19 | `Ctrl+W` is unbound everywhere; text widgets pass chords | PARTLY → D7 amended | widgets pass chords (text_field.rs:146-154, text_area.rs:191, chat/mod.rs:449-455); Boxes matches `Char('w')` without modifiers (settings/boxes.rs:711), so T3 guards it |
| 20 | A new `RevealTarget` variant breaks other matches | PARTLY | only `kind()`/`key()` in action.rs are exhaustive; others use let-else |
| 21 | The Backlog reaches the Runs pane typed | FALSE → D8 amended | `Box<dyn DetailTab>`, no downcast (detail/mod.rs:67, 122-127); the type is `RunsTab` (runs.rs:322); new `DetailTab::focus` |
| 22 | Reveal of the selected item re-reads runs | FALSE → D8 amended | `go` returns early (backlog/mod.rs:191); `on_item_change` clears the pane (runs.rs:1270) |
| 23 | 109 of 184 snapshots carry the top bar | PARTLY | 109 of 146 under crates/htui (184 repo-wide); 104 also carry the global help line; re-accept with `INSTA_UPDATE=always` (README.md:489-494) |
| 24 | `htui_worker::views::WaitingView` is nameable from htui | FALSE → T2 amended | `mod views` is private (lib.rs:43); re-export via lib.rs:56 |
| 25 | T1, T2 and T5 file sets are disjoint | TRUE (after amendments) | T1: htui-core store + tests/mem_store.rs, htui-store src/tests/.sqlx, htui-agent spies. T2: htui-worker views.rs + lib.rs. T5: htui action.rs, backlog/mod.rs, detail/mod.rs, detail/runs.rs. No shared file; T3 (serial) owns every snapshot, app/{state,update,mod}.rs and store_worker.rs |
| 26 | Theme has a warning style | FALSE → T3 uses `accent` | theme.rs:11-24 |
| 27 | Overlays use a `J`/`K` cursor | FALSE → T3 uses `j`/`k` + arrows | workspace_switcher.rs:161-170 |

## Acceptance
- [x] All tasks complete (T0, T1, T2, T5, T3; review round R1)
- [x] Validation passes (see `docs/decisions/mod/mod-69.md` Verification)
- [x] Patterns mirrored, not reinvented

**Status: done 2026-10-04.**
