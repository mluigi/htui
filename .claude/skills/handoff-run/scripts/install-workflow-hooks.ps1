# install-workflow-hooks.ps1 — install/refresh this repo's workflow git hooks. The bash twin
# is install-workflow-hooks.sh — keep the two in lockstep. The hook bodies this variant writes
# call pwsh -File ...ps1, which is what a Windows box needs (bash forks stall there).
#
# What it does:
#   core.hooksPath (TOOL-9): points it at scripts/githooks when that directory ships a
#               pre-commit. The committed hook runs the validator and then chains to whatever
#               .git/hooks holds, so the installer drops its own pre-commit block from
#               .git/hooks there: two copies would run the validator twice.
#   pre-commit  (only without committed hooks): an idempotent marker block
#               ('# workflow-hook-start/end') in .git/hooks/pre-commit. A staged
#               HANDOFF.md/DECISIONS.md/docs/decisions/**/docs/ANA-*.md or rule copy runs
#               validate-workflow-docs.ps1 -ScopePaths "<staged>" and blocks on errors in the
#               staged files (exit 1). The validator still checks the whole repo and still
#               prints everything it finds; -ScopePaths only stops findings in untouched files
#               from failing an unrelated commit.
#   post-commit: the cross-repo surface sync these hooks once ran is gone (this repo is the
#               source of truth for its workflow surface), so a marker block an older installer
#               left in .git/hooks/post-commit is removed; foreign blocks stay.
#
# Escape hatch (printed on block): WORKFLOW_SKIP_HOOK=1. Deliberately NOT --no-verify: a
# separate PreToolUse hook hard-blocks that flag for agent sessions (htui: guard G6 of the
# htui-dev mod, docs/htui-dev.md), so advertising it would be dead advice - and after the
# -ScopePaths change an unrelated commit needs neither hatch.
#
# Usage:
#   pwsh install-workflow-hooks.ps1 [-RepoRoot <path>] [-SelfTest]
#
# Exit codes: 0 installed, 1 self-test failure, 2 usage/missing paths.

[CmdletBinding()]
param(
    [string]$RepoRoot,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$StartMarker = '# workflow-hook-start'
$EndMarker = '# workflow-hook-end'
# Single source of truth for the sh-side path regex (kept identical across both installer
# variants and scripts/githooks/pre-commit{,.ps1}): the workflow docs, plus both copies of
# each rule, so the validator's rule-copy check runs when either copy is staged.
$DocsPattern = '^HANDOFF\.md$|^DECISIONS\.md$|^docs/decisions/.+\.md$|^docs/ANA-[^/]+\.md$|^\.claude/rules/(workflow|concept)-docs\.md$|^\.clinerules/(workflow|concept)-docs\.md$'

function ConvertTo-LfLines {
    # Whole-file CRLF->LF normalization (git sh hooks choke on stray \r), split to lines.
    # Strips exactly one trailing empty element only when the text really ended in a
    # newline — a file without trailing newline keeps its last line intact.
    param([string]$Text)
    $t = $Text -replace "`r`n", "`n" -replace "`r", "`n"
    if ($t.EndsWith("`n")) { $t = $t.Substring(0, $t.Length - 1) }
    return @($t -split "`n")
}

function Find-MarkerIndex {
    # Exact trimmed-line match — never substring, so a comment merely mentioning the
    # marker text can't be mistaken for it.
    param([string[]]$Lines, [string]$Marker, [int]$From = 0)
    for ($i = $From; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i].Trim() -eq $Marker) { return $i }
    }
    return -1
}

function Set-MarkerBlock {
    # Idempotent insert/replace of the workflow block. Foreign blocks (e.g. graphify's)
    # are untouched by construction: we only splice between our own exact markers, or
    # insert where no marker exists.
    param(
        [string]$HookPath,
        [string[]]$BlockLines,
        [switch]$InsertAfterShebang
    )
    if (Test-Path -LiteralPath $HookPath) {
        $lines = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($HookPath)))
        # Empty/whitespace-only existing file: treat as fresh so it still gets a shebang.
        if (-not ($lines | Where-Object { $_.Trim() -ne '' })) { $lines = @('#!/bin/sh') }
    } else {
        $lines = @('#!/bin/sh')
    }

    $start = Find-MarkerIndex -Lines $lines -Marker $StartMarker
    if ($start -ge 0) {
        $end = Find-MarkerIndex -Lines $lines -Marker $EndMarker -From ($start + 1)
        if ($end -lt 0) {
            throw "corrupted workflow-hook block in ${HookPath}: start marker without matching end - fix by hand"
        }
        $pre = if ($start -gt 0) { $lines[0..($start - 1)] } else { @() }
        $post = if ($end -lt $lines.Count - 1) { $lines[($end + 1)..($lines.Count - 1)] } else { @() }
        $new = @($pre) + $BlockLines + @($post)
    } elseif ($InsertAfterShebang -and $lines.Count -gt 0 -and $lines[0] -like '#!*') {
        # Prepend right after the shebang: foreign blocks below may exit 0 early and
        # must not swallow this block.
        $rest = if ($lines.Count -gt 1) { $lines[1..($lines.Count - 1)] } else { @() }
        $new = @($lines[0]) + @('') + $BlockLines + @($rest)
    } else {
        $new = @($lines) + @('') + $BlockLines
    }

    [System.IO.File]::WriteAllText(
        $HookPath,
        (($new -join "`n") + "`n"),
        (New-Object System.Text.UTF8Encoding($false)))
}

$HooksPathValue = 'scripts/githooks'

function Remove-MarkerBlock {
    # Drops our '# workflow-hook-start/end' block (markers included); every other line,
    # foreign blocks included, stays. No block or no file: nothing done.
    param([string]$HookPath)
    if (-not (Test-Path -LiteralPath $HookPath)) { return }
    $lines = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($HookPath)))
    $start = Find-MarkerIndex -Lines $lines -Marker $StartMarker
    if ($start -lt 0) { return }
    $end = Find-MarkerIndex -Lines $lines -Marker $EndMarker -From ($start + 1)
    if ($end -lt 0) {
        throw "corrupted workflow-hook block in ${HookPath}: start marker without matching end - fix by hand"
    }
    $pre = if ($start -gt 0) { $lines[0..($start - 1)] } else { @() }
    $post = if ($end -lt $lines.Count - 1) { $lines[($end + 1)..($lines.Count - 1)] } else { @() }
    $new = @($pre) + @($post)
    [System.IO.File]::WriteAllText(
        $HookPath,
        (($new -join "`n") + "`n"),
        (New-Object System.Text.UTF8Encoding($false)))
}

function Get-HooksPath {
    param([string]$Root)
    $v = & git -C $Root config --local --get core.hooksPath 2>$null
    if ($LASTEXITCODE -ne 0) { return '' }
    return "$v".Trim()
}

function Set-HooksPath {
    # Points core.hooksPath at the committed hooks (TOOL-9). Relative, so it resolves in the
    # main checkout and in every linked worktree. A repo without the committed hooks is left
    # alone: a hooksPath to nowhere would switch off every hook it has.
    param([string]$Root)
    if (-not (Test-Path -LiteralPath (Join-Path $Root $HooksPathValue 'pre-commit'))) { return }
    if ((Get-HooksPath -Root $Root) -ne $HooksPathValue) {
        & git -C $Root config --local core.hooksPath $HooksPathValue
        Write-UnchainedHookWarning -Root $Root
    }
}

function Write-UnchainedHookWarning {
    # core.hooksPath replaces .git/hooks, and the committed hooks chain only pre-commit and
    # post-commit. Any other hook the clone keeps there would silently stop running.
    param([string]$Root)
    $dir = Join-Path $Root '.git' 'hooks'
    if (-not (Test-Path -LiteralPath $dir)) { return }
    $others = @(Get-ChildItem -LiteralPath $dir -File | Where-Object {
            $_.Name -notlike '*.sample' -and $_.Name -notin @('pre-commit', 'post-commit') } | ForEach-Object { $_.Name })
    if ($others.Count -gt 0) {
        Write-Host "warning: core.hooksPath now bypasses .git/hooks/$($others -join ', .git/hooks/') - only pre-commit and post-commit are chained"
    }
}

function Test-CommittedHooksActive {
    param([string]$Root)
    return ((Test-Path -LiteralPath (Join-Path $Root $HooksPathValue 'pre-commit')) -and
        ((Get-HooksPath -Root $Root) -eq $HooksPathValue))
}


function New-PreCommitBlock {
    # No 'exit 0' anywhere in this block: it is prepended before any foreign hook blocks,
    # and an early exit 0 here would swallow them. Only 'exit 1' (a deliberate block) may
    # terminate; every pass path falls through past the end marker.
    return @(
        $StartMarker,
        '# Blocks commits with structural findings in workflow docs (HANDOFF.md/DECISIONS.md/docs/decisions/'
        '# docs/ANA-*.md, the rule copies). Installed by: pwsh .claude/skills/handoff-run/scripts/install-workflow-hooks.ps1'
        '# Escape hatch: WORKFLOW_SKIP_HOOK=1. Bypassing git hooks outright is blocked for'
        '# agent sessions, and an unrelated commit needs no hatch: only findings in the'
        '# files this commit stages block it.'
        ''
        'WF_STAGED=$(git diff --cached --name-only)'
        "WF_DOCS_PATTERN='$DocsPattern'"
        ''
        '# Fast path: pwsh only spawns when a watched path is staged.'
        'if [ -n "$WF_STAGED" ] && [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ] \'
        '   && echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN"; then'
        '    # -ScopePaths: whole-repo checks, but only findings in the staged files may'
        '    # block. Pre-existing findings elsewhere print as [WARNING] (pre-existing ...)'
        '    # and still fail a direct validate-workflow-docs.ps1 run.'
        '    pwsh -NoProfile -File ".claude/skills/handoff-run/scripts/validate-workflow-docs.ps1" -ScopePaths "$WF_STAGED"'
        '    wf_status=$?'
        '    if [ "$wf_status" -ne 0 ]; then'
        '        echo "[workflow-hook] validate-workflow-docs.ps1 exited $wf_status - findings above, in files this commit stages." >&2'
        '        echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
        '        exit 1'
        '    fi'
        'fi'
        $EndMarker
    )
}

function Get-HooksDir {
    # The hooks directory git really uses: in a linked worktree or a submodule .git is a
    # pointer file, which --git-common-dir resolves. '' when Root is not a git repo.
    param([string]$Root)
    $d = & git -C $Root rev-parse --path-format=absolute --git-common-dir 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $d) { return '' }
    return (Join-Path "$d".Trim() 'hooks')
}

function Install-WorkflowHooks {
    param([string]$Root, [switch]$Quiet)
    $gitDir = Get-HooksDir -Root $Root
    if (-not $gitDir) {
        Write-Host "Not a git repo: $Root"
        exit 2
    }
    if (-not (Test-Path -LiteralPath $gitDir)) {
        Write-Host "Hooks dir not found: $gitDir"
        exit 2
    }
    if (Test-CommittedHooksActive -Root $Root) {
        # scripts/githooks/pre-commit does this job and chains to the rest; two copies
        # would run the validator twice.
        Remove-MarkerBlock -HookPath (Join-Path $gitDir 'pre-commit')
    } else {
        # -InsertAfterShebang: a foreign block with an early exit 0 must never sit above ours
        # and swallow it.
        Set-MarkerBlock -HookPath (Join-Path $gitDir 'pre-commit') -BlockLines (New-PreCommitBlock) -InsertAfterShebang
    }
    # The surface-sync block older installers wrote here, if this clone still has it.
    Remove-MarkerBlock -HookPath (Join-Path $gitDir 'post-commit')
    if (-not $Quiet) { Write-Host "Installed workflow hooks -> $Root" }
}

function Invoke-SelfTest {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("wfhooks-selftest-" + [guid]::NewGuid().ToString('N'))
    $failures = @()
    $foreignBlock = @(
        '# graphify-hook-start'
        'export PYTHONHASHSEED=0'
        '[ "${GRAPHIFY_SKIP_HOOK:-0}" = "1" ] && exit 0'
        'echo graphify things'
        '# graphify-hook-end'
    )
    try {
        New-Item -ItemType Directory -Path $tmp -Force | Out-Null

        # Case 1: fresh install, no hook file.
        $p1 = Join-Path $tmp 'pre-commit-1'
        Set-MarkerBlock -HookPath $p1 -BlockLines (New-PreCommitBlock)
        $c1 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($p1)))
        if ($c1[0] -ne '#!/bin/sh') { $failures += 'case1: missing shebang first line' }
        if (@($c1 | Where-Object { $_ -eq $StartMarker }).Count -ne 1 -or
            @($c1 | Where-Object { $_ -eq $EndMarker }).Count -ne 1) {
            $failures += 'case1: expected exactly one marker pair'
        }

        # Case 2: re-install byte-idempotent.
        $h1 = (Get-FileHash $p1 -Algorithm SHA256).Hash
        Set-MarkerBlock -HookPath $p1 -BlockLines (New-PreCommitBlock)
        if ($h1 -ne (Get-FileHash $p1 -Algorithm SHA256).Hash) { $failures += 'case2: re-install not byte-identical' }

        # Case 3: prepend before existing foreign block, foreign preserved.
        $p3 = Join-Path $tmp 'pre-commit-3'
        [System.IO.File]::WriteAllText($p3, ((@('#!/bin/sh') + $foreignBlock -join "`n") + "`n"))
        Set-MarkerBlock -HookPath $p3 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        $raw3 = [System.IO.File]::ReadAllText($p3)
        if (-not $raw3.Contains(($foreignBlock -join "`n"))) { $failures += 'case3: foreign block altered' }
        $c3 = @(ConvertTo-LfLines $raw3)
        $ownIdx = Find-MarkerIndex -Lines $c3 -Marker $StartMarker
        $forIdx = Find-MarkerIndex -Lines $c3 -Marker '# graphify-hook-start'
        if ($ownIdx -lt 0 -or $forIdx -lt 0 -or $ownIdx -gt $forIdx) {
            $failures += 'case3: workflow block not prepended before foreign block'
        }

        # Case 4: refresh replaces only own block, foreign untouched, order kept.
        $stale = @($StartMarker, '# OLD CONTENT', $EndMarker)
        $p4 = Join-Path $tmp 'pre-commit-4'
        [System.IO.File]::WriteAllText($p4, ((@('#!/bin/sh') + $stale + $foreignBlock -join "`n") + "`n"))
        Set-MarkerBlock -HookPath $p4 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        $raw4 = [System.IO.File]::ReadAllText($p4)
        if ($raw4.Contains('# OLD CONTENT')) { $failures += 'case4: stale block content survived refresh' }
        if (-not $raw4.Contains(($foreignBlock -join "`n"))) { $failures += 'case4: foreign block altered' }
        $c4 = @(ConvertTo-LfLines $raw4)
        if (@($c4 | Where-Object { $_ -eq $StartMarker }).Count -ne 1) { $failures += 'case4: duplicated marker pair' }
        if ((Find-MarkerIndex -Lines $c4 -Marker $StartMarker) -gt (Find-MarkerIndex -Lines $c4 -Marker '# graphify-hook-start')) {
            $failures += 'case4: block order not preserved'
        }

        # Case 5: existing file without trailing newline keeps its last line.
        $p5 = Join-Path $tmp 'pre-commit-5'
        [System.IO.File]::WriteAllText($p5, "#!/bin/sh`necho keep-me")   # no trailing \n
        Set-MarkerBlock -HookPath $p5 -BlockLines (New-PreCommitBlock)
        $c5 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($p5)))
        if (@($c5 | Where-Object { $_ -eq 'echo keep-me' }).Count -ne 1) { $failures += 'case5: last line lost or glued' }
        $h5a = (Get-FileHash $p5 -Algorithm SHA256).Hash
        Set-MarkerBlock -HookPath $p5 -BlockLines (New-PreCommitBlock)
        if ($h5a -ne (Get-FileHash $p5 -Algorithm SHA256).Hash) { $failures += 'case5: not idempotent after newline fixup' }

        # Case 6: CRLF input fully normalized to LF (foreign region included).
        $p6 = Join-Path $tmp 'pre-commit-6'
        [System.IO.File]::WriteAllText($p6, ((@('#!/bin/sh') + $foreignBlock -join "`r`n") + "`r`n"))
        Set-MarkerBlock -HookPath $p6 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        if (([System.IO.File]::ReadAllText($p6)).Contains("`r")) { $failures += 'case6: CR bytes remain after install' }

        # Case 7: full install over a repo without committed hooks: the pre-commit block lands,
        # the post-commit surface-sync block an older installer wrote goes, a foreign block stays.
        $ws7 = Join-Path $tmp 'case7' 'repo'
        New-Item -ItemType Directory -Force $ws7 | Out-Null
        & git init -q $ws7
        [System.IO.File]::WriteAllText((Join-Path $ws7 '.git' 'hooks' 'post-commit'),
            ((@('#!/bin/sh', $StartMarker, 'pwsh sync-workflow-surface.ps1', $EndMarker) + $foreignBlock -join "`n") + "`n"))
        Install-WorkflowHooks -Root $ws7 -Quiet
        $pre7 = [System.IO.File]::ReadAllText((Join-Path $ws7 '.git' 'hooks' 'pre-commit'))
        if ($pre7 -notmatch [regex]::Escape('validate-workflow-docs.ps1" -ScopePaths "$WF_STAGED"')) {
            $failures += 'case7: pre-commit block missing after install'
        }
        $post7 = [System.IO.File]::ReadAllText((Join-Path $ws7 '.git' 'hooks' 'post-commit'))
        if ($post7.Contains('sync-workflow-surface')) { $failures += 'case7: old post-commit sync block survived' }
        if (-not $post7.Contains(($foreignBlock -join "`n"))) { $failures += 'case7: foreign post-commit block lost' }

        # Case 8: the block contains no 'exit 0' that could swallow a foreign block below it.
        $p8 = Join-Path $tmp 'pre-commit-8'
        Set-MarkerBlock -HookPath $p8 -BlockLines (New-PreCommitBlock)
        $raw8 = [System.IO.File]::ReadAllText($p8)
        if ($raw8 -match '\bexit 0\b') { $failures += "case8: pre-commit block contains 'exit 0' (would swallow foreign blocks)" }

        # Case 9: existing but empty hook file still gets a shebang.
        $p9 = Join-Path $tmp 'pre-commit-9'
        [System.IO.File]::WriteAllText($p9, '')
        Set-MarkerBlock -HookPath $p9 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        $c9 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($p9)))
        if ($c9[0] -ne '#!/bin/sh') { $failures += 'case9: empty existing file missing shebang' }

        # Case 10: the block hands the staged paths to the validator and advertises
        # WORKFLOW_SKIP_HOOK, never --no-verify, which a PreToolUse hook hard-blocks for agents.
        if ($raw8 -notmatch [regex]::Escape('validate-workflow-docs.ps1" -ScopePaths "$WF_STAGED"')) {
            $failures += 'case10: validator invoked without -ScopePaths (whole repo would block)'
        }
        if ($raw8 -match '--no-verify') { $failures += 'case10: block still advertises --no-verify' }
        if ($raw8 -notmatch [regex]::Escape('WORKFLOW_SKIP_HOOK=1 git commit')) {
            $failures += 'case10: block lost the WORKFLOW_SKIP_HOOK escape hatch'
        }

        # Case 11: the watched-path pattern covers the workflow docs and both copies of both rules
        # and nothing else, probed by running the emitted regex (-cmatch: the sh half's grep -E is
        # case-sensitive). The committed hooks must carry the identical pattern.
        $line11 = @(ConvertTo-LfLines $raw8) | Where-Object { $_ -match "WF_DOCS_PATTERN='" } | Select-Object -First 1
        $emitted11 = ($line11 -replace "^.*WF_DOCS_PATTERN='", '') -replace "'$", ''
        foreach ($probe11 in @('HANDOFF.md', 'DECISIONS.md', 'docs/decisions/mod/mod-1.md', 'docs/ANA-11.md',
                               '.claude/rules/workflow-docs.md', '.claude/rules/concept-docs.md',
                               '.clinerules/workflow-docs.md', '.clinerules/concept-docs.md')) {
            if ($probe11 -cnotmatch $emitted11) { $failures += "case11: pattern misses $probe11" }
        }
        foreach ($probe11 in @('CONCEPTS.md', 'README.md', '.claude/rules/minimalism.md', '.clinerules/gortex.md',
                               'docs/sub/ANA-1.md', '.claude/skills/handoff-run/SKILL.md')) {
            if ($probe11 -cmatch $emitted11) { $failures += "case11: pattern over-matches $probe11" }
        }
        $hooks11 = Join-Path $PSScriptRoot '..' '..' '..' '..' 'scripts' 'githooks'
        foreach ($committed11 in @((Join-Path $hooks11 'pre-commit'), (Join-Path $hooks11 'pre-commit.ps1'))) {
            if ((Test-Path -LiteralPath $committed11) -and
                -not ([System.IO.File]::ReadAllText($committed11)).Contains("'$DocsPattern'")) {
                $failures += "case11: $(Split-Path -Leaf $committed11) watches a different pattern"
            }
        }

        # Case 12-13: committed hooks (TOOL-9). core.hooksPath is set once, and the installer's
        # own pre-commit block goes while a foreign block stays. (The .sh twin's cases 14-15, which
        # run the committed sh hook itself, have no equivalent here: that hook is sh.)
        $ws12 = Join-Path $tmp 'case12' 'repo'
        New-Item -ItemType Directory -Force (Join-Path $ws12 'scripts' 'githooks') | Out-Null
        & git init -q $ws12
        Set-Content -LiteralPath (Join-Path $ws12 'scripts' 'githooks' 'pre-commit') -Value '#!/bin/sh'
        $own12 = Join-Path $ws12 '.git' 'hooks' 'pre-commit'
        [System.IO.File]::WriteAllText($own12, ((@('#!/bin/sh') + $foreignBlock) -join "`n") + "`n")
        Set-MarkerBlock -HookPath $own12 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        Set-HooksPath -Root $ws12
        if ((Get-HooksPath -Root $ws12) -ne 'scripts/githooks') { $failures += 'case12: core.hooksPath not set' }
        Install-WorkflowHooks -Root $ws12 -Quiet
        $raw12 = [System.IO.File]::ReadAllText($own12)
        if ($raw12 -match [regex]::Escape($StartMarker)) { $failures += 'case13: own pre-commit block survived next to the committed hook' }
        if ($raw12 -notmatch [regex]::Escape('# graphify-hook-start')) { $failures += 'case13: foreign pre-commit block lost' }
        $h12a = (Get-FileHash $own12 -Algorithm SHA256).Hash
        Set-HooksPath -Root $ws12
        Install-WorkflowHooks -Root $ws12 -Quiet
        if ((Get-FileHash $own12 -Algorithm SHA256).Hash -ne $h12a) { $failures += 'case12: second install changed the hook' }
    } finally {
        if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    }

    if ($failures.Count -eq 0) {
        Write-Host 'Self-test: 13/13 cases PASS.'
        return 0
    }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-SelfTest) }

if (-not $RepoRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..' '..' '..' '..')).Path
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "RepoRoot not found: $RepoRoot"
    exit 2
}
$RepoRoot = [System.IO.Path]::GetFullPath($RepoRoot).TrimEnd('\', '/')

Set-HooksPath -Root $RepoRoot
Install-WorkflowHooks -Root $RepoRoot
exit 0
