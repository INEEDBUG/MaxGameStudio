function New-FixtureUser([string]$Role, [Collections.Generic.List[Security.Principal.SecurityIdentifier]]$CreatedAccounts) {
  if (!$IsWindows -or ![Environment]::Is64BitProcess -or
      $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Synthetic local users are restricted to ephemeral x64 GitHub-hosted Windows runners.'
  }
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
  $CreatedAccounts.Add($user.SID)
  $group = Get-LocalGroup -SID 'S-1-5-32-545'
  if (!(Get-LocalGroupMember -Group $group | Where-Object { $_.SID -eq $user.SID })) {
    Add-LocalGroupMember -Group $group -Member $user
  }
  return @{ Name = $name; SID = $user.SID.Value; Credential = [PSCredential]::new("$env:COMPUTERNAME\$name", $password) }
}

function Start-StandardProcess {
  param(
    [Parameter(Mandatory)]$User,
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Directory,
    [string]$Arguments = '',
    [Parameter(Mandatory)][string]$Stage,
    [switch]$Visible
  )
  # CreateProcessWithLogonW accepts a maximum 1,024-character mutable command
  # line. Leave room for the quoted executable, separator and NUL terminator.
  $characters = $Exe.Length + 4 + $Arguments.Length
  if ($characters -ge 1024) {
    throw "Standard-user launch exceeds the Win32 command limit (stage=$Stage, commandChars=$characters)."
  }
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
  try {
    return [Diagnostics.Process]::Start($info)
  } catch {
    $failure = $_.Exception.GetBaseException()
    $code = if ($failure -is [ComponentModel.Win32Exception]) { $failure.NativeErrorCode } else { 'unavailable' }
    throw "Standard-user launch failed (stage=$Stage, nativeError=$code, commandChars=$characters, type=$($failure.GetType().Name))."
  }
}

function Test-StandardUserAccess($User, [string[]]$Own, [string[]]$Other) {
  # The local script contains fixture paths and a synthetic SID. The password
  # remains in SecureString and is handed only to the Windows logon API.
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
try {
  $handle = [IO.File]::Open($PSCommandPath, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
  $handle.Dispose()
  exit 15
} catch {
  if ($_.Exception.GetBaseException() -isnot [UnauthorizedAccessException]) { exit 16 }
}
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
  $stage = if ($Own.Count -eq 0 -and $Other.Count -eq 0) { 'profile-environment' } else { 'profile-access' }
  # Program Files grants standard users read/execute while the synthetic
  # account cannot modify its probe. The child verifies that ACL too.
  $folder = Join-Path $env:ProgramFiles ('MGS-Standard-Probe-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $folder | Out-Null
  $probePath = Join-Path $folder 'profile-check.ps1'
  try {
    [IO.File]::WriteAllText($probePath, $probe, [Text.UTF8Encoding]::new($true))
    $windowsPowerShell = "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
    $arguments = '-NoLogo -NoProfile -NonInteractive -File "' + $probePath + '"'
    $process = Start-StandardProcess $User $windowsPowerShell "$env:WINDIR\System32" $arguments -Stage $stage
    try {
      if (!$process.WaitForExit(30000)) {
        Stop-CiFixtureProcess $process
        throw "Timed out validating the standard-user fixture (stage=$stage)."
      }
      if ($process.ExitCode -ne 0) {
        throw "Standard-user fixture check failed (stage=$stage, exit=$($process.ExitCode))."
      }
    } finally {
      $process.Dispose()
    }
  } finally {
    if (Test-Path -LiteralPath $probePath -PathType Leaf) { Remove-Item -LiteralPath $probePath }
    if (!(Get-ChildItem -LiteralPath $folder -Force | Select-Object -First 1)) {
      Remove-Item -LiteralPath $folder
    }
  }
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

function Assert-StandardProcessOwner([Diagnostics.Process]$Process, $User) {
  if (![MgsAllUsersNative]::IsStandardUser($Process.Handle, $User.SID)) {
    throw 'The test process did not start under the expected standard-user token.'
  }
}

function Write-VerifiedNativeIdleSample([Diagnostics.Process]$App, $Product, [string]$IdleDataDir) {
  # The probe confirmed this exact process rendered Home before the warmup.
  Start-Sleep -Seconds 10
  $App.Refresh()
  if ($App.HasExited) { throw 'The verified app exited before sampling its idle memory.' }
  $pids = [Collections.Generic.HashSet[int]]::new()
  [void]$pids.Add($App.Id)
  $snapshot = @(Get-CimInstance Win32_Process)
  do {
    $added = $false
    foreach ($child in $snapshot) {
      if ($child.CreationDate -ge $App.StartTime -and $pids.Contains([int]$child.ParentProcessId) -and
          $pids.Add([int]$child.ProcessId)) { $added = $true }
    }
  } while ($added)
  $processes = @(Get-Process -Id @($pids) -ErrorAction SilentlyContinue)
  $counters = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
    Where-Object { $pids.Contains([int]$_.IDProcess) })
  if ($processes.Count -ne $pids.Count -or $counters.Count -ne $pids.Count) {
    throw 'The verified idle sample lost a process or its memory counter.'
  }
  $appBytes = (Get-ChildItem -LiteralPath $Product.Default -Recurse -File -Force | Measure-Object Length -Sum).Sum
  $profileBytes = (Get-ChildItem -LiteralPath $IdleDataDir -Recurse -File -Force | Measure-Object Length -Sum).Sum
  $info = [ordered]@{
    product = $Product.Name
    version = $Product.Version
    scene = 'verified Home, standard user, no game client'
    processes = $processes.Count
    privateWorkingSetMiB = [math]::Round(($counters | Measure-Object WorkingSetPrivate -Sum).Sum / 1MB, 2)
    privateCommitMiB = [math]::Round(($processes | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 2)
    installedProgramBytes = [long]$appBytes
    profileBytes = [long]$profileBytes
    installerBytes = (Get-Item -LiteralPath $Product.Installer).Length
    packageSha256 = (Get-FileHash -LiteralPath $Product.Installer -Algorithm SHA256).Hash
  }
  Write-Host ('NATIVE_HOME_RESOURCE_SAMPLE: ' + ($info | ConvertTo-Json -Compress))
}

function Invoke-NativeUpdaterProbe($Product, $User, [string]$ExpectedHash, [string]$IdleProfile) {
  $exe = Join-Path $Product.Default $Product.Exe
  if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -ine $ExpectedHash) {
    throw 'The installed executable does not match the exact verified package input.'
  }
  # WebView2 ignores user-scoped browser flags for elevated hosts. Give only
  # the ordinary fixture user's app process the temporary debugger settings.
  $reservation = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
  $reservation.Start()
  try { $port = $reservation.LocalEndpoint.Port }
  finally { $reservation.Stop() }

  $folder = Join-Path $env:ProgramFiles ('MGS-Update-Probe-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $folder | Out-Null
  $launchScript = Join-Path $folder 'launch.ps1'
  $launcher = $null
  try {
    [IO.File]::WriteAllText($launchScript, @'
param([string]$Executable, [int]$Port)
$ErrorActionPreference = 'Stop'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-address=127.0.0.1 --remote-debugging-port=$Port"
$app = Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable) -Wait -PassThru
exit $app.ExitCode
'@)
    $windowsPowerShell = "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
    $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Executable "{1}" -Port {2}' -f $launchScript, $exe, $port
    $launcher = Start-StandardProcess $User $windowsPowerShell $Product.Default $arguments -Stage 'native-updater-launch'
    $app = Wait-Fixture 'Installed updater app did not start' {
      $child = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($launcher.Id)" |
        Where-Object { $_.ExecutablePath -ieq $exe } | Select-Object -First 1
      if ($child) { Get-Process -Id $child.ProcessId -ErrorAction SilentlyContinue }
    } $launcher
    Assert-StandardProcessOwner $app $User
    $null = Wait-Fixture 'Installed app did not expose a loopback WebView2 debugger' {
      $listeners = @(Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue)
      if (!$listeners.Count) { return }
      foreach ($listener in $listeners) {
        if ($listener.LocalAddress -notin '127.0.0.1', '::1') { throw 'Debugger is not confined to loopback.' }
        $id = $listener.OwningProcess
        for ($depth = 0; $id -and $id -ne $app.Id -and $depth -lt 12; $depth++) {
          $ancestor = Get-CimInstance Win32_Process -Filter "ProcessId=$id"
          if (!$ancestor -or $ancestor.CreationDate.ToUniversalTime().AddTicks(9) -lt $app.StartTime.ToUniversalTime()) { break }
          $id = $ancestor.ParentProcessId
        }
        if ($id -ne $app.Id) { throw 'Debugger is not owned by the installed app process tree.' }
      }
      if ('127.0.0.1' -in $listeners.LocalAddress) { return $true }
    } $app
    $probe = Join-Path $PSScriptRoot 'verify-native-updaters.mjs'
    if ($IdleProfile) {
      & node $probe ($Product.Name.ToLowerInvariant()) $exe ($ExpectedHash.ToLowerInvariant()) $Product.Version $port idle
      if ($LASTEXITCODE -ne 0) { throw "$($Product.Name) Home was not ready for an idle resource sample." }
      Write-VerifiedNativeIdleSample $app $Product $IdleProfile
    }
    & node $probe ($Product.Name.ToLowerInvariant()) $exe ($ExpectedHash.ToLowerInvariant()) $Product.Version $port
    if ($LASTEXITCODE -ne 0) { throw "$($Product.Name) installed updater verification failed." }
    if ($Product.Name -eq 'League') {
      if (!$app.WaitForExit(8000) -or $app.ExitCode -ne 0) {
        throw 'The installed League Toolbox did not exit cleanly using its close dialog.'
      }
      Write-Host 'NATIVE_LEAGUE_EXIT_VERIFIED: the standalone close dialog exited the installed process.'
      return
    }
    if ($app.HasExited) { throw "$($Product.Name) exited during updater verification." }
    if ($Product.Name -eq 'Main') {
      $api = @(Get-NetTCPConnection -LocalPort 19871 -State Listen -OwningProcess $app.Id -ErrorAction SilentlyContinue)
      if ($api.Count -ne 1 -or $api[0].LocalAddress -ne '127.0.0.1') {
        throw 'The installed Main process did not own its native loopback API listener.'
      }
      Write-Host 'MAIN_NATIVE_RUNTIME_VERIFIED: installed PID served the expected health and authenticated config.'
    }
  } finally {
    try {
      if ($launcher) { Stop-CiFixtureProcess $launcher }
    } finally {
      if (Test-Path -LiteralPath $launchScript -PathType Leaf) { Remove-Item -LiteralPath $launchScript }
      Remove-Item -LiteralPath $folder
    }
  }
}
