param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][string]$BaselineInstaller,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-(?:beta|rc)\.\d+)?$')][string]$ExpectedVersion,
  [string]$LeagueInstaller
)
$ErrorActionPreference = 'Stop'
# Real product keys are required to exercise the released installer. This
# process may never use the developer's registry or application data.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Packaged upgrade acceptance is restricted to ephemeral GitHub-hosted runners.'
}
$baselineHash = '583201413a2b31d94f3009766a22dfa6cda65ddb6eba3847d3895eefdf863c1e'
if ((Get-FileHash -LiteralPath $BaselineInstaller).Hash -ne $baselineHash) {
  throw 'The baseline is not the published 3.1.4 repair package.'
}
Write-Host ("Candidate installer SHA256: " + (Get-FileHash -LiteralPath $Installer).Hash)
$productKey = 'HKCU:\Software\cs2insightagent\MaxGameStudio'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\MaxGameStudio'
$storageKey = 'HKCU:\Software\MaxGameStudio\Storage'
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Desktop')) 'MaxGameStudio.lnk'
$root = Join-Path $env:RUNNER_TEMP 'mgs-upgrade-acceptance'
$binaryName = 'cs2-insight-agent-desktop.exe'
$defaultProgram = Join-Path $env:LOCALAPPDATA "MaxGameStudio\$binaryName"
$pathsThatMustBeEmpty = @($productKey, $uninstallKey, $storageKey, $shortcutPath, $root, $defaultProgram)
if ($LeagueInstaller) {
  if ($ExpectedVersion -eq '3.1.4' -or
      [IO.Path]::GetFileName($LeagueInstaller) -cnotmatch '^MaxGameStudioLeague_(\d+\.\d+\.\d+(?:-beta\.\d+)?)_x64-setup\.exe$') {
    throw 'Coexistence requires a current Main candidate and the exact standalone League installer.'
  }
  $leagueVersion = $Matches[1]
  $leagueId = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
  $leagueInstallKey = "HKCU:\Software\$leagueId"
  $leagueUninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$leagueId"
  $leagueControl = Join-Path $env:APPDATA 'MaxGameStudioLeague'
  $leagueDirectory = Join-Path $root 'Toolbox Program'
  $leagueProfile = Join-Path $root 'Toolbox Profile'
  $leagueExe = Join-Path $leagueDirectory 'MaxGameStudioLeagueToolbox.exe'
  $leagueRemove = Join-Path $leagueDirectory 'Uninstall MaxGameStudioLeagueToolbox.exe'
  $leagueLocator = Join-Path $leagueControl 'data-location.txt'
  $leagueMarker = Join-Path $leagueProfile 'coexistence-marker.txt'
  $pathsThatMustBeEmpty += @(
    $leagueInstallKey,
    $leagueUninstallKey,
    "HKLM:\Software\$leagueId",
    "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$leagueId",
    $leagueControl,
    (Join-Path $env:APPDATA 'MaxGameStudio\league-runtime')
  )
}
foreach ($path in $pathsThatMustBeEmpty) {
  if (Test-Path -LiteralPath $path) { throw 'Runner already contains an acceptance or product path.' }
}
$oldDirectory = Join-Path $root 'Custom Install'
$expectedDirectory = Join-Path $root 'Expected'
$profile = Join-Path $root 'Existing Profile'
$data = Join-Path $profile 'data'
$relocated = Join-Path $root 'Relocated Profile'
New-Item -ItemType Directory -Path $oldDirectory,$expectedDirectory,$data | Out-Null

function Install-Package([string]$Exe, [string]$Arguments, [int]$ExpectedExit = 0) {
  Write-Host "INSTALL_ATTEMPT: $([IO.Path]::GetFileName($Exe)) $Arguments (expected $ExpectedExit)"
  $process = Start-Process -FilePath $Exe -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne $ExpectedExit) {
    if ([IO.Path]::GetFileName($Exe) -ceq 'MaxGameStudio_3.2.0-beta.1_x64-setup.exe') {
      $registered = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction SilentlyContinue
      Write-Host ("MAIN_INSTALL_DIAGNOSTIC: {0}" -f (@{
        registeredDirectory = $registered.InstallLocation; registeredVersion = $registered.DisplayVersion
        expectedDirectory = $oldDirectory
        binaryExists = Test-Path -LiteralPath (Join-Path $oldDirectory $binaryName)
        publisherKeyExists = Test-Path -LiteralPath $productKey
        runningMain = @(Get-Process -Name 'cs2-insight-agent-desktop' -ErrorAction SilentlyContinue).Count
      } | ConvertTo-Json -Compress))
      $checker = Join-Path $PSScriptRoot '../../../frontend/src-tauri/windows/validate-install-directories.ps1'
      & "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -File $checker -ProgramDir "$oldDirectory\."
      Write-Host "DIRECTORY_VALIDATOR_DIAGNOSTIC: exit $LASTEXITCODE"
    }
    throw "$([IO.Path]::GetFileName($Exe)) returned $($process.ExitCode), expected $ExpectedExit."
  }
}
function Read-StorageLocator {
  (Get-ItemPropertyValue -LiteralPath $storageKey -Name LocationV1) | ConvertFrom-Json
}

function Wait-AppWindow([Diagnostics.Process]$App, [string]$Title, [string]$Name) {
  $deadline = [DateTime]::UtcNow.AddSeconds(45)
  do {
    $App.Refresh()
    if ($App.HasExited) { throw "$Name exited before showing its main window." }
    if ($App.MainWindowHandle -ne [IntPtr]::Zero -and $App.MainWindowTitle -match $Title) {
      Write-Host "$Name opened its main window."
      return
    }
    Start-Sleep -Milliseconds 300
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "$Name did not show its main window. Observed title: $($App.MainWindowTitle)"
}

$sampledProducts = @{}
function Write-IdleResourceSample([Diagnostics.Process]$App, [string]$Product) {
  if ($sampledProducts.ContainsKey($Product)) { return }
  # Let fresh WebView/Electron profile initialization settle before sampling idle.
  Start-Sleep -Seconds 10
  $processIds = [Collections.Generic.HashSet[int]]::new()
  [void]$processIds.Add($App.Id)
  $snapshot = @(Get-CimInstance Win32_Process)
  do {
    $added = $false
    foreach ($process in $snapshot) {
      if ($processIds.Contains([int]$process.ParentProcessId) -and $processIds.Add([int]$process.ProcessId)) { $added = $true }
    }
  } while ($added)
  $first = @(Get-Process -Id @($processIds) -ErrorAction SilentlyContinue)
  $cpu = ($first | Measure-Object CPU -Sum).Sum
  $started = Get-Date
  Start-Sleep -Seconds 5
  $last = @(Get-Process -Id @($processIds) -ErrorAction SilentlyContinue)
  $App.Refresh()
  if ($App.HasExited) { throw "$Product exited during the idle resource sample." }
  $elapsed = ((Get-Date) - $started).TotalSeconds
  $privateSet = (Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
    Where-Object { $processIds.Contains([int]$_.IDProcess) } | Measure-Object WorkingSetPrivate -Sum).Sum
  $programBytes = (Get-ChildItem -LiteralPath ([IO.Path]::GetDirectoryName($App.Path)) -Recurse -File |
    Measure-Object Length -Sum).Sum
  $sample = [ordered]@{
    product = $Product; scene = 'fresh profile, no game client, idle main window'; warmupSeconds = 10; processes = $last.Count
    installedProgramBytes = [long]$programBytes; installedProgramMiB = [math]::Round($programBytes / 1MB, 2)
    privateWorkingSetMiB = [math]::Round($privateSet / 1MB, 2)
    privateCommitMiB = [math]::Round(($last | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 2)
    cpuCorePercent = [math]::Round(((($last | Measure-Object CPU -Sum).Sum - $cpu) / $elapsed * 100), 2)
    seconds = [math]::Round($elapsed, 2)
  }
  Write-Host ('IDLE_RESOURCE_SAMPLE: ' + ($sample | ConvertTo-Json -Compress))
  $sampledProducts[$Product] = $true
}

function Stop-FixtureApp([Diagnostics.Process]$App, [switch]$TryClose) {
  if ($App.HasExited) { return }
  if ($TryClose -and $App.CloseMainWindow() -and $App.WaitForExit(5000)) { return }
  & "$env:SystemRoot\System32\taskkill.exe" /PID "$($App.Id)" /T /F | Out-Null
  if (-not $App.WaitForExit(5000)) { throw 'Could not stop the fixture application.' }
}

function Assert-FileHash([string]$Path, [string]$Hash) {
  if ((Get-FileHash -LiteralPath $Path).Hash -ne $Hash) {
    throw "Fixture file changed unexpectedly: $Path"
  }
}

function Assert-RejectedProgramPath([string]$Exe, [string]$Target) {
  $existed = Test-Path -LiteralPath $Target -PathType Container
  $before = if ($existed) { @(Get-ChildItem -LiteralPath $Target -Force -Name | Sort-Object) -join [char]10 }
  Install-Package $Exe "/S /D=$Target" 2
  if ($existed) {
    $after = @(Get-ChildItem -LiteralPath $Target -Force -Name | Sort-Object) -join [char]10
    if ($before -cne $after) { throw "Rejected installer changed its forbidden target: $Target" }
  } elseif (Test-Path -LiteralPath $Target) {
    throw "Rejected installer created its forbidden target: $Target"
  }
}

function Assert-InstalledLeague {
  $record = Get-ItemProperty -LiteralPath $leagueUninstallKey
  if ($record.DisplayVersion -ne $leagueVersion -or
      (Get-ItemPropertyValue -LiteralPath $leagueInstallKey -Name InstallLocation).Trim('"') -ne $leagueDirectory -or
      -not (Test-Path -LiteralPath $leagueExe -PathType Leaf) -or
      -not (Test-Path -LiteralPath $leagueRemove -PathType Leaf)) {
    throw 'Standalone League did not register in its separate fixture directory.'
  }
}

function Test-LeagueWindow {
  $leagueApp = Start-Process -FilePath $leagueExe -PassThru
  try {
    Wait-AppWindow $leagueApp '^英雄联盟工具箱$' 'League Toolbox'
    Write-IdleResourceSample $leagueApp 'maxgamestudio-league'
    if ([IO.File]::ReadAllText($leagueLocator, [Text.Encoding]::Unicode).Trim() -ne $leagueProfile) {
      throw 'League did not commit the selected test profile.'
    }
  } finally {
    Stop-FixtureApp $leagueApp
  }
}

if ($LeagueInstaller) {
  Install-Package $LeagueInstaller "/S /D=$leagueDirectory"
  Assert-InstalledLeague
  New-Item -ItemType Directory -Path $leagueControl -Force | Out-Null
  # Match the Unicode handoff from the interactive NSIS data directory page.
  $leagueSelection = Join-Path $leagueControl 'installer-data-location.txt'
  [IO.File]::WriteAllText($leagueSelection, $leagueProfile, [Text.Encoding]::Unicode)
  Test-LeagueWindow
  [IO.File]::WriteAllText($leagueMarker, 'fixture toolbox profile')
  $initialLeagueHash = (Get-FileHash -LiteralPath $leagueExe).Hash
  $leagueMarkerHash = (Get-FileHash -LiteralPath $leagueMarker).Hash
  $leagueLocatorHash = (Get-FileHash -LiteralPath $leagueLocator).Hash
  foreach ($target in @($leagueDirectory, (Join-Path $leagueDirectory 'Overlapping Main'), $leagueProfile)) {
    Assert-RejectedProgramPath $Installer $target
  }
  if (Test-Path -LiteralPath $uninstallKey) { throw 'A rejected Main installer created a product registration.' }
  Assert-FileHash $leagueExe $initialLeagueHash
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash
}

# NSIS consumes the rest of the command line after /D= as the destination.
Install-Package $BaselineInstaller "/S /D=$oldDirectory"
$installed = Join-Path $oldDirectory $binaryName
$oldUninstall = Get-ItemProperty -LiteralPath $uninstallKey
if ($oldUninstall.DisplayVersion -ne '3.1.4' -or $oldUninstall.InstallLocation.Trim('"') -ne $oldDirectory) {
  throw 'The stable repair package did not register in the fixture directory.'
}
$oldHash = (Get-FileHash -LiteralPath $installed).Hash
$embedded = Join-Path $oldDirectory 'league-runtime\MaxGameStudioLeague.exe'
if (-not (Test-Path -LiteralPath $embedded -PathType Leaf)) { throw 'Expected embedded executable in the stable baseline.' }

$python = Join-Path $oldDirectory 'python\python.exe'
$fixture = Join-Path $root 'fixture.py'
@'
import json
import sqlite3
import sys
from pathlib import Path

action, folder = sys.argv[1:]
data = Path(folder)
config = data / "cs2-insight.config.json"
database = data / "cs2-insight.db"
if action == "seed":
    config.write_text(json.dumps({"locale": "zh-CN", "close_action": "exit"}), encoding="utf-8")
    with sqlite3.connect(database) as conn:
        conn.execute("create table acceptance_marker (name text)")
        conn.execute("insert into acceptance_marker values (?)", ("stable profile",))
else:
    assert json.loads(config.read_text(encoding="utf-8"))["locale"] == "zh-CN"
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as conn:
        assert conn.execute("pragma quick_check").fetchone() == ("ok",)
        assert conn.execute("select name from acceptance_marker").fetchone() == ("stable profile",)
'@ | Set-Content -LiteralPath $fixture -Encoding utf8
& $python -I -B $fixture seed $data
if ($LASTEXITCODE -ne 0) { throw 'Cannot create the synthetic config and SQLite data.' }
$configHash = (Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.config.json')).Hash
$databaseHash = (Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.db')).Hash
$location = @{ root = $profile; mode = 'unified'; required_paths = @($data) } | ConvertTo-Json -Compress
New-Item -Path $storageKey -Force | Out-Null
New-ItemProperty -LiteralPath $storageKey -Name LocationV1 -Value $location | Out-Null

& 'C:\Program Files\7-Zip\7z.exe' e $Installer "-o$expectedDirectory" $binaryName -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the candidate executable.' }
# The candidate takes the normal manual overlay route (no /D or /UPDATE).
# The published repair retest still exercises the stable updater's /P /UPDATE.
if ($ExpectedVersion -eq '3.1.4') {
  Install-Package $Installer '/P /UPDATE'
} else {
  # Exercise the exact uninstall-key fallback when the publisher key was lost.
  # This key was made by our baseline installer in the isolated runner above.
  Remove-Item -LiteralPath $productKey
  Install-Package $Installer '/S'
}
$installedHash = (Get-FileHash -LiteralPath $installed).Hash
$expectedHash = (Get-FileHash -LiteralPath (Join-Path $expectedDirectory $binaryName)).Hash
if ($installedHash -ne $expectedHash -or ($ExpectedVersion -ne '3.1.4' -and $installedHash -eq $oldHash)) {
  throw 'The original executable was not replaced by the selected candidate.'
}
if ((Get-ItemPropertyValue -LiteralPath $productKey -Name '(default)') -ne $oldDirectory) {
  throw 'Product registration changed directory.'
}
$record = Get-ItemProperty -LiteralPath $uninstallKey
if ($record.DisplayVersion -ne $ExpectedVersion -or $record.InstallLocation.Trim('"') -ne $oldDirectory) {
  throw 'Uninstall record is stale or points elsewhere.'
}
$shell = New-Object -ComObject WScript.Shell
if ($shell.CreateShortcut($shortcutPath).TargetPath -ne $installed) { throw 'Shortcut no longer points to the original installation.' }
if (Test-Path -LiteralPath $defaultProgram) { throw 'An unintended default-directory copy was created.' }
if ((Read-StorageLocator).root -ne $profile) { throw 'Default upgrade changed the active storage location.' }
if ((Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.config.json')).Hash -ne $configHash -or
    (Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.db')).Hash -ne $databaseHash) {
  throw 'Default upgrade changed synthetic profile files.'
}
& $python -I -B $fixture verify $data
if ($LASTEXITCODE -ne 0) { throw 'Candidate runtime cannot read the retained config and SQLite row.' }

if ($ExpectedVersion -ne '3.1.4') {
  if (Test-Path -LiteralPath $embedded) { throw 'Embedded League still occupies the program directory.' }

  # The native NSIS fixture checks the page's registry write separately. Seed
  # the same handoff here so the packaged executable verifies it end to end.
  $selection = "1-1|$relocated"
  New-ItemProperty -LiteralPath $storageKey -Name InstallerDataRootV1 -Value $selection | Out-Null
  $app = Start-Process -FilePath $installed -PassThru
  try {
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
      $locator = Read-StorageLocator
      if ($locator.last_switch_error) { throw "Profile relocation failed: $($locator.last_switch_error)" }
      if ($locator.root -eq $relocated -and (Test-Path -LiteralPath (Join-Path $relocated 'temp'))) { break }
      if ($app.HasExited) { throw "Candidate exited before selecting the relocated data root." }
      Start-Sleep -Milliseconds 300
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($locator.root -ne $relocated -or $locator.previous -ne $profile -or $locator.installer_selection -ne $selection) {
      throw 'Candidate did not commit the selected root and retain the old locator.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $relocated 'temp'))) { throw 'Candidate did not initialize the new runtime directory.' }
    Wait-AppWindow $app '^MaxGameStudio$' 'MaxGameStudio'
    Write-IdleResourceSample $app 'maxgamestudio'
    foreach ($folder in @($data, (Join-Path $relocated 'data'))) {
      & $python -I -B $fixture verify $folder
      if ($LASTEXITCODE -ne 0) { throw 'Cannot read the retained or relocated profile.' }
    }
  } finally {
    Stop-FixtureApp $app -TryClose
  }
}
Write-Host 'PACKAGED_UPGRADE_VERIFIED: repair package overlaid in place, registry and shortcut agree, and the synthetic data is readable.'

if ($LeagueInstaller) {
  $mainFiles = @{}
  foreach ($folder in @($data, (Join-Path $relocated 'data'))) {
    foreach ($file in @('cs2-insight.config.json', 'cs2-insight.db')) {
      $filePath = Join-Path $folder $file
      $mainFiles[$filePath] = (Get-FileHash -LiteralPath $filePath).Hash
    }
  }
  $mainFiles[$installed] = $installedHash
  Assert-FileHash $leagueExe $initialLeagueHash
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash

  Install-Package $leagueRemove '/S'
  if ((Test-Path -LiteralPath $leagueExe) -or (Test-Path -LiteralPath $leagueUninstallKey)) {
    throw 'League uninstall did not remove its own program and registration.'
  }
  if ((Get-ItemProperty -LiteralPath $uninstallKey).DisplayVersion -ne $ExpectedVersion -or
      (Read-StorageLocator).root -ne $relocated) { throw 'League uninstall changed Main registration or profile.' }
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash
  foreach ($target in @($oldDirectory, (Join-Path $oldDirectory 'Overlapping Toolbox'), $profile, $relocated)) {
    Assert-RejectedProgramPath $LeagueInstaller $target
  }
  if (Test-Path -LiteralPath $leagueUninstallKey) { throw 'A rejected League installer created a product registration.' }
  foreach ($path in $mainFiles.Keys) { Assert-FileHash $path $mainFiles[$path] }
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash

  $remainingMain = Start-Process -FilePath $installed -PassThru
  try {
    Wait-AppWindow $remainingMain '^MaxGameStudio$' 'MaxGameStudio after League uninstall'
  } finally {
    Stop-FixtureApp $remainingMain -TryClose
  }

  Install-Package $LeagueInstaller "/S /D=$leagueDirectory"
  Assert-InstalledLeague
  Assert-FileHash $leagueExe $initialLeagueHash
  Install-Package (Join-Path $oldDirectory 'uninstall.exe') '/S'
  if ((Test-Path -LiteralPath $installed) -or (Test-Path -LiteralPath $uninstallKey)) {
    throw 'Main uninstall did not remove its own program and registration.'
  }
  Assert-InstalledLeague
  Assert-FileHash $leagueExe $initialLeagueHash
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash
  foreach ($path in $mainFiles.Keys) {
    if ($path -ne $installed) { Assert-FileHash $path $mainFiles[$path] }
  }
  Test-LeagueWindow
  Write-Host 'TWO_PRODUCT_COEXISTENCE_VERIFIED: both installers rejected overlapping destinations, and each uninstall preserved the other product and the stored data.'
}
