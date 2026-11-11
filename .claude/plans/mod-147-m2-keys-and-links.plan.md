# Plan: MOD-147 M2 — keys and links across a workspace

**Source PRD**: `.claude/prds/mod-147-workspace-scope.prd.md`
**Selected Milestone**: 2 — Keys and links across a workspace
**Complexity**: Large (core model and MemStore, PgStore and cache, MCP `item_link`, htui store worker, CLI, Settings)
**Routing**: PRD path (C2 + C4); ultracode accepted for implement and review.

## Summary

A key may be written `slug:KEY` (`api:FEAT-9`, `@shop:WS-1`). `item_by_key` resolves one, so MCP
`item_link` can name an item in another project. `propose_link` accepts targets in the step's project,
in the workspaces holding it (their scope rows) and in those workspaces' member projects. A workspace
kind whose prefix a member project already uses is refused. A new project that shares a prefix with its
workspace still joins, and Settings warns about it. The waiting list (CLI and overlay) now covers the
workspace's own items, and the CLI qualifies a key when another project in the scope uses its prefix.
M1's carried note R1 L8 is closed: a scope row may use its own workspace's graphs, and its runs carry
its workspace.

## Decisions taken in this plan

| # | Decision | Why |
|---|---|---|
| D1 | **Key grammar**: `<slug>:<KEY>`, split at the **last** `:`; a bare key means the caller's own project. Helpers in a new `htui_core::model::key`: `split_qualified(&str) -> (Option<&str>, &str)`, `key_prefix(&str) -> Option<&str>` (text before the last `-`), and `KeyQualifier` (below) | A key is `PREFIX-N` and a prefix is `[A-Z][A-Z0-9]{1,15}` (`ItemKind::prefix_is_valid`, `model/kind.rs:169`), so a key never holds `:`; scope-row slugs are `@<slug>` (M1 D3) |
| D2 | **`item_by_key(project, key)` resolves a qualified key by slug** and ignores `project` then. A bare key is looked up in `project`, as today. The signature does not change | `item_by_key` has one production caller (`htui-mcp/src/tools/link.rs:82`) and 9 bodies (two trait declarations, wrappers and spies). Only the Mem (`mem.rs:8435`) and Pg (`pg/write.rs:9296`) bodies change, and the wrappers stay untouched. `UpstreamEntry.qualified_key` already prints `slug:KEY` (`mem.rs:1929`, `pg/read.rs:636`), the same grammar. ANA-35 §6: "lets `item_by_key(project, key)` resolve it unchanged" |
| D3 | **`propose_link` reach**: the target's project must be (a) the step's project, (b) a scope row of a workspace W that holds the step's project, or (c) a member of such a W. W is every workspace the project belongs to, plus its owner workspace when the step's project is itself a scope row. Pg uses one SQL predicate in the existing transaction. Mem uses `workspaces_of` widened by D7. The refusal sentence `link_outside_project` becomes `link_outside_reach` ("… is not in this project or its workspaces"); its callers are `mem.rs:8367`, `pg/write.rs:9191` and `conformance.rs:21156`. `withdraw_link` keeps no reach check: it can only tombstone this run's own proposals | ANA-35 §7.5 "`propose_link` accepts targets in the step's project, in the workspaces containing it, and in those workspaces' member projects"; today both stores refuse anything outside the step's project (`pg/write.rs:9168-9192`, `mem.rs:8351-8369`) |
| D4 | **MCP `item_link`**: `to` accepts `KEY`, `slug:KEY` or `@workspace:KEY`. The reply's `to` is canonical: bare when the target is in the session's project, `slug:KEY` otherwise. `from` stays bare. A miss reads `out of scope: <to> is not an item of this project or its workspaces`. The tool description and the `to` field doc name both forms. The canonical `to` costs one `item()` and one `project()` read, both already on `WorkerStore` | The session's own project is the agent's frame of reference, so a bare key there is unambiguous to it |
| D5 | **Prefix refusal**: `create_item_kind` and `update_item_kind` (prefix change) on a **scope row** refuse a prefix that any kind of a current member project already uses: `workspace_prefix_taken(prefix, member_slug)`, a `StoreError::Constraint`. A member project's own new kind is **not** refused against the workspace's prefixes (ANA-35 (i) applies only at workspace-kind creation; qualification covers the rest). A kind whose prefix is unchanged is not re-checked | ANA-35 §5.3 (i) and §7.5. Prevention cannot be complete (§6: a project can join after both minted), so a concurrent member kind is not locked against |
| D6 | **Join warning**: `StoreRequest::CreateProject` (the only production join path, `htui/src/hierarchy.rs:321-357`) compares the new project's kind prefixes with its workspace's scope-row kinds (`scope_project` + `item_kinds`) after the link. It answers a new reply, `StoreReply::HierarchyJoined { tree, shared_prefixes: Vec<String> }`. Settings' hierarchy section handles it like `Hierarchy(Some(_))` and, when `shared_prefixes` is non-empty, shows a one-line notice: "`<slug>` shares prefix WS with workspace `<ws>`: keys show as `<slug>:WS-1` and `@<ws>:WS-1`". No refusal | ANA-35 §7.5 "a project whose prefix collides may still join, with a warning". Composed from existing reads, so no new store method |
| D7 | **A scope row uses its own workspace's graphs** (R1 L8, part 1). Mem `workspaces_of` (`mem.rs:3828`) returns membership ∪ `owner_workspace_id`, which covers `check_item_kind` and `usable_graph_rows`. Pg's three membership subqueries (`pg/read.rs:3144-3147` `usable_graph_rows`, `pg/write.rs:3627-3629` `create_item_kind`, `:3722-3724` `update_item_kind`) gain `OR g.workspace_id = (SELECT owner_workspace_id FROM project WHERE id = $n)`. The workspace-delete held-graph refusal (`pg/write.rs:10125-10140` and Mem's twin) **skips holders in the workspace's own scope row**, which cascades with it | Without the last part, a WS kind holding a workspace graph would make its own workspace undeletable |
| D8 | **Scope-row runs carry their workspace** (R1 L8, part 2): `create_run` and `queue_item` (Pg and Mem), for an item whose project is a scope row, fill a `None` `workspace_id` with the owner workspace and refuse any other (`scope_row_run_workspace(…)`). The check runs **before** `queue_item`'s early return or `ON CONFLICT DO NOTHING` (Mem `mem.rs:1069`, Pg `pg/write.rs:10304`), so a re-queue is checked too | `claim_run` gates on `workspace_stop` by `run.workspace_id` (`pg/write.rs:6349-6362`), so a scope-row run must name its own workspace or a workspace stop misses it. Both production callers already pass `Some(scope.workspace_id)` (`runs.rs:905`, `store_worker.rs:2687`); the rule pins it in the store |
| D9 | **Upstream "in scope" includes the scope row** when a workspace scope is given: the `scope` CTE of `upstream_summaries` (Pg `pg/read.rs:629-633`, cache `cache/read.rs:899-903` with one more positional bind, Mem `upstream_entry` `mem.rs:~1916`) adds the workspace's scope row. **Only the TUI preview passes a workspace** (`preview.rs:248`). Real run prompts are built `PromptScope::project_only` (`htui-orch engine.rs:7250`), so they count only the item's own project and keep doing so. Widening run prompts to `run.workspace_id` is recorded for MOD-136 | Keeps the preview's in-scope set consistent with the workspace it shows, without changing what agents see in M2 |
| D10 | **The waiting list covers the scope row** (maintainer, 2026-11-10). The two callers build a widened `Scope` **for their reads only**: the store worker's `serve_waiting` (`store_worker.rs:2281`) and the CLI's `answer_online` (`state_cmd.rs:189`; the CLI has no offline path). The widened `Scope` is today's with `scope_project(scope.workspace_id)` inserted at `project_ids[0]` (`sort_canonical` orders by position in `project_ids`). It feeds `waiting_candidates`, `active_runs` and `open_permissions`, and those reads are unchanged. The reply still carries the **asked** scope, because `observe_reply` keeps a reply only `if *scope == self.scope` (`app/update.rs:515`). `WaitingRow` (`htui-worker/src/views.rs:523`) gains `project: ProjectId` and `display_key: String`. The overlay's Enter on a scope-row row must not fail silently while the Backlog does not yet list workspace items (M3); it shows the hierarchy notice-style sentence "workspace items open in the Backlog from MOD-147 M3" | A WS item parked at its hard review gate would otherwise never wait on anyone. The candidates already carry `Item.project_id` |
| D11 | **Qualification rule** (maintainer, 2026-11-10): within one answer's scope (the scope row included), a key prints `slug:KEY` when another scope project has a kind with the same prefix, otherwise bare. `KeyQualifier::new(&[(ProjectId, slug, prefixes)])` with `display(project, key)` is built from `item_kinds` of each scope project. (R1: `new` takes `impl IntoIterator<Item = (ProjectId, String, Vec<String>)>` by value, so the caller's rows move in uncloned.) Offline, `Backend` serves `item_kinds` from the mirror: a new `CacheStore::item_kinds` and `Backend` arm, since `item_kind` is mirrored but unread (`cache/mod.rs:52`). `waiting()` (`views.rs:600`) takes the qualifier as a new parameter and fills `WaitingRow.display_key` (overlay and CLI text). `htui waiting --json` keeps `item_key` and adds `project` (slug) and `qualified_key` (always `slug:KEY`) to each row. The scope row's slug comes from `scope_project` + `project()`. `docs/htui-cli.md:408-411` keeps `version` 1 for additive fields; the doc lists them. `human_waiting`'s `{:<10}` key column widens to the longest key | Members seeded alike share `ANA`/`FEAT`/`FIX`/`CLEAN`/`TOOL` (`seed.rs:58-146`), so bare keys are already ambiguous across members; a prefix rule is stable for a row whatever else the answer lists |
| D13 | **Out of scope, recorded**: the TUI's requirement `deciding_item` key resolver (`htui/src/requirements.rs:794`, upper-cases input, one project) stays bare-key; TUI key input is M3/MOD-149's. Run prompts' workspace scope: MOD-136 (D9) | Keeps M2 to MCP and CLI as the PRD row says |
| D12 | **The PRD metric "resolve in MCP and CLI" is read as**: MCP `item_link` resolves qualified keys, and the CLI *prints* them. No CLI verb takes a key (`cli.rs:166-192`: worker, provision, status, waiting, claude, mcp). Search hits (`--search-items`, MCP `search_concepts`) stay unqualified until M4 widens search to the scope row | Avoids inventing a CLI verb; M4 owns search |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Store refusal sentence | `crates/htui-core/src/store/traits.rs:2706` `invalid_prefix`, `:3329` `project_slug_is_reserved` | `pub fn …(…) -> String`, one person-readable sentence, asserted by conformance cases by equality |
| Pg guarded write in one tx | `crates/htui-store/src/pg/write.rs:9168` `propose_link` | `pool.begin()`, `step_scope`, lookup, refusal, upsert, commit |
| Kind guard split on the failure path | `pg/write.rs:3612` `create_item_kind`, `:10248` `kind_guard_refusal` | `INSERT … SELECT … WHERE EXISTS(…)`, refusal told apart only when no row returned |
| Mem twin | `crates/htui-core/src/store/mem.rs:3556` `check_item_kind`, `:8351` `propose_link` | same refusal order and sentence as Pg |
| Conformance case | `crates/htui-core/src/store/conformance.rs` `CASES` + `run_case_inline`; `item_by_key_answers_within_its_project` `:21392` | one name in `CASES`, one boxed arm; pins in `crates/htui-core/tests/mem_store.rs` and `crates/htui-store/tests/pg_conformance.rs` (`CASES` 248 on main) |
| MCP tool test | `crates/htui-mcp/tests/` (item_link cases) | MemStore-backed session, JSON in/out |
| Store-worker reply + Settings handling | `crates/htui/src/hierarchy.rs:477` `reread`, `ui/tabs/settings/hierarchy.rs:1553` `on_reply` | reply variant → section `on_tree` |
| CLI doc | `crates/htui/src/state_cmd.rs:810` `WaitingRowDoc`, `:1153` `human_waiting`; `docs/htui-cli.md` | versioned JSON document, additive fields |

## Files to Change (per task; independence is decided by these sets)

| Task | Files |
|---|---|
| T1 core (+ offline kinds read) | CREATE `crates/htui-core/src/model/key.rs`; UPDATE `crates/htui-core/src/model/mod.rs`, `crates/htui-core/src/store/{traits.rs,mod.rs,mem.rs,conformance.rs}`, `crates/htui-core/tests/mem_store.rs`; `crates/htui-store/src/{cache/read.rs,backend.rs}` (`CacheStore::item_kinds`, `Backend::item_kinds`), `crates/htui-store/tests/cache.rs` |
| T2 htui-store | UPDATE `crates/htui-store/src/pg/{write.rs,read.rs}`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/.sqlx/*`, `crates/htui-store/tests/{pg_conformance.rs,cache.rs}` |
| T3 MCP | UPDATE `crates/htui-mcp/src/tools/link.rs`, `crates/htui-mcp/tests/tools_backlog.rs` (item_link cases `:578-665`; `:598` asserts the old sentence), `crates/htui-mcp/src/tools/snapshots/htui_mcp__tools__box_profile__tests__tools_list_per_scope_shape.snap`, `docs/htui-mcp.md` (`:80`) |
| T4 htui | UPDATE `crates/htui/src/{hierarchy.rs,store_worker.rs,state_cmd.rs}`, `crates/htui/src/ui/tabs/settings/{hierarchy.rs,secrets.rs}`, `crates/htui/src/ui/overlay/{waiting_list.rs,leave.rs}`, `crates/htui/src/ui/header.rs`, `crates/htui-worker/src/views.rs`, `crates/htui/tests/{hierarchy.rs,state_cli_pg.rs}`, `docs/htui-cli.md`, moved `.snap` files |

T1 changes the trait sentences and helpers every other task uses, and adds the offline `item_kinds` read
T4 needs, so it goes first. After T1, T2/T3/T4 file sets are disjoint (T1 and T2 both touch
`cache/read.rs` and `tests/cache.rs`, but in sequence). **Order: T1 → (T2 ∥ T3 ∥ T4).** T3 and T4 test
against MemStore, so they do not wait on T2.

## Tasks

### T1: core helpers, sentences, MemStore, conformance (tests first)
- **Action**: unit tests for `model::key` first: split at the last `:`; `@shop:WS-1`; a bare key; `key_prefix`; `KeyQualifier` (a shared prefix is qualified, a unique one bare, the scope row qualified as `@shop:`). Then conformance cases (names in `CASES`, boxed arms):
  - `item_by_key_resolves_a_qualified_key`: `api:FEAT-9` and `@shop:WS-1`, including when a project is also named `shop`; an unknown slug gives `None`.
  - `propose_link_reaches_the_workspace_and_its_members`: scope-row target, member target, outside target refused with `link_outside_reach`, and from a scope-row step to a member.
  - `workspace_kind_refuses_a_member_prefix`: create and update; a member's own colliding kind is allowed.
  - `scope_row_uses_its_workspace_graphs`: kind create and `usable_graphs`.
  - `workspace_delete_ignores_its_scope_rows_graph_holders`.
  - `scope_row_run_carries_its_workspace`: `None` is filled, another workspace is refused, for both `create_run` and `queue_item`.
  - `upstream_counts_the_scope_row_in_scope`, in both directions: a member item's preview sees the WS item, and a scope-row item sees its own project and the members.

  Then the Mem implementation and the sentences `link_outside_reach`, `workspace_prefix_taken` and `scope_row_run_workspace`. The old `link_outside_project` callers move to the new sentence. Re-derive the `mem_store.rs` pin. Last, the offline kinds read: `CacheStore::item_kinds` (mirror table, the Pg order), the `Backend::item_kinds` arm, and a cache test that it matches Pg.
- **Validate**: `cargo test -p htui-core --all-features -- --test-threads=1`; `cargo test -p htui-store --all-features --test cache -- --test-threads=1`.

### T2: Postgres and cache
- **Action**: the Pg twins of every T1 rule:
  - `item_by_key` with a slug join for a qualified key.
  - `propose_link` reach predicate.
  - `create_item_kind`/`update_item_kind`: the scope-row prefix refusal and the owner-workspace graph clause.
  - `usable_graph_rows` owner clause.
  - `delete_workspace_once` held query: `AND pr.owner_workspace_id IS DISTINCT FROM $1` in both arms.
  - `create_run`/`queue_item` workspace fill/refusal, checked before `queue_item`'s `ON CONFLICT DO NOTHING`.
  - Keep `delete_workspace_once`'s shape: the source-scan test `delete_workspace_once_counts` (`pg/read.rs:3910`) slices its body at the first `"\n    }\n"`.
  - `upstream_summaries` scope CTE `UNION SELECT id FROM project WHERE owner_workspace_id = $3`, in Pg and in the cache (one more positional `.bind`).
  - `cargo sqlx prepare` against a migrated scratch DB, the `pg_conformance.rs` pin, and a cache test that the upstream scope includes the scope row offline.
- **Validate**: `cargo test -p htui-store --all-features -- --test-threads=1` (qdrant_live owed on host); `cargo sqlx prepare --check`.

### T3: MCP `item_link`
- **Action**: tests first, in the item_link test file:
  - `to: "api:FEAT-9"` from a member session links and replies `to: "api:FEAT-9"`.
  - `to: "@shop:WS-1"` links.
  - A bare own-project key replies bare.
  - An item outside the reach reads `out of scope: … this project or its workspaces`.
  - An unknown slug reads the same.

  Then `link.rs`: resolve through `item_by_key`, then canonical `to` (compare the target's `project_id` from `item()` with the session's, and take the slug from `project()`); description and `to` doc text; the tools-list snapshot (3 scopes) and `docs/htui-mcp.md`.
- **Validate**: `cargo test -p htui-mcp --all-features -- --test-threads=1`.

### T4: htui (join warning, waiting scope, CLI qualification)
- **Action**: tests first:
  - Hierarchy test: creating a project in a workspace whose scope row has a `FEAT` kind answers `HierarchyJoined` with `["FEAT"]`, and Settings shows the notice. A non-colliding create answers an empty list.
  - Waiting tests: a WS item at a hard review gate is listed first, in both the overlay view and `htui waiting`.
  - Two members' `FEAT-1` print `api:FEAT-1` and `web:FEAT-1`, and a lone prefix prints bare.
  - `--json` rows carry `project` and `qualified_key`.

  Then:
  - `HierarchyJoined` in `store_worker.rs` and `hierarchy.rs`. It is handled in `settings/hierarchy.rs`, which sets the existing `notice` field (as `RepoPathsInferred` does, `~:1570`), and in `settings/secrets.rs:1619`, which adopts the tree passively. There is no exhaustive `StoreReply` match (`observe_reply` ends in `_ => {}`).
  - Widen the `Scope` for the reads in `serve_waiting` and `answer_online`. The reply keeps the asked scope (test: the top bar still adopts the reply).
  - `WaitingRow.project`/`display_key` in `htui-worker/src/views.rs`. `waiting()` takes a `&KeyQualifier` and fills them. The 13 literals follow: `views.rs`, `waiting_list.rs`, `store_worker.rs`, `state_cmd.rs`, `header.rs`, `leave.rs`. Also update the `state_cli_pg.rs:241` caller.
  - The overlay's key column draws `display_key`. Enter on a scope-row row shows D10's sentence, not a silent miss.
  - `WaitingRowDoc` and `human_waiting`.
  - `docs/htui-cli.md`.
  - Full insta run, with every moved snapshot reviewed.
- **Validate**: `cargo test -p htui --all-features -- --test-threads=1`; `cargo test -p htui-worker --all-features`; `cargo insta test --workspace --all-features`.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features -- -D warnings
cargo clippy --workspace -- -D warnings          # featureless gate
bash scripts/gate.sh                              # full gate; live-Qdrant suites owed on host
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Snapshot churn: demo projects in one workspace share seeded prefixes, so waiting rows turn qualified | Medium | full insta run in T4, every moved snapshot reviewed against D11 |
| `.sqlx` churn from five edited queries | High | one `cargo sqlx prepare` at the end of T2 |
| `CASES` pin collides at collect with MOD-141/MOD-97 (other branches add cases) | Medium | re-derive the pins at `scripts/hr collect` |
| Overloading `item_by_key` hides a caller that passes user text with `:` | Low | one production caller; a conformance case pins both forms |
| `KeyQualifier` needs `item_kinds` offline | — | resolved: the cache has no kinds read today, so T1 adds `CacheStore::item_kinds` / `Backend::item_kinds` |
| A widened waiting scope echoed in the reply drops every top-bar update | Medium | D10: widen for reads only; test pins adoption |

## Acceptance
- [x] All tasks complete, tests written first
- [x] `@shop:WS-1` and `api:FEAT-9` resolve in MCP `item_link`, also when a project is named `shop`
- [x] A workspace kind with a member's prefix is refused; a colliding new project joins with a notice
- [x] `propose_link` reaches the workspace's scope row and members, nothing else
- [x] R1 L8 closed: scope row uses its workspace's graphs, its runs carry its workspace
- [x] Gate green except live-Qdrant suites (owed on host): merged tree 7849 passed; 27 live-Qdrant timeouts, main's `0027` comment test and the refresher flake (green alone) are the only failures
- [x] Patterns mirrored, not reinvented

**Status (2026-11-11): complete on hr/MOD-147.** T1-T4 merged (`488d20f6`), review R1 `93b38e35..5994ebaa`; rust-reviewer findings F1 (MCP remove existence) and F3 (Pg refusal sentence under a race) kept by decision, F2 filed as MOD-170.

## Verified claims (step 3.5 fact-check, 2026-11-10; three read-only checkers)

| Claim | Verdict | Evidence |
|---|---|---|
| A key never contains `:` (prefix `[A-Z][A-Z0-9]{1,15}`, key `PREFIX-N`) | TRUE | `model/kind.rs:169-176`; Mem mint `mem.rs:2214`; Pg generated `key`, `0001_init.sql:316` |
| `item_by_key` has one production caller | TRUE | `htui-mcp/src/tools/link.rs:82`; 9 bodies + 2 trait decls (plan said "9 implementations", amended) |
| Mem line refs (`item_by_key` 8435, `propose_link` 8351, `check_item_kind` 3556, `workspaces_of` 3828 membership-only, `usable_graph_rows` 3838) | TRUE | exact; `workspaces_of` has three callers |
| Workspace-delete held-graph check would trip on the scope row's own holders | TRUE | Mem `workspace_graph_holder` `mem.rs:5348` (no project filter), Pg `pg/write.rs:10125-10142`; D7 fix kept |
| Mem/Pg `create_run`, `queue_item` take `workspace_id` unchecked | TRUE (amended) | Mem `mem.rs:6217`, `mem.rs:1069` (inherent, early return on re-queue); Pg `pg/write.rs:6105-6211`, `:10304` (`ON CONFLICT DO NOTHING`); D8 check moved before both |
| `link_outside_project` callers | TRUE | `mem.rs:8367`, `pg/write.rs:9191`, `conformance.rs:21156` |
| `CASES` = 248, pins 248 | TRUE | `conformance.rs:87-348`; `mem_store.rs:36-37`; `pg_conformance.rs:53` (HANDOFF lines 70/461 still say 247: fix at close-out) |
| `scope_project` is on WriteStore | FALSE → amended | it is on ReadStore (`traits.rs:358`) and served by Pg, Mem, Writer, Cache, Backend; `WorkerStore` has `item`, `project`, `item_kinds` |
| Three Pg membership subqueries gate graph usability, no others | TRUE | `pg/read.rs:3145-3147`, `pg/write.rs:3627-3629`, `:3722-3725`; nothing else gates on `workspace_project` |
| Pg `propose_link` refuses outside the step's project in one tx | TRUE | `pg/write.rs:9168-9223` |
| `claim_run` gates on `workspace_stop` by `run.workspace_id` | TRUE | `pg/write.rs:6349-6362` |
| Run prompts read `run.workspace_id` | FALSE → amended | `phase_spec` uses `PromptScope::project_only` (`htui-orch engine.rs:7250`); only the TUI preview passes a workspace (`preview.rs:248`). D8 justified by the workspace-stop gate only; D9 narrowed to the preview |
| `upstream_summaries` scope CTE in Pg and cache | TRUE | `pg/read.rs:629-633`; `cache/read.rs:899-903` (positional binds) |
| Cache serves `item_kinds` | FALSE → amended | no `CacheStore`/`Backend` `item_kinds`; T1 adds them (table mirrored, `cache/mod.rs:52`) |
| Cache serves `scope_project` | TRUE | `cache/read.rs:1296`, `backend.rs:1112-1118` |
| `.sqlx` offline | TRUE | 454 files; `.cargo/config.toml` `SQLX_OFFLINE = "true"` |
| Only production join is `CreateProject` | TRUE | `htui/src/hierarchy.rs:350`; new projects seeded with `seed::KINDS` (`mem.rs:3249`) |
| A new `StoreReply` variant needs exhaustive-match updates | PARTLY | none exhaustive; `settings/secrets.rs:1619` adopts trees passively (added to T4); hierarchy `notice` field exists |
| `htui waiting` has an offline twin | FALSE → amended | CLI is online-only (`state_cmd.rs:165`); the store worker's `serve_waiting` has the offline path |
| Waiting order follows `scope.project_ids` position | TRUE | `sort_canonical`, missing projects sort last; scope row goes to index 0 |
| Reply must echo the asked scope | TRUE (new) | `app/update.rs:515` `if *scope == self.scope` |
| `WaitingRow` has no project field; literal sites | TRUE | 13 literals (2 production in `views.rs`); `waiting()` at `views.rs:600` with callers `store_worker.rs:2299`, `state_cmd.rs:203`, `tests/state_cli_pg.rs:241` |
| Overlay draws `item_key` | TRUE | `waiting_list.rs:201, 231`; Enter reveals by key `:134` |
| CLI JSON additive fields keep `version` 1 | TRUE | `docs/htui-cli.md:408-411` |
| No CLI verb takes an item key | TRUE | `cli.rs:166-192`; `StateArgs` = `--json`, `--project`, `--cwd`; `--search-items` is free text |
| Only `item_link` takes a key in MCP; its tests | TRUE | `htui-mcp/tests/tools_backlog.rs:578-665`; tools-list snapshot and `docs/htui-mcp.md:80` carry the text (added to T3) |
| Task file sets disjoint after T1 | TRUE (after amendment) | T1 owns the cache kinds read; T2/T3/T4 sets have an empty intersection |
