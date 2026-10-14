# TOOL-14 - Gate reports skipped Postgres suites (done, 2026-10-14)

From the 2026-10-14 requirements audit. Every Postgres-backed suite (`*_pg.rs`, the store conformance runs)
prints `testkit::SKIP` and returns when `HTUI_TEST_DATABASE_URL` is unset, and cargo hides a passing test's
stdout, so `scripts/gate.sh` reported `PASSED` with the whole Postgres surface unexercised. The audit's gate did
exactly that (6,220 passed); the same suite with the DSN set ran the Postgres cases live.

## Decision: a `postgres` step, skipped when the DSN is unset

Option taken: a new step before `test`, so the existing "skipped steps are named" machinery (TOOL-9 `run_gate`,
TOOL-8's status line and toast) carries the warning with no new channel. Refusing the gate outright was rejected:
a quick host gate without Postgres stays useful, it just may not read as a Postgres green.

- unset → `skipped` ("the Postgres suites return early and prove nothing"); the gate ends
  `PASSED (1 skipped: postgres)`;
- set, `postgres[ql]://…host[:port]` reachable over TCP within 3 s → `passed`;
- set and unreachable → `failed` (the suites would fail anyway, this names why first);
- set in a shape the regex does not parse (a key/value or socket DSN) → `passed`, "not probed".

The DSN is never printed (it may carry a password); only `host:port` is.

## Built

- `scripts/gate.sh`: `step_postgres`, `STEPS` and usage gain `postgres`; self-test cases for unset, unreachable,
  unparsed and "password never printed".
- `scripts/gate.ps1`: `Step-Postgres` (TCP probe via `System.Net.Sockets.TcpClient`), `$Steps`, `ValidateSet`
  and the same four self-test cases.
- `.claude/skills/htui-dev/hooks/commands.ts` `GATE_STEPS` and its two tests; `docs/htui-dev.md` step list.

## Verified

- `bash scripts/gate.sh --self-test`: all cases pass (it also checks `GATE_STEPS` lockstep).
- `claude plugin test .claude/skills/htui-dev`: 168 pass; `claude plugin validate --strict`: passed.
- `gate.sh --only postgres`: unset → skipped, `PASSED (1 skipped: postgres)`; with
  `postgres://postgres@localhost:5439/postgres` → `localhost:5439 reachable`, passed.

## Owed on the host

`gate.ps1` was not run (no `pwsh` on this box): `pwsh -NoProfile -File scripts/gate.ps1 -SelfTest` on Windows.

Landed on branch `audit/requirements-2026-10-14`.
