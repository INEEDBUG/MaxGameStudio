#Requires -Version 5.1
param(
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [Parameter(Mandatory = $true)][string]$UnpackedDir,
  [Parameter(Mandatory = $true)][string]$InstallerOutputDir,
  [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = 'Stop'
if ($Version -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-beta\.(0|[1-9][0-9]*))?$') {
  throw 'Expected x.y.z or x.y.z-beta.n.'
}
$source = (Resolve-Path -LiteralPath $SourceDir).Path
$unpacked = (Resolve-Path -LiteralPath $UnpackedDir).Path
$output = [IO.Path]::GetFullPath($InstallerOutputDir)
$outputPrefix = $output.TrimEnd('\') + '\'
$unpackedPrefix = $unpacked.TrimEnd('\') + '\'
if ($outputPrefix.StartsWith($unpackedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Installer output must be outside the unpacked payload.'
}
$installerName = "MaxGameStudioLeague_${Version}_x64-setup.exe"
$installer = Join-Path $output $installerName
if (Test-Path -LiteralPath $installer) { throw 'Use a fresh installer candidate path.' }
$builder = Join-Path $source 'node_modules\electron-builder\cli.js'
$electronPackage = Join-Path $source 'node_modules\electron\package.json'
$runtime = Join-Path $unpacked 'MaxGameStudioLeagueToolbox.exe'
$asar = Join-Path $unpacked 'resources\app.asar'
foreach ($file in @($builder, $electronPackage, $runtime, $asar)) {
  if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required League build input is missing: $file" }
}
$profileFile = Get-ChildItem -LiteralPath $unpacked -Recurse -Force | Where-Object {
  $_.Name -match '^(Cache|GPUCache|Local Storage|Session Storage|Cookies)$' -or
    $_.Extension -in @('.log', '.db', '.sqlite', '.sqlite3')
} | Select-Object -First 1
if ($profileFile) { throw 'Unpacked League input contains runtime logs or profile files. Use pristine build output.' }
if ((& git -C $source rev-parse HEAD | Out-String).Trim() -ne '14557723706ccc0e0a9d62c470141d4cb7190fcd') {
  throw 'SourceDir must be the pinned League Akari source with its installed dependencies.'
}
$node = (Get-Command node.exe -ErrorAction Stop).Source
$readPackage = @'
const {createRequire} = require('node:module')
const {join} = require('node:path')
const local = createRequire(join(process.argv[1], 'package.json'))
process.stdout.write(local('@electron/asar').extractFile(process.argv[2], 'package.json'))
'@
$metadata = & $node -e $readPackage $source $asar | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $metadata.version -cne $Version -or $metadata.name -cne 'league-akari') {
  throw 'Unpacked League package identity or version did not match.'
}
$electron = Get-Content -LiteralPath $electronPackage -Raw | ConvertFrom-Json

$tempRoot = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path
$tempPrefix = $tempRoot.TrimEnd('\') + '\'
$scratch = Join-Path $tempRoot ('mgs-ln-' + [Guid]::NewGuid().ToString('n'))
$uninstaller = Join-Path $scratch ('out\' + $installerName.Replace('.exe', '.__uninstaller.exe'))
if ($uninstaller.Length -ge 260) {
  throw 'Windows TEMP is too long for NSIS WriteUninstaller. Select a shorter build TEMP directory.'
}
$resources = Join-Path $scratch 'build'
New-Item -ItemType Directory -Path $resources | Out-Null
$encoding = [Text.UTF8Encoding]::new($false)
try {
  foreach ($name in @('installer.nsh', 'destination-guard.ps1')) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "league-standalone\$name") -Destination $resources
  }
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'league-workbench-icon.ico') -Destination (Join-Path $resources 'icon.ico')
  [IO.File]::WriteAllText((Join-Path $scratch 'package.json'), ([ordered]@{
    name = $metadata.name
    version = $metadata.version
    main = $metadata.main
    author = $metadata.author
    description = $metadata.description
  } | ConvertTo-Json -Depth 8), $encoding)
  $config = Join-Path $scratch 'electron-builder.json'
  [IO.File]::WriteAllText($config, ([ordered]@{
    appId = 'com.maxgamestudio.league.toolbox'
    productName = 'MaxGameStudio League'
    electronVersion = $electron.version
    directories = @{ output = 'out'; buildResources = 'build' }
    win = @{ target = @('nsis'); executableName = 'MaxGameStudioLeagueToolbox'; icon = 'build/icon.ico' }
    nsis = [ordered]@{
      artifactName = 'MaxGameStudioLeague_${version}_x64-setup.${ext}'
      oneClick = $false
      perMachine = $false
      allowElevation = $false
      packElevateHelper = $false
      selectPerMachineByDefault = $false
      allowToChangeInstallationDirectory = $false
      deleteAppDataOnUninstall = $false
      installerLanguages = @('zh_CN')
      language = '2052'
      shortcutName = '英雄联盟工具箱'
      uninstallDisplayName = '英雄联盟工具箱'
      include = 'build/installer.nsh'
    }
  } | ConvertTo-Json -Depth 8), $encoding)
  & $node $builder --projectDir $scratch --config $config --win nsis --x64 --prepackaged $unpacked --publish never
  $candidate = Join-Path $scratch "out\$installerName"
  if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $candidate -PathType Leaf)) {
    throw 'League NSIS repack failed.'
  }
  New-Item -ItemType Directory -Path $output -Force | Out-Null
  if (Test-Path -LiteralPath $installer) { throw 'Installer output was created by another process.' }
  [IO.File]::Move($candidate, $installer)
  Get-FileHash -LiteralPath $installer -Algorithm SHA256
} finally {
  $temporary = (Resolve-Path -LiteralPath $scratch).Path
  $temporaryItem = Get-Item -LiteralPath $temporary -Force
  if ($temporary.StartsWith($tempPrefix + 'mgs-ln-', [StringComparison]::OrdinalIgnoreCase) -and
      !($temporaryItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    Remove-Item -LiteralPath $temporary -Recurse -Force
  }
}
