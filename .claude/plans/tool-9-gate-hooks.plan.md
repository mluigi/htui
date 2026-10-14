# Plan: TOOL-9 - Canonical gate script, committed git hooks, DECISIONS merge rule

**Source**: `HANDOFF.md` TOOL-9 (from ANA-29 `docs/ANA-29.md` §4.1, §4.2 D5/D7/D8)
**Routing**: plan path, 0 of C1-C4 fired (accepted by the maintainer)
**Complexity**: Medium (about 8 files, no Rust code)

## Summary

Add `scripts/gate.sh` and `scripts/gate.ps1` (one gate sequence, optional detached run, JSON result in
`target/gate/last.json`), commit `scripts/githooks/` and have the installers point `core.hooksPath` at it,
and add `.gitattributes` with `DECISIONS.md merge=union`. Nothing here touches the crates.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Twin scripts | `.claude/skills/handoff-run/scripts/validate-workflow-docs.{sh,ps1}` | `.sh` and `.ps1` kept in lockstep, same step names and messages, `--self-test` |
| Windows refusal | `install-workflow-hooks.sh:35-46` | `.sh` exits 2 on MINGW/MSYS/CYGWIN naming the `.ps1`; override `WORKFLOW_ALLOW_SH_ON_WINDOWS=1` |
| Hook block | `install-workflow-hooks.sh` `new_pre_commit_block` | `--scope-paths "$WF_STAGED"`, `WORKFLOW_SKIP_HOOK=1` escape hatch, no `exit 0` in the block, no `--no-verify` advice |
| Installer self-test | `install-workflow-hooks.sh` `run_self_test` (13 cases) | each behaviour pinned by a case against a fixture repo |
| Script location | `scripts/hr`, `scripts/hr-mint` | repo-root `scripts/`, bash |

## Decisions the plan makes (flag any you want changed)

1. **`core.hooksPath` replaces `.git/hooks` entirely**, so the existing marker-block hooks stop running the moment it is set.
   Committed hooks therefore **chain**: after their own work they run `$(git rev-parse --git-common-dir)/hooks/<name>` if
   it exists and is executable (this keeps graphify and any host-side foreign block alive).
2. **No double validation.** The installer, when it sets `core.hooksPath`, **removes its own `# workflow-hook-start/end`
   block** from `.git/hooks/pre-commit` (foreign blocks untouched), since the committed hook now does that job. The
   post-commit sync block stays in `.git/hooks/post-commit`, reached through the chain.
3. **Relative `core.hooksPath = scripts/githooks`**, so it resolves in the main checkout and every linked worktree.
4. **Installer target default.** `engine` does not exist in this repo, so the installer's default `--targets "engine ../engine-template?"`
   exits 2 here today. The hooksPath step runs for the workspace entry **before** target resolution and does not depend on targets.
5. **Gate does not run Postgres/Qdrant suites separately.** It runs `cargo test` as specified; the host Postgres gate stays a host step.
6. **`htui-dev` is absent** (TOOL-8 is open), so the plugin step reports `skipped` rather than failing.

## Files to Change

| File | Action | Why |
|---|---|---|
| `scripts/gate.sh` | CREATE | canonical gate, `--detach`, `--status`, writes `target/gate/last.json` |
| `scripts/gate.ps1` | CREATE | Windows twin of `gate.sh` |
| `scripts/githooks/pre-commit` | CREATE | validator with `--scope-paths` on staged workflow docs, then chain |
| `scripts/githooks/pre-commit.ps1` | CREATE | pwsh body for Windows; `pre-commit` dispatches to it by `uname` |
| `scripts/githooks/post-commit` | CREATE | chain only (keeps the workspace sync block reachable) |
| `.claude/skills/handoff-run/scripts/install-workflow-hooks.sh` | UPDATE | set `core.hooksPath`, strip own pre-commit block, self-test cases 14-16 |
| `.claude/skills/handoff-run/scripts/install-workflow-hooks.ps1` | UPDATE | same, in lockstep |
| `.gitattributes` | CREATE | `DECISIONS.md merge=union` |

Close-out only (serial, after the tasks): `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/tool/tool-9.md`, and
`.claude/skills/handoff-run/references/lifecycle.md:98-99` if its commit-hook wording needs the new path.

## Tasks (touched-file sets; independence checked by intersection)

| Task | Files | Independent of |
|---|---|---|
| T1 gate.sh | `scripts/gate.sh` | T3, T4 |
| T2 gate.ps1 | `scripts/gate.ps1` | T3, T4; **after T1** (mirrors its step names) |
| T3 hooks + installers | `scripts/githooks/*`, both installers | T1, T2, T4 |
| T4 gitattributes | `.gitattributes` | T1, T2, T3 |

Pairwise intersections are empty; T2 is serial after T1 by design, not by file overlap.

### T1: `scripts/gate.sh`
- **Steps, in order, each timed and recorded**: `fmt` (`cargo fmt --check`), `clippy-all` (`cargo clippy --workspace --all-features -- -D warnings`),
  `clippy-bare` (`cargo clippy --workspace -- -D warnings`), `test` (`cargo test --workspace --all-features --no-fail-fast -- --test-threads=1`, output teed to `target/gate/test.log`),
  `abort-grep` (fails if `test.log` has `SIGABRT` or `overflowed its stack`), `migrations` (numbers under `crates/htui-store/migrations/` unique and contiguous from 0001),
  `workflow-docs` (`validate-workflow-docs.sh`), `plugin` (`claude plugin validate --strict` and `claude plugin test` on `.claude/skills/htui-dev` when the dir exists and `claude` is >= 2.1.287, else `skipped`).
- **Behaviour**: runs every step even after a failure (`--no-fail-fast` spirit), exit 1 if any step failed. `--detach` relaunches under `setsid`/`nohup` with output to `target/gate/run.log` and returns the PID. `--status` prints `last.json`. `--only <step>` for a single step.
- **`last.json`**: `{ "head", "started", "finished", "state": "running|passed|failed", "steps": [{"name","status","seconds"}] }`, written atomically (temp + rename), and written as `running` at start so a poller sees it.
- **Mirror**: `validate-workflow-docs.sh` structure and Windows refusal. Includes `--self-test` that runs the migrations check and the abort grep against fixtures.
- **Validate**: `bash scripts/gate.sh --only migrations`, `--only abort-grep`, `--only workflow-docs`, `--self-test`; then one full run (long) via `--detach` and `--status`.

### T2: `scripts/gate.ps1`
- Same steps, names, `last.json` shape and exit codes as T1. `--detach` via `Start-Process`. Windows cannot be exercised in this sandbox; the check is `pwsh -NoProfile -File scripts/gate.ps1 -SelfTest` if `pwsh` is present, else parse-only, and the remainder is a host/Windows step (stated in the close-out, not claimed).

### T3: committed hooks and installer
- `scripts/githooks/pre-commit`: `#!/bin/sh`; on Windows (`uname`) runs the `.ps1` body, else the same logic as the current marker block (`WF_DOCS_PATTERN`, `--scope-paths "$WF_STAGED"`, `WORKFLOW_SKIP_HOOK=1`); then chains. No `exit 0` before the chain.
- Installers: add a `set_hooks_path` step (`git -C <root> config core.hooksPath scripts/githooks`) for the workspace entry, strip the own pre-commit block, keep sub-repo logic unchanged. Self-test cases: 14 hooksPath set and idempotent, 15 own pre-commit block removed while a foreign block survives, 16 the committed hook invokes the validator with `--scope-paths` and chains to `.git/hooks/<name>`.
- **Validate**: `bash .../install-workflow-hooks.sh --self-test` (currently 13/13), then in a scratch clone: stage a bad `HANDOFF.md` and confirm the commit is blocked, stage an unrelated file and confirm it passes, confirm a `.git/hooks/pre-commit` foreign block still runs.

### T4: `.gitattributes`
- One line, `DECISIONS.md merge=union`. **Validate**: scratch-repo merge of two branches each adding an index line yields both lines and no conflict; then run the validator on the result (see risk 1).

## Validation

```bash
bash .claude/skills/handoff-run/scripts/install-workflow-hooks.sh --self-test
bash scripts/gate.sh --self-test
bash scripts/gate.sh --detach && bash scripts/gate.sh --status   # full gate, then read target/gate/last.json
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Verified claims (step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `core.hooksPath` is unset and `.git/hooks` holds only samples | TRUE | `git config --get core.hooksPath` empty; `ls .git/hooks` samples only |
| No `.gitattributes` exists | TRUE | `read_file .gitattributes`: no such file |
| `/target` is gitignored, so `target/gate/` needs no ignore entry | TRUE | `.gitignore` line 1 |
| Migrations are `0001`-`0018`, contiguous | TRUE | listing of `crates/htui-store/migrations/` |
| Integration tests need `testkit` on `htui`, other crates use `test-support`; `--all-features` covers both | TRUE | `crates/htui/Cargo.toml:26`, `crates/htui-agent/Cargo.toml:15` |
| `install-workflow-hooks.sh` self-test is 13/13 today | TRUE | run: `Self-test: 13/13 cases PASS.` |
| Workflow validator is clean on this tree | TRUE | run: `0 error(s), 0 warning(s)` |
| `validate-workflow-docs.sh` supports `--scope-paths` | TRUE | header, lines 5-6 |
| The `.sh` entry points refuse Windows shells | TRUE | `install-workflow-hooks.sh:35-46`, same in `validate-workflow-docs.sh` |
| `claude` is >= 2.1.287 here; `claude plugin validate --strict` and `claude plugin test [dir]` exist | TRUE | `claude --version` 2.1.296; `--help` output for both |
| `.claude/skills/htui-dev` exists | **FALSE** | no `plugin.json`; TOOL-8 is open. Plan makes the step `skipped` |
| The installer's default targets work in this repo | **FALSE** | `engine` is missing; default `--targets "engine ../engine-template?"` exits 2. Plan makes the hooksPath step independent of targets |
| The workspace root is the repo root and hooks resolve in linked worktrees | PARTIAL | `git rev-parse --git-common-dir` is `.git`; relative `hooksPath` resolution in a linked worktree is asserted from git docs, **probed in T3 validate** |
| Union merge keeps the DECISIONS index valid | **UNVERIFIED** | union keeps both sides' lines but not their order; the validator requires newest-first. Probed in T4 validate |
| `pwsh` is available in the sandbox | **UNVERIFIED** | checked in T2 |

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Setting `core.hooksPath` silently disables foreign hooks (graphify, gortex) | High if unhandled | decision 1: chain to `.git/hooks/<name>`; self-test case 16 |
| Validator runs twice (committed hook plus old marker block) | Medium | decision 2: installer strips its own block; case 15 |
| `merge=union` interleaves index lines out of order and the validator then fails | Medium | validator catches it at commit; recorded in the write-up, fixed by hand. Not solvable by git attributes |
| A shell hook on Windows stalls (bash fork failure, measured 2026-08-21) | Medium | dispatch by `uname` to pwsh; Windows exercise is a host step, not claimed here |
| Full test step is long and flaky under load (qdrant timeouts, stack headroom) | Medium | detached run, `--test-threads=1`, abort-grep step; re-run failed steps with `--only` |
| `hooksPath` is local config, so a fresh clone has no hooks until the installer runs | Certain | documented in the write-up; the sandbox `scripts/hr` setup is the only place that could automate it (out of scope) |

## Acceptance

- [ ] `gate.sh` and `gate.ps1` run the listed steps, `--detach` works, `last.json` is written and readable by `--status`
- [ ] `scripts/githooks/` committed; installer sets `core.hooksPath`; foreign hooks still run; no double validation
- [ ] `.gitattributes` has `DECISIONS.md merge=union`
- [ ] installer self-test passes with the new cases; workflow validator 0/0
- [ ] reviewer gate (`rust-reviewer` per `workflow-config.json`) run over the change set, findings applied or deferred
- [ ] Windows `.ps1` paths marked as not exercised here unless `pwsh` is available
