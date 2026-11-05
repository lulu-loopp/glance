<#
.SYNOPSIS
    Builds a signed release: glance.exe and glance-sensors.exe, then the
    installer with its uninstaller, each signed through Artifact Signing
    (see sign.ps1).

.DESCRIPTION
    Needs an `az login` session allowed to sign with the account sign.ps1
    names. Without -Sign it builds the same files unsigned, as anyone can.
    The installer lands in target\Glance_<version>_x64-setup.exe. With
    -UpdateSignature it gets Glance's own signature beside it, which every
    release must carry (see update-signature.ps1): only the one who holds the
    key can.
#>
[CmdletBinding()]
param([switch] $Sign, [switch] $UpdateSignature)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build exited $LASTEXITCODE" }
# The phone remote: a crate of its own, built into its own folder.
cargo build --release --manifest-path remote\Cargo.toml
if ($LASTEXITCODE -ne 0) { throw "cargo build (remote) exited $LASTEXITCODE" }

$found = Get-Command makensis -ErrorAction SilentlyContinue
$makensis = if ($found) { $found.Source } else { Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe' }
if (-not (Test-Path -LiteralPath $makensis)) { throw 'makensis not found; install NSIS 3.08 or newer' }

if ($Sign) {
    & (Join-Path $PSScriptRoot 'sign.ps1') -Files (Join-Path $root 'target\release\glance.exe'), (Join-Path $root 'target\release\glance-sensors.exe')
    & (Join-Path $PSScriptRoot 'sign.ps1') -Files (Join-Path $root 'remote\target\release\glance-remote.exe')
    # The installer signs its uninstaller and itself as it is made (glance.nsi).
    & $makensis /V2 /DSIGN 'installer\glance.nsi'
}
else {
    & $makensis /V2 'installer\glance.nsi'
}
if ($LASTEXITCODE -ne 0) { throw "makensis exited $LASTEXITCODE" }

$version = [regex]::Match((Get-Content -Raw Cargo.toml), '(?m)^version = "([^"]+)"').Groups[1].Value
$installer = Join-Path $root "target\Glance_${version}_x64-setup.exe"
Write-Host "installer: $installer"
if ($UpdateSignature) {
    & (Join-Path $PSScriptRoot 'update-signature.ps1') -Installer $installer -Version $version
}
