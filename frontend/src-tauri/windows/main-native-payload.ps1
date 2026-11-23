param(
  [Parameter(Mandatory)][ValidateSet('Build', 'Install', 'Uninstall', 'Retire')][string]$Phase,
  [Parameter(Mandatory)][string]$Program,
  [string]$Stage,
  [string]$Manifest,
  [string]$Catalog,
  [string]$CatalogSHA256,
  [string]$NativePriorCatalog,
  [string]$Version,
  [string]$UninstallerImage,
  [string]$ParserExe,
  [string]$ParserNotice,
  [switch]$Gnu
)

$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$ErrorActionPreference = 'Stop'
$exe = 'cs2-insight-agent-desktop.exe'
$uninstaller = 'uninstall.exe'
$assets = @('data/basic.ini', 'data/cs2-insight.config.example.json',
  'data/lite_cut_effect_contract.json', 'data/lite_cut_visual_acceptance.json',
  'backend/assets/fonts/NotoSansSC-Bold.ttf', 'backend/assets/fonts/NotoSansSC-Medium.ttf',
  'backend/assets/fonts/Rajdhani-Bold.ttf', 'backend/assets/fonts/Rajdhani-SemiBold.ttf',
  'backend/assets/fonts/OFL.txt',
  'pov/gameinfo.gi.default', 'pov/gameinfo.gi.pov', 'pov/pov_default.vpk')
$sidecars = @('WebView2Loader.dll', 'libunwind.dll')
$worker = 'maxgamestudio-parser-worker.exe'
$notice = 'maxgamestudio-parser-worker.LICENSE.txt'
$nativeFiles = @($exe, $worker, $notice) + $assets + $sidecars

function Get-Hash([string]$Path) {
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function ConvertTo-Inventory($Contents, [string[]]$Allowed) {
  if ($Contents -isnot [pscustomobject]) { throw 'Expected a program file inventory.' }
  $files = @{}
  foreach ($entry in $Contents.PSObject.Properties) {
    $relative = $entry.Name
    $known = $relative -cin $Allowed -or (!$Allowed -and
      ($relative -cin $nativeFiles + @($uninstaller, '.gitkeep') -or
        $relative -cmatch '^(python|backend|league-runtime)/'))
    if (!$known -or $relative -match '[\\:*?"<>|]' -or
        @($relative.Split('/') | Where-Object { !$_ -or $_ -in '.', '..' -or $_.EndsWith(' ') -or $_.EndsWith('.') }).Count -or
        $files.ContainsKey($relative) -or $entry.Value -cnotmatch '^[a-f0-9]{64}$') {
      throw 'Invalid program file inventory.'
    }
    $files[$relative] = $entry.Value
  }
  if (!$files.ContainsKey($exe)) { throw 'Program inventory lacks its executable.' }
  if ($files.ContainsKey($worker) -ne $files.ContainsKey($notice)) {
    throw 'Program inventory must contain the parser worker and MIT notice together.'
  }
  return $files
}

function Read-Inventory([string]$Path, [string[]]$Allowed) {
  ConvertTo-Inventory (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json) $Allowed
}

function Read-NativePriorCatalog([string]$Path) {
  $catalogs = @{}
  if ($Path) {
    $contents = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ($contents -isnot [pscustomobject]) { throw 'Expected a native predecessor catalog.' }
    foreach ($entry in $contents.PSObject.Properties) {
      $inventory = ConvertTo-Inventory $entry.Value ($nativeFiles + $uninstaller)
      if ($entry.Name -cnotmatch '^[a-f0-9]{64}$' -or $entry.Name -cne $inventory[$exe] -or !$inventory[$uninstaller]) {
        throw 'A native predecessor entry must identify its own executable and uninstaller digests.'
      }
      $catalogs[$entry.Name] = $inventory
    }
  }
  return $catalogs
}

function Get-VerifiedPredecessor([string]$Digest, [hashtable]$Legacy, [hashtable]$Native) {
  if ($Digest -eq $Legacy[$exe]) { return $Legacy }
  if ($Native.ContainsKey($Digest)) { return $Native[$Digest] }
  throw 'No verified predecessor inventory for the existing installation.'
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
  $filter = "Name='$exe' OR Name='MaxGameStudioLeague.exe' OR Name='python.exe' OR Name='pythonw.exe' OR Name='$worker'"
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
    $nativeSnapshot = Join-Path ([IO.Path]::GetDirectoryName($Manifest)) 'native-prior-catalog.json'
    $nativeInput = $env:MGS_MAIN_NATIVE_PRIOR_CATALOG
    $nativeDigest = $env:MGS_MAIN_NATIVE_PRIOR_CATALOG_SHA256
    if ($nativeInput -or $nativeDigest) {
      if (!$nativeInput -or $nativeDigest -cnotmatch '^[A-Fa-f0-9]{64}$') {
        throw 'Provide the native predecessor catalog and its independently pinned SHA-256.'
      }
      [IO.File]::Copy($nativeInput, $nativeSnapshot, $true)
      if ((Get-Hash $nativeSnapshot) -ine $nativeDigest) { throw 'Native predecessor catalog SHA-256 mismatch.' }
      if ((Read-NativePriorCatalog $nativeSnapshot).ContainsKey($prior[$exe])) {
        throw 'The native predecessor catalog duplicates the legacy executable digest.'
      }
    } else {
      [IO.File]::WriteAllText($nativeSnapshot, '{}')
    }
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
    if ([bool]$ParserExe -ne [bool]$ParserNotice) { throw 'Provide both the parser worker and its MIT notice.' }
    if ($ParserExe) {
      $workerInput = Get-OrdinaryItem $ParserExe
      if (!$workerInput -or $workerInput.PSIsContainer -or !$workerInput.Length -or
          ($workerInput.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Expected an ordinary, nonempty parser worker file.'
      }
      $noticeHash = Get-Hash $ParserNotice
      if ($noticeHash -ne (Get-Hash (Join-Path $Stage 'packaging/demoparser-lean/LICENSE.upstream.txt'))) {
        throw 'Parser MIT notice does not match the pinned upstream notice.'
      }
      $expected[$worker] = Get-Hash $workerInput.FullName
      $expected[$notice] = $noticeHash
    }
    [IO.File]::WriteAllText($Manifest, ($expected | ConvertTo-Json -Compress), [Text.UTF8Encoding]::new($false))
    exit 0
  }

  $root = Get-ProgramDirectory $Program
  Assert-ProgramClosed $root
  if ($Phase -eq 'Uninstall') {
    $expected = Read-Inventory $Manifest $nativeFiles
    if ($UninstallerImage) { $expected[$uninstaller] = Get-Hash $UninstallerImage }
    Remove-VerifiedFiles $root $expected @{}
    exit 0
  }
  $legacyPrior = Read-Inventory $Catalog @()
  $nativePrior = Read-NativePriorCatalog $NativePriorCatalog
  if ($Phase -eq 'Retire') {
    $oldExe = Get-ProgramObject $root $exe
    if (!$oldExe -or $oldExe.PSIsContainer) { throw 'The existing executable is missing or occupied.' }
    $prior = Get-VerifiedPredecessor (Get-Hash $oldExe.FullName) $legacyPrior $nativePrior
    Remove-VerifiedFiles $root $prior @{}
    exit 0
  }
  $stageRoot = Get-ProgramDirectory $Stage
  $expected = Read-Inventory $Manifest $nativeFiles
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
    else { $prior = Get-VerifiedPredecessor $digest $legacyPrior $nativePrior }
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
