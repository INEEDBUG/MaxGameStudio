param(
  [Parameter(Mandatory)][string]$ProgramDir,
  [string]$DataDir,
  [switch]$MachineScope,
  [switch]$MachineFilesOnly
)

$ErrorActionPreference = 'Stop'

try {
  # Resolve the longest existing prefix. This catches junctions, mapped volume
  # aliases and 8.3 aliases, including when the child does not exist yet.
  Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class MgsDirectoryIdentity {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint share,
        IntPtr attributes, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetFinalPathNameByHandle(SafeFileHandle handle, StringBuilder path,
        uint chars, uint flags);

    public static string Resolve(string path) {
        using (var h = CreateFile(path, 0, 7, IntPtr.Zero, 3, 0x02000000, IntPtr.Zero)) {
            if (h.IsInvalid) throw new Win32Exception();
            var name = new StringBuilder(32768);
            uint size = GetFinalPathNameByHandle(h, name, (uint)name.Capacity, 0);
            if (size == 0 || size >= name.Capacity) throw new Win32Exception();
            return name.ToString();
        }
    }
}
'@

  function Get-DirectoryIdentity([string]$Path) {
    $path = [IO.Path]::GetFullPath($Path.Trim().Trim('"'))
    $tail = New-Object 'Collections.Generic.Stack[string]'
    while (-not [IO.Directory]::Exists($path)) {
      if ([IO.File]::Exists($path)) { throw 'A path component is a file.' }
      $parent = [IO.Directory]::GetParent($path)
      if ($null -eq $parent) { return $path.TrimEnd('\') }
      $tail.Push([IO.Path]::GetFileName($path.TrimEnd('\')))
      $path = $parent.FullName
    }
    $actual = [MgsDirectoryIdentity]::Resolve($path)
    if ($actual.StartsWith('\\?\UNC\')) {
      $actual = '\\' + $actual.Substring(8)
    } elseif ($actual.StartsWith('\\?\')) {
      $actual = $actual.Substring(4)
    }
    foreach ($part in $tail) { $actual = [IO.Path]::Combine($actual, $part) }
    return $actual.TrimEnd('\')
  }

  function Test-Overlap([string]$Left, [string]$Right) {
    $comparison = [StringComparison]::OrdinalIgnoreCase
    return $Left.Equals($Right, $comparison) -or
      $Left.StartsWith($Right + '\', $comparison) -or $Right.StartsWith($Left + '\', $comparison)
  }

  function Get-RegistryRecord([string]$Key) {
    if (Test-Path -LiteralPath $Key) { Get-ItemProperty -LiteralPath $Key }
  }

  $program = Get-DirectoryIdentity $ProgramDir
  if ($program -notmatch '^[A-Za-z]:\\.+') { throw 'Choose a dedicated local program directory.' }
  if ($MachineScope) {
    $programFiles = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    $expected = Join-Path $programFiles 'MaxGameStudio'
    if (-not $program.Equals($expected, [StringComparison]::OrdinalIgnoreCase) -or
        -not ([IO.Path]::GetFullPath($ProgramDir).TrimEnd('\')).Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
      throw 'The machine program must use its dedicated Program Files directory.'
    }
    # Confirm inherited Windows permissions as well as the physical path.
    # A pre-created directory with a relaxed ACL must not receive code that
    # the machine uninstaller will later execute or replace as administrator.
    $trustedWriters = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    foreach ($path in @($programFiles, $program)) {
      if (-not [IO.Directory]::Exists($path)) { continue }
      $item = Get-Item -LiteralPath $path -Force
      if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'The machine program path includes a reparse point.'
      }
      $acl = Get-Acl -LiteralPath $path
      if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trustedWriters) {
        throw 'The machine program directory has an unexpected owner.'
      }
      $rules = $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
      $writeRights = [Security.AccessControl.FileSystemRights]'Write, Delete, DeleteSubdirectoriesAndFiles, ChangePermissions, TakeOwnership'
      foreach ($rule in $rules) {
        if ($rule.AccessControlType -eq 'Allow' -and ($rule.FileSystemRights -band $writeRights) -and
            $rule.IdentityReference.Value -notin $trustedWriters -and
            -not ($rule.IdentityReference.Value -eq 'S-1-3-0' -and ($rule.PropagationFlags -band 'InheritOnly'))) {
          throw 'The machine program directory is writable by an unprivileged principal.'
        }
      }
    }
  }
  if ($MachineFilesOnly) {
    $leaguePaths = @()
    $hives = @('HKLM:')
  } else {
  $control = Join-Path $env:APPDATA 'MaxGameStudioLeague'
  $leaguePaths = @($control)
  foreach ($file in @('data-location.txt', 'installer-data-location.txt')) {
    $pointer = Join-Path $control $file
    if ([IO.File]::Exists($pointer)) {
      $leaguePaths += [IO.File]::ReadAllText($pointer, [Text.Encoding]::Unicode).Trim()
    }
  }
    $hives = @('HKCU:', 'HKLM:')
  }
  foreach ($hive in $hives) {
    foreach ($key in @(
      "$hive\Software\b1d9ab62-895c-574e-a779-36d3f2ee3611",
      "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\b1d9ab62-895c-574e-a779-36d3f2ee3611"
    )) {
      $leaguePaths += (Get-RegistryRecord $key).InstallLocation
    }
  }
  $leagueIdentities = @($leaguePaths | Where-Object { $_ } | ForEach-Object { Get-DirectoryIdentity $_ })
  $data = if ($DataDir) { Get-DirectoryIdentity $DataDir }
  if ($data -and (Test-Overlap $program $data)) { throw 'Program and data directories overlap.' }
  foreach ($path in $leagueIdentities) {
    if ((Test-Overlap $program $path) -or ($data -and (Test-Overlap $data $path))) {
      throw 'The selected directory overlaps a League program or profile directory.'
    }
  }

  if (-not $MachineFilesOnly) {
  $storage = Get-RegistryRecord 'HKCU:\Software\MaxGameStudio\Storage'
  $mainPaths = @(
    (Join-Path $env:APPDATA 'CS2 Insight Agent')
    (Join-Path $env:APPDATA 'com.cs2insightagent.app')
    (Join-Path $env:APPDATA 'cs2-insight-agent')
    (Join-Path $env:APPDATA 'MaxGameStudio')
    (Join-Path $env:APPDATA 'MaxGameStudioData')
    (Join-Path $env:LOCALAPPDATA 'com.cs2insightagent.app')
  )
  foreach ($drive in [IO.DriveInfo]::GetDrives()) {
    if ($drive.DriveType -eq [IO.DriveType]::Fixed) {
      $mainPaths += Join-Path $drive.Name 'MaxGameStudioData'
    }
  }
  if ($storage.LocationV1) {
    $layout = $storage.LocationV1 | ConvertFrom-Json
    $mainPaths += @($layout.root, $layout.previous, $layout.pending) + @($layout.required_paths)
    if ($layout.paths) { $mainPaths += @($layout.paths.PSObject.Properties.Value) }
    if ($layout.locations) {
      foreach ($saved in $layout.locations.PSObject.Properties) {
        $mainPaths += $saved.Name
        if ($saved.Value.paths) { $mainPaths += @($saved.Value.paths.PSObject.Properties.Value) }
      }
    }
  }
  if ([string]$storage.InstallerDataRootV1 -match '^[^|]+\|(.+)$') { $mainPaths += $Matches[1] }
  foreach ($path in ($mainPaths | Where-Object { $_ -is [string] -and $_ })) {
    if (Test-Overlap $program (Get-DirectoryIdentity $path)) {
      throw 'The program directory overlaps a MaxGameStudio data directory.'
    }
  }
  }

  if ([IO.Directory]::Exists($ProgramDir) -and
      (Get-ChildItem -LiteralPath $ProgramDir -Force | Select-Object -First 1)) {
    $ownHive = if ($MachineScope) { 'HKLM:' } else { 'HKCU:' }
    $ownRegistration = Get-RegistryRecord "$ownHive\Software\cs2insightagent\MaxGameStudio"
    $ownUninstall = Get-RegistryRecord "$ownHive\Software\Microsoft\Windows\CurrentVersion\Uninstall\MaxGameStudio"
    $ownCandidates = @($ownRegistration.'(default)', $ownUninstall.InstallLocation)
    if ($MachineScope -and $ownUninstall.AllUsers -ne 1) { throw 'The machine directory is not registered.' }
    if (-not $MachineScope) {
    foreach ($hive in $hives) {
      $legacy = Get-RegistryRecord "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\CS2 Ultimate Insight Studio"
      $ownCandidates += $legacy.InstallLocation
    }
    }
    $registered = $ownCandidates | Where-Object {
      $_ -and $program.Equals((Get-DirectoryIdentity $_), [StringComparison]::OrdinalIgnoreCase)
    } | Select-Object -First 1
    $legacyElectron = -not $MachineScope -and [IO.File]::Exists((Join-Path $ProgramDir 'CS2 Insight Agent.exe')) -and
      [IO.File]::Exists((Join-Path $ProgramDir 'Uninstall CS2 Insight Agent.exe'))
    if (-not (($registered -and [IO.File]::Exists((Join-Path $ProgramDir 'cs2-insight-agent-desktop.exe'))) -or $legacyElectron)) {
      throw 'Choose an empty program directory.'
    }
  }
  exit 0
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 2
}
