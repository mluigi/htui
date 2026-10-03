# Blueprint: MOD-72, tool-call chips in the Runs flow view

**Status**: **proposed** (2026-10-03). Plan deviations B-1 to B-11 (§0) and blueprint decisions
E1–E17 (§6) belong to this blueprint. A deviation marked **Blocker** means the plan, read literally,
produces a defect its own tests would show. The Fix column is what the implementer builds. The
plan's D1–D8 are binding and are not reopened. Where a fix narrows one of them, the row cites the
evidence.

**Plan**: `.claude/plans/mod-72-tool-call-chips.plan.md` at `d3bcaad4`, **confirmed** 2026-10-03,
fact-checked (step 3.5). Design source: `docs/ANA-12.md` §3.2. Tasks are cited as "MOD-72 T*n*"
outside this file.

**Verified at**: HEAD `d3bcaad4`, branch `hr/MOD-72`, clean tree, sandbox (`HR_SANDBOX=1`). Anchors
were read through Gortex (`search`, `read`), with shell reads where a hook allowed one. **Line
numbers are pre-edit**: once a task commits to a file, a citation into that file moves. Two probes
ran this blueprint's exact SQL on fixture-shaped rows (the §2.5 case's rows): SQLite 3.50.4
(in-memory, Python) and the sandbox Postgres (`psql -h localhost -p 5439`, `TEMP` tables). Both
returned `(s1 execute 2) (s1 other 1) (s1 read 1) (s2 edit 1) (s2 other 2)` for the main item and
`(s3 read 1)` for the second item. Both showed the bare-`COALESCE` defect (B-2): SQLite returns the
integer `7`, and Postgres returns the text `"7"`. Counted at this HEAD: 134 tracked
`crates/htui/tests/snapshots`, 318 `crates/htui-store/.sqlx` files. `htui_sqlx` already exists on
`:5439`. `df -h` shows 2.8 T free (81 %).

**Graphify**: `graphify-out/` isn't in this checkout, so nothing here comes from it.

**Coupling verdict.** The plan's order stands: **serial T1 → T2 → T3 → T4, one implementer, no
fan-out**. T2 needs T1's trait method and model type. T3 needs T2's variants. No file joins a task
beyond the plan's list. The `runs.rs` test edits (B-1) are in a file T3 already owns.

**Scope**:
- **No migration, no new dependency, no new crate.** One `.sqlx` entry (Postgres `query!`).
- **New code**: `ToolCallCount` (model), `ReadStore::tool_call_counts` on seven implementors,
  `StoreRequest::ToolCalls` / `StoreReply::ToolCalls`, and in `execution_graph.rs` `chips`,
  `short_label`, `by_step` and the node's third line.
- **Pins that move** (T4 re-counts, and doesn't add to the HANDOFF numbers): store `CASES` 130 →
  **131**, `READ_CASES` 14 → **15**, `StoreRequest` 96 → **97**, `StoreReply` 55 → **56**, `.sqlx`
  318 → **319**, snapshots 134 → **135** (two move, one new). `htui-orch` `CASES` is unmoved at 92.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_debug_implementations` and `unused_qualifications` warn, and
  `clippy::all` warns. **`clippy::pedantic` is not enabled** (`Cargo.toml` `[workspace.lints]`), so
  `cast_possible_truncation` would not catch an `i64 as u32`. Use `u32::try_from` anyway, because
  `as` truncates silently (E4). The gate is `cargo clippy --workspace --all-targets --all-features
  -- -D warnings`. rustdoc denies broken and private intra-doc links.
- `SQLX_OFFLINE = "true"` (`.cargo/config.toml` `[env]`): a `query!` with no `.sqlx` entry does not
  compile. The entry lands in the **same commit** as the macro (H-2).
- Implementers commit after each step and stage only their own paths: never `-A`, never `stash`,
  never `--amend`. Every commit compiles. A red commit may put a `todo!()` body **only** in an item
  that no product path calls (H-3).
- Integration tests need `--all-features` (or `--features testkit`). Without it, `tests/*.rs` runs
  0 tests and still reports ok. Every `htui` gate uses `--test-threads=1`.
- UI code never panics in a render path and never logs.

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `d3bcaad4` | Fix |
|---|---|---|---|---|
| **B-1** | **Blocker** (three existing unit tests go red) | D4: "`v` into the flow view asks once". T3 lists only "`runs.rs:3313-3345` request-list asserts" as tests that move. | `runs.rs:3313-3345` run in the **list** view, so they're unchanged (B-9). But three MOD-28 flow tests pin that `v` sends nothing: `action_keys_in_flow_send_what_the_list_sends` asserts `shell.emit.take().is_empty()`, "`v` sends nothing" (`:3664`). `the_flow_view_draws_the_pending_request_a_digit_answers` asserts the exact list `[AnswerPermission]` after `v`, `2` (`:3825-3830`). `tab_and_a_digit_pass_in_flow_with_no_pending_request` asserts `shell.emit.is_empty()` after `v`, Tab, BackTab, `3` (`:3847`). With D4, `v` emits `Action::Store(ToolCalls { item })`. | §4.6: `:3664` becomes `assert!(matches!(requests(shell.emit.take()).as_slice(), [StoreRequest::ToolCalls { item }] if *item == ids::HTUI_FEAT_1), "`v` asks only for the chips")`. In the other two, insert `let _ = shell.emit.take();` right after the `v` key (`:3808`, `:3839`). The assertions after it stay as written. |
| **B-2** | Non-blocker (latent; no writer produces it today) | D3: "`COALESCE(payload->>'tool_kind', 'other')`" in SQL; "the mapper's own fallback" in Rust. | Rust's precedent is `StepSummary::from_events` (`htui-core/src/prompt/render.rs:822-832`): `.get("tool_kind").and_then(Value::as_str).unwrap_or("other")`, so a non-string kind is `other`. Probes: Postgres `'{"tool_kind":7}'::jsonb->>'tool_kind'` is `"7"`, so the backends disagree. SQLite `json_extract` gives the **integer** `7`, which `get::<String>` refuses, so the whole read fails. The mirror already guards with `json_type` for this exact reason (`cache/read.rs:550-559`, "T68, F-52"), and Postgres guards with `jsonb_typeof` (`pg/read.rs:366`, `:381-386`). | E3: the kind is **type-guarded** on every backend. Postgres uses `CASE WHEN jsonb_typeof(e.payload->'tool_kind') = 'string' THEN e.payload->>'tool_kind' ELSE 'other' END`. SQLite uses `CASE WHEN json_type(e.payload, '$.tool_kind') = 'text' THEN json_extract(…) ELSE 'other' END`. Mem uses `as_str().unwrap_or("other")`. This narrows D3's `COALESCE` and keeps D3's intent (missing → `other`). The `CASES` entry pins `null` and `7` (§2.5). |
| **B-3** | Non-blocker (scope) | Verified claims: "`ReadStore` implementors are … seven". | Correct for `ReadStore`. A **second** trait also re-declares `step_events`: `WorkerStore` (`htui-core/src/store/worker.rs:97-145`, "13 ReadStore reads"), implemented by `MemStore` (`worker.rs` `impl WorkerStore for MemStore`) and by `PgStore`/`Writer` in `htui-store/src/worker.rs:89`, `:380`. Its doc says it holds "exactly its call sites" (the engine's). | E6: **no** `WorkerStore` or `WorkerHost` method. The engine never reads counts. Adding one would break the "exactly its call sites" rule and the module doc's counts. |
| **B-4** | Non-blocker (the case would fail at setup) | T1 `CASES` entry: "two steps × three kinds, a second item's run that must not leak in". | `create_run` first checks `legal_move(item.status, Queued)` (`mem.rs:4296-4300`, the same in Postgres). `FEAT-1` is `in_progress`, and its legality isn't pinned by any case. The conformance precedents create runs on `HTUI_ANA_2` (open, 11 uses) and `HTUI_CLEAN_1` (failed, 5 uses). A fresh run holds no lease, so `StepFence::Unleased` is the fence (`traits.rs` `StepFence`; `append_events_idempotent_and_ordered` writes `Unleased`). | E7: the main run goes on `HTUI_ANA_2`, and the leak run on `HTUI_CLEAN_1` (same project, `PROJECT_HTUI`). Append **per step** (three `append_events` calls), not one cross-run batch. |
| **B-5** | Non-blocker (D6 fidelity at zoom ≠ 1) | D6: "fitted greedily into the 18-cell interior", with `StepNode` holding "precomputed strings". | `render` clips every line to `inner.width` (`execution_graph.rs:237-249`). At `-` (zoom 0.83) the interior is about 15 cells, so a line fitted for 18 is cut mid-chip with `…`, and its `+N` is lost. At `+` (zoom 1.2–2.0) chips that would fit stay folded into `+N`. | E10: `StepNode` keeps its step's rows, and `render` calls `chips(&self.calls, usize::from(inner.width))`. At zoom 1, `inner.width == 18`, which is D6 verbatim. |
| **B-6** | Non-blocker (coverage) | D5: "height-only literals updated". | `text_is_clipped_by_display_width` checks the right `│` on `y + 1..y + 3` (`execution_graph.rs:1011`), which is the two interior rows of a 4-high node. | Change it to `y + 1..y + 4`, which covers all three interior rows, the chip row included. |
| **B-7** | Non-blocker (compile) | Files table lists `mem.rs`. | `mem.rs` doesn't import `EventKind` (`mem.rs:24-47`, no `EventKind::` anywhere in the file). | Add `EventKind` and `ToolCallCount` to `mem.rs`'s `crate::model::{…}` list. Every other impl file adds `ToolCallCount` to its own list (§2.3 table). |
| **B-8** | Non-blocker (bookkeeping) | Risks/Files: `EXPECTED_CASES` 130 → 131 and the `CASES.len()`/`READ_CASES.len()` pins. | HANDOFF also pins `READ_CASES` 14, `StoreRequest` 96, `StoreReply` 55, 318 `.sqlx` and 134 snapshots (`HANDOFF.md:45-48`). HANDOFF's MOD-N row lists "MOD-72 tool-call chips" as open (`:686`). | T4 re-counts all six (Scope above) and drops MOD-72 from `:686`. |
| **B-9** | Non-blocker (confirmation) | T3: "`runs.rs:3313-3345` request-list asserts". | `the_first_runs_reply_subscribes_once_and_asks_for_the_actions` (`:3307-3350`) never presses `v`, so it runs in the list view, which D4 leaves unchanged. | Unchanged. A flow-view counterpart is new (`a_runs_reply_in_flow_asks_for_the_tool_calls_last`, §4.6). |
| **B-10** | Non-blocker (the gate as written errors) | T3 Validate: `cargo test -p htui --all-features execution_graph runs`. | `cargo test` takes one `TESTNAME` positional, so a second one is an argument error. There's also no `--test-threads=1`. | Use `cargo test -p htui --all-features --lib detail::runs -- --test-threads=1`. `execution_graph` is `…::detail::runs::execution_graph`, so one filter covers both modules. |
| **B-11** | Non-blocker (confirmation) | "two existing flow snapshots move by one node row". | `backlog__runs_flow_fanout.snap` is `RUN_3`, whose steps have no events (only `STEP_PLAN` does, `fixtures.rs:1720-1830`). `backlog__runs_flow_reject_note.snap` is `parked()`, whose script is `one_turn()`: only `Done` (`tests/backlog.rs:571-575`). | Neither snapshot gains a chip. Each node gains one blank interior row, and nothing else moves (§4.8). |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | `NODE_H` 4 → 5 moves every y-literal derived from `Y_STEP` (7 → 8). | All of them are listed: `execution_graph.rs:616-617` (`7.0`, `14.0` → `8.0`, `16.0`), `:636` (`(10.0, 7.0)` → `(10.0, 8.0)`), `:825-827` (`(21.0, 7.0)`, `(10.0, 14.0)`, `(31.0, 14.0)` → `8.0`, `16.0`, `16.0`). `node_ids_are_step_ids_and_no_two_nodes_overlap` uses `NODE_H` by name. The x-literals, widths and the row-1 corners (`a_new_run_is_centred_at_zoom_one`, `:1058-1075`) don't move. The zoom/fit tests still hold: `linear(10)` is 77 world rows (was 67), so at the 0.5 floor it's still taller than 23 rows (`fit_keeps_the_cursor_node_on_screen`). 8 steps is 61 rows, so `fit` still zooms below 1 (`zoom_is_clamped_and_fit_zooms_out_to_a_tall_run`). |
| **H-2** | `SQLX_OFFLINE=true`: a `query!` without its `.sqlx` file fails the build. The entry hashes the **literal** query text, and regenerating without `--all-targets --all-features` deletes the test-only entries. | §2.6 recipe, verbatim from `docs/hr-sandbox.md:196-210`. The macro and the entry land in one commit. Never touch the SQL text after `prepare` without re-running it. Gate: `cargo sqlx prepare --check` and exactly one new `.sqlx/query-*.json` (`git status --porcelain crates/htui-store/.sqlx` shows one `??`). |
| **H-3** | A red commit routes a live path to a `todo!()`. | T1's `todo!()` bodies (Mem/Pg/Cache) are reached only by the new cases until T2 serves the request. T2's red `serve` arm is reached only by its test until T3 asks. T3's red `chips`/`short_label`/`by_step` aren't wired until the green commit. |
| **H-4** | `every_cross_referenced_test_name_exists` (`conformance.rs:15899-16004`) panics on a back-ticked snake_case token with ≥4 `_` in a doc comment of `conformance.rs`/`mem.rs` that neither file defines as a fn. | New doc comments in those two files name only `tool_call_counts` (2 `_`) or the new cases themselves, which `conformance.rs` defines. Never name `cache.rs`/`pg_conformance.rs` tests there. |
| **H-5** | `i64` count → `u32`. | `u32::try_from(n).unwrap_or(u32::MAX)` saturates and is never an `as` (E4). Mem counts with `saturating_add(1)`. |
| **H-6** | Two `ToolCalls` in flight (`v`, then a `Runs` reply in flow). | The per-`(origin, discriminant)` staleness gate (`app/state.rs:308-334`) delivers only the newest. Both ask the same thing, so dropping one is harmless. |
| **H-7** | `RefCell` double borrow. | The reply arm and `v` run under `&mut self`, and `sync_graph` uses `get_mut` (`runs.rs:377-387`). `render_flow` holds the only runtime `borrow_mut`. |
| **H-8** | Offline, the mirror holds the last-N steps' events only. | Accepted by D2: a step outside the window draws no chips, the same as "no calls". `the_mirror_passes_the_read_cases` (`htui-store/tests/cache.rs:1055`) refreshes with a window of 20, which holds `STEP_PLAN`. |
| **H-9** | Sibling branches bump the same pins (`mem_store.rs:36`, `:63`; `pg_conformance.rs:26`). | On collect, the merged value is the sum of the bumps. The close-out names it (plan Risks). |
| **H-10** | Test stack (memory: `htui-orch` `every_case_name_dispatches` near 2 MiB). | Not applicable: no `htui-orch` change. `run_case`'s future is the **max** of its arms, and the new arm is small. |
| **H-11** | `⚒` drawn 2 cells wide by some terminals. | Accepted (plan Risks). The locked `unicode-width 0.2.2` says 1 (probe). Tests measure with `cells::cell_width`, not by counting `char`s. |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles) | Gate (`--test-threads=1` on every test run) |
|---|---|---|---|
| T1 store read | `htui-core`: `model/run.rs`, `model/mod.rs`, `store/traits.rs`, `store/mem.rs`, `store/conformance.rs`, `tests/mem_store.rs`; `htui-store`: `pg/read.rs`, `cache/read.rs`, `backend.rs`, `writer.rs`, `tests/pg_conformance.rs`, `.sqlx/query-*.json` (new); `htui-agent`: `src/conformance.rs`, `tests/recorder.rs` | 3 (§2.8) | `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo test -p htui-store --all-features --test pg_conformance --test cache -- --test-threads=1 --nocapture 2>&1 \| grep -E "tool_call_counts\|test result"`; `cargo test -p htui-agent --all-features -- --test-threads=1`; `(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)`; `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| T2 protocol | `htui/src/store_worker.rs` | 2 (§3.5) | `cargo test -p htui --all-features --lib store_worker -- --test-threads=1`; clippy as above |
| T3 chips | `runs/execution_graph.rs`, `runs.rs`, `tests/backlog.rs`, 2 moved + 1 new `.snap` | 2 (§4.9) | `cargo test -p htui --all-features --lib detail::runs -- --test-threads=1` (B-10); `cargo test -p htui --all-features --test backlog -- --test-threads=1`; `cargo insta pending-snapshots` empty after review; `git ls-files crates/htui/tests/snapshots \| wc -l` = **135**; clippy |
| T4 close-out | `docs/decisions/mod/mod-72.md` (new), `DECISIONS.md`, `HANDOFF.md`, `docs/ANA-12.md`, the plan's status | 1 (§5) | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | — | §7, on the real tree |

---

## 2. T1: the store read (D1, D2, D3)

### 2.1 Model (`crates/htui-core/src/model/run.rs`)

Insert after `RunSummary` (`run.rs:812-844`), before `#[cfg(test)] mod tests` (`:846`). `StepId`
is already imported (`:8`), and so are `serde::{Deserialize, Serialize}` (`:4`).

```rust
/// One row of [`crate::store::ReadStore::tool_call_counts`] (MOD-72 plan D3): how many `tool_call`
/// events one step recorded of one `tool_kind`.
///
/// `tool_kind` is the wire string, not `htui_agent`'s `ToolKind`: no store crate depends on
/// `htui-agent`. A row whose payload has no JSON-string `tool_kind` counts as `"other"`, the
/// transport mappers' own fallback (`StepSummary::from_events` reads it the same way).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallCount {
    /// `session_event.run_step_id`.
    pub step: StepId,
    /// `session_event.payload ->> 'tool_kind'` when it is a string, else `"other"`.
    pub tool_kind: String,
    /// How many `tool_call` rows; never `0`.
    pub calls: u32,
}

impl ToolCallCount {
    /// Sorts by `(step, tool_kind bytes)`: every backend runs it after its own read, because
    /// Postgres would order text by collation and the mirror by bytes, the reason
    /// [`UpstreamEntry::sort_canonical`](crate::model::UpstreamEntry::sort_canonical) gives.
    pub fn sort_canonical(rows: &mut [Self]) {
        rows.sort_by(|a, b| {
            a.step
                .cmp(&b.step)
                .then_with(|| a.tool_kind.as_bytes().cmp(b.tool_kind.as_bytes()))
        });
    }
}
```

`StepId: Ord` is the UUID's byte order (`ids.rs:19-22`, `id_newtype!` derives `PartialOrd, Ord`).

Re-export: add `ToolCallCount` to `pub use run::{…}` (`model/mod.rs:158-164`). `cargo fmt` places
it.

### 2.2 Trait (`crates/htui-core/src/store/traits.rs`)

Add `ToolCallCount` to `use crate::model::{…}` (`:47-65`). Append to `ReadStore` after
`requirement_coverage` (`:264`), before the trait's closing `}` (`:265`). It's last, so every impl
appends at its own end (E2):

```rust

    // ---- MOD-72: per-step tool-call counts (ANA-12 §3.2) ------------------------------------

    /// How many `tool_call` rows each step of the item's runs recorded, per `payload.tool_kind`
    /// (MOD-72 plan D1-D3): one row per `(step, tool_kind)` with `calls >= 1`, in
    /// [`ToolCallCount::sort_canonical`](crate::model::ToolCallCount::sort_canonical) order. A
    /// `tool_kind` that is missing or not a JSON string counts as `"other"`; a `tool_result` is
    /// half of a call and is not counted. Empty for an unknown item, or one whose steps recorded
    /// no call.
    ///
    /// On `ReadStore` because `session_event` is mirrored (`cache_migrations/0001_mirror.sql:125-128`):
    /// offline the mirror answers from its last-N-steps window, and a step outside it has no row,
    /// which reads as "no calls" (plan D2).
    ///
    /// # Errors
    /// The backend's own failures only.
    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>>;
```

### 2.3 The seven implementors

| Implementor | File, insert after (pre-edit) | Import list to extend | Body |
|---|---|---|---|
| `MemStore` | `mem.rs:6489-6491` (`requirement_coverage`), before `}` `:6492`; the `State` helper after `State::step_log` (`:1344-1357`) | `mem.rs:24-47`: `EventKind`, `ToolCallCount` (B-7) | `Ok(self.read(\|state\| state.tool_call_counts(item)))`, with the helper in §2.4 |
| `PgStore` | `pg/read.rs:1065-1119`, before `}` `:1120` | `pg/read.rs:17-29` | §2.4 |
| `CacheStore` | `cache/read.rs:1220-1246`, before `}` `:1247` | `cache/read.rs:30-40` | §2.4 |
| `Backend` | `backend.rs:828-834`, before `}` `:835` | `backend.rs:27` list | `match self { Self::Memory(store) => store.tool_call_counts(item).await, Self::Online { pg, .. } => pg.tool_call_counts(item).await, Self::Offline { cache, .. } => cache.tool_call_counts(item).await }` |
| `Writer` | `writer.rs:305-310`, before `}` `:311` | `writer.rs:24` list | `match self { Self::Memory(store) => store.tool_call_counts(item).await, Self::Online(pg) => pg.tool_call_counts(item).await }` |
| `UsageSpy` | `htui-agent/src/conformance.rs:735-740`, before `}` `:741` | `:32` list | `async fn tool_call_counts(&self, item: ItemId) -> StoreResult<Vec<ToolCallCount>> { self.inner.tool_call_counts(item).await }` |
| `SpyStore` | `htui-agent/tests/recorder.rs:423-428`, before `}` `:429` | `:32-50` list | same as `UsageSpy` |

### 2.4 Backend bodies

**`MemStore`** (`State`, after `step_log`):

```rust
    /// MOD-72 plan D3: the item's `tool_call` rows per `(step, tool_kind)`, a kind that is not a
    /// JSON string counted as `other` (blueprint E3), in canonical order.
    fn tool_call_counts(&self, item: ItemId) -> Vec<ToolCallCount> {
        let steps: BTreeSet<StepId> = self
            .steps
            .values()
            .filter(|step| {
                self.runs
                    .get(&step.run_id)
                    .is_some_and(|run| run.item_id == Some(item))
            })
            .map(|step| step.id)
            .collect();
        let mut counts: BTreeMap<(StepId, String), u32> = BTreeMap::new();
        for event in &self.events {
            if event.kind != EventKind::ToolCall || !steps.contains(&event.run_step_id) {
                continue;
            }
            let kind = event
                .payload
                .get("tool_kind")
                .and_then(Value::as_str)
                .unwrap_or("other");
            let calls = counts
                .entry((event.run_step_id, kind.to_owned()))
                .or_insert(0);
            *calls = calls.saturating_add(1);
        }
        let mut rows: Vec<ToolCallCount> = counts
            .into_iter()
            .map(|((step, tool_kind), calls)| ToolCallCount { step, tool_kind, calls })
            .collect();
        ToolCallCount::sort_canonical(&mut rows);
        rows
    }
```

(`BTreeMap`/`BTreeSet` are imported at `mem.rs:14`, and `Value` at `:19`. The `BTreeMap` key is
already canonical, and `sort_canonical` keeps that true by construction, not by accident.)

**`PgStore`**: `query!`, not `query_as!`, because `calls` is a `u32`, which has no Postgres type
(E5). Write the text exactly as below. It was probed (see Verified at).

```rust
    /// MOD-72 plan D2, D3: per `(step, tool_kind)`, the item's `tool_call` rows. The kind is
    /// guarded by `jsonb_typeof` (blueprint E3): `->>` alone turns a number into its text, where
    /// `MemStore` and the mirror answer `other`.
    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>> {
        let rows = sqlx::query!(
            r#"
            SELECT e.run_step_id AS "step!: StepId",
                   CASE WHEN jsonb_typeof(e.payload->'tool_kind') = 'string'
                        THEN e.payload->>'tool_kind' ELSE 'other' END AS "tool_kind!",
                   COUNT(*) AS "calls!"
              FROM session_event e
              JOIN run_step s ON s.id = e.run_step_id
              JOIN run r      ON r.id = s.run_id
             WHERE r.item_id = $1 AND e.kind = 'tool_call'
             GROUP BY 1, 2
            "#,
            item.as_uuid(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut counts: Vec<ToolCallCount> = rows
            .into_iter()
            .map(|row| ToolCallCount {
                step: row.step,
                tool_kind: row.tool_kind,
                calls: u32::try_from(row.calls).unwrap_or(u32::MAX),
            })
            .collect();
        ToolCallCount::sort_canonical(&mut counts);
        Ok(counts)
    }
```

There's no `ORDER BY`, because Rust sorts (D3). The join is driven by `run.item_id`
(`idx_run_item`), then `run_step.run_id`, then `session_event`'s primary key `(run_step_id, seq)`
(`migrations/0001_init.sql:523`).

**`CacheStore`**: a runtime `sqlx::query` with a `?` bind, as `step_events` does (`cache/read.rs:649-686`). Probed:

```rust
    async fn tool_call_counts(&self, item: ItemId) -> Result<Vec<ToolCallCount>> {
        // MOD-72 blueprint E3: the `json_type` guard keeps this total and equal to the other two.
        // A bare `json_extract` hands back the integer 7 for `"tool_kind": 7`, which does not
        // decode as `String` and would fail the whole read (the T68 rule above).
        let rows = sqlx::query(
            "SELECT e.run_step_id AS step, \
                    CASE WHEN json_type(e.payload, '$.tool_kind') = 'text' \
                         THEN json_extract(e.payload, '$.tool_kind') ELSE 'other' END AS tool_kind, \
                    COUNT(*) AS calls \
               FROM session_event e \
               JOIN run_step s ON s.id = e.run_step_id \
               JOIN run r      ON r.id = s.run_id \
              WHERE r.item_id = ? AND e.kind = 'tool_call' \
              GROUP BY e.run_step_id, tool_kind",
        )
        .bind(item.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut counts = rows
            .iter()
            .map(|row| {
                Ok(ToolCallCount {
                    step: uuid_col("session_event.run_step_id", &text(row, "step")?)?,
                    tool_kind: text(row, "tool_kind")?,
                    calls: u32::try_from(get::<i64>(row, "calls")?).unwrap_or(u32::MAX),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ToolCallCount::sort_canonical(&mut counts);
        Ok(counts)
    }
```

`tool_kind` in `GROUP BY` resolves to the result alias, because no joined table has a column by
that name (`0001_mirror.sql:103-128`). The probe ran this exact text.

### 2.5 Conformance cases (`crates/htui-core/src/store/conformance.rs`)

Add `ToolCallCount` to the `crate::model::{…}` import (`:16-39`). `EventKind`, `EventRole`,
`SessionEvent`, `StepFence`, `json!` and `Value` are already imported.

**`READ_CASES`**: append `"tool_call_counts_read_back",` after
`"requirement_revisions_or_not_cached",` (`:510`). Add a dispatch arm after `:541`:
`"tool_call_counts_read_back" => tool_call_counts_read_back(store).await,`.

```rust
/// MOD-72 plan D2, D3: the fixture's one `tool_call` (a `read` on `FEAT-1`'s plan step) is the
/// whole answer for `FEAT-1`, its `tool_result` not a second call; an item whose steps recorded no
/// call, one with no run and an unknown one answer nothing.
async fn tool_call_counts_read_back<S: ReadStore>(store: &S) {
    const CASE: &str = "tool_call_counts_read_back";
    assert_eq!(
        store.tool_call_counts(ids::HTUI_FEAT_1).await.expect(CASE),
        vec![ToolCallCount {
            step: ids::STEP_PLAN,
            tool_kind: "read".to_owned(),
            calls: 1,
        }],
        "{CASE}: the plan step's one read, its result not counted"
    );
    for item in [ids::HTUI_ANA_1, ids::HTUI_ANA_2, ItemId::new()] {
        assert!(
            store.tool_call_counts(item).await.expect(CASE).is_empty(),
            "{CASE}: {item} recorded no call"
        );
    }
}
```

(`HTUI_ANA_1` owns `RUN_3`, whose two steps have no events. `HTUI_ANA_2` has no run. Both hold on
the mirror too, because `RUN_3` is mirrored.)

**`CASES`**: append `"tool_call_counts_group_by_step_and_kind",` after
`"a_phase_persona_binding_sets_keeps_and_clears",` (`:182`). Add a dispatch arm before the `other
=>` panic arm (`:460`): `"tool_call_counts_group_by_step_and_kind" => {
tool_call_counts_group_by_step_and_kind(store).await; }`.

```rust
/// One `session_event` row of `kind` for `step` at `seq`, with `payload`; a tool row carries a
/// `tool_call_id` (§4.3).
fn tool_event(step: StepId, seq: i32, kind: EventKind, payload: Value) -> SessionEvent {
    SessionEvent {
        run_step_id: step,
        seq,
        turn: 0,
        kind,
        role: EventRole::Agent,
        tool_call_id: Some(format!("call-{seq}")),
        payload,
        raw: None,
        at: Utc::now(),
    }
}

/// `calls` `tool_kind` calls of `step`, as `tool_call_counts` answers it.
fn tool_calls_of(step: StepId, tool_kind: &str, calls: u32) -> ToolCallCount {
    ToolCallCount { step, tool_kind: tool_kind.to_owned(), calls }
}

/// MOD-72 plan D1-D3: counts per step and kind across two steps of one run; a `tool_result` and
/// a chat row are not calls; a missing, `null` or non-string `tool_kind` is `other` on every
/// backend (blueprint B-2); another item's run stays its own.
async fn tool_call_counts_group_by_step_and_kind<S: WriteStore>(store: &S) {
    const CASE: &str = "tool_call_counts_group_by_step_and_kind";
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    let first = store.create_step(new_run_step(run, 0, 1, 0)).await.expect(CASE).id;
    let second = store.create_step(new_run_step(run, 1, 1, 0)).await.expect(CASE).id;
    let other_run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_CLEAN_1, Vec::new()))
        .await
        .expect(CASE)
        .id;
    let elsewhere = store.create_step(new_run_step(other_run, 0, 1, 0)).await.expect(CASE).id;

    let call = EventKind::ToolCall;
    let batches = [
        vec![
            tool_event(first, 0, call, json!({ "title": "cargo test", "tool_kind": "execute" })),
            // A result names its call's kind too, so a kind-blind count would say 3.
            tool_event(first, 1, EventKind::ToolResult,
                       json!({ "status": "completed", "tool_kind": "execute" })),
            tool_event(first, 2, call, json!({ "title": "cargo fmt", "tool_kind": "execute" })),
            tool_event(first, 3, call, json!({ "title": "Read src/main.rs", "tool_kind": "read" })),
            tool_event(first, 4, call, json!({ "title": "no kind at all" })),
            chat_event(first, 5),
        ],
        vec![
            tool_event(second, 0, call, json!({ "title": "Edit", "tool_kind": "edit" })),
            tool_event(second, 1, call, json!({ "title": "null kind", "tool_kind": null })),
            tool_event(second, 2, call, json!({ "title": "number kind", "tool_kind": 7 })),
        ],
        vec![tool_event(elsewhere, 0, call, json!({ "title": "Read", "tool_kind": "read" }))],
    ];
    for rows in &batches {
        assert_eq!(
            store.append_events(StepFence::Unleased, rows).await.expect(CASE),
            rows.len(),
            "{CASE}: an unclaimed run's rows land unleased"
        );
    }

    let mut expected = vec![
        tool_calls_of(first, "execute", 2),
        tool_calls_of(first, "other", 1),
        tool_calls_of(first, "read", 1),
        tool_calls_of(second, "edit", 1),
        tool_calls_of(second, "other", 2),
    ];
    ToolCallCount::sort_canonical(&mut expected);
    assert_eq!(
        store.tool_call_counts(ids::HTUI_ANA_2).await.expect(CASE),
        expected,
        "{CASE}: per step and kind; the result and the chat row are not calls; a missing, null \
         or numeric kind is `other`"
    );
    assert_eq!(
        store.tool_call_counts(ids::HTUI_CLEAN_1).await.expect(CASE),
        vec![tool_calls_of(elsewhere, "read", 1)],
        "{CASE}: the other item's run is its own"
    );
}
```

`chat_event` is `conformance.rs:1628-1640`. `new_run` and `new_run_step` are `:4533-4559`.
`cargo fmt` reflows the hand-wrapped `tool_event` call.

### 2.6 Pins and `.sqlx`

- `crates/htui-core/tests/mem_store.rs:35-61`: `130` → `131`. Append to the message: `…, MOD-26 T1's
  five persona cases (plan D3-D5), and MOD-72's one for the tool-call counts (plan D1-D3)`.
  `:62-68`: `14` → `15`, and append `…, and MOD-72's tool-call counts (plan D2)`.
- `crates/htui-store/tests/pg_conformance.rs:18-26`: the doc gains "…, and MOD-72's tool-call
  count case (plan D1-D3) makes it 131". Change `EXPECTED_CASES` to `131`, and the `:33-34`
  message to `"(131 since MOD-72's tool-call count case)"`. The `READ_CASES` loop (`:53-75`) needs
  no pin.
- `.sqlx`, sandbox recipe (`docs/hr-sandbox.md:196-210`). `htui_sqlx` exists already, so skip the
  `CREATE DATABASE` (or ignore its "already exists"):
  ```bash
  cd crates/htui-store
  export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
  cargo sqlx migrate run --source migrations          # idempotent; brings it to 0012
  cargo sqlx prepare -- --all-targets --all-features  # never without both flags
  cargo sqlx prepare --check
  git status --porcelain .sqlx                          # exactly one `??`
  ```

### 2.7 Tests (first)

| Test | Where | Pins |
|---|---|---|
| `tool_call_counts_read_back` | READ case; runs on Mem (`run_read_case_accepts_every_name_in_read_cases`, `conformance.rs:16020`; `mem_store.rs`), Pg (`pg_conformance.rs` read loop) and the mirror (`cache.rs:1055`) | §2.5 |
| `tool_call_counts_group_by_step_and_kind` | CASES entry; runs on Mem (`run_case_accepts_every_name_in_cases`, `:16008`; `mem_store.rs`) and Pg (`pg_store_conformance`) | §2.5 |
| `tool_call_counts_sort_by_step_then_kind_bytes` | `model/run.rs` `mod tests` | `s1 = StepId::from_uuid(Uuid::from_u128(1))`, `s2 = …(2)`. Rows `[(s2,"read"), (s1,"read"), (s1,"edit"), (s1,"Zed")]` sort to `[(s1,"Zed"), (s1,"edit"), (s1,"read"), (s2,"read")]`. `"Zed"` before `"edit"` is byte order, which a collation would reverse |
| the two count pins | `mem_store.rs`, `pg_conformance.rs` | 131 / 15 / 131 |

### 2.8 Commits (T1)

1. `test(mod-72): tool-call count conformance cases and pins (red)`. §2.1 with `sort_canonical`'s
   body `todo!("MOD-72 T1")`, plus its unit test. §2.2. The Mem/Pg/Cache bodies are
   `todo!("MOD-72 T1")`, and the four delegations (Backend, Writer, UsageSpy, SpyStore) are real.
   Also §2.5 and the §2.6 pins. Red: the new cases panic, and no product path calls the method (H-3).
   An unused-parameter warning on a `todo!()` body is acceptable here, because the gate runs on
   commit 3.
2. `feat(mod-72): MemStore counts tool calls per step and kind`: `sort_canonical`'s body and the
   Mem body. Gate: `cargo test -p htui-core --all-features -- --test-threads=1` green.
3. `feat(mod-72): Postgres and the mirror count tool calls`: the Pg and Cache bodies plus the one
   `.sqlx` entry (H-2). Gate: the whole T1 row of §1.

---

## 3. T2: the worker protocol (D4)

**File**: `crates/htui/src/store_worker.rs`. Add `ToolCallCount` to `use htui_core::model::{…}`
(`:24-32`).

### 3.1 Variants

`StoreRequest`: after `RelayView { item }` (`:787-792`), before `AnswerPermission` (`:793-799`):

```rust
    /// MOD-72 plan D4: per-step tool-call counts of the item's runs, for the Runs flow's chips.
    /// An ordinary read, so the mirror answers it offline (plan D2).
    ToolCalls {
        /// The item the Runs pane shows.
        item: ItemId,
    },
```

`StoreReply`: after `RelayView { item, view }` (`:1270-1276`), before `PermissionAnswered`
(`:1277-1282`):

```rust
    /// Answer to [`StoreRequest::ToolCalls`] for `item`, in `ToolCallCount::sort_canonical` order.
    ToolCalls {
        /// The item asked about.
        item: ItemId,
        /// One row per `(step, tool_kind)` with at least one call.
        counts: Vec<ToolCallCount>,
    },
```

Unboxed: a `Vec` is 24 bytes. `RelayView` is boxed because it's a struct of two `Vec`s plus
fields. No `large_enum_variant` (E9).

### 3.2 `name()` (`:931-1052`)

After `Self::AnswerPermission { .. } => "answer_permission",` (`:1033`):

```rust
            // MOD-72 plan D4.
            Self::ToolCalls { .. } => "tool_calls",
```

### 3.3 `try_serve` (`:1571-…`, no wildcard, so exhaustive)

After `StoreRequest::Runs(id) => …` (`:1587`):

```rust
        // MOD-72 D4: an ordinary read, so an `Unreachable` drops an `Online` backend onto the
        // mirror, which answers from its window (plan D2) - unlike `RelayView`, which is the
        // writer's.
        StoreRequest::ToolCalls { item } => StoreReply::ToolCalls {
            item: *item,
            counts: backend.tool_call_counts(*item).await?,
        },
```

`ReadStore` is in scope (`:37-40`), and no `WorkerHost` import shadows it.

**Other exhaustive lists, checked**: the only exhaustive matches over `StoreRequest` are `name()`
and `try_serve`. `spawn`'s loop (`:2104`) and both testkit loops (`testkit.rs:271-363`, `:520-537`)
fall through to `serve` for unlisted variants. `backlog/mod.rs:2084-2093` and `:2105-2116` filter
with `_ => false`. There's no exhaustive match over `StoreReply` (`app/update.rs` and the panes
match the variants they want and end in `_`). There's no name-uniqueness test. `RunsTab::on_reply`
gains its arm in T3.

### 3.4 Test (first), after `step_events_answers_the_rows_of_a_recorded_step` (`:3315-3332`)

```rust
    /// MOD-72 plan D4: the Runs flow's chip read, through the ordinary read path.
    #[tokio::test]
    async fn tool_calls_answers_the_item_s_per_step_counts() {
        let backend = demo();
        let StoreReply::ToolCalls { item, counts } = serve(
            &backend,
            &StoreRequest::ToolCalls { item: ids::HTUI_FEAT_1 },
        )
        .await
        else {
            panic!("wrong reply variant")
        };
        assert_eq!(item, ids::HTUI_FEAT_1, "the reply names the item it read");
        assert_eq!(
            counts,
            vec![ToolCallCount { step: ids::STEP_PLAN, tool_kind: "read".to_owned(), calls: 1 }],
            "the fixture's one read on the plan step"
        );
        assert_eq!(StoreRequest::ToolCalls { item }.name(), "tool_calls");

        let StoreReply::ToolCalls { counts, .. } =
            serve(&backend, &StoreRequest::ToolCalls { item: ids::HTUI_ANA_2 }).await
        else {
            panic!("wrong reply variant")
        };
        assert!(counts.is_empty(), "an item with no run has no chips");
    }
```

(`demo()` is `store_worker.rs:2749`.)

### 3.5 Commits (T2)

1. `test(mod-72): ToolCalls worker test (red)`: §3.1, §3.2, and the §3.3 arm as
   `StoreRequest::ToolCalls { .. } => todo!("MOD-72 T2")`, plus §3.4. Nothing sends `ToolCalls`
   yet (H-3).
2. `feat(mod-72): the store worker serves ToolCalls`: the §3.3 body. Gate: the T2 row of §1.

---

## 4. T3: chips in the flow view (D5, D6, D7)

**Files**: `crates/htui/src/ui/tabs/backlog/detail/runs/execution_graph.rs`,
`crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `crates/htui/tests/backlog.rs`, and
`crates/htui/tests/snapshots/backlog__runs_flow_{fanout,reject_note,tool_chips}.snap`.

### 4.1 `execution_graph.rs`: imports and constants

- `:10` becomes `use htui_core::model::{RunId, RunStepSummary, RunSummary, StepId, StepStatus,
  ToolCallCount};`. `BTreeMap` is imported (`:8`), and so is `cells` (`:22`).
- `:26-27` becomes:
  ```rust
  /// Plan D10, MOD-72 D5: a node's height, a border, three lines (head, phase, chips) and a border.
  const NODE_H: f64 = 5.0;
  ```
  The `V_GAP` doc and value stay (3). `Y_STEP` follows (8).
- After `MARGIN` (`:40`):
  ```rust
  /// MOD-72 D6: the glyph that leads a node's chip line, once.
  const TOOL: char = '\u{2692}';
  /// MOD-72 D6: the sign between a chip's label and its count.
  const TIMES: char = '\u{d7}';
  ```

### 4.2 Pure functions (after `impl NodeContent for StepNode`, `:231-251`, before `handles`, `:254`)

```rust
/// MOD-72 D6: the short label of a `tool_kind` wire value (ACP's ten, `htui_agent::event::ToolKind`);
/// an unknown value is drawn as itself.
fn short_label(tool_kind: &str) -> &str {
    match tool_kind {
        "delete" => "del",
        "search" => "find",
        "execute" => "exec",
        "switch_mode" => "mode",
        // `read`, `edit`, `move`, `think`, `fetch`, `other`, and any value a later protocol adds.
        other => other,
    }
}

/// MOD-72 D6: a step's tool calls as one line of at most `width` cells: the tool glyph, then
/// `label×n` chips, larger counts first and ties by label, then `+N` for the `N` kinds that did
/// not fit. Empty when the step made no call. Never wider than `width` once `width` holds the
/// glyph and `+N`; `StepNode::render` clips anyway.
pub(super) fn chips(calls: &[ToolCallCount], width: usize) -> String

/// MOD-72 D7: a `ToolCalls` reply's rows by step, each step's rows in reply order.
pub(super) fn by_step(counts: &[ToolCallCount]) -> BTreeMap<StepId, Vec<ToolCallCount>> {
    let mut map: BTreeMap<StepId, Vec<ToolCallCount>> = BTreeMap::new();
    for count in counts {
        map.entry(count.step).or_default().push(count.clone());
    }
    map
}
```

`chips`, as pseudo-code (E11, E12):

```text
chips(calls, width):
    shown := [(short_label(c.tool_kind), c.calls) for c in calls if c.calls > 0]   # never ×0
    if shown empty: return ""
    sort shown by (calls DESC, label bytes ASC)
    line := String::from(TOOL);  used := cell_width(line)                          # 1
    n := len(shown)
    for (i, (label, count)) in shown.enumerate():
        chip    := format!(" {label}{TIMES}{count}")
        left    := n - i - 1                        # kinds after this one
        reserve := if left == 0 { 0 } else { cell_width(format!(" +{left}")) }
        if used + cell_width(chip) + reserve > width:
            # stop at the first chip that does not fit: the shown chips are always the top ones.
            # When i > 0, chip i-1 reserved exactly " +{n-i}", so this fits.
            return line + format!(" +{}", n - i)
        line += chip;  used += cell_width(chip)
    return line
```

`cells::cell_width` is `cells.rs:66`. It's per-cluster display width, the measure `ratatui`
draws by.

### 4.3 `StepNode` (`:165-251`)

- Field, after `phase` (`:170`):
  ```rust
      /// Line 3 (MOD-72 D5): this step's tool-call rows, fitted to the interior at draw time
      /// (blueprint E10); none for a step that made no call.
      calls: Vec<ToolCallCount>,
  ```
- `new` (`:179`) becomes `pub(super) fn new(step: &RunStepSummary, siblings: &[RunStepSummary],
  calls: &[ToolCallCount], theme: &Theme) -> Self`. Its doc gains "and `calls`, its rows of the
  last `ToolCalls` reply". It sets `calls: calls.to_vec()`.
- `render` (`:235-250`). The doc gains "then the chips, dim". The loop becomes:
  ```rust
          let width = usize::from(inner.width);
          let chips = chips(&self.calls, width);
          let lines = [
              (self.head.as_str(), self.text()),
              (self.phase.as_str(), self.text()),
              (chips.as_str(), self.theme.dim), // MOD-72 D6: secondary to the head and phase
          ];
          for (row, (line, style)) in (0u16..).zip(lines) {
              if row >= inner.height {
                  break; // zoom 0.5: no interior, so no line (D5)
              }
              buf.set_string(inner.x, inner.y + row, cells::clip(line, width), style);
          }
  ```
  An empty chips line writes nothing, so the row stays blank. `#[derive(PartialEq)]` still holds
  (`ToolCallCount: PartialEq`).

### 4.4 `ExecutionGraph::sync` (`:312-397`)

The signature becomes:

```rust
    pub(super) fn sync(
        &mut self,
        run: Option<&RunSummary>,
        cursor: Option<StepId>,
        calls: &BTreeMap<StepId, Vec<ToolCallCount>>,
        theme: &Theme,
    )
```

Its doc gains: "`calls` is the pane's last `ToolCalls` reply by step (MOD-72 D7). A re-sync that
only changes it keeps the viewport, because no position moves." The node line (`:340`) becomes
`StepNode::new(step, &run.steps, calls.get(id).map_or(&[][..], Vec::as_slice), theme)`. Nothing
else changes: the anchor (review L2) and `Reveal` logic already keep a same-run re-sync still.

**Every call site**: `runs.rs:386` (§4.5). Tests: `synced` (`execution_graph.rs:843-847`, which
passes `&BTreeMap::new()`), `:1038`, `:1051`, `:1208`, `:1248`, `:1275`. Each gains
`&BTreeMap::new()` before the theme.

### 4.5 `runs.rs`

- `:48` becomes `use std::collections::{BTreeMap, BTreeSet};`. Add `ToolCallCount` to `:51-55`.
  Add `use self::execution_graph::{ExecutionGraph, by_step};` at `:65`.
- Module doc: after the MOD-28 paragraph (`:37-39`), insert:
  ```text
  //!
  //! MOD-72: in the flow, a node's third line counts its step's tool calls by kind
  //! (`⚒ read×5 exec×3`). Every `Runs` reply in the flow, and `v` into it, asks for them
  //! (`ToolCalls`); the list never does.
  ```
- `RunsTab` field, after `graph` (`:208`):
  ```rust
      /// MOD-72 D7: the last `ToolCalls` reply for [`RunsTab::item`], by step. An item change
      /// clears it; only the flow view asks for it (plan D4), and a toggle back keeps it (E14).
      tool_calls: BTreeMap<StepId, Vec<ToolCallCount>>,
  ```
  `#[derive(Debug, Default)]` holds.
- `sync_graph` (`:377-387`): `self.graph.get_mut().sync(run, step, &self.tool_calls, theme);`. The
  borrows are disjoint fields (`runs`, `tool_calls`, `graph`), so this compiles under `&mut self`.
- `on_runs`, after `ctx.request(StoreRequest::RelayView { item });` (`:429`):
  ```rust
          // MOD-72 D4: the chips, only while they are drawn; last, so the list's requests are
          // byte-identical to before.
          if self.view == View::Flow {
              ctx.request(StoreRequest::ToolCalls { item });
          }
  ```
- `on_item_change` (`:1228-1237`): add `self.tool_calls.clear();` after `self.relay = None;`
  (`:1233`).
- The `v` arm (`:1291-1297`) becomes:
  ```rust
              KeyCode::Char('v') => {
                  self.view = match self.view {
                      View::List => View::Flow,
                      View::Flow => View::List,
                  };
                  // MOD-72 D4: into the flow, the chips are asked for once.
                  if self.view == View::Flow
                      && let Some(item) = self.item
                  {
                      ctx.request(StoreRequest::ToolCalls { item });
                  }
                  self.sync_graph(ctx.theme);
              }
  ```
  `let`-chains are already in use in this workspace (`prompt/render.rs:835-840`).
- `on_reply`, after the `RelayView` arm (`:1396-1398`):
  ```rust
              // MOD-72 D4, D7: this item's chips; another item's reply is dropped.
              StoreReply::ToolCalls { item, counts } if Some(*item) == self.item => {
                  self.tool_calls = by_step(counts);
                  self.sync_graph(ctx.theme);
              }
  ```
  A `Failed { request: "tool_calls", .. }` reaches the status line through `app/update.rs` as any
  read failure does. No new error path.

### 4.6 Tests (first): unit tests

**`execution_graph.rs` `mod tests`**: add the helper `fn kind(tool_kind: &str, calls: u32) ->
ToolCallCount { ToolCallCount { step: id(1), tool_kind: tool_kind.to_owned(), calls } }`. Add
`ToolCallCount` to the test imports if `super::*` doesn't cover it (it does, through the module's
`use`).

| Test | Pins |
|---|---|
| `chips_lead_with_the_tool_and_put_larger_counts_first` | `chips(&[kind("execute", 3), kind("read", 5)], 18) == "\u{2692} read\u{d7}5 exec\u{d7}3"` (D6's own example). `cell_width` of it is 15 |
| `chips_break_a_tie_by_label` | `chips(&[kind("edit", 2), kind("delete", 2)], 18) == "\u{2692} del\u{d7}2 edit\u{d7}2"` |
| `every_known_kind_has_its_short_label` | `short_label` over the ten wire values `read edit delete move search execute think fetch switch_mode other` gives `read edit del move find exec think fetch mode other` |
| `an_unknown_kind_is_drawn_as_itself` | `chips(&[kind("mcp_tool", 1)], 18) == "\u{2692} mcp_tool\u{d7}1"` |
| `chips_that_do_not_fit_become_plus_n` | `chips(&[kind("read",5), kind("execute",3), kind("edit",2), kind("search",1)], 18) == "\u{2692} read\u{d7}5 exec\u{d7}3 +2"`, and its `cell_width` is 18 |
| `the_last_chip_needs_no_room_for_plus_n` | `chips(&[kind("read",5), kind("delete",1)], 14) == "\u{2692} read\u{d7}5 del\u{d7}1"` (14 cells). At width 13 it's `"\u{2692} read\u{d7}5 +1"` |
| `chips_are_fitted_by_display_width` | `chips(&[kind("\u{8abf}\u{67fb}", 2)], 7) == "\u{2692} +1"` and `…, 8) == "\u{2692} \u{8abf}\u{67fb}\u{d7}2"`. Counting `char`s (6 ≤ 7) would wrongly fit it at 7 |
| `no_calls_is_no_line` | `chips(&[], 18) == ""` and `chips(&[kind("read", 0)], 18) == ""` |
| `by_step_groups_the_rows_of_a_reply` | rows for `id(1)` (2 kinds) and `id(2)` (1). `by_step` has 2 keys and `[&id(1)].len() == 2` |
| `a_node_draws_its_chips_dim_on_the_third_line` | `run(1, vec![step(1,0,1,0)])`, calls `{id(1): [kind("read",1)]}`. `graph.sync(Some(&run), Some(id(1)), &calls, &Theme::default())`, then draw. With `(x, y) = corner_of(&buf, "0.1 done")`: `rows(&buf)[y+3]` contains `"\u{2692} read\u{d7}1"`. `buf[(x + 1, y + 3)].fg == Color::DarkGray` (`theme.dim`). `buf[(x, y + 4)].symbol() == "\u{2514}"` (the bottom-left corner: 5 rows) |
| `a_node_without_calls_draws_a_blank_third_line` | `synced(&run(1, vec![step(1,0,1,0)]), Some(id(1)))`, draw. Every cell `(x+1..x+19, y+3)` is `" "`, `buf[(x, y+3)]` is `"\u{2502}"`, and `buf[(x, y+4)]` is `"\u{2514}"` |
| `a_re_sync_with_new_counts_keeps_the_viewport` | Two-step run, cursor `id(2)`. Draw, `zoom_in`, draw. Record `flow.viewport`. Re-sync the same run with `{id(2): [kind("read",1)]}`: `flow.viewport` is equal before and after the next draw, and that draw's rows contain `"\u{2692} read\u{d7}1"` (D7) |
| **moved** (H-1) | `:616-617`, `:636`, `:825-827` y-literals → 8/16. `:1011` → `y + 1..y + 4` (B-6). The six `sync` call sites gain `&BTreeMap::new()` (§4.4) |

**`runs.rs` `mod tests`**: new tests after `the_flow_view_survives_an_item_change`
(`:3851-3857`). They use `requests` (`:3290-3298`), `pane` (`:1649-1656`), `lines` (`:1737-1751`,
43×16) and `key`. The helper is `fn read_on(step: StepId) -> ToolCallCount { ToolCallCount {
step, tool_kind: "read".to_owned(), calls: 1 } }`.

| Test | Pins |
|---|---|
| `v_into_the_flow_asks_for_the_tool_calls_once` | `pane(&shell)` (drained), then `v`: `requests(shell.emit.take())` matches `[StoreRequest::ToolCalls { item }]` with `item == HTUI_FEAT_1`. Then `v` back to the list: `shell.emit.take().is_empty()` |
| `a_runs_reply_in_flow_asks_for_the_tool_calls_last` | `pane`, `v`, drain, then `on_reply(Runs(feat_1_runs()))`: the list matches `[RunActions(a), RelayView { item: r }, ToolCalls { item: t }]`, all `HTUI_FEAT_1` (E15). The list-view lists stay pinned by `:3307-3350` (B-9) |
| `a_tool_calls_reply_draws_the_chips_in_flow` | `pane`, `v`, `J` (cursor on `STEP_PLAN`, so the reveal keeps its node whole in 16 rows), drain. `on_reply(ToolCalls { item: HTUI_FEAT_1, counts: vec![read_on(STEP_PLAN)] })`: `pane.tool_calls[&STEP_PLAN].len() == 1`, and some row of `lines(&pane, &shell)` contains `"\u{2692} read\u{d7}1"` |
| `a_tool_calls_reply_for_another_item_is_dropped` | `pane`, `v`, then `on_reply(ToolCalls { item: HTUI_ANA_2, counts: vec![read_on(STEP_PLAN)] })`: `pane.tool_calls.is_empty()` |
| `an_item_change_forgets_the_tool_calls` | after an applied reply (as above), `on_item_change(Some(HTUI_ANA_2))`, then `pane.tool_calls.is_empty()` |
| **moved** (B-1) | `:3664`: the `v`-sends-nothing assert becomes the `[ToolCalls { item: HTUI_FEAT_1 }]` match. `:3808` and `:3839`: insert `let _ = shell.emit.take();` after `v` |

### 4.7 Tests (first): integration (`crates/htui/tests/backlog.rs`)

After `an_action_key_in_flow_answers_as_in_the_list` (`:868-888`), before the Graph section header (`:890-893`):

```rust
/// MOD-72 D1, D5, D6: `FEAT-1`'s plan step recorded the fixture's one tool call, a `read`, so its
/// node's third line is `⚒ read×1`; no other node draws a chip.
#[tokio::test]
async fn the_flow_draws_the_plan_step_s_tool_call_as_a_chip() {
    let mut harness = backlog().await;
    down(&mut harness, TO_FEAT_1).await;
    sub_tab(&mut harness, 1);
    harness.key("v");
    harness.drive_to_end().await;
    let frame = harness.render();
    assert!(
        frame.contains("\u{2692} read\u{d7}1"),
        "the plan step's chip:\n{frame}"
    );
    assert_eq!(
        frame.matches('\u{2692}').count(),
        1,
        "only the plan step made a call:\n{frame}"
    );
    insta::assert_snapshot!("runs_flow_tool_chips", frame);
}
```

Reply ordering: `v` queues `ToolCalls`. `drive_to_end` → `drive` drains the channel through
`store_worker::serve` (`testkit.rs:363`, the `(request, _)` arm), applies the reply, and repeats
until quiet (`SETTLE_ROUNDS`). `Backlog::on_reply` forwards to every detail sub-tab
(`backlog/mod.rs:682`, `detail/mod.rs:209-213`), so the reply re-syncs before `render`. The
fixture run is linear, 4 nodes. In the 22-row canvas the Reset puts them at canvas rows 1–5, 9–13
and 17–21, plus the review node below the fold. The chip is on canvas row 12.

### 4.8 Snapshots

- `backlog__runs_flow_fanout.snap` and `backlog__runs_flow_reject_note.snap`. Each node gains one
  blank interior row (`│                  │`) between the phase row and `└…┘`. Nothing else moves,
  and there are no chips (B-11). In `reject_note` the footer rows stay at the bottom.
- `backlog__runs_flow_tool_chips.snap` (new). Check the `FEAT-1` header, the run line, `0.1 done`
  / `prd`, then `1.1 done` / `plan` / `⚒ read×1`, then `2.1 done` / `implement`, all three nodes at
  the same column, and no `kind   status` list header.
- Review with `cargo insta review`. Accept exactly those three. Then `git ls-files
  crates/htui/tests/snapshots | wc -l` = **135**, and `git status --porcelain
  crates/htui/tests/snapshots` shows two ` M` and one `??`.

### 4.9 Commits (T3)

1. `test(mod-72): chip tests, the sync signature and the pane's map (red)`. This commit is
   behaviour-neutral:
   - §4.1 imports and `TOOL`/`TIMES`. `NODE_H` stays **4**.
   - §4.2 with `todo!("MOD-72 T3")` bodies in `short_label`, `chips` and `by_step`. Each carries
     `#[cfg_attr(not(test), expect(dead_code, reason = "MOD-72 T3's green commit draws the
     chips"))]`, as do `TOOL`/`TIMES`.
   - The §4.3 field and `new` parameter. `render` is unchanged, so nothing draws the chips.
   - The §4.4 signature and all call sites.
   - The §4.5 field, `on_item_change` clear and `sync_graph` argument. Not the requests or the
     reply arm.
   - All §4.6/§4.7 tests, including the moved literals and the B-1 edits.

   Red: the chip tests panic in `todo!()`, the y-literals expect 8/16, `v` asks for nothing, and
   the new snapshot is pending.
2. `feat(mod-72): tool-call chips in the Runs flow view`:
   - `NODE_H` 5 and the §4.2 bodies, with the `expect(dead_code)` attributes removed.
   - The §4.3 `render` and the §4.5 module doc, `on_runs` request, `v` arm and reply arm.
   - The three accepted snapshots (§4.8).

   Gate: the T3 row of §1, then `cargo test -p htui --all-features -- --test-threads=1`.

### 4.10 Data flow (whole feature)

`v` (into flow) or a `Runs` reply while in flow sends `StoreRequest::ToolCalls { item }`. The
worker's `try_serve` calls `Backend::tool_call_counts`: Memory, Online (Pg `query!`) or Offline
(mirror SQL), each sorted canonically. The worker replies `StoreReply::ToolCalls { item, counts }`.
The app's staleness gate keeps the newest. `Backlog` → `Detail::on_reply` → `RunsTab::on_reply`,
guarded by `Some(item) == self.item`. `by_step` fills `tool_calls`, and `sync_graph` (flow only)
calls `ExecutionGraph::sync(run, cursor, &tool_calls, theme)`. Each `StepNode` holds its step's
rows, positions are unchanged, and the viewport is kept. `render` → `StepNode::render` →
`chips(rows, inner.width)` draws the third line in `theme.dim`. The live `RunStream` re-read and the
5-refresh active-run poll re-ask `Runs`, so they refresh the chips with no new wiring (D4).

---

## 5. T4: close-out docs

1. `docs/decisions/mod/mod-72.md` (new, the `mod-28.md` shape): D1–D8 as decided, B-1..B-11,
   E1–E17, the pins moved (Scope), and D8's out-of-scope list as possible follow-ups. **None are
   minted** unless the maintainer asks.
2. `DECISIONS.md`: one index line at the top (`:5` style): `- **[MOD-72](docs/decisions/mod/mod-72.md)**
   - Tool-call chips in the Runs flow view (done, <date>)`.
3. `HANDOFF.md`:
   - Tick `:266` (MOD-72) per `lifecycle.md` P2.
   - Re-count the pins at `:45-48`, adding "MOD-72" to the "Pins after" list: `CASES` 131,
     `READ_CASES` 15, `StoreRequest` 97, `StoreReply` 56, 319 `.sqlx`, 135 snapshots. **Re-count,
     don't add** (B-8, H-9).
   - In the MOD-N row (`:686`), drop "MOD-72 tool-call chips" and re-count the total.
4. `docs/ANA-12.md:7` status line: append "MOD-72 done (<date>): §3.2's tool-call chips, by
   `tool_kind` (`⚒ read×5 exec×3`), in the flow view."
5. `.claude/plans/mod-72-tool-call-chips.plan.md`: Status → done, and tick the Acceptance boxes.
6. Commit: `docs(mod-72): close-out - write-up, DECISIONS index, HANDOFF (status, pins, MOD count),
   ANA-12 status`. Run §7 on the real tree **before** this commit.

---

## 6. Blueprint decisions

| # | Decision | Why |
|---|---|---|
| **E1** | `ToolCallCount { step, tool_kind, calls }` lives in `model/run.rs`, derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, and has `sort_canonical` by `(step, tool_kind bytes)` | D3. The model module's derive rule (`model/mod.rs:3-5`), plus `Eq`, which no field forbids. `UpstreamEntry::sort_canonical` is the precedent |
| **E2** | The trait method is **last** in `ReadStore`, under its own `MOD-72` banner, and every implementor appends it after `requirement_coverage` | One uniform anchor in seven files, and the trait's grouping by milestone (`traits.rs:154`, `:207` banners) |
| **E3** | The kind is type-guarded on all three backends (`jsonb_typeof` / `json_type` / `as_str`) instead of a bare `COALESCE` | B-2: parity and totality. Probe-proven. It narrows D3's SQL and keeps D3's meaning |
| **E4** | `u32::try_from(i64).unwrap_or(u32::MAX)`, and Mem uses `saturating_add` | `as` wraps silently. Saturating is the truthful bound. `clippy::pedantic` is off, so no lint would catch an `as` (H-5) |
| **E5** | Postgres uses `query!` + map, `GROUP BY 1, 2`, no `ORDER BY`. SQLite is a runtime `query`, `GROUP BY e.run_step_id, tool_kind` | `query_as!` can't target a `u32` field. Order comes from Rust (D3). The SQLite alias resolves (probe) |
| **E6** | No `WorkerStore`/`WorkerHost` method | B-3: those traits are "exactly their call sites", and the engine never reads counts |
| **E7** | `CASES` entry on `HTUI_ANA_2` + `HTUI_CLEAN_1`, appends per step under `StepFence::Unleased`, and the payloads pin missing, `null` and numeric kinds plus a kind-bearing `tool_result` | B-4, B-2. The `tool_result` carries `tool_kind` so a kind-blind count fails |
| **E8** | The `serve` arm sits on the ordinary read path, next to `Runs` | D2/D4: offline the mirror answers, and `Unreachable` triggers `go_offline`, unlike `RelayView`'s writer-only read |
| **E9** | `StoreReply::ToolCalls` carries an unboxed `Vec` | 24 bytes. No `large_enum_variant` |
| **E10** | `StepNode` keeps its rows, and `chips` fits them to the drawn interior width | B-5: D6's 18 cells at zoom 1, and honest `+N` at every zoom |
| **E11** | Only four labels differ from the wire value (`delete→del`, `search→find`, `execute→exec`, `switch_mode→mode`). Ties go by label bytes. Fitting stops at the first chip that doesn't fit | D6's vocabulary with no identity arms. The visible chips are always the largest ones, so no smaller chip jumps ahead |
| **E12** | `+N` room is reserved per chip as `cell_width(" +{left}")` when kinds remain, and the last chip reserves nothing | D6 "with room for it reserved". The reservation made by chip *i−1* is exactly the `+N` that chip *i*'s failure writes |
| **E13** | `by_step` lives in `execution_graph.rs`, `pub(super)` | It's the graph's input shape, and it's unit-tested beside `chips` |
| **E14** | The map survives list ↔ flow toggles for one item. Only an item change clears it, and `v` re-asks | D7 ("an item change clears it"). A toggle shows the last chips for one frame, then fresh ones |
| **E15** | In flow, `ToolCalls` is asked **last** after a `Runs` reply (`RunStream`?, `RunActions`, `RelayView`, `ToolCalls`) | The list-view request lists stay byte-identical (B-9) |
| **E16** | The whole chip line (glyph included) is `theme.dim`, even on a node whose text is `base` | D6: secondary to the head and phase |
| **E17** | Three T1 commits (red, Mem green, Pg + mirror green), two each for T2 and T3, and one docs commit | Memory: implementers commit incrementally. H-2 puts the `.sqlx` entry with its macro, and H-3 keeps red `todo!()`s off live paths |

## 7. Close-out gate (plan § Validation, on the real tree)

```bash
df -h .                                                     # target/ growth (memory: disk pressure)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-store --all-features --test pg_conformance --test cache -- --test-threads=1 --nocapture 2>&1 \
  | grep -E "tool_call_counts|skip|test result"             # the case ran on Pg (not skipped)
cargo test -p htui --all-features -- --test-threads=1      # all-features, else tests/*.rs run 0 tests
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
git ls-files crates/htui-store/.sqlx | wc -l                # 319
git ls-files crates/htui/tests/snapshots | wc -l            # 135
cargo insta pending-snapshots                                # none
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
