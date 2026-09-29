# Blueprint: TOOL-7 — containerized /handoff-run sandboxes (T1, T3, T4, T5)

> Produced by `code-architect` 2026-09-29 from the CONFIRMED plan
> `.claude/plans/tool-7-hr-sandbox.plan.md`, which stays the spec (D1–D9 unchanged). §0 lists where
> the plan disagreed with the tree; every fix there is adopted unless marked otherwise. F6 was
> **voided** by the main thread (see its row).

## 0. Plan claims contradicted by the tree

| # | Finding | Evidence | Resolution (adopted) |
|---|---|---|---|
| F1 | `--prune` could lower the lease floor → duplicate mint. A leases MOD-70, merges; gc prunes MOD-70 (on main); B, cloned pre-merge, tree says MOD-70, no lease ≥70 → mints MOD-70 again. | plan §ID leases; `next-item-id.sh` header "a defect can never LOWER the next ID" | `--prune` **never removes the highest lease of each prefix** (kept as floor). |
| F2 | Mounting only `registry/`+`git/` shares the cache without cargo's lock (`.package-cache*`, `.global-cache` live at `$CARGO_HOME` root). | `ls -la ~/.cargo` | Whole `htui-hr-cargo` volume = runtime `CARGO_HOME` (`$HOME/.cargo`); rustup + installed tools in `/opt/rust`. |
| F3 | Image apt list can't build the workspace: needs `libdbus-1-dev`, `libssl-dev`. | `README.md:52-53`; `cargo tree -i libdbus-sys` (keyring → dbus-secret-service); `cargo tree -i openssl-sys` (native-tls ← hf-hub/fastembed, reqwest/hyper-tls). No `cmake` needed. | Add both. |
| F4 | PGDG repo unnecessary. | `apt-cache policy postgresql-client-16` → `noble/main` | Ubuntu archive; PGDG block kept as a comment. |
| F5 | `ubuntu:24.04` has `ubuntu` at 1000:1000; bash `UID` is readonly. | `getent passwd 1000` → `ubuntu:x:1000:1000` | `userdel -r ubuntu`; build args `HR_UID`/`HR_GID`/`HR_USER`/`HR_HOME`. |
| F6 | ~~ecc plugin hooks need node~~ — **VOID.** `ecc@ecc` is `false` in `~/.claude/settings.json`; no project settings re-enable it. Enabled plugins: superpowers (bash `run-hook.cmd`), remember (bash + `jq` + `python3`). `python3` → `~/.local/bin/python3` → uv CPython 3.13.7 under the ro `~/.local` mount; probed in `ubuntu:24.04`: runs, `sqlite3`/`json` import. | main-thread check 2026-09-29 | No node in the image. T2 still checks remember's hooks run clean. |
| F7 | PATH dirs: uv bins are `installs/uv/latest/.mise-bins/`; rg is `installs/ripgrep/latest/ripgrep-<ver>-x86_64-unknown-linux-musl/`; claude `installs/claude/latest/claude`. | on disk | `hr up` resolves into `HR_MISE_PATH` (glob for ripgrep); apt `ripgrep` fallback lower on PATH. Never put mise `shims` on PATH (they exec `mise`, whose `~/.config` isn't mounted). |
| F8 | Global git ignore (`~/.config/git/ignore:1` `**/.claude/settings.local.json`) isn't visible in the sandbox → copied `settings.local.json` shows untracked. | — | `hr up` appends host global ignore + `/.claude/settings.local.json` to `<clone>/.git/info/exclude`. |
| F9 | `~/.gortex` also has `memories/`, `store/`, `telemetry/`, `notebook-cache/`. | `ls ~/.gortex` | Land in the per-run volume; run-written Gortex memories die at purge. T2 decides whether to share `memories/`. |
| F10 | "ask before minting" vs sibling list printed *by* the mint. | plan hr-mint steps 3–4 | Docs say "ask before **filing**"; unused lease = harmless gap. |
| F11 | `handoff-add` step 3 reports `max open / max archived / next` (`handoff-add/SKILL.md:62`); `--dry-run` must not lease. | — | hr-mint prints a provenance line on stderr; dry-run uses read-only `next-item-id.sh`. |
| F12 | T4 needs lease-file creation and purged-run lease drop without hr-mint parsing hr's registry. | — | `hr-mint --init`; `hr-mint --prune --purged-owner <branch>` (repeatable). |
| F13 | Docker creates missing bind points / host paths as root. | Docker behaviour | Image pre-creates `~/.cache ~/.config ~/.gortex ~/.cargo ~/projects`; `hr up` pre-creates `<clone>/.remember`; long-syntax mounts with `create_host_path: false`; one-shot `init` service chowns volume roots. |
| F14 | Host git inside the clone reads the sandbox-writable `.git/config` (`core.fsmonitor` executes on host). | git semantics | All host-side git in a clone via `src_git` (fsmonitor/hooks off). `git fetch <src>` from host is safe (upload-pack ignores repo-config `packObjectsHook`). |

---

## T1 — `docker/hr/Dockerfile`, `docker/hr/compose.hr.yaml`, `docker/hr/.dockerignore`

- Build context `docker/hr`; `.dockerignore` = `*` plus a comment.
- Cargo layout (F2): build-time `CARGO_HOME=/opt/rust/cargo`, `RUSTUP_HOME=/opt/rust/rustup`; runtime
  `CARGO_HOME=$HR_HOME/.cargo` = whole shared volume.
- Labels `hr.uid`, `hr.gid`, `hr.home`; `hr up` compares with host and demands a rebuild on mismatch.

```dockerfile
# syntax=docker/dockerfile:1
# htui-hr-dev — toolchain for one /handoff-run sandbox (TOOL-7). Built only by `scripts/hr build`.
# Host tools (claude, gortex, graphify, headroom, uv/uvx, serena) are NOT baked in: ~/.local is
# mounted read-only so a sandbox never drifts from the host. Design: .claude/plans/tool-7-hr-sandbox.plan.md
FROM ubuntu:24.04 AS base
ARG HR_USER=mluigi
ARG HR_UID=1000
ARG HR_GID=1000
ARG HR_HOME=/home/mluigi
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq less procps ripgrep \
      build-essential pkg-config libdbus-1-dev libssl-dev \
      postgresql-client-16 \
 && rm -rf /var/lib/apt/lists/*
# Optional PGDG alternative (F4) — only if a newer 16.x minor is wanted:
#   install -d /usr/share/postgresql-common/pgdg && curl -fsSLo /usr/share/postgresql-common/pgdg/apt.postgresql.org.asc https://www.postgresql.org/media/keys/ACCC4CF8.asc
#   echo "deb [signed-by=/usr/share/postgresql-common/pgdg/apt.postgresql.org.asc] https://apt.postgresql.org/pub/repos/apt noble-pgdg main" >/etc/apt/sources.list.d/pgdg.list
# ubuntu:24.04 ships user `ubuntu` at 1000:1000 (F5).
RUN (userdel -r ubuntu 2>/dev/null || true) \
 && (getent group "$HR_GID" >/dev/null || groupadd -g "$HR_GID" "$HR_USER") \
 && useradd -m -d "$HR_HOME" -u "$HR_UID" -g "$HR_GID" -s /bin/bash "$HR_USER" \
 && install -d -o "$HR_UID" -g "$HR_GID" /opt/rust \
      "$HR_HOME/.cache" "$HR_HOME/.config" "$HR_HOME/.gortex" "$HR_HOME/.cargo" "$HR_HOME/projects" \
 && install -d /host /hr-state /hr-seed

FROM base AS dev
ARG HR_USER HR_UID HR_GID HR_HOME
USER ${HR_UID}:${HR_GID}
ENV RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo PATH=/opt/rust/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl --proto '=https' --tlsv1.2 -sSfo /tmp/rustup-init.sh https://sh.rustup.rs \
 && sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain 1.98.1 \
 && rm /tmp/rustup-init.sh \
 && rustup component add rustfmt clippy rust-analyzer --toolchain 1.98.1
RUN cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls \
 && cargo install cargo-insta --version 1.48.0 --locked \
 && rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git"
# Runtime CARGO_HOME = shared htui-hr-cargo volume (whole, incl. .package-cache locks).
ENV CARGO_HOME=${HR_HOME}/.cargo HOME=${HR_HOME} USER=${HR_USER} SHELL=/bin/bash
LABEL hr.uid=${HR_UID} hr.gid=${HR_GID} hr.home=${HR_HOME} hr.toolchain=1.98.1 hr.sqlx-cli=0.9.0 hr.cargo-insta=1.48.0
WORKDIR ${HR_HOME}
CMD ["sleep", "infinity"]
```
cargo-insta 1.48.0 = `insta` 1.48.0 in `Cargo.lock` (crates.io, not yanked, host same). `flock` is in util-linux (base).

```yaml
# One /handoff-run sandbox (TOOL-7). Instantiated only by scripts/hr:
#   docker compose -p hr-<item> -f docker/hr/compose.hr.yaml up -d --wait
# Every host path and per-run value comes from the environment scripts/hr exports; `:?` fails the
# parse rather than mount the wrong thing. No `ports:` anywhere: all three services share postgres's
# network namespace, so localhost:5439/6333/6334 inside mean what they mean on the host and nothing
# collides with htui-postgres/htui-qdrant or another run. Deliberately NOT mounted (D7):
# ~/.gitconfig, ~/.git-credentials, ~/.ssh.
services:
  init:            # one-shot: named-volume roots are created root-owned (F13)
    image: ${HR_IMAGE:-htui-hr-dev}
    user: "0:0"
    network_mode: none
    command: ["chown", "${HR_UID:?}:${HR_GID:?}", "${HR_HOME:?}/.gortex", "${HR_HOME:?}/.cargo"]
    volumes:
      - {type: volume, source: gortex, target: "${HR_HOME:?}/.gortex"}
      - {type: volume, source: cargo,  target: "${HR_HOME:?}/.cargo"}

  postgres:
    image: postgres:16
    command: ["postgres", "-p", "5439", "-c", "fsync=off", "-c", "synchronous_commit=off", "-c", "full_page_writes=off"]
    environment:
      POSTGRES_HOST_AUTH_METHOD: trust
    shm_size: 1gb
    volumes:
      - {type: volume, source: pgdata, target: /var/lib/postgresql/data}
    healthcheck:   # -h localhost: the entrypoint's init-time temp server is socket-only, so this goes green only on the real server
      test: ["CMD-SHELL", "pg_isready -h localhost -p 5439 -U postgres"]
      interval: 2s
      timeout: 3s
      retries: 30

  qdrant:
    image: qdrant/qdrant:v1.19.1
    network_mode: service:postgres
    depends_on: [postgres]
    volumes:
      - {type: volume, source: qdrant, target: /qdrant/storage}

  dev:
    image: ${HR_IMAGE:-htui-hr-dev}
    network_mode: service:postgres
    init: true                      # reap claude/cargo children; sleep is PID 1 otherwise
    cpus: ${HR_CPUS:-4}
    working_dir: ${HR_HOST_REPO:?}
    depends_on:
      postgres: {condition: service_healthy}
      qdrant:   {condition: service_started}
      init:     {condition: service_completed_successfully}
    # $$ = literal $ (compose would interpolate $HOME from the host otherwise)
    command: ["bash", "-c", "test -e \"$$HOME/.claude.json\" || cp /hr-seed/claude.json \"$$HOME/.claude.json\"; exec sleep infinity"]
    environment:
      USERNAME: htui-ci
      HTUI_TEST_DATABASE_URL: postgres://postgres@localhost:5439/postgres
      HTUI_TEST_QDRANT_URL: http://localhost:6334
      HR_ITEM: ${HR_ITEM:?}
      HR_SANDBOX: "1"
      HR_STATE: /hr-state
      GIT_AUTHOR_NAME: ${HR_GIT_NAME:?}
      GIT_AUTHOR_EMAIL: ${HR_GIT_EMAIL:?}
      GIT_COMMITTER_NAME: ${HR_GIT_NAME:?}
      GIT_COMMITTER_EMAIL: ${HR_GIT_EMAIL:?}
      PATH: ${HR_MISE_PATH:?}:/opt/rust/cargo/bin:${HR_HOME:?}/.cargo/bin:${HR_HOME:?}/.local/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
      TERM: ${TERM:-xterm-256color}
      COLORTERM: ${COLORTERM:-truecolor}
    volumes:
      - {type: bind, source: "${HR_SRC:?}",                  target: "${HR_HOST_REPO:?}",                   bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOST_REPO:?}",            target: /host/htui, read_only: true,           bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOST_REPO:?}/.remember",  target: "${HR_HOST_REPO:?}/.remember",         bind: {create_host_path: false}}
      - {type: bind, source: "${HR_STATE_HOST:?}",           target: /hr-state,                             bind: {create_host_path: false}}
      - {type: bind, source: "${HR_CLAUDE_JSON:?}",          target: /hr-seed/claude.json, read_only: true, bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.local",          target: "${HR_HOME:?}/.local", read_only: true, bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.claude",         target: "${HR_HOME:?}/.claude",                bind: {create_host_path: false}}
      - {type: volume, source: gortex,                       target: "${HR_HOME:?}/.gortex"}
      - {type: bind, source: "${HR_HOME:?}/.gortex/config.yaml",  target: "${HR_HOME:?}/.gortex/config.yaml",  read_only: true, bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.gortex/instructions", target: "${HR_HOME:?}/.gortex/instructions", read_only: true, bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.gortex/models",       target: "${HR_HOME:?}/.gortex/models",       read_only: true, bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.headroom",       target: "${HR_HOME:?}/.headroom",              bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.serena",         target: "${HR_HOME:?}/.serena",                bind: {create_host_path: false}}
      - {type: bind, source: "${HR_HOME:?}/.cache/uv",       target: "${HR_HOME:?}/.cache/uv",              bind: {create_host_path: false}}
      - {type: volume, source: cargo,                        target: "${HR_HOME:?}/.cargo"}

volumes:
  pgdata:
  qdrant:
  gortex:
  cargo:
    external: true
    name: htui-hr-cargo
```

**Env contract** (names `scripts/hr` must export):

| var | meaning | default owner |
|---|---|---|
| `HR_ITEM` | item ID; also in `dev` | `hr up` |
| `HR_SRC` | host path of the run's clone `<root>/<ITEM>/src` | `hr up` (`HR_ROOT`, or `HR_SSD_ROOT` with `--ssd`) |
| `HR_HOST_REPO` | host repo main worktree; = clone's container path; also ro at `/host/htui` | `hr`: `dirname $(git rev-parse --path-format=absolute --git-common-dir)` of `scripts/..` |
| `HR_HOME` | host `$HOME` = container home | `hr` (must equal label `hr.home`) |
| `HR_UID` / `HR_GID` | owner for `init`'s chown | `hr` (`id -u`/`id -g`; must equal labels) |
| `HR_STATE_HOST` | host state dir → `/hr-state` | `hr` (`$HR_STATE`, else `/media/projects/htui-hr/.state`) |
| `HR_CLAUDE_JSON` | per-run `~/.claude.json` snapshot, ro, seeded once | `hr up` (`<run>/claude.json`, 600) |
| `HR_MISE_PATH` | resolved mise bin dirs, ahead of `~/.local/bin` | `hr up` |
| `HR_GIT_NAME` / `HR_GIT_EMAIL` | commit identity | `hr` (`git -C $HR_HOST_REPO config user.*`) |
| `HR_CPUS` | CPU quota | compose default 4; `hr up --cpus N` |
| `HR_IMAGE` | image tag | compose default `htui-hr-dev` |
| `TERM` / `COLORTERM` | terminal | compose defaults; `hr attach` passes `-e` |

hr-only: `HR_ROOT` (`/media/projects/htui-hr`), `HR_SSD_ROOT` (`~/htui-hr`), `HR_STATE`, `HR_DOCKER`
(default `docker`; tests stub), `HR_INTERACTIVE` (auto/1/0), `HR_NO_GUM`.

`HR_MISE_PATH` on this host, in order: `$HOME/.local/share/mise/installs/claude/latest` (required;
`hr up` exits 2 if `claude` missing), `$HOME/.local/share/mise/installs/uv/latest/.mise-bins`
(required; before stale `~/.local/bin` uv 0.8.22), glob `$HOME/.local/share/mise/installs/ripgrep/latest/ripgrep-*/` (optional).

T1 done-check additions: `ls -ld ~/.gortex ~/.cargo ~/.cache` user-owned; `rust-analyzer --version`;
host `stat -c %U <repo>/.remember` not root; `docker compose up --wait` tolerates the exited `init`
(else `hr` uses `up -d --wait postgres qdrant dev`).

---

## T3 — `scripts/hr-mint`, `tests/hr/test_helper.bash`, `tests/hr/hr-mint.bats`

```
scripts/hr-mint --prefix P --title "T" [--repo-root R] [--branch B]    # mint; ID on stdout only
scripts/hr-mint --leasing                                              # exit 0 iff lease file exists
scripts/hr-mint --init                                                 # mkdir -p state dir, create lease file with header; idempotent
scripts/hr-mint --prune [--ref main] [--repo-root R] [--purged-owner hr/X]...
```
- `R` default `$(cd "$SCRIPT_DIR/.." && pwd)`. `next-item-id.sh`/`workflow-patterns.sh` always from
  hr-mint's own repo (`$SCRIPT_DIR/../.claude/skills/handoff-run/scripts/`); `--repo-root` only
  selects data.
- Owner: `--branch B`, else `hr/$HR_ITEM` when set, else `host`. State: `$HR_STATE`, else
  `/media/projects/htui-hr/.state`. Lease file `$STATE/id-leases.tsv`; lock `$STATE/id-leases.lock`
  (separate, so prune's tmp+mv never swaps the locked inode).
- Style mirrors `next-item-id.sh`: header (purpose, usage, exit codes, "the law is workflow-docs.md"),
  `set -u`, `while case` parser, `uname` Windows refusal ("Linux-only; on Windows mint with
  next-item-id.ps1"); mode 100755, no suffix.

Lease format v1 (owned only by hr-mint):
```
# hr-mint leases v1 - owner: scripts/hr-mint (never edit by hand)
# id<TAB>owner<TAB>leased_at_utc<TAB>title
MOD-71	hr/MOD-65	2026-09-29T10:02:11Z	Ctrl-c leaves the terminal raw
```
Row regex `^((ANA|MOD|NEXT|VAL|TOOL|CLEAN)-[0-9]+)\t([^\t]+)\t([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z)\t(.*)$`;
`#` comments; anything else → exit 1 "lease file line N unparseable" (fail closed). Titles: tab/CR/LF
→ space; empty → exit 2.

Mint:
```
P=upper(P); P ∉ PREFIXES (source workflow-patterns.sh) → exit 2
no state dir → out=$(next-item-id.sh --prefix P --id-only --repo-root R); rc passthrough
             (rc≠0: captured stdout → STDERR — next-item-id prints "Unknown prefix" to stdout);
             print ID; stderr "hr-mint: leasing off (no state dir at $STATE)"; exit rc
state dir, no lease file → same, stderr "leasing off (run hr-mint --init)"
exec 9>>"$LOCK"; flock -w "${HR_MINT_LOCK_TIMEOUT:-30}" 9 || exit 3
tree=$(next-item-id.sh --prefix P --id-only --repo-root R 9>&-) ; rc≠0 → stdout→stderr, exit rc, no row
floor = max N over lease rows with prefix P (0 if none)
n = max(${tree#P-}, floor+1); id="P-n"
printf '%s\t%s\t%s\t%s\n' "$id" "$owner" "$(date -u +%FT%TZ)" "$title" >> "$LEASES"
exec 9>&-
stdout: $id
stderr: "hr-mint: leased $id for $owner (tree next $tree, lease floor P-$floor|none)"
stderr (if any): "hr-mint: other $P leases in the last 7 days — ask the maintainer before filing if one is the same problem:"
        then "  <id>  <owner>  <ts>  <title>" for every OTHER row with prefix P and ts ≥ now-7d
```
Prune:
```
flock as above
open=$(git -C R show "$REF:HANDOFF.md") || exit 2 "ref has no HANDOFF.md"
arch=$(git -C R show "$REF:DECISIONS.md" 2>/dev/null)
on_ref = IDs via CHECKLIST_LINE_PATTERN|CHECKLIST_LINE_LOOSE_PATTERN over open
       + INDEX_LINE_PATTERN|INDEX_LINE_LOOSE_PATTERN over arch
max[P] per prefix over all rows
keep row iff  id == P-max[P]  ||  (id ∉ on_ref && owner ∉ purged_owners)     # F1 floor
write kept rows (+comments) to "$LEASES.tmp.$$" same dir; mv -f; stderr "pruned N, kept M"
```
Exit codes: 0 ok · 1 blocked (next-item-id findings, unparseable lease file, `--leasing` off) · 2 usage
· 3 lock timeout / state dir unwritable.

`test_helper.bash`: `setup` → `bats_require_minimum_version 1.5.0`; `export HR_STATE="$BATS_TEST_TMPDIR/state"`
and assert it is under `BATS_TEST_TMPDIR`; `unset HR_ITEM`; `HR_MINT=$BATS_TEST_DIRNAME/../../scripts/hr-mint`.
`make_fixture DIR` (green under `next-item-id.sh` and `validate-workflow-docs.sh`): `git init -q -b main`
+ local user; `HANDOFF.md` = `# HANDOFF - Outstanding Work (fixture)`, `**Current status (2026-01-05):** fixture.`,
`## Open items` / `### Next features`, `- [ ] **MOD-4 - Thing.** body`, `- [ ] **MOD-5 - Other thing.** body`,
`## Summary` with `| MOD-N | 2 (MOD-4 thing, MOD-5 other) |`; `DECISIONS.md` =
`- **[MOD-3](docs/decisions/mod/mod-3.md)** - Old thing (done, 2026-01-05)`; `docs/decisions/mod/mod-3.md`;
commit. Tree next = **MOD-6**. `seed_lease ID OWNER TS TITLE` (fixture only). No bats-assert (not
installed): plain `[[ ]]`, `run --separate-stderr`.

`hr-mint.bats` cases:
1. `--leasing` 1 without file, 0 after `--init`; `--init` idempotent (no duplicate header).
2. No state dir → `MOD-6`, stderr "leasing off", nothing created.
3. First leased mint `MOD-6`; one row, owner `host`, ISO ts, title.
4. Floor beats tree: seed MOD-9 → MOD-10. 5. Tree beats lease: seed MOD-2 → MOD-6.
6. 20 parallel mints (distinct `--branch`) → exactly MOD-6..MOD-25, 20 well-formed rows.
7. next-item-id non-zero (MOD-4 open twice; no HANDOFF.md) → exit 1, empty stdout, lease file byte-identical.
8. `FOO` → exit 2, no row; `mod` → MOD-6.
9. Empty title exit 2; tab/newline title stored on one line.
10. Owner: `HR_ITEM=MOD-65` → `hr/MOD-65`; `--branch x` overrides; unset → `host`.
11. Siblings: other owner <7d same prefix listed; 8d old not; other prefix not; own row never; stdout ID-only.
12. `--prune`: main has MOD-4/5 open, MOD-3 archived; leases MOD-3,5,7,8 → drop 3,5; keep 7,8.
13. Floor kept (F1): all leases on ref → highest (MOD-5) survives; next mint MOD-6.
14. `--purged-owner hr/MOD-65` drops that owner's non-max rows only.
15. Prune reads the ref, not the worktree (uncommitted HANDOFF line doesn't prune).
16. Bad ref → exit 2, file unchanged. 17. Corrupt line → mint exit 1 naming the line.
18. Lock held elsewhere + `HR_MINT_LOCK_TIMEOUT=1` → exit 3.

---

## T4 — `scripts/hr`, `tests/hr/hr_helper.bash`, `tests/hr/hr.bats` (after T2)

Skeleton: `set -u`; header; `SELF_ROOT=scripts/..` (`readlink -f`); `HR_HOST_REPO` default main
worktree; `HR_SANDBOX=1` → exit 2 (host-only); `DOCKER=${HR_DOCKER:-docker}`;
`COMPOSE=("$DOCKER" compose -p "hr-${item,,}" -f "$SELF_ROOT/docker/hr/compose.hr.yaml")`;
`interactive` = `HR_INTERACTIVE` else `[[ -t 0 && -t 1 ]]`; `need_gum` only on interactive paths →
exit 2 "gum not found (apt install gum) — or pass the arguments". Non-TTY: missing choice = usage
(2); confirms need `--yes` else exit 1 "needs confirmation"; `gum table` → `column -t -s $'\t'`;
`gum spin` → plain.
`src_git() { git --no-pager -C "$SRC" -c core.fsmonitor=false -c core.hooksPath=/dev/null -c core.untrackedCache=false "$@"; }` (F14) for every host-side git call in a clone.
Exit codes: 0 ok · 1 refused (duplicate, not open, dirty, non-ff, uncollected, conflict, no confirm) ·
2 usage / missing dependency · 3 external failure.

Run registry `$HR_STATE/runs/<ITEM>` (owned by hr; tmp+mv under `flock $HR_STATE/runs.lock`; parsed
with `IFS='=' read -r k v`, never sourced):
```
# hr run registry v1 - owner: scripts/hr
item=MOD-65
project=hr-mod-65
branch=hr/MOD-65
run_dir=/media/projects/htui-hr/MOD-65
root_kind=hdd            # ssd with --ssd
base_ref=main
base_sha=<40hex>
cpus=4
created_at=2026-09-29T10:00:00Z
collected_sha=
collected_at=
```
`load_run ITEM` exports `HR_ITEM`, `HR_SRC=$run_dir/src`, `HR_CLAUDE_JSON=$run_dir/claude.json`,
`HR_CPUS` + host-derived vars. Live state always from Docker.

Verbs:
- **`hr`**: interactive → `gum choose build up attach ls collect down gc`; else usage, exit 2.
- **`build`**: `$DOCKER build -t $HR_IMAGE -f $SELF_ROOT/docker/hr/Dockerfile --build-arg HR_USER=$(id -un) --build-arg HR_UID=$(id -u) --build-arg HR_GID=$(id -g) --build-arg HR_HOME=$HOME $SELF_ROOT/docker/hr` (in `gum spin --show-output` when interactive); `$DOCKER volume create htui-hr-cargo`.
- **`up [ITEM] [--ssd] [--from REF] [--cpus N] [--yes]`**:
  1. `image inspect -f '{{index .Config.Labels "hr.uid"}} {{index .Config.Labels "hr.gid"}} {{index .Config.Labels "hr.home"}}'`; missing → interactive `gum confirm` build, else exit 2 "run scripts/hr build"; labels ≠ `(id -u, id -g, $HOME)` → exit 2 "rebuild".
  2. `sha=$(git -C HOST rev-parse --verify "$REF^{commit}")` else exit 2.
  3. No ITEM: picker from `git -C HOST show "$sha:HANDOFF.md" | sed -nE 's/^- \[ \] \*\*((ANA|MOD|NEXT|VAL|TOOL|CLEAN)-[0-9]+) - ([^*]*).*/\1\t\3/p'` minus IDs with a `runs/` file → `gum filter --header "Open items on $REF" --placeholder …`. ITEM: uppercase, `ID_ALT` (else 2), open line on `$sha` (else 1).
  4. `runs.lock` flock; `runs/ITEM` exists → exit 1 "run exists — hr attach/hr down"; root = `HR_SSD_ROOT` with `--ssd` else `HR_ROOT`; `run_dir` exists → exit 1 "stale run dir".
  5. `git -C HOST status --porcelain -- .claude HANDOFF.md DECISIONS.md` → stderr list + "the clone takes committed $REF only"; interactive `gum confirm` (default yes); non-interactive continue.
  6. Preflight: sources exist (`~/.local`, `~/.claude`, `~/.gortex/{config.yaml,instructions,models}`); `mkdir -p ~/.headroom ~/.serena ~/.cache/uv HOST/.remember $HR_STATE/runs`; `hr-mint --init` (with `HR_STATE` exported); resolve `HR_MISE_PATH` unless preset.
  7. Clone: `git clone -q --no-hardlinks --no-checkout "$HR_HOST_REPO" "$SRC"`; `src_git switch -q -c "hr/$ITEM" "$sha"`; `src_git remote set-url origin /host/htui`; `src_git config remote.origin.pushurl no-push-from-sandbox`; append `~/.config/git/ignore` (if any) + `/.claude/settings.local.json` to `$SRC/.git/info/exclude` (F8); `cp HOST/.claude/settings.local.json $SRC/.claude/` if present; `mkdir -p $SRC/.remember`; `install -m 600 ~/.claude.json $run_dir/claude.json`.
  8. Write registry **before** compose (so `down --purge --force` can clean a failed up); release lock.
  9. `$DOCKER volume create htui-hr-cargo`; `"${COMPOSE[@]}" up -d --wait` in `gum spin`; failure → exit 3, hint `hr down ITEM --purge --force`.
  10. Print `attach: scripts/hr attach ITEM`.
- **`attach [ITEM] [--shell]`**: no ITEM + interactive → `gum choose` over registered runs; `load_run`; `dev` not running (`compose ps --status running -q dev` empty) → `compose up -d --wait`; then `"${COMPOSE[@]}" exec -e TERM -e COLORTERM dev claude --dangerously-skip-permissions` (or `bash -l`).
- **`ls`**: per registry file, tab row `ITEM STATE BRANCH AHEAD DISK ROOT`. STATE running / stopped (`compose ps -a -q`) / down, `+collected` if the collected test passes. AHEAD: `M=$(git -C HOST rev-parse main)`; `src_git cat-file -e "$M^{commit}"` → `rev-list --count "$M..hr/$ITEM"`, else `base_sha..hr/$ITEM` + `*`. DISK `du -sh` (in `gum spin`). ROOT hdd/ssd. `gum table -p -s $'\t' -c ITEM,STATE,BRANCH,AHEAD,DISK,ROOT` or `column -t`.
- **`collect ITEM [--merge] [--force] [--yes]`**:
  ```
  load_run; tip=$(src_git rev-parse --verify -q refs/heads/hr/ITEM) || exit 1
  git -C HOST worktree list --porcelain | grep -qx "branch refs/heads/hr/ITEM" → exit 1
  old=$(git -C HOST rev-parse --verify -q refs/heads/hr/ITEM)
  if [[ -n $old && $old != $tip ]] && ! src_git merge-base --is-ancestor "$old" "$tip" 2>/dev/null:
       ! force → exit 1 "hr/ITEM was rewritten in the sandbox (host has $old); --force overwrites"
  spec=refs/heads/hr/ITEM:refs/heads/hr/ITEM; force → "+$spec"
  git -C HOST -c core.hooksPath=/dev/null fetch -q --no-tags --no-write-fetch-head --no-recurse-submodules "$SRC" "$spec" || exit 3
  registry collected_sha=$tip collected_at=now
  git -C HOST --no-pager log --oneline main..hr/ITEM
  --merge:
    dirty tracked (status --porcelain --untracked-files=no) → exit 1
    MERGE_HEAD exists → exit 1; detached HEAD → exit 1
    git -C HOST merge --no-ff --no-edit -m "Merge hr/ITEM (sandbox run)" hr/ITEM
    conflict → U=$(git -C HOST diff --name-only --diff-filter=U); print U
       all U ⊆ {HANDOFF.md, DECISIONS.md, docs/decisions/**} → print recipe; interactive && gum confirm
            → (cd HOST && claude "$RECIPE_PROMPT")   # interactive, normal permissions, maintainer present
       else "code conflicts: resolve by hand (git merge --abort to back out)"
       exit 1 (merge in progress)
    clean → validate-workflow-docs.sh --repo-root HOST; rc≠0 → exit 1 "merged, validator red — fix before push"
  never push
  ```
  `RECIPE_PROMPT` = the plan's 4-step recipe verbatim + "then `git add` the resolved files, `git commit --no-edit`, rerun the validator".
- **Collected test**: `git -C HOST cat-file -e "$tip^{commit}"` and (`merge-base --is-ancestor $tip refs/heads/hr/ITEM` or `… $tip main`). A no-commit run passes.
- **`down ITEM [--purge] [--force] [--yes]`**: plain → `compose down`. `--purge`: refuse (1) unless collected, clone tracked-clean, and no clone branch tip missing from HOST (list) — `--force` bypasses; confirm (gum / `--yes`); `compose down -v` (never removes `htui-hr-cargo`); guarded `rm -rf` (`run_dir` = `$root/$ITEM`, non-empty, under `HR_ROOT`/`HR_SSD_ROOT`) + registry file; forced uncollected → `hr-mint --prune --purged-owner hr/ITEM --repo-root HOST`.
- **`gc [--yes]`**: candidates = `dev` not running ∧ collected ∧ clean clone; interactive `gum choose --no-limit`, else list and purge only with `--yes`; same purge path; then `hr-mint --prune --ref main --repo-root "$HR_HOST_REPO"`. hr never edits the lease file.

`hr_helper.bash`: loads T3's `test_helper` (read-only reuse) for `make_fixture`; exports
`HR_HOST_REPO` (fixture), `HR_ROOT`, `HR_SSD_ROOT`, `HR_STATE`, fake `HOME` with `.claude.json`,
`.claude/`, `.local/`, `.gortex/{config.yaml,instructions,models}` — all asserted under
`BATS_TEST_TMPDIR`; `HR_MISE_PATH=/usr/bin`, `HR_INTERACTIVE=0`, `HR_DOCKER=$BATS_TEST_TMPDIR/bin/docker`.
Stub docker logs `$*` to `$HR_TEST_DOCKER_LOG`; `image inspect -f …` → `"$(id -u) $(id -g) $HOME"`;
`compose … ps …` → empty; else exit 0. `mk_run ITEM` = `hr up ITEM` + one commit in `$SRC`.
`host_snapshot` = HEAD, symbolic-ref, `status --porcelain`, `for-each-ref refs/heads`.

`hr.bats` (no Docker; `bats --filter-tags '!docker' tests/hr`):
1. No verb non-interactive → 2 + usage; `HR_SANDBOX=1` → 2.
2. `up` bad ID → 2; `up MOD-99` (not open) → 1.
3. `up MOD-5` happy path: clone on `hr/MOD-5` at host main sha; origin `/host/htui`; exclude has the settings line; settings copied; `$SRC/.remember`; claude.json 600; registry fields; docker log `compose -p hr-mod-5 -f …/compose.hr.yaml up -d --wait`; host snapshot unchanged.
4. `up` warns on modified tracked HANDOFF.md + untracked `.claude/x`; clone HANDOFF = committed.
5. Duplicate → 1, no second clone, no compose call.
6. `--ssd` → under `HR_SSD_ROOT`, `root_kind=ssd`; `--from feature` → that sha.
7. Missing image → 2 naming `scripts/hr build`; label mismatch → 2 "rebuild".
8. `collect` fetch-only: host ref = clone tip; snapshot otherwise unchanged; no FETCH_HEAD; log shown.
9. Second collect after more commits → ff. 10. After `--amend` in clone → 1, host ref unchanged; `--force` updates.
11. `hr/MOD-5` checked out in a host worktree → 1.
12. `--merge` dirty tracked → 1, no merge, HEAD unchanged. 13. `--merge` clean → 2-parent merge, validator green.
14. `--merge` HANDOFF-only conflict non-interactive → 1, list + recipe, MERGE_HEAD exists, no claude launched.
15. `--merge` code conflict → 1 "resolve by hand".
16. `down` → `compose … down` without `-v`; run dir kept.
17. `down --purge` uncollected → 1, dir intact; dirty tracked clone → 1.
18. `down --purge` non-interactive without `--yes` → 1.
19. `down --purge --yes` after collect → `down -v` logged, dir + registry gone, lease file untouched.
20. `down --purge --force --yes` uncollected → purged, owner lease pruned (per-prefix max stays).
21. `ls` → header + MOD-5 row, AHEAD = commits made.
22. `gc --yes` → purges collected, keeps uncollected; lease on main pruned.
23. `HR_INTERACTIVE=1`, no gum on PATH, `up` no ID → 2 naming gum.

Docker-tagged (`# bats test_tags=docker`, real HOME, fixture host repo): D1 `hr build` → labels
match; D2 `hr up` then `compose exec -T dev`: cargo 1.98.1, `cargo sqlx --version` 0.9.0, psql
`select 1`, `curl -fs localhost:6333`, empty `credential.helper`, `touch /host/htui/x` fails, no
publishers; D3 two runs, table in one absent in other; D4 `down --purge` removes per-run volumes,
`htui-hr-cargo` survives.

---

## T5 — text blocks

**`.claude/skills/handoff-run/SKILL.md`**
- References bullet after `next-item-id`: `` - `scripts/hr-mint` (repo root, Linux) — the leased mint; P0 mints through it whenever `scripts/hr-mint --leasing` exits 0 (sandbox runs exist). User guide: `docs/hr-sandbox.md` ``
- Top of "### 1. Locate the item": `` In a sandbox (`HR_SANDBOX=1` is set) read **Sandbox mode** below first — it overrides steps 1, 1.5 and 5. ``
- New section before "## Hard rules":

```markdown
## Sandbox mode (`HR_SANDBOX=1`)

Set by `scripts/hr` (TOOL-7; user guide `docs/hr-sandbox.md`). The session runs in a private clone on
branch `hr/$HR_ITEM`, with its own Postgres and Qdrant on the usual `localhost` ports, no Docker and no
git credentials. These overrides apply; everything else in this file — every maintainer gate and the
reviewer gate included — is unchanged.

- **Steps 1 / 1.5 — the item is `$HR_ITEM`.** `next`, or a bare `/handoff-run`, resolves to it with no
  selection subagent. A different ID is asked, never followed silently: the item was chosen on the host
  by `scripts/hr up`, whose picker already hides items that have a run.
- **New items (lifecycle P0) mint through `scripts/hr-mint`** — the lease file always exists in a
  sandbox. Its sibling listing is how two runs avoid filing one problem under two IDs: when a listed
  title looks like the same issue, ask the maintainer before filing; on reuse, cite the sibling's ID in
  this run's phase note instead of creating the item.
- **Database** — `psql -h localhost -p 5439 -U postgres` (trust auth, no password).
  `HTUI_TEST_DATABASE_URL` and `HTUI_TEST_QDRANT_URL` are already set. There is no `docker` here: a
  recipe that says `docker exec htui-postgres psql …` becomes the same `psql` command against
  `localhost:5439`.
- **Step 5, report — never push, never open a PR** (`origin` is the read-only host repo). The
  done-report ends with exactly: `branch hr/<ITEM> ready — on host: scripts/hr collect <ITEM>`.
  Merging back, and the real Postgres gate on the merged tree, happen on the host.
```
- Hard rules: `` - In a sandbox (`HR_SANDBOX=1`) the item is `$HR_ITEM` and nothing is pushed — see Sandbox mode. ``

**`references/lifecycle.md` P0 item 1** — insert after "1. Mint the ID — run the script, do not derive it by eye:" before the code block:
```markdown
   **When sandbox runs exist, lease the ID.** On Linux, if `scripts/hr-mint --leasing` exits 0 (always
   true inside a sandbox; true on the host while `scripts/hr` runs exist), mint with
   `scripts/hr-mint --prefix <PREFIX> --title "<one-line title>"` instead of the commands below. It
   runs the same `next-item-id.sh` and takes the higher of that and every other run's lease + 1, so
   parallel runs never share an ID. Stdout is the bare ID; stderr says how it was reached and lists
   other leases for the prefix from the last 7 days with their titles — if one looks like the same
   problem, ask the maintainer before filing (a leased-but-unused ID is a harmless gap). Non-zero exit
   blocks the mint exactly as below. Otherwise:
```
**`handoff-add/SKILL.md` step 3** — after the code block (line 60); also add `hr-mint` to the References bullet (line 20):
```markdown
With sandbox runs live (Linux: `scripts/hr-mint --leasing` exits 0), mint with
`scripts/hr-mint --prefix <PREFIX> --title "<title>"` instead — except under `--dry-run`, which runs
only the read-only report above and notes that the real mint is leased and may be higher. `hr-mint`'s
stderr line (`leased <ID> … (tree next …, lease floor …)`) replaces `max open / archived` in the
verdict's `ID mint:` field, and its sibling listing goes in the verdict too: a title that looks like the
same problem is put to the maintainer before anything is written. A rejected verdict leaves the leased
ID as a harmless gap.
```
**`handoff-docs.md` lines 39–43** replacement sentence: `… mints the next ID by running `.claude/skills/handoff-run/scripts/next-item-id.sh --prefix <PREFIX>` (macOS/Linux) or `next-item-id.ps1 -Prefix <PREFIX>` (Windows) — or, while `scripts/hr` sandbox runs exist (`scripts/hr-mint --leasing` exits 0), `scripts/hr-mint --prefix <PREFIX> --title "<title>"`, which leases the ID so parallel runs never mint the same one (owned-ID method; IDs are per-repo and never reused), files it …`

**`docs/hr-sandbox.md` outline**: 1 what/when (3–4 runs; Linux only) · 2 requirements (Docker ≥26 +
compose v2, gum, host tools via mise/`~/.local`, bats) · 3 `hr build` (rebuild after uid/home change)
· 4 `hr up` (committed-only warning; what's copied in) · 5 `hr attach`, `/handoff-run` inside · 6
inside a run: ports/DSN, psql, `sqlx prepare` variant, no docker/push/gh · 7 IDs across runs · 8
bringing work home: collect/--merge/--force, 4-step recipe, host Postgres gate with no runs building
(57P03) · 9 ls/down/gc · 10 storage + env overrides; watch `df -h /media` · 11 in/out of the sandbox
(condensed mounts; credentials absent) · 12 troubleshooting (stale labels, Gortex fallback, auth
fallback `claude setup-token`, disk) · 13 tests (`bats --filter-tags '!docker' tests/hr`; full `bats tests/hr`).

**`README.md`** — new last subsection of `## Development`, after "### Windows-specific code":
```markdown
### Running several `/handoff-run` sessions at once

`scripts/hr` gives each run its own container set: a fresh clone on branch `hr/<ITEM>`, a private
Postgres and Qdrant, and the full toolchain, so three or four runs can work side by side without
touching each other or this checkout. Linux only; needs Docker and `gum`. See
[`docs/hr-sandbox.md`](docs/hr-sandbox.md).
```

---

## Build order and commit boundaries

Explicit-path commits only; no `add -A`, `stash`, `--amend`; `chmod +x` before `git add`.

1. **Wave 1 — T1**: C1 `feat(hr): htui-hr-dev toolchain image (TOOL-7 T1)` (Dockerfile + .dockerignore); C2 `feat(hr): per-run compose project (TOOL-7 T1)` after the done-check.
2. **Wave 1 ∥ — T3**: C1 `test(hr): hr-mint lease cases (TOOL-7 T3)` (red); C2 `feat(hr): hr-mint cross-run ID leases (TOOL-7 T3)` (green, incl. `--init`, `--purged-owner`).
3. **T2 spike** (main thread + maintainer); adds F9 (gortex `memories/`) and remember-hooks-clean; plan amended with any fallback.
4. **Wave 2 — T4**: C1 tests (red); C2 `feat(hr): hr build/up/attach/ls`; C3 `feat(hr): hr collect/down/gc` (non-Docker bats green; Docker cases run once).
5. **Wave 2 ∥ — T5**: C1 `docs(handoff): mint through hr-mint when leasing is on (TOOL-7 T5)`; C2 `docs(handoff): handoff-run sandbox mode`; C3 `docs: hr sandbox user guide + README pointer`; validator after each.
6. **T6** as planned.

Ordering constraint: T4 reuses T3's `test_helper.bash` read-only → T3 C1 lands before T4 C1.
