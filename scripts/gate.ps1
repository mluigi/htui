# gate.ps1 - the canonical htui gate (TOOL-9, ANA-29 D5). PowerShell twin of gate.sh: same
# steps, same step names, same last.json shape, same exit codes. Keep the two in lockstep.
#
# Usage:
#   pwsh -NoProfile -File scripts/gate.ps1 [-Only <step>] [-Detach] [-Status] [-SelfTest]
#
#   -Only <step>  run one step (fmt, clippy-all, clippy-bare, test, abort-grep, migrations,
#                 workflow-docs, plugin)
#   -Detach       relaunch in a hidden process, log to target/gate/run.log, print the PID
#   -Status       print target/gate/last.json and exit
#
# Every step runs even after one fails. Exit 0 all passed or skipped, 1 a step failed, 2 usage.
# The test step's output goes to target/gate/test.log; abort-grep reads it, because a stack
# overflow aborts a test binary with no failure counted.

[CmdletBinding()]
param(
    [ValidateSet('fmt', 'clippy-all', 'clippy-bare', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin')]
    [string]$Only,
    [switch]$Detach,
    [switch]$Status,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'

$RepoRoot = if ($env:GATE_REPO_ROOT) { $env:GATE_REPO_ROOT } else { (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
$script:GateDir = Join-Path $RepoRoot 'target/gate'
$script:LastJson = Join-Path $script:GateDir 'last.json'
$script:TestLog = if ($env:GATE_TEST_LOG) { $env:GATE_TEST_LOG } else { Join-Path $script:GateDir 'test.log' }
$script:MigrationsDir = if ($env:GATE_MIGRATIONS_DIR) { $env:GATE_MIGRATIONS_DIR } else { Join-Path $RepoRoot 'crates/htui-store/migrations' }
$PluginDir = Join-Path $RepoRoot '.claude/skills/htui-dev'
$MinClaude = [version]'2.1.287'
$Steps = @('fmt', 'clippy-all', 'clippy-bare', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin')

$script:Results = New-Object System.Collections.Generic.List[object]
$script:Started = ''

# A step returns one of these.
$Pass = 0; $Fail = 1; $Skip = 3

function Write-Last {
    # Atomic: temp file in the same directory, then rename.
    param([string]$State, [string]$Finished = '')
    $head = (& git -C $RepoRoot rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or -not $head) { $head = 'unknown' }
    New-Item -ItemType Directory -Force $script:GateDir | Out-Null
    $doc = [ordered]@{
        head     = "$head".Trim()
        started  = $script:Started
        finished = $(if ($Finished) { $Finished } else { $null })
        state    = $State
        steps    = @($script:Results | ForEach-Object { [ordered]@{ name = $_.name; status = $_.status; seconds = $_.seconds } })
    }
    $tmp = "$($script:LastJson).tmp"
    ($doc | ConvertTo-Json -Depth 5 -Compress) | Set-Content -LiteralPath $tmp -Encoding utf8NoBOM
    Move-Item -Force -LiteralPath $tmp -Destination $script:LastJson
}

function Invoke-Native {
    # Run a native command; its output goes to the host, its exit code is the verdict.
    param([string]$File, [string[]]$Arguments)
    & $File @Arguments
    if ($LASTEXITCODE -eq 0) { return $Pass } else { return $Fail }
}

function Step-Fmt { Invoke-Native cargo @('fmt', '--check') }
function Step-ClippyAll { Invoke-Native cargo @('clippy', '--workspace', '--all-features', '--', '-D', 'warnings') }
function Step-ClippyBare { Invoke-Native cargo @('clippy', '--workspace', '--', '-D', 'warnings') }

function Step-Test {
    New-Item -ItemType Directory -Force $script:GateDir | Out-Null
    & cargo test --workspace --all-features --no-fail-fast -- --test-threads=1 2>&1 | Tee-Object -FilePath $script:TestLog
    # $LASTEXITCODE is cargo's: Tee-Object is a cmdlet and does not touch it.
    if ($LASTEXITCODE -eq 0) { return $Pass } else { return $Fail }
}

function Step-AbortGrep {
    if (-not (Test-Path -LiteralPath $script:TestLog)) {
        Write-Host "abort-grep: no $($script:TestLog) - run the test step first"
        return $Fail
    }
    $hits = @(Select-String -LiteralPath $script:TestLog -Pattern 'SIGABRT|overflowed its stack')
    if ($hits.Count -gt 0) {
        $hits | ForEach-Object { Write-Host "$($_.LineNumber):$($_.Line)" }
        Write-Host 'abort-grep: a test binary aborted; its failures were not counted'
        return $Fail
    }
    Write-Host "abort-grep: no aborts in $($script:TestLog)"
    return $Pass
}

function Step-Migrations {
    # NNNN_name.sql files: the prefixes must be unique and run 0001..N with no gap. A merge of
    # two branches that each added the next number is the failure this catches.
    $bad = $false
    $nums = New-Object System.Collections.Generic.List[int]
    foreach ($f in @(Get-ChildItem -LiteralPath $script:MigrationsDir -Filter '*.sql' -ErrorAction SilentlyContinue)) {
        if ($f.Name -match '^([0-9]{4})_[a-z0-9_]+\.sql$') {
            $nums.Add([int]$Matches[1])
        } else {
            Write-Host "migrations: $($f.Name) does not match NNNN_name.sql"
            $bad = $true
        }
    }
    if ($nums.Count -eq 0) {
        Write-Host "migrations: no migrations under $($script:MigrationsDir)"
        return $Fail
    }
    $expected = 1
    $seen = @{}
    foreach ($n in ($nums | Sort-Object)) {
        if ($seen.ContainsKey($n)) {
            Write-Host "migrations: number $n is used twice"
            $bad = $true
            continue
        }
        $seen[$n] = $true
        if ($n -ne $expected) {
            Write-Host ("migrations: expected {0:D4}, found {1:D4}" -f $expected, $n)
            $bad = $true
            $expected = $n
        }
        $expected++
    }
    if ($bad) { return $Fail }
    Write-Host "migrations: $($nums.Count) contiguous, unique"
    return $Pass
}

function Step-WorkflowDocs {
    Invoke-Native pwsh @('-NoProfile', '-File', (Join-Path $RepoRoot '.claude/skills/handoff-run/scripts/validate-workflow-docs.ps1'), '-RepoRoot', $RepoRoot)
}

function Step-Plugin {
    if (-not (Test-Path -LiteralPath $PluginDir)) { Write-Host "plugin: $PluginDir not present (TOOL-8)"; return $Skip }
    if (-not (Get-Command claude -ErrorAction SilentlyContinue)) { Write-Host 'plugin: claude not on PATH'; return $Skip }
    $out = "$(& claude --version 2>$null)"
    $ver = if ($out -match '([0-9]+\.[0-9]+\.[0-9]+)') { [version]$Matches[1] } else { $null }
    if (-not $ver -or $ver -lt $MinClaude) {
        Write-Host "plugin: claude $(if ($ver) { $ver } else { 'unknown' }) is older than $MinClaude"
        return $Skip
    }
    # Invoke-Native's pipeline is the command's output followed by its verdict: show the first,
    # compare only the last (an array on the left of -ne filters instead of comparing).
    $r = @(Invoke-Native claude @('plugin', 'validate', '--strict', $PluginDir))
    if ($r.Count -gt 1) { $r[0..($r.Count - 2)] | ForEach-Object { Write-Host $_ } }
    if ($r[-1] -ne $Pass) { return $Fail }
    Invoke-Native claude @('plugin', 'test', $PluginDir)
}

function Invoke-Step {
    param([string]$Name)
    $fn = 'Step-' + (($Name -split '-' | ForEach-Object { $_.Substring(0, 1).ToUpper() + $_.Substring(1) }) -join '')
    Write-Host "== gate: $Name"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    # A step's pipeline carries its native output and ends with its verdict (an [int]): print
    # the first, keep the last.
    $rc = $Fail
    & $fn | ForEach-Object { if ($_ -is [int]) { $rc = $_ } else { Write-Host $_ } }
    $status = switch ($rc) { 0 { 'passed' } 3 { 'skipped' } default { 'failed' } }
    $script:Results.Add([pscustomobject]@{ name = $Name; status = $status; seconds = [int]$sw.Elapsed.TotalSeconds })
    Write-Host "== gate: $Name $status"
    Write-Last 'running'
    return ($status -ne 'failed')
}

function Invoke-Gate {
    param([string]$OnlyStep)
    $list = if ($OnlyStep) { @($OnlyStep) } else { $Steps }
    $script:Started = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    Set-Location $RepoRoot
    Write-Last 'running'
    $failed = $false
    foreach ($name in $list) {
        if (-not (Invoke-Step $name)) { $failed = $true }
    }
    $finished = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    if ($failed) { Write-Last 'failed' $finished; Write-Host '== gate: FAILED'; return 1 }
    Write-Last 'passed' $finished
    $skipped = @($script:Results | Where-Object { $_.status -eq 'skipped' } | ForEach-Object { $_.name })
    Write-Host $(if ($skipped.Count -gt 0) { "== gate: PASSED ($($skipped.Count) skipped: $($skipped -join ', '))" } else { '== gate: PASSED' })
    return 0
}

function Invoke-GateSelfTest {
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('gate-selftest-' + [guid]::NewGuid().ToString('N'))
    $failures = @()
    try {
        New-Item -ItemType Directory -Path $tmp -Force | Out-Null
        function New-Migs { param([string]$Case, [string[]]$Names)
            $d = Join-Path $tmp $Case
            New-Item -ItemType Directory -Force $d | Out-Null
            foreach ($n in $Names) { New-Item -ItemType File -Force (Join-Path $d $n) | Out-Null }
            return $d
        }
        $script:MigrationsDir = New-Migs 'ok' @('0001_a.sql', '0002_b.sql', '0003_c.sql')
        if ((Step-Migrations | Select-Object -Last 1) -ne $Pass) { $failures += 'migrations: a contiguous set was refused' }
        $script:MigrationsDir = New-Migs 'dup' @('0001_a.sql', '0002_b.sql', '0002_c.sql')
        if ((Step-Migrations | Select-Object -Last 1) -eq $Pass) { $failures += 'migrations: a duplicated number passed' }
        $script:MigrationsDir = New-Migs 'gap' @('0001_a.sql', '0003_c.sql')
        if ((Step-Migrations | Select-Object -Last 1) -eq $Pass) { $failures += 'migrations: a gap passed' }
        $script:MigrationsDir = New-Migs 'badname' @('0001_a.sql', '2_b.sql')
        if ((Step-Migrations | Select-Object -Last 1) -eq $Pass) { $failures += 'migrations: a malformed name passed' }
        $script:MigrationsDir = New-Migs 'empty' @()
        if ((Step-Migrations | Select-Object -Last 1) -eq $Pass) { $failures += 'migrations: an empty directory passed' }

        $script:TestLog = Join-Path $tmp 'clean.log'; Set-Content $script:TestLog 'test result: ok. 3 passed'
        if ((Step-AbortGrep | Select-Object -Last 1) -ne $Pass) { $failures += 'abort-grep: a clean log failed' }
        $script:TestLog = Join-Path $tmp 'abort.log'; Set-Content $script:TestLog "thread 'x' has overflowed its stack"
        if ((Step-AbortGrep | Select-Object -Last 1) -eq $Pass) { $failures += 'abort-grep: a stack overflow passed' }
        $script:TestLog = Join-Path $tmp 'sigabrt.log'; Set-Content $script:TestLog 'signal: 6, SIGABRT: process abort signal'
        if ((Step-AbortGrep | Select-Object -Last 1) -eq $Pass) { $failures += 'abort-grep: SIGABRT passed' }
        $script:TestLog = Join-Path $tmp 'missing.log'
        if ((Step-AbortGrep | Select-Object -Last 1) -eq $Pass) { $failures += 'abort-grep: a missing log passed' }

        # last.json: the documented shape, and no .tmp left behind.
        $script:GateDir = Join-Path $tmp 'gate'; $script:LastJson = Join-Path $script:GateDir 'last.json'
        $script:Started = '2026-01-01T00:00:00Z'
        $script:Results.Clear()
        $script:Results.Add([pscustomobject]@{ name = 'fmt'; status = 'passed'; seconds = 1 })
        $script:Results.Add([pscustomobject]@{ name = 'test'; status = 'failed'; seconds = 2 })
        Write-Last 'failed' '2026-01-01T00:00:03Z'
        $raw = Get-Content -Raw $script:LastJson
        $j = $raw | ConvertFrom-Json
        # `finished` from the text: ConvertFrom-Json reads an ISO time as a DateTime.
        if ($j.state -ne 'failed' -or @($j.steps).Count -ne 2 -or $j.steps[1].status -ne 'failed' -or $raw -notmatch '"finished":"2026-01-01T00:00:03Z"') {
            $failures += 'last.json: shape'
        }
        if (Test-Path "$($script:LastJson).tmp") { $failures += 'last.json: temp file left behind' }
        Write-Last 'running'
        if ($null -ne (Get-Content -Raw $script:LastJson | ConvertFrom-Json).finished) { $failures += 'last.json: running state has a finished time' }

        # The htui-dev mod's /gate checks a step name against its own copy of the steps (TOOL-8 M2).
        $mod = Join-Path $RepoRoot '.claude/skills/htui-dev/hooks/commands.ts'
        if (Test-Path -LiteralPath $mod) {
            $m = [regex]::Match((Get-Content -Raw -LiteralPath $mod), 'GATE_STEPS = \[([^\]]*)\]')
            $listed = @([regex]::Matches($m.Groups[1].Value, "'([a-z-]+)'") | ForEach-Object { $_.Groups[1].Value })
            if (($listed -join ' ') -ne ($Steps -join ' ')) { $failures += "htui-dev GATE_STEPS ($($listed -join ' ')) differ from the steps ($($Steps -join ' '))" }
        }
    } finally {
        if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    }
    if ($failures.Count -eq 0) { Write-Host 'Self-test: all cases PASS.'; return 0 }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-GateSelfTest) }

if ($Status) {
    if (Test-Path -LiteralPath $script:LastJson) { Get-Content -Raw $script:LastJson; exit 0 }
    [Console]::Error.WriteLine("no gate result yet ($($script:LastJson))")
    exit 1
}

if ($Detach) {
    New-Item -ItemType Directory -Force $script:GateDir | Out-Null
    $runLog = Join-Path $script:GateDir 'run.log'
    if ($IsWindows) {
        # No redirection on Start-Process: with it, .NET starts the child with handle inheritance
        # on, and a caller reading this launcher through a pipe (the htui-dev mod's /gate) would
        # wait on the gate's copy of that pipe until the gate ends. Without it the child starts
        # through ShellExecute, inherits nothing, and writes its own log.
        $q = { param([string]$s) "'" + $s.Replace("'", "''") + "'" }
        $cmd = "& $(& $q $PSCommandPath)$(if ($Only) { " -Only $Only" }) *> $(& $q $runLog)"
        $p = Start-Process pwsh -ArgumentList @('-NoProfile', '-Command', $cmd) -WindowStyle Hidden -PassThru
    } else {
        # Elsewhere .NET hands the child only its three redirected streams.
        $argList = @('-NoProfile', '-File', $PSCommandPath)
        if ($Only) { $argList += @('-Only', $Only) }
        $p = Start-Process pwsh -ArgumentList $argList -PassThru `
            -RedirectStandardOutput $runLog `
            -RedirectStandardError (Join-Path $script:GateDir 'run.err.log')
    }
    Write-Host "gate started, pid $($p.Id) - log $(Join-Path $script:GateDir 'run.log'), result: pwsh scripts/gate.ps1 -Status"
    exit 0
}

exit (Invoke-Gate $Only)
