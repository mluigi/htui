# ANA-35 - Entity model for any kind of project

> **Scope note:** Settle the schema and migration for the entity model the 2026-10-15 requirements
> review asked for: a project **type** that seeds kinds and graphs (software, research, docs, ops, and
> the non-software seeds); **repo-less** projects and repos that are **plain folders**, and what refuses
> without git; workspaces with **no root path** that **own items, documents and requirements** (their
> key space and kinds, and `R-ENT-7`'s single counter); documents owned by a project or a workspace;
> **attachments** in Postgres under a size cap; link kinds `parent`, `duplicates`, `implements` and links
> to documents or attachments; the item status reduced to **open / blocked / closed**, with `queued`,
> `in_progress`, `awaiting_approval` and `failed` moving to the run; and MOD-106's question of the
> primary repo (`HANDOFF.md:123-134`). Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`,
> `docs/REQUIREMENTS.md`, `docs/requirements-review-2026-10-15.md` (the item's origin) and
> `docs/ui-redesign-findings-2026-10-15.md` §4.1-§4.2 (Workspaces, Backlog). Best read beside ANA-34
> (concluded), whose close-out law and workspace runs this item builds on, and ANA-33 (not yet
> written), which owns the graph model the type seeds fill.
>
> **Requirements addressed:** `R-ID-1`, `R-ENT-1`, `R-ENT-2`, `R-ENT-3`, `R-ENT-4`, `R-ENT-5`,
> `R-ENT-6`, `R-ENT-7`, `R-ENT-8`, `R-ENT-9`, `R-ENT-12`, `R-ENT-14`, `R-ENT-16`, `R-BOX-4`, `R-STO-8`,
> `R-TUI-2`. Touched at their seams: `R-ID-3` (where a workspace's description and goals live), `R-ID-7`
> (scrubbing attachments), `R-ENT-15` (workspace requirement splits), `R-ENT-17`/`R-SEC-5` (the workspace
> folder), `R-STO-3`/`R-STO-5` (cache contents, migrations), `R-ORCH-6`/`R-ORCH-8`/`R-ORCH-9` (readiness,
> isolation, overlap), `R-ORCH-14` (close-out law, workspace runs), `R-ORCH-15` (targets and folders),
> `R-TUI-1`/`R-TUI-3`/`R-TUI-5`/`R-TUI-8`/`R-TUI-11` (status words, attachments, graph, Settings,
> waiting-on-you), `R-MCP-1`/`R-MCP-2` (agent-visible enums, link addressing), `R-LATER-3` (the
> importer), `R-LATER-10` (object store).
>
> **Status (2026-10-18): drafted, awaiting maintainer acceptance of §8.** No implementation code is
> written here. Verdict (§7): the item keeps **three stored statuses** (`open`, `blocked`, `closed`);
> everything a person sees as progress (`queued`, `running`, `needs you`, `failed`, `done`) is **derived
> from the item's latest graph run**, and "one live run per item" moves from an item-status lock to a
> partial unique index on `run`. The migration closes nothing a run can explain: rows in the five
> removed states become `open`, and a finished item waits for a person's close-out; a `done` row with no
> finished run is closed `done` with a note, and a `failed` row with no failed run becomes `blocked` with
> a note. A blocker still stops blocking when its work is finished, as `done` does today. A **repo is
> `git` or `folder`**; a folder does `local` and `copy` (a `worktree` step copies it) and never gets a
> worktree, merge, push, clone or a driven target; a phase flagged **`needs_repo`** refuses at
> snapshot in a project without repos; a folder may be the primary; "repos means exactly one primary" is
> enforced at the store write with a deferred trigger behind it, and removing a repo archives it
> (MOD-106's part). **Project types are rows** whose seed is copied once at creation (software,
> research, docs, ops, general). **A workspace owns its items, kinds, key space, graphs, documents,
> requirements and runs through one hidden scope row in `project`**, so every per-project path (counter,
> cache, key lookup) serves it unchanged; search gains points for documents without an item. Workspace
> requirements are `W-<AREA>-<N>`, stored at mint, with nested children keyed and made unique. Documents
> gain `project_id` on every row and a nullable `item_id`. Attachments belong to an item or a document
> version, live in a separate `bytea` table, 10 MiB by default and 64 MiB at most; text is scrubbed, a
> binary is accepted only from a person. `parent` is single-valued and acyclic; closing as `duplicate`
> or `superseded` records its link; links to documents and attachments live in their own table and pin a
> document version. Proposed spawns: **P1** to **P7**; the workspace scope (P4) waits only on the
> small project-type table (P3), not on folders or seeds.

---

## 1. Context and problem statement

`htui` was built for code: every project was assumed to have git repositories, every item walked a
software graph, and the item's status tracked each run's progress. The 2026-10-15 requirements review
widened it to "any kind of project — software, research, documentation, operations, or any task an
agent can do" (`R-ID-1`, `docs/REQUIREMENTS.md:88-92`) and drew the consequences in one sweep
(`docs/requirements-review-2026-10-15.md:12-16, 54-88`): a project may have zero repos, a repo may be a
plain folder, a project has a type that seeds its kinds, a workspace is no longer a path root but owns
work of its own, documents and attachments get more owners, links get three new kinds, and the item
status shrinks from eight values to three.

Each of those is a schema change with consumers spread through the store, the orchestrator, the cache,
search, the MCP server and the TUI. The requirements were amended to say *what*; several say "design in
ANA-35" (`R-ENT-3`, `R-ENT-6`, `R-ENT-8`, `R-ENT-14`, `R-ENT-16`). Eight open items wait on the answer:
MOD-106 (primary repo), MOD-135 (close-out checklist), MOD-136 (workspace runs), MOD-112 (workspace
search), MOD-114 (attachments), MOD-8 (the importer), MOD-116 (workspace folder) and MOD-118 (workspace
requirements); MOD-119 (status grouping) and MOD-133 (folder integration) consume parts of it. This item
settles the model, the DDL and the migration order, and names the requirement
text the verdict needs.

Method: six readers in parallel (requirements and decisions; Postgres schema and store; item-status
consumers; repos, git and seeding; documents, links, requirements and search; prior art), each
returning facts with `path:line` or URL evidence. Where readers disagreed or hedged, the facts were
re-checked in the tree before writing (the status CHECK, `workspace_project`'s key, `closes_from`, the
readiness SQL, `waiting_candidates`, `finish_run`'s live-run count). Facts below are theirs; §6 and §7
are the synthesis. Caveats: line numbers inside long functions are from read windows and may be off by
one or two; a few Postgres facts are recalled, not fetched, and are labelled so.

## 2. Surface as read

### 2.1 What the requirements now say

- **Projects and repos.** `R-ENT-1`: "`repo` is a git checkout or a plain folder" (`:158-159`).
  `R-ENT-3`: a project "owns the item key space, the item kinds, step graphs, skill bindings, secret
  provider binding and phase defaults. It has a type (software, research, docs, ops, and more) that
  seeds its kinds and graphs … zero or more repos; when it has any, exactly one is primary. A phase that
  needs a repo refuses to run in a project without one" (`:169-173`). `R-ENT-4`: "What needs git —
  `worktree` isolation, commit hashes, merging — needs a git repository; `copy` and `local` isolation
  work on a folder" (`:174-177`). `R-BOX-4`: per-box paths "for every repo and folder" (`:148-154`).
- **Workspaces.** `R-ENT-2`: "not a directory its projects must live under. It owns its own items,
  documents and requirements … There are no implicit workspace rows … A workspace has a folder (by
  default `~/workspaces/<name>`) where `htui` keeps its description, goals, generated compose file
  (R-ENT-17) and, without Infisical, its environment file (R-SEC-5) … The per-box root path is dropped"
  (`:160-168`). `R-ENT-16`: "a workspace has its own key space and kinds, and owns items, documents and
  requirements that span its member projects, linked to their items by UUID" (`:240-242`).
- **Items and keys.** `R-ENT-5`: key `<PREFIX>-<N>`, "unique per project" (`:178-182`). `R-ENT-7`: "a
  per-project, per-prefix sequence held by Postgres. Never reused: a project has exactly one counter and
  exactly one writer of it" (`:196-198`). The review's example for the workspace was "`WS-1`" and it
  noted the new ownership "touches R-ENT-1..3, R-ENT-7 and R-PRM-1 scope"
  (`docs/requirements-review-2026-10-15.md:86-88`).
- **Kinds.** `R-ENT-6`: the seeded set "depends on the project type … (for example research → verdict,
  outline → draft → review, plan → execute → verify), settled by ANA-35" (`:183-195`). The review's
  proposals: research `RES` (research → verdict) and `Q` (question → answer); docs `DOC` (outline →
  draft → review); ops `OPS` (plan → execute → verify) and `INC` (investigate → fix → postmortem); "the
  prefixes are proposals to settle in the item that builds this" (`requirements-review:63-69`).
- **Status.** `R-ENT-8`: "one of `open`, `blocked` or `closed`. The progress of work — `queued`,
  `in_progress`, `awaiting_approval`, `failed` — belongs to the item's runs … Resolution is not a status"
  (`:199-204`); the review adds "`done` is no longer a status (it stays a resolution)"
  (`requirements-review:73-76`). ANA-34 §10.4's replacement of the transition sentence is accepted but
  not yet applied (`docs/ANA-34.md` §10.4).
- **Links.** `R-ENT-9`: kinds `blocked_by`, `origin`, `relates`, `supersedes`, `parent` ("decomposition
  … unlike `origin`, which is provenance"), `duplicates`, `implements`; "a link may also target a
  document or an attachment"; links come from agents through MCP and from the importer, "there is no
  manual link action" (`:205-210`).
- **Documents and attachments.** `R-ENT-12`: "versioned per owner and kind; the owner is an item, a
  project or a workspace … Items and documents may carry attachments (images, logs, other files), held
  in Postgres under a size cap in version one" (`:216-220`); the object store is `R-LATER-10` (`:695`).
- **Requirements.** `R-ENT-14`: "key `R-<AREA>-<N>` minted per owner and area … the owner is a project
  or a workspace … A requirement may refine another; the child's key is its parent's plus `.n` …
  Workspace requirements are keyed `W-…` (ANA-35 settles the exact form)" (`:226-232`). `R-ENT-15`: a
  workspace requirement "can be split into project requirements, each refining it" (`:233-239`).
- **Display and search.** `R-TUI-1`: "status shows as a glyph and a human word (`needs you`, not
  `awaiting_approval`)" (`:530-540`). `R-TUI-2`: grouping cycles "project, kind and status (attention
  order), done and closed items stay hidden until shown" (`:541-547`). `R-TUI-11` lists "a blocked run"
  among things waiting on a person (`:621-626`). `R-STO-8`: searches "scoped to projects or a workspace,
  workspace-owned items, documents and requirements included", and a decisions filter on items "closed
  as `done`, `concluded` or `rejected`" (`:290-302`).
- **The redesign's vocabulary.** Glyphs `● done ◐ running ⚠ needs you ✗ failed ⊘ blocked ◌ queued ○
  open ✓ closed`; status groups in attention order "needs you, failed, running, blocked, queued, open,
  done, closed" (`docs/ui-redesign-findings-2026-10-15.md:115-117, 286-290`). Requirement ids "workspace
  ones `W-n`, project ones the workspace's shared `R-n` (§8 Q8 still open)" (`:261-265`), later answered
  only in part: "nested ids stay … workspace ones are `W-n`" (`:657-658`).

### 2.2 Item status today and who reads it

- **Eight stored values.** `Status` in `crates/htui-core/src/model/item.rs:8-28`; the DDL CHECK lists all
  eight inline and unnamed (`crates/htui-store/migrations/0001_init.sql:319-320`). `is_terminal` is
  `Done | Closed` and drives the readiness rule (`item.rs:30-36`); `can_move_to` is ANA-2 §4.3's table
  plus D161's `blocked → awaiting_approval` (`item.rs:52-65`).
- **The close-out law keys on removed values.** `Resolution::closes_from`: `done` and `concluded` close
  only a `done` item; the other four close `open`, `blocked`, `failed` or `done` (`item.rs:94-104`).
  `default_for` maps `done → Done` and `open|blocked|failed → Withdrawn` (`item.rs:107-114`).
  `chk_item_resolution_iff_closed` ties resolution to `closed` (`0006_requirements.sql:27-28`).
- **The run already carries every progress state.** `RunStatus` is `queued | running |
  awaiting_approval | done | failed | cancelled` (`crates/htui-core/src/model/run.rs:33-50`; CHECK at
  `0001_init.sql:453-454`). The one item state with no run twin is "blocked over a parked run", which
  `gate.rs::escalate` sets after review-loop exhaustion, and which stays representable because `blocked`
  stays on the item.
- **Writers that mirror run moves onto the item.** `create_run` locks the item and moves it to
  `queued` after `legal_move(status, Queued)` (`crates/htui-store/src/pg/write.rs:4540, 4644`);
  `claim_run` moves it `queued → in_progress` or `queued → blocked` (`write.rs:4795, 4894`);
  `promote_step`/`park_step` move it to `awaiting_approval` (`write.rs:6004, 6145`); `finish_run_on`
  applies `finish_run_item_mirror` when no other run of the item is live (`write.rs:979-1006`;
  `crates/htui-core/src/store/traits.rs:2838-2847`). The engine makes 13 production `.transition(`
  calls (`crates/htui-orch/src/engine.rs:846, 869, 1717, 1739, 1878, 2618, 2972, 3209, 3460, 4790, 5618,
  5760, 6791`; three more are in tests from `:11534`): 7 mirror a run move (`cancel_queued`,
  `recover_run`, `park_interrupted`, `resume_window`, `park_selection`, `park_run`, `unpark`); 4 block
  the item (`refuse_rung_four`, `refuse_missing_tags`, `refuse_no_candidate`, `block_and_fail`), a fifth
  blocking write being `gate.rs::escalate` (through `move_item`, `gate.rs:344-349, 1061`); and 2 are `unblock`'s (`blocked → open`, and
  FollowRun's `blocked → awaiting_approval`). `enqueue_with` makes none; it reaches the item through
  `create_run`. MemStore repeats every move (`crates/htui-core/src/store/mem.rs:2195, 5274, 6420, 6499`).
- **`done → open` has no production caller.** The edge is legal (`item.rs:63`), but only tests take it
  (`crates/htui-core/src/store/conformance.rs:1281`; `crates/htui-store/tests/pg_criteria.rs:594, 618`).
- **Chat runs carry no item.** `start_chat_run` inserts `item_id NULL` (`pg/write.rs:2034-2039`), and
  `run.kind` is `graph | chat` only (`0001_init.sql:451`). The comment on `finish_run_item_mirror` that
  MOD-2's chat path and pre-MOD-25 offline uploads inserted `run` and `item` rows outside ANA-2 §4.3
  (`traits.rs:2833-2836`) means old rows may hold a status no run explains.
- **Item status is also a lock.** `create_run`'s `legal_move(status, Queued)` under `FOR UPDATE` is the
  only guard against two live runs on one item; no unique index exists on `run(item_id)`. The finish
  path counts other live runs of the item without filtering on `kind` (`write.rs:979-982`).
- **Item status is also the scheduler filter.** `items(ready)` and `ready_items` require `status =
  'open'` and no live `blocked_by` target `NOT IN ('done','closed')` (`crates/htui-store/src/pg/read.rs:116,
  2100`; `queue_rows` `:2367`; cache `crates/htui-store/src/cache/read.rs:402`). That silently excludes
  running, parked, failed and done items today.
- **Waiting-on-you** selects `i.status IN ('blocked','awaiting_approval') OR EXISTS (run awaiting_approval)`
  (`pg/read.rs:1209-1211`; cache `cache/read.rs:1321-1323`); every `WaitingRow` needs an item
  (`docs/ANA-34.md` §2.6).
- **Display and agents.** The Backlog filter walks `Status::ALL` (`crates/htui/src/ui/tabs/backlog/filter.rs:67,
  307-377`); `theme.rs::status_style` and the MOD-115 kit table give each of the eight a glyph and word
  (`crates/htui/src/ui/theme.rs:263-269`; `crates/htui/src/ui/kit/status.rs:19-76`). The MCP
  `item_status` and `search_concepts` schemas build their enums from `Status::ALL`
  (`crates/htui-mcp/src/tools/status.rs:33-36`; `tools/search.rs:55-61`), and the prompt prints upstream
  status ("(in_progress)") into the text (`crates/htui-core/src/prompt/render.rs:363-391`).
- **Search.** The Qdrant payload carries `status` and `resolution`, indexes `status`, and uses `status`
  as a staleness key because a status move does not touch `updated_at`
  (`crates/htui-store/src/vector.rs:67, 540, 792`; `vector_sync.rs:5-8, 89-98`). A point whose `status`
  no longer parses drops out of `indexed_point` (`vector.rs:600-618`). The decisions filter is
  **resolution-only** (`crates/htui/src/concepts.rs:67-94`; MOD-50), so it survives the reduction.
- **Blast radius.** References to `Status::{Queued, InProgress, AwaitingApproval, Failed}`: htui-core
  19 production / 104 test, htui-orch 17 / 63, htui-worker 0 / 20, htui 8 / 25; about 19 of 386
  `.sqlx` entries carry item-status literals; about 57 snapshots mention the removed words, several of
  them run or step statuses.

### 2.3 Repos, git and the primary repo

- **No discriminator.** `repo` has `remote_url` nullable, `default_branch TEXT NOT NULL DEFAULT 'main'`,
  `is_primary`, and `uq_repo_primary ON repo(project_id) WHERE is_primary`, whose comment says "exactly
  one" but which enforces at most one (`0001_init.sql:185-196`); names are `UNIQUE (project_id, name)`
  (`:194`). `repo_box_path` holds the per-box path (`:202-208`).
- **Isolation is chosen per step, applied per scope, recorded per repo.** `GixIsolator::prepare`
  dispatches the whole scope on the step's one mode (`real.rs:1429-1458`), while each `run_step_tree`
  row already records its own repo's `mode` (`crates/htui-core/src/model/run.rs:452-466`). The default
  mode is `app_setting default_isolation` (`worktree`, `0003_orchestration.sql:130`), overridable in
  `project.settings.default_isolation` (`0001_init.sql:150-151`) and per phase.
- **A plain folder fails in every isolation mode today, `local` included.** Every mode reads HEAD
  through `head_of` → `git::open` (`crates/htui-orch/src/isolate/real.rs:428-438`; `isolate/git.rs:1396-1410`);
  `copy` refuses a source without `.git` as `not_a_git_checkout` (`isolate/copy.rs:250-273`). Registration
  checks only that the path is an existing directory (`crates/htui-core/src/root_path.rs:43-65`).
- **Git-only pieces.** Worktree prepare (`real.rs:720-853`); copy's reset and label (`real.rs:924-970`);
  capture by HEAD (`real.rs:973-1082`); reconcile by `git merge --no-ff` (`real.rs:1127-1209`);
  `run_step_commit.before_hash TEXT NOT NULL` (`0001_init.sql:501-507`) and `run_step_tree.base_ref
  TEXT NOT NULL` (`0003_orchestration.sql:92-100`). Diff and changed paths already degrade to nothing
  without git (`real.rs:1492-1568`). `infer` finds only `.git` directories (`crates/htui-orch/src/infer.rs:290-307`).
- **Repo-less projects mostly work already.** An empty `repo_scope` overlaps nothing (`overlap.rs:38-90`;
  `pg/write.rs` `claim_run`'s `repo_scope && $2`), `local` and `shared_serialized` run in the step's
  session folder, and verify reports "no primary tree" as unavailable (`engine.rs:3837-3872`). Three gaps:
  `worktree`/`copy` probe for a git binary even with no repos (`real.rs` `prepare_worktree`,
  `prepare_copy` call `cli()` first); an empty tree batch persists no `isolation_path`
  (`pg/write.rs:5717-5740`); and promote-to-chat fails "the step has no tree to chat in"
  (`engine.rs:1413-1418`, `chat_dirs` `:7463-7478`; the fixture comment at
  `crates/htui/src/run_worker.rs:276-277` admits it). A repo-less project is also never found by cwd,
  which maps only through `repo_box_path` (`crates/htui/src/state_cmd.rs:261-325`).
- **No phase knows it needs a repo.** The seeds set no such flag and `isolation: None` (→ the project
  default `worktree`, `0003_orchestration.sql:130`; `crates/htui-core/src/seed.rs:52-160`).
- **The primary.** `create_repo` demotes the old primary when the new one is primary, but a first repo
  created with `is_primary=false` leaves none (`pg/write.rs:2532-2575`); `update_repo` with `Some(false)`
  simply unsets it (`write.rs:2595-2655`); the Settings form lets the person type `n`
  (`crates/htui/src/ui/tabs/settings/hierarchy.rs:683-693`). **No repo delete exists** anywhere, and
  `run_step_tree.repo_id` and `run_step_commit.repo_id` reference `repo` without `ON DELETE`, so deleting
  a repo with history would fail. Readers of the primary: bare `touched_paths` globs
  (`overlap.rs:44`; `crates/htui-core/src/model/item_spec.rs:365-370`), excerpts, the in-place session
  cwd (`real.rs:694-705`), verify's cwd (`engine.rs:3852-3859`), the chat cwd, `isolation_path`, and
  ANA-34's `local` integration "into the primary's checked-out branch". None of them needs git.
- **MOD-106** (`HANDOFF.md:628-637`): what remains after the review is "promote another repo when the
  primary is removed, and make 'repos but no primary' unreachable. Removing the last repo is allowed.
  ANA-35 owns the wider repo-less model, so land this after it or with it."

### 2.4 Workspaces, ownership and keys

- **A workspace owns nothing but membership today.** `workspace` has slug, name, description,
  `created_by` (`0001_init.sql:129-137`); `workspace_project` is keyed `(workspace_id, project_id)`, so
  **a project may sit in several workspaces** (`:161-167`); `workspace_box_path(workspace_id, box_id,
  root_path NOT NULL)` is the per-box root (`:173-179`). The only functional reader of that root is
  checkout inference (`crates/htui/src/hierarchy.rs` `infer`); about ten files reference the table; it is
  not mirrored.
- **Every owned table is project-keyed and NOT NULL.** `item.project_id`, `item_kind.project_id` with
  `default_graph_id NOT NULL → step_graph`, `step_graph.project_id`, `prompt_template.project_id`,
  `item_key_counter (project_id, prefix)`, `run.project_id` (`0001_init.sql:214-222, 282-334, 447-466`),
  the requirement tables (`0006_requirements.sql:34-127`), the cache cursor `(project_id, table_name)`
  (`cache_migrations/0001_mirror.sql:29-31`), the Qdrant tenant `project_id` (`vector.rs:641-684`), and
  `item_by_key(project, key)` (`traits.rs:2053`). Link and upstream reads inner-join `project` for the
  slug (`pg/read.rs:171-205, 576`), so a row with a NULL project would vanish from them.
- **The counter.** `item_key_counter` is keyed by prefix, not kind, and minted in one CTE (`INSERT … ON
  CONFLICT DO UPDATE … RETURNING` → item → revision), so a refused insert burns no number
  (`pg/write.rs:1013-1091`). `workspace.slug` and `project.slug` are unique separately, so `slug:KEY`
  can be ambiguous across the two (`0001_init.sql:129-131, 143-145`).
- **Bindings are per project.** The secret binding is the project's `secret_provider`/`secret_scope`
  columns (`0001_init.sql:147-148`; `R-SEC-1` is per project). No workspace-level secret, skill or
  target binding exists: workspace skill levels are MOD-113 (2) (`HANDOFF.md:697-705`) and a workspace
  `target_id` is MOD-130 (`:389-396`).
- **Workspace-wide features assume a workspace** that a lone project does not have: the compose file
  and `.env` live "in the workspace folder" (`R-ENT-17`, `R-SEC-5`); `R-ORCH-16` stops runs per workspace.
  No compose or `~/workspaces` code exists yet (MOD-116 `HANDOFF.md:745-754`, MOD-117 `:755-764`).
- **ANA-34 already needs a workspace owner for runs:** "`run` gains a workspace scope (nullable
  `project_id` with a `workspace_id` and a check that exactly one is set, or an equivalent the plan
  chooses)" (`docs/ANA-34.md` §7.2), carried by MOD-136 (`HANDOFF.md:431-438`).

### 2.5 Documents, links, attachments and search

- **Documents are item-owned only.** `document(item_id NOT NULL → item CASCADE, kind open set, version,
  title, body, produced_by_step_id, created_by, created_at, UNIQUE (item_id, kind, version))`,
  append-only (`0001_init.sql:389-400`); the model types make `item_id` non-optional
  (`crates/htui-core/src/model/document.rs:11-90`); version allocation locks the item row
  (`pg/write.rs:365-412`); the cache mirrors documents through a join on item (`cache/refresh.rs:1057-1096`).
  The MCP `document_write` writes only the step's own item and output kind (`crates/htui-mcp/src/tools/document.rs:26-69`).
- **Links.** `item_link(from_item_id, to_item_id, kind CHECK IN ('blocked_by','origin','relates','supersedes'),
  proposed_by_step_id, deleted_at, PK (from, to, kind), CHECK (from <> to))` (`0001_init.sql:357-368`);
  `LinkKind` has four variants (`crates/htui-core/src/model/link.rs:10-23`). The single production
  writer is `propose_link` under a step fence, which refuses a target outside the step's project
  (`pg/write.rs:7111-7171`); the MCP tool resolves `to` as a key within the session's project
  (`crates/htui-mcp/src/tools/link.rs:82-85`). The upstream prompt walk follows `blocked_by` and
  `origin` only (`pg/read.rs:559-569`). Citations to requirements live in their own table,
  `item_requirement` (`0006_requirements.sql:115-127`), which is the in-tree precedent for keeping
  non-item edges apart.
- **Attachments do not exist.** No `bytea` column in any migration; the "attachments" of
  `0007_skill_attachments.sql` are skill bindings, and the Rust name `Attachment` is taken by them
  (`crates/htui-core/src/model/skill.rs:246`). Size caps today are `app_setting` integers
  (`excerpt_max_file_bytes`, `copy_max_total_bytes`) or per-transport constants (MCP notes 16 KiB,
  `crates/htui-mcp/src/tools/note.rs:16`). Notes are scrubbed and fail closed (`note.rs:44`).
- **Requirements.** Project-owned: `requirement_spec (project_id PK)`, `requirement_area UNIQUE
  (project_id, code)`, `requirement_key_counter (area_id PK)`, `requirement` with a generated key
  `'R-' || area_code || '-' || number` and `UNIQUE (project_id, area_code, number)`, with no unique on
  `key` itself and no refines column (`0006_requirements.sql:34-109`; key at `:76`, unique at `:86`). Cross-project citation is legal by design (`docs/ANA-11.md:233-235`).
- **Search.** One collection `htui_concepts_v2`, tenant `project_id`, point types item, document and
  requirement; document points belong to items only: `Owner` is `Item | Requirement` and a
  `ConceptPoint`'s document is "set on document points, which only an item has" (`vector.rs:35-39,
  81-110, 192-197, 243`); the indexer walks `scope.project_ids` and reaches documents only through each
  item's `latest_documents` (`vector_sync.rs:63-131`, the item loop at `:88-110`), so a document with no
  item would never be indexed. Any payload change takes a new collection name by
  convention (`vector.rs:35-39`).

### 2.6 Project types and seeding

- **No type anywhere.** `project` has no `type` column (`0001_init.sql:143-155`); `NewProject`, the
  Settings form and `StoreRequest::CreateProject` carry slug, name and description only
  (`crates/htui-core/src/model/hierarchy.rs:86-97`).
- **One fixed seed in code.** `seed.rs:52-160` `KINDS` (ANA, FEAT, FIX, CLEAN, TOOL), 15 phases, three
  hard gates; `seed_project` writes graphs, phases, kinds and the ten `DEFAULT_TEMPLATES` in the
  project's creation transaction (`pg/write.rs:2404-2444, 7560-7650`; MemStore `mem.rs:2996-3060`;
  `crates/htui-core/src/prompt/defaults.rs:219-230`). No template body exists for outline, draft,
  execute, verify, question, answer, investigate or postmortem. The demo fixture assumes a constant
  row count per project for its id strides (`crates/htui-core/src/fixtures.rs:753-772`).
- **Readers waiting for a type.** ANA-39 asks "what a coding session is (the project type, phase or
  kind)" (`HANDOFF.md:158-166`); MOD-128 is "default-on for coding projects" (`HANDOFF.md:844`);
  `R-TUI-8` lists "project types" as Settings data (`:588-589`). This repository's own prefixes (ANA, MOD,
  CLEAN, TOOL, NEXT, VAL) match no seed; `R-LATER-3` maps prefixes to kinds per project (`:681-683`).

### 2.7 Migration mechanics

- Two embedded sets: Postgres `0001..0018` (`MIGRATOR`, `crates/htui-store/src/lib.rs:93`) and the
  SQLite mirror `0001..0005` (`CACHE_MIGRATOR`, `lib.rs:100`). Forward-only; `applied_state` refuses
  dirty, newer and drifted versions (`pg/mod.rs:897-935`); `apply_migrations` runs `MIGRATOR.run`, then
  `bootstrap` (the code seeds), then raises `htui_target_version` so an older headless binary refuses
  the new schema (MOD-40; `pg/mod.rs:398-403`). A worker never migrates.
- No Postgres ENUM type exists: every closed set is `TEXT` plus `CHECK`, pinned to its Rust `str_enum!`
  by unit tests (`crates/htui-core/src/model/mod.rs:271, 288, 296, 420`). Narrowing is a constraint swap.
- The mirror has no CHECKs or FKs and is rebuilt from scratch on any `schema_version` change
  (`cache/mod.rs:142-152`).
- The backfill test pattern is `run_to(N-1)` → plant rows → `run_to(N)` → assert, with the
  `updated_at` trigger disabled around the UPDATE (`crates/htui-store/tests/migrations.rs:1366-1437`).
- `.sqlx` offline entries hash the literal SQL; each changed query needs `cargo sqlx prepare` against a
  migrated scratch database. Postgres 16 is the floor (`UNIQUE NULLS NOT DISTINCT`, `0001_init.sql:429`).

## 3. Constraints from sibling items

1. **ANA-2 invariants.** Every status move is a compare-and-set with one writer; no agent in a
   bookkeeping path; htui writes no files into a managed repo (`R-ID-4`); stored `blocked` means "a human
   must clear this" and always carries an `item_note`, while dependency-blocked stays derived
   (`docs/decisions/ana/ana-2.md:36-48`). Status moves never bump `version` (`CONCEPTS.md:32-34`).
2. **ANA-11 / MOD-38.** Resolution is orthogonal to status and non-null iff `closed`; `closed` is
   reachable only through `close_out` (`docs/decisions/mod/mod-38.md:57-69`). Whatever replaces
   `closes_from` must keep both.
3. **ANA-34 (concluded).** Close-out is a person's confirmation over a checklist, never automatic, and
   resolution `done` requires every integration record `landed` or `pr_merged` (§7.6); a repo-less or
   plain-folder project, or a repo without a remote, accepts `local` integration only (§7.4); `run` needs
   a workspace owner (§7.2); `R-ENT-8`'s transition sentence is replaced (§10.4). MOD-135 must
   "coordinate with ANA-35's status migration" (`HANDOFF.md:424-430`).
4. **ANA-33 (unwritten)** owns the graph model, the snapshot and the Settings graph editor. A type's seed
   fills that model, so the seed's storage format must survive ANA-33 changing it. ANA-40 placed
   workflows beside kinds' graphs (`R-ORCH-1`).
5. **ANA-42 (drafted, §8 open)** prepares a driven target by cloning and collects by `git bundle`
   (`docs/ANA-42.md` §7.5-§7.6; MOD-132 `HANDOFF.md:404-408`): both need git. MOD-130 adds `target_id` at
   workspace, project, workflow and item (`HANDOFF.md:389-396`), the same tables this item migrates.
6. **MOD-50** keys the decisions filter on resolution and keeps `SearchQuery.statuses` for MOD-11
   (`docs/decisions/mod/mod-50.md:22-28`); MOD-11 reads status and links under `R-ENT-8`/`R-ENT-9`
   (`docs/decisions/mod/mod-11.md:3-5`).
7. **MOD-40** makes one binary version per schema version: an older binary refuses a newer schema, so a
   single migration may change data and narrow a constraint together as long as the binary that applies
   it already writes only the new values.
8. **Waiting items.** MOD-112 (4) wants workspace-scoped search "once ANA-35 lands"
   (`HANDOFF.md:689-696`); MOD-114 (3) attachments: "who may own one waits on ANA-35" (`:706-715`); MOD-8
   "should follow ANA-35, whose status, link and ownership model it imports into" (`:716-724`); MOD-116
   "the folder needs `R-ENT-2`'s amendment in ANA-35's schema" (`:745-754`); MOD-118 waits "on ANA-35 for
   workspace-owned requirements and their keys" (`:765-773`); MOD-119 groups by "status in attention
   order" (`:774-782`); MOD-107 adds labels to the same `item` table (`:638-646`); MOD-133 enforces
   "`local` only" for folders and needs the repo kind (`:409-416`).

## 4. Prior art

| System | Item state vs run state | Group-level items and keys | Links | Attachments | Without git | Source |
|---|---|---|---|---|---|---|
| GitLab | Custom statuses in 5 fixed categories; "Done and Canceled … set work items to closed state"; a status in use cannot cross open/closed | Group work items with a group-scoped number; `internal_ids` keyed by (usage, project or namespace) | relates, blocks; parent/child is a separate hierarchy | uploads, 100 MiB admin cap | n/a | https://docs.gitlab.com/user/work_items/status/ ; https://docs.gitlab.com/user/project/issues/related_issues/ |
| Jira | Statuses in 3 fixed categories; a separate Resolution (Done, Won't do, Duplicate) | No workspace items; initiatives live in a dedicated project | blocks, clones, duplicates, **implements**, relates…; Confluence pages through separate remote links | media store, 1 GB | n/a | https://support.atlassian.com/jira-cloud-administration/docs/what-are-issue-statuses-priorities-and-resolutions/ ; https://support.atlassian.com/jira-software-cloud/docs/link-issues/ |
| GitHub | `open`/`closed` plus `state_reason` (completed, not_planned, duplicate); Copilot's progress lives on the session and draft PR, not the issue | Issue types per org; numbers per repo only; org drafts have no number | sub-issues (≤8 levels, one parent), blocked-by | n/a | n/a | https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/adding-sub-issues ; https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/use-cloud-agent-on-github |
| Linear | Per-team statuses in fixed types; agent sessions carry `pending, active, error, awaitingInput, complete` | Team key + per-team counter; a moved issue gets a new id, the old resolves | blocks, related, duplicate (moves the issue to a reserved Duplicate status), parent | n/a | n/a | https://linear.app/developers/agent-interaction ; https://linear.app/docs/conceptual-model |
| Plane | State groups (backlog, unstarted, started, completed, cancelled) drive behaviour | Workspace pages and catalog states (`project_id = null`) | n/a | S3, 5 MB default | n/a | https://developers.plane.so/api-reference/v2/project-templates/overview |
| YouTrack / OpenProject | Per-state "resolved"/"is closed" flag | Templates seed fields, states and workflows; any project can become a template | Subtask: one parent; Duplicate, Depend, Relates | n/a | n/a | https://www.jetbrains.com/help/youtrack/server/link-types.html ; https://www.openproject.org/docs/system-admin-guide/manage-work-packages/work-package-status/ |
| Devin, OpenHands, GitHub Actions | `waiting_for_approval`, `waiting_for_confirmation`, `waiting` are **session/run** states | n/a | n/a | n/a | n/a | https://docs.github.com/en/rest/actions/workflow-runs |
| Taskwarrior, Beads | Blocked and ready are **derived** from dependencies, not stored | n/a | `blocks` alone drives readiness | n/a | n/a | https://taskwarrior.org/docs/tags/ |
| Aider, Codex CLI, Claude Code | n/a | n/a | n/a | n/a | Aider `--no-git` loses undo, diff, auto-commit; Codex refuses outside a repo by default; Claude Code works but `--worktree` needs git | https://aider.chat/docs/git.html ; https://code.claude.com/docs/en/checkpointing |
| Redmine, Gitea | n/a | n/a | n/a | disk, 5 MiB / up to 100 MB defaults; Redmine dedups by SHA-256 | n/a | https://www.redmine.org/projects/redmine/wiki/RedmineSettings |

Postgres facts for attachments: a `bytea` value tops out at 1 GB and is TOASTed out of line; large
objects do not cascade with their row and need `lo_manage` and `vacuumlo`
(https://www.postgresql.org/docs/current/lo.html); a benchmark puts `bytea` at about 6 ms for 4.5 MB
against 1 ms on a filesystem and advises `EXTERNAL` storage for compressed data
(https://www.cybertec-postgresql.com/en/binary-data-performance-in-postgresql/); sqlx maps `Vec<u8>` to
`bytea` and reads the whole value into memory (https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html).

Lessons that bind:

- **Every surveyed tracker separates a small fixed set the code branches on from the names a template
  seeds**, and every agent harness keeps "approval wait, running, queued, failed" on the attempt, not the
  task. `open | blocked | closed` plus a run-derived display state is the established shape. Run-to-item
  coupling elsewhere is an event-driven projection with known misfires (several PRs per item, double
  triggers); the derived read avoids the projection altogether.
- **Storing `blocked` is the minority choice**; most tools derive it from `blocks`. htui already keeps
  both: a stored `blocked` that a person clears and a derived "not ready". The verdict keeps that split
  and states it.
- **"Why closed" is a separate field everywhere**, and duplicates carry side effects (Linear's reserved
  status, GitHub's `duplicate_issue_id`). A duplicate close-out should name its canonical item.
- **Parent is single-valued** in YouTrack and GitHub, with a depth cap in GitHub. Links to documents are
  kept apart from item links in Jira (remote links), as htui keeps citations apart.
- **Group-level keys need their own counter scope** (GitLab's `internal_ids` keyed by project or
  namespace); collisions between scopes are handled by qualification, not prevention, everywhere they
  exist.
- **Templates are copied, not linked** (Plane's "use" remaps template ids onto new rows; OpenProject
  copies a template project). Editing a template never rewrites projects made from it.
- **Without git, harnesses lose the same things**: worktree isolation, diff and undo, commit-based
  integration and "is there uncommitted work" checks. Editing in place and running commands still work.
- **Attachments: `bytea`, not large objects**, under a cap well below 1 GB, with metadata apart from
  bytes so lists never touch TOAST. Surveyed defaults run from 5 MB to 100 MB.

## 5. Options per question

### 5.1 Q-a: project types seeding kinds and step graphs

**In plain terms.** Today every new project gets the five software kinds. A research project created
tomorrow would get `FEAT` and `FIX`, whose graphs run `implement` in a git worktree it does not have. A
type lets the person pick "research" and get `RES` (research → verdict) and `Q` (question → answer)
instead. The question is whether the type is just a choice at creation or a lasting attribute, and where
the seed recipes live.

- **A. A seed parameter only.** `create_project(type)` picks a code constant and the type is forgotten.
  Smallest; but ANA-39 and MOD-128 need to know later whether a project is a coding one, and Settings
  (`R-TUI-8`) has nothing to list.
- **B. A `type` column with a closed CHECK, seeds in code.** Persistent and cheap; but "and more" and
  `R-TUI-8`'s editable types become a migration per new type.
- **C. A `project_type` table; `project.type` references it.** Built-in rows carry no recipe (code
  supplies it, so ANA-33 can change the graph format freely); user-defined rows, later, carry a JSONB
  recipe. A `coding` flag answers ANA-39's question at the project level. Seeding copies once.

For the seeds, two sub-options: **(i)** the review's proposals (RES, Q, DOC, OPS, INC) plus today's
software set; **(ii)** the same plus a **general** type (`TASK`: do → review) for "any task an agent
can do" that fits none of the four.

### 5.2 Q-b: repo-less projects and plain-folder repos; what refuses without git

**In plain terms.** A docs project might point at `~/notes` (a folder, no `.git`); a research project
might point at nothing. Today both fail as soon as a step starts: every isolation mode reads HEAD. The
question is how the store knows a folder from a repo, and what each feature does with one.

- **A. Probe at run time.** No schema change; each step checks for `.git`. But the refusal comes late
  (mid-run), Settings cannot show the kind, and a repo whose `.git` was deleted silently changes kind.
- **B. A declared kind, `repo.vcs IN ('git','folder')`, checked at registration.** The snapshot can
  refuse a git-only mode before any agent starts; `default_branch` and `remote_url` become git-only.
- **C. Wrap folders in a shadow git repository** (Gemini CLI, Cline) so every git path keeps working.
  Gives undo and diff for free; but it writes a repository into or beside a managed folder (`R-ID-4`'s
  spirit), and a folder that later becomes a repo hits the known `.git`-in-snapshot trap.

For "a phase that needs a repo" (`R-ENT-3`): **(i)** derive it from the isolation mode; **(ii)** an
explicit `needs_repo` flag on the phase, seeded per type; **(iii)** derive it from the phase name. And for
`copy` on a folder, how changes come back: **(a)** refuse `copy` on folders after all; **(b)** sync back
files unchanged in the folder since the copy and park on any conflict.

### 5.3 Q-c: workspaces owning items, documents and requirements; key space, kinds, one counter

**In plain terms.** A workspace "web shop" groups `api` and `frontend`. A cross-project analysis ("which
auth library do both use?") belongs to neither. `R-ENT-16` lets the workspace own it, as `WS-1`, with its
own kinds and counter. Every table that holds items today says `project_id NOT NULL`, and so do the
counter, the graphs, the cache cursor and the search tenant.

- **K1. Owner columns.** Add `workspace_id` beside `project_id` on `item`, `item_kind`,
  `item_key_counter`, `step_graph`, `prompt_template`, `run`, `document` and the requirement tables, with
  `CHECK (num_nonnulls(project_id, workspace_id) = 1)` and uniques rewritten `NULLS NOT DISTINCT`. Honest
  in the schema; but every query that filters or joins on `project_id` (about 60 item sites across Pg,
  cache and MemStore, plus link and upstream inner joins that would silently drop workspace rows) and the
  cache cursor, the Qdrant tenant and `item_by_key` must learn a second owner.
- **K2. A hidden scope row per workspace in `project`.** `project.owner_workspace_id UNIQUE REFERENCES
  workspace ON DELETE CASCADE` marks the row that owns the workspace's items, kinds, counter, graphs,
  templates, documents, requirements and runs. Every per-project path works unchanged; the cost is
  keeping the row out of project lists and membership, and a reserved slug.
- **K3. A separate `owner`/key-space table** that `item`, `item_kind` and the counter reference instead
  of `project_id`. The cleanest model on paper; the same query audit as K1 plus a data migration of
  every item row.

On key collisions (`ANA-7` in a member project and `ANA-7` in the workspace): **(i)** refuse a workspace
prefix that equals a member's, at kind creation; **(ii)** qualify keys when ambiguous (`slug:KEY`);
**(iii)** a reserved workspace prefix grammar.

On the folder (`R-ENT-2`): **(a)** a column on `workspace`; **(b)** per box, repurposing
`workspace_box_path`. And on description and goals: **(1)** in `.htui/workspace.toml` in the folder, as
the redesign preview shows (`docs/ui-redesign-findings-2026-10-15.md:186-196`); **(2)** in Postgres.

### 5.4 Q-d: documents owned by a project or a workspace

**In plain terms.** Today a document always hangs off an item: a PRD for `FEAT-3`, a verdict for
`ANA-7`. A project's "architecture overview" or a workspace's "goals" has no item. The question is how a
document names an owner that is not an item.

- **D1. Exclusive arc.** `item_id` nullable, `project_id` and `workspace_id` added, `num_nonnulls = 1`.
- **D2. `project_id` on every row, `item_id` nullable.** An item document carries both (copied from
  the item); a project or workspace document carries only `project_id` (with K2, the workspace's scope
  row). The cache refresh, search scoping and delete reach become plain column filters.
- **D3. Separate `project_document` tables.** The item path is untouched; every list, search and link
  target becomes a union.

### 5.5 Q-e: attachments in Postgres under a size cap

**In plain terms.** A person drops a screenshot of a broken page onto `FIX-12`, or a run's log onto its
review document. `R-ENT-12` wants these in Postgres for now. Questions: who may own one, how big, how
stored, and how they meet `R-ID-7`, which says everything persisted is scrubbed of secrets and scrubbing
fails closed. An image cannot be scrubbed.

- **Storage. A1** one table with a `bytea` column; **A2** metadata in `attachment` and bytes in
  `attachment_blob`, so lists and cache refreshes never touch the bytes, and `R-LATER-10`'s object store
  later replaces only the blob table; **A3** large objects (no cascade, `vacuumlo`, no sqlx API).
- **Owners.** **(i)** item or document only, as `R-ENT-12` says; **(ii)** also notes, projects and
  workspaces. For a document: the document **version** it was added to, or the (owner, kind) family.
- **Cap.** 5 MiB (Plane, Redmine), 10 MiB, or 100 MiB (Gitea, GitLab); a soft `app_setting` cap plus a
  hard DDL ceiling.
- **Scrubbing.** **(a)** refuse anything the scrubber cannot read, which refuses images; **(b)** scrub
  text and accept binaries from anyone; **(c)** scrub text, fail closed, and accept a binary only from a
  person who confirms it is stored as given, with `R-ID-7` amended to say so.

### 5.6 Q-f: link kinds `parent`, `duplicates`, `implements`; links to documents or attachments

**In plain terms.** `parent` lets a workspace item `WS-4` "Add SSO" own `api` item `FEAT-9` and
`frontend` item `FEAT-14` as its parts. `duplicates` says `FIX-20` is the same bug as `FIX-17`.
`implements` says `FEAT-9` carries out the verdict of `ANA-35`. And a link to a document lets `FEAT-9`
point at the exact version of a spec it was built against.

- **Kinds.** Widen the CHECK either way. For `parent`: **(i)** free many-to-many; **(ii)** single-valued
  on the child with a cycle check. For `duplicates`/`supersedes` and the resolutions of the same name:
  **(i)** independent; **(ii)** closing as `duplicate` or `superseded` requires the link to exist; **(iii)**
  close-out records the link from the person's choice.
- **Non-item targets. L1** make `item_link.to_item_id` nullable and add `to_document_id` and
  `to_attachment_id` (an exclusive arc; every reader of item links must learn to skip non-items); **L2** a
  separate `item_artifact_link` table, as citations are kept apart today; **L3** a polymorphic
  `(to_type, to_id)` pair without foreign keys. And for a document target: pin a **version** (a document
  row) or follow the latest of (owner, kind).

### 5.7 Q-g: item status reduced to open / blocked / closed

**In plain terms.** Today `FEAT-3` says `in_progress` while its run runs, `awaiting_approval` while the
run waits at a gate, `failed` when it fails and `done` when it finishes. After the change, `FEAT-3` says
`open` throughout, and the run says what is happening. The person must still see "running", "needs you"
and "failed" in the Backlog, so those words have to come from somewhere. Underneath, four things lean
on the old values: the lock against two runs on one item, the auto-queue's "ready" filter, the
close-out law, and the readiness of items that wait on this one.

- **Display.** **(i)** show only the three statuses; **(ii)** derive a display state from the item and
  its latest graph run, in one pure function used by every surface.
- **What the removed values become.** `queued`, `in_progress`, `awaiting_approval` → `open` (the live
  run holds them). `failed` → `open` with its failed run, or `blocked`. `done` → **(a)** `closed` with
  resolution `done` (or `concluded`), closing every finished item in the migration; **(b)** `open`, with a
  finished latest run, waiting for a person's close-out.
- **When a blocker stops blocking.** **(a)** only when `closed`: dependents wait for a person's close-out
  of every blocker, which stalls auto mode between dependent items; **(b)** when `closed`, or when its
  latest graph run finished `done` and its work landed: exactly today's `done | closed` rule, now derived.
- **The one-live-run guard.** **(a)** check "no live graph run" in code under the item lock; **(b)** a
  partial unique index `run(item_id) WHERE kind = 'graph' AND status IN (live)`; **(c)** both.
- **Migration shape.** **(a)** one migration per spawn with expand, migrate and contract sections, relying
  on MOD-40's version guard; **(b)** separate releases with dual writes, as zero-downtime systems do.

### 5.8 Q-h: MOD-106's primary repo

**In plain terms.** The primary repo is where a bare path like `src/**` points and where in-place steps
and verify run. A project with two repos and no primary has bare paths that mean nothing; that state is
reachable today by typing `n` in a form. Nothing removes a repo yet, so "promote another when the primary
goes" has no trigger.

- **Folder as primary.** **(i)** allowed; **(ii)** only a git repo may be primary.
- **Enforcement.** **(a)** at the store write only (MemStore and PgStore alike); **(b)** a deferred
  constraint trigger only; **(c)** both, the trigger as a backstop.
- **Removal.** **(a)** hard delete (fails on run history: `run_step_tree` and `run_step_commit` reference
  `repo` without `ON DELETE`); **(b)** archive (`archived_at`), keeping history; the primary passes to a
  named successor or the oldest live repo.

## 6. Analysis

**Three stored statuses work because the run already says everything else.** Every removed value has
an exact twin on `run.status` (§2.2), and the places that need progress (the queue overlay, close-out,
waiting rows, the unblock verdict) already read the latest graph run through the `LATERAL` pattern of
`queue_rows`. The item copies are the problem, not the solution: they cost seven engine transitions that
do nothing but mirror a run move, three store mirrors and the E-2 crash window in `cancel_queued`, and each run move re-embeds the item in
Qdrant because `status` is a staleness key. Deriving the display state from (status, latest graph run)
yields exactly the redesign's eight words in its attention order: `closed`; `blocked`; for an open item,
`queued`, `running` or `needs you` from a live run, else `failed` or `done` from the last one, else
`open`. So 5.7 (ii) costs a function and an appended column, and gives the Backlog, the glyph table, the
prompt and MCP one vocabulary.

**The migration must not close anything a run can explain.** A `done` item today is one whose run finished and which no
person has closed. ANA-34 §7.6 made close-out a person's confirmation with a resolution they choose
(`done` for a feature, `concluded` for an analysis), and auto mode never closes. Closing every `done`
row in the migration would make that choice for the person, in a bookkeeping path. Mapping it to `open`
loses nothing: its finished run still says "done", the derived state shows it, and MOD-135's `CloseOut`
waiting row asks for the confirmation. `failed` maps to `open` for the same reason: the failed run is the
fact, and `blocked` stays what ANA-2 made it, "a human must clear this, with a note".

**Rows a run cannot explain need their own mapping.** That argument holds only when a run carries the
fact. A `done` item whose latest graph run is not `done` (none, or a different one: written by MOD-2's
chat path, a pre-MOD-25 offline upload, a demo fixture or a `transition` outside ANA-2 §4.3,
`traits.rs:2833-2836`) would come out `open` with nothing finished: auto mode would re-run finished
work, its dependents would stop being released, and the new law would refuse `done` at close-out. Three
mappings were weighed. `open` has all three faults. `blocked` with a note stops the re-run but still
holds back its dependents, and leaves the person no way to record `done`, since `blocked` closes only
as `rejected`, `withdrawn`, `superseded` or `duplicate`. Closing it `done` with a note keeps all three
of today's behaviours (not picked again, releases its dependents, `default_for(Done)` is `done` today)
and is irreversible, but it touches only rows whose one fact is a stored `done` that no run backs, and
the note says so. A `failed` item without a failed run is safer: `blocked` with a note keeps it out of auto mode
(it is not ready today either) until a person clears it, and keeps it holding its dependents, as today.
A count before the migration tells the maintainer how many rows each rule touches.

**Readiness must keep today's meaning or auto mode stalls.** Today a blocker releases its dependents
the moment its run is `done`, before close-out. Releasing only on `closed` would put a person between
every pair of dependent items in auto mode, against `R-ORCH-6`. Defining "finished" as "latest graph run
`done` and its integration landed" reproduces today's rule exactly while MOD-133 is unbuilt (every run
lands locally) and becomes stricter in the right way after it: an open pull request does not release
code that dependents need. A blocker closed as `rejected` or `withdrawn` still releases them, as today;
the dependent's own run then decides whether it can proceed. Readiness itself becomes "status `open`,
no live graph run, latest graph run neither `failed` nor `done`, every blocker released", which is the
set `status = 'open'` selects today. "Latest graph run" skips `cancelled` runs: a cancelled run changed
nothing, so cancelling a rework run returns the item to the state its last real run left. Rework of a
finished item, which today is the `done → open` edge only tests take (§2.2), becomes a new run started by
a person; while it is live the blocker is not released (dependents not yet started wait; running ones
are not stopped), and if it is cancelled the earlier `done` releases them again. The landed condition
needs MOD-133's integration records, which do not exist yet; P1 implements "latest graph run `done`" and
MOD-133 adds the landed check to every readiness site (Pg, MemStore, and the cache, which must mirror the
records).

**The lock moves to the database.** `create_run`'s item-status check is today's only guard against two
live runs on one item. A partial unique index on `run(item_id)` for live graph runs makes the guard
declarative, keeps working when the item's status no longer changes, and catches MemStore/PgStore
divergence in the conformance suite. It is scoped to `kind = 'graph'`: chat runs carry no item today
(§2.2), but the finish path's live-run count does not filter on kind, and MOD-136 adds a `coordinator`
kind that must not collide with an item's graph run. The code check under the item lock stays, so the
refusal sentence stays readable.

**The close-out law needs a new anchor, and ANA-34 already supplied it.** `closes_from` keyed on
`done`, `failed` and `blocked`. The replacement keys on facts that survive: no live run (already
checked), and for `done`/`concluded` a latest graph run that finished `done` with its integration landed.
For a repo-less or folder project the integration condition is vacuous, which is right: there is nothing
to land. `default_for` can then do better than today: `concluded` when the finished run's graph ends in
a phase whose output kind is `verdict`, `done` otherwise, `withdrawn` when nothing finished. It reads the
run's own `graph_snapshot` (its last phase's `output_kind`, `crates/htui-core/src/model/run.rs:546-566`),
not the kind's default graph, because a run may use the item's `step_graph_id` override
(`0001_init.sql:324`) or a picked workflow (MOD-126). That keeps the person's picker honest for research
and docs projects without any model in the path.

**A declared repo kind is the only option that refuses early.** With `repo.vcs` the snapshot can
resolve or refuse each repo's mode before an agent starts, Settings can show the kind,
MOD-133 can enforce "`local` only", and registration can check the path matches. The shadow-git option
(5.2 C) buys undo and diff at the price of a repository htui writes beside a managed folder; htui has no
diff view yet (MOD-3 is later tier), so the benefit is small now. `copy` on a folder is worth keeping
(`R-ENT-4` says it works): the copy itself is plain file I/O; only reconcile is git. A digest manifest
taken at prepare lets reconcile copy back files the folder has not changed since and park on the rest
with a `folder_conflict` refusal, which is the same "never resolve, park and note" discipline as ANA-2
§4.6. Fan-out above one on a folder needs a base snapshot per part and is refused for now, as `local`
already refuses it.

**A folder in a `worktree` step should be copied, not refused.** Isolation is one mode per step applied
to the whole scope (`real.rs:1429-1458`), and the software type's default is `worktree`. Refusing every
git-only mode over a scope that holds a folder would refuse every `implement`, `fix` and `review` in a
software project whose primary is a folder, and in any project mixing a git repo with a folder (code
plus a notes folder), unless the person rewrites every phase's mode. Resolving the mode per repo at
snapshot avoids that: a git repo takes the step's mode; a folder under `worktree` takes `copy`, the other
isolating mode, so `R-ORCH-9`'s "both use isolated trees" overlap rule and the session's private tree
stay true; a folder under `copy` or `local` takes it as set. `shared_serialized` over a folder still
refuses, naming the folder: its label and reset are git, and falling back to `local` would silently drop
the serialization the person asked for. The tree row already records each repo's own mode
(`RunStepTree.mode`), so capture and reconcile can follow it per repo; only `prepare`'s dispatch has to
partition the scope. With this rule the default isolation does not change when a folder joins a
software project: it stays `worktree`, and the folder is copied.

**"Needs a repo" should be said, not guessed.** Isolation does not imply it: a repo-less step under
`worktree` simply runs in its session folder. Phase names do not imply it either: `review` of a document
needs no repo. An explicit `needs_repo` flag on the phase, seeded true for the software code phases
(`reproduce`, `implement`, `fix`, `review`) and false elsewhere, lets `snapshot_phase` refuse with a
sentence a person understands, at run creation, before anything starts.

**A folder can be the primary.** Every reader of the primary (bare globs, session cwd, verify cwd,
`isolation_path`, ANA-34's local integration target) needs a directory, not a history. Forbidding it
would make a docs project with one folder primary-less, which `R-ENT-3` forbids. Enforcement at the store
write is where refusals can be explained; a deferred constraint trigger catches any writer that forgets
(the demo loader, a future importer). Removal has to be an archive because run history references repos
without cascade; archiving is also what a person expects ("I stopped using this repo", not "erase what
it did").

**The workspace scope row (K2) is the cheapest correct model, and ANA-34 already allowed it.**
Everything a workspace must own already exists for projects: a key space with a counter and one writer,
kinds with default graphs, templates, documents, requirement areas and counters, a cache cursor, a
search tenant, a secret binding (the row's own `secret_provider`/`secret_scope`, §2.4), and since
ANA-34 a run owner. K1 would duplicate each of those paths
with a second owner column and audit every join; a missed join silently hides workspace rows rather than
failing. K2 reuses them, and its risk (a hidden row leaking into project lists) is one predicate, tested
once in conformance. It also answers ANA-34 §7.2's "or an equivalent the plan chooses" for MOD-136:
a workspace coordinator's runs belong to the workspace's scope row, with no nullable `run.project_id`.
It does not create an implicit workspace (`R-ENT-2` forbids those); it creates one owning row per
explicit workspace. Search is the one per-project path it does not get for free: the index reaches
documents only through items (§2.5), so owner documents need their own point and indexer pass.
`R-ENT-7`'s "exactly one counter" then reads naturally as one counter per owner and
prefix, with Postgres the only writer. A workspace item's runs touch no member repos in version one:
the scope row has no repos, so a workspace item plans, decides and splits, and its children in member
projects (`parent` links) do the code.

**Key collisions are a display problem.** A project can join a workspace after both have minted `ANA-7`,
so prevention cannot be complete. Refusing a colliding prefix when a workspace kind is created, warning
when a project with a colliding prefix joins, and qualifying keys wherever they are ambiguous (MCP, CLI,
the importer) is what GitLab and Linear do. The Backlog already shows workspace items in their own group
(`R-TUI-2`), so the screen is unambiguous. The qualifier must itself be unambiguous: `workspace.slug` and
`project.slug` are unique separately (§2.4), so `shop:WS-1` could name either. Giving the scope row a
reserved slug, `@<workspace-slug>`, in `project.slug`'s own unique namespace, makes the qualifier
`@shop:WS-1`, which no project slug can take, and lets `item_by_key(project, key)` resolve it unchanged.

**The workspace folder holds generated files only.** `R-ID-3` names the stores and the folder is not one.
Description and goals are domain data: the description already has a column, and goals fit a
workspace-owned document of kind `goals` (versioned, searchable, and a target for `implements`). The
folder is a per-box path (`R-BOX-4`), so `workspace_box_path` is repurposed, its `root_path` renamed
`folder_path`, with an absent row meaning the default `~/workspaces/<slug>` on that box. Inference
anchors on that folder for clones; repos elsewhere are registered by path, as `R-ENT-2` says ("existing
repos stay where they are"). A lone project has no folder; its components and env file need a workspace.

**Requirement keys follow the item model.** `R-ENT-14` already says "per owner and area"; the findings'
`R-42` without an area and a "shared `R-n`" across projects contradict it and were left open. With the
scope row, a workspace requirement is a requirement whose project is a scope row, minted from that row's
areas and counters; its key prefix is `W`. Refinement inside one owner keeps the `.n` form; a project
requirement refining a workspace one keeps its own project key, since key spaces are per owner, and
shows "↑ from W-…". Because a generated column cannot read another row, the key is stored at mint;
Postgres 13+ can turn the generated column into a plain one in place (`DROP EXPRESSION`, §11). A stored
key needs its own uniqueness, and a child needs a place in the existing `UNIQUE (project_id, area_code,
number)`: reusing the parent's `number` would violate it, and minting from the area counter would burn an
area number. So a nested child keeps the parent's area, leaves `number` NULL (Postgres treats NULLs as
distinct, so the existing unique still holds for top-level rows), takes a `child_number` from a counter
per parent, and `UNIQUE (project_id, key)` guards the stored key.

**D2 for documents follows from K2.** With a workspace's documents owned by its scope row, every
document has a `project_id`, and an item document also names its item. The cache refresh, search and
delete reach stop joining through `item`. A composite foreign key `(item_id, project_id) → item(id,
project_id)` keeps the copy honest. Search does not follow for free: the Qdrant `Owner` is `Item |
Requirement` and documents are reached only through items (§2.5), so an owner document needs a new point
owner and an indexer pass over the project's item-less documents. That is a payload change, and the
collection convention gives it a new name; P1 already moves to `v3` and lands first, so P4 takes `v4`
rather than making P1 carry points for documents that cannot exist until P4. The cost is one more full
rebuild from Postgres.

**Attachments: small, apart, and honest about scrubbing.** Metadata apart from bytes (A2) keeps the
Backlog, the mirror and every list off TOAST, and makes the later object store a one-table swap.
Owning by document version keeps the foreign key exact; the Documents view lists a document family's
attachments with the version each came with. A 10 MiB default with a 64 MiB DDL ceiling sits inside
the surveyed range and bounds sqlx's in-memory reads. On `R-ID-7`: text is scrubbed and fails closed like
notes; a binary cannot be scrubbed, and refusing images contradicts `R-ENT-12`'s own example, so a
binary is accepted only from a person who is told it is stored as given, never from an agent, and the
requirement says so.

**Links: keep item-to-item semantics untouched.** Readiness, the upstream walk, the Graph tab and the
mirror's primary key all assume an item at both ends. A separate table for document and attachment
targets (L2) leaves them alone, exactly as `item_requirement` does for citations. Pinning the document
version matches citations, which stamp the requirement version. `parent` single-valued and acyclic is
what every tracker with a hierarchy does; the parent joins the upstream walk so a child's prompt sees
its parent's summary. Close-out recording `duplicates`/`supersedes` from the person's choice is a
deterministic write from a person's confirmation, which `R-ID-6` allows; `R-ENT-9`'s "no manual link
action" needs one clause for it.

## 7. Verdict

### 7.1 Q-g: three stored statuses, a derived display state, a database guard

**Stored.** `item.status IN ('open','blocked','closed')`. `blocked` is set by the orchestrator when a
person must clear something (rung four, missing tags, review-loop exhaustion, no candidate agent) and
always with an `item_note`; `Unblock` clears it. `closed` is reached only through `close_out`. Allowed
moves: `open ↔ blocked`, `open|blocked → closed` (close-out only). Status moves keep their own
compare-and-set and never bump `version`.

**Derived display state** (`ItemState` in htui-core, one pure function, attention order). Here and
below, the **latest graph run** is the newest `kind = 'graph'` run of the item by `(queued_at, id)`, as
`queue_rows` orders it (`pg/read.rs:2353-2355`), skipping `cancelled` runs: a cancelled run changed
nothing, so it never changes the item's state.

| Item status | Latest graph run | Display | Glyph |
|---|---|---|---|
| open | `awaiting_approval` | needs you | ⚠ |
| open | `failed` | failed | ✗ |
| open | `running` | running | ◐ |
| blocked | any | blocked | ⊘ |
| open | `queued` | queued | ◌ |
| open | none (or only cancelled ones) | open | ○ |
| open | `done` | done (finished, close-out waiting) | ● |
| closed | any | closed, with its resolution | ✓ |

The store appends `latest_run_status` to `ItemSummary` (positional projections append, plan D10); the
Backlog's status facet, grouping and `h` toggle work on `ItemState`; `ItemFilter.statuses` stays for the
stored value (MOD-11), and a `states` filter is added. The prompt's upstream line prints the display word.

**Readiness** (auto-queue, `items(ready)`, cache and MemStore alike): status `open`, no live graph run,
latest graph run neither `failed` nor `done`, and every live `blocked_by` target **released**. A target is
released when it is `closed` (any resolution) or **finished**: its latest graph run is `done` and every
integration record of that run is `landed` or `pr_merged` (no records counts as landed, so this equals
today's `done | closed` rule until MOD-133 exists). P1 implements "latest graph run `done`" only, since
the integration records do not exist before MOD-133; MOD-133, which creates them, extends every
readiness site with the landed check (`pg/read.rs:116, 2100, 2367`, MemStore, and `cache/read.rs:402`,
the cache mirroring the records to do so).

**Rework.** The `done → open` edge goes; nothing in production takes it today (§2.2). Rework of a
finished item is a new run a person starts on it (auto never re-picks a finished item). While that run
is live the item is not released: dependents not yet started wait, running ones are not stopped. If the
person cancels it, the cancelled run is skipped and the earlier `done` releases the dependents again; if
it fails, the item shows `failed` and holds them.

**One live graph run per item**: `create_run` checks for a live graph run under the item's `FOR UPDATE`
lock and refuses with today's sentence, and the database backs it with a partial unique index. `close_batch`'s
lock-ordering argument ("this item is open or failed") is re-stated as "this item has no other live run".

**Close-out law** (replaces MOD-38 D1, keeps D2's shape): close-out is refused while a run is live.
`done` and `concluded` require a finished latest graph run (as above). `rejected`, `withdrawn`,
`superseded` and `duplicate` close an `open` or `blocked` item whatever its runs did; `superseded` and
`duplicate` also need their link (7.6). `default_for`: finished and the last phase of the finished run's
`graph_snapshot` has `output_kind = 'verdict'` → `concluded`; finished otherwise → `done`; not finished
→ `withdrawn`. It reads the run's snapshot, not the kind's default graph, because the run may have used
the item's `step_graph_id` override or a picked workflow (MOD-126).

**The migration.** A count runs first and its numbers go into P1's plan: per removed status, how many
items have a live graph run, a latest graph run in each terminal state, or none, and how many items have
two live graph runs (which would make `uq_run_item_live` fail to build). Then:

| Today | Latest graph run | Becomes | Note |
|---|---|---|---|
| `queued`, `in_progress`, `awaiting_approval` | any | `open` | none; a live run carries the state, and one without a live run is ready again, as cancelling would leave it |
| `failed` | `failed` | `open` | none; shows `failed` |
| `failed` | none, or not `failed` | `blocked` | "failed outside a run before the status migration; clear to run it again" |
| `done` | `done` | `open` | none; shows `done`, close-out waiting |
| `done` | none, or not `done` | `closed`, resolution `done` | "closed by the status migration: marked done with no finished run" |

The notes are written as the item's `created_by` (`item_note.created_by` is NOT NULL,
`0001_init.sql:374-382`), with the `updated_at` trigger off around the updates, as today's backfills do.
The one `closed` mapping is the exception to "the migration closes nothing": such a row has no run from
which "finished" could be derived, so `open` would make auto mode re-run it and stop it releasing its
dependents, and `blocked` would hold its dependents and could not later close as `done` under the law
above. Closing it as `done` keeps all three of today's behaviours for it.

**Every consumer it touches:**

| Area | Change |
|---|---|
| Store writes (Pg, Mem, conformance) | `create_run` stops writing the item and checks the live run; `claim_run` drops `queued → in_progress` and keeps `queued → blocked` as `open → blocked` on missing tags; `promote_step`, `park_step` and `finish_run_on` drop the item mirror (`finish_run_item_mirror` is deleted); `transition` takes the 3-value enum; `close_out` uses the new law; queue drain reads "closed or finished". |
| Orchestrator | of the 13 production `.transition(` calls in `engine.rs`, the 7 that mirror a run move (`cancel_queued`, `recover_run`, `park_interrupted`, `resume_window`, `park_selection`, `park_run`, `unpark`) are dropped (`cancel_queued`'s E-2 window goes with them); the 4 blocking ones (`refuse_rung_four`, `refuse_missing_tags`, `refuse_no_candidate`, `block_and_fail`) and `gate.rs::escalate` move `open → blocked`; of `unblock`'s 2, `blocked → open` stays and FollowRun's `blocked → awaiting_approval` becomes `blocked → open` with the run's `awaiting_approval` carrying "needs you"; `start_enabled`, `unblock_enabled` (Resume = open + parked run) and `close_out_enabled` follow. |
| Readers | `ready_items`, `items(ready)`, `queue_rows` and the cache's runtime SQL use the readiness rule above; `classify_entry` reads `Failed` from the latest run and "parked and not blocked" for HardGate/Judge; `Wait::NotReady` carries the display state. |
| Waiting-on-you | candidates are `status = 'blocked'` or a run `awaiting_approval`; MOD-135 adds finished-not-closed items as `CloseOut`. `Unblock` rows read the item; the rest read runs, unchanged. |
| Cache | runtime SQL rewritten; the `schema_version` bump rebuilds every mirror (no cache migration needed for the CHECK). |
| Search | collection `htui_concepts_v2` → `v3`; the `status` payload carries the 3 values; staleness still compares it, and run moves stop re-embedding items. The decisions filter is unchanged (resolution only). |
| MCP and prompt | `item_status` and `search_concepts` enums shrink to 3 (snapshots move); prompt upstream prints the display word (prompt digests move). |
| TUI | `theme.rs`, `kit/status.rs`, the Backlog filter, list, detail and grouping key on `ItemState`; `toggle_queued` refuses `closed`; the requirements coverage view prints the display word. |
| Fixtures | demo items "across statuses" become items with runs in those states (`fixtures.rs:953-1070`, `pg/demo.rs`); every Backlog snapshot is re-accepted after a full insta run. |

### 7.2 Q-b: repo kinds, repo-less runs and what refuses without git

A repo declares `vcs = 'git' | 'folder'`. Registration checks the path against it: `git` must open as a
repository; `folder` must be a directory and gets a hint when it contains `.git`. A folder has no
`remote_url` and no `default_branch`. A phase carries `needs_repo`; `snapshot_phase` refuses a run whose
phase needs a repo in a project with no live repos, before anything starts.

**Isolation resolves per repo.** The step's mode (phase, else project, else `app_setting`) applies to
each git repo of the scope. For a folder: under `worktree` it takes `copy`; under `copy` or `local` it
takes that mode; under `shared_serialized` the snapshot refuses, naming the folder ("`notes` is a plain
folder: `shared_serialized` needs git; set this phase to `copy` or `local`"). `prepare` partitions the
scope by `vcs` and each `run_step_tree` row records the mode its repo got, which capture and reconcile
follow. A mixed scope, or a software project whose primary is a folder, therefore runs its code phases
with no setting changed. Fan-out above 1 refuses when any folder in the scope resolves to `copy`.
**Default isolation when a folder joins a software project:** unchanged, `worktree`; the folder is
copied by the rule above. No setting is rewritten on registration.

| Feature | Git repo | Folder | No repos |
|---|---|---|---|
| `worktree` isolation | yes | resolves to `copy` for that repo | session folder, no git probe |
| `shared_serialized` | yes | **refused at snapshot**, naming the folder (no label, no reset) | session folder |
| `copy` | yes | yes: digest manifest at prepare; reconcile copies back files unchanged in the folder since, parks `folder_conflict: <paths>` otherwise; fan-out above 1 refused | session folder |
| `local` | yes | yes, in place; no commit rows | session folder |
| Commit hashes, `run_step_commit` | yes | none written; close-out says "folder: n files changed" (copy) or "in place" (local) | none |
| Step reconcile (ANA-2 §4.6) | `git merge --no-ff` | copy-back as above; `local` is a no-op | none |
| Integration (ANA-34) | `local`, `push`, `pull_request` | `local` only | `local` only (vacuous) |
| Clone onto a box (`R-BOX-4`, MOD-108) | yes, from `remote_url` | **refused**; unavailable on a box without a path | n/a |
| Driven target (ANA-42) | clone and bundle | **refused**: a folder runs on this box only | allowed: nothing to prepare but components and env |
| Diff, `{{diff}}`, changed paths | yes | absent (already degrades) | absent |
| Verify, promoted chat | primary tree | primary folder | session folder (`isolation_path` recorded) |
| `touched_paths` | as today | as today | any entry refused at item write |
| cwd → project | `repo_box_path` | `repo_box_path` | `--project`, or the workspace folder |
| Checkout inference | under the workspace folder | never inferred | n/a |

Seeded `needs_repo`: true for software `reproduce`, `implement`, `fix` and `review`; false for every other
seeded phase. A person may change it per phase. Default isolation per type, written into the project's
`settings.default_isolation` by its seed: `worktree` for software, `local` for the others.

### 7.3 Q-h: the primary repo (MOD-106's part)

- A project with live repos has **exactly one primary**; a folder may be it. Zero repos is allowed.
- **Writes:** `create_repo` makes a project's first live repo primary whatever the flag says;
  `update_repo` refuses `is_primary = false` on the current primary (`primary_required`: "make another
  repo primary instead"); `is_primary = true` demotes the old one in the same transaction, as today.
- **Removal is archiving.** `archive_repo(repo, successor)` sets `archived_at`; an archived repo is never
  primary, is out of every scope, overlap and repo map, and keeps its run history. Archiving the primary
  promotes the named successor, else the oldest live repo `(created_at, id)`, in the same transaction.
  Archiving the last repo leaves the project repo-less. Settings shows archived repos greyed out.
  A repo's name is unique among the project's live repos only (`UNIQUE (project_id, name)` becomes a
  partial unique `WHERE archived_at IS NULL`), so a name can be registered again after archiving; the
  archived row keeps its history under its id.
- **Backstop:** a deferred constraint trigger checks, per touched project at commit, "no live repos, or
  exactly one live primary". MemStore enforces the same rule in code; the conformance suite pins both.
- **Backfill:** a project with live repos and no primary gets its oldest repo promoted.

### 7.4 Q-a: project types and their seeds

`project_type` is a table; `project.type` references it and is set at creation; the seed is copied into
the project's own kinds, graphs, phases and templates once, and **editing or adding a type never
re-seeds an existing project**. Built-in types carry no recipe in the row (code supplies it, so ANA-33
can change the graph format); user-defined types, which carry a JSONB recipe, arrive with ANA-33's
Settings graph editor (`R-TUI-8`) and can start as "save this project as a type". A `coding` flag marks
software-like types for ANA-39 and MOD-128.

| Type | Kind (prefix) | Default graph | `coding` |
|---|---|---|---|
| software | ANA, FEAT, FIX, CLEAN, TOOL | today's (`R-ENT-6`) | yes |
| research | RES research | research → verdict | no |
| research | Q question | question → answer | no |
| docs | DOC document | outline → draft → review | no |
| ops | OPS operation | plan → execute → verify | no |
| ops | INC incident | investigate → fix → postmortem | no |
| general | TASK task | execute → review | no |
| workspace (scope rows only, 7.5) | WS workspace item | plan → review | no |

New templates are added for `question`, `answer`, `outline`, `draft`, `execute`, `verify`, `investigate`,
`postmortem` and the workspace `plan`; a project receives only the templates its type's graphs use.
Every `review` and `verdict` stays hard-gated as today; `postmortem` and `verify` are hard-gated too. The
demo fixture's id strides become per-type offsets. Existing projects become `software`. This repository's
own prefixes (MOD, NEXT, VAL) are the importer's per-project mapping (`R-LATER-3`, MOD-8), not a seed.

Two spawns carry this: P3 lands the table, its six built-in rows (no recipes), `project.type` (existing
projects `software`) and the `coding` flag, which is all P4's scope rows, ANA-39 and MOD-128 need; P5
lands the per-type seeds, templates, `needs_repo` and default isolation per type, and the type picker,
after P2 (folders and `needs_repo`) and P3.

### 7.5 Q-c and Q-d: the workspace scope row, keys, documents and the folder

- **Scope row.** Each workspace has one row in `project` with `owner_workspace_id` set and type
  `workspace`, created with the workspace in the same transaction (the bootstrap step creates it for
  existing workspaces). It owns the workspace's items, kinds, counter, graphs, templates, documents,
  requirements and runs (MOD-136's workspace coordinator runs included). It never has repos, never
  joins `workspace_project`, and never appears in project lists, Settings' hierarchy, cwd mapping or
  `htui status`; every listing goes through one visible-projects predicate, pinned by a conformance test.
  Its slug is `@<workspace-slug>`, a form a user slug cannot take. Its secret binding is held in its own
  `secret_provider`/`secret_scope` columns (`0001_init.sql:147-148`), set from the workspace's Settings
  form, since no workspace-level secret binding exists; its skill and target bindings are the global
  ones until MOD-113 (2) adds a workspace skill level and MOD-130 a workspace `target_id`, which the scope
  row's runs then read from its workspace.
- **Keys.** `R-ENT-7`'s counter is per owner (a project or a scope row) and prefix, with Postgres the only
  writer; `mint_item` is unchanged. A workspace kind's prefix may not equal a current member project's;
  a project whose prefix collides may still join, with a warning; keys are qualified `slug:KEY` wherever
  ambiguous (MCP, CLI, importer), the slug being the owning `project` row's: `api:FEAT-9` for a project
  item, `@shop:WS-1` for an item of workspace `shop`, even when a project is also called `shop`.
  `propose_link` accepts targets in the step's project, in the workspaces containing it, and in those
  workspaces' member projects.
- **Execution.** A workspace item runs in its scope row, repo-less: phases flagged `needs_repo` refuse;
  code work happens in child items of member projects, linked by `parent`.
- **Documents.** Every document row carries `project_id`; `item_id` is nullable. An item document
  names both; a project document names its project; a workspace document names the scope row. Version
  allocation locks the owner row. Owner documents are written by a person in the TUI or as an accepted
  draft (`R-ID-6`); `document_write` over MCP stays item-only. Kinds are an open set; `goals` is
  conventional for workspaces and `summary` stays reserved for item close-out.
- **Folder.** `workspace_box_path.root_path` becomes `folder_path`; an absent row means
  `~/workspaces/<slug>` on that box. The folder holds what `htui` generates (the compose file, and the env
  file without Infisical) and clones made from a URL; no `workspace.toml`. Description stays the
  `workspace.description` column; goals are a workspace document. A lone project has no folder and, in
  version one, no components.
- **Cache** follows for free: the scope row is cached like any project when its workspace is open; the
  Backlog groups it as the workspace's own group.
- **Search** needs one addition. Items and requirements of the scope row index as any project's do, but
  the index reaches documents only through items (`Owner` is `Item | Requirement`, `vector.rs:192-197`;
  `vector_sync.rs:88-110`), so a project or workspace document would never be indexed and `R-STO-8`'s
  "workspace-owned … documents included" would fail. P4 adds `Owner::Document` for points of documents
  with no item (payload `document_id`, no `item_id`, no `status`), and an indexer pass over each
  project's latest owner documents per kind, beside the item loop. That is a payload change, so P4 moves
  the collection to `htui_concepts_v4` (P1 having taken `v3`; one more full rebuild from Postgres).
  MOD-112's workspace scope is "member projects plus the scope row".

### 7.6 Q-f: links

| Kind | From → to | Meaning | Rules |
|---|---|---|---|
| `blocked_by` | item → item | cannot start until the target is released (7.1) | unchanged |
| `origin` | item → item, or → document | was spawned by; provenance | unchanged |
| `relates` | item → item, document or attachment | see also | unchanged |
| `supersedes` | new → old | replaces | closing the old item as `superseded` requires a live `supersedes` to it, or close-out records one from the item the person names |
| `parent` | child → parent | is a part of | at most one live parent per child; no cycles (checked by walking up before insert); depth at most 8; joins the upstream prompt walk; does not affect readiness; closing a parent with open children is flagged on MOD-135's checklist, never automatic |
| `duplicates` | duplicate → canonical | is the same as | at most one live canonical per item; closing as `duplicate` requires it, or close-out records it from the item the person names |
| `implements` | item → item or document | carries out that decision or specification | e.g. a FEAT implements an ANA's verdict |

Item-to-item kinds stay in `item_link`. Links to a document or an attachment go to `item_artifact_link`,
whose kinds are `origin`, `relates` and `implements`; a document target pins a version (a document row),
shown as "v2 (latest v4)". Writers: MCP `item_link` (gaining a `to_document` argument and qualified
keys), the importer, and close-out for `duplicates`/`supersedes`. Graph legend words: blocks, from,
relates to, replaces, part of, duplicate of, implements; "refines" and "covered by" remain requirement
relations, not links.

### 7.7 Q-c (requirements): workspace requirement keys

Workspace requirements are `W-<AREA>-<N>`, minted from the scope row's own areas and counters; project
requirements stay `R-<AREA>-<N>`; there is no shared `R-n` across a workspace's projects. A requirement
may refine one other requirement (`refines_id`). Within one owner the child is keyed parent plus `.n`
(`R-TUI-2.1`, `W-ORCH-3.1`); a project requirement refining a workspace one keeps its own project key and
shows "↑ from W-ORCH-3". A workspace requirement may refine only a workspace one; a project requirement
its own project's or one of its workspaces'. Keys are stored at mint.

**Child keys.** A nested child (same owner as its parent) keeps the parent's `area_id` and `area_code`,
leaves `number` NULL and takes `child_number` from a counter per parent, so its key is the parent's key
plus `.<child_number>`. A cross-owner refinement is not nested: it takes a top-level `number` from its
own area's counter. The existing `UNIQUE (project_id, area_code, number)` keeps guarding top-level rows
(NULL numbers are distinct, so children never collide in it), `UNIQUE (refines_id, child_number)` keeps
siblings apart, and a new `UNIQUE (project_id, key)` guards the stored key itself. CHECKs tie the shape:
exactly one of `number`, `child_number` is set, and `child_number` needs `refines_id`. Same-owner nesting
and the parent's area are checked at the write (a CHECK cannot read the parent row) and pinned by the
conformance suite. The DDL is P6's (7.9).

### 7.8 Q-e: attachments

- **Owners:** an item, or a document version. Not notes, projects or workspaces directly (a project or
  workspace attaches through one of its documents).
- **Storage:** `attachment` (metadata, with `project_id` for scoping) and `attachment_blob` (`bytea`),
  `ON DELETE CASCADE` from the owner. `project_id` is kept equal to the owner's by composite foreign
  keys, as for documents: `(item_id, project_id) → item(id, project_id)` and `(document_id, project_id)
  → document(id, project_id)`, using P4's `uq_item_id_project` and `uq_document_id_project` (a NULL
  owner column leaves its key unchecked, which is what the one-owner CHECK wants). Removal tombstones the metadata and deletes the blob in one
  transaction. No large objects, no deduplication in version one.
- **Cap:** `app_setting attachment_max_bytes`, default 10 MiB, checked before the insert; a DDL ceiling
  of 64 MiB behind it.
- **Scrubbing:** text media types pass the scrubber and fail closed; a binary is accepted only from a
  person in the TUI, who confirms it is stored as given, and is marked `scrubbed = false`; agents may
  attach text only (when an MCP tool exists).
- **Cache and search:** metadata mirrored, bytes fetched online only; not indexed in version one.
- **Naming:** the Rust type is `FileAttachment`; the skills pane keeps its name.

### 7.9 Schema sketch

DDL is indicative; names of auto-generated constraints must be checked against a live database first.

```sql
-- P1: item status (migrate + contract in one file; MOD-40 keeps older binaries out)
-- Pre-flight count (run by hand first; its numbers go into P1's plan): rows no run explains.
--   SELECT i.status, count(*) FROM item i
--     LEFT JOIN LATERAL (SELECT r.status FROM run r
--                         WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled'
--                         ORDER BY r.queued_at DESC, r.id DESC LIMIT 1) lr ON true
--    WHERE i.status IN ('done', 'failed') AND lr.status IS DISTINCT FROM i.status
--    GROUP BY i.status;
CREATE UNIQUE INDEX uq_run_item_live ON run (item_id)
    WHERE kind = 'graph' AND status IN ('queued', 'running', 'awaiting_approval');
ALTER TABLE item DISABLE TRIGGER trg_item_updated_at;
CREATE TEMP TABLE unexplained ON COMMIT DROP AS                         -- the pre-flight's rows
    SELECT i.id, i.status, i.created_by FROM item i
      LEFT JOIN LATERAL (SELECT r.status FROM run r
                          WHERE r.item_id = i.id AND r.kind = 'graph' AND r.status <> 'cancelled'
                          ORDER BY r.queued_at DESC, r.id DESC LIMIT 1) lr ON true
     WHERE i.status IN ('done', 'failed') AND lr.status IS DISTINCT FROM i.status;
INSERT INTO item_note (item_id, body, created_by)
    SELECT id, CASE status
        WHEN 'done'   THEN 'closed by the status migration: marked done with no finished run'
        ELSE 'failed outside a run before the status migration; clear to run it again' END,
        created_by FROM unexplained;
UPDATE item SET status = 'closed', resolution = 'done', closed_at = COALESCE(closed_at, now())
 WHERE id IN (SELECT id FROM unexplained WHERE status = 'done');
UPDATE item SET status = 'blocked'
 WHERE id IN (SELECT id FROM unexplained WHERE status = 'failed');
UPDATE item SET status = 'open', closed_at = NULL
 WHERE status IN ('queued', 'in_progress', 'awaiting_approval', 'failed', 'done');
ALTER TABLE item ENABLE TRIGGER trg_item_updated_at;
ALTER TABLE item DROP CONSTRAINT item_status_check;                    -- auto-named today
ALTER TABLE item ADD CONSTRAINT chk_item_status
    CHECK (status IN ('open', 'blocked', 'closed')) NOT VALID;
ALTER TABLE item VALIDATE CONSTRAINT chk_item_status;

-- P2 and MOD-106: repo kinds, archive, primary
ALTER TABLE repo ADD COLUMN vcs TEXT NOT NULL DEFAULT 'git'
    CONSTRAINT chk_repo_vcs CHECK (vcs IN ('git', 'folder'));
ALTER TABLE repo ALTER COLUMN default_branch DROP NOT NULL;
ALTER TABLE repo ADD CONSTRAINT chk_repo_git_fields CHECK (
    (vcs = 'git' AND default_branch IS NOT NULL)
 OR (vcs = 'folder' AND default_branch IS NULL AND remote_url IS NULL));
ALTER TABLE repo ADD COLUMN archived_at TIMESTAMPTZ;
ALTER TABLE repo ADD CONSTRAINT chk_repo_archived_not_primary
    CHECK (archived_at IS NULL OR NOT is_primary);
ALTER TABLE repo DROP CONSTRAINT repo_project_id_name_key;             -- auto-named today
CREATE UNIQUE INDEX uq_repo_live_name ON repo (project_id, name) WHERE archived_at IS NULL;
-- backfill: promote the oldest live repo of every project with live repos and no primary
CREATE CONSTRAINT TRIGGER trg_repo_one_primary AFTER INSERT OR UPDATE OR DELETE ON repo
    DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION repo_one_primary();
ALTER TABLE run_step_tree ALTER COLUMN base_ref DROP NOT NULL;          -- folder trees
ALTER TABLE step_graph_phase ADD COLUMN needs_repo BOOLEAN NOT NULL DEFAULT false;
-- backfill: needs_repo = true for software reproduce/implement/fix/review phases

-- P3: project type table (P5's seeds and picker add no DDL)
CREATE TABLE project_type (
    slug         TEXT PRIMARY KEY CHECK (slug ~ '^[a-z][a-z0-9-]{0,31}$'),
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    coding       BOOLEAN NOT NULL DEFAULT false,
    builtin      BOOLEAN NOT NULL DEFAULT false,
    selectable   BOOLEAN NOT NULL DEFAULT true,     -- false for 'workspace'
    seed         JSONB,                             -- NULL for built-ins: code holds their recipe
    seed_version INTEGER NOT NULL DEFAULT 1,
    created_by   UUID REFERENCES app_user(id),      -- NULL for built-ins
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (builtin OR seed IS NOT NULL)
);
-- INSERT the six built-in rows (software, research, docs, ops, general, workspace)
ALTER TABLE project ADD COLUMN type TEXT NOT NULL DEFAULT 'software'
    REFERENCES project_type(slug);

-- P4: workspace scope row, owner documents, folder (scope rows take type 'workspace', slug '@<ws-slug>')
ALTER TABLE project ADD COLUMN owner_workspace_id UUID UNIQUE
    REFERENCES workspace(id) ON DELETE CASCADE;
ALTER TABLE project ADD CONSTRAINT chk_project_scope_row CHECK (
    (owner_workspace_id IS NULL) = (type <> 'workspace'));
ALTER TABLE item ADD CONSTRAINT uq_item_id_project UNIQUE (id, project_id);
ALTER TABLE document ADD COLUMN project_id UUID REFERENCES project(id) ON DELETE CASCADE;
UPDATE document d SET project_id = i.project_id FROM item i WHERE i.id = d.item_id;
ALTER TABLE document ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE document ALTER COLUMN item_id DROP NOT NULL;
ALTER TABLE document ADD CONSTRAINT fk_document_item_project
    FOREIGN KEY (item_id, project_id) REFERENCES item(id, project_id) ON DELETE CASCADE;
ALTER TABLE document ADD CONSTRAINT uq_document_id_project UNIQUE (id, project_id);  -- attachment FK
ALTER TABLE document DROP CONSTRAINT document_item_id_kind_version_key; -- auto-named today
ALTER TABLE document ADD CONSTRAINT uq_document_owner_kind_version
    UNIQUE NULLS NOT DISTINCT (project_id, item_id, kind, version);
ALTER TABLE workspace_box_path RENAME COLUMN root_path TO folder_path;

-- P6: workspace requirements and nesting
ALTER TABLE requirement ALTER COLUMN key DROP EXPRESSION;               -- keep values, stored from now on
ALTER TABLE requirement ADD COLUMN refines_id UUID REFERENCES requirement(id);
ALTER TABLE requirement ADD COLUMN child_number INTEGER CHECK (child_number >= 1);
ALTER TABLE requirement ALTER COLUMN number DROP NOT NULL;             -- NULL on nested children
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_number_or_child
    CHECK (num_nonnulls(number, child_number) = 1);
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_child_refines
    CHECK (child_number IS NULL OR refines_id IS NOT NULL);
ALTER TABLE requirement ADD CONSTRAINT chk_requirement_key_shape
    CHECK (key ~ '^[RW]-[A-Z][A-Z0-9]*-[1-9][0-9]*(\.[1-9][0-9]*)*$');
ALTER TABLE requirement ADD CONSTRAINT uq_requirement_key UNIQUE (project_id, key);
ALTER TABLE requirement ADD CONSTRAINT uq_requirement_child UNIQUE (refines_id, child_number);
-- UNIQUE (project_id, area_code, number) stays: NULL numbers are distinct, so children pass it
CREATE TABLE requirement_child_counter (
    parent_id  UUID PRIMARY KEY REFERENCES requirement(id) ON DELETE CASCADE,
    last_value INTEGER NOT NULL CHECK (last_value >= 0)
);

-- P7: links
ALTER TABLE item_link DROP CONSTRAINT item_link_kind_check;            -- auto-named today
ALTER TABLE item_link ADD CONSTRAINT chk_item_link_kind CHECK (kind IN
    ('blocked_by', 'origin', 'relates', 'supersedes', 'parent', 'duplicates', 'implements'));
CREATE UNIQUE INDEX uq_item_link_one_parent ON item_link (from_item_id)
    WHERE kind = 'parent' AND deleted_at IS NULL;
CREATE UNIQUE INDEX uq_item_link_one_canonical ON item_link (from_item_id)
    WHERE kind = 'duplicates' AND deleted_at IS NULL;
CREATE TABLE item_artifact_link (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    from_item_id        UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL CHECK (kind IN ('origin', 'relates', 'implements')),
    to_document_id      UUID REFERENCES document(id) ON DELETE CASCADE,
    to_attachment_id    UUID,          -- FK added by MOD-114 (3)'s migration, which lands after P7
    proposed_by_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at          TIMESTAMPTZ,
    CHECK (num_nonnulls(to_document_id, to_attachment_id) = 1),
    UNIQUE NULLS NOT DISTINCT (from_item_id, kind, to_document_id, to_attachment_id)
);

-- MOD-114 (3): attachments
CREATE TABLE attachment (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id  UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    item_id     UUID REFERENCES item(id) ON DELETE CASCADE,
    document_id UUID REFERENCES document(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    media_type  TEXT NOT NULL,
    byte_size   BIGINT NOT NULL CHECK (byte_size > 0 AND byte_size <= 67108864),
    sha256      TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    scrubbed    BOOLEAN NOT NULL,
    created_by  UUID NOT NULL REFERENCES app_user(id),
    box_id      UUID REFERENCES box(id) ON DELETE SET NULL,
    via_step_id UUID REFERENCES run_step(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at  TIMESTAMPTZ,
    CONSTRAINT chk_attachment_owner CHECK (num_nonnulls(item_id, document_id) = 1),
    CONSTRAINT fk_attachment_item_project
        FOREIGN KEY (item_id, project_id) REFERENCES item(id, project_id) ON DELETE CASCADE,
    CONSTRAINT fk_attachment_document_project
        FOREIGN KEY (document_id, project_id) REFERENCES document(id, project_id) ON DELETE CASCADE
);
CREATE TABLE attachment_blob (
    attachment_id UUID PRIMARY KEY REFERENCES attachment(id) ON DELETE CASCADE,
    data          BYTEA NOT NULL CHECK (octet_length(data) <= 67108864)
);
-- app_setting: attachment_max_bytes = 10485760
ALTER TABLE item_artifact_link ADD CONSTRAINT fk_item_artifact_link_attachment
    FOREIGN KEY (to_attachment_id) REFERENCES attachment(id) ON DELETE CASCADE;
```

### 7.10 Migration sequence

Each spawn ships one forward migration in the order below, applied by the binary that already reads and
writes the new shape; MOD-40's target version keeps older binaries out, so data changes and narrowing
may share a file. Each is tested `run_to(N-1)` → plant rows → `run_to(N)` → assert.

| Order | Spawn | Expand | Migrate | Contract |
|---|---|---|---|---|
| 1 | P1 item status | `uq_run_item_live` (refuses if duplicates exist: the pre-flight count checks first) | the 7.1 mapping: unexplained `done` → `closed`/`done` with a note, unexplained `failed` → `blocked` with a note, the rest of the removed values → `open`, `closed_at` cleared; trigger off around it | 3-value CHECK; Qdrant `v3`; mirror rebuild by version bump |
| 2 | P2 + MOD-106 repos | `vcs`, `archived_at`, `needs_repo`, nullable `default_branch` and `base_ref`, live-name partial unique, deferred trigger | primary backfill; `needs_repo` backfill on software phases | old `UNIQUE (project_id, name)` dropped |
| 3 | P3 type table | `project_type` with its six built-in rows, `project.type` | existing projects → `software` | none |
| 4 | P4 workspace scope | `owner_workspace_id`, scope-row CHECK, `document.project_id`, composite FK, `uq_item_id_project`, `uq_document_id_project` | `document.project_id` backfill; scope rows created by bootstrap | `document.item_id` nullable with the new unique; `root_path` renamed; Qdrant `v4` |
| 5 | P5 per-type seeds | none (seeds are code) | none | none |
| 6 | P6 requirements | `refines_id`, `child_number`, child counter, key and child uniques | none | `key` becomes stored; `number` nullable |
| 7 | P7 links | widened CHECK, partial uniques, `item_artifact_link` | none | none |
| 8 | MOD-114 (3) | `attachment` with composite FKs, `attachment_blob`, setting, `item_artifact_link.to_attachment_id` FK | none | none |

The order is the reference; P2, P3 and P1 have no blockers among themselves and P5 may land before P4, so
migration numbers follow actual landing order within the blockers of §9. P1's test plants, besides one
row per removed status with a matching run, a `done` item with no graph run, a `done` item whose latest
graph run failed, a `done` item whose `done` run is followed by a cancelled one (becomes `open`, shows
done), and a
`failed` item with no graph run, and asserts the 7.1 mapping and the two notes.

Every step that adds a mirrored column ships a cache migration (`cache_migrations/0006_*` onwards: repo
`vcs`/`archived_at`, project `type`/`owner_workspace_id`, document `project_id` with nullable `item_id`,
requirement `refines_id`/`child_number` with nullable `number`, `item_artifact_link`, attachment
metadata) and grows `MIRRORED_TABLES`. MOD-107 (labels) and MOD-130 (`target_id`) touch the same tables and take their own numbers in landing order; nothing here depends on
them.

## 8. Questions for the maintainer

1. **Three stored statuses and a derived display state (7.1):** `open | blocked | closed` stored;
   queued, running, needs you, failed and done derived from the latest graph run (cancelled runs
   skipped), in the redesign's attention order. **Recommended: accept.** The run already carries every
   removed value, and one function gives every surface the same words.
2. **The migration closes nothing a run can explain (7.1, 7.10):** queued, in progress, awaiting
   approval, and failed or done with a matching latest run, become `open`; a finished item shows "done"
   and waits for a person's close-out (MOD-135). **Recommended: accept.** Closing them in the migration
   would choose resolutions for the person, which ANA-34 §7.6 reserved to them.
3. **Rows no run explains (7.1):** after a pre-migration count, a `done` item whose latest graph run is
   not `done` is closed `done` with a note; a `failed` item whose latest graph run is not `failed` becomes
   `blocked` with a note. **Recommended: accept.** `open` would re-run finished work and stop it releasing
   its dependents; `blocked` for the `done` rows (the alternative) would hold its dependents and could
   never close as `done` under question 5's law. If you would rather no row is closed by a migration,
   say so and they go `blocked`, with a close-out exception for them to be designed in P1.
4. **A blocker releases its dependents when closed or finished (7.1):** finished means its latest graph
   run is `done` and its work landed; a blocker closed `rejected` or `withdrawn` still releases; a
   cancelled rework run does not un-release it. P1 checks "latest run `done`"; MOD-133 adds "landed" with
   its integration records. **Recommended: accept.** It is today's `done | closed` rule, so auto mode does
   not stall on close-out.
5. **Close-out law and rework (7.1):** `done`/`concluded` need a finished latest run; the other four close
   an open or blocked item; the default is `concluded` when the finished run's own graph snapshot ends in
   a verdict; `done → open` goes, and rework is a new run a person starts. **Recommended: accept.** It
   keeps MOD-38's guarantees on facts that survive the reduction and serves research kinds; no production
   code takes `done → open` today.
6. **One live graph run per item as a partial unique index, beside the code check (7.1).**
   **Recommended: accept.** The item-status lock it replaces disappears with the status values.
7. **Repo kinds and the refusal table (7.2):** `git | folder` declared and checked at registration;
   folders do `local` and `copy` (copy-back, park on conflict), never a worktree, merge, push, clone or a
   driven target. **Recommended: accept.** Refusals then happen at snapshot, before an agent starts.
8. **Folders in git-isolated steps (7.2):** isolation resolves per repo: a folder under `worktree` is
   copied, `shared_serialized` over a folder refuses naming it, and a software project keeps its
   `worktree` default when a folder joins. **Recommended: accept.** Refusing instead would block every
   code phase of a project with a folder primary or a mixed scope; `copy` keeps the isolated-tree overlap
   rule, and the tree row already records a mode per repo.
9. **`needs_repo` per phase (7.2):** explicit, seeded true for software `reproduce`, `implement`, `fix`,
   `review`; repo-less steps run in their session folder. **Recommended: accept.** Neither isolation nor
   phase names say reliably whether a phase needs code.
10. **Primary repo (7.3):** a folder may be primary; exactly one when any live repo exists, enforced at
    the store write with a deferred trigger behind it; removal is archiving with promotion; names unique
    among live repos only; MOD-106 carries it. **Recommended: accept.** Every reader of the primary needs
    a directory, not git, and run history forbids hard deletes.
11. **Project types (7.4):** a `project_type` table; built-ins software, research, docs, ops and general
    (plus the internal workspace type); seeds copied once, never re-applied; prefixes RES, Q, DOC, OPS, INC,
    TASK, WS; user-defined types with ANA-33's editor; a `coding` flag for ANA-39. **Recommended: accept;
    the prefixes and the `general` type are yours to change.**
12. **Workspace ownership through a scope row (7.5):** one hidden `project` row per workspace owns its
    items, kinds, counter, graphs, documents, requirements and runs (MOD-136 adopts it instead of a
    nullable `run.project_id`); its own secret columns hold the workspace's secret binding until
    workspace-level bindings exist. **Recommended: accept.** It reuses every per-project path; owner
    columns would duplicate them and fail silently where a join is missed.
13. **Workspace keys (7.5):** per-owner counters; a workspace kind prefix may not equal a member's at
    creation; keys qualified when ambiguous by the owning row's slug, the scope row's being
    `@<workspace-slug>` (`@shop:WS-1`); links allowed across a workspace and its members.
    **Recommended: accept.** Collisions cannot be prevented once projects join later, and the reserved
    slug keeps a workspace and a project of the same name apart.
14. **Workspace folder (7.5):** per box, default `~/workspaces/<slug>`, generated files and URL clones
    only; description a column and goals a workspace document; no `workspace.toml`; a lone project has
    no folder or components in version one. **Recommended: accept.** `R-ID-3` keeps domain data in
    Postgres.
15. **Workspace requirement keys (7.7):** `W-<AREA>-<N>`; `.n` nesting only within an owner, a nested
    child storing `number` NULL and a `child_number`, with `UNIQUE (project_id, key)` on the stored key;
    a project requirement refining a workspace one keeps its own key; no shared `R-n`. **Recommended:
    accept.** It is `R-ENT-14`'s "per owner and area" applied as written, and leaves the existing unique
    and area counters untouched.
16. **Owner documents and their search (7.5):** `project_id` on every document, `item_id` nullable;
    written by a person or an accepted draft; MCP stays item-only; P4 indexes item-less documents as
    their own points and moves the collection to `v4`. **Recommended: accept.** Without the new point
    owner, `R-STO-8`'s workspace documents are never found; a separate `v4` keeps P1 free of P4's work.
17. **Attachments (7.8):** an item or a document version owns one; metadata and bytes apart; `project_id`
    pinned to the owner's by composite keys; 10 MiB default, 64 MiB ceiling; text scrubbed and
    fail-closed; a binary only from a person, stored as given, with `R-ID-7` amended. **Recommended:
    accept.** Refusing binaries would refuse `R-ENT-12`'s own example, images.
18. **Links (7.6):** `parent` single and acyclic; close-out records `duplicates`/`supersedes` from the
    person's choice; `implements` as defined; document and attachment targets in their own table, pinning
    a document version. **Recommended: accept.** Item-to-item semantics stay untouched.
19. **Spawn order and new blockers (§9):** the project-type table is its own small spawn (P3) so the
    workspace scope (P4) waits on nothing else; MOD-136 gains P4 as a blocker; MOD-114 (3) waits on P4
    and P7. **Recommended: accept.** MOD-136, MOD-116, MOD-112 (4) and MOD-114 (3) then wait only for the
    work they use, not for folders or per-type seeds.
20. **Requirement edits (§10).** **Recommended: accept as written, the wording open to your edits.**

## 9. Proposed spawns

Proposed, not minted; ordering is by dependency.

| P | Subject | Scope | Blocked on | Requirements |
|---|---|---|---|---|
| P1 | Item status reduced to open, blocked, closed | 3-value `Status`; `ItemState` and `latest_run_status` (cancelled runs skipped); readiness and "released" rule as "latest graph run `done`" (MOD-133 adds landed); `uq_run_item_live` and the `create_run` check; new close-out law and `default_for` from the run's `graph_snapshot`; orchestrator (7 mirror calls dropped), store (Pg, Mem, cache), waiting candidates, queue classification; Qdrant `v3`; MCP enums; prompt upstream word; TUI status table, Backlog facet and grouping; fixtures and snapshots; the pre-flight count and the migration of 7.10 row 1, unexplained rows included. | none; MOD-135 lands after it or with it | R-ENT-8, R-TUI-1, R-TUI-2, R-TUI-11, R-STO-8, R-ORCH-6, R-MCP-2 |
| P2 | Plain-folder repos and repo-less runs | `repo.vcs` with registration checks; `needs_repo` and the snapshot refusals; isolation resolved per repo (`worktree` copies a folder, `shared_serialized` refuses one) with `prepare` partitioning the scope; folder `copy` with manifest copy-back and `folder_conflict`; no git probe for empty scopes; `isolation_path` = session folder; chat fallback; `touched_paths` refusal for repo-less projects; cwd → project via `--project` or workspace folder; clone and driven-target refusals for folders. | none; lands with MOD-106 | R-ENT-3, R-ENT-4, R-BOX-4, R-ORCH-8 |
| P3 | Project type table | `project_type` with its six built-in rows (no recipes), `project.type` (existing projects `software`), the `coding` flag; types listed read-only in Settings. Small. | none | R-ID-1, R-ENT-3, R-TUI-8 |
| P4 | Workspace scope: items, kinds, documents, folder | scope row (type `workspace`, slug `@<workspace-slug>`) and its visibility predicate, bootstrap creation; its secret binding on its own columns; workspace seed (WS kind, plan → review); prefix collision rule and qualified keys; cross-workspace `propose_link`; `document.project_id` and nullable `item_id` with owner documents in the TUI; `workspace_box_path.folder_path` and its default; goals as a document; cache and Backlog workspace group; search: `Owner::Document` points, the owner-document indexer pass, collection `v4`, MOD-112's workspace scope. | P3 | R-ENT-2, R-ENT-5, R-ENT-7, R-ENT-12, R-ENT-16, R-STO-3, R-STO-8, R-MCP-1 |
| P5 | Per-type seeds and the type picker | per-type kinds, graphs, phases, `needs_repo`, default isolation (`settings.default_isolation`) and templates; new template bodies; type picker in the new-project form; fixture strides. | P2, P3 | R-ENT-3, R-ENT-6, R-ORCH-8, R-TUI-8 |
| P6 | Workspace requirements and nested keys | `W-<AREA>-<N>` minting in scope rows; stored keys with `UNIQUE (project_id, key)`; `refines_id` with the cross-owner rule; `child_number`, nullable `number` and the child counter for `.n`; coverage derived from splits. | P4 | R-ENT-14, R-ENT-15 |
| P7 | Link kinds and links to documents and attachments | widened CHECK and `LinkKind`; single parent, cycle and depth checks; parent in the upstream walk; close-out recording `duplicates`/`supersedes`; `item_artifact_link` with version pinning (`to_attachment_id` without its FK); MCP `to_document` and qualified keys; Graph legend and nodes. | P1 | R-ENT-9, R-TUI-5, R-MCP-2 |

Dependencies: P1, P2 and P3 start unblocked; P4 waits only on P3; P5 on P2 and P3; P6 on P4; P7 on P1.

Effects on existing items, applied at acceptance: **MOD-106** gains archiving, the deferred trigger and
the backfill (7.3), the live-name partial unique, and lands with P2; **MOD-114** (3) waits on P4 and P7
with 7.8 as its specification (store, cap, scrub posture, the composite FKs and the Attachments sub-tab),
and its migration adds `item_artifact_link.to_attachment_id`'s FK; **MOD-135** builds on P1's close-out
law and adds the `CloseOut` candidates; **MOD-136 gains a new blocker, P4** (HANDOFF blocks it only on
MOD-135 and MOD-115 today): its workspace coordinator runs belong to the scope row instead of a nullable
`run.project_id`, but they still have no item, so it keeps its `ToolScope` and `WaitingRow` variants
without an item, and `uq_run_item_live` (graph runs only) leaves its `coordinator` kind alone;
**MOD-116** is unblocked by P4 for the folder, and its new-project sources map to `vcs` as follows: an
existing folder registers as `git` when it opens as a repository and `folder` otherwise, the person
confirming; a git URL clones into the workspace folder as `git`; a new repo is a new directory under the
workspace folder, `git init` with one empty commit so HEAD resolves, done before it is registered, so
`R-ID-4` (no htui files in a managed repo) holds and no file is written into its tree; the workspace
lifecycle states (open, background, stopping, stopped) are MOD-116's and MOD-125's, on the `workspace`
row or derived from runs, never on the scope row, and this item adds no column for them; **MOD-118**
waits on P6 instead of ANA-35, and its text's "`R-2` → `R-2.1`" becomes "`R-TUI-2` → `R-TUI-2.1`", with
workspace ones `W-<AREA>-<N>`; the findings' `W-n` (`docs/ui-redesign-findings-2026-10-15.md:265,
657-658`) gets a note pointing at 7.7; **MOD-112** (4) waits on P4; **MOD-119** consumes P1's
`ItemState`; **MOD-8** follows P1, P4 and P7 and maps `[ ]`/`[x]` to `open`/`closed`, "Blocked on X" to
`blocked_by`, "from X" to `origin` and "absorbs" to `parent`; it writes `closed` and the resolution
directly through an importer-only store path, not `close_out`, since imported items have no finished
run; an `[x]` without a `DECISIONS.md` row gets `done` (`concluded` when the mapped kind's graph ends in
a `verdict` phase); and `docs/REQUIREMENTS.md` imports into the project's own requirements (`R-…`), not
a workspace scope row; **MOD-133** reads `repo.vcs` for "`local` only", and extends readiness with the
landed check at every site (Pg, MemStore, and the cache, which mirrors the integration records);
**ANA-33** owns the recipe format user-defined types will store; **ANA-39** and **MOD-128** read
`project_type.coding` (P3); **ANA-42** inherits "a folder runs on this box only".

## 10. Requirement edits owed

All maintainer-gated; `docs/REQUIREMENTS.md` is not edited by this item.

**10.1 `R-ENT-2`**, replace the clause "and every workspace-scoped feature falls back to … the current
project" (`docs/REQUIREMENTS.md:162-163`) with "and the workspace-scoped views and searches cover just
that project", and replace "A workspace has a folder … existing repos stay where they are." with: "A
workspace has a folder on each box (by default `~/workspaces/<name>`) where `htui` writes what it
generates — the compose file (R-ENT-17) and, without Infisical, the environment file (R-SEC-5) — and
where projects cloned from a URL go; existing repos stay where they are. Its description and goals live
in Postgres, the goals as a workspace document (R-ENT-12). A single project with no workspace has no
folder, no components and no workspace-owned items."

**10.2 `R-ENT-3`**, replace from "It has a type" to the end with: "It has a type, chosen from the project
types (software, research, docs, ops, general, and the user's own, R-TUI-8), whose kinds, graphs and
templates are copied into it once when it is created; changing a type never re-seeds an existing
project. It has zero or more repos; when it has any, exactly one is primary, which may be a folder;
removing a repo archives it, and the primary passes to another. A phase marked as needing a repo
refuses to start in a project without one." Add: "A workspace owns its own key space, kinds, graphs,
documents and requirements in the same way (R-ENT-16)."

**10.3 `R-ENT-4`**, add: "A plain folder cannot be cloned onto another box, pushed or run on a target
other than the box that holds it. `copy` on a folder brings back only files the folder has not changed
since the copy and parks on the rest; a `worktree` step copies a folder instead, and `shared_serialized`
refuses one (R-ORCH-8)."

**10.4 `R-ENT-5`**, replace "unique per project" with "unique per owner — a project, or a workspace
(R-ENT-16)".

**10.5 `R-ENT-6`**, replace "a research, docs or ops project gets its own kinds (for example …), settled
by ANA-35" with the seed table of 7.4 (research: RES, Q; docs: DOC; ops: OPS, INC; general: TASK; a
workspace: WS), "the prefixes editable per project like any kind".

**10.6 `R-ENT-7`**, replace with: "Item keys are minted from a per-owner, per-prefix sequence held by
Postgres; the owner is a project or a workspace. Never reused: each owner has exactly one counter per
prefix, and Postgres is its only writer. Keys of different owners may coincide and are qualified by the
owner's slug where ambiguous. Creating an item in a project mirrored from a server, while that server is
unreachable, is not supported (see R-STO-4)."

**10.7 `R-ENT-8`**, apply ANA-34 §10.4's transition sentence and add: "`blocked` means a person must
clear something and always carries a note; a dependency only makes an item not ready. What a person sees
— queued, running, needs you, failed, done (finished, close-out waiting), open, blocked, closed — is
derived from the status and the item's latest run, a cancelled run changing nothing. A `blocked_by` link
stops blocking when its target is closed or its latest run finished and its work landed. Resolutions
`done` and `concluded` require a finished run; the importer (R-LATER-3) records closed items as it finds
them. Reworking a finished item means starting a new run on it."

**10.8 `R-ENT-9`**, add: "An item has at most one `parent` and parents never form a cycle. A link to a
document names the version it was made against. There is no manual link action, except that a person's
close-out as `duplicate` or `superseded` records the link to the item they name."

**10.9 `R-ENT-12`**, replace the attachment sentence with: "Items and document versions may carry
attachments (images, logs, other files), held in Postgres in version one under a size cap, 10 MiB by
default and never above 64 MiB (an object store is R-LATER-10). Text attachments are scrubbed (R-ID-7); a
binary attachment is accepted only from a person, who is told it is stored as given."

**10.10 `R-ENT-14`**, replace "Workspace requirements are keyed `W-…` (ANA-35 settles the exact form)"
with "Workspace requirements are keyed `W-<AREA>-<N>`. Nesting with `.n` stays within one owner; a project
requirement that refines a workspace one keeps its own key".

**10.11 `R-ENT-16`**, replace "design in ANA-35." with "amended by ANA-35 (repo-less workspace items).",
and add before it: "A workspace item has no repos: it plans, decides and splits, and code work goes to
child items in member projects, linked by `parent` (R-ENT-9)."

**10.12 `R-ID-7`**, add: "A binary attachment a person adds is the one exception: it is stored as given,
marked unscrubbed, and never accepted from an agent (R-ENT-12)."

**10.13 `R-STO-3`**, replace "a read-only cache of the projects it has opened" with "a read-only cache of
the projects and workspaces it has opened, workspace-owned rows included (R-ENT-16)", and add "attachment
metadata (not their contents)" to the contents.

**10.14 `R-ORCH-8`**, replace "`worktree` (default, one git worktree per session)" with "`worktree`
(one git worktree per session)", and add: "The default mode is set by the project type: `worktree` for
software projects, `local` for the others (R-ENT-3). On a plain folder a `worktree` step copies the
folder instead, and `shared_serialized` refuses it (R-ENT-4)."

**10.15 `R-TUI-2`**, replace "done and closed items stay hidden until shown" with "items done (finished,
close-out waiting) and closed stay hidden until shown", and "filters by status" with "filters by state
(R-ENT-8)".

**10.16 `R-TUI-11`**, replace "lists every run in the active workspace that waits on a person" with
"lists every run, or item, in the active workspace that waits on a person", and "a blocked run" with "a
blocked item".

**10.17 `CONCEPTS.md`** (not a requirement; noted for the same session): "`project` owns item keys,
kinds, step graphs and secrets" gains "and so does a workspace, for its own work"; the status line gains
"what a person sees is derived from the latest run".

## 11. Not verified

- The auto-generated constraint names `item_status_check`, `item_link_kind_check`,
  `document_item_id_kind_version_key` and `repo_project_id_name_key` follow Postgres's naming rule; confirm with `\d` on a live database
  before writing `DROP CONSTRAINT`.
- Live row counts per item status, how many `done`/`failed` items no latest graph run explains (the
  7.1 pre-flight count), whether any item sits in another removed state without a live run, and whether
  any item has two live graph runs today (which would make `uq_run_item_live` fail to build). No
  database was reachable from the sandbox; P1's plan runs the count first.
- Whether old chat rows written before `start_chat_run` took its present shape carry an `item_id`; the
  current path inserts `NULL` (§2.2), and the index is scoped to graph runs either way.
- `ALTER COLUMN … DROP EXPRESSION` (Postgres 13+) and `CREATE CONSTRAINT TRIGGER … DEFERRABLE` are
  recalled, not checked against the vendored sqlx or a running server; no constraint trigger exists in the
  schema yet.
- Whether project slug validation already forbids `@`, which the scope row's `@<workspace-slug>` relies
  on; if it does not, P4 adds the refusal.
- How `GixIsolator::prepare`'s `Prepared` result combines trees of different modes; the per-repo
  `RunStepTree.mode` exists, but the partitioned dispatch P2 needs was not sketched against
  `prepare_worktree`/`prepare_copy`'s return shapes.
- The owner-document point's exact payload and point id scheme in Qdrant, and whether
  `ensure_collection` creates every payload index on a fresh `v4`; read only as far as `Owner` and the
  indexer loop.
- Whether `requirement.area_code` values all match the key-shape CHECK sketched for P6, and whether the
  cache mirror's `requirement` table declares `number NOT NULL` (the version bump rebuilds it either
  way).
- The bodies of `GixIsolator::reset` and `release`, the recovery sweep and `run_step.opening` were not
  read for repo-less behaviour; `htui-orch/src/fake.rs`'s handling of an empty scope was not checked.
- Whether `copy_max_total_bytes` and the `copy_exclude` defaults suit non-code folders.
- How ANA-42's targets treat repo-less projects beyond "nothing to clone"; ANA-42's §8 is still open,
  so the folder refusal for driven targets rests on its draft.
- The exact `.sqlx` re-hash counts (about 19 for item-status literals, about 45 if `item` gains columns,
  11 for `document`) and the exact list of affected snapshots; both are grep estimates, and a full insta
  run is the real count.
- Prior-art rows marked as search summaries by the prior-art reader (Jira categories, Linear relation
  pages, Devin, OpenHands, Gitea and Redmine caps) were not opened; the 10 MiB and 64 MiB figures are a
  judgement from the surveyed range and sqlx's in-memory reads, not a sourced prescription.
- Whether ANA-33 will keep a phase-level `needs_repo` once graphs branch and loop; it is placed on the
  phase here and moves with whatever ANA-33 calls a step.
