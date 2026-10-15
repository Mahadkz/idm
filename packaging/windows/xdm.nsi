; Per-user XDM installer. Build with build-installer.sh (passes VERSION, BINDIR, EXTDIR, SRCDIR, OUTFILE).
Unicode true
RequestExecutionLevel user
!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !error "pass -DVERSION=x.y.z"
!endif
Name "XDM ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\XDM"
InstallDirRegKey HKCU "Software\XDM" "InstallDir"
SetCompressor /SOLID lzma
VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "XDM"
VIAddVersionKey "FileDescription" "XDM download manager installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\XDM"
!define NHOST "org.xdm.browser"

!define MUI_ICON "${SRCDIR}\assets\xdm.ico"
!define MUI_UNICON "${SRCDIR}\assets\xdm.ico"
!insertmacro MUI_PAGE_LICENSE "${SRCDIR}\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "XDM (required)" SecMain
  SectionIn RO
  ; Upgrade in place: stop running instances so files can be replaced.
  nsExec::Exec 'taskkill /F /IM xdm-gui.exe'
  nsExec::Exec 'taskkill /F /IM xdm-daemon.exe'
  SetOutPath "$INSTDIR"
  File "${BINDIR}\xdm-gui.exe"
  File "${BINDIR}\xdm-daemon.exe"
  File "${BINDIR}\xdm-cli.exe"
  File "${BINDIR}\xdm-native-host.exe"
  File "${SRCDIR}\assets\xdm.ico"
  File "${SRCDIR}\LICENSE"
  File "${SRCDIR}\THIRD_PARTY_LICENSES.md"
  File "${EXTDIR}\${NHOST}.chrome.json"
  File "${EXTDIR}\${NHOST}.firefox.json"
  SetOutPath "$INSTDIR\extensions"
  File "${EXTDIR}\xdm-chromium-${VERSION}.zip"
  File "${EXTDIR}\xdm-firefox-${VERSION}.xpi"

  WriteRegStr HKCU "Software\XDM" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\XDM" "Version" "${VERSION}"

  ; Start menu
  CreateDirectory "$SMPROGRAMS\XDM"
  CreateShortcut "$SMPROGRAMS\XDM\XDM.lnk" "$INSTDIR\xdm-gui.exe" "" "$INSTDIR\xdm.ico"
  CreateShortcut "$SMPROGRAMS\XDM\Uninstall XDM.lnk" "$INSTDIR\uninstall.exe"

  ; Native messaging host registration (per-user, HKCU). Manifests use paths relative to themselves.
  WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\${NHOST}" "" "$INSTDIR\${NHOST}.chrome.json"
  WriteRegStr HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NHOST}" "" "$INSTDIR\${NHOST}.chrome.json"
  WriteRegStr HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\${NHOST}" "" "$INSTDIR\${NHOST}.chrome.json"
  WriteRegStr HKCU "Software\Chromium\NativeMessagingHosts\${NHOST}" "" "$INSTDIR\${NHOST}.chrome.json"
  WriteRegStr HKCU "Software\Mozilla\NativeMessagingHosts\${NHOST}" "" "$INSTDIR\${NHOST}.firefox.json"

  ; Uninstall entry
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "XDM"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "XDM"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\xdm.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section /o "Start XDM daemon when I sign in" SecAuto
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "XDM" '"$INSTDIR\xdm-daemon.exe"'
SectionEnd

Section "-cleanup-stale-autostart"
  ; On upgrade, drop the Run entry when the user unticked autostart this time.
  ${IfNot} ${SectionIsSelected} ${SecAuto}
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "XDM"
  ${EndIf}
SectionEnd

LangString DESC_Main ${LANG_ENGLISH} "XDM programs, browser extension packages and native-host registration."
LangString DESC_Auto ${LANG_ENGLISH} "Start the background daemon automatically at sign-in."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAuto} $(DESC_Auto)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  nsExec::Exec 'taskkill /F /IM xdm-gui.exe'
  nsExec::Exec 'taskkill /F /IM xdm-daemon.exe'
  ; Only files this installer created. User data (%APPDATA%/%LOCALAPPDATA% XDM data, downloads) is untouched.
  Delete "$INSTDIR\xdm-gui.exe"
  Delete "$INSTDIR\xdm-daemon.exe"
  Delete "$INSTDIR\xdm-cli.exe"
  Delete "$INSTDIR\xdm-native-host.exe"
  Delete "$INSTDIR\xdm.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_LICENSES.md"
  Delete "$INSTDIR\${NHOST}.chrome.json"
  Delete "$INSTDIR\${NHOST}.firefox.json"
  Delete "$INSTDIR\extensions\xdm-chromium-${VERSION}.zip"
  Delete "$INSTDIR\extensions\xdm-firefox-${VERSION}.xpi"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR\extensions"
  RMDir "$INSTDIR"   ; non-recursive: stays if the user added files
  Delete "$SMPROGRAMS\XDM\XDM.lnk"
  Delete "$SMPROGRAMS\XDM\Uninstall XDM.lnk"
  RMDir "$SMPROGRAMS\XDM"
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\${NHOST}"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\${NHOST}"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\${NHOST}"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\${NHOST}"
  DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\${NHOST}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "XDM"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\XDM"
SectionEnd
