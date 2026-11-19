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
