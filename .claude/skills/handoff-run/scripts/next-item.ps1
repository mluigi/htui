# next-item.ps1 — pre-filter for `/handoff-run next` (references/selection.md).
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
#   pwsh next-item.ps1 [-RepoRoot <path>] [-Json] [-SelfTest]
#
# Exit codes: 0 candidates listed (including zero open), 1 self-test failure,
#             2 unusable -RepoRoot / missing HANDOFF.md.

[CmdletBinding()]
param(
    [string]$RepoRoot,
    [switch]$Json,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'workflow-patterns.ps1')


# Section order = R3, verbatim from references/selection.md (which quotes
# workflow-docs.md). Unknown section ranks last and is flagged.
$SectionRanks = [ordered]@{
    'Next features'               = 1
    'Analyses'                    = 2
    'Deferred backlog'            = 3
    'Runtime validation findings' = 4
    'Tooling findings'            = 5
}

$KnownRepos = @('workspace', 'engine', 'vfs', 'settings', 'logging', 'VulkanTutorials')

function Get-WorkspaceRoot {
    # Sub-repo roots are direct children of the workspace named after themselves; the
    # workspace root is the one whose parent holds no HANDOFF.md of its own.
    param([string]$Root)
    $name = Split-Path -Leaf $Root
    $parent = Split-Path -Parent $Root
    if (($KnownRepos -contains $name) -and $parent -and
        (Test-Path -LiteralPath (Join-Path $parent 'HANDOFF.md'))) {
        return $parent
    }
    return $Root
}

function Resolve-RepoHandoff {
    # Repo token from a blocked-on cross-link -> that repo's HANDOFF.md path, or $null
    # when the token is 'workspace' in the workspace itself etc. Existence NOT checked
    # here — the caller distinguishes missing (unverifiable) from closed.
    param([string]$Repo, [string]$Root, [string]$WorkspaceRoot)
    if (-not $Repo -or $Repo -eq (Split-Path -Leaf $Root)) { return (Join-Path $Root 'HANDOFF.md') }
    if ($Repo -eq 'workspace') { return (Join-Path $WorkspaceRoot 'HANDOFF.md') }
    return (Join-Path (Join-Path $WorkspaceRoot $Repo) 'HANDOFF.md')
}

function Get-OpenItems {
    # HANDOFF.md -> ordered candidate records with body text attached. Body = every line
    # after the checklist line until the next checklist line (strict or loose) or a `## `
    # section header; HANDOFF bodies are indented continuations, so this needs no
    # indentation rule.
    param([string[]]$Lines)
    $items = @()
    $warnings = @()
    $section = ''
    $current = $null
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        $line = $Lines[$i]
        if ($line -match '^##\s+(?<name>.+?)\s*$') {
            $section = $Matches['name']
            $current = $null
            continue
        }
        if ($line -match $ChecklistLinePattern) {
            $id = $Matches['id']
            $title = ''
            if ($line -match "^- \[ \] \*\*$id\s*[-–—]\s*(?<t>.+?)\*\*") { $title = $Matches['t'] }
            elseif ($line -match "^- \[ \] \*\*$id\s*[-–—]\s*(?<t>.+)$") { $title = $Matches['t'] }
            $current = [pscustomobject]@{
                Id          = $id
                Title       = $title
                Section     = $section
                SectionRank = if ($SectionRanks.Contains($section)) { $SectionRanks[$section] } else { $null }
                Line        = $i + 1
                Body        = @($line)
            }
            $items += $current
            continue
        }
        if ($line -match $ChecklistLineLoosePattern) {
            # Meant as an item but unparseable — the validator errors on this; here it
            # only ends the previous body and gets a warning so the skill knows the
            # candidate set may be incomplete.
            $warnings += "line $($i + 1): checklist-unparseable (run validate-workflow-docs.ps1): $($line.Trim())"
            $current = $null
            continue
        }
        if ($null -ne $current) { $current.Body += $line }
    }
    return [pscustomobject]@{ Items = $items; Warnings = $warnings }
}

function Get-BlockedRefs {
    # Every `blocked on [repo] PREFIX-N` cross-link in the body. Freeform blockers
    # without an ID ("blocked on the probe→enable seam") carry nothing checkable — they
    # are reported as raw `blocked on` text separately. A ref inside a `Phase N (...)`
    # parenthetical is Partial: a later phase's blocker, not necessarily the item's.
    param([object]$Item)
    $refs = @()
    $repoAlt = ($KnownRepos -join '|')
    $refPattern = "blocked on\s+(?:the\s+)?(?:\*\*)?(?:(?<repo>$repoAlt)\s+)?(?:\*\*)?(?<id>$PrefixPattern-\d+)"
    $followPattern = "^[, \n\r\t]+(?:and\s+)?(?:\*\*)?(?:(?<repo>$repoAlt)\s+)?(?:\*\*)?(?<id>$PrefixPattern-\d+)"
    
    $bodyText = $Item.Body -join " "
    
    foreach ($m in [regex]::Matches($bodyText, $refPattern, 'IgnoreCase')) {
        $partial = [regex]::IsMatch(
            $bodyText.Substring(0, $m.Index),
            'Phase\s+\d+\s*\([^)]*$', 'IgnoreCase')
            
        $repo = $m.Groups['repo'].Value
        $refs += [pscustomobject]@{
            Repo    = $repo  # '' = unqualified, resolved locally
            Id      = $m.Groups['id'].Value
            Partial = $partial
            Text    = $m.Value.Trim()
        }
        
        $idx = $m.Index + $m.Length
        $remaining = $bodyText.Substring($idx)
        $f_match = [regex]::Match($remaining, $followPattern, 'IgnoreCase')
        while ($f_match.Success) {
            $f_repo = $f_match.Groups['repo'].Value
            if (-not $f_repo) { $f_repo = $repo } else { $repo = $f_repo }
            
            # Use the same partial status for the whole list
            $refs += [pscustomobject]@{
                Repo    = $f_repo
                Id      = $f_match.Groups['id'].Value
                Partial = $partial
                Text    = $f_match.Value.Trim()
            }
            $idx += $f_match.Length
            $remaining = $bodyText.Substring($idx)
            $f_match = [regex]::Match($remaining, $followPattern, 'IgnoreCase')
        }
    }
    return $refs
}

function Get-InFlightSignals {
    param([object]$Item, [string]$Root)
    $bodyText = $Item.Body -join ' '
    $idLower = $Item.Id.ToLowerInvariant()
    $signals = @()
    if ($bodyText -match 'Phase\s+\d+[^.]{0,40}\blanded\b') { $signals += 'phase-landed' }
    foreach ($dir in @('.claude/plans', '.claude/prds')) {
        $d = Join-Path $Root $dir
        if ((Test-Path $d) -and @(Get-ChildItem -Path $d -Filter "$idLower-*" -File -ErrorAction SilentlyContinue).Count -gt 0) {
            $signals += (Split-Path -Leaf $dir).TrimEnd('s')  # 'plan' | 'prd'
        }
    }
    return $signals
}

function Get-RemainingNote {
    # E2 signal only — the anchor judgment stays with the skill.
    param([object]$Item)
    foreach ($line in $Item.Body) {
        if ($line -match '\*\*Remaining:?\*\*\s*(?<rest>.*)') { return $line.Trim() }
    }
    return $null
}

function Build-Candidates {
    param([string]$Root)
    $handoffPath = Join-Path $Root 'HANDOFF.md'
    if (-not (Test-Path -LiteralPath $handoffPath)) {
        throw "HANDOFF.md not found at $handoffPath"
    }
    $workspaceRoot = Get-WorkspaceRoot -Root $Root
    $parsed = Get-OpenItems -Lines @(Get-Content -LiteralPath $handoffPath -Encoding UTF8)
    $items = @($parsed.Items)
    $openIds = @($items | ForEach-Object Id)

    # Other repos' open-ID sets, read lazily and cached; $null = unreadable.
    $handoffCache = @{}
    foreach ($item in $items) {
        $refs = @(Get-BlockedRefs -Item $item)
        $blockers = @()
        foreach ($ref in $refs) {
            $path = Resolve-RepoHandoff -Repo $ref.Repo -Root $Root -WorkspaceRoot $workspaceRoot
            if (-not $handoffCache.ContainsKey($path)) {
                $handoffCache[$path] = if (Test-Path -LiteralPath $path) {
                    @((Get-OpenItems -Lines @(Get-Content -LiteralPath $path -Encoding UTF8)).Items | ForEach-Object Id)
                } else { $null }
            }
            $openSet = $handoffCache[$path]
            $status = if ($null -eq $openSet) { 'unverifiable' }
                      elseif ($openSet -contains $ref.Id) { 'open' }
                      else { 'not-open' }
            $blockers += [pscustomobject]@{
                Repo    = if ($ref.Repo) { $ref.Repo } else { 'local' }
                Id      = $ref.Id
                Status  = $status
                Partial = $ref.Partial
                Text    = $ref.Text
            }
        }

        # E1 verdict: a confirmed-open, non-partial blocker drops; an unverifiable
        # non-partial blocker keeps the item as `blocked?`; everything else is eligible
        # machine-side (partial blockers and E2 anchors are the skill's call).
        $hardOpen = @($blockers | Where-Object { $_.Status -eq 'open' -and -not $_.Partial })
        $unverif = @($blockers | Where-Object { $_.Status -eq 'unverifiable' -and -not $_.Partial })
        $eligibility = if ($hardOpen.Count -gt 0) { 'blocked' }
                       elseif ($unverif.Count -gt 0) { 'blocked?' }
                       else { 'eligible' }

        $item | Add-Member -NotePropertyMembers @{
            Blockers    = $blockers
            Eligibility = $eligibility
            InFlight    = @(Get-InFlightSignals -Item $item -Root $Root)
            Remaining   = Get-RemainingNote -Item $item
            Dependents  = 0
        }
    }

    # R2: open local items whose body names `blocked on ... <this ID>`.
    foreach ($item in $items) {
        $item.Dependents = @($items | Where-Object {
            $_.Id -ne $item.Id -and
            @($_.Blockers | Where-Object { $_.Repo -eq 'local' -and $_.Id -eq $item.Id }).Count -gt 0
        }).Count
    }

    return [pscustomobject]@{
        Root       = $Root
        Items      = $items
        Warnings   = @($parsed.Warnings)
        OpenIds    = $openIds
    }
}

function Rank-Candidates {
    # R1 in-flight, R2 dependents, R3 section, R4 file order — and name the key that
    # separated #1 from #2. R4-only separation is a tie the skill must ask about.
    # `blocked?` items rank alongside eligible ones (selection.md step 4: an unverifiable
    # blocker on the would-be winner is an ambiguity, not a drop).
    param([object[]]$Items)
    $rankable = @($Items | Where-Object { $_.Eligibility -ne 'blocked' })
    $sorted = @($rankable | Sort-Object `
        @{ Expression = { if ($_.InFlight.Count -gt 0) { 0 } else { 1 } } },
        @{ Expression = { -$_.Dependents } },
        @{ Expression = { $_.SectionRank ?? [int]::MaxValue } },
        @{ Expression = { $_.Line } })
    $decidedBy = $null
    $tie = $false
    if ($sorted.Count -ge 2) {
        $a = $sorted[0]; $b = $sorted[1]
        $decidedBy =
            if (($a.InFlight.Count -gt 0) -ne ($b.InFlight.Count -gt 0)) { 'R1 in-flight' }
            elseif ($a.Dependents -ne $b.Dependents) { 'R2 unblocks-others' }
            elseif (($a.SectionRank ?? [int]::MaxValue) -ne ($b.SectionRank ?? [int]::MaxValue)) { 'R3 section-order' }
            else { $tie = $true; 'R4 file-order — TIE, ask the maintainer' }
    } elseif ($sorted.Count -eq 1) {
        $decidedBy = 'only eligible candidate'
    }
    return [pscustomobject]@{ Ranked = $sorted; DecidedBy = $decidedBy; Tie = $tie }
}

function Invoke-NextItem {
    param([string]$Root, [switch]$AsJson)
    $data = Build-Candidates -Root $Root
    $ranking = Rank-Candidates -Items $data.Items

    $result = [pscustomobject]@{
        repoRoot   = $data.Root
        open       = $data.Items.Count
        rankable   = $ranking.Ranked.Count
        decidedBy  = $ranking.DecidedBy
        tie        = $ranking.Tie
        warnings   = $data.Warnings
        candidates = @($ranking.Ranked | ForEach-Object {
            [pscustomobject]@{
                id          = $_.Id
                title       = $_.Title
                section     = $_.Section
                line        = $_.Line
                eligibility = $_.Eligibility
                inFlight    = $_.InFlight
                dependents  = $_.Dependents
                remaining   = $_.Remaining
                blockers    = @($_.Blockers | ForEach-Object { "$($_.Repo) $($_.Id): $($_.Status)$(if ($_.Partial) { ' (phase-scoped)' })" })
            }
        })
        ineligible = @($data.Items | Where-Object { $_.Eligibility -eq 'blocked' } | ForEach-Object {
            $why = @($_.Blockers | Where-Object { $_.Status -eq 'open' -and -not $_.Partial } |
                ForEach-Object { "blocked on $($_.Repo) $($_.Id)" }) -join '; '
            [pscustomobject]@{ id = $_.Id; title = $_.Title; why = $why }
        })
    }

    if ($AsJson) {
        $result | ConvertTo-Json -Depth 5
        return
    }
    Write-Host "next-item pre-filter — $($result.open) open, $($result.rankable) rankable in $($result.repoRoot)"
    foreach ($w in $result.warnings) { Write-Host "  [WARN] $w" }
    $pos = 0
    foreach ($c in $result.candidates) {
        $pos++
        $flags = @()
        if ($c.eligibility -eq 'blocked?') { $flags += 'blocked?' }
        if ($c.inFlight.Count -gt 0) { $flags += "in-flight: $($c.inFlight -join ',')" }
        if ($c.dependents -gt 0) { $flags += "unblocks $($c.dependents)" }
        if ($c.remaining) { $flags += 'E2? has Remaining note' }
        $flagText = if ($flags.Count -gt 0) { ' [' + ($flags -join '; ') + ']' } else { '' }
        Write-Host ("  {0}. {1} ({2}, L{3}){4}" -f $pos, $c.id, $c.section, $c.line, $flagText)
        foreach ($b in $c.blockers) { Write-Host "       blocker: $b" }
    }
    foreach ($i in $result.ineligible) { Write-Host "  ineligible: $($i.id) — $($i.why)" }
    if ($result.decidedBy) { Write-Host "  top decided by: $($result.decidedBy)" }
    if ($result.tie) { Write-Host '  TIE — selection.md step 4: ask, never pick by file order.' }
}

function Invoke-SelfTest {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("next-item-selftest-" + [guid]::NewGuid().ToString('N'))
    $failures = @()
    try {
        # Workspace fixture with two sub-repos; script under test targets 'engine'.
        $ws = Join-Path $tmp 'ws'
        $eng = Join-Path $ws 'engine'
        $vfs = Join-Path $ws 'vfs'
        New-Item -ItemType Directory -Path $eng, $vfs -Force | Out-Null
        Set-Content -Path (Join-Path $ws 'HANDOFF.md') -Encoding UTF8 -Value @'
# HANDOFF

## Next features

- [ ] **MOD-1 - Workspace thing.** body
'@
        Set-Content -Path (Join-Path $vfs 'HANDOFF.md') -Encoding UTF8 -Value @'
# HANDOFF

## Next features

- [ ] **MOD-2 - Open vfs item.** body
'@
        Set-Content -Path (Join-Path $eng 'HANDOFF.md') -Encoding UTF8 -Value @'
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
'@
        $data = Build-Candidates -Root $eng
        $r = Rank-Candidates -Items $data.Items
        $byId = @{}; foreach ($i in $data.Items) { $byId[$i.Id] = $i }

        if ($data.Items.Count -ne 11) { $failures += "parse: expected 11 open items, got $($data.Items.Count)" }
        if ($byId['MOD-7'].Eligibility -ne 'blocked') { $failures += "E1-open: MOD-7 expected blocked, got $($byId['MOD-7'].Eligibility)" }
        if ($byId['MOD-8'].Eligibility -ne 'eligible') { $failures += "E1-closed: MOD-8 expected eligible, got $($byId['MOD-8'].Eligibility)" }
        if ($byId['MOD-9'].Eligibility -ne 'blocked?') { $failures += "E1-unverifiable: MOD-9 expected blocked?, got $($byId['MOD-9'].Eligibility)" }
        if ($byId['MOD-12'].Eligibility -ne 'eligible') { $failures += "phase-scoped: MOD-12 expected eligible, got $($byId['MOD-12'].Eligibility)" }
        if (@($byId['MOD-12'].Blockers | Where-Object Partial).Count -lt 1) { $failures += 'phase-scoped: MOD-12 blocker not marked Partial' }
        if ($byId['MOD-11'].Dependents -ne 3) { $failures += "R2: MOD-11 expected 3 dependents, got $($byId['MOD-11'].Dependents)" }
        if ($byId['MOD-14'].Dependents -ne 1) { $failures += "R2: MOD-14 expected 1 dependent, got $($byId['MOD-14'].Dependents)" }
        if ($byId['MOD-15'].Dependents -ne 1) { $failures += "R2: MOD-15 expected 1 dependent, got $($byId['MOD-15'].Dependents)" }
        if ($byId['CLEAN-1'].SectionRank -ne 3) { $failures += "R3: CLEAN-1 expected section rank 3, got $($byId['CLEAN-1'].SectionRank)" }
        if ($r.Ranked[0].Id -ne 'MOD-11') { $failures += "rank: expected MOD-11 first (R2), got $($r.Ranked[0].Id)" }
        if ($r.DecidedBy -ne 'R2 unblocks-others') { $failures += "decidedBy: expected R2, got $($r.DecidedBy)" }
        if (@($data.Warnings).Count -lt 1) { $failures += 'loose: expected a checklist-unparseable warning for CLEAN-2' }

        # R1 beats R2: give ANA-3 a referenced plan artifact, it must outrank MOD-11.
        New-Item -ItemType Directory -Path (Join-Path $eng '.claude/plans') -Force | Out-Null
        Set-Content -Path (Join-Path $eng '.claude/plans/ana-3-question.plan.md') -Value 'plan' -Encoding UTF8
        $data2 = Build-Candidates -Root $eng
        $r2 = Rank-Candidates -Items $data2.Items
        if ($r2.Ranked[0].Id -ne 'ANA-3') { $failures += "R1: expected in-flight ANA-3 first, got $($r2.Ranked[0].Id)" }
        if ($r2.DecidedBy -ne 'R1 in-flight') { $failures += "R1 decidedBy: got $($r2.DecidedBy)" }

        # Tie: two plain same-section items and nothing else.
        $tieRepo = Join-Path $tmp 'tie'
        New-Item -ItemType Directory -Path $tieRepo -Force | Out-Null
        Set-Content -Path (Join-Path $tieRepo 'HANDOFF.md') -Encoding UTF8 -Value @'
# HANDOFF

## Next features

- [ ] **MOD-1 - First.** body
- [ ] **MOD-2 - Second.** body
'@
        $r3 = Rank-Candidates -Items @((Build-Candidates -Root $tieRepo).Items)
        if (-not $r3.Tie) { $failures += 'tie: expected tie=true for two plain items' }
    } finally {
        if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    }

    if ($failures.Count -eq 0) {
        Write-Host 'Self-test: 17/17 assertions PASS.'
        return 0
    }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-SelfTest) }

if (-not $RepoRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ — repo root is four
    # levels up (same convention as the validator).
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "RepoRoot not found: $RepoRoot"
    exit 2
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path

try {
    Invoke-NextItem -Root $RepoRoot -AsJson:$Json
} catch {
    Write-Host $_.Exception.Message
    exit 2
}
exit 0
