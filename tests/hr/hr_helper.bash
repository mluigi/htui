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
        HR_ROOT HR_SSD_ROOT HR_RUNS HR_HOST_REPO XDG_CONFIG_HOME GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE \
        GIT_CONFIG_GLOBAL HR_TEST_DOCKER_IMAGE HR_TEST_DOCKER_RUNNING HR_TEST_DOCKER_FAIL_UP \
        HR_TEST_DOCKER_RUN HR_TEST_DOCKER_VOLUMES HR_CLAUDE_PROJECT HR_CLAUDE_MASK
}

# Per-test baseline for the non-Docker cases.
hr_setup() {
    hr_base_setup || return 1
    hr_clear_env
    export HOME="$BATS_TEST_TMPDIR/home"
    export HR_HOST_REPO="$BATS_TEST_TMPDIR/host"
    export HR_ROOT="$BATS_TEST_TMPDIR/hdd"
    export HR_SSD_ROOT="$BATS_TEST_TMPDIR/ssd"
    export HR_RUNS="$BATS_TEST_TMPDIR/runs"
    export HR_DOCKER="$BATS_TEST_TMPDIR/bin/docker"
    export HR_TEST_DOCKER_LOG="$BATS_TEST_TMPDIR/docker.log"
    export HR_TEST_DOCKER_VOLUMES="$BATS_TEST_TMPDIR/volumes"
    local p
    for p in "$HOME" "$HR_HOST_REPO" "$HR_ROOT" "$HR_SSD_ROOT" "$HR_RUNS" "$HR_STATE" "$HR_DOCKER" \
        "$HR_TEST_DOCKER_LOG" "$HR_TEST_DOCKER_VOLUMES"; do
        hr_assert_tmp_path "$p" || return 1
    done
    export GIT_CONFIG_NOSYSTEM=1
    export HR_MISE_PATH=/usr/bin
    export HR_INTERACTIVE=0

    hr_fake_home "$HOME" || return 1
    make_fixture "$HR_HOST_REPO" || return 1
    # As on the real host, HEAD carries the validator `collect --merge` takes from it (review H6).
    local wf=.claude/skills/handoff-run/scripts f
    mkdir -p "$HR_HOST_REPO/$wf"
    for f in validate-workflow-docs.sh workflow-patterns.sh; do
        cp "$HR_WF_SCRIPTS/$f" "$HR_HOST_REPO/$wf/$f" || return 1
    done
    git -C "$HR_HOST_REPO" add -- "$wf" && git -C "$HR_HOST_REPO" commit -q -m 'workflow scripts' || return 1
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
    mkdir -p "$h/.claude" "$h/.local/bin" "$h/.local/share/mise/installs" "$h/.local/share/uv/python" \
        "$h/.local/share/uv/tools" "$h/.gortex/instructions" "$h/.gortex/models" "$h/.config/git"
    printf '{"fake": "claude.json"}\n' >"$h/.claude.json"
    chmod 644 "$h/.claude.json"
    printf 'fake: gortex\n' >"$h/.gortex/config.yaml"
    printf '**/.claude/settings.local.json\n*.fake-global-ignore\n' >"$h/.config/git/ignore"
}

# The stub docker: logs every call's argv to $HR_TEST_DOCKER_LOG, and on `compose` the HR_* env it
# received to $HR_TEST_DOCKER_LOG.env. Knobs: HR_TEST_DOCKER_IMAGE=missing|mismatch,
# HR_TEST_DOCKER_RUNNING="hr-mod-5 ..." (projects whose `ps` is non-empty),
# HR_TEST_DOCKER_FAIL_UP=1 (`compose ... up` exits 1).
# `docker run` (hr's throwaway status-check container) is emulated on the host by default: the
# -v SRC:DST mount becomes a path swap (an argument equal to DST, and -w, map to SRC), then the
# entrypoint runs here; a named volume (`-v htui-hr-cargo:/v`) is the directory
# $HR_TEST_DOCKER_VOLUMES/<name>, which `volume create` makes. Only for benign fixture clones —
# HR_TEST_DOCKER_RUN=clean answers "clean"
# (exit 0) without running anything, =dirty prints a canned finding (exit 1), =fail exits 125.
hr_stub_docker() {
    mkdir -p "$(dirname "$HR_DOCKER")"
    cat >"$HR_DOCKER" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$HR_TEST_DOCKER_LOG"
if [[ "$1" == run ]]; then
    shift
    src='' dst='' wd='' ep=''
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --rm) shift ;;
            --pull | --name | --network | --cap-drop | --security-opt | -u) shift 2 ;;
            -v) IFS=: read -r src dst _ <<<"$2"
                # A named volume is a directory under $HR_TEST_DOCKER_VOLUMES.
                [[ "$src" == /* ]] || src="$HR_TEST_DOCKER_VOLUMES/$src"
                shift 2 ;;
            -w) wd="$2"; shift 2 ;;
            --entrypoint) ep="$2"; shift 2 ;;
            -*) echo "stub docker run: unexpected option $1" >&2; exit 125 ;;
            *) break ;;
        esac
    done
    shift # the image
    case "${HR_TEST_DOCKER_RUN:-emulate}" in
        clean) exit 0 ;;
        dirty) echo "uncommitted changes in $dst:"; echo "   M canned.txt"; exit 1 ;;
        fail) echo "stub: docker run failed" >&2; exit 125 ;;
    esac
    args=()
    for a in "$@"; do
        [[ "$a" == "$dst" ]] && a="$src"
        args+=("$a")
    done
    cd "$src${wd#"$dst"}" || exit 125
    exec "$ep" "${args[@]}"
fi
case "$1 ${2:-}" in
    "volume create") mkdir -p "$HR_TEST_DOCKER_VOLUMES/$3"; exit 0 ;;
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

# compose_config — compose.hr.yaml as `docker compose config --format json` resolves it, with every
# `${X:?}` set to "/X" (HR_HOME=/HR_HOME, ...). Only the real docker CLI's parser runs: no daemon.
compose_config() {
    command -v docker >/dev/null 2>&1 || skip "docker CLI not installed"
    local v
    local -a e=()
    for v in $(grep -oE '\$\{[A-Z_]+:\?\}' "$HR_COMPOSE_FILE" | tr -d '${}:?' | sort -u); do
        case "$v" in
            HR_CLAUDE_PROJECT) e+=("$v=-$v") ;;   # a directory name, not a path
            *) e+=("$v=/$v") ;;
        esac
    done
    env -i PATH="$PATH" HOME="$HOME" "${e[@]}" docker compose -p hr-contract -f "$HR_COMPOSE_FILE" \
        config --format json
}

# claude_project_of PATH — Claude Code's ~/.claude/projects/<name> for a working directory.
claude_project_of() { local LC_ALL=C p="$1"; printf '%s' "${p//[^a-zA-Z0-9]/-}"; }

# The ~/.claude entries every run masks (re-review M-C): tmpfs dirs, per-run empty files.
HR_CLAUDE_MASK_DIRS=(projects file-history paste-cache shell-snapshots session-env session-data sessions
    jobs daemon backups metrics .remember)
HR_CLAUDE_MASK_FILES=(history.jsonl bash-commands.log cost-tracker.log)

# src_of ITEM [ROOT] — the run's clone.
src_of() { printf '%s/%s/src' "${2:-$HR_ROOT}" "$1"; }

# reg_of ITEM — the run's registry file (host-only HR_RUNS, never under the mounted HR_STATE).
reg_of() { printf '%s/%s' "$HR_RUNS" "$1"; }

# reg_set ITEM KEY VALUE — overwrite one registry line in place (a tampered registry).
reg_set() {
    local f
    f="$(reg_of "$1")"
    hr_assert_tmp_path "$f" || return 1
    KEY="$2" VAL="$3" awk -F= 'BEGIN { k = ENVIRON["KEY"]; v = ENVIRON["VAL"] }
        $1 == k { print k "=" v; next } { print }' "$f" >"$f.new" && mv "$f.new" "$f"
}

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

# hr_docker_setup [--no-image] ITEM... — real docker + real HOME; fixture host repo with the given
# open items (TOOL-9xxx so no real run's project name can collide); HR_ROOT/HR_STATE under the
# tmpdir. Skips unless the image is built (--no-image: the case builds it itself).
hr_docker_setup() {
    hr_base_setup || return 1
    local need_image=1
    [[ "${1:-}" == --no-image ]] && { need_image=0; shift; }
    docker info >/dev/null 2>&1 || skip "docker daemon not reachable"
    if [[ $need_image -eq 1 ]]; then
        docker image inspect "${HR_IMAGE:-htui-hr-dev}" >/dev/null 2>&1 || skip "image htui-hr-dev not built (scripts/hr build)"
    fi
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
    export HR_RUNS="$BATS_TEST_TMPDIR/runs"
    export HR_INTERACTIVE=0
    local p
    for p in "$HR_HOST_REPO" "$HR_ROOT" "$HR_SSD_ROOT" "$HR_RUNS" "$HR_STATE"; do
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
    # `up` makes the fixture's (empty) project dir in ~/.claude/projects as the mount point of the
    # one project a run keeps; remove it again (rmdir: never anything with content in it).
    local pd
    pd="$HOME/.claude/projects/$(claude_project_of "$BATS_TEST_TMPDIR/host")"
    [[ -n "${HR_DOCKER_ITEMS[*]:-}" && -d "$pd" && ! -L "$pd" ]] && rmdir -- "$pd" 2>/dev/null
    return 0
}

# dexec ITEM CMD... — run a command in the run's dev container (no TTY).
dexec() {
    local item="$1"
    shift
    docker exec "hr-${item,,}-dev-1" "$@"
}
