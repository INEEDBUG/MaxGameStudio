; Shortcut property and Jump List helpers adapted from Tauri CLI v2.11.4, under the MIT License.
; https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/utils.nsh
; Copyright (c) 2017 - Present Tauri Apps Contributors. See native-shell-links.LICENSE.txt.
!ifndef MGS_LEAGUE_NATIVE_SHELL_LINKS
!define MGS_LEAGUE_NATIVE_SHELL_LINKS
!include Integration.nsh
!include LogicLib.nsh
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"

; Edit the link electron-builder just created or renamed, retaining its target and other properties.
!macro MgsLeagueSetLnkAUMI shortcut appId
  System::Store "S"
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}",.r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${shortcut}", ${STGM_READWRITE})i.r6'
      ${If} $6 >= 0
        ${IUnknown::QueryInterface} $0 '("${IID_IPropertyStore}",.r2)'
        ${If} $2 P<> 0
          System::Call 'shlwapi::SHStrDupW(w "${appId}", *p.r3)i.r6'
          ${If} $6 >= 0
            System::Call '*${SYSSTRUCT_PROPERTYKEY}(${PKEY_AppUserModel_ID})p.r4'
            System::Alloc ${SYSSIZEOF_PROPVARIANT}
            Pop $5
            ${V_SetPointer} $5 ${VT_LPWSTR} r3
            ${IPropertyStore::SetValue} $2 '($4,$5)i.r6'
            ${If} $6 >= 0
              ${IPropertyStore::Commit} $2 '()i.r6'
            ${EndIf}
            !insertmacro PropVariantClear $5
            System::Free $5
            System::Free $4
          ${EndIf}
          ${IUnknown::Release} $2 ""
          ${If} $6 >= 0
            ${IPersistFile::Save} $1 '("${shortcut}",1)'
          ${EndIf}
        ${EndIf}
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
  System::Store "L"
!macroend

!macro MgsLeagueUninstShortcut shortcut
  ${UnpinShortcut} "${shortcut}"
!macroend

!macro MgsLeagueUninstAppUserModelId appId
  System::Store "S"
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_DestinationList} ${IID_ICustomDestinationList} r1 ""
  ${If} $1 P<> 0
    ${ICustomDestinationList::DeleteList} $1 '("${appId}")'
    ${IUnknown::Release} $1 ""
  ${EndIf}
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ApplicationDestinations} ${IID_IApplicationDestinations} r1 ""
  ${If} $1 P<> 0
    ${IApplicationDestinations::SetAppID} $1 '("${appId}")i.r0'
    ${If} $0 >= 0
      ${IApplicationDestinations::RemoveAllDestinations} $1 ""
    ${EndIf}
    ${IUnknown::Release} $1 ""
  ${EndIf}
  System::Store "L"
!macroend
!endif
