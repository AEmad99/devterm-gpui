; DevTerm Windows installer. App id stays com.devterm.app.
; In-box ConPTY (kernel32 CreatePseudoConsole) is the default. Ship
; conpty\conpty.dll and OpenConsole.exe only for DEVTERM_USE_BUNDLED_CONPTY=1.
; Unsigned builds should set CSC_IDENTITY_AUTO_DISCOVERY=false when the
; Windows code-signing symlink helper fails.
;
; The Node sidecar for the bundled agent lives in $INSTDIR\node. A clean
; machine must be able to open a local PTY without that sidecar; the agent
; is a separate process.

!include "MUI2.nsh"

Name "DevTerm"
OutFile "..\dist\DevTerm-Setup.exe"
InstallDir "$LOCALAPPDATA\DevTerm"
InstallDirRegKey HKCU "Software\DevTerm" "InstallDir"
RequestExecutionLevel user

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "DevTerm" SecMain
  SetOutPath "$INSTDIR"
  File /oname=devterm.exe "..\target\release\devterm-gpui.exe"
  ; Optional bundled ConPTY. Absent on a normal install, so portable-pty
  ; stays on the in-box console host.
  SetOutPath "$INSTDIR\conpty"
  File /nonfatal "..\packaging\conpty\conpty.dll"
  File /nonfatal "..\packaging\conpty\OpenConsole.exe"
  SetOutPath "$INSTDIR\node"
  File /nonfatal /r "..\packaging\node\*.*"
  WriteRegStr HKCU "Software\DevTerm" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\DevTerm" "AppUserModelId" "com.devterm.app"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\DevTerm.lnk" "$INSTDIR\devterm.exe"
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\devterm.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\conpty"
  RMDir /r "$INSTDIR\node"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\DevTerm.lnk"
  DeleteRegKey HKCU "Software\DevTerm"
SectionEnd
