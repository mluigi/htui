# MOD-116 M3 R1a — review fix round: git helpers and the worker side

Three `rust-reviewer` instances reviewed `47e4ffa6..30f4fb69`; this brief carries the git/worker area's findings
(changes requested, no high). **Confirm each finding against the code before acting**; one that does not hold is
reported as rejected with evidence. Plan: `.claude/plans/mod-116-m3-create-and-edit.plan.md` (D3-D9 where cited).
Reviewer probes ran with real git 2.43 in `/tmp/rv-m3` (may be gone; reproduce in your own `/tmp` dir).

Files you may touch: `crates/htui-orch/src/isolate/git.rs`, `crates/htui-orch/tests/it/git_setup.rs`,
`crates/htui/src/workspace_setup.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/workspaces.rs`.
Tests first for every behaviour fix. No test writes into the real home.

## Medium

- **A1 — a clone can still pop a credential prompt.** `Cli::command` (`git.rs:~352-371`) and `NETWORK_ENV`
  (`:~62`) set `GIT_TERMINAL_PROMPT=0`, but git still asks `GIT_ASKPASS` / `core.askPass` / `SSH_ASKPASS` first: a
  private https URL on a desktop with `SSH_ASKPASS` set opens a dialog and the clone hangs up to 10 min (D7: htui
  never prompts). Reviewer evidence: a local 401 server with `GIT_ASKPASS` or only `SSH_ASKPASS` pointing at a
  logging script: the script was asked for Username and Password; with `GIT_ASKPASS=` (empty) it was not. Fix: add
  `("GIT_ASKPASS", "")` to `NETWORK_ENV` (so `ls_remote` and `push` get it too) or at least the clone's env. Test
  (`git_setup`): a tiny local HTTP server answering 401 (std `TcpListener` thread), `SSH_ASKPASS` set to a script
  that records it ran (write it with `tests/rust/write_executable.rs` if the crate has it, else the crate's
  existing helper; never `fs::write` + chmod): the clone fails readably and the script never ran.
- **A2 — a failed create can delete a folder that already existed.** `prepare_folder`
  (`workspace_setup.rs:~607-620`) sets `made` when the *typed* path is missing; `undo_folder` (`:~648-656`) removes
  the *canonical* folder. `/x/new/../empty` (`new` missing, `/x/empty` existing and empty) gives `made = true`,
  canonical `/x/empty`, and a later failure (taken slug `:~285`, refused box) deletes `/x/empty`. Fix: refuse a typed
  folder with a `..` component (readable `SetupRefused`), and decide `made` from the leaf itself: `create_dir_all`
  on the parent, then `create_dir` on the leaf, `made = true` only when that `create_dir` succeeded
  (`AlreadyExists` → use it as it is). `WorkspaceCreated.made` then tells the truth. Tests: the `..` path is
  refused; an existing empty folder survives a failed create.
- **A3 — a workspace folder inside a repository's work tree is accepted (R-ID-4).** `repo_facts` uses `gix::open`
  (no upward discovery), so only a repository's top is refused; `~/code/api/ws` inside the `api` checkout is
  accepted and htui then writes `projects/` (and later MOD-117's files) into a managed repo. Fix: in
  `prepare_folder`, and in D6 before creating `projects/` in an existing workspace folder, walk up from the
  canonical folder (`gix::discover` or `repo_facts` per parent) and refuse a folder inside a work tree with the
  same R-ID-4 sentence. A bare repository's directory counts too. Keep `repo_facts`' own no-discovery behaviour
  (its test pins it) — add the upward check beside it. Tests: a subfolder of a repository is refused as a
  workspace folder; a project create into a workspace whose stored folder is inside a repo is refused before
  anything is written.

## Low

- **A4 — the destination is not claimed atomically** (`workspace_setup.rs:~853` `create_dir_all(&dest)`, clone +
  `remove_tree(dest)` on failure `:~829-834`): a folder that appears in between is initialised into and later
  removed whole. Fix: `create_dir_all` the parent, `create_dir` the leaf (fail on `AlreadyExists` with the existing
  "already exists" refusal), clone into that empty folder (git accepts an empty destination), init into it; remove
  only a folder this request claimed. Test: a destination created between the check and the claim is refused and
  left untouched.
- **A5 — an inherited `GIT_ALLOW_PROTOCOL` overrides the `ext` guard** (`git.rs:~1106-1115`; `SCRUBBED_ENV`
  `:~121` keeps it). Reviewer: `GIT_ALLOW_PROTOCOL=ext git -c protocol.ext.allow=never clone -- 'ext::…'` ran the
  command. Fix: in `clone_repo` set `GIT_ALLOW_PROTOCOL=file:git:http:https:ssh` (or remove the variable; setting
  it is stronger). Test: with `GIT_ALLOW_PROTOCOL=ext` in the parent env, an `ext::` URL is refused and its command
  never runs (a marker file stays absent).
- **A6 — a panic in the spawned `NewProject` task leaves the tab stuck** (`store_worker.rs:~4043-4054`): no reply,
  so the tab's "a project is still being created" gate refuses every later project. Fix: keep the `JoinHandle`
  (or wrap) and answer `failed("new_project", …)` when the task does not finish normally. Test via a seam that
  panics, or document why one is impossible and test the join-error path.
- **A7 — store failures between two writes leave half-written rows** (`workspace_setup.rs:~289-297` new,
  `:~368-376` edit): an `upsert_workspace_box_path` failure after `create_workspace` leaves a workspace with no
  folder row (resolving to the default) and the made folder; in edit an applied update with a failed upsert moves
  the folder to the new slug's default (D5 forbids); an `update_workspace` error leaves the folder the edit made.
  Fix: a failed upsert after a create deletes the new workspace and undoes its folder (as D6 does for projects);
  in edit, undo the made folder on an `update_workspace` error, and on a failed upsert after an applied slug change
  report the failure clearly (restore the previous slug through a second CAS update if it is safe; else say the
  folder row could not be written and name the old folder). Tests over a store seam that fails the upsert.
- **A8 — the registered path is guessed from the folder name** (`workspace_setup.rs:~724`): only a final `.git` is
  stripped; a linked worktree's or submodule's git dir registers a checkout inside `.git`. Fix: `repo_facts`
  returns the repository's work directory (`repo.workdir()`), and that is the registered path and name source.
  Test with a linked worktree (`git worktree add`) typed by its git dir, or a typed path into `.git`.
- **A9 — tests that do not prove their names, and one dropped failure.**
  - `store_worker.rs` `a_new_project_is_answered_from_its_own_task`: inline serving would pass it too. Make the
    `NewProject` hang (e.g. clone from a local listener that accepts and never answers, with a short test budget)
    and assert a later request's reply (e.g. `Workspaces`) arrives first.
  - `workspaces.rs` `a_lost_connection_ends_the_batch_and_rides_in_the_reply`: assert the second target was never
    tried (a counting seam), not only that both read `Unreachable`.
  - `workspace_setup.rs:~457` `let _ = remove_tree(dest.clone())`: a failed removal is reported in the reason ("…
    and `<dest>` could not be removed"), as T2 asked.

## Check

```bash
cargo test -p htui-orch --all-features --lib -- isolate::git::
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui-orch --all-features -E 'binary(it) and test(/^git_setup::/)'
cargo test -p htui --all-features --lib -- workspace_setup:: store_worker:: workspaces:: hierarchy::
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_create::/) or test(/^workspaces_manage::/) or test(/^hierarchy::/))'
cargo clippy -p htui-orch -p htui --all-targets --all-features -- -D warnings
cargo clippy -p htui-orch -p htui -- -D warnings
cargo fmt -p htui-orch -p htui --check
```
