# Plan: MOD-89 - Warn or refuse when a run's worktree holds a `.env` file

**Source**: `HANDOFF.md` MOD-89 (from MOD-10, `docs/decisions/mod/mod-10.md`). `R-SEC-2`, `R-ID-7`.
**Route**: plan path (C3 only fired), accepted by the maintainer 2026-10-13. Ultracode: not needed.
**Complexity**: Medium (about 12 files, 4 tasks, one new module)

## Summary

Before every agent session of a run starts, htui scans the trees that session can read (its
`cwd` and `extra_dirs`) for dotenv-style files. A **provider project** (one with
`secret_provider` set, so MOD-10 already injects its secrets) is **refused**: the step fails with
a typed `env_files_refused: …` sentence before the driver is built. A **provider-less project**
gets a **warning**: one item note per run, naming the files. A per-project boolean
`allow_env_files` (default off) lets a provider project warn instead of refusing. The scan answers
file names only and never reads file contents.

## Decisions (the item's four open questions; proposed, settle at CONFIRM)

| # | Question | Proposal | Why |
|---|---|---|---|
| D1 | Which names count | `.env`; `.env.<suffix>` **except** the template suffixes `example`, `sample`, `template`, `dist`, `defaults`, `schema`; `.envrc` (direnv). ASCII case-insensitive. Regular files and symlinks only. A *directory* named like one (a Python venv called `.env/`) is not a finding and is not descended into. | Covers the dotenv family and direnv, which hold the same secrets. Templates are committed on purpose and hold no values. Case-insensitive so Windows and macOS read the same as Linux. |
| D2 | Where the scan looks | Each session's `cwd` plus `extra_dirs`, which is exactly what the agent can read. Recursive, pruning `.git` and the project's `copy_exclude` list (or `DEFAULT_COPY_EXCLUDE` when that is empty), capped at 100 000 entries. A cut walk reports `truncated` and logs a `warn`. A missing root answers nothing. An unreadable entry is skipped with a `warn`. | Monorepos keep `.env` per package (`web/.env.local`), so root-only misses real cases. The prune list is the one `copy` already uses, so a `local` step on a big checkout skips `target/` and `node_modules/`. The check is hygiene and fails open on I/O errors; the trust boundary is still MOD-10's `SessionSpec.env`. |
| D3 | Warn or refuse | Provider project: **refuse**. Provider-less project: **warn**. | A provider project has opted into "secrets come from the provider", so a leftover `.env` means the migration isn't finished. A provider-less project's `.env` is today's normal; refusing it would break every existing project. |
| D4 | Per project? | Yes: project-rung `SettingKind::Boolean` key `allow_env_files`, default off. On means a provider project warns instead of refusing. It has no effect on a provider-less project, which only warns. | It follows the MOD-33 `box_hostname` precedent (`e033470a`). Without it, refusing would have no escape hatch for a provider project that keeps a non-secret `.env` (`PORT=3000`). |
| D5 | Is a provider-less project checked | Yes (warn), per D3. | The exposure is the same with or without a provider. |
| D6 | Where it shows | **Refuse**: as the run failure. `EngineError::EnvFiles`, then `RunFailure::EnvFilesRefused`, so the sentence reaches `run.failure`, the Runs pane and `htui status --json`, the same way `SecretsRefused` does. **Warn**: one item note through `Engine::note(item, body, Some(step), now)`, deduplicated per run (skipped when the item already holds a note with the same body whose `via_step_id` is one of this run's steps), plus a `tracing::warn!`. | It reuses the two existing surfaces (the run failure and the item note) and adds no UI. Without the dedupe, a six-phase `local` run would write six identical notes. |
| D7 | Sentence | `env_files_refused: <list> in the session's trees; the project resolves its secrets from <provider>. Remove them, or set allow_env_files.` The warning reads `env files in the session's trees: <list>; the agent can read them`. `<list>` is backticked, root-relative paths with `/` separators, the first 5 then `and N more`. A path under an `extra_dirs` root is prefixed with that root's last component. With a cut walk, `(scan cut at 100000 entries)` is appended. | No absolute paths (notes are shared across users and boxes). Never a value. |

**Isolation mode decides what is exposed** (probed, see Verified claims). `local` and
`shared_serialized` run in the primary checkout, where a gitignored `.env` sits. `copy` copies the
tree byte for byte, and `.env` isn't in the exclude list. `worktree` checks out tracked files
only, so a `.env` shows up there only when it is committed. The check runs on the prepared trees,
so all four modes are covered by one rule.

**Out of scope**: Chat tab sessions (`agent_worker.rs`) start in htui's own process cwd with an
empty env, not in a run's tree. Leaving `.env` out of `copy` mode's copy would be a second fix at
the source; it is not taken here, so one check covers all modes uniformly. If wanted later, it's a
follow-up item.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Isolator verb | `crates/htui-orch/src/isolate.rs:139-200` (`Isolator` trait) | Boxed `IsolatorFuture`. The real impl does filesystem work under `spawn_blocking`. The fake answers from a FIFO script field with an unscripted default. |
| Pruned walk | `crates/htui-orch/src/isolate/copy.rs:60-140` (`Exclude`, `excludes`, `measure`) | `walkdir`, symlinks not followed, prune by path component, a bad exclude entry skipped with a `warn`. |
| Refusal before a session | `engine.rs:6288-6295` (`drive_once`'s `env_for`, then `EngineError::Secrets`); `engine.rs:6901-6912` (`failure_text`) | Fail in `drive_once` before the driver is built, so all three callers (`session`, `candidate_live`, `judge_calls`) settle it the way they settle a driver that refused to start. |
| Typed run failure | `crates/htui-orch/src/status.rs:115-121, 186` (`RunFailure::SecretsRefused`) | Variant, `Display` arm, and a bytes test (`run_failure_display_is_ana2s_bytes`). |
| Item note | `engine.rs:3514-3534` (`Engine::note`), `claim`'s `MissingTags` arm (`engine.rs:808-812`) | Note body is the sentence; `via_step_id` names the step. |
| Project boolean setting | `crates/htui-core/src/prompt/settings.rs` (`SettingKey::BoxHostname`, `resolve_box_hostname` at `:769`), commit `e033470a` | Registry row in key byte order, `Rungs::PROJECT`, a resolver that falls through to a const default. The Settings > Prompt pane lists every registry key, and per-key effective-value arms live at `prompt.rs:998`. |
| Engine tests | `engine.rs:19987+` (MOD-10 secrets cases: `a_provider_project_s_session_gets_the_resolved_env`, the refusal table) | MemStore and FakeIsolator harness, a provider project fixture, and an assert that the driver was never started. |

## Files to Change

| File | Action | Why | Task |
|---|---|---|---|
| `crates/htui-orch/src/isolate/env_files.rs` | CREATE | `is_env_file(name)`, `scan(roots, prune, cap) -> EnvFiles`, `EnvFiles { paths, truncated }`, the list formatter | T1 |
| `crates/htui-orch/src/isolate.rs` | UPDATE | `pub mod env_files;` and the trait verb `env_files(&self, roots: &[PathBuf]) -> IsolatorFuture<EnvFiles>` | T1 |
| `crates/htui-orch/src/isolate/real.rs` | UPDATE | `GixIsolator::env_files`: `spawn_blocking(scan)` with `excludes(&config.copy_exclude)` | T1 |
| `crates/htui-orch/src/fake.rs` | UPDATE | `FakeIsolator`: scripted `env_files` answers (FIFO, unscripted means empty), recorded roots | T1 |
| `crates/htui-orch/tests/gix_isolator.rs` | UPDATE | `StallAfterReconcile` delegates the new verb, plus the per-mode cases (local/copy find a gitignored `.env`, worktree doesn't) | T1 |
| `crates/htui-core/src/prompt/settings.rs` | UPDATE | `SettingKey::AllowEnvFiles` (first in byte order, renumbered), spec row, `ALL` 11→12, `resolve_allow_env_files`, tests | T2 |
| `crates/htui/src/ui/tabs/settings/prompt.rs` | UPDATE | Effective-value arm for the new key (mirrors `BoxHostname`, `:998`) | T2 |
| `crates/htui/src/prompt_settings.rs` | UPDATE (if needed) | e033470a touched it for the boolean; mirror whatever the new key needs | T2 |
| `crates/htui/tests/prompt_settings.rs` | UPDATE | Set and clear `allow_env_files` on the project rung, refused on App | T2 |
| `crates/htui/src/ui/tabs/settings/snapshots/*` | UPDATE | Settings > Prompt gains a row; full insta run, accept only that row | T2 |
| `crates/htui-orch/src/command.rs` | UPDATE | `EngineError::EnvFiles { .. }` whose `Display` is the refusal sentence | T3 |
| `crates/htui-orch/src/status.rs` | UPDATE | `RunFailure::EnvFilesRefused`, `Display` arm, bytes test | T3 |
| `crates/htui-orch/src/engine.rs` | UPDATE | `drive_once`: scan, then refuse or warn (deduped note); a `failure_text` arm; engine tests | T3 |
| `docs/htui-secrets.md` | UPDATE | New section "Env files in the session's trees" under "At run start": names, modes, refuse/warn, `allow_env_files` | T4 |

## Tasks

T1 and T2 touch disjoint file sets, so they can run in parallel (see the intersection check
below). T3 needs both. T4 comes last.

### Task 1: the scanner and the isolator verb (TDD)
- **Tests first** (`env_files.rs` unit tests): a name table (`.env`, `.ENV`, `.env.local`, and
  `.env.production` match; `.env.example`, `.env.sample`, `.env.template`, `.env.dist`,
  `.env.defaults`, `.env.schema`, `env`, `.environment`, and `x.env` don't; `.envrc` matches).
  Nested `web/.env.local` is found. `.git/`, `node_modules/` and `target/` are pruned. A directory
  `.env/` is neither found nor descended into. A symlink named `.env` is found and not followed.
  The cap cuts the walk and sets `truncated`. A missing root answers empty. Paths are relative
  with `/` separators and sorted. Then the `gix_isolator.rs` cases: `local` and `copy` find a
  gitignored `.env`, `worktree` doesn't, a committed `.env` is found in `worktree`.
- **Action**: implement the module, add the trait verb, `GixIsolator` under `spawn_blocking`,
  `FakeIsolator` scripted, `StallAfterReconcile` delegates.
- **Mirror**: `copy.rs` `measure` and `excludes`; `FakeIsolator`'s `changed` FIFO.
- **Validate**: `cargo test -p htui-orch --all-features env_files` and
  `cargo test -p htui-orch --all-features --test gix_isolator`.

### Task 2: the `allow_env_files` setting (TDD)
- **Tests first**: the registry's byte-order, `ALL`, `app_keys` (still 10, without the new key)
  and boolean-validate tests. `resolve_allow_env_files` with absent, `null`, a non-bool, `true`
  and `false`. The `prompt_settings.rs` integration cases.
- **Action**: registry row (`Rungs::PROJECT`, `SettingKind::Boolean`, unit `on/off`, doc
  `"A provider project whose session trees hold env files warns instead of refusing."`), the
  resolver, and the Settings > Prompt arm.
- **Mirror**: `BoxHostname` exactly (`e033470a`).
- **Validate**: `cargo test -p htui-core settings`,
  `cargo test -p htui --all-features --test prompt_settings`, and a full `cargo insta test -p htui
  --all-features` with only the new row accepted.

### Task 3: the engine check (TDD)
- **Tests first** (engine harness, FakeIsolator scripted with `[".env"]`):
  1. A provider project's run fails with the exact `env_files_refused: …` sentence, the driver is
     never started, and the secrets refusal still wins when both apply.
  2. A provider-less project's run proceeds and writes exactly one item note with the warning,
     `via_step_id` = the step.
  3. A two-phase provider-less run with the same findings still has exactly one note.
  4. A provider project with `allow_env_files = true` proceeds with the note.
  5. No findings means no note and no failure.
  6. A fan-out candidate on a provider project: the candidate step's failure carries the
     sentence.
  7. `failure_text` and `RunFailure` bytes.
  8. The roots handed to `env_files` are the session's `cwd` plus `extra_dirs`.
- **Action**: in `drive_once`, after `env_for` and before `open_tools` and the driver: call
  `isolator.env_files(&roots)`. If empty, do nothing. If refusing (provider set and
  `!resolve_allow_env_files`), return `Err(EngineError::EnvFiles{..})`. Otherwise do the deduped
  note and a `tracing::warn!` with the file count only.
- **Mirror**: `env_for` / `EngineError::Secrets` / `failure_text`; `claim`'s note.
- **Validate**: `cargo test -p htui-orch --all-features engine::` (and the conformance suite
  unchanged: `cargo test -p htui-orch --all-features`).

### Task 4: docs and close-out inputs
- **Action**: `docs/htui-secrets.md` section (D1–D7 in user words, the mode table, how to clear a
  refusal). Reference it from the MOD-89 write-up at close-out.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

### File-set intersection (step 3.5 independence check)
- T1 = {`isolate/env_files.rs`, `isolate.rs`, `isolate/real.rs`, `fake.rs`, `tests/gix_isolator.rs`}
- T2 = {`htui-core/src/prompt/settings.rs`, `htui/src/ui/tabs/settings/prompt.rs`,
  `htui/src/prompt_settings.rs`, `htui/tests/prompt_settings.rs`, settings snapshots}
- T1 ∩ T2 = ∅, so they can run in parallel. T3 = {`command.rs`, `status.rs`, `engine.rs`} is
  disjoint from both but depends on their APIs, so it runs after them. T4 = {`docs/htui-secrets.md`}.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo clippy --workspace -- -D warnings            # featureless gate
cargo test -p htui-orch --all-features -- --test-threads=1
cargo test -p htui-core
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast 2>&1 | grep -E "SIGABRT|FAILED|failed" || true
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `walk_live_step` resolves secrets before `isolator.prepare`, so trees exist only after the secrets check | ✓ | `engine.rs:3597-3613` |
| Every agent session of a run goes through `drive_once`, which has `cwd` + `extra_dirs` before the driver starts | ✓ | `drive_once` usages: `session`, `candidate_live` (`:4329`), `judge_calls` (`:5309`); `SessionSpec` built at `engine.rs:6361-6385` |
| `drive_once` already refuses before the driver via `env_for` then `EngineError::Secrets` | ✓ | `engine.rs:6288-6295` |
| `failure_text` maps engine errors to the run failure sentence; other variants use `Display` | ✓ | `engine.rs:6901-6912` |
| `RunFailure::SecretsRefused` is the typed precedent with a bytes test | ✓ | `status.rs:115-121`, `:186`, test `run_failure_display_is_ana2s_bytes` (`:484`) |
| `Isolator` has three impls: `GixIsolator`, `FakeIsolator`, test `StallAfterReconcile` | ✓ | graph `find_implementations` (`real.rs:263`, `fake.rs:81`, `tests/gix_isolator.rs:1977`) |
| `worktree` mode omits a gitignored `.env` | ✓ | probe 2026-10-13: `git worktree add` of a repo with an ignored `.env` lists only `.git`, `.gitignore` |
| `copy` mode carries `.env` (not in the default exclude list) | ✓ | `copy.rs:28-36` `DEFAULT_COPY_EXCLUDE` has no env entry; copy is byte-for-byte (`copy.rs:3-5`) |
| `local`/`shared_serialized` run in the primary checkout | ✓ | `isolate.rs` `Prepared.extra_dirs` doc (`:85-90`) |
| `walkdir` is already an `htui-orch` dependency | ✓ | `crates/htui-orch/Cargo.toml:43` |
| `IsolatorConfig.copy_exclude` carries the project list | ✓ | `real.rs:242-243` |
| `Exclude`/`excludes` are `pub` and reusable | ✓ | `copy.rs:60-135` |
| `Engine::note(item, body, via_step_id, now)` exists | ✓ | `engine.rs:3514-3534` |
| The engine's store can read an item's notes (dedupe) | ✓ | `ReadStore::notes(ItemId)` `traits.rs:107`; `WriteStore: ReadStore` `:326` |
| Project-rung boolean precedent: `BoxHostname`, 4 files in `e033470a` | ✓ | `git show --stat e033470a` |
| Registry keys are in key byte order and the discriminant indexes `SPECS` | ✓ | `settings.rs:150`, `:211` (`SPECS[self as usize]`), byte-order tests `:886`, `:1001`; `allow_env_files` sorts before `box_hostname`, so every discriminant shifts by one and the test at `:935` ("`box_hostname` sorts first") must be updated |
| `app_keys` stays 10 (project-only key) | ✓ | `settings.rs:202-206`, test `:986-990` |
| Settings > Prompt lists every registry key; per-key effective arms exist | ✓ | `prompt.rs:13-14`, `:998` |
| Chat sessions don't run in a run's tree | ✓ | `agent_worker.rs` `cwd = std::env::current_dir()` (`:1443` and others), `env: BTreeMap::new()` (`:1142`) |
| T1 ∩ T2 file sets are empty | ✓ | intersection above |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Walk cost on a huge `local` checkout, every session | Medium | Prune `.git` and the copy-exclude list, 100k cap, `spawn_blocking`; measure once on the htui repo itself in T1 |
| A fan-out refusal surfaces as `no_surviving_candidate` at run level | Medium | Each candidate's step failure carries the sentence (test 6). If that's not enough, also check in `drive_group` before the first candidate; decide at blueprint |
| Renumbering `SettingKey` discriminants breaks a hidden index | Low | Discriminants index `SPECS` only; byte-order tests catch drift |
| `allow_env_files` sits in Settings > Prompt, not Secrets | Medium (UX) | It's the only key-level project-settings editor today. Name it plainly, and the doc row says what it does. Moving it to the Secrets section is a follow-up if wanted |
| Settings snapshots shift by one row | High (expected) | Full insta run, accept only the new row (memory: snapshot impact needs a full run) |
| Fail-open on I/O errors misses a file | Low | Documented; the boundary stays `SessionSpec.env` (MOD-10) |

## Acceptance
- [ ] A provider project's run on a tree with `.env` fails before any agent starts, with the
      `env_files_refused` sentence in `run.failure`
- [ ] A provider-less project's run proceeds with exactly one warning note per run
- [ ] `allow_env_files` downgrades the refusal to the warning
- [ ] Template files, `.env/` directories and pruned directories are never findings
- [ ] Per-mode behaviour pinned against real git (`gix_isolator.rs`)
- [ ] Docs updated; validation commands green; reviewer gate (`rust-reviewer`) passed
