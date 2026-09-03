# install-workflow-hooks.ps1 — install/refresh the workflow git hooks (M3) in the
# workspace and the synced sub-repos. Hooks live in .git/hooks (untracked), so the
# file sync cannot carry them — this installer is the delivery mechanism.
#
# What it installs (idempotent marker blocks, '# workflow-hook-start/end'):
#   pre-commit  (workspace + sub-repos): staged HANDOFF.md/DECISIONS.md/docs/decisions/**/docs/ANA-*.md
#               -> run validate-workflow-docs.ps1 -ScopePaths "<staged>", block on errors
#               in the staged files (exit 1). The validator still checks the whole repo and
#               still prints everything it finds; -ScopePaths only stops findings in
#               untouched files from failing an unrelated commit.
#               Sub-repos additionally block staged edits to synced surface files
#               (owned by the workspace source — drift by construction).
#   post-commit (workspace only, prepended before foreign blocks): commit touching
#               surface source -> run sync-workflow-surface.ps1 + refresh hooks.
#
# Escape hatch (printed on block): WORKFLOW_SKIP_HOOK=1. Deliberately NOT --no-verify: a
# separate PreToolUse hook hard-blocks that flag for agent sessions, so advertising it
# would be dead advice - and after the -ScopePaths change an unrelated commit needs
# neither hatch.
#
# Usage:
#   pwsh install-workflow-hooks.ps1 [-WorkspaceRoot <path>] [-Targets <addresses>] [-SelfTest]
#
# A target is an ADDRESS, not a child name: 'engine', '../engine-template' and an
# absolute path all work; a trailing '?' marks it optional (absent = warn and skip).
# Same convention as sync-workflow-surface.{ps1,sh}.
#
# Exit codes: 0 installed, 1 self-test failure, 2 usage/missing paths.

[CmdletBinding()]
param(
    [string]$WorkspaceRoot,
    # engine-template lives OUTSIDE the workspace tree (MOD-43) and is optional.
    [string[]]$Targets = @('engine', '../engine-template?'),
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$StartMarker = '# workflow-hook-start'
$EndMarker = '# workflow-hook-end'
# Single source of truth for the sh-side path regexes (kept identical across variants).
$DocsPattern = '^HANDOFF\.md$|^DECISIONS\.md$|^docs/decisions/.+\.md$|^docs/ANA-[^/]+\.md$'
$SurfacePattern = '^\.claude/rules/(workflow-docs|concept-docs)\.md$|^\.claude/skills/handoff-docs\.md$|^\.claude/skills/handoff-(run|add)/'

function Resolve-TargetEntry {
    # One -Targets entry -> { Root, Label, IsOptional }. Duplicated VERBATIM from
    # sync-workflow-surface.ps1 (these scripts are standalone, there is no shared
    # module) — keep the two in lockstep, and both .sh twins with them.
    #
    # Path.Combine, NEVER the Join-Path cmdlet: Join-Path does not special-case an
    # already-absolute second argument, and this script bakes absolute target
    # addresses into the generated pre-commit block, which feeds them straight back
    # into the sync script's -Targets.
    param([string]$WorkspaceRoot, [string]$Entry)
    $isOptional = $Entry.EndsWith('?')
    $addr = if ($isOptional) { $Entry.Substring(0, $Entry.Length - 1) } else { $Entry }
    $normalized = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::Combine($WorkspaceRoot, $addr)).TrimEnd('\', '/')
    return [pscustomobject]@{
        Root       = $normalized
        Label      = (Split-Path -Leaf $normalized)
        IsOptional = $isOptional
    }
}

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

function ConvertTo-ShSingleQuoted {
    # A path going into a single-quoted POSIX string. An apostrophe in it (a redirected
    # profile, a user named O'Brien) would otherwise close the string early and leave the
    # rest of the path as shell code in a hook that runs on every commit. The POSIX idiom
    # for a literal quote inside single quotes is '\'' — close, escaped quote, reopen.
    param([string]$Value)
    return $Value.Replace("'", "'\''")
}

function New-PreCommitBlock {
    param([switch]$IsSubRepo, [string]$WorkspaceRootLiteral, [string]$TargetLiteral)
    # No default for either literal: a cwd-derived fallback ('..', '.') is exactly the
    # broken addressing the baked literals exist to remove, and a silent one would be
    # untestable. Matches the same guard in the .sh twin.
    if ($IsSubRepo -and (-not $WorkspaceRootLiteral -or -not $TargetLiteral)) {
        throw 'New-PreCommitBlock: sub-repo block needs workspace-root and target literals'
    }
    # No 'exit 0' anywhere in this block: it is prepended before any foreign hook blocks,
    # and an early exit 0 here would swallow them. Only 'exit 1' (a deliberate block) may
    # terminate; every pass path falls through past the end marker.
    $lines = @(
        $StartMarker,
        '# Blocks commits with structural findings in workflow docs (HANDOFF.md/DECISIONS.md/docs/decisions/'
        '# docs/ANA-*.md). Installed by: pwsh .claude/skills/handoff-run/scripts/install-workflow-hooks.ps1'
        '# Escape hatch: WORKFLOW_SKIP_HOOK=1. Bypassing git hooks outright is blocked for'
        '# agent sessions, and an unrelated commit needs no hatch: only findings in the'
        '# files this commit stages block it.'
        ''
        'WF_STAGED=$(git diff --cached --name-only)'
        "WF_DOCS_PATTERN='$DocsPattern'"
        "WF_SURFACE_PATTERN='$SurfacePattern'"
        ''
        '# Fast path: pwsh only spawns when watched paths are staged.'
        'if [ -n "$WF_STAGED" ] && [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ] \'
        '   && echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN|$WF_SURFACE_PATTERN"; then'
    )
    if ($IsSubRepo) {
        # Drift-aware guard: committing surface files that MATCH the workspace source is
        # legitimate (that's how sync results get committed); only divergence blocks.
        $lines += @(
            '    WF_SURFACE_HITS=$(echo "$WF_STAGED" | grep -E "$WF_SURFACE_PATTERN")'
            '    if [ -n "$WF_SURFACE_HITS" ]; then'
            '        # Both paths are baked in at install time. Deriving them from cwd'
            '        # ("..", basename $(pwd)) only works for a repo that is a CHILD of the'
            '        # workspace; a sibling target resolves ".." to a directory with no'
            '        # .claude source at all, and the guard would block every commit.'
            "        WF_WORKSPACE_ROOT='$(ConvertTo-ShSingleQuoted $WorkspaceRootLiteral)'"
            "        WF_TARGET='$(ConvertTo-ShSingleQuoted $TargetLiteral)'"
            '        if ! pwsh -NoProfile -File ".claude/skills/handoff-run/scripts/sync-workflow-surface.ps1" -WorkspaceRoot "$WF_WORKSPACE_ROOT" -Targets "$WF_TARGET" -Check >/dev/null 2>&1; then'
            '            echo "[workflow-hook] blocked: staged workflow-surface files differ from the workspace source." >&2'
            '            echo "[workflow-hook] The surface is owned by the workspace - edit ../.claude/... there and run" >&2'
            '            echo "[workflow-hook] sync-workflow-surface.ps1 instead of editing the copy here. Staged paths:" >&2'
            '            echo "$WF_SURFACE_HITS" | sed ''s/^/[workflow-hook]   /'' >&2'
            '            echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
            '            exit 1'
            '        fi'
            '    fi'
        )
    }
    $lines += @(
        '    if echo "$WF_STAGED" | grep -E -q "$WF_DOCS_PATTERN"; then'
        '        # -ScopePaths: whole-repo checks, but only findings in the staged files may'
        '        # block. Pre-existing findings elsewhere print as [WARNING] (pre-existing ...)'
        '        # and still fail a direct validate-workflow-docs.ps1 run.'
        '        pwsh -NoProfile -File ".claude/skills/handoff-run/scripts/validate-workflow-docs.ps1" -ScopePaths "$WF_STAGED"'
        '        wf_status=$?'
        '        if [ "$wf_status" -ne 0 ]; then'
        '            echo "[workflow-hook] validate-workflow-docs.ps1 exited $wf_status - findings above, in files this commit stages." >&2'
        '            echo "[workflow-hook] escape hatch: WORKFLOW_SKIP_HOOK=1 git commit ..." >&2'
        '            exit 1'
        '        fi'
        '    fi'
        'fi'
        $EndMarker
    )
    return $lines
}

function New-PostCommitBlock {
    $lines = @(
        $StartMarker,
        '# Non-blocking: after a workspace commit touching the workflow-surface source, mirrors'
        '# it into the sub-repos and refreshes their git hooks (hooks are untracked, so the'
        '# file sync alone cannot carry them). Deliberately placed before other hook blocks:'
        '# blocks below may exit 0 early and must not swallow this one.'
        '# Installed by: pwsh .claude/skills/handoff-run/scripts/install-workflow-hooks.ps1'
        ''
        'if [ "${WORKFLOW_SKIP_HOOK:-0}" != "1" ]; then'
        '    WF_CHANGED=$(git diff-tree --no-commit-id --name-only -r HEAD 2>/dev/null)'
        "    WF_SURFACE_PATTERN='$SurfacePattern'"
        '    if [ -n "$WF_CHANGED" ] && echo "$WF_CHANGED" | grep -E -q "$WF_SURFACE_PATTERN"; then'
        '        echo "[workflow-hook] surface source changed - syncing to sub-repos..."'
        '        pwsh -NoProfile -File ".claude/skills/handoff-run/scripts/sync-workflow-surface.ps1"'
        '        pwsh -NoProfile -File ".claude/skills/handoff-run/scripts/install-workflow-hooks.ps1"'
        '    fi'
        'fi'
        $EndMarker
    )
    return $lines
}

function Get-HookTargets {
    param([string]$Root, [string[]]$Names)
    $rootAbs = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $list = @([pscustomobject]@{ Name = 'workspace'; Root = $rootAbs; IsWorkspace = $true })
    foreach ($n in $Names) {
        $entry = Resolve-TargetEntry -WorkspaceRoot $rootAbs -Entry $n
        if (-not (Test-Path -LiteralPath $entry.Root)) {
            if ($entry.IsOptional) {
                Write-Host "Target repo not found (optional, skipping): $($entry.Root)"
                continue
            }
            Write-Host "Target repo not found: $($entry.Root)"
            exit 2
        }
        $list += [pscustomobject]@{ Name = $entry.Label; Root = $entry.Root; IsWorkspace = $false }
    }
    return $list
}

function Install-WorkflowHooks {
    param([object[]]$TargetList, [switch]$Quiet)
    # Index 0 is always the workspace entry (Get-HookTargets' contract). The sub-repo
    # drift guard is generated with THIS root baked in, so a target outside the
    # workspace tree resolves it correctly instead of guessing from its own cwd.
    $workspaceRootLiteral = $TargetList[0].Root
    foreach ($t in $TargetList) {
        # A present-but-broken checkout is a real defect even for an optional target:
        # "optional" covers an absent repo, never one that exists without .git/hooks.
        $hooksDir = Join-Path $t.Root '.git' 'hooks'
        if (-not (Test-Path -LiteralPath $hooksDir)) {
            Write-Host "Hooks dir not found: $hooksDir"
            exit 2
        }
        $preCommitLines = if ($t.IsWorkspace) {
            New-PreCommitBlock
        } else {
            New-PreCommitBlock -IsSubRepo -WorkspaceRootLiteral $workspaceRootLiteral `
                -TargetLiteral $t.Root
        }
        # -InsertAfterShebang for both hooks: a foreign block with an early exit 0 must
        # never sit above ours and swallow it.
        Set-MarkerBlock -HookPath (Join-Path $hooksDir 'pre-commit') `
            -BlockLines $preCommitLines -InsertAfterShebang
        if ($t.IsWorkspace) {
            Set-MarkerBlock -HookPath (Join-Path $hooksDir 'post-commit') `
                -BlockLines (New-PostCommitBlock) -InsertAfterShebang
        }
        if (-not $Quiet) { Write-Host "Installed workflow hooks -> $($t.Name)" }
    }
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
        $h2 = (Get-FileHash $p1 -Algorithm SHA256).Hash
        if ($h1 -ne $h2) { $failures += 'case2: re-install not byte-identical' }

        # Case 3: prepend before existing foreign block, foreign preserved.
        $p3 = Join-Path $tmp 'post-commit-3'
        [System.IO.File]::WriteAllText($p3, ((@('#!/bin/sh') + $foreignBlock -join "`n") + "`n"))
        Set-MarkerBlock -HookPath $p3 -BlockLines (New-PostCommitBlock) -InsertAfterShebang
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
        $p4 = Join-Path $tmp 'post-commit-4'
        [System.IO.File]::WriteAllText($p4, ((@('#!/bin/sh') + $stale + $foreignBlock -join "`n") + "`n"))
        Set-MarkerBlock -HookPath $p4 -BlockLines (New-PostCommitBlock) -InsertAfterShebang
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
        $p6 = Join-Path $tmp 'post-commit-6'
        [System.IO.File]::WriteAllText($p6, ((@('#!/bin/sh') + $foreignBlock -join "`r`n") + "`r`n"))
        Set-MarkerBlock -HookPath $p6 -BlockLines (New-PostCommitBlock) -InsertAfterShebang
        if (([System.IO.File]::ReadAllText($p6)).Contains("`r")) { $failures += 'case6: CR bytes remain after install' }

        # Case 7: full Install-WorkflowHooks over a fixture repo tree.
        $ws = Join-Path $tmp 'case7' 'workspace'
        $names = @('engine', 'vfs', 'settings', 'logging')
        New-Item -ItemType Directory -Path (Join-Path $ws '.git' 'hooks') -Force | Out-Null
        [System.IO.File]::WriteAllText((Join-Path $ws '.git' 'hooks' 'post-commit'),
            ((@('#!/bin/sh') + $foreignBlock -join "`n") + "`n"))
        foreach ($n in $names) {
            New-Item -ItemType Directory -Path (Join-Path $ws $n '.git' 'hooks') -Force | Out-Null
        }
        Install-WorkflowHooks -TargetList (Get-HookTargets -Root $ws -Names $names) -Quiet
        $wsPre = [System.IO.File]::ReadAllText((Join-Path $ws '.git' 'hooks' 'pre-commit'))
        if ($wsPre.Contains('owned by the workspace')) { $failures += 'case7: workspace pre-commit has sub-repo surface guard' }
        foreach ($n in $names) {
            $subPre = [System.IO.File]::ReadAllText((Join-Path $ws $n '.git' 'hooks' 'pre-commit'))
            if (-not $subPre.Contains('owned by the workspace')) { $failures += "case7: $n pre-commit missing surface guard" }
        }
        $wsPost = [System.IO.File]::ReadAllText((Join-Path $ws '.git' 'hooks' 'post-commit'))
        if (-not $wsPost.Contains(($foreignBlock -join "`n"))) { $failures += 'case7: workspace post-commit foreign block altered' }
        if (-not $wsPost.Contains('sync-workflow-surface.ps1')) { $failures += 'case7: workspace post-commit missing sync block' }
        foreach ($n in $names) {
            if (Test-Path (Join-Path $ws $n '.git' 'hooks' 'post-commit')) {
                $failures += "case7: $n unexpectedly got a post-commit hook"
            }
        }

        # Case 8: foreign pre-commit block present -> workflow block prepended before it
        # (and contains no exit 0 that could swallow it).
        $p8 = Join-Path $tmp 'pre-commit-8'
        [System.IO.File]::WriteAllText($p8, ((@('#!/bin/sh') + $foreignBlock -join "`n") + "`n"))
        Set-MarkerBlock -HookPath $p8 -BlockLines (New-PreCommitBlock -IsSubRepo `
            -WorkspaceRootLiteral "$tmp\ws" -TargetLiteral "$tmp\target") -InsertAfterShebang
        $c8 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($p8)))
        $own8 = Find-MarkerIndex -Lines $c8 -Marker $StartMarker
        $for8 = Find-MarkerIndex -Lines $c8 -Marker '# graphify-hook-start'
        $end8 = Find-MarkerIndex -Lines $c8 -Marker $EndMarker
        if ($own8 -lt 0 -or $for8 -lt 0 -or $own8 -gt $for8) { $failures += 'case8: pre-commit block not prepended before foreign block' }
        foreach ($i in $own8..$end8) {
            if ($c8[$i] -match '\bexit 0\b') { $failures += "case8: pre-commit block contains 'exit 0' (would swallow foreign blocks)"; break }
        }

        # Case 10: the pre-commit block hands the staged paths to the validator (so a
        # pre-existing finding in an untouched file cannot block an unrelated commit) and
        # no longer advertises --no-verify, which a PreToolUse hook hard-blocks for agents.
        $p10 = Join-Path $tmp 'pre-commit-10'
        Set-MarkerBlock -HookPath $p10 -BlockLines (New-PreCommitBlock -IsSubRepo `
            -WorkspaceRootLiteral "$tmp\ws" -TargetLiteral "$tmp\target")
        $raw10 = [System.IO.File]::ReadAllText($p10)
        if ($raw10 -notmatch [regex]::Escape('validate-workflow-docs.ps1" -ScopePaths "$WF_STAGED"')) {
            $failures += 'case10: validator invoked without -ScopePaths (whole repo would block)'
        }
        if ($raw10 -match '--no-verify') { $failures += 'case10: block still advertises --no-verify' }
        if ($raw10 -notmatch [regex]::Escape('WORKFLOW_SKIP_HOOK=1 git commit')) {
            $failures += 'case10: block lost the WORKFLOW_SKIP_HOOK escape hatch'
        }

        # Case 9: existing but empty hook file still gets a shebang.
        $p9 = Join-Path $tmp 'pre-commit-9'
        [System.IO.File]::WriteAllText($p9, '')
        Set-MarkerBlock -HookPath $p9 -BlockLines (New-PreCommitBlock) -InsertAfterShebang
        $c9 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($p9)))
        if ($c9[0] -ne '#!/bin/sh') { $failures += 'case9: empty existing file missing shebang' }

        # Case 11: the emitted surface pattern covers every synced rule file, not just the
        # first one — concept-docs.md joined $SyncFiles with MOD-11, and a rule file the
        # pattern misses is silently unguarded in both places it is embedded (the sub-repo
        # drift guard and the workspace post-commit auto-sync). Probed by running the
        # emitted regex, not by matching its text, and paired with a negative probe so
        # widening it to something that matches everything fails too.
        $p11a = Join-Path $tmp 'pre-commit-11'
        $p11b = Join-Path $tmp 'post-commit-11'
        Set-MarkerBlock -HookPath $p11a -BlockLines (New-PreCommitBlock -IsSubRepo `
            -WorkspaceRootLiteral "$tmp\ws" -TargetLiteral "$tmp\target")
        Set-MarkerBlock -HookPath $p11b -BlockLines (New-PostCommitBlock)
        foreach ($hook11 in @($p11a, $p11b)) {
            $name11 = Split-Path -Leaf $hook11
            $line11 = @(ConvertTo-LfLines ([System.IO.File]::ReadAllText($hook11))) |
                Where-Object { $_ -match "WF_SURFACE_PATTERN='" } | Select-Object -First 1
            if (-not $line11) { $failures += "case11: no WF_SURFACE_PATTERN emitted into $name11"; continue }
            $emitted11 = ($line11 -replace "^.*WF_SURFACE_PATTERN='", '') -replace "'$", ''
            # -cmatch/-cnotmatch, not -match/-notmatch: PowerShell's default is
            # case-insensitive and the sh half probes with case-sensitive grep -E. Keeping
            # the two halves semantically identical matters more than the current probe set,
            # none of which distinguishes case today.
            foreach ($probe11 in @('.claude/rules/workflow-docs.md', '.claude/rules/concept-docs.md',
                                   '.claude/skills/handoff-docs.md', '.claude/skills/handoff-run/scripts/x.sh')) {
                if ($probe11 -cnotmatch $emitted11) { $failures += "case11: surface pattern misses $probe11 in $name11" }
            }
            foreach ($probe11 in @('docs/ANA-11.md', 'CONCEPTS.md', '.claude/rules/minimalism.md')) {
                if ($probe11 -cmatch $emitted11) { $failures += "case11: surface pattern over-matches $probe11 in $name11" }
            }
        }
        # Case 12: a target OUTSIDE the workspace tree installs, and its drift guard
        # carries baked absolute literals instead of the cwd-derived ".."/basename pair
        # that only ever worked for a workspace child (MOD-43 M3). Fixture mirrors the
        # real dingine/engine-template layout: workspace and target are siblings.
        $ws12 = Join-Path $tmp 'case12\workspace'
        $ext12 = Join-Path $tmp 'case12\external-template'
        New-Item -ItemType Directory -Force (Join-Path $ws12 '.git\hooks') | Out-Null
        New-Item -ItemType Directory -Force (Join-Path $ext12 '.git\hooks') | Out-Null
        $targets12 = @(Get-HookTargets -Root $ws12 -Names @('../external-template'))
        if ($targets12.Count -ne 2) {
            $failures += "case12: expected workspace + 1 external target, got $($targets12.Count)"
        } elseif ($targets12[1].Name -ne 'external-template') {
            $failures += "case12: label not taken from the resolved leaf: $($targets12[1].Name)"
        }
        Install-WorkflowHooks -TargetList $targets12 -Quiet
        $hook12 = Join-Path $ext12 '.git\hooks\pre-commit'
        $raw12 = [System.IO.File]::ReadAllText($hook12)
        if ($raw12 -notmatch [regex]::Escape("WF_WORKSPACE_ROOT='$([System.IO.Path]::GetFullPath($ws12).TrimEnd('\'))'")) {
            $failures += 'case12: workspace root not baked into the external target hook'
        }
        if ($raw12 -notmatch [regex]::Escape("WF_TARGET='$([System.IO.Path]::GetFullPath($ext12).TrimEnd('\'))'")) {
            $failures += 'case12: target root not baked into the external target hook'
        }
        if ($raw12 -match [regex]::Escape('basename "$(pwd)"') -or $raw12 -match [regex]::Escape('-WorkspaceRoot ".."')) {
            $failures += 'case12: cwd-derived drift-guard literals are still emitted'
        }
        $h12a = (Get-FileHash $hook12 -Algorithm SHA256).Hash
        Install-WorkflowHooks -TargetList @(Get-HookTargets -Root $ws12 -Names @('../external-template')) -Quiet
        if ((Get-FileHash $hook12 -Algorithm SHA256).Hash -ne $h12a) {
            $failures += 'case12: re-install of an external target not byte-identical'
        }

        # Case 13: optional-missing skips, required-missing still exits 2. The second half
        # runs in a child process — exit 2 in this one would kill the self-test itself,
        # which is precisely why the regression is easy to introduce unnoticed.
        $ws13 = Join-Path $tmp 'case13\workspace'
        New-Item -ItemType Directory -Force (Join-Path $ws13 '.git\hooks') | Out-Null
        $targets13 = @(Get-HookTargets -Root $ws13 -Names @('../does-not-exist?'))
        if ($targets13.Count -ne 1 -or -not $targets13[0].IsWorkspace) {
            $failures += "case13: optional missing target was not skipped (got $($targets13.Count) entries)"
        }
        & pwsh -NoProfile -File $PSCommandPath -WorkspaceRoot $ws13 -Targets 'does-not-exist-required' | Out-Null
        if ($LASTEXITCODE -ne 2) {
            $failures += "case13: required missing target exited $LASTEXITCODE, expected 2"
        }
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

if (-not $WorkspaceRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    $WorkspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..' '..' '..' '..')).Path
}
if (-not (Test-Path -LiteralPath $WorkspaceRoot)) {
    Write-Host "WorkspaceRoot not found: $WorkspaceRoot"
    exit 2
}
# Normalize an explicitly-passed root as well: it is baked into every sub-repo hook
# as WF_WORKSPACE_ROOT, and a relative one there would be read against the committing
# repo's cwd — the exact breakage the baked literals exist to remove.
$WorkspaceRoot = [System.IO.Path]::GetFullPath($WorkspaceRoot).TrimEnd('\', '/')

Install-WorkflowHooks -TargetList (Get-HookTargets -Root $WorkspaceRoot -Names $Targets)
exit 0
