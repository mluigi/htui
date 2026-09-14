#!/usr/bin/env bash
# next-item.sh — pre-filter for `/handoff-run next` (references/selection.md).
# Bash port of next-item.ps1 — keep the two in lockstep.
#
# Builds the candidate table the selection step judges: open checklist items with their
# section (R3), machine-checkable eligibility signals (E1 blocked-on cross-links, verified
# against the named repo's HANDOFF.md), in-flight evidence (R1: phase-landed note or a
# plan/PRD artifact named for the item), and local dependent count (R2). It then ranks the
# survivors and names the key that decided the top spot.
#
# It is a PRE-FILTER, not a decider — the judgment calls stay with the skill:
#   - E2 (owned elsewhere / anchor items) is reported as a `remaining` note, never a drop.
#   - A blocker inside a `Phase N (...)` parenthetical is `partial` — flagged, not dropped,
#     because the item's earlier phases may still be runnable here.
#   - An unverifiable blocker (repo HANDOFF unreadable) marks the item `blocked?`, kept.
#   - A top spot that only file order decides is reported `tie=true` — the skill must ask.
#
# Usage:
#   bash next-item.sh [--repo-root <path>] [--json] [--self-test]
#
# Exit codes: 0 candidates listed (including zero open), 1 self-test failure,
#             2 unusable --repo-root / missing HANDOFF.md.

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

KNOWN_REPOS="workspace engine vfs settings logging VulkanTutorials"
REPO_ALT="workspace|engine|vfs|settings|logging|VulkanTutorials"
BLOCKED_PATTERN="[Bb]locked on[[:space:]]+(the[[:space:]]+)?(\\*\\*)?((${REPO_ALT})[[:space:]]+)?(\\*\\*)?((${PREFIX_ALT})-[0-9]+)"
FOLLOW_PATTERN="^[,[:space:]]+(and[[:space:]]+)?(\\*\\*)?((${REPO_ALT})[[:space:]]+)?(\\*\\*)?((${PREFIX_ALT})-[0-9]+)"


US=$'\x1e'  # item-body / blocker-record separator
FS=$'\x1f'  # blocker-record field separator

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

read_file_lines() { # $1=file -> sets global READ_LINES; CRLF-tolerant
    READ_LINES=()
    [[ -f "$1" ]] || return 0
    local line
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        READ_LINES+=("$line")
    done < "$1"
}

# Section order = R3, verbatim from references/selection.md (which quotes
# workflow-docs.md). Unknown section ranks last (empty here) and is flagged by the caller.
section_rank() { # $1=section name -> stdout rank or empty
    case "$1" in
        'Next features') printf '1' ;;
        'Analyses') printf '2' ;;
        'Deferred backlog') printf '3' ;;
        'Runtime validation findings') printf '4' ;;
        'Tooling findings') printf '5' ;;
        *) printf '' ;;
    esac
}

get_workspace_root() {
    # Sub-repo roots are direct children of the workspace named after themselves; the
    # workspace root is the one whose parent holds no HANDOFF.md of its own.
    # $1=root -> stdout workspace root
    local root="$1" name parent
    name="$(basename "$root")"
    parent="$(dirname "$root")"
    case " $KNOWN_REPOS " in
        *" $name "*)
            if [[ -n "$parent" && -f "$parent/HANDOFF.md" ]]; then
                printf '%s' "$parent"
                return
            fi
            ;;
    esac
    printf '%s' "$root"
}

resolve_repo_handoff() {
    # Repo token from a blocked-on cross-link -> that repo's HANDOFF.md path, or the local
    # one when the token is empty or the repo itself. Existence NOT checked here — the
    # caller distinguishes missing (unverifiable) from closed.
    # $1=repo(may be empty) $2=root $3=workspace_root -> stdout path
    local repo="$1" root="$2" wsroot="$3" base
    base="$(basename "$root")"
    if [[ -z "$repo" || "$repo" == "$base" ]]; then printf '%s/HANDOFF.md' "$root"; return; fi
    if [[ "$repo" == 'workspace' ]]; then printf '%s/HANDOFF.md' "$wsroot"; return; fi
    printf '%s/%s/HANDOFF.md' "$wsroot" "$repo"
}

get_open_ids_only() {
    # Lightweight cross-repo open-ID lookup (strict + loose) — used only for blocker
    # membership tests, so it must NOT touch the ITEM_* globals the caller is mid-loop over.
    # $1=file -> stdout space-separated ids
    local file="$1" line ids=''
    read_file_lines "$file"
    for line in "${READ_LINES[@]}"; do
        if [[ "$line" =~ $CHECKLIST_LINE_PATTERN ]]; then
            ids+="${BASH_REMATCH[1]} "
        elif [[ "$line" =~ $CHECKLIST_LINE_LOOSE_PATTERN ]]; then
            ids+="${BASH_REMATCH[1]} "
        fi
    done
    printf '%s' "$ids"
}

get_open_items() {
    # uses global HL_LINES -> sets ITEM_ID[]/ITEM_TITLE[]/ITEM_SECTION[]/ITEM_SECTIONRANK[]/
    # ITEM_LINE[]/ITEM_BODY[] (body lines joined by US) and OPEN_WARNINGS[].
    # Body = every line after the checklist line until the next checklist line (strict or
    # loose) or a `## ` section header; HANDOFF bodies are indented continuations, so this
    # needs no indentation rule.
    ITEM_ID=(); ITEM_TITLE=(); ITEM_SECTION=(); ITEM_SECTIONRANK=(); ITEM_LINE=(); ITEM_BODY=()
    OPEN_WARNINGS=()
    local section='' current=-1 i line id title rank loose_id p1 p2
    for ((i = 0; i < ${#HL_LINES[@]}; i++)); do
        line="${HL_LINES[i]}"
        if [[ "$line" =~ ^##[[:space:]]+(.+)$ ]]; then
            section="$(trim "${BASH_REMATCH[1]}")"
            current=-1
            continue
        fi
        if [[ "$line" =~ $CHECKLIST_LINE_PATTERN ]]; then
            id="${BASH_REMATCH[1]}"
            title=''
            # [^*]* stops at the first '*' — the closing '**' — approximating the lazy match
            # ERE cannot express; falls back to "rest of line" when there's no closing '**'.
            p1="^- \\[ \\] \\*\\*${id}[[:space:]]*[-–—][[:space:]]*([^*]*)\\*\\*"
            p2="^- \\[ \\] \\*\\*${id}[[:space:]]*[-–—][[:space:]]*(.+)\$"
            if [[ "$line" =~ $p1 ]]; then
                title="${BASH_REMATCH[1]}"
            elif [[ "$line" =~ $p2 ]]; then
                title="${BASH_REMATCH[1]}"
            fi
            rank="$(section_rank "$section")"
            ITEM_ID+=("$id"); ITEM_TITLE+=("$title"); ITEM_SECTION+=("$section")
            ITEM_SECTIONRANK+=("$rank"); ITEM_LINE+=("$((i + 1))"); ITEM_BODY+=("$line")
            current=$((${#ITEM_ID[@]} - 1))
            continue
        fi
        if [[ "$line" =~ $CHECKLIST_LINE_LOOSE_PATTERN ]]; then
            # Meant as an item but unparseable — the validator errors on this; here it only
            # ends the previous body and gets a warning so the skill knows the candidate set
            # may be incomplete.
            loose_id="${BASH_REMATCH[1]}"
            OPEN_WARNINGS+=("line $((i + 1)): checklist-unparseable (run validate-workflow-docs.sh): $(trim "$line")")
            current=-1
            continue
        fi
        if [[ $current -ge 0 ]]; then
            ITEM_BODY[current]="${ITEM_BODY[current]}${US}${line}"
        fi
    done
}

get_blocked_refs() {
    # $1=item index -> sets REFS_REPO[]/REFS_ID[]/REFS_PARTIAL[]/REFS_TEXT[]. A ref inside a
    # `Phase N (...)` parenthetical is Partial: a later phase's blocker, not necessarily the
    # item's.
    REFS_REPO=(); REFS_ID=(); REFS_PARTIAL=(); REFS_TEXT=()
    local body="${ITEM_BODY[$1]}"
    local bodytext="${body//$US/ }"
    local -a phase_spans=()
    while IFS= read -r pspan; do
        [[ -n "$pspan" ]] && phase_spans+=("$pspan")
    done < <(printf '%s\n' "$bodytext" | grep -oE 'Phase[[:space:]]+[0-9]+[[:space:]]*\([^)]*\)' 2>/dev/null || true)
    
    local remaining="$bodytext" matched repo id partial span f_matched f_repo f_id f_partial
    while [[ "$remaining" =~ $BLOCKED_PATTERN ]]; do
        matched="${BASH_REMATCH[0]}"; repo="${BASH_REMATCH[4]}"; id="${BASH_REMATCH[6]}"
        partial=0
        for span in "${phase_spans[@]:-}"; do
            case "$span" in *"$matched"*) partial=1; break ;; esac
        done
        REFS_REPO+=("$repo"); REFS_ID+=("$id"); REFS_PARTIAL+=("$partial"); REFS_TEXT+=("$(trim "$matched")")
        remaining="${remaining#*"$matched"}"
        
        while [[ "$remaining" =~ $FOLLOW_PATTERN ]]; do
            f_matched="${BASH_REMATCH[0]}"; f_repo="${BASH_REMATCH[4]}"; f_id="${BASH_REMATCH[6]}"
            if [[ -z "$f_repo" ]]; then
                f_repo="$repo"
            else
                repo="$f_repo"
            fi
            f_partial=0
            for span in "${phase_spans[@]:-}"; do
                case "$span" in *"$f_matched"*) f_partial=1; break ;; esac
            done
            REFS_REPO+=("$f_repo"); REFS_ID+=("$f_id"); REFS_PARTIAL+=("$f_partial"); REFS_TEXT+=("$(trim "$f_matched")")
            remaining="${remaining#*"$f_matched"}"
        done
    done
}

get_in_flight_signals() {
    # $1=item index $2=root -> sets IN_FLIGHT_SIGNALS (comma-joined, may be empty)
    local idx="$1" root="$2" body bodytext id id_lower
    body="${ITEM_BODY[$idx]}"
    bodytext="${body//$US/ }"
    id="${ITEM_ID[$idx]}"
    id_lower="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')"
    local -a signals=()
    if [[ "$bodytext" =~ Phase[[:space:]]+[0-9]+[^.]{0,40}landed ]]; then signals+=('phase-landed'); fi
    local dir base
    for dir in '.claude/plans' '.claude/prds'; do
        if [[ -d "$root/$dir" ]] && ls "$root/$dir/${id_lower}-"* >/dev/null 2>&1; then
            base="$(basename "$dir")"
            signals+=("${base%s}")
        fi
    done
    IN_FLIGHT_SIGNALS="$(IFS=,; echo "${signals[*]:-}")"
}

get_remaining_note() {
    # E2 signal only — the anchor judgment stays with the skill.
    # $1=item index -> stdout note (whole line, trimmed) or nothing
    local body="${ITEM_BODY[$1]}"
    local -a lines=()
    IFS="$US" read -ra lines <<< "$body"
    local line
    for line in "${lines[@]}"; do
        if [[ "$line" =~ \*\*Remaining:?\*\* ]]; then
            trim "$line"
            return
        fi
    done
}

build_candidates() {
    # $1=root -> populates BC_N, ITEM_*, ITEM_ELIG/INFLIGHT/REMAINING/DEPENDENTS/BLOCKERS,
    # BC_WARNINGS_JOINED. Returns 2 (message on stderr) if HANDOFF.md is missing.
    local root="$1" handoff="$1/HANDOFF.md"
    if [[ ! -f "$handoff" ]]; then
        echo "HANDOFF.md not found at $handoff" >&2
        return 2
    fi
    local wsroot
    wsroot="$(get_workspace_root "$root")"
    read_file_lines "$handoff"
    HL_LINES=("${READ_LINES[@]}")
    get_open_items
    local n=${#ITEM_ID[@]}
    ITEM_ELIG=(); ITEM_INFLIGHT=(); ITEM_REMAINING=(); ITEM_DEPENDENTS=(); ITEM_BLOCKERS=()

    # Other repos' open-ID sets, read lazily and cached; '__UNREADABLE__' = unreadable.
    CACHE_PATH=(); CACHE_IDS=()
    local i
    for ((i = 0; i < n; i++)); do
        get_blocked_refs "$i"
        local -a bl_records=()
        local hardopen=0 unverif=0 j
        for ((j = 0; j < ${#REFS_ID[@]}; j++)); do
            local repo="${REFS_REPO[j]}" id="${REFS_ID[j]}" partial="${REFS_PARTIAL[j]}" text="${REFS_TEXT[j]}"
            local path cache_idx=-1 k
            path="$(resolve_repo_handoff "$repo" "$root" "$wsroot")"
            for ((k = 0; k < ${#CACHE_PATH[@]}; k++)); do
                if [[ "${CACHE_PATH[k]}" == "$path" ]]; then cache_idx=$k; break; fi
            done
            if [[ $cache_idx -lt 0 ]]; then
                local ids
                if [[ -f "$path" ]]; then ids="$(get_open_ids_only "$path")"; else ids='__UNREADABLE__'; fi
                CACHE_PATH+=("$path"); CACHE_IDS+=("$ids")
                cache_idx=$((${#CACHE_PATH[@]} - 1))
            fi
            local openset="${CACHE_IDS[cache_idx]}" status
            if [[ "$openset" == '__UNREADABLE__' ]]; then
                status='unverifiable'
            elif [[ " $openset " == *" $id "* ]]; then
                status='open'
            else
                status='not-open'
            fi
            local disp_repo="$repo"
            [[ -z "$disp_repo" ]] && disp_repo='local'
            bl_records+=("${disp_repo}${FS}${id}${FS}${status}${FS}${partial}${FS}${text}")
            [[ "$status" == 'open' && "$partial" -eq 0 ]] && hardopen=$((hardopen + 1))
            [[ "$status" == 'unverifiable' && "$partial" -eq 0 ]] && unverif=$((unverif + 1))
        done
        # E1 verdict: a confirmed-open, non-partial blocker drops; an unverifiable
        # non-partial blocker keeps the item as `blocked?`; everything else is eligible
        # machine-side (partial blockers and E2 anchors are the skill's call).
        local elig
        if [[ $hardopen -gt 0 ]]; then elig='blocked'
        elif [[ $unverif -gt 0 ]]; then elig='blocked?'
        else elig='eligible'; fi
        ITEM_ELIG+=("$elig")
        get_in_flight_signals "$i" "$root"
        ITEM_INFLIGHT+=("$IN_FLIGHT_SIGNALS")
        ITEM_REMAINING+=("$(get_remaining_note "$i")")
        ITEM_DEPENDENTS+=(0)
        ITEM_BLOCKERS+=("$(IFS="$US"; echo "${bl_records[*]:-}")")
    done

    # R2: open local items whose body names `blocked on ... <this ID>`.
    for ((i = 0; i < n; i++)); do
        local count=0 j
        for ((j = 0; j < n; j++)); do
            [[ $j -eq $i ]] && continue
            local -a brecs=()
            IFS="$US" read -ra brecs <<< "${ITEM_BLOCKERS[j]}"
            local rec
            for rec in "${brecs[@]:-}"; do
                [[ -z "$rec" ]] && continue
                local -a fields=()
                IFS="$FS" read -ra fields <<< "$rec"
                if [[ "${fields[0]:-}" == 'local' && "${fields[1]:-}" == "${ITEM_ID[i]}" ]]; then
                    count=$((count + 1))
                    break
                fi
            done
        done
        ITEM_DEPENDENTS[i]=$count
    done

    BC_ROOT="$root"
    BC_N="$n"
    BC_WARNINGS_JOINED="$(IFS="$US"; echo "${OPEN_WARNINGS[*]:-}")"
    return 0
}

rank_candidates() {
    # R1 in-flight, R2 dependents, R3 section, R4 file order — and name the key that
    # separated #1 from #2. R4-only separation is a tie the skill must ask about.
    # `blocked?` items rank alongside eligible ones: an unverifiable blocker on the
    # would-be winner is an ambiguity, not a drop.
    # $1=n -> sets RANKED_IDX[], RANK_DECIDEDBY, RANK_TIE(0/1)
    local n="$1" i
    local -a rankable=()
    for ((i = 0; i < n; i++)); do
        [[ "${ITEM_ELIG[i]}" != 'blocked' ]] && rankable+=("$i")
    done
    local -a keys=()
    for i in "${rankable[@]}"; do
        local infl=1
        [[ -n "${ITEM_INFLIGHT[i]}" ]] && infl=0
        local negdep=$((-ITEM_DEPENDENTS[i]))
        local srank="${ITEM_SECTIONRANK[i]}"
        [[ -z "$srank" ]] && srank=999999
        keys+=("$(printf '%d|%d|%d|%d|%d' "$infl" "$negdep" "$srank" "${ITEM_LINE[i]}" "$i")")
    done
    RANKED_IDX=()
    if [[ ${#keys[@]} -gt 0 ]]; then
        local k
        while IFS= read -r k; do
            [[ -z "$k" ]] && continue
            RANKED_IDX+=("${k##*|}")
        done < <(printf '%s\n' "${keys[@]}" | sort -t'|' -k1,1n -k2,2n -k3,3n -k4,4n)
    fi
    RANK_DECIDEDBY=''
    RANK_TIE=0
    if [[ ${#RANKED_IDX[@]} -ge 2 ]]; then
        local a="${RANKED_IDX[0]}" b="${RANKED_IDX[1]}" a_infl=0 b_infl=0
        [[ -n "${ITEM_INFLIGHT[a]}" ]] && a_infl=1
        [[ -n "${ITEM_INFLIGHT[b]}" ]] && b_infl=1
        if [[ "$a_infl" != "$b_infl" ]]; then
            RANK_DECIDEDBY='R1 in-flight'
        elif [[ "${ITEM_DEPENDENTS[a]}" != "${ITEM_DEPENDENTS[b]}" ]]; then
            RANK_DECIDEDBY='R2 unblocks-others'
        else
            local ar="${ITEM_SECTIONRANK[a]}" br="${ITEM_SECTIONRANK[b]}"
            [[ -z "$ar" ]] && ar=999999
            [[ -z "$br" ]] && br=999999
            if [[ "$ar" != "$br" ]]; then
                RANK_DECIDEDBY='R3 section-order'
            else
                RANK_TIE=1
                RANK_DECIDEDBY='R4 file-order — TIE, ask the maintainer'
            fi
        fi
    elif [[ ${#RANKED_IDX[@]} -eq 1 ]]; then
        RANK_DECIDEDBY='only eligible candidate'
    fi
}

blockers_text_for_item() {
    # $1=item index -> stdout, one "repo id: status (phase-scoped)" per line
    local -a brecs=()
    IFS="$US" read -ra brecs <<< "${ITEM_BLOCKERS[$1]}"
    local rec
    for rec in "${brecs[@]:-}"; do
        [[ -z "$rec" ]] && continue
        local -a f=()
        IFS="$FS" read -ra f <<< "$rec"
        local suffix=''
        [[ "${f[3]:-0}" == 1 ]] && suffix=' (phase-scoped)'
        printf '%s %s: %s%s\n' "${f[0]}" "${f[1]}" "${f[2]}" "$suffix"
    done
}

invoke_next_item() {
    # $1=root $2=as_json(0/1)
    local root="$1" as_json="$2"
    build_candidates "$root" || return $?
    local n="$BC_N"
    rank_candidates "$n"

    local rankable_count=${#RANKED_IDX[@]}
    if [[ "$as_json" -eq 1 ]]; then
        local out i idx first
        out='{'
        out+="\"repoRoot\":\"$(json_escape "$BC_ROOT")\","
        out+="\"open\":$n,\"rankable\":$rankable_count,"
        if [[ -n "$RANK_DECIDEDBY" ]]; then out+="\"decidedBy\":\"$(json_escape "$RANK_DECIDEDBY")\","; else out+='"decidedBy":null,'; fi
        [[ "$RANK_TIE" -eq 1 ]] && out+='"tie":true,' || out+='"tie":false,'
        out+='"warnings":['
        first=1
        local -a warns=()
        IFS="$US" read -ra warns <<< "$BC_WARNINGS_JOINED"
        for w in "${warns[@]:-}"; do
            [[ -z "$w" ]] && continue
            [[ $first -eq 1 ]] || out+=','
            first=0
            out+="\"$(json_escape "$w")\""
        done
        out+='],"candidates":['
        first=1
        for idx in "${RANKED_IDX[@]:-}"; do
            [[ $first -eq 1 ]] || out+=','
            first=0
            out+='{'
            out+="\"id\":\"${ITEM_ID[idx]}\",\"title\":\"$(json_escape "${ITEM_TITLE[idx]}")\","
            out+="\"section\":\"$(json_escape "${ITEM_SECTION[idx]}")\",\"line\":${ITEM_LINE[idx]},"
            out+="\"eligibility\":\"${ITEM_ELIG[idx]}\",\"dependents\":${ITEM_DEPENDENTS[idx]}"
            out+='}'
        done
        out+=']}'
        printf '%s\n' "$out"
        return 0
    fi

    echo "next-item pre-filter — ${n} open, ${rankable_count} rankable in ${root}"
    local -a warns=()
    IFS="$US" read -ra warns <<< "$BC_WARNINGS_JOINED"
    local w
    for w in "${warns[@]:-}"; do
        [[ -n "$w" ]] && echo "  [WARN] $w"
    done
    local pos=0 idx
    for idx in "${RANKED_IDX[@]:-}"; do
        pos=$((pos + 1))
        local -a flags=()
        [[ "${ITEM_ELIG[idx]}" == 'blocked?' ]] && flags+=('blocked?')
        [[ -n "${ITEM_INFLIGHT[idx]}" ]] && flags+=("in-flight: ${ITEM_INFLIGHT[idx]}")
        [[ "${ITEM_DEPENDENTS[idx]}" -gt 0 ]] && flags+=("unblocks ${ITEM_DEPENDENTS[idx]}")
        [[ -n "${ITEM_REMAINING[idx]}" ]] && flags+=('E2? has Remaining note')
        local flagtext=''
        if [[ ${#flags[@]} -gt 0 ]]; then flagtext=" [$(IFS='; '; echo "${flags[*]}")]"; fi
        printf '  %d. %s (%s, L%s)%s\n' "$pos" "${ITEM_ID[idx]}" "${ITEM_SECTION[idx]}" "${ITEM_LINE[idx]}" "$flagtext"
        while IFS= read -r bline; do
            [[ -n "$bline" ]] && echo "       blocker: $bline"
        done < <(blockers_text_for_item "$idx")
    done
    for ((i = 0; i < n; i++)); do
        [[ "${ITEM_ELIG[i]}" == 'blocked' ]] || continue
        local -a whys=()
        local -a brecs=()
        IFS="$US" read -ra brecs <<< "${ITEM_BLOCKERS[i]}"
        local rec
        for rec in "${brecs[@]:-}"; do
            [[ -z "$rec" ]] && continue
            local -a f=()
            IFS="$FS" read -ra f <<< "$rec"
            [[ "${f[2]}" == 'open' && "${f[3]:-0}" == 0 ]] && whys+=("blocked on ${f[0]} ${f[1]}")
        done
        echo "  ineligible: ${ITEM_ID[i]} — $(IFS='; '; echo "${whys[*]:-}")"
    done
    [[ -n "$RANK_DECIDEDBY" ]] && echo "  top decided by: $RANK_DECIDEDBY"
    [[ "$RANK_TIE" -eq 1 ]] && echo '  TIE — selection.md step 4: ask, never pick by file order.'
    return 0
}

# ---- self-test ----

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/next-item-selftest-XXXXXX")"
    local -a failures=()

    # Workspace fixture with two sub-repos; script under test targets 'engine'.
    local ws="$tmp/ws" eng="$tmp/ws/engine" vfs="$tmp/ws/vfs"
    mkdir -p "$eng" "$vfs"
    printf '%s\n' '# HANDOFF' '' '## Next features' '' '- [ ] **MOD-1 - Workspace thing.** body' > "$ws/HANDOFF.md"
    printf '%s\n' '# HANDOFF' '' '## Next features' '' '- [ ] **MOD-2 - Open vfs item.** body' > "$vfs/HANDOFF.md"
    local content
    read -r -d '' content <<'EOF' || true
# HANDOFF

## Next features

- [ ] **MOD-7 - Blocked hard.** Body is blocked on vfs MOD-2 supplying the format.
- [ ] **MOD-8 - Blocked on a closed item.** Body is blocked on vfs MOD-9 which closed.
- [ ] **MOD-9 - Unverifiable.** Body is blocked on settings TOOL-3 (repo absent here).
- [ ] **MOD-10 - Depends locally.** Body is blocked on MOD-11 landing first.
- [ ] **MOD-11 - The unblocker.** body
- [ ] **MOD-12 - Phase-scoped.** Phase 1 free. Phase 2 (blocked on vfs MOD-2) later.
- [ ] **MOD-13 - Two ID blocker.** Blocked on MOD-11, MOD-14.
- [ ] **MOD-14 - Wrapped blocker.** Blocked on
MOD-11, and MOD-15 wrapping to the next line.
- [ ] **MOD-15 - Just another.** body

## Analyses

- [ ] **ANA-3 - Question.** body

## Deferred backlog

- [ ] **CLEAN-1 - Someday.** body
- [x] **CLEAN-2 - Ticked, left instead of deleted.** body
EOF
    printf '%s\n' "$content" > "$eng/HANDOFF.md"

    build_candidates "$eng"
    rank_candidates "$BC_N"
    local -a saved_id=("${ITEM_ID[@]}") saved_elig=("${ITEM_ELIG[@]}") saved_infl=("${ITEM_INFLIGHT[@]}")
    local -a saved_dep=("${ITEM_DEPENDENTS[@]}") saved_srank=("${ITEM_SECTIONRANK[@]}") saved_blockers=("${ITEM_BLOCKERS[@]}")
    idx_of() { local want="$1" i; for ((i = 0; i < ${#saved_id[@]}; i++)); do [[ "${saved_id[i]}" == "$want" ]] && { printf '%s' "$i"; return; }; done; printf -- '-1'; }

    [[ "$BC_N" -ne 11 ]] && failures+=("parse: expected 11 open items, got $BC_N")
    local i7 i8 i9 i12 i11 i13 i14 i15 i1clean
    i7="$(idx_of MOD-7)"; i8="$(idx_of MOD-8)"; i9="$(idx_of MOD-9)"; i12="$(idx_of MOD-12)"; i11="$(idx_of MOD-11)"; i13="$(idx_of MOD-13)"; i14="$(idx_of MOD-14)"; i15="$(idx_of MOD-15)"; i1clean="$(idx_of CLEAN-1)"
    [[ "${saved_elig[i7]}" != 'blocked' ]] && failures+=("E1-open: MOD-7 expected blocked, got ${saved_elig[i7]}")
    [[ "${saved_elig[i8]}" != 'eligible' ]] && failures+=("E1-closed: MOD-8 expected eligible, got ${saved_elig[i8]}")
    [[ "${saved_elig[i9]}" != 'blocked?' ]] && failures+=("E1-unverifiable: MOD-9 expected blocked?, got ${saved_elig[i9]}")
    [[ "${saved_elig[i12]}" != 'eligible' ]] && failures+=("phase-scoped: MOD-12 expected eligible, got ${saved_elig[i12]}")
    case "${saved_blockers[i12]}" in *"${FS}1${FS}"*) ;; *) failures+=('phase-scoped: MOD-12 blocker not marked Partial') ;; esac
    [[ "${saved_dep[i11]}" -ne 3 ]] && failures+=("R2: MOD-11 expected 3 dependents, got ${saved_dep[i11]}")
    [[ "${saved_dep[i14]}" -ne 1 ]] && failures+=("R2: MOD-14 expected 1 dependent, got ${saved_dep[i14]}")
    [[ "${saved_dep[i15]}" -ne 1 ]] && failures+=("R2: MOD-15 expected 1 dependent, got ${saved_dep[i15]}")
    [[ "${saved_srank[i1clean]}" != '3' ]] && failures+=("R3: CLEAN-1 expected section rank 3, got ${saved_srank[i1clean]}")
    [[ "${saved_id[${RANKED_IDX[0]}]}" != 'MOD-11' ]] && failures+=("rank: expected MOD-11 first (R2), got ${saved_id[${RANKED_IDX[0]}]}")
    [[ "$RANK_DECIDEDBY" != 'R2 unblocks-others' ]] && failures+=("decidedBy: expected R2, got $RANK_DECIDEDBY")
    local -a warns0=()
    IFS="$US" read -ra warns0 <<< "$BC_WARNINGS_JOINED"
    [[ ${#warns0[@]} -lt 1 || -z "${warns0[0]:-}" ]] && failures+=('loose: expected a checklist-unparseable warning for CLEAN-2')

    # R1 beats R2: give ANA-3 a referenced plan artifact, it must outrank MOD-11.
    mkdir -p "$eng/.claude/plans"
    printf 'plan\n' > "$eng/.claude/plans/ana-3-question.plan.md"
    build_candidates "$eng"
    rank_candidates "$BC_N"
    local -a saved_id2=("${ITEM_ID[@]}")
    [[ "${saved_id2[${RANKED_IDX[0]}]}" != 'ANA-3' ]] && failures+=("R1: expected in-flight ANA-3 first, got ${saved_id2[${RANKED_IDX[0]}]}")
    [[ "$RANK_DECIDEDBY" != 'R1 in-flight' ]] && failures+=("R1 decidedBy: got $RANK_DECIDEDBY")

    # Tie: two plain same-section items and nothing else.
    local tierepo="$tmp/tie"
    mkdir -p "$tierepo"
    printf '%s\n' '# HANDOFF' '' '## Next features' '' '- [ ] **MOD-1 - First.** body' '- [ ] **MOD-2 - Second.** body' > "$tierepo/HANDOFF.md"
    build_candidates "$tierepo"
    rank_candidates "$BC_N"
    [[ "$RANK_TIE" -ne 1 ]] && failures+=('tie: expected tie=true for two plain items')

    rm -rf "$tmp"

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: 17/17 assertions PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f
    for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() {
    echo "Usage: $0 [--repo-root <path>] [--json] [--self-test]"
}

REPO_ROOT=""
JSON=0
SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo-root) REPO_ROOT="$2"; shift 2 ;;
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

if [[ -z "$REPO_ROOT" ]]; then
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — repo root is four
    # levels up (same convention as the validator).
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$REPO_ROOT" || ! -d "$REPO_ROOT" ]]; then
    echo "RepoRoot not found: $REPO_ROOT"
    exit 2
fi
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

invoke_next_item "$REPO_ROOT" "$JSON"
exit $?
