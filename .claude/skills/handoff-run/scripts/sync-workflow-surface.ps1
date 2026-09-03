# sync-workflow-surface.ps1 — distribute the workflow surface from the workspace
# (single source) into sub-repos: .claude/rules/workflow-docs.md, the
# .claude/skills/handoff-docs.md overview, plus the
# .claude/skills/handoff-run and handoff-add
# directories, including
# the .workflow_version marker. Mirror semantics: synced dirs are wholly owned by
# the surface — stale files in targets are removed. VulkanTutorials is deliberately
# not a target (frozen donor repo, no work items run there).
#
# Usage:
#   pwsh sync-workflow-surface.ps1 [-WorkspaceRoot <path>] [-Targets <addresses>] [-Check]
#
# -Check: no writes; SHA256-compare every synced file across all targets, list
#         drift (missing/extra/modified), exit 1 on any.
#
# A target is an ADDRESS, not a child name: 'engine' (child of the workspace),
# '../engine-template' (a sibling outside the tree) and an absolute path all work.
# A trailing '?' marks the target OPTIONAL — absent on this checkout is a warning
# and a skip, not a failure. Everything else stays a hard exit 2, unchanged.
#
# Exit codes: 0 synced/clean, 1 drift found (-Check), 2 usage/missing paths.

[CmdletBinding()]
param(
    [string]$WorkspaceRoot,
    # engine-template lives OUTSIDE the workspace tree (MOD-43) and is optional:
    # it is a standalone repo, so a checkout without it must still sync the rest.
    [string[]]$Targets = @('engine', '../engine-template?'),
    [switch]$Check
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $WorkspaceRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — root is four levels up.
    $WorkspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
}
if (-not (Test-Path -LiteralPath $WorkspaceRoot)) {
    Write-Host "WorkspaceRoot not found: $WorkspaceRoot"
    exit 2
}
# Resolve to absolute: rel-path math below (FullName.Substring) needs absolute prefixes,
# and callers may pass a relative root (the sub-repo pre-commit hook passes "..").
$WorkspaceRoot = (Resolve-Path -LiteralPath $WorkspaceRoot).Path

# Synced set, relative to <repo>/.claude/. Dirs are mirrored wholesale.
$SyncFiles = @('rules\workflow-docs.md', 'rules\concept-docs.md', 'skills\handoff-docs.md')
$SyncDirs = @('skills\handoff-run', 'skills\handoff-add')

$sourceClaude = Join-Path $WorkspaceRoot '.claude'
foreach ($rel in $SyncFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $sourceClaude $rel))) {
        Write-Host "Source file missing: .claude\$rel"
        exit 2
    }
}
foreach ($rel in $SyncDirs) {
    if (-not (Test-Path -LiteralPath (Join-Path $sourceClaude $rel))) {
        Write-Host "Source dir missing: .claude\$rel"
        exit 2
    }
}

function Get-ContentHash {
    # SHA256 of the file's content with CR stripped — line endings are ENCODING, not
    # content, and comparing raw bytes makes a .gitattributes difference look like drift.
    # engine-template carries '* text=auto eol=lf' (so its build.sh runs on Linux), so its
    # checked-out surface is LF while this Windows workspace's source is CRLF: a raw hash
    # would report all 22 files as drift in a fresh clone and block every commit that
    # stages one. Git itself compares normalized, and so does this.
    # Array.FindAll, not a Where-Object pipeline: the latter walks every byte of every file
    # through the PowerShell pipeline one object at a time, which is orders of magnitude
    # slower and would be a real cliff if anything large ever joined the synced set.
    param([string]$Path)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $stripped = [System.Array]::FindAll($bytes, [System.Predicate[byte]] { param($b) $b -ne 13 })
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        return [System.BitConverter]::ToString($sha.ComputeHash($stripped)).Replace('-', '')
    } finally {
        $sha.Dispose()
    }
}

function Resolve-TargetEntry {
    # One -Targets entry -> { Root, Label, IsOptional }. Kept in lockstep with the
    # identical helper in install-workflow-hooks.ps1 and both .sh twins.
    #
    # Path.Combine, NEVER the Join-Path cmdlet: Join-Path does not special-case an
    # already-absolute second argument, so an absolute entry (which is exactly what
    # the generated pre-commit hook passes) would come back as
    # '<workspace>\C:\...\engine-template'. Path.Combine lets a rooted second arg win.
    # GetFullPath then removes the '..' segments — without that, rel-path math over
    # Get-ChildItem's normalized FullName silently over-skips by the length of the
    # un-normalized span and yields truncated relative paths (measured: 'rules\a.md'
    # came back as 'a.md'), which -Check would report as bogus drift.
    param([string]$WorkspaceRoot, [string]$Entry)
    $isOptional = $Entry.EndsWith('?')
    $addr = if ($isOptional) { $Entry.Substring(0, $Entry.Length - 1) } else { $Entry }
    $normalized = [System.IO.Path]::GetFullPath(
        [System.IO.Path]::Combine($WorkspaceRoot, $addr)).TrimEnd('\', '/')
    return [pscustomobject]@{
        Root       = $normalized
        # Label is the leaf of the RESOLVED path, so '../engine-template' and the
        # absolute form print the same name in drift messages.
        Label      = (Split-Path -Leaf $normalized)
        IsOptional = $isOptional
    }
}

function Get-SyncedRelPaths {
    # Every file in the synced set, relative to .claude\ of the given root.
    param([string]$ClaudeDir)
    $rels = @()
    foreach ($rel in $SyncFiles) {
        if (Test-Path -LiteralPath (Join-Path $ClaudeDir $rel)) { $rels += $rel }
    }
    foreach ($rel in $SyncDirs) {
        $dir = Join-Path $ClaudeDir $rel
        if (Test-Path -LiteralPath $dir) {
            $rels += Get-ChildItem -Recurse -File -Force $dir | ForEach-Object {
                $_.FullName.Substring($ClaudeDir.Length + 1)
            }
        }
    }
    return $rels
}

$sourceRels = @(Get-SyncedRelPaths -ClaudeDir $sourceClaude)
$drift = @()

$processed = 0

foreach ($t in $Targets) {
    $entry = Resolve-TargetEntry -WorkspaceRoot $WorkspaceRoot -Entry $t
    $targetRoot = $entry.Root
    $label = $entry.Label
    if (-not (Test-Path -LiteralPath $targetRoot)) {
        if ($entry.IsOptional) {
            Write-Host "Target repo not found (optional, skipping): $targetRoot"
            continue
        }
        Write-Host "Target repo not found: $targetRoot"
        exit 2
    }
    $processed++
    $targetClaude = Join-Path $targetRoot '.claude'

    if ($Check) {
        $targetRels = @(Get-SyncedRelPaths -ClaudeDir $targetClaude)
        foreach ($rel in $sourceRels) {
            $srcFile = Join-Path $sourceClaude $rel
            $dstFile = Join-Path $targetClaude $rel
            if (-not (Test-Path -LiteralPath $dstFile)) {
                $drift += "${label}: missing .claude\$rel"
            } elseif ((Get-ContentHash $srcFile) -ne (Get-ContentHash $dstFile)) {
                $drift += "${label}: modified .claude\$rel"
            }
        }
        foreach ($rel in $targetRels) {
            if ($sourceRels -notcontains $rel) { $drift += "${label}: extra .claude\$rel" }
        }
    } else {
        foreach ($rel in $SyncFiles) {
            $dst = Join-Path $targetClaude $rel
            New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
            Copy-Item -LiteralPath (Join-Path $sourceClaude $rel) -Destination $dst -Force
        }
        foreach ($rel in $SyncDirs) {
            $dst = Join-Path $targetClaude $rel
            if (Test-Path -LiteralPath $dst) { Remove-Item -Recurse -Force $dst }
            New-Item -ItemType Directory -Force (Split-Path $dst) | Out-Null
            Copy-Item -Recurse -LiteralPath (Join-Path $sourceClaude $rel) -Destination $dst
        }
        Write-Host "Synced $($sourceRels.Count) files -> $label"
    }
}

if ($Check) {
    if ($drift.Count -gt 0) {
        $drift | ForEach-Object { Write-Host "[DRIFT] $_" }
        Write-Host "workflow-surface check: $($drift.Count) drift finding(s) across $processed target(s)."
        exit 1
    }
    Write-Host "workflow-surface check: clean — $($sourceRels.Count) files identical in $processed target(s)."
}
exit 0
