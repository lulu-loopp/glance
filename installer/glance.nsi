; Glance's installer: for every user of the machine, under Program Files
; unless another folder is chosen.
; Build with: makensis installer\glance.nsi (after cargo build --release).
;
; Installing copies Glance and the PawnIO driver's setup, which Glance runs
; itself on its first start. Uninstalling takes away everything Glance set
; up: its scheduled tasks, its registry key and its settings. The PawnIO
; driver, if Glance was the one to install it, goes only if the user says
; so: other monitoring programs may have come to rely on it since.

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

!define MUI_PAGE_CUSTOMFUNCTION_LEAVE CheckFolder
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

LangString OpenFolder ${LANG_SIMPCHINESE} "装在这里，Glance 不能开机自启，每次启动也都要确认管理员权限。$\r$\n$\r$\n这个位置上面有你的账户能改名或改权限的文件夹，别的程序就能把 Glance 换成自己，从而悄悄拿到管理员权限。Program Files，或者磁盘根目录下的新文件夹（比如 D:\Glance）没有这个问题。$\r$\n$\r$\n仍然装在这里吗？"
LangString OpenFolder ${LANG_ENGLISH} "Installed here, Glance cannot start with Windows, and every start asks for administrator rights.$\r$\n$\r$\nA folder above this one can be renamed or re-permissioned by your account, so another program could put itself in Glance's place and quietly gain administrator rights. Program Files, or a new folder at a drive's root (such as D:\Glance), is safe.$\r$\n$\r$\nInstall here anyway?"
LangString RemovePawnIO ${LANG_SIMPCHINESE} "也要卸载 PawnIO 驱动吗？$\r$\n$\r$\n它是 Glance 安装的，用来读取温度和风扇。如果其他硬件监控或风扇控制软件（比如 HWiNFO、FanControl）也在用它，请保留。"
LangString RemovePawnIO ${LANG_ENGLISH} "Remove the PawnIO driver too?$\r$\n$\r$\nGlance installed it to read temperatures and fans. Keep it if other monitoring or fan control programs (such as HWiNFO or FanControl) use it."

; A running Glance holds its files.
!macro StopGlance
  ; System tools by their full path: never one that happens to lie beside
  ; the installer.
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM ${EXE} /F'
  Sleep 500
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Glance needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

; Whether Glance in the chosen folder could start unasked: asked of Glance
; itself, which applies the same rule when it starts.
Function CheckFolder
  InitPluginsDir
  File "/oname=$PLUGINSDIR\${EXE}" "..\target\release\${EXE}"
  ExecWait '"$PLUGINSDIR\${EXE}" --check-install-folder "$INSTDIR"' $0
  ${If} $0 != 0
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(OpenFolder)" /SD IDYES IDYES +2
    Abort
  ${EndIf}
FunctionEnd

Section "Glance"
  !insertmacro StopGlance
  SetOutPath "$INSTDIR"
  File "..\target\release\${EXE}"
  SetOutPath "$INSTDIR\resources"
  File "..\resources\PawnIO_setup.exe"
  ; What Glance is built from and ships with, and the source the modules'
  ; licence asks to come with them.
  SetOutPath "$INSTDIR\licenses"
  File "..\licenses\*.txt"
  File "..\licenses\*.html"
  SetOutPath "$INSTDIR\licenses\pawnio-modules-source"
  File /r "..\pawnio-modules\source\*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Glance's folder as Program Files keeps its folders: owned by the
  ; administrators, changed only by them and the system, read by everyone.
  ; What Glance put in it then takes those rights; anything else there is
  ; left as it was.
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR" /setowner *S-1-5-32-544 /C /Q'
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR" /inheritance:r /grant:r *S-1-5-32-544:(OI)(CI)F *S-1-5-18:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX /C /Q'
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR\${EXE}" "$INSTDIR\uninstall.exe" /reset /C /Q'
  nsExec::Exec '"$SYSDIR\icacls.exe" "$INSTDIR\resources" "$INSTDIR\licenses" /reset /T /C /Q'

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
  ; Every account's tasks for Glance, removed by the installed copy itself
  ; (in a folder only administrators can change) through Task Scheduler: no
  ; shell or script host that could load anything else.
  ExecWait '"$INSTDIR\${EXE}" --remove-tasks'

  ; The driver goes only if Glance put it there and the user agrees; a
  ; silent uninstall keeps it.
  ReadRegDWORD $0 HKLM "SOFTWARE\Glance" "InstalledPawnIO"
  ReadRegStr $1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO" "QuietUninstallString"
  ${If} $0 == 1
  ${AndIf} $1 != ""
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(RemovePawnIO)" /SD IDNO IDNO keep_pawnio
    nsExec::Exec '$1'
    keep_pawnio:
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
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
