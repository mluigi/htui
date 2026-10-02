# Blueprint: MOD-49, interactive path picker for repo and workspace roots

**Status**: **proposed** (2026-10-02). Plan deviations B-1 to B-9 (§0) and decisions D1–D24 (§7)
belong to this blueprint. A deviation marked **Blocker** means the plan, read literally, fails its
own gate or claims something the tree can't show. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-49-path-picker.plan.md` at `9a60cf96`, status **CONFIRMED**
2026-10-02, fact-checked (16 claims, 3 marked false at step 3.5). P1–P11 and the "Verified claims"
table bind this blueprint and aren't reopened here. Tasks are cited as "MOD-49 T*n*" outside this
file.

**Verified at**: HEAD `9a60cf96`, branch `hr/MOD-49`, clean tree. Every anchor below was located
through Gortex (`search`, `read`) and re-read at its line. **Line numbers are pre-edit**: once a
task commits to a file, a citation into that file moves. Counted at this HEAD: `StoreRequest`
**95** variants, `StoreReply` **54**, `crates/htui/tests/snapshots/` **128** files (HANDOFF.md:61-63
still says 91 / 52 / 127, the drift the plan's risk row names), `hierarchy::REQUEST_NAMES` 13.
`df -h /`: 135 GB free (70 %).

**Graphify**: `graphify-out/` doesn't exist in this checkout, so nothing here comes from it.

**Coupling verdict.** The plan's order stands: **serial T1 → T2 → T3 → T4 → T5, one implementer,
no fan-out**. One file joins a task's list. T4 rewrites two **existing** tests in
`crates/htui/tests/hierarchy.rs` (B-2). That file is already T4's, so nothing crosses a task
boundary. T2 doesn't touch `crates/htui/src/hierarchy.rs`: the listing is served from
`store_worker.rs` (D2), which keeps T2's file set exactly what the plan lists. T3 depends on T2's
`StoreRequest::ListDir` and on T1's `DirListing`. T4 depends on T3. That's the dependency the plan
already records.

**Scope**:
- **No migration, no `.sqlx`, no seam change** (`ReadStore`/`WriteStore` untouched, store `CASES`
  stay 119).
- **New API**: `htui_core::root_path::{DirEntry, DirListing, list_dirs}` plus one `RootRefusal`
  variant (T1). `StoreRequest::ListDir`, `StoreReply::DirListing`, `store_worker::{LIST_DIR,
  LIST_CAP}` (T2). `htui::ui::path_picker::{PathPicker, PickerOutcome, start_dir}` (T3). One
  `HierarchySection::with_home` builder (T4).
- **Pins that move** (re-count at T5, don't add to the HANDOFF numbers): `StoreRequest` 95 → **96**,
  `StoreReply` 54 → **55**, `crates/htui/tests/snapshots` 128 → **129** (T4 adds
  `hierarchy__picker.snap`; no existing snapshot changes, D17). Unmoved: `hierarchy::REQUEST_NAMES`
  13 (D2), seven Settings sections, store `CASES` 119, `.sqlx` 307.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_docs` warns, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings` is in the gate. rustdoc denies broken and private intra-doc links.
  A `pub` item's doc can't link a private fn, and a doc written before its target exists uses plain
  backticks.
- Implementers commit after each step and stage only their own paths: never `-A`, never `stash`,
  never `--amend`. Every commit compiles. A red commit may put a `todo!()` body **only** in an item
  that no existing path calls (MOD-7 H-6).
- Integration tests need `--features testkit` or `--all-features`. Without them, `tests/*.rs` runs 0
  tests and still reports ok. Every `htui` gate uses `--test-threads=1` (the keyring fake is
  process-wide).
- `R-BOX-4`: no display, refusal or `Debug` of a reply names a link's target.

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `9a60cf96` | Fix |
|---|---|---|---|---|
| **B-1** | **Blocker** (T3's test can't be written) | P3: `StoreRequest::ListDir { path: String }`. T1: `list_dirs(path, show_hidden, cap)`. T3: "`.` re-lists with hidden shown". | A request carrying only `path` can't ask the worker for hidden entries, so "re-lists with hidden shown" has no wire to travel on. Filtering on the client instead would let dot-entries use up the 1000 cap before any visible directory (P4). | D1: `ListDir { path: String, show_hidden: bool }`. The worker passes the flag to `list_dirs`. |
| **B-2** | **Blocker** (two existing tests go red) | Verified-claims row: "the `b` editor has no UI test today, so T4 adds the first." | `b_on_the_workspace_row_sets_the_root` (`tests/hierarchy.rs:1742-1763`) and `a_root_write_that_applied_is_followed_by_one_inference` (`:1971-2002`) both press `b`, type `/srv/htui` and press `Enter` on a `SectionBench`. With the editor gone, `b` emits a `ListDir`, `/` opens go-to, and the assertions fail. (The row is right that no **snapshot** covers `b`.) | T4 rewrites both onto the picker (§5.6): `b_on_the_workspace_row_picks_the_root` and the same-named follow-up test, which drives `S` on a synthetic listing. |
| **B-3** | **Blocker** (the refusal reaches no one) | P3: a refused listing is `Failed { request: "list_dir", .. }`. Plan § Files: "Route `DirListing`/`Failed{list_dir}` to the picker". | `HierarchySection::on_reply` matches `Failed` only for `"hierarchy"` (`ui/tabs/settings/hierarchy.rs:1270`) and for `REQUEST_NAMES.contains(request)` (`:1277`). Adding `"list_dir"` to `REQUEST_NAMES` breaks `offline_refuses_every_hierarchy_request_by_name` (`tests/hierarchy.rs:677-694`): a listing needs no store, so offline it lists rather than answering `DATABASE_UNREACHABLE`. It also breaks `hierarchy_names_are_stable` (`:101`). | D2: `store_worker::LIST_DIR = "list_dir"`, deliberately **outside** `hierarchy::REQUEST_NAMES` (13 stays). The section gets its own arm, `DirListing(_) \| Failed { request: LIST_DIR, .. }`, placed before `:1270`. |
| **B-4** | Non-blocker (dead arms) | T4: `Mode::Picking { kind: EditorKind, picker: PathPicker }`, "drop the `b` paths of `Mode::Editing` if they become dead". | `submit`'s `match editor.kind` has no wildcard (`:919-930`). Keeping `EditorKind::WorkspaceRoot`/`RepoPath` for the picker leaves two `submit` arms that nothing reaches. `written` (`:990-1010`) compares against `editor.text(0)`, and `on_tree`'s follow-up reads `Mode::Editing(editor)` (`:941-951`). The picker has neither. | D7: a new `PathTarget` enum replaces both kinds. `EditorKind` loses them, `Mode::Picking { target, picker, chosen }` carries the chosen string that `written` compares, and `PathTarget::request` takes over the request construction at `:919-930`. |
| **B-5** | Non-blocker (an untestable test, a wrong sentence) | T1: "an unreadable entry is skipped and doesn't fail the listing". `list_dirs` returns `RootRefusal`. | (1) A `read_dir` entry that yields `Err` can't be produced on purpose. A mode-`000` directory is still **listed**, because its type comes from the dirent, and the sandbox may run as root, which ignores modes. (2) When `read_dir` fails on the directory itself (EACCES), no `RootRefusal` fits. `canonical_root` folds that case into `Missing` (`root_path.rs:51-54`), but "does not exist on this box" is false for a directory the user can see in its parent's listing. | D3: `RootRefusal::Unreadable(PathBuf)`, "`` `{}` cannot be read on this box ``", returned only by `list_dirs`. T1's test `a_directory_that_cannot_be_read_is_refused_as_unreadable` checks `read_dir` itself first and returns early when that succeeds (as root). The skipped-`Err`-entry branch is covered by review. T1 adds `a_name_that_is_not_utf8_is_skipped` and `a_link_to_a_file_is_dropped` instead. |
| **B-6** | Non-blocker | T4 test: "A refused write (link to nothing) shows the existing `constraint violated` notice." | The picker can't choose a link to nothing. P4 drops dangling links from every listing, and `s`/`S` act only on a landed listing (D11), so a dangling path never reaches `SetRepoPath`. | T4's `a_directory_gone_before_the_choice_is_refused_on_the_status_line` (harness, real worker): list a tempdir, remove the highlighted directory, press `s`. The status line reads ``set_workspace_root: constraint violated: `…` does not exist on this box`` and the picker stays open (D8). |
| **B-7** | Non-blocker (determinism) | P7: "Otherwise `$HOME`, then `/`." | `HierarchySection::new()` is `Self::default()` (`:241-243`) and is built in 28 places (`app/mod.rs:62` and 27 tests). A `$HOME` read inside `open_path` would make every `b` test, and any snapshot, depend on the box running it. | D9: `home: Option<String>` is read once in `new()` (`std::env::var("HOME").ok()`). `pub fn with_home(self, home: Option<&str>) -> Self` overrides it for tests. The choice is a pure `start_dir(stored, fallback, home)`, unit-tested in T3. |
| **B-8** | Non-blocker (precision) | Patterns: the popup "mirrors `WorkspaceSwitcher::render`", which sizes its box to its content (`workspace_switcher.rs:197-205`). | A listing of up to 1000 entries can't be sized to its content: the box would leave the screen. | D14: a fixed box, `centered(area, area.width.saturating_sub(4).min(96), area.height.saturating_sub(2))`, with a stateless scroll window. Everything else mirrors the switcher: `Clear`, bordered `Block`, `theme.title` title, `> ` marker, dim hint. |
| **B-9** | Non-blocker (bookkeeping) | Risk row: pins "rough count … 95/54/128". | Re-counted at this HEAD: `StoreRequest` **95**, `StoreReply` **54**, snapshots **128**. | T5 writes **96 / 55 / 129** to `HANDOFF.md:61-63` and doesn't increment from 91 / 52 / 127. |

### 0a. Hazards, each with its guard

| # | Hazard | Guard |
|---|---|---|
| **H-1** | `R-BOX-4`: a link target leaks through a listing, its `Debug`, a header or a refusal. | `DirEntry` has no target field. `DirListing.path` echoes the path as given (D4). The picker joins and strips components lexically and never canonicalises (P5, D23). Two tests (T1 `a_link_to_a_directory_is_listed_and_marked_without_its_target`, T2 `a_link_is_listed_without_its_target`) assert that `format!("{listing:?}")` / `format!("{reply:?}")` doesn't contain the target's name `secret-target-name`. |
| **H-2** | The store worker loop stalls while a listing of a cold or hung mount runs (the plan's risk row). | Same exposure as `canonical` (`hierarchy.rs:469-484`). `spawn_blocking` keeps it off the async thread. `LIST_CAP` (1000) bounds the reply and the render, but not the walk: all qualifying names are read and sorted before truncation (D5). If the reviewer objects, defer it like the runtime requests (plan risk row). Not built now. |
| **H-3** | A `Failed { request: "list_dir" }` also goes to the status line. `App::on_reply` turns **every** fresh `Failed` into `Action::Error` (`app/update.rs:285-287`). | Accepted, D16: P3 binds the `Failed` shape. The status line clears on the next key (`the_next_key_clears_a_failure_from_the_status_line`), and the popup keeps the message until the next listing. |
| **H-4** | A key while picking reaches the global table: `q` quits, `?` opens help. | The picker swallows every key it doesn't bind (D15, `PickerOutcome::None` becomes `Handled::Consumed`). The section lets `CONTROL` chords through **before** it asks the picker, so `ctrl-c` still quits (`on_deleting_key`'s rule, `:688-690`). Tested by `ctrl_c_passes_through_the_picker` and `the_picker_swallows_q`. |
| **H-5** | The snapshot shows a tempdir path, so the frame moves from box to box. | D18: `hierarchy__picker` is rendered with `SectionBench::render_section(&section, 100)` (`testkit.rs:725`) from **synthetic** `DirListing` replies at `/srv`. The section never stats a path. Harness tests that run the real worker assert with `contains` and take no snapshot. |
| **H-6** | A red commit routes a live path to a `todo!()`. | T1's red `list_dirs`, T2's red `store_worker::list_dir` and T3's red `PathPicker` bodies are reached only by the new tests: the new request, the new module. T4's red commit adds only tests and the `home` field with `with_home`, and nothing in the product calls them yet. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (each compiles) | Gate (`--test-threads=1` throughout) |
|---|---|---|---|
| T1 `list_dirs` | htui-core | 2 (§2.4) | `cargo test -p htui-core root_path`; `cargo clippy -p htui-core --all-targets --all-features -- -D warnings` |
| T2 `ListDir` | htui | 2 (§3.5) | `cargo test -p htui --features testkit --test hierarchy -- --test-threads=1`; `cargo test -p htui --lib store_worker -- --test-threads=1`; clippy `-p htui` |
| T3 `PathPicker` | htui | 2 (§4.5) | `cargo test -p htui --lib path_picker -- --test-threads=1`; clippy `-p htui` |
| T4 Hierarchy `b` | htui | 2 (§5.7) | `cargo test -p htui --features testkit --test hierarchy -- --test-threads=1`; `cargo insta pending-snapshots` empty after accept; `ls crates/htui/tests/snapshots \| wc -l` = **129**; then the full `htui` suite |
| T5 docs | — | 1 (§6) | `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` |
| close | — | — | the plan's whole Validation block, on the real tree |

---

## 2. T1: `list_dirs` in `htui_core::root_path` (P4, P5)

**File**: `crates/htui-core/src/root_path.rs` (only). `tempfile = "3"` is already a dev-dependency
(`crates/htui-core/Cargo.toml`, `[dev-dependencies]`, with a comment naming `root_path`'s guard
tests), so the manifest doesn't change.

### 2.1 `RootRefusal` gains one variant (D3), after `NotADirectory` (`:21-23`)

```rust
    /// The directory exists and could not be read (MOD-49, blueprint D3). Only
    /// [`list_dirs`] answers it: [`canonical_root`] never reads a directory's entries.
    #[error("`{}` cannot be read on this box", .0.display())]
    Unreadable(PathBuf),
```

Nothing matches `RootRefusal` exhaustively. Its only users are `canonical_root`, the module's tests
and the doc link at `crates/htui/src/hierarchy.rs:216`.

### 2.2 Types and function, after `canonical_root` (`:40-61`), before `#[cfg(test)]` (`:63`)

```rust
/// One entry of a [`DirListing`] (MOD-49 P4): a directory, or a link that resolves to one.
///
/// Carries the entry's **name** only — never a path and never what a link points at (`R-BOX-4`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// The file name, UTF-8 (a name that isn't is skipped).
    pub name: String,
    /// Whether the entry is a symlink that resolves to a directory; the picker marks it `@`.
    pub is_link: bool,
}

/// What [`list_dirs`] found in one directory (MOD-49 P3/P4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirListing {
    /// The directory listed, **exactly as given**: never canonicalised, so it never names a link's
    /// target, and the picker can match a reply to the path it asked for (P9).
    pub path: String,
    /// The directories, in byte order of their names, at most `cap` of them.
    pub entries: Vec<DirEntry>,
    /// How many more qualifying entries the cap left out (`+N more`; nothing is cut silently).
    pub more: usize,
}

/// The subdirectories of `path`, or why it cannot be listed (MOD-49 P4).
///
/// Checked in [`canonical_root`]'s order — relative, missing, dangling, not a directory — then
/// [`RootRefusal::Unreadable`] when `read_dir` itself fails. Kept: directories, and links that
/// resolve to a directory (`is_link`). Dropped: files, dangling links, links to files, names that
/// aren't UTF-8, entries `read_dir` yields as errors, and `.`-prefixed names unless `show_hidden`.
/// Every qualifying name is read and sorted before `cap` applies, so which entries are shown never
/// depends on the order the filesystem returned them.
///
/// Sync and `std::fs` only, like [`canonical_root`]: `htui`'s store worker wraps it in
/// `spawn_blocking`.
///
/// # Errors
/// [`RootRefusal`], carrying `path` as given.
pub fn list_dirs(path: &Path, show_hidden: bool, cap: usize) -> Result<DirListing, RootRefusal>
```

Body (D3, D5, D6):
1. `!path.is_absolute()` → `Relative`.
2. `symlink_metadata(path)` `Err` → `Missing`.
3. `metadata(path)` (follows links): `Err` with `meta.is_symlink()` → `Dangling`, any other `Err` →
   `Missing`. `Ok(m)` with `!m.is_dir()` → `NotADirectory`.
4. `read_dir(path)` `Err` → `Unreadable`.
5. For each `Ok(entry)`: `name = entry.file_name().into_string()`, and an `Err` skips the entry.
   Hidden: `!show_hidden && name.starts_with('.')` skips it. Then `entry.file_type()`:
   - `is_dir()` → `DirEntry { name, is_link: false }`;
   - `is_symlink()` → `std::fs::metadata(entry.path())` `is_ok_and(is_dir)` gives
     `DirEntry { name, is_link: true }`, otherwise the entry is skipped (dangling, or a link to a
     file);
   - anything else, or a `file_type()` `Err`, is skipped.
6. Sort `entries` by `name` (`String`'s `Ord` is byte order). `more = len.saturating_sub(cap)`,
   then `entries.truncate(cap)`.
7. `path: path.to_string_lossy().into_owned()`. This is lossless for every caller, because the
   worker builds the `Path` from a `String` (D4).

### 2.3 Tests (first), in `mod tests` (`:63-172`), importing `{DirEntry, DirListing, list_dirs}`

| Test | Pins |
|---|---|
| `a_listing_holds_directories_only_in_byte_order` | dirs `b`, `a`, `B` and a file `f.txt` give `["B", "a", "b"]`, `more == 0`, every `is_link == false` |
| `hidden_entries_are_dropped_unless_asked` | `.git` is absent with `false` and present with `true` |
| `the_cap_keeps_the_first_entries_and_counts_the_rest` | five dirs at `cap = 3` give `a b c`, `more == 2` |
| `a_listing_refuses_as_the_guard_does` | relative `x/y` → `Relative`, missing → `Missing`, a file → `NotADirectory`, each carrying the path as given |
| `the_listing_echoes_the_path_as_given` | a path with a trailing `/` and a `.` component comes back byte-identical in `listing.path` |
| `a_link_to_a_directory_is_listed_and_marked_without_its_target` (`#[cfg(unix)]`) | link `shared` → dir `secret-target-name` gives `DirEntry { name: "shared", is_link: true }`, and `format!("{listing:?}")` doesn't contain `secret-target-name`. The target dir itself is listed only under its own name. |
| `a_dangling_link_and_a_link_to_a_file_are_dropped` (`#[cfg(unix)]`) | neither appears in the listing |
| `a_dangling_link_cannot_be_listed` (`#[cfg(unix)]`) | listing the link itself → `Dangling(link)`, and its sentence doesn't contain the target name |
| `a_name_that_is_not_utf8_is_skipped` (`#[cfg(unix)]`) | dir `OsStr::from_bytes(b"not\xffutf8")` is absent and the listing still succeeds |
| `a_directory_that_cannot_be_read_is_refused_as_unreadable` (`#[cfg(unix)]`) | a dir chmod `0o000`. If `std::fs::read_dir` succeeds anyway (root), the test restores the mode and **returns early**. Otherwise `Unreadable(dir)` with the sentence ``` `…` cannot be read on this box ```. It restores `0o755` before the tempdir drops. |

### 2.4 Commits (T1)

1. `test(mod-49): list_dirs tests` — §2.1, §2.2 with body `todo!("MOD-49 T1")`, §2.3. Red, and
   nothing outside the tests calls `list_dirs`.
2. `feat(mod-49): list_dirs lists directories and links to them` — the body. Gate green.

---

## 3. T2: `ListDir` / `DirListing` through the store worker (P3, P11)

**Files**: `crates/htui/src/store_worker.rs`, `crates/htui/tests/hierarchy.rs` (store half).

### 3.1 Imports and constants (`store_worker.rs`)

- After `use htui_core::prompt::SettingKey;` (`:32`): `use htui_core::root_path::{DirListing, list_dirs};`.
- After `PROMPT_PREVIEW` (`:72`):

```rust
/// [`StoreRequest::name`] of [`StoreRequest::ListDir`] (MOD-49 plan P3, blueprint D2). Not in
/// `hierarchy::REQUEST_NAMES`: a listing reads this box's filesystem and no store, so it is not one
/// of the thirteen offline refusals, and the Hierarchy section matches its `Failed` by this name.
pub const LIST_DIR: &str = "list_dir";

/// The most entries one [`StoreReply::DirListing`] carries; the rest is its `more` (P4).
pub const LIST_CAP: usize = 1000;
```

### 3.2 The variants

`StoreRequest`, after `InferRepoPaths(WorkspaceId),` (`:491`), before the catalogue comment
(`:493`). It sits outside the "thirteen" comment block, so that prose (`:393-397`) stays true:

```rust
    // MOD-49 (plan P3): not one of the thirteen above — no store, no box row, no writer.
    /// The directories in `path` on **this** box, for the path picker (MOD-49 P1, P3, P4). A read:
    /// the section that sends it marks nothing busy (P11). Answers [`StoreReply::DirListing`], or
    /// `Failed { request: "list_dir" }` carrying the guard's sentence about the path as typed.
    ListDir {
        /// The directory to list, as the picker navigated to it (never canonicalised, P5).
        path: String,
        /// Whether `.`-prefixed entries are listed (blueprint D1).
        show_hidden: bool,
    },
```

`StoreReply`, after `RepoPathsInferred { … },` (`:1107-1115`), before `Catalogue` (`:1116`):

```rust
    /// Answer to [`StoreRequest::ListDir`] (MOD-49 P3): directories and links to directories,
    /// names only, never a link's target (`R-BOX-4`).
    DirListing(DirListing),
```

Unboxed (P3 binds the shape). It's 56 bytes, below `Failed`'s neighbours, so
`clippy::large_enum_variant` stays quiet. Both enums derive only `Debug, Clone`
(`:107`, `:992`), which `DirListing` has.

### 3.3 `name()` and `try_serve`

- `name()`, after `Self::InferRepoPaths(..) => "infer_repo_paths",` (`:931`):
  ```rust
              // MOD-49: outside `hierarchy::REQUEST_NAMES` on purpose (blueprint D2).
              Self::ListDir { .. } => LIST_DIR,
  ```
  `name` is a `const fn`, and a `const` path is fine there (`PROMPT_PREVIEW` already is, `:888`).
- `try_serve`, after `StoreRequest::ApplyMigrations => …` (`:1637`), before the Qdrant or-arm
  (`:1638`). It goes **after** every or-arm on purpose: the "the twenty-two / twenty-five /
  twenty-nine above" counts in the comments at `:1567-1600` count the or-ed arms above them, and
  they stay true.
  ```rust
          // MOD-49 (plan P3): this box's filesystem under `spawn_blocking`; no store is read, so it
          // answers offline too, and a refusal is a `Constraint` that never drops the backend.
          StoreRequest::ListDir { path, show_hidden } => list_dir(path, *show_hidden).await?,
  ```
- New private fn, before `failed` (`:1753`):
  ```rust
  /// `ListDir` (MOD-49 P3): [`list_dirs`] off the async task, as `hierarchy::canonical` wraps
  /// `canonical_root`. A refusal becomes [`StoreError::Constraint`], so it reads
  /// ``list_dir: constraint violated: `/x` does not exist on this box``.
  async fn list_dir(path: &str, show_hidden: bool) -> StoreResult<StoreReply> {
      let typed = path.to_owned();
      let listing = tokio::task::spawn_blocking(move || {
          list_dirs(std::path::Path::new(&typed), show_hidden, LIST_CAP)
      })
      .await
      .map_err(|err| StoreError::Backend(err.to_string()))?
      .map_err(|refusal| StoreError::Constraint(refusal.to_string()))?;
      Ok(StoreReply::DirListing(listing))
  }
  ```
- **No other exhaustive site.** The worker loop's `match` ends in `other => try_serve(..)`
  (`:2277`), and every other `StoreRequest`/`StoreReply` match in `crates/htui/src` and
  `crates/htui/tests` has a wildcard. Only `name()` and `try_serve` list every variant, which
  confirms the plan's claim. No test pins the variant count. The only name-list pins are the
  per-module `REQUEST_NAMES` tests, and `ListDir` belongs to none of them.
- **No routing or staleness registration.** `App::dispatch` keys the staleness index by
  `(origin, discriminant(&request))` (`app/state.rs:309-320`). A `ListDir` sent through the
  section's `ctx.request` is addressed `Origin::Tab(SettingsTab::ID)`, and
  `SettingsTab::on_reply` hands every reply to **every** section (`ui/tabs/settings/mod.rs:368-372`).
  A newer `ListDir` therefore supersedes an older one at the gate, and its `Failed` passes the same
  gate (`update.rs:279-283`).

### 3.4 Tests (first), store half of `tests/hierarchy.rs`, after `a_box_with_no_row_is_refused_before_the_path_is_read` (`:456-476`)

Imports: `use htui::store_worker::{LIST_CAP, LIST_DIR, StoreReply, StoreRequest, serve};` (`:17`)
and `use htui_core::root_path::DirListing;`. A local `#[track_caller] fn listing(reply:
StoreReply) -> DirListing` panics with `{other:?}`, as `tree` (`:43-48`) does.

| Test | Pins |
|---|---|
| `list_dir_lists_the_subdirectories_of_a_tempdir` | dirs `beta`, `alpha`, a file and `.hidden`, over `demo()`: `path` is the typed string, `entries` are `alpha, beta`, `more == 0`. With `show_hidden: true`, `.hidden` comes first (byte order) |
| `a_missing_listing_is_refused_by_name` | `Failed { request: "list_dir", message }`. The message contains the path as typed, `does not exist on this box` and `constraint violated` |
| `a_link_is_listed_without_its_target` (`#[cfg(unix)]`) | `is_link: true`, and `format!("{reply:?}")` doesn't contain `secret-target-name` |
| `a_listing_past_the_cap_reports_the_rest` | `LIST_CAP + 2` dirs (named `d0000`…) give `entries.len() == LIST_CAP`, `more == 2` |
| `a_listing_needs_no_store` | over `Backend::Offline { .. }` (built as at `:679-686`), a tempdir lists. It isn't `DATABASE_UNREACHABLE` |
| `list_dir_is_named_outside_the_hierarchy_thirteen` | `StoreRequest::ListDir { .. }.name() == LIST_DIR == "list_dir"`, and `!REQUEST_NAMES.contains(&LIST_DIR)` |

### 3.5 Commits (T2)

1. `test(mod-49): ListDir through the store worker` — §3.1–§3.3 with `list_dir`'s body
   `todo!("MOD-49 T2")`, plus §3.4. Only `ListDir` reaches it.
2. `feat(mod-49): ListDir served under spawn_blocking` — the body. Gate green.

---

## 4. T3: the `PathPicker` component (P2, P5–P9)

**Files**: `crates/htui/src/ui/path_picker.rs` (new), `crates/htui/src/ui/mod.rs`.

### 4.1 `ui/mod.rs`

`pub mod path_picker;` between `pub mod overlay;` and `pub mod tabs;` (`:7-8`). Then
`pub use path_picker::{PathPicker, PickerOutcome};` after the `text_field` re-export (`:14`).

### 4.2 Types

```rust
//! Module doc: a directory picker a section owns as a mode (MOD-49 P2). It holds no store handle
//! and no channel: listings arrive through `on_reply`, and requests leave as
//! `PickerOutcome::Request` for the owning section to send (`R-NF-3`). Navigation is lexical and
//! never canonicalises, so nothing on screen names a link's target (P5, `R-BOX-4`).

/// What one key did.
#[derive(Debug, Clone)]
pub enum PickerOutcome {
    /// Handled here (moved, typed, swallowed): nothing for the section to do.
    None,
    /// A listing to send through `ctx.request` — a read, never marked busy (P11).
    Request(StoreRequest),
    /// The directory the user chose, as navigated (never canonicalised; the write does that).
    Chosen(String),
    /// `Esc` outside go-to: close the picker, write nothing.
    Cancelled,
}

/// A directory picker over this box's filesystem (MOD-49).
#[derive(Debug)]
pub struct PathPicker {
    /// The popup's title, e.g. "Root of `graphics` on this box".
    title: String,
    /// The directory the header shows and `S` chooses: the last path listed or refused (D10).
    path: String,
    /// The path the newest `ListDir` named; a listing of any other path is ignored (P9).
    asked: String,
    /// The last listing of `path`, or `None` while reading or after a refusal.
    listing: Option<DirListing>,
    /// The refusal of `asked`, shown in the popup (P7).
    error: Option<String>,
    /// Index into `listing.entries`.
    cursor: usize,
    /// Whether the newest request asked for `.`-entries (`.` toggles it, D1).
    show_hidden: bool,
    /// The one-line go-to field, while open (P6); its `Debug` never prints the text.
    goto: Option<TextField>,
    /// The child to highlight when the next listing lands — set by going up (D13).
    reselect: Option<String>,
}
```

`DirListing` doesn't derive `Default` and `TextField` isn't `PartialEq`, so `PathPicker` is
`Debug` only, and so is the section's `Mode`, which derives `Debug, Default` (`:173-174`).

### 4.3 Functions

```rust
/// The directory a picker opens on (P7, D9): `stored` when set, else `fallback` (a repo's
/// workspace root on this box), else `home`, else `/`. Empty strings count as unset.
#[must_use]
pub fn start_dir(stored: Option<&str>, fallback: Option<&str>, home: Option<&str>) -> String;

impl PathPicker {
    /// A picker on `start`, and the request that lists it: `ListDir { path: start, show_hidden:
    /// false }`. `path` is `start` from the first frame, so a refused start still has a header
    /// and `h` still goes up (P7).
    #[must_use]
    pub fn open(title: impl Into<String>, start: String) -> (Self, StoreRequest);
    /// The directory on screen.
    #[must_use]
    pub fn path(&self) -> &str;
    /// One key (P8, D11, D12, D15). The caller passes `CONTROL` chords on before calling this.
    pub fn on_key(&mut self, key: KeyEvent) -> PickerOutcome;
    /// A bracketed paste (P6, D12): into go-to when it is open, opening it otherwise.
    pub fn on_paste(&mut self, text: &str);
    /// A reply addressed to the section; `true` when it was this picker's (D10).
    pub fn on_reply(&mut self, reply: &StoreReply) -> bool;
    /// The popup, centred over `area` (D14).
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme);
}

/// `path/name`, lexically (D23).
fn child(path: &str, name: &str) -> String;     // Path::new(path).join(name), back to String
/// The lexical parent; `None` at `/`; an empty parent (a relative path) is `/` (D23).
fn parent(path: &str) -> Option<String>;        // Path::new(path).parent()
```

**Keys** (P8). Outside go-to:

| Key | Outcome |
|---|---|
| `j` / `Down`, `k` / `Up` | cursor ±1, clamped, no wrap (the section's rule, `:299-301`). `None` |
| `Enter` / `l` | with a landed listing and an entry: `ask(child(path, name))`. Otherwise `None` |
| `h` / `Backspace` | `parent(path)`. `Some(p)` sets `reselect` to the last component and returns `ask(p)`. At `/`, `None` |
| `s` | with a landed listing and an entry: `Chosen(child(path, name))`. Otherwise `None` (D11) |
| `S` | with a landed listing: `Chosen(path)`. Otherwise `None` (D11) |
| `/` | `goto = Some(TextField::with_text("/"))`. `None` |
| `.` | flip `show_hidden`, then `ask(path)` |
| `Esc` | `Cancelled` |
| any other | `None` (swallowed, H-4) |

`ask(p)` sets `asked = p` and returns `Request(ListDir { path: p, show_hidden })`.

**Keys inside go-to** go to the field first. `Submit`: take the trimmed text, close go-to, and
`ask(text)` if it isn't empty (`None` otherwise). `Cancel` closes go-to only. `Consumed` and `Pass`
return `None`.

**`on_paste`**: with go-to closed, it opens go-to holding the paste. With go-to holding exactly `/`
and an absolute paste (it starts with `/`), the paste replaces the field. Otherwise it goes to
`field.on_paste` (D12).

**`on_reply`**:
- `DirListing(l)` with `l.path == asked`: `path = asked`, `listing = Some(l)`, `error = None`, and
  `cursor` lands on the position of `reselect.take()` among the entries, else 0. Returns `true`.
- `DirListing` for another path: returns `false` and changes nothing (P9).
- `Failed { request: LIST_DIR, message }`: `path = asked`, `listing = None`, `error = Some(message)`,
  `cursor = 0`, `reselect = None`. Returns `true`.
- Anything else: `false`.

### 4.4 Render (D14)

`box_area = centered(area, area.width.saturating_sub(4).min(96), area.height.saturating_sub(2))`,
then `Clear`, then a `Paragraph` with `Block::new().borders(Borders::ALL).title(Span::styled(format!("
{} ", self.title), theme.title))`. Inner lines, top to bottom:
1. the header, `path`, in `theme.accent`, cut from the **left** with a leading `…` when wider than
   the inner width (the end of a path is the part that tells directories apart);
2. the entries window: `{CURSOR|NO_CURSOR}{name}/` for a directory and `{…}{name}@` for a link, the
   cursor row in `theme.accent`. `CURSOR = "> "` and `NO_CURSOR = "  "`, as in the switcher (`:24-27`);
3. `  +{more} more` (dim) when `more > 0`;
4. instead of 2–3: `  reading…` (dim) while `listing` and `error` are both `None`; `  no
   directories here` (dim, plus `  · . shows hidden` when `!show_hidden`) for an empty listing; the
   error message in `theme.error`;
5. while go-to is open: `go to: ` followed by `field.line(width, true, theme)`;
6. a blank line, then `HINT` (dim): `j/k move · Enter open · h up · s choose · S this dir · / go
   to · . hidden · Esc cancel`.

The window has `rows = inner_height − fixed lines`. Its offset is stateless,
`cursor.saturating_sub(rows − 1)`, so the cursor is always visible and `render` stays `&self`.

### 4.5 Tests (first), in-module `#[cfg(test)] mod tests`, and commits (T3)

Pure state, no filesystem. Listings are built by hand (`DirListing { path, entries, more }`), and
keys go through `KeyEvent::from(KeyCode::…)`.

| Test | Pins |
|---|---|
| `start_dir_prefers_stored_then_fallback_then_home_then_root` | the four rungs and empty-as-unset (P7) |
| `open_asks_for_the_start` | `ListDir { path: start, show_hidden: false }`; `path()` is `start` before any reply |
| `a_listing_for_another_path_is_ignored` | P9: `on_reply` is `false` and nothing renders from it |
| `j_and_k_clamp_at_both_ends` | no wrap |
| `enter_and_l_open_the_highlighted_directory` | `ListDir("/srv/htui")` from `/srv` + `htui` |
| `h_and_backspace_go_to_the_lexical_parent` | from `/srv/shared` (a link entry) to `ListDir("/srv")`: lexical, never the target (P5) |
| `h_at_the_root_stays_at_the_root` | `None` at `/` |
| `going_up_highlights_the_directory_you_came_from` | D13 |
| `s_chooses_the_highlighted_directory` | `Chosen("/srv/notes")` |
| `capital_s_chooses_the_listed_directory` | `Chosen("/srv")` |
| `s_and_capital_s_need_a_listing` | `None` while reading and after a refusal (D11) |
| `slash_opens_go_to_and_enter_lists_what_was_typed` | `/` then `srv/htui` then `Enter` gives `ListDir("/srv/htui")` |
| `a_paste_with_go_to_closed_opens_it` / `a_paste_into_a_lone_slash_replaces_it` | D12 |
| `esc_in_go_to_closes_only_go_to` | the next `Esc` is `Cancelled` |
| `dot_relists_with_hidden_shown` | `ListDir { path, show_hidden: true }` |
| `esc_cancels` | `Cancelled` |
| `a_refused_listing_shows_its_message_and_h_still_works` | `Failed { request: LIST_DIR, .. }`: the message renders, `h` asks for the parent |
| `unbound_keys_are_swallowed` | `q`, `?`, `1` give `None` (H-4) |
| `the_popup_marks_links_and_counts_the_rest` | `TestBackend` 100×30: `> htui/`, `shared@`, `+3 more` and the hint are in the buffer |
| `reading_and_empty_and_refused_are_three_texts` | the switcher's "reading vs empty" rule (`:115-121`), plus the error |

Commits:
1. `test(mod-49): PathPicker tests` — the module, `ui/mod.rs`, method bodies `todo!("MOD-49 T3")`
   (nothing in the product calls them yet), and the tests.
2. `feat(mod-49): PathPicker component` — the bodies. Gate green.

---

## 5. T4: Hierarchy `b` opens the picker (P1, P7, P8, P11)

**Files**: `crates/htui/src/ui/tabs/settings/hierarchy.rs`, `crates/htui/tests/hierarchy.rs`
(section half), `crates/htui/tests/snapshots/hierarchy__picker.snap` (new).

### 5.1 Types

- Module doc `:10-14`: "Three modes" becomes "Four modes …, or a directory picker (MOD-49)".
- Imports: `:31` becomes `use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};`. Add
  `use crate::ui::path_picker::{PathPicker, PickerOutcome, start_dir};`.
- After `HINT_EDITING` (`:56`): `const HINT_PICKING: &str = "choosing a directory \u{b7} Esc cancel";`.
- `EditorKind` (`:119-146`): delete `WorkspaceRoot(WorkspaceId)` (`:137-138`) and `RepoPath { .. }`
  (`:139-145`, with their docs).
- New, after `EditorKind`:
  ```rust
  /// Which row `b` writes a path for (MOD-49, blueprint D7): the two path writes, out of
  /// `EditorKind` now that no editor types them.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  enum PathTarget {
      /// `b` on the workspace row: this box's root.
      WorkspaceRoot(WorkspaceId),
      /// `b` on a repo row: this box's checkout.
      RepoPath { project: ProjectId, id: RepoId },
  }
  impl PathTarget {
      /// The unchanged write (plan summary): the worker's `canonical` guard decides what is stored.
      fn request(self, path: String) -> StoreRequest;  // the two arms moved from `submit` :919-930
  }
  ```
- `Mode` (`:174-191`): add
  ```rust
      /// `b`: choosing a directory (MOD-49). Stays open while the write is in flight (D8).
      Picking {
          /// The row the choice is written to.
          target: PathTarget,
          /// The popup.
          picker: PathPicker,
          /// The path sent, until the reply: what `written` compares the stored path with.
          chosen: Option<String>,
      },
  ```
- `HierarchySection` (`:214-237`): add `home: Option<String>` with the doc "`$HOME`, read once by
  `new` (D9); the picker's last fallback before `/`". `new()` (`:241-243`) becomes `Self { home:
  std::env::var("HOME").ok(), ..Self::default() }`. Add:
  ```rust
      /// The same section with `$HOME` fixed, so a test's start directory doesn't depend on the box
      /// running it (blueprint D9).
      #[must_use]
      pub fn with_home(mut self, home: Option<&str>) -> Self
  ```

### 5.2 `open_path` (`:561-603`) becomes the picker (P7)

The signature becomes `fn open_path(&mut self, row: Row, ctx: &Ctx<'_>)`, and its caller (the `b`
arm, `:1147-1153`) passes `ctx`.
- Workspace row: `target = WorkspaceRoot(ws.id)`, `stored = root_path`, `fallback = None`, title
  `` Root of `{slug}` on this box ``.
- Repo row: `target = RepoPath { project, id }`, `stored = local_path`,
  `fallback = snapshot.root_path`, title `` Checkout of `{repo name}` on this box ``.
- Project row: today's notice, unchanged.
- Then `start = start_dir(stored, fallback, self.home.as_deref())`,
  `(picker, request) = PathPicker::open(title, start)`, `self.notice = None`,
  `self.mode = Mode::Picking { target, picker, chosen: None }`, and `ctx.request(request)`.
  **Not `self.send`**: a listing marks nothing busy (P11).

### 5.3 Keys, paste, render

- `on_key` (`:1090-1096`): add `if matches!(self.mode, Mode::Picking { .. }) { return
  self.on_picker_key(key, ctx); }`.
- New `fn on_picker_key(&mut self, key: KeyEvent, ctx: &mut Ctx<'_>) -> Handled`, after
  `on_editor_key` (`:779-818`). A `CONTROL` chord returns `Handled::Pass` (H-4). Otherwise it calls
  `picker.on_key(key)`:
  - `None` → `Consumed`;
  - `Request(r)` → `ctx.request(r)`, then `Consumed`;
  - `Chosen(path)` → `if self.in_flight() { Consumed }`. Otherwise `request = target.request(path.
    clone())`, `chosen = Some(path)`, `notice = None`, then `self.send(request, ctx)` (a write: busy
    is set), then `Consumed`;
  - `Cancelled` → `mode = Browse`, `notice = None`, then `Consumed`.
- `captures_input` (`:1064-1066`): unchanged, because `!matches!(mode, Browse)` already covers
  `Picking` (P8).
- `on_paste` (`:1071-1088`): add `Mode::Picking { picker, .. } => { picker.on_paste(text);
  Handled::Consumed }`.
- `pane` (`:349-359`): add `Mode::Picking { .. } => Vec::new()`. The popup draws over the section,
  not in the pane.
- `hint_text` (`:391-416`): add `Mode::Picking { .. } => HINT_PICKING`.
- `deleting_slug` (`:761-766`): add `Mode::Picking { .. }` to the `None` arm.
- `render` (`:1298-1319`): after the hint line, `if let Mode::Picking { picker, .. } = &self.mode {
  picker.render(frame, area, ctx.theme); }`.

### 5.4 Replies

- `on_reply` (`:1192-1295`): a new arm **before** `:1270`:
  ```rust
              // MOD-49: a listing, or its refusal, for the open picker (blueprint D2, B-3). The
              // shell has already put a refusal on the status line too (D16).
              StoreReply::DirListing(_) | StoreReply::Failed { request: LIST_DIR, .. } => {
                  if let Mode::Picking { picker, .. } = &mut self.mode {
                      picker.on_reply(reply);
                  }
              }
  ```
- The `REQUEST_NAMES` arm (`:1277`): also `if let Mode::Picking { chosen, .. } = &mut self.mode {
  *chosen = None; }`. A refused path write leaves the picker open for another choice (D8, D21).
- `on_tree` (`:935-986`). The follow-up condition (`:941-951`) becomes `write ∈ {set_workspace_root,
  create_repo, update_repo}` **and** (`Mode::Editing` of `NewRepo`/`EditRepo` **or**
  `Mode::Picking { target: PathTarget::WorkspaceRoot(_), .. }`). `set_repo_path` stays unfollowed,
  as D136 decided. The reset (`:956`) becomes `matches!(self.mode, Mode::Editing(_) | Mode::Picking
  { .. })`.
- `written` (`:990-1010`) now reads `Mode::Picking { target, chosen: Some(chosen), .. }`. It looks
  up the stored path per target, as today, and returns `` stored as `{stored}` `` when
  `stored != *chosen`. `Mode::Editing` returns `None`, since no editor writes a path any more.
- `on_stale` (`:1013-1038`): add `Mode::Picking { .. } => Some(Reload::Keep)`. Path writes are
  upserts and never answer `HierarchyStale`, so this arm is unreachable and documented as such
  (D19).
- `reload` (`:1396-…`): drop `| EditorKind::WorkspaceRoot(id)` and `| EditorKind::RepoPath { id, .. }`
  from its patterns.
- `submit` (`:825-933`): delete the two arms at `:919-930`, which moved into `PathTarget::request`.

### 5.5 Dead after T4

`EditorKind::WorkspaceRoot`, `EditorKind::RepoPath`, the `Field::required("path", …)` editors in
`open_path`, and `submit`'s two path arms. `Field`, `Editor` and `repo_at` keep other callers.
`HINT_BROWSE` (`:45`) keeps `b path` (D17).

### 5.6 Tests (first), `tests/hierarchy.rs` section half

**Rewritten (B-2):**
- `b_on_the_workspace_row_sets_the_root` (`:1742-1763`) becomes
  **`b_on_the_workspace_row_picks_the_root`**: `HierarchySection::new().with_home(Some("/home/u"))`.
  `b` emits exactly `[Store(ListDir { path: "/home/u", show_hidden: false })]`. The reply
  `DirListing { path: "/home/u", entries: [srv] }` comes back, then `s` emits exactly
  `[Store(SetWorkspaceRoot { id: WORKSPACE_GRAPHICS, path: "/home/u/srv" })]`.
- `a_root_write_that_applied_is_followed_by_one_inference` (`:1971-2002`): same name. Steps: `b`, a
  synthetic listing of `/home/u`, `S` (giving `SetWorkspaceRoot("/home/u")`), then the
  `with_root(opened, "/home/u")` reply. Exactly one `InferRepoPaths`.

**New** (a `// ---- section: path picker (MOD-49 T4) ----` divider at the end of the file, `:2384`).
Two helpers: `fn dirs(path: &str, names: &[(&str, bool)], more: usize) -> StoreReply`, and
`fn with_repo_path(tree, repo: &str, path: &str) -> HierarchySnapshot` (a synthetic `RepoBoxPath`,
`box_id: ids::BOX`), next to `with_root` (`:1875`).

| Test | Bench | Pins |
|---|---|---|
| `the_picker_renders_over_the_section` | `SectionBench` | `with_root(tree, "/srv")` and `b` give `ListDir("/srv")`. Then `dirs("/srv", [htui, notes, shared@], 3)` and `j`. Snapshot **`picker`** (`render_section(&section, 100)`; H-5, D18) |
| `b_on_a_repo_row_starts_at_its_stored_path` | `SectionBench` | `ListDir(stored)` |
| `b_on_a_repo_row_starts_at_the_workspace_root` | `SectionBench` | no `local_path`, a root: `ListDir(root)` |
| `b_with_nothing_stored_starts_at_home` / `b_with_no_home_starts_at_the_root` | `SectionBench` | `with_home(Some(..))` gives `$HOME`, `with_home(None)` gives `/` |
| `b_on_a_project_row_keeps_its_notice` | `SectionBench` | no request, the notice `` `b` wants the workspace or a repo row `` |
| `navigating_then_s_writes_the_repo_path` | `SectionBench` | `l` gives `ListDir(root/core)`. A listing, then `s`, gives `SetRepoPath { project, repo, path: "<root>/core/src" }` |
| `a_chosen_path_stored_under_another_name_says_so` | `SectionBench` | the reply tree holds `/real/core`, so the notice is `` stored as `/real/core` `` and the mode is Browse |
| `esc_closes_the_picker_and_writes_nothing` | `SectionBench` | no write emitted, `captures_input()` is false afterwards |
| `a_listing_for_another_path_is_ignored` | `SectionBench` | P9: the frame doesn't contain the stray entry |
| `a_refused_listing_shows_in_the_picker_and_h_still_goes_up` | `SectionBench` | `Failed { request: "list_dir", message }`: the frame contains the message, and `h` gives `ListDir(parent)` |
| `a_paste_opens_go_to_with_the_pasted_path` | `SectionBench` | `bench.paste(section, "/srv/htui\n")` then `enter` gives `ListDir("/srv/htui")` |
| `a_listing_is_not_a_write_in_flight` | `SectionBench` | P11: `b`, `Esc`, then `i` emits `InferRepoPaths` (not refused) |
| `ctrl_c_passes_through_the_picker` / `the_picker_swallows_q` | `SectionBench` | `Handled::Pass` / `Handled::Consumed` (H-4) |
| `picking_in_the_shell_lists_the_root_and_keeps_h_and_l` | `Harness` (real worker) | a `MemStore::demo()` seeded with a root at a tempdir holding `alpha/` and `beta/` (`upsert_workspace_box_path`, `box_id: ids::BOX`). `b` shows `alpha/`. `l` opens `alpha`, and the frame isn't Agents (`!contains("transport")`, as `:1340`). `h` comes back. `S` stores the canonical tempdir path, and the tree shows it. `contains` only |
| `a_directory_gone_before_the_choice_is_refused_on_the_status_line` | `Harness` | B-6: list, remove `beta`, `j`, `s`. The frame contains `set_workspace_root: constraint violated` and `does not exist on this box`, and the popup is still open |

### 5.7 Commits (T4)

1. `test(mod-49): Hierarchy b opens the picker (red)` — the two rewrites and the new tests, plus the
   `home` field with `new()` and `with_home` (§5.1, live but behaviour-neutral). Red: `b` still
   opens the editor.
2. `feat(mod-49): Hierarchy b picks a directory` — §5.1–§5.5, plus the accepted
   `crates/htui/tests/snapshots/hierarchy__picker.snap`, reviewed in `cargo insta review`: popup
   border, title, header `/srv`, `> notes/`, `shared@`, `+3 more`, hint. Gate: the `hierarchy` suite,
   then `cargo test -p htui --all-features -- --test-threads=1`. Snapshots = 129, and
   `git status --porcelain crates/htui/tests/snapshots` shows exactly one `??`.

---

## 6. T5: close-out docs

`docs/decisions/mod/mod-49.md` (P1–P11 as decided, B-1..B-9, D1–D24, pins moved).
`DECISIONS.md` gets an index line in the existing style (`DECISIONS.md:5`). `README.md:299`: the
Hierarchy row's `` (`b`, or `i` to detect them) `` becomes "(`b` picks a directory, or `i` detects
them)". `HANDOFF.md`: tick the item at `:296` per `lifecycle.md` P2, write the re-counted pins at
`:61-63` (`StoreRequest` 96, `StoreReply` 55, snapshots 129; B-9), and take MOD-49 out of the
MOD-N list at `:616`. Commit: `docs(mod-49): close-out`. Run the plan's Validation block on the
real tree before this commit.

---

## 7. Decisions

| # | Decision | Why |
|---|---|---|
| **D1** | `StoreRequest::ListDir { path: String, show_hidden: bool }` | B-1. The worker filters before the cap, so `.` re-lists and the cap counts only what's shown |
| **D2** | `LIST_DIR`/`LIST_CAP` and a private `list_dir` live in `store_worker.rs`. The `try_serve` arm sits after `ApplyMigrations`. `ListDir` is outside `hierarchy::REQUEST_NAMES` and isn't served by `hierarchy::serve` | `hierarchy::serve` first demands a writer and reads `box_info` (`hierarchy.rs:222-225`), and a listing needs neither, so it works offline. Keeping it out of `REQUEST_NAMES` keeps the offline-refusal and name-stability tests true (B-3). T2's file set stays the plan's. Placing the arm after the or-arms keeps their "N above" prose true |
| **D3** | `RootRefusal::Unreadable(PathBuf)`, returned only by `list_dirs`, checked after `canonical_root`'s four in the same order | B-5. A picker that says "does not exist" about a directory listed one level up is wrong |
| **D4** | `DirEntry { name, is_link }` and `DirListing { path, entries, more }`, pub fields, `Debug, Clone, PartialEq, Eq`. `path` is the path as given. Unboxed in `StoreReply` | `StoreReply` needs `Debug + Clone` only. `PartialEq` serves the tests. The echoed path makes P9 a string compare and keeps `R-BOX-4` |
| **D5** | Collect, byte-sort, then truncate to `cap`; `more` is the remainder | Deterministic across filesystems; no silent truncation (P4) |
| **D6** | Classify by `entry.file_type()`, following only symlinks (`fs::metadata`). Non-UTF-8 names, `Err` entries, dangling links and links to files are dropped | P4's list. One `stat` per link only |
| **D7** | `PathTarget { WorkspaceRoot, RepoPath }` replaces the two `EditorKind`s, and `Mode::Picking { target, picker, chosen }` | B-4. No dead `submit` arms. `chosen` is what `written` compares |
| **D8** | The picker stays open while its write is in flight, and a refusal leaves it open. A `Hierarchy` reply closes it | The editor's rule ("stays open until the reply lands", `:820-824`): a refused path is re-chosen, not re-navigated |
| **D9** | `home` is read once in `new()`. `with_home` exists for tests. `start_dir` is a pure fn | B-7. Deterministic tests, and a one-line product read of `$HOME` (no disk I/O) |
| **D10** | `asked` is the newest requested path, and `path` is what's shown. A matching `DirListing` or a `list_dir` `Failed` sets `path = asked`. Navigation keys use `path` | P9. The shell gate already drops a superseded `ListDir` reply (same origin, same discriminant), so the check matters for benches and is cheap |
| **D11** | `s` and `S` need a landed listing. `Enter`/`l`/`s` on an empty listing do nothing | "Choose the **listed** directory" (P8). A refused path isn't offered as a choice |
| **D12** | `/` opens go-to holding `/`. A paste with go-to closed opens it with the paste. A paste over a lone `/` replaces it. `Esc` in go-to closes go-to only, and an empty submit closes it | Fixes the leading-slash trap, and pasting stays one keystroke (P6) |
| **D13** | Going up highlights the directory you came from | Back-and-forth navigation keeps its place; one `Option<String>` |
| **D14** | Fixed-size popup (`width−4` capped at 96, `height−2`), stateless scroll window, `name/` for a directory, `name@` for a link, `+N more`, three distinct empty texts, one hint line | B-8. `ls -F` marks. "Reading" vs "empty" is the switcher's rule |
| **D15** | The picker swallows every unbound key. The section passes `CONTROL` chords first | H-4. `q` can't quit mid-pick, `ctrl-c` still can |
| **D16** | A `list_dir` refusal shows in the popup **and** on the status line | H-3. P3 binds `Failed`. Exempting one name in `App::on_reply` would be a shell change for one view |
| **D17** | `HINT_BROWSE` keeps `b path`. Picking has its own section hint, and the popup carries the full key list | Changing it would move `hierarchy__demo`, `__inferred` and others for no gain. T4 then adds exactly one snapshot |
| **D18** | The one snapshot comes from synthetic replies on a `SectionBench`. Real-worker tests assert by `contains` | H-5 |
| **D19** | `on_stale` maps `Picking` to `Reload::Keep`, documented as unreachable | Path writes are upserts (`hierarchy.rs:355-371`) and never answer `HierarchyStale` |
| **D20** | Only `Picking { WorkspaceRoot }` is followed by an inference, as the root editor was. `set_repo_path` isn't | MOD-7 D136 unchanged |
| **D21** | A refused path write clears `busy` and `chosen`, and the popup stays | D8. A stale `chosen` would mislabel a later tree |
| **D22** | Titles: `` Root of `{slug}` on this box `` and `` Checkout of `{repo}` on this box `` | Says which row and which box (P1) |
| **D23** | `child` = `Path::join`, `parent` = `Path::parent`. `/` has no parent, and an empty parent is `/`. `.` and `..` aren't normalised | P5: lexical, and never a `canonicalize` |
| **D24** | Two commits per code task (red, then green), one docs commit | Memory: implementers commit after each step. H-6 keeps every red commit's `todo!()` off live paths |

## 8. Close-out gate (plan § Validation, on the real tree)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | grep -E "SIGABRT|FAILED|test result"
ls crates/htui/tests/snapshots | wc -l        # 129
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```
