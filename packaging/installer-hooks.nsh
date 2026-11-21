; Sets up PawnIO and the FoxDen service so memory lighting works without running as admin.

!include LogicLib.nsh

; Removes the app installed under its old name, FoxRGB. Its settings stay and are copied over on first start.
!macro REMOVE_FOXRGB ROOT
  ReadRegStr $0 ${ROOT} "Software\Microsoft\Windows\CurrentVersion\Uninstall\FoxRGB" "InstallLocation"
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 -1 1
  ${EndIf}
  ; Only a folder named FoxRGB is ever removed.
  StrCpy $1 $0 "" -6
  ${If} $1 == "FoxRGB"
  ${AndIf} ${FileExists} "$0\uninstall.exe"
    nsExec::Exec 'taskkill /F /T /IM foxrgb.exe'
    nsExec::Exec '"$0\foxrgb-service.exe" stop'
    nsExec::Exec '"$0\foxrgb-service.exe" uninstall'
    nsExec::Exec 'schtasks /Delete /TN FoxRGB /F'
    ExecWait '"$0\uninstall.exe" /S _?=$0'
    Delete "$0\uninstall.exe"
    RMDir /r "$0"
    DeleteRegKey ${ROOT} "Software\Microsoft\Windows\CurrentVersion\Uninstall\FoxRGB"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro REMOVE_FOXRGB HKLM
  !insertmacro REMOVE_FOXRGB HKCU
  nsExec::Exec '"$INSTDIR\foxden-service.exe" stop'
  ; An app still exiting after starting the update would make the running-app check fail.
  nsExec::Exec 'taskkill /F /T /IM foxden.exe'
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\PawnIO_setup.exe" -install -silent'
  nsExec::Exec '"$INSTDIR\foxden-service.exe" install'
  nsExec::Exec '"$INSTDIR\foxden-service.exe" start'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Only a VB-CABLE that FoxDen installed is removed, and never for an update.
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\foxden-service.exe" vcable remove'
  ${EndIf}
  nsExec::Exec '"$INSTDIR\foxden-service.exe" uninstall'
  nsExec::Exec 'schtasks /Delete /TN FoxDen /F'
!macroend
