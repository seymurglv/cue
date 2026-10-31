; CUE's additions to the installer, included by installer.nsi.
;
; Start at sign-in is the value "CUE" under HKCU ...\CurrentVersion\Run, the one CUE's own switch writes (the autostart
; plugin names it after the product). A first install turns it on, and removes the "turned off" mark Windows may keep
; for that name under Explorer\StartupApproved\Run. A later install keeps the user's choice, and an update (/UPDATE)
; never touches the value. Installing over CUE leaves the value alone; but when the installer runs the uninstaller of
; the version already there first (an upgrade by hand), that uninstaller deletes the value. So an uninstall that went
; through notes whether the value was there, and the next install puts it back. An uninstall stopped on the way (CUE
; still running, Cancel) notes nothing.
;
; "Installed before" is the install folder the installer keeps in HKCU\${MANUPRODUCTKEY} (the publisher's key): the
; uninstaller leaves it there, unless its "Delete the application data" box is ticked; then it keeps no note either,
; and the next install is a first one again.

!define CUE_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define CUE_STARTUP_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
; In HKCU\${MANUPRODUCTKEY}: "1" when the uninstaller found the Run value; read and removed by the next install.
!define CUE_WAS_ON "StartAtSignIn"

Var CueInstalledBefore
Var CueRunValue

!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $CueInstalledBefore HKCU "${MANUPRODUCTKEY}" ""
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $UpdateMode <> 1
    ${If} $CueInstalledBefore == ""
      WriteRegStr HKCU "${CUE_RUN_KEY}" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
      DeleteRegValue HKCU "${CUE_STARTUP_APPROVED_KEY}" "${PRODUCTNAME}"
    ${Else}
      ReadRegStr $0 HKCU "${MANUPRODUCTKEY}" "${CUE_WAS_ON}"
      ReadRegStr $1 HKCU "${CUE_RUN_KEY}" "${PRODUCTNAME}"
      ${If} $0 == "1"
      ${AndIf} $1 == ""
        WriteRegStr HKCU "${CUE_RUN_KEY}" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
      ${EndIf}
    ${EndIf}
  ${EndIf}
  DeleteRegValue HKCU "${MANUPRODUCTKEY}" "${CUE_WAS_ON}"
!macroend

; Before CheckIfAppIsRunning, which may stop the uninstall: only read, nothing written yet.
!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $CueRunValue HKCU "${CUE_RUN_KEY}" "${PRODUCTNAME}"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $DeleteAppDataCheckboxState <> 1
    ${If} $CueRunValue != ""
      WriteRegStr HKCU "${MANUPRODUCTKEY}" "${CUE_WAS_ON}" "1"
    ${Else}
      DeleteRegValue HKCU "${MANUPRODUCTKEY}" "${CUE_WAS_ON}"
    ${EndIf}
  ${EndIf}
!macroend
