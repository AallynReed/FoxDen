; Sets up PawnIO and the FoxRGB service so memory lighting works without running as admin.

!macro NSIS_HOOK_PREINSTALL
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" stop'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\PawnIO_setup.exe" -install -silent'
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" install'
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" start'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\foxrgb-service.exe" uninstall'
!macroend
