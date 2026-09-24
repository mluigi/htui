# ANA-16 - Research agent execution environments (Docker, remote shell) (done, 2026-09-24)

Research how to run an agent in a Docker container (local and remote) and in a remote shell, and
whether that requires a central server with htui as just the interface.

**Verdict: phased.** Phase 1 needs no server: a headless `htui worker` per executing box
(`R-ORCH-12`) claims runs by `target_box_id` and lease and talks directly to Postgres, which already
coordinates concurrent writers (status CAS, box-row lock on admission, per-process leases with
`SKIP LOCKED`). Eight multi-writer gaps (C1-C8: no lease fence on step writes, hostname-keyed boxes,
silently dropped duplicate `seq`, client-clock lease times, unordered quota writes, last-writer-wins
agent edits, schema skew) are fixed first because they apply to any design. Docker is an execution
environment modelled as a child box of the host running `dockerd`, driven by that host's worker over
`docker exec -i` with identical-path bind mounts, so adapters are unchanged. A remote shell is only
how a remote box's worker is provisioned; SSH is not an agent transport.

Phase 2 adds a self-hosted control-plane server (`htui server`) that workers dial out to: enrolment
and box ids, the only worker-facing DSN, dispatch, live and permission relay, a versioned config
manifest and secret distribution, target build pinning. It holds no durable state, so `R-ID-3`
stands, and the TUI keeps talking to Postgres. It opens on a trigger: a worker outside the trusted
network or behind NAT, team use (`R-USR-3`), worker count beyond the Postgres connection budget, or
`NOTIFY` streaming proving inadequate. A full server in front of the TUI too was rejected.

The first draft rejected any central server; the maintainer challenged it (concurrent writes, config
management) and the second round produced the concurrency audit (§6.1), the config inventory
(§6.2) and the phased verdict. Amendment record: `docs/ANA-16.md` §10.

Spawned: MOD-37 multi-writer hardening, MOD-38 headless worker, MOD-39 permission relay, MOD-40
remote dispatch, MOD-41 container environment, MOD-42 SSH provisioning, MOD-43 `NOTIFY` streaming
(optional), MOD-44 control plane (phase 2), MOD-45 config manager (phase 2). Requirement amendments
(`R-ID-2`, `R-ORCH-12`, `R-NF-2`, `R-STO-1`, `R-STO-5`, `R-USR-3`, `R-SEC-2`) are recorded as open
questions inside those items; `docs/REQUIREMENTS.md` is unchanged.

Commits: `ac6c2bc` (draft), `c7cd822` (central-server revision), and the close-out commit.

See `docs/ANA-16.md` for the full analysis.
