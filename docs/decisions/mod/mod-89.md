# MOD-89 - Warn or refuse when a run's session trees hold env files (done, 2026-10-13)

**Requirements:** `R-SEC-2`, `R-ID-7`.
**Origin:** MOD-10's PRD open question, decided 2026-10-03 as a follow-up filed at close-out
(`docs/decisions/mod/mod-10.md`).
**Artifacts:** plan `.claude/plans/mod-89-env-files.plan.md` (decisions D1-D7, verified-claims
table, 20 claims), blueprint `.claude/plans/mod-89-env-files.blueprint.md` (plan deltas, fan-out
decision F-1). Routed as **plan** (C3 only), no ultracode. User docs: `docs/htui-secrets.md`,
"Env files in the session's trees".

## The problem

MOD-10 lets a project drop its `.env` by resolving secrets from Infisical into `SessionSpec.env`
only. Nothing stopped a run on a tree that still held one: the agent can read the file, and from
there the values reach the model's context and the stored transcript.

## The decisions (D1-D7, confirmed by the maintainer at CONFIRM)

- **D1, what counts.** `.env`; `.env.<suffix>` except the template suffixes `example`, `sample`,
  `template`, `dist`, `defaults`, `schema` (suffix compared whole, so `.env.local.example`
  counts); direnv's `.envrc`. ASCII case-insensitive. Files and symlinks (never followed); a
  directory never counts and is never entered (a Python `.env/` venv). Names only, never contents.
- **D2, where.** Each session's `cwd` plus `extra_dirs`, recursively, pruning `.git` and the
  `copy` exclude list (production: always `DEFAULT_COPY_EXCLUDE`), 100 000 entries per session.
  Fail-open on I/O: an unreadable entry or a failed scan is a `warn`, never a refusal.
- **D3/D5, refuse or warn.** A project with `secret_provider` set is refused; a provider-less
  project is warned. Both are checked.
- **D4, per project.** Project-rung boolean `allow_env_files` (default off): on, a provider
  project is warned instead. No effect on a provider-less project.
- **D6, where it shows.** Refusal: `EngineError::EnvFiles` → `RunFailure::EnvFilesRefused`, the
  run's failure sentence. Warning: one item note per run (deduplicated against notes via any step
  of this run), plus a count-only `tracing::warn!`.
- **D7, sentences.** `env_files_refused: <list> in the session's trees; the project resolves its
  secrets from <provider>. Remove them, or set allow_env_files.` and `env files in the session's
  trees: <list>; the agent can read them`. `<list>` names at most five backticked root-relative
  paths, then `and N more`, then `(scan cut at <cap> entries)` when the walk was cut.

Isolation decides exposure (probed against real git): `local`/`shared_serialized` run in the
checkout (a git-ignored `.env` is there), `copy` copies it byte for byte, `worktree` holds tracked
files only. One check over the prepared trees covers all four modes.

## What was built

- **Scanner** (`crates/htui-orch/src/isolate/env_files.rs`): `is_env_file` (allocation-free),
  `scan(cwd, extra_dirs, prune, cap) -> EnvFiles { paths, truncated: Option<usize> }`, `listed`,
  `refused`, `warning`. The walk visits every root's top level before any deeper level, and files
  before directories within a directory (sorted by name), so a huge subtree cannot spend the cap
  before a root `.env` is seen. The cap is one budget across all roots. Control characters and
  backticks in a path render as `?`. Measured on the htui checkout: 2 719 entries, ~3.8 ms (debug).
  `ad00c663`, `7c59b3d5`.
- **`Isolator::env_files(cwd, extra_dirs)`**: `GixIsolator` on the blocking pool with
  `copy::excludes`; `FakeIsolator` scripted FIFO (one answer per session, judges included) with
  `fail_env_files` and recorded roots. Per-mode `gix_isolator` cases over real git. `2277d4bc`.
- **`allow_env_files`** (`crates/htui-core/src/prompt/settings.rs`): `SettingKey::AllowEnvFiles`
  first in key byte order (every discriminant shifted by one; they only index `SPECS`),
  `resolve_allow_env_files`, Settings > Prompt row (`on/off`, default off). Four
  `prompt_settings` snapshots gained the one row. `4fd174c5`, merged `65151191`.
- **Engine** (`crates/htui-orch/src/engine.rs`): `drive_once` runs `check_env_files` after
  `env_for` (a secrets refusal wins) and before the tool host and driver. A cancel that lands
  during the scan returns `Cancelled`, not the refusal (MOD-42 I-6). The warning path
  (`warn_env_files`) is best-effort: a store error is a `warn` and the session starts; a fence
  error still propagates. `WorkerStore::notes` added with three forwarders (no new SQL).
  `d608f5e4`, `d8b4e286`, `08af6e32`, `a0e0a3e8`.
- **Fan-out (blueprint F-1).** Trees exist only after each candidate's own `prepare`, so the
  check cannot sit at the group's entry. After `join_all`, when every member of the slot (re-read)
  is `Failed` and every candidate this walk ran was refused over env files, `fail_refused_group`
  fails the run with `EnvFilesRefused` (no retry, no selection, item not blocked). A slot holding
  a `Done` sibling routes as a mixed group (D48). `fail_candidate` now answers whether it moved
  the row, and only a moved row reports the refusal, so a lost compare-and-set never finishes a
  run another writer owns.
- **Docs**: `docs/htui-secrets.md` "Env files in the session's trees" plus a cross-link from
  "When htui refuses". `de69943e`, `89bd229d`.

Out of scope: chat sessions (they start in htui's own working directory with an empty env), and
leaving `.env` out of `copy`'s copy.

## Review and gate

**Review.** `rust-reviewer`: no CRITICAL/HIGH, 4 MEDIUM, 9 LOW. Applied (`7c59b3d5`, `a0e0a3e8`,
`89bd229d`): M1 cap could miss a root `.env` (top-levels and files first), M2 fold overrode a
`Done` sibling on a resumed slot, M3 fold acted after a lost compare-and-set, M4 no mixed-group
test; L5 cancel during the scan, L6 best-effort warning, L8 the sentence named `SCAN_CAP` not the
actual cap, L9 backtick in a path, L10 per-entry allocation, L11a `write!`, L12 three more tests,
L13 doc wording. Deferred (see Follow-ups): L7, L11b, L11c.

**Gate** (sandbox, `HTUI_TEST_DATABASE_URL` set): `cargo fmt --all --check`; `cargo clippy
--workspace --all-features --all-targets -- -D warnings`; `cargo clippy --workspace -- -D
warnings`; `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`. Before the
review fixes: 6 023 passed, 0 failed. Final tree (`89bd229d`): 6 031 passed, 0 failed, no SIGABRT.

## Follow-ups

- **Review L7, accepted.** A crash after every candidate's failure but before the fold leaves a
  resumed walk with an all-failed slot that routes through `Select` like any all-failed group:
  `no_surviving_candidate`, or under `never` a retry that is refused again. Each candidate's note
  still names the files. A comment at the fold names the window.
- **Review L11b/L11c, style.** `group_error` could take errors rather than `Result`s; the three
  "fail the run at this position" shapes could share one helper.
- Concurrent fan-out candidates of a provider-less project can each leave the warning note before
  seeing the others' (pinned as a range, documented).
- Owed on the host: the Postgres gate on the merged tree.
