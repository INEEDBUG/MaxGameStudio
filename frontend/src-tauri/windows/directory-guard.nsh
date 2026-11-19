!define MGS_DIRECTORY_CHECK_SCRIPT "${__FILEDIR__}\validate-install-directories.ps1"
Var MGSDirectoryMutex
Var MGSVerifiedProgramDir

!macro MGS_DefineInstallDirectoryLock prefix
Function ${prefix}MGS_ReserveInstallationDirectories
  StrCmp $MGSDirectoryMutex "" 0 mgs_directory_mutex_owned
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Global\MaxGameStudio.InstallDirectoryOwnership") p.s'
  Pop $MGSDirectoryMutex
  StrCmp $MGSDirectoryMutex 0 mgs_directory_mutex_busy
  System::Call 'kernel32::WaitForSingleObject(p $MGSDirectoryMutex, i 0) i.r0'
  StrCmp $0 "0" mgs_directory_mutex_owned
  StrCmp $0 "128" mgs_directory_mutex_owned
  mgs_directory_mutex_busy:
    StrCmp $MGSDirectoryMutex 0 +2
      System::Call 'kernel32::CloseHandle(p $MGSDirectoryMutex)'
    StrCpy $MGSDirectoryMutex ""
    StrCpy $R0 "2"
    Return
  mgs_directory_mutex_owned:
    ; The installer owns this mutex until Windows closes its process.
    StrCpy $R0 "0"
FunctionEnd
!macroend

!insertmacro MGS_DefineInstallDirectoryLock ""
!ifdef MGS_DUAL_SCOPE
  !insertmacro MGS_DefineInstallDirectoryLock "un."
!endif

Function MGS_CheckProgramAndDataDirectories
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=validate-install-directories.ps1 "${MGS_DIRECTORY_CHECK_SCRIPT}"
  StrCpy $R5 ""
  StrCmp $MGSSelectedDataDir "" +2
    StrCpy $R5 '-DataDir "$MGSSelectedDataDir\."'
  !ifdef MGS_DUAL_SCOPE
    ${If} $MGSMachineScope = 1
      StrCpy $R5 "$R5 -MachineScope"
      ${If} $MGSInnerInstaller = 1
        StrCpy $R5 "$R5 -MachineFilesOnly"
      ${EndIf}
    ${EndIf}
  !endif
  Call MGS_FindWindowsPowerShell
  ; The mutex loads the native System.dll into $PLUGINSDIR. Leave that working
  ; directory so PowerShell Add-Type resolves the .NET framework assembly.
  SetOutPath "$WINDIR"
  nsExec::ExecToStack /TIMEOUT=20000 '"$R6" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\validate-install-directories.ps1" -ProgramDir "$INSTDIR\." $R5'
  Pop $R0
  Pop $R1
FunctionEnd

Function MGS_FindWindowsPowerShell
  StrCpy $R6 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$R6" +2
    StrCpy $R6 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
FunctionEnd
