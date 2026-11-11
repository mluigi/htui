# Blueprint: MOD-147 M2 — keys and links across a workspace

**Plan**: `.claude/plans/mod-147-m2-keys-and-links.plan.md` (D1-D13 confirmed, fact-checked). **PRD**:
`.claude/prds/mod-147-workspace-scope.prd.md`. **Design**: `docs/ANA-35.md` §6-§7.5. Every line ref below was read
on branch `hr/MOD-147` (HEAD `876a4985`) through the Gortex graph. Nothing was built or run.

## Deviations from the plan (facts the plan did not have)

| # | Fact | Evidence | What changes |
|---|---|---|---|
| V1 | The existing case `propose_link_upserts_revives_and_keeps_a_live_proposer` asserts that an `agy` step linking to `htui:FEAT-1` is refused. `htui` and `agy` are both members of Platform, so under D3 that link is **legal**: renaming the sentence is not enough | `conformance.rs:21148-21158` (`propose(ids::HTUI_FEAT_1, …)` → `link_outside_project`), `:21179-21182` (`!live_link(.., HTUI_FEAT_1, ..)`); `fixtures.rs:728-745` (Platform = HTUI, AGY; Graphics = VULKAN) | T1 retargets both lines at `ids::VULKAN_FEAT_1` (Graphics) with `link_outside_reach(ids::VULKAN_FEAT_1)`; the doc comment above the case (`:21009-21012`) reads "an unknown or out-of-reach `to`" |
| V2 | `queue_item` is **inherent** on `MemStore` and `PgStore`, not on `WriteStore`; conformance cases are `S: WriteStore`, so the queue half of `scope_row_run_carries_its_workspace` cannot live in the conformance suite | `mem.rs:1057-1069` ("Neither table is mirrored, so these are inherent"), `pg/write.rs:10291-10304`; `backend.rs:629-630` dispatches them | The conformance case covers `create_run` only. Queue: a Mem unit test in `mem.rs` (T1, beside `queue_item_records_its_workspace` `:~16540`) and a both-stores test in `crates/htui-store/tests/pg_criteria.rs` (T2, beside `queue_item_records_its_workspace` `:11956`). **T2 gains `tests/pg_criteria.rs`** |
| V3 | Renaming `link_outside_project` in T1 breaks the `htui-store` build: `pg/write.rs` imports and calls it | `pg/write.rs:60` (import), `:9191` (call) | T1 does the mechanical rename in those two lines (Postgres behaviour unchanged, still project-only, until T2); otherwise T1's own validate (`cargo test -p htui-store --test cache`) cannot compile. T1 and T2 are sequential, so the shared file is safe |
| V4 | `HierarchyJoined { tree, shared_prefixes }` cannot drive D6's notice: it names the new project's slug, and the section cannot tell from the tree which project is new (the editor's state is reset by `on_tree`) | `settings/hierarchy.rs:1553, 1567-1578`; `hierarchy.rs:321-357` | `StoreReply::HierarchyJoined { tree: Box<HierarchySnapshot>, project: String, shared_prefixes: Vec<String> }` (`project` = the new slug). The workspace slug is `tree.workspace.slug` |
| V5 | D10 says both callers widen. The CLI's `--project`/`--cwd` build a **one-project** scope; widening it would list `@ws` rows under `htui waiting --project api` and break the parity test, which asks `--project htui` and recomputes the view over `[htui]` | `state_cmd.rs:1007-1021` (`project_target`), `:983-1005` (`first_workspace`); `tests/state_cli_pg.rs:200-270` | The CLI widens only a **whole-workspace** target (`first_workspace`); `Target` gains `whole_workspace: bool`. The TUI always widens: its scope is always a whole workspace (`app/update.rs:357-374`, `set_scope` is "the only path that changes the scope") |
| V6 | `KeyQualifier`'s two callers both need "the asked scope with the scope row first" | D10; `model/scope.rs` (no such helper) | T1 adds `Scope::with_scope_row(&self, row: ProjectId) -> Scope` to `crates/htui-core/src/model/scope.rs` (**T1 gains `model/scope.rs`**) so T4 does not reach into core |
| V7 | `item_kinds` is a `WriteStore` method, not `ReadStore`, and `Backend` implements no `WriteStore` | `traits.rs:962`; `backend.rs:896` (`impl ReadStore for Backend` only) | `Backend::item_kinds` is **inherent** (beside `Backend::item_kind`, `backend.rs:414`): Memory → `WriteStore::item_kinds`, Online → `pg.item_kind_rows` (`pub(crate)`, `pg/read.rs:3063`), Offline → `cache.item_kinds` |

Small blueprint choices, not deviations: the queue/run decision is one pure fn shared by both stores
(`run_workspace_for`, §T1.3); the Pg prefix refusal is told apart on the failure path by one shared helper
(`workspace_prefix_holder`, §T2.3); the Mem waiting test arranges its gate with `park_step` (`traits.rs:1953`), the
Pg CLI test with `seed_gate`-style raw SQL (`state_cli_pg.rs:162-173`).

---

## Refusal orders (both stores, decided here)

| Write | Order (first failing check answers) |
|---|---|
| `propose_link` | self link → `step_scope` (NotFound / Fenced) → own item (`step_writes_own_item`) → `to` NotFound → **reach** (`link_outside_reach`) → upsert. Unchanged except the last check |
| `create_item_kind` | invalid prefix → unknown project → **`workspace_prefix_taken`** (scope row only) → `graph_not_usable` → own `(project, prefix)` → own `(project, name)` |
| `update_item_kind` | NotFound → (invalid prefix only after the CAS: Stale first) → Stale → **`workspace_prefix_taken`** (scope row, prefix changed) → `graph_not_usable` (graph changed) → own clashes |
| `create_run` | item NotFound → `startable` → `item_not_in_project` → **`scope_row_run_workspace`** → the existing tail (Mem: id, project, box, workspace FK, user, repos, batch; Pg: phantom repo, batch, insert FKs) |
| `queue_item` | item NotFound → **`scope_row_run_workspace`** → (Mem) early return of a stored entry / (Pg) `ON CONFLICT DO NOTHING` → box / workspace / user |
| `delete_workspace` | NotFound → held graph **ignoring holders in its own scope row** → live scope-row run → cascade |

## Edge cases (decided)

1. **Unknown slug** (`nope:FEAT-1`): `item_by_key` → `None`; MCP answers `out of scope: nope:FEAT-1 is not an item of this project or its workspaces`.
2. **Qualified key naming the session's own project** (`htui:FEAT-1` from an `htui` session): resolves to the same item; MCP replies `to: "FEAT-1"` (canonical bare). `htui:ANA-2` from `ANA-2`'s own run is still `self_link`.
3. **`@slug` of a deleted workspace**: the scope row cascaded with it (M1 A1), so no project has the slug → `None` → `out of scope`.
4. **Empty halves**: `:FEAT-1` → slug `""` → `None`; `htui:` → key `""` → `None`. A NUL anywhere is `None` before the split (Pg `22021` guard kept).
5. **Scope-row step linking to a member**: allowed (D3 via the owner workspace, D7's widened `workspaces_of`); to another workspace's member or scope row: `link_outside_reach`.
6. **Re-queue**: the scope-row check runs before Mem's early return and Pg's `ON CONFLICT`, so `queue_item(ws_item, .., Some(other), ..)` is refused even when the item is already queued; `None` passes and the stored entry comes back unchanged.
7. **Unknown workspace on a scope-row run/queue**: `scope_row_run_workspace` (it runs before the FK). On a user project: the FK refusal, as today.
8. **D5**: a member's own new kind with a workspace prefix is allowed; a workspace with no members accepts any prefix; several members holding the prefix → the sentence names the first by `workspace_project.position`, then slug bytes.
9. **`withdraw_link`** keeps no reach check (D3). MCP `remove` of a resolved out-of-reach target answers `not yours` (this run proposed nothing there).
10. **Offline qualifier**: the refresher upserts `item_kind` by `updated_at` (`refresh.rs:809-818`), so a deleted kind lingers in the mirror until the next rebuild and may over-qualify one key offline. Accepted.
11. **`KeyQualifier`**: a project it does not know, or a key with no `-`, prints bare.
12. **Default joins share nothing**: a new project is seeded with `seed::KINDS` (ANA, FEAT, FIX, CLEAN, TOOL), the scope row with `WS` only, and D5 keeps a member prefix off a scope row while a member exists. So `shared_prefixes` is non-empty only after a custom workspace kind added while no member held it (the T4 test arranges exactly that).

---

## T1 — core helpers, sentences, MemStore, conformance, offline kinds read (tests first)

Files: CREATE `crates/htui-core/src/model/key.rs`; UPDATE `crates/htui-core/src/model/{mod.rs,scope.rs}` (V6),
`crates/htui-core/src/store/{traits.rs,mod.rs,mem.rs,conformance.rs}`, `crates/htui-core/tests/mem_store.rs`,
`crates/htui-store/src/{cache/read.rs,backend.rs}`, `crates/htui-store/src/pg/write.rs` (**two-line rename only**, V3),
`crates/htui-store/tests/cache.rs`.

### T1.1 `model/key.rs` (new) and exports

```rust
//! MOD-147 M2: qualified item keys. A key is `PREFIX-N` and a prefix is `[A-Z][A-Z0-9]{1,15}`
//! (`ItemKind::prefix_is_valid`, `kind.rs:169`), so a key never holds `:`; a qualified key is
//! `<slug>:<KEY>`, and a workspace's own items are `@<workspace>:<KEY>` (the scope row's slug).

use std::collections::BTreeSet;
use crate::model::ids::ProjectId;

/// Splits at the **last** `:`: `(Some("api"), "FEAT-9")`, `(Some("@shop"), "WS-1")`,
/// `(None, "FEAT-9")` for a bare key. Halves are not validated: `":FEAT-1"` is
/// `(Some(""), "FEAT-1")`, `"api:"` is `(Some("api"), "")`.
#[must_use]
pub fn split_qualified(text: &str) -> (Option<&str>, &str)

/// The text before the last `-`: `Some("FEAT")` for `FEAT-12`; `None` without a `-`.
#[must_use]
pub fn key_prefix(key: &str) -> Option<&str>

/// D11: how one answer prints keys. A key prints `slug:KEY` when **another** project of the set has
/// a kind with the key's prefix, else bare. Built from `item_kinds` of every scope project, the
/// scope row included (its slug is `@<workspace>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyQualifier {
    /// `(project, slug, prefixes)`, in the order given.
    projects: Vec<(ProjectId, String, BTreeSet<String>)>,
}

impl KeyQualifier {
    /// `(project, slug, its kinds' prefixes)` per scope project.
    #[must_use]
    pub fn new(projects: &[(ProjectId, String, Vec<String>)]) -> Self
    /// D11's display form: `slug:KEY` on a shared prefix, else `KEY`; bare for an unknown project
    /// or a key with no prefix.
    #[must_use]
    pub fn display(&self, project: ProjectId, key: &str) -> String
    /// Always `slug:KEY` (`htui waiting --json`'s `qualified_key`); bare for an unknown project.
    #[must_use]
    pub fn qualified(&self, project: ProjectId, key: &str) -> String
    /// The project's slug, if the set holds it (`--json`'s `project`).
    #[must_use]
    pub fn slug(&self, project: ProjectId) -> Option<&str>
}
```

`model/mod.rs`: `pub mod key;` after `pub mod item_spec;` (`:90`), and `pub use key::{KeyQualifier, key_prefix,
split_qualified};` after the `item_spec` re-export (`:131`).

`model/scope.rs` (V6), in `impl Scope` after `is_empty` (`:40`):

```rust
/// MOD-147 D10: this scope with `row` (the workspace's scope row) first, for the waiting reads
/// only; `row` already present stays where it is. `sort_canonical` orders by position here, so the
/// workspace's own rows list first.
#[must_use]
pub fn with_scope_row(&self, row: ProjectId) -> Self
```

Unit tests (`key.rs` `#[cfg(test)]`, written first):
- `split_qualified_splits_at_the_last_colon` — `api:FEAT-9`, `@shop:WS-1`, `FEAT-9`, `a:b:FEAT-1` → `(Some("a:b"), "FEAT-1")`, `:FEAT-1`, `api:`.
- `key_prefix_is_the_text_before_the_last_dash` — `FEAT-12` → `FEAT`, `WS-1` → `WS`, `NOPE` → `None`.
- `a_shared_prefix_is_qualified_and_a_unique_one_bare` — `api` {FEAT, TOOL}, `web` {FEAT}: `display(api, "FEAT-1")` = `api:FEAT-1`, `display(web, "FEAT-1")` = `web:FEAT-1`, `display(api, "TOOL-1")` = `TOOL-1`.
- `the_scope_row_qualifies_with_its_at_slug` — `@shop` {WS}, `api` {WS}: `@shop:WS-1`, `api:WS-2`; `@shop` alone with WS → `WS-1`.
- `unknown_projects_and_bare_text_print_bare`; `qualified_always_names_the_slug` (`qualified(api, "TOOL-1")` = `api:TOOL-1`).
- `scope.rs`: `with_scope_row_puts_the_row_first_once`.

### T1.2 Refusal sentences and the run-workspace rule (`store/traits.rs`)

Replace `link_outside_project` (`traits.rs:3519-3524`) and add beside it:

```rust
/// MOD-147 D3: an agent links its item only to items of its run's project, of each workspace
/// holding that project (the workspace's scope row) and of those workspaces' member projects.
#[must_use]
pub fn link_outside_reach(to: ItemId) -> String {
    format!("item {to} is not in this project or its workspaces")
}

/// MOD-147 D5: a scope-row kind may not take a prefix a current member project's kind uses.
#[must_use]
pub fn workspace_prefix_taken(prefix: &str, member: &str) -> String {
    format!(
        "item_kind.prefix `{prefix}` is used by member project `{member}`; a workspace kind may \
         not share a member's prefix"
    )
}

/// MOD-147 D8: a run (or queue entry) on a scope row names another workspace than its own.
#[must_use]
pub fn scope_row_run_workspace(scope_slug: &str, asked: WorkspaceId) -> String {
    format!(
        "project `{scope_slug}` is a workspace's scope row; its runs carry that workspace, not \
         workspace {asked}"
    )
}

/// MOD-147 D8: the `workspace_id` a run or queue entry stores. `owner` is the project's
/// `owner_workspace_id`: a scope row's runs carry it (`None` is filled), any other workspace is
/// `Err(asked)`; a user project (`owner` `None`) keeps `asked`.
#[must_use]
pub fn run_workspace_for(
    owner: Option<WorkspaceId>,
    asked: Option<WorkspaceId>,
) -> core::result::Result<Option<WorkspaceId>, WorkspaceId>
```

Unit tests (traits.rs test module): `run_workspace_for_fills_keeps_and_refuses` (4 rows: (None, None) → Ok(None);
(None, Some(a)) → Ok(Some(a)); (Some(w), None) → Ok(Some(w)); (Some(w), Some(w)) → Ok(Some(w)); (Some(w), Some(x)) →
Err(x)).

`store/mod.rs:22-50` re-export list gains `link_outside_reach`, `run_workspace_for`, `scope_row_run_workspace`,
`workspace_prefix_taken` (alphabetical). Doc edits in `traits.rs`: `propose_link` (`:2340-2359`, "`Constraint(
link_outside_reach(..))` when `to` is outside the reach of D3"); `item_by_key` (`:2365-2377`, "a qualified
`slug:KEY` is looked up by slug and `project` is ignored; an unknown slug is `None`"); `create_run` (`:1491`) and the
two kind writers (near `:940-962`) name the new refusals.

Import sweeps (rename `link_outside_project` → `link_outside_reach`): `mem.rs:80`, `conformance.rs:64`,
`pg/write.rs:60` + `:9191` (V3).

### T1.3 MemStore (`crates/htui-core/src/store/mem.rs`)

- **`workspaces_of`** (`:3828`): membership rows, **plus** `self.projects.get(&project).and_then(|row|
  row.owner_workspace_id)` (D7). Doc: "MOD-147 D7: and a scope row's own workspace". Covers `check_item_kind`
  (`:3573`) and `usable_graph_rows` (`:3839`); its third use is the new `link_reaches`.
- **`item_by_key`** (`:8435-8443`): after the NUL guard, `let (slug, key) = split_qualified(key);` a `Some(slug)`
  replaces `project` with `self.projects.values().find(|row| row.slug == slug)?.id`; then the existing find.
- **`propose_link`** (`:8351-8392`): the check at `:8366-8368` becomes
  `let target = self.require_item(link.to)?.project_id; if !self.link_reaches(project, target) { return
  Err(Constraint(link_outside_reach(link.to))) }`. New
  ```rust
  /// MOD-147 D3: whether a step of `from` may link to an item of `to`: the same project, or
  /// `to` is the scope row or a member of a workspace in `workspaces_of(from)`.
  fn link_reaches(&self, from: ProjectId, to: ProjectId) -> bool
  ```
  (`from == to || self.workspaces_of(from).iter().any(|ws| self.scope_row_of(*ws) == Some(to) ||
  self.workspace_projects.iter().any(|m| m.workspace_id == *ws && m.project_id == to))`; `scope_row_of` `:7479`).
- **`check_item_kind`** (`:3556-3598`): new last parameter `prefix_moved: bool`. After the project-exists check
  and **before** the graph check:
  ```rust
  if prefix_moved && let Some(member) = self.workspace_prefix_holder(project, prefix) {
      return Err(StoreError::Constraint(workspace_prefix_taken(prefix, &member)));
  }
  ```
  `create_item_kind` (`:3600`) passes `true`; `update_item_kind` (`:3631`, call `:3649`) passes
  `patch.prefix.as_deref().is_some_and(|p| p != current.prefix)`. New
  ```rust
  /// MOD-147 D5: the slug of the first member (by `position`, then slug bytes) of the workspace
  /// whose scope row is `project` that has a kind with `prefix`; `None` for a user project.
  fn workspace_prefix_holder(&self, project: ProjectId, prefix: &str) -> Option<String>
  ```
- **`upstream_entry`** (`:1917-1924`): the `Some(ws)` arm is "a member of `ws`, **or** an item of `ws`'s scope row"
  (`self.projects.get(&item.project_id).is_some_and(|row| row.owner_workspace_id == Some(ws))`). Comment cites D9.
- **`create_run`** (`:6217`): right after the `item_not_in_project` check (`:6236-6241`):
  ```rust
  let owner = self.projects.get(&new.project_id).and_then(|row| row.owner_workspace_id);
  let workspace_id = run_workspace_for(owner, new.workspace_id).map_err(|asked| {
      StoreError::Constraint(scope_row_run_workspace(&self.projects[&new.project_id].slug, asked))
  })?;
  ```
  The FK check (`:6268-6276`) and the row (`workspace_id: new.workspace_id` `:6302`) use `workspace_id`.
- **`MemStore::queue_item`** (`:1069`): in the closure, after `require_item` and **before** the
  `queue_entries.get(&item)` early return (`:1079-1081`), the same `run_workspace_for` with the item's project; the
  resolved value feeds the workspace FK check and `QueueEntry.workspace_id`.
- **`workspace_graph_holder`** (`:5348`) gains `except: Option<ProjectId>`; both chains skip rows whose
  `project_id == except`. **`delete_workspace`** (`:5670`): compute `let scope = self.scope_row_of(id);` before the
  holder check (move `:5689`), call `self.workspace_graph_holder(&graphs, scope)` (`:5681`). Its later uses of
  `scope` are unchanged.
- **Unit test** (mem.rs tests, beside `queue_item_records_its_workspace` `:~16540`, V2):
  `queue_item_fills_a_scope_rows_workspace_and_refuses_another` — mint `WS` on `PROJECT_SCOPE_PLATFORM`
  (`KIND_SCOPE_PLATFORM_WS`); `queue_item(ws, BOX, None, USER, at)` → `workspace_id == Some(WORKSPACE_PLATFORM)`;
  a second WS item with `Some(WORKSPACE_GRAPHICS)` → `Constraint(scope_row_run_workspace("@platform",
  GRAPHICS))`, nothing queued; re-queue of the first with `Some(GRAPHICS)` → the same refusal (before the early
  return); re-queue with `None` → the stored entry, unchanged.

### T1.4 Conformance (`crates/htui-core/src/store/conformance.rs`)

Append to `CASES` after `"scope_row_cannot_be_deleted_alone"` (`:347`) under `// MOD-147 M2: keys and links across a
workspace (plan D2-D9)`; one boxed arm each before `other =>` (`:1097`). Helpers in place: `new_item` (`:1210`),
`new_item_kind` (`:3226`), `new_workspace`, `new_project`, `new_run`, `agent_step` (`:20711`), `scoped_graph`,
`linked_workspace`, `constraint_exactly`, `live_link`. Add one: `mint_ws(case, store, scope, kind) -> Item` (mint a
`WS` item on a scope row).

Existing case edits: V1 (`:21148-21158`, `:21179-21182`).

1. **`item_by_key_resolves_a_qualified_key`** — bare unchanged (`item_by_key(HTUI, "FEAT-1")` = `HTUI_FEAT_1`);
   `item_by_key(AGY, "htui:FEAT-1")` = `HTUI_FEAT_1` (`project` ignored); `item_by_key(HTUI, "htui:FEAT-1")` same;
   `ws = mint_ws(PROJECT_SCOPE_PLATFORM)` → `item_by_key(HTUI, "@platform:WS-1")` = `ws.id`; a user project slug
   `platform` (`create_project(new_project("platform"))`, mint its `FEAT` via its `item_kinds`) →
   `"platform:FEAT-1"` = that item, `"@platform:FEAT-1"` = `None`, `"platform:WS-1"` = `None`; `"nope:FEAT-1"`,
   `":FEAT-1"`, `"htui:"`, `"htui:FEAT-1\0"` → `None`.
2. **`propose_link_reaches_the_workspace_and_its_members`** — `ws = mint_ws(PROJECT_SCOPE_PLATFORM)`, `gws =
   mint_ws(PROJECT_SCOPE_GRAPHICS)`; `(_, step) = agent_step(AGY, AGY_FIX_1, None)` (fence `Unleased`): to
   `HTUI_FEAT_1` relates → Ok (co-member); to `ws` blocked_by → Ok (scope row); to `VULKAN_FEAT_1` and to `gws` →
   `constraint_exactly(link_outside_reach(..))`, no live link; from the scope row: `(_, s2) = agent_step(
   PROJECT_SCOPE_PLATFORM, ws.id, None)` → to `HTUI_ANA_2` relates → Ok (member, via the owner workspace), to
   `VULKAN_TOOL_1` → `link_outside_reach`. `withdraw_link` of the `HTUI_FEAT_1` proposal → tombstoned (no reach check).
3. **`workspace_kind_refuses_a_member_prefix`** — `ws = create_workspace(new_workspace("prefixed"))`, `scope =
   scope_project(ws.id)`, `graph` = its `WS` graph (`step_graphs(scope)[0]`); members `pa`, `pb`
   (`new_project`, `upsert_workspace_project` positions 0, 1; both seeded with `FEAT`): create on `scope`
   `"FEAT"` → `Constraint(workspace_prefix_taken("FEAT", "pa"))` exactly, `item_kinds(scope)` unchanged; `"feat"` →
   `invalid_prefix` (first); `"FEAT"` with a Graphics workspace graph → still `workspace_prefix_taken` (before the
   graph); `"EPIC"` → Ok; `pa` creates its own `EPIC` → Ok (a member is not refused); `update_item_kind(epic, token,
   prefix "FIX")` → `workspace_prefix_taken("FIX", "pa")`, row unchanged; with a spent token → `Stale`; a
   name-only patch (prefix unchanged, `pa` shares `EPIC`) → `Applied`.
4. **`scope_row_uses_its_workspace_graphs`** — `g = create_step_graph(scoped_graph(Workspace(WORKSPACE_PLATFORM),
   "ws-flow", false))`, `gg` the same on Graphics: `usable_graphs(PROJECT_SCOPE_PLATFORM)` = the scope row's own
   graphs, then `g`, then the library's (no `gg`); `create_item_kind(new_item_kind(PROJECT_SCOPE_PLATFORM, "PLAN",
   "plan", g))` → Ok; `update_item_kind(KIND_SCOPE_PLATFORM_WS, token, default_graph_id g)` → Applied; with `gg` →
   `graph_not_usable(gg, PROJECT_SCOPE_PLATFORM)`.
5. **`workspace_delete_ignores_its_scope_rows_graph_holders`** — `ws` fresh; `g` a graph of `ws`; a scope-row kind
   `PLAN` on `g` and a scope-row item minted under it with `step_graph_id = Some(g)`; member `m` (linked) with a
   kind on `g`: `delete_workspace(ws)` → `Constraint(workspace_graph_is_held(.., "item_kind <m's prefix>",
   "m"))` (the member still holds it); `delete_item_kind(m's kind)`; `delete_workspace` → Ok, equal to
   `delete_reach(Workspace)` read just before; `scope_project(ws)` `None`.
6. **`scope_row_run_carries_its_workspace`** (create_run only, V2) — `ws1 = mint_ws(PROJECT_SCOPE_PLATFORM)`:
   `new_run(PROJECT_SCOPE_PLATFORM, ws1, ..)` with `workspace_id None` → returned and re-read `Some(PLATFORM)`;
   `ws2` with `Some(GRAPHICS)` → `Constraint(scope_row_run_workspace("@platform", GRAPHICS))` exactly, `run(id)`
   `None`; with `Some(WorkspaceId::new())` → the same sentence with that id (before the FK); with `Some(PLATFORM)` →
   Ok. A user project's `None` stays `None` (one assertion, `HTUI_ANA_2`).
7. **`upstream_counts_the_scope_row_in_scope`** — `ws1`, `ws2` WS items; `agent_step(AGY, AGY_FIX_1)` proposes
   `AGY_FIX_1 blocked_by ws1`: `upstream_summaries(AGY_FIX_1, 1, PromptScope { workspace: Some(PLATFORM), project:
   AGY })` holds `ws1` as `("@platform:WS-1", 1, in_scope true)`; under `project_only(AGY)` `in_scope false`.
   Reverse: `agent_step(PROJECT_SCOPE_PLATFORM, ws1)` proposes `ws1 blocked_by ws2` and `ws1 blocked_by
   HTUI_ANA_1`: under `{ Some(PLATFORM), PROJECT_SCOPE_PLATFORM }` both in scope, `HTUI_ANA_1`'s summary `Some`;
   under `project_only(PROJECT_SCOPE_PLATFORM)` `ws2` in scope, `HTUI_ANA_1` not.

**Pin**: `crates/htui-core/tests/mem_store.rs:37` 248 → **255**; message gains "and MOD-147 M2's seven for qualified
keys, link reach, workspace prefixes and graphs, scope-row runs and the upstream scope (plan D2-D9)". The Pg pin
(`pg_conformance.rs:53`) is T2's. HANDOFF's stale "247" (plan) is close-out's.

### T1.5 Offline kinds read (`crates/htui-store`)

`cache/read.rs`, inherent, after `projects` (`:1622-1650`):

```rust
/// A project's kinds off the mirror, ordered as `PgStore::item_kind_rows` orders them
/// (`position`, then `prefix` bytes: SQLite's BINARY is byte order). MOD-147 D11's offline
/// qualifier reads it; empty for a project the mirror does not hold.
///
/// # Errors
/// Whatever the driver reports, through [`map_sqlx`], or [`StoreError::Backend`] from a decoding
/// helper on a corrupt mirror.
pub async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>>
```

SQL: `SELECT id, project_id, prefix, name, description, default_graph_id, position, updated_at FROM item_kind
WHERE project_id = ? ORDER BY position, prefix`; decode with `uuid_col`, `text`, `get`, `ts_col` (as `project()`
`:946-972`).

`backend.rs`, inherent, after `item_kind` (`:414-420`) — offline answers from the mirror, unlike `item_kind`:

```rust
/// A project's kinds (MOD-147 D11): Postgres online, the mirror offline (`item_kind` is mirrored).
pub async fn item_kinds(&self, project: ProjectId) -> Result<Vec<ItemKind>> {
    match self {
        Self::Memory(store) => WriteStore::item_kinds(store, project).await,
        Self::Online { pg, .. } => pg.item_kind_rows(project).await,
        Self::Offline { cache, .. } => cache.item_kinds(project).await,
    }
}
```
(`use htui_core::store::WriteStore;` at `:38`.)

Test (`tests/cache.rs`): `the_mirror_lists_item_kinds_like_postgres` — Pg demo, `run_pass` over `all_projects()` plus
`PROJECT_SCOPE_PLATFORM` (the M1 scope-row test's pattern); `cache.item_kinds(HTUI)` == `pg.item_kinds(HTUI)`
(five, seed order) and the scope row's `[WS]`; an unknown project → empty; `Backend::Offline { cache, .. }
.item_kinds(HTUI)` == the same.

**Build order (T1)**: key.rs tests (red) → key.rs → scope.rs → sentences + `run_workspace_for` + tests → mod.rs
re-exports + import sweeps (V3) → conformance cases 1-7, V1 edit, pin (red on Mem) → MemStore → mem unit test →
cache/backend read + cache test.

---

## T2 — Postgres and the cache (htui-store)

Files: `crates/htui-store/src/pg/{write.rs,read.rs}`, `crates/htui-store/src/cache/read.rs`, `crates/htui-store/.sqlx/*`,
`crates/htui-store/tests/{pg_conformance.rs,cache.rs,pg_criteria.rs}` (V2).

### T2.1 `item_by_key` (`pg/write.rs:9296-9308`)

NUL guard first (whole text), then `match split_qualified(key)`: `(None, key)` → the existing statement (text
unchanged, `.sqlx` reused); `(Some(slug), key)` →

```sql
SELECT i.id AS "id: ItemId"
  FROM item i JOIN project p ON p.id = i.project_id
 WHERE p.slug = $1 AND i.key = $2
```

### T2.2 `propose_link` reach (`pg/write.rs:9168-9223`)

The lookup at `:9177-9192` becomes one statement on the same `tx` (`$1` = `link.to`, `$2` = `scope.project`):

```sql
SELECT i.project_id AS "project_id: ProjectId",
       (i.project_id = $2
        OR EXISTS (
             SELECT 1 FROM workspace w
              WHERE (w.id IN (SELECT wp.workspace_id FROM workspace_project wp WHERE wp.project_id = $2)
                     OR w.id = (SELECT p.owner_workspace_id FROM project p WHERE p.id = $2))
                AND (w.id = (SELECT t.owner_workspace_id FROM project t WHERE t.id = i.project_id)
                     OR EXISTS (SELECT 1 FROM workspace_project m
                                 WHERE m.workspace_id = w.id AND m.project_id = i.project_id))))
         AS "reachable!"
  FROM item i
 WHERE i.id = $1
```

No row → `NotFound { entity: "item" }` (as today); `!reachable` → `Constraint(link_outside_reach(link.to))`. A
snapshot read: membership racing the link is not locked (a link is not structural).

### T2.3 Kinds: the prefix refusal and the owner graph clause (`pg/write.rs:3612-3768`)

**`create_item_kind`** (`:3612`): the `WHERE EXISTS` (`:3622-3628`) gains the owner clause and a second conjunct:

```sql
INSERT INTO item_kind (id, project_id, prefix, name, description, default_graph_id, position)
SELECT $1, $2, $3, $4, $5, $6, $7
 WHERE EXISTS (SELECT 1 FROM step_graph g
                WHERE g.id = $6
                  AND (g.project_id = $2
                       OR (g.project_id IS NULL AND g.workspace_id IS NULL)
                       OR g.workspace_id IN (SELECT wp.workspace_id FROM workspace_project wp
                                              WHERE wp.project_id = $2)
                       OR g.workspace_id = (SELECT p.owner_workspace_id FROM project p WHERE p.id = $2)))
   AND NOT EXISTS (SELECT 1
                     FROM project sr
                     JOIN workspace_project wp ON wp.workspace_id = sr.owner_workspace_id
                     JOIN item_kind mk ON mk.project_id = wp.project_id
                    WHERE sr.id = $2 AND mk.prefix = $3)
RETURNING …   -- unchanged
```

`kind_guard_refusal` (`:10248`) → `kind_guard_refusal(&self, project, graph, prefix: &str)`: `project()` `None` →
the existing unknown-project sentence; `Some` → `workspace_prefix_holder(project, prefix)` → `Some(member)` →
`workspace_prefix_taken(prefix, &member)`, else `graph_not_usable(graph, project)` (Mem's order, §Refusal orders).
New `pub(crate)` helper beside it:

```rust
/// MOD-147 D5: the member of `project`'s owner workspace (by position, then slug bytes) whose kinds
/// use `prefix`; `None` when `project` is not a scope row or no member does.
async fn workspace_prefix_holder(&self, project: ProjectId, prefix: &str) -> Result<Option<String>>
```
```sql
SELECT m.slug
  FROM project sr
  JOIN workspace_project wp ON wp.workspace_id = sr.owner_workspace_id
  JOIN project m ON m.id = wp.project_id
 WHERE sr.id = $1
   AND EXISTS (SELECT 1 FROM item_kind mk WHERE mk.project_id = m.id AND mk.prefix = $2)
 ORDER BY wp.position, m.slug COLLATE "C"
 LIMIT 1
```

**`update_item_kind`** (`:3686`): the graph guard (`:3714-3725`) gains `OR g.workspace_id = (SELECT
p.owner_workspace_id FROM project p WHERE p.id = item_kind.project_id)`; one more conjunct:

```sql
   AND ($3::text IS NULL
        OR $3 = item_kind.prefix
        OR NOT EXISTS (SELECT 1
                         FROM project sr
                         JOIN workspace_project wp ON wp.workspace_id = sr.owner_workspace_id
                         JOIN item_kind mk ON mk.project_id = wp.project_id
                        WHERE sr.id = item_kind.project_id AND mk.prefix = $3))
```

Failure path (`:3750-3768`), after NotFound and Stale: `if let Some(prefix) = patch.prefix.as_deref().filter(|p|
*p != current.prefix) && let Some(member) = self.workspace_prefix_holder(current.project_id, prefix).await? {
return Err(Constraint(workspace_prefix_taken(prefix, &member))) }`, then the existing `graph_not_usable`. The
comment at `:3762-3764` names both conjuncts. No lock against a concurrent member kind (D5).

### T2.4 `usable_graph_rows` (`pg/read.rs:3128-3163`)

`WHERE` (`:3143-3147`) gains `OR g.workspace_id = (SELECT p.owner_workspace_id FROM project p WHERE p.id = $1)`.
Doc (`:3121-3123`) "and, for a scope row, its own workspace's (MOD-147 D7)".

### T2.5 Held graphs on workspace delete (`pg/write.rs:10123-10148`)

Both arms of the held query gain `AND pr.owner_workspace_id IS DISTINCT FROM $1` after `WHERE g.workspace_id = $1`.
Nothing else in `delete_workspace_once` moves, so the source scan `delete_workspace_once_counts`
(`pg/read.rs:3910-3920`, cuts at the first `"\n    }\n"`) still finds the live filter. Comment: "MOD-147 D7: the
workspace's own scope row cascades with it, so its holders do not hold".

### T2.6 Scope-row runs (`pg/write.rs`)

**`create_run`** (`:6105`): after the `item_not_in_project` check (`:~6160`), on `tx`:

```sql
SELECT slug, owner_workspace_id AS "owner_workspace_id: WorkspaceId" FROM project WHERE id = $1
```
(`fetch_one`: the item's project exists, it is locked through the item), then `run_workspace_for(owner,
new.workspace_id)` → `Err(asked)` → `Constraint(scope_row_run_workspace(&slug, asked))`. The `INSERT` binds the
resolved value for `$11` (`:6233`).

**`queue_item`** (`:10304`): the existence probe (`:10312-10322`) becomes

```sql
SELECT p.slug, p.owner_workspace_id AS "owner_workspace_id: WorkspaceId"
  FROM item i JOIN project p ON p.id = i.project_id
 WHERE i.id = $1
```
(no row → `NotFound { entity: "item" }`), then `run_workspace_for` → refusal, all **before** the `INSERT … ON
CONFLICT (item_id) DO NOTHING` (`:10323-10336`), which binds the resolved workspace for `$5`. The doc
(`:10294-10302`) adds the rule.

### T2.7 Upstream scope (D9)

Pg `upstream_summaries` (`pg/read.rs:629-633`), binds unchanged (`$3` reused):

```sql
scope AS (
        SELECT project_id FROM workspace_project WHERE workspace_id = $3::uuid
    UNION
        SELECT id FROM project WHERE owner_workspace_id = $3::uuid
    UNION
        SELECT $4::uuid WHERE $3::uuid IS NULL
)
```

Cache (`cache/read.rs:899-903`): `SELECT project_id FROM workspace_project WHERE workspace_id = ? UNION SELECT id
FROM project WHERE owner_workspace_id = ? UNION SELECT ? WHERE ? IS NULL`. Binds (`:917-922`) become, in order:
`id`, `hops`, `workspace`, **`workspace`** (new, 4th), `scope.project`, `workspace`, `id`. The doc above (`:870-876`)
says the workspace is bound three times. The mirror's `project.owner_workspace_id` exists since cache 0009 (M1).

### T2.8 `.sqlx` and tests

Changed or new statements: `item_by_key` (qualified), `propose_link` lookup, `create_item_kind`, `update_item_kind`,
`workspace_prefix_holder`, `usable_graph_rows`, `delete_workspace_once` held query, `create_run` project read,
`queue_item` probe, `upstream_summaries`. One `cargo sqlx prepare` at the end against a migrated scratch DB (README
`:723-739`; in the sandbox `sqlx database create` instead of `psql`, which the host lacks).

- `tests/pg_conformance.rs:53` 248 → **255**; doc block (`:40-52`) and message (`:62-71`) gain "and MOD-147 M2's
  seven key, reach, prefix, graph, run-workspace and upstream cases make it 255".
- `tests/pg_criteria.rs` (V2): `queue_item_fills_a_scope_rows_workspace_on_both_stores` — on `pg` and `mem`:
  mint WS on `PROJECT_SCOPE_PLATFORM`; `None` → `Some(PLATFORM)`; another WS item with `Some(GRAPHICS)` → the same
  `Constraint` sentence on both, `queue_entries` unchanged; re-queue of the first with `Some(GRAPHICS)` → refused on
  both (before `ON CONFLICT`); re-queue with `None` → the stored entry on both.
- `tests/cache.rs`: `the_mirror_counts_the_scope_row_in_upstream_scope` — Pg demo; mint `WS-1` on
  `PROJECT_SCOPE_PLATFORM`; raw `INSERT INTO item_link (…) VALUES (AGY_FIX_1, ws, 'blocked_by', …)` (the
  `a_cycle_never_renders_the_root_as_its_own_upstream` pattern, `:1901`); `run_pass` with the scope row;
  `cache.upstream_summaries(AGY_FIX_1, 2, {Some(PLATFORM), AGY})` == Pg's, and the WS entry is `in_scope`.

**Build order (T2)**: pin + the two tests (red) → T2.1-T2.7 → `cargo sqlx prepare` → tests.

---

## T3 — MCP `item_link` (htui-mcp)

Files: `crates/htui-mcp/src/tools/link.rs`, `crates/htui-mcp/tests/tools_backlog.rs`,
`crates/htui-mcp/src/tools/snapshots/htui_mcp__tools__box_profile__tests__tools_list_per_scope_shape.snap`,
`docs/htui-mcp.md` (`:80`).

### T3.1 `link.rs`

- Module doc (`:4-5`) and the `LinkArgs` comment (`:31-33`): `to` is `KEY` (this project), `slug:KEY` (another project
  of its workspaces) or `@workspace:KEY` (a workspace's own item).
- `to` field doc (`:40`): ``/// The other item's key: `FEAT-12` in this project, `api:FEAT-12` in project `api`, or `@shop:WS-1` for workspace `shop`'s own item.``
- `DEF.description` (`:62-63`): `"Proposes or withdraws a link from this step's item to another item of its project or its workspaces, named by key: FEAT-12 here, api:FEAT-12 in project api, @shop:WS-1 for workspace shop's own items."`
- Constant `fn out_of_scope(to: &str) -> ToolError` = `format!("out of scope: {to} is not an item of this project or its workspaces")`.
- `call` (`:69-130`): `item_by_key(store, scope.project_id, &to)` (unchanged call; qualified text resolves) →
  `None` → `out_of_scope(&to)`. Then `let target_item = WorkerStore::item(store, target)…` and the canonical `to`:
  bare `target_item.key` when `target_item.project_id == scope.project_id`, else
  `format!("{}:{}", WorkerStore::project(store, target_item.project_id)…slug, target_item.key)` (both reads
  `NotFound` → `not found: …`). The error map gains
  `StoreError::Constraint(s) if s == link_outside_reach(target) => out_of_scope(&to)` (the agent's own text), and
  `not yours` prints the canonical `to`. The reply's `"to"` is the canonical form.

### T3.2 Tests (`tools_backlog.rs`, item_link section `:571-700`)

`Leased::on` (`:80`) splits into `on_store(store: MemStore, project, item)`; `on` calls it with `MemStore::demo()`. A
helper `ws_item(store, scope) -> (ItemId, String)` mints `WS` on a scope row.

- Edit `item_link_to_another_projects_key_is_out_of_scope` (`:597-616`): the expected text becomes `"out of scope:
  {to} is not an item of this project or its workspaces"` (bare `FEAT-3` from `agy` and the raw id still miss).
- `item_link_add_resolves_a_qualified_key_of_a_co_member` — `agy` `FIX-1` session, `to: "htui:FEAT-3"` relates →
  `{"from":"FIX-1","to":"htui:FEAT-3",…}`, live.
- `item_link_add_reaches_the_workspaces_own_item` — `on_store` with `@platform`'s `WS-1` minted; `htui` `ANA-2`
  session, `to: "@platform:WS-1"` → reply `to: "@platform:WS-1"`, live.
- `item_link_from_a_scope_row_reaches_a_member` — a session on `@platform:WS-1` (run created with `workspace_id
  None`, filled by D8), `to: "htui:FEAT-1"` → live.
- `item_link_names_its_own_project_bare` — `htui` session, `to: "htui:FEAT-1"` → reply `to: "FEAT-1"`.
- `item_link_outside_the_workspaces_is_out_of_scope` — `htui` session, `to: "vulkan-tutorials:FEAT-1"`, op `add` →
  `"out of scope: vulkan-tutorials:FEAT-1 is not an item of this project or its workspaces"`, no live link.
- `item_link_to_an_unknown_slug_is_out_of_scope` — `to: "nope:FEAT-1"` and `"@gone:WS-1"`, both ops → the same text.
- Snapshot: `cargo insta test -p htui-mcp` moves the tools-list snapshot (3 scopes); review the description and the
  `to` field text only.

`docs/htui-mcp.md:80`: `to` "(an item key: `FEAT-12` in this project, `api:FEAT-12` in another project of its
workspaces, `@shop:WS-1` for a workspace's own item)"; the answer's `to` is "canonical: bare in this project,
`slug:KEY` elsewhere".

---

## T4 — htui (join warning, waiting scope, CLI qualification)

Files: the plan's T4 set, plus `crates/htui/tests/secrets_settings.rs` (one passive-adoption test).

### T4.1 Join warning (D6, V4)

`store_worker.rs`, beside `Hierarchy` (`:1617`):

```rust
/// MOD-147 D6: answer to [`StoreRequest::CreateProject`]: the tree with the new project linked, its
/// slug, and the prefixes its kinds share with the workspace's scope row (kind order of the new
/// project; empty when none). A notice, never a refusal.
HierarchyJoined {
    tree: Box<HierarchySnapshot>,
    project: String,
    shared_prefixes: Vec<String>,
},
```

`hierarchy.rs` `CreateProject` (`:321-357`): after `upsert_workspace_project`, `shared_prefixes` = the new project's
`writer.item_kinds(project.id)` prefixes that `writer.item_kinds(scope)` also has, where `scope =
writer.scope_project(*workspace)` (`None` → empty); then `Ok(StoreReply::HierarchyJoined { tree:
Box::new(fresh_tree(&writer, *workspace, this_box).await?), project: project.slug, shared_prefixes })` instead of
`reread`. (`Writer` serves `scope_project` as `ReadStore`, `item_kinds` as `WriteStore`.)

`settings/hierarchy.rs` `on_reply` (`:1536`), after the `Hierarchy(Some)` arm (`:1553`):

```rust
StoreReply::HierarchyJoined { tree, project, shared_prefixes } => {
    self.on_tree(tree, ctx);
    if !shared_prefixes.is_empty() {
        self.notice = Some(joined_notice(project, &tree.workspace.slug, shared_prefixes));
    }
}
```
```rust
/// MOD-147 D6: "`api` shares prefix FEAT with workspace `shop`: keys show as `api:FEAT-1` and
/// `@shop:FEAT-1`" ("prefixes FEAT, FIX" for several; the example uses the first).
fn joined_notice(project: &str, workspace: &str, prefixes: &[String]) -> String
```

`settings/secrets.rs:1619-1622`: the passive arm gains `| StoreReply::HierarchyJoined { tree, .. }`. No exhaustive
`StoreReply` match exists (`observe_reply` ends in `_ => {}`).

### T4.2 Waiting scope and qualifier (D10, D11, V5, V6)

`htui-worker/src/views.rs`:
- `WaitingRow` (`:523`) gains, after `item_key`:
  ```rust
  /// The item's project (MOD-147 D10): a scope row's for a workspace item.
  pub project: ProjectId,
  /// `item_key` as this answer prints it (D11): `slug:KEY` when another project of the scope uses the
  /// prefix, else bare. The overlay and `htui waiting` draw it; `Enter` still reveals by `item`.
  pub display_key: String,
  ```
- `waiting` (`:600`) gains a last parameter `keys: &KeyQualifier`; the candidate literal (`:635`) fills `project:
  item.project_id, display_key: keys.display(item.project_id, &item.key)`, the permission literal (`:782`)
  `project: p.project, display_key: keys.display(p.project, &p.item_key)`.
- Tests: `one()` (`:1460`) and the six direct calls pass `&KeyQualifier::default()`; literals `:1477`, `:2142` gain the
  fields. New `rows_carry_their_project_and_display_key` (two projects sharing `FEAT` → `api:FEAT-1`; a lone prefix
  bare).

`store_worker.rs`:
```rust
/// MOD-147 D10, D11: the scope the waiting reads use — the asked one with the workspace's scope row
/// first, when it has one — and the key qualifier over it (`project` + `item_kinds` per project,
/// the mirror's offline). The reply still names the asked scope (`app/update.rs:515`).
pub(crate) async fn waiting_reads(backend: &Backend, scope: &Scope) -> StoreResult<(Scope, KeyQualifier)>
```
`serve_waiting` (`:2281`): `let (read, keys) = waiting_reads(backend, scope).await?;` then `active_runs(&read)`,
`waiting_candidates(&read)`, `read_permissions(writer, &read)`, `htui_worker::waiting(&read, …, &keys)`; the reply's
`scope: scope.clone()` stays the asked one.

`state_cmd.rs`:
- `Target` (`:~977`) gains `whole_workspace: bool` (`first_workspace` true, `project_target` false; empty target
  false).
- `answer_online` (`:189`): `let (read, keys) = waiting_reads(pg, &target).await?` — the twin over `PgStore`
  (`scope_project`, `project`, `WriteStore::item_kinds`), widening only when `target.whole_workspace` (V5); the four
  reads use `read`; `waiting_body(&target, &view, ctx.now, &keys)`.
- `WaitingRowDoc` (`:810`) gains `project: String` (the row's slug, `keys.slug`), `qualified_key: String`
  (`keys.qualified`), and `#[serde(skip)] display_key: String`; `row_doc` (`:~1110`) fills them.
- `human_waiting` (`:1153`): the key column is `display_key`, padded to `max(10, longest display_key)` (`{:<width$}`).
- `docs/htui-cli.md`: the waiting row field table gains `project` and `qualified_key` (additive, `version` stays 1,
  `:408-411`); a sentence that without `--project`/`--cwd` the list covers the workspace's own items, first.

Overlay `ui/overlay/waiting_list.rs`: `Columns::of` (`:189`) and `item_row` (`:214-228`) use `display_key`. `enter`
(`:128-140`): when `!ctx.scope.contains(row.project)` (a scope-row row: the asked scope never holds the scope
row) → `ctx.emit(Action::Error(WORKSPACE_ITEMS_LATER.to_owned()))` and the overlay stays open, where
`const WORKSPACE_ITEMS_LATER: &str = "workspace items open in the Backlog from MOD-147 M3";`.

The 13 `WaitingRow` literals: `views.rs:635, 782, 1477, 2142`; `state_cmd.rs:1239`; `store_worker.rs:4405, 4415`;
`header.rs:625`; `leave.rs:242`; `waiting_list.rs:450, 463, 471, 635`. `tests/state_cli_pg.rs:241` passes
`&KeyQualifier::default()` (its parity compares no key text).

### T4.3 Tests (written first)

- `tests/hierarchy.rs`: the `tree()` helper (`:47`) also unwraps `HierarchyJoined`.
  `create_project_answers_the_prefixes_it_shares_with_its_workspace` — on the Memory backend's store,
  `create_workspace(empty)` and a `FEAT` kind on its scope row (no members, so D5 allows it; edge 12); `CreateProject
  { workspace, slug: "api" }` → `HierarchyJoined { project: "api", shared_prefixes: ["FEAT"], .. }`, tree lists
  `api`. `create_project_sharing_no_prefix_answers_an_empty_list` (Platform). Section bench:
  `a_joined_reply_naming_shared_prefixes_shows_the_notice` (the rendered notice equals `joined_notice`'s text);
  `a_joined_reply_without_shared_prefixes_shows_no_notice`.
- `tests/secrets_settings.rs`: `a_joined_reply_is_adopted_passively`.
- `store_worker.rs` tests: `serve_waiting_reads_the_demo_platform` (`:4367`) — rows gain `project: HTUI,
  display_key: "htui:FEAT-2"` / `"htui:TOOL-1"` (Platform's members share every seeded prefix);
  `serve_waiting_lists_the_scope_rows_items_first` — mint `WS-1` on `PROJECT_SCOPE_PLATFORM`, `create_run`
  (`workspace_id None`), `create_step` `review` at 0, `park_step(StepFence::Unleased, step)`: `rows[0]` is the
  WS gate, `project == PROJECT_SCOPE_PLATFORM`, `display_key == "WS-1"`, and the reply's `scope` equals the asked
  one (top-bar adoption, `update.rs:515`).
- `waiting_list.rs` tests: `enter_on_a_workspace_row_says_where_it_opens` (emits `Action::Error(..)`, no `Close`,
  no `Reveal`); `the_key_column_draws_the_display_key` (insta).
- `state_cmd.rs` tests: `human_waiting_widens_the_key_column_to_the_longest_key`;
  `row_doc_carries_project_and_qualified_key`; `a_project_target_is_not_a_whole_workspace`; the `:1650`/`:1659` calls
  pass a qualifier.
- `tests/state_cli_pg.rs`: `waiting_lists_the_workspace_items_first_and_qualifies_shared_prefixes` — demo DB; mint
  `WS-1` on `@platform` and park a gate on it (store writes + `seed_gate`-style `UPDATE`); `htui waiting --json` (no
  `--project`, first workspace Platform): `rows[0]` `{item_key "WS-1", project "@platform", qualified_key
  "@platform:WS-1"}`, a later row `{item_key "FEAT-2", project "htui", qualified_key "htui:FEAT-2"}`; the human form's
  first line starts `WS-1 ` and another holds `htui:FEAT-2`.
- Full insta run: every moved waiting snapshot shows `htui:`/`agy:` keys (D11, plan risk); review each.

---

## Compile and merge order

T1 first. After T1 the whole workspace compiles (V3 keeps htui-store building; no trait gains a method, so no
implementor moves): Mem has every rule, Pg only the renamed sentence, so `pg_conformance` fails its seven new cases
until T2. Then T2 ∥ T3 ∥ T4 on disjoint files: T3 and T4 test on MemStore; T4 uses T1's `KeyQualifier`,
`Scope::with_scope_row` and `Backend::item_kinds`.

## Verification hooks

Run each from `/home/mluigi/projects/htui`, sequentially, never in a `pgrep` wait loop. With `--no-fail-fast`, grep the
output for `SIGABRT` (stack overflows report as aborts). `--all-features` turns on `testkit`/`test-support`: without
it htui's `tests/*.rs` run 0 tests and report ok.

| Task | Command | Proves |
|---|---|---|
| T1 | `cargo test -p htui-core --all-features --no-fail-fast -- --test-threads=1` | key.rs, sentences, Mem rules, 255 cases on Mem, pin |
| T1 | `cargo test -p htui-store --all-features --test cache --no-fail-fast -- --test-threads=1` | `CacheStore::item_kinds` / `Backend::item_kinds` (needs the sandbox Postgres DSN) |
| T1 | `cargo check --workspace --all-features --tests` | V3: nothing else broke |
| T2 | `cd crates/htui-store && DATABASE_URL=<scratch htui_sqlx> cargo sqlx prepare -- --all-targets --all-features`, then `cargo sqlx prepare --check` | `.sqlx` current |
| T2 | `cargo test -p htui-store --all-features --no-fail-fast -- --test-threads=1` | Pg twins: `pg_conformance` 255, `pg_criteria` queue test, `cache` upstream test. `qdrant_live` is owed on host. `migrations.rs::the_ana_column_comments_are_present_and_verbatim` already fails on main (not this milestone) |
| T3 | `cargo test -p htui-mcp --all-features --no-fail-fast -- --test-threads=1`; `cargo insta test -p htui-mcp --all-features` (review) | item_link resolution, canonical `to`, sentences, tool text |
| T4 | `cargo test -p htui-worker --all-features --no-fail-fast -- --test-threads=1` | `waiting()` fields |
| T4 | `cargo test -p htui --all-features --no-fail-fast -- --test-threads=1` | join notice, widened waiting, CLI (state_cli_pg needs Postgres) |
| T4 | `cargo insta test --workspace --all-features` | every moved snapshot reviewed |
| all | `cargo fmt --all --check`; `cargo clippy --workspace --all-features -- -D warnings`; `cargo clippy --workspace -- -D warnings`; `bash scripts/gate.sh` | the gate (live-Qdrant suites owed on host) |

## Open for the maintainer

- V4's reply shape (`project` slug added) and V5's CLI rule (no scope row under `--project`/`--cwd`) are the two
  places this blueprint departs from the plan's text.
- Edge 12: with seeded kinds, the join notice fires only after a custom workspace kind; confirm that is the intent.
