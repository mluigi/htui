# Running `/handoff-run` in sandboxes (`scripts/hr`)

`scripts/hr` runs each `/handoff-run` lifecycle in its own set of containers, so three or four of
them can work at the same time without getting in each other's way or in your checkout's. Each run
gets:

- a fresh clone of this repo on branch `hr/<ITEM>`, taken from your local `main` (unpushed commits
  included);
- its own Postgres and Qdrant, on the same `localhost` ports as on the host;
- the full toolchain (Rust 1.98.1, `sqlx-cli`, `cargo-insta`, `psql`) plus your own Claude Code,
  Gortex, serena, headroom and graphify, with the same skills, plugins, hooks and MCP servers;
- no git credentials. Work leaves a sandbox only when you collect it on the host.

The sandboxes keep runs apart from **accidents**. They are not a security boundary against a run
that turns hostile: read [Threat model](#threat-model) before you trust one with anything.

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
- [Threat model](#threat-model)
- [Troubleshooting](#troubleshooting)
- [Tests](#tests)

## Requirements

- **Docker 26 or newer with Compose v2** (`docker compose version`). Your user must be able to run
  `docker` without `sudo`.
- **`gum`** for the menus (`apt install gum`). Every verb also takes plain arguments, so scripts and
  non-interactive shells work without it.
- **The host tools under `~/.local`**, installed through mise as usual: `claude`, `gortex`,
  `graphify`, `headroom`, `uv`/`uvx`. Four directories are mounted read-only into every run —
  `~/.local/bin`, `~/.local/share/mise/installs`, `~/.local/share/uv/python` and
  `~/.local/share/uv/tools` — so a sandbox always uses exactly the versions you have on the host.
  The rest of `~/.local` is never mounted: it holds credentials. `up` looks for `claude` and `uv`
  under `~/.local/share/mise/installs/<tool>/latest` (set `HR_MISE_PATH` to override). It also needs
  those four directories, `~/.claude.json`, `~/.gortex/config.yaml`, `~/.gortex/instructions/` and
  `~/.gortex/models/`.
- **A git identity** (`git config user.name` and `user.email`) in this repo: the run commits as
  you.
- **`bats`** (through mise) if you want to run the tests.
- **Disk.** A run's clone and `target/` take 10–40 GB. Runs live on `/media` by default; see
  [Where the data lives](#where-the-data-lives).

## Quick start

```
scripts/hr build              # once, and after a uid/gid or home change or a Dockerfile change
scripts/hr up MOD-65          # clone, start the containers
scripts/hr attach MOD-65      # claude inside the run; then /handoff-run MOD-65
# ... the run ends with: branch hr/MOD-65 ready — on host: scripts/hr collect MOD-65
scripts/hr collect MOD-65
git diff main...hr/MOD-65 -- .claude scripts docker    # what would run on the host
scripts/hr collect MOD-65 --merge                      # shows that diffstat and asks first
scripts/hr down MOD-65 --purge
```

Run `scripts/hr` on its own for a menu of the verbs. Exit codes: `0` ok, `1` refused (duplicate
run, item not open, dirty tree, rewritten history, not collected, conflict, vanished lease file, no
confirmation), `2` usage or a missing dependency (a malformed `ITEM` included, on every verb), `3`
Docker, git or file I/O failed, a run's registry entry is invalid, or a mount point in the shared
cargo volume was tampered with. `attach`, `collect` and
`down` take `ITEM` optionally: in a terminal they let you pick from the registered runs. Without a
terminal, a missing choice is a usage error and a confirmation (`down --purge`, `gc`) is refused
unless you pass `--yes`.

`scripts/hr` is a host tool. Inside a sandbox it refuses to run.

## Building the image

```
scripts/hr build
```

Builds `htui-hr-dev` for your user id, group id and home directory, and creates the shared
`htui-hr-cargo` volume. The image records those three values as labels; `scripts/hr up` refuses to
start a run when they no longer match you (`rebuild`). When the image is missing, `up` offers to
build it in a terminal and otherwise exits `2` naming `scripts/hr build`.

Rebuild whenever `docker/hr/Dockerfile` changes — for instance after the change that narrowed the
`~/.local` mounts to four directories: the image creates their mount points owned by you, and Docker
would otherwise create them owned by root. The Dockerfile pins Rust, `sqlx-cli` and `cargo-insta`
to match `rust-toolchain.toml` and `Cargo.lock`, so bump it together with them. `down --purge` and
`gc` need the image too: they check a clone in a throwaway container of it.

## Starting a run

```
scripts/hr up [ITEM] [--ssd] [--from REF] [--cpus N] [--yes]
```

- Without `ITEM` you pick from the open items in `HANDOFF.md` on `REF`; items that already have a
  run are left out. An item that is not an open checklist line is refused, and so is a second run
  for the same item.
- `--from REF` branches from `REF` instead of `main`. The clone takes the **committed** `REF` only:
  if `.claude/`, `HANDOFF.md` or `DECISIONS.md` have uncommitted or untracked changes on the host
  (your `.claude/settings.local.json` excepted), `up` lists them. In a terminal it then asks whether
  to go on with the committed `REF`; `--yes` skips the question, and without a terminal it warns and
  continues. Commit first if the run needs them.
- `--ssd` puts the run on the SSD (`~/htui-hr`) instead of `/media`.
- `--cpus N` limits the run's `dev` container (default 4).

What `up` copies in besides the clone: your untracked `.claude/settings.local.json` (it carries the
Gortex and headroom hooks), a private copy of `~/.claude.json`, and your global git ignore rules
(added to the clone's `.git/info/exclude`). The clone's `origin` is the host repo, mounted
read-only, and pushing is disabled.

`up` also adds the directories tools write untracked output into — `.serena/` (serena memories),
`.claude/skills/generated/`, `.kiro/` and `graphify-out/` — to the clone's `.git/info/exclude`, as
they are untracked on the host too. Files there never block `down --purge` or `gc`, and they are
deleted with the run: commit anything worth keeping.

Before the containers start, `up` (and `attach`, when it starts them) checks the shared
`htui-hr-cargo` volume: where compose mounts over `config`, `config.toml`, `credentials`,
`credentials.toml`, `bin`, `registry` or `registry/src`, the volume must hold nothing, or a file
(the first four) or a directory (the rest) — never a symlink. Anything else means a run replaced
it; `up` exits `3` naming it and the `docker run … rm` that removes it. It also creates, owned by
you, every mount point it needs inside `~/.claude` that is missing (see
[What is inside a sandbox](#what-is-inside-a-sandbox-and-what-is-not)); it never changes one that
exists.

`up` also turns ID leasing on (`scripts/hr-mint --init`, see
[New item IDs across runs](#new-item-ids-across-runs)). It refuses (`1`) when the lease file of
this state directory has vanished since, and with a non-default `HR_STATE` it warns, once per
`up`:
`HR_STATE is non-default: export HR_STATE=… in your shell profile, or host skill mints will not see
the leases`.

If `up` fails before the containers start (clone, copies), it removes the half-made run directory
itself; Ctrl-C or a `TERM` in that window does the same. Just fix the cause and run it again. Once
the run is registered — just before `docker compose up` — it stays registered: when the containers
fail to start, `up` says so, and you clean up with `scripts/hr down ITEM --purge --force` before
retrying.

## Working in a run

```
scripts/hr attach [ITEM] [--shell]
```

Starts the run's containers if `dev` is not running (after `down`, or a reboot), re-creating the
shared `htui-hr-cargo` volume first if it was pruned and checking its mount points as `up` does,
then opens
`claude --dangerously-skip-permissions` in the clone, or a login shell with `--shell`. Detaching is
just quitting `claude`; the containers keep running, and you can attach again, or from several
terminals at once.
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

No `docker`, no `git push`, no `gh`. The host's `htui-postgres` and `htui-qdrant` are published on
`127.0.0.1` only (`compose.yaml`), so a run cannot reach them. The host repo is visible read-only at
`/host/htui`: a run cannot write its files directly. It can still reach the host through the
directories every run shares read-write — see [Threat model](#threat-model).

## New item IDs across runs

Two runs that each open a new item would both mint the same next ID from their own trees. So when
`scripts/hr-mint --leasing` exits 0 — always in a sandbox, and on the host once `scripts/hr up` has
been used — every mint, in a run and in `/handoff-add` or `/handoff-run` on the host, goes through
`scripts/hr-mint`, which the skills do for you:

```
scripts/hr-mint --prefix MOD --title "Ctrl-c leaves the terminal raw"
```

It takes the highest of three sources, each of which can only raise the ID: the usual
`next-item-id.sh` on the tree; in a sandbox, `next-item-id.sh` on your repo at `/host/htui`
(read-only; `HR_HOST_TREE` overrides the path), which catches IDs that reached your `main` without a
lease; and the highest ID of the prefix already leased + 1. It records the lease and prints the ID.
On stderr it says how it got there and lists what other runs leased for the same prefix recently:

```
hr-mint: leased MOD-71 for hr/MOD-65 (tree next MOD-66, host next MOD-66, lease floor MOD-70)
hr-mint: other MOD leases in the last 7 days - ask the maintainer before filing if one is the same problem:
  MOD-70  hr/MOD-64  2026-09-29T10:02:11Z  Terminal stays raw after ctrl-c
```

On the host the line has no `host next` part (`tree next MOD-66, lease floor MOD-70`); the lease
floor reads `none` when the prefix has no lease yet. In a sandbox, `host next unavailable` means the
host tree did not count, after one of these notes on stderr — neither blocks the mint:
`hr-mint: host tree /host/htui not readable - skipped (no host raise)`, or
`hr-mint: warning: host tree mint failed (next-item-id.sh exit N on /host/htui) - no host raise; …`.

When a listed title is the same problem, the run asks you before filing; you can reuse the other
run's ID instead. A leased ID that ends up unused is a harmless gap — IDs are never reused, and gaps
are allowed.

- `scripts/hr-mint --leasing` exits 0 when leasing is on, which is the skills' switch.
  `scripts/hr up` turns it on — `scripts/hr-mint --init`, host only, creates the lease file and
  stamps its lock file, `id-leases.lock`, to record that the lease file existed — and nothing turns
  it off again: `down` and `gc` prune leases, never the file. `up` also records the state
  directory in the host-only `HR_RUNS/.leases-initialized`, which `hr-mint` reads on the host
  (never in a sandbox), so leasing stays on even if a sandbox deletes the lock as well. The
  owned-ID rule allows this raise (`.claude/rules/workflow-docs.md`, "Lease raise").
- **Leasing fails closed.** In a sandbox a missing state directory or lease file is exit `1`, never
  a tree-only mint:
  `hr-mint: in a sandbox (HR_SANDBOX=1) leasing is required, but there is no lease file at /hr-state/id-leases.tsv - nothing done; the maintainer restores it on the host`
  (or `… no state dir at /hr-state - nothing done; check the /hr-state mount on the host`). On the
  host, a lease file that is missing while the lock is stamped, or while `HR_RUNS/.leases-initialized`
  names this state directory, means the file vanished: mint, prune and `--init` exit `1` with
  `hr-mint: lease file vanished: … is gone but … - nothing done. Restore it, or run
  scripts/hr-mint --init --force after checking that no live run holds leases that are not on main
  yet (their floor is lost)`, and `--leasing` still exits 0, so the skills reach that block instead
  of falling back. `scripts/hr up`, `gc` and a forced purge of an uncollected run (the one purge
  that prunes leases) refuse the same way (`hr: lease file … vanished (a sandbox or a stray rm?) —
  restore it, or run scripts/hr-mint --init --force after checking`); any other `down --purge` goes
  ahead. Restore the file, or run `scripts/hr-mint --init --force` once you are sure no live run
  holds leases that are not on `main` yet.
- Only a host that never ran `--init` has leasing off. There `hr-mint` is the plain tree mint and
  says so: `hr-mint: leasing off (no state dir at …) - MOD-6 is the tree mint only`, or
  `(no lease file at …; run scripts/hr-mint --init)`.
- Exit codes: `0` ok, `1` blocked (the tree mint found a problem, the lease file has an unparseable
  line or is missing while leasing is on, `--init` in a sandbox), `2` usage (`--force` without
  `--init` included), `3` lock timeout (30 s, `HR_MINT_LOCK_TIMEOUT`) or the state directory is not
  writable. Non-zero means no ID: fix the cause, do not fall back to `next-item-id.sh`.
- The lease file (`id-leases.tsv` in the state directory) belongs to `hr-mint`; never edit it by
  hand. Titles and owners are stored, and rows are printed, without control characters.
  `scripts/hr gc` prunes leases whose ID has reached `main`, and a forced purge of an uncollected
  run drops that run's leases; both always keep the highest lease of each prefix, so an older clone
  cannot mint below it.

## Bringing work home

```
scripts/hr collect ITEM [--merge] [--force] [--yes]
```

Without `--merge`, `collect` only fetches the run's `hr/ITEM` branch into the host repo and prints
`git log --oneline main..hr/ITEM` (subjects without control characters). The fetch runs no hooks
and checks every object it brings in (`transfer.fsckObjects`). It lands in a temporary ref
(`refs/hr-collect/ITEM`); `hr/ITEM` and the registry move only after the checks below pass on what
actually arrived, and the temporary ref is deleted on every way out but a `SIGKILL` (see
[Troubleshooting](#troubleshooting)). The registry records the
host's `hr/ITEM` as it is after the fetch. Your working tree and checked-out branch are not touched,
so it is safe at any time and can be repeated as the run makes more commits. It refuses when:

- the branch was rewritten in the run (amend, rebase), also if that happens during the fetch —
  `--force` overwrites the host copy;
- `hr/ITEM` is checked out in one of your host worktrees.

`--merge` merges `hr/ITEM` into your **current** branch (`--no-ff`) and runs
`validate-workflow-docs.sh` — the copy from your `HEAD` before the merge (with the
`workflow-patterns.sh` it sources), never the one the run brings. A red validator after a clean
merge exits `1` (`merged … but the validator is red — fix before pushing`). It never pushes.

Before anything is fetched it refuses on uncommitted changes to tracked files, a merge already in
progress, a detached `HEAD`, or a `HEAD` without the validator (exit `2`); that refusal leaves the
host exactly as it was.

After the fetch and before the merge, it prints what the run changes in files that configure or run
tools on the host — after the merge, your next `claude` session, `scripts/hr` call or image build
uses them:

```
git diff --stat HEAD...hr/ITEM -- .claude scripts docker CLAUDE.md AGENTS.md .mcp.json .cargo rust-toolchain.toml
```

When that is not empty it always asks: in a terminal with a `gum confirm`, `--yes` or not; without
a terminal `--yes` is the confirmation, and without it `collect` exits `1` (`needs confirmation`).
Declining leaves the host as it was: no `hr/ITEM`, no registry change, nothing merged. To look at
the full diff first, run a plain `collect` and then `git diff HEAD...hr/ITEM -- …`.

### Merge conflicts

Runs that each add or close items conflict in `HANDOFF.md`, `DECISIONS.md` and the summary table.
The resolution is always the same:

1. keep every new checklist line from both sides (their IDs are distinct);
2. take the other side's edits to existing lines;
3. recount the summary table from the merged checklist;
4. run `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` — non-zero means not
   done.

When those files (and `docs/decisions/**`) are the only conflicts, `collect --merge` prints the
recipe and, in a terminal, offers to open a host `claude` session primed with it; you stay in it and
approve as usual. That session starts in the half-merged tree, so it loads the run's `.claude/`. If
the run changed any of those tool-config paths, `collect` prints that `git diff --stat` again and
asks again, `--yes` or not; otherwise `--yes` opens the session without asking. Conflicts in code are left to you (`git merge --abort` backs out). Either way the command
exits `1` with the merge in progress, and says so — also when you leave the `claude` session with
Ctrl-C.

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
  moved past anything the clone knows; `?` means the clone is missing), disk use and whether it is
  on the HDD or the SSD (`hdd`/`ssd`).
- **`down`** stops and removes the run's containers. The clone, the databases and the Gortex index
  stay; `attach` brings it back.
- **`down --purge`** also deletes the run's databases, Gortex store and run directory. It first
  stops the run, so nothing changes the clone while it is checked, then refuses (and lists why)
  unless:
  - `hr/ITEM` is collected — its tip is in the host's `hr/ITEM` or `main`;
  - no worktree of the clone has uncommitted changes or untracked files that are not ignored. That
    covers the clone itself and every linked worktree made in the run (implementer worktrees); a
    linked worktree that is missing or outside the clone blocks too (`git worktree prune` in the
    run). Ignored files never block: `target/`, and the tool-owned directories `up` excludes
    (`.serena/`, `.claude/skills/generated/`, `.kiro/`, `graphify-out/`). Other untracked output a
    tool leaves behind blocks: commit it, delete it in the run, or purge with `--force`;
  - the clone has no stash (`refs/stash`) — apply and commit it, or drop it;
  - every other branch in the clone, and every detached worktree `HEAD`, has its commit on the
    host. Bring branches home with `git fetch <run directory>/src <branch>:<branch>` on the host,
    or delete them in the run, or use `--force`.

  A refused purge leaves the run stopped; `attach` restarts it. The worktree checks run in a
  throwaway container of `htui-hr-dev` (no network, no capabilities, clone read-only), never on
  the host, so the image must exist; a check that cannot run (missing image, 300 s timeout) blocks
  like a finding.

  `--force` skips the stop and these checks, so everything in the clone that is not on the host —
  uncommitted work, untracked files, other branches, stashes — is deleted with it. When the run was
  not collected, a forced purge also drops its ID leases, except the highest lease of each prefix,
  which always stays as the floor; that purge alone refuses while the lease file is vanished.
  Purging always asks (or needs `--yes`). It refuses a run directory that is a symlink or not
  directly under `HR_ROOT` or `HR_SSD_ROOT`. The shared `htui-hr-cargo` volume is never removed.
- **`gc`** finds every run whose `dev` container is not running and that passes the same checks,
  offers them for purging (`--yes` purges them all; without a terminal and without `--yes` it only
  lists them and exits `1`), then prunes ID leases that have reached `main`.

## Where the data lives

| What | Where | Change with |
|---|---|---|
| Run directory: clone, `target/`, `claude.json`, `cargo-empty`, `claude-mask/` | `/media/projects/htui-hr/<ITEM>/` | `HR_ROOT`; `--ssd` → `HR_SSD_ROOT` (`~/htui-hr`) |
| ID leases: `id-leases.tsv`, `id-leases.lock` | `/media/projects/htui-hr/.state/` (`/hr-state` inside, shared read-write) | `HR_STATE` — every run and the host must agree |
| Run registry, its `.lock`, `.leases-initialized`, `.migrated` | `/media/projects/htui-hr/.runs/` (host only, never mounted) | `HR_RUNS` — never inside `HR_STATE` (exit `2`) |
| Postgres, Qdrant, Gortex store, extracted crate sources (`~/.cargo/registry/src`) | per-run Docker volumes, under `/var/lib/docker` | — |
| Cargo download cache, index and git cache | `htui-hr-cargo` Docker volume, shared by all runs | — |
| Host repo the runs clone and collect into | the main worktree of this repo | `HR_HOST_REPO` |

These are environment variables read by `scripts/hr`. Set them the same way for every call, not
only `up`:

- The registry records each run's directory as exactly `HR_ROOT/<ITEM>` or `HR_SSD_ROOT/<ITEM>`.
  Change `HR_ROOT` or `HR_SSD_ROOT` after `up` and that run's entry is invalid — every verb that
  reads it exits `3` (`ls` and `gc` read them all) until you set them back.
- Export a non-default `HR_STATE` in your shell profile, or host skill mints won't see the leases:
  `hr-mint` on the host reads the same variable.
- A registry in the old place, `$HR_STATE/runs`, is moved to `HR_RUNS` once per state directory
  (`moved N registry entries …`) and then validated like any other; `HR_RUNS/.migrated` records
  that. After that nothing is read from `$HR_STATE/runs` again — every sandbox can write there — and
  anything left there only draws a warning (`ignoring …/runs`); delete it by hand.

`HR_IMAGE` renames the image (default `htui-hr-dev`).

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
| `~/.local/bin`, `~/.local/share/mise/installs`, `~/.local/share/uv/python`, `~/.local/share/uv/tools` (claude, gortex, graphify, headroom, uv) | same paths | read-only; nothing else of `~/.local` |
| `~/.claude` (login, settings, plugins, skills, agents, commands) | same path | read-write, shared by all runs, except what the next rows mask |
| `~/.claude/projects/-home-mluigi-projects-htui` (this project's auto-memory and sessions) | same path | read-write, shared |
| the rest of `~/.claude/projects`, and `file-history`, `paste-cache`, `shell-snapshots`, `session-env`, `session-data`, `sessions`, `daemon`, `jobs`, `backups`, `metrics`, `.remember` in `~/.claude` | an empty `tmpfs` each | per container, gone when it stops |
| `~/.claude/history.jsonl`, `bash-commands.log`, `cost-tracker.log` | an empty file each, from `claude-mask/` in the run directory | per run |
| `~/.claude.json` | a private copy | changes inside do not flow back |
| `~/.gortex` config, instructions, models | same path | read-only |
| `~/.headroom`, `~/.serena`, `~/.cache/uv` | same path | read-write, shared |
| this repo's `.remember/` | `.remember/` in the clone | read-write, shared |
| state directory (ID leases) | `/hr-state` | read-write, shared |
| `htui-hr-cargo` volume | `~/.cargo` | read-write, shared by all runs; `config`, `config.toml`, `credentials` and `credentials.toml` are one empty read-only file (`cargo-empty` in the run directory); `bin` is an empty read-only `tmpfs`; `registry/src` is a per-run volume |

The project directory kept is the one Claude names after the clone's path, which is your checkout's
path (`HR_HOST_REPO`, every character outside `A-Z a-z 0-9` turned into `-`). `/rewind` history,
pasted images and shell snapshots of a run live in its `tmpfs` and are gone when the container
stops.

**Not there:** `~/.gitconfig`, `~/.git-credentials`, `~/.ssh`, the rest of `~/.local`, the run
registry, other projects' Claude transcripts and memory, your prompt history, `gh`, Docker, Node,
and the MCP servers already disabled for this project. Commits carry
your name and email from `git config user.*`, passed in as environment variables.

Inside a run these are set: `HR_SANDBOX=1`, `HR_ITEM=<ITEM>`, `HR_STATE=/hr-state`,
`USERNAME=htui-ci`, `HTUI_TEST_DATABASE_URL`, `HTUI_TEST_QDRANT_URL`, `UV_TOOL_DIR=/tmp/uv-tools`,
`CARGO_INSTALL_ROOT=/tmp/cargo-install`, the git identity, and a `PATH` with the mise tool
directories ahead of `~/.local/bin`, no `~/.cargo/bin`, and `/tmp/cargo-install/bin` last — a
`cargo install` stays in its container and cannot shadow the toolchain. `~/.cargo/bin` is also an
empty read-only `tmpfs`, because cargo looks for `cargo-<command>` there before `PATH`: `cargo sqlx`
is always the image's. `SQLX_OFFLINE` comes from
`.cargo/config.toml` as usual. There is no D-Bus and no keyring; the test suites use their mock
keyring.

Every run starts its own Gortex daemon with its own index (the first index takes about 20 seconds).
Gortex memories written in a run stay in that run and are deleted when it is purged — anything
worth keeping belongs in the repo or in Claude's memory of this project, which is shared.

## Threat model

The sandboxes isolate runs from **accidental** interference: a run cannot build into another run's
`target/`, write another run's database, commit to your branches, or push. They are not a boundary
against a run that turns hostile — a prompt-injected agent running with
`--dangerously-skip-permissions`. Such a run can reach the host through what every run shares
read-write:

| Shared read-write | What a hostile run can do with it |
|---|---|
| `~/.claude` — by design, so runs have your login, settings, plugins, skills, agents and commands | add hooks or settings that execute in your next host Claude session; read or use the login (`.credentials.json`) |
| `~/.claude/projects/-home-mluigi-projects-htui` | read this project's host session transcripts; change its auto-memory, which your next host session loads |
| `~/.cache/uv` | plant packages that `uvx` runs on the host |
| `~/.headroom`, `~/.serena` | change the state and configuration your host headroom and serena read |
| this repo's `.remember/` | replace a file such as `now.md` with a symlink to a file in your checkout; the host remember plugin then writes into the checkout |
| the state directory (`/hr-state`) | delete the lease file or hold its lock; bounded by the fail-closed rules in [New item IDs across runs](#new-item-ids-across-runs) |
| the `htui-hr-cargo` volume | change the cached `.crate` files and index other runs extract from. `registry/src` is per run, `bin/` is an empty read-only mount, the config and credentials files are read-only and empty, and `up` refuses a mount point replaced by the wrong kind of entry |

What stays protected:

- your checkout's files: the host repo is mounted read-only;
- git credentials: none are mounted, and nothing is pushed;
- your `htui-postgres` and `htui-qdrant`: `compose.yaml` publishes them on `127.0.0.1` only. Any
  other host service listening on all interfaces is reachable from a run through the Docker bridge;
- each run's databases, volumes and clone;
- host-side git never reads a clone's worktree: the purge checks run `git status` in a throwaway
  `--network none` container, and on the host only ref and object plumbing touches a clone;
  `collect` checks every fetched object, and `--merge` validates with your pre-merge validator;
- the run registry: host-only (`HR_RUNS`) and validated on every read, so a run cannot change what
  `attach` mounts or what `down -v` removes; an old registry in `$HR_STATE/runs` is imported once,
  and never again;
- the rest of `~/.claude`: other projects' transcripts and memory, `file-history`, `paste-cache`,
  `shell-snapshots`, `session-env`, `session-data`, the host's live `sessions`, the Claude
  `daemon`'s control key, `jobs`, `backups` of `~/.claude.json`, `metrics`, `.remember`, your
  prompt `history.jsonl`, `bash-commands.log` and `cost-tracker.log` are masked in every run;
- the rest of `~/.local`: only the four tool directories are mounted, read-only, so no keyrings,
  uv or gh credentials, or editor undo histories reach a run.

What to do about it:

- At collect time, read the tool-config diffstat `collect --merge` prints and asks about
  ([Bringing work home](#bringing-work-home)); look at the full diff before saying yes.
- After a run you distrust, inspect `~/.claude/settings.json` and the hooks it and your plugins
  register (`/hooks` in a host session), this project's auto-memory
  (`~/.claude/projects/-home-mluigi-projects-htui/memory/`), and look for symlinks in `.remember/`
  (`find .remember -type l`).

Known residuals:

- The download cache and index in `htui-hr-cargo` are shared. Extracted sources are per run now, so
  editing another run's `registry/src` is out; but a run that rewrites a cached `.crate` (and its
  index entry) can still get code into a later extraction in another run. `docker volume rm
  htui-hr-cargo` after a run you distrust; `build`, `up` and `attach` re-create it.
- A run can hold the lease lock: every other mint waits 30 s (`HR_MINT_LOCK_TIMEOUT`) and exits `3`.
- A run that deletes both the lease file and its lock no longer makes host mints tree-only:
  `hr-mint` on the host reads `HR_RUNS/.leases-initialized` and reports the file vanished — as long
  as it sees the same `HR_RUNS` as `scripts/hr` (export it, like `HR_STATE`, if you change it).
- A clone's `.git/commondir` can redirect the host's ref reads; the most it achieves is making its
  own run look collected, so a purge then drops that run's uncollected work.

## Troubleshooting

**`up` says `run scripts/hr build` or `rebuild`.** The image is missing, or was built for a
different uid, gid or home directory. Run `scripts/hr build`.

**`up` says the run exists, or `stale run dir`.** Each item has at most one run. `scripts/hr ls`
shows it; `attach` to it, or `down --purge` it first. A run directory without a registry entry is
left over from a crash: check it for work you want, then delete it by hand.

**`registry … is invalid`** (exit `3`). The entry in `HR_RUNS` is not what `scripts/hr` writes: a
hand edit, or `HR_ROOT` / `HR_SSD_ROOT` changed since `up` (`run_dir … is not …`). Set the variables
back, or fix or remove the entry by hand.

**`the shared cargo volume htui-hr-cargo has the wrong kind of entry … where compose mounts`**
(exit `3`). A run replaced one of the volume's mount points (say, `config.toml` with a directory).
Look at it, then run the `docker run … rm -rf -- /v/…` the message prints; after a run you
distrust, `docker volume rm htui-hr-cargo` instead.

**`ignoring …/.state/runs`.** Something wrote the old registry place after the one-time move.
Nothing is read from it; delete it by hand.

**`collect --merge` says `needs confirmation`** (exit `1`). The run changes tool configuration
(the diffstat above the message). Read it, then pass `--yes`, or run it in a terminal.

**A `refs/hr-collect/<ITEM>` ref is left over.** `collect` fetches into that temporary ref and
deletes it on every way out it can catch; a `SIGKILL` (or a crash) in the middle leaves it behind.
It shows up in `git for-each-ref`, and `git push --mirror` would push it. The next `collect` of that
item overwrites it; to remove it, run `git update-ref -d refs/hr-collect/<ITEM>`.

**`HR_RUNS … must not be inside HR_STATE`** (exit `2`). Every sandbox mounts `HR_STATE`
read-write; point `HR_RUNS` somewhere else.

**`lease file … vanished`.** A sandbox or a stray `rm` deleted `id-leases.tsv`. Restore it, or run
`scripts/hr-mint --init --force` once no live run holds leases that are not on `main` yet.

**`down --purge` says `the status check failed`.** The check runs in the `htui-hr-dev` image;
`scripts/hr build` if it is missing.

**`gum not found`.** Install it (`apt install gum`) or pass the arguments the menu would ask for.

**`up` stops on `missing …`, `git user.name / user.email not set` or `claude not found`.** `up`
checks everything the run mounts before it clones. Create the missing file, set your git identity,
or install the tool with mise (or point `HR_MISE_PATH` at it).

**Gortex is missing or says `daemon already running`.** The `dev` container clears Gortex's stale
daemon files every time it starts, so `scripts/hr down ITEM` followed by `scripts/hr attach ITEM`
fixes a stuck daemon. Give a fresh run 20 seconds to index before judging. If Gortex still will not
start, the session carries on without it, as its instructions say for an integration failure.

**serena fails with `Read-only file system`.** `uvx` writes into its tool directory, and
`~/.local/share/uv/tools` is read-only in a run, so the run sets `UV_TOOL_DIR=/tmp/uv-tools`. If you
start a process some other way than `attach`, keep that variable.

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
bats --filter-tags '!docker' tests/hr              # 85 cases (56 hr, 29 hr-mint); stub docker, fixture repos
bats --filter-tags docker tests/hr/hr.bats         # D1–D5; real image and containers
```

The fast cases stub `docker`, use a fake home directory and never need a terminal. The Docker cases
use your real `$HOME` mounts and a real Docker daemon, but still clone from a fixture repo whose
items are `TOOL-9001` and `TOOL-9002` (compose projects `hr-tool-9001`/`hr-tool-9002`; a case skips
if one already exists). D1 builds the image; D2–D5 skip without it. D2 checks the mounts, the
`~/.claude` masks, the loopback-only host databases and the cargo isolation (empty read-only
`~/.cargo/bin`, per-run `registry/src`); D3 checks that two runs share neither databases nor
extracted crate sources; D5 plants a git filter in a clone and checks that it runs only in the
throwaway check container. Neither kind touches your real repo, state directory or registry: state,
runs, run directories and the fixture live under a temporary directory. The Docker cases mount your
real `~/.claude`, and `up` creates the fixture's empty project directory in `~/.claude/projects`;
the teardown removes it again (`rmdir`, so never anything with content). A fast case that needs to
read `compose.hr.yaml` as compose does runs `docker compose config` (the CLI only, no daemon) and
skips without the `docker` CLI.

Knobs that help when driving `scripts/hr` from a script or a test: `HR_INTERACTIVE=0|1` forces
non-interactive or interactive behaviour (default: interactive when stdin and stdout are
terminals), `HR_NO_GUM=1` behaves as if `gum` were not installed, and `scripts/hr ls --tsv` prints
the table as tab-separated text.
