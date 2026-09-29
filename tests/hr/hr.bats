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
    run --separate-stderr "$HR" collect MOD-5 --merge
    [[ $status -eq 0 ]]
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

@test "26. a registry in the old place (\$HR_STATE/runs) moves to HR_RUNS on first use, then is validated" {
    mk_run MOD-5
    mkdir -p "$HR_STATE/runs"
    mv "$(reg_of MOD-5)" "$HR_STATE/runs/MOD-5"
    : >"$HR_STATE/runs.lock"
    run --separate-stderr "$HR" ls
    [[ $status -eq 0 ]]
    [[ "$stderr" == *"moved 1 registry entries from $HR_STATE/runs"* ]]
    [[ "$output" == *MOD-5* ]]
    [[ -f "$(reg_of MOD-5)" && ! -e "$HR_STATE/runs" && ! -e "$HR_STATE/runs.lock" ]]

    # A sandbox-edited entry is moved, then refused like any other.
    mkdir -p "$HR_STATE/runs"
    mv "$(reg_of MOD-5)" "$HR_STATE/runs/MOD-5"
    sed -i 's/^project=.*/project=htui/' "$HR_STATE/runs/MOD-5"
    : >"$HR_TEST_DOCKER_LOG"
    run --separate-stderr "$HR" down MOD-5 --purge --force --yes
    [[ $status -eq 3 ]]
    [[ "$stderr" == *'project'* ]]
    [[ -d "$(src_of MOD-5)/.git" ]]
    ! grep -q '^compose' "$HR_TEST_DOCKER_LOG" || false

    # In both places -> refused, neither touched; the old dir as a symlink -> refused.
    cp "$(reg_of MOD-5)" "$BATS_TEST_TMPDIR/both"
    mkdir -p "$HR_STATE/runs"
    cp "$BATS_TEST_TMPDIR/both" "$HR_STATE/runs/MOD-5"
    run --separate-stderr "$HR" ls
    [[ $status -eq 3 && "$stderr" == *'both'* ]]
    [[ -f "$HR_STATE/runs/MOD-5" && -f "$(reg_of MOD-5)" ]]
    rm -rf "$HR_STATE/runs"
    ln -s "$HR_RUNS" "$HR_STATE/runs"
    run --separate-stderr "$HR" ls
    [[ $status -eq 3 && "$stderr" == *'not a directory'* ]]
}

@test "27. the lease file vanished after hr-mint --init -> up, gc, down --purge refuse (1); never recreated" {
    mk_run MOD-5
    grep -qxF "$HR_STATE" "$HR_RUNS/.leases-initialized"
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
    # A different state dir has its own record: the guard is per HR_STATE.
    HR_STATE="$BATS_TEST_TMPDIR/state2" run --separate-stderr "$HR" up MOD-4
    [[ $status -eq 0 ]]
}

@test "28. up warns once that a non-default HR_STATE must be exported for host mints" {
    run --separate-stderr "$HR" up MOD-5
    [[ $status -eq 0 ]]
    [[ "$(grep -c 'HR_STATE is non-default' <<<"$stderr")" -eq 1 ]]
    [[ "$stderr" == *"export HR_STATE=$HR_STATE"* ]]
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
    run --separate-stderr "$hrx" collect MOD-5 --merge
    echo "$output$stderr"
    [[ $status -eq 0 ]]
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
    printf '#!/bin/sh\necho "$*" >>"%s/gum.log"\n[ "$1" = confirm ] && exit "${FAKE_GUM_CONFIRM:-1}"\nexit 0\n' \
        "$BATS_TEST_TMPDIR" >"$fb/gum"
    chmod +x "$fb/claude" "$fb/gum"
    local -a env=(PATH="$fb:$PATH" HR_INTERACTIVE=1)

    # MOD-5: --yes does not skip the question; the stat names the file; declined -> no claude.
    run --separate-stderr env "${env[@]}" "$HR" collect MOD-5 --merge --yes
    [[ $status -eq 1 ]]
    [[ "$output" == *'.claude/settings.json'* ]]
    grep -q '^confirm ' "$BATS_TEST_TMPDIR/gum.log"
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
