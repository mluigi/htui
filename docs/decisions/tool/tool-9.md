# TOOL-9 - Canonical gate script, committed git hooks, DECISIONS merge rule (done, 2026-10-14)

From ANA-29 (`docs/ANA-29.md` §4.1, §4.2 D5, D7, D8). Plan: `.claude/plans/tool-9-gate-hooks.plan.md`. No Rust changed.

## Built

- `scripts/gate.sh` / `scripts/gate.ps1` (twins, same step names and `last.json` shape): `fmt`, `clippy-all`
  (`--workspace --all-features -D warnings`), `clippy-bare` (featureless), `test` (`--all-features --no-fail-fast
  -- --test-threads=1`, output to `target/gate/test.log`), `abort-grep` (`SIGABRT` or `overflowed its stack` in that
  log), `migrations` (`NNNN_name.sql` unique and contiguous from 0001), `workflow-docs`, `plugin` (`claude plugin
  validate --strict` and `test` on `.claude/skills/htui-dev` when it exists and `claude` is at least 2.1.287, else
  `skipped`). Every step runs after a failure; the final line names skipped steps. `--only <step>`, `--detach`
  (setsid / hidden process, log in `target/gate/run.log`), `--status`, `--self-test`. `target/gate/last.json` is
  written as `running` at the start and replaced atomically after each step.
- `scripts/githooks/pre-commit` (+ `pre-commit.ps1` for Windows) runs the validator with `--scope-paths` on staged
  workflow docs, then chains to `$(git rev-parse --git-common-dir)/hooks/pre-commit`; `post-commit` only chains.
- `install-workflow-hooks.sh` / `.ps1` set `core.hooksPath` to `scripts/githooks` for the workspace (before target
  resolution, so the missing `engine` target here does not block it), remove their own pre-commit marker block (the
  committed hook replaces it, so the validator runs once), and warn about any other hook left in `.git/hooks`.
  Self-test is 16/16 (sh) and 14/14 (ps1). Case 16 runs the committed hook in a scratch repo: an unrelated commit
  passes, a dangling `DECISIONS.md` index line is blocked, `WORKFLOW_SKIP_HOOK=1` lets it through, the foreign hook
  in `.git/hooks` still runs.
- `.gitattributes`: `DECISIONS.md merge=union`. Measured in a scratch repo: two branches each adding an index line
  merge without a conflict, but union does not order them, so the result can break the newest-first rule; the
  validator catches that at commit.

## Review (rust-reviewer, shell and PowerShell only)

Fixed: ps1 `Step-Plugin` compared an array to the verdict (a passing validate would have failed the step); a green
gate now names its skipped steps; the installers warn about unchained hooks; ps1 self-test count and labels;
`pre-commit.ps1` mode.

## Known residuals

- **`core.hooksPath` is repo-wide, relative to each worktree.** A checkout on a branch without `scripts/githooks/`
  (an older `hr/*` branch, or `main` until this merges) runs no hooks at all, and the installer's old pre-commit
  block is gone from `.git/hooks`. Cut worktrees from a branch that has this commit. A per-worktree or
  git-common-dir dispatcher would remove the trap and was left out (it changes the item's shape).
- Only `pre-commit` and `post-commit` are chained; the installer warns about the rest.
- `last.json` stays `running` if the gate is killed, `--detach` has no lock, and `--only abort-grep` reads whatever
  `test.log` is there. Not fixed; a pid field and a HEAD stamp would close them.
- `abort-grep` matches the bare word `SIGABRT` as the item specifies; a test that prints it would fail the step.
- bash 3.2 (macOS) is not supported: empty-array expansion under `set -u`, and no `setsid`.
- **Not run:** any `.ps1` (no `pwsh` in the sandbox; judged by review only), a Windows commit through the hook, and
  the host Postgres gate.
- The first full gate run failed two `htui-agent` tests (`launch::an_interrupt_lets_the_child_exit_on_its_own_terms`,
  `cli_driver::a_cancel_drains_a_late_result_...`) while other jobs were running; both pass alone. Not touched by this
  change.
