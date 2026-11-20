$ErrorActionPreference = 'Stop'
$id = [Guid]::NewGuid().ToString('n')
$scratch = Join-Path ([IO.Path]::GetTempPath()) "mgs-league-guard-$id"
$registry = "HKCU:\Software\MaxGameStudioInstallerTests\$id"
$product = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
$arp = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
$old = Join-Path $scratch 'Old Toolbox'
$fresh = Join-Path $scratch 'Fresh Toolbox'
$uninstaller = Join-Path $old 'Uninstall MaxGameStudioLeagueToolbox.exe'
$userKey = "$registry\HKCU\Software\$product"
$userArp = "$registry\HKCU\$arp"
$machineKey = "$registry\HKLM\Software\$product"
$machineArp = "$registry\HKLM\$arp"
$profile = Join-Path $scratch 'AppData\MaxGameStudioLeague\Profile'
$keys = @($userKey, $userArp, $machineKey, $machineArp)
$environment = @{}
foreach ($name in 'APPDATA', 'LOCALAPPDATA', 'MGS_LEAGUE_INSTALL_PHASE', 'MGS_LEAGUE_INSTALL_SCOPE',
    'MGS_LEAGUE_INSTALL_DEST', 'MGS_LEAGUE_DATA_DEST', 'MGS_TEST_FIXTURE_REGISTRY') {
  $environment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$shell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'

function Assert-Guard([string]$Label, [string]$Phase, [string]$Scope, [string]$Destination, [int]$Exit) {
  $env:MGS_LEAGUE_INSTALL_PHASE = $Phase
  $env:MGS_LEAGUE_INSTALL_SCOPE = $Scope
  $env:MGS_LEAGUE_INSTALL_DEST = $Destination
  & $shell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $guard
  if ($LASTEXITCODE -ne $Exit) { throw "$Label returned $LASTEXITCODE, expected $Exit." }
}

[IO.Directory]::CreateDirectory((Join-Path $old 'resources')) | Out-Null
try {
  $guard = Join-Path $scratch 'destination-guard.ps1'
  $source = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'destination-guard.ps1'))
  $read = 'function Read-Value([string]$Key, [string]$Name) {'
  if ($source.IndexOf($read) -lt 0) { throw 'Registry reader test seam changed.' }
  # Redirect just the registry reads in the copied script. Both fixture hives live in a private HKCU key.
  $source = $source.Replace($read, ($read + "`n" + '  $Key = Join-Path $env:MGS_TEST_FIXTURE_REGISTRY $Key.Replace('':'', '''')'))
  [IO.File]::WriteAllText($guard, $source)
  foreach ($file in 'retire-user-install.ps1', 'shortcut-target.ps1') {
    [IO.File]::Copy((Join-Path $PSScriptRoot $file), (Join-Path $scratch $file))
  }
  foreach ($key in $keys) {
    New-Item -Path $key -Force | Out-Null
  }
  $env:APPDATA = Join-Path $scratch 'AppData'
  $env:LOCALAPPDATA = Join-Path $scratch 'LocalAppData'
  $env:MGS_TEST_FIXTURE_REGISTRY = $registry
  $env:MGS_LEAGUE_DATA_DEST = $null
  foreach ($relative in 'MaxGameStudioLeagueToolbox.exe', 'Uninstall MaxGameStudioLeagueToolbox.exe', 'resources\app.asar', 'keep.txt') {
    [IO.File]::WriteAllText((Join-Path $old $relative), 'old')
  }
  [IO.Directory]::CreateDirectory($profile) | Out-Null
  $data = Join-Path $profile 'settings.txt'
  [IO.File]::WriteAllText($data, 'user settings')

  Assert-Guard 'Clean registration' 'registrations' 'CurrentUser' '' 0
  Set-ItemProperty -LiteralPath $userKey -Name InstallLocation -Value $old
  Assert-Guard 'Missing uninstall string permits the wizard' 'registrations' 'CurrentUser' '' 8
  Assert-Guard 'A separate program directory is safe to install into' 'selection' 'CurrentUser' $fresh 5
  Assert-Guard 'An occupied old folder cannot be used as an owned upgrade' 'selection' 'CurrentUser' $old 2
  Assert-Guard 'A child of the old folder is not independent' 'selection' 'CurrentUser' (Join-Path $old 'Nested') 2

  Set-ItemProperty -LiteralPath $userArp -Name UninstallString -Value "bad command `"$uninstaller`""
  Assert-Guard 'A malformed command is never proof of ownership' 'selection' 'CurrentUser' $fresh 5
  Remove-ItemProperty -LiteralPath $userKey -Name InstallLocation
  Set-ItemProperty -LiteralPath $userArp -Name UninstallString -Value "`"$uninstaller`" /currentuser"
  Assert-Guard 'An uninstall string alone permits the wizard' 'registrations' 'CurrentUser' '' 8
  Assert-Guard 'An uninstall string alone permits an independent new target' 'selection' 'CurrentUser' $fresh 5
  Assert-Guard 'An uninstall string alone protects the referenced folder' 'selection' 'CurrentUser' $old 2

  Set-ItemProperty -LiteralPath $userKey -Name InstallLocation -Value $old
  $appExe = Join-Path $old 'MaxGameStudioLeagueToolbox.exe'
  [IO.File]::Delete($appExe)
  Assert-Guard 'A registered folder with no executable permits the recovery wizard' 'registrations' 'CurrentUser' '' 8
  Assert-Guard 'A missing executable permits a separate installation' 'selection' 'CurrentUser' $fresh 5
  Assert-Guard 'A registered partial folder is preserved' 'selection' 'CurrentUser' $old 2
  Assert-Guard 'A missing executable prevents registering a copy' 'payload' 'CurrentUser' $old 7
  [IO.File]::WriteAllText($appExe, '')
  Assert-Guard 'An empty executable prevents registering a copy' 'payload' 'CurrentUser' $old 7
  [IO.File]::WriteAllText($appExe, 'old')
  Assert-Guard 'All required files permit registration' 'payload' 'CurrentUser' $old 0

  Set-ItemProperty -LiteralPath $machineKey -Name InstallLocation -Value (Join-Path $scratch 'Missing old common folder')
  Assert-Guard 'Only the incomplete machine registration is flagged' 'registrations' 'CurrentUser' '' 16
  Assert-Guard 'A complete user registration still allows an in-place upgrade' 'selection' 'CurrentUser' $old 0
  Set-ItemProperty -LiteralPath $machineArp -Name UninstallString -Value "`"$uninstaller`" /allusers"
  Assert-Guard 'A mismatched complete pair stays blocked' 'selection' 'CurrentUser' $fresh 2
  if ([IO.File]::ReadAllText($data) -ne 'user settings' -or [IO.File]::ReadAllText((Join-Path $old 'keep.txt')) -ne 'old' -or
      (Test-Path -LiteralPath $fresh)) {
    throw 'Preflight changed the old data, old files or destination.'
  }
  Write-Host 'LEAGUE_DESTINATION_GUARD_VERIFIED'
} finally {
  foreach ($name in $environment.Keys) { [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process') }
  if (Test-Path -LiteralPath $registry) { Remove-Item -LiteralPath $registry -Recurse -Force }
  $resolved = (Resolve-Path -LiteralPath $scratch).Path
  $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
  if ($resolved.StartsWith($temp + 'mgs-league-guard-', [StringComparison]::OrdinalIgnoreCase) -and
      !((Get-Item -LiteralPath $resolved -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}
