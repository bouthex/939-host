; Instalador de 939 Host (por usuario, sin permisos de administrador)
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APPNAME "939 Host"
!define APPEXE "939Host.exe"
!ifndef VERSION
  !define VERSION "0.9.0"
!endif
!ifndef SRCDIR
  !define SRCDIR "payload"
!endif

; Firma opcional: makensis -DSIGN_CMD="<comando que firma %1>" …
; Se firma el instalador y también el desinstalador que genera (Smart App Control revisa los dos).
!ifdef SIGN_CMD
  !finalize '${SIGN_CMD}'
  !uninstfinalize '${SIGN_CMD}'
!endif

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "939 Host"
VIAddVersionKey "CompanyName" "939 Host"
VIAddVersionKey "FileDescription" "Instalador de 939 Host"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "© 2026 939 Host"

; Modos de compilación para firmar también el desinstalador con un servicio externo (SignPath):
;   -DINNER            genera un ejecutable auxiliar que solo escribe Desinstalar.exe en %TEMP%\939host-uninst
;   -DSIGNED_UNINST=…  usa ese Desinstalar.exe ya firmado en lugar de generarlo
Name "${APPNAME} ${VERSION}"
!ifdef INNER
  OutFile "939Host-uninst-generador.exe"
  SilentInstall silent
!else
  OutFile "939Host-${VERSION}-instalador.exe"
!endif
InstallDir "$LOCALAPPDATA\Programs\939 Host"
InstallDirRegKey HKCU "Software\939 Host" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "939 Host ${VERSION}"

!define MUI_ICON "${SRCDIR}/icon.ico"
!define MUI_UNICON "${SRCDIR}/icon.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEFINISHPAGE_BITMAP "bienvenida.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "bienvenida.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "encabezado.bmp"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APPEXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir 939 Host"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${SRCDIR}/LEEME-INSTALACION.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

Function .onInit
!ifdef INNER
  CreateDirectory "$TEMP\939host-uninst"
  WriteUninstaller "$TEMP\939host-uninst\Desinstalar.exe"
  Quit
!endif
  ; 939 Host abierto: pedir que se cierre antes de actualizar (los servidores se guardan al salir).
  FindWindow $0 "" "939 Host"
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "939 Host está abierto. Cerralo con «Salir» (así guarda y pasa los servidores) y volvé a ejecutar el instalador." /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

Section "939 Host" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${SRCDIR}/${APPEXE}"
  File "${SRCDIR}/WebView2Loader.dll"
  File "${SRCDIR}/icon.ico"
  File "${SRCDIR}/LEEME-INSTALACION.txt"
  File "${SRCDIR}/PRIVACIDAD.md"
  SetOutPath "$INSTDIR\licencias"
  File /r "${SRCDIR}/licencias/*.*"

  CreateDirectory "$SMPROGRAMS\939 Host"
  CreateShortcut "$SMPROGRAMS\939 Host\939 Host.lnk" "$INSTDIR\${APPEXE}" "" "$INSTDIR\icon.ico"
  CreateShortcut "$SMPROGRAMS\939 Host\Desinstalar 939 Host.lnk" "$INSTDIR\Desinstalar.exe"
  CreateShortcut "$DESKTOP\939 Host.lnk" "$INSTDIR\${APPEXE}" "" "$INSTDIR\icon.ico"

  WriteRegStr HKCU "Software\939 Host" "InstallDir" "$INSTDIR"
!ifdef SIGNED_UNINST
  File "/oname=Desinstalar.exe" "${SIGNED_UNINST}"
!else
  WriteUninstaller "$INSTDIR\Desinstalar.exe"
!endif
  !define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\939Host"
  WriteRegStr HKCU "${UNINST}" "DisplayName" "939 Host"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "939 Host"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\Desinstalar.exe"'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1

  ; WebView2 viene con Windows 10/11 actualizados. Si falta, se avisa (no se descarga nada solo).
  ReadRegStr $1 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ReadRegStr $2 HKCU "Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" "pv"
  ${If} $1 == ""
  ${AndIf} $2 == ""
    MessageBox MB_YESNO|MB_ICONINFORMATION "939 Host necesita Microsoft Edge WebView2, que no se encontró en esta PC. ¿Abrir la página oficial de descarga?" /SD IDNO IDNO +2
      ExecShell "open" "https://developer.microsoft.com/microsoft-edge/webview2/"
  ${EndIf}
SectionEnd

Section "Uninstall"
  ; Solo se quita el programa. Los servidores, mundos y copias (en %LOCALAPPDATA%\939Host) quedan.
  Delete "$INSTDIR\${APPEXE}"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LEEME-INSTALACION.txt"
  Delete "$INSTDIR\PRIVACIDAD.md"
  RMDir /r "$INSTDIR\licencias"
  Delete "$INSTDIR\Desinstalar.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\939 Host\939 Host.lnk"
  Delete "$SMPROGRAMS\939 Host\Desinstalar 939 Host.lnk"
  RMDir "$SMPROGRAMS\939 Host"
  Delete "$DESKTOP\939 Host.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\939Host"
  DeleteRegKey HKCU "Software\939 Host"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "939 Host"
  MessageBox MB_OK "939 Host se desinstaló. Tus servidores y copias siguen en $LOCALAPPDATA\939Host (podés borrarlos a mano si ya no los querés)." /SD IDOK
SectionEnd
