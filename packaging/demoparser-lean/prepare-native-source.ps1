#Requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot
)

$ErrorActionPreference = "Stop"
$pinPath = Join-Path $PSScriptRoot "demoparser-runtime.json"
$manifestPath = Join-Path $PSScriptRoot "native-source-files.json"
$patchPath = Join-Path $PSScriptRoot "demoparser2-v0.41.4.patch"
$pin = Get-Content -LiteralPath $pinPath -Raw -Encoding UTF8 | ConvertFrom-Json
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$manifestHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
$patchHash = (Get-FileHash -LiteralPath $patchPath -Algorithm SHA256).Hash.ToLowerInvariant()
$sourceLimit = 4MB

if ($pin.upstream_url -cne "https://github.com/LaihoE/demoparser.git" -or
    $manifest.commit -cne $pin.commit -or $pin.commit -cnotmatch '^[a-f0-9]{40}$' -or
    $patchHash -cne $pin.patch_sha256) {
    throw "The native parser source and patch pins do not agree."
}
if ($manifest.files.Count -eq 0 -or
    ($manifest.files | Measure-Object -Property size -Sum).Sum -gt $sourceLimit) {
    throw "The pinned parser source list is empty or exceeds its byte budget."
}

$root = [IO.Path]::GetFullPath($SourceRoot).TrimEnd('\', '/')
$leaf = [IO.Path]::GetFileName($root)
$parent = (Resolve-Path -LiteralPath ([IO.Path]::GetDirectoryName($root))).ProviderPath
$root = Join-Path $parent $leaf
$receiptName = ".mgs-native-parser-source.json"

function Assert-OwnedSource([string]$Path) {
    $receiptPath = Join-Path $Path $receiptName
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        throw "The parser source directory lacks a native source receipt."
    }
    $receipt = Get-Content -LiteralPath $receiptPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($receipt.commit -cne $pin.commit -or $receipt.patchSha256 -cne $patchHash -or
        $receipt.manifestSha256 -cne $manifestHash) {
        throw "The existing parser source receipt has different pins."
    }
    foreach ($entry in $manifest.files) {
        $file = Join-Path $Path $entry.path
        $expected = $receipt.files.PSObject.Properties[$entry.path].Value
        if (-not $expected -or -not (Test-Path -LiteralPath $file -PathType Leaf) -or
            (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -cne $expected) {
            throw "Prepared native parser source differs from its receipt for $($entry.path)."
        }
    }
}

if (Test-Path -LiteralPath $root) {
    Assert-OwnedSource $root
    Write-Output $root
    return
}

function Get-GitBlobHash([byte[]]$Bytes) {
    $prefix = [Text.Encoding]::ASCII.GetBytes("blob $($Bytes.Length)" + [char]0)
    $object = New-Object byte[] ($prefix.Length + $Bytes.Length)
    [Buffer]::BlockCopy($prefix, 0, $object, 0, $prefix.Length)
    [Buffer]::BlockCopy($Bytes, 0, $object, $prefix.Length, $Bytes.Length)
    $sha = [Security.Cryptography.SHA1]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($object))).Replace("-", "").ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

$stage = Join-Path $parent ("." + $leaf + ".partial-" + [Guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Add-Type -AssemblyName System.Net.Http
    $client = [Net.Http.HttpClient]::new()
    try {
        $client.Timeout = [TimeSpan]::FromSeconds(12)
        $client.MaxResponseContentBufferSize = 1MB
        $client.DefaultRequestHeaders.UserAgent.ParseAdd("MaxGameStudio-native-parser-source/1")
        $started = [Diagnostics.Stopwatch]::StartNew()
        foreach ($entry in $manifest.files) {
            if ($entry.path -cnotmatch '^(LICENSE|src/(parser|csgoproto)/(?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\.(rs|toml|lock|b))$' -or
                $entry.blob -cnotmatch '^[a-f0-9]{40}$' -or $entry.size -le 0 -or $entry.size -gt 1MB) {
                throw "Invalid parser source manifest entry."
            }
            if ($started.Elapsed.TotalSeconds -ge 90) {
                throw "The pinned parser source download timed out."
            }
            $uri = "https://raw.githubusercontent.com/LaihoE/demoparser/$($pin.commit)/$($entry.path)"
            $bytes = $client.GetByteArrayAsync($uri).GetAwaiter().GetResult()
            if ($bytes.Length -ne $entry.size -or (Get-GitBlobHash $bytes) -cne $entry.blob) {
                throw "Source blob verification failed for $($entry.path)."
            }
            $file = Join-Path $stage $entry.path
            New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($file)) -Force | Out-Null
            [IO.File]::WriteAllBytes($file, $bytes)
        }
    } finally {
        $client.Dispose()
    }

    $patchScope = @("--include=src/parser/**", "--include=src/csgoproto/**")
    $patchFiles = @(& git -C $stage apply --numstat @patchScope $patchPath)
    if ($LASTEXITCODE -ne 0 -or
        -not ($patchFiles -match '\tsrc/parser/build\.rs$') -or
        -not ($patchFiles -match '\tsrc/csgoproto/build\.rs$')) {
        throw "The native parser patch scope must cover both build scripts."
    }
    & git -C $stage apply --check @patchScope $patchPath
    if ($LASTEXITCODE -ne 0) { throw "The pinned native parser patch failed git apply --check." }
    & git -C $stage apply @patchScope $patchPath
    if ($LASTEXITCODE -ne 0) { throw "The pinned native parser patch did not apply." }
    & git -C $stage apply --reverse --check @patchScope $patchPath
    if ($LASTEXITCODE -ne 0) { throw "The prepared parser patch failed its reverse check." }

    $hashes = [ordered]@{}
    foreach ($entry in $manifest.files) {
        $hashes[$entry.path] = (Get-FileHash -LiteralPath (Join-Path $stage $entry.path) -Algorithm SHA256).Hash
    }
    $receipt = [ordered]@{
        commit = $pin.commit
        manifestSha256 = $manifestHash
        patchSha256 = $patchHash
        files = $hashes
    }
    $document = $receipt | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText((Join-Path $stage $receiptName), $document, [Text.UTF8Encoding]::new($false))
    Assert-OwnedSource $stage
    $resolvedStage = (Resolve-Path -LiteralPath $stage).ProviderPath
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($resolvedStage), $parent, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([IO.Path]::GetDirectoryName($root), $parent, [StringComparison]::OrdinalIgnoreCase)) {
        throw "The parser source publishing paths must share the selected directory."
    }
    [IO.Directory]::Move($resolvedStage, $root)
} finally {
    if (Test-Path -LiteralPath $stage) {
        $resolvedStage = (Resolve-Path -LiteralPath $stage).ProviderPath
        if ([string]::Equals($resolvedStage, $stage, [StringComparison]::OrdinalIgnoreCase) -and
            [string]::Equals([IO.Path]::GetDirectoryName($resolvedStage), $parent, [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $resolvedStage -Recurse -Force
        }
    }
}

Write-Output $root
