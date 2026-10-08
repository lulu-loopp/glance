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
!include "FileFunc.nsh"

!define NAME "Glance"
!define VERSION "0.2.3"
!define PUBLISHER "lulu-loopp"
!define EXE "glance.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${NAME}"
!define PAWNIO_KEY "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO"

Name "${NAME}"
OutFile "..\target\${NAME}_${VERSION}_x64-setup.exe"
InstallDir "$PROGRAMFILES64\${NAME}"
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
!define MUI_PAGE_CUSTOMFUNCTION_SHOW FinishShow
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

LangString OpenFolderAsk ${LANG_SIMPCHINESE} "所选位置不受保护。$\r$\n$\r$\n该位置的上级文件夹可由普通程序重命名或更改权限，因此安装在此处的 Glance 可能被其他程序替换，并在以管理员身份运行时被利用。$\r$\n$\r$\n为确保安全，安装在此处后：$\r$\n　· 每次启动 Glance 都需要确认管理员权限；$\r$\n　· 无法启用“开机时启动”。$\r$\n$\r$\n建议安装到 Program Files，或磁盘根目录下的新文件夹（例如 D:\Glance）。$\r$\n$\r$\n是否仍要安装到此位置？"
LangString OpenFolderAsk ${LANG_ENGLISH} "The selected location is not protected.$\r$\n$\r$\nA folder above it can be renamed or have its permissions changed by ordinary programs, so a copy of Glance installed here could be replaced by another program and misused when it runs with administrator rights.$\r$\n$\r$\nTo keep it safe, when installed here:$\r$\n  · Glance asks for administrator rights at every start;$\r$\n  · $\"Start with Windows$\" cannot be turned on.$\r$\n$\r$\nInstalling to Program Files, or to a new folder at the root of a drive (for example D:\Glance), is recommended.$\r$\n$\r$\nInstall to this location anyway?"
LangString OpenFolderStop ${LANG_SIMPCHINESE} "所选位置不受保护，安装已停止。$\r$\n$\r$\n请安装到 Program Files，或磁盘根目录下的新文件夹（例如 D:\Glance）。"
LangString OpenFolderStop ${LANG_ENGLISH} "The selected location is not protected; installation has stopped.$\r$\n$\r$\nInstall to Program Files, or to a new folder at the root of a drive (for example D:\Glance)."
LangString OccupiedFolder ${LANG_SIMPCHINESE} "无法安装到所选文件夹。$\r$\n$\r$\n该文件夹已存在，且不是仅管理员可修改的早期 Glance 安装：其他程序可能已改动其中的内容，以管理员身份向其中写入文件并不安全。$\r$\n$\r$\n请选择一个尚不存在的新文件夹（例如 D:\Glance），或先删除该文件夹。"
LangString OccupiedFolder ${LANG_ENGLISH} "Glance cannot be installed to the selected folder.$\r$\n$\r$\nThe folder already exists and is not an earlier Glance installation that only administrators can modify: other programs may have changed its contents, and writing into it with administrator rights is not safe.$\r$\n$\r$\nChoose a folder that does not exist yet (for example D:\Glance), or delete this folder first."
LangString IndirectFolder ${LANG_SIMPCHINESE} "无法安装到所选位置。$\r$\n$\r$\n所选位置是磁盘根目录、联接点或符号链接：文件将被写入其他位置，或与磁盘根目录中的其他文件混在一起，卸载时也无法完整清除。$\r$\n$\r$\n请选择一个普通文件夹（例如 D:\Glance）。"
LangString IndirectFolder ${LANG_ENGLISH} "Glance cannot be installed to the selected location.$\r$\n$\r$\nIt is the root of a drive, a junction or a symbolic link: the files would be written elsewhere, or mixed with other files at the root of the drive, and could not be removed completely when uninstalling.$\r$\n$\r$\nChoose an ordinary folder (for example D:\Glance)."
LangString InstallFailed ${LANG_SIMPCHINESE} "无法将文件写入安装文件夹，安装已停止。"
LangString InstallFailed ${LANG_ENGLISH} "The files could not be written to the installation folder; installation has stopped."
LangString RemoveFailed ${LANG_SIMPCHINESE} "以下文件夹中的部分文件未能删除，请手动删除：$\r$\n$INSTDIR"
LangString RemoveFailed ${LANG_ENGLISH} "Some files could not be removed from the following folder; please delete them manually:$\r$\n$INSTDIR"
LangString BackToDefaultNo ${LANG_SIMPCHINESE} "选择“否”将恢复为默认位置：$PROGRAMFILES64\${NAME}"
LangString BackToDefaultNo ${LANG_ENGLISH} "Choose No to return to the default location: $PROGRAMFILES64\${NAME}"
LangString BackToDefaultOk ${LANG_SIMPCHINESE} "点击“确定”后将恢复为默认位置：$PROGRAMFILES64\${NAME}"
LangString BackToDefaultOk ${LANG_ENGLISH} "Clicking OK returns to the default location: $PROGRAMFILES64\${NAME}"
LangString SettingsLeft ${LANG_SIMPCHINESE} "Glance 的设置文件正被其他程序占用，未能删除。可稍后手动删除以下文件夹：$\r$\n$APPDATA\dev.weiyi.glance"
LangString SettingsLeft ${LANG_ENGLISH} "Glance's settings file is in use by another program and could not be removed. You can delete this folder later:$\r$\n$APPDATA\dev.weiyi.glance"
LangString RemovePawnIO ${LANG_SIMPCHINESE} "是否同时卸载 PawnIO 驱动？$\r$\n$\r$\nPawnIO 由 Glance 安装，用于读取温度和风扇转速。如果其他硬件监控或风扇控制软件（例如 HWiNFO、FanControl）也在使用它，请选择“否”予以保留。"
LangString RemovePawnIO ${LANG_ENGLISH} "Remove the PawnIO driver as well?$\r$\n$\r$\nPawnIO was installed by Glance to read temperatures and fan speeds. If other monitoring or fan control software (for example HWiNFO or FanControl) also uses it, choose No to keep it."

; A running Glance holds its files.
!macro StopGlance
  ; System tools by their full path: never one that happens to lie beside
  ; the installer.
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM ${EXE} /F'
  Sleep 500
!macroend

; An update taken from Glance's settings runs silent with /RESTART: Glance
; is started again once it is in.
Function .onInstSuccess
  Call Restart
FunctionEnd

; One that failed starts again the Glance it closed, still there.
Function .onInstFailed
  Call Restart
FunctionEnd

Function Restart
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/RESTART" $1
  ${IfNot} ${Errors}
  ${AndIf} ${Silent}
  ${AndIf} ${FileExists} "$INSTDIR\${EXE}"
    SetOutPath "$INSTDIR"
    Exec '"$INSTDIR\${EXE}"'
  ${EndIf}
FunctionEnd

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Glance needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  ; Where an earlier Glance was put, so that installing again (an update)
  ; replaces it there. Read here, in the 64-bit view it was written in:
  ; InstallDirRegKey looks in the 32-bit one. A folder given with /D stands.
  ${If} $INSTDIR == "$PROGRAMFILES64\${NAME}"
    ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
    ${If} $0 != ""
      StrCpy $INSTDIR $0
    ${EndIf}
  ${EndIf}
FunctionEnd

; Everything that touches the install folder is done by a copy of Glance
; the installer (and uninstaller) carries, run from the plugins folder,
; which NSIS makes administrators' when elevated: never by the copy in the
; install folder, which may be somewhere others can change.
!macro Helper
  InitPluginsDir
  File "/oname=$PLUGINSDIR\${EXE}" "..\target\release\${EXE}"
!macroend

; Runs the helper with `arguments`; $0 is its answer. A helper that did not
; run (deleted from the plugins folder, say) answers 4, a failure.
!macro Ask arguments
  StrCpy $0 4
  ClearErrors
  ExecWait '"$PLUGINSDIR\${EXE}" ${arguments}' $0
  ${If} ${Errors}
    StrCpy $0 4
  ${EndIf}
!macroend

; Whether the user chose to install in an unprotected place all the same.
Var Anyway

; Why installing stopped, by the helper's answer: 1 a folder above can be
; changed by others, 2 the folder cannot be vouched for, 3 it is a link or
; a drive's root, 4 the files could not be put there.
!macro Refuse code
  ${If} ${code} == 1
    MessageBox MB_ICONSTOP "$(OpenFolderStop)" /SD IDOK
  ${ElseIf} ${code} == 2
    MessageBox MB_ICONSTOP "$(OccupiedFolder)" /SD IDOK
  ${ElseIf} ${code} == 3
    MessageBox MB_ICONSTOP "$(IndirectFolder)" /SD IDOK
  ${Else}
    MessageBox MB_ICONSTOP "$(InstallFailed)" /SD IDOK
  ${EndIf}
  Abort
!macroend

; An unprotected place is the user's call: told what it means, they may
; install there all the same (No is the default). A folder that cannot be
; vouched for, a link or a drive's root is refused.
Function CheckFolder
  StrCpy $Anyway 0
  !insertmacro Helper
  !insertmacro Ask '--check-install-folder "$INSTDIR"'
  ${If} $0 == 1
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(OpenFolderAsk)$\r$\n$\r$\n$(BackToDefaultNo)" /SD IDNO IDYES anyway
    Call BackToDefault
    Abort
    anyway:
    StrCpy $Anyway 1
  ${ElseIf} $0 == 2
    MessageBox MB_ICONSTOP "$(OccupiedFolder)$\r$\n$\r$\n$(BackToDefaultOk)" /SD IDOK
    Call BackToDefault
    Abort
  ${ElseIf} $0 == 3
    MessageBox MB_ICONSTOP "$(IndirectFolder)$\r$\n$\r$\n$(BackToDefaultOk)" /SD IDOK
    Call BackToDefault
    Abort
  ${ElseIf} $0 != 0
    !insertmacro Refuse $0
  ${EndIf}
FunctionEnd

; Puts the default folder back in the folder page's box, so that the user
; sees where Glance goes unless told otherwise.
Function BackToDefault
  StrCpy $INSTDIR "$PROGRAMFILES64\${NAME}"
  FindWindow $1 "#32770" "" $HWNDPARENT
  GetDlgItem $1 $1 1019
  SendMessage $1 ${WM_SETTEXT} 0 "STR:$INSTDIR"
FunctionEnd

; Installed in an unprotected place, Glance is not started from the finish
; page: started from there it would run with the installer's rights, from
; a folder others could have swapped since. The user starts it as usual.
Function FinishShow
  ${If} $Anyway == 1
    SendMessage $mui.FinishPage.Run ${BM_SETCHECK} ${BST_UNCHECKED} 0
    ShowWindow $mui.FinishPage.Run ${SW_HIDE}
  ${EndIf}
FunctionEnd

Section "Glance"
  !insertmacro StopGlance
  !insertmacro Helper
  ; The files are laid out in the plugins folder first; the helper then
  ; pins the install folder (nothing above it can be swapped for a link
  ; while it is held), creates it administrators' if it is new, checks it
  ; again, and copies them in. A silent install never installs "anyway".
  SetOutPath "$PLUGINSDIR\payload"
  File "..\target\release\${EXE}"
  SetOutPath "$PLUGINSDIR\payload\resources"
  File "..\resources\PawnIO_setup.exe"
  ; What Glance is built from and ships with, and the source the modules'
  ; licence asks to come with them.
  SetOutPath "$PLUGINSDIR\payload\licenses"
  File "..\licenses\*.txt"
  File "..\licenses\*.html"
  SetOutPath "$PLUGINSDIR\payload\licenses\pawnio-modules-source"
  File /r "..\pawnio-modules\source\*"
  WriteUninstaller "$PLUGINSDIR\payload\uninstall.exe"
  SetOutPath "$PLUGINSDIR"
  ${If} $Anyway == 1
    !insertmacro Ask '--install-anyway "$INSTDIR" "$PLUGINSDIR\payload"'
  ${Else}
    !insertmacro Ask '--install "$INSTDIR" "$PLUGINSDIR\payload"'
  ${EndIf}
  ${If} $0 != 0
    !insertmacro Refuse $0
  ${EndIf}

  ; The PawnIO driver, if no program has installed it yet: from the
  ; installer's own copy, wherever Glance goes (Glance installs it at start
  ; only from a protected place). Noted as Glance's, so that uninstalling
  ; offers to remove it.
  ReadRegStr $1 HKLM "${PAWNIO_KEY}" "DisplayVersion"
  ${If} $1 == ""
    ExecWait '"$PLUGINSDIR\payload\resources\PawnIO_setup.exe" -install -silent'
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
  !insertmacro Helper
  ; Every account's tasks for Glance, removed through Task Scheduler by the
  ; uninstaller's own copy of Glance.
  ExecWait '"$PLUGINSDIR\${EXE}" --remove-tasks'

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

  ; The settings of the user uninstalling (the one file Glance keeps, and
  ; its folder), removed by the helper through handles, following no link.
  !insertmacro Ask '--remove-settings'
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "$(SettingsLeft)" /SD IDOK
  ${EndIf}

  SetShellVarContext all
  Delete "$SMPROGRAMS\${NAME}.lnk"
  ; Glance's own files, taken out by the helper with the folder pinned.
  !insertmacro Ask '--uninstall-from "$INSTDIR"'
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "$(RemoveFailed)" /SD IDOK
  ${EndIf}
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd
