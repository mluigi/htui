# TOOL-8 - `htui-dev` mod: guards, status line and turnless commands (done, 2026-10-14)

From ANA-29 (`docs/ANA-29.md` §4.2 D1-D6, D8-D10, §6). Plans: `.claude/plans/tool-8-m1-guards.plan.md` (M1),
`.claude/plans/tool-8-m2-status-commands.plan.md` (M2). User guide: `docs/htui-dev.md`. No Rust changed.

A skills-directory Claude Code mod at `.claude/skills/htui-dev/`, loaded in place by every trusted checkout and
sandbox clone. It needs no install step and runs on Claude Code 2.1.287 or later.

## M1 - guards (`1d3963c3`..`40446f29`, phase note `a0ae47b6`)

Six guards, each a `{ deny }` that names the fix. None rewrites a call, and all fail open with a status warning:

| Guard | Refuses |
|---|---|
| G1 | a `cargo test` that leaves feature-gated integration tests off (it models the command line, dev-dependency features and `[features]` forwarding) |
| G2 | the other crate's test feature name (`testkit` / `test-support`) |
| G3 | the forbidden literal written into `crates/**/*.{rs,json}` by an edit tool (built-in, Gortex, Serena) |
| G4 | file writes and the listed git writes from an agent started read-only, workflow agents included |
| G5 | a subagent asked to run on Fable |
| G6 | `git commit --no-verify` / `-n` |

The milestone has 88 kit tests and went through two review rounds. Two questions were closed live: a
skills-directory mod needs nothing beyond workspace trust, and `agent.spawn` fires for Workflow agents.

## M2 - status line and commands (`28bb7c23`..`c38eb1b7`)

- **Status line**: in an interactive session that htui does not drive, with the `status_line` option on (the
  default), a 60 s timer composes one line. It holds the item (`HR_ITEM`, else the branch's `PREFIX-N`) and its
  HANDOFF phase (the last `**Phase X landed` inside the item's own line), `merging`, commits ahead of the upstream
  (or of `main`), dirty entries, the disk above 90%, `pg recovering|down`, and the last gate (`target/gate/last.json`,
  with skipped steps, a one-step run's name and `(older HEAD)`). The line is sent only when it changes, and a guard
  warning leads it for 10 minutes. `git status` runs with `--no-optional-locks`.
- **Gate toast**: once per gate that finishes during the session. `gateSeen` in `$.state` is seeded when the timer
  arms, so a gate that ended before the session never toasts.
- **Commands**, each answering its own `command.run`, registered by literal name in every session:
  - `/gate [step|status|force]` starts `gate.sh --detach [--only]` / `gate.ps1 -Detach [-Only]`. It refuses while a
    gate that started under 4 h ago is running.
  - `/next` runs `next-item`, and `/validate` runs `validate-workflow-docs`.

  Each answer is the script's text with its exit code, which `claude -p "/validate"` exits with. Windows runs the
  `.ps1` through `pwsh -NoProfile -File` (`OS=Windows_NT`).
- **`scripts/dev-health.sh` / `.ps1`** (new twins) print one JSON line with the repo disk's use (`df -P` on the repo,
  not `/`) and the dev Postgres state. The Postgres probes are tried in order: `pg_isready`; then, for a local
  target, the `htui-postgres` container's state and a `docker exec pg_isready`; then a TCP connect.
- **Reload**: live, saving a file of the mod re-raises `session.start`, which re-arms the timer. As a fallback, the
  first main-loop tool call re-arms from the `ambient` flag; that path is tested in the kit only.
- 168 kit tests (80 new), `validate --strict` green, and `tsc` 5.9 strict + `noUncheckedIndexedAccess` clean over
  the whole mod. The mod had never been type-checked before.

## Review (rust-reviewer, M2)

The verdict was approve with changes: no CRITICAL or HIGH findings, three MEDIUM and nine LOW, all fixed in
`c38eb1b7`.

The MEDIUM findings:
- A stopped container read as `recovering`.
- `gate.ps1 -Detach` passed its redirected pipes to the gate on Windows, which would have made `/gate` wait on the
  gate. It now launches through ShellExecute with no redirection, and the gate writes its own log.
- A failing state store stopped the line's refresh.

One LOW finding, a refused status line never resent, can only be fixed for a throw: `$.ui.status` returns `void`.
That limit is recorded in `docs/htui-dev.md`.

Also fixed: `gate.ps1 -SelfTest`'s `last.json` shape case never passed (TOOL-9). `ConvertFrom-Json` reads the ISO
time as a `DateTime`. Both gate self-tests now also check that the mod's step list has not drifted.

## Verified

- Kit tests, validator, `tsc`, `dev-health.sh --self-test`, and `gate.sh --self-test` / `gate.ps1 -SelfTest`. Both
  `.ps1` files ran under PowerShell 7.5.3 on Linux, fetched into the sandbox (`DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1`).
- A fake `docker` exercised the docker path of both twins: stopped → `down`, running → `up`, and a remote target
  goes to TCP.
- Live under a scratch configuration:
  - `claude -p /validate` and `/next` answer the scripts. `/validate` exited 1 on a worktree with a broken HANDOFF.
  - An interactive pty session set `TOOL-8 M1 landed · 6 ahead · gate passed 2m (older HEAD)` 0.1 s after
    `session.start` and refreshed it every 60 s.
  - A save reloaded the mod and re-raised `session.start`.
- `gate.ps1 -Detach` returned in 271 ms through a pipe on Linux. The Windows branch's `-Command` string runs the gate
  and writes `run.log`.

## Known residuals

- **Owed on the host:**
  - a Windows run: `claude -p "/gate fmt"` returns at once, `/next` and `/validate` run the `.ps1`, and
    `dev-health.ps1` reads the right drive;
  - the container path of `dev-health` against the real `htui-postgres`;
  - the Postgres gate on the merged tree.
- The re-arm-from-a-tool-call fallback has not run live, because a reload has always re-raised `session.start` so far.
- A guard warning lives in module memory, so a reload drops it early.
- A failed `last.json` read at arm time seeds nothing, so a pre-session gate may toast once.
- `/gate`'s 4 h staleness bound is a guess, with `/gate force` as the way out.
- M1's known gaps stand (`docs/htui-dev.md`, "Known gaps").
