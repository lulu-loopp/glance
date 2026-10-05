<#
.SYNOPSIS
    Builds a signed release: glance.exe, then the installer with its
    uninstaller, each signed through Artifact Signing (see sign.ps1).

.DESCRIPTION
    Needs an `az login` session allowed to sign with the account sign.ps1
    names. Without -Sign it builds the same files unsigned, as anyone can.
    The installer lands in target\Glance_<version>_x64-setup.exe.
#>
[CmdletBinding()]
param([switch] $Sign)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build exited $LASTEXITCODE" }

$found = Get-Command makensis -ErrorAction SilentlyContinue
$makensis = if ($found) { $found.Source } else { Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe' }
if (-not (Test-Path -LiteralPath $makensis)) { throw 'makensis not found; install NSIS 3.08 or newer' }

if ($Sign) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -Files (Join-Path $root 'target\release\glance.exe')
    # The installer signs its uninstaller and itself as it is made (glance.nsi).
    & $makensis /V2 /DSIGN 'installer\glance.nsi'
}
else {
    & $makensis /V2 'installer\glance.nsi'
}
if ($LASTEXITCODE -ne 0) { throw "makensis exited $LASTEXITCODE" }

Get-ChildItem -LiteralPath (Join-Path $root 'target') -Filter 'Glance_*_x64-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1 |
    ForEach-Object { Write-Host "installer: $($_.FullName)" }
