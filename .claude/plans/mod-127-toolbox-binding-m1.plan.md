# Plan: MOD-127 M1 - typed binding and resolver

**Source PRD**: `.claude/prds/mod-127-toolbox-binding.prd.md`
**Selected Milestone**: 1 - Typed binding and resolver
**Complexity**: Large (four serial tasks, ~45 files, one migration; routed PRD, ultracode for implement)
**Plan-only waiver**: `plan`'s confirm gate is waived (run from `/handoff-run`, sandbox `hr/MOD-127`).

## Summary
Skills attach at three levels today (global, project, phase) through `skill_binding`, resolved by one pure
`resolve` that both stores call. M1 turns that table into the one binding table every Toolbox kind will use
(`toolbox_binding`: `kind`, `ref_id`, scope keys), adds the workspace and item-kind levels `R-SKL-2` already
requires, passes the run's item kind and workspace into resolution, and keeps every existing skill
attachment resolving exactly as before. M2 and M3 add MCP server, command and env-preset rows to the same
table without another table migration.

## Shared decisions (every task reads this section)

- **D1 - Five levels, one order.** `SkillLevel` becomes `Global < Workspace < Project < ItemKind < Phase`
  (declaration order is specificity; the winner is the `max`, as now). It stays named `SkillLevel` in M1 (a
  rename would touch every consumer for no behaviour; M2 renames it with the generic types).
  Serde spellings: `global`, `workspace`, `project`, `item_kind`, `phase`.
- **D2 - Scope is four nullable keys, level is derived** (as now, nothing stored). Valid shapes only:
  `()` Global, `(workspace)` Workspace, `(project)` Project, `(project, item_kind)` ItemKind,
  `(project, phase)` Phase. Any other combination (workspace with anything else, item kind without project,
  item kind with phase, phase without project) is a CHECK in Postgres and a refusal in `check_attachment`.
  Item kinds are per project (`item_kind.project_id`), so an ItemKind row's kind must belong to its project;
  a Phase row's phase must belong to its project (existing rule).
- **D3 - Table.** One forward migration renames `skill_binding` to `toolbox_binding`, renames `skill_id` to
  `ref_id` (FK to `skill` dropped; integrity by the write path's existing existence check and by
  delete-with-referent), adds `kind TEXT NOT NULL DEFAULT 'skill' CHECK (kind IN ('skill'))` (M2 widens the
  CHECK), `workspace_id` (FK `workspace` ON DELETE CASCADE) and `item_kind_id` (FK `item_kind` ON DELETE
  CASCADE), recreates the unique key as `UNIQUE NULLS NOT DISTINCT (kind, ref_id, workspace_id, project_id,
  item_kind_id, phase_id)` and renames the CHECKs and the `updated_at` trigger. Existing rows keep their
  ids, positions, activation and globs. **No dual read window:** one forward migration, run before the new
  binary uses the table (`R-STO-5`); the PRD's last open question closes on that.
- **D4 - Rust keeps the skill types in M1.** `SkillBinding`/`SkillBindingKey`/`BoundSkill` gain
  `workspace_id`/`item_kind_id` (key: `workspace`/`item_kind`). `BindingKind` as a Rust type waits for M2,
  when a second kind exists to test it against. The `kind` column holds only `skill` in M1; the Pg skill
  queries filter `kind = 'skill'` and write it explicitly.
  *This deviates from the item's wording ("typed binding (kind, ref ...)"): the typed shape is in the schema
  now and in Rust with M2.*
- **D5 - Resolution input.** `bound_skills(project, phase)` becomes
  `bound_skills(scope: ResolveScope)` with `ResolveScope { workspace: Option<WorkspaceId>, project:
  ProjectId, item_kind: Option<ItemKindId>, phase: Option<PhaseId> }`. A row applies when its keys match
  the scope exactly at its level: global; workspace row when `scope.workspace == Some(row.workspace)`;
  project row on project; item-kind row on `(project, kind)`; phase row on `(project, phase)`. A `None` in
  the scope matches no row of that level (today's `b.phase_id = NULL` behaviour, kept). Winner rule, `off`,
  pins, position and byte-order tie-break are unchanged (`collapse` already takes a level-tagged list).
  Workspace is `Run.workspace_id`: `None` for a chat run, a run with no scope or a project in none or
  several workspaces, so the workspace level then does not apply (the engine holds no workspace today and
  uses `PromptScope::project_only`, `engine.rs:9852`).
- **D6 - Listing.** `skill_bindings(Option<ProjectId>)` becomes `skill_bindings(home: BindingHome)` with
  `BindingHome::{Global, Workspace(WorkspaceId), Project(ProjectId)}`; `Project` returns the project's
  project, item-kind and phase rows. Order: `(ref_id bytes, item_kind_id NULLS FIRST, phase_id NULLS
  FIRST)` (extends D91).
- **D7 - Globs.** A repo-qualified glob is refused on Global and Workspace rows (no single project) and
  allowed on Project, ItemKind and Phase rows.
- **D8 - Step record.** `skill_choices` gains no field. `SkillChoice.level` serialises the two new
  spellings; `RECORD_VERSION` stays 5 (additive values of an existing field; no reader decodes stored
  `skill_choices`, survey 5). Per-kind choice sections arrive with M2.
- **D9 - No freezing of skills in M1.** Skills stay live-resolved at every step (as now). A run's frozen
  tool set arrives with M2 following the `personas`/`memory` precedent in `GraphSnapshot`; whether skills
  join it is M2's call. The PRD's "exact frozen set" metric is M2's.
- **D10 - PRD open questions.** Closed here: dual-read window (D3). The other five (forms location, command
  delivery, secret reference, env merge, MCP override) belong to M2/M3 and keep their PRD defaults.
- **Migration number** is the next free at implementation (`0041` on this branch; latest is
  `0040_execution_targets.sql`). Sibling items also add migrations: the number renumbers at collect and
  that moves the count pins listed in T2.
- **Rules for every task.** TDD (tests first); crate-scoped checks only; no workspace gate in an agent;
  `cargo fmt` and `cargo clippy -p <crate> --all-features -- -D warnings` on each crate touched; commit the
  task when its check is green (small commits).

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming / levels | `crates/htui-core/src/model/skill.rs:37-46, 211-240` | level is declaration order; derived from nullable keys by `SkillBindingKey::level` |
| Shared rule | `crates/htui-core/src/store/traits.rs:3743-3780` | `check_attachment(&BindingFacts, &Attachment)` is the one refusal chain both stores call |
| Pure resolver | `skill.rs:330-380` (`collapse`, `resolve`) | one level-tagged list in, winners out, byte-order tie-break |
| Pg read/write | `crates/htui-store/src/pg/read.rs:2125-2188`, `pg/write.rs:6793-6960` | two-query `bound_skills`; CAS on `updated_at` in `set_skill_binding` |
| Mem mirror | `crates/htui-core/src/store/mem.rs:777-804, 5273-5373` | Rust filter equal to the SQL `WHERE`, same `resolve` |
| Conformance | `crates/htui-core/src/store/conformance.rs:15557, 15744, 15926` | one suite, run on Mem and Pg |
| Parity | `crates/htui-store/tests/it/skill_attachments.rs:177-324` | Pg and Mem asked the same question |
| Migration pins | `crates/htui-store/tests/it/migrations.rs:66, 469-487, 863-875` | TABLES, column comments, `Pending(N)` counts |
| UI levels | `crates/htui/src/ui/kit/label.rs:338-387`, `ui/tabs/skills/attach.rs:748-765` | `skill_level` label, `ARow`, `key_of` |

## Files to Change

| Task | File | Action |
|---|---|---|
| T1 | `crates/htui-core/src/model/skill.rs` | UPDATE (levels, keys, row fields, pure tests) |
| T1 | `crates/htui-core/src/fixtures.rs` | UPDATE (row literals) |
| T1 | `crates/htui/src/ui/kit/label.rs` | UPDATE (level labels, `SKILL_LEVELS`, `skill_level_after`) |
| T1 | `crates/htui/src/ui/tabs/skills/{attach.rs,library.rs}`, `crates/htui/src/skills.rs` | UPDATE (exhaustive arms only; the pane gains its rows in T4) |
| T1 | struct-literal sites of `SkillBinding`/`SkillBindingKey` (claim 11): `htui-core/src/store/{conformance,mem}.rs`, `htui-orch/src/{engine,graph}.rs` (tests), `htui-store/src/pg/rows.rs`, `htui-store/tests/it/{skill_writers,skill_attachments}.rs`, `htui/tests/it/{prompt_preview,skills}.rs`, `htui/src/ui/tabs/skills/{attach,library}.rs` | UPDATE (mechanical: new fields, constructors; no logic) |
| T1 | any other file a workspace `cargo check` names for a new field/variant | UPDATE (mechanical) |
| T2 | `crates/htui-store/migrations/0041_toolbox_binding.sql` | CREATE |
| T2 | `crates/htui-core/src/store/traits.rs` | UPDATE (`BindingHome`, `ResolveScope`, `BindingFacts`, `check_attachment`, signatures) |
| T2 | `crates/htui-core/src/store/mem.rs` | UPDATE (rows, `bound_skills`, `set_skill_binding`, cascades) |
| T2 | `crates/htui-core/src/store/conformance.rs` | UPDATE (new and extended cases) |
| T2 | `crates/htui-core/src/store/worker.rs` | UPDATE (`bound_skills` signature) |
| T2 | `crates/htui-store/src/pg/{read.rs,write.rs,rows.rs,demo.rs}`, `src/{worker.rs,backend.rs,writer.rs}` | UPDATE |
| T2 | `crates/htui-store/.sqlx/query-*.json` (the 8 skill-binding entries) | UPDATE (regenerated) |
| T2 | `crates/htui-store/tests/it/{migrations.rs,skill_attachments.rs,skill_writers.rs,pg_conformance.rs,pg_criteria.rs}` | UPDATE |
| T2 | `crates/htui-orch/src/{graph.rs,fake.rs}`, `crates/htui-worker/src/{graphs.rs,runtime.rs}`, `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/it/recorder.rs`, `crates/htui/src/{preview.rs,store_worker.rs,hierarchy.rs}` | UPDATE (signature plumbing, scope built with `item_kind: None, workspace: None`) |
| T3 | `crates/htui-orch/src/engine.rs` | UPDATE (`phase_skills` takes kind and workspace) |
| T3 | `crates/htui/src/preview.rs` | UPDATE (preview passes the item's kind and the open workspace) |
| T3 | `crates/htui-orch/tests/...`, `crates/htui/tests/it/prompt_preview.rs` | UPDATE (new cases) |
| T4 | `crates/htui/src/skills.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/skills/attach.rs` | UPDATE (workspace and item-kind rows, snapshot reads) |
| T4 | `crates/htui/tests/it/snapshots/it__skills__*.snap` | UPDATE (only if text moves) |
| close | `docs/decisions/mod/mod-127.md` (at the item's close, not this milestone), `HANDOFF.md` phase note | per lifecycle P1 |

T3 and T4 do not intersect by file (T3: `engine.rs`, `preview.rs`; T4: `skills.rs`, `store_worker.rs`, `attach.rs`);
`preview.rs` and `store_worker.rs` are touched by T2 first, so T3/T4 run after T2. The lane runs T1 to T4
serially regardless.

## Tasks

### Task 1: model - five levels, scope keys, every exhaustive arm  (foundation: verifier)
- **Owns**: `skill.rs`, `fixtures.rs`, `label.rs`, the exhaustive arms listed above, and any other file the
  workspace check flags for the new variants or fields.
- **Add/change**
  - `SkillLevel`: add `Workspace` (between `Global` and `Project`) and `ItemKind` (between `Project` and
    `Phase`); `as_str` spellings per D1.
  - `SkillBinding`: add `workspace_id: Option<WorkspaceId>`, `item_kind_id: Option<ItemKindId>`.
    `SkillBindingKey`: add `workspace: Option<WorkspaceId>`, `item_kind: Option<ItemKindId>`; constructors
    `SkillBindingKey::{global, workspace, project, item_kind, phase}(skill, ...)`; `of(&SkillBinding)` copies
    all four; `level()` per D2 (a shape outside D2 reports the narrowest level the keys imply and is
    refused by `check_attachment`, T2).
  - `BoundSkill`/`collapse`/`resolve`/`select`: unchanged logic; `level` now carries the new variants.
  - `ResolveScope` struct (D5) beside the types, with `ResolveScope::project(project, phase)` helper for T2's
    plumbing.
  - Exhaustive matches: `label.rs` (`skill_level`, `SKILL_LEVELS`, `skill_level_after`: words
    `workspace`, `item kind`), the three `ui/tabs/skills` arms, `skills.rs`, and whatever the workspace
    `cargo check --workspace --all-targets --all-features` reports. No new UI rows yet.
- **Tests first** (in `skill.rs`, beside the 18 existing): `five_levels_order_by_specificity`,
  `level_of_a_key_follows_d2_shapes`, `workspace_loses_to_project_and_wins_over_global`,
  `item_kind_beats_project_and_loses_to_phase`, `off_at_item_kind_hides_a_project_attachment`,
  `an_item_kind_pin_that_names_no_version_does_not_fall_back`, `a_key_and_an_attachment_copy_a_row` extended
  to the new keys, `choices_serialize_their_documented_keys` extended with the two level spellings. The 18
  existing tests pass unchanged except for constructor changes (that is the "behaviour preserved" proof; do
  not edit an assertion).
- **Mirror**: `skill.rs:772-870` for level cases.
- **Check**: `cargo test -p htui-core --all-features --lib model::skill` then
  `cargo check --workspace --all-targets --all-features` (compile only) then `cargo fmt --all --check` and
  clippy on each crate touched.
- **Commit**: `feat(core): MOD-127 M1 T1 workspace and item-kind levels in the skill model`.

### Task 2: stores - migration, trait seam, Mem, Pg, conformance  (foundation: verifier)
- **Owns**: the migration, `traits.rs`, `mem.rs`, `conformance.rs`, `store/worker.rs`, the `htui-store` files,
  `.sqlx`, the plumbing files in the table (scope built as `ResolveScope::project(p, ph)`; behaviour
  identical).
- **Add/change**
  - Migration `0041_toolbox_binding.sql` per D3 (one transaction, forward-only). Column comments updated; the
    `updated_at` trigger renamed; rename the two CHECKs; add the scope-shape CHECK from D2
    (`num_nonnulls(workspace_id, project_id, item_kind_id, phase_id)` plus the allowed combinations).
  - `traits.rs`: `BindingHome`, `ResolveScope` (re-exported from the model), `BindingFacts` gains
    `workspace_exists`, `kind_project: Option<ProjectId>`; `check_attachment` adds the D2 shape refusals, the
    kind-in-project refusal and D7 (new sentence helpers beside `traits.rs:3699-3738`);
    `WriteStore::skill_bindings(home: BindingHome)`; `bound_skills(scope: ResolveScope)` in `WorkerStore` and
    `GraphSource` (`graph.rs:138`).
  - Pg: `bound_skills` WHERE per D5 (`kind = 'skill'`), `skill_binding_rows`/`skill_binding_row` keyed on
    all four keys, `set_skill_binding` INSERT `ON CONFLICT (kind, ref_id, workspace_id, project_id,
    item_kind_id, phase_id) DO NOTHING`, delete reach count, demo loader, `DeleteReach` for workspace and
    item-kind deletion follows what `delete_workspace`/`delete_item_kind` already report.
  - Mem: rows carry the new keys; `bound_skills` filter equals the Pg WHERE; `set_skill_binding`;
    cascades in `delete_workspace`, `remove_project`, `delete_item_kind`, `delete_phase`, `replace_graph`.
  - `.sqlx`: regenerate the eight entries (recipe: `docs/hr-sandbox.md`, "sqlx prepare"; the scratch DB
    must be migrated; byte-identical SQL reuses an entry).
- **Tests first**
  - Conformance (run on Mem and Pg): `skill_binding_workspace_and_item_kind_levels_attach_change_detach`
    (CAS as the existing case), `skill_binding_refuses_by_rule` extended (workspace with a project key, item
    kind of another project, item kind with a phase, unknown workspace, repo glob on Workspace),
    `workspace_delete_takes_its_bindings`, `item_kind_delete_takes_its_bindings`,
    `project_delete_reach_counts_item_kind_bindings`.
  - `skill_attachments.rs`: `pg_and_mem_agree_on_all_five_levels`, `a_workspace_row_applies_only_when_the_scope_names_it`,
    `an_item_kind_row_applies_only_to_that_kind`, `old_rows_resolve_as_before_the_migration` (rows inserted
    through the pre-migration shape of the test template, then read).
  - `migrations.rs`: TABLES entry `toolbox_binding`, column-comment pins, the `Pending(N)`/count pins that
    move with a new migration (about seven, per the earlier new-migration rule), `0041` file.
- **Mirror**: `traits.rs:3743-3780`, `pg/read.rs:2125-2188`, `mem.rs:777-804`.
- **Check**:
  `cargo test -p htui-core --all-features --lib store::` and `cargo test -p htui-core --all-features --test it mem_store::`,
  `cargo test -p htui-store --all-features --test it skill_` and `... --test it migrations::` and
  `... --test it pg_conformance:: skill_binding`,
  `cargo check --workspace --all-targets --all-features` (compile only), fmt, clippy on touched crates.
  The Pg suites need `HTUI_TEST_DATABASE_URL` (already set in this sandbox).
- **Commit**: `feat(store): MOD-127 M1 T2 toolbox_binding table and the five-level resolver seam`.

### Task 3: callers - run workspace and item kind reach resolution
- **Owns**: `htui-orch/src/engine.rs`, `htui/src/preview.rs` and their tests.
- **Change**
  - `Engine::phase_skills` (`engine.rs:9948-9973`) gains two parameters (the run's `workspace_id` and the
    item's `kind_id`; both callers hold the run and load the item) and builds `ResolveScope { workspace,
    project, item_kind: Some(kind), phase }` for both call sites (`:8903` judge, `:9859` phase). The stale
    "holds no workspace" comment at `:9852` is reworded: the `upstream_summaries` walk stays project-only.
  - `preview.rs:268`: scope from the previewed item's kind and the open workspace's id when the UI has one;
    otherwise `None` (a preview of a project in several workspaces shows no workspace level; say so in the
    pane's existing "skipped" line only if the Prompt pane has one, else leave).
  - `graph.rs:1482/1707` (override-graph binding copy) stays phase-level only; assert that in a test.
- **Tests first**: engine tests next to `engine.rs:22859-23961`:
  `a_workspace_attachment_reaches_a_run_with_a_workspace`,
  `a_chat_run_without_a_workspace_gets_no_workspace_attachment`,
  `an_item_kind_attachment_reaches_only_items_of_that_kind`; `prompt_preview.rs`:
  `preview_resolves_item_kind_and_workspace_levels`. Existing snapshots must not move.
- **Mirror**: `engine.rs:22859` skill tests.
- **Check**: `cargo test -p htui-orch --all-features --lib engine::` filtered to the new names, then
  `cargo test -p htui-orch --all-features --test it` (if the crate has one), `cargo test -p htui --features testkit --test it prompt_preview::`,
  fmt, clippy on `htui-orch` and `htui`.
- **Commit**: `feat(orch): MOD-127 M1 T3 item kind and workspace reach skill resolution`.

### Task 4: Skills tab - workspace and item-kind attachments
- **Owns**: `skills.rs`, `store_worker.rs`, `ui/tabs/skills/attach.rs`, their tests and snapshots.
- **Change**
  - `SkillsSnapshot` loads `BindingHome::Workspace(w)` rows per workspace and keeps project/item-kind/phase
    rows under `ProjectSkills.bindings`; `SkillsSnapshot::binding(key)` finds a row by the full key.
  - `attach.rs`: `ARow` gains `Workspace(WorkspaceId)` and `ItemKind(ProjectId, ItemKindId)` rows between
    their neighbours (order = specificity), `key_of` builds the full key, the form writes
    `StoreRequest::SetSkillBinding` with it (one form, the same activation/pin/position/glob fields).
  - Labels already exist from T1. Every key and hint through `Keys` (no literal keys; guard test
    `no_hardcoded_keys.rs`).
- **Tests first**: `attach.rs` tests `attach_at_workspace_level`, `attach_at_item_kind_level`,
  `rows_list_in_specificity_order`; snapshot `it__skills__attachments` extended with a workspace row.
- **Mirror**: `attach.rs:709-765`, `skills.rs:116-162`.
- **Check**: `cargo test -p htui --features testkit --test it skills::` (read the names that run), the
  guards `cargo test -p htui --features testkit --test no_hardcoded_keys --test kit_chrome`, fmt, clippy on
  `htui`; snapshot diffs reviewed in the task's own words.
- **Commit**: `feat(ui): MOD-127 M1 T4 workspace and item-kind attachment rows`.

## Run shape and review
- **Lane shape** (verdict recommends ultracode for implement): `scripts/task-lane.js`, one lane of serial
  tasks T1-T4 on this checkout, one implementer per task from `.claude/plans/mod-127-toolbox-binding-m1.briefs/Tn.md`.
  **Verifier** for T1 and T2 (foundation; later tasks build on them) and for the fix round only.
- **Gate**: once after the lane, `bash scripts/gate.sh` by the main thread (plus `gate.sh --only doc`:
  public docs change in `htui-core`).
- **Review**: `rust-reviewer` once over `git diff <base>..HEAD`, one fix round.
- **Owed on the host** (sandbox limits): the merged-tree Postgres gate at `scripts/hr collect MOD-127`, and
  the count pins if another item's migration lands first.

## Validation
```bash
cargo test -p htui-core --all-features --lib model::skill
cargo test -p htui-store --all-features --test it skill_attachments:: migrations:: pg_conformance::
cargo test -p htui-orch --all-features --lib engine::
cargo test -p htui --features testkit --test it skills:: prompt_preview::
cargo clippy --workspace --all-features -- -D warnings && cargo clippy --workspace -- -D warnings
bash scripts/gate.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A field or variant added to `SkillBinding`/`SkillLevel` breaks literals and `match`es across crates | High | T1 owns every arm and fixes literals until `cargo check --workspace --all-targets --all-features` is clean (enum-variant rule) |
| A resolver edit changes an old skill outcome | Medium | the 18 old tests and `skill_attachments.rs` stay untouched in assertions; `old_rows_resolve_as_before_the_migration` |
| Pg/Mem WHERE drift | Medium | one conformance and one parity suite over all five levels |
| Migration number clashes with a sibling item | High | number taken at implementation, renumbered at collect; pins listed in T2 |
| `.sqlx` regeneration needs a migrated scratch DB | Medium | follow `docs/hr-sandbox.md`; byte-identical SQL reuses entries |
| Engine has no workspace for some runs | Medium | D5: workspace level applies only with `Run.workspace_id`; tested |
| Snapshot moves in unrelated tabs | Low | only `it__skills__attachments` is expected to move; full insta run before the gate |

## Acceptance
- [ ] T1-T4 committed; every old skill test unchanged in assertion
- [ ] `toolbox_binding` migration forward-only, old rows resolve identically
- [ ] Both stores agree on all five levels (conformance + parity)
- [ ] A workspace, item-kind, project and phase attachment of one skill resolve phase > item kind > project > workspace > global
- [ ] Gate green (or the known flake named); reviewer findings applied or deferred with the maintainer

## Verified claims
Step 3.5, 2026-11-15, against the tree at `0b5570b8` (plan amended where a claim was off).

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Skills cannot be deleted today, so dropping the `skill_id` FK (D3) loses no cascade in use | confirmed | no `delete_skill`/`remove_skill`/`DELETE FROM skill` in `crates/**` or migrations |
| 2 | Latest migration is `0040_execution_targets.sql`, so `0041` is next | confirmed | directory listing; renumbers at collect (D-note) |
| 3 | `WorkspaceId` and `ItemKindId` exist | confirmed | `model/ids.rs:83, 89` |
| 4 | `item_kind` is per project (`project_id NOT NULL`), so an ItemKind row needs a project (D2) | confirmed (survey of `0001_init.sql:282-293`) | `UNIQUE (project_id, prefix)` |
| 5 | `skill_binding` is not mirrored into the offline cache | confirmed (survey `backend.rs:345`, `pg/read.rs:2071`) | no cache migration needed |
| 6 | `phase_spec` and `judge_prompts` hold the run and reach the item | confirmed | `phase_spec(run: &Run, ..., item: ItemId)` loads `row` and calls `item_kind_name(&row)` (`engine.rs:9755-9860`); `judge_prompts(run: &Run, ...)` (`:8794`), call at `:8902-8903`. **Amended:** `phase_skills` (`:9948`) takes `project, snapshot, phase, notes` only; T3 adds the run's workspace and the item's kind as parameters |
| 7 | `Run.workspace_id: Option<WorkspaceId>` exists | confirmed (survey `model/run.rs:288-292`) | engine comment at `:9852` says "holds no workspace" and is stale for the workspace level; T3 updates it |
| 8 | The preview has the item, its kind and the UI scope | confirmed | `preview.rs:~268`: `backend.bound_skills(row.project_id, phase)`, `kind.name`, `PromptScope::from_scope(scope, row.project_id)` |
| 9 | Only one place adds a Postgres unique key over NULLable columns the same way | confirmed | `0001_init.sql` already uses `UNIQUE NULLS NOT DISTINCT (skill_id, project_id, phase_id)`; Postgres 18 |
| 10 | `bound_skills` implementers to update in T2 | **amended** | 13 symbols: `WorkerHost` (`htui-core/src/store/worker.rs:688`), `MemStore` (`mem.rs:777`), `GraphSource` + `TestSource` (`htui-orch/src/graph.rs:138, 2486`), `MemStore` + `FakeGraphSource` (`htui-orch/src/fake.rs:1671, 1821`), `Backend` (`htui-store/src/backend.rs:375`, `worker.rs:1358`), `PgStore` (`worker.rs:1245`, `pg/read.rs:2125`), `HostGraphs` (`htui-worker/src/graphs.rs:52`), `Failing` (`htui-worker/src/runtime.rs:4695`). `runtime.rs` joins T2's file set |
| 11 | `SkillBindingKey` / `SkillBinding` struct literals T1 must fix | **amended** | 26 `SkillBindingKey {` and 14 `SkillBinding {` hits: `skill.rs`, `fixtures.rs:667`, `conformance.rs:15224`, `mem.rs:5352, 14892`, `engine.rs:22768, 23692, 23705`, `graph.rs:1526, 1707, 3785, 4457`, `rows.rs:312`, `skill_writers.rs:140`, `skill_attachments.rs:137`, `htui/src/skills.rs`, `ui/tabs/skills/{attach,library}.rs`, `htui/tests/it/{prompt_preview,skills}.rs`. All join T1's owned set (mechanical field additions, constructors) |
| 12 | Test targets exist for the checks | confirmed | `htui-core/tests/it/mem_store.rs`, `htui-store/tests/it/{skill_attachments,pg_conformance}.rs` |
| 13 | Task file sets: T3 (`engine.rs`, `preview.rs`) and T4 (`skills.rs`, `store_worker.rs`, `attach.rs`) are disjoint; T1/T2 overlap T3/T4 files only mechanically and run first | confirmed by intersection | the lane is serial anyway |
| 14 | A `RECORD_VERSION` bump is not needed for new `level` spellings | confirmed (survey 5) | no production code decodes stored `skill_choices`; the field is `[]`/array of objects, values additive |
| 15 | The compile probe for the migration (`ALTER ... RENAME`, `NULLS NOT DISTINCT`) | deferred to T2's first step | T2 runs the migration against the test Postgres (`htui-postgres-test`/sandbox DSN) before writing Rust; a failed statement amends D3 |
| 16 | sqlx entries moved by T2 | confirmed count | eight `.sqlx` entries mention `skill_binding` (survey 8) |
