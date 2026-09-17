# Plan: MOD-15 milestone 4 — kinds and graphs are editable

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 4 only (`:252`). Design
authority: the PRD's Scope (`:187-190`), Constraints (`:225-240`), D2, D6, D10 (kind delete), D12
(cited as **PRD Dn**), milestone 1's plan (`.claude/plans/mod-15-hierarchy-seam.plan.md`, **M1 Dn**),
milestone 2's plan (`.claude/plans/mod-15-project-seed.plan.md`, **M2 Dn**), milestone 3's plan
(`.claude/plans/mod-15-hierarchy-section.plan.md`, **M3 Dn**), `docs/ANA-2.md` §4.1 and §5.4,
`docs/ANA-9.md` §5.4/§5.5, `HANDOFF.md:284-416`. This plan's own decisions are plain **Dn**.
**Requirements**: `R-ENT-6` (the five kinds and their graphs), `R-ENT-10` (no last-writer-wins),
`R-ORCH-1` (phases), `R-TUI-8`, `R-NF-3` by ownership.
**Bare filenames below**: `store_worker.rs`, `catalogue.rs`, `hierarchy.rs`, `testkit.rs`, `ui/**`,
`app/**` are `crates/htui/src/…`; `traits.rs`, `mem.rs`, `seed.rs`, `model/kind.rs`,
`prompt/settings.rs` are `crates/htui-core/src/…`.
**Complexity**: Medium-high (one worker module, nine `StoreRequest` and three `StoreReply`
variants, one section with four modes; **no seam change, no migration, no new `query!`**).
**Routing**: routed as **PRD** by `/handoff-run MOD-15`; milestone 4 resumes at `plan`. Reviewer:
`rust-reviewer` (`.claude/workflow-config.json`). Ultracode: not recommended (the tasks are serial;
see Tasks).

## Summary

Milestone 1 put every kind, graph and phase writer on the seam (`traits.rs:444-537`: 
`create_item_kind`, `update_item_kind`, `item_kinds`, `delete_item_kind`, `create_step_graph`,
`update_step_graph`, `step_graphs`, `create_phase`, `update_phase`, `phases`, plus
`set_setting`/`clear_setting`/`setting` over `App | Project(ProjectId) | Phase(PhaseId)`).
Milestone 2 made every created project arrive with 5 graphs, 15 phases, 5 kinds and 10 templates
from `htui_core::seed`. Milestone 3 built the section machinery: `TextField`,
`SettingsSection::captures_input`, `HierarchySection` and `htui::testkit::SectionBench`.

Nothing in `crates/htui` reads or writes a kind, a graph or a phase today: `item_kind`,
`step_graph` and `step_graph_phase` have no `StoreRequest`, no view and no section.

This milestone adds two things. (1) **`htui::catalogue`** (`crates/htui/src/catalogue.rs`), the
worker half: one `CatalogueSnapshot` per read — every project of the scope with its kinds, its
graphs and each graph's phases — and nine served requests, exactly as `htui::hierarchy` does for
the tree. (2) **`KindsSection`** (`SectionId("kinds")`, `ui/tabs/settings/kinds.rs`), registered
after `hierarchy`: kinds with their prefix, name, description and default graph; graphs with their
phases; create and edit for all three through milestone 1's compare-and-set methods; the
prefix-change warning of PRD D12; kind delete refused when referenced (PRD D6/D10); phase editing
limited to PRD D2's six columns with MOD-4's five rendered read-only.

`token_budget` is the sixth editable phase column and is **not** in `PhasePatch` — it is
`set_setting`'s on the `Phase` rung (M1 D8). This milestone therefore builds the first settings
writer in the app, scoped to that one rung and that one key; milestone 5 widens it to the ten keys
on `App` and `Project` (D6).

No `WriteStore` method is added: conformance `CASES` stays **36**, `EXPECTED_CASES` stays **36**,
`.sqlx/` and `migrations/` are untouched, and `0003_orchestration.sql` is still MOD-4's and still
next.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **One worker module, `crates/htui/src/catalogue.rs`**, mirroring `hierarchy.rs` one concept across: `CatalogueSnapshot`, `snapshot()`, `serve()`, `REQUEST_NAMES`, and the same `reread`/`cas` helpers. Not added to `hierarchy.rs`. | `hierarchy.rs` is already 607 lines and owns a different tree with a different identity story (`created_by`, `box_id`). The catalogue needs neither: no row it writes carries a user or a box. Two files, one responsibility each. |
| D2 | **The read is scope-wide, one request: `StoreRequest::Catalogue(Scope)`.** The snapshot carries one `ProjectCatalogue` per `scope.project_ids`, in scope order. | The staleness index is keyed by `(Origin, Discriminant<StoreRequest>)` (`app/state.rs:158`), so N per-project requests of one variant would leave **only the newest reply delivered** and the section would render one project's catalogue and nothing else. One request per event is the only shape that cannot drop a project. |
| D3 | **Snapshot types**: `CatalogueSnapshot { projects: Vec<ProjectCatalogue> }`, `ProjectCatalogue { project: Project, kinds: Vec<ItemKind>, graphs: Vec<GraphEntry> }`, `GraphEntry { graph: StepGraph, phases: Vec<StepGraphPhase> }`. Rows are derived from it on demand, never cached beside it. | M3 D5's rule: the section re-renders from one snapshot and never patches a single row into it, so there is exactly one source of truth on the render side. A project id in the scope that names no row is **skipped**, as a widowed `workspace_project` link is (`hierarchy.rs:128-131`) — a torn read, not a state to render. |
| D4 | **Row tree**: `Project` → each kind in `position` order, and directly under each kind the phases of **its default graph** in `position` order → then each graph **no kind points at**, with its own phases under it. A kind whose `default_graph_id` names no graph in the project renders `graph missing` and contributes no phase rows. | The seed is 1:1 (5 kinds, 5 graphs) so the common tree is kind-then-phases, which is how a user thinks about it. Listing unreferenced graphs separately is what keeps a graph created here from becoming invisible the moment no kind names it. |
| D5 | **Nine requests** (`REQUEST_NAMES`, in variant order): `catalogue`, `create_kind`, `update_kind`, `delete_kind`, `create_graph`, `update_graph`, `create_phase`, `update_phase`, `set_phase_budget`. No `delete_graph` and no `delete_phase`. | The seam has `delete_item_kind` and nothing else in this area (`traits.rs:481`; no `delete_step_graph`, no `delete_phase`), and this milestone adds no seam method. `d` on a graph or phase row answers "graphs and phases are not deleted here", exactly as `d` on a repo row does (`hierarchy.rs:658-663`). |
| D6 | **`set_phase_budget` is one request carrying `budget: Option<i64>`**: `Some` calls `set_setting(SettingRung::Phase(id), SettingKey::TokenBudget, value, expected)`, `None` calls `clear_setting` on the same rung. It is **not** a rung-generic `SetSetting`. | Milestone 5 owns the ten keys across `App` and `Project`, including which rung answered and the provenance line; a generic request built here would be built without its consumer and would be the wrong shape by the time it had one. One phase column, one request, one name on the status line. |
| D7 | **Every write carries the `Scope`** and the worker answers with a freshly assembled `CatalogueSnapshot` (or `CatalogueStale` on a compare-and-set miss). | The reply must re-read the same tree the section renders, and that tree is scope-wide (D2). `hierarchy.rs` resolves the workspace from a project id because its tree has one root; the catalogue's root *is* the scope, so it travels with the request rather than being guessed from it. Cost is one `Vec<ProjectId>` clone per write, never per keystroke. |
| D8 | **Compare-and-set on `updated_at` for kinds, graphs and phases** (M1 D3), and on the **phase's** `updated_at` for `set_phase_budget`. A miss answers `CatalogueStale`: the tree is replaced, the editor keeps its typed text, takes the current row's token, and the retry is a second `Enter` (PRD D8, M3 D7). Never an automatic retry. | `R-ENT-10`. Identical to milestone 3's behaviour so one habit covers both sections. |
| D9 | **`create_phase` takes the row the worker builds from `htui_core::seed::phase_row`**, with the section supplying only `name`, `position`, `template_name`, `gate_hard` and `input_kinds`; every other column is ANA-2's frozen default. **`phase_row` takes `&PhaseSeed`, whose `name` and `input_kinds` are `&'static str` / `&'static [&'static str]` (fact-check F-6), so runtime text cannot be passed through it**: the worker calls it with a `PhaseSeed { name: "", input_kinds: &[], gate_hard }` and then overwrites `name`, `output_kind`, `template_name` and `input_kinds` on the returned row. The eight frozen columns (`fan_out`, `gate`, `retry_limit`, `isolation`, `command_queue`, `verify_command`, `template_version`, `token_budget`) still come from `seed.rs` and from nowhere else. | `create_phase` takes a whole `StepGraphPhase` (`traits.rs:517`). Letting the section fill thirteen columns would make the app a second source of ANA-2's defaults, which M2 D1 deliberately collapsed into one. A phase created here is the same row the seeder writes. |
| D10 | **The prefix-change warning is a stage, not a toast** (PRD D12): submitting an edit whose `prefix` field differs from the stored one enters `Mode::ConfirmPrefix`, which prints `items keyed OLD-* keep their keys and their counter; the next item minted under this kind is NEW-1.` with `y` to write and `n`/`Esc` back to the editor with the text intact. Only `y` sends `update_kind`. | ANA-9 §10 asks only that the TUI warns; PRD D12 fixes the semantics (old keys keep their text, the old counter row survives, the new prefix mints from 1) and hands the wording here. A warning that cannot be read before the write is not a warning. |
| D11 | **Kind delete is one confirmation, not two** (PRD D13's typed slug stays workspace/project only): `d` on a kind row prints `delete kind NAME (PREFIX)? a kind any item uses is refused. y delete · n/Esc stop`, `y` sends `delete_kind`. A refusal arrives as `Failed` carrying the seam's own sentence (`item_kind FEAT is held by 4 items`). | D6's asymmetry, stated in the PRD: a referenced kind cannot be deleted at all, so the destructive case does not exist. Two confirmations over an act the database refuses would train the reflex PRD D13 exists to defeat. |
| D12 | **A kind delete rebuilds the mirror** through `CacheStore::rebuild()`, reported with milestone 3's `MirrorAfterDelete` (re-used from `htui::hierarchy`, not re-declared), in a new `StoreReply::KindDeleted { mirror, catalogue }`. | `item_kind` is a mirrored table whose deletes have no propagation (cache refresh rides `updated_at`; only `item_link` tombstones — PRD D13's third consequence). Without the rebuild the Backlog keeps offering a kind that is gone. Same worker-side treatment `DeleteProject` already gets (`hierarchy.rs:333-350`). |
| D13 | **`input_kinds` is typed as a comma-separated list**: split on `,`, each part trimmed, empty parts dropped, order preserved, no de-duplication, replaced whole. Rendered back the same way. | `PhasePatch::input_kinds` is `Option<Vec<String>>`, replaced whole (`model/kind.rs:228-229`). De-duplicating would silently edit what the user typed; the store takes the list as given. |
| D14 | **`gate_hard` is a `y`/`n` field**, parsed by the `yes_or_no` helper milestone 3 wrote, which is **promoted** from a private fn in `settings/hierarchy.rs` to `pub(crate)` in `settings/mod.rs` together with `some_text`. | One parser for one convention (`primary (y/n)` in M3's repo editor). Promotion rather than a copy, so the two sections cannot drift on what `Y ` means. |
| D15 | **MOD-4's columns are rendered, never edited** (PRD D2): under the selected phase the section prints one dimmed detail line carrying `fan_out`, `gate`, `isolation`, `command_queue`, `verify_command`, `retry_limit`, `output_kind` and `template_version`, followed by `MOD-4 owns these`. | Editing a column nothing reads is inventing MOD-4's semantics a milestone early; hiding it is pretending the column does not exist. The user sees the row as it is and why it is read-only. |
| D16 | **`token_budget` empty means inherit**: the phase editor's sixth field is empty when `step_graph_phase.token_budget` is `NULL`, and clearing it sends `set_phase_budget { budget: None }` (a `clear_setting`, which deletes the row's value so the project/app rung answers). The row renders `budget: inherit` or `budget: 60000`. | PRD D7's "`set` and `clear` are separate operations, because clear lets the compiled default answer, rather than the editor guessing at a constant." The editor never writes a number it was not given. |
| D17 | **Registered after `hierarchy`** in `app/mod.rs` (`Agents`, `Hierarchy`, `Kinds`), title `Kinds`. The PRD's open question 2 (whether `agents` stays first) stays open and unanswered here. | Registration order is strip order (`settings/mod.rs:98-101`); appending is the only choice that moves no existing snapshot's strip line except by adding to it. |
| D17b | **`output_kind` is set at create time and never edited**: a created phase gets `output_kind = name` (the seeder's own rule), and a later rename through `PhasePatch` leaves `output_kind` at the old string, because the seam has no patch field for it. The read-only detail line of D15 shows it, so the divergence is visible rather than silent. Open item **O-1**: whether a rename should follow `output_kind` is MOD-4's call — it is the side that reads `document.kind`. | `PhasePatch` carries exactly five fields (`model/kind.rs:219-230`) and this milestone adds no seam method. Inventing an `output_kind` writer here would decide a MOD-4 semantic a milestone early, which is the thing PRD D2 forbids. |
| D19 | **`g` opens the owning graph's editor** from a kind row or a phase row (and behaves as `e` on a graph row). Added after the blueprint surfaced H-6. | D4 gives a `Graph` row only to a graph **no kind points at**, so without this key the five seeded graphs — every graph that matters — would have no editable name or description. One key reaching the editor `e` already opens beats a fifth row variant that would double the tree. |
| D18 | **Tests**: `crates/htui/tests/kinds.rs`, worker half through `store_worker::serve` over `Backend::memory(MemStore::demo())` (one request in, one reply out, no shell), section half through `SectionBench` and `Harness`, with `kinds__*.snap` snapshots. | M3 D14, one file across. The demo fixture already carries the seeded catalogue (`htui_core::fixtures` builds from `seed`), so the worker half needs no hand-written rows. |

## Patterns to Mirror

| Concern | Pattern | Where |
|---|---|---|
| Worker module: snapshot types, `snapshot()`, `serve()`, `reread`, `cas`, `REQUEST_NAMES` | `htui::hierarchy` | `crates/htui/src/hierarchy.rs:24-153`, `:169-356`, `:361-395`, `:456-469` |
| One `try_serve` arm of or-ed patterns, **no guard** (a guarded arm is `E0004` — M3 F-12) | the twelve hierarchy patterns | `store_worker.rs:736-752` |
| Section skeleton: `Row`, `EditorKind`, `Field`, `Editor`, `Mode`, `busy`, `notice`, `hint` | `HierarchySection` | `ui/tabs/settings/hierarchy.rs:88-226`, `:342-381`, `:762-765` |
| Editor keys, focus, submit-guard while a write is in flight | `on_editor_key` / `submit` | `settings/hierarchy.rs:772-810`, `:818-832` |
| Confirmation stage that swallows every unlisted key except `CONTROL` chords | `on_deleting_key` | `settings/hierarchy.rs:679-735` |
| CAS miss handling that keeps the text and re-takes the token | `on_stale` + `reload` | `settings/hierarchy.rs:986-1010`, `:1318-1349` |
| `Failed` routed by request name | `REQUEST_NAMES.contains(request)` | `settings/hierarchy.rs:1195-1215` |
| Mirror rebuild reported inside a successful delete reply | `DeleteProject` arm | `hierarchy.rs:333-350` |
| Section tests: bench, keys, replies, snapshots | `tests/hierarchy.rs` | `crates/htui/tests/hierarchy.rs:1-120`, `:861+` |
| Multi-phase HANDOFF paragraph | "Milestone N landed (…)" appended to the one checklist line | `HANDOFF.md:324`, `:352`, `:376`; `.claude/rules/workflow-docs.md` lifecycle 4 |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/catalogue.rs` | **new** | T1 | D1/D3/D5/D7/D9/D12: snapshot types, `snapshot()`, `serve()`, `REQUEST_NAMES` |
| `crates/htui/src/lib.rs` | edit | T1 | `pub mod catalogue;` |
| `crates/htui/src/store_worker.rs` | edit | T1 | D5: nine request variants + `name()` arms; three reply variants; one `try_serve` arm of nine or-ed patterns delegating to `catalogue::serve` |
| `crates/htui/tests/kinds.rs` | **new** | T1 (worker half), T2 (section half) | D18 |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | **new** | T2 | D4/D10/D11/D13/D15/D16: the section |
| `crates/htui/src/ui/tabs/settings/mod.rs` | edit | T2 | `pub mod kinds;`, `pub use kinds::KindsSection;`, D14 promotion of `yes_or_no` / `some_text` |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | edit | T2 | D14: the two helpers move out; call sites now `super::` |
| `crates/htui/src/app/mod.rs` | edit | T2 | D17: register after `hierarchy` (`:48-49`) |
| `crates/htui/tests/settings.rs` | edit | T2 | D17: the strip-width pin now covers three sections |
| `crates/htui/tests/snapshots/kinds__*.snap` | **new** | T2 | D18 |
| `HANDOFF.md` | edit | T3 | MOD-15 entry: "Milestone 4 landed" paragraph |
| `.claude/prds/mod-15-hierarchy-management.prd.md` | edit | T3 | milestone table `:252`: row 4 → complete, plan link |

Not changed: `crates/htui-core/src/store/**` (no seam method; `CASES` stays 36),
`crates/htui-store/**` (no `query!`, no `.sqlx`, no migration), `crates/htui-agent/**`,
`crates/htui-core/src/seed.rs` (read, not edited), `ui/text_field.rs`, `chat/composer.rs`,
`keymap.rs`, `app/action.rs` (`TabAction::FocusSection` is milestone 6's).

## Tasks

**T1 → T2 → T3, serial.** T2 compiles against T1's request and reply variants and against its
snapshot types, so there is no independent pair to fan out: the file sets
T1 = {`catalogue.rs`, `lib.rs`, `store_worker.rs`, `tests/kinds.rs`} and
T2 = {`settings/kinds.rs`, `settings/mod.rs`, `settings/hierarchy.rs`, `app/mod.rs`,
`tests/settings.rs`, `tests/kinds.rs`, snapshots} **intersect on `tests/kinds.rs`** and T2 cannot
compile before T1 lands. This is why the routing verdict says ultracode is not needed.

Each implementer commits its own work incrementally — uncommitted subagent work does not survive
the session. Before blaming a Postgres failure in any gate, check `df -h /`: `target/` fills the
disk on this box.

TDD per task: tests first, red, then code. Every implementer prompt carries: PRD D2/D6/D10/D12 and
M1 D3/D8, M3 D5/D7 win over this plan where they disagree; graphify-first for codebase questions;
**no `WriteStore` change**; nothing sets `updated_at` by hand; no new migration, no new `query!`; a
section holds no store handle and no `UserId`/`BoxId`; `Debug` never prints a field's text;
`unsafe_code = "forbid"`, MSRV 1.98.

### Task 1: the catalogue worker

- **Files**: `crates/htui/src/catalogue.rs` (new), `crates/htui/src/lib.rs`,
  `crates/htui/src/store_worker.rs`, `crates/htui/tests/kinds.rs` (new, worker half only).
- **Action**:
  - D3 types and `snapshot(store, scope) -> Result<CatalogueSnapshot>`: per project id of the
    scope, `project(id)?` (skip `None`), `item_kinds(id)?`, `step_graphs(id)?` and `phases(graph)?`
    per graph. N+1 by design, per event and never per keystroke (M3 D5's trade, same words).
  - D5 nine `StoreRequest` variants with their `name()` arms:
    `Catalogue(Scope)`;
    `CreateKind { scope, project, prefix, name, description, graph: StepGraphId, position: i32 }`;
    `UpdateKind { scope, id: ItemKindId, expected: DateTime<Utc>, patch: ItemKindPatch }`;
    `DeleteKind { scope, id: ItemKindId }`;
    `CreateGraph { scope, project, name, description }`;
    `UpdateGraph { scope, id: StepGraphId, expected, patch: StepGraphPatch }`;
    `CreatePhase { scope, graph: StepGraphId, name, position: i32, template_name, gate_hard: bool,
    input_kinds: Vec<String> }`;
    `UpdatePhase { scope, id: PhaseId, expected, patch: PhasePatch }`;
    `SetPhaseBudget { scope, phase: PhaseId, expected: DateTime<Utc>, budget: Option<i64> }`.
  - Three `StoreReply` variants: `Catalogue(Box<CatalogueSnapshot>)`,
    `CatalogueStale(Box<CatalogueSnapshot>)`, `KindDeleted { mirror: MirrorAfterDelete,
    catalogue: Box<CatalogueSnapshot> }`.
  - One `try_serve` arm of **nine or-ed patterns**, no guard (M3 F-12: a guarded arm in a
    wildcard-free `match` is `E0004`), delegating to `catalogue::serve`.
  - D9: `CreatePhase` builds the row through `htui_core::seed::phase_row` and overrides only the
    five columns the request carries.
  - D12: `DeleteKind` rebuilds the mirror after the delete and reports the outcome inside the
    successful reply, re-using `crate::hierarchy::MirrorAfterDelete`.
  - Worker tests: the demo catalogue reads back 5 kinds / 5 graphs / 15 phases for the fixture
    project; a **two-project** scope answers both catalogues in scope order — the fixture holds one
    project (F-22), so the test creates the second through `create_project` over a `MemStore` and
    builds the backend from it; a scope with an unknown project id skips it rather than failing; a create, an edit and
    a stale edit each answer the shape D7/D8 promise; `delete_kind` of a referenced kind answers
    `Failed` carrying the seam's sentence; `Backend::Offline` refuses every write with
    `DATABASE_UNREACHABLE`; `REQUEST_NAMES` and `StoreRequest::name()` agree (the hierarchy test's
    `hierarchy_names_are_stable` one concept across).
- **Gate**: `cargo test -p htui --all-features`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo doc -p htui --no-deps`. `CASES`/`EXPECTED_CASES` still 36;
  `git status` shows no `.sqlx/` or `migrations/` change.

### Task 2: the kinds section

- **Files**: `crates/htui/src/ui/tabs/settings/kinds.rs` (new),
  `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs`,
  `crates/htui/src/app/mod.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/kinds.rs` (section half), `crates/htui/tests/snapshots/kinds__*.snap`.
- **Action**: D4 rows and rendering; D10 prefix-confirm stage; D11 delete confirmation; D13
  `input_kinds` parsing; D14 helper promotion; D15 read-only detail line; D16 budget field; D17
  registration and the strip pin. Keys, Browse mode: `j`/`k` move, `n` new (kind on a project or
  kind row, phase on a graph or phase row), `N` new graph, `e` edit the row, `d` delete (kinds
  only), `r` reload, `Esc` clears a notice. `captures_input` is `!matches!(self.mode, Mode::Browse)`
  — derived from the mode, never a flag.
- **Gate**: `cargo test -p htui --all-features` with the new `kinds__*.snap` accepted **after
  reading each one**, and no snapshot outside `kinds__*` changed except the strip line;
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`. `cargo run -p htui -- --demo`
  smoke where a TTY exists: `Settings` → `l` `l` → edit a phase's `input_kinds` and budget, rename a
  kind's prefix through the warning, try to delete a referenced kind and read the refusal, `q` still
  quits from Browse. **No TTY in the agent environment → say so rather than claiming it ran** (M3's
  close-out precedent).

### Task 3: docs

- **Files**: `HANDOFF.md`, `.claude/prds/mod-15-hierarchy-management.prd.md`.
- **Action**: PRD milestone row 4 → complete with this plan linked; HANDOFF's MOD-15 entry gains a
  "Milestone 4 landed (`<first>`..`<last>`, date)" paragraph carrying the test count, the nine
  request names, D6's scoping of the settings writer to the `Phase` rung, and the live coordinates
  milestones 5 and 6 need. Milestones 5 and 6 named as remaining.
- **Gate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` from the repo root
  (green); `git diff --stat` touches only the two files.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)   # no query added; must still pass
cargo doc --workspace --no-deps
```

Snapshots: `insta` writes `tests/snapshots/*.snap.new` on first run; read each, then rename to
`.snap` and commit. Pins that must **not** move: `CASES.len() == 36`
(`crates/htui-core/tests/mem_store.rs`), `EXPECTED_CASES == 36`
(`crates/htui-store/tests/pg_conformance.rs`), every `hierarchy__*.snap` and `settings__agents_*.snap`
except where the section strip gained `Kinds`. `.sqlx/` unchanged; a diff there means a query was
added against this plan.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Per-project catalogue requests silently drop all but the last project | Certain if built that way | High — a section that renders one project and looks correct | D2: one scope-wide request; a test with a two-project scope asserts both come back |
| A prefix rename is taken as a key rewrite | Medium | High — items keyed under a prefix that no longer exists | D10's warning states the semantics the seam already implements (`traits.rs:454-456`); a worker test asserts an existing `item.key_prefix` is untouched by `update_kind` |
| The phase editor writes `token_budget` twice — once through `PhasePatch`, once through the rung | Low | High — two writers for one column | `PhasePatch` has no `token_budget` field at all (`model/kind.rs:219-230`); D6's request is the only path, and there is nothing to un-build |
| A deleted kind lingers in the mirror and the Backlog keeps offering it | High if unguarded | Medium | D12's rebuild on the delete path, reported in the reply |
| `create_phase` from the section invents ANA-2's defaults | Medium | Medium — two sources for one table | D9: the worker builds the row from `seed::phase_row` |
| A `position` collision on `(graph_id, position)` refuses a phase create with a constraint name rather than a sentence | Medium | Low | The seam refuses before the statement (`traits.rs:515-516`); the section shows the refusal and leaves the editor open over the text, as M3 does |
| Four sections overflow the strip at the pinned 100×30 harness width | Medium | Low | D17's pin fails the moment the titles exceed the pane; MOD-30 owns the general fix (PRD `:378`) |
| The catalogue read costs N+1 round trips on a scope with many projects | Low | Low | Per event, not per keystroke; a joined reader is one seam method away if it ever matters |
| `--demo` smoke cannot run without a TTY | Certain in the agent environment | Low | Interactive behaviour is pinned by tests and snapshots; the close-out says the smoke did not run |

## Verified claims (fact-check, 2026-09-17)

Every row read out of this tree during `/handoff-run` step 3.5, before the CONFIRM gate.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F-1 | The staleness index is keyed by `(Origin, Discriminant<StoreRequest>)`, so two requests of one variant leave only the newest reply delivered — D2's whole reason | **true** | `app/state.rs:158`, `:273` (`std::mem::discriminant(&request)`), `is_fresh` `:290` |
| F-2 | `try_serve`'s `match request` still has **no wildcard and no guarded arm**, so the nine catalogue patterns must be or-ed into one arm | **true** | `store_worker.rs:692-758`: no `_ =>` / `_ if` / `other =>` in the match; the twelve hierarchy patterns are or-ed at `:741-752` with the `E0004` reasoning in the comment above them (M3 F-12's compile probe) |
| F-3 | All ten kind/graph/phase seam methods exist with the signatures D5 calls | **true** | `traits.rs:452` `create_item_kind`, `:461` `update_item_kind`, `:472` `item_kinds`, `:481` `delete_item_kind`, `:490` `create_step_graph`, `:497` `update_step_graph`, `:508` `step_graphs`, `:517` `create_phase`, `:526` `update_phase`, `:537` `phases` |
| F-4 | The seam has **no** `delete_step_graph` and **no** `delete_phase` | **true** | `grep -c` over `traits.rs` = 0 — D5's "no graph or phase delete" is forced, not chosen |
| F-5 | `set_setting(rung, key, value, expected: Option<DateTime<Utc>>)` and `clear_setting(rung, key, expected: DateTime<Utc>)`; `SettingRung::Phase(PhaseId)` writes `step_graph_phase.token_budget`; `TokenBudget` is the only key all three rungs accept | **true** | `traits.rs:553-575`, `:800-808`; `prompt/settings.rs:170-171`. Note the asymmetry D6 must respect: `expected` is `Option` on set (its `None` means "I expect no row", `App` only) and plain on clear |
| F-6 | `seed::phase_row(id, graph_id, position, &PhaseSeed, now)` can be handed runtime strings | **FALSE — plan amended** | `seed.rs:208-238` takes `&PhaseSeed`, whose `name: &'static str` and `input_kinds: &'static [&'static str]` (`:30-38`) cannot hold typed text. D9 now calls it with an empty static seed and overwrites the four text columns on the returned row |
| F-7 | The kind-delete refusal names what holds it: `item_kind FEAT is held by 4 items` | **true** | `traits.rs:664-666`; the FK would otherwise refuse with a constraint name (`:661-662`) |
| F-8 | Reserved template names (`judge`, `handoff`) are refused by the store on a phase write, so the section needs no second check | **true** | `mem.rs:2097-2106` (`TemplateRole::of_name` ≠ `Phase` → `Constraint(reserved_phase_name(name))`); `prompt/template.rs:514-517` |
| F-9 | The demo fixture already carries the seeded catalogue, so the worker tests need no hand-written rows | **true** | `fixtures.rs:675-720` builds kinds, graphs, phases and templates from `seed::KINDS` / `seed::phase_row` |
| F-10 | `yes_or_no` and `some_text` are private fns of `settings/hierarchy.rs` (so D14's promotion is a move, not a copy) | **true** | `settings/hierarchy.rs:1453`, `:1458` (and `is_error` `:1448`, which stays put) |
| F-11 | Reply matches outside the worker all carry `_ => {}`, so three new `StoreReply` variants compile untouched | **true** | `app/update.rs:223`, `settings/agents.rs:1267`, `settings/hierarchy.rs:1216`, `chat/mod.rs:447`, `:524` |
| F-12 | The strip pin is a real test over the **product's** registrations and lists two sections today | **true** | `tests/settings.rs:942-963` (`the_section_strip_fits_the_frame`, `SECTION_WIDE`), currently `AgentsSection` + `HierarchySection` — D17 adds the third |
| F-13 | `CASES` is pinned at 36 and `EXPECTED_CASES` at 36 | **true** | `htui-core/tests/mem_store.rs:36`, `htui-store/tests/pg_conformance.rs:19-25` |
| F-14 | `PhasePatch` has no `token_budget` field, so D6's request is the column's only writer | **true** | `model/kind.rs:219-230` (five fields), with the reasoning at `:214-217` |
| F-15 | `item_kind` is a mirrored table, which is why D12 rebuilds after a kind delete | **true** | `htui-store/src/cache/mod.rs:41-58` (`MIRRORED_TABLES`, 16 entries, `item_kind` at `:49`); refresh is cursor-based on `updated_at` with only `item_link` tombstoned (PRD `:347-352`) |
| F-16 | `DeleteTarget` is `Workspace | Project` only, so a kind delete cannot reuse `StoreReply::Deleted` without a seam change | **true** | `traits.rs:729-734` — hence D12's own reply variant |
| F-17 | Renaming a kind's prefix cannot rewrite existing item keys | **true** | `ItemPatch` has no `key_prefix` field (`model/item.rs:178-197`); `update_item_kind`'s doc states the rule (`traits.rs:454-456`) |
| F-18 | `StoreRequest` and `StoreReply` derive `Debug, Clone`, and `Scope` derives `Debug, Clone, PartialEq, Eq` — so D7's per-write `Scope` clone compiles and D2's request is cheap to carry | **true** | `store_worker.rs:67`, `:382`; `model/scope.rs:9-10`. The enum's "no secret as a plain `String`" rule (`store_worker.rs:60-66`) holds here: every field D5 adds is a prefix, a name, a description or a number |
| F-19 | `crates/htui/src/lib.rs` is where `pub mod catalogue;` goes, beside `pub mod hierarchy;` | **true** | `lib.rs:12-24` |
| F-20 | Workspace lints and MSRV are as the constraints say | **true** | root `Cargo.toml:7` (`rust-version = "1.98"`), `:91` (`unsafe_code = "forbid"`), `:92` |
| F-21 | **Task independence**: T1 and T2 are **not** independent | **true (no parallel marking)** | T1 ∩ T2 = {`crates/htui/tests/kinds.rs`}, and T2's section compiles against T1's `StoreRequest`/`StoreReply`/`CatalogueSnapshot`. The plan runs them serially; nothing here is fanned out |
| F-22 | The demo fixture's `Graphics` workspace holds **one** project, so D2's "two projects come back" test must build its own second project | **true** | `tests/hierarchy.rs:66-70` asserts `projects == [("vulkan-tutorials", 0)]`; `Harness::over(store)` exists for a hand-written store (`testkit.rs:106`) |

## Acceptance

- [ ] `htui::catalogue` assembles one `CatalogueSnapshot` per scope, skipping project ids that name no row
- [ ] Nine `StoreRequest` and three `StoreReply` variants, served through one wildcard-free `try_serve` arm of or-ed patterns; `REQUEST_NAMES` and `name()` agree in a test
- [ ] `KindsSection` registers after `hierarchy` as `SectionId("kinds")`; strip width pinned
- [ ] Kind create/edit/delete, graph create/edit, phase create/edit all land through milestone 1's CAS methods; a miss answers `CatalogueStale`, keeps the typed text and retries only on `Enter`
- [ ] Renaming a prefix passes through D10's warning, whose text names the old keys, the surviving counter and the next minted key
- [ ] Deleting a referenced kind is refused with the seam's own sentence; deleting an unreferenced one rebuilds the mirror and says what it did
- [ ] Phase editing covers exactly `name`, `position`, `template_name`, `gate_hard`, `input_kinds` and `token_budget`; MOD-4's five columns are rendered and not editable
- [ ] An empty budget field clears the setting rather than writing a default
- [ ] `CASES` 36, `EXPECTED_CASES` 36, `.sqlx/` and `migrations/` untouched, no `WriteStore` change
