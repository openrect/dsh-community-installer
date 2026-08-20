; The runtime seed changes what is installed, not the installation experience.
; Both editions silently place the controller before it opens the Tauri setup UI.
!include FileFunc.nsh
SilentInstall silent

!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  StrCmp $0 "" 0 webview_found
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  StrCmp $0 "" webview_missing webview_found
  webview_found:
    StrCmp $0 "0.0.0.0" webview_missing webview_ready
  webview_missing:
    MessageBox MB_ICONSTOP|MB_OK "Microsoft Edge WebView2 Runtime is required. Please use the online installer.$\r$\n$\r$\n此电脑缺少 WebView2，请改用联网安装版。"
    Abort
  webview_ready:
!macroend

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
