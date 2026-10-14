# Plan: TOOL-8 M2 — `htui-dev` status line and turnless commands

**Source**: `HANDOFF.md` TOOL-8 (from ANA-29, `docs/ANA-29.md` §4.2 D5/D6/D9/D10, §6.1-6.3)
**Routed**: plan path (`/handoff-run`, 0 criteria fired, maintainer accepted, 2026-10-14). M1 (guards
G1-G6) landed `1d3963c3`..`40446f29`, plan `.claude/plans/tool-8-m1-guards.plan.md`.
**Status**: done 2026-10-14 (`28bb7c23`..`c38eb1b7`; review R1 approve with changes → fixed). TOOL-8 closed: `docs/decisions/tool/tool-8.md`.
**Complexity**: Medium (mod-only TypeScript plus one small script pair, no Rust change)

## Summary

M2 adds the ambient half of the `htui-dev` mod: one status line, refreshed by a 60 s timer, showing
the item and its HANDOFF phase, merge/ahead/dirty state, the disk when above 90%, Postgres when not
up, and the last gate result; plus three turnless slash commands: `/gate` (starts TOOL-9's gate
detached, the timer toasts its result), `/next` and `/validate` (the existing workflow scripts,
by platform). It closes TOOL-8.

## Requirements (restated)

| # | What | Behaviour |
|---|---|---|
| S1 | item | `HR_ITEM` when set, else the first `[A-Z]+-\d+` in the branch name (case-insensitive, upper-cased), else the branch alone |
| S2 | HANDOFF phase | for the item's open checklist line in `HANDOFF.md` (`- [ ] **ITEM - `): the last `**Phase <X> landed` in its body → `<X> landed`; no phase note → `open`; no open line → `not open` |
| S3 | merge | `merging` while `git rev-parse -q --verify MERGE_HEAD` succeeds |
| S4 | ahead/dirty | ahead of the upstream (`branch.ab` of `git --no-optional-locks status --porcelain=v2 --branch`); with no upstream and a branch other than `main`, `git rev-list --count main..HEAD`; dirty = the status entry count (untracked included). `--no-optional-locks` so the tick never takes `index.lock` beside a commit |
| S5 | disk | the repo root's filesystem use (`df -P`), shown only above 90% (the sandbox's `/` is the overlay, not the repo disk: `memory:dev-postgres-crash-is-disk-pressure`) |
| S6 | Postgres | shown only when not up: `pg recovering` (pg_isready exit 1) or `pg down` (exit 2, or the TCP fallback refused); `unknown` shows nothing |
| S7 | gate | from `target/gate/last.json` (`$.fs.read`): `gate running 4m`, `gate passed 2h`, `gate failed: test, abort-grep 40m`; `(older HEAD)` when its `head` is not HEAD; nothing when the file is absent |
| S8 | timer | `$.clock.every(60000)` armed in `session.start` when `isInteractive`, `HTUI_SESSION` unset and the `status_line` option is on; one tick at start; ticks never overlap (a `polling` flag, as `crates/htui/claude-plugin/htui/hooks/register.tsx` does). Re-armed lazily from the `tool.call` hooks when a hot reload dropped it (see V-open-1) |
| S9 | guard warnings | M1's `$.ui.status` warnings and the ambient line share the plugin's one status line: a warning leads the line (`⚠ <warning> · <ambient>`) for 10 minutes, then drops off at the next tick |
| C1 | `/gate [step\|status\|force]` | bare: refuse while `last.json` says `running` and started under 4 h ago (`/gate force` overrides); else start `bash scripts/gate.sh --detach [--only <step>]` / `pwsh -NoProfile -File scripts/gate.ps1 -Detach [-Only <step>]` and answer its one line. `status`: the steps of `last.json`, one line each. Unknown step: the step list |
| C2 | gate toast | the tick toasts `gate passed`/`gate failed: <steps>` when `last.json`'s `finished` differs from the `gateSeen` value in `$.state`, which `session.start` seeds from the file (no toast for a gate that ended before the session) |
| C3 | `/next` | runs `next-item.sh` / `next-item.ps1 -RepoRoot`; answers its stdout (stderr appended on a non-zero exit) and passes its exit code as `exitCode` |
| C4 | `/validate` | runs `validate-workflow-docs.sh --repo-root` / `.ps1 -RepoRoot`; answers as C3. `claude -p /validate` exits with the validator's code |
| P1 | platform | Windows when `$.env.get('OS') === 'Windows_NT'`: every script is the `.ps1` through `pwsh -NoProfile -File`; elsewhere `bash <script>.sh` |
| P2 | repo root | `git rev-parse --show-toplevel` run with `cwd: $.plugin.root`, once per module load |

Each command answers its own `command.run` (no `next`), registered by a literal name so `validate
--strict` lists it as "answers its own command", not as a gate. Commands are registered in every
session (they work in `-p` too); only the timer depends on `isInteractive`/`HTUI_SESSION`.

**`scripts/dev-health.sh` / `.ps1`** (new; logic lives in scripts with `.ps1` twins, ANA-29 §6.3)
prints one JSON line:
`{"disk":{"path":"…","used_pct":80},"postgres":{"state":"up|recovering|down|unknown","via":"pg_isready|docker|tcp|none","target":"localhost:5439"}}`.
Postgres: host/port from `HTUI_TEST_DATABASE_URL`, else `localhost:5439`; then the first that
exists of `pg_isready -h H -p P -t 3` (0 up, 1 recovering, 2 down, 3 unknown) → `docker exec
htui-postgres pg_isready -t 3` (the host, which has no psql: `memory:host-has-no-psql`) → a 3 s TCP
connect (`/dev/tcp`; `.ps1` a `TcpClient`) giving up/down only. `--self-test` (sh) covers DSN
parsing and the JSON shape, as `gate.sh --self-test` does.

The git, HANDOFF and `last.json` reads stay in the mod (git is called directly per §6.3; the file
reads and the phase parse are TypeScript the kit tests on every platform). Only disk and Postgres,
which have no portable binary, go through the script.

## Decisions for the maintainer

| # | Question | Recommendation |
|---|---|---|
| D1 | Where the status logic lives | As above: git/HANDOFF/gate in the mod (kit-tested everywhere); disk+Postgres in `scripts/dev-health.{sh,ps1}`. The alternative, one `dev-status` script emitting everything, puts the HANDOFF phase parse in bash and PowerShell, and the `.ps1` cannot be run here (no `pwsh`) |
| D2 | Toast cadence | The one 60 s timer polls `last.json`; a gate takes tens of minutes, so ≤60 s toast latency costs nothing and saves a second timer |
| D3 | Off switch | A `userConfig` boolean `status_line` (default on) stops the timer; `--safe-mode`/`disableAllHooks` stay the whole-mod switches (`docs/htui-dev.md`) |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Timer + process runner | `crates/htui/claude-plugin/htui/hooks/register.tsx` (`runHtui`, `poll`, `startTimer`, `checkVersion`) | never-throwing runner returning a line; a `polling` flag; module-level `timer` reset in `register`; `HTUI_SESSION` stand-down |
| Command | same file, `command.run` `{ command: 'htui' }` | verb split on the first space; usage text for an unknown verb; no `next` |
| Kit stubs | `crates/htui/claude-plugin/htui/tests/fixtures.tsx` (`ok`, `htui`, `start`) | `process.run` answered `{ value: { exitCode, stdout, … } }`; `session.start` bottom; `command.register` bottom; `mock.env`, `mock.clock` |
| Status/state | `.claude/skills/htui-dev/hooks/register.ts` (`warn`, atoms) and `types/index.d.ts` | `atom({ plugin: 'htui-dev', key }, initial)`, types declared in `PluginState['htui-dev']` |
| Script | `scripts/gate.sh` / `gate.ps1` | header usage block, Windows refusal for the `.sh`, `--self-test` with a failures list, twins in lockstep |
| Tests | `.claude/skills/htui-dev/tests/*.test.ts` | one behaviour per `test('<sentence>')`, helpers in `fixtures.ts` |

## Files to Change

| File | Action | Why |
|---|---|---|
| `scripts/dev-health.sh` | CREATE | disk + Postgres JSON line, `--self-test` |
| `scripts/dev-health.ps1` | CREATE | the twin (Windows run owed to the host) |
| `.claude/skills/htui-dev/hooks/status.ts` | CREATE | pure: item (S1), phase (S2), porcelain v2 parse (S4), gate summary (S7), line composition (S9) |
| `.claude/skills/htui-dev/hooks/commands.ts` | CREATE | platform argv (P1), `/gate` `/next` `/validate` runners (C1, C3, C4) |
| `.claude/skills/htui-dev/hooks/register.ts` | UPDATE | `session.start` (commands, `gateSeen` seed, timer), tick + toast, warning composition, lazy re-arm; header comment (it now runs processes and reads files) |
| `.claude/skills/htui-dev/types/index.d.ts` | UPDATE | `gateSeen` state |
| `.claude/skills/htui-dev/.claude-plugin/plugin.json` | UPDATE | description; `userConfig.status_line` |
| `.claude/skills/htui-dev/tests/fixtures.ts` | UPDATE | process/fs/toast/env stubs, session start |
| `.claude/skills/htui-dev/tests/{status,commands,ambient}.test.ts` | CREATE | kit tests, written first |
| `docs/htui-dev.md` | UPDATE | status line and commands sections; "What it never does" (now runs processes, reads files; still no writes, no network); known gaps; M2 no longer "to come" |
| `docs/decisions/tool/tool-8.md`, `DECISIONS.md`, `HANDOFF.md` | CREATE/UPDATE | P2 close-out: TOOL-8 done (M1 + M2) |

## Tasks

Serial, one implementer, TDD; commits per task. T1-T3 touch disjoint files (verified below), but
together they are a few hundred lines and T4 needs all three, so fan-out saves little.

### Task 1: `scripts/dev-health.{sh,ps1}`
- **Action**: self-test cases first (DSN with/without user/port, no DSN, JSON shape), then the script.
- **Mirror**: `scripts/gate.sh` header, Windows refusal, `run_self_test`.
- **Validate**: `bash scripts/dev-health.sh --self-test`; `bash scripts/dev-health.sh | jq .` here reads `up` via `pg_isready`.

### Task 2: `hooks/status.ts`
- **Action**: tests for S1, S2 (open/landed/not open; the item's body only, not the next item's), S4 (with and without `branch.ab`), S7 (running/passed/failed/older HEAD/absent/malformed), S9; then the functions.
- **Validate**: `claude plugin test .claude/skills/htui-dev`.

### Task 3: `hooks/commands.ts`
- **Action**: tests for argv per platform, `/gate` refusal while running, `force`, `--only`, `status`, unknown step, `/next` and `/validate` text and `exitCode`, a script that cannot start; then the runners.
- **Validate**: as T2.

### Task 4: wiring in `register.ts`
- **Action**: tests: timer armed only when interactive and not `HTUI_SESSION` and `status_line` on; first tick at start; a 60 s advance ticks again; no overlap; toast on a `finished` change, none for the seeded one; a guard warning leads the line then drops after 10 min; lazy re-arm. Then the wiring.
- **Validate**: `claude plugin validate --strict .claude/skills/htui-dev` lists `/gate`, `/next`, `/validate` as "answers its own command", calls only `$.clock.*`, `$.command.register`, `$.env.get`, `$.fs.read`, `$.process.run`, `$.ui.status`, `$.ui.toast`, `$.state` (no `$.fs.write`, no `$.http`); `claude plugin test`.

### Task 5: live smoke + docs + close-out
- **Action**: under a scratch `CLAUDE_CONFIG_DIR` (`memory:claude-live-probes-use-scratch-config`): `claude -p /validate` (output, exit code), `claude -p /next`; an interactive check of the status line if a pty is available, else owed to the host. Then the docs and the P2 close-out.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; `bash scripts/gate.sh --only plugin`.

## Validation

```bash
bash scripts/dev-health.sh --self-test
claude plugin validate --strict .claude/skills/htui-dev
claude plugin test .claude/skills/htui-dev
bash scripts/gate.sh --only plugin
bash scripts/gate.sh --only workflow-docs
```

No Rust changes, so no cargo gate beyond what `gate.sh` already runs.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| `gate`, `next`, `validate` are free command names in 2.1.296; a built-in is refused | true | scratch plugin under a scratch config: `validate --strict` lists the three as "answers its own command", and `status` (built-in) as "gating hook" |
| Registering in a loop makes `validate` list the commands as gates; literal names do not | true | same probe, both forms |
| The kit's `mock.clock` drives `$.clock.every`; `fs.read` is stubbed `{ value }` with `e.path`; `ui.toast` and `ui.status` need `{ value: undefined }` bottoms; `mock.env` answers `$.env.get` | true | probe test passed: one tick after `advance(60000)`, read/run/status/toast recorded |
| `$.plugin.root` is the plugin folder, so `${root}/…` paths resolve | true | probe: `root=/tmp/tool8-probe/plug`, read of `${root}/../x.json` reached `/tmp/tool8-probe/x.json` |
| A `command.run` answer `{ text, exitCode }` comes back through `$.command.run` | true | probe printed `{"text":"ok","exitCode":1}`; `-p` exit effect: types (`CommandRunResult.exitCode`), live check in T5 |
| A `$` call's time is not charged to the hook's 10 s budget | true | types `HookBudget`: "the clock stops while … any `$` call of the hook's is in flight" |
| `gate.sh --detach` returns at once, so `$.process.run` does not wait for the gate | true | `scripts/gate.sh`: `setsid bash … > run.log 2>&1 < /dev/null &`; types: `process.run` waits only on output left open |
| `gate.ps1` has `-Only`, `-Detach`, `-Status` | true | `scripts/gate.ps1:5,20-22` |
| `next-item.sh` and the validator finish in about a second and exit 0 on this tree | true | run here: 26 lines / 0; 1 line / 0 |
| `next-item.ps1` takes `-RepoRoot`; the `.sh` scripts take `--repo-root` | true | script usage lines (`next-item.ps1:17`, `next-item.sh:19`, `validate-workflow-docs.sh:8`) |
| `pg_isready` answers 0 for an up server, 2 for no response, and is on PATH in the sandbox | true | `pg_isready -h localhost -p 5439` → 0; port 5998 → 2. Exit 1 (recovery) per PostgreSQL docs, not reproduced |
| The host has no `pg_isready` and reaches Postgres through `docker exec htui-postgres` | unverified here | `memory:host-has-no-psql`, `memory:dev-postgres-crash-is-disk-pressure`; no docker in the sandbox — host smoke owed |
| `df -P <repo>` reads the repo disk, not the overlay `/` | true | `/dev/sda … 80% /home/mluigi/projects/htui` |
| `OS=Windows_NT` identifies Windows | unverified here | standard Windows environment; no Windows here — Windows run owed (as TOOL-9's) |
| `git rev-parse -q --verify MERGE_HEAD` detects a merge in a linked worktree; porcelain v2 omits `branch.ab` with no upstream | true | scratch repo + worktree: rc 0 while conflicted; no `branch.ab` line |
| T1, T2, T3 touch disjoint files | true | Files to Change: T1 `scripts/dev-health.*`; T2 `hooks/status.ts`, `tests/status.test.ts`; T3 `hooks/commands.ts`, `tests/commands.test.ts`; `fixtures.ts`, `register.ts`, `types`, `plugin.json` are T4's alone |

**Open, settled during implementation**: V-open-1, whether `session.start` fires again after a hot
reload. The types do not say. The lazy re-arm in S8 makes the answer irrelevant to behaviour; T5
records what a live reload does.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A tick's `git status` contends with a commit | Low | `--no-optional-locks`; read-only git calls only |
| Status noise on a machine with no dev Postgres (`pg down` forever) | Medium | `status_line` off switch (D3); documented; only non-`up` states show, and `unknown` shows nothing |
| `.ps1` twin and Windows `OS` detection untested here | Medium | written in lockstep with the `.sh`; owed Windows run recorded in HANDOFF (as TOOL-9) |
| A stale `running` `last.json` (killed gate) blocks `/gate` | Low | 4 h staleness bound and `/gate force` |
| One status line shared with M1 warnings hides one or the other | Low | S9 composition, kit-tested |

## Acceptance

- [ ] All tasks complete, kit tests first
- [ ] `validate --strict` green, calls list matches T4, the three commands "answer their own command"
- [ ] `claude plugin test` green; `dev-health.sh --self-test` green
- [ ] Live `claude -p /validate` and `/next` checked; rust-reviewer gate (repo `workflow-config.json`) run over the change
- [ ] TOOL-8 closed out (write-up, DECISIONS, HANDOFF), validator green
