param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][string]$BaselineInstaller
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $PSVersionTable.PSEdition -ne 'Core' -or $PSVersionTable.PSVersion.Major -lt 7 -or
    [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64' -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    !$env:RUNNER_TEMP -or ![IO.Directory]::Exists($env:RUNNER_TEMP)) {
  throw 'Registration repair acceptance requires a disposable x64 GitHub-hosted Windows runner with PowerShell 7.'
}

function Get-FixtureInstallerVersion([string]$Path) {
  if ([IO.Path]::GetFileName($Path) -cnotmatch '^MaxGameStudioLeague_(\d+\.\d+\.\d+(?:-beta\.\d+)?)_x64-setup\.exe$') {
    throw 'Expected a versioned x64 League installer.'
  }
  return $Matches[1]
}

$Installer = (Resolve-Path -LiteralPath $Installer).Path
$BaselineInstaller = (Resolve-Path -LiteralPath $BaselineInstaller).Path
$candidateVersion = Get-FixtureInstallerVersion $Installer
$baselineVersion = Get-FixtureInstallerVersion $BaselineInstaller

$identity = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
$productPath = "Software\$identity"
$arpPath = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$identity"
$product = "HKCU:\$productPath"
$arp = "HKCU:\$arpPath"
$control = Join-Path $env:APPDATA 'MaxGameStudioLeague'
$locator = Join-Path $control 'data-location.txt'
$pending = Join-Path $control 'installer-data-location.txt'
$programName = 'MaxGameStudioLeagueToolbox.exe'
$removeName = 'Uninstall MaxGameStudioLeagueToolbox.exe'
$root = Join-Path $env:RUNNER_TEMP ('mgs-league-registration-repair-' + [Guid]::NewGuid().ToString('N'))
$old = Join-Path $root 'Old Toolbox Program'
$oldExe = Join-Path $old $programName
$parkedExe = Join-Path $root 'Parked Baseline Executable.exe'
$oldRemove = Join-Path $old $removeName
$oldArchive = Join-Path $old 'resources\app.asar'
$profileDir = Join-Path $root 'Existing Toolbox Profile'

function Assert-NoMachineRegistration {
  if ((Test-Path -LiteralPath "HKLM:\$productPath") -or (Test-Path -LiteralPath "HKLM:\$arpPath")) {
    throw 'The current-user fixture must leave machine registration absent.'
  }
}

Assert-NoMachineRegistration
if ((Test-Path -LiteralPath $product) -or (Test-Path -LiteralPath $arp) -or
    (Test-Path -LiteralPath $control) -or (Test-Path -LiteralPath $root)) {
  throw 'Registration repair requires a fresh runner with no existing League fixture or installation.'
}

function Invoke-FixtureInstaller([string]$Path, [string]$Arguments, [int]$ExpectedExit) {
  Write-Host "LEAGUE_REPAIR_ATTEMPT: $([IO.Path]::GetFileName($Path)) $Arguments expectedExit=$ExpectedExit"
  $process = Start-Process -FilePath $Path -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne $ExpectedExit) {
    throw "League setup or removal returned $($process.ExitCode), expected $ExpectedExit."
  }
}

function Assert-RegisteredProgram([string]$Path, [string]$Version, [switch]$Machine) {
  $hive = if ($Machine) { 'HKLM:' } else { 'HKCU:' }
  $scope = if ($Machine) { '/allusers' } else { '/currentuser' }
  $record = Get-ItemProperty -LiteralPath "$hive\$arpPath"
  $command = '"' + (Join-Path $Path $removeName) + '" ' + $scope
  if ((Get-ItemPropertyValue -LiteralPath "$hive\$productPath" -Name InstallLocation) -cne $Path -or
      $record.DisplayVersion -cne $Version -or $record.UninstallString -cne $command -or
      $record.QuietUninstallString -cne "$command /S" -or
      !(Test-Path -LiteralPath (Join-Path $Path $programName) -PathType Leaf) -or
      !(Test-Path -LiteralPath (Join-Path $Path $removeName) -PathType Leaf) -or
      !(Test-Path -LiteralPath (Join-Path $Path 'resources\app.asar') -PathType Leaf)) {
    throw 'Installed League program, version or registration does not match the fixture.'
  }
  if (!$Machine) { Assert-NoMachineRegistration }
}

[IO.Directory]::CreateDirectory($root) | Out-Null
Invoke-FixtureInstaller $BaselineInstaller "/S /currentuser /D=$old" 0
Assert-RegisteredProgram $old $baselineVersion
$originalUninstall = Get-ItemPropertyValue -LiteralPath $arp -Name UninstallString

[IO.Directory]::CreateDirectory($profileDir) | Out-Null
[IO.Directory]::CreateDirectory($control) | Out-Null
$config = Join-Path $profileDir 'base-config.json'
$userFile = Join-Path $profileDir 'keep-user-fixture.txt'
$oldFile = Join-Path $old 'keep-old-program-fixture.txt'
[IO.File]::WriteAllText($config, '{"locale":"zh-CN","fixture":"registration-repair"}')
[IO.File]::WriteAllText($userFile, 'Existing user profile bytes.')
[IO.File]::WriteAllText($oldFile, 'Unexpected old-program folder content must survive.')
[IO.File]::WriteAllText($locator, $profileDir, [Text.Encoding]::Unicode)

# Retain the real previous uninstaller as an execution canary. Its normal
# cleanup removes the executable and app.asar, which must both survive repair.
$saved = @{}
foreach ($file in @(
    $oldExe, $oldArchive, $oldRemove, $oldFile,
    $config, $userFile, $locator
  )) {
  $saved[$file] = (Get-FileHash -LiteralPath $file).Hash
}

function Assert-PreservedFixture([switch]$Machine) {
  if ((Test-Path -LiteralPath $parkedExe) -and (Test-Path -LiteralPath $oldExe)) {
    throw 'Repair populated the intentionally missing executable in the old program directory.'
  }
  foreach ($file in $saved.Keys) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf) -or
        (Get-FileHash -LiteralPath $file).Hash -cne $saved[$file]) {
      throw "Repair or removal changed a preserved fixture file: $file"
    }
  }
  if ([IO.File]::ReadAllText($locator, [Text.Encoding]::Unicode).Trim() -cne $profileDir -or
      (Test-Path -LiteralPath $pending)) {
    throw 'Repair changed or queued a change to the active League profile.'
  }
  if (!$Machine) { Assert-NoMachineRegistration }
}

Add-Type -Path (Join-Path $PSScriptRoot '../tests/AllUsersInstallerNative.cs')
. (Join-Path $PSScriptRoot '../tests/fixture-process-stop.ps1')

function Assert-RecoveryWizard([string]$Destination, [switch]$Machine) {
  if ($Machine) {
    if (!(Test-Path -LiteralPath $Destination -PathType Container) -or
        (Get-ChildItem -LiteralPath $Destination -Force | Select-Object -First 1)) {
      throw 'Machine recovery requires the existing, empty Program Files fixture.'
    }
  } elseif (Test-Path -LiteralPath $Destination) {
    throw 'The current-user recovery wizard must create a new program directory.'
  }
  $scope = if ($Machine) { '/allusers' } else { '/currentuser' }
  $pageText = if ($Machine) {
    '旧版安装不完整。若共用程序目录仍有文件'
  } else {
    '旧版安装不完整。请选择独立的空文件夹'
  }
  $wizard = Start-Process -FilePath $Installer -PassThru
  try {
    $scopeLabel = if ($Machine) { '所有\s*(?:用户|人)' } else { '当前(?:登录)?用户|仅(?:为|限)?我|只(?:为|限)?我|仅限本人' }
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    $picked = $false
    $advancedWelcome = $false
    $advanced = $false
    do {
      if ($wizard.HasExited) { break }
      $controls = [MgsAllUsersNative]::VisibleControls([uint32]$wizard.Id)
      $radio = @($controls | Where-Object { $_.Enabled -and $_.IsRadioButton -and ($_.Text -replace '&', '') -match $scopeLabel })
      $next = @($controls | Where-Object { $_.Enabled -and $_.IsButton -and ($_.Text -replace '&', '') -match '^下一步' })
      if ($radio.Count -eq 1 -and $next.Count -eq 1) {
        if ([MgsAllUsersNative]::CheckedState($radio[0].Handle) -eq 1) {
          if (![MgsAllUsersNative]::Click($next[0].Handle)) { throw 'Could not advance from the scope page.' }
          $advanced = $true
          break
        }
        if (!$picked) { $picked = [MgsAllUsersNative]::Click($radio[0].Handle) }
      } elseif (!$advancedWelcome -and $next.Count -eq 1 -and $controls.Text -match '欢迎') {
        $advancedWelcome = [MgsAllUsersNative]::Click($next[0].Handle)
      }
      Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if (!$advanced) { throw "The recovery wizard did not allow selecting $scope." }

    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
      if ($wizard.HasExited) { break }
      $controls = [MgsAllUsersNative]::VisibleControls([uint32]$wizard.Id)
      $installButton = @($controls | Where-Object { $_.Enabled -and $_.IsButton -and ($_.Text -replace '&', '').Trim() -match '^安装(?:\([A-Z]\))?$' })
      if ($installButton.Count -eq 1 -and $controls.Text -match $pageText) { break }
      Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($wizard.HasExited -or $installButton.Count -ne 1 -or !($controls.Text -match $pageText)) {
      throw "The $scope recovery wizard did not show its Chinese directory page."
    }

    $edits = @($controls | Where-Object IsEdit)
    if ($Machine) {
      $browse = @($controls | Where-Object { $_.Enabled -and $_.IsButton -and $_.Text -match '^浏览' })
      if ($edits.Count -ne 1 -or $edits[0].Enabled -or $edits[0].Text -cne $Destination -or $browse.Count) {
        throw 'Machine recovery exposed a program folder picker or did not lock the dedicated Program Files directory.'
      }
      if (!($controls.Text -like '当前管理员会话仅安装共用程序。*继续使用该帐户的数据。')) {
        throw 'Machine recovery must explain which account keeps its own data before installation.'
      }
    } else {
      $program = @($edits | Where-Object { $_.Text -cne $profileDir })
      if ($edits.Count -ne 2 -or $program.Count -ne 1 -or !$program[0].Enabled -or
          ![MgsAllUsersNative]::SetEditText($program[0].Handle, $Destination)) {
        throw 'User recovery did not retain the profile or accept the new program directory.'
      }
      $editTexts = @([MgsAllUsersNative]::VisibleControls([uint32]$wizard.Id) | Where-Object IsEdit | ForEach-Object Text)
      if ($editTexts -cnotcontains $Destination -or $editTexts -cnotcontains $profileDir) {
        throw 'The directory page did not retain both requested paths.'
      }
    }
    if ((!$Machine -and (Test-Path -LiteralPath $Destination)) -or
        ![MgsAllUsersNative]::Click($installButton[0].Handle)) {
      throw "Could not start the $scope recovery install."
    }
    Write-Host "LEAGUE_RECOVERY_WIZARD_INSTALL_CLICKED: $scope"

    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
      if ($wizard.HasExited) { throw "The $scope recovery wizard exited before Finish with code $($wizard.ExitCode)." }
      $controls = [MgsAllUsersNative]::VisibleControls([uint32]$wizard.Id)
      $failure = $controls.Text -match '无法验证工具箱安装|无法验证 Program Files|工具箱程序文件不完整|无法完成启动用户的数据设置'
      if ($failure) {
        throw "The $scope recovery wizard reported an installation error before Finish. $($failure -join ' ')"
      }
      $finish = @($controls | Where-Object { $_.Enabled -and $_.IsButton -and ($_.Text -replace '&', '').Trim() -match '^完成(?:\([A-Z]\))?$' })
      if ($finish.Count -eq 1) {
        $run = @($controls | Where-Object { $_.Dialog -eq $finish[0].Dialog -and $_.IsCheckBox -and ($_.Text -replace '&', '').Trim() -match '^运行' })
        if ($run.Count -ne 1 -or ![MgsAllUsersNative]::ClearCheckBox($run[0].Handle) -or
            ![MgsAllUsersNative]::Click($finish[0].Handle)) {
          throw 'Could not clear the launch option and safely finish recovery.'
        }
        if (!$wizard.WaitForExit(15000)) { throw 'The recovery wizard did not exit after Finish.' }
        if ($wizard.ExitCode -ne 0) { throw "The $scope recovery wizard returned $($wizard.ExitCode), expected 0." }
        Write-Host "LEAGUE_RECOVERY_WIZARD_VERIFIED: $scope"
        return
      }
      Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for the $scope recovery wizard to reach Finish."
  } finally {
    if (!$wizard.HasExited) { Stop-CiFixtureProcess $wizard }
    $wizard.Dispose()
  }
}

# A complete-looking installation can still have an uninstaller Windows cannot
# start. The stock helper must not continue into copying over that installation.
$uninstallerBytes = [IO.File]::ReadAllBytes($oldRemove)
$failedUpgrade = Join-Path $root 'Blocked old uninstaller'
try {
  [IO.File]::WriteAllText($oldRemove, 'invalid fixture executable')
  Invoke-FixtureInstaller $Installer "/S /currentuser /D=$failedUpgrade" 2
  if ((Test-Path -LiteralPath $failedUpgrade) -or
      (Get-ItemPropertyValue -LiteralPath $product -Name InstallLocation) -cne $old -or
      (Get-ItemPropertyValue -LiteralPath $arp -Name UninstallString) -cne $originalUninstall) {
    throw 'An unlaunchable old uninstaller did not stop setup before copying and registration.'
  }
} finally {
  [IO.File]::WriteAllBytes($oldRemove, $uninstallerBytes)
}
Assert-PreservedFixture
Write-Host 'LEAGUE_OLD_UNINSTALLER_FAILURE_VERIFIED'

foreach ($scenario in @('MissingLocation', 'NonLeadingQuote', 'MissingExecutable')) {
  $repaired = if ($scenario -eq 'MissingLocation') {
    Join-Path $root 'New Program Parent\Repaired Toolbox'
  } else {
    Join-Path $root "Repaired Toolbox $scenario"
  }
  $child = Join-Path $old 'New Child'
  if ($scenario -eq 'MissingLocation') {
    Remove-ItemProperty -LiteralPath $product -Name InstallLocation
    $expectedLocation = $null
    $badUninstall = $originalUninstall
  } else {
    # Make a partial registry again without copying a second baseline.
    $expectedLocation = $old
    if ($scenario -eq 'NonLeadingQuote') {
      # Stock NSIS can extract the quoted path after the invalid leading token.
      $badUninstall = '/unexpected ' + $originalUninstall
    } else {
      # Both registry values remain valid. Move only the exact fixture EXE
      # within the same runner volume to simulate missing program payload.
      $badUninstall = $originalUninstall
      [IO.File]::Move($oldExe, $parkedExe)
      $saved[$parkedExe] = $saved[$oldExe]
      $saved.Remove($oldExe)
    }
    New-Item -Path $product, $arp -Force | Out-Null
    Set-ItemProperty -LiteralPath $product -Name InstallLocation -Value $expectedLocation
    Set-ItemProperty -LiteralPath $arp -Name DisplayVersion -Value $baselineVersion
    Set-ItemProperty -LiteralPath $arp -Name UninstallString -Value $badUninstall
  }
  $destinationArgument = "/D=$repaired"
  if ($baselineVersion -ceq '1.0.0-beta.2') {
    Invoke-FixtureInstaller $BaselineInstaller "/S /currentuser $destinationArgument" 2
    Write-Host "LEAGUE_BETA2_REGISTRATION_FAILURE_REPRODUCED: $scenario"
  }
  if ($scenario -eq 'MissingLocation') {
    Invoke-FixtureInstaller $Installer '/S /currentuser' 2
    Invoke-FixtureInstaller $Installer "/S /currentuser --updated $destinationArgument" 2
  }
  if ($scenario -eq 'MissingExecutable') {
    Invoke-FixtureInstaller $Installer "/S /currentuser --updated $destinationArgument" 2
  }
  Invoke-FixtureInstaller $Installer "/S /currentuser /D=$old" 2
  Invoke-FixtureInstaller $Installer "/S /currentuser /D=$child" 2
  # The key must survive a rejected attempt, but InstallLocation may be absent.
  $actualLocation = (Get-ItemProperty -LiteralPath $product).InstallLocation
  if ($actualLocation -cne $expectedLocation -or
      (Get-ItemPropertyValue -LiteralPath $arp -Name UninstallString) -cne $badUninstall -or
      (Get-ItemPropertyValue -LiteralPath $arp -Name DisplayVersion) -cne $baselineVersion -or
      (Test-Path -LiteralPath $repaired) -or (Test-Path -LiteralPath $child)) {
    throw 'A rejected attempt wrote files or changed the incomplete registration.'
  }
  Assert-PreservedFixture

  if ($scenario -eq 'MissingLocation') {
    Assert-RecoveryWizard $repaired
  } else {
    Invoke-FixtureInstaller $Installer "/S /currentuser $destinationArgument" 0
  }
  Assert-RegisteredProgram $repaired $candidateVersion
  if ($candidateVersion -cne $baselineVersion -and
      (Get-FileHash -LiteralPath (Join-Path $repaired 'resources\app.asar')).Hash -ceq $saved[$oldArchive]) {
    throw 'The repaired program retained the baseline application payload.'
  }
  Assert-PreservedFixture

  # Execute only the new uninstaller from the verified, disjoint directory.
  Invoke-FixtureInstaller (Join-Path $repaired $removeName) '/S /currentuser' 0
  if ((Test-Path -LiteralPath $product) -or (Test-Path -LiteralPath $arp) -or
      (Test-Path -LiteralPath $repaired)) {
    throw 'The repaired program did not uninstall independently.'
  }
  Assert-PreservedFixture
  if ($scenario -eq 'MissingExecutable') {
    [IO.File]::Move($parkedExe, $oldExe)
    $saved[$oldExe] = $saved[$parkedExe]
    $saved.Remove($parkedExe)
  }
  Write-Host "LEAGUE_REGISTRATION_REPAIR_VERIFIED: $scenario"
}

# A stale machine record can be replaced only in the dedicated protected folder,
# and only if it is empty. The fixture is disposable and starts with no HKLM keys.
$machineDir = Join-Path $env:ProgramFiles 'MaxGameStudioLeagueToolbox'
if (Test-Path -LiteralPath $machineDir) { throw 'Expected an empty machine installation fixture.' }
$machineProduct = "HKLM:\$productPath"
[IO.Directory]::CreateDirectory($machineDir) | Out-Null
$occupied = Join-Path $machineDir 'keep-unknown-fixture.txt'
[IO.File]::WriteAllText($occupied, 'keep')
New-Item -Path $machineProduct -Force | Out-Null
Set-ItemProperty -LiteralPath $machineProduct -Name InstallLocation -Value $machineDir
Invoke-FixtureInstaller $Installer "/S /allusers /D=$machineDir" 2
if ([IO.File]::ReadAllText($occupied) -ne 'keep' -or (Test-Path -LiteralPath (Join-Path $machineDir $programName))) {
  throw 'The rejected repair touched an occupied machine directory.'
}
[IO.File]::Delete($occupied)
Assert-RecoveryWizard $machineDir -Machine
Assert-RegisteredProgram $machineDir $candidateVersion -Machine
Assert-PreservedFixture -Machine
$programBytes = (Get-ChildItem -LiteralPath $machineDir -Recurse -File -Force | Measure-Object -Property Length -Sum).Sum
if ($programBytes -le 0 -or $programBytes -gt 350MB) { throw "League exceeded the 350 MiB program budget: $programBytes." }
Write-Host ('LEAGUE_REPAIR_SIZE: ' + (@{
  version = $candidateVersion
  installerBytes = (Get-Item -LiteralPath $Installer).Length
  installerSha256 = (Get-FileHash -LiteralPath $Installer).Hash.ToLowerInvariant()
  programBytes = $programBytes
} | ConvertTo-Json -Compress))

# With complete registration, the same installer takes the ordinary update path.
Invoke-FixtureInstaller $Installer '/S /allusers --updated' 0
Assert-RegisteredProgram $machineDir $candidateVersion -Machine

# Exercise the exact installed executable without opening the data-preservation
# fixture. Use a disposable profile and restore the original locator bytes.
$closeProfile = Join-Path $root 'Close Behavior Profile'
[IO.Directory]::CreateDirectory($closeProfile) | Out-Null
$locatorBytes = [IO.File]::ReadAllBytes($locator)
try {
  [IO.File]::WriteAllText($locator, $closeProfile, [Text.Encoding]::Unicode)
  node (Join-Path $PSScriptRoot 'verify-close.mjs') (Join-Path $machineDir $programName) $closeProfile
  if ($LASTEXITCODE -ne 0) { throw 'Packaged toolbox close verification failed.' }
} finally {
  [IO.File]::WriteAllBytes($locator, $locatorBytes)
}
$remaining = @(Get-Process -Name 'MaxGameStudioLeagueToolbox' -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -ieq (Join-Path $machineDir $programName) })
if ($remaining -or (Get-Process -Name 'cs2-insight-agent-desktop' -ErrorAction SilentlyContinue)) {
  throw 'Closing the League toolbox left its own process running or opened MaxGameStudio.'
}
Invoke-FixtureInstaller (Join-Path $machineDir $removeName) '/S /allusers' 0
if ((Test-Path -LiteralPath $machineDir) -or (Test-Path -LiteralPath $product) -or (Test-Path -LiteralPath $arp)) {
  throw 'Machine repair, update or removal left a program or changed the current-user registration.'
}
Assert-PreservedFixture
Write-Host 'LEAGUE_MACHINE_REGISTRATION_REPAIR_VERIFIED'
