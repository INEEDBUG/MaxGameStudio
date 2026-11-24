param(
  [Parameter(Mandatory)][string]$Installer,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+-beta\.\d+$')][string]$Version,
  [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExecutableSha256
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows -or ![Environment]::Is64BitProcess -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Installed League startup verification requires an ephemeral x64 GitHub-hosted Windows runner.'
}
. (Join-Path $PSScriptRoot 'fixture-process-stop.ps1')
. (Join-Path $PSScriptRoot 'standard-user-fixture.ps1')
. (Join-Path $PSScriptRoot '../league-standalone/shortcut-target.ps1')
Add-Type -Path (Join-Path $PSScriptRoot 'AllUsersInstallerNative.cs')

$root = Join-Path $env:ProgramFiles 'MaxGameStudioLeagueToolbox'
$identity = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
$installKey = "HKLM:\Software\$identity"
$registry = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$identity"
$product = @{ Name='League'; Default=$root; Exe='MaxGameStudioLeagueToolbox.exe'; Version=$Version }
$installerInfo = Get-Item -LiteralPath $Installer
if ($installerInfo.Name -cne "MaxGameStudioLeague_${Version}_x64-setup.exe" -or
    (Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $installKey) -or (Test-Path -LiteralPath $registry)) {
  throw 'Expected the selected League candidate on a clean disposable runner.'
}
$hash = (Get-FileHash -LiteralPath $installerInfo.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
$sevenZip = Join-Path $env:ProgramFiles '7-Zip/7z.exe'
function Assert-NsisShellPlugins([string]$Package) {
  $contents = (& $sevenZip l -ba $Package) -join "`n"
  if ($LASTEXITCODE -ne 0 -or $contents -notmatch '\$PLUGINSDIR\\System\.dll') {
    throw "Could not list the League NSIS plugins in $Package."
  }
  if ($contents -match '\\WinShell\.dll\b') { throw "WinShell.dll still exists in $Package." }
}
Assert-NsisShellPlugins $installerInfo.FullName

$accounts = [Collections.Generic.List[Security.Principal.SecurityIdentifier]]::new()
try {
  $user = New-FixtureUser 'boot' $accounts
  Test-StandardUserAccess $user @() @()
  $process = Start-Process -FilePath $installerInfo.FullName -ArgumentList '/S /allusers' -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne 0) { throw "League machine installer failed with exit code $($process.ExitCode)." }
  $record = Get-ItemProperty -LiteralPath $registry
  $installedLocation = Get-ItemPropertyValue -LiteralPath $installKey -Name InstallLocation
  if ($record.DisplayVersion -cne $Version -or $installedLocation.Trim('"') -ine $root) {
    throw 'League setup did not register the planned native installation.'
  }
  $license = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../../../LICENSE-LEAGUE-MIT.txt'))
  $notices = [IO.File]::ReadAllText((Join-Path $root 'LeagueAkari-MIT.txt'))
  if (!$notices.StartsWith($license, [StringComparison]::Ordinal)) {
    throw 'The installed League notices do not include the scoped MIT license.'
  }
  Write-Host 'NATIVE_LEAGUE_LICENSE_VERIFIED: installed MIT notice matches source.'
  Assert-NsisShellPlugins (Join-Path $root 'Uninstall MaxGameStudioLeagueToolbox.exe')
  $shell = New-Object -ComObject Shell.Application
  $shortcutName = '英雄联盟工具箱.lnk'
  $target = Join-Path $root $product.Exe
  foreach ($folder in 'CommonDesktopDirectory', 'CommonPrograms') {
    $directory = [Environment]::GetFolderPath($folder)
    $shortcut = $shell.Namespace($directory).ParseName($shortcutName)
    if (!$shortcut -or $shortcut.ExtendedProperty('System.AppUserModel.ID') -cne 'com.maxgamestudio.league.toolbox') {
      throw "League $folder shortcut did not retain its taskbar group."
    }
    if ((Get-MgsShortcutTarget (Join-Path $directory $shortcutName)) -ine $target) {
      throw "League $folder shortcut points to an unexpected executable."
    }
  }
  Write-Host 'NATIVE_LEAGUE_SHORTCUTS_VERIFIED: setup and uninstaller without WinShell.dll, desktop and Start Menu targets and AppUserModelID preserved.'
  Invoke-NativeUpdaterProbe $product $user $ExecutableSha256
  if ((Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash -ine $hash) {
    throw 'The League installer changed during the startup test.'
  }
  Write-Host "NATIVE_LEAGUE_BOOT_VERIFIED: $($installerInfo.Name) sha256=$hash source=$env:GITHUB_SHA"
} finally {
  foreach ($account in $accounts) { Remove-LocalUser -SID $account }
}
