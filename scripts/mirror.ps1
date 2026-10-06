<#
.SYNOPSIS
    Copies a release published on GitHub to the Gitee mirror, which mainland
    China can reach: its commit and tag, its notes, and every file attached.

.DESCRIPTION
    Needs `gh` signed in to GitHub, and $env:GITEE_TOKEN, a Gitee personal
    access token with the `projects` scope. A release already on Gitee is
    left as it is, but for the files it lacks. Installed copies of Glance
    find updates on either site and run only an installer signed as they
    are, so the mirror carries nothing they would take on trust.

.EXAMPLE
    scripts\mirror.ps1 -Version 0.1.7
#>
[CmdletBinding()]
param([Parameter(Mandatory)] [string] $Version)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root

$token = $env:GITEE_TOKEN
if (-not $token) { throw 'GITEE_TOKEN is not set' }
$tag = "v$Version"
$api = 'https://gitee.com/api/v5/repos/lulu-loopp/glance'
$auth = @{ Authorization = "token $token" }

# The release as GitHub has it.
$release = gh release view $tag --repo lulu-loopp/glance --json name,body,tagName,assets | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "no release $tag on GitHub" }
$commit = (git rev-list -n 1 $tag).Trim()
if ($LASTEXITCODE -ne 0) { throw "no tag $tag here" }
# The source mirrored is the one the release was made from.
# An annotated tag is listed with its commit ("^{}"); a lightweight one is
# its commit.
$listed = @(git ls-remote https://github.com/lulu-loopp/glance.git "refs/tags/$tag" "refs/tags/$tag^{}")
$line = $listed | Where-Object { $_ -match '\^\{\}$' } | Select-Object -First 1
if (-not $line) { $line = $listed | Select-Object -First 1 }
$published = if ($line) { ($line -split '\s+')[0] } else { '' }
if ($published -ne $commit) { throw "GitHub's $tag is on $published, this checkout's on $commit" }

# The commit and tag on Gitee, pushed with the token in git's environment
# (not on its command line). The mirror pulls the same from GitHub in time;
# pushing them first means the release need not wait for it.
$tags = Invoke-RestMethod -Uri "$api/tags" -Headers $auth
$mirrored = @($tags | Where-Object { $_.name -eq $tag })
if ($mirrored) {
    if ($mirrored[0].commit.sha -ne $commit) { throw "Gitee's $tag is on $($mirrored[0].commit.sha), not $commit" }
    Write-Host "tag $tag is on Gitee already"
}
else {
    $basic = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("lulu-loopp:$token"))
    $env:GIT_CONFIG_COUNT = '1'
    $env:GIT_CONFIG_KEY_0 = 'http.https://gitee.com/.extraheader'
    $env:GIT_CONFIG_VALUE_0 = "Authorization: Basic $basic"
    try {
        git push https://gitee.com/lulu-loopp/glance.git "refs/tags/$tag"
        if ($LASTEXITCODE -ne 0) { throw "git push to Gitee exited $LASTEXITCODE" }
    }
    finally {
        Remove-Item Env:GIT_CONFIG_COUNT, Env:GIT_CONFIG_KEY_0, Env:GIT_CONFIG_VALUE_0
    }
}

# The release on Gitee, made if it is not there yet.
$existing = Invoke-RestMethod -Uri "$api/releases/tags/$tag" -Headers $auth -SkipHttpErrorCheck -StatusCodeVariable status
if ($status -eq 200 -and $existing -and $existing.id) {
    $target = $existing
    Write-Host "release $tag is on Gitee already (id $($target.id))"
}
else {
    $fields = @{ tag_name = $tag; name = $release.name; body = $release.body; target_commitish = $commit; prerelease = $false }
    $target = Invoke-RestMethod -Method Post -Uri "$api/releases" -Headers $auth -ContentType 'application/json; charset=utf-8' `
        -Body ([Text.Encoding]::UTF8.GetBytes(($fields | ConvertTo-Json)))
    Write-Host "release $tag made on Gitee (id $($target.id))"
}

# Every file GitHub has, that Gitee does not yet.
$have = @($target.assets | Where-Object { $_.PSObject.Properties['name'] } | ForEach-Object name)
$download = Join-Path ([IO.Path]::GetTempPath()) "glance-mirror-$tag"
New-Item -ItemType Directory -Force $download | Out-Null
try {
    foreach ($asset in $release.assets) {
        if ($have -contains $asset.name) {
            Write-Host "  $($asset.name): there already"
            continue
        }
        gh release download $tag --repo lulu-loopp/glance --pattern $asset.name --dir $download --clobber
        if ($LASTEXITCODE -ne 0) { throw "could not download $($asset.name) from GitHub" }
        $file = Get-Item -LiteralPath (Join-Path $download $asset.name)
        Invoke-RestMethod -Method Post -Uri "$api/releases/$($target.id)/attach_files" -Headers $auth -Form @{ file = $file } | Out-Null
        Write-Host "  $($asset.name): uploaded ($([math]::Round($file.Length / 1MB, 1)) MB)"
    }
}
finally {
    Remove-Item -LiteralPath $download -Recurse -Force
}

# What installed copies will read: the newest release, and its installer.
# With the token: anonymous calls from one address are rate limited.
$latest = Invoke-RestMethod -Uri "$api/releases/latest" -Headers $auth
$installer = $latest.assets | Where-Object { $_.name -eq "Glance_${Version}_x64-setup.exe" }
if (-not $installer) { throw "Gitee's latest release does not offer Glance_${Version}_x64-setup.exe" }
Write-Host "latest on Gitee: $($latest.tag_name), installer $($installer.browser_download_url)"
