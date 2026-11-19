$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
if (!$IsWindows) { throw 'The process teardown fixture requires Windows.' }
. (Join-Path $PSScriptRoot 'fixture-process-stop.ps1')

$fixtureShell = (Get-Process -Id $PID).Path
function New-FakeApp {
  $arguments = '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'
  Start-Process -FilePath $fixtureShell -ArgumentList $arguments -WindowStyle Hidden -PassThru
}

$app = New-FakeApp
try {
  Stop-CiFixtureProcess $app -ExitWaitMilliseconds 500 -ForceStop {
    param($owned)
    $owned.Kill()
    if (!$owned.WaitForExit(5000)) { throw 'Synthetic app did not exit.' }
    & $env:ComSpec /d /c 'exit 1'
  }
  if (!$app.HasExited -or $global:LASTEXITCODE -ne 0) {
    throw 'A benign teardown failure still poisoned the successful shell exit code.'
  }
} finally {
  if (!$app.HasExited) { $app.Kill(); $app.WaitForExit() }
  $app.Dispose()
}

$app = New-FakeApp
try {
  $reported = $false
  try {
    Stop-CiFixtureProcess $app -ExitWaitMilliseconds 500 -ForceStop {
      param($owned)
      & $env:ComSpec /d /c 'exit 1'
    }
  } catch {
    if ($_.Exception.Message -notmatch 'Owned fixture processes did not exit') { throw }
    $reported = $true
  }
  if (!$reported -or $app.HasExited -or $global:LASTEXITCODE -ne 1) {
    throw 'Teardown suppressed a real fixture process failure.'
  }
} finally {
  if (!$app.HasExited) { $app.Kill(); $app.WaitForExit() }
  $app.Dispose()
}

# A root-only success must also fail while an exact descendant remains.
$childSource = 'Start-Sleep -Seconds 30'
$encodedChild = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childSource))
$escaped = $fixtureShell.Replace("'", "''")
$parentSource = "Start-Process -FilePath '$escaped' -ArgumentList '-NoProfile -EncodedCommand $encodedChild' -WindowStyle Hidden; Start-Sleep -Seconds 30"
$encodedParent = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($parentSource))
$app = Start-Process -FilePath $fixtureShell -ArgumentList "-NoProfile -EncodedCommand $encodedParent" -WindowStyle Hidden -PassThru
$child = $null
try {
  $deadline = [DateTime]::UtcNow.AddSeconds(5)
  do {
    $name = [IO.Path]::GetFileName($fixtureShell)
    $row = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($app.Id) AND Name='$name'" -Property ProcessId
    if ($row) { $child = [Diagnostics.Process]::GetProcessById($row.ProcessId); break }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  if (!$child) { throw 'Synthetic descendant did not start.' }
  $reported = $false
  try {
    Stop-CiFixtureProcess $app -ExitWaitMilliseconds 500 -ForceStop {
      param($owned)
      $owned.Kill()
      if (!$owned.WaitForExit(5000)) { throw 'Synthetic root did not exit.' }
      & $env:ComSpec /d /c 'exit 1'
    }
  } catch {
    if ($_.Exception.Message -notmatch "Owned fixture processes did not exit \(PIDs $($child.Id),") { throw }
    $reported = $true
  }
  if (!$reported -or !$app.HasExited -or $child.HasExited -or $global:LASTEXITCODE -ne 1) {
    throw 'Teardown treated an exited root as proof its descendant also exited.'
  }
} finally {
  if (!$app.HasExited) { $app.Kill($true); $app.WaitForExit() }
  if ($child) {
    if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() }
    $child.Dispose()
  }
  $app.Dispose()
}

$global:LASTEXITCODE = 0
'FIXTURE_PROCESS_STOP_VERIFIED'
