# Plan: MOD-15 milestone 2 — a created project is a working project

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 2 only (`:250`). Design
authority: the PRD's Scope, Constraints (`:225-240`), D3–D6 and D12 (cited as **PRD Dn**),
milestone 1's plan (`.claude/plans/mod-15-hierarchy-seam.plan.md`, cited as **M1 Dn**),
`docs/ANA-2.md` §4.1 (`:239-352`, seeded defaults `:307-320`) and §10 item 5 (`:2053`),
`docs/ANA-9.md` §5.10 (`:820-824`), `docs/ANA-5.md` §5.3/§5.4/§9 (`:1235-1246`, `:1601-1606`,
`:2131-2133`). This plan's own decisions are plain **Dn**.
**Requirements**: `R-ENT-6` (five kinds per project), `R-ORCH-3` (the review loop the
`input_kinds` amendment encodes), `R-PRM-3`; `R-NF-3` by construction (no UI).
**Complexity**: Medium (one new core module, two seeders, one conformance case, four `.sqlx`
files, no trait change).
**Routing**: routed as **PRD** by `/handoff-run MOD-15`. Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`). Models per maintainer: plan and reviewer on Fable, every
implementer on Opus.

## Summary

Milestone 1 landed `create_project` "unseeded, structured for milestone 2 to seed inside the same
transaction" (M1 D9; `pg/write.rs:1043-1086`, `mem.rs:1568-1597`). This milestone fills the
transaction: creating a project also writes five `step_graph` rows, fifteen `step_graph_phase`
rows, five `item_kind` rows and ten `prompt_template` rows, in the order the schema accepts
(`item_kind.default_graph_id NOT NULL`, `0001_init.sql:288`), with ANA-2 §4.1's two
`input_kinds` amendments and PRD D3's three `gate_hard` flags applied at seed time, and never a
phase named `judge` or `handoff` (ANA-5 `:1238`).

The seed's one home is a new `htui_core::seed` module: the demo fixture (`fixtures.rs`) and both
stores build their rows from it, so the PRD's "seed drift between fixture and product" risk
(`:372`) has no second copy to drift. The fixture therefore takes the amendments (D2), which
changes no test and no snapshot. `NewProject`, the `WriteStore` signature and its six
implementors are untouched. One conformance case is added (`CASES` 35 → 36) and asserts, on both
stores, the counts, the names, the amended table row by row, the reserved-name rule and that a
mint works on the fresh project; template bodies and the never-seeded `item_key_counter` are
pinned per backend. PRD D12's "a prefix rename keeps history" is already two-thirds pinned by
milestone 1's case 7; the third fact (the old counter row survives) gets a per-backend twin (D8).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The seed table lives in `htui-core`, in a new `crates/htui-core/src/seed.rs` (`pub mod seed` in `lib.rs`, always compiled, not behind `demo`).** It holds `pub struct KindSeed { prefix, name, description, phases: &'static [PhaseSeed] }`, `pub struct PhaseSeed { name, input_kinds: &'static [&'static str], gate_hard: bool }`, `pub const KINDS: [KindSeed; 5]` (the D3 table), and four row constructors that take their ids from the caller: `graph_row(id, project_id, &KindSeed) -> NewStepGraph`, `phase_row(id, graph_id, position, &PhaseSeed, now) -> StepGraphPhase`, `kind_row(id, project_id, graph_id, position, &KindSeed) -> NewItemKind`, `template_row(id, project_id, name, body, created_by, now) -> PromptTemplate`; templates iterate `prompt::DEFAULT_TEMPLATES` (`prompt/defaults.rs:213-230`) directly. `fixtures.rs` **keeps** `PROJECT_SPECS`, `projects()`, `ids`, `demo_uuid`, the three strides (kind/graph `project_index*5+kind_index`, phases `project_index*15`, templates `project_index*10+i`) and `counters()`; its `KindSpec`, `KIND_SPECS` (`:656-699`) and `TEMPLATE_NAMES` (`:706-717`) are **deleted** and `catalogue()` (`:731-816`) iterates `seed::KINDS` and `DEFAULT_TEMPLATES` with `demo_uuid` ids through the same constructors. Two fixture tests read `TEMPLATE_NAMES` by name and move with it (F3): `ten_templates_per_project_from_the_default_bodies` (`:1736-1769`) drops its now-tautological name-order comparison and keeps the per-project name list, body, parse and `version == 1` pins against `DEFAULT_TEMPLATES`; and the template stride becomes `DEFAULT_TEMPLATES.len()`, keeping blueprint E-5's reason in the doc comment of `template_ids_are_distinct_across_projects` (`:1774-1797`) — the stride is a primary-key input, not a formatting choice. | ANA-5 `:1246`: "MOD-15 owns the seed; MOD-2 owns the fixture update" — one table, two id sources. PRD risk `:372` names fixture/product drift and its mitigation is a conformance case; a shared table makes the drift structurally impossible and the case a check on the table, not on two copies. `seed` sits beside `model` and `prompt` rather than under `model/` because it needs both (`body_of`, `TemplateRole`, and the row types). `TEMPLATE_NAMES` is already asserted equal to `DEFAULT_TEMPLATES`' names in order (`fixtures.rs:1738-1745`), so it is a copy with a test that says so; the test goes with it. |
| D2 | **The demo fixture takes the amendments.** After D1 the fixture's phases carry `implement.input_kinds = ['plan','review']` on `feature`, `refactor` and `tooling`, `fix.input_kinds = ['reproduce','review']` on `bug`, and `gate_hard = true` on `prd`, `plan` (feature) and `verdict` (analysis). Fixture ids do not move: phase *n* within a project is the same `(class 6, n)` it was (`PHASE_HTUI_IMPLEMENT` stays `demo_uuid(PHASE, 4)`, `fixtures.rs:280`). Position 0 of every graph keeps `input_kinds = []`; the one position-0 change is `prd.gate_hard`, which is a gate flag, not an input. | Test fallout is nil, verified: the fixture tests pin counts and bodies only (`fixtures.rs:1711-1769`); `prompt/fixtures.rs` is a literal `PromptSpec` corpus with no store read (`:1-11`), so `prompt_golden.rs`'s `phase_all_empty` does not see the fixture; the one snapshot naming `input_kinds` (`prompt_preview__preview_feat_1.snap:27`) carries the `DOCUMENTS_NOTE` text, not a value; no product code reads `input_kinds` yet (`preview.rs:43,79` asks `documents_of_kinds(item, &[])`; ANA-2's resolution arrives with MOD-4); `migrations.rs:870` counts `data.phases.len()`, unchanged at 45. ANA-2 `:1845` says the amendments belong to whichever of MOD-15/MOD-4 lands second — MOD-4 is unblocked but has not landed, so they are this milestone's (PRD D4). |
| D3 | **The amended seed table, row by row** (position, `input_kinds`, `gate_hard`); every other phase column is ANA-2 `:317-319`'s frozen default: `fan_out 1`, `gate Always`, `retry_limit 1`, `isolation None`, `command_queue FanOutOnly`, `verify_command None`, `output_kind = name`, `template_name = name`, `template_version None`, `token_budget None`. Graph name = kind name; graph description `"Default graph for {name} items"` (`fixtures.rs` wording, kept); kind `position` = index in `KINDS`. <br>**analysis / ANA** — "A question answered in writing": `research` 0 `[]` false; `verdict` 1 `[research]` **true**. <br>**feature / FEAT** — "New behaviour": `prd` 0 `[]` **true**; `plan` 1 `[prd]` **true**; `implement` 2 `[plan, review]` false; `review` 3 `[implement]` false. <br>**bug / FIX** — "Behaviour that is wrong": `reproduce` 0 `[]` false; `fix` 1 `[reproduce, review]` false; `review` 2 `[fix]` false. <br>**refactor / CLEAN** — "Behaviour kept, shape improved": `plan` 0 `[]` false; `implement` 1 `[plan, review]` false; `review` 2 `[implement]` false. <br>**tooling / TOOL** — "The workshop rather than the product": same three rows as refactor. <br>Templates: the ten names of `DEFAULT_TEMPLATES` in its order (`prd, plan, implement, review, research, verdict, reproduce, fix, judge, handoff`), `version 1`, `body = body_of(name)`, `created_by = NewProject.created_by`. | ANA-2 `:248-250` (R-ENT-6 kinds and phases), `:307-315` (the two `input_kinds` amendments), PRD D3 (`:276-279`: `gate_hard` on feature `prd`/`plan` and analysis `verdict`, none elsewhere — this **overrides** HANDOFF `:290`'s "`prd`, `plan` and `verdict`" wording, which would have flagged `plan` on refactor/tooling too), PRD D5 (`:285-287`: `review` in `implement.input_kinds` on CLEAN and TOOL as well as FEAT). ANA-9 §5.10 `:820-824` and ANA-5 §5.4 `:1601-1606` for the ten templates. The table is written here so the conformance case (D7) asserts a **literal**, not the module it is meant to check. |
| D4 | **`create_project` always seeds; nothing is parameterised.** `NewProject` (`hierarchy.rs:79-96`) is unchanged, the trait signature (`traits.rs:379-385`) is unchanged, so the six implementors (`MemStore`, `PgStore`, `Writer`, `BufferedWriter`, `UsageSpy` `htui-agent/src/conformance.rs:645`, `SpyStore` `htui-agent/tests/recorder.rs:323`) and every caller (`writer.rs:461,953-957`, `writer_buffered.rs:417-419`, `htui-agent/src/conformance.rs:779`, `recorder.rs:472`) compile untouched; only the two real bodies change. **Ids**: every one of the 35 rows gets a client-minted UUIDv7 (`StepGraphId::new()`, `PhaseId::new()`, `ItemKindId::new()`, `PromptTemplateId::new()`; `model/ids.rs:31-32`) on both stores, bound as a parameter on Postgres — the rule `NewProject.id`, `NewStepGraph.id` and `NewItemKind.id` already state (`hierarchy.rs:85`, `kind.rs:96,152`). The fixture passes `demo_uuid` ids into the same constructors. | PRD milestone 2's outcome sentence has no unseeded project; M1 D9 said "milestone 2 adds the 35 inserts between insert and commit". A flag would be a second create path with its own conformance cost (PRD Evidence: "conformance is the real cost of a seam method"). Server-side `gen_random_uuid()` would give Postgres a different minting rule from `MemStore` and the seeder no way to reference a graph id before the phase insert without a `RETURNING` round trip per row. |
| D5 | **`MemStore` validates first, then mutates, with no fallible step after the first write.** `State::create_project(new, now)` (`mem.rs:1568-1597`) keeps its order — `require_user`, duplicate id, duplicate slug — then builds the 35 rows from `seed` and pushes them straight into `graphs`, `phases`, `kinds`, `templates` (the `State` maps at `mem.rs:51-117`) together with the project row. It does **not** route them through `State::create_step_graph` / `create_phase` / `create_item_kind` (`:1957`, `:2081`, `:1845`), whose check-then-insert per row would leave a half-seeded project if a later row were refused. Seed self-consistency (unique names and dense positions per graph, valid prefixes, every phase name a `TemplateRole::Phase`, every template name with a body, every `input_kinds` entry naming a phase of the same graph) is pinned by `seed.rs` unit tests (D6), not re-checked at runtime. `item_key_counter` is never touched (`counters()` remains the fixture's, `mint_item` creates rows lazily). On Postgres the transaction is the atomicity: a seed statement that fails drops `tx` un-committed and sqlx rolls back, so "a project that fails to seed never existed" (M1 D9) holds by construction; it is not fault-injected in a test. | A `MemStore` failure path that is reachable is the unknown `created_by` (`require_user`, `mem.rs:1381`), and it fires before any mutation; the conformance case asserts that both stores leave nothing behind on it (on Postgres it is the `23503` on the project insert, first statement of the transaction). Constants cannot fail at runtime; a branch that cannot be taken is a test, not code. |
| D6 | **Reserved names: the seed never uses `judge` or `handoff` as a phase, pinned three ways and enforced zero ways at runtime.** (1) `seed.rs` unit test `no_seed_phase_is_a_reserved_template_role`: every `PhaseSeed.name` across `KINDS` has `TemplateRole::of_name(name) == TemplateRole::Phase` (`prompt/template.rs:48-58`); (2) the conformance case (D7) asserts the same on the rows read back from both stores; (3) `seed.rs` also pins `body_of(name).is_some()` for every phase name, so a phase always has its template. M1 D11's closing sentence ("its seeder drives this check through `create_project`") is **superseded**: the seeder does not call `check_phase`/`reserved_phase_name` (`mem.rs:2047-2079`, `traits.rs:653`), for D5's reason. The editor-side refusal (M1 D11) stays where it is and is what milestone 4's editor hits. | ANA-5 `:1238` — the refusal belongs to the editor; the seed obeys it by content. A test on the constant table is the strongest pin available: it fails at `cargo test -p htui-core` with no database. |
| D7 | **One new conformance case, `CASES` 35 → 36, `EXPECTED_CASES` 35 → 36; case 4's doc comment amended, its name and assertions kept.** New case `project_create_seeds_the_catalogue` on a fresh `new_project("seeded")` (`conformance.rs:1801-1809`): (a) `step_graphs(p)` returns the five names of D3 in `name` byte order (`analysis, bug, feature, refactor, tooling`; `traits.rs` orders graphs by name bytes); (b) `item_kinds(p)` returns `ANA, FEAT, FIX, CLEAN, TOOL` in position order, each `default_graph_id` the graph of the same name, positions `0..5`; (c) for every graph, `phases(g)` equals D3's `(name, position, input_kinds, gate_hard)` rows **as a literal in the test**, positions dense from 0, every position-0 row `input_kinds == []`, and the frozen defaults on every row; (d) every phase name satisfies `TemplateRole::of_name == Phase`; (e) `delete_reach(Project(p))` reports `step_graphs 5, phases 15, item_kinds 5, prompt_templates 10` (`DeleteReach`, `traits.rs:761`; the only trait-level count of templates); (f) `mint_item` on `FEAT` yields `FEAT-1` (the counter is lazy, not seeded); (g) `create_project` with an unknown `created_by` is `Constraint` and afterwards `project(id)` is `None` and `step_graphs(id)` is empty (D5). Case 4 `project_create_update_cas` (`:2144-2222`) keeps every assertion (`settings == {}`, secret columns `None`, duplicate slug, read-back, CAS) — none of them touches the seed — and its doc drops "unseeded". **Per-backend twins**, named from the new case's doc so `every_cross_referenced_test_name_exists` (`conformance.rs:3783-3795`) checks them — and named **in the task that writes them**, never earlier: T1's doc comments name only the `mem.rs::` twins, T2 appends the `pg_criteria.rs::` references when it adds those tests (fact-check F2; this is M1 blueprint flag E, `mod-15-hierarchy-seam.blueprint.md:19`, which exists because the cross-reference test reads `pg_criteria.rs` off disk and fails the moment a doc names a twin that is not there yet): `seeded_templates_carry_the_shipped_bodies` in `mem.rs` tests and `pg_criteria.rs` — via the inherent `prompt_templates` readers (`mem.rs:293`, `pg/read.rs:892`): ten rows, names = `DEFAULT_TEMPLATES` names, `body == body_of(name)`, `version == 1`, `created_by` = the request's; and `seed_never_writes_a_counter_row` — `pg_criteria.rs` with its `counter(pool, project, prefix)` helper (`:77-87`) `None` before the first mint and `Some(1)` after; `mem.rs` reading `state.item_key_counter` directly. | PRD success metric `:147` "The seed is exact" and risk `:372`'s mitigation "a conformance case asserting counts and names on both stores". One case, not two: the Postgres runner is one `CREATE DATABASE`/`DROP DATABASE` per case (`pg_conformance.rs:30-43`), and (a)–(g) share one fresh project; folding them into case 4 was rejected because case 4 is milestone 1's CAS case and would double in length, and because a seed regression should name the seed in its failure. `prompt_template` has no `WriteStore` reader and gets none here (MOD-9 owns the editor, M1 D9); `delete_reach` is the count, the twins are the content — M1 D12's rule for column facts the trait cannot read back. |
| D8 | **Prefix rename (PRD D12): the seam owes nothing new; milestone 2 owes the third fact as a per-backend twin.** Case 7 `item_kind_round_trip_and_prefix_rules` already pins two of D12's three facts on both stores: the old key text survives (`ANA-2` after `ANA → ANL`, `conformance.rs:2541-2550`) and the next mint under the kind is `ANL-1` (`:2551-2562`). Its doc claims the third — "the old counter survives" (`:2450-2451`) — but no assertion backs it, because no trait reader sees `item_key_counter`. Add `renamed_prefix_leaves_the_old_counter_row` to `pg_criteria.rs` (`counter(pool, PROJECT_HTUI, "ANA") == Some(2)` after the rename, the fixture's value at `fixtures.rs:822`; `Some(1)` for `"ANL"` after the mint) and to `mem.rs` tests (same facts on `state.item_key_counter`), both referenced from case 7's doc. No `WriteStore` reader for counters: counters are not a screen's concern, so M1 D1's argument for trait readers does not apply. | The milestone row says "A prefix rename keeps history"; two of three facts are already in the suite, and the third is a per-backend column fact, exactly M1 D12's shape. Adding a trait reader would cost six implementors for one test. |
| D9 | **The Postgres seeder is one private `seed_project(tx, project_id, created_by)` in `pg/write.rs` with four new `sqlx::query!` statements, one per table, executed per row on `&mut *tx`.** `INSERT INTO step_graph (id, project_id, name, description)`, `INSERT INTO step_graph_phase (id, graph_id, name, position, fan_out, gate, gate_hard, retry_limit, isolation, command_queue, verify_command, input_kinds, output_kind, template_name, template_version, token_budget)`, `INSERT INTO item_kind (id, project_id, prefix, name, description, default_graph_id, position)`, `INSERT INTO prompt_template (id, project_id, name, version, body, created_by)`; no `RETURNING`, no `SELECT … WHERE EXISTS` guard (the graph is in the same transaction and the ids are ours). The three existing trait writers (`create_step_graph :1562-1583`, `create_phase :1647-1698`, `create_item_kind :1330-1369`) are **not** refactored to share statements: they run on `&self.pool`, return `RETURNING` rows, and `create_item_kind`'s guard refusal reads through the pool (`kind_guard_refusal :2182`), which inside a transaction would not see the uncommitted graph. Four new `.sqlx/query-*.json` files, committed in the same task. | The crate has no `PgExecutor`-generic helper precedent (grep is empty) and a generic helper around `query_as!` would be a new pattern introduced to save four short statements. Per-row execution is 35 round trips inside one transaction on a create that happens once per project; a multi-row `UNNEST` insert would need a `text[][]` parameter for `input_kinds`, which this plan asserts sqlx's Postgres driver will not bind — **the fact-check did not verify that** (F4), so it is a secondary reason only: the decision stands on the absence of a batching precedent in the crate and on a once-per-project cost, and an implementer who finds `text[][]` bindable must still not batch. `demo.rs:187-249,318-334` is the fixture loader and keeps its explicit timestamps; it is not reused. |
| D10 | **Columns the seeder lets the database default: every timestamp column the four tables have** — `created_at` *and* `updated_at` on `step_graph` (`0001_init.sql:214-221`) and `prompt_template` (`:266-275`), and `updated_at` alone on `step_graph_phase` (`:228-247`) and `item_kind` (`:282-292`), **which carry no `created_at` column at all** (fact-check F1 corrected this plan's original "all four tables" wording); all `DEFAULT now()`, with the trigger maintaining `updated_at` afterwards. `project.settings = '{}'` is unchanged from M1 D9. **Everything else is bound explicitly**, including `gate_hard` and `input_kinds` where they equal the column default (`false`, `'{}'`), so the D3 table is the whole truth and a default change in a later migration cannot silently re-seed. No `is_override`-style column is written: it arrives with `0003` defaulting to `false` (PRD D4 `:282-283`, "satisfied by omission"). On `MemStore` the rows carry `now` from the one `Utc::now()` the wrapper takes (`mem.rs:2854`), as `create_phase` does. `project.settings` is not touched by the seed; `set_setting` stays the only writer (PRD risk `:370`). | PRD constraint "nothing sets `updated_at` by hand" (M1 validation); `RETURNING` is not needed because nothing reads the seed rows back inside the transaction. |

## Patterns to Mirror

| Concern | Pattern | Where |
|---|---|---|
| Transaction shape | `let mut tx = self.pool.begin().await.map_err(map_sqlx)?; … tx.commit()`; statements on `&mut *tx` | `pg/write.rs:1043-1086` (`create_project`), `:715`, `:771` |
| Insert without timestamps | `INSERT` omitting `created_at`/`updated_at`; `RETURNING` sees the trigger | `pg/write.rs:1562-1583` (`create_step_graph`), `:1647-1698` (`create_phase`, 16 params) |
| Error mapping | `.map_err(map_sqlx)`; `23505`/`23503` → `Constraint` | `pg/write.rs` throughout; `error.rs` |
| `MemStore` write | `self.write(\|state\| state.create_project(new, now))`, `now` taken once outside | `mem.rs:2854`, `State::create_project :1568-1597` |
| Fixture row building | `catalogue()`'s loops over specs with `demo_uuid(class, stride + i)` | `fixtures.rs:731-816` |
| Whole-row request | `create_phase(&StepGraphPhase)` takes the row the fixture already builds | M1 D10; `conformance.rs:1836+` (`new_phase` helper) |
| Conformance case | fresh project via `new_project(slug)`, `CASE` const in every message, read back through the trait | `conformance.rs:1801-1809`, `:2144-2222` |
| Per-backend twin | named in the case's doc; `pg_criteria.rs` reads raw SQL through `counter()`/`race_kind()` helpers | `pg_criteria.rs:35-58`, `:77-87`; `conformance.rs:3783-3795` |
| Offline sqlx | every new `query!` → `cargo sqlx prepare` → commit `.sqlx/query-*.json` | README `:478-497`; M1 plan Validation |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/seed.rs` | **new** | T1 | D1: `KindSeed`, `PhaseSeed`, `KINDS`, four row constructors, unit tests (D6) |
| `crates/htui-core/src/lib.rs` | edit | T1 | `pub mod seed;` (unconditional) |
| `crates/htui-core/src/fixtures.rs` | edit | T1 | D1/D2: delete `KindSpec`, `KIND_SPECS`, `TEMPLATE_NAMES`; `catalogue()` reads `seed::KINDS` + `DEFAULT_TEMPLATES`; tests `:1736-1769` drop the `TEMPLATE_NAMES` comparison, keep body/version pins |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | D5: `State::create_project` seeds after validation; doc on `:1568`; twins `seeded_templates_carry_the_shipped_bodies`, `seed_never_writes_a_counter_row`, `renamed_prefix_leaves_the_old_counter_row` in the test module |
| `crates/htui-core/src/store/conformance.rs` | edit | T1, then T2 (doc only) | D7: new case + `CASES` entry + `run_case` arm; case 4 doc; case 7 doc names the `mem.rs::` twins in T1 and gains the `pg_criteria.rs::` names in T2 (F2) |
| `crates/htui-core/src/store/traits.rs` | edit | T1 | `create_project` doc `:379-385`: "seeds … inside this same transaction" becomes present tense, names the counts and D3's amendments |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | `:35-41` pin 35 → 36 with its message updated |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | D9/D10: `seed_project` + four `query!`; `create_project` calls it between insert and commit; doc `:1043-1050` rewritten |
| `crates/htui-store/.sqlx/query-*.json` | **new ×4** | T2 | offline data for the four seed statements |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 35 → 36 (`:19`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T2 | D7/D8 twins: `seeded_templates_carry_the_shipped_bodies`, `seed_never_writes_a_counter_row`, `renamed_prefix_leaves_the_old_counter_row` |
| `HANDOFF.md` | edit | T3 | MOD-15 entry `:284-345`: milestone 2 landed, the `:290` `gate_hard` wording corrected to PRD D3's |
| `.claude/prds/mod-15-hierarchy-management.prd.md` | edit | T3 | milestone table `:250`: row 2 → complete, plan link |

Not changed: `migrations/` (no new migration; `0003_orchestration.sql` is MOD-4's), `pg/demo.rs`,
`cache/`, `writer.rs`, `writer_buffered.rs`, `htui-agent/`, `crates/htui/`, `model/`,
`prompt/`.

## Tasks

**T1 → T2 → T3, serial.** T2 needs T1's `htui_core::seed` to compile and T1's `CASES` bump makes
`pg_conformance.rs::case_list_matches_mem_store` red until T2's first edit. T3 records T2's
commits. No task pair has disjoint files *and* no compile dependency, so nothing runs in
parallel. `cargo test --workspace` is red only between T1's `CASES` bump and T2's
`EXPECTED_CASES` bump (one assertion); each task's crate-scoped gate is the live signal.

No trait method is added or changed, so `cargo check -p htui-store` is a complete compile check
for the seam — the two implementors outside it (`UsageSpy`, `SpyStore`) delegate `create_project`
and are not touched.

TDD per task: tests first, red, then code. Every implementer prompt carries: the PRD's D3–D6
and D12 win over this plan where they disagree; graphify-first for codebase questions; `.sqlx`
regenerated and committed with any query change; nothing sets `updated_at` by hand; no new
refusal constant; no new migration; `item_key_counter` is never seeded; `project.settings` is
never written by the seed.

### Task 1: `htui_core::seed`, fixture, `MemStore`, conformance (serial, first)
- **Files**: `crates/htui-core/src/seed.rs` (new), `crates/htui-core/src/lib.rs`,
  `crates/htui-core/src/fixtures.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-core/src/store/conformance.rs`, `crates/htui-core/src/store/traits.rs` (doc only),
  `crates/htui-core/tests/mem_store.rs`.
- **Action**: write the new case (D7 a–g) with D3's table as a literal, add
  `project_create_seeds_the_catalogue` to `CASES` and `run_case`, bump `mem_store.rs` to 36, add
  the three `mem.rs` twins (D7, D8) and the `seed.rs` unit tests (D6: reserved names; every phase
  has a body; 5/15/5/10 counts; dense positions and unique names per graph; position 0
  `input_kinds == []`; `gate_hard` exactly on `(feature, prd)`, `(feature, plan)`,
  `(analysis, verdict)`; every `input_kinds` entry names a phase of the same graph; every prefix
  passes `ItemKind::prefix_is_valid`) — red. Then `seed.rs` (D1, D3), `lib.rs`, `fixtures.rs`
  (D1, D2), `State::create_project` (D5, D10), docs (traits, case 4, case 7) — green.
- **Mirror**: `fixtures.rs:731-816` for row building; `mem.rs:1568-1597` for the create;
  `conformance.rs:2144-2222` for case shape; `fixtures.rs:1736-1769` for the body pins.
- **Gate**: `cargo test -p htui-core --all-features` — **green, including
  `every_cross_referenced_test_name_exists`**: T1's case docs name only the `mem.rs::` twins it
  writes, and the `pg_criteria.rs::` names are appended by T2 (D7, M1 blueprint flag E). Any
  backticked span with four or more underscores in a doc T1 touches must resolve to a fn in
  `conformance.rs` or `mem.rs`, for the same reason.
  `cargo clippy -p htui-core --all-targets --all-features -- -D warnings`;
  `cargo test -p htui --all-features` (snapshots unchanged — D2's claim, checked here).

### Task 2: `PgStore` seeder, `.sqlx`, Postgres twins (after T1)
- **Files**: `crates/htui-store/src/pg/write.rs`, `crates/htui-store/.sqlx/` (four new files),
  `crates/htui-store/tests/pg_conformance.rs`, `crates/htui-store/tests/pg_criteria.rs`,
  `crates/htui-core/src/store/conformance.rs` (doc only: the two case docs gain the
  `pg_criteria.rs::` twin names T2 writes, per D7's split naming rule — this is why T1 and T2
  share a file and why they are serial).
- **Action**: bump `EXPECTED_CASES` to 36; add the three `pg_criteria.rs` twins (D7, D8) using
  `counter()` and the inherent `prompt_templates`, and name them from the two case docs in
  `conformance.rs`; run `pg_conformance` and `pg_criteria` red.
  Then `seed_project` and the four `query!` statements (D9, D10), called from `create_project`
  between the project insert and `tx.commit()`; regenerate `.sqlx`; rewrite the `create_project`
  doc.
- **Mirror**: `pg/write.rs:1043-1086`, `:1562-1583`, `:1647-1698` for statement and binding
  shape (`AS "col: Type"` overrides are not needed: no `RETURNING`).
- **Gate**: `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features`
  (all 36 conformance cases and the twins);
  `cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features`
  then `cargo sqlx prepare --check -- --all-targets --all-features`;
  `cargo clippy -p htui-store --all-targets --all-features -- -D warnings`;
  `every_cross_referenced_test_name_exists` green.

### Task 3: docs (after T2)
- **Files**: `HANDOFF.md`, `.claude/prds/mod-15-hierarchy-management.prd.md`.
- **Action**: milestone row 2 → complete with this plan linked; HANDOFF's MOD-15 entry gains
  milestone 2's commit range and test count, and its `:290` `gate_hard` sentence is rewritten
  to PRD D3's three flags.
- **Gate**: `cargo doc --workspace --no-deps`; `git diff --stat` touches only the two files.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)
cargo doc --workspace --no-deps
cargo run -p htui -- --demo                                  # unchanged UI; smoke only
```
`cargo sqlx prepare` (not `--check`) needs `htui_sqlx` migrated per README `:486-493`. Check
`df -h /` before blaming Postgres for a crash loop (memory: `target/` fills the disk).

## Verified claims (fact-check, 2026-09-16)
Checked against the working tree by `/handoff-run` before the CONFIRM gate. Four findings (F1–F4)
amended the plan above; every amendment is marked with its finding number where it lands.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | `create_project` on both stores is unseeded and transaction-shaped; `NewProject` has five fields | **true** | `pg/write.rs:1043-1086` (`pool.begin()`, one insert, commit, doc naming milestone 2); `mem.rs:1568-1597`, `:2854`; `hierarchy.rs:85-96` lists `id, slug, name, description, created_by` |
| V2 | The fixture's `catalogue()` seeds `gate_hard: false` and `input_kinds` = previous phase only, from `KIND_SPECS` and `TEMPLATE_NAMES` | **true** | `fixtures.rs:668-699` (five kinds), `:706-717` (ten names), `:777` (`gate_hard: false`), `:779-782` (`position.checked_sub(1)` → the previous phase alone) |
| V3 | No test or snapshot pins seeded `gate_hard`/`input_kinds` values; `prompt/fixtures.rs` reads no store | **true** | `rg 'gate_hard\|input_kinds'` over `crates/*/tests`, `crates/htui/src`: only prose — `preview.rs:8,77`, `prompt_preview.rs:180`, the `DOCUMENTS_NOTE` line of `prompt_preview__preview_feat_1.snap:27`, and `prompt_golden.rs:61,71` (a *position-0* phase, which the amendments leave at `[]`); `prompt/fixtures.rs:1-11` states "no clock, no `Uuid::new_v4`, no store read" |
| V4 | `WriteStore` has no `prompt_template` reader; inherent readers exist on both stores | **true** | `traits.rs` carries only `DeleteReach.prompt_templates` (`:761`); inherent `MemStore::prompt_templates` (`mem.rs:293`), `PgStore::prompt_templates` (`pg/read.rs:892`) |
| V5 | Case 7 asserts key text and `ANL-1` but not the old counter row | **true** | `conformance.rs:2450-2451` doc claims all three facts; the body asserts `"ANA-2"` and `minted.key == "ANL-1"` (`:2530-2562`) and reads no counter — no trait reader exists |
| V6 | `CASES.len()` and `EXPECTED_CASES` are 35; `pg_conformance` is one database per case | **true** | `mem_store.rs:35-41` (35, message naming M1's twelve); `pg_conformance.rs:19` (`EXPECTED_CASES = 35`), `:30-43` (`demo_db()` then `db.drop_db()` inside the per-case loop) |
| V7 | `created_at`/`updated_at` default to `now()` on the four seed tables; `item_kind.default_graph_id NOT NULL` | **false as written → F1** | `step_graph` (`:214-221`) and `prompt_template` (`:266-275`) have both columns; `step_graph_phase` (`:228-247`) and `item_kind` (`:282-292`) have **`updated_at` only, no `created_at`**. All present timestamps are `DEFAULT now()`; `default_graph_id UUID NOT NULL REFERENCES step_graph(id)` confirmed, as are `gate_hard … DEFAULT false` and `input_kinds TEXT[] NOT NULL DEFAULT '{}'`. D10 amended |
| V8 | PRD D3 flags exactly three phases; ANA-2 `:307-315` amends exactly two `input_kinds` lists | **true** | PRD `:276-279` (feature `prd`/`plan`, analysis `verdict`, "none elsewhere", explicitly against HANDOFF `:290`); ANA-2 `:310-315` (implement gains `review`; bug's `fix` becomes `['reproduce','review']`), `:317-319` freezes the rest; ANA-2 `:2053` open item 5 agrees; PRD D5 `:285-287` extends `review` to CLEAN and TOOL |
| V9 | No `PgExecutor`-generic helper exists in `htui-store` | **true** | `rg PgExecutor crates/` is empty |
| V10 | The fixture counter for `(PROJECT_HTUI, "ANA")` is 2 | **true** | `fixtures.rs:822`; `counter()` helper at `pg_criteria.rs:77-87` returns `Option<i32>` |
| V11 | A case doc may not name a `pg_criteria.rs::` twin before that file has it | **true → F2** | `every_cross_referenced_test_name_exists` reads `../htui-store/tests/pg_criteria.rs` off disk at run time (`conformance.rs:3783-3795`); M1 blueprint flag E (`mod-15-hierarchy-seam.blueprint.md:19`) records the same trap and its split-naming fix. D7 and T1/T2 amended |
| V12 | Deleting `TEMPLATE_NAMES` is doc-and-test work, not just a constant move | **true → F3** | `ten_templates_per_project_from_the_default_bodies` (`fixtures.rs:1736-1769`) reads `super::TEMPLATE_NAMES` twice; `template_ids_are_distinct_across_projects` (`:1774-1797`) documents the stride as `TEMPLATE_NAMES.len()`. D1 amended |
| V13 | `mint_item` on a freshly created project needs nothing the seed does not give it (D7f) | **true** | `State::mint` (`mem.rs:887-930`) checks only that the kind exists, belongs to the project, the id is free and `created_by` is an author; no workspace membership, no counter row (`.entry(..).or_insert(0)`). `new_item` (`conformance.rs:219-233`) passes `box_id: Some(ids::BOX)`, which both fixture databases hold |
| V14 | sqlx's Postgres driver cannot bind `text[][]`, so a batched `UNNEST` seed is out | **unverified → F4** | No probe run; no nested-array binding exists in the tree to read off. Recorded as a secondary reason only — D9 stands on the missing batching precedent and the once-per-project cost |
| V15 | Tasks are serial and their file sets are stated | **true, no parallel marking to strip** | T1 `htui-core` only; T2 `htui-store` plus one doc-only edit in `conformance.rs` (F2); T3 two docs. T2 cannot compile before T1's `seed` module exists, so no pair is independent |

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| D3's table in this plan and `seed::KINDS` disagree after a review edit to one of them | Medium | High (the "exact seed" metric) | The conformance case asserts a **literal** copy of the table (D7c); the `seed.rs` unit tests pin the flags by name; the fact-check reads D3 against ANA-2 and PRD D3 (V8) |
| Fixture amendment changes a rendered screen nobody grepped for | Low | Medium | T1's gate runs `cargo test -p htui --all-features` (insta snapshots); D2's verification list is in the plan for the reviewer to re-run |
| `MemStore` seed inserted before a validation that can fail | Low | Medium | D5 orders every fallible check first; the case's (g) asserts nothing survives an unknown `created_by` on both stores |
| Postgres seed statement fails inside the transaction on a real project (a constraint the unit tests do not model) | Low | Low (rollback) | The four inserts bind every `NOT NULL` column; `sqlx query!` checks columns at compile time against `htui_sqlx`; the conformance case runs the real statements |
| `.sqlx` left stale after a query edit breaks `SQLX_OFFLINE` builds | Medium | High | `cargo sqlx prepare --check` in T2's gate and in Acceptance |
| A case doc names a `pg_criteria.rs::` twin before T2 writes it, reddening T1 | Low | Low | D7's split naming rule (fact-check F2): T1 names `mem.rs::` twins only, T2 appends the rest with the tests |
| One more create/drop cycle in `pg_conformance` | Certain | Low | Budgeted in D7: one case for seven assertions |
| MOD-4's `0003` re-seeds or re-defaults phase columns and the seed now writes them explicitly | Low | Low | D10: explicit binding means a default change never re-seeds silently; the case fails loudly if a column is dropped or renamed |
| The seed's `created_by` is a user the database does not have (first project on a fresh install) | Medium | Medium | Not this milestone's: `NewProject.created_by` is the caller's, and milestone 3's create screen passes the signed-in user; the case's (g) pins the refusal |

## Acceptance
- [ ] `htui_core::seed` exists, always compiled; `KINDS` is D3's table; `fixtures.rs` has no `KindSpec`, `KIND_SPECS` or `TEMPLATE_NAMES` and builds through `seed`'s constructors with `demo_uuid` ids
- [ ] The demo fixture carries the amendments; `cargo test -p htui --all-features` snapshots unchanged
- [ ] `create_project` on `MemStore` and `PgStore` seeds 5 graphs, 15 phases, 5 kinds, 10 templates; `NewProject`, the trait signature and the six implementors unchanged
- [ ] `CASES.len() == 36`, `EXPECTED_CASES == 36`, `READ_CASES` unchanged at 6; all 36 pass on `MemStore` and, with the env var, on `PgStore`
- [ ] The new case asserts D3's table as a literal, the reserved-name rule on read-back, `delete_reach` counts `(5, 15, 5, 10)`, `FEAT-1` on first mint, and nothing left behind on an unknown `created_by`
- [ ] `seed.rs` unit tests pin: no reserved phase name, every phase has a body, `gate_hard` on exactly three named phases, position 0 `input_kinds == []`, dense positions, valid prefixes
- [ ] Twins `seeded_templates_carry_the_shipped_bodies`, `seed_never_writes_a_counter_row`, `renamed_prefix_leaves_the_old_counter_row` exist in both `mem.rs` and `pg_criteria.rs` and are named from the cases' docs; `every_cross_referenced_test_name_exists` green
- [ ] No seed statement binds `created_at`/`updated_at`; grep `SET updated_at` in `pg/write.rs` is empty; `item_key_counter` and `project.settings` untouched by the seed
- [ ] Four new `.sqlx/query-*.json` committed; `cargo sqlx prepare --check -- --all-targets --all-features` passes from `crates/htui-store`
- [ ] No file under `migrations/`, `cache/`, `pg/demo.rs`, `writer*.rs`, `htui-agent/`, `crates/htui/` changed; `unsafe_code = "forbid"`, MSRV 1.98, lint set untouched
- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo doc` clean on the workspace
