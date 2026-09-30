# Blueprint: MOD-13 milestone 1, Backlog filters

This follows the binding design decisions D1–D6. Every anchor below was checked with Gortex. The plan has seven factual errors; they are listed first because Tasks 1, 3 and 4 depend on them.

## 0. Errata (plan vs. code)

| # | Plan claim | What the code shows | Consequence |
|---|---|---|---|
| E1 | "Demo data makes readiness parity non-trivial" (Verified claims, row 6) | **False.** htui `TOOL-1` [docker] is `awaiting_approval`, and vulkan `FEAT-1` [gpu,vulkan] is `in_progress` (`fixtures.rs` `ITEM_SPECS`, about :855-1063). Neither passes `ready`. Platform's ready set is `HTUI_ANA_2` [], `AGY_FEAT_1` [rust, probed] and `AGY_FIX_1` [] (its `blocked_by` points at done `AGY_ANA_1`, `links()` :1130). Graphics' ready set is `VULKAN_TOOL_1` [cmake, probed]. The capability half removes **nothing** in any demo workspace. | Parity tests must mint an open item with an unmet tag, as `mem.rs:10540` does: `needs_cuda`, `required_tags: ["cuda"]`, in `PROJECT_HTUI`. |
| E2 | Pattern table: `boxes.rs:467 submit` puts the refusal on the status line as `Action::Error` | `submit` puts it on the section's own `self.notice`, not `Action::Error`. | D3 still stands (the Backlog has no notice line, so `Action::Error` is right). Only the "mirror" attribution is wrong. |
| E3 | Empty state per D5: "No items match the filter." | `list::render` shows its empty text only when `lines.is_empty()`. `lines()` emits a header for every project in `ctx.projects`, even one with zero items, so the text shows only when the scope has no project. | The condition must be `lines.is_empty() \|\| (view.filter.is_some() && view.items.is_empty())`. The unfiltered path stays byte-identical. |
| E4 | Files table has no `app/mod.rs` | The help box is fed by tab-scope bindings in `register_all`. `m` has one (`app/mod.rs:117-124`), and `tests/backlog.rs::m_is_on_the_backlog_help_line` pins it. | Add `f`/`F` help bindings (Task 3). No `.snap` contains `open graph`, so no snapshot moves. |
| E5 | Plan puts the `Items` construction sites under later tasks | Adding the field breaks all seven sites at once, and Task 1's validate command compiles the whole lib. | Task 1's commit touches `store_worker.rs`, `app/update.rs` and `backlog/mod.rs` (the three `Items` sites only). |
| E6 | Risk row: a `box_info()` failure is "Pinned by a test" | `MemFault` covers three writes only (`mem.rs:113-120`), so there is no read-fault seam. | The `Failed` path holds by construction (`?`, like every other arm). Pin the pure helper instead. Adding a read fault would touch htui-core and is out of scope. |
| E7 | Form keys: "`j`/`k` move between rows … `x` clears" | `TextField::on_key` inserts every non-control `Char` (`text_field.rs:145`). | On the Tags row, `j k h l x space` are text. Rows move with `Up`/`Down`/`Tab`/`BackTab` there. `j`/`k`/`x`/`space`/`h`/`l` act only on the other rows. |

## 1. Worker composition (Task 1)

### `crates/htui/src/store_worker.rs`

**Variant** (`:115-121`). `StoreRequest` derives only `Debug, Clone`, with no `PartialEq`, so tests must use `matches!` or destructure.
```rust
/// The Backlog list.
Items {
    scope: Scope,
    /// Conjunctive filter; `ItemFilter::default()` means "everything in scope".
    filter: ItemFilter,
    /// MOD-13 D2: ANA-9 §7.4 for *this* box. The worker adds `ready: Some(true)` and keeps the
    /// rows whose `required_tags ⊆ probed_tags ∪ declared_tags` of `Backend::box_info()`;
    /// `None` (unregistered box) = no tags, as §7.4's `LEFT JOIN … COALESCE`.
    ready_here: bool,
},
```
`name()` at `:831` (`Self::Items { .. } => "items"`) and `spawn_with_concepts` at `:1817` (`Items { scope, .. }`) need no edit.

**Serve arm** (`:1431`):
```rust
StoreRequest::Items { scope, filter, ready_here } => {
    StoreReply::Items(read_items(backend, scope, filter, *ready_here).await?)
}
```
**Helpers**: two free, private functions placed directly after `try_serve`, before `fn failed`. `BoxInfo`, `ItemSummary` and `ItemFilter` are already imported at `:23-28`.
```rust
/// MOD-13 D2: `Items`, with this box's readiness composed in. Not `Backend::ready_items`, which
/// refuses offline (MOD-25: an offline box still browses).
async fn read_items(backend: &Backend, scope: &Scope, filter: &ItemFilter, ready_here: bool)
    -> StoreResult<Vec<ItemSummary>>
{
    if !ready_here {
        return backend.items(scope, filter).await;
    }
    let info = backend.box_info().await?;           // first: no item read when it fails
    let filter = ItemFilter { ready: Some(true), ..filter.clone() };
    Ok(runnable_here(backend.items(scope, &filter).await?, info.as_ref()))
}

/// The rows `info` has every required tag for, in the store's order; `None` has no tags.
fn runnable_here(rows: Vec<ItemSummary>, info: Option<&BoxInfo>) -> Vec<ItemSummary> {
    rows.into_iter()
        .filter(|row| row.required_tags.iter().all(|tag| info.is_some_and(|b|
            b.probed_tags.contains(tag) || b.declared_tags.contains(tag))))
        .collect()
}
```
This matches `MemStore::ready_items` (`mem.rs:700-719`). `retain`/`filter` keeps the store's order.

**The seven construction sites.** All get `ready_here: false` in Task 1. Two of them are later replaced in Task 3.

| Site | Task 1 | Task 3 |
|---|---|---|
| `app/update.rs:529` (`Recorder::wants_requests`, test) | `ready_here: false` | — |
| `app/update.rs:1220` (`Revealer::reveal`, test) | `ready_here: false` | — |
| `store_worker.rs:2592` (`serve_items_returns_the_scope_in_store_order`) | `ready_here: false` | — |
| `store_worker.rs:2864` (`every_items_request_publishes_its_scope_to_the_refresher`) | `ready_here: false` | — |
| `backlog/mod.rs:219` (`wants_requests`) | `ready_here: false` | becomes `self.filter.to_request(scope)` |
| `backlog/mod.rs:357` (`reveal`) | `ready_here: false` | becomes `self.filter.to_request(ctx.scope)` after the D4 clear |
| `backlog/mod.rs:545` (test `a_refused_items_read_disarms_a_pending_reveal`) | `ready_here: false` | — |

`update.rs:574` and `update.rs:1254` are `{ .. }` patterns and need no edit.

**Task 1 tests** (in the `store_worker.rs` `mod tests`, using `demo()`, `platform_scope()` and `serve()`). Add `use htui_core::model::NewItem; use htui_core::store::WriteStore as _; use htui_core::fixtures::demo_data;`.

- Shared setup `async fn demo_with_cuda() -> (MemStore, ItemId)`: `MemStore::demo()` plus `mint_item(NewItem { project_id: PROJECT_HTUI, kind_id: KIND_HTUI_FEAT, title: "needs a GPU toolchain", required_tags: vec!["cuda"], created_by: ids::USER, box_id: Some(ids::BOX), .. })`, copied from `mem.rs:10540`. Return a clone of the store (clones share state) and the id.
- `ready_here_equals_ready_items_for_this_box`: `backend = Backend::memory(store.clone())`. `serve(Items{platform, default, ready_here: true})` must `assert_eq!` `store.ready_items(&scope, ids::BOX)` as a whole `Vec<ItemSummary>` (same rows, same order). It must also contain `HTUI_ANA_2`, `AGY_FEAT_1` and `AGY_FIX_1`, and not `needs_cuda`. Non-triviality guard: `serve(Items{ItemFilter{ready:Some(true),..}, ready_here:false})` **does** contain `needs_cuda`.
- `ready_here_with_an_unregistered_box_keeps_only_untagged_items`: store `MemStore::from_demo(DemoData { this_box: None, ..demo_data() })`. The result equals `store.ready_items(&scope, BoxId::new())`, which is `[HTUI_ANA_2, AGY_FIX_1]`. `AGY_FEAT_1` [rust] is dropped.
- `ready_here_is_conjunctive_with_projects_tags_and_statuses` (demo): with `project_ids: [PROJECT_AGY]` the result is `{AGY_FEAT_1, AGY_FIX_1}`; with `tags: ["rust"]` it is `{AGY_FEAT_1}`; with `statuses: [Done]` it is empty.
- `ready_here_false_never_reads_the_box`: over the `this_box: None` store, `ready_here: false` with the default filter returns 11 rows, the same as `serve_items_returns_the_scope_in_store_order`.
- `runnable_here_keeps_order_and_treats_none_as_no_tags`: pure test on hand-built rows.

Validate: `cargo test -p htui --all-features --lib store_worker`.

## 2. `crates/htui/src/ui/tabs/backlog/filter.rs` (Task 2, created new)

The module line `pub mod filter;` goes in `backlog/mod.rs:8-9`, in this task's commit. Everything in the file is `pub` so that it does not trip `dead_code` before Task 3; `ui::tabs::backlog` is publicly reachable.

Imports: `htui_core::model::{ItemFilter, ProjectId, ProjectRef, Scope, Status, declared_tags_from_text}`, `crate::store_worker::StoreRequest`, `crate::ui::{FieldOutcome, TextField, Theme}`, `crossterm::event::{KeyCode, KeyEvent}`, and ratatui `Frame`, `Rect`, `Block`, `Borders`, `Paragraph`, `Line`, `Span`.

### `BacklogFilter`
```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BacklogFilter {
    pub statuses: Vec<Status>,    // empty = any; kept in Status::ALL order
    pub projects: Vec<ProjectId>, // empty = every project of the scope; kept in ctx.projects order
    pub tags: Vec<String>,        // canonical (declared_tags_from_text); empty = no capability filter
    pub ready_here: bool,
}
impl BacklogFilter {
    #[must_use] pub fn is_empty(&self) -> bool;                    // == Self::default()
    #[must_use] pub fn item_filter(&self) -> ItemFilter;           // empty Vec -> None; ready: None; text: None
    #[must_use] pub fn to_request(&self, scope: &Scope) -> StoreRequest
        // Items { scope: scope.clone(), filter: self.item_filter(), ready_here: self.ready_here }
    #[must_use] pub fn summary(&self, projects: &[ProjectRef]) -> Option<String>;
    pub fn retain_projects(&mut self, scope: &Scope);              // D6: projects.retain(|p| scope.contains(*p))
}
```
- The view never sets `ready`; the worker does (D2).
- An empty Vec must never be sent as `Some(vec![])`.
- `summary` takes `&[ProjectRef]` (not `()`, as the brief sketched) because names live in `ctx.projects`.
- **Summary format**: `None` if `is_empty()`. Otherwise the parts below, in this order, joined with `" · "`:
  - `status:` followed by the statuses' `as_str` joined with `,`
  - `project:` followed by the `ProjectRef.name` values joined with `,` (an id missing from `projects` is skipped)
  - `tags:` followed by the tags joined with `,`
  - `ready here`

  Example: `status:done,open · project:htui · tags:gpu,rust · ready here`.
- Statuses: `Status::ALL` exists. `str_enum!` generates `pub const ALL: &'static [Self]` in DDL order (`model/mod.rs:46`): `open, queued, in_progress, awaiting_approval, blocked, done, failed, closed`.

### Form
```rust
pub const FORM_HEIGHT: u16 = 7;          // 2 border + 4 rows + 1 hint
pub const HINT_CHOICES: &str = "space toggle · x clear · Enter apply · Esc cancel"; // 49 cols
pub const HINT_TAGS: &str = "comma list · ↑/↓ row · Enter apply · Esc cancel";      // 47 cols

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterRow { Status, Project, Tags, Ready }   // const ROWS: [FilterRow; 4] in that order

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome { Stay, Cancel, Apply(BacklogFilter), Refused(String) }

#[derive(Debug)]
pub struct FilterForm {
    draft: BacklogFilter,        // statuses / projects / ready_here being edited; draft.tags unused until Apply
    row: FilterRow,              // opens on Status
    status_at: usize,            // index into Status::ALL, opens at 0
    project_at: usize,           // index into `projects`, opens at 0
    projects: Vec<ProjectRef>,   // ctx.projects at open (the form is dropped on a scope change)
    tags: TextField,             // TextField::with_text(&filter.tags.join(", "))
}
impl FilterForm {
    #[must_use] pub fn open(filter: &BacklogFilter, projects: &[ProjectRef]) -> Self;
    pub fn on_key(&mut self, key: KeyEvent) -> FormOutcome;   // the tab has already filtered CONTROL chords
    pub fn on_paste(&mut self, text: &str);                   // Tags row: self.tags.on_paste; other rows: dropped
}
pub fn render(frame: &mut Frame<'_>, area: Rect, form: &FilterForm, theme: &Theme);
```
The form is pure and takes no `Ctx`, following `requirements/forms.rs`, where `FormOutcome` is answered to the tab.

**`on_key` contract.**
- **Tags row.** `self.tags.on_key(key)`:
  - `Submit` → `apply()`
  - `Cancel` → `Cancel`
  - `Pass` on `Up`/`BackTab` → previous row; `Pass` on `Down`/`Tab` → next row; both return `Stay`
  - `Consumed` and any other `Pass` → `Stay`
- **Other rows:**
  - `Enter` → `apply()`; `Esc` → `Cancel`
  - `j`, `Down` or `Tab` → next row; `k`, `Up` or `BackTab` → previous row. Clamped at both ends, no wrap.
  - `h`/`Left` and `l`/`Right` move the option cursor on Status (modulo 8) and on Project (modulo `projects.len()`; no-op when there are none). They are no-ops on Ready.
  - `space`:
    - Status toggles `Status::ALL[status_at]`, then re-sorts by `ALL` position.
    - Project toggles `projects[project_at].project_id`, then re-sorts by `projects` order.
    - Ready flips `ready_here`.
  - `x` resets the draft to default and clears the text (`self.tags.clear()`). It does **not** apply.
  - Everything else → `Stay`.
- **`apply()`**: `declared_tags_from_text(self.tags.text().unwrap_or_default())`. On `Err(s)` return `Refused(s)`; the form stays open. On `Ok(tags)` return `Apply(BacklogFilter { tags, ..draft.clone() })`.

**Render.** A `Block` with `Borders::ALL` and title `" Filter "`, drawn at the bottom of the list pane (§3 render). Inner lines, each prefixed with the marker `>` (focused row) or ` `, then the label padded to 8:
```
> status  ‹[x] done›  done, open        // ‹[check] ALL[status_at]›, two spaces, chosen list or "any"
  project ‹[ ] htui›  any               // "(none)" when projects is empty
  tags    <self.tags.line(remaining_width, row == Tags, theme)>
  ready   [x] only what this box can start
<HINT_TAGS on the Tags row, else HINT_CHOICES, in theme.dim>
```
Snapshots are text only, so every state must be visible as characters (`>`, `[x]`, `‹ ›`), not as style alone. Clip long lines with `list::clip`.

**Task 2 tests** (`#[cfg(test)] mod tests` in `filter.rs`, pure, no `Ctx`):
`an_empty_filter_is_the_default_read` (asserts `ItemFilter::default()` and `ready_here == false`, via `matches!`), `to_request_maps_every_field`, `summary_is_none_without_a_filter`, `summary_names_statuses_projects_tags_and_readiness` (the exact string above, using demo `ProjectRef`s built by hand or from `MemStore::demo()`), `retain_projects_drops_what_the_scope_lacks`, `j_and_k_move_between_rows_and_stop_at_the_ends`, `arrows_and_tab_leave_the_tags_row`, `h_and_l_move_within_the_status_options_and_wrap`, `space_toggles_a_status_in_all_order` (toggle done, then open, gives `[Open, Done]`), `space_toggles_a_project`, `space_toggles_ready_here`, `the_tags_row_types_every_letter` (`j k x space` all land in the text), `enter_applies_canonical_tags` (`" rust , gpu,rust"` gives `["gpu","rust"]`), `esc_cancels`, `x_clears_the_draft_without_applying`, `a_refused_tag_list_stays_open` (`"Rust"` gives `Refused(declared_tags_from_text("Rust").unwrap_err())`, and the next `Esc` still answers `Cancel`), `the_form_opens_on_the_active_filter`, `the_hints_fit_the_list_pane` (both hints ≤ `panes(chrome(100x30).body)[0]` inner width minus 2).

Validate: `cargo test -p htui --all-features --lib backlog::filter`.

## 3. `BacklogTab` (`backlog/mod.rs`, Task 3)

**New fields** (struct `:50-64`, `new()` `Self { .. }` about `:89-96`):
```rust
/// MOD-13 D1: the active filter; `default()` = the whole scope (D5).
filter: BacklogFilter,
/// The open filter form, capturing every key while `Some`.
form: Option<FilterForm>,
```
**Existing literals** must add `filter: BacklogFilter::default(), form: None`: the tests at `:460` (`a_capturing_sub_tab_gets_the_navigation_keys`), `:629` (`m_on_the_list_opens_the_graph`) and `:674` (`m_while_a_sub_tab_captures_is_the_sub_tab_s`).

**`on_key`** (`:242`), in this order:
1. `if self.form.is_some() { return self.on_form_key(key, ctx); }`, placed **before** `detail.captures_input()`. The form opens only while the detail pane is not capturing.
2. The existing detail capture guard.
3. The existing CONTROL/ALT branch to the detail pane.
4. In the letter `match`, beside `m`:
   - `KeyCode::Char('f') => self.form = Some(FilterForm::open(&self.filter, ctx.projects))`
   - `KeyCode::Char('F') => if !self.filter.is_empty() { self.apply(BacklogFilter::default(), ctx) }`

   Both arms consume the key. A real terminal sends `F` as `Char('F')` plus SHIFT; only CONTROL and ALT are diverted, so both forms match.

```rust
fn on_form_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled {
    if key.modifiers.contains(KeyModifiers::CONTROL) { return Handled::Pass; } // ctrl-c, Ctrl+F (boxes.rs on_editor_key rule)
    let Some(form) = self.form.as_mut() else { return Handled::Pass };
    match form.on_key(key) {
        FormOutcome::Stay => {}
        FormOutcome::Cancel => self.form = None,
        FormOutcome::Refused(sentence) => ctx.emit(Action::Error(sentence)),
        FormOutcome::Apply(filter) => { self.form = None; self.apply(filter, ctx); }
    }
    Handled::Consumed   // must swallow q, digits, w, ?, Tab/BackTab: global keymap resolves after Pass (state.rs:556-595)
}
/// Sets the filter and re-reads: one `Items`, same request kind, so the staleness index supersedes.
fn apply(&mut self, filter: BacklogFilter, ctx: &Ctx<'_>) {
    self.filter = filter;
    self.pending_reveal = None;   // a filtered reply must not decide an earlier reveal (D251)
    ctx.request(self.filter.to_request(ctx.scope));
}
```
Applying an unchanged filter still sends the read (the plan's risk row "applying twice shows the second result").

**`on_paste`** (about `:238`): if the form is open, call `form.on_paste(text)` and return `Handled::Consumed`. Otherwise delegate to `self.detail.on_paste` as now.

**`wants_requests`** (`:218`): `vec![self.filter.to_request(scope)]`.

**`on_scope_change`** (`:225`): keep the existing clears, then `self.filter.retain_projects(scope)` (uses `Scope::contains`, `scope.rs`) and `self.form = None`. The form's project snapshot belongs to the old scope. `App::set_scope` (`update.rs:175-189`) calls `on_scope_change` on every tab, then `activate_tab()`, which calls `wants_requests`, so the pruned filter is what goes out.

**`reveal`** (`:339`):
- Guard: `if self.form.is_some() || self.detail.captures_input()` → `CLOSE_THE_FIELD_FIRST`.
- On a hit, behaviour is unchanged.
- On a miss (the `None` branch): `self.filter = BacklogFilter::default();` then set `pending_reveal`, then `ctx.request(self.filter.to_request(ctx.scope))` (D4). This clears the filter even when the item is outside the workspace. The tab cannot tell "hidden" from "absent" before the reply, and D251's error then fires against the full list, as D4 intends.

**`reselect`** (`:190`) is unchanged. After a filtered reply it keeps the cursor if its row survives; otherwise it moves to the first `Selection::Item`, or the first project header if every group is empty, or `None` if there are no projects. `go()` then resets the detail pane or re-reads it. `on_reply` already prunes `folded`.

**`render`** (`:320`):
```rust
let [left, right] = panes(area);
let (list_area, form_area) = match self.form {
    Some(_) => { let [l, f] = Layout::vertical([Constraint::Min(0), Constraint::Length(filter::FORM_HEIGHT)]).areas(left); (l, Some(f)) }
    None => (left, None),
};
let summary = self.filter.summary(ctx.projects);
let view = ListView { items: &self.items, projects: ctx.projects, folded: &self.folded, selected: self.selected, filter: summary.as_deref() };
list::render(frame, list_area, &view, ctx.theme);
if let (Some(form), Some(at)) = (&self.form, form_area) { filter::render(frame, at, form, ctx.theme); }
```
Also update the module doc at `:3-5` ("Filters and editing are MOD-13").

### `list.rs`
- `ListView` (about `:37-47`) gets `pub filter: Option<&'a str>` ("The active filter's summary; `None` = no filter (D5)"). The only construction site is `mod.rs:323`.
- Add:
  ```rust
  pub const NO_ITEMS: &str = "No items in this workspace.";
  pub const NO_MATCH: &str = "No items match the filter.";
  #[must_use] pub fn title(count: usize, filter: Option<&str>) -> String
  // None -> format!(" Backlog ({count}) ")          (byte-identical to :96)
  // Some(s) -> format!(" Backlog ({count}) · {s} ")
  ```
- In `render`: `.title(title(view.items.len(), view.filter))`. The empty check becomes `if lines.is_empty() || (view.filter.is_some() && view.items.is_empty())`, with the text `if view.filter.is_some() { NO_MATCH } else { NO_ITEMS }` (E3).
- Known cosmetic behaviour, left as is: with a project filter, the other projects' headers still render as `▾ agy (0)`. `rows()`/`groups()` are untouched so that navigation and rendering stay in step.

### `app/mod.rs` `register_all` (after `:124`, E4)
Two `KeyScope::Tab(BacklogTab::ID)` bindings: `Char('f')` with help `"filter"`, and `Char('F')` with help `"clear filter"`. Action: `Action::Tab(TabAction::Focus(BacklogTab::ID))`, the same no-op as `m`. Add doc item "9." to the list at `:40-50`.

**Task 3 tests.** In `mod.rs`, reuse `platform()` and the `Ctx::new` boilerplate. Requests are visible as `Action::Store(StoreRequest)` (`state.rs:116`).
- `f_opens_the_filter_form_and_it_captures`: after `f` the form is open; `j` and `q` are `Consumed`; `selected` is unchanged; `emit` is empty.
- `applying_the_form_sends_one_filtered_items_read`: `f`, `l`×5, `space`, `Enter` emits exactly one `Action::Store(Items{ filter: ItemFilter{ statuses: Some([Done]), .. }, ready_here: false, .. })`, the form closes, and `tab.filter.statuses == [Done]`.
- `a_refused_tag_list_keeps_the_form_open_and_reports_it`: `f`, `down`, `down`, `R`, `Enter` gives one `Action::Error`, no `Store`, and the form stays open.
- `shift_f_clears_the_filter_and_re_reads`: exactly one default `Items`.
- `shift_f_without_a_filter_sends_nothing`.
- `wants_requests_carries_the_active_filter`.
- `a_reveal_of_a_hidden_item_clears_the_filter`: filter `{statuses:[Done]}`, items filtered to done only, reveal `HTUI_ANA_2`. `filter.is_empty()` holds, `pending_reveal.is_some()`, and the emitted `Items` has a default filter. Then `on_reply(Items(all))` selects `ANA-2`.
- `a_reveal_while_the_form_is_open_asks_to_close_it_first`.
- `applying_a_filter_disarms_a_pending_reveal`.
- `a_scope_change_drops_the_projects_it_lacks_and_closes_the_form`: `Scope { project_ids: [PROJECT_HTUI] }` keeps `[HTUI]` from `[HTUI, AGY]`; a filter that only named AGY becomes empty.
- `a_filtered_reply_moves_the_cursor_to_its_first_item`.
- `ctrl_c_passes_through_the_open_form` (`Handled::Pass`).
- `a_paste_while_the_form_is_open_goes_to_its_tag_field`.

In `list.rs` (new `mod tests`): `the_title_without_a_filter_is_unchanged` (`title(11, None) == " Backlog (11) "`), `the_title_with_a_filter_appends_its_summary`.

Validate: `cargo test -p htui --all-features --lib backlog`.

## 4. Integration and snapshots (Task 4, `crates/htui/tests/backlog.rs`)

Add a helper `async fn backlog_over(store: MemStore) -> Harness`: a copy of `backlog()` (`:52-65`) built on `Harness::over(store)`. Key names follow `KeyChord::parse`: `"f"`, `"F"`, `"l"`, `"space"`, `"down"`, `"enter"`, `"esc"`. Call `drive_to_end().await` after `enter`/`F`.

| Test | Keys (after `backlog()`) | Assertions |
|---|---|---|
| `filtering_by_status_done_lists_only_done_items` | `f l l l l l space enter` | `Backlog (2) · status:done`; contains `Data model, box registry` and `Prompt assembly survey`; no `TUI scaffold` |
| `filtering_by_one_project_lists_only_its_items` | `f j space enter` | `Backlog (8) · project:htui`; no `ACP transport upgrade` |
| `filtering_by_the_rust_tag_lists_only_rust_items` | `f down down r u s t enter` | `Backlog (4) · tags:rust` (htui FEAT-1/2/3, agy FEAT-1) |
| `ready_here_lists_what_this_box_can_start` | `backlog_over(demo + needs_cuda)`, then `f down down down space enter` | Before: frame contains `needs a GPU toolchain`. After: `Backlog (N) · ready here` where N = `store.ready_items(&platform, ids::BOX).len()` (3); every ready title is present; `needs a GPU toolchain` is absent |
| `shift_f_restores_the_whole_list` | status filter, then `F` | `Backlog (11) ` with no `·` in the title |
| `f_and_shift_f_are_on_the_backlog_help_line` | `register_all` | help contains `f filter` and `F clear filter` |
| snapshot `backlog__filter_form` | `f l l l l l space down down r u s t` (no apply) | The title is still `Backlog (11)`, the form is at the bottom of the list pane with `[x] done` and `> tags    rust` |
| snapshot `backlog__filtered_list` | `f l l l l l space enter` | `┌ Backlog (2) · status:done`, cursor on htui `ANA-1` |
| snapshot `backlog__filter_no_match` (optional, pins E3) | `f l l l l l space down down r u s t enter` | `No items match the filter.` |

All snapshots are 100x30. Accept them with `cargo insta accept`. No existing `backlog__*.snap` may change.

Validate: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`.

## 5. Commit boundaries
1. `feat(mod-13): ready_here on Items — this box's readiness composed in the worker`: `store_worker.rs`, plus the `ready_here: false` edits in `app/update.rs` and `backlog/mod.rs` (E5).
2. `feat(mod-13): BacklogFilter and the filter form`: `filter.rs` and the `pub mod filter;` line.
3. `feat(mod-13): wire the filter into the Backlog tab and list`: `backlog/mod.rs`, `list.rs`, `app/mod.rs` help bindings.
4. `test(mod-13): filter integration cases and snapshots`: `tests/backlog.rs` and the new `.snap` files.

Each commit must pass `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Close the milestone with `cargo test --workspace --all-features -- --test-threads=1`.

## 6. Hazards beyond the errata
- **Key propagation.** The tab sees every key before the global keymap, which resolves only on `Pass`. Any unconsumed key in the form would quit (`q`), switch tabs (digits, `Tab`), open the switcher (`w`) or toggle help (`?`). Hence "consume everything except CONTROL". `Esc` is bound only in overlay scope, so the form owns it with no collision.
- **Refusal wording.** `declared_tags_from_text` refusals read "declared tag `X` is not …", although these are *required* tags. This is accepted under D3 and noted for review.
- **Title length.** The list pane is 55 columns (53 inner). A long combined summary is truncated by ratatui. Tests keep to single-facet titles.
- **Pending reveal.** Applying or clearing a filter with a reveal in flight must disarm `pending_reveal`. Without that, the filtered reply falsely reports "not in this workspace's backlog".
- **Form visible across tab switches.** The form stays open across tab switches. `on_scope_change` closes it; switching tabs does not, which is harmless.
