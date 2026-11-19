function Stop-CiFixtureProcess {
  param(
    [Parameter(Mandatory)][Diagnostics.Process]$App,
    [switch]$TryClose,
    [int]$GracefulMilliseconds = 5000,
    [int]$ExitWaitMilliseconds = 5000,
    [scriptblock]$ForceStop = {
      param($OwnedApp)
      & "$env:SystemRoot\System32\taskkill.exe" /PID "$($OwnedApp.Id)" /T /F
    }
  )

  $rootId = $App.Id
  $owned = [Collections.Generic.Dictionary[int, datetime]]::new()
  $owned.Add($rootId, $App.StartTime.ToUniversalTime())
  $snapshot = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate)
  do {
    $added = $false
    foreach ($row in $snapshot) {
      $id = [int]$row.ProcessId
      $parent = [int]$row.ParentProcessId
      if ($owned.ContainsKey($id) -or !$owned.ContainsKey($parent)) { continue }
      if (!$row.CreationDate) { throw 'Could not identify an owned fixture descendant.' }
      $created = $row.CreationDate.ToUniversalTime()
      # CIM rounds creation time to microseconds. Ignore children of a prior
      # process that held the same parent PID.
      if ($created.AddTicks(9) -lt $owned[$parent]) { continue }
      $owned.Add($id, $created)
      $added = $true
    }
  } while ($added)

  if ($TryClose -and !$App.HasExited -and $App.CloseMainWindow()) {
    $null = $App.WaitForExit($GracefulMilliseconds)
  }
  $nativeExit = $null
  $nativeOutput = @()
  if (!$App.HasExited) {
    # Capture taskkill's status first, then decide success by process identity.
    $PSNativeCommandUseErrorActionPreference = $false
    $nativeOutput = @(& $ForceStop $App 2>&1)
    $nativeExit = $LASTEXITCODE
  }

  $clock = [Diagnostics.Stopwatch]::StartNew()
  do {
    $running = @()
    if (!$App.HasExited) { $running += $rootId }
    foreach ($entry in $owned.GetEnumerator()) {
      if ($entry.Key -eq $rootId) { continue }
      try { $process = [Diagnostics.Process]::GetProcessById($entry.Key) }
      catch [ArgumentException] { continue }
      try {
        if (!$process.HasExited -and [Math]::Abs(($process.StartTime.ToUniversalTime() - $entry.Value).Ticks) -lt 10) {
          $running += $entry.Key
        }
      } catch [InvalidOperationException] {
        if (!$process.HasExited) { throw }
      } finally {
        $process.Dispose()
      }
    }
    if ($running.Count -eq 0) {
      if ($null -ne $nativeExit) { $global:LASTEXITCODE = 0 }
      return
    }
    $remaining = $ExitWaitMilliseconds - $clock.ElapsedMilliseconds
    if ($remaining -le 0) { break }
    Start-Sleep -Milliseconds ([math]::Min($remaining, 150))
  } while ($true)

  $detail = ($nativeOutput | Select-Object -Last 1 | Out-String).Trim()
  throw "Owned fixture processes did not exit (PIDs $($running -join ', '), native exit $nativeExit). $detail"
}
