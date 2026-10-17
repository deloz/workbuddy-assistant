; NSIS installer hooks. Keep this file ASCII-only: makensis reads it with the
; installer's codepage, and ASCII avoids any encoding surprises.
;
; The app registers itself under the current user's Run key; see
; src/core/autostart.rs. Remove both the entry and its Task Manager override so
; an uninstall leaves no dangling startup item behind.

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "workbuddy-assistant"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "workbuddy-assistant"
!macroend
