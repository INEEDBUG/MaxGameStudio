. (Join-Path $PSScriptRoot 'shortcut-target.ps1')

function Get-LeagueOrdinaryFile([string]$Root, [string]$Relative) {
  if (!$Root) { return }
  $parts = $Relative.Replace('\', '/').Split('/')
  if (!$parts.Count -or @($parts | Where-Object { !$_ -or $_ -in '.', '..' -or $_ -match '[:*?"<>|]' }).Count) {
    return
  }
  $path = $Root
  foreach ($part in $parts) {
    $path = Join-Path $path $part
    if (!(Test-Path -LiteralPath $path)) { return }
    $item = Get-Item -LiteralPath $path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return }
  }
  if (!$item.PSIsContainer) { $item }
}

function Get-LeagueNativeRuntime([string]$Root) {
  $executableName = 'MaxGameStudioLeagueToolbox.exe'
  $executable = Get-LeagueOrdinaryFile $Root $executableName
  $inventory = Get-LeagueOrdinaryFile $Root 'maxgamestudio-runtime-hashes.json'
  if (!$executable -or !$executable.Length -or !$inventory -or !$inventory.Length) { return }

  $versionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($executable.FullName)
  if ($versionInfo.ProductName -cne 'MaxGameStudio League' -or
      $versionInfo.ProductVersion -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-beta\.(0|[1-9][0-9]*))?$') {
    return
  }
  try {
    $files = Get-Content -LiteralPath $inventory.FullName -Raw | ConvertFrom-Json
  } catch {
    return
  }
  if ($files -isnot [pscustomobject] -or
      @($files.PSObject.Properties | Where-Object { $_.Name -cnotin $LeagueNativeFiles }).Count) { return }
  $digest = $files.PSObject.Properties[$executableName].Value
  if ($digest -cnotmatch '^[a-f0-9]{64}$' -or
      (Get-FileHash -LiteralPath $executable.FullName -Algorithm SHA256).Hash -ine $digest) {
    return
  }
  [pscustomobject]@{ Executable = $executable; Version = $versionInfo.ProductVersion; Digest = $digest }
}

$LeagueRuntimeName = 'MaxGameStudioLeagueToolbox.exe'
$LeagueInventoryName = 'maxgamestudio-runtime-hashes.json'
$LeagueUninstallerName = 'Uninstall MaxGameStudioLeagueToolbox.exe'
$LeagueNativeFiles = @($LeagueRuntimeName, 'WebView2Loader.dll', 'libunwind.dll', 'LeagueAkari-MIT.txt')

function Get-LeaguePayloadHashes([string]$Path, [switch]$Native) {
  $hashes = @{}
  $inventory = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
  if ($inventory -isnot [pscustomobject]) { throw 'Expected a program file inventory.' }
  foreach ($entry in $inventory.PSObject.Properties) {
    $relative = $entry.Name.Replace('\', '/')
    if ($relative -match '[:*?"<>|]' -or @($relative.Split('/') | Where-Object {
        !$_ -or $_ -in '.', '..' -or $_.EndsWith('.') -or $_.EndsWith(' ')
      }).Count -or $hashes.ContainsKey($relative) -or $entry.Value -cnotmatch '^[A-Fa-f0-9]{64}$' -or
        ($Native -and $relative -cnotin $LeagueNativeFiles)) {
      throw 'Invalid program file inventory entry.'
    }
    $hashes[$relative] = $entry.Value.ToLowerInvariant()
  }
  if (!$hashes.ContainsKey($LeagueRuntimeName)) { throw 'Program inventory lacks an executable.' }
  $hashes
}

function Test-LeagueFileHash([string]$Root, [string]$Relative, [string]$Digest) {
  $file = Get-LeagueOrdinaryFile $Root $Relative
  [bool]($file -and $Digest -and (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ieq $Digest)
}

function Assert-LeagueReplacement([string]$Root, [hashtable]$Replacement, [hashtable]$Trusted) {
  foreach ($relative in $Replacement.Keys) {
    $path = Join-Path $Root $relative
    $existing = Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    if ($existing -and !(Test-LeagueFileHash $Root $relative $Trusted[$relative])) {
      throw 'Native files would overwrite an unverified program object.'
    }
  }
}

function Remove-LeagueVerifiedFiles([string]$Root, [hashtable]$Trusted, [hashtable]$Keep) {
  # Only parents of files owned by the catalog can be pruned. Directory.Delete
  # omits recursive mode and cannot remove a user file added during cleanup.
  $parents = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($relative in $Trusted.Keys) {
    if ($Keep.ContainsKey($relative) -or !(Test-LeagueFileHash $Root $relative $Trusted[$relative])) { continue }
    $file = Get-LeagueOrdinaryFile $Root $relative
    [IO.File]::Delete($file.FullName)
    $parent = $file.Directory
    while (!$parent.FullName.Equals($Root, [StringComparison]::OrdinalIgnoreCase)) {
      $null = $parents.Add($parent.FullName)
      $parent = $parent.Parent
    }
  }
  foreach ($parent in ($parents | Sort-Object Length -Descending)) {
    $item = Get-Item -LiteralPath $parent -Force
    if (!($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
        !(Get-ChildItem -LiteralPath $parent -Force | Select-Object -First 1)) {
      [IO.Directory]::Delete($parent)
    }
  }
}

function Get-LeaguePriorHashes([string]$Root, [string]$Catalog, [hashtable]$Replacement) {
  $exe = Get-LeagueOrdinaryFile $Root $LeagueRuntimeName
  if (!$exe) { throw 'The existing League executable is missing.' }
  $digest = (Get-FileHash -LiteralPath $exe.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  $prior = (Get-Content -LiteralPath $Catalog -Raw | ConvertFrom-Json).PSObject.Properties[$digest].Value
  if ($prior) {
    $hashes = @{}
    foreach ($entry in $prior.PSObject.Properties) { $hashes[$entry.Name] = $entry.Value }
    return $hashes
  }
  if ($digest -eq $Replacement[$LeagueRuntimeName]) { return $Replacement }
  throw 'No verified upgrade inventory for the installed League executable.'
}

function Install-LeagueNativeFiles([string]$Root, [string]$Stage, [hashtable]$Replacement, [hashtable]$Prior) {
  foreach ($relative in $Replacement.Keys) {
    if (!(Test-LeagueFileHash $Stage $relative $Replacement[$relative])) { throw 'Staged program did not match its inventory.' }
  }
  Assert-LeagueReplacement $Root $Replacement $Prior

  $null = [IO.Directory]::CreateDirectory($Root)
  $files = [Collections.Generic.List[object]]::new()
  $committed = $false
  try {
    foreach ($relative in $Replacement.Keys) {
      $target = Join-Path $Root $relative
      $temporary = Join-Path $Root ('.mgs-league-' + [Guid]::NewGuid().ToString('n'))
      $file = [pscustomobject]@{
        Target = $target; Next = "$temporary.next"; Previous = "$temporary.previous"
        Expected = $Prior[$relative]; Applied = $false
      }
      $files.Add($file)
      [IO.File]::Copy((Join-Path $Stage $relative), $file.Next)
    }
    foreach ($file in $files) {
      if ($file.Expected -and [IO.File]::Exists($file.Target)) {
        [IO.File]::Replace($file.Next, $file.Target, $file.Previous)
      } else {
        [IO.File]::Move($file.Next, $file.Target)
      }
      $file.Applied = $true
    }
    $committed = $true
  } finally {
    # Keep a fallback if rollback itself fails. Never delete that backup.
    foreach ($file in $files) {
      if (!$committed -and $file.Applied) {
        if ([IO.File]::Exists($file.Previous)) {
          [IO.File]::Replace($file.Previous, $file.Target, [NullString]::Value)
        } else {
          [IO.File]::Delete($file.Target)
        }
      }
      [IO.File]::Delete($file.Next)
      if ($committed) { [IO.File]::Delete($file.Previous) }
    }
  }
}

function Remove-LeagueOwnedPayload([string]$Root, [string]$NewInventory) {
  $inventory = Get-LeagueOrdinaryFile $Root 'maxgamestudio-runtime-hashes.json'
  if (!$inventory) { return }
  try {
    $old = Get-Content -LiteralPath $inventory.FullName -Raw | ConvertFrom-Json
    $known = Get-Content -LiteralPath $NewInventory -Raw | ConvertFrom-Json
  } catch {
    return
  }
  foreach ($entry in $known.PSObject.Properties) {
    $hash = $old.PSObject.Properties[$entry.Name].Value
    if ($hash -notmatch '^[a-f0-9]{64}$') { continue }
    $file = Get-LeagueOrdinaryFile $Root $entry.Name
    if ($file -and (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -eq $hash) {
      Remove-Item -LiteralPath $file.FullName
    }
  }
}

function Remove-LeagueOwnedShortcut([string]$Root, [string]$Relative, [string]$ExpectedExe) {
  $link = Get-LeagueOrdinaryFile $Root $Relative
  if ($link -and [string]::Equals((Get-MgsShortcutTarget $link.FullName), $ExpectedExe,
      [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $link.FullName
  }
}

function Retire-LeagueUserInstall([string]$Root, [string]$NewInventory, [string]$Identity, [hashtable]$Prior) {
  $installKey = "Software\$Identity"
  $uninstallKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$Identity"
  $name = Read-Value "HKCU:\$installKey" 'ShortcutName'
  $menu = Read-Value "HKCU:\$installKey" 'MenuDirectory'
  if ($name -and $name -notmatch '[\\/:*?"<>|]' -and $name -notin '.', '..') {
    $desktop = [Environment]::GetFolderPath('DesktopDirectory')
    $programs = [Environment]::GetFolderPath('Programs')
    $menuPath = if ($menu -and $menu -notmatch '[\\/:*?"<>|]' -and $menu -notin '.', '..') {
      "$menu\$name.lnk"
    } else { "$name.lnk" }
    $expectedExe = Join-Path $Root 'MaxGameStudioLeagueToolbox.exe'
    Remove-LeagueOwnedShortcut $desktop "$name.lnk" $expectedExe
    Remove-LeagueOwnedShortcut $programs $menuPath $expectedExe
  }
  [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($uninstallKey, $false)
  [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($installKey, $false)
  if ($Prior) {
    Remove-LeagueVerifiedFiles $Root $Prior @{}
    return
  }
  $uninstaller = Get-LeagueOrdinaryFile $Root 'Uninstall MaxGameStudioLeagueToolbox.exe'
  if ($uninstaller) { Remove-Item -LiteralPath $uninstaller.FullName }
  Remove-LeagueOwnedPayload $Root $NewInventory
}

function Complete-LeagueUserInstall(
  [string]$Control, [string]$Selection, [string]$OldProgram, [string]$Identity,
  [switch]$Defer, [string]$NativeCatalog
) {
  if ($Defer) { return 4 }
  if ($Selection) {
    $null = [IO.Directory]::CreateDirectory($Control)
    $path = Join-Path $Control 'installer-data-location.txt'
    $next = "$path.$([Guid]::NewGuid().ToString('n')).next"
    try {
      [IO.File]::WriteAllText($next, $Selection, [Text.Encoding]::Unicode)
      if ([IO.File]::Exists($path)) {
        if (!(Get-LeagueOrdinaryFile $Control 'installer-data-location.txt')) {
          throw 'The data selection file is not an ordinary file.'
        }
        [IO.File]::Replace($next, $path, [NullString]::Value)
      } else {
        [IO.File]::Move($next, $path)
      }
    } finally {
      if ([IO.File]::Exists($next)) { [IO.File]::Delete($next) }
    }
  }
  if ($OldProgram) {
    $inventory = Join-Path $PSScriptRoot $LeagueInventoryName
    $prior = $null
    if ($NativeCatalog) {
      try { $prior = Get-LeaguePriorHashes $OldProgram $NativeCatalog @{} }
      catch {
        if ($_.Exception.Message -ceq 'No verified upgrade inventory for the installed League executable.') { return 9 }
        throw
      }
    }
    Retire-LeagueUserInstall $OldProgram $inventory $Identity $prior
  }
  return 0
}
