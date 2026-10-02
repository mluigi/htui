# Blueprint: MOD-13 milestone 3, the three-way divergence view

This file makes the approved plan `.claude/plans/mod-13-divergence.plan.md` (D1–D9, T1–T4) ready to compile. It does not
reopen any of the plan's decisions. Every anchor was checked with Gortex on `hr/MOD-13` at `4da89ad3`.
Where this file and the plan disagree on a detail, this file wins (errata §0).

## 0. Errata (plan vs. tree)

| # | Plan claim | What the tree shows | Consequence |
|---|---|---|---|
| E1 | T2 / C5: "test patterns without `..` at `item_form.rs:1091, 1145` and `backlog/mod.rs:2020`" | `item_form.rs:1145-1148` (`a_legacy_tag_does_not_block_a_title_edit`) already ends in `..`, and so does `backlog/mod.rs:2219` (`a_diverged_edit_keeps_…`). The only exhaustive patterns are `item_form.rs:1091-1095` and `backlog/mod.rs:2020` | T2 touches `:1091` and `:2020` only. Both get `reason: EditReason::Edited` rather than `..`, so they pin the reason (§2c) |
| E2 | T1 validate `cargo test -p htui-core --all-features --lib item_merge item_spec`; T2 validate `… --lib item_writes store_worker` | `cargo test` takes **one** `[TESTNAME]`, so the second positional is a CLI error. This is milestone 2's E1 again | The filters go after `--`: `cargo test -p htui-core --all-features --lib -- item_merge item_spec` and `cargo test -p htui --all-features --lib -- item_writes store_worker` |
| E3 | T1 files: `item_merge.rs`, `mod.rs`, `item_spec.rs`, conformance and the two count files. `into_patch`'s third caller (`item_writes.rs:255`) is listed under T2 | `into_patch` is in `htui-core`, but `item_writes.rs:255` is in `htui`. If T1 changes the signature alone, the workspace stops compiling between T1 and T2, and every commit's `cargo clippy --workspace --all-targets` gate fails | T1's reason commit also carries the one-line `item_writes.rs:255` fix, `changes.into_patch(me, box_id, EditReason::Edited)`. T2 then replaces it with the request's `*reason` |
| E4 | T4: "`m`, then Ctrl+S, lands v3, and Body shows v3" | `BodyTab` writes `… · version {v}` (`detail/body.rs`), and no `v3` text exists. This is milestone 2's E3 again | Assert `version 3` in the detail pane |
| E5 | D8: `NOTHING_TO_SAVE` "only happens with `t` when every change of mine conflicted" | `merge` marks a field `Same` when both sides changed it to the same value (D2). When every change of mine is already in the head, both `m` and `t` resolve to the head, and Ctrl+S is also `NOTHING_TO_SAVE` | No behaviour change. The notice is `NOTHING_TO_SAVE` in both cases, which is correct because the head already holds my edit. The view may then list only `theirs` rows, or no rows at all (§3a render handles zero rows). Test `my_edit_already_in_the_head_has_nothing_to_save` (§3d) |
| E6 | C13: the item-form guard at `backlog/mod.rs:568-573`, "ahead of the list's `m` (`:586`)" | The guard is at `:572-574` and the list's `m` at `:593` (anchor drift) | None |
| E7 | D4 / D5: "`j/k/PgUp/PgDn` scroll" | The Backlog's shared `detail::Scroll` (`detail/mod.rs:321-357`) answers `J`/`K` (capitals) and `PageUp`/`PageDown`, and is `pub` only to the detail pane | The view keeps its own `scroll: u16` and takes `j`/`Down`, `k`/`Up`, `PageDown`/`PageUp` (10 rows, `detail::PAGE`'s value). It uses `Scroll`'s clamp rule: the logical line count minus one, a lower bound on the wrapped count, so the view never scrolls blank (§3a) |

## 1. Task 1, core model and conformance (`htui-core`, plus the `htui-store` count)

### 1a. `crates/htui-core/src/model/item_spec.rs`: the reason

After `EDITED` (`:24`):
```rust
/// `item_revision.reason` of an edit that resolves a divergence (MOD-13 milestone 3 D6, ANA-9 §4.2 step 3).
pub const DIVERGENCE_RESOLUTION: &str = "divergence_resolution";

/// Why an edit is written: the `item_revision.reason` it lands with (milestone 3 D6). Closed, so
/// the worker has nothing to validate. Not a `str_enum!`: `item_revision.reason` has no `CHECK`
/// (`0001_init.sql:348`), and the store keeps other reasons (`created`, `imported`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditReason {
    /// A form edit from the item it opened on: [`EDITED`].
    Edited,
    /// A form rebased on the head after a divergence: [`DIVERGENCE_RESOLUTION`].
    DivergenceResolution,
}

impl EditReason {
    /// The revision's `reason` text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self { Self::Edited => EDITED, Self::DivergenceResolution => DIVERGENCE_RESOLUTION }
    }
}
```
`SpecChanges::into_patch` (`:445-460`) changes to
`pub fn into_patch(self, author_id: UserId, box_id: Option<BoxId>, reason: EditReason) -> ItemPatch`, with
`reason: reason.as_str().to_owned()`. Its doc becomes "The `update_item` patch for these changes, with `reason`'s
revision text."

`model/mod.rs:125` becomes `pub use item_spec::{EditReason, ItemSpec, SpecChanges, SpecContext, SpecError};`.

**Callers (E3).**
- `item_spec.rs:994`: `SpecChanges::from(full.clone()).into_patch(user, box_id, EditReason::Edited)`.
- `item_spec.rs:1008`: `…into_patch(user, None, EditReason::Edited)`.
- `crates/htui/src/item_writes.rs:255`: `changes.into_patch(me, box_id, EditReason::Edited)`, with `EditReason` added to
  the `htui_core::model::{…}` import (`:39-41`).

Tests in `item_spec.rs` `mod tests`:
1. `edit_reason_names_its_revision_reason`: `EditReason::Edited.as_str() == "edited" == EDITED`, and
   `EditReason::DivergenceResolution.as_str() == "divergence_resolution" == DIVERGENCE_RESOLUTION`.
2. Extend `into_patch_and_into_new_item_carry_every_field` (`:960-1012`):
   `SpecChanges::from(full).into_patch(user, box_id, EditReason::DivergenceResolution).reason == "divergence_resolution"`.
   The existing `reason: "edited"` literal stays, behind `EditReason::Edited`.

### 1b. `crates/htui-core/src/model/item_merge.rs` (new)

Module doc: milestone 3 D2. It is a field-level three-way merge over the seven `version`-covered columns (ANA-9 §4.2).
`Same`/`Theirs`/`Mine` fields keep both sides' changes, and a pick decides conflicts only. This is the reason "take mine
whole" is wrong: the form's spec still holds the ancestor's value in every field the user did not touch, so sending it
whole would revert the head's changes (R-ENT-10, run the other way). Body and paths compare whole (no per-hunk merge,
per the PRD). There is no I/O.

Import: `use crate::model::ItemSpec;`.

```rust
/// The seven `version`-covered columns, in the item form's `Tab` order minus its project picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecField { Kind, Title, Priority, Tags, Graph, Paths, Body }

impl SpecField {
    /// Every field, in that order: the order the view lists its rows in.
    pub const ALL: [Self; 7] = [Self::Kind, Self::Title, Self::Priority, Self::Tags, Self::Graph, Self::Paths, Self::Body];
    /// The form's row label: `kind`, `title`, `priority`, `tags`, `graph`, `paths`, `body`.
    #[must_use] pub const fn label(self) -> &'static str;
}

/// How one field moved between the ancestor and each side (D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldState {
    /// No side changed it, or both changed it to the same value.
    Same,
    /// Only the head changed it.
    Theirs,
    /// Only the form changed it.
    Mine,
    /// Both changed it, to different values.
    Conflict,
}
impl FieldState {
    /// `same`, `theirs`, `mine`, `conflict`: the view's state column.
    #[must_use] pub const fn label(self) -> &'static str;
}

/// Which side wins the conflicts (`t` / `m`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side { Theirs, Mine }

/// The three specs a divergence compares. `Debug` is derived: `ItemSpec`'s own prints the body
/// and the paths as lengths (milestone 2 E6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecMerge { ancestor: ItemSpec, theirs: ItemSpec, mine: ItemSpec }

/// D2: `ancestor` is the item the form opened on, `theirs` the head, `mine` the form's spec.
#[must_use]
pub fn merge(ancestor: &ItemSpec, theirs: &ItemSpec, mine: &ItemSpec) -> SpecMerge; // clones the three

impl SpecMerge {
    #[must_use] pub const fn ancestor(&self) -> &ItemSpec;
    #[must_use] pub const fn theirs(&self) -> &ItemSpec;
    #[must_use] pub const fn mine(&self) -> &ItemSpec;
    /// One field's state, compared as stored values.
    #[must_use] pub fn state(&self, field: SpecField) -> FieldState;
    /// The fields that are not `Same`, in [`SpecField::ALL`] order.
    #[must_use] pub fn changed(&self) -> Vec<SpecField>;
    /// Whether any field is `Conflict` (without one, `m` and `t` give the same spec).
    #[must_use] pub fn has_conflict(&self) -> bool;
    /// D2: `theirs` for `Same`/`Theirs`, `mine` for `Mine`, `side` for `Conflict`.
    #[must_use] pub fn resolve(&self, side: Side) -> ItemSpec;
}
```
Body shape:
- A private generic `fn classify<T: PartialEq>(ancestor: &T, theirs: &T, mine: &T) -> FieldState`. It reads
  `(theirs != ancestor, mine != ancestor)`: `(false, false)` gives `Same`, `(true, false)` gives `Theirs` and
  `(false, true)` gives `Mine`. `(true, true)` gives `Same` when `theirs == mine`, else `Conflict`.
- `state` matches `field` to the one column: `kind_id`, `title`, `priority`, `required_tags`, `step_graph_id`,
  `touched_paths` or `body`.
- `resolve` builds an `ItemSpec` field by field through a private
  `fn pick<T>(&self, field: SpecField, side: Side, get: impl Fn(&ItemSpec) -> T) -> T`. It takes `&self.theirs` for
  `Same | Theirs` and for `(Conflict, Side::Theirs)`, and `&self.mine` for `Mine` and for `(Conflict, Side::Mine)`.

`model/mod.rs`:
- Add `pub mod item_merge;` between `pub mod item;` and `pub mod item_spec;` (`:87-88`).
- Add `pub use item_merge::{FieldState, Side, SpecField, SpecMerge};` beside `:125`. Callers write
  `item_merge::merge` (the `item_spec::check_spec` convention).
- No other `Side`/`SpecField`/`FieldState` exists in the workspace (checked).

Tests in `item_merge.rs` `mod tests`. Use hand-built specs: a `base()` with fixed `ItemKindId`/`StepGraphId`s from
`::new()`, body `"one\ntwo"`, tags `["rust"]`, paths `["src/**"]`, graph `Some(g1)`. Do not use the demo (A15).
1. `each_field_moves_through_every_state`: for each of the seven fields, `Same` (no change), `Theirs` (only theirs),
   `Mine` (only mine) and `Conflict` (both, differently). The graph field includes `Some(g1)` → `None` on one side.
2. `both_sides_to_the_same_value_is_same`: title changed identically on both sides gives `Same`. `changed()` is
   empty and `resolve(Theirs) == resolve(Mine) == theirs`.
3. `resolve_takes_the_chosen_side_for_conflicts_only`: title conflicts and body is mine-only. `resolve(Theirs)` has
   theirs' title and mine's body; `resolve(Mine)` has mine's title and mine's body.
4. `resolve_mine_keeps_a_priority_only_theirs_changed`. **The D2 regression**: mine edits the title, theirs edits the
   priority, and `resolve(Mine)` has both.
5. `resolve_theirs_keeps_my_changes_that_do_not_conflict`: mine edits title and tags, theirs edits title.
   `resolve(Theirs)` has theirs' title and mine's tags.
6. `without_a_conflict_both_sides_resolve_alike`: `!has_conflict()` and `resolve(Mine) == resolve(Theirs)`.
7. `changed_lists_fields_in_form_order`: changes to body, kind and tags give `[Kind, Tags, Body]`.
8. `labels_are_the_form_row_labels`: the seven `SpecField::label`s and the four `FieldState::label`s.

### 1c. Conformance: `crates/htui-core/src/store/conformance.rs`

- `CASES`: append `"item_edit_reason_lands_in_revision",` after `"update_spec_columns_roundtrip",` (`:168`).
- `run_case`: append the arm `"item_edit_reason_lands_in_revision" => item_edit_reason_lands_in_revision(store).await,`
  after `:421`.
- Add `EditReason` to the file's `crate::model` import. Place the function after `update_spec_columns_roundtrip`
  (`:837-951`):

```rust
/// MOD-13 milestone 3 D9: an edit's `reason` is the revision's, on both stores (ANA-9 §4.2 step 3).
/// No trait reads a revision (plan C9), so the reason is read back as the ancestor of a deliberately
/// stale edit, `status_cas_keeps_version`'s trick (`:1056-1083`).
async fn item_edit_reason_lands_in_revision<S: WriteStore>(store: &S) {
    const CASE: &str = "item_edit_reason_lands_in_revision";
    // before = store.item(ids::HTUI_ANA_2) (`"{CASE}: read must not fail"` / `"…: the fixture item exists"`)
    // 1. update_item(before.id, before.version, title_patch("Resolved", EditReason::DivergenceResolution.as_str()))
    //    → Updated(resolved); resolved.version == before.version + 1.
    // 2. update_item(before.id, resolved.version, title_patch("After", "edited")) → Updated(after) at resolved.version + 1.
    // 3. update_item(before.id, resolved.version, title_patch("Stale", "edited")) → Diverged { head, ancestor }.
    //    ancestor.version == resolved.version
    //    ancestor.reason == "divergence_resolution"       (the literal: the spec's column text)
    //    ancestor.title == "Resolved"; ancestor.author_id == ids::USER; ancestor.box_id == Some(ids::BOX)
    //    head.version == after.version && head.title == "After"  (the stale edit wrote nothing)
}
```
Every message reads `"{CASE}: …"`. `title_patch` (`:550-558`) already fills `author_id: ids::USER` and
`box_id: Some(ids::BOX)`. The fixture is `HTUI_ANA_2`, whose Pg FKs all hold, and no row is created.

**Counts.**
- `crates/htui-core/tests/mem_store.rs:37`: change `120` to `121`. Append to the message (`:56-57`):
  `, and MOD-13 milestone 3's one for the edit reason (plan D9)`.
- `crates/htui-store/tests/pg_conformance.rs:24`: change `EXPECTED_CASES` to `121`. Extend the doc (`:17-23`) with
  `…makes it 120, and MOD-13 milestone 3's edit-reason case (plan D9) makes it 121.`. Change the message (`:31-32`) to
  `(121 since MOD-13 milestone 3's edit-reason case)`.

Validate (E2):
- `cargo test -p htui-core --all-features --lib -- item_merge item_spec`
- `cargo test -p htui-core --all-features --test mem_store`
- `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`
- `cargo check --workspace --all-features --all-targets` (E3)

## 2. Task 2, the worker (`htui`)

### 2a. `crates/htui/src/store_worker.rs`
- `StoreRequest::EditItem` (`:887-896`) gains a last field:
  ```rust
  /// The revision's reason: `Edited` until the form is rebased on a divergence's head, then
  /// `DivergenceResolution` (milestone 3 D6).
  reason: EditReason,
  ```
  Update the variant doc (`:887-888`) to "§7.2 at `expected_version`, only the changed columns (D5), with its reason
  (milestone 3 D6)." Add `EditReason` to the `htui_core::model::{…}` import (`:24-30`).
- `name()` (`:1017`) and `try_serve` (`:1676`) already use `{ .. }` and do not change.
- `StoreReply::ItemDiverged` (`:1300`): the doc becomes "An edit that missed its version: nothing was written; both
  sides and the fresh catalogue (milestone 3 D7)."

### 2b. `crates/htui/src/item_writes.rs`
- **`ItemDivergence`** (`:111-119`):
  ```rust
  /// A stale edit (milestone 2 D6): both sides and the project's catalogue, for milestone 3's view.
  pub struct ItemDivergence {
      /// The row as it is now.
      pub head: Item,
      /// The revision at the version the edit was made from: the spec's cross-check (ANA-9 §4.2
      /// step 3). The view renders the form's own opened item instead (milestone 3 D1).
      pub ancestor: ItemRevision,
      /// The catalogue the worker read to check the edit, with `item = Some(head)` (milestone 3
      /// D7): the rebased form opens on it, so a re-kinded or re-graphed head still has labels.
      pub context: ItemFormContext,
  }
  ```
  `Debug` (`:122-129`) adds `.field("context", &self.context)`. `ItemFormContext`'s own `Debug` prints the catalogue as
  counts and the item through `ItemDigest`, so no body or path is printed.
- **`serve` `EditItem` arm** (`:238-271`): destructure `reason` too. Bind the catalogue as `let mut context`.
  - `.update_item(*id, *expected_version, changes.into_patch(me, box_id, *reason))` replaces T1's `EditReason::Edited`.
  - The `Diverged` arm becomes:
    `{ context.item = Some(head.clone()); Ok(StoreReply::ItemDiverged(Box::new(ItemDivergence { head, ancestor, context }))) }`.
  - The `check_changes(changes, &context.spec_context())` borrow ends before this, because it returns an owned
    `SpecChanges`.
- Module doc (`:21-23`): the D6 paragraph becomes "**A stale edit is never written over the head** (milestone 2 D6).
  [`UpdateOutcome::Diverged`] answers [`StoreReply::ItemDiverged`] with both sides and the catalogue just read
  (milestone 3 D7). The token is the request's own and nothing here moves it. The request carries its revision reason
  (milestone 3 D6): `divergence_resolution` once the form is rebased on a head."
- Test imports (`:324-333`): add `EditReason`.

### 2c. Mechanical compile fixes (F1, E1), the only other sites
A Gortex text search over every target for `EditItem {` and `ItemDivergence {` finds these:

| Site | Fix |
|---|---|
| `item_writes.rs:401` test `edit()` | `reason: EditReason::Edited` |
| `item_writes.rs:634` destructure | `let ItemDivergence { head: answered, ancestor, .. } = *divergence;` |
| `item_writes.rs:842` test constructor | `context: context.clone()` (the `context` built at `:835` moves into the reply later) |
| `ui/tabs/backlog/item_form.rs:570` constructor | `reason: EditReason::Edited` (T3 replaces it with `self.reason`); import `EditReason` (`:26-28`) |
| `item_form.rs:1091-1095` exhaustive test pattern | add `reason: EditReason::Edited,` |
| `ui/tabs/backlog/mod.rs:2020` exhaustive test pattern | `StoreRequest::EditItem { id, expected_version: 1, changes, reason: htui_core::model::EditReason::Edited }` |
| `tests/item_writes_pg.rs:106` `retitle()` | `reason: EditReason::Edited`; import `EditReason` (`:17`) |

`item_form.rs:1145` and `backlog/mod.rs:2219` already end in `..` (E1). `store_worker.rs:1017`/`:1676` use `{ .. }`.
`backlog/mod.rs:452` and `tests/item_writes_pg.rs:165-171` only read fields.

### 2d. Task 2 tests (`item_writes.rs` `mod tests`)
Add a helper `fn resolve(expected_version: i32, changes: SpecChanges) -> StoreRequest`: `edit()` with
`reason: EditReason::DivergenceResolution`.
1. `a_resolution_lands_with_reason_divergence_resolution`:
   - `store.update_item(HTUI_ANA_2, 1, theirs)` (the `:584-601` patch shape) moves the head to v2.
   - `serve(edit(1, retitle("Mine")))` answers `ItemDiverged`.
   - `serve(resolve(2, retitle("Resolved")))` answers `Edited { version: 3 }`.
   - `store.update_item(HTUI_ANA_2, 3, …)` moves the head to v4.
   - `serve(edit(3, retitle("Stale")))` answers `ItemDiverged` with `ancestor.version == 3`,
     `ancestor.reason == "divergence_resolution"`, `ancestor.title == "Resolved"` and `ancestor.author_id == ids::USER`.
2. `a_stale_edit_carries_the_fresh_catalogue_and_the_head` (D7):
   - `serve(edit(1, retitle("First")))`, then create a non-override graph `"after the form"` in `PROJECT_HTUI` (the
     `:524-534` `NewStepGraph` shape, `is_override: false`).
   - `serve(edit(1, retitle("Second")))` answers `ItemDiverged(d)`.
   - Assert: `d.context.project == PROJECT_HTUI`; `d.context.item.as_ref() == Some(&d.head)`; the new graph's id is in
     `d.context.graphs` and every graph there is non-override; and the kind ids equal
     `store.item_kinds(PROJECT_HTUI)`'s.
3. Extend `form_and_divergence_debug_print_no_body_and_no_paths` (`:815-855`): the divergence carries
   `context: context.clone()`, whose `item` holds `SECRET-BODY` and `secret/dir/**`. The existing loop over both
   replies already asserts that the three sentinels are absent. Add `assert!(printed.contains("kinds"))` for the
   `ItemDiverged` reply.

Validate (E2): `cargo test -p htui --all-features --lib -- item_writes store_worker`;
`cargo check -p htui --all-features --all-targets`.

## 3. Task 3, the view and the form wiring (`htui` UI)

### 3a. `crates/htui/src/ui/tabs/backlog/divergence.rs` (new; `pub mod divergence;` between `pub mod detail;` and `pub mod filter;`, `backlog/mod.rs:19-20`)
Everything is `pub` with a doc line, the `filter.rs`/`item_form.rs` precedent: the view commit must not trip
`dead_code` before the wiring commit.

The module doc covers D3–D5:
- The view is a mode of the item form.
- It draws over the whole tab area.
- Its ancestor is the item the form opened on (D1), not the reply's `ItemRevision`, which has no history for four of
  the seven columns.
- Rows list only the fields that are not `Same`.
- Body and paths show as two unified diffs side by side.
- Redaction: rows hold summaries, never body or path text, and `Divergence`'s `Debug` is hand-written.

Imports:
- `crossterm::event::{KeyCode, KeyEvent}`
- `htui_core::model::item_merge::{self, FieldState, Side, SpecField, SpecMerge}`
- `htui_core::model::item_spec::{self}`
- `htui_core::model::{Item, ItemKindId, ItemSpec, StepGraphId}`
- `ratatui::{Frame, layout::{Constraint, Layout, Rect}, text::{Line, Span}, widgets::{Block, Borders, Paragraph, Wrap}}`
- `crate::item_writes::{ItemDivergence, ItemFormContext}`
- `crate::ui::{Theme, diff}`
- `crate::ui::tabs::backlog::{filter, list::clip}`
- `super::item_form::notice_lines` (made `pub(super)`, §3b)

```rust
/// The hint's pick part while a field conflicts (D4).
pub const HINT_SIDES: &str = "t theirs wins  m mine wins";
/// The hint's pick part with no conflict (D5): `m` and `t` give the same spec.
pub const HINT_NO_CONFLICT: &str = "no conflicts \u{2014} m or t continues";
/// Shown when both body and paths differ.
pub const HINT_TAB: &str = "Tab body/paths";
/// Always last.
pub const HINT_BACK: &str = "Esc back";
/// Width of the label column with its trailing gap (`priority` + 2).
pub const LABEL_WIDTH: usize = 10;
/// Width of the state column with its trailing gap (`conflict` + 2).
pub const STATE_WIDTH: usize = 10;
/// Rows the diffs keep at least when a body or paths row is shown.
pub const DIFF_MIN: u16 = 6;
/// Rows `PageUp`/`PageDown` move (E7: `detail::PAGE`'s value).
pub const PAGE: usize = 10;

/// D4: the block title, `KEY  v{ancestor} → theirs v{head}`.
#[must_use] pub fn header(key: &str, ancestor: i32, head: i32) -> String;   // "{key}  v{ancestor} \u{2192} theirs v{head}"
/// D5: the form's notice after `Esc` in the view; its token and text are unchanged.
#[must_use] pub fn still_behind(head: i32) -> String;                       // "still behind v{head}; Ctrl+S compares again"
/// D5: the rebased form's notice; the token is now `head`.
#[must_use] pub fn rebased_on(head: i32) -> String;                         // "rebased on v{head}; Ctrl+S saves the resolution"

/// Which text the two diffs show (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffPane { Body, Paths }

/// One listed field: its state and three short values. Never body or path text: those two rows
/// hold counts (`3 lines`, `2 paths`), and the diffs show the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row { pub field: SpecField, pub state: FieldState, pub ancestor: String, pub theirs: String, pub mine: String }

/// What one key did in the view, for the form to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewOutcome {
    /// Scrolled, switched or swallowed.
    Stay,
    /// A chord (`filter::CHORD`) other than Ctrl+S: the form passes it.
    Pass,
    /// `Esc`: back to the form unchanged (D5).
    Back,
    /// `t` or `m` (D5).
    Resolve(Side),
}

/// The open three-way view (D3).
pub struct Divergence {
    context: ItemFormContext,   // the reply's catalogue, item = Some(head) enforced in `open`
    key: String,
    ancestor_version: i32,      // the form's token (D1)
    head_version: i32,
    merge: SpecMerge,
    rows: Vec<Row>,
    pane: Option<DiffPane>,     // None when body and paths are both Same
    scroll: u16,
}
```
`impl core::fmt::Debug for Divergence` is hand-written. It prints `key`, `ancestor_version`, `head_version`,
`rows` as `(field, state)` pairs, `pane` and `scroll`, then `finish_non_exhaustive()`. It never prints `merge` or a
row's values: the `ItemForm` E10 rule.

```rust
impl Divergence {
    /// D1, D2: `ancestor` is the item the form opened on and `catalogue` the form's own (the
    /// label fallback for a kind or graph the fresh one lacks); `mine` is the form's typed spec.
    /// `divergence.context` (D7) becomes the view's, with `item = Some(head)` set here as well.
    #[must_use]
    pub fn open(ancestor: &Item, catalogue: &ItemFormContext, mine: ItemSpec, divergence: &ItemDivergence) -> Self;
    pub fn on_key(&mut self, key: KeyEvent) -> ViewOutcome;
    #[must_use] pub fn rows(&self) -> &[Row];
    #[must_use] pub const fn pane(&self) -> Option<DiffPane>;
    #[must_use] pub const fn scroll(&self) -> u16;
    #[must_use] pub const fn head_version(&self) -> i32;
    #[must_use] pub const fn ancestor_version(&self) -> i32;
    #[must_use] pub const fn merge(&self) -> &SpecMerge;
    /// D4/D5: `HINT_SIDES` or `HINT_NO_CONFLICT`, then `HINT_TAB` when both body and paths
    /// differ, then `HINT_BACK`, joined by two spaces. With a conflict and both texts that is
    /// exactly D4's `t theirs wins  m mine wins  Tab body/paths  Esc back`.
    #[must_use] pub fn hint(&self) -> String;
    /// The rebased form's inputs: the view's catalogue (item = head) and `merge.resolve(side)`.
    #[must_use] pub fn resolve(self, side: Side) -> (ItemFormContext, ItemSpec);
}
/// D4: the view over the whole tab `area`.
pub fn render(frame: &mut Frame<'_>, area: Rect, view: &Divergence, theme: &Theme);
```
**`open`**:
- `theirs = ItemSpec::of(&divergence.head)`, `ancestor_spec = ItemSpec::of(ancestor)`,
  `merge = item_merge::merge(&ancestor_spec, &theirs, &mine)`.
- `rows`: one `Row` per `merge.changed()`, in `SpecField::ALL` order. Each value is drawn as follows:
  - kind: `"{prefix} {name}"`, looked up in `divergence.context.kinds`, then `catalogue.kinds`, else `?`;
  - title: as stored;
  - priority: `to_string()`;
  - tags: `item_spec::tags_text`, or `(none)` when empty;
  - graph: `kind default` for `None`, else the name looked up in the fresh graphs, then the old ones, else
    `override (this item's)` (the picker's word, `item_form.rs:695`);
  - paths: `"{n} path"`/`"{n} paths"`;
  - body: `"{n} line"`/`"{n} lines"` (`str::lines().count()`).
- `pane`: `Some(Body)` when body is not `Same`, else `Some(Paths)` when paths is not `Same`, else `None`.
- `scroll: 0`, `key = divergence.head.key.clone()`, `ancestor_version = ancestor.version`,
  `head_version = divergence.head.version`.

**`on_key`**, in this order:
1. Ctrl+S (the form's `ctrl_s` rule, `item_form.rs:215-218`; made `pub(super)`, §3b) gives `Stay`: it does nothing in
   the view (D5).
2. `key.modifiers.intersects(filter::CHORD)` gives `Pass`.
3. `Esc` gives `Back`; `Char('t')` gives `Resolve(Side::Theirs)`; `Char('m')` gives `Resolve(Side::Mine)`.
4. `Tab`/`BackTab` switch `pane` between `Body` and `Paths` only when both are not `Same`, and reset `scroll` to 0.
5. `Char('j') | Down` +1, `Char('k') | Up` -1, `PageDown` +`PAGE`, `PageUp` -`PAGE`. The new offset is clamped to
   `max(lines(theirs diff), lines(mine diff)) - 1` (E7). Without a pane, scrolling is a no-op.
6. Anything else gives `Stay` (swallowed).

**The diffs** are computed on demand by a private `fn diffs(&self) -> Option<(String, String)>`, so no body text is
stored beside the merge:
- For `Body` the texts are the three `body`s; for `Paths` they are `item_spec::paths_text(..)` of the three.
- The pair is
  `(diff::unified(a, t, &format!("ancestor v{av}"), &format!("theirs v{hv}")), diff::unified(a, m, &format!("ancestor v{av}"), "mine"))`.
- Texts are raw: a body with no trailing newline shows `similar`'s `\ No newline at end of file`. That is honest and
  expected in the snapshot.

**Render** at 100x30. The tab `area` is `chrome(..).body`, 100x27:
- A `Block::new().borders(Borders::ALL).title(format!(" {} ", header(..)))`, inner 98x25.
- `Layout::vertical` over the inner rect:
  - `[0]` column headings, `Length(1)`, dim: `"field"` padded to `LABEL_WIDTH`, `"state"` padded to `STATE_WIDTH`,
    then `ancestor v{av}`, `theirs v{hv}` and `mine`, each padded to `vw` with one space between.
    `vw = (inner.width - LABEL_WIDTH - STATE_WIDTH - 2) / 3`, which is 25 at 98.
  - `[1]` the table, `Length(table_h)`.
  - `[2]` the diff section row, `Length(1)`, or `Length(0)` without a pane. Its left half reads
    `"{label}  ancestor \u{2192} theirs"` and its right half `"{label}  ancestor \u{2192} mine"`, aligned on the two
    diff columns.
  - `[3]` the diffs, `Min(0)`.
  - `[4]` the hint, `Length(1)`, dim and clipped to the width.
- **Table rows.** Each row's three values wrap with `notice_lines(value, vw)`: at spaces, and inside a word only when
  the word alone is wider. The short fields wrap, never clip (D4). A row's height is the most lines of its three
  values, at least 1.
  - The first line carries the label, padded to `LABEL_WIDTH`, and the state label, padded to `STATE_WIDTH` and styled
    `theme.error` for `conflict`, `theme.accent` for `theirs`/`mine`.
  - Continuation lines pad those columns with spaces.
  - Each value cell is padded to `vw` with one space between cells, `theme.base`.
- **Row budget.** `rest = inner.height - 2` (headings and hint), 23 at 100x30.
  - With a pane, `table_h = min(sum_of_row_heights, rest - 1 - DIFF_MIN)`, so up to 16 rows, and the diffs take
    `rest - 1 - table_h` rows (at least 6).
  - Without a pane, `table_h = min(sum, rest)`.
  - Rows past `table_h` are cut, and the last visible line is replaced by `…` when anything was cut.
  - A view with zero rows (E5) draws the headings, an empty table and the hint.
- **Diffs.** `Layout::horizontal([Fill(1), Length(1), Fill(1)])` gives 48 | 1 | 49 at 98 columns. Each side is
  `Paragraph::new(diff::lines(&unified, theme)).wrap(Wrap { trim: false }).scroll((view.scroll, 0))`. The 1-column gap
  holds `\u{2502}` on every row, `theme.dim`.
- Every state shows as characters (`conflict`, `theirs`, `mine`, diff gutters), because snapshots are text.

### 3b. `crates/htui/src/ui/tabs/backlog/item_form.rs`
- **Module doc** (`:17-18`): the D6 bullet becomes "**A stale save opens the three-way view** (milestone 3 D3–D5,
  [`super::divergence`]). `m`/`t` rebase the form on the head, with a new token and reason
  `divergence_resolution`. `Esc` returns to it unchanged."
- **Fields** (`ItemForm`, `:129-156`), appended:
  ```rust
  /// The revision reason a save sends: `Edited`, or `DivergenceResolution` once rebased (D6).
  reason: EditReason,
  /// The open three-way view (D3): while `Some`, every key and paste goes to it, not the fields.
  resolving: Option<Divergence>,
  ```
  `build` sets `reason: EditReason::Edited, resolving: None`. `Debug` (`:182-203`) adds `.field("reason", &self.reason)`
  and `.field("resolving", &self.resolving.as_ref().map(Divergence::head_version))`.
- **`Texts`** (`:206-212`) becomes owned (`title`, `priority`, `tags`, `paths`, `body: String`), with two private
  constructors:
  - `fn blank() -> Self`: priority `"0"`, the rest empty (`open_new`).
  - `fn of(spec: &ItemSpec) -> Self`: the `open_edit` body at `:271-283`, through `to_string`/`tags_text`/`paths_text`.
- **`build`** (`:294-332`) becomes
  `fn build(context, projects, kind, graph, opened: &Texts, shown: &Texts) -> Self`. The widgets are built from
  `shown`. `Opened` is read back from throwaway widgets built from `opened` (`TextField::with_text(..).text()`,
  `TextArea::with_text(..).text()`), so the `\r` normalisation (`text_area.rs:94-104`) applies to both sides alike.
  `open_new` passes `&Texts::blank()` twice and `open_edit` `&Texts::of(&spec)` twice. Their behaviour is unchanged.
- **New public API**:
  ```rust
  /// D5: the form rebased on the head after `m`/`t`. `opened` holds the head's texts, so A4's
  /// "unchanged keeps the stored value" now means "unchanged from the head"; the widgets hold
  /// `resolved`'s texts; kind and graph are `resolved`'s; the token is `head.version`; the reason
  /// `DivergenceResolution`; focus Title; the notice `rebased_on(head.version)`. `None` when
  /// `context.item` is `None` (`open_edit`'s rule).
  #[must_use] pub fn open_resolution(context: ItemFormContext, resolved: &ItemSpec) -> Option<Self>;
  /// The compare-and-set token: `context.item`'s version; `None` on a new form.
  #[must_use] pub fn token(&self) -> Option<i32>;
  /// The reason the next save sends.
  #[must_use] pub const fn reason(&self) -> EditReason;
  /// The open three-way view, if any (the tab draws it over the whole area, D4).
  #[must_use] pub const fn resolving(&self) -> Option<&Divergence>;
  /// D1, D3: a divergence the tab matched to this form's save opens the view. `mine` is
  /// `typed_spec()`, which `busy` kept equal to what was sent. On an `Err` (unreachable: the sent
  /// spec passed it), the form settles with that sentence instead. Busy clears, the notice clears.
  pub fn open_divergence(&mut self, divergence: &ItemDivergence);
  ```
  `open_divergence` calls `Divergence::open(item, &self.context, mine, divergence)` with
  `item = self.context.item.as_ref()`. On a new form, which cannot be `Editing`, it returns without effect.
- **`on_key`** (`:384`). Add the first branch, ahead of the Ctrl+S check:
  `if self.resolving.is_some() { return self.on_view_key(key); }`. The private `on_view_key`:
  - `Stay` gives `Stay`; `Pass` gives `Pass`.
  - `Back`: `self.notice = Some(still_behind(view.head_version())); self.resolving = None;`, then `Stay`. The text,
    focus, token and reason are untouched (D5).
  - `Resolve(side)`: `let view = self.resolving.take()`; `let (context, resolved) = view.resolve(side)`. If
    `open_resolution(context, &resolved)` gives `Some(form)`, then `*self = form`. Answer `Stay`.
- **`on_paste`** (`:446`): the first line becomes `if self.busy.is_some() || self.resolving.is_some() { return; }`.
- **`request`** (`:547-579`): `reason: self.reason` replaces T2's `EditReason::Edited`. A `MintItem` has no reason.
- **Visibility**: `notice_lines` (`:853`) and `ctrl_s` (`:215`) become `pub(super)` for `divergence.rs`.
- **Remove** `item_changed_elsewhere` (`:867-873`). `NOTICE_HEIGHT`'s doc (`:59-60`) says "a notice (a refusal or
  `still_behind`) wraps to two" instead of "the D6 sentence".
- Imports: `EditReason`; `crate::item_writes::ItemDivergence`;
  `super::divergence::{Divergence, ViewOutcome, rebased_on, still_behind}`.

### 3c. `crates/htui/src/ui/tabs/backlog/mod.rs`
- `pub mod divergence;` (`:19-22`). Module doc (`:15-16`): "A stale edit opens the three-way view in the whole tab
  area (milestone 3). `m`/`t` rebase the form on the head, and Ctrl+S then lands a `divergence_resolution` revision."
- Imports (`:35-38`): drop `item_changed_elsewhere`.
- **`on_item_diverged`** (`:451-459`), with the D1 staleness check:
  ```rust
  /// Milestone 3 D1, D3: a stale edit opens the three-way view, only on the form whose save it
  /// answers: busy editing this item, and the reply's ancestor at this form's token. A late reply
  /// from an earlier token (a resolution has since moved it) is dropped, as one for another item is.
  fn on_item_diverged(&mut self, divergence: &ItemDivergence) {
      if let Some(form) = self.item_form.as_mut()
          && form.busy() == Some(Busy::Editing)
          && form.item_id() == Some(divergence.head.id)
          && form.token() == Some(divergence.ancestor.version)
      {
          form.open_divergence(divergence);
      }
  }
  ```
- **`render`** (`:690`): the first lines are
  ```rust
  // MOD-13 milestone 3 D4: the divergence view takes the whole tab area, list pane included.
  if let Some(view) = self.item_form.as_ref().and_then(ItemForm::resolving) {
      divergence::render(frame, area, view, ctx.theme);
      return;
  }
  ```
  Nothing else changes, so every existing frame is identical when no view is open.
- `on_key`, `on_paste`, the reveal guard (`:735`) and `on_scope_change` (`:539-540`) do not change. The form owns the
  view (D3).

### 3d. Task 3 tests
**`divergence.rs` `mod tests`.** Fixtures:
- `MemStore::demo()`, the edit context from `item_writes::serve(ItemForm { PROJECT_HTUI, Some(HTUI_ANA_1) })`.
- A divergence produced for real: `store.update_item(HTUI_ANA_1, 1, patch)`, then
  `item_writes::serve(EditItem { expected_version: 1, … })`, unwrapping `ItemDiverged`.
- `mine` built from `ItemSpec::of(&ancestor)` with fields replaced.

Tests:
1. `only_the_fields_that_moved_are_rows_each_with_its_state`: theirs changes title and priority, mine changes title and
   tags. The rows are `[(Title, Conflict), (Priority, Theirs), (Tags, Mine)]`.
2. `kind_and_graph_rows_read_through_the_catalogue`: theirs re-kinds to `KIND_HTUI_FEAT` and sets
   `GRAPH_HTUI_FEAT`. The kind row's theirs value is `"{prefix} {name}"` of that kind, and the graph row's ancestor
   value is `kind default`.
3. `a_body_conflict_draws_both_diffs`: drawn at `chrome(100x30).body`, the text contains `--- ancestor v1`,
   `+++ theirs v2`, `+++ mine` and `conflict`, and every line is ≤ 100 cells.
4. `tab_switches_to_paths_only_when_both_differ`: with only body differing, `Tab` keeps `Body`. With both, `Tab` gives
   `Paths`, `BackTab` gives `Body`, and `scroll` resets to 0.
5. `scrolling_is_clamped_to_the_longer_diff`: `PageDown` ×5 stops at `lines - 1`; `k` at 0 stays 0.
6. `the_hint_follows_the_conflicts`: with a conflict and both texts it is exactly
   `"t theirs wins  m mine wins  Tab body/paths  Esc back"`. Without a conflict it starts with `HINT_NO_CONFLICT`.
7. `the_longest_hint_fits_the_tab_area`: ≤ `chrome(Rect::new(0, 0, 100, 30)).body.width - 2` (98).
8. `a_long_title_wraps_inside_its_column`: a 70-character theirs title. The joined, trimmed rows contain it whole, and
   no row is wider than the area.
9. `keys_map_to_outcomes`: `Esc` gives `Back`; `t`/`m` give `Resolve`; `ctrl('s')` gives `Stay`; `ctrl('c')` gives
   `Pass`; `x`/`Enter` give `Stay`.
10. `debug_prints_no_body_and_no_paths`: the bodies `SECRET-BODY`/`SECRET-THEIRS` and the paths `secret/dir/**` are
    absent from `{view:?}` and `{view:#?}`, and `key` is present.

**`item_form.rs` `mod tests`** (reuse `context`, `edit_form`, `patch`, `type_text`, `ctrl`). Add a helper
`async fn diverged(store, form) -> ItemDivergence`: Ctrl+S, `item_writes::serve` of the `Save` request, then unwrap
`ItemDiverged`.
1. `open_resolution_opens_on_the_head_with_the_resolved_text`:
   - `token() == Some(head.version)`, `reason() == DivergenceResolution` and
     `notice() == Some(rebased_on(head.version))`.
   - `opened.title == head.title` and `title.text() == resolved.title`.
   - An unchanged Ctrl+S on a resolved spec equal to the head is `NOTHING_TO_SAVE`.
2. `m_takes_mine_for_conflicts_and_theirs_elsewhere` (D2, D5): theirs retitles and re-prioritises (7), mine retitles.
   After `open_divergence` and then `m`, Ctrl+S answers
   `Save(EditItem { expected_version: 2, reason: DivergenceResolution, changes: SpecChanges { title: Some(mine), ..Default } })`.
3. `t_takes_theirs_for_conflicts_and_keeps_my_other_changes`: theirs retitles, mine retitles and sets priority 9. After
   `t`, the changes are `{ priority: Some(9) }` only.
4. `t_with_every_change_conflicting_has_nothing_to_save` (D8): Ctrl+S gives `Stay`, the notice is `NOTHING_TO_SAVE`
   and `busy` is `None`.
5. `my_edit_already_in_the_head_has_nothing_to_save` (E5): theirs and mine set the same title. `m` and Ctrl+S give
   `NOTHING_TO_SAVE`.
6. `esc_in_the_view_returns_to_the_form_unchanged`:
   - `resolving()` is `None`, `token() == Some(1)`, `reason() == Edited`, the title still ends in ` mine`, and the
     notice is `still_behind(2)`.
   - The next Ctrl+S answers `EditItem { expected_version: 1, reason: Edited, .. }`.
   - Then, after `settle(None)` (to simulate the reply), `Esc` gives `Cancel`.
7. `the_rebased_pickers_label_a_kind_and_graph_only_the_new_catalogue_has` (D7):
   - After the form opened, create a non-override graph and a kind in `PROJECT_HTUI` (`NewItemKind`, prefix `NEW`,
     default graph that graph).
   - Theirs moves `ANA-1` to both. Mine retitles. Then `t`.
   - `picker_label(Field::Kind) == "NEW {name}"` and `picker_label(Field::Graph) == graph.name` (not `?` or
     `override`).
8. `the_view_swallows_letters_and_ctrl_s_but_passes_ctrl_c`: while resolving, `x`, `Tab` and `ctrl('s')` give
   `Stay`, `ctrl('c')` gives `Pass`, and the title text is unchanged.
9. `a_paste_while_the_view_is_open_is_dropped`.
10. `debug_prints_no_body_while_resolving` (the `debug_prints_no_body` shape, with the view open).

**`backlog/mod.rs` `mod tests`** (reuse `Bench`, `open_with`, `saved`, `served`, `notice`, `errors`, `drawn`,
`type_into`):
1. Rewrite `a_diverged_edit_keeps_the_form_its_text_and_its_token` (`:2182-2224`, milestone 2 D6) as
   `a_diverged_edit_opens_the_view_and_esc_keeps_the_text_and_token`:
   - Set up as now. After the `ItemDiverged` reply, `tab.item_form.as_ref().and_then(ItemForm::resolving).is_some()`
     holds, and there is no `Action::Error`.
   - `press(Esc)`: the view is closed and `notice(&tab) == Some(still_behind(2))`.
   - `saved` gives `EditItem { expected_version: 1, reason: Edited, changes }` with the title ending in ` mine`.
2. `a_divergence_for_another_item_or_token_is_dropped`: take a real `ItemDivergence` and clone it twice. One copy gets
   `ancestor.version = 7`; the other gets `head.id = ItemId::new()`. Each `on_reply` leaves `resolving()` `None` and
   `busy() == Some(Editing)`.
3. `m_rebases_and_ctrl_s_sends_a_resolution_at_the_head` (D5, D6): after the view, `m`; `saved` gives
   `EditItem { expected_version: 2, reason: DivergenceResolution, changes: { title } }`. Serving it answers
   `Edited { version: 3 }`, and `on_reply` closes the form.
4. `a_second_divergence_after_a_rebase_reopens_the_view` (D8): after `m`, the store moves the head to v3; Ctrl+S at 2
   answers `ItemDiverged` with `ancestor.version == 2 == token`. The view is open again with `ancestor_version() == 2`
   and `head_version() == 3`.
5. `the_view_takes_the_whole_tab_area` (D4):
   - `drawn(&tab, &bench)` contains `header("ANA-1", 1, 2)`.
   - It contains no list-only text: the key of another listed item, `bench.items[1].key`, and the list's project
     header slug are absent.
   - After `Esc`, the frame contains ` Edit ANA-1 (v1) ` again.
6. `a_scope_change_closes_the_open_view`: `on_scope_change` gives `item_form.is_none()`.
7. `a_reveal_while_the_view_is_open_asks_to_close_it_first` (the `:2505` shape).

Validate: `cargo test -p htui --all-features --lib backlog`.

## 4. Task 4, integration, snapshot and Pg parity

### 4a. `crates/htui/tests/backlog.rs`
Add a section header `// Divergence (MOD-13 milestone 3, plan D1-D9)`, and a constant
`TITLE_TO_BODY: [&str; 5] = ["tab"; 5]` (Priority, Tags, Graph, Paths, Body). Reuse `backlog_over`, `keys`,
`type_text`, `detail_pane`, `retitled` and `ANA_1_TITLE`.
1. Rewrite `a_stale_edit_keeps_the_form_and_never_overwrites_the_head` (`:1662-1720`) as
   `a_stale_edit_opens_the_view_and_m_lands_mine_at_version_3`:
   - `e`; the clone `retitled("Theirs")` at 1; type ` mine`; `ctrl-s`.
   - The frame holds `ANA-1  v1 \u{2192} theirs v2`, `Theirs`, `conflict` and `mine`. The head is `("Theirs", 2)`.
   - `m`: the frame holds ` Edit ANA-1 (v2) ` and `rebased on v2`.
   - `ctrl-s`: the head is `(format!("{ANA_1_TITLE} mine"), 3)`. The detail pane holds `version 3` (E4) and no
     ` Edit `. `status == None`.
2. `a_resolution_keeps_their_priority_and_my_title` (D2 end to end):
   - The clone sets `priority: Some(9)` at 1; `e`, type ` mine`, `ctrl-s`. The frame holds `priority` and `theirs`.
   - `m`, `ctrl-s`: the head has priority 9, title `"{ANA_1_TITLE} mine"` and version 3.
3. `esc_from_the_view_keeps_the_text_and_the_head`:
   - After the view: `esc`. The frame holds ` Edit ANA-1 (v1) `, `still behind v2` and `{ANA_1_TITLE} mine` (the title
     row may scroll inside `TextField`, so assert `mine` in the frame). The head is `("Theirs", 2)`.
   - `ctrl-s`: the view is back (`theirs v2`) and the head is unchanged.
   - `esc`, `esc`: the form is closed (no ` Edit `).
4. Snapshot `the_divergence_view_renders_over_the_whole_tab`:
   - The clone writes `title: "Theirs"` and `body: "Their body."` at 1.
   - `e`, type ` mine`, `TITLE_TO_BODY`, type `Mine first. ` (the body cursor starts at byte 0, so it prepends),
     `ctrl-s`.
   - Assert: the header, `conflict` (title and body), `--- ancestor v1`, `+++ theirs v2`, `+++ mine`, and the hint
     `t theirs wins  m mine wins  Esc back`. There is no `Tab body/paths`, because paths are `Same`.
   - `insta::assert_snapshot!("item_divergence", frame)` creates `tests/snapshots/backlog__item_divergence.snap`.

All frames are 100x30. Accept with `cargo insta accept`. **No existing `backlog__*.snap` may change.**

### 4b. `crates/htui/tests/item_writes_pg.rs`
Import `EditReason`. Add a helper `fn resolution(title: &str, expected: i32) -> StoreRequest`: `retitle` with
`reason: EditReason::DivergenceResolution`. Extend `mint_edit_and_a_stale_edit_on_postgres` after `:165-178`:
- `divergence.context.project == ids::PROJECT_HTUI`;
- `divergence.context.item.as_ref() == Some(&divergence.head)`;
- `!divergence.context.kinds.is_empty() && !divergence.context.graphs.is_empty()`.
- `written(serve(resolution("Resolved", 2)))` gives `Edited { version: 3 }`.
- `written(serve(retitle("After", 3)))` gives `Edited { version: 4 }`.
- `serve(retitle("Stale", 3))` gives `ItemDiverged(d)` with `d.ancestor.version == 3`,
  `d.ancestor.reason == "divergence_resolution"` and `d.ancestor.title == "Resolved"`.
- The head is `("After", 4)`.

Update the file doc (`:1-7`) for milestone 3 (the reason and the catalogue).

Validate: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`;
`cargo test -p htui --all-features --test item_writes_pg -- --test-threads=1`.

## 5. Commit boundaries
Implementers commit at each green. Stage explicit paths: no `-A`, no stash, no amend.
1. `feat(mod-13): EditReason and the revision reason on into_patch`: `model/item_spec.rs`, `model/mod.rs`,
   `htui/src/item_writes.rs` (the `:255` line only, E3).
2. `feat(mod-13): item_merge, the field-level three-way merge`: `model/item_merge.rs`, `model/mod.rs`.
3. `test(mod-13): item_edit_reason_lands_in_revision conformance case (121)`: `store/conformance.rs`,
   `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`.
4. `feat(mod-13): EditItem carries its reason, ItemDiverged its catalogue`: `store_worker.rs`, `item_writes.rs`, and
   the §2c lines in `item_form.rs`, `backlog/mod.rs` and `tests/item_writes_pg.rs`.
5. `feat(mod-13): the divergence view`: `backlog/divergence.rs`, `pub mod divergence;`, and the
   `notice_lines`/`ctrl_s` visibility in `item_form.rs`.
6. `feat(mod-13): a stale edit opens the view; m and t rebase the form`: `item_form.rs`, `backlog/mod.rs`.
7. `test(mod-13): divergence integration cases and snapshot`: `tests/backlog.rs`, the new `.snap`.
8. `test(mod-13): a divergence resolution on Postgres`: `tests/item_writes_pg.rs`.

Each commit passes `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
Every message ends with the `Co-Authored-By` line. Close the milestone with the plan's Validation block: run the full
gate with `--no-fail-fast -- --test-threads=1` and grep the output for `SIGABRT`.

## 6. Hazards beyond the errata
- **Lints.**
  - `#![warn(missing_docs)]` (`htui-core`, `htui` `lib.rs`) applies to every new `pub` item, variant and field.
  - `missing_debug_implementations` (workspace) needs a `Debug` on `SpecMerge`, `SpecField`, `FieldState`, `Side`,
    `EditReason`, `Divergence`, `Row`, `DiffPane` and `ViewOutcome`.
  - `unused_qualifications` is a workspace warning that `-D warnings` makes an error: do not write `crate::model::…`
    for an imported name.
  - `clippy::all` is on and pedantic is not (`Cargo.toml` `[workspace.lints.clippy]`). `build` grows to six
    parameters, under `too_many_arguments`' seven.
- **Redaction (milestone 2 E6/E10).** No `Debug` may print a body or path.
  - `SpecMerge` derives over `ItemSpec`, whose hand-written `Debug` prints lengths.
  - `Divergence` is hand-written, with rows as `(field, state)` only.
  - `ItemDivergence` adds `context` through `ItemFormContext`'s digest.
  - `Row` holds counts, never text, for body and paths. Keep that invariant in its doc.
  - Tests: §2d test 3, §3d `divergence` test 10, `item_form` test 10.
- **Key order.** In the view, Ctrl+S must be caught before the chord pass. Otherwise it passes, and the app or shell
  would see it. `ItemForm::on_key` must route to the view before its own Ctrl+S check, which would otherwise save.
- **`m` in the Backlog** is the list's "open graph" (`mod.rs:593`). The form guard (`:572-574`) sits ahead of it, so
  `m` reaches the view (C13).
- **The ancestor is the opened item (D1).** `typed_spec()` returns the stored, unparsed value for every untouched
  field (A4), so untouched fields equal the ancestor exactly and classify as `Same`/`Theirs`. Never build `mine` from
  the widgets' texts parsed whole.
- **`open_resolution` and A4.** `opened` is read back from widgets built on the head's texts, and `shown` from the
  resolved spec. A field resolved to the head's value therefore keeps the head's stored value unparsed, so a legacy
  head value never blocks the save. A field resolved to mine is re-parsed, and it parsed before.
- **No snapshot churn.** The view renders only while `resolving` is `Some`, and `render`'s early return is the only
  change on the draw path.
- **`htui-orch` stack headroom** is untouched: no orchestrator code changes. The full gate still runs `--no-fail-fast`
  and greps for `SIGABRT`.
- **Suite hygiene.** Run `--test-threads=1` (the keyring fake is process-wide). Run `--features testkit` or
  `--all-features`, otherwise `tests/*.rs` run 0 tests.
- **HANDOFF counts.** No request or reply variant is added, so `StoreRequest`/`StoreReply` counts do not move.

## 7. For the maintainer
Nothing blocks. The two small additions below follow the plan's sentence pattern (`pub fn` returning a `String`,
tested by text) and change no decision:
- `rebased_on(head)`, the rebased form's notice, so the user sees that the token moved before Ctrl+S.
- The body and paths rows show counts, with the diffs carrying the text, so a long body never floods the field table.
