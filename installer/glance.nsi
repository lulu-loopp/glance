; Glance's installer: for every user of the machine, under Program Files
; unless another folder is chosen.
; Build with: makensis installer\glance.nsi (after cargo build --release).
;
; Installing copies Glance and installs the PawnIO driver if no program has
; yet (Glance also carries its setup, to put it back from a protected place
; should it be removed). Uninstalling takes away everything Glance set
; up: its scheduled tasks, its registry key and its settings. The PawnIO
; driver, if Glance was the one to install it, goes only if the user says
; so: other monitoring programs may have come to rely on it since.

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define NAME "Glance"
!define VERSION "0.1.2"
!define PUBLISHER "lulu-loopp"
!define EXE "glance.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${NAME}"
!define SETTINGS_DIR "dev.weiyi.glance"
!define PAWNIO_KEY "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO"

Name "${NAME}"
OutFile "..\target\${NAME}_${VERSION}_x64-setup.exe"
InstallDir "$PROGRAMFILES64\${NAME}"
InstallDirRegKey HKLM "${UNINSTALL_KEY}" "InstallLocation"
RequestExecutionLevel admin

; A release build (scripts\release.ps1 -Sign, which defines SIGN) signs the
; uninstaller as it is written and the installer once it is made.
!ifdef SIGN
  !uninstfinalize 'pwsh -NoProfile -ExecutionPolicy Bypass -File "..\scripts\sign.ps1" -Files "%1"' = 0
  !finalize 'pwsh -NoProfile -ExecutionPolicy Bypass -File "..\scripts\sign.ps1" -Files "%1"' = 0
!endif
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

LangString OpenFolderAsk ${LANG_SIMPCHINESE} "所选位置不受保护。$\r$\n$\r$\n该位置的上级文件夹可由普通程序重命名或更改权限，因此安装在此处的 Glance 可能被其他程序替换，并在以管理员身份运行时被利用。$\r$\n$\r$\n为确保安全，安装在此处后：$\r$\n　· 每次启动 Glance 都需要确认管理员权限；$\r$\n　· 无法启用“开机时启动”。$\r$\n$\r$\n建议安装到 Program Files，或磁盘根目录下的新文件夹（例如 D:\Glance）。$\r$\n$\r$\n是否仍要安装到此位置？"
LangString OpenFolderAsk ${LANG_ENGLISH} "The selected location is not protected.$\r$\n$\r$\nA folder above it can be renamed or have its permissions changed by ordinary programs, so a copy of Glance installed here could be replaced by another program and misused when it runs with administrator rights.$\r$\n$\r$\nTo keep it safe, when installed here:$\r$\n  · Glance asks for administrator rights at every start;$\r$\n  · $\"Start with Windows$\" cannot be turned on.$\r$\n$\r$\nInstalling to Program Files, or to a new folder at the root of a drive (for example D:\Glance), is recommended.$\r$\n$\r$\nInstall to this location anyway?"
LangString OccupiedFolderAsk ${LANG_SIMPCHINESE} "所选文件夹已存在，且无法确认其中的内容未被其他程序改动。$\r$\n$\r$\n安装程序不会更改该文件夹中原有内容的权限。为确保安全，安装在此处后：$\r$\n　· 每次启动 Glance 都可能需要确认管理员权限；$\r$\n　· 可能无法启用“开机时启动”。$\r$\n$\r$\n建议选择一个尚不存在的新文件夹（例如 D:\Glance）。$\r$\n$\r$\n是否仍要安装到此文件夹？"
LangString OccupiedFolderAsk ${LANG_ENGLISH} "The selected folder already exists, and it cannot be confirmed that its contents have not been changed by other programs.$\r$\n$\r$\nThe installer does not change the permissions of anything already in it. To keep it safe, when installed here:$\r$\n  · Glance may ask for administrator rights at every start;$\r$\n  · $\"Start with Windows$\" may not be available.$\r$\n$\r$\nA folder that does not exist yet (for example D:\Glance) is recommended.$\r$\n$\r$\nInstall to this folder anyway?"
LangString OpenFolderStop ${LANG_SIMPCHINESE} "所选位置不受保护，安装已停止。$\r$\n$\r$\n请安装到 Program Files，或磁盘根目录下的新文件夹（例如 D:\Glance）。"
LangString OpenFolderStop ${LANG_ENGLISH} "The selected location is not protected; installation has stopped.$\r$\n$\r$\nInstall to Program Files, or to a new folder at the root of a drive (for example D:\Glance)."
LangString OccupiedFolderStop ${LANG_SIMPCHINESE} "所选文件夹已存在，且无法确认其中的内容未被其他程序改动，安装已停止。$\r$\n$\r$\n请选择一个尚不存在的新文件夹（例如 D:\Glance）。"
LangString OccupiedFolderStop ${LANG_ENGLISH} "The selected folder already exists, and it cannot be confirmed that its contents have not been changed by other programs; installation has stopped.$\r$\n$\r$\nChoose a folder that does not exist yet (for example D:\Glance)."
LangString IndirectFolder ${LANG_SIMPCHINESE} "无法安装到所选位置。$\r$\n$\r$\n所选位置是磁盘根目录、联接点或符号链接：文件将被写入其他位置，或与磁盘根目录中的其他文件混在一起，卸载时也无法完整清除。$\r$\n$\r$\n请选择一个普通文件夹（例如 D:\Glance）。"
LangString IndirectFolder ${LANG_ENGLISH} "Glance cannot be installed to the selected location.$\r$\n$\r$\nIt is the root of a drive, a junction or a symbolic link: the files would be written elsewhere, or mixed with other files at the root of the drive, and could not be removed completely when uninstalling.$\r$\n$\r$\nChoose an ordinary folder (for example D:\Glance)."
LangString UnpreparedFolder ${LANG_SIMPCHINESE} "无法准备安装文件夹，安装已停止。"
LangString UnpreparedFolder ${LANG_ENGLISH} "The installation folder could not be prepared; installation has stopped."
LangString RemovePawnIO ${LANG_SIMPCHINESE} "是否同时卸载 PawnIO 驱动？$\r$\n$\r$\nPawnIO 由 Glance 安装，用于读取温度和风扇转速。如果其他硬件监控或风扇控制软件（例如 HWiNFO、FanControl）也在使用它，请选择“否”予以保留。"
LangString RemovePawnIO ${LANG_ENGLISH} "Remove the PawnIO driver as well?$\r$\n$\r$\nPawnIO was installed by Glance to read temperatures and fan speeds. If other monitoring or fan control software (for example HWiNFO or FanControl) also uses it, choose No to keep it."

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

; Whether the user chose to install in an unprotected place all the same.
Var Anyway

; Why a folder is refused, by Glance's answer: 1 a folder above it can be
; changed by others, 2 it exists and is not an earlier Glance only
; administrators could change, 3 it is a link or a drive's root, 4 Glance
; could not answer.
!macro Refuse code
  ${If} ${code} == 1
    MessageBox MB_ICONSTOP "$(OpenFolderStop)" /SD IDOK
  ${ElseIf} ${code} == 2
    MessageBox MB_ICONSTOP "$(OccupiedFolderStop)" /SD IDOK
  ${ElseIf} ${code} == 3
    MessageBox MB_ICONSTOP "$(IndirectFolder)" /SD IDOK
  ${Else}
    MessageBox MB_ICONSTOP "$(UnpreparedFolder)" /SD IDOK
  ${EndIf}
  Abort
!macroend

; Runs Glance with `flag` on the chosen folder; $0 is its answer. A Glance
; that did not run (deleted from the plugins folder, say) answers 4, which
; refuses like any other failure.
!macro Ask flag
  StrCpy $0 4
  ClearErrors
  ExecWait '"$PLUGINSDIR\${EXE}" ${flag} "$INSTDIR"' $0
  ${If} ${Errors}
    StrCpy $0 4
  ${EndIf}
!macroend

; An unprotected place, or an existing folder, is the user's call: told
; what it means, they may install there all the same (the default answer
; is No). A link or a drive's root is refused.
Function CheckFolder
  StrCpy $Anyway 0
  !insertmacro Helper
  !insertmacro Ask --check-install-folder
  ${If} $0 == 1
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(OpenFolderAsk)" /SD IDNO IDYES +2
    Abort
    StrCpy $Anyway 1
  ${ElseIf} $0 == 2
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(OccupiedFolderAsk)" /SD IDNO IDYES +2
    Abort
    StrCpy $Anyway 1
  ${ElseIf} $0 != 0
    !insertmacro Refuse $0
  ${EndIf}
FunctionEnd

Section "Glance"
  !insertmacro StopGlance
  ; A new folder is created administrators' in one step, as Program Files'
  ; folders are; an earlier Glance's, which only administrators could ever
  ; change, is used as it is. Elsewhere only if the user chose so above;
  ; otherwise (a silent install given such a folder with /D, say) it stops.
  !insertmacro Helper
  ${If} $Anyway == 1
    !insertmacro Ask --prepare-install-folder-anyway
  ${Else}
    !insertmacro Ask --prepare-install-folder
  ${EndIf}
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

  ; The PawnIO driver, if no program has installed it yet: from the
  ; installer's own copy, wherever Glance goes (Glance installs it at start
  ; only from a protected place). Noted as Glance's, so that uninstalling
  ; offers to remove it.
  ReadRegStr $1 HKLM "${PAWNIO_KEY}" "DisplayVersion"
  ${If} $1 == ""
    File "/oname=$PLUGINSDIR\PawnIO_setup.exe" "..\resources\PawnIO_setup.exe"
    ExecWait '"$PLUGINSDIR\PawnIO_setup.exe" -install -silent'
    ReadRegStr $1 HKLM "${PAWNIO_KEY}" "DisplayVersion"
    ${If} $1 != ""
      WriteRegDWORD HKLM "SOFTWARE\Glance" "InstalledPawnIO" 1
    ${EndIf}
  ${EndIf}

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
  ReadRegStr $1 HKLM "${PAWNIO_KEY}" "QuietUninstallString"
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
