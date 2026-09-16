# Blueprint: MOD-15 milestone 2 — a created project is a working project

Elaborates `.claude/plans/mod-15-project-seed.plan.md` (D1–D10, one case name final). PRD D3, D4,
D5, D6, D12 and the Constraints win on conflict. Tree at `1e227e8`. Fact-check F1–F4 honoured:
T1/T2/T3 serial; `step_graph_phase` and `item_kind` have `updated_at` only; T1 doc comments name
`mem.rs::` twins only; no `UNNEST` batching. Milestone 1's blueprint is
`.claude/plans/mod-15-hierarchy-seam.blueprint.md` (its flag E is this plan's F2).

## 0. Flags — read before building

The plan is buildable but the things below are wrong, under-specified, or would redden a gate as
written. None re-opens D1–D10; each is the smallest fix that keeps the PRD's rule.

| # | Where | Problem | Resolution used below |
|---|---|---|---|
| **A** | D1 constructors `graph_row(..) -> NewStepGraph`, `kind_row(..) -> NewItemKind` | Neither caller wants a request struct. `fixtures.rs::catalogue()` pushes full `StepGraph` / `ItemKind` rows with `epoch()` timestamps into `DemoData` (`fixtures.rs:749-766`, `:314-316`); `MemStore`'s maps are `graphs: HashMap<StepGraphId, StepGraph>` and `kinds: HashMap<ItemKindId, ItemKind>` (`mem.rs:71-73`). A `NewStepGraph` would have to be re-wrapped with a timestamp at both sites — two more copies of the row shape, which is what D1 exists to remove. | All four constructors return the **row** type and take `now`: `graph_row(id, project_id, &KindSeed, now) -> StepGraph`, `kind_row(id, project_id, graph_id, position, &KindSeed, now) -> ItemKind`; `phase_row` and `template_row` as the plan has them. `PgStore` binds the non-timestamp columns off the row (D10); `now` is discarded there. §1. |
| **B** | D7(e) "`delete_reach(Project(p))` reports `step_graphs 5, phases 15, item_kinds 5, prompt_templates 10`" | `DeleteReach` has 21 fields (`traits.rs:745-789`), including `item_key_counters` and `command_runs`; a whole-struct literal would pin 17 zeros the case is not about, and M1 flag F already recorded that two of them are `0` on both stores for a reason unrelated to the seed. | Assert the four counts as one tuple and, separately, `(items, item_key_counters) == (0, 0)` — the second tuple is D5's "counter never seeded" seen through the trait. §5. |
| **C** | D9 "Four new `.sqlx/query-*.json` files" and T2's twins | Only true if `pg_criteria.rs` adds no `query!`/`query_scalar!` string of its own. `pg_criteria.rs:102-103` states the rule: "a new `query!` string would need a `cargo sqlx prepare` pass, and `.sqlx/` belongs to the crate, not to a test". | The three twins use the existing `counter()` helper (`pg_criteria.rs:78-87`, already prepared) and runtime `sqlx::query_scalar::<_, i64>(..)` for the two raw counts (precedent `:2636-2650`). `cargo sqlx prepare` then produces exactly four new files; the gate's `git status --porcelain crates/htui-store/.sqlx` shows `?? ×4` and nothing else. §6. |
| **D** | D6/D7 "pinned three ways", T1 gate | `every_cross_referenced_test_name_exists` (`conformance.rs:3783-3848`) resolves `file.rs::name` only for `conformance`, `mem`, `pg_criteria` and **panics** on any other file (`:3822-3826`); a bare snake_case span with ≥ 4 underscores must be a `fn` in `conformance.rs` or `mem.rs` (`:3835-3841`). `no_seed_phase_is_a_reserved_template_role` (7 underscores) lives in `seed.rs`; written in backticks anywhere in `conformance.rs` it fails T1 either way. | The case doc refers to "the `seed` module's unit tests" in prose and never backticks a `seed.rs` test name. Same rule for `every_phase_name_has_a_shipped_body` etc. §5, §8. |
| **E** | D7/D8 twins in `pg_criteria.rs` | The file is `#![cfg(feature = "demo")]` (`:13`) and every test starts from `common::demo_db()` — the three fixture projects, fixture user `ids::USER`, fixture kinds. The plan does not say which project each twin runs on. | `seeded_templates_carry_the_shipped_bodies` and `seed_never_writes_a_counter_row` create a **fresh** project through `db.store.create_project(..)` with `created_by: ids::USER` (the only user the demo database has after `demo_db()` deletes the seeded one, `testkit.rs:170-193`); `renamed_prefix_leaves_the_old_counter_row` runs on `ids::PROJECT_HTUI` / `ids::KIND_HTUI_ANA`, the fixture counter being `2` (`fixtures.rs:822`). Same split for the `mem.rs` twins on `MemStore::demo()`. §6, §3. |
| **F** | D1 "`pub mod seed`" | `htui-core` is `#![warn(missing_docs)]` (`lib.rs:9`) and the workspace warns `missing_debug_implementations` and `unused_qualifications` (`Cargo.toml:90-93`); clippy is `-D warnings`. Every `pub` item **and every pub field** in `seed.rs` needs a doc line; `KindSeed`/`PhaseSeed` need `Debug`. | `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` on both structs, doc on every field, `#[must_use]` on the four constructors; `pub mod seed;` goes after `pub mod scrub;` at `lib.rs:13`, outside the `demo` gate. §1. |
| **G** | Files table `tests/mem_store.rs` "pin 35 → 36 with its message updated" | The message (`mem_store.rs:38-40`) is an inventory: "B.9's fifteen cases, MOD-2's five … MOD-15 milestone 1's twelve". A count bump without the inventory line makes the next reader recount. | Append "and milestone 2's seed case (plan D7)" to the string. Same for `pg_conformance.rs:19`'s constant, whose doc needs no change. §5. |
| **H** | T3 "HANDOFF's MOD-15 entry gains milestone 2's commit range" | `.claude/rules/workflow-docs.md`: a multi-phase item stays **one** checklist line and each phase appends a `**Milestone N landed (commit)**` note; nothing is archived until the whole MOD closes. Milestone 1's note is the paragraph at `HANDOFF.md:317-345`. | T3 appends a "Milestone 2 landed" paragraph after milestone 1's, edits the `:290` `gate_hard` sentence in place, and does not touch the checkbox. §7. |
| **I** | Files table "Not changed: `migrations/`" | There is no `migrations/` at the repo root; the migrations are `crates/htui-store/migrations/` (README `:478-497`, `--source crates/htui-store/migrations`). Harmless in the plan, but an implementer who greps for `0001_init.sql` at the root finds nothing. | Cited as `crates/htui-store/migrations/0001_init.sql:214-295` below. Still not changed. |
| **J** | D3 "`created_by = NewProject.created_by`" vs `fixtures.rs:801` `created_by: ids::USER` | Both are right for their caller; the plan's `template_row(id, project_id, name, body, created_by, now)` already takes it as a parameter. Stated here so nobody "fixes" the fixture to read `PROJECT_SPECS` for a user it does not carry. | The fixture passes `ids::USER`; `MemStore`/`PgStore` pass `new.created_by`. §2, §3, §4. |
| **K** | D9's `INSERT INTO step_graph_phase (…)` column order | D9 lists `name, position, … isolation, command_queue, verify_command, input_kinds, output_kind …`; `create_phase`'s statement (`pg/write.rs:1647-1698`, the mirror T2 names) is `position, name, fan_out, gate, gate_hard, retry_limit, input_kinds, output_kind, isolation, command_queue, verify_command, template_name, template_version, token_budget`. D9's list is a set; two orders for the same 16 columns is a diff nobody can review. | The seeder's statement uses `create_phase`'s order, and its parameters follow that order. §4. |
| **L** | D7 twin `seeded_templates_carry_the_shipped_bodies`: "names = `DEFAULT_TEMPLATES` names" | Both inherent readers order by `name` bytes (`mem.rs:293-309`, `pg/read.rs:892` with `COLLATE "C"`), not by `DEFAULT_TEMPLATES` order (`prd, plan, …, handoff`). An in-order comparison fails on both stores. | The twins compare against `DEFAULT_TEMPLATES`' names **sorted by bytes**; the per-name body check is order-free. §3, §6. |

## 1. `htui_core::seed` (`crates/htui-core/src/seed.rs`, new; `lib.rs` `pub mod seed;`)

Unconditional module: `MemStore::create_project` needs it in every build, so it cannot share
`fixtures`' `#[cfg(feature = "demo")]` (`lib.rs:16-17`). It depends on `model` (row types,
`ItemKind::prefix_is_valid`) and `prompt` (`DEFAULT_TEMPLATES`, `body_of`, `TemplateRole`) and
nothing depends on it except `fixtures`, `store::mem` and `htui-store`.

`lib.rs` after the edit:

```rust
pub mod model;
pub mod prompt;
pub mod scrub;
pub mod seed;
pub mod store;

#[cfg(feature = "demo")]
pub mod fixtures;
```

The module, in full. Every doc line is load-bearing under `missing_docs`.

```rust
//! The catalogue every project is born with (MOD-15 milestone 2, plan D1–D3).
//!
//! `R-ENT-6` fixes five item kinds, each with a default step graph; ANA-5 §5.4 fixes ten prompt
//! templates. One table for both of this crate's row sources: `store::mem` and `htui-store`'s
//! `PgStore` build a project's thirty-five rows from it with fresh UUIDv7 ids, and the demo
//! fixture builds the same rows with its deterministic `demo_uuid` ids. There is no second copy
//! anywhere, which is what makes PRD risk "seed and fixture drift" structurally impossible.
//!
//! The table is ANA-2 §4.1 as amended (`docs/ANA-2.md:307-319`): `implement` takes `plan` *and*
//! `review` (PRD D5 extends that to `refactor` and `tooling`), `bug`'s `fix` takes `reproduce`
//! and `review`, and `gate_hard` is set on exactly three phases — `feature`'s `prd` and `plan`,
//! `analysis`'s `verdict` (PRD D3, which overrides HANDOFF's "prd, plan and verdict" wording).
//! Every other phase column is ANA-2's frozen default, written by [`phase_row`].
//!
//! The reserved template names `judge` and `handoff` ([`TemplateRole::of_name`]) are never phase
//! names here; the unit tests pin that, and nothing re-checks it at runtime (plan D6).

use chrono::{DateTime, Utc};

use crate::model::{
    CommandQueue, Gate, ItemKind, ItemKindId, PhaseId, ProjectId, PromptTemplate,
    PromptTemplateId, StepGraph, StepGraphId, StepGraphPhase, UserId,
};

/// One phase of a default graph: the three columns the table varies, in position order within
/// [`KindSeed::phases`]. Everything else on the row is [`phase_row`]'s frozen default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseSeed {
    /// `step_graph_phase.name`; also its `output_kind` and `template_name`.
    pub name: &'static str,
    /// `step_graph_phase.input_kinds`: names of phases of the **same** graph whose documents this
    /// phase reads. Empty at position 0 on every graph.
    pub input_kinds: &'static [&'static str],
    /// `step_graph_phase.gate_hard` (PRD D3).
    pub gate_hard: bool,
}

/// One `R-ENT-6` kind with its default graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindSeed {
    /// `item_kind.prefix`, e.g. `FEAT`; passes [`ItemKind::prefix_is_valid`].
    pub prefix: &'static str,
    /// `item_kind.name`, which is also `step_graph.name`.
    pub name: &'static str,
    /// `item_kind.description`.
    pub description: &'static str,
    /// The default graph's phases, position order.
    pub phases: &'static [PhaseSeed],
}

/// The five kinds, in `item_kind.position` order (plan D3).
pub const KINDS: [KindSeed; 5] = [
    KindSeed {
        prefix: "ANA",
        name: "analysis",
        description: "A question answered in writing",
        phases: &[
            PhaseSeed { name: "research", input_kinds: &[], gate_hard: false },
            PhaseSeed { name: "verdict", input_kinds: &["research"], gate_hard: true },
        ],
    },
    KindSeed {
        prefix: "FEAT",
        name: "feature",
        description: "New behaviour",
        phases: &[
            PhaseSeed { name: "prd", input_kinds: &[], gate_hard: true },
            PhaseSeed { name: "plan", input_kinds: &["prd"], gate_hard: true },
            PhaseSeed { name: "implement", input_kinds: &["plan", "review"], gate_hard: false },
            PhaseSeed { name: "review", input_kinds: &["implement"], gate_hard: false },
        ],
    },
    KindSeed {
        prefix: "FIX",
        name: "bug",
        description: "Behaviour that is wrong",
        phases: &[
            PhaseSeed { name: "reproduce", input_kinds: &[], gate_hard: false },
            PhaseSeed { name: "fix", input_kinds: &["reproduce", "review"], gate_hard: false },
            PhaseSeed { name: "review", input_kinds: &["fix"], gate_hard: false },
        ],
    },
    KindSeed {
        prefix: "CLEAN",
        name: "refactor",
        description: "Behaviour kept, shape improved",
        phases: &[
            PhaseSeed { name: "plan", input_kinds: &[], gate_hard: false },
            PhaseSeed { name: "implement", input_kinds: &["plan", "review"], gate_hard: false },
            PhaseSeed { name: "review", input_kinds: &["implement"], gate_hard: false },
        ],
    },
    KindSeed {
        prefix: "TOOL",
        name: "tooling",
        description: "The workshop rather than the product",
        phases: &[
            PhaseSeed { name: "plan", input_kinds: &[], gate_hard: false },
            PhaseSeed { name: "implement", input_kinds: &["plan", "review"], gate_hard: false },
            PhaseSeed { name: "review", input_kinds: &["implement"], gate_hard: false },
        ],
    },
];

/// Phases across all of [`KINDS`]: fifteen. The fixture's per-project phase id stride.
pub const PHASES_PER_PROJECT: usize = {
    let mut total = 0;
    let mut i = 0;
    while i < KINDS.len() {
        total += KINDS[i].phases.len();
        i += 1;
    }
    total
};

/// `step_graph.description` of a kind's default graph.
#[must_use]
pub fn graph_description(kind_name: &str) -> String {
    format!("Default graph for {kind_name} items")
}

/// A kind's default `step_graph` row. `now` fills both timestamp columns; a Postgres seeder
/// leaves those to the database and discards it (plan D10).
#[must_use]
pub fn graph_row(
    id: StepGraphId,
    project_id: ProjectId,
    kind: &KindSeed,
    now: DateTime<Utc>,
) -> StepGraph {
    StepGraph {
        id,
        project_id,
        name: kind.name.to_owned(),
        description: graph_description(kind.name),
        created_at: now,
        updated_at: now,
    }
}

/// A `step_graph_phase` row at `position` of `graph_id`: the seed's three columns plus ANA-2
/// §4.1's frozen defaults — `fan_out 1`, `gate Always`, `retry_limit 1`, `isolation None`,
/// `command_queue FanOutOnly`, `verify_command None`, `output_kind` and `template_name` equal to
/// the name, `template_version None` (follow latest), `token_budget None` (project default).
#[must_use]
pub fn phase_row(
    id: PhaseId,
    graph_id: StepGraphId,
    position: i32,
    phase: &PhaseSeed,
    now: DateTime<Utc>,
) -> StepGraphPhase {
    StepGraphPhase {
        id,
        graph_id,
        position,
        name: phase.name.to_owned(),
        fan_out: 1,
        gate: Gate::Always,
        gate_hard: phase.gate_hard,
        retry_limit: 1,
        input_kinds: phase.input_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
        output_kind: phase.name.to_owned(),
        isolation: None,
        command_queue: CommandQueue::FanOutOnly,
        verify_command: None,
        template_name: phase.name.to_owned(),
        template_version: None,
        token_budget: None,
        updated_at: now,
    }
}

/// An `item_kind` row whose default graph is `graph_id` (the row [`graph_row`] built for the
/// same `kind`) and whose `position` is the kind's index in [`KINDS`].
#[must_use]
pub fn kind_row(
    id: ItemKindId,
    project_id: ProjectId,
    graph_id: StepGraphId,
    position: i32,
    kind: &KindSeed,
    now: DateTime<Utc>,
) -> ItemKind {
    ItemKind {
        id,
        project_id,
        prefix: kind.prefix.to_owned(),
        name: kind.name.to_owned(),
        description: kind.description.to_owned(),
        default_graph_id: graph_id,
        position,
        updated_at: now,
    }
}

/// A version-1 `prompt_template` row. Callers iterate [`crate::prompt::DEFAULT_TEMPLATES`] and
/// pass its `(name, _, body)`; `created_by` is the project's creator on the stores and the
/// fixture user in `fixtures.rs`.
#[must_use]
pub fn template_row(
    id: PromptTemplateId,
    project_id: ProjectId,
    name: &str,
    body: &str,
    created_by: UserId,
    now: DateTime<Utc>,
) -> PromptTemplate {
    PromptTemplate {
        id,
        project_id,
        name: name.to_owned(),
        version: 1,
        body: body.to_owned(),
        created_by,
        created_at: now,
        updated_at: now,
    }
}
```

Unit tests, in the same file's `#[cfg(test)] mod tests`. They run under plain
`cargo test -p htui-core` (no feature): nothing here may reach for `crate::fixtures`. Names
carry ≥ 4 underscores on purpose — they are **never** written in backticks inside
`conformance.rs` (flag D).

| Test | Pins (plan T1's list) |
|---|---|
| `five_kinds_and_fifteen_phases` | `KINDS.len() == 5`; `PHASES_PER_PROJECT == 15`; `KINDS.iter().map(\|k\| k.phases.len()).collect::<Vec<_>>() == [2, 4, 3, 3, 3]` |
| `kind_names_and_prefixes_are_unique` | five distinct `name`s, five distinct `prefix`es (`HashSet` sizes) |
| `every_prefix_passes_the_check` | `ItemKind::prefix_is_valid(kind.prefix)` for all (`model/kind.rs:80-87`) |
| `no_seed_phase_is_a_reserved_template_role` | `TemplateRole::of_name(phase.name) == TemplateRole::Phase` for every phase of every kind (D6.1) |
| `every_phase_name_has_a_shipped_body` | `crate::prompt::body_of(phase.name).is_some()` for every phase (D6.3); and `DEFAULT_TEMPLATES.len() == 10` with `judge`/`handoff` present |
| `names_are_unique_and_positions_dense_per_graph` | per kind: distinct phase names; `phase_row(.., i as i32, ..)` gives `position == i` — the density is by construction, so the assertion is that `phases` has no duplicate name |
| `position_zero_takes_no_inputs` | `kind.phases[0].input_kinds.is_empty()` for every kind |
| `gate_hard_is_exactly_the_three_prd_flags` | collect `(kind.name, phase.name)` where `gate_hard`; equals `[("analysis","verdict"), ("feature","prd"), ("feature","plan")]` in table order |
| `every_input_kind_names_a_phase_of_the_same_graph` | for every phase, every `input_kinds` entry is some `phases[j].name` of the same kind |
| `rows_carry_the_frozen_defaults` | one `phase_row` call: `(fan_out, gate, retry_limit, isolation, command_queue, verify_command, template_version, token_budget) == (1, Gate::Always, 1, None, CommandQueue::FanOutOnly, None, None, None)`, `output_kind == template_name == name`, `input_kinds` cloned in order, `updated_at == now`; one `graph_row`: description `"Default graph for feature items"`, both timestamps `now`; one `kind_row`: `default_graph_id`, `position`, `updated_at`; one `template_row`: `version == 1`, `created_by` passed through, both timestamps `now` |

`Gate`, `CommandQueue`, `Isolation` derive `PartialEq` (`model/kind.rs:10-47` via `str_enum!`),
`TemplateRole` derives `Debug, PartialEq` (`prompt/template.rs:37`), so `assert_eq!` works
throughout.

## 2. `fixtures.rs` diff (`crates/htui-core/src/fixtures.rs`)

**Deleted** (`:655-717`): `struct KindSpec`, `const KIND_SPECS: [KindSpec; 5]`,
`const TEMPLATE_NAMES: [&str; 10]`, with their doc comments.

**Imports** (`:18-24`): add `use crate::prompt::DEFAULT_TEMPLATES;` and `use crate::seed;`.
After `catalogue()` is rewritten, `Gate` and `CommandQueue` are no longer named in the file
unless another fixture builder uses them — let `cargo check --all-features` say; an unused import
is `-D warnings` red. `ItemKind`, `StepGraph`, `StepGraphPhase`, `PromptTemplate` stay (the
`DemoData` fields at `:314-320` and the `catalogue()` return type).

**`catalogue()`** (`:721-816`) becomes:

```rust
/// Kinds, their default graphs, the graphs' phases and one template version per default
/// template name, all built through `seed`'s constructors with `demo_uuid` ids.
///
/// Ids follow blueprint §G: kind and graph `n` are `project_index * KINDS.len() + kind_index`,
/// phases are numbered across the whole project (`seed::PHASES_PER_PROJECT` per project),
/// templates are `project_index * DEFAULT_TEMPLATES.len() + template_index`.
///
/// Every stride is the length of the table it indexes and not a literal, because it is a
/// primary-key input and not a formatting choice: [`demo_uuid`] is a pure function of
/// `(class, n)`, so a stride below the number of rows per project hands project *i*'s first row
/// the id project *i−1*'s late rows already hold, and `load_demo` fails on the second insert
/// (blueprint E-5, when the template stride was `8` for eight names).
fn catalogue() -> (
    Vec<ItemKind>,
    Vec<StepGraph>,
    Vec<StepGraphPhase>,
    Vec<PromptTemplate>,
) {
    let kinds_per_project = seed::KINDS.len() as u8;
    let phases_per_project = seed::PHASES_PER_PROJECT as u8;
    let templates_per_project = DEFAULT_TEMPLATES.len() as u8;

    let mut kinds = Vec::new();
    let mut graphs = Vec::new();
    let mut phases = Vec::new();
    let mut templates = Vec::new();

    for (project_index, (project_id, _, _, _)) in PROJECT_SPECS.iter().enumerate() {
        let project_index = project_index as u8;
        let mut phase_n = project_index * phases_per_project;

        for (kind_index, kind) in seed::KINDS.iter().enumerate() {
            let n = project_index * kinds_per_project + kind_index as u8;
            let graph = seed::graph_row(
                StepGraphId::from_uuid(demo_uuid(class::STEP_GRAPH, n)),
                *project_id,
                kind,
                epoch(),
            );
            let graph_id = graph.id;
            graphs.push(graph);
            kinds.push(seed::kind_row(
                ItemKindId::from_uuid(demo_uuid(class::ITEM_KIND, n)),
                *project_id,
                graph_id,
                kind_index as i32,
                kind,
                epoch(),
            ));

            for (position, phase) in kind.phases.iter().enumerate() {
                phases.push(seed::phase_row(
                    PhaseId::from_uuid(demo_uuid(class::PHASE, phase_n)),
                    graph_id,
                    position as i32,
                    phase,
                    epoch(),
                ));
                phase_n += 1;
            }
        }

        for (template_index, (name, _, body)) in DEFAULT_TEMPLATES.iter().enumerate() {
            templates.push(seed::template_row(
                PromptTemplateId::from_uuid(demo_uuid(
                    class::PROMPT_TEMPLATE,
                    project_index * templates_per_project + template_index as u8,
                )),
                *project_id,
                name,
                body,
                ids::USER,
                epoch(),
            ));
        }
    }

    (kinds, graphs, phases, templates)
}
```

Id preservation (plan D2): `n` for kinds/graphs is unchanged (`project_index * 5 + kind_index`);
`phase_n` starts at `project_index * 15` and increments in the same graph-then-position walk, so
`ids::PHASE_HTUI_IMPLEMENT = (PHASE, 4)` (`:280`) is still `feature`'s `implement` (analysis
takes 0–1, feature 2–5); templates are `project_index * 10 + i` as before. The
`body_of(name).expect(..)` at `:798-800` goes: the body now comes from `DEFAULT_TEMPLATES` itself.

**Tests** (`mod tests`, `:1594`):

- `each_project_carries_the_five_seeded_kinds` (`:1711-1728`): unchanged — 5/15/45/30.
- `ten_templates_per_project_from_the_default_bodies` (`:1736-1769`): delete the first
  `assert_eq!` (`:1738-1745`, `TEMPLATE_NAMES` vs `DEFAULT_TEMPLATES` names — tautological now).
  The per-project loop compares `names` against
  `DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect::<Vec<_>>()` with the same "ten per
  project, in order" message (the fixture pushes in `DEFAULT_TEMPLATES` order, so an in-order
  comparison holds **here** — it is the store readers that sort, flag L). Body, parse and
  `version == 1` pins stay verbatim.
- `template_ids_are_distinct_across_projects` (`:1771-1790`): doc's last sentence becomes "at
  ten they only agree because the stride is `DEFAULT_TEMPLATES.len()` (blueprint E-5)"; body
  unchanged.
- **New** `the_fixture_phases_are_the_seed_rows`: for `ids::PROJECT_HTUI`, for each
  `(kind_index, kind)` in `seed::KINDS`, the fixture phases whose `graph_id` is
  `demo_uuid(class::STEP_GRAPH, kind_index)` map to `(name, input_kinds, gate_hard)` equal to
  `kind.phases`' — and specifically the row with id `ids::PHASE_HTUI_IMPLEMENT` has
  `input_kinds == ["plan", "review"]` and `name == "implement"`. This is D2's "the demo fixture
  takes the amendments" as a test rather than a claim, and it is what catches a future stride
  edit that shifts `PHASE_HTUI_IMPLEMENT` onto another phase.

## 3. `MemStore` (`crates/htui-core/src/store/mem.rs`)

Imports (`:36-40` region): add `use crate::prompt::DEFAULT_TEMPLATES;` and `use crate::seed;`;
add `PromptTemplateId` to the `crate::model` list if it is not there (`PhaseId`, `ItemKindId`,
`StepGraphId` already are — `create_phase`/`create_step_graph`/`create_item_kind` use them).

`State::create_project` (`:1568-1597`) — the whole new body:

```rust
    /// A project with `settings = {}` and no secret provider (M1 D9), plus the thirty-five rows
    /// `seed` gives every project — five graphs, fifteen phases, five kinds, ten templates (M2
    /// D4).
    ///
    /// Validate, then mutate: `require_user`, the duplicate id and the duplicate slug all fire
    /// before the first insert, and nothing after it can fail — the rows come from `seed::KINDS`
    /// and `DEFAULT_TEMPLATES`, whose self-consistency is the `seed` module's unit tests' to
    /// prove, not this fn's to re-check (D5). None of `create_step_graph`, `create_phase` or
    /// `create_item_kind` is called: each validates against the maps and a refusal midway would
    /// leave a half-seeded project. `item_key_counter` is untouched; `mint` creates the row
    /// lazily. `settings` stays `{}`: `set_setting` is that column's only writer.
    fn create_project(&mut self, new: NewProject, now: DateTime<Utc>) -> Result<Project> {
        self.require_user(new.created_by, "project.created_by")?;
        if self.projects.contains_key(&new.id) {
            return Err(StoreError::Constraint(format!(
                "project `{}` already exists",
                new.id
            )));
        }
        if self.projects.values().any(|row| row.slug == new.slug) {
            return Err(StoreError::Constraint(format!(
                "project.slug `{}` is taken",
                new.slug
            )));
        }

        let created_by = new.created_by;
        let row = Project {
            id: new.id,
            slug: new.slug,
            name: new.name,
            description: new.description,
            secret_provider: None,
            secret_scope: None,
            settings: Value::Object(serde_json::Map::new()),
            created_by,
            created_at: now,
            updated_at: now,
        };
        let project_id = row.id;
        self.projects.insert(project_id, row.clone());

        for (position, kind) in seed::KINDS.iter().enumerate() {
            let graph = seed::graph_row(StepGraphId::new(), project_id, kind, now);
            let graph_id = graph.id;
            self.graphs.insert(graph_id, graph);
            for (phase_position, phase) in kind.phases.iter().enumerate() {
                self.phases.push(seed::phase_row(
                    PhaseId::new(),
                    graph_id,
                    phase_position as i32,
                    phase,
                    now,
                ));
            }
            let kind_row = seed::kind_row(
                ItemKindId::new(),
                project_id,
                graph_id,
                position as i32,
                kind,
                now,
            );
            self.kinds.insert(kind_row.id, kind_row);
        }
        for (name, _, body) in &DEFAULT_TEMPLATES {
            self.templates.push(seed::template_row(
                PromptTemplateId::new(),
                project_id,
                name,
                body,
                created_by,
                now,
            ));
        }

        Ok(row)
    }
```

Maps written, in order: `projects` (`:69`), `graphs` (`:73`), `phases` (`:75`), `kinds` (`:71`),
`templates` (`:77`). Not written: `item_key_counter` (`:100`), `app_settings`, anything else.
The trait arm at `:2854-2857` is unchanged (`let now = Utc::now(); self.write(|state|
state.create_project(new, now))`) — one clock read for thirty-six rows.

`traits.rs:379-385` doc, present tense:

```rust
    /// Inserts a project with `settings = {}` and no secret provider (M1 D9) and seeds it, in
    /// the same transaction, with `htui_core::seed`'s catalogue: five default graphs, their
    /// fifteen phases (ANA-2 §4.1 as amended by PRD D3/D5), five kinds and the ten
    /// `DEFAULT_TEMPLATES` at version 1 (M2 D4). `item_key_counter` is not seeded.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when `slug` is taken or
    /// `created_by` names no user; either way nothing is written.
```

**Three twins** in `mod tests` (`:3003`), placed after
`delete_project_leaves_no_row_in_any_map` (`:3820-3949`), same style (`MemStore::demo()`,
`store.read(|state| ..)`). Imports needed there beyond `:3004-3011`: `NewProject`, `ProjectId`,
`NewItem`, `ItemKindPatch`, `ItemId`, `ItemKindId`, `crate::prompt::{DEFAULT_TEMPLATES, body_of}`.

```rust
    /// A fresh project, authored by the fixture user, with the slug the test names.
    fn fresh_project(slug: &str) -> NewProject {
        NewProject {
            id: ProjectId::new(),
            slug: slug.to_owned(),
            name: slug.to_uppercase(),
            description: String::new(),
            created_by: ids::USER,
        }
    }

    /// The template *content* the seed writes, which no `WriteStore` reader can return
    /// (M1 D9: MOD-9 owns the editor). `project_create_seeds_the_catalogue` counts ten
    /// through `delete_reach`; this reads them through the inherent `prompt_templates`.
    #[tokio::test]
    async fn seeded_templates_carry_the_shipped_bodies() {
        let store = MemStore::demo();
        let project = store
            .create_project(fresh_project("seeded"))
            .await
            .expect("the create lands");

        let rows = store
            .prompt_templates(project.id)
            .await
            .expect("the inherent reader answers");
        let mut expected: Vec<&str> = DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect();
        expected.sort_unstable();
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            expected,
            "ten rows, one per default template, in the reader's name-byte order"
        );
        for row in &rows {
            assert_eq!(Some(row.body.as_str()), body_of(&row.name), "`{}` body", row.name);
            assert_eq!(row.version, 1, "`{}` is version 1", row.name);
            assert_eq!(row.created_by, ids::USER, "`{}` is the creator's", row.name);
            assert_eq!(row.project_id, project.id);
        }
    }

    /// D5: the seed writes no `item_key_counter` row; `mint` creates it on first use.
    #[tokio::test]
    async fn seed_never_writes_a_counter_row() {
        let store = MemStore::demo();
        let project = store
            .create_project(fresh_project("lazy"))
            .await
            .expect("the create lands");
        let counter = |prefix: &str| {
            store.read(|state| {
                state
                    .item_key_counter
                    .get(&(project.id, prefix.to_owned()))
                    .copied()
            })
        };
        assert!(
            store.read(|state| state.item_key_counter.keys().all(|(p, _)| *p != project.id)),
            "no counter row of any prefix after the create"
        );

        let feat = store
            .item_kinds(project.id)
            .await
            .expect("kinds read")
            .into_iter()
            .find(|kind| kind.prefix == "FEAT")
            .expect("the seeded FEAT kind");
        let minted = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: project.id,
                kind_id: feat.id,
                title: "first".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the first mint lands");
        assert_eq!(minted.key, "FEAT-1");
        assert_eq!(counter("FEAT"), Some(1), "the row exists only after the mint");
        assert_eq!(counter("ANA"), None, "and only for the prefix that minted");
    }

    /// PRD D12's third fact: the counter row of the **old** prefix survives a rename.
    /// `item_kind_round_trip_and_prefix_rules` pins the other two (old key text kept, `ANL-1`
    /// next) on both stores; no trait reader sees `item_key_counter`, so this one is per backend.
    #[tokio::test]
    async fn renamed_prefix_leaves_the_old_counter_row() {
        let store = MemStore::demo();
        let project = ids::PROJECT_HTUI;
        let counter = |prefix: &str| {
            store.read(|state| state.item_key_counter.get(&(project, prefix.to_owned())).copied())
        };
        assert_eq!(counter("ANA"), Some(2), "the fixture minted ANA-1 and ANA-2");

        let ana = store
            .item_kinds(project)
            .await
            .expect("kinds read")
            .into_iter()
            .find(|kind| kind.id == ids::KIND_HTUI_ANA)
            .expect("the fixture kind");
        let renamed = store
            .update_item_kind(
                ana.id,
                ana.updated_at,
                ItemKindPatch { prefix: Some("ANL".to_owned()), ..ItemKindPatch::default() },
            )
            .await
            .expect("the rename lands");
        assert!(matches!(renamed, CasOutcome::Applied(_)));
        assert_eq!(counter("ANA"), Some(2), "the old row is history, not garbage");
        assert_eq!(counter("ANL"), None, "nothing minted under the new prefix yet");

        let minted = store
            .mint_item(NewItem {
                id: ItemId::new(),
                project_id: project,
                kind_id: ids::KIND_HTUI_ANA,
                title: "after the rename".to_owned(),
                body: String::new(),
                required_tags: Vec::new(),
                touched_paths: Vec::new(),
                priority: 0,
                step_graph_id: None,
                created_by: ids::USER,
                box_id: Some(ids::BOX),
            })
            .await
            .expect("the mint lands");
        assert_eq!(minted.key, "ANL-1");
        assert_eq!(counter("ANL"), Some(1));
        assert_eq!(counter("ANA"), Some(2), "still");
    }
```

The rename twin is green the moment it is written (the rename never touched the counter); that is
D8's point — the fact was claimed at `conformance.rs:2450-2451` and asserted nowhere.

## 4. `PgStore` (`crates/htui-store/src/pg/write.rs`)

Imports (`:18-39`): add `PromptTemplateId` and `UserId` to the `htui_core::model` list;
`use htui_core::prompt::DEFAULT_TEMPLATES;` next to the existing `TemplateRole` import; `use
htui_core::seed;`. `Isolation`, `PhaseId`, `StepGraphId`, `ItemKindId`, `PgConnection`,
`map_sqlx` are already imported.

`create_project` (`:1043-1086`) — doc and call site:

```rust
    /// The project row and, in the same transaction, its thirty-five seed rows
    /// ([`seed_project`]): a project that fails to seed never existed (M1 D9, M2 D4).
    /// `settings` takes the column default `{}` and the two secret columns are MOD-10's.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when `slug` is taken (`23505`) or `created_by` names no
    /// `app_user` row (`23503`) — both on the first statement, before any seed row.
    async fn create_project(&self, new: NewProject) -> Result<Project> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let created = sqlx::query_as!( /* unchanged, :1046-1070 */ )
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        seed_project(&mut tx, created.id, created.created_by).await?;

        tx.commit().await.map_err(map_sqlx)?;
        Ok(created)
    }
```

`seed_project`, a private free fn at module level (after the `impl WriteStore for PgStore` block,
beside `kind_guard_refusal` at `:2182`). `Transaction<'_, Postgres>` derefs to `PgConnection`,
so `&mut tx` coerces:

```rust
/// The thirty-five rows a project is born with (plan D9/D10), on the caller's transaction.
///
/// Graphs first, then each graph's phases, then the kind (`item_kind.default_graph_id` is
/// `NOT NULL REFERENCES step_graph`, `0001_init.sql:288`), then the templates, which reference
/// only the project and its creator. Every non-timestamp column is bound — `gate_hard` and
/// `input_kinds` included where they equal the column default — so `seed::KINDS` is the whole
/// truth and a later migration's default cannot re-seed by omission. The timestamp columns are
/// the database's: `created_at` and `updated_at` on `step_graph` and `prompt_template`,
/// `updated_at` alone on `step_graph_phase` and `item_kind`, all `DEFAULT now()` (`0001_init.sql
/// :214-295`); the `now` the row constructors take is discarded here. No `RETURNING`: nothing
/// reads a seed row before the commit.
///
/// None of `create_step_graph`, `create_phase`, `create_item_kind` is reused: they run on the
/// pool and return `RETURNING` rows, and `create_item_kind`'s `EXISTS` guard reads through the
/// pool, where this transaction's uncommitted graph is invisible.
async fn seed_project(
    tx: &mut PgConnection,
    project_id: ProjectId,
    created_by: UserId,
) -> Result<()> {
    // For the row constructors only; every timestamp column takes the server's clock.
    let now = Utc::now();

    for (position, kind) in seed::KINDS.iter().enumerate() {
        let graph = seed::graph_row(StepGraphId::new(), project_id, kind, now);
        sqlx::query!(
            "INSERT INTO step_graph (id, project_id, name, description) \
             VALUES ($1, $2, $3, $4)",
            graph.id.as_uuid(),
            graph.project_id.as_uuid(),
            graph.name,
            graph.description,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        for (phase_position, phase) in kind.phases.iter().enumerate() {
            let row = seed::phase_row(PhaseId::new(), graph.id, phase_position as i32, phase, now);
            sqlx::query!(
                "INSERT INTO step_graph_phase (id, graph_id, position, name, fan_out, gate, \
                 gate_hard, retry_limit, input_kinds, output_kind, isolation, command_queue, \
                 verify_command, template_name, template_version, token_budget) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
                row.id.as_uuid(),
                row.graph_id.as_uuid(),
                row.position,
                row.name,
                row.fan_out,
                row.gate.as_str(),
                row.gate_hard,
                row.retry_limit,
                &row.input_kinds[..],
                row.output_kind,
                row.isolation.map(Isolation::as_str),
                row.command_queue.as_str(),
                row.verify_command.as_deref(),
                row.template_name,
                row.template_version,
                row.token_budget,
            )
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }

        let kind_row = seed::kind_row(
            ItemKindId::new(),
            project_id,
            graph.id,
            position as i32,
            kind,
            now,
        );
        sqlx::query!(
            "INSERT INTO item_kind (id, project_id, prefix, name, description, \
             default_graph_id, position) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            kind_row.id.as_uuid(),
            kind_row.project_id.as_uuid(),
            kind_row.prefix,
            kind_row.name,
            kind_row.description,
            kind_row.default_graph_id.as_uuid(),
            kind_row.position,
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    for (name, _, body) in &DEFAULT_TEMPLATES {
        let row = seed::template_row(PromptTemplateId::new(), project_id, name, body, created_by, now);
        sqlx::query!(
            "INSERT INTO prompt_template (id, project_id, name, version, body, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            row.id.as_uuid(),
            row.project_id.as_uuid(),
            row.name,
            row.version,
            row.body,
            row.created_by.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    Ok(())
}
```

Column and parameter inventory (bind everything except timestamps — D10 as amended by F1):

| Table | Bound | Left to `DEFAULT now()` | DDL |
|---|---|---|---|
| `step_graph` | `id, project_id, name, description` (4) | `created_at, updated_at` | `0001_init.sql:214-222` |
| `step_graph_phase` | `id, graph_id, position, name, fan_out, gate, gate_hard, retry_limit, input_kinds, output_kind, isolation, command_queue, verify_command, template_name, template_version, token_budget` (16, `create_phase`'s order) | `updated_at` | `:228-249` |
| `item_kind` | `id, project_id, prefix, name, description, default_graph_id, position` (7) | `updated_at` | `:282-293` |
| `prompt_template` | `id, project_id, name, version, body, created_by` (6) | `created_at, updated_at` | `:266-276` |

Binding types mirror `create_phase` (`:1647-1698`): `gate.as_str()`, `&input_kinds[..]` for
`TEXT[]`, `isolation.map(Isolation::as_str)` for the nullable `TEXT`, `command_queue.as_str()`,
`verify_command.as_deref()`. `AS "col: Type"` overrides are not needed: no `RETURNING`.

`.sqlx` regeneration, from the crate (README `:478-497`; `htui_sqlx` must carry the migrations
at `crates/htui-store/migrations`):

```bash
cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features
cargo sqlx prepare --check -- --all-targets --all-features
git status --porcelain .sqlx        # expect exactly four `??` lines (flag C)
```

The four files are committed in the same commit as `seed_project` (memory: `.sqlx` couples
tasks; an offline build must never sit between the query and its JSON).

## 5. Conformance (`crates/htui-core/src/store/conformance.rs`)

### 5.1 Wiring

`CASES` (`:30-66`): append `"project_create_seeds_the_catalogue"` after
`"settings_phase_rung_writes_token_budget_only"` — appended, never inserted, because MOD-6
reports per name and the list is run order.

`run_case` (`:77-130`): one arm before `other =>`:

```rust
        "project_create_seeds_the_catalogue" => project_create_seeds_the_catalogue(store).await,
```

`tests/mem_store.rs:35-41`: `35` → `36`; message gains `, and MOD-15 milestone 2's seed case
(plan D7)` (flag G). `pg_conformance.rs:19` is T2's.

Imports: `TemplateRole` is not in `:9-27`; add `use crate::prompt::TemplateRole;`. `Gate`,
`CommandQueue`, `DeleteTarget`, `UserId`, `StoreError` are already imported.

### 5.2 The case

Placed after `project_delete_takes_everything_and_says_so` (ends before `:2450`). T1 writes this
doc with the `mem.rs::` names only; T2 appends the `pg_criteria.rs::` names (§6.3).

```rust
/// D4's seed on a fresh project, read back through the trait: five graphs, fifteen phases, five
/// kinds and — by count alone, through `delete_reach` — ten templates. D3's table is repeated
/// here as a literal so the case checks the seed against the PRD and ANA-2 §4.1, not against
/// `seed::KINDS`, which is what it is meant to check; the `seed` module's own unit tests pin the
/// table's self-consistency (reserved names, bodies, density) with no store at all.
///
/// Template *content* and the counter's absence are per-backend facts no trait reader returns:
/// `mem.rs::seeded_templates_carry_the_shipped_bodies` and `mem.rs::seed_never_writes_a_counter_row`.
async fn project_create_seeds_the_catalogue<S: WriteStore>(store: &S) {
    const CASE: &str = "project_create_seeds_the_catalogue";
    /// `(graph and kind name, prefix, description, [(phase, input_kinds, gate_hard)])`, in
    /// `item_kind.position` order — PRD D3/D5 and ANA-2 `:307-319`, not the `seed` module.
    const TABLE: [(&str, &str, &str, &[(&str, &[&str], bool)]); 5] = [
        (
            "analysis",
            "ANA",
            "A question answered in writing",
            &[("research", &[], false), ("verdict", &["research"], true)],
        ),
        (
            "feature",
            "FEAT",
            "New behaviour",
            &[
                ("prd", &[], true),
                ("plan", &["prd"], true),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
        (
            "bug",
            "FIX",
            "Behaviour that is wrong",
            &[
                ("reproduce", &[], false),
                ("fix", &["reproduce", "review"], false),
                ("review", &["fix"], false),
            ],
        ),
        (
            "refactor",
            "CLEAN",
            "Behaviour kept, shape improved",
            &[
                ("plan", &[], false),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
        (
            "tooling",
            "TOOL",
            "The workshop rather than the product",
            &[
                ("plan", &[], false),
                ("implement", &["plan", "review"], false),
                ("review", &["implement"], false),
            ],
        ),
    ];

    let project = store.create_project(new_project("seeded")).await.expect(CASE);
    let p = project.id;

    // (a) graphs, by name bytes
    let graphs = store.step_graphs(p).await.expect(CASE);
    assert_eq!(
        graphs.iter().map(|graph| graph.name.as_str()).collect::<Vec<_>>(),
        ["analysis", "bug", "feature", "refactor", "tooling"],
        "{CASE}: five default graphs, ordered by name bytes (a)"
    );
    for graph in &graphs {
        assert_eq!(graph.project_id, p, "{CASE}: `{}` is the new project's (a)", graph.name);
        assert_eq!(
            graph.description,
            format!("Default graph for {} items", graph.name),
            "{CASE}: `{}` description (a)",
            graph.name
        );
    }

    // (b) kinds, by position; each default graph is the graph of the same name
    let kinds = store.item_kinds(p).await.expect(CASE);
    assert_eq!(
        kinds
            .iter()
            .map(|kind| (kind.prefix.as_str(), kind.name.as_str(), kind.position))
            .collect::<Vec<_>>(),
        TABLE
            .iter()
            .enumerate()
            .map(|(position, (name, prefix, ..))| (*prefix, *name, position as i32))
            .collect::<Vec<_>>(),
        "{CASE}: five kinds, positions dense from 0 (b)"
    );
    for (kind, (_, _, description, _)) in kinds.iter().zip(TABLE.iter()) {
        let graph = graphs
            .iter()
            .find(|graph| graph.id == kind.default_graph_id)
            .unwrap_or_else(|| panic!("{CASE}: `{}` has a default graph (b)", kind.prefix));
        assert_eq!(graph.name, kind.name, "{CASE}: `{}`'s graph is its namesake (b)", kind.prefix);
        assert_eq!(kind.description, *description, "{CASE}: `{}` description (b)", kind.prefix);
    }

    // (c) every graph's phases are D3's rows; (d) none is a reserved template name
    for (name, _, _, expected) in TABLE {
        let graph = graphs
            .iter()
            .find(|graph| graph.name == name)
            .unwrap_or_else(|| panic!("{CASE}: graph `{name}` (c)"));
        let phases = store.phases(graph.id).await.expect(CASE);
        let got: Vec<(&str, Vec<&str>, bool)> = phases
            .iter()
            .map(|phase| {
                (
                    phase.name.as_str(),
                    phase.input_kinds.iter().map(String::as_str).collect(),
                    phase.gate_hard,
                )
            })
            .collect();
        let want: Vec<(&str, Vec<&str>, bool)> = expected
            .iter()
            .map(|(phase, inputs, hard)| (*phase, inputs.to_vec(), *hard))
            .collect();
        assert_eq!(got, want, "{CASE}: `{name}` phases are D3's rows (c)");

        for (position, phase) in phases.iter().enumerate() {
            assert_eq!(phase.graph_id, graph.id, "{CASE}: `{name}`/`{}` (c)", phase.name);
            assert_eq!(
                phase.position, position as i32,
                "{CASE}: `{name}` positions are dense from 0 (c)"
            );
            if position == 0 {
                assert!(
                    phase.input_kinds.is_empty(),
                    "{CASE}: `{name}` position 0 reads nothing (c)"
                );
            }
            assert_eq!(
                (
                    phase.fan_out,
                    phase.gate,
                    phase.retry_limit,
                    phase.isolation,
                    phase.command_queue,
                    phase.verify_command.as_deref(),
                    phase.template_version,
                    phase.token_budget,
                ),
                (1, Gate::Always, 1, None, CommandQueue::FanOutOnly, None, None, None),
                "{CASE}: `{name}`/`{}` carries ANA-2's frozen defaults (c)",
                phase.name
            );
            assert_eq!(phase.output_kind, phase.name, "{CASE}: output_kind = name (c)");
            assert_eq!(phase.template_name, phase.name, "{CASE}: template_name = name (c)");
            assert_eq!(
                TemplateRole::of_name(&phase.name),
                TemplateRole::Phase,
                "{CASE}: `{}` is a phase name, not `judge`/`handoff` (d)",
                phase.name
            );
        }
    }

    // (e) the count of what was seeded, templates included
    let reach = store
        .delete_reach(DeleteTarget::Project(p))
        .await
        .expect(CASE)
        .unwrap_or_else(|| panic!("{CASE}: the project exists (e)"));
    assert_eq!(
        (reach.step_graphs, reach.phases, reach.item_kinds, reach.prompt_templates),
        (5, 15, 5, 10),
        "{CASE}: 5 graphs, 15 phases, 5 kinds, 10 templates (e)"
    );
    assert_eq!(
        (reach.items, reach.item_key_counters),
        (0, 0),
        "{CASE}: the seed mints no item and writes no counter row (e)"
    );

    // (f) the counter is lazy: the first mint under FEAT is FEAT-1
    let feat = kinds
        .iter()
        .find(|kind| kind.prefix == "FEAT")
        .unwrap_or_else(|| panic!("{CASE}: FEAT (f)"));
    let minted = store
        .mint_item(new_item(p, feat.id, "first"))
        .await
        .expect(CASE);
    assert_eq!(minted.key, "FEAT-1", "{CASE}: no counter row was seeded (f)");

    // (g) an unknown creator is refused before anything is written
    let mut orphan = new_project("orphan");
    orphan.created_by = UserId::new();
    let orphan_id = orphan.id;
    let refused = store.create_project(orphan).await;
    assert!(
        matches!(refused, Err(StoreError::Constraint(_))),
        "{CASE}: an unknown created_by is Constraint, got {refused:?} (g)"
    );
    assert!(
        store.project(orphan_id).await.expect(CASE).is_none(),
        "{CASE}: no project row survives the refusal (g)"
    );
    assert!(
        store.step_graphs(orphan_id).await.expect(CASE).is_empty(),
        "{CASE}: no graph survives the refusal (g)"
    );
    assert!(
        store.item_kinds(orphan_id).await.expect(CASE).is_empty(),
        "{CASE}: no kind survives the refusal (g)"
    );
}
```

Trait facts the case leans on: `step_graphs` orders by name bytes (`traits.rs:503-506`),
`item_kinds` by `position` then prefix (`:466-470`), `phases` by position (case 9 asserts it,
`:2818-2833`), `project(id) -> Option<Project>` (`:126`), `DeleteReach` fields (`:745-789`),
`new_item(project_id, kind_id, title)` (`:219-233`), `new_project(slug)` (`:1801-1809`).
`UserId::new()` is the id macro's UUIDv7 (`model/ids.rs:31-32`); it cannot collide with a fixture
user.

### 5.3 Doc edits to existing cases

- Case 4 `project_create_update_cas` (`:2144-2146`): "D9's create, unseeded, plus D3's
  compare-and-set." → "D9's create (its seed is `project_create_seeds_the_catalogue`'s to check)
  plus D3's compare-and-set." Assertions unchanged.
- Case 7 `item_kind_round_trip_and_prefix_rules` (`:2450-2451`): the doc's "the old counter
  survives" claim gains its backing — T1: "…which `mem.rs::renamed_prefix_leaves_the_old_counter_row`
  reads off `item_key_counter` directly." T2 extends to "…and
  `pg_criteria.rs::renamed_prefix_leaves_the_old_counter_row` off the table."

Every backticked name above is either a `fn` in `conformance.rs`/`mem.rs` (bare form) or a
`mem.rs::`/`pg_criteria.rs::` reference; `seed::KINDS` is skipped by the scanner (`::` without
`.rs`, and `KINDS` is not snake_case).

## 6. `pg_criteria.rs` twins (`crates/htui-store/tests/pg_criteria.rs`, T2)

Imports to add to `:15-33`: `ItemKindPatch`, `NewProject`, `ProjectId` is there, `htui_core::prompt::{DEFAULT_TEMPLATES, body_of}`, `htui_core::seed` is **not** needed. One local helper:

```rust
/// A fresh project authored by the fixture user, the one `app_user` the demo database keeps.
fn fresh_project(slug: &str) -> NewProject {
    NewProject {
        id: ProjectId::new(),
        slug: slug.to_owned(),
        name: slug.to_uppercase(),
        description: String::new(),
        created_by: ids::USER,
    }
}

/// Rows of `table` whose `project_id` is `project`, through a runtime query: a new `query!`
/// string would need a `cargo sqlx prepare` pass, and `.sqlx/` belongs to the crate.
async fn rows_of(pool: &PgPool, table: &str, project: ProjectId) -> i64 {
    sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM {table} WHERE project_id = $1"
    ))
    .bind(project.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap_or_else(|error| panic!("count {table}: {error}"))
}
```

(`table` is one of four literals from the test bodies, never input; the `format!` is the same
shape as `:2636-2650`'s runtime SQL.)

### 6.1 `seeded_templates_carry_the_shipped_bodies`

```rust
/// The seed's template rows, which no `WriteStore` reader returns: ten, named by
/// `DEFAULT_TEMPLATES`, body `body_of(name)`, version 1, `created_by` the creator's — read
/// through the inherent `PgStore::prompt_templates` and counted by SQL.
#[tokio::test(flavor = "multi_thread")]
async fn seeded_templates_carry_the_shipped_bodies() {
    let Some(db) = common::demo_db().await else { return; };
    let project = db.store.create_project(fresh_project("seeded")).await.expect("the create lands");

    assert_eq!(rows_of(&db.pool, "prompt_template", project.id).await, 10);
    assert_eq!(rows_of(&db.pool, "step_graph", project.id).await, 5);
    assert_eq!(rows_of(&db.pool, "item_kind", project.id).await, 5);

    let rows = db.store.prompt_templates(project.id).await.expect("the inherent reader answers");
    let mut expected: Vec<&str> = DEFAULT_TEMPLATES.iter().map(|(name, ..)| *name).collect();
    expected.sort_unstable();
    assert_eq!(rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(), expected);
    for row in &rows {
        assert_eq!(Some(row.body.as_str()), body_of(&row.name), "`{}` body", row.name);
        assert_eq!(row.version, 1);
        assert_eq!(row.created_by, ids::USER);
        assert_eq!(row.created_at, row.updated_at, "`{}` untouched since insert", row.name);
    }
    db.drop_db().await;
}
```

(`step_graph_phase` has no `project_id`; its 15 is `project_create_seeds_the_catalogue`'s (e).)

### 6.2 `seed_never_writes_a_counter_row`

```rust
/// D5 on the table itself: `item_key_counter` has no row for a project until its first mint.
#[tokio::test(flavor = "multi_thread")]
async fn seed_never_writes_a_counter_row() {
    let Some(db) = common::demo_db().await else { return; };
    let project = db.store.create_project(fresh_project("lazy")).await.expect("the create lands");

    assert_eq!(rows_of(&db.pool, "item_key_counter", project.id).await, 0);
    assert_eq!(counter(&db.pool, project.id, "FEAT").await, None);

    let feat = db.store.item_kinds(project.id).await.expect("kinds read")
        .into_iter().find(|kind| kind.prefix == "FEAT").expect("the seeded FEAT kind");
    let minted = db.store.mint_item(NewItem { project_id: project.id, kind_id: feat.id, ..race_item(feat.id, "first") })
        .await.expect("the first mint lands");
    assert_eq!(minted.key, "FEAT-1");
    assert_eq!(counter(&db.pool, project.id, "FEAT").await, Some(1));
    assert_eq!(rows_of(&db.pool, "item_key_counter", project.id).await, 1);
    db.drop_db().await;
}
```

(`race_item` (`:61-75`) hard-codes `project_id: ids::PROJECT_HTUI`; the struct-update overrides
it. `NewItem` derives `Clone`? If not, build the struct inline as §3's twin does.)

### 6.3 `renamed_prefix_leaves_the_old_counter_row`

Same calls as `mem.rs`'s twin (§3), through `db.store`, with `counter(&db.pool,
ids::PROJECT_HTUI, "ANA").await == Some(2)` before and after the rename, `"ANL"` `None` then
`Some(1)` after a `mint_item(race_item(ids::KIND_HTUI_ANA, "after the rename"))` whose key is
`"ANL-1"`. The rename is `update_item_kind(ana.id, ana.updated_at, ItemKindPatch { prefix:
Some("ANL".to_owned()), ..ItemKindPatch::default() })` (`conformance.rs:2526-2537`).

**Doc appends in `conformance.rs` (T2, doc only):** the seed case's second paragraph gains
"…; on Postgres, `pg_criteria.rs::seeded_templates_carry_the_shipped_bodies` and
`pg_criteria.rs::seed_never_writes_a_counter_row`."; case 7's doc gains the `pg_criteria.rs::`
sentence of §5.3.

## 7. Build order and commit sequence

One commit per step, each compiling (`cargo check --workspace --all-features`) and `cargo fmt`
clean. "red" below means a failing **test**, never a failing build, except where marked.

| Step | Task | Files | Work | First red test lives in | `cargo test -p htui-core --all-features` | `-p htui-store` (with DB) |
|---|---|---|---|---|---|---|
| 1 | T1 | `store/conformance.rs`, `store/mem.rs` (tests), `tests/mem_store.rs` | case fn (§5.2), `CASES` entry, `run_case` arm, case 4/7 docs naming `mem.rs::` twins, three `mem.rs` twins (§3), pin 36 | `conformance.rs` (the case, at (a)); `mem.rs` (templates and counter twins; the rename twin is green) | **red** — case + 2 twins; `every_cross_referenced_test_name_exists` green | `case_list_matches_mem_store` red (35 ≠ 36) until step 5 — expected |
| 2 | T1 | `seed.rs` (new), `lib.rs` | module (§1) with its unit tests | `seed.rs` tests written first in the same step; `cargo test -p htui-core` (no features) covers them | red — the case still fails | red |
| 3 | T1 | `fixtures.rs` | delete the three items, rewrite `catalogue()`, amend two tests, add `the_fixture_phases_are_the_seed_rows` (§2) | `fixtures.rs::the_fixture_phases_are_the_seed_rows` | red — case; fixtures green; `cargo test -p htui --all-features` snapshots unchanged | red |
| 4 | T1 | `store/mem.rs` (`State::create_project`, imports), `store/traits.rs` (doc) | §3 body; trait doc | — | **green**; T1 gate | red (count pin only) |
| 5 | T2 | `tests/pg_conformance.rs`, `tests/pg_criteria.rs`, `store/conformance.rs` (doc only) | `EXPECTED_CASES` 36; three twins + helpers (§6); `pg_criteria.rs::` doc appends | `pg_conformance.rs` (`project_create_seeds_the_catalogue` at (a)); `pg_criteria.rs` (§6.1, §6.2) | green | **red** — case + 2 twins; `every_cross_referenced_test_name_exists` green (file now has the fns) |
| 6 | T2 | `pg/write.rs`, `.sqlx/query-*.json` ×4 | `seed_project`, call site, doc (§4); `cargo sqlx prepare` | — | green | **green**; T2 gate |
| 7 | T3 | `HANDOFF.md`, `.claude/prds/mod-15-hierarchy-management.prd.md` | milestone-2 note, `:290` sentence, milestone row 2 | — | green | green; `cargo doc --workspace --no-deps` |

Commit messages (attribution trailer per the session's rule):

- T1
  1. `test(core): seed catalogue conformance case, MemStore twins, pin 36 (red)`
  2. `feat(core): htui_core::seed, the catalogue every project is born with`
  3. `refactor(core): demo fixture builds its catalogue through seed`
  4. `feat(core): MemStore::create_project seeds the catalogue`
- T2
  5. `test(store): PgStore seed twins and EXPECTED_CASES 36 (red)`
  6. `feat(store): PgStore::create_project seeds the catalogue in its transaction`
- T3
  7. `docs(mod-15): milestone 2 landed; gate_hard wording per PRD D3`

Gates, verbatim from the plan: after step 4 `cargo test -p htui-core --all-features`,
`cargo clippy -p htui-core --all-targets --all-features -- -D warnings`,
`cargo test -p htui --all-features`; after step 6
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features`,
the `cargo sqlx prepare` pair of §4, `cargo clippy -p htui-store --all-targets --all-features -- -D warnings`;
after step 7 `cargo doc --workspace --no-deps` and `git diff --stat` naming only the two docs.
Before step 6's DB work: `df -h /` (memory: `target/` fills the disk and Postgres crash-loops).

T3's HANDOFF edit (flag H): after the milestone-1 paragraph (`HANDOFF.md:317-345`) append one
paragraph opening `**Milestone 2 landed (<first>..<last>)**` with: 36 conformance cases both
stores, 5/15/5/10 per create, `htui_core::seed` as the single table, fixture on the amendments,
test count; rewrite the `:290` sentence to "`gate_hard` on `feature`'s `prd` and `plan` and
`analysis`'s `verdict`, none elsewhere (PRD D3)". The checkbox stays open (milestones 3–4
pending). PRD `:250` row 2 → `complete`, link `.claude/plans/mod-15-project-seed.plan.md`.

## 8. Hazards

1. **Scanner traps** (`conformance.rs:3783-3848`). (i) `seed.rs::anything` panics; (ii) a bare
   backticked snake_case token with ≥ 4 underscores must be a `fn` in `conformance.rs` or
   `mem.rs` — so `seed.rs` test names, `pg_criteria.rs` test names in bare form, and helper names
   like `rows_of` (fine, 1 underscore) obey it; (iii) `pg_criteria.rs::name` is checked **on
   disk**, so T1 must not write it (step 5 does). Fixture test
   `the_fixture_phases_are_the_seed_rows` (6 underscores) must not be backticked in
   `conformance.rs` either.
2. **Feature gates.** `seed` unconditional; `fixtures` behind `demo` (`lib.rs:16-17`);
   `conformance` behind `test-support` (which implies `demo`, `htui-core/Cargo.toml:12-13`);
   `pg_criteria.rs` behind `demo`. `seed.rs` tests must compile with no feature: no
   `crate::fixtures::ids` in them. `cargo test -p htui-core` (bare) and `--all-features` both
   run in T1's gate.
3. **Lints.** `missing_docs` on every pub item and field of `seed.rs`;
   `missing_debug_implementations` → derive `Debug`; `unused_qualifications` → after
   `use crate::seed;` write `seed::KINDS`, never `crate::seed::KINDS`; `-D warnings` on clippy
   `all` (pedantic off, `Cargo.toml:102-104`), so `as i32`/`as u8` casts are the fixture's
   existing idiom (`fixtures.rs:743-749`) and pass; `clippy::too_many_arguments` fires at 8 —
   `kind_row` and `template_row` have 6. No `#[expect(dead_code)]` anywhere: every constructor
   is used by `mem.rs` unconditionally, and an unfulfilled `expect` is itself a warning.
4. **Unused imports after the fixture rewrite.** `Gate`/`CommandQueue` (`fixtures.rs:19`) and
   possibly `PromptTemplateId` usage patterns change; let `cargo check -p htui-core --all-features`
   list them and remove exactly those.
5. **Reader order ≠ table order.** `prompt_templates` sorts by name bytes on both stores (flag L);
   `step_graphs` by name bytes (`analysis, bug, feature, refactor, tooling`, not `KINDS` order);
   `item_kinds` by position (`KINDS` order). The case's (a) and (b) lists differ for that reason.
6. **`MemStore` ordering of `State.phases`.** `phase_rows(graph)` (`mem.rs:2955`) is what
   `phases()` returns; the seed pushes in position order, as the fixture does, so no re-sort is
   needed — but the case asserts position density explicitly, so a change there fails loudly.
7. **Partial move in `create_project`.** `new.slug`/`name`/`description` move into `row`;
   take `let created_by = new.created_by;` first (it is `Copy`, but the intent reads better and
   the templates loop needs it after `row` is built).
8. **Transaction reborrow.** `seed_project(&mut tx, ..)` relies on `Transaction: DerefMut<Target
   = PgConnection>`; inside, `.execute(&mut *tx)` reborrows per statement (same as `demo.rs:187+`).
9. **`.sqlx`.** Any edit to a `query!` string after `prepare` re-reddens `SQLX_OFFLINE` builds;
   run the `--check` last, commit the JSON with the query. `prepare` needs `htui_sqlx` migrated
   (README `:486-493`); `pg_criteria.rs` must add no `query!` of its own (flag C).
10. **Timestamps.** `now` on `MemStore` rows is the wrapper's one `Utc::now()`; on Postgres the
    columns are the server's. The case never compares timestamps across stores; the Pg twin
    asserts `created_at == updated_at` on a template because both defaulted in one statement.
11. **Count pins across crates.** Step 1 reddens `pg_conformance.rs::case_list_matches_mem_store`
    (35 ≠ 36) without a database; that test has no `demo` gate. It is green again at step 5. Do
    not "fix" it in T1.
12. **Rename twin is green on arrival** (both stores). That is expected — D8's third fact was
    never asserted; the test is a pin. Do not invent a red for it.
13. **(g) on Postgres.** `UserId::new()` fails the `project.created_by` FK (`23503` →
    `Constraint`) on the first statement; the transaction is dropped uncommitted. On `MemStore`,
    `require_user` (`mem.rs:1381-1389`) fires before any insert. If `UserId` turns out to lack
    `new()` (the id macro should provide it, `ids.rs:31-32`), `UserId::from_uuid(Uuid::now_v7())`.
14. **Fixture test `ten_templates_per_project_from_the_default_bodies`** compares names in
    `DEFAULT_TEMPLATES` order and stays correct because `catalogue()` pushes in that order; do not
    "align" it with the store twins' sorted comparison.
15. **`cargo test -p htui --all-features`** after step 3: insta snapshots must not change (plan
    D2, V3). A changed snapshot means a screen reads `input_kinds`/`gate_hard` the plan says
    nothing reads — stop and report, do not `--accept`.
16. **`item_key_counter` on `PgStore`** — `mint_item`'s `ON CONFLICT`/upsert path creates the
    row (V13); the seeder must not `INSERT INTO item_key_counter`. `rows_of(.., "item_key_counter", ..)`
    in §6.2 is the assertion.
