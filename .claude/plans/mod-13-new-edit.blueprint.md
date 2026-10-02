# Blueprint: MOD-13 milestone 2, Backlog `new` and `edit`

This turns the approved plan `.claude/plans/mod-13-new-edit.plan.md` (D1–D11, A1–A16) into compile-ready
design. It reopens none of those decisions. Every anchor was checked with Gortex on `hr/MOD-13` at `d712a11`.
Where this file and the plan disagree on a detail, this file wins (errata §0).

## 0. Errata (plan vs. tree)

| # | Plan claim | What the tree shows | Consequence |
|---|---|---|---|
| E1 | T1 validate `cargo test -p htui-core --all-features --lib item_spec excerpt skill_glob`; T3 validate `… --lib item_writes store_worker` | `cargo test` takes **one** `[TESTNAME]`; a second positional is `error: unexpected argument 'excerpt' found` (checked, `cargo test --help`) | Filters go after `--`: `cargo test -p htui-core --all-features --lib -- item_spec excerpt skill_glob` and `cargo test -p htui --all-features --lib -- item_writes store_worker` |
| E2 | A13: add a `backlog_over_agent(store)` helper to `tests/backlog.rs` | `backlog_over(store: MemStore)` already exists at `tests/backlog.rs:1202` and already adds the agent runtime. `type_text` (`:598`) and `keys` (`:1216`) exist too | No new helper. T5 reuses `backlog_over`, `type_text`, `keys`, `detail_pane` |
| E3 | T5: "`e`, a changed title, Ctrl+S: Body shows `v2`" | `BodyTab::render` writes `"{tags} · priority {p} · version {v}"` (`detail/body.rs:93-99`); no `v2` text exists | Assert `version 2` and the new title in the detail pane |
| E4 | A16: `N` "both with and without SHIFT" | `KeyChord::new` strips `SHIFT` from every `Char` (`keymap.rs:37-40`), so `Harness::key("N")` and `key("shift-N")` both arrive as `Char('N')` with no modifier | The SHIFT case is a **unit** test only, with `KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT)` (§4 test 1) |
| E5 | C1/C2: `overlap::resolve`, `overlap.rs:51, 55, 59-60`; "other callers … `model/overlap.rs:227`" | `resolve` is `crates/htui-orch/src/overlap.rs:37` (parse `:51`, bare-without-primary drop `:55`, `UnknownTouchedRepo` `:59-60`). `crates/htui-core/src/model/overlap.rs:227` is a **test** (`prefixes_come_from_the_excerpt_path_prefix`) | Paths only. The rule the validator mirrors is the orch one: a qualified entry resolves by name even with no primary; only a bare one needs a primary |
| E6 | A9: "`body` and `touched_paths` travel in a newtype … (`RequirementText` pattern)" in `item_writes.rs` | The validator's typed output *is* the wire payload. Per-field htui newtypes would need a second seven-field mirror struct and a conversion each way | Same rule, one place: the two core wire structs `ItemSpec` and `SpecChanges` (`item_spec.rs`) hand-write `Debug` and print `body` and `touched_paths` as lengths only. `StoreRequest` still never carries `NewItem` or `ItemPatch`. The T3 `Debug` test is unchanged |
| E7 | D8: "On a new item, the project is a picker" | The form's kinds, graphs and repos are per project (D3); one `ItemFormContext` cannot serve two projects | Moving the picker emits `ItemFormOutcome::Reload(project)`; the tab sends `ItemForm { project, item: None }`; the reply re-targets the open form (kind and graph reset, text kept). Keys are swallowed meanwhile (`Busy::Reloading`) |
| E8 | A7 closes the form on a scope change | `App::forget` is called for overlays only (`app/update.rs:115`); a tab's staleness entry survives a scope change, so a late `ItemForm` reply still reaches the tab | The tab keeps an `opening: Option<Opening>` token; an `ItemForm` reply opens a form only when it answers that token. `on_scope_change` clears it |
| E9 | T3: "`MintItem` lands `KEY-n+1` with revision 1"; "`EditItem` … lands `version + 1` with reason `edited`" | No public read returns `item_revision` rows (`ReadStore` has none; `MemStore` has no accessor). A revision appears only as `UpdateOutcome::Diverged.ancestor` | Assert `version == 1` on the mint (the revision itself is pinned by conformance `mint_writes_revision_v1`). Pin `reason = "edited"` through the `ancestor` of a later, deliberately stale edit (§3 test 7) |
| E10 | (silent) | `BacklogTab` derives `Debug` (`backlog/mod.rs:53`) and the form holds an `Item` whose `Debug` prints `body` | `ItemForm` hand-writes `Debug` (lengths only), the `RequirementForm` precedent (`requirements/forms.rs:122-125`) |
| E11 | (silent) | Offline harness: once item rows are seeded, the first `Items` reply selects `htui ANA-1` and sends the seven detail reads, so the status line is **not** clean before `N`. The milestone-1 offline test's `status == None` precheck (`tests/backlog.rs:1555`) cannot be copied | The offline test sets the scope to Platform explicitly, then asserts the **exact** status after `N` and after `e` (§5 test 5) |
| E12 | §1c: the body is returned "as given"; the check order is title, kind, tags, graph, paths; D4 refuses a NUL in `touched_paths` only | Postgres `text` cannot hold U+0000 (`22021`). A NUL in the title or body mints on `MemStore` but fails on `PgStore` as a `Backend` error, which §10.1 would hedge as "may have been written". Both stores already refuse such text by rule elsewhere (`has_nul`, `store/traits.rs:1897`) | The validator refuses it by rule (implemented in `6caae9d`): `SpecError::Nul(&'static str)`, `"{0} must not contain a NUL character"` (`has_nul`'s sentence), with the column named `item.title` or `item.body`. `parse_title` refuses it after `BlankTitle`; a private `check_body` refuses it in the body. The order is **title, body, kind, tags, graph, paths**. T3's `refused` maps it to `Constraint` like every other `SpecError`, so `mint_refused` (§10.1) treats it as a refusal, not a hedge. Nothing matches `SpecError` exhaustively in T3/T4 (both use `Display`). Test `a_nul_in_the_title_or_body_is_refused` |

## 1. Shared validator (Task 1, `htui-core`)

### 1a. `crates/htui-core/src/prompt/excerpt.rs`

Factor the qualifier rule out of `PathPrefix::parse` (`:72-76`). Behaviour is unchanged.

```rust
/// ANA-2 §4.7's qualifier rule, `PathPrefix::parse`'s own (`docs/ANA-2.md:1025`): the text before
/// the **first** `:` when it is non-empty and holds no `/`. A qualified entry answers
/// `(Some(repo), glob)`, a bare one `(None, touched)`. Nothing is trimmed: the validator refuses
/// whitespace next to the `:` (MOD-13 A2), and `SkillGlob::parse`'s trimming split is a different rule.
#[must_use]
pub fn split_qualifier(touched: &str) -> (Option<&str>, &str) {
    match touched.split_once(':') {
        Some((repo, rest)) if !repo.is_empty() && !repo.contains('/') => (Some(repo), rest),
        _ => (None, touched),
    }
}
```
`PathPrefix::parse` becomes `let (repo, glob) = match split_qualifier(touched) { (Some(repo), glob) => (repo, glob), (None, glob) => (primary_repo, glob) };`.

New test beside `a_touched_glob_may_name_its_repo` (`:2608`): `split_qualifier_is_the_parse_rule`. Cases: `"web:src/**"` gives `(Some("web"), "src/**")`; `"a/b:c"` gives `(None, "a/b:c")`; `":src"` gives `(None, ":src")`; `"web:"` gives `(Some("web"), "")`; `"web: src"` gives `(Some("web"), " src")`; `"src/**"` gives `(None, "src/**")`. All existing `PathPrefix` tests stay green.

### 1b. `crates/htui-core/src/model/skill_glob.rs`
`fn matcher` (`:227`) becomes `pub(crate) fn matcher`. Nothing else changes.

### 1c. `crates/htui-core/src/model/item_spec.rs` (new)

Imports: `crate::model::{BoxId, Item, ItemId, ItemKind, ItemKindId, ItemPatch, NewItem, ProjectId, Repo, StepGraph, StepGraphId, UserId, canonical_declared_tags, declared_tags_from_text}`, `crate::model::skill_glob::{self, GlobError}` and `crate::prompt::excerpt::split_qualifier`. `thiserror` is already a dependency (`store/error.rs`).

```rust
/// `item_revision.reason` of a form edit (D5).
pub const EDITED: &str = "edited";
/// D5: an edit whose every field equals the item's. Said by the form, refused by the worker.
pub const NOTHING_TO_SAVE: &str = "nothing to save: no field differs from the item";

/// What a spec is checked against: one project's kinds, step graphs and repos.
#[derive(Debug, Clone, Copy)]
pub struct SpecContext<'a> {
    pub kinds: &'a [ItemKind],
    /// The project's graphs. `is_override` rows may be present (the worker passes
    /// `step_graphs()` whole); the check never accepts one as a change (A3).
    pub graphs: &'a [StepGraph],
    pub repos: &'a [Repo],
}

/// Every `version`-covered spec column of a new item, typed and canonical (D4). The wire payload
/// of `StoreRequest::MintItem`.
#[derive(Clone, PartialEq, Eq)]
pub struct ItemSpec {
    pub kind_id: ItemKindId,
    pub title: String,
    pub body: String,
    pub priority: i16,
    pub required_tags: Vec<String>,
    pub touched_paths: Vec<String>,
    /// `None` = the kind's default graph.
    pub step_graph_id: Option<StepGraphId>,
}

/// The columns an edit changes; `None` leaves a column alone (D5, A4). The wire payload of
/// `StoreRequest::EditItem`. `step_graph_id` is `ItemPatch`'s double option: `Some(None)` clears.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SpecChanges {
    pub kind_id: Option<ItemKindId>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub priority: Option<i16>,
    pub required_tags: Option<Vec<String>>,
    pub touched_paths: Option<Vec<String>>,
    pub step_graph_id: Option<Option<StepGraphId>>,
}
```
**Redaction (E6).** `impl core::fmt::Debug for ItemSpec` uses `debug_struct("ItemSpec")` with `kind_id`, `title`, `body_len` (`body.len()`), `priority`, `required_tags`, `touched_paths` (as `touched_paths.len()`) and `step_graph_id`. `SpecChanges` does the same, with `body: self.body.as_ref().map(String::len)` and `touched_paths: self.touched_paths.as_ref().map(Vec::len)`. Each impl gets a doc line citing `store_worker.rs:96-104`.

```rust
/// Why a spec is refused. Every sentence names the field or the entry (D4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    #[error("an item needs a title")]
    BlankTitle,
    #[error("priority `{0}` is not a whole number from -32768 to 32767")]
    Priority(String),
    /// `canonical_declared_tags`' sentence, which names the tag (`box_.rs:298-303`).
    #[error("{0}")]
    Tags(String),
    #[error("that kind is not one of this project's item kinds")]
    Kind(ItemKindId),
    #[error("that step graph is not one of this project's graphs")]
    Graph(StepGraphId),
    /// E12: a NUL in the title or body; the column is `item.title` or `item.body`.
    #[error("{0} must not contain a NUL character")]
    Nul(&'static str),
    #[error("touched path `{entry}` {why}")]
    Path { entry: String, why: String },
}
```

**Text front (form only):**
```rust
pub fn parse_title(text: &str) -> Result<String, SpecError>;   // trim; empty -> BlankTitle; NUL -> Nul("item.title") (E12)
pub fn parse_priority(text: &str) -> Result<i16, SpecError>;   // text.trim().parse::<i16>(); Err -> Priority(trimmed)
pub fn parse_tags(text: &str) -> Result<Vec<String>, SpecError>; // declared_tags_from_text, Err -> Tags
#[must_use] pub fn parse_paths(text: &str) -> Vec<String>;     // lines, trim each, drop blanks, dedup keeping order
#[must_use] pub fn tags_text(tags: &[String]) -> String;       // join(", ")
#[must_use] pub fn paths_text(paths: &[String]) -> String;     // join("\n")
```

**Checks (worker authority, form early feedback):**
```rust
pub fn check_paths(paths: &[String], repos: &[Repo]) -> Result<Vec<String>, SpecError>;
pub fn check_spec(spec: &ItemSpec, ctx: &SpecContext<'_>) -> Result<ItemSpec, SpecError>;
pub fn check_changes(changes: &SpecChanges, ctx: &SpecContext<'_>) -> Result<SpecChanges, SpecError>;
```
Both checks return the **canonical** value: the title trimmed, the tags through `canonical_declared_tags`, the paths through `check_paths`, everything else as given, except that a body holding a NUL is refused (E12). `check_changes` checks only the `Some` fields (A4). The order is title, body, kind, tags, graph, paths (E12), and the first refusal wins. `check_spec` uses the same private per-field functions, so no `expect` is needed.
- kind: `ctx.kinds.iter().any(|k| k.id == id)`, else `Kind(id)`.
- graph: `None` passes; `Some(id)` passes when `ctx.graphs.iter().any(|g| g.id == id && !g.is_override)`, else `Graph(id)`. An unchanged override graph is never in `SpecChanges`, so it is never checked (A4 covers D4's "only as the unchanged value").
- `check_paths`: first canonicalise (trim, drop blanks, dedup keeping order, so the function is idempotent). Then, per entry, the first rule that fires gives the `why`:
  1. contains `'\0'`: `contains a NUL character`
  2. `split_qualifier(entry)` gives `(Some(repo), glob)` with `repo.ends_with(char::is_whitespace) || glob.starts_with(char::is_whitespace)`: ``has whitespace next to its `:` ``
  3. qualified and `glob.is_empty()`: ``has no glob after `{repo}:` ``
  4. qualified and no `repos` row has `name == repo`: ``names `{repo}`, which is not a repo of this project``
  5. bare and no `repos` row `is_primary`: ``is bare and this project has no primary repo; write it as `<repo>:<glob>` ``
  6. `glob.starts_with('/')`: `is absolute; touched paths are repo-relative`
  7. `skill_glob::matcher(glob)` fails: `: {message}`. Here `message` is `GlobError::Invalid`'s own; `UnknownLanguage` cannot occur and maps through `to_string()`.

**Conversions:**
```rust
impl ItemSpec {
    /// The spec as `item` stores it, unchecked (the edit form's base, A4).
    #[must_use] pub fn of(item: &Item) -> Self;
    #[must_use] pub fn into_new_item(self, id: ItemId, project_id: ProjectId, created_by: UserId, box_id: Option<BoxId>) -> NewItem;
}
impl From<ItemSpec> for SpecChanges { /* every field Some; step_graph_id: Some(spec.step_graph_id) */ }
impl SpecChanges {
    /// The fields whose values differ between `base` and `next` (compared as stored values).
    #[must_use] pub fn between(base: &ItemSpec, next: &ItemSpec) -> Self;
    #[must_use] pub fn is_empty(&self) -> bool;       // == Self::default()
    /// `reason = EDITED`.
    #[must_use] pub fn into_patch(self, author_id: UserId, box_id: Option<BoxId>) -> ItemPatch;
}
```
`crate::fixtures` has a private `struct ItemSpec` (`fixtures.rs:823`). Its import list is explicit (`:23-31`), so there is no clash.

### 1d. `crates/htui-core/src/model/mod.rs`
Add `pub mod item_spec;` between `pub mod item;` (`:87`) and `pub mod kind;`. Add `pub use item_spec::{ItemSpec, SpecChanges, SpecContext, SpecError};`. Callers use the functions as `item_spec::check_spec`.

### Task 1 tests (`item_spec.rs` `mod tests`; hand-built rows with `DateTime::UNIX_EPOCH`, never demo data, A15)
Helpers: `fn repo(name: &str, primary: bool) -> Repo`, `fn kind(project) -> ItemKind`, `fn graph(project, is_override) -> StepGraph`, `fn ctx(...) -> SpecContext`.
1. `a_blank_title_is_refused`: `""`, `"   "` and `"\t"` give `BlankTitle`; `"  Fix it "` gives `"Fix it"`.
2. `priority_parses_as_an_i16`: `"x"` gives `Priority("x")`; `"40000"` gives `Priority("40000")`; `"-3"` gives `-3`; `" 7 "` gives `7`.
3. `tags_are_canonical_and_a_bad_one_is_named`: `"Rust"` gives `Tags(s)` with `s` containing `` `Rust` ``; `"rust, docker,rust"` gives `["docker","rust"]`; `""` gives `[]`.
4. `a_bare_path_is_kept_with_a_primary_repo`: `src/**` with `[htui*]`.
5. `a_bare_path_is_refused_without_a_primary_repo`: the `why` is rule 5.
6. `a_qualified_path_naming_a_project_repo_is_kept`: `web:src/**` with `[web]`, which has **no** primary (E5).
7. `an_unknown_repo_is_refused_by_name`: `nope:src`; the message contains `` `nope` ``.
8. `a_qualifier_with_no_glob_is_refused`: `web:`.
9. `whitespace_next_to_the_colon_is_refused`: `web: src` and `web :src`.
10. `an_absolute_path_is_refused`: `/abs` with a primary repo.
11. `a_glob_that_does_not_compile_is_refused`: `src/[` with a primary repo.
12. `a_colon_after_a_slash_is_bare`: `a/b:c` with a primary repo is kept verbatim.
13. `a_nul_is_refused`.
14. `blank_and_duplicate_paths_are_dropped_in_order`: `parse_paths("src/**\n\n  docs/*  \nsrc/**\n") == ["src/**","docs/*"]`, and `check_paths` of `[" src/** ","src/**"]` gives `["src/**"]`.
15. `a_kind_of_another_project_is_refused`.
16. `a_graph_outside_the_project_is_refused_and_none_is_the_kind_default`.
17. `an_override_graph_is_refused_as_a_change`.
18. `only_changed_fields_are_checked`: the context holds no repos and the base has legacy tags `["Rust"]` and paths `["nope:x"]`. A title-only `SpecChanges` gives `Ok`; `between(base, base-with-new-title)` holds the title only.
19. `between_names_only_the_fields_that_differ`, then `is_empty` on equal specs.
20. `into_patch_and_into_new_item_carry_every_field`: `reason == "edited"`; `Some(None)` survives.
21. `text_round_trips`: `parse_tags(&tags_text(x)) == x` and `parse_paths(&paths_text(y)) == y` for canonical `x`, `y`.
22. `debug_prints_lengths_not_body_or_paths`: the sentinels `SECRET-BODY` and `secret/dir/**` are absent from `format!("{:?}")` of both structs.

Validate (E1): `cargo test -p htui-core --all-features --lib -- item_spec excerpt skill_glob`.

## 2. Conformance parity case (Task 2)

`crates/htui-core/src/store/conformance.rs`: append `"update_spec_columns_roundtrip",` after `:166` in `CASES`. Append `"update_spec_columns_roundtrip" => update_spec_columns_roundtrip(store).await,` after the `deleting_…` arm (`:415-417`). Put the function after `update_cas_diverged` (ends about `:834`):
```rust
/// MOD-13 milestone 2 D10 (A10): the four spec columns the item form edits beyond title and body
/// land on the head, and an explicit `Some(None)` clears the graph back to the kind default (§4.2,
/// §7.2). Asserted on the head: `item_revision` keeps title, body and tags only.
async fn update_spec_columns_roundtrip<S: WriteStore>(store: &S) {
    const CASE: &str = "update_spec_columns_roundtrip";
    let before = store.item(ids::HTUI_ANA_2).await.expect(..).expect(..);
    let set = ItemPatch {
        priority: Some(7),
        touched_paths: Some(vec!["src/**".to_owned(), "web:docs/*.md".to_owned()]),
        required_tags: Some(vec!["docker".to_owned(), "rust".to_owned()]),  // already canonical
        step_graph_id: Some(Some(ids::GRAPH_HTUI_FEAT)),                    // htui, non-override; Pg FK holds
        author_id: ids::USER, box_id: Some(ids::BOX), reason: "edited".to_owned(),
        ..ItemPatch::default()
    };
    // Updated: version+1, the four columns as set; title, body, kind_id, key, project_id untouched.
    // A re-read `store.item(id)` agrees on the four columns.
    let clear = ItemPatch { step_graph_id: Some(None), author_id: ids::USER, box_id: Some(ids::BOX),
                            reason: "edited".to_owned(), ..ItemPatch::default() };
    // Updated at head.version: step_graph_id None, version+1, priority/paths/tags unchanged.
}
```
Messages follow the suite's `"{CASE}: …"` form. No title is set, which Pg's `COALESCE($3, title)` allows (`pg/write.rs:828`).

`crates/htui-core/tests/mem_store.rs:37-56`: change `118` to `119` and append `, and MOD-13 milestone 2's one for the spec columns (plan D10)` to the message. `crates/htui-store/tests/pg_conformance.rs:18-29`: change `EXPECTED_CASES` to `119`, extend the doc (`… make it 118, and MOD-13 milestone 2's spec-columns case (plan D10) makes it 119.`) and the message (`119 since MOD-13 milestone 2's spec-columns case`).

Validate: `cargo test -p htui-core --all-features --test mem_store`; `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`.

## 3. Worker (Task 3)

### `crates/htui/src/item_writes.rs` (new)
Module doc: D1–D3, D5, D6, D11, plus a pointer to E6 redaction. Imports: `htui_core::model::{Item, ItemId, ItemKind, ItemRevision, ProjectId, Repo, StepGraph, SpecError, item_spec::{self, NOTHING_TO_SAVE, SpecContext}}`, `htui_core::store::{ReadStore as _, Result, StoreError, UpdateOutcome, WriteStore as _}`, `htui_store::{Backend, DATABASE_UNREACHABLE, Writer}`, `crate::store_worker::{StoreReply, StoreRequest}`.

```rust
/// The three requests, in `StoreRequest::name` order.
pub const REQUEST_NAMES: [&str; 3] = ["item_form", "mint_item", "edit_item"];
pub const FORM_NAME: &str = REQUEST_NAMES[0];
pub const MINT_NAME: &str = REQUEST_NAMES[1];
pub const EDIT_NAME: &str = REQUEST_NAMES[2];
#[must_use] pub fn is_item_request(name: &str) -> bool;   // REQUEST_NAMES.contains(&name)

/// The answer to `StoreRequest::ItemForm` (D3): one project's catalogue, read through the writer.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemFormContext {
    pub project: ProjectId,
    /// `item_kinds(project)`, store order (position, then prefix).
    pub kinds: Vec<ItemKind>,
    /// `step_graphs(project)` minus `is_override` (A3).
    pub graphs: Vec<StepGraph>,
    /// `repos(project)`, name and `is_primary` included.
    pub repos: Vec<Repo>,
    /// The item, fresh, for an edit (its `version` is the compare-and-set token); `None` for new.
    pub item: Option<Item>,
}
impl ItemFormContext { #[must_use] pub fn spec_context(&self) -> SpecContext<'_>; }

/// D6: both sides of a stale edit, carried for milestone 3's view.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDivergence { pub head: Item, pub ancestor: ItemRevision }

/// What an applied item write did (self-naming, MOD-59).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemWrite {
    Minted { key: String },
    Edited { key: String, version: i32 },
}

/// The refusal of an edit-form read whose item is in another project.
#[must_use] pub fn item_not_in_project(key: &str) -> String; // "{key} is not in the project the form was opened for"

/// # Errors
/// Offline: `Unreachable(DATABASE_UNREACHABLE)` for all three, before anything is read (D2).
/// `Constraint` for a spec refusal (`SpecError` through `Display`), `NOTHING_TO_SAVE`, or a
/// cross-project edit read. `NotFound` for an unknown item. `Backend` for a non-item request.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
fn write_access(backend: &Backend) -> Result<Writer>;   // requirements.rs:710 copy
fn offline() -> StoreError;                             // requirements.rs:715 copy
fn refused(err: SpecError) -> StoreError;               // StoreError::Constraint(err.to_string())
async fn catalogue(writer: &Writer, project: ProjectId) -> Result<ItemFormContext>; // item: None, graphs filtered
```
**Arms** (always the writer first):
- `ItemForm { project, item }`: `writer`; `let mut context = catalogue(&writer, *project).await?;` For `Some(id)`: `writer.item(id)` (`NotFound { entity: "item" }` when absent); a `project_id != *project` gives `Constraint(item_not_in_project(&row.key))`; then `context.item = Some(row)`. Answer `StoreReply::ItemForm(Box::new(context))`.
- `MintItem { project, spec }`: `writer`; `context = catalogue(..)` (the validator also drops overrides itself); `let spec = item_spec::check_spec(spec, &context.spec_context()).map_err(refused)?;` `me = backend.this_user()`; `box_id = backend.box_info().await?.map(|b| b.box_id)`; `writer.mint_item(spec.into_new_item(ItemId::new(), *project, me, box_id))`. Answer `ItemWritten { item: row.id, outcome: ItemWrite::Minted { key: row.key } }`.
- `EditItem { id, expected_version, changes }`: `writer`; `changes.is_empty()` gives `Constraint(NOTHING_TO_SAVE)`; `project = writer.item(id)…project_id` (`NotFound` when absent); `context = catalogue(..)`; `let changes = item_spec::check_changes(changes, ..).map_err(refused)?;` then user and box; `writer.update_item(*id, *expected_version, changes.into_patch(me, box_id))`. `Updated(head)` answers `ItemWritten { item: head.id, outcome: Edited { key: head.key, version: head.version } }`; `Diverged { head, ancestor }` answers `StoreReply::ItemDiverged(Box::new(ItemDivergence { head, ancestor }))`. The token is the request's own, never moved here (D6).
- `other`: `Err(StoreError::Backend(format!("not an item request: {}", other.name())))`.

### `crates/htui/src/lib.rs`
Add `pub mod item_writes;` between `pub mod hierarchy;` (`:23`) and `pub mod keymap;` (`:24`).

### `crates/htui/src/store_worker.rs`
- Imports: add `ItemSpec, SpecChanges` to the `htui_core::model` list (`:23-29`). Add `use crate::item_writes::{self, ItemDivergence, ItemFormContext, ItemWrite};` beside `use crate::requirements::{…}` (`:51`).
- `StoreRequest`: after `ReconfirmCitation { … }` (`:847-855`):
```rust
/// MOD-13 milestone 2 D3: the read that opens the Backlog's item form; answered with
/// [`StoreReply::ItemForm`]. Refused offline (D2). For an edit, `project` must be the item's.
ItemForm { project: ProjectId, item: Option<ItemId> },
/// §7.1 through the shared validator (D4); the worker mints the id and fills `created_by` and
/// `box_id`. Answered with [`StoreReply::ItemWritten`].
MintItem { project: ProjectId, spec: ItemSpec },
/// §7.2 at `expected_version`, only the changed columns (D5). Answered with
/// [`StoreReply::ItemWritten`] or [`StoreReply::ItemDiverged`].
EditItem { id: ItemId, expected_version: i32, changes: SpecChanges },
```
  Each field gets a doc line (`missing_docs`).
- `name()`: after `:969`: `// The three of item_writes::REQUEST_NAMES, in that order (MOD-13 milestone 2).`, then `Self::ItemForm { .. } => "item_form"`, `Self::MintItem { .. } => "mint_item"`, `Self::EditItem { .. } => "edit_item"`.
- `StoreReply`: after `ItemCitations(Box<ItemCitations>)` (`:1241`):
```rust
/// Answer to [`StoreRequest::ItemForm`]; boxed, it carries a whole `Item`.
ItemForm(Box<ItemFormContext>),
/// An item write that applied (self-naming, MOD-59): the tab lands a write on this alone.
ItemWritten { item: ItemId, outcome: ItemWrite },
/// An edit that missed its version (D6): nothing was written.
ItemDiverged(Box<ItemDivergence>),
```
- `try_serve`: after the requirements arm (`:1598-1609`), a new or-arm with the F-12 comment: `StoreRequest::ItemForm { .. } | StoreRequest::MintItem { .. } | StoreRequest::EditItem { .. } => item_writes::serve(backend, request).await?,`.

### Task 3 tests (`item_writes.rs` `mod tests`; `Backend::memory(store.clone())`, clones share state)
Helpers: `fn demo() -> (MemStore, Backend)`; `async fn with_repos(store: &MemStore)`, which creates `htui` (`is_primary: true`) and `web` in `PROJECT_HTUI` via `create_repo` (A15); `fn spec(title: &str) -> ItemSpec` (`KIND_HTUI_ANA`, priority 0, no tags or paths, `None` graph, body `"Body."`); `async fn form(backend, project, item) -> ItemFormContext`.
1. `request_names_match_the_name_arms`: `REQUEST_NAMES` equals the three `name()`s; `is_item_request`.
2. `the_new_form_read_carries_kinds_graphs_and_repos`: ids equal to `store.item_kinds/step_graphs/repos(PROJECT_HTUI)`; `item: None`; `project == PROJECT_HTUI`.
3. `the_form_read_drops_override_graphs` (A3): after `create_step_graph(NewStepGraph { is_override: true, .. })`, the graph is absent from `graphs`.
4. `the_edit_form_read_carries_the_item_at_its_version`: `HTUI_ANA_2`, `item.version == 1`.
5. `an_edit_form_read_outside_its_project_is_refused`: `ItemForm { PROJECT_AGY, Some(HTUI_ANA_2) }` gives `Constraint(item_not_in_project("ANA-2"))`.
6. `a_mint_lands_the_next_key_at_version_one_by_this_user` (E9): the answer is `ItemWritten { outcome: Minted { key: "ANA-3" } }`; the row has `version == 1`, `created_by == ids::USER` and the trimmed title; `item_count + 1`.
7. `an_edit_lands_the_next_version_with_reason_edited` (E9): a title-only edit at v1 gives `Edited { version: 2 }`. Then `store.update_item(…, 2, title "Theirs")` moves the head to 3. An `EditItem` at 2 gives `ItemDiverged` with `ancestor.version == 2`, `ancestor.reason == "edited"` and `ancestor.author_id == ids::USER`.
8. `a_stale_edit_diverges_and_writes_nothing`: the head is unchanged after the answer (title and version).
9. `an_empty_edit_is_refused_and_writes_nothing`: `Failed { "edit_item", message ends with NOTHING_TO_SAVE }`; the version is unchanged.
10. `a_mint_canonicalises_its_paths` (after `with_repos`): `[" web:src/** ","src/**","web:src/**"]` lands as `["web:src/**","src/**"]`.
11. `every_spec_refusal_fails_naming_the_entry_and_writes_nothing` (through `store_worker::serve`, so the or-arm is covered). Cases: blank title; tags `["Rust"]`; `KIND_AGY_FEAT`; graph `GRAPH_AGY_FEAT`; `nope:src`; `web: src`; and `EditItem` with `touched_paths: Some(["nope:src"])`. Each answers `Failed { request: name(), message }` with the entry or tag in `message`; `item_count` and `ANA-2`'s version are unchanged.
12. `offline_every_item_request_is_refused_before_anything_is_read`: `Backend::Offline` over a fresh `CacheStore::open(tempdir, "item-writes-offline", 1)`; all three give `Err(Unreachable(DATABASE_UNREACHABLE))` (the `requirements.rs:1973` shape).
13. `mint_and_edit_debug_print_no_body_and_no_paths`: `format!("{request:?}")` holds neither `SECRET-BODY` nor `secret/dir`.
14. `a_request_that_is_not_an_item_one_is_named`: `serve(&backend, &StoreRequest::BoxInfo)` gives `Backend("not an item request: box_info")`.

Validate (E1): `cargo test -p htui --all-features --lib -- item_writes store_worker`.

## 4. Backlog form and wiring (Task 4)

### `crates/htui/src/ui/tabs/backlog/item_form.rs` (new, `pub mod item_form;` beside `pub mod filter;` at `mod.rs:13`)
Everything is `pub`, the `filter.rs` precedent (milestone-1 blueprint §2), so the form commit does not trip `dead_code` before the wiring commit. Each item gets a doc line.

```rust
pub const PAGE: u16 = 5;
/// Hints; both fit the detail pane's inner width at 100x30 (43 columns).
pub const HINT_TEXT: &str = "Tab field  Ctrl+S save  Esc cancel";                   // 34
pub const HINT_PICK: &str = "\u{2190}/\u{2192} choose  Tab field  Esc cancel";      // 33
pub const NO_KINDS: &str = "this project has no item kinds; Settings \u{2192} Kinds adds one";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field { Project, Kind, Title, Priority, Tags, Graph, Paths, Body } // Tab order; Project only on a new form

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy { Reloading(ProjectId), Minting, Editing }

#[derive(Debug)]
pub enum ItemFormOutcome {
    Stay,
    /// A chord the form does not own (`ctrl-c`, `Ctrl+F`): the tab passes it.
    Pass,
    Cancel,
    /// The new form's project picker moved (E7): read that project's catalogue.
    Reload(ProjectId),
    /// Checked and ready; the form is already `busy`.
    Save(StoreRequest),
}

pub struct ItemForm {
    projects: Vec<ProjectRef>,          // ctx.projects at open (new form); empty on an edit
    context: ItemFormContext,
    kind: Option<ItemKindId>,           // None only when context.kinds is empty
    graph: Option<StepGraphId>,         // None = kind default
    title: TextField, priority: TextField, tags: TextField,
    paths: TextArea, body: TextArea,
    opened: Opened,                     // each widget's text right after open, read back from it
    focus: Field,
    busy: Option<Busy>,
    notice: Option<String>,
}
struct Opened { title: String, priority: String, tags: String, paths: String, body: String }
```
`impl Debug for ItemForm` is hand-written (E10): `project`, `item` (id and key only), `kind`, `graph`, `focus`, `busy`, `notice`, and `body_len`/`paths_len`. `Opened` is private with a hand-written `Debug` that prints lengths.

```rust
impl ItemForm {
    /// `N`: text blank, priority "0", kind = `kind` when it is one of `context.kinds` else the
    /// first, graph None, focus Title. `projects` is `ctx.projects` (scope order).
    #[must_use] pub fn open_new(context: ItemFormContext, projects: &[ProjectRef], kind: Option<ItemKindId>) -> Self;
    /// `e`: every widget from `ItemSpec::of(item)` via `tags_text`/`paths_text`/`to_string`,
    /// focus Title. `None` when `context.item` is `None`.
    #[must_use] pub fn open_edit(context: ItemFormContext) -> Option<Self>;
    /// E7: the reload landed. Kind and graph reset, text kept, busy cleared.
    pub fn retarget(&mut self, context: ItemFormContext);
    #[must_use] pub const fn busy(&self) -> Option<Busy>;
    #[must_use] pub fn item_id(&self) -> Option<ItemId>;     // context.item's id
    #[must_use] pub fn project(&self) -> ProjectId;           // context.project
    /// A reply ended the flight: busy cleared, the notice set (or cleared with `None`).
    pub fn settle(&mut self, notice: Option<String>);
    pub fn on_key(&mut self, key: KeyEvent) -> ItemFormOutcome;
    /// Into the focused text widget; dropped on a picker or while busy.
    pub fn on_paste(&mut self, text: &str);
}
pub fn render(frame: &mut Frame<'_>, area: Rect, form: &ItemForm, theme: &Theme);
/// D6.
#[must_use] pub fn item_changed_elsewhere(head: i32) -> String;
// "changed elsewhere \u{2014} now v{head}; your text is kept. Esc and `e` reopen on the head"
/// D11.
#[must_use] pub fn mint_may_have_landed(why: &str) -> String;
// "{why} \u{2014} it may have been written; the list is being re-read, look for it before Ctrl+S"
```
**`on_key` contract**, in this order:
1. `ctrl_s(&key)` (copy of `requirements/mod.rs:180-183`): busy gives `Stay`; otherwise `self.save()` (A6).
2. `key.modifiers.intersects(filter::CHORD)` gives `Pass`.
3. busy gives `Stay` (D8; `Esc` included).
4. `Tab` / `BackTab` cycle the focus with wrap, skipping `Project` on an edit.
5. Per field:
   - **Pickers** (`Project`/`Kind`/`Graph`): `Left`/`h` go back and `Right`/`l` go forward, with wrap. `Enter`/`Down` go to the next field and `Up` to the previous one; `Esc` gives `Cancel`. Moving `Project` sets `busy = Reloading(p)` and answers `Reload(p)`; the picker draws `p` while reloading, otherwise `context.project`. Graph options are `[None]`, then `context.graphs` ids, then, on an edit, the item's own `step_graph_id` when it is not among them (an override, drawn `override (this item's)`).
   - **`TextField`** (`Title`/`Priority`/`Tags`): `Submit` goes to the next field and never saves (D8); `Cancel` gives `Cancel`; `Pass` on `Up`/`Down` goes to the previous or next field; anything else gives `Stay`.
   - **`TextArea`** (`Paths`/`Body`): `on_key(key, PAGE)`; `Cancel` gives `Cancel`; everything else gives `Stay`.

**`save()`**:
- `typed_spec()` builds an `ItemSpec`. On an edit, a field whose widget text equals `opened` keeps the item's stored value **unparsed** (A4). Every other field goes through `parse_title`/`parse_priority`/`parse_tags`/`parse_paths`; the body is taken as typed. `kind: None` refuses with `NO_KINDS`.
- **New**: `check_spec(&spec, &context.spec_context())`, then `busy = Minting` and `Save(MintItem { project: context.project, spec })`.
- **Edit**: `changes = SpecChanges::between(&ItemSpec::of(item), &spec)`. Empty sets the notice to `NOTHING_TO_SAVE` and answers `Stay` (D5). Otherwise `check_changes`, then `busy = Editing` and `Save(EditItem { id, expected_version: item.version, changes })`. The token is `context.item.version` and never moves (D6).
- Any refusal sets `notice = Some(sentence)`, answers `Stay` and leaves `busy` at `None`.

**Render** into the detail pane's `right` rect: a `Block` with `Borders::ALL`, titled `" New item · {slug} "` or `" Edit {key} (v{version}) "`. The inner rows are a `Layout::vertical` with one `Length(1)` per picker or one-line field, the `paths (one per line)` label, the paths area at `Length(3)`, the `body` label, the body at `Min(3)`, the notice at `Length(2)` (wrapped, `theme.error`) and the hint at `Length(1)` (`theme.dim`; `HINT_PICK` on a picker, else `HINT_TEXT`). Rows read `"> "` or `"  "`, a label padded to 9, then `‹label›` for a picker or `TextField::line(width, focused, theme)`. The text areas use `TextArea::lines(w, h, focused, theme)`. Kind labels read `"{prefix} {name}"`, the graph `kind default` or the graph name, the project its slug. Every state is visible as characters (`>`, `‹ ›`), because snapshots are text.

### `crates/htui/src/ui/tabs/backlog/mod.rs`
- Imports: `crate::item_writes::{self, ItemDivergence, ItemFormContext, ItemWrite}`; `item_form::{Busy, ItemForm, ItemFormOutcome, item_changed_elsewhere, mint_may_have_landed}`; `htui_core::model::ItemKindId`. Update the module doc (`:1-10`) for milestone 2.
- Fields (struct `:53-82`, `new()` `:103-116`):
```rust
/// MOD-13 milestone 2 D8 (A7): the open item form, capturing every key but chords while `Some`.
item_form: Option<ItemForm>,
/// E8: the `ItemForm` read `N`/`e` is waiting on; a reply opens a form only when it answers this.
opening: Option<Opening>,
```
  `#[derive(Debug, Clone, Copy, PartialEq, Eq)] struct Opening { project: ProjectId, item: Option<ItemId>, kind: Option<ItemKindId> }` (private). The three test literals (`:594`, `:767`, `:815`) get `item_form: None, opening: None`.
- `go()` (`:147-167`): after `let Some(id) = item else { return };`, replace the seven `ctx.request`s with `self.read_item(id, ctx);`. New `fn read_item(&self, id: ItemId, ctx: &Ctx<'_>)` holds those seven lines verbatim (A5). It does not call `on_item_change`, so the sub-tabs keep their state and take the fresh replies.
- `on_key` (`:323`): after the filter-form guard (`:326-328`), add `if self.item_form.is_some() { return self.on_item_form_key(key, ctx); }`. In the letter match, beside `F` (`:355`), add `KeyCode::Char('N') => self.open_new(ctx),` and `KeyCode::Char('e') => self.open_edit(ctx),`. Both are consumed.
```rust
/// D7: the selected item's project, a selected header's project, else the first scope project.
/// `kind` hints the selected item's kind (§9 decision 4).
fn open_new(&mut self, ctx: &Ctx<'_>) {
    let target = match self.selected {
        Some(Selection::Item(_)) => self.item().map(|i| (i.project_id, Some(i.kind_id))),
        Some(Selection::Project(p)) => Some((p, None)),
        None => None,
    }.or_else(|| ctx.projects.first().map(|p| (p.project_id, None)));
    let Some((project, kind)) = target else { return };
    self.opening = Some(Opening { project, item: None, kind });
    ctx.request(StoreRequest::ItemForm { project, item: None });
}
/// D7: nothing selected, nothing sent.
fn open_edit(&mut self, ctx: &Ctx<'_>) { let Some(item) = self.item() else { return }; /* opening + ItemForm { item.project_id, Some(item.id) } */ }
fn on_item_form_key(&mut self, key: KeyEvent, ctx: &Ctx<'_>) -> Handled {
    let Some(form) = self.item_form.as_mut() else { return Handled::Pass };
    match form.on_key(key) {
        ItemFormOutcome::Pass => return Handled::Pass,
        ItemFormOutcome::Stay => {}
        ItemFormOutcome::Cancel => self.item_form = None,
        ItemFormOutcome::Reload(project) => ctx.request(StoreRequest::ItemForm { project, item: None }),
        ItemFormOutcome::Save(request) => ctx.request(request),
    }
    Handled::Consumed // q, digits, w, ?, Tab must not reach the global keymap
}
```
- `on_paste` (`:313-319`): before the filter-form branch, `if let Some(form) = self.item_form.as_mut() { form.on_paste(text); return Handled::Consumed; }`.
- `on_scope_change` (`:299-306`): add `self.item_form = None; self.opening = None;`.
- `reveal` guard (`:468`): `if self.form.is_some() || self.item_form.is_some() || self.detail.captures_input()`.
- `on_reply` (`:374`): at the top, before the `Items` branch:
```rust
match reply {
    StoreReply::ItemForm(context) => return self.on_item_form(context, ctx),
    StoreReply::ItemWritten { item, outcome } => return self.on_item_written(*item, outcome, ctx),
    StoreReply::ItemDiverged(divergence) => return self.on_item_diverged(divergence),
    StoreReply::Failed { request, message } if item_writes::is_item_request(request) => {
        return self.on_item_failed(request, message, ctx);
    }
    _ => {}
}
```
  - `on_item_form(&mut self, context: &ItemFormContext, ctx: &Ctx<'_>)`:
    - If a form is open with `busy == Some(Reloading(p))`, `context.project == p` and `context.item.is_none()`: `form.retarget(context.clone())`.
    - Otherwise, when there is no item form: `let Some(opening) = self.opening.take()`. It must match `(context.project, context.item id)`, else drop. If the filter form opened meanwhile or `detail.captures_input()`, drop as well (no error). Then `open_new(context.clone(), ctx.projects, opening.kind)` or `open_edit(context.clone())`.
    - Anything else is dropped.
  - `on_item_written(&mut self, item: ItemId, outcome: &ItemWrite, ctx: &mut Ctx<'_>)` lands only on the write in flight; anything else is dropped:
    - `Edited` with `busy == Editing` and `form.item_id() == Some(item)`: `item_form = None; self.read_item(item, ctx); ctx.request(self.filter.to_request(ctx.scope));` (D9, A5).
    - `Minted { key }` with `busy == Minting`: `item_form = None; Tab::reveal(self, &RevealTarget::Item { id: item, key: key.clone() }, ctx);`. The reveal's miss branch (`:479-483`) clears the filter and arms `pending_reveal`; the next `Items` selects it (A5).
  - `on_item_diverged`: `busy == Editing` and `form.item_id() == Some(divergence.head.id)` give `form.settle(Some(item_changed_elsewhere(divergence.head.version)))`. The text and the token are kept (D6).
  - `on_item_failed(&mut self, request: &str, message: &str, ctx: &Ctx<'_>)`. No `Action::Error`, because `App::on_reply` already emits it (A8, `app/update.rs:285-287`).
    - `FORM_NAME`: a form `Reloading` settles with `Some(message)`; otherwise `self.opening = None`.
    - `MINT_NAME` with `Minting`: `settle(Some(mint_may_have_landed(message)))` and `ctx.request(self.filter.to_request(ctx.scope))` (D11).
    - `EDIT_NAME` with `Editing`: `settle(Some(message.to_owned()))`.
- `render` (`:427-460`): replace the `detail::render(…)` call (`:453`) with `match &self.item_form { Some(form) => item_form::render(frame, right, form, ctx.theme), None => detail::render(frame, right, &self.detail, self.item().map(|item| item.key.as_str()), ctx) }` (D8). With no form open, nothing changes.

### `crates/htui/src/app/mod.rs`
After the `f`/`F` loop (`:129-136`), the same loop shape: `for (key, help) in [('N', "new item"), ('e', "edit item")]` binds `KeyScope::Tab(BacklogTab::ID)` to `Action::Tab(TabAction::Focus(BacklogTab::ID))`. Add doc item `10.` to `:52-53`.

### Task 4 tests
**`item_form.rs` `mod tests`** (pure; contexts from `crate::item_writes::serve` over `Backend::memory(MemStore::demo())`):
1. `a_new_form_opens_on_the_title_with_the_hinted_kind`; an unknown hint falls back to the first kind.
2. `tab_cycles_every_field_and_the_edit_form_has_no_project`.
3. `enter_on_a_text_field_moves_on_and_never_saves`.
4. `ctrl_s_on_a_blank_title_refuses_in_the_notice`: `Stay`, notice `SpecError::BlankTitle.to_string()`, `busy` stays `None`.
5. `an_unchanged_edit_says_nothing_to_save`.
6. `a_changed_title_saves_only_the_title_at_the_opened_version`: `matches!(Save(StoreRequest::EditItem { expected_version: 1, changes, .. }) if changes == SpecChanges { title: Some(..), ..Default::default() })`.
7. `reformatted_tags_are_not_a_change`: `rust,docker` over a base `docker, rust` gives `NOTHING_TO_SAVE`.
8. `a_legacy_tag_does_not_block_a_title_edit` (A4): the item gets `required_tags: ["Rust"]` straight through `MemStore::update_item`, then a title edit saves.
9. `a_bad_path_is_refused_by_name_before_anything_is_sent`: `nope:src` gives a notice with `` `nope` `` and no `Save`.
10. `moving_the_project_picker_reloads_and_retarget_keeps_the_text`.
11. `busy_swallows_plain_keys_esc_and_ctrl_s_but_passes_ctrl_c`.
12. `a_paste_lands_in_the_focused_text_field_and_not_on_a_picker`.
13. `the_graph_picker_offers_kind_default_project_graphs_and_keeps_an_override`.
14. `the_hints_fit_the_detail_pane`: both are `<= panes(chrome(Rect::new(0,0,100,30)).body)[1].width - 2`.
15. `debug_prints_no_body`.

**`mod.rs` `mod tests`** (reuse `platform()` `:714`; a small `Bench { scope, projects, top_bar, keymap, theme, emit }` with `fn ctx(&self) -> Ctx<'_>` and `fn sent(&self) -> Vec<StoreRequest>` from `Action::Store`). Replies come from `crate::store_worker::serve` over a shared `MemStore`:
1. `n_reads_the_item_form_on_the_selected_project_with_and_without_shift` (E4): `Char('N')` with `NONE` and with `SHIFT` each send exactly one `ItemForm { project: PROJECT_HTUI, item: None }`.
2. `e_reads_the_selected_item_and_nothing_without_a_selection`.
3. `the_item_form_reply_opens_the_form_and_it_captures`: `j` and `q` are `Consumed`, `selected` is unchanged, nothing is sent.
4. `an_item_form_reply_nobody_asked_for_opens_nothing` (E8).
5. `a_paste_goes_to_the_open_item_form`.
6. `ctrl_s_on_a_blank_title_refuses_in_the_form_and_sends_nothing`.
7. `an_unchanged_edit_says_nothing_to_save_and_sends_nothing`.
8. `a_changed_title_sends_edit_item_with_only_the_title_at_the_read_version`.
9. `an_applied_edit_closes_the_form_and_re_reads_the_item_keeping_the_selection`: the seven per-item requests plus one `Items`; `selected` is unchanged.
10. `an_applied_mint_closes_the_form_clears_the_filter_and_reveals_it`: with a `{statuses:[Done]}` filter set first, the result is `filter.is_empty()`, `pending_reveal == Some((id, "ANA-3"))` and one default `Items`. Then `on_reply(Items(served))` gives `selected == Some(Selection::Item(id))`.
11. `a_diverged_edit_keeps_the_form_its_text_and_its_token`: a second Ctrl+S still sends `expected_version: 1`.
12. `a_failed_mint_keeps_the_text_hedges_and_re_reads_the_list`.
13. `a_failed_item_form_read_opens_nothing_and_adds_no_error`: no `Action::Error` in `emit`.
14. `a_scope_change_closes_the_item_form_and_forgets_the_opening`.
15. `a_reveal_while_the_item_form_is_open_asks_to_close_it_first`.
16. `ctrl_c_passes_through_the_open_item_form`.

Validate: `cargo test -p htui --all-features --lib backlog`.

## 5. Integration, snapshots, Pg (Task 5)

### `crates/htui/tests/backlog.rs`
Add a section header `// New and edit (MOD-13 milestone 2)`. Reuse `backlog()`, `backlog_over` (E2), `keys`, `type_text`, `detail_pane`. From Title, `tab` ×4 reaches Paths (Priority, Tags, Graph, Paths).
1. `n_mints_an_item_and_reveals_it`: `N`, type `Fresh item`, `ctrl-s`, then `drive_to_end`. The detail pane holds `ANA-3` and `Fresh item` (the Body proves the selection; list selection is style-only). `status == None`.
2. `e_edits_the_title_and_the_body_shows_version_2` (E3): `e`, `type_text(" v2")` (cursor at the end), `ctrl-s`. The detail pane holds `Data model, box registry v2` and `version 2`.
3. `a_stale_edit_keeps_the_form_and_never_overwrites_the_head` (A13): `let store = MemStore::demo(); backlog_over(store.clone())`. Then `e`, and `store.update_item(HTUI_ANA_1, 1, title "Theirs")`. Type ` mine`, then `ctrl-s`: the frame holds `now v2` and ` Edit ANA-1 (v1) `, and the head is `"Theirs"` at v2. A second `ctrl-s` still diverges and the head is unchanged.
4. `an_unknown_repo_in_touched_paths_is_refused_by_name`: `N`, a title, `tab` ×4, type `nope:src`, `ctrl-s`. The frame holds `` `nope` ``, `store.item_count()` is unchanged and `status == None` (a form-side refusal).
5. `offline_n_and_e_are_refused_with_the_read_only_notice` (A11, E11):
   - Start as `offline_the_runs_pane_asks_for_no_error` does (`:1540-1560`): keyring guard, tempdir, `CacheStore::open(.., PgStore::schema_version())`, `seed_mirror`.
   - Then `seed_mirror_items(&cache, &demo_data().items).await`, build the harness, `SetScope { workspace: workspace("platform") }` and `drive_to_end`.
   - `N` gives `status == Some(format!("item_form: store unreachable: {DATABASE_UNREACHABLE}"))`, and `New item` is absent from the frame. Clear `harness.app().status = None`, then `e` gives the same; ` Edit ` is absent.
   - `cache.item(HTUI_ANA_1)` is still at `version 1`; the Platform items count is still 11.
   ```rust
   /// A11: the demo's items in the mirror's `item` table, with its encodings (TEXT uuids, JSON
   /// arrays, microsecond stamps, `0001_mirror.sql:75-82` plus `0004`'s `resolution`).
   async fn seed_mirror_items(cache: &CacheStore, items: &[Item]) {
       for item in items {
           sqlx::query("INSERT INTO item (id, project_id, kind_id, key_prefix, key_number, key, title, body, \
                        status, priority, required_tags, touched_paths, step_graph_id, version, created_by, \
                        created_at, updated_at, closed_at, resolution) \
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
               .bind(item.id.to_string()).bind(item.project_id.to_string()).bind(item.kind_id.to_string())
               .bind(&item.key_prefix).bind(i64::from(item.key_number)).bind(&item.key)
               .bind(&item.title).bind(&item.body).bind(item.status.as_str()).bind(i64::from(item.priority))
               .bind(serde_json::to_string(&item.required_tags).expect("json"))
               .bind(serde_json::to_string(&item.touched_paths).expect("json"))
               .bind(item.step_graph_id.map(|id| id.to_string())).bind(i64::from(item.version))
               .bind(item.created_by.to_string()).bind(item.created_at.timestamp_micros())
               .bind(item.updated_at.timestamp_micros()).bind(item.closed_at.map(|at| at.timestamp_micros()))
               .bind(item.resolution.map(|r| r.as_str()))
               .execute(cache.pool()).await.expect("a mirror item row");
       }
   }
   ```
   `sqlx` is already a dev-dependency (`Cargo.toml` `[dev-dependencies]`) with `sqlite`. It is runtime-checked, so `.sqlx/` does not move.
6. `n_and_e_are_on_the_backlog_help_line`: help contains `N new item` and `e edit item` (the shape of `:1362-1371`).
7. Snapshot `backlog__item_form_new`: `backlog()`, `N`, `drive_to_end`, type `Fresh item`. Assert ` New item · htui `, `> title`, `‹ANA ` and `kind default`; `insta::assert_snapshot!("item_form_new", frame)`.
8. Snapshot `backlog__item_form_edit`: `backlog()`, `e`. Assert ` Edit ANA-1 (v1) ` and that there is no `project` row; `insta::assert_snapshot!("item_form_edit", frame)`.

All frames are 100x30. Accept the snapshots with `cargo insta accept`. **No existing `backlog__*.snap` may change.**

### `crates/htui/tests/item_writes_pg.rs` (new; `#![cfg(feature = "testkit")]`, `Stack` copied from `tests/requirements_pg.rs:23-67`)
1. `mint_edit_and_a_stale_edit_on_postgres`: a mint answers `Minted { key: "ANA-3" }` and the row has `created_by == stack.db.store.this_user()`. A title edit at v1 answers `Edited { version: 2 }`. An edit at v1 again answers `ItemDiverged` with `head.version == 2` and `ancestor.version == 1`, and the head is unchanged.
2. `a_spec_refusal_writes_nothing_on_postgres`: `nope:src` gives `Failed { "mint_item", contains "`nope`" }`; the scope's item count is unchanged.
3. `the_form_read_on_postgres_matches_memory`: `ItemForm { PROJECT_HTUI, None }` kind ids and graph ids equal `MemStore::demo()`'s; `repos` is empty.

Each case `return`s when `Stack::new` is `None` (the SKIP rule) and ends with `stack.finish().await`.

Validate: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`; `cargo test -p htui --all-features --test item_writes_pg -- --test-threads=1`.

## 6. Answers to the brief's specific questions
- **The project picker's projects**: `ctx.projects` (the shell's scope projects, `Ctx::projects`, `app/state.rs:78`), copied into the form by `on_item_form` at open, the way `FilterForm::open` copies them. A scope change closes the form. A move re-reads that project's catalogue (E7).
- **Routing `ItemForm` to the requester**: `ctx.request` pushes `Action::Store`. `drain` stamps it with the tab's `Origin::Tab(BacklogTab::ID)` (`app/state.rs:352`). `App::on_reply` hands the reply to that tab alone (`app/update.rs:291-314`), and the freshness gate keeps the newest per `(origin, discriminant)` (`state.rs:309-312, 329-333`). A second `N` therefore supersedes the first, and the `opening` token covers a reply that outlives a scope change (E8).
- **`busy` and matching `ItemWritten`**: the form's `busy` is `Minting` or `Editing`. Only one write can be in flight, because busy swallows Ctrl+S, and the gate keys each request kind separately. `Edited` must also match `form.item_id()`; `ItemDiverged` must match `head.id`; a `Failed` must match its request name to the busy kind. Every other reply is dropped.
- **Where the form renders**: `right` from `panes(area)` (`mod.rs:428`), in place of `detail::render` (`:453`).
- **Mirror seeding**: `seed_mirror_items` (§5 test 5) over `cache.pool()` (`cache/mod.rs:231`, `pub const fn`). The columns are those of `0001_mirror.sql:75-82` plus `resolution` (`0004_requirements.sql:16`).
- **Conformance case body**: §2. The fixture is `HTUI_ANA_2` (an htui `ANA`) and the graph is `GRAPH_HTUI_FEAT` (`fixtures.rs:197`), a non-override htui graph, so the Pg FK holds and no graph is created.

## 7. Commit boundaries (implementers commit at each green; explicit paths; no `-A`, no stash, no amend)
1. `refactor(mod-13): split_qualifier out of PathPrefix::parse; skill_glob matcher pub(crate)`: `prompt/excerpt.rs`, `model/skill_glob.rs`.
2. `feat(mod-13): item_spec, the shared spec validator`: `model/item_spec.rs`, `model/mod.rs`.
3. `test(mod-13): update_spec_columns_roundtrip conformance case (119)`: `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`.
4. `feat(mod-13): item_writes worker (ItemForm, MintItem, EditItem)`: `item_writes.rs`, `lib.rs`, `store_worker.rs`.
5. `feat(mod-13): the Backlog item form`: `backlog/item_form.rs` and the `pub mod item_form;` line.
6. `feat(mod-13): N and e open the item form in the Backlog`: `backlog/mod.rs`, `app/mod.rs`.
7. `test(mod-13): new/edit integration cases and snapshots`: `tests/backlog.rs`, the two new `.snap` files.
8. `test(mod-13): item writes on Postgres`: `tests/item_writes_pg.rs`.

Each commit passes `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Every message ends with the `Co-Authored-By` line. Close the milestone with the plan's Validation block, running the full gate with `--no-fail-fast -- --test-threads=1` and grepping for `SIGABRT`.

## 8. Hazards beyond the errata
- **Key propagation.** The form consumes every non-chord. A passed `q`, digit, `w`, `?` or `Tab` would act globally (`state.rs:556-595`). Ctrl+S is checked before the chord pass (A6), and `TextArea` would also submit on it.
- **Busy swallows `Esc`.** A write that never answers keeps the form shut until a scope change. That is accepted: one reply per request is the worker's contract (`store_worker.rs:1450-1455`).
- **`large_enum_variant`.** `MintItem` and `EditItem` are about 150 bytes, near the largest existing variants. If clippy flags them, box `spec`/`changes` (`Box<ItemSpec>`), with no other change.
- **`unused_qualifications`** is a workspace warn under `-D warnings`. Do not write `crate::model::…` where the name is imported.
- **Width.** The detail pane is 45 columns (43 inner). The D6 notice wraps to two lines, and `Length(2)` holds it. Titles longer than the field width scroll inside `TextField`.
- **Suite hygiene.** Run the Backlog integration file with `--test-threads=1`, because the keyring fake is process-wide (memory note). Run `--features testkit` or the `tests/*.rs` files report 0 tests.
- **HANDOFF counts.** `HANDOFF.md:59` tracks `StoreRequest 91, StoreReply 52`; they become 94 and 55. Update at close-out, not in these commits.
- **Naming.** `StoreRequest::ItemForm`, `StoreReply::ItemForm` and `item_form::ItemForm` coexist. The variants are always written qualified.

## 9. For the maintainer to decide (defaults applied above)
1. **D11 versus MOD-59 re-review L2.** The Requirements tab shows a mint refused *before* the insert as a plain refusal (`requirements.rs:292-303` `mint_refused`). D11 hedges every mint `Failed`. The blueprint follows D11 literally, so a stale-catalogue refusal (for example a kind deleted since the form opened) reads "… it may have been written". Adopting L2 means adding `item_writes::mint_refused(message) -> bool` (offline, `NOTHING_TO_SAVE`, or any `constraint violated: ` prefix, since a store `Constraint` never follows a COMMIT).
2. **The hedge re-read uses the active filter (D11).** A filter can hide the minted item the notice asks the user to look for. The alternative is to clear the filter, as the mint's reveal does (A5).
3. **The project picker re-reads per project (E7).** The alternative is to fix the project at open (the selected one) and drop the picker.
4. **The kind default on `N`** is the selected item's kind, else the first kind (§4 `open_new`). The alternative is always the first kind.
5. **Editing any path re-checks the whole path list** (A4 works per field, not per entry). An item with one legacy path must fix it to change any path.

## 10. Maintainer decisions (2026-10-01): these override §9's defaults and the sections above
1. **Mint hedge: hedge store errors only (adopts MOD-59 re-review L2; amends D11).** Add
   `item_writes::mint_refused(message: &str) -> bool`, mirroring `requirements.rs:292-303`
   `mint_refused`. It returns true for offline (`DATABASE_UNREACHABLE`), for `NOTHING_TO_SAVE`, and
   for any `StoreError::Constraint` rendering. Before relying on the prefix, verify the exact
   `Display` of `StoreError::Constraint` in the tree; a store `Constraint` never follows a COMMIT.
   The tab works as follows:
   - **Refused mint** (`mint_refused` is true): the notice is the plain message, the form keeps its
     text, nothing is re-read, and the filter is not touched.
   - **Any other mint `Failed`:** the notice is the `mint_may_have_landed` hedge, the form keeps its
     text, and the list is re-read.

   Tests:
   - T3 adds `mint_refused_tells_a_refusal_from_a_store_failure`: a spec refusal, `NOTHING_TO_SAVE`
     and offline give true; `Backend`/`Unreachable`-other give false.
   - T4 test 12 splits into 12a `a_store_failed_mint_keeps_the_text_hedges_and_re_reads_unfiltered`
     and 12b `a_refused_mint_keeps_the_text_and_says_why_without_hedging_or_re_reading`.
2. **The hedge re-read clears the filter (amends D11).** It resets the filter to default and
   re-reads `Items` unfiltered, the same as the reveal path of a successful mint (A5). The notice
   asks the user to look for the item, and the filter must not hide it. Test 12a asserts
   `filter.is_empty()` and exactly one default `Items`.
3. **E7 project picker re-reads per project: accepted as written.**
4. **`N` kind default (the selected item's kind, else the first kind): accepted as written.**
5. **Path edits re-check the whole list: accepted as written.**
