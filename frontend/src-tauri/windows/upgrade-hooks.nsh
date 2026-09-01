; Electron -> Tauri installer bridge + self-upgrade hardening.
;
; Design goals (every path must also work fully silent, e.g. when the Tauri
; updater or electron-updater runs this installer with /S):
;   * User data under %APPDATA% is never touched by anything in this file.
;   * The exact legacy Tauri entry "CS2 Ultimate Insight Studio" is retired
;     only from its registered install directory; "MaxGameStudio" is ignored.
;   * A legacy Electron install in a DIFFERENT directory is only uninstalled
;     after the new files and the user-data migration are verified
;     (postinstall), keeping a runnable fallback until that point.
;   * A legacy Electron install in the SAME directory must be uninstalled
;     BEFORE files are copied: electron-builder's uninstaller deletes the
;     whole install directory and would otherwise wipe the freshly installed
;     Tauri files.
;   * A still-running Tauri shell is usually just finishing its graceful
;     backend shutdown; wait for it instead of aborting, and only force-kill
;     (including the Python backend child tree) as a last resort.

; tauri-build copies the GNU WebView2 loader beside the release executable,
; but tauri-bundler 2.6 does not add that sibling DLL to NSIS automatically.
; GNU Rust builds also import libunwind.dll dynamically. The build script stages
; the matching active-toolchain DLL beside the release executable before NSIS.
; Capture this directory while the hook is included so macro expansion later
; does not change __FILEDIR__ to the generated NSIS directory.
!define CS2_TAURI_RELEASE_DIR "${__FILEDIR__}\..\target\release"
!define CS2_LEGACY_TAURI_PRODUCT "CS2 Ultimate Insight Studio"
!define CS2_LEGACY_TAURI_UNINSTALL_KEY "CS2 Ultimate Insight Studio"
!define CS2_CURRENT_TAURI_PRODUCT "MaxGameStudio"
!define CS2_LEGACY_TAURI_EXE "cs2-insight-agent-desktop.exe"

Var CS2ElectronScope     ; "samedir" (preinstall) or "all" (postinstall)
Var CS2ElectronDir       ; lowercased install dir of the legacy entry, no trailing backslash
Var CS2ElectronUninsExe  ; parsed legacy uninstaller executable path
Var CS2LegacyTauriScope  ; "samedir" (preinstall) or "differentdir" (postinstall)
Var CS2LegacyTauriDir    ; lowercased install dir of the legacy entry, no trailing backslash
Var CS2LegacyTauriUninsExe ; parsed legacy Tauri uninstaller executable path

Function CS2_AbortMigrationInstall
  IfSilent cs2_abort_silent cs2_abort_interactive
  cs2_abort_interactive:
    MessageBox MB_ICONSTOP|MB_OK "$R7"
  cs2_abort_silent:
    SetErrorLevel 2
    Abort
FunctionEnd

; In: $R9 = image name. Out: $R0 = 1 running / 0 not running.
; installerHooks are included before Tauri registers its additional plugin
; directory, so use the stock NSIS nsExec plugin and Windows tasklist here.
; /FO CSV keeps the full image name intact and makes the exact lookup stable.
Function CS2_IsProcessRunning
  nsExec::ExecToStack '"$SYSDIR\tasklist.exe" /FI "IMAGENAME eq $R9" /FO CSV /NH'
  Pop $R0
  Pop $R1
  ${StrCase} $R2 $R1 "L"
  ${StrCase} $R3 $R9 "L"
  ${StrLoc} $R4 $R2 '"$R3"' ">"
  ${If} $R4 != ""
    StrCpy $R0 1
  ${Else}
    StrCpy $R0 0
  ${EndIf}
FunctionEnd

; In: $R9 = image name, $R8 = max iterations (500ms each).
; Out: $R0 = 1 still running / 0 gone.
Function CS2_WaitProcessGone
  StrCpy $R5 0
  cs2_wait_proc_loop:
    Call CS2_IsProcessRunning
    ${If} $R0 = 0
      Return
    ${EndIf}
    IntOp $R5 $R5 + 1
    ${If} $R5 >= $R8
      Return
    ${EndIf}
    Sleep 500
    Goto cs2_wait_proc_loop
FunctionEnd

Function CS2_PrepareRunningApps
  ; The Tauri shell closes its window instantly but the process can keep
  ; running for several seconds while the Python backend shuts down. Wait for
  ; that instead of aborting; force-kill the whole child tree only if it never
  ; exits (e.g. a hung backend).
  StrCpy $R9 "cs2-insight-agent-desktop.exe"
  Call CS2_IsProcessRunning
  ${If} $R0 = 1
    DetailPrint "等待正在退出的 MaxGameStudio 进程结束…"
    StrCpy $R8 50
    Call CS2_WaitProcessGone
    ${If} $R0 = 1
      DetailPrint "强制结束仍在运行的 MaxGameStudio…"
      nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /IM "cs2-insight-agent-desktop.exe" /F /T'
      Pop $R0
      Pop $R1
      Sleep 1500
      StrCpy $R9 "cs2-insight-agent-desktop.exe"
      Call CS2_IsProcessRunning
      ${If} $R0 = 1
        StrCpy $R7 "无法结束仍在运行的 MaxGameStudio。$\r$\n$\r$\n请手动关闭应用（必要时在任务管理器结束进程），再重新运行安装程序。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ; A backend orphaned by an earlier force-kill keeps port 19871 busy and
  ; would make the freshly installed app fail its startup identity check.
  ; Only python.exe listeners on that port are terminated.
  nsExec::ExecToStack `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command "Get-NetTCPConnection -LocalPort 19871 -State Listen -ErrorAction SilentlyContinue | ForEach-Object { $$proc = Get-Process -Id $$_.OwningProcess -ErrorAction SilentlyContinue; if ($$proc -and $$proc.ProcessName -eq 'python') { Stop-Process -Id $$proc.Id -Force } }"`
  Pop $R0
  Pop $R1

  ; Legacy Electron build: give it time to finish exiting (electron-updater
  ; launches this installer right after quitting the app), but never
  ; force-kill it — it may still be flushing config or the SQLite database.
  StrCpy $R9 "CS2 Insight Agent.exe"
  Call CS2_IsProcessRunning
  ${If} $R0 = 1
    DetailPrint "等待旧版应用 (Electron) 退出…"
    StrCpy $R8 40
    Call CS2_WaitProcessGone
    ${If} $R0 = 1
      StrCpy $R7 "检测到旧版应用 (Electron) 正在运行。$\r$\n$\r$\n为避免配置或数据库损坏，请先正常关闭旧版应用，再重新运行安装程序。"
      Call CS2_AbortMigrationInstall
    ${EndIf}
  ${EndIf}
FunctionEnd

; Tauri's in-place installer overwrites files but does not remove package
; directories that disappeared from a newer bundled Python runtime. Remove
; the parser package and every historical dist-info directory before file
; copy so importlib.metadata can never resolve a stale local version.
Function CS2_RemoveBundledDemoparser
  Push $0
  Push $1

  RMDir /r "$INSTDIR\python\Lib\site-packages\demoparser2"
  FindFirst $0 $1 "$INSTDIR\python\Lib\site-packages\demoparser2-*.dist-info"
  cs2_remove_demoparser_metadata_loop:
    StrCmp $1 "" cs2_remove_demoparser_metadata_done
    RMDir /r "$INSTDIR\python\Lib\site-packages\$1"
    ClearErrors
    FindNext $0 $1
    IfErrors cs2_remove_demoparser_metadata_done
    Goto cs2_remove_demoparser_metadata_loop
  cs2_remove_demoparser_metadata_done:
    FindClose $0

  Pop $1
  Pop $0
FunctionEnd

; Validate the runtime that is physically present in $INSTDIR.  This is kept
; as a function because the post-install migration still performs destructive
; legacy cleanup after the first validation.  A second call at the very end
; prevents the installer from reporting success if that cleanup removed or
; damaged files from the freshly installed runtime.
Function CS2_ValidateBundledRuntime
  ClearErrors
  ExecWait '"$INSTDIR\python\python.exe" -I "$INSTDIR\backend\app\demoparser_runtime.py"' $R0
  ${If} ${Errors}
    StrCpy $R7 "Tauri 已安装，但无法执行内置 Rust Demo 解析器校验。安装已停止，请重新运行完整安装包。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $R0 != 0
    StrCpy $R7 "内置 Rust Demo 解析器版本校验失败（退出码 $R0）。安装已停止，请重新下载完整安装包。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
FunctionEnd

; productName changed from the stable Tauri release to MaxGameStudio. The
; generated installer creates the new shortcuts, but an in-place update does
; not know the historical shortcut filenames. Remove only these exact paths;
; never recurse through the Desktop or Start Menu and never touch user data.
Function CS2_RemoveLegacyBrandShortcuts
  Delete "$DESKTOP\CS2 Ultimate Insight Studio.lnk"
  Delete "$SMPROGRAMS\CS2 Ultimate Insight Studio.lnk"
  Delete "$SMPROGRAMS\CS2 Ultimate Insight Studio\CS2 Ultimate Insight Studio.lnk"
  RMDir "$SMPROGRAMS\CS2 Ultimate Insight Studio"
FunctionEnd

; In: $R0 = raw InstallLocation, $R8 = raw UninstallString.
; Out: $CS2LegacyTauriDir, $CS2LegacyTauriUninsExe.
; The old Tauri NSIS uninstaller is normally registered as a quoted path to
; uninstall.exe. Keep the parser conservative so no registry arguments are
; ever executed as part of the migration command.
Function CS2_ResolveLegacyTauriDir
  Push $0
  Push $1

  ; Parse only the executable path from the uninstall command.
  StrCpy $0 $R8
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 "" 1
    ${StrLoc} $1 $0 '"' ">"
    ${If} $1 != ""
      StrCpy $0 $0 $1
    ${EndIf}
  ${EndIf}
  StrCpy $CS2LegacyTauriUninsExe $0

  ; Prefer the registered InstallLocation, else the uninstaller's directory.
  StrCpy $1 $R0
  StrCpy $0 $1 1
  ${If} $0 == '"'
    StrCpy $1 $1 "" 1
  ${EndIf}
  StrCpy $0 $1 1 -1
  ${If} $0 == '"'
    StrCpy $1 $1 -1
  ${EndIf}
  ${If} $1 == ""
    ${GetParent} $CS2LegacyTauriUninsExe $1
  ${EndIf}
  StrCpy $0 $1 1 -1
  ${If} $0 == "\"
    StrCpy $1 $1 -1
  ${EndIf}
  ${StrCase} $CS2LegacyTauriDir $1 "L"

  Pop $1
  Pop $0
FunctionEnd

; Out: $R0 = 1 when the legacy Tauri entry lives in the directory being
; installed to. Path comparison is case-insensitive and ignores one trailing
; separator, matching Windows registry path semantics.
Function CS2_LegacyTauriDirMatchesInstDir
  Push $0
  Push $1
  StrCpy $1 $INSTDIR
  StrCpy $0 $1 1 -1
  ${If} $0 == "\"
    StrCpy $1 $1 -1
  ${EndIf}
  ${StrCase} $1 $1 "L"
  StrCpy $R0 0
  ${If} $CS2LegacyTauriDir != ""
  ${AndIf} $CS2LegacyTauriDir == $1
    StrCpy $R0 1
  ${EndIf}
  Pop $1
  Pop $0
FunctionEnd

Function CS2_RunLegacyTauriUninstaller
  ; Run only the parsed executable and add NSIS silent mode explicitly. Do not
  ; pass any data-deletion switch: the old user's %APPDATA% remains intact.
  DetailPrint "正在静默卸载旧版 Tauri 应用…"
  ClearErrors
  ExecWait '"$CS2LegacyTauriUninsExe" /S' $R0
  ${If} ${Errors}
    StrCpy $R7 "无法启动旧版 Tauri 卸载程序。安装已中止，旧程序和用户数据均未删除。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $R0 != 0
    StrCpy $R7 "旧版 Tauri 卸载没有成功完成（退出码 $R0）。安装已中止，旧程序和用户数据均未删除。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
FunctionEnd

; The silent NSIS uninstaller may copy itself to %TEMP% before returning. Do
; not let same-directory installation race the old executable's removal.
Function CS2_WaitLegacyTauriUninstallerGone
  Push $0
  StrCpy $0 0
  cs2_legacy_tauri_unins_gone_loop:
    IfFileExists "$CS2LegacyTauriUninsExe" 0 cs2_legacy_tauri_unins_gone_done
    IntOp $0 $0 + 1
    ${If} $0 < 30
      Sleep 500
      Goto cs2_legacy_tauri_unins_gone_loop
    ${EndIf}
    StrCpy $R7 "旧版 Tauri 卸载程序仍在运行或未能移除旧安装。安装已中止，旧程序和用户数据均未删除。"
    Call CS2_AbortMigrationInstall
  cs2_legacy_tauri_unins_gone_done:
    Sleep 500
  Pop $0
FunctionEnd

Function CS2_VerifyLegacyTauriRetired
  ; Never remove an arbitrary leftover directory: only verify that the old
  ; registered application binary is gone after its own uninstaller ran.
  IfFileExists "$CS2LegacyTauriDir\${CS2_LEGACY_TAURI_EXE}" 0 cs2_legacy_tauri_retired
  StrCpy $R7 "旧版 Tauri 主程序仍然存在，安装已中止以避免并排运行。旧程序和用户数据均未删除。"
  Call CS2_AbortMigrationInstall
  cs2_legacy_tauri_retired:
FunctionEnd

; Remove one exact old Tauri registry entry from HKCU. The DisplayName and key
; must both be exactly the known legacy product name; in particular an entry
; for MaxGameStudio is explicitly excluded.
Function CS2_RemoveLegacyTauriHKCU
  StrCpy $R4 0
  cs2_legacy_tauri_hkcu_loop:
    EnumRegKey $R5 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall" $R4
    StrCmp $R5 "" cs2_legacy_tauri_hkcu_done
    IntOp $R4 $R4 + 1

    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "DisplayName"
    ${If} $R0 == "${CS2_CURRENT_TAURI_PRODUCT}"
      Goto cs2_legacy_tauri_hkcu_loop
    ${EndIf}
    ${If} $R0 != "${CS2_LEGACY_TAURI_PRODUCT}"
      Goto cs2_legacy_tauri_hkcu_loop
    ${EndIf}
    ${If} $R5 != "${CS2_LEGACY_TAURI_UNINSTALL_KEY}"
      Goto cs2_legacy_tauri_hkcu_loop
    ${EndIf}

    ReadRegStr $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "UninstallString"
    ${If} $R8 == ""
      StrCpy $R7 "发现旧版 Tauri 卸载项，但缺少 UninstallString。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}
    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "InstallLocation"
    Call CS2_ResolveLegacyTauriDir
    ${If} $CS2LegacyTauriDir == ""
      StrCpy $R7 "无法解析旧版 Tauri 安装目录。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}

    ; Respect the two-phase migration boundary before either running the old
    ; uninstaller or retiring a stale entry. PREINSTALL only handles the old
    ; entry that points at $INSTDIR; POSTINSTALL handles a different directory
    ; after the new runtime and data migration have already been verified.
    ${If} $CS2LegacyTauriScope == "samedir"
      Call CS2_LegacyTauriDirMatchesInstDir
      ${If} $R0 != 1
        Goto cs2_legacy_tauri_hkcu_loop
      ${EndIf}
    ${ElseIf} $CS2LegacyTauriScope == "differentdir"
      Call CS2_LegacyTauriDirMatchesInstDir
      ${If} $R0 == 1
        StrCpy $R7 "旧版 Tauri 注册项仍指向当前安装目录。安装已中止，避免卸载新版本。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
    ${Else}
      StrCpy $R7 "旧版 Tauri 迁移阶段无效。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    IfFileExists "$CS2LegacyTauriUninsExe" 0 cs2_legacy_tauri_hkcu_missing_uninstaller
    Goto cs2_legacy_tauri_hkcu_have_uninstaller
    cs2_legacy_tauri_hkcu_missing_uninstaller:
      ; A previous migration can leave only a stale uninstall registry entry
      ; (for example after the user manually removed uninstall.exe). There is
      ; no executable left to run, so retire this exact entry and continue;
      ; never recurse through the old directory or touch %APPDATA% data.
      DetailPrint "旧版 Tauri 卸载程序不存在，清理失效的注册项并继续安装…"
      ClearErrors
      DeleteRegKey HKCU "$R9"
      ${If} ${Errors}
        StrCpy $R7 "旧版 Tauri 失效注册项无法清理。安装已中止，旧程序和用户数据均未删除。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
      ReadRegStr $R3 HKCU "$R9" "DisplayName"
      ${If} $R3 != ""
        StrCpy $R7 "旧版 Tauri 失效注册项无法清理。安装已中止，旧程序和用户数据均未删除。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
      StrCpy $R4 0
      Goto cs2_legacy_tauri_hkcu_loop
    cs2_legacy_tauri_hkcu_have_uninstaller:

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    Call CS2_RunLegacyTauriUninstaller
    StrCpy $R6 0
    cs2_legacy_tauri_hkcu_wait_removed:
      ReadRegStr $R3 HKCU "$R9" "DisplayName"
      ${If} $R3 == ""
        Goto cs2_legacy_tauri_hkcu_removed
      ${EndIf}
      IntOp $R6 $R6 + 1
      ${If} $R6 < 30
        Sleep 500
        Goto cs2_legacy_tauri_hkcu_wait_removed
      ${EndIf}
      StrCpy $R7 "旧版 Tauri 卸载程序返回成功，但卸载项仍然存在。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    cs2_legacy_tauri_hkcu_removed:
    Call CS2_WaitLegacyTauriUninstallerGone
    Call CS2_VerifyLegacyTauriRetired
    StrCpy $R4 0
    Goto cs2_legacy_tauri_hkcu_loop
  cs2_legacy_tauri_hkcu_done:
FunctionEnd

; HKLM counterpart for installations registered for all users.
Function CS2_RemoveLegacyTauriHKLM
  StrCpy $R4 0
  cs2_legacy_tauri_hklm_loop:
    EnumRegKey $R5 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall" $R4
    StrCmp $R5 "" cs2_legacy_tauri_hklm_done
    IntOp $R4 $R4 + 1

    ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "DisplayName"
    ${If} $R0 == "${CS2_CURRENT_TAURI_PRODUCT}"
      Goto cs2_legacy_tauri_hklm_loop
    ${EndIf}
    ${If} $R0 != "${CS2_LEGACY_TAURI_PRODUCT}"
      Goto cs2_legacy_tauri_hklm_loop
    ${EndIf}
    ${If} $R5 != "${CS2_LEGACY_TAURI_UNINSTALL_KEY}"
      Goto cs2_legacy_tauri_hklm_loop
    ${EndIf}

    ReadRegStr $R8 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "UninstallString"
    ${If} $R8 == ""
      StrCpy $R7 "发现旧版 Tauri 卸载项，但缺少 UninstallString。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}
    ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "InstallLocation"
    Call CS2_ResolveLegacyTauriDir
    ${If} $CS2LegacyTauriDir == ""
      StrCpy $R7 "无法解析旧版 Tauri 安装目录。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}

    ; Keep HKLM on the same two-phase boundary as HKCU.
    ${If} $CS2LegacyTauriScope == "samedir"
      Call CS2_LegacyTauriDirMatchesInstDir
      ${If} $R0 != 1
        Goto cs2_legacy_tauri_hklm_loop
      ${EndIf}
    ${ElseIf} $CS2LegacyTauriScope == "differentdir"
      Call CS2_LegacyTauriDirMatchesInstDir
      ${If} $R0 == 1
        StrCpy $R7 "旧版 Tauri 注册项仍指向当前安装目录。安装已中止，避免卸载新版本。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
    ${Else}
      StrCpy $R7 "旧版 Tauri 迁移阶段无效。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    ${EndIf}

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    IfFileExists "$CS2LegacyTauriUninsExe" 0 cs2_legacy_tauri_hklm_missing_uninstaller
    Goto cs2_legacy_tauri_hklm_have_uninstaller
    cs2_legacy_tauri_hklm_missing_uninstaller:
      ; See the HKCU branch above. HKLM is handled with the same exact-key
      ; guard, while the old files remain untouched for a manual cleanup.
      DetailPrint "旧版 Tauri 卸载程序不存在，清理失效的系统注册项并继续安装…"
      ClearErrors
      DeleteRegKey HKLM "$R9"
      ${If} ${Errors}
        StrCpy $R7 "旧版 Tauri 失效系统注册项无法清理。安装已中止，旧程序和用户数据均未删除。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
      ReadRegStr $R3 HKLM "$R9" "DisplayName"
      ${If} $R3 != ""
        StrCpy $R7 "旧版 Tauri 失效系统注册项无法清理。安装已中止，旧程序和用户数据均未删除。"
        Call CS2_AbortMigrationInstall
      ${EndIf}
      StrCpy $R4 0
      Goto cs2_legacy_tauri_hklm_loop
    cs2_legacy_tauri_hklm_have_uninstaller:

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    Call CS2_RunLegacyTauriUninstaller
    StrCpy $R6 0
    cs2_legacy_tauri_hklm_wait_removed:
      ReadRegStr $R3 HKLM "$R9" "DisplayName"
      ${If} $R3 == ""
        Goto cs2_legacy_tauri_hklm_removed
      ${EndIf}
      IntOp $R6 $R6 + 1
      ${If} $R6 < 30
        Sleep 500
        Goto cs2_legacy_tauri_hklm_wait_removed
      ${EndIf}
      StrCpy $R7 "旧版 Tauri 卸载程序返回成功，但系统级卸载项仍然存在。安装已中止，旧程序和用户数据均未删除。"
      Call CS2_AbortMigrationInstall
    cs2_legacy_tauri_hklm_removed:
    Call CS2_WaitLegacyTauriUninstallerGone
    Call CS2_VerifyLegacyTauriRetired
    StrCpy $R4 0
    Goto cs2_legacy_tauri_hklm_loop
  cs2_legacy_tauri_hklm_done:
FunctionEnd

Function CS2_RemoveLegacyTauri
  ${If} $CS2LegacyTauriScope != "samedir"
  ${AndIf} $CS2LegacyTauriScope != "differentdir"
    StrCpy $R7 "旧版 Tauri 迁移阶段无效。安装已中止，旧程序和用户数据均未删除。"
    Call CS2_AbortMigrationInstall
  ${EndIf}

  ${If} ${RunningX64}
    SetRegView 64
    Call CS2_RemoveLegacyTauriHKCU
    Call CS2_RemoveLegacyTauriHKLM
    SetRegView 32
    Call CS2_RemoveLegacyTauriHKCU
    Call CS2_RemoveLegacyTauriHKLM
  ${Else}
    SetRegView 32
    Call CS2_RemoveLegacyTauriHKCU
    Call CS2_RemoveLegacyTauriHKLM
  ${EndIf}

  ; Restore the registry view selected by the generated Tauri installer.
  !insertmacro SetContext
FunctionEnd

; In: $R0 = raw InstallLocation, $R8 = raw UninstallString.
; Out: $CS2ElectronDir, $CS2ElectronUninsExe.
Function CS2_ResolveElectronDir
  Push $0
  Push $1

  ; Parsed uninstaller path: strip surrounding quotes and trailing arguments.
  StrCpy $0 $R8
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 "" 1
    ${StrLoc} $1 $0 '"' ">"
    ${If} $1 != ""
      StrCpy $0 $0 $1
    ${EndIf}
  ${EndIf}
  StrCpy $CS2ElectronUninsExe $0

  ; Prefer the registered InstallLocation, else the uninstaller's directory.
  StrCpy $1 $R0
  StrCpy $0 $1 1
  ${If} $0 == '"'
    StrCpy $1 $1 "" 1
  ${EndIf}
  StrCpy $0 $1 1 -1
  ${If} $0 == '"'
    StrCpy $1 $1 -1
  ${EndIf}
  ${If} $1 == ""
    ${GetParent} $CS2ElectronUninsExe $1
  ${EndIf}
  StrCpy $0 $1 1 -1
  ${If} $0 == "\"
    StrCpy $1 $1 -1
  ${EndIf}
  ${StrCase} $CS2ElectronDir $1 "L"

  Pop $1
  Pop $0
FunctionEnd

; Out: $R0 = 1 when the legacy entry lives in the directory being installed to.
Function CS2_ElectronDirMatchesInstDir
  Push $0
  Push $1
  StrCpy $1 $INSTDIR
  StrCpy $0 $1 1 -1
  ${If} $0 == "\"
    StrCpy $1 $1 -1
  ${EndIf}
  ${StrCase} $1 $1 "L"
  StrCpy $R0 0
  ${If} $CS2ElectronDir != ""
  ${AndIf} $CS2ElectronDir == $1
    StrCpy $R0 1
  ${EndIf}
  Pop $1
  Pop $0
FunctionEnd

Function CS2_RunElectronUninstaller
  ; Inputs: $R8 = uninstall command. Fully silent by design: electron-builder's
  ; /S uninstall keeps the %APPDATA% user data in place, and the user asked for
  ; a no-questions upgrade path.
  DetailPrint "正在静默卸载旧版应用 (Electron)…"
  ClearErrors
  ExecWait '$R8 /S' $R0
  ${If} ${Errors}
    StrCpy $R7 "无法启动旧版 Electron 卸载程序。安装已中止，旧程序和用户数据均未删除。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $R0 != 0
    StrCpy $R7 "旧版 Electron 卸载没有成功完成（退出码 $R0）。安装已中止，避免留下两套互相冲突的程序。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
FunctionEnd

; The silent NSIS uninstaller copies itself to %TEMP% and returns before the
; real work finishes. After the registry entry disappears, also wait for the
; uninstaller executable (and with it the directory deletion) to settle so a
; same-directory install cannot race the file copy.
Function CS2_WaitElectronUninsGone
  Push $0
  StrCpy $0 0
  cs2_unins_gone_loop:
    IfFileExists "$CS2ElectronUninsExe" 0 cs2_unins_gone_done
    IntOp $0 $0 + 1
    ${If} $0 < 30
      Sleep 500
      Goto cs2_unins_gone_loop
    ${EndIf}
  cs2_unins_gone_done:
    Sleep 500
    Pop $0
FunctionEnd

; electron-builder and Tauri use different NSIS identities even though the
; product name is the same. Detect the old electron-builder uninstaller by
; its executable name instead of relying on a single generated registry GUID.
Function CS2_RemoveElectronHKCU
  StrCpy $R4 0
  cs2_hkcu_loop:
    EnumRegKey $R5 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall" $R4
    StrCmp $R5 "" cs2_hkcu_done
    IntOp $R4 $R4 + 1

    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "DisplayName"
    ${StrCase} $R1 $R0 "L"
    ${StrLoc} $R2 $R1 "cs2 insight agent" ">"
    StrCmp $R2 0 0 cs2_hkcu_loop

    ReadRegStr $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "UninstallString"
    ${StrCase} $R1 $R8 "L"
    ${StrLoc} $R2 $R1 "uninstall cs2 insight agent.exe" ">"
    StrCmp $R2 "" cs2_hkcu_loop

    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "InstallLocation"
    Call CS2_ResolveElectronDir
    ${If} $CS2ElectronScope == "samedir"
      Call CS2_ElectronDirMatchesInstDir
      StrCmp $R0 1 0 cs2_hkcu_loop
    ${EndIf}

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    Call CS2_RunElectronUninstaller
    StrCpy $R6 0
    cs2_hkcu_wait_removed:
    ReadRegStr $R3 HKCU "$R9" "UninstallString"
    ${If} $R3 == ""
      Goto cs2_hkcu_removed
    ${EndIf}
    IntOp $R6 $R6 + 1
    ${If} $R6 < 30
      Sleep 500
      Goto cs2_hkcu_wait_removed
    ${EndIf}
    StrCpy $R7 "旧版 Electron 卸载程序返回成功，但等待 15 秒后卸载注册项仍然存在。安装已中止，请勿继续并排安装。"
    Call CS2_AbortMigrationInstall
    cs2_hkcu_removed:
    Call CS2_WaitElectronUninsGone
    StrCpy $R4 0
    Goto cs2_hkcu_loop
  cs2_hkcu_done:
FunctionEnd

Function CS2_RemoveElectronHKLM
  StrCpy $R4 0
  cs2_hklm_loop:
    EnumRegKey $R5 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall" $R4
    StrCmp $R5 "" cs2_hklm_done
    IntOp $R4 $R4 + 1

    ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "DisplayName"
    ${StrCase} $R1 $R0 "L"
    ${StrLoc} $R2 $R1 "cs2 insight agent" ">"
    StrCmp $R2 0 0 cs2_hklm_loop

    ReadRegStr $R8 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "UninstallString"
    ${StrCase} $R1 $R8 "L"
    ${StrLoc} $R2 $R1 "uninstall cs2 insight agent.exe" ">"
    StrCmp $R2 "" cs2_hklm_loop

    ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5" "InstallLocation"
    Call CS2_ResolveElectronDir
    ${If} $CS2ElectronScope == "samedir"
      Call CS2_ElectronDirMatchesInstDir
      StrCmp $R0 1 0 cs2_hklm_loop
    ${EndIf}

    StrCpy $R9 "Software\Microsoft\Windows\CurrentVersion\Uninstall\$R5"
    Call CS2_RunElectronUninstaller
    StrCpy $R6 0
    cs2_hklm_wait_removed:
    ReadRegStr $R3 HKLM "$R9" "UninstallString"
    ${If} $R3 == ""
      Goto cs2_hklm_removed
    ${EndIf}
    IntOp $R6 $R6 + 1
    ${If} $R6 < 30
      Sleep 500
      Goto cs2_hklm_wait_removed
    ${EndIf}
    StrCpy $R7 "旧版 Electron 卸载程序返回成功，但等待 15 秒后系统级卸载注册项仍然存在。安装已中止，请勿继续并排安装。"
    Call CS2_AbortMigrationInstall
    cs2_hklm_removed:
    Call CS2_WaitElectronUninsGone
    StrCpy $R4 0
    Goto cs2_hklm_loop
  cs2_hklm_done:
FunctionEnd

Function CS2_RemoveLegacyElectron
  ${If} ${RunningX64}
    SetRegView 64
    Call CS2_RemoveElectronHKCU
    Call CS2_RemoveElectronHKLM
    SetRegView 32
    Call CS2_RemoveElectronHKCU
    Call CS2_RemoveElectronHKLM
  ${Else}
    SetRegView 32
    Call CS2_RemoveElectronHKCU
    Call CS2_RemoveElectronHKLM
  ${EndIf}

  ; Restore the registry view selected by the generated Tauri installer.
  !insertmacro SetContext
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  Call CS2_PrepareRunningApps

  ; Same-directory legacy installs must be retired before any file copy —
  ; their uninstallers may delete $INSTDIR together with the new files.
  ; Different-directory installs stay untouched until NSIS_HOOK_POSTINSTALL.
  ; Do not hold $INSTDIR as the working directory while it may be deleted.
  SetOutPath $PLUGINSDIR
  StrCpy $CS2LegacyTauriScope "samedir"
  Call CS2_RemoveLegacyTauri
  StrCpy $CS2ElectronScope "samedir"
  Call CS2_RemoveLegacyElectron
  SetOutPath $INSTDIR

  ; Delete every previous patched-parser generation before NSIS copies the
  ; newly staged runtime. This also makes same-version repair installs clean.
  Call CS2_RemoveBundledDemoparser

  ; GNU builds import WebView2Loader.dll dynamically. Keep it beside the main
  ; executable so Windows can resolve the dependency before Rust/Tauri starts.
  !if /FileExists "${CS2_TAURI_RELEASE_DIR}\WebView2Loader.dll"
    File /a "/oname=WebView2Loader.dll" "${CS2_TAURI_RELEASE_DIR}\WebView2Loader.dll"
  !endif
  ; LLVM-MinGW (gnullvm) also imports its unwind runtime dynamically.
  !if /FileExists "${CS2_TAURI_RELEASE_DIR}\libunwind.dll"
    File /a "/oname=libunwind.dll" "${CS2_TAURI_RELEASE_DIR}\libunwind.dll"
  !endif
!macroend

!macro NSIS_HOOK_POSTINSTALL
  RMDir /r "$INSTDIR\python\Lib\site-packages\pyarrow"
  RMDir /r "$INSTDIR\python\Lib\site-packages\pyarrow.libs"
  RMDir /r "$INSTDIR\python\Lib\site-packages\pyarrow-25.0.0.dist-info"

  ; Validate the exact installed runtime before the finish page can launch
  ; Tauri. This catches stale metadata, a missing extension, or an incomplete
  ; file copy at install time instead of surfacing as a backend startup dialog.
  Call CS2_ValidateBundledRuntime

  ; Run the same idempotent migration used by the desktop startup before the
  ; finish page can launch Tauri. A failure leaves every legacy source intact.
  ClearErrors
  ExecWait '"$INSTDIR\python\python.exe" -I "$INSTDIR\backend\app\desktop_data_migration.py" --appdata "$APPDATA" --require-desktop-stopped --require-electron-ui-export' $R0
  ${If} ${Errors}
    StrCpy $R7 "MaxGameStudio 已安装，但无法启动用户数据迁移程序。旧数据仍然保留；安装已停止，请查看应用数据目录中的 desktop-data-migration-error.log。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $R0 != 0
    StrCpy $R7 "用户数据迁移校验失败（退出码 $R0）。旧数据仍然保留，应用不会以空配置启动。请查看应用数据目录中的 desktop-data-migration-error.log。"
    Call CS2_AbortMigrationInstall
  ${EndIf}

  ; Only retire different-directory legacy installs after the new runtime and
  ; user-data migration are known-good. Any earlier failure leaves a runnable
  ; legacy fallback in place.
  StrCpy $CS2ElectronScope "all"
  Call CS2_RemoveLegacyElectron
  StrCpy $CS2LegacyTauriScope "differentdir"
  Call CS2_RemoveLegacyTauri

  ; The generated installer has already created/updated the MaxGameStudio
  ; shortcut at this point. Remove only the exact old-brand shortcut paths.
  Call CS2_RemoveLegacyBrandShortcuts

  ; Legacy uninstallers run after the first validation and can remove files
  ; asynchronously on some upgrade paths.  Never show the success page until
  ; the final on-disk runtime has passed the same import/version contract.
  Call CS2_ValidateBundledRuntime
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\libunwind.dll"
  ; The generated uninstaller tries to remove $INSTDIR before this custom
  ; file is deleted. Retry non-recursively once the loader is gone.
  RmDir "$INSTDIR"
!macroend
