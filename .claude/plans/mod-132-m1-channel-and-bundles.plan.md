# Plan: MOD-132 M1 — Channel and bundle mechanics

**Source PRD**: `.claude/prds/mod-132-target-prepare-and-collect.prd.md`
**Selected Milestone**: 1 — Channel and bundle mechanics
**Complexity**: Medium-Large
**Status**: complete (`738f1d3c`..`1e4b30ad`, 2026-11-16). Review fix round R1a `c47d8088`, R1b `9a0d6301`; R1b's
verifier low (quarantine vs a dropped collect's blocking steps) fixed on the main thread, `1e4b30ad`. Deviations from
this text: the temp ref (P6 step 5, P8) is gone, because objects wait in a quarantine object dir until the move;
`Consent` gained `from` (the local tip). P9 was widened to nested `.cargo/` and to `rust-toolchain(.toml)`,
`package.json` and `.envrc` anywhere (maintainer, 2026-11-16).
**Routing**: PRD path, ultracode for implement accepted (`/handoff-run MOD-132`, `hr/MOD-132` sandbox). One serial
lane of four tasks through `scripts/task-lane.js` (every task after T1 reads an earlier one's files; T3 and T4 both
add a line to `target.rs`). No `code-architect`: the one new seam is a single trait whose shape is fixed by the probes
below (P1), and the reviewer reads it against the plan.

## Open questions

None for the maintainer. The PRD's first open question (the host-executing path list) is settled here as **P9**,
the default the maintainer might want otherwise. PRD open questions 2 and 3 are M2's.

## Summary

M1 builds the mechanics with no engine, store or TUI change:

- a **target channel** (`htui-agent`): run one argv on a target with stdin from a file on this box and stdout to a
  file on this box or a captured tail, with a deadline and a kill. Plus a test-only **stand-in** that runs the argv
  on this box under a private home directory;
- **bundle header parsing** and the **git verbs** collect needs, in `htui-orch`;
- **seed** (prepare's clone): bundle a base ref here, load its pack into a fresh repository on the target with
  `git index-pack --stdin`, set the run branch, and set origin with push disabled;
- **collect**: read the run branch's tip on the target, stream `git bundle create -` into a capped quarantine file
  here, check the header, `git bundle verify`, `git index-pack --strict`, a connectivity check, a temp ref, then a
  compare-and-swap of the local branch. Rewritten history and host-executing paths return "needs consent" until the
  caller passes a consent bound to the same tip.

Everything is proved against the stand-in, with hostile clones and crafted bundles.

## Probe results (2026-11-15, this box's `git version 2.43.0`, scratch dir `/tmp/bprobe.*`)

| # | Probe | Result |
|---|---|---|
| B1 | `git bundle create - <ref> > f` | writes the bundle to stdout, exit 0 |
| B2 | `git bundle create - <branch> ^<tip>` (nothing new) | exit 128, `fatal: Refusing to create empty bundle.` |
| B3 | `git bundle create - <sha>` | exit 128, same: a tip must be a ref name |
| B4 | `git -C host bundle verify f` / `list-heads f` | `f is okay`, prints the ref and the required prerequisite |
| B5 | `fetch` from a bundle holding a commit `git fsck` reports as `missingEmail`, with `-c transfer.fsckObjects=true` (and `fetch.fsckObjects=true`) | **accepted, exit 0, ref written**: fsck settings do **not** apply to a bundle fetch on 2.43 |
| B6 | The same pack (bundle bytes after the first `\n\n`) into `git index-pack --strict --fix-thin --stdin` | exit 128, `fsck error in packed object`, object absent afterwards |
| B7 | A good collect pack into `index-pack --strict --fix-thin --stdin` | exit 0, `pack <hash>` on stdout |
| B8 | `git init t; git -C t bundle unbundle - < seed.bundle` | works on 2.43 but undocumented: **not used** (P5 uses `index-pack --stdin`) |

## Shared decisions

Every brief carries this section.

### From the PRD

| D | What M1 does with it |
|---|---|
| D1 | The channel is a trait in `htui-agent`; the only implementation in this item is the test-only stand-in. No target kind becomes runnable; `RUNNABLE_KINDS` is untouched. |
| D3 | Seed makes the clone and the run branch; it does not register the clone in the store (M2) or make step worktrees (MOD-44/MOD-131). |
| D4 | The clone is seeded from a bundle made **here**; the target needs no credentials or network. |
| D5 | Collect returns an outcome; waiting, retrying and the person's consent are M2's. M1's API is stateless so a waiting row can store the consent facts. |

### Decisions this plan takes

- **P1 — channel shape: argv only, no cwd, no environment.** `ExecSpec { verb, argv, stdin, stdout, timeout }`. Every
  git command names its repository with `-C <dir>`, so neither `docker exec -w` nor ssh's remote shell quoting of a
  `cd` is needed. Environment injection is ANA-42 §7.4 (secrets), not needed for git, and stays out of the seam;
  config travels as `-c key=value` argv. `stdin` is `Null` or `File { path, offset }` (a file on this box, read from
  `offset`); `stdout` is `Tail` (last 64 KiB, lossily decoded) or `File { path, cap }` (created `create_new`, `0600` on
  Unix, at most `cap` bytes: past the cap the child is killed, the file removed and `ChannelError::OverCap` returned).
  Stderr is always a 64 KiB tail. A deadline kills the process group (`ChannelError::TimedOut`). Fits
  `docker exec -i <ctr> -- argv` and `ssh -T host -- argv` (argv joined with POSIX quoting by MOD-131), checked in the
  fact-check.
- **P2 — where the seam lives.** `crates/htui-agent/src/target_channel.rs`, `pub mod target_channel`. `htui-orch`
  depends on `htui-agent` (not the other way), and MOD-44's `docker exec -i` decorator lives under `TransportBuilder` in
  `htui-agent`, so the decorator and the channel can share their argv prefix there. The trait is dyn-compatible with
  a boxed future (`ChannelFuture`), as `DriverFuture` and `IsolatorFuture` are; no `async_trait`.
- **P3 — the stand-in.** `StandInChannel` in the same file, `#[cfg(feature = "test-support")]` (the crate's gate for
  `conformance` and `fake`, `htui-agent/src/lib.rs:110-118`). It runs argv
  on this box with `current_dir = home` and `HOME = home` (so relative target paths land under `home`), sets
  `HTUI_STAND_IN_TARGET=1`, and removes `htui_core::secret::HTUI_OWN_ENV` and the git location variables
  (`GIT_DIR`, `GIT_WORK_TREE`, …: the same ten names as `htui_orch::isolate::git::SCRUBBED_ENV`, copied since
  `htui-agent` cannot depend on `htui-orch`). Test knobs: `unreachable()` makes every exec fail with
  `ChannelError::Unreachable`; `canned(matcher)` answers an argv the matcher accepts with given stdout bytes and exit
  code instead of running it (crafted bundles, truncated streams).
- **P4 — hostile-clone sentinel.** Hostile fixtures install hooks and config commands that create
  `$SENTINEL/<name>` **only when `HTUI_STAND_IN_TARGET` is unset**. Target-side commands may run them (that is the
  target's own business); host-side commands never run inside the clone, so any sentinel file is a failure.
- **P5 — seeding.** Host: `git bundle create <scratch>/seed-<n>.bundle <base_ref>` in this box's checkout (a ref name,
  B3), header parsed to get the base hash. Target, in order: `git init --quiet <dir>`, then
  `git -C <dir> index-pack --stdin --fix-thin` (stdin = the bundle file from its pack offset). Then
  `git -C <dir> update-ref refs/heads/<run_branch> <base>`, `git -C <dir> checkout --quiet <run_branch>`, and
  `git -C <dir> remote add origin <url>` when a URL is given. Then `git -C <dir> config remote.origin.pushurl
  no-push-from-htui-target` (`PUSH_REFUSED`), and last `git -C <dir> config htui.seededBase <base>`. Re-seeding
  a dir whose `htui.seededBase` equals the base is `Seeded { reused: true }`; one that differs, or a non-empty dir
  without it, is refused. The quarantine file is removed on every exit.
- **P6 — collect: one stateless call.** `collect(cli, channel, req) -> Result<CollectOutcome, CollectRefusal>`.
  1. Target: `git -C <dir> rev-parse --verify --quiet refs/heads/<run_branch>^{commit}`; it must be 40 lowercase hex
     (P7) or the call refuses.
  2. Tip equals the base: `Nothing` (B2).
  3. Target: `git -C <dir> bundle create - refs/heads/<run_branch> ^<base>` into a quarantine file
     (`create_new`, `0600`, `req.cap`; default `COLLECT_CAP` = 2 GiB).
  4. Host checks, all before any ref is written:
     - The header parses (P7) with exactly one ref, `refs/heads/<run_branch>`, at the tip from step 1 (else `Changed`).
     - Every prerequisite is a commit this box holds (`has_commit`).
     - `git bundle verify` passes (`-c core.hooksPath=/dev/null`).
     - `git index-pack --strict --fix-thin --stdin` from the pack offset passes (B5/B6: the fsck that `fetch` skips).
     - `git rev-list --objects --quiet <tip> --not <prerequisites>` exits 0, so the tip is connected.
  5. The temp ref is written by gix, `MustNotExist`, after deleting a leftover; it holds the objects through the
     decision.
  6. Decide:
     - Local branch absent: `Created`.
     - Local branch present and an ancestor of the tip: `Moved`.
     - Not an ancestor: rewritten.
     - The branch checked out anywhere (`checked_out_at`): refused.
     - Host-executing paths (P9) changed between the old tip (or the base when the branch is new) and the tip: listed.
     - Rewritten, or any path listed, and `req.consent` absent or not for this tip (or a different path list):
       `NeedsConsent { tip, rewritten_from, config_paths }`, nothing moved.
     - Otherwise the branch moves by compare-and-swap (`create_branch`'s `MustNotExist`, `move_branch`'s
       `MustExistAndMatch`; a lost race is `CollectRefusal::LocalMoved`). A consented rewrite is `Forced`.
  7. The temp ref and the quarantine file are removed on **every** exit, refusal included.
- **P7 — bundle header parser, ours.** `htui-orch/src/target/bundle.rs`. Accepts `# v2 git bundle` and
  `# v3 git bundle`. In v3 only `@object-format=sha1` is accepted. `@object-format=sha256` refuses by name, because
  the workspace builds gix with the `sha1` feature only (root `Cargo.toml:166-167`), so no local repository is
  sha256. `@filter=` refuses by name (a partial bundle), and any other capability refuses. Prerequisite lines
  (`-<hash>[ comment]`) and ref lines (`<hash> <refname>`) are validated: 40 lowercase hex, and the ref name through
  `gix::validate::reference::name`. The header is at most `HEADER_CAP` (1 MiB) and ends at the first empty line;
  `pack_offset` is the byte after it. Only `pack_offset..` is ever handed to `index-pack`.
- **P8 — temp ref name.** `refs/htui-collect/<run uuid>/<repo uuid>` (`temp_ref(run, repo)`), not ANA-42's
  `refs/htui-collect/<item>`: an item may have several runs, a run may span repos, and a uuid needs no
  ref-name escaping. Recorded as a deviation.
- **P9 — host-executing paths (PRD OQ-1).** `HOST_EXECUTING_PATHS`, matched on repo-relative `/`-separated paths:
  - **Prefixes (a directory at the root):** `.claude/`, `.clinerules/`, `.cursor/`, `.gemini/`, `.codex/`, `.agent/`,
    `.agents/`, `.vscode/`, `.idea/`, `.githooks/`, `.husky/`, `.cargo/`, `.devcontainer/`.
  - **Exact root files:** `.mcp.json`, `.envrc`, `.pre-commit-config.yaml`, `rust-toolchain`, `rust-toolchain.toml`,
    `Makefile`, `justfile`, `package.json`, `.npmrc`, `.gitmodules`, `compose.yaml`, `docker-compose.yml`.
  - **Basenames anywhere:** `CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`, `GEMINI.md`, `.gitattributes`, `build.rs`.

  `req.extra_confirm_paths` adds caller-given prefixes (an entry ending in `/`) or exact paths. M2 reads them from
  `project.settings.collect_confirm_paths`. The result is sorted and deduplicated. No flag skips consent: only a
  `Consent` naming the same tip and the same list does.
- **P10 — budgets.** Small target commands (rev-parse, init, config, update-ref, checkout, remote) get `VERB_TIMEOUT`
  (120 s); bundle create and both index-packs get `TRANSFER_TIMEOUT` = 15 min. `Cli` gains a public
  `with_transfer_budget()` beside the test-only `with_budget`.
- **P11 — sentences.** One named function per refusal sentence, the house style (`isolate/git.rs:138-140`). Every
  sentence names the repo directory and, where there is one, the hash.

## Patterns to mirror

| Category | Source | Pattern |
|---|---|---|
| Boxed dyn future | `crates/htui-orch/src/isolate.rs` `IsolatorFuture` | `Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>`; trait `Send + Sync + fmt::Debug` |
| Process spawn | `crates/htui-orch/src/isolate/git.rs` `spawn_supervised`, `Cli::run_captured` | process group (Unix) / job object (Windows), both pipes drained concurrently into tail buffers, budget kills the group, `PIPE_GRACE` after exit |
| Git env | `git.rs` `Cli::command` | `SCRUBBED_ENV` + `HTUI_OWN_ENV` removed, `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, null stdin, `kill_on_drop` |
| Ref CAS | `git.rs` `create_branch`, `move_branch` | gix `PreviousValue::MustNotExist` / `MustExistAndMatch`, called through `with_retry` under `blocking` |
| Errors | `isolate.rs` `IsolateError`; `git.rs:138-160` | `thiserror` enum; sentences as named `pub fn … -> String` |
| Integration tests | `crates/htui-orch/tests/it/run_branch.rs`, `landing.rs`, `fixtures.rs` | real `git` in temp dirs, `tests/rust/write_executable.rs` for scripts (never `fs::write` + chmod) |

## Tasks

### T1 — The target channel and the stand-in (foundation)

- **Files**:
  - `crates/htui-agent/src/target_channel.rs` (new)
  - `crates/htui-agent/src/lib.rs` (one `pub mod`)
  - `crates/htui-agent/tests/it/target_channel.rs` (new)
  - `crates/htui-agent/tests/it/main.rs` (one `mod`)
- **Add**:
  ```rust
  pub type ChannelFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ChannelError>> + Send + 'a>>;
  pub enum StdinFrom { Null, File { path: PathBuf, offset: u64 } }
  pub enum StdoutTo { Tail, File { path: PathBuf, cap: u64 } }
  pub struct ExecSpec { pub verb: &'static str, pub argv: Vec<String>, pub stdin: StdinFrom,
                        pub stdout: StdoutTo, pub timeout: Duration }
  pub struct ExecOutput { pub code: Option<i32>, pub stdout: String /* tail; empty under File */,
                          pub stderr: String /* tail */, pub stdout_bytes: u64 }
  impl ExecOutput { pub fn ok(&self) -> bool }
  #[derive(Debug, thiserror::Error)]
  pub enum ChannelError { Unreachable(String), Spawn(String), TimedOut { verb: &'static str, after: Duration },
                          OverCap { verb: &'static str, cap: u64 }, Io(#[from] std::io::Error) }
  pub trait TargetChannel: Send + Sync + fmt::Debug {
      fn describe(&self) -> String;                       // "stand-in at /tmp/x", for sentences
      fn exec<'a>(&'a self, spec: &'a ExecSpec) -> ChannelFuture<'a, ExecOutput>;
  }
  #[cfg(feature = "test-support")]
  pub struct StandInChannel { /* home, unreachable flag, canned answers */ }
  // new(home), home(), unreachable(self) -> Self,
  // canned(self, matcher: impl Fn(&[String]) -> Option<Canned> + Send + Sync + 'static) -> Self
  #[cfg(feature = "test-support")]
  pub struct Canned { pub stdout: Vec<u8>, pub code: i32 }
  pub const STAND_IN_MARKER: &str = "HTUI_STAND_IN_TARGET";
  ```
  The stand-in mirrors `htui-orch`'s `isolate/git.rs` `spawn_supervised` in a private function: a process group on
  Unix; on Windows `CREATE_NO_WINDOW` and a job object, with its downgrade when the job is refused. `launch.rs`'s
  `spawn_supervised` is private and takes a `ResolvedLaunch`. A canned answer writes its bytes through the same
  `StdoutTo` handling, cap included.
- **Tests** (`target_channel::`):
  - argv runs under `home` with `HOME` and the marker set, and `HTUI_OWN_ENV` absent;
  - exit codes pass through;
  - stdin from a file at an offset;
  - stdout to a file: `0600`, `create_new` refuses an existing file, the byte count is reported;
  - over the cap, the child is killed and the file removed;
  - the deadline kills a `sleep` and its child;
  - `unreachable()`;
  - a canned answer.
  Unix-only cases are `#[cfg(unix)]`.
- **Check**: `cargo test -p htui-agent --all-features --test it target_channel::` then
  `cargo clippy -p htui-agent --all-features --all-targets -- -D warnings` and `cargo clippy -p htui-agent -- -D warnings`.

### T2 — Bundle header and the git verbs (foundation)

- **Files**:
  - `crates/htui-orch/src/target.rs` (new: module doc and `pub mod bundle;`)
  - `crates/htui-orch/src/target/bundle.rs` (new)
  - `crates/htui-orch/src/lib.rs` (one `pub mod target;`)
  - `crates/htui-orch/src/isolate/git.rs`
  - `crates/htui-orch/tests/it/target_bundle.rs` (new)
  - `crates/htui-orch/tests/it/main.rs`
- **Add** in `bundle.rs` (P7): `BundleHeader { version,
  prerequisites: Vec<String>, refs: Vec<(String, String)>, pack_offset: u64 }`, `HEADER_CAP`,
  `parse_header(&[u8]) -> Result<BundleHeader, BundleRefusal>`, `read_header(&Path)` (reads at most `HEADER_CAP`),
  `BundleRefusal` (`thiserror`, one variant per class: not a bundle, unknown version, capability, filter, malformed
  line, bad hash, bad ref name, header too long, no end of header, sha256) with sentences.
- **Add** in `git.rs`:
  - `pub const TRANSFER_TIMEOUT: Duration` (15 min);
  - `Cli::with_transfer_budget(&self) -> Self`;
  - a private stdin parameter for `run_captured` (`Stdio::null()` stays the default for every existing verb);
  - `Cli::bundle_create(repo, full_ref, out)`, which refuses when `out` exists;
  - `Cli::bundle_verify(repo, file)`;
  - `Cli::index_pack(repo, file, offset, strict: bool) -> Result<(), IsolateError>`: `index-pack --stdin --fix-thin`,
    plus `--strict` when asked, under `-c core.hooksPath=/dev/null`;
  - `Cli::connected(repo, tip, not: &[String]) -> Result<bool, IsolateError>`;
  - gix `ref_target(path, full_ref) -> Result<Option<String>>`;
  - `create_ref(path, full_ref, target)` (`MustNotExist`);
  - `delete_ref(path, full_ref) -> Result<bool>`.

  New sentences as named functions.
- **Tests** (`target_bundle::`):
  - parser table: v2, v3 with object-format, filter refused, unknown capability, CRLF and garbage, 1 MiB without an end,
    bad hash length, a ref name with `..`, prerequisites with comments,
    `@object-format=sha256` refused;
  - round trip against a real `git bundle create`;
  - `index_pack` strict refuses the B5 object (made with `hash-object --literally`) and leaves no object;
  - `connected` false for a tip whose parent is missing;
  - `create_ref` refuses an existing ref;
  - `delete_ref` of an absent ref returns `false`.
- **Check**: `cargo test -p htui-orch --all-features --test it target_bundle::` plus
  `cargo test -p htui-orch --all-features --lib isolate::git` (existing unit tests stay green), clippy as in T1 for
  `htui-orch`.

### T3 — Seed: prepare's clone

- **Files**:
  - `crates/htui-orch/src/target/seed.rs` (new)
  - `crates/htui-orch/src/target.rs` (one `pub mod seed;`)
  - `crates/htui-orch/tests/it/target_seed.rs` (new)
  - `crates/htui-orch/tests/it/main.rs`
- **Add** (P5):
  ```rust
  pub const PUSH_REFUSED: &str = "no-push-from-htui-target";
  pub struct SeedRequest { pub repo: PathBuf /* this box's checkout */, pub base_ref: String /* refs/heads/… */,
                           pub target_dir: String, pub run_branch: String, pub remote_url: Option<String>,
                           pub scratch: PathBuf }
  pub struct Seeded { pub base: String, pub reused: bool }
  #[derive(Debug, thiserror::Error)] pub enum PrepareRefusal { Channel(#[from] ChannelError), Git(#[from] IsolateError),
      Bundle(#[from] BundleRefusal), Occupied { dir: String }, SeededOther { dir: String, base: String },
      TargetGit { verb: &'static str, dir: String, stderr: String } }
  pub async fn seed(cli: &Cli, channel: &dyn TargetChannel, req: &SeedRequest) -> Result<Seeded, PrepareRefusal>;
  ```
- **Tests** (`target_seed::`, over `StandInChannel`):
  - the clone holds the base commit, is on `run_branch` with a clean tree, origin is the URL, `git push origin` fails,
    and no credential file or helper is configured;
  - re-seed is `reused`; a different base is `SeededOther`; an occupied dir is `Occupied`;
  - an unreachable channel refuses with the channel's sentence;
  - the quarantine file is gone after success and after a refusal;
  - a hostile `~/.gitconfig` in the stand-in home does not reach this box's commands (P4 sentinel).
- **Check**: `cargo test -p htui-orch --all-features --test it target_seed::`, clippy and rustfmt on `htui-orch`.

### T4 — Collect (foundation for M2)

- **Files**:
  - `crates/htui-orch/src/target/collect.rs` (new)
  - `crates/htui-orch/src/target.rs` (one `pub mod collect;`)
  - `crates/htui-orch/tests/it/target_collect.rs` (new)
  - `crates/htui-orch/tests/it/main.rs`
- **Add** (P6, P8, P9):
  ```rust
  pub const COLLECT_CAP: u64 = 2 << 30;
  pub const HOST_EXECUTING_PATHS: … ; pub fn host_executing(changed: &[String], extra: &[String]) -> Vec<String>;
  pub fn temp_ref(run: RunId, repo: RepoId) -> String;
  pub struct Consent { pub tip: String, pub force: bool, pub config_paths: Vec<String> }
  pub struct CollectRequest { pub repo: PathBuf, pub target_dir: String, pub run_branch: String, pub base: String,
      pub local_branch: String, pub temp_ref: String, pub extra_confirm_paths: Vec<String>, pub scratch: PathBuf,
      pub cap: u64, pub consent: Option<Consent> }
  pub enum CollectOutcome { Nothing, Created { tip: String }, Moved { from: String, tip: String },
      Forced { from: String, tip: String },
      NeedsConsent { tip: String, rewritten_from: Option<String>, config_paths: Vec<String> } }
  #[derive(Debug, thiserror::Error)] pub enum CollectRefusal { Channel(#[from] ChannelError), Git(#[from] IsolateError),
      Bundle(#[from] BundleRefusal), NoBranch { dir: String, branch: String }, NotAHash { got: String },
      Changed { expected: String, got: String }, MissingPrerequisite { hash: String }, VerifyFailed(String),
      FsckFailed(String), NotConnected { tip: String }, CheckedOut { branch: String, at: PathBuf },
      LocalMoved { branch: String } }
  pub async fn collect(cli: &Cli, channel: &dyn TargetChannel, req: &CollectRequest)
      -> Result<CollectOutcome, CollectRefusal>;
  ```
  A `Consent` whose `force` is false never lets a rewrite through; one whose `tip` or `config_paths` differ from this
  call's is ignored (the outcome is `NeedsConsent` again).
- **Tests** (`target_collect::`, stand-in clones made by T3's `seed`):
  - `Nothing`, `Created`, `Moved`;
  - a rewritten branch: `NeedsConsent`, then `Forced` with consent, while consent without `force` stays `NeedsConsent`;
  - a config path (`.claude/settings.json`, nested `AGENTS.md`, `build.rs`, an `extra` prefix): `NeedsConsent`, then
    a move with consent; consent for another tip is ignored;
  - the local branch checked out in a worktree is `CheckedOut`;
  - a concurrent local move between decide and CAS is `LocalMoved` (a gix ref write through a test seam, or a
    second branch move, before the call);
  - crafted bundles through `canned`: an extra ref, the wrong ref, the tip changed, a missing prerequisite, the B5
    object (`FsckFailed`), a truncated pack, a v3 `@filter`, over the cap;
  - an unreachable target;
  - a hostile clone (P4: `core.fsmonitor`, `uploadpack.packObjectsHook`, `core.hooksPath` and hooks,
    `.git/config` `[include]`) leaves no sentinel;
  - after every case, no `refs/htui-collect/*` ref and no quarantine file remain.
- **Check**: `cargo test -p htui-orch --all-features --test it target_collect::`, clippy and rustfmt on `htui-orch`.

## Lane

One lane, serial, on `hr/MOD-132` in `/home/mluigi/projects/htui`: T1 → T2 → T3 → T4. Verifiers: T1 and T4
(`foundation`; T4's attack: make host-side git read the clone, make a bad object land, leave a temp ref behind,
move the branch without consent). T2 and T3 go to the gate and the reviewer.

## Files to change

| File | Action | Task |
|---|---|---|
| `crates/htui-agent/src/target_channel.rs` | CREATE | T1 |
| `crates/htui-agent/src/lib.rs` | UPDATE | T1 |
| `crates/htui-agent/tests/it/target_channel.rs` | CREATE | T1 |
| `crates/htui-agent/tests/it/main.rs` | UPDATE | T1 |
| `crates/htui-orch/src/lib.rs` | UPDATE | T2 |
| `crates/htui-orch/src/target.rs` | CREATE | T2, T3, T4 (one line each) |
| `crates/htui-orch/src/target/bundle.rs` | CREATE | T2 |
| `crates/htui-orch/src/isolate/git.rs` | UPDATE | T2 |
| `crates/htui-orch/src/target/seed.rs` | CREATE | T3 |
| `crates/htui-orch/src/target/collect.rs` | CREATE | T4 |
| `crates/htui-orch/tests/it/main.rs` | UPDATE | T2, T3, T4 |
| `crates/htui-orch/tests/it/target_bundle.rs` | CREATE | T2 |
| `crates/htui-orch/tests/it/target_seed.rs` | CREATE | T3 |
| `crates/htui-orch/tests/it/target_collect.rs` | CREATE | T4 |

## Verified claims (step 3.5, 2026-11-15)

| Claim | Verdict | Evidence |
|---|---|---|
| `htui-orch` depends on `htui-agent`, not the reverse (P2) | true | `crates/htui-orch/Cargo.toml`; `htui-agent` depends only on `htui-core` among htui crates |
| `htui-agent` has `thiserror`, `process-wrap`, `windows` (cfg windows) | true | `crates/htui-agent/Cargo.toml` |
| The stand-in's gate is `any(test, feature = "test-support")` | **amended**: the crate gates with `cfg(feature = "test-support")` | `crates/htui-agent/src/lib.rs:110-118` |
| `launch.rs` has a reusable supervised spawn | **false**: private and takes a `ResolvedLaunch`; T1 now mirrors `git.rs`'s | `crates/htui-agent/src/launch.rs:1315-1319` |
| `DriverFuture` and `IsolatorFuture` are boxed dyn futures | true | `htui-agent/src/driver.rs:41`; `htui-orch/src/isolate.rs` |
| `Cli::command` nulls stdin; `run_captured` is the one spawn path | true | `isolate/git.rs:345-364`, `:403-450` |
| `create_branch` (`MustNotExist`), `move_branch` (`MustExistAndMatch`), `checked_out_at`, `has_commit`, `is_ancestor`, `with_retry`, `name_only` exist with those shapes | true | `isolate/git.rs:2236`, `:2256`, `:2278`, `:1856`, `:1902`, `:1390`, `:1000` |
| gix validates a ref name | true | `gix::validate` re-export (`gix-0.87.1/src/lib.rs:162`), `gix_validate::reference::name` (`gix-validate-0.11.4/src/reference.rs:77`) |
| sha256 bundles can be accepted (and 64-hex tips) | **false**: gix is built `sha1` only; P6 and P7 amended to 40 hex and to refuse sha256 | root `Cargo.toml:166-167` |
| The seed sequence (P5) works on `git` 2.43 | true: `init`, `index-pack --stdin --fix-thin` from the pack offset, `update-ref`, `checkout`, `remote add`, `pushurl`; the push then exits 128 | probe 2026-11-15 |
| fsck settings do not cover a bundle fetch; `index-pack --strict` does | true | probes B5, B6 |
| `bundle create` refuses an empty bundle and a bare hash tip | true | probes B2, B3 |
| Integration tests use real `git` in temp dirs and `write_executable` | true | `crates/htui-orch/tests/it/main.rs`, `run_branch.rs`, `landing.rs` |
| P1 fits `docker exec -i <ctr> -- argv` and `ssh -T host -- argv` | true by construction: both pass stdin and stdout binary-clean and return the exit status; ssh re-joins argv in the remote shell, so MOD-131 quotes | ANA-42 §7.2 |
| Task independence | none claimed: one serial lane; T2–T4 share `target.rs` and `tests/it/main.rs` | Files table |

## Validation

```bash
cargo test -p htui-agent --all-features --test it target_channel::
cargo test -p htui-orch --all-features --test it target_
cargo clippy --workspace --all-features --all-targets -- -D warnings && cargo clippy --workspace -- -D warnings
bash scripts/gate.sh   # main thread, once for the lane
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The channel's argv-only shape misses something docker or ssh need | Medium | P1 checked against both command lines in the fact-check; environment stays out of the seam on purpose |
| `index-pack --strict` accepts something `fetch` would refuse | Low | Connectivity check after it; B5/B6 pinned as tests |
| A `git` in `PATH` newer or older than 2.43 behaves differently (fsck on bundle fetch, B2's wording) | Low | Collect never parses wording: the tip-equals-base check precedes bundle create; fsck is explicit |
| Featureless clippy flags stand-in-only code | Medium | Stand-in behind `cfg(feature = "test-support")`; featureless clippy in each check |
| Windows: `0600`, process-group kill | Medium | `#[cfg(unix)]` mode; job object path mirrored from `git.rs` |

## Acceptance

- [ ] All four tasks committed, each with its check green
- [ ] Gate green on the lane
- [ ] Reviewer findings applied or deferred with the maintainer
- [ ] No engine, store, TUI, migration or snapshot change
