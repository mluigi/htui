# dev-health.ps1 - the two dev-machine facts the htui-dev status line shows that have no portable
# binary (TOOL-8 M2, ANA-29 D6): how full the repo's disk is, and whether the dev Postgres is up.
# PowerShell twin of dev-health.sh: same JSON line, same exit codes. Keep the two in lockstep.
#
# Usage:
#   pwsh -NoProfile -File scripts/dev-health.ps1 [-SelfTest]
#
# Prints one JSON line and exits 0, whatever Postgres answered:
#   {"disk":{"path":"<repo root>","used_pct":80},
#    "postgres":{"state":"up|recovering|down|unknown","via":"pg_isready|docker|tcp","target":"host:port"}}
# used_pct is null when the drive cannot be read.
#
# Postgres is the host and port of HTUI_TEST_DATABASE_URL, else localhost:5439 (compose.yaml).
# The first probe that exists answers: pg_isready (0 up, 1 recovering, 2 down, else unknown), then,
# for a local target only, the htui-postgres container: stopped is down, running is
# `docker exec htui-postgres pg_isready`. Last, a 3 s TCP connect, which can only tell up from down.

[CmdletBinding()]
param(
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'

$RepoRoot = if ($env:DEV_HEALTH_REPO_ROOT) { $env:DEV_HEALTH_REPO_ROOT } else { (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
$Container = 'htui-postgres'

function ConvertFrom-Dsn {
    # A DSN -> @{ Host; Port }. No DSN, or no postgres:// scheme: compose.yaml's localhost:5439.
    # A DSN with no port gets libpq's 5432. user:password@ is dropped up to the last @, a query or
    # fragment is cut, a multi-host list keeps its first host, and an IPv6 host loses its brackets.
    param([string]$Dsn)
    # -c: case-sensitive, as bash's =~ is.
    if (-not $Dsn -or $Dsn -cnotmatch '^postgres(ql)?://') { return @{ Host = 'localhost'; Port = '5439' } }
    $rest = $Dsn -creplace '^postgres(ql)?://', ''
    $rest = ($rest -split '[?#]', 2)[0]
    $rest = ($rest -split '/', 2)[0]
    $at = $rest.LastIndexOf('@')
    if ($at -ge 0) { $rest = $rest.Substring($at + 1) }
    $rest = ($rest -split ',', 2)[0]
    $h = $rest; $p = ''
    if ($rest -match '^\[([^\]]*)\](:([0-9]+))?$') {
        $h = $Matches[1]; $p = if ($Matches[3]) { $Matches[3] } else { '' }
    } elseif ($rest -match '^(.*):([0-9]+)$') {
        $h = $Matches[1]; $p = $Matches[2]
    }
    return @{ Host = $(if ($h) { $h } else { 'localhost' }); Port = $(if ($p) { $p } else { '5432' }) }
}

function Get-Target {
    # host:port, an IPv6 host bracketed.
    param([hashtable]$Pg)
    if ($Pg.Host.Contains(':')) { return "[$($Pg.Host)]:$($Pg.Port)" }
    return "$($Pg.Host):$($Pg.Port)"
}

function Get-DiskPct {
    # The use of $Path's drive as an integer, or $null: used / (used + available), rounded up, as df
    # computes it (blocks reserved for root count as neither). The drive is the
    # mount whose root is the longest prefix of the path: GetPathRoot alone answers `/` on Linux,
    # which in an hr sandbox is the overlay, not the repo's disk.
    param([string]$Path)
    try {
        $full = (Resolve-Path -LiteralPath $Path).Path
        $sep = [System.IO.Path]::DirectorySeparatorChar
        $drive = [System.IO.DriveInfo]::GetDrives() |
            Where-Object {
                $r = $_.RootDirectory.FullName
                $full -eq $r.TrimEnd($sep) -or $full.StartsWith($r.TrimEnd($sep) + $sep, [System.StringComparison]::OrdinalIgnoreCase) -or $r -eq "$sep"
            } |
            Sort-Object { $_.RootDirectory.FullName.Length } -Descending |
            Select-Object -First 1
        if ($null -eq $drive -or $drive.TotalSize -le 0) { return $null }
        $used = $drive.TotalSize - $drive.TotalFreeSpace
        $avail = $drive.AvailableFreeSpace
        if ($used + $avail -le 0) { return $null }
        return [int][math]::Ceiling(100.0 * $used / ($used + $avail))
    } catch {
        return $null
    }
}

function ConvertTo-PgState {
    # pg_isready's exit code -> a state.
    param([int]$Code)
    switch ($Code) { 0 { 'up' } 1 { 'recovering' } 2 { 'down' } default { 'unknown' } }
}

function Test-TcpUp {
    # Whether a TCP connect to $HostName:$Port succeeds within 3 s.
    param([string]$HostName, [int]$Port)
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        return $client.ConnectAsync($HostName, $Port).Wait(3000) -and $client.Connected
    } catch {
        return $false
    } finally {
        $client.Dispose()
    }
}

function Test-Local {
    # Whether $HostName names this machine.
    param([string]$HostName)
    return @('localhost', '127.0.0.1', '::1') -ccontains $HostName
}

function Test-Pg {
    # -> @{ State; Via } for $Pg.
    param([hashtable]$Pg)
    if (Get-Command pg_isready -ErrorAction SilentlyContinue) {
        & pg_isready -h $Pg.Host -p $Pg.Port -t 3 *> $null
        return @{ State = (ConvertTo-PgState $LASTEXITCODE); Via = 'pg_isready' }
    }
    # The container answers only for a local target. Its state first: `docker exec` on a missing or
    # stopped container exits 1, which would read as recovering.
    if ((Test-Local $Pg.Host) -and (Get-Command docker -ErrorAction SilentlyContinue)) {
        $running = "$(& docker container inspect -f '{{.State.Running}}' $Container 2>$null)".Trim()
        if ($LASTEXITCODE -eq 0) {
            if ($running -ne 'true') { return @{ State = 'down'; Via = 'docker' } }
            & docker exec $Container pg_isready -t 3 *> $null
            return @{ State = (ConvertTo-PgState $LASTEXITCODE); Via = 'docker' }
        }
    }
    return @{ State = $(if (Test-TcpUp $Pg.Host ([int]$Pg.Port)) { 'up' } else { 'down' }); Via = 'tcp' }
}

function Format-Health {
    param([string]$Path, $Pct, [string]$State, [string]$Via, [string]$Target)
    $doc = [ordered]@{
        disk     = [ordered]@{ path = $Path; used_pct = $Pct }
        postgres = [ordered]@{ state = $State; via = $Via; target = $Target }
    }
    return ($doc | ConvertTo-Json -Compress -Depth 4)
}

function Invoke-DevHealthSelfTest {
    $failures = @()
    $cases = @(
        @('', 'localhost:5439'),
        @('not a dsn', 'localhost:5439'),
        @('postgres://u:p@localhost:5439/postgres', 'localhost:5439'),
        @('postgres://postgres@localhost:5439/postgres', 'localhost:5439'),
        @('postgres://localhost/db', 'localhost:5432'),
        @('postgresql://u@db.example:6543/x?sslmode=disable#f', 'db.example:6543'),
        @('postgres://u:p@x@h:1/d', 'h:1'),
        @('postgres://[::1]:5440/d', '::1:5440'),
        @('postgres://h1:1,h2:2/d', 'h1:1'),
        @('postgres:///db?host=/run/pg', 'localhost:5432'),
        @('POSTGRES://h:1/d', 'localhost:5439')
    )
    foreach ($c in $cases) {
        $pg = ConvertFrom-Dsn $c[0]
        $got = "$($pg.Host):$($pg.Port)"
        if ($got -ne $c[1]) { $failures += "ConvertFrom-Dsn '$($c[0])': expected $($c[1]), got $got" }
    }
    $t = Get-Target (ConvertFrom-Dsn 'postgres://[::1]:5440/d')
    if ($t -ne '[::1]:5440') { $failures += "Get-Target: IPv6 not bracketed: $t" }

    foreach ($pair in @(@(0, 'up'), @(1, 'recovering'), @(2, 'down'), @(3, 'unknown'), @(127, 'unknown'))) {
        if ((ConvertTo-PgState $pair[0]) -ne $pair[1]) { $failures += "ConvertTo-PgState $($pair[0]): expected $($pair[1])" }
    }

    $j = Format-Health '/a "b"\c' 80 'up' 'pg_isready' 'localhost:5439' | ConvertFrom-Json
    if ($j.disk.path -ne '/a "b"\c' -or $j.disk.used_pct -ne 80 -or $j.postgres.state -ne 'up' -or
        (@($j.postgres.PSObject.Properties.Name) -join ',') -ne 'state,via,target') {
        $failures += 'Format-Health: shape'
    }
    $j = Format-Health '/x' $null 'unknown' 'none' 'localhost:5439' | ConvertFrom-Json
    if ($null -ne $j.disk.used_pct) { $failures += 'Format-Health: null pct' }

    $pct = Get-DiskPct $RepoRoot
    if ($null -eq $pct -or $pct -lt 0 -or $pct -gt 100) { $failures += "Get-DiskPct: not a percentage: '$pct'" }

    if (Test-TcpUp '127.0.0.1' 1) { $failures += 'Test-TcpUp: 127.0.0.1:1 answered' }

    if (-not (Test-Local 'localhost') -or -not (Test-Local '::1') -or (Test-Local 'db.example')) { $failures += 'Test-Local: wrong answer' }

    if ($failures.Count -eq 0) { Write-Host 'Self-test: all cases PASS.'; return 0 }
    Write-Host "Self-test FAIL ($($failures.Count)):"
    $failures | ForEach-Object { Write-Host "  - $_" }
    return 1
}

# ---- main ----

if ($SelfTest) { exit (Invoke-DevHealthSelfTest) }

$pg = ConvertFrom-Dsn $env:HTUI_TEST_DATABASE_URL
$probe = Test-Pg $pg
Write-Output (Format-Health $RepoRoot (Get-DiskPct $RepoRoot) $probe.State $probe.Via (Get-Target $pg))
exit 0
