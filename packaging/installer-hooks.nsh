; Sets up PawnIO and the FoxRGB service so memory lighting works without running as admin.

!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" stop'
  ; An app still exiting after starting the update would make the running-app check fail.
  nsExec::Exec 'taskkill /F /T /IM foxrgb.exe'
  Sleep 1500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\PawnIO_setup.exe" -install -silent'
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" install'
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" start'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" uninstall'
  nsExec::Exec 'schtasks /Delete /TN FoxRGB /F'
!macroend
