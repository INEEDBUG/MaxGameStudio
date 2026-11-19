; The outer installer stays asInvoker. Only its machine write phase is
; delegated to the elevated child. HKCU and data selection belong to the outer.
!define MGS_SCOPE_DIR "${__FILEDIR__}"
!addplugindir "${MGS_SCOPE_DIR}\vendor\uac"
!include "${MGS_SCOPE_DIR}\vendor\uac\UAC.nsh"

Function MGS_ReadScopeDirectories
  StrCpy $MGSMachineTarget "$PROGRAMFILES64\${PRODUCTNAME}"
  ReadRegStr $0 HKLM "${UNINSTKEY}" "InstallLocation"
  StrCmp $0 '$\"$MGSMachineTarget$\"' 0 mgs_find_user_install
  ReadRegDWORD $0 HKLM "${UNINSTKEY}" "AllUsers"
  StrCmp $0 1 0 mgs_find_user_install
  IfFileExists "$MGSMachineTarget\${MAINBINARYNAME}.exe" 0 mgs_find_user_install
  StrCpy $MGSMachineProgramDir $MGSMachineTarget
  mgs_find_user_install:
  ReadRegStr $0 HKCU "${MANUPRODUCTKEY}" ""
  StrCmp $0 "" 0 mgs_check_user_install
  ReadRegStr $0 HKCU "${UNINSTKEY}" "InstallLocation"
  StrCpy $1 $0 1
  StrCmp $1 '$\"' 0 mgs_check_user_install
    StrCpy $0 $0 -1 1
  mgs_check_user_install:
  StrCmp $0 "" mgs_read_scope_done
  IfFileExists "$0\${MAINBINARYNAME}.exe" 0 mgs_read_scope_done
  StrCpy $MGSUserProgramDir $0
  mgs_read_scope_done:
FunctionEnd

Function MGS_ResolveCommandScope
  StrCpy $R0 2
  StrCpy $MGSMachineScope 0
  StrCpy $MGSScopeRequested ""
  StrCpy $MGSExplicitProgramDir ""
  System::Call 'kernel32::GetCommandLineW() w.r1'
  ClearErrors
  ${GetOptions} $1 "/D=" $0
  IfErrors +2 0
    GetFullPathName $MGSExplicitProgramDir "$INSTDIR"
  ClearErrors
  ${GetOptions} $CMDLINE "/ALLUSERS" $0
  IfErrors +2 0
    StrCpy $MGSScopeRequested "all"
  ClearErrors
  ${GetOptions} $CMDLINE "/CURRENTUSER" $0
  IfErrors mgs_command_scope_detect
  StrCmp $MGSScopeRequested "" 0 mgs_command_scope_done
  StrCpy $MGSScopeRequested "current"

  mgs_command_scope_detect:
  StrCmp $MGSScopeRequested "all" mgs_command_scope_machine
  StrCmp $MGSExplicitProgramDir "" mgs_command_scope_default
  StrCmp $MGSExplicitProgramDir $MGSMachineTarget 0 mgs_command_scope_user
  StrCmp $UpdateMode 1 0 mgs_command_scope_done
  StrCmp $MGSScopeRequested "current" mgs_command_scope_done
  Goto mgs_command_scope_machine

  mgs_command_scope_default:
  StrCmp $MGSScopeRequested "current" mgs_command_scope_user
  StrCmp $MGSUserProgramDir "" 0 mgs_command_scope_user
  StrCmp $MGSMachineProgramDir "" mgs_command_scope_user

  mgs_command_scope_machine:
  StrCmp $MGSExplicitProgramDir "" mgs_command_scope_machine_update
  StrCmp $MGSExplicitProgramDir $MGSMachineTarget 0 mgs_command_scope_done
  mgs_command_scope_machine_update:
  StrCmp $UpdateMode 1 0 +2
    StrCmp $MGSMachineProgramDir "" mgs_command_scope_done
  StrCpy $MGSMachineScope 1
  mgs_command_scope_user:
  StrCpy $R0 0
  mgs_command_scope_done:
FunctionEnd

Function MGS_ApplyScope
  StrCmp $MGSMachineScope 1 0 mgs_apply_current_scope
    SetShellVarContext all
    StrCpy $INSTDIR $MGSMachineTarget
    StrCpy $MGSExistingProgramDir $MGSMachineProgramDir
    Return
  mgs_apply_current_scope:
    SetShellVarContext current
    StrCpy $INSTDIR $MGSUserInstallDirChoice
    StrCpy $MGSExistingProgramDir $MGSUserProgramDir
FunctionEnd

Function MGS_InitInstallScope
  StrCpy $MGSUserInstallDirChoice $INSTDIR
  StrCpy $MGSInnerInstaller 0
  ${If} ${UAC_IsInnerInstance}
    ${IfNot} ${UAC_IsAdmin}
      SetErrorLevel 2
      Quit
    ${EndIf}
    StrCpy $MGSInnerInstaller 1
    StrCpy $MGSMachineScope 1
    StrCpy $MGSMachineTarget "$PROGRAMFILES64\${PRODUCTNAME}"
    ; Recheck the machine registration in the elevated write process.
    ReadRegStr $0 HKLM "${UNINSTKEY}" "InstallLocation"
    ${If} $0 == '$\"$MGSMachineTarget$\"'
      ReadRegDWORD $0 HKLM "${UNINSTKEY}" "AllUsers"
      ${If} $0 = 1
        StrCpy $MGSMachineProgramDir $MGSMachineTarget
      ${EndIf}
    ${EndIf}
    ${If} $UpdateMode = 1
    ${AndIf} $MGSMachineProgramDir == ""
      SetErrorLevel 2
      Quit
    ${EndIf}
    Call MGS_ApplyScope
    Return
  ${EndIf}

  ${If} ${UAC_IsAdmin}
    StrCpy $MGSDirectAdmin 1
  ${EndIf}
  Call MGS_ReadScopeDirectories
  Call MGS_ResolveCommandScope
  ${If} $R0 != 0
    StrCpy $R7 "指定的安装范围与原程序目录不匹配。为所有用户安装时，程序将放在受保护的 Program Files 目录。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ${If} $MGSMachineScope != 1
    Call MGS_ApplyOverlayDirectory
    StrCpy $MGSUserInstallDirChoice $INSTDIR
  ${EndIf}
  Call MGS_ApplyScope
FunctionEnd

Function MGS_ShowInstallScope
  Call MGS_SkipInputPages
  StrCmp $UpdateMode 1 0 +2
    Abort
  !insertmacro MUI_HEADER_TEXT "安装范围" "选择哪些 Windows 用户可以使用 MaxGameStudio"
  nsDialogs::Create 1018
  Pop $0
  StrCmp $0 "error" 0 +2
    Abort
  ${NSD_CreateRadioButton} 0 0 100% 14u "仅为当前用户安装"
  Pop $MGSUserRadio
  ${NSD_CreateRadioButton} 0 20u 100% 14u "为所有用户安装"
  Pop $MGSMachineRadio
  ${NSD_CreateLabel} 0 43u 100% 43u "为所有用户安装时，程序放在 $MGSMachineTarget，并在确认安装时请求管理员权限。每个 Windows 用户仍使用各自的数据目录。"
  Pop $0
  ${If} $MGSMachineScope = 1
    ${NSD_Check} $MGSMachineRadio
  ${Else}
    ${NSD_Check} $MGSUserRadio
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function MGS_LeaveInstallScope
  ${NSD_GetState} $MGSMachineRadio $0
  ${If} $MGSMachineScope != 1
    StrCpy $MGSUserInstallDirChoice $INSTDIR
  ${EndIf}
  StrCpy $MGSMachineScope 0
  ${If} $0 = ${BST_CHECKED}
    StrCpy $MGSMachineScope 1
  ${EndIf}
  Call MGS_ApplyScope
FunctionEnd

Function MGS_RequestMachineInstallAccess
  ; Keep the lock in this synchronous wrapper. The authenticated child must
  ; not wait on the lock that this original-user instance already owns.
  ${If} $MGSMachineScope != 1
  ${OrIf} $MGSInnerInstaller = 1
    Return
  ${EndIf}
  ${If} ${UAC_IsAdmin}
    Return
  ${EndIf}
  Call MGS_ReserveInstallationDirectories
  ${If} $R0 != 0
    StrCpy $R7 "另一个 MaxGameStudio 或英雄联盟工具箱安装程序正在运行。请等待它完成后重试。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  Call MGS_CheckProgramAndDataDirectories
  ${If} $R0 != 0
    StrCpy $R7 "安装位置无法验证，或与已有的程序、数据目录重合。请选择彼此分开的专用文件夹后重试。"
    Call CS2_AbortMigrationInstall
  ${EndIf}
  ShowWindow $HWNDPARENT ${SW_HIDE}
  !insertmacro UAC_RunElevated
  ${If} $0 = 0
  ${AndIf} $1 = 1
    SetErrorLevel $2
    Quit
  ${EndIf}
  ShowWindow $HWNDPARENT ${SW_SHOW}
  BringToFront
  ${If} $0 = 1223
    StrCpy $R7 "已取消管理员授权。可以返回并选择仅为当前用户安装。"
  ${ElseIf} $0 = 0
    StrCpy $R7 "此安装范围需要管理员账户。请重试或选择仅为当前用户安装。"
  ${Else}
    StrCpy $R7 "无法获得安装所需的管理员权限（错误 $0）。请重试或选择仅为当前用户安装。"
  ${EndIf}
  Call CS2_AbortMigrationInstall
FunctionEnd

Function MGS_FinalizeOriginalUser
  SetShellVarContext current
  ; An already-elevated setup has no original medium-integrity user.
  ; Defer personal settings and never execute a per-user uninstaller here.
  ${If} ${UAC_IsAdmin}
    StrCmp $UpdateMode 1 mgs_retire_user_done
    StrCmp $MGSUserProgramDir "" mgs_retire_user_done
    Goto mgs_retire_user_deferred
  ${EndIf}
  Call MGS_RecordDataDirectorySelection
  StrCmp $UpdateMode 1 mgs_retire_user_done
  StrCmp $MGSUserProgramDir "" mgs_retire_user_done
  StrCmp $MGSUserProgramDir $MGSMachineTarget mgs_retire_user_done
  ReadRegStr $0 HKCU "${UNINSTKEY}" "UninstallString"
  StrCmp $0 '$\"$MGSUserProgramDir\uninstall.exe$\"' 0 mgs_retire_user_deferred
  IfFileExists "$MGSUserProgramDir\uninstall.exe" 0 mgs_retire_user_deferred
  SetOutPath "$WINDIR"
  ; _?= stops NSIS from spawning a detached temporary copy. The uninstaller
  ; cannot delete its own running image in this mode, so remove it afterward.
  ClearErrors
  ExecWait '"$MGSUserProgramDir\uninstall.exe" /S _?=$MGSUserProgramDir' $0
  IfErrors mgs_retire_user_deferred
  StrCmp $0 0 0 mgs_retire_user_deferred
  IfFileExists "$MGSUserProgramDir\${MAINBINARYNAME}.exe" mgs_retire_user_deferred 0
  Delete "$MGSUserProgramDir\uninstall.exe"
  RMDir "$MGSUserProgramDir"
  ReadRegStr $0 HKCU "${MANUPRODUCTKEY}" ""
  StrCmp $0 $MGSUserProgramDir 0 mgs_retire_user_done
  DeleteRegValue HKCU "${MANUPRODUCTKEY}" ""
  DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
  Goto mgs_retire_user_done

  mgs_retire_user_deferred:
    MessageBox MB_ICONEXCLAMATION|MB_OK "所有用户版本已安装。原来仅为当前用户安装的程序仍保留，可在 Windows 设置的已安装应用中单独卸载。原数据位置已保留。" /SD IDOK
  mgs_retire_user_done:
FunctionEnd

; Tasklist in the administrator process sees the shared executable in other
; sessions. A running instance or a failed probe stops the machine change.
!macro MGS_DefineMachineAppCheck prefix
Function ${prefix}MGS_CheckMachineAppClosed
  SetOutPath "$WINDIR"
  nsExec::ExecToStack /TIMEOUT=15000 '"$SYSDIR\tasklist.exe" /FI "IMAGENAME eq ${MAINBINARYNAME}.exe" /FO CSV /NH'
  Pop $0
  Pop $1
  StrCpy $R0 2
  StrCmp $0 0 0 mgs_machine_app_check_done
  StrCpy $2 '"${MAINBINARYNAME}.exe"'
  System::Call 'shlwapi::StrStrIW(w r1, w r2) p.r3'
  StrCmp $3 0 0 mgs_machine_app_check_done
  StrCpy $R0 0
  mgs_machine_app_check_done:
FunctionEnd
!macroend
!insertmacro MGS_DefineMachineAppCheck ""
!insertmacro MGS_DefineMachineAppCheck "un."

Function un.MGS_InitUninstallScope
  SetShellVarContext current
  StrCpy $MGSMachineTarget "$PROGRAMFILES64\${PRODUCTNAME}"
  StrCmp $INSTDIR $MGSMachineTarget 0 mgs_uninstall_scope_done
  ReadRegStr $0 HKLM "${UNINSTKEY}" "InstallLocation"
  StrCmp $0 '$\"$MGSMachineTarget$\"' 0 mgs_uninstall_scope_invalid
  ReadRegDWORD $0 HKLM "${UNINSTKEY}" "AllUsers"
  StrCmp $0 1 0 mgs_uninstall_scope_invalid
  StrCpy $MGSMachineScope 1
  ${IfNot} ${UAC_IsAdmin}
    ; NSIS normally runs un.onInit from a temporary copy. Request elevation
    ; from the registered Program Files original rather than that temp EXE.
    StrCpy $R7 ""
    IfSilent 0 +2
      StrCpy $R7 "/S"
    ClearErrors
    ${GetOptions} $CMDLINE "/P" $0
    IfErrors +2 0
      StrCpy $R7 "$R7 /P"
    SetOutPath "$WINDIR"
    ClearErrors
    ExecShell "runas" "$MGSMachineTarget\uninstall.exe" "$R7"
    ${If} ${Errors}
      MessageBox MB_ICONEXCLAMATION|MB_OK "未获得管理员权限，已取消为所有用户卸载。" /SD IDOK
      SetErrorLevel 2
    ${EndIf}
    Quit
  ${EndIf}
  Call un.MGS_ReserveInstallationDirectories
  ${If} $R0 != 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "另一个 MaxGameStudio 或英雄联盟工具箱安装程序正在运行。请等待它完成后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetShellVarContext all
  Goto mgs_uninstall_scope_done
  mgs_uninstall_scope_invalid:
    MessageBox MB_ICONSTOP|MB_OK "无法确认注册的机器安装位置，已停止卸载。" /SD IDOK
    SetErrorLevel 2
    Abort
  mgs_uninstall_scope_done:
FunctionEnd
