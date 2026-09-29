# shellcheck shell=bash
# hr_helper.bash — fixtures for tests/hr/hr.bats (TOOL-7 T4, `scripts/hr`).
#
# Layers on T3's test_helper.bash (read-only reuse: hr_base_setup, make_fixture, lease helpers).
# Everything a test touches lives under $BATS_TEST_TMPDIR: the "host repo" is a fixture repo, HOME
# is a fake home, HR_ROOT / HR_SSD_ROOT / HR_STATE are tmp dirs, and `docker` is a stub that only
# logs. hr_setup asserts every one of those paths before a test runs, so no case can reach the real
# repo, the real ~/.claude*, or /media/projects/htui-hr.
#
# Docker-tagged cases (hr_docker_setup) keep the real HOME and the real docker, but the host repo,
# HR_ROOT and HR_STATE stay under $BATS_TEST_TMPDIR.

load test_helper

HR="$HR_TEST_REPO_ROOT/scripts/hr"
HR_COMPOSE_FILE="$HR_TEST_REPO_ROOT/docker/hr/compose.hr.yaml"

# Variables a caller's shell may carry that would point hr somewhere real.
hr_clear_env() {
    unset HR_SANDBOX HR_ITEM HR_SRC HR_CLAUDE_JSON HR_CPUS HR_IMAGE HR_NO_GUM HR_GIT_NAME \
        HR_GIT_EMAIL HR_HOME HR_UID HR_GID HR_STATE_HOST HR_MISE_PATH HR_INTERACTIVE HR_DOCKER \
        HR_ROOT HR_SSD_ROOT HR_HOST_REPO XDG_CONFIG_HOME GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE \
        GIT_CONFIG_GLOBAL HR_TEST_DOCKER_IMAGE HR_TEST_DOCKER_RUNNING HR_TEST_DOCKER_FAIL_UP
}

# Per-test baseline for the non-Docker cases.
hr_setup() {
    hr_base_setup || return 1
    hr_clear_env
    export HOME="$BATS_TEST_TMPDIR/home"
    export HR_HOST_REPO="$BATS_TEST_TMPDIR/host"
    export HR_ROOT="$BATS_TEST_TMPDIR/hdd"
    export HR_SSD_ROOT="$BATS_TEST_TMPDIR/ssd"
    export HR_DOCKER="$BATS_TEST_TMPDIR/bin/docker"
    export HR_TEST_DOCKER_LOG="$BATS_TEST_TMPDIR/docker.log"
    local p
    for p in "$HOME" "$HR_HOST_REPO" "$HR_ROOT" "$HR_SSD_ROOT" "$HR_STATE" "$HR_DOCKER" \
        "$HR_TEST_DOCKER_LOG"; do
        hr_assert_tmp_path "$p" || return 1
    done
    export GIT_CONFIG_NOSYSTEM=1
    export HR_MISE_PATH=/usr/bin
    export HR_INTERACTIVE=0

    hr_fake_home "$HOME" || return 1
    make_fixture "$HR_HOST_REPO" || return 1
    # The real repo ignores .remember/ through its own .remember/.gitignore; the fixture via exclude.
    printf '/.remember/\n' >>"$HR_HOST_REPO/.git/info/exclude"
    # Untracked, globally ignored (fake ~/.config/git/ignore), as on the real host.
    mkdir -p "$HR_HOST_REPO/.claude"
    printf '{"hooks": {"fixture": true}}\n' >"$HR_HOST_REPO/.claude/settings.local.json"

    hr_stub_docker || return 1
    : >"$HR_TEST_DOCKER_LOG"
}

# hr_fake_home DIR — the host-side sources `hr up` preflights and mounts.
hr_fake_home() {
    local h="$1"
    hr_assert_tmp_path "$h" || return 1
    mkdir -p "$h/.claude" "$h/.local/bin" "$h/.gortex/instructions" "$h/.gortex/models" \
        "$h/.config/git"
    printf '{"fake": "claude.json"}\n' >"$h/.claude.json"
    chmod 644 "$h/.claude.json"
    printf 'fake: gortex\n' >"$h/.gortex/config.yaml"
    printf '**/.claude/settings.local.json\n*.fake-global-ignore\n' >"$h/.config/git/ignore"
}

# The stub docker: logs every call's argv to $HR_TEST_DOCKER_LOG, and on `compose` the HR_* env it
# received to $HR_TEST_DOCKER_LOG.env. Knobs: HR_TEST_DOCKER_IMAGE=missing|mismatch,
# HR_TEST_DOCKER_RUNNING="hr-mod-5 ..." (projects whose `ps` is non-empty),
# HR_TEST_DOCKER_FAIL_UP=1 (`compose ... up` exits 1).
hr_stub_docker() {
    mkdir -p "$(dirname "$HR_DOCKER")"
    cat >"$HR_DOCKER" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$HR_TEST_DOCKER_LOG"
case "$1 ${2:-}" in
    "image inspect")
        case "${HR_TEST_DOCKER_IMAGE:-ok}" in
            missing) echo "Error: No such image: ${*: -1}" >&2; exit 1 ;;
            mismatch) echo "4242 4242 /home/somebody-else"; exit 0 ;;
            *) echo "$(id -u) $(id -g) $HOME"; exit 0 ;;
        esac
        ;;
esac
if [[ "$1" == compose ]]; then
    env | grep -E '^(HR_[A-Z_]*|TERM|COLORTERM)=' | sort >"$HR_TEST_DOCKER_LOG.env"
    proj='' prev=''
    for a in "$@"; do
        [[ "$prev" == -p ]] && proj="$a"
        prev="$a"
    done
    if [[ " $* " == *" ps "* ]]; then
        [[ " ${HR_TEST_DOCKER_RUNNING:-} " == *" $proj "* ]] && echo "c0ffee-$proj"
        exit 0
    fi
    if [[ " $* " == *" up "* && -n "${HR_TEST_DOCKER_FAIL_UP:-}" ]]; then
        echo "stub: compose up failed" >&2
        exit 1
    fi
fi
exit 0
EOF
    chmod +x "$HR_DOCKER"
}

# docker_log — the stub's call log.
docker_log() { cat "$HR_TEST_DOCKER_LOG"; }

# src_of ITEM [ROOT] — the run's clone.
src_of() { printf '%s/%s/src' "${2:-$HR_ROOT}" "$1"; }

# reg_of ITEM — the run's registry file.
reg_of() { printf '%s/runs/%s' "$HR_STATE" "$1"; }

# reg_get ITEM KEY — one registry value.
reg_get() { sed -n "s/^$2=//p" "$(reg_of "$1")"; }

# src_commit ITEM FILE CONTENT [MSG] — one commit in the run's clone (sandbox-side work).
src_commit() {
    local src
    src="$(src_of "$1")"
    hr_assert_tmp_path "$src" || return 1
    mkdir -p "$(dirname "$src/$2")"
    printf '%s\n' "$3" >"$src/$2"
    git -C "$src" add -- "$2"
    git -C "$src" -c user.name=sandbox -c user.email=sandbox@example.invalid \
        -c commit.gpgsign=false commit -q -m "${4:-work on $1}"
}

# host_commit FILE CONTENT [MSG] — one commit on the host fixture's current branch.
host_commit() {
    mkdir -p "$(dirname "$HR_HOST_REPO/$1")"
    printf '%s\n' "$2" >"$HR_HOST_REPO/$1"
    git -C "$HR_HOST_REPO" add -- "$1"
    git -C "$HR_HOST_REPO" commit -q -m "${3:-host edit $1}"
}

# mk_run ITEM [up args...] — `hr up ITEM` plus one commit in its clone.
mk_run() {
    local item="$1"
    shift
    "$HR" up "$item" "$@" >/dev/null 2>&1 || { echo "mk_run: hr up $item failed" >&2; return 1; }
    src_commit "$item" "notes/$item.txt" "sandbox work for $item"
}

# host_snapshot — what `hr` must never change on the host except the refs it is asked to write.
host_snapshot() {
    git -C "$HR_HOST_REPO" rev-parse HEAD
    git -C "$HR_HOST_REPO" symbolic-ref -q HEAD || echo detached
    git -C "$HR_HOST_REPO" status --porcelain --untracked-files=all
    git -C "$HR_HOST_REPO" for-each-ref --format='%(refname) %(objectname)' refs/heads
}

# ---- Docker-tagged cases ----

# hr_docker_setup ITEM... — real docker + real HOME; fixture host repo with the given open items
# (TOOL-9xxx so no real run's project name can collide); HR_ROOT/HR_STATE under the tmpdir.
hr_docker_setup() {
    hr_base_setup || return 1
    docker info >/dev/null 2>&1 || skip "docker daemon not reachable"
    docker image inspect "${HR_IMAGE:-htui-hr-dev}" >/dev/null 2>&1 || skip "image htui-hr-dev not built"
    local item proj
    for item in "$@"; do
        proj="hr-${item,,}"
        [[ -z "$(docker ps -aq --filter "label=com.docker.compose.project=$proj")" ]] \
            || skip "compose project $proj already exists"
    done
    local home="$HOME"
    hr_clear_env
    export HOME="$home"
    export HR_HOST_REPO="$BATS_TEST_TMPDIR/host"
    export HR_ROOT="$BATS_TEST_TMPDIR/hdd"
    export HR_SSD_ROOT="$BATS_TEST_TMPDIR/ssd"
    export HR_INTERACTIVE=0
    local p
    for p in "$HR_HOST_REPO" "$HR_ROOT" "$HR_SSD_ROOT" "$HR_STATE"; do
        hr_assert_tmp_path "$p" || return 1
    done
    make_fixture "$HR_HOST_REPO" || return 1
    for item in "$@"; do
        printf -- '- [ ] **%s - Docker case.** body\n' "$item" >>"$HR_HOST_REPO/HANDOFF.md"
    done
    git -C "$HR_HOST_REPO" commit -q -am 'docker case items'
    HR_DOCKER_ITEMS=("$@")
}

# hr_docker_teardown — remove every container, network and per-run volume of the test's projects.
# Never touches the shared htui-hr-cargo volume or the image.
hr_docker_teardown() {
    local item proj ids
    for item in "${HR_DOCKER_ITEMS[@]}"; do
        proj="hr-${item,,}"
        ids="$(docker ps -aq --filter "label=com.docker.compose.project=$proj")"
        [[ -n "$ids" ]] && docker rm -f $ids >/dev/null 2>&1
        ids="$(docker volume ls -q --filter "label=com.docker.compose.project=$proj")"
        [[ -n "$ids" ]] && docker volume rm $ids >/dev/null 2>&1
        ids="$(docker network ls -q --filter "label=com.docker.compose.project=$proj")"
        [[ -n "$ids" ]] && docker network rm $ids >/dev/null 2>&1
    done
    return 0
}

# dexec ITEM CMD... — run a command in the run's dev container (no TTY).
dexec() {
    local item="$1"
    shift
    docker exec "hr-${item,,}-dev-1" "$@"
}
