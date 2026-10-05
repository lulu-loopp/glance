; Glance's installer: for every user of the machine, under Program Files.
; Build with: makensis installer\glance.nsi (after cargo build --release).
;
; Installing copies Glance and the PawnIO driver's setup, which Glance runs
; itself on its first start. Uninstalling takes away everything Glance set
; up: its scheduled tasks, its registry key, the PawnIO driver if Glance was
; the one to install it (another program may rely on it), and its settings.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define NAME "Glance"
!define VERSION "0.1.0"
!define PUBLISHER "lulu-loopp"
!define EXE "glance.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${NAME}"
!define SETTINGS_DIR "dev.weiyi.glance"

Name "${NAME}"
OutFile "..\target\${NAME}_${VERSION}_x64-setup.exe"
InstallDir "$PROGRAMFILES64\${NAME}"
InstallDirRegKey HKLM "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ManifestDPIAware true
BrandingText "${NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${NAME} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}"

!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

; A running Glance holds its files.
!macro StopGlance
  nsExec::Exec 'schtasks.exe /End /TN "${NAME}"'
  nsExec::Exec 'taskkill.exe /IM ${EXE} /F'
  Sleep 500
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Glance needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "Glance"
  !insertmacro StopGlance
  SetOutPath "$INSTDIR"
  File "..\target\release\${EXE}"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\PawnIO_setup.exe"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  SetShellVarContext all
  CreateShortcut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\${EXE}"

  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ; Kilobytes, for Apps & features.
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" 2048
SectionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Uninstall"
  !insertmacro StopGlance
  nsExec::Exec 'schtasks.exe /Delete /TN "${NAME}" /F'
  nsExec::Exec 'schtasks.exe /Delete /TN "${NAME} at sign-in" /F'

  ; The driver goes only if Glance put it there.
  ReadRegDWORD $0 HKLM "SOFTWARE\Glance" "InstalledPawnIO"
  ${If} $0 == 1
    ReadRegStr $1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO" "QuietUninstallString"
    ${If} $1 != ""
      nsExec::Exec '$1'
    ${EndIf}
  ${EndIf}
  DeleteRegKey HKLM "SOFTWARE\Glance"

  ; The settings of the user uninstalling.
  SetShellVarContext current
  RMDir /r "$APPDATA\${SETTINGS_DIR}"

  SetShellVarContext all
  Delete "$SMPROGRAMS\${NAME}.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\resources\PawnIO_setup.exe"
  RMDir "$INSTDIR\resources"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
