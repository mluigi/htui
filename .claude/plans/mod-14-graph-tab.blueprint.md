# MOD-14 blueprint: Graph sub-tab traversal

> **Superseded detail (review, 2026-09-30):** widths are measured in terminal cells (`ui::cells::cell_width`) with a grapheme clip, not `chars().count()` / `list::clip` as written below; the label yields title → kind padding → slug → status, the key last. See `docs/decisions/mod/mod-14.md` § Review.

Plan: `.claude/plans/mod-14-graph-tab.plan.md` (CONFIRMED, D1-D8 with defaults; gate fix `2c1a783`).
Base `2c1a783` on `hr/MOD-14`. The plan's decisions stand as written. Where the code contradicts the
plan's prose, the difference is listed under **Plan deviations** (§6) and resolved there without
changing a decision.

House rules that apply to every task:
- rustfmt `max_width = 100`. Run `cargo fmt --all` before each commit.
- Lints: `[workspace.lints.clippy] all = warn`, and **pedantic is deliberately NOT enabled**
  (`Cargo.toml` last line). No `cast_*`, `must_use_candidate` or `missing_errors_doc` pressure. The
  gate is `clippy --all-targets -- -D warnings`, so every `clippy::all` warning fails it. The rust
  lints are `missing_debug_implementations = warn` (derive `Debug` on every new type) and
  `unused_qualifications = warn`. `crates/htui/src/lib.rs` has `#![warn(missing_docs)]`, so every
  `pub` item needs a doc. Private items are documented anyway (house style). The rustdoc lints
  (`private_intra_doc_links = deny`) only run under `cargo doc`, but do not link a `pub` doc to a
  private item.
- `tests/backlog.rs` is `#![cfg(feature = "testkit")]`. Without `--features testkit` it runs 0 tests
  and still reports green.

---

## 1. `crates/htui/src/ui/tabs/backlog/detail/graph.rs` (T1, REWRITE)

### 1.1 Types

```rust
/// An edge that leaves the parent row's item (D4).
const OUT: &str = "\u{2192}";
/// An edge that arrives at the parent row's item (D4).
const IN: &str = "\u{2190}";
/// Marks the row under the cursor (the `runs.rs` / `requirements.rs` glyph).
const CURSOR: &str = "\u{25b8}";
/// Default view depth (D2).
const DEFAULT_DEPTH: u8 = 2;
/// Columns one tree level indents by.
const INDENT: usize = 2;
/// Width the kind column is padded to: `blocked_by` / `supersedes`, the old table's `Length(10)`.
const KIND_WIDTH: usize = 10;
/// Narrowest title worth drawing: below it a clipped title is an ellipsis and a letter or two.
const TITLE_MIN: usize = 8;
/// Marks a non-tree edge (D3).
const REPEAT: &str = " (*)";

/// Which way an edge points relative to the row it hangs under (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrow { Out, In }            // + `const fn glyph(self) -> &'static str` -> OUT / IN

/// How a non-root row is reached from the row it hangs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Link {
    /// `→` / `←` relative to the parent row's item.
    arrow: Arrow,
    /// The edge's kind.
    kind: LinkKind,
    /// Not a tree edge: a `(*)` leaf whose item is expanded under its own tree edge.
    repeat: bool,
}

/// One line of the tree, borrowed from the `LinkGraph` it was built from.
#[derive(Debug, Clone, Copy, PartialEq)]      // LinkNode is PartialEq only, so no Eq
struct GraphRow<'a> {
    /// `0` for the root, one more than the row it hangs under otherwise.
    level: u8,
    /// `None` on the root row.
    link: Option<Link>,
    /// The item this row names.
    node: &'a LinkNode,
}

/// The selected item's link neighbourhood as a navigable tree (MOD-14).
#[derive(Debug)]
pub struct GraphTab {
    /// The `Links` reply for the selected item, fetched at `MAX_HOPS`.
    graph: Option<LinkGraph>,
    /// Whether an item is selected at all.
    item: Option<ItemId>,
    /// Index into `rows()`.
    cursor: usize,
    /// View depth, `1..=MAX_HOPS`. A preference: survives `on_item_change` (D2).
    depth: u8,
}

impl GraphTab {
    /// Identity of the Graph sub-tab.
    pub const ID: DetailId = DetailId("graph");
    /// How far the Backlog's `Links` read reaches, and the deepest view `+` reaches (D2).
    pub const MAX_HOPS: u8 = 3;
    /// A sub-tab with no graph yet, at the default view depth.
    #[must_use] pub fn new() -> Self { /* explicit fields, depth: DEFAULT_DEPTH */ }
}
impl Default for GraphTab { fn default() -> Self { Self::new() } }
```

**Hazard:** today `GraphTab` is `#[derive(Default)]`, and `new()` calls `Self::default()`. A derived
default would give `depth: 0`, which is outside `1..=3`. Drop the derive and write `Default` by hand
through `new()`. `clippy::new_without_default` requires the `Default` impl. Remove the
`Scroll` import: the Graph no longer uses `detail::Scroll`. `PageDown`/`PageUp` use the parent
module's private `PAGE` (`super::PAGE`; a child module can read its ancestor's private items).

### 1.2 `fn rows(graph: &LinkGraph, depth: u8) -> Vec<GraphRow<'_>>` (pure, private)

Do **not** rely on backend order. `MemStore` returns nodes in BFS discovery order and edges in
fixture order. Pg orders nodes by `depth, key` in the database collation and returns edges
unordered. Sort in code, by bytes (`UpstreamEntry::sort_canonical`'s reasoning, `model/link.rs`).

- `node_order(a, b)` = `(depth, key bytes, project_slug bytes, item_id)`.
- `other_order` (rows under one parent) = `(other.key bytes, other.project_slug bytes,
  other.item_id, kind.as_str(), Out before In)`. D3's "ordered by key"; the slug breaks the
  `agy FEAT-1` / `htui FEAT-1` tie.

Algorithm:
1. `root = graph.node(graph.root)`. If there is none, return `vec![]`.
2. **Visible nodes**: `depth <= view`, sorted by `node_order`. **Visible edges**: both endpoints
   visible (D2 filter). Sort them by `(from node_order, to node_order, kind.as_str())`.
3. **Tree parent** of each visible non-root node `n` (depth `d`): the first node in `node_order`
   with depth `d - 1` that shares a visible edge with `n`. Its **tree edge** is the first visible
   edge (edge order) joining that parent and `n`. A node with no such neighbour is dropped along
   with its edges (a malformed reply; a BFS depth always has a `d - 1` neighbour).
4. **Owner** of every other visible edge: the shallower endpoint. On equal depth, the endpoint first
   in `node_order` owns it. A self-loop is owned by its node.
5. **Preorder DFS** from the root (`level 0`, `link: None`). For node `x` at level `l`, collect every
   tree edge whose parent is `x` and every non-tree edge owned by `x`, and sort them by
   `other_order`. For each one, push
   `GraphRow { level: l + 1, link: Some(Link { arrow, kind, repeat }), node: other }`, where
   `arrow = Out` iff `edge.from_item_id == x.item_id` (a self-loop is `Out`). For a tree edge, recurse
   into `other`. A repeat row is a leaf.

Invariants (the unit tests pin these):
- `rows.len() == 1 + visible edges`: every live visible edge appears exactly once.
- Each item appears in exactly one non-repeat row, so each node is expanded once.
- The walk terminates, because a parent is strictly shallower than its child.
- The tombstoned edge never appears (no backend returns it).

### 1.3 Worked rows: demo htui `FEAT-1` (Platform scope, pane width 43)

The 3-hop reply has 8 nodes: FEAT-1 d0; ANA-1, FEAT-2, FEAT-3 d1; agy ANA-1, TOOL-1, agy FEAT-1 d2;
agy FIX-1 d3. It has 9 live edges. vulkan FEAT-1 is 4 hops away, and `TOOL-1 relates FEAT-1` is
tombstoned.

**Depth 2 (default)**: 8 rows, 7 edges. This is the new `backlog__detail_graph` pane content:
```
▸ FEAT-1 in_progress TUI scaffold
  → origin     ANA-1 done Data model, box …
    ← blocked_by agy:ANA-1 done Prompt ass…
    ← origin     FEAT-3 queued (*)
    ← blocked_by TOOL-1 awaiting_approval
  ← blocked_by FEAT-2 blocked Agent driver…
    ← relates    agy:FEAT-1 open ACP trans…
  ← relates    FEAT-3 queued Postgres stor…
```
(`FEAT-3 origin ANA-1` has both ends at depth 1. It is owned by ANA-1 (`ANA-1` < `FEAT-3`) and is
the triangle's single `(*)` row. Under ANA-1 the order is agy `ANA-1`, `FEAT-3`, `TOOL-1`.)
**One `J` from the root row reaches htui ANA-1** (row 1).

**Depth 1**: 5 rows. The three old one-hop edges, with MOD-1's arrows, plus the triangle's `(*)` row:
```
▸ FEAT-1 in_progress TUI scaffold
  → origin     ANA-1 done Data model, box …
    ← origin     FEAT-3 queued (*)
  ← blocked_by FEAT-2 blocked Agent driver…
  ← relates    FEAT-3 queued Postgres stor…
```
**Depth 3**: 10 rows, 9 edges. agy FIX-1 hangs under agy ANA-1 (`ANA-1` < `TOOL-1`), and
`agy FIX-1 origin TOOL-1` is a `(*)` row under TOOL-1:
```
▸ FEAT-1 in_progress TUI scaffold
  → origin     ANA-1 done Data model, box …
    ← blocked_by agy:ANA-1 done Prompt ass…
      ← blocked_by agy:FIX-1 open Session …
    ← origin     FEAT-3 queued (*)
    ← blocked_by TOOL-1 awaiting_approval
      ← origin     agy:FIX-1 open (*)
  ← blocked_by FEAT-2 blocked Agent driver…
    ← relates    agy:FEAT-1 open ACP trans…
  ← relates    FEAT-3 queued Postgres stor…
```

### 1.4 Worked rows: agy `FIX-1`, depth 2 (reaches out-of-workspace vulkan `FEAT-1`)

6 rows. htui ANA-1 hangs under agy ANA-1 (`ANA-1` < `TOOL-1`). `TOOL-1 blocked_by ANA-1` is a `(*)`
row under TOOL-1.
```
▸ FIX-1 open Session leak on cancel
  → blocked_by ANA-1 done Prompt assembly …
    → blocked_by htui:ANA-1 done Data mode…
  → origin vulkan-tutor…:FEAT-1 in_progress      <- dim: not in ctx.projects (D7); 43 cols
  → origin htui:TOOL-1 awaiting_approval         <- kind unpadded to fit (44 -> 40)
    → blocked_by htui:ANA-1 done (*)
```
**Three `J` from the root row reach vulkan FEAT-1.** `Enter` there emits
`Action::Error("vulkan-tutorials:FEAT-1 is outside this workspace")`. The message uses the full slug,
unclipped.

Other roots used by the tests (depth 2):
- htui `ANA-1` (after a re-root): `ANA-1 done` / `← blocked_by agy:ANA-1` / `← blocked_by agy:FIX-1`
  (L2) / `← origin FEAT-1` / `← blocked_by FEAT-2` (L2) / `← relates FEAT-3 (*)` (L2) /
  `← origin FEAT-3` / `← blocked_by TOOL-1` / `← origin agy:FIX-1 (*)` (L2). 9 rows.
- htui `FEAT-2`: `FEAT-2 blocked` / `← relates agy:FEAT-1` / `→ blocked_by FEAT-1` /
  `→ origin ANA-1` (L2) / `← origin FEAT-3 (*)` (L3) / `← relates FEAT-3` (L2). 6 rows. **One `J`
  reaches agy FEAT-1**: the `FEAT-1` key tie is broken by slug, `agy` < `htui`.

### 1.5 Line layout (pure `fn line(row, on_cursor, root_project, in_workspace, width, theme) -> Line<'static>`)

Widths are `chars().count()`, as in `list.rs`. `list::clip` does the clipping.
- Root row: `{CURSOR|' '}' '{KEY}' '{status}` then the title rule. The root carries no slug.
- Other rows: `{CURSOR|' '}' '{INDENT*(level-1) spaces}{arrow}' '{kind}' '{label}' '{status}{REPEAT?}`
  then the title rule. `label = [slug:]KEY`, where the slug is present iff `node.project_id !=
  root.project_id`. Indent is capped at `INDENT * MAX_HOPS` (6 columns). Levels only reach
  `MAX_HOPS + 1`: a `(*)` row under a depth-3 node, e.g. agy FIX-1 at depth 3 has the
  `FEAT-3 relates FEAT-1` edge at level 4. The cap therefore guards a future larger `MAX_HOPS`,
  not today's data.
- Degradation, in this order, only when the mandatory part exceeds `width`:
  1. **Titles are dropped first.** A title is drawn only when
     `room = width - used - 1 >= TITLE_MIN`, as `' ' + clip(title, room)`. Repeat rows never carry a
     title, because their item's own row does.
  2. The kind loses its padding to `KIND_WIDTH` (plain `kind.as_str()`).
  3. The slug is clipped: `slug_room = label_room - KEY - 1`, and the label becomes
     `clip(slug, slug_room) + ":" + KEY`. If `slug_room < 2`, the label becomes
     `clip(label, label_room)`.
  4. Anything left is cut at the pane edge by ratatui. Render with `Paragraph` **without** `Wrap`:
     a wrapped row would break the row-to-cursor mapping.
- Styles: `CURSOR` `accent`; arrow `dim`; kind `base`; label `accent`; status
  `theme.status_style(status)`; `(*)` `dim`; title `base`. An out-of-workspace node (D7) draws label,
  status and title in `dim`.

### 1.6 Render

- `item.is_none()` -> `message(.., "No item selected.")`.
- `graph.is_none()` or `rows.len() <= 1` -> `message(.., "No links for this item.")`. This is the new
  `empty_graph` text. It keeps `"No "` for the test, and follows the siblings' "No X for this item."
- Otherwise split with `Layout::vertical([Min(0), Length(1)])` into `[tree, footer]`. At 100x30 the
  content is 24 rows (23 tree rows + footer). `lines.split_off(list::window(cursor, len,
  tree.height))`. The footer, in `dim`, is `format!("depth {depth}/{MAX_HOPS} · +/- depth · Enter
  re-root")` (37 columns). Clamp the cursor to `len - 1` defensively.

### 1.7 Keys (`on_key`)

The Backlog consumes `j k g G h l [ ] ← → ↑ ↓ Home End` before any sub-tab. It sends `Enter` down
only on an item row, and `m` never (T2). Match on `key.code` only. **Never** require
`modifiers == NONE`: terminals send `J`, `K` and `+` with `SHIFT`.

| Key | Condition | Effect | Returns |
|---|---|---|---|
| any `CONTROL`/`ALT` chord | - | none (so `ctrl-c` quits) | `Pass` |
| `J` / `K` | any, including no graph | cursor ±1, clamped to `0..=len-1` (0 when empty) | `Consumed` |
| `PageDown` / `PageUp` | any | cursor ±`PAGE` (10), clamped | `Consumed` |
| `+` | any | `depth = (depth + 1).min(MAX_HOPS)`; cursor re-found | `Consumed` |
| `-` | any | `depth = depth.saturating_sub(1).max(1)`; cursor re-found | `Consumed` |
| `Enter` | no item / no graph yet | nothing | `Consumed` (plan T1: the `replay step` miss must not fire from the Graph) |
| `Enter` | cursor on the root row | nothing | `Consumed` (D6) |
| `Enter` | node's `project_id` in `ctx.projects` | `ctx.emit(Action::Reveal(RevealTarget::Item { id, key: node.key.clone() }))` (bare key, as `concepts_search::target`) | `Consumed` |
| `Enter` | node outside `ctx.projects` | `ctx.emit(Action::Error(format!("{slug}:{key} is outside this workspace")))` | `Consumed` |
| anything else | - | - | `Pass` |

"Cursor re-found" means: before the change, record the cursor row's `node.item_id`. After it,
move to that item's **non-repeat** row, or to row 0 if the item is no longer shown. The row index
moves because preorder changes with depth.

`on_item_change(item)`: `self.item = item; self.graph = None; self.cursor = 0;`. Depth is **kept**.
`on_reply(Links(g))`: ignore it unless `Some(g.root) == self.item` (a cheap guard on top of the
shell's staleness index); otherwise `self.graph = Some(g.clone()); self.cursor = 0`.

Module doc: replace the "one hop, as a flat table ... `ctx.request(Links ..)` ... nothing outside
this file" paragraph with the D2/D3/D6 summary. The fetch is at `MAX_HOPS` and filtered locally;
the rows form a tree; `Enter` re-roots through `Action::Reveal`, so the sub-tab never names the list.

---

## 2. Wiring (T1 line, then T2)

### 2.1 `backlog/mod.rs` `HOPS` (moved to T1, see §6 D-4)
```rust
/// How far the link traversal behind the Graph sub-tab reaches: its deepest view (MOD-14 D2).
/// Fetched once per selection and filtered by `+`/`-` locally, so a depth change costs no read and
/// each of the seven reads stays one reply per selection (blueprint C.2).
const HOPS: u8 = GraphTab::MAX_HOPS;
```
Deriving `HOPS` from `GraphTab::MAX_HOPS` keeps "one constant" (plan risk row). Its value is 3, as D2
says. `GraphTab` is already imported at `:16`.

### 2.2 `detail/mod.rs`: `DetailRegistry::select_id` (T2). Place it right after `select`:
```rust
/// Activates the sub-tab registered under `id` (MOD-14 D8: `m` opens the Graph). `false` (and no
/// change) when nothing is registered under it.
pub fn select_id(&mut self, id: DetailId) -> bool {
    match self.tabs.iter().position(|tab| tab.id() == id) {
        Some(idx) => self.select(idx),
        None => false,
    }
}
```
Do not add `#[must_use]`. `select` has none, and the `m` arm ignores the result.

### 2.3 `backlog/mod.rs` `on_key`: the `m` arm (T2)
Put it inside the `match key.code`, **after** both guards (`captures_input` first, then the
`CONTROL|ALT` guard) and after the `h`/`[`/`Left` arm, before `Enter`:
```rust
// MOD-14 D8, `open graph`. Below the capture guard, so an `m` typed into a note stays the note's.
KeyCode::Char('m') => {
    self.detail.select_id(GraphTab::ID);
}
```
It falls through to the trailing `Handled::Consumed`. The module doc's line "graph traversal MOD-14
— each of them lands as a DetailTab body or a binding, not as a change here" becomes true only for
the others. Reword it: "graph traversal MOD-14 (its hop count and the `m` key are the two lines it
adds here)".

### 2.4 `app/mod.rs`: help binding (T2)
Append it after the `Enter replay step` binding (`:107-112`), so the help box reads
`Enter replay step · m open graph`:
```rust
// MOD-14 D8: `m` opens the Graph sub-tab. The Backlog's own arm does it (no `Action` selects a
// sub-tab) and always consumes the key, so this row never fires: it is the help box's half, as the
// `Enter` row is. Its action is a no-op focus of the tab that is already active.
app.keymap.bind(Binding {
    scope: KeyScope::Tab(BacklogTab::ID),
    key: KeyChord::new(KeyCode::Char('m'), KeyModifiers::NONE),
    action: Action::Tab(TabAction::Focus(BacklogTab::ID)),
    help: "open graph",
});
```
`TabAction` is already in scope via the module's `pub use action::{..}`. Add item 8 to the
`register_all` doc list.

---

## 3. Tests (write each block first, watch it fail, then implement)

### T1 unit tests: `graph.rs` `#[cfg(test)] mod tests`
Fixtures:
- `async fn demo(root: ItemId) -> LinkGraph` = `MemStore::demo().links(root, GraphTab::MAX_HOPS)`
  (`ReadStore as _`).
- `async fn platform() -> (Scope, Vec<ProjectRef>)`, as in `backlog/mod.rs` tests.
- `fn shape(rows) -> Vec<String>` renders each row as `"{2*(level-1) spaces}{glyph} {kind}
  {slug:}KEY{ (*)}"`, with the root as `KEY`, so a whole tree is one `assert_eq!`.
- `Ctx::new(.., Origin::Tab(BacklogTab::ID), &emit)` and `emit.take()`. `Action` is not
  `PartialEq`: use `matches!` and compare the `RevealTarget` (which is).
- For synthetic graphs, build ids with `ItemId::from_uuid(Uuid::from_u128(n))` as `model/link.rs`
  tests do (same for `ProjectId`).

1. `feat_1_at_depth_2_is_a_tree_of_every_live_edge_once`: `shape(rows(FEAT-1, 2))` equals the 8
   lines of §1.3, and `rows.len() == 1 + 7`.
2. `the_triangle_yields_one_repeat_row_and_terminates`: at depth 3, exactly one repeat row names
   FEAT-3. Every item has exactly one non-repeat row. `rows.len() == 1 + graph.edges.len()` (9).
3. `a_tombstoned_edge_is_not_drawn`: at depths 1..=3, no row has TOOL-1 at level 1 under FEAT-1.
   TOOL-1's only non-repeat row is `← blocked_by` under ANA-1.
4. `depth_1_keeps_the_one_hop_arrows`: the level-1 rows are exactly
   `[(Out, origin, ANA-1), (In, blocked_by, FEAT-2), (In, relates, FEAT-3)]`, and the only other
   row is `← origin FEAT-3 (*)` at level 2 (§6 D-2).
5. `backend_order_does_not_change_the_tree`: reversing `graph.nodes` and `graph.edges` gives the
   same `shape` at depths 1..=3.
6. `a_cross_project_parent_tie_is_broken_by_key`: `shape(rows(agy FIX-1, 2))` equals §1.4's six
   rows.
7. `parallel_edges_and_a_self_loop_each_get_one_row` (synthetic): root R with `R blocked_by X`,
   `R relates X` and `R relates R`. The tree has one tree row plus two `(*)` rows, `len == 1 + 3`.
8. `plus_and_minus_clamp_and_survive_an_item_change`: default 2; `-`,`-` -> 1; `+`x3 -> 3;
   `on_item_change(Some(other))` keeps 3, and the cursor is 0 and the graph `None`.
9. `j_k_and_paging_clamp_to_the_rows`: on FEAT-1 depth 2, `K` at 0 stays 0; 20 `J` -> 7;
   `PageUp` -> 0; `PageDown` -> 7. `J` with no graph is `Consumed`, cursor 0.
10. `a_depth_change_keeps_the_cursor_on_its_item`: cursor on agy FEAT-1 (row 6 at depth 2); `+`
    -> row 8; `-`,`-` (depth 1, agy FEAT-1 gone) -> row 0.
11. `enter_on_a_node_in_the_workspace_reveals_it`: FEAT-1, `J`, `Enter` gives `Consumed` and
    exactly one `Reveal(Item { id: HTUI_ANA_1, key: "ANA-1" })`.
12. `enter_on_the_root_does_nothing`: `Consumed`, `emit.is_empty()`.
13. `enter_outside_the_workspace_says_so`: agy FIX-1 in Platform, `J`x3, `Enter` gives `Consumed`
    and exactly one `Error("vulkan-tutorials:FEAT-1 is outside this workspace")`, with no `Reveal`.
14. `enter_with_no_graph_is_consumed`: `on_item_change(Some(id))`, no reply, `Enter` gives
    `Consumed` and no emit. The same holds with `item == None`.
15. `a_reply_for_another_root_is_ignored`.
16. `other_keys_pass`: `j`, `x`, `q`, `ctrl-c` (`KeyEvent::new(Char('c'), CONTROL)`) -> `Pass`.
17. `every_line_fits_the_pane`: for FEAT-1 and agy FIX-1 at depths 1..=3, every `line(.., 43, ..)`
    has width <= 43 (`const PANE: usize = 43`; comment: the detail pane's inner width at
    `DEFAULT_SIZE`, pinned by `the_detail_strip_fits_the_detail_pane`). Assert the exact text of
    the vulkan row `"  → origin vulkan-tutor…:FEAT-1 in_progress"` and the TOOL-1 row
    `"  → origin htui:TOOL-1 awaiting_approval"`.

T1 also regenerates `backlog__detail_graph` and `backlog__empty_graph` (§6 D-4). The expected detail
pane is §1.3 depth 2 plus the footer on the last content row. The expected empty pane is
`No links for this item.`

### T2 unit tests: `backlog/mod.rs` `mod tests`
1. `select_id_finds_a_registered_sub_tab`: `BacklogTab::new().detail.select_id(GraphTab::ID)` gives
   `true` and `active_id() == Some(GraphTab::ID)`. `DetailId("nope")` gives `false`, and the active
   sub-tab is unchanged.
2. `m_on_the_list_opens_the_graph`: Platform items, cursor on the first item, `BacklogTab::new()`
   fields as in the capture test; `on_key('m')` gives `Consumed`, `active_id() == GraphTab::ID`,
   the list cursor is unchanged, and `emit.is_empty()` holds (no re-read).
3. `m_while_a_sub_tab_captures_is_the_sub_tab_s`: registry `[CapturingProbe, GraphTab]`, capturing
   = true. `m` gives `Consumed`; the probe saw `Char('m')`; the active id is still the probe's. With
   capturing = false, `m` makes the active id Graph, and the probe saw nothing new.

T2 integration (in `tests/backlog.rs`): `m_is_on_the_backlog_help_line`, using `Harness::demo()` +
`htui::app::register_all(harness.app())`, asserts that
`keymap.help_line(&KeyScope::Tab(BacklogTab::ID))` contains both `"m open graph"` and
`"Enter replay step"`.

### T3 integration tests: `tests/backlog.rs`
New helpers:
```rust
/// Steps right of Body to the Graph sub-tab.
const TO_GRAPH: usize = 2;
/// Rows down from the arrival row to htui `FEAT-2`.
const TO_FEAT_2: usize = 4;
/// `backlog()` with the Backlog named as the tab that reveals items, as `register_all` does: the
/// Graph's `Enter` is an `Action::Reveal`, which reveals nothing without this (§6 D-1).
async fn revealing_backlog() -> Harness { /* backlog().await; app().reveal_tabs = vec![(RevealKind::Item, BacklogTab::ID)] */ }
```
(Import `htui::app::RevealKind` and `htui::keymap::KeyScope`.)

1. `enter_on_a_graph_node_re_roots_the_backlog`: `revealing_backlog`, `down(3)`,
   `sub_tab(TO_GRAPH)`, `J`, `enter`, `drive_to_end`. The frame has `"┌ ANA-1"`, `"agy:ANA-1"` (only
   an htui root labels it so), `"depth 2/3"` (Graph still active), and `status == None`. Snapshot
   `graph_re_rooted`. Then `j` + drive gives `"┌ ANA-2"`, which proves the list cursor was on htui
   ANA-1 and not agy ANA-1.
2. `enter_reveals_a_node_of_the_other_project_and_unfolds_its_group`: fold agy with `G`, drive,
   `k`x3 (drive each), `enter`, drive. Assert that `"ACP transport upgrade"` is absent. Then `g`,
   `down(5)` gives `"┌ FEAT-2"`. Then `sub_tab(TO_GRAPH)`, `J`, `enter`, `drive_to_end`: the frame has
   `"┌ FEAT-1"`, `"htui:FEAT-2"`, `"▾ agy (3)"` and `"ACP transport upgrade"`. Then `j` gives
   `"┌ FIX-1"`.
3. `enter_on_a_node_outside_the_workspace_refuses_on_the_status_line`: `revealing_backlog`, `G`,
   drive (`"┌ FIX-1"`), `sub_tab(TO_GRAPH)`, `J`x3, `enter`, drive. Status is
   `Some("vulkan-tutorials:FEAT-1 is outside this workspace")`, and the frame still has `"┌ FIX-1"`.
   Snapshot `graph_outside_workspace`; it pins §1.4's clipping at 100x30.
4. `m_opens_the_graph_from_any_sub_tab`: `down(3)`; on Body, `m` gives a frame with `"depth 2/3"`;
   `sub_tab(1)` (Runs), then `m` gives `"depth 2/3"` again.
5. `plus_and_minus_change_the_graph_depth`: `down(3)`, `m`, `-` gives snapshot `graph_depth_1`
   (§1.3 depth 1); `+`,`+` gives snapshot `graph_depth_3`; one more `+` still shows `"depth 3/3"`;
   `j` + drive (FEAT-2) still shows `"depth 3/3"` (D2 survives item change).

---

## 4. Build sequence and commits (serial; one commit per task, explicit `git add <paths>`)

**T1** `feat(mod-14): the Graph sub-tab draws the link neighbourhood as a navigable tree`
Files: `detail/graph.rs`, `backlog/mod.rs` (the `HOPS` line and its doc only), and the two
regenerated snapshots.
Validate:
- `cargo test -p htui --lib ui::tabs::backlog::detail::graph`
- `cargo insta test -p htui --features testkit --test backlog`. Read each `.snap.new` against §1.3
  and §1.6, then `cargo insta accept`.
- `cargo test -p htui --features testkit --test backlog`, then `cargo insta pending-snapshots` is
  empty. (`cargo-insta 1.48.0` is installed.)

**T2** `feat(mod-14): m opens the Graph sub-tab`
Files: `detail/mod.rs`, `backlog/mod.rs`, `app/mod.rs`, `tests/backlog.rs` (the help-line test
only).
Validate:
- `cargo test -p htui --lib ui::tabs::backlog`
- `cargo test -p htui --features testkit --test backlog m_is_on`
- `cargo test -p htui --features testkit --test replay` (the `Enter` miss and its help line are
  untouched)

**T3** `test(mod-14): re-root, cross-project reveal, out-of-workspace refusal and depth snapshots`
Files: `tests/backlog.rs` and 4 new `tests/snapshots/backlog__graph_*.snap`.
Validate: `cargo test -p htui --features testkit --test backlog`, then `cargo insta
pending-snapshots` is empty.

Gates, after T3 (plan, as fixed in `2c1a783`):
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
Never commit `*.snap.new` or `*.pending-snap`.

## 5. Hazards (no plan change)

- **SHIFT**: see §1.7. `KeyChord::parse("J")` in the harness has no `SHIFT`; real terminals add it.
- **Default depth 0**: see §1.1.
- **Wrap**: rows must not wrap (§1.5.4).
- **Order**: see §1.2. A test that passes on `MemStore` order alone proves nothing for Pg (test 5).
- **Enter consumed**: this diverges from `RunsTab`'s "never consume a key it cannot act on" (its
  `Enter` passes to the miss). This is deliberate (plan T1): the Graph's `Enter` means re-root, and
  "select a step in the Runs pane" would be the wrong answer here. Say so in a comment.
- **Reveal from a tab**: `App::reveal` runs inside `drain` after the tab's `on_key` borrow ended
  (`update.rs:197-238`). `select_item` → `go` resets every sub-tab (graph `None`, cursor 0, depth
  kept) and re-issues the seven reads under `Origin::Tab(backlog)`. Older replies are dropped by the
  staleness index. There is no new state.
- **Width is char-counted** (`list::clip`, same as the list pane): a title with wide glyphs can be
  one or two cells longer than computed, and ratatui cuts it at the edge. Keys, slugs and statuses
  are ASCII.
- **`LinkNode` has no `Eq`**, so `GraphRow` derives `PartialEq` only. `derive_partial_eq_without_eq`
  is a nursery lint and is not enabled.

## 6. Plan deviations (reported, decisions unchanged)

- **D-1 (T3)**: the plan's re-root tests assume `backlog()` reveals. It does not.
  `app.reveal_tabs` is set only by `register_all`, and `backlog()` registers the tab by hand, so an
  `Action::Reveal` is a logged no-op there ("no tab reveals this kind", `update.rs:199-208`).
  Resolution: the `revealing_backlog()` helper sets `reveal_tabs` (the field is `pub`). Production
  behaviour is as D6 says.
- **D-2 (T1 test "depth 1 reproduces the old three edges")**: D2's own rule ("edges whose two ends
  are both shown") also shows `FEAT-3 origin ANA-1` at depth 1, because both ends are one hop from
  FEAT-1. The old table dropped edges between neighbours. Depth 1 is therefore the old three rows,
  with identical arrows, plus one `(*)` row. D2 wins; the test asserts the three plus the one.
- **D-3 (files table, `detail/mod.rs` "one hop wording")**: `detail/mod.rs` has no "one hop"
  wording. The stale prose is in `graph.rs`'s module doc (rewritten in T1) and in `backlog/mod.rs`'s
  module doc and `HOPS` doc (T1/T2 per §2).
- **D-4 (task boundaries)**: T1 rewrites the renderer, which makes `backlog__detail_graph` and
  `backlog__empty_graph` fail. Leaving them for T3 would give two red commits. With `HOPS = 1` the
  depth-2 view would also show only one hop, so a T1-only regeneration would be wrong and redone.
  Resolution: the one-line `HOPS = GraphTab::MAX_HOPS` moves from T2 to T1 (it references T1's
  constant anyway), and T1 regenerates both snapshots. Every commit is green; T3 keeps the new
  snapshots.
- **D-5 (D8's binding "action is the miss")**: the `m` arm always consumes the key, so no miss can
  fire. The binding carries a no-op `Tab(Focus(BacklogTab::ID))` (update.rs re-activates only on a
  change of active tab) and exists for the help box, which is D8's stated purpose.
- **Additions the plan left open (render detail, not decisions)**: a one-line footer
  `depth N/3 · +/- depth · Enter re-root` (the view depth has to be visible for `+`/`-` to be
  usable), no title on `(*)` rows, the `▸` cursor glyph from `runs.rs`/`requirements.rs`, the empty
  text `No links for this item.`, and the root-mismatch guard in `on_reply`.
