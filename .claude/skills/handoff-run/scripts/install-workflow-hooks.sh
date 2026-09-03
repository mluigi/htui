#!/usr/bin/env bash
# install-workflow-hooks.sh — install/refresh the workflow git hooks (M3) in the
# workspace and the synced sub-repos. Hooks live in .git/hooks (untracked), so the
# file sync cannot carry them — this installer is the delivery mechanism. Bash
# port of install-workflow-hooks.ps1 — keep the two in lockstep. Unlike the ps1
# original, the hook bodies this installer generates call the .sh siblings
# directly (sh "...validate-workflow-docs.sh" / "...sync-workflow-surface.sh")
# instead of pwsh -File ...ps1 — hook execution on macOS/Linux no longer needs
# pwsh at all once this installer has run.
#
# What it installs (idempotent marker blocks, '# workflow-hook-start/end'):
#   pre-commit  (workspace + sub-repos): staged HANDOFF.md/DECISIONS.md/docs/decisions/**/docs/ANA-*.md
#               -> run validate-workflow-docs.sh --scope-paths "<staged>", block on errors
#               in the staged files (exit 1). The validator still checks the whole repo and
#               still prints everything it finds; --scope-paths only stops findings in
#               untouched files from failing an unrelated commit.
#               Sub-repos additionally block staged edits to synced surface files
#               (owned by the workspace source — drift by construction).
#   post-commit (workspace only, prepended before foreign blocks): commit touching
#               surface source -> run sync-workflow-surface.sh + refresh hooks.
#
# Escape hatch (printed on block): WORKFLOW_SKIP_HOOK=1. Deliberately NOT --no-verify: a
# separate PreToolUse hook hard-blocks that flag for agent sessions, so advertising it
# would be dead advice - and after the --scope-paths change an unrelated commit needs
# neither hatch.
#
# Usage:
#   bash install-workflow-hooks.sh [--workspace-root <path>] [--targets <addresses>] [--self-test]
#
# A target is an ADDRESS, not a child name: 'engine', '../engine-template' and an
# absolute path all work; a trailing '?' marks it optional (absent = warn and skip).
# Same convention as sync-workflow-surface.{ps1,sh}.
#
# Exit codes: 0 installed, 1 self-test failure, 2 usage/missing paths.

set -u

# Windows runs the .ps1 twin of this script, never this one — refuse rather than be slow.
# It matters most HERE: the hooks this variant generates call `bash .../*.sh`, so a single
# Windows run of it replaces every pwsh call site with a bash one and every later commit
# pays the difference. Measured 2026-08-21: a bash-installed pre-commit hook ran >2 min and
# was killed where the pwsh one took 1.3 s, and bash subprocesses on that box die on fork
# (0xC0000142) often enough that a .sh failure reads as a hang instead of an error.
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

START_MARKER='# workflow-hook-start'
END_MARKER='# workflow-hook-end'
# Single source of truth for the sh-side path regexes (kept identical across variants).
DOCS_PATTERN='^HANDOFF\.md$|^DECISIONS\.md$|^docs/decisions/.+\.md$|^docs/ANA-[^/]+\.md$'
SURFACE_PATTERN='^\.claude/rules/(workflow-docs|concept-docs)\.md$|^\.claude/skills/handoff-docs\.md$|^\.claude/skills/handoff-(run|add)/'

resolve_target_entry() {
    # $1=workspace_root $2=entry -> sets RESOLVED_ROOT, RESOLVED_LABEL, RESOLVED_OPTIONAL.
    # Duplicated VERBATIM from sync-workflow-surface.sh (standalone scripts, no shared
    # library) — keep both, and the two .ps1 twins, in lockstep. A target is an address:
    # a workspace child, a sibling outside the tree, or an absolute path; a trailing '?'
    # marks it optional.
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

trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

convert_to_lf_lines() {
    # $1=file -> sets global LF_LINES. Whole-file CRLF->LF normalization (git sh hooks
    # choke on stray \r); a file without a trailing newline keeps its last line intact
    # (the `|| [[ -n "$line" ]]` catches the unterminated final read).
    local file="$1" line
    LF_LINES=()
    [[ -s "$file" ]] || return 0
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        LF_LINES+=("$line")
    done < "$file"
}

set_marker_block() {
    # $1=hook_path $2=insert_after_shebang(0/1); block lines come from global BLOCK_LINES.
    # Idempotent insert/replace of the workflow block. Foreign blocks (e.g. graphify's)
    # are untouched by construction: only splice between our own exact markers, or
    # insert where no marker exists.
    local hook_path="$1" insert_after_shebang="$2"
    local -a lines=()
    if [[ -s "$hook_path" ]]; then
        convert_to_lf_lines "$hook_path"
        lines=("${LF_LINES[@]}")
        local nonblank=0 l
        for l in "${lines[@]}"; do
            if [[ -n "$(trim "$l")" ]]; then nonblank=1; break; fi
        done
        # Empty/whitespace-only existing file: treat as fresh so it still gets a shebang.
        [[ $nonblank -eq 1 ]] || lines=('#!/bin/sh')
    else
        lines=('#!/bin/sh')
    fi

    local start=-1 end=-1 i
    for ((i = 0; i < ${#lines[@]}; i++)); do
        if [[ "$(trim "${lines[i]}")" == "$START_MARKER" ]]; then start=$i; break; fi
    done

    local -a new_lines=()
    if [[ $start -ge 0 ]]; then
        for ((i = start + 1; i < ${#lines[@]}; i++)); do
            if [[ "$(trim "${lines[i]}")" == "$END_MARKER" ]]; then end=$i; break; fi
        done
        if [[ $end -lt 0 ]]; then
            echo "corrupted workflow-hook block in ${hook_path}: start marker without matching end - fix by hand" >&2
            exit 1
        fi
        local -a pre=() post=()
        [[ $start -gt 0 ]] && pre=("${lines[@]:0:start}")
        [[ $((end + 1)) -lt ${#lines[@]} ]] && post=("${lines[@]:$((end + 1))}")
        new_lines=("${pre[@]}" "${BLOCK_LINES[@]}" "${post[@]}")
    elif [[ $insert_after_shebang -eq 1 && ${#lines[@]} -gt 0 && "${lines[0]}" == '#!'* ]]; then
        # Prepend right after the shebang: foreign blocks below may exit 0 early and
        # must not swallow this block.
        local -a rest=()
        [[ ${#lines[@]} -gt 1 ]] && rest=("${lines[@]:1}")
        new_lines=("${lines[0]}" '' "${BLOCK_LINES[@]}" "${rest[@]}")
    else
        new_lines=("${lines[@]}" '' "${BLOCK_LINES[@]}")
    fi

    printf '%s\n' "${new_lines[@]}" > "$hook_path"
    # Git requires the execute bit on Unix hook files (unlike Windows, where the ps1
    # original never needed this) — a hook git can't exec is silently skipped, not an error.
    chmod +x "$hook_path"
}

sh_single_quoted() {
    # A path going into a single-quoted POSIX string. An apostrophe in it (a redirected
    # profile, a user named O'Brien) would otherwise close the string early and leave the
    # rest of the path as shell code in a hook that runs on every commit. The POSIX idiom
    # for a literal quote inside single quotes is '\'' — close, escaped quote, reopen.
    # Kept in lockstep with ConvertTo-ShSingleQuoted in install-workflow-hooks.ps1.
    printf '%s' "${1//\'/\'\\\'\'}"
}

new_pre_commit_block() {
    # $1=is_subrepo(0/1) [$2=workspace_root_literal $3=target_literal] -> sets BLOCK_LINES.
    # The two literals are baked in at install time for a sub-repo: see the comment in
    # the guard below for why cwd-derived values cannot work for an external target.
    # No default for either: a cwd-derived fallback ('..', '.') is exactly the broken
    # addressing this function exists to remove, and a silent one would be untestable.
    if [[ "$1" -eq 1 && ( $# -lt 3 || -z "${2:-}" || -z "${3:-}" ) ]]; then
        echo "new_pre_commit_block: sub-repo block needs workspace-root and target literals" >&2
        exit 2
    fi
    # No 'exit 0' anywhere in this block: it is prepended before any foreign hook blocks,
    # and an early exit 0 here would swallow them. Only 'exit 1' (a deliberate block) may
    # terminate; every pass path falls through past the end marker.
    BLOCK_LINES=(
        "$START_MARKER"
        '# Blocks commits with structural findings in workflow docs (HANDOFF.md/DECISIONS.md/docs/decisions/'
        '# docs/ANA-*.md). Installed by: bash .claude/skills/handoff-run/scripts/install-workflow-hooks.sh'
        '# Escape hatch: WORKFLOW_SKIP_HOOK=1. Bypassing git hooks outright is blocked for'
        '# agent sessions, and an unrelated commit needs no hatch: only findings in the'
        '# files this commit stages block it.'
        ''
        'WF_STAGED=$(git diff --cached --name-only)'
        "WF_DOCS_PATTERN='$DOCS_PATTERN'"
        "WF_SURFACE_PATTERN='$SURFACE_PATTERN'"
        ''
        '# Fast path: validator only spawns when watched paths are staged.'
        'if [ -n "$WF_STAGED" ] && [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ] \'
        '   && echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN|$WF_SURFACE_PATTERN"; then'
    )
    if [[ "$1" -eq 1 ]]; then
        # Drift-aware guard: committing surface files that MATCH the workspace source is
        # legitimate (that's how sync results get committed); only divergence blocks.
        BLOCK_LINES+=(
            '    WF_SURFACE_HITS=$(echo "$WF_STAGED" | grep -E "$WF_SURFACE_PATTERN")'
            '    if [ -n "$WF_SURFACE_HITS" ]; then'
            '        # Both paths are baked in at install time. Deriving them from cwd'
            '        # ("..", basename $(pwd)) only works for a repo that is a CHILD of the'
            '        # workspace; a sibling target resolves ".." to a directory with no'
            '        # .claude source at all, and the guard would block every commit.'
            "        WF_WORKSPACE_ROOT='$(sh_single_quoted "$2")'"
            "        WF_TARGET='$(sh_single_quoted "$3")'"
            '        if ! bash ".claude/skills/handoff-run/scripts/sync-workflow-surface.sh" --workspace-root "$WF_WORKSPACE_ROOT" --targets "$WF_TARGET" --check >/dev/null 2>&1; then'
            '            echo "[workflow-hook] blocked: staged workflow-surface files differ from the workspace source." >&2'
            '            echo "[workflow-hook] The surface is owned by the workspace - edit ../.claude/... there and run" >&2'
            '            echo "[workflow-hook] sync-workflow-surface.sh instead of editing the copy here. Staged paths:" >&2'
            '            echo "$WF_SURFACE_HITS" | sed '\''s/^/[workflow-hook]   /'\'' >&2'
            '            echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
            '            exit 1'
            '        fi'
            '    fi'
        )
    fi
    BLOCK_LINES+=(
        '    if echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN"; then'
        '        # --scope-paths: whole-repo checks, but only findings in the staged files may'
        '        # block. Pre-existing findings elsewhere print as [WARNING] (pre-existing ...)'
        '        # and still fail a direct validate-workflow-docs.sh run.'
        '        bash ".claude/skills/handoff-run/scripts/validate-workflow-docs.sh" --scope-paths "$WF_STAGED"'
        '        wf_status=$?'
        '        if [ "$wf_status" -ne 0 ]; then'
        '            echo "[workflow-hook] validate-workflow-docs.sh exited $wf_status - findings above, in files this commit stages." >&2'
        '            echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
        '            exit 1'
        '        fi'
        '    fi'
        'fi'
        "$END_MARKER"
    )
}

new_post_commit_block() {
    # -> sets global BLOCK_LINES.
    BLOCK_LINES=(
        "$START_MARKER"
        '# Non-blocking: after a workspace commit touching the workflow-surface source, mirrors'
        '# it into the sub-repos and refreshes their git hooks (hooks are untracked, so the'
        '# file sync alone cannot carry them). Deliberately placed before other hook blocks:'
        '# blocks below may exit 0 early and must not swallow this one.'
        '# Installed by: bash .claude/skills/handoff-run/scripts/install-workflow-hooks.sh'
        ''
        'if [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ]; then'
        '    WF_CHANGED=$(git diff-tree --no-commit-id --name-only -r HEAD 2>/dev/null)'
        "    WF_SURFACE_PATTERN='$SURFACE_PATTERN'"
        '    if [ -n "$WF_CHANGED" ] && echo "$WF_CHANGED" | grep -E -q "$WF_SURFACE_PATTERN"; then'
        '        echo "[workflow-hook] surface source changed - syncing to sub-repos..."'
        '        bash ".claude/skills/handoff-run/scripts/sync-workflow-surface.sh"'
        '        bash ".claude/skills/handoff-run/scripts/install-workflow-hooks.sh"'
        '    fi'
        'fi'
        "$END_MARKER"
    )
}

get_hook_targets() {
    # $1=root $2=names(space-separated) -> sets TARGET_NAMES/TARGET_ROOTS/TARGET_IS_WORKSPACE
    local root="$1" names="$2" n
    [[ -d "$root" ]] && root="$(cd "$root" && pwd)"
    TARGET_NAMES=('workspace'); TARGET_ROOTS=("$root"); TARGET_IS_WORKSPACE=(1)
    for n in $names; do
        resolve_target_entry "$root" "$n"
        if [[ ! -d "$RESOLVED_ROOT" ]]; then
            if [[ $RESOLVED_OPTIONAL -eq 1 ]]; then
                echo "Target repo not found (optional, skipping): $RESOLVED_ROOT"
                continue
            fi
            echo "Target repo not found: $RESOLVED_ROOT"
            exit 2
        fi
        TARGET_NAMES+=("$RESOLVED_LABEL"); TARGET_ROOTS+=("$RESOLVED_ROOT"); TARGET_IS_WORKSPACE+=(0)
    done
}

install_workflow_hooks() {
    # $1=quiet(0/1); uses TARGET_* arrays.
    # Index 0 is always the workspace entry (get_hook_targets' contract); its root is
    # baked into every sub-repo drift guard so an external target resolves the source
    # correctly instead of guessing from its own cwd.
    local quiet="${1:-0}" i hooks_dir is_subrepo git_dir workspace_root_literal
    workspace_root_literal="${TARGET_ROOTS[0]}"
    for ((i = 0; i < ${#TARGET_NAMES[@]}; i++)); do
        # Not a plain .git/hooks join: a submodule's .git is a "gitdir: ..." pointer file,
        # so its real hooks dir lives under the superproject's .git/modules/<name>/hooks.
        # rev-parse --absolute-git-dir resolves both cases correctly.
        git_dir="$(git -C "${TARGET_ROOTS[i]}" rev-parse --absolute-git-dir 2>/dev/null)"
        if [[ -z "$git_dir" ]]; then
            echo "Not a git repo: ${TARGET_ROOTS[i]}"
            exit 2
        fi
        hooks_dir="$git_dir/hooks"
        if [[ ! -d "$hooks_dir" ]]; then
            echo "Hooks dir not found: $hooks_dir"
            exit 2
        fi
        # InsertAfterShebang for both hooks: a foreign block with an early exit 0 must
        # never sit above ours and swallow it.
        if [[ "${TARGET_IS_WORKSPACE[i]}" -eq 1 ]]; then is_subrepo=0; else is_subrepo=1; fi
        if [[ $is_subrepo -eq 1 ]]; then
            new_pre_commit_block "$is_subrepo" "$workspace_root_literal" "${TARGET_ROOTS[i]}"
        else
            new_pre_commit_block "$is_subrepo"
        fi
        set_marker_block "$hooks_dir/pre-commit" 1
        if [[ "${TARGET_IS_WORKSPACE[i]}" -eq 1 ]]; then
            new_post_commit_block
            set_marker_block "$hooks_dir/post-commit" 1
        fi
        [[ "$quiet" -eq 1 ]] || echo "Installed workflow hooks -> ${TARGET_NAMES[i]}"
    done
}

# ---- self-test ----

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/wfhooks-selftest-XXXXXX")"
    local -a failures=()
    local -a foreign_block=(
        '# graphify-hook-start'
        'export PYTHONHASHSEED=0'
        '[ "${GRAPHIFY_SKIP_HOOK:-0}" = "1" ] && exit 0'
        'echo graphify things'
        '# graphify-hook-end'
    )
    local foreign_joined
    foreign_joined="$(printf '%s\n' "${foreign_block[@]}")"
    foreign_joined="${foreign_joined%$'\n'}"

    # Case 1: fresh install, no hook file. No InsertAfterShebang.
    local p1="$tmp/pre-commit-1"
    new_pre_commit_block 0
    set_marker_block "$p1" 0
    convert_to_lf_lines "$p1"
    local -a c1=("${LF_LINES[@]}")
    [[ "${c1[0]:-}" == '#!/bin/sh' ]] || failures+=('case1: missing shebang first line')
    local sc=0 ec=0 l
    for l in "${c1[@]}"; do
        [[ "$l" == "$START_MARKER" ]] && sc=$((sc + 1))
        [[ "$l" == "$END_MARKER" ]] && ec=$((ec + 1))
    done
    [[ $sc -eq 1 && $ec -eq 1 ]] || failures+=('case1: expected exactly one marker pair')

    # Case 2: re-install byte-idempotent.
    local h1 h2
    h1="$(sha256_of "$p1")"
    new_pre_commit_block 0
    set_marker_block "$p1" 0
    h2="$(sha256_of "$p1")"
    [[ "$h1" == "$h2" ]] || failures+=('case2: re-install not byte-identical')

    # Case 3: prepend before existing foreign block, foreign preserved.
    local p3="$tmp/post-commit-3"
    { printf '%s\n' '#!/bin/sh'; printf '%s\n' "${foreign_block[@]}"; } > "$p3"
    new_post_commit_block
    set_marker_block "$p3" 1
    local raw3 own3 for3
    raw3="$(cat "$p3")"
    case "$raw3" in *"$foreign_joined"*) ;; *) failures+=('case3: foreign block altered') ;; esac
    convert_to_lf_lines "$p3"
    own3=-1; for3=-1
    for ((i = 0; i < ${#LF_LINES[@]}; i++)); do
        [[ $own3 -lt 0 && "${LF_LINES[i]}" == "$START_MARKER" ]] && own3=$i
        [[ $for3 -lt 0 && "${LF_LINES[i]}" == '# graphify-hook-start' ]] && for3=$i
    done
    if [[ $own3 -lt 0 || $for3 -lt 0 || $own3 -gt $for3 ]]; then
        failures+=('case3: workflow block not prepended before foreign block')
    fi

    # Case 4: refresh replaces only own block, foreign untouched, order kept.
    local p4="$tmp/post-commit-4"
    {
        printf '%s\n' '#!/bin/sh' "$START_MARKER" '# OLD CONTENT' "$END_MARKER"
        printf '%s\n' "${foreign_block[@]}"
    } > "$p4"
    new_post_commit_block
    set_marker_block "$p4" 1
    local raw4
    raw4="$(cat "$p4")"
    case "$raw4" in *'# OLD CONTENT'*) failures+=('case4: stale block content survived refresh') ;; esac
    case "$raw4" in *"$foreign_joined"*) ;; *) failures+=('case4: foreign block altered') ;; esac
    convert_to_lf_lines "$p4"
    sc=0
    local own4=-1 for4=-1
    for ((i = 0; i < ${#LF_LINES[@]}; i++)); do
        [[ "${LF_LINES[i]}" == "$START_MARKER" ]] && { sc=$((sc + 1)); [[ $own4 -lt 0 ]] && own4=$i; }
        [[ $for4 -lt 0 && "${LF_LINES[i]}" == '# graphify-hook-start' ]] && for4=$i
    done
    [[ $sc -eq 1 ]] || failures+=('case4: duplicated marker pair')
    [[ $own4 -gt $for4 ]] && failures+=('case4: block order not preserved')

    # Case 5: existing file without trailing newline keeps its last line.
    local p5="$tmp/pre-commit-5"
    printf '%s\n%s' '#!/bin/sh' 'echo keep-me' > "$p5"
    new_pre_commit_block 0
    set_marker_block "$p5" 0
    convert_to_lf_lines "$p5"
    local kc=0
    for l in "${LF_LINES[@]}"; do [[ "$l" == 'echo keep-me' ]] && kc=$((kc + 1)); done
    [[ $kc -eq 1 ]] || failures+=('case5: last line lost or glued')
    local h5a h5b
    h5a="$(sha256_of "$p5")"
    new_pre_commit_block 0
    set_marker_block "$p5" 0
    h5b="$(sha256_of "$p5")"
    [[ "$h5a" == "$h5b" ]] || failures+=('case5: not idempotent after newline fixup')

    # Case 6: CRLF input fully normalized to LF (foreign region included).
    local p6="$tmp/post-commit-6"
    {
        printf '#!/bin/sh\r\n'
        for l in "${foreign_block[@]}"; do printf '%s\r\n' "$l"; done
    } > "$p6"
    new_post_commit_block
    set_marker_block "$p6" 1
    case "$(cat "$p6")" in *$'\r'*) failures+=('case6: CR bytes remain after install') ;; esac

    # Case 7: full install_workflow_hooks over a fixture repo tree.
    local ws="$tmp/case7/workspace"
    local names="engine vfs settings logging"
    mkdir -p "$ws"
    git init -q "$ws"
    { printf '%s\n' '#!/bin/sh'; printf '%s\n' "${foreign_block[@]}"; } > "$ws/.git/hooks/post-commit"
    local n
    for n in $names; do mkdir -p "$ws/$n"; git init -q "$ws/$n"; done
    get_hook_targets "$ws" "$names"
    install_workflow_hooks 1
    local ws_pre ws_post
    ws_pre="$(cat "$ws/.git/hooks/pre-commit")"
    case "$ws_pre" in *'owned by the workspace'*) failures+=('case7: workspace pre-commit has sub-repo surface guard') ;; esac
    for n in $names; do
        case "$(cat "$ws/$n/.git/hooks/pre-commit")" in
            *'owned by the workspace'*) ;;
            *) failures+=("case7: $n pre-commit missing surface guard") ;;
        esac
    done
    ws_post="$(cat "$ws/.git/hooks/post-commit")"
    case "$ws_post" in *"$foreign_joined"*) ;; *) failures+=('case7: workspace post-commit foreign block altered') ;; esac
    case "$ws_post" in *'sync-workflow-surface.sh'*) ;; *) failures+=('case7: workspace post-commit missing sync block') ;; esac
    for n in $names; do
        [[ -f "$ws/$n/.git/hooks/post-commit" ]] && failures+=("case7: $n unexpectedly got a post-commit hook")
    done

    # Case 8: foreign pre-commit block present -> workflow block prepended before it
    # (and contains no 'exit 0' that could swallow it).
    local p8="$tmp/pre-commit-8"
    { printf '%s\n' '#!/bin/sh'; printf '%s\n' "${foreign_block[@]}"; } > "$p8"
    new_pre_commit_block 1 "$tmp/ws" "$tmp/target"
    set_marker_block "$p8" 1
    convert_to_lf_lines "$p8"
    local own8=-1 for8=-1 end8=-1
    for ((i = 0; i < ${#LF_LINES[@]}; i++)); do
        [[ $own8 -lt 0 && "${LF_LINES[i]}" == "$START_MARKER" ]] && own8=$i
        [[ $for8 -lt 0 && "${LF_LINES[i]}" == '# graphify-hook-start' ]] && for8=$i
        [[ "${LF_LINES[i]}" == "$END_MARKER" ]] && end8=$i
    done
    if [[ $own8 -lt 0 || $for8 -lt 0 || $own8 -gt $for8 ]]; then
        failures+=('case8: pre-commit block not prepended before foreign block')
    fi
    for ((i = own8; i <= end8; i++)); do
        case "${LF_LINES[i]}" in
            *'exit 0'*) failures+=("case8: pre-commit block contains 'exit 0' (would swallow foreign blocks)"); break ;;
        esac
    done

    # Case 10: the pre-commit block hands the staged paths to the validator (so a
    # pre-existing finding in an untouched file cannot block an unrelated commit) and no
    # longer advertises --no-verify, which a PreToolUse hook hard-blocks for agents.
    local p10="$tmp/pre-commit-10"
    new_pre_commit_block 1 "$tmp/ws" "$tmp/target"
    set_marker_block "$p10" 0
    local raw10
    raw10="$(cat "$p10")"
    case "$raw10" in
        *'validate-workflow-docs.sh" --scope-paths "$WF_STAGED"'*) ;;
        *) failures+=('case10: validator invoked without --scope-paths (whole repo would block)') ;;
    esac
    case "$raw10" in *'--no-verify'*) failures+=('case10: block still advertises --no-verify') ;; esac
    case "$raw10" in
        *'WORKFLOW_SKIP_HOOK=1 git commit'*) ;;
        *) failures+=('case10: block lost the WORKFLOW_SKIP_HOOK escape hatch') ;;
    esac

    # Case 9: existing but empty hook file still gets a shebang.
    local p9="$tmp/pre-commit-9"
    : > "$p9"
    new_pre_commit_block 0
    set_marker_block "$p9" 1
    convert_to_lf_lines "$p9"
    [[ "${LF_LINES[0]:-}" == '#!/bin/sh' ]] || failures+=('case9: empty existing file missing shebang')

    # Case 11: the emitted surface pattern covers every synced rule file, not just the
    # first one — concept-docs.md joined $SyncFiles with MOD-11, and a rule file the
    # pattern misses is silently unguarded in both places it is embedded (the sub-repo
    # drift guard and the workspace post-commit auto-sync). Probed by running the emitted
    # regex, not by matching its text, and paired with a negative probe so widening it to
    # something that matches everything fails too.
    local p11a="$tmp/pre-commit-11" p11b="$tmp/post-commit-11" hook11 emitted11 probe11
    new_pre_commit_block 1 "$tmp/ws" "$tmp/target"
    set_marker_block "$p11a" 0
    new_post_commit_block
    set_marker_block "$p11b" 0
    for hook11 in "$p11a" "$p11b"; do
        emitted11="$(grep -m1 "WF_SURFACE_PATTERN='" "$hook11")"
        emitted11="${emitted11#*WF_SURFACE_PATTERN=\'}"
        emitted11="${emitted11%\'}"
        if [[ -z "$emitted11" ]]; then
            failures+=("case11: no WF_SURFACE_PATTERN emitted into $(basename "$hook11")")
            continue
        fi
        for probe11 in '.claude/rules/workflow-docs.md' '.claude/rules/concept-docs.md' \
                       '.claude/skills/handoff-docs.md' '.claude/skills/handoff-run/scripts/x.sh'; do
            echo "$probe11" | grep -E -q "$emitted11" \
                || failures+=("case11: surface pattern misses $probe11 in $(basename "$hook11")")
        done
        for probe11 in 'docs/ANA-11.md' 'CONCEPTS.md' '.claude/rules/minimalism.md'; do
            echo "$probe11" | grep -E -q "$emitted11" \
                && failures+=("case11: surface pattern over-matches $probe11 in $(basename "$hook11")")
        done
    done

    # Case 12: a target OUTSIDE the workspace tree installs, and its drift guard carries
    # baked absolute literals instead of the cwd-derived ".."/basename pair that only ever
    # worked for a workspace child (MOD-43 M3). Fixture mirrors the real layout: workspace
    # and target are siblings. Real `git init` here, unlike the .ps1 twin's plain dirs —
    # install_workflow_hooks resolves the hooks dir through `git rev-parse`.
    local ws12="$tmp/case12/workspace" ext12="$tmp/case12/external-template"
    mkdir -p "$ws12" "$ext12"
    git init -q "$ws12" && git init -q "$ext12"
    get_hook_targets "$ws12" "../external-template"
    if [[ ${#TARGET_NAMES[@]} -ne 2 ]]; then
        failures+=("case12: expected workspace + 1 external target, got ${#TARGET_NAMES[@]}")
    elif [[ "${TARGET_NAMES[1]}" != 'external-template' ]]; then
        failures+=("case12: label not taken from the resolved leaf: ${TARGET_NAMES[1]}")
    fi
    install_workflow_hooks 1
    local hook12 raw12 ws12abs ext12abs
    hook12="$(git -C "$ext12" rev-parse --absolute-git-dir)/hooks/pre-commit"
    raw12="$(cat "$hook12")"
    ws12abs="$(cd "$ws12" && pwd)"; ext12abs="$(cd "$ext12" && pwd)"
    grep -Fq "WF_WORKSPACE_ROOT='$ws12abs'" <<< "$raw12" \
        || failures+=('case12: workspace root not baked into the external target hook')
    grep -Fq "WF_TARGET='$ext12abs'" <<< "$raw12" \
        || failures+=('case12: target root not baked into the external target hook')
    grep -Fq 'basename "$(pwd)"' <<< "$raw12" \
        && failures+=('case12: cwd-derived drift-guard literals are still emitted')
    local h12a
    h12a="$(sha256_of "$hook12")"
    get_hook_targets "$ws12" "../external-template"
    install_workflow_hooks 1
    [[ "$(sha256_of "$hook12")" == "$h12a" ]] \
        || failures+=('case12: re-install of an external target not byte-identical')

    # Case 13: optional-missing skips, required-missing still exits 2. The second half runs
    # in a child process — exit 2 in this one would kill the self-test itself.
    local ws13="$tmp/case13/workspace"
    mkdir -p "$ws13"
    git init -q "$ws13"
    get_hook_targets "$ws13" "../does-not-exist?"
    if [[ ${#TARGET_NAMES[@]} -ne 1 ]]; then
        failures+=("case13: optional missing target was not skipped (got ${#TARGET_NAMES[@]} entries)")
    fi
    bash "${BASH_SOURCE[0]}" --workspace-root "$ws13" --targets 'does-not-exist-required' >/dev/null 2>&1
    local rc13=$?
    [[ $rc13 -eq 2 ]] || failures+=("case13: required missing target exited $rc13, expected 2")

    rm -rf "$tmp"

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: 13/13 cases PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f
    for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() {
    echo "Usage: $0 [--workspace-root <path>] [--targets <names>] [--self-test]"
}

WORKSPACE_ROOT=""
TARGETS_RAW=""
SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --workspace-root) WORKSPACE_ROOT="$2"; shift 2 ;;
        --targets) TARGETS_RAW="$2"; shift 2 ;;
        --self-test) SELF_TEST=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ $SELF_TEST -eq 1 ]]; then
    run_self_test
    exit $?
fi

if [[ -z "$WORKSPACE_ROOT" ]]; then
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$WORKSPACE_ROOT" || ! -d "$WORKSPACE_ROOT" ]]; then
    echo "WorkspaceRoot not found: $WORKSPACE_ROOT"
    exit 2
fi

# engine-template lives OUTSIDE the workspace tree (MOD-43) and is optional.
TARGETS="engine ../engine-template?"
[[ -n "$TARGETS_RAW" ]] && TARGETS="$(printf '%s' "$TARGETS_RAW" | tr ',' ' ')"

get_hook_targets "$WORKSPACE_ROOT" "$TARGETS"
install_workflow_hooks 0
exit 0
