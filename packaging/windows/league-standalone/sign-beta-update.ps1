#Requires -Version 7.0
param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][string]$TauriCli
)

$ErrorActionPreference = 'Stop'
$package = Get-Item -LiteralPath $Installer
$version = $package.VersionInfo.ProductVersion
if ($version -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-beta\.(0|[1-9][0-9]*)$' -or
    $package.VersionInfo.ProductName -cne 'MaxGameStudio League' -or
    $package.Name -cne "MaxGameStudioLeague_${version}_x64-setup.exe") {
  throw 'Expected a League Beta installer with matching embedded version and filename.'
}
if (!$env:TAURI_SIGNING_PRIVATE_KEY) { throw 'The League updater signing key is required.' }
$signature = $package.FullName + '.sig'
if (Test-Path -LiteralPath $signature) { throw 'A signature already exists for this installer.' }
$cli = (Resolve-Path -LiteralPath $TauriCli).Path
& node $cli signer sign '--password=' $package.FullName *> $null
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $signature -PathType Leaf)) {
  throw 'Tauri did not produce the League updater signature.'
}
Write-Output "LEAGUE_BETA_UPDATER_SIGNATURE_CREATED: $($package.Name) bytes=$($package.Length)"
