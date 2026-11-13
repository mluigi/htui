#!/usr/bin/env bash
# gate.sh - the canonical htui gate (TOOL-9, ANA-29 D5). One sequence, so a green means the
# same thing everywhere: fmt, clippy with and without --all-features, the whole workspace test
# suite under cargo-nextest without fail-fast and then its doctests (TOOL-17), a scan of that output for aborts,
# the migration numbering, the workflow validator, and the htui-dev mod when it exists. Bash twin of gate.ps1
# - keep the two in lockstep (same step names, same last.json shape).
#
# Usage:
#   bash scripts/gate.sh [--only <step>] [--detach] [--status] [--self-test]
#
#   --only <step>  run one step (fmt, clippy-all, clippy-bare, postgres, qdrant, test,
#                  abort-grep, migrations, workflow-docs, plugin)
#   --detach       relaunch under setsid, log to target/gate/run.log, print the PID and return
#   --status       print target/gate/last.json and exit
#
# Every step runs even after one fails. Exit 0 all passed or skipped, 1 a step failed, 2 usage.
# Result: target/gate/last.json, written as "running" at the start and replaced atomically.
# The test step's output goes to target/gate/test.log; abort-grep reads it for a crash (stack
# overflow, SIGABRT). Under cargo test a crash ends the test's whole binary with no failure counted
# for the rest of it; under nextest it fails that one test, which may then pass on its one retry
# (FLAKY) while the abort text stays in the log. abort-grep keeps a crash red even when the retry
# passes, deliberately: a crash is a bug, not a flake. The test step needs cargo-nextest and fails
# without it (TOOL-17): there is no fallback to cargo test, which would be a different green.

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
STEPS=(fmt clippy-all clippy-bare postgres qdrant test abort-grep migrations workflow-docs plugin)
# The pinned cargo-nextest (TOOL-17 D3): the test step prints its install line when none is on PATH.
NEXTEST_VERSION="0.9.151"

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

step_postgres() {
    # TOOL-14: without HTUI_TEST_DATABASE_URL every Postgres suite returns early and passes, so
    # the test step's green says nothing about Postgres. Say so as a skipped step. Never print
    # the DSN: it may carry a password.
    local dsn="${HTUI_TEST_DATABASE_URL:-}" host port
    if [[ -z "$dsn" ]]; then
        echo "postgres: HTUI_TEST_DATABASE_URL unset - the Postgres suites return early and prove nothing"
        return 3
    fi
    # The userinfo runs to the last '@' before the path, and the host holds no '@': a password
    # with an unencoded '@' stays out of the output.
    if [[ ! "$dsn" =~ ^postgres(ql)?://([^/?#]*@)?([^:/?#@]+)(:([0-9]+))? ]]; then
        echo "postgres: DSN set, host not parsed (not probed)"
        return 0
    fi
    host="${BASH_REMATCH[3]}"; port="${BASH_REMATCH[5]:-5432}"
    if ! timeout 3 bash -c ">/dev/tcp/$host/$port" 2>/dev/null; then
        echo "postgres: $host:$port unreachable - the Postgres suites would fail"
        return 1
    fi
    echo "postgres: $host:$port reachable; the Postgres suites run live"
}

step_qdrant() {
    # MOD-160 D10: the Qdrant suites return early without HTUI_TEST_QDRANT_URL, as the Postgres
    # ones do without a DSN. Same verdicts as step_postgres. Never print more than host:port: the
    # URL may carry credentials.
    local url="${HTUI_TEST_QDRANT_URL:-}" host port
    if [[ -z "$url" ]]; then
        echo "qdrant: HTUI_TEST_QDRANT_URL unset - the Qdrant suites return early and prove nothing"
        return 3
    fi
    # Userinfo to the last '@', no '@' in the host: as step_postgres.
    if [[ ! "$url" =~ ^(https?)://([^/?#]*@)?(\[[^]/]+\]|[^:/?#@]+)(:([0-9]+))? ]]; then
        echo "qdrant: URL set, host not parsed (not probed)"
        return 0
    fi
    host="${BASH_REMATCH[3]}"; host="${host#[}"; host="${host%]}"
    # The port the client would dial: the URL's, else the scheme's.
    port="${BASH_REMATCH[5]:-}"
    [[ -n "$port" ]] || { [[ "${BASH_REMATCH[1]}" == https ]] && port=443 || port=80; }
    if ! timeout 3 bash -c ">/dev/tcp/$host/$port" 2>/dev/null; then
        echo "qdrant: $host:$port unreachable - the Qdrant suites would fail"
        return 1
    fi
    echo "qdrant: $host:$port reachable; the Qdrant suites run live"
}

step_test() {
    # TOOL-17 (D6): cargo-nextest runs each test in its own process, so tests no longer share one
    # per binary; the live Qdrant cases take turns through the live-qdrant test group in
    # .config/nextest.toml. Profile `gate` retries a failure once and names a pass on retry FLAKY.
    # nextest runs no doctests, so cargo test --doc follows into the same log. Red if either is.
    # No cargo-nextest: fail before running anything, with the install line. Not in an hr sandbox:
    # there ~/.cargo/bin is an empty read-only tmpfs off PATH (docker/hr/compose.hr.yaml), and the
    # image carries nextest, so a sandbox without it runs an image built before TOOL-17.
    local rc_nextest rc_doc
    if ! command -v cargo-nextest >/dev/null 2>&1; then
        if [[ "${HR_SANDBOX:-}" == 1 ]]; then
            echo "test: cargo-nextest not installed - this hr image predates TOOL-17; rebuild it on the host with scripts/hr build"
        else
            echo "test: cargo-nextest not installed - curl -LsSf https://get.nexte.st/$NEXTEST_VERSION/linux | tar zxf - -C \"${CARGO_HOME:-$HOME/.cargo}/bin\""
        fi
        return 1
    fi
    mkdir -p "$GATE_DIR"
    # PIPESTATUS, not $?: tee would otherwise report success for a failed cargo.
    cargo nextest run --workspace --all-features --no-fail-fast --profile gate 2>&1 | tee "$TEST_LOG"
    rc_nextest="${PIPESTATUS[0]}"
    cargo test --workspace --all-features --doc 2>&1 | tee -a "$TEST_LOG"
    rc_doc="${PIPESTATUS[0]}"
    [[ $rc_nextest -eq 0 && $rc_doc -eq 0 ]]
}

step_abort_grep() {
    if [[ ! -f "$TEST_LOG" ]]; then
        echo "abort-grep: no $TEST_LOG - run the test step first"
        return 1
    fi
    if grep -n -E 'SIGABRT|overflowed its stack' "$TEST_LOG"; then
        echo "abort-grep: a test aborted. Under nextest it may have passed on retry; under cargo test the rest of its binary did not run"
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

    # TOOL-17 D6: without cargo-nextest the test step fails at once and prints the install line,
    # into $CARGO_HOME/bin. PATH holds only a stand-in cargo that leaves a mark, so a broken check
    # cannot start a real run.
    mkdir -p "$tmp/bin"
    printf '#!/bin/sh\n: > "%s/cargo-ran"\n' "$tmp" > "$tmp/bin/cargo"
    chmod +x "$tmp/bin/cargo"
    out="$(PATH="$tmp/bin"; HR_SANDBOX=""; CARGO_HOME="$tmp/cargo-home"; GATE_DIR="$tmp/nextest"
        TEST_LOG="$tmp/nextest.log"; step_test 2>&1)"
    rc=$?
    [[ $rc -eq 1 ]] || failures+=("test: a missing cargo-nextest returned $rc, not a failure")
    [[ "$out" == *"test: cargo-nextest not installed - "*"https://get.nexte.st/0.9.151/"*"$tmp/cargo-home/bin"* ]] \
        || failures+=('test: a missing cargo-nextest printed no install line into CARGO_HOME/bin')
    [[ ! -e "$tmp/cargo-ran" ]] || failures+=('test: a missing cargo-nextest still ran cargo')
    # In an hr sandbox nothing can be installed: the message names the image rebuild instead.
    out="$(PATH="$tmp/bin"; HR_SANDBOX=1; GATE_DIR="$tmp/nextest"; TEST_LOG="$tmp/nextest.log"
        step_test 2>&1)"
    rc=$?
    [[ $rc -eq 1 ]] || failures+=("test: a sandbox without cargo-nextest returned $rc, not a failure")
    [[ "$out" == *"predates TOOL-17"*"scripts/hr build"* ]] \
        || failures+=('test: a sandbox without cargo-nextest did not ask for an image rebuild')
    [[ "$out" != *get.nexte.st* ]] || failures+=('test: a sandbox without cargo-nextest printed an install line')
    [[ ! -e "$tmp/cargo-ran" ]] || failures+=('test: a sandbox without cargo-nextest still ran cargo')

    ( unset HTUI_TEST_DATABASE_URL; step_postgres >/dev/null; [[ $? -eq 3 ]] ) \
        || failures+=('postgres: an unset DSN was not skipped')
    ( HTUI_TEST_DATABASE_URL='postgres://u@127.0.0.1:1/db' step_postgres >/dev/null; [[ $? -eq 1 ]] ) \
        || failures+=('postgres: an unreachable host passed')
    ( HTUI_TEST_DATABASE_URL='host=/run/postgresql dbname=x' step_postgres >/dev/null ) \
        || failures+=('postgres: an unparsed DSN failed')
    out="$(HTUI_TEST_DATABASE_URL='postgres://u:s3cret@127.0.0.1:1/db' step_postgres 2>&1)"
    [[ "$out" != *s3cret* ]] || failures+=('postgres: the DSN password was printed')
    # An unencoded '@' in the password: the userinfo ends at the last '@', as the client splits it.
    out="$(HTUI_TEST_DATABASE_URL='postgres://u:p@ss@127.0.0.1:1/db' step_postgres 2>&1)"
    [[ "$out" != *ss@* ]] || failures+=('postgres: a password with an @ was printed')

    # Every step names a function: a missing one would only show up when that step runs.
    local s
    for s in "${STEPS[@]}"; do
        declare -F "step_${s//-/_}" >/dev/null || failures+=("step $s has no step_${s//-/_}")
    done

    ( unset HTUI_TEST_QDRANT_URL; step_qdrant >/dev/null; [[ $? -eq 3 ]] ) \
        || failures+=('qdrant: an unset URL was not skipped')
    ( HTUI_TEST_QDRANT_URL='http://127.0.0.1:1' step_qdrant >/dev/null; [[ $? -eq 1 ]] ) \
        || failures+=('qdrant: an unreachable host passed')
    ( HTUI_TEST_QDRANT_URL='not-a-url' step_qdrant >/dev/null ) \
        || failures+=('qdrant: an unparsed URL failed')
    out="$(HTUI_TEST_QDRANT_URL='http://u:s3cret@127.0.0.1:1' step_qdrant 2>&1)"
    [[ "$out" != *s3cret* ]] || failures+=('qdrant: the URL credentials were printed')
    out="$(HTUI_TEST_QDRANT_URL='http://u:p@ss@127.0.0.1:1' step_qdrant 2>&1)"
    [[ "$out" != *ss@* ]] || failures+=('qdrant: a password with an @ was printed')

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

    # The htui-dev mod's /gate checks a step name against its own copy of STEPS (TOOL-8 M2).
    local mod="$REPO_ROOT/.claude/skills/htui-dev/hooks/commands.ts" listed
    if [[ -f "$mod" ]]; then
        listed="$(grep -o "GATE_STEPS = \[[^]]*\]" "$mod" | grep -o "'[a-z-]*'" | tr -d "'" | tr '\n' ' ')"
        [[ "$listed" == "${STEPS[*]} " ]] || failures+=("htui-dev GATE_STEPS ($listed) differ from STEPS (${STEPS[*]})")
    fi

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
    # `env --default-signal` (TOOL-16): a non-interactive shell starts a background job with SIGINT
    # and SIGQUIT ignored, every process under it inherits that, and a shell that starts with a
    # signal ignored cannot trap it. Two tests interrupt a `trap ... INT` child and failed in every
    # detached gate; the orphaned children showed `SigIgn: 0x6`. Bare `--default-signal` (every
    # signal) is the form GNU and uutils `env` agree on. Not `set -m`: a job-control job leads its
    # process group, `setsid` then forks, and `$!` would name a parent that exits at once.
    setsid env --default-signal bash "${BASH_SOURCE[0]}" "${args[@]}" > "$GATE_DIR/run.log" 2>&1 < /dev/null &
    echo "gate started, pid $! - log $GATE_DIR/run.log, result: bash scripts/gate.sh --status"
    exit 0
fi

run_gate "$ONLY"
exit $?
