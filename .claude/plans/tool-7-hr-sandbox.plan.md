# Design: TOOL-7 — containerized handoff-run sandboxes

> **Status: CONFIRMED 2026-09-29 (`/handoff-run TOOL-7`, routed as plan by maintainer override of
> a threshold PRD verdict); fact-checked; implementation in progress.** Task breakdown and the
> verified-claims table are at the end of this document. Claims #2–#4 can only be settled by the
> T2 spike, which gates every Docker-dependent task.

**Source**: HANDOFF.md TOOL-7. Maintainer request: "when running the handoff-run skill I'd like to
run it in a dev container, with all the necessary tools, so that I can run multiple without
interfering with each other."

## Goal and success criteria

Run **3–4 `/handoff-run` lifecycles concurrently** on this box (12 cores, 61 GiB RAM, 108 GB free on
the NVMe `/`, 3 TB free on the spinning `/media`), each in its own sandbox with the full toolchain,
such that:

1. No run can change another run's files, databases, build output, or git state.
2. No run can change the host repo, its branches, or the host's `htui-postgres` / `htui-qdrant`.
3. No run holds git credentials; work leaves only when the maintainer collects it on the host.
4. Every maintainer gate of `handoff-run` stays interactive (routing confirm, CONFIRM, PRD open
   questions, push).
5. Two runs never mint the same HANDOFF ID; same-problem duplicates are surfaced, not silently filed.
6. Inside a sandbox, Claude runs with the same skills, plugins, MCP servers, hooks and memory as on
   the host, in `--dangerously-skip-permissions` mode.

## Decisions (maintainer, 2026-09-29)

| # | Decision |
|---|----------|
| D1 | Terminal-driven: a host script `scripts/hr` creates sandboxes; the maintainer attaches `claude` to each via `docker exec -it`. No VS Code / `devcontainer` CLI. |
| D2 | Source tree = a **fresh clone per run** from the host repo mounted read-only; branch `hr/<ITEM>` off current local `main` (unpushed commits included). |
| D3 | Budget for 3–4 concurrent runs; bulk data may live on `/media` (HDD, 3 TB free), with an opt-in SSD placement. |
| D4 | Claude runs `--dangerously-skip-permissions` inside the sandbox. |
| D5 | Approach A: one **docker compose project per run** (`hr-<item>`), not a single fat container, not devcontainer.json. |
| D6 | `scripts/hr` uses **gum** (host has v2.0.2) for its TUI; every verb also takes plain arguments. |
| D7 | **No git credentials in the sandbox.** Host-side `hr collect` fetches the run branch; `--merge` merges into the current branch. |
| D8 | Host skills/tools are brought in per the inventory below; the host `.remember/` is **shared rw** across runs. |
| D9 | Tracked as TOOL-7; this design lives at `.claude/plans/tool-7-hr-sandbox.plan.md`. |

## Architecture

### One run = one compose project `hr-<item>`

`docker/hr/compose.hr.yaml`, instantiated as `docker compose -p hr-<item-lowercase> …`:

| service | image | notes |
|---|---|---|
| `postgres` | `postgres:16` | Owns the run's network namespace; `command: postgres -p 5439 -c fsync=off -c synchronous_commit=off -c full_page_writes=off`; `POSTGRES_HOST_AUTH_METHOD=trust`; `shm_size: 1gb` (as `compose.yaml`); healthcheck `pg_isready -p 5439`. Data in a per-run named volume (SSD, `/var/lib/docker`). |
| `qdrant` | `qdrant/qdrant:v1.19.1` | `network_mode: service:postgres`; default ports 6333/6334; per-run named volume. |
| `dev` | `htui-hr-dev` (local build) | `network_mode: service:postgres`; `command: sleep infinity`; `cpus: ${HR_CPUS:-4}`; `depends_on: postgres (healthy)`. |

All three share one network namespace, so inside a run `localhost:5439`, `localhost:6333`,
`localhost:6334` mean exactly what they mean on the host: every DSN in docs, memories and
`compose.yaml` comments works unchanged, including the password-free gate
`postgres://postgres@localhost:5439/postgres`. **No ports are published**, so runs coexist with
each other and with the host's `htui-postgres` (host port 5439) and `htui-qdrant`. Compose project
scoping namespaces container names, the network and volumes.

### Storage placement

| data | location | why |
|---|---|---|
| Run directory `$HR_ROOT/<ITEM>/` → `src/` (clone, incl. `target/` and any `.claude/worktrees/*/target`) | `HR_ROOT` default `/media/projects/htui-hr`; `hr up --ssd` → `~/htui-hr` | Bulk (10–40 GB per run, more with in-run worktree fan-out) on the disk that has room. `CARGO_TARGET_DIR` is **not** overridden, so in-run worktrees keep their own `target/` exactly as today. |
| Postgres / Qdrant data | per-run named volumes | Small, fsync-heavy → SSD. |
| Cargo registry + git cache | shared named volume `htui-hr-cargo` (`$CARGO_HOME/registry`, `$CARGO_HOME/git`) | Cargo file-locks it; runs don't re-download crates. |
| Shared state `.state/` (run registry, ID leases) | always `/media/projects/htui-hr/.state`, mounted at `/hr-state` | Fixed path regardless of `--ssd`, so every run and the host see one copy. |

The clone is mounted in the container at **`/home/mluigi/projects/htui`** — the host path — so
Claude's per-project key (`-home-mluigi-projects-htui`), auto-memory, Gortex path resolution and
absolute paths in docs line up. Consequence (accepted): sessions from every run land in the same
`~/.claude/projects/-home-mluigi-projects-htui/`, as parallel host sessions already do.

The host repo is mounted **read-only** at `/host/htui` and is the clone's `origin`.

### Toolchain image `htui-hr-dev` (`docker/hr/Dockerfile`)

- Base `ubuntu:24.04` — the host's release, so host binaries mounted from `~/.local` run unmodified.
- User `mluigi` with host uid/gid (build args), `HOME=/home/mluigi`.
- rustup with the `rust-toolchain.toml` pin (**1.98.1**) + `rustfmt`, `clippy`, **`rust-analyzer`**
  (serena's Rust LSP). The pin's `components` list omits `rust-analyzer`, so the image adds it
  explicitly (`rustup component add rust-analyzer --toolchain 1.98.1`).
- `sqlx-cli` (`--no-default-features --features postgres,rustls`, version = `sqlx` in `Cargo.lock`),
  `cargo-insta`, `postgresql-client-16` (PGDG apt), `build-essential`, `pkg-config`, `git`, `jq`,
  `ripgrep`, `curl`, `ca-certificates`.
- **Not baked in** (mounted from the host, so a sandbox never drifts from the host's versions):
  Claude Code, Gortex, graphify, headroom, uv/uvx, serena.

### Host mounts into `dev`

| host | container | mode | purpose |
|---|---|---|---|
| `~/.local` | same path | ro | `claude` (mise install), `gortex`, `graphify`, `headroom`, `uv`/`uvx` + managed Pythons |
| `~/.claude/` | same path | rw | auth (`.credentials.json`), settings, user skills, plugins (superpowers, remember), memory, sessions |
| `~/.claude.json` | **copied** into the container at `hr up` | per-run | Claude rewrites it by atomic rename, which a single-file bind mount cannot take. Same MCP config as host; in-run changes don't flow back. |
| `~/.gortex/{config.yaml,instructions,models}` | same path | ro | same Gortex profile and models |
| `~/.gortex` cache + `sidecar.sqlite` | per-run volume | rw | each run runs **its own Gortex daemon** over its own clone; the host daemon's store is never shared |
| `~/.headroom` | same path | rw | one place for compression stats |
| `~/.serena` | same path | rw | serena global config/logs |
| `~/.cache/uv` | same path | rw | shared uv cache (6.6 GB; uv locks it) |
| host repo `.remember/` | `<clone>/.remember` | rw | one shared remember timeline (D8) |
| host repo | `/host/htui` | ro | clone origin |
| `$HR_ROOT/<ITEM>/src` | `/home/mluigi/projects/htui` | rw | the run's clone |
| `/media/projects/htui-hr/.state` | `/hr-state` | rw | run registry, ID leases |
| **not mounted** | — | — | `~/.gitconfig` (carries `credential.helper store`), `~/.git-credentials`, `~/.ssh` |

`hr up` also copies the host repo's **untracked** `.claude/settings.local.json` into the clone: it
carries the Gortex hooks (8 events) and headroom's `SessionStart` self-heal, which call
`/home/mluigi/.local/bin/{gortex,headroom}` by absolute path.

### Environment in `dev`

`USERNAME=htui-ci`, `HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres`,
`HTUI_TEST_QDRANT_URL=http://localhost:6334` (gates `crates/htui-store/tests/qdrant_live.rs`),
`HR_ITEM=<ITEM>`, `HR_SANDBOX=1`, `HR_STATE=/hr-state`, `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL`/`GIT_COMMITTER_NAME`/
`GIT_COMMITTER_EMAIL` from host `git config user.*`, `PATH` including the mise install dirs for
`claude`, `uv`, `ripgrep` **ahead of `~/.local/bin`** (which holds a stale uv/uvx 0.8.22 that
would otherwise shadow mise's 0.12.x — serena's MCP entry runs `uvx`; the Gortex MCP entry runs
bare `gortex`, so `~/.local/bin` must still be on `PATH`), and `TERM`/`COLORTERM` passthrough. `SQLX_OFFLINE` stays as
`.cargo/config.toml` sets it. No D-Bus, no keyring: suites use `mock_keyring()`, a real keyring read
fails fast.

### Brought in / left out (D8 inventory)

Brought in: repo skills/agents/commands (in the clone: `handoff-run`, `handoff-add`,
`code-architect`, `rust-reviewer`, `gortex-*`); user skills in `~/.claude/skills` (`graphify`,
`find-skills`, `learned`, `gortex-*`); enabled plugins superpowers and remember; MCP gortex,
headroom, serena; graphify CLI.

Left out: MCPs already disabled for this project (fetch, semble, sigmap, sequential-thinking(-tools),
cloudflare, chrome-devtools, claude.ai connectors — the seeded `~/.claude.json` keeps them
disabled); kandev (host `127.0.0.1:38429`, unreachable, already disabled); `gh` (no credentials; PRs
from the host); Node/Volta (only disabled MCPs need it); other mise toolchains.

## `scripts/hr` (host, bash + gum)

| verb | behaviour |
|---|---|
| `hr` | `gum choose` over the verbs. |
| `hr build` | Build `htui-hr-dev` with host uid/gid; `gum spin`. |
| `hr up [<ITEM>] [--ssd] [--from <ref>]` | No ID → `gum filter` over HANDOFF open items, minus items with a registered run. Refuses an ID that already has a run. **Warns (and lists files) when the host tree has uncommitted changes under `.claude/`, `HANDOFF.md` or `DECISIONS.md`** — the clone takes committed `<ref>` (default `main`) only. Clones, switches to `hr/<ITEM>`, copies `settings.local.json` and `~/.claude.json`, `compose up -d`, registers the run in `/hr-state/runs/<ITEM>`. |
| `hr attach [<ITEM>] [--shell]` | No ID → `gum choose` over live runs. `docker compose exec dev claude --dangerously-skip-permissions`, or a shell. |
| `hr ls` | `gum table`: item, state, branch, commits ahead of host `main`, disk use, root (HDD/SSD). |
| `hr collect <ITEM> [--merge] [--force]` | `git fetch <run>/src hr/<ITEM>:hr/<ITEM>` into the host repo (creates or fast-forwards; refuses a rewritten history unless `--force`); prints `git log --oneline main..hr/<ITEM>`. Never touches the working tree or checked-out branch. `--merge`: refuses on dirty tracked files, then `git merge --no-ff hr/<ITEM>` into the **current** branch and runs `validate-workflow-docs.sh`. Never pushes. |
| `hr down <ITEM> [--purge]` | `compose down`. `--purge`: `gum confirm`, refuses unless `hr/<ITEM>` was collected (or `--force`), then `down -v` and removes the run directory. |
| `hr gc` | Offers (`gum confirm`) to purge stopped + collected runs; prunes ID leases (below). |

Missing gum → exit naming it. Missing image → `hr up` offers `hr build`.

## Cross-run coordination

### ID leases — `scripts/hr-mint`

`next-item-id.sh` derives the next ID from its own tree only, so two runs spawning items in
parallel would both mint the same ID. `scripts/hr-mint --prefix <P> --title "<one line>"`:

1. `flock /hr-state/id-leases.lock` (host: `/media/projects/htui-hr/.state/…`).
2. Run `next-item-id.sh --prefix <P> --id-only` (non-zero exit blocks, as today).
3. Result = `max(tree mint, highest leased <P> + 1)`; append `<ID>\t<hr branch or "host">\t<ISO time>\t<title>`.
4. Print the ID, plus other runs' leases for `<P>` from the last 7 days with titles.

`next-item-id.sh` / `.ps1` stay **untouched** (twin parity unaffected). The three mint call sites —
`handoff-run` (`references/lifecycle.md` P0), `handoff-add` step 3 and the overview
`.claude/skills/handoff-docs.md` — mint through `hr-mint` **whenever the lease file exists** — on the host too, so a host `/handoff-add` cannot reuse a sandbox's leased ID. A
leased-then-abandoned ID is a harmless gap (owned-ID method takes the max). `hr gc` prunes a lease
once its ID is on host `main` (open line or archive index) or its run was purged uncollected; the
pruning itself is `hr-mint --prune` so the lease-file format has exactly one owner. State dir
resolution: `$HR_STATE`, else `/media/projects/htui-hr/.state`; no state dir → `hr-mint` behaves as
`next-item-id.sh --id-only` and says leasing is off.

**Same problem, different IDs.** Step 4's listing exists for this: the skill tells the agent to
**ask the maintainer** before minting when a sibling lease's title looks like the same issue. On
reuse, the run references the sibling's ID in its phase note rather than creating the item (no
dangling link for that run's validator). Residual duplicates surface at `--merge` review.

### Selection

In a sandbox, `/handoff-run` runs exactly `$HR_ITEM`: `next` or a bare call resolves to it; a
different ID is asked, never followed silently. Choosing the item happens on the host in `hr up`'s
picker, which excludes items with a registered run.

### Merge-back conflicts

Concurrent runs touching HANDOFF.md (checklist lines, the summary table) will conflict textually at
merge. `merge=union` is rejected (duplicates summary rows, resurrects ticked boxes); deferring item
creation to merge is rejected (breaks the in-run validator's link checks). The resolution recipe is
fixed:

1. keep every new checklist line from both sides (IDs are distinct by construction);
2. take the other side's edits to existing lines;
3. recount the summary table from the merged checklist;
4. `validate-workflow-docs.sh` — non-zero means not done.

`hr collect --merge` stops on any conflict. When the only conflicted paths are `HANDOFF.md`,
`DECISIONS.md` and `docs/decisions/**`, it offers (`gum confirm`) to open an interactive host
`claude` session primed with that recipe; the maintainer stays present. Code conflicts are always
left to the maintainer. The real Postgres gate on the merged tree runs on the host with no runs
building (memory: concurrent lanes pushed the dev Postgres into `57P03`).

## `handoff-run` sandbox mode (`HR_SANDBOX=1`)

A new SKILL.md section overriding:

- **Step 1 / 1.5** — the item is `$HR_ITEM` (see Selection).
- **Step 5.4** — never push; the done-report ends `branch hr/<ITEM> ready — on host: scripts/hr collect <ITEM>`.
- **P0 mint** — through `scripts/hr-mint` (with the ask-on-similar-title rule).
- **DB access** — `psql -h localhost -p 5439 -U postgres`; no `docker exec` (no Docker in the
  sandbox). The sqlx-prepare memory's `docker exec htui-postgres psql …` recipe gets a sandbox variant.
- Everything else, including every maintainer gate and the reviewer gate, is unchanged.

## Verification

1. **T0 spike (gates the rest).** One image, one run. Must show: `claude mcp list` inside lists
   gortex, headroom, serena ✔; the Gortex `SessionStart` banner appears and `gortex` indexes the
   clone; remember appends to the shared `.remember/`; 3 sandboxes + host run `claude -p`
   concurrently across an OAuth refresh without logging each other out.
   Fallbacks: Gortex fails → Gortex off in the sandbox (the profile's own integration-failure
   clause); auth fails → `claude setup-token` long-lived token passed as env.
2. **bats suite** `tests/hr/*.bats` (bats is on the host via mise): `hr-mint` 20 parallel mints →
   20 unique IDs, tree-vs-lease max, sibling listing, gc pruning; `hr collect` fetch-only leaves the
   host tree untouched, refuses non-fast-forward, `--merge` refuses on a dirty tree; `hr up` warns
   on uncommitted workflow files and refuses a duplicate item. Docker-touching cases tagged so the
   rest run in seconds.
3. **Non-interference acceptance.** 3 runs up simultaneously, each running
   `cargo test --workspace --all-features -- --test-threads=1` at the same time. Pass = all three
   green, no `57P03`, host `htui-postgres` uptime unchanged, host tree and `main` byte-identical
   before/after. Record disk and wall time per run on `/media` vs `--ssd`; the numbers set the
   default `HR_ROOT`.

## Deliverables

- `docker/hr/Dockerfile`, `docker/hr/compose.hr.yaml`
- `scripts/hr`, `scripts/hr-mint`
- `tests/hr/*.bats`
- `.claude/skills/handoff-run/SKILL.md` sandbox-mode section; `references/lifecycle.md` P0,
  `.claude/skills/handoff-add/SKILL.md` step 3 and `.claude/skills/handoff-docs.md` mint via
  `hr-mint` when the lease file exists
- `docs/hr-sandbox.md` + README pointer
- Memory updates: sqlx-prepare (`docker exec` recipe sandbox variant), worktree-implementers
  (in-sandbox fan-out)

## Out of scope

VS Code / devcontainer.json; Windows or macOS hosts (the `.ps1` twins get no `hr` counterpart);
pushing from a sandbox; opening PRs from a sandbox; sccache (revisit if the acceptance run shows
dependency builds dominate); resource quotas beyond `cpus`.

## Claims to verify (for the plan fact-check)

1. Host is Ubuntu 24.04 and `~/.local` binaries (`claude`, `gortex`, `headroom`, uv Pythons) run in `ubuntu:24.04`.
2. `gortex` can run a daemon in a container with a fresh store and the ro config/models mounts.
3. Claude Code honours an uncommitted `.claude/settings.local.json` in the clone (hooks fire).
4. Concurrent OAuth refresh on a shared `~/.claude/.credentials.json` is safe.
5. `~/.claude.json` is rewritten by rename (single-file bind mount unsafe).
6. Postgres accepts `-p 5439` with `network_mode: service:postgres` peers reaching it on `localhost:5439`; Qdrant binds 6333/6334 in the shared namespace.
7. `sqlx` version in `Cargo.lock` has a matching `sqlx-cli` release.
8. The store suites need only `CREATEDB` on the maintenance DSN and pass against a `trust`-auth server.
9. `next-item-id.sh --id-only` prints only the ID.
10. `lifecycle.md` P0 and `handoff-add` step 3 are the only mint call sites.
11. `.remember/` is untracked in the repo (so it is not in the clone).
12. gum v2.0.2 supports `choose`, `filter`, `table`, `spin`, `confirm` with the flags used.

## Tasks

Ordering: **wave 1** T1 → T2 (spike) on the Docker side, with T3 in parallel (no Docker);
**wave 2** T4 ∥ T5 after T2 passes; **wave 3** T6. Every implementer commits incrementally with
explicit paths (no `git add -A`, `git stash`, `--amend` while another agent shares the tree).
TDD where there is a test surface: the bats cases land before the script they cover.

### T1 — Toolchain image and run compose file

Files: `docker/hr/Dockerfile`, `docker/hr/compose.hr.yaml`, `docker/hr/.dockerignore`.

- Dockerfile per "Toolchain image": `ubuntu:24.04`, build args `UID`/`GID`/`USER=mluigi`,
  rustup 1.98.1 + `rustfmt` `clippy` `rust-analyzer`, `cargo install sqlx-cli --version 0.9.0
  --locked --no-default-features --features postgres,rustls`, `cargo-insta`, PGDG
  `postgresql-client-16`, apt basics. Cargo registry/git under a path the shared `htui-hr-cargo`
  volume mounts over.
- compose file per "One run = one compose project", every host path and per-run value from env
  (`HR_SRC`, `HR_ITEM`, `HR_CPUS`, `HR_STATE_HOST`, `HR_CLAUDE_JSON`, git identity, …) so
  `scripts/hr` is the only thing that knows the defaults; no `ports:` anywhere; `.remember`,
  `~/.claude`, `~/.local` etc. mounts exactly per the mounts table; `~/.gitconfig`, `~/.ssh`,
  `~/.git-credentials` absent.
- Done when: `docker build` succeeds; `docker compose -p hr-t1 … up -d --wait` on a throwaway clone
  gives a `dev` where `cargo --version` is 1.98.1, `cargo sqlx --version` is 0.9.0,
  `psql postgres://postgres@localhost:5439/postgres -c 'select 1'` works, `claude --version` and
  `gortex version` run, `git config --global credential.helper` is empty; then `down -v`.

### T2 — T0 spike (gate; main thread + maintainer)

Files: this plan (spike results section); fixes to T1 files if a check fails.

Checks, with a throwaway run brought up by hand from the T1 compose file:
1. `claude mcp list` inside lists gortex, headroom, serena connected.
2. An attached interactive `claude` shows the Gortex `SessionStart` banner and the Gortex daemon
   indexes the clone under the per-run store (claim #2), and the untracked
   `settings.local.json` hooks fire (claim #3) — **maintainer attaches**.
3. The remember plugin appends to the shared host `.remember/`.
4. Three sandboxes + the host run `claude -p` concurrently and repeatedly across an OAuth token
   refresh without logging anyone out (claim #4). If a refresh can't be forced within the spike,
   record that and keep the `claude setup-token` fallback ready.

Fallbacks per "Verification" §1. **Any fallback taken amends this plan before wave 2.**

### T3 — `scripts/hr-mint` (parallel with T1/T2)

Files: `scripts/hr-mint`, `tests/hr/test_helper.bash`, `tests/hr/hr-mint.bats`.

- Tests first: a fixture repo (minimal `HANDOFF.md`) plus `HR_STATE` in a tmpdir; cases for
  20 parallel mints → 20 unique IDs; `max(tree, lease+1)` both ways; a non-zero exit from
  `next-item-id.sh` blocks; the sibling listing (other branches, last 7 days, with titles);
  `--prune` drops leases whose ID is on the given ref (open line or archive index) or whose run
  is listed as purged; no state dir → plain tree mint + "leasing off" notice.
- Interface: `hr-mint --prefix P --title T [--repo-root R] [--branch B]` (prints the ID on stdout,
  siblings on stderr); `hr-mint --prune [--ref main] [--repo-root R]`; `hr-mint --leasing`
  (exit 0 iff the lease file exists — the skill docs' switch).

### T4 — `scripts/hr` (after T2)

Files: `scripts/hr`, `tests/hr/hr_helper.bash`, `tests/hr/hr.bats`.

- Every verb in the `scripts/hr` table; gum for interaction, plain args for everything; `hr gc`
  calls `hr-mint --prune`, never edits the lease file itself.
- Tests first (non-Docker cases run in seconds, Docker cases tagged `docker`): `collect`
  fetch-only leaves the host tree and HEAD untouched; refuses non-fast-forward without `--force`;
  `--merge` refuses on dirty tracked files; `up` warns on uncommitted `.claude/`, `HANDOFF.md`,
  `DECISIONS.md` and refuses an item with a registered run; `down --purge` refuses an uncollected
  branch. Git-side cases run against fixture repos, never the real host repo.

### T5 — Skill sandbox mode and docs (after T2; parallel with T4)

Files: `.claude/skills/handoff-run/SKILL.md`, `.claude/skills/handoff-run/references/lifecycle.md`,
`.claude/skills/handoff-add/SKILL.md`, `.claude/skills/handoff-docs.md`, `docs/hr-sandbox.md`,
`README.md`.

- SKILL.md "Sandbox mode (`HR_SANDBOX=1`)" section per the design; P0 / step 3 / overview mint via
  `scripts/hr-mint` when `scripts/hr-mint --leasing` succeeds, with the ask-on-similar-title rule.
- `docs/hr-sandbox.md`: user guide (build, up, attach, collect, merge recipe, gc, storage, what is
  and is not in the sandbox). README pointer.
- `validate-workflow-docs.sh` green.

### T6 — Acceptance and close-out (main thread)

Files: this plan (results), memories outside the repo (sqlx-prepare sandbox variant,
worktree-implementers in-sandbox fan-out), HANDOFF/DECISIONS bookkeeping.

- Non-interference acceptance per "Verification" §3 (3 runs, `--test-threads=1` workspace suite
  each, host `htui-postgres` uptime and host tree/`main` unchanged); record per-run disk and wall
  time on `/media` vs `--ssd` and set the default `HR_ROOT` from them.

### Independence (file-set intersection)

| pair | intersection | verdict |
|---|---|---|
| T1 ∩ T3 | ∅ | parallel |
| T3 ∩ T4 | ∅ (`test_helper.bash` vs `hr_helper.bash`; `hr` reaches leases only through `hr-mint --prune`) | parallel-safe; T4 still waits for T2 |
| T4 ∩ T5 | ∅ | parallel |
| T3 ∩ T5 | ∅ (T5 documents the T3 interface fixed above) | parallel-safe |
| T1 ∩ T4 | ∅ files, but `hr` consumes the compose file's env names | serial by dependency |

### Risk noted

`.claude/skills/handoff-run/**`, `handoff-add/**` and `handoff-docs.md` are a *synced workflow
surface* distributed from a workspace source by `sync-workflow-surface.sh` (default targets
`engine ../engine-template?`, not htui). htui's copy is already edited in place (85448c8), and T5
extends that divergence. **Accepted by the maintainer at CONFIRM (2026-09-29):** these skills are
what htui was started for and the workflow is migrating to htui, so htui's copy is allowed to lead;
no sync back into htui is planned.

## Verified claims

| # | claim | verdict | evidence |
|---|---|---|---|
| 1 | Host is Ubuntu 24.04; `~/.local` binaries run in `ubuntu:24.04` | ✓ | `/etc/os-release` 24.04, glibc 2.39; probe container: `claude` 2.1.284, `gortex` v0.64.5, `headroom` 0.38.0, `uvx`, `graphify` all exit 0 |
| 1a | PATH inside mirrors host tool versions | ✗ → amended | `~/.local/bin/uv{,x}` is 0.8.22 (2025) vs mise 0.12.20; mise dirs must precede `~/.local/bin` |
| 2 | Gortex daemon runs in a container with a fresh store | ✓ (T2) | sandbox `claude -p`: SessionStart banner "cwd `/home/mluigi/projects/htui` is tracked … 27795 nodes", daemon indexed the clone in ~17 s. Needed fix `4459066`: stale `daemon.pid` in the per-run volume blocked respawn after a container restart; `dev` start command clears the runtime files |
| 3 | Untracked `.claude/settings.local.json` in the clone is honoured | ✓ (T2) | all 4 SessionStart hooks (Gortex, superpowers, remember, headroom) `hook_response` exit 0 in the sandbox stream-json |
| 4 | Concurrent OAuth refresh on a shared `.credentials.json` is safe | argued ✓, observe in T6 (maintainer 2026-09-29) | `~/.claude` is a bind-mounted directory on the same filesystem, so atomic renames and lock dirs behave as for the parallel host sessions already in use; forced refresh not run (it would edit the live auth file). Watch for logouts if T6 crosses the token expiry; fallback `claude setup-token` |
| 5 | `~/.claude.json` is rewritten by rename | ✓ | inode 2149922 → 2149837 across one minute of this session |
| 6 | Postgres `-p 5439` + `network_mode: service:postgres` peers reach it on `localhost`; Qdrant binds 6333/6334 in the shared namespace; nothing published | ✓ | probe compose: `psql …@localhost:5439` → PG 16.15, `fsync=off`; `GET :6333` → 200; `:6334` open; `docker compose ps` shows no publishers |
| 7 | `sqlx` in `Cargo.lock` has a matching `sqlx-cli` with the named features | ✓ | lock 0.9.0; crates.io `sqlx-cli` 0.9.0 has `postgres`, `rustls`; MSRV 1.94 ≤ 1.98.1 |
| 8 | Store suites need only a `CREATEDB` maintenance DSN; trust auth is fine | ✓ | `crates/htui-store/src/testkit.rs:5` (maintenance DSN with `CREATEDB`); no role DDL in any crate's `src`/`tests` |
| 8a | Postgres is the only live test backend | ✗ → amended | `qdrant_live.rs` gates on `HTUI_TEST_QDRANT_URL`; added to the env list |
| 9 | `next-item-id.sh --id-only` prints only the ID | ✓ | prints `TOOL-8`, exit 0 |
| 10 | `lifecycle.md` P0 and `handoff-add` step 3 are the only mint call sites | ✗ → amended | third: `.claude/skills/handoff-docs.md:41`; added to T5 |
| 10a | Toolchain pin brings `rust-analyzer` | ✗ → amended | `rust-toolchain.toml` components are `rustfmt`, `clippy` only; image adds it |
| 11 | `.remember/` is untracked (not in the clone) | ✓ | `.remember/.gitignore` is `*`; `git ls-files .remember` empty |
| 12 | gum v2.0.2 has `choose`/`filter`/`table`/`spin`/`confirm` with the flags used | ✓ | all five `--help` exit 0; `table -c/-s/-p/-f`, `filter --header/--placeholder` present |
| — | Task independence | ✓ | intersection table above; every task lists its files |

## T2 spike results (2026-09-29)

One sandbox brought up by hand from `docker/hr/compose.hr.yaml`; headless `claude -p` inside
(maintainer: headless is enough, no interactive attach).

| check | result |
|---|---|
| `claude mcp list` | gortex ✔, headroom ✔, serena ✔ (after fix); host-disabled servers disabled |
| auth + model inside | ✔ `claude-opus-5-5[1m]`, answered `hr/SPIKE` from the clone |
| plugins | ✔ remember, superpowers (+ agents-md) |
| hooks | ✔ Gortex / superpowers / remember / headroom SessionStart exit 0 |
| remember → shared `.remember/` | ✔ writes land in host `.remember/tmp` |

Fixes (`4459066`): `UV_TOOL_DIR=/tmp/uv-tools` (uvx writes temp files into its tool dir even for
ephemeral tools; `~/.local` is ro); `dev` start command clears Gortex `daemon.{pid,sock,spawn.lock,spawn.fail}`.
No fallback taken. F9 (share Gortex `memories/`): **not shared** — each run's Gortex memories live
in its volume and die at purge; durable decisions belong in the repo docs anyway.

Accepted limitation: remember's save lock records a PID, and container PIDs are meaningless on the
host (and vice versa), so a host save and a sandbox save racing each other can break each other's
lock; worst case an interleaved `now.md` entry. Documented in `docs/hr-sandbox.md`, not engineered.

## Threat model (after the review gate, 2026-09-29)

Goals 1 and 2 hold against **accidental** interference only. A sandbox is not a boundary against a
prompt-injected agent running with `--dangerously-skip-permissions`; the user-facing version is
`docs/hr-sandbox.md`, "Threat model".

Shared read-write channels, and what a hostile run can do through each:

| channel | allows |
|---|---|
| `~/.claude` (by design, goal 6): login, settings, plugins, skills, agents, commands | hooks or settings that execute in the host's next Claude session; the shared login |
| `~/.claude/projects/-home-mluigi-projects-htui` (re-review M-C) | this project's host transcripts; its auto-memory, loaded by the next host session |
| `~/.cache/uv` | packages that `uvx` runs on the host |
| `~/.headroom`, `~/.serena` | state and config the host's headroom and serena read |
| shared `.remember/` (M6) | e.g. `now.md` replaced by a symlink to a host-checkout file; the host remember plugin then writes into the checkout |
| `/hr-state` | delete the lease file or hold its lock; bounded by the fail-closed rules (`scripts/hr-mint` header, `scripts/hr` `check_leases`) |
| `htui-hr-cargo` | the shared download cache and index (`registry/cache`, `registry/index`); `registry/src` is a per-run volume, `bin/` an empty read-only tmpfs (cargo searches it for subcommands ahead of `PATH`, M-A), config/credentials empty read-only binds, and `up`/`attach` refuse a mount point of the wrong kind (L8b) |

Protected: host repo read-only; no git credentials; host Postgres/Qdrant published on `127.0.0.1`
only (`compose.yaml`, H2); per-run databases and volumes; host-side git never reads a clone's
worktree outside a throwaway `--network none` container (H3); run registry host-only
(`HR_RUNS`) and validated (H4), an old `$HR_STATE/runs` imported once only (M-B); `~/.local`
narrowed to four read-only tool dirs (C1); in `~/.claude`, everything that holds other projects' or
the host's own session data is masked per run (M-C): tmpfs over `projects` (the htui project dir
bound back rw), `file-history`, `paste-cache`, `shell-snapshots`, `session-env`, `session-data`,
`sessions`, `daemon`, `jobs`, `backups`, `metrics`, `.remember`; per-run empty files over
`history.jsonl`, `bash-commands.log`, `cost-tracker.log`. `collect --merge` prints the diffstat of
`.claude scripts docker CLAUDE.md AGENTS.md .mcp.json .cargo rust-toolchain.toml` and requires a
confirmation before every merge that changes them, fetching into a temporary ref so a refusal
leaves the host's refs as they were (L1). Advice: read that diff before confirming; after a
distrusted run, inspect `~/.claude/settings.json`, hooks and the htui auto-memory.

Residuals (after the re-review fixes, 2026-09-29):

- the shared cargo download cache and index: a run that rewrites a cached `.crate` and its index
  entry can still reach a later extraction in another run (`registry/src` itself is per run now);
- lease-lock DoS, bounded by the 30 s `HR_MINT_LOCK_TIMEOUT` (mints exit 3);
- a clone's `.git/commondir` can redirect host ref reads, but only to make its own run look collected;
- deleting both `id-leases.tsv` and `id-leases.lock` no longer makes host mints tree-only: host
  `hr-mint` reads `$HR_RUNS/.leases-initialized` too (L3), provided it sees the same `HR_RUNS`.

## T6 acceptance results (2026-09-29)

Three real runs brought up with `scripts/hr up --yes` (≈6 s each): CLEAN-4 and CLEAN-6 on `/media`
(HDD), MOD-59 with `--ssd`; each ran `cargo test --workspace --all-features -- --test-threads=1`
from a cold `target/` **simultaneously** (4-CPU quota each).

| run | root | result | tests passed | Postgres/Qdrant skips | `57P03` | wall | run dir |
|---|---|---|---|---|---|---|---|
| CLEAN-4 | hdd | ✔ rc 0 | 2453 | 0 / 0 | 0 | 411 s | 15 GB |
| CLEAN-6 | hdd | ✔ rc 0 | 2453 | 0 / 0 | 0 | 419 s | 15 GB |
| MOD-59 | ssd | ✔ rc 0 | 2453 | 0 / 0 | 0 | 388 s | 15 GB |

Host checks: repo HEAD/`main`/status/diff/every branch byte-identical before vs after; host
`htui-postgres`/`htui-qdrant` same `StartedAt`, 0 restarts, no recovery lines in the log.
**Default `HR_ROOT` stays `/media/projects/htui-hr`:** the HDD costs ~7% wall time on a cold full
run, while 15 GB per run on the SSD (89 GB free after this test) would crowd the host's own
`target/`. `--ssd` remains the opt-in. Claim #4 (concurrent OAuth refresh) was not exercised: the
runs made no Claude calls; it stays an argued pass with the documented `claude setup-token`
fallback.
