# Blueprint: MOD-141 M3 - Every scope, and the picker fix

Source plan: `.claude/plans/mod-141-m3-every-scope-picker-fix.plan.md` (D1-D6, M-1/M-2). Written by
`code-architect`, 2026-11-12, read through Gortex. Where this blueprint and the plan differ, **this
blueprint wins** (plan corrections in §4).

## Design calls

- `WorkspaceNames` lives in `catalogue.rs`. `ProjectCatalogue` and `ItemFormContext` each carry one and
  expose `label(&StepGraph)`, so a list never needs to know which context it reads.
- The item form fills names only where a context reaches the UI: the `ItemForm` arm and the `Diverged`
  arm. Mint and edit validation read nothing extra.
- The host boxes the editor itself (`State::Open(Box<GraphEditor>)`, which avoids clippy's
  `large_enum_variant`), and Kinds holds the host unboxed.
- `lib.rs:10` sets `#![warn(missing_docs)]` and the gate runs with `-D warnings`, so every pub item gets
  docs. Use `#[must_use]` on pure fns and `# Errors` on fallible ones.

## 1. Types and signatures

### `crates/htui/src/catalogue.rs` (T1)

```rust
/// MOD-141 M3 D1
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceName { pub id: WorkspaceId, pub name: String, pub slug: String }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceNames { rows: Vec<WorkspaceName> }   // sorted by id, one row per id

impl WorkspaceNames {
    #[must_use] pub fn new(rows: impl IntoIterator<Item = WorkspaceName>) -> Self;
    /// One `WriteStore::workspace(id)` per distinct `GraphScope::Workspace` among `graphs`; a `None`
    /// answer is left out (that graph gets the fallback label).
    pub async fn read<'a, S: WriteStore + ?Sized>(
        store: &S, graphs: impl IntoIterator<Item = &'a StepGraph>,
    ) -> Result<Self>;
    #[must_use] pub fn label(&self, name: &str, scope: GraphScope) -> String;
    #[must_use] pub fn len(&self) -> usize;
    #[must_use] pub fn is_empty(&self) -> bool;
}
```

`read` collects the ids into a `BTreeSet<WorkspaceId>` before the first `.await`, so the future stays
`Send` and the ids come out sorted and unique (if `WorkspaceId` is not `Ord`, sort by its uuid).

`label` answers:

| Graph | Label |
|---|---|
| Project | the bare name |
| Library | `name · library` |
| Workspace, row read | `name · workspace <name>`, or `<slug>` in place of the name when another listed workspace shares the name |
| Workspace, row not read | `name · workspace` (so `WorkspaceNames::default().label(..)` is the old label, which D3 step 3 needs) |

Also in this file:
- `ProjectCatalogue` gets `pub workspaces: WorkspaceNames`, plus `pub fn label(&self, graph: &StepGraph)
  -> String` and `pub fn label_of(&self, id: StepGraphId) -> Option<String>`
  (`graph_name(id)` + `workspaces.label`).
- `snapshot` calls `WorkspaceNames::read(store, shared.iter().chain(&kept))` after `kept` is built and
  fills the field in the one literal at `:186`.
- `graph_label` (`:126-136`) is deleted.

### `crates/htui/src/item_writes.rs` (T1)

- `ItemFormContext` gets `pub workspaces: WorkspaceNames` and `pub fn label(&self, graph: &StepGraph) ->
  String`. Its hand-written `Debug` adds `.field("workspaces", &self.workspaces.len())`.
- `catalogue()` sets `workspaces: WorkspaceNames::default()`.
- New `async fn named(writer: &Writer, context: &mut ItemFormContext) -> Result<()>` reads over
  `context.graphs.iter().chain(&context.item_graph)`. It is called after `item_graph` is set, in the
  `ItemForm` arm (`:238`) and the `Diverged` arm (`:292`).

### `crates/htui/src/ui/tabs/settings/kinds.rs` (T1)

```rust
pub(crate) fn ambiguous_graph(text: &str, labels: &[String]) -> String
// "\"flow\" names 2 graphs; type one of: flow · workspace Platform, flow · workspace Graphics"
```

`graph_named` resolves in this order:
1. If `current` is set and `project.label_of(current)` equals the text, return `current`.
2. Else collect every graph in `project.graphs` and `project.shared` whose `project.label(g)` equals the
   text.
3. If that is empty, collect every graph in `shared` whose name equals the text, or whose
   `WorkspaceNames::default().label(..)` equals it.

The result then decides:

| Candidates | Answer |
|---|---|
| none | `NO_GRAPH_NAMED` |
| one | its id |
| several | `ambiguous_graph(text, labels)`, labels in candidate order (`shared` order) |

`kept` stays out of typed resolution.

### `crates/htui/src/ui/graph_editor/host.rs` (T2, new)

```rust
pub struct GraphEditorHost { state: State }
enum State {
    Loading { graph: StepGraphId, personas: Option<Vec<PersonaSummary>> },
    Open(Box<GraphEditor>),
    Refused { graph: StepGraphId, message: String },
}
impl GraphEditorHost {
    #[must_use] pub fn open_by_id(graph: StepGraphId) -> (Self, Vec<StoreRequest>);   // [GraphBody(graph), Personas]
    #[must_use] pub fn with_body(body: GraphBody, personas: Vec<PersonaSummary>, scope: &Scope)
        -> (Self, Vec<StoreRequest>);                                                // GraphEditor::open wrapped
    pub fn on_reply(&mut self, reply: &StoreReply, ctx: &mut Ctx<'_>) -> bool;
    #[must_use] pub fn graph(&self) -> StepGraphId;
    pub fn editor_mut(&mut self) -> Option<&mut GraphEditor>;
    pub fn set_personas(&mut self, personas: &[PersonaSummary]);
    #[must_use] pub fn busy(&self) -> bool;
    #[must_use] pub fn stack(&self) -> Stack<'static>;                 // Open: editor's, else views::GRAPH_EDITOR
    #[must_use] pub fn hint_text(&self, keys: &Keys) -> String;        // else a close-only hint (Act::GraphEditorClose)
    #[must_use] pub fn wants_requests(&self, scope: &Scope) -> Vec<StoreRequest>; // Loading: resend both; Refused: []
    #[must_use] pub fn takes_paste(&self) -> bool;
    pub fn on_paste(&mut self, text: &str) -> Handled;
    pub fn on_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> EditorKey;      // else: GraphEditorClose -> Close, rest modal_rest
    pub fn on_mouse(&mut self, mouse: MouseEvent, ctx: &mut Ctx<'_>) -> Handled;
    pub fn on_mouse_lost(&mut self);
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>);        // Loading/Refused: one line
}
```

Kinds calls these `GraphEditor` methods today: `open`, `select`, `open_node_form`, `graph`,
`set_personas`, `on_reply`, `hint_text`, `stack`, `wants_requests`, `takes_paste`, `on_paste`, `on_key`,
`on_mouse`, `on_mouse_lost`, `render`. Kinds never calls `busy`.

`on_reply` takes these replies (returns `true`); everything else returns `false`:

| State | Reply | Effect |
|---|---|---|
| Loading | `GraphBody(Some(b))`, `b.graph.id == graph` | open the editor with the held personas; send its three reads with `ctx.scope` |
| Loading | `GraphBody(None)` | Refused "graph not found" |
| Loading | `Failed{"graph_body"}` | Refused, the store's sentence. Only while Loading: Kinds' `REQUEST_NAMES` holds `"graph_body"` |
| Loading | `Failed{"personas"}` | taken; the editor opens without personas |
| any | `Personas(rows)` | mapped locally to `PersonaSummary`; held while Loading, else `set_personas`. Map in `host.rs`, not as a `From` in `catalogue.rs`, which would put T2 in T1's file set |
| any | `Catalogue(s)` | `set_personas(&s.personas)` |
| Open | `GraphReplaced{catalogue}` | personas from the catalogue when present, then to the editor |
| Open | `Agents`, `Templates`, `GraphEditorSettings` | to the editor |
| Open | `Failed` for `replace_graph`, `agents`, `templates`, `graph_editor_settings` | to the editor |

`Debug` is hand-written: the state name and the graph id, never step names.

`graph_editor/mod.rs` gets `mod host; pub use host::GraphEditorHost;`. Its module doc says "Only this
module names `Act`, `Keys`, `Stack`, `Ctx` or `StoreRequest`", which becomes false and must be amended.

### Kinds `Mode::Graph` (T3)

- `Mode::Graph { host: GraphEditorHost }`; `Mode`'s `Debug` prints `.field("host", host)`.
- `open_editor`: `GraphEditorHost::with_body(entry.clone(), self.personas().to_vec(), ctx.scope)`, then
  `select` and `open_node_form` through `host.editor_mut()`.
- `close_editor`: `host.graph()`.
- `on_editor_reply`:
  - `GraphReplaced`: keep the snapshot, `unavailable`, `clamp_cursor` and the R1 L1 re-read, then
    `host.on_reply`. Drop the section's own `set_personas`, since the host now does it.
  - `Catalogue`: `self.on_catalogue(..)`, then `host.on_reply`.
  - everything else: `host.on_reply(reply, ctx)`.
  - The `Agents`/`Templates`/`GraphEditorSettings`/`Failed` arms (`:767-786`) go.
- These pass-throughs switch to the host: `:1255`, `:1291`, `:1460`, `:1486`, `:1494`, `:1509`, `:1592`,
  `:1598`, `:1616`.
- Move the `GraphEditor` import into the test module, or featureless clippy fails on an unused import.

## 2. Tasks

### T1: Workspace-qualified labels and the ambiguity refusal

**Files and symbols:**
- `catalogue.rs`: `WorkspaceNames`, `ProjectCatalogue`, `snapshot`; delete `graph_label`; add a test
  module (there is none yet).
- `item_writes.rs`: `ItemFormContext` and its `Debug`, `catalogue`, the new `named`, the `serve` arms at
  `:238` and `:292`, the test literal at `:1052`.
- `item_form.rs`: `picker_label` (`:968`/`:993`), the import at `:43`.
- `divergence.rs`: `Labels::graph` (`:459`/`:474`), the import at `:33`. Use `find_map` to return the
  context with the graph, then `context.label(graph)`.
- `kinds.rs`: the import at `:49`, `not_a_project_graph` `:649`, `kind_line` `:1227`, `edit_kind_fields`
  `:1766`, `graph_named`, the new `ambiguous_graph`, and the docs at `:1761` and `:1797`.
- `tests/kinds.rs`.

**Tests first:**

| File | Tests |
|---|---|
| `catalogue.rs` | `workspace_names_label_each_scope`, `same_named_workspaces_read_by_slug`, `an_unread_workspace_keeps_the_bare_word`, `the_snapshot_names_the_workspaces_of_shared_and_kept_graphs` |
| `item_writes.rs` | `the_form_names_each_workspace_of_its_graphs`, `a_kept_item_graph_is_named_too`, `a_divergence_context_names_its_workspaces`; extend `form_and_divergence_debug_print_no_body_and_no_paths` so `workspaces` prints as a count |
| `item_form.rs` | `two_workspaces_flow_graphs_cycle_to_two_labels` |
| `divergence.rs` | `two_workspaces_flow_graphs_read_apart` |
| `kinds.rs` (unit, catalogue built with `catalogue::snapshot` over a `MemStore`) | `graph_named_resolves_an_exact_qualified_label`, `graph_named_resolves_a_unique_bare_name`, `graph_named_refuses_an_ambiguous_bare_name_with_both_labels`, `graph_named_refuses_an_ambiguous_old_style_label`, `graph_named_keeps_current_when_ambiguous`, `ambiguous_graph_names_the_count_and_the_labels` |
| `tests/kinds.rs` | `a_kind_line_names_its_workspace`, `an_ambiguous_graph_name_is_refused_with_both_labels`. Type bare `flow` key by key; paste the qualified label with `bench.paste`, since it contains `·` |

**Validate:**
- `cargo test -p htui --features testkit --lib -- catalogue item_writes item_form divergence kinds`
- `cargo test -p htui --features testkit --test kinds -- --test-threads=1`
- `cargo insta test -p htui --features testkit`
- `cargo clippy -p htui --all-features -- -D warnings`
- `cargo clippy -p htui -- -D warnings`

### T2: `GraphEditorHost`

**Files:** `graph_editor/host.rs` (new); `graph_editor/mod.rs` (`mod`, `pub use`, module doc).

**Fixture:** a `HostBench` and `served` of its own. `mod.rs`'s `Bench` is private to
`graph_editor::tests` and works on `&mut GraphEditor`.

**Tests first:**
- `open_by_id_asks_for_the_body_and_the_personas`
- `each_scope_opens_by_id_edits_saves_and_rereads` (table over project, workspace, library)
- `a_missing_graph_is_refused_and_close_still_answers`
- `a_refused_body_read_says_the_stores_sentence`
- `another_graphs_body_is_not_taken`
- `personas_reach_the_editor_before_and_after_it_opens` (`Personas` and `Catalogue`)
- `with_body_sends_exactly_the_editors_three_reads`
- `the_editors_own_replies_pass_through`
- `loading_draws_one_line_and_offers_close`
- `debug_prints_no_step_names`

**Validate:** `cargo test -p htui --features testkit --lib -- graph_editor::host`, plus both clippy runs.

### T3: Kinds opens through the host

**Files:** `kinds.rs` only.

**Test first:** `a_refused_templates_read_reaches_the_editor_through_the_host`, after
`the_editors_settings_read_is_never_the_sections` (`:2136`).

**Pin that moves:** `kinds.rs:2171-2176` builds `Mode::Graph { editor: Box::new(editor) }`.

**Validate:** `cargo test -p htui --features testkit --test kinds --test graph_editor --
--test-threads=1`, `cargo insta test -p htui --features testkit`, both clippy runs.

### T4: Entry-point acceptance

**File:** `tests/graph_editor.rs`. Drive the host with `SectionBench::new().await` and
`&mut bench.ctx()`. `ctx()`, `KeyChord::parse` and `to_event` are public, so no testkit change is
needed.

**Tests first:**
- `the_entry_point_edits_every_scope_by_id_and_the_save_resolves`
- `a_stale_save_through_the_entry_point_rebases`
- `an_unknown_graph_id_opens_nothing`
- `a_kind_line_names_its_workspace_and_refuses_an_ambiguous_name` (`kinds_over`)

**Validate:** `cargo test -p htui --features testkit --test graph_editor -- --test-threads=1`.

### T1 ∥ T2

The file sets are disjoint, as long as T2:
- maps personas in `host.rs`;
- builds no `ProjectCatalogue` literal (it uses `served(Catalogue(..))` or `CatalogueSnapshot {
  projects: Vec::new(), .. }`);
- leaves `kinds.rs` alone;
- adds no snapshots.

`testkit.rs` needs no change. No `.snap` moves: the only `· workspace` in a snapshot is the unrelated
`tests/snapshots/project_types__browse.snap:11`.

## 3. Pins that move

| Where | Now | Becomes |
|---|---|---|
| `divergence.rs:908`, `:975` | `"wsflow \u{b7} workspace"` | `… workspace Platform` |
| `item_form.rs:1764`, `:1855` | same | `… workspace Platform` |
| `tests/kinds.rs:2119` | `"DOC  docs \u{b7} graph wsflow \u{b7} workspace"` | `… workspace Graphics` |
| `tests/kinds.rs:2133` | `"graph         wsflow \u{b7} workspace"` | `… workspace Graphics` |
| `item_writes.rs:1052-1059` | `ItemFormContext` literal | + `workspaces: WorkspaceNames::default()` (import at `:398`) |
| `kinds.rs:2171-2176` (T3) | `Mode::Graph { editor }` literal | the host |

Callers of `graph_label`: `divergence.rs:474`, `item_form.rs:993`, `kinds.rs:649`, `:1227`, `:1766`,
`:1814`, `:1823`. Imports that go: `divergence.rs:33`, `item_form.rs:43`, `kinds.rs:49`. Docs that move:
`catalogue.rs:127`, `divergence.rs:455`, `kinds.rs:1761` and `:1797`.

Still green under D3: `tests/kinds.rs` ~1960-2059 (a bare name picks the project's own graph, a unique
exact match in step 2), and the library pins `:1952` and `:3648`.

## 4. Plan corrections

1. `item_form.rs:1253` is not an `ItemFormContext` literal. It is the return type of a `context()` helper
   that calls `serve`.
2. D2's fill sites are wrong. `item_graph` returns an `Option<StepGraph>`, and `catalogue()` also serves
   mint and edit validation. Fill once in the `ItemForm` and `Diverged` arms instead.
3. D4's pass-through list was incomplete. It is now `takes_paste`, `on_mouse_lost`, `graph`,
   `set_personas` and `editor_mut` (for `select`/`open_node_form`); `busy` is unused by Kinds.
4. The pins at `tests/kinds.rs:2119` and `:2133` become **Graphics**: that graph lives in
   `WORKSPACE_GRAPHICS`.
5. Linking `PROJECT_HTUI` into Graphics works for the `item_writes`, `item_form` and `divergence` tests.
   The Kinds bench scope is Graphics with only `PROJECT_VULKAN` (a fresh demo store), so the Kinds tests
   (T1, T4) link **`PROJECT_VULKAN` into Platform** instead.
6. `pub(crate) ambiguous_graph` can't be called from the integration tests. Hardcode the sentence there,
   as `tests/kinds.rs:1863` already does for `NO_GRAPH_NAMED`.
7. T1's validation now includes `cargo insta test`.
8. The `· workspace` pins are written as `\u{b7}` escapes, so grepping for the literal character misses
   them.
9. The workspace and library graphs the tests create have no steps, so `e`+`2`+`enter` has nothing to
   edit. Seed each through `store.replace_graph`, e.g. with `GRAPH_VULKAN_ANA`'s body under new ids
   (check first whether the store validates template names). T4's resolve uses `pin_feat_1`
   (`VULKAN_FEAT_1`), so its workspace graph goes in **`WORKSPACE_GRAPHICS`**.
10. T3 owns the compile break at `kinds.rs:2171` and the now-unused `GraphEditor` import.
11. The `graph_editor/mod.rs` module doc needs amending once `host.rs` exists.

## Data flow

- **Labels:** store worker → `snapshot` or the item-form context → `WorkspaceNames::read` → the reply →
  every list labels through `context.label(graph)`.
- **Open by id:** host → `GraphBody` + `Personas` → `host.on_reply` → editor open → its three reads; then
  `ReplaceGraph`/`GraphReplaced` go through the host.
- **Kinds:** `with_body`; the section keeps its catalogue arms and hands every other reply to the host.

## Build sequence

1. T1 ∥ T2.
2. T3, after T1.
3. T4.
4. Full validation: fmt, both clippy runs, `--all-features --test-threads=1`, insta, `scripts/gate.sh`,
   validator.
5. At close, update HANDOFF.md ~1115, which describes the ambiguity.
