# Plan: MOD-15 milestone 5 — the prompt is tunable from the app

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 5 only (`:253`). Design
authority: the PRD's Scope (`:196-198`), Constraints (`:225-240`), D7 (the typed registry), D8
(compare-and-set on `updated_at`) — cited as **PRD Dn**; milestone 1's plan
(`.claude/plans/mod-15-hierarchy-seam.plan.md`, **M1 Dn**), milestone 3's plan
(`.claude/plans/mod-15-hierarchy-section.plan.md`, **M3 Dn**), milestone 4's plan
(`.claude/plans/mod-15-kinds-graphs.plan.md`, **M4 Dn**), `docs/ANA-5.md` §5.3/§5.4/§9,
`docs/ANA-2.md` §4.4, `HANDOFF.md:284-462`. This plan's own decisions are plain **Dn**.
**Requirements**: `R-TUI-8` (the configuration surface), `R-ENT-10` (no last-writer-wins),
`R-PRM-*` by consequence (the ten keys the assembler reads), `R-NF-3` by ownership.
**Bare filenames below**: `store_worker.rs`, `catalogue.rs`, `hierarchy.rs`, `prompt_settings.rs`,
`testkit.rs`, `ui/**`, `app/**` are `crates/htui/src/…`; `traits.rs`, `mem.rs`,
`prompt/settings.rs`, `model/hierarchy.rs` are `crates/htui-core/src/…`.
**Complexity**: Medium (one worker module, three `StoreRequest` and two `StoreReply` variants, one
section with two modes; **no seam change, no migration, no new `query!`**).
**Routing**: routed as **PRD** by `/handoff-run MOD-15`; milestone 5 resumes at `plan`
(`/handoff-run MOD-15`, 2026-09-17, maintainer accepted the `plan` verdict). Reviewer:
`rust-reviewer` (`.claude/workflow-config.json`). Ultracode: not recommended (T1 → T2 → T3 are
serial; see Tasks).

## Summary

Milestone 1 put the whole settings seam in place: a typed `SettingKey`/`SPECS` registry in
`htui_core::prompt::settings`, and `set_setting` / `clear_setting` / `setting` over
`SettingRung = App | Project(ProjectId) | Phase(PhaseId)`, validating on write with the reader's own
rules. Milestone 4 built the app's first settings writer, deliberately scoped to **one key on one
rung**: `token_budget` on `Phase`, through `StoreRequest::SetPhaseBudget` (M4 D6, which names this
milestone as the one that widens it).

Today the other nine keys, and `token_budget` on the two rungs above a phase, are editable only by
SQL against the database. `prompt::settings::{resolve_budget, resolve_hops, resolve_max_skill_tokens,
resolve_excerpt_caps, resolve_reserve_bp}` read them through the chain
phase → `project.settings` → `app_setting` → the compiled `DEFAULTS` table, and
`trim_record.budget_source` already records which rung answered — but nothing in the app writes any
of it, and nothing shows which rung a number came from.

This milestone adds two things. (1) **`htui::prompt_settings`** (`crates/htui/src/prompt_settings.rs`),
the worker half: one `SettingsSnapshot` per read — the ten keys on the `App` rung with their
compare-and-set tokens, plus every project of the scope with the two keys the `Project` rung accepts
— and three served requests (`PromptSettings`, `SetSetting`, `ClearSetting`). (2) **`PromptSection`**
(`SectionId("prompt")`, `ui/tabs/settings/prompt.rs`), registered **after** `kinds`: one row per
`(rung, key)` pair that the registry says exists, each showing the stored value, the effective value
and which rung answered, edited through a one-field editor where an empty field **clears** rather
than writes a guessed constant.

Everything is driven off `SPECS` (PRD `:196` — "driven off the registry rather than a hard-coded
list"): the rows, the labels, the units, the accepted rungs, the doc line and the range in the
refusal all come from `SettingKey::ALL` and `SettingKey::spec()`, so an eleventh key added by MOD-4
or MOD-12 appears in this section without touching it.

No `WriteStore` method is added: conformance `CASES` stays **36**, `EXPECTED_CASES` stays **36**,
`.sqlx/` and `migrations/` are untouched, and `0003_orchestration.sql` is still MOD-4's and still
next.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **One worker module, `crates/htui/src/prompt_settings.rs`** (`htui::prompt_settings`), mirroring `hierarchy.rs` and `catalogue.rs` one concept across: `SettingsSnapshot`, `snapshot()`, `serve()`, `REQUEST_NAMES`, and the same `reread`/`cas` helpers. Not added to `catalogue.rs`. | M4 D1's reasoning, one milestone on: `catalogue.rs` owns a different tree (kinds, graphs, phases) and its snapshot is what its `cas` helper re-reads. A settings write must re-read the settings snapshot, not the catalogue. Two files, one responsibility each. |
| D2 | **Every stored value comes from `WriteStore::setting(rung, key)`** — never from parsing `project.settings` in this crate. The snapshot's project entries still carry the whole `Project` row, because the raw blob is what the reader's own resolvers take (D5) and `project.updated_at` is the `Project` rung's compare-and-set token. | M1's live coordinate 2: `project.settings` holds `upstream_hops` under a **different name** than the `app_setting` key `prompt_upstream_hops`, and `SettingSpec::project_key` carries the mapping. `setting()` applies it; a reader in this crate that indexed the blob by `spec.key` would silently show nothing for one of the two project keys. |
| D3 | **The read is scope-wide, one request: `StoreRequest::PromptSettings(Scope)`.** The snapshot is `SettingsSnapshot { app: Vec<AppEntry>, projects: Vec<ProjectEntry> }`, `app` in `SettingKey::ALL` order (ten entries, always), `projects` in `scope.project_ids` order. A project id that names no row is **skipped**. | M4 D2, same words and the same reason: the staleness index is keyed by `(Origin, Discriminant<StoreRequest>)` (`app/state.rs:158`), so N requests of one variant would leave only the newest reply delivered. |
| D4 | **Snapshot types**: `AppEntry { key: SettingKey, value: Option<Value>, updated_at: Option<DateTime<Utc>> }` — `updated_at` is `None` exactly when no `app_setting` row exists, which is the state a set must pass `expected: None` for; `ProjectEntry { project: Project, values: Vec<ProjectValue> }` with `ProjectValue { key: SettingKey, value: Option<Value> }` for the keys whose `spec.rungs` contains `Rungs::PROJECT`. The `Project` rung's token is `project.updated_at`, held once per project rather than per key. | `set_setting`'s `expected: Option<DateTime<Utc>>` means "I expect no row" **only on `App`** (`traits.rs:553-560`), and on `Project` the compare token is the project row's, not a per-key one, because the write is a key-level merge into one JSONB column. The types say exactly that, so no call site has to remember it. |
| D5 | **Provenance is the reader's, not a second implementation.** The section labels a row with `htui_core::prompt::settings::BudgetSource` — `Phase`, `Project`, `AppSetting`, `AppSettingDefault` — and for the two `Project`-rung keys it asks the reader itself: `resolve_budget(None, project_blob, &app_map).source` for `token_budget`, and for `upstream_hops` the same two-rung rule the snapshot can state exactly (`Project` when the project holds a usable value, else `AppSetting` when the row holds one, else `AppSettingDefault`). The eight `App`-only keys are `AppSetting` when the row is present and `AppSettingDefault` when it is not. | `trim_record.budget_source` already records this and PRD `:197` asks the editor to show it, so a second enum would be a second spelling of one fact. `BudgetSource::as_str` is the one spelling (`prompt/settings.rs:567-577`). A test pins the derived label against `resolve_budget`'s own `source` for all four rung combinations, so the two cannot drift. |
| D6 | **The effective value shown beside the stored one is computed by the reader's functions**, over the snapshot's own `app_map()` (`BTreeMap<String, Value>` of the entries that have a value, exactly what `Backend::app_settings()` returns) and the project blob: `resolve_budget`, `resolve_hops`, `resolve_max_skill_tokens` and `resolve_excerpt_caps`. Nothing in the section re-implements a clamp or a fallback. **Amended at blueprint (flag A)**: `resolve_reserve_bp` is **private** (`prompt/settings.rs:665`), so `prompt_reserve_fraction`'s effective value is `resolve_budget(None, None, &app_map).reserve_bp` / `Budget::reserve()` — still the reader's own arithmetic, reached through the one public door. | The risk the PRD names (`:371`) is a value stored that the read half ignores. Showing the number the reader would produce, next to the number the row holds, makes a divergence visible in the one place a user would look for it. |
| D7 | **Three requests** (`REQUEST_NAMES`, in variant order): `prompt_settings`, `set_setting`, `clear_setting`. `SetSetting { scope, rung: SettingRung, key: SettingKey, value: Value, expected: Option<DateTime<Utc>> }`, `ClearSetting { scope, rung: SettingRung, key: SettingKey, expected: DateTime<Utc> }`. Both answer `StoreReply::PromptSettings` (applied) or `StoreReply::PromptSettingsStale` (compare-and-set miss). | M1's seam is two methods over a rung enum rather than one per key (PRD D7), so the request layer is shaped the same. `expected` keeps the seam's asymmetry — `Option` on set, plain on clear — rather than inventing a uniform one the store would have to undo. |
| D8 | **`SetPhaseBudget` stays.** M4's request is not folded into `SetSetting`, and this section never sends a `Phase` rung. | The two differ in the *reply*, not the request: `SetPhaseBudget` answers a re-read `CatalogueSnapshot` because the kinds section renders a tree of phases, and `SetSetting` answers a re-read `SettingsSnapshot`. A folded request would have to guess which tree its caller renders. Recorded as open item **O-2**: if a third caller ever wants the phase rung from here, the reply — not the request — is what has to generalise. |
| D9 | **Rows are `(rung, key)` pairs the registry admits, in a two-level tree**: an `App` group with the ten keys in `SettingKey::ALL` order, then one group per project of the scope with the keys whose `spec.rungs` contains `Rungs::PROJECT` — today `prompt_upstream_hops` and `token_budget`, tomorrow whatever `SPECS` says. No key is listed on a rung its spec refuses. | PRD `:196`: driven off the registry. `rung_refusal` (`prompt/settings.rs:441-443`) is the seam's own gate; a section that offered the row anyway would be building an editor whose only outcome is a refusal. |
| D10 | **One-field editor, opened with `e` on a value row.** The field starts at the stored value as text (empty when the rung holds none). `Enter` on a non-empty field sends `SetSetting`; `Enter` on an **empty** field sends `ClearSetting` when the rung holds a value, and answers `nothing is set on this rung` when it does not. `Esc` cancels. | M4 D16's rule, generalised: "empty means inherit" is already how the phase budget field behaves, and clear-vs-set is PRD D7's third consequence ("clear lets the compiled default answer, rather than the editor guessing at a constant"). One habit for both sections. |
| D11 | **The section parses only *shape*, never range.** `SettingKind::Integer` → `i64` (`\`{key}\` is a whole number, or empty to clear`), `SettingKind::Fraction` → a finite `f64` (`\`{key}\` is a decimal fraction, or empty to clear`). Every bound — `min`, `max`, the `Phase` narrowing, `not_above` — is the store's, and its sentence is shown verbatim. | M4's `BUDGET_IS_A_NUMBER` precedent and M1 D7: validation refuses with one sentence, coined in `htui-core` so `MemStore` and `PgStore` say the same thing. A range checked twice is a range that can disagree with itself. |
| D12 | **A fraction renders as what it is and what it means**: `0.1 (1000 bp)`. It is sent as `Value::from(f64)` and never as basis points. | `SPECS`'s `min`/`max` for `prompt_reserve_fraction` are **basis points of the stored float** (`prompt/settings.rs:287-290`), and `validate` rounds `value × 10 000` before comparing (`:511-526`). Showing only the float hides the unit the refusal is written in; showing only bp would invite typing `1000`, which rounds to 10 000 000 bp and is refused. |
| D13 | **The `not_above` asymmetry is shown, not implied.** When a row's stored value is above what the reader will clamp it to — today only `excerpt_head_lines` against `excerpt_file_line_cap` — the row carries `clamped to {peer} = {n}`. | Blueprint flag J, accepted at M1 and recorded in `validate`'s doc comment (`prompt/settings.rs:452-464`) with "so the settings editor can show the consequence instead of implying the pair is symmetric". This is that editor; the line is derived from `spec.not_above` and the resolver's answer, so no key is hard-coded. |
| D14 | **Compare-and-set, miss handling and the in-flight guard are milestone 3's and 4's, unchanged**: one write in flight by `StoreRequest::name`, a miss answers `PromptSettingsStale` and the editor keeps its typed text, takes the reloaded token and retries only on a second `Enter` (PRD D8, M3 D7, M4 D8); a row that is gone from the reload closes the editor. The `CHANGED_ELSEWHERE` / `CHANGED_ELSEWHERE_CLOSED` / `DELETED_ELSEWHERE` sentences and `is_error` come from `settings/mod.rs`. **Amended at blueprint (flag D)**: M4 promoted only `some_text`, `yes_or_no` and `is_error` (`settings/mod.rs:37-59`); the three sentences are still private `const`s of `kinds.rs:63-77`, so **this milestone promotes kinds' three** and rewrites its call sites. `hierarchy.rs` keeps its own pair — its `DELETED_ELSEWHERE` differs by text and M3's snapshots pin it. | `R-ENT-10`, and one habit across three sections. M4's H-9 residue ("a miss with no editor open says nothing was written") is inherited with the text rather than re-derived. |
| D15 | **Registered after `kinds`** in `app/mod.rs` (`Agents`, `Hierarchy`, `Kinds`, `Prompt`), title `Prompt`. The PRD's open question 2 stays open. | M4 D17: registration order is strip order; appending is the only change that moves no existing strip line except by adding to it. Four titles are **34** columns of the pinned 100-column harness (`tests/settings.rs:942-956`; corrected at blueprint, flag E), so the pin still holds — it is re-run with the fourth section, not relaxed. |
| D16 | **Tests**: `crates/htui/tests/prompt_settings.rs`, worker half through `store_worker::serve` over `Backend::memory(MemStore::demo())`, section half through `SectionBench` and `Harness`, with `prompt_settings__*.snap` snapshots. One Postgres-gated test asserts a **migrated** database presents all ten `App` rows with tokens, because `MemStore` starts with an empty `app_settings` map and a migrated Postgres does not (F-7). | M3 D14 / M4 D18, one file across. The two-store difference is the whole reason `expected: None` exists on the set path, so it is tested on the store that actually produces it. |

## Patterns to Mirror

| Concern | Pattern | Where |
|---|---|---|
| Worker module: snapshot types, `snapshot()`, `serve()`, `reread`, `cas`, `REQUEST_NAMES` | `htui::catalogue` | `crates/htui/src/catalogue.rs:25-108`, `:124-294`, `:296-331` |
| One `try_serve` arm of or-ed patterns, **no guard** (a guarded arm is `E0004` — M3 F-12) | the nine catalogue patterns | `store_worker.rs:893-902` |
| Section skeleton: `Row`, `Field`, `Editor`, `Mode`, `busy`, `Notice`, hint line | `KindsSection` | `ui/tabs/settings/kinds.rs:109-380`, `:1941` |
| Editor keys, focus, submit-guard while a write is in flight | `on_editor_key` / `submit` | `settings/kinds.rs` (`Mode::Editing` arm), `settings/hierarchy.rs:772-832` |
| CAS miss that keeps the text and re-takes the token | `on_stale` + `reload` | `settings/hierarchy.rs:986-1010`, `:1318-1349` |
| `Failed` routed by request name | `REQUEST_NAMES.contains(request)` | `settings/hierarchy.rs:1195-1215` |
| `Debug` that prints labels and never a buffer | `Editor` / `Mode` hand-written impls | `settings/kinds.rs:232-252`, `:287-316` |
| Section tests: bench, keys, replies, snapshots | `tests/kinds.rs` | `crates/htui/tests/kinds.rs:1-120` |
| Multi-phase HANDOFF paragraph | "Milestone N landed (…)" appended to the one checklist line | `HANDOFF.md:324`, `:352`, `:376`, `:415`; `.claude/rules/workflow-docs.md` lifecycle 4 |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/prompt_settings.rs` | **new** | T1 | D1/D2/D3/D4/D7: snapshot types, `snapshot()`, `serve()`, `REQUEST_NAMES`, `app_map()` |
| `crates/htui/src/lib.rs` | edit | T1 | `pub mod prompt_settings;` beside `pub mod preview;` (`:19-20`) |
| `crates/htui/src/store_worker.rs` | edit | T1 | D7: three request variants + `name()` arms; two reply variants; one `try_serve` arm of three or-ed patterns delegating to `prompt_settings::serve` |
| `crates/htui/tests/prompt_settings.rs` | **new** | T1 (worker half), T2 (section half) | D16 |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | **new** | T2 | D9/D10/D11/D12/D13: the section |
| `crates/htui/src/ui/tabs/settings/mod.rs` | edit | T2 | `pub mod prompt;`, `pub use prompt::PromptSection;`; promote M4's three CAS sentences if they are still private to `kinds.rs` |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | edit | T2 | only if a sentence is promoted by the line above; call sites become `super::` |
| `crates/htui/src/app/mod.rs` | edit | T2 | D15: register after `kinds` (`:50`) |
| `crates/htui/tests/settings.rs` | edit | T2 | D15: the strip-width pin now covers four sections (`:942-956`) |
| `crates/htui/tests/snapshots/prompt_settings__*.snap` | **new** | T2 | D16 |
| `HANDOFF.md` | edit | T3 | MOD-15 entry: "Milestone 5 landed" paragraph |
| `.claude/prds/mod-15-hierarchy-management.prd.md` | edit | T3 | milestone table `:253`: row 5 → complete, plan link |

Not changed: `crates/htui-core/src/store/**` (no seam method; `CASES` stays 36),
`crates/htui-core/src/prompt/settings.rs` (read, not edited — the registry is already what this
milestone consumes), `crates/htui-store/**` (no `query!`, no `.sqlx`, no migration),
`crates/htui-agent/**`, `ui/text_field.rs`, `chat/composer.rs`, `app/action.rs`
(`TabAction::FocusSection` is milestone 6's).

## Tasks

**T1 → T2 → T3, serial.** T2 compiles against T1's request, reply and snapshot types, and the file
sets T1 = {`prompt_settings.rs`, `lib.rs`, `store_worker.rs`, `tests/prompt_settings.rs`} and
T2 = {`settings/prompt.rs`, `settings/mod.rs`, `settings/kinds.rs`, `app/mod.rs`,
`tests/settings.rs`, `tests/prompt_settings.rs`, snapshots} **intersect on
`tests/prompt_settings.rs`**. No independent pair exists, which is why the routing verdict says
ultracode is not needed.

Each implementer commits its own work incrementally — uncommitted subagent work does not survive the
session. Before blaming a Postgres failure in any gate, check `df -h /`: `target/` fills the disk on
this box.

TDD per task: tests first, red, then code. Every implementer prompt carries: PRD D7/D8 and M1 D3/D7,
M3 D5/D7, M4 D14/D16 win over this plan where they disagree; graphify-first for codebase questions;
**no `WriteStore` change**; nothing sets `updated_at` by hand; no new migration, no new `query!`; a
section holds no store handle and no `UserId`/`BoxId`; `Debug` never prints a field's text;
`unsafe_code = "forbid"`, MSRV 1.98.

### Task 1: the settings worker

- **Files**: `crates/htui/src/prompt_settings.rs` (new), `crates/htui/src/lib.rs`,
  `crates/htui/src/store_worker.rs`, `crates/htui/tests/prompt_settings.rs` (new, worker half only).
- **Action**:
  - D3/D4 types and `snapshot(store, scope) -> Result<SettingsSnapshot>`: for each
    `key` of `SettingKey::ALL`, `setting(SettingRung::App, key)?` → `AppEntry`; for each project id
    of the scope, `project(id)?` (skip `None`) and, for each key whose `spec().rungs` contains
    `Rungs::PROJECT`, `setting(SettingRung::Project(id), key)?` → `ProjectValue`. Bound is
    `ReadStore + WriteStore`, as `catalogue::snapshot`'s is (`setting` lives on `WriteStore`).
    N+1 by design, per event and never per keystroke (M3 D5's trade, same words).
  - `SettingsSnapshot::app_map(&self) -> BTreeMap<String, Value>`: the entries that carry a value,
    keyed by `spec().key` — the same map `Backend::app_settings()` hands the resolvers, so D6's
    effective values are computed over the reader's own input shape.
  - D7 three `StoreRequest` variants with their `name()` arms:
    `PromptSettings(Scope)`;
    `SetSetting { scope, rung: SettingRung, key: SettingKey, value: Value,
    expected: Option<DateTime<Utc>> }`;
    `ClearSetting { scope, rung: SettingRung, key: SettingKey, expected: DateTime<Utc> }`.
    Two `StoreReply` variants: `PromptSettings(Box<SettingsSnapshot>)`,
    `PromptSettingsStale(Box<SettingsSnapshot>)`.
  - One `try_serve` arm of **three or-ed patterns**, no guard (M3 F-12: a guarded arm in a
    wildcard-free `match` is `E0004`), delegating to `prompt_settings::serve`.
  - `serve` mirrors `catalogue::serve`: `Backend::writer()` or `StoreError::Unreachable`, the read
    arm answers `reread`, the two write arms answer through `cas`, and the last arm refuses a
    request that is not one of the three by name. Carry `catalogue.rs:116-119`'s recorded residue
    (a re-read that fails after an applied write answers `Failed`) in the module doc rather than
    silently repeating it.
- **Tests** (`tests/prompt_settings.rs`, worker half), over `Backend::memory(MemStore::demo())`
  unless noted:
  - the snapshot carries ten `AppEntry` in `SettingKey::ALL` order, every one `value: None` and
    `updated_at: None` on a fresh `MemStore`, and one `ProjectEntry` per scope project carrying
    exactly the two `Project`-rung keys;
  - a scope project id that names no row is skipped rather than failing the read;
  - `SetSetting` on `App` with `expected: None` applies and the reply carries the value and a token;
    a second `SetSetting` re-using the **old** `expected` answers `PromptSettingsStale`;
  - `ClearSetting` on `App` with the current token applies and the entry returns to
    `value: None, updated_at: None`;
  - `SetSetting` on `Project` for `SettingKey::UpstreamHops` stores under `upstream_hops` and is
    read back by `setting(Project(id), UpstreamHops)` (M1 live coordinate 2), while a foreign key
    already in `project.settings` survives the merge (PRD `:370`);
  - an out-of-range value answers `Failed` carrying the seam's own sentence
    (``constraint violated: `token_budget` = 0 is outside 1..=…``-shaped, asserted on the key and
    the range, not on the whole string);
  - `SettingKey::MaxSkillTokens` on `Project` answers `Failed` with `rung_refusal`'s sentence;
  - `Backend::Offline` refuses all three with `DATABASE_UNREACHABLE`;
  - `REQUEST_NAMES` and `StoreRequest::name()` agree (the `kinds.rs` test's
    `catalogue_names_are_stable` one concept across);
  - **Postgres-gated** (`HTUI_TEST_DATABASE_URL`, skipped otherwise as the existing PgStore tests
    are): a migrated database answers ten `AppEntry` that all carry a value **and** a token, since
    `0002_agent_probe.sql:68-79` inserts all ten rows.
- **Gate**: `cargo test -p htui --all-features`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo doc -p htui --no-deps`. `CASES`/`EXPECTED_CASES` still 36;
  `git status` shows no `.sqlx/` or `migrations/` change.

### Task 2: the prompt section

- **Files**: `crates/htui/src/ui/tabs/settings/prompt.rs` (new),
  `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/ui/tabs/settings/kinds.rs` (only for a
  promoted sentence), `crates/htui/src/app/mod.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/prompt_settings.rs` (section half),
  `crates/htui/tests/snapshots/prompt_settings__*.snap`.
- **Action**: D9's two-level row tree; D10's one-field editor with the clear-on-empty rule; D11's
  shape-only parsing; D12's fraction rendering; D13's clamp line; D5/D6's provenance and effective
  value; D14's CAS, in-flight guard and inherited sentences; D15's registration and the strip pin.
  Keys, Browse mode: `j`/`k` move, `e` edit the row under the cursor, `r` reload, `Esc` clears a
  notice. `captures_input` is `!matches!(self.mode, Mode::Browse)` — derived from the mode, never a
  flag. A row renders
  `key   stored | effective (source)   unit`, and the pane under the cursor prints `spec.doc`, the
  range as `min..=max unit`, the rungs the key accepts (`Rungs`'s `Display`), and D13's clamp line
  when it applies.
- **Tests** (section half): the tree lists ten `App` rows and two rows per project; a key is never
  listed on a rung its spec refuses; `e` then a number then `Enter` emits `SetSetting` with the
  entry's own `expected` (`None` when no row exists); `e` then clearing the field then `Enter`
  emits `ClearSetting`, and on a rung that holds nothing emits **no request** and says so; a
  `PromptSettingsStale` reply keeps the typed text, re-takes the token and only a second `Enter`
  retries; a `Failed` for one of the three names shows the seam's sentence verbatim; a second write
  while one is in flight is refused; the provenance label matches `resolve_budget(...).source` for
  all four combinations of phase-absent × project-present × app-present (D5); a stored
  `excerpt_head_lines` above `excerpt_file_line_cap` renders the clamp line (D13); the fraction row
  renders `0.1 (1000 bp)` and its editor round-trips `0.1` (D12); `captures_input` is true only in
  `Editing`, so `l` reaches the field rather than the strip; snapshots `prompt_settings__*.snap`.
- **Gate**: `cargo test -p htui --all-features` with the new `prompt_settings__*.snap` accepted
  **after reading each one**, and no snapshot outside `prompt_settings__*` changed except the strip
  line; `cargo clippy -p htui --all-targets --all-features -- -D warnings`.
  `cargo run -p htui -- --demo` smoke where a TTY exists: `Settings` → `l` `l` `l` → edit
  `token_budget` on `App`, watch a project row's source flip from `app_setting` to `project` after
  editing it there, clear it and watch it flip back, type `0` and read the refusal. **No TTY in the
  agent environment → say so rather than claiming it ran** (M3 and M4's close-out precedent).

### Task 3: docs

- **Files**: `HANDOFF.md`, `.claude/prds/mod-15-hierarchy-management.prd.md`.
- **Action**: PRD milestone row 5 → complete with this plan linked; HANDOFF's MOD-15 entry gains a
  "Milestone 5 landed (`<first>`..`<last>`, date)" paragraph carrying the test count, the three
  request names, D8's reason for keeping `SetPhaseBudget`, and the live coordinates milestone 6
  needs. Milestone 6 named as the only one remaining.
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
(`crates/htui-store/tests/pg_conformance.rs`), and — corrected at blueprint (flag F) — **every
existing `.snap` without exception**: the snapshots that show the strip build their own section
vector rather than `register_all`'s (`tests/kinds.rs:763-775`), so registering a fourth section moves
none of them. `.sqlx/` unchanged; a diff there means a query was added against this plan.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The section shows a provenance that disagrees with what the assembler actually used | Medium | High — the editor becomes a lie about the prompt | D5 reuses `BudgetSource` and asks `resolve_budget` itself; a test pins the label against the resolver for all four rung combinations |
| A `Project`-rung write erases MOD-4's or MOD-12's keys in `project.settings` | Low (the seam merges) | High — invisible for months | PRD D7's key-level merge is milestone 1's and is already conformance-tested; T1 re-asserts it from the request layer with a foreign key in the blob |
| `prompt_upstream_hops` is written into a project under the App spelling | Medium if the blob were parsed here | High — a key `resolve_hops` never looks at | D2: every value goes through `setting()`, which applies `SettingSpec::project_key`; a test reads the stored key back by name |
| A set after a clear passes a token for a row that no longer exists | Medium | Medium — a write that silently never applies | D4 makes `updated_at: None` the type-level state; the clear test asserts the entry returns to `None` and the following set carries `expected: None` |
| The section re-implements a range or a clamp and drifts from the reader | Medium | Medium | D11: shape only; every bound is the store's sentence, shown verbatim |
| A fraction is typed as basis points (`1000`) and silently refused as absurd | Medium | Low | D12 renders both (`0.1 (1000 bp)`), and the refusal names the rounding (`rounds to 10000000 bp, outside 0..=5000 bp`) |
| `MemStore` and a migrated Postgres disagree about whether the ten `App` rows exist | Certain (F-7) | Medium — a test suite that passes on memory and fails in the app | D16's Postgres-gated test asserts the migrated shape; both `expected: None` and `expected: Some` paths are covered |
| Four sections overflow the strip at the pinned 100×30 harness width | Low (36 columns of 100) | Low | D15 re-runs `the_section_strip_fits_the_frame` with the fourth section rather than relaxing it; MOD-30 owns the general fix |
| An `App` row cleared elsewhere makes the editor's token dead: the seam answers `NotFound`, **not** `Stale`, so the reload-and-retry path never runs (found at blueprint, flag H / H-1) | Low | Medium — the retry habit fails on one path | The refusal is the seam's own sentence naming `app_setting`, shown verbatim; recovery is `Esc`, `r`, `e`; a worker test pins the behaviour, and open item O-3 names the seam change (a `Stale` able to carry "no row") that would close it |
| A read's reply is taken for a write's (M4's recorded hazard) | Low | Low | The in-flight guard of D14, plus M4's two open doors (a scope change, a tab re-activation) argued in the same place — `on_prompt_settings` |
| `--demo` smoke cannot run without a TTY | Certain in the agent environment | Low | Interactive behaviour is pinned by tests and snapshots; the close-out says the smoke did not run |

## Verified claims (fact-check, 2026-09-17)

Every row read out of this tree during `/handoff-run` step 3.5, before the CONFIRM gate.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F-1 | `WriteStore` carries `set_setting(rung, key, value, expected: Option<DateTime<Utc>>)`, `clear_setting(rung, key, expected: DateTime<Utc>)` and `setting(rung, key) -> Option<StoredSetting>`, and `setting` is on the **write** half | **true** | `traits.rs:553-560`, `:570-576`, `:584`; `StoredSetting { value: Option<Value>, updated_at }` at `:830-835` |
| F-2 | `expected: None` means "I expect no row" and is accepted on `App` only; `Project`/`Phase` refuse it with a constraint | **true** | `traits.rs:542-552` (doc) and `:670` |
| F-3 | `setting()` answers `None` only when the rung's row is absent; a present project without the key answers `Some` with `value: None` | **true** | `traits.rs:578-583` |
| F-4 | The registry is ten keys, `SettingKey::ALL` is `[Self; 10]`, `spec()` indexes `SPECS`, and `SettingKey` is `Copy + Eq + Hash` | **true** | `prompt/settings.rs:145-151`, `:174-192` |
| F-5 | Exactly two keys accept the `Project` rung (`prompt_upstream_hops` → `upstream_hops`, `token_budget`), one of which also accepts `Phase`; the other eight are `App`-only | **true** | `SPECS`, `prompt/settings.rs:320-435` — `rungs: Rungs::APP` on eight, `Rungs::APP.or(Rungs::PROJECT)` on `prompt_upstream_hops`, `.or(Rungs::PHASE)` on `token_budget` |
| F-6 | `SettingSpec::project_key` is `Some` exactly for the two `Project`-rung keys and differs from `key` for one of them | **true** | `prompt/settings.rs:295-298`, `:409-411` (`project_key: Some("upstream_hops")` under `key: "prompt_upstream_hops"`), `:420-422` |
| F-7 | A migrated Postgres holds all ten `app_setting` rows, while `MemStore` starts with an empty map and the demo fixture seeds none — so both `expected` paths are reachable | **true** | `0002_agent_probe.sql:68-79` (`INSERT … ON CONFLICT (key) DO NOTHING`, ten rows); `mem.rs:95`, `:183` (`app_settings: BTreeMap::new()`); no `app_setting` write in `fixtures.rs` |
| F-8 | `BudgetSource` has exactly the four variants D5 reuses, with `as_str` as their one spelling | **true** | `prompt/settings.rs:548-577` (`Phase`, `Project`, `AppSetting`, `AppSettingDefault`) |
| F-9 | The resolvers D6 calls take the shapes the snapshot can hand them | **partly FALSE — plan amended at blueprint** | `prompt/settings.rs:637-641` `resolve_budget(phase: Option<i32>, project: Option<&Value>, app: &BTreeMap<String, Value>) -> Budget` ✓; `:715` `resolve_max_skill_tokens(app) -> i64` ✓; `:732` `resolve_excerpt_caps(app) -> (ExcerptCaps, u32, Duration)` ✓. **`resolve_hops` takes a third argument** — `resolve_hops(project, app, notes: &mut Vec<String>) -> u8` (`:690-694`, flag B): the section passes a scratch `Vec` and prints a non-empty `notes` in the detail pane. `resolve_reserve_bp` is private (flag A, see D6) |
| F-10 | `DEFAULTS.value_of(key) -> Value` is public, so the compiled default can be rendered; `Defaults::integer` is **private** and cannot be | **true** | `prompt/settings.rs:101` (`pub fn value_of`), `:118-121` (`fn integer`, no `pub`) — D6 therefore renders the default through `value_of` and the resolvers, never through `integer` |
| F-11 | `validate` refuses rather than clamps, in the order rung → kind → range → `not_above`, and its `String` is the whole sentence the store wraps unchanged | **true** | `prompt/settings.rs:445-540`; `rung_refusal` at `:441-443` |
| F-12 | The `not_above` rule is one-directional by decision, and its doc names the settings editor as the place to show the consequence — D13's mandate | **true** | `prompt/settings.rs:452-464` |
| F-13 | `try_serve`'s `match request` still has **no wildcard and no guarded arm**, so the three new patterns must be or-ed into one arm | **true** | `store_worker.rs:830-902`: the twelve hierarchy patterns at `:877-890` and the nine catalogue patterns at `:893-902` are each one or-ed arm; no `_ =>` / `_ if` in the match |
| F-14 | Reply matches outside the worker all carry `_ => {}`, so two new `StoreReply` variants compile untouched | **true** | `app/update.rs:223`, `settings/agents.rs:1267`, `settings/hierarchy.rs:1216`, `settings/kinds.rs:1504`, `chat/mod.rs:447`, `:524` |
| F-15 | The strip pin is a real test over the product's registrations and lists **three** sections today; `app/mod.rs` registers them at `:47-51` | **true** | `tests/settings.rs:942-956`; `app/mod.rs:47-51` (`AgentsSection`, `HierarchySection`, `KindsSection`) |
| F-16 | `SectionId("prompt")`, `SettingsSnapshot` and `prompt_settings` have **zero** occurrences in `crates/` — nothing is being renamed | **true** | `grep -rn 'SectionId("prompt")\|SettingsSnapshot\|prompt_settings' crates --include="*.rs"` → no matches |
| F-17 | `crates/htui/src/lib.rs` is where `pub mod prompt_settings;` goes, beside `pub mod catalogue;` and `pub mod preview;` | **true** | `lib.rs:12-25` |
| F-18 | `htui::testkit::SectionBench` exists with `ctx()` and `key()` and is the shared section bench M3 left for milestones 4–6 | **true** | `testkit.rs:453-512` |
| F-19 | `Scope` is `{ workspace_id, project_ids }` and derives `Clone`, so D3's per-write scope compiles and is cheap | **true** | `model/scope.rs:9-15` |
| F-20 | `Project` carries `settings: Value` and `updated_at`, so D4's project entry needs no second read for the blob or the token | **true** | `model/hierarchy.rs:56-77` |
| F-21 | `M4`'s `SetPhaseBudget` answers a `CatalogueSnapshot`, which is what makes D8's "not folded" argument a fact rather than a preference | **true** | `catalogue.rs:257-285` (the arm ends in `cas(&writer, scope, &outcome)`, whose fresh value is a `CatalogueSnapshot`, `:309-315`) |
| F-22 | **Task independence**: T1 and T2 are **not** independent | **true (no parallel marking)** | T1 ∩ T2 = {`crates/htui/tests/prompt_settings.rs`}, and T2's section compiles against T1's `StoreRequest`/`StoreReply`/`SettingsSnapshot`. The plan runs them serially; nothing here is fanned out |
| F-23 | Workspace lints and MSRV are as the constraints say | **true** | root `Cargo.toml:7` (`rust-version = "1.98"`), `:91` (`unsafe_code = "forbid"`) |
| F-24 | `SettingRung` derives `Debug, Clone, Copy, PartialEq, Eq`, so D7's requests compile inside `StoreRequest` (`#[derive(Debug, Clone)]`) and carry no secret — every field is a rung, a typed key or a JSON number | **true** | `traits.rs:799-800`; `store_worker.rs:69` and the enum's "no secret as a plain `String`" rule at `:62-66`; `CatalogueSnapshot` already proves `Project` is `Debug + Clone` inside a reply (`catalogue.rs:35-39`) |

## Acceptance

- [ ] `htui::prompt_settings` assembles one `SettingsSnapshot` per scope: ten `App` entries in
      `SettingKey::ALL` order with their tokens, plus every scope project with the keys the
      `Project` rung accepts, skipping project ids that name no row
- [ ] Three `StoreRequest` and two `StoreReply` variants, served through one wildcard-free
      `try_serve` arm of or-ed patterns; `REQUEST_NAMES` and `name()` agree in a test
- [ ] `PromptSection` registers after `kinds` as `SectionId("prompt")`; the strip pin passes with
      four sections
- [ ] Every row, label, unit, range and doc line is read from `SPECS`; no key name is spelled in the
      section. The **effective** column is the one exhaustive `match key` (blueprint flag G): an
      eleventh key is a compile error naming the missing arm, never a silent blank
- [ ] A value is editable exactly on the rungs its spec accepts; an empty field clears rather than
      writing a constant, and clearing a rung that holds nothing sends no request and says so
- [ ] Each row shows the stored value, the effective value the reader would produce, and which rung
      answered, labelled with `BudgetSource`'s own spelling; a test pins the label against
      `resolve_budget`
- [ ] `excerpt_head_lines` above `excerpt_file_line_cap` renders the clamp the reader will apply
- [ ] A compare-and-set miss answers `PromptSettingsStale`, keeps the typed text and retries only on
      `Enter`; one write in flight at a time
- [ ] A refused value shows the seam's own sentence verbatim, on both stores
- [ ] `CASES` 36, `EXPECTED_CASES` 36, `.sqlx/` and `migrations/` untouched, no `WriteStore` change
