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
  $digest = $files.PSObject.Properties[$executableName].Value
  if ($digest -cnotmatch '^[a-f0-9]{64}$' -or
      (Get-FileHash -LiteralPath $executable.FullName -Algorithm SHA256).Hash -ine $digest) {
    return
  }
  [pscustomobject]@{ Executable = $executable; Version = $versionInfo.ProductVersion; Digest = $digest }
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

function Retire-LeagueUserInstall([string]$Root, [string]$NewInventory, [string]$Identity) {
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
  $uninstaller = Get-LeagueOrdinaryFile $Root 'Uninstall MaxGameStudioLeagueToolbox.exe'
  if ($uninstaller) { Remove-Item -LiteralPath $uninstaller.FullName }
  Remove-LeagueOwnedPayload $Root $NewInventory
}

function Complete-LeagueUserInstall([string]$Control, [string]$Selection, [string]$OldProgram, [string]$Identity, [switch]$Defer) {
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
    Retire-LeagueUserInstall $OldProgram (Join-Path $PSScriptRoot 'maxgamestudio-runtime-hashes.json') $Identity
  }
  return 0
}
