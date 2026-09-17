# Blueprint: MOD-15 milestone 5 — the prompt is tunable from the app

**From**: `.claude/plans/mod-15-prompt-settings.plan.md` (D1–D16, F-1..F-24). **Tree**: `1975dae`, `main`, 2026-09-17. **Read notes**: graphify was consulted first; every coordinate below was then read from the tree, as M4's blueprint did. Bare filenames are `crates/htui/src/…` unless prefixed; `settings.rs` is `crates/htui-core/src/prompt/settings.rs`, `traits.rs`/`mem.rs` are `crates/htui-core/src/store/…`.

## 0. Choices and flags

### 0.1 Plan corrections (found in the tree)

| Flag | Plan says | Tree says | Resolution |
|---|---|---|---|
| A | D6 names `resolve_reserve_bp` as a function the section calls | `fn resolve_reserve_bp` is **private** (`settings.rs:665`) | The reserve row's effective value is `resolve_budget(None, None, &app_map).reserve_bp` (`Budget.reserve_bp: u32`, `Budget::reserve() -> f64`, `settings.rs:584-618`). No other path exists without editing `htui-core`, which the plan forbids. |
| B | F-9: `resolve_hops(project, app) -> u8` | `resolve_hops(project: Option<&Value>, app: &BTreeMap<String, Value>, notes: &mut Vec<String>) -> u8` (`settings.rs:690-694`) | Section passes a scratch `Vec<String>`; a non-empty `notes` is printed in the detail pane (it only fires for a hand-written row outside `1..=2`, since `validate` refuses those). |
| C | Summary implies the resolvers are importable from `htui_core::prompt` | `prompt/mod.rs:42-45` re-exports only `Budget, BudgetSource, DEFAULTS, Defaults, Rungs, SPECS, SettingKey, SettingKind, SettingSpec, rung_refusal, validate` | Import `htui_core::prompt::settings::{resolve_budget, resolve_excerpt_caps, resolve_hops, resolve_max_skill_tokens}` (`pub mod settings`). |
| D | D14: the three CAS sentences "come from `settings/mod.rs`, promoted there by M4 D14 where they are not already shared" | `settings/mod.rs` shares only `some_text`, `yes_or_no`, `is_error` (`:37-59`). `CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`, `DELETED_ELSEWHERE` are still `const` private to `kinds.rs:65-77`. `hierarchy.rs:60-64` has its **own** `CHANGED_ELSEWHERE` (same text) and a different `DELETED_ELSEWHERE` ("deleted elsewhere while you were editing") | T2 promotes **kinds'** three to `pub(crate) const` in `settings/mod.rs`; `kinds.rs` call sites become `super::…`. `hierarchy.rs` is not touched (its `DELETED_ELSEWHERE` differs by text and M3's snapshots pin it). |
| E | D15: four titles are 36 columns | `Agents`(8) + `Hierarchy`(11) + `Kinds`(7) + `Prompt`(8) = **34** (`render_strip` pads each title by two, `mod.rs:306`) | Pin holds either way; the test is re-run, not relaxed. |
| F | Validation: "every … `.snap` except where the section strip gained `Prompt`" | The only snapshots showing the strip with `Kinds` are `kinds__demo/no_workspace/offline.snap`, and those tests build their **own** three-section tab (`tests/kinds.rs:763-775`, `:887-891`), not `register_all`'s | **No existing snapshot changes at all.** The T2 gate tightens to "no snapshot outside `prompt_settings__*` changed". |
| G | Acceptance: an eleventh key "appears in this section without touching it" | The effective column needs the reader's per-key resolver, and the four resolvers return four shapes (`Budget`, `u8`, `i64`, `(ExcerptCaps, u32, Duration)`) | Rows, labels, units, range, doc, rungs and the editor are registry-driven. The effective value is one **exhaustive** `match key` (§4.5): an eleventh key is a compile error naming the missing arm, not a silent `?`. Recorded as O-4. |
| H | D14 says a miss "answers `PromptSettingsStale`" | On the `App` rung an `expected: Some(token)` over a row that was **cleared elsewhere** answers `Err(NotFound { entity: "app_setting", id: key })` on both stores (`mem.rs:2262-2288`, `pg/write.rs:1868`, `cas_miss` `:61-72`) — `CasOutcome::Stale` needs a row to carry | Reaches the section as `Failed { request: "set_setting", message: "app_setting \`token_budget\` not found" }`. Hazard H-1; a test pins it; O-3 names the seam change that would fix it. |

### 0.2 Choices (B-n)

| # | Choice | Why |
|---|---|---|
| B-1 | Group headers are **rows** (`Row::AppHeader`, `Row::ProjectHeader`), and `e` on one says `` `e` edits a value row `` | Keeps row index == line index (kinds' invariant for scrolling, `kinds.rs:1533-1542`) without a skip-list in `move_cursor`. |
| B-2 | Stored value renders as the JSON text as stored (`0.1`, `5000`, `"abc"` for a hand-written string); the effective value renders `0.1 (1000 bp)` for the fraction row, from `reserve_bp` | D12's two units come from the resolver, so no rounding is re-implemented here; a hand-written oddity shows as itself beside what the reader made of it (D6's whole point). Editor round-trips the stored text. |
| B-3 | Provenance for the nine non-`token_budget` keys is the **presence** rule `present_source(project_value, app_holds)`; `token_budget` rows ask `resolve_budget(...).source` directly | D5 verbatim. "Usable" and "present" differ only for a row the store never validated (hand SQL); the `stored | effective` pair makes that visible. A test pins `present_source` against `resolve_budget`'s own `source` for the four combinations. |
| B-4 | `project_keys()` is one `pub fn` in `prompt_settings.rs`, used by `snapshot()` **and** the section's `rows()` | The section indexes `ProjectEntry.values[v]` by position; one iterator means the two cannot disagree on which keys are there or in what order. |
| B-5 | `Reload` has `NoRow` instead of kinds' `Keep` | An `App` row cleared elsewhere must make the editor's next set pass `expected: None` (D4, F-2); a create-style `Keep` would carry the dead token. |
| B-6 | Detail pane is drawn in **Browse** for the selected value row (doc, range, rungs, clamp, notes) and stays above the field line in `Editing` | Plan T2: "the pane under the cursor prints `spec.doc` …". Kinds' Browse pane is empty because it has nothing row-specific to say; this section does. |
| B-7 | Section holds `unset` as the stored text for `None` on both rungs | Kinds' `inherit` is wrong on `App` (nothing above it); one word for both rungs. |
| B-8 | The Postgres-gated test calls `prompt_settings::snapshot(&db.store, &scope)` directly | `TestDb.store: PgStore` satisfies `ReadStore + WriteStore`; building a `Backend` over it is not something `htui`'s Postgres tests do (`chat_usage_pg.rs:158-169` uses `Writer::Online(db.store.clone())`). |
| B-9 | A `Failed` for `set_setting`/`clear_setting` leaves the editor open with its text (kinds `:1493-1503`) — including H-1's `NotFound` | One habit across three sections (D14). Recovery for H-1 is `Esc`, `r`, `e`. |

## 1. `crates/htui/src/prompt_settings.rs` (new, T1)

Module doc (mirrors `catalogue.rs:1-9` and carries `:116-119`'s residue): one read per event; every write through M1's seam; the section renders from the snapshot and patches nothing in; **known residue** — a write arm answers `Failed` when the re-read after an applied write fails, and separating the two needs a seam method this milestone does not add. No identity resolved (no `this_user`, no `box_info`).

### 1.1 Imports

```rust
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use htui_core::model::{Project, Scope};
use htui_core::prompt::{Rungs, SettingKey};
use htui_core::store::{CasOutcome, ReadStore, Result, SettingRung, StoreError, WriteStore};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};
use serde_json::Value;

use crate::store_worker::{StoreReply, StoreRequest};
```

### 1.2 Types (D3/D4)

```rust
/// One read of the scope: the ten `App` rows, then every project of the scope that still names a
/// row, each with the keys the `Project` rung accepts.
///
/// `Eq` is absent for the reason `CatalogueSnapshot` gives: `Project` derives `PartialEq` only.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    /// `SettingKey::ALL` order, always ten entries.
    pub app: Vec<AppEntry>,
    /// `scope.project_ids` order; an id that names no row is skipped (D3).
    pub projects: Vec<ProjectEntry>,
}

/// One key on the `App` rung. `updated_at` is `None` exactly when no `app_setting` row exists —
/// the state a set must pass `expected: None` for (D4, F-2). `value` is `Some` iff `updated_at`
/// is, on both stores (`mem.rs:2217-2224`; Pg reads the row or nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    pub key: SettingKey,
    pub value: Option<Value>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// One project with the keys the `Project` rung accepts. `project.updated_at` is the rung's
/// compare-and-set token, held once per project (D4); `project.settings` is the blob the reader's
/// resolvers take (D5/D6).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEntry {
    pub project: Project,
    /// [`project_keys`] order.
    pub values: Vec<ProjectValue>,
}

/// One key's value on the `Project` rung, read through `setting()` so `SettingSpec::project_key`
/// is applied (D2); `None` when the blob holds no such key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectValue {
    pub key: SettingKey,
    pub value: Option<Value>,
}

impl SettingsSnapshot {
    /// The entries that carry a value, keyed by `spec().key` — the shape `Backend::app_settings()`
    /// hands the resolvers (`backend.rs:394`, `mem.rs:389-397`), so D6's numbers are computed over
    /// the reader's own input.
    #[must_use]
    pub fn app_map(&self) -> BTreeMap<String, Value>;

    /// The `App` entry for `key`; `None` only on a snapshot built by hand with fewer than ten.
    #[must_use]
    pub fn app_entry(&self, key: SettingKey) -> Option<&AppEntry>;
}

/// The keys whose spec admits the `Project` rung, in `SettingKey::ALL` order (B-4).
pub fn project_keys() -> impl Iterator<Item = SettingKey> {
    SettingKey::ALL
        .into_iter()
        .filter(|key| key.spec().rungs.contains(Rungs::PROJECT))
}
```

### 1.3 `snapshot`

```rust
/// N+1 by design (M3 D5's trade, same words): ten `setting(App, _)` plus, per project, one
/// `project()` and one `setting(Project(id), _)` per project key — per event, never per keystroke.
/// Bound is `ReadStore + WriteStore` because `setting` lives on `WriteStore` (`traits.rs:584`).
///
/// # Errors
/// Whatever the store reports.
pub async fn snapshot<S: ReadStore + WriteStore + ?Sized>(
    store: &S,
    scope: &Scope,
) -> Result<SettingsSnapshot>
```

Body: for `key in SettingKey::ALL` → `store.setting(SettingRung::App, key).await?` → `AppEntry { key, value: stored.as_ref().and_then(|s| s.value.clone()), updated_at: stored.map(|s| s.updated_at) }`. For `id in &scope.project_ids` → `let Some(project) = store.project(*id).await? else { continue };` then for `key in project_keys()` → `store.setting(SettingRung::Project(*id), key).await?.and_then(|s| s.value)` → `ProjectValue`. A project that vanishes between the two reads yields `value: None` (torn read, H-2), never an error.

### 1.4 `serve`

```rust
/// # Errors
/// Whatever the seam reports, plus `StoreError::Unreachable` offline and `StoreError::Backend`
/// for a request that is not one of this module's three.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>
```

| Arm | Does | Reply |
|---|---|---|
| first line | `backend.writer().ok_or_else(|| StoreError::Unreachable(DATABASE_UNREACHABLE.to_owned()))?` | — |
| `PromptSettings(scope)` | `reread(&writer, scope)` | `PromptSettings` |
| `SetSetting { scope, rung, key, value, expected }` | `writer.set_setting(*rung, *key, value.clone(), *expected).await?` → `cas(&writer, scope, &outcome)` | `PromptSettings` / `PromptSettingsStale` |
| `ClearSetting { scope, rung, key, expected }` | `writer.clear_setting(*rung, *key, *expected).await?` → `cas(...)` | same |
| `other` | `Err(StoreError::Backend(format!("not a prompt settings request: {}", other.name())))` | — |

`reread(writer: &Writer, scope: &Scope) -> Result<StoreReply>` and `cas<T>(writer, scope, outcome: &CasOutcome<T>) -> Result<StoreReply>` are `catalogue.rs:297-315` with the two reply names swapped. No `Utc::now()` anywhere in the file (nothing sets `updated_at` by hand; the seam's clock does).

```rust
/// The three request names, in `StoreRequest` order; `name()`'s arms and the section's `Failed`
/// match both read from here.
pub const REQUEST_NAMES: [&str; 3] = ["prompt_settings", "set_setting", "clear_setting"];
```

### 1.5 `crates/htui/src/lib.rs`

`pub mod prompt_settings;` between `pub mod preview;` and `pub mod store_worker;` (`lib.rs:12-22`, alphabetical).

## 2. `crates/htui/src/store_worker.rs` edits (T1)

| Where | Edit |
|---|---|
| imports `:14-35` | `use htui_core::prompt::SettingKey;`, `SettingRung` added to the existing `htui_core::store::{…}` line, `use serde_json::Value;`, `use crate::prompt_settings::{self, SettingsSnapshot};` after the `catalogue` line `:32`. `chrono::{DateTime, Utc}` already present. |
| `StoreRequest`, after `SetPhaseBudget` (closes `:441`) | three variants below |
| `name()`, after `Self::SetPhaseBudget { .. } => "set_phase_budget"` (`:498`) | `Self::PromptSettings(..) => "prompt_settings"`, `Self::SetSetting { .. } => "set_setting"`, `Self::ClearSetting { .. } => "clear_setting"`, with the comment "The three of `prompt_settings::REQUEST_NAMES`, in that order (MOD-15 M5 D7)". |
| `StoreReply`, after `KindDeleted` (closes `:639`), before `Failed` `:641` | `PromptSettings(Box<SettingsSnapshot>)`, `PromptSettingsStale(Box<SettingsSnapshot>)` |
| `try_serve`, after the catalogue arm (`:902`), before `StoreRequest::StoreState` (`:903`) | `StoreRequest::PromptSettings(..) | StoreRequest::SetSetting { .. } | StoreRequest::ClearSetting { .. } => prompt_settings::serve(backend, request).await?,` — one or-ed arm, no guard (F-13, E0004). |

```rust
    /// The ten prompt keys on `App` plus the `Project`-rung keys of every scope project (M5 D3).
    PromptSettings(Scope),
    /// `set_setting` on `App` or `Project` (M5 D7). `expected: None` is "I expect no row", accepted
    /// on `App` only (`traits.rs:553-560`); the section always passes `Some(project.updated_at)`
    /// for a project. CAS on the rung row's `updated_at`.
    SetSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which rung is written; never `Phase` from the prompt section (M5 D8).
        rung: SettingRung,
        /// Which key.
        key: SettingKey,
        /// The JSON to store: an integer, or an `f64` for the one fraction key (M5 D12).
        value: Value,
        /// The rung row's `updated_at` the editor opened on; `None` for an absent `App` row.
        expected: Option<DateTime<Utc>>,
    },
    /// `clear_setting` on `App` or `Project`, so the rung below answers (M5 D10).
    ClearSetting {
        /// The scope the reply re-reads.
        scope: Scope,
        /// Which rung is cleared.
        rung: SettingRung,
        /// Which key.
        key: SettingKey,
        /// The rung row's `updated_at` the editor opened on.
        expected: DateTime<Utc>,
    },
```

`StoreRequest` stays `#[derive(Debug, Clone)]` — `SettingRung` is `Copy + Debug` (`traits.rs:799`), `SettingKey` is `Copy + Eq + Hash`, `Value` is `Clone + Debug`; nothing secret (F-24). `Discriminant<StoreRequest>` (`app/state.rs:158`) gains three keys; the read is one variant so the staleness index keeps its newest reply.

Every `match reply` outside the worker carries `_ => {}` (F-14), so the two replies compile untouched. `StoreRequest` count 47 → **50**, `StoreReply` 26 → **28**.

## 3. Worker tests — `crates/htui/tests/prompt_settings.rs` (T1 half)

Header: `#![cfg(feature = "testkit")]`; helpers as `tests/kinds.rs:36-90`: `demo() -> Backend` (`Backend::memory(MemStore::demo())`), `vulkan_scope()`, `settings(reply) -> SettingsSnapshot` (panics on anything but `PromptSettings`), `refusal(reply) -> (&'static str, String)`, `demo_settings(&backend)`, `nil_scope()`, `prompt_requests() -> Vec<StoreRequest>` (one of each of the three).

| Test | Asserts |
|---|---|
| `prompt_settings_names_are_stable` | `REQUEST_NAMES` equals `prompt_requests().iter().map(name)` and each name is unique |
| `the_demo_snapshot_has_ten_app_entries_and_two_keys_per_project` | `app.len() == 10`, keys == `SettingKey::ALL`, every `value` and `updated_at` `None` (F-7); one `ProjectEntry` per scope project in scope order, `values` keys == `project_keys()` (today `[UpstreamHops, TokenBudget]`) |
| `app_map_holds_only_entries_with_a_value` | after one `SetSetting(App, MaxSkillTokens, 4000, None)`, `app_map()` == `{"max_skill_tokens": 4000}` |
| `an_unknown_project_id_is_skipped` | scope with `ProjectId::new()` appended → same projects as without |
| `set_setting_on_app_with_no_row_applies_then_the_old_token_is_stale` | first `SetSetting { expected: None }` → `PromptSettings` whose entry has `value: Some(v)`, `updated_at: Some(t)`; second `SetSetting { expected: None }` (no row expected, row present) → `PromptSettingsStale`; third with `Some(t)` → applied; fourth re-using `Some(t)` → `PromptSettingsStale` |
| `clear_setting_on_app_returns_the_entry_to_none` | `ClearSetting { expected: Some(t) }` → entry `value: None, updated_at: None`; a following `SetSetting { expected: None }` applies (plan risk row 4) |
| `a_token_over_a_cleared_app_row_answers_failed_not_found` (H-1) | set, clear, then `SetSetting { expected: Some(old) }` → `Failed { request: "set_setting", message }` with `message.contains("app_setting")` and `contains("not found")` |
| `set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys` | `set_setting(Project, UpstreamHops, 2, Some(updated_at))` through `serve`, then read `writer.project(id).settings` and assert `settings["upstream_hops"] == 2` **and** no `"prompt_upstream_hops"` key (M1 live coordinate 2); assert `values[0].value == Some(2)` in the reply. For the foreign key: the fixture's `project.settings` is `{}` (`mem.rs:1604`), so the test writes one key through the seam first, then the other, and asserts both survive |
| `an_out_of_range_value_is_refused_with_the_seams_sentence` | `SetSetting(App, TokenBudget, 0, None)` → `Failed { request: "set_setting", message }`; `message.contains("`token_budget` = 0 is outside 1..=")` and `contains("tokens")` — key and range, not the whole string |
| `a_fraction_typed_as_basis_points_is_refused_by_rounding` | `SetSetting(App, PromptReserveFraction, 1000.0, None)` → message contains `rounds to 10000000 bp, outside 0..=5000 bp` |
| `a_key_on_a_rung_its_spec_refuses_answers_the_rung_sentence` | `SetSetting(Project(vulkan), MaxSkillTokens, 1, Some(t))` → message contains `` `max_skill_tokens` is not accepted on the project rung `` |
| `offline_refuses_all_three_by_name` | `CacheStore::open` + `Backend::Offline` as `tests/kinds.rs:239-260`; each reply is `Failed { request: name, message: DATABASE_UNREACHABLE }` |
| `serve_refuses_a_foreign_request_by_name` | `prompt_settings::serve(&demo(), &StoreRequest::Catalogue(scope))` → `Err(StoreError::Backend(m))` with `m == "not a prompt settings request: catalogue"` |
| `a_migrated_postgres_presents_ten_app_rows_with_tokens` (`#[tokio::test(flavor = "multi_thread")]`) | `let Some(db) = htui_store::testkit::demo_db().await else { return };` (prints `SKIP` inside); `prompt_settings::snapshot(&db.store, &scope)` → ten entries all `value.is_some() && updated_at.is_some()`; `app_map()` equals `DEFAULTS.as_rows()` as a map (`0002_agent_probe.sql:68-79`); `db.drop_db().await` |

## 4. `crates/htui/src/ui/tabs/settings/prompt.rs` (new, T2)

### 4.1 Imports

```rust
use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use chrono::{DateTime, Utc};
use htui_core::model::Scope;
use htui_core::prompt::settings::{
    resolve_budget, resolve_excerpt_caps, resolve_hops, resolve_max_skill_tokens,
};
use htui_core::prompt::{BudgetSource, SettingKey, SettingKind, SettingSpec};
use htui_core::store::SettingRung;
use serde_json::Value;

use crate::app::{Ctx, Handled};
use crate::prompt_settings::{AppEntry, ProjectEntry, REQUEST_NAMES, SettingsSnapshot, project_keys};
use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::tabs::settings::{
    CHANGED_ELSEWHERE, CHANGED_ELSEWHERE_CLOSED, DELETED_ELSEWHERE, SectionId, SettingsSection,
    message,
};
use crate::ui::{FieldOutcome, TextField, Theme};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
```

### 4.2 Constants

| Const | Text | Use |
|---|---|---|
| `NOT_READ` | `settings not read yet` | rows pane, `snapshot == None` |
| `UNAVAILABLE` | `settings unavailable` | prefix of the refused-read line |
| `UNSET` | `unset` | stored text for `None` (B-7) |
| `HINT_BROWSE` | `j/k · e edit · r reload` | Browse with a snapshot |
| `HINT_NO_SNAPSHOT` | `r reload` | Browse, nothing read or unavailable |
| `HINT_EDITING` | `Enter save · Esc cancel · empty clears` | one field, so no Tab |
| `NOT_A_VALUE_ROW` | `` `e` edits a value row `` | `e` on a header (B-1) |
| `NOTHING_SET` | `nothing is set on this rung` | D10, empty field over `None` |
| `APP_HEADER` | `app` | the `App` group line |
| `PROJECT_HEADER` | `project` | prefix of `project {name}` |

Sentences built by `fn`: `integer_sentence(key)` = ``"`{key}` is a whole number, or empty to clear"``, `fraction_sentence(key)` = ``"`{key}` is a decimal fraction, or empty to clear"`` (D11 verbatim), `clamp_line(peer, n)` = `"clamped to {peer} = {n}"` (D13), `fraction_text(&Budget)` = `"{reserve} ({reserve_bp} bp)"` from `Budget::reserve()` and `Budget.reserve_bp` (B-2, flag A); `in_flight(busy)` as `kinds.rs:1831-1838`.

### 4.3 Types

```rust
/// One line of the tree, by index into the snapshot (D9). Indices, not ids, for kinds' reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// The `app` group line (B-1).
    AppHeader,
    /// `snapshot.app[i]`.
    App { i: usize },
    /// The `project {name}` group line.
    ProjectHeader { p: usize },
    /// `snapshot.projects[p].values[v]`.
    Project { p: usize, v: usize },
}

/// What an editor writes back to: one rung, one key. Both `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Target {
    rung: SettingRung,
    key: SettingKey,
}

/// The open editor: the target, its one field, and the token it opened on.
struct Editor {
    target: Target,
    /// The buffer; its `Debug` never prints the text.
    input: TextField,
    /// `updated_at` of the rung row this opened on; `None` for an absent `App` row (D4).
    expected: Option<DateTime<Utc>>,
}

/// Target and token, never the buffer (H-5).
impl core::fmt::Debug for Editor { /* debug_struct("Editor").field("target").field("expected").finish() */ }

/// Browsing, or typing into one row.
#[derive(Default)]
enum Mode {
    #[default]
    Browse,
    Editing(Editor),
}

impl core::fmt::Debug for Mode { /* "Browse" | debug_tuple("Editing").field(editor) */ }

/// The last outcome, and whether it is one the user has to act on (kinds `:335-355`, verbatim).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice { Info(String), Error(String) }
impl Notice { fn text(&self) -> &str; fn is_error(&self) -> bool; }

/// The rung-row's token after a reload (D14; B-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reload {
    /// The project the editor opened on is not in the reloaded snapshot.
    Gone,
    /// The rung row's `updated_at` as it is now.
    Token(DateTime<Utc>),
    /// An `App` row that no longer exists: the next set passes `expected: None`.
    NoRow,
}

/// The reader's answer for one row: what it would use, from which rung, and any note it made.
struct Effective {
    /// As the row prints it (`5000`, `0.1 (1000 bp)`).
    text: String,
    /// The same as a number, for D13's comparison; `None` for the fraction.
    number: Option<i64>,
    source: BudgetSource,
    /// `resolve_hops`'s clamp note, when it made one (flag B).
    notes: Vec<String>,
}

/// The prompt settings of the scope, with the keys that edit them (D9/D10).
#[derive(Debug, Default)]
pub struct PromptSection {
    snapshot: Option<SettingsSnapshot>,
    /// `Some(message)` after `Failed { request: "prompt_settings" }`.
    unavailable: Option<String>,
    cursor: usize,
    mode: Mode,
    /// The write in flight, by `StoreRequest::name` (kinds' rule, `:368-371`).
    busy: Option<&'static str>,
    notice: Option<Notice>,
}

impl PromptSection {
    pub const ID: SectionId = SectionId("prompt");
    #[must_use] pub fn new() -> Self { Self::default() }
}
```

### 4.4 Rows and lookups

```rust
fn rows(&self) -> Vec<Row>            // AppHeader, App{0..10}, then per project: ProjectHeader, Project{p, 0..values.len()}
fn move_cursor(&mut self, down: bool) // kinds :432-442, no wrap
fn clamp_cursor(&mut self)            // kinds :448-450
fn selected(&self) -> Option<Row>
fn app(&self, i: usize) -> Option<&AppEntry>
fn project(&self, p: usize) -> Option<&ProjectEntry>
fn target_of(&self, row: Row) -> Option<(Target, Option<&Value>, Option<DateTime<Utc>>)>
    // App{i}       → (Target{App, e.key},                   e.value.as_ref(), e.updated_at)
    // Project{p,v} → (Target{Project(p.project.id), val.key}, val.value.as_ref(), Some(p.project.updated_at))
    // headers      → None
fn holds(&self, target: Target) -> Option<bool>   // Some(value.is_some()) for the target's row now; None when the row is gone
fn blocked(&mut self) -> bool         // kinds :499-505
fn send(&mut self, request: StoreRequest, ctx: &Ctx<'_>)   // kinds :726-729
fn say(&mut self, text: &str) / fn refuse(&mut self, text: String)   // kinds :1320-1331, `super::is_error`
```

`rows()` uses `values.len()` from the snapshot, never `project_keys()` recomputed (H-8); `snapshot()` built `values` from `project_keys()` so the two agree.

### 4.5 Provenance (D5) and effective value (D6)

```rust
/// D5's presence rule for every key but `token_budget`, pinned against `resolve_budget` (B-3).
fn present_source(project_value: Option<&Value>, app_holds: bool) -> BudgetSource {
    match (project_value, app_holds) {
        (Some(_), _) => BudgetSource::Project,
        (None, true) => BudgetSource::AppSetting,
        (None, false) => BudgetSource::AppSettingDefault,
    }
}

/// What the reader would use for `target`'s key, over the snapshot's own `app_map` and — on a
/// project row — that project's blob (D6). Exhaustive on purpose (flag G, O-4).
fn effective(
    key: SettingKey,
    project: Option<&Value>,        // `Some(&entry.project.settings)` on a project row, `None` on App
    project_value: Option<&Value>,  // the row's own stored value on the Project rung, `None` on App
    app: &BTreeMap<String, Value>,
) -> Effective
```

| `SettingKey` | Effective from | `number` | `source` |
|---|---|---|---|
| `TokenBudget` | `resolve_budget(None, project, app)` → `.tokens` | `Some(tokens)` | `budget.source` (the reader's own) |
| `UpstreamHops` | `resolve_hops(project, app, &mut notes)` | `Some(i64::from(hops))` | `present_source(project_value, app.contains_key(key.key()))`; `notes` carried |
| `MaxSkillTokens` | `resolve_max_skill_tokens(app)` | `Some(n)` | presence |
| `PromptReserveFraction` | `resolve_budget(None, None, app)` → `fraction_text(&budget)` (flag A) | `None` | presence |
| `ExcerptFileLineCap` | `resolve_excerpt_caps(app).0.file_line_cap` | `Some` | presence |
| `ExcerptHeadLines` | `.0.head_lines` (already `.min(file_line_cap)`, `settings.rs:739-741`) | `Some` | presence |
| `ExcerptMaxFileBytes` | `.0.max_file_bytes` | `Some(i64::try_from(..).unwrap_or(i64::MAX))` | presence |
| `ExcerptMaxFiles` | `.0.max_files` | `Some` | presence |
| `ExcerptMaxScanFiles` | `.1` | `Some` | presence |
| `ExcerptProviderDeadlineMs` | `.2.as_millis()` | `Some(i64::try_from(..).unwrap_or(i64::MAX))` | presence |

`app` is `snapshot.app_map()`, computed once per `lines()`/`pane()` call (ten entries; not cached, D3). The compiled default is never read from `Defaults::integer` (private, F-10); it reaches the screen through the resolvers, and `DEFAULTS.value_of(key)` is only used by the tests that assert the demo tree.

Source label text: `BudgetSource::as_str()` (`settings.rs:567-574`): `app_setting`, `app_setting_default`, `project`; `phase` is unreachable from this section (D8).

### 4.6 Row line (D9), width 100

```
app
  excerpt_file_line_cap         unset | 400 (app_setting_default)   lines
  …
  prompt_reserve_fraction       0.1 | 0.1 (1000 bp) (app_setting)   bp
  token_budget                  unset | 100000 (app_setting_default)   tokens
project Vulkan renderer
  prompt_upstream_hops          2 | 2 (project)   hops
  token_budget                  unset | 100000 (app_setting_default)   tokens
```

`fn value_line(key, stored: Option<&Value>, eff: &Effective) -> String` = `format!("  {key:<width$}  {stored} | {} ({})   {unit}", eff.text, eff.source.as_str())` where `width = key_width()` (max `key().chars().count()` over `SettingKey::ALL`, computed, not a constant), `stored = stored.map_or(UNSET, Value::to_string)` (B-2), `unit = key.spec().unit`. Header lines are `APP_HEADER` and `format!("{PROJECT_HEADER} {}", entry.project.name)`. Selected row in `theme.selected`, headers and value rows in `theme.base`. Everything spelled from `SettingKey::key()`/`SettingSpec` — no key literal in the file (acceptance line 4; a test greps the source for the ten key strings).

### 4.7 Detail pane (B-6) and the clamp line (D13)

`fn pane(&self, width: u16, theme: &Theme) -> Vec<Line<'static>>`:

| Line | Text | When |
|---|---|---|
| 1 | `spec.doc`, wrapped, in `theme.dim` | a value row is selected (Browse or Editing) |
| 2 | `range {min}..={max} {unit} · rungs {spec.rungs}` (`Rungs` `Display`: `app`, `app|project`, `app|project|phase`; `settings.rs:254-272`) | same |
| 3 | `clamp_line(peer.key(), n)` in `theme.error` | `spec.not_above == Some(peer)`, `stored.as_i64() == Some(s)`, `eff.number == Some(n)`, `n < s` |
| 4.. | each of `eff.notes` in `theme.error` | `resolve_hops` made one |
| last | `{key}: {field}` — `Span::styled(format!("{key}: "), theme.accent)` + `input.line(room, true, theme).spans` (kinds `Editor::lines`, `:1587-1612`, one field) | `Mode::Editing` |

`wrapped` is copied from `kinds.rs:1893-1913` (private there; a three-line helper is cheaper than a fourth `pub(crate)` promotion — O-5). A header row selected → empty pane.

### 4.8 Hint line

`hint(width, theme)` is `kinds.rs:1229-1250` verbatim; `hint_text()`: `Editing` → `HINT_EDITING`; Browse → `HINT_NO_SNAPSHOT` when `unavailable.is_some() || snapshot.is_none()`, else `HINT_BROWSE`; `busy` suffix `{keys} · {busy} in flight` in Browse with no notice (`:1272-1279`).

### 4.9 Keys

Browse (`on_key`, after the `Editing` dispatch):

| Key | Does |
|---|---|
| `e` | `!blocked()` and `selected()` → `open_edit(row)`; on a header `say(NOT_A_VALUE_ROW)`; `Consumed` |
| `j`/`Down`, `k`/`Up` | `move_cursor` |
| `r` | `ctx.request(StoreRequest::PromptSettings(ctx.scope.clone()))`, never blocked (kinds `:1458-1466`) |
| `Esc` | only `if self.notice.is_some()` → clear; else `Pass` (H-13) |
| other | `Pass` |

`open_edit(row)`: `target_of(row)` → `Editor { target, input: TextField::with_text(&stored.map_or(String::new(), Value::to_string)), expected }`; `self.notice = None; self.mode = Mode::Editing(editor)`.

Editing (`on_editor_key`, kinds `:736-776` minus the Tab arms): `input.on_key(key)` → `Consumed`; `Submit` → `submit(ctx)`; `Cancel` → `Browse`, `notice = None`; `Pass` → `CONTROL` chords `Pass`, everything else `Consumed` (so `l`, `q`, digits are letters — `captures_input` is `!matches!(self.mode, Mode::Browse)`).

### 4.10 `submit` (D10/D11)

```
if let Some(busy) = self.busy            → refuse(in_flight(busy)); return
let Mode::Editing(editor) = &self.mode   else return
let text = editor.input.text().unwrap_or_default().trim()
let Some(holds) = self.holds(editor.target) else { mode = Browse; say(DELETED_ELSEWHERE); return }
if text.is_empty():
    if !holds                            → say(NOTHING_SET); return          (no request)
    let Some(expected) = editor.expected else { say(NOTHING_SET); return }   (unreachable by the AppEntry value ⇔ updated_at invariant; a guard, not an expect)
    send(ClearSetting { scope: ctx.scope.clone(), rung, key, expected })
else:
    value = match key.spec().kind {
        Integer  => text.parse::<i64>().map(Value::from).map_err(|_| integer_sentence(key)),
        Fraction => text.parse::<f64>().ok().filter(|f| f.is_finite()).map(Value::from).ok_or_else(|| fraction_sentence(key)),
    }
    Err(sentence) → refuse(sentence); return
    send(SetSetting { scope: ctx.scope.clone(), rung, key, value, expected: editor.expected })
```

Nothing else is checked: `min`/`max`/`not_above`/Phase narrowing are `validate`'s (`settings.rs:479-540`) and its sentence comes back verbatim in `Failed`. The editor stays open until the reply (kinds `:778-783`).

### 4.11 Replies

| Reply | Handler | Does |
|---|---|---|
| `PromptSettings(s)` | `on_settings(s)` | `write = busy.take(); unavailable = None; snapshot = Some(s.clone()); clamp_cursor(); if write.is_some() { notice = None; if Editing → Browse }` (kinds `:1350-1366`, H-3's argument carried in the doc comment) |
| `PromptSettingsStale(s)` | `on_stale(s)` | `busy = None; reloaded = Editing → Some(reload(s, target)); snapshot = Some(s.clone()); clamp; match reloaded { None → say(CHANGED_ELSEWHERE_CLOSED); Some(Gone) → Browse + say(DELETED_ELSEWHERE); Some(Token(t)) → editor.expected = Some(t), say(CHANGED_ELSEWHERE); Some(NoRow) → editor.expected = None, say(CHANGED_ELSEWHERE) }` |
| `Failed { request: "prompt_settings", message }` | inline | `unavailable = Some(message.clone())`; `busy` untouched (kinds `:1482-1489`) |
| `Failed { request, message } if REQUEST_NAMES.contains(request)` | inline | `busy = None; refuse(message.clone())`; editor stays (B-9) |
| `_` | — | ignored |

```rust
/// The token an open editor retries against after a reload (D14, B-5).
fn reload(snapshot: &SettingsSnapshot, target: Target) -> Reload {
    match target.rung {
        SettingRung::App => snapshot.app_entry(target.key)
            .map_or(Reload::Gone, |e| e.updated_at.map_or(Reload::NoRow, Reload::Token)),
        SettingRung::Project(id) => snapshot.projects.iter()
            .find(|p| p.project.id == id)
            .map_or(Reload::Gone, |p| Reload::Token(p.project.updated_at)),
        SettingRung::Phase(_) => Reload::Gone,   // never opened here (D8)
    }
}
```

### 4.12 `impl SettingsSection for PromptSection`

`id()` → `Self::ID`; `title()` → `"Prompt"`; `wants_requests(scope)` → `vec![StoreRequest::PromptSettings(scope.clone())]`; `on_scope_change` → `snapshot = None; mode = Browse; busy = None; cursor = 0` (notice survives, kinds `:1382-1390`); `captures_input` → `!matches!(self.mode, Mode::Browse)`; `on_key`/`on_reply` as §4.9/§4.11; `render`:

```
pane = self.pane(area.width, theme)
[rows, pane_area, hint] = Layout::vertical([Min(3), Length(pane.len()), Length(1)])
(Some(why), _)  → Paragraph "{UNAVAILABLE}: {why}" in theme.error, wrapped (the refusal wins over a stale tree)
(None, None)    → message(NOT_READ)
(None, Some(_)) → Paragraph(lines).scroll(cursor.saturating_sub(height - 1))   — the App group is never empty, so there is no NO_PROJECT case
pane, hint as kinds :1545-1548
```

### 4.13 In-module test

`#[cfg(test)] mod tests` — `an_editor_never_prints_its_buffer`: `Editor { target: Target { rung: SettingRung::App, key: SettingKey::TokenBudget }, input: TextField::with_text("424242"), expected: None }` inside `PromptSection { mode: Mode::Editing(..), ..Default::default() }`; `format!("{section:?}")` does not contain `424242`, does contain `Editing` and `TokenBudget` (kinds `:1925-1965`, one concept across).

## 5. `settings/mod.rs` and `kinds.rs` edits (T2)

| File | Edit |
|---|---|
| `settings/mod.rs:11-13` | `pub mod prompt;` after `pub mod kinds;` |
| `settings/mod.rs:28-30` | `pub use prompt::PromptSection;` |
| `settings/mod.rs`, after `is_error` (`:59`) | `pub(crate) const CHANGED_ELSEWHERE`, `CHANGED_ELSEWHERE_CLOSED`, `DELETED_ELSEWHERE` — text and doc comments moved verbatim from `kinds.rs:63-77`, with "(M4 D14, promoted for M5)" appended; `is_error`'s doc gains "and the prompt section" |
| `kinds.rs:63-77` | the three `const`s deleted; call sites become `super::CHANGED_ELSEWHERE_CLOSED` etc. (or one `use super::{…}` beside `:31`) |

`hierarchy.rs` untouched (flag D). `kinds__*.snap` and `hierarchy__*.snap` unchanged — same bytes on screen.

## 6. Registration and the strip pin (T2)

| File | Edit |
|---|---|
| `app/mod.rs:47-51` | `Box::new(PromptSection::new())` appended after `KindsSection`; the `use crate::ui::tabs::settings::{…}` line gains `PromptSection` |
| `tests/settings.rs:943-947` | `Box::new(PromptSection::new())` appended to `the_section_strip_fits_the_frame`'s vector; assertion unchanged (34 ≤ 100, flag E) |

## 7. Section tests — `tests/prompt_settings.rs` (T2 half) and snapshots

Helpers as `tests/kinds.rs:763-838`, `:1059-1065`: `prompt_over(store) -> Harness` (four sections, three `l`), `bench_scope()`, `bench_with(backend) -> (SectionBench, PromptSection, SettingsSnapshot)` (feeds `PromptSettings(demo_settings)` and drains), `drawn`, `error_text`, `type_at`. Seeding a value for a section test goes through `store_worker::serve(&backend, &SetSetting{…})` and feeds the reply's snapshot to the section — the bench's own store is never written (`R-NF-3`: the section sees replies only).

| Test | Asserts | Snapshot |
|---|---|---|
| `the_demo_snapshot_renders_the_tree` | render contains `app`, ten key lines each `unset | … (app_setting_default)`, `project Vulkan…` with two rows; the ten effective numbers equal `DEFAULTS.value_of(key)` rendered | `prompt_settings__demo` |
| `no_workspace_lists_the_app_group_alone` | nil scope → `app` header + ten rows, no project header | `prompt_settings__app_only` |
| `offline_is_unavailable_with_the_worker_sentence` | frame contains `settings unavailable` and `DATABASE_UNREACHABLE` | `prompt_settings__offline` |
| `no_key_is_listed_on_a_rung_its_spec_refuses` | for every `Row::Project{p,v}` the key's `spec().rungs.contains(PROJECT)`; count of project rows == `project_keys().count()` per project; ten `Row::App` | |
| `no_key_name_is_spelled_in_the_section` | `include_str!("../src/ui/tabs/settings/prompt.rs")` contains none of `SettingKey::ALL.map(key)` as a quoted literal (acceptance line 4) | |
| `e_then_a_number_then_enter_sends_set_setting_with_the_entrys_token` | cursor on `token_budget` (App, no row): `e`, type `4000`, `enter` → exactly `[Action::Store(SetSetting { rung: App, key: TokenBudget, value: 4000, expected: None, scope })]` | |
| `e_on_a_project_row_carries_the_projects_updated_at` | `SetSetting { rung: Project(vulkan), key: UpstreamHops, expected: Some(project.updated_at) }` | |
| `an_empty_field_clears_when_the_rung_holds_a_value` | seed `max_skill_tokens` via `serve`, feed snapshot, `e`, backspaces, `enter` → `ClearSetting { expected: Some(t) }` | |
| `an_empty_field_on_an_unset_rung_sends_nothing_and_says_so` | `e`, `enter` on an unset row → `drained().is_empty()`, notice `nothing is set on this rung` (not in error colour) | |
| `a_shape_refusal_names_the_key` | `abc` on an Integer row → no request, error text `` `token_budget` is a whole number, or empty to clear ``; fraction row likewise | |
| `a_stale_reply_keeps_the_text_and_retakes_the_token` | `e`, type, `enter` (busy), feed `PromptSettingsStale` with the row at `t2` → editor open, text intact, `CHANGED_ELSEWHERE`; second `enter` → `SetSetting { expected: Some(t2) }` | `prompt_settings__stale` |
| `a_stale_reply_over_a_cleared_app_row_passes_no_token` | reload without the row → second `enter` carries `expected: None` (B-5) | |
| `a_stale_reply_for_a_vanished_project_closes_the_editor` | project rung, reload without the project → `Browse`, `DELETED_ELSEWHERE` | |
| `a_stale_reply_with_no_editor_open_says_nothing_was_written` | `CHANGED_ELSEWHERE_CLOSED` (M4 H-9 inherited) | |
| `a_failed_write_shows_the_seams_sentence_verbatim` | feed `Failed { request: "set_setting", … }` → `busy` cleared, editor open, error text carries the message | |
| `a_second_write_while_one_is_in_flight_is_refused` | `enter` twice → one request, `` `set_setting` is still in flight `` | |
| `the_source_label_matches_resolve_budget_for_all_four_combinations` | `present_source(project.get("token_budget"), app.contains_key(..)) == resolve_budget(None, project, &app).source` over the four combinations (B-3) | |
| `a_project_row_flips_to_project_and_back` | seed `token_budget` on the project → `(project)`; clear → `(app_setting_default)` (the `--demo` smoke, pinned) | |
| `a_head_above_the_cap_renders_the_clamp_line` | `SetSetting(App, ExcerptHeadLines, 50, None)` then `SetSetting(App, ExcerptFileLineCap, 10, None)` (accepted: one-directional, F-12); pane has `clamped to excerpt_file_line_cap = 10` in error colour; row shows `50 | 10 (app_setting)` | `prompt_settings__clamped` |
| `the_fraction_row_renders_both_units_and_round_trips` | seed `0.1` → row `0.1 | 0.1 (1000 bp) (app_setting)`; `e` → field `0.1`; `enter` → `SetSetting { value: 0.1 }` | `prompt_settings__editor_fraction` |
| `the_pane_prints_doc_range_and_rungs` | `token_budget` (App) → pane has `spec.doc`, `range 1..=9223372036854775807 tokens`, `rungs app|project|phase` | |
| `l_is_a_letter_while_editing` | `captures_input()` false in Browse, true after `e`; `l` reaches the field, no cycle | |
| `e_on_a_header_edits_nothing` | cursor 0 → `e` → no mode change, notice `` `e` edits a value row `` | |
| `r_reloads_and_esc_clears_the_notice` | `r` → `[PromptSettings(scope)]`; `Esc` with notice → cleared, `Consumed`; without → `Pass` | |
| `a_scope_change_drops_the_editor_and_keeps_the_notice` | kinds `:1026-1057` shape | |
| `q_quits_from_browse` | `Harness` `q` → `should_quit` | |

Snapshots created (six): `prompt_settings__demo.snap`, `__app_only.snap`, `__offline.snap`, `__stale.snap`, `__clamped.snap`, `__editor_fraction.snap`. None outside `prompt_settings__*` changes (flag F).

## 8. Data flow

1. Tab activation / scope change → `SettingsTab::wants_requests` → `PromptSettings(scope)` → `try_serve` → `prompt_settings::serve` → `snapshot()` (ten `setting(App)`, per project `project()` + two `setting(Project)`) → `StoreReply::PromptSettings` → `PromptSection::on_settings` → `snapshot` replaced whole.
2. Render: `rows()` from the snapshot; per value row `app_map()` + `effective()` → `stored | effective (source) unit`; selected row → pane (doc, range, rungs, clamp, notes).
3. `e` → `Editor { target, input, expected }` from the row; `Enter` → shape parse only → `SetSetting`/`ClearSetting` with the row's token → `busy = name`.
4. Worker: `set_setting`/`clear_setting` (rung refusal → kind → range → `not_above`, then CAS) → `cas()` re-reads → `PromptSettings` (applied: editor closes) / `PromptSettingsStale` (miss: text kept, token re-taken via `reload`) / `Failed` (constraint or `NotFound`: sentence shown, editor open).

## 9. Build order and commit plan

| # | Commit | Files | Compile-visible steps |
|---|---|---|---|
| 1 | `test(htui): prompt_settings worker tests (red)` | `tests/prompt_settings.rs` (worker half) | fails to compile: no `prompt_settings`, no variants |
| 2 | `feat(htui): prompt_settings worker — snapshot, three requests, two replies (M5 T1)` | `store_worker.rs` (variants → `name()` E0004 → arms; replies; `try_serve` E0004 → or-ed arm), `prompt_settings.rs`, `lib.rs` | green; `CASES` 36 / `EXPECTED_CASES` 36; `git status` shows no `.sqlx/`, `migrations/` |
| 3 | `refactor(htui): the three CAS sentences move to settings/mod.rs (M5 flag D)` | `settings/mod.rs`, `kinds.rs` | green, `kinds__*.snap` unchanged |
| 4 | `test(htui): prompt section tests (red)` | `tests/prompt_settings.rs` (section half) | fails: no `PromptSection` |
| 5 | `feat(htui): PromptSection — rows, provenance, pane (M5 T2a)` | `prompt.rs` (§4.3–4.8, §4.12 without editor), `settings/mod.rs` (`mod`/`use`), `app/mod.rs`, `tests/settings.rs` | render tests green; `demo`, `app_only`, `offline` snapshots read then accepted |
| 6 | `feat(htui): PromptSection — one-field editor, clear on empty, CAS (M5 T2b)` | `prompt.rs` (§4.9–4.11, 4.13) | remaining tests green; `stale`, `clamped`, `editor_fraction` accepted |
| 7 | `docs(mod-15): milestone 5 landed` | `HANDOFF.md:415` region, PRD `:253` row | `validate-workflow-docs.sh` green |

Gates per plan T1/T2/T3; `--demo` smoke: no TTY in the agent environment — the close-out says so (M3/M4 precedent); `a_project_row_flips_to_project_and_back` pins the smoke's observation.

## 10. Hazards

| # | Hazard | Guard |
|---|---|---|
| H-1 | `App` rung: editor's `expected: Some(t)` over a row cleared elsewhere → `Failed` (`NotFound`), not `Stale` (flag H); the editor keeps a dead token and a second `Enter` fails the same way | Test `a_token_over_a_cleared_app_row_answers_failed_not_found`; the seam's sentence names `app_setting`; recovery `Esc`, `r`, `e`; O-3 |
| H-2 | Torn read: project vanishes between `project()` and `setting(Project)` → `value: None` shown as `unset`; the write then fails `NotFound` | Same treatment as `catalogue::snapshot`'s skipped project; `Failed` shows the sentence |
| H-3 | A read reply mistaken for a write's (M4 H-9) closes the editor early | `r` is a letter while editing; the two doors (scope change, tab re-activation) argued in `on_settings`'s doc; `CHANGED_ELSEWHERE_CLOSED` covers the stale case |
| H-4 | Two writes racing under one discriminant lose a reply | `busy` by name blocks any second write until the reply |
| H-5 | `Debug` of `PromptSection` prints the buffer | Hand-written `Editor`/`Mode` `Debug`; in-module test |
| H-6 | `Value::from(f64)` on NaN/inf is `Null` | Section filters `is_finite()` first; the seam sentence stays as backstop |
| H-7 | `effective()` non-exhaustive after an eleventh key | The `match` has no wildcard: compile error names the arm (flag G) |
| H-8 | Section and worker disagree on the project keys → `values[v]` indexes wrong | One `project_keys()` (B-4); `rows()` uses `values.len()` |
| H-9 | Clamp line hard-codes a key | Derived from `spec.not_above` and `eff.number`; the test seeds the pair through the seam |
| H-10 | A fraction typed as `1000` | Refused by the seam (`rounds to 10000000 bp, outside 0..=5000 bp`); worker test pins; row shows both units |
| H-11 | Hand-written non-numeric `App` row (`"abc"`) labelled `app_setting` while the reader used the default | `stored | effective` shows `"abc" | 400 (app_setting)` — the divergence D6 exists to show |
| H-12 | Strip overflow | 34 columns of 100; pin re-run with four |
| H-13 | `Esc` swallowed in Browse breaks overlay close | Only consumed when a notice is set |
| H-14 | `unavailable` left set after a store recovers | `on_settings` clears it; `Failed` for `prompt_settings` never touches `busy` |
| H-15 | Fraction stored text `0.1` vs `f64` printing `0.1` | Stored side prints the JSON `Value`, effective side prints `Budget::reserve()`; the test asserts the exact line |
| H-16 | A project row for a project whose `settings` is not an object | The seam refuses (`settings_not_an_object`, `mem.rs:2317`); `resolve_budget`'s `project_key` read answers `None`; row shows `unset` |

## 11. What must not change

`crates/htui-core/src/store/**` (no seam method; `CASES` 36), `crates/htui-core/src/prompt/settings.rs` (read only; `resolve_reserve_bp` stays private — flag A routes around it), `crates/htui-store/**` (no `query!`, `.sqlx/` byte-identical, no migration), `hierarchy.rs` section (its own sentences), `ui/text_field.rs`, `app/action.rs`, every existing `.snap` (flag F), workspace lints, MSRV 1.98, `unsafe_code = "forbid"`. `SetPhaseBudget` and `catalogue::REQUEST_NAMES: [&str; 9]` stay (D8). Nothing in `prompt_settings.rs` or `prompt.rs` calls `Utc::now()`.

## 12. Open items

| # | Item |
|---|---|
| O-1 | PRD open question 2 (section order) stays open; `Prompt` is appended (D15). |
| O-2 | `SetPhaseBudget` is not folded into `SetSetting`: the two differ in reply type, not request (D8). If a third caller wants the phase rung, the *reply* generalises. |
| O-3 | Flag H: a `Stale` that could carry "no row" (`CasOutcome::Stale(Option<StoredSetting>)` or a `NoRow` outcome) would let the `App` editor self-heal after a clear elsewhere; a seam change, so a later milestone's. |
| O-4 | Flag G: the effective column is one exhaustive match per key; an eleventh key is a compile error here rather than invisible. |
| O-5 | `wrapped` is copied from `kinds.rs:1893`; a third copy would justify promoting it beside `is_error`. |
| O-6 | `resolve_hops`'s clamp note reaches the pane only for a row `validate` never saw (hand SQL); with the editor as the only writer it never fires. |
