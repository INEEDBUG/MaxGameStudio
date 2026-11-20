!define MGS_NATIVE_PAYLOAD_SCRIPT "${MGS_WINDOWS_DIR}\main-native-payload.ps1"
!define MGS_NATIVE_ASSETS "${MGS_WINDOWS_DIR}\..\..\.."
!define MGS_NATIVE_CATALOG "$%MGS_MAIN_NATIVE_WORK%\legacy-catalog.json"
!define MGS_NATIVE_MANIFEST "$%MGS_MAIN_NATIVE_WORK%\candidate.json"
!if "$%MGS_MAIN_NATIVE_GNU%" == "1"
  !define MGS_NATIVE_GNU_FLAG "-Gnu"
!else
  !define MGS_NATIVE_GNU_FLAG ""
!endif
!if "$%MGS_MAIN_NATIVE_PARSER%" == "1"
  !define MGS_NATIVE_PARSER_FLAGS '-ParserExe "$%MGS_MAIN_NATIVE_PARSER_EXE%" -ParserNotice "$%MGS_MAIN_NATIVE_PARSER_NOTICE%"'
!else
  !define MGS_NATIVE_PARSER_FLAGS ""
!endif

; Hash the exact Tauri build inputs at compilation, after the bundler has
; prepared the candidate binaries. Never trust an installed inventory.
!execute '"$%WINDIR%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "${MGS_NATIVE_PAYLOAD_SCRIPT}" -Phase Build -Program "${MAINBINARYSRCPATH}" -Stage "${MGS_NATIVE_ASSETS}" -Manifest "${MGS_NATIVE_MANIFEST}" -Catalog "$%MGS_MAIN_NATIVE_CATALOG%" -CatalogSHA256 "$%MGS_MAIN_NATIVE_CATALOG_SHA256%" -Version "${VERSION}" ${MGS_NATIVE_GNU_FLAG} ${MGS_NATIVE_PARSER_FLAGS}' = 0

Function un.MGS_FindWindowsPowerShell
  StrCpy $R6 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$R6" +2 0
    StrCpy $R6 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
FunctionEnd

!macro MGS_NativeProgramAction phase program extra
  InitPluginsDir
  SetOutPath "$WINDIR"
  File "/oname=$PLUGINSDIR\main-native-payload.ps1" "${MGS_NATIVE_PAYLOAD_SCRIPT}"
  File "/oname=$PLUGINSDIR\candidate.json" "${MGS_NATIVE_MANIFEST}"
  !if "${phase}" != "Uninstall"
    File "/oname=$PLUGINSDIR\prior.json" "${MGS_NATIVE_CATALOG}"
  !endif
  Call ${extra}MGS_FindWindowsPowerShell
  nsExec::ExecToStack '"$R6" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\main-native-payload.ps1" -Phase ${phase} -Program "${program}\." -Stage "$PLUGINSDIR\main-native-files" -Manifest "$PLUGINSDIR\candidate.json" -Catalog "$PLUGINSDIR\prior.json" -UninstallerImage "$EXEPATH"'
  Pop $R0
  Pop $R1
!macroend

!macro MGS_InstallNativeProgram
  InitPluginsDir
  SetOutPath "$PLUGINSDIR\main-native-files"
  File "${MAINBINARYSRCPATH}"
  !if "$%MGS_MAIN_NATIVE_GNU%" == "1"
    File "$%MGS_MAIN_NATIVE_LOADER%"
    File "$%MGS_MAIN_NATIVE_UNWIND%"
  !endif
  !if "$%MGS_MAIN_NATIVE_PARSER%" == "1"
    File "/oname=maxgamestudio-parser-worker.exe" "$%MGS_MAIN_NATIVE_PARSER_EXE%"
    File "/oname=maxgamestudio-parser-worker.LICENSE.txt" "$%MGS_MAIN_NATIVE_PARSER_NOTICE%"
  !endif
  WriteUninstaller "$PLUGINSDIR\main-native-files\uninstall.exe"
  SetOutPath "$PLUGINSDIR\main-native-files\data"
  File "${MGS_NATIVE_ASSETS}\data\basic.ini"
  File "${MGS_NATIVE_ASSETS}\data\cs2-insight.config.example.json"
  File "${MGS_NATIVE_ASSETS}\data\lite_cut_effect_contract.json"
  File "${MGS_NATIVE_ASSETS}\data\lite_cut_visual_acceptance.json"
  SetOutPath "$PLUGINSDIR\main-native-files\pov"
  File "${MGS_NATIVE_ASSETS}\pov\gameinfo.gi.default"
  File "${MGS_NATIVE_ASSETS}\pov\gameinfo.gi.pov"
  File "${MGS_NATIVE_ASSETS}\pov\pov_default.vpk"
  !insertmacro MGS_NativeProgramAction "Install" "$INSTDIR" ""
  ${If} $R0 != 0
    StrCpy $R7 "无法验证或替换 MaxGameStudio 程序。请关闭应用、检查磁盘空间，并确认该目录内的旧版本受支持。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $R1 == "MGS_NATIVE_LEGACY_RETAINED"
    DetailPrint "部分旧版程序文件无法清理，已保留。"
  ${EndIf}
  SetOutPath "$INSTDIR"
!macroend

!macro MGS_RemoveNativeProgram
  Call un.MGS_ReserveInstallationDirectories
  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "另一个安装程序正在运行。请等待它完成后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ReadRegStr $0 SHCTX "${UNINSTKEY}" "InstallLocation"
  ${If} $0 != '$\"$INSTDIR$\"'
    MessageBox MB_ICONEXCLAMATION|MB_OK "无法验证 MaxGameStudio 的注册目录，已停止卸载。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  !insertmacro MGS_NativeProgramAction "Uninstall" "$INSTDIR" "un."
  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "无法安全移除 MaxGameStudio 程序。请关闭应用后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend
