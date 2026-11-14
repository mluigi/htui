# Plan: MOD-147 M4 — Search

**Source PRD**: `.claude/prds/mod-147-workspace-scope.prd.md`
**Selected Milestone**: 4 — Search
**Complexity**: Medium (concepts index payload and indexer, one scope resolver, CLI and TUI search wiring, MCP hit mapping)
**Routing**: plan path on PRD milestone 4 (C2 only, C4 borderline); ultracode not needed. Two tasks, so the
light shape of `SKILL.md` step 4 (one implementer per task, serial); T1 is the foundation and gets one verifier.

## Summary

Owner documents (a `document` row with `project_id` and no `item_id`, MOD-147 M1) are indexed in today's
collection as document points owned by the document, `Owner::Document`. An owner-document pass runs per project
beside the item loop, and the payload schema stamp moves from 2 to 3, so an existing collection is dropped and
refilled. A workspace-scoped search (the TUI search overlay in "all projects" mode, `htui --search-items`,
`--index-items` and the worker's index job) covers the member projects plus the workspace's scope row. That
row's `WS` items and its `goals` document are then both indexed and found. The MCP `search_concepts` tool keeps
its one-project scope (MOD-112). It does map an owner-document hit, which a project's own documents can now
produce.

## Decisions taken in this plan

| # | Decision | Why |
|---|---|---|
| D1 | **The point model** (`crates/htui-store/src/vector.rs`). `Subject` gains `OwnerDocument { id: DocumentId }`, and `Owner` gains `Document(DocumentId)` (ANA-35 §7.5's name). `Subject::owner` maps `OwnerDocument { id }` to `Owner::Document(id)`. `status`, `resolution`, `state` and `version` answer `None` for it. `Owner::item`/`requirement` answer `None` for `Document`; add `Owner::document(self) -> Option<DocumentId>` beside them. `ConceptPoint::point_type` is `PointType::Document` for `(OwnerDocument, _)`. `ConceptPoint::id` is `document_point_id(doc.id, doc.chunk)`, the same derivation an item's document uses (the document ID differs, so the IDs never meet). The `document` field stays `Some(DocumentRef)` on every owner-document point; the doc comments "only an item has documents" (`ConceptPoint.document`, `payload`) are reworded | One new subject, and no new `PointType`: a `types: [Document]` filter, the TUI's place word and the CLI's `place: "document"` keep working |
| D2 | **The payload** of an owner-document point: `type: "document"`, `key` (D4), `project_id`, `updated_at` (the row's `created_at`), `snippet`, `document_id`, `doc_kind`, `doc_version` and `chunk`. There is **no** `item_id`, `kind_id`, `status` or `resolution` (ANA-35: "payload `document_id`, no `item_id`, no `status`"). `owner_of` reads a `Document` point as `Owner::Item(item_id)` when `item_id` is present, else as `Owner::Document(document_id)`. `indexed_point` requires `status` only on an `Item` point or a `Document` point that has an `item_id`; an owner document's `status` is `None`. `hit` needs no change beyond `owner_of`. A status or resolution filter leaves owner documents out, as it leaves out requirements (a point missing the key fails `match any`). `MemVectorStore` already does the same through `Subject::status()`/`resolution()` | The payload a reader can tell apart without a new type |
| D3 | **`PAYLOAD_SCHEMA` 2 → 3**, with its doc comment ("3: MOD-147 M4's owner-document points"). `COLLECTION` keeps its name (PRD decision: no `v4` rename while htui is in development). Every payload-stamp test is written relative to the constant (`vector.rs` `an_older_payload_stamp_is_rebuilt`, `a_newer_payload_stamp_is_refused_not_rebuilt`, `qdrant_live.rs:810,867,910,1059`), so none needs editing | `check_existing` already drops and recreates an older stamp, and the next sync refills it |
| D4 | **Key and text of an owner-document point**: `key` is the owner project's slug (`@platform` for a scope row, `htui` for a project), read once per project with `ReadStore::project`. Text: `"{slug} {title} ({kind})\n\n{section}"`, chunked with `chunk_markdown(body, MAX_CHUNK_CHARS)` like `document_points` (an empty body gives one point). One new `pub fn owner_document_points(owner: &Project, doc: &OwnerDocument) -> Vec<ConceptPoint>` in `vector_sync.rs`, beside `document_points` | `Hit.key` is non-optional and is what the CLI, the TUI and MCP print. The slug names where the document lives, and `@` marks a workspace's own |
| D5 | **The owner-document pass**: `async fn sync_owner_documents(read, project, indexed: HashMap<DocumentId, Vec<IndexedPoint>>, store) -> Result<SyncReport>`, a separate function beside `sync_requirements` (PRD risk: easy to move into MOD-159's indexer). It reads `read.owner_documents(project)` once (full rows, bodies included) and keeps the newest version per kind. An owner-document row is never updated (a new version is a new row), so a latest document whose ID already has points is in step, and a latest one without points is built. Points whose document is no longer a latest are deleted. In `Indexer::sync` the `store.indexed(project)` split gains `Owner::Document(id) => owner_points…`. The pass runs after `sync_requirements` and before the item orphans are deleted, so an owner-document point is never taken for an item orphan. When a document is built and the owner row is gone (`project()` answers `None`), the pass writes nothing for that project. `SyncReport` gains no field: the pass counts in `points_upserted` and `points_deleted` | Same incremental, idempotent contract as items and requirements (`vector_sync.rs` module doc, which gains a paragraph) |
| D6 | **The workspace search scope resolver**: `pub async fn workspace_search_scope(read: &impl ReadStore, scope: &Scope) -> Scope` in `vector_sync.rs`. It returns `scope.with_scope_row(row)` (`htui-core/src/model/scope.rs:47`, which puts the row first) when `read.scope_project(scope.workspace_id)` answers `Some(row)`. It returns `scope.clone()` for a nil workspace (no lookup, as `scope_row_ref` does at `store_worker.rs:3013`), for a workspace with no scope row, and for a failed lookup. A failed lookup is logged with `tracing::warn!` (workspace and error), mirroring `serve_items`' fallback (`store_worker.rs:2984-2994`) | One resolver for every caller (PRD: "ships as a resolver used by the TUI and CLI search"). A refused lookup costs the workspace's own row, not the search |
| D7 | **CLI, `--index-items` and the worker job** (`crates/htui/src/concepts.rs`). `scopes(pg, project)` (`:181`) is the one scope source for `search_projects` (`:672`), `index_items` (`:211`) and `PgStore::index_scopes` (`:268`, the worker's job). With no `project`, each workspace's scope is widened through `workspace_search_scope`, and a workspace with no member projects is kept when it has a scope row. **`--project @<ws-slug>`** narrows to that workspace's scope row alone: a slug starting with `@` matches `WorkspaceSummary.slug` after the `@` and resolves the row through `scope_project`. Any other slug matches member projects as today. An unmatched `@` slug is `unknown_project`, as an unmatched slug is today. `--cwd` is unchanged. The `index_scopes` and `scopes` doc comments are updated | `R-STO-8`: search covers workspace-owned documents. Without the widening, neither the CLI nor the worker would ever index the scope row |
| D8 | **TUI requests**. `StoreRequest::SearchConcepts(SearchQuery)` becomes `SearchConcepts { query: SearchQuery, scope_row_of: Option<WorkspaceId> }`, and `IndexConcepts { scope }` becomes `IndexConcepts { scope: Scope, scope_row_of: Option<WorkspaceId> }`. `Some(ws)` means "also that workspace's scope row, resolved off the loop". The `ConceptsRuntime::serve` tasks (`concepts_worker.rs:1220`) take a `backend.clone()` and widen through `workspace_search_scope`: a search widens a `Scope { workspace_id: ws, project_ids: query.projects }` and searches a clone of the query with the widened projects. The reply's `ConceptsReply::Hits { query }` stays the **request's** query, so the overlay's `sent == query` match is unchanged. The empty short-circuits become `projects.is_empty() && scope_row_of.is_none()`. Pattern sites move from `SearchConcepts(_)` to `SearchConcepts { .. }`: `store_worker.rs:2736` (`try_serve`), `:3709` (`spawn_with_concepts`), `testkit.rs:403` (`Harness.drive`), the overlay (`concepts_search.rs:253` and `:278` for the two requests, `:854` and `:1383` in its tests), the `concepts_worker.rs` tests (`:1434-2895`: 10 `SearchConcepts` and 6 `IndexConcepts` sites) and `store_worker.rs:4193` | Explicit intent: one project cycled to with `Ctrl+P` must not pull the scope row in. `Ctx` and `Scope` stay members-only (M1 D1, M3 D2) |
| D9 | **The overlay** (`crates/htui/src/ui/overlay/concepts_search.rs`). In "all projects" mode (`self.project == None`), `enter` and `reindex` send `scope_row_of: Some(ctx.scope.workspace_id)` when the workspace is not nil. With a project cycled to, they send `None`. `NO_PROJECTS` is refused only when there are no projects and the workspace is nil. The `Ctrl+P` cycle and the header are unchanged | The overlay is the TUI's workspace-scoped search |
| D10 | **Enter on an owner-document hit**: `target(hit)` returns `Option<RevealTarget>`, `None` for `Owner::Document`. On `None`, `enter` keeps the overlay open and sets a new notice `OWNER_DOCUMENT: &str = "a project or workspace document: open it from Settings › Hierarchy"`. No `RevealTarget` variant is added | Owner documents live in the Settings documents pane (M3 D9); revealing into Settings is MOD-116's to place |
| D11 | **MCP mapping**. `htui_mcp::search::OwnerKind` gains `Project` ("a project's or a workspace's own document; `key` is its slug"), serialised `"project"`. `crate::mcp_search::concept_hit` (`mcp_search.rs:71`) maps `Owner::Document(_) => OwnerKind::Project`. The tool's scope stays the session's one project (MOD-112). `docs/htui-mcp.md:137` lists `owner_kind` as `item`, `requirement` or `project` | A project's own documents now index in that project, so a session can see them |
| D12 | **Docs**: `docs/htui-cli.md`. The `--search-items` default (`:92`) covers each workspace's own items and documents too, and `--project @<ws>` narrows to them. `hits[].key` (`:312`) is "the item's or requirement's key, or the owning project's slug on a project or workspace document". `hits[].place` (`:313`) says `"document"` is an item's document or a project's or workspace's own. The CLI JSON adds no field, so its version stays (verify the rule the doc states). The `cli.rs` `--project` help says `@<workspace>` selects the workspace's own items and documents | User-visible behaviour change |
| D13 | **Out of scope, recorded**: MCP `search_concepts` widening (MOD-112). Per-workspace collections and the indexer lease (MOD-159). A Settings reveal for owner-document hits (MOD-116). A `Ctrl+P` entry for the scope row alone. A slug rename re-keying owner-document points: the key is the slug at index time, and a renamed project's points keep the old slug until a new version of that document is written | Keeps M4 to the PRD row |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Indexer pass | `crates/htui-store/src/vector_sync.rs` `sync_requirements` (`:135-174`) | one listing, stale points rebuilt in one upsert, gone rows' points deleted, counts into `SyncReport` |
| Point builder | `vector_sync.rs` `document_points` (`:237-260`) | heading line + `chunk_markdown` sections, empty body → one point |
| Payload / owner parse | `vector.rs` `payload` (`:566`), `owner_of` (`:637`), `indexed_point` (`:650`) | keys absent rather than null; `owner_of` by type |
| Scope-row widening with fallback | `crates/htui/src/store_worker.rs` `serve_items` (`:2978-3002`) | lookup failure → `tracing::warn!` + members alone |
| Off-loop concepts work | `concepts_worker.rs` `ConceptsRuntime::serve` (`:1220-1297`) | `tokio::spawn` per request, reply carries the request's own query |
| Tests | `vector_sync.rs` tests over `MemStore` + `MemVectorStore`; `concepts.rs` `search_projects_default_to_every_workspace_and_project_narrows` (`:984`, demo Postgres); `qdrant_live.rs` (live group, `testkit::qdrant_turn()` held for the body) | real stores and fakes, no mocks |

## Files to Change (per task; independence is decided by these sets)

| Task | Files |
|---|---|
| T1 Owner-document points, the pass, the resolver, hit mapping | UPDATE `crates/htui-store/src/vector.rs`, `crates/htui-store/src/vector_sync.rs`, `crates/htui-store/tests/it/qdrant_live.rs`, `crates/htui-mcp/src/search.rs`, `crates/htui-mcp/tests/it/tools_search.rs`, `crates/htui/src/mcp_search.rs`, `crates/htui/src/ui/overlay/concepts_search.rs` (D10 only), `docs/htui-mcp.md` |
| T2 Workspace-scoped search in the CLI, worker job and TUI | UPDATE `crates/htui/src/concepts.rs`, `crates/htui/src/cli.rs`, `crates/htui/src/concepts_worker.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/ui/overlay/concepts_search.rs` (D8, D9), `crates/htui/tests/it/concepts_search.rs`, `docs/htui-cli.md` |

Intersection: `concepts_search.rs`, so T1 and T2 run **serial** (T1 first; T2 builds on the resolver).

## Tasks

### T1: Owner-document points, the pass, the resolver, hit mapping (tests first)

- **Action**: D1–D6, D10 and D11.
  - `vector.rs` unit tests:
    - an owner-document point's payload has `document_id`, `doc_kind`, `doc_version`, `chunk`, `key` and `project_id`, and no `item_id`, `kind_id`, `status` or `resolution`;
    - it round-trips to an `IndexedPoint` (`owner: Owner::Document(id)`, `status: None`, `document_id: Some(id)`) and to a `Hit` (`owner: Owner::Document(id)`, `document: Some((id, kind))`);
    - its `point_type` is `Document` and its `id` is `document_point_id(id, chunk)`;
    - `MemVectorStore`: a `statuses` or `resolutions` filter leaves it out, and `types: [Document]` keeps it;
    - an item's document still parses as `Owner::Item`.
  - `vector_sync.rs` unit tests (`MemStore`, writing owner documents with `WriteStore::write_owner_document`):
    - the first sync indexes each kind's newest version only (goals v1 + v2 + notes v1 → goals v2 and notes points), keyed by the slug, with the `"{slug} {title} ({kind})"` heading;
    - a second sync writes and deletes nothing;
    - a new version replaces the old version's points;
    - an item sync leaves owner-document points alone (not item orphans);
    - a sync over `workspace_search_scope` of the Platform scope indexes the scope row's `WS` items and its goals document;
    - `workspace_search_scope` puts the row first, leaves a nil workspace and a workspace without a row unchanged, and, if the test seams allow it cheaply, leaves the scope unchanged on a failed lookup.
  - `qdrant_live.rs`: `a_workspace_goals_document_is_found_by_a_workspace_search`. Sync the Platform workspace's widened scope, then a search scoped to the widened projects returns a hit with `owner: Owner::Document(_)`, `key == "@platform"` (verify the demo slug) and `document` kind `goals`. A search scoped to the members alone does not return it. **Owed on the host** (the sandbox's Qdrant times out).
  - MCP: `OwnerKind::Project` serialises `"project"`; `mcp_search.rs` `hits_map_every_field_by_hand` gains an owner-document hit; `tools_search.rs` gains one case if its fixtures make it cheap.
  - Overlay (D10): a unit test where `Enter` on an owner-document hit emits no `Close`/`Reveal` and shows `OWNER_DOCUMENT`; the existing reveal test is kept.
- **Mirror**: Patterns table rows 1–3.
- **Validate**: `cargo nextest run -p htui-store --all-features --lib -E 'test(/vector/)'`; `cargo nextest run -p htui-mcp --all-features`; `cargo test -p htui --all-features --lib -- mcp_search:: concepts_search::`; `cargo clippy -p htui-store -p htui-mcp -p htui --all-features --all-targets -- -D warnings`; `cargo fmt --check`. The live test compiles (`cargo test -p htui-store --all-features --test it --no-run`) and is not run here.

### T2: Workspace-scoped search in the CLI, worker job and TUI (tests first)

- **Action**: D7–D9 and D12.
  - `concepts.rs`: extend `search_projects_default_to_every_workspace_and_project_narrows` (demo Postgres):
    - the default puts each workspace's scope row first, then its members;
    - `--project @<platform-slug>` answers that scope row alone;
    - `--project @nope` is `unknown_project`.
  - Add a `scopes` case: a workspace whose only project is its scope row is kept.
  - `concepts_worker.rs`:
    - a search with `scope_row_of: Some(PLATFORM)` over a `MemIndex` seeded with the widened scope finds a scope-row hit (a goals document written in the test);
    - the same search with `None` does not;
    - an `IndexConcepts` with `scope_row_of` reports the scope row's points;
    - the reply's query is the request's own.
  - Overlay unit tests:
    - "all" mode sends `scope_row_of: Some(ws)` for both search and `Ctrl+R`;
    - a cycled project sends `None`;
    - a nil workspace with no projects still says `NO_PROJECTS`.
  - `tests/it/concepts_search.rs`: end to end over the demo shell. Write a Platform goals document, `Ctrl+R`, then search its text: the hit lists as `@platform`, and `Enter` shows the D10 notice.
  - Docs: D12.
- **Mirror**: Patterns table rows 4–5.
- **Validate**: `cargo nextest run -p htui --all-features -E '(binary(it) and test(/^concepts_search::/)) or (kind(lib) and test(/concepts|store_worker::|concepts_search/))'`; `cargo test -p htui --all-features --lib -- concepts:: concepts_worker::`; `cargo clippy -p htui --all-features --all-targets -- -D warnings`; `cargo fmt --check`.

## Validation

```bash
bash scripts/gate.sh                     # main thread, once, after T2
# owed on the host (sandbox Qdrant times out):
cargo nextest run -p htui-store --all-features -E 'binary(it) and test(/^qdrant_live::/)'
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A sandbox gate cannot run the live Qdrant test that is the PRD's success metric | High | MemVectorStore unit and end-to-end tests cover the logic; the live test is owed on the host before close-out |
| The stamp bump drops every box's index on upgrade | Certain, intended | `check_existing` rebuilds; the next sync (worker job, `--index-items`, `Ctrl+R`) refills it from Postgres |
| An owner-document point is taken for an item orphan and deleted every sync | Medium | D5 splits `Owner::Document` before the orphan sweep; a T1 test pins it |
| Request shape change misses a pattern site | Low | the compiler finds every `SearchConcepts(..)`/`IndexConcepts { scope }`; D8 lists them |
| A slug rename leaves stale keys on owner-document points | Low | recorded (D13); a new version re-keys |

## Acceptance

- [ ] A workspace's goals document is returned by a workspace-scoped search (MemVectorStore in the sandbox; the live test on the host)
- [ ] Scope-row `WS` items are indexed and searchable from the TUI overlay and the CLI
- [ ] A project's own documents are indexed in that project and map in MCP
- [ ] `PAYLOAD_SCHEMA` is 3; existing stamp tests pass unchanged
- [ ] Gate green in the sandbox apart from the live-Qdrant suites (owed on the host)

## Verified claims (step 3.5 fact-check, 2026-11-14; inline against the tree)

| Claim | Verdict | Evidence |
|---|---|---|
| `Owner` is `Item \| Requirement`; `Subject` is `Item \| Requirement` | confirmed | `vector.rs:204-209`, `:127-153` |
| `PAYLOAD_SCHEMA` is 2 and every stamp test is relative to it | confirmed | `vector.rs:53`; text search: `vector.rs:1893,1919`, `qdrant_live.rs:451,790,810,835,840,867,880,889,910,925,1059,1073` all use the constant |
| `owner_of` maps every `Document` point to `Owner::Item(item_id)`; `indexed_point` requires `status` on `Document` | confirmed | `vector.rs:637-644`, `:650-670` |
| `Indexer::sync` iterates `scope.project_ids` and deletes every `Owner::Item` point not listed as an orphan | confirmed | `vector_sync.rs:67-131` |
| `ReadStore::owner_documents(project)` returns full rows (body included), every version | confirmed | `traits.rs:380`; `OwnerDocument` `document.rs:39-58`; Mem `owner_document_rows` `mem.rs:8189` |
| `ReadStore::scope_project` and `ReadStore::project` exist on every store | confirmed | `traits.rs:370`, `:180`; impls Mem/Pg/Cache/Backend/Writer |
| `Scope::with_scope_row` puts the row first | confirmed | `scope.rs:47-53` |
| `scopes()` feeds `search_projects`, `index_items` and `PgStore::index_scopes` | confirmed | callers graph: `concepts.rs:213`, `:269`, `:686` |
| `WorkspaceSummary` carries `slug`; the scope row's slug is `@<ws-slug>`; the demo's is `@platform` | confirmed | `hierarchy.rs:307`; `seed.rs:1056` `scope_row_slug("platform") == "@platform"` |
| Production `match` sites on `Owner`: `concepts_search.rs` `target`, `mcp_search.rs` `concept_hit`, `vector_sync.rs` `Indexer::sync`, `vector.rs` | confirmed | usages of `Owner` (27 refs, 8 files; others are tests/bench constructors) |
| `Subject` is matched only inside `htui-store` (plus test constructors in `mcp_search.rs`) | confirmed | usages of `Subject` |
| `SearchConcepts(..)` pattern/constructor sites as listed in D8 | confirmed | text search: 17 `SearchConcepts(` and 19 `IndexConcepts` matches, lines as cited |
| `htui-store` depends on `tracing` | confirmed | `crates/htui-store/Cargo.toml:43` |
| The demo seeds no owner documents (tests write their own) | confirmed | `GOALS_KIND` appears only in model, `owner_documents.rs`, the Settings pane and its tests |
| CLI JSON `version` does not change for a new value of an existing field | confirmed | `docs/htui-cli.md:419-425` |
| `docs/htui-mcp.md` lists `owner_kind` as `item` or `requirement` | confirmed | `docs/htui-mcp.md:137` |
| T1 and T2 are independent | **falsified** | both touch `concepts_search.rs`; plan marks them serial |
