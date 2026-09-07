param([Parameter(Mandatory=$true)][string]$Installer)
$ErrorActionPreference = 'Stop'
# This test intentionally uses the real product keys. It must never run on a
# developer/customer machine, only an ephemeral GitHub-hosted Windows runner.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
  throw 'Packaged upgrade acceptance is restricted to ephemeral GitHub-hosted runners.'
}
$productKey = 'HKCU:\Software\cs2insightagent\MaxGameStudio'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\MaxGameStudio'
if ((Test-Path $productKey) -or (Test-Path $uninstallKey)) { throw 'Runner already contains a product registration; refusing to overwrite it.' }
$root = Join-Path $env:RUNNER_TEMP 'mgs-upgrade-acceptance'
if (Test-Path -LiteralPath $root) { throw 'Acceptance directory already exists.' }
$oldDirectory = Join-Path $root 'Custom Install'
$expectedDirectory = Join-Path $root 'Expected'
New-Item -ItemType Directory -Path $oldDirectory,$expectedDirectory | Out-Null
$binaryName = 'cs2-insight-agent-desktop.exe'
# The marker cannot execute and differs from every real release executable.
[IO.File]::WriteAllText((Join-Path $oldDirectory $binaryName), 'old-installation-fixture')
New-Item -Path $productKey -Force | Out-Null
Set-Item -LiteralPath $productKey -Value $oldDirectory
New-Item -Path $uninstallKey -Force | Out-Null
New-ItemProperty -Path $uninstallKey -Name DisplayVersion -Value '3.1.2' | Out-Null
New-ItemProperty -Path $uninstallKey -Name InstallLocation -Value ('"' + $oldDirectory + '"') | Out-Null
New-ItemProperty -Path $uninstallKey -Name MainBinaryName -Value $binaryName | Out-Null
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Desktop')) 'MaxGameStudio.lnk'
if (Test-Path -LiteralPath $shortcutPath) { throw 'Unexpected existing desktop shortcut.' }
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = Join-Path $oldDirectory $binaryName
$shortcut.Save()
& 'C:\Program Files\7-Zip\7z.exe' e $Installer "-o$expectedDirectory" $binaryName -y | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect candidate executable.' }
# Exactly the updater's passive mode, with no explicit /D override and no /R.
$process = Start-Process -FilePath $Installer -ArgumentList '/P /UPDATE' -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Installer returned $($process.ExitCode)" }
$installed = Join-Path $oldDirectory $binaryName
if ((Get-FileHash $installed).Hash -ne (Get-FileHash (Join-Path $expectedDirectory $binaryName)).Hash) { throw 'The original executable was not replaced by the candidate.' }
if ((Get-Item $installed).VersionInfo.FileVersion -ne '3.1.4') { throw 'Wrong installed version.' }
if ((Get-ItemProperty $productKey).'(default)' -ne $oldDirectory) { throw 'Product registration changed directory.' }
$record = Get-ItemProperty $uninstallKey
if ($record.DisplayVersion -ne '3.1.4' -or $record.InstallLocation.Trim('"') -ne $oldDirectory) { throw 'Uninstall record is stale or points elsewhere.' }
if ($shell.CreateShortcut($shortcutPath).TargetPath -ne $installed) { throw 'Shortcut no longer points to the original installation.' }
if (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA "MaxGameStudio\$binaryName")) { throw 'An unintended default-directory copy was created.' }
Write-Host 'PACKAGED_UPGRADE_VERIFIED: original executable replaced; registry and desktop shortcut agree; no default-directory duplicate.'
