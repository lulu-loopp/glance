<#
.SYNOPSIS
    Makes Glance's update key, once: the key its own signature on an installer
    is made with (see src\own_signature.rs).

.DESCRIPTION
    The private half is kept in this user's environment variable
    GLANCE_UPDATE_KEY and shown nowhere. The public half is printed, to be
    written into src\own_signature.rs. Refuses if the variable is set already:
    a key made anew would leave every Glance that knows the old one without
    updates.

    The private half must be backed up by hand. Lost, Glance installed with its
    public half takes no more updates signed this way. Leaked, anyone can make
    an installer those copies run elevated.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ([Environment]::GetEnvironmentVariable('GLANCE_UPDATE_KEY', 'User')) {
    throw 'GLANCE_UPDATE_KEY is set already: not made again'
}
$key = [Security.Cryptography.ECDsa]::Create([Security.Cryptography.ECCurve+NamedCurves]::nistP256)
[Environment]::SetEnvironmentVariable('GLANCE_UPDATE_KEY', [Convert]::ToBase64String($key.ExportPkcs8PrivateKey()), 'User')
$point = $key.ExportParameters($false).Q
Write-Host ([Convert]::ToHexString($point.X + $point.Y).ToLowerInvariant())
