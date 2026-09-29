# Design: TOOL-7 — containerized handoff-run sandboxes

> **Status: design approved in conversation 2026-09-29; awaiting written-spec review.** Not yet
> fact-checked — "Claims to verify" below lists every tree/host fact the design leans on, for the
> `/handoff-run TOOL-7` plan fact-check (step 3.5). No task breakdown yet: that is the `plan`
> step's output, from this document.

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
  (serena's Rust LSP).
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
`HR_ITEM=<ITEM>`, `HR_SANDBOX=1`, `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL`/`GIT_COMMITTER_NAME`/
`GIT_COMMITTER_EMAIL` from host `git config user.*`, `PATH` including the mise install dirs for
`claude`, `uv`, `ripgrep`, and `TERM`/`COLORTERM` passthrough. `SQLX_OFFLINE` stays as
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

`next-item-id.sh` / `.ps1` stay **untouched** (twin parity unaffected). `handoff-run`
(`references/lifecycle.md` P0) and `handoff-add` step 3 mint through `hr-mint` **whenever the lease
file exists** — on the host too, so a host `/handoff-add` cannot reuse a sandbox's leased ID. A
leased-then-abandoned ID is a harmless gap (owned-ID method takes the max). `hr gc` prunes a lease
once its ID is on host `main` (open line or archive index) or its run was purged uncollected.

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
- `.claude/skills/handoff-run/SKILL.md` sandbox-mode section; `references/lifecycle.md` P0 and
  `.claude/skills/handoff-add/SKILL.md` step 3 mint via `hr-mint` when the lease file exists
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
