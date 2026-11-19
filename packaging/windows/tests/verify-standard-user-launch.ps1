$ErrorActionPreference = 'Stop'
if (!$IsWindows -or ![Environment]::Is64BitProcess -or
    $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Standard-user launch acceptance requires an ephemeral x64 GitHub-hosted Windows runner.'
}
. (Join-Path $PSScriptRoot 'fixture-process-stop.ps1')
. (Join-Path $PSScriptRoot 'standard-user-fixture.ps1')

$accounts = [Collections.Generic.List[Security.Principal.SecurityIdentifier]]::new()
try {
  $user = New-FixtureUser 'P' $accounts
  Test-StandardUserAccess $user @() @()
  Write-Host 'STANDARD_USER_LAUNCH_VERIFIED: short file probe ran with the expected standard SID, private environment and read-only fixture.'
} finally {
  foreach ($account in $accounts) { Remove-LocalUser -SID $account }
}
