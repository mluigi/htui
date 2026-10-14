#!/usr/bin/env bash
# dev-health.sh - the two dev-machine facts the htui-dev status line shows that have no portable
# binary (TOOL-8 M2, ANA-29 D6): how full the repo's disk is, and whether the dev Postgres is up.
# Bash twin of dev-health.ps1 - keep the two in lockstep (same JSON line, same exit codes).
#
# Usage:
#   bash scripts/dev-health.sh [--self-test]
#
# Prints one JSON line and exits 0, whatever Postgres answered:
#   {"disk":{"path":"<repo root>","used_pct":80},
#    "postgres":{"state":"up|recovering|down|unknown","via":"pg_isready|docker|tcp","target":"host:port"}}
# used_pct is null when df cannot say. Exit 2 is usage only.
#
# Postgres is the host and port of HTUI_TEST_DATABASE_URL, else localhost:5439 (compose.yaml).
# The first probe that exists answers: pg_isready (0 up, 1 recovering, 2 down, else unknown), then,
# for a local target only, the htui-postgres container (the host has no psql client): stopped is
# down, running is `docker exec htui-postgres pg_isready`. Last, a 3 s TCP connect, which can only
# tell up from down. The disk is the repo root's filesystem, never `/`: in an hr
# sandbox `/` is the container overlay and the repo lives on another disk.

set -u

# Windows runs the .ps1 twin, never this one - see install-workflow-hooks.sh for why.
# Override, for a deliberate twin-parity check only: WORKFLOW_ALLOW_SH_ON_WINDOWS=1.
case "$(uname -s 2>/dev/null || echo unknown)" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
        if [[ "${WORKFLOW_ALLOW_SH_ON_WINDOWS:-0}" != "1" ]]; then
            echo "$(basename "${BASH_SOURCE[0]}"): refusing to run on Windows - use the .ps1 twin:" >&2
            echo "    pwsh -NoProfile -File \"${BASH_SOURCE[0]%.sh}.ps1\" [same arguments]" >&2
            echo "  (override for parity checks only: WORKFLOW_ALLOW_SH_ON_WINDOWS=1)" >&2
            exit 2
        fi
        ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${DEV_HEALTH_REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
CONTAINER="htui-postgres"

PG_HOST=""; PG_PORT=""; PG_STATE=""; PG_VIA=""

json_str() {
    # A JSON string literal for $1 (backslash and quote escaped).
    local s="${1//\\/\\\\}"
    printf '"%s"' "${s//\"/\\\"}"
}

parse_dsn() {
    # $1=DSN -> PG_HOST, PG_PORT. No DSN, or no postgres:// scheme: compose.yaml's localhost:5439.
    # A DSN with no port gets libpq's 5432. user:password@ is dropped up to the last @, a query or
    # fragment is cut, a multi-host list keeps its first host, and an IPv6 host loses its brackets.
    local dsn="${1:-}" rest host="" port=""
    PG_HOST=localhost; PG_PORT=5439
    [[ "$dsn" =~ ^postgres(ql)?:// ]] || return 0
    rest="${dsn#*://}"
    rest="${rest%%[?#]*}"
    rest="${rest%%/*}"
    rest="${rest##*@}"
    rest="${rest%%,*}"
    if [[ "$rest" =~ ^\[([^]]*)\](:([0-9]+))?$ ]]; then
        host="${BASH_REMATCH[1]}"; port="${BASH_REMATCH[3]}"
    elif [[ "$rest" =~ ^(.*):([0-9]+)$ ]]; then
        host="${BASH_REMATCH[1]}"; port="${BASH_REMATCH[2]}"
    else
        host="$rest"
    fi
    PG_HOST="${host:-localhost}"; PG_PORT="${port:-5432}"
}

target_of() {
    # host:port, an IPv6 host bracketed.
    if [[ "$PG_HOST" == *:* ]]; then printf '[%s]:%s' "$PG_HOST" "$PG_PORT"; else printf '%s:%s' "$PG_HOST" "$PG_PORT"; fi
}

disk_pct() {
    # The use of $1's filesystem as a bare integer, or nothing when df cannot say.
    df -P "$1" 2>/dev/null | awk 'NR==2 { sub("%", "", $5); if ($5 ~ /^[0-9]+$/) print $5 }'
}

map_isready() {
    # pg_isready's exit code -> a state.
    case "$1" in
        0) echo up ;;
        1) echo recovering ;;
        2) echo down ;;
        *) echo unknown ;;
    esac
}

tcp_up() {
    # $1=host $2=port: whether a TCP connect succeeds within 3 s.
    if command -v timeout >/dev/null 2>&1; then
        timeout 3 bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$1" "$2" >/dev/null 2>&1
    else
        bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$1" "$2" >/dev/null 2>&1
    fi
}

is_local() {
    # Whether $1 names this machine.
    case "$1" in localhost|127.0.0.1|::1) return 0 ;; *) return 1 ;; esac
}

pg_probe() {
    # -> PG_STATE, PG_VIA for PG_HOST:PG_PORT.
    local rc
    if command -v pg_isready >/dev/null 2>&1; then
        pg_isready -h "$PG_HOST" -p "$PG_PORT" -t 3 >/dev/null 2>&1; rc=$?
        PG_STATE="$(map_isready "$rc")"; PG_VIA=pg_isready
        return
    fi
    # The container answers only for a local target. Its state first: `docker exec` on a missing or
    # stopped container exits 1, which would read as recovering.
    local running
    if is_local "$PG_HOST" && command -v docker >/dev/null 2>&1 \
        && running="$(docker container inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null)"; then
        PG_VIA=docker
        if [[ "$running" != true ]]; then PG_STATE=down; return; fi
        docker exec "$CONTAINER" pg_isready -t 3 >/dev/null 2>&1; rc=$?
        PG_STATE="$(map_isready "$rc")"
        return
    fi
    if tcp_up "$PG_HOST" "$PG_PORT"; then PG_STATE=up; else PG_STATE=down; fi
    PG_VIA=tcp
}

emit_json() {
    # $1=path $2=used_pct (empty or "null" -> null) $3=state $4=via $5=target
    local pct="${2:-null}"
    printf '{"disk":{"path":%s,"used_pct":%s},"postgres":{"state":%s,"via":%s,"target":%s}}\n' \
        "$(json_str "$1")" "$pct" "$(json_str "$3")" "$(json_str "$4")" "$(json_str "$5")"
}

# ---- self-test ----

run_self_test() {
    local -a failures=()
    local got out pct

    dsn_case() {
        # dsn_case <dsn> <expected host:port>
        parse_dsn "$1"
        got="$PG_HOST:$PG_PORT"
        [[ "$got" == "$2" ]] || failures+=("parse_dsn '$1': expected $2, got $got")
    }
    dsn_case '' 'localhost:5439'
    dsn_case 'not a dsn' 'localhost:5439'
    dsn_case 'postgres://u:p@localhost:5439/postgres' 'localhost:5439'
    dsn_case 'postgres://postgres@localhost:5439/postgres' 'localhost:5439'
    dsn_case 'postgres://localhost/db' 'localhost:5432'
    dsn_case 'postgresql://u@db.example:6543/x?sslmode=disable#f' 'db.example:6543'
    dsn_case 'postgres://u:p@x@h:1/d' 'h:1'
    dsn_case 'postgres://[::1]:5440/d' '::1:5440'
    dsn_case 'postgres://h1:1,h2:2/d' 'h1:1'
    dsn_case 'postgres:///db?host=/run/pg' 'localhost:5432'
    dsn_case 'POSTGRES://h:1/d' 'localhost:5439'

    parse_dsn 'postgres://[::1]:5440/d'
    [[ "$(target_of)" == '[::1]:5440' ]] || failures+=("target_of: IPv6 not bracketed: $(target_of)")

    local rc want
    for rc in 0 1 2 3 127; do
        case $rc in 0) want=up ;; 1) want=recovering ;; 2) want=down ;; *) want=unknown ;; esac
        [[ "$(map_isready "$rc")" == "$want" ]] || failures+=("map_isready $rc: expected $want")
    done

    out="$(emit_json '/a "b"\c' 80 up pg_isready 'localhost:5439')"
    jq -e '(.disk.path == "/a \"b\"\\c") and (.disk.used_pct == 80)
        and ((.postgres | keys) == ["state", "target", "via"]) and (.postgres.state == "up")' \
        <<<"$out" >/dev/null 2>&1 || failures+=("emit_json shape: $out")
    out="$(emit_json /x '' unknown none 'localhost:5439')"
    jq -e '.disk.used_pct == null' <<<"$out" >/dev/null 2>&1 || failures+=("emit_json null pct: $out")

    pct="$(disk_pct "$REPO_ROOT")"
    [[ "$pct" =~ ^[0-9]+$ ]] && ((pct <= 100)) || failures+=("disk_pct: not a percentage: '$pct'")

    tcp_up 127.0.0.1 1 && failures+=('tcp_up: 127.0.0.1:1 answered')

    is_local localhost && is_local ::1 && ! is_local db.example || failures+=('is_local: wrong answer')

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: all cases PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f; for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() { echo "Usage: $0 [--self-test]"; }

case "${1:-}" in
    '') ;;
    --self-test) run_self_test; exit $? ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
esac

parse_dsn "${HTUI_TEST_DATABASE_URL:-}"
pg_probe
emit_json "$REPO_ROOT" "$(disk_pct "$REPO_ROOT")" "$PG_STATE" "$PG_VIA" "$(target_of)"
