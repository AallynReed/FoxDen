; Installer for the native FoxDen build. Same folder, uninstall entry and hooks as the earlier installer.
; makensis /DVERSION=x.y.z /DAPP=<foxden-native.exe> /DSERVICE=<foxden-service.exe> /DOUT=<setup.exe> packaging\native.nsi

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel admin

!include MUI2.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include x64.nsh

Name "FoxDen"
OutFile "${OUT}"
InstallDir "$PROGRAMFILES64\FoxDen"
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "FoxDen"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "FoxDen"
VIAddVersionKey "FileDescription" "FoxDen setup"
VIAddVersionKey "LegalCopyright" "GPL-2.0-or-later"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\FoxDen"
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\foxden.exe"

Var UpdateMode
Var Passive

!include "installer-hooks.nsh"

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetRegView 64
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${Else}
    StrCpy $UpdateMode 0
  ${EndIf}
  ClearErrors
  ${GetOptions} $0 "/P" $1
  ${IfNot} ${Errors}
    StrCpy $Passive 1
  ${Else}
    StrCpy $Passive 0
  ${EndIf}
  ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
  ${If} $0 != ""
    StrCpy $1 $0 1
    ${If} $1 == '"'
      StrCpy $0 $0 -1 1
    ${EndIf}
    StrCpy $INSTDIR $0
  ${EndIf}
FunctionEnd

Function SkipIfPassive
  ${If} $Passive = 1
    Abort
  ${EndIf}
FunctionEnd

; Explorer starts the app as the signed-in user, not elevated like the installer.
Function Launch
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\foxden.exe"'
FunctionEnd

Function .onInstSuccess
  ${If} $UpdateMode = 1
    Call Launch
  ${EndIf}
FunctionEnd

Function un.onInit
  SetRegView 64
  StrCpy $UpdateMode 0
FunctionEnd

Section "FoxDen" Main
  SetShellVarContext all
  !insertmacro NSIS_HOOK_PREINSTALL
  SetOutPath "$INSTDIR"
  File "/oname=foxden.exe" "${APP}"
  File "/oname=foxden-service.exe" "${SERVICE}"
  File "/oname=PawnIO_setup.exe" "${PAWNIO}"
  File "/oname=LICENSE.txt" "..\LICENSE"
  File "/oname=NOTICE.txt" "..\NOTICE"
  File "/oname=THIRD_PARTY_NOTICES.txt" "..\THIRD_PARTY_NOTICES.txt"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "FoxDen"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" '"$INSTDIR\foxden.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "FoxDen"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" '"$INSTDIR"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "MainBinaryName" "foxden.exe"
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0

  CreateShortcut "$SMPROGRAMS\FoxDen.lnk" "$INSTDIR\foxden.exe"
  !insertmacro NSIS_HOOK_POSTINSTALL
SectionEnd

Section "Uninstall"
  SetShellVarContext all
  !insertmacro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /F /T /IM foxden.exe'
  Delete "$INSTDIR\foxden.exe"
  Delete "$INSTDIR\foxden-service.exe"
  Delete "$INSTDIR\PawnIO_setup.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\NOTICE.txt"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\FoxDen.lnk"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
