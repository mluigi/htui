# Blueprint: MOD-89 — env files in a session's trees: refuse for provider projects, warn otherwise

**Architect: code-architect. Read-only except this file. No cargo command was run; every compile
fact below was read off the tree at `8add8d9d` (branch `hr/MOD-89`) with `sed`/`grep`, not seen in
a build.** Spec: `.claude/plans/mod-89-env-files.plan.md` (D1–D7 settled by the maintainer
2026-10-13; nothing here reopens them). Precedent for T2: MOD-33's `box_hostname`, `e033470a`.
Line numbers are at `8add8d9d`.

House rules restated: rustfmt `max_width = 100`; clippy `all = warn`, pedantic off, so `-D
warnings` enforces `too_many_arguments` (> 7; override is `#[expect(…, reason = "…")]`). `pub fn`
returning a value carries `#[must_use]`. Every implementer: test first; commit incrementally,
staging only your own paths by name (no `git add -A`, no `git stash`, no `--amend`, no push);
every message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Run
`df -h .` before a build; a worktree `target/` is about 10 GB. Never use the literal `zeta`
anywhere (htui-agent's extensibility test fails on it).

---

## Execution order

```
T1 (scanner + Isolator::env_files) ─┐
                                     ├─► T3 (engine check, run failure, note) ─► T4 (docs)
T2 (allow_env_files registry key) ──┘
```

T1 and T2 are file-disjoint and each leaves the workspace green (see "T1/T2 independence"). T3
needs T1's `EnvFiles`, `Isolator::env_files`, `FakeIsolator::script_env_files` and T2's
`settings::resolve_allow_env_files`. T4 is docs only. Serial order when disk is short: T1, T2,
T3, T4.

---

## Plan deltas (read before starting)

1. **False claim: "the engine's store can read an item's notes".** `ReadStore::notes` exists
   (`htui-core/src/store/traits.rs:107`), but the engine is bound by
   `S: htui_core::store::WorkerStore`, a separate narrow trait (`htui-core/src/store/worker.rs:141`)
   whose 13 `ReadStore` reads do **not** include `notes`. The D6 dedupe cannot compile without
   it. **Resolution (T3.1):** add `WorkerStore::notes` and three one-line forwarding impls
   (`MemStore` in `htui-core/src/store/worker.rs`, `PgStore` and `Writer` in
   `htui-store/src/worker.rs`). No new SQL, no `.sqlx` entry: they forward to the existing
   `ReadStore::notes`. T3's file set grows by those two files; they are still disjoint from T1 and
   T2.
2. **Wrong snapshot path.** The plan names `crates/htui/src/ui/tabs/settings/snapshots/*`, which
   does not exist. The Settings > Prompt snapshots are
   `crates/htui/tests/snapshots/prompt_settings__{demo,clamped,editor_fraction,stale}.snap` (the
   four that hold the project group; `app_only` and `offline` must not change).
3. **`crates/htui/src/prompt_settings.rs` needs no change.** `project_keys()` (`:114-118`) filters
   `SettingKey::ALL` by `Rungs::PROJECT`, so the new key joins every project group, first,
   untouched. MOD-33 edited this file only to switch the App loop to `app_keys()`, which is done.
4. **Verb signature.** The plan's `env_files(&self, roots: &[PathBuf]) -> IsolatorFuture<EnvFiles>`
   lacks the trait's `'a` lifetime and drops which root is the `cwd`, which D7's prefix rule needs
   (an `extra_dirs` path is prefixed with that root's last component, a `cwd` path is not). The
   verb is `env_files<'a>(&'a self, cwd: &'a Path, extra_dirs: &'a [PathBuf]) ->
   IsolatorFuture<'a, EnvFiles>` (T1).
5. **"Check in `drive_group` before the first candidate" is not possible.** A candidate's trees do
   not exist until its own `prepare` (`candidate_live`, `engine.rs:4343-4357`); `worktree` and
   `copy` create them there. The secrets check can sit at the group's entry
   (`drive_group`, `engine.rs:4058-4070`) only because resolution needs no tree. The blueprint's
   resolution is a **fold after the group's `join_all`** (T3, decision F-1 below): a group every
   candidate of which was refused over env files fails the run with
   `RunFailure::EnvFilesRefused`. Without it, acceptance line 1 is false for a fan-out phase (the
   run would read `no_surviving_candidate: <phase>` or park), and a `never`-gated group would retry,
   re-preparing every tree (a `copy` re-copies the checkout) only to refuse again.
6. **A plain-step refusal is later than the secrets refusal.** `drive_once` runs after stage 2
   (`prepare`, `upsert_step_tree`, `record_commits`), stage 3 (`set_step_prompt`) and
   `open_recorder` (seq-0 prompt row). "Fails before any agent starts" holds (no `driver.start`),
   but the trees, the trim record and the prompt row exist. T3's tests must **not** copy the
   secrets table's `trim_record == None`, "no event" or `prepares() == 0` asserts
   (`engine.rs:20248-20258`).
7. **`IsolatorConfig.copy_exclude` is always empty in production** (`htui-worker/src/runtime.rs:501`),
   so the scan's prune list is `DEFAULT_COPY_EXCLUDE` in practice, exactly as `copy` mode's is. Not a
   behaviour problem; T4's doc says "the default build-directory list".
8. **Settings test anchor.** The plan's `settings.rs:935` is the assertion message inside
   `box_hostname_is_a_project_only_boolean` (`:912`, assertion `:932-936`); that assertion moves to
   the new key's test. Correct otherwise.
9. **D2 fail-open reaches the engine too.** `GixIsolator::env_files` itself never errs except a
   `spawn_blocking` join failure (`IsolateError::Git`). The engine treats any `Err` from the verb
   as "nothing found" plus a `warn`, never `?` (which would fail the step through
   `EngineError::Isolate`).

### Pins where D1/D7 leave a reading open (not reopened; recorded so implementers agree)

- `<suffix>` is everything after `.env.`, compared whole: `.env.local.example` **is** a finding
  (its suffix `local.example` is not a template word); `.env.` (empty suffix) is not.
- The exclude list prunes **directories only** in the scan. A project that lists `.env*` to keep
  env files out of a copy must not hide a real `.env` from the check in `local` mode. `.git` is
  pruned whatever its type.
- `and N more` follows the fifth item with a space, no comma: `` `a`, `b`, `c`, `d`, `e` and 2 more``.
- `(scan cut at 100000 entries)` is appended to `<list>` (one space before it), so both sentences
  carry it in the same place.
- A control character in a path renders as `?` (Linux allows `\n` in a file name; `run.failure`
  and a note are single-line in the TUI).
- `<provider>` is `project.secret_provider` verbatim (`infisical`), not backticked (D7's text).

---

## Decision F-1 (blueprint-time risk): fan-out

**Recommendation: no check in `drive_group` before the first candidate; fold the refusals after
the group's `join_all`.** Minimal shape, all in `engine.rs`:

- `run_candidate` (`:4268`) returns `Result<Option<RunFailure>, EngineError>`. `Some(failure)`
  only when `candidate_live` failed with `EngineError::EnvFiles { provider, files }` and
  `fail_candidate` ran; `Ok(None)` on every other path, success included. Its note is unchanged
  (`fan-out candidate <i> of `<phase>` attempt <n>: env_files_refused: …`, via `failure_text`).
- `drive_group`'s tail (`:4127-4138`) splits the settled results: errors still go through
  `group_error` first (cancel > fence > first, unchanged); then, when the slot's pending
  candidates **all** answered `Some(_)`, the run fails with the first failure through a new boxed
  helper `fail_refused_group(run, phase, failure) -> Result<Rest, EngineError>`:
  `finish_run(run, Failed, Some(&failure.to_string()), now)`, `cleanup_run(run)`, and
  `Rest { run: Failed, position: Some(phase.position), failure: Some(failure) }`. That is the
  `Route::GroupFailed` terminal arm's write (`:4606-4618`) and `fail_group_before_a_token`'s
  `block = false` arm (`:4243-4248`): the item mirrors `failed`, it is not blocked, no retry, no
  selection.
- A mixed group (some refused, some not) routes as today; a refused candidate is one failed
  sibling (D48).

Why this and not the alternatives: the secrets pre-check works at the group entry because it
needs no tree; this check needs the trees `prepare` makes, so the earliest point that can name the
files is each candidate. Leaving it per-candidate only (no fold) makes the run read
`no_surviving_candidate` and, under `never`, retires the slot and re-prepares every tree for a
refusal that is deterministic until a human removes the file. The judge needs nothing: it runs in
a fresh scratch `cwd` (`judge_sessions`, `prepare(…, &[], Local, None)`, `:5262-5266`), which can
hold no env file; were it ever to refuse, `judge_sessions`' catch-all parks the selection as
`judge_session_failed`, unchanged.

---

## T1 — the scanner and `Isolator::env_files`

**Files (complete):** `crates/htui-orch/src/isolate/env_files.rs` (new),
`crates/htui-orch/src/isolate.rs`, `crates/htui-orch/src/isolate/real.rs`,
`crates/htui-orch/src/fake.rs`, `crates/htui-orch/tests/gix_isolator.rs`.

### `isolate/env_files.rs` (new, synchronous, no store, no `gix`)

Module doc: MOD-89 D1, D2, D7; "file names only, never contents"; "fails open: an unreadable
entry is skipped with a `warn`, a missing root answers nothing; the trust boundary stays
`SessionSpec.env` (MOD-10)"; "called by `GixIsolator::env_files` under `spawn_blocking`".

```rust
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use super::copy::Exclude;

/// D2: entries the walk visits across all roots before it stops and reports `truncated`.
pub const SCAN_CAP: usize = 100_000;
/// D7: how many paths a sentence names before `and N more`.
pub const LISTED: usize = 5;
/// D1: `.env.<suffix>` suffixes that are committed templates, compared ASCII case-insensitively.
pub const TEMPLATE_SUFFIXES: [&str; 6] =
    ["example", "sample", "template", "dist", "defaults", "schema"];

/// What [`scan`] found (MOD-89 D2): root-relative, `/`-separated, sorted by bytes, deduplicated;
/// a path under an `extra_dirs` root carries that root's last component first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvFiles {
    /// The paths.
    pub paths: Vec<String>,
    /// The walk stopped at [`SCAN_CAP`] entries; more may exist.
    pub truncated: bool,
}

impl EnvFiles {
    #[must_use] pub fn is_empty(&self) -> bool { self.paths.is_empty() }
}

/// D1: `.env`, `.envrc`, or `.env.<suffix>` with a non-empty suffix that is not a
/// [`TEMPLATE_SUFFIXES`] word; ASCII case-insensitive.
#[must_use] pub fn is_env_file(name: &str) -> bool;

/// D2: walks `cwd`, then each of `extra_dirs`, for env files. Never fails.
#[must_use] pub fn scan(cwd: &Path, extra_dirs: &[PathBuf], excludes: &[Exclude], cap: usize)
    -> EnvFiles;

/// D7's `<list>`: the first [`LISTED`] paths backticked and joined with `, `, then
/// ` and {n} more`, then ` (scan cut at {SCAN_CAP} entries)` when truncated.
#[must_use] pub fn listed(files: &EnvFiles) -> String;

/// D7's refusal sentence.
#[must_use] pub fn refused(provider: &str, files: &EnvFiles) -> String {
    format!(
        "env_files_refused: {} in the session's trees; the project resolves its secrets from \
         {provider}. Remove them, or set allow_env_files.",
        listed(files)
    )
}

/// D7's warning sentence (the item note's body).
#[must_use] pub fn warning(files: &EnvFiles) -> String {
    format!("env files in the session's trees: {}; the agent can read them", listed(files))
}
```

`scan` body, in order:

1. `let mut found = Vec::new(); let mut visited = 0_usize; let mut truncated = false;`
2. Roots: `cwd` with prefix `None`; each `extra_dirs` entry with prefix
   `dir.file_name().map(|n| n.to_string_lossy().into_owned())` (a root with no last component
   gets no prefix).
3. Per root: `if !root.is_dir() { continue; }` (missing root answers nothing; `is_dir` follows a
   symlinked root, as `WalkDir` does for its root). Then
   `WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| keep(e, excludes))` with
   `keep`: depth 0 → `true`; name `== ".git"` → `false`; `e.file_type().is_dir()` and
   (`is_env_file(name)` or any `exclude.matches(name)`) → `false`; else `true`.
4. Per item: `Err(err)` → `tracing::warn!(path = ?err.path(), error = %err, "the env-file scan
   skipped an entry it could not read")`, continue. `Ok(entry)`: if `visited == cap` → `truncated =
   true`, stop **all** roots; else `visited += 1`. Skip `entry.depth() == 0` and any
   `file_type().is_dir()`. If `is_env_file(&entry.file_name().to_string_lossy())`: the path is
   `entry.path().strip_prefix(root)`'s components, each `to_string_lossy`, joined with `/`, with
   `"{prefix}/"` in front when the root has one. A symlink is never followed (`follow_links(false)`)
   and is recorded when its own name matches.
5. `if truncated { tracing::warn!(cap, "the env-file scan stopped at its cap"); }`;
   `found.sort(); found.dedup();` (byte order: `String`'s `Ord`).

`listed`: each path with `char::is_control` mapped to `?`, wrapped in backticks; first `LISTED`
joined `", "`; if `paths.len() > LISTED`, `format!(" and {} more", len - LISTED)`; if
`truncated`, `format!(" (scan cut at {SCAN_CAP} entries)")`.

### `isolate.rs`

- `pub mod env_files;` after `pub mod copy;` (`:21`); `pub use env_files::EnvFiles;` beside
  `pub use real::…` (`:26`).
- Trait doc (`:135-137`): "four verbs plus **four** reads (milestone 4's two, `changed_paths`,
  MOD-9 D119, and `env_files`, MOD-89) …, and all **ten** sit behind plan D6's seam."
- New verb after `changed_paths` (`:206-210`), before `reconcile`'s doc (`:212`):

```rust
/// MOD-89 D1, D2: the env files a session started in `cwd` with `extra_dirs` could read, by
/// root-relative name; never their contents. A read: no guard, no `git` child, no store. A
/// missing root answers nothing and an unreadable entry is skipped, so a real isolator fails
/// open; the engine also reads an `Err` as "nothing found" (D2).
fn env_files<'a>(
    &'a self,
    cwd: &'a Path,
    extra_dirs: &'a [PathBuf],
) -> IsolatorFuture<'a, EnvFiles>;
```

  Import `std::path::Path` (`:14` imports `PathBuf` only).

### `isolate/real.rs`

Import `super::env_files::{self, EnvFiles}`. In `impl Isolator for GixIsolator`, after
`changed_paths` (ends `:1564`), before `reconcile` (`:1566`):

```rust
/// MOD-89 D2: [`env_files::scan`] on the blocking pool, pruning `.git` and
/// [`copy::excludes`] of the configured list — `copy`'s own, so a `local` step on a large
/// checkout skips `target/` and `node_modules/`.
fn env_files<'a>(&'a self, cwd: &'a Path, extra_dirs: &'a [PathBuf]) -> IsolatorFuture<'a, EnvFiles> {
    Box::pin(async move {
        let excludes = copy::excludes(&self.config.copy_exclude);
        let (cwd, extra_dirs) = (cwd.to_path_buf(), extra_dirs.to_vec());
        blocking(move || Ok(env_files::scan(&cwd, &extra_dirs, &excludes, env_files::SCAN_CAP)))
            .await
    })
}
```

(`blocking` is `git::blocking`, already imported at `:21`; `Exclude: Clone + Send`.)

### `fake.rs` (`FakeIsolator`)

- Imports: `crate::isolate::EnvFiles`, `std::path::Path`.
- Fields after `tree_root` (`:137`), house doc style:
  `env_file_answers: Mutex<VecDeque<std::result::Result<EnvFiles, IsolateError>>>` ("MOD-89:
  scripted `env_files` answers, FIFO, one consumed per call; unscripted is `EnvFiles::default()`")
  and `env_file_requests: Mutex<Vec<(PathBuf, Vec<PathBuf>)>>` ("every call's `(cwd,
  extra_dirs)`, in call order"). Not named `env_files`: a field and the trait method of one name
  read badly.
- Methods after `changed_path_requests` (`:283-289`): `pub fn script_env_files(&self, files:
  EnvFiles)`, `pub fn fail_env_files(&self, reason: &str)` (pushes `Err(IsolateError::Git(…))`),
  `#[must_use] pub fn env_file_requests(&self) -> Vec<(PathBuf, Vec<PathBuf>)>`.
- Impl after `changed_paths` (`:649-668`): record the request, then
  `pop_front().unwrap_or_else(|| Ok(EnvFiles::default()))`. Touches no filesystem (module rule,
  `fake.rs:4-5`).

### `tests/gix_isolator.rs`

- Import `EnvFiles` from `htui_orch::isolate` (`:46-49`).
- `StallAfterReconcile` delegates after `changed_paths` (`:2040-2046`):
  `fn env_files<'a>(&'a self, cwd: &'a Path, extra_dirs: &'a [PathBuf]) ->
  IsolatorFuture<'a, EnvFiles> { self.inner.env_files(cwd, extra_dirs) }`.

### Tests first

`env_files.rs` `#[cfg(test)] mod tests` (tempdirs via `tempfile`, a `touch(root, rel)` helper that
creates parents; mirror `copy.rs`'s `lay_out` `:455`):

| Test | Asserts |
|---|---|
| `env_file_names_are_the_dotenv_family_and_direnv` | `true`: `.env`, `.ENV`, `.Env.Local`, `.env.local`, `.env.production`, `.envrc`, `.ENVRC`, `.env.local.example`. `false`: `.env.example`, `.env.sample`, `.env.template`, `.env.dist`, `.env.defaults`, `.env.schema`, `.ENV.EXAMPLE`, `.env.`, `env`, `.environment`, `x.env`, `.envrc.bak`, `.venv`. |
| `a_nested_env_file_is_found_by_its_root_relative_path` | `.env`, `web/.env.local`, `web/src/app.ts` → `paths == [".env", "web/.env.local"]`, `!truncated`. |
| `git_and_the_build_directories_are_pruned` | with `copy::excludes(&[])`: `.git/.env`, `node_modules/pkg/.env`, `target/.env`, `crate/target/.env.local` → empty. |
| `a_directory_named_like_an_env_file_is_neither_found_nor_entered` | dirs `.env/` holding `.env`, and `.envrc/` holding `x` → empty. |
| `a_project_exclude_prunes_directories_only` | `excludes(&[".env*".into()])`: file `.env` found, `.envdir/.env` not. |
| `a_symlink_named_env_is_found_and_not_followed` (`#[cfg(unix)]`) | `.env` → symlink to a dir holding `.env.production`; `link` → symlink to a dir holding `.env` → `paths == [".env"]`. |
| `the_cap_cuts_the_walk_and_says_so` | ten files `a0..a9` and `.env`, `cap = 3` → `truncated`; `cap = SCAN_CAP` → `.env` found, `!truncated`. |
| `a_missing_root_answers_nothing` | `cwd` and an extra dir that do not exist → `EnvFiles::default()`. |
| `an_extra_dir_is_named_by_its_last_component` | `cwd/.env`, `other/docs/.envrc` (extra `other/docs`) → `[".env", "docs/.envrc"]`. |
| `an_unreadable_directory_is_skipped` (`#[cfg(unix)]`) | `locked/` mode `0o000` beside `.env` → `[".env"]`; restore mode before drop. If `read_dir(locked)` still succeeds (running as root), return early: root ignores the mode. |
| `the_list_names_five_then_counts_the_rest` | 1 path → `` `.env` ``; 5 → five, no tail; 7 → `` `a`, `b`, `c`, `d`, `e` and 2 more``; truncated → ` (scan cut at 100000 entries)` appended; `"a\nb/.env"` → `` `a?b/.env` ``. |
| `the_sentences_are_d7s_bytes` | `refused("infisical", [".env", "web/.env.local"])` ==  `` env_files_refused: `.env`, `web/.env.local` in the session's trees; the project resolves its secrets from infisical. Remove them, or set allow_env_files.``; `warning(same)` == `` env files in the session's trees: `.env`, `web/.env.local`; the agent can read them``. |

`fake.rs` tests (`mod tests`, `:2415`):
`env_files_answers_its_script_then_nothing_and_records_the_roots` — two scripted answers then an
unscripted call (`default`), a `fail_env_files` answer is `Err(IsolateError::Git(_))`, and
`env_file_requests()` lists every `(cwd, extra_dirs)` in order.

`tests/gix_isolator.rs`, each over `Fixture::new(mode, None)` (`:155`), with a helper
`ignore_env(fix)` = `commit_file(&fix.core.path, ".gitignore", ".env\n", "ignore env")` then
`std::fs::write(fix.core.path.join(".env"), "A=1\n")`, and `prepare` called directly as
`committed_worktree_step` does (`:2737-2765`); `worktree`/`copy` cases open with
`let Some(_git) = skip_without_git!() else { return };` (`local` needs no `git` CLI, `:586-590`):

| Test | Asserts |
|---|---|
| `a_local_step_s_trees_hold_a_gitignored_env_file` | scope `[core, docs]`, `Local`: `env_files(&prepared.cwd, &prepared.extra_dirs) == [".env"]`. |
| `a_local_step_names_the_second_repo_s_env_file_by_its_name` | `.env` in `docs` only: `["docs/.env"]` (`docs` is an `extra_dirs` root, D28). |
| `a_copy_step_carries_a_gitignored_env_file` | `Copy`, scope `[core]`: `["core/.env"]` (`cwd` is `<root>/<run>/<step>`, the copy is `core/`). |
| `a_worktree_step_omits_a_gitignored_env_file` | `Worktree`: empty. |
| `a_worktree_step_holds_a_committed_env_file` | `commit_file(core, ".env", "A=1\n", …)` before `prepare`, `Worktree`: `["core/.env"]`. |

### Commits

1. `feat(mod-89): an env-file scanner over a session's trees` — `env_files.rs` with its tests, and
   `pub mod env_files;` + `pub use env_files::EnvFiles;` in `isolate.rs`. Green alone.
2. `feat(mod-89): Isolator::env_files, real and fake` — the trait verb, `GixIsolator`,
   `FakeIsolator` (+ its test), `StallAfterReconcile`, the five `gix_isolator` cases. One commit:
   the trait method breaks all three impls until each has it.

### Verify

```bash
cargo test -p htui-orch --all-features --lib isolate::env_files -- --test-threads=1
cargo test -p htui-orch --all-features --lib fake:: -- --test-threads=1
cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1
cargo clippy -p htui-orch --all-features --all-targets -- -D warnings
cargo clippy -p htui-orch -- -D warnings      # featureless gate
cargo fmt --all -- --check
```

Plus the plan's one measurement: a throwaway `#[ignore]`d test (deleted before commit 2, or kept
`#[ignore]`) running `scan` over `env!("CARGO_MANIFEST_DIR")/../..` with `copy::excludes(&[])`,
printing `visited`/elapsed; record the number in commit 2's body.

### Hazards

- Count the cap on yielded entries **after** pruning, across all roots together (one budget per
  session).
- Never read a file's contents, metadata beyond `file_type`, or follow a link.
- `copy::excludes` warns once per bad entry per call. Production's list is empty (plan delta 7),
  so this is silent in practice.

---

## T2 — the `allow_env_files` key (D4)

**Files (complete):** `crates/htui-core/src/prompt/settings.rs`,
`crates/htui/src/ui/tabs/settings/prompt.rs`, `crates/htui/tests/prompt_settings.rs`,
`crates/htui/tests/snapshots/prompt_settings__{demo,clamped,editor_fraction,stale}.snap`.
(`crates/htui/src/prompt_settings.rs`: no change, plan delta 3.)

### `settings.rs`

- **New constant** after `BOX_HOSTNAME_DEFAULT` (`:49-51`):
  ```rust
  /// `project.settings.allow_env_files`'s compiled default: a provider project whose session
  /// trees hold env files is refused (MOD-89 D3, D4). Not a field of [`Defaults`]: no App rung.
  pub const ALLOW_ENV_FILES_DEFAULT: bool = false;
  ```
- **`SettingKey`** (`:149-181`): new first variant `AllowEnvFiles = 0`, doc "`allow_env_files`,
  MOD-89 D4's project-only switch: on, a provider project whose session trees hold env files
  warns instead of refusing." Every other discriminant +1 (`BoxHostname = 1` … `TokenBudget =
  11`). `"allow_env_files"` < `"box_hostname"` bytewise, so byte order and discriminant order
  still coincide. Enum doc: "The twelve keys of the registry — ANA-5 §5.3's ten `app_setting` keys
  and two project-only switches, MOD-33's `box_hostname` and MOD-89's `allow_env_files` —
  declared in **key byte order**: the discriminant indexes [`SPECS`]." `BoxHostname`'s doc
  (`:158-159`): "`box_hostname`, a [`SettingKind::Boolean`] only a project holds (MOD-33 D267)."
- **`ALL`** (`:185`): `[Self; 12]`, `AllowEnvFiles` first. `from_key` doc: "outside the twelve".
- **`SettingKind`** doc (`:293-295`): "nine keys are integers, `prompt_reserve_fraction` is a
  float …, and the two switches are booleans."
- **`SPECS`** (`:344`): `[SettingSpec; 12]`, new entry **at index 0**:
  ```rust
  SettingSpec {
      key: "allow_env_files",
      project_key: Some("allow_env_files"),
      kind: SettingKind::Boolean,
      min: 0,
      max: 1,
      rungs: Rungs::PROJECT,
      not_above: None,
      unit: "on/off",
      doc: "A provider project whose session trees hold env files warns instead of refusing.",
  },
  ```
- **Exhaustive matches**: `Defaults::value_of` (`:104`):
  `SettingKey::AllowEnvFiles => Value::Bool(ALLOW_ENV_FILES_DEFAULT),`; `Defaults::integer`
  (`:128`): `SettingKey::AllowEnvFiles => i64::from(ALLOW_ENV_FILES_DEFAULT),`. `validate` needs
  nothing: its `SettingKind::Boolean` arm is generic.
- **Resolver** after `resolve_box_hostname` (`:762-773`):
  ```rust
  /// `project.settings.allow_env_files` (MOD-89 D4): whether a provider project whose session
  /// trees hold env files warns instead of refusing. The house fall-through rule: absent, `null`
  /// or anything but a JSON boolean is not a value, and [`ALLOW_ENV_FILES_DEFAULT`] (off)
  /// answers. Project rung only. No effect on a provider-less project, which only warns (D3).
  #[must_use]
  pub fn resolve_allow_env_files(project: Option<&Value>) -> bool {
      project_key(project, "allow_env_files")
          .and_then(Value::as_bool)
          .unwrap_or(ALLOW_ENV_FILES_DEFAULT)
  }
  ```

### `ui/tabs/settings/prompt.rs`

- Import `resolve_allow_env_files` with the other resolvers (`:35-37`).
- `effective` (`:987`): new arm **before** `SettingKey::BoxHostname` (`:998`), identical in shape:
  `resolve_allow_env_files(project)`, source `BudgetSource::Project.as_str()` only when
  `project_value.and_then(Value::as_bool).is_some()` else `COMPILED_DEFAULT`, text
  `SWITCH_ON`/`SWITCH_OFF`, `number: None`. Optional: fold both arms through one private
  `fn switch(on: bool, project_value: Option<&Value>) -> (String, Option<i64>, &'static str)`.
- Nothing else: `submit`'s Boolean parse (`:573`) and the pane's `on or off · rungs project`
  line (`:685`) are kind-driven. **Never spell `"allow_env_files"` in this file**
  (`no_key_name_is_spelled_in_the_section`, `tests/prompt_settings.rs:974`).
- In-file test beside `the_switch_source_is_project_only_for_a_stored_boolean` (`:1257`):
  `the_env_files_switch_is_off_unless_a_project_stores_true` — `Some(true)` → `on`/`project`;
  `Some(false)` → `off`/`project`; `Some("on")`, `Some(1)`, `Some(Null)`, `None` → `off`/`default`.

### Tests first

`settings.rs` (tests module `:816`):

| Test | Change |
|---|---|
| `allow_env_files_is_a_project_only_boolean` (new) | the spec fields exactly as above (doc non-empty); `rung_refusal(AllowEnvFiles, APP)` and `(…, PHASE)` are `Some`, `(…, PROJECT)` `None`; `SettingKey::ALL[0] == AllowEnvFiles`, "`allow_env_files` sorts first in key byte order". |
| `box_hostname_is_a_project_only_boolean` (`:912`) | the last assertion (`:932-936`) becomes `ALL[1] == BoxHostname`, "second, after `allow_env_files`". |
| `validate_takes_a_boolean_and_nothing_else` (`:941`) | loop the body over `[AllowEnvFiles, BoxHostname]`; refusal prefix `` `{key}` must be a JSON boolean, got ``. |
| `resolve_allow_env_files_falls_through_to_off` (new) | `false` for `None`, `{}`, `{"allow_env_files": null}`, `1`, `"on"`, `"true"`, `"not an object"`, `{"allow_env_files": false}`; `true` only for `{"allow_env_files": true}`. |
| `app_keys_are_the_ten_app_setting_keys_in_byte_order` (`:986`) | `!app.contains(&AllowEnvFiles)` too; the comparison filters `!matches!(key, AllowEnvFiles | BoxHostname)`. |
| `specs_are_indexed_by_discriminant` (`:1088`) | logic unchanged; message "twelve distinct keys". |
| `every_default_validates_under_its_own_spec`, `the_defaults_are_migration_0002s_ten_rows_verbatim`, `as_rows_is_key_byte_order`, `ranges_are_the_readers_clamps`, `project_key_is_some_exactly_where_project_is_accepted` | **unchanged**, must stay green. |

`crates/htui/tests/prompt_settings.rs`:

| Test | Change |
|---|---|
| `the_demo_snapshot_has_ten_app_entries_and_three_keys_per_project` (`:157`) | rename `…_and_four_keys_per_project`; `value_keys == [AllowEnvFiles, BoxHostname, UpstreamHops, TokenBudget]`; `values[0..=2]` `None`, `values[3] == Some(json!(120_000))`; doc (`:151`) "`allow_env_files` first". |
| `set_setting_on_project_stores_under_the_project_key_and_keeps_foreign_keys` (`:384-385`) | `values[1]` → `values[2]`. |
| `set_setting_box_hostname_on_a_project_stores_a_boolean_and_keeps_foreign_keys` (`:422-423`) | `values[0]` → `values[1]`. |
| `set_setting_allow_env_files_on_a_project_stores_a_boolean_and_keeps_foreign_keys` (new, shape of `:395`) | `SetSetting { rung: Project(PROJECT_VULKAN), key: AllowEnvFiles, value: json!(true), expected: Some(token) }` → blob `allow_env_files == true`, `retention_days`/`keep_raw_events`/`token_budget` kept, `values[0] == (AllowEnvFiles, Some(json!(true)))`, `resolve_allow_env_files(Some(blob))`, token moved. |
| `allow_env_files_is_refused_on_the_app_rung` (new, shape of `:488`) | `SetSetting { rung: SettingRung::App, key: AllowEnvFiles, value: json!(true), expected: None }` → `refusal(..)` is `("set_setting", m)` with `m` containing `` `allow_env_files` is not accepted on the app rung``. |
| `the_demo_snapshot_renders_the_tree` (`:831`) | add `row_of(&frame, SettingKey::AllowEnvFiles, 0).contains("unset | off (default)")`. |
| `e_then_on_then_enter_sends_set_setting_true_for_allow_env_files` (new, shape of `:1048`) | `move_to(project_row(AllowEnvFiles))`, `e`, type `on`, `enter` → one `SetSetting { rung: Project(PROJECT_VULKAN), key: AllowEnvFiles, value: Value::Bool(true), expected: Some(project_token) }`. |

Every other test computes rows through `app_row`/`project_row`/`project_keys()` (`:700-713`) and
moves by itself.

### Snapshot rule

`demo`, `clamped`, `editor_fraction`, `stale` each gain exactly one line as the **first row under
`project Vulkan Tutorials`**, and lose one trailing blank line of padding; nothing else moves.
The line is two spaces, `allow_env_files`, 15 spaces (pad to the 28-cell `KEY_WIDTH` of
`excerpt_provider_deadline_ms`, then the two-space gap), and
`unset | off (default)   on/off`:

```
│  allow_env_files               unset | off (default)   on/off                                    │
```

`app_only` and `offline` must produce no `.snap.new` (a key leaking into the App group would).

### Commit

One commit (the new variant breaks `prompt.rs`'s exhaustive `effective` match, so core and UI
cannot land apart): `feat(mod-89): allow_env_files, a per-project switch in Settings > Prompt`.
Stage the three source/test files and the four `.snap` files by path.

### Verify

```bash
cargo test -p htui-core --all-features --lib prompt::settings -- --test-threads=1
cargo test -p htui --all-features --lib ui::tabs::settings::prompt -- --test-threads=1
cargo insta test -p htui --all-features --test prompt_settings -- --test-threads=1   # .snap.new
cargo insta pending-list   # read each diff against the rule above, then:
cargo insta accept
cargo insta test -p htui --all-features -- --test-threads=1   # full run: no other .snap.new
find crates -name '*.snap.new' | wc -l                        # 0
cargo test -p htui-core --all-features -- --test-threads=1
cargo clippy -p htui-core -p htui --all-features --all-targets -- -D warnings
cargo clippy -p htui-core -p htui -- -D warnings              # featureless gate
cargo fmt --all -- --check
```

The `prompt_settings` Postgres-gated case needs `HTUI_TEST_DATABASE_URL`; an hr sandbox has no
database, so it is the host's to run.

### Hazards

- **Discriminant shift.** `SPECS[0]` must be the new entry; anywhere else misindexes every key.
  `specs_are_indexed_by_discriminant` is the guard. The discriminant indexes `SPECS` only
  (`settings.rs:211`, test `:1091`); `SettingKey` derives no `Serialize` and is never persisted.
  No other crate names a discriminant or matches `SettingKey` exhaustively; they name variants
  (`TokenBudget`, `UpstreamHops`, …), so htui-store, htui-agent and htui-orch need no edit.
- `number: None` for the switch, or D13's clamp arithmetic reads a boolean.
- The source label is `(default)`, never `app_setting_default` (D276's rule, MOD-33).

---

## T3 — the engine check, the typed failure, the deduplicated note

**Files (complete):** `crates/htui-core/src/store/worker.rs`, `crates/htui-store/src/worker.rs`,
`crates/htui-orch/src/command.rs`, `crates/htui-orch/src/status.rs`,
`crates/htui-orch/src/engine.rs`, `crates/htui-orch/tests/gix_isolator.rs` (one end-to-end case).

### T3.1 `WorkerStore::notes` (plan delta 1)

- `htui-core/src/store/worker.rs`, trait, after `resolve_inputs` (`:181-186`), before
  `// -- 5 WriteStore reads` (`:188`):
  ```rust
  /// [`ReadStore::notes`]: MOD-89 D6's dedupe reads the item's notes before it writes one.
  fn notes(&self, id: ItemId) -> impl Future<Output = Result<Vec<Note>>> + Send;
  ```
  Section comment `// -- 13 ReadStore reads` (`:142`) → `14`; module doc (`:15`) "`WorkerStore`
  is 60 methods: 14 [`ReadStore`] reads, …".
- Forwarders `async fn notes(&self, id: ItemId) -> Result<Vec<Note>> { ReadStore::notes(self, id).await }`
  after `resolve_inputs` in `impl WorkerStore for MemStore` (`htui-core/src/store/worker.rs:722-729`),
  `for PgStore` (`htui-store/src/worker.rs:158-165`) and `for Writer` (`:536-543`). `Note` is
  already imported in both files. No query, so no `.sqlx` entry.
- E0034 check done: no module imports `WorkerStore` by name beside `ReadStore` and calls
  `.notes(` (the one `use … WorkerStore` is `htui-mcp/src/tools/command.rs:530`, which never
  calls it).

### T3.2 the failure vocabulary

`status.rs` — `RunFailure` (`:41`, derives `Debug, Clone, PartialEq, Eq`; `EnvFiles` derives
the same), new last variant after `SecretsRefused` (`:117-121`):

```rust
/// MOD-89 D3, D6: a provider project's session trees hold env files and `allow_env_files` is
/// off, so the step failed before its driver was built. Names the files, never a value.
EnvFilesRefused {
    /// `project.secret_provider`, as the column holds it.
    provider: String,
    /// What the scan found.
    files: crate::isolate::EnvFiles,
},
```

`Display` arm after `:174`:
`Self::EnvFilesRefused { provider, files } => f.write_str(&crate::isolate::env_files::refused(provider, files)),`

`command.rs` — `EngineError`, after `Secrets` (`:531-534`):

```rust
/// MOD-89 D3, D6: the session's trees hold env files on a provider project with
/// `allow_env_files` off. Its `Display` is the run's `env_files_refused: …` sentence.
#[error("{}", crate::isolate::env_files::refused(.provider, .files))]
EnvFiles {
    /// `project.secret_provider`.
    provider: String,
    /// What the scan found.
    files: crate::isolate::EnvFiles,
},
```

(`.field` format args are the `MissingTags` variant's precedent, `:423`.) Nothing outside
`htui-orch` matches `EngineError` or `RunFailure` exhaustively (`htui-worker/src/runtime.rs:2817`
has a wildcard).

### T3.3 the engine

Imports: `std::path::{Path, PathBuf}` (`:21`), `crate::isolate::env_files`.

**`failure_text`** (`:6901-6912`), arm after `Secrets`:
`EngineError::EnvFiles { provider, files } => RunFailure::EnvFilesRefused { provider: provider.clone(), files: files.clone() }.to_string(),`

**New method** on `Engine`, beside `secrets_ready` (`:6246`):

```rust
/// MOD-89 D2-D7: the trees a session of `run` can read, scanned for env files before its
/// driver is built. A provider project refuses ([`EngineError::EnvFiles`]) unless
/// `allow_env_files` is on; any other finding is one item note per run and a `warn`. A scan
/// fault fails open (D2): logged, nothing found.
async fn check_env_files(
    &self,
    run: &Run,
    step: &RunStep,
    project: &Project,
    cwd: &Path,
    extra_dirs: &[PathBuf],
) -> Result<(), EngineError> {
    let files = match self.parts.isolator.env_files(cwd, extra_dirs).await {
        Ok(files) => files,
        Err(err) => {
            tracing::warn!(run = %run.id, step = %step.id, %err,
                "the env-file scan failed; the session starts unchecked");
            return Ok(());
        }
    };
    if files.is_empty() {
        return Ok(());
    }
    if let Some(provider) = project.secret_provider.as_deref()
        && !settings::resolve_allow_env_files(Some(&project.settings))
    {
        return Err(EngineError::EnvFiles { provider: provider.to_owned(), files });
    }
    tracing::warn!(run = %run.id, step = %step.id, files = files.paths.len(),
        truncated = files.truncated, "env files in the session's trees");
    let Some(item) = run.item_id else { return Ok(()) };
    let body = env_files::warning(&files);
    // D6: one note per run. A note of this body via any step of this run already says it.
    let steps: BTreeSet<StepId> =
        self.parts.store.run_steps(run.id).await?.iter().map(|row| row.id).collect();
    let noted = self.parts.store.notes(item).await?.iter().any(|note| {
        note.body == body && note.via_step_id.is_some_and(|id| steps.contains(&id))
    });
    if !noted {
        self.note(item, body, Some(step.id), self.now()).await?;
    }
    Ok(())
}
```

**`drive_once`** (`:6267`), directly after the `env_for` statement (`:6290-6295`), before the
policy (`:6296`):

```rust
// MOD-89 D2-D6: the trees this session can read, checked once its secrets are known (a
// secrets refusal wins) and before any tool host or driver exists. Boxed: the walk's
// frames sit near the debug stack (MOD-10 M3).
Box::pin(self.check_env_files(run, step, &project, &cwd, &extra_dirs)).await?;
```

All three callers (`session` `:6111`, `candidate_live` `:4329`, `judge_calls` `:5309`) already
finish the recorder on `drive_once`'s outer `Err` (`refused_over_finish`, `:7054`). A plain step
then lands in `walk_step`'s `fail_hard(failure_text(..))` (`:3587`): run and step `failed`,
`run.failure` = the sentence, item not blocked, no retry, and `dispatch` answers
`Err(EngineError::EnvFiles { .. })`, as the secrets refusal does.

**Fan-out fold (decision F-1).** `run_candidate` (`:4268-4305`):

```rust
async fn run_candidate(&self, stage: CandidateStage<'_>) -> Result<Option<RunFailure>, EngineError>
```

`Ok(())` from `candidate_live` → `Ok(None)`. On `Err(err)`, after the existing release and the
cancel/fence early return: `let refused = match &err { EngineError::EnvFiles { provider, files }
=> Some(RunFailure::EnvFilesRefused { provider: provider.clone(), files: files.clone() }), _ =>
None };` then `self.fail_candidate(..).await?; Ok(refused)`. Update its doc: "`Some` is MOD-89's
env-file refusal, which [`drive_group`] folds into the run's failure when every candidate met
it."

`drive_group`'s tail (`:4127-4138`):

```rust
let settled = futures::future::join_all(/* unchanged */).await;
let mut errors = Vec::new();
let mut refusals = Vec::with_capacity(settled.len());
for outcome in settled {
    match outcome {
        Ok(refusal) => refusals.push(refusal),
        Err(err) => errors.push(Err(err)),
    }
}
if let Some(err) = group_error(run.id, errors) {
    return Err(err);
}
// MOD-89 (blueprint F-1): every candidate refused over env files fails the run with the
// sentence, as a secrets refusal does at the group's entry; no retry, no selection.
if !refusals.is_empty() && refusals.iter().all(Option::is_some)
    && let Some(failure) = refusals.into_iter().flatten().next()
{
    return Box::pin(self.fail_refused_group(run, phase, failure)).await.map(Some);
}
Ok(None)
```

`group_error`'s signature is unchanged (`impl IntoIterator<Item = Result<(), EngineError>>`;
`errors: Vec<Result<(), EngineError>>`). New helper beside `fail_group_before_a_token` (`:4221`):

```rust
/// MOD-89 (blueprint F-1): every candidate of the group already failed with its note; the run
/// fails with `failure` and is cleaned up, as `Route::GroupFailed`'s terminal arm does.
async fn fail_refused_group(&self, run: &Run, phase: &SnapshotPhase, failure: RunFailure)
    -> Result<Rest, EngineError>
```

Body: `finish_run(run.id, RunStatus::Failed, Some(&failure.to_string()), self.now())`,
`self.cleanup_run(run.id)`, `Ok(Rest { run: RunStatus::Failed, position: Some(phase.position),
failure: Some(failure) })`.

Doc touch: `drive_group`'s doc (`:3995-4024`) gains one sentence naming the fold.

### Tests first

New `mod env_files` **nested inside** `mod run_secrets` (append before its closing brace,
`engine.rs:20762`), so it reaches the parent's private helpers through `super::`: `Harness`
(`:7587`, `new`, `dispatch`, `repoint`, `free_feat_3`), `provider_project` (`:20019`), `columns`,
`source`, `failing`, `feat_3_with_prd_ungated` (`:20075`), `started_run` (`:20087`), `step_at`
(`:20098`), `key` (`:20001`), `start_feat_3`, `VALUE`, `SCOPE`. The fake is
`harness.orch.isolator` (`fake.rs:1611`, public). Scripted answers are FIFO and **every**
`drive_once` consumes one (each step, each fan-out candidate, each of a judge's two calls):
script one per session. A helper `fn found(paths: &[&str]) -> EnvFiles`. Allow on a provider
project: merge `"allow_env_files": true` into the demo blob read through
`store.project_settings(project)` (the `ana_2_fanned_and_judged` pattern, `:19337-19346`), then
`store.set_project_settings`.

| # | Test | Asserts |
|---|---|---|
| 1 | `env_files_refuse_a_provider_project_s_run_before_its_driver` | provider project, `script_env_files(found([".env", "web/.env.local"]))`: `dispatch(start_feat_3())` is `Err(EngineError::EnvFiles { provider, files })` with `provider == "infisical"` and those files; run `Failed` with `failure == Some(env_files::refused("infisical", &files))`; `prd` `Failed`; `spec_for(&key("prd")).is_none()`; item not `Blocked`; no item note carries the warning body. Not asserted (plan delta 6): trim record, event log, `prepares()`. |
| 2 | `a_secrets_refusal_wins_over_env_files` | `failing(&harness, BadCredentials)` and a scripted finding: `Err(EngineError::Secrets(_))`, and `env_file_requests().is_empty()` (the walk never reached the scan). |
| 3 | `a_provider_less_project_s_run_proceeds_with_one_warning_note` | no provider, one finding `[".env"]`: the walk parks at `prd`'s gate; `spec_for(&key("prd"))` is `Some`; exactly one note with body `` env files in the session's trees: `.env`; the agent can read them`` and `via_step_id == Some(prd.id)`. |
| 4 | `a_two_phase_run_with_the_same_findings_writes_one_note` | `feat_3_with_prd_ungated`, two scripted findings (`prd`, `plan`): both sessions ran; still exactly one warning note, via `prd`. |
| 5 | `allow_env_files_turns_the_refusal_into_the_warning` | provider project + `allow_env_files: true` + one finding: the walk parks at `prd`'s gate, `spec_for` is `Some`, `spec.env` holds the resolved map, one warning note. |
| 6 | `no_findings_no_note_no_failure` | provider project, nothing scripted: the walk parks at `prd`'s gate; no note body starts with `env files in` or `env_files_refused`. |
| 7 | `a_scan_fault_fails_open` | provider project, `fail_env_files("boom")`: the walk parks at `prd`'s gate; no failure, no warning note. |
| 8 | `the_scan_reads_the_session_s_cwd_and_extra_dirs` | `script_extra_dirs(vec!["/fake/elsewhere/docs".into()])`: `env_file_requests()[0] == (PathBuf::from(format!("/fake/trees/{run}/{prd_step}")), vec!["/fake/elsewhere/docs".into()])`. |
| 9 | `a_fan_out_whose_candidates_are_all_refused_fails_the_run_with_the_sentence` | `repoint` `prd.fan_out = 2` (the `a_candidate_driver_error_holding_a_resolved_value_is_noted_masked` setup, `:20722`), provider project, two scripted findings: `dispatch` is `Ok(Started { rest, .. })` with `rest.run == Failed`, `rest.failure == Some(RunFailure::EnvFilesRefused { provider: "infisical", files })`; run row `failure` is the sentence; both candidates `Failed`, only attempt 1 exists; notes hold `` fan-out candidate {i} of `prd` attempt 1: env_files_refused: … `` for `i` in `0..2`; no candidate session started; `isolator.cleanups() == 1`. |
| 10 | `a_provider_less_fan_out_proceeds_with_the_warning` | as 9 without a provider: both candidate sessions started; the warning note count is in `1..=2`. Concurrent candidates may both read before either writes (hazard below); the range pins "at least one, never one per phase". |
| 11 | `failure_text_types_an_env_files_refusal` (sync) | `failure_text("prd", &EngineError::EnvFiles { .. })` equals `RunFailure::EnvFilesRefused { .. }.to_string()` and `err.to_string()`; the sentence contains `SettingKey::AllowEnvFiles.key()` (pins T1's literal to T2's key). |

`status.rs` `run_failure_display_is_ana2s_bytes` (`:484`), one more `assert_eq!` before its end
(`:607`): `RunFailure::EnvFilesRefused { provider: "infisical".into(), files: EnvFiles { paths:
vec![".env".into()], truncated: false } }` displays
`` env_files_refused: `.env` in the session's trees; the project resolves its secrets from infisical. Remove them, or set allow_env_files.``,
"MOD-89 D7: names the files and the provider, never a value".

`tests/gix_isolator.rs`, the real wiring end to end (no `git` CLI needed under `Local`, as
`criterion_12_a_local_step_on_a_dirty_tree` `:592`):
`a_local_run_over_a_gitignored_env_file_writes_the_warning` — `Fixture::new(Isolation::Local,
None)`, T1's `ignore_env(&fix)`, `fix.start(&fix.orch)`, `fix.parked(run)`; the item's notes hold
exactly one `` env files in the session's trees: `.env`; the agent can read them``.

Existing expectations that must stay green untouched: the whole `run_secrets` module, every
`conformance.rs` case (all fakes answer `EnvFiles::default()` unscripted), notably
`a_fan_out_whose_secrets_refuse_fails_the_run_before_any_candidate_starts` (`conformance.rs:9359`)
and `a_group_raises_a_cancel_then_a_lost_fence_then_the_first_error` (`engine.rs:18508`).

### Commits

1. `feat(mod-89): WorkerStore reads an item's notes` — T3.1 (two files).
2. `feat(mod-89): EnvFilesRefused and EngineError::EnvFiles` — T3.2 with the bytes assert.
3. `feat(mod-89): a session's trees are checked for env files before its driver` — T3.3 with tests
   1-11 and the `gix_isolator` case.

### Verify

```bash
cargo test -p htui-core --all-features --lib store:: -- --test-threads=1
cargo build -p htui-store --all-features            # SQLX_OFFLINE=true via .cargo/config.toml
cargo test -p htui-orch --all-features --lib status:: -- --test-threads=1
cargo test -p htui-orch --all-features --lib engine:: -- --test-threads=1
cargo test -p htui-orch --all-features --test gix_isolator -- --test-threads=1
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1 2>&1 \
  | grep -E "SIGABRT|FAILED|failed|stack overflow" || true
cargo test -p htui-worker --all-features -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
```

### Hazards

- **Stack.** `check_env_files` and `fail_refused_group` are boxed at their call sites; an unboxed
  `notes`/`run_steps` pair inside `drive_once` grows every walk future (MOD-10 M3's 2 MiB debug
  stack). Run the orch suite with `--no-fail-fast` and grep for `SIGABRT`.
- **Dedupe race.** Concurrent fan-out candidates of a provider-less project can each read the
  notes before either writes, so a group can leave up to `fan_out` identical notes. Sequential
  sessions (phases, retries, resumes of the same run) leave one. Accepted; T4 does not promise
  more than "one per run" for plain steps.
- Secrets first: `env_for` runs before the scan, so a run that is refused twice reads
  `secrets_refused` (and on the plain path the walk never prepares a tree).
- Fail open on the verb's `Err`; never `?` it.
- The judge is scanned too (two calls per judge); its scratch `cwd` is empty, so this costs one
  short walk and finds nothing.

---

## T4 — `docs/htui-secrets.md`

**Files (complete):** `docs/htui-secrets.md`.

A new `### Env files in the session's trees` as the **last subsection of `## At run start`**,
after `### The verify command` (`:433-447`), before `## Logins, tokens and lockout safety`
(`:449`). One cross-link line after `### When htui refuses`' bullet list (`:300-309`):
"A provider project whose session trees hold env files is refused too, see
[Env files in the session's trees](#env-files-in-the-sessions-trees)." Content, in user words
(no decision IDs in prose; the house doc style of this file):

1. **What counts** (D1): `.env`, `.env.<anything>` except the template names `.env.example`,
   `.env.sample`, `.env.template`, `.env.dist`, `.env.defaults`, `.env.schema`, and direnv's
   `.envrc`; letter case ignored; files and symlinks, never a directory (a Python `.env/`
   virtualenv is skipped and not entered). htui reads names only, never contents.
2. **Where htui looks** (D2): before every agent session of a run, the session's working
   directory and every extra directory it may read, recursively, skipping `.git` and the build
   directories `copy` mode skips (`target/`, `build/`, `cmake-build-*/`, `node_modules/`,
   `.venv/`, `out/`, `dist/`), up to 100 000 entries (a longer walk says
   `(scan cut at 100000 entries)` and logs a warning). An unreadable entry is skipped with a
   warning: the check never stops a run over an I/O error.
3. **Which mode exposes what** — a four-row table: `local` and `shared_serialized` run in the
   checkout itself, so a git-ignored `.env` is there; `copy` copies the checkout byte for byte,
   `.env` included; `worktree` checks out tracked files only, so an env file shows up only when
   it is committed. A second repository of a `local` step is named by its directory
   (`docs/.env`); under `worktree` and `copy` every path starts with the repository's name
   (`core/.env`).
4. **Refuse or warn** (D3, D4): a project with `secret_provider` set is refused, before its agent
   starts: the run's failure reads the D7 refusal sentence (quote it in a code block with an
   example). A project without a provider gets one item note per run, the D7 warning sentence
   (quoted), and the run goes on. A group of fan-out candidates that are all refused fails the
   run with the same sentence, each candidate noted as for `secrets_refused`.
5. **`allow_env_files`** (D4): Settings > Prompt, the project's group, `on`/`off` (default
   `off`); on, a provider project gets the warning instead. No effect on a project without a
   provider. For a file that holds no secret (`PORT=3000`).
6. **Clearing a refusal**: move the file out of the checkout (or delete it), commit its removal
   for `worktree` mode, or turn `allow_env_files` on; then run again.
7. **Not checked**: chats (they start in htui's own working directory, not a run's tree).

Commit: `docs(mod-89): env files in a session's trees`.

Verify: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (it checks
HANDOFF/DECISIONS structure; run it anyway as the plan asks) and read the anchor
`#env-files-in-the-sessions-trees` against GitHub's slug rule (the apostrophe drops).

---

## T1/T2 independence (they run in parallel)

- **Files.** T1 = {`htui-orch/src/isolate/env_files.rs`, `htui-orch/src/isolate.rs`,
  `htui-orch/src/isolate/real.rs`, `htui-orch/src/fake.rs`, `htui-orch/tests/gix_isolator.rs`}.
  T2 = {`htui-core/src/prompt/settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`,
  `htui/tests/prompt_settings.rs`, four `htui/tests/snapshots/prompt_settings__*.snap`}.
  Intersection empty. T3 adds `htui-core/src/store/worker.rs`, `htui-store/src/worker.rs`,
  `htui-orch/src/{command,status,engine}.rs` and appends to `gix_isolator.rs` (T1's file, but T3
  runs after T1 lands).
- **Compile coupling: none.** T1 spells `allow_env_files` as a literal inside `refused()` and
  imports nothing of T2; T3 test 11 is what ties the literal to `SettingKey::AllowEnvFiles.key()`.
  T2 changes `htui-core`, which `htui-orch` depends on, but `htui-orch` matches no `SettingKey`
  exhaustively (it calls `settings::resolve_box_hostname` by name only).
- **`SettingKey` renumber: contained.** Discriminants index `SPECS` only; no crate persists,
  serialises or names one numerically. The only exhaustive `match SettingKey` outside
  `settings.rs` is `prompt.rs`'s `effective`, in T2's own file set.
- **Snapshots.** T1 renders nothing. T2 moves exactly four `prompt_settings__*` snapshots; a full
  `cargo insta test -p htui` confirms nothing else renders the project group.
- **Shared fixtures.** Neither edits `MemStore::demo()`, `htui_core::fixtures`, or a shared test
  helper. `.sqlx`: neither adds a query (T3's `notes` forwards to an existing one).
- **Workspace.** No new dependency (`walkdir` is already `htui-orch`'s, `Cargo.toml:43`), so no
  `Cargo.lock` change. Two worktrees build two `target/` dirs (~10 GB each): check `df -h .`.

---

## Whole-item gate (after T1-T4, on the merged tree)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings
cargo test -p htui-core --all-features -- --test-threads=1
cargo test -p htui-orch --all-features --no-fail-fast -- --test-threads=1
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast 2>&1 | grep -E "SIGABRT|FAILED|failed" || true
find crates -name '*.snap.new' | wc -l      # 0
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Re-run any `htui`/`htui-orch` failure serially before calling it a regression (the keyring fake
is process-wide; `qdrant_live` times out under load). `--all-features` is required: without
`testkit`/`test-support`, `crates/htui/tests/*.rs` run zero tests and report `ok`.

## Acceptance trace

| Plan acceptance line | Where it is pinned |
|---|---|
| Provider run with `.env` fails before any agent, sentence in `run.failure` | T3 tests 1, 9; `status.rs` bytes |
| Provider-less run proceeds with one warning note per run | T3 tests 3, 4; `gix_isolator` e2e |
| `allow_env_files` downgrades the refusal to the warning | T3 test 5; T2 resolver tests |
| Templates, `.env/` directories and pruned directories are never findings | T1 name table, prune, directory tests |
| Per-mode behaviour against real git | T1's five `gix_isolator` cases |
| Docs updated, gates green, `rust-reviewer` passed | T4; whole-item gate |
