# Plan: MOD-15 milestone 3 — the app can take typed input

**Source**: `.claude/prds/mod-15-hierarchy-management.prd.md`, milestone 3 only (`:251`). Design
authority: the PRD's Constraints (`:225-240`), D1, D8, D11, D13 (cited as **PRD Dn**),
milestone 1's plan (`.claude/plans/mod-15-hierarchy-seam.plan.md`, **M1 Dn**), milestone 2's plan
(`.claude/plans/mod-15-project-seed.plan.md`, **M2 Dn**), `docs/ANA-10.md` §4.9 (`:1240-1345`)
and M0 (`:2411-2414`, `:2504-2526`), `HANDOFF.md:284-360`. This plan's own decisions are plain
**Dn**.
**Requirements**: `R-ENT-1..4`, `R-BOX-4` (per-box paths), `R-TUI-8`, `R-ENT-10` (no
last-writer-wins), `R-NF-3` by ownership.
**Bare filenames below resolve to two crates** (fact-check F-14): `backend.rs`, `writer.rs`,
`cache/mod.rs`, `cache/refresh.rs`, `pg/*` and `mem.rs`-adjacent store code are
`crates/htui-store/src/…` (and `mem.rs`/`traits.rs`/`conformance.rs` are
`crates/htui-core/src/store/…`); `store_worker.rs`, `testkit.rs`, `app/update.rs`, `ui/**` are
`crates/htui/src/…`.
**Complexity**: High (one shared widget, one trait method, twelve `StoreRequest` and four
`StoreReply` variants, one worker module, one section with three modes, one guard in `htui-core`;
no seam change, no migration, no new query).
**Routing**: routed as **PRD** by `/handoff-run MOD-15`. Reviewer: `rust-reviewer`
(`.claude/workflow-config.json`). Models per maintainer: plan and reviewer on Fable, every
implementer on Opus.

## Summary

Milestones 1 and 2 left a seam that can create, edit (compare-and-set on `updated_at`) and delete
the hierarchy (`traits.rs:299-608`) and a Settings tab with exactly one section, `AgentsSection`
(`app/mod.rs:47-49`), whose keys are single letters and whose `on_key` returns `Handled::Pass` for
everything it does not bind (`agents.rs:1205`). Nothing in `crates/htui` takes typed text except
the chat composer, which is "a mode, not a widget with focus" and exposes its buffer as `&str`
(`chat/composer.rs:42-45`).

This milestone adds three things. (1) `TextField` (`crates/htui/src/ui/text_field.rs`): one
single-line field with a char cursor, insertion, deletion, a render-time window and an optional
mask — the widget PRD D1 owes MOD-22/MOD-23 and milestone 6's DSN entry; its mask is built here
and has **no consumer until milestone 6**. (2) `SettingsSection::captures_input` with a `false`
default and the gate in `SettingsTab::on_key` before the `h`/`l`/`[`/`]` cycle match
(`settings/mod.rs:197-208`; ANA-10 `:1245-1258`). (3) `HierarchySection`
(`settings/hierarchy.rs`), registered after `agents`, that lists the active workspace, its
projects, their repos (primary flagged) and this box's paths, creates and edits all of them through
the seam's CAS methods, sets paths through a symlink guard (canonicalise or refuse, PRD D11) and
deletes a workspace or project behind two confirmations, the second typed (PRD D13), with the
mirror rebuilt by the worker on the delete path (M1 D5) so the stale-mirror consequence of PRD
D13 does not exist before milestone 6.

Every request is served off the UI task: the section holds no store handle, the worker fills in
`created_by` and `box_id` from backend identity (`backend.rs:172-181`, `:254-260`), and every write
answers with a fresh snapshot or a `Failed` the shell already puts on the status line
(`update.rs:132-134`). No `WriteStore` method is added, so the six implementors and the conformance
count (36) are untouched.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The widget is `TextField` at `crates/htui/src/ui/text_field.rs` (`pub mod text_field;` in `ui/mod.rs`, `pub use text_field::{TextField, FieldOutcome}`).** State: `text: String`, `cursor: usize` (char index, `0..=chars`), `masked: bool`. API, the durable part: `TextField::new()`, `TextField::masked()`, `TextField::with_text(&str)` (cursor at end); `on_key(&mut self, KeyEvent) -> FieldOutcome` where `enum FieldOutcome { Consumed, Submit, Cancel, Pass }`; `text(&self) -> Option<&str>` (**`None` when masked**); `take(&mut self) -> String` (moves the buffer out and resets, the only read of a masked field); `clear()`, `len()`, `is_empty()`, `is_masked()`; `line(&self, width: u16, focused: bool, theme: &Theme) -> Line<'static>` for the caller to place. Keys: `Char(c)` with neither `CONTROL` nor `ALT` and `!c.is_control()` inserts at the cursor; a control char is swallowed silently (`Consumed`, nothing inserted); `Backspace`/`Delete`/`Left`/`Right`/`Home`/`End` edit and move (`Consumed`); `Enter` → `Submit`; `Esc` → `Cancel`; **everything else → `Pass`** (`Tab`, `BackTab`, `Up`, `Down`, `F(n)`, any modifier chord), so a form can move focus and a section can keep its own keys. Rendering: a window of the last `width - 1` chars ending at the cursor, with a leading `…` when clipped, computed at render time from the `Rect` and never cached; the cell under the cursor (or a trailing space) carries `theme.selected` when `focused`; masked renders one `•` per char plus ` (n)`. `Debug` is **hand-written** and prints `TextField { masked, len, cursor }` — never the text — for both modes. Width is counted in `char`s, not columns (no `unicode-width` is declared, `Cargo.toml` grep; wide chars misalign the cursor by design here — open item, not a bug in slug/path/DSN input). **Not built**: multi-line, history, bracketed paste, a reveal toggle, validation, `Zeroizing`. Unit tests in-module (`#[cfg(test)] mod tests`, as `composer.rs:104`). | PRD D1 (`:266-269`): "a shared widget with an optional mask", named in MOD-22/MOD-23 and consumed by milestone 6's DSN entry; the API is what those items depend on, the internals are not. Key table and control-char rule from ANA-10 `:1336-1345` (bracketed paste is not enabled: `terminal.rs`'s bare `ratatui::init()`, `App::on_terminal_event` matches only `Key`/`Resize`), the window and `…` from `:1330-1334` (`Composer::render` clips and loses the cursor), `•` + count and no reveal from `:1317-1321`. `text()` returning `None` when masked is what ANA-10 `:1260-1261` rejects the composer for offering. `Debug` by hand because `StoreRequest`, `RequestEnvelope` and every section derive `Debug` and `missing_debug_implementations = "warn"` (root `Cargo.toml:90-110`) means a field *will* be printed by something. `zeroize` is present only transitively (`Cargo.lock`, one entry) and ANA-10 M2b (`:2510`) assigns its promotion to the DSN milestone; swapping the buffer for `Zeroizing<String>` behind `take()` is additive and is milestone 6's, stated so nobody builds a second field. **Name**: PRD D1 (maintainer, 2026-09-16) supersedes ANA-10 M0's `MaskedField` (`:1265`) — one type with a mask flag, not a masked type; HANDOFF `:314-316` still reads "`MaskedField` names" and T4 rewrites that line so milestone 6 looks for `TextField::masked()`. |
| D2 | **`SettingsSection` gains `fn captures_input(&self) -> bool { false }` (the trait's first default body, `settings/mod.rs:42-58`), and `SettingsTab::on_key` (`:197-213`) checks the active section's `captures_input()` before the cycle match: when `true`, the key goes straight to `section.on_key`.** `AgentsSection` and the test-only `ProbeSection` (`tests/settings.rs:867-893`) keep the default. `HierarchySection::captures_input` is `!matches!(self.mode, Mode::Browse)`. A capturing section is still under the shell's chain (`state.rs:330-422`): a `Consumed` from it swallows `q`, `?`, `Tab` and the digits, which is the wanted behaviour while typing, and its `Pass` on `Tab`/digits in *Browse* keeps the globals alive. | ANA-10 `:1240-1258` names the exact five-line delegation-order problem and this fix; M0 (`:2411`) fixes the method name; HANDOFF `:314-316` forbids an ad-hoc focus mechanism instead of it. A default body rather than six-implementor churn is why it is on the section trait and not on `Tab` (`registry.rs:33-48`): `ChatTab` already does the equivalent inline (`chat/mod.rs:403-412`) and moving it is not this milestone's. |
| D3 | **`chat/composer.rs` is not refactored onto `TextField`.** Its six references are all in `chat/mod.rs` (`:38,43,132,166,601,607`) plus the inline mode check (`:355`, `:403-412`), it has its own unit tests (`composer.rs:104+`), and its contract — `Enter` submits, `Esc` leaves, no cursor, `text()` public — is pinned by the chat suites. Recorded as a CLEAN candidate to mint at MOD-15 close-out, not now. | ANA-10 `:1260-1266` rejects widening the composer on four grounds; the reverse (rebuilding it on the new widget) has no user-visible outcome in milestone 3's row and would put `chat/` into this milestone's file set and its snapshot suite into its gate. |
| D4 | **`HierarchySection` lives at `crates/htui/src/ui/tabs/settings/hierarchy.rs`, `SectionId("hierarchy")`, title `Hierarchy`, exported beside `AgentsSection` (`settings/mod.rs:11,26`), and registers second: `app/mod.rs:47-49` becomes `with_sections(vec![Box::new(AgentsSection::new()), Box::new(HierarchySection::new())])`.** The registry-empty placeholder at `settings/mod.rs:235` is unchanged (unreachable in the product once any section registers). The switcher's empty-state copy "no workspaces — creating one arrives with MOD-15" (`workspace_switcher.rs:115`) becomes "no workspaces — `N` in Settings > Hierarchy creates one" and `src/snapshots/htui__testkit__tests__shell_empty.snap:19` moves with it. | Strip order is registration order (`settings/mod.rs:91`); the maintainer's routing assumption is "hierarchy after `agents`" and it is accepted here. **PRD open question 2 (`:363-364`) stays open**: whether `agents` keeps first place once five sections exist is not decided by this milestone. ANA-10 §9.4 (`:2526`) gives MOD-15 the switcher's empty-state copy; the moment creation exists the copy is false. PRD risk `:378` (strip overflow at 100 columns) gets its pin now — a test in `tests/settings.rs` asserts the joined strip titles fit `SECTION_WIDE` — while two titles cost 19 columns. |
| D5 | **One read, one snapshot, worker-assembled.** `StoreRequest::Hierarchy(WorkspaceId)` → `StoreReply::Hierarchy(Option<Box<HierarchySnapshot>>)`, `None` when `workspace(id)` is `None` (the nil startup scope, `update.rs:251-253`, or a workspace deleted elsewhere). `HierarchySnapshot { workspace: Workspace, this_box: Option<BoxId>, root_path: Option<WorkspaceBoxPath>, projects: Vec<ProjectEntry> }`, `ProjectEntry { link: WorkspaceProject, project: Project, repos: Vec<RepoEntry> }`, `RepoEntry { repo: Repo, local_path: Option<RepoBoxPath> }`; projects ordered by `link.position`, repos by name; paths filtered to `this_box`. Assembled in a new `crates/htui/src/hierarchy.rs` (`pub mod hierarchy;` in `lib.rs:12-23`) by `async fn snapshot<S: ReadStore + WriteStore + ?Sized>(store: &S, ws, this_box) -> Result<Option<HierarchySnapshot>>` from `workspace` (`traits.rs:334`), `workspace_projects` (`:361`), `project` (`:126`, `ReadStore`), `repos` (`:429`), `repo_box_paths` (`:442`), `workspace_box_paths` (`:375`). `HierarchySnapshot::summary(&self) -> WorkspaceSummary` builds the switcher row (`hierarchy.rs:224-233`) so the section can emit `Action::SetScope`. `wants_requests(scope)` is `vec![Hierarchy(scope.workspace_id)]`; `on_scope_change` drops the snapshot and resets to `Mode::Browse` (an editor open across a workspace switch is dropped, deliberately: its `expected` token belongs to the other workspace). | `R-NF-3`: the section names a read and is handed rows (`settings/mod.rs:40-41`). One variant per read keeps the staleness index (`state.rs:158, 290-294`, keyed by request discriminant) doing its ordinary work: a scope change re-issues `Hierarchy` and the older reply is dropped. The N+1 reads are per keystroke-free event (activation, scope change, after a write) on a workspace-sized tree; a joined reader would be a new seam method costing six implementors (HANDOFF `:330-333`) for a read nothing else needs. The module is new rather than more of `store_worker.rs` (1819 lines) so the arms there stay one line each, as the reads at `:552-565` are. |
| D6 | **Writes are twelve `StoreRequest` variants carrying user-typed fields only; the worker fills identity.** `CreateWorkspace { slug, name, description }`, `UpdateWorkspace { id, expected: DateTime<Utc>, patch: WorkspacePatch }`, `SetWorkspaceRoot { id, path: String }`, `CreateProject { workspace: WorkspaceId, slug, name, description }` (`create_project` then `upsert_workspace_project` at `position = links.len()`), `UpdateProject { id, expected, patch: ProjectPatch }`, `CreateRepo { project, name, remote_url: Option<String>, default_branch, is_primary }`, `UpdateRepo { id, expected, patch: RepoPatch }`, `SetRepoPath { repo, path: String }`, `DeleteReach(DeleteTarget)`, `DeleteWorkspace(WorkspaceId)`, `DeleteProject(ProjectId)`, plus `Hierarchy` (D5): `StoreRequest` 26 → 38, `name()` (`store_worker.rs:230-259`) gains twelve snake-case arms (`"hierarchy"`, `"create_workspace"`, …, `"delete_project"`). All are served inside `try_serve` (`:550-599`) through **one arm whose pattern is the twelve variants or-ed together** — `StoreRequest::Hierarchy(..) | StoreRequest::CreateWorkspace { .. } | … | StoreRequest::DeleteProject(..) => hierarchy::serve(backend, request).await?` — so a `StoreError::Unreachable` still drops an `Online` backend onto the mirror at `:755-765`. **Not a guarded arm**: `try_serve`'s `match request` has no wildcard (fact-check F-12, verified by compile probe — "match arms with guards don't count towards exhaustivity", E0004 on rustc 1.98.1), so `_ if request.is_hierarchy() =>` would not compile and `is_hierarchy()` is not written. `hierarchy::serve` takes `backend.writer()` (`backend.rs:150-156`; `Writer: ReadStore + WriteStore`, `writer.rs:657, 752`) and answers `Err(StoreError::Unreachable(DATABASE_UNREACHABLE))` when it is `None` (`Offline`), `backend.this_user().await?` for `created_by` (`backend.rs:172-181`) and `backend.box_info().await?.map(|b| b.box_id)` for `box_id` (`:254-260`; `MemStore::box_info` `mem.rs:221-233`). A path write on a box with no row is refused `NotFound { entity: "box", id: "(this box)" }`, the shape of `backend.rs:174-177`. **Replies**: every create/update/path write re-assembles and answers `StoreReply::Hierarchy(Some(snapshot))`; a `CasOutcome::Stale(_)` answers `StoreReply::HierarchyStale(Box<HierarchySnapshot>)`; `DeleteReach` answers `StoreReply::DeleteReach(Option<DeleteReach>)`; the two deletes answer `StoreReply::Deleted { target: DeleteTarget, reach: DeleteReach, mirror: MirrorAfterDelete }` with `enum MirrorAfterDelete { Rebuilt, NoMirror, NotNeeded, Failed(String) }` (D10). `StoreReply` 4 new variants. Errors go through `failed()` (`:603-608`) unchanged. The section refuses a second write while one is in flight (`busy: Option<&'static str>`, the `probing` pattern at `agents.rs:256-259`) because the staleness index keeps only the newest of a kind. Not in scope, named: `remove_workspace_project` (unlinking without deleting) has no row in milestone 3's outcome and no key. | No view holds a `UserId` or `BoxId` (grep of `app/*.rs`, `agents.rs`, `store_worker.rs`); `Backend::this_user`'s own doc says the render side never learns one (`backend.rs:158-161`). `writer()` rather than `writable()` because `writable()` is `None` on `Backend::Memory` (`:118-128`) and the harness and `--demo` run on `MemStore`; `Writer::Memory` is a `WriteStore` (`:130-135`). Snapshot-as-reply keeps one render path: a section that patched its rows locally from `Applied(row)` would be a second source of truth. `Stale` carries the live row (M1 D3) but the section needs the whole tree to re-render, so the worker re-reads — one round trip, not two. |
| D7 | **CAS miss UX (PRD D8): reload, keep the typed text, retry by hand.** The editor holds `expected` from the row it opened on. On `HierarchyStale(snapshot)`: rows replaced, editor stays open with its fields untouched, `expected` becomes the current row's `updated_at` (found by id; if the id is gone the editor closes with notice "deleted elsewhere while you were editing"), `notice = "changed elsewhere since you opened it — reloaded; Enter retries against the current row"` in `theme.error`. No automatic retry. | PRD D8 `:305-309`: "the timestamp decides *whether* the write applies, it never decides *who wins*"; an automatic retry would be last-writer-wins with one extra round trip (`R-ENT-10`, PRD `:233`). Keeping the text is why the reload exists: retyping is the cost the PRD said not to pay. |
| D8 | **The symlink guard (PRD D11) is `htui_core::root_path::canonical_root(path: &Path) -> Result<PathBuf, RootRefusal>`, new file `crates/htui-core/src/root_path.rs` (`pub mod root_path;` in `lib.rs`), `std::fs` only, sync.** Rule, in order: not `is_absolute()` → `RootRefusal::Relative`; `std::fs::symlink_metadata` fails → `Missing`; `is_symlink()` and `std::fs::canonicalize` fails → `Dangling`; the metadata of the result is not a directory → `NotADirectory`; otherwise the **canonical** path (`canonicalize` is applied to plain directories too, so one directory has one stored string). `Display` texts, each with the path the user typed and never the link target: "`{p}` is not an absolute path", "`{p}` does not exist on this box", "`{p}` is a link to nothing", "`{p}` is not a directory". The worker calls it inside `tokio::task::spawn_blocking` in the `SetWorkspaceRoot`/`SetRepoPath` arms, stores `canonical.to_string_lossy()` as `root_path`/`local_path`, and the section's notice says "stored as `{canonical}`" when it differs from what was typed. Errors reach the status line as ``set_repo_path: `/x` is a link to nothing`` through `Failed` mapped as `StoreError::Constraint(refusal.to_string())`. Windows `\\?\` prefixes from `canonicalize` are MOD-16's (PRD `:239-240`). Tests: `tempfile` (`crates/htui/Cargo.toml:59` dev-dep; `htui-core` gains it as a dev-dep) with a real symlink on Unix (`#[cfg(unix)]`). | PRD D11 `:322-325`: `FsRepoReader` refuses a link *below* the root and deliberately leaves the root itself unchecked (`htui-agent/src/excerpt.rs:398-401`, "`/tmp` is a symlink on more than one box — refusing it would refuse a legitimate checkout"), which is exactly why the milestone row says "canonicalised **or** refused" rather than refused: a root that is a link to a real directory is stored as the directory. It lives in `htui-core` because MOD-7 is the named second caller (PRD D11, `:379`) and `htui-core` is the one crate every other depends on and already does `std::fs` in `prompt/excerpt.rs`; `htui-core`'s tokio is `macros, rt` only (`crates/htui-core/Cargo.toml:27`), so the guard is sync and the worker wraps it (the worker has no `spawn_blocking` today, grep; ANA-10 M2b `:2510` plans the same wrapper for the keyring). Message style and "never name the target" from `excerpt.rs:419-430`. |
| D9 | **Delete (PRD D13): `d` on a workspace or project row; stage 1 counts, stage 2 is typed.** `d` → `ctx.request(DeleteReach(target))`, `mode = Deleting { target, stage: Counting }`; `DeleteReach(Some(reach))` → `stage: Warn`; `None` → notice "already gone", re-read. **Warn pane**, every line `theme.error`: project — "This deletes project `{slug}` and its entire history. Gone for good: {n} items, {n} runs, {n} run steps, {n} session events, {n} run step commits, {n} notes, {n} revisions, {n} links, {n} documents, {n} kinds, {n} graphs, {n} phases, {n} phase agents, {n} templates, {n} repos, {n} repo paths, {n} skill bindings, {n} key counters." (zero counts omitted, order = `DeleteReach` field order `traits.rs:744-792`), then "Nothing here can be undone. `y` to continue, `n` or `Esc` to stop." Workspace — "This deletes workspace `{slug}`: {n} project links and {n} box root paths. Its projects survive and stay reachable from other workspaces." (`0001_init.sql:162,174`; M1 D4). `y` → `stage: Typed { field: TextField::new() }` with the prompt "Type `{slug}` to confirm:"; `Enter` with the exact slug → `ctx.request(Delete{Workspace,Project}(id))`, `stage: InFlight`; any other text → notice "that is not the slug; nothing was deleted", field cleared, stage kept; `Esc`/`n` at any stage → `Browse`. While `Deleting`, `captures_input` is `true` and every key not listed is `Consumed` (the modality of `answer_consent`, `agents.rs:768-801`). **After `Deleted`**: notice "deleted `{slug}`: {total} rows across {tables} tables; mirror rebuilt / no mirror / mirror not rebuilt: {err}"; project → the fresh `Hierarchy` read arrives and D11 emits `SetScope`; workspace → `ctx.request(StoreRequest::Workspaces)` and on `StoreReply::Workspaces(list)` the first remaining summary is emitted as `Action::SetScope`, an empty list leaves the shell on the dead scope with the section reading "no workspace — `N` creates one" (the `Hierarchy(None)` render of D5). The workspace slug is what is typed for a workspace; PRD D13 wrote "the project's slug" for the project case only. | PRD D13 `:329-355` and risk `:374`: two confirmations, the second typed, counts before the act, copy that says "history". `DeleteReach` carries exactly the twenty-one counts (`traits.rs:744-792`) and `delete_*` returns what it took, so the numbers shown are the numbers removed (M1 D4). In-section modality rather than an overlay because a reply to a popped overlay is dropped silently (`update.rs:179-180`) and the section is what has to re-read afterwards. `SetScope` is "the only path that changes the scope" (`update.rs:89-104`) and is how the Backlog stops listing the deleted project. |
| D10 | **Before milestone 6 there is no Rebuild button, and the delete path does not need one: the worker calls `CacheStore::rebuild()` (`cache/mod.rs:176-195`) itself, inside the `DeleteProject` arm, after `writer.delete_project(id)` returns.** `backend.cache()` (`backend.rs:226-231`) → `Some` → `rebuild().await` → `MirrorAfterDelete::Rebuilt`, or `Failed(err.to_string())` **without** turning the reply into a `Failed` (the delete has already happened and must be reported as done); `None` (`Memory`) → `NoMirror`. `DeleteWorkspace` never rebuilds: `MirrorAfterDelete::NotNeeded`. A rebuild drops every mirrored table and every cursor and clears `last_full_refresh_at`; the refresher's next pass therefore starts from cursor zero (`cache/refresh.rs:228`: set "only when every cursor was 0 at the top of the pass") and refills from a Postgres that no longer has the project. Milestone 6 adds PRD D10's user-facing button and copy around the same method; nothing in this arm is undone then. | M1 D5 settles that the rebuild is the store worker's, "milestone 3's `StoreRequest` handler", and this plan does not re-decide it. PRD D13 bullet 3 (`:347-352`): `project`, `item`, `repo`, `item_kind`, `document` are cursor-refreshed with no delete propagation, so without the rebuild the Backlog would serve a deleted project from the mirror once offline; `workspace` and `workspace_project` are full-table replaced (`:349-350`), so a workspace delete needs none. HANDOFF `:317-321`: the rebuild "must not be 'helpfully' extended" — this arm calls the method and nothing else. |
| D11 | **Scope follows the tree.** On every `Hierarchy(Some(snapshot))` whose project id list (by position) differs from `ctx.scope.project_ids`, or whose `workspace.id` differs from `ctx.scope.workspace_id` (a just-created workspace), the section emits `Action::SetScope { workspace: snapshot.summary() }`. `set_scope` re-runs `on_scope_change` on every tab, clears overlays and re-issues the active tab's reads (`update.rs:89-104`), which re-reads `Hierarchy` once more; the second snapshot matches the scope and the loop ends. `CreateWorkspace`'s reply is the new workspace's snapshot, so `N` creates *and enters* the workspace. | `App.projects` and `Scope` are only ever set by `set_scope` (`update.rs:89-104`; `state.rs`), and `on_app_reply` picks a scope only while none is set (`:262-279`). Emitting the existing action keeps the shell ignorant of the section (blueprint D5 rule at `update.rs:1-4`). |
| D12 | **Primary flag invariant: at most one primary per project (`uq_repo_primary`, `0001_init.sql:196`); `p` moves it, nothing unsets it.** `p` on a repo row → `UpdateRepo { patch: RepoPatch { is_primary: Some(true), ..Default } }`; M1 D10 clears the previous primary in the same transaction. The repo editor's `is_primary` is a `y`/`n` toggle field pre-filled `y` for a project's first repo and `n` otherwise, and `RepoPatch.is_primary` is `None` on edit (the editor never carries the flag; `p` is its only writer). Rendered as `*` before the primary's name. | PRD milestone row "repos with the primary flag"; one writer per column is M1 D8/D10's rule for `token_budget` applied to `is_primary`. A project with repos and no primary is reachable by the schema but wanted by nothing, so no key produces it. |
| D13 | **Section shape.** `struct HierarchySection { snapshot: Option<HierarchySnapshot>, unavailable: Option<String>, cursor: usize, mode: Mode, busy: Option<&'static str>, notice: Option<String> }`; `enum Mode { Browse, Editing(Editor), Deleting { target, slug, stage } }`; `struct Editor { kind: EditorKind, fields: Vec<Field>, focus: usize, expected: Option<DateTime<Utc>> }`, `Field { label: &'static str, input: TextField, required: bool }`, `enum EditorKind { NewWorkspace, EditWorkspace(WorkspaceId), NewProject, EditProject(ProjectId), NewRepo(ProjectId), EditRepo(RepoId), WorkspaceRoot(WorkspaceId), RepoPath(RepoId) }`. Rows are a flat `Vec<Row>` rebuilt from the snapshot: the workspace line (`{name} ({slug}) — root on this box: {path|unset}`), one line per project (`  {slug}  {name}`), one per repo (`    {*|' '}{name}  {default_branch}  {remote_url|—}  {local_path|unset}`). **Browse keys**: `j`/`k` cursor (no wrap, `agents.rs:290-304`), `N` new workspace, `n` new project (cursor on the workspace row) or new repo (cursor on a project or repo row), `e` edit the row, `p` primary (repo row), `b` this box's path (workspace or repo row), `d` delete (workspace or project row), `r` re-read, `Esc` clears the notice; everything else `Pass`. **Editor keys**: the focused `TextField` answers first; `Pass` on `Tab`/`Down` → next field, `BackTab`/`Up` → previous; `Submit` → required fields non-empty or notice "`{label}` is required", then one request per `EditorKind` with `busy` set; `Cancel` → `Browse`. Text fields carry `description`/`remote_url` as optional (empty → `""` / `None`). Slug format is not validated here — `workspace.slug`/`project.slug` are `TEXT NOT NULL UNIQUE` with no `CHECK` (`0001_init.sql:131,145`), so a duplicate is the store's `Constraint` on the status line and the editor stays open. Layout: `[rows Min(3)] [pane Length(n)] [hint Length(1)]` as `agents.rs:1271-1294`; the pane holds the editor or the delete stage; the hint line lists the live keys for the mode. `unavailable` renders "hierarchy needs Postgres" (`agents.rs:1281` wording family) on a `Failed { request: "hierarchy" }`. | Mirrors `AgentsSection` (`agents.rs:249-271`, `:1117-1207`): cursor, modality, `busy`, notice, and the free-letter rule at `:1128-1130` (`q`, `?`, digits, `Tab` global; `h`/`l`/`[`/`]`/arrows the tab's). `N`/`n` split because the workspace row and a project row both want "new child" and "new sibling" and one letter cannot say which. `b` for the path because `p` is primary and `l`/`r` are taken. |
| D14 | **Tests.** Widget: unit tests in `text_field.rs` (insert at cursor, backspace/delete at both ends, Home/End, control char dropped, `Enter`/`Esc`/`Tab` outcomes, window `…` at `width` and cursor cell style via a `Buffer`, mask `•` and count, `text()` is `None` when masked, `take()` empties, `Debug` never contains the text). Trait gate: `tests/settings.rs` gains a capturing `ProbeSection` twin asserting `l` reaches the section while capturing and cycles otherwise, plus the strip-width pin (D4). Worker: `tests/hierarchy.rs` (new) drives `serve(&Backend::memory(MemStore::demo()), &request)` (`store_worker.rs:537-542`) end to end — create workspace/project/repo, primary move, CAS miss via two updates from one `expected`, path guard through a `tempfile` dir and symlink, `DeleteReach` equals `Deleted.reach`, `Offline` refusal text is `DATABASE_UNREACHABLE` (`Backend::Offline` needs a `CacheStore`; `Harness::over_backend`, `testkit.rs:113`, and `chat_offline.rs` show the construction). Section: the same file, over a promoted `testkit::SectionBench` (D15) for key/reply units and `Harness::demo().with_tab(SettingsTab::with_sections(..))` (`tests/settings.rs:897-900`) for `insta` frames: `hierarchy_demo`, `hierarchy_editor_repo`, `hierarchy_delete_warn`, `hierarchy_delete_typed`, `hierarchy_stale`, `hierarchy_no_workspace`, `hierarchy_offline` under `tests/snapshots/hierarchy__*.snap`. Snapshot text carries no path from the test machine: paths in frames come from `MemStore::demo()` rows or a fixed `/srv/…` string the harness never stats. | Conventions: `Bench` (`tests/settings.rs:126-188`), `render_section`/`accented_lines` (`:91-115`), `settings_over` (`:192-199`), snapshot names `settings__agents_*.snap` (five exist). Worker tests through `serve` are how `try_serve`'s reads are already exercised without a runtime (`:530-536`). |
| D15 | **`Bench` moves to `crates/htui/src/testkit.rs` as `pub struct SectionBench` (same seven fields and five methods, `pub async fn new()`), `tests/settings.rs` imports it.** | Two sections' test files need it and a second 60-line copy is the drift the house style forbids; `testkit.rs` is already the `testkit`-feature home of `Harness` (`crates/htui/Cargo.toml` feature; `lib.rs:23`). Done in T1 so T3 finds it. |

## Patterns to Mirror

| Concern | Pattern | Where |
|---|---|---|
| Section skeleton, cursor, modality, `busy`, notice, hint row | `AgentsSection` struct and `on_key`/`on_reply`/`render` | `agents.rs:249-304`, `:768-801`, `:1117-1294` |
| Trait method with default body checked before the cycle match | ANA-10's five-line fix | `settings/mod.rs:197-213`; ANA-10 `:1245-1258` |
| Worker read arm, one line, `?` for `Unreachable` | `StoreRequest::Workspaces => StoreReply::Workspaces(backend.workspaces().await?)` | `store_worker.rs:552-565`, fallback `:755-765` |
| Identity resolved worker-side | `backend.this_user().await?` before a write | `agent_worker.rs:1178`; `backend.rs:158-181` |
| Reply-name matching on `Failed` | `StoreReply::Failed { request, .. } if *request == "agents"` | `agents.rs:1224-1266` |
| Scope change from a view | `Action::SetScope { workspace }` | `update.rs:89-104`; `workspace_switcher.rs` |
| Symlink refusal text and "never name the target" | `descend` | `htui-agent/src/excerpt.rs:383-430` |
| Section test bench, snapshots, strip cycling | `Bench`, `render_section`, `h_and_l_move_between_sections` | `tests/settings.rs:91-199`, `:895-919` |
| Widget unit tests in-module | `composer.rs` | `chat/composer.rs:104+` |
| Multi-phase HANDOFF paragraph | "Milestone N landed (…)" appended to the one checklist line | `HANDOFF.md:322`, `:350`; `.claude/rules/workflow-docs.md` lifecycle 4 |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui/src/ui/text_field.rs` | **new** | T1 | D1: `TextField`, `FieldOutcome`, hand-written `Debug`, unit tests |
| `crates/htui/src/ui/mod.rs` | edit | T1 | `pub mod text_field;` + re-export |
| `crates/htui/src/ui/tabs/settings/mod.rs` | edit | T1, T3 | T1: `captures_input` default + gate in `on_key` (`:197-213`); T3: `pub mod hierarchy;`, `pub use hierarchy::HierarchySection;` |
| `crates/htui/src/testkit.rs` | edit | T1 | D15: `SectionBench` |
| `crates/htui/tests/settings.rs` | edit | T1 | import `SectionBench`; capturing-probe test; strip-width pin |
| `crates/htui-core/src/root_path.rs` | **new** | T2 | D8: `canonical_root`, `RootRefusal`, tests |
| `crates/htui-core/src/lib.rs` | edit | T2 | `pub mod root_path;` |
| `crates/htui-core/Cargo.toml` | edit | T2 | `tempfile` dev-dep for the guard tests |
| `crates/htui/src/hierarchy.rs` | **new** | T2 | D5/D6/D10: snapshot types, `snapshot()`, `serve()`, `MirrorAfterDelete` |
| `crates/htui/src/lib.rs` | edit | T2 | `pub mod hierarchy;` |
| `crates/htui/src/store_worker.rs` | edit | T2 | D6: 12 request variants + `name()` arms; 4 reply variants; one `try_serve` arm of twelve or-ed patterns |
| `crates/htui/tests/hierarchy.rs` | **new** | T2 (worker half), T3 (section half) | D14 |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | **new** | T3 | D9/D11/D12/D13: the section |
| `crates/htui/src/app/mod.rs` | edit | T3 | D4: register after `agents` (`:47-49`) |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | edit | T3 | D4: empty-state copy (`:115`) |
| `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap` | edit | T3 | follows the copy (`:19`) |
| `crates/htui/tests/snapshots/hierarchy__*.snap` | **new ×7** | T3 | D14 |
| `HANDOFF.md` | edit | T4 | MOD-15 entry: "Milestone 3 landed" paragraph; `:314-316` `MaskedField` sentence rewritten to `TextField::masked()` (D1) |
| `.claude/prds/mod-15-hierarchy-management.prd.md` | edit | T4 | milestone table `:251`: row 3 → complete, plan link |

Not changed: `crates/htui-core/src/store/**` (no seam change; `CASES` stays 36), `crates/htui-store/**` (no query, no `.sqlx`, no migration), `crates/htui-agent/**`, `chat/composer.rs` (D3), `app/action.rs` (`TabAction::FocusSection` is milestone 6's), `keymap.rs`.

## Tasks

**T1 ∥ T2 → T3 → T4.** File sets: T1 = {`ui/text_field.rs`, `ui/mod.rs`, `settings/mod.rs`, `testkit.rs`, `tests/settings.rs`}; T2 = {`htui-core/src/root_path.rs`, `htui-core/src/lib.rs`, `htui-core/Cargo.toml`, `htui/src/hierarchy.rs`, `htui/src/lib.rs`, `store_worker.rs`, `tests/hierarchy.rs`}; T1 ∩ T2 = ∅ and neither compiles against the other, so they run in parallel. T3 = {`settings/hierarchy.rs`, `settings/mod.rs`, `app/mod.rs`, `workspace_switcher.rs`, its snapshot, `tests/hierarchy.rs`, seven new snapshots} intersects T1 on `settings/mod.rs` and T2 on `tests/hierarchy.rs`, and needs both to compile: serial after both. T4 is docs after T3. `cargo test --workspace` stays green between tasks: T1 adds a defaulted method, T2 adds variants no view matches exhaustively (`agents.rs:1267` has `_ => {}`; `update.rs:223` likewise — the implementer checks `match reply`/`match request` sites for exhaustiveness at compile time).

**Build caveat on the parallel pair (fact-check F-29).** T1 and T2 are file-disjoint but both add
modules to the **same crate** (`htui`), on one working tree with one `target/`: their `cargo`
invocations contend on the build lock, and while either is mid-write the other's `cargo test -p
htui` sees a crate that does not compile. So: the two implementers may run concurrently, but
**their gates are serialized** — T1 gates first, then T2 re-runs the full gate on the merged tree
(no gate result from a half-written tree is accepted), or the pair runs in separate worktrees and
the gate runs once on the merge. Each implementer commits its own work incrementally; uncommitted
subagent work does not survive the session. Before blaming a Postgres failure in any gate, check
`df -h /` — `target/` fills the disk on this box.

TDD per task: tests first, red, then code. Every implementer prompt carries: PRD D1/D8/D11/D13 and M1 D3/D4/D5/D10 win over this plan where they disagree; graphify-first for codebase questions; no `WriteStore` change; nothing sets `updated_at` by hand; no new migration, no new `query!`; a section holds no store handle and no `UserId`/`BoxId`; `Debug` never prints a field's text; `unsafe_code = "forbid"`, MSRV 1.98.

### Task 1: `TextField`, `captures_input`, `SectionBench` (parallel with T2)
- **Files**: `crates/htui/src/ui/text_field.rs` (new), `crates/htui/src/ui/mod.rs`,
  `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/testkit.rs`,
  `crates/htui/tests/settings.rs`.
- **Action**: D1 widget with its in-module tests; D2 default method and the gate at
  `settings/mod.rs:197-208` (the `captures_input()` check is the first statement of `on_key`);
  D15 promotion of `Bench`; the two new tests in `tests/settings.rs` (capturing probe; strip width
  ≤ `SECTION_WIDE`).
- **Gate**: `cargo test -p htui --all-features` (five `settings__agents_*` snapshots unchanged),
  `cargo clippy -p htui --all-targets --all-features -- -D warnings`, `cargo doc -p htui --no-deps`.

### Task 2: guard, snapshot, worker arms (parallel with T1)
- **Files**: `crates/htui-core/src/root_path.rs` (new), `crates/htui-core/src/lib.rs`,
  `crates/htui-core/Cargo.toml`, `crates/htui/src/hierarchy.rs` (new), `crates/htui/src/lib.rs`,
  `crates/htui/src/store_worker.rs`, `crates/htui/tests/hierarchy.rs` (new, worker half only).
- **Action**: D8 guard and its `tempfile` tests (symlink case `#[cfg(unix)]`); D5 snapshot and
  `summary()`; D6 twelve requests, four replies, `name()` arms, `is_hierarchy()`, one `try_serve`
  arm delegating to `hierarchy::serve` (twelve or-ed patterns, **no guard** — F-12); D10 rebuild in the `DeleteProject` arm; worker tests
  through `serve` over `Backend::memory(MemStore::demo())` and one `Backend::Offline` refusal.
  `StoreRequest::name()`'s test (if one pins the arm count) moves with it.
- **Gate**: `cargo test -p htui-core`, `cargo test -p htui --all-features`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

### Task 3: the section (after T1 and T2)
- **Files**: `crates/htui/src/ui/tabs/settings/hierarchy.rs` (new),
  `crates/htui/src/ui/tabs/settings/mod.rs`, `crates/htui/src/app/mod.rs`,
  `crates/htui/src/ui/overlay/workspace_switcher.rs`,
  `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap`,
  `crates/htui/tests/hierarchy.rs` (section half), `crates/htui/tests/snapshots/hierarchy__*.snap`.
- **Action**: D4 registration and copy; D7 stale handling; D9 delete flow and copy; D11 scope
  follow; D12 primary; D13 modes, keys, layout, hint; D14 section tests and seven snapshots, read
  before acceptance.
- **Gate**: `cargo test -p htui --all-features` with the seven new snapshots accepted and
  `shell_empty` updated, nothing else in `snapshots/` changed; `cargo run -p htui -- --demo` smoke:
  `Settings` → `l` → create a repo, set its path to a real directory, move primary, delete a
  project through both confirmations, `q` still quits from Browse.

### Task 4: docs (after T3)
- **Files**: `HANDOFF.md`, `.claude/prds/mod-15-hierarchy-management.prd.md`.
- **Action**: milestone row 3 → complete with this plan linked; HANDOFF's MOD-15 entry gains
  "Milestone 3 landed (`<first>`..`<last>`, date)" with the test count, the twelve request names,
  D10's "delete rebuilds the mirror from the worker" and D1's rename (`:314-316` now names
  `TextField::masked()` as the field milestone 6 uses). No CLEAN item is minted here (D3's
  candidate is noted in the paragraph for close-out).
- **Gate**: `cargo doc --workspace --no-deps`; `git diff --stat` touches only the two files.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                       # Postgres tests skip without the env var
HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features
(cd crates/htui-store && cargo sqlx prepare --check -- --all-targets --all-features)   # no query added; must still pass
cargo doc --workspace --no-deps
cargo run -p htui -- --demo                                  # T3 smoke script above
```
Snapshots: `insta` writes `tests/snapshots/*.snap.new` on first run; read each, then rename to
`.snap` (or `cargo insta accept` where installed) and commit. Pins that must **not** move:
`CASES.len() == 36` (`crates/htui-core/tests/mem_store.rs:35-41`), `EXPECTED_CASES == 36`
(`crates/htui-store/tests/pg_conformance.rs:19`), the five `settings__agents_*.snap`. Pins that
**must** move: `shell_empty.snap:19` (D4). `.sqlx/` is unchanged; a diff there means a query was
added against this plan. Check `df -h /` before blaming Postgres for a crash loop.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| A capturing section swallows `q`/`?`/digits forever after a lost key | Low | Medium | `captures_input` is derived from `mode`, never a flag; `Esc` returns to `Browse` from every mode; test asserts `q` reaches the global keymap in Browse |
| A field's text lands in a log through `{:?}` of a section or envelope | Medium if derived | High for the masked case in M6 | D1's hand-written `Debug`; unit test asserts `format!("{:?}")` of a field holding `"secret"` does not contain it |
| Refresher pass in flight while `rebuild()` runs | Medium | Medium (a row re-inserted from a pre-delete read) | The rows would come from Postgres after the delete committed, so the worst case is an extra pass; `last_full_refresh_at` cleared forces a full pass next (`refresh.rs:228`). Open item V-open-2 |
| Second write of the same kind in flight is silently dropped by the staleness index | Medium | Medium | `busy` refuses a second request until the reply, as `probing` does; notice "`{name}` is still in flight" |
| `canonicalize` on Windows yields `\\?\C:\…` and MOD-7 compares strings | Medium | Low now | Stored as returned; MOD-16 owns Windows path facts (PRD `:239-240`); noted in the guard's doc |
| Deleting the last workspace strands the shell on a dead scope | Certain when done | Low | D9: section renders "no workspace — `N` creates one" and `N` enters the new one (D11); the switcher's copy names the key |
| Strip overflow once five sections register | Low now | Low | D4's width pin fails the moment titles exceed 100 columns (PRD `:378`) |
| `HierarchySnapshot` N+1 reads on a large workspace feel slow on a remote Postgres | Low | Low | Reads are per event, not per key; a joined reader is one seam method away if it ever matters |
| Editor dropped by a scope change mid-typing | Low | Low | Deliberate (D5): the CAS token belongs to the other workspace; the notice says so |

## Verified claims (fact-check, 2026-09-16)

Every row read out of this tree during `/handoff-run` step 3.5, before the CONFIRM gate.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F-1 | `SettingsTab::on_key` matches the `l`/`]`/`Right` and `h`/`[`/`Left` cycle **before** delegating to the active section | **true** | `settings/mod.rs:197-213`, cycle arms `:199-206`, delegation `:209-212` — D2's insertion point is exact |
| F-2 | `SettingsSection` is a trait with no default bodies today (`id`, `title`, `wants_requests`, `on_key`, `on_reply`, render) | **true** | `settings/mod.rs:42-55` |
| F-3 | A defaulted `fn captures_input(&self) -> bool { false }` keeps the trait object-safe behind `Box<dyn SettingsSection>` | **true** | compile probe, rustc 1.98.1: `trait S { fn id(&self)->u8; fn captures(&self)->bool{false} }` used through `&dyn S` compiles |
| F-4 | `captures_input`, `MaskedField`, `FocusSection`, `TextField` have **zero** occurrences in `crates/` | **true** | `grep -rn` over `crates/` returns nothing (matches HANDOFF `:314-315`) |
| F-5 | Registration order is strip order; `AgentsSection` is the only section registered | **true** | `settings/mod.rs:90-92` (`register`'s doc "Registration order is strip order and cycling order"), `app/mod.rs:47-48` |
| F-6 | `Backend::writer()` answers `Some(Writer::Memory(..))` on `Memory` and `None` on `Offline`; `writable()` answers `None` on `Memory` | **true** | `htui-store/src/backend.rs:150-156`, `:123-128` |
| F-7 | `Writer` implements both `ReadStore` and `WriteStore` | **true** | `htui-store/src/writer.rs:657`, `:752` |
| F-8 | `this_user`, `box_info`, `cache()` exist on `Backend`; the render side never learns a `UserId` | **true** | `backend.rs:172`, `:254`, `:226`; doc at `:167-171`; grep of `crates/htui/src/ui/` + `app/` for `UserId`/`BoxId` is empty |
| F-9 | `MemStore::box_info` and `MemStore::demo()` exist | **true** | `htui-core/src/store/mem.rs:221`, `:131` |
| F-10 | The six reads D5 composes exist with those signatures | **true** | `traits.rs`: `project` `:126`, `workspace` `:334`, `workspace_projects` `:361`, `workspace_box_paths` `:375`, `repos` `:429`, `repo_box_paths` `:442` |
| F-11 | `DeleteReach` carries 21 counts; `delete_reach` and `DeleteTarget` exist | **true** | `traits.rs:745-792` (21 `pub` fields), `:594`, `:729` |
| F-12 | **`try_serve`'s `match request` has no wildcard arm, so the plan's guarded `_ if request.is_hierarchy()` arm would not compile** | **FALSE — plan amended** | `store_worker.rs:551-599`, `grep '^\s*_\s*(if)?=>'` empty; compile probe on rustc 1.98.1 gives `E0004 … match arms with guards don't count towards exhaustivity`. D6 now uses one arm of twelve or-ed patterns and drops `is_hierarchy()` |
| F-13 | Reply matches outside the worker all carry `_ => {}`, so four new `StoreReply` variants compile untouched | **true** | `app/update.rs:223`, `settings/agents.rs:1267`, `chat/mod.rs:524` |
| F-14 | Plan's bare `backend.rs` / `writer.rs` / `cache/*.rs` are in `htui-store`, not `htui` | **corrected** | `crates/htui-store/src/{backend.rs,writer.rs,cache/mod.rs,cache/refresh.rs}`; note added to the header block |
| F-15 | `store_worker.rs` is 1334 lines | **FALSE — plan amended** | `wc -l` = **1819**; D5 now says 1819. `StoreRequest` has 26 variants (`:57-229`), `StoreReply` 25 (`:264+`) |
| F-16 | `CacheStore::rebuild()` exists; `last_full_refresh_at` is set only when every cursor was 0 | **true** | `htui-store/src/cache/mod.rs:176`, `cache/refresh.rs:228` |
| F-17 | `uq_repo_primary` is a partial unique index over `repo(project_id) WHERE is_primary`; `workspace.slug`/`project.slug` are `TEXT NOT NULL UNIQUE` with no `CHECK` | **true** | `0001_init.sql:196`, `:131`, `:145` |
| F-18 | `workspace_project` and `workspace_box_path` cascade from `workspace` | **true** | `0001_init.sql:162`, `:174` |
| F-19 | `composer.rs` exposes `text() -> &str`, has in-module tests, and is referenced only from `chat/` | **true** | `composer.rs:29`, `:43`, `:104`; references all under `crates/htui/src/ui/tabs/chat/` |
| F-20 | ANA-10 names the delegation-order fix and the `MaskedField` path this plan renames | **true** | `docs/ANA-10.md:992`, `:1233`, `:1251` (the `captures_input` snippet), `:1261`, `:1265` |
| F-21 | `htui-core` has no `tempfile` dev-dep and its tokio is `macros, rt` only; `htui` already has `tempfile` | **true** | `htui-core/Cargo.toml:27` + dev-deps block (tokio, insta only); `htui/Cargo.toml:59` |
| F-22 | `unicode-width` is declared nowhere; `zeroize` is lockfile-only (transitive) | **true** | no match in any `Cargo.toml`; `Cargo.lock:5123` |
| F-23 | Workspace lints: `unsafe_code = "forbid"`, `missing_debug_implementations = "warn"`, MSRV 1.98 | **true** | root `Cargo.toml:91`, `:92`, `:7`; toolchain is rustc 1.98.1 |
| F-24 | `CASES` is pinned at 36 and `EXPECTED_CASES` at 36 | **true** | `htui-core/tests/mem_store.rs:36-41`, `htui-store/tests/pg_conformance.rs:19` |
| F-25 | `tests/settings.rs` holds `Bench`, `render_section`, `settings_over`, `ProbeSection`, `SECTION_WIDE = 100`, and five `settings__agents_*.snap` exist | **true** | `:126`, `:91`, `:192`, `:867`, `:46`; `tests/snapshots/settings__agents_{demo,empty,probed,quota,unknown_row}.snap` |
| F-26 | `Harness::demo()`, `Harness::over_backend()`, `Harness::with_tab()` exist | **true** | `crates/htui/src/testkit.rs:93`, `:113`, `:318` |
| F-27 | The switcher's empty-state copy and its snapshot line are where D4 says | **true** | `workspace_switcher.rs:115`; `src/snapshots/htui__testkit__tests__shell_empty.snap:19` |
| F-28 | `FsRepoReader` refuses a symlink **below** the root and deliberately leaves the root itself unchecked | **true** | `htui-agent/src/excerpt.rs:292-296`, `:383-397` (`/tmp` reasoning at `:397`) |
| F-29 | **Task independence**: T1 ∩ T2 = ∅ over the declared file sets | **true (with a build caveat)** | T1 = {`ui/text_field.rs`, `ui/mod.rs`, `settings/mod.rs`, `testkit.rs`, `tests/settings.rs`}; T2 = {`htui-core/src/root_path.rs`, `htui-core/src/lib.rs`, `htui-core/Cargo.toml`, `htui/src/hierarchy.rs`, `htui/src/lib.rs`, `store_worker.rs`, `tests/hierarchy.rs`} — no shared path, and neither references a symbol the other adds. Caveat below |

## Acceptance
- [ ] `TextField` exists with D1's API; `text()` is `None` when masked; `Debug` never prints the buffer; window, `…`, `•`+count and control-char rejection unit-tested
- [ ] `SettingsSection::captures_input` defaults `false`; `SettingsTab::on_key` checks it before the cycle match; a capturing section receives `l`
- [ ] `HierarchySection` registers after `agents`; strip width pinned; PRD open question 2 left open in the PRD
- [ ] Twelve `StoreRequest` and four `StoreReply` variants, all served in `try_serve` through `hierarchy::serve`; identity filled worker-side; `Offline` refuses with `DATABASE_UNREACHABLE`
- [ ] CAS miss reloads, keeps the text, retries only on `Enter`; stale-row test through `serve`
- [ ] `canonical_root` in `htui-core`: relative/missing/dangling/not-a-directory refused with the four texts; a link to a directory is stored canonical; tests with `tempfile`
- [ ] Delete: counts shown before the act from `DeleteReach`, second confirmation typed, `Deleted.reach` equals the shown counts; project delete rebuilds the mirror from the worker; workspace delete does not
- [ ] `p` moves the primary; no key unsets it; editor never carries `is_primary`
- [ ] `CASES` 36, `EXPECTED_CASES` 36, `.sqlx/` and `migrations/` untouched, no `WriteStore` change, `chat/composer.rs` untouched
- [ ] Seven `hierarchy__*.snap` read and accepted; `shell_empty.snap` updated; `settings__agents_*` unchanged
- [ ] `cargo fmt --check`, `clippy -D warnings`, `cargo doc`, workspace tests green; HANDOFF milestone 3 paragraph and PRD row written
