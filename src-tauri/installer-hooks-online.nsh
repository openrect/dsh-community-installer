; The Tauri controller owns the visible setup experience. NSIS only places the
; small controller binary and immediately launches its custom setup window.
!include FileFunc.nsh
SilentInstall silent

!macro NSIS_HOOK_POSTINSTALL
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$DESKTOP\DSH Community.lnk"
  CreateShortCut "$INSTDIR\DSH Community.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  ${GetParameters} $0
  ${GetOptions} "$0" "/S" $1
  IfErrors launch_application postinstall_done
  launch_application:
  Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
  postinstall_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --shutdown-for-maintenance'
  Sleep 600
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\${PRODUCTNAME}.lnk"
  Delete "$DESKTOP\DSH Community.lnk"
  RMDir /r "$LOCALAPPDATA\DSHCommunityInstaller"
!macroend
