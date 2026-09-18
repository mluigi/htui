# Blueprint: MOD-4 milestone 2 — a graph walks

**Plan**: `.claude/plans/mod-4-orch-engine.plan.md` (APPROVED; D1–D20 settled). **PRD**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md` D1–D8 win on conflict. **Design authority**: `docs/ANA-2.md` §4.1–§4.4, §4.9, §5.1, §5.4, §6.1, §6.2, §9; `docs/ANA-5.md` §12 criteria 3 and 16 and the front-matter grammar (`docs/ANA-5.md:1322-1336`).

**Verified at**: HEAD `35dc1ff`. `graphify-out/GRAPH_REPORT.md` was built at `3e346107` (five commits stale), so every graph-located fact below was re-read from the tree and the line cited is the tree's. **Line numbers are pre-edit**: T1 inserts into `traits.rs`, `mem.rs`, `pg/write.rs`, `writer.rs` and the three test files, so any T1 citation below moves after T1's first commit; T2–T5 citations into `htui-core`/`htui-agent`/`htui-store` are unaffected except `model/run.rs:742-743`, which T5 edits last.

**Scope**: T1 ∥ T2, then T3 → T4 → T5. Five tasks, seven new modules in one new crate, one seam writer on three stores, six ANA-2 criteria. No migration, no `.snap`, no `crates/htui/**`.

**House style**: one named free function per refusal sentence (`traits.rs:1053-1125`); contract order lookup → `NotFound`, `legal_move` → `Constraint`, stale `from` → `Ok(false)`, else `Ok(true)` (`traits.rs:976-977`); every instant a writer stamps comes from the caller and is `trunc_subsecs(TIMESTAMPTZ_DIGITS)`-truncated (`model/run.rs:272`); `MemStore` writes are one non-async `write` closure (`mem.rs:657`); doc comments cite the ANA line they implement; `#![warn(missing_docs)]`; `[lints] workspace = true`.

---

## 0. Flags

Plan-vs-tree discrepancies and architect additions. Each has a resolution the implementer follows; the **A-** rows were surfaced to the maintainer before dispatch.

| # | Plan says | Tree at HEAD | Resolution |
|---|---|---|---|
| F-A | D20: `phase_agents` at `mem.rs:463-472`; `State::resolve_graph` at `:3856-3862` | fn `phase_agents` is `:469-472` (doc `:463-468`); `State::resolve_graph` is `:3839-3862`, `agents: Vec::new()` at `:3860` | Cite the tree's lines in doc comments. |
| F-B | D7: Pg `promote_step` at `pg/write.rs:3303-3335`, Mem at `mem.rs:3680-3720` | Pg `:3323-3392`; Mem `:3681-3720` | Same. The Pg one is the template for Pg `finish_run` (§2.4). |
| F-C | D6: `registry.rs:149-189` | `caps_for` `:152`, `caps_from` `:157-195` | Same. |
| F-D | D7: banner `traits.rs:669-675` "five → seven" | Banner is `:670-675`; text names `create_run`, `claim_run`, `select_fanout`, `write_document`, `close_out` | Rewrite the sentence to name seven: those five plus `promote_step` and `finish_run` (§2.2). |
| F-E | "Not touched": "this milestone adds … no `.sqlx` query, so `cargo sqlx prepare --check` must simply stay green" | Pg `finish_run` is three `sqlx::query!` calls (§2.4); each needs a `.sqlx/query-*.json` | T1 regenerates and commits `.sqlx` (the implementer prompt already says so). The "no query" sentence is wrong; `--check` stays green only because T1 committed the files. |
| F-F | D20: fake candidates from rung 3, "the single enabled agent on the box"; harness seeds `default_agent_id` for rung 2 | All three seeded agents are `enabled: true` from one literal (`model/agent.rs:156`; `fixtures.rs:457-470` clones the seed), so rung 3 ("the *single* enabled agent") is empty on `MemStore::demo()`. `SettingKey` is a closed enum (`prompt/settings.rs:151`) with no `default_agent_id`, so `set_setting` cannot seed rung 2; `ProjectSettings` (`kind.rs:254-277`) is reachable only through `update_project`'s settings patch. | **A-1.** `FakeGraphSource` (§4.3) computes rung 1 from the store (always empty), rung 3 over `MemStore::agents()` filtered `enabled` (empty on demo), and then an explicit per-phase `candidates` map the harness sets, defaulting to `(AGENT_CLAUDE, "sonnet")` for every phase — the pair fixture `RUN_1`'s steps already carry (`fixtures.rs:1422-1432`). The map is the fake's stand-in for rungs 2 and 3, documented as such. Rung 2 stays in `graph.rs` (it reads `Project.settings` through `ReadStore::project`, `traits.rs:128`) and is exercised by one unit test that writes `default_agent_id` into a `Project.settings` value directly. |
| F-G | D19: four `GraphSource` methods; §4.1 chains for `token_budget` and `deadline_seconds` end at `app_setting` | No `app_setting` read exists on `ReadStore`/`WriteStore` (grep: none); `MemStore::app_settings()` is inherent (`mem.rs:413`) | **A-2.** `graph::resolve` takes `app: &BTreeMap<String, Value>` as a parameter, the same shape `settings::resolve_budget(phase, project, app)` already takes (`prompt/settings.rs:637`). The harness passes `MemStore::app_settings().await?`; milestone 6 passes `Backend::app_settings()`. D19's four methods are untouched. |
| F-H | D7: `finish_run(run, to, at)` | A `Failed` finish must write `run.failure` (`fail_run` does, `mem.rs:3739`); D12's strings are `run.failure` text; a three-argument writer would leave it NULL and force a second write, which is what R-1 (b) exists to avoid | **A-3.** Signature is `finish_run(&self, run: RunId, to: RunStatus, failure: Option<&str>, at: DateTime<Utc>) -> Result<()>` with `failure` refused (`Constraint`) when it disagrees with `to` (§2.1). D7's substance — composite, item derived from remaining live runs, one transaction, seventh — stands. |
| F-I | §4.4 escalation writes `run.failure = "review loop exhausted after N attempts"` with `run.status = 'awaiting_approval'` | No shipped writer sets `failure` on a non-terminal run (`fail_run` moves to `failed`, `mem.rs:3722-3741`; `transition_run` never touches `failure`, `:3366-3394`); `finish_run` is terminal-only by D7 | **A-4.** Milestone 2 records the exact wording in the `item_note` body (criterion 6 asserts the note, not `run.failure`, `docs/ANA-2.md:2101-2102`) and in the `RunFailure::ReviewLoopExhausted` value returned to the caller; `run.failure` stays NULL on a parked run. Carried as **R-3** for milestone 5, whose recovery sweep is the first reader of a parked run's `failure`. |
| F-J | §4.3 item table: escalation is `in_progress → blocked`; §4.4: after escalation `approve` resumes at `p_review + 1` | `Status::can_move_to` (`item.rs:46-60`) has `AwaitingApproval → InProgress \| Failed \| Open` (no `Blocked`) and `Blocked → Open \| Closed` (no `InProgress`). The human path finds the item at `awaiting_approval`; the resume finds it at `blocked`. | **A-5.** Escalation from the human path moves the item `awaiting_approval → in_progress` (the "reject with note, phase loopable" row, ANA-2 `:577`) **before** the loop decides, then `in_progress → blocked` on exhaustion (§5.6). Resume-after-escalation is **not** implemented here: `AnswerGate` on a `failed` step is refused by §6.2's own guard (`awaiting_approval` only), and `RetryStep` on a step whose item is `blocked` is refused with `EngineError::ItemBlocked` until milestone 6 ships `unblock`. Carried as **R-4**. |
| F-K | ANA-2 §4.2 gate table: "the run mirrors it; the item mirrors the run … both mirrors are written in the step's transaction" (`:660-664`) | Milestone 1 shipped no composite writer for `running → awaiting_approval` on step+run+item; `promote_step` refuses a `running` step (`mem.rs:3683-3690`) | **A-6.** The gate park is three compare-and-sets in fixed order (step, run, item; §5.5), atomic on `MemStore` per call but not across calls, and not atomic on Postgres. Acceptable this milestone (no Postgres in the loop); carried as **R-5** with `finish_run` as the precedent for the composite. |
| F-L | T2: "the override clone copies `step_graph_phase` and `phase_agent` and not `skill_binding`" | No `WriteStore` writer for `phase_agent` (grep `traits.rs`: none; `MemStore` holds no table, `mem.rs:463-468`); `is_override` is settable by no writer (`NewStepGraph` `kind.rs:152-161` has no field; `StepGraphPatch` doc `:165-166` says so) | **A-7.** `graph::override_graph` clones `step_graph_phase` via `create_step_graph` + `create_phase` (`traits.rs:550`, `:577`) and repoints the item via `update_item` with `ItemPatch.step_graph_id: Some(Some(id))` (`item.rs:222`). `phase_agent` copy and `is_override = true` are recorded in the fn doc as owed to a later writer; the binding-count test (§3.5) asserts `skill_binding` is untouched, which is true by construction. Carried as **R-6**. |
| F-M | ANA-2 §6.2 `AnswerGate { Rejected }` on a non-review phase: item "reject with note, terminal → failed" (`:578`) | D12 lists five `RunFailure` variants and none names a plain rejection | **A-8.** Sixth variant `RunFailure::Rejected { phase: String }` → `rejected: <phase>`. Same shape as D12's identifier-form strings. |
| F-N | D19: "`fake.rs` implementing it over `&MemStore`" | A trait impl on `&MemStore` makes the engine's `G` be `&MemStore` and the call site `&&MemStore` | `impl GraphSource for MemStore` (local trait, foreign type: allowed). The engine borrows `&G`, so the fake is used "over `&MemStore`" exactly as the plan says. |
| F-O | Files table: `UsageSpy` in `htui-agent/src/conformance.rs` not listed for T1 | `UsageSpy<'a, S: WriteStore>` hand-forwards every `WriteStore` method (`:1013-1019` for the run seam); so does `SpyStore` in `htui-agent/tests/recorder.rs:707-710` | T1 adds a `finish_run` arm to both, or `htui-agent` stops compiling under `--all-features`. Added to §9's T1 file set. |
| F-P | `writer_buffered.rs` doc: "All **23** new methods are listed here" (`:662`) | `finish_run` makes 24 | T1 edits the number and appends the `refused("finish_run", …)` entry after `fail_run`'s (`:892-897`). |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (minimum) | Validation |
|---|---|---|---|
| T1 `finish_run` | htui-core, htui-store, htui-agent (spies) | 4: conformance case + Mem; Pg + `.sqlx`; Writer pair + buffered test + spies; count pins + `pg_criteria` twin + banner | `cargo test -p htui-core --all-features && cargo test -p htui-agent --all-features && USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features -- --test-threads=1 && cargo clippy -p htui-core -p htui-store -p htui-agent --all-targets --all-features -- -D warnings && (cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features)` |
| T2 crate + `graph.rs` + `status.rs` | root manifest, htui-orch | 3: manifest + `lib.rs` (with `[lints]`); `status.rs`; `graph.rs` | `cargo test -p htui-orch --all-features && cargo clippy -p htui-orch --all-targets --all-features -- -D warnings` |
| T3 `command.rs`, `isolate.rs`, `fake.rs`, `conformance.rs` | htui-orch | 3: `isolate.rs` + `FakeIsolator` + clocks; `fake.rs` source/orchestrator; `command.rs` + suite skeleton | as T2 |
| T4 `engine.rs`, `gate.rs` | htui-orch | 3: settle + gate table + parser; engine stages 1–6; review loop | as T2 |
| T5 tests, fixtures, C-4 | htui-orch, htui-core (`run.rs` comment) | 2: fixtures + six criteria; C-4 marker | as T2, then the full workspace gate (plan Validation): `cargo fmt --all -- --check; cargo clippy --workspace --all-targets --all-features -- -D warnings; USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features -- --test-threads=1; cargo doc --workspace --no-deps` (six pre-existing htui-store errors, zero new); `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |

Tests first in every task; the first failing test is named in each section.

---

## 2. T1 — `finish_run`, the nineteenth writer

### 2.1 Trait (`crates/htui-core/src/store/traits.rs`)

Insert after `fail_run` (`:885`) and before `close_out` (`:903`), inside the `// ---- ANA-2 §8` block:

```rust
/// Ends a graph run and mirrors the item in the same transaction (ANA-2 §4.3 verdict 3 and
/// the propagation rule at `:660-664`; milestone 2 plan D7, blueprint R-1 option (b)).
///
/// `to` must be terminal (`done`, `failed`, `cancelled`). The run moves `<current> -> to` under
/// [`legal_move`], `finished_at = COALESCE(finished_at, at)`, and `failure` is written when
/// `to = failed`. Then, **only when no other run of the item is non-terminal**, the item moves:
///
/// | `to`        | item, from → to                                              |
/// |-------------|--------------------------------------------------------------|
/// | `done`      | `in_progress → done`                                         |
/// | `failed`    | `in_progress → failed`, `awaiting_approval → failed`         |
/// | `cancelled` | `queued → open`, `in_progress → open`, `awaiting_approval → open` |
///
/// An item at any other status is left alone (plan D17: the row may not have passed through
/// `can_move_to`), and an item that still has a live run is held where it is — the second
/// leg of the conformance case. A chat run (`item_id IS NULL`) moves only the run.
///
/// [`transition_run`](WriteStore::transition_run) and [`fail_run`](WriteStore::fail_run) are
/// unchanged and remain the run-only writers the chat path uses.
///
/// # Errors
/// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`;
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) with
/// [`finish_run_needs_a_terminal_status`] when `to` is not terminal, with
/// [`failure_disagrees_with_status`] when `failure.is_some() != (to == Failed)`, and with
/// [`illegal_move`]'s sentence when the run is already terminal.
async fn finish_run(
    &self,
    run: RunId,
    to: RunStatus,
    failure: Option<&str>,
    at: DateTime<Utc>,
) -> Result<()>;
```

Two refusal functions appended to the MOD-4 block after `run_is_terminal` (`:1123`):

```rust
/// `finish_run` was asked to leave a run non-terminal.
#[must_use]
pub fn finish_run_needs_a_terminal_status(run: RunId, to: RunStatus) -> String {
    format!("run {run}: `finish_run` moves to a terminal status, not `{to}` (ANA-2 §4.3)")
}

/// `finish_run`'s `failure` and `to` disagree: text on a non-failure, or none on a failure.
#[must_use]
pub fn failure_disagrees_with_status(run: RunId, to: RunStatus, has_failure: bool) -> String {
    if has_failure {
        format!("run {run}: a failure text is refused on a move to `{to}`")
    } else {
        format!("run {run}: a move to `failed` needs a failure text")
    }
}
```

Order of checks (both stores): lookup → `NotFound`; terminal-target → `Constraint`; failure/status agreement → `Constraint`; `legal_move(row.status, to)` → `Constraint` (a terminal row has no legal moves, so `run_is_terminal` is not needed); then the writes.

### 2.2 Banner (`traits.rs:670-675`)

Replace lines 672–673 with:

```
// In §8's order. Seven are transactions on every backend, `MemStore` included (plan M1 D6,
// M2 D7): `create_run`, `claim_run`, `select_fanout`, `write_document`, `promote_step`,
// `finish_run` and `close_out`. Every writer …
```

### 2.3 `MemStore` (`crates/htui-core/src/store/mem.rs`)

`State::finish_run` after `State::fail_run` (`:3741`), modelled on `State::promote_step` (`:3681-3720`) for the item half and `State::transition_run` (`:3366-3394`) for the run half:

```rust
/// Plan M2 D7: the run's terminal move and the item's mirror, one closure.
fn finish_run(
    &mut self,
    run: RunId,
    to: RunStatus,
    failure: Option<&str>,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<()> {
    let row = self.require_run(run)?;                       // NotFound first (D14)
    if !to.is_terminal() {
        return Err(StoreError::Constraint(finish_run_needs_a_terminal_status(run, to)));
    }
    if failure.is_some() != (to == RunStatus::Failed) {
        return Err(StoreError::Constraint(failure_disagrees_with_status(run, to, failure.is_some())));
    }
    legal_move(row.status, to)?;
    let item_id = row.item_id;
    if let Some(row) = self.runs.get_mut(&run) {
        row.status = to;
        row.failure = failure.map(str::to_owned).or(row.failure.take());
        row.finished_at = row.finished_at.or(Some(at));
        row.updated_at = now;
    }
    let Some(item) = item_id else { return Ok(()) };
    let another_live = self.runs.values()
        .any(|r| r.item_id == Some(item) && r.id != run && r.status.is_active());
    if another_live { return Ok(()); }
    let Some(status) = self.items.get(&item).map(|i| i.status) else { return Ok(()) };
    let target = match (to, status) {
        (RunStatus::Done, Status::InProgress) => Status::Done,
        (RunStatus::Failed, Status::InProgress | Status::AwaitingApproval) => Status::Failed,
        (RunStatus::Cancelled, Status::Queued | Status::InProgress | Status::AwaitingApproval) => Status::Open,
        _ => return Ok(()),                                  // D17: leave an unexpected item alone
    };
    self.transition(item, status, target, now)?;             // `closed_at` follows (`:1285`)
    Ok(())
}
```

`impl WriteStore for MemStore` arm after `fail_run` (`:4387-4390`), same shape: `let now = Utc::now(); self.write(|state| state.finish_run(run, to, failure, at, now))`. Extend the run-seam comment above `:4273` ("are each a **single** `write` closure") to name `finish_run`.

`RunStatus::is_active` exists (used at `mem.rs` close_out, `:3752`); it is the "non-terminal" predicate.

### 2.4 `PgStore` (`crates/htui-store/src/pg/write.rs`)

After `fail_run` (`:3411-3448`), a transaction modelled on `promote_step` (`:3323-3392`):

1. `SELECT status AS "status: RunStatus", item_id AS "item_id: ItemId", failure FROM run WHERE id = $1 FOR UPDATE` → `NotFound { entity: "run" }` on none.
2. The two `Constraint` checks, then `legal_move(row.status, to)?` — all before any `UPDATE`.
3. `UPDATE run SET status = $2, failure = COALESCE($3, failure), finished_at = COALESCE(finished_at, $4) WHERE id = $1` with `to` bound as `RunStatus` (sqlx `Type`) and `failure` as `Option<&str>`.
4. If `item_id` is `Some`: `SELECT count(*) FROM run WHERE item_id = $1 AND id <> $2 AND status IN ('queued','running','awaiting_approval')`; when `0`, one `UPDATE item SET status = $2, closed_at = CASE WHEN $3 THEN now() ELSE NULL END WHERE id = $1 AND status = ANY($4)` where `$2` is the target, `$3 = target.is_terminal()` (mirrors the `closed_at` rule of `transition`, `pg/write.rs:580-582`), `$4` the `from` set of the table in §2.1 as `&[Status]`. Zero rows affected is not an error (D17).
5. `tx.commit()`.

Three new `sqlx::query!` sites → regenerate `.sqlx` (F-E).

### 2.5 `Writer` / `BufferedWriter` (`crates/htui-store/src/writer.rs`)

- `BufferedWriter`: after the `fail_run` arm (`:738-740`), `async fn finish_run(&self, _run: RunId, _to: RunStatus, _failure: Option<&str>, _at: DateTime<Utc>) -> Result<()> { Err(hierarchy_needs_the_server()) }` (`:826-828`; no new constant, M1 D5).
- `Writer`: after `fail_run` (`:1570-1576`), the three-arm `match self { Memory | Online | Buffered }` delegation.
- `crates/htui-store/tests/writer_buffered.rs`: `:662` "23" → "24"; a `refused("finish_run", writer.finish_run(run, RunStatus::Done, None, at).await.expect_err("no run is finished offline"))` entry after `:897`.

### 2.6 Spies (F-O)

- `crates/htui-agent/src/conformance.rs` after `:1018`: `async fn finish_run(&self, run: RunId, to: RunStatus, failure: Option<&str>, at: DateTime<Utc>) -> StoreResult<()> { self.inner.finish_run(run, to, failure, at).await }`. `RunStatus` may need adding to the file's `htui_core::model` import.
- `crates/htui-agent/tests/recorder.rs` after `:711`: same arm on `SpyStore`.

### 2.7 Conformance (`crates/htui-core/src/store/conformance.rs`)

**First failing test**: `finish_run_moves_run_and_item_together` — fails with `E0599 no method named finish_run` until the trait lands. Case name appended to `CASES` after `"illegal_transitions_are_constraint"`; `run_case` arm before the panic (`:162`); count 47 → 48 in `crates/htui-core/tests/mem_store.rs:36-42` (append `, and MOD-4 milestone 2's one for \`finish_run\` (plan D7)` to the ledger string) and `crates/htui-store/tests/pg_conformance.rs:19`.

Legs, all on fresh runs minted with the module's `new_run(project, item, scope)` helper (`:3906` region, as `run_create_moves_the_item` uses at `:3888`):

1. `create_run` on `ids::HTUI_ANA_2` (item `open → queued`), `claim_run(run, ids::BOX, owner, at, lease)` → `Ok(true)` (item `queued → in_progress`, `mem.rs:3236-3244`), `finish_run(run, Done, None, at)` → run `done`, `finished_at == Some(at)`, item `done`, `closed_at.is_some()`, `version` unchanged.
2. Two runs on `ids::AGY_FEAT_1`: `create_run` ×2 (second `create_run` on a queued item — if `create_run` refuses a live run, claim the first then create the second; the case asserts whichever the store permits and the plan's Risks row "a second live run is lost" is the point). Finish the first `Done` → item stays `in_progress`; finish the second `Done` → item `done`.
3. `finish_run(run, Failed, Some("missing_output"), at)` on a claimed run → `run.failure == Some("missing_output")`, item `failed`.
4. `finish_run(run, Cancelled, None, at)` on a queued (unclaimed) run → item `open`.
5. `finish_run(run, Running, None, at)` → `Constraint` containing `"terminal status"`; `finish_run(run, Done, Some("x"), at)` → `Constraint` containing ``"refused on a move to `done`"``; `finish_run(run, Failed, None, at)` → `Constraint` containing `"needs a failure text"`.
6. Unknown run → `NotFound { entity: "run" }`, checked before the terminal-target check (pass `Running` to prove precedence).
7. Finishing a `done` run again → `Constraint` with `illegal_move`'s sentence (``"run.status `done` cannot move to `done` (ANA-2 §4.3)"``).

Doc comment names the Pg twin `pg_criteria.rs::finish_run_holds_the_item_while_another_run_is_live`; add it to `PENDING` (`:6396`) until T1's Pg commit lands, then remove it (the drift test at `:6438-6444` enforces the removal).

### 2.8 `pg_criteria` twin (`crates/htui-store/tests/pg_criteria.rs`)

`finish_run_holds_the_item_while_another_run_is_live` after `document_versions_do_not_collide_under_contention` (`:3873`): the `common::demo_db()` skip prologue (`:481-483`), leg 2 of §2.7 against Postgres, plus a `SELECT closed_at` assertion that the item's `closed_at` is set by the `done` move and NULL after a `cancelled` one.

---

## 3. T2 — the crate, `graph.rs`, `status.rs`

### 3.1 Manifests

`Cargo.toml:2` → `members = ["crates/htui-core", "crates/htui", "crates/htui-store", "crates/htui-agent", "crates/htui-orch"]`; after `:25` add `htui-orch = { path = "crates/htui-orch" }`.

`crates/htui-orch/Cargo.toml` (mirrors `crates/htui-agent/Cargo.toml`, order per plan Patterns row 1):

```toml
[package]
name = "htui-orch"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true
description = "The step-graph orchestrator: ANA-2 §4.2's six-stage walk, headless (R-ORCH-12)"

[features]
default = []
test-support = ["htui-core/test-support", "htui-agent/test-support"]

[dependencies]
htui-core = { workspace = true }
htui-agent = { workspace = true }
chrono = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["sync"] }
tracing = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
htui-core = { workspace = true, features = ["test-support"] }
htui-agent = { workspace = true, features = ["test-support"] }
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread"] }

[lints]
workspace = true
```

No `htui-store`, no `sha2` (the hasher is `htui_core::prompt::digest::sha256_hex`, `digest.rs:73-75`). First commit = manifest + `lib.rs` + empty-but-documented modules, with the `[lints]` block (D1).

### 3.2 `src/lib.rs`

```rust
//! `htui-orch`: the step-graph orchestrator (`docs/ANA-2.md` §4.2), milestone 2 — a graph walks.
//! Depends on `htui-core` and `htui-agent`, never `htui-store` (invariant 10, ANA-2 `:143-146`).
#![warn(missing_docs)]

pub mod command;
#[cfg(feature = "test-support")]
pub mod conformance;
pub mod engine;
#[cfg(feature = "test-support")]
pub mod fake;
pub mod gate;
pub mod graph;
pub mod isolate;
pub mod status;

pub use command::{Command, CommandOutcome, GateAnswer};
pub use engine::{AgentSelector, Clock, Engine, EngineError, FirstCandidate, NoSink, Rest, SessionSink, SystemClock};
pub use gate::{Settle, Verdict, parse_verdict};
pub use graph::{GraphSource, Resolved, ResolveError, resolve, topology};
pub use isolate::{Isolator, IsolatorFuture, IsolateError, Prepared, PreparedTree};
pub use status::{Cursor, RunFailure, cursor, may_attempt};
```

### 3.3 `src/status.rs`

```rust
/// ANA-2 §4.2's retry admission, prospective (plan D3): may attempt `next` be *created*?
#[must_use]
pub const fn may_attempt(next_attempt: i32, retry_limit: i32) -> bool {
    next_attempt <= retry_limit + 1
}

/// `run.failure` and refusal texts, ANA-2's exact bytes (plan D12; F-M adds `Rejected`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    /// Stage 3: an `input_kinds` entry resolved to no document (`docs/ANA-2.md:414`).
    MissingInput(String),
    /// Stage 5: no document of `output_kind` produced by this step (`:430`).
    MissingOutput,
    /// Stage 1: no candidate survives the inline-approval interlock (`:482`).
    MissingCapability,
    /// §4.4 escalation (`:746`).
    ReviewLoopExhausted(i32),
    /// §4.4 step 1 found no `p_impl` (plan D5).
    NoLoopTarget,
    /// A human rejected a non-loopable gated step (ANA-2 `:578`).
    Rejected { phase: String },
}
impl core::fmt::Display for RunFailure { /* exact strings:
    "missing input document: {kind}" | "missing_output" | "missing_capability: inline_approval"
    | "review loop exhausted after {n} attempts" | "no_loop_target" | "rejected: {phase}" */ }

/// Where the walk is, re-derived from `run_steps` on every call (plan D16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    /// No live or finished step at `position`: create `(position, attempt, 0)` pending.
    Create { position: i32, attempt: i32 },
    /// A `pending` step exists: run it.
    Run(StepId),
    /// A step is `running`, `awaiting_approval` or `failed`: the walk cannot advance.
    Rest { step: StepId, status: StepStatus },
    /// Every position has a `done` step at its latest attempt.
    Finished,
}

/// The latest-attempt step at `position` (`fanout_index = 0` only this milestone).
#[must_use]
pub fn latest_at(steps: &[RunStep], position: i32) -> Option<&RunStep>;
/// `max(attempt at position) + 1`, or `1`.
#[must_use]
pub fn next_attempt(steps: &[RunStep], position: i32) -> i32;
/// Walks `snapshot.phases` in position order; the first position whose latest step is not
/// `done` decides. `superseded`/`cancelled` at the latest attempt count as "no live step" and
/// yield `Create` with `next_attempt` — the lazy re-insertion plan D5 needs.
#[must_use]
pub fn cursor(snapshot: &GraphSnapshot, steps: &[RunStep]) -> Cursor;
```

Unit tests: `Display` bytes for all six; `may_attempt(2, 1)` true and `may_attempt(3, 1)` false; `cursor` on fixture `RUN_1`'s four done steps → `Finished`, on `RUN_2`'s pending prd → `Run`, on a superseded implement → `Create { attempt: 2 }`. **First failing test**: `run_failure_display_is_ana2s_bytes`.

### 3.4 `src/graph.rs` — `GraphSource` (D19)

```rust
/// What `htui-orch` needs from a store to build a `GraphSnapshot` and that no `ReadStore`
/// method answers (plan D19): the four inherent reads of `MemStore` (`mem.rs:459-504`) and
/// `Backend` (`backend.rs:440-593`). Plain `async fn` with the targeted allow, mirroring
/// `crates/htui-core/src/store/traits.rs:62`: the engine is already generic over `S: WriteStore`
/// and takes `G` beside it, so no `dyn` is ever formed and a boxed-future alias
/// (`driver.rs:37`) would buy nothing but an allocation per read. `fake.rs` implements it for
/// `MemStore`; `htui` implements it for `Backend` at milestone 6.
#[allow(async_fn_in_trait)]
pub trait GraphSource: Sync {
    /// `MemStore::resolve_graph` (`mem.rs:502`): `item.step_graph_id`, else the kind's default.
    async fn resolve_graph(&self, item: ItemId) -> Result<Option<ResolvedGraph>>;
    /// Rung 1 of ANA-2 §4.1's candidate chain (`:290`), in `position` order.
    async fn phase_agents(&self, phase: PhaseId) -> Result<Vec<PhaseAgent>>;
    /// `(name, version)` or the latest of `name` when `version` is `None` (`mem.rs:480-495`).
    async fn prompt_template(&self, project: ProjectId, name: &str, version: Option<i32>) -> Result<Option<PromptTemplate>>;
    /// The `agent` row, for `agent_name` (ANA-2 §5.1) and `registry::caps_for` (plan D6).
    async fn agent(&self, id: AgentId) -> Result<Option<Agent>>;
}
```

Resolution:

```rust
/// The snapshot and the scope, ready for `NewRun` (`model/run.rs:353-378`).
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved { pub snapshot: GraphSnapshot, pub repo_scope: Vec<RepoId> }

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ResolveError {
    #[error("item {0} has no step graph")] NoGraph(ItemId),
    #[error("phase `{phase}` names template `{name}` which project {project} does not hold")] NoTemplate { .. },
    #[error("phase `{phase}` has no candidate agent (ANA-2 §4.1 rung 4)")] NoCandidate { phase: String },
    #[error("phase `{phase}` candidate {agent} names no agent row")] NoAgentRow { .. },
    #[error("phase `{phase}` is `local` with fan_out {fan_out} (ANA-2 §4.6)")] LocalFanOut { .. },
    #[error("item {item} names primary repo {repo}; an empty repo_scope is refused (plan D14)")] EmptyScopeWithPrimary { item: ItemId, repo: RepoId },
    #[error(transparent)] Store(#[from] StoreError),
}

/// Builds §5.1's snapshot from the live graph (ANA-2 §4.1 chains, `:284-291`).
pub async fn resolve<S: ReadStore + WriteStore, G: GraphSource>(
    store: &S, source: &G, item: &Item, mode: RunMode,
    app: &BTreeMap<String, Value>,             // F-G: the `app_setting` rung
    requested_scope: Option<&[RepoId]>,
) -> Result<Resolved, ResolveError>;
```

Per phase (input `ResolvedPhase`, `kind.rs:309-314`; output `SnapshotPhase`, `run.rs:482-521`, never a third type — D15):

| Snapshot field | Source |
|---|---|
| `gate`, `gate_effective` | `phase.gate`; `gate_effective = gate` in manual mode (§4.10 downgrade is auto-mode, milestone 7+) |
| `isolation` | `phase.isolation` → `ProjectSettings::deserialize(project.settings).default_isolation` (`kind.rs:256`, `#[serde(default)]` gives `Worktree`) |
| `token_budget` | `phase.token_budget` → `settings.token_budget` → `app["token_budget"]` as `i32` → `None` |
| `template` | `SnapshotTemplate { name: phase.template_name, version }` where `version` = `phase.template_version` or `source.prompt_template(project, name, None)?.version` (latest); missing → `NoTemplate` |
| `deadline_seconds` | `StepGraphPhase` has no `deadline_seconds` field at HEAD (`kind.rs:177-212`; the column exists, `0003_orchestration.sql:19`) → chain starts at `settings.step_deadline_seconds` → `app["step_deadline_seconds"]` → `Some(7200)` |
| `candidates` | `source.phase_agents(phase.id)` → `settings.default_agent_id` as one candidate with `agent.default_model` → empty (rung 3 is the fake's, F-F) → `NoCandidate`. Each mapped through `source.agent(id)` for `agent_name`; `model` from `PhaseAgent.model` |
| `judge` | `settings.judge_agent_id` → `None` (`StepGraphPhase` has no judge fields at HEAD) |
| `verify_command`, `command_queue`, `fan_out`, `gate_hard`, `retry_limit`, `input_kinds`, `output_kind`, `position`, `name` | copied |

`LocalFanOut` refused when `isolation == Local && fan_out > 1` (ANA-2 `:1003-1006`). Positions re-numbered dense `0..n` in `phase.position` order (ANA-2 `:275-277`).

`SnapshotSettings`: `default_isolation` as resolved; caps from `ProjectSettings`; `max_fan_out` and `max_agents_per_run` from `app["max_fan_out"]`/`app["max_agents_per_run"]` else `4`/`6` (the §5.1 example values).

Topology (D9):

```rust
/// `"sha256:"` + hex over `serde_json::to_string(phases)` on the **typed** slice. Never through
/// `serde_json::Value`: `preserve_order` is feature-unified on in a workspace build (via
/// `schemars` ← `agent-client-protocol-schema` ← `htui-agent`) and off in `cargo test -p`, so a
/// `Value` round trip yields two digests for one graph (plan D9, fact-check F2d).
#[must_use]
pub fn topology(phases: &[SnapshotPhase]) -> String {
    let json = serde_json::to_string(phases).expect("SnapshotPhase serialises: no map keys, no non-string keys");
    format!("sha256:{}", htui_core::prompt::digest::sha256_hex(&json))
}
```

Scope (D14): `resolve_scope(item, repos: &[Repo], requested) -> Result<Vec<RepoId>, ResolveError>`: `requested` `Some(scope)` is used as given, except `Some([])` with a primary repo → `EmptyScopeWithPrimary`; `None` → `[primary.id]` when the project has a primary (`Repo.is_primary`, `hierarchy.rs:136`), else `[]`. `repos` comes from `WriteStore::repos(project)` (`traits.rs:489`). Demo has no repos (`mem.rs:196` `repos: HashMap::new()`), so the walk's scope is `[]` unless a test creates one.

Override clone (F-L):

```rust
/// `<item.key>-override`: a deep clone over `step_graph_phase`, never `skill_binding`
/// (ANA-2 §4.1 `:294-300`, PRD `:399`). `phase_agent` and `is_override = true` await their
/// writers (R-6).
pub async fn override_graph<S: WriteStore, G: GraphSource>(store: &S, source: &G, item: &Item) -> Result<StepGraph, ResolveError>;
```

Tests (`graph.rs` `mod tests`, over `MemStore::demo()` + a local `GraphSource` impl for the test only — the fake lives in T3, so T2's tests implement the trait on a tiny struct holding the store and a fixed candidate): `feature_snapshot_topology_is_pinned` (the digest string literal of the seeded `feature` graph resolved with candidate `(AGENT_CLAUDE, "claude", "sonnet")`, template versions `1`, deadline `7200`, isolation `worktree`; the literal is computed once and pasted — **this is the first failing test**); `field_chains_walk_phase_project_app` (one assertion per row of the table above, values driven through `Project.settings` and `app`); `local_with_fan_out_is_refused`; `empty_scope_with_primary_is_refused` (creates a primary repo through `create_repo`, `traits.rs:470`); `override_clone_leaves_bindings_alone` (`MemStore::bound_skills(project, Some(new_phase))` (`mem.rs:353`) equals `bound_skills(project, None)`, and the item's `step_graph_id` now names the clone); `positions_are_dense`.

---

## 4. T3 — `command.rs`, `isolate.rs`, `fake.rs`, `conformance.rs`

### 4.1 `src/isolate.rs` — the seam only (D6)

Dyn-compatible, mirroring `driver.rs:37` and `:363`, because milestone 3's gix implementation and the fake must be swappable behind one `&dyn Isolator` held by a `FakeOrchestrator` that owns neither:

```rust
/// The boxed future every [`Isolator`] method returns; the `DriverFuture` shape (`driver.rs:37`).
pub type IsolatorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, IsolateError>> + Send + 'a>>;

#[derive(Debug, thiserror::Error)]
pub enum IsolateError {
    #[error("isolation refused: {0}")] Refused(String),          // ANA-2 §4.6 "Refused when" column
    #[error("git: {0}")] Git(String),                             // milestone 3
    #[error(transparent)] Io(#[from] std::io::Error),
}

/// One tree the step will work in, with the hash the step starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedTree { pub tree: RunStepTree, pub before_hash: String }

/// Stage 2's result: the trees, and the directory the agent session runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared { pub trees: Vec<PreparedTree>, pub cwd: PathBuf }

/// ANA-2 §4.6's four verbs (`:1762-1770`). The engine persists what these return
/// (`upsert_step_tree`, `record_commits`); an isolator touches no store.
pub trait Isolator: Send + Sync + core::fmt::Debug {
    /// Stage 2: one tree per repo in `scope` under `isolation`, `before_hash = HEAD` per repo.
    fn prepare<'a>(&'a self, run: RunId, step: StepId, scope: &'a [RepoId], isolation: Isolation) -> IsolatorFuture<'a, Prepared>;
    /// Stage 5: `after_hash` per tree, `None` when the step committed nothing.
    fn capture<'a>(&'a self, step: StepId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Vec<RunStepCommit>>;
    /// The winner's branch into the primary tree; returns the merge hash per repo (§4.6 step 4).
    fn reconcile<'a>(&'a self, winner: StepId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, Vec<RunStepCommit>>;
    /// Run-terminal cleanup, never at step end (invariant 6).
    fn cleanup<'a>(&'a self, run: RunId, trees: &'a [RunStepTree]) -> IsolatorFuture<'a, ()>;
}
```

`RunStepCommit` (`run.rs:574-583`) is reused as the capture record so `record_commits` (`traits.rs:856`) takes it verbatim.

### 4.2 `src/fake.rs` — `FakeIsolator`, clocks

```rust
/// Synthetic hashes, no filesystem (plan T3: "creating no directory tree that outlives the
/// test" — it creates none at all). Deterministic: `before` is `fake:base:<n>`, `after` is the
/// next scripted value or `fake:after:<step>:<n>`, `n` a per-isolator counter.
#[derive(Debug, Default)]
pub struct FakeIsolator {
    after: Mutex<VecDeque<Option<String>>>,   // scripted `after_hash` per capture, FIFO
    calls: Mutex<u32>,
}
impl FakeIsolator {
    pub fn new() -> Self;
    /// Queue the `after_hash` the next `capture` returns for every repo (criterion 7 pins
    /// two identical values; `None` is "committed nothing").
    pub fn script_after(&self, hash: Option<&str>);
}
impl Isolator for FakeIsolator { /* prepare: one RunStepTree per repo, mode = isolation,
    path = "/fake/trees/<run>/<step>/<repo>", base_ref = before_hash, dirty = false;
    cwd = "/fake/trees/<run>/<step>"; reconcile: echoes capture; cleanup: no-op */ }
```

```rust
/// `Utc::now()` truncated to the column (plan D8).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;
/// A clock a test moves: starts at `htui_agent::conformance::epoch()` (`:276-278`) so every
/// stamp shares the fake driver's origin; `advance` is how a step-deadline case elapses time.
#[derive(Debug)]
pub struct TestClock { now: Mutex<DateTime<Utc>> }
impl TestClock { pub fn at(now: DateTime<Utc>) -> Self; pub fn advance(&self, by: Duration); pub fn set(&self, to: DateTime<Utc>); }
```

`trait Clock` itself is in `engine.rs` (§5.1); `SystemClock` lives beside it and `TestClock` here behind `test-support`.

### 4.3 `src/fake.rs` — `FakeGraphSource`, `FakeOrchestrator`, the scripted document (D13)

```rust
/// `GraphSource` over `MemStore` (plan D19). Rung 1 is the store's (always empty, `mem.rs:469-472`);
/// rung 3 is computed over `MemStore::agents()` (`mem.rs:283`) — also empty on the demo fixture,
/// whose three agents are all enabled (`model/agent.rs:156`); then `candidates`, which the harness
/// sets and which defaults to `(AGENT_CLAUDE, "sonnet")` for every phase (F-F).
#[derive(Debug)]
pub struct FakeGraphSource<'a> { store: &'a MemStore, candidates: BTreeMap<String, Vec<PhaseAgent>> }
impl<'a> FakeGraphSource<'a> {
    pub fn new(store: &'a MemStore) -> Self;
    pub fn with_candidates(self, phase: &str, agents: Vec<(AgentId, &str)>) -> Self;
    /// No candidate for `phase` at all: the rung-4 refusal case.
    pub fn without_candidates(self, phase: &str) -> Self;
}
impl GraphSource for MemStore { /* the four inherent reads, verbatim */ }   // F-N
impl GraphSource for FakeGraphSource<'_> { /* delegates, then the map for phase_agents */ }
```

`FakeGraphSource::phase_agents` needs the phase *name* from a `PhaseId`. Registering candidates by name and resolving through the `ResolvedGraph` the orchestrator already holds is the clean path: `FakeOrchestrator::start` passes the graph's phases to the source before `resolve`, so the source can build a `PhaseId → name` map once. The test surface is `with_candidates(name, …)`.

```rust
/// One phase's scripted behaviour: the driver script plus the document the harness writes
/// after `Done` (plan D13 — the test-side stand-in for MOD-11's `document_write`).
#[derive(Debug, Clone)]
pub struct ScriptedStep { pub script: Script, pub output: Option<String> }
impl ScriptedStep {
    /// `Emit(Done { EndTurn })` and a body of `"<phase> v<attempt>"` — the happy path.
    pub fn done_with_output(body: &str) -> Self;
    /// `Done` and no document: the `missing_output` path.
    pub fn done_without_output() -> Self;
    /// A review whose body starts with ANA-5's three-line front matter (`ANA-5.md:1322-1336`).
    pub fn review(verdict: &str, body: &str) -> Self;
    pub fn failing(stop: StopReason) -> Self;                // Refusal | MaxTokens | MaxTurnRequests
    pub fn erroring(code: &str, message: &str) -> Self;      // Emit(Error)
}

/// `MemStore` + `FakeDriver` + `FakeIsolator` + `TestClock` + `FakeGraphSource` (ANA-2 `:1762`).
#[derive(Debug)]
pub struct FakeOrchestrator {
    pub store: MemStore, pub isolator: FakeIsolator, pub clock: TestClock,
    scripts: Mutex<BTreeMap<(String, i32), ScriptedStep>>,   // (phase name, attempt) → step
    default_script: ScriptedStep,
    caps: DriverCaps,
}
impl FakeOrchestrator {
    pub fn demo() -> Self;                                    // MemStore::demo(), full_caps, done_with_output
    pub fn script(&self, phase: &str, attempt: i32, step: ScriptedStep);
    pub fn with_caps(self, caps: DriverCaps) -> Self;
    /// Builds the engine over borrowed parts and dispatches one command (§4.4).
    pub async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError>;
    pub async fn steps(&self, run: RunId) -> Vec<RunStep>;   // `run_steps`, `(position, attempt)` order
    pub async fn item(&self, id: ItemId) -> Item;
    pub async fn run(&self, id: RunId) -> Run;
}
```

The driver: `dispatch` builds a `FakeDriver::new(FAKE_AGENT_NAME, caps, script)` (`fake.rs:117`) per step from the `(phase, attempt)` script — `FakeDriver` plays its script once (`fake.rs:158-183`), so one driver per session is the fake's own rule. `FakeOrchestrator` implements `SessionSink` (§5.1): after `Done`, when the step's `ScriptedStep.output` is `Some`, it calls `WriteStore::write_document(NewDocument { id: DocumentId::new(), item_id, kind: phase.output_kind, title: format!("{kind} (attempt {attempt})"), body, produced_by_step_id: Some(step.id), created_by: this_user, created_at: clock.now() })` (`document.rs:73-90`; `MemStore::this_user`, `mem.rs:161`). The engine never writes a document.

Box and user: `MemStore::demo()` sets `this_box = Some(ids::BOX)` (`fixtures.rs:375`); the owner uuid is minted once per orchestrator; `created_by = this_user()`.

### 4.4 `src/command.rs`

```rust
/// ANA-2 §6.2's three verbs this milestone answers (`:1551-1571`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `open → queued`, then claim and walk to rest.
    StartRun { item: ItemId, mode: RunMode, repo_scope: Option<Vec<RepoId>> },
    /// Enabled when the step is `awaiting_approval`; `Approved` also needs the output document.
    AnswerGate { run: RunId, step: StepId, answer: GateAnswer },
    /// Enabled when the step is `awaiting_approval | failed` and `may_attempt(attempt + 1, retry_limit)`.
    RetryStep { run: RunId, step: StepId },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateAnswer { Approved, Rejected { note: String }, Skipped }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Started { run: RunId, rest: Rest },
    Answered { rest: Rest },
    Retried { step: StepId, rest: Rest },
}

/// The enabling guards, pure over rows, so the Runs tab (milestone 6) and the engine agree.
#[must_use] pub fn answer_gate_enabled(step: &RunStep, has_output: bool, answer: &GateAnswer) -> Result<(), EngineError>;
#[must_use] pub fn retry_enabled(step: &RunStep, phase: &SnapshotPhase) -> Result<(), EngineError>;
```

**`AnswerGate` and `RetryStep` carry `run: RunId`** because there is no per-step read on the seam — `ReadStore::run_steps` is per run (`traits.rs:149`) and no `step(id)` exists (H-11).

`Command::dispatch` is `Engine::dispatch(&self, command)` in `engine.rs`; `command.rs` holds the types and guards only.

### 4.5 `src/conformance.rs` (D18)

```rust
/// What a case needs: a fresh orchestrator per case, nothing named.
pub trait CaseHarness: Sync {
    type Orch: Orchestrate;
    fn fresh(&self) -> Self::Orch;
}
/// The surface a case drives — `FakeOrchestrator` implements it; a milestone-3 harness with a
/// real `Isolator` implements it too, and adds no case.
#[allow(async_fn_in_trait)]
pub trait Orchestrate {
    async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError>;
    fn store(&self) -> &MemStore;
    fn script(&self, phase: &str, attempt: i32, step: ScriptedStep);
    fn isolator(&self) -> &FakeIsolator;
    fn clock(&self) -> &TestClock;
}

pub const CASES: &[&str] = &[
    "feat_walks_end_to_end",                  // criterion 1
    "live_run_ignores_a_gate_edit",           // criterion 2
    "topology_mismatch_parks_on_resume",      // criterion 3
    "approve_needs_the_output_document",      // criterion 5
    "review_rejection_loops_then_escalates",  // criterion 6
    "identical_after_hash_stops_the_loop",    // criterion 7
    "missing_input_fails_before_a_token",     // ANA-2 :414
    "missing_output_settles_failed",          // ANA-2 :430
    "cli_agent_is_refused_at_a_gated_phase",  // ANA-2 :482
    "step_deadline_settles_failed",           // ANA-2 :439, plan D8
    "finish_run_is_the_last_write",           // run done ⇒ item done, one write
];
pub async fn run_case<H: CaseHarness>(name: &str, harness: &H);
pub async fn run_all<H: CaseHarness>(harness: &H);
```

Dispatcher panic: ``unknown conformance case `{other}`; CASES and run_case disagree``. Drift test `every_case_name_dispatches` runs each `CASES` entry against `FakeOrchestrator::demo()` (mirrors `htui-agent/src/conformance.rs:3069`'s shape). **First failing test in T3**: `cases_are_unique_and_eleven`. The case bodies are T5's; T3 lands them as stubs that assert only `fresh()` works, so the drift test is green at T3's gate and each T5 case replaces one stub.

---

## 5. T4 — `engine.rs`, `gate.rs`

### 5.1 Engine surface

```rust
/// Plan D8's seam: the one place an instant comes from.
pub trait Clock: Send + Sync {
    /// Now, already `trunc_subsecs(TIMESTAMPTZ_DIGITS)` (`model/run.rs:272`).
    fn now(&self) -> DateTime<Utc>;
}
#[derive(Debug, Default, Clone, Copy)] pub struct SystemClock;   // Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS)

/// Stage 1's third duty (plan D6): which surviving candidate runs.
pub trait AgentSelector: Send + Sync {
    fn select<'c>(&self, phase: &SnapshotPhase, eligible: &'c [SnapshotCandidate]) -> Option<&'c SnapshotCandidate>;
}
/// The only selector this milestone: `eligible.first()`.
#[derive(Debug, Default, Clone, Copy)] pub struct FirstCandidate;

/// Called between stage 4 and stage 5 (plan D13). Production is [`NoSink`].
#[allow(async_fn_in_trait)]
pub trait SessionSink: Sync {
    async fn after_done(&self, step: &RunStep, phase: &SnapshotPhase, done: &DoneEvent) -> Result<(), StoreError>;
}
#[derive(Debug, Default, Clone, Copy)] pub struct NoSink;

/// Where a walk stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rest { pub run: RunStatus, pub position: Option<i32>, pub failure: Option<RunFailure> }

/// The walk, generic over everything it touches and holding nothing between calls (plan D16).
pub struct Engine<'a, S, G, I, C, A, K>
where S: WriteStore, G: GraphSource, I: Isolator + ?Sized, C: Clock, A: AgentSelector, K: SessionSink {
    store: &'a S, graphs: &'a G, isolator: &'a I, clock: &'a C, selector: &'a A, sink: &'a K,
    driver: &'a dyn Fn(&SnapshotCandidate) -> Box<dyn AgentDriver>,   // one driver per session (§4.3)
    scrubber: &'a dyn Scrubber,
    app: BTreeMap<String, Value>,                                      // F-G
    box_profile: BoxProfile,                                           // H-11
    box_id: BoxId, owner: Uuid, user: UserId,
}
impl<'_, …> Engine<'_, …> {
    pub fn new(parts: EngineParts<'a, …>) -> Self;                     // a struct literal, not a builder
    pub async fn dispatch(&self, command: Command) -> Result<CommandOutcome, EngineError>;
    /// Stages 1–6 per position until the run rests (`awaiting_approval`, terminal, or refused).
    pub async fn run_to_rest(&self, run: RunId) -> Result<Rest, EngineError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)] Store(#[from] StoreError),
    #[error(transparent)] Resolve(#[from] ResolveError),
    #[error("run {run} is `{status}`; expected `{expected}`")] RunStatus { run: RunId, status: RunStatus, expected: &'static str },
    #[error("step {step} is `{status}` and cannot be answered (ANA-2 §6.2)")] NotGated { step: StepId, status: StepStatus },
    #[error("step {step} has no `{kind}` document; approve needs one (ANA-2 §6.2)")] MissingOutputForApproval { step: StepId, kind: String },
    #[error("step {step} is at attempt {attempt}; retry_limit {retry_limit} permits no more")] RetryExhausted { .. },
    #[error("item {item} is blocked; milestone 6's unblock clears it (R-4)")] ItemBlocked { item: ItemId },
    #[error("claim refused: the box is full or the scope overlaps (ANA-2 §4.7)")] ClaimRefused { run: RunId },
    #[error("run {run} snapshot v{v} is not readable by this engine")] SnapshotVersion { run: RunId, v: u32 },
    #[error(transparent)] Prompt(#[from] AssembleError),
    #[error(transparent)] Record(#[from] RecordError),
    #[error(transparent)] Driver(#[from] DriverError),
}
```

The driver factory is a closure rather than `&dyn AgentDriver` because `FakeDriver` plays one script once (`fake.rs:158-183`) and milestone 3's registry builds a driver per `(agent, box)` anyway (`registry.rs:132`).

### 5.2 `dispatch`

- `StartRun { item, mode, repo_scope }`: `item_row` (`ReadStore::item`, `traits.rs:69`) → `project` (`:128`) → `graph::resolve` → `create_run(NewRun { id: RunId::new(), project_id, item_id: item, mode, target_box_id: box_id, started_by: user, graph_snapshot, repo_scope, queued_at: now })` (`run.rs:353-378`; item `open → queued` inside) → `claim_run(run, box_id, owner, now, now + 120s)` (`traits.rs:709-716`; `Ok(false)` → `ClaimRefused`, run stays queued — R-2 untouched, D6) → `run_to_rest`.
- `AnswerGate { run, step, answer }`: the step row via `run_steps(run)`. Guards: status `awaiting_approval` else `NotGated`; `Approved` requires `documents_of_kinds(item, [output_kind])` (`:100`) to contain one with `produced_by_step_id == Some(step)` else `MissingOutputForApproval` and **no write** (criterion 5). Then `answer_gate(step, outcome, note, now)` (`:793-799`: `Approved|Skipped → done`, `Rejected → failed`) → per-answer follow-up (§5.5) → `run_to_rest`.
- `RetryStep { run, step }`: `retry_enabled` (`awaiting_approval | failed`; `may_attempt(step.attempt + 1, phase.retry_limit)`); item `blocked` → `ItemBlocked` (F-J); `awaiting_approval` → `answer_gate(step, Retried, None, now)` (→ `superseded`); `failed` left alone (D5); `create_step((position, attempt + 1, 0))` pending; run `awaiting_approval → running` + item `awaiting_approval → in_progress` when they are there → `run_to_rest`.

### 5.3 `run_to_rest` — the six stages per position

Loop: read `run` (`:141`) — match exhaustively (D17): `Running` proceeds; `Queued` → `RunStatus { expected: "running" }`; `AwaitingApproval` → `Rest`; terminal → `Rest`. Decode `graph_snapshot` (`serde_json::from_value::<GraphSnapshot>`), `v != GraphSnapshot::V` → `SnapshotVersion`. `steps = run_steps(run)`, `cursor(&snapshot, &steps)`:

- `Finished` → `finish_run(run, Done, None, now)` → `Rest { Done, None, None }`.
- `Rest { .. }` → return.
- `Create { position, attempt }` → **stage 1** (below) then `create_step(NewRunStep { id, run_id, position, attempt, fanout_index: 0, phase_name, agent_id: Some(c.agent_id), model: Some(c.model) })` (`run.rs:384-401`), fall into `Run`.
- `Run(step)` → stages 2–6, then loop.

**Stage 1 — admit.** Capability interlock (ANA-2 `:478-484`): for each `phase.candidates`, `graphs.agent(id)` → `caps_for(&agent)` (`registry.rs:152`); when `phase.gate_effective != Never`, drop a candidate with `!(caps.permission_requests || caps.edit_proposals)`; none left → refuse in this order: `transition(item, InProgress, Blocked)` **first** (legal, `item.rs:52`), then `finish_run(run, Failed, Some(&RunFailure::MissingCapability.to_string()), now)`, which finds the item at `blocked` and leaves it (the `_ =>` arm); `add_note(NewNote { body: "missing_capability: inline_approval", … })` (`note.rs:34-56`, `traits.rs:915`). Return `Rest { Failed, Some(position), Some(MissingCapability) }`. Selection: `selector.select(phase, &eligible)`.

**Stage 2 — prepare.** `transition_step(step, Pending, Running, now)` (`:772-778`; `Ok(false)` → re-read and `Rest`). `isolator.prepare(run, step, &run.repo_scope, phase.isolation)`; `upsert_step_tree(step, &trees)` (`:847`); `record_commits(step, &[RunStepCommit { run_step_id, repo_id, before_hash, after_hash: None }])` (`:856`).

**Stage 3 — prompt.** `resolve_inputs(item, run, &phase.input_kinds)` (`:176-181`); a required kind resolving to `None` → hard failure before a token: `transition_step(step, Running, Failed, now)`, `finish_run(run, Failed, Some("missing input document: <kind>"), now)`, `Rest`.

**The back-edge rule (H-8).** The seeded `implement` phase lists `review` in `input_kinds` (`seed.rs:87`), which cannot exist on attempt 1, while ANA-2 `:412-414` makes a missing input a hard failure — read literally, criterion 1 fails at position 2. Rule adopted: **a kind is required when it is not the `output_kind` of a later position in the snapshot; a kind produced later (the loop's back-edge) is optional and its absence is recorded in the trim record's notes.** This is the only reading under which ANA-2 §4.1's seed amendment and §4.2's hard failure coexist. Documented on `engine::required_inputs`.

Then `graphs.prompt_template(project, &template.name, Some(template.version))`, a `PromptSpec` literal per `preview.rs:229-262` with: `role: TemplateRole::of_name(&name)`, `template: TemplateRef { name, version }`, `body: template.body`, `item_key`, `phase: phase.name`, `output_kind: Some(phase.output_kind)`, `attempt: step.attempt`, `documents` from the resolved inputs, `upstream` via `upstream_summaries` (`:113`), `box_profile` from `EngineParts` (H-11), `skills: Vec::new()` (milestone 6 fills them), `excerpts: ExcerptSet::empty(caps)` per `preview.rs:271`, `command_queue: phase.command_queue != CommandQueue::Off`, `verify_failure: None`, `previous_diff: None` (both milestone 3), `budget: settings::resolve_budget(…)`, `estimator: TokenEstimator::DEFAULT`. `assemble(&spec, scrubber)` → `set_step_prompt(step, &prompt.digest, &serde_json::to_value(&prompt.trim)?)` (`traits.rs:356`).

**Stage 4 — session.** `SessionSpec { agent_id, step_id: step, cwd: prepared.cwd, extra_dirs: vec![], env: BTreeMap::new(), model: Some(model), tools: ToolExposure::default(), mcp: vec![], permission: PermissionPolicy::default(), retain_raw, resume: None, budget_micros }` (`driver.rs:250-273`). `Recorder::new(store, scrubber, step, retain_raw, None)` (`record.rs:455`), `.with_run_cap(…)` when capped (`:515`), `record_prompt(&prompt.text, prompt.payload_sections_value(), now)` (`:572`), `driver.start(spec, prompt.text)` (`driver.rs:376-380`), `pump(&mut *session, &mut recorder)` (`record.rs:1684-1687`) → `Ok(DoneEvent)` or `Err(DriverError)` (a closed stream is an error, not a `Done`), then `recorder.finish()` (`:928`) → summary (`cap_breach` read here). Then `sink.after_done(&step, &phase, &done)`.

**Stage 5 — settle.** `gate::settle(...)` (§5.4) with the driver result, the summary, the deadline, the output document and the captured hashes: `isolator.capture(step, &trees)` → `record_commits(step, &after)`; `finish_step(step, StepOutcome { exit_code: None, usage, trim_record, verify_outcome: None, verify_exit_code: None, finished_at: now })` (`run.rs:409-422`, `traits.rs:784`; never `status`).

**Stage 6 — gate.** `gate::apply(...)` (§5.5).

### 5.4 `gate.rs` — settle outcome and verdict

```rust
/// ANA-2 §4.2's three-valued settle (`:436-441`), `ok` per plan D2 (`verify_outcome != 'fail'`, NULL ok).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle { Ok, Failed(StepFailure), Rejected { verdict_line: String } }

pub struct SettleInput<'a> {
    pub driver: &'a Result<DoneEvent, DriverError>,
    pub cap_breach: Option<CapBreach>,
    pub started_at: DateTime<Utc>, pub now: DateTime<Utc>, pub deadline_seconds: Option<u32>,
    pub output: Option<&'a Document>,          // produced by this step, of `output_kind`
    pub verify_outcome: Option<VerifyOutcome>,  // always None this milestone
    pub is_review: bool,
}
#[must_use] pub fn settle(input: &SettleInput<'_>) -> Settle;
```

Order (first hit wins): `Err(_)` → `Failed`; `Done { stop_reason: Refusal | MaxTokens | MaxTurnRequests }` → `Failed` (`event.rs:96-108`; `Cancelled` is not a settle outcome — a cancelled step is `cancelled`, ANA-2 `:441`); `cap_breach.is_some()` → `Failed`; `now > started_at + deadline` → `Failed`; `output.is_none()` → `Failed(MissingOutput)`; `verify_outcome == Some(Fail)` → `Failed`; `is_review && parse_verdict(&output.body) == RequestChanges` → `Rejected`; else `Ok`. Failure reasons other than `MissingOutput` are step facts carried on the step's `gate_note` (`"stop_reason: refusal"`, `"driver: <Display>"`, `"deadline elapsed"`, `"cap breached"`), since D12's enum is `run.failure`'s vocabulary.

```rust
/// ANA-5's three-line front matter (`ANA-5.md:1322-1336`), plan D10: only the exact value rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict { Approve, RequestChanges, Other(String), Absent }
#[must_use] pub fn parse_verdict(body: &str) -> Verdict;
```

Lines 1 and 3 must be exactly `---`; line 2 `verdict:` + value, trimmed, lowercased; `Other(v)` → recorded on `gate_note` as `verdict: <v> (unrecognised)`; `Absent` → nothing recorded. Tests: the four arms, CRLF input, trailing-space `--- ` is `Absent`, `VERDICT: Approve` is `Approve`.

### 5.5 `gate.rs` — the gate table and its writes

```rust
pub async fn apply<S: WriteStore, C: Clock>(store: &S, clock: &C, run: &Run, snapshot: &GraphSnapshot,
    step: &RunStep, phase: &SnapshotPhase, settle: Settle) -> Result<Rest, EngineError>;
```

| `gate_effective` | `Ok` | `Failed` | `Rejected` |
|---|---|---|---|
| `Always` | park | park | park |
| `OnFailure` | done + `gate_outcome = skipped` | park | park |
| `Never` | done + skipped | §4.2 retry: `may_attempt(attempt + 1, retry_limit)` → step `running → failed`, `create_step(attempt + 1)`, continue; else fail run | `review_loop` (automatic entry, D4) |

- **park**: `transition_step(step, Running, AwaitingApproval, now)`; `transition_run(run, Running, AwaitingApproval, now)` (`:758-764`); `transition(item, InProgress, AwaitingApproval)` (`:210`) — three writes, F-K. `Rest { AwaitingApproval, Some(position), None }`.
- **done + skipped**: `answer_gate` is `awaiting_approval`-only (`:786-791`), so a `never`/`on_failure` pass is `transition_step(step, Running, Done, now)`; `gate_outcome = skipped` has no writer on a `running` step — hazard H-9; the step's `gate_outcome` stays NULL this milestone and the gap is carried with R-5.
- **fail run** (non-loopable failure): `transition_step(Running → Failed)`, `finish_run(run, Failed, Some(&failure.to_string()), now)`.

After `AnswerGate` (§5.2): `Approved`/`Skipped` → step is `done`; `transition_run(AwaitingApproval → Running)`, `transition(item, AwaitingApproval → InProgress)`, continue the walk. `Rejected { note }` → step is `failed` with `gate_outcome = rejected`, `gate_note = note`; `transition_run(AwaitingApproval → Running)` and `transition(item, AwaitingApproval → InProgress)` **first** (F-J); then if `review_loop` finds a target: `review_loop(..)`; else `finish_run(run, Failed, Some("rejected: <phase>"), now)`.

### 5.6 `gate.rs` — the review loop (D4, D5, D11)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOutcome {
    /// Inserted `(p_impl, a + 1, 0)` pending; the walk resumes there.
    Resumed { position: i32, attempt: i32 },
    /// Parked: run `awaiting_approval`, item `blocked`, note written.
    Escalated { attempts: i32, reason: StopReason },
    /// No `p_impl`: run failed with `no_loop_target`.
    NoTarget,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason { Exhausted, NoProgressHash, NoProgressReview }

/// ANA-2 §4.4 `:712-722`, one routine for both entry points (plan D4).
pub async fn review_loop<S: WriteStore, C: Clock>(store: &S, clock: &C, run: &Run, snapshot: &GraphSnapshot,
    review: &RunStep, verdict_line: &str, user: UserId, box_id: BoxId) -> Result<LoopOutcome, EngineError>;
```

1. `p_impl`: greatest `position < review.position` with `phases[p].name == "implement"`, else `review.position - 1`; `review.position == 0` → `finish_run(Failed, "no_loop_target")` → `NoTarget`.
2. `steps = run_steps(run)`; `a = latest_at(steps, p_impl).attempt`; `next = a + 1`.
3. No-progress (D11), evaluated only when `a >= 2`: the implement steps at attempts `a` and `a - 1`, `step_commits` (`:161`) for each → per `repo_id`, `after_hash` equal (both `None` equal) for every repo in scope (empty scope → the review-body test decides alone); or the two latest review documents with `sha256_hex(&canonical(&body))` equal (`digest.rs:36`, `:73`). Either → `Escalated`.
4. Admission: `may_attempt(next, phases[p_impl].retry_limit)` else `Escalated { Exhausted }`.
5. Retire, for each position `p_impl..=review.position`, the latest-attempt step: `Pending | AwaitingApproval | Done` → `supersede_step` (`:838`); `Running` → `transition_step(Running, Cancelled, now)` then leave (``running → superseded`` is illegal, `run.rs:115`); `Failed` → leave (the review itself, keeping `gate_outcome = rejected`); `Superseded | Cancelled` → leave.
6. `create_step(NewRunStep { position: p_impl, attempt: next, fanout_index: 0, … })`. Intermediate positions are created lazily by `cursor` at `next_attempt(p)` (§3.3).
7. `Resumed`.

**Both entry points converge.** The automatic path (gate `never`) parks first — `transition_step(Running → AwaitingApproval)` then `answer_gate(step, Rejected, Some(verdict_line), now)` (→ `failed`) — because `answer_gate` needs `awaiting_approval`. That is two writes and it makes the human and automatic paths reach `review_loop` with identical rows. Documented on `review_loop`.

Escalation writes (ANA-2 `:741-753`, F-I): `transition_run(Running → AwaitingApproval)`; `transition(item, InProgress → Blocked)` (`item.rs:52`); `add_note(NewNote { item_id, body: format!("review loop exhausted after {n} attempts: phase `{}`, attempt {n}, stop reason `{reason}`", phases[p_impl].name), created_by: user, box_id: Some(box_id), via_step_id: Some(review.id), created_at: now })` where `n = a`. `run.failure` stays NULL (A-4). `Rest { AwaitingApproval, Some(review.position), Some(ReviewLoopExhausted(n)) }`.

---

## 6. T5 — fixtures, the six criteria, C-4

### 6.1 `tests/fixtures/`

Recorded graphs as JSON `GraphSnapshot` documents (ANA-2 `:1690`), loaded with `serde_json::from_str::<GraphSnapshot>` and asserted equal to a fresh `graph::resolve` of the same fixture item — the fixture is the pin, the resolver is under test:

- `feature.snapshot.json`: `ids::GRAPH_HTUI_FEAT` (`fixtures.rs:183`) resolved for `ids::HTUI_FEAT_3` (`:222`, `open`), manual, candidate `(AGENT_CLAUDE, "claude", "sonnet")`, `topology` = the §3.4 literal.
- `feature-with-verify.snapshot.json`: same with an intermediate `verify` position between implement and review (plan D5's risk row: "a fixture graph with an intermediate position"), built by `create_phase` in the case before resolving.
- `expected/feature-walk.steps.json` and `expected/review-loop.steps.json`: the `(position, attempt, fanout_index, phase_name, status)` sequences criteria 1 and 6 assert.

### 6.2 `tests/fake_conformance.rs`

`struct Demo; impl CaseHarness for Demo { type Orch = FakeOrchestrator; fn fresh(&self) -> FakeOrchestrator { FakeOrchestrator::demo() } }`; `cases_len_is_eleven`; `case_names_are_unique`; `fake_orchestrator_passes_every_case` → `run_all(&Demo)`.

### 6.3 The cases (bodies in `src/conformance.rs`)

- **`feat_walks_end_to_end`** (criterion 1): `StartRun { HTUI_FEAT_3, Manual, None }` → parks at position 0. Assert: run `awaiting_approval`, item `awaiting_approval`, one step `(0,1,0,"prd",awaiting_approval)`, one `prd` document with `produced_by_step_id`, `prompt_digest == prompt.digest` (H-13). Three `AnswerGate { Approved }` → positions 1, 2 each park. Fourth approve → `Rest { Done, None }`; four steps `done`, four documents whose kinds are `prd|plan|implement|review`, item `done`, `closed_at` set, `run.finished_at == clock.now()`. **First failing test in T5.**
- **`live_run_ignores_a_gate_edit`** (criterion 2): start; `update_phase(review, expected, PhasePatch { gate: Some(Gate::Never), .. })` (`traits.rs:586`); approve through to review → still parks (snapshot `always`). A second run started after the edit reaches `never` and does not park.
- **`topology_mismatch_parks_on_resume`** (criterion 3): start and park at position 1; `create_phase` adds a position to the live graph; `Engine::resume(run)` (re-resolve, compare `topology`, on mismatch record a note and refuse to advance) → the walk is not advanced. `resume` is milestone 5's sweep entry, cut here as the one function criterion 3 needs.
- **`approve_needs_the_output_document`** (criterion 5): `script("prd", 1, done_without_output())` → settle `Failed(MissingOutput)` → `always` parks. `AnswerGate { Approved }` → `Err(MissingOutputForApproval { kind: "prd" })`, step still `awaiting_approval`, no new rows.
- **`review_rejection_loops_then_escalates`** (criterion 6): the case first `create_repo`s a primary and passes `repo_scope: None` → `[repo]` (also exercising D14's happy path). Scripts: `implement` 1 and 2 `done_with_output`, `review` 1 and 2 `review("request-changes", …)` with different bodies; `script_after(Some("h1"))` then `Some("h2")`. Approve prd, plan, implement; at review `AnswerGate { Rejected { note: "no tests" } }` → `Resumed`: `(2,1) superseded`, `(3,1) failed/rejected/gate_note "no tests"`, `(2,2) pending`; `resolve_inputs(item, run, ["plan","review"])` for the new implement step contains the review document. Approve `(2,2)`; `(3,2)` parks; reject again → `Escalated { attempts: 2, Exhausted }` (`retry_limit = 1`): run `awaiting_approval`, item `blocked`, note contains `"review loop exhausted after 2 attempts"`, `"implement"`, `"attempt 2"`.
- **`identical_after_hash_stops_the_loop`** (criterion 7): as above with `retry_limit` raised to 3 and `script_after(Some("same"))` twice → after the second rejection `Escalated { attempts: 2, NoProgressHash }` while `may_attempt(3, 3)` would have permitted more; note contains `"no_progress_hash"`.
- **`missing_input_fails_before_a_token`**: `update_phase(prd, input_kinds: ["spec"])` — a kind nothing produces and no later position outputs, so the back-edge rule does not excuse it → run `failed`, `failure == "missing input document: spec"`, item `failed`, `prompt_digest.is_none()` on the step.
- **`missing_output_settles_failed`**: `gate: Never` on prd, `done_without_output` → retry once (`retry_limit 1`), second also without → run `failed`, `failure == "missing_output"`, `(0,1) failed`, `(0,2) failed`.
- **`cli_agent_is_refused_at_a_gated_phase`**: `with_candidates("prd", [(AGENT_CLAUDE_CLI, "default")])` → Cli caps have `permission_requests: false, edit_proposals: false` (`registry.rs:178-181`) → run `failed` with `"missing_capability: inline_approval"`, item `blocked`, note present.
- **`step_deadline_settles_failed`**: `step_deadline_seconds = 1` through `update_project`; `FakeOrchestrator::advance_after_done(2s)` → settle `Failed`, park under `always`, `gate_note` contains `"deadline elapsed"`.
- **`finish_run_is_the_last_write`**: criterion 1's tail — `run.finished_at` and the item's `closed_at` both stamped at the last approve's instant, item `done`, no extra `transition_run` needed.

### 6.4 C-4 marker (`crates/htui-core/src/model/run.rs:742-743`)

Replace the comment at `:742` with:

```rust
        // `failed` — terminal except for the promotion row, and cancellable with its run
        // (derived from row 652 + row 654's exception: ANA-2 §4.3's step table sanctions
        // `failed -> awaiting_approval` only through §4.8's promotion, blueprint C-4).
```

No code change; the `(StepStatus::Failed, StepStatus::AwaitingApproval)` row at `:743` stays.

---

## 7. Data flow — one FEAT walk, manual mode, `HTUI_FEAT_3`

Statuses after each store call; `I` item, `R` run, `S` step `(pos,att)`. Clock `t0 = epoch()`, each call `t+1µs` truncated.

| # | Call | I | R | S |
|---|---|---|---|---|
| 1 | `graph::resolve` (reads only) | open | — | — |
| 2 | `create_run(NewRun{ graph_snapshot, repo_scope: [] })` | queued | queued | — |
| 3 | `claim_run(run, BOX, owner, t, t+120s)` → `true` | in_progress | running | — |
| 4 | `run_steps` → `cursor` = `Create{0,1}`; `agent(AGENT_CLAUDE)` + `caps_for` → Acp, eligible | | | |
| 5 | `create_step((0,1,0,"prd"))` | in_progress | running | (0,1) pending |
| 6 | `transition_step(Pending→Running)` | | | (0,1) running |
| 7 | `isolator.prepare` → no repos → `trees = []`; `upsert_step_tree(step, [])`; `record_commits(step, [])` | | | |
| 8 | `resolve_inputs(item, run, [])` → `[]`; `prompt_template(HTUI, "prd", Some(1))`; `assemble`; `set_step_prompt` | | | prompt_digest set |
| 9 | `Recorder::record_prompt`; `driver.start`; `pump` → `Done{EndTurn}`; `recorder.finish` | | | usage set |
| 10 | `sink.after_done` → `write_document(NewDocument{ kind: "prd", produced_by_step_id })` | | | |
| 11 | `settle` → `Ok`; `isolator.capture` → `[]`; `finish_step(step, StepOutcome{ finished_at: t })` | | | finished_at |
| 12 | gate `always` → `transition_step(Running→AwaitingApproval)`; `transition_run(Running→AwaitingApproval)`; `transition(item, InProgress→AwaitingApproval)` | awaiting_approval | awaiting_approval | (0,1) awaiting_approval |
| 13 | `AnswerGate{Approved}`: document check; `answer_gate(step, Approved, None, t)` | | | (0,1) done, gate_outcome approved |
| 14 | `transition_run(AwaitingApproval→Running)`; `transition(item, AwaitingApproval→InProgress)` | in_progress | running | |
| 15–24 | positions 1 (`plan`, inputs `[prd]`) and 2 (`implement`, inputs `[plan, review]`, `review` optional per §5.3) repeat 4–14 | … | … | (1,1) done, (2,1) done |
| 25–33 | position 3 (`review`, inputs `[implement]`), body `---\nverdict: approve\n---\n…` → `Approve` → settle `Ok` → park | awaiting_approval | awaiting_approval | (3,1) awaiting_approval |
| 34 | `AnswerGate{Approved}` → done; run `→ Running`, item `→ InProgress` | in_progress | running | (3,1) done |
| 35 | `cursor` → `Finished`; `finish_run(run, Done, None, t)` | **done** (`closed_at = t`) | **done** (`finished_at = t`) | four done |

Rejection branch at 34 (`Rejected { note }`): `answer_gate` → (3,1) failed/rejected; run `→ Running`, item `→ InProgress`; `review_loop`: `p_impl = 2`, `a = 1`, no-progress skipped (`a < 2`), `may_attempt(2, 1)` true; `supersede_step((2,1))`; (3,1) left `failed`; `create_step((2,2,0))`; `Resumed{2,2}`; the walk runs (2,2) with `resolve_inputs` now resolving `review` to the rejecting document (preferring this run's output, `traits.rs:167-170`), then creates (3,2) at `next_attempt(3) = 2`.

---

## 8. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | **Illegal transitions are `Constraint`, not `Ok(false)`.** `running → superseded` (`run.rs:115`), `awaiting_approval → blocked` and `blocked → in_progress` (`item.rs:46-60`) all refuse. | D5's status split; A-5's item order; every CAS the engine issues has its pair listed in §5. |
| H-2 | **`preserve_order` digest trap** (D9). `serde_json::to_value(&snapshot)` then `to_string` gives a different byte string under `cargo test --workspace` than under `cargo test -p htui-orch`. | `topology()` serialises the typed slice; the test vector runs in both invocations. Clippy cannot catch it; a grep for `to_value` in `graph.rs` at review can. |
| H-3 | **`MemStore::phase_agents` is unconditionally empty** (`mem.rs:469-472`) and rung 3 is empty on the demo fixture (all three agents enabled, `agent.rs:156`). | `FakeGraphSource` candidates map (F-F); `NoCandidate` is a named refusal, never a silent empty walk. |
| H-4 | **`UNIQUE (run_id, position, attempt, fanout_index)`** — `create_step` refuses with `step_slot_is_taken` (`traits.rs:1074`). | `next_attempt` reads `max + 1` on every call (D16); `cursor` never proposes an attempt that exists. The `feature-with-verify` fixture exercises the intermediate position. |
| H-5 | **`UsageSpy` (`htui-agent/src/conformance.rs:570`) and `SpyStore` (`htui-agent/tests/recorder.rs`) hand-forward `WriteStore`**; so do `BufferedWriter` and `Writer`. A missing `finish_run` arm breaks `-p htui-agent --all-features` or `-p htui-store`. | T1's file set names all four; T1's gate runs both crates. |
| H-6 | **`traits.rs:670-675` banner** says five transactions; `promote_step` was already a sixth. | F-D text: seven, named. |
| H-7 | **Count-pin ledger**: `mem_store.rs:36-42` and `pg_conformance.rs:19` both say 47; `every_cross_referenced_test_name_exists` (`conformance.rs:6379`) fails when a doc comment names a `pg_criteria` fn that does not exist and again when a `PENDING` entry becomes defined. | 48 in both; `PENDING` entry added in T1's first commit and removed in its last. |
| H-8 | **Seeded `implement.input_kinds = ["plan","review"]`** (`seed.rs:87`) versus ANA-2 `:412-414`'s hard failure on a missing input: a literal reading fails criterion 1 at position 2. | §5.3's back-edge rule, documented on `required_inputs`; `missing_input_fails_before_a_token` pins the hard path with a kind nothing produces. |
| H-9 | **No writer sets `gate_outcome = skipped` on a `running` step**: `answer_gate` is `awaiting_approval`-only (`traits.rs:786-791`). A `never`/`on_failure` pass leaves `gate_outcome` NULL. | Documented on `gate::apply`; carried with R-5. No milestone-2 criterion asserts `skipped`. |
| H-10 | **Three-write park is not one transaction** on Postgres (F-K). | Order step → run → item so the forbidden direction (a run waiting with no waiting step) cannot occur; R-5 for the composite. |
| H-11 | **`ReadStore` has no per-step read and no `box_profile`/`app_settings`/`bound_skills`** (all inherent, `mem.rs:353`, `:382`, `:413`). | Commands carry `run` (§4.4); `EngineParts` carries `app` and `box_profile`; skills empty this milestone. |
| H-12 | **`FakeDriver` plays one script once** (`fake.rs:158-183`) and refuses an empty prompt. | One driver per session via the factory closure. |
| H-13 | **`Recorder::record_prompt` also writes `prompt_digest`** (`record.rs:572-618`) after `set_step_prompt` wrote it at stage 3. Same digest by construction; a test that re-hashes a scrubbed variant will disagree. | Assert `prompt_digest == prompt.digest`; the scrubber is `MinimalScrubber::new([])` on both sides. |
| H-14 | **`StepGraphPhase` has no `deadline_seconds`/`judge_*` fields** (`kind.rs:177-212`) though `0003` added the columns. | §3.4's chains start at the project rung and say so; no model change here (it would touch `.sqlx`). |
| H-15 | **`claim_run` returns `Ok(false)` for a full box** (`max_concurrent_items`, default 2, `box_.rs:121`). | Fresh `MemStore::demo()` per case; `ClaimRefused` is named. |
| H-16 | **`Status::Blocked` before `finish_run(Failed)`** (stage 1): reversed order gives item `failed`, not `blocked`. | Order fixed in §5.3; `cli_agent_is_refused_at_a_gated_phase` asserts `blocked`. |
| H-17 | **The topology literal is pasted once** and depends on field order, `agent_name`, template versions and the `7200` default. Any of those moving is a legitimate new digest **and** a `GraphSnapshot::V` question (D9). | The test's failure message says so verbatim. |
| H-18 | **`cargo doc`** exits 101 at HEAD (six `htui-store` errors, CLEAN-3), so a new `htui-orch` doc error is invisible in the exit code. | T5's gate greps the doc output for `htui-orch`/`htui_orch`; zero allowed. |

---

## 9. File sets

**T1** — gate per §1.
- `crates/htui-core/src/store/traits.rs` (method after `:885`; banner `:672-673`; two refusal fns after `:1125`)
- `crates/htui-core/src/store/mem.rs` (`State::finish_run` after `:3741`; `impl WriteStore` arm after `:4390`; comment above `:4273`)
- `crates/htui-core/src/store/conformance.rs` (case; `CASES`; `run_case` arm before `:162`; `PENDING` `:6396`)
- `crates/htui-core/tests/mem_store.rs` (`:36-42`)
- `crates/htui-store/src/pg/write.rs` (after `:3448`) + `crates/htui-store/.sqlx/` (regenerated)
- `crates/htui-store/src/writer.rs` (after `:740`; after `:1576`)
- `crates/htui-store/tests/writer_buffered.rs` (`:662`; after `:897`)
- `crates/htui-store/tests/pg_conformance.rs` (`:19`)
- `crates/htui-store/tests/pg_criteria.rs` (after `:3873`)
- `crates/htui-agent/src/conformance.rs` (after `:1018`)
- `crates/htui-agent/tests/recorder.rs` (after `:711`)

**T2** — `cargo test -p htui-orch --all-features && cargo clippy -p htui-orch --all-targets --all-features -- -D warnings`
- `Cargo.toml` (`:2`, after `:25`)
- `crates/htui-orch/Cargo.toml`, `src/lib.rs`, `src/status.rs`, `src/graph.rs` (create)
- `crates/htui-orch/src/{command,isolate,engine,gate,fake,conformance}.rs` (create as documented stubs so `lib.rs` compiles)

**T3** — as T2: `crates/htui-orch/src/{isolate,fake,command,conformance}.rs`

**T4** — as T2: `crates/htui-orch/src/{engine,gate}.rs`

**T5** — as T2, then the full workspace gate and `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.
- `crates/htui-orch/tests/fixtures/*.json`, `tests/fake_conformance.rs`, `src/conformance.rs` (case bodies)
- `crates/htui-core/src/model/run.rs` (`:742` comment only)

**Not touched, on purpose**: no migration and no column (PRD `:272`); `crates/htui/**` (milestone 6); `fanout.rs`, `select.rs`, `verify.rs`, `overlap.rs`, `recover.rs`, `queue.rs` (not created, D1); `claim_run`'s predicate (R-2, D6); `transition_run`/`fail_run` bodies (D7); `crates/htui-store/Cargo.toml` (D19's invariant-10 check: zero diff); every `.snap`; `crates/htui-core/src/model/kind.rs` (H-14 recorded, not fixed).

**Carried out of this milestone**: R-3 (`run.failure` on a parked run, F-I), R-4 (resume after escalation needs `blocked → in_progress` or `unblock`, F-J), R-5 (composite park writer and `gate_outcome = skipped`, F-K/H-9), R-6 (`phase_agent` copy and `is_override` writers, F-L), C-5 (unchanged: needs a fixture repo).
