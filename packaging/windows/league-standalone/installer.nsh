!include "nsDialogs.nsh"
!include "LogicLib.nsh"

!ifndef BUILD_UNINSTALLER
!define MGS_LEAGUE_GUARD "${__FILEDIR__}\destination-guard.ps1"
Var LeagueDataPath
Var LeagueDataEdit
Var LeagueProgramPath
Var LeagueProgramEdit
Var LeagueMutex

!macro customPageAfterChangeDir
  Page custom LeagueDataPage LeagueDataLeave
!macroend

!macro customInit
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
  ${If} ${Silent}
    StrCpy $LeagueProgramPath $INSTDIR
    StrCpy $3 "selection"
  ${Else}
    StrCpy $3 "registrations"
  ${EndIf}
  Call LeagueCheckDestination
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "无法验证工具箱安装目录。请选择独立的空目录，并确认旧版安装有效。" /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
!macroend

; The custom include is prepended before electron-builder loads MUI2.
!macro customHeader
Function LeagueCheckDestination
  InitPluginsDir
  File "/oname=$PLUGINSDIR\mgs-league-destination.ps1" "${MGS_LEAGUE_GUARD}"
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_INSTALL_PHASE", w r3) i.r0'
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_INSTALL_DEST", w "$LeagueProgramPath") i.r0'
  System::Call 'kernel32::SetEnvironmentVariableW(w "MGS_LEAGUE_DATA_DEST", w "$LeagueDataPath") i.r0'
  StrCpy $0 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$0" +2 0
  StrCpy $0 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  nsExec::Exec '"$0" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\mgs-league-destination.ps1"'
  Pop $0
FunctionEnd

Function LeagueDataPage
  ${If} $LeagueProgramPath == ""
    StrCpy $LeagueProgramPath $INSTDIR
  ${EndIf}
  ${if} $LeagueDataPath == ""
    ClearErrors
    FileOpen $0 "$APPDATA\MaxGameStudioLeague\data-location.txt" r
    ${if} ${Errors}
      StrCpy $LeagueDataPath "$APPDATA\MaxGameStudioLeague\Profile"
    ${else}
      FileReadUTF16LE $0 $LeagueDataPath
      FileClose $0
    ${endif}
  ${endif}
  !insertmacro MUI_HEADER_TEXT "选择工具箱安装和数据目录" "程序与数据使用不同的目录，卸载时保留数据。"
  nsDialogs::Create 1018
  Pop $0
  ${if} $0 == error
    SetErrorLevel 2
    Quit
  ${endif}
  ${NSD_CreateLabel} 0 0 100% 22u "程序安装目录。全新安装请使用空文件夹。"
  Pop $0
  ${NSD_CreateDirRequest} 0 22u 76% 13u "$LeagueProgramPath"
  Pop $LeagueProgramEdit
  ${NSD_CreateBrowseButton} 79% 21u 21% 15u "浏览..."
  Pop $0
  ${NSD_OnClick} $0 LeagueProgramBrowse
  ${NSD_CreateLabel} 0 48u 100% 24u "工具箱数据目录。首次启动会迁移旧版设置和数据库，原数据仍保留。"
  Pop $0
  ${NSD_CreateDirRequest} 0 73u 76% 13u "$LeagueDataPath"
  Pop $LeagueDataEdit
  ${NSD_CreateBrowseButton} 79% 72u 21% 15u "浏览..."
  Pop $0
  ${NSD_OnClick} $0 LeagueDataBrowse
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
  ${NSD_GetText} $LeagueDataEdit $LeagueDataPath
  StrCpy $3 "selection"
  Call LeagueCheckDestination
  ${If} $0 != 0
  ${OrIf} $LeagueDataPath == ""
    MessageBox MB_ICONEXCLAMATION "请选择本机磁盘上的独立文件夹。"
    Abort
  ${EndIf}
  GetFullPathName $INSTDIR "$LeagueProgramPath"
FunctionEnd
!macroend

!macro customInstall
  ${if} $LeagueDataPath != ""
    CreateDirectory "$APPDATA\MaxGameStudioLeague"
    ClearErrors
    FileOpen $0 "$APPDATA\MaxGameStudioLeague\installer-data-location.txt" w
    ${if} ${Errors}
      MessageBox MB_ICONEXCLAMATION "无法保存数据目录选择。请检查磁盘权限后重新运行安装向导。"
      Abort
    ${endif}
    FileWriteUTF16LE /BOM $0 "$LeagueDataPath"
    FileClose $0
  ${endif}
!macroend
!endif
