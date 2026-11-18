$ErrorActionPreference = 'Stop'

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

try {
  if ([Environment]::Is64BitOperatingSystem -and ![Environment]::Is64BitProcess) {
    throw 'Native registry view is required.'
  }
  $leagueIdentity = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
  $uninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall'
  $leagueRoot = Join-Path $env:APPDATA 'MaxGameStudioLeague'
  $activeLeagueProfile = Join-Path $leagueRoot 'Profile'
  $leagueProfiles = @($activeLeagueProfile)
  foreach ($filename in 'data-location.txt', 'installer-data-location.txt') {
    $path = Join-Path $leagueRoot $filename
    if (Test-Path -LiteralPath $path -PathType Leaf) {
      $savedProfile = [IO.File]::ReadAllText($path, [Text.Encoding]::Unicode).Trim()
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
  $owned = @()
  foreach ($hive in 'HKCU:', 'HKLM:') {
    $existing = Read-InstallLocation "$hive\Software\$leagueIdentity"
    $remove = Read-Value "$hive\$uninstallKey\$leagueIdentity" 'UninstallString'
    if (!$existing -and !$remove) { continue }
    if (!$existing -or $remove -notmatch '^"([^"]+\.exe)"(?:\s|$)') { throw 'Invalid League registration.' }
    $uninstaller = $Matches[1]
    $existing = Get-LocalDirectory $existing
    Assert-Separate $existing $protected
    if ((Get-LocalDirectory (Split-Path -Parent $uninstaller)) -ne $existing -or
        (Split-Path -Leaf $uninstaller) -ne 'Uninstall MaxGameStudioLeagueToolbox.exe') {
      throw 'League registration does not own the selected executable.'
    }
    $resources = Get-LocalDirectory (Join-Path $existing 'resources')
    foreach ($ownedFile in @($uninstaller, (Join-Path $existing 'MaxGameStudioLeagueToolbox.exe'),
        (Join-Path $resources 'app.asar'))) {
      $item = Get-Item -LiteralPath $ownedFile -Force
      if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Invalid registered League file.'
      }
    }
    $owned += $existing
  }
  if ($env:MGS_LEAGUE_INSTALL_PHASE -ne 'registrations') {
    $program = Get-LocalDirectory $env:MGS_LEAGUE_INSTALL_DEST
    if ($env:MGS_LEAGUE_DATA_DEST) {
      $data = Get-LocalDirectory $env:MGS_LEAGUE_DATA_DEST
      Assert-Separate $data ($mainProtected + $owned)
      $control = Get-LocalDirectory $leagueRoot
      if ($data -eq $control -or $control.StartsWith($data + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The data directory would contain the Toolbox location settings.'
      }
      if ($data -ne (Get-LocalDirectory $activeLeagueProfile) -and
          (Test-Path -LiteralPath $data) -and
          (Get-ChildItem -LiteralPath $data -Force | Select-Object -First 1) -and
          !(Test-Path -LiteralPath (Join-Path $data '.maxgamestudio-league-profile') -PathType Leaf)) {
        throw 'New data folder is not empty.'
      }
      $protected += $data
    }
    Assert-Separate $program $protected
    if ($program -notin $owned -and (Test-Path -LiteralPath $program) -and
        (Get-ChildItem -LiteralPath $program -Force | Select-Object -First 1)) {
      throw 'New program folder is not empty.'
    }
  }
  exit 0
} catch {
  exit 2
}
