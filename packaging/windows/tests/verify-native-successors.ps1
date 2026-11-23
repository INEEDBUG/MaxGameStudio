param(
  [Parameter(Mandatory)][ValidateSet('Catalog', 'Verify')][string]$Phase,
  [Parameter(Mandatory)][string]$Directory,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-beta\.\d+)?$')][string]$MainBaselineVersion,
  [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+(?:-beta\.\d+)?$')][string]$LeagueBaselineVersion,
  [ValidatePattern('^\d+\.\d+\.\d+(?:-beta\.\d+)?$')][string]$MainVersion,
  [ValidatePattern('^\d+\.\d+\.\d+(?:-beta\.\d+)?$')][string]$LeagueVersion,
  [string]$FixturePythonExe,
  [ValidatePattern('^\d+\.\d+\.\d+$')][string]$FixturePythonVersion = '3.12.10'
)

$ErrorActionPreference = 'Stop'
if (!$IsWindows -or ![Environment]::Is64BitProcess -or $env:GITHUB_ACTIONS -ne 'true' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or !$env:RUNNER_TEMP) {
  throw 'Native successor acceptance requires an ephemeral x64 GitHub-hosted Windows runner.'
}
if (!([Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'The hosted runner must be an administrator for the machine-scope fixture.'
}
if ($Phase -eq 'Verify' -and (!$MainVersion -or !$LeagueVersion -or
    [semver]$MainVersion -le [semver]$MainBaselineVersion -or [semver]$LeagueVersion -le [semver]$LeagueBaselineVersion)) {
  throw 'Verify requires two distinct successor versions newer than their baselines.'
}

function Assert-OrdinaryAncestry([string]$Path) {
  for ($parent = [IO.DirectoryInfo]::new($Path); $parent; $parent = $parent.Parent) {
    $item = Get-Item -LiteralPath $parent.FullName -Force
    if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
      throw 'A fixture directory is redirected or occupied by a file.'
    }
  }
}

$temp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$Directory = [IO.Path]::GetFullPath($Directory).TrimEnd('\')
$fixture = Join-Path $temp 'mgs-native-successors'
if (!$Directory.StartsWith($temp + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $Directory -eq $fixture -or $Directory.StartsWith($fixture + '\', [StringComparison]::OrdinalIgnoreCase) -or $Directory.Contains('"')) {
  throw 'The candidate directory must be an independent directory under RUNNER_TEMP.'
}
Assert-OrdinaryAncestry $Directory
Assert-OrdinaryAncestry $env:APPDATA
Assert-OrdinaryAncestry $env:ProgramFiles

$arp = 'Software\Microsoft\Windows\CurrentVersion\Uninstall'
$leagueId = 'b1d9ab62-895c-574e-a779-36d3f2ee3611'
$mainUserKey = 'HKCU:\Software\MaxGameStudio'
$storageKey = "$mainUserKey\Storage"
$leagueControl = Join-Path $env:APPDATA 'MaxGameStudioLeague'
$leagueLocator = Join-Path $leagueControl 'data-location.txt'
$main = @{
  Name = 'Main'; Caption = 'MaxGameStudio'; Stem = 'MaxGameStudio'; Old = $MainBaselineVersion; Next = $MainVersion
  Exe = 'cs2-insight-agent-desktop.exe'; Remove = 'uninstall.exe'
  Product = 'Software\cs2insightagent\MaxGameStudio'; Arp = "$arp\MaxGameStudio"; Location = ''
  Machine = Join-Path $env:ProgramFiles 'MaxGameStudio'; Personal = Join-Path $fixture 'Main program'
  Catalog = Join-Path $Directory 'main-native-predecessor.json'; Update = '/UPDATE'
  Required = @('data/basic.ini', 'data/cs2-insight.config.example.json', 'data/lite_cut_effect_contract.json',
    'data/lite_cut_visual_acceptance.json', 'backend/assets/fonts/NotoSansSC-Bold.ttf', 'backend/assets/fonts/NotoSansSC-Medium.ttf',
    'backend/assets/fonts/Rajdhani-Bold.ttf', 'backend/assets/fonts/Rajdhani-SemiBold.ttf', 'backend/assets/fonts/OFL.txt',
    'pov/gameinfo.gi.default', 'pov/gameinfo.gi.pov', 'pov/pov_default.vpk')
  Optional = @('WebView2Loader.dll', 'libunwind.dll', 'maxgamestudio-parser-worker.exe', 'maxgamestudio-parser-worker.LICENSE.txt')
}
$league = @{
  Name = 'League'; Caption = 'MaxGameStudio League'; Stem = 'MaxGameStudioLeague'; Old = $LeagueBaselineVersion; Next = $LeagueVersion
  Exe = 'MaxGameStudioLeagueToolbox.exe'; Remove = 'Uninstall MaxGameStudioLeagueToolbox.exe'
  Product = "Software\$leagueId"; Arp = "$arp\$leagueId"; Location = 'InstallLocation'
  Machine = Join-Path $env:ProgramFiles 'MaxGameStudioLeagueToolbox'; Personal = Join-Path $fixture 'League program'
  Catalog = Join-Path $Directory 'league-native-predecessor.json'; Update = '--updated'
  Required = @('maxgamestudio-runtime-hashes.json', 'LeagueAkari-MIT.txt'); Optional = @('WebView2Loader.dll', 'libunwind.dll')
}

function Assert-CleanRunner {
  $paths = @($fixture, $mainUserKey, $leagueControl, $main.Machine, $league.Machine,
    (Join-Path $env:LOCALAPPDATA 'MaxGameStudio'), (Join-Path $env:LOCALAPPDATA 'Programs\MaxGameStudio'),
    (Join-Path $env:LOCALAPPDATA 'Programs\MaxGameStudioLeagueToolbox'), (Join-Path $env:LOCALAPPDATA 'com.cs2insightagent.app'))
  foreach ($folder in 'MaxGameStudioLeague', 'MaxGameStudioLeagueToolbox') { $paths += Join-Path $env:LOCALAPPDATA $folder }
  foreach ($name in 'MaxGameStudio', 'MaxGameStudioData', 'CS2 Insight Agent', 'com.cs2insightagent.app', 'cs2-insight-agent') {
    $paths += Join-Path $env:APPDATA $name
  }
  foreach ($drive in [IO.DriveInfo]::GetDrives()) {
    if ($drive.DriveType -eq [IO.DriveType]::Fixed) { $paths += Join-Path $drive.RootDirectory.FullName 'MaxGameStudioData' }
  }
  foreach ($hive in 'HKCU:', 'HKLM:') {
    foreach ($key in $main.Product, $main.Arp, $league.Product, $league.Arp, "$arp\CS2 Ultimate Insight Studio", 'Software\MaxGameStudio\Storage') {
      $paths += "$hive\$key", "$hive\Software\WOW6432Node\$($key.Substring(9))"
    }
  }
  if ($paths | Where-Object { Test-Path -LiteralPath $_ }) { throw 'A product or fixture already exists on the runner.' }
  if (Get-Process -Name 'cs2-insight-agent-desktop', 'MaxGameStudioLeagueToolbox' -ErrorAction SilentlyContinue) {
    throw 'A product is running on the runner.'
  }
}

function Get-OrdinaryFileHash([string]$Path) {
  Assert-OrdinaryAncestry (Split-Path -Parent $Path)
  $item = Get-Item -LiteralPath $Path -Force
  if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or !$item.Length) {
    throw 'A required fixture file is empty, redirected or occupied by a directory.'
  }
  (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-Setup([string]$Path, [string]$ExpectedHash, [string]$Arguments) {
  if ((Get-OrdinaryFileHash $Path) -cne $ExpectedHash) { throw 'Installation executable changed after verification.' }
  Write-Host "NATIVE_SUCCESSOR_ATTEMPT: $([IO.Path]::GetFileName($Path)) $Arguments"
  $process = Start-Process -FilePath $Path -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
  if ($process.ExitCode -ne 0) { throw "$([IO.Path]::GetFileName($Path)) failed with exit $($process.ExitCode)." }
}

function Get-ScopeFlag($Product, [bool]$Machine) {
  $flag = if ($Machine) { '/allusers' } else { '/currentuser' }
  if ($Product.Name -eq 'Main') { return $flag.ToUpperInvariant() }
  return $flag
}

function Read-Installation($Product, [bool]$Machine, [string]$Version) {
  $hive = if ($Machine) { 'HKLM:' } else { 'HKCU:' }
  $otherHive = if ($Machine) { 'HKCU:' } else { 'HKLM:' }
  $root = if ($Machine) { $Product.Machine } else { $Product.Personal }
  $key = Get-Item -LiteralPath "$hive\$($Product.Product)"
  $record = Get-ItemProperty -LiteralPath "$hive\$($Product.Arp)"
  $uninstaller = Join-Path $root $Product.Remove
  $expectedUninstall = '"{0}"' -f $uninstaller
  if ($Product.Name -eq 'League') { $expectedUninstall += ' ' + (Get-ScopeFlag $Product $Machine) }
  if ($record.DisplayVersion -cne $Version -or ([string]$key.GetValue($Product.Location)).Trim('"') -ne $root -or
      ([string]$record.UninstallString).Trim() -ne $expectedUninstall -or
      (Test-Path -LiteralPath "$otherHive\$($Product.Product)") -or (Test-Path -LiteralPath "$otherHive\$($Product.Arp)") -or
      ($Product.Name -eq 'Main' -and (([string]$record.InstallLocation).Trim('"') -ne $root -or ($Machine -and $record.AllUsers -ne 1)))) {
    throw "$($Product.Name) did not keep the expected scope, registration and directory at $Version."
  }
  $hashes = @{}
  foreach ($relative in @($Product.Exe, $Product.Remove) + $Product.Required) {
    $hashes[$relative] = Get-OrdinaryFileHash (Join-Path $root $relative)
  }
  foreach ($relative in $Product.Optional) {
    $path = Join-Path $root $relative
    if (Test-Path -LiteralPath $path) { $hashes[$relative] = Get-OrdinaryFileHash $path }
  }
  $info = [Diagnostics.FileVersionInfo]::GetVersionInfo((Join-Path $root $Product.Exe))
  if ($info.ProductName -cne $Product.Caption -or $info.ProductVersion -cne $Version) {
    throw "$($Product.Name) installed executable identity or version did not match."
  }
  return $hashes
}

function Assert-Inventory($Actual, $Expected, [string]$Label) {
  if ($Actual.Count -ne $Expected.Count) { throw "$Label file count changed." }
  foreach ($file in $Expected.Keys) {
    if ($Actual[$file] -cne $Expected[$file]) { throw "$Label changed a verified file: $file" }
  }
}

function Remove-Installation($Product, [bool]$Machine, [string]$Version, $Inventory) {
  $root = if ($Machine) { $Product.Machine } else { $Product.Personal }
  $hive = if ($Machine) { 'HKLM:' } else { 'HKCU:' }
  Assert-Inventory (Read-Installation $Product $Machine $Version) $Inventory $Product.Name
  $arguments = '/S'
  if ($Product.Name -eq 'League') { $arguments += ' ' + (Get-ScopeFlag $Product $Machine) }
  Invoke-Setup (Join-Path $root $Product.Remove) $Inventory[$Product.Remove] $arguments
  if ((Test-Path -LiteralPath "$hive\$($Product.Arp)") -or
      ($Inventory.Keys | Where-Object { Test-Path -LiteralPath (Join-Path $root $_) })) {
    throw "$($Product.Name) did not remove its registered program files."
  }
  $key = "$hive\$($Product.Product)"
  if (!$Machine -and $Product.Name -eq 'Main' -and (Test-Path -LiteralPath $key)) {
    # The personal Main uninstaller intentionally keeps its previous directory and language.
    $record = Get-Item -LiteralPath $key
    $saved = $record.GetValue('')
    if (($saved -and $saved -ne $root) -or $record.SubKeyCount -or
        ($record.GetValueNames() | Where-Object { $_ -notin '', 'Installer Language' })) {
      throw 'Unexpected values in the fixture-owned Main registration.'
    }
    Remove-Item -LiteralPath $key
  }
  if (Test-Path -LiteralPath $key) { throw "$($Product.Name) left its launcher registration." }
  $dirs = foreach ($file in $Inventory.Keys) {
    for ($folder = Split-Path -Parent $file; $folder; $folder = Split-Path -Parent $folder) { Join-Path $root $folder }
  }
  foreach ($folder in @($dirs | Sort-Object -Unique | Sort-Object { $_.Length } -Descending) + $root) {
    if (Test-Path -LiteralPath $folder) {
      Assert-OrdinaryAncestry $folder
      [IO.Directory]::Delete($folder, $false)
    }
  }
}

function Invoke-Overlay($Product, [bool]$Machine, [switch]$Upgrade) {
  $root = if ($Machine) { $Product.Machine } else { $Product.Personal }
  $version = if ($Upgrade) { $Product.Next } else { $Product.Old }
  $arguments = '/S ' + (Get-ScopeFlag $Product $Machine)
  if ($Upgrade) { $arguments += ' ' + $Product.Update }
  # Main's native updater relies on the existing scoped registration, without /D.
  if ((!$Machine -and !$Upgrade) -or ($Upgrade -and $Product.Name -eq 'League')) { $arguments += " /D=$root" }
  $setup = $Product.Packages[$version]
  $hash = $Product.PackageHashes[$version]
  Invoke-Setup $setup $hash $arguments
  if ((Get-OrdinaryFileHash $setup) -cne $hash) { throw 'Installer bytes changed during the fixture.' }
  Read-Installation $Product $Machine $version
}

Assert-CleanRunner
foreach ($product in $main, $league) {
  $product.Packages = @{}
  $product.PackageHashes = @{}
  $versions = @($product.Old)
  if ($Phase -eq 'Verify') { $versions += $product.Next }
  foreach ($version in $versions) {
    $path = Join-Path $Directory "$($product.Stem)_${version}_x64-setup.exe"
    $product.Packages[$version] = $path
    $product.PackageHashes[$version] = Get-OrdinaryFileHash $path
  }
  if ($Phase -eq 'Catalog') {
    if (Test-Path -LiteralPath $product.Catalog) { throw 'A predecessor catalog already exists in the candidate directory.' }
  } else {
    if ($product.PackageHashes[$product.Old] -ceq $product.PackageHashes[$product.Next]) { throw 'Expected different setup files for the two versions.' }
    $null = Get-OrdinaryFileHash $product.Catalog
    $product.PublishedInventory = Get-Content -LiteralPath $product.Catalog -Raw | ConvertFrom-Json -AsHashtable
  }
}

function Assert-Profile($Profile) {
  if (!$Profile) { return }
  foreach ($file in $Profile.Hashes.Keys) {
    if ((Get-OrdinaryFileHash $file) -cne $Profile.Hashes[$file]) { throw 'An installer changed the fixture config, database or locator.' }
  }
  $storage = Get-ItemProperty -LiteralPath $storageKey
  if ($storage.LocationV1 -cne $Profile.MainLocator -or $storage.InstallerDataRootV1 -or
      (Test-Path -LiteralPath (Join-Path $leagueControl 'installer-data-location.txt'))) {
    throw 'An installer changed the profile locators or unexpectedly selected another path.'
  }
}

function New-Profile {
  $mainRoot = Join-Path $fixture 'Main profile'
  $mainData = Join-Path $mainRoot 'data'
  $leagueData = Join-Path $fixture 'League profile'
  $null = New-Item -ItemType Directory -Path $mainRoot, $mainData, $leagueData, $leagueControl
  $mainLocator = @{ root = $mainRoot; mode = 'unified'; required_paths = @($mainData) } | ConvertTo-Json -Compress
  $null = New-Item -Path $storageKey -Force
  $null = New-ItemProperty -LiteralPath $storageKey -Name LocationV1 -Value $mainLocator
  [IO.File]::WriteAllText($leagueLocator, $leagueData, [Text.Encoding]::Unicode)
  $profile = @{ MainLocator = $mainLocator; Hashes = @{ $leagueLocator = Get-OrdinaryFileHash $leagueLocator }; DbLabels = @{}
    Directories = @($mainData, $mainRoot, $leagueData, $leagueControl) }
  foreach ($row in @(
    @{ Name = 'main'; Config = Join-Path $mainData 'cs2-insight.config.json'; Db = Join-Path $mainData 'cs2-insight.db' },
    @{ Name = 'league'; Config = Join-Path $leagueData 'base-config.json'; Db = Join-Path $leagueData 'LeagueAkari.db' }
  )) {
    [IO.File]::WriteAllText($row.Config, '{"locale":"zh-CN"}')
    & $FixturePythonExe -I -B -c 'import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); c.execute("create table marker (label text)"); c.execute("insert into marker values (?)", (sys.argv[2],)); c.commit(); c.close()' $row.Db $row.Name
    if ($LASTEXITCODE -ne 0) { throw 'Cannot seed the test SQLite profile.' }
    $profile.Hashes[$row.Config] = Get-OrdinaryFileHash $row.Config
    $profile.Hashes[$row.Db] = Get-OrdinaryFileHash $row.Db
    $profile.DbLabels[$row.Db] = $row.Name
  }
  return $profile
}

if ($Phase -eq 'Verify') {
  if (!$FixturePythonExe) {
    $FixturePythonExe = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source
  }
  $FixturePythonExe = (Get-Item -LiteralPath $FixturePythonExe).FullName
  if ($FixturePythonExe.StartsWith($fixture + '\', [StringComparison]::OrdinalIgnoreCase) -or
      $FixturePythonExe.StartsWith($Directory + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Use external fixture Python.' }
  $pythonVersion = & $FixturePythonExe -I -B -c 'import sqlite3, sys; print(".".join(map(str, sys.version_info[:3])))'
  if ($LASTEXITCODE -ne 0 -or $pythonVersion -cne $FixturePythonVersion) { throw 'The pinned fixture interpreter is unavailable.' }
}

$null = New-Item -ItemType Directory -Path $fixture
foreach ($machine in $false, $true) {
  $scope = if ($machine) { 'machine' } else { 'personal' }
  foreach ($product in $main, $league) {
    $product.RemovalInventory = Invoke-Overlay $product $machine
    $product.RemovalVersion = $product.Old
    if ($Phase -eq 'Catalog' -and !$machine) { $product.Predecessor = $product.RemovalInventory }
    else {
      $expected = if ($Phase -eq 'Catalog') { $product.Predecessor }
        elseif ($product.Name -eq 'Main') { $product.PublishedInventory[$product.RemovalInventory[$product.Exe]] }
        else { $product.PublishedInventory }
      Assert-Inventory $product.RemovalInventory $expected "$scope $($product.Name) predecessor"
    }
  }
  $profile = $null
  if ($Phase -eq 'Verify') {
    $profile = New-Profile
    foreach ($product in $main, $league) {
      $other = if ($product.Name -eq 'Main') { $league } else { $main }
      $next = Invoke-Overlay $product $machine -Upgrade
      if ($next[$product.Exe] -ceq $product.RemovalInventory[$product.Exe]) { throw 'The installed executable did not advance.' }
      $product.RemovalVersion = $product.Next
      $product.RemovalInventory = $next
      Assert-Profile $profile
      Assert-Inventory (Read-Installation $other $machine $other.RemovalVersion) $other.RemovalInventory $other.Name
      Write-Host "NATIVE_SUCCESSOR_VERIFIED: $($product.Name) $scope $($product.Old) -> $($product.Next)"
    }
  }
  $first, $second = if ($machine) { $league, $main } else { $main, $league }
  Remove-Installation $first $machine $first.RemovalVersion $first.RemovalInventory
  Assert-Inventory (Read-Installation $second $machine $second.RemovalVersion) $second.RemovalInventory $second.Name
  Assert-Profile $profile
  Remove-Installation $second $machine $second.RemovalVersion $second.RemovalInventory
  Assert-Profile $profile
  if ($profile) {
    foreach ($db in $profile.DbLabels.Keys) {
      $label = & $FixturePythonExe -I -B -c 'import sqlite3, sys; from pathlib import Path; c = sqlite3.connect(Path(sys.argv[1]).resolve().as_uri() + "?mode=ro", uri=True); print(c.execute("select label from marker").fetchone()[0]); c.close()' $db
      if ($LASTEXITCODE -ne 0 -or $label -cne $profile.DbLabels[$db]) { throw 'The installed overlay or uninstall did not preserve the fixture SQLite row.' }
    }
    foreach ($file in $profile.Hashes.Keys) { [IO.File]::Delete($file) }
    foreach ($folder in $profile.Directories) { [IO.Directory]::Delete($folder, $false) }
    Remove-ItemProperty -LiteralPath $storageKey -Name LocationV1
    if ((Get-Item -LiteralPath $storageKey).ValueCount -or (Get-Item -LiteralPath $storageKey).SubKeyCount) { throw 'Unexpected fixture storage registration.' }
    Remove-Item -LiteralPath $storageKey
    if ((Get-Item -LiteralPath $mainUserKey).ValueCount -or (Get-Item -LiteralPath $mainUserKey).SubKeyCount) { throw 'Unexpected fixture product data registration.' }
    Remove-Item -LiteralPath $mainUserKey
  }
}
[IO.Directory]::Delete($fixture, $false)
Assert-CleanRunner
if ($Phase -eq 'Catalog') {
  $null = New-Item -ItemType File -Path $main.Catalog -Value (@{$main.Predecessor[$main.Exe] = $main.Predecessor} | ConvertTo-Json -Depth 4)
  $null = New-Item -ItemType File -Path $league.Catalog -Value ($league.Predecessor | ConvertTo-Json)
  Write-Host 'NATIVE_PREDECESSOR_CATALOGS_VERIFIED: both scopes have identical owned files, including uninstallers.'
} else {
  Write-Host 'NATIVE_SUCCESSOR_UNINSTALLS_VERIFIED: both scopes were cleaned, with independent removal and original profiles preserved.'
}
