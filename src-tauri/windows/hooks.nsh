; NSIS hooks for the Readaity installer.

!macro NSIS_HOOK_POSTINSTALL
  ; An "Uninstall readaity" entry next to the app shortcut, like the other aity apps have. The folder is the one the
  ; wizard chose, or aity when the wizard page was skipped (silent installs).
  StrCpy $R9 "aity"
  ${If} $AppStartMenuFolder != ""
    StrCpy $R9 "$AppStartMenuFolder"
  ${EndIf}
  CreateDirectory "$SMPROGRAMS\$R9"
  CreateShortcut "$SMPROGRAMS\$R9\Uninstall ${PRODUCTNAME}.lnk" "$INSTDIR\uninstall.exe"
!macroend

; Remove that shortcut again on uninstall (RMDir only removes the aity folder if it is empty, so the other aity
; apps' shortcuts keep it alive).
!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$SMPROGRAMS\aity\Uninstall ${PRODUCTNAME}.lnk"
  ${If} $AppStartMenuFolder != ""
    Delete "$SMPROGRAMS\$AppStartMenuFolder\Uninstall ${PRODUCTNAME}.lnk"
  ${EndIf}
  RMDir "$SMPROGRAMS\aity"
!macroend
