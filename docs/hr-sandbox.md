# Running `/handoff-run` in sandboxes (`scripts/hr`)

`scripts/hr` runs each `/handoff-run` lifecycle in its own set of containers, so three or four of
them can work at the same time without touching each other or your checkout. Each run gets:

- a fresh clone of this repo on branch `hr/<ITEM>`, taken from your local `main` (unpushed commits
  included);
- its own Postgres and Qdrant, on the same `localhost` ports as on the host;
- the full toolchain (Rust 1.98.1, `sqlx-cli`, `cargo-insta`, `psql`) plus your own Claude Code,
  Gortex, serena, headroom and graphify, with the same skills, plugins, hooks and MCP servers;
- no git credentials. Work leaves a sandbox only when you collect it on the host.

Linux only. The design, and why it is built this way, is in
[`.claude/plans/tool-7-hr-sandbox.plan.md`](../.claude/plans/tool-7-hr-sandbox.plan.md) (TOOL-7).

- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Building the image](#building-the-image)
- [Starting a run](#starting-a-run)
- [Working in a run](#working-in-a-run)
- [New item IDs across runs](#new-item-ids-across-runs)
- [Bringing work home](#bringing-work-home)
- [Listing, stopping and cleaning up](#listing-stopping-and-cleaning-up)
- [Where the data lives](#where-the-data-lives)
- [What is inside a sandbox, and what is not](#what-is-inside-a-sandbox-and-what-is-not)
- [Troubleshooting](#troubleshooting)
- [Tests](#tests)

## Requirements

- **Docker 26 or newer with Compose v2** (`docker compose version`). Your user must be able to run
  `docker` without `sudo`.
- **`gum`** for the menus (`apt install gum`). Every verb also takes plain arguments, so scripts and
  non-interactive shells work without it.
- **The host tools under `~/.local`**, installed through mise as usual: `claude`, `gortex`,
  `graphify`, `headroom`, `uv`/`uvx`. They are mounted read-only into every run, so a sandbox always
  uses exactly the versions you have on the host. Gortex also needs `~/.gortex/config.yaml`,
  `~/.gortex/instructions/` and `~/.gortex/models/`.
- **`bats`** (through mise) if you want to run the tests.
- **Disk.** A run's clone and `target/` take 10–40 GB. Runs live on `/media` by default; see
  [Where the data lives](#where-the-data-lives).

## Quick start

```
scripts/hr build              # once, and after a uid/gid or home change
scripts/hr up MOD-65          # clone, start the containers
scripts/hr attach MOD-65      # claude inside the run; then /handoff-run MOD-65
# ... the run ends with: branch hr/MOD-65 ready — on host: scripts/hr collect MOD-65
scripts/hr collect MOD-65 --merge
scripts/hr down MOD-65 --purge
```

Run `scripts/hr` on its own for a menu of the verbs. Exit codes: `0` ok, `1` refused (duplicate
run, item not open, dirty tree, rewritten history, not collected, conflict, no confirmation), `2`
usage or a missing dependency, `3` Docker or git failed. Without a terminal, a missing choice is a
usage error and every confirmation needs `--yes`.

`scripts/hr` is a host tool. Inside a sandbox it refuses to run.

## Building the image

```
scripts/hr build
```

Builds `htui-hr-dev` for your user id, group id and home directory, and creates the shared
`htui-hr-cargo` volume. The image records those three values as labels; `scripts/hr up` refuses to
start a run when they no longer match you (`rebuild`), and offers to build when the image is
missing. Rebuild whenever `docker/hr/Dockerfile` changes. It pins Rust, `sqlx-cli` and
`cargo-insta` to match `rust-toolchain.toml` and `Cargo.lock`, so bump it together with them.

## Starting a run

```
scripts/hr up [ITEM] [--ssd] [--from REF] [--cpus N] [--yes]
```

- Without `ITEM` you pick from the open items in `HANDOFF.md` on `REF`; items that already have a
  run are left out. An item that is not an open checklist line is refused, and so is a second run
  for the same item.
- `--from REF` branches from `REF` instead of `main`. The clone takes the **committed** `REF` only:
  if `.claude/`, `HANDOFF.md` or `DECISIONS.md` have uncommitted changes on the host, `up` lists them
  and asks before going on. Commit first if the run needs them.
- `--ssd` puts the run on the SSD (`~/htui-hr`) instead of `/media`.
- `--cpus N` limits the run's `dev` container (default 4).

What `up` copies in besides the clone: your untracked `.claude/settings.local.json` (it carries the
Gortex and headroom hooks), a private copy of `~/.claude.json`, and your global git ignore rules
(added to the clone's `.git/info/exclude`). The clone's `origin` is the host repo, mounted
read-only, and pushing is disabled.

When `up` fails half-way, clean up with `scripts/hr down ITEM --purge --force` and try again.

## Working in a run

```
scripts/hr attach [ITEM] [--shell]
```

Starts the run if it is stopped, then opens `claude --dangerously-skip-permissions` in the clone,
or a login shell with `--shell`. Detaching is just quitting `claude`; the containers keep running.
Inside, start the lifecycle as usual:

```
/handoff-run MOD-65          # or: /handoff-run next
```

In a sandbox `/handoff-run` always works on the run's item: `next` resolves to it, and naming a
different item makes it ask you first. Every gate still waits for you — route confirm, CONFIRM, PRD
questions and the reviewer. Instead of pushing, the run ends with
`branch hr/<ITEM> ready — on host: scripts/hr collect <ITEM>`.

The clone sits at the same path as your checkout (`~/projects/htui`), so absolute paths in docs,
Claude's per-project memory and Gortex all line up.

### Databases

Postgres listens on `localhost:5439` (user `postgres`, no password) and Qdrant on
`localhost:6333`/`6334`, exactly as the host's `compose.yaml` services do, but these are the run's
own servers. The test variables are already set:

```
HTUI_TEST_DATABASE_URL=postgres://postgres@localhost:5439/postgres
HTUI_TEST_QDRANT_URL=http://localhost:6334
```

So the full suite runs as is:

```
cargo test --workspace --all-features -- --test-threads=1
```

There is no `docker` in a sandbox. Where a note says `docker exec htui-postgres psql …`, run `psql`
directly:

```
psql -h localhost -p 5439 -U postgres
```

### Changing SQL queries in a run

`cargo sqlx prepare` needs a database with the migrations applied. Make one, then work from inside
the crate:

```
psql -h localhost -p 5439 -U postgres -c "CREATE DATABASE htui_sqlx;"
cd crates/htui-store
export DATABASE_URL=postgres://postgres@localhost:5439/htui_sqlx
cargo sqlx migrate run --source migrations
cargo sqlx prepare -- --all-targets --all-features    # regenerate .sqlx
cargo sqlx prepare --check                            # or only check it
```

Keep `--all-targets --all-features` when regenerating: without them the queries used only by tests
and the demo are deleted from `.sqlx`. `warning: potentially unused queries found in .sqlx` from
`--check` is expected.

### What a run cannot do

No `docker`, no `git push`, no `gh`, no access to the host's Postgres or Qdrant. The host repo is
visible read-only at `/host/htui`; nothing in a run can change it.

## New item IDs across runs

Two runs that each open a new item would both mint the same next ID from their own trees. So while
sandbox runs exist, every mint — in a run, and in `/handoff-add` or `/handoff-run` on the host — goes
through `scripts/hr-mint`, which the skills do for you:

```
scripts/hr-mint --prefix MOD --title "Ctrl-c leaves the terminal raw"
```

It runs the usual `next-item-id.sh` on the tree, takes the higher of that and the highest ID of the
prefix already leased + 1, records the lease, and prints the ID. On stderr it says how it got there
and lists what other runs leased for the same prefix recently:

```
hr-mint: leased MOD-71 for hr/MOD-65 (tree next MOD-66, lease floor MOD-70)
hr-mint: other MOD leases in the last 7 days - ask the maintainer before filing if one is the same problem:
  MOD-70  hr/MOD-64  2026-09-29T10:02:11Z  Terminal stays raw after ctrl-c
```

When a listed title is the same problem, the run asks you before filing; you can reuse the other
run's ID instead. A leased ID that ends up unused is a harmless gap — IDs are never reused, and gaps
are allowed.

- `scripts/hr-mint --leasing` exits 0 when leasing is on, which is the skills' switch.
  `scripts/hr up` turns it on (`scripts/hr-mint --init`); it stays on for the host from then on.
- Without the lease file, or without the state directory, `hr-mint` is the plain tree mint and says
  so: `hr-mint: leasing off (no state dir at …) - MOD-6 is the tree mint only`, or
  `(no lease file at …; run scripts/hr-mint --init)`.
- Exit codes: `0` ok, `1` blocked (the tree mint found a problem, or the lease file has an
  unparseable line), `2` usage, `3` lock timeout or the state directory is not writable. Non-zero
  means no ID: fix the cause, do not fall back to `next-item-id.sh`.
- The lease file (`id-leases.tsv` in the state directory) belongs to `hr-mint`; never edit it by
  hand. `scripts/hr gc` prunes leases whose ID has reached `main`, but always keeps the highest lease
  of each prefix so an older clone cannot mint below it.

## Bringing work home

```
scripts/hr collect ITEM [--merge] [--force] [--yes]
```

Without `--merge`, `collect` only fetches the run's `hr/ITEM` branch into the host repo and prints
`git log --oneline main..hr/ITEM`. Your working tree and checked-out branch are not touched, so it
is safe at any time and can be repeated as the run makes more commits. It refuses when:

- the branch was rewritten in the run (amend, rebase) — `--force` overwrites the host copy;
- `hr/ITEM` is checked out in one of your host worktrees.

`--merge` then merges `hr/ITEM` into your **current** branch (`--no-ff`) and runs
`validate-workflow-docs.sh`. It refuses on uncommitted changes to tracked files, a merge already in
progress or a detached `HEAD`. It never pushes.

### Merge conflicts

Runs that each add or close items conflict in `HANDOFF.md`, `DECISIONS.md` and the summary table.
The resolution is always the same:

1. keep every new checklist line from both sides (their IDs are distinct);
2. take the other side's edits to existing lines;
3. recount the summary table from the merged checklist;
4. run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` — non-zero means not
   done.

When those files (and `docs/decisions/**`) are the only conflicts, `collect --merge` offers to open
a host `claude` session primed with this recipe; you stay in it and approve as usual. Conflicts in
code are left to you (`git merge --abort` backs out). Either way the command exits `1` with the
merge in progress.

### After merging

Run the Postgres gate on the merged tree on the host, with no sandbox building at the same time —
parallel builds and test runs have pushed the host Postgres into recovery
(`the database system is in recovery mode`, `57P03`) before. Push when you are satisfied, as always.

## Listing, stopping and cleaning up

```
scripts/hr ls
scripts/hr down ITEM [--purge] [--force] [--yes]
scripts/hr gc [--yes]
```

- **`ls`** shows each run's state (`running`, `stopped`, `down`, plus `+collected`), branch, commits
  ahead of your `main` (a `*` means counted from the run's starting point, because your `main` has
  moved past anything the clone knows), disk use and whether it is on the HDD or the SSD.
- **`down`** stops and removes the run's containers. The clone, the databases and the Gortex index
  stay; `attach` brings it back.
- **`down --purge`** also deletes the run's databases, Gortex store and run directory. It refuses
  unless the branch was collected, the clone has no uncommitted tracked changes and no clone branch
  is missing from the host. `--force` skips those checks, and then also drops the run's ID leases.
  It always asks (or needs `--yes`). The shared `htui-hr-cargo` volume is never removed.
- **`gc`** offers to purge every stopped, collected, clean run, then prunes ID leases that have
  reached `main`.

## Where the data lives

| What | Where | Change with |
|---|---|---|
| Run directory: clone, `target/`, `claude.json` | `/media/projects/htui-hr/<ITEM>/` | `HR_ROOT`; `--ssd` → `HR_SSD_ROOT` (`~/htui-hr`) |
| Shared state: run registry, ID leases | `/media/projects/htui-hr/.state/` (`/hr-state` inside) | `HR_STATE` — every run and the host must agree |
| Postgres, Qdrant, Gortex store | per-run Docker volumes, under `/var/lib/docker` | — |
| Cargo registry and git cache | `htui-hr-cargo` Docker volume, shared by all runs | — |
| Host repo the runs clone and collect into | the main worktree of this repo | `HR_HOST_REPO` |

These are environment variables read by `scripts/hr`. Set them the same way for every call, not
only `up`: the registry lives in `HR_STATE`, and `--purge` only deletes run directories under
`HR_ROOT` or `HR_SSD_ROOT`.

The run directories are the big part. Watch both disks while runs are building:

```
df -h /media /
docker system df
```

`/` holds the Docker volumes, and a full `/` takes the runs' Postgres down with it. Purge collected
runs rather than letting them pile up.

## What is inside a sandbox, and what is not

| Host | In the run | |
|---|---|---|
| the run's clone | `~/projects/htui` | read-write |
| this repo | `/host/htui` | read-only, the clone's `origin` |
| `~/.local` (claude, gortex, graphify, headroom, uv) | same path | read-only |
| `~/.claude` (login, settings, skills, plugins, memory, sessions) | same path | read-write, shared by all runs |
| `~/.claude.json` | a private copy | changes inside do not flow back |
| `~/.gortex` config, instructions, models | same path | read-only |
| `~/.headroom`, `~/.serena`, `~/.cache/uv` | same path | read-write, shared |
| this repo's `.remember/` | `.remember/` in the clone | read-write, shared |
| state directory | `/hr-state` | read-write, shared |

**Not there:** `~/.gitconfig`, `~/.git-credentials`, `~/.ssh`, `gh`, Docker, Node, and the MCP
servers already disabled for this project. Commits carry your name and email from
`git config user.*`, passed in as environment variables.

Inside a run these are set: `HR_SANDBOX=1`, `HR_ITEM=<ITEM>`, `HR_STATE=/hr-state`,
`USERNAME=htui-ci`, `HTUI_TEST_DATABASE_URL`, `HTUI_TEST_QDRANT_URL`, `UV_TOOL_DIR=/tmp/uv-tools`,
the git identity, and a `PATH` with the mise tool directories ahead of `~/.local/bin`.
`SQLX_OFFLINE` comes from `.cargo/config.toml` as usual. There is no D-Bus and no keyring; the test
suites use their mock keyring.

Every run starts its own Gortex daemon with its own index (the first index takes about 20 seconds).
Gortex memories written in a run stay in that run and are deleted when it is purged — anything
worth keeping belongs in the repo or in Claude's memory, which is shared.

## Troubleshooting

**`up` says `run scripts/hr build` or `rebuild`.** The image is missing, or was built for a
different uid, gid or home directory. Run `scripts/hr build`.

**`up` says the run exists, or `stale run dir`.** Each item has at most one run. `scripts/hr ls`
shows it; `attach` to it, or `down --purge` it first. A run directory without a registry entry is
left over from a crash: check it for work you want, then delete it by hand.

**`gum not found`.** Install it (`apt install gum`) or pass the arguments the menu would ask for.

**Gortex is missing or says `daemon already running`.** The `dev` container clears Gortex's stale
daemon files every time it starts, so `scripts/hr down ITEM` followed by `scripts/hr attach ITEM`
fixes a stuck daemon. Give a fresh run 20 seconds to index before judging. If Gortex still will not
start, the session carries on without it, as its instructions say for an integration failure.

**serena fails with `Read-only file system`.** `uvx` writes into its tool directory, and `~/.local`
is read-only in a run, so the run sets `UV_TOOL_DIR=/tmp/uv-tools`. If you start a process some
other way than `attach`, keep that variable.

**The wrong `uv` runs.** `~/.local/bin` holds an old `uv`/`uvx` (0.8.22). The run's `PATH` puts the
mise directories first, and the image's `~/.profile` deliberately does not prepend `~/.local/bin`.
Don't add it back in a shell startup file.

**MCP servers changed on the host don't show up in a run.** `~/.claude.json` is copied at `up` and
seeded into the container once. Toggles made inside a run stay there, and the copy is re-seeded
from the run directory whenever the container is re-created (`down`, then `attach`). To pick up a
new host config: `install -m 600 ~/.claude.json <run directory>/claude.json`, then `down` and
`attach`.

**A run is logged out of Claude.** All runs share `~/.claude/.credentials.json` with the host, the
same way parallel host sessions do. Log in again with `/login` in any one of them; the shared file
then serves them all. If it
keeps happening, the fallback is a long-lived token from `claude setup-token`, used as
`CLAUDE_CODE_OAUTH_TOKEN`; `scripts/hr` does not pass that variable in yet.

**A stray entry in `.remember/now.md`.** The remember plugin's save lock holds a process id, and
process ids inside a container mean nothing on the host and vice versa. When a host save and a run
save collide, an entry can land interleaved. This is a known, accepted limitation.

**Postgres in a run restarts or reports recovery mode.** Check `df -h /` first: the run databases
live under `/var/lib/docker`.

## Tests

```
bats --filter-tags '!docker' tests/hr   # seconds; fixture repos only, no Docker
bats tests/hr                           # also builds the image and starts real runs
```

The tests never touch your real repo or your real state directory: every case clones from a fixture
repo and keeps its state under a temporary directory.
