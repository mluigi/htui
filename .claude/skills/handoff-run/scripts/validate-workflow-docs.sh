#!/usr/bin/env bash
# validate-workflow-docs.sh — structural checks for HANDOFF.md / DECISIONS.md per
# .claude/rules/workflow-docs.md. Lists findings, never auto-fixes. Bash port of
# validate-workflow-docs.ps1 for macOS/Linux — keep the two in lockstep (same check
# names, same messages, same fixture behavior in --self-test).
#
# Usage:
#   bash validate-workflow-docs.sh [--repo-root <path>] [--strict] [--self-test]
#                                  [--scope-paths <newline-separated paths>]
#
# Checks always cover the whole repo. --scope-paths only narrows what is allowed to FAIL
# the run: errors attributed to a file outside the scope list are printed as [WARNING]
# "(pre-existing...)" and do not affect the exit code. The pre-commit hook passes the
# staged paths so a commit is blocked by findings in the files it touches, never by
# unrelated pre-existing state elsewhere in the repo; a plain run (no --scope-paths, as
# /handoff-run close-out does it) is unchanged and still fails on any error anywhere.
# A finding with no file attribution is always in scope (fail-closed).
#
# Exit codes: 0 clean (warnings allowed unless --strict), 1 findings (including a missing
#           HANDOFF.md), 2 unusable --repo-root.
# Errors:   summary-table count mismatch, duplicate/reused IDs, index not
#           reverse-chronological, index line unparseable or linking a missing file
#           (decision-index), write-up file with no index line (decision-orphan), index
#           ID disagreeing with its path (decision-id-path), legacy `## PREFIX-N`
#           write-up section still present (decision-legacy), open checklist line meant as
#           an item but failing the strict pattern (checklist-unparseable, mirrors
#           next-item-id.sh).
# Warnings: status-line recap over cap (~3), broken cross-link paths, unparseable summary
#           rows, undated archive entries.

set -u

# Windows runs the .ps1 twin of this script, never this one — refuse rather than be slow.
# Measured 2026-08-21: a bash-installed pre-commit hook ran >2 min and was killed where the
# pwsh one took 1.3 s, and bash subprocesses on that box die on fork (0xC0000142) often
# enough that a .sh failure reads as a hang instead of an error. The variants stay
# behaviourally identical, so this costs nothing but the habit.
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
source "${SCRIPT_DIR}/workflow-patterns.sh"


# Standalone date extractor. BASH_REMATCH: 1=date
DATE_PATTERN='([0-9]{4}-[0-9]{2}-[0-9]{2})'

# Summary-table row: literal "PREFIX-N" placeholder text, not an actual ID.
# BASH_REMATCH: 1=prefix 2=cell
SUMMARY_ROW_PATTERN="^\\|[[:space:]]*(${PREFIX_ALT})-N[[:space:]]*\\|[[:space:]]*([^|]*)"

STATUS_LINE_START_PATTERN='^\*\*Current status'
COMPLETION_PATTERN="(${PREFIX_ALT})-[0-9]+[[:space:]]+(\\*\\*)?(done|closed|concluded|complete)"

# ---- utilities ----

trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

# Findings accumulate as parallel arrays (severity/check/message/files) rather than a
# delimited blob — avoids picking a delimiter that can never appear in a message.
# FINDING_FILES holds the repo-relative path(s) the finding is about, space-separated
# (workflow doc paths never contain spaces). It drives --scope-paths only; an empty entry
# means "unattributed", which counts as in scope so a new check can never silently stop
# blocking by forgetting its attribution.
FINDING_SEV=(); FINDING_CHECK=(); FINDING_MSG=(); FINDING_FILES=()

reset_findings() { FINDING_SEV=(); FINDING_CHECK=(); FINDING_MSG=(); FINDING_FILES=(); }

add_finding() { # severity check message [files]
    FINDING_SEV+=("$1"); FINDING_CHECK+=("$2"); FINDING_MSG+=("$3"); FINDING_FILES+=("${4:-}")
}

count_findings() { # severity [check] -> stdout count; check omitted = any check
    local sev="$1" chk="${2:-}" n=0 i
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        [[ "${FINDING_SEV[i]}" == "$sev" ]] || continue
        [[ -z "$chk" || "${FINDING_CHECK[i]}" == "$chk" ]] || continue
        n=$((n + 1))
    done
    printf '%s' "$n"
}

count_findings_error_in() { # space-separated check names -> count of Error findings among them
    local list=" $1 " i c=0
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        [[ "${FINDING_SEV[i]}" == "Error" ]] || continue
        case "$list" in *" ${FINDING_CHECK[i]} "*) c=$((c + 1)) ;; esac
    done
    printf '%s' "$c"
}

count_findings_error_not() { # exclude check name -> count of Error findings NOT that check
    local exclude="$1" i c=0
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        if [[ "${FINDING_SEV[i]}" == "Error" && "${FINDING_CHECK[i]}" != "$exclude" ]]; then
            c=$((c + 1))
        fi
    done
    printf '%s' "$c"
}

# ---- scope gate ----
# SCOPE_ACTIVE=0 -> every finding is in scope (the whole-repo run: unchanged behavior).
SCOPE_ACTIVE=0
SCOPE_PATHS=()

add_scope_paths() { # $1=newline-separated path list; repeatable
    local line
    while IFS= read -r line; do
        line="$(trim "$line")"
        [[ -z "$line" ]] && continue
        line="${line#./}"
        SCOPE_PATHS+=("$line")
    done <<< "$1"
    SCOPE_ACTIVE=1
}

reset_scope() { SCOPE_ACTIVE=0; SCOPE_PATHS=(); }

finding_in_scope() { # $1=finding index -> exit 0 when it may block
    [[ $SCOPE_ACTIVE -eq 1 ]] || return 0
    local files="${FINDING_FILES[$1]}" f s
    # Unattributed finding: fail-closed, treat as in scope.
    [[ -z "$files" ]] && return 0
    for f in $files; do
        for s in "${SCOPE_PATHS[@]:-}"; do
            [[ "$f" == "$s" ]] && return 0
        done
    done
    return 1
}

classify_findings() {
    # Splits FINDING_* into what may fail the run and what is only reported. Sets
    # OUT_LINES (display lines, in finding order), BLOCKING_ERRORS, DEMOTED_ERRORS
    # (errors outside the scope, printed as warnings), SCOPED_WARNINGS (warnings inside
    # the scope — the only ones --strict acts on).
    OUT_LINES=(); BLOCKING_ERRORS=0; DEMOTED_ERRORS=0; SCOPED_WARNINGS=0
    local i sev_upper
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        if finding_in_scope "$i"; then
            sev_upper="$(printf '%s' "${FINDING_SEV[i]}" | tr '[:lower:]' '[:upper:]')"
            OUT_LINES+=("[${sev_upper}] ${FINDING_CHECK[i]}: ${FINDING_MSG[i]}")
            if [[ "${FINDING_SEV[i]}" == "Error" ]]; then
                BLOCKING_ERRORS=$((BLOCKING_ERRORS + 1))
            else
                SCOPED_WARNINGS=$((SCOPED_WARNINGS + 1))
            fi
        elif [[ "${FINDING_SEV[i]}" == "Error" ]]; then
            # Loud, every run, but not this commit's fault — still fails a plain run.
            OUT_LINES+=("[WARNING] ${FINDING_CHECK[i]}: ${FINDING_MSG[i]} (pre-existing, outside this commit's staged files)")
            DEMOTED_ERRORS=$((DEMOTED_ERRORS + 1))
        else
            OUT_LINES+=("[WARNING] ${FINDING_CHECK[i]}: ${FINDING_MSG[i]}")
        fi
    done
}

read_file_lines() { # $1=file -> sets global READ_LINES; CRLF-tolerant
    READ_LINES=()
    [[ -f "$1" ]] || return 0
    local line
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        READ_LINES+=("$line")
    done < "$1"
}

# ---- parsing ----

get_open_item_ids() { # uses HANDOFF_LINES -> OPEN_IDS
    OPEN_IDS=()
    local line
    for line in "${HANDOFF_LINES[@]}"; do
        if [[ "$line" =~ $CHECKLIST_LINE_PATTERN ]]; then
            OPEN_IDS+=("${BASH_REMATCH[1]}")
        fi
    done
}

get_decision_headers() { # uses DECISIONS_LINES -> HEADER_IDS/HEADER_DATES/HEADER_LINES
    HEADER_IDS=(); HEADER_DATES=(); HEADER_LINES=()
    local line date ids id
    for line in "${DECISIONS_LINES[@]}"; do
        if [[ "$line" =~ $LEGACY_HEADER_PATTERN ]]; then
            date=""
            if [[ "$line" =~ $DATE_PATTERN ]]; then date="${BASH_REMATCH[1]}"; fi
            ids=($(parse_legacy_header_ids "$line"))
            for id in "${ids[@]}"; do
                HEADER_IDS+=("$id"); HEADER_DATES+=("$date"); HEADER_LINES+=("$(trim "$line")")
            done
        fi
    done
}

get_decision_index_entries() { # uses DECISIONS_LINES -> IDX_ID/IDX_PATH/IDX_TITLE/IDX_STATUS/IDX_DATE/IDX_LINE
    IDX_ID=(); IDX_PATH=(); IDX_TITLE=(); IDX_STATUS=(); IDX_DATE=(); IDX_LINE=()
    local line
    for line in "${DECISIONS_LINES[@]}"; do
        if [[ "$line" =~ $INDEX_LINE_PATTERN ]]; then
            IDX_ID+=("${BASH_REMATCH[1]}"); IDX_PATH+=("${BASH_REMATCH[2]}")
            IDX_TITLE+=("${BASH_REMATCH[3]}"); IDX_STATUS+=("${BASH_REMATCH[4]}")
            IDX_DATE+=("${BASH_REMATCH[5]}"); IDX_LINE+=("$(trim "$line")")
        fi
    done
}

get_archive_entries() {
    # One ordered scan producing the union of legacy `## PREFIX-N` headers and index
    # lines, tagged and in file order. Order matters: duplicate-id and reverse-chron run
    # over this, and building the two separately would not preserve true position across
    # the boundary between an index block and a legacy block.
    # uses DECISIONS_LINES -> ARCH_ID/ARCH_DATE/ARCH_LINE/ARCH_KIND
    ARCH_ID=(); ARCH_DATE=(); ARCH_LINE=(); ARCH_KIND=()
    local line date ids id
    for line in "${DECISIONS_LINES[@]}"; do
        if [[ "$line" =~ $INDEX_LINE_PATTERN ]]; then
            ARCH_ID+=("${BASH_REMATCH[1]}"); ARCH_DATE+=("${BASH_REMATCH[5]}")
            ARCH_LINE+=("$(trim "$line")"); ARCH_KIND+=("Index")
        elif [[ "$line" =~ $LEGACY_HEADER_PATTERN ]]; then
            date=""
            if [[ "$line" =~ $DATE_PATTERN ]]; then date="${BASH_REMATCH[1]}"; fi
            ids=($(parse_legacy_header_ids "$line"))
            for id in "${ids[@]}"; do
                ARCH_ID+=("$id"); ARCH_DATE+=("$date")
                ARCH_LINE+=("$(trim "$line")"); ARCH_KIND+=("Header")
            done
        fi
    done
}

get_decision_files() {
    # Absent docs/decisions/ is legal (a repo that has not migrated yet) — empty result,
    # silently. Cutting relative paths is done by `cd`-ing into the resolved root first, so
    # a relative -RepoRoot (case 15) behaves identically to an absolute one.
    # $1=root -> DFILE_PATH (repo-relative)/DFILE_FULLPATH (absolute)/DFILE_ISITEM (0/1)
    DFILE_PATH=(); DFILE_FULLPATH=(); DFILE_ISITEM=()
    local root="$1" abs_root rel base
    abs_root="$(cd "$root" 2>/dev/null && pwd)" || return 0
    [[ -d "$abs_root/docs/decisions" ]] || return 0
    while IFS= read -r rel; do
        [[ -z "$rel" ]] && continue
        base="$(basename "$rel")"
        DFILE_PATH+=("$rel"); DFILE_FULLPATH+=("$abs_root/$rel")
        # An item write-up is named <prefix>-N.md. Anything else under docs/decisions/ is
        # supplementary prose (a README, notes) — it holds no ID, so it is not orphaned by
        # having no index line.
        if [[ "$base" =~ ^[a-z]+-[0-9]+\.md$ ]]; then DFILE_ISITEM+=(1); else DFILE_ISITEM+=(0); fi
    done < <(cd "$abs_root" && find docs/decisions -type f -name '*.md' | sort)
}

# ---- checks ----

test_checklist_lines() {
    # HANDOFF counterpart of test_decision_index's loose rescan. Check name matches the
    # minter's (checklist-unparseable, not an area-based name) so both tools report the
    # same defect under the same label. The at-risk ID is deliberately not added to the
    # open-ID count: this validator computes no next ID, and counting a ticked line as
    # open would only stack a summary-table error on an already-red repo.
    local line loose_id
    for line in "${HANDOFF_LINES[@]}"; do
        if [[ "$line" =~ $CHECKLIST_LINE_LOOSE_PATTERN ]]; then
            loose_id="${BASH_REMATCH[1]}"
            if [[ ! "$line" =~ $CHECKLIST_LINE_PATTERN ]]; then
                add_finding Error checklist-unparseable \
                    "${loose_id}: open-item line unparseable - ID space untrustworthy until fixed: $(trim "$line")" \
                    "HANDOFF.md"
            fi
        fi
    done
}

count_open_ids_for_prefix() { # $1=prefix -> stdout count
    local p="$1" c=0 id
    for id in "${OPEN_IDS[@]}"; do
        [[ "$id" == "${p}"-* ]] && c=$((c + 1))
    done
    printf '%s' "$c"
}

test_summary_table() {
    local line prefix cell count actual p rp found
    local -a row_prefixes=()
    for line in "${HANDOFF_LINES[@]}"; do
        if [[ "$line" =~ $SUMMARY_ROW_PATTERN ]]; then
            prefix="${BASH_REMATCH[1]}"; cell="${BASH_REMATCH[2]}"
            row_prefixes+=("$prefix")
            if [[ "$cell" =~ ([0-9]+) ]]; then
                count="${BASH_REMATCH[1]}"
                actual="$(count_open_ids_for_prefix "$prefix")"
                if [[ "$count" -ne "$actual" ]]; then
                    add_finding Error summary-table \
                        "Summary row ${prefix}-N says ${count} open, HANDOFF checklist has ${actual}." \
                        "HANDOFF.md"
                fi
            else
                add_finding Warning summary-table \
                    "Summary row ${prefix}-N has no parseable count: '$(trim "$cell")'." \
                    "HANDOFF.md"
            fi
        fi
    done
    for p in "${PREFIXES[@]}"; do
        actual="$(count_open_ids_for_prefix "$p")"
        [[ "$actual" -gt 0 ]] || continue
        found=0
        for rp in "${row_prefixes[@]:-}"; do [[ "$rp" == "$p" ]] && { found=1; break; }; done
        if [[ $found -eq 0 ]]; then
            add_finding Warning summary-table \
                "Open ${p} items exist but summary table has no ${p}-N row." "HANDOFF.md"
        fi
    done
}

test_duplicate_ids() {
    # ARCH_ID is the union of legacy header IDs and index-line IDs, so this works
    # unchanged whether a repo is pre-migration, migrated, or mid-migration.
    local dup id cnt
    if [[ ${#OPEN_IDS[@]} -gt 0 ]]; then
        while IFS= read -r dup; do
            [[ -z "$dup" ]] && continue
            cnt="$(printf '%s\n' "${OPEN_IDS[@]}" | grep -Fxc "$dup")"
            add_finding Error duplicate-id "ID ${dup} open ${cnt}x in HANDOFF.md." "HANDOFF.md"
        done < <(printf '%s\n' "${OPEN_IDS[@]}" | sort | uniq -d)
    fi
    if [[ ${#ARCH_ID[@]} -gt 0 ]]; then
        while IFS= read -r dup; do
            [[ -z "$dup" ]] && continue
            cnt="$(printf '%s\n' "${ARCH_ID[@]}" | grep -Fxc "$dup")"
            add_finding Error duplicate-id "ID ${dup} archived ${cnt}x in DECISIONS.md." "DECISIONS.md"
        done < <(printf '%s\n' "${ARCH_ID[@]}" | sort | uniq -d)
    fi
    if [[ ${#OPEN_IDS[@]} -gt 0 ]]; then
        while IFS= read -r id; do
            [[ -z "$id" ]] && continue
            if [[ ${#ARCH_ID[@]} -gt 0 ]] && printf '%s\n' "${ARCH_ID[@]}" | grep -Fxq "$id"; then
                add_finding Error duplicate-id \
                    "ID ${id} is open in HANDOFF.md AND archived in DECISIONS.md (IDs are never reused)." \
                    "HANDOFF.md DECISIONS.md"
            fi
        done < <(printf '%s\n' "${OPEN_IDS[@]}" | sort -u)
    fi
}

test_reverse_chronology() {
    # Walks ARCH_* (both legacy headers and index lines, in file order) — reverse-chron is
    # enforced over the union, not just legacy headers.
    local prev_date="" prev_id="" i date id line
    for ((i = 0; i < ${#ARCH_ID[@]}; i++)); do
        id="${ARCH_ID[i]}"; date="${ARCH_DATE[i]}"; line="${ARCH_LINE[i]}"
        if [[ -z "$date" ]]; then
            add_finding Warning reverse-chron "No date parseable in header: ${line}" "DECISIONS.md"
            continue
        fi
        if [[ -n "$prev_date" && "$date" > "$prev_date" ]]; then
            add_finding Error reverse-chron \
                "DECISIONS.md not reverse-chronological: ${id} (${date}) sits below ${prev_id} (${prev_date})." \
                "DECISIONS.md"
        fi
        prev_date="$date"; prev_id="$id"
    done
}

test_decision_index() {
    # $1=root (as originally passed — relative or absolute; existence checks resolve
    # against it the same way a plain shell test would, matching the PS Join-Path+Test-Path
    # behavior against the process CWD).
    local root="$1" line loose_id i id path prefix id_lower expected p found j
    for line in "${DECISIONS_LINES[@]}"; do
        if [[ "$line" =~ $INDEX_LINE_LOOSE_PATTERN ]]; then
            loose_id="${BASH_REMATCH[1]}"
            if [[ ! "$line" =~ $INDEX_LINE_PATTERN ]]; then
                # Name the at-risk ID: duplicate-id can only compare IDs it can see, so it
                # is structurally unable to catch a line that failed to parse.
                add_finding Error decision-index \
                    "${loose_id}: index line unparseable — ID space untrustworthy until fixed: $(trim "$line")" \
                    "DECISIONS.md"
            fi
        fi
    done

    for ((i = 0; i < ${#IDX_ID[@]}; i++)); do
        id="${IDX_ID[i]}"; path="${IDX_PATH[i]}"
        if [[ ! -f "$root/$path" ]]; then
            # Attributed to the linked path too: staging the deletion of a write-up must
            # block even when DECISIONS.md itself is untouched.
            add_finding Error decision-index \
                "${id}: index line links ${path}, which does not exist." "DECISIONS.md ${path}"
        fi
        prefix="$(printf '%s' "${id%%-*}" | tr '[:upper:]' '[:lower:]')"
        id_lower="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
        expected="docs/decisions/${prefix}/${id_lower}.md"
        if [[ "$path" != "$expected" ]]; then
            add_finding Error decision-id-path \
                "${id}: index line links ${path}, expected ${expected}." "DECISIONS.md ${path}"
        fi
    done

    for ((i = 0; i < ${#DFILE_PATH[@]}; i++)); do
        [[ "${DFILE_ISITEM[i]}" -eq 1 ]] || continue
        p="${DFILE_PATH[i]}"
        found=0
        for ((j = 0; j < ${#IDX_PATH[@]}; j++)); do
            if [[ "${IDX_PATH[j]}" == "$p" ]]; then found=1; break; fi
        done
        if [[ $found -eq 0 ]]; then
            add_finding Error decision-orphan "${p} has no index line in DECISIONS.md." "DECISIONS.md ${p}"
        fi
    done

    if [[ ${#HEADER_IDS[@]} -gt 0 ]]; then
        # Error since the workspace-wide migration completed (MOD-7). It was a Warning
        # only while repos were mid-migration; every repo is now on the per-item layout.
        add_finding Error decision-legacy \
            "DECISIONS.md holds ${#HEADER_IDS[@]} legacy '## PREFIX-N' write-up section(s); the write-up belongs at docs/decisions/<prefix>/<prefix>-N.md with an index line here." \
            "DECISIONS.md"
    fi
}

test_status_line_cap() {
    local start=-1 i cur paragraph="" mentions
    for ((i = 0; i < ${#HANDOFF_LINES[@]}; i++)); do
        if [[ "${HANDOFF_LINES[i]}" =~ $STATUS_LINE_START_PATTERN ]]; then start=$i; break; fi
    done
    [[ $start -lt 0 ]] && return 0
    i=$start
    while ((i < ${#HANDOFF_LINES[@]})); do
        cur="${HANDOFF_LINES[i]}"
        [[ -z "$(trim "$cur")" ]] && break
        paragraph+="$cur "
        i=$((i + 1))
    done
    mentions="$(printf '%s' "$paragraph" | grep -oiE "$COMPLETION_PATTERN" | wc -l | tr -d '[:space:]')"
    if [[ "$mentions" -gt 3 ]]; then
        add_finding Warning status-line \
            "Status line recaps ${mentions} completions; rule caps it at ~2-3 (drop oldest)." \
            "HANDOFF.md"
    fi
}

CL_LINES=()

test_cross_links() {
    # A backticked `*.md` mention resolves against the containing file's directory OR the
    # repo root — this corpus writes both: true relative links ("../HANDOFF.md") and
    # root-relative conventions ("docs/ANA-2.md") that stay root-relative no matter which
    # file they appear in. Only a path that satisfies neither is a finding.
    # $1=file_dir $2=root_dir (may be "") $3=file_label ; reads global CL_LINES
    local file_dir="$1" root_dir="$2" file_label="$3"
    local -a bases=("$file_dir")
    if [[ -n "$root_dir" && "$root_dir" != "$file_dir" ]]; then bases+=("$root_dir"); fi
    local -a seen=()
    local line m path already s base found
    for line in "${CL_LINES[@]}"; do
        while IFS= read -r m; do
            [[ -z "$m" ]] && continue
            path="${m#\`}"; path="${path%\`}"
            # Skip URLs, globs, and `<placeholder>` path templates.
            case "$path" in *"://"*) continue ;; esac
            case "$path" in *[\*\?\<\>]*) continue ;; esac
            already=0
            for s in "${seen[@]:-}"; do [[ "$s" == "$path" ]] && { already=1; break; }; done
            [[ $already -eq 1 ]] && continue
            seen+=("$path")
            found=0
            for base in "${bases[@]}"; do
                if [[ -e "$base/$path" ]]; then found=1; break; fi
            done
            if [[ $found -eq 0 ]]; then
                add_finding Warning cross-link "${file_label}: linked path not found: ${path}" "$file_label"
            fi
        done < <(printf '%s\n' "$line" | grep -oE '`[^`]+\.md`')
    done
}

invoke_workflow_docs_checks() {
    # $1=root -> populates FINDING_* (via reset_findings then each check)
    local root="$1"
    local handoff_path="$root/HANDOFF.md" decisions_path="$root/DECISIONS.md"
    reset_findings

    if [[ ! -f "$handoff_path" ]]; then
        add_finding Error files "HANDOFF.md not found at ${handoff_path}" "HANDOFF.md"
        return 0
    fi
    read_file_lines "$handoff_path"; HANDOFF_LINES=("${READ_LINES[@]}")
    DECISIONS_LINES=()
    if [[ -f "$decisions_path" ]]; then
        read_file_lines "$decisions_path"; DECISIONS_LINES=("${READ_LINES[@]}")
    else
        add_finding Warning files \
            "DECISIONS.md not found at ${decisions_path} (skipping archive checks)." "DECISIONS.md"
    fi

    get_open_item_ids
    get_decision_headers
    get_decision_index_entries
    get_archive_entries
    get_decision_files "$root"

    test_checklist_lines
    test_summary_table
    test_duplicate_ids
    test_reverse_chronology
    test_decision_index "$root"
    test_status_line_cap
    CL_LINES=("${HANDOFF_LINES[@]}"); test_cross_links "$root" "" "HANDOFF.md"
    CL_LINES=("${DECISIONS_LINES[@]}"); test_cross_links "$root" "" "DECISIONS.md"

    # Each item file's relative links resolve against its own directory, not the repo root.
    local i fdir
    for ((i = 0; i < ${#DFILE_PATH[@]}; i++)); do
        fdir="$(dirname "${DFILE_FULLPATH[i]}")"
        read_file_lines "${DFILE_FULLPATH[i]}"; CL_LINES=("${READ_LINES[@]}")
        test_cross_links "$fdir" "$root" "${DFILE_PATH[i]}"
    done
}

# ---- self-test fixtures ----

mkfixture() { # $1=dir $2=handoff_content $3=decisions_content
    mkdir -p "$1"
    printf '%s\n' "$2" > "$1/HANDOFF.md"
    printf '%s\n' "$3" > "$1/DECISIONS.md"
}

mkfile() { # $1=path $2=content
    mkdir -p "$(dirname "$1")"
    printf '%s\n' "$2" > "$1"
}

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/wfdocs-selftest-XXXXXX")"
    local -a failures=()

    # Shared fixture pieces for the per-item-layout cases. No open items and no summary
    # rows, so a clean fixture yields zero findings of any kind.
    local empty_handoff write_up
    read -r -d '' empty_handoff <<'EOF' || true
# HANDOFF

**Current status (2026-01-05):** nothing open.

## Open items

## Summary

| Area  | Open |
|-------|------|
EOF
    read -r -d '' write_up <<'EOF' || true
# MOD-7 - Thing (done, 2026-01-05)

Write-up body.
EOF

    local content dir errs cmp_ids

    # Case 1: clean pair -> no errors expected.
    dir="$tmp/clean"
    read -r -d '' content <<'EOF' || true
# HANDOFF

**Current status (2026-01-02):** ANA-1 done.

## Open items

- [ ] **MOD-1 - Thing.** body `DECISIONS.md`

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
| ANA-N | 0    |
EOF
    mkdir -p "$dir"; printf '%s\n' "$content" > "$dir/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[ANA-1](docs/decisions/ana/ana-1.md)** - Old analysis (done, 2026-01-02)
EOF
    printf '%s\n' "$content" > "$dir/DECISIONS.md"
    read -r -d '' content <<'EOF' || true
# ANA-1 - Old analysis (done, 2026-01-02)

Write-up.
EOF
    mkfile "$dir/docs/decisions/ana/ana-1.md" "$content"
    invoke_workflow_docs_checks "$dir"
    errs="$(count_findings Error)"
    [[ "$errs" -ne 0 ]] && failures+=("clean: expected 0 errors, got $errs")

    # Case 2: summary count wrong -> summary-table error expected.
    dir="$tmp/badcount"; mkdir -p "$dir"
    sed 's/| MOD-N | 1/| MOD-N | 2/' "$tmp/clean/HANDOFF.md" > "$dir/HANDOFF.md"
    cp "$tmp/clean/DECISIONS.md" "$dir/DECISIONS.md"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error summary-table)" -lt 1 ]] && failures+=("badcount: expected summary-table error, got none")

    # Case 3: ID both open and archived -> duplicate-id error expected.
    dir="$tmp/dup"; mkdir -p "$dir"
    cp "$tmp/clean/HANDOFF.md" "$dir/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
## MOD-1 - Thing (done, 2026-01-03)

## ANA-1 - Old analysis (done, 2026-01-02)
EOF
    printf '%s\n' "$content" > "$dir/DECISIONS.md"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error duplicate-id)" -lt 1 ]] && failures+=("dup: expected duplicate-id error, got none")

    # Case 4: DECISIONS dates increasing downward -> reverse-chron error expected.
    dir="$tmp/order"; mkdir -p "$dir"
    cp "$tmp/clean/HANDOFF.md" "$dir/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
## ANA-1 - Old analysis (done, 2026-01-02)

## ANA-2 - Newer analysis below older (done, 2026-03-01)
EOF
    printf '%s\n' "$content" > "$dir/DECISIONS.md"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error reverse-chron)" -lt 1 ]] && failures+=("order: expected reverse-chron error, got none")

    # ---- per-item decision layout ----

    # Case 5: migrated repo, index + file agree -> no errors at all.
    dir="$tmp/migrated"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    errs="$(count_findings Error)"
    [[ "$errs" -ne 0 ]] && failures+=("migrated: expected 0 errors, got $errs")

    # Case 6: index line points at a file that does not exist.
    dir="$tmp/missingfile"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error decision-index)" -lt 1 ]] && failures+=("missingfile: expected decision-index error, got none")

    # Case 7: item file exists with no index line pointing at it.
    dir="$tmp/orphan"
    mkfixture "$dir" "$empty_handoff" "# DECISIONS"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error decision-orphan)" -lt 1 ]] && failures+=("orphan: expected decision-orphan error, got none")

    # Case 8: index line ID disagrees with the path it links.
    dir="$tmp/idpath"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-6.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-6.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error decision-id-path)" -lt 1 ]] && failures+=("idpath: expected decision-id-path error, got none")

    # Case 9: line looks like an index line but does not parse -> must NOT be skipped, a
    # dropped line drops its ID and the next mint reuses it.
    dir="$tmp/unparseable"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error decision-index)" -lt 1 ]] && failures+=("unparseable: expected decision-index error, got none")

    # Case 10: legacy repo during the transition -> warned about, never blocked. Two-part
    # assertion on purpose: asserting only the warning would let an implementation that
    # also false-positives the new Errors pass this case.
    dir="$tmp/legacy"
    read -r -d '' content <<'EOF' || true
# DECISIONS

## MOD-7 - Thing (done, 2026-01-05)

Write-up body.
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error decision-legacy)" -lt 1 ]] && failures+=("legacy: expected decision-legacy error, got none")
    errs="$(count_findings_error_in 'decision-index decision-orphan decision-id-path')"
    [[ "$errs" -ne 0 ]] && failures+=("legacy: unexpected errors ($errs spurious)")

    # Case 11: reverse-chronology enforced over index lines, not just headers.
    dir="$tmp/indexorder"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-6](docs/decisions/mod/mod-6.md)** - Older (done, 2026-01-02)
- **[MOD-7](docs/decisions/mod/mod-7.md)** - Newer below older (done, 2026-03-01)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-6.md" "$write_up"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error reverse-chron)" -lt 1 ]] && failures+=("indexorder: expected reverse-chron error over index lines, got none")

    # Case 12: ID open in HANDOFF.md and archived in the index -> never reused.
    dir="$tmp/indexdup"
    read -r -d '' content <<'EOF' || true
# HANDOFF

**Current status (2026-01-05):** one open.

## Open items

- [ ] **MOD-7 - Thing.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
EOF
    local handoff12="$content"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$handoff12" "$content"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"
    [[ "$(count_findings Error duplicate-id)" -lt 1 ]] && failures+=("indexdup: expected duplicate-id error across HANDOFF/index, got none")

    # Case 13: index lines malformed in FORMAT rather than content — leading whitespace, a
    # `*` bullet, an out-of-set prefix. Each still holds a real ID, so each must error; if
    # the loose pattern is no looser than the strict one these slip through silently and
    # the ID gets minted a second time later.
    dir="$tmp/badformat"
    read -r -d '' content <<'EOF' || true
# DECISIONS

 - **[MOD-7](docs/decisions/mod/mod-7.md)** - Leading space (done, 2026-01-05)
* **[MOD-8](docs/decisions/mod/mod-8.md)** - Star bullet (done, 2026-01-04)
- **[FOO-3](docs/decisions/foo/foo-3.md)** - Prefix outside the six (done, 2026-01-03)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    invoke_workflow_docs_checks "$dir"
    errs="$(count_findings Error decision-index)"
    [[ "$errs" -lt 3 ]] && failures+=("badformat: expected 3 decision-index errors, got $errs")

    # Case 14: supplementary prose under docs/decisions/ holds no ID, so it is not an
    # orphan. A README there must not block the repo.
    dir="$tmp/supplementary"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    mkfile "$dir/docs/decisions/README.md" "$(printf '# Notes\n\nHow this directory is organised.')"
    invoke_workflow_docs_checks "$dir"
    errs="$(count_findings Error)"
    [[ "$errs" -ne 0 ]] && failures+=("supplementary: expected 0 errors, got $errs")

    # Case 16: checklist lines malformed in format — a ticked box left instead of deleted,
    # and a leading space. Same dropped-ID logic as index lines, mirrored onto the HANDOFF
    # half; the well-formed line must stay silent.
    dir="$tmp/badchecklist"
    read -r -d '' content <<'EOF' || true
# HANDOFF

**Current status (2026-01-05):** one open.

## Open items

- [ ] **MOD-11 - Fine.** body
- [x] **MOD-12 - Ticked, left instead of deleted.** body
 - [ ] **MOD-13 - Leading space.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
EOF
    mkfixture "$dir" "$content" "# DECISIONS"
    invoke_workflow_docs_checks "$dir"
    errs="$(count_findings Error checklist-unparseable)"
    [[ "$errs" -lt 2 ]] && failures+=("badchecklist: expected 2 checklist-unparseable errors, got $errs")
    errs="$(count_findings_error_not checklist-unparseable)"
    [[ "$errs" -ne 0 ]] && failures+=("badchecklist: unexpected errors ($errs spurious)")

    # Case 15: a RELATIVE -RepoRoot must behave identically to an absolute one — cutting
    # relative paths out of absolute FullNames means cutting by the length of a relative
    # root would mangle every path and orphan every write-up.
    (
        cd "$tmp" || exit 1
        invoke_workflow_docs_checks "migrated"
        count_findings Error > "$tmp/.relerrs"
    )
    errs="$(cat "$tmp/.relerrs" 2>/dev/null || echo -1)"
    [[ "$errs" -ne 0 ]] && failures+=("relativeroot: expected 0 errors via relative root, got $errs")

    # Case 17: --scope-paths gate. A pre-existing DECISIONS.md error must not fail a commit
    # that stages only HANDOFF.md (the repro this flag exists for) — but it stays visible,
    # still fails a plain whole-repo run, and still blocks the moment DECISIONS.md is staged.
    dir="$tmp/scopegate"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    mkfile "$dir/docs/decisions/mod/mod-7.md" "$write_up"
    invoke_workflow_docs_checks "$dir"

    reset_scope; add_scope_paths "HANDOFF.md"
    classify_findings
    [[ "$BLOCKING_ERRORS" -ne 0 ]] && failures+=("scopegate: HANDOFF.md-only scope blocked on $BLOCKING_ERRORS unrelated error(s)")
    [[ "$DEMOTED_ERRORS" -lt 1 ]] && failures+=('scopegate: pre-existing error not reported as demoted warning')
    printf '%s\n' "${OUT_LINES[@]}" | grep -q 'pre-existing' \
        || failures+=('scopegate: demoted finding not marked pre-existing in output')

    reset_scope; add_scope_paths "DECISIONS.md"
    classify_findings
    [[ "$BLOCKING_ERRORS" -lt 1 ]] && failures+=('scopegate: staging DECISIONS.md did not make its error blocking')

    # Empty scope (a commit staging none of the watched paths) blocks on nothing.
    reset_scope; add_scope_paths ""
    classify_findings
    [[ "$BLOCKING_ERRORS" -ne 0 ]] && failures+=("scopegate: empty scope still blocked on $BLOCKING_ERRORS error(s)")

    # No --scope-paths at all: whole-repo run, unchanged, still red.
    reset_scope
    classify_findings
    [[ "$BLOCKING_ERRORS" -lt 1 ]] && failures+=('scopegate: unscoped run stopped failing on the error')
    [[ "$DEMOTED_ERRORS" -ne 0 ]] && failures+=('scopegate: unscoped run demoted a finding')

    # Case 18: file attribution reaches beyond HANDOFF/DECISIONS — staging only the write-up
    # path (e.g. deleting it) must block on the index line that points at it, even though
    # DECISIONS.md itself is untouched.
    dir="$tmp/scopeattr"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    invoke_workflow_docs_checks "$dir"
    reset_scope; add_scope_paths "docs/decisions/mod/mod-7.md"
    classify_findings
    [[ "$BLOCKING_ERRORS" -lt 1 ]] && failures+=('scopeattr: missing-write-up error not attributed to the write-up path')
    reset_scope

    # Case 19: compound legacy header parses all contained IDs.
    dir="$tmp/compound"
    read -r -d '' content <<'EOF' || true
# DECISIONS

## MOD-18/19/20 - Relocated to engine as MOD-2/3/4 (relocated, 2026-07-18)
EOF
    mkfixture "$dir" "$empty_handoff" "$content"
    read_file_lines "$dir/DECISIONS.md"; DECISIONS_LINES=("${READ_LINES[@]}")
    get_archive_entries
    # Space-padded join relies on default IFS; every IFS= in this file is command-local.
    cmp_ids=" ${ARCH_ID[*]} "
    if [[ "$cmp_ids" != *" MOD-18 "* || "$cmp_ids" != *" MOD-19 "* || "$cmp_ids" != *" MOD-20 "* ]]; then
        failures+=("compound: expected MOD-18, MOD-19, MOD-20 from compound header, got:${cmp_ids}")
    fi

    rm -rf "$tmp"

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: 19/19 cases PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f
    for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() {
    echo "Usage: $0 [--repo-root <path>] [--strict] [--self-test] [--scope-paths <paths>]"
}

REPO_ROOT=""
STRICT=0
SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --strict) STRICT=1; shift ;;
        # Repeatable; an empty value still activates the scope (nothing staged that this
        # validator owns -> nothing may block), never silently falls back to whole-repo.
        --scope-paths) add_scope_paths "$2"; shift 2 ;;
        --self-test) SELF_TEST=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ $SELF_TEST -eq 1 ]]; then
    run_self_test
    exit $?
fi

if [[ -z "$REPO_ROOT" ]]; then
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ in every repo (M2 keeps
    # this layout) — repo root is four levels up.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$REPO_ROOT" || ! -d "$REPO_ROOT" ]]; then
    echo "RepoRoot not found: $REPO_ROOT"
    exit 2
fi

invoke_workflow_docs_checks "$REPO_ROOT"
classify_findings

error_count="$BLOCKING_ERRORS"
# Demoted errors print as warnings, so they count as warnings in the summary line — but
# they never trip --strict, which is about warnings the current change is answerable for.
warning_count=$(( $(count_findings Warning) + DEMOTED_ERRORS ))

if [[ ${#OUT_LINES[@]} -gt 0 ]]; then printf '%s\n' "${OUT_LINES[@]}"; fi
summary="workflow-docs validation: ${error_count} error(s), ${warning_count} warning(s) in ${REPO_ROOT}"
if [[ $SCOPE_ACTIVE -eq 1 && $DEMOTED_ERRORS -gt 0 ]]; then
    summary+=" (${DEMOTED_ERRORS} pre-existing error(s) outside the staged files, not blocking - a plain run still fails on them)"
fi
echo "$summary"

if [[ "$error_count" -gt 0 ]]; then exit 1; fi
if [[ $STRICT -eq 1 && "$SCOPED_WARNINGS" -gt 0 ]]; then exit 1; fi
exit 0
