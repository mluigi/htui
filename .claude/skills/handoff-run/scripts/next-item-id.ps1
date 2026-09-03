# next-item-id.ps1 — the owned-ID mint of .claude/rules/workflow-docs.md, as a command.
# Reports the next free item ID for a prefix from OWNED IDs only: open checklist lines in
# HANDOFF.md plus the archive (index lines in DECISIONS.md cross-checked against the
# docs/decisions/<prefix>/ listing). Never greps raw PREFIX-N mentions — cross-repo
# references ("engine MOD-11") are other repos' items and inflate the count.
#
# Purely file-derived and offline: it reads the working tree of the repo it is invoked in
# and nothing else. It never reads another repo's files.
#
# Usage:
#   pwsh next-item-id.ps1 -Prefix MOD [-RepoRoot <path>] [-IdOnly] [-Json]
#   pwsh next-item-id.ps1 [-All] [-RepoRoot <path>] [-Json]
#   pwsh next-item-id.ps1 -SelfTest
#
# Exit codes: 0 clean, 1 findings (including a missing HANDOFF.md — the ID space cannot be
#           trusted, so the mint is blocked), 2 unusable -RepoRoot or an out-of-set -Prefix.
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

[CmdletBinding()]
param(
    [string]$Prefix,
    [switch]$All,
    [string]$RepoRoot,
    [switch]$IdOnly,
    [switch]$Json,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'workflow-patterns.ps1')


function New-Finding {
    param([string]$Severity, [string]$Check, [string]$Message)
    [pscustomobject]@{ Severity = $Severity; Check = $Check; Message = $Message }
}

function Update-Max {
    # Hashtables are reference types, so this updates the caller's table in place. IDs whose
    # prefix is outside the legal six (a `FOO-3` index line) are reported elsewhere but hold
    # no place in any prefix's number line, so they never move a max.
    param([hashtable]$Table, [string]$Id)
    if ($Id -notmatch "^($PrefixPattern)-(\d+)$") { return }
    $p = $Matches[1]
    $n = [int]$Matches[2]
    if ($n -gt [int]($Table[$p] ?? 0)) { $Table[$p] = $n }
}

function Get-DecisionFileIds {
    # The directory half of the archive. Absent docs/decisions/ is legal (a repo with no
    # archive yet) — return empty, silently.
    param([string]$Root)
    $dir = Join-Path $Root 'docs/decisions'
    if (-not (Test-Path -LiteralPath $dir)) { return @() }
    # Resolve first: Get-ChildItem yields absolute FullNames, so the repo-relative path must
    # be cut using the ABSOLUTE root length — cutting by the length of a caller-supplied
    # relative root mangles every path.
    $prefixLen = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\', '/').Length + 1
    return @(Get-ChildItem -Recurse -File -Filter '*.md' -LiteralPath $dir | ForEach-Object {
        # An item write-up is named <prefix>-N.md. Anything else under docs/decisions/ is
        # supplementary prose (a README, notes) — it holds no ID.
        if ($_.Name -match '^([a-z]+)-(\d+)\.md$') {
            [pscustomobject]@{
                Id   = ($Matches[1].ToUpperInvariant() + '-' + $Matches[2])
                Path = $_.FullName.Substring($prefixLen).Replace('\', '/')
            }
        }
    })
}

function Get-IdSpace {
    # The mint itself: every owned ID, both halves, with a finding for anything that makes
    # the ID space untrustworthy. Returns
    #   Findings [object[]], Open @{prefix -> int}, Archived @{prefix -> int}
    param([string]$Root)
    $findings = @()
    $openIds = @()
    # Multiset on purpose: duplicate detection needs the repeats, not the set.
    $archiveIds = @()
    $indexIds = @()
    $legacyIds = @()

    $handoffPath = Join-Path $Root 'HANDOFF.md'
    if (-not (Test-Path -LiteralPath $handoffPath)) {
        $findings += New-Finding 'Error' 'files' "HANDOFF.md not found at $handoffPath"
    } else {
        foreach ($line in @(Get-Content -LiteralPath $handoffPath -Encoding UTF8)) {
            if ($line -match $ChecklistLinePattern) {
                $openIds += $Matches['id']
            } elseif ($line -match $ChecklistLineLoosePattern) {
                # Name the at-risk ID and keep counting it: a line that fails the strict
                # pattern and gets skipped drops its ID, and a dropped ID is minted twice.
                $looseId = $Matches['id']
                $findings += New-Finding 'Error' 'checklist-unparseable' `
                    "${looseId}: open-item line unparseable - ID space untrustworthy until fixed: $($line.Trim())"
                $openIds += $looseId
            }
        }
    }

    $decisionsPath = Join-Path $Root 'DECISIONS.md'
    if (Test-Path -LiteralPath $decisionsPath) {
        foreach ($line in @(Get-Content -LiteralPath $decisionsPath -Encoding UTF8)) {
            if ($line -match $IndexLinePattern) {
                $id = $Matches['id']
                $path = $Matches['path']
                $indexIds += $id
                $archiveIds += $id
                $prefix = ($id -split '-')[0].ToLowerInvariant()
                $expected = "docs/decisions/$prefix/$($id.ToLowerInvariant()).md"
                if ($path -cne $expected) {
                    $findings += New-Finding 'Error' 'index-id-path' `
                        "${id}: index line links $path, expected $expected."
                }
            } elseif ($line -match $IndexLineLoosePattern) {
                $looseId = $Matches['id']
                $findings += New-Finding 'Error' 'index-unparseable' `
                    "${looseId}: index line unparseable - ID space untrustworthy until fixed: $($line.Trim())"
                $indexIds += $looseId
                $archiveIds += $looseId
            } elseif (($ids = @(Get-LegacyHeaderIds -Line $line)).Count -gt 0) {
                $findings += New-Finding 'Error' 'legacy-header' `
                    ("$($ids -join '/'): legacy '## PREFIX-N' write-up section in DECISIONS.md; " +
                     'the write-up belongs at docs/decisions/<prefix>/<prefix>-N.md with an index line here.')
                $legacyIds += $ids
                $archiveIds += $ids
            }
        }
    }

    # An ID already accounted for by the archive — indexed, or still carried by a legacy
    # header mid-migration — is not orphaned by its file, and counting it a second time
    # would report a phantom duplicate-id for an ID that was never actually reused.
    $accountedIds = @($indexIds) + @($legacyIds)
    foreach ($file in @(Get-DecisionFileIds -Root $Root)) {
        if ($accountedIds -notcontains $file.Id) {
            # A write-up with no index line still holds a real ID. Counting it is the point:
            # the archive is inconsistent, but the number line must not shrink.
            $findings += New-Finding 'Error' 'decision-orphan' `
                "$($file.Id): $($file.Path) has no index line in DECISIONS.md."
            $archiveIds += $file.Id
        }
    }

    foreach ($g in @($openIds | Group-Object | Where-Object Count -gt 1)) {
        $findings += New-Finding 'Error' 'duplicate-id' "ID $($g.Name) open $($g.Count)x in HANDOFF.md."
    }
    foreach ($g in @($archiveIds | Group-Object | Where-Object Count -gt 1)) {
        $findings += New-Finding 'Error' 'duplicate-id' "ID $($g.Name) archived $($g.Count)x."
    }
    foreach ($id in @($openIds | Select-Object -Unique)) {
        if ($archiveIds -contains $id) {
            $findings += New-Finding 'Error' 'duplicate-id' `
                "ID $id is open in HANDOFF.md AND archived (IDs are never reused)."
        }
    }

    $open = @{}
    $archived = @{}
    foreach ($id in $openIds) { Update-Max -Table $open -Id $id }
    foreach ($id in $archiveIds) { Update-Max -Table $archived -Id $id }

    return [pscustomobject]@{ Findings = @($findings); Open = $open; Archived = $archived }
}

# ---- self-test ----

function New-Fixture {
    # Builds a fixture repo: HANDOFF.md (unless $NoHandoff), DECISIONS.md (unless
    # $DecisionsContent is $null) and any docs/decisions item files. $Files maps
    # repo-relative path -> content.
    # Content params are [object], not [string], on purpose: binding $null to a [string]
    # parameter yields '' and would create the very file a case means to leave absent.
    param(
        [string]$Dir,
        [object]$HandoffContent,
        [object]$DecisionsContent,
        [hashtable]$Files = @{}
    )
    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    if ($null -ne $HandoffContent) {
        Set-Content -Path (Join-Path $Dir 'HANDOFF.md') -Value ([string]$HandoffContent) -Encoding UTF8
    }
    if ($null -ne $DecisionsContent) {
        Set-Content -Path (Join-Path $Dir 'DECISIONS.md') -Value ([string]$DecisionsContent) -Encoding UTF8
    }
    foreach ($rel in $Files.Keys) {
        $full = Join-Path $Dir $rel
        New-Item -ItemType Directory -Path (Split-Path $full) -Force | Out-Null
        Set-Content -Path $full -Value $Files[$rel] -Encoding UTF8
    }
}

function Get-NextOf {
    # Test helper: the next ID string for a prefix from an already-computed space.
    param([object]$Space, [string]$P)
    $open = [int]($Space.Open[$P] ?? 0)
    $arch = [int]($Space.Archived[$P] ?? 0)
    $max = [Math]::Max($open, $arch)
    return "$P-$($max + 1)"
}

function Invoke-SelfTest {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("nextid-selftest-" + [guid]::NewGuid().ToString('N'))
    $failures = @()
    $script = $PSCommandPath

    $writeUp = @'
# MOD-8 - Thing (done, 2026-01-05)

Write-up body.
'@
    $emptyHandoff = @'
# HANDOFF

**Current status (2026-01-05):** nothing open.

## Open items

## Summary

| Area  | Open |
|-------|------|
'@

    function Test-Case {
        param([string]$Name, [scriptblock]$Body)
        try { & $Body } catch { return "${Name}: threw $($_.Exception.Message)" }
        return $null
    }

    try {
        # Case 1: open MOD-4 + archived MOD-8 -> next MOD-9.
        $c1 = Join-Path $tmp 'basic'
        New-Fixture -Dir $c1 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **MOD-4 - Thing.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-8.md' = $writeUp }
        $s = Get-IdSpace -Root $c1
        $e = @($s.Findings | Where-Object Severity -eq 'Error')
        if ($e.Count -ne 0) { $failures += "basic: expected 0 errors, got $($e.Count): $($e.Message -join '; ')" }
        if ((Get-NextOf $s 'MOD') -ne 'MOD-9') { $failures += "basic: expected MOD-9, got $(Get-NextOf $s 'MOD')" }

        # Case 2: prefix with no owned IDs at all starts at 1.
        if ((Get-NextOf $s 'TOOL') -ne 'TOOL-1') { $failures += "empty-prefix: expected TOOL-1, got $(Get-NextOf $s 'TOOL')" }

        # Case 3: open half is the max.
        $c3 = Join-Path $tmp 'openmax'
        New-Fixture -Dir $c3 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **MOD-12 - Thing.** body
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-8.md' = $writeUp }
        $s3 = Get-IdSpace -Root $c3
        if ((Get-NextOf $s3 'MOD') -ne 'MOD-13') { $failures += "openmax: expected MOD-13, got $(Get-NextOf $s3 'MOD')" }

        # Case 4: archive half is the max (the workspace's own TOOL case).
        $c4 = Join-Path $tmp 'archmax'
        New-Fixture -Dir $c4 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **MOD-2 - Thing.** body
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-8.md' = $writeUp }
        $s4 = Get-IdSpace -Root $c4
        if ((Get-NextOf $s4 'MOD') -ne 'MOD-9') { $failures += "archmax: expected MOD-9, got $(Get-NextOf $s4 'MOD')" }

        # Case 5: raw cross-repo mentions must NOT inflate the count. This is the whole
        # reason the mint is defined over owned IDs.
        $c5 = Join-Path $tmp 'crossrepo'
        New-Fixture -Dir $c5 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **MOD-4 - Thing.** blocked on engine MOD-11 (`../engine/HANDOFF.md`); see vfs MOD-1
  and VulkanTutorials MOD-20 for prior art. Also mentions MOD-99 in prose.
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-3](docs/decisions/mod/mod-3.md)** - Relocated to engine MOD-11 (done, 2026-01-04)
'@ -Files @{ 'docs/decisions/mod/mod-3.md' = $writeUp }
        $s5 = Get-IdSpace -Root $c5
        $e5 = @($s5.Findings | Where-Object Severity -eq 'Error')
        if ($e5.Count -ne 0) { $failures += "crossrepo: expected 0 errors, got $($e5.Message -join '; ')" }
        if ((Get-NextOf $s5 'MOD') -ne 'MOD-5') { $failures += "crossrepo: expected MOD-5, got $(Get-NextOf $s5 'MOD')" }

        # Case 6: unparseable index line -> error, and its ID still counts.
        $c6 = Join-Path $tmp 'unparseable'
        New-Fixture -Dir $c6 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $s6 = Get-IdSpace -Root $c6
        $f6 = @($s6.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'index-unparseable' })
        if ($f6.Count -lt 1) { $failures += 'unparseable: expected index-unparseable error, got none' }
        if ((Get-NextOf $s6 'MOD') -ne 'MOD-8') { $failures += "unparseable: ID dropped out of the count, got $(Get-NextOf $s6 'MOD')" }

        # Case 7: malformed in FORMAT rather than content — leading space, `*` bullet, a
        # prefix outside the legal six. Each still holds an ID meant as an index entry.
        $c7 = Join-Path $tmp 'badformat'
        New-Fixture -Dir $c7 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

 - **[MOD-7](docs/decisions/mod/mod-7.md)** - Leading space (done, 2026-01-05)
* **[MOD-8](docs/decisions/mod/mod-8.md)** - Star bullet (done, 2026-01-04)
- **[FOO-3](docs/decisions/foo/foo-3.md)** - Prefix outside the six (done, 2026-01-03)
'@
        $s7 = Get-IdSpace -Root $c7
        $f7 = @($s7.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'index-unparseable' })
        if ($f7.Count -lt 3) { $failures += "badformat: expected 3 index-unparseable errors, got $($f7.Count)" }
        if ((Get-NextOf $s7 'MOD') -ne 'MOD-9') { $failures += "badformat: expected MOD-9, got $(Get-NextOf $s7 'MOD')" }

        # Case 8: compound legacy header. `## MOD-18/19/20` hid two IDs from every
        # `^## MOD-\d+` scan during MOD-7 — all three must count, and the leftover section
        # is itself an error post-MOD-7.
        $c8 = Join-Path $tmp 'compound'
        New-Fixture -Dir $c8 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

## MOD-18/19/20 - Three items, one header (done, 2026-01-05)

Body, which mentions engine MOD-77 as prior art.
'@
        $s8 = Get-IdSpace -Root $c8
        $f8 = @($s8.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'legacy-header' })
        if ($f8.Count -lt 1) { $failures += 'compound: expected legacy-header error, got none' }
        if ((Get-NextOf $s8 'MOD') -ne 'MOD-21') { $failures += "compound: expected MOD-21, got $(Get-NextOf $s8 'MOD')" }

        # Case 9: write-up file with no index line -> orphan, ID still counts.
        $c9 = Join-Path $tmp 'orphan'
        New-Fixture -Dir $c9 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS
'@ -Files @{ 'docs/decisions/mod/mod-9.md' = $writeUp }
        $s9 = Get-IdSpace -Root $c9
        $f9 = @($s9.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-orphan' })
        if ($f9.Count -lt 1) { $failures += 'orphan: expected decision-orphan error, got none' }
        if ((Get-NextOf $s9 'MOD') -ne 'MOD-10') { $failures += "orphan: expected MOD-10, got $(Get-NextOf $s9 'MOD')" }

        # Case 10: index ID disagrees with the path it links.
        $c10 = Join-Path $tmp 'idpath'
        New-Fixture -Dir $c10 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-6.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-6.md' = $writeUp }
        $s10 = Get-IdSpace -Root $c10
        $f10 = @($s10.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'index-id-path' })
        if ($f10.Count -lt 1) { $failures += 'idpath: expected index-id-path error, got none' }

        # Case 11: checklist lines malformed in format — a ticked box and a leading space.
        # Same dropped-ID logic as index lines, mirrored onto the HANDOFF half.
        $c11 = Join-Path $tmp 'badchecklist'
        New-Fixture -Dir $c11 -HandoffContent @'
# HANDOFF

## Open items

- [x] **MOD-12 - Ticked, never deleted.** body
 - [ ] **MOD-13 - Leading space.** body
'@ -DecisionsContent @'
# DECISIONS
'@
        $s11 = Get-IdSpace -Root $c11
        $f11 = @($s11.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'checklist-unparseable' })
        if ($f11.Count -lt 2) { $failures += "badchecklist: expected 2 checklist-unparseable errors, got $($f11.Count)" }
        if ((Get-NextOf $s11 'MOD') -ne 'MOD-14') { $failures += "badchecklist: expected MOD-14, got $(Get-NextOf $s11 'MOD')" }

        # Case 12: an ID open in HANDOFF.md AND archived — IDs are never reused.
        $c12 = Join-Path $tmp 'dup'
        New-Fixture -Dir $c12 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **MOD-8 - Thing.** body
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-8.md' = $writeUp }
        $s12 = Get-IdSpace -Root $c12
        $f12 = @($s12.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'duplicate-id' })
        if ($f12.Count -lt 1) { $failures += 'dup: expected duplicate-id error, got none' }

        # Case 13: no DECISIONS.md and no docs/decisions/ is legal (a repo with no archive
        # yet) — mint from HANDOFF alone, silently.
        $c13 = Join-Path $tmp 'noarchive'
        New-Fixture -Dir $c13 -HandoffContent @'
# HANDOFF

## Open items

- [ ] **ANA-2 - Thing.** body
'@ -DecisionsContent $null
        $s13 = Get-IdSpace -Root $c13
        $e13 = @($s13.Findings | Where-Object Severity -eq 'Error')
        if ($e13.Count -ne 0) { $failures += "noarchive: expected 0 errors, got $($e13.Message -join '; ')" }
        if ((Get-NextOf $s13 'ANA') -ne 'ANA-3') { $failures += "noarchive: expected ANA-3, got $(Get-NextOf $s13 'ANA')" }

        # Case 14: missing HANDOFF.md -> files error, exit 1 (validator parity).
        $c14 = Join-Path $tmp 'nohandoff'
        New-Fixture -Dir $c14 -HandoffContent $null -DecisionsContent @'
# DECISIONS
'@
        & $script -Prefix MOD -RepoRoot $c14 *> $null
        if ($LASTEXITCODE -ne 1) { $failures += "nohandoff: expected exit 1, got $LASTEXITCODE" }

        # Case 15: an out-of-set prefix is a usage error. The law forbids repo-specific
        # prefixes, so the script must refuse to invent one.
        & $script -Prefix FOO -RepoRoot $c1 *> $null
        if ($LASTEXITCODE -ne 2) { $failures += "badprefix: expected exit 2, got $LASTEXITCODE" }

        # Case 16: -All reports every prefix; empty ones start at 1.
        $all = & $script -All -RepoRoot $c1
        if ($LASTEXITCODE -ne 0) { $failures += "all: expected exit 0, got $LASTEXITCODE" }
        $allText = $all -join "`n"
        foreach ($p in $Prefixes) {
            if ($allText -notmatch "(?m)^$p\b") { $failures += "all: no row for $p" }
        }
        if ($allText -notmatch 'MOD-9') { $failures += 'all: MOD row does not report MOD-9' }
        if ($allText -notmatch 'VAL-1') { $failures += 'all: empty VAL row does not report VAL-1' }

        # Case 17: -IdOnly emits exactly one bare line, for $(...) capture.
        $idOnly = @(& $script -Prefix MOD -RepoRoot $c1 -IdOnly)
        if ($LASTEXITCODE -ne 0) { $failures += "idonly: expected exit 0, got $LASTEXITCODE" }
        if ($idOnly.Count -ne 1 -or $idOnly[0] -ne 'MOD-9') {
            $failures += "idonly: expected exactly 'MOD-9', got $($idOnly.Count) line(s): $($idOnly -join '|')"
        }

        # Case 17b: -IdOnly over an untrustworthy archive must emit NOTHING. A caller
        # capturing stdout must never receive a mintable-looking string from a broken
        # ID space.
        $idOnlyBad = @(& $script -Prefix MOD -RepoRoot $c6 -IdOnly 2>$null)
        if ($LASTEXITCODE -ne 1) { $failures += "idonly-bad: expected exit 1, got $LASTEXITCODE" }
        if ($idOnlyBad.Count -ne 0) { $failures += "idonly-bad: expected no stdout, got: $($idOnlyBad -join '|')" }

        # Case 18: a RELATIVE -RepoRoot must behave identically to an absolute one — the
        # trap validate-workflow-docs.ps1 hit when cutting repo-relative paths.
        Push-Location $tmp
        try {
            $rel = @(& $script -Prefix MOD -RepoRoot 'basic' -IdOnly)
        } finally { Pop-Location }
        if ($rel.Count -ne 1 -or $rel[0] -ne 'MOD-9') {
            $failures += "relativeroot: expected MOD-9 via relative root, got: $($rel -join '|')"
        }

        # Case 19: supplementary prose under docs/decisions/ holds no ID — a README there
        # is not an orphan and must not block the mint.
        $c19 = Join-Path $tmp 'supplementary'
        New-Fixture -Dir $c19 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-8](docs/decisions/mod/mod-8.md)** - Thing (done, 2026-01-05)
'@ -Files @{
            'docs/decisions/mod/mod-8.md' = $writeUp
            'docs/decisions/README.md'    = "# Notes`n`nHow this directory is organised."
        }
        $s19 = Get-IdSpace -Root $c19
        $e19 = @($s19.Findings | Where-Object Severity -eq 'Error')
        if ($e19.Count -ne 0) { $failures += "supplementary: expected 0 errors, got $($e19.Message -join '; ')" }

        # Case 20: a lowercase prefix is accepted but CANONICALIZED — never echoed back as
        # `mod-9`, which is not a mintable ID. (Review finding: `-notcontains` is
        # case-insensitive, so the out-of-set guard alone let this through unnormalized.)
        $lower = @(& $script -Prefix mod -RepoRoot $c1 -IdOnly)
        if ($LASTEXITCODE -ne 0) { $failures += "lowercase-prefix: expected exit 0, got $LASTEXITCODE" }
        if ($lower.Count -ne 1 -or $lower[0] -cne 'MOD-9') {
            $failures += "lowercase-prefix: expected canonical 'MOD-9', got: $($lower -join '|')"
        }

        # Case 21: mid-migration state — a legacy header AND a write-up file already created
        # for the same ID, not yet indexed. The ID is accounted for once: the leftover
        # section is the defect, and reporting `duplicate-id` for an ID that was never
        # reused sends the maintainer chasing a phantom.
        $c21 = Join-Path $tmp 'midmigration'
        New-Fixture -Dir $c21 -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

## MOD-18/19/20 - Three items, one header (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-18.md' = $writeUp }
        $s21 = Get-IdSpace -Root $c21
        $dup21 = @($s21.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'duplicate-id' })
        if ($dup21.Count -ne 0) { $failures += "midmigration: phantom duplicate-id: $($dup21.Message -join '; ')" }
        $leg21 = @($s21.Findings | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'legacy-header' })
        if ($leg21.Count -lt 1) { $failures += 'midmigration: expected legacy-header error, got none' }
        if ((Get-NextOf $s21 'MOD') -ne 'MOD-21') { $failures += "midmigration: expected MOD-21, got $(Get-NextOf $s21 'MOD')" }

        # Case 22: -Json is parseable and carries both maxima plus the findings.
        $json = (& $script -Prefix MOD -RepoRoot $c1 -Json) -join "`n" | ConvertFrom-Json
        if ($json.prefixes[0].next -ne 'MOD-9') { $failures += "json: expected next MOD-9, got $($json.prefixes[0].next)" }
        if ($json.prefixes[0].maxOpen -ne 'MOD-4') { $failures += "json: expected maxOpen MOD-4, got $($json.prefixes[0].maxOpen)" }
        if ($json.prefixes[0].maxArchived -ne 'MOD-8') { $failures += "json: expected maxArchived MOD-8, got $($json.prefixes[0].maxArchived)" }
        if (-not $json.trustworthy) { $failures += 'json: expected trustworthy true' }
    } finally {
        if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    }

    if ($failures.Count -eq 0) {
        Write-Host 'Self-test: 22/22 cases PASS.'
        return 0
    }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-SelfTest) }

if ($Prefix -and $All) {
    Write-Host 'Use -Prefix or -All, not both.'
    exit 2
}
if ($Prefix) {
    # Normalize before validating: IDs are canonically uppercase, and every string this
    # script emits (-IdOnly, -Json, the report) must be mintable verbatim. Accepting
    # `-Prefix mod` and echoing back `mod-9` would put a non-canonical ID into HANDOFF.md.
    $Prefix = $Prefix.ToUpperInvariant()
    if ($Prefixes -cnotcontains $Prefix) {
        Write-Host "Unknown prefix '$Prefix'. The law defines exactly six: $($Prefixes -join ', ')."
        exit 2
    }
}
if ($IdOnly -and -not $Prefix) {
    Write-Host '-IdOnly needs -Prefix.'
    exit 2
}

if (-not $RepoRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ in every repo — repo root
    # is four levels up. This is what makes the mint run against the repo it is invoked in.
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "RepoRoot not found: $RepoRoot"
    exit 2
}

$space = Get-IdSpace -Root $RepoRoot
$errors = @($space.Findings | Where-Object Severity -eq 'Error')
$warnings = @($space.Findings | Where-Object Severity -eq 'Warning')
$trustworthy = ($errors.Count -eq 0)
$wanted = if ($Prefix) { @($Prefix) } else { $Prefixes }

$rows = @($wanted | ForEach-Object {
    $open = [int]($space.Open[$_] ?? 0)
    $arch = [int]($space.Archived[$_] ?? 0)
    [pscustomobject]@{
        prefix      = $_
        maxOpen     = if ($open -gt 0) { "$_-$open" } else { $null }
        maxArchived = if ($arch -gt 0) { "$_-$arch" } else { $null }
        next        = "$_-$([Math]::Max($open, $arch) + 1)"
    }
})

if ($IdOnly) {
    # Nothing on stdout when the ID space cannot be trusted: a caller capturing this must
    # never receive a mintable-looking string from a broken archive.
    if (-not $trustworthy) {
        # Findings to the error stream, never stdout: -IdOnly's stdout contract is "one bare
        # ID or nothing". -ErrorAction Continue keeps $ErrorActionPreference='Stop' from
        # turning a report line into a terminating error.
        foreach ($f in $space.Findings) {
            Write-Error ("[{0}] {1}: {2}" -f $f.Severity.ToUpper(), $f.Check, $f.Message) -ErrorAction Continue
        }
        exit 1
    }
    Write-Output $rows[0].next
    exit 0
}

if ($Json) {
    [pscustomobject]@{
        repoRoot     = $RepoRoot
        trustworthy  = $trustworthy
        prefixes     = @($rows | ForEach-Object {
            [pscustomobject]@{
                prefix      = $_.prefix
                maxOpen     = $_.maxOpen
                maxArchived = $_.maxArchived
                next        = if ($trustworthy) { $_.next } else { $null }
            }
        })
        findings     = @($space.Findings)
    } | ConvertTo-Json -Depth 5
    if ($errors.Count -gt 0) { exit 1 }
    exit 0
}

# Report goes to the output stream, not the host: this script is meant to be consumed
# (`$(... -IdOnly)`, `-Json`, a skill reading the run), and Write-Host output cannot be
# captured by any of them.
foreach ($f in $space.Findings) {
    Write-Output ("[{0}] {1}: {2}" -f $f.Severity.ToUpper(), $f.Check, $f.Message)
}

$suffix = if ($trustworthy) { '' } else { ' (UNTRUSTED - fix findings before minting)' }
if ($Prefix) {
    $r = $rows[0]
    Write-Output ("{0}: max open {1}, max archived {2} -> next {3}{4}" -f `
        $r.prefix, ($r.maxOpen ?? '-'), ($r.maxArchived ?? '-'), $r.next, $suffix)
} else {
    Write-Output ('{0,-7} {1,-9} {2,-12} {3}' -f 'Prefix', 'MaxOpen', 'MaxArchived', 'Next')
    foreach ($r in $rows) {
        Write-Output ('{0,-7} {1,-9} {2,-12} {3}{4}' -f `
            $r.prefix, ($r.maxOpen ?? '-'), ($r.maxArchived ?? '-'), $r.next, $suffix)
    }
}
Write-Output ("ID space: {0} error(s), {1} warning(s) in {2}" -f $errors.Count, $warnings.Count, $RepoRoot)

if ($errors.Count -gt 0) { exit 1 }
exit 0
