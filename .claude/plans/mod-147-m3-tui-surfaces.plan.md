# Plan: MOD-147 M3 — TUI surfaces

**Source PRD**: `.claude/prds/mod-147-workspace-scope.prd.md`
**Selected Milestone**: 3 — TUI surfaces
**Complexity**: Large (Backlog list, store worker, Settings hierarchy and secrets, a new Settings pane, key catalogue)
**Routing**: plan path on PRD milestone 3 (C4 only); ultracode accepted for implement.

## Summary

The Backlog lists the workspace's own items (its scope row) as a first group titled with the workspace
name, and `N` on that group creates a `WS` item. The waiting overlay's Enter on a workspace item reveals it
there, which replaces M2's placeholder notice. Settings › Hierarchy opens an **owner documents** pane from a
project or workspace row (`o`): it lists the documents, shows one, and writes a new document or a new
version through `$EDITOR`. A workspace pane offers `goals` first. Settings › Secrets gains a workspace
row that binds the scope row's Infisical scope with the existing form. The workspace row and the workspace
editor show the folder on this box, falling back to the `~/workspaces/<slug>` default.

## Decisions taken in this plan

| # | Decision | Why |
|---|---|---|
| D1 | **The Items read covers the scope row.** `read_items` (`store_worker.rs:2923`) widens the asked `Scope` with `Scope::with_scope_row` (`htui-core/src/model/scope.rs:47-53`, which inserts at index 0) for its read only. The scope row comes from `backend.scope_project(scope.workspace_id)`; the lookup is skipped for a nil workspace, as `refresher_projects` does at `store_worker.rs:3192`. With D2's `project()`, that is **two reads per Items request, uncached**. The loop's memo `scope_rows` is a local of the worker loop (`:3309`) that `try_serve`/`serve` cannot reach, and `serve_waiting` also re-reads uncached (`:2380`). No filter guard is needed: Pg, cache and Mem all intersect `filter.project_ids` with the scope (`pg/read.rs:104`, `cache/read.rs:428`, `mem.rs:1780`), so a project facet already leaves scope-row items out | Same shape as M2 D10's waiting widening. Pg orders by `array_position`, so the scope row lists first. The cache already mirrors scope-row items (`htui-store/tests/it/cache.rs` `a_scope_row_mirrors_its_kinds_items_and_owner_documents`) |
| D2 | **`StoreReply::Items` carries the scope row**: `StoreReply::Items { items: Vec<ItemSummary>, scope_row: Option<ProjectRef> }` (variant at `store_worker.rs:1555`). The `ProjectRef` is built from `scope_project` + `project()`: the slug is `@<ws>`, the name is the scope row's `name` (= the workspace name, `fixtures.rs:795`) and `position` is 0 (`i32`). The sites that follow are the constructor at `store_worker.rs:2453`, the one consumer at `backlog/mod.rs:542`, and 20 test sites (`app/update.rs:764,825` under `#[cfg(test)]`; `store_worker.rs:4709,4766,4950,5408,5436`; `backlog/tests/{filter,navigation,queue,writes}.rs`). No other tab or overlay consumes it (`update.rs:450-470`) | The Backlog needs the scope row's id even when the workspace has no items yet (for `N`). `ctx.projects` and `Scope` stay members-only (M1 D1), so pickers and the top bar do not change |
| D3 | **The workspace group**: `BacklogTab.scope_row: Option<ProjectRef>`, set from the reply and reset in `on_scope_change`, and `BacklogTab::group_projects(&self, ctx) -> Vec<ProjectRef>` = `scope_row` followed by `ctx.projects`. That one source replaces `ctx.projects` in every group-source site: `BacklogTab.rows` (`navigation.rs:71-74`), `ListView { projects }` (`mod.rs:660`, which feeds `list::render`/`list_rows`/`empty_state` at `list.rs:217`), fold retention (`mod.rs:549-553`) and `select_item` (`navigation.rs:132`). The graph sub-tab's `in_workspace` is called from `re_root` (`detail/graph.rs:246`) and from `lines()` (`:515`, `:536`, via `render` `:817`, which dims nodes outside the workspace). It takes the same list, handed to `GraphTab` by the Backlog when the reply lands (a setter or a field), so WS nodes are neither dimmed nor re-rooted out. The workspace group is always drawn when the reply names a scope row, including as `▾ Platform (0)` when it is empty | One list of group projects, so a scope-row item is never selected without being drawn |
| D4 | **Filter facets stay members-only.** `FilterForm::open` keeps `ctx.projects`, and a project facet hides the workspace group (D1) | A workspace facet is a filter feature; MOD-119 redesigns the Backlog filters |
| D5 | **`N` on the workspace group** (maintainer, 2026-11-13). `open_new` (`writes.rs:63-85`) already targets `Selection::Project(p)` on a header and the item's project on a row, so a scope-row target needs no change there. The change is in `on_item_form` (`writes.rs:151-155`): when `opening.project == scope_row.project_id`, it passes the projects slice `[scope_row]` to `ItemForm::open_new(context, projects, kind)` (`item_form.rs:315`). The picker (`pick`, `:778`) therefore cannot cycle to members, and `slug()` (`:959`) names `@<ws>` instead of `"?"`. The form catalogue is served for the scope row's `ProjectId` unchanged (`item_writes.rs:240-253`, `catalogue` `:344`), so its kinds are `WS`. Edit, queue and run actions are unchanged (keyed by item, `queue.rs:54`, `store_worker.rs:2701`) | Without it no TUI path creates a WS item before MOD-136 |
| D6 | **Waiting overlay**: `WaitingList::enter` (`waiting_list.rs:132-147`) drops the `ctx.scope.contains` check and the `WORKSPACE_ITEMS_LATER` constant (`:66`). Every row closes the overlay and reveals by `ItemId` (`RevealTarget::Step`). The unit test `enter_on_a_workspace_row_says_where_it_opens` (`:845`, constant at `:855`) and `tests/it/waiting.rs:442-463` `a_workspace_items_gate_is_listed_first` (string literal at `:460`) now assert the reveal | Reveal resolves by `ItemId` (`backlog/mod.rs:785`), and D1 loads scope-row items, so the reveal lands |
| D7 | **Owner documents in the worker**: a new module `crates/htui/src/owner_documents.rs` (`pub mod` in `lib.rs`) mirrors `hand_written.rs`. It has its own `REQUEST_NAMES` and a `request_names_match_the_name_arms` test (as `hand_written.rs:381` has), `serve`, and `DocumentOwner::{Project(ProjectId), Workspace(WorkspaceId)}`, which the worker resolves through `scope_project`; a workspace without a scope row answers `NotFound`. The requests are `StoreRequest::OwnerDocuments(DocumentOwner)` and `StoreRequest::WriteOwnerDocument { owner, kind, title, body: HandText }`. The replies are `StoreReply::OwnerDocuments { owner, documents: Vec<OwnerDocument> }` and `StoreReply::OwnerDocumentWritten { owner, kind, version }`. `StoreRequest::name` gets the new arms, and `try_serve` (`:2505+`, which has no wildcard) gets a dispatch arm. The read goes through `Backend::owner_documents` and works offline (`backend.rs:1225`). That differs on purpose from `hand_written`, which refuses offline reads too (`:13-15`). The write goes through `backend.writer()`; offline it answers `Unreachable(htui_store::DATABASE_UNREACHABLE)`. It fills `created_by = this_user`, a UUIDv7 `id` and `created_at = now`, and checks the kind with `htui_core::model::hand_written::document_kind` (`htui-core/src/model/hand_written.rs:62`; `crate::hand_written` imports it `as rules`) | The store API exists since M1, but nothing in `crates/htui` reaches it today |
| D8 | **`goals` kind**: `pub const GOALS_KIND: &str = "goals";` in `htui-core/src/model/document.rs`, beside `OwnerDocument` (`:35`) and `NewOwnerDocument` (`:60`). A workspace pane always lists `goals` first: when none exists, its row reads `goals — none yet` and `n` there pre-fills the kind | PRD: "goals preset for a workspace". No production code uses the literal today |
| D9 | **The owner documents pane**: `settings/owner_documents.rs` (`pub mod` in `settings/mod.rs`), `OwnerDocumentsPane`, copies the memory pane (`settings/memory.rs`). It is opened by a new act `HierarchyDocuments`, **default `o`** (free in `HIERARCHY_BROWSE`, `keys/views.rs:265-284`), on a workspace or project row (Hierarchy `Mode::Documents { owner, pane }`, as `Mode::Memory` is at `settings/hierarchy.rs:301-332`); any other row shows a notice. It shows **one row per document version**, ordered by kind and newest version first: `kind · vN · title · date`. **Enter** shows the selected version's body, read-only, through `ui/markdown.rs` `lines`/`row_count` (`:56`) with scrolling, and Esc goes back. **`n`** (common New) opens a two-field form (kind, title), then `$EDITOR` with an empty body. **`v`** (new act) on a row opens the same form with the kind fixed and the title pre-filled, then `$EDITOR` with that version's body. `Edited` writes the document; `Unchanged` writes nothing and shows `editor::NO_CHANGES`; `Failed` shows its text. A new context `settings.documents` is added with the full new-context checklist: the `Context` variant (`catalogue.rs:108`), `ALL` (`:208`), `table()` (`:268`), `heading()` (`:330`), `is_view` (`:387`), `context_position` (`:2684`, renumbering the later ones), the headings test (`:3608`, `:3661`), the `VIEW_DEFAULTS` down/up rows (`:2536`), the acts' `ALL`/`position()` entries, the stacks in `views.rs` with their pinned resolutions, `TABLES` in `keys/load.rs:539-548` and its mirror `tests/it/keys_file.rs:244-253`, and `docs/htui-keys.md` regenerated (`cargo run -q -p htui -- --default-keys --print-keys`; test `keys_file.rs:729`). The `o` hint goes in `HINT_BROWSE` (`settings/hierarchy.rs:102-113`) **before** `memory`, so it shows in the cut-off hint line | PRD: "a documents list, view, new version through `$EDITOR`". `Compose::document` is bound to an item (`compose.rs:159`), so it is not reused |
| D10 | **`$EDITOR` reaches Settings**: `SettingsSection` (`settings/mod.rs:142-220`) gains `fn on_external_edit(&mut self, outcome: ExternalEditOutcome, ctx: &mut Ctx<'_>) {}` (a default no-op). `SettingsTab` overrides `Tab::on_external_edit` (`ui/tabs/registry.rs:105`) and forwards it to the active section. The hierarchy section forwards it to its documents pane. Edits emitted by a section are already stamped `Origin::Tab(SettingsTab::ID)` (`app/state.rs:1065`) and returned through `finish_external_edit` (`:1118`). This works in both Suspend and Pane modes, and the pane needs `Ctx::claim_editor_area` | The return path ends at `Tab::on_external_edit`, a default no-op, and Settings has no override today |
| D11 | **Secrets workspace row** (maintainer, 2026-11-13: in Secrets, not the workspace form). `HierarchySnapshot` (`crates/htui/src/hierarchy.rs:39-48`; one struct literal, `snapshot()` `:227`) gains `scope_row: Option<Project>`, filled through `scope_project` + `project()`. Secrets adds `Row::Workspace` between the fixed rows and the projects (`secrets.rs:211-222`, `row_at` `:478-483`), labelled `workspace <name>` with `project_text(&Project)` (`:970`). One helper `project_in(tree, id) -> Option<&Project>` searches `scope_row`, then `projects`. It replaces the `tree.projects`-only lookups in `entry` (`:506`), `edit` (`:551`), `clear` (`:609`), `test` (`:672`), `closed_if_gone` (`:1298`, run on every passive tree) and `on_scope_stale` (`:1397`). Without it, a passive tree would close the workspace-row form and a stale reply would say `DELETED_ELSEWHERE`. `e` and clear send the existing `SetProjectSecretScope` with the scope row's id and `updated_at`, which already resolves a scope row (M1 D9, `tests/it/hierarchy.rs:938`). `t` sends `CheckSecretScope { project }`, which the runtime resolves by id (`agent_worker.rs:1843-1862`) | The PRD row is amended to say so. This reuses the form, the CAS handling and the stale path |
| D12 | **The folder on this box, computed in the UI**: the Hierarchy section captures `dirs::home_dir()` once at construction. It renders `htui_core::model::hierarchy::workspace_folder(slug, tree.folder_path, home)` (`model/hierarchy.rs:269-285`), writing a path under `home` as `~/…`. The workspace row (`settings/hierarchy.rs:469-477`) then reads `{name} ({slug}) — folder on this box: ~/workspaces/graphics (default)`; a stored root has no `(default)`; `unset` appears only when the slug is not one plain name. The workspace editor shows the same line read-only, under its fields, with the `b` hint (`HierarchyChoosePath`). `HierarchySnapshot` gets no folder field | `snapshot()` has no `home`. Threading one through would touch `serve_with_home`, `fresh_tree`, `reread`, `cas`, `infer` and three test callers, and the `with_root` test helper (`tests/it/hierarchy.rs:2315`) would leave a derived field stale. The worker's `home` is the real `$HOME` anyway (`hierarchy.rs:248-249`), and the `~` form keeps snapshots stable wherever `$HOME` is set |
| D13 | **Out of scope, recorded**: the requirement `deciding_item` resolver (`requirements.rs:794`) stays bare-key. A project requirement's deciding item is an item of that project, and workspace requirements are MOD-149's (M2 D13 closed here). Owner documents in search: M4. Owner documents in the Workspaces tab: MOD-116 | Keeps M3 to the PRD row |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Read widened for the read only | `crates/htui/src/store_worker.rs:2344-2395` `waiting_reads`; `htui-core/src/model/scope.rs:47` `with_scope_row` | widen a cloned `Scope`, keep the asked scope everywhere else |
| Worker request module | `crates/htui/src/hand_written.rs:41` `REQUEST_NAMES`, `:174` `serve`, `:66` `HandText`, `:275` offline refusal, `:381` names test | one module per surface, names listed and pinned, offline write refused before any read |
| Settings pane opened from a hierarchy row | `crates/htui/src/ui/tabs/settings/memory.rs` `MemoryPane`; `settings/hierarchy.rs:681-688`, `:743-759` `open_memory`, `:1508-1550` delegation | `Mode::Memory { project, pane }`; key, paste, reply, render, hint and stack delegated |
| `$EDITOR` round trip | `crates/htui/src/editor.rs:231` `ExternalEdit`, `:249` `ExternalEditOutcome`; `ui/tabs/backlog/detail/documents.rs:251`, `:556`; `app/state.rs:1065`, `:1118` | emit `Action::EditExternally`, handle the outcome in `on_external_edit` |
| Secrets project row and form | `settings/secrets.rs:211-222` rows, `:300-309`, `:576-604` `EditingScope`, `:844-871` submit, `:1365` `on_scope_written` | same mode and request for the workspace row |
| Key acts and contexts | `crates/htui/src/keys/catalogue.rs:1419` `HierarchyMemory`, `:1548-1565` `[settings.memory]`; `keys/views.rs:265-284`, `:306-345` | act + row + `ALL` + `position()` + stack; new context checklist (D9); `docs/htui-keys.md` regenerated |
| Tests | `crates/htui/tests/it/settings_memory.rs:140-183` (`SectionBench`, serve then reply, `#![cfg(feature = "testkit")]`); `tests/it/backlog.rs:60-88` `backlog()`; `tests/it/secrets_settings.rs` | MemStore demo backend, snapshots through `insta`; new files join `tests/it/main.rs` |

## Files to Change (per task; independence is decided by these sets)

| Task | Files |
|---|---|
| T1 Backlog workspace group + waiting reveal | UPDATE `crates/htui/src/store_worker.rs` (`read_items`, `StoreReply::Items` shape, its tests), `crates/htui/src/app/update.rs` (test sites), `crates/htui/src/ui/tabs/backlog/{mod.rs,list.rs,navigation.rs,writes.rs,detail/graph.rs}` (+ `item_form.rs` only if `open_new` needs a doc touch), `crates/htui/src/ui/tabs/backlog/tests/{filter.rs,navigation.rs,queue.rs,writes.rs}`, `crates/htui/src/ui/overlay/waiting_list.rs`, `crates/htui/tests/it/{backlog.rs,backlog_forms.rs,waiting.rs}`; ~70 moved snapshots: 50 `it__backlog*` plus `it__concepts_search__*` 7, `it__shell__*` 5, `it__prompt_preview__*` 2, `it__overlays__*` 2, `it__waiting__*` 2, `it__integration__demo_shell`, `it__replay__runs_step_selected` |
| T2 Secrets workspace row + folder line | UPDATE `crates/htui/src/hierarchy.rs` (`HierarchySnapshot.scope_row`, `snapshot()`), `crates/htui/src/ui/tabs/settings/{secrets.rs,hierarchy.rs}`, `crates/htui/tests/it/{secrets_settings.rs,hierarchy.rs}`; moved snapshots: all 8 `it__secrets_settings__*`, and the 8 `it__hierarchy__*` that show the workspace row (`delete_typed`, `delete_warn`, `demo`, `demo_80`, `editor_repo`, `inferred`, `picker`, `stale`) |
| T3 Owner documents | CREATE `crates/htui/src/owner_documents.rs`, `crates/htui/src/ui/tabs/settings/owner_documents.rs`, `crates/htui/tests/it/settings_documents.rs`; UPDATE `crates/htui/src/{lib.rs,store_worker.rs}`, `crates/htui/src/ui/tabs/settings/{mod.rs,hierarchy.rs}`, `crates/htui/src/keys/{catalogue.rs,views.rs,load.rs}`, `crates/htui/tests/it/{main.rs,keys_file.rs}`, `crates/htui-core/src/model/document.rs`, `docs/htui-keys.md`; moved hierarchy snapshots `demo`, `demo_80` (the `o` hint before `memory`) |

T1 and T3 both touch `store_worker.rs`. T2 and T3 both touch `settings/hierarchy.rs` and the hierarchy
snapshots `demo`/`demo_80`. The intersections are non-empty, so the tasks run **serial in one lane:
T1 → T2 → T3**. None of them is a foundation the others build on, so none gets a verifier.

## Tasks

### T1: Backlog workspace group and the waiting reveal (tests first)
- **Action**: tests first:
  - Store worker: `Items` for Platform with a minted `WS-1` lists it first and answers `scope_row` `@platform`/"Platform". A project facet without the scope row leaves it out. A nil workspace answers `scope_row: None`. The 11-item pin (`store_worker.rs:4706`) still holds, because the demo seeds no scope-row items. The offline (cache) answer is the same.
  - Backlog: the list shows `▾ Platform (1)` above the member groups. An empty workspace shows `▾ Platform (0)`. Fold and unfold work on it and survive an Items reply. `N` on the workspace header opens `New item · @platform` with kind `WS`, and its picker does not move. Reveal of a WS item selects a drawn row. The graph sub-tab neither dims nor re-roots out a WS node.
  - Waiting: Enter on a WS row closes the overlay and reveals it (`waiting_list.rs:845`; `tests/it/waiting.rs:442-463`).

  Then:
  - D1 `read_items` widening.
  - D2 reply shape.
  - D3 `scope_row`/`group_projects` at every group-source site, and the graph hand-off.
  - D5 `on_item_form` slice.
  - D6 `enter`.
  - A full insta run of the `htui` crate. Every moved snapshot is reviewed: list frames gain the `▾ Platform (0)` header line (Graphics frames gain `▾ Graphics (0)`), and nothing else moves.
- **Validate**: `cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^backlog/) or test(/^waiting::/))'`; `cargo test -p htui --all-features --lib -- store_worker:: backlog:: waiting_list::`; `cargo insta test -p htui --all-features` (review, never blind-accept); `cargo clippy -p htui --all-features -- -D warnings`.

### T2: Secrets workspace row and the folder line (tests first)
- **Action**: tests first:
  - `hierarchy.rs`: the snapshot carries the scope row (Graphics: `@graphics`).
  - Secrets: a `workspace Graphics` row sits between the fixed rows and the project rows. `e` there sends `SetProjectSecretScope` for the scope row's id with its `updated_at`. Applied, stale and clear behave as on a project row. A passive tree does not close its form. `t` sends `CheckSecretScope` for it. The row text shows the binding.
  - Hierarchy: the workspace row reads `folder on this box: ~/workspaces/graphics (default)` with `$HOME` set; a stored root drops `(default)`; the workspace editor shows the same line.

  Then D11 and D12, and the moved `it__hierarchy__*` / `it__secrets_settings__*` snapshots are reviewed.
- **Validate**: `cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^hierarchy::/) or test(/^secrets_settings::/))'`; `cargo test -p htui --all-features --lib -- hierarchy:: secrets::`; `cargo insta test -p htui --all-features`; `cargo clippy -p htui --all-features -- -D warnings`.

### T3: Owner documents (tests first)
- **Action**: tests first:
  - `owner_documents::serve`:
    - A project owner lists its documents and no item documents.
    - A workspace owner lists its scope row's documents.
    - A write mints v1, then v2 for the same kind, with `created_by` the box user.
    - A bad kind is refused with `document_kind`'s sentence.
    - An offline write answers `Unreachable` and an offline read answers from the mirror.
    - `request_names_match_the_name_arms`.
  - Pane (`tests/it/settings_documents.rs`, `SectionBench` as in `settings_memory.rs`):
    - `o` on the workspace row opens the pane with `goals — none yet` first.
    - `n` then Enter emits `EditExternally`; `Edited` sends `WriteOwnerDocument`, and the written reply rereads and lists `goals · v1`.
    - `v` on it pre-fills the title and hands the body to `$EDITOR`.
    - `Unchanged` sends nothing.
    - Enter shows the body and Esc goes back.
    - `o` on a repo row shows the notice.
    - The outcome reaches the pane through `SettingsTab::on_external_edit`.
    - Snapshots: `list`, `view`, `form`.

  Then:
  - D7 module and store-worker variants, name arms and dispatch.
  - D8 `GOALS_KIND`.
  - D9 pane, `Mode::Documents`, the `o` act, and the `settings.documents` context with its full checklist; regenerate `docs/htui-keys.md`.
  - D10 `on_external_edit` path.
  - The moved `demo`/`demo_80` hierarchy snapshots are reviewed.
- **Validate**: `cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^settings_documents::/) or test(/^hierarchy::/) or test(/^keys_file::/) or test(/^no_hardcoded_keys::/))'`; `cargo test -p htui --all-features --lib -- owner_documents:: keys::`; `cargo test -p htui-core --all-features --lib model::document`; `cargo insta test -p htui --all-features`; `cargo clippy -p htui --all-features -- -D warnings`; `cargo clippy -p htui -- -D warnings`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
bash scripts/gate.sh                              # main thread, once per lane; live-Qdrant suites owed on host
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Snapshot churn: ~70 frames gain the workspace header line, beyond the Backlog's own files | High | full insta run in T1, every moved snapshot reviewed against D3 (only the header line may move) |
| A Backlog path still keys off `ctx.projects` and treats a WS item as outside the workspace | Medium | D3 lists every group-source site; tests for fold, reveal and the graph on a WS item |
| Two more reads per Items request | Low | both are indexed point reads, as in `serve_waiting`; a memo reachable from `try_serve` is possible later |
| A Secrets path still looks only in `tree.projects` | Medium | D11's one `project_in` helper; tests for passive-tree survival and stale on the workspace row |
| `$EDITOR` return lands on the wrong section | Low | D10 forwards to the active section only; test pins it |
| `$HOME` unset in a test runner turns the folder line into `unset` | Low | the gate and nextest inherit `$HOME`; the test asserts with it set |

## Acceptance
- [ ] All tasks complete, tests written first
- [ ] The Backlog lists WS items as the workspace's first group; `N` there creates one; the waiting overlay reveals one
- [ ] Settings lists, shows and adds owner documents for a project and a workspace, `goals` first for a workspace
- [ ] Secrets binds the scope row from a workspace row; the folder on this box shows its default
- [ ] Gate green except live-Qdrant suites (owed on host)
- [ ] Patterns mirrored, not reinvented

## Verified claims (step 3.5 fact-check, 2026-11-13; two read-only checkers)

| Claim | Verdict | Evidence |
|---|---|---|
| `Scope::with_scope_row` inserts at 0 | TRUE | `htui-core/src/model/scope.rs:47-53`; a row already present stays where it is |
| `read_items` can use `refresher_projects`' memo | FALSE → amended | `scope_rows` is a loop local (`store_worker.rs:3309`); `try_serve` (`:2439`)/`serve` (`:2292`) cannot reach it. D1 reads `scope_project` + `project` per request |
| A filter guard is needed for the widened scope | FALSE → amended | Pg `pg/read.rs:104`, cache `cache/read.rs:428`, Mem `mem.rs:1780` all intersect `filter.project_ids` |
| `StoreReply::Items` sites: 22, 2 production | PARTLY → amended | 22 hits include the constructor `:2453`; production = `:2453` + `backlog/mod.rs:542` (+ variant `:1555`); `update.rs:764/825` are `#[cfg(test)]` |
| Demo seeds no scope-row items; 11-item pin | TRUE | `fixtures.rs:767-790` (kinds only); `store_worker.rs:4706-4723` |
| Refresher publish test | TRUE | `store_worker.rs:5380-5441` asserts the refresher list `[HTUI, AGY, SCOPE_PLATFORM]`; the reply only `matches!` |
| `ProjectRef { project_id, slug, name, position: i32 }` | TRUE | `model/hierarchy.rs:289-298` |
| D5 slice goes in `open_new` | FALSE → amended | `open_new` (`writes.rs:63`) targets correctly already; the slice is passed in `on_item_form` (`writes.rs:151-155`); `ItemForm::open_new` `item_form.rs:315` |
| `in_workspace` only re-roots | PARTLY → amended | also `lines()` `detail/graph.rs:515/536` via `render` `:817` (dimming); added to D3 |
| `groups` callers | TRUE, extended | `rows` `navigation.rs:71-74`, `ListView` `mod.rs:660` → `list_rows`/`empty_state` `list.rs:217`; listed in D3 |
| `WORKSPACE_ITEMS_LATER` references | TRUE | `waiting_list.rs:66,137,855`; `tests/it/waiting.rs:460` is a string literal |
| Snapshot impact of the workspace header | PARTLY → amended | 70 snapshots show Backlog group headers (50 `it__backlog*` + 20 elsewhere); listed in the T1 file set |
| Other consumers of `StoreReply::Items` | none | replies route to the requester (`update.rs:450-470`); `observe_reply` ignores Items |
| `HierarchySnapshot` literal sites | TRUE | one, `snapshot()` `hierarchy.rs:227` |
| `snapshot()` has `home` | FALSE → amended | `hierarchy.rs:192-196`; D12 moves the folder computation to the UI |
| Worker/test `home` deterministic | FALSE (noted) | `serve` uses `dirs::home_dir()` (`hierarchy.rs:248-249`); `~/…` form keeps snapshots stable with `$HOME` set |
| Secrets lookups only search `tree.projects` | TRUE → D11 amended | `entry :506`, `edit :551`, `clear :609`, `test :672`, `closed_if_gone :1298`, `on_scope_stale :1397` |
| `project_text(&Project)`; `CheckSecretScope` resolves by id | TRUE | `secrets.rs:970`; `agent_worker.rs:1843-1862` |
| Settings has no `on_external_edit` path | TRUE | `settings/mod.rs:142-220`; default `ui/tabs/registry.rs:105`; stamping `app/state.rs:1065`, return `:1118` (plan's `update.rs:61` is only the non-tab refusal) |
| `hand_written::document_kind` | PARTLY → amended | it is `htui_core::model::hand_written::document_kind` (`:62`); `HandText` `htui/src/hand_written.rs:66`; `DATABASE_UNREACHABLE` `htui-store/src/writer.rs:112`, re-exported; `NewOwnerDocument` needs `created_at` |
| `o` free in `HIERARCHY_BROWSE` | TRUE | `keys/views.rs:265-284`; `v`, `D`, `O` also free |
| New context needs only `TABLES` edits | FALSE → amended | full checklist in D9 (`catalogue.rs:108,208,268,330,387,2684,3608,3661`, `VIEW_DEFAULTS :2536`) |
| `docs/htui-keys.md` = `keys::print(defaults)`; CLI flags exist | TRUE | `tests/it/keys_file.rs:729`; `cli.rs:558-593` |
| `markdown::lines(body, width, theme)` | TRUE | `ui/markdown.rs:56`, `row_count` beside it |
| Module and test registration | TRUE | `htui/src/lib.rs:24,31`; `settings/mod.rs:24`; `tests/it/main.rs` mod list |
| `editor::NO_CHANGES` pub; outcome variants | TRUE | `editor.rs:35`, `:249` |
| No production `"goals"` literal | TRUE | tests and `spikes/` only |
| Hierarchy hint snapshots move with a new act | PARTLY → amended | `demo`/`demo_80` cut the hint before delete/reload (`HINT_BROWSE :102-113`); D9 puts `o` before `memory` |
| A shared request-name stability test | FALSE (noted) | per-module `request_names_match_the_name_arms`; `try_serve` has no wildcard (`:2505+`) |
| Task file sets disjoint | FALSE → serial | T1∩T3 `store_worker.rs`; T2∩T3 `settings/hierarchy.rs`, hierarchy `demo`/`demo_80`; one lane T1 → T2 → T3 |
