$ErrorActionPreference = 'Stop'
# NSIS can inherit a PowerShell 7 module path. Use the modules for this Windows PowerShell process.
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
. (Join-Path $PSScriptRoot 'retire-user-install.ps1')

Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
using System.Text;
public static class LeagueInstallPath {
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
  public static extern uint GetLongPathName(string path, StringBuilder result, uint length);
}
'@

function Get-LocalDirectory([string]$Path) {
  if ($Path -notmatch '^[a-z]:\\' -or $Path.Substring(2) -match '[:*?"<>|]') {
    throw 'Invalid local directory.'
  }
  $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
  $root = [IO.Path]::GetPathRoot($full + '\')
  if ($full -eq $root.TrimEnd('\')) { throw 'Volume root is not an installation directory.' }
  $result = $root
  foreach ($segment in $full.Substring($root.Length).Split('\')) {
    if (!$segment -or $segment.EndsWith('.') -or $segment.EndsWith(' ')) {
      throw 'Ambiguous path segment.'
    }
    $result = Join-Path $result $segment
    if (Test-Path -LiteralPath $result) {
      $item = Get-Item -LiteralPath $result -Force
      if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'File or reparse point in directory path.'
      }
      $buffer = [Text.StringBuilder]::new(32768)
      $length = [LeagueInstallPath]::GetLongPathName($result, $buffer, [uint32]$buffer.Capacity)
      if (!$length -or $length -ge $buffer.Capacity) { throw 'Cannot canonicalize local path.' }
      $result = $buffer.ToString()
    }
  }
  return $result
}

function Test-Overlap([string]$One, [string]$Other) {
  $comparison = [StringComparison]::OrdinalIgnoreCase
  return $One.Equals($Other, $comparison) -or
    $One.StartsWith($Other + '\', $comparison) -or $Other.StartsWith($One + '\', $comparison)
}

function Assert-Separate([string]$Path, [string[]]$Protected) {
  foreach ($item in $Protected) {
    if ($item -and (Test-Overlap $Path (Get-LocalDirectory $item))) { throw 'Directory overlap.' }
  }
}

function Read-Value([string]$Key, [string]$Name) {
  if (Test-Path -LiteralPath $Key) { (Get-ItemProperty -LiteralPath $Key).$Name }
}

function Read-InstallLocation([string]$Key) {
  $value = Read-Value $Key 'InstallLocation'
  if ($value) { $value.Trim() -replace '^"([^"]+)"$', '$1' }
}

function Read-LeagueRegistration([string]$Hive, [string]$Identity, [string]$UninstallKey) {
  $location = Read-InstallLocation "$Hive\Software\$Identity"
  $remove = Read-Value "$Hive\$UninstallKey\$Identity" 'UninstallString'
  if (!$location -and !$remove) { return }
  $uninstaller = if ($remove -is [string] -and $remove -match '^"([^"]+\.exe)"(?:\s|$)') { $Matches[1] }
  [pscustomobject]@{ Location = $location; Uninstaller = $uninstaller }
}

function Test-NonemptyDirectory([string]$Path) {
  (Test-Path -LiteralPath $Path -PathType Container) -and
    [bool](Get-ChildItem -LiteralPath $Path -Force | Select-Object -First 1)
}

function Get-IncompleteInstallDirectory($Registration) {
  $path = $Registration.Location
  if (!$path -and $Registration.Uninstaller -match '^[a-z]:\\.*\\Uninstall MaxGameStudioLeagueToolbox\.exe$') {
    $path = Split-Path -Parent $Registration.Uninstaller
  }
  if (!$path) { return }
  try { $path = Get-LocalDirectory $path } catch { return }
  # A known occupied directory is protected. An unverified record never establishes ownership.
  if (Test-NonemptyDirectory $path) { $path }
}

function Test-LeaguePayload([string]$Program) {
  $resources = Get-LocalDirectory (Join-Path $Program 'resources')
  $complete = $true
  foreach ($path in (Join-Path $Program 'MaxGameStudioLeagueToolbox.exe'),
      (Join-Path $Program 'Uninstall MaxGameStudioLeagueToolbox.exe'), (Join-Path $resources 'app.asar')) {
    try { $item = Get-Item -LiteralPath $path -Force }
    catch [System.Management.Automation.ItemNotFoundException] { $complete = $false; continue }
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw 'Invalid League program file.'
    }
    if (!$item.Length) { $complete = $false }
  }
  return $complete
}

function Get-OwnedLeagueDirectory($Registration, [string[]]$Protected) {
  if (!$Registration.Location -or !$Registration.Uninstaller) { return }
  $existing = Get-LocalDirectory $Registration.Location
  Assert-Separate $existing $Protected
  if ((Get-LocalDirectory (Split-Path -Parent $Registration.Uninstaller)) -ne $existing -or
      (Split-Path -Leaf $Registration.Uninstaller) -ne 'Uninstall MaxGameStudioLeagueToolbox.exe') {
    throw 'League registration does not own the selected executable.'
  }
  if (Test-LeaguePayload $existing) { $existing }
}

function Assert-ProtectedMachinePath([string]$Path) {
  $trusted = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
  $acl = Get-Acl -LiteralPath $Path
  if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) {
    throw 'Unexpected machine installation path owner.'
  }
  $write = [Security.AccessControl.FileSystemRights]'Write, Delete, DeleteSubdirectoriesAndFiles, ChangePermissions, TakeOwnership'
  foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
    if ($rule.AccessControlType -eq 'Allow' -and ($rule.FileSystemRights -band $write) -and
        $rule.IdentityReference.Value -notin $trusted -and
        -not ($rule.IdentityReference.Value -eq 'S-1-3-0' -and ($rule.PropagationFlags -band 'InheritOnly'))) {
      throw 'Machine installation path is writable by an unprivileged principal.'
    }
  }
}

function Assert-MachineDestination([string]$Destination, [string]$Identity, [string]$UninstallKey) {
  $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
  $program = Join-Path $programFiles 'MaxGameStudioLeagueToolbox'
  if ((Get-LocalDirectory $Destination) -ne $program -or $Destination.TrimEnd('\') -ne $program) {
    throw 'The machine program must use its dedicated Program Files directory.'
  }
  Assert-ProtectedMachinePath $programFiles
  if ([IO.Directory]::Exists($program)) { Assert-ProtectedMachinePath $program }
  $registration = Read-LeagueRegistration 'HKLM:' $Identity $UninstallKey
  if (!$registration) { return 0 }
  if ($registration.Location -and $registration.Uninstaller) {
    $uninstaller = Join-Path $program 'Uninstall MaxGameStudioLeagueToolbox.exe'
    if ($registration.Location -ne $program -or $registration.Uninstaller -ne $uninstaller) {
      throw 'Unexpected machine League registration.'
    }
    if (Test-LeaguePayload $program) {
      Assert-ProtectedMachinePath $uninstaller
      return 0
    }
  }
  if (Test-NonemptyDirectory $program) { throw 'Incomplete machine installation has an occupied program folder.' }
  return 5
}

try {
  if ([Environment]::Is64BitOperatingSystem -and ![Environment]::Is64BitProcess) {
    throw 'Native registry view is required.'
  }
  $phase = $env:MGS_LEAGUE_INSTALL_PHASE
  $scope = $env:MGS_LEAGUE_INSTALL_SCOPE
  if ($phase -notin 'registrations', 'selection', 'machine', 'payload', 'complete' -or $scope -notin 'CurrentUser', 'all') {
    throw 'Unknown League installation phase or scope.'
  }
  if ($phase -eq 'selection' -and (Get-Process -Name 'MaxGameStudioLeagueToolbox' -ErrorAction SilentlyContinue)) {
    exit 3
  }
  $leagueIdentity = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
  $uninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall'
  if ($phase -eq 'machine') {
    if ($scope -ne 'all') { throw 'Wrong scope for machine destination check.' }
    exit (Assert-MachineDestination $env:MGS_LEAGUE_INSTALL_DEST $leagueIdentity $uninstallKey)
  }
  if ($phase -eq 'payload') {
    $program = Get-LocalDirectory $env:MGS_LEAGUE_INSTALL_DEST
    if (!(Test-LeaguePayload $program)) { exit 7 }
    exit 0
  }
  $leagueRoot = Get-LocalDirectory (Join-Path $env:APPDATA 'MaxGameStudioLeague')
  $activeLeagueProfile = Join-Path $leagueRoot 'Profile'
  $leagueProfiles = @($activeLeagueProfile)
  foreach ($filename in 'data-location.txt', 'installer-data-location.txt') {
    $locator = Get-LeagueOrdinaryFile $leagueRoot $filename
    if ($locator) {
      $savedProfile = [IO.File]::ReadAllText($locator.FullName, [Text.Encoding]::Unicode).Trim()
      $leagueProfiles += $savedProfile
      if ($savedProfile -and $filename -eq 'data-location.txt') { $activeLeagueProfile = $savedProfile }
    }
  }
  $mainProtected = @(
    (Join-Path $env:APPDATA 'MaxGameStudio'),
    (Join-Path $env:LOCALAPPDATA 'MaxGameStudio')
  )
  $storageKey = 'HKCU:\Software\MaxGameStudio\Storage'
  $mainLocation = Read-Value $storageKey 'LocationV1'
  if ($mainLocation) {
    $location = ConvertFrom-Json -InputObject $mainLocation
    $mainProtected += @($location.root, $location.pending, $location.previous,
      $location.paths.data, $location.paths.webview, $location.paths.league_runtime)
  }
  $mainSelection = Read-Value $storageKey 'InstallerDataRootV1'
  if ($mainSelection) { $mainProtected += ($mainSelection -split '\|', 2)[-1] }
  foreach ($hive in 'HKCU:', 'HKLM:') {
    foreach ($name in 'MaxGameStudio', 'CS2 Ultimate Insight Studio') {
      $mainProtected += Read-InstallLocation "$hive\$uninstallKey\$name"
    }
  }
  $protected = $mainProtected + $leagueProfiles + $leagueRoot
  $owned = @{}
  $incomplete = @{}
  $preserved = @()
  foreach ($hive in 'HKCU:', 'HKLM:') {
    $registration = Read-LeagueRegistration $hive $leagueIdentity $uninstallKey
    if (!$registration) { continue }
    $registeredScope = if ($hive -eq 'HKCU:') { 'CurrentUser' } else { 'all' }
    $existing = Get-OwnedLeagueDirectory $registration $protected
    if (!$existing) {
      $incomplete[$registeredScope] = $true
      if ($phase -ne 'registrations') { $preserved += Get-IncompleteInstallDirectory $registration }
      continue
    }
    $owned[$registeredScope] = $existing
  }
  if ($phase -eq 'registrations') {
    # NSIS uses these bits to show a fresh destination for the affected scope.
    exit ((8 * [int]$incomplete.ContainsKey('CurrentUser')) + (16 * [int]$incomplete.ContainsKey('all')))
  }
  if ($phase -ne 'registrations') {
    $program = Get-LocalDirectory $env:MGS_LEAGUE_INSTALL_DEST
    $otherScope = if ($scope -eq 'all') { 'CurrentUser' } else { 'all' }
    Assert-Separate $program @($owned[$otherScope])
    $deferUserChanges = $false
    if ($scope -eq 'all' -and $phase -eq 'complete') {
      $caller = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
      $deferUserChanges = $caller.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    }
    $data = $null
    if (!$deferUserChanges -and $env:MGS_LEAGUE_DATA_DEST) {
      $data = Get-LocalDirectory $env:MGS_LEAGUE_DATA_DEST
      Assert-Separate $data ($mainProtected + @($owned.Values) + $preserved)
      $control = Get-LocalDirectory $leagueRoot
      if ($data -eq $control -or $control.StartsWith($data + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The data directory would contain the Toolbox location settings.'
      }
      if ($data -ne (Get-LocalDirectory $activeLeagueProfile) -and (Test-NonemptyDirectory $data) -and
          !(Test-Path -LiteralPath (Join-Path $data '.maxgamestudio-league-profile') -PathType Leaf)) {
        throw 'New data folder is not empty.'
      }
      $protected += $data
    }
    Assert-Separate $program ($protected + $preserved)
    if ($program -ne $owned[$scope] -and (Test-NonemptyDirectory $program)) {
      throw 'New program folder is not empty.'
    }
    if ($phase -eq 'complete') {
      if ($program -ne $owned[$scope]) { throw 'The new program installation is not registered.' }
      $selection = if ($data -and $data -ne (Get-LocalDirectory $activeLeagueProfile)) { $data }
      $oldProgram = if ($scope -eq 'all') { $owned['CurrentUser'] }
      exit (Complete-LeagueUserInstall $leagueRoot $selection $oldProgram $leagueIdentity -Defer:$deferUserChanges)
    }
  }
  if ($incomplete[$scope]) { exit 5 }
  exit 0
} catch {
  # Return an installer support code without copying registry contents or paths into the dialog.
  switch -Exact ($_.Exception.Message) {
    'Invalid local directory.' { Write-Output 'LG01' }
    'Directory overlap.' { Write-Output 'LG02' }
    'New program folder is not empty.' { Write-Output 'LG03' }
    'New data folder is not empty.' { Write-Output 'LG04' }
    'League registration does not own the selected executable.' { Write-Output 'LG05' }
  }
  exit 2
}
