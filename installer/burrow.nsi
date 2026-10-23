; Burrow's Windows installer (D-030). Built by scripts/package.sh, which passes:
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
; PowerShell, 64-bit: the installer is 32-bit, and a 32-bit PowerShell can't see 64-bit processes.
!define POWERSHELL '"$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass'

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
!include "WordFunc.nsh"
!include "LogicLib.nsh"

; An existing install (D-056): its version, or "?" when it can't be told; empty when there is
; none. Updating skips the folder page and keeps the installed folder.
Var Installed
Var WelcomeTitle
Var WelcomeText
Var NextLabel

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
; Side panel on the welcome and finish pages (installer/make-art.py).
!define MUI_WELCOMEFINISHPAGE_BITMAP "${__FILEDIR__}\welcome.bmp"

!define MUI_WELCOMEPAGE_TITLE "$WelcomeTitle"
!define MUI_WELCOMEPAGE_TEXT "$WelcomeText"
!define MUI_PAGE_CUSTOMFUNCTION_SHOW WelcomeShow
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenInstalled
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
    ; The 64-bit PowerShell (see POWERSHELL), which can see the 64-bit burrow.exe's Path.
    nsExec::Exec `${POWERSHELL} -Command "Get-Process burrow -ErrorAction SilentlyContinue | Where-Object { $$_.Path -eq '$INSTDIR\burrow.exe' } | ForEach-Object { [void]$$_.CloseMainWindow() }"`
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

; Running an old setup (e.g. one left in Downloads) shouldn't install an old Burrow (D-056): if
; GitHub has a newer release, offer to download its setup (checked against the release's
; checksums) and run that instead. Not in silent mode, which is how Burrow updates itself.
Function OfferLatest
  IfSilent done
  InitPluginsDir
  File "/oname=$PLUGINSDIR\latest.ps1" "${__FILEDIR__}\latest.ps1"
  nsExec::ExecToStack `${POWERSHELL} -File "$PLUGINSDIR\latest.ps1" -Mode check`
  Pop $0
  Pop $1
  ${If} $0 != 0
  ${OrIf} $1 == ""
    Return
  ${EndIf}
  ${VersionCompare} $1 "${VERSION}" $2
  ${If} $2 != 1
    Return
  ${EndIf}
  MessageBox MB_YESNO|MB_ICONINFORMATION "${APP} $1 is out, and this setup is the older ${VERSION}.$\r$\n$\r$\nDownload and install ${APP} $1 instead? (Recommended)" IDNO done
  StrCpy $3 "$TEMP\burrow-$1-windows-x64-setup.exe"
  Banner::show /NOUNLOAD "Downloading ${APP} $1…"
  nsExec::ExecToStack `${POWERSHELL} -File "$PLUGINSDIR\latest.ps1" -Mode download -Version $1 -Out "$3"`
  Pop $0
  Pop $4
  Banner::destroy
  ${If} $0 == 0
    Exec '"$3"'
    Quit
  ${EndIf}
  MessageBox MB_OK|MB_ICONEXCLAMATION "${APP} $1 couldn't be downloaded. This setup will install ${VERSION}."
  done:
FunctionEnd

Function .onInit
  Call OfferLatest
  ; InstallDirRegKey has already put the installed folder in $INSTDIR, if there is one.
  ; The version is the installed burrow.exe's own (e.g. 0.0.4.0), else the uninstall entry's.
  StrCpy $Installed ""
  ${If} ${FileExists} "$INSTDIR\burrow.exe"
    StrCpy $Installed "?"
    ClearErrors
    ${GetFileVersion} "$INSTDIR\burrow.exe" $0
    ${If} ${Errors}
      ReadRegStr $0 HKCU "${UNINST_KEY}" "DisplayVersion"
    ${ElseIf} $0 == "0.0.0.0"
      ReadRegStr $0 HKCU "${UNINST_KEY}" "DisplayVersion"
    ${Else}
      StrCpy $1 $0 "" -2
      ${If} $1 == ".0"
        StrCpy $0 $0 -2
      ${EndIf}
    ${EndIf}
    ${If} $0 != ""
      StrCpy $Installed $0
    ${EndIf}
  ${EndIf}

  StrCpy $WelcomeTitle "Install ${APP} ${VERSION}"
  StrCpy $WelcomeText "This will install ${APP} ${VERSION} for your Windows account. No administrator rights are needed."
  StrCpy $NextLabel ""
  ${If} $Installed == "?"
    StrCpy $WelcomeTitle "Update ${APP}"
    StrCpy $WelcomeText "${APP} is already installed in $INSTDIR.$\r$\n$\r$\nThis will update it to ${VERSION}. Your spaces, tabs and logins are kept."
    StrCpy $NextLabel "&Update"
  ${ElseIf} $Installed != ""
    ${VersionCompare} $Installed "${VERSION}" $1
    ${If} $1 == 2
      StrCpy $WelcomeTitle "Update ${APP}"
      StrCpy $WelcomeText "${APP} $Installed is installed.$\r$\n$\r$\nThis will update it to ${VERSION}. Your spaces, tabs and logins are kept."
      StrCpy $NextLabel "&Update"
    ${ElseIf} $1 == 0
      StrCpy $WelcomeTitle "${APP} ${VERSION} is installed"
      StrCpy $WelcomeText "${APP} ${VERSION} is already installed in $INSTDIR.$\r$\n$\r$\nYou can install it again, for example to repair it. Your spaces, tabs and logins are kept."
      StrCpy $NextLabel "&Reinstall"
    ${Else}
      StrCpy $WelcomeTitle "A newer ${APP} is installed"
      StrCpy $WelcomeText "${APP} $Installed is installed, which is newer than this setup (${VERSION}).$\r$\n$\r$\nInstalling would replace it with the older version. Your spaces, tabs and logins are kept, but the older version may not read everything the newer one saved."
      StrCpy $NextLabel "&Install ${VERSION}"
    ${EndIf}
  ${EndIf}
FunctionEnd

; Names the welcome page's button after what it will do.
Function WelcomeShow
  ${If} $NextLabel != ""
    GetDlgItem $0 $HWNDPARENT 1
    SendMessage $0 ${WM_SETTEXT} 0 "STR:$NextLabel"
  ${EndIf}
FunctionEnd

; An update goes into the installed folder; only a new install asks where.
Function SkipWhenInstalled
  ${If} $Installed != ""
    Abort
  ${EndIf}
FunctionEnd

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

  ; Burrow's updater runs `setup.exe /S /RELAUNCH` after "Restart to Update" (D-037).
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/RELAUNCH" $1
  IfErrors +2 0
    Exec '"$INSTDIR\burrow.exe"'
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
