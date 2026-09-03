# validate-workflow-docs.ps1 — structural checks for HANDOFF.md / DECISIONS.md
# per .claude/rules/workflow-docs.md. Lists findings, never auto-fixes.
#
# Usage:
#   pwsh validate-workflow-docs.ps1 [-RepoRoot <path>] [-Strict] [-SelfTest]
#                                   [-ScopePaths <newline-separated paths>]
#
# Checks always cover the whole repo. -ScopePaths only narrows what is allowed to FAIL
# the run: errors attributed to a file outside the scope list are printed as [WARNING]
# "(pre-existing...)" and do not affect the exit code. The pre-commit hook passes the
# staged paths so a commit is blocked by findings in the files it touches, never by
# unrelated pre-existing state elsewhere in the repo; a plain run (no -ScopePaths, as
# /handoff-run close-out does it) is unchanged and still fails on any error anywhere.
# A finding with no file attribution is always in scope (fail-closed).
#
# Exit codes: 0 clean (warnings allowed unless -Strict), 1 findings (including a missing
#           HANDOFF.md), 2 unusable -RepoRoot.
# Errors:   summary-table count mismatch, duplicate/reused IDs, index not
#           reverse-chronological, index line unparseable or linking a missing file
#           (decision-index), write-up file with no index line (decision-orphan), index
#           ID disagreeing with its path (decision-id-path), legacy `## PREFIX-N`
#           write-up section still present (decision-legacy), open checklist line meant as
#           an item but failing the strict pattern (checklist-unparseable, mirrors
#           next-item-id.ps1).
# Warnings: status-line recap over cap (~3), broken cross-link paths, unparseable summary
#           rows, undated archive entries.

[CmdletBinding()]
param(
    [string]$RepoRoot,
    [switch]$Strict,
    [switch]$SelfTest,
    # Repeatable/multiline; passing it at all activates the scope, so an empty value means
    # "nothing staged that this validator owns" -> nothing may block, never a silent
    # fallback to whole-repo blocking.
    [string[]]$ScopePaths
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'workflow-patterns.ps1')


function New-Finding {
    # $Files: the repo-relative path(s) the finding is about. Drives -ScopePaths only; an
    # empty list means "unattributed", which counts as in scope so a new check can never
    # silently stop blocking by forgetting its attribution.
    param([string]$Severity, [string]$Check, [string]$Message, [string[]]$Files = @())
    [pscustomobject]@{ Severity = $Severity; Check = $Check; Message = $Message; Files = @($Files) }
}

function Get-ScopeSet {
    param([string[]]$Raw)
    $set = @{}
    foreach ($chunk in @($Raw)) {
        if ($null -eq $chunk) { continue }
        foreach ($line in ($chunk -split "`r?`n")) {
            $path = $line.Trim()
            if (-not $path) { continue }
            if ($path.StartsWith('./')) { $path = $path.Substring(2) }
            $set[$path] = $true
        }
    }
    return $set
}

function Test-FindingInScope {
    param([object]$Finding, [hashtable]$ScopeSet, [bool]$ScopeActive)
    if (-not $ScopeActive) { return $true }
    if (@($Finding.Files).Count -eq 0) { return $true }
    foreach ($f in $Finding.Files) { if ($ScopeSet.ContainsKey($f)) { return $true } }
    return $false
}

function Get-FindingReport {
    # Splits findings into what may fail the run and what is only reported. Returns Lines
    # (display lines, in finding order), BlockingErrors, DemotedErrors (errors outside the
    # scope, printed as warnings), ScopedWarnings (warnings inside the scope - the only
    # ones -Strict acts on).
    param([object[]]$Findings, [hashtable]$ScopeSet, [bool]$ScopeActive)
    $lines = @()
    $blocking = 0
    $demoted = 0
    $scopedWarnings = 0
    foreach ($f in $Findings) {
        if (Test-FindingInScope -Finding $f -ScopeSet $ScopeSet -ScopeActive $ScopeActive) {
            $lines += ("[{0}] {1}: {2}" -f $f.Severity.ToUpper(), $f.Check, $f.Message)
            if ($f.Severity -eq 'Error') { $blocking++ } else { $scopedWarnings++ }
        } elseif ($f.Severity -eq 'Error') {
            # Loud, every run, but not this commit's fault - still fails a plain run.
            $lines += ("[WARNING] {0}: {1} (pre-existing, outside this commit's staged files)" -f $f.Check, $f.Message)
            $demoted++
        } else {
            $lines += ("[WARNING] {0}: {1}" -f $f.Check, $f.Message)
        }
    }
    [pscustomobject]@{
        Lines = $lines; BlockingErrors = $blocking; DemotedErrors = $demoted
        ScopedWarnings = $scopedWarnings
    }
}

function Get-OpenItemIds {
    param([string[]]$HandoffLines)
    $ids = @()
    foreach ($line in $HandoffLines) {
        if ($line -match $ChecklistLinePattern) { $ids += $Matches['id'] }
    }
    return $ids
}

function Test-ChecklistLines {
    # HANDOFF counterpart of Test-DecisionIndex's loose rescan. Check name matches the
    # minter's (`checklist-unparseable`, not the validator's area-based naming) so the two
    # tools report the same defect under the same label. The at-risk ID is deliberately
    # NOT added to the open-ID count: the validator computes no next ID, and counting a
    # ticked line as open would only stack a summary-table error on an already-red repo.
    param([string[]]$HandoffLines)
    $findings = @()
    foreach ($line in $HandoffLines) {
        if ($line -match $ChecklistLineLoosePattern) {
            $looseId = $Matches['id']
            if ($line -notmatch $ChecklistLinePattern) {
                $findings += New-Finding 'Error' 'checklist-unparseable' `
                    "${looseId}: open-item line unparseable - ID space untrustworthy until fixed: $($line.Trim())" `
                    @('HANDOFF.md')
            }
        }
    }
    return $findings
}

function Get-DecisionHeaders {
    param([string[]]$DecisionsLines)
    $headers = @()
    foreach ($line in $DecisionsLines) {
        $ids = @(Get-LegacyHeaderIds -Line $line)
        if ($ids.Count -gt 0) {
            $date = $null
            if ($line -match '(\d{4}-\d{2}-\d{2})') { $date = $Matches[1] }
            foreach ($id in $ids) {
                $headers += [pscustomobject]@{ Id = $id; Date = $date; Line = $line.Trim() }
            }
        }
    }
    return $headers
}

function Get-DecisionIndex {
    # Well-formed index lines only. Malformed ones are surfaced by Test-DecisionIndex,
    # which rescans the raw lines — they must never be silently skipped.
    param([string[]]$DecisionsLines)
    $entries = @()
    foreach ($line in $DecisionsLines) {
        if ($line -match $IndexLinePattern) {
            $entries += [pscustomobject]@{
                Id     = $Matches['id']
                Path   = $Matches['path']
                Title  = $Matches['title']
                Status = $Matches['status']
                Date   = $Matches['date']
                Line   = $line.Trim()
            }
        }
    }
    return $entries
}

function Get-ArchiveEntries {
    # One ordered scan producing the union of legacy `## PREFIX-N` headers and index
    # lines, tagged and in file order. Order matters: duplicate-id and reverse-chron run
    # over this, and concatenating two separately-built lists would not preserve true
    # position across the boundary between an index block and a legacy block.
    param([string[]]$DecisionsLines)
    $entries = @()
    foreach ($line in $DecisionsLines) {
        if ($line -match $IndexLinePattern) {
            $entries += [pscustomobject]@{
                Id = $Matches['id']; Date = $Matches['date']; Line = $line.Trim(); Kind = 'Index'
            }
        } else {
            $ids = @(Get-LegacyHeaderIds -Line $line)
            if ($ids.Count -gt 0) {
                $date = $null
                if ($line -match '(\d{4}-\d{2}-\d{2})') { $date = $Matches[1] }
                foreach ($id in $ids) {
                    $entries += [pscustomobject]@{
                        Id = $id; Date = $date; Line = $line.Trim(); Kind = 'Header'
                    }
                }
            }
        }
    }
    return $entries
}

function Get-DecisionFiles {
    # Absent docs/decisions/ is legal (a repo that has not migrated yet) — return empty,
    # silently. Throwing here would crash every legacy repo with exit 2.
    param([string]$Root)
    $dir = Join-Path $Root 'docs/decisions'
    if (-not (Test-Path -LiteralPath $dir)) { return @() }
    # Resolve first: Get-ChildItem yields absolute FullNames, so the relative path must be
    # cut using the ABSOLUTE root length. Chopping by the length of a caller-supplied
    # relative root (`-RepoRoot engine`) silently mangles every path, and every write-up
    # then reports as orphaned.
    $prefixLen = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\', '/').Length + 1
    return @(Get-ChildItem -Recurse -File -Filter '*.md' -LiteralPath $dir | ForEach-Object {
        [pscustomobject]@{
            Path      = $_.FullName.Substring($prefixLen).Replace('\', '/')
            FullPath  = $_.FullName
            # An item write-up is named <prefix>-N.md. Anything else under docs/decisions/
            # is supplementary prose (a README, notes) — it holds no ID, so it is not
            # orphaned by having no index line.
            IsItemFile = ($_.Name -match '^[a-z]+-\d+\.md$')
        }
    })
}

function Test-DecisionIndex {
    param(
        [string]$Root,
        [string[]]$DecisionsLines,
        [object[]]$IndexEntries,
        [object[]]$DecisionFiles,
        [object[]]$DecisionHeaders
    )
    $findings = @()

    foreach ($line in $DecisionsLines) {
        if ($line -match $IndexLineLoosePattern) {
            $looseId = $Matches['id']
            if ($line -notmatch $IndexLinePattern) {
                # Name the at-risk ID: duplicate-id can only compare IDs it can see, so it
                # is structurally unable to catch a line that failed to parse.
                $findings += New-Finding 'Error' 'decision-index' `
                    "${looseId}: index line unparseable — ID space untrustworthy until fixed: $($line.Trim())" `
                    @('DECISIONS.md')
            }
        }
    }

    foreach ($e in $IndexEntries) {
        $full = Join-Path $Root $e.Path
        if (-not (Test-Path -LiteralPath $full)) {
            # Attributed to the linked path too: staging the deletion of a write-up must
            # block even when DECISIONS.md itself is untouched.
            $findings += New-Finding 'Error' 'decision-index' `
                "$($e.Id): index line links $($e.Path), which does not exist." `
                @('DECISIONS.md', $e.Path)
        }
        $prefix = ($e.Id -split '-')[0].ToLowerInvariant()
        $expected = "docs/decisions/$prefix/$($e.Id.ToLowerInvariant()).md"
        if ($e.Path -cne $expected) {
            $findings += New-Finding 'Error' 'decision-id-path' `
                "$($e.Id): index line links $($e.Path), expected $expected." `
                @('DECISIONS.md', $e.Path)
        }
    }

    $indexedPaths = @($IndexEntries | ForEach-Object Path)
    foreach ($file in ($DecisionFiles | Where-Object IsItemFile)) {
        if ($indexedPaths -notcontains $file.Path) {
            $findings += New-Finding 'Error' 'decision-orphan' `
                "$($file.Path) has no index line in DECISIONS.md." `
                @('DECISIONS.md', $file.Path)
        }
    }

    if ($DecisionHeaders.Count -gt 0) {
        # Error since the workspace-wide migration completed (MOD-7). It was a Warning
        # only while repos were mid-migration; every repo is now on the per-item layout.
        $findings += New-Finding 'Error' 'decision-legacy' `
            ("DECISIONS.md holds $($DecisionHeaders.Count) legacy '## PREFIX-N' write-up section(s); " +
             'the write-up belongs at docs/decisions/<prefix>/<prefix>-N.md with an index line here.') `
            @('DECISIONS.md')
    }

    return $findings
}

function Test-SummaryTable {
    param([string[]]$HandoffLines, [string[]]$OpenIds)
    $findings = @()
    $openCounts = @{}
    foreach ($id in $OpenIds) {
        $prefix = ($id -split '-')[0]
        $openCounts[$prefix] = 1 + ($openCounts[$prefix] ?? 0)
    }
    $rowPrefixes = @()
    foreach ($line in $HandoffLines) {
        if ($line -match "^\|\s*($PrefixPattern)-N\s*\|\s*([^|]*)") {
            $prefix = $Matches[1]
            $cell = $Matches[2]
            $rowPrefixes += $prefix
            if ($cell -match '(\d+)') {
                $tableCount = [int]$Matches[1]
                $actual = [int]($openCounts[$prefix] ?? 0)
                if ($tableCount -ne $actual) {
                    $findings += New-Finding 'Error' 'summary-table' `
                        "Summary row $prefix-N says $tableCount open, HANDOFF checklist has $actual." `
                        @('HANDOFF.md')
                }
            } else {
                $findings += New-Finding 'Warning' 'summary-table' `
                    "Summary row $prefix-N has no parseable count: '$($cell.Trim())'." `
                    @('HANDOFF.md')
            }
        }
    }
    foreach ($prefix in $openCounts.Keys) {
        if ($rowPrefixes -notcontains $prefix) {
            $findings += New-Finding 'Warning' 'summary-table' `
                "Open $prefix items exist but summary table has no $prefix-N row." `
                @('HANDOFF.md')
        }
    }
    return $findings
}

function Test-DuplicateIds {
    # $ArchivedIds is the union of legacy header IDs and index-line IDs, so this works
    # unchanged whether a repo is pre-migration, migrated, or mid-migration.
    param([string[]]$OpenIds, [string[]]$ArchivedIds)
    $findings = @()
    $openDups = $OpenIds | Group-Object | Where-Object Count -gt 1
    foreach ($g in $openDups) {
        $findings += New-Finding 'Error' 'duplicate-id' "ID $($g.Name) open $($g.Count)x in HANDOFF.md." `
            @('HANDOFF.md')
    }
    $archDups = $ArchivedIds | Group-Object | Where-Object Count -gt 1
    foreach ($g in $archDups) {
        $findings += New-Finding 'Error' 'duplicate-id' "ID $($g.Name) archived $($g.Count)x in DECISIONS.md." `
            @('DECISIONS.md')
    }
    foreach ($id in ($OpenIds | Select-Object -Unique)) {
        if ($ArchivedIds -contains $id) {
            $findings += New-Finding 'Error' 'duplicate-id' `
                "ID $id is open in HANDOFF.md AND archived in DECISIONS.md (IDs are never reused)." `
                @('HANDOFF.md', 'DECISIONS.md')
        }
    }
    return $findings
}

function Test-ReverseChronology {
    param([object[]]$DecisionHeaders)
    $findings = @()
    $prevDate = $null
    $prevId = $null
    foreach ($h in $DecisionHeaders) {
        if (-not $h.Date) {
            $findings += New-Finding 'Warning' 'reverse-chron' "No date parseable in header: $($h.Line)" `
                @('DECISIONS.md')
            continue
        }
        if ($prevDate -and ($h.Date -gt $prevDate)) {
            $findings += New-Finding 'Error' 'reverse-chron' `
                "DECISIONS.md not reverse-chronological: $($h.Id) ($($h.Date)) sits below $prevId ($prevDate)." `
                @('DECISIONS.md')
        }
        $prevDate = $h.Date
        $prevId = $h.Id
    }
    return $findings
}

function Test-StatusLineCap {
    param([string[]]$HandoffLines)
    $findings = @()
    $start = -1
    for ($i = 0; $i -lt $HandoffLines.Count; $i++) {
        if ($HandoffLines[$i] -match '^\*\*Current status') { $start = $i; break }
    }
    if ($start -lt 0) { return $findings }
    $paragraph = ''
    for ($i = $start; $i -lt $HandoffLines.Count -and $HandoffLines[$i].Trim() -ne ''; $i++) {
        $paragraph += $HandoffLines[$i] + ' '
    }
    $completionPattern = "$PrefixPattern-\d+\s+(?:\*\*)?(?:done|closed|concluded|complete)"
    $mentions = [regex]::Matches($paragraph, $completionPattern, 'IgnoreCase')
    if ($mentions.Count -gt 3) {
        $findings += New-Finding 'Warning' 'status-line' `
            "Status line recaps $($mentions.Count) completions; rule caps it at ~2-3 (drop oldest)." `
            @('HANDOFF.md')
    }
    return $findings
}

function Test-CrossLinks {
    # A backticked `*.md` mention resolves against the containing file's directory OR the
    # repo root — this corpus writes both: true relative links ("../HANDOFF.md") and
    # root-relative conventions ("docs/ANA-2.md") that stay root-relative no matter which
    # file they appear in. Only a path that satisfies neither is a finding.
    param([string]$FileDir, [string[]]$Lines, [string]$FileLabel, [string]$RootDir)
    $findings = @()
    $seen = @{}
    $bases = @($FileDir)
    if ($RootDir -and $RootDir -ne $FileDir) { $bases += $RootDir }
    foreach ($line in $Lines) {
        foreach ($m in [regex]::Matches($line, '`([^`\r\n]+?\.md)`')) {
            $path = $m.Groups[1].Value
            # Skip URLs, globs, and `<placeholder>` path templates.
            if ($path -match '://' -or $path -match '[*?<>]' -or $seen.ContainsKey($path)) { continue }
            $seen[$path] = $true
            $found = $false
            foreach ($base in $bases) {
                try {
                    $full = [System.IO.Path]::GetFullPath((Join-Path $base $path))
                } catch { continue }
                if (Test-Path -LiteralPath $full) { $found = $true; break }
            }
            if (-not $found) {
                $findings += New-Finding 'Warning' 'cross-link' "${FileLabel}: linked path not found: $path" `
                    @($FileLabel)
            }
        }
    }
    return $findings
}

function Invoke-WorkflowDocsChecks {
    param([string]$Root)
    $handoffPath = Join-Path $Root 'HANDOFF.md'
    $decisionsPath = Join-Path $Root 'DECISIONS.md'
    $findings = @()

    if (-not (Test-Path -LiteralPath $handoffPath)) {
        return @(New-Finding 'Error' 'files' "HANDOFF.md not found at $handoffPath" @('HANDOFF.md'))
    }
    $handoffLines = @(Get-Content -LiteralPath $handoffPath -Encoding UTF8)
    $decisionsLines = @()
    if (Test-Path -LiteralPath $decisionsPath) {
        $decisionsLines = @(Get-Content -LiteralPath $decisionsPath -Encoding UTF8)
    } else {
        $findings += New-Finding 'Warning' 'files' "DECISIONS.md not found at $decisionsPath (skipping archive checks)." `
            @('DECISIONS.md')
    }

    $openIds = @(Get-OpenItemIds $handoffLines)
    $headers = @(Get-DecisionHeaders $decisionsLines)
    $indexEntries = @(Get-DecisionIndex $decisionsLines)
    $archiveEntries = @(Get-ArchiveEntries $decisionsLines)
    $decisionFiles = @(Get-DecisionFiles -Root $Root)

    $findings += Test-ChecklistLines -HandoffLines $handoffLines
    $findings += Test-SummaryTable -HandoffLines $handoffLines -OpenIds $openIds
    $findings += Test-DuplicateIds -OpenIds $openIds -ArchivedIds @($archiveEntries | ForEach-Object Id)
    $findings += Test-ReverseChronology -DecisionHeaders $archiveEntries
    $findings += Test-DecisionIndex -Root $Root -DecisionsLines $decisionsLines `
        -IndexEntries $indexEntries -DecisionFiles $decisionFiles -DecisionHeaders $headers
    $findings += Test-StatusLineCap -HandoffLines $handoffLines
    $findings += Test-CrossLinks -FileDir $Root -Lines $handoffLines -FileLabel 'HANDOFF.md'
    $findings += Test-CrossLinks -FileDir $Root -Lines $decisionsLines -FileLabel 'DECISIONS.md'
    # Each item file's relative links resolve against its own directory, not the repo root.
    foreach ($file in $decisionFiles) {
        $findings += Test-CrossLinks -FileDir (Split-Path $file.FullPath) -RootDir $Root `
            -Lines @(Get-Content -LiteralPath $file.FullPath -Encoding UTF8) -FileLabel $file.Path
    }
    return $findings
}

function New-DecisionFixture {
    # Builds a fixture repo: HANDOFF.md, DECISIONS.md, and any docs/decisions item files.
    # $Files maps repo-relative path -> content.
    param(
        [string]$Dir,
        [string]$HandoffContent,
        [string]$DecisionsContent,
        [hashtable]$Files = @{}
    )
    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    Set-Content -Path (Join-Path $Dir 'HANDOFF.md') -Value $HandoffContent -Encoding UTF8
    Set-Content -Path (Join-Path $Dir 'DECISIONS.md') -Value $DecisionsContent -Encoding UTF8
    foreach ($rel in $Files.Keys) {
        $full = Join-Path $Dir $rel
        New-Item -ItemType Directory -Path (Split-Path $full) -Force | Out-Null
        Set-Content -Path $full -Value $Files[$rel] -Encoding UTF8
    }
}

function Invoke-SelfTest {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("wfdocs-selftest-" + [guid]::NewGuid().ToString('N'))
    $failures = @()

    # Shared fixture pieces for the per-item-layout cases. No open items and no summary
    # rows, so a clean fixture yields zero findings of any kind.
    $emptyHandoff = @'
# HANDOFF

**Current status (2026-01-05):** nothing open.

## Open items

## Summary

| Area  | Open |
|-------|------|
'@
    $writeUp = @'
# MOD-7 - Thing (done, 2026-01-05)

Write-up body.
'@

    try {
        # Case 1: clean pair -> no errors expected.
        $clean = Join-Path $tmp 'clean'; New-Item -ItemType Directory -Path $clean -Force | Out-Null
        @'
# HANDOFF

**Current status (2026-01-02):** ANA-1 done.

## Open items

- [ ] **MOD-1 - Thing.** body `DECISIONS.md`

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
| ANA-N | 0    |
'@ | Set-Content -Path (Join-Path $clean 'HANDOFF.md') -Encoding UTF8
        @'
# DECISIONS

- **[ANA-1](docs/decisions/ana/ana-1.md)** - Old analysis (done, 2026-01-02)
'@ | Set-Content -Path (Join-Path $clean 'DECISIONS.md') -Encoding UTF8
        $cleanItem = Join-Path $clean 'docs/decisions/ana'
        New-Item -ItemType Directory -Path $cleanItem -Force | Out-Null
        @'
# ANA-1 - Old analysis (done, 2026-01-02)

Write-up.
'@ | Set-Content -Path (Join-Path $cleanItem 'ana-1.md') -Encoding UTF8
        $f = @(Invoke-WorkflowDocsChecks -Root $clean | Where-Object Severity -eq 'Error')
        if ($f.Count -ne 0) { $failures += "clean: expected 0 errors, got $($f.Count): $($f.Message -join '; ')" }

        # Case 2: summary count wrong -> summary-table error expected.
        $bad = Join-Path $tmp 'badcount'; New-Item -ItemType Directory -Path $bad -Force | Out-Null
        (Get-Content (Join-Path $clean 'HANDOFF.md') -Raw -Encoding UTF8) -replace '\| MOD-N \| 1', '| MOD-N | 2' |
            Set-Content -Path (Join-Path $bad 'HANDOFF.md') -Encoding UTF8
        Copy-Item (Join-Path $clean 'DECISIONS.md') $bad
        $f = @(Invoke-WorkflowDocsChecks -Root $bad | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'summary-table' })
        if ($f.Count -lt 1) { $failures += 'badcount: expected summary-table error, got none' }

        # Case 3: ID both open and archived -> duplicate-id error expected.
        $dup = Join-Path $tmp 'dup'; New-Item -ItemType Directory -Path $dup -Force | Out-Null
        Copy-Item (Join-Path $clean 'HANDOFF.md') $dup
        @'
## MOD-1 - Thing (done, 2026-01-03)

## ANA-1 - Old analysis (done, 2026-01-02)
'@ | Set-Content -Path (Join-Path $dup 'DECISIONS.md') -Encoding UTF8
        $f = @(Invoke-WorkflowDocsChecks -Root $dup | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'duplicate-id' })
        if ($f.Count -lt 1) { $failures += 'dup: expected duplicate-id error, got none' }

        # Case 4: DECISIONS dates increasing downward -> reverse-chron error expected.
        $ord = Join-Path $tmp 'order'; New-Item -ItemType Directory -Path $ord -Force | Out-Null
        Copy-Item (Join-Path $clean 'HANDOFF.md') $ord
        @'
## ANA-1 - Old analysis (done, 2026-01-02)

## ANA-2 - Newer analysis below older (done, 2026-03-01)
'@ | Set-Content -Path (Join-Path $ord 'DECISIONS.md') -Encoding UTF8
        $f = @(Invoke-WorkflowDocsChecks -Root $ord | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'reverse-chron' })
        if ($f.Count -lt 1) { $failures += 'order: expected reverse-chron error, got none' }

        # ---- per-item decision layout ----

        # Case 5: migrated repo, index + file agree -> no errors at all.
        $mig = Join-Path $tmp 'migrated'
        New-DecisionFixture -Dir $mig -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $f = @(Invoke-WorkflowDocsChecks -Root $mig | Where-Object Severity -eq 'Error')
        if ($f.Count -ne 0) { $failures += "migrated: expected 0 errors, got $($f.Count): $($f.Message -join '; ')" }

        # Case 6: index line points at a file that does not exist.
        $miss = Join-Path $tmp 'missingfile'
        New-DecisionFixture -Dir $miss -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
'@
        $f = @(Invoke-WorkflowDocsChecks -Root $miss | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-index' })
        if ($f.Count -lt 1) { $failures += 'missingfile: expected decision-index error, got none' }

        # Case 7: item file exists with no index line pointing at it.
        $orph = Join-Path $tmp 'orphan'
        New-DecisionFixture -Dir $orph -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $f = @(Invoke-WorkflowDocsChecks -Root $orph | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-orphan' })
        if ($f.Count -lt 1) { $failures += 'orphan: expected decision-orphan error, got none' }

        # Case 8: index line ID disagrees with the path it links.
        $idp = Join-Path $tmp 'idpath'
        New-DecisionFixture -Dir $idp -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-6.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-6.md' = $writeUp }
        $f = @(Invoke-WorkflowDocsChecks -Root $idp | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-id-path' })
        if ($f.Count -lt 1) { $failures += 'idpath: expected decision-id-path error, got none' }

        # Case 9: line looks like an index line but does not parse -> must NOT be skipped,
        # a dropped line drops its ID and the next mint reuses it.
        $bad = Join-Path $tmp 'unparseable'
        New-DecisionFixture -Dir $bad -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $f = @(Invoke-WorkflowDocsChecks -Root $bad | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-index' })
        if ($f.Count -lt 1) { $failures += 'unparseable: expected decision-index error, got none' }

        # Case 10: legacy repo during the transition -> warned about, never blocked.
        # Two-part assertion on purpose: asserting only the warning would let an
        # implementation that also false-positives the new Errors pass this case.
        $leg = Join-Path $tmp 'legacy'
        New-DecisionFixture -Dir $leg -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

## MOD-7 - Thing (done, 2026-01-05)

Write-up body.
'@
        $all = @(Invoke-WorkflowDocsChecks -Root $leg)
        $leftover = @($all | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-legacy' })
        # The other new checks must stay silent: a legacy section is one defect, not four.
        $spurious = @($all | Where-Object { $_.Severity -eq 'Error' -and $_.Check -in @('decision-index', 'decision-orphan', 'decision-id-path') })
        if ($leftover.Count -lt 1) { $failures += 'legacy: expected decision-legacy error, got none' }
        if ($spurious.Count -ne 0) { $failures += "legacy: unexpected errors $($spurious.Check -join ',')" }

        # Case 11: reverse-chronology enforced over index lines, not just headers.
        $iord = Join-Path $tmp 'indexorder'
        New-DecisionFixture -Dir $iord -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-6](docs/decisions/mod/mod-6.md)** - Older (done, 2026-01-02)
- **[MOD-7](docs/decisions/mod/mod-7.md)** - Newer below older (done, 2026-03-01)
'@ -Files @{
            'docs/decisions/mod/mod-6.md' = $writeUp
            'docs/decisions/mod/mod-7.md' = $writeUp
        }
        $f = @(Invoke-WorkflowDocsChecks -Root $iord | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'reverse-chron' })
        if ($f.Count -lt 1) { $failures += 'indexorder: expected reverse-chron error over index lines, got none' }

        # Case 12: ID open in HANDOFF.md and archived in the index -> never reused.
        $idup = Join-Path $tmp 'indexdup'
        New-DecisionFixture -Dir $idup -HandoffContent @'
# HANDOFF

**Current status (2026-01-05):** one open.

## Open items

- [ ] **MOD-7 - Thing.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
'@ -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $f = @(Invoke-WorkflowDocsChecks -Root $idup | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'duplicate-id' })
        if ($f.Count -lt 1) { $failures += 'indexdup: expected duplicate-id error across HANDOFF/index, got none' }

        # Case 13: index lines malformed in FORMAT rather than content — leading
        # whitespace, a `*` bullet, an out-of-set prefix. Each still holds a real ID, so
        # each must error; if the loose pattern is no looser than the strict one these
        # slip through silently and the ID gets minted a second time later.
        $fmt = Join-Path $tmp 'badformat'
        New-DecisionFixture -Dir $fmt -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

 - **[MOD-7](docs/decisions/mod/mod-7.md)** - Leading space (done, 2026-01-05)
* **[MOD-8](docs/decisions/mod/mod-8.md)** - Star bullet (done, 2026-01-04)
- **[FOO-3](docs/decisions/foo/foo-3.md)** - Prefix outside the six (done, 2026-01-03)
'@
        $f = @(Invoke-WorkflowDocsChecks -Root $fmt | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'decision-index' })
        if ($f.Count -lt 3) { $failures += "badformat: expected 3 decision-index errors, got $($f.Count)" }

        # Case 14: supplementary prose under docs/decisions/ holds no ID, so it is not an
        # orphan. A README there must not block the repo.
        $supp = Join-Path $tmp 'supplementary'
        New-DecisionFixture -Dir $supp -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
'@ -Files @{
            'docs/decisions/mod/mod-7.md' = $writeUp
            'docs/decisions/README.md'    = "# Notes`n`nHow this directory is organised."
        }
        $f = @(Invoke-WorkflowDocsChecks -Root $supp | Where-Object Severity -eq 'Error')
        if ($f.Count -ne 0) { $failures += "supplementary: expected 0 errors, got $($f.Count): $($f.Message -join '; ')" }

        # Case 16: checklist lines malformed in format — a ticked box left instead of
        # deleted, and a leading space. Same dropped-ID logic as index lines, mirrored onto
        # the HANDOFF half (minter case 11); the well-formed line must stay silent.
        $bchk = Join-Path $tmp 'badchecklist'
        New-DecisionFixture -Dir $bchk -HandoffContent @'
# HANDOFF

**Current status (2026-01-05):** one open.

## Open items

- [ ] **MOD-11 - Fine.** body
- [x] **MOD-12 - Ticked, left instead of deleted.** body
 - [ ] **MOD-13 - Leading space.** body

## Summary

| Area  | Open |
|-------|------|
| MOD-N | 1    |
'@ -DecisionsContent @'
# DECISIONS
'@
        $all16 = @(Invoke-WorkflowDocsChecks -Root $bchk)
        $f = @($all16 | Where-Object { $_.Severity -eq 'Error' -and $_.Check -eq 'checklist-unparseable' })
        if ($f.Count -lt 2) { $failures += "badchecklist: expected 2 checklist-unparseable errors, got $($f.Count)" }
        $spurious16 = @($all16 | Where-Object { $_.Severity -eq 'Error' -and $_.Check -ne 'checklist-unparseable' })
        if ($spurious16.Count -ne 0) { $failures += "badchecklist: unexpected errors $($spurious16.Check -join ',')" }

        # Case 15: a RELATIVE -RepoRoot must behave identically to an absolute one.
        # Get-DecisionFiles cuts repo-relative paths out of absolute FullNames, so cutting
        # by the length of a relative root mangles every path and orphans every write-up.
        Push-Location $tmp
        try {
            $rel = @(Invoke-WorkflowDocsChecks -Root 'migrated' | Where-Object Severity -eq 'Error')
        } finally { Pop-Location }
        if ($rel.Count -ne 0) { $failures += "relativeroot: expected 0 errors via relative root, got $($rel.Count): $($rel.Message -join '; ')" }

        # Case 17: -ScopePaths gate. A pre-existing DECISIONS.md error must not fail a
        # commit that stages only HANDOFF.md (the repro this flag exists for) - but it
        # stays visible, still fails a plain whole-repo run, and still blocks the moment
        # DECISIONS.md is staged.
        $scope = Join-Path $tmp 'scopegate'
        New-DecisionFixture -Dir $scope -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** Thing, no status or date
'@ -Files @{ 'docs/decisions/mod/mod-7.md' = $writeUp }
        $scopeFindings = @(Invoke-WorkflowDocsChecks -Root $scope)

        $r = Get-FindingReport -Findings $scopeFindings -ScopeSet (Get-ScopeSet -Raw @('HANDOFF.md')) -ScopeActive $true
        if ($r.BlockingErrors -ne 0) { $failures += "scopegate: HANDOFF.md-only scope blocked on $($r.BlockingErrors) unrelated error(s)" }
        if ($r.DemotedErrors -lt 1) { $failures += 'scopegate: pre-existing error not reported as demoted warning' }
        if (-not (@($r.Lines) -match 'pre-existing')) { $failures += 'scopegate: demoted finding not marked pre-existing in output' }

        $r = Get-FindingReport -Findings $scopeFindings -ScopeSet (Get-ScopeSet -Raw @('DECISIONS.md')) -ScopeActive $true
        if ($r.BlockingErrors -lt 1) { $failures += 'scopegate: staging DECISIONS.md did not make its error blocking' }

        # Empty scope (a commit staging none of the watched paths) blocks on nothing.
        $r = Get-FindingReport -Findings $scopeFindings -ScopeSet (Get-ScopeSet -Raw @('')) -ScopeActive $true
        if ($r.BlockingErrors -ne 0) { $failures += "scopegate: empty scope still blocked on $($r.BlockingErrors) error(s)" }

        # No -ScopePaths at all: whole-repo run, unchanged, still red.
        $r = Get-FindingReport -Findings $scopeFindings -ScopeSet @{} -ScopeActive $false
        if ($r.BlockingErrors -lt 1) { $failures += 'scopegate: unscoped run stopped failing on the error' }
        if ($r.DemotedErrors -ne 0) { $failures += 'scopegate: unscoped run demoted a finding' }

        # Case 18: file attribution reaches beyond HANDOFF/DECISIONS - staging only the
        # write-up path (e.g. deleting it) must block on the index line that points at it,
        # even though DECISIONS.md itself is untouched.
        $sattr = Join-Path $tmp 'scopeattr'
        New-DecisionFixture -Dir $sattr -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

- **[MOD-7](docs/decisions/mod/mod-7.md)** - Thing (done, 2026-01-05)
'@
        $r = Get-FindingReport -Findings @(Invoke-WorkflowDocsChecks -Root $sattr) `
            -ScopeSet (Get-ScopeSet -Raw @('docs/decisions/mod/mod-7.md')) -ScopeActive $true
        if ($r.BlockingErrors -lt 1) { $failures += 'scopeattr: missing-write-up error not attributed to the write-up path' }

        # Case 19: compound legacy header parses all contained IDs.
        $cmp = Join-Path $tmp 'compound'
        New-DecisionFixture -Dir $cmp -HandoffContent $emptyHandoff -DecisionsContent @'
# DECISIONS

## MOD-18/19/20 - Relocated to engine as MOD-2/3/4 (relocated, 2026-07-18)
'@
        $cmpEntries = @(Get-ArchiveEntries -DecisionsLines @(Get-Content -LiteralPath (Join-Path $cmp 'DECISIONS.md') -Encoding UTF8))
        $cmpIds = @($cmpEntries | ForEach-Object Id)
        if ($cmpIds -notcontains 'MOD-18' -or $cmpIds -notcontains 'MOD-19' -or $cmpIds -notcontains 'MOD-20') {
            $failures += "compound: expected MOD-18, MOD-19, MOD-20 from compound header, got: $($cmpIds -join ',')"
        }
    } finally {
        if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    }

    if ($failures.Count -eq 0) {
        Write-Host 'Self-test: 19/19 cases PASS.'
        return 0
    }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-SelfTest) }

if (-not $RepoRoot) {
    # Script lives at <root>/.claude/skills/handoff-run/scripts/ in every repo (M2 keeps
    # this layout) — repo root is four levels up.
    $RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "RepoRoot not found: $RepoRoot"
    exit 2
}

$scopeActive = $PSBoundParameters.ContainsKey('ScopePaths')
$scopeSet = Get-ScopeSet -Raw $ScopePaths

$findings = @(Invoke-WorkflowDocsChecks -Root $RepoRoot)
$report = Get-FindingReport -Findings $findings -ScopeSet $scopeSet -ScopeActive $scopeActive

$errorCount = $report.BlockingErrors
# Demoted errors print as warnings, so they count as warnings in the summary line - but
# they never trip -Strict, which is about warnings the current change is answerable for.
$warningCount = @($findings | Where-Object Severity -eq 'Warning').Count + $report.DemotedErrors

foreach ($line in $report.Lines) { Write-Host $line }
$summary = "workflow-docs validation: $errorCount error(s), $warningCount warning(s) in $RepoRoot"
if ($scopeActive -and $report.DemotedErrors -gt 0) {
    $summary += " ($($report.DemotedErrors) pre-existing error(s) outside the staged files, " +
                'not blocking - a plain run still fails on them)'
}
Write-Host $summary

if ($errorCount -gt 0) { exit 1 }
if ($Strict -and $report.ScopedWarnings -gt 0) { exit 1 }
exit 0

