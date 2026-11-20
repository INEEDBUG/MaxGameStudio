#Requires -Version 5.1

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$SourceRoot
)

$ErrorActionPreference = "Stop"
$source = & (Join-Path $PSScriptRoot "prepare-native-source.ps1") -SourceRoot $SourceRoot
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "../..")).ProviderPath
$worker = "rust/maxgamestudio-parser-worker"
$inputRoot = Join-Path $repo $worker
$stageRoot = Join-Path $source $worker

foreach ($relative in @("Cargo.toml", "Cargo.lock", "src/main.rs", "src/replay.rs", "tests/ipc.rs")) {
    $destination = Join-Path $stageRoot $relative
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination)) | Out-Null
    Copy-Item -LiteralPath (Join-Path $inputRoot $relative) -Destination $destination -Force
}

# The original upstream notice travels beside any distributed worker.
$notice = Join-Path $stageRoot "LICENSE.upstream.txt"
Copy-Item -LiteralPath (Join-Path $source "LICENSE") -Destination $notice -Force

[pscustomobject]@{
    Manifest = Join-Path $stageRoot "Cargo.toml"
    Notice = $notice
}
