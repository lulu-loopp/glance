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
# The programs released and their PDBs, kept here (never shipped): what a hang written down on a user's PC holds is read
# back with them (see src/hang.rs). Each build kept apart, by version, program and the id its PDB is known by (as a hang
# written down names it), as symbol stores keep them: a build of the same version again is kept beside, never over.
function Get-DebugId([string]$program) {
    # The id in the program's debug directory, of its CodeView entry (RSDS): the PDB's GUID and age.
    $bytes = [IO.File]::ReadAllBytes($program)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $optional = $pe + 24
    $sections = $optional + [BitConverter]::ToUInt16($bytes, $pe + 20)
    # (A 64-bit program's: the debug directory is the seventh of those listed from 112 bytes in.)
    $debug = [BitConverter]::ToUInt32($bytes, $optional + 112 + 6 * 8)
    $debugSize = [BitConverter]::ToUInt32($bytes, $optional + 112 + 6 * 8 + 4)
    $offsetOf = {
        param($rva)
        for ($i = 0; $i -lt [BitConverter]::ToUInt16($bytes, $pe + 6); $i++) {
            $section = $sections + 40 * $i
            $at = [BitConverter]::ToUInt32($bytes, $section + 12)
            $size = [BitConverter]::ToUInt32($bytes, $section + 16)
            if ($rva -ge $at -and $rva -lt $at + $size) { return $rva - $at + [BitConverter]::ToUInt32($bytes, $section + 20) }
        }
        throw "$program has no section holding $rva"
    }
    $directory = & $offsetOf $debug
    for ($entry = $directory; $entry -lt $directory + $debugSize; $entry += 28) {
        if ([BitConverter]::ToUInt32($bytes, $entry + 12) -ne 2) { continue }
        $codeView = [BitConverter]::ToUInt32($bytes, $entry + 24)
        if ([Text.Encoding]::ASCII.GetString($bytes, $codeView, 4) -ne 'RSDS') { continue }
        $guid = [Guid]::new([byte[]]$bytes[($codeView + 4)..($codeView + 19)])
        return $guid.ToString('N').ToUpperInvariant() + ('{0:X}' -f [BitConverter]::ToUInt32($bytes, $codeView + 20))
    }
    throw "$program names no PDB"
}
foreach ($name in 'glance', 'glance-sensors') {
    $program = Join-Path $root "target\release\$name.exe"
    $symbols = Join-Path $root "symbols\$version\$name\$(Get-DebugId $program)"
    New-Item -ItemType Directory -Force -Path $symbols | Out-Null
    # (A program's PDB is named as its crate is, with _ for -.)
    foreach ($file in "$name.exe", "$($name.Replace('-', '_')).pdb") {
        Copy-Item -LiteralPath (Join-Path $root "target\release\$file") -Destination $symbols -Force
    }
    Write-Host "symbols: $symbols"
}
if (-not $WithoutUpdateSignature) {
    & (Join-Path $PSScriptRoot 'update-signature.ps1') -Installer $installer -Version $version
    & (Join-Path $PSScriptRoot 'update-signature.ps1') -Installer $installer -Version $version -Check
}
