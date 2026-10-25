#!/usr/bin/env bash
# install-workflow-hooks.sh — install/refresh this repo's workflow git hooks. Bash port of
# install-workflow-hooks.ps1 — keep the two in lockstep. The hook bodies this variant writes
# call the .sh siblings directly, so hook execution on macOS/Linux needs no pwsh.
#
# What it does:
#   core.hooksPath (TOOL-9): points it at scripts/githooks when that directory ships a
#               pre-commit. The committed hook runs the validator and then chains to whatever
#               .git/hooks holds, so the installer drops its own pre-commit block from
#               .git/hooks there: two copies would run the validator twice.
#   pre-commit  (only without committed hooks): an idempotent marker block
#               ('# workflow-hook-start/end') in .git/hooks/pre-commit. A staged
#               HANDOFF.md/DECISIONS.md/docs/decisions/**/docs/ANA-*.md or rule copy runs
#               validate-workflow-docs.sh --scope-paths "<staged>" and blocks on errors in
#               the staged files (exit 1). The validator still checks the whole repo and still
#               prints everything it finds; --scope-paths only stops findings in untouched
#               files from failing an unrelated commit.
#   post-commit: the cross-repo surface sync these hooks once ran is gone (this repo is the
#               source of truth for its workflow surface), so a marker block an older installer
#               left in .git/hooks/post-commit is removed; foreign blocks stay.
#
# Escape hatch (printed on block): WORKFLOW_SKIP_HOOK=1. Deliberately NOT --no-verify: a
# separate PreToolUse hook hard-blocks that flag for agent sessions (htui: guard G6 of the
# htui-dev mod, docs/htui-dev.md), so advertising it would be dead advice - and after the
# --scope-paths change an unrelated commit needs neither hatch.
#
# Usage:
#   bash install-workflow-hooks.sh [--repo-root <path>] [--self-test]
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
# Single source of truth for the sh-side path regex (kept identical across both installer
# variants and scripts/githooks/pre-commit{,.ps1}): the workflow docs, plus both copies of
# each rule, so the validator's rule-copy check runs when either copy is staged.
DOCS_PATTERN='^HANDOFF\.md$|^DECISIONS\.md$|^docs/decisions/.+\.md$|^docs/ANA-[^/]+\.md$|^\.claude/rules/(workflow|concept)-docs\.md$|^\.clinerules/(workflow|concept)-docs\.md$'

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

new_pre_commit_block() {
    # -> sets global BLOCK_LINES.
    # No 'exit 0' anywhere in this block: it is prepended before any foreign hook blocks,
    # and an early exit 0 here would swallow them. Only 'exit 1' (a deliberate block) may
    # terminate; every pass path falls through past the end marker.
    BLOCK_LINES=(
        "$START_MARKER"
        '# Blocks commits with structural findings in workflow docs (HANDOFF.md/DECISIONS.md/docs/decisions/'
        '# docs/ANA-*.md, the rule copies). Installed by: bash .claude/skills/handoff-run/scripts/install-workflow-hooks.sh'
        '# Escape hatch: WORKFLOW_SKIP_HOOK=1. Bypassing git hooks outright is blocked for'
        '# agent sessions, and an unrelated commit needs no hatch: only findings in the'
        '# files this commit stages block it.'
        ''
        'WF_STAGED=$(git diff --cached --name-only)'
        "WF_DOCS_PATTERN='$DOCS_PATTERN'"
        ''
        '# Fast path: the validator only spawns when a watched path is staged.'
        'if [ -n "$WF_STAGED" ] && [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ] \'
        '   && echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN"; then'
        '    # --scope-paths: whole-repo checks, but only findings in the staged files may'
        '    # block. Pre-existing findings elsewhere print as [WARNING] (pre-existing ...)'
        '    # and still fail a direct validate-workflow-docs.sh run.'
        '    bash ".claude/skills/handoff-run/scripts/validate-workflow-docs.sh" --scope-paths "$WF_STAGED"'
        '    wf_status=$?'
        '    if [ "$wf_status" -ne 0 ]; then'
        '        echo "[workflow-hook] validate-workflow-docs.sh exited $wf_status - findings above, in files this commit stages." >&2'
        '        echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
        '        exit 1'
        '    fi'
        'fi'
        "$END_MARKER"
    )
}

install_workflow_hooks() {
    # $1=root $2=quiet(0/1).
    local root="$1" quiet="${2:-0}" git_dir hooks_dir
    # Not a plain .git/hooks join: in a linked worktree or a submodule .git is a pointer
    # file; --git-common-dir resolves the hooks directory git really uses.
    git_dir="$(cd "$root" 2>/dev/null && git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"
    if [[ -z "$git_dir" ]]; then
        echo "Not a git repo: $root"
        exit 2
    fi
    hooks_dir="$git_dir/hooks"
    if [[ ! -d "$hooks_dir" ]]; then
        echo "Hooks dir not found: $hooks_dir"
        exit 2
    fi
    if committed_hooks_active "$root"; then
        # scripts/githooks/pre-commit does this job and chains to the rest; two copies
        # would run the validator twice.
        remove_marker_block "$hooks_dir/pre-commit"
    else
        # InsertAfterShebang: a foreign block with an early exit 0 must never sit above
        # ours and swallow it.
        new_pre_commit_block
        set_marker_block "$hooks_dir/pre-commit" 1
    fi
    # The surface-sync block older installers wrote here, if this clone still has it.
    remove_marker_block "$hooks_dir/post-commit"
    [[ "$quiet" -eq 1 ]] || echo "Installed workflow hooks -> $root"
}

HOOKS_PATH_VALUE='scripts/githooks'

remove_marker_block() {
    # $1=hook_path. Drops our '# workflow-hook-start/end' block (markers included) and leaves
    # every other line, foreign blocks included, as it was. No block or no file: nothing done.
    local hook_path="$1" i start=-1 end=-1
    [[ -s "$hook_path" ]] || return 0
    convert_to_lf_lines "$hook_path"
    local -a lines=("${LF_LINES[@]}")
    for ((i = 0; i < ${#lines[@]}; i++)); do
        if [[ "$(trim "${lines[i]}")" == "$START_MARKER" ]]; then start=$i; break; fi
    done
    [[ $start -ge 0 ]] || return 0
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
    printf '%s\n' "${pre[@]}" "${post[@]}" > "$hook_path"
    chmod +x "$hook_path"
}

set_hooks_path() {
    # $1=root. Points core.hooksPath at the committed hooks (TOOL-9). Relative, so it resolves
    # in the main checkout and in every linked worktree. A repo without the committed hooks is
    # left alone: a hooksPath to nowhere would switch off every hook it has.
    local root="$1"
    [[ -x "$root/$HOOKS_PATH_VALUE/pre-commit" ]] || return 0
    [[ "$(git -C "$root" config --local --get core.hooksPath 2>/dev/null)" == "$HOOKS_PATH_VALUE" ]] \
        || {
            git -C "$root" config --local core.hooksPath "$HOOKS_PATH_VALUE"
            warn_unchained_hooks "$root"
        }
}

warn_unchained_hooks() {
    # core.hooksPath replaces .git/hooks, and the committed hooks chain only pre-commit and
    # post-commit. Any other hook the clone keeps there would silently stop running.
    local dir="$1/.git/hooks" f n others=()
    [[ -d "$dir" ]] || return 0
    for f in "$dir"/*; do
        [[ -f "$f" ]] || continue
        n="$(basename "$f")"
        case "$n" in *.sample|pre-commit|post-commit) continue ;; esac
        others+=("$n")
    done
    if [[ ${#others[@]} -gt 0 ]]; then
        echo "warning: core.hooksPath now bypasses .git/hooks/{${others[*]}} - only pre-commit and post-commit are chained"
    fi
}

committed_hooks_active() {
    # $1=root. True when the committed pre-commit hook is the one git runs here.
    [[ -x "$1/$HOOKS_PATH_VALUE/pre-commit" \
        && "$(git -C "$1" config --local --get core.hooksPath 2>/dev/null)" == "$HOOKS_PATH_VALUE" ]]
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
    local i l

    # Case 1: fresh install, no hook file. No InsertAfterShebang.
    local p1="$tmp/pre-commit-1"
    new_pre_commit_block
    set_marker_block "$p1" 0
    convert_to_lf_lines "$p1"
    local -a c1=("${LF_LINES[@]}")
    [[ "${c1[0]:-}" == '#!/bin/sh' ]] || failures+=('case1: missing shebang first line')
    local sc=0 ec=0
    for l in "${c1[@]}"; do
        [[ "$l" == "$START_MARKER" ]] && sc=$((sc + 1))
        [[ "$l" == "$END_MARKER" ]] && ec=$((ec + 1))
    done
    [[ $sc -eq 1 && $ec -eq 1 ]] || failures+=('case1: expected exactly one marker pair')

    # Case 2: re-install byte-idempotent.
    local h1 h2
    h1="$(sha256_of "$p1")"
    new_pre_commit_block
    set_marker_block "$p1" 0
    h2="$(sha256_of "$p1")"
    [[ "$h1" == "$h2" ]] || failures+=('case2: re-install not byte-identical')

    # Case 3: prepend before existing foreign block, foreign preserved.
    local p3="$tmp/pre-commit-3" raw3 own3=-1 for3=-1
    { printf '%s\n' '#!/bin/sh'; printf '%s\n' "${foreign_block[@]}"; } > "$p3"
    new_pre_commit_block
    set_marker_block "$p3" 1
    raw3="$(cat "$p3")"
    case "$raw3" in *"$foreign_joined"*) ;; *) failures+=('case3: foreign block altered') ;; esac
    convert_to_lf_lines "$p3"
    for ((i = 0; i < ${#LF_LINES[@]}; i++)); do
        [[ $own3 -lt 0 && "${LF_LINES[i]}" == "$START_MARKER" ]] && own3=$i
        [[ $for3 -lt 0 && "${LF_LINES[i]}" == '# graphify-hook-start' ]] && for3=$i
    done
    if [[ $own3 -lt 0 || $for3 -lt 0 || $own3 -gt $for3 ]]; then
        failures+=('case3: workflow block not prepended before foreign block')
    fi

    # Case 4: refresh replaces only own block, foreign untouched, order kept.
    local p4="$tmp/pre-commit-4" raw4 own4=-1 for4=-1
    {
        printf '%s\n' '#!/bin/sh' "$START_MARKER" '# OLD CONTENT' "$END_MARKER"
        printf '%s\n' "${foreign_block[@]}"
    } > "$p4"
    new_pre_commit_block
    set_marker_block "$p4" 1
    raw4="$(cat "$p4")"
    case "$raw4" in *'# OLD CONTENT'*) failures+=('case4: stale block content survived refresh') ;; esac
    case "$raw4" in *"$foreign_joined"*) ;; *) failures+=('case4: foreign block altered') ;; esac
    convert_to_lf_lines "$p4"
    sc=0
    for ((i = 0; i < ${#LF_LINES[@]}; i++)); do
        [[ "${LF_LINES[i]}" == "$START_MARKER" ]] && { sc=$((sc + 1)); [[ $own4 -lt 0 ]] && own4=$i; }
        [[ $for4 -lt 0 && "${LF_LINES[i]}" == '# graphify-hook-start' ]] && for4=$i
    done
    [[ $sc -eq 1 ]] || failures+=('case4: duplicated marker pair')
    [[ $own4 -gt $for4 ]] && failures+=('case4: block order not preserved')

    # Case 5: existing file without trailing newline keeps its last line.
    local p5="$tmp/pre-commit-5" kc=0 h5a h5b
    printf '%s\n%s' '#!/bin/sh' 'echo keep-me' > "$p5"
    new_pre_commit_block
    set_marker_block "$p5" 0
    convert_to_lf_lines "$p5"
    for l in "${LF_LINES[@]}"; do [[ "$l" == 'echo keep-me' ]] && kc=$((kc + 1)); done
    [[ $kc -eq 1 ]] || failures+=('case5: last line lost or glued')
    h5a="$(sha256_of "$p5")"
    new_pre_commit_block
    set_marker_block "$p5" 0
    h5b="$(sha256_of "$p5")"
    [[ "$h5a" == "$h5b" ]] || failures+=('case5: not idempotent after newline fixup')

    # Case 6: CRLF input fully normalized to LF (foreign region included).
    local p6="$tmp/pre-commit-6"
    {
        printf '#!/bin/sh\r\n'
        for l in "${foreign_block[@]}"; do printf '%s\r\n' "$l"; done
    } > "$p6"
    new_pre_commit_block
    set_marker_block "$p6" 1
    case "$(cat "$p6")" in *$'\r'*) failures+=('case6: CR bytes remain after install') ;; esac

    # Case 7: full install over a repo without committed hooks: the pre-commit block lands,
    # the post-commit surface-sync block an older installer wrote goes, a foreign block stays.
    local ws7="$tmp/case7/repo" post7
    mkdir -p "$ws7"
    git init -q "$ws7"
    {
        printf '%s\n' '#!/bin/sh' "$START_MARKER" 'bash sync-workflow-surface.sh' "$END_MARKER"
        printf '%s\n' "${foreign_block[@]}"
    } > "$ws7/.git/hooks/post-commit"
    install_workflow_hooks "$ws7" 1
    case "$(cat "$ws7/.git/hooks/pre-commit")" in
        *'validate-workflow-docs.sh" --scope-paths "$WF_STAGED"'*) ;;
        *) failures+=('case7: pre-commit block missing after install') ;;
    esac
    post7="$(cat "$ws7/.git/hooks/post-commit")"
    case "$post7" in *sync-workflow-surface*) failures+=('case7: old post-commit sync block survived') ;; esac
    case "$post7" in *"$foreign_joined"*) ;; *) failures+=('case7: foreign post-commit block lost') ;; esac

    # Case 8: the block contains no 'exit 0' that could swallow a foreign block below it.
    local p8="$tmp/pre-commit-8"
    new_pre_commit_block
    set_marker_block "$p8" 0
    grep -q 'exit 0' "$p8" && failures+=("case8: pre-commit block contains 'exit 0' (would swallow foreign blocks)")

    # Case 9: existing but empty hook file still gets a shebang.
    local p9="$tmp/pre-commit-9"
    : > "$p9"
    new_pre_commit_block
    set_marker_block "$p9" 1
    convert_to_lf_lines "$p9"
    [[ "${LF_LINES[0]:-}" == '#!/bin/sh' ]] || failures+=('case9: empty existing file missing shebang')

    # Case 10: the block hands the staged paths to the validator (so a pre-existing finding in
    # an untouched file cannot block an unrelated commit) and advertises WORKFLOW_SKIP_HOOK,
    # never --no-verify, which a PreToolUse hook hard-blocks for agents.
    local raw10
    raw10="$(cat "$p8")"
    case "$raw10" in
        *'validate-workflow-docs.sh" --scope-paths "$WF_STAGED"'*) ;;
        *) failures+=('case10: validator invoked without --scope-paths (whole repo would block)') ;;
    esac
    case "$raw10" in *'--no-verify'*) failures+=('case10: block still advertises --no-verify') ;; esac
    case "$raw10" in
        *'WORKFLOW_SKIP_HOOK=1 git commit'*) ;;
        *) failures+=('case10: block lost the WORKFLOW_SKIP_HOOK escape hatch') ;;
    esac

    # Case 11: the watched-path pattern covers the workflow docs and both copies of both rules
    # (so a staged copy reaches the rule-copy check) and nothing else, probed by running the
    # emitted regex. The committed hook must carry the identical pattern.
    local emitted11 probe11 committed11
    emitted11="$(grep -m1 "WF_DOCS_PATTERN='" "$p8")"
    emitted11="${emitted11#*WF_DOCS_PATTERN=\'}"
    emitted11="${emitted11%\'}"
    for probe11 in 'HANDOFF.md' 'DECISIONS.md' 'docs/decisions/mod/mod-1.md' 'docs/ANA-11.md' \
                   '.claude/rules/workflow-docs.md' '.claude/rules/concept-docs.md' \
                   '.clinerules/workflow-docs.md' '.clinerules/concept-docs.md'; do
        echo "$probe11" | grep -E -q "$emitted11" || failures+=("case11: pattern misses $probe11")
    done
    for probe11 in 'CONCEPTS.md' 'README.md' '.claude/rules/minimalism.md' '.clinerules/gortex.md' \
                   'docs/sub/ANA-1.md' '.claude/skills/handoff-run/SKILL.md'; do
        echo "$probe11" | grep -E -q "$emitted11" && failures+=("case11: pattern over-matches $probe11")
    done
    committed11="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)/scripts/githooks/pre-commit"
    if [[ -f "$committed11" ]] && ! grep -Fq "WF_DOCS_PATTERN='$DOCS_PATTERN'" "$committed11"; then
        failures+=('case11: scripts/githooks/pre-commit watches a different pattern')
    fi

    # Case 12-13: committed hooks (TOOL-9). Fixture: a repo carrying real copies of
    # scripts/githooks, one foreign pre-commit block in .git/hooks, and our old block.
    local src12 ws12 own12 raw12 h12a
    src12="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)/scripts/githooks"
    ws12="$tmp/case12/repo"
    mkdir -p "$ws12/scripts"
    git init -q "$ws12"
    cp -r "$src12" "$ws12/scripts/githooks"
    own12="$ws12/.git/hooks/pre-commit"
    { printf '%s\n' '#!/bin/sh'; printf '%s\n' "${foreign_block[@]}"; } > "$own12"
    new_pre_commit_block
    set_marker_block "$own12" 1
    set_hooks_path "$ws12"
    [[ "$(git -C "$ws12" config --local --get core.hooksPath)" == 'scripts/githooks' ]] \
        || failures+=('case12: core.hooksPath not set')
    install_workflow_hooks "$ws12" 1
    raw12="$(cat "$own12")"
    case "$raw12" in *"$START_MARKER"*) failures+=('case13: own pre-commit block survived next to the committed hook') ;; esac
    case "$raw12" in *"$foreign_joined"*) ;; *) failures+=('case13: foreign pre-commit block lost') ;; esac
    h12a="$(sha256_of "$own12")"
    set_hooks_path "$ws12"
    install_workflow_hooks "$ws12" 1
    [[ "$(sha256_of "$own12")" == "$h12a" ]] || failures+=('case12: second install changed the hook')

    # Case 14: the committed hook blocks a staged workflow doc that fails the validator, lets an
    # unrelated commit through, and still reaches the clone's own hook (the foreign block here
    # touches a marker file).
    local ws14="$tmp/case14/repo" marker14
    mkdir -p "$ws14/scripts" "$ws14/.claude/skills/handoff-run/scripts"
    git init -q "$ws14"
    git -C "$ws14" config user.email t@t && git -C "$ws14" config user.name t
    cp -r "$src12" "$ws14/scripts/githooks"
    cp "$(dirname "${BASH_SOURCE[0]}")"/*.sh "$ws14/.claude/skills/handoff-run/scripts/"
    marker14="$ws14/foreign-ran"
    printf '%s\n' '#!/bin/sh' "touch '$marker14'" > "$ws14/.git/hooks/pre-commit"
    chmod +x "$ws14/.git/hooks/pre-commit"
    git -C "$ws14" config core.hooksPath scripts/githooks
    echo x > "$ws14/unrelated.txt"
    git -C "$ws14" add unrelated.txt
    git -C "$ws14" commit -q -m unrelated >/dev/null 2>&1 \
        || failures+=('case14: an unrelated commit was blocked')
    [[ -e "$marker14" ]] || failures+=('case14: the hook in .git/hooks was not chained')
    # A HANDOFF.md must exist or the validator reports that instead (outside the staged files).
    printf '# H\n\n## Open\n\n## Summary\n\n| Area | Open |\n|---|---|\n' > "$ws14/HANDOFF.md"
    printf '# D\n- **[MOD-1](docs/decisions/mod/mod-1.md)** - missing write-up (done, 2026-01-01)\n' > "$ws14/DECISIONS.md"
    git -C "$ws14" add DECISIONS.md HANDOFF.md
    git -C "$ws14" commit -q -m docs >/dev/null 2>&1 \
        && failures+=('case14: a staged DECISIONS.md with a dangling index line was committed')
    WORKFLOW_SKIP_HOOK=1 git -C "$ws14" commit -q -m docs >/dev/null 2>&1 \
        || failures+=('case14: WORKFLOW_SKIP_HOOK=1 did not let the commit through')

    # Case 15: the committed hook blocks a commit that stages one rule copy out of step with its
    # twin, and lets it through once both copies match.
    mkdir -p "$ws14/.claude/rules" "$ws14/.clinerules"
    printf '# rule v1\n' > "$ws14/.claude/rules/workflow-docs.md"
    printf '# rule v1\n' > "$ws14/.clinerules/workflow-docs.md"
    git -C "$ws14" add .claude/rules .clinerules
    git -C "$ws14" commit -q -m 'rule copies' >/dev/null 2>&1 \
        || failures+=('case15: matching rule copies were blocked')
    printf '# rule v2\n' > "$ws14/.claude/rules/workflow-docs.md"
    git -C "$ws14" add .claude/rules/workflow-docs.md
    git -C "$ws14" commit -q -m 'one copy only' >/dev/null 2>&1 \
        && failures+=('case15: a rule copy out of step with its twin was committed')
    printf '# rule v2\n' > "$ws14/.clinerules/workflow-docs.md"
    git -C "$ws14" add .clinerules/workflow-docs.md
    git -C "$ws14" commit -q -m 'both copies' >/dev/null 2>&1 \
        || failures+=('case15: both rule copies updated together were blocked')

    rm -rf "$tmp"

    if [[ ${#failures[@]} -eq 0 ]]; then
        echo 'Self-test: 15/15 cases PASS.'
        return 0
    fi
    echo "Self-test FAIL (${#failures[@]}):"
    local f
    for f in "${failures[@]}"; do echo "  - $f"; done
    return 1
}

# ---- main ----

usage() {
    echo "Usage: $0 [--repo-root <path>] [--self-test]"
}

REPO_ROOT=""
SELF_TEST=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo-root) REPO_ROOT="${2:-}"; shift 2 || { usage >&2; exit 2; } ;;
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
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." 2>/dev/null && pwd)"
fi
if [[ -z "$REPO_ROOT" || ! -d "$REPO_ROOT" ]]; then
    echo "RepoRoot not found: $REPO_ROOT"
    exit 2
fi
REPO_ROOT="$(cd "$REPO_ROOT" && pwd)"

set_hooks_path "$REPO_ROOT"
install_workflow_hooks "$REPO_ROOT" 0
exit 0
