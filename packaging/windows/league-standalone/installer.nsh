!include "nsDialogs.nsh"
!include "LogicLib.nsh"

!ifndef BUILD_UNINSTALLER
!define MGS_LEAGUE_GUARD "${__FILEDIR__}\destination-guard.ps1"
!define MGS_LEAGUE_RETIRE "${__FILEDIR__}\retire-user-install.ps1"
!define MGS_LEAGUE_INVENTORY "${__FILEDIR__}\maxgamestudio-runtime-hashes.json"
Var LeagueDataPath
Var LeagueDataEdit
Var LeagueProgramPath
Var LeagueProgramEdit
Var LeagueMutex
Var LeagueDeferUserData

!macro customPageAfterChangeDir
  Page custom LeagueDataPage LeagueDataLeave
!macroend

!macro customInit
  ${If} ${UAC_IsInnerInstance}
    ; The outer process requested machine scope. The credential account may also have HKCU installs.
    !insertmacro setInstallModePerAllUsers
    StrCpy $hasPerMachineInstallation "1"
    StrCpy $hasPerUserInstallation "0"
  ${Else}
    System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Global\MaxGameStudio.InstallDirectoryOwnership") p.r0'
    StrCpy $LeagueMutex $0
    StrCpy $1 4294967295
    ${If} $LeagueMutex P<> 0
      System::Call 'kernel32::WaitForSingleObject(p r0, i 0) i.r1'
    ${EndIf}
    ${If} $1 != 0
    ${AndIf} $1 != 128
      MessageBox MB_ICONEXCLAMATION "另一个 MaxGameStudio 安装向导正在运行。" /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
  ${If} ${Silent}
    StrCpy $LeagueProgramPath $INSTDIR
    StrCpy $R0 "selection"
  ${Else}
    StrCpy $R0 "registrations"
  ${EndIf}
  Call LeagueRunForUser
  ${If} $R7 == 3
    MessageBox MB_ICONEXCLAMATION "请先关闭英雄联盟工具箱，再继续安装。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} $R7 != 0
    MessageBox MB_ICONEXCLAMATION "无法验证工具箱安装目录。请选择独立的空目录，并确认旧版安装有效。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} ${Silent}
  ${AndIf} $installMode == "all"
    Call LeagueCheckMachineDestination
    ${If} $R7 != 0
      MessageBox MB_ICONEXCLAMATION "所有用户版的程序必须安装到受保护的 Program Files 专用文件夹。" /SD IDOK
      SetErrorLevel 2
      Abort
    ${EndIf}
  ${EndIf}
!macroend

; Directory validation already checks for running copies. Never kill another account's app.
!macro customCheckAppRunning
!macroend

; The custom include is prepended before electron-builder loads MUI2.
!macro customHeader
Function LeagueRunForUser
  StrCpy $R1 $LeagueProgramPath
  StrCpy $R2 $LeagueDataPath
  StrCpy $R3 $installMode
  StrCpy $R7 "error"
  ${If} ${UAC_IsInnerInstance}
    !insertmacro UAC_AsUser_Call Function LeagueUserAction ${UAC_SYNCREGISTERS}
  ${Else}
    Call LeagueUserAction
  ${EndIf}
FunctionEnd

Function LeagueCheckMachineDestination
  ; Validate Program Files and HKLM independently in the process that writes machine files.
  StrCpy $R0 "machine"
  StrCpy $R1 $LeagueProgramPath
  StrCpy $R2 ""
  StrCpy $R3 "all"
  Call LeagueUserAction
FunctionEnd

Function LeagueUserAction
  ; Private data phases use the original user's token. The machine phase reads only HKLM.
  ${If} $R0 == "default"
    SetShellVarContext current
    StrCpy $R4 "$APPDATA\MaxGameStudioLeague\Profile"
    ClearErrors
    FileOpen $0 "$APPDATA\MaxGameStudioLeague\data-location.txt" r
    ${IfNot} ${Errors}
      FileReadUTF16LE $0 $R4
      FileClose $0
    ${EndIf}
    ${If} $installMode == "all"
      SetShellVarContext all
    ${EndIf}
    StrCpy $R7 0
    Return
  ${EndIf}

  InitPluginsDir
  File "/oname=$PLUGINSDIR\mgs-league-destination.ps1" "${MGS_LEAGUE_GUARD}"
  File "/oname=$PLUGINSDIR\retire-user-install.ps1" "${MGS_LEAGUE_RETIRE}"
  File "/oname=$PLUGINSDIR\maxgamestudio-runtime-hashes.json" "${MGS_LEAGUE_INVENTORY}"
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_INSTALL_PHASE", w "$R0") i.r0'
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_INSTALL_DEST", w "$R1") i.r0'
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_DATA_DEST", w "$R2") i.r0'
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_INSTALL_SCOPE", w "$R3") i.r0'
  StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$0" +2 0
  StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::Exec '"$0" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\mgs-league-destination.ps1"'
  Pop $R7
FunctionEnd

Function LeagueDataPage
  StrCpy $LeagueProgramPath $INSTDIR
  StrCpy $LeagueDeferUserData "0"
  ${If} $installMode == "all"
  ${AndIfNot} ${UAC_IsInnerInstance}
  ${AndIf} ${UAC_IsAdmin}
    StrCpy $LeagueDeferUserData "1"
    StrCpy $LeagueDataPath ""
  ${ElseIf} $LeagueDataPath == ""
    StrCpy $R0 "default"
    Call LeagueRunForUser
    ${If} $R7 != 0
    ${OrIf} $R4 == ""
      MessageBox MB_ICONEXCLAMATION "无法读取启动安装的 Windows 用户的数据目录。"
      SetErrorLevel 2
      Quit
    ${EndIf}
    StrCpy $LeagueDataPath $R4
  ${EndIf}
  !insertmacro MUI_HEADER_TEXT "选择工具箱安装和数据目录" "程序与数据使用不同的目录，卸载时保留数据。"
  nsDialogs::Create 1018
  Pop $0
  ${if} $0 == error
    SetErrorLevel 2
    Quit
  ${endif}
  ${If} $installMode == "all"
    StrCpy $1 "所有用户共用 Program Files 中的程序。每位用户的数据独立保存。"
  ${Else}
    StrCpy $1 "程序安装目录。全新安装请使用空文件夹。"
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 22u "$1"
  Pop $0
  ${NSD_CreateDirRequest} 0 22u 76% 13u "$LeagueProgramPath"
  Pop $LeagueProgramEdit
  ${NSD_CreateBrowseButton} 79% 21u 21% 15u "浏览..."
  Pop $0
  ${NSD_OnClick} $0 LeagueProgramBrowse
  ${If} $installMode == "all"
    EnableWindow $LeagueProgramEdit 0
    EnableWindow $0 0
  ${EndIf}
  ${If} $LeagueDeferUserData == "1"
    ${NSD_CreateLabel} 0 48u 100% 40u "当前管理员会话仅安装共用程序。请使用自己的 Windows 帐户正常启动，工具箱会继续使用该帐户的数据。"
    Pop $0
  ${Else}
    ${NSD_CreateLabel} 0 48u 100% 24u "工具箱数据目录。首次启动会迁移旧版设置和数据库，原数据仍保留。"
    Pop $0
    ${NSD_CreateDirRequest} 0 73u 76% 13u "$LeagueDataPath"
    Pop $LeagueDataEdit
    ${NSD_CreateBrowseButton} 79% 72u 21% 15u "浏览..."
    Pop $0
    ${NSD_OnClick} $0 LeagueDataBrowse
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function LeagueProgramBrowse
  Pop $0
  ${NSD_GetText} $LeagueProgramEdit $LeagueProgramPath
  nsDialogs::SelectFolderDialog "选择工具箱程序文件夹" "$LeagueProgramPath"
  Pop $0
  ${If} $0 != error
    ${NSD_SetText} $LeagueProgramEdit $0
  ${EndIf}
FunctionEnd

Function LeagueDataBrowse
  Pop $0
  ${NSD_GetText} $LeagueDataEdit $LeagueDataPath
  nsDialogs::SelectFolderDialog "选择工具箱数据文件夹" "$LeagueDataPath"
  Pop $0
  ${if} $0 != error
    ${NSD_SetText} $LeagueDataEdit $0
  ${endif}
FunctionEnd

Function LeagueDataLeave
  ${NSD_GetText} $LeagueProgramEdit $LeagueProgramPath
  ${If} $LeagueDeferUserData == "0"
    ${NSD_GetText} $LeagueDataEdit $LeagueDataPath
    ${If} $LeagueDataPath == ""
      MessageBox MB_ICONEXCLAMATION "请选择工具箱的数据文件夹。"
      Abort
    ${EndIf}
  ${EndIf}
  StrCpy $R0 "selection"
  Call LeagueRunForUser
  ${If} $R7 == 3
    MessageBox MB_ICONEXCLAMATION "请先关闭英雄联盟工具箱，再继续安装。"
    Abort
  ${EndIf}
  ${If} $R7 != 0
    MessageBox MB_ICONEXCLAMATION "请选择本机磁盘上的独立文件夹。"
    Abort
  ${EndIf}
  ${If} $installMode == "all"
    Call LeagueCheckMachineDestination
    ${If} $R7 != 0
      MessageBox MB_ICONEXCLAMATION "所有用户版的程序必须安装到受保护的 Program Files 专用文件夹。"
      Abort
    ${EndIf}
  ${EndIf}
  GetFullPathName $INSTDIR "$LeagueProgramPath"
FunctionEnd
!macroend

!macro customInstall
  StrCpy $LeagueProgramPath $INSTDIR
  StrCpy $R0 "complete"
  Call LeagueRunForUser
  ${If} $R7 == 4
    DetailPrint "所有用户版已安装。管理员会话未修改个人数据设置或移除旧个人版。"
    MessageBox MB_ICONINFORMATION "所有用户版已安装。个人数据设置及已有个人版均保留。请使用自己的 Windows 帐户正常启动。" /SD IDOK
  ${ElseIf} $R7 != 0
    MessageBox MB_ICONEXCLAMATION "工具箱已写入，无法完成启动用户的数据设置或个人版清理。请检查该用户的数据目录权限后重试。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend
!endif
