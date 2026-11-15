# Plan: MOD-116 M3 — Creating and editing

**Item**: MOD-116 — Workspaces tab and workspace lifecycle
**Source PRD**: `.claude/prds/mod-116-workspaces-tab.prd.md`
**Selected Milestone**: 3 — Creating and editing
**Routing**: routed as PRD by `/handoff-run` (C2, C4); PRD gate answered (Q1-Q4, 2026-11-15). Sandbox run
`hr/MOD-116`, plan written at `b473420a` while the M2 lane (T1 landed, T2/T3 to come) was editing
`crates/htui/src/{stop_report,stop_cmd,workspaces,store_worker}.rs`, `ui/tabs/workspaces/*`,
`keys/{catalogue,views,stack,hint}.rs`, `ui/overlay/leave.rs`, `ui/about.rs` and docs: **line anchors in those files
are approximate (`~`) and M2 symbols are cited by the name M2's plan gives them**; the fact-check runs after M2 lands,
and T2 starts only after it (T1 touches no M2 file and may start at once).
**(fact-check)** M2 landed at `1692b8bb`; the fact-check ran at `d6cd0909` and made every anchor and M2 name exact
against it (see Verified claims). Anchors in `ui/tabs/workspaces/*` stay approximate: an M2 fix round may still touch
those files.
**Requirements**: `R-TUI-14` (creating workspaces and projects from the tab), `R-ENT-2` (the workspace folder, where
clones go), `R-ID-4` (no htui file in a managed repo's tree), `R-NF-3` (no long work on the UI thread)
**Complexity**: Medium-large (three git helpers in `htui-orch`; one new worker module serving five requests, one of
them off the store-worker loop; a chooser, three forms, a delete flow and a target picker on the tab; Settings ›
Hierarchy loses its workspace row's verbs; two new acts, one retired act, two new stacks; no migration, no store
trait change, no `.sqlx`)
**Status**: DRAFT, fact-checked at `d6cd0909` (79 claims: 56 true, 11 false and amended, 12 amended)

## Summary

M1 and M2 let a person see, open, stop and resume workspaces from the Workspaces tab; creating one still means
Settings › Hierarchy, which writes rows and never touches the disk. M3 moves creation and editing to the tab. `N`
opens **"New…"** (workspace · project in the selected workspace). The **new-workspace form** takes a name, a slug
that follows the name, a folder that follows the slug (`~/workspaces/<slug>`) until edited, and a description, with
a preview of the folder and what goes there; creating writes the workspace and its scope row, creates the folder
(an existing directory is used as it is) and records a folder row only when the folder is not the default. The new
workspace reads `○ stopped` and is not opened, unless the shell has no listed workspace open. The **new-project
form** takes a name, slug, description and a source: an **existing folder** that must open as a git repository
(registered where it is, remote from `origin`, branch from `HEAD`; a plain folder is refused naming MOD-145), a
**git URL** cloned with the `git` binary into `<workspace folder>/projects/<slug>`, or a **new repo** created there
with `git init` and one empty commit before it is registered. Clone and init run off the UI thread and off the store
worker's loop, with a progress line, a bound, and a readable failure that leaves nothing half-registered. On a
workspace's rows `e` edits its name, slug, description and folder, `d` deletes it behind Hierarchy's counts and
typed slug, and `T` picks its default target (written on the scope row). Settings › Hierarchy keeps the open
workspace's projects, repos and documents and loses the workspace row's create, edit, folder and delete.

## Shared decisions

- **D1 — "New…" offers workspace and project only.** `N` (`workspaces.new`) opens a small chooser over the tab (kit
  `Modal` + kit list, the path picker's shape): `workspace`, then `project in <name>` for the selected row's
  workspace (absent when no workspace is listed). Component and requirement (findings §4.1) are **left out**, not
  shown disabled: a row nobody can choose is noise, and MOD-117/MOD-118 add theirs with their sub-tabs. With no
  workspace listed, `N` opens the new-workspace form directly. Keys: stack `views::WORKSPACES_PICK` (D14): `j`/`k`
  move, `y` or `Enter` (M2's `[workspaces]` view default on `confirm.yes`) chooses, `n`/`Esc` closes.
- **D2 — Slugs, folders and what follows what (pure, `htui_core::model::hierarchy`).** `slug_of(name)`: ASCII
  letters lowercased and digits kept, every other run of characters one `-`, leading and trailing `-` trimmed, cut
  to 40 bytes (then trailing `-` trimmed again); `""` when nothing is left (the form's required-field refusal).
  `PROJECTS_DIR = "projects"`; `project_folder(workspace_folder, slug) -> Option<PathBuf>` is `<folder>/projects/<slug>`
  under `workspace_folder`'s one-plain-name rule (factored into a private `one_name(slug)` both share; MOD-147 M1
  R1). In the forms the slug follows the name until the slug field is edited; the workspace folder follows the slug
  until the folder field is edited; a project's location always follows its slug (read-only).
- **D3 — Paths on the tab are shown under `~` and sent absolute.** The tab reads the home directory once in `new()`
  (`dirs::home_dir()`, as `HierarchySection::new`, `settings/hierarchy.rs:460`) and takes `with_home(Option<&Path>)`
  for tests. It shows a path under home as `~/…`, expands a typed leading `~` or `~/` with that home, and sends only
  absolute paths; the worker refuses a relative one in `RootRefusal::Relative`'s words. **Every integration test that
  creates a folder gives the tab a temp home** (`WorkspacesTab::new().with_home(Some(tmp.path()))`) so the default
  folder lands under the temp dir, never the real `~/workspaces`. No I/O on the UI side (`R-NF-3`).
  **(fact-check)** The tab's temp home does not reach the worker in integration tests: `Harness` serves every request
  through `store_worker::serve` (`testkit.rs:402`, `:422`), so `workspace_setup::serve` runs with the real
  `dirs::home_dir()`. There, D4 step 5 compares against the real-home default (a new workspace in a harness test
  always gets a folder row) and D6's `workspace_root` resolves a workspace **without** a folder row under the real
  `~/workspaces`. So every harness case that creates a project does it in a workspace whose folder row points under
  its temp dir, and the M1 module's `demo()` (folder rows `/srv/graphics`, `/srv/platform`,
  `tests/it/workspaces_tab.rs:89-107`) is copied with those rows pointed at existing temp folders, never `/srv`.
  "No folder row for a default folder" is asserted only at the worker level (`serve_with_home`, T2).
- **D4 — Creating a workspace (worker, `crate::workspace_setup`).** In order, nothing written before a refusal:
  1. the writer, else `Unreachable(DATABASE_UNREACHABLE)`;
  2. the folder, under `spawn_blocking` (`prepare_folder`): absolute, else refused; a missing path is created with
     its parents (`made = true`); an existing directory is used as it is and nothing in it is touched; a
     non-directory is refused (`NotADirectory`'s words); a directory that opens as a git repository
     (`isolate::git::repo_facts` answers `Some`) is refused: "`<path>` is a git repository; htui writes the
     workspace's files into this folder, and never into a repository (R-ID-4)"; then canonicalised (`canonical`);
  3. `this_box` is needed only when a row will be written (step 5); a box with no row is refused here, before the
     workspace row, in `box_id`'s words;
  4. `create_workspace` (its scope row as today); a refusal (a taken slug) removes the folder only when `made` and
     still empty (`remove_dir`, the leaf alone), then answers the refusal;
  5. the folder row only when the canonical folder differs from the canonical default `workspace_folder(slug, None,
     home)` (a default that does not resolve, or no home, always differs);
  6. reply `WorkspaceCreated { workspace: WorkspaceSummary, folder: String, made: bool }` (summary from
     `hierarchy::snapshot(..).summary()`).
  The new workspace reads `○ stopped` (PRD Q4: no live run, no stop row; nothing written) and **is not opened**,
  with one exception: when the shell's scope names no workspace the tab lists (first start, or the open one was just
  deleted, D10), the tab emits `Action::SetScope` with it so the shell is never left without a workspace. The
  cursor lands on the new group row. No coordinator chat (PRD Q1).
- **D5 — Editing a workspace.** One form: name, slug, description, folder, prefilled from the held overview
  (`overview.tree.workspace`, `overview.folder`); with no current overview for that workspace the tab forces one read
  first (M2's pending-ask mechanism, a new `Asked::Edit`). The folder field does not follow the slug here. Worker
  (`EditWorkspace`): the folder is prepared first as in D4 (created if missing; **never moved**: the old folder and
  what it holds stay where they are, and the form says so under the field); then `update_workspace(id, expected,
  patch)` (CAS on `updated_at`, `UpdateWorkspace`'s rule, `hierarchy.rs:~303-311`); on `Applied`, the folder row is
  upserted when a row exists or the folder differs from the **new** slug's default, and nothing is written when no
  row exists and the folder equals that default (a slug change with no row therefore records the old folder, so the
  folder never moves on its own); on `Stale`, nothing more is written and the reply carries the current row: the form
  keeps its text, takes the new token and says `changed_elsewhere` (`settings/mod.rs`), so a second `Enter` saves over
  it. The folder made for a stale edit stays (an empty directory; said in the risk table).
- **D6 — Creating a project (worker).** Common steps, in order, before any disk work: the writer; `this_box`
  (`box_id`, a repo path row is always written); the workspace's tree (`hierarchy::snapshot`, `None` → `NotFound`);
  `project_slug_refusal(slug)` (`traits.rs:~4166`); a slug already used by a project any workspace lists
  (`backend.workspaces()`) is refused ("project slug `x` is taken"; an unlinked project's slug is only caught by the
  store at step 4); the workspace folder `hierarchy::workspace_root(&tree, home)` (`None` → refused "this workspace
  has no folder on this box"). Then per source:
  - **Existing folder**: `canonical(path)`; `repo_facts(path)` under `spawn_blocking`: `Ok(None)` → refused "`<path>`
    is not a git repository; plain folders come with MOD-145", a bare repository refused ("register a checkout, not
    a bare repository"); repo name = the folder's file name; default branch = `HEAD`'s branch, else `main`; remote =
    `origin`'s fetch URL through `htui_orch::infer::redact_remote`, else none; path = the canonical folder. Nothing is
    written on disk. **(fact-check)** `hierarchy::canonical` turns a `RootRefusal` into `StoreError::Constraint`,
    which reaches the person as `new_project: constraint violated: …` (`hierarchy.rs:569-588`); D13 wants a sentence,
    so this step calls `htui_core::root_path::canonical_root` under `spawn_blocking` itself (or maps `canonical`'s
    `Constraint(reason)`) and answers `SetupRefused` with the `RootRefusal`'s words. `gix::open` also opens a path that
    ends in `.git` as the git directory (`gix-0.87.1/src/open/repository.rs:74-92`): take the name and the registered
    path from the opened repository's work directory, not from the typed path.
  - **Git URL**: the URL trimmed, non-empty; dest = `project_folder(workspace folder, slug)` (`None` → refused); dest
    must not exist ("`<dest>` already exists; nothing was cloned", the path kept); `<folder>/projects` created with
    its parents; `Cli::clone_repo(url, dest)`; a failure removes dest (it did not exist before) and answers git's
    redacted line; repo name = slug; branch = the clone's `HEAD` branch, else `main`; remote = `redact_remote(url)`
    (the clone's own `origin` keeps what was typed: the box's git setup, not htui's). **(fact-check)** A clone can
    exit 0 with an unborn `HEAD` (probed with git 2.43.0: an empty remote, "You appear to have cloned an empty
    repository", or a remote whose `HEAD` names a branch it lacks, "remote HEAD refers to nonexistent ref"), so the
    branch comes from `repo_facts(dest).branch` and `head` is never a post-condition of the clone. `IsolateError`'s
    `Display` prefixes `isolation refused: ` (`isolate.rs:51-52`): the worker answers the `Refused(reason)` string
    itself, never `err.to_string()`.
  - **New repo**: dest as above, must not exist; created; `Cli::init_with_empty_commit(dest, "main")`; a failure
    removes dest; remote none; branch `main`. Nothing but `.git` is written (`R-ID-4`).
  Registration: `create_project` and its link at max + 1 (factored out of `CreateProject`'s arm,
  `hierarchy.rs:~330-376`, as `pub(crate) async fn create_linked_project`), `create_repo` (`is_primary: true`),
  `upsert_repo_box_path` (canonical dest). A failure after `create_project` deletes that project
  (`delete_project`, then `rebuild_mirror` as `DeleteProject` does) and removes dest when this request made it; the
  reply is the failure. Reply `ProjectCreated { workspace, project: ProjectRef, repo_path: String }`. No coordinator
  chat (PRD Q1).
- **D7 — Off the loop, bounded, with progress.** `NewProject` is served **from its own task** in the store-worker
  loop, as `ListDir` is (`store_worker.rs:3895-3906`: clone the request's fields, `seq`, `origin` and `tx`, spawn,
  send the `ReplyEnvelope`, `continue`), plus an inline arm in `try_serve` for `serve`'s callers (the harness,
  `--demo`), as `ListDir` keeps (`:2873`). **(fact-check)** `ListDir`'s task touches no backend; this one also clones
  `backend` (`htui_store::Backend` is `Clone`, `htui-store/src/backend.rs:64`) and answers a failure with
  `failed("new_project", &err)` as `ListDir` does with `LIST_DIR`; the loop's `lost_the_store`/`go_offline` check
  (`:3913-3923`) does not see the task's errors. The other four requests are inline (one stat or `mkdir` under
  `spawn_blocking`, as `canonical` does). The form shows a progress line while the request is out: `cloning <URL with
  userinfo dropped> into <dest>…`, `creating <dest> with git init…` or `registering <path>…`, with whole seconds
  counted by the tab's `on_refresh` (once a second; deterministic in tests, no clock read). `Esc` while it is out
  closes the form and says "the project is still being created; you are told when it lands" in the told box; the
  reply is still taken (a told box with the outcome, D13). One `NewProject` out per tab at a time (the shell's
  staleness gate would drop the first reply): `N` → project is told "a project is still being created". Bounds:
  `CLONE_TIMEOUT` (10 min) on the clone, `NETWORK_ENV`'s low-speed abort, ssh `BatchMode=yes` and
  `ConnectTimeout=20`, `GIT_TERMINAL_PROMPT=0`. **No cancel key in M3** (OQ1, settled by the maintainer
  2026-11-15: bounded clone, `Esc` closes the form while the clone finishes in the background; the PRD's risk row
  says so). **(fact-check)** `on_refresh` reaches only the active tab (`App::refresh_active_tab`,
  `app/update.rs:404-410`), so the seconds counter pauses while another tab is in front (acceptable: the count is a
  sign of life, not a clock).
- **D8 — The git helpers (`htui_orch::isolate::git`).** Beside the network verbs, on `Cli` (whose `run` scrubs
  `SCRUBBED_ENV` and `HTUI_OWN_ENV`, sets `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0` and kills the group on timeout):
  - `clone_repo(url, dest)`: argv `-c protocol.ext.allow=never clone --quiet -- <url> <dest>` (the `--` keeps a URL
    spelled like an option a URL), run in `dest`'s parent, under `network_env(self.network_ssh(parent))`, with
    `clone_budget()` (`CLONE_TIMEOUT`, or this `Cli`'s own budget when a test shortened it below `VERB_TIMEOUT`).
    **(fact-check)** not "as `ls_remote_budget`": that is `self.budget.min(LS_REMOTE_TIMEOUT)` (`git.rs:1224-1226`),
    and `self.budget` is `VERB_TIMEOUT` (120 s) in production (`git.rs:293`), so the same shape would cap every clone
    at two minutes. The rule is `if self.budget < VERB_TIMEOUT { self.budget } else { CLONE_TIMEOUT }`.
    Exit ≠ 0 → `IsolateError::Refused(format!("git clone: {}", redact_urls(&network_failure(&exited))))`.
  - `init_with_empty_commit(dir, branch) -> String` (the commit's hex): `init --quiet --initial-branch=<branch>` in
    `dir`, then `-c core.hooksPath=/dev/null -c commit.gpgsign=false commit --allow-empty --no-verify --quiet -m
    "Initial commit"` under `IDENTITY_ENV` (htui's identity; no signing prompt, no user hook), then `head(dir)` under
    `blocking`. **(fact-check)** `--no-verify` bypasses only `pre-commit` and `commit-msg`; `prepare-commit-msg` (which
    can fail the commit) and `post-commit` still run from a global `core.hooksPath` or an `init.templateDir`
    (githooks(5)). `-c core.hooksPath=/dev/null` runs none: probed with git 2.43.0 under a global config holding a
    failing hook of each of the four and `commit.gpgsign = true`: no hook ran, exit 0, author `htui
    <htui@localhost>`, `ls-tree -r HEAD` and `status --porcelain` empty; without `-c commit.gpgsign=false` that config
    fails the commit (`cannot run gpg`), and with hooks active `pre-commit` fails it.
  - `repo_facts(path) -> Result<Option<RepoFacts>, IsolateError>` (sync, `gix`): `None` when no repository is at
    `path` (`gix::open`'s not-a-repository error; `open` does not discover upward); `RepoFacts { branch:
    Option<String>, origin: Option<String>, head: Option<String>, bare: bool }`.
  Every message names paths as given and never a URL's userinfo.
- **D9 — The default target (`T`).** `T` (`workspaces.target`) on any row of a workspace reads
  `TargetChoices(workspace)` and opens a picker over the tab titled `Default target of <name>`: first `inherit the
  Settings default (<name>)` (`WriteStore::default_target`, no row = `seed::LOCAL_TARGET_NAME`, as M1's
  `workspaces::default_target`, `workspaces.rs:273`, private), then every target with `retired_at: None` by name, each with
  its kind; a kind this build does not run (`!kind.runnable()`) is drawn dim with `planned · <TargetKind::owner()>`
  and **cannot be chosen** (told "<name> runs once <MOD-n> lands"). The current choice is pre-selected and marked
  `· current`. Choosing sends `SetWorkspaceTarget { workspace, expected: <scope row's updated_at>, target }` →
  `set_project_target` on the scope row (`traits.rs:~3079`); `Applied` → a forced overview read (the overview's
  `default target` line follows); `Stale` → told "the default target of <name> changed meanwhile; press T again"; no
  scope row → told. **MOD-130's resolver (its M2) is not on this tree** (no resolver symbol, no MOD-130 M2 commit at
  `b473420a`): the overview keeps M1's `TargetLine` words (PRD OQ3, settled).
- **D10 — Deleting a workspace on the tab.** `d` (`common.delete`) on a workspace's group or `overview` row runs
  Hierarchy's flow (`settings/hierarchy.rs:1106-1208`): `DeleteReach(DeleteTarget::Workspace(id))` → counting →
  the warning with the store's counts (the same headline, now `crate::hierarchy::workspace_delete_warning`) →
  `y`/`Enter` → the typed slug (`Enter` submits; a wrong slug clears the field and says so) → `DeleteWorkspace(id)`,
  drawn as a kit `Ask` over the tab (`Ask::input` for the slug). The store's refusals (`workspace_has_live_run`,
  `workspace_has_stopped_run`) are said in the box; a refused delete goes back to the warning, a refused count
  closes. The box adds "Its folder `<path>` stays on disk." (nothing on disk is removed). After `Deleted` the tab
  reads `Workspaces` again; when the deleted workspace was the open one, the tab emits `SetScope` with the first one
  listed (Hierarchy's rule, `settings/hierarchy.rs:1822-1832`), or none, leaving the empty state.
- **D11 — The scope follows the list.** On a `Workspaces` reply, when the listed summary of the open workspace has
  other project ids than `ctx.scope.project_ids`, or another name than `ctx.top_bar.workspace`, the tab emits
  `SetScope` with it (Hierarchy's D11, `settings/hierarchy.rs:1510-1522`, mirrored here): a project created here, or a
  rename of the open workspace, reaches the Backlog and the header. The shell refuses it while an in-pane editor is
  open, as today (`docs/htui-editor.md:182-188`, rewritten in T5). **(fact-check)** Hierarchy keeps its own D11 (its
  `n` still creates projects): "mirrored", not "moved". The name half never fires: `App::observe_reply` sets
  `top_bar.workspace` from **every** `Workspaces` reply before the tab sees it (`app/update.rs:620-627`), so a rename
  already reaches the header; the rule is the project ids alone, compared as `Scope::from_workspace(&summary)
  .project_ids` (position order) against `ctx.scope.project_ids`.
- **D12 — Verbs by row.** `N`, `T`: any row (the project choice and the target are the row's workspace's). `e`,
  `d`: a workspace's group or `overview` row; on a project row they are told "projects are edited in Settings ›
  Hierarchy until MOD-123". `o`, `s`, `r`: as M1/M2.
- **D13 — Outcomes.** A refusal the person can act on (a path, a slug, a taken destination, git's own line, not a
  repository) is answered `StoreReply::SetupRefused { request, reason }`, its reason a sentence with no
  "constraint violated" prefix; a store failure stays `StoreReply::Failed`. With the form open either is shown under
  the form, the text kept, and `Enter` retries; with the form closed (D7) either is a told box (M2's
  `Question::Told`). `absorbs_failure` takes every request of D15's names (the tab says them itself). Success closes
  the form, re-reads `Workspaces` (and forces the overview read) and puts the cursor on the new row.
- **D14 — Keys** (every key a catalogue row; `tests/it/no_hardcoded_keys.rs`). After M2: `ALL`/`position` end at
  `LeaveStop` 254, `CATALOGUE.len()` 255, `DECLARED.len()` 103 (`[102]` = `WORKSPACES_CONFIRM`).
  - T3: `Act::WorkspacesNew`, `row(Act::WorkspacesNew, Workspaces, "new", &["N"], "new workspace or project")`,
    `position` 255 (appended to `ALL`), declared in the enum after M2's `WorkspacesResume`; `CATALOGUE.len()` 256.
    Stacks: `WORKSPACES_PICK = [Layer::view(Context::Workspaces, &[]), CONFIRM, LIST, MODAL]` and `WORKSPACES_FORM =
    [Layer::view(Context::Workspaces, &[]), FIELD, PICK, FORM_FIELDS, MODAL]` (as `ITEM_FORM`, `views.rs:1163`, without
    `form.save`/`external_editor`), each with its stack-list entry and a `DECLARED` entry **appended** (`[103]` "in a
    Workspaces picker", `[104]` "in a Workspaces form"; len 105). `VIEW_DEFAULTS` gains `(Workspaces, FormNextField,
    ["down"])` and `(Workspaces, FormPrevField, ["up"])` right **after** M2's `(Workspaces, ConfirmYes, ["enter"])`
    (`catalogue.rs:2642`; the table's order is `Context::ALL`, then `Act` order, and `ConfirmYes` precedes
    `FormNextField`). **(fact-check)** the test pins that move with these stacks, all in T3's `views.rs`:
    `VIEW_STACKS` (`:1441`); `the_m2_layers_sit_where_the_blueprint_puts_them`'s list of capturing stacks (`:2160`)
    gains `WORKSPACES_FORM`; `every_view_stack_is_named_by_a_view` (`:2393`) and
    `every_act_is_offered_by_a_stack_and_handled_by_a_view` (`:2422`) read only the files in `VIEWS` (`:1337`), so a
    new file that names `views::WORKSPACES_*` or handles their acts (`chooser.rs`, `forms.rs` in T3; `delete.rs` in
    T4) is added to `VIEWS` (or the acts are named in `mod.rs`).
  - T4: `Act::WorkspacesTarget`, `row(.., Workspaces, "target", &["T"], "default target")`, `position` 256,
    `CATALOGUE.len()` 257; `WORKSPACES_BROWSE` gains it and `Layer::only(Context::Common, &[Act::Edit, Act::Delete])`
    after the view layer (layer order: view, common, list, pane, layout, history, global; `e`/`d` are bound by no other
    layer there). Edit uses `WORKSPACES_FORM`, `T` `WORKSPACES_PICK`, the delete M2's `WORKSPACES_CONFIRM` (counting,
    warning) and `views::CAPTURE` (typed slug, in flight). No new stack.
  - T5 (M1 D10's slot rule): `Act::HierarchyNewWorkspace` is deleted (enum `catalogue.rs:565`, row `:1421-1429`,
    `ALL` `:2924`, `position` 56 `:3185`, `HIERARCHY_BROWSE` `views.rs:269`); `WorkspacesNew` takes `position` 56 and its `ALL` slot; `WorkspacesTarget`
    moves 256 → 255; `CATALOGUE.len()` 256. A user `keys.toml` naming `[settings.hierarchy] new_workspace` keeps
    loading (MOD-115 MD4) with one notice, "[settings.hierarchy] new_workspace: new workspaces are created on the
    Workspaces tab (its key is [workspaces] new)", through a `RETIRED_ACTIONS` list beside `RETIRED`
    (`keys/load.rs:62`) checked in `entry` before the "no such action" refusal. **(fact-check)** M1's mechanism is
    table-level only (`walk_table`, `load.rs:369-377`, matches a whole table's path): it does **not** cover one
    retired action inside a live table, so `RETIRED_ACTIONS` is new work, as planned.
  `docs/htui-keys.md` is regenerated in T3, T4 and T5 (`cargo run -q -p htui -- --default-keys --print-keys`).
- **D15 — Request names.** `"new_workspace"`, `"edit_workspace"`, `"new_project"`, `"target_choices"`,
  `"set_workspace_target"` (`workspace_setup::REQUEST_NAMES`), all served by `crate::workspace_setup`; the delete reuses
  Hierarchy's `"delete_reach"`/`"delete_workspace"` from the tab's own origin (staleness is per origin and kind).
- **D16 — A workspace with no project keeps its counts read (M1 gap found here).** `App::on_tick`
  (`app/update.rs:390-396`) dispatches `WorkspaceRunCounts` only inside `!self.scope.is_empty()`, i.e. only when
  the open workspace has a project: every workspace M3 creates starts with none, so opening one would freeze the
  states and the background chip. T3 moves the counts dispatch to "the scope names a workspace" (`workspace_id !=
  WorkspaceId::default()`); `Waiting` keeps its gate. **(fact-check)** Confirmed (`Scope::is_empty` is
  `project_ids.is_empty()`, `htui-core/src/model/scope.rs:39-41`); only `set_scope` (`:465-466`) and the tab's own
  `wants_requests` on activation (`ui/tabs/workspaces/mod.rs:~995-997`) still ask. Put the new dispatch right after
  the gated `Waiting` so `a_tick_only_asks_for_the_top_bar_once_a_second` keeps its order (`StoreState`, `Waiting`,
  `WorkspaceRunCounts`, `SweepRetention`, `:1422-1443`).
- **D17 — Hierarchy after M3 (PRD Q3).** It keeps the workspace row as the tree's head (name, slug, folder, read
  only), projects (`n` on the workspace row still creates a bare project), repos, paths, infer, documents and memory.
  `N` is gone; `e`, `b` and `d` on the workspace row answer a notice naming the tab's key ("`e` on the Workspaces
  tab edits the workspace", "… sets its folder", "`d` on the Workspaces tab deletes it"); its empty states name
  `` `N` on the Workspaces tab ``. The worker requests `CreateWorkspace`, `UpdateWorkspace` and `SetWorkspaceRoot`
  stay served and tested (worker-level tests and `hierarchy_pg` pin them) but nothing in the UI sends them after T5:
  their removal is MOD-123's cleanup (named in its handoff note by the bookkeeping, not here).
- **D18 — Snapshots are reviewed, never blindly accepted** (M1 D13): each task runs `cargo insta test -p htui
  --all-features`, then `cargo insta pending-snapshots`, and checks every `.snap.new` against what its section says
  moves; anything else is a regression to fix. Grepping `.snap` undercounts (memory).
- **D19 — Not in M3.** Coordinator chats (Q1, MOD-136/MOD-121); plain-folder repos (Q2, MOD-145); joining an existing
  project to a second workspace; a directory picker inside the forms (typed paths only, OQ4); cancelling a clone
  (OQ1, settled: no cancel); moving projects and repos out of Settings (MOD-123); components, compose and `.env`
  (MOD-117).

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| A `git` verb on `Cli` with its own budget | `crates/htui-orch/src/isolate/git.rs:1221-1333` (`ls_remote_budget`, `network_ssh`, `ls_remote`, `push`), `:1067` (`network_env`), `:377` (`run`), `:344` (`command`) | network env + bounded clone of `self`; classify the exit, never parse stdout |
| Identity for a commit | `git.rs:131` (`IDENTITY_ENV`), its use `tests/it/gix_isolator.rs:2618-2632` (`git_as`/`git_ok`) | env, not `-c user.*` |
| `gix` reads | `git.rs:1776` (`open`), `:1788-1814` (`head`, unborn refusal), `:1402` (`blocking`) | sync over `&Path`, owned values |
| Test repos and a local remote | `git.rs:2554-2700` (`testkit`: `usable_git`, `skip_without_git!`, `repo_with_one_commit`), `tests/it/gix_isolator.rs:4527-4549` (`bare_remote`) | no network; skip without git |
| Redaction | `crates/htui-orch/src/infer.rs:178` (`redact_remote`), `:205` (`redact_urls`) | userinfo dropped, nothing else touched |
| Worker-assembled writes, identity in the worker | `crates/htui/src/hierarchy.rs:268-489` (`serve_with_home`; `CreateWorkspace` `:286-302`, `CreateProject` `:331-374`), `:560` (`box_id`), `:577` (`canonical`), `:597` (`rebuild_mirror`), `:177` (`workspace_root`) | writer or `Unreachable`; refuse before writing; `spawn_blocking` for the disk |
| Folder default | `crates/htui-core/src/model/hierarchy.rs:279-305` (`WORKSPACES_DIR`, `workspace_folder`), tests `:340-380` | pure, one-plain-name rule |
| Spawned store-worker job | `store_worker.rs:3895-3906` (`ListDir` arm), inline arm `:2873`, `list_dir` `:3238` (fact-check: exact) | `tokio::spawn`, reply at the request's `seq` and origin, `continue` |
| Request / reply / name | `store_worker.rs:476-510` (M1's and M2's requests), `name()` `:1439` (arms `:1489-1491`), `name_arms_are_stable` `:6909`, workspaces or-arm `:2794-2796` (fact-check: exact) | one variant, one name arm, one or-arm |
| Form of labelled fields | `settings/hierarchy.rs:335-357` (`Field`, `Editor`), `:1986-2033` (`Field::required/optional`, `Editor::form` as a kit `Form`), `:1251-1304` (`on_editor_key`), `:1347-1448` (`submit`), `:1568-1606` (`on_stale`) | the focused field first, then the stack; `Enter` submits; the editor stays open until the reply |
| A choice field | `ui/tabs/backlog/item_form.rs:619` (`on_pressed`), `:760` (`on_picker_key`), `ui/kit/form.rs:44-59` (`Value::Choice` `:52`) | `choice.prev`/`next` only on the choice's focus |
| List in a modal | `ui/path_picker.rs:292` (`PathPicker::render`: `Modal` + kit list), `ui/kit/modal.rs:59-147` | cursor row, hint on the border |
| Delete flow | `settings/hierarchy.rs:1106-1225` (`begin_delete`, `on_deleting_key`, `deleted`), `:2217-2300` (`delete_pane`), `:1782-1832` (the `DeleteReach`/`Deleted`/`Workspaces` replies) | count, warn, typed slug, then the write |
| Kit question over a tab | M2's `Question`/`Question::Told { title, text }` and `tell` in `ui/tabs/workspaces/mod.rs:~249-282`, `:~565`, its texts in `ui/tabs/workspaces/question.rs`; `ui/tabs/backlog/detail/runs.rs:2476` (`question`), `ui/kit/modal.rs:171-330` (`Ask`, `input`, `error`) | the modal carries the keys |
| Retired key-file entry | `keys/load.rs:62` (`RETIRED`), `:369-377` (`walk_table`), `:1492-1530` (its test), M1 fixture `tests/fixtures/keys/switcher.toml` | loads with one notice, nothing read |

## Tasks

Order: **T1 → T2 → T3 → T4 → T5**, one serial lane. T1 is the foundation and touches only `htui-core` and
`htui-orch` (no M2 file): it may start before M2 lands. T2-T5 all touch `crates/htui/src/store_worker.rs`,
`ui/tabs/workspaces/*` or `keys/*`, and start after M2's T3 has landed. Every task leaves `cargo check --workspace
--all-targets --all-features` green and runs only its crate-scoped checks. Integration tests are modules of each
crate's one `it` binary (TOOL-15): run them with `--all-features`. `cargo test` takes **one** positional filter;
put several after `--`. Grep every test log for `SIGABRT`. A hook blocks shell `grep` on bare symbol names: use the
Gortex tools for Rust source and add `\b` when grepping logs.

### T1: Folder rules and the git helpers (foundation; `htui-core`, `htui-orch`)

- **Files**:
  - `crates/htui-core/src/model/hierarchy.rs` (`slug_of`, `PROJECTS_DIR`, `project_folder`, private `one_name`; unit
    tests), `crates/htui-core/src/model/mod.rs` (re-export beside `workspace_folder`, which is re-exported there, `:135-138`)
  - `crates/htui-orch/src/isolate/git.rs` (`CLONE_TIMEOUT`, `RepoFacts`, `repo_facts`, `Cli::clone_repo`,
    `Cli::clone_budget`, `Cli::init_with_empty_commit`, `clone_args`, `init_args`, `commit_args`; unit tests in its
    `mod tests`, `:2802`)
  - `crates/htui-orch/tests/it/git_setup.rs` (CREATE), `crates/htui-orch/tests/it/main.rs` (`mod git_setup;`)
- **Types and signatures**:
  ```rust
  // htui_core::model::hierarchy
  /// MOD-116 M3 D2: the directory under a workspace folder that holds projects cloned or started there.
  pub const PROJECTS_DIR: &str = "projects";
  /// MOD-116 M3 D2: the slug a name suggests: ASCII letters lowercased and digits kept, every other run one `-`,
  /// trimmed of `-`, at most 40 bytes; `""` when nothing is left.
  #[must_use] pub fn slug_of(name: &str) -> String;
  /// MOD-116 M3 D2: `<workspace_folder>/projects/<slug>`; `None` when `slug` is not one plain name.
  #[must_use] pub fn project_folder(workspace_folder: &Path, slug: &str) -> Option<PathBuf>;
  ```
  ```rust
  // htui_orch::isolate::git
  /// MOD-116 M3 D8: a clone's wall-clock budget; on expiry the group is killed.
  pub const CLONE_TIMEOUT: Duration = Duration::from_secs(600);
  /// MOD-116 M3 D8: what registering an existing repository needs. Never a redacted URL: the caller redacts.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct RepoFacts { pub branch: Option<String>, pub origin: Option<String>,
                         pub head: Option<String>, pub bare: bool }
  /// `None` when no repository is at `path` (not discovered upward).
  pub fn repo_facts(path: &Path) -> Result<Option<RepoFacts>, IsolateError>;
  #[must_use] pub fn clone_args(url: &str, dest: &Path) -> Vec<OsString>;   // D8's argv
  impl Cli {
      fn clone_budget(&self) -> Duration;          // CLONE_TIMEOUT, or self.budget when that is below VERB_TIMEOUT
      /// # Errors `Refused("git clone: <redacted line>")` on a non-zero exit; `Git` on spawn or timeout.
      pub async fn clone_repo(&self, url: &str, dest: &Path) -> Result<(), IsolateError>;
      /// `dir` exists and is empty; answers the commit's hex.
      pub async fn init_with_empty_commit(&self, dir: &Path, branch: &str) -> Result<String, IsolateError>;
  }
  ```
  `repo_facts`: `gix::open(path)`: its not-a-repository error → `Ok(None)` (match the `gix::open::Error` variant;
  confirm the name in `gix` 0.87.1), any other error → `Git`; `bare` = no work dir; `branch` = `repo.head_name()`'s
  short name (`None` detached); `head` = the peeled id or `None` unborn; `origin` = `repo.find_remote("origin")`'s
  fetch URL as text (`None` without one). `clone_repo` runs in `dest.parent()` (refused when `dest` has none) with
  `network_env(self.network_ssh(parent).await.as_deref())` and a `Self { budget: self.clone_budget(), ..self.clone() }`.
  `init_with_empty_commit`: `init --quiet --initial-branch=<branch>` then `-c core.hooksPath=/dev/null -c
  commit.gpgsign=false commit --allow-empty --no-verify --quiet -m "Initial commit"` (fact-check D8) under
  `IDENTITY_ENV`, each exit checked with
  `Exited::failure`, then `blocking(head(dir))`. Module doc gains one line: the two new verbs (`clone`, and `init` +
  `commit` for a new repository, MOD-116 M3).
- **Tests first**:
  - `model/hierarchy.rs`: `slug_of_keeps_letters_and_digits_and_joins_the_rest` (`"Mobile App 2"` → `mobile-app-2`,
    `"  --Ünïcode__x "` → `n-code-x` (each non-ASCII letter is one separator), `"!!!"` → `""`, a 60-char name is cut
    to ≤ 40 bytes ending in a letter or digit); `project_folder_is_under_projects_and_refuses_a_path_slug`
    (`/w`, `api` → `/w/projects/api`; `../x`, `a/b`, `/`, `""` → `None`); `workspace_folder`'s existing tests unchanged.
  - `git.rs` unit: `the_clone_argv_forbids_ext_and_ends_with_the_url_and_dest` (`-c protocol.ext.allow=never`, then
    `clone --quiet -- <url> <dest>`; a URL `--upload-pack=x` stays after `--`); `the_clone_budget_is_ten_minutes_unless_a_test_shortened_it`;
    `the_init_and_commit_argv_never_sign_and_never_run_hooks` (asserts `-c core.hooksPath=/dev/null` and
    `-c commit.gpgsign=false`, fact-check D8).
  - `tests/it/git_setup.rs` (each case `let Some(git) = skip_without_git!() else { return };`, temp dirs only):
    `a_clone_of_a_local_bare_repo_lands_at_dest_with_its_head` (a `repo_with_one_commit` pushed to a bare remote as
    `gix_isolator.rs:4527` does, **but** made with `init --quiet --bare --initial-branch=main` (fact-check: that
    helper's bare repo keeps `HEAD -> master` while the push lands `main`, and git 2.43.0 then clones with exit 0, an
    unborn `HEAD` on `master` and nothing checked out, so `head(dest)` would fail); the helper is private to
    `gix_isolator.rs`, so `git_setup.rs` writes its own; `clone_repo(<bare path>, tmp/projects/api)` → `head(dest)` equals the source commit;
    `repo_facts(dest)` → branch `main`, origin = the bare path); `a_clone_of_a_missing_url_is_refused_without_a_prompt_and_leaves_no_dest`
    (URL `tmp/nothing.git` → `Refused` starting `git clone:`; `dest` absent); `a_clone_url_s_userinfo_never_reaches_the_message`
    (`https://user:secret@127.0.0.1:9/x.git` → message without `secret`, quickly: a refused connection; probed: git
    itself prints `unable to access 'https://127.0.0.1:9/x.git/'` in 0.03 s, `redact_urls` is the second guard);
    `an_empty_remote_clones_with_an_unborn_head` (a bare `--initial-branch=main` with no commit: `Ok`, dest exists,
    `repo_facts` branch `main`, `head` `None`; fact-check D6);
    `a_new_repo_has_one_empty_commit_and_nothing_in_its_tree` (`git log --oneline` one line, author `htui
    <htui@localhost>`, `git ls-tree -r HEAD` empty, `git status --porcelain` empty, `head` resolves; with
    `GIT_CONFIG_GLOBAL` **not** touched: run under the user's config as production does);
    `repo_facts_reads_branch_origin_and_head` (origin set by `git remote add origin https://u:t@h/x`; facts carry it raw);
    `repo_facts_of_a_plain_folder_is_none_and_of_a_subfolder_too` (a plain dir; a subdirectory of a repo);
    `repo_facts_of_an_unborn_repo_has_no_head_but_a_branch` (`empty_repo`); `repo_facts_says_bare` (`git init --bare`).
- **Check**:
  ```bash
  cargo test -p htui-core --all-features --lib -- model::hierarchy::
  cargo test -p htui-orch --all-features --lib -- isolate::git::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^git_setup::/)'
  cargo clippy -p htui-core -p htui-orch --all-targets --all-features -- -D warnings
  cargo clippy -p htui-core -p htui-orch -- -D warnings      # featureless (memory)
  cargo fmt -p htui-core -p htui-orch --check
  ```

### T2: The worker side — create, edit, project sources, targets (after T1 and M2)

- **Files**:
  - `crates/htui/src/workspace_setup.rs` (CREATE; module doc names D4-D9, `R-NF-3`, `R-ID-4`; unit tests),
    `crates/htui/src/lib.rs` (`pub mod workspace_setup;` beside `pub mod workspaces;`)
  - `crates/htui/src/hierarchy.rs` (`canonical`, `box_id`, `rebuild_mirror` become `pub(crate)`; `CreateProject`'s
    create-and-link body factored into `pub(crate) async fn create_linked_project(writer, workspace, NewProject) ->
    Result<Project>` and called from that arm, behaviour unchanged)
  - `crates/htui/src/store_worker.rs` (five requests, five replies, `SetupRefused`, five name arms, one `try_serve`
    or-arm, the loop's spawned `NewProject` arm, `name_arms_are_stable`, one loop test)
  - `crates/htui/src/workspaces.rs` (**fact-check**: only to make `default_target`, `:273`, `pub(crate)` for
    `TargetChoices`' default name; it is private, so "as M1's" means reuse through this file or a six-line copy)
  - Not expected to change: every snapshot; `tests/it/hierarchy.rs` (the factored helper keeps `CreateProject`'s
    reply byte for byte).
- **Types and signatures** (`workspace_setup.rs`; all `Debug, Clone, PartialEq`, `Eq` where the fields allow):
  ```rust
  pub const REQUEST_NAMES: [&str; 5] =
      ["new_workspace", "edit_workspace", "new_project", "target_choices", "set_workspace_target"];
  pub enum ProjectSource { Existing { path: String }, Clone { url: String }, NewRepo }
  pub enum EditOutcome { Applied { workspace: WorkspaceSummary, folder: String }, Stale(Box<Workspace>) }
  pub struct TargetChoice { pub id: TargetId, pub name: String, pub kind: TargetKind,
                            /// `kind.owner()` when this build does not run the kind (D9).
                            pub planned: Option<&'static str> }
  pub struct TargetChoices { pub scope_row: Option<(ProjectId, DateTime<Utc>)>, pub current: Option<TargetId>,
                             pub default_name: String, pub targets: Vec<TargetChoice> }
  pub enum TargetWrite { Applied, Stale }
  /// D4-D9. `Unreachable(DATABASE_UNREACHABLE)` offline; `Backend` for a request that is not this module's.
  pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply>;
  pub async fn serve_with_home(backend: &Backend, request: &StoreRequest, home: Option<&Path>) -> Result<StoreReply>;
  /// D4 step 2, sync (called under `spawn_blocking`): the canonical folder and whether it was made now.
  fn prepare_folder(path: &Path) -> std::result::Result<(PathBuf, bool), String>;
  ```
  ```rust
  // store_worker.rs (doc comments name MOD-116 M3 and the D they serve)
  StoreRequest::NewWorkspace { slug: String, name: String, description: String, folder: String },   // "new_workspace"
  StoreRequest::EditWorkspace { id: WorkspaceId, expected: DateTime<Utc>, patch: WorkspacePatch,
                                folder: String },                                                   // "edit_workspace"
  StoreRequest::NewProject { workspace: WorkspaceId, slug: String, name: String, description: String,
                             source: ProjectSource },                                               // "new_project"
  StoreRequest::TargetChoices(WorkspaceId),                                                         // "target_choices"
  StoreRequest::SetWorkspaceTarget { workspace: WorkspaceId, expected: DateTime<Utc>,
                                     target: Option<TargetId> },                                    // "set_workspace_target"
  StoreReply::WorkspaceCreated { workspace: WorkspaceSummary, folder: String, made: bool },
  StoreReply::WorkspaceEdited { workspace: WorkspaceId, outcome: EditOutcome },
  StoreReply::ProjectCreated { workspace: WorkspaceId, project: ProjectRef, repo_path: String },
  StoreReply::TargetChoices { workspace: WorkspaceId, choices: Box<TargetChoices> },
  StoreReply::WorkspaceTargetSet { workspace: WorkspaceId, outcome: TargetWrite },
  /// D13: a refusal the person can act on, in a sentence (no "constraint violated" prefix).
  StoreReply::SetupRefused { request: &'static str, reason: String },
  // try_serve: one or-arm
  StoreRequest::NewWorkspace { .. } | StoreRequest::EditWorkspace { .. } | StoreRequest::NewProject { .. }
      | StoreRequest::TargetChoices(..) | StoreRequest::SetWorkspaceTarget { .. } => workspace_setup::serve(backend, request).await?,
  // the loop, beside the ListDir arm (D7)
  StoreRequest::NewProject { .. } => { /* clone backend, seq, origin, tx; tokio::spawn(serve then send); continue */ }
  ```
  Inside `serve_with_home`: the writer first (offline → `Unreachable` before any disk or store call), then the
  request's steps exactly as D4, D5, D6 and D9 order them. Refusals of D4-D6 (paths, slugs, taken destinations,
  `not a git repository … MOD-145`, bare, git's line, no folder, no box row) are `Ok(StoreReply::SetupRefused {
  request: request.name(), reason })`; a `StoreError` from the store (a taken workspace slug included) is `Err`, which
  the worker turns into `Failed` as for every request. Git: `tokio::task::spawn_blocking(Cli::locate)` per git-backed
  request (an existing folder needs none: `gix` only); `Refused(reason)` from `locate` → `SetupRefused` "a git URL
  or a new repo needs git: <reason>" (the inner `reason`: `IsolateError`'s `Display` adds `isolation refused: `). Clean-up (D4, D6) never removes a path the request did not make, and is
  best-effort (a failed removal is logged with `tracing::warn!` and named in the reason: "… and `<dest>` could not
  be removed"). `TargetChoices`: `scope_project(ws)` → the row's `(id, updated_at)` and `target_id`; `targets()`
  without retired ones, in the store's (name) order; the default name as M1's `default_target`. `SetWorkspaceTarget`:
  no scope row → `SetupRefused`; else `set_project_target(row, expected, target)` → `Applied`/`Stale`.
- **Tests first**:
  - `workspace_setup.rs` unit tests (`Backend::memory(MemStore::demo())`, `serve_with_home` with a temp home, temp
    dirs; git cases start with `skip_without_git!`; a local bare remote built as `gix_isolator.rs:4527` does, with
    `--initial-branch=main`, T1's fact-check):
    `a_new_workspace_makes_its_default_folder_and_records_no_row` (folder `<home>/workspaces/mobile`: made, no
    `workspace_box_paths` row, a scope row, the reply's summary has no project);
    `a_new_workspace_elsewhere_records_its_canonical_row`; `an_existing_folder_is_used_as_it_is` (a file inside
    stays, `made == false`); `a_folder_that_is_a_file_or_a_repository_is_refused_before_any_write` (no workspace row
    with that slug afterwards); `a_relative_folder_is_refused`;
    `a_taken_slug_removes_only_the_folder_it_made` (`graphics` taken: a made folder is gone, a pre-existing one stays);
    `an_edit_renames_and_points_at_a_new_folder_without_moving_the_old` (old folder and its file still there, new one
    made, row = new canonical path); `a_slug_change_with_no_row_records_the_old_folder`;
    `a_stale_edit_writes_neither_the_row_nor_the_folder_row` (the made folder may stay: assert the rows only);
    `a_new_repo_project_is_registered_with_one_empty_commit` (project linked at the end, one primary repo `api` on
    `main`, no remote, a `repo_box_path` = canonical `<folder>/projects/api`, `head` resolves, tree empty);
    `a_git_url_project_clones_into_projects_and_keeps_the_remote_redacted` (URL `file://` or the bare path; remote
    stored equals `redact_remote(url)`); `an_existing_git_folder_registers_where_it_is_with_origin_redacted`
    (`https://u:t@host/x.git` stored as `https://host/x.git`, branch from `HEAD`, nothing written in the folder: its
    `git status --porcelain` unchanged); `a_plain_folder_is_refused_naming_mod_145`;
    `an_existing_destination_is_refused_and_kept`; `a_taken_project_slug_is_refused_before_cloning` (`htui`: no dest
    made); `a_failed_clone_leaves_no_dest_and_no_project`; `a_failed_registration_deletes_the_project_and_the_clone`
    (an unlinked project with the slug created first through the store, so the pre-check passes and `create_project`
    refuses after the clone: dest gone, no second project); `a_workspace_without_a_folder_cannot_take_a_project`
    (no home, no row); `target_choices_list_live_targets_and_mark_planned_kinds` (a retired target absent; a
    `docker` target `planned == Some("MOD-44")`; `default_name == "this box"`);
    `the_workspace_target_is_set_and_cleared_on_the_scope_row`; `a_stale_target_write_is_refused`;
    `offline_every_setup_request_is_refused_before_any_write` (as `workspaces.rs`'s `offline_the_overview_is_refused`;
    no folder made); `another_request_is_refused_by_name`.
  - `store_worker.rs`: `name_arms_are_stable` gains the five names; `a_new_project_is_answered_from_its_own_task`
    (over the real loop with a memory backend, as the `round_trip` tests do, `:4317`: a `NewProject { NewRepo }`
    into a temp workspace folder (fact-check: the loop serves `workspace_setup::serve`, i.e. the real home, so the
    workspace gets a folder row on the memory store's box pointing at the temp dir first) answers `ProjectCreated` at its own `seq`, and a `Workspaces` request sent right after
    it is answered too; skip without git).
  - `hierarchy.rs` tests: unchanged and green (the factored helper).
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- workspace_setup:: store_worker:: hierarchy::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^hierarchy::/) or test(/^workspaces_tab::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # expected: none (D18)
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T3: `N`, the chooser, the two creation forms and the counts read of an empty workspace (after T2)

- **Files**:
  - `crates/htui/src/ui/tabs/workspaces/mod.rs` (mode, keys, replies, render, paste, `with_home`, empty state, D11
    scope follow, D16 is not here), `crates/htui/src/ui/tabs/workspaces/chooser.rs` (CREATE: a list in a kit modal,
    reused by T4's target picker), `crates/htui/src/ui/tabs/workspaces/forms.rs` (CREATE: `WorkspaceForm`,
    `ProjectForm`, previews, D2/D3 helpers)
  - `crates/htui/src/app/update.rs` (D16, `on_tick`; its tick tests)
  - `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs` (the stacks, `VIEW_STACKS`, the capturing-stack
    pin and `VIEWS` += `chooser.rs`, `forms.rs`: D14's fact-check), `crates/htui/src/keys/stack.rs`,
    `crates/htui/src/keys/hint.rs` (the `WORKSPACES_BROWSE` help-line pin, `:751-763`, gains `N new workspace or
    project`)
  - `crates/htui/src/ui/about.rs` (Workspaces actions gain `(WorkspacesNew, "new")`)
  - `crates/htui/tests/it/workspaces_create.rs` (CREATE), `crates/htui/tests/it/main.rs` (`mod workspaces_create;`)
  - `crates/htui/tests/it/workspaces_tab.rs` (**fact-check**: `an_empty_store_says_how_to_create_a_workspace`,
    `:357-367`, asserts the old next step `` `N` in Settings › Hierarchy creates one ``; it moves to `` `N` creates
    one ``)
  - snapshots: new `it__workspaces_create__*`; moved per D18 (below); `docs/htui-keys.md` (regenerated);
    `README.md` (`:181-183`: "If the database has none yet, press `N` on the Workspaces tab"; the **1 Workspaces**
    row `:189`: "`N` creates a workspace or a project (an existing git folder, a git URL it clones, or a new repo)")
- **Types and signatures** (private unless said):
  ```rust
  // chooser.rs
  pub(super) struct Chooser<T> { title: String, rows: Vec<ChoiceRow<T>>, cursor: usize }
  pub(super) struct ChoiceRow<T> { pub label: String, pub note: Option<String>, pub enabled: bool, pub value: T }
  impl<T: Clone> Chooser<T> {
      pub(super) fn new(title: impl Into<String>, rows: Vec<ChoiceRow<T>>, selected: usize) -> Self;
      pub(super) fn on_act(&mut self, act: Act) -> ChooserOutcome<T>;   // ListDown/Up move (skipping nothing);
                                                                        // ConfirmYes → Chosen(v) | Disabled(i); ConfirmNo → Closed
      pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, hint: &str, ctx: &Ctx<'_>);
  }
  pub(super) enum ChooserOutcome<T> { Stay, Chosen(T), Disabled(usize), Closed }
  // forms.rs
  pub(super) struct WorkspaceForm { name: TextField, slug: TextField, folder: TextField, description: TextField,
      focus: usize, slug_edited: bool, folder_edited: bool, home: Option<PathBuf>,
      edit: Option<(WorkspaceId, DateTime<Utc>)>,  // T4; None = new
      busy: Option<u64>, error: Option<String> }
  pub(super) struct ProjectForm { workspace: WorkspaceId, workspace_name: String,
      workspace_folder: Option<PathBuf>, name: TextField, slug: TextField, description: TextField,
      source: SourceKind, location: TextField /* the folder path or the URL */, focus: usize,
      slug_edited: bool, home: Option<PathBuf>, busy: Option<u64>, error: Option<String> }
  pub(super) enum SourceKind { Existing, Url, NewRepo }        // words "existing folder" · "git URL" · "new repo"
  pub(super) enum FormOutcome { Stay, Submit(StoreRequest), Refused(String), Cancel, Pass }
  impl WorkspaceForm { pub(super) fn new(home: Option<&Path>) -> Self;
      pub(super) fn on_key(&mut self, key: KeyEvent, pressed: KeyChord, keys: &Keys) -> FormOutcome;
      pub(super) fn on_paste(&mut self, text: &str);
      pub(super) fn lines(&self, width: u16, ctx: &Ctx<'_>) -> Vec<Line<'static>>; } // the form, then the preview
  impl ProjectForm { /* same four */ }
  pub(super) fn shown(path: &Path, home: Option<&Path>) -> String;          // `~/…` under home (D3)
  pub(super) fn expanded(typed: &str, home: Option<&Path>) -> Option<PathBuf>; // `~`, `~/x` → home; absolute kept; else None
  ```
  `mod.rs`: `WorkspacesTab` gains `home: Option<PathBuf>`, `pub fn with_home(self, home: Option<&Path>) -> Self`,
  `mode: Mode` (`Browse`, `Choosing(Chooser<NewWhat>)`, `Workspace(Box<WorkspaceForm>)`, `Project(Box<ProjectForm>)`)
  beside M2's `question`/`pending`, and `creating: Option<String>` (the slug of the `NewProject` out, D7). `enum
  NewWhat { Workspace, Project(WorkspaceId) }`.
  Behaviour:
  - `on_key` order: M2's question first; then the mode: chooser through `WORKSPACES_PICK`, a form through
    `WORKSPACES_FORM` (the focused text field first, as `on_editor_key`, `settings/hierarchy.rs:1251`; on the source
    choice `choice.prev`/`next` cycle it; `field.submit` anywhere submits; `field.cancel` closes; `form.next_field`/
    `prev_field` move; the rest `modal_rest`); while `busy`, every key but `field.cancel` is swallowed. Browse adds
    `Act::WorkspacesNew` → the chooser (D1), or straight to the workspace form with no workspace listed.
  - `WorkspaceForm`: focus order name, slug, folder, description. Each edit of `name` re-derives `slug` while
    `!slug_edited` and then `folder` while `!folder_edited` (`shown(home/workspaces/<slug>)`); with no home or a slug
    that is not one plain name the folder is left empty and the preview says to type one. Submit: name and slug and
    folder required (`` `name` is required ``, as `submit`, `settings/hierarchy.rs:1361-1367`); `expanded(folder)`
    `None` → refused "the folder must be an absolute path or start with ~"; → `StoreRequest::NewWorkspace {.., folder:
    <absolute>}`; `busy = Some(0)`. Preview (dim, under the form): `<folder>/` and "created now; an existing folder is
    used as it is", `  projects/` and "projects cloned from a URL or started here", then one line "the compose and env
    files land here once components arrive (MOD-117)"; then "A new workspace starts stopped."
  - `ProjectForm`: focus order name, slug, description, source, location (hidden for `NewRepo`). `location` is the
    folder path (`Existing`) or the URL (`Url`). Preview per source (D6): existing — "registers `<path>` as it is; htui
    writes nothing in it; plain folders come with MOD-145"; URL — "clones it into `<ws folder>/projects/<slug>` with
    this machine's git (its ssh agent and credential helper; htui never prompts)"; new repo — "creates `<ws
    folder>/projects/<slug>` with git init and one empty commit on main". Submit → `NewProject { .., source }` (an
    existing path through `expanded`, a URL trimmed). Busy line per D7 (`shown` paths; the URL through
    `htui_orch::infer::redact_remote`) with ` · <n> s` once `n > 0`.
  - `on_refresh`: a busy form's counter `+= 1` (then M1/M2's reads).
  - `on_reply`: `WorkspaceCreated` → mode `Browse`, `StoreRequest::Workspaces` asked, the cursor lands on its group
    (selected once the list holds it, `user_placed`), and D4's exception (`SetScope` when the scope names no listed
    workspace); `ProjectCreated` → close the form if it is that project's, `creating = None`, `Workspaces` asked, the
    overview marked stale, the cursor on `Entry::Project`; with the form closed, a told box "Project `<slug>` is ready
    in <name>: <shown repo_path>"; `SetupRefused`/`Failed` for `"new_workspace"`/`"new_project"` → `error` on the open
    form (busy cleared) or a told box (D13); `Workspaces` → D11's scope follow after M1's handling.
  - `absorbs_failure` adds `workspace_setup::REQUEST_NAMES`. `key_stack`: the mode's stack; `key_hint`: `None` while
    a chooser or form is up (the box and the pane carry their keys). `on_paste`: into the focused field.
  - `render`: the chooser as a modal over the tab body (`Chooser::render`, hint `j/k move · Enter choose · Esc
    close`); a form **in the detail pane** in place of the overview (pane title `New workspace` / `New project in
    <name>`, kit `Form` lines then the preview, the error in `theme.error`, the busy line), and in the list area when
    the layout hides the detail. Hint on the pane border: `Enter create · Esc cancel · Tab field` (+ `h/l source`).
  - The empty state (`empty_state`, `mod.rs:~779-793`, today through `views::HIERARCHY_BROWSE`'s
    `HierarchyNewWorkspace`): `` `N` creates one `` through
    `keys.label(views::WORKSPACES_BROWSE, Act::WorkspacesNew)`, else "a workspace created elsewhere shows up here".
  - D16 (`app/update.rs`): inside `on_tick`'s refresh, `WorkspaceRunCounts` is dispatched when
    `self.scope.workspace_id != WorkspaceId::default()`, outside the `!self.scope.is_empty()` gate that keeps
    `Waiting`, placed after that gate (D16's fact-check: the request order the tick test pins).
- **Keys**: D14's T3 half (`WorkspacesNew` 255, `CATALOGUE` 256, `WORKSPACES_PICK`, `WORKSPACES_FORM`, `DECLARED`
  `[103]`/`[104]` appended with pins, len 105, two `VIEW_DEFAULTS` rows); M2's `HINT_*` consts gain
  `Hint::One(Act::WorkspacesNew, "new")` after their state verbs. If the loader's clash or shadowing validation
  objects to a stack, report it rather than adding an allow-list entry.
- **Tests first**:
  - `views.rs`: `resolve(WORKSPACES_BROWSE, "N") == [WorkspacesNew]`; `resolve(WORKSPACES_PICK, "j")` → `ListDown`,
    `"enter"`/`"y"` → `ConfirmYes`, `"esc"` → `ConfirmNo`; `resolve(WORKSPACES_FORM, "tab")` → `FormNextField`,
    `"down"` → `[FormNextField, FieldLineDown]` (**fact-check**: the field's `line_down` is a second candidate, as
    `resolve(ITEM_FORM, "down")` and `resolve(AGENTS_FORM, "down")` pin, `views.rs:1669-1672`, `:1945-1948`), `"l"`
    → `ChoiceNext`, `"enter"` → `FieldSubmit`.
  - `forms.rs` unit: `the_slug_and_folder_follow_the_name_until_edited`; `an_edited_folder_stops_following`;
    `a_relative_folder_is_refused_on_the_form`; `tilde_paths_expand_with_the_tab_s_home`; `the_project_location_follows_the_slug`;
    `each_source_previews_what_it_does` (the three sentences).
  - `app/update.rs`: `a_workspace_with_no_project_still_reads_its_counts` (scope of a summary with no project: a
    refresh dispatches `WorkspaceRunCounts`, not `Waiting`); `a_tick_only_asks_for_the_top_bar_once_a_second` and
    `the_store_state_refresh_is_not_gated_on_a_scope` keep their counts (check them; a nil scope still asks two).
  - `crates/htui/tests/it/workspaces_create.rs` (`#![cfg(feature = "testkit")]`; the M1 module's `open_over`/`demo`
    copied, with `WorkspacesTab::new().with_home(Some(home.path()))` over a temp home and, **fact-check D3**, both
    folder rows pointed at existing folders under the temp dir, not `/srv/…`; git cases `skip_without_git!`):
    `n_offers_a_workspace_or_a_project_in_the_selected_workspace` (snapshot `chooser_100`);
    `n_on_an_empty_store_opens_the_workspace_form` (`Harness::empty()`; snapshot `workspace_form_empty_100`; the
    empty state names `N`); `the_workspace_form_previews_the_folder_that_follows_the_name` (type `Mobile App`:
    `slug mobile-app`, folder `~/workspaces/mobile-app`; snapshot `workspace_form_100`);
    `creating_a_workspace_makes_its_folder_and_lists_it_stopped_without_opening_it` (folder under the temp home exists;
    `Mobile App ○ stopped` listed; `app.scope` still Graphics; cursor on it);
    `the_first_workspace_is_opened` (`Harness::empty()`: after create, `top_bar.workspace == "Mobile App"`);
    `a_taken_slug_keeps_the_form_and_says_so` (`graphics`: the form stays with its text and the store's message; no
    status-line text); `esc_closes_the_form_and_writes_nothing`;
    `a_new_repo_project_is_created_and_selected` (`N`, `j`, `Enter`: snapshot `project_form_new_repo_100`; type the
    name, `l`/`h` to `new repo`, `Enter` → the project row is listed under Graphics and selected; the open workspace's
    scope gains it, D11: `app.scope.project_ids` contains it);
    `a_git_url_project_shows_its_progress_then_lands` (a local bare remote; snapshot `project_progress_100` taken
    after submit and before `settle`; then settled: listed);
    `a_plain_folder_is_refused_naming_mod_145` (snapshot `project_refused_100`);
    `esc_while_cloning_closes_the_form_and_the_outcome_is_told` (`Esc` before `settle`; after: told box names the
    project); `a_second_project_while_one_is_out_is_refused`;
    `typing_digits_and_q_in_a_form_does_not_switch_tabs_or_quit` (capturing stack).
- **Snapshots** (D18): new `it__workspaces_create__{chooser,workspace_form_empty,workspace_form,project_form_new_repo,
  project_progress,project_refused}_100`; moved: every `it__workspaces_tab__*` **in its hint row only** (`N new`),
  `src/snapshots/htui__testkit__tests__shell_empty` (both panes' empty-state next step: `` `N` creates one ``).
  Anything else that moves is a regression.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- ui::tabs::workspaces:: keys:: ui::about:: app::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_create::/) or test(/^workspaces_tab::/) or test(/^keys_file::/) or test(/^keys::/) or test(/^no_hardcoded_keys::/) or test(/^shell::/) or test(/^integration::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new (D18)
  cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T4: Edit, delete and the default target on the tab (after T3)

- **Files**:
  - `crates/htui/src/ui/tabs/workspaces/mod.rs` (`e`, `d`, `T`, `Asked::Edit`, replies, modes),
    `crates/htui/src/ui/tabs/workspaces/forms.rs` (`WorkspaceForm::edit`), `crates/htui/src/ui/tabs/workspaces/delete.rs`
    (CREATE: stages and their `Ask`), `crates/htui/src/ui/tabs/workspaces/chooser.rs` (target rows)
  - `crates/htui/src/hierarchy.rs` (`pub fn workspace_delete_warning(slug: &str, reach: &DeleteReach) -> String`: the
    workspace arm of `delete_pane`'s headline, moved verbatim), `crates/htui/src/ui/tabs/settings/hierarchy.rs`
    (`delete_pane`'s workspace arm calls it; nothing else)
  - `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs` (`VIEWS` += `delete.rs`, D14's fact-check),
    `crates/htui/src/keys/hint.rs`
  - `crates/htui/src/ui/about.rs` (actions gain `(WorkspacesTarget, "target")` if the test allows)
  - `crates/htui/tests/it/workspaces_manage.rs` (CREATE), `crates/htui/tests/it/main.rs` (`mod workspaces_manage;`)
  - snapshots: new `it__workspaces_manage__*`; `docs/htui-keys.md` (regenerated); `README.md` (the **1 Workspaces** row:
    "`e` edits a workspace, `d` deletes it, `T` sets its default target")
- **Types and signatures**:
  ```rust
  // forms.rs
  impl WorkspaceForm { /// D5: prefilled; the folder does not follow the slug; `edit = Some((id, updated_at))`.
      pub(super) fn edit(workspace: &Workspace, folder: Option<&Path>, home: Option<&Path>) -> Self;
      /// D5 Stale: keep the text, take the token.
      pub(super) fn retoken(&mut self, current: &Workspace); }
  // delete.rs
  pub(super) struct Deleting { workspace: WorkspaceId, slug: String, folder: Option<String>, stage: Stage }
  pub(super) enum Stage { Counting, Warn(Box<DeleteReach>), Typed { reach: Box<DeleteReach>, field: TextField,
                          wrong: bool }, InFlight(Box<DeleteReach>), Refused { reach: Option<Box<DeleteReach>>, message: String } }
  impl Deleting { pub(super) fn begin(workspace: WorkspaceId, slug: String, folder: Option<String>) -> (Self, StoreRequest);
      pub(super) fn on_key(&mut self, key: KeyEvent, pressed: KeyChord, keys: &Keys) -> DeleteOutcome;
      pub(super) fn on_reply(&mut self, reply: &StoreReply) -> DeleteOutcome;
      pub(super) fn stack(&self) -> Stack<'static>;                       // WORKSPACES_CONFIRM | CAPTURE
      pub(super) fn render(&self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx<'_>); }
  pub(super) enum DeleteOutcome { Stay, Send(StoreRequest), Deleted { workspace: WorkspaceId }, Closed }
  // mod.rs: Mode gains Editing(Box<WorkspaceForm>), Deleting(Box<Deleting>), Targeting(Chooser<TargetPick>, WorkspaceId,
  // Option<(ProjectId, DateTime<Utc>)>); enum TargetPick { Inherit, Target(TargetId), Planned(String /* owner */) }
  ```
  Behaviour:
  - `e` on a workspace's group or `overview` row (D12): the held, current overview of that workspace → `Mode::Editing`
    with `WorkspaceForm::edit(&overview.tree.workspace, overview.folder.as_deref(), home)`; else M2's pending ask with
    `Asked::Edit` and a forced read, the form opening when it lands. Submit → `EditWorkspace { id, expected, patch:
    WorkspacePatch { slug, name, description all Some }, folder }`. Under the folder field (help line): "changing it
    points the workspace at another folder; nothing is moved". `WorkspaceEdited { Applied }` → close, `Workspaces`
    asked, overview stale (D11 follows a rename of the open one); `Stale(current)` → `retoken`, error line
    `changed_elsewhere(<Enter's label>)`; `SetupRefused`/`Failed` `"edit_workspace"` → the form's error.
  - `d` → `Deleting::begin` (`DeleteReach(Workspace)`), folder = the held overview's `shown` folder if any. Replies:
    `DeleteReach(Some)` → `Warn`; `DeleteReach(None)` → told "already gone", `Workspaces` asked; `Deleted { target:
    Workspace(id), .. }` → close, `Workspaces` asked, and when `id` was the open workspace the first listed workspace
    other than it is opened on the next `Workspaces` reply (D10); `Failed` `"delete_reach"` → close with a told box;
    `Failed` `"delete_workspace"` → `Refused { reach, message }` drawn as the warning plus the message (`y` retries,
    `Esc` closes). The `Ask`'s lines: `workspace_delete_warning(slug, reach)` and `Nothing here can be undone.` in
    `theme.error`, then "Its folder `<folder>` stays on disk." (dim); `Typed` adds `Ask::input("Type `<slug>` to
    confirm: ", field)` and a wrong slug `Ask::error("that is not the slug; nothing was deleted")`. Hints: warn
    `y delete · Esc keep`, typed `Enter delete · Esc keep`.
  - `T` (D9): `TargetChoices(ws)` asked (one out at a time); its reply opens `Mode::Targeting` with rows: `inherit the
    Settings default (<default_name>)` then each target `<name>  <kind>` (+ `planned · <owner>`, `enabled: false`),
    the current one selected and noted `· current`. `Chosen(Inherit | Target(id))` → `SetWorkspaceTarget { expected:
    scope row's updated_at, target }`; `Disabled` → told "<name> runs once <owner> lands"; no scope row → told.
    `WorkspaceTargetSet { Applied }` → forced overview read; `{ Stale }` → told (D9). Refusals `"target_choices"`/
    `"set_workspace_target"` → told.
  - `e`/`d` on a project row → told (D12).
  - **(fact-check)** `absorbs_failure` (`mod.rs:~1142-1150`) also takes `"delete_reach"` and `"delete_workspace"`:
    the shell asks the origin tab (`App::absorbed`, `app/update.rs:592-597`), and without them a refused delete also
    lands on the status line, which `a_workspace_with_live_runs_is_refused_in_the_box` forbids. M2's refused-read arm
    (`Failed` `"workspace_overview"` with a pending ask, `mod.rs:~1112-1127`) reads `asked == Asked::Stop` and
    otherwise says "Resuming needs the database": it gains an `Asked::Edit` branch (the edit's own sentence), and
    `question_of`'s match (`:~488-563`) one too.
- **Keys**: D14's T4 half (`WorkspacesTarget` 256, `CATALOGUE` 257, `WORKSPACES_BROWSE` gains it and the common
  `[Edit, Delete]` layer); the help-line pin in `hint.rs` gains `T default target` and, **fact-check**, a new
  `Common: e edit · d delete` line after the `Workspaces:` one (a layer of its own in `help_lines`). The tab's hint row is unchanged
  (the verbs are in `?` and the about panel).
- **Tests first**:
  - `views.rs`: `resolve(WORKSPACES_BROWSE, "e") == [Edit]`, `"d"` → `[Delete]`, `"T"` → `[WorkspacesTarget]`.
  - `delete.rs` unit: `the_stages_answer_their_keys` (counting: `y` nothing, `Esc` closes; warn: `y`/`Enter` → typed;
    typed: wrong slug → `wrong`, field cleared; right slug → `Send(DeleteWorkspace)`); `a_refused_delete_goes_back_to_the_warning`.
  - `hierarchy.rs`: `the_workspace_delete_warning_reads_as_before` (the sentence `a_workspace_delete_names_what_its_scope_row_takes`,
    `tests/it/hierarchy.rs:4040`, pins on screen); that integration test stays green.
  - `crates/htui/tests/it/workspaces_manage.rs` (temp home and folders as T3):
    `e_opens_the_workspace_prefilled_and_saves_a_rename` (snapshot `edit_form_100`; rename Platform → "Platform
    Core": the list and, after `o`, the header read it); `renaming_the_open_workspace_moves_the_header`
    (D11); `editing_the_folder_points_at_it_and_moves_nothing` (old folder and its file stay; the overview's folder
    line is the new one); `an_edit_changed_elsewhere_keeps_the_text_and_retries` (a direct `update_workspace` between
    `e` and `Enter`: error line, then `Enter` saves);
    `d_counts_warns_and_deletes_after_the_typed_slug` (snapshots `delete_warn_100`, `delete_typed_100`; a workspace
    created first, so no run blocks it; after: not listed, its folder still on disk);
    `a_wrong_slug_deletes_nothing`; `a_workspace_with_live_runs_is_refused_in_the_box` (Platform: the store's
    sentence in the box, nothing deleted, nothing on the status line);
    `deleting_the_open_workspace_opens_the_first_left` (**fact-check**: create and open a workspace first; the demo's
    open Graphics holds stopped `RUN_8` and Platform live runs, so the store refuses deleting either); `e_and_d_on_a_project_row_say_where_projects_are_edited`;
    `t_lists_the_targets_and_sets_the_workspace_default` (snapshot `target_picker_100`; choose the local target: the
    overview's `default target` line reads `this box`; `T` again, `inherit …`: back to `inherits the Settings
    default (this box)`); `a_planned_target_cannot_be_chosen` (a `docker` target created in the store first);
    `a_target_changed_elsewhere_is_refused_and_said`.
- **Snapshots** (D18): new `it__workspaces_manage__{edit_form,delete_warn,delete_typed,target_picker}_100`; nothing else
  moves (the Hierarchy delete text is byte-identical).
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- ui::tabs::workspaces:: keys:: ui::about:: hierarchy::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_manage::/) or test(/^workspaces_create::/) or test(/^workspaces_tab::/) or test(/^hierarchy::/) or test(/^keys_file::/) or test(/^keys::/) or test(/^no_hardcoded_keys::/) or test(/^shell::/))'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new (D18)
  cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```

### T5: Settings › Hierarchy gives up the workspace row; the retired key; the docs (after T4)

- **Files**:
  - `crates/htui/src/ui/tabs/settings/hierarchy.rs` (D17: the `HierarchyNewWorkspace` arm, `EditorKind::NewWorkspace`
    and `EditWorkspace`, `PathTarget::WorkspaceRoot`, the workspace arm of `begin_delete` become notices or go; the
    `follow` rule's `set_workspace_root` case; `doing`'s `create_workspace`/`update_workspace`/`set_workspace_root`/
    `delete_workspace` entries; `reload`'s workspace arms; `HINT_BROWSE`/`HINT_NO_WORKSPACE` lose `N workspace`;
    `creates_one`/`no_workspace` and `refuse` name the tab's key through `views::WORKSPACES_BROWSE`; the `Workspaces`
    reply arm and `Deleted { Workspace }` arm, which only a workspace delete reached, go; unit tests)
  - `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs` (`HIERARCHY_BROWSE`), `crates/htui/src/keys/load.rs`
    (`RETIRED_ACTIONS`, `entry`, unit tests), `crates/htui/src/keys/mod.rs` (**fact-check**: the doc comments of
    `retired`, `:44-46`, and `notices`, `:288-291`, say "one per retired table"; they name a retired action too.
    No new field: `retired: Vec<KeyFileError>` takes the action's notice on its line)
  - `crates/htui/tests/fixtures/keys/hierarchy_new_workspace.toml` (CREATE: `version = 2`, `[settings.hierarchy]
    new_workspace = ["ctrl-n"]` and `[global] quit = ["ctrl-y"]`), `crates/htui/tests/it/keys_file.rs` (a case over it,
    as M1's `a_key_file_with_the_retired_switcher_table_still_loads_with_a_notice`)
  - `crates/htui/tests/it/hierarchy.rs` (below), `crates/htui/tests/it/hierarchy_pg.rs` (read
    `a_workspace_delete_rebuilds_the_mirror_its_scope_row_was_in`, `:250`: worker-level, expected unchanged)
  - snapshots: Hierarchy ones moved per D18; `docs/htui-keys.md` (regenerated); `README.md` (`:459` the Hierarchy row:
    "The open workspace's projects and repositories (`n`) … workspaces are created, edited and deleted on the
    Workspaces tab"; `:468` "Deleting a project in **Hierarchy** or a workspace on the Workspaces tab shows …";
    `:274` stays: the picker is still Hierarchy's for repos); `docs/htui-editor.md` (`:182-188`: the Workspaces tab's
    own scope moves — opening the first workspace created, deleting the open one, a rename, a project created there —
    and Hierarchy's project create/delete are refused during an edit and not retried; after the edit press `w`,
    select the workspace and press `o`)
- **Behaviour** (`settings/hierarchy.rs`):
  - `on_browse_key`: the `Act::HierarchyNewWorkspace` arm goes; `Act::Edit` on `Row::Workspace` → notice `` `e` on
    the Workspaces tab edits the workspace `` (the chord through `keys.label(views::WORKSPACES_BROWSE, Act::Edit)`;
    unbound: "edit the workspace on the Workspaces tab"); `Act::HierarchyChoosePath` on `Row::Workspace` → `` `e` on
    the Workspaces tab sets its folder ``; `Act::Delete` on `Row::Workspace` → `` `d` on the Workspaces tab deletes
    it ``. `Act::New` on the workspace row still opens `NewProject`.
  - `refuse`: with no tree every key is refused with `no_workspace(new)` where `new` =
    `keys.label(views::WORKSPACES_BROWSE, Act::WorkspacesNew)`: "no workspace — `N` on the Workspaces tab creates one"
    (unbound: "no workspace"); the empty state's next step the same (`creates_one`).
  - The workspace row keeps its line (name, slug, folder on this box): the folder is read only here now.
- **Keys**: D14's T5 half: delete `Act::HierarchyNewWorkspace` everywhere; `WorkspacesNew` to `position` 56 and its
  `ALL` slot (row moved to the `[workspaces]` block, after `WorkspacesOpen`'s M2 neighbours); `WorkspacesTarget` 256 →
  255; `CATALOGUE.len()` 256 with a comment ("MOD-116 M3 D14: `workspaces.new` took the retired
  `settings.hierarchy.new_workspace`'s slot"); `HIERARCHY_BROWSE` loses the act; `RETIRED_ACTIONS: &[(&str, &str,
  &str)] = &[("settings.hierarchy", "new_workspace", "<D14's notice>")]` checked in `Loader::entry` before the "no
  such action" refusal, pushing to `keys.retired` on the key's line; the `[settings.hierarchy]` "has …" list in that
  refusal no longer names `new_workspace`.
- **Tests first**:
  - `load.rs` unit: `the_retired_new_workspace_key_loads_with_one_notice` (any value, even a bad chord, one notice on
    its line, nothing bound); `keys_file.rs`: `a_key_file_naming_hierarchy_s_new_workspace_still_loads_with_a_notice`
    (the binary starts, `--print-keys` exits 0 with `[global] quit` changed and `[workspaces] new = ["N"]`; stderr
    names the fixture's line; `load_path` answers one notice).
  - `views.rs`: `resolve(HIERARCHY_BROWSE, "N") == []`.
  - `tests/it/hierarchy.rs`: the section-level cases that press `N`, or `e`/`b`/`d` with the cursor on row 0 (the
    workspace row is row 0, so a case that presses them without moving acts on it) are, after reading each: moved to
    a project row (`j` first) when their point is the editor, picker or delete mechanics; replaced by a notice
    assertion when their point is the workspace row; or deleted when the Workspaces tab's T3/T4 cases cover them
    (say which in the commit). Candidates (confirm by reading): `deleting_the_last_workspace_leaves_the_pane_empty`
    (`:1868`), `b_on_the_workspace_row_picks_the_root` (`:2179`), `a_root_write_that_applied_is_followed_by_one_inference`
    (`:2420`: the flow is gone; the worker-level inference tests stay), `a_rebound_new_workspace_key_is_the_one_the_empty_tree_names`
    (`:3644` → the tab's `new` rebound), `the_empty_states_carry_a_next_step` (`:3952`),
    `a_workspace_delete_names_what_its_scope_row_takes` (`:4040`, to a direct `delete_pane` or the tab),
    `the_workspace_editor_shows_the_folder_line` (`:4379`), `a_workspace_with_a_stopped_run_is_refused_on_the_status_line`
    (`:3340`), `picking_in_the_shell_lists_the_root_and_keeps_h_and_l` (`:3257`), `a_directory_gone_before_the_choice_is_refused_on_the_status_line`
    (`:3307`), `ctrl_d_in_browse_deletes_nothing` (`:3435`), `a_stale_reply_keeps_the_typed_text` (`:1737`),
    `a_second_enter_does_not_resend_the_write` (`:1937`), `a_tree_that_lands_mid_delete_keeps_the_confirmation`
    (`:1965`), `a_refused_count_leaves_the_confirmation` (`:2011`), the editor-key cases `:3488-3540`, `:3811-3950`.
    New: `e_b_and_d_on_the_workspace_row_name_the_workspaces_tab`, `n_on_the_workspace_row_still_creates_a_project`.
  - Worker-level cases (`create_workspace_answers_its_own_tree`, `a_root_is_stored_canonical_and_a_link_is_refused`,
    `a_stale_update_answers_the_current_tree`, `delete_workspace_keeps_its_projects`, the inference cases): unchanged.
- **Snapshots** (D18): the Hierarchy section's frames move in their hint row (`N workspace` gone) and, where drawn,
  its empty state; list them from `cargo insta pending-snapshots` (expected `it__hierarchy__*`, `it__settings__*`
  frames showing Hierarchy, and Hierarchy's own unit snapshots). Rewritten cases' snapshots change with them. Nothing
  else moves.
- **Check**:
  ```bash
  cargo test -p htui --all-features --lib -- ui::tabs::settings:: keys:: hierarchy::
  env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it)'
  cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new (D18)
  cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
  cargo clippy -p htui --all-targets --all-features -- -D warnings
  cargo clippy -p htui -- -D warnings
  cargo fmt -p htui --check
  ```
  (`binary(it)` of `htui` whole: T5 edits tests across `hierarchy.rs`, the largest module of that binary.)

## Validation (after T5, the merged tree)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings
env -u HTUI_TEST_QDRANT_URL bash scripts/gate.sh          # grep the log for SIGABRT
cargo run -q -p htui -- --default-keys --print-keys | diff - <(sed -n '/^```toml$/,/^```$/p' docs/htui-keys.md | sed '1d;$d')
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

A manual run in the sandbox with a scratch `HOME` (`HOME=$(mktemp -d) cargo run -p htui -- --demo`; `--demo` writes
folders on disk too): `w`, `N`, `Enter`, type `Mobile`, `Enter` → `Mobile ○ stopped` and `$HOME/workspaces/mobile`;
`N`, `j`, `Enter` on Mobile, source `new repo`, `Enter` → `projects/<slug>` with one empty commit; a git URL of a
local bare repo clones with the progress line; `e` renames, `T` sets `this box`, `d` with the typed slug deletes and
the folder stays; Settings › Hierarchy no longer offers `N`.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A test creates folders or clones in the real `~/workspaces` | Medium | D3: the tab sends absolute paths and every test gives it a temp home; worker tests use `serve_with_home` |
| A clone hangs on a prompt or a dead host | Low | `GIT_TERMINAL_PROMPT=0`, ssh `BatchMode`/`ConnectTimeout`, the low-speed abort, `CLONE_TIMEOUT`; off the loop (D7) |
| A failure leaves a half-registered project or a stray clone | Medium | D6 orders disk before rows and compensates with `delete_project` + removal of what this request made; three tests pin it |
| A URL's token reaches the screen, a log or the database | Low | `redact_urls` on git's line, `redact_remote` on the stored remote and the progress line; a test with `user:secret@` |
| A `git` with `commit.gpgsign` or a global hook breaks the new repo | Low | `-c commit.gpgsign=false`, `-c core.hooksPath=/dev/null` (fact-check: `--no-verify` alone leaves `prepare-commit-msg` and `post-commit`), `IDENTITY_ENV` (D8) |
| An edit with a stale token leaves an empty folder it created | Low | said in D5; nothing points at it; the retry uses it |
| Hierarchy test churn in T5 hides a lost behaviour | Medium | T5 names each candidate and says per case why it moved; D18 snapshot review |
| The worker's `CreateWorkspace`/`UpdateWorkspace`/`SetWorkspaceRoot` stay with no UI sender | Certain | D17: kept and tested on purpose; removal named for MOD-123 |
| M2's names (`Question::Told`, `WORKSPACES_CONFIRM`, `HINT_*`, `Asked`) differ when it lands | Resolved | fact-check at `d6cd0909`: `Question::Told { title, text }`, `tell`, `pending: Option<(WorkspaceId, Asked)>`, `Asked::{Stop, Resume}`, `HINT_IDLE`/`HINT_STOP`/`HINT_FORCE`/`HINT_RESUME`, `views::WORKSPACES_CONFIRM` all exist as cited; an M2 fix round may still move `ui/tabs/workspaces/*` lines |
| Parallel items (MOD-127, MOD-130 M2) touch `store_worker.rs`, `catalogue.rs` | Medium | additions beside named anchors; positions named in D14 |

## Acceptance

- [ ] T1-T5 complete; no store trait, migration or `.sqlx` change
- [ ] `N` offers workspace and project; with no workspace it opens the workspace form; component/requirement absent
- [ ] A new workspace has its folder on disk (default or typed), a folder row only when not the default, reads
  `○ stopped`, and is opened only when none is open
- [ ] A new project registers from an existing git folder (a plain folder refused naming MOD-145), a git URL (cloned
  into `<folder>/projects/<slug>`, remote redacted) or a new repo (one empty commit, HEAD resolves, nothing in its
  tree); clone and init run off the loop with a progress line; a failure leaves no project and no directory it made
- [ ] `e` edits name, slug, description and folder (nothing moved on disk); `d` counts, warns and asks for the slug;
  `T` sets or clears the workspace's default target and the overview follows
- [ ] Settings › Hierarchy no longer creates, edits, re-roots or deletes a workspace and points at the tab; a
  `keys.toml` naming `new_workspace` still loads with one notice
- [ ] Every key a catalogue row; `docs/htui-keys.md` matches `--print-keys`; README and `docs/htui-editor.md` updated
- [ ] Validation passes; patterns mirrored, not reinvented

## Owed on the host

- **Real clones**: a private repository over ssh (the maintainer's agent) and over https (a credential helper) clone
  without a prompt; a mistyped host fails within ~20 s with git's line.
- **Real use**: create a workspace and one project per source against the maintainer's database; check
  `workspace_box_path`, `repo_box_path` and `~/workspaces`.
- **Postgres gate** on the merged tree (`hierarchy_pg::`, the conformance suites untouched) and the live-Qdrant
  suites (sandbox limits); Windows `scripts/gate.ps1` (paths, `~` expansion, `\\?\` canonical prefixes) is not run
  here.

## Open questions

None changes M3's scope. OQ1 is settled by the maintainer; the rest are settled here at the recommendation, named so
the maintainer can override:

- **OQ1 — Cancelling a clone. SETTLED (maintainer, 2026-11-15): no clone cancel in M3.** The PRD's risk row now
  says so ("no cancel … `Esc` closes the form and the clone finishes in the background"). M3 bounds the clone instead
  (D7: ten minutes, the low-speed abort, ssh's connect timeout, no prompt) and lets `Esc` close the form while the
  clone finishes off the loop. A cancel needs a token threaded into `Cli`'s verb
  loop, a `CancelProjectCreate` request and a task registry in the store worker (the `InstallCancel` shape); it can
  follow if real use asks for it.
- **OQ2 — Component and requirement in "New…".** Left out until MOD-117/MOD-118 (D1). The alternative, disabled rows
  naming the item, is one line each in the chooser.
- **OQ3 — Opening a new workspace.** Not opened, except when none is open (D4). The alternative, always opening it, is
  one `SetScope` in the reply arm.
- **OQ4 — Choosing folders.** Typed paths (with `~`) only; Hierarchy's directory picker is not wired into the forms
  (D19). The picker could open from the folder fields with a `ctrl` chord later.
- **OQ5 — Planned targets.** Listed but not choosable (D9) until their kind runs; MOD-130's store would accept them.
- **PRD OQ (clone transport)** — settled: the `git` binary through `isolate::git::Cli` (D8), not `gix` clone: the
  box's own credentials (ssh agent, credential helpers, `core.sshCommand`) apply as they do for landing's push.
- **PRD OQ (`T` without MOD-130's resolver)** — settled: the workspace level is written; the overview keeps M1's line
  (D9).

## Verified claims

Fact-checked at `d6cd0909` (branch `hr/MOD-116`, M2 landed at `1692b8bb`), 2026-11-15. Method: Gortex `read`/`search`
on the Rust source at HEAD; `cat`/`grep` on docs, fixtures and snapshots; `git` 2.43.0 probes in a `/tmp` scratch
directory (clone, init, empty commit, hooks, signing, empty and mismatched bare remotes); the `gix` 0.87.1 and
`gix-url` 0.38.0 sources in the cargo registry. No compile probe: the plan relies on no Rust construct the tree does
not already use. Of **79** claims: **56 true**, **11 false** (each amended in place, marked "(fact-check)"),
**12 amended** (anchor drift or an imprecise detail made exact). No maintainer question; OQ1 settled by the
maintainer (no clone cancel in M3).

| Claim | Verdict | Evidence |
|---|---|---|
| After M2, `ALL`/`position` end at `LeaveStop` 254 | true | `keys/catalogue.rs:3122`, `:3383` |
| `CATALOGUE.len()` is 255 | true | `catalogue.rs:3422` (`every_act_has_exactly_one_row`) |
| `DECLARED.len()` 103, `[102]` = `WORKSPACES_CONFIRM` | true | `keys/stack.rs:389`, `:486-495` |
| `HierarchyNewWorkspace` has `position` 56 | true | `catalogue.rs:3185`; `ALL` `:2924`; enum `:565` |
| Its row is at `catalogue.rs:~1414-1421` | amended | `:1421-1429` |
| `WorkspacesNew` can be declared after `WorkspacesResume`; positions are explicit, not enum order | true | enum `:619`; `const fn position` `:3127`; `spec()` is a linear find `:1014-1016` |
| `ITEM_FORM` at `views.rs:1155` | amended | `views.rs:1163` (`[view, FIELD, PICK, FORM, MODAL]`) |
| The layers `CONFIRM`, `LIST`, `MODAL`, `FIELD`, `PICK`, `FORM_FIELDS` exist as the stacks need | true | `views.rs:61`, `:70`, `:73`, `:76`, `:170`, `:178` |
| M2's view default `(Workspaces, ConfirmYes, ["enter"])` | true | `catalogue.rs:2642`; `resolve(WORKSPACES_CONFIRM, "enter")` `views.rs:1997` |
| `resolve(WORKSPACES_FORM, "down")` is `FormNextField` | **false** | `field.line_down` binds `down` (`catalogue.rs:2553-2560`): `[FormNextField, FieldLineDown]`, as `views.rs:1669-1672`, `:1945-1948` pin |
| `WORKSPACES_BROWSE` = view, list, pane, layout, history, global; `N`, `T`, `e`, `d` bound by no other layer there | true | `views.rs:1232-1246`; `hint.rs:753-763` lists every chord of those layers |
| M1's retired-key mechanism is table-level and does not cover one retired action (so `RETIRED_ACTIONS` is new) | true | `keys/load.rs:62-66` (`RETIRED: &[(&str, &str)]` by table path), `walk_table` `:369-377` |
| `RETIRED` `load.rs:62`, `walk_table` `:368-378`, its test `:1491-1510` | true | `:62`, `:369-377`, `:1492-1530` |
| `entry` refuses an unknown name with "no such action; … has …" | true | `load.rs:412-446` |
| `keys/mod.rs` needs no change unless a field is added | amended | no field needed, but the doc comments of `retired` (`mod.rs:44-46`) and `notices` (`:288-291`) say "per retired table": listed in T5 |
| Hierarchy editors: `Field` `:335`, `Editor` `:346`, `on_editor_key` `:1251`, `submit` `:1347`, `on_stale` `:1568`, `Editor::form` in `:1986-2033` | true | `settings/hierarchy.rs` at those lines (`Editor::form` `:2022`) |
| `HierarchySection::new` reads `dirs::home_dir()` | true | `settings/hierarchy.rs:460-465` |
| Delete flow `begin_delete` `:1106`, `deleted` `:1210`, `delete_pane` `:2217-2300` | true | same lines; workspace headline `:2252-2279` |
| `DeleteReach`/`Deleted` replies `~1785-1831`; the first listed workspace is entered after a delete `~1820-1830` | true | `:1782-1832` |
| Hierarchy's D11 at `~1515-1525` | true | `:1510-1522` |
| The tab's D11 is "moved" from Hierarchy, and its name half moves the header | **false** | Hierarchy keeps D11 (its `n` still creates projects); `App::observe_reply` sets `top_bar.workspace` from every `Workspaces` reply first (`app/update.rs:620-627`), so the name half never fires |
| `CreateWorkspace` `:285-301`, `UpdateWorkspace` `:~303-311`, `CreateProject` `:330-376` in `serve_with_home` | true | `hierarchy.rs:286`, `:303`, `:331-374`; `serve_with_home` `:268-489` |
| `box_id` `:560`, `canonical` `:577`, `rebuild_mirror` `:597` are private; `workspace_root` `:177` is `pub` | true | same lines |
| D6 refuses an unusable existing folder in a sentence through `canonical` | **false** | `canonical` maps `RootRefusal` to `StoreError::Constraint` (`hierarchy.rs:577-588`), shown as "constraint violated"; amended: `canonical_root` directly, answer `SetupRefused` |
| `create_workspace` writes the scope row; a taken slug is `Constraint`, nothing written | true | `htui-core/src/store/traits.rs:828-835` |
| `update_workspace` is a CAS whose `Stale` carries the current row | true | `traits.rs:837-850` |
| `set_project_target` `~3079` (scope row = workspace level); `default_target` on `WriteStore`; `targets()` lists retired too, by name | true | `traits.rs:3079-3084`, `:3107`, `:416-421` |
| `project_slug_refusal` at `traits.rs:~4166` | true | `traits.rs:4166` |
| `TargetKind::runnable()`/`owner()`; docker is `MOD-44` | true | `htui-core/src/model/target.rs:110-124` |
| `seed::LOCAL_TARGET_NAME` is the default with no row ("this box") | true | `htui-core/src/seed.rs:550` |
| M1's `workspaces::default_target` at `workspaces.rs:~215` | amended | `workspaces.rs:273`, private: T2 lists `workspaces.rs` to make it `pub(crate)` |
| MOD-130's resolver is not on the tree | true | only MOD-130 M1 commits in `git log`; no resolver symbol |
| `Cli::run` scrubs `SCRUBBED_ENV` and `HTUI_OWN_ENV`, sets `LC_ALL=C` and `GIT_TERMINAL_PROMPT=0`, kills the group on timeout | true | `isolate/git.rs:336-360` (also `GIT_OPTIONAL_LOCKS=0`, `GIT_ADVICE=0`, `kill_on_drop`), `:366-387` |
| `NETWORK_ENV` is the low-speed abort; ssh `BatchMode=yes`, `ConnectTimeout=20` | true | `git.rs:47-57` (`SSH_OPTIONS`, 1000 B/s for 30 s) |
| `IDENTITY_ENV` commits as `htui <htui@localhost>` | true | `git.rs:131-136`; probe: `git log` reads `htui <htui@localhost>` |
| `SCRUBBED_ENV` is the discovery-redirecting variables | true | `git.rs:113-124` (ten) |
| `clone_budget` is `CLONE_TIMEOUT` or a shorter own budget, "as `ls_remote_budget`" | **false** | `ls_remote_budget` is `self.budget.min(…)` (`git.rs:1224-1226`) and `self.budget` is 120 s in production (`:293`): that shape caps clones at 2 min; amended to the Types rule (`self.budget` only below `VERB_TIMEOUT`) |
| `network_ssh`, `network_env`, `network_failure`, `redact_urls`, `redact_remote` exist as cited | true | `git.rs:1231`, `:1067`, `:1112`; `infer.rs:178`, `:205` (both `pub`) |
| `blocking` and `Exited::failure` | true | `git.rs:1402` (`pub(crate)`), `:1465` (answers `Git`) |
| `gix::open` does not discover upward; the not-a-repository variant | true | `gix-0.87.1/src/open/repository.rs:74-92`; `open::Error::NotARepository { source, path }` |
| `head_name`, `find_remote`, `workdir`/`is_bare` exist; a remote URL's text keeps its userinfo (raw) | true | `gix-0.87.1/src/repository/reference.rs:219`, `remote.rs:68`, `location.rs:97`, `worktree.rs:101`; `gix-url-0.38.0/src/lib.rs:525-638` writes the password |
| `-c protocol.ext.allow=never clone --quiet -- <url> <dest>`: an option-like URL stays a URL, `ext::` is refused, a missing URL fails with no dest, git prints no userinfo | true | probe: `--upload-pack=…` → `repository '--upload-pack=…' does not exist` (128, nothing run); `transport 'ext' not allowed`; missing → 128, dest absent; `https://user:secret@127.0.0.1:9/x.git` → `unable to access 'https://127.0.0.1:9/x.git/'` in 0.03 s; non-empty dest refused |
| A bare remote built as `gix_isolator.rs:4527` clones with the source's `HEAD` on `main` | **false** | probe: that helper's `init --bare` keeps `HEAD -> master` while the push lands `main`; the clone exits 0 with an unborn `HEAD` on `master` ("remote HEAD refers to nonexistent ref"); with `--initial-branch=main` it lands `842d768…` on `main` |
| `init --initial-branch` + `-c commit.gpgsign=false commit --allow-empty` under `IDENTITY_ENV` gives one empty commit, empty tree, clean status | true | probe: exit 0, `ls-tree -r HEAD` 0 entries, `status --porcelain` 0 lines, `HEAD -> main`; with a global `commit.gpgsign = true` and no flag the commit fails (`cannot run gpg`) |
| `--no-verify` means no user hook runs | **false** | githooks(5): it skips `pre-commit` and `commit-msg` only; probe: `-c core.hooksPath=/dev/null` runs none of four failing global hooks; amended (D8, T1, risk row) |
| `NewProject` is served "exactly as `ListDir`" (`~3851-3862`), cloning the backend | **false** | `ListDir` (`store_worker.rs:3895-3906`) clones the request's fields, `seq`, `origin`, `tx` and no backend; amended: this task also clones `backend` (`Backend: Clone`, `htui-store/src/backend.rs:64`) |
| `ListDir`'s inline arm `~2829`, `list_dir` `~3194` | amended | `:2873`, `:3238` |
| M1 requests `~476-490`, `name()` `~1415`, `name_arms_are_stable` `~6865`, workspaces or-arm `~2752`, `round_trip` `~6279` | amended | `:476-510` (M2's two added), `:1439` (arms `:1489-1491`), `:6909`, `:2794-2796`, `:4317` |
| The shell drops a reply that is not the newest of its (origin, kind) | true | `app/state.rs:480-481`, `:1124-1150` |
| The five request names are free | true | no `"new_workspace"` etc. request name in `crates/htui/src` (the catalogue's `new_workspace` is a key name) |
| D16: the counts are asked only when the open workspace has a project | true | `app/update.rs:390-396`; `Scope::is_empty` = `project_ids.is_empty()` (`htui-core/src/model/scope.rs:39-41`); only `set_scope` (`:465-466`) and the tab's activation (`wants_requests`) still ask |
| The tick tests' counts (four with a scope, two with none) | true | `update.rs:1422-1443`, `:1651-1665` |
| M2's tab names: `Question::Told`, `tell`, `pending`/`Asked`, `HINT_*`, `views::WORKSPACES_CONFIRM`, `question.rs` | true | `ui/tabs/workspaces/mod.rs:~240-282`, `:~565`, `:~319`, `:~85-114`; `views.rs:1009-1010`; `question.rs` (pure texts) |
| The tab's empty state at `mod.rs:~429-440` | amended | `mod.rs:~781-793`, through `views::HIERARCHY_BROWSE`'s `HierarchyNewWorkspace` |
| `absorbs_failure` with `REQUEST_NAMES` keeps the tab's refusals off the status line | **false** | the shell asks the origin tab (`app/update.rs:592-597`); T4's delete sends `"delete_reach"`/`"delete_workspace"` from the tab, which the plan did not absorb though a T4 test forbids the status line; amended |
| A temp home on the tab keeps every integration test out of the real `~/workspaces` | **false** | `Harness` serves through `store_worker::serve` (`testkit.rs:402`, `:422`), so the worker uses the real `dirs::home_dir()`; M1's `demo()` points folder rows at `/srv/…` (`tests/it/workspaces_tab.rs:89-107`); amended (D3, T3) |
| T3's Files cover every file its change breaks | **false** | `tests/it/workspaces_tab.rs:357-367` asserts the old empty-state wording; added to T3 |
| T4's `deleting_the_open_workspace_opens_the_first_left` runs on the demo as written | **false** | the open Graphics holds stopped `RUN_8` and Platform live runs (`workspaces_tab.rs:1-7`), so the store refuses both deletes; amended: create and open a workspace first |
| T3's key changes touch only the named pins | amended | `views.rs` also pins capturing stacks (`:2159-2195`) and checks acts per stack over `VIEWS` (`:1337`, `:2393`, `:2422`): new files join `VIEWS` |
| T4's help-line pin gains only `T default target` | amended | the new `common` layer adds a `Common: e edit · d delete` line |
| D6/D8's refusals answer git's line | amended | `IsolateError`'s `Display` prefixes `isolation refused: ` (`htui-orch/src/isolate.rs:51-52`): the worker uses the inner string |
| A clone that exits 0 has a `HEAD` to register | amended | probe: an empty remote clones with exit 0 and an unborn `HEAD` ("cloned an empty repository"): branch from `repo_facts`, no `head` post-condition |
| An existing folder registers from the typed path | amended | `gix::open` opens a typed `…/.git` as the git dir (`repository.rs:74`): name and path from the work dir |
| The Hierarchy test candidates' lines (`:1737`, `:1868`, `:1937`, `:1965`, `:2011`, `:2179`, `:2420`, `:3257`, `:3307`, `:3340`, `:3435`, `:3644`, `:3952`, `:4040`, `:4379`) | true | `tests/it/hierarchy.rs`, each `fn` at that line |
| `a_workspace_delete_rebuilds_the_mirror_its_scope_row_was_in` at `hierarchy_pg.rs:250` | true | same line |
| Worker-level Hierarchy cases exist | true | `tests/it/hierarchy.rs:166`, `:393`, `:439`, `:877` |
| README `:181-183`, `:189`, `:274`, `:459`, `:468`; `docs/htui-editor.md:182-188` | true | `README.md:183`, `:189`, `:274`, `:459`, `:468`; `htui-editor.md:182-188` |
| `docs/htui-keys.md` is one `toml` block holding `[settings.hierarchy] new_workspace` | true | one fence; `htui-keys.md:116-117` |
| `workspace_folder` is re-exported from `htui_core::model` | true | `htui-core/src/model/mod.rs:135-138` |
| `htui-orch`'s `tests/it/main.rs`, `git.rs`'s `mod tests` `:2802` and `testkit` `:2554` | true | same; `skip_without_git!` is `#[macro_export]` (`git.rs:2585-2597`) |
| `htui`'s tests can use `skip_without_git!` | true | `crates/htui/Cargo.toml:105` (`htui-orch` dev-dep with `test-support`) |
| `git_as`/`git_ok` `gix_isolator.rs:2618-2632`, `bare_remote` `:4527-4549` | true | same lines (`bare_remote` private) |
| `WORKSPACES_DIR`/`workspace_folder` `model/hierarchy.rs:279-305`, tests `:340-380` | true | same lines |
| Kit `Ask` has `input` and `error` | true | `ui/kit/modal.rs:171-226` (builder methods) |
| `changed_elsewhere` is reachable from the tab | true | `ui/tabs/settings/mod.rs:235` (`pub(crate)`) |
| Every user of `HierarchyNewWorkspace` is in T3 or T5 | true | `catalogue.rs`, `views.rs`, `settings/hierarchy.rs`, `ui/tabs/workspaces/mod.rs` only |
| One serial lane; T1 shares no file with T2-T5 | true | Files lists; T3-T5 share `keys/*` and `ui/tabs/workspaces/*`, serial |
| `shell_empty` shows the old next step in both panes | true | `src/snapshots/htui__testkit__tests__shell_empty.snap:21` |
| The PRD's risk row says no cancel (OQ1) | true | `.claude/prds/mod-116-workspaces-tab.prd.md:160` |
