<#
.SYNOPSIS
    Signs files through Microsoft's Artifact Signing service (formerly Trusted
    Signing), time-stamped, and verifies each signature.

.DESCRIPTION
    No key is kept here. The service holds the key and issues a certificate
    valid for three days; signtool reaches it through Microsoft's signing
    library (/dlib), told which account and certificate profile to use by a
    small JSON file (/dmdf). Who may sign is decided by Azure, against the
    `az login` session the library finds on its own.

    Because the certificate lives three days, every signature is time-stamped
    and a signature without a time stamp is refused afterwards.

    The account, endpoint and profile are names of public resources, not
    secrets; GLANCE_SIGN_ENDPOINT, GLANCE_SIGN_ACCOUNT and GLANCE_SIGN_PROFILE
    override them.

.PARAMETER Files
    The files to sign, in place.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string[]] $Files,
    [string] $ClientVersion = '1.0.128',
    [int] $TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (Test-Path -LiteralPath 'Variable:PSNativeCommandUseErrorActionPreference') {
    $PSNativeCommandUseErrorActionPreference = $false
}

$Endpoint = if ($env:GLANCE_SIGN_ENDPOINT) { $env:GLANCE_SIGN_ENDPOINT } else { 'https://eus.codesigning.azure.net' }
$Account = if ($env:GLANCE_SIGN_ACCOUNT) { $env:GLANCE_SIGN_ACCOUNT } else { 'folio-sign' }
$CertificateProfile = if ($env:GLANCE_SIGN_PROFILE) { $env:GLANCE_SIGN_PROFILE } else { 'folio-public' }
$TimestampUrl = 'http://timestamp.acs.microsoft.com'

$targets = foreach ($file in $Files) {
    $path = (Resolve-Path -LiteralPath $file).ProviderPath
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 2 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) { throw "$path is not a PE image" }
    $path
}

# The newest x64 signtool: the signing library is loaded into its process, and
# one older than 10.0.22621.755 ignores /dlib and looks in the local store.
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$tool = Get-ChildItem -LiteralPath $kits -Directory |
    Where-Object { $_.Name -match '^10\.' } |
    Sort-Object { [version] $_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path -LiteralPath $_ } |
    Select-Object -First 1
if (-not $tool) { throw 'no x64 signtool.exe in the Windows Kits; install the Windows SDK signing tools' }
$info = (Get-Item -LiteralPath $tool).VersionInfo
$version = [version] ('{0}.{1}.{2}.{3}' -f $info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart)
if ($version -lt [version] '10.0.22621.755') { throw "$tool is $version; 10.0.22621.755 or newer is needed" }

# The library is a .NET 8 assembly; without the runtime signing fails with no
# error code at all.
$runtimes = Join-Path $env:ProgramFiles 'dotnet\shared\Microsoft.NETCore.App'
$net8 = (Test-Path -LiteralPath $runtimes) -and (Get-ChildItem -LiteralPath $runtimes -Directory |
    Where-Object { $_.Name -match '^(\d+)\.' -and [int] $Matches[1] -ge 8 })
if (-not $net8) { throw 'the signing library needs the x64 .NET Runtime 8 or newer' }

# The library, fetched from nuget.org once per version and kept.
$cache = Join-Path $env:LOCALAPPDATA "Glance\artifact-signing\$ClientVersion"
$dlib = Join-Path $cache 'bin\x64\Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path -LiteralPath $dlib)) {
    Write-Host "fetching Microsoft.ArtifactSigning.Client $ClientVersion"
    $package = Join-Path ([IO.Path]::GetTempPath()) "artifact-signing-$ClientVersion.zip"
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.artifactsigning.client/$ClientVersion/microsoft.artifactsigning.client.$ClientVersion.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $package -UseBasicParsing
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($package)
    try {
        foreach ($entry in $zip.Entries | Where-Object { $_.FullName.StartsWith('bin/x64/') -and -not $_.FullName.EndsWith('/') }) {
            $out = Join-Path $cache $entry.FullName
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($out)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $out, $true)
        }
    }
    finally { $zip.Dispose(); Remove-Item -LiteralPath $package -Force }
    if (-not (Test-Path -LiteralPath $dlib)) { throw "the package has no $dlib" }
}

# The library runs `az` by name from inside signtool, so the Azure CLI must be
# on the PATH, and its sign-in still good: an expired one stalls on a prompt
# nobody sees instead of failing.
if (-not (Get-Command az -ErrorAction SilentlyContinue)) {
    $cli = Join-Path $env:ProgramFiles 'Microsoft SDKs\Azure\CLI2\wbin'
    if (-not (Test-Path -LiteralPath (Join-Path $cli 'az.cmd'))) { throw 'the Azure CLI is not installed' }
    $env:PATH = "$cli;$env:PATH"
}
$token = & az account get-access-token --scope 'https://codesigning.azure.net/.default' --query expiresOn -o tsv 2>&1
if ($LASTEXITCODE -ne 0) { throw "no usable Azure sign-in (run az login): $token" }
Write-Host "Azure sign-in good until $token"

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('glance-sign-' + [Guid]::NewGuid().ToString('n'))
[IO.Directory]::CreateDirectory($scratch) | Out-Null
try {
    $metadata = Join-Path $scratch 'metadata.json'
    $json = [ordered] @{ Endpoint = $Endpoint; CodeSigningAccountName = $Account; CertificateProfileName = $CertificateProfile } | ConvertTo-Json
    [IO.File]::WriteAllText($metadata, $json, (New-Object Text.UTF8Encoding $false))
    $arguments = @('sign', '/v', '/fd', 'SHA256', '/tr', $TimestampUrl, '/td', 'SHA256', '/dlib', $dlib, '/dmdf', $metadata) + $targets
    $quoted = ($arguments | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
    # Bounded: a signtool waiting on something it will never get is killed.
    $process = Start-Process -FilePath $tool -ArgumentList $quoted -NoNewWindow -PassThru
    # Its handle taken while it runs: without it, the exit code reads empty
    # once the process has gone.
    $null = $process.Handle
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill()
        throw "signtool did not finish within $TimeoutSeconds s"
    }
    if ($process.ExitCode -ne 0) { throw "signtool sign exited $($process.ExitCode)" }
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue
}

foreach ($target in $targets) {
    $signature = Get-AuthenticodeSignature -LiteralPath $target
    if ($signature.Status -ne 'Valid') { throw "${target}: signature is $($signature.Status): $($signature.StatusMessage)" }
    if (-not $signature.TimeStamperCertificate) { throw "${target}: the signature has no time stamp" }
    $verified = & $tool verify /pa /tw $target 2>&1
    if ($LASTEXITCODE -ne 0) { $verified | Write-Host; throw "${target}: signtool verify exited $LASTEXITCODE" }
    Write-Host ("{0}: signed by {1}; time-stamped by {2}" -f [IO.Path]::GetFileName($target),
        $signature.SignerCertificate.GetNameInfo('SimpleName', $false), $signature.TimeStamperCertificate.GetNameInfo('SimpleName', $false))
}
