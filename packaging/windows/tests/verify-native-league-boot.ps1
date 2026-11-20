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
  Invoke-NativeUpdaterProbe $product $user $ExecutableSha256
  if ((Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash -ine $hash) {
    throw 'The League installer changed during the startup test.'
  }
  Write-Host "NATIVE_LEAGUE_BOOT_VERIFIED: $($installerInfo.Name) sha256=$hash source=$env:GITHUB_SHA"
} finally {
  foreach ($account in $accounts) { Remove-LocalUser -SID $account }
}
