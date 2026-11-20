$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'retire-user-install.ps1')

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('lg-scope-' + [Guid]::NewGuid().ToString('n'))
$old = Join-Path $scratch 'personal-install'
$profile = Join-Path $scratch 'private-data'
$activePointer = Join-Path $profile 'data-location.txt'
$pendingPointer = Join-Path $profile 'installer-data-location.txt'
$requested = Join-Path $scratch 'unapplied-custom-data'
$newInventory = Join-Path $scratch 'new-hashes.json'
$symbol = [char]::ConvertFromUtf32(0x1F680)
$oldLink = "old-$symbol.lnk"
$otherLink = "other-$symbol.lnk"
$invalidLink = "invalid-$symbol.lnk"
$fixtureLinks = @('old.lnk', 'other.lnk', $oldLink, $otherLink, $invalidLink)
New-Item -ItemType Directory -Path $old, $profile -Force | Out-Null
try {
  $exe = Join-Path $old 'MaxGameStudioLeagueToolbox.exe'
  $unknown = Join-Path $old 'keep.txt'
  $changed = Join-Path $old 'resources.pak'
  $data = Join-Path $profile 'settings.txt'
  Set-Content -LiteralPath $exe -Value 'known program'
  Set-Content -LiteralPath $changed -Value 'stock bytes'
  Set-Content -LiteralPath $unknown -Value 'user file'
  Set-Content -LiteralPath $data -Value 'private settings'
  $shell = New-Object -ComObject WScript.Shell
  try {
    foreach ($pair in @(@('old.lnk', $exe), @('other.lnk', $unknown))) {
      $link = $shell.CreateShortcut((Join-Path $scratch $pair[0]))
      try {
        $link.TargetPath = $pair[1]
        $link.Save()
      } finally {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($link)
      }
    }
  } finally {
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
  }
  [IO.File]::Copy((Join-Path $scratch 'old.lnk'), (Join-Path $scratch $oldLink))
  [IO.File]::Copy((Join-Path $scratch 'other.lnk'), (Join-Path $scratch $otherLink))
  [IO.File]::WriteAllText((Join-Path $scratch $invalidLink), 'not a shortcut')
  if ((Get-MgsShortcutTarget (Join-Path $scratch $oldLink)) -ne $exe -or
      (Get-MgsShortcutTarget (Join-Path $scratch $otherLink)) -ne $unknown -or
      (Get-MgsShortcutTarget (Join-Path $scratch $invalidLink))) {
    throw 'The Unicode shortcut reader did not return the expected raw targets.'
  }
  foreach ($name in $oldLink, $otherLink, $invalidLink) {
    Remove-LeagueOwnedShortcut $scratch $name $exe
  }
  if ((Test-Path -LiteralPath (Join-Path $scratch $oldLink)) -or
      !(Test-Path -LiteralPath (Join-Path $scratch $otherLink) -PathType Leaf) -or
      !(Test-Path -LiteralPath (Join-Path $scratch $invalidLink) -PathType Leaf)) {
    throw 'Scope retirement must remove the exact old target and retain other or unreadable links.'
  }
  Write-Host 'Unicode shortcuts read correctly. Only the exact old program link was removed.'
  [IO.File]::WriteAllText($activePointer, $profile, [Text.Encoding]::Unicode)
  [IO.File]::WriteAllText($pendingPointer, $profile, [Text.Encoding]::Unicode)
  $before = @(Get-FileHash -LiteralPath $activePointer, $pendingPointer).Hash
  $status = Complete-LeagueUserInstall $profile $requested '' '' -Defer
  $after = @(Get-FileHash -LiteralPath $activePointer, $pendingPointer).Hash
  if ($status -ne 4 -or (Compare-Object $before $after) -or (Test-Path -LiteralPath $requested)) {
    throw 'An already-elevated machine install attempted a private data change.'
  }
  Write-Host 'Elevated machine completion retained original private locator bytes.'
  $status = Complete-LeagueUserInstall $profile $requested '' ''
  if ($status -ne 0 -or [IO.File]::ReadAllText($pendingPointer, [Text.Encoding]::Unicode) -ne $requested -or
      (Get-FileHash -LiteralPath $activePointer).Hash -ne $before[0]) {
    throw 'A normal selection did not write the pending directory while preserving the active locator.'
  }
  $hashes = @{
    'MaxGameStudioLeagueToolbox.exe' = (Get-FileHash -LiteralPath $exe).Hash
    'resources.pak' = (Get-FileHash -LiteralPath $changed).Hash
    'keep.txt' = (Get-FileHash -LiteralPath $unknown).Hash
    '..\private-data\settings.txt' = (Get-FileHash -LiteralPath $data).Hash
  }
  $hashes | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $old 'maxgamestudio-runtime-hashes.json')
  $new = @{
    'MaxGameStudioLeagueToolbox.exe' = $hashes['MaxGameStudioLeagueToolbox.exe']
    'resources.pak' = $hashes['resources.pak']
    '..\private-data\settings.txt' = $hashes['..\private-data\settings.txt']
  }
  $new | ConvertTo-Json | Set-Content -LiteralPath $newInventory
  Set-Content -LiteralPath $changed -Value 'modified program file'
  Remove-LeagueOwnedPayload $old $newInventory
  if ((Test-Path -LiteralPath $exe) -or
      (Get-Content -LiteralPath $unknown) -ne 'user file' -or
      (Get-Content -LiteralPath $changed) -ne 'modified program file' -or
      (Get-Content -LiteralPath $data) -ne 'private settings') {
    throw 'Scope retirement did not retain private, unknown and modified files.'
  }
  Write-Host 'Known program file removed. Private, unknown and modified files retained.'
  # The native installer must use its own verified catalog, even if the
  # installed Electron inventory claims an unrelated file.
  $obsolete = Join-Path $old 'old-electron.dll'
  $legacyInventory = Join-Path $scratch 'legacy-hashes.json'
  Set-Content -LiteralPath $obsolete -Value 'shipped Electron bytes'
  Set-Content -LiteralPath $exe -Value 'shipped Electron executable'
  $oldInventory = Join-Path $old 'maxgamestudio-runtime-hashes.json'
  @{
    'MaxGameStudioLeagueToolbox.exe' = (Get-FileHash -LiteralPath $exe).Hash
    'maxgamestudio-runtime-hashes.json' = (Get-FileHash -LiteralPath $oldInventory).Hash
    'old-electron.dll' = (Get-FileHash -LiteralPath $obsolete).Hash
    'resources.pak' = $hashes['resources.pak']
  } | ConvertTo-Json | Set-Content -LiteralPath $legacyInventory
  $trusted = Get-LeaguePayloadHashes $legacyInventory
  @{ 'MaxGameStudioLeagueToolbox.exe' = $hashes['MaxGameStudioLeagueToolbox.exe'] } |
    ConvertTo-Json | Set-Content -LiteralPath $newInventory
  $replacement = Get-LeaguePayloadHashes $newInventory -Native
  Assert-LeagueReplacement $old $replacement $trusted
  $collision = Join-Path $old 'libunwind.dll'
  Set-Content -LiteralPath $collision -Value 'unrelated file with the same name'
  $replacement['libunwind.dll'] = $hashes['resources.pak']
  $blocked = $false
  try { Assert-LeagueReplacement $old $replacement $trusted } catch { $blocked = $true }
  if (!$blocked -or (Get-Content -LiteralPath $collision) -ne 'unrelated file with the same name') {
    throw 'Native replacement did not reject an unrelated file with a colliding path.'
  }
  Remove-LeagueVerifiedFiles $old $trusted $replacement
  if ((Test-Path -LiteralPath $obsolete) -or !(Test-Path -LiteralPath $exe) -or
      (Get-Content -LiteralPath $unknown) -ne 'user file' -or (Get-Content -LiteralPath $changed) -ne 'modified program file') {
    throw 'Native retirement removed unrelated/changed files or retained exact obsolete Electron files.'
  }
  $stage = Join-Path $scratch 'stage'
  New-Item -ItemType Directory -Path $stage | Out-Null
  $newExe = Join-Path $stage 'MaxGameStudioLeagueToolbox.exe'
  Set-Content -LiteralPath $newExe -Value 'native replacement'
  $digest = (Get-FileHash -LiteralPath $newExe).Hash
  Remove-Item -LiteralPath $collision
  $replacement.Remove('libunwind.dll')
  $replacement['MaxGameStudioLeagueToolbox.exe'] = $digest
  Install-LeagueNativeFiles $old $stage $replacement $trusted
  if ((Get-FileHash -LiteralPath $exe).Hash -ne $digest -or
      (Get-ChildItem -LiteralPath $old -Force -Filter '.mgs-league-*')) {
    throw 'Native payload installation did not commit its file cleanly.'
  }
  Remove-LeagueVerifiedFiles $old $replacement @{}
  if ((Test-Path -LiteralPath $exe) -or !(Test-Path -LiteralPath $unknown)) {
    throw 'Native payload uninstall did not retain the unrelated original file.'
  }
  Write-Host 'Native overlay rejects unknown collisions and retires only verified obsolete files.'
} finally {
  foreach ($name in $fixtureLinks) {
    $file = Join-Path $scratch $name
    if (Test-Path -LiteralPath $file -PathType Leaf) { Remove-Item -LiteralPath $file }
  }
  foreach ($file in @($newInventory, $legacyInventory, $obsolete, $collision, $exe, $changed, $unknown, $data, $activePointer, $pendingPointer, (Join-Path $old 'maxgamestudio-runtime-hashes.json'))) {
    if ($file -and (Test-Path -LiteralPath $file -PathType Leaf)) { Remove-Item -LiteralPath $file }
  }
  if ($newExe -and (Test-Path -LiteralPath $newExe)) { Remove-Item -LiteralPath $newExe }
  if ($stage -and (Test-Path -LiteralPath $stage)) { Remove-Item -LiteralPath $stage }
  foreach ($directory in @($profile, $old, $scratch)) { Remove-Item -LiteralPath $directory }
}
