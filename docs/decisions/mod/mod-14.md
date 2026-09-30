# MOD-14 - Graph tab (done, 2026-09-30)

**Requirements:** `R-TUI-5` (the item and everything connected, one to N hops, across projects, with
status and link kind, navigable), `R-TUI-3` (Graph is a sub-tab of the Backlog's right pane),
`R-TUI-2` (the `open graph` action), `R-ENT-9` (directed links by UUID; no manual link action).
**Origin:** MOD-1 (`docs/decisions/mod/mod-1.md`), which shipped the Graph sub-tab as a one-hop table
and deferred traversal here.
**Artifacts:** plan with its verified-claims table:
[`.claude/plans/mod-14-graph-tab.plan.md`](../../../.claude/plans/mod-14-graph-tab.plan.md);
blueprint:
[`.claude/plans/mod-14-graph-tab.blueprint.md`](../../../.claude/plans/mod-14-graph-tab.blueprint.md).
No PRD. Routed as a plan on 2026-09-30 with 0 of C1-C4 fired (low confidence on C3: hop count,
layout, and tab-vs-pane were left to the plan, which settled them as D1-D8). The maintainer confirmed
the plan with its defaults.
**Commits:** `b32a1c1` plan, `e38a5ab` confirmed, `2c1a783` gate fix, `0852eaa` blueprint, `30b0426`
tree sub-tab (T1), `d7935bb` `m` opens the Graph (T2), `4e06fd9` integration tests (T3), then review
fixes `8794a0a`, `91241ea`, `f2ff9c8`, `1ad7d72`, `63f0708`, then this write-up.

## What shipped

The Backlog detail pane's **Graph** sub-tab (`crates/htui/src/ui/tabs/backlog/detail/graph.rs`) draws
the selected item's link neighbourhood as an indented tree, one row per live link:

```
▸ FEAT-1 in_progress TUI scaffold
  → origin     ANA-1 done Data model, box …
    ← blocked_by agy:ANA-1 done Prompt ass…
    ← origin     FEAT-3 queued (*)
    ← blocked_by TOOL-1 awaiting_approval
  ← blocked_by FEAT-2 blocked Agent driver…
    ← relates    agy:FEAT-1 open ACP trans…
  ← relates    FEAT-3 queued Postgres stor…
depth 2/3 · +/- depth · Enter re-root
```

- **Depth** — the Backlog now asks `Links { hops: 3 }` (`HOPS = GraphTab::MAX_HOPS`) once per
  selection; `+`/`-` change a view depth of 1-3 (default 2) locally, with no new request. The depth
  is a preference and survives selection changes; the cursor does not.
- **Tree** — each node hangs under its first parent one level up, in byte order of `slug:key`, so
  the shape is the same whatever order a backend returns rows in (Postgres sorts by collation,
  `MemStore` by BFS). A link that is not a tree edge (a cycle or diamond) is drawn once as a `(*)`
  row under the shallower end, and its node is not expanded twice. Every live link appears exactly
  once.
- **Row** — the arrow is relative to the tree parent (`→` the parent's link, `←` the child's), then
  kind, key (prefixed `slug:` for another project), status, and the title if there is room. Width is
  measured in terminal cells (`ui::cells::cell_width`), graphemes are never split, and the kind
  column is padded for all rows or none. When room runs out the title goes first, then the kind
  padding, then the slug, then the status; the key goes last.
- **Navigation** — `J`/`K` and `PageDown`/`PageUp` move a cursor (`▸`); the viewport follows it.
  `Enter` re-roots: it emits `Action::Reveal(RevealTarget::Item { .. })`, so MOD-64's reveal path
  moves the Backlog cursor, unfolds the project and re-reads all seven detail views; the Graph stays
  the active sub-tab and shows the new root. `Enter` on the root is consumed and does nothing.
- **Outside the workspace** — a node whose project is not in the scope is drawn dim, and `Enter` on
  it says `<slug>:<KEY> is outside this workspace` on the status line instead of revealing.
- **States** — `Loading links…` until the reply arrives, `Links unavailable: <message>` after a
  refused `links` read, `No links for this item.` when there are none. A reply for another item is
  ignored.
- **`open graph`** — `m` on the Backlog selects the Graph sub-tab (`DetailRegistry::select_id`); a
  `KeyScope::Tab(backlog)` binding lists `m open graph` in the help box.

No store, schema, `.sqlx` or dependency change: the three backends already walked N hops in both
directions, across projects, with a per-node depth.

## Decisions

| # | Decision |
|---|---|
| D1 | Graph stays a Backlog detail sub-tab (`R-TUI-3`), not a top-level tab. |
| D2 | Fetch at 3 hops once per selection; view depth 1-3, default 2, changed locally. Supersedes MOD-1 plan D11's one hop. |
| D3 | `cargo tree`-style tree, one row per link, `(*)` for non-tree links. |
| D4 | The arrow is relative to the tree parent, as the old table's was relative to the root. |
| D5 | `J`/`K`, `PageDown`/`PageUp` move, `Enter` re-roots, `+`/`-` depth; the Backlog keeps `j k g G h l`. |
| D6 | Re-root is `Action::Reveal`, reusing MOD-64 D235/D251; nothing outside `graph.rs` changes for it. |
| D7 | A node outside the workspace refuses with a status-line message; graph-only re-root is not built. |
| D8 | `open graph` is `m`. |

## Review

`rust-reviewer` approved with warnings: 3 MEDIUM and 6 LOW, all fixed. The fixes: a clamped depth
change moved the cursor; the tree was rebuilt on every key and frame; long functions were split;
widths were counted in chars; the narrowest label cut the key; the kind column was ragged; the
loading and failure states were missing; there were test gaps (`Enter` under the real registration,
every width 0-43, a short pane's scrolling); and two comments were wrong. The re-review confirmed
all nine and found one regression from the narrow-label fix: at 80 columns (a 34-cell pane) a row
could lose its key. `63f0708` fixed it, so the status now yields before the key.

The blueprint's notes that widths are `chars().count()` and clipped by `list::clip` describe the
first cut, not the shipped code, which uses terminal cells and a grapheme clip.

## Not done

- The Backlog help line still reads `Enter replay step` while the Graph is shown; the Graph's own
  footer names its keys. A per-sub-tab help line is a shell change.
- `Enter` on a self-loop `(*)` row reveals the current item, which is a no-op.
- Re-rooting the graph alone on an item outside the workspace (D7's alternative), a back history,
  and link editing (`R-ENT-9`: agents propose links through MOD-11).
- On the offline cache, a node reached only through an item whose project is not mirrored is dropped
  along with its links. The cache cannot return that item, so the node has no parent one level up.

## Verification

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
and `cargo test --workspace --all-features -- --test-threads=1`: 2873 passed, 0 failed.
`tests/backlog.rs` is `#![cfg(feature = "testkit")]`, and without the feature it runs 0 tests and
still reports green. It went from 20 to 27 tests. It has six Graph snapshots: `detail_graph`,
`empty_graph`, and `graph_{depth_1,depth_3,re_rooted,outside_workspace}`. The Postgres gate
on the merged tree runs on the host.
