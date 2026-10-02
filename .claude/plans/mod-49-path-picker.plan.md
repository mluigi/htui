# Plan: MOD-49 Interactive path picker for repo and workspace roots

**Source**: HANDOFF `MOD-49` (from MOD-7; `R-BOX-4`, `R-TUI-8`, `R-NF-3`, F-102)
**Routed**: plan path via `/handoff-run` (C2 fired; C3/C4 borderline, low confidence), accepted by the maintainer 2026-10-02.
**Complexity**: Medium
**Status**: CONFIRMED 2026-10-02 (maintainer: "proceed"), implementation in progress

## Summary

Settings > Hierarchy's `b` currently opens a one-field text editor for this box's workspace root
(workspace row) or a repo's checkout path (repo row). MOD-49 replaces that editor with a popup that
lists directories on this box, lets the user move through them, and chooses one. The chosen path is
sent through the **unchanged** `SetWorkspaceRoot` / `SetRepoPath` requests, so the store worker's
existing `canonical` → `htui_core::root_path::canonical_root` guard (F-102) still decides what is
stored. Listing is a new store-worker request served under `spawn_blocking`, so the UI task never
touches the disk (`R-NF-3`).

## Design decisions (proposed, maintainer may amend at CONFIRM)

| # | Decision | Why |
|---|---|---|
| P1 | **This box only, local filesystem.** No remote listing protocol. | `b` writes this box's row only (`open_path` doc, `ui/tabs/settings/hierarchy.rs:562`; store side `box_id(this_box)`, `hierarchy.rs:362`). This settles the routing verdict's open point (C3). |
| P2 | **Picker is a reusable component, `crate::ui::path_picker::PathPicker`, embedded in a section as a mode** (`Mode::Picking`), drawn centred over the section with `Clear` + bordered `Block`. **Not** a registered `Overlay`. | Overlay factories are `Fn() -> Box<dyn Overlay>` with no arguments (`ui/overlay/registry.rs`) and no way to return a result to a section; a picker needs a start path in and a chosen path out. A component the section owns needs neither. |
| P3 | **Listing = `StoreRequest::ListDir { path: String }` → `StoreReply::DirListing(DirListing)`**, served in `store_worker::try_serve` with a `spawn_blocking` around a sync `htui_core::root_path::list_dirs`. A refused listing is `StoreReply::Failed { request: "list_dir", .. }`. | Sections and overlays get data only via `StoreReply` ("holds no store handle and no channel"). There's precedent for requests that never touch the store: the Qdrant keyring arms, `store_worker.rs:2168-2214`. `root_path` is already the "sync, std::fs only" module the worker wraps (`hierarchy.rs:459-475`). |
| P4 | **What a listing shows:** directories only, plus links that resolve to a directory (marked `@`, **target never shown**, `R-BOX-4`). Files, dangling links and unreadable entries are dropped. Byte order. Hidden (`.`-prefixed) entries are hidden unless toggled. Capped at **1000** entries; the excess is reported as `+N more` (no silent truncation). | Same link rule as `canonical_root`/excerpts. The cap bounds the reply and the render. |
| P5 | **Navigation never canonicalises.** The picker joins/strips path components lexically, so the header shows the path as navigated, never a link's target. Canonicalisation happens once, on choose, in the existing write. | `R-BOX-4`: a refusal or a display never names a link target. |
| P6 | **Typing survives inside the picker:** `/` opens a one-line "go to" `TextField` (accepts bracketed paste); `Enter` lists that path. | "Replace the typed fallback" removes the editor. Pasting a long path is still the fastest route, and the popup keeps it. |
| P7 | **Start directory:** the stored path when there is one. Otherwise, for a repo, the workspace root on this box. Otherwise `$HOME`, then `/`. If listing the start fails, the error shows in the popup and `h` still goes up. | Opens where the user most likely wants to be. |
| P8 | **Keys:** `j`/`k` move · `Enter`/`l` open · `h`/`Backspace` parent · `s` choose highlighted · `S` choose the listed directory · `/` go to · `.` toggle hidden · `Esc` cancel. | Matches the tab's `j`/`k`/`h`/`l` idiom. `captures_input` is true while picking, so `h`/`l` stop cycling sections (same as the editor, D2). |
| P9 | **Stale replies:** the picker remembers the path it last asked for and ignores a `DirListing` for any other path. | The staleness index keeps the newest request of a kind, but a late reply for an older path must not overwrite the current one. |
| P10 | **Boxes section is not wired.** It has no path field. MOD-7 D115/OQ-28 moved the typed fallback to Hierarchy's `b`. The component stays reusable for any later path field. | The HANDOFF text's "repo paths in the box section" predates D115. |
| P11 | `ListDir` is a read: it does **not** set the section's `busy`. `b` is still refused while a write is in flight (existing `refuse('b')`). | Same as `r`, which is allowed while busy. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Popup render | `crates/htui/src/ui/overlay/workspace_switcher.rs` (`render`, `lines`, `CURSOR`/`NO_CURSOR`, `HINT`) | `centered` + `Clear` + bordered `Block`, cursor marker visible in snapshots, a dim hint line, "reading…" vs empty text distinct |
| Section modes | `crates/htui/src/ui/tabs/settings/hierarchy.rs` `Mode`, `on_key`, `on_paste`, `captures_input` | Mode enum, key dispatch per mode, paste routed to the focused `TextField` |
| Blocking FS off the UI task | `crates/htui/src/hierarchy.rs:459-480` `canonical` | `spawn_blocking` around a sync `htui_core::root_path` fn, a refusal turned into `StoreError::Constraint`, path as typed only |
| Sync FS guard | `crates/htui-core/src/root_path.rs` `canonical_root`, `RootRefusal` | Typed refusal enum, fixed check order, never names a link target, unit tests in-module with `tempfile` |
| Request plumbing | `crates/htui/src/store_worker.rs` `StoreRequest`/`StoreReply`/`name()`/`try_serve` | One variant + `name()` arm + serve arm, doc comment naming the plan decision |
| Section integration tests | `crates/htui/tests/hierarchy.rs` (`hierarchy_over`, `harness.key`, `settle`, `type_into`, `insta::assert_snapshot!`) | Drive the section through the harness, assert the request sent, snapshot the frame |
| Errors | `StoreReply::Failed { request, message }` | Picker shows the message inline, never panics |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-core/src/root_path.rs` | UPDATE | `DirListing`, `DirEntry`, `list_dirs(path, show_hidden, cap)`, `ListRefusal` (or reuse `RootRefusal`), unit tests | T1 |
| `crates/htui/src/store_worker.rs` | UPDATE | `ListDir` request, `DirListing` reply, `name()` = `"list_dir"`, `try_serve` arm under `spawn_blocking` | T2 |
| `crates/htui/tests/hierarchy.rs` (store-level part) | UPDATE | `serve(ListDir)` against a tempdir: listing, refusal, link marker, cap | T2 |
| `crates/htui/src/ui/path_picker.rs` | CREATE | `PathPicker` component: state, keys → `PickerOutcome`, `on_reply`, `render`; in-module unit tests | T3 |
| `crates/htui/src/ui/mod.rs` | UPDATE | `pub mod path_picker;` + re-export | T3 |
| `crates/htui/src/ui/tabs/settings/hierarchy.rs` | UPDATE | `Mode::Picking { kind, picker }`. `open_path` opens the picker. Choosing submits `SetWorkspaceRoot`/`SetRepoPath`. Route `DirListing`/`Failed{list_dir}` to the picker. Update the hint. Drop the `b` text-editor kinds if nothing else uses them. | T4 |
| `crates/htui/tests/hierarchy.rs` (section part) + `crates/htui/tests/snapshots/hierarchy__picker*.snap` | UPDATE/CREATE | Flow tests (`b` → `ListDir(start)` → navigate → `s` → `SetRepoPath{path}`), workspace row, `Esc` sends nothing, stale reply ignored, snapshot(s) | T4 |
| `README.md` (line 300), `docs/decisions/mod/mod-49.md`, `DECISIONS.md` index, `HANDOFF.md` (item close + pins) | UPDATE/CREATE | Close-out bookkeeping | T5 |

## Tasks

**Order: serial, T1 → T2 → T3 → T4 → T5.** The file sets of T2 and T4 intersect (`tests/hierarchy.rs`).
T3 depends on T2's `StoreRequest::ListDir` and T1's `DirListing`. **No parallel fan-out.** One
implementer runs the chain and commits after each task (memory: implementer agents commit
incrementally).

### Task 1: `list_dirs` in `htui_core::root_path` (TDD)
- **Tests first** (in-module, `tempfile`): directories only, byte order; files excluded; a link to a
  directory included and marked as a link, its target not in any field or message; a dangling link
  excluded; hidden excluded unless `show_hidden`; `cap` truncation reports `more = N`; a relative
  path, a missing path and a file are refused with the as-typed path; an unreadable entry is skipped
  and doesn't fail the listing.
- **Action**: `pub struct DirListing { path: String, entries: Vec<DirEntry>, more: usize }`,
  `pub struct DirEntry { name: String, is_link: bool }`, `pub fn list_dirs(path: &Path, show_hidden:
  bool, cap: usize) -> Result<DirListing, RootRefusal>`. Names that aren't UTF-8 are skipped (they
  can't be stored anyway; same rule as `canonical`).
- **Mirror**: `canonical_root` + its tests.
- **Validate**: `cargo test -p htui-core root_path`

### Task 2: `ListDir` / `DirListing` through the store worker (TDD)
- **Tests first** (`crates/htui/tests/hierarchy.rs`, store level, `MemStore::demo()` backend): a
  tempdir lists its subdirectories; a missing path answers
  `Failed { request: "list_dir", message }` that names the path as typed; `name()` is `"list_dir"`.
- **Action**: request + reply variants with doc comments, `name()` arm, `try_serve` arm wrapping
  `list_dirs` in `spawn_blocking` (cap 1000, P4), with the refusal mapped to `StoreError::Constraint`.
  Fix any other exhaustive `match` the compiler names.
- **Validate**: `cargo test -p htui --features testkit --test hierarchy -- --test-threads=1`

### Task 3: `PathPicker` component (TDD)
- **Tests first** (in-module, pure state): `open(start)` yields `ListDir(start)`; a reply for another
  path is ignored (P9); `j`/`k` clamp; `Enter`/`l` on an entry yields `ListDir(start/entry)`; `h` at
  `/` stays; `h` yields the lexical parent (P5); `s` yields `Chosen(path/entry)`; `S` yields
  `Chosen(path)`; `/` + typed/pasted text + `Enter` yields `ListDir(typed)`; `.` re-lists with hidden
  shown; `Esc` yields `Cancelled`; a `Failed` reply shows the message and keeps `h` working.
- **Action**: `PathPicker { path, listing: Option<DirListing>, error, cursor, show_hidden, goto:
  Option<TextField>, asked: String }`, with `on_key(KeyEvent) -> PickerOutcome { None, Request(StoreRequest),
  Chosen(String), Cancelled }`, `on_paste`, `on_reply(&StoreReply) -> bool`, and
  `render(frame, area, theme)`, which mirrors `WorkspaceSwitcher::render`. The picker holds no channel:
  the owning section turns `Request` into `ctx.request`.
- **Validate**: `cargo test -p htui path_picker`

### Task 4: Hierarchy `b` opens the picker (TDD)
- **Tests first** (`tests/hierarchy.rs`, harness): `b` on a repo row sends `ListDir(<stored or
  workspace root or $HOME>)`, and the frame shows the popup (snapshot `hierarchy__picker`).
  Navigating then `s` sends `SetRepoPath { project, repo, path }` with the chosen path, and the
  reply's canonical path appears in the tree as before. `b` on the workspace row sends
  `SetWorkspaceRoot`. `b` on a project row keeps today's notice. `Esc` closes and sends no write.
  While picking, `h`/`l` do not cycle sections (`captures_input`). A `stored as …` notice and the
  follow-up `InferRepoPaths` (D136) still fire after a root write. A refused write (link to nothing)
  shows the existing `constraint violated` notice.
- **Action**: `Mode::Picking { kind: EditorKind, picker: PathPicker }`; `open_path` builds the picker
  (P7); `on_key`/`on_paste`/`on_reply`/`render` route to it; on `Chosen` → `submit`-equivalent for the
  two kinds (reusing the existing request construction at `hierarchy.rs:919-930` and `written` /
  `reload` / `on_stale` handling); update `hint_text`. Drop the `b` paths of `Mode::Editing` if they
  become dead.
- **Validate**: `cargo test -p htui --features testkit --test hierarchy -- --test-threads=1`, then
  `cargo insta` review of the new snapshot(s).

### Task 5: Close-out docs
- `docs/decisions/mod/mod-49.md` (P1–P11 as decided, deviations, pins moved), `DECISIONS.md` index,
  README line 300 (`b` picks a directory), HANDOFF close-out per `lifecycle.md` P2. Re-count the
  pins (`StoreRequest`, `StoreReply`, snapshots) instead of incrementing them.

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core
cargo test -p htui --all-features -- --test-threads=1      # testkit, else tests/*.rs run 0 tests
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A cold/hung mount stalls the store-worker loop while a listing is awaited inline (the UI stays responsive; other store replies queue) | Low | Same exposure `canonical` already has. Cap the entries. If the reviewer objects, defer the arm like the runtime requests (`Served::Deferred`). |
| A link target leaks through display or error text (`R-BOX-4`) | Medium | P5 lexical navigation, `DirEntry` carries no target, and a T1 test asserts the target string is absent. |
| Removing the text editor drops paste-a-path | Medium | P6 `/` go-to with `on_paste`, T3 test. |
| Pin drift: HANDOFF says `StoreRequest` 91 / `StoreReply` 52 / snapshots 127, a rough count of the tree gives 95/54/128 entries | Medium | Re-count at T5 and don't increment. Not this item's defect, but the close-out writes the true numbers. |
| Disk | Low | The repo is on `/dev/sda` (2.8T free); `/` is only the sandbox overlay. |

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `b` edits only this box's path (workspace root / repo path) | ✅ true | `crates/htui/src/ui/tabs/settings/hierarchy.rs:562` doc "`b`: this box's path…"; store side `box_id(this_box)` `crates/htui/src/hierarchy.rs:362` |
| `SetRepoPath` canonicalises via `canonical_root` on the worker | ✅ true | `crates/htui/src/hierarchy.rs:355-365` → `canonical` `:469-480` → `canonical_root` |
| `SetWorkspaceRoot` canonicalises the same way | ✅ true | `crates/htui/src/hierarchy.rs:263` `let root_path = canonical(path).await?` |
| `canonical_root` is sync `std::fs`, meant to be wrapped in `spawn_blocking` | ✅ true | `crates/htui-core/src/root_path.rs:33-35` doc |
| Overlay factories take no arguments and have no result path to a section | ✅ true | `crates/htui/src/ui/overlay/registry.rs` `type Factory = Box<dyn Fn() -> Box<dyn Overlay>>`; effects leave only via `Ctx` actions |
| Sections and overlays receive data only through `StoreReply` | ✅ true | `workspace_switcher.rs` module doc "holds no store handle and no channel: data arrives through `on_reply`" |
| The store worker already serves requests that never touch the store, under `spawn_blocking` | ✅ true | `crates/htui/src/store_worker.rs:2168-2214` (`QdrantInfo`, `SetQdrantUrl`, …) |
| The Boxes section has no path/root field | ✅ true | no `path`/`root` match in `crates/htui/src/ui/tabs/settings/boxes.rs` |
| The typed fallback is Hierarchy's `b`, not a Boxes field (D115/OQ-28) | ✅ true | `docs/decisions/mod/mod-7.md:193` |
| `TextField` supports bracketed paste | ✅ true | `hierarchy.rs` `on_paste` → `field.input.on_paste(text)` |
| `centered` layout helper exists | ✅ true | `use crate::ui::layout::centered` in `workspace_switcher.rs` |
| The `hierarchy__editor_repo` snapshot is the `b` editor | ❌ **false**: it is `n` on a project (new-repo editor) | `crates/htui/tests/hierarchy.rs:1331-1346`. No existing snapshot changes; the `b` editor has no UI test today, so T4 adds the first. |
| A new `StoreRequest` variant needs arms in `name()` and `try_serve` | ✅ true (others surfaced by the compiler) | `InferRepoPaths` occurs at `store_worker.rs:491, 931, 1562`, no other exhaustive site in `src` |
| `tests/hierarchy.rs` integration tests need `--features testkit` | ✅ true (memory, `htui-integration-tests-need-testkit`) | Validation commands use `--all-features` |
| T2 and T4 are independent | ❌ **false**: both touch `crates/htui/tests/hierarchy.rs` | file-set intersection, so the chain runs serial |
| T3 is independent of T2 | ❌ **false**: the picker emits `StoreRequest::ListDir` | type dependency, so it runs serial |
