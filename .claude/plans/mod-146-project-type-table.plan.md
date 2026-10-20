# Plan: MOD-146 — Project type table

**Source**: `HANDOFF.md` MOD-146 (from ANA-35, `docs/ANA-35.md` §7.4; DDL sketch "P3" in §7.9).
"`project_type` with its six built-in rows (software, research, docs, ops, general, workspace; no
recipes), `project.type` (existing projects `software`), and the `coding` flag ANA-39 and MOD-128
read; types listed read-only in Settings. Small. Not blocked."

**Requirements**: `R-ID-1`, `R-ENT-3`, `R-TUI-8` ("Settings tab: … project types, item kinds, and step
graphs …", `docs/REQUIREMENTS.md:757-758`).

**Routing**: routed as **plan** by `/handoff-run MOD-146` (C1–C4 none fired; ultracode not needed).
Sandbox run `hr/MOD-146`. Staffing: every agent inherits the session model (Opus 5.5); no `model:`
override.

**Complexity**: Small. One Postgres migration, one mirror migration, one model type and one new field
on `Project`, one `WriteStore` read across five implementors, one Settings section.

**Status**: **confirmed** 2026-10-20 at the CONFIRM gate with the defaults of OQ-1–OQ-4;
fact-checked the same day (see "Verified claims").

**Out of scope (owned elsewhere)**: per-type seeds, templates, `needs_repo`, default isolation and the
type picker (MOD-148, ANA-35 P5); scope rows and `chk_project_scope_row` (MOD-147, P4); user-defined
types with a JSONB recipe and the graph editor (ANA-33 / `R-TUI-8` editor). No write API for types
ships here. MOD-146 changes nothing that seeds a project: `seed_project` and `State::create_project`
still seed today's software kinds and graphs for every project.

---

## Open questions (each has a default this plan adopts)

- [ ] **OQ-1 — The six rows' `name` and `description`.** Default (D3):

  | slug | name | description | coding | selectable |
  |---|---|---|---|---|
  | `docs` | Documents | Documents outlined, drafted and reviewed. | no | yes |
  | `general` | General | Tasks carried out and reviewed. | no | yes |
  | `ops` | Operations | Operations planned, executed and verified; incidents investigated. | no | yes |
  | `research` | Research | Questions answered and research concluded with a verdict. | no | yes |
  | `software` | Software | Code in repositories: analyses, features, fixes. | **yes** | yes |
  | `workspace` | Workspace | A workspace's own items; not offered for new projects. | no | **no** |

- [ ] **OQ-2 — List order.** Default (D5): `ORDER BY slug COLLATE "C"`, as `personas()` orders by
  name. That is the table above, alphabetical. **Alternative:** a `position` column so `software`
  comes first (ANA-35 §7.9's DDL has none; MOD-148's picker can put the default first by itself).
- [ ] **OQ-3 — `NewProject` and the type at creation.** Default (D4): `NewProject` is unchanged; every
  new project gets the column default `software` until MOD-148 adds the picker and the field.
  **Alternative:** add `NewProject.project_type` now. This moves every `NewProject` literal
  (conformance, mem tests, pg tests, the hierarchy worker) for a value nothing can choose yet.
- [ ] **OQ-4 — Where the section sits.** Default (D8): appended after Queue as **Project types**,
  following the "appending moves no existing section's line" convention (`app/mod.rs:73-88`).
  **Alternative:** beside Kinds. That reads better but moves every later section's strip position
  and its positional tests.

## Decisions

- **D1 — Migration `0020_project_type.sql`.** It contains ANA-35 §7.9's P3 DDL verbatim: the
  `project_type` table, an `INSERT` of the six built-in rows (`builtin = true`, `seed` NULL,
  `created_by` NULL), and `ALTER TABLE project ADD COLUMN type TEXT NOT NULL DEFAULT 'software'
  REFERENCES project_type(slug)`. Existing rows take the default. Column comments follow the house
  style that `tests/migrations.rs` pins (`MOD125_COLUMN_COMMENTS`).
- **D2 — Model.** A new `ProjectType` in `htui-core` (`model/hierarchy.rs`, beside `Project`) carries
  every column: `slug`, `name`, `description`, `coding`, `builtin`, `selectable`,
  `seed: Option<Value>`, `seed_version: i32`, `created_by: Option<UserId>`, `created_at`,
  `updated_at`. `Project` gains `pub project_type: String`, the slug. The SQL column stays `type`, so
  `query_as!` reads it as `type AS project_type`. `type` is a keyword; `r#type` appears nowhere in the
  tree, and `kind` already means item kind.
- **D3 — One source of truth for the built-ins.** `htui-core::seed` gains `PROJECT_TYPES` (six
  `BuiltinProjectType { slug, name, description, coding, selectable }`) and `DEFAULT_PROJECT_TYPE =
  "software"`. MemStore answers from it. A Postgres test pins the migrated rows equal to the constant,
  in the spirit of `the_defaults_are_migration_0002s_ten_rows_verbatim`.
- **D4** — See OQ-3. The creation paths set `software`: MemStore's `State::create_project` from
  `DEFAULT_PROJECT_TYPE`, Postgres by the column default, and `RETURNING` reads it back.
- **D5 — The read.** `WriteStore::project_types(&self) -> Result<Vec<ProjectType>>` goes beside
  `personas()` (`traits.rs:1068`). It is the same kind of global, online-only registry read. It sits
  on `WriteStore` because the cache is not a `WriteStore`, so offline Settings says "unavailable", as
  Personas does. It has five implementors: MemStore, PgStore, `Writer`, `UsageSpy` and the recorder
  `SpyStore`. ANA-39 and MOD-128 read `coding` from this list; no `is_coding` helper ships here
  (YAGNI).
- **D6 — The mirror gets the column, not the table.** `cache_migrations/0007_project_type.sql` adds
  `project.type TEXT NOT NULL DEFAULT 'software'`, because `CacheStore::project` must build a full
  `Project`. `project_type` itself joins no `MIRRORED_TABLES`, as `persona` and `app_setting` don't.
- **D7 — Conformance.** Two cases in `CASES`, so both stores pass the same law:
  `project_types_are_the_six_builtins` (slugs, order, `coding` only on software, `selectable` false
  only on workspace, all `builtin`) and `a_new_project_is_software`. Postgres-only tests in
  `tests/migrations.rs`: `TABLES` gains `project_type`; a project row that existed before 0020 reads
  back `software`; an unknown `type` is refused by the FK; a bad slug is refused by the CHECK.
- **D8 — Settings section `ProjectTypesSection`** (`ui/tabs/settings/project_types.rs`). It is
  read-only and app-wide (its `wants_requests` ignores scope, like Personas). It shows one line per
  type (name, slug, a `coding` mark, "not selectable" on workspace) and the cursor row's description
  beneath. j/k move the cursor through the shared list acts. It has no write and no `Mode`. The
  request and reply plumbing is a new `StoreRequest::ProjectTypes` and
  `StoreReply::ProjectTypes(Vec<ProjectType>)`, served through `backend.writer()`. Offline and
  `Failed` give "unavailable", as Queue and Personas do. Keys: a new `Context::SettingsProjectTypes`,
  table `settings.project_types`, heading "Project types", and a browse view stack. Every key goes
  through keys.toml; nothing is hard-coded.

## Files to change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/model/hierarchy.rs` (+ `model/mod.rs` re-export) | UPDATE: `ProjectType`, `Project.project_type` | T1 |
| `crates/htui-core/src/seed.rs` | UPDATE: `PROJECT_TYPES`, `DEFAULT_PROJECT_TYPE` | T1 |
| `crates/htui-core/src/store/traits.rs` | UPDATE: `WriteStore::project_types` | T1 |
| `crates/htui-core/src/store/mem.rs` | UPDATE: impl, `create_project` literal (`:3031`) | T1 |
| `crates/htui-core/src/store/conformance.rs` | UPDATE: two cases in `CASES` + `run_case` arms | T1 |
| `crates/htui-core/src/fixtures.rs` (`:733`) | UPDATE: literal | T1 |
| `crates/htui-core/src/secret.rs` (`:1380`, test) | UPDATE: literal | T1 |
| `crates/htui-orch/src/secrets.rs` (`:214`, test) | UPDATE: literal | T1 |
| `crates/htui-agent/src/conformance.rs` (`UsageSpy`, `:751`) | UPDATE: delegate | T1 |
| `crates/htui-agent/tests/recorder.rs` (`SpyStore`, `:439`) | UPDATE: delegate | T1 |
| `crates/htui-store/migrations/0020_project_type.sql` | CREATE | T2 |
| `crates/htui-store/src/pg/read.rs` (`:628`), `pg/write.rs` (`:2423`, `:2479`) | UPDATE: `query_as!(Project…)` ×3, `project_types` | T2 |
| `crates/htui-store/src/writer.rs` | UPDATE: delegate | T2 |
| `crates/htui-store/cache_migrations/0007_project_type.sql` | CREATE | T2 |
| `crates/htui-store/src/cache/refresh.rs` (`:684`, `:697-739`) | UPDATE: `PROJECT_COLUMNS`, SELECT, bind | T2 |
| `crates/htui-store/src/cache/read.rs` (`:907`) | UPDATE: `CacheStore::project` | T2 |
| `crates/htui-store/src/testkit.rs` (`seed_mirror`) | UPDATE if it lists project columns | T2 |
| `crates/htui-store/tests/migrations.rs` (`TABLES` `:30`) + `tests/cache.rs` | UPDATE | T2 |
| `crates/htui-store/.sqlx/` | REGENERATE (changed + new queries) | T2 |
| `crates/htui/src/store_worker.rs` | UPDATE: request/reply/name/route | T3 |
| `crates/htui/src/ui/tabs/settings/project_types.rs` + `settings/mod.rs` | CREATE / UPDATE | T3 |
| `crates/htui/src/app/mod.rs` (`register_all`) | UPDATE: append section | T3 |
| `crates/htui/src/keys/catalogue.rs`, `keys/views.rs` | UPDATE: context, table, heading, stack | T3 |
| `crates/htui/tests/project_types.rs` + affected `.snap` | CREATE / UPDATE | T3 |
| `HANDOFF.md`, `DECISIONS.md` (if a decision is recorded), this plan | UPDATE | T4 |

## Tasks (serial: T1 → T2 → T3 → T4)

No two tasks are independent. T2 and T3 both compile against T1's model. T2 owns `.sqlx/`, which
`refresh_project` and the `query_as!` sites share. Everything runs in one tree in this sandbox, TDD
first in every task.

### T1 — Model, built-ins, `WriteStore::project_types`, MemStore, conformance
- **Tests first**: the two conformance cases (D7), plus a unit test that `PROJECT_TYPES` has six
  unique slugs matching ANA-35's CHECK regex, exactly one `coding` type and exactly one
  non-selectable type.
- **Then**: D2, D3, D4, D5 for MemStore and the two spies; the 5 literal sites.
- **Mirror**: `personas()` (traits `:1068`, mem `:8733`, `UsageSpy` `:1043`, `SpyStore` `:770`).
- **Validate**: `cargo test -p htui-core --all-features`;
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

### T2 — Postgres migration, PgStore, Writer, mirror
- **Tests first**: the `tests/migrations.rs` cases of D7; the Postgres conformance run picks up T1's
  cases; `tests/cache.rs` `projects_agree` carries `project_type`.
- **Then**: D1, the three `query_as!` sites and `refresh_project` (D6), `PgStore::project_types`
  (`ORDER BY slug COLLATE "C"`), `Writer` delegation, the mirror migration, `CacheStore::project`,
  `seed_mirror`. Run `cargo sqlx prepare` against a migrated scratch DB on `localhost:5439`
  (`docs/hr-sandbox.md`), not the empty compose `htui` database.
- **Mirror**: the 0019 migration's comment style; `PgStore::personas` / `persona_rows`.
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1`;
  `cargo sqlx prepare --check --workspace`.

### T3 — Settings › Project types
- **Tests first** (`tests/project_types.rs`, after `tests/personas.rs`): one line per type over the
  demo; j/k move and stop at the ends; offline says unavailable and offers no keys; the product
  registers it after Queue; `wants_requests` ignores scope; the request name is stable
  (`name_arms_are_stable`).
- **Then**: D8.
- **Mirror**: `QueueSection` (`queue.rs:226-240`, `616-618`, `677-691`) for plumbing;
  `PersonasSection` (`personas.rs:200-218`, `list_lines` `1212-1242`) for rows.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; a full `cargo insta test`
  run. Grepping `.snap` files undercounts: every full-frame Settings strip snapshot gains
  " Project types ", and the help box gains a context.

### T4 — Close-out
- Phase note or P2 close-out in `HANDOFF.md`, and an ANA-35 §7.4 / MOD-148 / MOD-147 cross-reference
  if needed. While there, fix the duplicated line in ANA-39's entry (`HANDOFF.md:101-102`, "of
  `R-PRM-1` (ANA-3 shipped none). Not blocked." appears twice; a merge artifact).
- Run the validator.

## Validation (gate on the real tree)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                # featureless gate
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1
cargo sqlx prepare --check --workspace
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Migration-number collision at collect: another in-flight run (MOD-144, MOD-145, MOD-125's successors) also takes `0020`/`0007` | Medium | Renumber on the host at merge; `gate.sh` refuses duplicates and gaps, so a collision is caught, never silent |
| Snapshot churn larger than expected (strip, help box) | Medium | A full `cargo insta test` run; review every `.snap` diff, accept only strip/help-box lines |
| `.sqlx` drift: `query_as!(Project…)` hash changes at 3 sites + `refresh_project` | High (expected) | Prepare against a migrated scratch DB; `--check` in the gate |
| An existing test compares a whole `Project` against hand-built JSON (MCP / `--json` CLI) | Low | Full workspace run; the field is additive |
| Keys catalogue positional tests (`context_position`, `context_tables_and_headings`) shift | Certain | Append the context after `SettingsQueue`, update the pinned lists |

## Verified claims (fact-check, 2026-10-20, tree `a801f477`)

| Claim | Verdict | Evidence |
|---|---|---|
| Next Postgres migration is `0020`; next mirror migration is `0007` | true | `migrations/` ends at `0019_stop_runs.sql`; `cache_migrations/` ends at `0006_run_workspace.sql` |
| `Project` has no type field; derives `Serialize, Deserialize` | true | `model/hierarchy.rs:56-78` |
| No persisted data deserializes a `Project` (an additive field is safe) | true | no `from_str/from_slice/from_value::<…Project>` in `crates/`; holders are in-memory snapshots (`catalogue.rs:58`, `hierarchy.rs:53`, `prompt_settings.rs:67`, `queue_settings.rs:54`) |
| `Project` literals that need the field: 5 | true | `fixtures.rs:733`, `secret.rs:1380`, `mem.rs:3031`, `orch/secrets.rs:214`, `cache/read.rs:907`. `htui/src/secrets.rs:353` and `engine.rs:20344` spread `..project` (no change) |
| `query_as!(Project…)` sites: 3 | true | `pg/read.rs:628`, `pg/write.rs:2423`, `pg/write.rs:2479` |
| The mirror's project refresh lists columns explicitly | true | `cache/refresh.rs:684-693` (`PROJECT_COLUMNS`), `:697-739` (`query!` SELECT + binds) |
| `persona` and `app_setting` are not mirrored (D6 precedent) | true | `cache/mod.rs:44-66`, 21 tables |
| `personas()` is a `WriteStore` read; `WriteStore` has exactly 5 implementors | true | `traits.rs:1068`; impls `mem.rs:8382`, `pg/write.rs:1013`, `writer.rs:331`, `htui-agent/src/conformance.rs:751`, `htui-agent/tests/recorder.rs:439`; `backend.rs:6` states there is no `impl WriteStore for Backend` |
| `tests/migrations.rs` pins the table list | true | `const TABLES` `:30`, ordered by migration, last entry `workspace_stop` (0019) |
| Settings sections are appended last by convention; the "last" tests check adjacency, not end position | true | `app/mod.rs:73-88`; `tests/personas.rs:1410-1433` asserts `" Boxes  Personas "` |
| Each section has a keys `Context`, table and heading; positions are pinned | true | `keys/catalogue.rs:96-100`, `:243-245`, `:299-301`, `context_position` `:2351-2353`, `context_tables_and_headings` `:3195-3197` |
| `.sqlx` offline data lives in `crates/htui-store/.sqlx/` | true | `query-*.json` files there |
| ANA-35 §7.9's P3 DDL is the source for D1 | true | `docs/ANA-35.md:1092-1110` |
| Task independence | n/a | All tasks run serially (T2/T3 depend on T1's model; T2 alone owns `.sqlx/`), so no independence claim is made |

## Acceptance
- [ ] `project_type` with six built-in rows; `project.type` NOT NULL, FK, existing rows `software`
- [ ] `ProjectType` + `Project.project_type`; `WriteStore::project_types` on every implementor
- [ ] Conformance green on Mem and Pg; mirror carries `type`
- [ ] Settings › Project types lists the six read-only, unavailable offline, keys via keys.toml
- [ ] Full gate green; validator green
