# MOD-116 M3 R1b — review fix round: the tab, Settings › Hierarchy and the docs

Three `rust-reviewer` instances reviewed `47e4ffa6..30f4fb69`; this brief carries the tab/forms area's and the
Hierarchy/docs area's findings (approve with fixes, no high). R1a (git and the worker) lands first on this branch.
**Confirm each finding against the code before acting**; one that does not hold is reported as rejected with
evidence. Plan: `.claude/plans/mod-116-m3-create-and-edit.plan.md` (D3, D10, T3, T4 where cited).

Files you may touch: `crates/htui/src/ui/tabs/workspaces/{mod,forms,chooser,delete}.rs`,
`crates/htui/tests/it/{workspaces_create,workspaces_manage}.rs` and their snapshots,
`crates/htui/src/ui/tabs/settings/hierarchy.rs`, `crates/htui/tests/it/hierarchy.rs` and its snapshots,
`docs/htui-editor.md`. Tests first for every behaviour fix; review every `.snap.new`. No test writes into the
real home.

## Medium

- **B1 — the project form's source key is never visible.** The form draws in the detail pane but its hint goes on
  the focused pane (the list), where at 100 columns it is cut before `h/l source` (`mod.rs:~2021`, `form_hints`
  `:~1578`); the detail border is empty (`it__workspaces_create__project_form_new_repo_100.snap`,
  `…project_progress_100`, `…project_refused_100`). Plan T3: "Hint on the pane border: `Enter create · Esc cancel ·
  Tab field` (+ `h/l source`)". Fix: while a form is open, its hints go on the form pane's own border (draw the
  form pane as the focused one, or give it its hint line), so `h/l source` shows at 100 columns. Snapshots move:
  review them.
- **B2 — `a_taken_slug_keeps_the_form_and_says_so` passes without any refusal**
  (`workspaces_create.rs:~316-328`): its row check also matches the busy line. Fix: assert the store's taken-slug
  words (or that the busy line is gone and the error line shows), plus a snapshot if useful.
- **B3 — deleting the last workspace is not tested** (Hierarchy's `deleting_the_last_workspace_leaves_the_pane_empty`
  was deleted; `mod.rs:~1802-1812`, plan D10's "or none, leaving the empty state"). Add a `workspaces_manage.rs`
  case: a store with one workspace and no runs, open it, delete it; no `SetScope` is emitted, the tab shows its
  empty state naming `N`, and Settings › Hierarchy then shows "no workspace — `N` on the Workspaces tab creates
  one" with `e`/`n` sending nothing.

## Low

- **B4 — a create reply closes a form that is not its own** (`mod.rs:~1465` `workspace_created`): submit, `Esc`
  while busy, `N`, type: the first reply closes the new form. Fix: close only a busy form (as `project_created` and
  `setup_refused` check). Test.
- **B5 — the busy hint still says `Esc cancel` and offers `Enter create`** (`forms.rs:~812`, `mod.rs:~1578`;
  `…project_progress_100.snap`): `Esc` only closes (no clone cancel, maintainer 2026-11-15) and `Enter` is
  swallowed. Fix: a busy hint spec, e.g. `Esc close`.
- **B6 — the preview accepts a path the submit refuses** (`forms.rs:~375-389`): a relative `rel/web` previews as
  created. Fix: build the preview from `expanded(..)`; when it gives `None`, show the "type the folder" line. Test.
- **B7 — the git URL field ignores the `~` rule** (`forms.rs:~673-675`): `~/src/x.git` reaches git unexpanded; a
  relative path resolves against `<workspace folder>/projects`. Fix: a URL with no scheme that is not `host:path`
  (scp form) is a local path: expand `~` and refuse a relative one (D3). Update the integration test that clones
  `../../remote.git` to pass an absolute local path (keep the progress snapshot independent of the temp dir, e.g.
  by asserting text rather than snapshotting that frame, or by redacting the temp prefix).
- **B8 — tests that check nothing.** `workspaces_manage.rs:~390` `a_workspace_with_live_runs_is_refused_in_the_box`
  asserts only the stopped-run refusal: rename it to what it pins, and add a case where the scope row has a live
  run (or cite the store-level test covering it in the doc). `workspaces_create.rs:~196`
  `row_with(&frame, "workspace ").contains("workspace")` is always true: assert the chooser's row text. `forms.rs:~1238-1243`
  `a_url_s_busy_line_drops_its_userinfo_and_counts_seconds` passes a literal `2`: assert the rendered busy line's
  `· 2 s` after two ticks.
- **B9 — Hierarchy's "no root" notice leads nowhere** (`settings/hierarchy.rs` `inferred_notice` ~`:2017-2030`): it
  sends the person to `e` on the Workspaces tab to set the folder, but that edit no longer runs inference. Fix: the
  notice adds "then `i` here" (through the `settings.hierarchy.infer` label, following a rebound/unbound key as the
  other notices do). Test.
- **B10 — unreachable workspace-delete code in Hierarchy** (`hierarchy.rs:~1147`, `~2163`, `~1743`): narrow the
  stored delete target to projects (or remove the workspace arms) so no path can leave `Mode::Deleting` in flight.
- **B11 — `docs/htui-editor.md:~189` overstates how a refused scope move recovers**: only a project created on the
  tab is moved again on the next re-read; creating the first workspace and deleting the open one move once, and the
  person then picks a workspace (`w`, select, `o`). Fix the sentence.

## Check

```bash
cargo test -p htui --all-features --lib -- ui::tabs::workspaces:: ui::tabs::settings:: keys::
env -u HTUI_TEST_QDRANT_URL cargo nextest run -p htui --all-features -E 'binary(it) and (test(/^workspaces_create::/) or test(/^workspaces_manage::/) or test(/^workspaces_tab::/) or test(/^hierarchy::/) or test(/^shell::/))'
cargo insta test -p htui --all-features; cargo insta pending-snapshots   # review every .snap.new
cargo clippy -p htui --all-targets --all-features -- -D warnings
cargo clippy -p htui -- -D warnings
cargo fmt -p htui --check
```
