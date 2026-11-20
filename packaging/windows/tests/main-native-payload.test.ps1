$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot '../../../frontend/src-tauri/windows/main-native-payload.ps1'
$shell = Join-Path $env:WINDIR 'System32/WindowsPowerShell/v1.0/powershell.exe'
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('mgs-native-test-' + [guid]::NewGuid().ToString('n'))
$stage = Join-Path $scratch 'staged'
$program = Join-Path $scratch 'existing'
$catalog = Join-Path $scratch 'prior.json'
$manifest = Join-Path $scratch 'candidate.json'
$exe = 'cs2-insight-agent-desktop.exe'
$uninstaller = 'uninstall.exe'
$null = New-Item -ItemType Directory -Path $stage, (Join-Path $program 'python'), (Join-Path $program 'data') -Force
function Put([string]$Root, [string]$Relative, [string]$Content) {
  [IO.File]::WriteAllText((Join-Path $Root $Relative), $Content)
}
function Digest([string]$Root, [string]$Relative) {
  (Get-FileHash -LiteralPath (Join-Path $Root $Relative) -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Run([string]$Phase, [int]$Code) {
  & $shell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $script -Phase $Phase -Program "$program\." -Stage $stage -Manifest $manifest -Catalog $catalog -UninstallerImage (Join-Path $stage $uninstaller)
  if ($LASTEXITCODE -ne $Code) { throw "$Phase returned $LASTEXITCODE, expected $Code." }
}
try {
  Put $program $exe 'old runtime'
  Put $program $uninstaller 'old uninstaller'
  Put $program 'python/python.exe' 'obsolete interpreter'
  Put $program 'python/changed.dll' 'original dependency'
  Put $program 'data/personal.txt' 'local settings'
  Put $stage $exe 'new runtime'
  Put $stage $uninstaller 'new uninstaller'
  Put $stage 'libunwind.dll' 'staged loader'
  $old = @{$exe = (Digest $program $exe); $uninstaller = (Digest $program $uninstaller)
    'python/python.exe' = (Digest $program 'python/python.exe')
    'python/changed.dll' = (Digest $program 'python/changed.dll')}
  $old | ConvertTo-Json | Set-Content -LiteralPath $catalog
  $new = @{$exe = (Digest $stage $exe); 'libunwind.dll' = (Digest $stage 'libunwind.dll')}
  $new | ConvertTo-Json | Set-Content -LiteralPath $manifest
  Put $program 'python/changed.dll' 'modified locally'
  Put $program 'libunwind.dll' 'unknown collision'
  Run Install 2
  if ((Digest $program $exe) -ne $old[$exe] -or [IO.File]::ReadAllText((Join-Path $program 'libunwind.dll')) -ne 'unknown collision') {
    throw 'An ambiguous file collision changed the existing program.'
  }
  [IO.File]::Delete((Join-Path $program 'libunwind.dll'))
  Put $stage 'libunwind.dll' 'tampered loader'
  Run Install 2
  if (!(Test-Path -LiteralPath (Join-Path $program 'python/python.exe')) -or (Digest $program $exe) -ne $old[$exe]) {
    throw 'A staged validation failure changed the existing install.'
  }
  Put $stage 'libunwind.dll' 'staged loader'
  Run Install 0
  Run Install 0
  if ((Digest $program $exe) -ne $new[$exe] -or (Test-Path -LiteralPath (Join-Path $program 'python/python.exe')) -or
      [IO.File]::ReadAllText((Join-Path $program 'python/changed.dll')) -ne 'modified locally' -or
      [IO.File]::ReadAllText((Join-Path $program 'data/personal.txt')) -ne 'local settings') {
    throw 'Native installation did not preserve local files and retire the exact obsolete interpreter.'
  }
  Run Uninstall 0
  if ((Test-Path -LiteralPath (Join-Path $program $exe)) -or (Test-Path -LiteralPath (Join-Path $program $uninstaller)) -or
      !(Test-Path -LiteralPath (Join-Path $program 'data/personal.txt'))) {
    throw 'Native uninstall did not retain local data while removing verified files.'
  }
  $program = Join-Path $scratch 'Never created\Fresh Program'
  Run Install 0
  if ((Digest $program $exe) -ne $new[$exe]) { throw 'Fresh native install was not created.' }
  Write-Host 'MAIN_NATIVE_PAYLOAD_VERIFIED'

  if ($env:MGS_TEST_MAKENSIS) {
    $binary = Join-Path $scratch $exe
    Add-Type -OutputAssembly $binary -OutputType ConsoleApplication -TypeDefinition @'
using System.Reflection;
[assembly: AssemblyProduct("MaxGameStudio")]
[assembly: AssemblyInformationalVersion("3.2.0-beta.9")]
public static class NativeMainFixture { public static void Main() {} }
'@
    $fixture = Join-Path $scratch 'fixture.nsi'
    $win = (Resolve-Path (Split-Path -Parent $script)).Path
    $output = Join-Path $scratch 'fixture-setup.exe'
    $buildScript = @'
Unicode true
Name "Native Main fixture"
OutFile "$%MGS_NATIVE_FIXTURE_OUT%"
InstallDir "$TEMP\never-run-main-native-fixture"
RequestExecutionLevel user
!include "LogicLib.nsh"
!define MGS_WINDOWS_DIR "$%MGS_NATIVE_FIXTURE_WINDOWS%"
!define MAINBINARYSRCPATH "$%MGS_NATIVE_FIXTURE_BINARY%"
!define VERSION "3.2.0-beta.9"
!define UNINSTKEY "Software\MaxGameStudioInstallerTests\Fixture"
Function MGS_FindWindowsPowerShell
FunctionEnd
Function CS2_AbortMigrationInstall
FunctionEnd
Function un.MGS_ReserveInstallationDirectories
FunctionEnd
!include "${MGS_WINDOWS_DIR}\main-native-payload.nsh"
Section
  !insertmacro MGS_InstallNativeProgram
SectionEnd
Section "Uninstall"
  !insertmacro MGS_RemoveNativeProgram
SectionEnd
'@
    [IO.File]::WriteAllText($fixture, $buildScript)
    $testCatalog = if ($env:MGS_TEST_MAIN_LEGACY_CATALOG) { $env:MGS_TEST_MAIN_LEGACY_CATALOG } else { $catalog }
    $buildEnvironment = @{
      MGS_NATIVE_FIXTURE_OUT = $output
      MGS_NATIVE_FIXTURE_WINDOWS = $win
      MGS_NATIVE_FIXTURE_BINARY = $binary
      MGS_MAIN_NATIVE_WORK = $scratch
      MGS_MAIN_NATIVE_CATALOG = $testCatalog
      MGS_MAIN_NATIVE_CATALOG_SHA256 = (Get-FileHash -LiteralPath $testCatalog).Hash
      MGS_MAIN_NATIVE_GNU = '0'
    }
    $before = @{}
    try {
      foreach ($name in $buildEnvironment.Keys) {
        $before[$name] = [Environment]::GetEnvironmentVariable($name)
        [Environment]::SetEnvironmentVariable($name, $buildEnvironment[$name])
      }
      & $env:MGS_TEST_MAKENSIS /V2 $fixture
      if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $output -PathType Leaf)) { throw 'Native NSIS fixture did not compile.' }
      Write-Host 'MAIN_NATIVE_NSIS_COMPILE_VERIFIED'
    } finally {
      foreach ($name in $before.Keys) { [Environment]::SetEnvironmentVariable($name, $before[$name]) }
    }
  }
} finally {
  $item = Get-Item -LiteralPath $scratch -Force
  $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
  if ($item.FullName.StartsWith($temp + 'mgs-native-test-', [StringComparison]::OrdinalIgnoreCase) -and
      !($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    [IO.Directory]::Delete($item.FullName, $true)
  }
}
