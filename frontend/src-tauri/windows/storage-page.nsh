; The template places this page after the standard program directory page.
; The selection remains in memory until every install step succeeds.
Var MGSSelectedDataDir
Var MGSDataDirField
Var MGSDataDirectoryConfirmed

Function MGS_ShowDataDirectory
  Call SkipIfPassive
  ClearErrors
  ${GetOptions} $CMDLINE "/UPDATE" $0
  IfErrors +2 0
    Abort

  !insertmacro MUI_HEADER_TEXT "数据存储位置" "选择配置、数据库、缓存和日志的存放位置"
  nsDialogs::Create 1018
  Pop $0
  StrCmp $0 "error" 0 +2
    Abort

  ${NSD_CreateLabel} 0 0 100% 28u "留空将沿用现有数据位置。首次安装时会自动选择，优先使用非系统盘的本地固定磁盘。"
  Pop $0
  ${NSD_CreateDirRequest} 0 32u 72% 13u "$MGSSelectedDataDir"
  Pop $MGSDataDirField
  ${NSD_CreateBrowseButton} 75% 32u 25% 14u "浏览(&B)…"
  Pop $0
  ${NSD_OnClick} $0 MGS_BrowseDataDirectory
  ${NSD_CreateLabel} 0 54u 100% 45u "如需更换位置，请选择专用的空文件夹。下次启动将先校验并复制已有数据，原目录会保留。数据量较大时，首次启动可能需要等待。"
  Pop $0

  nsDialogs::Show
FunctionEnd

Function MGS_BrowseDataDirectory
  Pop $0
  ${NSD_GetText} $MGSDataDirField $0
  nsDialogs::SelectFolderDialog "选择 MaxGameStudio 数据目录" "$0"
  Pop $0
  StrCmp $0 "error" +2
    ${NSD_SetText} $MGSDataDirField "$0"
FunctionEnd

Function MGS_LeaveDataDirectory
  ${NSD_GetText} $MGSDataDirField $MGSSelectedDataDir
  StrCmp $MGSSelectedDataDir "" mgs_data_dir_done

  ; Keep relative paths out of the drive picker. The Rust bootstrap also
  ; checks actual paths, directory ownership and destination writability.
  StrCpy $0 $MGSSelectedDataDir 2 1
  StrCmp $0 ":\" 0 mgs_data_dir_invalid
  ClearErrors
  GetFullPathName $MGSSelectedDataDir "$MGSSelectedDataDir"
  IfErrors mgs_data_dir_invalid
  StrLen $0 $MGSSelectedDataDir
  IntCmp $0 3 mgs_data_dir_invalid mgs_data_dir_invalid 0
  StrCpy $0 $MGSSelectedDataDir 3
  System::Call 'kernel32::GetDriveTypeW(w r0) i.r1'
  StrCmp $1 3 0 mgs_data_dir_invalid

  StrCpy $0 $WINDIR 2
  StrCpy $1 $MGSSelectedDataDir 2
  StrCmp $0 $1 0 mgs_data_dir_done
  MessageBox MB_ICONQUESTION|MB_YESNO|MB_DEFBUTTON2 "所选位置在系统盘上。确定把应用数据存放在系统盘吗？" IDYES mgs_data_dir_done
  Abort

  mgs_data_dir_invalid:
  MessageBox MB_ICONEXCLAMATION|MB_OK "请选择本地固定磁盘内的专用文件夹。"
  Abort

  mgs_data_dir_done:
  StrCpy $MGSDataDirectoryConfirmed "1"
FunctionEnd

Function MGS_RecordDataDirectorySelection
  StrCmp $MGSDataDirectoryConfirmed "1" 0 mgs_storage_record_done
  StrCmp $MGSSelectedDataDir "" 0 mgs_storage_record_custom
    DeleteRegValue HKCU "Software\MaxGameStudio\Storage" "InstallerDataRootV1"
    Goto mgs_storage_record_done
  mgs_storage_record_custom:
  System::Call 'kernel32::GetCurrentProcessId() i.r0'
  System::Call 'kernel32::GetTickCount64() l.r1'
  ClearErrors
  WriteRegStr HKCU "Software\MaxGameStudio\Storage" "InstallerDataRootV1" "$0-$1|$MGSSelectedDataDir"
  IfErrors 0 mgs_storage_record_done
  StrCpy $R7 "无法保存选择的数据目录。已保留原数据位置，请稍后重新运行安装程序。"
  Call CS2_AbortMigrationInstall
  mgs_storage_record_done:
FunctionEnd
