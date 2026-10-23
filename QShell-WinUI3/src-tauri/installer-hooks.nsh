; QShell NSIS hooks
;
; The stock Tauri uninstaller removes "%APPDATA%\\<bundle-id>" and
; "%LOCALAPPDATA%\\<bundle-id>". QShell stores its config in
; "%APPDATA%\\QShell" and installs under "%LOCALAPPDATA%\\QShell", so those
; directories need explicit cleanup.

!macro NSIS_HOOK_PREUNINSTALL
  ; The uninstaller cannot replace a running executable. Ask it to exit first;
  ; QShell's single-instance mutex prevents a second copy from starting.
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM QShell.exe'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  SetShellVarContext current

  ; Always remove the application directory. It contains QShell.exe,
  ; uninstall.exe and the notification icon, and may not be empty after the
  ; stock uninstaller has deleted the two executables.
  RmDir /r "$INSTDIR"

  ; Only remove user data when the user checked "Delete app data".
  ${If} $DeleteAppDataCheckboxState = 1
    RmDir /r "$APPDATA\QShell"
    RmDir /r "$LOCALAPPDATA\QShell"

    ; Legacy QShell versions wrote their config here.
    Delete "$LOCALAPPDATA\config.json"
  ${EndIf}
!macroend
