$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')

$exitCode = 201
$fileHandles = [Collections.Generic.List[IO.FileStream]]::new()
$sessionRoot = $null
$sessionCreated = $false

try {
$configJson = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__CONFIG_BASE64__'))
$config = $configJson | ConvertFrom-Json
if ($config.manifestSha256 -notmatch '^[0-9a-f]{64}$') { throw 'Invalid runtime manifest digest.' }
if ($config.sessionName -notmatch '^session-[0-9a-f]{32}$') { throw 'Invalid protected session name.' }

function Assert-SafeRelativePath([string]$value) {
  $normalized = $value.Replace('\', '/')
  if (-not $normalized -or $normalized.StartsWith('/') -or $normalized.Contains(':')) {
    throw "Unsafe runtime manifest path: $value"
  }
  foreach ($part in $normalized.Split('/')) {
    if (-not $part -or $part -eq '.' -or $part -eq '..') { throw "Unsafe runtime manifest path: $value" }
  }
  return $normalized
}

function Get-Sha256HexFromBytes([byte[]]$bytes) {
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant() }
  finally { $sha.Dispose() }
}

function Get-Sha256HexFromStream([IO.Stream]$stream) {
  $sha = [Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
  finally { $sha.Dispose() }
}

$adminSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$systemSid = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$fullControl = [Security.AccessControl.FileSystemRights]::FullControl
$inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
$noPropagation = [Security.AccessControl.PropagationFlags]::None
$allow = [Security.AccessControl.AccessControlType]::Allow

function New-ProtectedDirectory([string]$path) {
  $security = [Security.AccessControl.DirectorySecurity]::new()
  $security.SetAccessRuleProtection($true, $false)
  $security.SetOwner($adminSid)
  [void]$security.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($adminSid, $fullControl, $inheritance, $noPropagation, $allow))
  [void]$security.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($systemSid, $fullControl, $inheritance, $noPropagation, $allow))
  [void][IO.Directory]::CreateDirectory($path, $security)
}

function Assert-ProtectedDirectoryAcl([string]$path) {
  if (([IO.File]::GetAttributes($path) -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Protected runtime directories cannot be links.' }
  $acl = [IO.Directory]::GetAccessControl($path)
  if (-not $acl.AreAccessRulesProtected) { throw "Protected runtime ACL inherits permissions: $path" }
  $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
  if ($owner -ne $adminSid.Value -and $owner -ne $systemSid.Value) { throw "Protected runtime owner is not trusted: $path" }
  $rules = @($acl.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]))
  if ($rules.Count -ne 2) { throw "Protected runtime ACL has unexpected entries: $path" }
  foreach ($rule in $rules) {
    if ($rule.AccessControlType -ne $allow -or ($rule.IdentityReference.Value -ne $adminSid.Value -and $rule.IdentityReference.Value -ne $systemSid.Value)) {
      throw "Protected runtime ACL grants an unexpected principal: $path"
    }
    if (($rule.FileSystemRights -band $fullControl) -ne $fullControl) { throw "Protected runtime ACL is incomplete: $path" }
  }
}

function Ensure-ProtectedDirectory([string]$path) {
  if ([IO.File]::Exists($path)) { throw "Protected runtime directory is occupied by a file: $path" }
  if (-not [IO.Directory]::Exists($path)) { New-ProtectedDirectory $path }
  Assert-ProtectedDirectoryAcl $path
}

function Get-PlainTreeEntries([string]$root) {
  $entries = [Collections.Generic.List[string]]::new()
  $pending = [Collections.Generic.Queue[string]]::new()
  $pending.Enqueue($root)
  while ($pending.Count -gt 0) {
    $directory = $pending.Dequeue()
    foreach ($entry in [IO.Directory]::EnumerateFileSystemEntries($directory)) {
      $attributes = [IO.File]::GetAttributes($entry)
      if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Protected runtime contains a reparse point: $entry" }
      $entries.Add($entry)
      if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) { $pending.Enqueue($entry) }
    }
  }
  return $entries
}

function Remove-ProtectedSession([string]$root) {
  Assert-ProtectedDirectoryAcl $root
  [void]@(Get-PlainTreeEntries $root)
  [IO.Directory]::Delete($root, $true)
}

  $exitCode = 211
  $sourceRoot = [IO.Path]::GetFullPath([string]$config.sourceRoot).TrimEnd('\')
  if (-not [IO.Path]::IsPathRooted($sourceRoot) -or $sourceRoot.StartsWith('\\')) { throw 'The runtime source must be on a local drive.' }
  $storageRoot = [IO.Path]::GetFullPath([string]$config.storageRoot).TrimEnd('\')
  if ($storageRoot -notmatch '^[A-Za-z]:\\' -or $storageRoot.StartsWith('\\')) { throw 'Storage must be on a local drive.' }
  $driveRoot = [IO.Path]::GetPathRoot($storageRoot)
  $adminRoot = Join-Path $driveRoot 'MaxGameStudioAdminRuntime'
  $sessionsRoot = Join-Path $adminRoot 'Sessions'
  $profilesRoot = Join-Path $adminRoot 'Profiles'
  $profileDigest = Get-Sha256HexFromBytes ([Text.Encoding]::UTF8.GetBytes($sourceRoot.ToLowerInvariant()))
  $profileRoot = Join-Path $profilesRoot ('profile-' + $profileDigest.Substring(0, 32))
  foreach ($parent in @($adminRoot, $sessionsRoot, $profilesRoot)) {
    Ensure-ProtectedDirectory $parent
    Assert-ProtectedDirectoryAcl $parent
  }

  # A previous version kept the protected profile on the installation volume.
  # Copy it only while all workspaces are closed, retain the original, and do
  # not expose privileged profile contents to the ordinary user's data tree.
  $legacyProfiles = Join-Path ([IO.Path]::GetPathRoot($sourceRoot)) 'MaxGameStudioAdminRuntime\Profiles'
  $legacyProfile = Join-Path $legacyProfiles ('profile-' + $profileDigest.Substring(0, 32))
  if (-not [IO.Directory]::Exists($profileRoot) -and [IO.Directory]::Exists($legacyProfile)) {
    if (@([Diagnostics.Process]::GetProcessesByName('MaxGameStudioLeague')).Count -ne 0) { throw 'Close other workspaces before profile migration.' }
    Assert-ProtectedDirectoryAcl $legacyProfile
    $profileEntries = @(Get-PlainTreeEntries $legacyProfile)
    $profileStage = Join-Path $profilesRoot ([string]$config.sessionName + '-profile')
    if ([IO.Directory]::Exists($profileStage)) { throw 'Profile staging path already exists.' }
    New-ProtectedDirectory $profileStage
    try {
      foreach ($entry in $profileEntries) {
        $relative = $entry.Substring($legacyProfile.Length + 1)
        $target = Join-Path $profileStage $relative
        if ([IO.Directory]::Exists($entry)) { [void][IO.Directory]::CreateDirectory($target); continue }
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
        $original = [IO.File]::Open($entry, 'Open', 'Read', 'Read')
        try {
          $copy = [IO.File]::Open($target, 'CreateNew', 'ReadWrite', 'None')
          try {
            $original.CopyTo($copy)
            $copy.Flush($true)
            $original.Position = 0; $copy.Position = 0
            if ((Get-Sha256HexFromStream $original) -ne (Get-Sha256HexFromStream $copy)) { throw 'Profile verification failed.' }
          } finally { $copy.Dispose() }
        } finally { $original.Dispose() }
      }
      [IO.Directory]::Move($profileStage, $profileRoot)
    } finally {
      if ([IO.Directory]::Exists($profileStage)) { Remove-ProtectedSession $profileStage }
    }
  }
  Ensure-ProtectedDirectory $profileRoot

  $exitCode = 212
  # Every launch uses a cryptographically random session name. Do not delete
  # other sessions here: another elevated workspace may still own one, and an
  # elevated cleanup race is riskier than leaving a recoverable orphan behind.

  $exitCode = 213
  $sessionRoot = Join-Path $sessionsRoot ([string]$config.sessionName)
  if ([IO.Directory]::Exists($sessionRoot) -or [IO.File]::Exists($sessionRoot)) { throw 'The protected runtime session path already exists.' }
  New-ProtectedDirectory $sessionRoot
  $sessionCreated = $true
  Assert-ProtectedDirectoryAcl $sessionRoot

  $exitCode = 214
  $manifestPath = Join-Path $sourceRoot 'maxgamestudio-runtime-hashes.json'
  $manifestBytes = [IO.File]::ReadAllBytes($manifestPath)
  if ((Get-Sha256HexFromBytes $manifestBytes) -ne [string]$config.manifestSha256) { throw 'The runtime manifest does not match the signed host.' }
  $manifest = ([Text.Encoding]::UTF8.GetString($manifestBytes) | ConvertFrom-Json)
  $expected = @{}
  foreach ($property in $manifest.PSObject.Properties) {
    $relative = Assert-SafeRelativePath ([string]$property.Name)
    $digest = ([string]$property.Value).Trim().ToLowerInvariant()
    if ($digest -notmatch '^[0-9a-f]{64}$' -or $expected.ContainsKey($relative)) { throw "Invalid runtime manifest entry: $relative" }
    $expected[$relative] = $digest
  }
  foreach ($required in @('MaxGameStudioLeague.exe', 'resources/app.asar')) {
    if (-not $expected.ContainsKey($required)) { throw "The runtime manifest is missing $required" }
  }

  $exitCode = 215
  foreach ($relative in @($expected.Keys | Sort-Object)) {
    $nativeRelative = $relative.Replace('/', [IO.Path]::DirectorySeparatorChar)
    $source = [IO.Path]::GetFullPath((Join-Path $sourceRoot $nativeRelative))
    $sourcePrefix = $sourceRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $source.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "Runtime source escaped its root: $relative" }
    $sourceAttributes = [IO.File]::GetAttributes($source)
    if (($sourceAttributes -band [IO.FileAttributes]::Directory) -ne 0 -or ($sourceAttributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw "Runtime source is not a plain file: $relative"
    }
    $destination = Join-Path $sessionRoot $nativeRelative
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
    [IO.File]::Copy($source, $destination, $false)
  }

  $exitCode = 216
  $actualFiles = @([IO.Directory]::EnumerateFiles($sessionRoot, '*', [IO.SearchOption]::AllDirectories))
  if ($actualFiles.Count -ne $expected.Count) { throw 'The protected runtime file count does not match its manifest.' }
  foreach ($path in $actualFiles) {
    $relative = $path.Substring($sessionRoot.Length + 1).Replace('\', '/')
    if (-not $expected.ContainsKey($relative)) { throw "The protected runtime contains an unlisted file: $relative" }
    $attributes = [IO.File]::GetAttributes($path)
    if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "The protected runtime contains a linked file: $relative" }
    $stream = [IO.FileStream]::new($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read, 65536, [IO.FileOptions]::SequentialScan)
    $fileHandles.Add($stream)
    if ((Get-Sha256HexFromStream $stream) -ne $expected[$relative]) { throw "Protected runtime integrity check failed: $relative" }
  }

  $exitCode = 217
  $windowsDir = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
  $startInfo = [Diagnostics.ProcessStartInfo]::new()
  $startInfo.FileName = Join-Path $sessionRoot 'MaxGameStudioLeague.exe'
  $protectedUserDataArgument = '--user-data-dir=' + $profileRoot
  $startInfo.Arguments = ($protectedUserDataArgument + ' ' + [string]$config.commandLine).Trim()
  $startInfo.WorkingDirectory = $sessionRoot
  $startInfo.UseShellExecute = $false
  $protectedTemp = Join-Path $profileRoot 'Temp'
  [void][IO.Directory]::CreateDirectory($protectedTemp)
  foreach ($name in @($startInfo.EnvironmentVariables.Keys)) {
    if ([string]$name -match '^(NODE|ELECTRON|CHROME|NPM|YARN|PNPM|MAXGAMESTUDIO)_') {
      [void]$startInfo.EnvironmentVariables.Remove([string]$name)
    }
  }
  $startInfo.EnvironmentVariables['PATH'] = (Join-Path $windowsDir 'System32') + ';' + $windowsDir
  $startInfo.EnvironmentVariables['SystemRoot'] = $windowsDir
  $startInfo.EnvironmentVariables['windir'] = $windowsDir
  $startInfo.EnvironmentVariables['ComSpec'] = Join-Path $windowsDir 'System32\cmd.exe'
  $startInfo.EnvironmentVariables['TEMP'] = $protectedTemp
  $startInfo.EnvironmentVariables['TMP'] = $protectedTemp
  $startInfo.EnvironmentVariables['MAXGAMESTUDIO_EMBEDDED'] = '1'
  $startInfo.EnvironmentVariables['MAXGAMESTUDIO_HOST_PID'] = [string]$config.hostPid
  $child = [Diagnostics.Process]::Start($startInfo)
  if ($null -eq $child) { throw 'The protected League workspace did not start.' }
  $child.WaitForExit()
  $childExitCode = $child.ExitCode
  $child.Dispose()
  $exitCode = if ($childExitCode -eq 0) { 0 } else { 202 }
} catch {
  # The numeric stage is returned to the non-elevated host. Error text and
  # local paths never cross the integrity boundary or get written to disk.
} finally {
  foreach ($stream in $fileHandles) { $stream.Dispose() }
  if ($sessionCreated -and $sessionRoot -and [IO.Directory]::Exists($sessionRoot)) {
    try { Remove-ProtectedSession $sessionRoot } catch {}
  }
}
exit $exitCode
