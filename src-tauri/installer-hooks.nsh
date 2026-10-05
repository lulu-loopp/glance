; Uninstalling Glance takes away what Glance set up: its scheduled tasks, its
; own registry key, and the PawnIO driver if Glance was the one to install it
; (another program may have installed it, and may rely on it).

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'schtasks.exe /Delete /TN "Glance" /F'
  nsExec::Exec 'schtasks.exe /Delete /TN "Glance at sign-in" /F'
  SetRegView 64
  ReadRegDWORD $0 HKLM "SOFTWARE\Glance" "InstalledPawnIO"
  ${If} $0 == 1
    ReadRegStr $1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO" "QuietUninstallString"
    ${If} $1 != ""
      nsExec::Exec '$1'
    ${EndIf}
  ${EndIf}
  DeleteRegKey HKLM "SOFTWARE\Glance"
  SetRegView lastused
!macroend
