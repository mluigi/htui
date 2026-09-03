#!/usr/bin/env bash
# next-item-id.sh — the owned-ID mint of .claude/rules/workflow-docs.md, as a command.
# Bash port of next-item-id.ps1 — keep the two in lockstep. Reports the next free item ID
# for a prefix from OWNED IDs only: open checklist lines in HANDOFF.md plus the archive
# (index lines in DECISIONS.md cross-checked against the docs/decisions/<prefix>/ listing).
# Never greps raw PREFIX-N mentions — cross-repo references ("engine MOD-11") are other
# repos' items and inflate the count.
#
# Purely file-derived and offline: it reads the working tree of the repo it is invoked in
# and nothing else. It never reads another repo's files.
#
# Usage:
#   bash next-item-id.sh --prefix MOD [--repo-root <path>] [--id-only] [--json]
#   bash next-item-id.sh --all [--repo-root <path>] [--json]
#   bash next-item-id.sh --self-test
#
# Exit codes: 0 clean, 1 findings (including a missing HANDOFF.md — the ID space cannot be
#           trusted, so the mint is blocked), 2 unusable --repo-root or an out-of-set --prefix.
#
# Errors (each blocks the mint; the offending ID is still counted into the max, so a defect
# can never LOWER the next ID):
#   index-unparseable     line meant as an index line fails the strict pattern
#   index-id-path         index-line ID disagrees with the path it links
#   decision-orphan       docs/decisions write-up with no index line
#   legacy-header         `## PREFIX-N` write-up section still in DECISIONS.md (compound-aware)
#   checklist-unparseable line meant as an open checklist item fails the strict pattern
#   duplicate-id          ID open twice, archived twice, or both open and archived
#   files                 HANDOFF.md missing
#
# The law is .claude/rules/workflow-docs.md; this script implements it, it does not restate
# it. On any conflict the rule wins.

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


SCRIPT_SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"

trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

json_escape() {
    local s="$1"
    s="${s//\\/\\\\}"
    s="${s//\"/\\\"}"
    s="${s//$'\n'/\\n}"
    s="${s//$'\t'/\\t}"
    printf '%s' "$s"
}

contains() {
    # $1=needle, remaining args = haystack
    local needle="$1" x
    shift
    for x in "$@"; do [[ "$x" == "$needle" ]] && return 0; done
    return 1
}

FINDING_SEV=(); FINDING_CHECK=(); FINDING_MSG=()
reset_findings() { FINDING_SEV=(); FINDING_CHECK=(); FINDING_MSG=(); }
add_finding() { FINDING_SEV+=("$1"); FINDING_CHECK+=("$2"); FINDING_MSG+=("$3"); }
count_findings_error() {
    local n=0 i
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do [[ "${FINDING_SEV[i]}" == Error ]] && n=$((n + 1)); done
    printf '%s' "$n"
}
count_findings_error_check() { # $1=check name
    local chk="$1" n=0 i
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        [[ "${FINDING_SEV[i]}" == Error && "${FINDING_CHECK[i]}" == "$chk" ]] && n=$((n + 1))
    done
    printf '%s' "$n"
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

get_decision_file_ids() {
    # The directory half of the archive. Absent docs/decisions/ is legal (a repo with no
    # archive yet) — empty result, silently. An item write-up is named <prefix>-N.md;
    # anything else under docs/decisions/ is supplementary prose and holds no ID.
    # $1=root -> DFID_ID[] / DFID_PATH[]
    DFID_ID=(); DFID_PATH=()
    local root="$1" abs_root rel base
    abs_root="$(cd "$root" 2>/dev/null && pwd)" || return 0
    [[ -d "$abs_root/docs/decisions" ]] || return 0
    while IFS= read -r rel; do
        [[ -z "$rel" ]] && continue
        base="$(basename "$rel")"
        if [[ "$base" =~ ^([a-z]+)-([0-9]+)\.md$ ]]; then
            local p="${BASH_REMATCH[1]}" n="${BASH_REMATCH[2]}"
            DFID_ID+=("$(printf '%s' "$p" | tr '[:lower:]' '[:upper:]')-$n")
            DFID_PATH+=("$rel")
        fi
    done < <(cd "$abs_root" && find docs/decisions -type f -name '*.md' | sort)
}

max_id_num() {
    # $1=prefix, remaining args = id list (multiset ok) -> stdout: highest N for that prefix, or 0
    local p="$1" max=0 id num
    shift
    for id in "$@"; do
        case "$id" in
            "$p"-[0-9]*)
                num="${id#"$p"-}"
                if [[ "$num" =~ ^[0-9]+$ ]] && [[ "$num" -gt "$max" ]]; then max="$num"; fi
                ;;
        esac
    done
    printf '%s' "$max"
}

get_id_space() {
    # The mint itself: every owned ID, both halves, with a finding for anything that makes
    # the ID space untrustworthy. Populates FINDING_*, OPEN_IDS[], ARCHIVE_IDS[] (multisets).
    local root="$1"
    reset_findings
    OPEN_IDS=(); ARCHIVE_IDS=(); INDEX_IDS=(); LEGACY_IDS=()

    local handoff="$root/HANDOFF.md" decisions="$root/DECISIONS.md"
    if [[ ! -f "$handoff" ]]; then
        add_finding Error files "HANDOFF.md not found at $handoff"
    else
        read_file_lines "$handoff"
        local -a hl=("${READ_LINES[@]}")
        local line loose_id
        for line in "${hl[@]}"; do
            if [[ "$line" =~ $CHECKLIST_LINE_PATTERN ]]; then
                OPEN_IDS+=("${BASH_REMATCH[1]}")
            elif [[ "$line" =~ $CHECKLIST_LINE_LOOSE_PATTERN ]]; then
                loose_id="${BASH_REMATCH[1]}"
                add_finding Error checklist-unparseable \
                    "${loose_id}: open-item line unparseable - ID space untrustworthy until fixed: $(trim "$line")"
                OPEN_IDS+=("$loose_id")
            fi
        done
    fi

    if [[ -f "$decisions" ]]; then
        read_file_lines "$decisions"
        local -a dl=("${READ_LINES[@]}")
        local id path prefix id_lower expected p nums ids_str
        for line in "${dl[@]}"; do
            if [[ "$line" =~ $INDEX_LINE_PATTERN ]]; then
                id="${BASH_REMATCH[1]}"; path="${BASH_REMATCH[2]}"
                INDEX_IDS+=("$id"); ARCHIVE_IDS+=("$id")
                prefix="$(printf '%s' "${id%%-*}" | tr '[:upper:]' '[:lower:]')"
                id_lower="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
                expected="docs/decisions/${prefix}/${id_lower}.md"
                if [[ "$path" != "$expected" ]]; then
                    add_finding Error index-id-path "${id}: index line links ${path}, expected ${expected}."
                fi
            elif [[ "$line" =~ $INDEX_LINE_LOOSE_PATTERN ]]; then
                loose_id="${BASH_REMATCH[1]}"
                add_finding Error index-unparseable \
                    "${loose_id}: index line unparseable - ID space untrustworthy until fixed: $(trim "$line")"
                INDEX_IDS+=("$loose_id"); ARCHIVE_IDS+=("$loose_id")
            elif [[ "$line" =~ $LEGACY_HEADER_PATTERN ]]; then
                p="${BASH_REMATCH[1]}"; nums="${BASH_REMATCH[2]}"
                local -a numparts=()
                IFS='/' read -ra numparts <<< "$nums"
                local -a ids=()
                local n
                for n in "${numparts[@]}"; do ids+=("${p}-${n}"); done
                ids_str="$(IFS=/; echo "${ids[*]}")"
                add_finding Error legacy-header \
                    "${ids_str}: legacy '## PREFIX-N' write-up section in DECISIONS.md; the write-up belongs at docs/decisions/<prefix>/<prefix>-N.md with an index line here."
                LEGACY_IDS+=("${ids[@]}")
                ARCHIVE_IDS+=("${ids[@]}")
            fi
        done
    fi

    # An ID already accounted for by the archive — indexed, or still carried by a legacy
    # header mid-migration — is not orphaned by its file, and counting it a second time
    # would report a phantom duplicate-id for an ID that was never actually reused.
    local -a accounted=("${INDEX_IDS[@]}" "${LEGACY_IDS[@]}")
    get_decision_file_ids "$root"
    local i fid fpath
    for ((i = 0; i < ${#DFID_ID[@]}; i++)); do
        fid="${DFID_ID[i]}"; fpath="${DFID_PATH[i]}"
        if ! contains "$fid" "${accounted[@]}"; then
            add_finding Error decision-orphan "${fid}: ${fpath} has no index line in DECISIONS.md."
            ARCHIVE_IDS+=("$fid")
        fi
    done

    local dup cnt
    if [[ ${#OPEN_IDS[@]} -gt 0 ]]; then
        while IFS= read -r dup; do
            [[ -z "$dup" ]] && continue
            cnt="$(printf '%s\n' "${OPEN_IDS[@]}" | grep -Fxc "$dup")"
            add_finding Error duplicate-id "ID ${dup} open ${cnt}x in HANDOFF.md."
        done < <(printf '%s\n' "${OPEN_IDS[@]}" | sort | uniq -d)
    fi
    if [[ ${#ARCHIVE_IDS[@]} -gt 0 ]]; then
        while IFS= read -r dup; do
            [[ -z "$dup" ]] && continue
            cnt="$(printf '%s\n' "${ARCHIVE_IDS[@]}" | grep -Fxc "$dup")"
            add_finding Error duplicate-id "ID ${dup} archived ${cnt}x."
        done < <(printf '%s\n' "${ARCHIVE_IDS[@]}" | sort | uniq -d)
    fi
    if [[ ${#OPEN_IDS[@]} -gt 0 ]]; then
        local uid
        while IFS= read -r uid; do
            [[ -z "$uid" ]] && continue
            if [[ ${#ARCHIVE_IDS[@]} -gt 0 ]] && printf '%s\n' "${ARCHIVE_IDS[@]}" | grep -Fxq "$uid"; then
                add_finding Error duplicate-id "ID ${uid} is open in HANDOFF.md AND archived (IDs are never reused)."
            fi
        done < <(printf '%s\n' "${OPEN_IDS[@]}" | sort -u)
    fi
}

build_rows() {
    # $1=space-separated wanted prefixes -> ROW_PREFIX[]/ROW_MAXOPEN[]/ROW_MAXARCH[]/ROW_NEXT[]
    ROW_PREFIX=(); ROW_MAXOPEN=(); ROW_MAXARCH=(); ROW_NEXT=()
    local p om am mx
    for p in $1; do
        om="$(max_id_num "$p" "${OPEN_IDS[@]}")"
        am="$(max_id_num "$p" "${ARCHIVE_IDS[@]}")"
        mx=$((om > am ? om : am))
        ROW_PREFIX+=("$p")
        if [[ "$om" -gt 0 ]]; then ROW_MAXOPEN+=("${p}-${om}"); else ROW_MAXOPEN+=(""); fi
        if [[ "$am" -gt 0 ]]; then ROW_MAXARCH+=("${p}-${am}"); else ROW_MAXARCH+=(""); fi
        ROW_NEXT+=("${p}-$((mx + 1))")
    done
}

emit_json() {
    # $1=root $2=trustworthy(0/1); uses ROW_*/FINDING_* -> stdout
    local root="$1" trustworthy="$2" out i first
    out='{'
    out+="\"repoRoot\":\"$(json_escape "$root")\","
    if [[ "$trustworthy" -eq 1 ]]; then out+='"trustworthy":true,'; else out+='"trustworthy":false,'; fi
    out+='"prefixes":['
    first=1
    for ((i = 0; i < ${#ROW_PREFIX[@]}; i++)); do
        [[ $first -eq 1 ]] || out+=','
        first=0
        out+='{'
        out+="\"prefix\":\"${ROW_PREFIX[i]}\","
        if [[ -n "${ROW_MAXOPEN[i]}" ]]; then out+="\"maxOpen\":\"${ROW_MAXOPEN[i]}\","; else out+='"maxOpen":null,'; fi
        if [[ -n "${ROW_MAXARCH[i]}" ]]; then out+="\"maxArchived\":\"${ROW_MAXARCH[i]}\","; else out+='"maxArchived":null,'; fi
        if [[ "$trustworthy" -eq 1 ]]; then out+="\"next\":\"${ROW_NEXT[i]}\""; else out+='"next":null'; fi
        out+='}'
    done
    out+='],"findings":['
    first=1
    for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
        [[ $first -eq 1 ]] || out+=','
        first=0
        out+='{'
        out+="\"Severity\":\"${FINDING_SEV[i]}\","
        out+="\"Check\":\"${FINDING_CHECK[i]}\","
        out+="\"Message\":\"$(json_escape "${FINDING_MSG[i]}")\""
        out+='}'
    done
    out+=']}'
    printf '%s\n' "$out"
}

# ---- self-test ----

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/nextid-selftest-XXXXXX")"
    local -a failures=()
    local write_up
    read -r -d '' write_up <<'EOF' || true
# MOD-8 - Thing (done, 2026-01-05)

Write-up body.
EOF
    local empty_handoff
    read -r -d '' empty_handoff <<'EOF' || true
# HANDOFF

**Current status (2026-01-05):** nothing open.

## Open items

## Summary

| Area  | Open |
|-------|------|
EOF

    # Case 1: open MOD-4 + archived MOD-8 -> next MOD-9.
    local c1="$tmp/basic" content
    mkdir -p "$c1/docs/decisions/mod"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **MOD-4 - Thing.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
EOF
    printf '%s\n' "$content" > "$c1/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c1/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c1/docs/decisions/mod/mod-8.md"
    get_id_space "$c1"
    [[ "$(count_findings_error)" -ne 0 ]] && failures+=("basic: expected 0 errors, got $(count_findings_error)")
    build_rows MOD
    [[ "${ROW_NEXT[0]}" != 'MOD-9' ]] && failures+=("basic: expected MOD-9, got ${ROW_NEXT[0]}")

    # Case 2: prefix with no owned IDs at all starts at 1. (Reuses case1's OPEN_IDS/ARCHIVE_IDS.)
    build_rows TOOL
    [[ "${ROW_NEXT[0]}" != 'TOOL-1' ]] && failures+=("empty-prefix: expected TOOL-1, got ${ROW_NEXT[0]}")

    # Case 3: open half is the max.
    local c3="$tmp/openmax"
    mkdir -p "$c3/docs/decisions/mod"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **MOD-12 - Thing.** body
EOF
    printf '%s\n' "$content" > "$c3/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c3/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c3/docs/decisions/mod/mod-8.md"
    get_id_space "$c3"; build_rows MOD
    [[ "${ROW_NEXT[0]}" != 'MOD-13' ]] && failures+=("openmax: expected MOD-13, got ${ROW_NEXT[0]}")

    # Case 4: archive half is the max.
    local c4="$tmp/archmax"
    mkdir -p "$c4/docs/decisions/mod"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **MOD-2 - Thing.** body
EOF
    printf '%s\n' "$content" > "$c4/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c4/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c4/docs/decisions/mod/mod-8.md"
    get_id_space "$c4"; build_rows MOD
    [[ "${ROW_NEXT[0]}" != 'MOD-9' ]] && failures+=("archmax: expected MOD-9, got ${ROW_NEXT[0]}")

    # Case 5: raw cross-repo mentions must NOT inflate the count.
    local c5="$tmp/crossrepo"
    mkdir -p "$c5/docs/decisions/mod"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **MOD-4 - Thing.** blocked on engine MOD-11 (`../engine/HANDOFF.md`); see vfs MOD-1
  and VulkanTutorials MOD-20 for prior art. Also mentions MOD-99 in prose.
EOF
    printf '%s\n' "$content" > "$c5/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-3](docs/decisions/mod/mod-3.md)** - Relocated to engine MOD-11 (done, 2026-01-04)
EOF
    printf '%s\n' "$content" > "$c5/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c5/docs/decisions/mod/mod-3.md"
    get_id_space "$c5"; build_rows MOD
    [[ "$(count_findings_error)" -ne 0 ]] && failures+=("crossrepo: expected 0 errors, got $(count_findings_error)")
    [[ "${ROW_NEXT[0]}" != 'MOD-5' ]] && failures+=("crossrepo: expected MOD-5, got ${ROW_NEXT[0]}")

    # Case 6: unparseable index line -> error, and its ID still counts.
    local c6="$tmp/unparseable"
    mkdir -p "$c6/docs/decisions/mod"
    printf '%s\n' "$empty_handoff" > "$c6/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
EOF
    printf '%s\n' "$content" > "$c6/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c6/docs/decisions/mod/mod-7.md"
    get_id_space "$c6"; build_rows MOD
    [[ "$(count_findings_error_check index-unparseable)" -lt 1 ]] && failures+=('unparseable: expected index-unparseable error, got none')
    [[ "${ROW_NEXT[0]}" != 'MOD-8' ]] && failures+=("unparseable: ID dropped out of the count, got ${ROW_NEXT[0]}")

    # Case 7: malformed in FORMAT rather than content.
    local c7="$tmp/badformat"
    mkdir -p "$c7"
    printf '%s\n' "$empty_handoff" > "$c7/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

 - **[MOD-7](docs/decisions/mod/mod-7.md)** - Leading space (done, 2026-01-05)
* **[MOD-8](docs/decisions/mod/mod-8.md)** - Star bullet (done, 2026-01-04)
- **[FOO-3](docs/decisions/foo/foo-3.md)** - Prefix outside the six (done, 2026-01-03)
EOF
    printf '%s\n' "$content" > "$c7/DECISIONS.md"
    get_id_space "$c7"; build_rows MOD
    [[ "$(count_findings_error_check index-unparseable)" -lt 3 ]] && failures+=("badformat: expected 3 index-unparseable errors, got $(count_findings_error_check index-unparseable)")
    [[ "${ROW_NEXT[0]}" != 'MOD-9' ]] && failures+=("badformat: expected MOD-9, got ${ROW_NEXT[0]}")

    # Case 8: compound legacy header.
    local c8="$tmp/compound"
    mkdir -p "$c8"
    printf '%s\n' "$empty_handoff" > "$c8/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

## MOD-18/19/20 - Three items, one header (done, 2026-01-05)

Body, which mentions engine MOD-77 as prior art.
EOF
    printf '%s\n' "$content" > "$c8/DECISIONS.md"
    get_id_space "$c8"; build_rows MOD
    [[ "$(count_findings_error_check legacy-header)" -lt 1 ]] && failures+=('compound: expected legacy-header error, got none')
    [[ "${ROW_NEXT[0]}" != 'MOD-21' ]] && failures+=("compound: expected MOD-21, got ${ROW_NEXT[0]}")

    # Case 9: write-up file with no index line -> orphan, ID still counts.
    local c9="$tmp/orphan"
    mkdir -p "$c9/docs/decisions/mod"
    printf '%s\n' "$empty_handoff" > "$c9/HANDOFF.md"
    printf '%s\n' '# DECISIONS' > "$c9/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c9/docs/decisions/mod/mod-9.md"
    get_id_space "$c9"; build_rows MOD
    [[ "$(count_findings_error_check decision-orphan)" -lt 1 ]] && failures+=('orphan: expected decision-orphan error, got none')
    [[ "${ROW_NEXT[0]}" != 'MOD-10' ]] && failures+=("orphan: expected MOD-10, got ${ROW_NEXT[0]}")

    # Case 10: index ID disagrees with the path it links.
    local c10="$tmp/idpath"
    mkdir -p "$c10/docs/decisions/mod"
    printf '%s\n' "$empty_handoff" > "$c10/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-6.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c10/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c10/docs/decisions/mod/mod-6.md"
    get_id_space "$c10"
    [[ "$(count_findings_error_check index-id-path)" -lt 1 ]] && failures+=('idpath: expected index-id-path error, got none')

    # Case 11: checklist lines malformed in format.
    local c11="$tmp/badchecklist"
    mkdir -p "$c11"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [x] **MOD-12 - Ticked, never deleted.** body
 - [ ] **MOD-13 - Leading space.** body
EOF
    printf '%s\n' "$content" > "$c11/HANDOFF.md"
    printf '%s\n' '# DECISIONS' > "$c11/DECISIONS.md"
    get_id_space "$c11"; build_rows MOD
    [[ "$(count_findings_error_check checklist-unparseable)" -lt 2 ]] && failures+=("badchecklist: expected 2 checklist-unparseable errors, got $(count_findings_error_check checklist-unparseable)")
    [[ "${ROW_NEXT[0]}" != 'MOD-14' ]] && failures+=("badchecklist: expected MOD-14, got ${ROW_NEXT[0]}")

    # Case 12: an ID open in HANDOFF.md AND archived — IDs are never reused.
    local c12="$tmp/dup"
    mkdir -p "$c12/docs/decisions/mod"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **MOD-8 - Thing.** body
EOF
    printf '%s\n' "$content" > "$c12/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c12/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c12/docs/decisions/mod/mod-8.md"
    get_id_space "$c12"
    [[ "$(count_findings_error_check duplicate-id)" -lt 1 ]] && failures+=('dup: expected duplicate-id error, got none')

    # Case 13: no DECISIONS.md and no docs/decisions/ is legal.
    local c13="$tmp/noarchive"
    mkdir -p "$c13"
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Open items

- [ ] **ANA-2 - Thing.** body
EOF
    printf '%s\n' "$content" > "$c13/HANDOFF.md"
    get_id_space "$c13"; build_rows ANA
    [[ "$(count_findings_error)" -ne 0 ]] && failures+=("noarchive: expected 0 errors, got $(count_findings_error)")
    [[ "${ROW_NEXT[0]}" != 'ANA-3' ]] && failures+=("noarchive: expected ANA-3, got ${ROW_NEXT[0]}")

    # Case 14: missing HANDOFF.md -> files error, exit 1 (validator parity).
    local c14="$tmp/nohandoff"
    mkdir -p "$c14"
    printf '%s\n' '# DECISIONS' > "$c14/DECISIONS.md"
    "$SCRIPT_SELF" --prefix MOD --repo-root "$c14" >/dev/null 2>&1
    [[ $? -ne 1 ]] && failures+=("nohandoff: expected exit 1, got $?")

    # Case 15: an out-of-set prefix is a usage error.
    "$SCRIPT_SELF" --prefix FOO --repo-root "$c1" >/dev/null 2>&1
    [[ $? -ne 2 ]] && failures+=("badprefix: expected exit 2, got $?")

    # Case 16: --all reports every prefix; empty ones start at 1.
    local all_out ac
    all_out="$("$SCRIPT_SELF" --all --repo-root "$c1")"
    ac=$?
    [[ $ac -ne 0 ]] && failures+=("all: expected exit 0, got $ac")
    local p
    for p in "${PREFIXES[@]}"; do
        printf '%s\n' "$all_out" | grep -qE "^${p}[[:space:]]" || failures+=("all: no row for $p")
    done
    printf '%s' "$all_out" | grep -q 'MOD-9' || failures+=('all: MOD row does not report MOD-9')
    printf '%s' "$all_out" | grep -q 'VAL-1' || failures+=('all: empty VAL row does not report VAL-1')

    # Case 17: --id-only emits exactly one bare line, for $(...) capture.
    local id_only
    id_only="$("$SCRIPT_SELF" --prefix MOD --repo-root "$c1" --id-only)"
    ac=$?
    [[ $ac -ne 0 ]] && failures+=("idonly: expected exit 0, got $ac")
    [[ "$id_only" != 'MOD-9' ]] && failures+=("idonly: expected exactly 'MOD-9', got: $id_only")

    # Case 17b: --id-only over an untrustworthy archive must emit NOTHING on stdout.
    local id_only_bad
    id_only_bad="$("$SCRIPT_SELF" --prefix MOD --repo-root "$c6" --id-only 2>/dev/null)"
    ac=$?
    [[ $ac -ne 1 ]] && failures+=("idonly-bad: expected exit 1, got $ac")
    [[ -n "$id_only_bad" ]] && failures+=("idonly-bad: expected no stdout, got: $id_only_bad")

    # Case 18: a RELATIVE --repo-root must behave identically to an absolute one.
    local rel
    rel="$(cd "$tmp" && "$SCRIPT_SELF" --prefix MOD --repo-root basic --id-only)"
    [[ "$rel" != 'MOD-9' ]] && failures+=("relativeroot: expected MOD-9 via relative root, got: $rel")

    # Case 19: supplementary prose under docs/decisions/ holds no ID.
    local c19="$tmp/supplementary"
    mkdir -p "$c19/docs/decisions/mod"
    printf '%s\n' "$empty_handoff" > "$c19/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c19/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c19/docs/decisions/mod/mod-8.md"
    printf '# Notes\n\nHow this directory is organised.\n' > "$c19/docs/decisions/README.md"
    get_id_space "$c19"
    [[ "$(count_findings_error)" -ne 0 ]] && failures+=("supplementary: expected 0 errors, got $(count_findings_error)")

    # Case 20: a lowercase prefix is accepted but CANONICALIZED.
    local lower
    lower="$("$SCRIPT_SELF" --prefix mod --repo-root "$c1" --id-only)"
    ac=$?
    [[ $ac -ne 0 ]] && failures+=("lowercase-prefix: expected exit 0, got $ac")
    [[ "$lower" != 'MOD-9' ]] && failures+=("lowercase-prefix: expected canonical 'MOD-9', got: $lower")

    # Case 21: mid-migration state — a legacy header AND a write-up file already created
    # for the same ID, not yet indexed. The ID is accounted for once.
    local c21="$tmp/midmigration"
    mkdir -p "$c21/docs/decisions/mod"
    printf '%s\n' "$empty_handoff" > "$c21/HANDOFF.md"
    read -r -d '' content <<'EOF' || true
# DECISIONS

## MOD-18/19/20 - Three items, one header (done, 2026-01-05)
EOF
    printf '%s\n' "$content" > "$c21/DECISIONS.md"
    printf '%s\n' "$write_up" > "$c21/docs/decisions/mod/mod-18.md"
    get_id_space "$c21"; build_rows MOD
    [[ "$(count_findings_error_check duplicate-id)" -ne 0 ]] && failures+=("midmigration: phantom duplicate-id")
    [[ "$(count_findings_error_check legacy-header)" -lt 1 ]] && failures+=('midmigration: expected legacy-header error, got none')
    [[ "${ROW_NEXT[0]}" != 'MOD-21' ]] && failures+=("midmigration: expected MOD-21, got ${ROW_NEXT[0]}")

    # Case 22: --json is parseable (substring checks) and carries both maxima plus findings.
    local json
    json="$("$SCRIPT_SELF" --prefix MOD --repo-root "$c1" --json)"
    case "$json" in *'"next":"MOD-9"'*) ;; *) failures+=("json: expected next MOD-9, got: $json") ;; esac
    case "$json" in *'"maxOpen":"MOD-4"'*) ;; *) failures+=('json: expected maxOpen MOD-4') ;; esac
    case "$json" in *'"maxArchived":"MOD-8"'*) ;; *) failures+=('json: expected maxArchived MOD-8') ;; esac
    case "$json" in *'"trustworthy":true'*) ;; *) failures+=('json: expected trustworthy true') ;; esac

    rm -rf "$tmp"

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: 22/22 cases PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f
    for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() {
    echo "Usage: $0 [--prefix P] [--all] [--repo-root <path>] [--id-only] [--json] [--self-test]"
}

PREFIX=""
ALL=0
REPO_ROOT=""
ID_ONLY=0
JSON=0
SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --all) ALL=1; shift ;;
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
        --id-only) ID_ONLY=1; shift ;;
        --json) JSON=1; shift ;;
        --self-test) SELF_TEST=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ $SELF_TEST -eq 1 ]]; then
    run_self_test
    exit $?
fi

if [[ -n "$PREFIX" && $ALL -eq 1 ]]; then
    echo 'Use --prefix or --all, not both.'
    exit 2
fi
if [[ -n "$PREFIX" ]]; then
    # Normalize before validating: every string this script emits (--id-only, --json, the
    # report) must be mintable verbatim — accepting `mod` and echoing back `mod-9` would put
    # a non-canonical ID into HANDOFF.md.
    PREFIX="$(printf '%s' "$PREFIX" | tr '[:lower:]' '[:upper:]')"
    if ! contains "$PREFIX" "${PREFIXES[@]}"; then
        echo "Unknown prefix '$PREFIX'. The law defines exactly six: ANA, MOD, NEXT, VAL, TOOL, CLEAN."
        exit 2
    fi
fi
if [[ $ID_ONLY -eq 1 && -z "$PREFIX" ]]; then
    echo '--id-only needs --prefix.'
    exit 2
fi

if [[ -z "$REPO_ROOT" ]]; then
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ in every repo — repo root
    # is four levels up. This is what makes the mint run against the repo it is invoked in.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$REPO_ROOT" || ! -d "$REPO_ROOT" ]]; then
    echo "RepoRoot not found: $REPO_ROOT"
    exit 2
fi
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

get_id_space "$REPO_ROOT"
error_count="$(count_findings_error)"
warning_count=0
trustworthy=1
[[ "$error_count" -gt 0 ]] && trustworthy=0
wanted="${PREFIXES[*]}"
[[ -n "$PREFIX" ]] && wanted="$PREFIX"
build_rows "$wanted"

if [[ $ID_ONLY -eq 1 ]]; then
    # Nothing on stdout when the ID space cannot be trusted: a caller capturing this must
    # never receive a mintable-looking string from a broken archive.
    if [[ $trustworthy -eq 0 ]]; then
        for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
            sev_upper="$(printf '%s' "${FINDING_SEV[i]}" | tr '[:lower:]' '[:upper:]')"
            echo "[${sev_upper}] ${FINDING_CHECK[i]}: ${FINDING_MSG[i]}" >&2
        done
        exit 1
    fi
    printf '%s\n' "${ROW_NEXT[0]}"
    exit 0
fi

if [[ $JSON -eq 1 ]]; then
    emit_json "$REPO_ROOT" "$trustworthy"
    [[ "$error_count" -gt 0 ]] && exit 1
    exit 0
fi

for ((i = 0; i < ${#FINDING_SEV[@]}; i++)); do
    sev_upper="$(printf '%s' "${FINDING_SEV[i]}" | tr '[:lower:]' '[:upper:]')"
    echo "[${sev_upper}] ${FINDING_CHECK[i]}: ${FINDING_MSG[i]}"
done

suffix=''
[[ $trustworthy -eq 0 ]] && suffix=' (UNTRUSTED - fix findings before minting)'
if [[ -n "$PREFIX" ]]; then
    printf '%s: max open %s, max archived %s -> next %s%s\n' \
        "${ROW_PREFIX[0]}" "${ROW_MAXOPEN[0]:--}" "${ROW_MAXARCH[0]:--}" "${ROW_NEXT[0]}" "$suffix"
else
    printf '%-7s %-9s %-12s %s\n' 'Prefix' 'MaxOpen' 'MaxArchived' 'Next'
    for ((i = 0; i < ${#ROW_PREFIX[@]}; i++)); do
        printf '%-7s %-9s %-12s %s%s\n' \
            "${ROW_PREFIX[i]}" "${ROW_MAXOPEN[i]:--}" "${ROW_MAXARCH[i]:--}" "${ROW_NEXT[i]}" "$suffix"
    done
fi
echo "ID space: ${error_count} error(s), ${warning_count} warning(s) in ${REPO_ROOT}"

[[ "$error_count" -gt 0 ]] && exit 1
exit 0
