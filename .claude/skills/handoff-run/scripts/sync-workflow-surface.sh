#!/usr/bin/env bash
# sync-workflow-surface.sh — distribute the workflow surface from the workspace
# (single source) into sub-repos: .claude/rules/workflow-docs.md, the
# .claude/skills/handoff-docs.md overview, plus the
# .claude/skills/handoff-run and handoff-add
# directories. Mirror semantics: synced dirs are wholly owned by the surface —
# stale files in targets are removed. VulkanTutorials is deliberately not a
# target (frozen donor repo, no work items run there). Bash port of
# sync-workflow-surface.ps1 — keep the two in lockstep.
#
# Usage:
#   bash sync-workflow-surface.sh [--workspace-root <path>] [--targets <names>] [--check]
#
# --targets takes a space- or comma-separated list, default
# "engine ../engine-template?". A target is an ADDRESS, not a
# child name: a child ('engine'), a sibling outside the tree ('../engine-template')
# and an absolute path all work. A trailing '?' marks the target OPTIONAL — absent
# on this checkout warns and skips; a required target that is missing still exits 2.
# --check: no writes; SHA256-compare every synced file across all targets, list
#          drift (missing/extra/modified), exit 1 on any.
#
# Exit codes: 0 synced/clean, 1 drift found (--check), 2 usage/missing paths.

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

SYNC_FILES="rules/workflow-docs.md rules/concept-docs.md skills/handoff-docs.md"
SYNC_DIRS="skills/handoff-run skills/handoff-add"

usage() {
    echo "Usage: $0 [--workspace-root <path>] [--targets <names>] [--check]"
}

sha256_of() {
    # CR-stripped: line endings are ENCODING, not content. engine-template carries
    # '* text=auto eol=lf' (so its build.sh runs on Linux), so its checked-out surface is
    # LF while a Windows workspace source is CRLF — a raw hash would report all 22 files
    # as drift in a fresh clone and block every commit that stages one. Git itself
    # compares normalized, and so does this. Kept in lockstep with Get-ContentHash in
    # sync-workflow-surface.ps1.
    if command -v sha256sum >/dev/null 2>&1; then
        tr -d '\r' < "$1" | sha256sum | awk '{print $1}'
    else
        tr -d '\r' < "$1" | shasum -a 256 | awk '{print $1}'
    fi
}

resolve_target_entry() {
    # $1=workspace_root $2=entry -> sets RESOLVED_ROOT, RESOLVED_LABEL, RESOLVED_OPTIONAL.
    # Kept in lockstep with Resolve-TargetEntry in sync-workflow-surface.ps1 and both
    # install-workflow-hooks variants. An entry starting with '/' is already rooted and
    # is used as-is; anything else joins the workspace root. Normalization goes through
    # cd&&pwd, which only works on a path that exists — so a missing target keeps the
    # un-normalized string, which is exactly what its "not found" message should print.
    local root="$1" entry="$2" addr
    RESOLVED_OPTIONAL=0
    addr="$entry"
    case "$addr" in
        *\?) RESOLVED_OPTIONAL=1; addr="${addr%\?}" ;;
    esac
    # Rooted covers a drive letter too: this runs under Git Bash beside a .ps1 twin that
    # resolves 'C:\...' correctly through Path.Combine, and joining one onto the workspace
    # root would produce '<root>/C:/...' — which does not exist, so an optional target
    # would report "not found, skipping" for a repo that is right there.
    case "$addr" in
        /*|[A-Za-z]:[/\\]*) RESOLVED_ROOT="$addr" ;;
        *)                  RESOLVED_ROOT="$root/$addr" ;;
    esac
    if [[ -d "$RESOLVED_ROOT" ]]; then
        RESOLVED_ROOT="$(cd "$RESOLVED_ROOT" && pwd)"
    fi
    RESOLVED_LABEL="$(basename "$RESOLVED_ROOT")"
}

get_synced_rel_paths() {
    # $1=claude_dir -> stdout, one repo-relative-to-.claude path per line
    local claude_dir="$1" rel
    for rel in $SYNC_FILES; do
        [[ -f "$claude_dir/$rel" ]] && printf '%s\n' "$rel"
    done
    for rel in $SYNC_DIRS; do
        [[ -d "$claude_dir/$rel" ]] || continue
        (cd "$claude_dir" && find "$rel" -type f | sort)
    done
}

WORKSPACE_ROOT=""
TARGETS_RAW=""
CHECK=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --workspace-root) WORKSPACE_ROOT="$2"; shift 2 ;;
        --targets) TARGETS_RAW="$2"; shift 2 ;;
        --check) CHECK=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ -z "$WORKSPACE_ROOT" ]]; then
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$WORKSPACE_ROOT" || ! -d "$WORKSPACE_ROOT" ]]; then
    echo "WorkspaceRoot not found: $WORKSPACE_ROOT"
    exit 2
fi
# Absolute, so every downstream path built from it is unambiguous regardless of
# whether the caller passed a relative root (a sub-repo pre-commit hook passes "..").
WORKSPACE_ROOT="$(cd "$WORKSPACE_ROOT" && pwd)"

# engine-template lives OUTSIDE the workspace tree (MOD-43) and is optional: it is a
# standalone repo, so a checkout without it must still sync the rest.
TARGETS="engine ../engine-template?"
if [[ -n "$TARGETS_RAW" ]]; then
    TARGETS="$(printf '%s' "$TARGETS_RAW" | tr ',' ' ')"
fi

SOURCE_CLAUDE="$WORKSPACE_ROOT/.claude"
for rel in $SYNC_FILES; do
    if [[ ! -f "$SOURCE_CLAUDE/$rel" ]]; then
        echo "Source file missing: .claude/$rel"
        exit 2
    fi
done
for rel in $SYNC_DIRS; do
    if [[ ! -d "$SOURCE_CLAUDE/$rel" ]]; then
        echo "Source dir missing: .claude/$rel"
        exit 2
    fi
done

SOURCE_RELS="$(get_synced_rel_paths "$SOURCE_CLAUDE")"
source_count="$(printf '%s\n' "$SOURCE_RELS" | grep -c . || true)"

drift_count=0
target_count=0

for t in $TARGETS; do
    resolve_target_entry "$WORKSPACE_ROOT" "$t"
    target_root="$RESOLVED_ROOT"
    label="$RESOLVED_LABEL"
    if [[ ! -d "$target_root" ]]; then
        if [[ $RESOLVED_OPTIONAL -eq 1 ]]; then
            echo "Target repo not found (optional, skipping): $target_root"
            continue
        fi
        echo "Target repo not found: $target_root"
        exit 2
    fi
    target_count=$((target_count + 1))
    target_claude="$target_root/.claude"

    if [[ $CHECK -eq 1 ]]; then
        target_rels="$(get_synced_rel_paths "$target_claude")"
        while IFS= read -r rel; do
            [[ -z "$rel" ]] && continue
            src_file="$SOURCE_CLAUDE/$rel"
            dst_file="$target_claude/$rel"
            if [[ ! -f "$dst_file" ]]; then
                echo "[DRIFT] ${label}: missing .claude/$rel"
                drift_count=$((drift_count + 1))
            elif [[ "$(sha256_of "$src_file")" != "$(sha256_of "$dst_file")" ]]; then
                echo "[DRIFT] ${label}: modified .claude/$rel"
                drift_count=$((drift_count + 1))
            fi
        done <<< "$SOURCE_RELS"
        while IFS= read -r rel; do
            [[ -z "$rel" ]] && continue
            if ! printf '%s\n' "$SOURCE_RELS" | grep -Fxq "$rel"; then
                echo "[DRIFT] ${label}: extra .claude/$rel"
                drift_count=$((drift_count + 1))
            fi
        done <<< "$target_rels"
    else
        for rel in $SYNC_FILES; do
            dst="$target_claude/$rel"
            mkdir -p "$(dirname "$dst")"
            cp -f "$SOURCE_CLAUDE/$rel" "$dst"
        done
        for rel in $SYNC_DIRS; do
            dst="$target_claude/$rel"
            rm -rf "$dst"
            mkdir -p "$(dirname "$dst")"
            cp -R "$SOURCE_CLAUDE/$rel" "$dst"
        done
        echo "Synced ${source_count} files -> $label"
    fi
done

if [[ $CHECK -eq 1 ]]; then
    if [[ $drift_count -gt 0 ]]; then
        echo "workflow-surface check: ${drift_count} drift finding(s) across ${target_count} target(s)."
        exit 1
    fi
    echo "workflow-surface check: clean — ${source_count} files identical in ${target_count} target(s)."
fi
exit 0
