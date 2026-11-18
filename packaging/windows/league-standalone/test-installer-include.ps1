param([Parameter(Mandatory = $true)][string]$Makensis)

$ErrorActionPreference = 'Stop'
$compiler = (Resolve-Path -LiteralPath $Makensis).Path
$include = Join-Path $PSScriptRoot 'installer.nsh'
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('mgs-nsis-test-' + [Guid]::NewGuid().ToString('n'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$script = Join-Path $scratch 'include-test.nsi'
$out = Join-Path $scratch 'include-test.exe'
[IO.File]::WriteAllText($script, @'
Unicode true
RequestExecutionLevel user
Name "League installer include test"
OutFile "${LEAGUE_OUTPUT}"
InstallDir "$TEMP\MaxGameStudioLeagueIncludeTest"
!include "${LEAGUE_CUSTOM_NSIS}"
!include "MUI2.nsh"
!ifndef BUILD_UNINSTALLER
  !insertmacro customPageAfterChangeDir
!endif
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!ifmacrodef customHeader
  !insertmacro customHeader
!endif
!ifdef BUILD_UNINSTALLER
SilentInstall silent
!else
Function .onInit
  !insertmacro customInit
FunctionEnd
!endif
Section
  !ifndef BUILD_UNINSTALLER
    SetOutPath $INSTDIR
    FileOpen $0 "$INSTDIR\fixture-payload.txt" w
    FileClose $0
  !endif
SectionEnd
'@)
try {
  foreach ($mode in @('uninstaller', 'installer')) {
    $flags = @('/V2', "/DLEAGUE_CUSTOM_NSIS=$include", "/DLEAGUE_OUTPUT=$out")
    if ($mode -eq 'uninstaller') { $flags += '/DBUILD_UNINSTALLER' }
    & $compiler @flags $script
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $out -PathType Leaf)) {
      throw "League NSIS include failed in $mode mode."
    }
    Write-Host "League NSIS include compiled in $mode mode."
    if ($mode -eq 'uninstaller') { Remove-Item -LiteralPath $out }
  }
  $occupied = Join-Path $scratch 'wrong-folder'
  $empty = Join-Path $scratch 'empty-program'
  New-Item -ItemType Directory -Path $occupied, $empty | Out-Null
  $sentinel = Join-Path $occupied 'main-marker.txt'
  Set-Content -LiteralPath $sentinel -Value 'keep'
  $blocked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$occupied" -WindowStyle Hidden -Wait -PassThru
  if ($blocked.ExitCode -eq 0 -or (Test-Path (Join-Path $occupied 'fixture-payload.txt')) -or
      ((Get-Content -LiteralPath $sentinel -Raw).Trim() -ne 'keep')) {
    throw 'NSIS silent destination did not retain the protected marker.'
  }
  $accepted = Start-Process -FilePath $out -ArgumentList '/S', "/D=$empty" -WindowStyle Hidden -Wait -PassThru
  if ($accepted.ExitCode -ne 0 -or !(Test-Path -LiteralPath (Join-Path $empty 'fixture-payload.txt'))) {
    throw "NSIS did not accept an independent empty folder (exit ${accepted.ExitCode})."
  }
  Write-Host 'League NSIS blocked a nonempty destination and accepted an independent empty destination.'
} finally {
  foreach ($child in 'wrong-folder\main-marker.txt', 'wrong-folder\fixture-payload.txt', 'wrong-folder', 'empty-program\fixture-payload.txt', 'empty-program') {
    $file = Join-Path $scratch $child
    if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
  }
  if (Test-Path -LiteralPath $out -PathType Leaf) { Remove-Item -LiteralPath $out }
  Remove-Item -LiteralPath $script
  Remove-Item -LiteralPath $scratch
}
