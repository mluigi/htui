# Blueprint: MOD-4 milestone 1 — the seam knows what a run is

**Plan**: `.claude/plans/mod-4-orch-seam.plan.md` (APPROVED; D0–D16 settled). **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md` D1–D8 win on conflict. **Design authority**: `docs/ANA-2.md` §4.1–§4.3, §4.6, §4.7, §4.9, §5.1, §5.4, §8, §9; `docs/ANA-5.md` §4.6.
**Verified at**: HEAD `0cf232d`. Every `file:line` below was read at that commit. **Line numbers are pre-edit**: once a task's first commit lands they drift, so anchor on the quoted text, not the number.
**Scope**: T1 (`htui-core`) → T2 (`htui-store`, one migration pair) → T3 (`Writer`/`BufferedWriter`/spies). No engine, no git, no agent, no UI. `crates/htui-orch` is not created.
**House style**: mirrors `.claude/plans/mod-15-hierarchy-seam.blueprint.md` and `mod-15-connection-section.blueprint.md`.

---

## 0. Flags — where the plan and the tree disagree, and what this blueprint does about it

Each row is a plan statement checked against HEAD. "Resolution" is binding on the implementer; anything marked **architect addition** goes beyond the plan's text and is called out so the reviewer can see it was deliberate.

| # | Plan says | Tree at HEAD | Resolution |
|---|---|---|---|
| F-A | Files table: the only `htui` file touched in T1 is `crates/htui/src/app/update.rs`; T2 touches `connection.rs`, `tests/connection.rs`, the confirm snapshot | Confirmed: `update.rs:416-428` builds an `ItemSummary` literal with all eleven fields and no `..`. `ui/tabs/backlog/detail/runs.rs:549-558` uses functional update (`..`) and needs nothing | As planned. The plan's `detail/runs.rs` is `crates/htui/src/ui/tabs/backlog/detail/runs.rs` |
| F-B | Patterns: `pg/write.rs:495-522` is `append_events` | `transition` is `pg/write.rs:442-475`; `append_events` follows it. `set_step_prompt` is `:826-845` as cited | Mirror `transition` (`:442-475`) for every CAS writer, `set_step_prompt` (`:826-845`) for every single-statement writer |
| F-C | D10: `pg/read.rs:701-714` and `:81-95` are struct literals | Both are `query_as!` select lists (`box_info` `:700-714` is a **positional** `query_as!(BoxInfo, "SELECT id AS box_id, hostname, os_family FROM box WHERE id = $1")`; `items` `:81-94` ends `i.updated_at`) | Append the new columns at the **end** of each select list (D10) |
| F-D | D7: the mirror needs `probed_tags`, `declared_tags`, `settings` added to `box` | `cache_migrations/0001_mirror.sql:37-45` **already mirrors all three** | **No DDL** for `box` in `cache_migrations/0003`. Only `cache/read.rs:839-850`'s SELECT and `BoxInfo` builder change |
| F-E | D13: `review` in `implement.input_kinds` and `gate_hard` seeding already applied | Confirmed in `seed.rs` | `seed.rs` untouched |
| F-F | D9: three refresh sites | Confirmed, plus one subtlety: the match at `refresh.rs:271-282` ends `_ => refresh_run_step_commit` (`:281`). Adding `RUN_STEP_TREE` to the const list without touching the match would silently route it to the commit refresher | The match gains an explicit `RUN_STEP_COMMIT =>` arm and a new `RUN_STEP_TREE =>` arm; the `_` fallback dies (see T2 §3.8) |
| F-G | `load_demo` needs edits for new columns | `pg/demo.rs:400`, `:427`, `:498` iterate `data.runs/steps/documents` with explicit column lists; every new column has a default | `load_demo` gains no column. It gains nothing else either: the fixture growth of F-P adds rows to tables it already inserts |
| F-H | D12's "MemStore holds `run_step_commit` rows it never held" | `mem.rs:2575-2576` hard-codes `run_step_commits: 0, command_runs: 0` in the `delete_project` reach literal; `traits.rs:741-743` documents that | `run_step_commits` is counted from `State.step_commits`; `DeleteReach` gains **`run_step_trees`** (its own doc `:736-739`: "a table `0003` adds is a field here") — **architect addition**, moves one tuple assertion at `conformance.rs:2271-2277` |
| F-I | D5: reuse `DATABASE_UNREACHABLE` | `writer.rs:638` `pub const DATABASE_UNREACHABLE`; `hierarchy_needs_the_server()` `:652-654` wraps it and its doc `:640-651` already says it serves reads | `BufferedWriter` arms call `hierarchy_needs_the_server()` unchanged (no new helper). `Backend::Offline` arms for the eleven inherent reads need a like-named helper beside `prompt_offline()` (`backend.rs:423-425`): `fn orchestration_offline() -> StoreError { StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()) }` |
| F-J | D8: `Pending(2)` at `migrations.rs:353-357`, `connect.rs:93-97` | `migrations.rs:355`; `connect.rs:95` **and** `connect.rs:110` (`pending, 2`) — three sites, not two. `applied == vec![1, 2]` is `:69-73`; the "six commented columns" message is `:260` inside `:206-264`; `TABLES` `:18-51`, `TABLES.len() == 32` `:88-92`, `present.len() == TABLES.len() + 1` `:94-98` | All amended in T2's migration commit (§3.9) |
| F-K | D7: "~13 `COMMENT ON COLUMN`" | ANA-2 §9 has **18** `COMMENT ON COLUMN` + 1 `COMMENT ON TABLE` | Transcribed in full in §3.2. `ANA_COLUMN_COMMENTS` (`migrations.rs:159-203`) is **extended** to 24 entries, never trimmed |
| F-L | `RunStep.fanout_index` doc: `0..fan_out` | `run.rs:136` | Doc becomes "`0..fan_out`; `-1` is the judge step (ANA-2 §4.5)". No `CHECK` anywhere |
| F-M | "sixteen mirrored tables" strings | `connection.rs:136` (`CONFIRM_REBUILD`), `:989`; `crates/htui/tests/connection.rs:566` (doc), `:1664`; `tests/snapshots/connection__confirm.snap:32`; docs at `cache/mod.rs:31` and `store_worker.rs:488` | All become seventeen; one snapshot re-recorded (§3.9) |
| F-N | Files table: "5 trait reads + 16 inherent reads" | D1 (authoritative) counts **eleven** new inherent reads: `step_graph`, `phase_agents`, `prompt_template`, `resolve_graph`, `agent_boxes`, `box_row`, `repo_paths`, `ready_items`, `missing_tags`, `active_runs_on_box`, `overlapping_runs`. Verified none exists on `traits.rs`, `backend.rs` or `pg/read.rs` today | Eleven. Each needs a like-named `MemStore` inherent (the `Backend::Memory` arm calls it), which lands in **T1** |
| F-O | D2: `resolve_inputs(...) -> Vec<Document>` "a missing kind reported as such" | A `Vec<Document>` cannot report a missing kind | Return type is `Vec<ResolvedInput>` (`{ kind, document: Option<Document> }`), one entry per requested kind in request order — refinement of D2, not a redesign |
| F-P | D12's READ cases "read the fixture and write nothing" (`conformance.rs:166`); D13 says the fixture corrections "break nothing" | The fixture has **no** `repo`, `run_step_tree`, `run_step_commit` row, **no** loser step (`selected: Some(false)`), and its only step-produced documents are `DOC_FEAT_1_PLAN_V1/V2` (`fixtures.rs:1184`, `:1193`, both by `STEP_PLAN`). `trees_and_commits_read_back` and `resolve_inputs_prefers_this_run_and_skips_losers` cannot assert their names on it. A fixture `repo` is ruled out: `repo_round_trip_and_primary_flag` asserts `repos(PROJECT_HTUI) == ["core","docs"]` (`conformance.rs:2636-2645`), `project_delete_takes_everything_and_says_so` asserts `report.repos == 0` (`:2271-2277`), and the hierarchy pane renders repos | **Architect addition (F-P growth)**: one finished fan-out run on `HTUI_ANA_1` (`RUN_3`, winner `STEP_R3_RESEARCH_A`, loser `STEP_R3_RESEARCH_B`, documents `DOC_ANA_1_RESEARCH_V2` by the winner and `_V3` by the loser). Nothing renders or pins ANA-1's runs or research versions (checked: `backlog__*` snapshots select FEAT-1; `documents_ordered_by_version` and `documents_of_kinds_latest_per_kind_in_order` read FEAT-1; `migrations.rs:878-880` counts derive from `data.*.len()`; `project_delete…` asserts only `> 0` for runs/steps/documents). `trees_and_commits_read_back` asserts totality on the fixture (empty, never `NotFound`) and defers row content to its writer twin and to `pg_criteria`. Details in §2.9 and §2.10 |
| F-Q | §8 signature `add_note(item, body, via_step)` | Schema `item_note.created_by NOT NULL`; `mem.rs:2714` `require_author` | `add_note(&self, note: NewNote)` with `NewNote` carrying `created_by` and `created_at` (mirrors `NewItem`/`ChatRunSpec` request-struct naming) |
| F-R | §8 `select_fanout(run, position, attempt, winner)` | §4.5 (`docs/ANA-2.md:850-856`) writes the judge's `gate_note` in the same transaction; four args cannot carry it | Fifth parameter `reason: Option<String>` |
| F-S | §8 `claim_run(run, box, owner, ttl)`, `refresh_lease(run, owner, ttl)`, `adopt_runs(box, owner)` | `MemStore` has no `now()` the conformance suite can compare against Postgres; `ChatRunSpec.started_at` (`run.rs:216-223`) and `finish_chat_run(finished_at)` already pass the clock in | Every writer that stamps a `started_at`/`finished_at`/`lease_expires_at`/`promoted_at` takes the instant from the caller (`at`, `lease_until`, `now`). `updated_at` stays the trigger's / `MemStore`'s own clock and is never asserted equal across backends |
| F-T | §4.7 `BoxSettings.max_concurrent_items: u32` with `#[serde(default)]` | A `u32` default is `0`, which admits nothing; the rung below is `app_setting.max_concurrent_items` | `max_concurrent_items: Option<u32>`; `None` means "use the `app_setting` rung, else `DEFAULT_MAX_CONCURRENT_ITEMS = 2`" |
| F-U | D11: `ProjectSettings` in `kind.rs`, `BoxSettings` in `box_.rs` | `hierarchy.rs` holds `Project`; `box_.rs` holds `BoxRow` | As the plan's Files table says: `BoxSettings` in `box_.rs`, `ProjectSettings` in `kind.rs` |
| F-V | Snapshot enum serialisation | `str_enum!` (`model/mod.rs:25-50`) puts `#[serde(rename = $text)]` on every variant | `GraphSnapshot` fields typed with `Isolation`, `Gate`, `CommandQueue`, `RunMode` serialise to §5.1's lowercase strings with no extra attributes |
| F-W | `step_graph_phase` gains judge columns in `0003` | The plan's Files table widens `StepGraph` (`is_override`) but **not** `StepGraphPhase` | Columns land in DDL only; `StepGraphPhase` is unchanged this milestone. Milestone 2's snapshot builder reads them when it needs them |
| F-X | `cache.rs` schema_version literals | Plan D8: unchanged. `PgStore::schema_version()` (`pg/mod.rs:389-391`) is the max embedded version → **3**, which forces a full mirror rebuild on first connect | As planned; hazard H-6 |


### 0.1 Amendments from the T1 adversarial audit (2026-09-18, after T1's conformance commits)

Four verifiers audited T1 at `0a85383`. Everything actionable inside `htui-core` was fixed in T1's own
commits; what follows is what they found **for T2**, recorded here because the sections above were what
caused each miss. Each is also written into the section it belongs to, marked "(T1 audit)". The plan's
D0–D16 are untouched: none of this redesigns a decision, and where a finding argued against a settled
one it was rejected (see the T1 close-out note).

| # | What the audit found | Where it is now written | Why it was missed |
|---|---|---|---|
| A-1 | `crates/htui/tests/kinds.rs:975` (`StepGraph`) and `crates/htui/src/hierarchy.rs:478-525` (`DeleteReach`) are exhaustive literals no Files-to-Change row names | Fixed in T1, commit `13f3020`; §2.5's enumeration recipe is now unscoped | §2.5 said `rg -n 'StepGraph \{' crates/htui-core/src crates/htui-store/src` — two crates, and both sites are in a third |
| A-2 | `pg_criteria.rs:485-491` needs `Open → Queued → InProgress → Done`, not `Open → Queued`: the legs after it assert `closed_at.is_some()` and a reopen | §3.9's `tests/pg_criteria.rs` row, part (a) | D4 names the site; §3.9 named the site but not the shape |
| A-3 | `pg_criteria.rs:2739-2832` zips `TABLES: [&str; 21]` against `claimed: [u64; 21]` positionally, and `run_step_trees` is struct index 16 — appending to both arrays compiles and mis-pairs every later field | §3.9's `tests/pg_criteria.rs` row, part (b) | F-H listed the `DeleteReach` sites and stopped at `conformance.rs:2271-2277` |
| A-4 | `query_as!(StepGraph, …)` must **insert** `is_override`, not append it, at **four** sites | §3.4 (`- Every `query_as!` over `step_graph` …`) and §3.6's `step_graph(id)` row | The blueprint contradicted itself: §2.5 said "before `created_at`" (correct, and what shipped), §3.4/§3.6 said "appends" |
| A-5 | `box_info`'s `probed_tags` / `declared_tags` are both `Vec<String>` and adjacent, so a transposed append binds silently | §3.4's `box_info` bullet | D10's append rule assumes the compiler catches a mis-order; here it cannot |
| A-6 | `tests/cache.rs`'s tree/commit case is the only pin that the mirror carries the two new tables — `trees_and_commits_read_back` is a totality case on an empty fixture and both its row-content twins are `WriteStore` cases | §3.9's `tests/cache.rs` row, now marked a gate; the case's own doc in `conformance.rs` says it too | F-P set up the empty fixture deliberately and did not name the consequence for the mirror |
| A-7 | The four Postgres-only twin names are listed in `PENDING` in `conformance.rs`'s `every_cross_referenced_test_name_exists`; deleting an entry is part of writing its test, and the test fails by name if the entry outlives the fn | §3.9's `tests/pg_criteria.rs` row, part (c) | T1 shipped them as `pg_criteria::<name>`, a spelling the scanner skipped in silence; fixed in T1, commit `958a987` |

### 0.2 Amendments from the T2 adversarial audit (2026-09-18, after T2's close-out)

Four verifiers audited T2 at `ea48712`. Everything actionable inside `htui-store` (and the two
`htui-core` halves a store-side divergence implicated) was fixed in T2's repair commits `e51c824`,
`06131b7` and `78326dd`; the one T3 finding is §4.5. D0–D16 are untouched.

| # | What the audit found | Disposition |
|---|---|---|
| B-2 | `PgStore::create_run` accepted a `repo_scope` naming a repo that does not exist, where `MemStore` refuses. `run.repo_scope` is a `UUID[]` and an array element cannot carry a `REFERENCES` clause, so the schema cannot refuse it and the writer must — and `claim_run`'s `repo_scope && $2::uuid[]` would otherwise compare against a phantom id for the run's life. The only unenforced reference among the eighteen writers | **Fixed** (`e51c824`): an `unnest` / `LEFT JOIN repo` probe inside the transaction, before the first write, returning `references_no_row("run.repo_scope", …)`. `run_create_moves_the_item` gained the leg |
| B-3 | `upsert_step_tree` and `record_commits` judged the batch before probing the step, so an unknown step carrying a stray row was `Constraint` where `MemStore`'s `check_step_batch` answers `NotFound` — plan D14 inverted on the Postgres side | **Fixed** (`e51c824`): the probe moved above the loop on both writers; `trees_and_commits_round_trip` gained a leg for each |
| B-4 | `MemStore::create_run` checked the duplicate id and four foreign keys **before** it looked the item up, so an unknown item plus an unknown project was `Constraint` where `PgStore` answers `NotFound`. `PgStore` is D14-compliant here and `MemStore` was the deviant one | **Fixed** (`e51c824`): `require_item` + `legal_move` moved to the top of `MemStore::create_run`; one conformance leg gets both wrong at once |
| B-5 | The mirror's three-armed `CASE` rank in `resolve_inputs` had no test behind it — §0.1's A-6 hole in its second form. Deleting the whole `CASE` left `the_mirror_passes_the_read_cases` green | **Fixed** (`06131b7`): `tests/cache.rs::the_mirror_ranks_resolve_inputs_the_way_postgres_does`, checked by deleting the `CASE` and watching it turn red while the read-case harness stayed green |
| B-1 | `cargo check -p htui --all-targets` does not pass at HEAD; five hand-written `htui` files have never been compiled | **Deferred to T3**, §4.5. Verified correct behind a throwaway stub, not committed |
| B-6 | `run.repo_scope` / `lease_*` had no test that gave `repos_col` and `opt_ts_col` a value to decode, and `tests/cache.rs`'s "have no reader yet" comment was stale from `9da9565` | **Fixed** (`06131b7`) |
| B-7 | `resolve_inputs` on the mirror can answer a fan-out **loser's** document mid-pass: `document` is refreshed and committed before `run_step`, so between the two commits the row joins to no step and passes eligibility | **Documented, no code change** (`06131b7`). Design-inherent to the per-table cursor pass; ANA-9 §6.2 promises whole-table, not cross-table, consistency |
| B-8 | ANA-2 §5.4's twelve seeded `app_setting` defaults were pinned by count only; ANA-5's ten are value-pinned twice over | **Fixed** (`78326dd`): `ANA2_DEFAULTS` and `the_twelve_ana2_defaults_land_with_their_values`, as JSON text because three are `null` and one is an object |
| B-9 | `adopt_runs` broke a `queued_at` tie by nothing, where `MemStore` sorts `(queued_at, RunId)`; `close_out`'s live-run probe already spells the tiebreak | **Fixed** (`78326dd`): `ORDER BY queued_at, id`, and the trait doc now says the order is total |
| B-10 | `the_six_ana_comments_are_present_and_verbatim` guards twenty-five entries | **Fixed** (`78326dd`): renamed count-free |
| B-11 | The T2 gate line names no `DATABASE_URL`; run with the bare `…/postgres` one it emits 212 errors and exits 1 | **Fixed**: §1's T2 row and §3.10 |

---

## 1. Build order and validation, at a glance

Serial: **T1 → T2 → T3**. `cargo test --workspace` is red from T1's first commit to T2's last (D0). Each task's commits are grouped by concern, in the order the conformance cases are listed in D12, and each commit is one `cargo test -p <crate>` green.

| Task | Crate(s) | Commits (minimum) | Validation (copied from the plan `:175-183`) |
|---|---|---|---|
| T1 | `htui-core` (+ one `htui` test helper) | 6 | `cargo test -p htui-core --all-features` then `cargo clippy -p htui-core --all-targets --all-features -- -D warnings` |
| T2 | `htui-store`, `htui` strings | 6 | `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features` then, from `crates/htui-store`, `DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features` (the URL must name a **migrated** database, not the bare `…/postgres` one — see §3.10) |
| T3 | `htui-store` writer, `htui-agent` spies | 3 | `cargo test --workspace --all-features` |

Memory note that applies: run each gate yourself with `--test-threads=1` before declaring green (the keyring fake is process-wide); check `df -h /` before blaming Postgres.

---

## 2. T1 — `htui-core`: types, law, traits, `MemStore`, fixture, conformance

### 2.1 Files

| File | Action | What |
|---|---|---|
| `crates/htui-core/src/model/run.rs` | edit | `RunStatus::{can_move_to,is_terminal}`, `StepStatus::{can_move_to,is_terminal}`, `VerifyOutcome`, `Run` +3, `RunStep` doc, `RunStepSummary` +6, `NewRun`, `NewRunStep`, `StepOutcome`, `RunStepTree`, `GraphSnapshot` family |
| `crates/htui-core/src/model/item.rs` | edit | `Status::can_move_to`; `ItemSummary.touched_paths`; `Item::summary()` |
| `crates/htui-core/src/model/box_.rs` | edit | `BoxInfo` +3; `BoxSettings`; `DEFAULT_MAX_CONCURRENT_ITEMS` |
| `crates/htui-core/src/model/kind.rs` | edit | `StepGraph.is_override`; `ProjectSettings`; `ResolvedGraph`, `ResolvedPhase` |
| `crates/htui-core/src/model/note.rs` | edit | `NewNote` |
| `crates/htui-core/src/model/document.rs` | edit | `NewDocument`, `ResolvedInput` |
| `crates/htui-core/src/model/mod.rs` | edit | re-exports |
| `crates/htui-core/src/store/traits.rs` | edit | `TransitionLaw`, `legal_move`, `illegal_move`; `ReadStore` +5; `WriteStore` +18; `DeleteReach.run_step_trees` + doc |
| `crates/htui-core/src/store/mem.rs` | edit | `State.{step_trees,step_commits}`; 5 trait reads; 18 writers (five as single `write` closures); 11 inherent reads; reach counts |
| `crates/htui-core/src/seed.rs` | edit | `StepGraph {` literal at `:193` gains `is_override: false` (the only seed edit) |
| `crates/htui-core/src/fixtures.rs` | edit | D13 (`attempt` ×5, `graph_snapshot` ×2) and F-P growth |
| `crates/htui-core/src/store/conformance.rs` | edit | D4's three call sites; 14 cases; `CASES` 36 → 47; `READ_CASES` 6 → 9 |
| `crates/htui-core/tests/mem_store.rs` | edit | `CASES.len() == 47` (`:37`), `READ_CASES.len() == 9` (`:44`) |
| `crates/htui/src/app/update.rs` | edit | `ItemSummary` literal at `:416-428` gains `touched_paths: Vec::new()` |

### 2.2 `model/run.rs`

**Law on the two run enums.** Insert directly after `RunStatus::is_active` (`run.rs:50-53`) and after the `StepStatus` `str_enum!` (`:55-73`). Transcribed from ANA-2 §4.3 (`docs/ANA-2.md:565-654`); exhaustive over `self`, never `_`.

```rust
impl RunStatus {
    /// ANA-2 §4.3's `run` table: whether `self → to` is a sanctioned move. Every terminal status
    /// answers `false` for every `to`.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Queued => matches!(to, Self::Running | Self::Cancelled | Self::Failed),
            Self::Running => matches!(
                to,
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Cancelled
            ),
            Self::AwaitingApproval => matches!(to, Self::Running | Self::Failed | Self::Cancelled),
            Self::Done | Self::Failed | Self::Cancelled => false,
        }
    }

    /// `done | failed | cancelled` — the complement of [`RunStatus::is_active`].
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !self.is_active()
    }
}

impl StepStatus {
    /// ANA-2 §4.3's `run_step` table. `failed` is **not** terminal for a step: it can be promoted
    /// to `awaiting_approval` (§4.8) while its run is live, and, like every non-terminal status,
    /// can be cancelled. The "while the run is non-terminal" condition on promotion belongs to
    /// [`WriteStore::promote_step`](crate::store::WriteStore::promote_step), not to this table.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Pending => matches!(to, Self::Running | Self::Superseded | Self::Cancelled),
            Self::Running => matches!(
                to,
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Cancelled
            ),
            Self::AwaitingApproval => matches!(
                to,
                Self::Done
                    | Self::Failed
                    | Self::Superseded
                    | Self::AwaitingApproval
                    | Self::Cancelled
            ),
            Self::Failed => matches!(to, Self::AwaitingApproval | Self::Cancelled),
            Self::Done => matches!(to, Self::Superseded),
            Self::Cancelled | Self::Superseded => false,
        }
    }

    /// `done | cancelled | superseded` (ANA-2 §4.3). `failed` is deliberately absent.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Superseded)
    }
}
```

`AwaitingApproval → AwaitingApproval` is the promotion row of §4.3 (a gated step promoted to chat keeps its status and gains `promoted_at`). It is the one self-move any table sanctions; the CAS still returns `Ok(true)` for it because the row's `promoted_at` changes.

**`VerifyOutcome`.** New `str_enum!` beside `GateOutcome` (`:75-87`):

```rust
str_enum!(
    /// `run_step.verify_outcome` (ANA-2 §4.2): `unavailable` never fails a step.
    VerifyOutcome {
        /// The verify command exited 0.
        Pass => "pass",
        /// The verify command exited non-zero.
        Fail => "fail",
        /// No verify command, or it could not be run.
        Unavailable => "unavailable",
    }
);
```

**`Run` (`:90-123`)** — three fields appended after `updated_at`'s predecessor group; keep `updated_at` last to match every other row type:

```rust
    /// `run.repo_scope`: repos this run may touch (ANA-2 §4.7); empty on every row older than
    /// `0003` and on a chat run.
    pub repo_scope: Vec<RepoId>,
    /// `run.lease_box_id` (ANA-2 §4.9).
    pub lease_box_id: Option<BoxId>,
    /// `run.lease_expires_at`; `None` = never claimed. `run.lease_owner` is deliberately not on
    /// this row: the mirror does not carry it and a reader has no use for another process's token.
    pub lease_expires_at: Option<DateTime<Utc>>,
```

Place them **before** `updated_at`. Construction sites (`rg -n 'Run \{' crates` — verify the list): `fixtures.rs:1226`, `:1243` (+ `RUN_3`), `mem.rs:1286-1302` (`start_chat_run`, `repo_scope: Vec::new()`, both leases `None`), and the new `mem.rs` `create_run`/`adopt_runs` paths. `RunSummary` (`:331-360`) is **not** widened: it is the Runs pane's projection and nothing in this milestone renders a lease.

**`RunStep` (`:126-169`)** — no new fields on the row type itself except the three columns, appended before `updated_at`:

```rust
    /// `run_step.verify_outcome` (ANA-2 §4.2).
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.verify_exit_code`: the verify command's own exit code, distinct from `exit_code`.
    pub verify_exit_code: Option<i32>,
    /// `run_step.promoted_at` (ANA-2 §4.8).
    pub promoted_at: Option<DateTime<Utc>>,
```

and the `fanout_index` doc at `:136` becomes `` /// `run_step.fanout_index`: `0..fan_out`; `-1` is the judge step of this position and attempt (ANA-2 §4.5). ``. Construction sites: `fixtures.rs` `done_step` (`:1318-1348`) and the `STEP_R2_PRD` literal (`:1293-1314`), `mem.rs:1303-1324` (`start_chat_run`), the new `create_step` in `mem.rs`.

**`RunStepSummary` (`:264-298`)** — six fields appended after `trimmed` (`:297`), in this order (D10, ANA-2 §6.2):

```rust
    /// `run_step.usage`, whole (ANA-2 §6.2).
    pub usage: Option<Value>,
    /// `run_step.selected`.
    pub selected: Option<bool>,
    /// `run_step.exit_code`.
    pub exit_code: Option<i32>,
    /// `run_step.verify_outcome`.
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.promoted_at`.
    pub promoted_at: Option<DateTime<Utc>>,
    /// `agent.name` of `agent_id`, denormalised for the Runs pane; `None` when `agent_id` is
    /// `None` or names no row.
    pub agent_name: Option<String>,
```

Builders that must agree field for field (pinned by `run_and_steps_round_trip`): `pg/rows.rs:125-141` (`StepRow::into_summary`), `mem.rs:828-845` (`run_steps` helper), `cache/read.rs:537-557`. `detail/runs.rs:549-558` uses `..` and compiles unchanged.

**Request and outcome types**, after `ChatRunSpec` (`:199-249`):

```rust
/// Arguments of [`crate::store::WriteStore::create_run`]: a `kind = 'graph'` run inserted at
/// `queued`, with the item moved to `queued` in the same transaction (ANA-2 §4.3, §5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRun {
    /// `run.id`, minted by the caller so a retry is idempotent.
    pub id: RunId,
    /// `run.project_id`.
    pub project_id: ProjectId,
    /// `run.item_id`: a graph run always has one.
    pub item_id: ItemId,
    /// `run.mode`.
    pub mode: RunMode,
    /// `run.target_box_id`.
    pub target_box_id: BoxId,
    /// `run.started_by`.
    pub started_by: UserId,
    /// `run.graph_snapshot`, serialised by the store (`R-ORCH-11`).
    pub graph_snapshot: GraphSnapshot,
    /// `run.repo_scope` (ANA-2 §4.7).
    pub repo_scope: Vec<RepoId>,
    /// `run.queued_at`; the caller's clock, microsecond-truncated like [`ChatRunSpec::started_at`].
    pub queued_at: DateTime<Utc>,
}

/// Arguments of [`crate::store::WriteStore::create_step`]: a `run_step` inserted at `pending`
/// with every settle column `NULL`. `UNIQUE (run_id, position, attempt, fanout_index)` is the
/// database's; a repeat is `Constraint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewRunStep {
    /// `run_step.id`.
    pub id: StepId,
    /// `run_step.run_id`.
    pub run_id: RunId,
    /// `run_step.position`.
    pub position: i32,
    /// `run_step.attempt`, 1-based.
    pub attempt: i32,
    /// `run_step.fanout_index`; `-1` for the judge.
    pub fanout_index: i32,
    /// `run_step.phase_name`.
    pub phase_name: String,
    /// `run_step.agent_id`.
    pub agent_id: Option<AgentId>,
    /// `run_step.model`.
    pub model: Option<String>,
}

/// What [`crate::store::WriteStore::finish_step`] writes: the settle columns and nothing about
/// `status`, which only the §4.3 law moves. `usage` and `trim_record` `None` **leave** the column
/// (the assembler and the usage summer write them earlier); every other field overwrites.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// `run_step.exit_code`.
    pub exit_code: Option<i32>,
    /// `run_step.usage`; `None` leaves the column.
    pub usage: Option<Value>,
    /// `run_step.trim_record`; `None` leaves the column.
    pub trim_record: Option<Value>,
    /// `run_step.verify_outcome`.
    pub verify_outcome: Option<VerifyOutcome>,
    /// `run_step.verify_exit_code`.
    pub verify_exit_code: Option<i32>,
    /// `run_step.finished_at`.
    pub finished_at: DateTime<Utc>,
}

/// A row of `run_step_tree` (ANA-2 §4.6): the isolation tree of one repo for one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStepTree {
    /// `run_step_tree.run_step_id`.
    pub run_step_id: StepId,
    /// `run_step_tree.repo_id`.
    pub repo_id: RepoId,
    /// `run_step_tree.mode`; the same `CHECK` list as `step_graph_phase.isolation`.
    pub mode: Isolation,
    /// `run_step_tree.path`: absolute, on the executing box.
    pub path: String,
    /// `run_step_tree.base_ref`.
    pub base_ref: String,
    /// `run_step_tree.dirty`.
    pub dirty: bool,
}
```

**`GraphSnapshot` family** (ANA-2 §5.1, `docs/ANA-2.md:1440-1475`; `#[serde(default)]` on every optional so a snapshot written by a later builder still decodes here). `Run.graph_snapshot` stays `Option<Value>` (a reader that only lists runs never decodes it); `NewRun` carries the typed form and the store serialises it.

```rust
/// `run.graph_snapshot` (`R-ORCH-11`, ANA-2 §5.1): the graph as it was at queue time. The only
/// route to a graph while offline, since `step_graph` is not mirrored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// Schema version; a reader that meets an unknown `v` refuses rather than guesses.
    pub v: u32,
    pub graph: SnapshotGraph,
    /// `sha256:…` over the canonical `phases[]`; milestone 2 computes it, milestone 1 stores it.
    pub topology: String,
    pub mode: RunMode,
    pub phases: Vec<SnapshotPhase>,
    pub settings: SnapshotSettings,
}

impl GraphSnapshot {
    /// The `v` this crate writes and the only one it reads.
    pub const V: u32 = 1;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotGraph {
    pub id: StepGraphId,
    pub name: String,
    #[serde(default)]
    pub is_override: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPhase {
    pub position: i32,
    pub name: String,
    pub fan_out: i32,
    pub gate: Gate,
    /// `gate` after the `R-ORCH-6` downgrade; equal to `gate` when nothing downgraded it.
    pub gate_effective: Gate,
    pub gate_hard: bool,
    pub retry_limit: i32,
    pub input_kinds: Vec<String>,
    pub output_kind: String,
    /// Resolved: the phase's own, else `ProjectSettings::default_isolation`.
    pub isolation: Isolation,
    pub command_queue: CommandQueue,
    #[serde(default)]
    pub verify_command: Option<String>,
    #[serde(default)]
    pub deadline_seconds: Option<u32>,
    pub template: SnapshotTemplate,
    #[serde(default)]
    pub token_budget: Option<i32>,
    pub candidates: Vec<SnapshotCandidate>,
    #[serde(default)]
    pub judge: Option<SnapshotJudge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotTemplate {
    pub name: String,
    /// Resolved: never `None` in a snapshot, unlike `step_graph_phase.template_version`.
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotCandidate {
    pub agent_id: AgentId,
    pub agent_name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotJudge {
    pub agent_id: AgentId,
    pub agent_name: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotSettings {
    pub default_isolation: Isolation,
    #[serde(default)]
    pub per_token_cap_run: Option<i64>,
    #[serde(default)]
    pub per_token_cap_batch: Option<i64>,
    pub max_fan_out: u32,
    pub max_agents_per_run: u32,
}
```

`Gate`, `Isolation`, `CommandQueue` come from `kind.rs:10-48`; their `str_enum!` `#[serde(rename)]` makes them the lowercase strings §5.1 shows (F-V).

### 2.3 `model/item.rs`

After `Status::is_terminal` (`item.rs:31-36`), the item table of §4.3:

```rust
    /// ANA-2 §4.3's `item` table: whether `self → to` is sanctioned. `closed` reaches nothing.
    #[must_use]
    pub const fn can_move_to(self, to: Self) -> bool {
        match self {
            Self::Open => matches!(to, Self::Queued | Self::Blocked),
            Self::Queued => matches!(to, Self::InProgress | Self::Open | Self::Blocked),
            Self::InProgress => matches!(
                to,
                Self::AwaitingApproval | Self::Done | Self::Failed | Self::Blocked | Self::Open
            ),
            Self::AwaitingApproval => matches!(to, Self::InProgress | Self::Failed | Self::Open),
            Self::Blocked => matches!(to, Self::Open | Self::Closed),
            Self::Failed => matches!(to, Self::Queued | Self::Closed),
            Self::Done => matches!(to, Self::Closed | Self::Open),
            Self::Closed => false,
        }
    }
```

`ItemSummary` (`:101-125`): append `/// \`item.touched_paths\` (ANA-2 §4.7). pub touched_paths: Vec<String>,` **after `updated_at`** — D10 says append, and `pg/read.rs:81-94`'s select list ends `i.updated_at`, so the new column goes after it in both. Builders: `Item::summary()` (`:83-97`, `touched_paths: self.touched_paths.clone()`), `pg/read.rs:81-94` (`i.touched_paths` appended — T2), `cache/read.rs:169-183` (`item_summary_of`, from the mirror row's `touched_paths` which `0001_mirror.sql:80` already holds — T2), `crates/htui/src/app/update.rs:416-428` (`touched_paths: Vec::new()` — T1, same commit as the type).

### 2.4 `model/box_.rs`

`BoxInfo` (`:85-92`), three fields appended after `os_family`:

```rust
    /// `box.probed_tags` (`R-ORCH-10`).
    pub probed_tags: Vec<String>,
    /// `box.declared_tags`.
    pub declared_tags: Vec<String>,
    /// `box.settings`, whole; decode with [`BoxSettings`].
    pub settings: Value,
```

Builders: `mem.rs:221-231` (from the `BoxRow`), `pg/read.rs:700-714` (append `probed_tags, declared_tags, settings` to the positional select — T2), `cache/read.rs:839-850` (append the three columns to `SELECT id, hostname, os_family FROM box ORDER BY id LIMIT 1` and decode with the file's `strings_col`/JSON text helpers — T2). `BoxProfile` (`:97-101`) is untouched on purpose.

`BoxSettings` and its rung constant, after `BoxInfo`:

```rust
/// `box.settings` as ANA-2 §4.7 reads it. Read-only this milestone (plan D11): MOD-15's
/// key-level `set_setting` is the only writer, so unknown keys survive because nothing here is
/// ever re-serialised onto the row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoxSettings {
    /// `R-ORCH-9`; `None` = the `app_setting` rung, else [`DEFAULT_MAX_CONCURRENT_ITEMS`].
    pub max_concurrent_items: Option<u32>,
    /// `R-MCP-3` `{class: n}`; empty = the `app_setting` rung.
    pub command_limits: BTreeMap<String, u32>,
}

/// The value `0003_orchestration.sql` seeds under `app_setting.max_concurrent_items`, and what
/// a store answers when neither the box nor the table names one.
pub const DEFAULT_MAX_CONCURRENT_ITEMS: u32 = 2;
```

### 2.5 `model/kind.rs`

`StepGraph` (`:130-146`): append `/// \`step_graph.is_override\` (ANA-2 §4.1): hidden from the graph list. pub is_override: bool,` before `created_at`. Construction sites: `seed.rs:193` (`is_override: false`), `mem.rs` `create_step_graph` literal, and in T2 every `query_as!` whose text reads `step_graph` — `pg/write.rs:251` and `:339` (the mint/update pair; confirm by their SQL), `pg/read.rs:1251` and `:1390`. Run `rg -n 'StepGraph \{' crates/htui-core/src crates/htui-store/src` and `rg -n -B1 -A6 'FROM step_graph\b|INTO step_graph\b|UPDATE step_graph\b' crates/htui-store/src/pg` before touching anything; `NewStepGraph`/`StepGraphPatch` gain nothing (an override is minted by milestone 2's clone path, not by a user edit). **Enumerate construction sites unscoped** — `grep -rn 'StepGraph {' crates/ --include='*.rs'` — not over `crates/htui-core/src crates/htui-store/src`: the two-crate recipe this line first carried cannot see `crates/htui/tests/kinds.rs:975`, which names all six old fields with no `..`. Same for `DeleteReach`, whose `crates/htui/src/hierarchy.rs:478-525` destructure is exhaustive and returns a fixed-length array on purpose. Both are fixed in T1 (commit `13f3020`); the rule stands for every widened type. (T1 audit.)

`ProjectSettings` (ANA-2 §4.7, `docs/ANA-2.md:1121-1138`), beside `PhaseAgent`:

```rust
/// `project.settings` as ANA-2 §4.7 reads it. Read-only (plan D11); every field defaults so
/// `'{}'` decodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    pub default_isolation: Isolation,
    pub token_budget: Option<i32>,
    pub retention_days: Option<i32>,
    pub cached_transcript_steps: Option<i32>,
    pub keep_raw_events: bool,
    /// Micros.
    pub per_token_cap_run: Option<i64>,
    /// Micros.
    pub per_token_cap_batch: Option<i64>,
    pub step_deadline_seconds: Option<u32>,
    pub default_agent_id: Option<AgentId>,
    pub judge_agent_id: Option<AgentId>,
    pub copy_exclude: Vec<String>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            default_isolation: Isolation::Worktree,
            token_budget: None,
            retention_days: None,
            cached_transcript_steps: None,
            keep_raw_events: false,
            per_token_cap_run: None,
            per_token_cap_batch: None,
            step_deadline_seconds: None,
            default_agent_id: None,
            judge_agent_id: None,
            copy_exclude: Vec::new(),
        }
    }
}

/// What [`Backend::resolve_graph`](../../htui-store) answers in one round trip (ANA-2 §8): the
/// graph an item runs under and its phases with their candidate agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedGraph {
    pub graph: StepGraph,
    /// In `position` order.
    pub phases: Vec<ResolvedPhase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPhase {
    pub phase: StepGraphPhase,
    /// `phase_agent` rows in `position` order; empty on `MemStore`, which holds no such table.
    pub agents: Vec<PhaseAgent>,
}
```

### 2.6 `model/note.rs`, `model/document.rs`

```rust
// note.rs, after Note (:11-26)
/// Arguments of [`crate::store::WriteStore::add_note`]: `item_note` verbatim minus nothing, since
/// the caller mints the id and reads the clock (F-S).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewNote {
    pub id: NoteId,
    pub item_id: ItemId,
    pub body: String,
    pub created_by: UserId,
    pub box_id: Option<BoxId>,
    pub via_step_id: Option<StepId>,
    pub created_at: DateTime<Utc>,
}

// document.rs, after DocumentHead
/// Arguments of [`crate::store::WriteStore::write_document`]: everything but `version`, which
/// the store allocates under the item's row lock (ANA-2 §4.2, plan D6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewDocument {
    pub id: DocumentId,
    pub item_id: ItemId,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub produced_by_step_id: Option<StepId>,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
}

/// One requested kind of [`crate::store::ReadStore::resolve_inputs`], present or missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedInput {
    pub kind: String,
    /// `None` = the item has no eligible document of this kind; the step fails (ANA-2 §4.2).
    pub document: Option<Document>,
}
```

### 2.7 `model/mod.rs` re-exports

Extend the existing lines rather than adding new ones: `pub use run::{…}` (`:121-124`) gains `GraphSnapshot, NewRun, NewRunStep, RunStepTree, SnapshotCandidate, SnapshotGraph, SnapshotJudge, SnapshotPhase, SnapshotSettings, SnapshotTemplate, StepOutcome, VerifyOutcome`; `pub use box_::{…}` (`:98`) gains `BoxSettings, DEFAULT_MAX_CONCURRENT_ITEMS`; `pub use kind::{…}` (`:111-114`) gains `ProjectSettings, ResolvedGraph, ResolvedPhase`; `pub use note::Note` (`:116`) becomes `pub use note::{NewNote, Note}`; `pub use document::{Document, DocumentHead}` (`:99`) gains `NewDocument, ResolvedInput`. Keep each list alphabetical as the file does.

### 2.8 `store/traits.rs`

**Shared legality helper (D15, D14)**, placed directly after `chat_step_status` (`:617-625`) and before `not_a_terminal_status` (`:629`):

```rust
/// The three ANA-2 §4.3 tables behind one name, so `MemStore` and `PgStore` call the same rule
/// (plan D15; the precedent is [`chat_step_status`]).
pub trait TransitionLaw: Copy + std::fmt::Display {
    /// `"item"`, `"run"` or `"run_step"`: the `entity` a refusal names.
    const ENTITY: &'static str;
    /// The table.
    fn can_move_to(self, to: Self) -> bool;
}

impl TransitionLaw for crate::model::Status {
    const ENTITY: &'static str = "item";
    fn can_move_to(self, to: Self) -> bool { Self::can_move_to(self, to) }
}
impl TransitionLaw for RunStatus {
    const ENTITY: &'static str = "run";
    fn can_move_to(self, to: Self) -> bool { Self::can_move_to(self, to) }
}
impl TransitionLaw for crate::model::StepStatus {
    const ENTITY: &'static str = "run_step";
    fn can_move_to(self, to: Self) -> bool { Self::can_move_to(self, to) }
}

/// `Ok(())` when `from → to` is in the table, else the [`StoreError::Constraint`] every CAS
/// returns **before** touching the row. Precedence (plan D14): the caller looks the row up
/// first, so a missing row is `NotFound` even when the pair is also illegal.
pub fn legal_move<T: TransitionLaw>(from: T, to: T) -> Result<()> {
    if from.can_move_to(to) {
        Ok(())
    } else {
        Err(StoreError::Constraint(illegal_move(T::ENTITY, from, to)))
    }
}

/// The refusal text of an illegal move, one sentence for the three tables.
#[must_use]
pub fn illegal_move(entity: &'static str, from: impl std::fmt::Display, to: impl std::fmt::Display) -> String {
    format!("{entity}.status `{from}` cannot move to `{to}` (ANA-2 §4.3)")
}
```

Order every CAS writer honours, on both stores: **(1)** row lookup → `NotFound`; **(2)** `legal_move` → `Constraint`, no write; **(3)** `from` mismatch → `Ok(false)`, no write; **(4)** update → `Ok(true)`. `transition` (`:150`) keeps its signature; its doc gains one sentence: "An illegal `(from, to)` is `Constraint` without an update; a missing row is `NotFound` first (plan D14)."

**`ReadStore` (`:63-127`)** — five methods appended after `project` (`:126`), in the file's style:

```rust
    /// One `run` row, or `None`. Mirrored (`cache_migrations/0001_mirror.sql:103`), so every
    /// backend answers it.
    async fn run(&self, id: RunId) -> Result<Option<Run>>;

    /// Every `run_step` of a run in `(position, attempt, fanout_index)` order — the judge
    /// (`fanout_index = -1`) sorts before its candidates. Empty for an unknown run: a list read is
    /// total, like [`documents`](ReadStore::documents).
    async fn run_steps(&self, run: RunId) -> Result<Vec<RunStep>>;

    /// The step's `run_step_tree` rows in `repo_id` order; empty for an unknown step.
    async fn step_trees(&self, step: StepId) -> Result<Vec<RunStepTree>>;

    /// The step's `run_step_commit` rows in `repo_id` order; empty for an unknown step.
    async fn step_commits(&self, step: StepId) -> Result<Vec<RunStepCommit>>;

    /// ANA-2 §4.2's resolver, which [`documents_of_kinds`](ReadStore::documents_of_kinds) is not
    /// (plan D2): per requested kind, the latest document of the item whose producing step is not
    /// a fan-out loser (`selected IS NOT FALSE`), preferring one produced by a step of `run`;
    /// hand-written documents rank after any run's output (`ORDER BY (s.run_id = $run) DESC
    /// NULLS LAST, d.version DESC`). One entry per kind in `kinds` order, a missing kind carried
    /// as `document: None`. An empty `kinds` means every kind the item has, in kind byte order.
    async fn resolve_inputs(
        &self,
        item: ItemId,
        run: RunId,
        kinds: &[String],
    ) -> Result<Vec<ResolvedInput>>;
```

**`WriteStore` (`:139-609`)** — eighteen methods appended after `delete_project` (`:608`), in ANA-2 §8's order, each doc stating outcome, refusals and transaction shape:

```rust
    // ---- ANA-2 §8: graph runs (MOD-4 milestone 1) --------------------------------------------

    /// Inserts a `kind = 'graph'` run at `queued` with its snapshot and moves the item to
    /// `queued` under the §4.3 law, in one transaction (plan D6).
    ///
    /// # Errors
    /// `NotFound { entity: "item" }`; `Constraint` when the item's status cannot move to `queued`
    /// (only `open` and `failed` can), when `id` already exists, or on any foreign key.
    async fn create_run(&self, new: NewRun) -> Result<Run>;

    /// ANA-2 §4.7's admission, one transaction: lock the box row, count its live runs against
    /// `BoxSettings::max_concurrent_items` (else `app_setting`, else
    /// [`DEFAULT_MAX_CONCURRENT_ITEMS`]), refuse any overlap between `run.repo_scope` and a live
    /// run's scope on the same box, then move the run `queued → running` with
    /// `executing_box_id = box`, `started_at = at`, `lease_box_id = box`, `lease_owner = owner`,
    /// `lease_expires_at = lease_until`, and the item `queued → in_progress`.
    ///
    /// `Ok(false)` when the slot or the overlap check refuses, when the run is not `queued`, or
    /// when its `target_box_id` is not `box_id`; nothing is written in any of those cases.
    ///
    /// # Errors
    /// `NotFound` for an unknown run (`"run"`) or box (`"box"`), the run looked up first.
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<bool>;

    /// ANA-2 §4.9's heartbeat: `UPDATE run SET lease_expires_at = until WHERE id = run AND
    /// lease_owner = owner`. `Ok(false)` = zero rows = abandon; the run exists but is not ours.
    ///
    /// # Errors
    /// `NotFound { entity: "run" }`.
    async fn refresh_lease(&self, run: RunId, owner: Uuid, until: DateTime<Utc>) -> Result<bool>;

    /// ANA-2 §4.9's sweep: every `running` run whose `executing_box_id` is `box_id` and whose
    /// lease is `NULL` or expired at `now` becomes ours (`lease_owner = owner`,
    /// `lease_expires_at = lease_until`). Returns the adopted rows in `queued_at` order; empty
    /// when nothing was abandoned. A box that does not exist adopts nothing (`Ok(vec![])`).
    async fn adopt_runs(
        &self,
        box_id: BoxId,
        owner: Uuid,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Vec<Run>>;

    /// Inserts a step at `pending`. `fanout_index = -1` is the judge and is accepted.
    ///
    /// # Errors
    /// `Constraint` on an unknown run or agent (foreign key, like
    /// [`append_events`](WriteStore::append_events)), on a duplicate id, or on a repeat of
    /// `(run_id, position, attempt, fanout_index)`.
    async fn create_step(&self, new: NewRunStep) -> Result<RunStep>;

    /// §4.3 compare-and-set on `run.status`; `started_at = COALESCE(started_at, at)` when `to` is
    /// `running`, `finished_at = COALESCE(finished_at, at)` when `to` is terminal. Same contract
    /// as [`transition`](WriteStore::transition): `Ok(false)` on a stale `from`, `Constraint`
    /// on an illegal pair without an update, `NotFound` first (plan D14).
    async fn transition_run(
        &self,
        run: RunId,
        from: RunStatus,
        to: RunStatus,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// The `run_step` twin of [`transition_run`](WriteStore::transition_run).
    async fn transition_step(
        &self,
        step: StepId,
        from: StepStatus,
        to: StepStatus,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// Writes the settle columns of [`StepOutcome`]; never `status`.
    ///
    /// # Errors
    /// `NotFound { entity: "run_step" }`.
    async fn finish_step(&self, step: StepId, outcome: StepOutcome) -> Result<()>;

    /// The four `R-ORCH-2` answers, a CAS on `awaiting_approval`: `Approved | Skipped → done`,
    /// `Rejected → failed`, `Retried → superseded`, with `gate_outcome`, `gate_note` and
    /// `finished_at = COALESCE(finished_at, at)`. `Ok(false)` when the step is not awaiting.
    ///
    /// # Errors
    /// `NotFound { entity: "run_step" }`.
    async fn answer_gate(
        &self,
        step: StepId,
        outcome: GateOutcome,
        note: Option<String>,
        at: DateTime<Utc>,
    ) -> Result<bool>;

    /// ANA-2 §4.5's bookkeeping, one transaction (plan D6): among the steps of
    /// `(run, position, attempt)` with `fanout_index >= 0`, the winner becomes
    /// `selected = true, status = done`; every other candidate becomes `selected = false` and,
    /// when its status is `pending`, `awaiting_approval` or `done`, `superseded` (a `failed` or
    /// `cancelled` loser keeps its status); the judge row (`fanout_index = -1`), when there is
    /// one, becomes `done` with `gate_note = reason`.
    ///
    /// # Errors
    /// `NotFound { entity: "run_step" }` for `winner`; `Constraint` when `winner` is not a
    /// candidate of that `(run, position, attempt)` or is not `awaiting_approval | done`. Either
    /// refusal leaves every row untouched.
    async fn select_fanout(
        &self,
        run: RunId,
        position: i32,
        attempt: i32,
        winner: StepId,
        reason: Option<String>,
    ) -> Result<()>;

    /// §4.4's loop half: `pending | awaiting_approval | done → superseded`, no other status.
    ///
    /// # Errors
    /// `NotFound`; `Constraint` from any other status (the law's table, not a stale CAS).
    async fn supersede_step(&self, step: StepId) -> Result<()>;

    /// Upserts `run_step_tree` rows on the table's `(run_step_id, repo_id)` key; an empty slice
    /// checks the step and writes nothing.
    ///
    /// # Errors
    /// `NotFound { entity: "run_step" }`; `Constraint` when a row's `run_step_id` is not `step`
    /// or names an unknown repo.
    async fn upsert_step_tree(&self, step: StepId, trees: &[RunStepTree]) -> Result<()>;

    /// `R-ORCH-11`'s two hashes, upserted on `(run_step_id, repo_id)`; same refusals as
    /// [`upsert_step_tree`](WriteStore::upsert_step_tree).
    async fn record_commits(&self, step: StepId, commits: &[RunStepCommit]) -> Result<()>;

    /// Inserts the document at `max(version) + 1` for `(item, kind)` under the item's row lock
    /// (plan D6), so two writers cannot allocate the same version.
    ///
    /// # Errors
    /// `NotFound { entity: "item" }`; `Constraint` on a duplicate id or an unknown
    /// `produced_by_step_id` / `created_by`.
    async fn write_document(&self, new: NewDocument) -> Result<Document>;

    /// §4.8's promotion, one transaction: the step `failed | awaiting_approval →
    /// awaiting_approval` with `promoted_at = at`; its run `running → awaiting_approval`
    /// (already `awaiting_approval` is fine); its item `in_progress → awaiting_approval`
    /// (already `awaiting_approval` is fine).
    ///
    /// # Errors
    /// `NotFound`; `Constraint` when the step's status is any other, or its run is terminal.
    async fn promote_step(&self, step: StepId, at: DateTime<Utc>) -> Result<()>;

    /// `status = failed, failure = failure, finished_at = COALESCE(finished_at, at)` from any
    /// non-terminal status (the law allows `queued | running | awaiting_approval → failed`).
    ///
    /// # Errors
    /// `NotFound`; `Constraint` when the run is already terminal.
    async fn fail_run(&self, run: RunId, failure: &str, at: DateTime<Utc>) -> Result<()>;

    /// `R-TUI-9`'s three effects, one transaction (plan D6): the summary document at its next
    /// version, the commits upserted, the item moved to `closed` under the law with `closed_at`
    /// set. Refused while any run of the item is active.
    ///
    /// # Errors
    /// `NotFound { entity: "item" }`; `Constraint` when a run of the item is
    /// `queued | running | awaiting_approval`, when `summary.kind != "summary"`, when
    /// `summary.item_id != item`, or when the item's status cannot move to `closed` (only
    /// `blocked`, `failed` and `done` can). Any refusal writes nothing.
    async fn close_out(
        &self,
        item: ItemId,
        summary: NewDocument,
        commits: &[RunStepCommit],
    ) -> Result<Document>;

    /// Inserts an `item_note`; the refusal notes of ANA-2 invariant 7.
    ///
    /// # Errors
    /// `Constraint` on an unknown item, author, box or step (foreign keys) or a duplicate id.
    async fn add_note(&self, note: NewNote) -> Result<Note>;
```

`Uuid` is `uuid::Uuid`, already a dependency of `htui-core` (`ids.rs`). Add the `use` lines the file needs (`RunStatus` is already imported for `chat_step_status`).

**`DeleteReach` (`:744-792`)**: append `/// \`run_step_tree\` rows. pub run_step_trees: usize,` immediately after `run_step_commits`. Doc `:739-743` becomes: "`phase_agents` and `command_runs` are `0` on `MemStore`, which holds neither table, and `0` on the demo database, which seeds neither; `run_step_commits` and `run_step_trees` are counted on both since MOD-4." Construction sites: `mem.rs` `delete_workspace` and `delete_project` literals (`:2575-2576` region), the `pg/write.rs` delete CTE (`:166-169` region — add `t AS (SELECT run_step_id FROM run_step_tree WHERE run_step_id IN (SELECT id FROM s))` and its count; T2), and `conformance.rs:2271-2277`'s tuple (a seventh `0`).

### 2.9 `store/mem.rs`

**`State` (`:53-119`)**: after `steps` (`:116`):

```rust
    /// `run_step_tree` rows keyed by the table's primary key, so `step_trees` is in `repo_id`
    /// order for free. No fixture loads it.
    step_trees: BTreeMap<(StepId, RepoId), RunStepTree>,
    /// `run_step_commit` rows, same key. MOD-4 is the first writer; `DeleteReach.run_step_commits`
    /// counts it.
    step_commits: BTreeMap<(StepId, RepoId), RunStepCommit>,
```

`MemStore::demo()` (`:131`) loads nothing into either (the fixture holds none, F-P). `delete_project` reach (`:2575-2576`): `run_step_commits` and the new `run_step_trees` are counted by retaining rows whose step belongs to a deleted run, mirroring how `run_steps` is counted there; `command_runs: 0` stays.

**Trait reads**: `run` = `state.runs.get(&id).cloned()`; `run_steps` = filter `state.steps` by `run_id`, sort by `(position, attempt, fanout_index)`; `step_trees`/`step_commits` = `range((step, RepoId::default())..)` bounded on the step, or a filter + the map's order; `resolve_inputs` = for each kind, candidates `state.documents` with `item_id == item && kind == kind` whose `produced_by_step_id` is `None` or names a step with `selected != Some(false)`; rank by `(rank, Reverse(version))` where `rank = 0` if the producing step's `run_id == run`, `1` if produced by another run, `2` if hand-written — this is `ORDER BY (s.run_id = $run) DESC NULLS LAST, d.version DESC` spelled out; empty `kinds` = every kind of the item in byte order (reuse whatever `documents_of_kinds` does for its empty case).

**Writers**, each a single `self.write(|state| …)` (`:432-435`) so the five transactions of D6 are atomic by construction; no `.await` under the guard (`mem.rs:3-7`). The per-method rules are the doc comments of §2.8, in `impl State` beside `State::transition` (`:1046-1067`) and `State::start_chat_run` (`:1264-1326`), which are the templates for a CAS and for a two-row insert. Notes that are not obvious from the docs:

- `create_run`: look the item up → `NotFound`; `legal_move(item.status, Status::Queued)`; insert `Run { kind: RunKind::Graph, status: Queued, executing_box_id: None, graph_snapshot: Some(serde_json::to_value(&new.graph_snapshot)…), repo_scope: new.repo_scope, lease_*: None, started_at/finished_at/failure: None, queued_at: new.queued_at, updated_at: now }`; then `state.transition(item, item.status, Queued)` (the existing helper, which never bumps `version`). Duplicate `id` → `Constraint`, checked before any mutation.
- `claim_run`: run lookup → `NotFound`; box lookup → `NotFound`; `status != Queued || target_box_id != box_id` → `Ok(false)`; `limit = BoxSettings::deserialize(box.settings).max_concurrent_items` else `state.app_settings["max_concurrent_items"]` as `u32` else `DEFAULT_MAX_CONCURRENT_ITEMS`; `running = runs.values().filter(|r| r.executing_box_id == Some(box_id) && matches!(r.status, Running | AwaitingApproval)).count()`; `running >= limit` → `Ok(false)`; overlap = any such run whose `repo_scope` intersects the claimed run's (`Vec` intersection; an empty scope intersects nothing — hazard H-10) → `Ok(false)`; else write the run fields of the doc and, when `item.status == Queued`, move the item `queued → in_progress` through `state.transition` (a stale item status is not a refusal: the run is what is being claimed).
- `refresh_lease`: `lease_owner` lives in a **parallel private map** `lease_owners: HashMap<RunId, Uuid>` on `State` (it is not a `Run` field, F-S); `Ok(false)` when absent or different.
- `adopt_runs`: filter, set, collect, sort by `queued_at`.
- `select_fanout`: gather candidates first, validate everything, then mutate; return `Constraint` before the first write.
- `write_document`: `version = documents.values().filter(item, kind).map(version).max().unwrap_or(0) + 1`; foreign keys checked with the file's existing `require_*` helpers (`require_author` is at `:2714`).
- `close_out`: `any(runs of item is_active)` → `Constraint(format!("item {item} has a live run {run_id}"))` naming the first live run; `summary.kind != "summary"` → `Constraint`; `legal_move(item.status, Closed)`; then document, commits, `state.transition(item, status, Closed)` which sets `closed_at` (mirror `pg/write.rs:442-475`'s `CASE WHEN $3 IN ('done','closed')`; check `State::transition` `:1046-1067` does the same — it must, since `pg_criteria` already pins `closed_at`).
- `add_note`: `require_author(created_by)`, item exists, optional step exists, duplicate id → `Constraint`; insert; return the `Note`.

**Eleven inherent reads** (F-N), `pub async fn` on `MemStore` beside `project_settings` (the existing inherent the trait doc at `traits.rs:121-124` names), each a `self.read(|state| …)`:

| Method | Mem body |
|---|---|
| `step_graph(&self, id: StepGraphId) -> Result<Option<StepGraph>>` | `state.graphs.get(&id).cloned()` |
| `phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>>` | `Ok(Vec::new())` — no table |
| `prompt_template(&self, project: ProjectId, name: &str, version: Option<i32>) -> Result<Option<PromptTemplate>>` | filter templates by project + name; `version` `Some` = that row, `None` = max version |
| `resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>>` | item → `step_graph_id` else the kind's `default_graph_id`; phases in position order; `agents: Vec::new()` |
| `agent_boxes(&self, box_id: BoxId) -> Result<Vec<AgentBox>>` | filter the existing agent-box rows |
| `box_row(&self, id: BoxId) -> Result<Option<BoxRow>>` | `state.boxes.get(&id).cloned()` |
| `repo_paths(&self, box_id: BoxId) -> Result<Vec<RepoBoxPath>>` | filter the existing repo-box-path rows by box |
| `ready_items(&self, scope: &Scope, box_id: BoxId) -> Result<Vec<ItemSummary>>` | the existing `items()` with `ready = true` filter, then drop every item whose `required_tags` is not a subset of `probed_tags ∪ declared_tags` of the box; order as `items()` orders |
| `missing_tags(&self, item: ItemId, box_id: BoxId) -> Result<Vec<String>>` | `required_tags − (probed ∪ declared)`, byte order; `NotFound` for either id |
| `active_runs_on_box(&self, box_id: BoxId) -> Result<usize>` | count `executing_box_id == Some(box_id) && matches!(status, Running \| AwaitingApproval)` |
| `overlapping_runs(&self, scope: &[RepoId]) -> Result<Vec<Run>>` | active runs whose `repo_scope` intersects `scope`, `queued_at` order |

### 2.10 `fixtures.rs` — D13 and the F-P growth

**D13.** `attempt: 0 → 1` at `:1296` (the `STEP_R2_PRD` literal) and `:1330` (inside `done_step`, four rows). `graph_snapshot: None → Some(demo_graph_snapshot())` at `:1234` (RUN_1) and `:1251` (RUN_2), where

```rust
/// The §5.1 snapshot both graph runs carry: the `htui` graph's phases as `phases()` seeds them,
/// one candidate per phase (the agent its `RUN_1` step ran on), no judge, default settings,
/// `topology` a fixed literal because milestone 1 has no builder to compute one.
fn demo_graph_snapshot() -> serde_json::Value {
    serde_json::to_value(GraphSnapshot { v: GraphSnapshot::V, graph: SnapshotGraph { id: <the htui graph id>, name: <its name>, is_override: false }, topology: "sha256:demo".to_owned(), mode: RunMode::Manual, phases: <one SnapshotPhase per phases() row of that graph: gate_effective = gate, isolation = phase.isolation.unwrap_or(Isolation::Worktree), deadline_seconds: Some(7200), template: SnapshotTemplate { name: phase.template_name.clone(), version: phase.template_version.unwrap_or(1) }, candidates: vec![SnapshotCandidate { agent_id, agent_name, model }] taken from the matching RUN_1 step, judge: None>, settings: SnapshotSettings { default_isolation: Isolation::Worktree, per_token_cap_run: None, per_token_cap_batch: None, max_fan_out: 4, max_agents_per_run: 6 } }).expect("a literal this module owns")
}
```

Written as real code by the implementer; the point is that it is built from the **typed** `GraphSnapshot` (D13's correction: there is no `graph.rs` builder).

**F-P growth** — new ids in `mod ids` (`:104-…`, free slots verified): `RUN_3: RunId = (class::RUN, 2)` ("finished fan-out run of `HTUI_ANA_1`: one `research` phase, two candidates, a winner and a loser"), `STEP_R3_RESEARCH_A: StepId = (class::RUN_STEP, 5)` (winner), `STEP_R3_RESEARCH_B: StepId = (class::RUN_STEP, 6)` (loser), `DOC_ANA_1_RESEARCH_V2: DocumentId = (class::DOCUMENT, 6)`, `DOC_ANA_1_RESEARCH_V3: DocumentId = (class::DOCUMENT, 7)`.

Rows:

| Row | Literal |
|---|---|
| `RUN_3` (`runs()`, `:1223`) | `Run { id: RUN_3, project_id: PROJECT_HTUI, item_id: Some(HTUI_ANA_1), kind: Graph, mode: Manual, status: Done, target_box_id: BOX, executing_box_id: Some(BOX), graph_snapshot: Some(demo_graph_snapshot()), started_by: USER, queued_at: demo_at(1, 14), started_at: Some(demo_at(1, 14)), finished_at: Some(demo_at(1, 17)), failure: None, repo_scope: vec![], lease_box_id: None, lease_expires_at: None, updated_at: demo_at(1, 17) }` |
| `STEP_R3_RESEARCH_A` (`steps()`) | `RunStep { id, run_id: RUN_3, position: 0, attempt: 1, fanout_index: 0, phase_name: "research", agent_id: Some(AGENT_CLAUDE), model: Some("opus"), status: Done, gate_outcome: Some(Approved), gate_note: None, selected: Some(true), exit_code: Some(0), …None…, started_at: Some(demo_at(1, 14)), finished_at: Some(demo_at(1, 15)), verify_outcome: None, verify_exit_code: None, promoted_at: None, updated_at: demo_at(1, 17) }` |
| `STEP_R3_RESEARCH_B` | same, `fanout_index: 1`, `agent_id: Some(AGENT_AGY)`, `model: Some("default")`, `status: Superseded`, `gate_outcome: None`, `selected: Some(false)`, `finished_at: Some(demo_at(1, 16))` |
| `DOC_ANA_1_RESEARCH_V2` (`documents()`, after `:1150`) | `document(DOC_ANA_1_RESEARCH_V2, HTUI_ANA_1, "research", 2, "Research: store topology (claude)", Some(STEP_R3_RESEARCH_A), 15)` |
| `DOC_ANA_1_RESEARCH_V3` | `document(DOC_ANA_1_RESEARCH_V3, HTUI_ANA_1, "research", 3, "Research: store topology (agy)", Some(STEP_R3_RESEARCH_B), 16)` |

Why these rows break nothing (checked at HEAD): `documents_ordered_by_version` (`conformance.rs:895-925`) and `documents_of_kinds_latest_per_kind_in_order` (`:3734`) read `HTUI_FEAT_1`; `notes_ordered_by_created_at` reads FEAT-1; `project_delete_takes_everything_and_says_so` asserts `runs`/`run_steps`/`documents` only `> 0` (`:2280-2293`); `migrations.rs:878-880` derive counts from `data.documents/runs/steps.len()`; `backlog__detail_*.snap` and `replay__runs_step_selected.snap` render FEAT-1; `prompt_preview__preview_ana_2.snap` reads ANA-2 and its upstream ANA-1 **summary**, which is unchanged; no `.snap` renders ANA-1's runs or research versions. `hierarchy__demo.snap` renders no repo (none is added). Update the `run`/`run_step`/`document` doc comments on `runs()` (`:1222`), `steps()` (`:1262`) and `documents()` to name the third run.

### 2.11 `store/conformance.rs`

**D4 amendments (three edits, three non-edits).** Verified pairs at HEAD:

| Site | Today | Becomes | Why |
|---|---|---|---|
| `:537` | `Open → Queued`, asserted true | unchanged | legal |
| `:558` | `Open → Done`, asserted `!stale` (Ok(false)) | `transition(id, Status::Queued, Status::InProgress)` against the row that `:537` already moved to `Queued` **and then moved on** — insert one legal `Queued → Open` at `:537`'s successor first, so `Queued → InProgress` is legal-but-stale | The case proves a **stale `from` is `Ok(false)`**; the replacement pair must be legal so the law does not pre-empt the staleness check (plan D4) |
| `:580` | `Queued → Done`, asserted true | `Queued → InProgress`, asserted true, then `InProgress → Done`, asserted true, so the row still ends `Done` for the version assertion that follows | `queued` reaches only `in_progress/open/blocked` |
| `:596` | `Done → Open` | unchanged | legal (reopen) |
| `:649` (`no_delete_path`) | `Open → Closed`, asserted true | `Open → Blocked` then `Blocked → Closed`, both asserted true; the case's later assertions on `closed` stand | `open` does not reach `closed` |
| `:872` | `Blocked → Open` | unchanged | legal |

`pg_criteria.rs:487` (`Open → Done`, Postgres version twin) → `Open → Queued`; `:504` and `:528` unchanged; `:535` (`Open → Done` on an unknown id, asserted `NotFound`) **unchanged** — it is now D14's precedence pin for Postgres and its doc says so. `writer_buffered.rs:251` unchanged (D15).

**Fourteen cases.** Signature and style of `run_case` (`:79`) and `run_all` (`:146-155`); every assertion message starts with the case name; per-backend twins named in the doc comment may only cite `conformance`, `mem` or `pg_criteria` (`every_cross_referenced_test_name_exists`, `:4104-4112`). `CASES` (`:31-68`) grows by eleven after `project_create_seeds_the_catalogue`; `READ_CASES` (`:169-176`) by three after its last entry; `run_case`/`run_read_case` gain one arm each.

| # | Case | Kind | Fixture rows it leans on | Asserts |
|---|---|---|---|---|
| 37 | `run_create_moves_the_item` | W | `HTUI_ANA_2` (open), `HTUI_CLEAN_1` (failed), `HTUI_FEAT_1` (in_progress) | (a) `create_run` on ANA-2 returns `status Queued, kind Graph, item_id Some, executing_box_id None, repo_scope == new.repo_scope, lease_* None, graph_snapshot Some` that decodes to the `GraphSnapshot` passed with `v == 1`; (b) `item(ANA_2).status == Queued` and its `version` unchanged; (c) `run(id)` equals the returned row; (d) same id again → `Constraint`; (e) unknown item → `NotFound{entity:"item"}`; (f) FEAT-1 (`in_progress`) → `Constraint` **and** `run(that id)` is `None` (transaction); (g) CLEAN-1 (`failed`) → Ok, item `Queued` |
| 38 | `claim_run_admits_one_and_refuses_the_second` | W | `BOX`, `PROJECT_HTUI`, two open/failed items + two minted via `mint_item` | `create_repo(new_repo(PROJECT_HTUI, "core", true))` as `R`; runs A (scope `[R]`), B (scope `[R]`), C (scope `[]`), D (scope `[]`) on four distinct items, all targeting `BOX`; owner `O1`. (a) claim A → `true`; `run(A)`: `Running`, `executing_box_id Some(BOX)`, `started_at == at`, `lease_box_id Some(BOX)`, `lease_expires_at == until`; `item(A.item).status == InProgress`; (b) claim B → `false` (overlap), `run(B)` and its item unchanged; (c) claim C → `true`; (d) claim D → `false` (`DEFAULT_MAX_CONCURRENT_ITEMS` = 2 running); (e) claim A again → `false` (not queued); (f) run targeting another box (`BoxId::new()` is unknown → `NotFound{"box"}` looked up **after** the run: assert with an existing run and unknown box → `NotFound{"box"}`; with unknown run and unknown box → `NotFound{"run"}`) |
| 39 | `lease_refresh_is_a_cas_on_owner` | W | as 38, one run | claim with `O1`; (a) `refresh_lease(A, O1, until2)` → `true`, `run(A).lease_expires_at == until2`; (b) `refresh_lease(A, O2, until3)` → `false`, unchanged; (c) unknown run → `NotFound`; (d) `adopt_runs(BOX, O2, now = until2 - 1s, …)` → empty; (e) `adopt_runs(BOX, O2, now = until2 + 1s, until4)` → `[A]` with `lease_expires_at == until4`; (f) now `refresh_lease(A, O1, …)` → `false` and `(A, O2)` → `true`; (g) `adopt_runs(BoxId::new(), …)` → empty |
| 40 | `step_create_and_transition_law` | W | `RUN_2` (queued, its step now `(0, 1, 0)`) | (a) `create_step` at `(1, 1, 0)` → row `Pending`, every settle column `None`, equals `run_steps(RUN_2)[1]`; (b) `(0, 1, 0)` again → `Constraint`; (c) unknown run → `Constraint`; (d) `fanout_index: -1` at `(1, 1, -1)` → Ok and `run_steps` lists it **before** `(1, 1, 0)`; (e) `transition_step(s, Pending, Running, t1)` → `true`, `started_at == Some(t1)`; (f) `(s, Running, Done, t2)` → `true`, `finished_at == Some(t2)`; (g) `(s, Done, Running, t3)` → `Constraint`, row unchanged; (h) `(s, Pending, Running, t3)` → `Ok(false)` (stale); (i) `transition_run(RUN_2, Queued, Running, t1)` → `true`, `started_at`; `(Running, Done, t2)` → `true`, `finished_at`; `(Done, Queued, t3)` → `Constraint`; (j) `supersede_step` on a fresh `pending` step → Ok, `Superseded`; on the `Done` step `s` → Ok; on a `Running` step → `Constraint`; on `StepId::new()` → `NotFound`; (k) `item(HTUI_FEAT_3)` is untouched by every run/step move (only `claim_run`, `promote_step`, `create_run`, `close_out` touch the item) |
| 41 | `finish_step_records_the_settle` | W | `STEP_R2_PRD` | (a) full `StepOutcome` → `run_steps(RUN_2)[0]` carries every field, `status` still `Pending`; (b) second call with `usage: None, trim_record: None, verify_outcome: None` keeps `usage`/`trim_record`, clears `verify_outcome`; (c) unknown → `NotFound`; (d) `runs(HTUI_FEAT_3)[0].steps[0]` (`RunStepSummary`) has `usage`, `exit_code`, `verify_outcome`, `selected`, `promoted_at` equal to the `RunStep`'s and `agent_name == Some("claude")` — the writer-side builders pin |
| 42 | `gate_answers_write_their_outcome` | W | `RUN_2` | four steps driven `Pending → Running → AwaitingApproval`; (a) `Approved` → `true`, `Done`, `gate_outcome Some(Approved)`, `gate_note`, `finished_at == at`; (b) same step again → `false`; (c) `Rejected` → `Failed`; (d) `Retried` → `Superseded`; (e) `Skipped` → `Done`; (f) unknown → `NotFound`; (g) `promote_step` on the `Failed` step of (c) with run `Running` → Ok: step `AwaitingApproval`, `promoted_at == at`, `run(RUN_2).status == AwaitingApproval`, `item(FEAT_3)` moved to `AwaitingApproval` only if it was `InProgress` (drive it there first via `claim_run` or `transition`); `promote_step` on a `Done` step → `Constraint`; (h) `add_note(NewNote { via_step_id: Some(step), .. })` → returned `Note` equals `notes(FEAT_3).last()`; unknown author → `Constraint`; unknown item → `Constraint` |
| 43 | `select_fanout_is_one_transaction` | W | `RUN_2` | steps `(1,1,0)`, `(1,1,1)`, `(1,1,2)` driven to `AwaitingApproval`, `(1,1,-1)` driven to `Running`, `(1,1,2)` then `Retried` → `Failed`; (a) `select_fanout(RUN_2, 1, 1, w = (1,1,0), Some("shorter diff"))` → Ok: winner `selected Some(true)`, `Done`; `(1,1,1)` `Some(false)`, `Superseded`; `(1,1,2)` `Some(false)`, still `Failed`; judge `Done`, `gate_note Some("shorter diff")`; (b) `write_document` produced by the winner (`v1`) and by the loser (`v2`) of kind `"implementation"` on FEAT-3, then `resolve_inputs(FEAT_3, RUN_2, ["implementation"])` → the winner's `v1` while `documents_of_kinds` → `v2` (D2's contrast); (c) a winner from another position → `Constraint` and every row unchanged; (d) `StepId::new()` → `NotFound`; (e) `runs(FEAT_3)` summaries show `selected` |
| 44 | `trees_and_commits_round_trip` | W | `STEP_R2_PRD`, `create_repo` ×2 | (a) `upsert_step_tree(step, [t1(R1), t2(R2)])` → `step_trees(step) == [t(R1), t(R2)]` ordered by `repo_id` regardless of input order; (b) upsert `t1` with `dirty: true` → still two rows, `dirty` updated; (c) a row whose `run_step_id != step` → `Constraint`, nothing written; (d) unknown repo → `Constraint`; (e) unknown step → `NotFound`; (f) empty slice on unknown step → `NotFound`, on a real step → Ok; (g)–(l) the same six for `record_commits`/`step_commits` with `after_hash: None` then `Some`; (m) `delete_project(PROJECT_HTUI)` report: `run_step_commits == 2`, `run_step_trees == 2` (D12's "MemStore holds rows it never held") and both reads now empty |
| 45 | `write_document_allocates_its_version` | W | `HTUI_FEAT_1` (plan v1, v2), `HTUI_ANA_2` | (a) `"plan"` on FEAT-1 → `version 3`; again → `4`; new kind → `1`; (b) `documents(FEAT_1)` lists them; (c) unknown item → `NotFound`; (d) unknown `produced_by_step_id` → `Constraint`; (e) duplicate id → `Constraint`; (f) **preference leg**: run B on ANA-2, `transition(ANA_2, Queued, Open)`, run C on ANA-2, steps SB, SC; `"research"` by SB → v1, by SC → v2, hand-written → v3; `resolve_inputs(ANA_2, B, ["research"]) == v1`, `(…, C, …) == v2`, `(…, RUN_1, …) == v2` (another run's output outranks hand-written, higher version wins among equals), `documents_of_kinds(ANA_2, ["research"]) == v3`, `resolve_inputs(ANA_2, B, ["research", "nope"])` has `document: None` for `"nope"` |
| 46 | `close_out_refuses_a_live_run` | W | `HTUI_FEAT_3` (RUN_2 queued), `HTUI_FEAT_1`, `HTUI_ANA_2` | (a) FEAT-3 → `Constraint` naming RUN_2; `documents(FEAT_3)` unchanged; item unchanged; (b) `transition(FEAT_1, InProgress, Done)` then `close_out(FEAT_1, summary(kind "summary"), [commit on STEP_IMPL over a created repo])` → `Document { version: 1 }`; `item(FEAT_1)` `Closed` with `closed_at Some`; `step_commits(STEP_IMPL)` has the row; (c) ANA-2 (`open`) → `Constraint` (law); (d) `kind: "plan"` → `Constraint`; (e) `summary.item_id != item` → `Constraint`; (f) unknown item → `NotFound` |
| 47 | `illegal_transitions_are_constraint` | W | `HTUI_ANA_2`, `RUN_2`, `STEP_R2_PRD` | (a) for each of `Status::ALL × Status::ALL`, `RunStatus::ALL²`, `StepStatus::ALL²` where `!from.can_move_to(to)`: the seam returns `Constraint` **when the row's status is `from`** (drive one row per enum through a few representative `from`s: `Open`, `Queued`, `Done`; `Queued`, `Running`; `Pending`, `Running`, `Done`) and the row is byte-identical after (status, `updated_at`); (b) D14: `transition(ItemId::new(), Queued, Done)`, `transition_run(RunId::new(), Done, Queued, at)`, `transition_step(StepId::new(), Done, Pending, at)` → `NotFound`, not `Constraint`; (c) `fail_run(RUN_2, "boom", at)` → Ok, `Failed`, `failure Some("boom")`, `finished_at == at`; again → `Constraint`; `RunId::new()` → `NotFound`; (d) the `Constraint` message contains both status texts (so the sentence is `illegal_move`'s) |
| R7 | `run_and_steps_round_trip` | R | RUN_1/2/3 | (a) `run(RUN_1)` equals the fixture row incl. `repo_scope == []`, leases `None`, `graph_snapshot` decoding to `GraphSnapshot { v: 1, .. }` with four phases; (b) `run(RUN_2).status == Queued`; `run(RUN_3).item_id == Some(HTUI_ANA_1)`; (c) `run(RunId::new()) == None`; (d) `run_steps(RUN_1)` ids `[STEP_PRD, STEP_PLAN, STEP_IMPL, STEP_REVIEW]`, all `attempt == 1`, `verify_outcome/promoted_at None`; (e) `run_steps(RUN_3) == [A, B]` with `selected Some(true)/Some(false)`, B `Superseded`; (f) `run_steps(RunId::new())` empty; (g) for every step of `runs(HTUI_ANA_1)` and `runs(HTUI_FEAT_1)`: the `RunStepSummary`'s `usage, selected, exit_code, verify_outcome, promoted_at` equal the `RunStep`'s and `agent_name` is `Some("claude")`/`Some("agy")` per `agent_id` — the three-builder pin on Mem, Pg **and** the mirror |
| R8 | `trees_and_commits_read_back` | R | every fixture step | `step_trees`/`step_commits` → `Ok(vec![])` for each of the seven fixture steps and for `StepId::new()`; never `NotFound`. Doc: "The fixture seeds no `repo`, so no tree or commit row can exist here (blueprint F-P); row content is `trees_and_commits_round_trip` on both writers and `pg_criteria::step_tree_rows_cascade_with_their_step` on Postgres." |
| R9 | `resolve_inputs_prefers_this_run_and_skips_losers` | R | `HTUI_ANA_1`, RUN_3 | (a) `resolve_inputs(ANA_1, RUN_3, ["research", "verdict", "missing"])` → `[research v2 (A's), verdict v1, None]` in that order; (b) `documents_of_kinds(ANA_1, ["research"])` → v3 (the loser's) — D2's contrast; (c) `resolve_inputs(ANA_1, RUN_1, ["research"])` → v2 (another run's selected output outranks hand-written v1); (d) `resolve_inputs(ANA_1, RUN_3, [])` → kinds `research, summary, verdict` in byte order, none `None`; (e) `resolve_inputs(FEAT_1, RUN_1, ["plan"])` → plan v2 (`selected: None` is not a loser); (f) `resolve_inputs(ItemId::new(), RUN_1, ["x"])` → one entry, `document: None` |

`mem_store.rs:37` → `47`, `:44` → `9`. `htui-agent/tests/{acp_conformance.rs:91, fake_conformance.rs:37,52, cli_conformance.rs:144}` compare against `CASES.len()`; confirm with `rg -n 'CASES\.len\(\)|== 36' crates/htui-agent/tests` before touching them — the plan says they need nothing.

### 2.12 T1 commit sequence

| # | Commit | Contents | Green gate |
|---|---|---|---|
| 1 | `feat(htui-core): the three ANA-2 §4.3 tables as const fn` | §2.2 law + `is_terminal`s, §2.3 `Status::can_move_to`, `TransitionLaw`/`legal_move`/`illegal_move`, `transition` guard on `MemStore` (`State::transition` `:1046-1067` gains the lookup → law → stale order), D4's three conformance edits, a unit test per enum in `run.rs`/`item.rs` that lists the sanctioned pairs as a literal table and asserts `can_move_to` against `ALL × ALL` | `cargo test -p htui-core --all-features` |
| 2 | `feat(htui-core): run, step, snapshot and settings types` | §2.2 rest, §2.4, §2.5, §2.6, §2.7; `RunStepSummary`/`BoxInfo`/`ItemSummary`/`StepGraph`/`Run`/`RunStep` builders in `mem.rs`, `seed.rs:193`, `fixtures.rs`, `update.rs:416-428`; `DeleteReach.run_step_trees` | same |
| 3 | `feat(htui-core): ReadStore +5, WriteStore +18 on MemStore` | §2.8 traits, §2.9 `State` + reads + writers + inherent reads; `mem.rs:2575` reach counts | same (suite still 36/6; `htui-store` red from here) |
| 4 | `test(htui-core): fixture attempts are 1-based, runs carry a snapshot, ANA-1 has a fan-out run` | §2.10 | same |
| 5 | `test(htui-core): eleven writer cases for the run seam` | cases 37–47, `CASES` → 47, `run_case` arms, `mem_store.rs:37` | same |
| 6 | `test(htui-core): three read cases for the run seam` | R7–R9, `READ_CASES` → 9, `run_read_case` arms, `mem_store.rs:44` | `cargo test -p htui-core --all-features` and `cargo clippy -p htui-core --all-targets --all-features -- -D warnings` |

---

## 3. T2 — `htui-store`: migrations, `PgStore`, `Backend`, the mirror, the pins

### 3.1 Files

| File | Action |
|---|---|
| `crates/htui-store/migrations/0003_orchestration.sql` | create (§3.2) |
| `crates/htui-store/cache_migrations/0003_orchestration.sql` | create (§3.3) |
| `crates/htui-store/src/pg/rows.rs` | `RunRow` +3, `StepRow` +4, `into_summary` ×2 |
| `crates/htui-store/src/pg/read.rs` | `items` +1 col, `box_info` +3 cols, `runs` step query joins `agent`, 5 trait reads, 11 inherent reads, `step_graph` `query_as!`s +`is_override` |
| `crates/htui-store/src/pg/write.rs` | `transition` guard, 18 writers, delete CTE +`run_step_tree`, `step_graph` `query_as!`s +`is_override` |
| `crates/htui-store/src/backend.rs` | 11 inherent `match self`, 5 trait reads, `orchestration_offline()` |
| `crates/htui-store/src/cache/mod.rs` | `MIRRORED_TABLES` 16 → 17, doc `:31` |
| `crates/htui-store/src/cache/refresh.rs` | const, array, match; `RUN_COLUMNS`, `RUN_STEP_COLUMNS`, two refreshers widened; `refresh_run_step_tree` |
| `crates/htui-store/src/cache/read.rs` | `item_summary_of` +`touched_paths`, `box_info` +3, runs step SQL +join +5 cols, 5 trait reads |
| `crates/htui-store/.sqlx/query-*.json` | regenerate |
| `crates/htui-store/tests/migrations.rs`, `tests/connect.rs`, `tests/pg_conformance.rs`, `tests/cache.rs`, `tests/pg_criteria.rs` | pins and twins (§3.9) |
| `crates/htui/src/ui/tabs/settings/connection.rs`, `crates/htui/tests/connection.rs`, `crates/htui/tests/snapshots/connection__confirm.snap`, `crates/htui/src/store_worker.rs` | the seventeen strings (§3.9) |

### 3.2 `migrations/0003_orchestration.sql` — ready to write

ANA-2 §9's text (`docs/ANA-2.md:1861-1999`) with the header in `0002_agent_probe.sql:1-8`'s voice. Nothing is dropped, renamed or retyped. No `CHECK (fanout_index >= 0)` anywhere.

```sql
-- 0003_orchestration.sql - MOD-4 milestone 1: the ANA-2 (orchestration) amendments to the ANA-9
-- schema, docs/ANA-2.md 9 verbatim. Forward-only (R-STO-5): 0001_init.sql and
-- 0002_agent_probe.sql are never edited. Depends on 0002_agent_probe.sql (ANA-4 9, MOD-2).
-- The cache-mirror companion is cache_migrations/0003_orchestration.sql (plan D7).

-- --------------------------------------------------------------------------------------------
-- 1. step_graph: item override graphs are hidden from the project graph list (ANA-2 §4.1)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph ADD COLUMN is_override BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_step_graph_listed ON step_graph(project_id) WHERE NOT is_override;
COMMENT ON COLUMN step_graph.is_override IS
  'true = an <item.key>-override clone (R-ORCH-1, ANA-2 §4.1); hidden from the R-TUI-8 graph list';

-- --------------------------------------------------------------------------------------------
-- 2. step_graph_phase: the R-ORCH-7 judge and the step deadline (ANA-2 §4.5, §4.2)
-- --------------------------------------------------------------------------------------------
ALTER TABLE step_graph_phase ADD COLUMN judge_agent_id   UUID REFERENCES agent(id);
ALTER TABLE step_graph_phase ADD COLUMN judge_model      TEXT;
ALTER TABLE step_graph_phase ADD COLUMN deadline_seconds INTEGER
  CHECK (deadline_seconds IS NULL OR deadline_seconds > 0);
ALTER TABLE step_graph_phase ADD CONSTRAINT ck_phase_judge_model
  CHECK (judge_model IS NULL OR judge_agent_id IS NOT NULL);

COMMENT ON COLUMN step_graph_phase.judge_agent_id IS
  'R-ORCH-7 judge; NULL = human selection is required whenever fan_out > 1 (ANA-2 §4.5)';
COMMENT ON COLUMN step_graph_phase.judge_model IS
  'model for the judge; overrides agent.default_model (ANA-2 §7)';
COMMENT ON COLUMN step_graph_phase.deadline_seconds IS
  'wall clock for one step attempt; NULL = project.settings.step_deadline_seconds (ANA-2 §4.1)';
COMMENT ON COLUMN step_graph_phase.verify_command IS
  'ANA-2 §4.2: runs after the session and before the gate, in the primary repo tree, through '
  'command_run when the phase advertises it; outcome lands in run_step.verify_outcome';
COMMENT ON COLUMN step_graph_phase.input_kinds IS
  'ANA-2 §4.2: each kind resolves to the latest document version on this item whose producing '
  'step is not a fan-out loser, preferring this run''s own output; a missing kind fails the step';

-- --------------------------------------------------------------------------------------------
-- 3. run: repo scope (R-ORCH-9), the recovery lease, the R-ORCH-11 snapshot guarantee
-- --------------------------------------------------------------------------------------------
ALTER TABLE run ADD COLUMN repo_scope       UUID[] NOT NULL DEFAULT '{}';
ALTER TABLE run ADD COLUMN lease_box_id     UUID REFERENCES box(id);
ALTER TABLE run ADD COLUMN lease_owner      UUID;
ALTER TABLE run ADD COLUMN lease_expires_at TIMESTAMPTZ;

-- NOT VALID: demo and fixture rows predate ANA-2 and carry a NULL snapshot on a graph run.
-- New rows are checked; the backfill is a separate, optional VALIDATE CONSTRAINT.
ALTER TABLE run ADD CONSTRAINT ck_run_graph_snapshot
  CHECK (kind <> 'graph' OR graph_snapshot IS NOT NULL) NOT VALID;

CREATE INDEX idx_run_lease ON run(executing_box_id, lease_expires_at)
  WHERE status IN ('queued','running');
CREATE INDEX idx_run_repo_scope ON run USING GIN (repo_scope);

COMMENT ON COLUMN run.repo_scope IS
  'repos this run may touch, resolved at queue time from item.touched_paths (ANA-2 §4.7)';
COMMENT ON COLUMN run.lease_owner IS
  'per-process id of the orchestrator holding this run; a zero-row lease refresh means abandon '
  '(ANA-2 §4.9)';
COMMENT ON COLUMN run.graph_snapshot IS
  'ANA-2 §5.1: {v, graph, topology, mode, phases[], settings}; carries both gate and '
  'gate_effective so the R-ORCH-6 downgrade is auditable';

-- --------------------------------------------------------------------------------------------
-- 4. run_step: verification outcome (ANA-2 §4.2) and chat promotion (ANA-2 §4.8)
--    No CHECK on fanout_index, ever: -1 is the judge step (ANA-2 risk 12).
-- --------------------------------------------------------------------------------------------
ALTER TABLE run_step ADD COLUMN verify_outcome   TEXT
  CHECK (verify_outcome IN ('pass','fail','unavailable'));
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at      TIMESTAMPTZ;

COMMENT ON COLUMN run_step.verify_outcome IS
  'ANA-2 §4.2: pass | fail | unavailable; unavailable never fails a step';
COMMENT ON COLUMN run_step.exit_code IS
  'the agent process exit code (R-ORCH-11); verification has its own verify_exit_code';
COMMENT ON COLUMN run_step.fanout_index IS
  '0..fan_out-1; -1 = the R-ORCH-7 judge step for this position and attempt (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.selected IS
  'fan-out winner; NULL when fan_out = 1; false on a loser, which is also superseded (ANA-2 §4.5)';
COMMENT ON COLUMN run_step.attempt IS
  '1-based; retry_limit is the number of additional attempts, so attempt <= retry_limit + 1 '
  '(ANA-2 §4.2)';
COMMENT ON COLUMN run_step.isolation_path IS
  'the primary repo tree; every repo in scope has a run_step_tree row (ANA-2 §4.6)';
COMMENT ON COLUMN run_step.promoted_at IS
  'set when the step was promoted to an interactive chat (R-ORCH-5, ANA-2 §4.8)';

-- --------------------------------------------------------------------------------------------
-- 5. run_step_tree: one row per (step, repo), because a project has one or more repos (R-ENT-3)
--    No updated_at and no trigger: like run_step_commit it rides its parent step (ANA-9 §6.2).
-- --------------------------------------------------------------------------------------------
CREATE TABLE run_step_tree (
    run_step_id UUID NOT NULL REFERENCES run_step(id) ON DELETE CASCADE,
    repo_id     UUID NOT NULL REFERENCES repo(id),
    mode        TEXT NOT NULL CHECK (mode IN ('worktree','copy','shared_serialized','local')),
    path        TEXT NOT NULL,                 -- absolute, on the executing box, outside every repo
    base_ref    TEXT NOT NULL,                 -- the commit the tree was created at (ANA-2 §4.6)
    dirty       BOOLEAN NOT NULL DEFAULT false,-- the tree had uncommitted work at step start
    PRIMARY KEY (run_step_id, repo_id)
);
COMMENT ON TABLE run_step_tree IS
  'R-ORCH-8 isolation, per repo; ANA-2 §4.6. A dirty local or shared_serialized tree is never '
  'reset by the recovery sweep (ANA-2 §4.9).';

-- --------------------------------------------------------------------------------------------
-- 6. item: touched_paths become repo-qualified (ANA-2 §4.7). Existing bare globs keep meaning
--    the primary repo, so no data migration is required.
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN item.touched_paths IS
  'R-ORCH-9 declared overlap set: "<repo_name>:<glob>" entries, or a bare glob meaning the '
  'primary repo. Empty means unknown, which overlaps the whole primary repo (ANA-2 §4.7).';

-- --------------------------------------------------------------------------------------------
-- 7. settings contracts (ANA-2 §4.7, §5.2)
-- --------------------------------------------------------------------------------------------
COMMENT ON COLUMN box.settings IS
  'ANA-2 §4.7 BoxSettings: max_concurrent_items (R-ORCH-9), command_limits {class: n} (R-MCP-3). '
  'Every field optional; defaults come from app_setting.';
COMMENT ON COLUMN project.settings IS
  'ANA-2 §4.7 ProjectSettings: token_budget, retention_days, cached_transcript_steps, '
  'keep_raw_events, default_isolation, per_token_cap_run, per_token_cap_batch, '
  'step_deadline_seconds, default_agent_id, judge_agent_id, copy_exclude. Every field optional.';

-- --------------------------------------------------------------------------------------------
-- 8. app_setting defaults (ANA-2 §5.4). Idempotent, so a re-run and the MOD-6 seed agree.
-- --------------------------------------------------------------------------------------------
INSERT INTO app_setting (key, value) VALUES
  ('max_concurrent_items',  '2'::jsonb),
  ('command_limits',        '{"build":1,"test":4,"verify":1}'::jsonb),
  ('default_isolation',     '"worktree"'::jsonb),
  ('step_deadline_seconds', '7200'::jsonb),
  ('max_fan_out',           '4'::jsonb),
  ('max_agents_per_run',    '6'::jsonb),
  ('copy_max_total_bytes',  '21474836480'::jsonb),
  ('lease_ttl_seconds',     '120'::jsonb),
  ('lease_refresh_seconds', '60'::jsonb),
  ('per_token_cap_run',     'null'::jsonb),
  ('per_token_cap_batch',   'null'::jsonb),
  ('scheduler_window',      'null'::jsonb)
ON CONFLICT (key) DO NOTHING;
```

Note on the `COMMENT ON COLUMN` count: 18 (`step_graph.is_override`; five on `step_graph_phase`; three on `run`; seven on `run_step`; `item.touched_paths`; `box.settings`; `project.settings`) plus one `COMMENT ON TABLE`. If `app_setting`'s `value` column is not `jsonb` in `0001_init.sql`, drop the `::jsonb` casts — check the table definition before the first run; `0002_agent_probe.sql` seeds two keys the same way and is the precedent to copy.

### 3.3 `cache_migrations/0003_orchestration.sql` — ready to write

`0001_mirror.sql`'s type mapping (TEXT ids, INTEGER epochs, INTEGER booleans, TEXT JSON). `box` gains nothing (F-D). `lease_owner` is not mirrored (ANA-2 `:2029`).

```sql
-- ------------------------------------------------------------------------------------------------
-- Mirror side of ANA-2 (MOD-4 milestone 1, plan D7 / D9; `docs/ANA-2.md` 9's sketch corrected).
--
-- `run` and `run_step` gain the columns `migrations/0003_orchestration.sql` adds to them, minus
-- `run.lease_owner`, a liveness token for a process that is by definition not running while the
-- mirror is read. `run_step_tree` is the seventeenth mirrored table: no `updated_at` and no cursor
-- of its own, it rides its parent step's `updated_at` exactly as `run_step_commit` does.
-- `box.probed_tags`, `box.declared_tags` and `box.settings` are already in `0001_mirror.sql`.
--
-- `cache_meta.schema_version` moves to 3 through `PgStore::schema_version()`, which forces a
-- full rebuild, so no backfill is written here.
-- ------------------------------------------------------------------------------------------------
ALTER TABLE run      ADD COLUMN repo_scope       TEXT NOT NULL DEFAULT '[]';  -- JSON array of uuids
ALTER TABLE run      ADD COLUMN lease_box_id     TEXT;
ALTER TABLE run      ADD COLUMN lease_expires_at INTEGER;
ALTER TABLE run_step ADD COLUMN verify_outcome   TEXT;
ALTER TABLE run_step ADD COLUMN verify_exit_code INTEGER;
ALTER TABLE run_step ADD COLUMN promoted_at      INTEGER;

CREATE TABLE run_step_tree (
    run_step_id TEXT NOT NULL, repo_id TEXT NOT NULL, mode TEXT NOT NULL,
    path TEXT NOT NULL, base_ref TEXT NOT NULL, dirty INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_step_id, repo_id));
```

### 3.4 `pg/rows.rs`

`RunRow` (`:36-63`): append `repo_scope: Vec<Uuid>`, `lease_box_id: Option<Uuid>`, `lease_expires_at: Option<DateTime<Utc>>` **at the end**; `into_summary` (`:67-84`) ignores them (F-S: `RunSummary` is not widened) — but a new `RunRow::into_run(self) -> Run` is added and used by `run(id)` and every `RETURNING *`-style writer that returns a `Run`. `StepRow` (`:89-121`, positional warning at `:114-117` stands): append `verify_outcome: Option<VerifyOutcome>`, `verify_exit_code: Option<i32>`, `promoted_at: Option<DateTime<Utc>>`, `agent_name: Option<String>`; `into_summary` (`:125-141`) fills the six new `RunStepSummary` fields (`usage: self.usage`, `selected`, `exit_code`, `verify_outcome`, `promoted_at`, `agent_name`). A new `StepRow::into_step(self) -> RunStep` serves `run_steps`.

### 3.5 `pg/read.rs`

- `items` (`:81-94`): append `i.touched_paths` after `i.updated_at`.
- `box_info` (`:700-714`): `"SELECT id AS box_id, hostname, os_family, probed_tags, declared_tags, settings FROM box WHERE id = $1"` — positional, so the order is the struct's (F-C). `probed_tags` and `declared_tags` are both `Vec<String>` and adjacent, so transposing them type-checks and binds silently — the one appended-field case positional binding does not protect. Add a `tests/cache.rs` (or `pg_criteria.rs`) assertion that a box seeded with *distinct* probed and declared tag sets reads back unswapped from both `PgStore` and `CacheStore`. (T1 audit.)
- `runs` step query (`:322-`): `LEFT JOIN agent a ON a.id = s.agent_id`, and `s.verify_outcome AS "verify_outcome: VerifyOutcome", s.verify_exit_code, s.promoted_at, a.name AS agent_name` appended after the last existing column; `ORDER BY s.run_id, s.position, s.attempt, s.fanout_index` unchanged.
- Every `query_as!` over `step_graph` **inserts** `is_override` between `description` and `created_at` — it does **not** append it. `is_override` sits mid-struct (§2.5) and `query_as!` binds positionally, so an appended column maps `created_at` onto `is_override` and type-checks. Plan D10's "append" rule is a rule about *matching the struct*, and its three named projections (`RunStepSummary`, `ItemSummary`, `BoxInfo`) are the ones whose new fields are genuinely last. There are **four** sites, not two: `pg/read.rs:1243`, `pg/read.rs:1382`, `pg/write.rs:1568`, `pg/write.rs:1602`. Enumerate them unscoped — `grep -rn -A1 'sqlx::query_as!($' crates/htui-store/src/pg/*.rs | grep StepGraph`. (T1 audit, amends the "appends" wording this line carried.)

**Trait reads** (`impl ReadStore for PgStore`, `:58`):

| Method | SQL |
|---|---|
| `run` | `query_as!(RunRow, "SELECT … , repo_scope, lease_box_id, lease_expires_at FROM run WHERE id = $1")` `.fetch_optional` → `into_run` |
| `run_steps` | `query_as!(StepRow, "SELECT s.…, a.name AS agent_name FROM run_step s LEFT JOIN agent a ON a.id = s.agent_id WHERE s.run_id = $1 ORDER BY s.position, s.attempt, s.fanout_index")` → `into_step` |
| `step_trees` | `query_as!(RunStepTree, "SELECT run_step_id AS \"run_step_id: StepId\", repo_id AS \"repo_id: RepoId\", mode AS \"mode: Isolation\", path, base_ref, dirty FROM run_step_tree WHERE run_step_id = $1 ORDER BY repo_id")` |
| `step_commits` | `query_as!(RunStepCommit, "SELECT run_step_id AS \"…\", repo_id AS \"…\", before_hash, after_hash FROM run_step_commit WHERE run_step_id = $1 ORDER BY repo_id")` |
| `resolve_inputs` | one statement over `UNNEST($3::text[]) WITH ORDINALITY AS k(kind, ord)` (or, for empty `kinds`, `SELECT DISTINCT kind FROM document WHERE item_id = $1 ORDER BY kind`): `SELECT DISTINCT ON (k.ord) k.kind, d.* FROM k LEFT JOIN LATERAL (SELECT d.* FROM document d LEFT JOIN run_step s ON s.id = d.produced_by_step_id WHERE d.item_id = $1 AND d.kind = k.kind AND (s.id IS NULL OR s.selected IS NOT FALSE) ORDER BY (s.run_id = $2) DESC NULLS LAST, d.version DESC LIMIT 1) d ON true ORDER BY k.ord` — ANA-2 `:397-405` verbatim inside the lateral; map to `ResolvedInput { kind, document: d.id.map(…) }` |

**Eleven inherent reads** on `PgStore` (`pub async fn`, beside `box_info` `:700` and `active_runs` `:725`), signatures as §2.9's table:

| Method | SQL |
|---|---|
| `step_graph(id)` | `SELECT id, project_id, name, description, is_override, created_at, updated_at FROM step_graph WHERE id = $1` — struct order, `is_override` **between** `description` and `created_at` |
| `phase_agents(phase)` | `SELECT phase_id, position, agent_id, model FROM phase_agent WHERE phase_id = $1 ORDER BY position` |
| `prompt_template(project, name, version)` | `… WHERE project_id = $1 AND name = $2 AND ($3::int IS NULL OR version = $3) ORDER BY version DESC LIMIT 1` |
| `resolve_graph(item)` | three statements, no transaction (reads): the item (`NotFound`), `COALESCE(item.step_graph_id, item_kind.default_graph_id)`, the graph, its phases in position order, then one `phase_agent` query `WHERE phase_id = ANY($1) ORDER BY phase_id, position` grouped in Rust |
| `agent_boxes(box)` | `SELECT … FROM agent_box WHERE box_id = $1 ORDER BY agent_id` |
| `box_row(id)` | the existing `BoxRow` select `WHERE id = $1` |
| `repo_paths(box)` | `SELECT … FROM repo_box_path WHERE box_id = $1 ORDER BY repo_id` |
| `ready_items(scope, box)` | the existing `items` query with `ItemFilter { ready: true, .. }`, plus `AND NOT EXISTS (SELECT 1 FROM UNNEST(i.required_tags) t WHERE t <> ALL (b.probed_tags || b.declared_tags))` over `CROSS JOIN box b WHERE b.id = $box`; same ordering as `items` |
| `missing_tags(item, box)` | `SELECT t FROM item i CROSS JOIN box b, UNNEST(i.required_tags) t WHERE i.id = $1 AND b.id = $2 AND t <> ALL (b.probed_tags \|\| b.declared_tags) ORDER BY t`; `NotFound` for either id via two `SELECT 1`s when the result is empty |
| `active_runs_on_box(box)` | `SELECT COUNT(*) FROM run WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval')` |
| `overlapping_runs(scope)` | `SELECT … FROM run WHERE status IN ('running','awaiting_approval') AND repo_scope && $1::uuid[] ORDER BY queued_at` |

### 3.6 `pg/write.rs` — writers at SQL-statement level

`transition` (`:442-475`) gains the D14/D15 guard before the UPDATE: `if !from.can_move_to(to) { return match sqlx::query_scalar!("SELECT 1 FROM item WHERE id = $1", …).fetch_optional(…).await.map_err(map_sqlx)? { Some(_) => Err(StoreError::Constraint(illegal_move("item", from, to))), None => Err(StoreError::NotFound { entity: "item", id: id.to_string() }) }; }` — one extra round trip on the illegal path only; the legal path is unchanged. `transition_run`/`transition_step` copy the whole shape (`UPDATE … SET status = $3, started_at = CASE WHEN $3 = 'running' THEN COALESCE(started_at, $4) ELSE started_at END, finished_at = CASE WHEN $3 IN ('done','failed','cancelled') THEN COALESCE(finished_at, $4) ELSE finished_at END WHERE id = $1 AND status = $2`; the step form's terminal list is `('done','cancelled','superseded')` per `StepStatus::is_terminal` — note `failed` is not in it, so a step that fails keeps `finished_at` for `finish_step` to write).

`RETURNING` always reads the trigger's `updated_at`; **never `SET updated_at`**. `map_sqlx` turns `23xxx` into `Constraint`. `pool.begin()` only where more than one row moves.

| Method | Shape | Statements |
|---|---|---|
| `create_run` | **tx** | `SELECT status AS "status: Status" FROM item WHERE id = $1 FOR UPDATE` → `NotFound`; `legal_move(status, Queued)`; `INSERT INTO run (id, project_id, item_id, kind, mode, status, target_box_id, graph_snapshot, started_by, queued_at, repo_scope) VALUES ($1,$2,$3,'graph',$4,'queued',$5,$6,$7,$8,$9::uuid[]) RETURNING …` (`RunRow`); `UPDATE item SET status = 'queued' WHERE id = $1 AND status = $2` (the `transition` statement inline, `rows_affected == 1` else `Constraint` — impossible under the lock, but stated); commit |
| `claim_run` | **tx** | `SELECT status, target_box_id, repo_scope FROM run WHERE id = $1 FOR UPDATE` → `NotFound{"run"}`; `SELECT settings FROM box WHERE id = $2 FOR UPDATE` → `NotFound{"box"}` (ANA-2 `:1094`); if `status <> 'queued' OR target_box_id <> $2` → rollback, `Ok(false)`; `limit` from `BoxSettings` else `SELECT value FROM app_setting WHERE key = 'max_concurrent_items'` else the const; `SELECT COUNT(*) FROM run WHERE executing_box_id = $2 AND status IN ('running','awaiting_approval')`; `SELECT EXISTS (SELECT 1 FROM run r2 WHERE r2.executing_box_id = $2 AND r2.status IN ('running','awaiting_approval') AND r2.repo_scope && $3::uuid[])`; either refusal → rollback, `Ok(false)`; `UPDATE run SET status = 'running', executing_box_id = $2, started_at = $4, lease_box_id = $2, lease_owner = $3, lease_expires_at = $5 WHERE id = $1 AND status = 'queued'`; `UPDATE item SET status = 'in_progress' WHERE id = (SELECT item_id FROM run WHERE id = $1) AND status = 'queued'` (zero rows tolerated); commit, `Ok(true)` |
| `refresh_lease` | single | `UPDATE run SET lease_expires_at = $3 WHERE id = $1 AND lease_owner = $2`; 0 rows → `SELECT 1 FROM run WHERE id = $1` → `Ok(false)` / `NotFound` (the `transition` idiom) |
| `adopt_runs` | single | `UPDATE run SET lease_owner = $2, lease_expires_at = $4 WHERE executing_box_id = $1 AND status = 'running' AND (lease_expires_at IS NULL OR lease_expires_at < $3) RETURNING …` (`RunRow`), sorted by `queued_at` in Rust (ANA-2 `:1287-1291`) |
| `create_step` | single | `INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, agent_id, model, status) VALUES (…, 'pending') RETURNING …` (`StepRow` minus `agent_name`, or a second select — simplest: `RETURNING` into a local row struct then `into_step`; `agent_name` is not on `RunStep`) |
| `transition_run` / `transition_step` | single (+1 on the illegal path) | as above |
| `finish_step` | single | `UPDATE run_step SET exit_code = $2, usage = COALESCE($3, usage), trim_record = COALESCE($4, trim_record), verify_outcome = $5, verify_exit_code = $6, finished_at = $7 WHERE id = $1`; 0 rows → `NotFound{"run_step"}` (`set_step_prompt` `:826-845` idiom) |
| `answer_gate` | single | `UPDATE run_step SET status = $2, gate_outcome = $3, gate_note = $4, finished_at = COALESCE(finished_at, $5) WHERE id = $1 AND status = 'awaiting_approval'`; `$2` from the outcome mapping in Rust; 0 rows → existence check → `Ok(false)` / `NotFound` |
| `select_fanout` | **tx** | `SELECT id, status, fanout_index FROM run_step WHERE run_id = $1 AND position = $2 AND attempt = $3 FOR UPDATE`; validate in Rust (winner present with `fanout_index >= 0` and status `awaiting_approval|done`, else `NotFound`/`Constraint` before any write); `UPDATE run_step SET selected = true, status = 'done' WHERE id = $winner`; `UPDATE run_step SET selected = false, status = CASE WHEN status IN ('pending','awaiting_approval','done') THEN 'superseded' ELSE status END WHERE run_id = $1 AND position = $2 AND attempt = $3 AND fanout_index >= 0 AND id <> $winner`; `UPDATE run_step SET status = 'done', gate_note = $5 WHERE run_id = $1 AND position = $2 AND attempt = $3 AND fanout_index = -1`; commit (ANA-2 `:850-856`) |
| `supersede_step` | single | `UPDATE run_step SET status = 'superseded' WHERE id = $1 AND status IN ('pending','awaiting_approval','done')`; 0 rows → `SELECT status` → `NotFound` / `Constraint(illegal_move("run_step", status, Superseded))` |
| `upsert_step_tree` | **tx** | Rust check `all(run_step_id == step)` → `Constraint`; `SELECT 1 FROM run_step WHERE id = $1` → `NotFound`; per row `INSERT INTO run_step_tree (run_step_id, repo_id, mode, path, base_ref, dirty) VALUES (…) ON CONFLICT (run_step_id, repo_id) DO UPDATE SET mode = EXCLUDED.mode, path = EXCLUDED.path, base_ref = EXCLUDED.base_ref, dirty = EXCLUDED.dirty` (or one `UNNEST` statement like `append_events`); commit |
| `record_commits` | **tx** | same, `ON CONFLICT (run_step_id, repo_id) DO UPDATE SET before_hash = EXCLUDED.before_hash, after_hash = EXCLUDED.after_hash` |
| `write_document` | **tx** | `SELECT 1 FROM item WHERE id = $2 FOR UPDATE` → `NotFound`; `INSERT INTO document (id, item_id, kind, version, title, body, produced_by_step_id, created_by, created_at) SELECT $1, $2, $3, COALESCE(MAX(version), 0) + 1, $4, $5, $6, $7, $8 FROM document WHERE item_id = $2 AND kind = $3 RETURNING …` (`Document`); commit |
| `promote_step` | **tx** | `SELECT s.status, r.id, r.status, r.item_id FROM run_step s JOIN run r ON r.id = s.run_id WHERE s.id = $1 FOR UPDATE OF s, r` → `NotFound`; Rust: step status `failed|awaiting_approval` else `Constraint`; run non-terminal else `Constraint`; `UPDATE run_step SET status = 'awaiting_approval', promoted_at = $2 WHERE id = $1`; `UPDATE run SET status = 'awaiting_approval' WHERE id = $r AND status = 'running'`; `UPDATE item SET status = 'awaiting_approval' WHERE id = $item AND status = 'in_progress'`; commit (ANA-2 `:1185-1191`) |
| `fail_run` | single | `UPDATE run SET status = 'failed', failure = $2, finished_at = COALESCE(finished_at, $3) WHERE id = $1 AND status IN ('queued','running','awaiting_approval')`; 0 rows → `SELECT status` → `NotFound` / `Constraint(illegal_move("run", status, Failed))` |
| `close_out` | **tx** | `SELECT status FROM item WHERE id = $1 FOR UPDATE` → `NotFound`; `SELECT id FROM run WHERE item_id = $1 AND status IN ('queued','running','awaiting_approval') LIMIT 1` → `Constraint("item {item} has a live run {id}")`; `summary.kind == "summary"` and `summary.item_id == item` else `Constraint`; `legal_move(status, Closed)`; the `write_document` insert; the `record_commits` upserts (grouped per step, each step's existence via the FK); `UPDATE item SET status = 'closed', closed_at = clock_timestamp() WHERE id = $1 AND status = $2`; commit; return the document (ANA-2 `:1380-1391`) |
| `add_note` | single | `INSERT INTO item_note (id, item_id, body, created_by, box_id, via_step_id, created_at) VALUES (…) RETURNING …` (`Note`) |

Delete CTE (`:166-169` region): add `t AS (SELECT run_step_id FROM run_step_tree WHERE run_step_id IN (SELECT id FROM s))` and count it into `DeleteReach.run_step_trees`; `run_step_commits` is already counted there (its `0` on the demo is a fact of the data, not the code — confirm by reading the CTE before assuming).

### 3.7 `backend.rs`

`orchestration_offline()` beside `prompt_offline()` (`:423-425`, F-I; import `DATABASE_UNREACHABLE` from `writer.rs:638`, already `pub`). Eleven `pub async fn` on `Backend` in the `agents` shape (`:302-308`): `Memory(store) => store.x(…)`, `Online { pg, .. } => pg.x(…)`, `Offline { .. } => Err(orchestration_offline())`. Five trait reads in `impl ReadStore for Backend` (`:430`) in the existing shape: `Online` → `pg`, `Offline` → `cache`, `Memory` → `store`.

### 3.8 The mirror

- `cache/mod.rs:41-58`: `MIRRORED_TABLES: [&str; 17]`, `"run_step_tree"` appended after `"run_step_commit"`; doc `:31` "seventeen".
- `refresh.rs:200-210`: `const RUN_STEP_TREE: &str = "run_step_tree";` after `RUN_STEP_COMMIT`. Array literal `:254-265`: append it. Match `:271-282`: replace `_ => refresh_run_step_commit` (`:281`) with `RUN_STEP_COMMIT => refresh_run_step_commit(…)`, `RUN_STEP_TREE => refresh_run_step_tree(…)`, and a final `_ => unreachable!("every table in the cursor list has an arm")` or, better, make the list an enum so the compiler proves it (only if the file already leans that way; otherwise the explicit arms + `unreachable!` is the minimal change, F-F).
- `RUN_COLUMNS` (`:1066-1082`) + `refresh_run` (`:1084-1132`): `repo_scope` (bound as JSON text of the uuid array via the file's `strings_text`-style helper), `lease_box_id`, `lease_expires_at` (epoch).
- `RUN_STEP_COLUMNS` (`:1134-1155`) + `refresh_run_step` (`:1157-1212`): `verify_outcome`, `verify_exit_code`, `promoted_at`.
- `refresh_run_step_tree`: a copy of `refresh_run_step_commit` (`:1216-1256`) with `RUN_STEP_TREE_COLUMNS = ["run_step_id","repo_id","mode","path","base_ref","dirty"]`, the Postgres side `SELECT t.*, s.updated_at AS "ts!" FROM run_step_tree t JOIN run_step s ON s.id = t.run_step_id WHERE s.updated_at > $1 …`, `dirty` as `0/1`, `upsert_sql(RUN_STEP_TREE, cols, 2)`.
- `cache/read.rs`: `item_summary_of` (`:169-183`) + `touched_paths`; `box_info` (`:839-850`) select + three fields; the runs step SQL (`:513-524`) gains `LEFT JOIN agent a ON a.id = s.agent_id` and `s.verify_outcome, s.verify_exit_code, s.promoted_at, a.name AS agent_name` (`agent` is mirrored: `MIRRORED_TABLES[1]`), the builder (`:537-557`) the six fields; the five trait reads — `run`/`run_steps` decoding through the file's row helpers, `step_trees`/`step_commits` straight selects `ORDER BY repo_id`, `resolve_inputs` as the same SQL as Pg with SQLite spellings (`(s.run_id = ?) DESC` orders `1, 0, NULL` in SQLite too when written `ORDER BY CASE WHEN s.id IS NULL THEN 2 WHEN s.run_id = ? THEN 0 ELSE 1 END, d.version DESC` — write the `CASE` form on both backends if the implementer prefers one spelling; the conformance case is what pins parity).

### 3.9 Tests and strings (D8, F-J, F-M)

| Site | Today | Becomes |
|---|---|---|
| `tests/migrations.rs:18-51` `TABLES` | 32 names | + `"run_step_tree"` (33) |
| `:69-73` | `applied == vec![1, 2]` | `vec![1, 2, 3]` |
| `:88-92` | `TABLES.len() == 32` | `33` |
| `:94-98` | `present.len() == TABLES.len() + 1` | unchanged (derived) |
| `:159-203` `ANA_COLUMN_COMMENTS` | six `(table, column, text)` | + eighteen entries with §3.2's texts verbatim (24); the tuple shape is unchanged |
| `:206-264` | doc `:231-232` "exactly the six contracts"; message `:260` "exactly the six commented columns, and no others" | "twenty-four" in both; the `COMMENT ON TABLE run_step_tree` gets its own small assertion in the same test (`pg_description` on the class, `objsubid = 0`) |
| `:268-278` `ANA5_INTEGER_DEFAULTS` | unchanged | — |
| `:355` | `MigrationState::Pending(2)` | `Pending(3)` |
| `tests/connect.rs:95`, `:110` | `Pending(2)`, `pending, 2` | `3` |
| `tests/pg_conformance.rs:19` | `EXPECTED_CASES = 36` | `47` |
| `tests/cache.rs` | — | new: `run_step_tree_refreshes_off_its_parent_step` (write a tree via `PgStore`, refresh, read via `CacheStore::step_trees`; touch the step so `updated_at` moves, refresh again, the tree row is re-upserted); the seventeen-table rebuild assertion if one counts `MIRRORED_TABLES`; the two `schema_version` literals stay (D8) **This row is a milestone gate, not a line item**: `trees_and_commits_read_back` asserts totality on an empty fixture and both of its row-content twins are `WriteStore` cases a `CacheStore` cannot run, so this test is the only thing in the tree pinning that the mirror carries `run_step_tree` and `run_step_commit` at all. Add the `step_commits` half beside the tree half. (T1 audit.) |
| `tests/pg_criteria.rs` | `:485-491` `Open → Done` asserted to succeed; `:2739` `TABLES: [&str; 21]` / `:2800` `claimed: [u64; 21]` | **(a) `status_cas_never_bumps_version` (`:485-491`)**: the law refuses `open → done`, and the assertions after it need the row *at* `done` with `closed_at` set, so drive it `Open → Queued → InProgress → Done` — the shape `status_cas_keeps_version` now uses in `conformance.rs` — and leave `:504` (`Done → Open`) and `:528` (stale `Done → Closed`) alone. A bare `Open → Queued` breaks the `closed_at.is_some()` and reopen legs that follow. `:535` needs no code change; its doc gains "plan D14: an unknown id is `NotFound` even though the pair is illegal". **(b) `project_delete_takes_everything_and_says_so` (`:2737-2838`)**: `TABLES` and `claimed` are zipped **positionally**, and `run_step_trees` sits at struct index 16 (after `run_step_commits`, before `command_runs`) — appending `"run_step_tree"` to both arrays compiles and silently compares `document` counts against `item_link` claims for every field after 16. Insert `"run_step_tree",` into `TABLES` between `"run_step_commit"` and `"command_run"`, bump both lengths to 22, add `run_step_trees,` to the destructure after `run_step_commits,` and to `claimed` at the matching index, and fix the "Twenty-one entries for twenty-one fields" doc at `:2737`. Seed at least one `run_step_tree` row so the entry cannot pass on `0 == 0`, and extend the `(phase_agents, run_step_commits, command_runs, repo_box_paths) == (1, 1, 1, 1)` guard at `:2838` to cover it. **(c)** the four new Postgres-only twins named in the case docs: `admission_is_serialised_by_the_box_row_lock` (two concurrent `claim_run`s on one slot, exactly one `true`), `ck_run_graph_snapshot_is_not_valid_for_old_rows_and_checked_for_new` (fixture rows load; a raw `INSERT … kind='graph', graph_snapshot=NULL` fails), `document_versions_do_not_collide_under_contention` (two concurrent `write_document`, versions `n+1`, `n+2`), `step_tree_rows_cascade_with_their_step` (delete the run, `run_step_tree` count 0). **Each of the four is listed in `PENDING` in `conformance.rs`'s `every_cross_referenced_test_name_exists`; deleting its entry is part of writing it**, and the test fails by name if the entry outlives the fn. (T1 audit, (a) recipe corrected and (b) added.) |
| `crates/htui/src/ui/tabs/settings/connection.rs:136`, `:989` | "16 mirrored tables" | "17 mirrored tables" |
| `crates/htui/tests/connection.rs:566` doc, `:1664` | "sixteen" / `"16 mirrored tables"` | seventeen / `"17 mirrored tables"` |
| `crates/htui/tests/snapshots/connection__confirm.snap:32` | rendered "16" | re-record (`cargo insta` or the repo's snapshot flow) |
| `crates/htui/src/store_worker.rs:488` doc | "sixteen mirrored tables" | "seventeen" |

### 3.10 `.sqlx`

From `crates/htui-store`: `DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features` against the migrated dev database after every commit that adds or changes a `query!`; commit the new `query-*.json` files with the code that uses them.

**Both forms of the check need a `DATABASE_URL`, and it must name a *migrated* database** (T2 audit). The URL the milestone hands around for `HTUI_TEST_DATABASE_URL` is `…/postgres`, which is **bare** — the harness mints and drops its own schemas — so `cargo sqlx prepare --check` run with that one exported emits 212 compile errors and exits 1. That is a false red: the `.sqlx` data is current and the checker simply cannot see a `run` table. Use `…/htui_sqlx`, or run the CI-equivalent, which needs no database at all and is the stronger check of the two:

```
SQLX_OFFLINE=true cargo check -p htui-store --all-targets --all-features
```

### 3.11 T2 commit sequence

| # | Commit | Contents | Gate |
|---|---|---|---|
| 1 | `feat(htui-store): 0003_orchestration on both sides, with the pins that said two and sixteen` | §3.2, §3.3, `TABLES`, `applied`, `Pending(3)` ×3, `ANA_COLUMN_COMMENTS` ×24, the seventeen strings, snapshot re-record | migrations tests green; rest of the crate still red |
| 2 | `feat(htui-store): rows and projections carry the 0003 columns` | §3.4, `items`/`box_info`/`runs` in `pg/read.rs`, `step_graph` `query_as!`s, mirror columns + refreshers (§3.8 minus the tree table), `cache/read.rs` projections, `.sqlx` | compiles; existing suites green |
| 3 | `feat(htui-store): the five run reads on PgStore, CacheStore and Backend` | trait reads ×3 impls, `run_step_tree` refresher + `MIRRORED_TABLES`, `tests/cache.rs` tree test, `.sqlx` | `READ_CASES` green on Pg and mirror |
| 4 | `feat(htui-store): run creation, admission, lease and step CAS` | `transition` guard, `create_run`, `claim_run`, `refresh_lease`, `adopt_runs`, `create_step`, `transition_run`, `transition_step`, `supersede_step`; `pg_criteria.rs:487` edit and the admission twin | cases 37–40 green on Pg |
| 5 | `feat(htui-store): settle, gates, fan-out, trees and commits` | `finish_step`, `answer_gate`, `promote_step`, `add_note`, `select_fanout`, `upsert_step_tree`, `record_commits`, delete CTE; cascade twin | cases 41–44 |
| 6 | `feat(htui-store): documents, close-out, failure, and the eleven inherent reads` | `write_document`, `close_out`, `fail_run`, §3.5 inherent + §3.7; `EXPECTED_CASES = 47`; contention and NOT VALID twins | full T2 gate incl. `sqlx prepare --check` |

---

## 4. T3 — `Writer`, `BufferedWriter`, the spies

### 4.1 `crates/htui-store/src/writer.rs`

`impl WriteStore for BufferedWriter` (`:233`): eighteen arms, each `Err(hierarchy_needs_the_server())` (`:652-654`) with every parameter `_`-prefixed, in the trait's order, in the `transition` arm's shape (`:247-249`). `impl ReadStore for BufferedWriter` (`:182`): five arms, same refusal (D5: "the five new `ReadStore` reads refuse there too"). `impl WriteStore for Writer` (`:752`): eighteen three-arm delegations in the `mint_item` shape (`:754-758`); `impl ReadStore for Writer` (`:657`): five. The doc block at `:640-651` gains one line naming the eighteen + five as the second family the sentence serves.

### 4.2 `crates/htui-store/tests/writer_buffered.rs`

`every_hierarchy_method_is_unreachable_offline` (`:317`) keeps its `refused` closure (`:323-329`, `sentence == DATABASE_UNREACHABLE`); a sibling `every_run_seam_method_is_unreachable_offline` calls all twenty-three with fixture ids and asserts the same sentence. `:251`'s `Open → InProgress` assertion is untouched (D15).

### 4.3 Spies

`crates/htui-agent/src/conformance.rs` `UsageSpy` (`ReadStore` `:600`, `WriteStore` `:645`) and `crates/htui-agent/tests/recorder.rs` `SpyStore` (`:278`, `:323`): twenty-three plain `self.inner.x(…).await` delegations each, `StoreResult` return type as the files use. No recording: the spies exist to count `set_step_usage`-family calls, and nothing in this milestone changes what they count.

### 4.4 T3 commits

1. `feat(htui-store): Writer delegates and BufferedWriter refuses the run seam` (§4.1, §4.2) — `cargo test -p htui-store --all-features`.
2. `test(htui-agent): the spies pass the run seam through` (§4.3) — `cargo test -p htui-agent --all-features`.
3. `docs(htui-store): the one sentence serves twenty-three more methods` — doc-only, then the full gate `cargo test --workspace --all-features` (with `--test-threads=1` once, per memory).

#### 4.5 Amendments from the T2 adversarial audit (2026-09-18, after T2's close-out) — **T3's gate**

Everything actionable inside `htui-store` was fixed in T2's repair commits. One finding is T3's, and it
corrects a claim T1 and T2 both carried forward:

| # | What the audit found | What T3 must do |
|---|---|---|
| B-1 | **`cargo check -p htui --all-targets` does not pass at HEAD and has not since T1.** `htui-agent`'s `UsageSpy` was never widened, `htui` depends on `htui-agent`, so `htui`'s own code is never type-checked. T1's commit `13f3020` says "the `htui` crate does not compile until T2 lands anyway (plan D0)", which implies it becomes gatable once T2 lands. **It does not** — §4.3's spies are what unblock it, and they are T3's. Five hand-written `htui` files therefore ship through the end of T2 with no compiler behind them: `src/hierarchy.rs` (T1's 22-field `DeleteReach` destructure), `tests/kinds.rs` (T1's `StepGraph` literal), `src/ui/tabs/settings/connection.rs`, `tests/connection.rs` and `tests/snapshots/connection__confirm.snap` (T2's seventeen-strings, the last of them a **hand-edited insta snapshot that could not have been re-recorded**). | Add two lines to §4.4 commit 2's gate, run **after** the spy arms land and before commit 3: `cargo check -p htui --all-targets` and `cargo test -p htui --all-features`. Do not trust `connection__confirm.snap`: run the suite and, if insta reports a mismatch, re-record it rather than hand-editing again. |

T2's repair verified all five by building them behind a **throwaway** stub — the twenty-three `todo!()`
arms the compiler itself suggests, pasted into `UsageSpy`'s two impls and reverted immediately after.
With that stub in place `cargo check -p htui --all-targets` exits 0 and `cargo test -p htui
--all-features` is green, snapshot suite included. That is evidence the five files are correct as
written; it is **not** a substitute for T3 running the real gate, and nothing about it was committed.

#### 4.6 Amendments from the T3 adversarial audit (2026-09-18, milestone close-out)

Two verifiers audited T3 and the milestone as a whole. Both MEDIUM findings were verified against the
tree before acting; the three LOW ones are dispositioned here so they reach the item's close-out.

The `R-*` rows are the configured reviewer's gate on the finished milestone (2026-09-18). Its HIGH
and its cheap findings were applied (`d622814`, `7bc9b39`, `f73cffd`, `58007f4`), which grew two of
§2.11's cases: case 43 `select_fanout_is_one_transaction` gains leg **(f)** — a second slot whose
judge is driven to `failed` with a `gate_note`, asserting that the human's pick leaves both alone —
and case 37 `run_create_moves_the_item` gains a leg refusing a run filed under a project its item is
not in. The two rows below
are **not defects in this milestone** — each argues against a settled decision (D6, ANA-2 §8's writer
table) and each is a seam question milestone 2 must answer before it can write an orchestrator. They
are recorded here, with what the seam does today and the options each offers, so milestone 2 starts
from them rather than rediscovering them.

| # | What the audit found | Disposition |
|---|---|---|
| C-1 | T2 added a **second** offline-refusal helper, `orchestration_offline()`, for `Backend`'s eleven inherent orchestration reads. It reuses `DATABASE_UNREACHABLE`, so the sentence discipline holds — but nothing pinned it. The eleven are inherent to `Backend`, on no trait, so `BufferedWriter`'s twenty-three cannot reach them, and no test anywhere constructed a `Backend::Offline` and asked any of them anything. The helper sits beside `prompt_offline()` in a block of identical shape that answers a **different** sentence (`PROMPT_ON_SERVER_ONLY`); one arm wired to the wrong neighbour compiles silently | **Fixed** (`33d1c87`): `writer_buffered.rs::every_inherent_orchestration_read_is_unreachable_offline`, the `refused`-closure sibling of the other two, over a throwaway mirror. Checked by mis-wiring `step_graph` to `prompt_offline()` and watching it fail **by method name** on the sentence. The Acceptance bullet is reworded (C-3) |
| C-2 | `cargo doc --workspace --no-deps` — one of the five commands in the plan's own Validation block — exits 101, so the acceptance line cannot honestly be ticked | **Recorded, no code change.** Verified: six errors, all `htui-store` (`dsn.rs` ×4, `secret.rs` ×2), **none** MOD-4's. `git diff --name-only 0cf232d..HEAD` returns zero files for `dsn.rs`/`secret.rs`/`connect.rs`/`lib.rs`, and `cargo doc -p htui --no-deps` is exit 0. Two of the six (`FAKE`, `crate::testkit::mock_keyring`) are **feature-gated** items (`#[cfg(feature = "test-support")]`), so this is a real `--all-features`-vs-default docs question and not a de-link: adding `--all-features` resolves those two but exposes three the default run hides (`htui-core/src/prompt/fixtures.rs` `handoff_events` ×1, `htui-agent/src/conformance.rs` `open_case` ×2, the latter from `8f1b6b3`, MOD-2 era). Neither invocation is green and neither is MOD-4's doing. **A CLEAN item**, not this milestone's: the files are outside every MOD-4 file set, and the right fix is a deliberate decision about which feature set the doc gate documents. The plan's Acceptance bullet is amended to say so rather than tick against a red command |
| C-3 | The claim "every new offline path answers the one sentence" is over-broad: `Backend::Offline`'s `run`, `run_steps`, `step_trees`, `step_commits`, `resolve_inputs` answer **from the mirror**, not with a refusal (plan D12; all five tables are in `MIRRORED_TABLES`) | **Fixed** (`33d1c87`), documentation only: the plan's Acceptance bullet and the new case's doc comment both now say "every new offline **refusal**", and name the five and why they are exempt. The behaviour is deliberate and correct; only the wording was wrong |
| C-4 | `STEP_SANCTIONED`'s doc claims verbatim transcription of ANA-2 §4.3, but `(Failed, Cancelled)` is derived from row 652 + row 654's exception clause rather than transcribed | **Deferred to milestone 2's close-out**; not trivially right either way. The *behaviour* is what §4.3 means (a non-terminal run's cancellation takes its steps with it) and `can_move_to` agrees, so nothing is wrong in code; the defect is a doc claim, and the two offered fixes disagree about which document moves. Cheapest correct fix is a `// derived from row 652 + row 654's exception` marker, mirroring the annotation `awaiting_approval -> awaiting_approval` already carries |
| C-5 | `trees_and_commits_read_back` (one of the three new `READ_CASES`) asserts only `Ok(vec![])` for eight step ids, so a `CacheStore` mirroring neither table passes it byte for byte | **No change** — the auditor's own recommendation. Disclosed in the case's own doc and compensated by T2's `tests/cache.rs::the_0003_columns_reach_the_mirror` and `::run_step_tree_refreshes_off_its_parent_step` (§0.1 A-6, §0.2 B-5). The case is a **totality** pin on F-P's deliberately empty fixture; strengthening it needs a fixture `repo`, which §0's F-P row rules out for this milestone. Carry to milestone 2 |
| R-1 | **A terminal run move does not write the item mirror in the same transaction.** ANA-2 §4.3 adopts "item status is a function of the item's non-terminal runs, **written in the same transaction as the run move**" (`docs/ANA-2.md:543`) and its table gives the orchestrator `in_progress -> done` (last position done), `-> failed` (budget exhausted, not loopable) and `-> blocked` (escalation). What the seam does today: `create_run` moves `open\|failed -> queued` and `claim_run` moves `queued -> in_progress`, each inside its own transaction — so the two *entry* moves are mirrored — while `transition_run` and `fail_run` write the `run` row and nothing else (`pg/write.rs`, `transition_run`'s single `UPDATE run`; `fail_run`'s single `UPDATE run ... WHERE status IN ('queued','running','awaiting_approval')`). An orchestrator finishing a run therefore needs two calls and cannot get them into one transaction through this seam. This is not an oversight: plan **D6** names exactly five transactions and neither of these is one, and ANA-2 §8's writer table (`docs/ANA-2.md:1586`) assigns `item.status` to "orchestrator and close-out only", i.e. **not** to the run writers | **Recorded for milestone 2, no code change.** Two options, and they disagree about which document moves. **(a) Widen the seam**: `transition_run` and `fail_run` take the item mirror into their own transaction, D6 grows from five transactions to seven, `MemStore` follows, and §8's writer column is reread as "the orchestrator's writers, wherever they live". Cost: the mirror target is not a function of the run alone — §4.3's mapping is over the item's *remaining non-terminal runs* — so both writers would have to count the item's other live runs under the item lock, which is a read `transition_run` does not do today and a rule `MemStore` would have to duplicate. **(b) Keep the seam thin** and give milestone 2 one composite writer (`finish_run(run, to, at)`) that moves the run and derives the item from §4.3's mapping in one transaction, leaving `transition_run` as the general-purpose compare-and-set the chat path and the conformance law already use. (b) is the smaller change and the one that keeps D6 honest; (a) is the one that makes it impossible to forget the mirror. **Decide before the first orchestrator writer lands** — whichever is chosen, the conformance suite needs the leg that pins item and run moving together |
| R-2 | **`claim_run`'s overlap predicate is repo-set-on-same-box only, so ANA-2 §4.7's rules I and P cannot be evaluated inside the admission critical section.** What the seam does today, under the `box` row's `FOR UPDATE` (`pg/write.rs`, `claim_run`): `SELECT EXISTS (SELECT 1 FROM run WHERE executing_box_id = $1 AND status IN ('running','awaiting_approval') AND repo_scope && $2::uuid[])`. That is §4.7's rule L-and-I collapsed into "any shared repo overlaps". §4.7's adopted predicate (`docs/ANA-2.md:1009`ff) is over `(repo scope, isolation, touched paths)`: rule **L** (either side `local` on `r`) and rule **I** (not both isolated on `r`) and rule **P** (declared path prefixes intersect), with `RunScope` carrying `isolated`, `local` and `paths` **per repo**, all "resolved at queue time and stored on the run". The run table stores only `repo_scope`. Two consequences, in opposite directions: within a box the predicate is **conservative** — it serialises a pair §4.7 would admit (both isolated, disjoint paths), costing parallelism but never safety; across boxes it is **narrower than §4.7**, whose predicate has no box filter at all, so two runs sharing a repo on two boxes do not see each other here | **Recorded for milestone 2, no code change.** The milestone-1 predicate is what the stored columns can answer, and D6/§8 put it inside the critical section deliberately. Two options. **(a) Store the rest of `RunScope` at queue time** — per-repo `isolated`, `local` and normalised `paths` on the `run` row (a milestone-2 migration; `isolated[r]` is "every phase of the snapshot uses `worktree`/`copy` for `r`", which `create_run` can already compute from the snapshot it is handed), so the whole predicate stays one SQL statement under the box lock. Cost: a second migration and three more columns whose value is fixed at queue time, which is exactly what §4.7 asks for ("resolved once at queue time and stored"). **(b) Compute rules I and P in Rust inside the critical section** from the candidates' `graph_snapshot` and the items' `touched_paths`. Cost: the admission critical section grows from one `EXISTS` to a read of every overlapping run's snapshot plus a prefix-intersection pass while the box row is locked, and the rule leaves SQL — which also means `MemStore` and `PgStore` share it, an argument in (b)'s favour. **The box filter is a third question** and belongs with them: it is either a deliberate narrowing (a repo checkout is per box, so two boxes cannot collide) or a gap, and §4.7 does not say which. Settle all three together with the overlap module ANA-2 sketches as `crates/htui-orch/src/overlap.rs` |

---

## 5. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | `query_as!(BoxInfo, …)` at `pg/read.rs:700-714` and `StepRow` (`rows.rs:114-117`) bind **positionally**: a field appended to the struct but not to the select, or in a different order, compiles and mis-assigns | Append in both at the same time; `run_and_steps_round_trip` (g) and the existing `box_info` tests catch a swap |
| H-2 | `the_ana_column_comments_are_present_and_verbatim` (`migrations.rs:206-264`; renamed count-free by the T2 repair, it was `the_six_…`) asserts "and no others": adding comments without extending the pin fails; trimming the pin to pass loses the guarantee | Extend to 24; never delete an entry |
| H-3 | `refresh.rs:281`'s `_ =>` fallback would route `run_step_tree` to the commit refresher — compiles, refreshes nothing, no test fails until the mirror is read | Explicit arms (F-F) and the `tests/cache.rs` tree test |
| H-4 | Someone "fixes" the judge with `CHECK (fanout_index >= 0)` | §3.2's section-4 comment; case 40 (d) inserts `-1` on both backends |
| H-5 | `mem.rs:2575-2576`'s hard-coded zeros | Case 44 (m) |
| H-6 | `schema_version()` becomes 3 → every existing mirror rebuilds on first connect; on a slow box the connection section shows the rebuild copy | Expected; the seventeen-table string is that copy. Not a defect |
| H-7 | `.sqlx` drift: a query edited without `cargo sqlx prepare` passes locally with a live DB and fails CI under `SQLX_OFFLINE` | `prepare --check` is in the T2 gate |
| H-8 | `htui-store` is red from T1 commit 3 to T2 commit 2; an implementer running `cargo test --workspace` mid-way will see unrelated-looking failures | Stated in §1; per-crate gates until T3 |
| H-9 | Fixture growth (F-P) is the one place this blueprint adds rows the plan did not; a later `.snap` that renders ANA-1's runs will move | Rows are confined to ANA-1; the reviewer checks `git diff --stat crates/htui/tests/snapshots` is exactly `connection__confirm.snap` |
| H-10 | Empty `repo_scope` overlaps nothing in `claim_run` (`'{}' && x` is false); ANA-2 §4.7 says an *empty `touched_paths`* "overlaps the whole primary repo", which is milestone 2's resolution from paths to scope, not this seam's | Doc on `NewRun.repo_scope`; milestone 2 must never queue a run with an empty scope when the item has a primary repo |
| H-11 | `StepStatus::Failed` is non-terminal; `chat_step_status` (`traits.rs:617-625`) maps `RunStatus::Failed → StepStatus::Failed` for chat closes, and D16 records that chat rows never pass through the law | Unchanged; the law applies to graph steps only, and `finish_chat_run` keeps its unconditional UPDATE |
| H-12 | `max_concurrent_items` on `MemStore` is the const unless the fixture's `app_settings` names the key; on Postgres `0003` seeds it — both answer 2, but a future `SEEDED_SETTINGS` change in `seed.rs` that adds the key with another value would split them | `claim_run` reads the same three rungs on both; case 38 (d) pins 2 |
| H-13 | `select_fanout` leaves a `failed`/`cancelled` loser's status alone (the table has no `failed → superseded`) but still sets `selected = false` | Case 43 (a) pins it; ANA-2 §4.5's "every loser superseded" is read as "every loser that can be" |
| H-14 | `resolve_inputs` ordering across SQLite and Postgres (`DESC NULLS LAST` semantics differ) | Write the `CASE … END` rank on both backends (§3.8); case R9 (c) and case 45 (f) pin the three ranks |

---

## 6. File sets (absolute paths)

**T1**
- /home/mluigi/projects/htui/crates/htui-core/src/model/run.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/item.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/box_.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/kind.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/note.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/document.rs
- /home/mluigi/projects/htui/crates/htui-core/src/model/mod.rs
- /home/mluigi/projects/htui/crates/htui-core/src/store/traits.rs
- /home/mluigi/projects/htui/crates/htui-core/src/store/mem.rs
- /home/mluigi/projects/htui/crates/htui-core/src/store/conformance.rs
- /home/mluigi/projects/htui/crates/htui-core/src/seed.rs
- /home/mluigi/projects/htui/crates/htui-core/src/fixtures.rs
- /home/mluigi/projects/htui/crates/htui-core/tests/mem_store.rs
- /home/mluigi/projects/htui/crates/htui/src/app/update.rs

**T2**
- /home/mluigi/projects/htui/crates/htui-store/migrations/0003_orchestration.sql (new)
- /home/mluigi/projects/htui/crates/htui-store/cache_migrations/0003_orchestration.sql (new)
- /home/mluigi/projects/htui/crates/htui-store/src/pg/rows.rs
- /home/mluigi/projects/htui/crates/htui-store/src/pg/read.rs
- /home/mluigi/projects/htui/crates/htui-store/src/pg/write.rs
- /home/mluigi/projects/htui/crates/htui-store/src/backend.rs
- /home/mluigi/projects/htui/crates/htui-store/src/cache/mod.rs
- /home/mluigi/projects/htui/crates/htui-store/src/cache/refresh.rs
- /home/mluigi/projects/htui/crates/htui-store/src/cache/read.rs
- /home/mluigi/projects/htui/crates/htui-store/.sqlx/ (regenerated)
- /home/mluigi/projects/htui/crates/htui-store/tests/migrations.rs
- /home/mluigi/projects/htui/crates/htui-store/tests/connect.rs
- /home/mluigi/projects/htui/crates/htui-store/tests/pg_conformance.rs
- /home/mluigi/projects/htui/crates/htui-store/tests/cache.rs
- /home/mluigi/projects/htui/crates/htui-store/tests/pg_criteria.rs
- /home/mluigi/projects/htui/crates/htui/src/ui/tabs/settings/connection.rs
- /home/mluigi/projects/htui/crates/htui/tests/connection.rs
- /home/mluigi/projects/htui/crates/htui/tests/snapshots/connection__confirm.snap
- /home/mluigi/projects/htui/crates/htui/src/store_worker.rs (doc only)

**T3**
- /home/mluigi/projects/htui/crates/htui-store/src/writer.rs
- /home/mluigi/projects/htui/crates/htui-store/tests/writer_buffered.rs
- /home/mluigi/projects/htui/crates/htui-agent/src/conformance.rs
- /home/mluigi/projects/htui/crates/htui-agent/tests/recorder.rs

**Not touched, on purpose**: `crates/htui-orch` (does not exist), `run_worker.rs`, `agent_worker.rs`, every `ui/tabs/backlog/**` file, `seed.rs` beyond the one `is_override: false`, `pg/demo.rs`, `cache/pending.rs`, MOD-2's chat write path (`start_chat_run` `pg/write.rs:715-749`, `finish_chat_run` `:762-809`; D16).
