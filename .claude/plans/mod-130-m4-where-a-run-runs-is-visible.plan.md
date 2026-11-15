# Plan: MOD-130 M4 — Where a run runs is visible

**Source PRD**: `.claude/prds/mod-130-execution-targets.prd.md`
**Selected Milestone**: 4 — Where a run runs is visible
**Complexity**: Medium
**Status**: planned and fact-checked (2026-11-15); implementation not started.
**Routing**: plan path (PRD exists; M4 is its last milestone), no ultracode, lane shape (4 serial tasks, more than
five files) (`/handoff-run` in the `hr/MOD-130` sandbox, 2026-11-15). The plan runs without a confirm gate. No
architect: M4 adds no module seam; it widens one read model (`RunSummary`) and edits three render sites.

## Open questions

None. Every choice below has a default recorded under "Decisions this plan takes"; the one the maintainer might
want otherwise is **P3** (the queue overlay needs no code change).

## Summary

M1–M3 store *where* a run runs (`run.target_id`, `run.target_source`, `target_box_id`, the driver). M4 shows it,
read-only, on the surfaces that already show a run or a box:

- **Runs tab** (the item's detail): a line under each run's `kind status box started` / `mode … finished` pair:
  `runs on docker-here · from project`. Shown only when there is something to explain (P1).
- **Queue overlay**: the `Elsewhere` sentence names the **driving** box. M3 already made `EntryState::Elsewhere`
  carry the driver's hostname, so this milestone *pins* that with tests and fixes the stale docs (P3).
- **Settings › Boxes**: a child box is listed under its parent, indented, and its detail pane gains `kind` and
  `parent` rows (P4).

Data: `RunSummary` (the Runs read's row) gains `target_id`, `target_name`, `target_source`, read from `run` joined
to `target` in Memory, Postgres and the offline cache. **No migration and no cache migration**: every column
already exists (M1 `0040`, cache `0014`; `target` is a mirrored table).

## Shared decisions

Every task's brief carries this section.

### From the PRD

| D | What M4 does with it |
|---|---|
| D7 | A run's `target_id` and `target_source` are snapshots, written at enqueue and never updated. The display reads them as stored; a later cascade edit moves no shown run. |
| D2 | The seeded `local` target is `LOCAL_TARGET_ID`. A run on it with source `default` is today's run. |
| D12 | `box.kind` and `box.parent_box_id`; a child box has `parent_box_id = Some(machine)`. |
| D17 | A chat run has no target (`target_id` and `target_source` are `None`): nothing is shown for it. |
| D18 | Local behaviour is the regression bar: no existing snapshot of a local run moves. |

### Decisions this plan takes

- **P1 — when the target line shows.** `target_line(run, theme)` returns a line only when
  `run.target_id` is `Some` **and** not (`target_id == LOCAL_TARGET_ID` **and** `target_source == Default`). So:
  a chat run, a run queued before M2, and every plain local run read exactly as today (no snapshot of them moves);
  a local run a level *pinned* reads `runs on this box · from project` (the seeded local target is named `this box`, `seed::LOCAL_TARGET_NAME`); a child-box run reads
  `runs on docker-here · from item`. Text: `runs on {name} · from {source}` with `{source}` =
  `TargetSource::as_str()` (`item`, `workflow`, `project`, `workspace`, `settings`, `default`), clipped to `PANE`
  (43 cells) by `cells::fit` like the lines around it. The separator is the middle dot `\u{b7}` the Boxes pane
  already uses. A target whose row no longer answers (the mirror between passes) reads `runs on a removed target ·
  from {source}` rather than hiding the line.
- **P2 — the name comes from a join, not a second read.** `RunSummary.target_name` is `target.name` through
  `LEFT JOIN target t ON t.id = r.target_id` in the Pg and cache statements, and the `targets` map in Mem. A
  retired target still names itself (M1 keeps the row). `RunSummary.target_id` is carried so the renderer can
  test `LOCAL_TARGET_ID` without a string compare.
- **P3 — the overlay sentence is already right; M4 pins it.** `classify_entry` (M3) puts `driver_hostname` in
  `Elsewhere { hostname }`, and `sentence` prints `queued for {host}` / `running on {host}`. M4 adds no field to
  `EntryState` (it is matched in ~15 places, the PRD's own D8 keeps the driver's name as the one name) and adds a
  test whose run has a driver other than its target box, so the wording cannot drift back to the target's name.
  *Alternative the maintainer may ask for later:* `running on {driver} for {target}` when the two differ; it moves
  `EntryState::Elsewhere`, `Display`, `sentence`, `queue_overview` and `htui state`.
- **P4 — Boxes tree order and indent.** `BoxesSnapshot.boxes` is re-ordered where the snapshot is built: a
  machine, then its children (by hostname, then id), machines in the order the store gave. A child whose parent is
  not in the list (a read race, or a parent of another user) is listed after every machine, still with the
  indent, and the detail's `parent` row reads `(not listed)`. The list label of a child is prefixed
  `\u{2514} ` (`└ `), clipped as the hostname already is. Detail rows: `kind` (`machine` / `container`, `BoxKind::as_str`) after `host`, and `parent` (the parent's hostname, or `(not listed)`) for a child only.
- **P5 — surfaces left alone.** `htui stop`'s report keeps printing the driving box (it prints
  `COALESCE(executing, target)` already); the queue overview and `htui state` use `EntryState`'s `Display`,
  unchanged (P3); the execution-graph canvas draws steps, not placement. The Backlog's "runs on …" line is MOD-119
  and the setters are MOD-123/116/124 (PRD out of scope).
- **P6 — every struct-literal site of `RunSummary` is the foundation task's.** `RunSummary` has no `Default`.
  Fact-checked sites (2026-11-15): `crates/htui-core/src/model/run.rs:2170` (a test), `crates/htui-core/src/store/mem.rs:2410`,
  `crates/htui-orch/src/closeout.rs:434`, `crates/htui-orch/src/command.rs:3039` (a test),
  `crates/htui-store/src/cache/read.rs:917`, `crates/htui-store/src/pg/rows.rs:76`,
  `crates/htui/src/ui/tabs/backlog/detail/runs.rs:3414, :3459, :7734` (tests; `copied_run` and `stepless_run` are
  helpers to re-read) and `.../runs/execution_graph.rs:845`. T1 owns all of them and ends on
  `cargo check --workspace --all-targets --all-features`, so a site this list missed is found there and not by a
  later task.
- **P7 — `.sqlx`.** `PgStore::runs`'s `query_as!` text changes, so its `.sqlx` entry is replaced. Run
  `cargo sqlx prepare` against a **migrated scratch database** on `localhost:5439` (the sandbox's), per
  `docs/hr-sandbox.md`; the empty `htui` database cannot prepare. The old entry is deleted, the new one added.

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Read-model field + three store bodies | `RunSummary` (`crates/htui-core/src/model/run.rs:1780`), `State::run_summaries` (`mem.rs:2402`), `PgStore::runs` (`pg/read.rs:430`) + `RunRow::into_summary` (`pg/rows.rs:75`), `CacheStore::runs` (`cache/read.rs:813`) | One field, three bodies, one conformance case that runs on Mem and Pg. |
| Joined display name | `COALESCE(b.hostname, '') AS box_hostname` through `LEFT JOIN box b` in the same two SQL statements | Same join shape for `target`. |
| Run header lines | `run_lines` (`runs.rs:1510`), `mode_line`, `cells::fit(.., PANE)` | A conditional extra line pushed like `cancel_requested` and `failure`. |
| Source label | `TargetSource::chosen_by` (`model/target.rs`) | A `const fn` on the enum beside the existing one, if a label is needed beyond `as_str`. |
| Boxes rows | `list_label` (`settings/boxes.rs:1314`), `detail` (`:1097`) with `read(name, value)` | Rows appended to the `fields` vec; `list_label` keeps the suffix and clips only the hostname. |
| Overlay test | `every_state` (`ui/overlay/queue.rs:928`) | One test per `EntryState` sentence. |

## Tasks

Serial on one checkout, in this order. T1 is a **foundation** task (T2 reads what it adds): it gets the one
read-only verifier. T2–T4 go to the gate and the review unverified.

### T1 — `RunSummary` names its target (foundation)

- **Files owned** (touched-file set for the independence test):
  `crates/htui-core/src/model/run.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-core/src/store/conformance.rs`, `crates/htui-store/src/pg/read.rs`,
  `crates/htui-store/src/pg/rows.rs`, `crates/htui-store/src/cache/read.rs`,
  `crates/htui-store/.sqlx/` (one entry replaced), the conformance count pins in
  `crates/htui-core/tests/it/mem_store.rs` and `crates/htui-store/tests/it/pg_conformance.rs`,
  `crates/htui-store/tests/it/cache.rs`, and the literal sites of P6
  (P6: `crates/htui-orch/src/closeout.rs`, `crates/htui-orch/src/command.rs`,
  `crates/htui/src/ui/tabs/backlog/detail/runs.rs` (tests only), `.../runs/execution_graph.rs`, others the compiler names).
- **Types.** `RunSummary` gains, after `target_box_id`:
  `target_id: Option<TargetId>`, `target_name: Option<String>`, `target_source: Option<TargetSource>`, each
  documented (`run.target_id`, `target.name` or `None`, `run.target_source`). `RunRow` gains the same three
  (`target_name` as `"target_name?"`, `target_source` as `"target_source?: TargetSource"`,
  `target_id` as `"target_id?: TargetId"`).
- **Behaviour.**
  - `State::run_summaries` reads the name from the `targets` map by `run.target_id`.
  - `PgStore::runs` adds `r.target_id, r.target_source, t.name AS "target_name?"` and
    `LEFT JOIN target t ON t.id = r.target_id`; `CacheStore::runs` the same through its dynamic SQL
    (`target` is mirrored; no cache migration).
  - A chat run and a pre-M2 run answer `None` for all three.
- **Tests (TDD, first).**
  - A conformance case `runs_name_their_target_and_source` in `crates/htui-core/src/store/conformance.rs`, added
    to `CASES`: a run enqueued on a child-box target with source `project` reads
    `(Some(target), Some("<name>"), Some(Project))`; a run on the local default reads
    `(Some(LOCAL_TARGET_ID), Some("local"), Some(Default))`; a chat run reads `(None, None, None)`; a retired
    target still names itself. Move the `CASES` count pins by one.
  - A cache test beside `the_mirror_projects_a_malformed_trim_record_like_postgres` (`cache.rs:546`): the offline
    `runs` answers the same triple off the mirror.
- **Check.** `cargo nextest run -p htui-core -p htui-store --all-features -E 'test(runs_name_their_target)'`, then
  `cargo check --workspace --all-targets --all-features` (finds every literal site; not the gate), then
  `cargo clippy -p htui-core -p htui-store --all-features -- -D warnings` and `cargo fmt -p htui-core -p htui-store`.

### T2 — The Runs tab names the target

- **Depends on** T1.
- **Files owned.** `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, and the snapshots the full insta run moves
  (expected: none, by P1; any that move are read, not accepted blindly).
- **Types.** `fn target_line(run: &RunSummary, theme: &Theme) -> Option<Line<'static>>` beside `run_lines`;
  `run_lines` pushes it after the `mode_line` and before the cancel and failure lines, so a failed run reads
  header, mode, target, failure. It is the one function the list and `flow_head` share (both call `run_lines`).
- **Behaviour.** P1 exactly. Text `runs on {name} · from {source}`, `theme.dim`, `cells::fit(.., PANE)`.
- **Tests.** In the file's `mod tests`: a local default run adds no line (the existing
  `the_run_lines_keep_the_run_grid` stays unedited); a pinned local run, a child-box run, a long target name
  (clipped to 43 cells), a missing target row, a chat run, and a failed run (line order). One insta snapshot of the
  list with a child-box run, one of the flow head.
- **Check.** `cargo nextest run -p htui --features testkit -E 'binary(it) and test(/^backlog::/)'` for the
  snapshots, plus `cargo nextest run -p htui --features testkit -E 'test(/detail::runs::tests::/)'` for the unit
  tests (read the names that run); then `cargo clippy -p htui --all-features -- -D warnings` and
  `cargo fmt -p htui`.

### T3 — Settings › Boxes shows a child under its parent

- **Depends on** nothing in T1/T2 (its files are disjoint from theirs); runs after T2 only because the lane is
  serial and both can move snapshots under `crates/htui/tests/it/snapshots/`.
- **Files owned.** `crates/htui/src/box_settings.rs` (the ordering, where `BoxesSnapshot` is built),
  `crates/htui/src/ui/tabs/settings/boxes.rs` (`list_label`, `detail`), the snapshots a full insta run moves.
- **Types.** `fn tree_order(boxes: Vec<BoxRecord>) -> Vec<BoxRecord>` in `box_settings.rs` (pure), applied once
  where the snapshot is assembled so the section's cursor, selection and `selected_record` keep indexing the one
  `snapshot.boxes` order. `list_label` prefixes `\u{2514} ` for a row with `parent_box_id.is_some()`; the prefix
  is part of what survives clipping (like the `(this box)` suffix).
- **Behaviour.** P4 exactly. A snapshot with no child box reads byte-for-byte as today.
- **Tests.** `tree_order`: machines only (unchanged order), one parent with two children (children follow, by
  hostname), a child of an unlisted parent (last, `(not listed)`), two machines each with a child. `list_label`:
  prefix, clipping beside a long hostname and `(this box)`. `detail`: the `kind` row for both kinds and the
  `parent` row for a child only. One insta snapshot of the Boxes pane with a parent and its container.
- **Check.** `cargo nextest run -p htui --features testkit -E 'test(/settings::boxes::/) or test(/box_settings::/)'`
  plus `-E 'binary(it) and test(/^box_settings::/)'` (the substring rule: read the names that run), then clippy
  and fmt on `htui`.

### T4 — The overlay sentence is pinned, stale docs follow M3

- **Files owned.** `crates/htui/src/ui/overlay/queue.rs` (test only), `crates/htui-core/src/model/queue.rs`
  (doc comments only: `EntryState`'s "MOD-43 owns choosing; M3 reports", and the `Elsewhere` doc), and
  `crates/htui-core/src/model/run.rs` (the `Run::target_box_id` doc if it still says "executes only when local").
  No behaviour moves.
- **Tests.** In `ui/overlay/queue.rs`: `sentence(&Elsewhere { hostname: Some("driver"), status: Queued, .. })` is
  `queued for driver` and `Running` status reads `running on driver`; with `hostname: None` it reads `another
  box`. A classify-then-sentence test over a `QueueRunFact` whose `target_hostname` and `driver_hostname`
  differ asserts the sentence names the driver and not the target.
- **Check.** `cargo nextest run -p htui --features testkit -E 'test(/overlay::queue::/)'` and
  `-p htui-core -E 'test(/model::queue::/)'`; `cargo doc -p htui-core --no-deps` is part of the gate's `doc` step:
  run `bash scripts/gate.sh --only doc` once after the doc edits (lane checks miss rustdoc links).

## Files to change

| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/run.rs` | UPDATE | `RunSummary` fields (T1); stale `target_box_id` doc (T4) |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `run_summaries` reads the target name (T1) |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | `runs_name_their_target_and_source` case (T1) |
| `crates/htui-store/src/pg/{read,rows}.rs` | UPDATE | join `target`, carry three fields (T1) |
| `crates/htui-store/src/cache/read.rs` | UPDATE | the same off the mirror (T1) |
| `crates/htui-store/.sqlx/query-*.json` | UPDATE | one entry replaced (T1) |
| `crates/htui-core/tests/it/mem_store.rs`, `crates/htui-store/tests/it/{pg_conformance,cache}.rs` | UPDATE | count pins and the offline test (T1) |
| `RunSummary` literal sites (P6) | UPDATE | the three new fields |
| `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | `target_line` (T2) |
| `crates/htui/src/box_settings.rs`, `crates/htui/src/ui/tabs/settings/boxes.rs` | UPDATE | tree order, indent, rows (T3) |
| `crates/htui/src/ui/overlay/queue.rs`, `crates/htui-core/src/model/queue.rs` | UPDATE | pinning tests, doc comments (T4) |
| `crates/htui/tests/it/snapshots/*.snap` | UPDATE | new snapshots for T2 and T3 only |

## Verified claims (step 3.5, 2026-11-15)

| Claim | Verdict | Evidence |
|---|---|---|
| `run.target_id` / `run.target_source` exist in Pg and the cache; `target.name` exists in both | confirmed | `0040_execution_targets.sql:25-27, :63-66`; `cache_migrations/0014_execution_targets.sql:10-17` (cache `target` has `name`) |
| `target` is a mirrored table (no cache migration needed) | confirmed | `MIRRORED_TABLES` (25 entries) lists `"target"`, `cache/mod.rs:51` doc |
| `RunSummary` has no `Default`; its literal sites are those of P6 | amended | The first draft listed `htui-store/tests/it/cache.rs:546` as a literal site; it is a reference, not a literal (struck). `closeout.rs:434`, `mem.rs:2410`, `rows.rs:76`, `cache/read.rs:917`, `command.rs:3039`, `run.rs:2170`, `runs.rs:3414/:3459/:7734`, `execution_graph.rs:845` confirmed by search; T1's workspace `cargo check` is the backstop |
| The conformance count pin is 358 on this branch | confirmed | `crates/htui-core/tests/it/mem_store.rs:65`, `crates/htui-store/tests/it/pg_conformance.rs:82` (`EXPECTED_CASES`). T1's new case moves both to 359 |
| `EntryState::Elsewhere.hostname` is the driver's (P3) | confirmed | `classify_entry` (`model/queue.rs:478`) builds `hostname: run.driver_hostname.clone()`; `sentence` prints it (`ui/overlay/queue.rs:295`) |
| `BoxRow.kind` / `parent_box_id` exist; `BoxKind` is `machine`/`container` | confirmed | `model/box_.rs:27-31` and the `BoxRow` fields; no kit label function exists, so P4 uses `BoxKind::as_str` (amended) |
| `BoxesSnapshot` is assembled in one place | confirmed | the only non-test literal is `crates/htui/src/box_settings.rs:74`; `settings/boxes.rs:1522` is a test helper; the `tests/it/box_settings.rs` snapshots hold no child box |
| The sandbox can run `cargo sqlx prepare` | confirmed | `docs/hr-sandbox.md:227-236` (`CREATE DATABASE htui_sqlx`, `migrate run --source migrations` from `crates/htui-store`, `prepare -- --all-targets --all-features`) |
| Task independence | confirmed serial | T1 and T2 both touch `runs.rs` (T1 its test literals), T1 and T4 both touch `model/run.rs`, T2 and T3 share the snapshots directory: no task is marked parallel, the lane is serial |

## Validation

```bash
cargo nextest run -p htui-core -p htui-store --all-features -E 'test(runs_name_their_target)'
cargo nextest run -p htui --features testkit -E 'binary(it)'   # after T2 and T3, read the snapshot diffs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                          # featureless gate
bash scripts/gate.sh                                             # once, by the main thread
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A `RunSummary` literal site is missed and a later task halts at its first workspace check | Medium | T1 ends on `cargo check --workspace --all-targets --all-features` (P6) |
| The new `query_as!` text needs `.sqlx` regeneration and the scratch DB is not migrated | Medium | P7; prepare against a migrated scratch database, never the empty `htui` one |
| A snapshot of a local run moves | Low | P1 hides the line for the plain local default; any moved `.snap` is read, not accepted |
| Reordering `snapshot.boxes` breaks the Boxes cursor and selection tests | Low | The order is fixed once where the snapshot is built; `selected_record` and moves still index the one vector |
| The offline cache's `target` mirror lags a run | Low | P1's `a removed target` wording instead of a hidden line |
| The overlay should name both the driver and the target | Low | P3 records the alternative and its blast radius |

## Acceptance

- [ ] `RunSummary` carries target id, name and source on Mem, Pg and the cache, with one conformance case on both stores.
- [ ] The Runs tab names the target and level for any run that is not a plain local default, and for none other.
- [ ] Settings › Boxes lists a child box under its parent and its detail names kind and parent.
- [ ] The queue overlay's `Elsewhere` sentence is pinned to the driving box by tests.
- [ ] No migration; no existing local-run snapshot moved; the gate is green but for known flakes.
- [ ] The PRD milestone row 4 is `complete` with the commit range, and `HANDOFF.md` carries the M4 phase note
      (the item closes: all four milestones are done; close-out per `lifecycle.md` P2).
