# TOOL-11 - hr sandbox: optional host headroom (done, 2026-10-13)

`scripts/hr up ITEM --headroom` (default off; `--no-headroom` is the explicit default, giving both is refused with
exit 2) routes a run's Claude through [headroom](https://github.com/headroomlabs-ai/headroom). Plan:
`.claude/plans/tool-11-hr-headroom.plan.md`. User guide: `docs/hr-sandbox.md`, "Headroom in a run".

## Decision: a proxy per run, not the host's

Option (b) of the item: each run starts its own `headroom proxy` in the container from the already-mounted
`~/.local/share/uv/tools`. Option (a), the host's proxy, was rejected: `dev` shares `postgres`'s network
namespace, so reaching the host needs `extra_hosts` on `postgres` and a host proxy bound beyond `127.0.0.1`
(a credential-forwarding listener for every container on the bridge), and the host proxy is not running by
default. In (b) the proxy listens on the run's own `localhost:8787`: no published port, no host dependency.

## Built

- `scripts/hr`: `headroom` registry key, **optional on read** (absent = 0) so existing runs still load; written by
  `up`, upgraded by any `write_reg`. `attach` of a `headroom=1` run execs a guard (`HEADROOM_GUARD`) that
  starts the proxy if none runs (under `flock`, anchored `pgrep -f`), waits up to 30 s for `/readyz`, then sets
  `ANTHROPIC_BASE_URL` for that Claude or shell only. Not ready: one stderr line and a direct session.
- Proxy state is `HEADROOM_WORKSPACE_DIR=/tmp/headroom`, set on that one process, so the shared `~/.headroom` is
  never written by a run's proxy.
- Tracking: `compose.hr.yaml` `dev.environment` sets `HEADROOM_BEACON=off`, `HEADROOM_TELEMETRY=off`,
  `HEADROOM_NO_SUBSCRIPTION_TRACKING=1` and `DO_NOT_TRACK=1` for every run, flag or not (added after the
  maintainer saw a proxy log "Anonymous upload beacon: ENABLED"); the guard repeats them inline for containers
  made earlier. Cost: the next `up`/`attach` recreates a stopped run's container (new compose environment).
  The host's own headroom is untouched; headroom has no persistent opt-out, only the environment.
- The compose file carries no proxy start command. A first version added one plus an `HR_HEADROOM` variable;
  review found it left two starters racing for the port, so the guard became the only starter.
- Tests: `tests/hr/hr.bats` 3e-3m (flag parsing, conflict, old-registry load and upgrade, tamper cases, compose
  unchanged, attach argv for both modes, guard against fake `curl`/`pgrep`/`headroom`); 95 fast cases green.

## Measured (the claims the design rests on)

- A dead `ANTHROPIC_BASE_URL` makes `claude -p` hang (still waiting at 90 s), so the variable is set only after a
  readiness check.
- The mounted headroom runs in the image under the sandbox's env; cold first start took 6 to 15 s (hence the 30 s
  bound), a restart about 4 s.
- A canary bearer token, API key and prompt were not written to the proxy log at the default level.
- Live guard runs in the image: concurrent cold attaches start one proxy; warm attach is instant; a killed proxy is
  restarted; an unrelated `headroom` CLI process does not suppress the start.

## Known residuals

- A proxy dying mid-session fails that session's requests until the next `attach`.
- A run's savings are not added to the host's totals; its proxy data dies with the container.
- Registries written by this version carry a `headroom=` line an older `scripts/hr` rejects.
- Not run end to end: a real `up --headroom` on a real item (host step).

Commits: `2df13b92`, `9598a8d8` (compose, reverted in `852bc884`), `7ca05953`, `4c3df118`, `852bc884`.
