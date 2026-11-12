# Blueprint: MOD-141 M2 - Splits, palette, deadline and the graph header

**Plan**: `.claude/plans/mod-141-m2-splits-palette-header.plan.md` (M-1..M-4 settled by the maintainer,
D1-D14, tasks T1-T9).
**PRD**: `.claude/prds/mod-141-step-graph-editor.prd.md`, milestone 2.
**Branch**: `hr/MOD-141`. Wave-1 worktrees branch from the last T1 commit.

Line numbers are `hr/MOD-141` at `d81465fa`. Where a line has moved, the named symbol wins.

---

## Plan corrections

Each one was checked against the code. The blueprint below builds the corrected version. None
reopens M-1..M-4.

| # | Plan claim | What the code says | Correction |
|---|---|---|---|
| C1 | Risk table: "about 50 literals across six crates" | Only **four** `StepGraphPhase { .. }` literals name every field: `seed.rs:260`, `store/conformance.rs:3357` (`new_phase`), `store/graph_rules.rs:698` (test helper `phase`), `htui/src/ui/graph_editor/edge_form.rs:661` (test `body_with`). Every other literal spreads `..` (search `planner_model:` finds only those four). The other compile breaks are the five `sqlx::query_as!(StepGraphPhase, ..)` macros, which must select the three columns. `fixtures.rs` builds phases through `seed::phase_row` and needs no change. | T1's inventory below. |
| C2 | D2 / risk table: "any agent-delete guard that counts phase agent columns"; "a conformance case for deleting an agent named as a judge, run on both stores" | Neither store has an agent delete. `WriteStore` has `upsert_agent` only (`traits.rs:501`); `MemStore` never removes from `agents`; Postgres deletes agents only in `load_demo` (`pg/demo.rs:157-164`, before any phase row is inserted) and in the SQLite mirror (`cache/refresh.rs:495`, no phase rows there). `judge_agent_id UUID REFERENCES agent(id)` (`0003:17`, no `ON DELETE`) would refuse such a delete, but no writer reaches it. | No guard and no delete case. What does need the judge: `MemStore::create_phase`'s agent loop (`mem.rs:4032-4047`), Postgres `phase_reference_refusal` (`pg/write.rs:1389-1450`, a pre-read, so a new `.sqlx` entry) and `phase_fk_refused` (`pg/write.rs:1208-1252`), which must map the judge's foreign key. 0003 left that key unnamed, so Postgres calls it `step_graph_phase_judge_agent_id_fkey`; T1 pins the name with a `pg_constraint` test. |
| C3 | D1: the Postgres phase queries are `phase_row`, `phase_rows_on`, `insert_phase`, `update_kept_phase`, `update_phase` | `update_phase` holds **two** `query_as!` macros (the `SELECT .. FOR UPDATE`, `pg/write.rs:4232-4271`, and the `UPDATE .. RETURNING`, `:4313-4361`). `load_demo` (`pg/demo.rs:248-289`) claims "every column `StepGraphPhase` carries", and `pg_criteria.rs:13194` `load_demo_keeps_every_graph_and_phase_column` pins that. `update_phase`'s `written` row spreads `..current` and needs no change. `seed_project`'s insert (`pg/write.rs:10305-10334`) writes only the 16 seed columns and the three stay `NULL`: no change. | Eight `.sqlx` entries move (list in T1). `load_demo` carries the three. The test helper `pg_criteria.rs:12747` `racer_copies_phase` (a runtime `sqlx::query`, no `.sqlx`) copies them too, so its "every other column copied" stays true. |
| C4 | T1 conformance: "an override clone keeps the deadline and the judge" | `override_graph` is htui-orch (`graph.rs:1132`). A store conformance case in htui-core cannot call it. | A `graph.rs` unit test in T1: `override_clone_keeps_the_deadline_and_the_judge`. |
| C5 | T1: "Update the conformance count pins this moves" | The pins are `crates/htui-core/tests/mem_store.rs:37` (`274`) and `crates/htui-store/tests/pg_conformance.rs:58` (`EXPECTED_CASES = 274`) plus the message at `:65`. | 274 → **277** (three new cases). |
| C6 | T1 Validate: `cargo sqlx prepare --workspace --check`; `cargo test -p htui-core --all-features --lib store::` | `--workspace` writes a second `.sqlx` at the root that nothing reads (`docs/hr-sandbox.md` "Changing SQL queries in a run"; memory). The conformance cases run from `tests/mem_store.rs`, not the lib. | Run `prepare` inside `crates/htui-store` against a migrated scratch database (T1 procedure). Add `--test mem_store`. |
| C7 | T7 and T8 file sets omit `ui/tabs/settings/kinds.rs` | `kinds.rs:2108` `a_write_in_flight_reads_in_words` loops over `catalogue::REQUEST_NAMES` and needs a phrase in `doing` (`:1911-1928`) for every name. Kinds hosts the editor: `on_editor_reply` (`:737-781`) forwards only `Agents`, `Templates` and their `Failed`, so the new reply would never reach the editor. `on_reply`'s `REQUEST_NAMES.contains` arm (`:1556`) would treat a failed settings read as a Kinds write refusal. The tests also pin the editor's two reads in `tests/kinds.rs:3369, 3406, 3424, 3469` and `tests/graph_editor.rs:485-488`. | T7 adds `doing`. T8 adds the routing and updates those test pins. Both are serial in wave 2. |
| C8 | D6/T8: "`Lookups` gains its settings" | T5's and T6's tests build `Lookups { personas, agents, templates }` as full literals (`node_form.rs:776-803`). A new field breaks them in T8. | The settings live on `GraphEditor` (`settings: Option<EditorSettings>`), not on `Lookups`. The form receives `compete: bool` and the notes receive `max_fan_out: Option<u32>`, so neither needs `Lookups` to carry them. |
| C9 | D11: `notes(node, max_fan_out)` | `max_fan_out` is unknown until the settings read answers. | `notes(node: &NodeBody, max_fan_out: Option<u32>) -> Vec<String>`. `None` skips the two checks that need it. |
| C10 | D11: "the `NotRunnable` texts ... `RunFailure`'s text" | Resolve raises `ResolveError::NotYetRunnable { node, reason }`, shown as ``phase `{node}` cannot run yet: {reason}`` (`graph.rs:289-296`). It also raises `DivideRefused`, shown as `{reason}`, and `FanOutCap` (`:236-245`). | Each note is the Display of the `ResolveError` resolve would raise, built from the `NotRunnable`/`RunFailure` variant. That is "resolve's own sentence" (M-4), and the variant's Display is inside it. |
| C11 | D11 lists the checks "in resolve's order" | `admission_refusal` (`divide.rs:177-249`) returns its **first** refusal only. | `notes` lists every check that fails, in admission's order. Each sentence is the one resolve would give if that check failed first. |
| C12 | D10/T8: "The palette path in `node_write`" | `node_write` (`mod.rs:1067-1084`) only calls `model::add_after` / `model::replace_node`. | T6 puts both palette rules inside those two functions. `node_write` needs no change; T8 only verifies it. |
| C13 | M-3 drops M1's "undo loses deadline and judge" limit | The delete question still says it (`mod.rs:176-183`, pinned by `mod.rs:2058-2062`). | T8 drops "deadline or judge" from the question and its test. |
| C14 | T5's file set: `node_form.rs`, `part_form.rs`, their snapshots, the `NodeForm` call sites | The step form is drawn in `tests/snapshots/kinds__graph_editor_step.snap` and in the `mod.rs` snapshot `editor_node_form`. `mod.rs:2171` (test `every_mode_names_its_stack`) also calls `NodeForm::new`. | T5 owns those two snapshots and that call site too. They are disjoint from every other wave-1 lane. |
| C15 | T1: a new deadline row anywhere in the form | `tests/graph_editor.rs:703-705` drives the M1 form by row index (`at_row(14, ..)` is the second candidate). | Every M2 row goes **after** the candidate rows, so M1's 13 fixed rows and the candidate indices do not move. |

Plan claims re-verified and true: `StepGraphPhase` at `kind.rs:501-571`; 0003's columns and CHECKs
(`0003:17-22`); `phase_row_refusal` at `graph_rules.rs:29-57`; `body_agent_refs` at `:276-303`; the
F14 doc at `traits.rs:1100-1104`; resolve's chains at `graph.rs:1670-1699`; `topology` serialising
`SnapshotPhase` only; `override_graph` spreading `..node.phase` (`:1268-1272`); `SettingKey::ALL` 13
and App rung 11; `REQUEST_NAMES` 12 (`catalogue.rs:440`); the node-form stack's four list acts
(`views.rs:431-446`); hint labels as editor constants (`mod.rs:216-230`); `owned_path_refusal`
private (`divide.rs:253`); `NotRunnable` and `RunFailure` public; `htui` depends on `htui-orch`
(`crates/htui/Cargo.toml:31`).

---

## Blueprint decisions (the plan left them open)

- **B1 `RunBounds` lives in `htui-orch/src/graph.rs`, beside `run_bounds`.** The code defaults it
  reads (`DEFAULT_MAX_FAN_OUT`, `DEFAULT_MAX_AGENTS_PER_RUN`, `graph.rs:41-46`) and the
  "positive or silent" reader `app_u32` (`:1588-1591`) are private to that file. Keeping them in
  one place means `resolve` and the editor cannot disagree. `htui` already depends on `htui-orch`.
- **B2 `EditorSettings` lives in `htui/src/catalogue.rs`.** `catalogue::serve` builds it, the same
  way it builds `CatalogueSnapshot`, and `store_worker.rs` already imports from there.
- **B3 Step-form row order** (C15): M1's 13 fixed rows, then the candidates or their placeholder,
  then `deadline`, `palette`, `split`, then the split's own rows. The form's split-dependent
  rows are built from a row list (T5), never from fixed indices past row 12.
- **B4 New act**: `Act::GraphEditorOpenPart`, `[graph_editor] open_part`, default `alt-e`, help
  `edit part`. `alt-e` is bound nowhere: the only `alt-` defaults are `alt-n`, `alt-d`, `alt-up`,
  `alt-down` (graph_editor) and `alt-1`, `alt-2`, `alt-left`, `alt-right` (layout and history,
  `catalogue.rs:2372-2399`). No new context, so `Context::ALL` and `context_position` do not move.
- **B5 Warning marker**: a noted node's canvas title is `! {name}`. Each note is drawn under the
  step form's rows as a wrapped `theme.error` line, with `! ` before its first line. While the
  form parses, the notes are computed live from `form.build(base, lookups)`; otherwise from the
  base.
- **B6 Sentence names** keep at most three underscores. `conformance.rs:30211`
  `every_cross_referenced_test_name_exists` treats any inline-code snake_case span with four or
  more underscores in that file as a test name that must exist.

---

## Shared conventions (every task)

- **Wave allow**: a wave-1 item that nothing calls until T8 carries
  `#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]` on the **entry point** clippy flags.
  rustc treats an `allow(dead_code)` item as live, so whatever it calls is not flagged. T8
  removes every one: `git grep -n 'MOD-141 M2 T8 wires it'` must come back empty.
- **Errors in words**: refusals are `pub(crate)` constants or `fn`s returning `String`. There is no
  panic or `unwrap` on data. Hand-written `Debug` prints ids, focus and counts, never typed text
  (H-7).
- **Keys**: no chord appears in a module other than `keys/`. Words that name a key take its label
  from `Keys::label` (R-TUI-10).
- **Tests**: TDD. Write the listed tests first, watch them fail, then implement. Every command
  runs with `--test-threads=1`. `crates/htui/tests/*.rs` needs `--features testkit`, or it runs
  0 tests and reports ok.
- **Lint gate** before every commit, for each crate touched:
  ```bash
  cargo fmt --all -- --check
  cargo clippy -p <crate> --all-targets -- -D warnings
  cargo clippy -p <crate> --all-targets --all-features -- -D warnings
  ```
- **Snapshots**: review every changed `.snap`; never accept blindly. Unit snapshots are
  `crates/htui/src/ui/graph_editor/snapshots/htui__ui__graph_editor__<module>__tests__<name>.snap`.
- **Worktrees** (wave 1):
  - Check `df -h .` first (2.6T free at blueprint time).
  - For N in 2..6: `git worktree add target/wt/t<N> -b mod-141-m2-t<N> hr/MOD-141`.
  - Commit incrementally and never stash.
  - Merge serially into `hr/MOD-141` in the order T2, T3, T4, T5, T6. After each merge, re-run
    that task's Validate on the merged tree.
  - `git worktree remove` before `git branch -d`.
  - Gortex edits land in the primary checkout (memory), so inside a worktree edit with the file
    tools on the worktree path.

---

## T1 - Phase model widening (D1, D2), then the two module stubs (wave 0, serial)

**File set**:
- `crates/htui-core/src/model/kind.rs`, `seed.rs`
- `crates/htui-core/src/store/{graph_rules,traits,mod,mem,conformance}.rs`
- `crates/htui-core/tests/mem_store.rs`
- `crates/htui-store/src/pg/{read,write,demo}.rs`, `crates/htui-store/.sqlx/*`
- `crates/htui-store/tests/{pg_conformance,pg_criteria}.rs`
- `crates/htui-orch/src/{divide,graph}.rs` (`graph.rs`: one test only)
- `crates/htui/src/ui/graph_editor/{mod,edge_form}.rs`
- new `crates/htui/src/ui/graph_editor/{notes,part_form}.rs`

### 1.1 Model (`kind.rs`)

Insert after `pub max_parts: Option<i32>,` (`kind.rs:568`), before `updated_at`:
```rust
    /// `step_graph_phase.deadline_seconds` (0003, MOD-141 M2 D1): one step attempt's wall clock;
    /// `None` follows the project's, then `app_setting`'s, then two hours. At least 1.
    #[serde(default)]
    pub deadline_seconds: Option<i32>,
    /// `step_graph_phase.judge_agent_id` (0003, MOD-141 M2 D1): the R-ORCH-7 judge of a fanned-out
    /// node; `None` follows the project's judge, else a person selects.
    #[serde(default)]
    pub judge_agent_id: Option<AgentId>,
    /// `step_graph_phase.judge_model` (0003): the judge's model; only with a judge agent
    /// (`ck_phase_judge_model`).
    #[serde(default)]
    pub judge_model: Option<String>,
```
In the test `a_v1_phase_json_decodes_with_v2_defaults` (`kind.rs:1087-1119`), add
`assert_eq!(phase.deadline_seconds, None)`, `assert_eq!(phase.judge_agent_id, None)` and
`assert_eq!(phase.judge_model, None)`.

### 1.2 Literal inventory

**Full literals that must add the three fields:**

| Path:line | What | Add |
|---|---|---|
| `crates/htui-core/src/seed.rs:260-293` `phase_row` | production | `deadline_seconds: None, judge_agent_id: None, judge_model: None,` after `max_parts: None,` (`:291`); extend the doc (`:247-252`) with "and MOD-141 M2's three: no deadline, no judge" |
| `crates/htui-core/src/store/conformance.rs:3356-3386` `new_phase` | test helper | same, after `max_parts: None,` (`:3384`) |
| `crates/htui-core/src/store/graph_rules.rs:697-727` `phase` (tests) | test helper | same, after `max_parts: None,` (`:725`) |
| `crates/htui/src/ui/graph_editor/edge_form.rs:661-690` `body_with` (tests) | test helper | same, after `max_parts: None,` (`:688`) |

Also in `seed.rs`'s test `rows_carry_the_frozen_defaults`: after the "no divide columns"
assertion (`:747-767`), assert
`(phase.deadline_seconds, phase.judge_agent_id, phase.judge_model.as_deref()) == (None, None, None)`
with the message "no deadline and no judge".

**`..`-spread literals (no change)**:
- htui-core: `conformance.rs:20076, 20096, 20108-20160, 20579, 23540, 25740, 25805-25889`; `mem.rs:16918`.
- htui-orch: `conformance.rs:1903, 1975, 1993`; `engine.rs:10736`; `graph.rs:1268, 3482, 4289`; `tests/fixtures.rs:113, 120`; `tests/gix_isolator.rs:365`.
- htui-store: `pg/write.rs:4308` (`written`); `tests/pg_criteria.rs:12401, 13006, 13013`.
- htui-worker: `tests/auto_queue.rs:845`.
- htui: `ui/graph_editor/canvas.rs:1176`, `ui/graph_editor/mod.rs:1443`.

**`query_as!(StepGraphPhase, ..)` (compile breaks)**: `pg/read.rs:3010` `phase_row`,
`pg/read.rs:3540` `phase_rows_on`, `pg/write.rs:1459` `insert_phase` (`RETURNING`),
`pg/write.rs:4232` and `:4313` in `update_phase`.

**Raw phase inserts**:
- `pg/demo.rs:248-289` `load_demo`: **change** (C3).
- `pg/write.rs:10305` `seed_project`: no change.
- `tests/migrations.rs:2240` `insert_phase_row` (v19 columns): no change.
- `tests/pg_criteria.rs:12747` `racer_copies_phase`: **change** (C3).

### 1.3 Postgres queries (`htui-store`)

Every select list gains these three lines between `max_parts,` and `updated_at`:
```
                   deadline_seconds,
                   judge_agent_id       AS "judge_agent_id: AgentId",
                   judge_model,
```
Apply them in:
- `pg/read.rs` `phase_row` (`:3039-3040`)
- `pg/read.rs` `phase_rows_on` (`:3569-3570`)
- `pg/write.rs` `insert_phase` `RETURNING` (`:1497-1498`)
- `update_phase` `SELECT .. FOR UPDATE` (`:4261-4262`)
- `update_phase` `UPDATE .. RETURNING` (`:4349-4350`)

Writes:
- `insert_phase` (`pg/write.rs:1452-1532`):
  - Column list: `.., integrate_model, max_parts, deadline_seconds, judge_agent_id, judge_model)`.
  - `VALUES ($1 .. $30)`.
  - Binds after `phase.max_parts,`: `phase.deadline_seconds, phase.judge_agent_id.map(AgentId::as_uuid), phase.judge_model.as_deref(),`.
- `update_kept_phase` (`:1590-1640`):
  - `.., max_parts = $27, deadline_seconds = $28, judge_agent_id = $29, judge_model = $30 WHERE id = $1 AND graph_id = $2`, with the same three binds.
  - Doc becomes: "`replace_graph`'s in-place write of a kept node: every column [`StepGraphPhase`] carries (MOD-141 M2 D1). The driver's error comes back raw for the caller's [`phase_fk_refused`]."
- `pg/demo.rs` `load_demo` (`:248-289`):
  - Add `deadline_seconds, judge_agent_id, judge_model` after `max_parts`.
  - `VALUES` `$1 .. $31`.
  - Binds `row.deadline_seconds, row.judge_agent_id.map(AgentId::as_uuid), row.judge_model.as_deref()`.
  - Keep the comment ("Every column `StepGraphPhase` carries ...") true by adding "and MOD-141 M2's three".
- `phase_reference_refusal` (`:1383-1450`):
  - Add a sixth select column:
    ```sql
               ($6::uuid IS NULL OR EXISTS (SELECT 1 FROM agent WHERE id = $6))
                   AS "judge_agent!"
    ```
  - Bind `phase.judge_agent_id.map(AgentId::as_uuid)`.
  - The `agents` array gets a third entry `("step_graph_phase.judge_agent_id", found.judge_agent, phase.judge_agent_id)` after `integrate_agent_id`.
  - Doc: "six foreign keys ... 0003's judge key is the oldest, so its trigger is not in the order `MemStore` checks; personas first, then planner, integrate and judge agents".
- `phase_fk_refused` (`:1201-1252`):
  - The array becomes `[(&str, &str, Option<Uuid>, &str); 6]`.
  - Append `("step_graph_phase_judge_agent_id_fkey", "step_graph_phase.judge_agent_id", phase.judge_agent_id.map(AgentId::as_uuid), "agent")`.
  - Doc "five" → "six (0003 left the judge's unnamed: Postgres's own `<table>_<column>_fkey`)".
- Comments:
  - `pg/write.rs:4627-4628` step 10 → "Kept nodes in place, new ones through the shared insert; both write every column (MOD-141 M2 D1)".
  - `create_phase` doc (`:4093-4097`): add `judge_agent_id` to the list of columns checked, and "six FKs".

`tests/pg_criteria.rs`:
- `racer_copies_phase` (`:12747-12774`): add `deadline_seconds, judge_agent_id, judge_model` to both the insert list and the `SELECT` list.
- `load_demo_keeps_every_graph_and_phase_column` (`:13194-13245`): after `phase.max_parts = Some(3);` add `phase.deadline_seconds = Some(900); phase.judge_agent_id = Some(ids::AGENT_AGY); phase.judge_model = Some("opus".to_owned());`.
- New test, after that one:
  ```rust
  /// MOD-141 M2 D2: `phase_fk_refused` maps the judge's foreign key by the name Postgres gave
  /// it (0003 declared `REFERENCES agent(id)` with no `CONSTRAINT`); a renamed key fails here.
  #[tokio::test(flavor = "multi_thread")]
  async fn the_judge_agent_foreign_key_has_the_name_the_writer_maps() {
      let Some(db) = common::fresh_db().await else { return; };
      let name: String = sqlx::query_scalar(
          "SELECT conname::text FROM pg_constraint \
            WHERE conrelid = 'step_graph_phase'::regclass AND contype = 'f' \
              AND conkey = ARRAY[(SELECT attnum FROM pg_attribute \
                                   WHERE attrelid = 'step_graph_phase'::regclass \
                                     AND attname = 'judge_agent_id')]",
      )
      .fetch_one(&db.pool)
      .await
      .expect("one foreign key on judge_agent_id");
      assert_eq!(name, "step_graph_phase_judge_agent_id_fkey");
      db.drop_db().await;
  }
  ```
  This is a runtime query, so it adds no `.sqlx` entry.

### 1.4 Rules and sentences (`htui-core`)

`traits.rs`: insert before `parts_off_divide`'s doc (`:3568`), after `phase_token_budget_refusal`:
```rust
/// 0003's `ck_phase_judge_model` (MOD-141 M2 D2), in words.
#[must_use]
pub fn judge_model_without_agent(phase: &str) -> String {
    format!(
        "phase `{phase}` has a judge_model and no judge_agent_id; a judge model needs its judge \
         agent"
    )
}

/// 0003's `step_graph_phase.deadline_seconds` CHECK (`deadline_seconds > 0`, MOD-141 M2 D2), in
/// words.
#[must_use]
pub fn deadline_below_one(phase: &str, seconds: i32) -> String {
    format!("phase `{phase}` has deadline_seconds {seconds}; a step deadline is at least 1 second")
}
```
`store/mod.rs` `pub use traits::{..}` (`:22-55`) gains `deadline_below_one` (between
`close_out_needs_a_summary` and `divide_columns_off_divide`) and `judge_model_without_agent`
(between `item_not_in_project` and `kind_not_narrowable`).

`graph_rules.rs`:
- Import both sentences.
- `phase_row_refusal` (`:29-57`): replace the tail `phase.token_budget.and_then(phase_token_budget_refusal)` (`:56`) with:
  ```rust
      if let Some(refusal) = phase.token_budget.and_then(phase_token_budget_refusal) {
          return Some(refusal);
      }
      if phase.judge_model.is_some() && phase.judge_agent_id.is_none() {
          return Some(judge_model_without_agent(&phase.name));
      }
      phase
          .deadline_seconds
          .filter(|seconds| *seconds < 1)
          .map(|seconds| deadline_below_one(&phase.name, seconds))
  ```
  Doc (`:22-27`): append "then 0003's `ck_phase_judge_model` (a `judge_model` with no `judge_agent_id`) and its `deadline_seconds > 0` CHECK (MOD-141 M2 D2)".
- `body_agent_refs` (`:276-303`): the array becomes
  `[("step_graph_phase.planner_agent_id", ..), ("step_graph_phase.integrate_agent_id", ..), ("step_graph_phase.judge_agent_id", phase.judge_agent_id)]`.
  Doc (`:270-275`): "`planner_agent_id`, `integrate_agent_id`, `judge_agent_id`, its candidates' ...".
- `traits.rs` doc updates:
  - `create_phase` (`:1044-1052`): add `judge_agent_id` after `integrate_agent_id`, and "a `judge_model` without a judge agent, a `deadline_seconds` below 1" to the node-CHECK list.
  - `replace_graph` (`:1100-1104`): replace the F14 sentence with "Every column [`StepGraphPhase`] carries is written, on a kept node and on a new one alike (MOD-141 M2 D1)."

`mem.rs` `State::create_phase` (`:3990-4061`):
- The agent loop (`:4032-4047`) gets a third entry `("step_graph_phase.judge_agent_id", phase.judge_agent_id)`.
- Comment `:4010-4012` "the three personas, then the three agents".
- `replace_graph` and `update_phase` store the row whole: no change.

Unit tests (`graph_rules.rs` tests module):
- `judge_model_needs_a_judge_agent`: `phase(Split::Compete, 2)` with `judge_model = Some("m")` → `Some(judge_model_without_agent("implement"))`; adding `judge_agent_id = Some(AgentId::new())` → `None`. On `Split::None` the result is the same (the store ties the judge to no split).
- `deadline_is_at_least_one_second`: `Some(0)` and `Some(-5)` → `deadline_below_one("implement", n)`; `Some(1)` and `None` → `None`.
- `the_node_rules_fire_in_order` (`:836-869`): extend after its last assertion:
  - `row.retry_limit = 0; row.token_budget = Some(0); row.judge_model = Some("m"); row.deadline_seconds = Some(0)` → the token-budget sentence;
  - then `token_budget = None` → the judge sentence;
  - then `judge_model = None` → the deadline sentence.
- `body_refs_walk_personas_then_agents_in_body_order` (`:1440-1484`): `agents` grows to 5. Set `body.nodes[0].phase.judge_agent_id = Some(agents[4])`. Expect `("step_graph_phase.judge_agent_id", agents[4])` right after the integrate entry.

### 1.5 Conformance

Append to `CASES` after `"resolve_inputs_skips_part_documents"` (`conformance.rs:~382`):
```rust
    // MOD-141 M2: the phase's judge and deadline columns (plan D1, D2)
    "a_judge_model_needs_a_judge_agent",
    "a_step_deadline_is_at_least_one_second",
    "a_phase_naming_no_judge_agent_is_refused",
```
Add three boxed arms to `run_case_inline`, like the others. All new code goes in a block after
`create_phase_keeps_split_in_step_with_fan_out` (`:25799`).

Round trip: in `replace_graph_round_trips_every_node_field` (`:25690-25786`), the `full` closure
gains `row.deadline_seconds = Some(900); row.judge_agent_id = Some(ids::AGENT_CLAUDE); row.judge_model = Some("opus".to_owned());`.
This covers a kept node (`nodes[1]`) and a new one (`merge`). Doc: "every phase column, MOD-141 M2's
deadline and judge included".

**`a_judge_model_needs_a_judge_agent`** (doc: "MOD-141 M2 D2: 0003's `ck_phase_judge_model` on both writers, after the token budget; with its agent the pair is written."):
1. `create_phase(&StepGraphPhase { judge_model: Some("opus"), ..new_phase(ids::GRAPH_HTUI_FEAT, 9, "judged") })` → `constraint_exactly(.., &judge_model_without_agent("judged"), "create: a judge model alone")`.
2. The same with `token_budget: Some(0)` → `&phase_token_budget_refusal(0).expect(..)`, "the token budget first".
3. `let (graph, _) = scratch_graph(CASE, store, "judged", &["plan", "build"]).await; let body = body_now(..)`.
   - `nodes[1].phase.judge_model = Some("opus")` → replace refused with `judge_model_without_agent("build")` (a kept node).
   - Push `bare_node(new_phase(graph, 2, "merge"))` with a judge model → `..("merge")` (a new node).
   - `body_now == body`: nothing written.
4. `nodes[1]` with `judge_agent_id = Some(ids::AGENT_AGY)` and `judge_model = Some("opus")` → `applied`. The answer's `nodes[1].phase` carries both.

**`a_step_deadline_is_at_least_one_second`**:
1. `create_phase` with `deadline_seconds: Some(0)` → `deadline_below_one("late", 0)`.
2. `Some(-5)` → `(.., -5)`.
3. `judge_model: Some("m"), deadline_seconds: Some(0)` → `judge_model_without_agent("late")`, "the judge rule first".
4. `Some(1)` applies; `phases()` reads back `Some(1)`.
5. `replace_graph` on a scratch graph with `deadline_seconds = Some(0)` on a kept node → refused; with `Some(7200)` → applied and read back.

**`a_phase_naming_no_judge_agent_is_refused`**:
1. `let ghost = AgentId::new();`
2. `create_phase` with `judge_agent_id: Some(ghost)` (split none, fan-out 1) → `references_no_row("step_graph_phase.judge_agent_id", ghost, "agent")`.
3. A divide node with `integrate_agent_id: Some(ghost)` and `judge_agent_id: Some(AgentId::new())` → the integrate sentence (D2 position: after integrate).
4. `persona_id: Some(PersonaId::new())` plus a ghost judge → the persona sentence (personas first).
5. `replace_graph`, scratch graph: `nodes[0]` with a ghost judge and a ghost candidate (`candidate(plan, 0, other_ghost)`) → the judge sentence (the judge comes before the candidates).
6. Nothing written: `body_now == before` and `phases(ids::GRAPH_HTUI_FEAT)` unchanged.

Doc comments must not name `judge_model_without_agent` with more underscores (B6). These three
names are themselves defined in `conformance.rs`, so citing them is fine.

Count pins (C5):
- `crates/htui-core/tests/mem_store.rs:37`: `274` → `277`. Append to the message: ", and MOD-141 M2's three for the phase's judge and deadline columns (plan D1, D2)".
- `crates/htui-store/tests/pg_conformance.rs:58`: `EXPECTED_CASES = 277`. Add a doc line `/// and MOD-141 M2's three judge and deadline cases (plan D1, D2) make it 277.`. The `:65` message becomes "(277 since ... and MOD-141 M2's three judge and deadline cases)".

### 1.6 Override clone (`htui-orch/src/graph.rs`, tests module)

Add `override_clone_keeps_the_deadline_and_the_judge` after `override_clone_carries_phase_agents`
(`:3021-3090`):
1. `let store = MemStore::demo();`
2. `replace_feature(&store, |body| { let implement = body.nodes.iter_mut().find(|n| n.phase.name == "implement").unwrap(); implement.phase.deadline_seconds = Some(900); implement.phase.judge_agent_id = Some(ids::AGENT_AGY); implement.phase.judge_model = Some("opus".to_owned()); }).await;`
3. `override_graph(&store, &TestSource::claude(&store), &feat_1(&store).await)`.
4. The cloned `implement` (`store.phases(clone.id)`) has the same three values. The other clones have `None, None, None`.

### 1.7 `divide.rs` and the htui module stubs

`divide.rs:252-253`: `fn owned_path_refusal` → `#[must_use] pub fn owned_path_refusal`. Extend
the doc with "Public for the graph editor's warnings (MOD-141 M2 D11)".

`crates/htui/src/ui/graph_editor/mod.rs`:
- Module list (`:17-22`) becomes `canvas, edge_form, history, links, model, node_form, notes, part_form`, all `pub(crate)`.
- The `//!` bullet list (`:9-12`) gains:
  - `` - `notes`: what a run of one step would be refused with today, in resolve's own words (M2 D11). ``
  - `` - `part_form`: one default part of a divide step (M2 D9). ``

`notes.rs` (literal):
```rust
//! MOD-141 M2 D11 (M-4): what a run of one step would be refused with today, in
//! `htui_orch::graph::resolve`'s own sentences and order. A warning beside the form and a `!` on
//! the canvas, never a refused save. Pure: no keys, no store.
```
`part_form.rs` (literal):
```rust
//! MOD-141 M2 D9: the part form - one default part of a divide step: key, title, brief, owned
//! paths, acceptance, agent and model (none, or one of the step's own candidates) and persona.
//! Key-agnostic: the editor maps its acts onto these methods.
```

### 1.8 `.sqlx` procedure (inside the sandbox)

Run this after 1.3, before any offline build (`SQLX_OFFLINE = "true"` comes from
`.cargo/config.toml`). Run it again after any query edit.
```bash
psql -h localhost -p 5439 -U postgres -c "DROP DATABASE IF EXISTS htui_sqlx;" -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features    # never without the flags (memory)
cargo sqlx prepare --check                            # "potentially unused queries" warning is expected
cd ../..
```
Stale-entry check: the number of `.sqlx` files is unchanged
(`ls crates/htui-store/.sqlx | wc -l` before and after). `git status --porcelain
crates/htui-store/.sqlx` shows exactly **8 `D`** and **8 `??`**. The deleted hashes are:
- `eb8ed105` `phase_row`
- `431ea560` `phase_rows_on`
- `0d9ffaba` `insert_phase`
- `22ded809` `update_kept_phase`
- `a1aa5928` `update_phase` SELECT
- `816f2c71` `update_phase` UPDATE
- `899fe8ae` `load_demo` phase insert
- `e77a8a82` `phase_reference_refusal`

Any other `D` means a plain `prepare` ran. Recover with `git checkout -- crates/htui-store/.sqlx && git clean -fq crates/htui-store/.sqlx`, then re-run with the flags.

### 1.9 Validate
```bash
cargo check --workspace --all-targets --all-features
cargo test -p htui-core --all-features --lib -- --test-threads=1 store:: model::kind seed::
cargo test -p htui-core --all-features --test mem_store -- --test-threads=1
cargo test -p htui-store --test pg_conformance -- --test-threads=1
cargo test -p htui-store --test pg_criteria -- --test-threads=1 load_demo_keeps_every_graph_and_phase_column the_judge_agent_foreign_key racing
cargo test -p htui-orch --all-features --lib -- --test-threads=1 graph::tests::override_clone
cargo test -p htui --lib ui::graph_editor -- --test-threads=1
cargo clippy --workspace -- -D warnings && cargo clippy --workspace --all-targets --all-features -- -D warnings
```
`HTUI_TEST_DATABASE_URL` is set in the sandbox. A Postgres test that prints `skipped:` did not run.

### 1.10 Commits
1. `feat(MOD-141): M2 T1 StepGraphPhase carries deadline_seconds, judge_agent_id and judge_model`
   - Contents: model, the four literals, every Postgres query and `load_demo`, `.sqlx`, `racer_copies_phase`, the round-trip and `load_demo` test extensions, the kind/seed asserts.
2. `feat(MOD-141): M2 T1 the judge-model and deadline CHECKs, judge agent references`
   - Contents: sentences, `phase_row_refusal`, `body_agent_refs`, `MemStore::create_phase`, `phase_reference_refusal`, `phase_fk_refused`, `.sqlx`, the unit tests, the three conformance cases and pins, the FK-name test.
3. `test(MOD-141): M2 T1 an override clone keeps the deadline and the judge`
4. `chore(MOD-141): M2 T1 owned_path_refusal public; notes and part_form stubs`

---

## T2 - Resolve reads the phase rung; run bounds (D3, D4) (wave 1)

**File set**: `crates/htui-orch/src/graph.rs`.

### 2.1 `RunBounds` and `run_bounds`

Add `RuntimeChanges` to the `htui_core::model::{..}` import (`graph.rs:17-24`). After `Resolved`
(`:150-159`):
```rust
/// MOD-141 M2 D4: the run limits [`resolve`] freezes into [`SnapshotSettings`], over the same
/// `app_setting` keys and code defaults; the graph editor's header shows exactly these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunBounds {
    /// `app_setting.max_fan_out`, else 4.
    pub max_fan_out: u32,
    /// `app_setting.max_agents_per_run`, else 8.
    pub max_agents_per_run: u32,
    /// `app_setting.max_parallel_sessions`, else [`SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS`].
    pub max_parallel_sessions: u32,
    /// `app_setting.max_plan_revisions`, else [`SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS`].
    pub max_plan_revisions: u32,
    /// `app_setting.max_child_runs`, else [`SnapshotSettings::DEFAULT_MAX_CHILD_RUNS`].
    pub max_child_runs: u32,
    /// `app_setting.runtime_changes`, else `ask`.
    pub runtime_changes: RuntimeChanges,
}
```
After `app_i32` (`:1593-1596`):
```rust
/// MOD-141 M2 D4: [`RunBounds`] over `app`, each key under [`app_u32`]'s "positive or silent"
/// rule; `runtime_changes` a known word or `ask`.
#[must_use]
pub fn run_bounds(app: &BTreeMap<String, Value>) -> RunBounds {
    RunBounds {
        max_fan_out: app_u32(app, "max_fan_out").unwrap_or(DEFAULT_MAX_FAN_OUT),
        max_agents_per_run: app_u32(app, "max_agents_per_run").unwrap_or(DEFAULT_MAX_AGENTS_PER_RUN),
        max_parallel_sessions: app_u32(app, "max_parallel_sessions")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_PARALLEL_SESSIONS),
        max_plan_revisions: app_u32(app, "max_plan_revisions")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_PLAN_REVISIONS),
        max_child_runs: app_u32(app, "max_child_runs")
            .unwrap_or(SnapshotSettings::DEFAULT_MAX_CHILD_RUNS),
        runtime_changes: app
            .get("runtime_changes")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or_default(),
    }
}
```
In `resolve_or_refused`, `let settings = SnapshotSettings { .. }` (`:621-638`) becomes:
```rust
    let bounds = run_bounds(app);
    let settings = SnapshotSettings {
        default_isolation: settings.default_isolation,
        per_token_cap_run: caps.run_micros,
        per_token_cap_batch: caps.batch_micros,
        max_fan_out: bounds.max_fan_out,
        max_agents_per_run: bounds.max_agents_per_run,
        // MOD-138 D13: the bounds, frozen over their code defaults like the caps above.
        max_parallel_sessions: bounds.max_parallel_sessions,
        max_plan_revisions: bounds.max_plan_revisions,
        max_child_runs: bounds.max_child_runs,
        runtime_changes: bounds.runtime_changes,
    };
```

### 2.2 The phase rung (`snapshot_phase`, `:1608-1734`)

Replace `deadline_seconds` (comment `:1670-1672` and expression `:1673-1678`) with:
```rust
        // MOD-141 M2 D3: the phase rung first (a positive column), then the project, the app and
        // two hours. A NULL column resolves exactly as before, so no digest moves.
        deadline_seconds: Some(
            phase
                .deadline_seconds
                .filter(|seconds| *seconds > 0)
                .and_then(|seconds| u32::try_from(seconds).ok())
                .or(settings.step_deadline_seconds)
                .or_else(|| app_u32(app, "step_deadline_seconds"))
                .unwrap_or(DEFAULT_DEADLINE_SECONDS),
        ),
```
Replace `judge` (comment `:1689-1690` and match `:1691-1699`) with:
```rust
        // MOD-141 M2 D3: the phase's judge with its own model, else the project's with none;
        // `None` means human selection (§4.5).
        judge: match phase
            .judge_agent_id
            .map(|id| (id, phase.judge_model.clone()))
            .or_else(|| settings.judge_agent_id.map(|id| (id, None)))
        {
            None => None,
            Some((id, model)) => Some(SnapshotJudge {
                agent_id: id,
                agent_name: agent_name(source, &phase.name, id).await?,
                model,
                template: judge_template(source, project).await?,
            }),
        },
```
Test comments that cite H-14:
- `field_chains_walk_phase_project_app` (`:2378-2380`, `:2388`) → "the phase carries none, and neither does the project nor `app_setting`" / "no phase judge and no project judge".
- `the_project_rung_answers_when_the_phase_does_not` (`:2414`) message → "the project rung carries no model".

### 2.3 Tests (write first; `graph.rs` tests module)

- `the_phase_deadline_answers_before_the_project`:
  - `store_with_settings(json!({"step_deadline_seconds": 600}))`, then `replace_feature` sets `prd`'s `deadline_seconds = Some(90)`.
  - Resolve with `resolve_feat`: `phases[0].deadline_seconds == Some(90)`. Every other phase holds `None` and reads the project's `Some(600)`.
- `a_phase_judge_carries_its_model_and_beats_the_project_judge`:
  - `store_with_settings(json!({"judge_agent_id": ids::AGENT_AGY}))`.
  - `replace_feature` sets `implement`'s `judge_agent_id = Some(ids::AGENT_CLAUDE)` and `judge_model = Some("opus")`.
  - Resolve. `implement.judge`: agent CLAUDE, name `"claude"`, `model == Some("opus")`, `template == Some(SnapshotTemplate { name: "judge", version: 1 })`. `prd.judge`: agent AGY, `model == None`.
- `null_phase_columns_resolve_as_before`: on `MemStore::demo()`, `resolve_feat(..).snapshot.topology == FEATURE_TOPOLOGY_V2` (the existing pin constant), every `deadline_seconds == Some(7200)` and every `judge == None`.
- `run_bounds_are_the_code_defaults_over_no_row`: `run_bounds(&BTreeMap::new()) == RunBounds { max_fan_out: 4, max_agents_per_run: 8, max_parallel_sessions: 4, max_plan_revisions: 3, max_child_runs: 8, runtime_changes: RuntimeChanges::Ask }`.
- `run_bounds_read_each_key_and_fall_through_on_silence`:
  - `{max_fan_out: 2, max_agents_per_run: 5, max_parallel_sessions: 7, max_plan_revisions: 1, max_child_runs: 3, runtime_changes: "off"}` → those values.
  - `{max_fan_out: 0, max_agents_per_run: "6", max_parallel_sessions: -1, runtime_changes: "maybe"}` → the defaults.
- `resolve_freezes_exactly_run_bounds`: for the first map above (with `max_agents_per_run` ≥ the feature graph's planned agents, e.g. 5), `resolve(..).snapshot.settings`'s six fields equal `run_bounds(&app)`'s.

Existing tests that must stay green unchanged: `a_snapshot_freezes_the_bounds_and_the_source`
(`:5139`), `the_app_rung_answers_when_the_project_does_not` (`:2471`), `field_chains_walk_phase_project_app`.

### 2.4 Validate
```bash
cargo test -p htui-orch --all-features --lib graph:: -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1      # once, before merge: engine and fixtures read the snapshot
```

### 2.5 Commits
1. `feat(MOD-141): M2 T2 resolve reads the phase deadline and judge first`
2. `refactor(MOD-141): M2 T2 run_bounds, the limits resolve freezes`

---

## T3 - Compete switch (D5) (wave 1)

**File set**:
- `crates/htui-core/src/prompt/settings.rs`
- `crates/htui/src/prompt_settings.rs` (docs only)
- `crates/htui/src/ui/tabs/settings/prompt.rs`
- `crates/htui/tests/prompt_settings.rs`
- `crates/htui/tests/snapshots/prompt_settings__*.snap`
- any other `.snap` the full insta run shows moving **only** because the Prompt section's App group grew a row (review each)

### 3.1 Registry (`prompt/settings.rs`)

- After `MEMORY_CAPTURE_DEFAULT` (`:53-56`):
  ```rust
  /// MOD-141 M2 D5: `compete_fan_out`'s compiled default, off. An App key with no seeded row, like
  /// `memory_capture`; only the graph editor reads it (`R-LATER-13`).
  pub const COMPETE_FAN_OUT_DEFAULT: bool = false;
  ```
- `SettingKey` (`:172-200`): insert `CompeteFanOut = 2` after `BoxHostname = 1`. Its doc: "`compete_fan_out`, MOD-141 M2 D5: whether the step-graph editor offers the compete split; App only, off by default."
  Renumber (key byte order, `c` < `e`):

  | Variant | Before | After |
  |---|---|---|
  | `ExcerptFileLineCap` | 2 | 3 |
  | `ExcerptHeadLines` | 3 | 4 |
  | `ExcerptMaxFileBytes` | 4 | 5 |
  | `ExcerptMaxFiles` | 5 | 6 |
  | `ExcerptMaxScanFiles` | 6 | 7 |
  | `ExcerptProviderDeadlineMs` | 7 | 8 |
  | `MaxSkillTokens` | 8 | 9 |
  | `MemoryCapture` | 9 | 10 |
  | `PromptReserveFraction` | 10 | 11 |
  | `UpstreamHops` | 11 | 12 |
  | `TokenBudget` | 12 | 13 |

  `AllowEnvFiles` (0) and `BoxHostname` (1) do not move.
- `ALL: [Self; 14]`: insert `Self::CompeteFanOut` third. `SPECS: [SettingSpec; 14]`: insert at index 2:
  ```rust
    SettingSpec {
        key: "compete_fan_out",
        project_key: None,
        kind: SettingKind::Boolean,
        min: 0,
        max: 1,
        rungs: Rungs::APP,
        not_above: None,
        unit: "on/off",
        doc: "Offer the compete split (several agents, one winner) in the step-graph editor.",
    },
  ```
- `value_of` (`:113-131`): `SettingKey::CompeteFanOut => Value::Bool(COMPETE_FAN_OUT_DEFAULT),`. `integer` (`:139-161`): `SettingKey::CompeteFanOut => i64::from(COMPETE_FAN_OUT_DEFAULT),`.
- After `resolve_memory_capture` (`:819-828`):
  ```rust
  /// `compete_fan_out` (MOD-141 M2 D5): the `app_setting` row's boolean, else
  /// [`COMPETE_FAN_OUT_DEFAULT`] (off). The house fall-through rule: absent, `null` or a
  /// non-boolean is not a value. Only the graph editor reads it; the engine runs a compete node
  /// whatever it says.
  #[must_use]
  pub fn resolve_compete_fan_out(app: &BTreeMap<String, Value>) -> bool {
      app.get("compete_fan_out")
          .and_then(Value::as_bool)
          .unwrap_or(COMPETE_FAN_OUT_DEFAULT)
  }
  ```
- Doc wording: "thirteen" → "fourteen" (`:164-168`, `from_key`'s doc); "the three switches" → "the four switches" (`:316-317`); `as_rows` and `app_keys` docs: "plus MOD-163's unseeded `memory_capture` and MOD-141's unseeded `compete_fan_out`".

### 3.2 Pins in `prompt/settings.rs` tests

- `the_defaults_are_migration_0002s_ten_rows_verbatim` (`:937-958`):
  - `rows.len()` 11 → 12.
  - Assert `rows.contains(&("compete_fan_out", Value::Bool(false)))`.
  - The filter drops both unseeded keys.
- `memory_capture_is_an_app_and_project_boolean`: `SettingKey::ALL[9]` → `ALL[10]` (`:1150`).
- `app_keys_are_0002s_ten_plus_memory_capture_in_byte_order` (`:1189-1213`): 11 → 12 (both asserts); `assert!(app.contains(&SettingKey::CompeteFanOut))`; message "and `memory_capture` and `compete_fan_out`, unseeded".
- New `compete_fan_out_is_an_app_only_boolean`:
  - Spec equality as `memory_capture`'s test does.
  - `rung_refusal(.., APP) == None`; `PROJECT` and `PHASE` are `Some`.
  - `SettingKey::ALL[2] == CompeteFanOut`, "sorts between `box_hostname` and `excerpt_file_line_cap`".
  - `validate(.., APP, true/false)` is `Ok`; `json!(1)` is `Err`.
- New `resolve_compete_fan_out_defaults_off`: `{}` → false; `{"compete_fan_out": true}` → true; `{"compete_fan_out": "on"}` and `1` → false.
- `ranges_are_the_readers_clamps`, `every_default_validates_under_its_own_spec`, `specs_are_indexed_by_discriminant`, `project_key_is_some_exactly_where_project_is_accepted`: unchanged; they iterate `ALL`.

### 3.3 Settings > Prompt (`htui`)

`ui/tabs/settings/prompt.rs`:
- Add `resolve_compete_fan_out` to the import at `:42-46`.
- `effective` (`:1091`), in the switches group after `MemoryCapture` (`:1113-1117`):
  ```rust
        SettingKey::CompeteFanOut => switch(
            resolve_compete_fan_out(app),
            project_value,
            app.get(key.key()),
        ),
  ```
  The comment above the switches: "The four switches (MOD-89 D4, MOD-33 D267, MOD-163 D19, MOD-141 M2 D5)".
- The row, its label, unit and help come from the registry: no other code.

`prompt_settings.rs`: docs "eleven" → "twelve" at `:1, :29, :37, :104, :122, :125`, naming
`compete_fan_out` beside `memory_capture` as unseeded.

`tests/prompt_settings.rs`:
- `the_demo_snapshot_has_eleven_app_entries_and_five_keys_per_project` (`:161`): `snapshot.app.len()` 11 → 12. Fix the doc. Keep the name: it is the suite's.
- `a_migrated_postgres_presents_ten_seeded_app_rows_and_an_unseeded_memory_capture` (`:636-676`):
  - 11 → 12.
  - Assert `compete_fan_out` has `value: None, updated_at: None`.
  - The two filters drop both `MemoryCapture` and `CompeteFanOut`.
  - This test needs Postgres; it runs in the lane against its own throwaway database.
- `the_demo_snapshot_renders_the_tree` (`:980`): add `assert!(row_of(&frame, SettingKey::CompeteFanOut, 0).contains("unset | off (default)"))`.
- New `compete_fan_out_is_listed_on_the_app_rung_only_and_set_there`, mirroring `memory_capture_is_listed_and_set_on_both_rungs` (`:1284-1360`), App row only:
  - `e`, the value to on, `Enter` sends `SetSetting { rung: App, key: CompeteFanOut, value: true, expected: None }`.
  - The re-read shows `| on (app)`.
  - The project group has no `compete_fan_out` row.

### 3.4 Snapshots

Expected to move: `prompt_settings__demo`, `__demo_80`, `__app_only`, `__clamped`, `__stale`,
`__editor_fraction` (one more App row). `__offline` is expected unchanged. The full insta run is
the authority (memory).

### 3.5 Validate
```bash
cargo test -p htui-core --lib prompt::settings -- --test-threads=1
cargo test -p htui --lib ui::tabs::settings::prompt -- --test-threads=1
cargo insta test -p htui --features testkit --test prompt_settings -- --test-threads=1
cargo insta test -p htui --features testkit -- --test-threads=1     # find any other frame the row moved; review
```

### 3.6 Commits
1. `feat(MOD-141): M2 T3 compete_fan_out, an App-rung switch off by default`
2. `feat(MOD-141): M2 T3 Settings > Prompt lists compete_fan_out; snapshots`

---

## T4 - Canvas badges and the warning marker (D12) (wave 1)

**File set**: `crates/htui/src/ui/graph_editor/canvas.rs` and
`snapshots/htui__ui__graph_editor__canvas__tests__*.snap`.

- Delete the `DIVIDE` const and its doc (`canvas.rs:46-49`).
- `node_lines` (the `Split::Divide => DIVIDE.to_owned()` arm): `Split::Divide => format!("divide \u{b7} {}", label::plural(node.parts.len(), "part", "parts")),`. Compete (`compete ×N`) and palette are unchanged.
- `StepNode` gains `warned: bool`.
  - `StepNode::new(node, agents, theme, warned)`: `title = if warned { format!("! {}", node.phase.name) } else { node.phase.name.clone() }`.
  - `flow_node(node, at, selected, agents, theme, warned)` passes it through.
- `Canvas` gains `warned: HashSet<PhaseId>` (empty in `new`, kept by `clear`).
  ```rust
  /// MOD-141 M2 D11, D12: the steps a run would refuse today, drawn with `!` before their name
  /// from the next [`Canvas::sync`] on.
  #[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]
  pub(crate) fn set_warned(&mut self, warned: HashSet<PhaseId>) {
      self.warned = warned;
  }
  ```
  `sync` passes `self.warned.contains(id)` to `flow_node`. `sync`'s signature does not change, so
  `mod.rs` is untouched.
- Module doc: drop any MOD-139 wording.

**Tests (write first)**:
- `the_divide_badge_counts_its_parts`: `node_lines` third line for a divide node with 0, 1 and 2 parts is `divide · 0 parts`, `divide · 1 part`, `divide · 2 parts`. A compete node reads `compete ×3`. A palette node reads `palette`.
- `a_warned_step_is_marked_from_the_next_sync`: `set_warned({research})` then `sync`; the drawn rows contain `! research`, and `verdict` has no `!`. Then `set_warned({})`, `sync`: no `!`.
- Snapshot `divide_badge` (`snapshot_divide_badge`, `:2149-2161`): add two parts to `verdict` (`PhasePart { phase_id: verdict, position: 0/1, key: "ui"/"store", title: same, .. }`). It reads `divide · 2 parts`.
- New snapshot `warned`: `snapshot_warned`, the analysis graph (`demo(ids::GRAPH_VULKAN_ANA)`), `set_warned({research})`, 100x28.
- `palette` stays (the palette band, unchanged).

**Validate**:
```bash
cargo insta test -p htui --lib -- --test-threads=1 ui::graph_editor::canvas
```

**Commit**: `feat(MOD-141): M2 T4 the divide badge counts parts; a warned step is marked`

---

## T5 - Step form M2 rows and the part form (D7-D10, D14's list half) (wave 1)

**File set**:
- `crates/htui/src/ui/graph_editor/node_form.rs`, `part_form.rs`
- `snapshots/htui__ui__graph_editor__node_form__tests__*.snap`, `snapshots/htui__ui__graph_editor__part_form__tests__*.snap`
- In `crates/htui/src/ui/graph_editor/mod.rs`, exactly:
  - `:426` `open_node_form` (`NodeForm::new(.., false)`)
  - `:807` `on_canvas_act` `GraphEditorAdd` (`NodeForm::new(.., false)`)
  - `panel_lines` `:689-701` (the `lines` call)
  - `:2171` `every_mode_names_its_stack` (`NodeForm::new(.., false)`)
- `snapshots/htui__ui__graph_editor__tests__editor_node_form.snap`
- `crates/htui/tests/snapshots/kinds__graph_editor_step.snap` (C14)

### 5.1 `node_form.rs` types

```rust
// Kept from M1: NAME_REQUIRED, RETRY_IS_A_NUMBER, BUDGET_IS_A_NUMBER, TEMPLATE_REQUIRED, NO_TEMPLATE,
// AGENTS_NOT_READ, NO_AGENTS, NO_CANDIDATE, no_candidates(add). DIVIDE_NOTE and divide_words go.
pub(crate) const DEADLINE_IS_A_NUMBER: &str =
    "`deadline` is a whole number of seconds, at least 1, or empty";
pub(crate) const FAN_OUT_IS_A_NUMBER: &str = "`fan-out` is a whole number, 2 or more";
pub(crate) const MAX_PARTS_IS_A_NUMBER: &str = "`max parts` is a whole number of at least 1, or empty";
pub(crate) const NO_PART: &str = "no part is selected";
pub(crate) const PARTS_ON_DIVIDE_ONLY: &str = "only a divide step has parts; choose divide first";
/// The parts placeholder: `none ({add} adds one)`, or `none` with nothing bound.
pub(crate) fn no_parts(add: Option<&str>) -> String;   // same words as no_candidates

/// The key labels the form's own words name (R-TUI-10); the editor resolves them from its stack.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FormKeys<'a> {
    /// `graph_editor.candidate_add`'s label: the two placeholders.
    pub(crate) add: Option<&'a str>,
    /// `graph_editor.open_part`'s label: a part row's help. `None` until T8 binds it.
    pub(crate) open: Option<&'a str>,
}

/// Which list the focus is on, for the four list acts (D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListFocus { Candidates, Parts }

/// What `open_part` opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartOpen { New, Focused }

// NodeRow (pub(crate), the editor-facing kind) gains `Part(usize)` and `NoParts`.

/// Every row the form can show (private).
enum Row {
    Name, Template, Version, Persona, Gate, HardGate, Retry, Isolation, Commands, Verify, Budget,
    Inputs, Output, Candidate(usize), NoCandidates, Deadline, Palette, Split, FanOut, Judge,
    PlannerPersona, PlannerAgent, IntegratePersona, IntegrateAgent, MaxParts, PlanScope,
    Part(usize), NoParts,
}
/// A seat: an agent and its model (`None` when the pair's model is empty).
type Seat = (AgentId, Option<String>);
```

`NodeForm` gains these fields (hand-written `Debug` adds `parts: len`, `split`, `locked`):
- `phase_id: PhaseId`
- `compete: bool` (the switch)
- `locked: bool` (`!compete && node.phase.split == Split::Compete` at open)
- `deadline: TextField`, `palette: bool`, `split: Split`, `fan_out: TextField`, `judge: Option<Seat>`
- `planner_persona: Option<PersonaId>`, `planner: Option<Seat>`, `integrate_persona: Option<PersonaId>`, `integrate: Option<Seat>`
- `max_parts: TextField`, `plan_scope: Option<PlanScope>`
- `parts: Vec<PhasePart>` (sorted by position at open)

`focus: usize` stays an index into `self.rows()`. The constants `NAME..OUTPUT = 0..12` and
`FIXED_ROWS = 13` keep their values (B3), so M1's unit tests and `tests/graph_editor.rs` drive the
same rows.

### 5.2 Rows

`fn rows(&self) -> Vec<Row>` gives the focusable rows in order. `fn shown(&self, base: &NodeBody)
-> Vec<(Row, bool)>` gives every drawn row with "editable"; `lines` draws from `shown` and focuses
`shown`'s position of `rows()[self.focus]`.

| # | Row | Label | Value | Shown when | Help | Parse / refusal in `build` |
|---|---|---|---|---|---|---|
| 0-12 | M1's 13 | as M1 | as M1 | always | as M1 | as M1 |
| 13.. | `Candidate(i)` / `NoCandidates` | `agent {i+1}` / `agents` | as M1 | always | as M1 | as M1 |
| next | `Deadline` | `deadline` | Text | always | `seconds; empty for the project's deadline` | empty → `None`; `i32 >= 1` else `DEADLINE_IS_A_NUMBER` |
| next | `Palette` | `palette` | Toggle | always | `a planner places it; it has no links` | → `palette` |
| next | `Split` | `split` | Choice `none` / `divide` / `compete`; **read-only** `compete · read-only while compete is off (Settings › Prompt)` when `locked` | always | switch on: `none, divide into parts, or compete: several agents, one winner`; off: `none or divide into parts; compete is off in Settings › Prompt` | — |
| next | `FanOut` | `fan-out` | Text; read-only when `locked` | split compete | `agents that compete, 2 or more` | `i32 >= 2` else `FAN_OUT_IS_A_NUMBER` (not when `locked`) |
| next | `Judge` | `judge` | Choice `project default` / `{agent} · {model}`; read-only when `locked` | split compete | `picks the winner; project default asks the project's judge, else you` | — |
| next | `PlannerPersona` | `planner` | Choice `none` / persona names | split divide | `the persona that plans the parts` | — |
| next | `PlannerAgent` | `planner agent` | Choice `none` / pairs | split divide | `an agent and model, or none` | — |
| next | `IntegratePersona` | `integrate` | Choice `none` / persona names | split divide | `the persona that merges the parts` | — |
| next | `IntegrateAgent` | `integrate agent` | Choice `none` / pairs | split divide | `an agent and model, or none` | — |
| next | `MaxParts` | `max parts` | Text | split divide | `empty for no cap, else at least 1` | empty → `None`; `i32 >= 1` else `MAX_PARTS_IS_A_NUMBER` |
| next | `PlanScope` | `plan scope` | Choice `none` / `its parts` / `its successors` / `the whole graph` (`plan_scope` words, `:658-664`) | split divide; **read-only** when split is not divide and the base holds one | `what the planner may plan` | — |
| next | `Part(i)` / `NoParts` | `part {i+1}` / `parts` | `Value::ReadOnly` `{key} · {title}` (`{key}` alone when the title is empty) / `no_parts(keys.add)` | split divide | `{open} edits it` when `keys.open` is `Some` | — |

Read-only rows are styled `theme.dim`, as M1's were.

`focused()`:
- `Row::Name | Retry | Verify | Budget | Inputs | Output | Deadline | FanOut | MaxParts` → `NodeRow::Text`
- `HardGate | Palette` → `NodeRow::Toggle`
- `Part(i)` → `NodeRow::Part(i)`; `NoParts` → `NodeRow::NoParts`
- `Candidate(i)` and `NoCandidates` as M1
- the rest → `NodeRow::Choice`

`text_mut` adds `Deadline`, `FanOut` and `MaxParts`.

### 5.3 `cycle` and the split transitions (D7)

`cycle(&mut self, forward, lookups) -> Result<(), String>`, by row:
- `Palette`: flip.
- `Split`: step through `[None, Divide]`, or `[None, Divide, Compete]` when `compete`. From `old` to `new`:
  - leaving `Divide`: `planner_persona = planner = integrate_persona = integrate = None`, `max_parts` cleared, `parts.clear()`, `plan_scope = None`;
  - entering `Compete`: `fan_out` text `"2"`;
  - leaving `Compete`: `fan_out` text `"1"`, `judge = None`;
  - entering `Divide`: `judge = None`.
- `Judge`, `PlannerAgent`, `IntegrateAgent`:
  - Options: `None`, then `agent_pairs(agents)` mapped to `(id, (!model.is_empty()).then_some(model))`, then the current seat when the options lack it.
  - Agents unread → `Err(AGENTS_NOT_READ)`.
- `PlannerPersona`, `IntegratePersona`: as M1's persona choice.
- `PlanScope`: `[None, Some(Parts), Some(Successors), Some(Graph)]`.
- `Part`, `NoParts`, `Text` rows: no-op.
- Locked rows are not in `rows()`, so they never cycle.

Moving the split row changes the rows after it, never the rows before it, so `focus` stays on
`split`.

### 5.4 `build(&self, base, lookups) -> Result<NodeBody, String>`

1. Refusals in row order: M1's, then `DEADLINE_IS_A_NUMBER`, `FAN_OUT_IS_A_NUMBER` (compete, not `locked`), `MAX_PARTS_IS_A_NUMBER` (divide).
2. Then write:
   - M1's columns; `deadline_seconds`; `palette`.
   - `locked`: `split`, `fan_out`, `judge_agent_id` and `judge_model` are `base`'s, byte for byte.
   - Otherwise: `split`; `fan_out` = the parsed value on compete, else `1`; `judge_agent_id` and `judge_model` from `judge` (both `None` off compete).
   - `planner_*`, `integrate_*` and `max_parts` from the fields (all `None` off divide, as 5.3 cleared them).
   - `plan_scope = self.plan_scope` (kept from base when read-only).
   - `node.parts` = `self.parts` renumbered `0..n` with `phase_id = base.phase.id`.
3. `NodeForm::new(node, target, compete: bool)` pre-fills every field from `node`. Untouched, `build(node)` equals `node` (the M1 prefill test, extended to M2 fields).

### 5.5 Lists (D14) and parts

```rust
pub(crate) fn list_focus(&self) -> ListFocus;            // Parts on Part(_)/NoParts, else Candidates
pub(crate) fn list_remove(&mut self) -> Result<(), String>;        // Parts: part_remove; else candidate_remove
pub(crate) fn list_move(&mut self, up: bool) -> Result<(), String>; // Parts: part_move; else candidate_move
pub(crate) fn part_remove(&mut self) -> Result<(), String>;        // Err NO_PART off a part row
pub(crate) fn part_move(&mut self, up: bool) -> Result<(), String>; // focus follows; ends stay
/// `New`: a blank part (Err PARTS_ON_DIVIDE_ONLY off divide). `Focused`: the focused part,
/// `PartTarget::Edit(i)` (Err NO_PART off a part row). The part form takes the form's own
/// candidates as they are now.
pub(crate) fn open_part(&self, open: PartOpen) -> Result<PartForm, String>;
/// `form.build(self.phase_id, others)` (others: every part but the edited one); `New` appends,
/// `Edit(i)` replaces; positions renumbered 0..n; the focus moves onto that part's row.
pub(crate) fn put_part(&mut self, form: &PartForm) -> Result<(), String>;
pub(crate) fn lines(&self, base: &NodeBody, lookups: &Lookups, keys: FormKeys<'_>,
                    width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>>;
```
- `candidate_add`, `candidate_remove` and `candidate_move` keep their M1 behaviour and names, so `mod.rs::on_node_form_act` compiles untouched in T5. T8 switches it to `list_*`.
- Put `#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]` on `list_focus`, `list_remove`, `list_move`, `open_part`, `put_part` and `FormKeys.open`, wherever clippy flags them.
- Helpers `part_form.rs` reuses become `pub(super)`: `text_of`, `step`, `pair_word`, `persona_word`, `agent_pairs` (already `pub(crate)`), and `input_kinds`, renamed `comma_list` (M1 callers updated).

### 5.6 `part_form.rs`

```rust
pub(crate) const PART_KEY_REQUIRED: &str = "`key` is required";
pub(crate) fn part_key_taken(key: &str) -> String {
    format!("`key` {key} is taken by another part of this step")
}
/// The persona row's help (D9: per-part personas run from MOD-140 on).
pub(crate) const PART_PERSONA_NOTE: &str = "per-part personas run from MOD-140 on";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartTarget { New, Edit(usize) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartRow { Text, Choice }

pub(crate) struct PartForm {           // hand-written Debug: target, focus, candidates len
    target: PartTarget,
    key: TextField, title: TextField, brief: TextField, owned_paths: TextField, acceptance: TextField,
    agent: Option<(AgentId, Option<String>)>,
    persona: Option<PersonaId>,
    candidates: Vec<(AgentId, String)>,   // the step form's candidates when it opened
    focus: usize,
}
impl PartForm {
    pub(crate) fn new(part: Option<&PhasePart>, target: PartTarget, candidates: Vec<(AgentId, String)>) -> Self;
    pub(crate) const fn target(&self) -> PartTarget;
    pub(crate) fn focused(&self) -> PartRow;
    pub(crate) fn next_field(&mut self);  pub(crate) fn prev_field(&mut self);   // wrap over 7 rows
    pub(crate) fn on_text_key(&mut self, key: KeyEvent) -> FieldOutcome;
    pub(crate) fn on_paste(&mut self, text: &str);
    pub(crate) fn cycle(&mut self, forward: bool, lookups: &Lookups);
    pub(crate) fn build(&self, phase: PhaseId, others: &[PhasePart]) -> Result<PhasePart, String>;
    pub(crate) fn title(&self, step: &str) -> String;   // "new part of {step}" / "part {key} of {step}"
    pub(crate) fn lines(&self, lookups: &Lookups, width: u16, accent: Style, theme: &Theme) -> Vec<Line<'static>>;
}
```

| # | Row | Label | Value | Help | `build` |
|---|---|---|---|---|---|
| 0 | key | `key` | Text | `unique in this step` | trimmed; empty → `PART_KEY_REQUIRED`; equal to an `others` key → `part_key_taken(key)` |
| 1 | title | `title` | Text | — | trimmed |
| 2 | brief | `brief` | Text | `what this part does` | trimmed |
| 3 | owned paths | `owned paths` | Text | `repo:glob, comma separated` | `comma_list` |
| 4 | acceptance | `acceptance` | Text | `empty for none` | trimmed; empty → `None` |
| 5 | agent | `agent` | Choice `none` / the step's candidates as `pair_word`; a current agent missing from them is kept and shown | `one of the step's own agents (a run refuses any other)` | `agent_id`, `model` (`None` for an empty model) |
| 6 | persona | `persona` | Choice `none` / the registry; an unknown id is kept | `PART_PERSONA_NOTE` | `persona_id` |

`build` answers a `PhasePart` with `position: 0`; `put_part` renumbers. The whole module is
unused until T8: put the wave allow on `PartForm` and on the constants clippy flags.

### 5.7 `mod.rs` call sites (exactly these)

- `:426`: `NodeForm::new(node, NodeTarget::Edit(node.phase.id), false)` with `// MOD-141 M2 T8 passes the switch`.
- `:807`: `NodeForm::new(&base, NodeTarget::New { after: self.selected }, false)`.
- `:689-701` `panel_lines`: `editing.form.lines(&editing.base, &self.lookups, node_form::FormKeys { add: add.as_deref(), open: None }, width, ctx.accent(), theme)`.
- `:2171`: `NodeForm::new(&node, NodeTarget::Edit(phase(1)), false)`.

### 5.8 Tests (write first)

Adjusted M1 tests:
- Every `NodeForm::new(..)` / `edit(..)` passes `false`.
- `the_form_prefills_every_m1_field_from_the_node`: also asserts the M2 fields prefill. Untouched, `build == node` for a divide node with parts and for a compete node with a judge.
- `build_writes_every_field_back_and_keeps_the_m2_columns_of_the_base` → `build_writes_every_m1_field_and_keeps_the_m2_fields_it_opened_with`:
  - Open the form **on** a store-valid divide base: `fan_out 1`, divide columns, `plan_scope Parts`, one part, `deadline_seconds Some(600)`.
  - Edit the M1 rows exactly as today.
  - `build(&base)` equals `base` with the M1 edits; the M2 columns are unchanged.
- `next_field_skips_the_read_only_rows`: after `Candidate(0), Candidate(1)` come `Text` (deadline), `Toggle` (palette), `Choice` (split), then `Text` (name) again. The loop runs `FIXED_ROWS + 5` times.
- `node_form_divide` snapshot: rewritten (5.9).

New tests:
1. `deadline_empty_is_none_and_below_1_is_refused` (`""`, `"  "`, `"1"`, `"7200"`, `"0"`, `"-3"`, `"soon"`).
2. `palette_toggles`.
3. `split_offers_compete_only_with_the_switch_on`: off cycles `none → divide → none`; on cycles `none → divide → compete → none`.
4. `leaving_divide_clears_the_divide_columns_parts_and_plan_scope`.
5. `entering_compete_sets_fan_out_2_and_leaving_it_sets_1_and_clears_the_judge`.
6. `entering_divide_clears_the_judge`.
7. `a_compete_node_with_the_switch_off_is_read_only_and_kept_byte_for_byte`:
   - Base: compete, fan-out 3, judge `(AGY, Some("gemini"))`.
   - `rows()` holds no `Split`, `FanOut` or `Judge`.
   - `build(base)` keeps the four columns even after the name is edited.
8. `fan_out_is_a_whole_number_2_or_more` (`"2"`, `"5"`, `"1"`, `"0"`, `"x"`).
9. `the_judge_cycles_project_default_then_every_pair_and_needs_the_agents_read`.
10. `divide_seats_cycle_personas_and_pairs`.
11. `max_parts_empty_is_none_and_below_1_is_refused`.
12. `plan_scope_cycles_none_parts_successors_graph`.
13. `a_stored_plan_scope_off_divide_is_read_only_and_kept`.
14. `the_list_acts_follow_the_focus`:
    - On a part row, `list_remove` and `list_move` act on parts.
    - On a candidate row they act on candidates; on another row, `NO_CANDIDATE` (M1's).
    - `open_part(Focused)` off a part row → `NO_PART`; `open_part(New)` off divide → `PARTS_ON_DIVIDE_ONLY`.
15. `put_part_appends_replaces_and_renumbers`.
16. `a_part_key_is_required_and_unique_in_the_step`.
17. `part_paths_split_on_commas_and_acceptance_empty_is_none`.
18. `the_part_agent_cycles_none_then_the_steps_candidates_and_keeps_an_unknown_one`.
19. `the_part_persona_cycles_none_then_the_registry`.
20. `part_form_debug_never_prints_typed_text`.

### 5.9 Snapshots (width 56, `text(lines)`)

- `node_form_edit`, `node_form_new`: change (three rows appended).
- `node_form_divide`: research as divide: planner `architect · claude · opus`, integrate `reviewer · agy` (no model), max parts 4, plan scope parts, parts `ui · UI` and `store · Store`, focus on `part 1`, `FormKeys { add: Some("Alt+n"), open: Some("Alt+e") }`.
- New `node_form_compete_switch_on`: compete, fan-out 3, judge `claude · opus`, switch on, focus on `judge`.
- New `node_form_compete_switch_off`: the same node, switch off (three read-only rows).
- New `part_form_edit`: key `ui`, title `UI`, brief, owned paths `htui:crates/htui/src/ui/**`, acceptance `cargo test -p htui`, agent `claude · opus`, persona `reviewer`, focus on `persona` (its note shows).
- `mod.rs` `editor_node_form`, and `tests/snapshots/kinds__graph_editor_step.snap`: re-accepted (three rows appended, no M2 note).

### 5.10 Validate
```bash
cargo insta test -p htui --features testkit --lib --test kinds --test graph_editor -- --test-threads=1
```
Accept only the snapshots listed in 5.9. `every_m1_node_field_round_trips` must pass unchanged
(C15).

### 5.11 Commits
1. `feat(MOD-141): M2 T5 step form edits deadline, palette and split`
2. `feat(MOD-141): M2 T5 compete and divide rows, the switch-off read-only compete node`
3. `feat(MOD-141): M2 T5 part form and the focus-relative list methods`

---

## T6 - Palette add and the warnings (D10, D11) (wave 1)

**File set**: `crates/htui/src/ui/graph_editor/model.rs`, `notes.rs`.

### 6.1 `model.rs`

```rust
use crate::ui::kit::label;

/// D10: turning palette on for a step that has links, in words.
pub(crate) fn palette_has_links(links: usize) -> String {
    format!(
        "a palette step has no links; delete its {} first",
        label::plural(links, "link", "links")
    )
}
```
- `add_after` (`model.rs:96-128`): read `let palette = node.phase.palette;` before `adopted`, then `let flow = after.filter(|_| !palette).filter(|&after| ..)`. A palette node is appended with no edges and takes nothing over. Doc: "... or a palette **new node** (D10): `node` is a root with no edges."
- `replace_node` (`:130-142`): before writing, if `node.phase.palette` and `n = body.edges.iter().filter(|e| e.from == node.phase.id || e.to == node.phase.id).count() > 0` → `Err(palette_has_links(n))`. Doc: add the refusal.
- `adopted` (`:249-258`) also re-tags and renumbers `node.parts` (`phase_id`, `position` `0..n`).

Tests (write first):
- `a_palette_step_is_added_with_no_edges_and_takes_nothing_over`: feature graph, add after `plan` with `palette = true`. The body gains the node; the edges are byte-identical.
- `palette_on_for_a_linked_step_is_refused_in_words`: `review` with `palette = true` → `Err(palette_has_links(n))`, `n` = its edge count. The words read `delete its 1 link first` for one edge.
- `palette_on_for_an_unlinked_step_is_kept`.
- `adopted_retags_and_renumbers_parts`.

### 6.2 `notes.rs`

```rust
use htui_core::model::{Isolation, NodeBody, PhasePart, PlanScope, Split};
use htui_orch::divide::owned_path_refusal;
use htui_orch::graph::{NotRunnable, ResolveError};
use htui_orch::status::{PlanCap, RunFailure};

/// MOD-141 M2 D11 (M-4): every sentence `htui_orch::graph::resolve` would refuse a run of `node`
/// with today, each the `ResolveError` resolve raises, in resolve's order; empty when none.
/// `max_fan_out` is `RunBounds::max_fan_out` once the editor's settings read answered; `None`
/// skips the two checks that need it. Known limits: `max_agents_per_run` needs the whole graph,
/// the project's default isolation and candidate rungs are not visible here, and the graph-wide
/// checks (`FanOutEdge`, `LocalConcurrent`, `ReviewFanOut`) stay resolve's.
#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]
pub(crate) fn notes(node: &NodeBody, max_fan_out: Option<u32>) -> Vec<String>
```
In order. `name = node.phase.name`, `divide = split == Divide`:
1. `divide && planner_agent_id.is_some()` → `ResolveError::NotYetRunnable { node: name, reason: NotRunnable::PlannerAgent }`. This mirrors `not_yet_runnable` (`graph.rs:752-765`), where `divided_name` of the planner node is the divided node's own name.
2. `divide && parts.is_empty()` → `NotRunnable::NoParts` (`:770-775`).
3. `divide && !matches!(plan_scope, None | Some(PlanScope::Parts))` → `NotRunnable::PlanScope` (`:776-778`).
4. `palette` → `NotRunnable::Palette` (`:780-782`).
5. `Some(max)` and `u32::try_from(fan_out).is_ok_and(|f| f > max)` → `ResolveError::FanOutCap { phase: name, fan_out, max }`. This is `check_fan_out_caps` (`:1499-1521`).
6. `divide && !parts.is_empty()`: the parts sorted by position, `count = parts.len() as u32`. Each sentence is `ResolveError::DivideRefused { node: name, reason }` (`divide_refusal`, `:796-812`), whose Display is `reason`'s:
   - a. `isolation == Some(Isolation::Local) && count > 1` → `RunFailure::LocalCannotDivide { node, parts: count }` (`divide.rs:189-194`; only the step's own `local`).
   - b. `max_parts = Some(m)` and `count > u32::try_from(m).unwrap_or(0)` → `RunFailure::PlanOverCaps { node, cap: PlanCap::MaxParts, planned: count, max }` (`:201-206`).
   - c. `Some(max)` and `count > max` → `PlanOverCaps { cap: PlanCap::MaxFanOut, planned: count, max }` (`:208-210`).
   - d. Per part, in position order:
     - `persona_id.is_some()` → `PartPersonaUnsupported { node, part: key }`;
     - each `owned_paths` entry `p` with `owned_path_refusal(p.trim()) == Some(reason)` → `PartPathRefused { node, part: key, path: p.trim().to_owned(), reason }`;
     - `agent_id = Some(a)`, `!node.agents.is_empty()` and no candidate holds `a` → `PartAgentNotCandidate { node, part: key, agent: a }` (`:220-247`).
   - `EmptyPlan` never: item 2 covers it.

Each note is `.to_string()` of the constructed `ResolveError`.

Tests (write first; each `assert_eq!(notes(..), vec![ResolveError::...{..}.to_string(), ..])`):
1. `a_clean_step_has_no_notes`
2. `a_planner_agent_is_noted_first`
3. `a_part_less_divide_is_noted_no_parts`
4. `a_plan_scope_past_parts_is_noted`
5. `a_palette_step_is_noted`
6. `a_compete_fan_out_above_max_fan_out_is_noted`
7. `the_default_parts_are_judged_as_admission_judges_them`: one divide node trips each of 6a-6d, and the list is in that order.
8. `every_failing_check_is_listed_not_only_the_first` (C11)
9. `without_max_fan_out_the_two_cap_checks_are_skipped`
10. `a_part_agent_is_judged_only_against_listed_candidates`

### 6.3 Validate
```bash
cargo test -p htui --lib -- --test-threads=1 ui::graph_editor::model ui::graph_editor::notes
```

### 6.4 Commits
1. `feat(MOD-141): M2 T6 palette add with no edges; palette on a linked step refused in words`
2. `feat(MOD-141): M2 T6 notes, resolve's own sentences for a step`

---

## Wave-1 independence (re-confirmed after detailing)

| Task | Files |
|---|---|
| T2 | `htui-orch/src/graph.rs` |
| T3 | `htui-core/src/prompt/settings.rs`, `htui/src/prompt_settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/tests/prompt_settings.rs`, `htui/tests/snapshots/prompt_settings__*` (and any frame the full run shows moving only by the Prompt App group) |
| T4 | `graph_editor/canvas.rs`, `graph_editor/snapshots/*canvas__tests__*` |
| T5 | `graph_editor/node_form.rs`, `graph_editor/part_form.rs`, `graph_editor/snapshots/*node_form__tests__*` + `*part_form__tests__*` + `*graph_editor__tests__editor_node_form.snap`, `graph_editor/mod.rs` (four call sites, 5.7), `htui/tests/snapshots/kinds__graph_editor_step.snap` |
| T6 | `graph_editor/model.rs`, `graph_editor/notes.rs` |

- All pairwise intersections are empty.
- T4 keeps `Canvas::sync`'s signature and T6 keeps `node_write`'s calls, so `mod.rs` stays T5's alone.
- Build coupling: T3 changes `htui-core`, which every htui lane compiles. T6 compiles against T1's `pub` `owned_path_refusal`.
- No lane writes `.sqlx`. Only T3's one Postgres test and T1/T7 touch the database.
- After the last merge, run on `hr/MOD-141`: `cargo test -p htui --lib ui::graph_editor -- --test-threads=1` and `cargo test -p htui --features testkit --test kinds --test graph_editor --test prompt_settings -- --test-threads=1`.

---

## Wave-1 outcomes (main thread, 2026-11-12, binding on T7-T9 and review)

Merged on `hr/MOD-141` at `b8b05d0a` (T4 `0d4d927b`, T2 `5d92d9b1`, T6 `03a46994`, T3 `381fbcd7`,
T5 `b8b05d0a`). Where a lane went past this blueprint after its verifiers, the code wins:

- **W1 `notes` order (T6 repair).** `notes` first lists the two per-step refusals `snapshot_phase` raises
  before anything else: `ResolveError::LocalFanOut` (the step's own `isolation == Some(Local)` with
  `fan_out > 1`), then `ReviewFanOut` (a step named `review` with `fan_out > 1`). Only after those does it
  follow §6.2's list. On a **divide** step the palette note names `N:plan`, as `not_yet_runnable` does.
  The known limits also name `NoTemplate` (the project's templates are not visible here). The test
  `the_first_note_is_the_refusal_resolve_raises` cross-checks the first note against the real `resolve`.
- **W2 Step form (T5).** `NodeForm::new` takes `compete: bool`, and `lines` takes `FormKeys { add, open }`.
  The part form's agent choice lists a candidate the step holds twice only once. A judge stored on a step
  whose split is not compete is cleared when the step is saved through the form (§5.4).
- **W3 Wave allows** to remove in T8: `git grep -n 'MOD-141 M2 T8 wires it'`.

## T8 outcomes (main thread, 2026-11-12, binding on T9 and review)

T8's repair 1 (`d919caa7`) went past §8.2 after its verifiers; the code wins. W4 and W5 are T8's
additions to `node_form.rs`, a T5 file that §8.2 otherwise opens only for its allow attributes.

- **W4 The switch's first read.** `NodeForm::switch_read(compete)` is new. On `true` the form offers
  compete and a locked compete step unlocks: its split, fan-out and judge hold the step's own values,
  and the rows unlocking adds come after every focusable row a locked form has, so the focus keeps its
  row. On `false` it changes nothing. `GraphEditor::on_reply` calls it on the **first**
  `GraphEditorSettings` reply only (`self.settings.is_none()`), on the open step form or the step form
  under a part form: Kinds' `e` opens the step form before the editor's reads leave. So §8.2's "An open
  form keeps the switch it opened with" and Open risk 4 hold only for reads after the first. Pin:
  `a_form_opened_before_the_first_settings_read_takes_its_switch` (mod.rs).
- **W5 The step form's layout.** `NodeForm::layout(..) -> kit::form::FormLines` is new: the rows and
  the focused row's lines among them. `NodeForm::lines` (W2's `FormKeys` signature) is `#[cfg(test)]`
  now, `layout`'s lines alone. `panel_lines` takes the panel's height, and `fitted` keeps the D11 notes
  under the rows (at most half the panel, `…` ending them when cut) and scrolls the rows to the focused
  one. Pins: `a_tall_step_form_scrolls_to_its_focus_and_keeps_its_notes`,
  `notes_past_half_the_panel_end_in_an_ellipsis`.
- **W6 The limits row.** The header keeps §8.2's `Length(2)`. `limits_row` puts `limits_line` in its
  one row: past the width it drops whole `label value` segments and ends ` · …`, and a row too narrow
  for the first segment is `…` (Open risk 5). Pin: `the_header_shows_the_limits_once_read`.
- **W7 The part form waits for a write.** `open_part` refuses with `history::IN_FLIGHT` while a write is
  in flight, as `open_node_form` does (the reply would close the part form). Main thread, `16d50afe`,
  from the last unrepaired verifier finding. Pin: `no_part_form_opens_while_a_write_is_in_flight`.

---

## T7 - Editor settings read (D6) (wave 2, serial)

**File set**:
- `crates/htui/src/store_worker.rs`, `crates/htui/src/catalogue.rs`
- `crates/htui/src/ui/tabs/settings/kinds.rs` (`doing` only, C7)
- `crates/htui/tests/kinds.rs`

### 7.1 Types

`catalogue.rs` (B2), near `CatalogueSnapshot`:
```rust
/// MOD-141 M2 D6: what the graph editor reads beside the body, from one `app_setting` read: the
/// run limits resolve freezes and the compete switch. The answer to
/// [`StoreRequest::GraphEditorSettings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorSettings {
    /// [`htui_orch::graph::run_bounds`] over the `app_setting` rows.
    pub bounds: htui_orch::graph::RunBounds,
    /// [`htui_core::prompt::settings::resolve_compete_fan_out`]: whether the compete split is offered.
    pub compete: bool,
}
```
`store_worker.rs`:
- `StoreRequest`, after `GraphBody(StepGraphId)` (`:796-799`):
  ```rust
    /// MOD-141 M2 D6: the run limits and the compete switch the graph editor shows and obeys, from
    /// one `app_setting` read. A read. Answers [`StoreReply::GraphEditorSettings`].
    GraphEditorSettings,
  ```
- `StoreReply`, after `GraphBody(..)` (`:1691-1692`):
  ```rust
    /// MOD-141 M2 D6: the answer to [`StoreRequest::GraphEditorSettings`].
    GraphEditorSettings(Box<EditorSettings>),
  ```
  Import `crate::catalogue::EditorSettings`.
- `name()` (`:1409-1422`): `Self::GraphEditorSettings => "graph_editor_settings",` after `graph_body`; the comment "The thirteen of `catalogue::REQUEST_NAMES` ... MOD-141 M2 D6".
- `try_serve` (`:2487-2501`): `| StoreRequest::GraphEditorSettings` joins the catalogue or-arm. Comment "thirteen".
- Doc comments "twelve" → "thirteen": `:656-660`, `catalogue.rs:194-209`, `:404-406`.

### 7.2 `catalogue::serve`

After the `GraphBody` arm (`:399-403`):
```rust
        // MOD-141 M2 D6: one `app_setting` read, the editor's limits and switch. Offline it never
        // gets here: `writer()` is `None` and the request answers `Unreachable` like the rest.
        StoreRequest::GraphEditorSettings => {
            let app = backend.app_settings().await?;
            Ok(StoreReply::GraphEditorSettings(Box::new(EditorSettings {
                bounds: htui_orch::graph::run_bounds(&app),
                compete: htui_core::prompt::settings::resolve_compete_fan_out(&app),
            })))
        }
```
`REQUEST_NAMES: [&str; 13]`, with `"graph_editor_settings"` after `"graph_body"`. Doc: "The
thirteen request names ... then MOD-141 D13's one and M2 D6's one".

`kinds.rs::doing` (`:1911-1928`): `"graph_editor_settings" => "reading the run limits",` after
`"graph_body"`.

### 7.3 Tests (`tests/kinds.rs`, write first)

- `catalogue_requests` (`:99-182`): append `StoreRequest::GraphEditorSettings`. Doc "thirteen".
- `catalogue_names_are_stable` (`:186-199`): `REQUEST_NAMES.len() == 13`; `REQUEST_NAMES[9..] == ["replace_graph", "delete_phase", "graph_body", "graph_editor_settings"]`.
- `offline_refuses_every_catalogue_request_by_name`: unchanged; it covers the new one.
- `graph_editor_settings_answers_the_code_defaults`: over `Backend::memory(MemStore::demo())`, `StoreReply::GraphEditorSettings(s)` with `s.bounds == RunBounds { 4, 8, 4, 3, 8, RuntimeChanges::Ask }` and `!s.compete`.
- `graph_editor_settings_reads_every_bound`: `store.set_app_setting` for the six keys (`2, 5, 7, 1, 3, "off"`) → those values.
- `graph_editor_settings_reads_the_compete_switch`: `serve(SetSetting { rung: App, key: CompeteFanOut, value: true, expected: None })` → `compete`. Then set it false with the row's token → `!compete`.
- `kinds.rs` unit test `a_write_in_flight_reads_in_words` passes (the new phrase).

### 7.4 Validate
```bash
cargo test -p htui --features testkit --test kinds -- --test-threads=1
cargo test -p htui --lib -- --test-threads=1 catalogue store_worker ui::tabs::settings::kinds
```

### 7.5 Commit
`feat(MOD-141): M2 T7 GraphEditorSettings, the editor's limits and switch; REQUEST_NAMES 13`

---

## T8 - Editor orchestration and keys (D6, D11-D14) (wave 2, serial)

**File set**:
- `crates/htui/src/ui/graph_editor/mod.rs` and its snapshots
- `crates/htui/src/ui/tabs/settings/kinds.rs` (routing, C7)
- `crates/htui/src/keys/{catalogue,views,stack}.rs`
- `crates/htui/tests/{kinds,graph_editor}.rs` (request pins only) and every snapshot the full run moves
- `docs/htui-keys.md`
- the wave-1 modules, to remove the wave allows

`keys/load.rs` and `tests/keys_file.rs` need no change: no new context, so their table lists hold.

### 8.1 Keys

`catalogue.rs`:
- `enum Act`: after `GraphEditorCandidateDown` (`:920`):
  ```rust
    /// `graph_editor.open_part`: the step form: open the focused part's form (MOD-141 M2 D14).
    GraphEditorOpenPart,
  ```
- The four list acts' docs (`:913-920`) widen to "... the focused list's: a candidate agent, or a part of a divide step".
- `CATALOGUE`, after the `candidate_down` row (`:1664-1671`):
  ```rust
    row(
        Act::GraphEditorOpenPart,
        GraphEditor,
        "open_part",
        &["alt-e"],
        "edit part",
    ),
  ```
- Help words of the four list rows (D14):

  | Row | Help (was) | Help (now) |
  |---|---|---|
  | `candidate_add` | `add agent` | `add agent or part` |
  | `candidate_remove` | `remove agent` | `remove agent or part` |
  | `candidate_up` | `agent up` | `agent or part up` |
  | `candidate_down` | `agent down` | `agent or part down` |

- Tests: append `Act::GraphEditorOpenPart` to `ALL` after `Act::KindsEditor` (`:2982`). `position`: `Act::GraphEditorOpenPart => 245` after `KindsEditor => 244` (`:3233`). `every_act_has_exactly_one_row` (`:3267`): 245 → **246**.
- Chord check: `no_two_actions_in_one_context_share_a_default_chord` holds, since `alt-e` is unused in `[graph_editor]` (B4). `no_act_name_is_a_nested_table` holds, since there is no `graph_editor.open_part` table.

`views.rs`:
- `GRAPH_EDITOR_NODE_FORM` (`:431-446`): append `Act::GraphEditorOpenPart` to its view layer; doc "... the candidate and part lists, and the part form's open".
- After `GRAPH_EDITOR_EDGE_FORM` (`:448-457`):
  ```rust
  /// The graph editor's part form (MOD-141 M2 D9, D14): text fields and choices, as the link form.
  pub const GRAPH_EDITOR_PART_FORM: Stack<'static> = Stack::new(&[
      Layer::view(Context::GraphEditor, &[]),
      FIELD,
      PICK,
      FORM_FIELDS,
      MODAL,
  ]);
  ```
  It equals `GRAPH_EDITOR_EDGE_FORM` by value. That is harmless: `validate::check` reports a pair and chord once.
- Tests:
  - `VIEW_STACKS` (`:1365`): `GRAPH_EDITOR_PART_FORM` after `GRAPH_EDITOR_EDGE_FORM` (`:1388`).
  - `the_m2_layers_sit_where_the_blueprint_puts_them` (`:2034`): `"GRAPH_EDITOR_PART_FORM"` after `"GRAPH_EDITOR_EDGE_FORM"`.
  - `every_view_layer_comes_first...` (`:2133`): 90 → **91**; comment "MOD-141 M2 D14: the part form's, 91".
  - `the_view_stacks_resolve_as_designed` (`:1547`) adds:
    - `resolve(GRAPH_EDITOR_NODE_FORM, "alt-e") == [Act::GraphEditorOpenPart]`
    - `resolve(GRAPH_EDITOR_PART_FORM, "tab") == [Act::FormNextField]`
    - `resolve(GRAPH_EDITOR_PART_FORM, "alt-e") == []`
    - `resolve(GRAPH_EDITOR_PART_FORM, "enter") == [Act::FieldSubmit]`

`stack.rs` `DECLARED`: append after the delete-question entry (`:364-367`):
`("in the graph editor's part form", views::GRAPH_EDITOR_PART_FORM),`. Its pin (`:462`):
97 → **98**. `DECLARED[83]` and `[84]` are unchanged.

`docs/htui-keys.md`: regenerate the toml block from `cargo run -q -p htui -- --default-keys
--print-keys` (rule `tests/keys_file.rs:732`). Splice it in:
```bash
cargo run -q -p htui -- --default-keys --print-keys > /tmp/keys.toml
python3 -I - docs/htui-keys.md /tmp/keys.toml <<'PY'
import sys
page, block = open(sys.argv[1]).read(), open(sys.argv[2]).read()
head, rest = page.split("```toml\n", 1); _, tail = rest.split("```\n", 1)
open(sys.argv[1], "w").write(head + "```toml\n" + block + "```\n" + tail)
PY
```

### 8.2 `GraphEditor` (mod.rs)

- **Settings** (C8): field `settings: Option<EditorSettings>` (`None` in `open`).
  - `wants_requests` (`:434-438`) → `vec![StoreRequest::Agents, StoreRequest::Templates(scope.clone()), StoreRequest::GraphEditorSettings]`. Update the docs at `:368-371`.
  - `on_reply` (`:585-627`) adds:
    - `StoreReply::GraphEditorSettings(settings) => { self.settings = Some(**settings); self.resync.set(true); }`
    - `StoreReply::Failed { request, message } if *request == "graph_editor_settings" => self.notice = Some(Notice::Info(unavailable("run limits", message)))`
- **Compete**: both `NodeForm::new(..)` call sites pass `self.settings.is_some_and(|settings| settings.compete)`. An open form keeps the switch it opened with (except on the first read: W4).
- **Header** (D13): `render` (`:631-661`) uses `Constraint::Length(2)` for the header. The paragraph becomes `vec![Line::styled(self.header(keys), theme.title), Line::styled(limits_line(self.settings.as_ref()), theme.dim)]`.
  ```rust
  /// The header's second line before the settings read answered (D6).
  const LIMITS_NOT_READ: &str = "limits not read yet";

  /// D13: the run limits resolve freezes, read-only (M-1).
  fn limits_line(settings: Option<&EditorSettings>) -> String {
      settings.map_or_else(
          || LIMITS_NOT_READ.to_owned(),
          |settings| {
              let bounds = &settings.bounds;
              format!(
                  "limits \u{b7} fan-out {} \u{b7} agents {} \u{b7} plan revisions {} \u{b7} parallel {} \
                   \u{b7} child runs {} \u{b7} run-time changes {}",
                  bounds.max_fan_out,
                  bounds.max_agents_per_run,
                  bounds.max_plan_revisions,
                  bounds.max_parallel_sessions,
                  bounds.max_child_runs,
                  bounds.runtime_changes.as_str(),
              )
          },
      )
  }
  ```
- **Part form mode**:
  ```rust
  /// The part form over the step form it came from (M2 D9).
  struct PartEditing { form: PartForm, node: Box<NodeEditing> }
  // Mode::PartForm(Box<PartEditing>); Mode::name => "part form";
  // stack() => views::GRAPH_EDITOR_PART_FORM; panel shown.
  ```
  Update `stack`'s doc. `hint_text` uses `HINT_PART_FORM`:
  ```rust
  /// The part form's keys.
  const HINT_PART_FORM: HintSpec = &[
      Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
      Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "value"),
      Hint::One(Act::FieldSubmit, "done"),
      Hint::One(Act::FieldCancel, "cancel"),
  ];
  ```
  - `takes_paste`, `on_paste` and `on_text_key` gain `Mode::PartForm` arms on its `PartRow::Text` rows.
  - In `on_text_key`, `Submit` → `self.keep_part()` and `Cancel` → `self.back_to_step_form()` when in the part form.
  - `on_act` → `Mode::PartForm(_) => self.on_part_form_act(act)`:
    - `FormNextField`/`FormPrevField`: move;
    - `ChoicePrev`/`ChoiceNext`: `form.cycle(.., &self.lookups)`;
    - `FieldSubmit`: `keep_part`;
    - `FieldCancel`: `back_to_step_form`.
  - `keep_part`:
    1. `mem::replace` the mode out.
    2. `node.form.put_part(&form)`.
    3. `Ok` → `Mode::NodeForm(node)` and clear the notice. `Err` → put the part form back and `refuse`.
  - `back_to_step_form`: `Mode::NodeForm(node)`, notice cleared.
  - `panel_lines`: title `form.title(step_name(&self.body, editing.node.base.phase.id).unwrap_or("the new step"))`, rows `form.lines(&self.lookups, width, ctx.accent(), theme)`.
- **Lists** (D14), in `on_node_form_act` (`:916-950`):
  ```rust
  Act::GraphEditorCandidateAdd => match form.list_focus() {
      ListFocus::Candidates => form.candidate_add(&self.lookups),
      ListFocus::Parts => return self.open_part(PartOpen::New),
  },
  Act::GraphEditorOpenPart => return self.open_part(PartOpen::Focused),
  Act::GraphEditorCandidateRemove => form.list_remove(),
  Act::GraphEditorCandidateUp => form.list_move(true),
  Act::GraphEditorCandidateDown => form.list_move(false),
  ```
  `open_part(open)`: `node.form.open_part(open)`. `Ok(form)` moves the step form into `Mode::PartForm(Box::new(PartEditing { form, node }))`. `Err` → `refuse`. It answers `true`.
- **Hints** (D14): `hint_text`'s `Mode::NodeForm(editing)` picks `HINT_NODE_FORM_PARTS` when `editing.form.list_focus() == ListFocus::Parts`, else `HINT_NODE_FORM` (unchanged words):
  ```rust
  /// The step form's keys while a part row has the focus.
  const HINT_NODE_FORM_PARTS: HintSpec = &[
      Hint::Pair(Act::FormNextField, Act::FormPrevField, "field"),
      Hint::Pair(Act::ChoicePrev, Act::ChoiceNext, "value"),
      Hint::One(Act::GraphEditorCandidateAdd, "add part"),
      Hint::One(Act::GraphEditorOpenPart, "edit part"),
      Hint::One(Act::GraphEditorCandidateRemove, "remove part"),
      Hint::Pair(Act::GraphEditorCandidateUp, Act::GraphEditorCandidateDown, "order"),
      Hint::One(Act::FieldSubmit, "save"),
      Hint::One(Act::FieldCancel, "cancel"),
  ];
  ```
  `panel_lines` passes `FormKeys { add, open: keys.label(views::GRAPH_EDITOR_NODE_FORM, Act::GraphEditorOpenPart).as_deref() }`.
- **Notes and markers** (D11, D12, B5):
  - `let max_fan_out = self.settings.map(|settings| settings.bounds.max_fan_out);`
  - `panel_lines`, `Mode::NodeForm`: after the form's rows, `let node = editing.form.build(&editing.base, &self.lookups).unwrap_or_else(|_| editing.base.clone());`. Then for each `note` in `notes::notes(&node, max_fan_out)`: `wrapped(&note, width - 2)`, the first line prefixed `! ` and the rest `  `, each `notice(line, Tone::Error, theme)`.
  - `sync_canvas` (`:753-761`): before `sync`, `canvas.set_warned(self.body.nodes.iter().filter(|n| !notes::notes(n, max_fan_out).is_empty()).map(|n| n.phase.id).collect())`.
- **Palette** (C12): `node_write` is unchanged. `model::add_after` and `replace_node` answer the palette rules.
- **Delete question** (C13): `delete_question` (`:176-183`) → "... undo brings the step back but not its skill bindings. {answers}". Fix the expected string at `:2058-2062`.
- **Allows**: remove every `#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]` in `canvas.rs`, `node_form.rs`, `part_form.rs` and `notes.rs`.

`kinds.rs` (C7):
- `on_editor_reply` (`:737-781`):
  - `StoreReply::Agents(_) | StoreReply::Templates(_) | StoreReply::GraphEditorSettings(_)` forward to the editor.
  - The forwarded `Failed` names become `"replace_graph" | "agents" | "templates" | "graph_editor_settings"`.
- `on_reply` (`:1528-1570`): before the `REQUEST_NAMES.contains` arm, add `StoreReply::Failed { request, .. } if *request == "graph_editor_settings" => {}`, with the comment "the editor's read, after it closed; nothing of the section's".

### 8.3 Tests (write first)

`mod.rs`:
- `opening_asks_for_agents_and_templates` (`:1559`) → `opening_asks_for_agents_templates_and_settings`: three requests, in that order.
- `the_header_shows_the_limits_once_read`: before the reply, the second header line is `limits not read yet`. After `GraphEditorSettings(EditorSettings { bounds: run_bounds(&{}), compete: false })` it is `limits · fan-out 4 · agents 8 · plan revisions 3 · parallel 4 · child runs 8 · run-time changes ask`.
- `a_refused_settings_read_says_so`.
- `compete_is_offered_once_the_switch_reads_on`.
- `alt_n_on_the_parts_list_opens_a_new_part_form_and_enter_keeps_it`.
- `alt_e_edits_the_focused_part_and_esc_leaves_it_unchanged`.
- `the_hint_names_the_focused_lists_noun`: `add agent` vs `add part`, `edit part`.
- `a_noted_step_is_marked_and_its_form_lists_the_notes`.
- `every_mode_names_its_stack` (`:2158`): add `PartForm`.
- `debug_never_prints_typed_text` (`:2194`): also the part form.
- `delete_asks_then_rewires_in_one_write` (`:2053`): the new question.
- Snapshots: `editor_*` re-accepted (second header line; the confirm words). New `editor_part_form` (alt-e on a part of a divide step) and `editor_notes` (a part-less divide step's form and its marked canvas node).

Keys: the pins in 8.1.

`tests/kinds.rs`:
- `answer_lookups` (`:3365-3375`): also answers `StoreRequest::GraphEditorSettings`.
- `enter_on_a_graph_row_opens_the_editor_and_asks_for_agents_and_templates` (`:3394-3428`): `[Agents, Templates(scope), GraphEditorSettings]`, and `wanted == [Catalogue, Agents, Templates, GraphEditorSettings]`.
- `e_on_a_phase_row_opens_its_step_form_in_the_editor` (`:3459-3478`): the three.

`tests/graph_editor.rs` `analysis_bench` (`:485-488`): `[StoreRequest::Agents, StoreRequest::Templates(_), StoreRequest::GraphEditorSettings]`.

### 8.4 Snapshot impact (the full run decides)

| Snapshot | Change |
|---|---|
| `graph_editor/snapshots/*tests__editor_{canvas,node_form,links,confirm,connecting}` | second header line; the confirm words |
| `tests/snapshots/graph_editor__reference_graph` | second header line (`limits not read yet` vs read: the shell routes the reply) |
| `tests/snapshots/kinds__graph_editor`, `kinds__graph_editor_step` | second header line |
| help-overlay or key-reference frames that list `[graph_editor]` | `edit part` and the widened help |
| new `editor_part_form`, `editor_notes` | new |

### 8.5 Validate
```bash
git grep -n 'MOD-141 M2 T8 wires it'          # empty
cargo test -p htui --lib -- --test-threads=1 keys:: ui::graph_editor ui::tabs::settings::kinds
cargo test -p htui --features testkit --test keys_file --test no_hardcoded_keys --test kinds --test graph_editor -- --test-threads=1
cargo insta test -p htui --features testkit -- --test-threads=1     # review every changed editor, Kinds and help frame
cargo clippy --workspace -- -D warnings
```

### 8.6 Commits
1. `feat(MOD-141): M2 T8 graph_editor.open_part and the part-form stack`
2. `feat(MOD-141): M2 T8 the editor reads its settings; the header's limits line; Kinds routes the read`
3. `feat(MOD-141): M2 T8 part-form mode, focus-relative lists and hints, notes and markers`
4. `chore(MOD-141): M2 T8 drop the wave allows; docs/htui-keys.md; snapshots`

---

## T9 - End-to-end acceptance (PRD metrics for M2) (wave 2, serial)

**File set**: `crates/htui/tests/graph_editor.rs` and its snapshots.

Drivers are the existing ones: `analysis_bench` (`:477`), `bench_round`, `press`, `type_in`,
`bench_select`, `retyped` and `at_row`. Add these helpers:
- `fn m2_row(seed: &NodeBody, offset: usize) -> usize { 13 + seed.agents.len().max(1) + offset }`. The offsets are `deadline 0`, `palette 1`, `split 2`; compete `fan-out 3`, `judge 4`; divide `planner 3`, `planner agent 4`, `integrate 5`, `integrate agent 6`, `max parts 7`, `plan scope 8`, first part row `9`.
- `async fn pin_feat_1(store, graph)`: `update_item(ids::VULKAN_FEAT_1, item.version, ItemPatch { step_graph_id: Some(Some(graph)), author_id: ids::USER, reason: "edited".to_owned(), ..ItemPatch::default() })`.
- `async fn resolved(store) -> GraphSnapshot`: `htui_orch::graph::resolve(&store, &store, &item, RunMode::Manual, &store.app_settings().await?, None, ids::BOX)`, the shape of `:343-353`.
- `async fn with_candidates(bench, section, backend)`: give `research` and `verdict` one candidate each (`e`, `alt-n`, `enter`, round). The analysis steps have none (`kinds__graph_editor_step.snap`), and the box's two enabled agents leave candidate rung 3 ambiguous.

Tests:
1. `every_m2_node_field_round_trips` (table-driven like `every_m1_node_field_round_trips`, `:617-760`). It compares `shape(stored)` with the seed changed as expected:
   - deadline set (`"900"`) and cleared;
   - palette on an **unlinked** new step (via add);
   - split → divide, then planner persona, planner agent, integrate persona, integrate agent, max parts `3`, plan scope `parts`;
   - one part through the part form (key, title, brief, owned paths, acceptance, agent among the step's candidates, persona);
   - a second part, then reorder;
   - remove a part;
   - split back to none (the divide columns, parts and plan scope cleared);
   - with the switch on, split → compete with fan-out `3` and a judge pair.
2. `with_the_switch_off_a_compete_node_saves_byte_identical`:
   - The store makes `verdict` compete through `replace_graph` (fan-out 2, judge `(AGY, Some("gemini"))`, deadline 60).
   - The editor renames it; stored split, fan-out, judge and deadline are unchanged.
   - Research's split cycles never show `compete`.
   - Then, switch on via `SetSetting` (App, `CompeteFanOut`, true) and a fresh `analysis_bench`: fan-out edits to 3 and the judge cycles to the first pair; both stored.
3. `a_divide_step_with_one_part_saves_and_resolves`:
   - `with_candidates`; research → divide; one part `ui` (agent none); save.
   - `pin_feat_1(ANALYSIS)`; `resolved` is `Ok`.
   - The snapshot holds `research:plan` (planner) and `research` (integrate) with the part as a default part.
   - The editor frame shows no `!`.
4. `a_part_less_divide_shows_no_parts_and_still_saves`:
   - `with_candidates` (else resolve stops at `NoCandidate` first); research → divide, no part; save. A `ReplaceGraph` was served; stored `split == Divide`, `parts == []`.
   - The flattened frame contains `ResolveError::NotYetRunnable { node: "research".into(), reason: NotRunnable::NoParts }.to_string()` while the form is open again.
   - The canvas title reads `! research`.
   - Resolving after `pin_feat_1` answers that same error.
5. `a_saved_deadline_and_judge_reach_the_resolved_snapshot`:
   - Switch on; `with_candidates`.
   - Research deadline `900`; verdict → compete, fan-out 2, judge the first pair; save.
   - `pin_feat_1(ANALYSIS)`, `resolved`: research `deadline_seconds == Some(900)`; verdict `fan_out == 2` and `judge == Some(SnapshotJudge { agent_id: pair.0, model: Some(pair.1), .. })`.
6. `a_palette_step_is_added_with_no_edges`:
   - `a` with research selected, name `triage`, a template, palette on; save. Stored `triage.palette`, no edge touches it, `research -> verdict` is intact.
   - Then `e` on research, palette on, `enter`: the notice reads `palette_has_links(1)`'s words and no `ReplaceGraph` was served.
7. `the_header_shows_the_limits`:
   - At open, before the settings reply, the header reads `limits not read yet`.
   - After the round it reads the default line.
   - With `store.set_app_setting("max_fan_out", json!(2))` and a fresh bench, it reads `fan-out 2`.
   - Render at width 120: the line is 99 characters.

**Validate**:
```bash
cargo test -p htui --features testkit --test graph_editor -- --test-threads=1
```

**Commits**:
1. `test(MOD-141): M2 T9 every M2 node field round-trips; compete hidden while off`
2. `test(MOD-141): M2 T9 a saved divide resolves; notes, deadline and judge, palette, header`

---

## Milestone validation (after T9, on `hr/MOD-141`)

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings                  # featureless (memory)
cargo clippy --workspace --all-features -- -D warnings
(cd crates/htui-store && DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx cargo sqlx prepare --check)
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1
cargo test -p htui-store -- --test-threads=1             # pg_conformance at 277
cargo test -p htui --all-features -- --test-threads=1
bash scripts/gate.sh                                     # sandbox Qdrant timeouts owed on the host
```

---

## Open risks

1. **The judge FK name.** It is Postgres's default naming, not declared. The T1 `pg_constraint` test catches a mismatch. If it fails, use the name it reports in `phase_fk_refused`.
2. **Notes drift from resolve.** `notes` restates resolve's per-node checks. Mitigations:
   - unit tests compare each note with the constructed `ResolveError`;
   - T9 resolves a saved divide and a part-less one;
   - a save is never refused.
   Graph-wide refusals (`FanOutEdge`, `LocalConcurrent`, `ReviewFanOut`, `AgentCap`) and those that need the project rungs (a project-default `local`, candidate rungs 2-3) stay resolve's. The editor cannot see them.
3. **A part key holding `/`.** `divide::part_key` splits a node key at its last `/` (`divide.rs:51-55`), so a key such as `a/b` would be mis-read at run time. Neither the store nor the plan refuses it. Flag it in review; it is outside M2's settled scope.
4. **An open form keeps the switch it opened with.** If the settings read answers after the form opened, a compete node stays read-only until the form is reopened. T8 narrowed it (W4): the first read reaches an open form, so this holds for later reads only.
5. **The header line width.** At 99 characters the line is clipped in a narrow editor. The editor clips; it does not wrap.
6. **Snapshot churn** across the Prompt, editor and Kinds frames in T3, T5 and T8. Full insta runs, reviewed (memory).
7. **The settings key renumbering.** Only `SPECS[self as usize]` reads the discriminant. If anything else shows up in T3's run, stop and report.
8. **Disk and Postgres** with five lanes building at once. Check `df -h .` before fan-out. Only T3's one Postgres test runs in a lane.
