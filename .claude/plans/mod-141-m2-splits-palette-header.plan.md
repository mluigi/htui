# Plan: MOD-141 M2 - Splits, palette, deadline and the graph header

**Source PRD**: `.claude/prds/mod-141-step-graph-editor.prd.md`
**Selected Milestone**: 2 - Splits, palette and graph header
**Complexity**: Large
**Routing**: PRD path, ultracode for implement and review (accepted 2026-11-10, re-confirmed 2026-11-11).

## Summary

The step form stops showing the v2 node fields read-only and starts editing them. Those fields are split
(none, divide, and compete behind a new off-by-default switch), fan-out, the per-step judge, the step
deadline, the palette flag and plan scope. A divide node also gets its planner and integrate columns, its
part cap and its default parts list. The deadline and judge columns exist in Postgres since `0003`, but
`StepGraphPhase` never carried them, so M2 widens the model and the stores. Resolve then reads the phase
first for both. The graph header gains a read-only line of the run limits that resolve freezes. Making a
graph tighten those limits, and the `runtime_changes` override, move to the new MOD-171. The stale
"runs once MOD-139 lands" note goes. Instead, the form and the canvas carry resolve's own sentence for
anything a run would refuse today. That is a warning, never a refused save.

## Maintainer decisions (2026-11-11, at planning)

| # | Question (PRD open question) | Decision |
|---|---|---|
| M-1 | Header bounds and the graph's `runtime_changes` override: storage or migration? | Neither in M2. Storage has no per-graph column. The bounds are global `app_setting` keys, not the project rung ANA-33 §7.8 names, and nothing reads `runtime_changes` until MOD-140. M2's header shows the effective limits read-only. Per-graph tightening and the override are **MOD-171** (migration on `step_graph`, resolve takes the minimum, MOD-140 reads the override). |
| M-2 | Where the compete toggle lives; who honours it | A new App-rung boolean `SettingKey` `compete_fan_out`, off by default, unseeded. It is edited in `Settings > Prompt`'s app group beside `memory_capture`. **Only the editor reads it**; the engine runs compete nodes as today (`R-LATER-13`: compete code stays). |
| M-3 | How far the body widens | `deadline_seconds`, `judge_agent_id` and `judge_model` all join `StepGraphPhase`. `replace_graph` writes them, so M1's "undoing a delete loses deadline and judge" limit (M1 D6) goes; skill bindings are still lost. Resolve reads the phase rung first for the deadline and the judge. The judge is edited only on a compete node. |
| M-4 | What resolve does with a divide node before MOD-140 | MOD-139 made divide runnable. Resolve still refuses a planner agent, a part-less divide, a plan scope beyond `parts` and palette nodes (`NotRunnable`), and default parts that admission would refuse (`DivideRefused`). The editor **warns with resolve's own sentence and saves anyway**; it never refuses. |

## Decisions (D)

- **D1 Model widening.** `StepGraphPhase` gains `deadline_seconds: Option<i32>`, `judge_agent_id:
  Option<AgentId>` and `judge_model: Option<String>`, each `#[serde(default)]`. Every Postgres phase read
  and write carries them: `phase_row`, `phase_rows_on`, `insert_phase`, `update_kept_phase`,
  `update_phase` and its `written` row, regenerated through `.sqlx`. `MemStore` stores them whole. The
  `replace_graph` doc drops the F14 paragraph: a kept node now takes the body's values, and a new node takes
  them too.
- **D2 Rules mirror the CHECKs.** `phase_row_refusal` adds two checks after the token budget, each with a
  new sentence in `traits.rs`. The first is `ck_phase_judge_model`, a `judge_model` with no
  `judge_agent_id`. The second is the `0003` deadline CHECK, `deadline_seconds < 1`. `body_agent_refs` adds
  `step_graph_phase.judge_agent_id` after `integrate_agent_id`, before the candidates. `MemStore`'s
  `create_phase` agent references and any agent-delete guard that counts phase agent columns count the
  judge too, so `MemStore` never stores what the `REFERENCES agent(id)` foreign key refuses.
- **D3 Resolve reads the phase rung.** In `snapshot_phase`, the deadline chain is phase (`> 0`), then
  project, then app, then 2 h. The judge chain is the phase's `judge_agent_id` with its `judge_model`, then
  the project's `judge_agent_id` with model `None`, then none (human selection). A row whose new columns are
  NULL resolves exactly as before, so no existing snapshot or topology digest changes and `GraphSnapshot::V`
  stays. The H-14 comments go.
- **D4 Shared run bounds.** `htui_orch::graph::run_bounds(app) -> RunBounds` returns `max_fan_out`,
  `max_agents_per_run`, `max_parallel_sessions`, `max_plan_revisions`, `max_child_runs` and
  `runtime_changes` over the same keys and code defaults resolve uses today. `resolve_or_refused` builds
  `SnapshotSettings`' six fields from it, a pure extraction that the existing bound tests pin. The editor
  shows exactly what a run would freeze.
- **D5 Compete switch (M-2).** `SettingKey::CompeteFanOut` takes the key `compete_fan_out`, kind Boolean,
  rung App only, default off. It slots in at its key byte position, which renumbers the later
  discriminants. `resolve_compete_fan_out(app) -> bool` mirrors `resolve_memory_capture`. `SettingKey::ALL`
  goes from 13 to 14, and the app rung from 11 entries to 12. `Settings > Prompt` lists it in the app group
  with the switch's words.
- **D6 Editor settings read.** `StoreRequest::GraphEditorSettings` answers
  `StoreReply::GraphEditorSettings(Box<EditorSettings { bounds: RunBounds, compete: bool }>)` from one
  `app_settings()` read. The catalogue's `REQUEST_NAMES` goes from 12 to 13. The editor sends it with
  `Agents` and `Templates` on open and on activation. Until it answers, compete is treated as off and the
  limits read "not read yet".
- **D7 Split.** The split choice offers none and divide, plus compete when the switch is on. Leaving divide
  clears the seven divide columns, the parts and `plan_scope`. Entering compete sets fan-out to 2. Leaving
  compete sets fan-out to 1 and clears the judge. Entering divide clears the judge. **Switch off on a
  compete node**: split, fan-out and judge are read-only and are kept byte for byte on save.
- **D8 Compete rows.** The node shows fan-out as text, a whole number of at least 2, and the judge as a
  choice of "project default" or each agent·model pair (from `agent_pairs`).
- **D9 Divide rows.** Planner persona and integrate persona are persona choices. Planner agent·model and
  integrate agent·model are choices of none or a pair. Max parts is text, empty for none or a whole number
  of at least 1. Plan scope is a choice of none, parts, successors or graph. The default parts are listed as
  `key · title` rows, with a placeholder when there are none. A part opens the **part form**: key (required,
  unique in the node), title, brief, owned paths (comma separated), acceptance (empty for none), agent·model
  (none or one of the node's own candidates, since admission refuses any other agent) and persona (with the
  MOD-140 note). On a node that is not divide but has a stored `plan_scope`, plan scope is read-only and
  kept.
- **D10 Deadline and palette rows.** The deadline is text in seconds, empty for the project's deadline,
  otherwise a whole number of at least 1. Palette is a toggle. A new step saved with palette on is added
  with no edges and takes over nothing. Turning palette on for a step that has links is refused in words
  ("a palette step has no links; delete its N links first"), because the store refuses a palette node with
  edges.
- **D11 Warnings (M-4).** `notes(node, max_fan_out)` lists, in resolve's order, every sentence a run of this
  node would be refused with today. The `NotRunnable` texts cover `PlannerAgent`, `NoParts`, `PlanScope` and
  `Palette`. The default-part checks use `RunFailure`'s text: `LocalCannotDivide` (only when the step's own
  isolation is `local`), `PlanOverCaps` for `max_parts` and `max_fan_out`, `PartPersonaUnsupported`,
  `PartPathRefused` (through `divide::owned_path_refusal`, made `pub`) and `PartAgentNotCandidate`. A
  compete fan-out above `max_fan_out` is noted too. The form shows them under its rows. The canvas marks a
  noted node with `!`. A save still goes through. Known limit: `max_agents_per_run` needs the whole graph's
  planned sessions, so it is left to resolve.
- **D12 Canvas.** The divide badge reads `divide · N parts`, with no MOD-139 note; compete `×N` and palette
  are unchanged. `Canvas::set_warned` takes the noted node ids.
- **D13 Header.** A second header line: `limits · fan-out 4 · agents 8 · plan revisions 3 · parallel 4 ·
  child runs 8 · run-time changes ask`. It is read-only and its numbers come from D6. Nothing about it is
  editable in M2 (M-1).
- **D14 Keys (`R-TUI-10`).** The four list acts (`candidate_add`, `candidate_remove`, `candidate_up` and
  `candidate_down`, chords `alt-n`/`alt-d`/`alt-up`/`alt-down`) work on the focused list: candidates or
  parts. Their key-file names stay and their catalogue descriptions widen. The hint row names the focused
  list's noun ("add agent", "add part"), because hints are the editor's own `HintSpec` constants. One new act
  opens the focused part's form. Its default chord is settled in T8 against
  `no_two_actions_in_one_context_share_a_default_chord`, `alt-e` first. A new stack,
  `GRAPH_EDITOR_PART_FORM`, has the field, pick, form-field and modal layers. Every file commit `9c267f92`
  touched is updated, and `docs/htui-keys.md` is regenerated.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Body-carried phase columns | `crates/htui-store/src/pg/write.rs:1593-1639` `update_kept_phase`; `:1462` `insert_phase` | One `sqlx::query!` per write naming every carried column; `.sqlx` regenerated, never hand-edited |
| Row rules mirroring CHECKs | `crates/htui-core/src/store/graph_rules.rs:29-57`, sentences `traits.rs:3518-3551` | One `pub fn` sentence per CHECK; `MemStore` refuses what Postgres refuses, in one order |
| Resolve rung chains | `crates/htui-orch/src/graph.rs:1670-1699`; tests `:2378-2532` | `phase.or(project).or(app).unwrap_or(default)`; one test per rung answering |
| App-rung switch | commits `80bec1df` (core registry), `29571b61` (Settings > Prompt) | `memory_capture`: registry slot in key byte order, a `resolve_*` fn, the app group row, snapshots |
| Node form rows | `crates/htui/src/ui/graph_editor/node_form.rs:448-551`, `:375-430` | Kit `Form` rows; key-agnostic methods; `build` refuses in row order with `pub(crate)` constant sentences |
| Sub-list in a form | `node_form.rs:323-367` candidates | Add, remove and move renumber positions; a placeholder row while empty |
| Editor modes and stacks | `crates/htui/src/ui/graph_editor/mod.rs:283-310`, `keys/views.rs:411-469`; commit `947a941e` | One `Mode` variant, one stack and one `HintSpec` per mode |
| Store worker read | `crates/htui/src/catalogue.rs` `REQUEST_NAMES`; M1 T1's `GraphBody` request | Request and reply variants, `try_serve` arm, name pin, a worker test over `MemStore::demo()` |
| Tests | `crates/htui/tests/graph_editor.rs`, `crates/htui-core/src/store/conformance.rs:25740` `replace_graph_round_trips_every_node_field` | End to end over `Harness::over(MemStore::demo())`; conformance per store |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/model/kind.rs` | UPDATE | three phase fields (D1) | T1 |
| `crates/htui-core/src/store/{graph_rules,traits,mem,conformance}.rs` | UPDATE | rules, sentences, the mem store, conformance cases (D1, D2) | T1 |
| `crates/htui-core/src/{seed,fixtures}.rs` | UPDATE | phase literals | T1 |
| `crates/htui-store/src/pg/{read,write}.rs`, `crates/htui-store/.sqlx/*` | UPDATE | phase reads and writes (D1) | T1 |
| every other `StepGraphPhase { .. }` literal (htui-orch, htui-store tests, htui-worker tests, htui) | UPDATE | the new fields | T1 |
| `crates/htui-orch/src/divide.rs` | UPDATE | `owned_path_refusal` made `pub` (D11) | T1 |
| `crates/htui/src/ui/graph_editor/mod.rs` | UPDATE | `mod notes; mod part_form;` lines | T1 (end) |
| `crates/htui/src/ui/graph_editor/{notes,part_form}.rs` | CREATE | module docs only | T1 (end) |
| `crates/htui-orch/src/graph.rs` | UPDATE | phase rung (D3), `run_bounds` (D4) | T2 |
| `crates/htui-core/src/prompt/settings.rs`, `crates/htui/src/prompt_settings.rs`, `crates/htui/src/ui/tabs/settings/prompt.rs`, `crates/htui/tests/prompt_settings.rs`, `crates/htui/tests/snapshots/prompt_settings__*.snap` | UPDATE | the compete switch (D5) | T3 |
| `crates/htui/src/ui/graph_editor/canvas.rs` + its snapshots | UPDATE | badges, `set_warned` (D12) | T4 |
| `crates/htui/src/ui/graph_editor/node_form.rs`, `part_form.rs` + snapshots; the `NodeForm` call sites in `mod.rs` | UPDATE | M2 rows and the part form (D7-D10) | T5 |
| `crates/htui/src/ui/graph_editor/model.rs`, `notes.rs` | UPDATE | palette add, palette refusal, warnings (D10, D11) | T6 |
| `crates/htui/src/store_worker.rs`, `crates/htui/src/catalogue.rs`, `crates/htui/tests/kinds.rs` | UPDATE | `GraphEditorSettings` (D6) | T7 |
| `crates/htui/src/ui/graph_editor/mod.rs`, `crates/htui/src/keys/{catalogue,views,stack,load}.rs`, `crates/htui/tests/keys_file.rs`, `docs/htui-keys.md`, editor snapshots | UPDATE | orchestration, header, part-form mode, keys (D6, D11-D14) | T8 |
| `crates/htui/tests/graph_editor.rs` | UPDATE | M2 acceptance | T9 |

## Tasks

TDD throughout: each task writes its failing tests first. New items that are unused until T8 carry
`#[allow(dead_code, reason = "MOD-141 M2 T8 wires it")]`. It is `allow` and not `expect`, because a test
build uses them, and T8 removes every one (grep the reason). The bare clippy run in `scripts/gate.sh`
refuses dead code otherwise.

### Wave 0 (serial, primary checkout)

#### Task 1: Phase model widening (D1, D2), then the two module stubs
- **Action**: The three fields, then every literal and every Postgres phase query. Add the rules and
  sentences and the `body_agent_refs` change. The `MemStore` writes and agent guards count the judge.
  Conformance gets these cases:
  - every node field round-trips, the three included, on a kept node and on a new one;
  - `judge_model` without a judge agent is refused;
  - `deadline_seconds 0` is refused;
  - a judge agent naming no row is refused, in its D2 position;
  - an override clone keeps the deadline and the judge.

  Then run `cargo sqlx prepare` against a migrated scratch database (`docs/hr-sandbox.md`, memory: sqlx
  prepare needs a migrated scratch DB), make `divide::owned_path_refusal` `pub`, and add the `notes`/`part_form`
  stubs. Update the conformance count pins this moves.
- **Mirror**: `update_kept_phase`, `phase_row_refusal`, `replace_graph_round_trips_every_node_field`.
- **Validate**: `cargo check --workspace --all-targets --all-features`; `cargo test -p htui-core
  --all-features --lib store:: -- --test-threads=1`; the Postgres conformance run against
  `$HTUI_TEST_DATABASE_URL`; `cargo sqlx prepare --workspace --check`.

### Wave 1 (parallel, one worktree each under `target/wt/`, branched from T1)

#### Task 2: Resolve reads the phase rung; run bounds (D3, D4)
- **Action**: The deadline and judge chains. Extract `RunBounds` and `run_bounds` and build
  `SnapshotSettings` from them. Tests: the phase deadline answers first; a phase judge carries its model; a
  NULL phase falls through (existing tests stay green); `run_bounds` defaults and parsed values.
- **Validate**: `cargo test -p htui-orch --all-features --lib graph:: -- --test-threads=1`.

#### Task 3: Compete switch (D5)
- **Action**: The registry variant and its spec, `resolve_compete_fan_out`, the app row in
  `Settings > Prompt`, and the count pins (13 → 14, 11 → 12). Review every changed snapshot.
- **Validate**: `cargo test -p htui-core --lib prompt::settings`; `cargo test -p htui --features testkit
  --test prompt_settings -- --test-threads=1`; `cargo insta test -p htui --features testkit`.

#### Task 4: Canvas badges and the warning marker (D12)
- **Action**: `divide · N parts`, `set_warned` and the `!` marker, plus snapshots: a divide with parts, a
  noted node and a palette band.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::canvas`.

#### Task 5: Step form M2 rows and the part form (D7-D10, D14's list half)
- **Action**: Rows, the split transitions, the switch-off read-only compete rows, the focus-relative list
  methods and the part form (key-agnostic methods, refusals as `pub(crate)` constants). `NodeForm` gains a
  `compete: bool` argument. Its `mod.rs` call sites pass `false` until T8, and T5 is the only wave-1 task
  that touches `mod.rs`. `DIVIDE_NOTE` goes. Snapshots: a divide form, a compete form with the switch on and
  off, and the part form.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::node_form ui::graph_editor::part_form`.

#### Task 6: Palette add and the warnings (D10, D11)
- **Action**: In `model.rs`, add a palette step with no edges, and refuse palette on a linked step in words.
  In `notes.rs`, the D11 list in resolve's order. Its unit tests assert each sentence equals the
  `NotRunnable`/`RunFailure` display it borrows.
- **Validate**: `cargo test -p htui --lib ui::graph_editor::model ui::graph_editor::notes`.

**Independence check (step 3.5):**
- T2 = {`htui-orch/src/graph.rs`}
- T3 = {`htui-core/src/prompt/settings.rs`, `htui/src/prompt_settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/tests/prompt_settings.rs`, `htui/tests/snapshots/prompt_settings__*`}
- T4 = {`graph_editor/canvas.rs`, its snapshots}
- T5 = {`graph_editor/node_form.rs`, `graph_editor/part_form.rs`, their snapshots, the `NodeForm` call sites in `graph_editor/mod.rs`}
- T6 = {`graph_editor/model.rs`, `graph_editor/notes.rs`}

All pairwise intersections are empty. T1 writes `graph.rs`, `mod.rs` and the stubs before the wave starts.
Insta names unit snapshots by module path, so no two tasks share a `.snap`. There are no migrations, and
`.sqlx` belongs to T1 alone (T2 adds no query). Build coupling: T3 changes `htui-core`, which every htui
lane compiles, so each lane runs in its own worktree. The branches merge serially into `hr/MOD-141`
(memory: workflow worktree implementers). Disk is 2.6 T free on sda; check `df -h .` again before
launching.

### Wave 2 (serial, primary checkout)

#### Task 7: Editor settings read (D6)
- **Action**: Add the request and reply, the `try_serve` arm and `REQUEST_NAMES` 13. Worker tests over
  `MemStore::demo()`: the code defaults with no rows, set values for all six bounds, and `compete_fan_out`
  on and off.
- **Validate**: `cargo test -p htui --features testkit --test kinds -- --test-threads=1`;
  `cargo test -p htui --lib catalogue`.

#### Task 8: Editor orchestration and keys (D6, D11-D14)
- **Action**:
  - `Lookups` gains its settings, and open and activation ask for them.
  - The header's limits line.
  - The part-form mode, its stack and its open act.
  - Focus-relative hints.
  - Notes in the form panel and the canvas markers.
  - The palette path in `node_write`.
  - The `compete` argument from the settings.
  - Remove every T-wave `dead_code` allow.
  - Key pins, `docs/htui-keys.md`, and a full insta run (review every changed editor and Kinds frame).
- **Validate**: `cargo test -p htui --lib keys:: -- --test-threads=1`; `--test keys_file`;
  `--test no_hardcoded_keys`; `--lib ui::graph_editor`; `cargo insta test -p htui --features testkit`.

#### Task 9: End-to-end acceptance (PRD metrics for M2)
- **Action**: Tests in `tests/graph_editor.rs`:
  - Every M2 node field round-trips through the editor (split, fan-out, judge, deadline, palette, plan
    scope, the divide columns, the parts).
  - With the switch off, a compete node saves byte-identical. With it on, its fan-out and judge edit.
  - A divide node with one part saves and resolves (`htui_orch` resolve over the same `MemStore`).
  - A part-less divide shows the `NoParts` sentence and still sends `ReplaceGraph`.
  - A saved deadline and judge reach the resolved snapshot.
  - A palette step is added with no edges.
  - The header shows the limits.
- **Validate**: `cargo test -p htui --features testkit --test graph_editor -- --test-threads=1`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings                  # featureless (memory: featureless clippy gate)
cargo clippy --workspace --all-features -- -D warnings
cargo sqlx prepare --workspace --check                   # scratch DB migrated first
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1
bash scripts/gate.sh                                     # full gate; sandbox Qdrant timeouts owed on the host
```

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `StepGraphPhase` lacks `deadline_seconds`, `judge_agent_id` and `judge_model`; the columns exist | true | `kind.rs:501-571`; `0003_orchestration.sql:17-22` |
| `ck_phase_judge_model` and the deadline `> 0` CHECK exist; `phase_row_refusal` mirrors neither | true | `0003:19-22`; `graph_rules.rs:29-57` |
| `body_agent_refs` omits the judge agent; the column references `agent(id)` | true | `graph_rules.rs:276-303`; `0003:17` |
| `replace_graph` keeps the three columns on a kept node and NULLs them on a new one (F14) | true | `traits.rs:1100-1104`; `pg/write.rs:1590-1606` |
| Resolve starts the deadline and judge chains at the project rung | true | `htui-orch/src/graph.rs:1670-1699` |
| A NULL phase column leaves the snapshot, and so the digest, unchanged | true | The chains fall through to today's value; `topology` serialises `SnapshotPhase` only (`graph.rs:367`) |
| `override_graph` copies a phase with `..node.phase`, so the clone carries new fields with no code change | true | `graph.rs:1268-1272` |
| Postgres phase rows are read through `phase_row`/`phase_rows_on` (graph body, resolve and shape alike) | true | `pg/read.rs:3041,3571,3216,3769,3820`; `pg/write.rs:1564` |
| The SQLite mirror holds no phase rows | true | no `step_graph_phase` under `htui-store/src/cache` |
| The bounds are global `app_setting` keys with code defaults, not project settings; no per-graph storage | true | `graph.rs:621-638`; `SnapshotSettings` `run.rs:1010-1036`; `ProjectSettings` has none (`kind.rs:651`) |
| Nothing reads `runtime_changes` but the snapshot | true | text search: only `SnapshotSettings` and fixtures |
| No compete or later-tier setting exists | true | `SettingKey` has 13 keys, none compete (`prompt/settings.rs:172-218`); no `later_tier` anywhere |
| `memory_capture` is the App-rung switch precedent | true | `prompt/settings.rs:192-193`; commits `80bec1df`, `29571b61` |
| `SettingKey::ALL` is 13 and the App rung lists 11 | true | `prompt/settings.rs:204`; `prompt_settings.rs:37` |
| Divide runs since MOD-139; resolve refuses `PlannerAgent`, `NoParts`, `PlanScope` and `Palette`, then `DivideRefused` | true | `graph.rs:744-797` |
| `NotRunnable` and `RunFailure` are public with display sentences, reachable from `htui` | true | `graph.rs:322-343`, `status.rs:344`; `htui/Cargo.toml:31`; `htui-orch/src/lib.rs` `pub mod graph/divide/status` |
| `owned_path_refusal` is private | true | `divide.rs:253` (T1 makes it `pub`) |
| Admission checks default parts: local, `max_parts`, `max_fan_out`, `max_agents_per_run`, persona, paths, agent among candidates | true | `divide.rs:177-249` |
| The M1 form shows the M2 fields read-only and `build` keeps them from the base | true | `node_form.rs:403, 522-548` |
| The canvas badge and the form still name MOD-139 | true | `canvas.rs:50`; `node_form.rs:36` |
| Hint labels are editor-local constants, so per-focus nouns need no catalogue change | true | `graph_editor/mod.rs:217-230` |
| The step-form stack holds exactly the four list acts in the editor's context | true | `keys/views.rs:433-447`; `catalogue.rs:1644-1671` |
| `REQUEST_NAMES` in `catalogue.rs` is 12 | true | `catalogue.rs:440` |
| The editor asks for `Agents` and `Templates` on open and activation | true | `graph_editor/mod.rs:372-438` |
| Wave 1 tasks are file-disjoint | true | Intersection check above |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| T1 is wide: about 50 literals across six crates, plus `.sqlx` | High | One serial task first; `cargo check --workspace --all-targets --all-features` before any test; prepare against a migrated scratch DB |
| The Postgres agent FK and `MemStore` disagree on deleting a judge's agent | Medium | D2: a conformance case for deleting an agent named as a judge, run on both stores |
| Renumbering the `SettingKey` discriminants breaks an index pin | Medium | T3 runs the whole `prompt::settings` and `prompt_settings` suites; precedent `80bec1df` |
| The warnings drift from resolve | Medium | Unit tests compare each warning with the `NotRunnable`/`RunFailure` display; T9 resolves a saved divide; the warnings never block a save |
| Snapshot churn across editor, Kinds and Prompt frames | High | Full `cargo insta test` in T3 and T8, not a `.snap` grep (memory: snapshot impact) |
| Five worktree lanes contend for the sandbox Postgres and disk | Medium | Only T1 and T7 need Postgres and both are serial; `df -h .` before wave 1 |
| The step form grows past the panel height | Medium | Rows shown by split (compete rows only on compete, divide rows only on divide); the part form is its own mode |

## Acceptance

- [ ] All tasks complete, TDD (tests first)
- [ ] Validation passes (the sandbox's Qdrant/Postgres-live failures are recorded and owed on the host)
- [ ] Patterns mirrored, not reinvented
- [ ] The PRD's M2 metrics hold: every M2 node field round-trips, every saved divide that resolve accepts resolves, compete stays hidden while the switch is off, zero hardcoded keys
