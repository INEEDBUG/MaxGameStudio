param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][string]$BaselineInstaller,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+-beta\.\d+$')][string]$ExpectedVersion,
  [Parameter(Mandatory)][string]$LeagueInstaller,
  [Parameter(Mandatory)][string]$LeagueBaselineInstaller
)

$ErrorActionPreference = 'Stop'
# This test uses real product registration, Program Files and disposable local users.
if (!$IsWindows -or ![Environment]::Is64BitProcess -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
  throw 'All-users acceptance requires an ephemeral x64 GitHub-hosted Windows runner.'
}
$administrator = [Security.Principal.WindowsBuiltInRole]::Administrator
if (!([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole($administrator)) {
  throw 'The hosted runner must be an administrator to exercise silent machine installation.'
}
if ((Get-FileHash -LiteralPath $BaselineInstaller).Hash -ne '583201413a2b31d94f3009766a22dfa6cda65ddb6eba3847d3895eefdf863c1e' -or
    (Get-FileHash -LiteralPath $LeagueBaselineInstaller).Hash -ne 'e8e06419c5209647ca8f03b5357560d4a03409bbcc2ae8db93cf7dbae044f61b') {
  throw 'The published release-baseline identity did not match the expected published installers.'
}
if ([IO.Path]::GetFileName($Installer) -cne "MaxGameStudio_$($ExpectedVersion)_x64-setup.exe" -or
    [IO.Path]::GetFileName($LeagueInstaller) -cnotmatch '^MaxGameStudioLeague_(\d+\.\d+\.\d+-beta\.\d+)_x64-setup\.exe$') {
  throw 'Expected both versioned x64 beta installers.'
}
$leagueVersion = $Matches[1]
Add-Type -Path (Join-Path $PSScriptRoot 'AllUsersInstallerNative.cs')

$root = Join-Path $env:RUNNER_TEMP 'mgs-all-users-acceptance'
$previous = Join-Path $env:RUNNER_TEMP 'mgs-upgrade-acceptance'
$leagueID = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
$arp = 'Software\Microsoft\Windows\CurrentVersion\Uninstall'
$storageKey = 'HKCU:\Software\MaxGameStudio\Storage'
$leagueLocator = Join-Path $env:APPDATA 'MaxGameStudioLeague\data-location.txt'
$leagueSelection = Join-Path $env:APPDATA 'MaxGameStudioLeague\installer-data-location.txt'
$main = @{
  Name = 'Main'; Version = $ExpectedVersion; Installer = $Installer; Limit = 120MB
  Product = 'Software\cs2insightagent\MaxGameStudio'; Arp = "$arp\MaxGameStudio"; Location = '(default)'
  Exe = 'cs2-insight-agent-desktop.exe'; Remove = 'uninstall.exe'; Shortcut = 'MaxGameStudio'; MachineFlag = '/ALLUSERS'
  Default = (Join-Path $env:ProgramFiles 'MaxGameStudio'); Old = (Join-Path $root 'Main per-user program')
}
$league = @{
  Name = 'League'; Version = $leagueVersion; Installer = $LeagueInstaller; Limit = 350MB
  Product = "Software\$leagueID"; Arp = "$arp\$leagueID"; Location = 'InstallLocation'
  Exe = 'MaxGameStudioLeagueToolbox.exe'; Remove = 'Uninstall MaxGameStudioLeagueToolbox.exe'; Shortcut = '英雄联盟工具箱'; MachineFlag = '/allusers'
  Default = (Join-Path $env:ProgramFiles 'MaxGameStudioLeagueToolbox'); Old = (Join-Path $root 'League per-user program')
}
$shell = New-Object -ComObject WScript.Shell

function Invoke-Installer([string]$Exe, [string]$Arguments, [switch]$Rejected) {
  $process = Start-Process -FilePath $Exe -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
  if (($Rejected -and $process.ExitCode -eq 0) -or (!$Rejected -and $process.ExitCode -ne 0)) {
    throw "Unexpected installer result for $([IO.Path]::GetFileName($Exe)): exit $($process.ExitCode)."
  }
}

function Assert-Installed($Product, [string]$Hive, [string]$Version, [string]$Directory) {
  $record = Get-ItemProperty -LiteralPath "$($Hive):\$($Product.Arp)"
  $path = Get-ItemPropertyValue -LiteralPath "$($Hive):\$($Product.Product)" -Name $Product.Location
  $expectedRemove = Join-Path $Directory $Product.Remove
  $expectedExe = Join-Path $Directory $Product.Exe
  if ($record.DisplayVersion -cne $Version -or
      ($Product.Name -eq 'Main' -and ([string]$record.InstallLocation).Trim('"') -ne $Directory) -or
      $path.Trim('"') -ne $Directory -or
      $record.UninstallString -notmatch '^"([^"]+\.exe)"(?:\s|$)' -or $Matches[1] -ne $expectedRemove -or
      !(Test-Path -LiteralPath $expectedExe -PathType Leaf) -or !(Test-Path -LiteralPath $expectedRemove -PathType Leaf)) {
    throw "$($Product.Name) installer registration, uninstaller or program files did not match the expected $Hive installation."
  }
}

function Assert-CommonShortcuts($Product) {
  $target = Join-Path $Product.Default $Product.Exe
  $desktop = Join-Path ([Environment]::GetFolderPath('CommonDesktopDirectory')) "$($Product.Shortcut).lnk"
  $menu = @(Get-ChildItem -LiteralPath ([Environment]::GetFolderPath('CommonPrograms')) -Filter "$($Product.Shortcut).lnk" -File -Recurse |
    Where-Object { $shell.CreateShortcut($_.FullName).TargetPath -eq $target })
  if (!(Test-Path -LiteralPath $desktop -PathType Leaf) -or $shell.CreateShortcut($desktop).TargetPath -ne $target -or $menu.Count -ne 1) {
    throw "$($Product.Name) common Desktop and Start Menu shortcuts did not target the machine program."
  }
}

function Assert-RetiredUserInstall($Product) {
  if ((Test-Path -LiteralPath "HKCU:\$($Product.Arp)") -or (Test-Path -LiteralPath "HKCU:\$($Product.Product)") -or
      (Test-Path -LiteralPath (Join-Path $Product.Old $Product.Exe)) -or
      (Test-Path -LiteralPath (Join-Path $Product.Old $Product.Remove))) {
    throw "$($Product.Name) left prior current-user registration or owned entry points."
  }
  foreach ($folder in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {
    $stale = Get-ChildItem -LiteralPath $folder -Filter "$($Product.Shortcut).lnk" -File -Recurse |
      Where-Object { $shell.CreateShortcut($_.FullName).TargetPath -eq (Join-Path $Product.Old $Product.Exe) } |
      Select-Object -First 1
    if ($stale) { throw "$($Product.Name) left its old current-user shortcut." }
  }
}

function Assert-Hash([string]$Path, [string]$Expected) {
  if ((Get-FileHash -LiteralPath $Path).Hash -cne $Expected) {
    throw "Synthetic fixture or installed file changed unexpectedly: $Path"
  }
}

function Test-ScopePage($Product, $User, [string]$InstallerPath) {
  $manifest = [xml][MgsAllUsersNative]::ReadManifest($InstallerPath)
  $level = $manifest.SelectSingleNode("//*[local-name()='requestedExecutionLevel']")
  if (!$level -or $level.level -cne 'asInvoker') {
    throw "$($Product.Name) installer must start at the caller's level before the scope choice."
  }
  $gui = Start-StandardProcess $User $InstallerPath (Split-Path -Parent $InstallerPath) -Visible
  try {
    Assert-StandardProcessOwner $gui $User
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $advancedWelcome = $false
    do {
      if ($gui.HasExited) { throw "$($Product.Name) installer exited before the scope page." }
      $controls = [MgsAllUsersNative]::VisibleControls([uint32]$gui.Id)
      $radios = @($controls | Where-Object IsRadioButton)
      $current = $radios | Where-Object { ($_.Text -replace '&', '') -match '当前(?:登录)?用户|仅(?:为|限)?我|只(?:为|限)?我|仅限本人' } | Select-Object -First 1
      $everybody = $radios | Where-Object { ($_.Text -replace '&', '') -match '所有\s*(?:用户|人)' } | Select-Object -First 1
      if ($current -and $everybody -and $current.Handle -ne $everybody.Handle) {
        if (!$current.Enabled -or !$everybody.Enabled) { throw "$($Product.Name) has a disabled installation scope." }
        foreach ($pair in @(@{Pick = $everybody; Other = $current}, @{Pick = $current; Other = $everybody})) {
          if (![MgsAllUsersNative]::Click($pair.Pick.Handle)) { throw 'Could not select an enabled installation scope.' }
          $selectionDeadline = [DateTime]::UtcNow.AddSeconds(3)
          do {
            $selected = [MgsAllUsersNative]::CheckedState($pair.Pick.Handle) -eq 1 -and
              [MgsAllUsersNative]::CheckedState($pair.Other.Handle) -eq 0
            if (!$selected) { Start-Sleep -Milliseconds 100 }
          } while (!$selected -and [DateTime]::UtcNow -lt $selectionDeadline)
          if (!$selected) { throw "$($Product.Name) ignored the installation scope selection." }
        }
        Write-Host "INSTALLER_SCOPE_UI: $($Product.Name) allowed both Chinese scope choices under the verified standard-user token and asInvoker manifest."
        return
      }
      if (!$advancedWelcome -and ($controls | Where-Object { $_.Text -match '欢迎' } | Select-Object -First 1)) {
        $next = $controls | Where-Object { $_.IsButton -and ($_.Text -replace '&', '') -match '^下一步' } | Select-Object -First 1
        if ($next) { $advancedWelcome = [MgsAllUsersNative]::Click($next.Handle) }
      }
      Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$($Product.Name) did not display both Chinese scope choices."
  } finally {
    # Cancel before selecting a scope or reaching the installation action.
    if (!$gui.HasExited) { Stop-Process -InputObject $gui -Force }
    if (!$gui.WaitForExit(5000)) { throw 'Could not close the fixture installer.' }
  }
}

. (Join-Path $PSScriptRoot 'fixture-process-stop.ps1')
function Stop-TestProcess([Diagnostics.Process]$Process) {
  Stop-CiFixtureProcess $Process -TryClose -GracefulMilliseconds 3000
}

function Wait-Fixture([string]$Failure, [scriptblock]$Condition, [Diagnostics.Process]$Process) {
  $until = [DateTime]::UtcNow.AddSeconds(45)
  do {
    if ($Process.HasExited) { throw "$Failure Process exited early." }
    $result = & $Condition
    if ($result) { return $result }
    Start-Sleep -Milliseconds 300
  } while ([DateTime]::UtcNow -lt $until)
  throw $Failure
}

function New-FixtureUser([string]$Role) {
  # Keep the generated credential in SecureString and never pass it as an argument.
  $name = 'MGSci-' + $Role + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
  $password = [Security.SecureString]::new()
  foreach ($character in [char[]]'Ar2!') { $password.AppendChar($character) }
  $alphabet = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
  for ($i = 0; $i -lt 28; $i++) {
    $password.AppendChar($alphabet[[Security.Cryptography.RandomNumberGenerator]::GetInt32($alphabet.Length)])
  }
  $password.MakeReadOnly()
  $user = New-LocalUser -Name $name -Password $password
  $accounts.Add($user.SID)
  $group = Get-LocalGroup -SID 'S-1-5-32-545'
  if (!(Get-LocalGroupMember -Group $group | Where-Object { $_.SID -eq $user.SID })) {
    Add-LocalGroupMember -Group $group -Member $user
  }
  return @{ Name = $name; SID = $user.SID.Value; Credential = [PSCredential]::new("$env:COMPUTERNAME\$name", $password) }
}

function Assert-StandardProcessOwner([Diagnostics.Process]$Process, $User) {
  if (![MgsAllUsersNative]::IsStandardUser($Process.Handle, $User.SID)) {
    throw 'The test process did not start under the expected standard-user token.'
  }
}

function Start-StandardProcess($User, [string]$Exe, [string]$Directory, [string]$Arguments = '', [switch]$Visible) {
  $info = [Diagnostics.ProcessStartInfo]::new($Exe, $Arguments)
  $info.WorkingDirectory = $Directory
  $info.UseShellExecute = $false
  $info.UserName = $User.Name
  $info.Domain = $env:COMPUTERNAME
  $info.Password = $User.Credential.Password
  $info.LoadUserProfile = $true
  if (!$Visible) {
    $info.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $info.CreateNoWindow = $true
  }
  # Keep Environment untouched so Windows constructs it from the target profile.
  return [Diagnostics.Process]::Start($info)
}

function Test-BusyMainMachine([Diagnostics.Process]$Running) {
  $executable = Join-Path $main.Default $main.Exe
  $extensions = @(Get-ChildItem -LiteralPath (Join-Path $main.Default 'python\Lib\site-packages\demoparser2') -Filter '*.pyd' -File)
  if ($extensions.Count -ne 1) { throw 'Expected a single installed Main parser extension.' }
  $parser = $extensions[0].FullName
  $exeHash = (Get-FileHash -LiteralPath $executable).Hash
  $parserHash = (Get-FileHash -LiteralPath $parser).Hash

  Invoke-Installer $Installer '/S /UPDATE /ALLUSERS' -Rejected
  Assert-Hash $executable $exeHash
  Assert-Hash $parser $parserHash
  if ($Running.HasExited) { throw 'The rejected machine updater stopped the other user''s Main process.' }
  Assert-Installed $main HKLM $main.Version $main.Default

  Invoke-Installer (Join-Path $main.Default $main.Remove) '/S /ALLUSERS' -Rejected
  Assert-Hash $executable $exeHash
  Assert-Hash $parser $parserHash
  if ($Running.HasExited) { throw 'The rejected machine uninstaller stopped the other user''s Main process.' }
  Assert-Installed $main HKLM $main.Version $main.Default
  Write-Host 'BUSY_MACHINE_MAIN_VERIFIED: updater and removal both refused while another standard user ran Main, preserving its process, executable and parser.'
}

function Open-StandardProfiles($User, [switch]$CheckMachineBusy) {
  $standardMain = Start-StandardProcess $User (Join-Path $main.Default $main.Exe) $main.Default
  try {
    Assert-StandardProcessOwner $standardMain $User
    $hku = "Registry::HKEY_USERS\$($User.SID)"
    $privateMain = Wait-Fixture 'Main did not initialize private storage for the standard user.' {
      $value = Get-ItemPropertyValue -LiteralPath "$hku\Software\MaxGameStudio\Storage" -Name LocationV1 -ErrorAction SilentlyContinue
      if ($value) {
        $layout = $value | ConvertFrom-Json
        if ($layout.root -and (Test-Path -LiteralPath (Join-Path $layout.root 'temp') -PathType Container)) { $layout.root }
      }
    } $standardMain
    if ($CheckMachineBusy) { Test-BusyMainMachine $standardMain }
    foreach ($product in $main, $league) {
      if (Test-Path -LiteralPath "$hku\$($product.Arp)") {
        throw 'Starting a machine program created a per-user installer registration.'
      }
    }
  } finally {
    Stop-TestProcess $standardMain
  }

  $standardLeague = Start-StandardProcess $User (Join-Path $league.Default $league.Exe) $league.Default
  try {
    Assert-StandardProcessOwner $standardLeague $User
    $profileKey = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$($User.SID)"
    $profileImage = [Environment]::ExpandEnvironmentVariables((Get-ItemPropertyValue -LiteralPath $profileKey -Name ProfileImagePath))
    $privatePointer = Join-Path $profileImage 'AppData\Roaming\MaxGameStudioLeague\data-location.txt'
    $privateLeague = Wait-Fixture 'League did not initialize a private standard-user profile.' {
      if (Test-Path -LiteralPath $privatePointer -PathType Leaf) {
        $target = [IO.File]::ReadAllText($privatePointer, [Text.Encoding]::Unicode).Trim()
        if ($target -and (Test-Path -LiteralPath (Join-Path $target 'LeagueAkari.db') -PathType Leaf)) { $target }
      }
    } $standardLeague
    $expectedMainProfile = Join-Path $profileImage 'AppData\Roaming\MaxGameStudioData'
    if ($privateMain -ne $expectedMainProfile -or
        $privateLeague -ne (Join-Path (Split-Path -Parent $privatePointer) 'Profile') -or
        $privateMain -eq $mainProfile -or $privateLeague -eq $leagueProfile -or $privateMain -eq $privateLeague) {
      throw 'The standard-user installation reused another profile.'
    }
    foreach ($product in $main, $league) {
      if (Test-Path -LiteralPath "Registry::HKEY_USERS\$($User.SID)\$($product.Arp)") {
        throw 'Starting a machine program created a per-user installer registration.'
      }
    }
  } finally {
    Stop-TestProcess $standardLeague
  }
  return @($privateMain, $privateLeague)
}

function Test-StandardScopePages($User) {
  # The release artifacts may be under a runner-private directory. Stage the
  # exact GUI fixtures under Program Files for standard-user read and execute.
  $folder = Join-Path $env:ProgramFiles ('MGS-Scope-Acceptance-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $folder | Out-Null
  $owned = [Collections.Generic.List[string]]::new()
  try {
    foreach ($product in $main, $league) {
      $copy = Join-Path $folder ([IO.Path]::GetFileName($product.Installer))
      $owned.Add($copy)
      Copy-Item -LiteralPath $product.Installer -Destination $copy -ErrorAction Stop
      Assert-Hash $copy ((Get-FileHash -LiteralPath $product.Installer).Hash)
      Test-ScopePage $product $User $copy
    }
  } finally {
    foreach ($copy in $owned) {
      if (Test-Path -LiteralPath $copy -PathType Leaf) { Remove-Item -LiteralPath $copy -Force }
    }
    if (!(Get-ChildItem -LiteralPath $folder -Force | Select-Object -First 1)) {
      Remove-Item -LiteralPath $folder
    }
  }
}

function Test-StandardUserAccess($User, [string[]]$Own, [string[]]$Other) {
  # The encoded script contains only fixture paths and a SID. The password is
  # passed only through PSCredential to the Windows logon API.
  $ownPaths = ($Own | ForEach-Object { "'" + $_.Replace("'", "''") + "'" }) -join ','
  $otherPaths = ($Other | ForEach-Object { "'" + $_.Replace("'", "''") + "'" }) -join ','
  $probe = @'
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if ($identity.User.Value -ne '@USER_SID@' -or
    ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  exit 10
}
$profileKey = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$($identity.User.Value)"
$profile = [Environment]::ExpandEnvironmentVariables((Get-ItemPropertyValue -LiteralPath $profileKey -Name ProfileImagePath))
if ($env:USERPROFILE -ne $profile -or $env:APPDATA -ne (Join-Path $profile 'AppData\Roaming')) { exit 14 }
foreach ($file in @(@OWN_PATHS@)) {
  [IO.File]::WriteAllText($file, 'synthetic private profile')
  if ([IO.File]::ReadAllText($file) -cne 'synthetic private profile') { exit 11 }
}
foreach ($file in @(@OTHER_PATHS@)) {
  try {
    $handle = [IO.File]::OpenRead($file)
    $handle.Dispose()
    exit 12
  } catch {
    if ($_.Exception.GetBaseException() -isnot [UnauthorizedAccessException]) { exit 13 }
  }
}
exit 0
'@
  $probe = $probe.Replace('@USER_SID@', $User.SID).Replace('@OWN_PATHS@', $ownPaths).Replace('@OTHER_PATHS@', $otherPaths)
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($probe))
  $windowsPowerShell = "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
  $process = Start-StandardProcess $User $windowsPowerShell "$env:WINDIR\System32" "-NoLogo -NoProfile -NonInteractive -EncodedCommand $encoded"
  if (!$process.WaitForExit(30000)) {
    Stop-TestProcess $process
    throw 'Timed out validating private profile access.'
  }
  if ($process.ExitCode -ne 0) { throw "Standard-user profile access check failed (exit $($process.ExitCode))." }
}

function Invoke-MediumFixture([string]$Label, [string]$Exe = '', [string]$Arguments = '', [int]$Seconds = 12) {
  $resultPath = Join-Path $root "$Label-result.json"
  if (Test-Path -LiteralPath $resultPath) { throw 'An unknown medium fixture result already exists.' }
  $source = @'
$ErrorActionPreference = 'Stop'
$caller = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$groups = & "$env:WINDIR\System32\whoami.exe" /groups /fo csv /nh
$medium = ($groups -match '"S-1-16-(8192|8448)"') -and !($groups -match '"S-1-16-(12288|16384)"')
$code = $null
if ('@EXE@' -ne '' -and $medium -and $caller -eq '@SID@') {
  $process = Start-Process -FilePath '@EXE@' -ArgumentList '@ARGS@' -WindowStyle Hidden -Wait -PassThru
  $code = $process.ExitCode
}
@{sameUser = ($caller -eq '@SID@'); medium = [bool]$medium; exitCode = $code} |
  ConvertTo-Json -Compress | Set-Content -LiteralPath '@RESULT@' -Encoding utf8
'@
  $expectedSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $source = $source.Replace('@EXE@', $Exe.Replace("'", "''")).Replace('@ARGS@', $Arguments.Replace("'", "''")).
    Replace('@SID@', $expectedSid).Replace('@RESULT@', $resultPath.Replace("'", "''"))
  $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($source))
  $desktopShell.ShellExecute("$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe",
    "-NoLogo -NoProfile -NonInteractive -EncodedCommand $encoded", $root, 'open', 0)
  $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
  do {
    if (Test-Path -LiteralPath $resultPath -PathType Leaf) {
      $data = Get-Content -LiteralPath $resultPath -Raw
      if ($data) { return ($data | ConvertFrom-Json) }
    }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  return $null
}

function Invoke-ScopeTransition($Product, [switch]$Rejected) {
  $arguments = "/S $($Product.MachineFlag)"
  if (!$mediumCaller) {
    Invoke-Installer $Product.Installer $arguments -Rejected:$Rejected
    return
  }
  $label = "$($Product.Name)-" + $(if ($Rejected) { 'blocked' } else { 'upgrade' })
  $result = Invoke-MediumFixture $label (Resolve-Path -LiteralPath $Product.Installer).Path $arguments 240
  if (!$result -or !$result.medium -or !$result.sameUser -or $null -eq $result.exitCode) {
    throw "$($Product.Name) did not complete the machine transition from the verified medium caller."
  }
  if (($Rejected -and $result.exitCode -eq 0) -or (!$Rejected -and $result.exitCode -ne 0)) {
    throw "$($Product.Name) machine transition from the medium caller returned $($result.exitCode)."
  }
}

function Write-InstalledSize($Product) {
  $files = Get-ChildItem -LiteralPath $Product.Default -Recurse -File -Force
  [long]$programBytes = ($files | Measure-Object Length -Sum).Sum
  [long]$installerBytes = (Get-Item -LiteralPath $Product.Installer).Length
  $sample = @{ product = $Product.Name; installerBytes = $installerBytes; programBytes = $programBytes }
  Write-Host ('ALL_USERS_PACKAGE_SIZE: ' + ($sample | ConvertTo-Json -Compress))
  if ($programBytes -le 0 -or $programBytes -gt $Product.Limit) {
    throw "$($Product.Name) exceeded its installed program budget ($programBytes bytes)."
  }
}

$mainProfile = Join-Path $root 'Main synthetic profile'
$mainData = Join-Path $mainProfile 'data'
$leagueProfile = Join-Path $root 'League synthetic profile'
foreach ($path in @($root, $main.Default, $league.Default, "HKCU:\$($main.Arp)",
    "HKLM:\$($main.Arp)", "HKLM:\$($league.Arp)", "HKLM:\$($main.Product)", "HKLM:\$($league.Product)")) {
  if (Test-Path -LiteralPath $path) { throw 'The runner already contains an unexpected suite location or registration.' }
}

# The immediately preceding coexistence test leaves only its exact League
# program installed. Retire that program while keeping its fixture profile.
$priorLeagueProgram = Join-Path $previous 'Toolbox Program'
$priorLeagueProfile = Join-Path $previous 'Toolbox Profile'
if (Test-Path -LiteralPath "HKCU:\$($league.Arp)") {
  Assert-Installed $league HKCU $leagueVersion $priorLeagueProgram
  Invoke-Installer (Join-Path $priorLeagueProgram $league.Remove) '/S /currentuser'
  if ((Test-Path -LiteralPath "HKCU:\$($league.Arp)") -or ! (Test-Path -LiteralPath $priorLeagueProfile -PathType Container)) {
    throw 'The earlier fixture uninstaller did not preserve its profile and remove its own registration.'
  }
} elseif (Test-Path -LiteralPath "HKCU:\$($league.Product)") {
  throw 'Found an unrecognized current-user League registration.'
}
if ((Test-Path -LiteralPath $leagueLocator) -and
    [IO.File]::ReadAllText($leagueLocator, [Text.Encoding]::Unicode).Trim() -ne $priorLeagueProfile) {
  throw 'Found an unrecognized League data locator.'
}
if (Test-Path -LiteralPath $storageKey) {
  $prior = Get-ItemPropertyValue -LiteralPath $storageKey -Name LocationV1 -ErrorAction SilentlyContinue
  if ($prior -and !([IO.Path]::GetFullPath(($prior | ConvertFrom-Json).root)).StartsWith($previous + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Found an unrecognized Main data locator.'
  }
}
New-Item -ItemType Directory -Path $root, $mainData | Out-Null

# NSIS parses /D from the end of the native command line, including spaces.
Invoke-Installer $BaselineInstaller "/S /D=$($main.Old)"
Assert-Installed $main HKCU '3.1.4' $main.Old
Invoke-Installer $LeagueBaselineInstaller "/S /D=$($league.Old)"
Assert-Installed $league HKCU '1.0.0-beta.1' $league.Old

# Replay the real data-page handoff for the baseline, then let the packaged
# app commit the new dedicated fixture profile before any scope transition.
New-Item -ItemType Directory -Path (Split-Path -Parent $leagueSelection) -Force | Out-Null
[IO.File]::WriteAllText($leagueSelection, $leagueProfile, [Text.Encoding]::Unicode)
$oldLeagueApp = Start-Process -FilePath (Join-Path $league.Old $league.Exe) -WindowStyle Hidden -PassThru
$leagueDb = Join-Path $leagueProfile 'LeagueAkari.db'
try {
  $null = Wait-Fixture 'The packaged League baseline did not open its chosen fixture profile.' {
    (Test-Path -LiteralPath $leagueDb -PathType Leaf) -and
    [IO.File]::ReadAllText($leagueLocator, [Text.Encoding]::Unicode).Trim() -eq $leagueProfile
  } $oldLeagueApp
} finally {
  Stop-TestProcess $oldLeagueApp
}

$mainConfig = Join-Path $mainData 'cs2-insight.config.json'
$mainDb = Join-Path $mainData 'cs2-insight.db'
Set-Content -LiteralPath $mainConfig -Encoding utf8 -NoNewline -Value '{"locale":"zh-CN","close_action":"exit"}'
$leagueConfig = Join-Path $leagueProfile 'base-config.json'
if (!(Test-Path -LiteralPath $leagueConfig)) { Set-Content -LiteralPath $leagueConfig -Encoding utf8 -NoNewline -Value '{}' }
$fixture = Join-Path $root 'sqlite-fixture.py'
@'
import sqlite3
import sys
from pathlib import Path

mode, filename, label = sys.argv[1:]
database = Path(filename)
if mode == "seed":
    with sqlite3.connect(database) as connection:
        connection.execute("create table mgs_all_users_acceptance (kind text)")
        connection.execute("insert into mgs_all_users_acceptance values (?)", (label,))
    connection.execute("pragma wal_checkpoint(truncate)")
else:
    connection = sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)
    assert connection.execute("pragma quick_check").fetchone() == ("ok",)
    assert connection.execute("select kind from mgs_all_users_acceptance").fetchone() == (label,)
connection.close()
'@ | Set-Content -LiteralPath $fixture -Encoding utf8
$oldPython = Join-Path $main.Old 'python\python.exe'
foreach ($kind in 'Main', 'League') {
  $database = if ($kind -eq 'Main') { $mainDb } else { $leagueDb }
  & $oldPython -I -B $fixture seed $database $kind
  if ($LASTEXITCODE -ne 0) { throw "Could not seed the $kind SQLite fixture." }
}
$layout = @{root = $mainProfile; mode = 'unified'; required_paths = @($mainData)} | ConvertTo-Json -Compress
New-Item -Path $storageKey -Force | Out-Null
Set-ItemProperty -LiteralPath $storageKey -Name LocationV1 -Value $layout

$expectedFiles = @{}
foreach ($product in $main, $league) {
  $sentinel = Join-Path $product.Old 'keep-unowned-fixture.txt'
  [IO.File]::WriteAllText($sentinel, "$($product.Name) unrelated synthetic file")
  $expectedFiles[$sentinel] = (Get-FileHash -LiteralPath $sentinel).Hash
}
foreach ($path in $mainConfig, $mainDb, $leagueConfig, $leagueDb, $leagueLocator) {
  $expectedFiles[$path] = (Get-FileHash -LiteralPath $path).Hash
}
$oldMainHash = (Get-FileHash -LiteralPath (Join-Path $main.Old $main.Exe)).Hash
$oldLeagueHash = (Get-FileHash -LiteralPath (Join-Path $league.Old 'resources\app.asar')).Hash

$accounts = [Collections.Generic.List[Security.Principal.SecurityIdentifier]]::new()
try {
  $first = New-FixtureUser 'A'
  $second = New-FixtureUser 'B'
  Test-StandardUserAccess $first @() @()
  Test-StandardUserAccess $second @() @()
  Test-StandardScopePages $first

  # Use the actual Explorer token only when the existing runner policy permits
  # unattended UAC. Preserve runner security settings and report its limits.
  $desktopShell = $null
  $mediumCaller = $false
  $policy = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
  $appInfo = Get-CimInstance Win32_Service -Filter "Name='Appinfo'"
  if ($policy.EnableLUA -eq 1 -and $policy.ConsentPromptBehaviorAdmin -eq 0 -and $appInfo -and $appInfo.StartMode -ne 'Disabled') {
    try {
      $sessionShell = New-Object -ComObject Shell.Application
      $window = 0
      $desktop = $sessionShell.Windows().FindWindowSW(0, 0, 8, [ref]$window, 1)
      $desktopShell = $desktop.Document.Application
      if ($desktopShell) {
        $token = Invoke-MediumFixture 'scope-token'
        $mediumCaller = [bool]($token -and $token.sameUser -and $token.medium)
      }
    } catch {
      Write-Host 'MEDIUM_SCOPE_PROBE: Explorer automation did not supply a usable fixture token.'
    }
  }
  if ($mediumCaller) {
    Write-Host 'MEDIUM_SCOPE_PROBE: verified caller SID and medium integrity using existing UAC policy.'
  } else {
    Write-Host 'MEDIUM_SCOPE_PROBE: hosted runner cannot prove interactive UAC or medium-origin retirement. Testing the direct-admin preservation route.'
  }

  foreach ($product in $main, $league) {
    $baselineVersion = if ($product.Name -eq 'Main') { '3.1.4' } else { '1.0.0-beta.1' }
    $block = Join-Path $product.Default 'foreign-fixture.txt'
    New-Item -ItemType Directory -Path $product.Default | Out-Null
    [IO.File]::WriteAllText($block, 'occupied machine destination')
    $blockHash = (Get-FileHash -LiteralPath $block).Hash
    Invoke-ScopeTransition $product -Rejected
    Assert-Installed $product HKCU $baselineVersion $product.Old
    Assert-Hash $block $blockHash
    $contents = @(Get-ChildItem -LiteralPath $product.Default -Force)
    if ((Test-Path -LiteralPath "HKLM:\$($product.Arp)") -or
        (Test-Path -LiteralPath "HKLM:\$($product.Product)") -or $contents.Count -ne 1) {
      throw "$($product.Name) changed a machine destination after rejecting it."
    }
    Remove-Item -LiteralPath $block
    Remove-Item -LiteralPath $product.Default
    Invoke-ScopeTransition $product
    Assert-Installed $product HKLM $product.Version $product.Default
    Assert-CommonShortcuts $product
    if ($mediumCaller) {
      Assert-RetiredUserInstall $product
    } else {
      Assert-Installed $product HKCU $baselineVersion $product.Old
    }
    Write-InstalledSize $product
  }

  $machineMain = Join-Path $main.Default $main.Exe
  $machineLeague = Join-Path $league.Default $league.Exe
  $machineLeagueArchive = Join-Path $league.Default 'resources\app.asar'
  $machineMainHash = (Get-FileHash -LiteralPath $machineMain).Hash
  $machineLeagueHash = (Get-FileHash -LiteralPath $machineLeague).Hash
  $machineLeagueArchiveHash = (Get-FileHash -LiteralPath $machineLeagueArchive).Hash
  if ($machineMainHash -eq $oldMainHash -or $machineLeagueArchiveHash -eq $oldLeagueHash) {
    throw 'The machine programs did not contain the candidate application payloads.'
  }

  # Use the real pinned Tauri updater command, which supplies scope but no /D.
  Invoke-Installer $Installer '/P /UPDATE /ALLUSERS'
  Invoke-Installer $Installer "/S /UPDATE /ALLUSERS /D=$($main.Default)"
  Invoke-Installer $LeagueInstaller "/S /allusers --updated /D=$($league.Default)"
  foreach ($product in $main, $league) {
    Assert-Installed $product HKLM $product.Version $product.Default
    Assert-CommonShortcuts $product
    if ($mediumCaller) { Assert-RetiredUserInstall $product }
  }
  if (!$mediumCaller) {
    Assert-Installed $main HKCU '3.1.4' $main.Old
    Assert-Installed $league HKCU '1.0.0-beta.1' $league.Old
    Assert-Hash (Join-Path $main.Old $main.Exe) $oldMainHash
    Assert-Hash (Join-Path $league.Old 'resources\app.asar') $oldLeagueHash
  }
  Assert-Hash $machineMain $machineMainHash
  Assert-Hash $machineLeague $machineLeagueHash
  Assert-Hash $machineLeagueArchive $machineLeagueArchiveHash
  foreach ($path in $expectedFiles.Keys) { Assert-Hash $path $expectedFiles[$path] }
  if ((Get-ItemPropertyValue -LiteralPath $storageKey -Name LocationV1) -cne $layout) {
    throw 'The machine transition or update changed the initiating user Main data locator.'
  }

  $python = Join-Path $main.Default 'python\python.exe'
  foreach ($kind in 'Main', 'League') {
    $database = if ($kind -eq 'Main') { $mainDb } else { $leagueDb }
    & $python -I -B $fixture verify $database $kind
    if ($LASTEXITCODE -ne 0) { throw "The preserved $kind SQLite fixture was unreadable." }
  }
  Write-Host 'MACHINE_SCOPE_UPDATE_VERIFIED: both machine installations reached the expected paths, shortcuts and versions and kept the original data.'
  if ($mediumCaller) {
    Write-Host 'ORIGINAL_USER_RETIREMENT_VERIFIED: both products retired the prior HKCU entries and owned entry points after success, retaining unknown files.'
  } else {
    Write-Host 'DIRECT_ADMIN_DEFERRAL_VERIFIED: both products preserved prior HKCU entries and owned files during direct-admin installation.'
  }

  $firstRoots = @(Open-StandardProfiles $first -CheckMachineBusy)
  $secondRoots = @(Open-StandardProfiles $second)
  if ($firstRoots.Count -ne 2 -or $secondRoots.Count -ne 2 -or
      @(@($firstRoots + $secondRoots) | Sort-Object -Unique).Count -ne 4) {
    throw 'The two standard users did not receive four separate application profiles.'
  }
  $firstMarkers = @($firstRoots | ForEach-Object { Join-Path $_ 'mgs-private-user-fixture.txt' })
  $secondMarkers = @($secondRoots | ForEach-Object { Join-Path $_ 'mgs-private-user-fixture.txt' })
  Test-StandardUserAccess $first $firstMarkers @()
  foreach ($path in $firstMarkers) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw 'A private fixture file was not created by its owner.' }
  }
  Test-StandardUserAccess $second $secondMarkers $firstMarkers
  foreach ($path in $secondMarkers) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw 'A private fixture file was not created by its owner.' }
  }
  Test-StandardUserAccess $first $firstMarkers $secondMarkers
  foreach ($path in @($firstMarkers + $secondMarkers)) { $expectedFiles[$path] = (Get-FileHash -LiteralPath $path).Hash }
  Write-Host 'STANDARD_USER_PROFILES_VERIFIED: both products ran under both expected standard SIDs, each user could write their profiles and Windows denied cross-account reads.'
} finally {
  foreach ($account in $accounts) { Remove-LocalUser -SID $account }
}

function Remove-MachineFixture($Product) {
  $record = Get-ItemProperty -LiteralPath "HKLM:\$($Product.Arp)"
  $exe = Join-Path $Product.Default $Product.Remove
  if ($record.UninstallString -notmatch '^"([^"]+\.exe)"(.*)$' -or $Matches[1] -ne $exe) {
    throw 'Machine uninstaller changed after acceptance.'
  }
  Invoke-Installer $exe ("/S" + $Matches[2])
  if ((Test-Path -LiteralPath "HKLM:\$($Product.Arp)") -or (Test-Path -LiteralPath (Join-Path $Product.Default $Product.Exe))) {
    throw "$($Product.Name) did not remove its own machine program and uninstall registration."
  }
  foreach ($path in $expectedFiles.Keys) { Assert-Hash $path $expectedFiles[$path] }
  if ((Get-ItemPropertyValue -LiteralPath $storageKey -Name LocationV1) -cne $layout) {
    throw 'Machine uninstall changed the initiating user Main data locator.'
  }
}

Remove-MachineFixture $main
Assert-Installed $league HKLM $league.Version $league.Default
Assert-CommonShortcuts $league
Assert-Hash $machineLeague $machineLeagueHash
Assert-Hash $machineLeagueArchive $machineLeagueArchiveHash
Invoke-Installer $Installer '/S /ALLUSERS'
Assert-Installed $main HKLM $main.Version $main.Default
Remove-MachineFixture $league
Assert-Installed $main HKLM $main.Version $main.Default
Assert-CommonShortcuts $main
Assert-Hash $machineMain $machineMainHash

Write-Host 'ALL_USERS_INSTALL_VERIFIED: both machine uninstallers preserved the other product, the original fixtures and both private profiles.'
Write-Host 'Interactive UAC consent requires a real interactive Windows user session; this runner checked only its observed token route.'
