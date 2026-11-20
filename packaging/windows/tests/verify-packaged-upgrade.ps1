param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][string]$BaselineInstaller,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$BaselineVersion,
  [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$BaselineSha256,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-(?:beta|rc)\.\d+)?$')][string]$ExpectedVersion,
  [string]$LeagueInstaller,
  [switch]$NativePayloads,
  [ValidatePattern('^[a-f0-9]{64}$')][string]$CandidateSha256,
  [ValidatePattern('^[a-f0-9]{64}$')][string]$ParserWorkerSha256,
  [ValidatePattern('^[a-f0-9]{64}$')][string]$LeagueCandidateSha256,
  [string]$LeagueBaselineInstaller,
  [ValidatePattern('^\d+\.\d+\.\d+(?:-beta\.\d+)?$')][string]$LeagueBaselineVersion,
  [ValidatePattern('^[a-f0-9]{64}$')][string]$LeagueBaselineSha256,
  [string]$FixturePythonExe,
  [ValidatePattern('^\d+\.\d+\.\d+$')][string]$FixturePythonVersion
)
$ErrorActionPreference = 'Stop'
# Real product keys are required to exercise the released installer. This
# process may never use the developer's registry or application data.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Packaged upgrade acceptance is restricted to ephemeral GitHub-hosted runners.'
}
if ((Get-FileHash -LiteralPath $BaselineInstaller).Hash -ne $BaselineSha256) {
  throw 'The baseline does not match the verified published Main installer SHA-256.'
}
$candidateHash = (Get-FileHash -LiteralPath $Installer).Hash
$retestPublishedBaseline = $ExpectedVersion -eq $BaselineVersion -and $candidateHash -eq $BaselineSha256
Write-Host "Candidate installer SHA256: $candidateHash"
$productKey = 'HKCU:\Software\cs2insightagent\MaxGameStudio'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\MaxGameStudio'
$storageKey = 'HKCU:\Software\MaxGameStudio\Storage'
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Desktop')) 'MaxGameStudio.lnk'
$root = Join-Path $env:RUNNER_TEMP 'mgs-upgrade-acceptance'
$binaryName = 'cs2-insight-agent-desktop.exe'
$defaultProgram = Join-Path $env:LOCALAPPDATA "MaxGameStudio\$binaryName"
$pathsThatMustBeEmpty = @($productKey, $uninstallKey, $storageKey, $shortcutPath, $root, $defaultProgram)
if ($NativePayloads) {
  if (!$LeagueInstaller -or !$LeagueBaselineInstaller -or !$FixturePythonExe -or !$FixturePythonVersion -or
      [IO.Path]::GetFileName($Installer) -cne "MaxGameStudio_${ExpectedVersion}_x64-setup.exe" -or
      $candidateHash -ine $CandidateSha256 -or
      (Get-FileHash -LiteralPath $LeagueInstaller).Hash -ine $LeagueCandidateSha256 -or
      [IO.Path]::GetFileName($LeagueBaselineInstaller) -cne "MaxGameStudioLeague_${LeagueBaselineVersion}_x64-setup.exe" -or
      (Get-FileHash -LiteralPath $LeagueBaselineInstaller).Hash -ine $LeagueBaselineSha256) {
    throw 'Native acceptance requires the exact verified pair, the League baseline, and the pinned test interpreter.'
  }
  $FixturePythonExe = (Resolve-Path -LiteralPath $FixturePythonExe).Path
  if ($FixturePythonExe.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'SQLite test tooling must be outside the installation fixture.'
  }
  $pythonVersion = & $FixturePythonExe -I -B -c 'import sqlite3, sys; print(".".join(map(str, sys.version_info[:3])))'
  if ($LASTEXITCODE -ne 0 -or $pythonVersion -cne $FixturePythonVersion) { throw 'Test Python or its standard SQLite module did not match the pinned interpreter.' }
}
if ($LeagueInstaller) {
  if ($retestPublishedBaseline -or
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
    if ([IO.Path]::GetFileName($Exe) -ceq "MaxGameStudio_${ExpectedVersion}_x64-setup.exe") {
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
function Write-IdleResourceSample(
  [Diagnostics.Process]$App, [string]$Product, [string]$Version, [string]$Package,
  [string]$Profile, [string]$Role = 'candidate'
) {
  $sampleKey = "$Product-$Role"
  if ($sampledProducts.ContainsKey($sampleKey)) { return }
  # Let WebView/Electron profile initialization settle before sampling idle.
  Start-Sleep -Seconds 10
  $processIds = [Collections.Generic.HashSet[int]]::new()
  [void]$processIds.Add($App.Id)
  $appStart = $App.StartTime
  $snapshot = @(Get-CimInstance Win32_Process)
  do {
    $added = $false
    foreach ($process in $snapshot) {
      if ($process.CreationDate -ge $appStart -and $processIds.Contains([int]$process.ParentProcessId) -and
          $processIds.Add([int]$process.ProcessId)) { $added = $true }
    }
  } while ($added)
  $first = @(Get-Process -Id @($processIds) -ErrorAction SilentlyContinue)
  $cpu = ($first | Measure-Object CPU -Sum).Sum
  $started = Get-Date
  Start-Sleep -Seconds 5
  $last = @(Get-Process -Id @($processIds) -ErrorAction SilentlyContinue)
  $App.Refresh()
  if ($App.HasExited) { throw "$Product exited during the idle resource sample." }
  # Both Main builds open Home first and defer their backend until a tool needs it.
  $pythonChildren = @($last.ProcessName -match '^python(?:w)?$').Count
  if ($NativePayloads -and $Role -eq 'candidate' -and $pythonChildren) {
    throw "$Product ran a Python child during its native idle sample."
  }
  $elapsed = ((Get-Date) - $started).TotalSeconds
  $privateSet = (Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
    Where-Object { $processIds.Contains([int]$_.IDProcess) } | Measure-Object WorkingSetPrivate -Sum).Sum
  $programBytes = (Get-ChildItem -LiteralPath ([IO.Path]::GetDirectoryName($App.Path)) -Recurse -File -Force |
    Measure-Object Length -Sum).Sum
  $profileBytes = (Get-ChildItem -LiteralPath $Profile -Recurse -File -Force | Measure-Object Length -Sum).Sum
  $sample = [ordered]@{
    product = $Product; version = $Version; role = $Role; packageSha256 = (Get-FileHash -LiteralPath $Package).Hash
    scene = 'fixture profile, no game client, idle main window'; warmupSeconds = 10; processes = $last.Count
    pythonChildren = $pythonChildren
    installerBytes = (Get-Item -LiteralPath $Package).Length; profileBytes = [long]$profileBytes
    installedProgramBytes = [long]$programBytes; installedProgramMiB = [math]::Round($programBytes / 1MB, 2)
    privateWorkingSetMiB = [math]::Round($privateSet / 1MB, 2)
    privateCommitMiB = [math]::Round(($last | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 2)
    cpuCorePercent = [math]::Round(((($last | Measure-Object CPU -Sum).Sum - $cpu) / $elapsed * 100), 2)
    seconds = [math]::Round($elapsed, 2)
  }
  Write-Host ('IDLE_RESOURCE_SAMPLE: ' + ($sample | ConvertTo-Json -Compress))
  $sampledProducts[$sampleKey] = $true
}

. (Join-Path $PSScriptRoot 'fixture-process-stop.ps1')
function Stop-FixtureApp([Diagnostics.Process]$App, [switch]$TryClose) {
  Stop-CiFixtureProcess $App -TryClose:$TryClose
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

function Assert-InstalledLeague([string]$Version = $leagueVersion) {
  $record = Get-ItemProperty -LiteralPath $leagueUninstallKey
  if ($record.DisplayVersion -ne $Version -or
      (Get-ItemPropertyValue -LiteralPath $leagueInstallKey -Name InstallLocation).Trim('"') -ne $leagueDirectory -or
      -not (Test-Path -LiteralPath $leagueExe -PathType Leaf) -or
      -not (Test-Path -LiteralPath $leagueRemove -PathType Leaf)) {
    throw 'Standalone League did not register in its separate fixture directory.'
  }
}

function Test-LeagueWindow([string]$Version = $leagueVersion, [string]$Package = $LeagueInstaller, [string]$Role = 'candidate') {
  $leagueApp = Start-Process -FilePath $leagueExe -PassThru
  try {
    Wait-AppWindow $leagueApp '^英雄联盟工具箱$' 'League Toolbox'
    if ([IO.File]::ReadAllText($leagueLocator, [Text.Encoding]::Unicode).Trim() -ne $leagueProfile) {
      throw 'League did not commit the selected test profile.'
    }
    Write-IdleResourceSample $leagueApp 'maxgamestudio-league' $Version $Package $leagueProfile $Role
  } finally {
    Stop-FixtureApp $leagueApp
  }
}

$fixture = Join-Path $root 'fixture.py'
@'
import json
import sqlite3
import sys
from pathlib import Path

action, folder, *args = sys.argv[1:]
data = Path(folder)
is_league = args == ["league"]
config = data / ("base-config.json" if is_league else "cs2-insight.config.json")
database = data / ("LeagueAkari.db" if is_league else "cs2-insight.db")
label = "league profile" if is_league else "stable profile"
if action == "seed":
    if not is_league:
        config.write_text(json.dumps({"locale": "zh-CN", "close_action": "exit"}), encoding="utf-8")
    with sqlite3.connect(database) as conn:
        conn.execute("create table acceptance_marker (name text)")
        conn.execute("insert into acceptance_marker values (?)", (label,))
        conn.commit()
        conn.execute("pragma wal_checkpoint(truncate)")
else:
    saved_config = json.loads(config.read_text(encoding="utf-8"))
    if not is_league:
        assert saved_config["locale"] == "zh-CN"
    with sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True) as conn:
        assert conn.execute("pragma quick_check").fetchone() == ("ok",)
        assert conn.execute("select name from acceptance_marker").fetchone() == (label,)
'@ | Set-Content -LiteralPath $fixture -Encoding utf8

$sevenZip = 'C:\Program Files\7-Zip\7z.exe'
if ($LeagueInstaller) {
  $firstLeaguePackage = if ($NativePayloads) { $LeagueBaselineInstaller } else { $LeagueInstaller }
  $firstLeagueVersion = if ($NativePayloads) { $LeagueBaselineVersion } else { $leagueVersion }
  Install-Package $firstLeaguePackage "/S /D=$leagueDirectory"
  Assert-InstalledLeague $firstLeagueVersion
  New-Item -ItemType Directory -Path $leagueControl -Force | Out-Null
  # Match the Unicode handoff from the interactive NSIS data directory page.
  $leagueSelection = Join-Path $leagueControl 'installer-data-location.txt'
  [IO.File]::WriteAllText($leagueSelection, $leagueProfile, [Text.Encoding]::Unicode)
  if ($NativePayloads) {
    $oldAsar = Join-Path $leagueDirectory 'resources\app.asar'
    if (!(Test-Path -LiteralPath $oldAsar -PathType Leaf)) { throw 'The pinned League baseline does not contain Electron.' }
    $leagueProgramMarker = Join-Path $leagueDirectory 'preserve-unowned-file.txt'
    [IO.File]::WriteAllText($leagueProgramMarker, 'fixture user-owned League program file')
    $leagueProgramMarkerHash = (Get-FileHash -LiteralPath $leagueProgramMarker).Hash
    Test-LeagueWindow $LeagueBaselineVersion $LeagueBaselineInstaller 'baseline'
    $leagueDatabase = Join-Path $leagueProfile 'LeagueAkari.db'
    $leagueConfig = Join-Path $leagueProfile 'base-config.json'
    if (!(Test-Path -LiteralPath $leagueDatabase -PathType Leaf)) {
      throw 'The installed League baseline did not create its SQLite database.'
    }
    if (!(Test-Path -LiteralPath $leagueConfig)) { [IO.File]::WriteAllText($leagueConfig, '{}') }
    & $FixturePythonExe -I -B $fixture seed $leagueProfile league
    if ($LASTEXITCODE -ne 0) { throw 'Cannot seed the installed League baseline database.' }
    $leagueBefore = @{}
    foreach ($path in $leagueDatabase, $leagueConfig, $leagueLocator, $leagueProgramMarker) {
      $leagueBefore[$path] = (Get-FileHash -LiteralPath $path).Hash
    }
    $oldLeagueExeHash = (Get-FileHash -LiteralPath $leagueExe).Hash
    $leagueArchive = Join-Path $expectedDirectory 'app-64.7z'
    & $sevenZip e $LeagueInstaller "-o$expectedDirectory" '$PLUGINSDIR\app-64.7z' -y | Out-Null
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $leagueArchive -PathType Leaf)) { throw 'Could not inspect the League candidate archive.' }
    & $sevenZip e $leagueArchive "-o$expectedDirectory" 'MaxGameStudioLeagueToolbox.exe' -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the League candidate executable.' }
    $expectedLeagueExeHash = (Get-FileHash -LiteralPath (Join-Path $expectedDirectory 'MaxGameStudioLeagueToolbox.exe')).Hash
    [IO.File]::Delete($leagueArchive)
    Install-Package $LeagueInstaller "/S /currentuser --updated /D=$leagueDirectory"
    Assert-InstalledLeague
    Assert-FileHash $leagueExe $expectedLeagueExeHash
    $leaguePe = [Diagnostics.FileVersionInfo]::GetVersionInfo($leagueExe)
    if ($expectedLeagueExeHash -eq $oldLeagueExeHash -or (Test-Path -LiteralPath $oldAsar) -or
        $leaguePe.ProductName -cne 'MaxGameStudio League' -or $leaguePe.ProductVersion -cne $leagueVersion) {
      throw 'The League overlay did not replace its Electron runtime with the verified native candidate.'
    }
    foreach ($path in $leagueBefore.Keys) { Assert-FileHash $path $leagueBefore[$path] }
    Test-LeagueWindow
    & $FixturePythonExe -I -B $fixture verify $leagueProfile league
    if ($LASTEXITCODE -ne 0) { throw 'The League marker is missing or invalid after native startup.' }
    $leagueSavedData = @{
      $leagueDatabase = (Get-FileHash -LiteralPath $leagueDatabase).Hash
      $leagueConfig = (Get-FileHash -LiteralPath $leagueConfig).Hash
    }
    Write-Host 'LEAGUE_NATIVE_OVERLAY_VERIFIED: original directory, program sentinel, data and locator preserved.'
  } else {
    Test-LeagueWindow
  }
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
if ($oldUninstall.DisplayVersion -ne $BaselineVersion -or $oldUninstall.InstallLocation.Trim('"') -ne $oldDirectory) {
  throw 'The published stable installer did not register the expected version in the fixture directory.'
}
$oldHash = (Get-FileHash -LiteralPath $installed).Hash
$embedded = Join-Path $oldDirectory 'league-runtime\MaxGameStudioLeague.exe'
$hadEmbedded = Test-Path -LiteralPath $embedded -PathType Leaf
if ($BaselineVersion -eq '3.1.4' -and !$hadEmbedded) {
  throw 'Expected embedded League executable in the 3.1.4 repair baseline.'
}

$python = if ($NativePayloads) { $FixturePythonExe } else { Join-Path $oldDirectory 'python\python.exe' }
& $python -I -B $fixture seed $data
if ($LASTEXITCODE -ne 0) { throw 'Cannot create the synthetic config and SQLite data.' }
$location = @{ root = $profile; mode = 'unified'; required_paths = @($data) } | ConvertTo-Json -Compress
New-Item -Path $storageKey -Force | Out-Null
New-ItemProperty -LiteralPath $storageKey -Name LocationV1 -Value $location | Out-Null
if ($NativePayloads) {
  $mainProgramMarker = Join-Path $oldDirectory 'preserve-unowned-file.txt'
  [IO.File]::WriteAllText($mainProgramMarker, 'fixture user-owned Main program file')
  $mainProgramMarkerHash = (Get-FileHash -LiteralPath $mainProgramMarker).Hash
  $oldMainApp = Start-Process -FilePath $installed -PassThru
  try {
    Wait-AppWindow $oldMainApp '^MaxGameStudio$' 'Main baseline'
    Write-IdleResourceSample $oldMainApp 'maxgamestudio' $BaselineVersion $BaselineInstaller $profile 'baseline'
  } finally {
    Stop-FixtureApp $oldMainApp -TryClose
  }
  & $python -I -B $fixture verify $data
  if ($LASTEXITCODE -ne 0) { throw 'The Main baseline lost the synthetic profile before overlay.' }
}
$configHash = (Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.config.json')).Hash
$databaseHash = (Get-FileHash -LiteralPath (Join-Path $data 'cs2-insight.db')).Hash

$candidateBinaryPath = if ($NativePayloads) { '$PLUGINSDIR\main-native-files\' + $binaryName } else { $binaryName }
& $sevenZip e $Installer "-o$expectedDirectory" $candidateBinaryPath -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the candidate executable.' }
# The candidate takes the normal manual overlay route (no /D or /UPDATE).
# Reinstalling the exact published baseline exercises the stable updater's /P /UPDATE.
if ($retestPublishedBaseline) {
  Install-Package $Installer '/P /UPDATE'
} else {
  # Exercise the exact uninstall-key fallback when the publisher key was lost.
  # This key was made by our baseline installer in the isolated runner above.
  Remove-Item -LiteralPath $productKey
  Install-Package $Installer '/S'
}
$installedHash = (Get-FileHash -LiteralPath $installed).Hash
$expectedHash = (Get-FileHash -LiteralPath (Join-Path $expectedDirectory $binaryName)).Hash
if ($installedHash -ne $expectedHash -or (!$retestPublishedBaseline -and $installedHash -eq $oldHash)) {
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
if ($LASTEXITCODE -ne 0) { throw 'The retained config and SQLite row are unreadable.' }

if (!$retestPublishedBaseline) {
  if ($hadEmbedded -and (Test-Path -LiteralPath $embedded)) {
    throw 'Embedded League from the baseline still occupies the program directory.'
  }
  if ($NativePayloads) {
    Assert-FileHash $mainProgramMarker $mainProgramMarkerHash
    $mainPe = [Diagnostics.FileVersionInfo]::GetVersionInfo($installed)
    foreach ($relative in 'python\python.exe', 'python\pythonw.exe', 'backend\app\run_server.py') {
      if (Test-Path -LiteralPath (Join-Path $oldDirectory $relative)) { throw "The native Main overlay retained a legacy runtime: $relative" }
    }
    if ($mainPe.ProductName -cne 'MaxGameStudio' -or $mainPe.ProductVersion -cne $ExpectedVersion) {
      throw 'The installed Main PE did not match the native candidate version.'
    }
    if ($ParserWorkerSha256) {
      $parserExe = Join-Path $oldDirectory 'maxgamestudio-parser-worker.exe'
      Assert-FileHash $parserExe $ParserWorkerSha256
      $probe = '{"protocol":1,"action":"probe"}' | & $parserExe | ConvertFrom-Json
      if ($LASTEXITCODE -ne 0 -or $probe.protocol -ne 1 -or !$probe.ok -or
          $probe.parser_commit -cne 'db3ba5f7a1b02b8cb27d0063d52649883a98469c' -or
          $probe.result.actions -cnotcontains 'write_replay_parquet' -or
          !(Test-Path -LiteralPath (Join-Path $oldDirectory 'maxgamestudio-parser-worker.LICENSE.txt') -PathType Leaf)) {
        throw 'The packaged Main parser did not expose the pinned native protocol and notice.'
      }
      Write-Host 'MAIN_NATIVE_PARSER_VERIFIED: installed parser hash, protocol and notice matched.'
    }
    $currentMain = Start-Process -FilePath $installed -PassThru
    try {
      Wait-AppWindow $currentMain '^MaxGameStudio$' 'Native Main'
      Write-IdleResourceSample $currentMain 'maxgamestudio' $ExpectedVersion $Installer $profile
    } finally {
      Stop-FixtureApp $currentMain -TryClose
    }
    if ((Read-StorageLocator).root -ne $profile) { throw 'Native Main startup did not keep the original profile.' }
    & $python -I -B $fixture verify $data
    if ($LASTEXITCODE -ne 0) { throw 'The Main marker is missing or invalid after native startup.' }
    Write-Host 'MAIN_NATIVE_PROFILE_VERIFIED: the installed application opened with the original profile and data.'
  }

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
    Write-IdleResourceSample $app 'maxgamestudio' $ExpectedVersion $Installer $relocated
    foreach ($folder in @($data, (Join-Path $relocated 'data'))) {
      & $python -I -B $fixture verify $folder
      if ($LASTEXITCODE -ne 0) { throw 'Cannot read the retained or relocated profile.' }
    }
  } finally {
    Stop-FixtureApp $app -TryClose
  }
}
Write-Host "PACKAGED_UPGRADE_VERIFIED: stable $BaselineVersion overlaid with $ExpectedVersion, registry and shortcut agree, and the synthetic data is readable."

if ($LeagueInstaller) {
  $mainFiles = @{}
  foreach ($folder in @($data, (Join-Path $relocated 'data'))) {
    foreach ($file in @('cs2-insight.config.json', 'cs2-insight.db')) {
      $filePath = Join-Path $folder $file
      $mainFiles[$filePath] = (Get-FileHash -LiteralPath $filePath).Hash
    }
  }
  $mainFiles[$installed] = $installedHash
  if ($NativePayloads) { $mainFiles[$mainProgramMarker] = $mainProgramMarkerHash }
  Assert-FileHash $leagueExe $initialLeagueHash
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash
  if ($NativePayloads) {
    Assert-FileHash $leagueProgramMarker $leagueProgramMarkerHash
    foreach ($path in $leagueSavedData.Keys) { Assert-FileHash $path $leagueSavedData[$path] }
  }

  Install-Package $leagueRemove '/S'
  if ((Test-Path -LiteralPath $leagueExe) -or (Test-Path -LiteralPath $leagueUninstallKey)) {
    throw 'League uninstall did not remove its own program and registration.'
  }
  if ((Get-ItemProperty -LiteralPath $uninstallKey).DisplayVersion -ne $ExpectedVersion -or
      (Read-StorageLocator).root -ne $relocated) { throw 'League uninstall changed Main registration or profile.' }
  Assert-FileHash $leagueMarker $leagueMarkerHash
  Assert-FileHash $leagueLocator $leagueLocatorHash
  if ($NativePayloads) {
    Assert-FileHash $leagueProgramMarker $leagueProgramMarkerHash
    foreach ($path in $leagueSavedData.Keys) { Assert-FileHash $path $leagueSavedData[$path] }
    # Clear only the verified test marker before exercising a fresh reinstall.
    [IO.File]::Delete($leagueProgramMarker)
  }
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
  if ($NativePayloads) { foreach ($path in $leagueSavedData.Keys) { Assert-FileHash $path $leagueSavedData[$path] } }
  Test-LeagueWindow
  Write-Host 'TWO_PRODUCT_COEXISTENCE_VERIFIED: both installers rejected overlapping destinations, and each uninstall preserved the other product and the stored data.'
}
