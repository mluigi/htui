# MOD-116 — Workspaces tab and workspace lifecycle

> Routed as **PRD** by `/handoff-run` (C2 and C4 fired; C2 at low confidence), sandbox run `hr/MOD-116`,
> 2026-11-15. Ultracode accepted for the implement phase. Origin: UI redesign review,
> `docs/ui-redesign-findings-2026-10-15.md` §4.1. Requirements: `R-TUI-14`, `R-ENT-2`, `R-ORCH-16`,
> `R-TUI-18`, `R-ID-4`. PRD gate: maintainer took the recommended answer to Q1-Q4, 2026-11-15.

## Problem

`htui` holds one workspace on screen and offers no view of the others. Runs of a workspace the person
switched away from keep walking, but nothing says so: the header names only the open workspace, and the
leave dialog counts only that one. MOD-125 made runs stoppable and resumable per workspace, but only
through `htui stop`/`htui resume` on the command line. Creating a workspace or a project is buried in
Settings › Hierarchy, does not create the workspace's folder, and cannot clone a repository or start a new
one. `R-TUI-14` asks for one tab that lists every workspace with its state, opens, stops and resumes them,
and creates workspaces and projects. `R-TUI-18` asks for a leave dialog that lists every live workspace.

## Evidence

Read at `e4d609b3` (branch `hr/MOD-116`). Survey: `/tmp/mod-116-survey.md` (sandbox-local).

- **Maintainer design decision, not an incident.** `R-TUI-14` and `R-TUI-18` were added on 2026-10-17 in
  the UI review (`docs/REQUIREMENTS.md:833`, `:861`; findings §4.1 `docs/ui-redesign-findings-2026-10-15.md:153-199`).
  No user report stands behind it. Real use is an **assumption — needs validation by using the tab on the
  maintainer's own workspaces**.
- **No Workspaces tab.** Tabs are `TabId`s in registration order (`crates/htui/src/ui/tabs/registry.rs`).
  `register_all` (`crates/htui/src/app/mod.rs:70`) builds Backlog, Skills, Requirements, Settings, Chat, Logs.
- **The switcher only re-scopes.** The `w` overlay (`crates/htui/src/ui/overlay/workspace_switcher.rs`) lists
  `WorkspaceSummary` rows. Enter sends `Action::SetScope`. `App::set_scope` (`crates/htui/src/app/update.rs:438`)
  drops every tab's cached rows and asks for the new waiting counts. Nothing persists the last open workspace:
  startup picks `workspaces.first()` (`update.rs:744-760`).
- **Runs do not depend on the open workspace.** The in-process executor runs inside the store worker, once
  per process (`store_worker.rs:3253`). It walks every run claimed by this box (`htui-worker/src/runtime.rs:2253`,
  `claim_scan` at `:2844`). `set_scope` never touches it. So "background" already holds. Nothing shows it:
  no "+N in background" chip exists (`crates/htui/src/ui/header.rs`).
- **The lifecycle can be derived.** MOD-125 ships `stop_workspace`, `resume_workspace`, `workspace_stop`,
  `workspace_live_runs` (`crates/htui-core/src/store/traits.rs:1711-1742`) and `WorkspaceStopState::of`
  (`crates/htui-core/src/model/stop.rs`). `htui stop`'s `Report`/`RunLine`/`RunResume` and `report()`
  (`crates/htui/src/stop_cmd.rs:93-188`, `:473`) already describe each run and how it resumes. The TUI has no
  `StoreRequest` for stop or resume. ANA-35 §9 puts the states on the `workspace` row or derives them from runs,
  never on the scope row.
- **The leave dialog counts one workspace.** It counts the open workspace and has no "stop first" choice
  (`crates/htui/src/ui/overlay/leave.rs:7-10`: "until MOD-116/MOD-125 it would do what leaving does").
- **Creation lives in Settings › Hierarchy.** It has `N`, the workspace `e` edit, the folder picker and delete
  (`crates/htui/src/ui/tabs/settings/hierarchy.rs:106-122`, `:151-168`, `:283`). Its store verbs are
  `create_workspace`, `update_workspace`, `upsert_workspace_box_path`, `create_project`, `create_repo` and
  `delete_workspace` (`traits.rs:835-981`, `:1526`). No folder is created on disk. The default folder is
  `workspace_folder(slug, row, home)` (`htui-core/src/model/hierarchy.rs:289`).
- **Git verbs are partial.** `isolate::git::open`/`head` and `htui_orch::infer` read repositories. `isolate::git::Cli::run`
  spawns the `git` binary. There is no clone helper. `git init` with an empty commit exists only in the testkit
  (`crates/htui-orch/src/isolate/git.rs:1776`, `:377`, `:2554-2700`). The repo `vcs` kind (`git|folder`) is not
  in code (MOD-145).
- **Targets are on the tree.** MOD-130 M1 (`0040_execution_targets`) stores `target` rows, the Settings
  default and `set_project_target`. A scope row's `target_id` is the workspace level. MOD-130's PRD leaves the
  workspace default with `T` to MOD-116 (`.claude/prds/mod-130-execution-targets.prd.md:199`).
- **No coordinator chat exists.** "coordinator" appears nowhere under `crates/`. A chat is a run/step pair
  minted by `ChatStart` for one project (`store_worker.rs:226`).

## Users

- **Primary**: the maintainer, with several workspaces on one box (often runs going in more than one), who
  wants to see what each is doing, move between them without disturbing runs, stop or resume a whole
  workspace, and set up a new workspace or project without leaving the TUI.
- **Not for**: components (MOD-117), requirements in Workspaces (MOD-118), the memory sub-tab (`R-ENT-13`'s
  view), coordinator chats (MOD-136/MOD-121), Settings' redesign (MOD-123).

## Hypothesis

We believe **a Workspaces tab that shows every workspace's derived state and lets the person open, stop,
resume and create workspaces and projects** will **replace the switcher overlay, the CLI stop verbs and
Settings' workspace editing as the way a person runs several workspaces**.
We'll know we're right when **every workspace's state on the tab matches its runs and stop record on both
stores, opening a workspace leaves every other workspace's runs walking, a stop or resume from the tab has
the same effect as `htui stop`/`htui resume`, and a new project from a URL or a new repo is registered with
a HEAD that resolves and no htui file in its tree**.

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| State is derived correctly | open / background · n running / stopping / stopped (· n held) read right for every combination of open, live runs and stop row | unit table over the derivation + store conformance for the per-workspace read (MemStore and PgStore) |
| Opening keeps runs going | after `o` on another workspace, a run of the first keeps walking and the header shows `+1 in background` | testkit test with the run runtime |
| Stop parity with the CLI | the tab's confirmation lists the same runs and resume lines as `htui stop`'s preview; the writes are the same verbs | shared report code; test over both surfaces |
| Leave dialog lists every live workspace | one row per workspace with live runs, counts per workspace; "stop gracefully first" writes a graceful stop for each and waits for them to settle | testkit tests |
| New repo is clean | a new-repo project has one empty commit, HEAD resolves, and no file in its tree | integration test over a temp dir |
| Clone lands in the folder | a git-URL project clones into `<workspace folder>/projects/<name>` and registers as a repo with that path | integration test against a local bare repo |
| Nothing lost in the move | everything the switcher and Hierarchy's workspace row did is reachable from the tab | snapshot + key catalogue tests |

## Scope

**MVP**: the Workspaces tab (first in the strip; `w` jumps to it), listing every workspace with its
derived state, the workspace overview and project overviews; `o` opens a workspace (the current one goes to
the background with its runs); the header's `+N in background` chip; `s` graceful stop, `s` on a stopping
workspace a forced stop, `r` resume, each confirmed and listing what happens to each run; the leave dialog
listing every live workspace with "stop gracefully first"; `N` "new what?" (workspace, project); the
new-workspace form (name, folder following the name, description, a preview; creates the folder); the
new-project form (existing git folder, git URL, new repo); editing and deleting a workspace; the
workspace's default target with `T`.

Decisions taken at the PRD gate (maintainer, 2026-11-15):

- **Q1 — coordinator chats are deferred to MOD-136/MOD-121.** The forms create no chat. The overview shows no
  coordinator line until those items land.
- **Q2 — a plain folder is refused until MOD-145.** "Existing folder" must open as a git repository. A plain
  folder is refused with a message naming MOD-145. Git URL (clone into the workspace folder) and new repo
  (`git init` plus one empty commit before registering, `R-ID-4`) ship in full.
- **Q3 — the workspace row moves.** Creating, editing, the folder and deleting a workspace move to
  Workspaces; Settings › Hierarchy keeps the open workspace's projects, repos and documents until MOD-123.
- **Q4 — an idle workspace reads `○ stopped`.** A workspace that is not open, has no live runs and has no stop
  record shows as stopped. No `workspace_stop` row is written for it, so runs can start there without a
  resume. A workspace whose stop record holds runs reads `○ stopped · n held`.

Decided from the item and requirements (no question needed):

- **Lifecycle has no new column.** open is this TUI's scope (per process); background, stopping and stopped
  derive from live runs and the `workspace_stop` row (ANA-35 §9). The store gains a read that answers every
  workspace's counts and stop record in one call, so the list and header do not query per workspace.
- **The leave dialog is MOD-116's** (`R-TUI-18`, moved here from MOD-115 by its M2 D6). Leaving while this TUI
  is the executor still drops its walks, as today (MOD-125 Q5); the dialog keeps saying so.
- **Sub-tab chips** show `overview` only. MOD-117 adds components, MOD-118 requirements, and the memory view
  comes later.
- **The last open workspace persists** in the UI state file, so a restart reopens it (today it opens the
  first workspace).

**Out of scope**

- Components, compose and `.env` (MOD-117) — the preview names the files only as what later items write.
- Requirements in Workspaces and removing the Requirements tab (MOD-118).
- The memory sub-tab (`R-ENT-13`) — a later item.
- Coordinator chats (Q1) — MOD-136/MOD-121.
- Plain-folder repos (Q2) — MOD-145.
- Moving projects and repos out of Settings (Q3) — MOD-123.
- Joining an existing project to a second workspace — no requirement asks for it here.
- Stopping a single run — `R-ORCH-16` is per workspace.

## Delivery Milestones

<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | The tab, states and opening | Workspaces is tab 1; every workspace with its derived state, workspace and project overviews (state, folder, description, goals, projects with counts, default target, held runs); `o` opens and backgrounds; `w` jumps to the tab and the switcher overlay is gone; the header's `+N in background`; the last open workspace persists | pending | — |
| 2 | Stop, resume and leaving | `s`, `s` again and `r` from the tab, each confirmed with a per-run preview shared with `htui stop`; the overview lists held runs and how each resumes; the leave dialog lists every live workspace and offers "stop gracefully first" | pending | — |
| 3 | Creating and editing | `N` chooser; new-workspace form with folder and preview, creating the folder; new-project form (existing git folder, git URL clone, new repo); workspace edit and delete on the tab, removed from Hierarchy; `T` sets the workspace's default target | pending | — |

## Open Questions

- [ ] Where does the per-workspace read live, and does it need an index? TBD — settle in `/plan`, by measuring
  against `workspace_live_runs`'s query.
- [ ] Clone transport: the `git` binary through `isolate::git::Cli` (scrubbed env, timeout, no prompt), or
  `gix` clone. Credentials for a private URL follow the box's own git setup. TBD — settle in `/plan` (M3).
- [ ] What `T` offers when MOD-130's resolver (its M2) is not yet on the tree: show the workspace level as
  set or "inherits the Settings default". TBD — `/plan` (M1/M3), against the tree at that time.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Moving Workspaces to tab 1 shifts every digit and many snapshots | High | Wide, shallow snapshot churn; the person's habits change | Design asked for it (findings §3.1); review every snapshot change; the snapshot-impact memory |
| Clone or init hangs on a prompt or a slow network | Medium | Frozen form | Run on the store worker, `GIT_TERMINAL_PROMPT=0`, a timeout, a cancellable progress line |
| Two TUIs disagree on which workspace is "open" | Medium | Confusing state on one of them | open is per process by design; the others read as background or stopped |
| Removing Hierarchy's workspace row breaks a flow (first start with no workspace) | Medium | No way to create the first workspace | The startup path opens the Workspaces tab's new-workspace form instead of the switcher |
| Parallel items touch the same files (MOD-127, MOD-130 M2+ in wave 1) | Medium | Merge conflicts at collect | Keep store additions additive; note touched files in the plan |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
