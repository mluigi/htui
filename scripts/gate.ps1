# gate.ps1 - the canonical htui gate (TOOL-9, ANA-29 D5). PowerShell twin of gate.sh: same
# steps, same step names, same last.json shape, same exit codes. Keep the two in lockstep.
# The doc step runs cargo doc under the workspace's deny-level rustdoc lints.
#
# Usage:
#   pwsh -NoProfile -File scripts/gate.ps1 [-Only <step>] [-Slow] [-Detach] [-Status] [-SelfTest]
#
#   -Only <step>  run one step (fmt, clippy-all, clippy-bare, doc, postgres, qdrant, test,
#                 abort-grep, migrations, workflow-docs, plugin)
#   -Slow         run the opt-in slow sweeps even when nothing they cover differs from main
#   -Detach       relaunch in a hidden process, log to target/gate/run.log, print the PID
#   -Status       print target/gate/last.json and exit
#
# Every step runs even after one fails. Exit 0 all passed or skipped, 1 a step failed, 2 usage.
# The test step runs the whole workspace test suite under cargo-nextest without fail-fast and
# then its doctests (TOOL-17). Its output goes to target/gate/test.log; abort-grep reads it for a
# crash (stack overflow, SIGABRT). Under cargo test a crash ends the test's whole binary with no
# failure counted for the rest of it; under nextest it fails that one test, which may then pass on
# its one retry (FLAKY) while the abort text stays in the log. abort-grep keeps a crash red even
# when the retry passes, deliberately: a crash is a bug, not a flake. The test step needs
# cargo-nextest and fails without it: there is no fallback to cargo test, which would be a
# different green.

[CmdletBinding()]
param(
    [ValidateSet('fmt', 'clippy-all', 'clippy-bare', 'doc', 'postgres', 'qdrant', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin')]
    [string]$Only,
    [switch]$Slow,
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
$Steps = @('fmt', 'clippy-all', 'clippy-bare', 'doc', 'postgres', 'qdrant', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin')
# The pinned cargo-nextest (TOOL-17 D3): the test step prints its install line when none is on PATH.
$NextestVersion = '0.9.151'
# The opt-in slow sweeps: `#[ignore]`d tests in a `slow_sweep` module, too slow for every run. The
# test step runs them when a path below differs from main (committed, staged, unstaged or
# untracked), or under -Slow. A path joins the list with the sweep that reads it.
$SlowSweepFilter = 'test(/::slow_sweep::/)'
$SlowSweepPaths = @('crates/htui/src/keys/')
$script:SlowSweepBase = if ($env:GATE_SLOW_SWEEP_BASE) { $env:GATE_SLOW_SWEEP_BASE } else { 'main' }
$script:SlowSweepRoot = $RepoRoot
$script:RunSlow = [bool]$Slow

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
# [workspace.lints.rustdoc] denies broken, private and redundant intra-doc links, and only
# rustdoc reads it: no other step would see one.
function Step-Doc { Invoke-Native cargo @('doc', '--workspace', '--all-features', '--no-deps') }

function Step-Postgres {
    # TOOL-14: without HTUI_TEST_DATABASE_URL every Postgres suite returns early and passes, so
    # the test step's green says nothing about Postgres. Say so as a skipped step. Never print
    # the DSN: it may carry a password.
    $dsn = $env:HTUI_TEST_DATABASE_URL
    if (-not $dsn) {
        Write-Host 'postgres: HTUI_TEST_DATABASE_URL unset - the Postgres suites return early and prove nothing'
        return $Skip
    }
    # The userinfo runs to the last '@' before the path, and the host holds no '@': a password
    # with an unencoded '@' stays out of the output.
    $m = [regex]::Match($dsn, '^postgres(ql)?://([^/?#]*@)?([^:/?#@]+)(:([0-9]+))?')
    if (-not $m.Success) {
        Write-Host 'postgres: DSN set, host not parsed (not probed)'
        return $Pass
    }
    $hostName = $m.Groups[3].Value
    $port = if ($m.Groups[5].Success) { [int]$m.Groups[5].Value } else { 5432 }
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $reached = $false
        try { $reached = $client.ConnectAsync($hostName, $port).Wait(3000) } catch { $reached = $false }
        if (-not $reached) {
            Write-Host "postgres: ${hostName}:$port unreachable - the Postgres suites would fail"
            return $Fail
        }
    } finally { $client.Dispose() }
    Write-Host "postgres: ${hostName}:$port reachable; the Postgres suites run live"
    return $Pass
}

function Step-Qdrant {
    # MOD-160 D10: the Qdrant suites return early without HTUI_TEST_QDRANT_URL, as the Postgres
    # ones do without a DSN. Same verdicts as Step-Postgres. Never print more than host:port: the
    # URL may carry credentials.
    $url = $env:HTUI_TEST_QDRANT_URL
    if (-not $url) {
        Write-Host 'qdrant: HTUI_TEST_QDRANT_URL unset - the Qdrant suites return early and prove nothing'
        return $Skip
    }
    # Userinfo to the last '@', no '@' in the host: as Step-Postgres.
    $m = [regex]::Match($url, '^(https?)://([^/?#]*@)?(\[[^\]/]+\]|[^:/?#@]+)(:([0-9]+))?')
    if (-not $m.Success) {
        Write-Host 'qdrant: URL set, host not parsed (not probed)'
        return $Pass
    }
    $hostName = $m.Groups[3].Value.TrimStart('[').TrimEnd(']')
    # The port the client would dial: the URL's, else the scheme's.
    $port = if ($m.Groups[5].Success) { [int]$m.Groups[5].Value } elseif ($m.Groups[1].Value -eq 'https') { 443 } else { 80 }
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $reached = $false
        try { $reached = $client.ConnectAsync($hostName, $port).Wait(3000) } catch { $reached = $false }
        if (-not $reached) {
            Write-Host "qdrant: ${hostName}:$port unreachable - the Qdrant suites would fail"
            return $Fail
        }
    } finally { $client.Dispose() }
    Write-Host "qdrant: ${hostName}:$port reachable; the Qdrant suites run live"
    return $Pass
}

function Test-SlowSweepsDue {
    # $true = run them. -Slow, or a covered path that differs from the merge base with main. No
    # merge base (main absent, a shallow clone) runs them too: a sweep skipped by accident is the
    # failure this guards against. Returns the sentence it prints, then the verdict.
    if ($script:RunSlow) { return 'test: slow sweeps run (-Slow)', $true }
    $base = & git -C $script:SlowSweepRoot merge-base HEAD $script:SlowSweepBase 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $base) {
        return "test: slow sweeps run (no merge base with $($script:SlowSweepBase))", $true
    }
    $changed = @(& git -C $script:SlowSweepRoot diff --name-only $base -- @SlowSweepPaths) +
        @(& git -C $script:SlowSweepRoot ls-files --others --exclude-standard -- @SlowSweepPaths) |
        Where-Object { $_ }
    if (@($changed).Count -gt 0) {
        return "test: slow sweeps run ($(@($changed).Count) changed under $($SlowSweepPaths -join ' '))", $true
    }
    return "test: slow sweeps skipped (nothing under $($SlowSweepPaths -join ' ') differs from $($script:SlowSweepBase); -Slow runs them)", $false
}

function Step-Test {
    # TOOL-17 (D6): cargo-nextest runs each test in its own process, so tests no longer share one
    # per binary; the live Qdrant cases take turns through the live-qdrant test group in
    # .config/nextest.toml. Profile `gate` retries a failure once and names a pass on retry FLAKY.
    # nextest runs no doctests, so cargo test --doc follows into the same log. Red if either is.
    # No cargo-nextest: fail before running anything, with the install line. Not in an hr sandbox:
    # there ~/.cargo/bin is an empty read-only tmpfs off PATH (docker/hr/compose.hr.yaml), and the
    # image carries nextest, so a sandbox without it runs an image built before TOOL-17.
    if (-not (Get-Command cargo-nextest -CommandType Application -ErrorAction SilentlyContinue)) {
        if ($env:HR_SANDBOX -eq '1') {
            Write-Host 'test: cargo-nextest not installed - this hr image predates TOOL-17; rebuild it on the host with scripts/hr build'
        } else {
            $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
            $cargoBin = Join-Path $cargoHome 'bin'
            if ($IsWindows) {
                Write-Host "test: cargo-nextest not installed - curl.exe -LsSf https://get.nexte.st/$NextestVersion/windows-tar | tar zxf - -C `"$cargoBin`""
            } else {
                Write-Host "test: cargo-nextest not installed - curl -LsSf https://get.nexte.st/$NextestVersion/linux | tar zxf - -C `"$cargoBin`""
            }
        }
        return $Fail
    }
    New-Item -ItemType Directory -Force $script:GateDir | Out-Null
    & cargo nextest run --workspace --all-features --no-fail-fast --profile gate 2>&1 | Tee-Object -FilePath $script:TestLog
    # $LASTEXITCODE is cargo's: Tee-Object is a cmdlet and does not touch it.
    $nextestCode = $LASTEXITCODE
    # The default profile, not `gate`: a sweep is deterministic, so no retry, and the gate's
    # junit.xml stays the main run's.
    $slowCode = 0
    $sentence, $due = Test-SlowSweepsDue
    $sentence | Tee-Object -FilePath $script:TestLog -Append | Write-Host
    if ($due) {
        & cargo nextest run --workspace --all-features --no-fail-fast --run-ignored only -E $SlowSweepFilter 2>&1 |
            Tee-Object -FilePath $script:TestLog -Append
        $slowCode = $LASTEXITCODE
    }
    & cargo test --workspace --all-features --doc 2>&1 | Tee-Object -FilePath $script:TestLog -Append
    $docCode = $LASTEXITCODE
    if ($nextestCode -eq 0 -and $slowCode -eq 0 -and $docCode -eq 0) { return $Pass } else { return $Fail }
}

function Step-AbortGrep {
    if (-not (Test-Path -LiteralPath $script:TestLog)) {
        Write-Host "abort-grep: no $($script:TestLog) - run the test step first"
        return $Fail
    }
    $hits = @(Select-String -LiteralPath $script:TestLog -Pattern 'SIGABRT|overflowed its stack')
    if ($hits.Count -gt 0) {
        $hits | ForEach-Object { Write-Host "$($_.LineNumber):$($_.Line)" }
        Write-Host 'abort-grep: a test aborted. Under nextest it may have passed on retry; under cargo test the rest of its binary did not run'
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

        # TOOL-17 D6: without cargo-nextest the test step fails at once and prints the install line,
        # into $CARGO_HOME/bin. PATH holds only a stand-in cargo that leaves a mark, so a broken check
        # cannot start a real run.
        $bin = Join-Path $tmp 'bin'
        New-Item -ItemType Directory -Force $bin | Out-Null
        $mark = Join-Path $tmp 'cargo-ran'
        if ($IsWindows) {
            Set-Content -LiteralPath (Join-Path $bin 'cargo.cmd') "@type nul > `"$mark`""
        } else {
            Set-Content -LiteralPath (Join-Path $bin 'cargo') "#!/bin/sh`n: > '$mark'"
            & chmod +x (Join-Path $bin 'cargo')
        }
        $cargoHomeCase = Join-Path $tmp 'cargo-home'
        $savedPath = $env:PATH; $savedGateDir = $script:GateDir
        $savedSandbox = $env:HR_SANDBOX; $savedCargoHome = $env:CARGO_HOME
        try {
            $env:PATH = $bin
            $env:HR_SANDBOX = $null; $env:CARGO_HOME = $cargoHomeCase
            $script:GateDir = Join-Path $tmp 'nextest'; $script:TestLog = Join-Path $tmp 'nextest.log'
            $r = @(Step-Test 6>&1)
            # In an hr sandbox nothing can be installed: the message names the image rebuild instead.
            $env:HR_SANDBOX = '1'
            $rs = @(Step-Test 6>&1)
        } finally {
            $env:PATH = $savedPath; $script:GateDir = $savedGateDir
            $env:HR_SANDBOX = $savedSandbox; $env:CARGO_HOME = $savedCargoHome
        }
        $verdict = if ($r.Count -gt 0 -and $r[-1] -is [int]) { $r[-1] } else { $null }
        if ($verdict -ne $Fail) { $failures += "test: a missing cargo-nextest returned $verdict, not a failure" }
        if ("$r" -notlike "*test: cargo-nextest not installed - *https://get.nexte.st/0.9.151/*$(Join-Path $cargoHomeCase 'bin')*") {
            $failures += 'test: a missing cargo-nextest printed no install line into CARGO_HOME/bin'
        }
        $verdict = if ($rs.Count -gt 0 -and $rs[-1] -is [int]) { $rs[-1] } else { $null }
        if ($verdict -ne $Fail) { $failures += "test: a sandbox without cargo-nextest returned $verdict, not a failure" }
        if ("$rs" -notlike '*predates TOOL-17*scripts/hr build*') {
            $failures += 'test: a sandbox without cargo-nextest did not ask for an image rebuild'
        }
        if ("$rs" -like '*get.nexte.st*') { $failures += 'test: a sandbox without cargo-nextest printed an install line' }
        if (Test-Path -LiteralPath $mark) { $failures += 'test: a missing cargo-nextest still ran cargo' }

        # The slow sweeps run on a covered path that differs from main, under -Slow, and without a
        # merge base; nothing else runs them.
        $sweep = Join-Path $tmp 'sweep'
        $g = { & git -C $sweep -c user.name=gate -c user.email=gate@localhost @args }
        New-Item -ItemType Directory -Force (Join-Path $sweep 'crates/htui/src/keys'), (Join-Path $sweep 'crates/other') | Out-Null
        Set-Content -LiteralPath (Join-Path $sweep 'crates/htui/src/keys/a.rs') ''
        Set-Content -LiteralPath (Join-Path $sweep 'crates/other/b.rs') ''
        & $g init -q -b main; & $g add -A; & $g commit -qm base; & $g switch -qc feature
        $savedRoot = $script:SlowSweepRoot; $savedSlow = $script:RunSlow; $savedBase = $script:SlowSweepBase
        function Test-Due { param([bool]$SlowFlag = $false, [string]$Base = 'main')
            $script:SlowSweepRoot = $sweep; $script:RunSlow = $SlowFlag; $script:SlowSweepBase = $Base
            (Test-SlowSweepsDue)[-1]
        }
        try {
            if (Test-Due) { $failures += 'slow sweeps: ran with nothing changed' }
            Set-Content -LiteralPath (Join-Path $sweep 'crates/other/b.rs') 'x'
            if (Test-Due) { $failures += 'slow sweeps: ran for a change outside their paths' }
            if (-not (Test-Due -SlowFlag $true)) { $failures += 'slow sweeps: -Slow did not run them' }
            if (-not (Test-Due -Base 'no-such-branch')) { $failures += 'slow sweeps: no merge base did not run them' }
            Set-Content -LiteralPath (Join-Path $sweep 'crates/htui/src/keys/new.rs') ''
            if (-not (Test-Due)) { $failures += 'slow sweeps: an untracked file under their paths did not run them' }
            Remove-Item -LiteralPath (Join-Path $sweep 'crates/htui/src/keys/new.rs')
            Set-Content -LiteralPath (Join-Path $sweep 'crates/htui/src/keys/a.rs') 'x'
            if (-not (Test-Due)) { $failures += 'slow sweeps: an unstaged edit under their paths did not run them' }
            & $g commit -qam edit
            if (-not (Test-Due)) { $failures += 'slow sweeps: a commit under their paths since main did not run them' }
        } finally {
            $script:SlowSweepRoot = $savedRoot; $script:RunSlow = $savedSlow; $script:SlowSweepBase = $savedBase
        }

        $savedDsn = $env:HTUI_TEST_DATABASE_URL
        try {
            $env:HTUI_TEST_DATABASE_URL = $null
            if ((Step-Postgres | Select-Object -Last 1) -ne $Skip) { $failures += 'postgres: an unset DSN was not skipped' }
            $env:HTUI_TEST_DATABASE_URL = 'postgres://u@127.0.0.1:1/db'
            if ((Step-Postgres | Select-Object -Last 1) -ne $Fail) { $failures += 'postgres: an unreachable host passed' }
            $env:HTUI_TEST_DATABASE_URL = 'host=/run/postgresql dbname=x'
            if ((Step-Postgres | Select-Object -Last 1) -ne $Pass) { $failures += 'postgres: an unparsed DSN failed' }
            $env:HTUI_TEST_DATABASE_URL = 'postgres://u:s3cret@127.0.0.1:1/db'
            if ("$(Step-Postgres 6>&1)" -match 's3cret') { $failures += 'postgres: the DSN password was printed' }
            # An unencoded '@' in the password: the userinfo ends at the last '@', as the client splits it.
            $env:HTUI_TEST_DATABASE_URL = 'postgres://u:p@ss@127.0.0.1:1/db'
            if ("$(Step-Postgres 6>&1)" -match 'ss@') { $failures += 'postgres: a password with an @ was printed' }
        } finally { $env:HTUI_TEST_DATABASE_URL = $savedDsn }

        # Every step names a function. A missing one ends its case's `if` before the case can fail,
        # so the cases below cannot catch it.
        foreach ($s in $Steps) {
            $fn = 'Step-' + (($s -split '-' | ForEach-Object { $_.Substring(0, 1).ToUpper() + $_.Substring(1) }) -join '')
            if (-not (Get-Command $fn -CommandType Function -ErrorAction SilentlyContinue)) { $failures += "step $s has no $fn" }
        }

        $savedQdrant = $env:HTUI_TEST_QDRANT_URL
        try {
            $env:HTUI_TEST_QDRANT_URL = $null
            if ((Step-Qdrant | Select-Object -Last 1) -ne $Skip) { $failures += 'qdrant: an unset URL was not skipped' }
            $env:HTUI_TEST_QDRANT_URL = 'http://127.0.0.1:1'
            if ((Step-Qdrant | Select-Object -Last 1) -ne $Fail) { $failures += 'qdrant: an unreachable host passed' }
            $env:HTUI_TEST_QDRANT_URL = 'not-a-url'
            if ((Step-Qdrant | Select-Object -Last 1) -ne $Pass) { $failures += 'qdrant: an unparsed URL failed' }
            $env:HTUI_TEST_QDRANT_URL = 'http://u:s3cret@127.0.0.1:1'
            if ("$(Step-Qdrant 6>&1)" -match 's3cret') { $failures += 'qdrant: the URL credentials were printed' }
            $env:HTUI_TEST_QDRANT_URL = 'http://u:p@ss@127.0.0.1:1'
            if ("$(Step-Qdrant 6>&1)" -match 'ss@') { $failures += 'qdrant: a password with an @ was printed' }
        } finally { $env:HTUI_TEST_QDRANT_URL = $savedQdrant }

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
        $cmd = "& $(& $q $PSCommandPath)$(if ($Only) { " -Only $Only" })$(if ($Slow) { ' -Slow' }) *> $(& $q $runLog)"
        $p = Start-Process pwsh -ArgumentList @('-NoProfile', '-Command', $cmd) -WindowStyle Hidden -PassThru
    } else {
        # Elsewhere .NET hands the child only its three redirected streams.
        $argList = @('-NoProfile', '-File', $PSCommandPath)
        if ($Only) { $argList += @('-Only', $Only) }
        if ($Slow) { $argList += '-Slow' }
        $p = Start-Process pwsh -ArgumentList $argList -PassThru `
            -RedirectStandardOutput $runLog `
            -RedirectStandardError (Join-Path $script:GateDir 'run.err.log')
    }
    Write-Host "gate started, pid $($p.Id) - log $(Join-Path $script:GateDir 'run.log'), result: pwsh scripts/gate.ps1 -Status"
    exit 0
}

exit (Invoke-Gate $Only)
