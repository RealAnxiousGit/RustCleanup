<#
.SYNOPSIS
    Clean RAM stress test (development builds only).

.DESCRIPTION
    Builds a development copy of RustCleanup, then has it fill RAM with
    helper processes and time Clean RAM against them, several runs in a row.
    Each run prints how much the cleanup freed and how long trimming and
    purging took. A CSV with every run can be written as well.

    Run it from an elevated PowerShell to test the real thing. Without admin
    rights only your own processes can be trimmed and the standby purge is
    skipped, so the numbers will be lower.

.EXAMPLE
    .\scripts\stress-ram.ps1
    .\scripts\stress-ram.ps1 -SizeMB 4096 -Hogs 8 -Runs 10 -Purge -Csv stress.csv
    .\scripts\stress-ram.ps1 -Release   # optimised build with the stress-test feature
#>
param(
    [int]$SizeMB = 2048,
    [int]$Hogs = 4,
    [int]$Runs = 5,
    [switch]$Purge,
    [string]$Csv,
    [switch]$Release
)

# No 'Stop' here: cargo writes its progress to stderr, which Windows
# PowerShell 5.1 would treat as a fatal error. Exit codes are checked instead.
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    if ($Release) {
        cargo build --release --features stress-test
        $exe = Join-Path $root 'target\release\rustcleanup.exe'
    } else {
        cargo build
        $exe = Join-Path $root 'target\debug\rustcleanup.exe'
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    $cli = @('--stress-ram', '--size-mb', $SizeMB, '--hogs', $Hogs, '--runs', $Runs)
    if ($Purge) { $cli += '--purge' }
    if ($Csv) { $cli += @('--csv', $Csv) }

    # The exe asks for admin rights. Without them, run it as the current user
    # instead of letting Windows raise a UAC prompt.
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $admin) {
        Write-Warning 'Not elevated: only your own processes are trimmed and the standby purge is skipped.'
        $env:__COMPAT_LAYER = 'RunAsInvoker'
    }

    Write-Host "Running $exe $($cli -join ' ')"
    & $exe @cli
    $code = $LASTEXITCODE
    Write-Host "Stress test exited with code $code"
    exit $code
} finally {
    Pop-Location
}
