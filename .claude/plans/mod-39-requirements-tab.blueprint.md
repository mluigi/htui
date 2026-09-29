# Blueprint: MOD-39, the Requirements tab and item traceability (with CLEAN-6)

**Status**: proposed (2026-09-29). Findings F-1 to F-17 (§0) amend the plan; everything else in the
plan stands.

**Plan**: `.claude/plans/mod-39-requirements-tab.plan.md` (P1-P13, T1-T6, confirmed). **PRD**:
`.claude/prds/mod-39-requirements-tab.prd.md` (D1-D4 all (a), maintainer, 2026-09-29). PRD wins
over this blueprint where they disagree; this blueprint wins over the plan where §0 says so.

**Verified at**: HEAD `c25bb49`, branch `claude/mod-39-requirements-tab-deuoit`. `c25bb49` only adds
the PRD and plan on top of `34b9888`, so every plan citation still holds. Every signature below was
checked by opening the symbol with `grep`/`sed`. **Line numbers are pre-edit.** Gortex and graphify
were not reachable in this session; nothing here comes from a code graph.

**Scope**:
- **Order**: (T1 ∥ T2) → (T3 ∥ T4) → T5 → T6, as the plan. T3 and T4 code against §2 alone.
- **No migration**, no `.sqlx` change, nothing under `htui-store/src`, `htui-core/src/store`,
  `htui-orch/src/{engine,recover}.rs`, `ui/tabs/skills`, `ui/text_area.rs`.
- **Request enums**: `StoreRequest` 75 → 85, `StoreReply` 43 → 47 (T1). No test pins either count;
  HANDOFF "Live coordinates" does (T6).
- **New modules**: `htui::requirements` (T1), `htui::ui::tabs::requirements` (T3, private submodules
  `tree`, `detail`, `forms`), `htui::ui::tabs::backlog::detail::requirements` (T4).
- **Existing snapshots**: only T5 accepts any, and only the diffs §6 lists.

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`, `missing_debug_implementations`
and `unused_qualifications` warn and the gate is `-D warnings`; rustdoc denies broken/private intra-doc
links. A hand-written `Debug` that prints lengths for anything holding user prose. No `std` guard
across an `.await`. Nothing reads the clock in `htui::requirements`; the store stamps. Module docs cite
decisions as "MOD-39 plan P5", "MOD-39 PRD D1", "MOD-39 blueprint F-6". **No intra-doc link from T3 or
T4 to an item the other creates** (each runs `cargo doc` alone); name those in plain backticks.
Implementers stage their own paths only (never `-A`, never `stash`); every commit compiles.

---

## 0. Plan deviations

| # | Sev | Plan says | Code says | Fix (what the implementer builds) |
|---|---|---|---|---|
| F-1 | Blocker | P11/T3: forms end on `Enter` ("body/rationale/Enter mints"); a `Stale` shows `CHANGED_ELSEWHERE` | `TextArea::on_key` inserts `\n` on `Enter` and submits on `Ctrl+S` only (`ui/text_area.rs:190-201`); `CHANGED_ELSEWHERE` says "Enter retries" (`settings/mod.rs:105`) | Every requirement form saves on **`Ctrl+S`** from any field; `Enter` also saves from a single-line last field (area title, amend's deciding key). A stale amend/withdraw shows the tab's own `requirement_changed_elsewhere(head)` (§3.2). T3's tests press `ctrl-s`. |
| F-2 | Major | P13: only `default_for` changes, `close_out_enabled` is "unchanged code" | `EngineError::NotClosable`'s `#[error]` reads "close-out needs `done`, `failed` or `blocked`" (`command.rs:600-603`), pinned with `Status::Open` at `:1837-1844`. After P13 `open` is closable, so the sentence lies | T2 rewrites the text to "item {item} is `{status}`; close-out needs `open`, `blocked`, `failed` or `done` (ANA-11 §4.2)" and moves the pin to `Status::Queued`. Docs at `command.rs:128-131`, `:1184-1194`, `closeout.rs:26`, `item.rs:104-106` are rewritten. `engine.rs:1548-1549` and `:2048-2053` also say "until MOD-39" but are MOD-40 ground: **not edited**, listed in the T6 write-up as a follow-up. |
| F-3 | Major | T5 accepts "only the top strip or the detail strip" | `backlog__runs_closeout_warn.snap` draws the Warn footer `y continue · n cancel`; P13's `←/→` hint changes that line | T5's accept list names this one file for its hint line too (§6). No other T2 change reaches an existing snapshot (the Typed prompt text is unchanged). |
| F-4 | Major | T5: "only `tests/connection.rs:2055` presses `3`"; `tab` presses "unverified" | Three more non-snapshot assertions break on D2: `tests/skills.rs:933-941` and `tests/templates.rs:909-913` assert the literal strip ` 1 Backlog  2 Skills  3 Settings  4 Chat`; `tests/skills.rs:919-923` asserts `Tab` from Skills lands on `"settings"` | T5 edits all four (§6): the strip literal becomes ` 1 Backlog  2 Skills  3 Requirements  4 Settings  5 Chat`, `Tab` lands on `"requirements"`, `connection.rs:2055` presses `4`. |
| F-5 | Major | T4 changes only new tests | `tests/backlog.rs:224-249` `h_and_l_cycle_the_sub_tabs_both_ways` asserts `h` from Body wraps to Prompt (`"digest"`) | T4 updates it: `h` from Body lands on Reqs (FEAT-1 shows `R-STO-1`), a second `h` on Prompt. Same file T4 owns. |
| F-6 | Minor | P4: snapshot carries project `slug`/`name` via `ReadStore::project` | `Ctx::projects` already carries slug and name in scope order (`app/state.rs:76-77`), which is what the Backlog headers use; and no view knows whether the backend is writable, so P10's "write keys greyed offline" has no signal | Snapshot drops slug/name and gains `writable: bool` (`backend.writer().is_some()`); `ItemCitations` carries it too. The worker reads `project()` only to name a project in a refusal. |
| F-7 | Minor | P2: `MintRequirement { scope, area, .. }` | The gate needs the project, and an area id alone costs a scan of every scope project to find it | `MintRequirement` also carries `project`; the worker checks the area is one of `requirement_areas(project)`. |
| F-8 | Minor | P2: variants `Cite`, `Uncite`, `Reconfirm` | 85 variants in one enum; bare verbs read as generic | `CiteRequirement`, `UnciteRequirement`, `ReconfirmCitation` (names `cite_requirement`, `uncite_requirement`, `reconfirm_citation`). |
| F-9 | Major | P5: "the first gated write calls `set_requirement_spec` before its own write" | A write refused *after* the spec is claimed (bad area code, blank body, unknown deciding key) would leave a spec owned by whoever tried | Every input check runs **before** the gate; the gate is the last step before the write (§2.6). |
| F-10 | Minor | P5: `maintainer` from `this_user` | `CacheStore::this_user` is `NotFound` on a mirror that never synced this OS user (`backend.rs:170-174`); a `?` would fail the whole offline read | The snapshot uses `this_user().await.ok()`; an unknown user is maintainer only of a project with no spec. Writes still `?` it. |
| F-11 | Minor | P6: `text: Some(key)` then exact match | `ItemFilter.text` is a substring of key **or title** (`item.rs:224-227`): `ANA-1` also matches `ANA-10` and any title containing it | Match `summary.key == key.trim().to_ascii_uppercase()`; item keys are ASCII upper (`ItemKind::prefix_is_valid`). |
| F-12 | Minor | P2/P5: revisions carry "author" | `RequirementPatch.box_id` and `withdraw_requirement(.., box_id)` want this box | Worker passes `backend.box_info().await?.map(|b| b.box_id)`. |
| F-13 | Minor | P10: `/` filters "so a keystroke never reads the store" | Re-selecting on each keystroke would send `RequirementDetail` per key | While the filter field is open the cursor is not moved; a filtered-out selection is drawn with no highlight. `Enter` closes the field and re-selects (one read). |
| F-14 | Minor | T1 test: "offline detail keeps revisions `None`" | `testkit::seed_mirror` (`htui-store/src/testkit.rs:385`) seeds no requirement table, so an offline `RequirementDetail` has no row to answer | T1's offline tests cover `writable: false` and the write refusals; the `None` branch is a T3 unit test on the detail renderer (PRD metric already says "unit test on the `None` branch"). |
| F-15 | Info | PRD Constraints: `close_out_enabled` returns "`default_for`, else `withdrawn`" | Plan P13 changes `default_for(Open)` instead; `default_for` has no other caller (`command.rs:1203` and tests) | Plan's route (P13) stands: one arm in `item.rs`, no logic change in `command.rs`. Equivalent behaviour. |
| F-16 | Info | P3 replies re-read on write | The freshness gate is per `(origin, request discriminant)` (`app/state.rs:163`); a write and a read are different kinds, so an older read reply can arrive after a write reply | Views land writes **by content** (§4.5, §5.4), the Skills rule; a read that does not show the write changes nothing but the rows. |
| F-17 | Info | Pins | `StoreRequest` 75, `StoreReply` 43, 96 snapshots (`HANDOFF.md:47-48`) | 85, 47, and 96 + T3's + T4's new files (count at T6). |

---

## 1. Cross-task contract (what T3 and T4 may rely on)

Everything in §2.1-§2.4 is **public and fixed**: names, fields, derive sets, name strings, order.
T3 imports from `crate::requirements` and `crate::store_worker` only. T4 likewise. Neither reads a
`UserId`, and neither calls the store.

| Consumer | Sends | Receives |
|---|---|---|
| T3 tab | `Requirements`, `RequirementDetail`, `CreateRequirementArea`, `MintRequirement`, `AmendRequirement`, `WithdrawRequirement` | `StoreReply::Requirements`, `RequirementsStale`, `RequirementDetail`, `Failed { request ∈ REQUEST_NAMES[0..7] }` |
| T4 sub-tab | `ItemRequirements`, `CiteRequirement`, `UnciteRequirement`, `ReconfirmCitation` | `StoreReply::ItemCitations`, `Failed { request ∈ REQUEST_NAMES[2] ∪ [7..10] }` |

---

## 2. T1: `htui::requirements` and the worker wiring

Files: `crates/htui/src/requirements.rs` (new), `crates/htui/src/store_worker.rs` (variants, `name()`
arms, one `try_serve` arm, `use`), `crates/htui/src/lib.rs` (`pub mod requirements;` between
`qdrant_settings_info` and `run_worker`, alphabetical).

Module doc (to match `templates.rs`/`skills.rs`): the Requirements tab's and the Reqs sub-tab's reads
and writes (MOD-39 plan P1), one read per event never per keystroke, every write re-reads; the
maintainer gate (PRD D1, plan P5, blueprint F-9); the deciding key (plan P6, F-11); the worker fills
the author and the box, the view never holds a `UserId` (`R-NF-3`); the known residue of
`templates.rs` (a re-read that fails after an applied write answers `Failed`).

### 2.1 Types

```rust
use htui_core::model::{
    CitationKind, CoverageRow, ItemCitation, ItemFilter, ItemId, NewRequirement, NewRequirementArea,
    Priority, ProjectId, Requirement, RequirementArea, RequirementAreaId, RequirementFilter,
    RequirementId, RequirementPatch, RequirementRevision, RequirementSpec, RequirementState,
    RequirementUpdate, Scope,
};
use htui_core::store::{CasOutcome, ReadStore as _, Result, StoreError, WriteStore as _, invalid_area_code};
use htui_store::{Backend, DATABASE_UNREACHABLE, Writer};

/// Body or rationale on its way to the store. `StoreRequest` derives `Debug`, and this is user
/// prose, so it prints its length only (`TemplateBody`'s rule).
#[derive(Clone, PartialEq, Eq)]
pub struct RequirementText(String);

impl RequirementText {
    #[must_use] pub fn new(text: impl Into<String>) -> Self;
    #[must_use] pub fn as_str(&self) -> &str;
}
impl core::fmt::Debug for RequirementText { /* debug_struct("RequirementText").field("len", ..) */ }

/// The whole tab in one read (plan P4, blueprint F-6). `PartialEq` only: `Requirement` has no `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub struct RequirementsSnapshot {
    /// One entry per id of `scope.project_ids`, in that order.
    pub projects: Vec<ProjectRequirements>,
    /// `backend.writer().is_some()`: `false` offline, where every write key answers
    /// `DATABASE_UNREACHABLE` without a request.
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRequirements {
    pub project_id: ProjectId,
    /// `requirement_spec(project)`; `None` until the first gated write claims it (PRD D1).
    pub spec: Option<RequirementSpec>,
    /// `requirement_areas(project)`, `(position, code)` order.
    pub areas: Vec<RequirementArea>,
    /// `requirements(project, &RequirementFilter::default())`: every state, `(area_code, number)`.
    pub requirements: Vec<Requirement>,
    /// `spec.map_or(true, |s| Some(s.owner_id) == this_user)` (plan P5, F-10).
    pub maintainer: bool,
}

impl RequirementsSnapshot {
    /// Whether this snapshot answers `scope` (project ids equal, in order): the `in_scope` check of
    /// `skills/library.rs:1708`, so a reply for a scope already left is ignored.
    #[must_use] pub fn is_for(&self, scope: &Scope) -> bool;
    #[must_use] pub fn project(&self, id: ProjectId) -> Option<&ProjectRequirements>;
    #[must_use] pub fn requirement(&self, id: RequirementId) -> Option<&Requirement>;
    #[must_use] pub fn area(&self, id: RequirementAreaId) -> Option<&RequirementArea>;
}

impl ProjectRequirements {
    /// The area's requirements, snapshot order.
    pub fn in_area(&self, area: RequirementAreaId) -> impl Iterator<Item = &Requirement> + '_;
}

/// Plan P4's client-side filter: `needle` (trimmed; empty matches everything) is a
/// case-insensitive (`to_lowercase`) substring of `key` or `body`.
#[must_use]
pub fn matches_filter(requirement: &Requirement, needle: &str) -> bool;

/// One requirement for the detail pane (plan P7).
#[derive(Debug, Clone, PartialEq)]
pub struct RequirementDetail {
    /// `requirement(id)`, as it is now.
    pub requirement: Requirement,
    /// `requirement_coverage(id)`, the store's order.
    pub coverage: Vec<CoverageRow>,
    /// `requirement_revisions(id)` in version order, each with its deciding key; `None` offline
    /// (the mirror holds no revisions), drawn as "revisions need the database".
    pub revisions: Option<Vec<RevisionRow>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevisionRow {
    pub revision: RequirementRevision,
    /// `item(amended_by_item_id)?.key`; `None` when the revision names no item or the item is gone.
    pub deciding_key: Option<String>,
}

/// One item's citations for the Reqs sub-tab (plan P8).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemCitations {
    pub item: ItemId,
    /// The item's project: where `candidates` come from.
    pub project_id: ProjectId,
    /// `item_requirements(item)`: live citations, `suspect` derived, store order.
    pub citations: Vec<ItemCitation>,
    /// `requirements(project, { states: Some(vec![Active]), .. })`: what `c` can cite.
    pub candidates: Vec<Requirement>,
    /// As [`RequirementsSnapshot::writable`].
    pub writable: bool,
}
```

### 2.2 Names, sentences, entry points

```rust
/// The ten request names, in `StoreRequest` order; `request_names_match_the_name_arms` pins them.
pub const REQUEST_NAMES: [&str; 10] = [
    "requirements",            // 0 read (tab)
    "requirement_detail",      // 1 read (tab)
    "item_requirements",       // 2 read (sub-tab)
    "create_requirement_area", // 3 tab write, gated
    "mint_requirement",        // 4 tab write, gated
    "amend_requirement",       // 5 tab write, gated
    "withdraw_requirement",    // 6 tab write, gated
    "cite_requirement",        // 7 sub-tab write
    "uncite_requirement",      // 8 sub-tab write
    "reconfirm_citation",      // 9 sub-tab write
];
pub const READ_NAME: &str = REQUEST_NAMES[0];
pub const DETAIL_NAME: &str = REQUEST_NAMES[1];
pub const CITATIONS_NAME: &str = REQUEST_NAMES[2];
/// `REQUEST_NAMES[3..7]`.
#[must_use] pub fn is_tab_write(name: &str) -> bool;
/// `REQUEST_NAMES[7..10]`.
#[must_use] pub fn is_citation_write(name: &str) -> bool;

/// A mint or amend whose body is blank after `trim`.
pub const BLANK_BODY: &str = "a requirement needs a body";
/// An area whose title is blank after `trim`.
pub const BLANK_AREA_TITLE: &str = "an area needs a title";
/// An amend or withdraw whose deciding key is blank after `trim`.
pub const DECIDING_KEY_NEEDED: &str = "an amend or a withdraw names its deciding item by key";

/// PRD D1: `project` is the slug. The view says the same sentence before sending.
#[must_use] pub fn not_the_maintainer(project: &str) -> String;
// "only the owner of {project}'s requirements can add areas or create, amend or withdraw requirements"

/// Plan P6: no item of that key in the requirement's project.
#[must_use] pub fn no_deciding_item(key: &str, project: &str) -> String;
// "no item {key} in {project}"

/// Plan P9: `amends`/`withdraws` record a decision and stay.
#[must_use] pub fn decision_citation_stays(kind: CitationKind) -> String;
// "a `{kind}` citation records a decision; it is not uncited"

pub async fn snapshot(backend: &Backend, scope: &Scope) -> Result<RequirementsSnapshot>;
pub async fn detail(backend: &Backend, id: RequirementId) -> Result<RequirementDetail>;
pub async fn citations(backend: &Backend, item: ItemId) -> Result<ItemCitations>;
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
```

### 2.3 `StoreRequest` variants (appended after `RunActions(ItemId)`, in this order)

```rust
/// MOD-39 plan P2: the scope's specs, areas and requirements, answered with
/// [`StoreReply::Requirements`].
Requirements(Scope),
/// One requirement with its coverage and revisions (plan P7).
RequirementDetail(RequirementId),
/// One item's citations and what it could cite (plan P8), answered with
/// [`StoreReply::ItemCitations`].
ItemRequirements(ItemId),
/// `create_requirement_area`, gated (PRD D1). The worker picks `position` (last + 1).
CreateRequirementArea { scope: Scope, project: ProjectId, code: String, title: String },
/// `mint_requirement`, gated. The worker mints the id and fills `created_by` and `box_id`.
MintRequirement {
    scope: Scope, project: ProjectId, area: RequirementAreaId,
    body: RequirementText, rationale: RequirementText, priority: Priority,
},
/// `amend_requirement` at `expected_version`, gated; `deciding` is the typed item key (plan P6).
/// [`StoreReply::RequirementsStale`] on `Diverged`.
AmendRequirement {
    scope: Scope, id: RequirementId, expected_version: i32,
    body: RequirementText, rationale: RequirementText, priority: Priority, deciding: String,
},
/// `withdraw_requirement` at `expected_version`, gated; `deciding` as above.
WithdrawRequirement { scope: Scope, id: RequirementId, expected_version: i32, deciding: String },
/// `cite(item, requirement, kind, None)`: a human citation. Not gated (plan P5).
CiteRequirement { item: ItemId, requirement: RequirementId, kind: CitationKind },
/// `uncite`; `amends`/`withdraws` refused (plan P9).
UnciteRequirement { item: ItemId, requirement: RequirementId, kind: CitationKind },
/// `reconfirm`: re-stamp at the current version.
ReconfirmCitation { item: ItemId, requirement: RequirementId, kind: CitationKind },
```

Every field carries a one-line `///` doc (workspace `missing_docs`). `name()` arms go after
`Self::RunActions(_) => "run_actions",` under the comment
`// The ten of `requirements::REQUEST_NAMES`, in that order (MOD-39 plan P2).`, as literals
(`name` stays `const fn`).

### 2.4 `StoreReply` variants (inserted after `BoxesStale`, before `Failed`)

```rust
/// The scope's requirements, freshly read: the answer to [`StoreRequest::Requirements`] and to
/// every tab write that applied (MOD-39 plan P3).
Requirements(Box<RequirementsSnapshot>),
/// An amend or withdraw missed its version (plan P3): the snapshot as it is now. The form keeps
/// its text and retries only by hand.
RequirementsStale(Box<RequirementsSnapshot>),
/// Answer to [`StoreRequest::RequirementDetail`].
RequirementDetail(Box<RequirementDetail>),
/// Answer to [`StoreRequest::ItemRequirements`] and to every citation write that applied.
ItemCitations(Box<ItemCitations>),
```

`use crate::requirements::{self, ItemCitations, RequirementDetail, RequirementText, RequirementsSnapshot};`
in `store_worker.rs`.

### 2.5 `try_serve` arm (after the box arm, before `StoreState`)

```rust
// The ten requirement requests, or-ed for the reason the arms above are: a guard does not count
// towards exhaustivity in a wildcard-free `match` (MOD-15 M3 plan F-12, MOD-39 plan P1).
StoreRequest::Requirements(..)
| StoreRequest::RequirementDetail(..)
| StoreRequest::ItemRequirements(..)
| StoreRequest::CreateRequirementArea { .. }
| StoreRequest::MintRequirement { .. }
| StoreRequest::AmendRequirement { .. }
| StoreRequest::WithdrawRequirement { .. }
| StoreRequest::CiteRequirement { .. }
| StoreRequest::UnciteRequirement { .. }
| StoreRequest::ReconfirmCitation { .. } => requirements::serve(backend, request).await?,
```

Nothing else in `store_worker.rs` changes (MOD-40's ticker and notice are elsewhere). The `spawn`
loop needs no interception: every arm is a plain `&Backend` call, so an `Unreachable` still drops an
`Online` backend onto the mirror through the `?`.

### 2.6 `serve` control flow

Private helpers:

```rust
/// The writer, or `Unreachable(DATABASE_UNREACHABLE)` (skills.rs' `write_access`).
fn write_access(backend: &Backend) -> Result<Writer>;
/// `project(id)?.map_or(id.to_string(), |p| p.slug)`: for refusal sentences only.
async fn project_label(backend: &Backend, project: ProjectId) -> Result<String>;
/// Plan P6 / F-11. Blank → `Constraint(DECIDING_KEY_NEEDED)`; else `items(scope, ItemFilter {
/// project_ids: Some(vec![project]), text: Some(wanted.clone()), ..Default::default() })` and the
/// one whose `key == wanted` (`wanted = key.trim().to_ascii_uppercase()`); none →
/// `Constraint(no_deciding_item(&wanted, &project_label))`.
async fn deciding_item(backend: &Backend, scope: &Scope, project: ProjectId, key: &str) -> Result<ItemId>;
/// Plan P5 / PRD D1 / F-9, the last step before a gated write:
///   spec = writer.requirement_spec(project)
///   Some(s) if s.owner_id == me          → Ok
///   Some(_)                              → Err(Constraint(not_the_maintainer(label)))
///   None → set_requirement_spec(project, None, me, String::new()):
///          Applied(_)                     → Ok
///          Stale(row) if row.owner_id==me → Ok   (a race we won in another session)
///          Stale(_)                       → Err(Constraint(not_the_maintainer(label)))
async fn gate(backend: &Backend, writer: &Writer, project: ProjectId, me: UserId) -> Result<()>;
/// `Requirements(fresh)` or `RequirementsStale(fresh)`.
async fn answer(backend: &Backend, scope: &Scope, stale: bool) -> Result<StoreReply>;
```

Reads go through `backend` (the `ReadStore` impl, so offline reads the mirror); writes through
`write_access(backend)?`. `me = backend.this_user().await?` and
`box_id = backend.box_info().await?.map(|b| b.box_id)` are read after `write_access`.

| Request | Steps, in order (a failing step returns its error; nothing after it runs) | Reply |
|---|---|---|
| `Requirements(scope)` | `snapshot(backend, scope)`: `me = this_user().ok()`, `writable = writer().is_some()`; per scope project in order: `requirement_spec`, `requirement_areas`, `requirements(p, &default)`, `maintainer` per §2.1 | `Requirements` |
| `RequirementDetail(id)` | `requirement(id)?` → `None` is `NotFound { entity: "requirement", id }`; `requirement_coverage(id)`; `requirement_revisions(id)` mapped: `Some(rows)` → each `RevisionRow { deciding_key: item(amended_by)?.map(|i| i.key) }`, `None` stays `None` | `RequirementDetail` |
| `ItemRequirements(item)` | `item(item)?` → `None` is `NotFound { entity: "item" }`; `item_requirements(item)`; `requirements(project, states [Active])`; `writable` | `ItemCitations` |
| `CreateRequirementArea` | writer; `code = code.trim()`, `RequirementArea::code_is_valid` else `Constraint(invalid_area_code(code))`; blank title → `Constraint(BLANK_AREA_TITLE)`; `me`; `gate`; `position = max(areas.position) + 1` (0 when none); `create_requirement_area(NewRequirementArea { id: new, project, code, title: title.trim(), description: "", position })` | `Requirements` |
| `MintRequirement` | writer; blank body → `BLANK_BODY`; `area ∈ requirement_areas(project)` else `NotFound { entity: "requirement_area", id }`; `me`, `box_id`; `gate`; `mint_requirement(area, NewRequirement { id: new, body, rationale, priority, created_by: me, box_id })` | `Requirements` |
| `AmendRequirement` | writer; blank body → `BLANK_BODY`; `req = requirement(id)?` else `NotFound`; `deciding = deciding_item(scope, req.project_id, deciding)`; `me`, `box_id`; `gate(req.project_id)`; `amend_requirement(id, expected_version, RequirementPatch { body: Some, rationale: Some, priority: Some, author_id: me, box_id, reason: "amended" }, deciding)`: `Updated` → fresh, `Diverged` → stale | `Requirements` / `RequirementsStale` |
| `WithdrawRequirement` | writer; `req`; `deciding`; `me`, `box_id`; `gate`; `withdraw_requirement(id, expected_version, deciding, me, box_id)`: as amend | `Requirements` / `RequirementsStale` |
| `CiteRequirement` | writer; `cite(item, requirement, kind, None)` | `ItemCitations` (re-read `citations(item)`) |
| `UnciteRequirement` | writer; `kind ∈ {Amends, Withdraws}` → `Constraint(decision_citation_stays(kind))`; `uncite(..)` | `ItemCitations` |
| `ReconfirmCitation` | writer; `reconfirm(..)` | `ItemCitations` |
| anything else | `Err(StoreError::Backend(format!("not a requirement request: {}", other.name())))` | |

Store refusals (a withdrawn requirement amended, cited or re-confirmed; `NotFound` of a citation) stay
errors, so the shell answers `Failed` with the store's sentence. Only `Diverged` is stale: a
requirement is never deleted, so `NotFound` is not a race here (unlike `skills.rs`).

### 2.7 T1 tests (in `requirements.rs`, `#[cfg(test)] mod tests`, MemStore demo unless said)

Helpers: `demo()`, `platform_scope(&Backend)` (as `templates.rs`), `read(..)`, and `stranger_first()`:
`fixtures::demo_data()` with an `AppUser` pushed whose `created_at` is `demo_at(0, 0) - 1 day`, so
`MemStore::this_user` (earliest, `mem.rs:220`) is the stranger and `htui`'s owner `ids::USER` is not.

1. `the_read_answers_every_scope_project_in_scope_order`: htui has the spec, areas `ENT`,`STO`, keys
   `R-ENT-1`,`R-ENT-2`,`R-STO-1`, `maintainer`; agy has no spec, nothing, `maintainer`; `writable`.
2. `a_non_owner_reads_maintainer_false_and_every_gated_write_is_refused`: htui `maintainer == false`;
   the four gated writes each answer `Failed { request: <its name>, message ∋ not_the_maintainer("htui") }`
   through `store_worker::serve`; the snapshot after equals the one before.
3. `the_first_gated_write_claims_a_project_without_a_spec`: `CreateRequirementArea` on agy → agy's
   spec owner is `ids::USER` (this user), area present at position 0.
4. `a_refused_input_claims_no_spec`: agy area code `bad` → `Failed` with `invalid_area_code("bad")`,
   agy still has no spec. Same for a blank title.
5. `mint_answers_the_snapshot_with_the_next_key`: `R-ENT-3`, v1, `created_by == ids::USER`, active.
6. `mint_into_an_area_of_another_project_is_refused`: `project: agy, area: AREA_ENT` → `NotFound`.
7. `amend_at_the_head_records_the_deciding_item`: `R-ENT-1` at 2 with deciding `ana-2` → v3; detail
   revision v3 has `deciding_key == Some("ANA-2")`; `ANA-1`'s citation still `suspect`.
8. `amend_at_a_stale_version_answers_requirements_stale_and_writes_nothing`: at 1 → `RequirementsStale`,
   `R-ENT-1` still v2.
9. `an_unknown_or_partial_deciding_key_is_refused_before_any_write`: `FEAT-99` →
   `no_deciding_item("FEAT-99", "htui")`; `ANA` → same shape; blank → `DECIDING_KEY_NEEDED`; version
   unchanged each time.
10. `withdraw_marks_the_requirement_withdrawn_and_cites_the_deciding_item`: `R-STO-1` by `ANA-2`;
    state `withdrawn`, still listed; `ANA-2`'s citations include `withdraws`.
11. `the_detail_of_r_ent_1_lists_its_coverage_and_revisions`: coverage `ANA-1` addresses suspect,
    `ANA-2` amends not suspect; revisions v1 (`None` key), v2 (`Some("ANA-2")`).
12. `item_requirements_of_ana_1_is_suspect_until_reconfirmed`: `suspect` then `ReconfirmCitation` →
    stamp 2, not suspect; `candidates` are the three active htui requirements.
13. `cite_then_uncite_on_feat_1`: cite `R-ENT-2` addresses → listed; uncite → gone.
14. `uncite_of_a_decision_citation_is_refused`: `ANA-2`/`R-ENT-1`/`Amends` →
    `decision_citation_stays(Amends)`, still listed.
15. `requirement_text_debug_prints_lengths_not_text`: a `MintRequirement` `{:?}` has no body text and
    `len: N`.
16. `request_names_match_the_name_arms`: one sample of each variant, `name()` list == `REQUEST_NAMES`;
    `READ_NAME`, `DETAIL_NAME`, `CITATIONS_NAME` are indices 0-2; `is_tab_write`/`is_citation_write`
    partition 3..10.
17. `offline_the_snapshot_is_read_only_and_writes_are_refused`: `Backend::Offline` over a fresh
    `CacheStore` (as `templates.rs`): the read answers with empty projects and `writable == false`;
    each of the seven writes answers `Failed` containing `DATABASE_UNREACHABLE`.
18. `matches_filter_is_case_insensitive_over_key_and_body`.

---

## 3. T2: close-out resolution picker + CLEAN-6

Files: `htui-core/src/model/item.rs`, `htui-orch/src/command.rs`, `htui-orch/src/closeout.rs`,
`htui/src/ui/tabs/backlog/detail/runs.rs`, `htui/tests/runs_pg.rs`. **Not** `engine.rs` (F-2).

### 3.1 Core and orch

- `Resolution::default_for`: `Status::Open | Status::Blocked | Status::Failed => Some(Self::Withdrawn)`.
  Doc: "The resolution the Runs pane's picker starts on (MOD-39 plan P13): `done` starts on `Done`;
  `open`, `blocked` and `failed` on `Withdrawn`; nothing else is closable."
- `item.rs` test `the_default_resolution_is_a_sanctioned_close_out`: the `(Status::Open, None)` row
  becomes `(Status::Open, Some(Resolution::Withdrawn))`.
- `command.rs`: `NotClosable` text per F-2; `close_out_enabled` docs say the Runs pane starts its
  picker on this answer and `Command::CloseOut` sends the picked one; test
  `close_out_needs_no_live_run_and_a_closable_item` adds `(Status::Open, Resolution::Withdrawn)` to
  the table and drops `Status::Open` from the refused list; the Display pin moves to `Status::Queued`
  with the new text; `CloseOut`'s doc at `:128-131` loses "until MOD-39".
- `closeout.rs:26`: "The resolution the confirmation starts on: `close_out_enabled`'s answer. The
  Runs pane may send another that `closes_from(status)`."

### 3.2 Runs pane

- `CloseOutStage::Warn(Preview)` keeps its shape: the picker **mutates `preview.resolution`**, so
  `Typed` already sends the chosen one (`runs.rs:716-719` unchanged).
- New private fns:
  ```rust
  /// ANA-11 §4.2 through `Resolution::closes_from`: `Resolution::ALL` in declaration order,
  /// filtered (MOD-39 plan P13). The store stays the authority.
  fn resolutions(status: Status) -> Vec<Resolution>;
  /// The next (`forward`) or previous legal resolution after `current`, wrapping; the first legal
  /// one when `current` is not legal.
  fn cycle(status: Status, current: Resolution, forward: bool) -> Resolution;
  ```
- `close_out_key`, Warn arm: `Right | Char('l')` → `preview.resolution = cycle(preview.status, preview.resolution, true)`;
  `Left | Char('h')` → `false`; stays in `Warn`. `y`, `n`, `Esc` as today.
- Footer Warn: the title line is unchanged; the hint becomes `"←/→ resolution · y continue · n cancel"`
  (38 columns, fits 43).
- Module doc (CLEAN-6): row `` | `a` / `x` | step | approve (`AnswerGate`) / reject with a typed note | ``;
  row `` | `C` | item | close-out: the counts and a resolution picked with `←`/`→` (legal ones only), a `y`, then the item key typed back (D167, MOD-39 plan P13) | ``.

### 3.3 T2 tests

Unit (`runs.rs` tests; `warned_with(shell, Preview { status, resolution, .. })` exists at `:2517`):
1. `right_on_a_done_preview_walks_all_six_resolutions` (done → concluded → … → duplicate → done).
2. `left_walks_back_and_wraps` (done ← duplicate).
3. `a_blocked_preview_offers_the_four_non_success_resolutions` (starts withdrawn; `l` ×4 visits
   superseded, duplicate, rejected, withdrawn; never done/concluded).
4. `an_open_preview_starts_on_withdrawn` (status `Open`, resolution `Withdrawn`, four legal).
5. `the_chosen_resolution_is_what_close_out_sends` (`l` to `rejected`, `y`, key, `Enter` →
   `Command::CloseOut { resolution: Rejected }`).
6. `the_warn_footer_names_the_picker` (footer contains `←/→ resolution` and `as rejected` after a move).

Orch/core: the two edited tests above. Postgres (`runs_pg.rs`):
7. `an_open_item_closes_as_withdrawn_on_postgres`: `htui` `ANA-2` (open, no run):
   `Command::CloseOut { item, resolution: Withdrawn }` → no status; item `closed`,
   `resolution == Some(Withdrawn)`, one `summary` document.

---

## 4. T3: the Requirements tab

Files: `ui/tabs/requirements/{mod,tree,detail,forms}.rs` (new), `ui/tabs/mod.rs`
(`pub mod requirements;`, `pub use requirements::RequirementsTab;`), `app/mod.rs`
(`app.register_tab(Box::new(RequirementsTab::new()));` between Skills and Settings, and the doc's
"Backlog, Skills and Settings" becomes "Backlog, Skills, Requirements and Settings" with PRD D2),
`tests/requirements.rs`, `tests/requirements_pg.rs`, their new snapshots.

### 4.1 Public surface

```rust
/// The Requirements tab (MOD-39 PRD D2, plan P10).
#[derive(Debug, Default)]
pub struct RequirementsTab { /* §4.2 */ }
impl RequirementsTab {
    pub const ID: TabId = TabId("requirements");
    #[must_use] pub fn new() -> Self;
}
impl Tab for RequirementsTab { /* title "Requirements"; wants_requests = vec![Requirements(scope)] */ }
```

### 4.2 State (in `mod.rs`)

```rust
pub struct RequirementsTab {
    snapshot: Option<RequirementsSnapshot>,
    /// `Some(message)` after a refused `READ_NAME`.
    unavailable: Option<String>,
    folded: Vec<Fold>,
    selected: Option<Row>,
    /// The applied filter (plan P4); empty = none.
    filter: String,
    /// The selected requirement's detail, `None` until its reply; cleared on every move.
    detail: Option<RequirementDetail>,
    /// A refused `DETAIL_NAME`, drawn in the pane.
    detail_error: Option<String>,
    scroll: Scroll,          // crate::ui::tabs::backlog::detail::Scroll
    mode: Mode,
    /// The tab write in flight, by name: one at a time (Skills' rule).
    busy: Option<&'static str>,
    sent: Option<Sent>,
    notice: Option<Notice>,  // Info(String) | Error(String), one line above the hint
}

// tree.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Row { Project(ProjectId), Area(RequirementAreaId), Requirement(RequirementId) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fold { Project(ProjectId), Area(RequirementAreaId) }
/// Visible rows: `ctx.projects` order; under a project its areas, under an area its requirements.
/// With a non-empty filter only matching requirements, and headers that have one.
pub(super) fn rows(snapshot: &RequirementsSnapshot, projects: &[ProjectRef], folded: &[Fold], filter: &str) -> Vec<Row>;
pub(super) fn lines(..) -> Vec<Line<'static>>;
/// `theme.dim` for a withdrawn requirement, `theme.base` otherwise (PRD "withdrawn dimmed").
pub(super) fn requirement_style(requirement: &Requirement, theme: &Theme) -> Style;

// mod.rs
enum Mode {
    Browse,
    /// `/`: the filter as typed; every key updates `filter`, no request (F-13).
    Filter { field: TextField },
    NewArea(AreaForm),
    /// `n` (`FormTarget::Mint`) and `e` (`FormTarget::Amend`).
    Requirement(RequirementForm),
    Withdraw(WithdrawForm),
}

// forms.rs
pub(super) struct AreaForm { project: ProjectId, code: TextField, title: TextField, focus: AreaFocus }
pub(super) enum AreaFocus { Code, Title }
pub(super) enum FormTarget {
    Mint { project: ProjectId, area: RequirementAreaId, code: String },
    Amend { id: RequirementId, key: String, expected_version: i32 },
}
/// Never `Debug`s text (hand-written `Debug`, lengths only).
pub(super) struct RequirementForm {
    target: FormTarget, body: TextArea, rationale: TextArea, priority: Priority,
    deciding: TextField, focus: FormFocus,
}
pub(super) enum FormFocus { Body, Rationale, Priority, Deciding /* Amend only */ }
pub(super) enum WithdrawForm {
    Deciding { id: RequirementId, key: String, expected_version: i32, field: TextField },
    Typed { id: RequirementId, key: String, expected_version: i32, deciding: String, field: TextField },
}
/// What a form key did.
pub(super) enum FormOutcome { Stay, Cancel, Submit }

enum Sent {
    Area { project: ProjectId, code: String },
    Mint { project: ProjectId, area: RequirementAreaId, body: String, known: Vec<RequirementId> },
    Amend { id: RequirementId, expected_version: i32 },
    Withdraw { id: RequirementId },
}
```

### 4.3 Layout

`area` → vertical `[content Min(1), notice Length(1..=2), hint Length(1)]` (Skills' `library.rs`
shape); `content` → horizontal `[tree Percentage(45), detail Percentage(55)]`, both bordered
(`" Requirements (N) "`, `" R-ENT-1 "` / `" New requirement in ENT "` / `" Amend R-ENT-1 (v2) "` /
`" Withdraw R-ENT-1 "` / `" New area in htui "`).

Tree lines: project `▾ htui (3)` (`▸` folded), plus ` · read-only` in `theme.dim` when
`!maintainer`; area `  ▾ ENT Entity model (2)`; requirement `    R-ENT-1  must   <first line of
body>` cut with `…` to the pane, `requirement_style`, cursor row `theme.selected`. A project with no
area: `  no areas` (dim). No snapshot yet: `requirements not read yet`; refused read: the message.

Detail (`detail.rs`, `pub(super) fn lines(detail: &RequirementDetail, width: u16, theme: &Theme) ->
Vec<Line<'static>>`): wrapped body; blank; `rationale:` + wrapped rationale (omitted when empty);
`must · active · v2`; blank; `Coverage` (title style), one row per `CoverageRow`:
`ANA-1    addresses v1  done   —      ! suspect` (key, kind, stamp, status, resolution or `—`,
suspect marker in `theme.error`), or `no item cites it` dim; blank; `Revisions`, one row per
`RevisionRow`: `v2  amended  by ANA-2  09-04` (`STAMP` format), or the constant
`REVISIONS_NEED_THE_DATABASE = "revisions need the database"` (dim) when `revisions` is `None`.
Project row selected: owner line (`you own these requirements` / `read-only: another user owns
these requirements` / `no spec yet: the first write claims it`). Area row: code, title, count.

Hint row (Browse): `j/k move  Enter fold  / filter  a area  n new  e amend  W withdraw  r reload`;
the four write words are `theme.dim` when the selected row's project is not `maintainer` or the
snapshot is not `writable`, `theme.base` otherwise. Forms: `Tab field  Ctrl+S save  Esc cancel`
(`m/l priority` added on the priority field); withdraw: `Enter next  Esc cancel`.

### 4.4 Keys

Browse (the tab passes `CONTROL`/`ALT` chords and `Tab`/`BackTab` to the shell):

| Key | On | Does |
|---|---|---|
| `j`/`Down`, `k`/`Up` | any | move; landing on a requirement sends `RequirementDetail(id)` |
| `g`/`Home`, `G`/`End` | any | first / last row |
| `Enter` | project / area | fold / unfold |
| `J`/`K`, `PgDn`/`PgUp` | any | scroll the detail (`Scroll::on_key`) |
| `/` | any | `Mode::Filter` over the current filter |
| `Esc` | any | clears a non-empty filter (re-select, one read) |
| `a` | any row of a project | `NewArea` for that project |
| `n` | area or requirement | `Requirement(Mint)` in that area; on a project row: notice `select an area first` |
| `e` | active requirement | `Requirement(Amend)` prefilled, `expected_version = version` |
| `W` | active requirement | `Withdraw(Deciding)` |
| `r` | any | `Requirements(scope)` |

A write key checks, in order: `busy` → notice `` `{busy}` is still in flight ``; `!writable` →
`Action::Error(DATABASE_UNREACHABLE)`; `!maintainer` → `Action::Error(not_the_maintainer(slug))`
(slug from `ctx.projects`); a withdrawn requirement for `e`/`W` →
`Action::Error(htui_core::store::requirement_withdrawn(key))`. None of these sends anything.

Forms capture every non-`CONTROL` key (so digits, `q`, `Tab` stay in the form):
- `NewArea`: `Tab`/`BackTab` switch field; `Enter` on code → title; `Enter` on title or `Ctrl+S`
  → validate (`code_is_valid` else notice `invalid_area_code`, blank title → `BLANK_AREA_TITLE`)
  and send `CreateRequirementArea` with the code trimmed; `Esc` cancels.
- `Requirement`: `Tab`/`BackTab` cycle `Body → Rationale → Priority (→ Deciding)`; on `Priority`
  `m` = must, `l` = later, `Space`/`Left`/`Right` toggle; `Ctrl+S` anywhere or `Enter` on
  `Deciding` → validate (blank body → `BLANK_BODY`; amend with blank deciding →
  `DECIDING_KEY_NEEDED`) and send `MintRequirement` / `AmendRequirement`; `Esc` (a `Cancel` from any
  field) cancels.
- `Withdraw`: `Deciding` `Enter` → blank refuses with `DECIDING_KEY_NEEDED`, else `Typed`;
  `Typed` `Enter` → text == key sends `WithdrawRequirement`, else notice
  `NOT_THE_REQUIREMENT_KEY = "that is not the requirement's key"` and the field clears; `Esc` cancels.
- While `busy`: the form stays drawn, notice `saving…`, every key but `CONTROL` chords is swallowed.
- `Filter`: `TextField` keys update `filter`; `Enter` → `Browse` and re-select; `Esc` → clear and
  `Browse`.

### 4.5 Replies

| Reply | Does |
|---|---|
| `Requirements(s)` if `s.is_for(ctx.scope)` | replace snapshot, clear `unavailable`; if `busy` and the write **shows** (§ below) → close the form, `busy = None`, notice (`added area ABC`, `minted R-ENT-3`, `amended R-ENT-1 to v3`, `withdrew R-ENT-1`), select the minted row, send `RequirementDetail` for the selection; else re-select (keep a row that still exists, else the first requirement) |
| `RequirementsStale(s)` | replace snapshot; if `busy`: `busy = None`, keep the form, set its `expected_version` to the head's, notice `requirement_changed_elsewhere(head)` = `"changed elsewhere since you opened it — now v{head}; your text is kept and Ctrl+S saves over it"` |
| `RequirementDetail(d)` if `selected == Row::Requirement(d.requirement.id)` | `detail = Some`, `detail_error = None` |
| `Failed { request == READ_NAME }` | `unavailable = Some(message)` |
| `Failed { request == DETAIL_NAME }` | `detail_error = Some(message)` |
| `Failed { is_tab_write(request) }` | `busy = None`, `sent = None`, notice `Error(message)`; the form stays |

"Shows": `Area` → the project has an area with that code; `Mint` → the area holds a requirement not
in `known` whose body == sent body; `Amend` → `version > expected_version`; `Withdraw` → state
`Withdrawn` (F-16). `on_scope_change`: drop everything, `Mode::Browse`, `busy = None`.

### 4.6 T3 tests

Unit (in the tab modules): `withdrawn_rows_render_dim`, `revisions_none_says_they_need_the_database`
(F-14), `the_filter_keeps_matching_rows_and_their_headers`, `a_keystroke_in_the_filter_sends_no_request`,
`write_keys_are_refused_offline_without_a_request`, `a_non_maintainer_write_key_says_so_and_sends_nothing`,
`captures_input_follows_the_mode` (via `on_key` returning `Consumed` for `q` in a form).

Harness (`tests/requirements.rs`, `#![cfg(feature = "testkit")]`; helper `open_platform_over(store)`:
`Harness::over`, `register_all`, `settle`, `w`, `j`, `enter`, `settle`, `3`, `settle`):
1. `the_tab_is_third_in_the_strip` (strip literal `3 Requirements  4 Settings  5 Chat`).
2. `the_tab_shows_the_tree_and_r_ent_1_detail` → snapshot `requirements_tree` (coverage with the
   suspect `ANA-1`, `ANA-2` amends; revisions v1, v2 by ANA-2).
3. `slash_filters_over_key_and_body` → `/`, `STO`, `enter` → snapshot `requirements_filter`.
4. `n_mints_the_next_key_in_the_area` → `ctrl-s` → snapshot `requirements_minted` (`R-ENT-3`).
5. `e_amends_with_a_deciding_item` → deciding `ANA-2`, `enter` → snapshot `requirements_amended`
   (v3; `ANA-1` still suspect).
6. `shift_w_withdraws_after_the_key_is_typed_back` (`R-ENT-2`, deciding `ANA-2`) → snapshot `requirements_withdrawn` (row dim,
   still listed; the dim is asserted on the buffer style in a unit test, not the text snapshot).
7. `a_creates_an_area` → snapshot `requirements_new_area`.
8. `a_non_maintainer_is_answered_on_the_status_line` (stranger-first store) → snapshot
   `requirements_read_only`; the store is unchanged.
9. `a_stale_amend_keeps_the_form` (amend `R-ENT-1` directly through a store clone while the form is
   open; `ctrl-s` → the notice, the text kept, a second `ctrl-s` lands v4).

Postgres (`tests/requirements_pg.rs`, the `skills_pg.rs` skip/panic rule; `store_worker::serve` over
`Backend::Online { pg: db.store, cache }`, no harness needed):
10. `mint_amend_and_a_stale_amend_on_postgres`.
11. `the_first_write_claims_agy_and_a_non_owner_is_refused_on_postgres` (a second `app_user` inserted
    through `db.pool()`, `set_requirement_spec(htui, Some(1), stranger, ..)`, then a gated write
    answers `not_the_maintainer("htui")`).

---

## 5. T4: the Reqs sub-tab

Files: `ui/tabs/backlog/detail/requirements.rs` (new), `detail/mod.rs` (`pub mod requirements;`,
`pub use requirements::ReqsTab;`, module doc "seven sub-tabs"), `detail/documents.rs` (`title()`
returns `"Docs"`; `ID` stays `"documents"`), `backlog/mod.rs` (register `ReqsTab` after `PromptTab`;
`go` sends `StoreRequest::ItemRequirements(id)` after `Notes`, doc "seven reads"), `tests/backlog.rs`.

### 5.1 Public surface and state

```rust
/// The Reqs sub-tab (MOD-39 PRD D3, D4; plan P12).
#[derive(Debug, Default)]
pub struct ReqsTab {
    item: Option<ItemId>,
    citations: Option<ItemCitations>,
    cursor: usize,
    mode: Mode,
    /// The citation write in flight, by name.
    busy: Option<&'static str>,
    scroll: Scroll,
}
impl ReqsTab {
    pub const ID: DetailId = DetailId("reqs");
    #[must_use] pub fn new() -> Self;
}
// title() == "Reqs"; captures_input() == !matches!(mode, Mode::Browse)

enum Mode {
    Browse,
    /// `c`: a cursor over `candidates`.
    Pick { cursor: usize },
    /// A candidate picked: `a` addresses, `v` reserves.
    Kind { requirement: RequirementId },
    /// `u`: `y` uncites.
    ConfirmUncite { requirement: RequirementId, kind: CitationKind },
}
```

Sentences (pub consts): `NOT_SUSPECT = "this citation is current; nothing to re-confirm"`,
`NO_CITATION = "no citation is under the cursor"`,
`NO_CANDIDATES = "the project has no active requirement to cite"`.

### 5.2 Render

Two lines per citation: `▸ R-ENT-1 addresses v1 ! suspect` (`▸`/space cursor; key `theme.accent`;
`! suspect` `theme.error`; the requirement's line `theme.dim` if withdrawn), then
`  <first line of body>` dim, cut to the width. Empty: `No requirements cited.` (via `message`).
No item: `No item selected.`. Footer by mode: Browse `J/K move · r reconfirm · c cite · u uncite`
(42 columns); Pick: the candidates `▸ R-ENT-2 later An item may carry…` one per line, then
`j/k move · Enter pick · Esc back`; Kind: `cite R-ENT-2 as: a addresses · v reserves · Esc back`;
ConfirmUncite: `uncite R-ENT-2 (addresses)? y uncite · n keep`.

### 5.3 Keys

| Mode | Key | Does |
|---|---|---|
| Browse | `J`/`K` | cursor |
| Browse | `r` | suspect → `ReconfirmCitation`; else `Action::Error(NOT_SUSPECT)` |
| Browse | `c` | no candidates → `NO_CANDIDATES`; else `Pick { 0 }` |
| Browse | `u` | `Amends`/`Withdraws` → `Action::Error(decision_citation_stays(kind))`; else `ConfirmUncite` |
| Browse | other | `Scroll::on_key` (PgUp/PgDn), else `Pass` |
| Pick | `j`/`k`/`J`/`K`/`Down`/`Up`, `Enter`, `Esc` | move, → `Kind`, → `Browse` |
| Kind | `a`, `v`, `Esc` | `CiteRequirement { kind: Addresses \| Reserves }` → `Browse`; → `Pick` |
| ConfirmUncite | `y`, `n`/`Esc` | `UnciteRequirement` → `Browse`; → `Browse` |

Every write checks `busy` (Action::Error `` `{busy}` is still in flight ``) then `writable`
(`Action::Error(DATABASE_UNREACHABLE)`) before sending. Capturing modes pass `CONTROL` chords and
consume the rest (Runs' rule).

### 5.4 Replies

`ItemCitations(c)` with `c.item == self.item` → store, clamp `cursor`, `busy = None`.
`Failed { request == CITATIONS_NAME }` → the pane shows the message.
`Failed { is_citation_write(request) }` → `busy = None` (the status line already has it).
`on_item_change` → drop `citations`, `cursor = 0`, `Mode::Browse`, `busy = None`.

### 5.5 T4 tests

Unit (`requirements.rs`): `u_is_refused_on_a_decision_citation`, `r_on_a_current_citation_says_so`,
`a_reply_for_another_item_is_ignored`, `captures_input_follows_the_mode`, `writes_are_refused_offline`.

`tests/backlog.rs` (`backlog()` helper; Reqs is `sub_tab(&mut h, 6)`; the arrival row is `ANA-1`):
1. `the_reqs_sub_tab_shows_ana_1_s_suspect_citation` → snapshot `detail_reqs`.
2. `r_reconfirms_the_suspect_citation` → `R-ENT-1 addresses v2`, no marker → snapshot
   `detail_reqs_reconfirmed`.
3. `c_cites_a_requirement_of_the_item_s_project` (FEAT-1, pick `R-ENT-2`, `a`) → snapshot
   `detail_reqs_cite_picker` taken at the Pick stage; the row is present after.
4. `u_then_y_uncites` (the cited row of 3 goes away).
5. `u_on_an_amends_citation_is_answered_on_the_status_line` (ANA-2).
6. `the_reqs_sub_tab_says_so_when_it_has_nothing` (CLEAN-1) → snapshot `empty_reqs`.
7. `h_and_l_cycle_the_sub_tabs_both_ways` updated (F-5).
8. `the_detail_strip_fits_the_detail_pane` (unchanged; now measures 40 columns with seven titles).

---

## 6. T5: snapshot, test and README sweep

Run `cargo test -p htui --features testkit`. Accept (`cargo insta review`, one file at a time) an
**existing** snapshot only when its whole diff is one of:
- the top strip ` 1 Backlog  2 Skills  3 Settings  4 Chat` → ` 1 Backlog  2 Skills  3 Requirements
  4 Settings  5 Chat` (20 files at `c25bb49`);
- the detail strip ` Body Runs Graph Documents Notes Prompt ` → ` Body Runs Graph Docs Notes Prompt
  Reqs ` (28 files mention `Documents`; check each is the strip);
- `backlog__runs_closeout_warn.snap`: its strip and the hint line → `←/→ resolution · y continue ·
  n cancel` (F-3).

Any other diff is a bug for its task's owner. Non-snapshot edits (F-4): `tests/connection.rs:2055`
`"3"` → `"4"`; `tests/skills.rs:938` and `tests/templates.rs:911` strip literals;
`tests/skills.rs:919-923` expects `"requirements"`. README.md: the tab tour and key list gain
Requirements at `3` (Settings `4`, Chat `5`), the Reqs sub-tab and the close-out picker; no work-item
ids.

## 7. T6 notes

HANDOFF pins: `StoreRequest` 85, `StoreReply` 47, snapshot count, tab strip. Write-up
(`docs/decisions/mod/mod-39.md`) lists F-1..F-17 and the stale `engine.rs:1548-1549`, `:2048-2053`
docs (F-2) as a follow-up for whoever next owns `engine.rs`. CLEAN-6 closes with MOD-39.
