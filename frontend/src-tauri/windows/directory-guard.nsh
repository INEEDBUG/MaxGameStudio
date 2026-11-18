!define MGS_DIRECTORY_CHECK_SCRIPT "${__FILEDIR__}\validate-install-directories.ps1"
Var MGSDirectoryMutex
Var MGSVerifiedProgramDir

Function MGS_ReserveInstallationDirectories
  System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Global\MaxGameStudio.InstallDirectoryOwnership") p.s'
  Pop $MGSDirectoryMutex
  StrCmp $MGSDirectoryMutex 0 mgs_directory_mutex_busy
  System::Call 'kernel32::WaitForSingleObject(p $MGSDirectoryMutex, i 0) i.r0'
  StrCmp $0 "0" mgs_directory_mutex_owned
  StrCmp $0 "128" mgs_directory_mutex_owned
  mgs_directory_mutex_busy:
    StrCpy $R0 "2"
    Return
  mgs_directory_mutex_owned:
    ; The installer owns this mutex until Windows closes its process.
    StrCpy $R0 "0"
FunctionEnd

Function MGS_CheckProgramAndDataDirectories
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=validate-install-directories.ps1 "${MGS_DIRECTORY_CHECK_SCRIPT}"
  StrCpy $R5 ""
  StrCmp $MGSSelectedDataDir "" +2
    StrCpy $R5 '-DataDir "$MGSSelectedDataDir\."'
  StrCpy $R6 "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
  IfFileExists "$R6" +2
    StrCpy $R6 "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe"
  ; The mutex loads the native System.dll into $PLUGINSDIR. Leave that working
  ; directory so PowerShell Add-Type resolves the .NET framework assembly.
  SetOutPath "$WINDIR"
  nsExec::ExecToStack /TIMEOUT=20000 '"$R6" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\validate-install-directories.ps1" -ProgramDir "$INSTDIR\." $R5'
  Pop $R0
  Pop $R1
FunctionEnd
