<#
.SYNOPSIS
    Writes, or checks, Glance's own signature on an installer: the file
    <installer>.sig beside it, which an installed Glance checks before it runs
    an update (see src\own_signature.rs).

.DESCRIPTION
    What is signed is what src\own_signature.rs checks: the words below, the
    version the installer is released as, and the installer's SHA-256. The key
    is the user's GLANCE_UPDATE_KEY (see new-update-key.ps1), and it must be the
    one whose public half the source has. With -Check, nothing is written and no
    private key is needed: an installer that is not as it should be throws.

.EXAMPLE
    scripts\update-signature.ps1 -Installer target\Glance_0.4.3-preview_x64-setup.exe -Version 0.4.3-preview
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Installer,
    [Parameter(Mandatory)] [string] $Version,
    [switch] $Check
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot

$public = [regex]::Match((Get-Content -Raw (Join-Path $root 'src\own_signature.rs')), 'const PUBLIC: &str = "([0-9a-f]{128})"').Groups[1].Value
if (-not $public) { throw 'the public key is not named in src\own_signature.rs' }
$point = [Convert]::FromHexString($public)
$parameters = [Security.Cryptography.ECParameters]@{
    Curve = [Security.Cryptography.ECCurve+NamedCurves]::nistP256
    Q = [Security.Cryptography.ECPoint]@{ X = $point[0..31]; Y = $point[32..63] }
}
$checker = [Security.Cryptography.ECDsa]::Create($parameters)

$digest = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant()
$message = [Text.Encoding]::UTF8.GetBytes("Glance installer`n$Version`n$digest`n")
$file = "$((Resolve-Path -LiteralPath $Installer).Path).sig"

if (-not $Check) {
    $private = [Environment]::GetEnvironmentVariable('GLANCE_UPDATE_KEY', 'User')
    if (-not $private) { throw 'GLANCE_UPDATE_KEY is not set' }
    $signer = [Security.Cryptography.ECDsa]::Create()
    $read = 0
    $signer.ImportPkcs8PrivateKey([Convert]::FromBase64String($private), [ref] $read)
    $signature = $signer.SignData($message, [Security.Cryptography.HashAlgorithmName]::SHA256)
    if (-not $checker.VerifyData($message, $signature, [Security.Cryptography.HashAlgorithmName]::SHA256)) {
        throw 'GLANCE_UPDATE_KEY is not the key whose public half src\own_signature.rs has'
    }
    [IO.File]::WriteAllText($file, [Convert]::ToHexString($signature).ToLowerInvariant(), (New-Object Text.UTF8Encoding $false))
    Write-Host "signature: $file"
}

$written = [Convert]::FromHexString((Get-Content -Raw -LiteralPath $file).Trim())
if (-not $checker.VerifyData($message, $written, [Security.Cryptography.HashAlgorithmName]::SHA256)) {
    throw "$Installer is not signed by Glance's update key as version $Version"
}
