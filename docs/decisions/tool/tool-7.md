# TOOL-7 - Containerized handoff-run sandboxes (shipped, 2026-09-29)

Three or four `/handoff-run` lifecycles can now run side by side on this box, each in its own
sandbox, without getting in each other's way or touching the host checkout. User guide:
`docs/hr-sandbox.md`. Design, decisions D1–D9, fact-check, spike and acceptance results:
`.claude/plans/tool-7-hr-sandbox.plan.md`; implementation blueprint:
`.claude/plans/tool-7-hr-sandbox.blueprint.md`. Routed as **plan** by maintainer override of a
threshold PRD verdict (the design doc already filled the PRD's role).

## What shipped

- **One compose project per run** (`docker/hr/compose.hr.yaml`, `hr-<item>`): toolchain image
  `htui-hr-dev` (`docker/hr/Dockerfile`: Ubuntu 24.04, Rust 1.98.1 + rust-analyzer, sqlx-cli 0.9.0,
  cargo-insta 1.48.0) plus a private Postgres 16 and Qdrant v1.19.1 sharing one network namespace,
  so `localhost:5439/6333/6334` mean inside what they mean on the host. Nothing is published.
- **A fresh clone per run** on `hr/<ITEM>`, mounted at the host repo's path; the host repo is only
  mounted read-only as the clone's `origin`. No git credentials inside. Host tools (Claude Code,
  Gortex, headroom, graphify, uv/serena) come from four read-only `~/.local` tool dirs, so a sandbox
  never drifts from the host; each run runs its own Gortex daemon over its own clone.
- **`scripts/hr`** (bash + gum, every verb also plain-argument): `build`, `up`, `attach`, `ls`,
  `collect [--merge]`, `down [--purge]`, `gc`. Work leaves a sandbox only when the maintainer runs
  `collect` on the host (fetch into a temporary ref, fast-forward check, then `update-ref`; `--merge`
  shows the diffstat of `.claude`/`scripts`/`docker`/`CLAUDE.md`/… and validates with the pre-merge
  validator). Host-side git never reads a clone's worktree outside a throwaway `--network none`
  container. The run registry is host-only (`HR_RUNS`) and validated field by field.
- **`scripts/hr-mint`** — cross-run ID leases under `flock`: the minted ID is the max of the run's
  tree mint, the host tree (inside a sandbox) and the highest lease + 1; `--prune` never removes a
  prefix's highest lease; a vanished lease file fails closed. The owned-ID rule gained a "Lease
  raise" paragraph (`.claude/rules/workflow-docs.md`, maintainer-approved), and lifecycle P0,
  `handoff-add` step 3 and `.claude/skills/handoff-docs.md` mint through `hr-mint` when `--leasing` succeeds.
- **`handoff-run` sandbox mode** (`HR_SANDBOX=1`): the item is `$HR_ITEM`, nothing is pushed, the
  done-report ends with the `scripts/hr collect` line; every maintainer and reviewer gate unchanged.
- **Host dev ports on loopback** (`compose.yaml`): Postgres 5439 and Qdrant 6333/6334 are published
  on `127.0.0.1` only; on `0.0.0.0` the dev database was reachable from every sandbox.

## Threat model

The sandbox isolates runs from **accidents**, not from a prompt-injected agent running with
`--dangerously-skip-permissions`: `~/.claude` (settings, plugins, skills, credentials, the htui
project dir) is shared read-write by design, as are `~/.cache/uv`, `~/.headroom`, `~/.serena` and the
host `.remember/`. Cross-project Claude data (other projects' transcripts, file-history,
paste-cache, shell snapshots, history, cost logs, …) is masked per run. Full table and residuals:
`docs/hr-sandbox.md` § Threat model.

## Verification

- 85 bats cases (`tests/hr/`, 56 `hr` + 29 `hr-mint`) green, plus five Docker cases D1–D5 (image,
  isolation incl. the gateway probe and the masks, two runs' databases apart, purge, hostile clone).
- Acceptance: three real runs (two HDD, one `--ssd`) ran the whole workspace suite simultaneously —
  2453 tests passed in each, no Postgres/Qdrant skips, no `57P03`; host repo byte-identical and host
  `htui-postgres` untouched. 15 GB and ~7 min per cold run; `HR_ROOT` stays on `/media` (HDD costs
  ~7%). Concurrent OAuth refresh was not exercised (accepted by argument; `claude setup-token` is
  the documented fallback).
- Review gate (`rust-reviewer`, three rounds): BLOCK (1 critical: the whole-`~/.local` mount leaked
  credential undo histories and keyrings; 6 high incl. a sandbox-defined git clean filter executing
  on the host and a sandbox-writable run registry) → approve with fixes → approve. Every finding,
  LOWs included, was fixed and pinned by a test (maintainer decision).

## Commits

`08f8fbc` plan · `0696688` blueprint · `f4efa04`, `e6bfd11` image and compose (T1) · `d177c48`,
`3f66429` hr-mint (T3) · `f27c440`, `4813bee` spike fixes and results (T2) · `0084edd`, `fc88762`,
`50aa884`, `9554a6b`, `2bdf2d6` hr (T4) · `a2027c7`, `676884c`, `fa23ed8`, `fd2a821` docs and
sandbox mode (T5) · `82a6fc3` rule amendment · review fixes `d737c84`, `aa1f157`, `14ebd2b`,
`f31d819`, `8504643`, `bb1466c`, `ce9b18e`, `e481134`, `5aae30f`, `e694c31`, `0bb7f04`, `4d87731`,
`623ea8f`, `a1791a6`, `02185c1`, `93201c3`, `3d1f541`, `c270bf5`, `ddf7c30`, `951080e`, `213d901`.
