param(
  [Parameter(Mandatory)][ValidateSet('Build', 'Install', 'Uninstall', 'Retire')][string]$Phase,
  [Parameter(Mandatory)][string]$Program,
  [string]$Stage,
  [string]$Manifest,
  [string]$Catalog,
  [string]$CatalogSHA256,
  [string]$Version,
  [string]$UninstallerImage,
  [switch]$Gnu
)

$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$ErrorActionPreference = 'Stop'
$exe = 'cs2-insight-agent-desktop.exe'
$uninstaller = 'uninstall.exe'
$assets = @('data/basic.ini', 'data/cs2-insight.config.example.json',
  'data/lite_cut_effect_contract.json', 'data/lite_cut_visual_acceptance.json',
  'pov/gameinfo.gi.default', 'pov/gameinfo.gi.pov', 'pov/pov_default.vpk')
$sidecars = @('WebView2Loader.dll', 'libunwind.dll')

function Get-Hash([string]$Path) {
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Read-Inventory([string]$Path, [string[]]$Allowed) {
  $contents = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
  $files = @{}
  foreach ($entry in $contents.PSObject.Properties) {
    $relative = $entry.Name
    $known = $relative -cin $Allowed -or (!$Allowed -and
      ($relative -cin @($exe, $uninstaller, '.gitkeep') + $assets + $sidecars -or
        $relative -cmatch '^(python|backend|league-runtime)/'))
    if (!$known -or $relative -match '[\\:*?"<>|]' -or
        @($relative.Split('/') | Where-Object { !$_ -or $_ -in '.', '..' -or $_.EndsWith(' ') -or $_.EndsWith('.') }).Count -or
        $files.ContainsKey($relative) -or $entry.Value -cnotmatch '^[a-f0-9]{64}$') {
      throw 'Invalid program file inventory.'
    }
    $files[$relative] = $entry.Value
  }
  if (!$files.ContainsKey($exe)) { throw 'Program inventory lacks its executable.' }
  return $files
}

function Get-OrdinaryItem([string]$Path) {
  try { Get-Item -LiteralPath $Path -Force }
  catch [System.Management.Automation.ItemNotFoundException] { return }
}

function Get-ProgramDirectory([string]$Path) {
  $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
  if ($full -notmatch '^[a-z]:\\.+') { throw 'Expected a local program directory.' }
  for ($parent = [IO.DirectoryInfo]::new($full); $parent; $parent = $parent.Parent) {
    $item = Get-OrdinaryItem $parent.FullName
    if ($item -and (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint))) {
      throw 'Program directory contains a file or redirection.'
    }
  }
  return $full
}

function Get-ProgramObject([string]$Root, [string]$Relative) {
  $path = $Root
  $parts = $Relative.Split('/')
  for ($i = 0; $i -lt $parts.Count; $i++) {
    $path = Join-Path $path $parts[$i]
    $item = Get-OrdinaryItem $path
    if (!$item) { return }
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint -or ($i -lt $parts.Count - 1 -and !$item.PSIsContainer)) {
      throw 'Program object is redirected or has an invalid parent.'
    }
  }
  return $item
}

function Assert-ProgramClosed([string]$Root) {
  $filter = "Name='cs2-insight-agent-desktop.exe' OR Name='MaxGameStudioLeague.exe' OR Name='python.exe' OR Name='pythonw.exe'"
  foreach ($process in Get-CimInstance -ClassName Win32_Process -Filter $filter) {
    if ($process.ExecutablePath -and $process.ExecutablePath.StartsWith($Root + '\', [StringComparison]::OrdinalIgnoreCase)) {
      throw 'A program from this directory is still running.'
    }
  }
}

function Remove-VerifiedFiles([string]$Root, [hashtable]$Known, [hashtable]$Keep) {
  $parents = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($relative in $Known.Keys) {
    if ($Keep.ContainsKey($relative)) { continue }
    # Redirections and replaced objects are user-owned. Never follow or remove them.
    try { $file = Get-ProgramObject $Root $relative } catch { continue }
    if (!$file -or $file.PSIsContainer -or (Get-Hash $file.FullName) -ne $Known[$relative]) { continue }
    [IO.File]::Delete($file.FullName)
    for ($parent = $file.Directory; $parent.FullName -ne $Root; $parent = $parent.Parent) {
      $null = $parents.Add($parent.FullName)
    }
  }
  foreach ($parent in ($parents | Sort-Object Length -Descending)) {
    $item = Get-OrdinaryItem $parent
    if ($item -and !($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -and
        !(Get-ChildItem -LiteralPath $parent -Force | Select-Object -First 1)) {
      [IO.Directory]::Delete($parent)
    }
  }
}

function Install-VerifiedFiles([string]$Root, [string]$Source, [hashtable]$Expected, [hashtable]$Prior) {
  foreach ($relative in $Expected.Keys) {
    $incoming = Get-ProgramObject $Source $relative
    if (!$incoming -or $incoming.PSIsContainer -or (Get-Hash $incoming.FullName) -ne $Expected[$relative]) {
      throw 'Staged native program files did not match the installation package.'
    }
    $existing = Get-ProgramObject $Root $relative
    if ($existing -and ($existing.PSIsContainer -or !$Prior[$relative] -or (Get-Hash $existing.FullName) -ne $Prior[$relative])) {
      throw 'A native program file would overwrite an unverified object.'
    }
  }

  $files = [Collections.Generic.List[object]]::new()
  $committed = $false
  $backupRetained = $false
  try {
    foreach ($relative in $Expected.Keys) {
      $target = Join-Path $Root $relative
      $parent = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
      $temp = Join-Path $parent.FullName ('.mgs-native-' + [guid]::NewGuid().ToString('n'))
      $entry = [pscustomobject]@{Target = $target; Next = "$temp.next"; Backup = "$temp.previous"; Applied = $false}
      $files.Add($entry)
      [IO.File]::Copy((Join-Path $Source $relative), $entry.Next)
    }
    foreach ($entry in $files) {
      if ([IO.File]::Exists($entry.Target)) {
        [IO.File]::Replace($entry.Next, $entry.Target, $entry.Backup)
      } else {
        [IO.File]::Move($entry.Next, $entry.Target)
      }
      $entry.Applied = $true
    }
    foreach ($relative in $Expected.Keys) {
      if ((Get-Hash (Join-Path $Root $relative)) -ne $Expected[$relative]) { throw 'Installed program failed verification.' }
    }
    $committed = $true
  } finally {
    foreach ($entry in $files) {
      if (!$committed -and $entry.Applied) {
        if ([IO.File]::Exists($entry.Backup)) {
          [IO.File]::Replace($entry.Backup, $entry.Target, [NullString]::Value)
        } else {
          [IO.File]::Delete($entry.Target)
        }
      }
      [IO.File]::Delete($entry.Next)
      if ($committed) {
        try { [IO.File]::Delete($entry.Backup) }
        catch { $backupRetained = $true }
      }
    }
  }
  return $backupRetained
}

try {
  if ($Phase -eq 'Build') {
    $snapshot = Join-Path ([IO.Path]::GetDirectoryName($Manifest)) 'legacy-catalog.json'
    [IO.File]::Copy($Catalog, $snapshot, $true)
    if ($CatalogSHA256 -cnotmatch '^[A-Fa-f0-9]{64}$' -or (Get-Hash $snapshot) -ine $CatalogSHA256) {
      throw 'Legacy release catalog did not match its independently pinned digest.'
    }
    $prior = Read-Inventory $snapshot @()
    if (!$prior[$uninstaller]) { throw 'Legacy catalog lacks the installed uninstaller digest.' }
    $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($Program)
    if ([IO.Path]::GetFileName($Program) -cne $exe -or
        $info.ProductName -cne 'MaxGameStudio' -or $info.ProductVersion -cne $Version) {
      throw 'Native executable identity or version does not match.'
    }
    $expected = @{$exe = (Get-Hash $Program)}
    foreach ($relative in $assets) { $expected[$relative] = Get-Hash (Join-Path $Stage $relative) }
    if ($Gnu) {
      foreach ($relative in $sidecars) { $expected[$relative] = Get-Hash (Join-Path ([IO.Path]::GetDirectoryName($Program)) $relative) }
    }
    [IO.File]::WriteAllText($Manifest, ($expected | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    exit 0
  }

  $root = Get-ProgramDirectory $Program
  Assert-ProgramClosed $root
  if ($Phase -eq 'Uninstall') {
    $expected = Read-Inventory $Manifest (@($exe) + $assets + $sidecars)
    if ($UninstallerImage) { $expected[$uninstaller] = Get-Hash $UninstallerImage }
    Remove-VerifiedFiles $root $expected @{}
    exit 0
  }
  $prior = Read-Inventory $Catalog @()
  if ($Phase -eq 'Retire') {
    $oldExe = Get-ProgramObject $root $exe
    if (!$oldExe -or $oldExe.PSIsContainer -or (Get-Hash $oldExe.FullName) -ne $prior[$exe]) {
      throw 'This personal installation has no verified legacy catalog.'
    }
    Remove-VerifiedFiles $root $prior @{}
    exit 0
  }
  $stageRoot = Get-ProgramDirectory $Stage
  $expected = Read-Inventory $Manifest (@($exe) + $assets + $sidecars)
  $stagedUninstaller = Get-ProgramObject $stageRoot $uninstaller
  if (!$stagedUninstaller -or $stagedUninstaller.PSIsContainer -or !$stagedUninstaller.Length) {
    throw 'Generated native uninstaller is missing.'
  }
  $expected[$uninstaller] = Get-Hash $stagedUninstaller.FullName
  $existingExe = Get-ProgramObject $root $exe
  if ($existingExe) {
    if ($existingExe.PSIsContainer) { throw 'The existing executable path is occupied.' }
    $digest = Get-Hash $existingExe.FullName
    if ($digest -eq $expected[$exe]) { $prior = $expected }
    elseif ($digest -ne $prior[$exe]) { throw 'This installation has no verified legacy catalog.' }
  } else {
    if ([IO.Directory]::Exists($root) -and (Get-ChildItem -LiteralPath $root -Force | Select-Object -First 1)) {
      throw 'A fresh install requires an empty directory.'
    }
    $prior = @{}
  }
  $retained = Install-VerifiedFiles $root $stageRoot $expected $prior
  try { Remove-VerifiedFiles $root $prior $expected }
  catch { $retained = $true }
  if ($retained) { Write-Output 'MGS_NATIVE_LEGACY_RETAINED' }
  exit 0
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 2
}
