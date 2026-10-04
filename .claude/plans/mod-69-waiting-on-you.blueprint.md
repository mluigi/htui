# Blueprint: MOD-69, waiting-on-you list across items

**Status**: **proposed** (2026-10-04). The amendments A-1 to A-12 (§0) and the hazards H-1 to H-24
(§0a) belong to this blueprint. So do the blueprint decisions E1 to E14 (§7). The plan's D1–D9 are
binding and stay closed. Where a fix narrows one of them, the row cites the evidence. An
amendment marked **Blocker** means that the plan, read literally, does not compile, or produces a
defect that its own tests would show.

**Plan**: `.claude/plans/mod-69-waiting-on-you.plan.md` at `c81ffbbd`, confirmed and fact-checked
(27 claims). PRD: `.claude/prds/mod-69-waiting-on-you.prd.md`. Requirements: `docs/REQUIREMENTS.md`
R-TUI-1 (`:336-338`, "working run count and waiting-on-you count") and R-TUI-11 (`:373-377`). Outside
this file, cite tasks as "MOD-69 T*n*".

**Verified at**: HEAD `c81ffbbd`, branch `hr/MOD-69`, clean tree, sandbox (`HR_SANDBOX=1`).
Anchors were read through Gortex (`search`, `read`, `relations`). It answers despite its INACTIVE
banner. Shell reads were used only for counts. **Line numbers are pre-edit**: once a task commits
to a file, any citation into that file moves. Counted at this HEAD:
- 322 `crates/htui-store/.sqlx` files;
- 145 `crates/htui/tests/snapshots` plus 1 `crates/htui/src/snapshots` (HANDOFF's 143 is stale);
- store `CASES` 135 (HANDOFF says 134, which is stale: MOD-13 milestone 5 added one);
- `READ_CASES` 15.

**Graphify**: `graphify-out/` is not built and was not consulted.

**Coupling verdict.** The plan's waves stand: **T0 → {T1 ∥ T2 ∥ T5} → T3 → T6**. The file sets after
A-4/A-7 are:

| Task | Crates and files |
|---|---|
| T0 | `htui-core/src/model/{waiting.rs,mod.rs}` |
| T1 | `htui-core` store + `tests/mem_store.rs`, `htui-store` src/tests/`.sqlx`, `htui-agent` spies |
| T2 | `htui-worker/src/{views.rs,lib.rs}` |
| T5 | `htui/src/app/action.rs`, `ui/tabs/backlog/{mod.rs,detail/mod.rs,detail/runs.rs}`, `tests/reveal.rs` |
| T3 | `htui/src/{store_worker.rs,app/{state,update,mod}.rs,ui/top_bar.rs,ui/overlay/*,ui/tabs/settings/boxes.rs,testkit.rs}`, `htui/tests/{shell,integration,backlog}.rs`, new `tests/waiting.rs`, every `.snap` |

No two parallel tasks share a file. T3 runs alone, after the merge.

**Scope**:
- **No migration, no new dependency, no new crate.** Four new `.sqlx` entries: three in
  `waiting_candidates` and one in `open_permissions`.
- **New code**:
  - model `WaitingCandidate`, `WaitingPermission`;
  - `ReadStore::waiting_candidates` on 7 implementors and `WriteStore::open_permissions` on 5;
  - worker `WaitingReason`/`WaitingRow`/`WaitingView`/`waiting`, plus `unblock_case`, extracted from
    `verdicts`;
  - `RevealTarget::Step`, `DetailTab::focus`/`DetailRegistry::focus`, and the RunsTab pending focus;
  - `StoreRequest::Waiting`/`StoreReply::Waiting`, which replace `ActiveRuns`;
  - the `WaitingList` overlay and `Ctrl+W`.
- **Pins that move** (T6 re-counts them):
  - store `CASES` 135 → **137**; `READ_CASES` stays at 15;
  - `StoreRequest` and `StoreReply` keep their counts (one removed and one added);
  - `.sqlx` 322 → **326**;
  - snapshots: ~109 top-bar lines move, plus 2 new `tests/snapshots` and 1 new `src` snapshot.

**House style (carried)**:
- `unsafe_code = "forbid"`. `missing_docs` warns in `htui-worker` (`lib.rs:38`), so every new `pub`
  item gets a doc. `unused_qualifications` and `clippy::all` warn. The gate is `cargo clippy
  --workspace --all-targets --all-features -- -D warnings`.
- `SQLX_OFFLINE=true`. A `query!` lands in the **same commit** as its `.sqlx` entry (H-16).
- Implementers commit after each step and stage only their own paths. Never `-A`, never `stash`,
  never `--amend`. Every commit compiles.
- A red commit may hold a `todo!()` body only where no product path calls it. Until T3,
  nothing calls either new store method or `waiting`.
- Integration tests need `--all-features`. Without it, `tests/*.rs` runs 0 tests and still reports
  ok. Every test gate uses `--test-threads=1`.
- UI code never panics in a render path and never logs. `tracing::debug!` is for silent no-ops in
  update paths only.

---

## 0. Amendments (the plan against the tree)

| # | Blocker? | Plan says | Tree at `c81ffbbd` | Fix |
|---|---|---|---|---|
| **A-1** | **Blocker** (does not compile) | T0: `OpenPermission { item, item_key, permission }` in `model/waiting.rs`. | `htui_core::model::OpenPermission` already exists. It is `WriteStore::open_permission`'s argument (`model/relay.rs:111-130`), is re-exported (`model/mod.rs` relay list), and is used in `traits.rs:54`, `pg/relay.rs:20`, `conformance.rs:14266`. | The new row is **`WaitingPermission`**. The method name `open_permissions` stays: it is plural, and it is distinct from `open_permission`. |
| **A-2** | **Blocker** (D9 cannot be honoured, and the overlay cannot label a permission's step) | `OpenPermission { item, item_key, permission: StepPermission }`. `waiting(active, candidates, permissions)`. | D9 sorts every row by project position, item key, run creation, then step position. A pending permission's run is usually `running`, so its item is usually **not** a candidate (D2), and the classifier has no other source for its project, key parts, run `queued_at` or step slot/phase. `StepPermission` carries only `run_id`/`run_step_id`. | §1: `WaitingPermission` carries `project`, `item_key`, `key_prefix`, `key_number`, `run_queued_at`, `step_position`, `step_attempt`, `step_fanout_index`, `phase_name` and `permission`. The store joins `run`, `item` and `run_step` once. `waiting` also takes `scope: &Scope` for the project position (§3.4). |
| **A-3** | Non-blocker (smaller churn) | T3: "104 carry the global help line that gains `Ctrl+w waiting`". | 72 snapshots end at `? help`, because their harness never calls `register_all`. The 32 that do (31 + `htui__testkit…shell_empty` style, all at 100 columns) carry `q quit · … · w workspaces · Ctrl+f find`, which is **99 cells**. The status line is an unwrapped `Paragraph` (`state.rs:710-713`), so ` · Ctrl+w waiting` is clipped at column 100. The harness then trims trailing blanks (`testkit.rs` `buffer_text`). No snapshot shows a `?` box. | **Zero** existing snapshots change for the binding. Only the top-bar line moves. The binding is visible in the `?` box (which wraps) and on terminals wider than 116 columns. See **M-1**. |
| **A-4** | **Blocker** (tests go red, and those files are outside T3) | T3's files: `store_worker.rs`, `app/*`, `top_bar.rs`, overlay, `boxes.rs`, `tests/backlog.rs` (comment), snapshots. | Five non-snapshot assertions spell the top bar: `tests/shell.rs:114`, `:146`, `:204`, `tests/integration.rs:55` and `src/testkit.rs:947` (`"… · memory · 0 runs"` / `"· 1 run"`). | Add those three files to T3. New expected strings: Graphics `… · 0 working · 0 waiting` and Platform `… · 1 working · 1 waiting` (H-5). T3 also gets a new `crates/htui/tests/waiting.rs` for its integration cases, so it touches no shared test file beyond these. |
| **A-5** | **Blocker** (the flow view would show a stale node, and the already-selected case cannot re-read) | D8: `DetailTab::focus(&mut self, run, step)` with a no-op default. "otherwise it re-requests `Runs`". | `RunsTab::sync_graph(&mut self, theme)` (`runs.rs:377-387`) needs the theme to rebuild the flow after the cursor moves, and a sub-tab has no other path to a `Theme` or to `ctx.request`. | `fn focus(&mut self, _run: Option<RunId>, _step: Option<StepId>, _ctx: &Ctx<'_>) {}`. **The Backlog** re-requests `Runs(item)` when `go` short-circuits, because it knows that case (§4.3). The pane only applies the focus, immediately or on the next `on_runs`. |
| **A-6** | Non-blocker (an honest empty state) | D6: `TopBarState.waiting: WaitingView`. | `WaitingView::default()` has `permissions_known = false`. An overlay opened before the first `Waiting` reply (the startup frames, or an empty scope where the tick never asks, `update.rs:130`) would claim "permissions unavailable offline" while online. The workspace switcher tells "still reading" apart from "nothing there" (`workspace_switcher.rs:47-49`, `:111-118`). | `TopBarState.waiting: Option<WaitingView>`. `None` means no reply yet: the overlay says "reading the store", and the top bar shows `0 working · 0 waiting`. `TopBarState.active_runs` is **removed**. Its only reader is `top_bar.rs:29-33` and its only writer `update.rs:363` (`find_usages`: none else). |
| **A-7** | Non-blocker (coverage) | T5's files: `action.rs`, `backlog/mod.rs`, `detail/mod.rs`, `detail/runs.rs`. "New snapshot files only". | `crates/htui/tests/reveal.rs` is the existing end-to-end reveal suite (`platform_unserved`, `platform`, `reveal`, `titled`), with string assertions rather than snapshots. T3 does not touch it. | Add `tests/reveal.rs` to T5 for three end-to-end cases (§4.6). T5 then needs no new snapshot at all. |
| **A-8** | Non-blocker | T0: "No logic." | Every backend has to group three reads into `(Item, [(Run, [RunStep])])` and return the same order (MOD-72's "Rust sorts" rule). Four copies of the grouping would drift. | T0 adds `WaitingCandidate::assemble` and `sort_canonical`, plus `WaitingPermission::sort_canonical`, with unit tests (§1). |
| **A-9** | Non-blocker | T2: "`WaitingView`/`WaitingRow` derive `Debug, Clone, Default, PartialEq, Eq`." | `WaitingRow` holds a `WaitingReason`, which has no meaningful default. | `WaitingRow`: `Debug, Clone, PartialEq, Eq`. `WaitingView`: all five derives. `TopBarState` still derives `Default` (A-6: `Option`). |
| **A-10** | Non-blocker (the gate as written errors) | T5 Validate: `cargo test -p htui --all-features backlog`. T3: `-p htui`. | One positional `TESTNAME` is fine, but `--test-threads=1` is missing (memory: the htui suite is scheduling-dependent). | §6 lists per-task gates with filters and `--test-threads=1`. |
| **A-11** | Non-blocker (bookkeeping) | "count pins bumped (mem_store.rs:36, :66; pg_conformance.rs:28)". | `CASES.len()` is `135` at `mem_store.rs:36-37` and `EXPECTED_CASES = 135` at `pg_conformance.rs:28`, with the message at `:35`. `READ_CASES` is `15` at `:64-66` and does not move: both new cases write. | 135 → **137** in all three places (§2.6). |
| **A-12** | Non-blocker (a fixture that cannot be built as worded) | T1 negatives: "a chat run" (candidate) and "chat-run rows" (permission). | A chat run has `item_id NULL`, so the item-driven candidate read can never reach it. A chat run holds no lease (`conformance.rs:15093-15096`), so `open_permission` fences it. `take_lease` admits it, though: a chat run is `running` with `executing_box_id = target_box_id` (`run.rs:291-293`), and `take_lease` takes `lease_owner IS NULL` (`traits.rs:1236-1240`). | §2.5 builds the chat-run permission with `start_chat_run` → `take_lease` → `open_permission`, and asserts it absent. The candidate negative stays: a chat run sits in scope, and the list must not change. |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | `unblock_enabled` is order-sensitive. It takes the **first** parked run and the **first** active run (`command.rs:1176-1185`). `verdicts` receives runs in `ReadStore::runs` order, `queued_at DESC` (`pg/read.rs:315`, `cache/read.rs:527`). | `WaitingCandidate::sort_canonical` orders each candidate's runs `(queued_at DESC, id DESC)`, the pane's order (§1). D9's row order is applied later, inside `waiting`, and never feeds `unblock_enabled`. |
| **H-2** | The list could drift from the pane's `u` verdict: `ItemActions.unblock` drops the case (`views.rs:441`), so the plan re-implements the `active` loop. | E2: `views::unblock_case(item, runs) -> Result<UnblockCase, String>` holds the loop once. `verdicts` calls it as `.map(drop)`, so both read one function. The existing `unblock_is_enabled_over_a_crashed_rejection` / `…_followed_escalation` tests pin the refactor (§3.2). |
| **H-3** | `Overlay::render` takes `&self` (`registry.rs:56`), so a cursor clamp cannot be stored at draw time. | §5.4: the cursor is stored unclamped. `fn at(&self, len) -> Option<usize>` clamps on every read (render, `j`/`k`, `Enter`), and `j`/`k` write back the clamped value. |
| **H-4** | `pending_reveal`'s type changes (it now carries the focus). Its tuple is asserted by `an_applied_mint_closes_the_form_clears_the_filter_and_reveals_it` (`backlog/mod.rs:2358`). | Change that one assertion to the `PendingReveal` struct (§4.3). The struct literals at `:916`, `:1091` and `:1141` say `pending_reveal: None` and still compile. |
| **H-5** | The demo fixture already holds candidates. In Platform scope, `htui` FEAT-2 is `blocked` with no run, a **Reopen** row. `htui` TOOL-1 is `awaiting_approval` with no run: a candidate, but **no row** (`unblock_enabled` → `nothing_is_blocked`). `RUN_2` (FEAT-3) is the only active run (`fixtures.rs:960-1010`, `:1436-1500`). | Every Platform expectation counts them. The top bar reads `1 working · 1 waiting`, and the conformance baseline lists FEAT-2 and TOOL-1 with `runs: []` (§2.5). Graphics (Vulkan) holds none: `0 working · 0 waiting`. |
| **H-6** | `claim_run` refuses a third **running** run on the box. `DEFAULT_MAX_CONCURRENT_ITEMS = 2` (`model/box_.rs:341`), and the demo box sets none. | The §2.5 fixtures park each run before claiming the next. A parked run holds no slot (`traits.rs` `claim_run` doc). The `running` negative is claimed last. |
| **H-7** | The three candidate statements are not one snapshot, so a park landing between them can mismatch for one tick. | Accepted, on the relay precedent ("a display read, so two snapshots are harmless", `pg/relay.rs:499-500`). The next tick corrects it. The doc comment says so. |
| **H-8** | A promoted step is `awaiting_approval` with `promoted_at` set (`traits.rs` `promote_step`), so D3 lists it as a gate. | It is a gate row, and it waits on a person (accept or reject). With no `gate_note`, the text is `"promoted to chat"`, not `"gate"` (E6). See **M-2**. |
| **H-9** | Conformance doc-name rule: `every_cross_referenced_test_name_exists` (`conformance.rs:16377`) panics on a back-ticked snake_case token with ≥4 `_` that `conformance.rs`/`mem.rs` does not define. | New doc comments there name only `waiting_candidates`, `open_permissions`, `relay_view` and the two new cases. They never name a `cache.rs` or `views.rs` test. |
| **H-10** | `observe_reply` runs **before** the freshness gate (`update.rs:278`). After a scope change, the old scope's queued `Waiting` reply lands first and briefly writes the old counts. | The store worker serves FIFO, so the new scope's reply lands last and wins within the same drain. `set_scope` also clears `top_bar.waiting = None` (E9), so nothing of the old workspace shows past one frame. |
| **H-11** | `Item` (with `body`), `Run` (with `graph_snapshot`) and every step of each candidate's active runs are read once a second. | The set is small (D2). T3 records one timing of `serve(Waiting)` over the demo in the decisions write-up. No cache is added (PRD "no own state"). |
| **H-12** | Online, a failing `open_permissions` fails the whole `Waiting` reply: a `Failed` (status line) once a second, or `Unreachable` (`go_offline`). | That is `ActiveRuns`' behaviour today, and `RunActions`'. `Unreachable` must still drop the backend, so the reply propagates and is not swallowed (E10). |
| **H-13** | `Ctrl+W` is "delete word" in many line editors, so a future embedded editor (MOD-57) may want it. | Not bound by any widget today: every text widget passes chords (plan D7). MOD-67 makes it remappable. Recorded in the decisions write-up. |
| **H-14** | Settings › Boxes matches `Char('w')` with no modifier check (`boxes.rs:711`), so `Ctrl+W` opens a box's executor today (plan D7). | Guard only the `'w'` arm (§5.5). The other arms (`j`, `k`, `t`, `e`, `s`, `p`, `r`) keep their behaviour: only `Ctrl+W` is globally bound. |
| **H-15** | `MemStore` timestamps carry nanoseconds and Postgres microseconds. | The conformance writers use `seam_clock()` (`conformance.rs:4517`, truncated). The candidate sort breaks `queued_at` ties by id. |
| **H-16** | `SQLX_OFFLINE`: a `query!` with no `.sqlx` entry does not compile, and the entry hashes the **literal** text. | §2.6 recipe. The macro and its entry land in one commit. Re-run `prepare` after any SQL edit. Gate: exactly four `??` in `git status --porcelain crates/htui-store/.sqlx`. |
| **H-17** | `WorkerStore` re-declares some `ReadStore`/`WriteStore` reads (`relay_view`, `worker.rs:164`). | **No** `WorkerStore`/`WorkerHost` method: the engine never reads the list (MOD-72 B-3 precedent). |
| **H-18** | The judge transient. `fail_judge` moves the judge `running → awaiting_approval → failed` in two writes (`engine.rs:5155-5166`), so a tick between them shows a gate row on the judge. | Accepted (plan Risks). It lasts one tick. |
| **H-19** | One run can own several rows (gates on candidates of a gated slot, gate plus FollowRun, gate plus permission). | D5 counts distinct runs: `working = active.saturating_sub(distinct row runs)`. `saturating_sub` also absorbs the race between `active_runs` and the candidate read. |
| **H-20** | Without a modifier guard, a `Ctrl+W` inside a capturing Boxes editor would still be swallowed (`boxes.rs:533`: CONTROL chords **pass**, which is fine). Inside another **modal overlay**, `Ctrl+W` is swallowed by `is_modal` (`state.rs:613-616`). | Accepted. "From every screen" means every tab (R-TUI-11). An overlay over an overlay is not wanted. |
| **H-21** | The `fan_out` of the conformance runs' snapshot is empty (`run_snapshot()` has no phases, `conformance.rs:4530`). | Fine for T1, which compares raw rows. Only T2 classifies, and T2 builds its own snapshots (§3.6). |
| **H-22** | The help-line clip (A-3) means `Ctrl+w` is undiscoverable on the default 100-column status line. | See **M-1**. The top bar's `M waiting` and the `?` box are the discovery paths. |
| **H-23** | T5's `tests/reveal.rs` cases run over the demo, and T3 then changes the top-bar text. | The reveal cases assert the detail title and the cursor line, never the top bar, so T3 does not touch them. |
| **H-24** | `ACTIVE_RUN_STATUSES` (`pg/read.rs:50`) pins the `status IN (…)` literal of `active_runs` only. | The new run read repeats the same literal. A comment names the const (§2.4). The existing `active_runs_literal_matches_run_status` test still guards the set. |

### 0b. Needs a maintainer decision

- **M-1 (A-3, H-22).** `Ctrl+w waiting` does not fit the 100-column status line (99 of 100 cells are
  taken). Options:
  - (a) **accept**: discovery is through the top bar and the `?` box. This is the blueprint
    default and needs no extra change;
  - (b) shorten another global help text (for example `Shift+Tab previous tab` → `prev tab`), which
    churns 32 snapshots.
- **M-2 (H-8).** A promoted step is listed as a gate row with the text `promoted to chat`. The
  alternative is to leave promoted steps out. The blueprint lists them, because they hold the run
  for a person.
- **M-3 (E5).** The wording of the three Unblock texts (§3.3), which the overlay shows verbatim.

---

## 1. T0: model types (`crates/htui-core/src/model/waiting.rs`, new)

```rust
//! The waiting-on-you list's store rows (MOD-69 plan D2, D4; blueprint A-1, A-2, A-8): what
//! `ReadStore::waiting_candidates` and `WriteStore::open_permissions` answer. The list itself is
//! derived in `htui-worker` (`views::waiting`) and keeps no state of its own.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ids::{ItemId, ProjectId, RunId};
use crate::model::item::Item;
use crate::model::relay::StepPermission;
use crate::model::run::{Run, RunStep};
use crate::model::scope::Scope;

/// One item of [`crate::store::ReadStore::waiting_candidates`] (MOD-69 plan D2): an item in scope
/// that is `blocked` or `awaiting_approval`, or that owns a run at `awaiting_approval`, with its
/// **active** runs (`RunStatus::is_active`) and each such run's steps.
///
/// Derives no `Eq`: `Item`, `Run` and `RunStep` derive none (they hold `serde_json::Value`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitingCandidate {
    /// The item row, body included: `verdicts` and `unblock_enabled` read it whole.
    pub item: Item,
    /// Its active runs, newest first (`queued_at DESC, id DESC`: `ReadStore::runs`' order, which
    /// `unblock_enabled` is sensitive to, blueprint H-1), each with its steps in
    /// `(position, attempt, fanout_index)` order.
    pub runs: Vec<(Run, Vec<RunStep>)>,
}

impl WaitingCandidate {
    /// Groups three flat reads into candidates and sorts them ([`Self::sort_canonical`]): every
    /// backend reads items, runs and steps separately and calls this, so the grouping and the order
    /// cannot differ between them (blueprint A-8). A run whose item is not in `items`, and a step
    /// whose run is not in `runs`, is dropped; an item with no run keeps `runs: []`.
    #[must_use]
    pub fn assemble(scope: &Scope, items: Vec<Item>, runs: Vec<Run>, steps: Vec<RunStep>) -> Vec<Self> {
        let mut steps_of: BTreeMap<RunId, Vec<RunStep>> = BTreeMap::new();
        for step in steps {
            steps_of.entry(step.run_id).or_default().push(step);
        }
        let mut runs_of: BTreeMap<ItemId, Vec<(Run, Vec<RunStep>)>> = BTreeMap::new();
        for run in runs {
            let Some(item) = run.item_id else { continue };
            let steps = steps_of.remove(&run.id).unwrap_or_default();
            runs_of.entry(item).or_default().push((run, steps));
        }
        let mut rows: Vec<Self> = items
            .into_iter()
            .map(|item| {
                let runs = runs_of.remove(&item.id).unwrap_or_default();
                Self { item, runs }
            })
            .collect();
        Self::sort_canonical(&mut rows, scope);
        rows
    }

    /// Items by `(scope position of project_id, key_prefix bytes, key_number, id)` (a project not
    /// in `scope` last); each item's runs by `(queued_at, id)` **descending**; each run's steps by
    /// `(position, attempt, fanout_index, id)`. Rust, not SQL: Postgres would order text by
    /// collation and the mirror by bytes (the `UpstreamEntry::sort_canonical` reason).
    pub fn sort_canonical(rows: &mut [Self], scope: &Scope) {
        let position = |project: ProjectId| {
            scope.project_ids.iter().position(|id| *id == project).unwrap_or(usize::MAX)
        };
        for row in rows.iter_mut() {
            for (_, steps) in &mut row.runs {
                steps.sort_by_key(|step| (step.position, step.attempt, step.fanout_index, step.id));
            }
            row.runs.sort_by(|(a, _), (b, _)| (b.queued_at, b.id).cmp(&(a.queued_at, a.id)));
        }
        rows.sort_by(|a, b| {
            (position(a.item.project_id), a.item.key_prefix.as_bytes(), a.item.key_number, a.item.id)
                .cmp(&(position(b.item.project_id), b.item.key_prefix.as_bytes(), b.item.key_number, b.item.id))
        });
    }
}

/// One row of [`crate::store::WriteStore::open_permissions`] (MOD-69 plan D4, blueprint A-1, A-2):
/// a pending permission request of an item run whose owner holds the run's lease live, with what
/// the list sorts and labels it by (plan D9), so the classifier needs no second read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitingPermission {
    /// `run.item_id`; never a chat run's (plan D4).
    pub item: ItemId,
    /// `item.project_id`: plan D9's project position.
    pub project: ProjectId,
    /// `item.key`, e.g. `FEAT-2`.
    pub item_key: String,
    /// `item.key_prefix`: the key's sort parts, so `FEAT-2` sorts before `FEAT-10`.
    pub key_prefix: String,
    /// `item.key_number`.
    pub key_number: i32,
    /// `run.queued_at`: plan D9's run creation.
    pub run_queued_at: DateTime<Utc>,
    /// `run_step.position` of `permission.run_step_id`.
    pub step_position: i32,
    /// `run_step.attempt`.
    pub step_attempt: i32,
    /// `run_step.fanout_index` (`-1` = the slot's judge).
    pub step_fanout_index: i32,
    /// `run_step.phase_name`.
    pub phase_name: String,
    /// The request, as `relay_view` lists it.
    pub permission: StepPermission,
}

impl WaitingPermission {
    /// `(permission.created_at, permission.id)`: `relay_view`'s order, applied in Rust on every
    /// backend.
    pub fn sort_canonical(rows: &mut [Self]) {
        rows.sort_by_key(|row| (row.permission.created_at, row.permission.id));
    }
}
```

**`model/mod.rs`**: add `pub mod waiting;` after `pub mod user;` (the list is alphabetical:
`usage`, `user`, then `waiting`). Add `pub use waiting::{WaitingCandidate, WaitingPermission};`
after `pub use user::{…}`.

**Tests (first)**: `#[cfg(all(test, feature = "demo"))] mod tests` in `waiting.rs`. Rows are
cloned from `crate::fixtures::demo_data()` and their fields mutated.
- `waiting_candidates_assemble_by_item_and_drop_orphans`: two items, three runs (one of them on a
  third item, so it is dropped), steps of two runs plus one orphan step. Each item gets its runs
  with their steps, the orphan run and step are gone, and an item with no run keeps `runs: []`.
- `waiting_candidates_sort_by_scope_key_then_runs_newest_first`:
  - project B before project A when the scope lists B first;
  - `FEAT-2` before `FEAT-10` (by number, not text);
  - `"Zed"` before `"edit"` (bytes);
  - two runs newest first, a `queued_at` tie broken by id descending;
  - steps by `(position, attempt, fanout_index)` with the judge (`-1`) first.
- `waiting_permissions_sort_by_creation_then_id`.

**Validate**: `cargo test -p htui-core --all-features --lib model::waiting -- --test-threads=1`;
clippy.

**Commit (T0)**: one commit, `feat(mod-69): waiting-list model rows (WaitingCandidate, WaitingPermission)`.
Paths: `crates/htui-core/src/model/waiting.rs`, `crates/htui-core/src/model/mod.rs`.

---

## 2. T1: the store read surface (D1, D2, D4)

### 2.1 Trait (`crates/htui-core/src/store/traits.rs`)

Add `WaitingCandidate, WaitingPermission` to the `crate::model::{…}` list (`:47-65`). `cargo fmt`
places them.

Append to `ReadStore` after `tool_call_counts` (`:281`), before `}` (`:282`):

```rust

    // ---- MOD-69: the waiting-on-you list (ANA-27 §5.1 T8) ------------------------------------

    /// The scope's candidates for the waiting-on-you list (MOD-69 plan D2): every item of a scope
    /// project that is `blocked` or `awaiting_approval`, **or** owns a run at `awaiting_approval`
    /// (parks move the item only from `in_progress`, so run status is the anchor), each with its
    /// active runs (`queued | running | awaiting_approval`) and each such run's steps. A finished
    /// run of a candidate is not carried; a chat run (no item) never is. In
    /// [`WaitingCandidate::sort_canonical`](crate::model::WaitingCandidate::sort_canonical) order.
    /// Empty for an empty scope.
    ///
    /// Rows only: what waits, and why, is `htui-worker`'s classification over them (plan D1),
    /// because the Resume case needs `status::cursor` over the run's snapshot. A display read: its
    /// statements need not share a snapshot, and the next refresh corrects a park that landed
    /// between them.
    ///
    /// On `ReadStore` because `item`, `run` and `run_step` are mirrored: offline the mirror answers
    /// from its last refresh.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn waiting_candidates(&self, scope: &Scope) -> Result<Vec<WaitingCandidate>>;
```

Append to `WriteStore` after `answer_permission` (`:1805-1811`), before `}` (`:1812`):

```rust

    /// MOD-69 plan D4: every `pending` permission request of a scope project's **item** runs whose
    /// owner holds the run's lease live by the store's clock — [`relay_view`](Self::relay_view)'s
    /// predicate over a scope instead of one item — each with the item's key and project, the
    /// run's `queued_at` and the step's slot and phase (blueprint A-2). A chat run's request is
    /// never listed: it is answered in its Chat tab and has no item to reveal. In
    /// [`WaitingPermission::sort_canonical`](crate::model::WaitingPermission::sort_canonical)
    /// order. Empty for an empty scope.
    ///
    /// A read on `WriteStore` by `relay_view`'s precedent: `step_permission` is not mirrored
    /// (MOD-42 OQ-4), so offline there is nothing to answer from.
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn open_permissions(&self, scope: &Scope) -> Result<Vec<WaitingPermission>>;
```

No default bodies. A default would let a spy skip forwarding without anyone noticing (plan T1.3).

### 2.2 The implementors

| Implementor | Trait | Insert after (pre-edit) | Body |
|---|---|---|---|
| `MemStore` | Read | `tool_call_counts` (`mem.rs:6565-6567`), before `}` `:6568` | `Ok(self.read(\|state\| state.waiting_candidates(scope)))` |
| `MemStore` | Write | `answer_permission` (`mem.rs:7361…`), the impl's last method | `let now = self.now(); Ok(self.read(\|state\| state.open_permissions(scope, now)))` |
| `PgStore` | Read | `tool_call_counts` (`pg/read.rs:1125-1154`), before `}` `:1155` | §2.4 |
| `PgStore` | Write | `answer_permission` (`pg/write.rs:6429…`) | `super::relay::open_permissions(self, scope).await` |
| `CacheStore` | Read | `tool_call_counts` (`cache/read.rs:1249-1279`), before `}` `:1280` | §2.4 |
| `Backend` | Read | `tool_call_counts` (`backend.rs:836-842`) | `match self { Self::Memory(store) => store.waiting_candidates(scope).await, Self::Online { pg, .. } => pg.waiting_candidates(scope).await, Self::Offline { cache, .. } => cache.waiting_candidates(scope).await }` |
| `Writer` | Read | `tool_call_counts` (`writer.rs:312-317`) | `match self { Self::Memory(store) => store.waiting_candidates(scope).await, Self::Online(pg) => pg.waiting_candidates(scope).await }` |
| `Writer` | Write | `answer_permission` (`writer.rs:1346…`) | the same two arms over `open_permissions` |
| `UsageSpy` | Read | `tool_call_counts` (`htui-agent/src/conformance.rs:741`) | `self.inner.waiting_candidates(scope).await` |
| `UsageSpy` | Write | `answer_permission` (`:1388…`) | `self.inner.open_permissions(scope).await` |
| `SpyStore` | Read | `tool_call_counts` (`htui-agent/tests/recorder.rs:429`) | as `UsageSpy` |
| `SpyStore` | Write | `answer_permission` (`recorder.rs:~1114`) | as `UsageSpy` |

The spies return `StoreResult<…>`, as their neighbours do. Each file adds `WaitingCandidate`,
`WaitingPermission` (and `Scope` where it is missing) to its own `model::{…}` import.

### 2.3 `MemStore` (`crates/htui-core/src/store/mem.rs`)

Add two `State` helpers. Put `waiting_candidates` after `State::tool_call_counts` (`:1361-1398`) and
`open_permissions` after `State::relay_view` (`:6329-6356`), where `live_owner` (`:6133`) lives:

```rust
    /// MOD-69 plan D2: the scope's candidates, grouped and sorted by `WaitingCandidate::assemble`.
    fn waiting_candidates(&self, scope: &Scope) -> Vec<WaitingCandidate> {
        let parked: BTreeSet<ItemId> = self
            .runs
            .values()
            .filter(|run| run.status == RunStatus::AwaitingApproval)
            .filter_map(|run| run.item_id)
            .collect();
        let items: Vec<Item> = self
            .items
            .values()
            .filter(|item| scope.contains(item.project_id))
            .filter(|item| {
                matches!(item.status, Status::Blocked | Status::AwaitingApproval)
                    || parked.contains(&item.id)
            })
            .cloned()
            .collect();
        let wanted: BTreeSet<ItemId> = items.iter().map(|item| item.id).collect();
        let runs: Vec<Run> = self
            .runs
            .values()
            .filter(|run| run.status.is_active())
            .filter(|run| run.item_id.is_some_and(|item| wanted.contains(&item)))
            .cloned()
            .collect();
        let owners: BTreeSet<RunId> = runs.iter().map(|run| run.id).collect();
        let steps: Vec<RunStep> = self
            .steps
            .values()
            .filter(|step| owners.contains(&step.run_id))
            .cloned()
            .collect();
        WaitingCandidate::assemble(scope, items, runs, steps)
    }

    /// MOD-69 plan D4: `relay_view`'s predicate over the scope's item runs, joined to the item and
    /// the step.
    fn open_permissions(&self, scope: &Scope, now: DateTime<Utc>) -> Vec<WaitingPermission> {
        let mut rows: Vec<WaitingPermission> = self
            .permissions
            .values()
            .filter(|p| {
                p.row.status == PermissionStatus::Pending
                    && self.live_owner(p.row.run_id, now) == Some(p.owner)
            })
            .filter_map(|p| {
                let run = self.runs.get(&p.row.run_id)?;
                let item = self.items.get(&run.item_id?)?;
                if !scope.contains(item.project_id) {
                    return None;
                }
                let step = self.steps.get(&p.row.run_step_id)?;
                Some(WaitingPermission {
                    item: item.id,
                    project: item.project_id,
                    item_key: item.key.clone(),
                    key_prefix: item.key_prefix.clone(),
                    key_number: item.key_number,
                    run_queued_at: run.queued_at,
                    step_position: step.position,
                    step_attempt: step.attempt,
                    step_fanout_index: step.fanout_index,
                    phase_name: step.phase_name.clone(),
                    permission: p.row.clone(),
                })
            })
            .collect();
        WaitingPermission::sort_canonical(&mut rows);
        rows
    }
```

Imports (`mem.rs:24-47`): add `WaitingCandidate`, `WaitingPermission`. `Status`, `RunStatus`,
`PermissionStatus`, `Item`, `Run`, `RunStep`, `RunId`, `ItemId`, `Scope`, `BTreeSet`, `DateTime` and
`Utc` are already in scope. Check with `cargo check`, and add any that are missing.

### 2.4 `PgStore` and `CacheStore`

**`PgStore::waiting_candidates`** (`pg/read.rs`, inside `impl ReadStore for PgStore`). It uses
three `query_as!` calls, so it needs three `.sqlx` entries. Write the text exactly as below. The
column lists are `PgStore::item` (`:132-157`), `PgStore::run` (`:642-670`) and
`PgStore::run_steps` (`:676-707`), copied.

```rust
    /// MOD-69 plan D1, D2: three reads — the candidate items, their active runs, those runs' steps —
    /// grouped and ordered by `WaitingCandidate::assemble`. A display read: the statements need not
    /// share a snapshot (blueprint H-7). The run read's `status IN (...)` is `ACTIVE_RUN_STATUSES`'
    /// literal (blueprint H-24).
    async fn waiting_candidates(&self, scope: &Scope) -> Result<Vec<WaitingCandidate>> {
        if scope.is_empty() {
            return Ok(Vec::new());
        }
        let projects = project_uuids(scope);
        let items = sqlx::query_as!(
            Item,
            r#"
            SELECT i.id            AS "id: ItemId",
                   i.project_id    AS "project_id: ProjectId",
                   i.kind_id       AS "kind_id: htui_core::model::ItemKindId",
                   i.key_prefix,
                   i.key_number,
                   i.key           AS "key!",
                   i.title,
                   i.body,
                   i.status        AS "status: htui_core::model::Status",
                   i.priority,
                   i.required_tags,
                   i.touched_paths,
                   i.step_graph_id AS "step_graph_id: htui_core::model::StepGraphId",
                   i.version,
                   i.created_by    AS "created_by: htui_core::model::UserId",
                   i.created_at,
                   i.updated_at,
                   i.closed_at,
                   i.resolution    AS "resolution: htui_core::model::Resolution"
              FROM item i
             WHERE i.project_id = ANY($1)
               AND (i.status IN ('blocked', 'awaiting_approval')
                    OR EXISTS (SELECT 1 FROM run r
                                WHERE r.item_id = i.id AND r.status = 'awaiting_approval'))
            "#,
            &projects[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let item_ids: Vec<Uuid> = items.iter().map(|item| item.id.as_uuid()).collect();
        let runs = sqlx::query_as!(
            Run,
            r#"
            SELECT id               AS "id: RunId",
                   project_id       AS "project_id: ProjectId",
                   item_id          AS "item_id: ItemId",
                   kind             AS "kind: RunKind",
                   mode             AS "mode: RunMode",
                   status           AS "status: RunStatus",
                   target_box_id    AS "target_box_id: BoxId",
                   executing_box_id AS "executing_box_id: BoxId",
                   graph_snapshot,
                   started_by       AS "started_by: UserId",
                   queued_at,
                   started_at,
                   finished_at,
                   failure,
                   repo_scope       AS "repo_scope: Vec<RepoId>",
                   lease_box_id     AS "lease_box_id: BoxId",
                   lease_expires_at,
                   updated_at
              FROM run
             WHERE item_id = ANY($1)
               AND status IN ('queued','running','awaiting_approval')
            "#,
            &item_ids[..],
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let run_ids: Vec<Uuid> = runs.iter().map(|run| run.id.as_uuid()).collect();
        let steps = if run_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_as!(
                RunStep,
                r#"
                SELECT id                  AS "id: StepId",
                       run_id              AS "run_id: RunId",
                       position,
                       attempt,
                       fanout_index,
                       phase_name,
                       agent_id            AS "agent_id: AgentId",
                       model,
                       status              AS "status: StepStatus",
                       gate_outcome        AS "gate_outcome: GateOutcome",
                       gate_note,
                       selected,
                       exit_code,
                       prompt_digest,
                       trim_record,
                       usage,
                       isolation_path,
                       started_at,
                       finished_at,
                       verify_outcome      AS "verify_outcome: VerifyOutcome",
                       verify_exit_code,
                       promoted_at,
                       updated_at
                  FROM run_step
                 WHERE run_id = ANY($1)
                "#,
                &run_ids[..],
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        };
        Ok(WaitingCandidate::assemble(scope, items, runs, steps))
    }
```

There is no `ORDER BY`: `assemble` sorts. Imports (`pg/read.rs:17-31`): add `WaitingCandidate`.
`Uuid` is already imported (`:36`).

**`open_permissions`** (`pg/relay.rs`, a free `pub(super)` fn after `relay_view`, `:501-545`). It
uses `query!` and maps by hand, because the record is wider than `PermissionRecord`:

```rust
/// [`WriteStore::open_permissions`](htui_core::store::WriteStore::open_permissions) (MOD-69 plan
/// D4): `relay_view`'s open predicate over the scope's item runs, joined to the item and the step.
/// `r.item_id IS NOT NULL` is implied by the item join and kept explicit: plan D4's chat-run rule.
pub(super) async fn open_permissions(store: &PgStore, scope: &Scope) -> Result<Vec<WaitingPermission>> {
    if scope.is_empty() {
        return Ok(Vec::new());
    }
    let projects: Vec<Uuid> = scope.project_ids.iter().map(|id| id.as_uuid()).collect();
    let rows = sqlx::query!(
        r#"SELECT p.id           AS "id: PermissionId",
                  p.run_id       AS "run_id: RunId",
                  p.run_step_id  AS "run_step_id: StepId",
                  p.session      AS "session: RelaySessionId",
                  p.request_id,
                  p.tool_call_id,
                  p.summary,
                  p.options      AS "options: Json<Vec<RelayOption>>",
                  p.status       AS "status: PermissionStatus",
                  p.option_id,
                  p.answered_by  AS "answered_by: UserId",
                  p.answered_box AS "answered_box: BoxId",
                  p.created_at,
                  p.answered_at,
                  p.resolved_at,
                  i.id           AS "item_id: ItemId",
                  i.project_id   AS "project_id: ProjectId",
                  i.key          AS "item_key!",
                  i.key_prefix,
                  i.key_number,
                  r.queued_at    AS run_queued_at,
                  s.position     AS step_position,
                  s.attempt      AS step_attempt,
                  s.fanout_index AS step_fanout_index,
                  s.phase_name
             FROM step_permission p
             JOIN run r      ON r.id = p.run_id
             JOIN item i     ON i.id = r.item_id
             JOIN run_step s ON s.id = p.run_step_id
            WHERE i.project_id = ANY($1) AND r.item_id IS NOT NULL
              AND p.status = 'pending'
              AND r.lease_owner = p.owner AND r.lease_expires_at > clock_timestamp()
            ORDER BY p.created_at, p.id"#,
        &projects[..],
    )
    .fetch_all(&store.pool)
    .await
    .map_err(map_sqlx)?;

    let mut out: Vec<WaitingPermission> = rows
        .into_iter()
        .map(|row| WaitingPermission {
            item: row.item_id,
            project: row.project_id,
            item_key: row.item_key,
            key_prefix: row.key_prefix,
            key_number: row.key_number,
            run_queued_at: row.run_queued_at,
            step_position: row.step_position,
            step_attempt: row.step_attempt,
            step_fanout_index: row.step_fanout_index,
            phase_name: row.phase_name,
            permission: StepPermission {
                id: row.id,
                run_id: row.run_id,
                run_step_id: row.run_step_id,
                session: row.session,
                request_id: row.request_id,
                tool_call_id: row.tool_call_id,
                summary: row.summary,
                options: row.options.0,
                status: row.status,
                option_id: row.option_id,
                answered_by: row.answered_by,
                answered_box: row.answered_box,
                created_at: row.created_at,
                answered_at: row.answered_at,
                resolved_at: row.resolved_at,
            },
        })
        .collect();
    WaitingPermission::sort_canonical(&mut out);
    Ok(out)
}
```

Imports (`pg/relay.rs:18-22`): add `ProjectId, Scope, WaitingPermission`. `ItemId` is already
there. If sqlx infers a joined `NOT NULL` column as nullable (it does not for inner-joined base
columns, but `prepare` is the judge), add the `!` override on that alias **before** `prepare`.

**`CacheStore`** (`cache/read.rs`). It uses runtime `sqlx::query` with `placeholders(n)` +
`AssertSqlSafe`, as `active_runs` (`:1380-1395`) does. The file contributes no `.sqlx`.

1. **Extract, behaviour-identical**:
   - `fn run_of(row: &SqliteRow) -> Result<Run>` from `CacheStore::run`'s body (`:837-878`);
   - `fn run_step_of(row: &SqliteRow) -> Result<RunStep>` from `run_steps`' closure (`:899-926`);
   - `const RUN_SELECT: &str` (the 18 columns of `run`'s `SELECT`) and `const RUN_STEP_SELECT: &str`
     (the 23 of `run_steps`').

   Put them beside `item_of`/`REQUIREMENT_SELECT` (`:158-228`), and have `run` and `run_steps`
   call them (the `REQUIREMENT_SELECT` precedent). Also add `const ITEM_SELECT: &str = "i.id,
   i.project_id, i.kind_id, i.key_prefix, i.key_number, i.key, i.title, i.body, i.status,
   i.priority, i.required_tags, i.touched_paths, i.step_graph_id, i.version, i.created_by,
   i.created_at, i.updated_at, i.closed_at, i.resolution"`. SQLite names a result column `i.id` as
   `id`, which `item_of` reads.

2. **The method**:

```rust
    async fn waiting_candidates(&self, scope: &Scope) -> Result<Vec<WaitingCandidate>> {
        if scope.project_ids.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {ITEM_SELECT} FROM item i WHERE i.project_id IN ({}) \
               AND (i.status IN ('blocked', 'awaiting_approval') \
                    OR EXISTS (SELECT 1 FROM run r \
                                WHERE r.item_id = i.id AND r.status = 'awaiting_approval'))",
            placeholders(scope.project_ids.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for id in &scope.project_ids {
            query = query.bind(id.to_string());
        }
        let items = query
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
            .iter()
            .map(item_of)
            .collect::<Result<Vec<_>>>()?;
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {RUN_SELECT} FROM run WHERE item_id IN ({}) \
               AND status IN ('queued','running','awaiting_approval')",
            placeholders(items.len()),
        );
        let mut query = sqlx::query(AssertSqlSafe(sql));
        for item in &items {
            query = query.bind(item.id.to_string());
        }
        let runs = query
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
            .iter()
            .map(run_of)
            .collect::<Result<Vec<_>>>()?;
        let steps = if runs.is_empty() {
            Vec::new()
        } else {
            let sql = format!(
                "SELECT {RUN_STEP_SELECT} FROM run_step WHERE run_id IN ({})",
                placeholders(runs.len()),
            );
            let mut query = sqlx::query(AssertSqlSafe(sql));
            for run in &runs {
                query = query.bind(run.id.to_string());
            }
            query
                .fetch_all(&self.pool)
                .await
                .map_err(map_sqlx)?
                .iter()
                .map(run_step_of)
                .collect::<Result<Vec<_>>>()?
        };
        Ok(WaitingCandidate::assemble(scope, items, runs, steps))
    }
```

The mirror carries every `run` and `run_step` row of a project (`cache/refresh.rs:1124-1150`,
`:1205-1218`: filtered by `updated_at`, not by a window), so the mirror answer equals Postgres'
after a pass.

### 2.5 Conformance cases (`crates/htui-core/src/store/conformance.rs`)

Add `WaitingCandidate, WaitingPermission, ChatRunSpec` to the `crate::model::{…}` import where they
are missing. Append to `CASES` after `"hand_written_rows_round_trip",` (`:188`):

```rust
    "waiting_candidates_hold_every_park_and_nothing_else",
    "open_permissions_list_live_pending_item_requests",
```

Add the dispatch arms before the `other =>` panic (`:479`), in the shape of the arms beside them.

Shared helpers go next to `leased_step` (`:6575`). They use `new_run` (`:4554`), `new_run_step`
(`:4569`), `LEASE` (`:4522`), `seam_clock` (`:4517`) and `platform_scope` (`:585`):

```rust
/// A run of `item` created and claimed by `owner` (the run `running`, the item `in_progress`).
async fn claimed<S: WriteStore>(case: &str, store: &S, item: ItemId, owner: Uuid, at: DateTime<Utc>) -> RunId {
    let run = store.create_run(new_run(ids::PROJECT_HTUI, item, Vec::new())).await.expect(case).id;
    assert_eq!(store.claim_run(run, ids::BOX, owner, at, LEASE).await.expect(case), Claim::Admitted,
        "{case}: the claim is admitted (blueprint H-6: one running run at a time)");
    run
}

/// A step of `run` at the slot, moved `pending -> running`.
async fn running_step<S: WriteStore>(case: &str, store: &S, run: RunId, (position, attempt, fanout): (i32, i32, i32), at: DateTime<Utc>) -> StepId {
    let step = store.create_step(new_run_step(run, position, attempt, fanout)).await.expect(case).id;
    assert!(store.transition_step(step, StepStatus::Pending, StepStatus::Running, at).await.expect(case));
    step
}

/// A fresh open item of `htui` (no tags, so `claim_run` admits it).
async fn minted<S: WriteStore>(case: &str, store: &S, title: &str) -> ItemId {
    store.mint_item(new_item(ids::PROJECT_HTUI, ids::KIND_HTUI_FEAT, title)).await.expect(case).id
}

/// `(item, item status, [(run, run status, [(step, step status)])])`: what a case compares, since
/// the rows' timestamps are the stores'.
type CandidateShape = Vec<(ItemId, Status, Vec<(RunId, RunStatus, Vec<(StepId, StepStatus)>)>)>;

fn shape_of(rows: &[WaitingCandidate]) -> CandidateShape { /* map the fields, order kept */ }
```

**Case 1, `waiting_candidates_hold_every_park_and_nothing_else`** (`S: WriteStore`; `owner =
Uuid::now_v7()`, `at = seam_clock()`). Build, in this order (H-6):

| Item (mint order = key order) | Writes | Expected |
|---|---|---|
| `gate` (FEAT-4) | `create_run` → `finish_run(Cancelled, None)` (the item back to `open`, the `views.rs:613-616` precedent); then `claimed` → `running_step((0,1,0))` → `park_step(StepFence::Lease(owner), step)` = `Parked` | item `awaiting_approval`, one run (the **second**: the cancelled one is not carried), one step `awaiting_approval` |
| `selection` (FEAT-5) | `claimed`; steps `(0,1,0)` and `(0,1,1)` each `running → done`; `transition_run(run, Running, AwaitingApproval)`; `transition(item, InProgress, AwaitingApproval)` (`park_selection`'s shape, `engine.rs:4470-4480`) | item `awaiting_approval`; run parked; two steps `done` |
| `judge` (FEAT-6) | as `selection`, plus the judge `(0,1,-1)` `running → awaiting_approval` (`transition_step`), then `answer_gate(judge, Rejected, Some("judge: no verdict"), at)` = `true`, then the run and item moves | three steps, the judge first (`fanout_index -1`) and `failed`. Assert that its `gate_note` reads `Some("judge: no verdict")` and its `gate_outcome` is `Rejected` (`fail_judge`'s rows, `engine.rs:5155-5166`) |
| `escalation` (FEAT-7) | `claimed`; `(0,1,0)` `running → done`; `transition_run(Running, AwaitingApproval)`; `transition(item, InProgress, Blocked)` (`escalate`'s shape, `gate.rs:1050-1051`: no step parked) | item `blocked`; run parked; one step `done` |
| `open under park` (FEAT-8) | as `gate` (no cancelled run), then `transition(item, AwaitingApproval, Open)` (legal, `item.rs:62`) | item `open`; run parked; step `awaiting_approval` |
| `walking` (FEAT-9), the negative | `claimed`; `running_step((0,1,0))`, left running | **absent** (`in_progress`, run `running`) |
| `VULKAN_TOOL_1`, out of scope | `transition(VULKAN_TOOL_1, Open, Blocked)` | absent from Platform; the **only** row of `Scope { workspace_id: ids::WORKSPACE_GRAPHICS, project_ids: vec![ids::PROJECT_VULKAN] }`, with `runs: []` |
| a chat run in `htui` | `ChatRunSpec::mint(PROJECT_HTUI, BOX, USER, Some(AGENT_CLAUDE), Some("sonnet"))` → `start_chat_run` | nothing changes |

Expected `shape_of(platform)`, exactly in this order (H-5: two fixture candidates bracket the
minted ones by key):

1. `HTUI_FEAT_2` `blocked` `[]`
2. gate
3. selection
4. judge
5. escalation
6. open-under-park
7. `HTUI_TOOL_1` `awaiting_approval` `[]`

Also assert:
- `HTUI_FEAT_3` (`RUN_2` queued), `HTUI_FEAT_1` (`RUN_1` done) and FEAT-9 are absent;
- `store.waiting_candidates(&Scope { workspace_id: ids::WORKSPACE_PLATFORM, project_ids: vec![] })`
  is empty.

Name items by the `Item.id`s that `mint_item` returned, never by a literal key, so the case does not
depend on the counter value.

**Case 2, `open_permissions_list_live_pending_item_requests`** (`a`, `b` = `Uuid::now_v7()`).
It uses `open_request` (`:14260`), `parked` (`:14281`), `permission_row` (`:14292`) and the
existing `answer` helper (`~:14300`):

| Row | Writes | Expected |
|---|---|---|
| **live** | `leased_step(CASE, store, a, at)` (HTUI_ANA_2); `parked(open_request(run, step, RelaySessionId::new(), "req-1", a))` | **listed** |
| answered | a second request `"req-2"` on the same step and session; `answer(…)` it | absent |
| expired, then foreign | a minted item; `claimed(…, a)`; `running_step`; `parked(open_request(…, "req-3", a))`; `refresh_lease(run2, a, TimeDelta::zero())` | absent (lapsed). Then `take_lease(run2, BOX, b, minutes(14))` = `true`: still absent (owner mismatch, the `relay_view` case's shape) |
| chat run (A-12) | `start_chat_run(&chat)`; `take_lease(chat.run_id, BOX, a, LEASE)` = `true` (precondition); `parked(open_request(chat.run_id, chat.step_id, …, "req-chat", a))` | absent (`item_id NULL`) |

Expected `store.open_permissions(&platform_scope())` is
`vec![WaitingPermission { item: HTUI_ANA_2, project: PROJECT_HTUI, item_key: "ANA-2", key_prefix:
"ANA", key_number: 2, run_queued_at: run_row(CASE, store, run).await.queued_at, step_position: 0,
step_attempt: 1, step_fanout_index: 0, phase_name: "implement", permission: permission_row(CASE,
store, live).await }]`. Also expect the Graphics scope and the empty scope to answer `vec![]`.

Slots: the live run and run2 are both `running` (2 = the limit, H-6), and the chat run is never
claimed.

**If `take_lease` on the chat run answers `false`** on either backend, the chat fixture is
dropped, and the decisions write-up records why. The item join still excludes chat runs.

### 2.6 Pins, the cache test, `.sqlx`

- `crates/htui-core/tests/mem_store.rs:36`: `135` → `137`. Append to the message: `…, and MOD-69's
  two for the waiting-on-you reads (plan D2, D4)`. `READ_CASES` (`:64-69`) stays at 15.
- `crates/htui-store/tests/pg_conformance.rs`:
  - the doc (`:18-27`) gains "…, and MOD-69's two waiting-list cases (plan D2, D4) make it 137";
  - `EXPECTED_CASES` → `137`;
  - the message (`:35`) → `"(137 since MOD-69's waiting-list cases)"`.
- `crates/htui-store/tests/cache.rs`: add `the_mirror_lists_waiting_candidates_like_postgres`,
  after `the_mirror_counts_tool_calls_like_postgres` (`:555-639`) and in its shape (`demo_db`,
  `open_cache`, `run_pass(.., &settings(&db, 20))`, `teardown`). Steps:
  1. On `db.store`, write a gate park on HTUI_ANA_2 (`create_run` → `claim_run` → `create_step` →
     `transition_step` → `park_step`), a selection park on a minted item (as in case 1), and
     `transition(AGY_FIX_1, Open, Blocked)`.
  2. Run one pass.
  3. Assert that `cache.waiting_candidates(&platform_scope().await)` equals
     `db.store.waiting_candidates(&…)` **as whole rows**, and has 5 entries: FEAT-2, ANA-2, the
     minted item, TOOL-1, and `agy` FIX-1.
- `.sqlx`, sandbox recipe (`docs/hr-sandbox.md:194-209`). `htui_sqlx` may exist already, so ignore
  "already exists":
  ```bash
  cd crates/htui-store
  export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations
  cargo sqlx prepare -- --all-targets --all-features   # never without both flags
  cargo sqlx prepare --check
  git status --porcelain .sqlx                           # exactly four `??`
  ```

### 2.7 Tests (first)

| Test | Where | Runs on |
|---|---|---|
| `waiting_candidates_hold_every_park_and_nothing_else` | `CASES` | Mem (`run_case_accepts_every_name_in_cases`, `mem_store.rs`), Pg (`pg_store_conformance`) |
| `open_permissions_list_live_pending_item_requests` | `CASES` | Mem, Pg |
| `the_mirror_lists_waiting_candidates_like_postgres` | `htui-store/tests/cache.rs` | mirror against Pg |
| the T0 unit tests | `model/waiting.rs` | — |
| the pins | `mem_store.rs`, `pg_conformance.rs` | 137 / 15 / 137 |

### 2.8 Commits (T1)

1. `test(mod-69): waiting-list conformance cases and pins (red)`. This holds:
   - §2.1;
   - every delegation in §2.2 (real code);
   - the Mem/Pg/Cache bodies as `todo!("MOD-69 T1")`;
   - §2.5 and the §2.6 pins.

   Red: the new cases panic, and no product path calls the methods.
2. `feat(mod-69): MemStore lists waiting candidates and open permissions`: the §2.3 helpers and
   bodies. Gate: `cargo test -p htui-core --all-features -- --test-threads=1` green.
3. `feat(mod-69): Postgres and the mirror list waiting candidates`: the §2.4 bodies, the
   `cache/read.rs` extraction, the four `.sqlx` entries (H-16) and the cache test. Gate: the whole
   T1 row of §6.

---

## 3. T2: the classifier (D3, D5, D9)

**Files**: `crates/htui-worker/src/views.rs`, `crates/htui-worker/src/lib.rs`.

### 3.1 Imports (`views.rs:10-20`)

- Add `ProjectId`, `RunStatus`, `Scope`, `StepStatus`, `WaitingCandidate` and `WaitingPermission` to
  `htui_core::model::{…}`.
- Extend `htui_orch::status::{group_at, resumable}` with `judge_at`.
- Add `UnblockCase` to `htui_orch::{…}`.
- Add `chrono::{DateTime, Utc}` (`chrono` is already a dependency, `Cargo.toml:22`).

### 3.2 E2: `unblock_case`, shared with `verdicts`

```rust
/// `u`'s case for `item` over its runs (MOD-69 blueprint E2): the one place both the Runs pane's
/// verdict (`verdicts`, `.map(drop)`) and the waiting list (`waiting`) read it, so the two cannot
/// disagree on whether, or how, an item is unblocked.
///
/// Every active run must decode its snapshot; the first that does not refuses with its sentence,
/// as `verdicts` always has. `runs` is in `ReadStore::runs` order (newest first), which
/// `unblock_enabled` is sensitive to (blueprint H-1).
fn unblock_case(item: &Item, runs: &[(Run, Vec<RunStep>)]) -> Result<UnblockCase, String> {
    let mut active = Vec::new();
    for (run, steps) in runs {
        if !run.status.is_active() {
            continue;
        }
        let snapshot = snapshot_of(run).map_err(|err| err.to_string())?;
        active.push((run.clone(), resumable(&cursor(&snapshot, steps), steps)));
    }
    unblock_enabled(item, &active).map_err(|err| err.to_string())
}
```

In `verdicts` (`:322-444`), delete `let mut active` and `let mut unblock_refusal` (`:353-354`), the
`unblock_refusal.get_or_insert_with` inside the `Err(err)` arm (`:370-372`; keep the
`refused_step` loop and the `continue`), and `active.push(…)` (`:378-380`). Then replace
`:439-442` with `actions.unblock = unblock_case(item, runs).map(drop);`.

This preserves behaviour: an inactive run's undecodable snapshot never affected `u`, and the first
active one's sentence wins either way. Gate: the two existing `unblock_is_enabled_*` tests stay
green.

### 3.3 Types

```rust
/// Why a row of the waiting-on-you list waits on a person (MOD-69 plan D3). Declaration order is
/// plan D9's last sort key, so `Ord` is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WaitingReason {
    /// A step parked at `awaiting_approval`: approve or reject it.
    Gate,
    /// A parked fan-out slot whose judge failed: pick a winner.
    JudgeFailed,
    /// A parked fan-out slot with no judge verdict: pick a winner.
    Selection,
    /// `u` clears the item: reopen it, follow its parked run, or resume the run.
    Unblock,
    /// An open permission request of a live session.
    Permission,
}

impl WaitingReason {
    /// The overlay's reason column.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gate => "gate",
            Self::JudgeFailed => "judge failed",
            Self::Selection => "selection",
            Self::Unblock => "unblock",
            Self::Permission => "permission",
        }
    }
}

/// One row of the waiting-on-you list: ids, strings and counts only, so `TopBarState` keeps `Eq`
/// (plan D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingRow {
    /// The item, which `Enter` reveals.
    pub item: ItemId,
    /// `item.key`.
    pub item_key: String,
    /// The run the row is about; `None` for a Reopen row.
    pub run: Option<RunId>,
    /// The step `Enter` puts the cursor on; `None` focuses the run (or, with no run, nothing).
    pub step: Option<StepId>,
    /// The step as the Runs pane's slot column reads it, after the phase: `prd 0.1`,
    /// `research 0.1/1`, `research 0.1/j`; empty when `step` is `None`.
    pub step_label: String,
    /// Why it waits.
    pub reason: WaitingReason,
    /// The reason's text: a gate or judge note, the tool, or the Unblock case's sentence.
    pub text: String,
}

/// The waiting-on-you list and the top bar's two counts (MOD-69 plan D5, D6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitingView {
    /// Active runs in scope that own no row.
    pub working: usize,
    /// Every row, in plan D9's order.
    pub rows: Vec<WaitingRow>,
    /// `false` offline: permission requests are not mirrored, so none are listed (plan D4).
    pub permissions_known: bool,
}

impl WaitingView {
    /// How many rows wait on a person: the top bar's second count.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.rows.len()
    }
}
```

Text constants are private to `views.rs`, and the tests use them. E5: the wording is M-3's.

```rust
const GATE_TEXT: &str = "gate";
const PROMOTED_TEXT: &str = "promoted to chat";
const SELECTION_TEXT: &str = "awaits selection";
const PERMISSION_TEXT: &str = "permission";
const REOPEN_TEXT: &str = "blocked, no active run: u reopens it";
const FOLLOW_TEXT: &str = "blocked over a parked run: u follows it";
const RESUME_TEXT: &str = "parked by an interrupted command: u resumes it";
```

### 3.4 `waiting`

```rust
/// The waiting-on-you list over one candidate read (MOD-69 plan D1-D5, D9), with the engine's own
/// guards: `verdicts` (empty heads, no live chat; heads feed only approve/accept/open, plan D3)
/// decides a slot's `select`, `unblock_case` the item's `u`, step status a gate. `permissions` is
/// `None` offline. `active` is `Backend::active_runs` over the same scope.
#[must_use]
pub fn waiting(
    scope: &Scope,
    active: usize,
    candidates: &[WaitingCandidate],
    permissions: Option<&[WaitingPermission]>,
) -> WaitingView
```

The algorithm, step by step:

1. For each candidate, call `let verdict = verdicts(&candidate.item, &candidate.runs, &[],
   &LiveChats::default());`. `candidate.runs` is already `[(Run, Vec<RunStep>)]`, the exact input
   type.
2. For each `(run, steps)` in `candidate.runs`:
   1. Let `parked` be the steps with `status == StepStatus::AwaitingApproval`. For each, push a
      **Gate** row with:
      - `run: Some(run.id)`, `step: Some(step.id)`;
      - `step_label: label_of(step, steps)`;
      - `text`: the `gate_note`, non-empty, else `PROMOTED_TEXT` when `promoted_at.is_some()`
        (E6), else `GATE_TEXT`.

      Step status decides this, not `approve`: a gate parked without its output greys `approve`
      but still waits on a person (plan D3).
   2. If `run.status == RunStatus::AwaitingApproval && parked.is_empty()`:
      1. Collect the slots: `BTreeSet<(i32, i32)>` of `(position, attempt)` over the steps with
         `fanout_index >= 0` and
         `verdict.steps.get(&step.id).is_some_and(|actions| actions.select.is_ok())`.
         `verdicts` greys `select` for a run whose snapshot does not decode (`refused_step`), so
         such a run has no slot.
      2. For each slot, match `judge_at(steps, position, attempt)` (`status.rs:231`) filtered to
         `status == StepStatus::Failed && gate_note.is_some()`:
         - `Some(judge)` → a **JudgeFailed** row: `step: Some(judge.id)`,
           `label_of(judge, steps)`, `text = judge.gate_note`. This covers `fail_judge` and the
           sweep's `"interrupted"` judge (plan D3).
         - `None` → if `group_at(steps, position, attempt).first()` is `Some(first)`, a
           **Selection** row: `step: Some(first.id)`, `label_of(first, steps)`,
           `text = SELECTION_TEXT`.
3. Match `unblock_case(&candidate.item, &candidate.runs)`:
   - `Ok(UnblockCase::Reopen)` → an **Unblock** row: `run: None`, `step: None`, `step_label: ""`,
     `REOPEN_TEXT`.
   - `Ok(UnblockCase::FollowRun(id))` → `run: Some(id)`, `FOLLOW_TEXT`.
   - `Ok(UnblockCase::Resume(id))` → `run: Some(id)`, `RESUME_TEXT`.
   - `Err(_)` → no row.

   An undecodable active snapshot answers `Err` (H-2), so that item gets no Unblock row, while its
   gate rows from step 2.1 stay.
4. If `permissions` is `Some(rows)`, push one **Permission** row per row:
   - `run: Some(p.permission.run_id)`, `step: Some(p.permission.run_step_id)`;
   - `step_label: slot_label(&p.phase_name, p.step_position, p.step_attempt, p.step_fanout_index,
     p.step_fanout_index != 0)`;
   - `text`: the non-empty `summary` (`"<tool_kind>: <title>"`), else `PERMISSION_TEXT`.
5. Sort by the private key below, then drop the keys:

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
   struct RowKey {
       project: usize,               // scope.project_ids position; usize::MAX when absent
       key_prefix: String,           // byte order (String's Ord)
       key_number: i32,
       item: ItemId,
       run_missing: bool,            // false first: rows with a run before the item's Reopen row
       run: Option<(DateTime<Utc>, RunId)>, // (queued_at, id) ascending: plan D9's run creation
       step_missing: bool,           // false first: a run's step rows before its Unblock row
       step: Option<(i32, i32, i32)>,       // (position, attempt, fanout_index)
       reason: WaitingReason,        // D3 order
       text: String,                 // last tie-break: two permissions on one step
   }
   ```

   Candidate rows take the item's `project_id`, `key_prefix` and `key_number`. The run's
   `queued_at` comes from `candidate.runs`, looked up by id for FollowRun and Resume. Permission
   rows take them from the `WaitingPermission`. E8 defines the order for rows with no run or no
   step, which D9 leaves open.
6. `working = active.saturating_sub(rows.iter().filter_map(|row| row.run).collect::<BTreeSet<_>>().len())`
   (D5, H-19), and `permissions_known = permissions.is_some()`.

Label helpers. They mirror `runs.rs:1053-1066` (`slot`), with the phase in front:

```rust
/// `phase p.a`, then `/i` in a fan-out slot and `/j` for its judge (the Runs pane's slot column).
fn slot_label(phase: &str, position: i32, attempt: i32, fanout_index: i32, fanned: bool) -> String {
    let at = format!("{phase} {position}.{attempt}");
    match (fanned, fanout_index) {
        (false, _) => at,
        (true, -1) => format!("{at}/j"),
        (true, index) => format!("{at}/{index}"),
    }
}

/// [`slot_label`] of a step among its run's steps: a slot is fanned when a sibling at the same
/// `(position, attempt)` has a non-zero `fanout_index`.
fn label_of(step: &RunStep, steps: &[RunStep]) -> String {
    let fanned = steps.iter().any(|sibling| {
        sibling.position == step.position && sibling.attempt == step.attempt && sibling.fanout_index != 0
    });
    slot_label(&step.phase_name, step.position, step.attempt, step.fanout_index, fanned)
}
```

A permission's siblings are not read, so its slot counts as fanned exactly when its own
`fanout_index != 0`. A fanned slot's index-0 permission therefore reads `prd 0.1`, not
`prd 0.1/0`. The decisions write-up records this.

### 3.5 `lib.rs`

Change `:56-59` to
`pub use views::{Enabled, FrameKind, ItemActions, LiveChats, ORCH_NAMES, OrchReply, OrchRequest,
ProgressSink, RunActions, RunFrame, StepActions, StepAuthor, Via, WaitingReason, WaitingRow,
WaitingView, actions, waiting};`. `mod views` is private (`lib.rs:43`, plan claim 24).

### 3.6 Tests (first): row fixtures, no store

Add these builders to `views.rs` `mod tests` (`:556`). They take demo rows as templates (the
`status.rs` tests' `steps_of`/`snapshot` precedent, `status.rs:470-490`). The `htui-core/demo`
feature is already on in this crate's dev build, because `htui_core::fixtures::ids` is used at
`:558`.

```rust
/// The demo `feature` snapshot (RUN_1's) with phase 0's `fan_out` set: 1 = no slot, 2 = a slot.
fn snapshot(fan_out: i32) -> serde_json::Value {
    let mut graph: GraphSnapshot = serde_json::from_value(
        demo_data().runs.into_iter().find(|run| run.id == ids::RUN_1)
            .and_then(|run| run.graph_snapshot).expect("RUN_1 carries a snapshot"),
    ).expect("the demo snapshot decodes");
    graph.phases[0].fan_out = fan_out;
    serde_json::to_value(graph).expect("it encodes")
}
fn item(prefix: &str, number: i32, project: ProjectId, status: Status) -> Item { /* demo FEAT-2 row,
    id = ItemId::from_uuid(Uuid::from_u128(..)), key = format!("{prefix}-{number}") */ }
fn run(n: u128, item: &Item, status: RunStatus, snapshot: Option<Value>, hour: i64) -> Run { /* demo
    RUN_2 row: id from n, item_id, project_id = item.project_id, queued_at = demo_at(2, hour) */ }
fn step(n: u128, run: &Run, (position, attempt, fanout): (i32, i32, i32), status: StepStatus) -> RunStep
    { /* demo STEP_R2_PRD row: id from n, run_id, phase_name = the demo phase at `position`
    (prd, plan, implement, review) */ }
fn scope(projects: &[ProjectId]) -> Scope
fn permission(item: &Item, run: &Run, step: &RunStep, summary: Option<&str>, created_minute: i64) -> WaitingPermission
```

These need `Uuid` (`uuid` is a dependency), `GraphSnapshot`, `Value`, `demo_data` and `demo_at`.
Check that `demo_at` is `pub` in `fixtures`. If it is not, use `Utc.with_ymd_and_hms`.

| Test | Fixture | Pins |
|---|---|---|
| `a_parked_gate_without_output_is_one_gate_row` | item `awaiting_approval`; run `awaiting_approval`, `snapshot(1)`; step `(0,1,0)` `awaiting_approval`, `gate_note: None`; `active = 2` | one row: `Gate`, `step_label "prd 0.1"`, text `"gate"`. `working == 1`. **Parity**: `verdicts(..).steps[step].approve.is_err()` (no output) while the row exists, so gates go by step status (plan D3) |
| `a_judge_failed_slot_is_a_judge_row_not_a_selection_row` | `snapshot(2)`; run parked; candidates `(0,1,0)`, `(0,1,1)` `done`; judge `(0,1,-1)` `failed`, `gate_outcome Rejected`, `gate_note "judge: tie"` | exactly one row: `JudgeFailed`, the judge's step, `"prd 0.1/j"`, text `"judge: tie"` |
| `an_interrupted_judge_is_a_judge_row` | the same, with the judge's `gate_outcome: None`, `gate_note "interrupted"` | one `JudgeFailed` row, text `"interrupted"` |
| `an_unjudged_parked_slot_is_a_selection_row_on_its_first_candidate` | `snapshot(2)`; run parked; two candidates `done`; no judge | one `Selection` row on candidate 0, `"prd 0.1/0"`, `"awaits selection"`. **Parity**: some `verdicts(..).steps[c].select.is_ok()` |
| `a_resolved_selection_is_no_row` | `snapshot(2)`; run parked; candidate 0 `done` `selected Some(true)`, candidate 1 `superseded` `Some(false)`; a gate at position 1 `awaiting_approval` | only the position-1 `Gate` row |
| `an_escalation_is_one_follow_run_row_and_no_gate_row` | item `blocked`; run parked, `snapshot(1)`; `(0,1,0)` `done`, `(1,1,0)` `failed` + `Rejected` | one `Unblock` row: `run Some`, `step None`, `FOLLOW_TEXT`. **Parity**: `verdicts(..).unblock == Ok(())` |
| `a_person_blocked_item_over_a_parked_gate_is_a_gate_row_and_an_unblock_row` | item `blocked`; run parked; `(0,1,0)` `awaiting_approval` | `[Gate, Unblock(FollowRun)]` in that order (same run; E8) |
| `an_open_item_under_a_parked_gate_is_a_gate_row_only` | item `open`; run parked; `(0,1,0)` `awaiting_approval` | one `Gate` row. **Parity**: `verdicts(..).unblock.is_err()` |
| `a_blocked_item_with_no_run_is_a_reopen_row` | item `blocked`; `runs: []` | one `Unblock` row: `run None`, `step None`, `REOPEN_TEXT`, `step_label ""` |
| `a_crashed_rejection_is_a_resume_row` | item `awaiting_approval`; run parked; `(0,1,0)` `failed` + `Rejected` | one `Unblock` row, `RESUME_TEXT` |
| `a_snapshot_that_does_not_decode_keeps_its_gate_row_and_loses_the_rest` | item `blocked`; run parked with `graph_snapshot: Some(json!({ "v": 999 }))`; `(0,1,0)` `awaiting_approval` | one `Gate` row, no `Unblock` (H-2). **Parity**: `verdicts(..).unblock.is_err()` |
| `permissions_are_rows_online_and_unknown_offline` | a running run on an `in_progress` item (not a candidate) with one `WaitingPermission`, summary `Some("edit: src/main.rs")` and one `None` | `Some(&perms)`: two `Permission` rows, texts `"edit: src/main.rs"` and `"permission"`, `permissions_known`. `None`: no rows, `!permissions_known` |
| `counts_split_working_from_waiting_and_never_count_a_run_twice` | `active = 5`. Reopen item (no run). Person-blocked item over a parked gate on run R1 (2 rows). A permission on running run R2 | `waiting() == 4`, `working == 5 - 2 == 3`. A second call with `active = 1` gives `working == 0` (saturating) |
| `rows_sort_by_project_key_run_step_then_reason` | scope `[PROJECT_HTUI, PROJECT_AGY]`. `agy` FEAT-1: a gate. `htui` FEAT-10: Reopen. `htui` FEAT-2: two parked runs, an older one (hour 1) with a gate at position 1 and a newer one (hour 2) with a gate at position 0, plus a permission on that position-0 step. `htui` ANA-1: one permission, not a candidate | keys in order: ANA-1 permission; FEAT-2 old-run gate (pos 1); FEAT-2 new-run gate (pos 0); FEAT-2 new-run permission (pos 0); FEAT-10 Reopen; agy FEAT-1 gate |
| `unblock_case_is_verdicts_unblock_with_the_case_kept` | the escalation, crashed-rejection, Reopen and open-under-park fixtures | `unblock_case(..).map(drop) == verdicts(..).unblock` for each (E2) |

### 3.7 Commits (T2)

1. `refactor(mod-69): unblock_case shared by verdicts` (§3.2). Gate: `cargo test -p htui-worker
   --all-features -- --test-threads=1` green, with no test edited.
2. `test(mod-69): waiting-list classifier fixtures (red)`: §3.3, `waiting` as
   `todo!("MOD-69 T2")`, §3.5, §3.6. Red: the new tests panic, and nothing in the product calls
   `waiting` yet.
3. `feat(mod-69): classify the waiting-on-you list`: §3.4 and the label helpers. Gate: the T2 row
   of §6.

---

## 4. T5: reveal to step (D8)

### 4.1 `RevealTarget::Step` (`crates/htui/src/app/action.rs:98-133`)

`RunId` and `StepId` are already imported (`:6`).

```rust
    /// MOD-69 plan D8: an item, opened on its Runs sub-tab with the cursor on `step` (or on
    /// `run`'s first entry when `step` is `None`; with neither, on whatever the pane selects).
    Step {
        /// The item.
        item: ItemId,
        /// Its key, for the miss sentence (blueprint D248).
        key: String,
        /// The run, when the row has one.
        run: Option<RunId>,
        /// The step, when the row has one.
        step: Option<StepId>,
    },
```

- `kind()`: `Self::Item { .. } | Self::Step { .. } => RevealKind::Item`.
- `key()`: `Self::Item { key, .. } | Self::Step { key, .. } | Self::Requirement { key, .. } => key`.
- Leave the Requirements tab's let-else (`requirements/mod.rs:1186`) alone.

### 4.2 `DetailTab::focus` and `DetailRegistry::focus` (`backlog/detail/mod.rs`)

Imports: `htui_core::model::{ItemId, RunId, StepId}`. Trait, after `on_mouse` (`:110-113`):

```rust
    /// MOD-69 plan D8 (blueprint A-5): a reveal asks for this run and step under the cursor. Only
    /// [`RunsTab`] answers; the default ignores it. Called after the item change, so it survives
    /// `on_item_change`'s reset.
    fn focus(&mut self, _run: Option<RunId>, _step: Option<StepId>, _ctx: &Ctx<'_>) {}
```

Registry, after `select_id` (`:187-192`):

```rust
    /// Hands a reveal's run and step to the sub-tab registered under `id` (MOD-69 plan D8).
    /// `false` when nothing is registered under it.
    pub fn focus(&mut self, id: DetailId, run: Option<RunId>, step: Option<StepId>, ctx: &Ctx<'_>) -> bool {
        match self.tabs.iter_mut().find(|tab| tab.id() == id) {
            Some(tab) => {
                tab.focus(run, step, ctx);
                true
            }
            None => false,
        }
    }
```

### 4.3 The Backlog (`backlog/mod.rs`)

Add a named struct in place of the tuple (H-4):

```rust
/// A reveal waiting for the next `Items` reply (MOD-64 D235), with the key its miss is reported by
/// and, for `RevealTarget::Step`, the run and step the Runs pane is to focus (MOD-69 plan D8).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingReveal {
    id: ItemId,
    key: String,
    focus: Option<(Option<RunId>, Option<StepId>)>,
}
```

Then:
- the field (`:88-90`): `pending_reveal: Option<PendingReveal>`;
- the `apply`, `on_scope_change` and `Failed` sites set `None` and do not change;
- add `RunId, StepId` to the `htui_core::model` import (`:33`).

Add a helper beside `select_item` (`:284-288`):

```rust
    /// MOD-69 plan D8: opens the Runs sub-tab and hands it the reveal's run and step. The item is
    /// already selected; `was_selected` says `go` short-circuited (`:189`), so no `Runs` read went
    /// out and one is asked here for the pane to apply the focus to (blueprint A-5).
    fn focus_runs(&mut self, id: ItemId, run: Option<RunId>, step: Option<StepId>, was_selected: bool, ctx: &Ctx<'_>) {
        self.detail.select_id(RunsTab::ID);
        if run.is_none() && step.is_none() {
            return; // a Reopen row: the item and its Runs pane are the target
        }
        self.detail.focus(RunsTab::ID, run, step, ctx);
        if was_selected {
            ctx.request(StoreRequest::Runs(id));
        }
    }
```

`reveal` (`:786-813`) becomes:

```rust
    fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
        let (id, key, focus) = match target {
            RevealTarget::Item { id, key } => (*id, key, None),
            RevealTarget::Step { item, key, run, step } => (*item, key, Some((*run, *step))),
            RevealTarget::Requirement { .. } => return false,
        };
        // (the half-typed-field guard, unchanged)
        match self.items.iter().find(|item| item.id == id) {
            Some(item) => {
                let project = item.project_id;
                let was_selected = self.selected == Some(Selection::Item(id));
                self.pending_reveal = None;
                self.select_item(id, project, ctx);
                if let Some((run, step)) = focus {
                    self.focus_runs(id, run, step, was_selected, ctx);
                }
            }
            None => {
                self.filter = BacklogFilter::default();
                self.pending_reveal = Some(PendingReveal { id, key: key.clone(), focus });
                ctx.request(self.filter.to_request(ctx.scope));
            }
        }
        true
    }
```

Change the `Items` arm (`:683-689`) to destructure `PendingReveal { id, key, focus }`. On a hit:
compute `let was_selected = self.selected == Some(Selection::Item(id));`, call
`self.select_item(id, item.project_id, ctx)`, then
`if let Some((run, step)) = focus { self.focus_runs(id, run, step, was_selected, ctx) }`. The miss
sentence is unchanged.

**Ordering** (plan D8): `select_item` → `go` → `detail.on_item_change` clears the pane
(`runs.rs:1270-1281`). Only **then** is the focus armed, so the reset cannot eat it.
`on_item_change` does not reset the active sub-tab (`detail/mod.rs:220-224`), so `select_id`
survives.

### 4.4 The RunsTab pending focus (`backlog/detail/runs.rs`)

Add a field to `RunsTab` (`:179-235`):

```rust
    /// MOD-69 plan D8: a reveal's run and step, applied by the next `Runs` reply (or at once by
    /// `focus` when the loaded rows hold it). An item change disarms it.
    pending_focus: Option<(Option<RunId>, Option<StepId>)>,
```

Add the methods, beside `select_step` (`:400-413`):

```rust
    /// The entry a reveal targets (MOD-69 plan D8): the step's entry in **any** run (`select_step`
    /// searches only the cursor's run), else — `step` being `None` — the run's first entry. `None`
    /// when the rows do not hold it: a `step` the rows lack does not fall back to its run.
    fn focus_index(&self, run: Option<RunId>, step: Option<StepId>) -> Option<usize> {
        let entries = self.entries();
        if let Some(step) = step {
            return entries.iter().position(|entry| matches!(entry, Entry::Step { step: s, .. } if *s == step));
        }
        let index = self.runs.iter().position(|summary| Some(summary.id) == run)?;
        entries.iter().position(|entry| matches!(entry, Entry::Step { run: r, .. } | Entry::Run { run: r } if *r == index))
    }

    /// Applies the pending focus to the rows on hand. `true` = it landed, and is disarmed.
    fn apply_focus(&mut self) -> bool {
        let Some((run, step)) = self.pending_focus else { return false };
        match self.focus_index(run, step) {
            Some(at) => {
                self.selected = Some(at);
                self.pending_focus = None;
                true
            }
            None => false,
        }
    }
```

`impl DetailTab for RunsTab`:

```rust
    fn focus(&mut self, run: Option<RunId>, step: Option<StepId>, ctx: &Ctx<'_>) {
        self.pending_focus = Some((run, step));
        // The item was already selected (`go` short-circuited): the rows are loaded, and a target
        // they hold lands now; the Backlog's `Runs` re-read covers one they do not (blueprint A-5).
        if self.apply_focus() {
            self.sync_graph(ctx.theme);
        }
    }
```

Two further changes:
- In `on_item_change` (`:1270-1281`), add `self.pending_focus = None;`.
- In `on_runs` (`:446-472`), between the `self.selected = kept…;` statement and
  `self.sync_graph(ctx.theme)`, add:

  ```rust
          // MOD-69 plan D8: a reveal's focus beats the kept cursor; one the reply does not hold is
          // dropped silently and the cursor stays where D198 put it.
          if self.pending_focus.is_some() && !self.apply_focus() {
              tracing::debug!("the revealed step is not in the runs reply");
              self.pending_focus = None;
          }
  ```

`sync_graph` then runs after it, so the flow view follows.

### 4.5 Unit tests (first)

**`runs.rs`**: use the module's existing pane shell (the one
`the_first_runs_reply_subscribes_once_and_asks_for_the_actions`, `:3430`, builds), with
`RunSummary` rows of two runs:

- `a_focus_armed_before_the_runs_reply_moves_the_cursor_to_the_step`: `on_item_change(Some(item))`,
  `focus(None, Some(s3))`, then the `Runs` reply: `selected_step() == Some(s3)`.
- `a_focus_on_loaded_runs_applies_at_once`: reply first, then `focus`: it moves at once and
  emits nothing.
- `a_focus_finds_a_step_in_a_run_other_than_the_cursors`: the cursor is on run 0, and the target
  is a step of run 1.
- `a_focus_with_no_step_lands_on_the_runs_first_entry`, including a run with no steps (`Entry::Run`).
- `a_focus_the_reply_does_not_hold_leaves_the_cursor_and_says_nothing`: no `Action::Error`, the
  cursor on D198's entry, and the focus disarmed (a second `Runs` reply does not move it).
- `an_item_change_disarms_a_pending_focus`.

**`backlog/mod.rs`**: use `Bench` (`:1550` precedent):

- `revealing_a_step_of_a_loaded_item_selects_it_opens_runs_and_reads_once`: `detail.active_id() ==
  Some(RunsTab::ID)`; exactly one `StoreRequest::Runs(id)` among the actions (from `go`).
- `revealing_a_step_of_the_selected_item_re_reads_its_runs`: the item is preselected, so `go`
  short-circuits. `focus_runs` asks for `Runs(id)`.
- `revealing_a_step_of_an_unloaded_item_carries_the_focus_to_the_items_reply`: `pending_reveal ==
  Some(PendingReveal { focus: Some((Some(run), Some(step))), .. })`. After the `Items` reply: the
  item is selected, Runs is active, and one `Runs(id)` was requested.
- `a_step_reveal_while_a_field_is_open_asks_to_close_it_first`: `CLOSE_THE_FIELD_FIRST`.
- `a_step_reveal_with_no_run_opens_runs_without_asking_twice`: `run: None, step: None`.
- **Edit** `an_applied_mint_closes_the_form_clears_the_filter_and_reveals_it` (`:2358`) to the
  struct form (H-4).

**`action.rs`**: add `a_step_target_routes_as_an_item_and_keeps_its_key`.

### 4.6 End-to-end (`crates/htui/tests/reveal.rs`, A-7)

These use the file's own `platform()`, `platform_unserved()`, `reveal()` and `titled()`:

- `revealing_a_step_lands_on_the_runs_pane_with_that_step_selected`: `RevealTarget::Step { item:
  ids::HTUI_FEAT_1, key: "FEAT-1", run: Some(ids::RUN_1), step: Some(ids::STEP_REVIEW) }`. The
  frame holds `titled("FEAT-1")`, the Runs strip is active, and the line holding `▸` (the pane's
  `CURSOR`, `runs.rs:164`) holds `review`, not `prd`.
- `revealing_a_step_of_the_already_selected_item_moves_the_cursor`: select FEAT-1 first (Runs
  cursor on `prd`), then reveal `STEP_IMPL`: the `▸` line holds `implement`.
- `revealing_a_step_before_the_list_lands_selects_it_on_arrival`: `platform_unserved()`, then the
  reveal: the same assertion after `drive_to_end`.

### 4.7 Commits (T5)

1. `feat(mod-69): RevealTarget::Step and the Runs pane's pending focus`: §4.1, §4.2, §4.4, and the
   `runs.rs` and `action.rs` tests. Gate: `cargo test -p htui --all-features --lib
   tabs::backlog::detail::runs -- --test-threads=1`.
2. `feat(mod-69): the Backlog reveals a step`: §4.3, the Backlog tests, the `:2358` edit and §4.6.
   Gate: the T5 row of §6.

---

## 5. T3: TUI wiring, top bar, overlay, one snapshot pass (D5–D7)

### 5.1 The store request (`crates/htui/src/store_worker.rs`)

- Add `use htui_worker::WaitingView;`. The worker path is direct: `run_worker.rs` stays untouched.
- **Remove** `StoreRequest::ActiveRuns { scope }` (`:131-136`) and add, in the same place:
  ```rust
      /// MOD-69 plan D6: the waiting-on-you list and the top bar's two counts, re-read on the
      /// shell's refresh tick and on a scope change.
      Waiting {
          /// The workspace scope to read.
          scope: Scope,
      },
  ```
- `name()` (`:1014`): `Self::Waiting { .. } => "waiting",`.
- **Remove** `StoreReply::ActiveRuns(usize)` (`:1152-1153`) and add
  `/// Answer to [`StoreRequest::Waiting`]. Waiting(WaitingView),`. It is not boxed: it is three
  words.
- `try_serve` (`:1709-1711`):
  ```rust
          // MOD-69 plan D1, D4, D6: one candidate read, plus the open permissions online; the
          // classification is the engine's own guards (`htui_worker::waiting`). An `Unreachable`
          // from either read propagates, so `go_offline` still drops the backend (blueprint H-12).
          StoreRequest::Waiting { scope } => {
              let active = backend.active_runs(scope).await?;
              let candidates = backend.waiting_candidates(scope).await?;
              let permissions = match backend.writer() {
                  Some(writer) => Some(WriteStore::open_permissions(&writer, scope).await?),
                  None => None,
              };
              StoreReply::Waiting(htui_worker::waiting(
                  scope,
                  active,
                  &candidates,
                  permissions.as_deref(),
              ))
          }
  ```
  `Backend::active_runs` stays (plan D6). `ReadStore` and `WriteStore` are already imported
  (`:35-38`).

### 5.2 The shell (`app/state.rs`, `app/update.rs`)

- `TopBarState` (`state.rs:32-43`): replace `active_runs` with
  ```rust
      /// MOD-69 plan D6: the last `Waiting` reply, which the top bar counts and the overlay lists;
      /// `None` until the first one (blueprint A-6).
      pub waiting: Option<WaitingView>,
  ```
  It keeps its five derives. `WaitingView` is `Eq` (A-9). Edit the module doc's "`N runs`".
- `on_tick` (`update.rs:126-137`): change `StoreRequest::ActiveRuns { scope }` to
  `StoreRequest::Waiting { scope }`. Doc: "…re-reads the store state and the waiting list…".
- `set_scope` (`:176-189`): add `self.top_bar.waiting = None;` before the dispatch (E9, H-10), then
  dispatch `StoreRequest::Waiting { scope }`.
- `observe_reply` (`:363`): `StoreReply::Waiting(view) => self.top_bar.waiting = Some(view.clone()),`.
- `TICKS_PER_REFRESH` doc (`:18`): "the top bar's counts".

### 5.3 The top bar (`ui/top_bar.rs`)

Module doc: ``//! The one-line header: `workspace · box · store · N working · M waiting` (`R-TUI-1`,
`R-TUI-11`).`` Replace the `runs` binding and the last span:

```rust
    let (working, waiting) = state
        .waiting
        .as_ref()
        .map_or((0, 0), |view| (view.working, view.waiting()));
    // Plan T3.3: the waiting part in `accent` when a person is owed something (`Theme` has no
    // warning style); no plural rule — "1 working · 1 waiting" reads as written.
    let waiting_style = if waiting > 0 { theme.accent } else { theme.base };
    // … the first five spans unchanged, then:
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{working} working"), theme.base),
        Span::styled(SEP, theme.dim),
        Span::styled(format!("{waiting} waiting"), waiting_style),
```

The exact text is `<workspace> · <box> · <store> · <N> working · <M> waiting`, the same for every
N and M.

### 5.4 The overlay (`ui/overlay/waiting_list.rs`, new)

```rust
//! The waiting-on-you list (MOD-69 plan D6, D7; `R-TUI-11`): every row of the shell's last
//! `Waiting` reply, read straight from `ctx.top_bar.waiting`, so the list holds nothing but its
//! cursor and refreshes on every tick with no request of its own. `Enter` reveals the row's step;
//! the Runs pane stays the one place that answers (PRD scope).

/// Lists what waits on a person and reveals the selected row's step.
#[derive(Debug, Default)]
pub struct WaitingList {
    /// The highlighted row, stored unclamped; every read clamps it to the rows on hand (H-3).
    cursor: usize,
}

impl WaitingList {
    /// Identity of the list: the factory and the global `Ctrl+W` binding are registered under it.
    pub const ID: OverlayId = OverlayId("waiting_list");
    /// A list with its cursor on the first row.
    #[must_use]
    pub fn new() -> Self { Self::default() }
    /// The cursor clamped to `len` rows; `None` while there are none.
    fn at(&self, len: usize) -> Option<usize> { len.checked_sub(1).map(|last| self.cursor.min(last)) }
}
```

Constants, in the switcher's shape (`workspace_switcher.rs:25-38`):

```rust
const CURSOR: &str = "> ";
const NO_CURSOR: &str = "  ";
const GAP: &str = "  ";
const CHROME: u16 = 3;
const HINT: &str = "j/k move · Enter open · Esc close";
const EMPTY: &str = "nothing is waiting on you";
const READING: &str = "reading the store";
const OFFLINE: &str = "permissions unavailable offline";
const NO_STEP: &str = "\u{2014}"; // the Runs pane's `PENDING` dash
```

`impl Overlay`:
- `id()` → `Self::ID`;
- `title()` → `"Waiting on you"`;
- `is_modal()` → `true`;
- `wants_requests` → `Vec::new()` (plan D6: no request of its own);
- `on_reply` ignores every reply (the top bar already took it).

`on_key`:

| Key | Effect |
|---|---|
| `j` / `Down` | `self.cursor = at(len).map_or(0, \|i\| (i + 1).min(len - 1))`; `Consumed` |
| `k` / `Up` | `self.cursor = at(len).map_or(0, \|i\| i.saturating_sub(1))`; `Consumed` |
| `Enter` | If `at(len)` is `Some(i)`: `ctx.emit(Action::Overlay(OverlayAction::Close))`, **then** `ctx.emit(Action::Reveal(RevealTarget::Step { item: row.item, key: row.item_key.clone(), run: row.run, step: row.step }))`, the order `ConceptsSearch` uses (`concepts_search.rs:215-216`). `Consumed` either way. |
| anything else | `Pass` (`Esc` → the wildcard close; modal swallows the rest) |

`len` is `ctx.top_bar.waiting.as_ref().map_or(0, |view| view.rows.len())`.

`render(&self, frame, area, ctx)`: the switcher's layout (`workspace_switcher.rs:190-213`), with the
box width capped at `area.width.saturating_sub(4)` and the height capped at
`area.height.saturating_sub(2)`. Lines, in order:
1. `None` → `"{NO_CURSOR}{READING}"` (dim). No rows → `"{NO_CURSOR}{EMPTY}"` (dim). Otherwise one
   line per **visible** row: `{marker}{key}{GAP}{step}{GAP}{label}{GAP}{text}`. Key, step and label
   are padded with `cells::fit` to their column's widest value, where the step column shows
   `step_label` or `NO_STEP` and the label is `reason.label()`. The text is `cells::fit` to the
   width that remains, so it is clipped and never wrapped. The selected row is `theme.accent`, the
   others `theme.base`.
2. When `view.permissions_known` is `false`: `"{NO_CURSOR}{OFFLINE}"` (dim).
3. A blank line, then `"{NO_CURSOR}{HINT}"` (dim).

Scroll: `visible = box_height - 2 - fixed_lines`. The window starts at
`at.saturating_sub(visible - 1)` when `at >= visible`, else at 0, so the cursor is always drawn.
Every width is measured with `cell_width` (the MOD-60 rule, `workspace_switcher.rs:236-249`).

**`ui/overlay/mod.rs`**: `pub mod waiting_list;` and `pub use waiting_list::WaitingList;`.

### 5.5 The key (`app/mod.rs`, `settings/boxes.rs`)

`register_all` (`app/mod.rs:58`). After the `Ctrl+F` block (`:88-96`), add:

```rust
    // MOD-69 D7: the waiting-on-you list, global `Ctrl+W`. A chord, for `Ctrl+F`'s reason; MOD-67
    // makes it a named action.
    app.overlay_factories
        .register(WaitingList::ID, || Box::new(WaitingList::new()));
    app.keymap.bind(Binding {
        scope: KeyScope::Global,
        key: KeyChord::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        action: Action::Overlay(OverlayAction::Open(WaitingList::ID)),
        help: "waiting",
    });
```

Also:
- add `WaitingList` to the overlay import (`:12`);
- add item **11** to the doc list (`:25-53`): "The waiting list's factory goes in and global `Ctrl+W`
  is bound to opening it (MOD-69 D7)".

`boxes.rs:711`: guard **only** the `'w'` arm (H-14):

```rust
            // MOD-69 D7: `Ctrl+W` is the global waiting list, not this section's executor.
            KeyCode::Char('w')
                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && self.unavailable.is_none()
                    && self.selected_record().is_some() =>
```

### 5.6 Tests (first)

**`store_worker.rs`**:
- Replace `serve_active_runs_counts_the_queued_run` (`:3159-3168`) with
  `serve_waiting_reads_the_demo_platform`, over `demo()` + `platform_scope`. Assert
  `StoreReply::Waiting(view)` with:
  - `view.working == 1` (RUN_2) and `view.permissions_known` (a `Memory` backend has a writer);
  - `view.rows == vec![WaitingRow { item: ids::HTUI_FEAT_2, item_key: "FEAT-2".into(), run: None,
    step: None, step_label: String::new(), reason: WaitingReason::Unblock, text: <REOPEN_TEXT
    value> }]` (H-5);
  - `StoreRequest::Waiting { scope }.name() == "waiting"`.
- Add `serve_waiting_offline_leaves_permissions_unknown`, a copy of
  `relay_reads_are_empty_offline_and_answers_are_refused`'s set-up (`:4843-4856`): `mock_keyring`,
  an empty `CacheStore`, `Backend::Offline`. `try_serve(Waiting { scope: <any non-empty scope> })`
  is `Ok(Waiting(view))` with `!view.permissions_known`, `rows` empty and `working == 0`.

**`update.rs`**:
- Port `a_scope_change_closes_every_overlay_and_re_reads_the_run_count` (`:661-671`) to
  `…_re_reads_the_waiting_list` (`matches!(…, StoreRequest::Waiting { .. })`). Also assert
  `app.top_bar.waiting.is_none()` right after `SetScope` (E9).
- Port `a_tick_only_asks_for_the_top_bar_once_a_second` (`:725-741`): `requests[1]` is
  `StoreRequest::Waiting { .. }`.
- Add `a_waiting_reply_sets_the_top_bar_whatever_its_origin`: a `StoreReply::Waiting(view)` at
  `UNSOLICITED` from `Origin::Overlay(WaitingList::ID)` → `top_bar.waiting == Some(view)` (plan D6:
  `observe_reply` runs before the gate).

**`waiting_list.rs`** `mod tests`: build a `Ctx` by hand (the `backlog/mod.rs:978-997` shape) over a
`TopBarState { waiting: Some(view), .. }`.
- `the_cursor_clamps_when_rows_shrink`: cursor at 2 of 3 rows; the view shrinks to 1 row; `at(1) ==
  Some(0)`; `Enter` reveals row 0; `k` writes back 0.
- `enter_closes_then_reveals_the_rows_step`: the exact two actions, in order.
- `enter_on_an_empty_list_emits_nothing`.
- `a_row_reads_key_step_reason_and_text`: `lines()` text equality for a gate row, a Reopen row
  (`—` in the step column) and a permission row.
- `before_the_first_reply_it_says_reading`.
- `offline_it_says_permissions_are_unavailable`: an insta snapshot of a 100x30 `TestBackend` render
  with two rows and `permissions_known: false`. This is the one new `src` snapshot,
  `crates/htui/src/ui/overlay/snapshots/htui__ui__overlay__waiting_list__tests__offline.snap`.

**`top_bar.rs`**: add `the_waiting_count_is_accented_only_when_non_zero`. Render into a `Buffer`,
then check the text and the style of the `waiting` cell for `Some(view)` with 0 rows and with 2
rows.

**`boxes.rs`**: add `ctrl_w_over_a_listed_box_passes_and_w_still_opens_the_executor`. Use the
section's own bench (`SectionBench`), as its other key tests do.

**`crates/htui/tests/waiting.rs`** (new, `#![cfg(feature = "testkit")]`): use `Harness::over(store.clone())`
over `MemStore::demo()` with `register_all`, then move to Platform the `tests/reveal.rs` way
(`platform()`). The helper `park_ana_2(&store)` runs `create_run` → `claim_run` → `create_step` →
`transition_step` → `park_step` on HTUI_ANA_2. Its snapshot is the demo `RUN_1` snapshot, decoded,
so the classifier reads the `feature` phases. Optionally, `open_permission` adds a request
(`summary "edit: src/lib.rs"`) under the claim's owner.

| Test | Pins |
|---|---|
| `the_top_bar_counts_working_and_waiting` | Graphics `… · memory · 0 working · 0 waiting`; Platform `… · 1 working · 1 waiting` |
| `a_park_shows_on_the_next_refresh_tick` | park, then 4 × `Action::Tick`, `settle`: `1 working · 2 waiting` (active 2 minus the parked run; rows ANA-2 gate and FEAT-2 Reopen) |
| `ctrl_w_opens_the_list_from_every_tab_and_esc_closes_it` | for each of `1`..`5`: `ctrl-w` → top overlay id `WaitingList::ID`; `esc` closes it |
| `the_open_list_follows_a_park_without_reopening` | `ctrl-w` (render shows FEAT-2's row), park, tick ×4, settle: the same overlay renders the ANA-2 row too |
| `enter_opens_the_rows_step_in_the_runs_pane` | `ctrl-w`, `enter` on ANA-2's gate row: no overlay, Backlog active, `titled("ANA-2")`, and the `▸` line holds `prd` |
| snapshot `waiting__graphics_empty` | the overlay over Graphics: `nothing is waiting on you` |
| snapshot `waiting__platform_mixed` | Platform after the park and the permission: three rows in D9 order (ANA-2 gate `prd 0.1`; ANA-2 permission `prd 0.1` `edit: src/lib.rs`; FEAT-2 unblock `—`) |

**Literal top-bar assertions** (A-4), updated in commit 1:
- `tests/shell.rs:114` → `"Graphics · DESKTOP-HTUI · memory · 0 working · 0 waiting"`;
- `tests/shell.rs:146` → `"Platform · DESKTOP-HTUI · memory · 1 working · 1 waiting"`;
- `tests/shell.rs:204` → `"Graphics · DESKTOP-HTUI · offline · 3m · 0 working · 0 waiting"`;
- `tests/integration.rs:55` and `src/testkit.rs:947` → the Graphics string.

**`tests/backlog.rs:2366-2379`**: in the comment, `ActiveRuns` → `Waiting`. Nothing else changes.

### 5.7 The snapshot re-accept (once, alone, last)

```bash
INSTA_UPDATE=always cargo test -p htui --all-features -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1          # green without the variable (README.md:489-494)
git diff --stat -- crates/htui/tests/snapshots crates/htui/src  # ~109 files, 1 line each, plus the 3 new ones
# Every changed line of an EXISTING snapshot is the top bar, and nothing else:
git diff -U0 -- crates/htui/tests/snapshots crates/htui/src/snapshots \
  | grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' \
  | grep -vE '· [0-9]+ runs?$|· [0-9]+ working · [0-9]+ waiting$'   # must print nothing
git ls-files --others --exclude-standard -- '*.snap'            # exactly the 3 new snapshots
```

The expected mapping is `· 0 runs` → `· 0 working · 0 waiting` (Graphics and the empty shell) and
`· 1 run` → `· 1 working · 1 waiting` (Platform over the untouched demo). A snapshot whose test
starts or parks runs gets its own counts. Review those individually and expect `working +
waiting ≠ active` in general (D5). `backlog__runs_closeout_warn.snap`'s `1 runs` is the close-out
preview, not the top bar. It must **not** change, and the grep above still allows it.

### 5.8 Commits (T3)

1. `feat(mod-69): Waiting replaces ActiveRuns; the top bar counts working and waiting`: §5.1–§5.3,
   the `store_worker.rs` and `update.rs` test ports, the `top_bar.rs` test, the literal top-bar
   assertions (A-4) and the `tests/backlog.rs` comment. Existing **snapshots** are red until
   commit 4. Everything else is green.
2. `feat(mod-69): the waiting-on-you overlay on Ctrl+W`: §5.4, §5.5 and their unit tests, including
   the `src` offline snapshot and the Boxes guard test.
3. `test(mod-69): waiting list end to end`: `tests/waiting.rs` and its two new snapshots.
4. `test(mod-69): re-accept the top bar in existing snapshots`: §5.7 only, with no code.

---

## 6. Build order and validation, at a glance

| Task | Commits | Gate (`--test-threads=1` on every test run) |
|---|---|---|
| T0 | 1 | `cargo test -p htui-core --all-features --lib model::waiting -- --test-threads=1`; clippy |
| T1 | 3 (§2.8) | `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo test -p htui-store --all-features --test pg_conformance --test cache -- --test-threads=1` (Pg via `HTUI_TEST_DATABASE_URL`); `cargo test -p htui-agent --all-features -- --test-threads=1`; `(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)`; clippy |
| T2 | 3 (§3.7) | `cargo test -p htui-worker --all-features -- --test-threads=1`; clippy |
| T5 | 2 (§4.7) | `cargo test -p htui --all-features --lib tabs::backlog -- --test-threads=1`; `cargo test -p htui --all-features --test reveal -- --test-threads=1`; clippy |
| T3 | 4 (§5.8) | `cargo test -p htui --all-features -- --test-threads=1`; §5.7's diff check; clippy |
| T6 | 1 | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | the plan's § Validation on the real tree: `cargo fmt --all -- --check`; workspace clippy; `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` (then grep `SIGABRT`, per memory); the validator |

## 7. Blueprint decisions

- **E1 (A-1).** The new permission row is named `WaitingPermission`, so it does not shadow
  `OpenPermission`.
- **E2 (H-2).** `views::unblock_case` is the only place `u`'s case is computed. `verdicts` drops
  the case. `waiting` keeps it.
- **E3 (A-8).** `WaitingCandidate::assemble` and `sort_canonical` are the single grouping and order
  for Mem, Pg and the mirror.
- **E4.** The Pg candidate read is three `query_as!` statements, not one JSON-aggregated statement,
  because the row types stay the existing `Item`, `Run` and `RunStep` mappings.
- **E5.** The texts are the constants of §3.3. Reason labels are short (`judge failed` is the
  widest), so the reason column stays narrow.
- **E6 (H-8).** A promoted step with no note reads `promoted to chat`.
- **E7.** The step label is `phase p.a[/i|/j]`, the Runs pane's slot column with the phase in
  front, so a person recognises the step in the pane.
- **E8.** D9's tie rules. Inside an item, rows with a run come before rows without one (Reopen
  last). Inside a run, step rows come before step-less rows (an Unblock row after its run's
  gates). The last tie-breaks are reason (D3 order), then text.
- **E9 (H-10).** `set_scope` clears `top_bar.waiting`, so the old workspace's rows never show under
  the new name.
- **E10 (H-12).** `Waiting` fails as a whole online: no partial view, and `Unreachable` still
  reaches `go_offline`.
- **E11 (A-5).** The Backlog, not the pane, re-reads `Runs` when the item was already selected,
  because only the Backlog knows that `go` short-circuited.
- **E12 (A-3, M-1).** Shortening other help texts is not part of this blueprint.
- **E13.** The overlay asks for nothing on open. A list at most one second old is the plan's
  contract (D6, PRD freshness metric).
- **E14.** No `WorkerStore` method (H-17). There is no `Backend::open_permissions` either: the
  serve arm uses `backend.writer()`, the `RelayView` precedent (`store_worker.rs:1913-1919`).

## 8. Data flow (whole feature)

1. Every fourth 250 ms tick, and on `set_scope`, `App` dispatches `StoreRequest::Waiting { scope }`
   from `Origin::App`.
2. `store_worker::try_serve` reads `Backend::active_runs(scope)` and
   `ReadStore::waiting_candidates(scope)`. Online (`backend.writer()` is `Some`) it also reads
   `WriteStore::open_permissions(scope)`.
3. `htui_worker::waiting(scope, active, &candidates, permissions)` classifies them. It runs
   `verdicts` per candidate for `select`, `unblock_case` for `u`, and step status for gates. It
   then sorts by D9 and E8 and counts by D5.
4. `StoreReply::Waiting(view)` comes back. `observe_reply` sets `top_bar.waiting = Some(view)`
   whatever the origin, and `dirty` redraws.
5. `top_bar::render` draws `N working · M waiting`. If the `WaitingList` overlay is open, it
   renders `ctx.top_bar.waiting`'s rows and clamps its cursor.
6. `Enter` emits `Close`, then `Reveal(Step { item, key, run, step })`. `App::reveal` focuses the
   Backlog (`RevealKind::Item`), and `BacklogTab::reveal` selects the item. If the item is not
   loaded, `PendingReveal` carries the focus to the `Items` reply.
7. The Backlog opens the Runs sub-tab and arms `RunsTab::focus`. If `go` short-circuited, it
   re-requests `Runs`.
8. The pane applies the focus at once when its loaded rows hold the target. Otherwise it applies it
   in `on_runs`, after D198's kept cursor, and syncs the flow.

## 9. T6: close-out (after the review gate)

- Write `docs/decisions/mod/mod-69.md`. Record:
  - D1–D9 as built, and A-1–A-12 and E1–E14 in short;
  - M-1–M-3's outcomes;
  - the measured `serve(Waiting)` timing (H-11);
  - the permission label rule (§3.4) and the chat-run fixture's fate (A-12).
- Add the `DECISIONS.md` index line.
- `HANDOFF.md`:
  - drop the MOD-69 entry (`:579-590`) and its mention in the MOD-N row (`:622`);
  - re-count the pins (`:57-60`): store `CASES` 137, `READ_CASES` 15, `StoreRequest`/`StoreReply`
    unchanged, `.sqlx` 326, `crates/htui/tests/snapshots` = 145 + 2.
- Mark both PRD milestones `complete`. Set the plan's status line.
- Run the validator until it is green.
