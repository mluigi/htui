# Plan: MOD-141 M3 - Every scope, and the picker fix

**Source PRD**: `.claude/prds/mod-141-step-graph-editor.prd.md`
**Selected Milestone**: 3 - Every scope, and the picker fix
**Complexity**: Medium
**Routing**: PRD path continuing, `plan` on milestone 3; no ultracode (C1-C4 none fired for M3 alone;
accepted 2026-11-12).

## Summary

M3 has two parts. First, a scope-generic **entry point**, `GraphEditorHost`, opens the step-graph editor
on any graph by id or by a body the host already holds. It reads the body and personas itself, routes the
editor's replies, and is tested end to end for a project, a workspace and a library graph. `Settings >
Kinds` switches to it and stays project-graphs-only in the UI. MOD-124 wires the Toolbox to it. Second,
the **picker fix**: a workspace graph's label always names its workspace (`flow · workspace Platform`).
The item form's graph picker, the divergence view and every Kinds line therefore tell two workspaces'
same-named graphs apart. A typed graph name in a kind form that matches more than one graph is refused
and lists the labels to type instead.

## Maintainer decisions (2026-11-12, at planning)

| # | Question | Decision |
|---|---|---|
| M-1 | When does the workspace name appear in a label? | **Always**: every workspace graph reads `name · workspace <workspace name>`. A label never changes when a sibling graph appears, so typed labels stay valid. |
| M-2 | Is a typed label still accepted when it has exactly one match? | **Yes**. The exact qualified label always resolves. A bare `flow`, or the old `flow · workspace`, resolves while it names exactly one graph. Once it names two or more, it is refused and the refusal lists the qualified labels. |

## Decisions (D)

- **D1 Workspace names travel with the catalogue.** New `catalogue::WorkspaceNames`, a small sorted list
  of `(WorkspaceId, name, slug)` with `label(name, scope) -> String`:
  - a project graph is its bare name;
  - a library graph is `name · library`, unchanged;
  - a workspace graph is `name · workspace <name>`, using the workspace's slug instead of its name when
    two listed workspaces share a name (`workspace.slug` is unique, `name` is not);
  - a workspace no read found (`workspace(id)` answered `None`) is `name · workspace`, today's label.

  The free fn `graph_label(name, scope)` goes; every caller (7 sites, verified) labels through the
  `WorkspaceNames` its context carries.
- **D2 Filled where the graphs are read.**
  - `catalogue::snapshot` fills `ProjectCatalogue::workspaces` with one `WriteStore::workspace(id)` read
    per distinct `GraphScope::Workspace` in `shared` and `kept`. That is per event, never per keystroke
    (the module's D3 trade).
  - `item_writes::catalogue` and `item_graph` fill `ItemFormContext::workspaces` the same way, over
    `graphs` and `item_graph`.
  - `ItemFormContext`'s hand-written `Debug` prints the count only (review L2's rule).
- **D3 Typed resolution refuses ambiguity (M-2).** `graph_named` in `kinds.rs` collects candidates instead
  of taking the first `find`:
  1. `current`, when its label equals the text;
  2. else every graph (own and shared) whose label equals the text;
  3. else every shared graph whose bare name equals the text, or whose old-style `name · workspace`
     equals it.

  One candidate resolves. Two or more answer a new `pub(crate)` sentence,
  `ambiguous_graph(text, labels)`:
  `"flow" names 2 graphs; type one of: flow · workspace Platform, flow · workspace Graphics`.
  None keeps `NO_GRAPH_NAMED`. `kept` graphs stay out of typed resolution, as today. The store still
  re-checks the write, so this stays a convenience.
- **D4 `GraphEditorHost` (the entry point).** New `crates/htui/src/ui/graph_editor/host.rs`.
  - **Open by id**: `open_by_id(graph: StepGraphId) -> (Self, Vec<StoreRequest>)` sends
    `[GraphBody(graph), Personas]` and holds `Loading { graph, personas }`.
  - **Open on a held body**: `with_body(body, personas, scope) -> (Self, Vec<StoreRequest>)` is
    `GraphEditor::open` wrapped. It sends no persona read, because Kinds' personas come from its
    catalogue and replies are broadcast to every Settings section, verified.
  - **Replies**: `on_reply(&mut self, reply, ctx) -> bool` (taken or not) handles:
    - `GraphBody(Some(body))` whose id matches the pending one: open the editor and send its reads;
    - `GraphBody(None)`, or `Failed { "graph_body" }`: a refusal notice (`graph not found`, or the
      store's sentence), leaving nothing open and `Close` available;
    - `Personas(rows)`: `set_personas`, mapped to `PersonaSummary`;
    - `Catalogue(snapshot)`: `set_personas(&snapshot.personas)`. The editor's refused-undo path still
      asks for `Catalogue` (R1 M1), so either host answers it;
    - every reply `GraphEditor::on_reply` takes (`GraphReplaced`, `Agents`, `Templates`,
      `GraphEditorSettings`, and their `Failed`s).
  - **Pass-through**: `on_key`, `on_mouse`, `on_paste`, `render`, `stack`, `hint_text`, `busy` and
    `wants_requests` pass through, with a loading line while `Loading`.
  - No reply carries the requested id on `None`, so a host has one open in flight. A second `open_by_id`
    replaces the first.
- **D5 Kinds adopts the host.** `kinds::Mode::Graph` holds `Box<GraphEditorHost>` built with `with_body`.
  `on_editor_reply` keeps the section's own catalogue handling (snapshot, `clamp_cursor`, the R1 L1
  re-read) and hands the rest to `host.on_reply`. The duplicated match arms for
  `Agents`/`Templates`/`GraphEditorSettings`/`Failed` go. Kinds still refuses a kind's workspace or
  library default graph (`graph … is not edited here`): UI reach stays project-only (PRD framing). Its
  label now names the workspace (D1).
- **D6 Header names the workspace, not required.** Out of scope. The editor header keeps
  `workspace graph`; MOD-124 may pass a label when it wires the Toolbox.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / labels | `crates/htui/src/catalogue.rs:126-136` `graph_label` | `\u{b7}`-joined label, scope word last; one labeler every list shares (MOD-138 D12) |
| Catalogue fill | `crates/htui/src/catalogue.rs:162-201` `snapshot`; `item_writes.rs:331-347` `catalogue` | N+1 store reads per event, torn reads skipped (`None` → fallback label, never an error) |
| Typed resolution | `kinds.rs:1806-1827` `graph_named`, `NO_GRAPH_NAMED` | `Result<Id, String>`, refusal sentence a `pub(crate)` constant/fn, store re-checks |
| Host routing | `kinds.rs:737-786` `on_editor_reply` | match on `StoreReply`, `Failed { request }` by request name, returns `bool` taken |
| Editor open | `graph_editor/mod.rs:501-526` `GraphEditor::open` → `(Self, Vec<StoreRequest>)` | constructors return the reads they need; host sends them |
| Errors | `item_writes.rs` `refused`/`offline`; worker `Failed { request, message }` | refusal in words, never a panic |
| Unit tests | `graph_editor/mod.rs:1598-1745` `Bench` + `served` over `MemStore::demo()` | `bench.reply`/`bench.requests`; serve each request on the shared demo backend |
| E2E tests | `crates/htui/tests/graph_editor.rs` `kinds_over`/`SectionBench`/`bench_round`; `tests/kinds.rs:1227` `graph_body_reads_one_graph_of_each_scope` | `#![cfg(feature = "testkit")]`; `MemStore` clones share state |
| Duplicate setup | `item_writes.rs:563-622` (two workspace graphs); `WriteStore::upsert_workspace_project` (`traits.rs:822`) | put `PROJECT_HTUI` in `WORKSPACE_GRAPHICS` too, one `flow` graph per workspace |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui/src/catalogue.rs` | UPDATE | `WorkspaceNames`, `ProjectCatalogue::workspaces`, `snapshot` fill; `graph_label` → labeler (D1, D2) | T1 |
| `crates/htui/src/item_writes.rs` | UPDATE | `ItemFormContext::workspaces`, fill in `catalogue`/`item_graph`, `Debug`, test literal at :1052 (D2) | T1 |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | UPDATE | `picker_label` through the labeler; `context()` test helper :1253; pins :1764, :1855; duplicate test | T1 |
| `crates/htui/src/ui/tabs/backlog/divergence.rs` | UPDATE | `Labels::graph` through the labeler; pins :908, :975 | T1 |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | UPDATE | `kind_line`, `edit_kind_fields`, `not_a_project_graph`, `graph_named` (D3); `ambiguous_graph` sentence | T1 |
| `crates/htui/tests/kinds.rs` | UPDATE | pins :2119, :2133; ambiguity e2e (D3) | T1 |
| `crates/htui/tests/item_writes_pg.rs` | UPDATE (if needed) | Postgres fill of `workspaces` (compile + one assertion) | T1 |
| `crates/htui/src/ui/graph_editor/host.rs` | CREATE | `GraphEditorHost` (D4) + unit tests per scope | T2 |
| `crates/htui/src/ui/graph_editor/mod.rs` | UPDATE | `mod host; pub use host::GraphEditorHost;` | T2 |
| `crates/htui/src/ui/tabs/settings/kinds.rs` | UPDATE | `Mode::Graph` holds the host; `on_editor_reply` delegates (D5) | T3 |
| `crates/htui/tests/graph_editor.rs` | UPDATE | entry point e2e: open by id, edit, save, re-read, for each scope; `GraphBody(None)` refusal | T4 |
| `docs/htui-keys.md`, `keys.toml` | none | no new keys | — |

## Tasks

Order: **T1 ∥ T2 → T3 → T4**. T1 and T2 file sets are disjoint (T1: catalogue, item_writes, item_form,
divergence, kinds, tests/kinds, tests/item_writes_pg; T2: graph_editor/host.rs, graph_editor/mod.rs).
T3 touches `kinds.rs`, which T1 also touches, so it runs after T1. T4 needs T2 and T3.

### Task 1: Workspace-qualified labels and the ambiguity refusal (D1-D3)
- **Action**: TDD.
  - Tests first:
    - `WorkspaceNames::label` for all four cases (project, library, workspace, unknown workspace) plus
      the same-name → slug rule;
    - `snapshot` and `item_writes::catalogue` fill names for shared and kept graphs over `MemStore::demo()`
      with `PROJECT_HTUI` linked into both workspaces and a `flow` graph in each;
    - the item picker cycles to two distinct labels;
    - divergence labels;
    - `graph_named`: an exact qualified label resolves, a unique bare name resolves, an ambiguous bare
      name and an ambiguous `flow · workspace` refuse with both labels, `current` still wins;
    - a kind line names the workspace.
  - Then implement, and move the ~6 pinned `· workspace` strings.
- **Mirror**: `graph_label`, `snapshot`, `graph_named`, `item_writes.rs:563` setup.
- **Validate**: `cargo test -p htui --features testkit --lib -- catalogue item_writes item_form divergence kinds`
  and `cargo test -p htui --features testkit --test kinds -- --test-threads=1`.

### Task 2: `GraphEditorHost` (D4)
- **Action**: TDD in `host.rs`'s own `#[cfg(test)]` module.
  - Tests first, over a `Bench`-style fixture with `served` on `MemStore::demo()`. For each of project
    (`GRAPH_VULKAN_ANA`), workspace (created in `WORKSPACE_PLATFORM`) and library (created) graphs:
    - `open_by_id` sends `[GraphBody(id), Personas]`;
    - serving them opens the editor; its header reads `… · {scope} graph · revision n`, and it sends
      `Agents`/`Templates`/`GraphEditorSettings`;
    - an edit + save produces one `ReplaceGraph` on that id and revision;
    - serving it answers `Applied`, and a fresh `GraphBody` read shows the edit.
  - Also:
    - `GraphBody(None)` → refusal notice, nothing open;
    - a `GraphBody` for another id is not taken;
    - a `Catalogue` or `Personas` reply sets personas;
    - `with_body` sends exactly the editor's three reads.
- **Mirror**: `graph_editor/mod.rs` `Bench`, `served`, `each_scope_saves_its_own_graph_and_revision`.
- **Validate**: `cargo test -p htui --features testkit --lib -- graph_editor::host`.

### Task 3: Kinds opens through the host (D5)
- **Action**: Replace `Mode::Graph { editor }` with `Mode::Graph { host }`.
  - `open_editor` uses `GraphEditorHost::with_body`. `select`/`open_node_form` go through accessors.
  - `on_editor_reply` keeps the catalogue arms and delegates the rest.
  - Close and return behaviour is unchanged. The existing Kinds and `graph_editor.rs` tests are the
    regression net; add one test that a `Failed { "templates" }` still reaches the editor through the
    host.
- **Mirror**: `kinds.rs:661-786`.
- **Validate**:
  `cargo test -p htui --features testkit --test kinds --test graph_editor -- --test-threads=1`,
  plus `cargo insta test -p htui --features testkit` (a full run, not a `.snap` grep).

### Task 4: Entry-point acceptance (PRD M3 outcome)
- **Action**: In `tests/graph_editor.rs`, write a table-driven test over the three scopes through the real
  store worker (`serve`). It opens by id with `GraphEditorHost`, adds a step and connects a `loop` edge
  with `loop_limit` 3, saves, and re-reads through `GraphBody`. The saved body must resolve
  (`htui_orch` resolve, as M2's T9 does), and a stale revision answers `Stale` and the editor rebases.
  One shell-level test (`kinds_over`) shows a kind line `graph flow · workspace Platform` and the
  ambiguity refusal typed into the kind form.
- **Mirror**: `tests/graph_editor.rs` M2 T9 cases, `tests/kinds.rs::graph_body_reads_one_graph_of_each_scope`.
- **Validate**: `cargo test -p htui --features testkit --test graph_editor -- --test-threads=1`.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless gate (memory)
cargo test -p htui --all-features -- --test-threads=1
cargo insta test -p htui --features testkit        # full snapshot run
bash scripts/gate.sh                               # milestone gate; sandbox Qdrant timeouts owed on host
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

No migration, no `.sqlx` change (`workspace(id)` already exists on every store). The merged-tree Postgres
gate stays owed on the host.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Labels move snapshots in tabs not grepped (Backlog, divergence) | Medium | Full `cargo insta test` per task, not a `.snap` grep; demo store has no workspace graphs (grep of `· workspace` in `crates/htui` finds only doc comments) |
| Replies are broadcast to every Settings section, so a `Personas` read from Kinds would reach the Personas section | Medium | D4: `with_body` sends no persona read; only `open_by_id` (Toolbox, MOD-124) does |
| `GraphBody(None)` carries no id, so two opens in flight cannot be told apart | Low | One open in flight per host; a second replaces the first (D4) |
| Kinds refactor regresses M1/M2 editor behaviour | Medium | T3 after T1, existing `tests/kinds.rs` + `tests/graph_editor.rs` as the net, `--test-threads=1` (memory: scheduling-dependent suite) |
| Workspace names equal across workspaces | Low | slug fallback (D1), tested |

## Verified claims (step 3.5, 2026-11-12)

| Claim | Verdict | Evidence |
|---|---|---|
| `graph_label` labels any workspace graph `name · workspace`, no workspace name | ✓ | `catalogue.rs:130-136` |
| `graph_label` has 7 call sites outside its definition | ✓ | divergence.rs:474; item_form.rs:993; kinds.rs:649, 1227, 1766, 1814, 1823 |
| `graph_named` takes the first match, incl. a bare-name fallback over `shared` | ✓ | `kinds.rs:1806-1827` (`find`, `.or_else(... graph.name == text)`) |
| The item form's graph field is a cycling picker, not typed | ✓ | `item_form.rs:1023` `Value::Choice`; `:710` paste ignored for `Field::Graph` |
| `WriteStore::workspace(id) -> Option<Workspace>` exists on every store and on `Writer` | ✓ | `traits.rs:810`; `mem.rs:10297`; `pg/write.rs:3218`; `writer.rs:585` |
| `Workspace` carries `name` and a unique `slug` | ✓ | `model/hierarchy.rs:14-29` |
| `ProjectCatalogue` is built in one place | ✓ | only literal at `catalogue.rs:186` |
| `ItemFormContext` literals: `item_writes.rs:339`, test `:1052`, `item_form.rs:1253` helper | ✓ | text search `ItemFormContext {` |
| `GraphEditor::open` is scope-generic and a unit test saves each scope | ✓ | `graph_editor/mod.rs:501`, `:2048` `each_scope_saves_its_own_graph_and_revision` |
| `StoreRequest::GraphBody` reads any scope; answers `None` for an unknown id; request name `graph_body` | ✓ | `catalogue.rs:413`; `store_worker.rs:1425, 1695`; `tests/kinds.rs:1227` |
| `StoreReply::Personas(Vec<Persona>)` exists for a `Personas` request | ✓ | `store_worker.rs:425, 1830` |
| Settings broadcasts every reply to every section | ✓ | `settings/mod.rs:453-457` |
| Editor's refused-undo path asks the host for `Catalogue(scope)` | ✓ | `graph_editor/mod.rs:735-797` (R1 M1 arm) |
| Kinds only opens project graphs and refuses a shared default graph | ✓ | `kinds.rs:661-706`, `:646-654`; `tests/kinds.rs:1952, 3648` |
| `upsert_workspace_project` can put one project in two workspaces | ✓ | `traits.rs:822` |
| T1 and T2 touch disjoint files; T3 shares `kinds.rs` with T1, so it runs serially | ✓ | Files-to-Change table |
| No migration or `.sqlx` change | ✓ | the only new store read is the existing `workspace(id)` |
| `item_form.rs:1253` is an `ItemFormContext` literal | ✗ (architect) | it is a `context()` helper's return type; nothing changes there |
| D2: `catalogue`/`item_graph` fill the names | ✗ (architect) | fill in the `ItemForm` and `Diverged` arms via `named()`; `catalogue()` also serves mint/edit |
| Kinds tests link `PROJECT_HTUI` into Graphics | ✗ (architect) | the Kinds bench scope is Graphics + `PROJECT_VULKAN`; link Vulkan into Platform |
| Test-created workspace/library graphs are editable as-is | ✗ (architect) | they have no steps; seed them through `replace_graph` |

Amended by the blueprint (`mod-141-m3-every-scope-picker-fix.blueprint.md` §4, which wins on any
difference): the D4 pass-through list, the `tests/kinds.rs:2119/:2133` pins move to `Graphics`, T1
validation adds `cargo insta test`, and the `graph_editor/mod.rs` module doc changes.

## Acceptance

- [ ] `GraphEditorHost` opens, edits and saves a project, a workspace and a library graph by id (T2, T4)
- [ ] `Settings > Kinds` opens through the host; M1/M2 tests unchanged and green (T3)
- [ ] Two workspaces' `flow` graphs read `flow · workspace Platform` / `flow · workspace Graphics` in the
      item picker, the divergence view and Kinds (T1)
- [ ] An ambiguous typed graph name is refused naming the qualified labels; a unique one still resolves (T1)
- [ ] Validation passes (sandbox Qdrant timeouts excepted, owed on host)
- [ ] Patterns mirrored, not reinvented
