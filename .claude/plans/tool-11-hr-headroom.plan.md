# Plan: TOOL-11 - hr sandbox: optional host headroom

**Source**: HANDOFF.md TOOL-11 (routed as plan; criteria fired: C3 only)
**Complexity**: Small-Medium (3 files + 1 test file; shell, compose, docs)

## Summary
`scripts/hr up --headroom` makes a run's Claude go through a headroom proxy; default stays off.
Decision (the item's first question): **option (b)** - each run starts its own `headroom proxy` inside
the container from the already-mounted tool - not (a), the host's proxy over the network.
Attach points Claude at it only after the proxy answers `/readyz`; otherwise it runs direct and says so.

## Decision record: (a) host proxy vs (b) per-run proxy
| Factor | (a) host proxy over the network | (b) per-run proxy in the container (chosen) |
|---|---|---|
| Reachability | `dev` shares `postgres`'s netns, so `localhost` is the container; needs `extra_hosts: host-gateway` on `postgres` and the host proxy bound to `0.0.0.0`/the bridge (default `127.0.0.1`) | `localhost:8787` in the run's own netns; no `ports:`, no host exposure, no collision with the host proxy |
| Host state | host proxy must be running (it is not by default: `headroom doctor` says unreachable) | nothing on the host needs to run |
| Exposure of credentials | host-wide listener that forwards bearer tokens; reachable by any container on the bridge | stays inside the run; headroom still sees every prompt and credential of that run |
| Cost | none | ~1.5 s start (measured), one more process per run |
| Failure when absent | `ANTHROPIC_BASE_URL` at a dead port **hangs claude** (measured: `claude -p hi` against `127.0.0.1:1` was still waiting at 90 s) | same hang unless attach guards it - see Task 3 |

## Findings that shape the design (all measured, see Verified claims)
- The mounted tool runs in the image: `headroom proxy` is ready in ~1.5 s with `UV_TOOL_DIR=/tmp/uv-tools`, read-only tool mounts, as the run user.
- A dead `ANTHROPIC_BASE_URL` does not degrade: so the env var is set **only after a readiness check**, per attach.
- `~/.headroom` is mounted read-write and shared. A container proxy writes `logs/` and `ccr_store.db` (SQLite WAL) into it; the first probe appended to the host's `proxy-8787.log`. `HEADROOM_WORKSPACE_DIR=/tmp/headroom` redirects all of it (measured: ccr store and logs land there, host log unchanged), so runs and the host never share a SQLite store. Cost: savings stats of a run die with it.
- The proxy's default beacon sends "session summaries" to Headroom Labs. `HEADROOM_BEACON=off` disables it (measured: "DISABLED"). The sandbox sets it off.
- Default log level did not write an `authorization` / `x-api-key` header or prompt text (canary request, 0 hits in the proxy log or the workspace dir).
- `REG_KEYS` is strict: a registry missing a key is invalid (`scripts/hr` `read_reg`). A new `headroom` key must be optional on read (absent = 0) or every existing run breaks.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Flag parsing | `scripts/hr` `cmd_up` option loop (`--ssd`, `--cpus`, `--yes`) | `case` arm per flag, `usage_die` on bad input, usage text updated |
| Per-run value | `scripts/hr` `load_run` / `cmd_up` step 9 (`HR_CPUS`) | `RUN[...]` -> `export HR_*` -> compose `${HR_...:-default}` |
| Registry | `scripts/hr` `REG_KEYS`, `read_reg` validation, `write_reg` | validated key, written in `cmd_up` |
| Compose env | `docker/hr/compose.hr.yaml` `dev.environment` (`UV_TOOL_DIR` comment style) | one env entry with a why-comment |
| Attach exec | `scripts/hr` `cmd_attach` (`exec compose exec ... dev claude ...`) | the only place a Claude process is launched |
| Tests | `tests/hr/hr.bats` | bats, fake `docker`; no daemon needed |
| Docs | `docs/hr-sandbox.md` "What is inside a sandbox" table and "Threat model" tables | row per shared mount / risk |

## Files to Change
| File | Action | Why |
|---|---|---|
| `scripts/hr` | UPDATE | `--headroom` / `--no-headroom` on `up`; optional `headroom` registry key; export `HR_HEADROOM` in `cmd_up` and `load_run`; guarded launch in `cmd_attach`; usage text |
| `docker/hr/compose.hr.yaml` | UPDATE | `dev.environment`: `HR_HEADROOM`, `HEADROOM_WORKSPACE_DIR`, `HEADROOM_BEACON`; start the proxy in the `dev` command when on |
| `docs/hr-sandbox.md` | UPDATE | flag in "Starting a run"; `~/.headroom` row; env list; threat-model row + residuals; troubleshooting |
| `tests/hr/hr.bats` | UPDATE | flag parsing, registry round-trip incl. an old registry without the key, attach guard branches |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/tool/tool-11.md` | UPDATE/CREATE | close-out per `lifecycle.md` P2 |

No Dockerfile change, so no image rebuild. No Rust.

## Tasks (strictly serial: every task touches `scripts/hr` or depends on it)
### Task 1: registry + flag (TDD)
- Tests first in `tests/hr/hr.bats`: `up --headroom` and `up --no-headroom` parse; `--headroom --no-headroom` last wins or errors (pick: error, like other conflicting flags); a registry file **without** a `headroom` line still loads with `RUN[headroom]=0`; a `headroom=maybe` line is `reg_bad`.
- `scripts/hr`: add `headroom` to `REG_KEYS` but treat it as optional in `read_reg` (default `0`, validate `0|1`), `write_reg` always writes it; `cmd_up` sets `RUN[headroom]`, exports `HR_HEADROOM`; `load_run` exports it from the registry; usage line `up [ITEM] [--ssd] [--from REF] [--cpus N] [--headroom|--no-headroom] [--yes]`.
- **Validate**: `bats tests/hr/hr.bats`.

### Task 2: compose wiring
- `dev.environment`: `HR_HEADROOM: ${HR_HEADROOM:-0}`, `HEADROOM_WORKSPACE_DIR: /tmp/headroom`, `HEADROOM_BEACON: "off"` (comment: why each).
- `dev.command`: when `HR_HEADROOM=1`, start `headroom proxy --port 8787` in the background (output to `/tmp/headroom/proxy.out`) before `exec sleep infinity`; `init: true` reaps it. Failure to start must not fail `up` (`|| true`, backgrounded).
- Does it need `--host`? No: default `127.0.0.1` is the run's own netns.
- **Validate**: `docker compose config` with the HR_* vars exported, for both `HR_HEADROOM=0` and `1`; real `up` on a scratch item: `curl localhost:8787/readyz` inside, and `HR_HEADROOM=0` starts nothing.

### Task 3: guarded attach (the "no hang" requirement)
- `cmd_attach`: when `RUN[headroom]=1`, run Claude through `bash -c` that polls `curl -fs -m 2 localhost:8787/readyz` for up to 15 s; ready -> `export ANTHROPIC_BASE_URL=http://localhost:8787` then `exec claude --dangerously-skip-permissions`; not ready -> print `hr: headroom is not answering; this session runs direct` on stderr and `exec claude` without the variable. Same guard for `--shell` (export only when ready).
- A proxy that dies mid-session is **not** recoverable by this guard; documented as a residual (requests fail until the user re-attaches; attach re-checks and falls back).
- **Validate**: bats with a fake `docker` recording the exec argv for ready / not-ready; live: kill the proxy in a scratch run, re-attach, confirm the direct warning and a working `claude -p hi`.

### Task 4: docs
- "Starting a run": the flag, default off, what it does, the fallback line.
- "What is inside a sandbox": `~/.headroom` row now says read-write, shared, **but a headroom-on run keeps its proxy state in `/tmp/headroom`**; env list gains `HR_HEADROOM`, `HEADROOM_WORKSPACE_DIR`, `HEADROOM_BEACON`.
- Threat model: new row/paragraph "headroom on": the proxy sees every prompt, tool result and credential of the run (it forwards the bearer token upstream); a hostile run already holds those, so the added exposure is the proxy's compression of them and its third-party code path, not a new credential reach; telemetry beacon forced off; the proxy listens on the run's `localhost` only. Residuals: proxy dies mid-session; savings are not recorded to the host; `headroom` version is whatever the host has installed.
- Troubleshooting: "headroom is not answering" (check `/tmp/headroom/proxy.out` via `attach --shell`).
- **Validate**: `scripts/validate-workflow-docs.sh`.

## Verified claims (step 3.5)
| Claim | Verdict | Evidence |
|---|---|---|
| Mounted headroom runs in the image under the sandbox's uv/tool env | true | `docker run` with the four-mount subset, `UV_TOOL_DIR=/tmp/uv-tools`, run uid: `headroom 0.40.0`, `/readyz` 200 in ~1.5 s, upstream check healthy |
| Host proxy is not running by default and binds `127.0.0.1` | true | `headroom doctor`: unreachable at 127.0.0.1:8787; `headroom proxy --help`: `--host` default 127.0.0.1 |
| `dev` shares `postgres`'s netns, so (a) needs `extra_hosts` on `postgres` | true | `compose.hr.yaml:44` `network_mode: service:postgres`; header comment on shared netns |
| A dead `ANTHROPIC_BASE_URL` makes claude hang rather than fail fast | true | `ANTHROPIC_BASE_URL=http://127.0.0.1:1 claude -p hi` hit the 90 s timeout with no output |
| `HEADROOM_WORKSPACE_DIR` moves the proxy's state off the shared `~/.headroom` | true | probe 2: `/tmp/hw` held `ccr_store.db*` and `logs/`; host `proxy-8787.log` size unchanged |
| `HEADROOM_BEACON=off` disables the upload | true | proxy log: "Anonymous upload beacon: DISABLED" |
| Default logging omits credentials and prompt | true for the default level | canary `authorization`, `x-api-key` and prompt text: 0 hits in proxy output and in `/tmp/hw` + `~/.headroom` |
| Existing registries lack a `headroom` key and `read_reg` rejects missing keys | true | `scripts/hr` `read_reg`: "missing key" loop over `REG_KEYS` -> Task 1 makes it optional |
| Tasks independent | **none are**: all four touch `scripts/hr` or its tests / docs sequentially | file-set intersection non-empty -> serial |
| `docs/hr-sandbox.md` already lists `~/.headroom` as shared rw | true | doc lines 446 and 488 |

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| Old registries rejected after the key is added | High if done naively | optional key + bats case with an old file (Task 1) |
| Proxy dies mid-session, Claude hangs on a dead base URL | Medium | documented residual; attach re-guards; consider a loop wrapper later (out of scope) |
| Headroom logs a credential at a higher log level | Low | default level verified; docs forbid `HEADROOM_LOG_MESSAGES` in a run |
| Compression changes Claude behaviour in a run (cache mode default) | Low | opt-in, default off |
| Probe wrote debris into host `~/.headroom/logs` (first probe, no redirect) | Done | one appended proxy log; harmless, reported |

## Acceptance
- [ ] `up --headroom` run: proxy ready on the run's `localhost:8787`, Claude launched with the base URL; `up` without the flag unchanged
- [ ] proxy down -> attach prints the direct-run line and does not hang
- [ ] existing registries still load
- [ ] bats green; compose config valid in both modes; workflow validator green
- [ ] `docs/hr-sandbox.md`: flag, `~/.headroom` row, risk table
- [ ] No host `~/.headroom` writes from a headroom-on run (check mtime of `ccr_store.db*`)
