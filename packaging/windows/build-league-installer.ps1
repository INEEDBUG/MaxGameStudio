#Requires -Version 5.1
param(
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [Parameter(Mandatory = $true)][string]$UnpackedDir,
  [Parameter(Mandatory = $true)][string]$InstallerOutputDir,
  [Parameter(Mandatory = $true)][string]$Version,
  [switch]$Native,
  [string[]]$LegacyInventory,
  [string[]]$LegacyInventorySHA256,
  [string]$WebView2Bootstrapper
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
$inventory = Join-Path $unpacked 'maxgamestudio-runtime-hashes.json'
foreach ($file in @($builder, $electronPackage, $runtime)) {
  if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required League build input is missing: $file" }
}
if ((& git -C $source rev-parse HEAD | Out-String).Trim() -ne '14557723706ccc0e0a9d62c470141d4cb7190fcd') {
  throw 'SourceDir must be the pinned League Akari source with its installed dependencies.'
}
$node = (Get-Command node.exe -ErrorAction Stop).Source
if (!$Native) {
  if ($LegacyInventory -or $LegacyInventorySHA256 -or $WebView2Bootstrapper) {
    throw 'The upgrade catalogs and WebView2 bootstrapper are native package inputs.'
  }
  foreach ($file in @($asar, $inventory)) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required League build input is missing: $file" }
  }
  $profileFile = Get-ChildItem -LiteralPath $unpacked -Recurse -Force | Where-Object {
    $_.Name -match '^(Cache|GPUCache|Local Storage|Session Storage|Cookies)$' -or
      $_.Extension -in @('.log', '.db', '.sqlite', '.sqlite3')
  } | Select-Object -First 1
  if ($profileFile) { throw 'Unpacked League input contains runtime logs or profile files. Use pristine build output.' }
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
$standalone = Join-Path $PSScriptRoot 'league-standalone'
New-Item -ItemType Directory -Path $resources | Out-Null
$encoding = [Text.UTF8Encoding]::new($false)
try {
  foreach ($name in @('installer.nsh', 'destination-guard.ps1', 'retire-user-install.ps1', 'shortcut-target.ps1')) {
    Copy-Item -LiteralPath (Join-Path $standalone $name) -Destination $resources
  }
  if ($Native) {
    . (Join-Path $resources 'retire-user-install.ps1')
    if (!$LegacyInventory.Count -or $LegacyInventory.Count -ne $LegacyInventorySHA256.Count) {
      throw 'Native packaging requires paired verified prior program inventories and SHA-256 values.'
    }
    $prior = @{}
    for ($i = 0; $i -lt $LegacyInventory.Count; $i++) {
      $path = (Resolve-Path -LiteralPath $LegacyInventory[$i]).Path
      if ($LegacyInventorySHA256[$i] -cnotmatch '^[a-fA-F0-9]{64}$' -or
          (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $LegacyInventorySHA256[$i]) {
        throw 'Prior program inventory did not match its pinned SHA-256.'
      }
      $hashes = Get-LeaguePayloadHashes $path
      if (!$hashes[$LeagueInventoryName] -or !$hashes[$LeagueUninstallerName] -or
          $prior.ContainsKey($hashes[$LeagueRuntimeName])) {
        throw 'A prior inventory must uniquely identify its program, inventory and uninstaller.'
      }
      $prior[$hashes[$LeagueRuntimeName]] = $hashes
    }
    [IO.File]::WriteAllText((Join-Path $resources 'prior-program-hashes.json'), ($prior | ConvertTo-Json -Depth 4 -Compress), $encoding)

    $payload = Join-Path $scratch 'native'
    New-Item -ItemType Directory -Path $payload | Out-Null
    $hashes = @{}
    foreach ($item in Get-ChildItem -LiteralPath $unpacked -Force) {
      if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
          $item.Name -cnotin $LeagueNativeFiles -or $item.Name -ceq 'LeagueAkari-MIT.txt' -or !$item.Length) {
        throw 'Unexpected object in unpacked native League payload.'
      }
      $target = Join-Path $payload $item.Name
      [IO.File]::Copy($item.FullName, $target)
      $hashes[$item.Name] = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $notice = Join-Path $payload 'LeagueAkari-MIT.txt'
    $license = & git -C $source show HEAD:LICENSE
    if ($LASTEXITCODE -ne 0 -or !$license) { throw 'Cannot read the pinned League Akari MIT license.' }
    $nativeNotices = @(
      [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../../LICENSE-LEAGUE-MIT.txt'))
      $license -join [Environment]::NewLine
      [IO.File]::ReadAllText((Join-Path $standalone 'native-renderer/artwork/LUCIDE-LICENSE.txt'))
      [IO.File]::ReadAllText((Join-Path $standalone 'native-renderer/artwork/IONICONS-LICENSE.txt'))
      [IO.File]::ReadAllText((Join-Path $standalone 'native-renderer/artwork/COMFORTAA-OFL.txt'))
      [IO.File]::ReadAllText((Join-Path $standalone 'native-shell-links.LICENSE.txt'))
    )
    [IO.File]::WriteAllText($notice, ($nativeNotices -join "`r`n`r`n"), $encoding)
    $hashes['LeagueAkari-MIT.txt'] = (Get-FileHash -LiteralPath $notice -Algorithm SHA256).Hash.ToLowerInvariant()
    $inventory = Join-Path $payload $LeagueInventoryName
    [IO.File]::WriteAllText($inventory, ($hashes | ConvertTo-Json -Compress), $encoding)
    $identity = Get-LeagueNativeRuntime $payload
    if (!$identity -or $identity.Version -cne $Version) { throw 'Native League executable identity or version did not match.' }
    Copy-Item -LiteralPath $inventory -Destination $resources
    $metadata = @{
      name = 'league-akari'
      version = $Version
      main = 'main.js'
      author = 'MaxGameStudio'
      description = 'MaxGameStudio League'
    }
    $unpacked = $payload
    if (!$WebView2Bootstrapper) { throw 'Native packaging requires a local Microsoft WebView2 bootstrapper input.' }
    $verifyWebView2 = Join-Path $PSScriptRoot '../../frontend/src-tauri/windows/verify-webview2-bootstrapper.ps1'
    $bootstrapper = Join-Path $resources 'MicrosoftEdgeWebview2Setup.exe'
    Copy-Item -LiteralPath (Resolve-Path -LiteralPath $WebView2Bootstrapper).Path -Destination $bootstrapper
    $windowsPowerShell = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
    & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $verifyWebView2 -Bootstrapper $bootstrapper
    if ($LASTEXITCODE -ne 0) { throw 'Expected the signed Microsoft WebView2 bootstrapper.' }
    Copy-Item -LiteralPath $verifyWebView2 -Destination $resources
    $include = Join-Path $resources 'installer.nsh'
    [IO.File]::WriteAllText($include, ('!define MGS_LEAGUE_NATIVE_PACKAGE' + [Environment]::NewLine +
      [IO.File]::ReadAllText($include)), $encoding)
  } else {
    Copy-Item -LiteralPath $inventory -Destination $resources
  }
  $icon = if ($Native) { Join-Path $standalone 'native-renderer/artwork/toolbox.ico' }
    else { Join-Path $PSScriptRoot 'league-workbench-icon.ico' }
  Copy-Item -LiteralPath $icon -Destination (Join-Path $resources 'icon.ico')
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
    # The bundled NSIS 3.12 hardens temporary directories for elevated installers.
    toolsets = @{ nsis = '1.2.1' }
    win = @{ target = @('nsis'); executableName = 'MaxGameStudioLeagueToolbox'; icon = 'build/icon.ico' }
    nsis = [ordered]@{
      artifactName = 'MaxGameStudioLeague_${version}_x64-setup.${ext}'
      oneClick = $false
      perMachine = $false
      allowElevation = $true
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
  $launcher = Join-Path $standalone 'run-league-nsis.cjs'
  $nativeShortcutArgs = @()
  if ($Native) { $nativeShortcutArgs += '--native-shortcuts' }
  & $node $launcher $builder $scratch @nativeShortcutArgs --projectDir $scratch --config $config --win nsis --x64 --prepackaged $unpacked --publish never
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
