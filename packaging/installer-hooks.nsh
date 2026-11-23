; Sets up PawnIO and the FoxDen service so memory lighting works without running as admin.

!include LogicLib.nsh

; Removes the app installed under its old name, FoxRGB. Its settings stay and are copied over on first start.
!macro REMOVE_FOXRGB
  ReadRegStr $0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\FoxRGB" "InstallLocation"
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 -1 1
  ${EndIf}
  ${If} $0 != ""
    GetFullPathName $0 $0
  ${EndIf}
  ; Only a folder named FoxRGB under Program Files is ever removed.
  StrCpy $3 0
  StrLen $2 "$PROGRAMFILES64\"
  StrCpy $1 $0 $2
  ${If} $1 == "$PROGRAMFILES64\"
    StrCpy $3 1
  ${EndIf}
  StrLen $2 "$PROGRAMFILES\"
  StrCpy $1 $0 $2
  ${If} $1 == "$PROGRAMFILES\"
    StrCpy $3 1
  ${EndIf}
  StrCpy $1 $0 "" -7
  ${If} $3 = 1
  ${AndIf} $1 == "\FoxRGB"
  ${AndIf} ${FileExists} "$0\uninstall.exe"
    ; Its own uninstaller stops and removes its service and sign-in task.
    ExecWait '"$0\uninstall.exe" /S _?=$0'
    Delete "$0\uninstall.exe"
    RMDir "$0"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro REMOVE_FOXRGB
  nsExec::Exec '"$SYSDIR\sc.exe" stop foxden-service'
  ; The service hands its fans back before exiting; one that never exits is closed so its files can be replaced.
  nsExec::Exec '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -Command "Wait-Process -Name foxden-service -Timeout 20 -ErrorAction SilentlyContinue"'
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM foxden-service.exe'
  ; An app still exiting after starting the update would make the running-app check fail.
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /T /IM foxden.exe'
  ; An AI app keeps this running, which would lock the file.
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM foxden-mcp.exe'
  Sleep 1500
!macroend

; Only SYSTEM and administrators can change the install folder, wherever it is.
!macro LOCK_INSTDIR
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR" /setowner *S-1-5-32-544 /T /C /Q'
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /Q'
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR\*" /reset /T /C /Q'
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
  nsExec::Exec '"$SYSDIR\schtasks.exe" /Delete /TN FoxDen /F'
!macroend
