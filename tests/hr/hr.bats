#!/usr/bin/env bats
# hr.bats — the host driver scripts/hr (TOOL-7 T4). Contract: .claude/plans/tool-7-hr-sandbox.blueprint.md §T4.
# Fast cases (stub docker, fixture host repo, fake HOME):  bats --filter-tags '!docker' tests/hr
# Docker cases (real image + daemon, fixture host repo):    bats --filter-tags docker tests/hr/hr.bats

load hr_helper

setup() {
    # Docker-tagged cases do their own setup (real HOME, real docker).
    [[ "$BATS_TEST_DESCRIPTION" == D* ]] && return 0
    hr_setup
}

teardown() {
    [[ "$BATS_TEST_DESCRIPTION" == D* ]] && hr_docker_teardown
    return 0
}

main_sha() { git -C "$HR_HOST_REPO" rev-parse main; }

# ---------------------------------------------------------------------------------------------
# up

@test "1. no verb non-interactive -> usage, exit 2; inside a sandbox -> exit 2" {
    run --separate-stderr "$HR"
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'Usage'* ]]

    HR_SANDBOX=1 run --separate-stderr "$HR" ls
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'host'* ]]

    run --separate-stderr "$HR" frobnicate
    [[ $status -eq 2 ]]

    run --separate-stderr "$HR" --help
    [[ $status -eq 0 ]]
    [[ "$output" == *'Usage'* ]]
    [[ ! -s "$HR_TEST_DOCKER_LOG" ]]
}

@test "2. up: malformed ID -> 2; ID not open on the ref -> 1; nothing cloned" {
    run --separate-stderr "$HR" up FOO-1
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR" up MOD5
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR" up MOD-99
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'MOD-99'*'not open'* ]]
    run --separate-stderr "$HR" up MOD-3     # archived, not open
    [[ $status -eq 1 ]]
    [[ ! -e "$HR_ROOT/MOD-99" && ! -e "$HR_ROOT/MOD-3" ]]
    [[ ! -e "$(reg_of MOD-99)" ]]
    ! grep -q '^compose' "$HR_TEST_DOCKER_LOG" || false
}

@test "3. up MOD-5: clone, branch, origin, excludes, copies, registry, compose call; host untouched" {
    local before
    before="$(host_snapshot)"
    run --separate-stderr "$HR" up mod-5
    [[ $status -eq 0 ]]
    [[ "$output" == *'scripts/hr attach MOD-5'* ]]

    local src
    src="$(src_of MOD-5)"
    [[ "$(git -C "$src" symbolic-ref HEAD)" == refs/heads/hr/MOD-5 ]]
    [[ "$(git -C "$src" rev-parse HEAD)" == "$(main_sha)" ]]
    [[ "$(git -C "$src" remote get-url origin)" == /host/htui ]]
    [[ "$(git -C "$src" config remote.origin.pushurl)" == no-push-from-sandbox ]]
    [[ -z "$(git -C "$src" status --porcelain --untracked-files=all)" ]]
    grep -qxF '/.claude/settings.local.json' "$src/.git/info/exclude"
    grep -qxF '*.fake-global-ignore' "$src/.git/info/exclude"
    cmp "$src/.claude/settings.local.json" "$HR_HOST_REPO/.claude/settings.local.json"
    [[ -d "$src/.remember" && -d "$HR_HOST_REPO/.remember" ]]
    [[ "$(stat -c %a "$HR_ROOT/MOD-5/claude.json")" == 600 ]]
    cmp "$HR_ROOT/MOD-5/claude.json" "$HOME/.claude.json"
    [[ -d "$HOME/.headroom" && -d "$HOME/.serena" && -d "$HOME/.cache/uv" ]]
    # hr-mint --init ran: leasing is on for every run and the host.
    [[ -f "$HR_STATE/id-leases.tsv" ]]

    # Registry.
    [[ "$(head -n 1 "$(reg_of MOD-5)")" == '# hr run registry v1 - owner: scripts/hr' ]]
    [[ "$(reg_get MOD-5 item)" == MOD-5 ]]
    [[ "$(reg_get MOD-5 project)" == hr-mod-5 ]]
    [[ "$(reg_get MOD-5 branch)" == hr/MOD-5 ]]
    [[ "$(reg_get MOD-5 run_dir)" == "$HR_ROOT/MOD-5" ]]
    [[ "$(reg_get MOD-5 root_kind)" == hdd ]]
    [[ "$(reg_get MOD-5 base_ref)" == main ]]
    [[ "$(reg_get MOD-5 base_sha)" == "$(main_sha)" ]]
    [[ "$(reg_get MOD-5 cpus)" == 4 ]]
    [[ "$(reg_get MOD-5 created_at)" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z$ ]]
    grep -qx 'collected_sha=' "$(reg_of MOD-5)"

    # Docker: shared cargo volume, then the run's compose project.
    grep -qx 'volume create htui-hr-cargo' "$HR_TEST_DOCKER_LOG"
    grep -qxF "compose -p hr-mod-5 -f $HR_COMPOSE_FILE up -d --wait" "$HR_TEST_DOCKER_LOG"

    [[ "$(host_snapshot)" == "$before" ]]
}

@test "3b. up exports every variable compose.hr.yaml requires (env contract)" {
    run --separate-stderr "$HR" up MOD-5 --cpus 6
    [[ $status -eq 0 ]]
    local env="$HR_TEST_DOCKER_LOG.env" var
    local -a required
    mapfile -t required < <(grep -oE '\$\{[A-Z_]+:\?\}' "$HR_COMPOSE_FILE" | tr -d '${}:?' | sort -u)
    [[ ${#required[@]} -ge 10 ]]
    for var in "${required[@]}"; do
        grep -qE "^$var=.+" "$env" || { echo "missing $var" >&2; return 1; }
    done
    grep -qx "HR_ITEM=MOD-5" "$env"
    grep -qx "HR_SRC=$HR_ROOT/MOD-5/src" "$env"
    grep -qx "HR_HOST_REPO=$HR_HOST_REPO" "$env"
    grep -qx "HR_HOME=$HOME" "$env"
    grep -qx "HR_UID=$(id -u)" "$env"
    grep -qx "HR_GID=$(id -g)" "$env"
    grep -qx "HR_STATE_HOST=$HR_STATE" "$env"
    grep -qx "HR_CLAUDE_JSON=$HR_ROOT/MOD-5/claude.json" "$env"
    grep -qx "HR_GIT_NAME=hr fixture" "$env"
    grep -qx "HR_GIT_EMAIL=hr-fixture@example.invalid" "$env"
    grep -qx "HR_CPUS=6" "$env"
    grep -qx "HR_CARGO_EMPTY=$HR_ROOT/MOD-5/cargo-empty" "$env"
    # The file bound over the shared cargo volume's config/credentials: empty, read-only.
    [[ -f "$HR_ROOT/MOD-5/cargo-empty" && ! -s "$HR_ROOT/MOD-5/cargo-empty" ]]
    [[ "$(stat -c %a "$HR_ROOT/MOD-5/cargo-empty")" == 444 ]]
    grep -qx "HR_MISE_PATH=/usr/bin" "$env"
    [[ "$(reg_get MOD-5 cpus)" == 6 ]]
}

@test "3c. HR_MISE_PATH is resolved from the mise installs: claude, uv, ripgrep; no trailing slash" {
    unset HR_MISE_PATH
    local m="$HOME/.local/share/mise/installs"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 2 ]]
    [[ "$stderr" == *claude* ]]
    [[ ! -e "$(reg_of MOD-5)" ]]

    mkdir -p "$m/claude/latest" "$m/uv/latest/.mise-bins" "$m/ripgrep/latest/ripgrep-15.2.0-x86_64-unknown-linux-musl"
    printf '#!/bin/sh\n' >"$m/claude/latest/claude"
    chmod +x "$m/claude/latest/claude"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 2 ]]
    [[ "$stderr" == *uv* ]]

    printf '#!/bin/sh\n' >"$m/uv/latest/.mise-bins/uv"
    chmod +x "$m/uv/latest/.mise-bins/uv"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    grep -qx "HR_MISE_PATH=$m/claude/latest:$m/uv/latest/.mise-bins:$m/ripgrep/latest/ripgrep-15.2.0-x86_64-unknown-linux-musl" \
        "$HR_TEST_DOCKER_LOG.env"
}

@test "3d. up preflights the ~/.local tool dirs it mounts (never ~/.local itself)" {
    local d
    for d in .local/bin .local/share/mise/installs .local/share/uv/python .local/share/uv/tools; do
        mv "$HOME/$d" "$HOME/$d.away"
        run --separate-stderr "$HR" up MOD-5
        mv "$HOME/$d.away" "$HOME/$d"
        [[ $status -eq 2 ]]
        [[ "$stderr" == *"missing $HOME/$d"* ]]
        [[ ! -e "$(reg_of MOD-5)" && ! -e "$HR_ROOT/MOD-5" ]]
    done
    # The compose file mounts exactly those four, read-only, and nothing else under ~/.local.
    run grep -cE 'source: "\$\{HR_HOME:\?\}/\.local' "$HR_COMPOSE_FILE"
    [[ "$output" == 4 ]]
    run grep -E 'source: "\$\{HR_HOME:\?\}/\.local' "$HR_COMPOSE_FILE"
    local line
    for line in "${lines[@]}"; do [[ "$line" == *'read_only: true'* ]]; done
}

@test "4. up warns on uncommitted workflow files; the clone takes the committed ref" {
    printf 'local edit\n' >>"$HR_HOST_REPO/HANDOFF.md"
    mkdir -p "$HR_HOST_REPO/.claude"
    printf 'x\n' >"$HR_HOST_REPO/.claude/x"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    [[ "$stderr" == *'HANDOFF.md'* ]]
    [[ "$stderr" == *'.claude/x'* ]]
    [[ "$stderr" == *'committed main only'* ]]
    [[ "$stderr" != *'settings.local.json'* ]]
    [[ "$(cat "$(src_of MOD-5)/HANDOFF.md")" == "$(git -C "$HR_HOST_REPO" show main:HANDOFF.md)" ]]
    [[ ! -e "$(src_of MOD-5)/.claude/x" ]]
}

@test "5. duplicate up -> 1, no second clone, no compose call" {
    mk_run MOD-5
    local tip
    tip="$(git -C "$(src_of MOD-5)" rev-parse HEAD)"
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'run exists'* ]]
    [[ "$(git -C "$(src_of MOD-5)" rev-parse HEAD)" == "$tip" ]]
    ! grep -q '^compose' "$HR_TEST_DOCKER_LOG" || false
    run --separate-stderr "$HR" up MOD-5 --ssd
    [[ $status -eq 1 ]]
    [[ ! -e "$HR_SSD_ROOT/MOD-5" ]]

    # A leftover run dir without a registry entry is refused too.
    mkdir -p "$HR_ROOT/MOD-4"
    run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'stale run dir'* ]]
    [[ ! -e "$(reg_of MOD-4)" ]]
}

@test "6. --ssd places the run under HR_SSD_ROOT; --from takes that ref's sha" {
    git -C "$HR_HOST_REPO" switch -q -c feature
    host_commit code.txt 'feature work'
    git -C "$HR_HOST_REPO" switch -q main
    local fsha
    fsha="$(git -C "$HR_HOST_REPO" rev-parse feature)"

    run --separate-stderr "$HR" up MOD-5 --ssd --from feature
    [[ $status -eq 0 ]]
    [[ -d "$(src_of MOD-5 "$HR_SSD_ROOT")/.git" ]]
    [[ ! -e "$HR_ROOT/MOD-5" ]]
    [[ "$(reg_get MOD-5 root_kind)" == ssd ]]
    [[ "$(reg_get MOD-5 run_dir)" == "$HR_SSD_ROOT/MOD-5" ]]
    [[ "$(reg_get MOD-5 base_ref)" == feature ]]
    [[ "$(reg_get MOD-5 base_sha)" == "$fsha" ]]
    [[ "$(git -C "$(src_of MOD-5 "$HR_SSD_ROOT")" rev-parse hr/MOD-5)" == "$fsha" ]]

    run --separate-stderr "$HR" up MOD-4 --from no-such-ref
    [[ $status -eq 2 ]]
    run --separate-stderr "$HR" up MOD-4 --cpus lots
    [[ $status -eq 2 ]]
}

@test "7. up: missing image -> 2 naming scripts/hr build; label mismatch -> 2 rebuild" {
    HR_TEST_DOCKER_IMAGE=missing run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'scripts/hr build'* ]]
    [[ ! -e "$HR_ROOT/MOD-5" ]]

    HR_TEST_DOCKER_IMAGE=mismatch run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'rebuild'* ]]
    [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
}

@test "7b. compose up failure -> 3 with the purge hint; the registry stays for down --purge --force" {
    HR_TEST_DOCKER_FAIL_UP=1 run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 3 ]]
    [[ "$stderr" == *'scripts/hr down MOD-5 --purge --force'* ]]
    [[ -f "$(reg_of MOD-5)" ]]
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 0 ]]
    [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
}

@test "7c. build passes the host identity as build args, then creates the cargo volume" {
    run --separate-stderr "$HR" build
    [[ $status -eq 0 ]]
    local line
    line="$(grep '^build ' "$HR_TEST_DOCKER_LOG")"
    [[ "$line" == *"-t htui-hr-dev -f $HR_TEST_REPO_ROOT/docker/hr/Dockerfile"* ]]
    [[ "$line" == *"--build-arg HR_USER=$(id -un)"* ]]
    [[ "$line" == *"--build-arg HR_UID=$(id -u)"* ]]
    [[ "$line" == *"--build-arg HR_GID=$(id -g)"* ]]
    [[ "$line" == *"--build-arg HR_HOME=$HOME"* ]]
    [[ "$line" == *" $HR_TEST_REPO_ROOT/docker/hr" ]]
    [[ "$(tail -n 1 "$HR_TEST_DOCKER_LOG")" == 'volume create htui-hr-cargo' ]]
}

@test "7d. attach: starts a stopped run, then execs claude (or a shell) in dev" {
    mk_run MOD-5
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" attach MOD-5 </dev/null
    [[ $status -eq 0 ]]
    grep -qxF "compose -p hr-mod-5 -f $HR_COMPOSE_FILE up -d --wait" "$HR_TEST_DOCKER_LOG"
    # The shared cargo volume is (re)created first: `docker volume prune --all` may have dropped it.
    [[ "$(grep -v ' ps ' "$HR_TEST_DOCKER_LOG" | head -n 1)" == 'volume create htui-hr-cargo' ]]
    grep -qE -- "^compose -p hr-mod-5 -f .* exec (-T )?-e TERM -e COLORTERM dev claude --dangerously-skip-permissions$" \
        "$HR_TEST_DOCKER_LOG"

    : >"$HR_TEST_DOCKER_LOG"
    HR_TEST_DOCKER_RUNNING=hr-mod-5 run --separate-stderr "$HR" attach MOD-5 --shell </dev/null
    [[ $status -eq 0 ]]
    ! grep -q ' up ' "$HR_TEST_DOCKER_LOG" || false
    grep -qE -- " exec (-T )?-e TERM -e COLORTERM dev bash -l$" "$HR_TEST_DOCKER_LOG"

    run --separate-stderr "$HR" attach MOD-4
    [[ $status -eq 1 ]]
    run --separate-stderr "$HR" attach
    [[ $status -eq 2 ]]
}

# ---------------------------------------------------------------------------------------------
# collect

@test "8. collect is fetch-only: host ref = clone tip, nothing else on the host changes" {
    mk_run MOD-5
    local before tip
    before="$(host_snapshot)"
    tip="$(git -C "$(src_of MOD-5)" rev-parse hr/MOD-5)"
    run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 0 ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse refs/heads/hr/MOD-5)" == "$tip" ]]
    [[ "$(host_snapshot | grep -v 'refs/heads/hr/MOD-5 ')" == "$before" ]]
    [[ ! -e "$HR_HOST_REPO/.git/FETCH_HEAD" ]]
    [[ "$output" == *'work on MOD-5'* ]]
    [[ "$(reg_get MOD-5 collected_sha)" == "$tip" ]]
    [[ -n "$(reg_get MOD-5 collected_at)" ]]
}

@test "9. a second collect after more sandbox commits fast-forwards" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    src_commit MOD-5 notes/more.txt 'more' 'more work'
    local tip
    tip="$(git -C "$(src_of MOD-5)" rev-parse hr/MOD-5)"
    run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 0 ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" == "$tip" ]]
    [[ "$output" == *'more work'* ]]
}

@test "10. rewritten sandbox history -> 1, host ref unchanged; --force overwrites" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    local old
    old="$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)"
    git -C "$(src_of MOD-5)" -c user.name=s -c user.email=s@x.invalid commit -q --amend -m 'rewritten'
    run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'rewritten'*'--force'* ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" == "$old" ]]

    run --separate-stderr "$HR" collect MOD-5 --force
    [[ $status -eq 0 ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" == "$(git -C "$(src_of MOD-5)" rev-parse hr/MOD-5)" ]]
}

@test "11. hr/<ITEM> checked out in a host worktree -> 1" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    git -C "$HR_HOST_REPO" worktree add -q "$BATS_TEST_TMPDIR/wt" hr/MOD-5
    src_commit MOD-5 notes/more.txt 'more'
    local old
    old="$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)"
    run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'worktree'* ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" == "$old" ]]
}

@test "12. collect --merge with dirty tracked files -> 1, no merge, HEAD unchanged" {
    mk_run MOD-5
    printf 'dirty\n' >>"$HR_HOST_REPO/DECISIONS.md"
    local head
    head="$(git -C "$HR_HOST_REPO" rev-parse HEAD)"
    run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'DECISIONS.md'* ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse HEAD)" == "$head" ]]
    ! git -C "$HR_HOST_REPO" rev-parse -q --verify MERGE_HEAD || false
    # Refused before anything was written: not even the fetch.
    ! git -C "$HR_HOST_REPO" rev-parse -q --verify refs/heads/hr/MOD-5 || false
}

@test "13. collect --merge on a clean tree: 2-parent merge into the current branch, validator green" {
    mk_run MOD-5
    host_commit other.txt 'host moved on'
    local head
    head="$(git -C "$HR_HOST_REPO" rev-parse HEAD)"
    # L4: the pre-merge validator's temp dir is gone afterwards, and the exit is clean.
    export TMPDIR="$BATS_TEST_TMPDIR/tmpdir"
    mkdir -p "$TMPDIR"
    run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 0 ]]
    [[ "$stderr" != *unbound* && -z "$(ls -A "$TMPDIR")" ]]
    # Nothing under .claude/scripts/docker/...: no confirmation needed without a terminal.
    [[ "$output$stderr" != *'needs confirmation'* ]]
    [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect)" ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-list --parents -n 1 HEAD | wc -w)" -eq 3 ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse HEAD^1)" == "$head" ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse HEAD^2)" == "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" ]]
    [[ "$(git -C "$HR_HOST_REPO" log -1 --format=%s)" == 'Merge hr/MOD-5 (sandbox run)' ]]
    [[ -f "$HR_HOST_REPO/notes/MOD-5.txt" ]]
    [[ "$output$stderr" == *'0 error(s)'* ]]
}

@test "14. collect --merge, HANDOFF-only conflict, non-interactive -> 1, recipe, MERGE_HEAD, no claude" {
    mk_run MOD-5
    local src
    src="$(src_of MOD-5)"
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** sandbox body/' "$src/HANDOFF.md"
    git -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q -am 'sandbox edits MOD-4'
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** host body/' "$HR_HOST_REPO/HANDOFF.md"
    git -C "$HR_HOST_REPO" commit -q -am 'host edits MOD-4'
    # A claude on PATH that would leave a trace if launched.
    mkdir -p "$BATS_TEST_TMPDIR/fakebin"
    printf '#!/bin/sh\ntouch "%s/claude-launched"\n' "$BATS_TEST_TMPDIR" >"$BATS_TEST_TMPDIR/fakebin/claude"
    chmod +x "$BATS_TEST_TMPDIR/fakebin/claude"

    PATH="$BATS_TEST_TMPDIR/fakebin:$PATH" run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 1 ]]
    [[ "$output$stderr" == *'HANDOFF.md'* ]]
    [[ "$output$stderr" == *'keep every new checklist line from both sides'* ]]
    [[ "$output$stderr" == *'recount the summary table'* ]]
    git -C "$HR_HOST_REPO" rev-parse -q --verify MERGE_HEAD
    [[ ! -e "$BATS_TEST_TMPDIR/claude-launched" ]]
}

@test "15. collect --merge with a code conflict -> 1, resolve by hand" {
    mk_run MOD-5
    src_commit MOD-5 code.txt 'sandbox version'
    host_commit code.txt 'host version'
    run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 1 ]]
    [[ "$output$stderr" == *'code.txt'* ]]
    [[ "$output$stderr" == *'resolve by hand'* ]]
    [[ "$output$stderr" != *'recount the summary table'* ]]
    git -C "$HR_HOST_REPO" rev-parse -q --verify MERGE_HEAD
}

# ---------------------------------------------------------------------------------------------
# down / ls / gc

@test "16. down stops the project without -v; the run dir and registry stay" {
    mk_run MOD-5
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-5
    [[ $status -eq 0 ]]
    grep -qxF "compose -p hr-mod-5 -f $HR_COMPOSE_FILE down" "$HR_TEST_DOCKER_LOG"
    ! grep -q -- ' -v' "$HR_TEST_DOCKER_LOG" || false
    [[ -d "$(src_of MOD-5)/.git" && -f "$(reg_of MOD-5)" ]]
}

@test "17. down --purge refuses an uncollected run and a dirty clone" {
    mk_run MOD-5
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'not collected'* ]]
    [[ -d "$(src_of MOD-5)/.git" && -f "$(reg_of MOD-5)" ]]
    ! grep -q 'down' "$HR_TEST_DOCKER_LOG" || false

    "$HR" collect MOD-5 >/dev/null 2>&1
    printf 'dirty\n' >>"$(src_of MOD-5)/HANDOFF.md"
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'HANDOFF.md'* ]]
    [[ -d "$(src_of MOD-5)/.git" ]]

    # A branch made in the sandbox that the host has never seen blocks too.
    git -C "$(src_of MOD-5)" checkout -q -- HANDOFF.md
    git -C "$(src_of MOD-5)" switch -q -c side
    src_commit MOD-5 side.txt 'side work'
    git -C "$(src_of MOD-5)" switch -q hr/MOD-5
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'side'* ]]
    [[ -d "$(src_of MOD-5)/.git" ]]
}

@test "18. down --purge without --yes, non-interactive -> 1 needs confirmation" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    run --separate-stderr "$HR" down MOD-5 --purge
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'needs confirmation'* ]]
    [[ -d "$(src_of MOD-5)/.git" && -f "$(reg_of MOD-5)" ]]
    ! grep -q 'down' "$HR_TEST_DOCKER_LOG" || false
}

@test "19. down --purge --yes after collect: down -v, run dir + registry gone, leases untouched" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    seed_lease MOD-7 hr/MOD-5 "$(ts_ago '1 hour ago')" 'leased in the run'
    cp "$(lease_file)" "$BATS_TEST_TMPDIR/leases.before"
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 0 ]]
    grep -qxF "compose -p hr-mod-5 -f $HR_COMPOSE_FILE down -v" "$HR_TEST_DOCKER_LOG"
    ! grep -q 'volume rm' "$HR_TEST_DOCKER_LOG" || false
    [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
    [[ -d "$HR_ROOT" ]]
    cmp "$(lease_file)" "$BATS_TEST_TMPDIR/leases.before"
    # The host keeps the collected branch.
    git -C "$HR_HOST_REPO" rev-parse -q --verify refs/heads/hr/MOD-5
}

@test "20. down --purge --force --yes on an uncollected run: purged, its leases pruned but the floor" {
    mk_run MOD-5
    local ts
    ts="$(ts_ago '1 hour ago')"
    seed_lease MOD-7 hr/MOD-5 "$ts" 'leased, never merged'
    seed_lease MOD-8 hr/other "$ts" 'another run'
    seed_lease MOD-9 hr/MOD-5 "$ts" 'leased, per-prefix max'
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 0 ]]
    [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-8 MOD-9 ' ]]
}

@test "21. ls: header plus one row per run, AHEAD = sandbox commits over host main" {
    mk_run MOD-5
    src_commit MOD-5 notes/two.txt 'second'
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 ]]
    [[ "${lines[0]}" =~ ^ITEM\ +STATE\ +BRANCH\ +AHEAD\ +DISK\ +ROOT$ ]]
    [[ "${#lines[@]}" -eq 2 ]]
    local -a f
    read -ra f <<<"${lines[1]}"
    [[ "${f[0]}" == MOD-5 ]]
    [[ "${f[1]}" == down ]]
    [[ "${f[2]}" == hr/MOD-5 ]]
    [[ "${f[3]}" == 2 ]]
    [[ "${f[5]}" == hdd ]]

    "$HR" collect MOD-5 >/dev/null 2>&1
    HR_TEST_DOCKER_RUNNING=hr-mod-5 run --separate-stderr "$HR" ls
    read -ra f <<<"${lines[1]}"
    [[ "${f[1]}" == running+collected ]]
}

@test "22. gc --yes purges stopped + collected + clean runs, keeps the rest, prunes leases on main" {
    mk_run MOD-4
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    local ts
    ts="$(ts_ago '1 hour ago')"
    seed_lease MOD-5 host "$ts" 'open on main'
    seed_lease MOD-7 hr/MOD-4 "$ts" 'still in flight'
    run --separate-stderr "$HR" gc
    [[ $status -eq 1 ]]
    [[ "$output$stderr" == *MOD-5* ]]
    [[ "$stderr" == *'needs confirmation'* ]]
    [[ -f "$(reg_of MOD-5)" ]]

    run --separate-stderr "$HR" gc --yes
    [[ $status -eq 0 ]]
    [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
    [[ -d "$(src_of MOD-4)/.git" && -f "$(reg_of MOD-4)" ]]
    [[ "$(lease_ids | tr '\n' ' ')" == 'MOD-7 ' ]]
}

@test "22b. gc skips a collected run whose dev container is running" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    HR_TEST_DOCKER_RUNNING=hr-mod-5 run --separate-stderr "$HR" gc --yes
    [[ $status -eq 0 ]]
    [[ -f "$(reg_of MOD-5)" && -d "$(src_of MOD-5)/.git" ]]
}

@test "23. interactive without gum: up with no ID -> 2 naming gum" {
    HR_INTERACTIVE=1 HR_NO_GUM=1 run --separate-stderr "$HR" up
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'gum'* ]]
    HR_INTERACTIVE=1 HR_NO_GUM=1 run --separate-stderr "$HR"
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'gum'* ]]
    # Non-interactive, the same call is plain usage.
    run --separate-stderr "$HR" up
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'Usage'* ]]
}

# ---------------------------------------------------------------------------------------------
# registry (review H4) and lease guard

@test "24. the registry lives in the host-only HR_RUNS; HR_RUNS inside HR_STATE -> 2" {
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    [[ -f "$HR_RUNS/MOD-5" && -f "$HR_RUNS/.lock" ]]
    # Nothing of the registry in the state dir every sandbox mounts rw.
    [[ ! -e "$HR_STATE/runs" && ! -e "$HR_STATE/runs.lock" ]]
    ! grep -v '^HR_RUNS=' "$HR_TEST_DOCKER_LOG.env" | grep -qF "$HR_RUNS" || false

    HR_RUNS="$HR_STATE/runs" run --separate-stderr "$HR" ls
    [[ $status -eq 2 ]]
    [[ "$stderr" == *'HR_RUNS'*'HR_STATE'* ]]
    HR_RUNS="$HR_STATE" run --separate-stderr "$HR" ls
    [[ $status -eq 2 ]]
}

@test "25. a tampered registry -> 3 for every verb; nothing started, fetched, stopped or deleted" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    local reg good="$BATS_TEST_TMPDIR/reg.good" canary="$BATS_TEST_TMPDIR/canary"
    reg="$(reg_of MOD-5)"
    cp "$reg" "$good"
    mkdir -p "$canary"
    : >"$canary/keep"
    local -a cases=(
        "item|MOD-4" "project|hr-mod-4" "project|htui" "branch|main" "branch|hr/MOD-4"
        "run_dir|$HR_HOST_REPO" "run_dir|$HR_ROOT/MOD-4" "run_dir|$canary" "run_dir|$HR_ROOT/x/../../MOD-5"
        "run_dir|$HR_SSD_ROOT/MOD-5" "run_dir|" "root_kind|ssd" "root_kind|nvme" "root_kind|"
        "cpus|4;touch x" "cpus|" "cpus|0" "base_sha|abc" "base_sha|$(printf 'g%.0s' {1..40})"
        "base_ref|" "created_at|yesterday" "collected_sha|zzz" "collected_at|now"
    )
    local c verb
    local -a verbs=("attach MOD-5" "down MOD-5 --purge --force --yes" "down MOD-5" "collect MOD-5" "ls" "gc --yes")
    check_refused() { # $1=case label
        local v
        for v in "${verbs[@]}"; do
            : >"$HR_TEST_DOCKER_LOG"
            # shellcheck disable=SC2086
            run --separate-stderr "$HR" $v </dev/null
            [[ $status -eq 3 && "$stderr" == *"registry $reg is invalid"* ]] \
                || { echo "case '$1', hr $v: status $status: $stderr" >&2; return 1; }
            ! grep -q . "$HR_TEST_DOCKER_LOG" || { echo "case '$1', hr $v called docker" >&2; return 1; }
        done
        [[ -d "$(src_of MOD-5)/.git" && -f "$canary/keep" && -d "$HR_HOST_REPO/.git" && -f "$reg" ]]
    }
    for c in "${cases[@]}"; do
        cp "$good" "$reg"
        reg_set MOD-5 "${c%%|*}" "${c#*|}"
        check_refused "$c"
    done
    # Structure: missing / duplicate / unknown key, control characters, no header, a symlink.
    cp "$good" "$reg"; sed -i '/^cpus=/d' "$reg";                      check_refused 'missing cpus'
    cp "$good" "$reg"; printf 'cpus=8\n' >>"$reg";                     check_refused 'duplicate cpus'
    cp "$good" "$reg"; printf 'mount=/\n' >>"$reg";                    check_refused 'unknown key'
    cp "$good" "$reg"; reg_set MOD-5 base_ref $'main\e]0;x\a';         check_refused 'control characters'
    cp "$good" "$reg"; sed -i 1d "$reg";                               check_refused 'no header'
    cp "$good" "$BATS_TEST_TMPDIR/reg.link"; rm "$reg"; ln -s "$BATS_TEST_TMPDIR/reg.link" "$reg"
    check_refused 'symlink'

    # The untampered file still works.
    rm "$reg"; cp "$good" "$reg"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 && "$output" == *MOD-5* ]]
}

@test "26. a registry in the old place (\$HR_STATE/runs) moves to HR_RUNS once, then is validated" {
    mk_run MOD-5
    # That up already recorded this state dir as migrated (there was nothing to move). Act as if
    # an hr from before the registry move had made the run.
    grep -qxF "$HR_STATE" "$HR_RUNS/.migrated"
    local good="$BATS_TEST_TMPDIR/reg.good"
    cp "$(reg_of MOD-5)" "$good"
    old_registry() { # the registry back in $HR_STATE/runs, no migration record
        rm -f "$HR_RUNS/.migrated"
        mkdir -p "$HR_STATE/runs"
        mv "$(reg_of MOD-5)" "$HR_STATE/runs/MOD-5"
        : >"$HR_STATE/runs.lock"
    }
    old_registry
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 ]]
    [[ "$stderr" == *"moved 1 registry entries from $HR_STATE/runs"* ]]
    [[ "$output" == *MOD-5* ]]
    [[ -f "$(reg_of MOD-5)" && ! -e "$HR_STATE/runs" && ! -e "$HR_STATE/runs.lock" ]]
    grep -qxF "$HR_STATE" "$HR_RUNS/.migrated"

    # A sandbox-edited entry is moved by that one migration, then refused like any other.
    old_registry
    sed -i 's/^project=.*/project=htui/' "$HR_STATE/runs/MOD-5"
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 3 ]]
    [[ "$stderr" == *'project'* ]]
    [[ -d "$(src_of MOD-5)/.git" ]]
    ! grep -q '^compose' "$HR_TEST_DOCKER_LOG" || false
    cp "$good" "$(reg_of MOD-5)"

    # In both places before the migration -> refused, neither touched, nothing recorded.
    rm -f "$HR_RUNS/.migrated"
    mkdir -p "$HR_STATE/runs"
    cp "$good" "$HR_STATE/runs/MOD-5"
    run --separate-stderr "$HR" ls
    [[ $status -eq 3 && "$stderr" == *'both'* ]]
    [[ -f "$HR_STATE/runs/MOD-5" && -f "$(reg_of MOD-5)" && ! -e "$HR_RUNS/.migrated" ]]
    # The old place as a symlink before the migration: nothing to import, a warning, recorded.
    rm -rf "$HR_STATE/runs"
    ln -s "$HR_RUNS" "$HR_STATE/runs"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 && "$stderr" == *'not a directory'* && "$output" == *MOD-5* ]]
    grep -qxF "$HR_STATE" "$HR_RUNS/.migrated"
}

@test "27. the lease file vanished after hr-mint --init -> up, gc, down --purge refuse (1); never recreated" {
    mk_run MOD-5
    grep -qxF "$(realpath -m "$HR_STATE")" "$HR_RUNS/.leases-initialized"
    rm -f "$(lease_file)" "$HR_STATE/id-leases.lock"
    run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'vanished'*'hr-mint --init --force'* ]]
    [[ ! -e "$HR_ROOT/MOD-4" && ! -e "$(reg_of MOD-4)" && ! -e "$(lease_file)" ]]
    run --separate-stderr "$HR" gc --yes
    [[ $status -eq 1 && "$stderr" == *'vanished'* ]]
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 1 && "$stderr" == *'vanished'* ]]
    [[ -d "$(src_of MOD-5)/.git" && ! -e "$(lease_file)" ]]
    ! grep -q -- 'down -v' "$HR_TEST_DOCKER_LOG" || false
    # A different state dir has its own record: the guard is per HR_STATE. This one is spelled
    # through a symlink; the record holds it with symlinks resolved, and is matched that way.
    mkdir -p "$BATS_TEST_TMPDIR/state2"
    ln -s "$BATS_TEST_TMPDIR/state2" "$BATS_TEST_TMPDIR/state-link"
    HR_STATE="$BATS_TEST_TMPDIR/state-link" run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 0 ]]
    grep -qxF "$(realpath -m "$BATS_TEST_TMPDIR/state2")" "$HR_RUNS/.leases-initialized"
    ! grep -qF 'state-link' "$HR_RUNS/.leases-initialized" || false
    rm -f "$BATS_TEST_TMPDIR/state2/id-leases.tsv" "$BATS_TEST_TMPDIR/state2/id-leases.lock"
    # Either spelling is guarded, by scripts/hr and by hr-mint.
    HR_STATE="$BATS_TEST_TMPDIR/state2" run --separate-stderr "$HR" gc --yes
    [[ $status -eq 1 && "$stderr" == *'vanished'* ]]
    HR_STATE="$BATS_TEST_TMPDIR/state-link" run --separate-stderr "$HR" gc --yes
    [[ $status -eq 1 && "$stderr" == *'vanished'* ]]
    HR_STATE="$BATS_TEST_TMPDIR/state-link" run --separate-stderr "$HR_MINT" --prefix MOD --title x \
        --repo-root "$HR_HOST_REPO"
    [[ $status -eq 1 && "$stderr" == *'lease file vanished'* ]]
}

@test "28. up warns once per up that a non-default HR_STATE must be exported for host mints" {
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    # One line per `up`, not one per step that touches the state dir; other verbs never warn.
    [[ "$(grep -c 'HR_STATE is non-default' <<<"$stderr")" -eq 1 ]]
    [[ "$stderr" == *"export HR_STATE=$HR_STATE"* ]]
    run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 0 && "$(grep -c 'HR_STATE is non-default' <<<"$stderr")" -eq 1 ]]
    run --separate-stderr "$HR" ls
    [[ "$stderr" != *'HR_STATE is non-default'* ]]
}

# ---------------------------------------------------------------------------------------------
# purge checks (review H3, M1, M2)

# purge_state ITEM — "clean" when the run passes every purge check (gc lists it as purgeable),
# else the blocker text (stderr of a refused down --purge). Never purges: no --yes.
purge_state() {
    run --separate-stderr "$HR" gc
    if [[ "$output" == *"  $1"* ]]; then
        echo clean
        return 0
    fi
    run --separate-stderr "$HR" down "$1" --purge
    [[ $status -eq 1 && "$stderr" == *'refusing to purge'* ]] || { echo "unexpected: $status $stderr" >&2; return 1; }
    printf '%s\n' "$stderr"
}

@test "29. the purge status check runs in a throwaway container (no network, no caps, clone ro); failure blocks" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    : >"$HR_TEST_DOCKER_LOG"
    [[ "$(purge_state MOD-5)" == clean ]]
    local line
    line="$(grep '^run ' "$HR_TEST_DOCKER_LOG" | head -n 1)"
    [[ "$line" == 'run --rm --pull never --name hr-check-mod-5-'* ]]
    [[ "$line" == *' --network none --cap-drop ALL --security-opt no-new-privileges '* ]]
    [[ "$line" == *" -u $(id -u):$(id -g) -v $(src_of MOD-5):$HR_HOST_REPO:ro -w $HR_HOST_REPO --entrypoint bash htui-hr-dev -c "* ]]

    # Fail closed: a check that cannot run, or reports anything, blocks.
    local out
    out="$(HR_TEST_DOCKER_RUN=fail purge_state MOD-5)"
    [[ "$out" == *'status check failed'* ]]
    out="$(HR_TEST_DOCKER_RUN=dirty purge_state MOD-5)"
    [[ "$out" == *'canned.txt'* ]]
    HR_TEST_DOCKER_RUN=fail run --separate-stderr "$HR" gc --yes
    [[ $status -eq 0 && -d "$(src_of MOD-5)/.git" && -f "$(reg_of MOD-5)" ]]
    HR_TEST_DOCKER_RUN=fail run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 1 && -d "$(src_of MOD-5)/.git" ]]
}

@test "30. purge blockers: untracked files, linked worktrees, stash, detached HEADs; ignored files do not block" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    local src out
    src="$(src_of MOD-5)"
    gs() { git -C "$src" -c user.name=s -c user.email=s@x.invalid "$@"; }

    # Ignored (target/) is fine; untracked non-ignored is not.
    printf '/target/\n' >>"$src/.git/info/exclude"
    mkdir -p "$src/target" && : >"$src/target/big"
    [[ "$(purge_state MOD-5)" == clean ]]
    : >"$src/stray.txt"
    [[ "$(purge_state MOD-5)" == *'stray.txt'* ]]
    rm "$src/stray.txt"

    # An in-run linked worktree: checked on its own, not reported by the main one.
    gs worktree add -q "$src/.claude/worktrees/w1" -b w1
    [[ "$(purge_state MOD-5)" == clean ]]
    printf 'wip\n' >>"$src/.claude/worktrees/w1/HANDOFF.md"
    out="$(purge_state MOD-5)"
    [[ "$out" == *".claude/worktrees/w1"*'HANDOFF.md'* ]]
    gs -C "$src/.claude/worktrees/w1" checkout -q -- HANDOFF.md
    : >"$src/.claude/worktrees/w1/new.txt"
    [[ "$(purge_state MOD-5)" == *'new.txt'* ]]
    rm "$src/.claude/worktrees/w1/new.txt"
    [[ "$(purge_state MOD-5)" == clean ]]

    # A detached worktree HEAD with a commit the host never saw.
    gs worktree add -q --detach "$src/.claude/worktrees/w2"
    printf 'x\n' >"$src/.claude/worktrees/w2/det.txt"
    gs -C "$src/.claude/worktrees/w2" add det.txt
    gs -C "$src/.claude/worktrees/w2" commit -q -m detached
    [[ "$(purge_state MOD-5)" == *'detached worktree HEADs'*'w2'* ]]
    gs worktree remove --force "$src/.claude/worktrees/w2"
    [[ "$(purge_state MOD-5)" == clean ]]

    # A missing worktree dir, and one outside the clone, cannot be checked.
    gs worktree add -q --detach "$src/.claude/worktrees/w3"
    rm -rf "$src/.claude/worktrees/w3"
    [[ "$(purge_state MOD-5)" == *'w3 is missing'* ]]
    gs worktree prune
    gs worktree add -q --detach "$BATS_TEST_TMPDIR/outside"
    [[ "$(purge_state MOD-5)" == *'outside the clone'* ]]
    gs worktree remove --force "$BATS_TEST_TMPDIR/outside"

    # A stash.
    printf 'stash me\n' >>"$src/HANDOFF.md"
    gs stash -q
    [[ "$(purge_state MOD-5)" == *'refs/stash'* ]]
    gs stash drop -q
    [[ "$(purge_state MOD-5)" == clean ]]
}

@test "31. down --purge stops the project before the checks; a refused purge leaves it stopped, never down -v" {
    mk_run MOD-4
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-4 --purge --yes
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'not collected'* && "$stderr" == *'now stopped'* ]]
    grep -qxF "compose -p hr-mod-4 -f $HR_COMPOSE_FILE stop" "$HR_TEST_DOCKER_LOG"
    ! grep -q -- 'down' "$HR_TEST_DOCKER_LOG" || false

    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 0 ]]
    local stop check down
    stop="$(grep -n ' stop$' "$HR_TEST_DOCKER_LOG" | cut -d: -f1)"
    check="$(grep -n '^run ' "$HR_TEST_DOCKER_LOG" | cut -d: -f1)"
    down="$(grep -n ' down -v$' "$HR_TEST_DOCKER_LOG" | cut -d: -f1)"
    [[ -n "$stop" && -n "$check" && -n "$down" && "$stop" -lt "$check" && "$check" -lt "$down" ]]
}

@test "32. a hostile clone (filters, fsmonitor, hooks, pager, submodule config) never executes on the host" {
    mk_run MOD-4
    mk_run MOD-5
    local sub="$BATS_TEST_TMPDIR/subsrc" m="$BATS_TEST_TMPDIR/pwned" evil="$BATS_TEST_TMPDIR/evil"
    git init -q -b main "$sub"
    git -C "$sub" -c user.name=s -c user.email=s@x.invalid commit -q --allow-empty -m sub
    local s4 s5
    s4="$(src_of MOD-4)"
    s5="$(src_of MOD-5)"
    git -C "$s5" -c protocol.file.allow=always submodule add -q "$sub" sub
    git -C "$s5" -c user.name=s -c user.email=s@x.invalid commit -q -m 'add sub'
    "$HR" collect MOD-4 >/dev/null 2>&1
    "$HR" collect MOD-5 >/dev/null 2>&1
    src_commit MOD-5 notes/late.txt 'late' 'late sandbox work'   # fetched below, from the hostile clone

    # Everything a sandbox can write into a clone's .git that makes git execute something.
    printf '#!/bin/sh\ntouch "%s"\ncat\n' "$m" >"$evil"
    chmod +x "$evil"
    mkdir -p "$BATS_TEST_TMPDIR/hooks"
    local h gd
    for h in reference-transaction post-checkout post-merge pre-commit post-index-change pre-auto-gc \
        post-rewrite fsmonitor-watchman push-to-checkout; do
        ln -sf "$evil" "$BATS_TEST_TMPDIR/hooks/$h"
    done
    for gd in "$s4/.git" "$s5/.git" "$s5/.git/modules/sub"; do
        git config -f "$gd/config" filter.x.clean "$evil"
        git config -f "$gd/config" filter.x.smudge "$evil"
        git config -f "$gd/config" filter.x.required true
        git config -f "$gd/config" core.fsmonitor "$evil"
        git config -f "$gd/config" core.hooksPath "$BATS_TEST_TMPDIR/hooks"
        git config -f "$gd/config" core.pager "$evil"
        git config -f "$gd/config" pager.status "$evil"
        git config -f "$gd/config" diff.external "$evil"
        git config -f "$gd/config" uploadpack.packObjectsHook "$evil"
        git config -f "$gd/config" core.alternateRefsCommand "$evil"
        git config -f "$gd/config" core.sshCommand "$evil"
        git config -f "$gd/config" core.editor "$evil"
        git config -f "$gd/config" credential.helper "!$evil"
        git config -f "$gd/config" gpg.program "$evil"
        git config -f "$gd/config" status.showUntrackedFiles all
        mkdir -p "$gd/info"
        printf '* filter=x\n' >>"$gd/info/attributes"
    done
    # Stat-dirty tracked files: a status would have to run the clean filter to compare them.
    touch "$s4/HANDOFF.md" "$s5/HANDOFF.md"

    # Log every git the host runs.
    local real
    real="$(command -v git)"
    mkdir -p "$BATS_TEST_TMPDIR/gitshim"
    printf '#!/bin/bash\nprintf "%%s\\t%%s\\n" "$PWD" "$*" >>"%s"\nexec "%s" "$@"\n' \
        "$BATS_TEST_TMPDIR/git.log" "$real" >"$BATS_TEST_TMPDIR/gitshim/git"
    chmod +x "$BATS_TEST_TMPDIR/gitshim/git"
    : >"$BATS_TEST_TMPDIR/git.log"
    : >"$HR_TEST_DOCKER_LOG"

    local -a env=(PATH="$BATS_TEST_TMPDIR/gitshim:$PATH" HR_TEST_DOCKER_RUN=clean)
    run --separate-stderr env "${env[@]}" "$HR" ls
    [[ $status -eq 0 ]]
    run --separate-stderr env "${env[@]}" "$HR" collect MOD-5
    [[ $status -eq 0 && "$output" == *'late sandbox work'* ]]
    run --separate-stderr env "${env[@]}" "$HR" gc
    [[ $status -eq 1 && "$output" == *MOD-4* && "$output" == *MOD-5* ]]
    run --separate-stderr env "${env[@]}" "$HR" down MOD-5 --purge --yes
    [[ $status -eq 0 && ! -e "$s5" ]]
    run --separate-stderr env "${env[@]}" "$HR" gc --yes
    [[ $status -eq 0 && ! -e "$s4" ]]

    # Nothing ran on the host...
    [[ ! -e "$m" ]]
    # ... the worktree checks went to the container (gc x2 + down) ...
    [[ "$(grep -c '^run ' "$HR_TEST_DOCKER_LOG")" -ge 4 ]]
    # ... and every host git touching a clone was ref/object plumbing, or the host's own fetch.
    local cwd argv sub_cmd n=0 fetches=0 prev
    local -a a
    while IFS=$'\t' read -r cwd argv; do
        [[ "$cwd/" != "$HR_ROOT"/* ]] || { echo "git ran inside a clone: $cwd $argv" >&2; return 1; }
        [[ "$argv" == *"$HR_ROOT/"* ]] || continue
        read -ra a <<<"$argv"
        sub_cmd='' prev=''
        for h in "${a[@]}"; do
            if [[ "$prev" == -C || "$prev" == -c ]]; then prev=''; continue; fi
            case "$h" in
                -C | -c) prev="$h" ;;
                --no-pager | --git-dir=*) ;;
                *) sub_cmd="$h"; break ;;
            esac
        done
        case "$sub_cmd" in
            rev-parse | for-each-ref | cat-file | merge-base | rev-list) n=$((n + 1)) ;;
            worktree) [[ "$argv" == *' worktree list '* ]] || { echo "host git: $argv" >&2; return 1; } ;;
            fetch) [[ "$argv" == "-C $HR_HOST_REPO "* ]] || { echo "host git: $argv" >&2; return 1; }
                   fetches=$((fetches + 1)) ;;
            *) echo "host git on a clone: $argv" >&2; return 1 ;;
        esac
    done <"$BATS_TEST_TMPDIR/git.log"
    [[ $n -gt 0 && $fetches -eq 1 ]]
}

@test "33. the purge rm -rf guard: a run dir outside the roots, of another item, or a symlink is never deleted" {
    mk_run MOD-4
    mk_run MOD-5
    local reg good="$BATS_TEST_TMPDIR/reg.good" canary="$BATS_TEST_TMPDIR/canary"
    reg="$(reg_of MOD-5)"
    cp "$reg" "$good"
    mkdir -p "$canary" && : >"$canary/keep"

    # Outside the roots / another item's dir: refused before anything runs (registry validation).
    local rd
    for rd in "$canary" "$HR_ROOT/MOD-4" "$HR_ROOT/../canary"; do
        cp "$good" "$reg"
        reg_set MOD-5 run_dir "$rd"
        : >"$HR_TEST_DOCKER_LOG"
        run --separate-stderr "$HR" down MOD-5 --purge --force --yes
        [[ $status -eq 3 && "$stderr" == *'run_dir'* ]]
        [[ -f "$canary/keep" && -d "$(src_of MOD-4)/.git" && -d "$(src_of MOD-5)/.git" ]]
        ! grep -q . "$HR_TEST_DOCKER_LOG" || false
    done
    cp "$good" "$reg"

    # The run dir swapped for a symlink: refused, neither the link nor its target touched.
    mv "$HR_ROOT/MOD-5" "$BATS_TEST_TMPDIR/moved"
    ln -s "$canary" "$HR_ROOT/MOD-5"
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 1 && "$stderr" == *'symlink'* ]]
    [[ -L "$HR_ROOT/MOD-5" && -f "$canary/keep" && -f "$(reg_of MOD-5)" ]]
    ! grep -q -- 'down -v' "$HR_TEST_DOCKER_LOG" || false
    rm "$HR_ROOT/MOD-5"
    mv "$BATS_TEST_TMPDIR/moved" "$HR_ROOT/MOD-5"

    # Symlinks inside the clone are removed, not followed.
    ln -s "$canary" "$(src_of MOD-5)/evil"
    ln -s "$canary/keep" "$(src_of MOD-5)/evil-file"
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 0 && ! -e "$HR_ROOT/MOD-5" && -f "$canary/keep" && -d "$(src_of MOD-4)/.git" ]]
}

# hr_selfhost — commit this checkout's hr (+ what it sources and runs) into the fixture host repo,
# as on the real host, where scripts/hr runs from the repo it merges into. Prints that hr's path.
hr_selfhost() {
    local f
    for f in scripts/hr scripts/hr-mint docker/hr/compose.hr.yaml \
        .claude/skills/handoff-run/scripts/workflow-patterns.sh \
        .claude/skills/handoff-run/scripts/validate-workflow-docs.sh \
        .claude/skills/handoff-run/scripts/next-item-id.sh; do
        mkdir -p "$HR_HOST_REPO/${f%/*}"
        cp -p "$HR_TEST_REPO_ROOT/$f" "$HR_HOST_REPO/$f"
    done
    git -C "$HR_HOST_REPO" add -A scripts docker .claude/skills >/dev/null
    git -C "$HR_HOST_REPO" commit -q -m 'hr itself'
    printf '%s/scripts/hr' "$HR_HOST_REPO"
}

@test "34. collect --merge validates with the pre-merge validator, never the one the run branch brings" {
    local hrx
    hrx="$(hr_selfhost)"
    mk_run MOD-5
    local src wf=.claude/skills/handoff-run/scripts
    src="$(src_of MOD-5)"
    # The run branch rewrites the validator and the patterns file it sources.
    sed -i "2i touch '$BATS_TEST_TMPDIR/validator-ran'" "$src/$wf/validate-workflow-docs.sh"
    sed -i "1i touch '$BATS_TEST_TMPDIR/patterns-ran'" "$src/$wf/workflow-patterns.sh"
    git -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q -am 'sandbox rewrites the validator'
    export TMPDIR="$BATS_TEST_TMPDIR/tmpdir"
    mkdir -p "$TMPDIR"
    # The run changes .claude/: without a terminal that needs --yes (re-review L1).
    run --separate-stderr "$hrx" collect MOD-5 --merge --yes
    echo "$output$stderr"
    [[ $status -eq 0 ]]
    [[ "$stderr" != *unbound* && -z "$(ls -A "$TMPDIR")" ]]
    [[ "$output$stderr" == *'0 error(s)'* ]]
    # The merge landed (the hostile files are in the host tree now) but never ran.
    grep -q 'validator-ran' "$HR_HOST_REPO/$wf/validate-workflow-docs.sh"
    [[ ! -e "$BATS_TEST_TMPDIR/validator-ran" && ! -e "$BATS_TEST_TMPDIR/patterns-ran" ]]
}

@test "35. doc-conflict recipe session: when the run changed .claude/scripts/docker, show them and always ask" {
    mk_run MOD-4
    mk_run MOD-5
    local s4 s5
    s4="$(src_of MOD-4)"
    s5="$(src_of MOD-5)"
    # Both runs conflict with the host on HANDOFF.md; MOD-5 also brings a .claude/settings.json.
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** sandbox body/' "$s4/HANDOFF.md" "$s5/HANDOFF.md"
    git -C "$s4" -c user.name=s -c user.email=s@x.invalid commit -q -am 'sandbox edits MOD-4'
    printf '{"hooks": {"SessionStart": "evil"}}\n' >"$s5/.claude/settings.json"
    git -C "$s5" add .claude/settings.json
    git -C "$s5" -c user.name=s -c user.email=s@x.invalid commit -q -am 'sandbox edits MOD-4 and settings'
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** host body/' "$HR_HOST_REPO/HANDOFF.md"
    git -C "$HR_HOST_REPO" commit -q -am 'host edits MOD-4'

    local fb="$BATS_TEST_TMPDIR/fakebin"
    mkdir -p "$fb"
    printf '#!/bin/sh\ntouch "%s/claude-launched"\n' "$BATS_TEST_TMPDIR" >"$fb/claude"
    # Yes to collect's tool-config question (test 45 covers it); FAKE_GUM_CONFIRM for the session one.
    printf '#!/bin/sh\necho "$*" >>"%s/gum.log"\ncase "$1 $2" in "confirm Merge hr/"*) exit 0 ;; confirm*) exit "${FAKE_GUM_CONFIRM:-1}" ;; esac\nexit 0\n' \
        "$BATS_TEST_TMPDIR" >"$fb/gum"
    chmod +x "$fb/claude" "$fb/gum"
    local -a env=(PATH="$fb:$PATH" HR_INTERACTIVE=1)

    # MOD-5: --yes does not skip the question; the stat names the file; declined -> no claude.
    run --separate-stderr env "${env[@]}" "$HR" collect MOD-5 --merge --yes
    [[ $status -eq 1 ]]
    [[ "$output" == *'.claude/settings.json'* ]]
    grep -q '^confirm Open a host claude session' "$BATS_TEST_TMPDIR/gum.log"
    [[ ! -e "$BATS_TEST_TMPDIR/claude-launched" ]]
    git -C "$HR_HOST_REPO" merge --abort

    # MOD-4 (docs only): --yes still launches it without asking, as before.
    : >"$BATS_TEST_TMPDIR/gum.log"
    run --separate-stderr env "${env[@]}" "$HR" collect MOD-4 --merge --yes
    [[ $status -eq 1 ]]
    [[ "$output" != *'.claude/settings.json'* ]]
    ! grep -q '^confirm ' "$BATS_TEST_TMPDIR/gum.log" || false
    [[ -e "$BATS_TEST_TMPDIR/claude-launched" ]]
}

@test "36. a malformed ID -> 2 on every verb (never the picker, never an empty item)" {
    mk_run MOD-5
    local v
    for v in "collect FOO-1" "collect mod5" "down mod5" "down FOO-1 --purge --yes" "attach FOO-1"; do
        # shellcheck disable=SC2086
        run --separate-stderr "$HR" $v </dev/null
        [[ $status -eq 2 && "$stderr" == *'not an item ID'* ]] || { echo "hr $v: $status $stderr" >&2; return 1; }
    done
    # In a terminal, `up FOO-1` must not fall through to the picker (which would ask for gum).
    HR_INTERACTIVE=1 HR_NO_GUM=1 run --separate-stderr "$HR" up FOO-1
    [[ $status -eq 2 && "$stderr" == *'not an item ID: FOO-1'* && "$stderr" != *gum* ]]
    [[ "$stderr" == *'ANA|MOD|NEXT|VAL|TOOL|CLEAN'* ]]
}

@test "37. collected_sha is the host ref after the fetch, even if the sandbox committed meanwhile" {
    mk_run MOD-5
    local src real
    src="$(src_of MOD-5)"
    real="$(command -v git)"
    # A git that, on the host's fetch, first lets the "sandbox" commit once more.
    mkdir -p "$BATS_TEST_TMPDIR/gitshim"
    cat >"$BATS_TEST_TMPDIR/gitshim/git" <<EOF
#!/bin/bash
if [[ " \$* " == *" fetch "* && ! -e "$BATS_TEST_TMPDIR/raced" ]]; then
    : >"$BATS_TEST_TMPDIR/raced"
    printf 'race\n' >"$src/race.txt"
    "$real" -C "$src" add race.txt
    "$real" -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q -m race
fi
exec "$real" "\$@"
EOF
    chmod +x "$BATS_TEST_TMPDIR/gitshim/git"
    PATH="$BATS_TEST_TMPDIR/gitshim:$PATH" run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 0 && -e "$BATS_TEST_TMPDIR/raced" ]]
    [[ "$(reg_get MOD-5 collected_sha)" == "$(git -C "$HR_HOST_REPO" rev-parse refs/heads/hr/MOD-5)" ]]

    # A rewrite that lands between the check and the fetch is caught on what arrived: refused, the
    # host ref and the registry as they were, the temporary ref gone.
    local old reg
    old="$(git -C "$HR_HOST_REPO" rev-parse refs/heads/hr/MOD-5)"
    reg="$(cat "$(reg_of MOD-5)")"
    cat >"$BATS_TEST_TMPDIR/gitshim/git" <<EOF
#!/bin/bash
if [[ " \$* " == *" fetch "* && ! -e "$BATS_TEST_TMPDIR/amended" ]]; then
    : >"$BATS_TEST_TMPDIR/amended"
    "$real" -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q --amend -m amended
fi
exec "$real" "\$@"
EOF
    PATH="$BATS_TEST_TMPDIR/gitshim:$PATH" run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 1 && "$stderr" == *'rewritten'* && -e "$BATS_TEST_TMPDIR/amended" ]]
    [[ "$(git -C "$HR_HOST_REPO" rev-parse refs/heads/hr/MOD-5)" == "$old" && "$(cat "$(reg_of MOD-5)")" == "$reg" ]]
    [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect)" ]]
}

@test "38. Ctrl-C (INT) or TERM during up's clone removes the half-made run dir; no registry" {
    local real sig pid i
    real="$(command -v git)"
    mkdir -p "$BATS_TEST_TMPDIR/gitshim"
    printf '#!/bin/bash\nif [[ "$1" == clone ]]; then : >"%s/cloning"; sleep 30; fi\nexec "%s" "$@"\n' \
        "$BATS_TEST_TMPDIR" "$real" >"$BATS_TEST_TMPDIR/gitshim/git"
    chmod +x "$BATS_TEST_TMPDIR/gitshim/git"
    for sig in INT TERM; do
        rm -f "$BATS_TEST_TMPDIR/cloning"
        # Own process group (as a terminal's foreground job), INT not ignored (background jobs
        # of a non-interactive shell start with it ignored).
        PATH="$BATS_TEST_TMPDIR/gitshim:$PATH" setsid env --default-signal=INT "$HR" up MOD-5 \
            >"$BATS_TEST_TMPDIR/up.out" 2>&1 &
        pid=$!
        for i in $(seq 1 100); do [[ -e "$BATS_TEST_TMPDIR/cloning" ]] && break; sleep 0.1; done
        [[ -e "$BATS_TEST_TMPDIR/cloning" && -d "$HR_ROOT/MOD-5" ]]
        kill -s "$sig" -- "-$pid"
        wait "$pid" && return 1
        cat "$BATS_TEST_TMPDIR/up.out"
        [[ ! -e "$HR_ROOT/MOD-5" && ! -e "$(reg_of MOD-5)" ]]
        grep -q 'interrupted' "$BATS_TEST_TMPDIR/up.out"
    done
    # The lock was released: a plain up works.
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
}

@test "39. HANDOFF titles, sandbox commit subjects and conflict paths reach the terminal without control characters" {
    # A title with an OSC escape, in the up picker (a fake gum records what it is shown).
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing\x1b]0;pwned\x07.** body/' "$HR_HOST_REPO/HANDOFF.md"
    git -C "$HR_HOST_REPO" commit -q -am 'escape in a title'
    local fb="$BATS_TEST_TMPDIR/fakebin"
    mkdir -p "$fb"
    printf '#!/bin/bash\ncase "$1" in\n  filter) tee "%s/picker.in" | tail -n 1 ;;\n  spin) while [[ "$1" != -- ]]; do shift; done; shift; exec "$@" ;;\nesac\n' \
        "$BATS_TEST_TMPDIR" >"$fb/gum"
    chmod +x "$fb/gum"
    PATH="$fb:$PATH" HR_INTERACTIVE=1 run --separate-stderr "$HR" up --yes
    [[ $status -eq 0 ]]
    grep -q 'MOD-4  Thing' "$BATS_TEST_TMPDIR/picker.in"
    ! grep -q $'[\x01-\x08\x0b-\x1f\x7f]' "$BATS_TEST_TMPDIR/picker.in" || false

    # A sandbox commit subject with escapes, in collect's log.
    src_commit MOD-5 notes/esc.txt 'x' $'evil \e]0;pwned\a\e[2J subject'
    run --separate-stderr "$HR" collect MOD-5
    [[ $status -eq 0 && "$output" == *'evil ]0;pwned[2J subject'* ]]
    [[ "$output$stderr" != *$'\e'* && "$output$stderr" != *$'\a'* ]]
    run --separate-stderr "$HR" ls
    [[ "$output$stderr" != *$'\e'* ]]
}

# ---------------------------------------------------------------------------------------------
# re-review (TOOL-7): cargo volume, ~/.claude narrowing, registry migration, merge confirmation

@test "40. compose: ~/.cargo/bin is an empty read-only tmpfs; registry/src is a per-run volume" {
    local cfg
    cfg="$(compose_config)"
    # M-A: cargo runs cargo-<cmd> from CARGO_HOME/bin ahead of PATH, so it must be empty and read-only.
    [[ "$(jq -c '.services.dev.volumes[] | select(.target == "/HR_HOME/.cargo/bin") | [.type, .read_only]' <<<"$cfg")" \
        == '["tmpfs",true]' ]]
    # Sized, like every tmpfs: never the default half of the host's RAM.
    [[ "$(jq -c '.services.dev.volumes[] | select(.target == "/HR_HOME/.cargo/bin") | .tmpfs.size' <<<"$cfg")" \
        == '"1048576"' ]]
    [[ "$(jq '[.services[].volumes[]? | select(.type == "tmpfs" and (.tmpfs.size // null) == null)] | length' <<<"$cfg")" \
        -eq 0 ]]
    # Residual: extracted crate sources per run, in a volume `down -v` removes (not external).
    [[ "$(jq -c '.services.dev.volumes[] | select(.target == "/HR_HOME/.cargo/registry/src") | [.type, .source]' <<<"$cfg")" \
        == '["volume","cargo-src"]' ]]
    [[ "$(jq -c '.volumes["cargo-src"] | [.name, .external]' <<<"$cfg")" == '["hr-contract_cargo-src",null]' ]]
    # init chowns the mount points Docker creates root-owned.
    [[ "$(jq -c '.services.init.command' <<<"$cfg")" == *'"/HR_HOME/.cargo/registry","/HR_HOME/.cargo/registry/src"'* ]]
    [[ "$(jq -c '.services.init.volumes[] | select(.target == "/HR_HOME/.cargo/registry/src") | .source' <<<"$cfg")" \
        == '"cargo-src"' ]]
}

@test "40b. compose: the dev container shares the host's clock zone, read-only" {
    local cfg f
    cfg="$(compose_config)"
    for f in /etc/localtime; do
        [[ "$(jq -c --arg t "$f" '.services.dev.volumes[] | select(.target == $t) | [.type, .source, .read_only]' <<<"$cfg")" \
            == "[\"bind\",\"$f\",true]" ]]
    done
}

@test "41. up and attach refuse a tampered cargo-volume mount point (3), naming it; a clean volume passes" {
    local vol="$HR_TEST_DOCKER_VOLUMES/htui-hr-cargo"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    # The check runs in a throwaway container before compose up.
    local check up
    check="$(grep -n '^run .*htui-hr-cargo:' "$HR_TEST_DOCKER_LOG" | head -n 1 | cut -d: -f1)"
    up="$(grep -n ' up -d --wait$' "$HR_TEST_DOCKER_LOG" | cut -d: -f1)"
    [[ -n "$check" && -n "$up" && "$check" -lt "$up" ]]
    grep -q '^run .* --network none --cap-drop ALL .*-v htui-hr-cargo:/v:ro ' "$HR_TEST_DOCKER_LOG"

    # A sandbox swapped a mount-point file for a directory, and bin/ for a file.
    local n
    for n in config config.toml credentials credentials.toml bin registry registry/src; do
        mkdir -p "$vol/registry"
        rm -rf "${vol:?}/$n"
        case "$n" in
            bin | registry | registry/src) : >"$vol/$n" ;;
            *) mkdir -p "$vol/$n/x" ;;
        esac
        : >"$HR_TEST_DOCKER_LOG"
        run --separate-stderr "$HR" attach MOD-5 </dev/null
        [[ $status -eq 3 && "$stderr" == *"htui-hr-cargo"*"$n"* ]] || { echo "$n: $status $stderr" >&2; return 1; }
        ! grep -q ' up ' "$HR_TEST_DOCKER_LOG" || false
        rm -rf "${vol:?}/$n"
    done
    # A symlink in place of a mount point is refused too.
    ln -s /etc "$vol/bin"
    run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 3 && "$stderr" == *'bin'* ]]
    [[ ! -e "$HR_ROOT/MOD-4" && ! -e "$(reg_of MOD-4)" ]]
    rm "$vol/bin"
    # Mount points of the right kind (as Docker leaves them) pass.
    mkdir -p "$vol/bin" "$vol/registry/src"
    : >"$vol/config.toml"
    run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 0 ]]
}

@test "42. compose: ~/.claude stays rw, cross-project data is masked, the run's own project comes back rw" {
    local cfg vols
    cfg="$(compose_config)"
    vols="$(jq -c '[.services.dev.volumes[] | {type, source, target, ro: (.read_only // false)}]' <<<"$cfg")"
    local c=/HR_HOME/.claude
    # Still the whole ~/.claude read-write: credentials, settings, plugins, skills, agents.
    jq -e --arg c "$c" 'any(.[]; . == {type: "bind", source: $c, target: $c, ro: false})' <<<"$vols" >/dev/null
    local d f i_tmp i_bind
    for d in "${HR_CLAUDE_MASK_DIRS[@]}"; do
        jq -e --arg t "$c/$d" 'any(.[]; . == {type: "tmpfs", source: null, target: $t, ro: false})' <<<"$vols" >/dev/null \
            || { echo "no tmpfs over $d" >&2; return 1; }
        # Sized: a tmpfs defaults to half of the host's RAM.
        [[ "$(jq -r --arg t "$c/$d" '.services.dev.volumes[] | select(.target == $t) | .tmpfs.size' <<<"$cfg")" \
            == "${HR_CLAUDE_TMPFS_SIZE[$d]}" ]] || { echo "tmpfs over $d: wrong or no size" >&2; return 1; }
    done
    for f in "${HR_CLAUDE_MASK_FILES[@]}"; do
        jq -e --arg s "/HR_CLAUDE_MASK/$f" --arg t "$c/$f" \
            'any(.[]; . == {type: "bind", source: $s, target: $t, ro: false})' <<<"$vols" >/dev/null \
            || { echo "no per-run file over $f" >&2; return 1; }
    done
    # The run's own project dir (auto-memory, its sessions), bound back rw after the tmpfs.
    local p="$c/projects/-HR_CLAUDE_PROJECT"
    jq -e --arg p "$p" 'any(.[]; . == {type: "bind", source: $p, target: $p, ro: false})' <<<"$vols" >/dev/null
    i_tmp="$(jq --arg t "$c/projects" 'map(.target) | index($t)' <<<"$vols")"
    i_bind="$(jq --arg t "$p" 'map(.target) | index($t)' <<<"$vols")"
    [[ "$i_tmp" -lt "$i_bind" ]]
    # Writable by the user: Docker gives a tmpfs the mode of the directory under it otherwise.
    [[ "$(jq --arg c "$c/" '[.services.dev.volumes[] | select(.type == "tmpfs" and (.target | startswith($c))) | .tmpfs.mode] | unique' <<<"$cfg" | tr -d ' \n')" \
        == '[1023]' ]]
    # Nothing else under ~/.claude is mounted: the list above is the whole mask. In particular
    # ~/.claude/security itself stays shared (the security plugin's agent-sdk-venv); only its
    # cross-project log.txt is masked, and the per-session plugin state is not (threat model).
    [[ "$(jq --arg c "$c/" '[.[] | select(.target | startswith($c))] | length' <<<"$vols")" \
        -eq $((${#HR_CLAUDE_MASK_DIRS[@]} + ${#HR_CLAUDE_MASK_FILES[@]} + 1)) ]]
    jq -e --arg t "$c/security/log.txt" 'any(.[]; .target == $t)' <<<"$vols" >/dev/null
    ! jq -e --arg t "$c/security" 'any(.[]; .target == $t)' <<<"$vols" >/dev/null || false
}

@test "43. up/attach: the Claude project name, the mount points on the host, the per-run empty files" {
    # The security plugin's venv lives next to the masked log.txt: it stays as it is.
    mkdir -p "$HOME/.claude/security/agent-sdk-venv/bin"
    printf 'venv\n' >"$HOME/.claude/security/agent-sdk-venv/bin/marker"
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    local env="$HR_TEST_DOCKER_LOG.env" name d f
    name="$(claude_project_of "$HR_HOST_REPO")"
    [[ "$name" == -* && "$name" != *[!a-zA-Z0-9-]* ]]
    grep -qxF "HR_CLAUDE_PROJECT=$name" "$env"
    grep -qxF "HR_CLAUDE_MASK=$HR_ROOT/MOD-5/claude-mask" "$env"
    # Mount points exist, as directories owned by the user (Docker would make them root-owned).
    [[ -d "$HOME/.claude/projects/$name" && -O "$HOME/.claude/projects/$name" ]]
    for d in "${HR_CLAUDE_MASK_DIRS[@]}"; do [[ -d "$HOME/.claude/$d" && -O "$HOME/.claude/$d" ]]; done
    for f in "${HR_CLAUDE_MASK_FILES[@]}"; do
        [[ -f "$HOME/.claude/$f" && -O "$HOME/.claude/$f" ]]
        [[ -f "$HR_ROOT/MOD-5/claude-mask/$f" && ! -s "$HR_ROOT/MOD-5/claude-mask/$f" ]]
        [[ "$(stat -c %a "$HR_ROOT/MOD-5/claude-mask/$f")" == 600 ]]
    done
    [[ "$(cat "$HOME/.claude/security/agent-sdk-venv/bin/marker")" == venv ]]
    # Existing host data is never touched: only missing mount points are created.
    printf 'host history\n' >"$HOME/.claude/history.jsonl"
    printf 'host security log\n' >"$HOME/.claude/security/log.txt"
    # A run made before the mask existed gets it on attach; down still parses (the vars are set).
    rm -rf "$HR_ROOT/MOD-5/claude-mask"
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" attach MOD-5 </dev/null
    [[ $status -eq 0 && -f "$HR_ROOT/MOD-5/claude-mask/history.jsonl" ]]
    [[ ! -s "$HR_ROOT/MOD-5/claude-mask/history.jsonl" ]]
    [[ -f "$HR_ROOT/MOD-5/claude-mask/security/log.txt" && ! -s "$HR_ROOT/MOD-5/claude-mask/security/log.txt" ]]
    [[ "$(cat "$HOME/.claude/history.jsonl")" == 'host history' ]]
    [[ "$(cat "$HOME/.claude/security/log.txt")" == 'host security log' ]]
    [[ "$(cat "$HOME/.claude/security/agent-sdk-venv/bin/marker")" == venv ]]
    run --separate-stderr "$HR" down MOD-5
    [[ $status -eq 0 ]]
    grep -qxF "HR_CLAUDE_PROJECT=$name" "$env"
    grep -qxF "HR_CLAUDE_MASK=$HR_ROOT/MOD-5/claude-mask" "$env"
}

@test "44. after the migration \$HR_STATE/runs is never imported again; it only warns, once per call" {
    mk_run MOD-5
    grep -qxF "$HR_STATE" "$HR_RUNS/.migrated"
    local good="$BATS_TEST_TMPDIR/reg.good"
    cp "$(reg_of MOD-5)" "$good"
    # A sandbox plants entries in the old place: a new run, and a changed copy of MOD-5.
    mkdir -p "$HR_STATE/runs"
    sed 's/MOD-5/MOD-4/g; s/mod-5/mod-4/g' "$good" >"$HR_STATE/runs/MOD-4"
    sed 's/^project=.*/project=htui/' "$good" >"$HR_STATE/runs/MOD-5"
    local v
    for v in ls "collect MOD-5" "down MOD-5" "attach MOD-5"; do
        # shellcheck disable=SC2086
        run --separate-stderr "$HR" $v </dev/null
        [[ $status -eq 0 ]] || { echo "hr $v: $status $stderr" >&2; return 1; }
        [[ "$(grep -c "$HR_STATE/runs" <<<"$stderr")" -eq 1 ]] || { echo "hr $v: $stderr" >&2; return 1; }
        [[ "$stderr" != *"registry entries"* ]]
        [[ ! -e "$(reg_of MOD-4)" && -f "$HR_STATE/runs/MOD-4" ]]
        cmp <(grep -v '^collected_' "$(reg_of MOD-5)") <(grep -v '^collected_' "$good")
    done
    run --separate-stderr "$HR" ls
    [[ "$output" != *MOD-4* ]]
    # A plain file, or a symlink, there: the same warning, never a dead verb.
    rm -rf "$HR_STATE/runs"
    : >"$HR_STATE/runs"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 && "$output" == *MOD-5* && "$(grep -c "$HR_STATE/runs" <<<"$stderr")" -eq 1 ]]
    rm "$HR_STATE/runs"
    ln -s /nonexistent "$HR_STATE/runs"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 && "$output" == *MOD-5* && "$(grep -c "$HR_STATE/runs" <<<"$stderr")" -eq 1 ]]
    rm "$HR_STATE/runs"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 && "$stderr" != *"$HR_STATE/runs"* ]]
}

@test "45. collect --merge shows what the run changes in tool config and asks first; a refusal leaves the host as it was" {
    mk_run MOD-5
    local src p
    src="$(src_of MOD-5)"
    local -a paths=(.claude/settings.json scripts/tool.sh docker/x/Dockerfile CLAUDE.md AGENTS.md .mcp.json
        .cargo/config.toml rust-toolchain.toml)
    for p in "${paths[@]}"; do
        mkdir -p "$src/$(dirname "$p")"
        printf 'from the run\n' >"$src/$p"
        git -C "$src" add -f -- "$p"
    done
    git -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q -m 'run changes tool config'
    local before reg
    before="$(host_snapshot)"
    reg="$(cat "$(reg_of MOD-5)")"
    host_unchanged() {
        [[ "$(host_snapshot)" == "$before" && "$(cat "$(reg_of MOD-5)")" == "$reg" ]] || return 1
        [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect refs/heads/hr)" ]] || return 1
        ! git -C "$HR_HOST_REPO" rev-parse -q --verify MERGE_HEAD >/dev/null
    }

    # No terminal, no --yes: the stat names every path, then a refusal before anything moves.
    run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 1 && "$stderr" == *'needs confirmation'*'--yes'* ]]
    for p in "${paths[@]}"; do [[ "$output$stderr" == *"$p"* ]] || { echo "stat lacks $p" >&2; return 1; }; done
    host_unchanged

    # In a terminal it always asks, --yes or not; declined -> the same refusal.
    local fb="$BATS_TEST_TMPDIR/fakebin"
    mkdir -p "$fb"
    printf '#!/bin/sh\necho "$*" >>"%s/gum.log"\n[ "$1" = confirm ] && exit "${FAKE_GUM_CONFIRM:-1}"\nexit 0\n' \
        "$BATS_TEST_TMPDIR" >"$fb/gum"
    chmod +x "$fb/gum"
    PATH="$fb:$PATH" HR_INTERACTIVE=1 run --separate-stderr "$HR" collect MOD-5 --merge --yes
    [[ $status -eq 1 ]]
    grep -q '^confirm .*hr/MOD-5' "$BATS_TEST_TMPDIR/gum.log"
    [[ "$output$stderr" == *'.claude/settings.json'* ]]
    host_unchanged

    # Accepted in a terminal, or --yes without one: collected and merged.
    PATH="$fb:$PATH" HR_INTERACTIVE=1 FAKE_GUM_CONFIRM=0 run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 0 && "$(git -C "$HR_HOST_REPO" log -1 --format=%s)" == 'Merge hr/MOD-5 (sandbox run)' ]]
    [[ "$(reg_get MOD-5 collected_sha)" == "$(git -C "$HR_HOST_REPO" rev-parse hr/MOD-5)" ]]
    [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect)" ]]
    git -C "$HR_HOST_REPO" reset -q --hard HEAD^
    src_commit MOD-5 .claude/more.txt 'more config'
    run --separate-stderr "$HR" collect MOD-5 --merge --yes
    [[ $status -eq 0 && "$output$stderr" == *'.claude/more.txt'* ]]
    [[ "$(git -C "$HR_HOST_REPO" log -1 --format=%s)" == 'Merge hr/MOD-5 (sandbox run)' ]]
}

@test "46. collect --merge without a validator at HEAD -> 2 before the fetch: no hr/ITEM, registry unchanged" {
    mk_run MOD-5
    git -C "$HR_HOST_REPO" rm -q .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
    git -C "$HR_HOST_REPO" commit -q -m 'no validator'
    local reg before
    reg="$(cat "$(reg_of MOD-5)")"
    before="$(host_snapshot)"
    run --separate-stderr "$HR" collect MOD-5 --merge --yes
    [[ $status -eq 2 && "$stderr" == *'validate-workflow-docs.sh'* ]]
    ! git -C "$HR_HOST_REPO" rev-parse -q --verify refs/heads/hr/MOD-5 || false
    [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect)" ]]
    [[ "$(cat "$(reg_of MOD-5)")" == "$reg" && "$(host_snapshot)" == "$before" ]]
}

@test "47. up excludes tool-owned dirs in the clone; their untracked files never block a purge" {
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    local src d
    src="$(src_of MOD-5)"
    for d in /.serena/ /.claude/skills/generated/ /.kiro/ /graphify-out/; do
        grep -qxF "$d" "$src/.git/info/exclude" || { echo "no $d in exclude" >&2; return 1; }
    done
    mkdir -p "$src/.serena/memories" "$src/.claude/skills/generated/x" "$src/.kiro/steering" "$src/graphify-out"
    : >"$src/.serena/memories/note.md"
    : >"$src/.claude/skills/generated/x/SKILL.md"
    : >"$src/.kiro/steering/a.md"
    : >"$src/graphify-out/graph.json"
    [[ -z "$(git -C "$src" status --porcelain)" ]]
    [[ "$(purge_state MOD-5)" == clean ]]
    # Anything else untracked still blocks.
    : >"$src/.claude/skills/new.md"
    [[ "$(purge_state MOD-5)" == *'.claude/skills/new.md'* ]]
}

@test "48. a vanished lease file blocks only the purge that prunes leases (forced, uncollected)" {
    mk_run MOD-4
    mk_run MOD-5
    "$HR" collect MOD-5 >/dev/null 2>&1
    rm -f "$(lease_file)" "$HR_STATE/id-leases.lock"
    # A checked purge, and a forced purge of a collected run, prune nothing: they go through.
    run --separate-stderr "$HR" down MOD-5 --purge --yes
    [[ $status -eq 0 && ! -e "$HR_ROOT/MOD-5" ]]
    "$HR" collect MOD-4 >/dev/null 2>&1
    run --separate-stderr "$HR" down MOD-4 --purge --force --yes
    [[ $status -eq 0 && ! -e "$HR_ROOT/MOD-4" ]]
    [[ ! -e "$(lease_file)" ]]
    # (A forced purge of an uncollected run would prune its leases: refused — test 27.)
}

@test "49. Ctrl-C in the recipe claude session: hr still prints the merge-in-progress hint and cleans up" {
    mk_run MOD-4
    local s4
    s4="$(src_of MOD-4)"
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** sandbox body/' "$s4/HANDOFF.md"
    git -C "$s4" -c user.name=s -c user.email=s@x.invalid commit -q -am 'sandbox edits MOD-4'
    sed -i 's/^- \[ \] \*\*MOD-4 - Thing\.\*\* body$/- [ ] **MOD-4 - Thing.** host body/' "$HR_HOST_REPO/HANDOFF.md"
    git -C "$HR_HOST_REPO" commit -q -am 'host edits MOD-4'

    # A Ctrl-C in a terminal reaches the whole foreground process group: hr, its subshell and
    # claude. hr runs in a session of its own here (setsid), so `kill -INT 0` in the fake claude
    # hits exactly that group, never bats. The fake dies of it, or catches it and exits 130.
    local fb="$BATS_TEST_TMPDIR/fakebin" tmp="$BATS_TEST_TMPDIR/tmp" mode
    mkdir -p "$fb" "$tmp"
    printf '#!/bin/sh\nexit 0\n' >"$fb/gum"
    printf '#!/bin/sh\ntouch "%s/claude-launched"\n[ "$FAKE_CLAUDE" = catch ] && trap "exit 130" INT\nkill -INT 0\nsleep 2\n' \
        "$BATS_TEST_TMPDIR" >"$fb/claude"
    chmod +x "$fb/claude" "$fb/gum"
    for mode in die catch; do
        rm -f "$BATS_TEST_TMPDIR/claude-launched"
        run --separate-stderr env PATH="$fb:$PATH" HR_INTERACTIVE=1 TMPDIR="$tmp" FAKE_CLAUDE="$mode" \
            setsid -w "$HR" collect MOD-4 --merge --yes
        echo "$mode: $status $stderr"
        [[ -e "$BATS_TEST_TMPDIR/claude-launched" ]]
        [[ $status -eq 1 && "$stderr" == *'merge in progress'*'git merge --abort'* ]]
        git -C "$HR_HOST_REPO" rev-parse -q --verify MERGE_HEAD
        # The EXIT cleanup ran: no temporary ref, no validator copy.
        [[ -z "$(git -C "$HR_HOST_REPO" for-each-ref refs/hr-collect)" ]]
        [[ -z "$(ls -A "$tmp")" ]]
        git -C "$HR_HOST_REPO" merge --abort
    done
}

# ---------------------------------------------------------------------------------------------
# Docker (real image and daemon; fixture host repo; run once: bats --filter-tags docker tests/hr/hr.bats)

# bats test_tags=docker
@test "D1. hr build: image labels match the host identity" {
    hr_docker_setup --no-image TOOL-9001
    run --separate-stderr "$HR" build
    [[ $status -eq 0 ]]
    run docker image inspect -f '{{index .Config.Labels "hr.uid"}} {{index .Config.Labels "hr.gid"}} {{index .Config.Labels "hr.home"}}' htui-hr-dev
    [[ "$output" == "$(id -u) $(id -g) $HOME" ]]
    docker volume inspect htui-hr-cargo >/dev/null
}

# bats test_tags=docker
@test "D2. hr up: toolchain, private postgres + qdrant, no credentials, ro host repo, no publishers" {
    hr_docker_setup TOOL-9001
    run --separate-stderr "$HR" up TOOL-9001
    echo "$output"; echo "$stderr"
    [[ $status -eq 0 ]]
    [[ "$(dexec TOOL-9001 cargo --version)" == 'cargo 1.98.1'* ]]
    [[ "$(dexec TOOL-9001 cargo sqlx --version)" == *'0.9.0'* ]]
    [[ "$(dexec TOOL-9001 psql -h localhost -p 5439 -U postgres -tAc 'select 1')" == 1 ]]
    local i ok=0
    for i in $(seq 1 30); do
        dexec TOOL-9001 curl -fs localhost:6333 >/dev/null && { ok=1; break; }
        sleep 1
    done
    [[ $ok -eq 1 ]]
    [[ -z "$(dexec TOOL-9001 git config --global credential.helper || true)" ]]
    run ! dexec TOOL-9001 touch /host/htui/x
    [[ "$(dexec TOOL-9001 git -C "$HR_HOST_REPO" symbolic-ref --short HEAD)" == hr/TOOL-9001 ]]
    [[ "$(dexec TOOL-9001 git -C "$HR_HOST_REPO" remote get-url origin)" == /host/htui ]]
    dexec TOOL-9001 git -C "$HR_HOST_REPO" fetch -q origin
    [[ "$(dexec TOOL-9001 printenv HR_SANDBOX)" == 1 ]]
    [[ "$(dexec TOOL-9001 stat -c %U "$HR_HOST_REPO/.remember")" == "$(id -un)" ]]
    # Exposed-only ports (5432/tcp, 6333/tcp) are listed too; a published one reads host:port->port.
    local ports
    ports="$(docker ps --filter label=com.docker.compose.project=hr-tool-9001 --format '{{.Names}} {{.Ports}}')"
    echo "$ports"
    [[ "$(wc -l <<<"$ports")" -eq 3 ]]
    [[ "$ports" != *'->'* ]]
    [[ "$(stat -c %U "$HR_HOST_REPO/.remember")" == "$(id -un)" ]]

    # C1: of ~/.local only the tool dirs are visible, read-only; nothing that holds secrets.
    [[ "$(dexec TOOL-9001 bash -c 'ls -A ~/.local | tr "\n" " "')" == 'bin share state ' ]]
    [[ "$(dexec TOOL-9001 bash -c 'ls -A ~/.local/share | tr "\n" " "')" == 'mise uv ' ]]
    [[ "$(dexec TOOL-9001 bash -c 'ls -A ~/.local/share/uv | tr "\n" " "')" == 'python tools ' ]]
    [[ -z "$(dexec TOOL-9001 bash -c 'ls -A ~/.local/state')" ]]
    [[ -z "$(dexec TOOL-9001 bash -c 'for p in ~/.local/share/keyrings ~/.local/share/uv/credentials \
        ~/.local/state/nvim ~/.local/share/gh ~/.local/share/pipx; do [ -e "$p" ] && echo "$p"; done; true')" ]]
    run ! dexec TOOL-9001 touch "$HOME/.local/bin/hr-x"
    run ! dexec TOOL-9001 touch "$HOME/.local/share/uv/tools/hr-x"
    # ... and every host tool still runs from them.
    dexec TOOL-9001 claude --version
    dexec TOOL-9001 gortex version
    dexec TOOL-9001 headroom --version
    [[ "$(dexec TOOL-9001 uvx --version)" == 'uvx 0.12.'* ]]
    [[ "$(dexec TOOL-9001 "$HOME/.local/bin/python3" --version)" == 'Python 3.'* ]]
    dexec TOOL-9001 graphify --help >/dev/null

    # M-C: of ~/.claude/projects only the run's own project is there; the other cross-project data
    # is empty (its host copy untouched); the login and settings are still shared.
    [[ "$(dexec TOOL-9001 bash -c 'ls -A ~/.claude/projects')" == "$(claude_project_of "$HR_HOST_REPO")" ]]
    local d
    for d in "${HR_CLAUDE_MASK_DIRS[@]}"; do
        [[ "$d" == projects ]] && continue
        [[ -z "$(dexec TOOL-9001 bash -c "ls -A ~/.claude/$d")" ]] || { echo "~/.claude/$d not empty" >&2; return 1; }
    done
    for f in "${HR_CLAUDE_MASK_FILES[@]}"; do
        [[ -z "$(dexec TOOL-9001 bash -c "cat ~/.claude/$f")" ]] || { echo "~/.claude/$f not empty" >&2; return 1; }
    done
    dexec TOOL-9001 test -s "$HOME/.claude/.credentials.json"
    dexec TOOL-9001 test -s "$HOME/.claude/settings.json"
    dexec TOOL-9001 test -d "$HOME/.claude/plugins"
    dexec TOOL-9001 test -d "$HOME/.claude/skills"
    # Of ~/.claude/security only log.txt is masked (in the loop above); the plugin's venv is shared.
    if [[ -d "$HOME/.claude/security/agent-sdk-venv" ]]; then
        dexec TOOL-9001 test -d "$HOME/.claude/security/agent-sdk-venv"
    fi
    # The tmpfs masks carry the sizes compose gives them (none is half of the host's RAM).
    local opts
    opts="$(dexec TOOL-9001 findmnt -no OPTIONS "$HOME/.claude/projects")"
    [[ "$opts" == *'size=262144k'* ]] || { echo "projects tmpfs: $opts" >&2; return 1; }
    opts="$(dexec TOOL-9001 findmnt -no OPTIONS "$HOME/.claude/sessions")"
    [[ "$opts" == *'size=16384k'* ]] || { echo "sessions tmpfs: $opts" >&2; return 1; }

    # H2: the host's dev Postgres/Qdrant are published on loopback only, so the bridge gateway
    # (the host, seen from the run's network) refuses 5439 and 6333.
    local probe='h=$(awk '"'"'$2 == "00000000" { print $3; exit }'"'"' /proc/net/route)
        gw=$(printf "%d.%d.%d.%d" 0x${h:6:2} 0x${h:4:2} 0x${h:2:2} 0x${h:0:2})
        echo "$gw"; timeout 3 bash -c "</dev/tcp/$gw/$1" 2>/dev/null'
    run dexec TOOL-9001 bash -c "$probe" _ 5439
    echo "gateway $output"
    [[ $status -ne 0 && "$output" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]
    run dexec TOOL-9001 bash -c "$probe" _ 6333
    [[ $status -ne 0 && "$output" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]

    # M5: the shared cargo volume cannot reconfigure other runs' builds or shadow their tools.
    local path
    path="$(dexec TOOL-9001 printenv PATH)"
    [[ "$path" != *'.cargo/bin'* && "$path" == *':/tmp/cargo-install/bin' ]]
    [[ "$(dexec TOOL-9001 printenv CARGO_INSTALL_ROOT)" == /tmp/cargo-install ]]
    local f
    for f in config config.toml credentials credentials.toml; do
        [[ -z "$(dexec TOOL-9001 cat "$HOME/.cargo/$f")" ]]
        run ! dexec TOOL-9001 bash -c "echo '[build]' >>\"\$HOME/.cargo/$f\""
    done
    # One file behind all four names: cargo does not warn that config and config.toml both exist.
    run --separate-stderr dexec TOOL-9001 bash -c 'cd /tmp && cargo new -q --vcs none hr-probe && cd hr-probe && cargo metadata -q --no-deps --offline --format-version 1 >/dev/null'
    [[ $status -eq 0 ]]
    [[ "$stderr" != *warning* ]]
    # M-A: cargo looks in CARGO_HOME/bin for cargo-<cmd> ahead of PATH: it is empty and read-only,
    # so `cargo sqlx` is always the image's.
    [[ -z "$(dexec TOOL-9001 bash -c 'ls -A ~/.cargo/bin')" ]]
    run ! dexec TOOL-9001 bash -c 'touch ~/.cargo/bin/cargo-sqlx'
    [[ "$(dexec TOOL-9001 bash -c 'command -v cargo-sqlx')" == /opt/rust/cargo/bin/cargo-sqlx ]]
    [[ "$(dexec TOOL-9001 bash -c 'cd /tmp/hr-probe && cargo sqlx --version')" == *'0.9.0'* ]]
    # Residual: the extracted crate sources are a per-run volume, owned by the user.
    dexec TOOL-9001 mountpoint -q "$HOME/.cargo/registry/src"
    [[ "$(dexec TOOL-9001 stat -c %U "$HOME/.cargo/registry/src")" == "$(id -un)" ]]
    [[ "$(docker volume ls -q --filter label=com.docker.compose.project=hr-tool-9001 | grep -c '_cargo-src$')" -eq 1 ]]

    run --separate-stderr "$HR" ls
    [[ "$output" == *'TOOL-9001'*'running'* ]]
}

# bats test_tags=docker
@test "D3. two runs have separate databases" {
    hr_docker_setup TOOL-9001 TOOL-9002
    run --separate-stderr "$HR" up TOOL-9001
    [[ $status -eq 0 ]]
    run --separate-stderr "$HR" up TOOL-9002
    [[ $status -eq 0 ]]
    dexec TOOL-9001 psql -h localhost -p 5439 -U postgres -qc 'create table only_in_a (x int)'
    [[ "$(dexec TOOL-9001 psql -h localhost -p 5439 -U postgres -tAc "select to_regclass('only_in_a') is not null")" == t ]]
    [[ "$(dexec TOOL-9002 psql -h localhost -p 5439 -U postgres -tAc "select to_regclass('only_in_a') is not null")" == f ]]
    # Extracted crate sources are per run: what one run writes there, the other does not compile.
    dexec TOOL-9001 bash -c 'touch ~/.cargo/registry/src/only-in-a'
    dexec TOOL-9001 test -e "$HOME/.cargo/registry/src/only-in-a"
    run ! dexec TOOL-9002 test -e "$HOME/.cargo/registry/src/only-in-a"
}

# bats test_tags=docker
@test "D4. down --purge removes the run's volumes; htui-hr-cargo survives" {
    hr_docker_setup TOOL-9001
    run --separate-stderr "$HR" up TOOL-9001
    [[ $status -eq 0 ]]
    [[ -n "$(docker volume ls -q --filter label=com.docker.compose.project=hr-tool-9001)" ]]
    run --separate-stderr "$HR" down TOOL-9001 --purge --yes
    echo "$output"; echo "$stderr"
    [[ $status -eq 0 ]]
    [[ -z "$(docker volume ls -q --filter label=com.docker.compose.project=hr-tool-9001)" ]]
    [[ -z "$(docker ps -aq --filter label=com.docker.compose.project=hr-tool-9001)" ]]
    docker volume inspect htui-hr-cargo >/dev/null
    [[ ! -e "$HR_ROOT/TOOL-9001" && ! -e "$(reg_of TOOL-9001)" ]]
}

# bats test_tags=docker
@test "D5. hostile clone: the sandbox's clean filter runs in the throwaway check container, never on the host" {
    hr_docker_setup TOOL-9001
    run --separate-stderr "$HR" up TOOL-9001
    [[ $status -eq 0 ]]
    local src m="$BATS_TEST_TMPDIR/pwned"
    src="$(src_of TOOL-9001)"
    git -C "$src" -c user.name=s -c user.email=s@x.invalid commit -q --allow-empty -m 'sandbox work'
    run --separate-stderr "$HR" collect TOOL-9001
    [[ $status -eq 0 ]]
    # A filter that leaves a marker where it runs and changes what it cleans: if it runs in the
    # container, status there reports HANDOFF.md modified; if it ran here, the marker would exist.
    git config -f "$src/.git/config" filter.x.clean "sh -c 'mkdir -p $BATS_TEST_TMPDIR && touch $m; cat; echo tampered'"
    git config -f "$src/.git/config" filter.x.required true
    printf '* filter=x\n' >>"$src/.git/info/attributes"
    touch "$src/HANDOFF.md"
    run --separate-stderr "$HR" down TOOL-9001 --purge --yes
    echo "$stderr"
    [[ $status -eq 1 ]]
    [[ "$stderr" == *'uncommitted work'*'HANDOFF.md'* ]]
    [[ ! -e "$m" && -d "$src/.git" ]]
    # Without the filter the same tree is clean, and the purge goes through.
    : >"$src/.git/info/attributes"
    run --separate-stderr "$HR" down TOOL-9001 --purge --yes
    echo "$stderr"
    [[ $status -eq 0 && ! -e "$HR_ROOT/TOOL-9001" && ! -e "$m" ]]
}
