#!/usr/bin/env bash
# gate.sh - the canonical htui gate (TOOL-9, ANA-29 D5). One sequence, so a green means the
# same thing everywhere: fmt, clippy with and without --all-features, the whole workspace test
# suite single-threaded without fail-fast, a scan of that output for aborts, the migration
# numbering, the workflow validator, and the htui-dev mod when it exists. Bash twin of gate.ps1
# - keep the two in lockstep (same step names, same last.json shape).
#
# Usage:
#   bash scripts/gate.sh [--only <step>] [--detach] [--status] [--self-test]
#
#   --only <step>  run one step (fmt, clippy-all, clippy-bare, test, abort-grep, migrations,
#                  workflow-docs, plugin)
#   --detach       relaunch under setsid, log to target/gate/run.log, print the PID and return
#   --status       print target/gate/last.json and exit
#
# Every step runs even after one fails. Exit 0 all passed or skipped, 1 a step failed, 2 usage.
# Result: target/gate/last.json, written as "running" at the start and replaced atomically.
# The test step's output goes to target/gate/test.log; abort-grep reads it, because a stack
# overflow aborts a test binary with no failure counted.

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
REPO_ROOT="${GATE_REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
GATE_DIR="$REPO_ROOT/target/gate"
LAST_JSON="$GATE_DIR/last.json"
TEST_LOG="${GATE_TEST_LOG:-$GATE_DIR/test.log}"
MIGRATIONS_DIR="${GATE_MIGRATIONS_DIR:-$REPO_ROOT/crates/htui-store/migrations}"
PLUGIN_DIR="$REPO_ROOT/.claude/skills/htui-dev"
MIN_CLAUDE="2.1.287"
STEPS=(fmt clippy-all clippy-bare test abort-grep migrations workflow-docs plugin)

STEP_NAMES=(); STEP_STATUS=(); STEP_SECONDS=()
STARTED=""

now_iso() { date -u +%Y-%m-%dT%H:%M:%SZ; }

json_str() {
    # A JSON string literal for $1 (backslash and quote escaped; step names and shas only).
    local s="${1//\\/\\\\}"
    printf '"%s"' "${s//\"/\\\"}"
}

write_last() {
    # $1=state ($2=finished iso, optional). Atomic: temp file in the same directory, then rename.
    local state="$1" finished="${2:-}" head i sep=""
    head="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
    mkdir -p "$GATE_DIR"
    {
        printf '{"head":%s,"started":%s,"finished":' "$(json_str "$head")" "$(json_str "$STARTED")"
        if [[ -n "$finished" ]]; then json_str "$finished"; else printf 'null'; fi
        printf ',"state":%s,"steps":[' "$(json_str "$state")"
        for ((i = 0; i < ${#STEP_NAMES[@]}; i++)); do
            printf '%s{"name":%s,"status":%s,"seconds":%s}' "$sep" \
                "$(json_str "${STEP_NAMES[i]}")" "$(json_str "${STEP_STATUS[i]}")" "${STEP_SECONDS[i]}"
            sep=","
        done
        printf ']}\n'
    } > "$LAST_JSON.tmp" && mv "$LAST_JSON.tmp" "$LAST_JSON"
}

version_ge() {
    # $1 >= $2 as dotted numbers.
    [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

# ---- steps: each returns 0 pass, 1 fail, 3 skipped (message on stdout) ----

step_fmt()         { cargo fmt --check; }
step_clippy_all()  { cargo clippy --workspace --all-features -- -D warnings; }
step_clippy_bare() { cargo clippy --workspace -- -D warnings; }

step_test() {
    mkdir -p "$GATE_DIR"
    # PIPESTATUS, not $?: tee would otherwise report success for a failed cargo.
    cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | tee "$TEST_LOG"
    return "${PIPESTATUS[0]}"
}

step_abort_grep() {
    if [[ ! -f "$TEST_LOG" ]]; then
        echo "abort-grep: no $TEST_LOG - run the test step first"
        return 1
    fi
    if grep -n -E 'SIGABRT|overflowed its stack' "$TEST_LOG"; then
        echo "abort-grep: a test binary aborted; its failures were not counted"
        return 1
    fi
    echo "abort-grep: no aborts in $TEST_LOG"
}

step_migrations() {
    # NNNN_name.sql files: the prefixes must be unique and run 0001..N with no gap. A merge of
    # two branches that each added the next number is the failure this catches.
    local f base num expected=1 seen="" bad=0
    local -a nums=()
    shopt -s nullglob
    for f in "$MIGRATIONS_DIR"/*.sql; do
        base="$(basename "$f")"
        if [[ ! "$base" =~ ^([0-9]{4})_[a-z0-9_]+\.sql$ ]]; then
            echo "migrations: $base does not match NNNN_name.sql"
            bad=1
            continue
        fi
        nums+=("$((10#${BASH_REMATCH[1]}))")
    done
    shopt -u nullglob
    for num in $(printf '%s\n' "${nums[@]}" | sort -n); do
        if [[ " $seen " == *" $num "* ]]; then
            echo "migrations: number $num is used twice"
            bad=1
            continue
        fi
        seen="$seen $num"
        if [[ $num -ne $expected ]]; then
            echo "migrations: expected $(printf '%04d' "$expected"), found $(printf '%04d' "$num")"
            bad=1
            expected=$num
        fi
        expected=$((expected + 1))
    done
    [[ ${#nums[@]} -gt 0 ]] || { echo "migrations: no migrations under $MIGRATIONS_DIR"; return 1; }
    [[ $bad -eq 0 ]] && echo "migrations: ${#nums[@]} contiguous, unique"
    return "$bad"
}

step_workflow_docs() {
    bash "$REPO_ROOT/.claude/skills/handoff-run/scripts/validate-workflow-docs.sh" --repo-root "$REPO_ROOT"
}

step_plugin() {
    local ver
    if [[ ! -d "$PLUGIN_DIR" ]]; then echo "plugin: $PLUGIN_DIR not present (TOOL-8)"; return 3; fi
    if ! command -v claude >/dev/null 2>&1; then echo "plugin: claude not on PATH"; return 3; fi
    ver="$(claude --version 2>/dev/null | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -n1)"
    if [[ -z "$ver" ]] || ! version_ge "$ver" "$MIN_CLAUDE"; then
        echo "plugin: claude ${ver:-unknown} is older than $MIN_CLAUDE"
        return 3
    fi
    claude plugin validate --strict "$PLUGIN_DIR" && claude plugin test "$PLUGIN_DIR"
}

run_step() {
    # $1=step name -> appends to STEP_*, updates last.json, returns the step's verdict.
    local name="$1" fn start rc status
    fn="step_${name//-/_}"
    echo "== gate: $name"
    start=$SECONDS
    "$fn"; rc=$?
    case $rc in
        0) status=passed ;;
        3) status=skipped; rc=0 ;;
        *) status=failed; rc=1 ;;
    esac
    STEP_NAMES+=("$name"); STEP_STATUS+=("$status"); STEP_SECONDS+=("$((SECONDS - start))")
    echo "== gate: $name $status"
    write_last running
    return "$rc"
}

run_gate() {
    local only="$1" name failed=0 list=("${STEPS[@]}")
    [[ -n "$only" ]] && list=("$only")
    STARTED="$(now_iso)"
    cd "$REPO_ROOT" || exit 2
    write_last running
    for name in "${list[@]}"; do
        run_step "$name" || failed=1
    done
    if [[ $failed -eq 0 ]]; then write_last passed "$(now_iso)"; else write_last failed "$(now_iso)"; fi
    if [[ $failed -eq 0 ]]; then
        # A skipped step is not a verified one: name them so a green cannot hide a missing check.
        local skipped=() i
        for ((i = 0; i < ${#STEP_NAMES[@]}; i++)); do
            [[ "${STEP_STATUS[i]}" == skipped ]] && skipped+=("${STEP_NAMES[i]}")
        done
        if [[ ${#skipped[@]} -gt 0 ]]; then
            echo "== gate: PASSED (${#skipped[@]} skipped: ${skipped[*]})"
        else
            echo "== gate: PASSED"
        fi
    else
        echo "== gate: FAILED"
    fi
    return "$failed"
}

# ---- self-test ----

run_self_test() {
    local tmp rc out
    local -a failures=()
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/gate-selftest-XXXXXX")"

    mig() {
        # mig <case> <names...> -> a migrations dir holding empty files with those names.
        local d="$tmp/$1" n; shift
        mkdir -p "$d"; for n in "$@"; do : > "$d/$n"; done
        printf '%s' "$d"
    }

    MIGRATIONS_DIR="$(mig ok 0001_a.sql 0002_b.sql 0003_c.sql)"
    step_migrations >/dev/null || failures+=('migrations: a contiguous set was refused')

    MIGRATIONS_DIR="$(mig dup 0001_a.sql 0002_b.sql 0002_c.sql)"
    step_migrations >/dev/null && failures+=('migrations: a duplicated number passed')

    MIGRATIONS_DIR="$(mig gap 0001_a.sql 0003_c.sql)"
    step_migrations >/dev/null && failures+=('migrations: a gap passed')

    MIGRATIONS_DIR="$(mig badname 0001_a.sql 2_b.sql)"
    step_migrations >/dev/null && failures+=('migrations: a malformed name passed')

    MIGRATIONS_DIR="$(mig empty)"
    step_migrations >/dev/null && failures+=('migrations: an empty directory passed')

    TEST_LOG="$tmp/clean.log"; echo "test result: ok. 3 passed" > "$TEST_LOG"
    step_abort_grep >/dev/null || failures+=('abort-grep: a clean log failed')
    TEST_LOG="$tmp/abort.log"; echo "thread 'x' has overflowed its stack" > "$TEST_LOG"
    step_abort_grep >/dev/null && failures+=('abort-grep: a stack overflow passed')
    TEST_LOG="$tmp/sigabrt.log"; echo "process didn't exit successfully (signal: 6, SIGABRT: process abort signal)" > "$TEST_LOG"
    step_abort_grep >/dev/null && failures+=('abort-grep: SIGABRT passed')
    TEST_LOG="$tmp/missing.log"
    step_abort_grep >/dev/null && failures+=('abort-grep: a missing log passed')

    version_ge 2.1.296 2.1.287 || failures+=('version_ge: 2.1.296 < 2.1.287')
    version_ge 2.1.287 2.1.287 || failures+=('version_ge: equal versions refused')
    version_ge 2.1.9 2.1.287 && failures+=('version_ge: 2.1.9 compared as >= 2.1.287')
    version_ge 10.0.0 2.1.287 || failures+=('version_ge: 10.0.0 compared as < 2.1.287')

    # last.json is valid JSON with the documented shape and no .tmp left behind.
    GATE_DIR="$tmp/gate"; LAST_JSON="$GATE_DIR/last.json"; STARTED="2026-01-01T00:00:00Z"
    STEP_NAMES=(fmt test); STEP_STATUS=(passed failed); STEP_SECONDS=(1 2)
    write_last failed "2026-01-01T00:00:03Z"
    out="$(jq -r '[.state, (.steps | length), .steps[1].status, .finished] | join(",")' "$LAST_JSON" 2>&1)"
    [[ "$out" == "failed,2,failed,2026-01-01T00:00:03Z" ]] || failures+=("last.json shape: $out")
    [[ ! -e "$LAST_JSON.tmp" ]] || failures+=('last.json: temp file left behind')
    write_last running
    [[ "$(jq -r '.finished' "$LAST_JSON")" == "null" ]] || failures+=('last.json: running state has a finished time')

    rm -rf "$tmp"
    rc=0
    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: all cases PASS.'
    else
        echo "Self-test FAIL (${#failures[@]}):"
        local f; for f in "${failures[@]}"; do echo "  - $f"; done
        rc=1
    fi
    return "$rc"
}

# ---- main ----

usage() { echo "Usage: $0 [--only <step>] [--detach] [--status] [--self-test]"; }

ONLY=""; DETACH=0; STATUS=0; SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --only) ONLY="${2:-}"; shift 2 || { usage >&2; exit 2; } ;;
        --detach) DETACH=1; shift ;;
        --status) STATUS=1; shift ;;
        --self-test) SELF_TEST=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ -n "$ONLY" ]]; then
    case " ${STEPS[*]} " in
        *" $ONLY "*) ;;
        *) echo "Unknown step: $ONLY (steps: ${STEPS[*]})" >&2; exit 2 ;;
    esac
fi

if [[ $SELF_TEST -eq 1 ]]; then run_self_test; exit $?; fi

if [[ $STATUS -eq 1 ]]; then
    if [[ -f "$LAST_JSON" ]]; then cat "$LAST_JSON"; exit 0; fi
    echo "no gate result yet ($LAST_JSON)" >&2
    exit 1
fi

if [[ $DETACH -eq 1 ]]; then
    mkdir -p "$GATE_DIR"
    args=(); [[ -n "$ONLY" ]] && args=(--only "$ONLY")
    setsid bash "${BASH_SOURCE[0]}" "${args[@]}" > "$GATE_DIR/run.log" 2>&1 < /dev/null &
    echo "gate started, pid $! - log $GATE_DIR/run.log, result: bash scripts/gate.sh --status"
    exit 0
fi

run_gate "$ONLY"
exit $?
