param(
  [Parameter(Mandatory = $true)][string]$Makensis,
  [Parameter(Mandatory = $true)][string]$SourceDir,
  [Parameter(Mandatory = $true)][string]$PluginDirectory
)

$ErrorActionPreference = 'Stop'
$compiler = (Resolve-Path -LiteralPath $Makensis).Path
$headers = (Resolve-Path (Join-Path $SourceDir 'node_modules/app-builder-lib/templates/nsis/include')).Path
$plugins = (Resolve-Path -LiteralPath $PluginDirectory).Path
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('mgs-nsis-test-' + [Guid]::NewGuid().ToString('n'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$include = Join-Path $scratch 'installer.nsh'
foreach ($name in 'installer.nsh', 'destination-guard.ps1', 'retire-user-install.ps1', 'shortcut-target.ps1') {
  Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $scratch
}
$inventory = Join-Path $scratch 'maxgamestudio-runtime-hashes.json'
[IO.File]::WriteAllText($inventory, '{}')
$script = Join-Path $scratch 'include-test.nsi'
$out = Join-Path $scratch 'include-test.exe'
$originalAppData = $env:APPDATA
$originalUserProfile = $env:USERPROFILE
$env:APPDATA = Join-Path $scratch 'fixture-appdata'
$main = Join-Path $env:APPDATA 'MaxGameStudio'
New-Item -ItemType Directory -Path $main -Force | Out-Null
[IO.File]::WriteAllText($script, @'
Unicode true
RequestExecutionLevel user
Name "League installer include test"
OutFile "${LEAGUE_OUTPUT}"
InstallDir "$TEMP\MaxGameStudioLeagueIncludeTest"
!addincludedir "${LEAGUE_HEADERS}"
!addplugindir "${LEAGUE_PLUGINS}"
!include "LogicLib.nsh"
!include "UAC.nsh"
Var installMode
Var hasPerUserInstallation
Var hasPerMachineInstallation
!macro setInstallModePerAllUsers
  StrCpy $installMode "all"
!macroend
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
Function setInstallSectionSpaceRequired
FunctionEnd
Function .onInit
  Call setInstallSectionSpaceRequired

  SetOutPath $INSTDIR
  StrCpy $installMode "CurrentUser"
  ReadEnvStr $0 "MGS_LEAGUE_FIXTURE_PATH_HANDOFF"
  ${If} $0 == "1"
    StrCpy $LeagueProgramPath $INSTDIR
    StrCpy $INSTDIR ""
    Call LeagueSetProgramDirectory
    ${If} $R7 != 0
    ${OrIf} $INSTDIR != $LeagueProgramPath
      SetErrorLevel 2
      Quit
    ${EndIf}
    StrCpy $LeagueProgramPath $INSTDIR
    Call LeagueValidateDestination
    SetErrorLevel $R7
    Quit
  ${EndIf}
  ReadEnvStr $0 "MGS_LEAGUE_FIXTURE_MACHINE"
  ${If} $0 == "check"
    StrCpy $LeagueProgramPath $INSTDIR
    Call LeagueCheckMachineDestination
    ${If} $R7 == "0"
      SetErrorLevel 0
    ${Else}
      SetErrorLevel 2
    ${EndIf}
    Quit
  ${EndIf}
  ${If} $0 == "1"
    StrCpy $installMode "all"
  ${EndIf}
  !insertmacro customInit
FunctionEnd
!endif
Section
  !ifndef BUILD_UNINSTALLER
    SetOutPath $INSTDIR
    ReadEnvStr $0 "MGS_LEAGUE_FIXTURE_CHECK_PAYLOAD"
    ${If} $0 == "1"
      SetOutPath "$INSTDIR\resources"
      FileOpen $1 "$OUTDIR\app.asar" w
      FileWrite $1 "fixture archive"
      FileClose $1
      SetOutPath $INSTDIR
      FileOpen $1 "$OUTDIR\Uninstall MaxGameStudioLeagueToolbox.exe" w
      FileWrite $1 "fixture uninstaller"
      FileClose $1
      !insertmacro leagueBeforeRegistration
    ${EndIf}
    FileOpen $0 "$INSTDIR\fixture-payload.txt" w
    FileClose $0
  !endif
SectionEnd
'@)
try {
  $launcher = Join-Path $PSScriptRoot 'run-league-nsis.cjs'
  node -e "const fs = require('node:fs'); const { patchInstallerTemplate } = require(process.argv[1]); const script = process.argv[2]; fs.writeFileSync(script, patchInstallerTemplate(fs.readFileSync(script, 'utf8')));" $launcher $script
  if ($LASTEXITCODE -ne 0) { throw 'Could not patch the NSIS test startup template.' }
  foreach ($mode in @('uninstaller', 'installer')) {
    $flags = @('/V2', "/DLEAGUE_CUSTOM_NSIS=$include", "/DLEAGUE_OUTPUT=$out", "/DLEAGUE_HEADERS=$headers", "/DLEAGUE_PLUGINS=$plugins")
    if ($mode -eq 'uninstaller') { $flags += '/DBUILD_UNINSTALLER' }
    & $compiler @flags $script
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $out -PathType Leaf)) {
      throw "League NSIS include failed in $mode mode."
    }
    Write-Host "League NSIS include compiled in $mode mode."
    if ($mode -eq 'uninstaller') { Remove-Item -LiteralPath $out }
  }
  $machine = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)) 'MaxGameStudioLeagueToolbox'
  $machineExisted = Test-Path -LiteralPath $machine
  $env:MGS_LEAGUE_FIXTURE_MACHINE = 'check'
  $checked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$machine" -WindowStyle Hidden -Wait -PassThru
  if ($checked.ExitCode -ne 0 -or (Test-Path -LiteralPath $machine) -ne $machineExisted) {
    throw "The read-only NSIS machine bridge did not approve its dedicated protected destination (exit $($checked.ExitCode))."
  }
  $env:MGS_LEAGUE_FIXTURE_MACHINE = $null
  Write-Host 'League NSIS machine bridge approved the protected default without creating a directory.'
  $uncreated = Join-Path $scratch 'Not Created\Nested Toolbox'
  $env:MGS_LEAGUE_FIXTURE_PATH_HANDOFF = '1'
  $checked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$uncreated" -WindowStyle Hidden -Wait -PassThru
  $env:MGS_LEAGUE_FIXTURE_PATH_HANDOFF = $null
  if ($checked.ExitCode -ne 0 -or (Test-Path -LiteralPath (Split-Path -Parent $uncreated))) {
    throw 'A new program directory must survive the wizard handoff without being created before validation.'
  }
  Write-Host 'League NSIS preserved a not-yet-created program directory through the final check.'
  $occupied = Join-Path $scratch 'wrong-folder'
  $empty = Join-Path $scratch 'empty-program'
  $overlap = Join-Path $main 'Overlapping Toolbox'
  New-Item -ItemType Directory -Path $occupied | Out-Null
  $sentinel = Join-Path $occupied 'main-marker.txt'
  Set-Content -LiteralPath $sentinel -Value 'keep'
  $blocked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$occupied" -WindowStyle Hidden -Wait -PassThru
  if ($blocked.ExitCode -eq 0 -or (Test-Path (Join-Path $occupied 'fixture-payload.txt')) -or
      ((Get-Content -LiteralPath $sentinel -Raw).Trim() -ne 'keep')) {
    throw 'NSIS silent destination did not retain the protected marker.'
  }
  $blocked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$overlap" -WindowStyle Hidden -Wait -PassThru
  if ($blocked.ExitCode -ne 2 -or (Test-Path -LiteralPath $overlap)) {
    throw "NSIS created or accepted a forbidden Main child (exit $($blocked.ExitCode), exists $(Test-Path -LiteralPath $overlap))."
  }
  $accepted = Start-Process -FilePath $out -ArgumentList '/S', "/D=$empty" -WindowStyle Hidden -Wait -PassThru
  if ($accepted.ExitCode -ne 0 -or !(Test-Path -LiteralPath (Join-Path $empty 'fixture-payload.txt'))) {
    throw "NSIS did not accept an independent empty folder (exit ${accepted.ExitCode})."
  }
  $missing = Join-Path $scratch 'missing-copied-program'
  $env:MGS_LEAGUE_FIXTURE_CHECK_PAYLOAD = '1'
  $blocked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$missing" -WindowStyle Hidden -Wait -PassThru
  $env:MGS_LEAGUE_FIXTURE_CHECK_PAYLOAD = $null
  if ($blocked.ExitCode -ne 2 -or (Test-Path -LiteralPath (Join-Path $missing 'fixture-payload.txt'))) {
    throw 'NSIS continued past the registration gate without the copied program files.'
  }
  $shared = Join-Path $scratch 'unprotected-machine-program'
  $env:MGS_LEAGUE_FIXTURE_MACHINE = '1'
  $env:USERPROFILE = $null
  $blocked = Start-Process -FilePath $out -ArgumentList '/S', "/D=$shared" -WindowStyle Hidden -Wait -PassThru
  if ($blocked.ExitCode -ne 2 -or (Test-Path -LiteralPath $shared)) {
    throw "NSIS accepted an unprotected machine installation directory (exit $($blocked.ExitCode))."
  }
  Write-Host 'League NSIS accepted an independent directory, blocked protected paths, and refused registration when the copied EXE was missing.'
} finally {
  $env:MGS_LEAGUE_FIXTURE_MACHINE = $null
  $env:MGS_LEAGUE_FIXTURE_PATH_HANDOFF = $null
  $env:MGS_LEAGUE_FIXTURE_CHECK_PAYLOAD = $null
  $env:APPDATA = $originalAppData
  $env:USERPROFILE = $originalUserProfile
  foreach ($child in 'wrong-folder\main-marker.txt', 'wrong-folder\fixture-payload.txt', 'wrong-folder', 'empty-program\fixture-payload.txt', 'empty-program', 'unprotected-machine-program\fixture-payload.txt', 'unprotected-machine-program', 'fixture-appdata\MaxGameStudio\Overlapping Toolbox\fixture-payload.txt', 'fixture-appdata\MaxGameStudio\Overlapping Toolbox', 'fixture-appdata\MaxGameStudio', 'fixture-appdata') {
    $file = Join-Path $scratch $child
    if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
  }
  foreach ($child in 'missing-copied-program\fixture-payload.txt', 'missing-copied-program\resources\app.asar', 'missing-copied-program\resources',
      'missing-copied-program\Uninstall MaxGameStudioLeagueToolbox.exe', 'missing-copied-program') {
    $file = Join-Path $scratch $child
    if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
  }
  if (Test-Path -LiteralPath $out -PathType Leaf) { Remove-Item -LiteralPath $out }
  Remove-Item -LiteralPath $script
  foreach ($name in 'installer.nsh', 'destination-guard.ps1', 'retire-user-install.ps1', 'shortcut-target.ps1', 'maxgamestudio-runtime-hashes.json') {
    Remove-Item -LiteralPath (Join-Path $scratch $name)
  }
  Remove-Item -LiteralPath $scratch
}
