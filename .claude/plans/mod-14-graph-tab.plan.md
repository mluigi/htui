# Plan: MOD-14 - Graph tab

**Status: CONFIRMED by the maintainer 2026-09-30 with the defaults (D2 max 3 / default 2, D7 status-line refusal, D8 `m`); implemented and closed out 2026-09-30 (`docs/decisions/mod/mod-14.md`).**

**Source**: `HANDOFF.md:408-411` (MOD-14, from MOD-1). Requirements `R-TUI-5` ("Graph tab: the item
and everything connected, one to N hops, across projects, with status and link kind, navigable"),
`R-TUI-3` (Graph is a sub-tab of the Backlog's right pane), `R-TUI-2` (the `open graph` action),
`R-ENT-9` (directed edges by UUID, four kinds, cross-project for free, no manual link action).
Design authority: MOD-1's skeleton (`docs/decisions/mod/mod-1.md`: "Graph at one hop", traversal
deferred to MOD-14) and the MOD-64 reveal path (`Action::Reveal`, D235/D251).

**Routing**: routed as plan by `/handoff-run MOD-14` (0 of C1-C4 fired; low confidence on C3 — hop
count, terminal layout, and tab-vs-pane were left to this plan; they are D1-D4 below). Ultracode
not needed. Staffing: session model (Opus 5.5) for every agent, no `model:` override.

**Base**: `hr/MOD-14` @ `4b1c323`. No migration, no `.sqlx` entry, no new dependency, no store
change: every backend already walks N hops, both directions, across projects, with per-node depth
(`PgStore::links` `pg/read.rs:168`, `MemStore` `State::link_graph` `mem.rs:992`, `CacheStore::links`
`cache/read.rs:373`).

**Numbering**: MOD-14's own plan. Decisions **D1…D8**, tasks **T1…T3**. Global D-numbers are
assigned at close-out (other sandboxes are minting concurrently).

---

## Requirements restatement

1. The Backlog detail pane's **Graph** sub-tab shows the selected item's link neighbourhood to a
   depth the user can change (1…N), not the flat one-hop table it shows today.
2. Every edge shows its **link kind** and direction; every node shows its **status**; a node in
   another project is labelled with its project slug.
3. The graph is **navigable**: a cursor moves over nodes, and choosing a node **re-roots the Backlog
   selection** on it (the list cursor moves, every sub-tab re-reads, the Graph sub-tab stays open and
   now shows the new item's neighbourhood).
4. `R-TUI-2`'s **`open graph`** action: one key on the Backlog list jumps to the Graph sub-tab.
5. Read-only: `R-ENT-9` forbids a manual link action, and MOD-25 makes an offline box read-only. This
   item writes nothing.

## Design decisions

| # | Decision | Why |
|---|---|---|
| D1 | **The Graph stays a Backlog detail sub-tab** (`ui/tabs/backlog/detail/graph.rs`), not a new top-level tab. | `R-TUI-3` lists Graph among the right-pane tabs; `R-TUI-1`'s top-level tab list (Backlog, Chat, Skills, Requirements, Settings) has no Graph. The HANDOFF line scopes the work to replacing `graph.rs`. |
| D2 | **Fetch once at `MAX_HOPS = 3`, filter the view locally.** The Backlog's `HOPS` constant (`backlog/mod.rs:33`, MOD-1 plan D11 "one hop") becomes 3; the sub-tab keeps a *view depth* (default **2**, range **1…3**) changed with `+`/`-`, and renders only nodes with `depth <= view` and edges whose two ends are both shown. The view depth **survives item changes** (it is a preference, not per-item state). | A sub-tab has no `ctx` in `on_item_change` and cannot change what `go()` asks for without a shared cell or a second fetch. Fetching the maximum once keeps "one reply per selection" (blueprint C.2 staleness) intact, makes `+`/`-` free, and needs no new request plumbing. 3 hops of a backlog is small; the CTE's `UNION` + `depth < $2` already bound it on cycles. |
| D3 | **Layout: an indented tree, one row per edge, `cargo tree`-style.** The root is row 0. Children of a node are its neighbours one level deeper (by the store's `depth`), ordered by key, each shown under the **first** parent that reaches it (the traversal order `ORDER BY depth, key` gives). An edge that is **not** a tree edge (a cycle or a diamond: FEAT-1/ANA-1/FEAT-3 in the demo) is rendered once as a leaf row marked `(*)` under the shallower endpoint (ties by key), and that node is not expanded again. Row: `indent · arrow · kind · [slug:]KEY · status`, with the title only if columns remain (the pane is 43 columns wide at 100x30). | Every edge appears exactly once with its kind (R-TUI-5 "link kind per edge"), cycles terminate, and the reader sees the shape of the neighbourhood, not a flat list. A drawn node-link diagram does not fit 43 columns. |
| D4 | **Arrow is relative to the tree parent**: `→` when the edge is `parent --kind--> child`, `←` when it is `child --kind--> parent` — the same glyphs and meaning the current table uses relative to the root. | Keeps the one-hop reading identical to what MOD-1 shipped, so the depth-1 rows of the new view read the same as the old table. |
| D5 | **Navigation keys**: `J`/`K` move the graph cursor (the viewport follows it via `list::window`), `PageDown`/`PageUp` move it ten rows, `Enter` re-roots, `+`/`-` change the view depth. `j`/`k`/`g`/`G`/`h`/`l` stay the Backlog's. | `J`/`K` are already the detail pane's "scroll this pane" keys on every sub-tab (`detail::Scroll`), and the Backlog tab takes lower-case `j`/`k` before any sub-tab sees them (`backlog/mod.rs` `on_key`). |
| D6 | **Re-root = `Action::Reveal(RevealTarget::Item { id, key })`**, emitted by the sub-tab. The shell focuses the Backlog (already active, so no re-activation, `update.rs:56-74`) and calls `BacklogTab::reveal`, which unfolds the node's project, moves the list cursor and issues the seven reads (`select_item` → `go`). Nothing outside `graph.rs` changes for it. `Enter` on the **root** row is consumed with no effect. | Reuses MOD-64 D235/D251 end to end, including "not loaded yet → re-read and decide", the `captures_input` guard, and the staleness rule. The sub-tab never learns that a list exists. |
| D7 | **A node outside the active workspace** (its `project_id` not in `ctx.projects`) is drawn dim with its `slug:` prefix, and `Enter` on it puts `"<slug>:<KEY> is outside this workspace"` on the status line (`Action::Error`) instead of revealing. | Revealing it would only produce `not_in_this_backlog` after a pointless `Items` re-read. Re-rooting the graph alone (graph root ≠ list selection) would break the invariant that the Graph shows the selected item; that is a larger design and is out of scope. |
| D8 | **`open graph` = `m` on the Backlog tab** ("map"): a Backlog-level `on_key` arm that selects the Graph sub-tab by id through a new `DetailRegistry::select_id(DetailId) -> bool`. It is also listed as a `KeyScope::Tab(BacklogTab::ID)` binding with help text `open graph`, so the help overlay shows it (the `Enter replay step` row in `app/mod.rs:107-112` is the precedent). | `m` is not used by the Backlog, any sub-tab, or the global table (verified). A static binding cannot select a sub-tab (no such `Action`), so the arm does the work and the binding only documents it, exactly as the `Enter` row does. |

**Maintainer choices this plan asks for at CONFIRM** (defaults above; any can be changed):
D2 (max 3 / default 2), D7 (status-line refusal vs graph-local re-root), D8 (the `m` key).

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Sub-tab shape | `detail/runs.rs`, `detail/graph.rs` | `DetailTab` impl; state reset in `on_item_change`; `on_reply` matches its one `StoreReply` variant; `message()` for empty states |
| Cursor + viewport | `backlog/list.rs:219-226` `window()`; `Selection` by id | Stateless viewport from the cursor; cursor clamped on reply |
| Emitting actions | `backlog/mod.rs:276-277`, `concepts_search.rs::target` | `ctx.emit(Action::Reveal(..))` / `ctx.emit(Action::Error(..))` |
| Help-only binding | `app/mod.rs:102-112` | `KeyScope::Tab(BacklogTab::ID)` row whose action is the miss |
| Unit tests | `backlog/mod.rs` `mod tests` | `MemStore::demo()` + `Ctx::new(..)` + `Emit::default()`; assert on `emit.take()` |
| Integration + snapshots | `tests/backlog.rs` `backlog()`, `down()`, `sub_tab()`; `insta::assert_snapshot!` | `harness.key(..)`, `drive_to_end().await`, `render()` |

## Files to change

| File | Action | Why |
|---|---|---|
| `crates/htui/src/ui/tabs/backlog/detail/graph.rs` | REWRITE | Tree model (D3/D4), cursor (D5), view depth (D2), re-root (D6/D7), render; unit tests |
| `crates/htui/src/ui/tabs/backlog/detail/mod.rs` | UPDATE | `DetailRegistry::select_id` (D8); module doc "one hop" wording |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | `HOPS` 1 → 3 with the D2 comment; `m` arm (D8); module doc line |
| `crates/htui/src/app/mod.rs` | UPDATE | `m` help binding on the Backlog scope (D8) |
| `crates/htui/tests/backlog.rs` | UPDATE | Re-root, open-graph, depth and out-of-workspace integration tests |
| `crates/htui/tests/snapshots/backlog__detail_graph.snap`, `backlog__empty_graph.snap` | REGENERATE | New layout / empty text; reviewed, not blindly accepted |
| `crates/htui/tests/snapshots/backlog__graph_*.snap` | CREATE | New cases (depth 1/3, re-rooted) |
| `docs/decisions/mod/mod-14.md`, `DECISIONS.md`, `HANDOFF.md` | CREATE/UPDATE | Close-out (lifecycle P2) |

## Tasks (serial — T2 and T3 build on T1's API; one crate, one pane)

### T1: Graph model and sub-tab (`graph.rs`)
- **Tests first** (unit, in `graph.rs`): tree from the demo `FEAT-1` graph has each live edge once,
  the FEAT-1/ANA-1/FEAT-3 triangle yields one `(*)` row and terminates; the tombstoned
  `TOOL-1 relates FEAT-1` edge is absent; view depth 1 reproduces the old three edges with the same
  arrows; `+`/`-` clamp to 1…3 and survive `on_item_change`, while the cursor resets; `J`/`K` clamp;
  `Enter` on an in-scope node emits exactly one `Action::Reveal(Item{..})`; on the root emits
  nothing and is `Consumed`; on an out-of-workspace node (agy `FIX-1` → vulkan `FEAT-1`) emits one
  `Action::Error` naming it; `Enter` with no graph loaded is `Consumed` (so the Backlog's
  `replay step` miss does not fire from the Graph).
- **Action**: pure `fn rows(graph, view_depth) -> Vec<GraphRow>` (unit-testable without a frame),
  cursor over rows, render per D3.
- **Validate**: `cargo test -p htui --lib ui::tabs::backlog::detail::graph`

### T2: Wiring (`backlog/mod.rs`, `detail/mod.rs`, `app/mod.rs`)
- **Tests first**: `select_id` true/false; `m` on the list selects Graph and is `Consumed`; `m`
  while a sub-tab captures input still goes to the sub-tab (existing capture test pattern).
- **Action**: `HOPS = 3`; `m` arm; `select_id`; help binding.
- **Validate**: `cargo test -p htui --lib ui::tabs::backlog`

### T3: Integration tests and snapshots (`tests/backlog.rs`)
- Re-root: `FEAT-1` → Graph → `J` to `ANA-1` → `Enter` → the detail frame is titled `ANA-1`, the list
  cursor is on htui `ANA-1`, and the Graph sub-tab is still the active one.
- Cross-project re-root inside the workspace: from htui `FEAT-2`, reach agy `FEAT-1` and `Enter` → the
  list cursor is on agy `FEAT-1` (group unfolded if it was folded).
- Out of workspace: agy `FIX-1` → vulkan node → `Enter` → status line error, list cursor unchanged.
- `m` from Body lands on Graph; `+`/`-` snapshots at depth 1 and 3.
- Regenerate `detail_graph` / `empty_graph` and review the diffs by eye.
- **Validate**: `cargo test -p htui --features testkit --test backlog`, then `cargo insta pending-snapshots` is empty.

## Validation (gates)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1   # scheduling-dependent suite; `tests/backlog.rs` is `#![cfg(feature = "testkit")]`
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A hub item makes a 3-hop fetch large on every `j`/`k` | Low (backlog-sized data) | `MAX_HOPS` is one constant; rows past the viewport are never styled; can drop to 2 without design change |
| 43-column pane cannot fit indent + kind + key + status + slug | Medium | Titles dropped first, indent capped, key column sized to content; the snapshot at 100x30 pins it |
| `Enter` routed differently by Backlog (header row folds first) | Low | Graph only sees `Enter` on an item row — the only row with a graph; covered by T3 |
| Reveal lands while a later `j` already moved on | Low | Existing D251 staleness path; no new state |
| Existing snapshots change (`detail_graph`, `empty_graph`) | Certain | Expected; regenerated in T3 and reviewed |

## Out of scope

- Graph-local re-root without moving the list (D7 alternative), a "back" history, link editing
  (`R-ENT-9` forbids a manual action; MCP-proposed links are MOD-11).
- Filters on the Backlog (MOD-13), a top-level Graph tab.

## Acceptance

- [ ] T1-T3 complete, tests written first
- [ ] Gates green
- [ ] Snapshots reviewed, no pending
- [ ] Close-out docs written, validator green

---

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Graph is a detail sub-tab, registered third | true | `backlog/mod.rs:73-79` (`BodyTab`, `RunsTab`, `GraphTab`, …) |
| `R-TUI-3` lists Graph among the right-pane tabs; no top-level Graph tab in `R-TUI-1` | true | `docs/REQUIREMENTS.md:312-320` |
| The Backlog requests `Links { id, hops: HOPS }` with `HOPS = 1`, and is the only sender | true | `backlog/mod.rs:33`, `:130`; grep of `StoreRequest::Links` over `crates/htui` |
| All three backends walk N hops, both directions, across projects, return per-node `depth`, skip tombstones (Pg/Mem) | true | `pg/read.rs:168-241`, `mem.rs:992-1045`; cache mirrors live rows (`cache/read.rs:373`) |
| `LinkNode` carries `project_id`, `project_slug`, `status`, `depth`; `LinkEdge` carries `kind` | true | `htui-core/src/model/link.rs:46-86` |
| Pg orders nodes `depth, key` | true | `pg/read.rs` `ORDER BY n.depth, i.key` |
| A sub-tab's `on_item_change` has no `ctx` (why D2 fetches the max) | true | `detail/mod.rs` `DetailTab::on_item_change(&mut self, item)` |
| Backlog consumes `j k g G h l [ ] ←→↑↓`; passes everything else (incl. `+ - J K Enter` on item rows) to the active sub-tab | true | `backlog/mod.rs` `on_key` |
| `m`, `+`, `-` are unused by the Backlog, every sub-tab and the global table | true | char-key census of `backlog/**`; global table `keymap.rs` `default_global` + `app/mod.rs:73,84` (`w`, `ctrl-f`) |
| Key chain: active tab first, then its `KeyScope::Tab` binding, then global | true | `app/state.rs:472-512` |
| A tab-emitted `Action::Reveal` is dispatched (drain → `update`) and routed to `BacklogTab::reveal` | true | `app/state.rs:343-362`; `update.rs:197-238`; `app/mod.rs:98-101` |
| `Focus` on the already-active tab does not re-activate it | true | `update.rs:48-75` (`activate_tab` only when `active_id` changed) |
| `BacklogTab::reveal` refuses while a sub-tab captures, unfolds and selects when loaded, re-reads otherwise | true | `backlog/mod.rs` `reveal`, `select_item` |
| `go()` resets every sub-tab via `on_item_change` but keeps the active sub-tab index | true | `backlog/mod.rs` `go`; `DetailRegistry::on_item_change` touches tabs only |
| `list::window` and `list::clip` are `pub` | true | `backlog/list.rs:219`, `:206` |
| `ctx.projects` is the scope's `&[ProjectRef]` with `project_id` | true | `ListView { projects: ctx.projects }`, `list.rs` `groups` |
| Demo has a triangle (FEAT-1/ANA-1/FEAT-3), a tombstoned edge (TOOL-1→FEAT-1), a cross-project edge inside Platform (agy FEAT-1 → htui FEAT-2) and one leaving it (agy FIX-1 → vulkan FEAT-1) | true | `htui-core/src/fixtures.rs:1130-1178`; Platform = htui + agy (`tests/backlog.rs:46`) |
| Snapshots pinning today's Graph are `backlog__detail_graph` and `backlog__empty_graph` only | true | `tests/backlog.rs:164`, `:184`; `tests/snapshots/` listing |
| `harness.key("+")` / `("-")` parse as plain chars | true | `KeyChord::parse`: a one-character spec is that character (`keymap.rs`) |
| `tests/backlog.rs` only compiles with `--features testkit` (without it: 0 tests, silently green) | true | `tests/backlog.rs:6`; `crates/htui/Cargo.toml:25`; baseline 20 passed at `e38a5ab` |
| Tasks are independent | **false — serial** | T2 uses T1's `GraphTab` API and T3 exercises both; all three touch `crates/htui` only. No fan-out. |
