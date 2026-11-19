$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'retire-user-install.ps1')

$scratch = Join-Path ([IO.Path]::GetTempPath()) ('lg-scope-' + [Guid]::NewGuid().ToString('n'))
$old = Join-Path $scratch 'personal-install'
$profile = Join-Path $scratch 'private-data'
$activePointer = Join-Path $profile 'data-location.txt'
$pendingPointer = Join-Path $profile 'installer-data-location.txt'
$requested = Join-Path $scratch 'unapplied-custom-data'
$newInventory = Join-Path $scratch 'new-hashes.json'
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
} finally {
  foreach ($file in @($newInventory, $exe, $changed, $unknown, $data, $activePointer, $pendingPointer, (Join-Path $old 'maxgamestudio-runtime-hashes.json'))) {
    if ($file -and (Test-Path -LiteralPath $file -PathType Leaf)) { Remove-Item -LiteralPath $file }
  }
  foreach ($directory in @($profile, $old, $scratch)) { Remove-Item -LiteralPath $directory }
}
