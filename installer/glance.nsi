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
!define VERSION "0.1.1"
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

LangString OpenFolder ${LANG_SIMPCHINESE} "Glance 不能装在这里。$\r$\n$\r$\n这个位置上面有你的账户（或其他程序）能改名、改权限的文件夹，装进去的 Glance 可能被别的程序换掉，再以管理员身份运行。$\r$\n$\r$\n请选 Program Files，或者磁盘根目录下的新文件夹，比如 D:\Glance。"
LangString OpenFolder ${LANG_ENGLISH} "Glance cannot be installed here.$\r$\n$\r$\nA folder above this one can be renamed or re-permissioned by your account (or other programs), so Glance installed here could be replaced by another program and then run with administrator rights.$\r$\n$\r$\nChoose Program Files, or a new folder at a drive's root, such as D:\Glance."
LangString OccupiedFolder ${LANG_SIMPCHINESE} "请选一个还不存在的新文件夹，比如 D:\Glance。$\r$\n$\r$\n这个文件夹已经存在，而且不是只有管理员能改动的旧版 Glance：里面的东西没法保证没被动过。"
LangString OccupiedFolder ${LANG_ENGLISH} "Choose a folder that does not exist yet, such as D:\Glance.$\r$\n$\r$\nThis one exists and is not an earlier Glance only administrators could change: what is in it cannot be vouched for."
LangString IndirectFolder ${LANG_SIMPCHINESE} "这个位置是联接点、符号链接或磁盘根目录，实际指向的不是这里。请选一个普通文件夹。"
LangString IndirectFolder ${LANG_ENGLISH} "This is a junction, a symbolic link or a drive's root: it leads somewhere else. Choose an ordinary folder."
LangString UnpreparedFolder ${LANG_SIMPCHINESE} "无法设置安装文件夹的权限，安装已停止。"
LangString UnpreparedFolder ${LANG_ENGLISH} "The install folder's permissions could not be set; installation stopped."
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

; Glance itself, in the plugins folder (which NSIS makes administrators'
; when elevated), answers for the chosen folder and prepares it: by the same
; rules it applies when it starts.
!macro Helper
  InitPluginsDir
  File "/oname=$PLUGINSDIR\${EXE}" "..\target\release\${EXE}"
!macroend

; Why a folder is refused, by Glance's answer: 1 a folder above it can be
; changed by others, 2 it exists and is not an earlier Glance only
; administrators could change, 3 it is a link or a drive's root.
!macro Refuse code
  ${If} ${code} == 1
    MessageBox MB_ICONSTOP "$(OpenFolder)" /SD IDOK
  ${ElseIf} ${code} == 2
    MessageBox MB_ICONSTOP "$(OccupiedFolder)" /SD IDOK
  ${ElseIf} ${code} == 3
    MessageBox MB_ICONSTOP "$(IndirectFolder)" /SD IDOK
  ${Else}
    MessageBox MB_ICONSTOP "$(UnpreparedFolder)" /SD IDOK
  ${EndIf}
  Abort
!macroend

Function CheckFolder
  !insertmacro Helper
  ExecWait '"$PLUGINSDIR\${EXE}" --check-install-folder "$INSTDIR"' $0
  ${If} $0 != 0
    !insertmacro Refuse $0
  ${EndIf}
FunctionEnd

Section "Glance"
  !insertmacro StopGlance
  ; A new folder is created administrators' in one step, as Program Files'
  ; folders are; an earlier Glance's, which only administrators could ever
  ; change, is used as it is. Anything else is refused and left alone (a
  ; silent install given one with /D stops here).
  !insertmacro Helper
  ExecWait '"$PLUGINSDIR\${EXE}" --prepare-install-folder "$INSTDIR"' $0
  ${If} $0 != 0
    !insertmacro Refuse $0
  ${EndIf}
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
