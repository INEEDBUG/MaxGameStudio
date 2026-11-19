function Write-MainInstallDiagnostic {
  param([string]$Setup, [string]$Program, [string]$ArpKey)
  Write-Host ('MAIN_MACHINE_STATE: ' + (@{
    programDirectory = Test-Path -LiteralPath $Program
    executable = Test-Path -LiteralPath (Join-Path $Program 'cs2-insight-agent-desktop.exe')
    python = Test-Path -LiteralPath (Join-Path $Program 'python\python.exe')
    registered = Test-Path -LiteralPath "HKLM:\$ArpKey"
    administrator = ([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  } | ConvertTo-Json -Compress))
  if (Test-Path -LiteralPath $Program -PathType Container) {
    $remaining = @(Get-ChildItem -LiteralPath $Program -Force -Recurse)
    $files = @($remaining | Where-Object { !$_.PSIsContainer })
    Write-Host ('MAIN_REMAINING_SUMMARY: ' + (@{ entries = $remaining.Count; files = $files.Count; bytes = ($files | Measure-Object Length -Sum).Sum } | ConvertTo-Json -Compress))
    foreach ($item in ($remaining | Select-Object -First 25)) {
      Write-Host ('MAIN_REMAINING_ENTRY: ' + (@{ path = $item.FullName.Substring($Program.Length); directory = $item.PSIsContainer; bytes = $item.Length; attributes = [string]$item.Attributes } | ConvertTo-Json -Compress))
    }
  }
  $windowsPowerShell = "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe"
  $validator = Join-Path $PSScriptRoot '../../../frontend/src-tauri/windows/validate-install-directories.ps1'
  foreach ($mode in @('direct', 'inner')) {
    $options = @('-MachineScope')
    if ($mode -eq 'inner') { $options += '-MachineFilesOnly' }
    & $windowsPowerShell -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $validator -ProgramDir "$Program\." @options
    Write-Host "MAIN_DIRECTORY_DIAGNOSTIC: mode=$mode exit=$LASTEXITCODE"
  }
  & "$env:WINDIR\System32\tasklist.exe" /FI 'IMAGENAME eq cs2-insight-agent-desktop.exe' /FO CSV /NH
  Write-Host "MAIN_TASKLIST_DIAGNOSTIC: exit=$LASTEXITCODE"
  $webview = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
  foreach ($hive in @('HKLM:', 'HKCU:')) {
    foreach ($key in @("SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$webview", "SOFTWARE\Microsoft\EdgeUpdate\Clients\$webview")) {
      $version = Get-ItemPropertyValue -LiteralPath "$hive\$key" -Name pv -ErrorAction SilentlyContinue
      Write-Host "MAIN_WEBVIEW_DIAGNOSTIC: $hive\$key pv=$version"
    }
  }
  foreach ($path in @($env:ProgramFiles, $Program)) {
    if (!(Test-Path -LiteralPath $path -PathType Container)) { continue }
    $acl = Get-Acl -LiteralPath $path
    Write-Host "MAIN_ACL_OWNER: $path $($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value)"
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
      Write-Host "MAIN_ACL_RULE: $($rule.IdentityReference.Value) $($rule.AccessControlType) $($rule.FileSystemRights) $($rule.InheritanceFlags) $($rule.PropagationFlags)"
    }
  }

  # /P skips input pages but exposes installer-owned abort text suppressed by /S.
  # This is a bounded diagnostic on the same disposable runner, never a PASS retry.
  $passive = Start-Process -FilePath $Setup -ArgumentList '/P /ALLUSERS' -WindowStyle Hidden -PassThru
  try {
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    $seen = [Collections.Generic.HashSet[string]]::new()
    do {
      foreach ($control in [MgsAllUsersNative]::VisibleControls([uint32]$passive.Id)) {
        if ($seen.Add($control.Text)) { Write-Host ('MAIN_PASSIVE_UI: ' + $control.Text) }
      }
      if ($passive.HasExited) { break }
      Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
  } finally {
    Stop-CiFixtureProcess $passive
  }
}
