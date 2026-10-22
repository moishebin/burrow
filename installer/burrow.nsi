; Burrow's Windows installer (D-027). Built by scripts/package.sh, which passes:
;   VERSION     e.g. 0.0.1
;   SRCDIR      the staged Burrow/ folder (exe, CEF runtime, ui/)
;   UNLIST      a generated .nsh that deletes exactly the installed files
;   OUTFILE     the setup .exe to write
;   ICON        burrow.ico
; Per-user install, no admin: %LOCALAPPDATA%\Programs\Burrow (same place as install-win.sh).
; User data lives in %LOCALAPPDATA%\Burrow and is kept unless the user asks to remove it.

Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define APP "Burrow"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

!ifdef SIGN
  ; Sign the setup and the uninstaller it writes (package.sh passes BURROW_SIGN).
  !finalize '${SIGN} "%1"' = 0
  !uninstfinalize '${SIGN} "%1"' = 0
!endif

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "Software\${APP}" "InstallDir"
BrandingText "${APP} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "CompanyName" "${APP}"
VIAddVersionKey "LegalCopyright" "MPL-2.0"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
; Side panel on the welcome and finish pages (installer/make-art.py).
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\welcome.bmp"

!define MUI_WELCOMEPAGE_TEXT "This will install ${APP} ${VERSION} for your Windows account. No administrator rights are needed.$\r$\n$\r$\nIf ${APP} is already installed, it is updated and your spaces, tabs and logins are kept."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\burrow.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${APP}"
; The "show readme" checkbox doubles as the desktop shortcut option.
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Add a desktop shortcut"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION DesktopShortcut
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; A running burrow.exe can't be overwritten or deleted. Offer to close it (gracefully, so it saves).
!macro CLOSE_BURROW un
Function ${un}CloseBurrow
  StrCpy $1 0
  check:
    IfFileExists "$INSTDIR\burrow.exe" 0 done
    ClearErrors
    FileOpen $0 "$INSTDIR\burrow.exe" a
    IfErrors running
    FileClose $0
    Goto done
  running:
    IntCmp $1 0 ask wait wait
  ask:
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "${APP} is running. Click OK to close it and continue. Your tabs are saved." /SD IDOK IDOK close
    Abort
  close:
    ; The installer is 32-bit: a 32-bit PowerShell can't see a 64-bit process's Path, so use the
    ; 64-bit one through Sysnative.
    nsExec::Exec `"$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -Command "Get-Process burrow -ErrorAction SilentlyContinue | Where-Object { $$_.Path -eq '$INSTDIR\burrow.exe' } | ForEach-Object { [void]$$_.CloseMainWindow() }"`
    Pop $0
  wait:
    ; Up to 20 s for Burrow to save and quit.
    IntOp $1 $1 + 1
    IntCmp $1 40 stuck 0 stuck
    Sleep 500
    Goto check
  stuck:
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP} is still running. Close it, then click Retry." /SD IDCANCEL IDRETRY retry
    Abort
  retry:
    StrCpy $1 1
    Goto check
  done:
FunctionEnd
!macroend
!insertmacro CLOSE_BURROW ""
!insertmacro CLOSE_BURROW "un."

Function DesktopShortcut
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\burrow.exe"
FunctionEnd

Section "Install"
  Call CloseBurrow
  SetOutPath "$INSTDIR"
  File /r "${SRCDIR}\*"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\burrow.exe"

  WriteRegStr HKCU "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\burrow.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/moishebin/burrow"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Uninstall"
  Call un.CloseBurrow
  ; Only the files the installer put there, in case the folder holds anything else.
  !include "${UNLIST}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${APP}"

  MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "Also delete your ${APP} data (spaces, tabs, history and logins)?" /SD IDNO IDNO keep
    RMDir /r "$LOCALAPPDATA\${APP}"
  keep:
SectionEnd
