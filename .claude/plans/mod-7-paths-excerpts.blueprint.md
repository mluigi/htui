# Blueprint: MOD-7 milestone 4, "paths and excerpts are real"

**Status**: **proposed** (2026-09-26). Plan deviations P-1 to P-12 (§0) and decisions D126–D144
(§9) are this blueprint's. Where a deviation says **Blocker**, the plan read literally either fails
its own gate or asserts something the tree cannot show. The Fix column is what the implementer
builds.

**Plan**: `.claude/plans/mod-7-paths-excerpts.plan.md` at `e4ca5b3`, status **confirmed** (the
maintainer adopted OQ-26..OQ-32 as defaulted), fact-checked (56 claims: 46 verified, 10 amended, 0
falsified). Its D104–D125, its "Verified claims" table and every "(amended at fact-check)" note are
binding and are not reopened here. PRD D0–D7 win over this blueprint where they disagree. Tasks are
cited as "milestone 4 T*n*" outside this file.

**Verified at**: HEAD `e4ca5b3`, branch `mod-7-m4`. `git diff --stat 9151eaa HEAD -- crates
Cargo.toml Cargo.lock` is empty, so the plan's line numbers (taken at `9151eaa`) still hold. Every
anchor below was located through Gortex (`search`, `read`, `relations`) and re-read at its line.
**Line numbers are pre-edit**: a citation into a file a task edits moves after that task's first
commit. `crates/htui-store/.sqlx/` holds **267** files, `crates/htui/tests/snapshots/` **87**,
migrations are `0001`..`0007`, and `df -h /` shows **85 GB free (81 % used)**. Check it before each
wave: two worktrees are two `target/` directories (project memory: `target/` fills the disk and the
dev Postgres crash-loops).

**Graphify**: `graphify-out/` does not exist in this checkout; nothing here comes from it.

**Coupling verdict.** The plan's waves stand unchanged. Wave 1 is T0 ∥ T1 ∥ T2, merged **T0, then
T1, then T2**. Wave 2 is T3 ∥ T4, merged one at a time. No file moves between tasks. Every file
this blueprint adds work to is already in the owning task's list: P-1 moves root resolution and the
pass orchestration into `crates/htui-agent/src/excerpt.rs` (T2's) and out of `engine.rs` and
`preview.rs`, and it moves nothing across a task boundary. T3 consumes T2's new public API, which is
the dependency the plan already records.

**Scope**:
- **Order**:
  1. Wave 1: T0 (worktree A) ∥ T1 (worktree B) ∥ T2 (worktree C).
  2. Merge T0, then T1, then T2. After **each** merge, re-run on the real tree the gates of every
     crate the merged task touched, with `--test-threads=1` (binding condition 2).
  3. Wave 2: T3 (worktree D) ∥ T4 (worktree E), both branched from the merged Wave 1.
  4. Merge T3 or T4 first (either), re-run `cargo test -p htui --all-features -- --test-threads=1`
     and `ls crates/htui/tests/snapshots | wc -l` on the merged tree; then the other, the same
     gate again (88 files at the end).
  5. The workspace gate (§8). The main thread then updates the `HANDOFF.md:39-42` pins (binding
     condition 3), then the optional live check (plan §Validation).
- **No migration**: the next is still `0008`.
- **New API**: one `WriteStore` method (T0); one `htui_orch::infer` module (T1); two pure
  `htui_core::prompt` functions and four `htui_agent::excerpt` items (T2); one private engine method
  (T2); one `FakeIsolator` override (T2); one `StoreRequest` and one `StoreReply` variant with three
  report types (T4).
- **Pins that move**: store `CASES` 76 → 77 (T0); `.sqlx` 267 → **268** (T0); `StoreRequest` /
  `StoreReply` 68 / 39 → 69 / 40 (T4); `hierarchy::REQUEST_NAMES` 12 → 13 (T4); snapshots 87 →
  **88** (T4 adds one; T3 and T4 update four). `htui-orch` `CASES` stay 72; `GraphSource` stays 7.

**House style (carried)**:
- `unsafe_code = "forbid"`, `missing_docs` warns, clippy `all` with `-D warnings` in the gate,
  pedantic **not** enabled. rustdoc denies broken **and private** intra-doc links: a `pub` item's
  doc must not link to a private fn (`unscanned`, `Engine::with_excerpts`), and a doc written before
  the target exists uses plain backticks.
- Nothing sets `updated_at` by hand on Postgres (the column default and the trigger do); `MemStore`
  stamps `now` as its twin.
- Implementers commit incrementally, staging their own paths only: never `-A`, never `stash`, never
  `--amend`. Every commit compiles. A red commit uses `todo!()` bodies **only where no existing path
  calls them** (H-6).
- Every gate is re-run with `--test-threads=1` on the real tree after the merge (the keyring fake is
  process-wide; project memory).
- No raw remote URL, credential or excerpt content in a log line, a note, a report or a `Debug` of a
  reply (R-51, H-7).

---

## 0. Plan deviations (found against the tree)

| # | Blocker? | Plan says | Tree at `e4ca5b3` | Fix |
|---|---|---|---|---|
| **P-1** | Non-blocker (drift) | D107: root resolution is "a private free function in `engine.rs`". D119: callers run `excerpt_pass` under `spawn_blocking`, and **each caller** applies the scrub filter. D120: the preview "runs the same pass". | Two callers would re-implement the placeholder test (D109), the residual (D118), the join and fail-open (D119), the scrub filter and `NoPath` recording, and the preview would re-implement D107 besides. MOD-2 D103's rule is that the preview's bytes and a run's cannot drift, and two copies of five steps is how they drift. `htui-agent` already depends on `tokio` with `rt` (`crates/htui-agent/Cargo.toml`), so `spawn_blocking` is available there. | D126, D127: T2 puts `excerpt_roots`, `touched_prefixes`, `PassInput`, `excerpt_pass` and one async `excerpts_for` in `crates/htui-agent/src/excerpt.rs` (T2's file). `Engine::with_excerpts` (D125) and `preview::build` (T3) each resolve their inputs and make **one** call. The plan's engine-side `excerpt_roots` unit tests move to `crates/htui-agent/tests/excerpt.rs` (T2's file). |
| **P-2** | Non-blocker (fail-closed parity) | D119: keep "only files whose `content` scrubs clean", note ``excerpt: `repo:path` dropped; the scrubber refused it (rule `…`)``. | `scrubbed_inputs` masks **four** strings of every excerpt, `repo`, `path`, `content` and `provider` (`crates/htui-core/src/prompt/mod.rs:764-772`), and fails `assemble` on residue in any of them. `notes()` (`:978-984`) copies notes into `trim_record.notes` **unscrubbed**, so a note that names a refused path persists the residue. | D129: the filter is one pure `htui_core::prompt::drop_unmaskable_excerpts` beside `scrubbed_inputs`. It checks the same four strings, and its note names `repo:path` only when both scrubbed clean (§4.1). |
| **P-3** | **Blocker** (T2's test cannot pass as named) | T2 test `a_bare_glob_without_a_primary_selects_nothing` "(D119's primary rule, in a project with no `is_primary` repo)". | (1) Tier 5 (lexical) takes up to `max_files / 3` leftovers whatever the item says (`rank`, `crates/htui-core/src/prompt/excerpt.rs:789-813`; `lexical`, `:830`), so a listed `src/lib.rs` **is** selected, at weight 10. (2) With no primary and only bare globs, `overlap::resolve` derives an **empty** scope (`crates/htui-orch/src/overlap.rs:55-60`, `:78-86`), so `StartRun` with `repo_scope: None` has no root at all. | D133: renamed `a_bare_glob_without_a_primary_matches_no_repo`. It starts with an explicit `repo_scope: Some([repo])` and asserts that no file carries `ExcerptReason::TouchedPath`. The prefix half is pinned purely in T2's `touched_prefixes_follow_the_overlap_primary_rule`. |
| **P-4** | Non-blocker | T2 test `a_fan_out_group_reads_repo_box_path` "if the harness can set a `repo_box_path` row for its box, otherwise covered by `excerpt_roots`". | Engine unit tests have no fan-out prologue: every fan-out case lives in `crates/htui-orch/src/conformance.rs` (`fan_out_three_with_a_judge_selects_one_winner`, `:3040`), whose `CASES` must not move (plan, "Not touched"). | D133: `a_step_without_trees_reads_repo_box_path_with_a_note` calls `with_excerpts` over a step id with no `run_step_tree` rows, which is exactly `drive_group`'s state at stage 3. It pins D108's rung and D108's note. |
| **P-5** | Non-blocker (secrecy, test cost) | D113: `Checkout { path, name, remotes }`, "every configured remote's fetch URL"; the cap is the constant `MAX_DIRS` 5 000; T1 tests "the directory cap setting `truncated`". | Raw fetch URLs keep `user:tok@` (fact-check claim 19, note c). Carrying them out of `infer.rs` puts a credential one `{:?}` away from a log. A cap test against the constant needs 5 001 directories. | D130: `Checkout.remote_keys` holds **normalised keys only** (D111 strips userinfo). `find_checkouts_with(root, Limits)` is `pub`, and `find_checkouts(root)` is `find_checkouts_with(root, Limits::DEFAULT)`. |
| **P-6** | Non-blocker | T1: "filesystem cases … over `tempfile` trees with `git init` and a written remote". | `htui_orch::isolate::git::testkit` builds real repositories "with `gix` alone so they need no `git` on the box (D40)" (`crates/htui-orch/src/isolate/git.rs:1903-1904`); `repo_with_one_commit(&Path)` is what `gix_isolator.rs::make_repo` uses. | D131: T1's `tests/infer.rs` uses `repo_with_one_commit` and appends a `[remote "…"]` section to `.git/config` by hand. No `git` binary, no `skip_without_git!`. |
| **P-7** | Non-blocker (stale prose) | T4 rewords "twelve" in `serve`'s doc (`hierarchy.rs:165-167`) and the `try_serve` comment (`store_worker.rs:1073`). | "Twelve" is also at `hierarchy.rs:1` (module doc), `:451` (`REQUEST_NAMES` doc), `store_worker.rs:297`, `:300` (enum comment), `:617` (`name`'s comment), `:1090` ("the same reason the twelve above are"), `tests/hierarchy.rs:88`, `:664`, `:687`. The cumulative "twenty-one above" (`store_worker.rs:1102`), "twenty-four above" (`:1113`) and "twenty-eight above" (`:1122`) each grow by one. | T4 rewords all of them (§6.8). |
| **P-8** | Non-blocker (design) | D114: the section sends `InferRepoPaths` "once after a `set_workspace_root`, `create_repo` or `update_repo` reply applies". | `p` is also an `update_repo` (`move_primary`, `settings/hierarchy.rs:604`) and changes no column inference reads. A follow-up on a workspace with no root on this box can only answer "no root". `on_tree` (`:928`) resets `Mode::Editing` to `Browse` before anything later in it could read the editor's kind. | D136: the follow-up fires only after an **editor** write of kind `WorkspaceRoot`, `NewRepo` or `EditRepo` that applied (`on_tree`, not `on_stale`), and only when the fresh tree has a root on this box. The kind is captured before the mode is reset. `i` always sends. |
| **P-9** | Non-blocker | T4's Postgres case asserts the written row. | `PgStore::box_info` reads `self.this_box` (`crates/htui-store/src/pg/read.rs:1196-1217`), and whether `testkit::demo_db` sets it to `ids::BOX` is not visible from this crate. | D144: the case reads the box through `backend.box_info()` and asserts the row's `box_id` equals it. |
| **P-10** | Non-blocker | T2: "`FakeIsolator` tree-root override for tests (`FAKE_TREE_ROOT`, `:47`, `:431`)". | Tree paths are `{FAKE_TREE_ROOT}/{run}/{step}/{repo}` (`crates/htui-orch/src/fake.rs:431`, `:445`). A test cannot write files under a run id and a step id it does not know yet. | D132: `FakeIsolator::root_trees_at(&self, dir: PathBuf)`. Once set, `cwd` is `dir` and every tree is `dir/<repo id>` for every step. The fixture writes `dir/<repo id>/src/lib.rs` before `StartRun`. |
| **P-11** | Non-blocker | D120: "`empty_excerpts` is deleted or kept only for the no-repo case"; the unit test at `preview.rs:479-488` "is rewritten … or deleted with it". | `excerpts_for` over no roots returns exactly `empty_excerpts`' value: `select` with no root lists nothing, writes no note and reconciles no cap, because `FsRepoReader::new(caps)` reports the same `max_file_bytes`. | D139: T3 deletes `empty_excerpts` and its unit test. T2's `excerpts_for_with_no_roots_is_the_empty_audit` pins the value, and `prompt_preview.rs`'s rewritten `the_preview_assembles_from_real_store_reads` pins it through `build`. |
| **P-12** | Non-blocker (precision) | D117: "`inferred N of M · K already set · no checkout: a, b · ambiguous: c (2) — b on a repo sets it by hand`". | `M` is not defined, a workspace with no repo is not covered, and 40 unmatched names would push the line past any pane. | D137: `M` counts every repo that had no row before the pass. The empty workspace has its own sentence, and each name list shows at most three names, then `+N more` (§6.5). |

### 0a. Hazards, each with its guard

| # | Hazard | Guard (who, how it fails loudly) |
|---|---|---|
| **H-1** | **Binding condition 1: T2 adds no `WriteStore` implementor.** A test-only wrapper in `engine.rs` would stop compiling once T0's method (no default body) merges. | T2's tests use `MemStore` and the existing `upsert_repo_box_path` / `create_repo` / `update_item`. The reviewer runs `rg -n 'impl.*WriteStore for' crates/htui-orch crates/htui` and expects nothing. The post-merge `cargo check --workspace --all-features --all-targets` after T2 catches a violation by name. |
| **H-2** | **Binding condition 2: merge order T0, T1, T2, then T3/T4 one at a time**, with every touched crate's gate re-run on the merged tree. T1 and T2 compile against the base `htui-core`, T4 needs T0 and T1, and T3 needs T2. | §1's table and §8. A lane never merges its own branch; the main thread merges and runs the gates. T3 and T4 branch from the merged Wave 1, not from `e4ca5b3`. |
| **H-3** | **Binding condition 3: the main thread owns the `HANDOFF.md:39-42` pins** (store `CASES` 77, `.sqlx` 268, `StoreRequest`/`StoreReply` 69/40, snapshots 88) and every doc. No script or test checks them. | No implementer stages `HANDOFF.md`, `docs/**` or the PRD. The reviewer checks `git diff --stat` of each lane for them. |
| **H-4** | `store::conformance::every_cross_referenced_test_name_exists` (`crates/htui-core/src/store/conformance.rs:10492`) scans **every backticked span** of the file. A bare snake_case span with four or more underscores must be a fn in `conformance.rs` or `mem.rs`, and a `<file>.rs::<name>` span whose file is not `conformance`, `mem`, `pg_criteria` or `box_identity` **panics**. | T0's case doc and messages backtick only `infer_repo_box_path`, `upsert_repo_box_path` and `repo_box_paths` (three underscores each) and the case's own name (defined in the file). Other crates' halves are named in prose ("T4's Postgres serve case"), never as `hierarchy_pg.rs::…`. The `htui-core` gate runs the scanner. |
| **H-5** | `.sqlx` regeneration needs a scratch database migrated through `0007`; the compose `htui` database is empty (project memory). | §2.6's recipe on a freshly recreated `htui_prepare_mod7m4`, then `prepare --check`. `git status --porcelain crates/htui-store/.sqlx` shows **exactly one `??` line and no `M` or `D`**, and `ls crates/htui-store/.sqlx \| wc -l` prints **268**. |
| **H-6** | A red commit that wires a `todo!()` into an existing path turns every existing walk red for the wrong reason, and the lane then cannot tell its own red from breakage. | T2's red engine commit defines `with_excerpts` with a `todo!()` body but **does not call it** from `assemble_prompt` (§4.8). T4's red commit routes `InferRepoPaths` to an `infer` whose body is `todo!()`: only the new request reaches it. T0's `todo!()` bodies are new trait methods nothing else calls. |
| **H-7** | A remote URL carries `user:tok@` (claim 19c). `StoreRequest`/`StoreReply` derive `Debug`, and the tests print replies with `{reply:?}` (`store_worker.rs:87-93`). | `Checkout` holds normalised keys only (D130), and `normalise_remote` drops userinfo (D111). `InferReport` carries repo names, canonical paths and outcomes, never a URL. T1 pins `normalise_remote_never_keeps_credentials` and `a_remote_with_a_token_leaves_only_its_key`. The reviewer greps `infer.rs` and `hierarchy.rs` for `tracing::` and `remote_url` in any format string. |
| **H-8** | **D125: `phase_spec` must not change.** It is also the promote/handoff builder (`engine.rs:1270`, `strict = false`), and `promote::handoff_spec` keeps `excerpts` through `..phase` (`crates/htui-orch/src/promote.rs:81-106`). | `phase_spec` keeps `excerpts: no_excerpts(caps)`; only its comment at `:5028-5030` changes. T2's `a_handoff_spec_carries_no_excerpts_and_runs_no_pass` pins it through `PromoteStep`. `git diff crates/htui-orch/src/promote.rs` is empty after T2 (§8). |
| **H-9** | R-49: `spawn_blocking` in engine tests under `start_paused = true`. | D128: `excerpts_for` calls `spawn_blocking` only when a root is readable (not `NoPath`). Otherwise the pass runs inline and does no I/O and spawns no thread: the only provider is `BuiltinRanker`, whose `propose` returns nothing (`excerpt.rs:619-634`), and `run_providers` runs `providers[0]` on the caller's thread. T2's gate runs the whole `htui-orch` and `htui` suites. |
| **H-10** | A snapshot of a tempdir path is not reproducible: the repo row prints `local_path` and the workspace row prints the root, so the path length moves the frame. | D138: `hierarchy__inferred` is rendered with `SectionBench::render_section(&section, 100)` (`crates/htui/src/testkit.rs:655`) from a **synthetic** `RepoPathsInferred` reply whose paths are fixed strings (`/srv/graphics/core`). The section never stats a path, so no filesystem is involved. |
| **H-11** | Runtime coupling after T2: every walk in a suite outside T2's list whose scope has a repo now runs the pass over `FakeIsolator` trees that do not exist. That is `run_worker.rs`, `chat.rs`, `runs_pg.rs` and the orch conformance suite. | Each such step gains one `excerpt: repo `…` could not be listed: read_dir: …` note (`excerpt.rs:1101-1113`; the `io::Error` display names no path). No snapshot renders it, and no test compares those notes by equality (claim 43). T2's gate runs `cargo test -p htui --all-features -- --test-threads=1`. |
| **H-12** | Worktree implementers (project memory): Gortex `edit` writes into the **primary** checkout, not the worktree. | Implementers in a worktree edit files by their worktree path, create their named branch first, remove the worktree before `git branch -d`, and budget about 10 GB of `target/` per worktree. |

---

## 1. Build order and validation, at a glance

| Task | Crate(s) | Commits (each compiles) | Gate (all `--test-threads=1`) |
|---|---|---|---|
| T0 writer | htui-core, htui-store, htui-agent (worktree A) | 3 (§2.7) | `htui-core`; `cargo check --workspace --all-features --all-targets`; Postgres `pg_conformance`; `htui-agent`; prepare + `--check`; `.sqlx` = 268; clippy on the three |
| T1 discovery | htui-orch (worktree B) | 3 (§3.6) | `htui-orch` (lib + `--test infer`); clippy `-p htui-orch` |
| T2 excerpt pass | htui-core, htui-agent, htui-orch (worktree C) | 6 (§4.8) | `htui-core`; `htui-agent`; `htui-orch`; `htui` with the Postgres suites; clippy on the three |
| merge | — | **T0, then T1, then T2** | after **each**: the gates of the crates it touched, on the real tree; after T2 also `cargo clippy -p htui-orch --all-features --all-targets -- -D warnings` |
| T3 preview | htui (worktree D) | 2 (§5.5) | `htui`; clippy `-p htui`; snapshot count 87 in-lane |
| T4 inference served | htui (worktree E) | 3 (§6.9) | `htui`; `hierarchy_pg` must **run**; clippy `-p htui`; snapshot count 88 in-lane |
| merge | — | T3 and T4, **one at a time** | after each: `cargo test -p htui --all-features -- --test-threads=1`; snapshots 88 after both; then §8 |

---

## 2. T0: the insert-if-absent writer (D104, D105)

**First failing test**: `store::conformance` case `infer_repo_box_path_inserts_only_where_absent`
(through `tests/mem_store.rs::mem_store_conformance`).

**Files** (the plan's list, unchanged): `crates/htui-core/src/store/traits.rs`,
`crates/htui-core/src/store/mem.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui-core/tests/mem_store.rs`, `crates/htui-store/src/pg/write.rs`,
`crates/htui-store/src/writer.rs`, `crates/htui-store/tests/pg_conformance.rs`,
`crates/htui-store/.sqlx/`, `crates/htui-agent/src/conformance.rs`,
`crates/htui-agent/tests/recorder.rs`.

### 2.1 `traits.rs`: the contract (`WriteStore`, between `upsert_repo_box_path` `:604` and `repo_box_paths` `:610`)

```rust
    /// Inserts this box's checkout path for a repo **only where none exists** (MOD-7 milestone 4,
    /// D104): `Ok(true)` when the row was written, `Ok(false)` when a row for
    /// `(repo_id, box_id)` already existed, which is then left exactly as it was.
    ///
    /// The compare-and-set path inference needs, keyed on the row's **absence**, which no
    /// reconnect changes (the PRD's box-writer constraint). A manual
    /// [`upsert_repo_box_path`](Self::upsert_repo_box_path) racing it either lands first, so this
    /// answers `false`, or overwrites the inferred row, so the manual path wins. `updated_at` is the
    /// store's: the value passed in is ignored on both backends.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) when either id names no
    /// row, and nothing is written.
    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool>;
```

No default body: the trait has none (claim 10). All five implementors owe the method in commit (a).

### 2.2 `MemStore` (`mem.rs`)

- `State`, directly after `upsert_repo_box_path` (`:2292-2316`):

```rust
    /// `WriteStore::infer_repo_box_path`'s twin (MOD-7 milestone 4, D104): the same two existence
    /// checks as `upsert_repo_box_path`, in the same order and with the same sentences, then a push
    /// only when no row holds `(repo_id, box_id)`. Answers whether it pushed.
    fn infer_repo_box_path(&mut self, path: &RepoBoxPath, now: DateTime<Utc>) -> Result<bool> {
        // the repo check, then the box check: copied from `upsert_repo_box_path`
        if self
            .repo_box_paths
            .iter()
            .any(|held| held.repo_id == path.repo_id && held.box_id == path.box_id)
        {
            return Ok(false);
        }
        let mut row = path.clone();
        row.updated_at = now;
        self.repo_box_paths.push(row);
        Ok(true)
    }
```

  Factor the two checks into a private `fn check_repo_box_path_ids(&self, path: &RepoBoxPath) ->
  Result<()>` used by both writers, so the two sentences cannot drift. That is optional; if it is
  done, `upsert_repo_box_path`'s behaviour must not change (the existing case
  `repo_round_trip_and_primary_flag` pins it).
- `impl WriteStore for MemStore`, after `upsert_repo_box_path` (`:5437-5440`):
  `async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool> { let now = Utc::now();
  self.write(|state| state.infer_repo_box_path(path, now)) }`.

### 2.3 `PgStore` (`pg/write.rs`, after `upsert_repo_box_path` `:1904-1921`)

```rust
    /// One row per `(repo_id, box_id)`, inserted only where none exists (MOD-7 milestone 4, D104).
    /// The conflict clause decides it atomically, so a concurrent manual upsert either lands first
    /// (this answers `false`) or replaces what this wrote.
    ///
    /// # Errors
    ///
    /// [`StoreError::Constraint`] when either id names no row (`23503`). A row that already holds
    /// the pair implies both ids exist, so a conflict never hides a foreign-key refusal.
    async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool> {
        let done = sqlx::query!(
            "INSERT INTO repo_box_path (repo_id, box_id, local_path) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (repo_id, box_id) DO NOTHING",
            path.repo_id.as_uuid(),
            path.box_id.as_uuid(),
            path.local_path,
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(done.rows_affected() == 1)
    }
```

`updated_at DEFAULT now()` (`0001_init.sql:206`) stamps the row. **`.sqlx` +1.**

### 2.4 The three forwards

- `Writer` (`crates/htui-store/src/writer.rs`, after `:599-604`): `match self { Self::Memory(store)
  => store.infer_repo_box_path(path).await, Self::Online(pg) => pg.infer_repo_box_path(path).await
  }`.
- `UsageSpy` (`crates/htui-agent/src/conformance.rs`, after `:882-884`):
  `async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> StoreResult<bool> {
  self.inner.infer_repo_box_path(path).await }`.
- `SpyStore` (`crates/htui-agent/tests/recorder.rs`, after `:577-579`): the same body.

### 2.5 The conformance case (`store/conformance.rs`; `CASES` 76 → 77)

`CASES` entry appended after `"claim_run_checks_tags_after_claimability_and_before_the_slot",`
(`:117`). `run_case` arm before `other =>` (`:251`). The fn goes directly after
`repo_round_trip_and_primary_flag` (ends `:2897`), beside the other `repo_box_path` writer.
`RepoBoxPath`, `RepoId`, `BoxId`, `Utc` and `new_repo` are already in scope in that fn's
neighbourhood; add any missing import to the file's `use` block.

```rust
/// MOD-7 milestone 4 (plan D104, D105): `infer_repo_box_path`, the insert-if-absent writer path
/// inference uses. An empty pair is written and answers `true`; a second call answers `false` and
/// changes nothing; a row `upsert_repo_box_path` wrote is never replaced, and the manual writer
/// still replaces an inferred row; an unknown repo or box is `Constraint` and writes nothing.
async fn infer_repo_box_path_inserts_only_where_absent<S: WriteStore>(store: &S)
```

Body, in order (`CASE` constant, `.expect(CASE)` on every `Ok`, messages prefixed `{CASE}:`):

| Step | Asserts |
|---|---|
| setup | `core = create_repo(new_repo(ids::PROJECT_HTUI, "core", true))`, `docs = create_repo(new_repo(ids::PROJECT_HTUI, "docs", false))`. A local closure `row(repo, local)` builds `RepoBoxPath { repo_id, box_id: ids::BOX, local_path, updated_at: Utc::now() }`, and `paths(repo)` maps `repo_box_paths(repo)` to `(box_id, local_path)` pairs. |
| 1 | `infer_repo_box_path(&row(core.id, "/src/inferred"))` is `true`; `paths(core.id) == [(ids::BOX, "/src/inferred")]`. |
| 2 | `infer_repo_box_path(&row(core.id, "/src/other"))` is `false`; `paths(core.id)` unchanged: "a second inference never replaces a row". |
| 3 | `upsert_repo_box_path(&row(docs.id, "/src/manual"))`, then `infer_repo_box_path(&row(docs.id, "/src/guess"))` is `false`, and `paths(docs.id) == [(ids::BOX, "/src/manual")]`: "the manual row stands (PRD: a manual row is never replaced by inference)". |
| 4 | `upsert_repo_box_path(&row(core.id, "/src/by-hand"))`, then `paths(core.id) == [(ids::BOX, "/src/by-hand")]`: "the manual writer replaces an inferred row". |
| 5 | `infer_repo_box_path(&RepoBoxPath { repo_id: RepoId::new(), .. })` is `Err(StoreError::Constraint(_))`. |
| 6 | `infer_repo_box_path(&RepoBoxPath { repo_id: docs.id, box_id: BoxId::new(), .. })` is `Err(StoreError::Constraint(_))`, and `paths(docs.id)` is still the single manual row. |

H-4: the doc and every message backtick only three-underscore names and this case's own name.

**Pins**: `crates/htui-core/tests/mem_store.rs:37` becomes `77`. The message's tail "…, and MOD-7
milestone 3's two for the claim-time capability check (plan D80, D81)" becomes "…, MOD-7 milestone
3's two for the claim-time capability check (plan D80, D81), and MOD-7 milestone 4's one for the
path-inference writer (plan D104, D105)". `crates/htui-store/tests/pg_conformance.rs:19` becomes
`const EXPECTED_CASES: usize = 77;`. `READ_CASES` stays 14.

### 2.6 `.sqlx` (+1, 267 → 268)

```bash
df -h /                                      # 85 GB free at e4ca5b3
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod7m4
docker compose exec -T postgres createdb -U postgres htui_prepare_mod7m4
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m4 \
  sqlx migrate run --source crates/htui-store/migrations            # through 0007
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m4 \
  cargo sqlx prepare -- --all-targets --all-features)
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m4 \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l           # 268
git status --porcelain crates/htui-store/.sqlx   # exactly one `??` line; no `M`, no `D` (H-5)
```

The service is `postgres` (container `htui-postgres`, host port 5439). The test DSN (`…/postgres`)
and the prepare DSN (`…/htui_prepare_mod7m4`) are different databases (project memory).

### 2.7 Commits (T0) and gate

1. **(a) red**: the trait method and its doc; `MemStore::infer_repo_box_path` and
   `PgStore::infer_repo_box_path` with `todo!()` bodies (the Pg one names no `query!` yet, so it
   compiles offline); the three forwards (delegation, green on arrival); the case, its `CASES` entry
   and `run_case` arm; both pins and the message clause. Red: the case panics at `todo!()` on
   `MemStore`.
2. **(b) green, `htui-core`**: `State::infer_repo_box_path` (and the optional shared check) and the
   `MemStore` body.
3. **(c) green, `htui-store`**: `PgStore::infer_repo_box_path` and its doc; `.sqlx` (§2.6).

```bash
cargo fmt --all -- --check
cargo test -p htui-core --all-features -- --test-threads=1
cargo check --workspace --all-features --all-targets                 # all five implementors, every target
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1          # UsageSpy and SpyStore
cargo clippy -p htui-core -p htui-store -p htui-agent --all-features --all-targets -- -D warnings
# §2.6: prepare --check, 268 files, one `??` only
```

---

## 3. T1: discovery and matching (D111, D112, D113, D123)

**First failing test**: `infer::tests::normalise_remote_folds_the_four_spellings_of_one_repo`. T1
builds on the **base** `htui-core` and names nothing T0 or T2 adds.

**Files** (the plan's list, unchanged): `crates/htui-orch/src/infer.rs` (new),
`crates/htui-orch/src/lib.rs`, `crates/htui-orch/tests/infer.rs` (new).

### 3.1 `lib.rs`

`pub mod infer;` between `pub mod gate;` and `pub mod isolate;` (alphabetical, `lib.rs:28-35`). The
module doc paragraph gains one sentence: "MOD-7 milestone 4 adds [`infer`], repo-path inference's
pure half: remote-URL normalisation, a bounded checkout walk and the remote-first, name-second
choice (plan D111–D113)." No crate-root re-export: callers name `htui_orch::infer::…`.

### 3.2 `infer.rs`: module doc and types

Module doc (intent): the pure half of PRD D5. Nothing here writes, and nothing here touches a store:
`crate::hierarchy` in `crates/htui` serves the request, walks under `spawn_blocking` and writes. The
walk never follows a link, so every candidate under a canonical root is canonical (F-102 by
construction). A truncated scan infers nothing, because an unseen second clone would turn a wrong
single match into a write. Remote URLs never leave this module: `Checkout` carries normalised keys,
which hold no userinfo.

```rust
/// How deep below the workspace root a checkout is looked for; the root itself is depth 0 and may
/// be a checkout (plan D113).
pub const MAX_DEPTH: usize = 3;
/// How many directories one scan examines before it stops and reports `truncated` (plan D113).
pub const MAX_DIRS: usize = 5_000;

/// The walk's two bounds. `find_checkouts` uses `Limits::DEFAULT`; tests pass smaller ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Deepest level descended to, the root being 0.
    pub max_depth: usize,
    /// Directories examined before the scan stops, the root included.
    pub max_dirs: usize,
}

impl Limits {
    /// `MAX_DEPTH` and `MAX_DIRS`.
    pub const DEFAULT: Self = Self { max_depth: MAX_DEPTH, max_dirs: MAX_DIRS };
}

/// One git checkout found under the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The directory holding `.git`, under the canonical root, reached without following a link.
    pub path: PathBuf,
    /// Its file name, which the name rung compares with `repo.name` byte for byte.
    pub name: String,
    /// Every configured remote's fetch URL, **normalised** (`normalise_remote`), sorted and
    /// deduplicated. Never a raw URL: a fetch URL can carry `user:token@`. Empty when the checkout
    /// has no remote, or `gix` could not open it.
    pub remote_keys: Vec<String>,
}

/// What one scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    /// Every checkout, in depth-first order with each level sorted by file-name bytes.
    pub checkouts: Vec<Checkout>,
    /// The scan stopped at `Limits::max_dirs`. The caller must infer nothing (plan D113).
    pub truncated: bool,
}

/// Which rung chose a checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    /// A remote's normalised key equals the repo's.
    Remote,
    /// The directory name equals `repo.name`, and no remote contradicts it (plan OQ-31).
    Name,
}

impl MatchedBy {
    /// `remote` or `name`, the word the Hierarchy section's notice uses.
    #[must_use]
    pub const fn as_str(self) -> &'static str { /* "remote" | "name" */ }
}

/// `choose`'s answer for one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Exactly one candidate at the first rung that had any.
    Inferred {
        /// The checkout's path, not yet re-canonicalised.
        path: PathBuf,
        /// Which rung.
        by: MatchedBy,
    },
    /// Two or more candidates at the deciding rung: nothing is written (PRD D5).
    Ambiguous {
        /// How many.
        candidates: usize,
    },
    /// No candidate at either rung.
    NoMatch,
}
```

### 3.3 `normalise_remote` (D111)

```rust
/// The comparison key of a remote URL (plan D111), or `None` for an empty or unparseable one.
///
/// `https://github.com/o/r.git`, `git@github.com:o/r`, `ssh://git@github.com:22/o/r/` and
/// `https://user:tok@GitHub.com/o/r` are all `github.com/o/r`. The host is lowercased and the path
/// is kept byte-exact, so a case mismatch only falls through to the name rung, which is the safe
/// direction. The key never carries userinfo: `gix` keeps `user:tok@` in a fetch URL, so this
/// strips it itself.
#[must_use]
pub fn normalise_remote(url: &str) -> Option<String>
```

Algorithm, in order:
1. `s = url.trim()`; empty → `None`. Trim every trailing `/`, strip one trailing `.git`, then trim
   trailing `/` again.
2. **Scheme form** (contains `://`): the scheme is lowercased.
   - `https`, `http`, `ssh`, `git`, `git+ssh` or `ssh+git`: the authority is the text up to the
     first `/`. Drop everything up to and including the **last** `@` (userinfo). Drop `:port` when
     the text after the last `:` is all ASCII digits. Lowercase the host. The path is the rest with
     repeated `/` collapsed and the leading `/` removed. `host/path`; `None` if either is empty.
   - `file`: `file:` + the path, repeated `/` collapsed.
   - Any other scheme: `None`.
3. **Local path**: a leading `/`, or a Windows drive (`^[A-Za-z]:[\\/]`, backslashes turned to
   `/`), gives `file:` + the collapsed path.
4. **scp-like** (`[user@]host:path`): a `:` with no `/` before it. The host is the text before the
   `:` minus any `user@`, lowercased. The path is the text after it, with a leading `/` removed and
   repeated `/` collapsed. `None` if either is empty.
5. Otherwise (a relative path, a bare word) → `None`.

### 3.4 `find_checkouts` / `find_checkouts_with` (D113)

```rust
/// `find_checkouts_with(root, Limits::DEFAULT)`. Synchronous `std::fs` plus one `gix::open` per
/// checkout: call it under `tokio::task::spawn_blocking` (plan D113, R-45).
#[must_use]
pub fn find_checkouts(root: &Path) -> Scan

/// The checkouts under `root`, depth-first, each level sorted by file-name bytes (the
/// `FsRepoReader` walk's determinism rule).
///
/// A directory holding a `.git` entry (a directory, or a file beginning `gitdir:`) is a checkout
/// and is **not** descended into, so a nested checkout is never listed. Entries whose name starts
/// with `.` are skipped. Links are never followed (`symlink_metadata`). An unreadable directory
/// contributes nothing, and a name that is not UTF-8 is skipped. Every directory examined, the root
/// included, counts towards `limits.max_dirs`, and the first one past it stops the scan with
/// `truncated`.
#[must_use]
pub fn find_checkouts_with(root: &Path, limits: Limits) -> Scan
```

Private helpers: `fn visit(dir: &Path, name: &str, depth: usize, limits: Limits, examined: &mut
usize, scan: &mut Scan) -> bool` (`true` = stop); `fn is_checkout(dir: &Path) -> bool`; `fn
remote_keys(dir: &Path) -> Vec<String>`. The root's `name` is its own `file_name()` (UTF-8, else
skip it as a candidate but still walk it).

`remote_keys` uses the fact-check's `gix` 0.87.1 notes (claim 19):

```rust
fn remote_keys(dir: &Path) -> Vec<String> {
    let Ok(repo) = gix::open(dir) else {
        return Vec::new(); // an unopenable checkout has no remotes (plan D113)
    };
    let mut keys: Vec<String> = repo
        .remote_names()
        .iter()
        .filter_map(|name| {
            let name: &gix::bstr::BStr = name.as_ref(); // (a) E0283 without the annotation
            let remote = repo.find_remote(name).ok()?;
            let url = remote.url(gix::remote::Direction::Fetch)?;
            normalise_remote(&url.to_bstring().to_string()) // (c) keeps user:tok@; the key drops it
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}
```

(b) an unborn `HEAD` opens. Adjust `remote_names()`'s iteration to what 0.87.1 returns; the probe
compiled this shape. No new `gix` feature.

### 3.5 `choose` (D112, OQ-31)

```rust
/// Remote first, name second, ambiguity is failure (PRD D5, plan D112), for one repo with no row on
/// this box.
///
/// Paths in `held` (other repos' rows on this box) are excluded before either rung. Rung 1, when
/// `repo.remote_url` normalises to a key: the candidates with **any** remote key equal to it. One
/// is `Inferred { by: Remote }`, several are `Ambiguous`, and none falls to rung 2. Rung 2: the
/// candidates whose name equals `repo.name` byte for byte. When the repo has a key, only candidates
/// with **no** remote are accepted: a same-named checkout whose remotes all normalise elsewhere is
/// a fork or an unrelated project (OQ-31). One is `Inferred { by: Name }`, several `Ambiguous`,
/// none `NoMatch`. A `remote_url` that does not normalise counts as none.
#[must_use]
pub fn choose(repo: &Repo, checkouts: &[Checkout], held: &BTreeSet<PathBuf>) -> Choice
```

`Repo` is `htui_core::model::Repo`.

### 3.6 Tests (first) and commits (T1)

**Unit tests** (`infer.rs` `mod tests`; `choose` cases build `Checkout` values by hand):

| Test | Asserts |
|---|---|
| `normalise_remote_folds_the_four_spellings_of_one_repo` | D111's four spellings → `Some("github.com/o/r")`; also `http://github.com/o/r`, `git://github.com/o/r.git`, `git+ssh://git@github.com/o/r`, `https://github.com//o//r/`. |
| `normalise_remote_keeps_other_hosts_and_paths_apart` | `gitlab.com/o/r`, `github.com/o/other` and `github.com/O/R` all differ from `github.com/o/r`. |
| `normalise_remote_refuses_empty_and_unparseable` | `""`, `"   "`, `"relative/dir"`, `"ftp://h/o/r"`, `"https://github.com"` (no path) → `None`. |
| `normalise_remote_never_keeps_credentials` | For `https://user:tok@github.com/o/r` and `ssh://me:pw@host:2222/o/r`, the key contains neither `user`, `tok`, `me`, `pw` nor `@`. |
| `normalise_remote_maps_local_paths_to_file_keys` | `/srv/git/r.git`, `file:///srv/git/r` → `file:/srv/git/r`; `C:\git\r` → `file:C:/git/r`. |
| `choose_takes_the_one_remote_match` | Repo with `remote_url` `https://github.com/o/r`; candidates `a` (key `github.com/o/r`) and `b` (other key) → `Inferred { path: a, by: Remote }`. |
| `choose_refuses_two_remote_matches_without_falling_back_to_the_name` | Two candidates with the key, one also named like the repo → `Ambiguous { candidates: 2 }`. |
| `choose_falls_back_to_the_name_when_no_remote_matches` | Repo with a key; the only name match has **no** remote → `Inferred { by: Name }`. |
| `choose_rejects_a_name_match_whose_remote_contradicts` | Repo with a key; the name match's remote is `github.com/fork/r` → `NoMatch` (OQ-31). |
| `choose_accepts_a_name_match_for_a_repo_with_no_remote_url` | `remote_url: None`; a name match with any remote → `Inferred { by: Name }`. |
| `choose_refuses_two_name_matches` | Two candidates named like the repo, both without remotes → `Ambiguous { candidates: 2 }`. |
| `choose_excludes_a_path_another_repo_holds` | The single remote match's path is in `held` → `NoMatch`. |

**Filesystem cases** (`tests/infer.rs`, `tempfile::tempdir()`). A repository is
`htui_orch::isolate::git::testkit::repo_with_one_commit(&dir)` (D131). A remote is appended to
`dir/.git/config` as `[remote "origin"]\n\turl = …\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n`.
A local helper `checkout(root, rel, remote: Option<&str>) -> PathBuf` does both.

| Test | Asserts |
|---|---|
| `one_checkout_is_found_with_its_remote_key` | `root/core` with `git@github.com:o/core` → one `Checkout { name: "core", remote_keys: ["github.com/o/core"] }`, `truncated: false`. |
| `two_checkouts_and_none` | Two siblings → both, in byte order; an empty root → `Scan::default()`. |
| `a_checkout_at_depth_three_is_found_and_at_depth_four_is_not` | `root/a/b/c` found; `root/a/b/c/d` (with `a/b/c` not a checkout) not found. |
| `a_checkout_inside_a_checkout_is_not_listed` | `root/outer` and `root/outer/inner` → only `outer`. |
| `a_symlinked_checkout_is_not_followed` (`#[cfg(unix)]`) | `root/link -> elsewhere/real` → nothing. |
| `a_dot_directory_is_skipped` | `root/.cache/core` → nothing. |
| `a_gitdir_file_checkout_is_listed` | `root/wt/.git` a file `gitdir: <path of a real .git>` → listed, `name: "wt"`. |
| `the_order_is_byte_order_at_every_level` | `B`, `a`, `a/…`: `["B", "a"]`. Two scans are equal. |
| `the_directory_cap_sets_truncated` | `find_checkouts_with(root, Limits { max_depth: 3, max_dirs: 3 })` over five sibling directories → `truncated: true`. |
| `the_root_itself_may_be_a_checkout` | The root is a repository → one checkout whose `path` is the root; nothing below is examined. |
| `an_unopenable_checkout_has_no_remote_keys` | `root/broken/.git` an empty directory → listed with `remote_keys: []`. |
| `a_remote_with_a_token_leaves_only_its_key` | Remote `https://user:tok@github.com/o/r` → `remote_keys == ["github.com/o/r"]`, and `format!("{scan:?}")` contains neither `tok` nor `user` (H-7). |

**Commits**:
1. **(a) red**: `pub mod infer;` and the doc sentence; `infer.rs` with every type, the constants
   and `todo!()` bodies for `normalise_remote`, `find_checkouts_with` and `choose` (and
   `find_checkouts` delegating); all unit tests and `tests/infer.rs`. Red: every test panics at
   `todo!()`.
2. **(b) green**: `normalise_remote`, `choose`, `MatchedBy::as_str`.
3. **(c) green**: `find_checkouts_with`, `visit`, `is_checkout`, `remote_keys`.

```bash
cargo fmt --all -- --check
cargo test -p htui-orch --all-features --lib infer -- --test-threads=1
cargo test -p htui-orch --all-features --test infer -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1
cargo clippy -p htui-orch --all-features --all-targets -- -D warnings
```

---

## 4. T2: the excerpt pass in the engine (D106–D109, D118, D119, D121, D122, D125)

**First failing test**: `engine::tests::a_phase_prompt_reads_excerpts_from_the_step_tree`. T2 builds
on the **base** tree and names nothing T0 or T1 adds (H-1).

**Files** (the plan's list, unchanged): `crates/htui-core/src/prompt/mod.rs`,
`crates/htui-agent/src/excerpt.rs`, `crates/htui-agent/src/lib.rs` (optional re-export),
`crates/htui-agent/tests/excerpt.rs`, `crates/htui-orch/src/engine.rs`,
`crates/htui-orch/src/fake.rs`.

### 4.1 `htui_core::prompt` (`prompt/mod.rs`): two pure functions

After `assemble` (ends `:519`), before `substitute`:

```rust
/// ANA-5 §4.4 step 6's residual (MOD-7 milestone 4, D118): the tokens left under the target once
/// everything but the excerpts is assembled. `spec` is assembled with an empty [`ExcerptSet`] and
/// the answer is `(trim.target - trim.estimated_after).max(0)`, the budget §4.5's selection may
/// spend. The excerpt section's framing is not in it; an overshoot is §4.4's to trim, and excerpt
/// files are what it drops first.
///
/// # Errors
/// Whatever [`assemble`] refuses the excerpt-less spec with, unchanged: the caller skips the pass,
/// because the real assembly will refuse the same way.
pub fn excerpt_residual(spec: &PromptSpec, scrubber: &dyn Scrubber) -> Result<i64, AssembleError> {
    let bare = PromptSpec { excerpts: ExcerptSet::default(), ..spec.clone() };
    let assembled = assemble(&bare, scrubber)?;
    Ok((assembled.trim.target - assembled.trim.estimated_after).max(0))
}
```

After the `ScrubbedInputs` struct (which follows `scrubbed_inputs`, `:706-851`):

```rust
/// Drops every excerpt the scrubber cannot mask (MOD-7 milestone 4, plan OQ-30 default, D119),
/// with a note per file, so one secret-shaped file never refuses the whole prompt.
///
/// It checks the four strings [`assemble`] masks for an excerpt (`repo`, `path`, `content`, and
/// `provider` when there is one), in that order, so every file kept here is one `assemble` will not
/// refuse over. The note names the rule and never the content. It names `repo:path` only when both
/// scrubbed clean, because `trim_record.notes` is persisted unscrubbed:
/// - ``excerpt: `repo:path` dropped; the scrubber refused it (rule `…`)`` (content or provider);
/// - ``excerpt: a file in repo `repo` dropped; the scrubber refused its path (rule `…`)``;
/// - ``excerpt: a file dropped; the scrubber refused its repo slug (rule `…`)``.
///
/// `audit` is left as it is: `selected` still counts a dropped file, and `assemble` rebuilds
/// `audit.files` from the survivors (`ExcerptAudit`'s documented asymmetry). Survivors keep their
/// ranks, so "the highest rank number is the worst file" still holds.
pub fn drop_unmaskable_excerpts(set: &mut ExcerptSet, scrubber: &dyn Scrubber)
```

The probe per string is `scrub_text(scrubber, value, "excerpts")` (`:903-922`), whose `Err`
carries `rule`. A string that masks cleanly (a known session secret) is **kept**: `assemble`
masks it the same way.

**Unit tests** (a new `#[cfg(all(test, feature = "test-support"))] mod residual_tests` at the end
of the file, because they build specs with `crate::prompt::fixtures`):

| Test | Asserts |
|---|---|
| `excerpt_residual_is_the_room_left_under_the_target` | `fixtures::phase_all_empty()` (or the smallest phase fixture): the answer equals `target - estimated_after` of `assemble` over the same spec with `ExcerptSet::default()`, and is `> 0`. |
| `excerpt_residual_ignores_the_specs_own_excerpts` | The same spec carrying two excerpt files answers the same number. |
| `excerpt_residual_is_never_negative` | `fixtures::phase_oversize()`: the answer is `>= 0` and equals `(target - estimated_after).max(0)`. |
| `excerpt_residual_passes_an_assemble_error_through` | `body = "{{no_such_placeholder}}"` → `Err(AssembleError::UnknownPlaceholder { .. })`. |
| `drop_unmaskable_excerpts_drops_and_names_the_file` | Two files, one whose content holds `-----BEGIN RSA PRIVATE KEY-----`. With `MinimalScrubber::new([])`: one survivor; the note is exactly ``excerpt: `htui:src/key.rs` dropped; the scrubber refused it (rule `private_key_pem`)``; `audit` unchanged. |
| `drop_unmaskable_excerpts_never_names_a_refused_path` | A file whose `path` holds `sk-…`: dropped, and the note starts `excerpt: a file in repo `htui` dropped` and contains no `sk-`. |
| `drop_unmaskable_excerpts_keeps_a_file_whose_secret_masks` | `MinimalScrubber::new(["hunter2hunter2".into()])` and a file containing it: kept. |

### 4.2 `htui_agent::excerpt`: the shared pass (D119, D126–D128)

Imports added: `std::collections::BTreeMap`; `htui_core::model::{BoxId, Repo, RepoBoxPath, RepoId,
RunStepTree}`; `htui_core::prompt::excerpt::{BuiltinRanker, ExcerptAudit, ExcerptSet, PathPrefix,
RootRecord, RootSource, BUILTIN_ID, select}`; `htui_core::prompt::{Placeholder, PromptSpec,
TokenEstimator, drop_unmaskable_excerpts, excerpt_residual, parse}`; `htui_core::scrub::Scrubber`.
Module doc gains one paragraph: "MOD-7 milestone 4 wires the pass. [`excerpts_for`] is the one
function both the engine's phase prompt and the Backlog preview call, so the bytes a maintainer
previews and the bytes a run sends cannot drift (MOD-2 D103)."

```rust
/// ANA-5 §4.5 step 1 (plan D107): one root per scope repo, by **row presence**.
///
/// The step's `run_step_tree` row for the repo (`RootSource::RunStepTree`), else this box's
/// `repo_box_path` row (`RootSource::RepoBoxPath`; rows of other boxes are ignored), else
/// `RootSource::NoPath` with an empty root. `repo` is the repo's name, the slug a rendered
/// `path="repo:…"` and `PathPrefix` use. No `stat`: an unreadable root is `select`'s "could not
/// be listed" note, which keeps this pure. In `scope` order.
#[must_use]
pub fn excerpt_roots(
    scope: &[(RepoId, String)],
    trees: &[RunStepTree],
    paths: &[RepoBoxPath],
    box_id: BoxId,
) -> Vec<RepoRoot>

/// `item.touched_paths` as §4.5's tier-1 prefixes, under `overlap::resolve`'s primary rule
/// (`crates/htui-orch/src/overlap.rs:43-54`, plan D119): a bare glob belongs to the project's
/// `is_primary` repo, and to the empty slug when there is none, which matches no repo.
#[must_use]
pub fn touched_prefixes(touched: &[String], repos: &[Repo]) -> Vec<PathPrefix>

/// What the shared pass needs beside the spec.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassInput {
    /// `excerpt_roots`' answer.
    pub roots: Vec<RepoRoot>,
    /// `touched_prefixes`' answer.
    pub touched_prefixes: Vec<PathPrefix>,
    /// The caller's own notes about the roots, which go first: a scope repo with no row, or
    /// D108's "no `run_step_tree` row yet".
    pub notes: Vec<String>,
}

/// §4.5 over the filesystem (plan D119): the built-in provider, then `select` over an
/// `FsRepoReader` built from the request's own caps (F-101). **Blocking**: `excerpts_for` calls it
/// under `spawn_blocking` whenever a root is readable.
#[must_use]
pub fn excerpt_pass(req: &OwnedExcerptRequest, est: TokenEstimator) -> ExcerptSet {
    let providers: Vec<Arc<dyn ExcerptProvider>> = vec![Arc::new(BuiltinRanker)];
    let request = req.as_request();
    let (merged, provider_set) = run_providers(&providers, &request);
    select(&FsRepoReader::new(req.caps), &request, merged, provider_set, est)
}

/// The excerpt set for `spec` (plan D109, D118, D119; MOD-7 milestone 4 D126, D128).
///
/// 1. `resolve_excerpt_caps(app)`.
/// 2. `spec.body` parsed in `spec.role` does not place `{{excerpts}}` (or does not parse): no
///    pass. The roots are recorded unscanned, with the note
///    ``excerpt: template `name` places no {{excerpts}}; nothing was read``.
/// 3. No readable root (none, or all `NoPath`): `excerpt_pass` **inline**, with a zero budget. It
///    reads nothing and spawns nothing (H-9), and records each `NoPath` with `select`'s own "no
///    readable root" note.
/// 4. Otherwise the budget is `excerpt_residual(spec, scrubber)`. An `Err` records the roots
///    unscanned, because `assemble` will refuse the same way. Then `excerpt_pass` runs under
///    `tokio::task::spawn_blocking`; a `JoinError` records the roots unscanned with
///    `excerpt: the pass panicked; no excerpts` (§4.5 fail-open).
/// 5. `drop_unmaskable_excerpts(&mut set, scrubber)`.
/// 6. `set.notes` = `input.notes`, then the pass's notes, then the drop notes.
///
/// The request is `item_key`, `item_body`, `phase` and the input documents' bodies from `spec`,
/// `input.touched_prefixes`, no `changed_paths` (D122), `input.roots`, the budget, and the caps,
/// `scan_cap` and `deadline` of step 1. `est` is `spec.estimator`.
pub async fn excerpts_for(
    spec: &PromptSpec,
    input: PassInput,
    app: &BTreeMap<String, serde_json::Value>,
    scrubber: &dyn Scrubber,
) -> ExcerptSet
```

Private `fn unscanned(roots: &[RepoRoot], caps: ExcerptCaps, notes: Vec<String>) -> ExcerptSet`:
`files: []`, audit `provider_set: [BUILTIN_ID]`, `roots` as `RootRecord { repo, source,
scan_truncated: false }` **sorted by repo bytes** (as `select` sorts them), `considered: 0`,
`selected: 0`, `caps`, `files: []`, and the given notes. The future holds `&dyn Scrubber` (which is
`Sync`) across the `spawn_blocking` await, so it stays `Send`.

`lib.rs:148` (optional, D126): `pub use excerpt::{FsRepoReader, GitignoreSubset, PassInput,
SkipRule, excerpt_pass, excerpt_roots, excerpts_for, run_providers, touched_prefixes};`. Callers
may name `htui_agent::excerpt::…` either way.

**Tests** (`crates/htui-agent/tests/excerpt.rs`, after the existing cases; `#[tokio::test]` for the
async ones; specs from `htui_core::prompt::fixtures`, with `body` overridden where a case needs a
template shape):

| Test | Asserts |
|---|---|
| `excerpt_pass_selects_a_touched_file_from_a_real_tree` | Tempdir with `src/lib.rs`; `base_request(&["src/lib.rs"], vec![RepoRoot { repo: "htui", root: dir, source: RunStepTree }])`. A file `src/lib.rs` with `reason == TouchedPath`; `audit.provider_set[0] == BUILTIN_ID`; `audit.roots == [RootRecord { "htui", RunStepTree, false }]`. |
| `excerpt_pass_records_a_no_path_root_and_scans_nothing` | One `NoPath` root → no files, `considered == 0`, `roots[0].source == NoPath`, a note containing `no readable root for repo `htui``. |
| `excerpt_roots_takes_the_tree_then_the_box_path_then_no_path` | Scope `[(a, "a"), (b, "b"), (c, "c")]`, a tree for `a`, a `repo_box_path` row for `a` and for `b` on `box` → `a` RunStepTree (the tree's path), `b` RepoBoxPath, `c` NoPath with an empty root; scope order kept. |
| `excerpt_roots_ignores_another_boxs_path` | A row for `b` on another box → `b` is `NoPath`. |
| `touched_prefixes_follow_the_overlap_primary_rule` | Repos `htui` (primary) and `docs`: `src/**` → `PathPrefix { repo: "htui", prefix: "src/" }`, `docs:guide/**` → `docs`. No primary: `src/lib.rs` → `repo: ""`. |
| `excerpts_for_reads_a_tree_it_can_render` | A phase spec whose body places `{{excerpts}}`; a readable root holding `src/lib.rs`; touched `src/lib.rs` → the set has the file, and `notes` start with `input.notes`. |
| `excerpts_for_skips_a_template_without_the_placeholder` | Body = `htui_core::prompt::body_of("verdict")`'s text; a readable root → no files, `considered == 0`, `roots` recorded as `RunStepTree`, the `places no {{excerpts}}` note. |
| `excerpts_for_with_no_roots_is_the_empty_audit` | `PassInput::default()` → exactly `empty_excerpts`' old value: `provider_set == [BUILTIN_ID]`, `roots` empty, `(considered, selected) == (0, 0)`, `caps == resolve_excerpt_caps(app).0`, no files, no notes (P-11). |
| `excerpts_for_drops_a_file_the_scrubber_refuses` | The readable file holds `PRIVATE KEY` → no files, `audit.selected == 1`, the drop note present. |

### 4.3 `fake.rs`: the tree-root override (D132)

- `FakeIsolator` gains `tree_root: Mutex<Option<PathBuf>>` (doc: "Where `prepare` roots its trees,
  when a case points it at a real directory (MOD-7 milestone 4); `None` keeps
  `FAKE_TREE_ROOT/<run>/<step>`").
- ```rust
      /// Root every tree [`prepare`](Isolator::prepare) reports at `dir`: `cwd` becomes `dir` and
      /// each repo's tree `dir/<repo id>`, for every step (MOD-7 milestone 4). The fake still creates
      /// nothing; a case that wants files under a tree writes them there itself, which is how the
      /// excerpt pass gets a readable root without a real isolator.
      pub fn root_trees_at(&self, dir: impl Into<PathBuf>) { … }
  ```
- `prepare` (`:431`, `:445`): `let cwd: PathBuf = override.unwrap_or_else(||
  PathBuf::from(format!("{FAKE_TREE_ROOT}/{run}/{step}")));`, tree `path:
  cwd.join(repo.to_string()).to_string_lossy().into_owned()`, `Prepared.cwd: cwd`. Unset, the bytes
  are the old ones (`"{cwd}/{repo}"`), so every existing case is unaffected.

### 4.4 `engine.rs`: `with_excerpts` (D125) and the call in `assemble_prompt`

Imports: `htui_agent::excerpt::{PassInput, excerpt_roots, excerpts_for, touched_prefixes}`;
`RepoBoxPath` into the `htui_core::model` list.

`assemble_prompt` (`:4898-4914`) becomes:

```rust
        let mut spec = match self
            .phase_spec(run, snapshot, step, phase, item, true)
            .await?
        {
            Ok(spec) => spec,
            Err(missing) => return Ok(Err(StageThree::MissingInput(missing))),
        };
        // MOD-7 milestone 4 (D125): here and not in `phase_spec`, which is also the handoff's
        // builder; a handoff reads no file.
        self.with_excerpts(run, step, item, &mut spec).await?;
        Ok(assemble(&spec, self.parts.scrubber).map_err(StageThree::Refused))
```

Its doc gains: "The excerpt pass runs between the two ([`Self::with_excerpts`])." Link to a private
method from a private method's doc is fine.

New private method, directly after `assemble_prompt`:

```rust
    /// ANA-5 §4.5 for a phase prompt (MOD-7 milestone 4, plan D107–D109, D125): the roots, then
    /// the shared pass, into `spec.excerpts`.
    ///
    /// The scope is `run.repo_scope` joined to the names of `repos(run.project_id)`. An id with no
    /// row there is left out with a note. The roots are this step's `run_step_tree` rows, else
    /// this box's `repo_box_path` rows, else `no_path` (`excerpt_roots`). A step with no tree rows
    /// at all, which is a fan-out group before its candidates are prepared (`drive_group`, OQ-32),
    /// that read a root from `repo_box_path` says so (D108). The item is re-read for
    /// `touched_paths`, with the same read `phase_spec` uses. Everything else, the placeholder
    /// test, the residual, the blocking read and the scrub filter, is `excerpts_for`'s.
    ///
    /// # Errors
    /// A store read's failure. Nothing about the excerpts themselves is an error: §4.5 fails open.
    async fn with_excerpts(
        &self,
        run: &Run,
        step: &RunStep,
        item: ItemId,
        spec: &mut PromptSpec,
    ) -> Result<(), EngineError>
```

Body, in order:
1. `let row = self.item(item).await?;`
2. `let repos = self.parts.store.repos(run.project_id).await?;`
3. `let mut notes = Vec::new();` The scope is built by mapping `run.repo_scope` through `repos`. A
   miss pushes ``excerpt: repo `{id}` in the run's scope has no row in its project; left out``.
4. `let trees = self.parts.store.step_trees(step.id).await?;`
5. `let mut paths = Vec::new(); for (id, _) in &scope { paths.extend(self.parts.store.repo_box_paths(*id).await?); }`
6. `let roots = excerpt_roots(&scope, &trees, &paths, self.parts.box_id);`
7. D108: `if trees.is_empty() && roots.iter().any(|root| root.source == RootSource::RepoBoxPath)`,
   push `excerpt: no run_step_tree row yet for this step; roots read from repo_box_path`.
8. `let input = PassInput { roots, touched_prefixes: touched_prefixes(&row.touched_paths, &repos), notes };`
9. `spec.excerpts = excerpts_for(spec, input, &self.parts.app, self.parts.scrubber).await;` (bind
   the answer first, then assign, so the shared borrow of `*spec` ends).
10. `Ok(())`.

**Comment and doc rewordings (D109, D121)**:
- `phase_spec`'s comment above `excerpts: no_excerpts(caps)` (`:5028-5030`): "The pass runs in
  `assemble_prompt` (`Self::with_excerpts`, MOD-7 milestone 4 D125), not here: this builder is also
  the promote/handoff path's, and a handoff never reads a file. So the caps are recorded and nothing
  else, and `with_excerpts` replaces this for a phase prompt."
- `judge_prompts`, above `excerpts: no_excerpts(caps),` (`:4402`), a new comment: "No pass for a
  judge (plan D109): its placeholder set cannot place `{{excerpts}}` (`template.rs:188-215`), so a
  file read here could only reach the audit."
- `no_excerpts`' doc (`:5750-5754`): "An excerpt set with no files, whose audit records the caps a
  pass *would* have run under. Used where no pass runs: `phase_spec`, which the handoff path shares
  (D125), and the judge, whose placeholder set cannot place `{{excerpts}}` (D109). A phase prompt's
  set is `Self::with_excerpts`'."

`phase_spec`'s body, `opening` and `promote.rs` do not change (H-8).

### 4.5 Engine unit tests (`engine.rs` `mod tests`, beside the MOD-9 skills cases `:12178`)

Helpers (test module, T2's):
- `async fn touch_feat_3(harness: &Harness, paths: &[&str])`: `update_item(FEAT-3, row.version,
  ItemPatch { touched_paths: Some(..), author_id: row.created_by, reason: "a test's touched paths",
  ..ItemPatch::default() })`, the shape of `resume_walks_a_run_whose_touched_paths_name_an_unknown_repo`
  (`:9885-9901`).
- `fn write_tree(dir: &Path, repo: RepoId, file: &str, body: &str)`: creates
  `dir/<repo>/<file>`'s parents and writes `body`.
- `async fn excerpt_prologue(harness: &Harness, dir: &Path, body: &str) -> (RepoId, Run,
  GraphSnapshot, RunStep)`: `repo = harness.add_primary_repo()` (name `htui`, `:6142`);
  `write_tree(dir, repo, "src/lib.rs", body)`; `harness.orch.isolator.root_trees_at(dir)`;
  `touch_feat_3(harness, &["src/lib.rs"])`; then `skills_prologue(harness)` (`:12180`, which calls
  `started`: `free_feat_3` plus `StartRun` with `repo_scope: None`, derived as `[repo]`).
- Every case builds its engine with `harness_engine!(harness.orch, engine)` (`:7938`), whose
  scrubber is `MinimalScrubber::new([])`.

| Test | Asserts |
|---|---|
| `a_phase_prompt_reads_excerpts_from_the_step_tree` | Prologue with `pub fn marker() {}`. `engine.assemble_prompt(&row, &snapshot, &prd, &snapshot.phases[0], FEAT-3)` is `Ok(Ok(prompt))`. `prompt.text` contains `<file path="htui:src/lib.rs"`; `prompt.trim.excerpts.roots == [RootRecord { repo: "htui", source: RunStepTree, scan_truncated: false }]`; a file record with `reason == TouchedPath`. |
| `a_template_without_excerpts_runs_no_pass` | Prologue; `spec = engine.phase_spec(.., true)`; `spec.body = body_of("verdict")`'s text; `engine.with_excerpts(&row, &prd, FEAT-3, &mut spec)`. `spec.excerpts.files` empty, `audit.considered == 0`, `audit.roots` is `htui` / `RunStepTree`, and the `places no {{excerpts}}` note is present. |
| `a_scrubber_refused_excerpt_is_dropped_with_a_note` | Prologue with a body holding `-----BEGIN RSA PRIVATE KEY-----`. `assemble_prompt` is `Ok(Ok(prompt))`, **not** `StageThree::Refused`. The text has no `<file path=`; `trim.notes` contains ``excerpt: `htui:src/lib.rs` dropped; the scrubber refused it (rule `private_key_pem`)``; `trim.excerpts.selected == 1`; `trim.excerpts.files` empty (OQ-30, D119). |
| `a_bare_glob_without_a_primary_matches_no_repo` (P-3) | A **non**-primary repo `htui` (`create_repo` with `is_primary: false`); tree written; `root_trees_at`; `touch_feat_3(&["src/lib.rs"])`; `free_feat_3`; `StartRun { repo_scope: Some(vec![repo]) }`. Through `assemble_prompt`, no `trim.excerpts.files` entry has `reason == TouchedPath`, and `roots[0].source == RunStepTree` (the file may still arrive by tier 5, which is correct). |
| `a_step_without_trees_reads_repo_box_path_with_a_note` (P-4, D108) | Primary repo `htui`; `upsert_repo_box_path(RepoBoxPath { repo, box_id: harness.orch.box_id(), local_path: dir/checkout })` with `src/lib.rs` there; `started`; a `RunStep` clone of `prd` with `id: StepId::new()` (no tree rows; `step_trees` of an unknown step is an empty list on `MemStore`, so confirm that first, else use a pending step the walk has not prepared); `with_excerpts` over a `prd` spec. `roots[0].source == RepoBoxPath`, the file is present, and `notes` contains `excerpt: no run_step_tree row yet for this step; roots read from repo_box_path`. |
| `a_handoff_spec_carries_no_excerpts_and_runs_no_pass` (D125, H-8) | Same prologue and file as the first case. (1) `engine.phase_spec(.., false)`'s `excerpts == no_excerpts(settings::resolve_excerpt_caps(&app).0)`: no files, no roots, `considered 0`. (2) `promote::handoff_spec(spec, &template, &[], &[], None, "why".to_owned()).excerpts` is that value, with `template` from `prompt_template(PROJECT_HTUI, "handoff", None)`. (3) `harness.dispatch(Command::PromoteStep { run, step: prd.id, chat_open: false })` answers `Promoted { opening, .. }` with `OpeningPath::Handoff { text, .. }`, and `!text.contains("<file path=")`, the path `the_opening_uses_the_step_s_own_trees_as_cwd` (`:10931`) drives. (4) The same fixture through `assemble_prompt` **does** hold `<file path="htui:src/lib.rs"`. |

The pure `excerpt_roots` cases the plan listed for `engine.rs` are T2's `htui-agent` tests (P-1).

### 4.6 Build coupling

T2 names only base-tree items plus its own. No `query!`. No new `WriteStore` or `GraphSource`
method, no request variant, no snapshot. After T0 merges, T2 still compiles: it implements no
`WriteStore` (H-1).

### 4.7 Runtime coupling

H-11. `crates/htui-orch/src/conformance.rs` is not edited, and its 72 cases stay green: fake roots do
not exist, so each step in a scope with a repo gains one "could not be listed" note and no file.

### 4.8 Commits (T2) and gate

1. **(a) red, `htui-core`**: `excerpt_residual` and `drop_unmaskable_excerpts` with `todo!()`
   bodies; `residual_tests`.
2. **(b) green, `htui-core`**: both bodies.
3. **(c) red, `htui-agent`**: `excerpt_roots`, `touched_prefixes`, `PassInput`, `excerpt_pass`,
   `excerpts_for` with `todo!()` bodies; the module-doc paragraph; the optional `lib.rs`
   re-export; the nine `tests/excerpt.rs` cases.
4. **(d) green, `htui-agent`**: the five bodies and `unscanned`.
5. **(e) red, `htui-orch`**: `FakeIsolator::root_trees_at` (green on arrival); `with_excerpts`
   with a `todo!()` body, **not called** from `assemble_prompt` (H-6); the helpers and the six engine
   tests. Red: the tests that call `assemble_prompt` find no file, and those that call
   `with_excerpts` panic.
6. **(f) green, `htui-orch`**: `with_excerpts`' body, the call in `assemble_prompt`, the three
   comment and doc rewordings.

```bash
cargo fmt --all -- --check
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-agent --all-features -- --test-threads=1
cargo test -p htui-orch --all-features -- --test-threads=1          # conformance's 72 still green (§4.7)
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1             # run_worker, chat, runs_pg walks (H-11)
cargo clippy -p htui-core -p htui-agent -p htui-orch --all-features --all-targets -- -D warnings
rg -n 'impl.*WriteStore for' crates/htui-orch crates/htui           # nothing (H-1)
git diff --stat e4ca5b3 -- crates/htui-orch/src/promote.rs crates/htui-orch/src/overlap.rs \
  crates/htui-orch/src/graph.rs crates/htui-orch/src/conformance.rs  # empty (H-8)
```

---

## 5. T3: the preview (D120, D121)

T3 branches from the merged Wave 1 (worktree D) and needs T2's `htui_agent::excerpt` API.

**First failing test**: `prompt_preview.rs::a_repo_path_row_puts_its_files_in_the_preview`.

**Files** (the plan's list, unchanged): `crates/htui/src/preview.rs`,
`crates/htui/src/ui/tabs/backlog/detail/prompt.rs`, `crates/htui/tests/prompt_preview.rs`,
`crates/htui/tests/settings.rs`, `crates/htui/tests/snapshots/{backlog__detail_prompt,
prompt_preview__preview_ana_2, prompt_preview__preview_feat_1}.snap`.

### 5.1 `preview::build` (`preview.rs:151-291`)

- Imports: `htui_agent::excerpt::{PassInput, excerpt_roots, excerpts_for, touched_prefixes}`;
  `htui_core::model::RepoId`; `htui_core::prompt::excerpt::ExcerptSet` (already imported);
  `htui_core::store::WriteStore as _`. Drop `BUILTIN_ID` and `ExcerptAudit` from the imports if
  nothing else uses them.
- Remove the `resolve_excerpt_caps` line (`:245-247`), because `excerpts_for` resolves the caps.
- Keep the box `match` but bind `box_id = info.box_id` for below.
- After `bound_skills` (`:249`), read two things **after** every existing read, so an offline arm
  still refuses with the prompt sentence first (the D64/F-L rule the fn's own comment cites):

```rust
    // MOD-7 milestone 4 (plan D120): ANA-5 §4.5 step 1's "otherwise the project's repos". A
    // preview has no run, so no `run_step_tree` row: each root is this box's `repo_box_path` row,
    // else `no_path`.
    let writer = backend
        .writer()
        .ok_or_else(|| StoreError::Unreachable(offline_refusal().to_owned()))?;
    let repos = writer.repos(row.project_id).await?;
    let paths = backend.repo_paths(box_id).await?;
    let scope: Vec<(RepoId, String)> =
        repos.iter().map(|repo| (repo.id, repo.name.clone())).collect();
    let input = PassInput {
        roots: excerpt_roots(&scope, &[], &paths, box_id),
        touched_prefixes: touched_prefixes(&row.touched_paths, &repos),
        notes: Vec::new(),
    };
```

- The spec is built with `excerpts: ExcerptSet::default()` and made `mut`. Then `let scrubber =
  MinimalScrubber::new([]);`, `spec.excerpts = excerpts_for(&spec, input, &app, &scrubber).await;`,
  and `outcome: assemble(&spec, &scrubber)` reuses the one scrubber.
- `build`'s doc: replace "Nine reads … and three fields filled by [`STAND_INS`] instead of by a
  `run_step`" with the same sentence plus "Two more since MOD-7 milestone 4: the project's repos and
  this box's `repo_box_path` rows, the roots the excerpt pass reads (plan D120)."

### 5.2 Deletions and rewordings (D121, P-11)

- Delete `empty_excerpts` (`:292-314`) and the unit test
  `the_empty_audit_registers_the_builtin_and_records_the_caps` (`:479-491`).
- `EXCERPTS_NOTE` (`:104-107`) becomes exactly `"preview: no run exists, so no run_step_tree row;
  roots come from this box's repo_box_path rows, else no_path"`, and stays in `STAND_INS`. Its doc
  becomes "ANA-5 §12 criterion 12, as the preview meets it (plan D103; MOD-7 milestone 4 D120)."
- Module doc (`:1-10`): after "…which defeats the only purpose it has.", add: "Since MOD-7
  milestone 4 the excerpt section is real: the roots are this box's `repo_box_path` rows, the rung a
  run reads before its trees exist, and the pass is `htui_agent::excerpt::excerpts_for`, the
  engine's own."

### 5.3 The Prompt sub-tab (`ui/tabs/backlog/detail/prompt.rs`, `excerpt_lines` `:235-264`)

The empty-roots arm (`:242-246`) becomes

```rust
    let roots = if audit.roots.is_empty() {
        // The preview's scope is the project's repos (MOD-7 milestone 4, D120), so an empty list
        // means the project has none; a repo with no path records `no_path` like any other root.
        "none \u{b7} this project has no repo".to_owned()
    } else {
```

It fits the 43-column detail pane: `roots     ` plus 31 characters is 41.

### 5.4 Tests (`prompt_preview.rs`, `settings.rs`) and snapshots

| Test | Change |
|---|---|
| `the_prompt_sub_tab_previews_feat_1` (`:119-122`) | `frame.contains("this project has no repo")`, message: "ANA-5 §12 criterion 12: the demo project has no repo, and the roots line says so". |
| `the_preview_assembles_from_real_store_reads` (`:142-145`) | The `roots.is_empty()` message becomes "the demo project seeds no repo, so there is no root to record". The other asserts stand, including no `<section name="excerpts">`, `provider_set == ["builtin@1"]` and `caps.max_files > 0`. |
| `an_item_with_nothing_attached_still_previews` (`:481`) | `frame.contains("this project has no repo")`, "criterion 12 again, at 100x30". |
| **new** `a_repo_path_row_puts_its_files_in_the_preview` | `store = MemStore::demo()`; `repo = create_repo(NewRepo { project_id: ids::PROJECT_HTUI, name: "htui", is_primary: true, .. })`; tempdir with `src/lib.rs`; `upsert_repo_box_path(RepoBoxPath { repo_id: repo, box_id: ids::BOX, local_path: dir, .. })`; FEAT-1's `touched_paths = ["src/lib.rs"]` through `update_item`. `preview::build(&Backend::memory(store), FEAT-1, None, &scope)`: the text contains `<file path="htui:src/lib.rs"`; `trim.excerpts.roots == [("htui", RepoBoxPath)]`; the digest differs from the same item's digest over a fresh `MemStore::demo()`. |
| **new** `a_repo_without_a_path_row_records_no_path` | A second repo `docs` (not primary) with no path row, in the same fixture: `roots` holds `docs` / `NoPath`, and `trim.notes` contains `excerpt: no readable root for repo `docs`; nothing was scanned`. |
| `tests/settings.rs:794` | The doc becomes "A second section, so the strip has something to cycle between whichever product sections are registered." |

**Snapshots**, reviewed with `cargo insta review`. Accept only these moves:
- `backlog__detail_prompt.snap:15` and `prompt_preview__preview_ana_2.snap:15`: `roots     none ·
  this project has no repo`.
- `prompt_preview__preview_feat_1.snap:15`, the same, and `:36`, the clipped reworded note.
- **No digest line (`:11`) and no `tokens` line moves**: the demo project has no repo, so no file
  renders (claims 40-42). A moved digest line is a T2 or T3 defect: stop and route it back.

`backlog__detail_prompt.snap` comes from `tests/backlog.rs`, which T3 does not edit. In-lane
snapshot count: **87**.

### 5.5 Commits (T3) and gate

1. **(a) red**: the five test changes above, including the two new cases; the deletion of the
   `preview.rs` unit test. Red: the new cases find no file and no root, and the frame asserts do not
   find the new copy.
2. **(b) green**: §5.1–§5.3, `tests/settings.rs:794`, the three snapshots.

```bash
cargo fmt --all -- --check
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
ls crates/htui/tests/snapshots | wc -l          # 87 in this lane
git status --porcelain crates/htui/tests/snapshots   # three `M`, no `??`, no `.snap.new`
```

---

## 6. T4: inference served and shown (D114–D117, D124)

T4 branches from the merged Wave 1 (worktree E). It needs T0's `infer_repo_box_path` and T1's
`htui_orch::infer`.

**First failing test**: `hierarchy.rs::inference_writes_one_canonical_row_by_remote`.

**Files** (the plan's list, unchanged): `crates/htui/src/hierarchy.rs`,
`crates/htui/src/store_worker.rs`, `crates/htui/src/ui/tabs/settings/hierarchy.rs`,
`crates/htui/tests/hierarchy.rs`, `crates/htui/tests/hierarchy_pg.rs` (new),
`crates/htui/tests/snapshots/hierarchy__demo.snap`, `hierarchy__inferred.snap` (new).

### 6.1 `store_worker.rs`: the variants (D143)

- `StoreRequest`, directly after `DeleteProject(ProjectId)` (`:389`):

```rust
    /// Infer this box's checkout paths for every repo of a workspace that has none here (MOD-7
    /// milestone 4, PRD D5, plan D114): the workspace root on this box is walked, and each repo
    /// with exactly one matching checkout gets a canonical row, inserted only where none exists,
    /// so a manual path is never replaced. Answers [`StoreReply::RepoPathsInferred`].
    InferRepoPaths(WorkspaceId),
```

- `StoreRequest::name`, after `Self::DeleteProject(..) => "delete_project",` (`:630`):
  `Self::InferRepoPaths(..) => "infer_repo_paths",`.
- `StoreReply`, directly after `Deleted { .. }` (`:779-786`):

```rust
    /// Answer to [`StoreRequest::InferRepoPaths`]: the tree as it is now and what the pass did,
    /// per repo (plan D116). One reply carries both, so the section never patches a row locally.
    /// No URL travels here: the report names repos and canonical paths only.
    RepoPathsInferred {
        /// The workspace re-read after the writes.
        tree: Box<HierarchySnapshot>,
        /// Per repo, what happened and why.
        report: InferReport,
    },
```

  Import `InferReport` beside `HierarchySnapshot` from `crate::hierarchy`.
- `try_serve`: add `| StoreRequest::InferRepoPaths(..)` to the hierarchy or-arm (`:1078-1089`).

### 6.2 `hierarchy.rs`: the report types (D116, D135)

After `MirrorAfterDelete` (`:78`):

```rust
/// What one `InferRepoPaths` did (MOD-7 milestone 4, plan D116). Carries no URL and no id of a
/// box or a user: repo names, canonical paths and outcomes only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferReport {
    /// This box's workspace root, canonical; `None` when the workspace has no root here, in which
    /// case nothing was walked and `repos` is empty.
    pub root: Option<String>,
    /// The walk hit `htui_orch::infer::MAX_DIRS`; every repo without a row is `ScanTruncated`.
    pub truncated: bool,
    /// One per repo of the workspace, in the tree's order (projects by position, repos by name).
    pub repos: Vec<RepoInference>,
}

/// One repo's line of an [`InferReport`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInference {
    /// The repo.
    pub repo: RepoId,
    /// `repo.name`, what the notice prints.
    pub name: String,
    /// What happened.
    pub outcome: InferOutcome,
}

/// Why a repo did or did not get a row (plan D116).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferOutcome {
    /// A row existed before the pass, or a manual write landed first (the insert answered `false`).
    AlreadySet,
    /// A row was written.
    Inferred {
        /// The canonical path stored.
        path: String,
        /// Which rung chose it.
        by: htui_orch::infer::MatchedBy,
    },
    /// No checkout matched.
    NoMatch,
    /// Several did; nothing was written.
    Ambiguous {
        /// How many.
        candidates: usize,
    },
    /// The chosen path was refused by `canonical_root`, or the store refused the row. The
    /// sentence names the path as found, never a link's target.
    Refused(String),
    /// The scan was cut short, so nothing was inferred.
    ScanTruncated,
}
```

### 6.3 `hierarchy.rs`: serving it (D116, D134)

`serve`'s match gains `StoreRequest::InferRepoPaths(ws) => infer(backend, &writer, *ws,
this_box).await,` before the `other =>` arm. New private fn after `workspace_of`:

```rust
/// `InferRepoPaths` (MOD-7 milestone 4, PRD D5, plan D114, D116, D124): this box's filesystem,
/// this box's rows, this workspace only.
///
/// The box first, then the tree, then this box's root, canonicalised (a legacy link row walks its
/// target). With no root, the answer is `root: None` and nothing is walked. With every repo
/// already set, nothing is walked either. Otherwise one `find_checkouts` under `spawn_blocking`,
/// then per repo without a row, in tree order: `choose` against the paths already held on this
/// box, `canonical` of the chosen path, and `infer_repo_box_path`. A path this pass writes is held
/// for every later repo. A truncated scan infers nothing. The reply is the re-read tree and the
/// report.
async fn infer(
    backend: &Backend,
    writer: &Writer,
    ws: WorkspaceId,
    this_box: Option<BoxId>,
) -> Result<StoreReply>
```

Body, in order:
1. `let box_id = box_id(this_box)?;`
2. `let tree = snapshot(writer, ws, this_box).await?.ok_or_else(|| NotFound { "workspace", ws })?;`
3. No `tree.root_path`: reply `RepoPathsInferred { tree, report: InferReport { root: None,
   truncated: false, repos: vec![] } }`.
4. `let root = canonical(&root_row.root_path).await?;`. A root that no longer resolves is a
   `Constraint`, and the status line reads `infer_repo_paths: constraint violated: …`.
5. For every `RepoEntry` in `tree.projects[..].repos[..]`, `local_path: Some(_)` →
   `AlreadySet`; the rest are pending.
6. No pending → reply with the report, and walk nothing.
7. `let mut held: BTreeSet<PathBuf> = backend.repo_paths(box_id).await?.into_iter().map(|row|
   PathBuf::from(row.local_path)).collect();`, every repo's row on this box, not only this
   workspace's.
8. `let scan = tokio::task::spawn_blocking({ let root = PathBuf::from(&root); move ||
   infer::find_checkouts(&root) }).await.map_err(|err| StoreError::Backend(err.to_string()))?;`
9. `scan.truncated` → every pending repo is `ScanTruncated`, nothing is written, and the report has
   `truncated: true`.
10. Else, per pending repo in order: `infer::choose(&entry.repo, &scan.checkouts, &held)`:
    - `NoMatch` / `Ambiguous { candidates }` map across.
    - `Inferred { path, by }`: `path.to_str()`, else `Refused("the checkout's path is not valid
      UTF-8")`. Then `canonical(text)`: `Err(StoreError::Constraint(msg))` → `Refused(msg)`, any
      other `Err` propagates. Then `writer.infer_repo_box_path(&RepoBoxPath { repo_id, box_id,
      local_path: canonical, updated_at: Utc::now() })`: `Ok(true)` → `Inferred`, and the path goes
      into `held`; `Ok(false)` → `AlreadySet`; `Err(StoreError::Constraint(msg))` → `Refused(msg)`;
      any other `Err` propagates.
11. `reread`'s tree (a vanished workspace is `NotFound`), reply `RepoPathsInferred { tree:
    Box::new(fresh), report }`.

Rewordings (P-7): the module doc (`:1-2`) "…, thirteen served requests, …"; `serve`'s doc
(`:165-167`) "`try_serve` routes exactly the hierarchy variants below here" (no count);
`REQUEST_NAMES`' doc (`:451`) "The thirteen request names, in [`StoreRequest`] order."; the array
becomes `[&str; 13]` with `"infer_repo_paths"` last.

### 6.4 The section: key, follow-up, reply (`ui/tabs/settings/hierarchy.rs`; D114, D136)

- `HINT_BROWSE` (`:46`): `"j/k · N workspace · n project/repo · e edit · p primary · b path · i
  infer · d delete · r reload"` (with `\u{b7}` separators; 96 columns, claim 48).
- `on_key` Browse comment (`:1049-1051`): add `i` to the list of free keys. New arm after `'b'`:

```rust
            // MOD-7 milestone 4 (D114): infer this box's repo paths under the workspace root. A
            // write, so it is refused while another is in flight, as the other write keys are.
            KeyCode::Char('i') => {
                if !self.refuse('i')
                    && let Some(snapshot) = &self.snapshot
                {
                    let request = StoreRequest::InferRepoPaths(snapshot.workspace.id);
                    self.notice = None;
                    self.send(request, ctx);
                }
                Handled::Consumed
            }
```

- `on_tree` (`:928`), D136. Before `self.mode = Mode::Browse`, capture `let follow = write.is_some_and(|name| matches!(name, "set_workspace_root" | "create_repo" | "update_repo")) && matches!(&self.mode, Mode::Editing(editor) if matches!(editor.kind, EditorKind::WorkspaceRoot(_) | EditorKind::NewRepo(_) | EditorKind::EditRepo { .. }));`. At the end, after the D11 scope follow: `if follow && snapshot.root_path.is_some() { self.send(StoreRequest::InferRepoPaths(snapshot.workspace.id), ctx); }`. The `written` notice ("stored as …") set above stays on screen until the inference answers.
- `on_reply`, new arm after `HierarchyStale`:

```rust
            // MOD-7 milestone 4 (D117): a tree like any other, then what the pass did. A notice a
            // write left (`stored as …`, a follow-up's cause) is kept in front of the report.
            StoreReply::RepoPathsInferred { tree, report } => {
                let before = self.notice.take();
                self.on_tree(tree, ctx);
                let report = inferred_notice(report);
                self.notice = Some(match before {
                    Some(before) => format!("{before} \u{b7} {report}"),
                    None => report,
                });
            }
```

  `on_tree` takes `busy`, which was `"infer_repo_paths"`, and its `written` answers `None` because
  the mode is `Browse`. A refused inference lands in the existing `Failed` arm through
  `REQUEST_NAMES`.

### 6.5 The notice (D137)

A free `pub(crate) fn inferred_notice(report: &InferReport) -> String` in the section module:

| Report | Notice (exact) |
|---|---|
| `root: None` | `no root on this box for this workspace — b on the workspace row sets it` |
| `truncated: true` | `the scan stopped at 5000 directories; nothing inferred` (the number from `htui_orch::infer::MAX_DIRS`) |
| `repos` empty | `no repo in this workspace to infer` |
| otherwise | `inferred {n} of {m}`, then, each only when non-empty and joined with ` · `: `{k} already set`, `no checkout: {names}`, `ambiguous: {name} ({c}), …`, `refused: {names}`; then ` — b on a repo sets it by hand` when any repo is `NoMatch`, `Ambiguous` or `Refused` |

`n` counts `Inferred`, `k` counts `AlreadySet`, and `m` counts every repo that is not `AlreadySet`.
`{names}` lists at most three repo names in report order, then `+{rest} more`. `Refused`'s sentence
goes to the status line only through a `Failed` reply; in the notice a refused repo is named, not
explained. Example: `inferred 1 of 3 · 1 already set · no checkout: docs · ambiguous: web (2) — b on
a repo sets it by hand`.

### 6.6 Tests (first)

**Serve cases** (`tests/hierarchy.rs`, worker half, over `demo()` = `Backend::memory(MemStore::demo())`,
workspace `ids::WORKSPACE_GRAPHICS`, project `ids::PROJECT_VULKAN`). Local helpers:
`create_repo(backend, name, remote) -> RepoId` through `serve(CreateRepo)` then a tree read;
`checkout(dir, rel, remote)` = `repo_with_one_commit` plus a hand-written `[remote]` section (D131;
`htui-orch` is a dev-dependency with `test-support`); `inferred(reply) -> (HierarchySnapshot,
InferReport)`; `set_root(backend, dir)` through `serve(SetWorkspaceRoot)`.

| Test | Asserts |
|---|---|
| `inference_writes_one_canonical_row_by_remote` | Repo `core`, remote `https://example.com/o/core.git`; checkout `root/src/core` with remote `git@example.com:o/core`. `InferRepoPaths` → `core` is `Inferred { path: canonical(root/src/core), by: Remote }`; the tree's `local_path` is that path, with `box_id == ids::BOX`; `report.root == Some(canonical(root))`. |
| `inference_leaves_a_manual_row_and_reports_it_already_set` | Repo `docs`, a checkout `root/docs`, and a manual `SetRepoPath` to `root/elsewhere` (an existing directory). Inference → `AlreadySet`; the tree still shows `root/elsewhere`. |
| `two_clones_are_ambiguous_and_write_nothing` | Two checkouts with the repo's remote → `Ambiguous { candidates: 2 }`; `local_path` stays `None`. |
| `no_checkout_reports_no_match` | Repo `web`, nothing under the root → `NoMatch`, no row. |
| `a_name_match_is_inferred_for_a_repo_without_a_remote` | Repo `tools` (`remote_url: None`), checkout `root/tools` → `Inferred { by: Name }`. |
| `a_symlinked_root_yields_paths_under_its_target` (`#[cfg(unix)]`) | Real root `real/` holds checkout `core`. The workspace root is set through the link `link -> real`, and `SetWorkspaceRoot` stores `canonical(real)`. The inferred path starts with `canonical(real)` and never contains `link`. |
| `no_root_reports_none_and_walks_nothing` | No root → `report == InferReport { root: None, truncated: false, repos: vec![] }`. |
| `a_second_pass_changes_nothing` | After the first case, a second `InferRepoPaths` → `core` is `AlreadySet`, and the tree equals the first reply's. |
| `a_scan_that_hits_the_cap_infers_nothing` | `htui_orch::infer::MAX_DIRS` empty sibling directories plus one matching checkout → `truncated: true`, the repo `ScanTruncated`, no row. (`MAX_DIRS` `mkdir`s take well under a second.) |
| `hierarchy_names_are_stable` (`:92`) | Gains `StoreRequest::InferRepoPaths(WorkspaceId::default())` last; the doc says "thirteen". |
| `offline_refuses_every_hierarchy_request_by_name` (`:667`) | Through `hierarchy_requests()` (`:688`), which gains the same request last; the docs at `:664`, `:687` say "thirteen". |

**Section cases** (`SectionBench`, below the `section (T3)` divider at `:742`):

| Test | Asserts |
|---|---|
| `i_sends_infer_repo_paths_for_the_scope` | After `demo_tree(GRAPHICS)` is replied, `i` → exactly `[Action::Store(StoreRequest::InferRepoPaths(ids::WORKSPACE_GRAPHICS))]`. |
| `i_is_refused_while_a_write_is_in_flight` | Over the tree with repos that `p_moves_the_primary` builds, `p` on a repo row (in flight, no reply yet), then `i` → no second request, and the notice is `` `update_repo` is still in flight``. |
| `a_root_write_that_applied_is_followed_by_one_inference` | `b` on the workspace row, type, `Enter` (`SetWorkspaceRoot`), then reply a tree **with** a root → the next drained actions hold exactly one `InferRepoPaths(GRAPHICS)`. |
| `a_new_repo_on_a_workspace_without_a_root_is_not_followed` | `n` on the project, fill, `Enter`, reply a tree with no root → no `InferRepoPaths`. |
| `p_is_not_followed_by_an_inference` | `p`, reply a tree with a root → no `InferRepoPaths` (P-8). |
| `an_inference_reply_renders_the_tree_and_the_report` | A synthetic `RepoPathsInferred` whose tree is `demo_tree(GRAPHICS)` with two repos and fixed `local_path` strings (`/srv/graphics/core`, `None`), and whose report is `core` `Inferred { by: Remote }` and `docs` `NoMatch`. `insta::assert_snapshot!("inferred", bench.render_section(&section, 100))` (H-10). The frame contains `inferred 1 of 2 · no checkout: docs — b on a repo sets it by hand`. |
| `the_report_notice_names_what_was_not_inferred` | `inferred_notice` over the four table rows of §6.5, byte-exact, including a fifth name becoming `+2 more`. |
| `the_demo_tree_renders` (existing) | `hierarchy__demo.snap:32` gains `· i infer`, and nothing else moves. |

`inferred_notice` is `pub(crate)`, so the last case goes in the section module's own `#[cfg(test)]`
block if it has one; otherwise the case asserts the rendered frame text through `SectionBench` for
each row.

**Postgres case** (`crates/htui/tests/hierarchy_pg.rs`, new; D144). The module doc names MOD-7
milestone 4 and says what only Postgres can show: the insert-if-absent statement's conflict clause
against a manual upsert, and the canonical path round-tripped through `TEXT`. `#![cfg(feature =
"testkit")]` and `#![cfg(unix)]` (the symlink-free case still writes files; unix-only matches the
other `_pg` suites that touch the disk). Conventions of `box_probe_pg.rs:29-31`: `let Some(db) =
htui_store::testkit::demo_db().await else { return };` (the testkit prints `SKIP` and panics under
`CI`). `Backend::Online { pg: db.store.clone(), cache }` over a `CacheStore::open(tempdir, …)`.
Driven through `htui::store_worker::serve`, with no keyring guard: nothing on this path reads
`secret::`, and if a read ever does, add `testkit::mock_keyring()` as `templates_pg.rs` does.

| Test | Asserts |
|---|---|
| `inference_writes_one_row_and_keeps_a_manual_one_on_postgres` | `core` (remote `https://example.com/o/core.git`) and `docs` (no remote) in `PROJECT_VULKAN`; checkouts `root/core` (remote `git@example.com:o/core`) and `root/docs`; `SetWorkspaceRoot(root)`; `SetRepoPath(docs → root/manual)`. `InferRepoPaths` → `core` `Inferred { by: Remote }`, `docs` `AlreadySet`. `db.store.repo_box_paths(core)` is exactly one row, `local_path == canonical(root/core)`, `box_id == backend.box_info()?.box_id`. `repo_box_paths(docs)` is still `root/manual`. A second pass: both `AlreadySet`, and the rows are unchanged. `cache.close()` and `db.drop_db()` last. |

### 6.7 Snapshots

`hierarchy__demo.snap`: only line 32 changes. `hierarchy__inferred.snap`: new. In-lane count:
**88**.

### 6.8 Stale-count rewordings (P-7)

`store_worker.rs:297` "The thirteen hierarchy requests…"; `:300` "All thirteen are served…";
`:617` "The thirteen of `hierarchy::REQUEST_NAMES`…"; `:1073` "The thirteen hierarchy
requests, or-ed…"; `:1090` "…the same reason the thirteen above are"; `:1102` "twenty-one" →
"twenty-two"; `:1113` "twenty-four" → "twenty-five"; `:1122` "twenty-eight" → "twenty-nine".

### 6.9 Commits (T4) and gate

1. **(a) red**: the variants, `name`, the `try_serve` arm; the report types; `REQUEST_NAMES` 13;
   the `serve` arm to `infer` with a `todo!()` body (only the new request reaches it, H-6); every
   test and the Postgres file; all rewordings of §6.3 and §6.8. Red: the serve cases panic, and
   the section cases find no `i` and no reply arm.
2. **(b) green, worker**: `infer`'s body.
3. **(c) green, section**: `HINT_BROWSE`, `i`, the follow-up, the reply arm, `inferred_notice`,
   both snapshots.

```bash
cargo fmt --all -- --check
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features -- --test-threads=1
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui --all-features --test hierarchy_pg -- --test-threads=1 --nocapture   # must RUN, no `SKIP` line
cargo clippy -p htui --all-features --all-targets -- -D warnings
ls crates/htui/tests/snapshots | wc -l          # 88 in this lane
rg -n 'twelve' crates/htui/src/hierarchy.rs crates/htui/src/store_worker.rs crates/htui/tests/hierarchy.rs   # nothing
```

---

## 7. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T0 `htui_core::store::WriteStore` | `async fn infer_repo_box_path(&self, path: &RepoBoxPath) -> Result<bool>`: `true` inserted, `false` a row held the pair and is untouched; `Constraint` on an unknown id, nothing written | T4 (`hierarchy::infer` through `Writer`) |
| T1 `htui_orch::infer` | `MAX_DEPTH = 3`, `MAX_DIRS = 5_000`, `Limits { max_depth, max_dirs }` + `Limits::DEFAULT`, `Checkout { path, name, remote_keys }`, `Scan { checkouts, truncated }`, `MatchedBy { Remote, Name }` + `as_str`, `Choice { Inferred { path, by }, Ambiguous { candidates }, NoMatch }`, `normalise_remote(&str) -> Option<String>`, `find_checkouts(&Path) -> Scan`, `find_checkouts_with(&Path, Limits) -> Scan`, `choose(&Repo, &[Checkout], &BTreeSet<PathBuf>) -> Choice` | T4 |
| T2 `htui_core::prompt` | `excerpt_residual(&PromptSpec, &dyn Scrubber) -> Result<i64, AssembleError>`; `drop_unmaskable_excerpts(&mut ExcerptSet, &dyn Scrubber)` | T2 (`excerpts_for`) |
| T2 `htui_agent::excerpt` | `excerpt_roots(&[(RepoId, String)], &[RunStepTree], &[RepoBoxPath], BoxId) -> Vec<RepoRoot>`; `touched_prefixes(&[String], &[Repo]) -> Vec<PathPrefix>`; `PassInput { roots, touched_prefixes, notes }`; `excerpt_pass(&OwnedExcerptRequest, TokenEstimator) -> ExcerptSet`; `async excerpts_for(&PromptSpec, PassInput, &BTreeMap<String, Value>, &dyn Scrubber) -> ExcerptSet` | T2 (engine), T3 (preview) |
| T2 `htui_orch::fake::FakeIsolator` | `root_trees_at(&self, impl Into<PathBuf>)` | T2 tests |
| T4 `htui::store_worker` | `StoreRequest::InferRepoPaths(WorkspaceId)`, name `infer_repo_paths`; `StoreReply::RepoPathsInferred { tree: Box<HierarchySnapshot>, report: InferReport }` | T4 section |
| T4 `htui::hierarchy` | `InferReport { root, truncated, repos }`, `RepoInference { repo, name, outcome }`, `InferOutcome { AlreadySet, Inferred { path, by }, NoMatch, Ambiguous { candidates }, Refused(String), ScanTruncated }`; `REQUEST_NAMES: [&str; 13]` | T4 section and tests |

**Byte-exact strings**: ``excerpt: `htui:src/lib.rs` dropped; the scrubber refused it (rule
`private_key_pem`)``; `excerpt: no run_step_tree row yet for this step; roots read from
repo_box_path`; ``excerpt: template `verdict` places no {{excerpts}}; nothing was read``;
`excerpt: the pass panicked; no excerpts`; `preview: no run exists, so no run_step_tree row; roots
come from this box's repo_box_path rows, else no_path`; `none · this project has no repo`; §6.5's
notice table.

**Parallel-lane hazards**: T0 ∩ T1 = T0 ∩ T2 = T1 ∩ T2 = ∅ and T3 ∩ T4 = ∅ (the plan's check,
re-verified: P-1 adds no file to any list). `.sqlx` moves in T0 only. Store pins move in T0 only.
T3 owns the three preview snapshots and T4 the two hierarchy ones. T3 and T4 both compile
`crates/htui`, which is why each has its own worktree and they merge one at a time.

---

## 8. Count pins, merge order and the workspace gate

| Pin | Now | After | Task |
|---|---|---|---|
| Store `CASES` | 76 | 77 | T0 (`store/conformance.rs`, `mem_store.rs:37`, `pg_conformance.rs:19`); doc pin `HANDOFF.md:42` is the main thread's |
| `READ_CASES` | 14 | 14 | — |
| `htui-orch` `CASES` | 72 | 72 | — |
| `GraphSource` methods | 7 | 7 | — |
| `WriteStore` methods | n | n + 1 | T0 |
| `.sqlx` files | 267 | 268 (one `??`, nothing modified) | T0 |
| `StoreRequest` / `StoreReply` | 68 / 39 | 69 / 40 | T4 |
| `hierarchy::REQUEST_NAMES` | 12 | 13 | T4 |
| Migrations | `0001`..`0007` | unchanged; next `0008` | — |
| `crates/htui/tests/snapshots` | 87 | 88 (T4 adds `hierarchy__inferred`; T3 updates three, T4 one) | T3, T4 |

[T0 ∥ T1 ∥ T2] → merge T0 (core, store, agent gates; `cargo check --workspace`) → merge T1 (orch
gate) → merge T2 (core, agent, orch, htui gates, plus the orch clippy) → [T3 ∥ T4] → merge one (htui
gate) → merge the other (htui gate, snapshots 88) → workspace gate → main thread updates the
`HANDOFF.md:39-42` pins → optional live check.

```bash
df -h /
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
docker compose exec -T postgres dropdb -U postgres --if-exists htui_prepare_mod7m4
docker compose exec -T postgres createdb -U postgres htui_prepare_mod7m4
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m4 \
  sqlx migrate run --source crates/htui-store/migrations
(cd crates/htui-store && DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_prepare_mod7m4 \
  cargo sqlx prepare --check -- --all-targets --all-features)
ls crates/htui-store/.sqlx | wc -l                    # 268
ls crates/htui/tests/snapshots | wc -l                # 88
cargo doc --workspace --no-deps --keep-going          # exactly the six baseline errors (HANDOFF.md:43-47)
git diff --stat e4ca5b3 -- crates/htui-store/migrations crates/htui-store/cache_migrations \
  crates/htui-core/src/fixtures.rs crates/htui-orch/src/graph.rs crates/htui-orch/src/conformance.rs \
  crates/htui-orch/src/promote.rs crates/htui-orch/src/overlap.rs crates/htui/src/run_worker.rs   # empty
rg -n 'impl.*WriteStore for' crates/htui-orch crates/htui   # nothing
```

Before believing a Postgres failure, run `df -h /` and re-run the case alone (project memory: the
dev Postgres also restarts under load and reports "healthy" while still recovering).

---

## 9. Decisions (D126 onward) and risks (R-54 onward)

| # | Decision |
|---|---|
| D126 | One async `htui_agent::excerpt::excerpts_for` holds D109's placeholder test, D118's residual, D119's blocking read, its fail-open and its scrub filter. `Engine::with_excerpts` and `preview::build` each make one call (P-1). |
| D127 | `excerpt_roots` and `touched_prefixes` are `pub` in `htui_agent::excerpt`, so the engine and the preview resolve roots and tier-1 prefixes with one function each. `excerpt_roots` takes `box_id` and filters `repo_box_path` rows itself, so a caller cannot forget to (P-1). |
| D128 | `spawn_blocking` only when a root is readable. With no readable root the pass runs inline with a zero budget and no residual: no I/O, no thread, and the same audit and "no readable root" notes as a real pass (H-9). |
| D129 | The scrub filter is pure `htui_core::prompt::drop_unmaskable_excerpts`. It checks the four strings `scrubbed_inputs` masks, keeps a file whose secret masks, and never names a string the scrubber refused (P-2). |
| D130 | `Checkout.remote_keys` holds normalised keys only. `find_checkouts_with(root, Limits)` is public so the cap is testable (P-5). |
| D131 | T1's and T4's test repositories are `isolate::git::testkit::repo_with_one_commit` plus a hand-written `[remote]` section: no `git` binary (P-6). |
| D132 | `FakeIsolator::root_trees_at(dir)` roots every tree at `dir/<repo id>` and `cwd` at `dir`. Unset, the fake's bytes are unchanged (P-10). |
| D133 | T2's test list: `a_bare_glob_without_a_primary_matches_no_repo` replaces the plan's "selects nothing" (tier 5 still selects), and `a_step_without_trees_reads_repo_box_path_with_a_note` replaces the fan-out case (P-3, P-4). |
| D134 | `hierarchy::infer`'s order: box, tree, root, `canonical(root)`, pending, held (every row on this box), one walk, then `choose` per repo **sequentially**, the held set growing with each write, then the re-read. A truncated scan writes nothing. |
| D135 | `InferOutcome::Inferred.by` is `htui_orch::infer::MatchedBy`. The report carries repo names and canonical paths, never a URL, a `BoxId` or a `UserId`. |
| D136 | The follow-up inference fires only after an editor write of kind `WorkspaceRoot`, `NewRepo` or `EditRepo` that applied, and only when the fresh tree has a root on this box. `p` is excluded; `i` always sends (P-8). |
| D137 | The notice grammar of §6.5: `M` counts repos without a row before the pass, the empty workspace has its own sentence, and each name list is capped at three plus `+N more` (P-12). |
| D138 | `hierarchy__inferred` is a `SectionBench::render_section` snapshot of a synthetic reply with fixed paths (H-10). |
| D139 | The preview's `empty_excerpts` and its unit test are deleted; `excerpts_for` over no roots returns the same value, pinned in `htui-agent` and through `build` (P-11). |
| D140 | A red commit never wires a `todo!()` into an existing path: `with_excerpts` stays uncalled until its green commit, and `infer` is reachable only through the new request (H-6). |
| D141 | `excerpt_residual` assembles the spec with `ExcerptSet::default()`; the audit and notes do not reach the estimate. |
| D142 | Every stale count in `hierarchy.rs`, `store_worker.rs` and `tests/hierarchy.rs` is reworded in T4, the cumulative "twenty-…" comments included (P-7). |
| D143 | `InferRepoPaths` is the last of the hierarchy requests (after `DeleteProject`) and the last entry of `REQUEST_NAMES`; `RepoPathsInferred` follows `Deleted`. |
| D144 | The Postgres case compares the written row's box with `backend.box_info()`, not with a constant (P-9). |

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| R-54 | A global `url.<base>.insteadOf` rewrite changes what `gix` reports as a fetch URL, so the key misses the repo's. | Low | A miss falls to the name rung, which OQ-31 guards; `b` corrects any wrong row, and the path is on screen. |
| R-55 | Two repos with one name in two projects of one workspace, both without a remote: the first in tree order takes a single same-named checkout (D134's sequential held set). | Low | Deterministic and named in the notice with its path; a remote on either repo decides it by rung 1; `b` corrects it. |
| R-56 | A superseded preview is aborted (`a_second_preview_from_one_origin_aborts_the_first`) while its `spawn_blocking` read runs; the blocking thread finishes on its own. | Low | Read-only, bounded by `excerpt_max_scan_files`; its result is dropped. |
| R-57 | The follow-up inference holds `busy`, so a write key pressed during the walk is refused with "`infer_repo_paths` is still in flight". | Low | The walk is bounded (R-45); the refusal names itself; `r` still re-reads. |
| R-58 | `gix::open` refuses a checkout owned by another user (`safe.directory` rules). | Low | It has no remote keys, so only the name rung can pick it; the report says why nothing was written. |

`HANDOFF.md` (the four pins at `:39-42`, the MOD-7 entry), `docs/**` (ANA-5 §4.5's "no reader, no
writer"), the PRD's recorded disagreements (plan §"Where the PRD, HANDOFF or tree disagree", items
1–10) and the ANA-5 open 8 decision are the main thread's to update at close.
