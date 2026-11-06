<#
.SYNOPSIS
    Builds a signed release: glance.exe and glance-sensors.exe, then the
    installer with its uninstaller, each signed through Artifact Signing
    (see sign.ps1).

.DESCRIPTION
    Needs an `az login` session allowed to sign with the account sign.ps1
    names. Without -Sign it builds the same files unsigned, as anyone can.
    The installer lands in target\Glance_<version>_x64-setup.exe, with Glance's
    own signature beside it, which every release must carry (see
    update-signature.ps1): only the one who holds the key can, and without the
    key this stops before it builds. -WithoutUpdateSignature is for a build that
    will not be released: installed copies will not take it as an update.
#>
[CmdletBinding()]
param([switch] $Sign, [switch] $WithoutUpdateSignature)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

if (-not $WithoutUpdateSignature -and -not [Environment]::GetEnvironmentVariable('GLANCE_UPDATE_KEY', 'User')) {
    throw 'GLANCE_UPDATE_KEY is not set: every release carries Glance''s own signature (for a build not to be released, -WithoutUpdateSignature)'
}

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
# The programs released and their PDBs, kept here by version (never shipped): what a hang written down on a user's PC
# holds is read back with them (see src/hang.rs). A build of the same version again replaces them.
$symbols = Join-Path $root "symbols\$version"
New-Item -ItemType Directory -Force -Path $symbols | Out-Null
foreach ($name in 'glance', 'glance-sensors') {
    foreach ($kind in 'exe', 'pdb') {
        Copy-Item -LiteralPath (Join-Path $root "target\release\$name.$kind") -Destination $symbols -Force
    }
}
Write-Host "symbols: $symbols"
if (-not $WithoutUpdateSignature) {
    & (Join-Path $PSScriptRoot 'update-signature.ps1') -Installer $installer -Version $version
    & (Join-Path $PSScriptRoot 'update-signature.ps1') -Installer $installer -Version $version -Check
}
