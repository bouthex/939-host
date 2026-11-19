; Instalador de 939 Host (por usuario, sin permisos de administrador)
Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "StrFunc.nsh"
!include "FileFunc.nsh"
${StrStr}
${UnStrStr}

; Script fijo que quita solo el bloque de 939 Host del archivo hosts (ver alias::HOSTS_CLEAN_SCRIPT;
; la prueba desinstalador_quita_solo_el_bloque verifica que este texto sea exactamente ese script).
!define HOSTS_CLEAN_B64 "JABwAD0AIgAkAGUAbgB2ADoAUwB5AHMAdABlAG0AUgBvAG8AdABcAFMAeQBzAHQAZQBtADMAMgBcAGQAcgBpAHYAZQByAHMAXABlAHQAYwBcAGgAbwBzAHQAcwAiADsAJABvAD0AWwBJAE8ALgBGAGkAbABlAF0AOgA6AFIAZQBhAGQAQQBsAGwAVABlAHgAdAAoACQAcAApADsAJABuAD0AJABvACAALQByAGUAcABsAGEAYwBlACAAJwAoAD8AcwApACgAXAByAD8AXABuACkAKgAjACAAPgA+AD4AIAA5ADMAOQAgAEgAbwBzAHQAOgAgAGQAaQByAGUAYwBjAGkAbwBuAGUAcwAgAGQAZQAgAHMAZQByAHYAaQBkAG8AcgBlAHMAIABcACgAbgBvACAAZQBkAGkAdABhAHIAIABhACAAbQBhAG4AbwBcACkALgAqAD8AIwAgADwAPAA8ACAAOQAzADkAIABIAG8AcwB0AFwAcgA/AFwAbgA/ACcALAAnACcAOwBpAGYAKAAkAG4AIAAtAGMAbgBlACAAJABvACkAewBbAEkATwAuAEYAaQBsAGUAXQA6ADoAVwByAGkAdABlAEEAbABsAFQAZQB4AHQAKAAkAHAALAAkAG4AKQB9AA=="

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
  ; 939 Host abierto (también en la bandeja, sin ventana): pedir que se cierre antes de actualizar.
  ; Se busca el proceso: con un servidor encendido, 939 Host puede seguir sin ventana.
  nsExec::ExecToStack '"$SYSDIR\tasklist.exe" /FI "IMAGENAME eq ${APPEXE}" /NH'
  Pop $0
  Pop $1
  ${StrStr} $2 $1 "${APPEXE}"
  ${If} $2 != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "939 Host está abierto (puede estar en la bandeja, junto al reloj). Cerralo con «Salir de 939 Host…» (así guarda y pasa los servidores) y volvé a ejecutar el instalador." /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

Var DeleteData

Function un.onInit
  nsExec::ExecToStack '"$SYSDIR\tasklist.exe" /FI "IMAGENAME eq ${APPEXE}" /NH'
  Pop $0
  Pop $1
  ${UnStrStr} $2 $1 "${APPEXE}"
  ${If} $2 != ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "939 Host está abierto (puede estar en la bandeja, junto al reloj). Cerralo con «Salir de 939 Host…» para que guarde los servidores y volvé a desinstalar." /SD IDOK
    Abort
  ${EndIf}
  ; Por defecto (y en modo silencioso) se conservan servidores, mundos y copias. Para borrarlos
  ; sin preguntas hay que pedirlo de forma explícita: Desinstalar.exe /S /BORRARDATOS
  StrCpy $DeleteData "0"
  ${If} ${Silent}
    ${un.GetParameters} $R0
    ClearErrors
    ${un.GetOptions} $R0 "/BORRARDATOS" $R1
    ${IfNot} ${Errors}
      StrCpy $DeleteData "1"
    ${EndIf}
    Goto keep
  ${EndIf}
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "¿Querés borrar también los datos de 939 Host?$\r$\n$\r$\nNO (recomendado): se quita solo el programa. Tus servidores, mundos, copias de seguridad y configuración quedan en $LOCALAPPDATA\939Host.$\r$\n$\r$\nSÍ: se borran también esos datos." /SD IDNO IDNO keep
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "Se va a borrar para siempre la carpeta $LOCALAPPDATA\939Host, que contiene:$\r$\n$\r$\n• todos tus servidores y sus MUNDOS$\r$\n• las copias de seguridad y los puntos de restauración$\r$\n• la Papelera de 939 Host$\r$\n• la identidad de esta PC en los grupos, amigos y mensajes$\r$\n• la configuración, Java descargado por 939 Host y las cachés$\r$\n$\r$\nNo se puede deshacer. ¿Borrar todo?" /SD IDNO IDNO keep
      StrCpy $DeleteData "1"
  keep:
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
  ; Se quita todo lo que instaló o configuró 939 Host fuera de su carpeta de datos. Los servidores,
  ; mundos y copias (en %LOCALAPPDATA%\939Host) quedan, salvo que se haya elegido borrarlos.
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
  ; Caché de la interfaz (WebView2): no tiene datos de servidores.
  RMDir /r "$LOCALAPPDATA\ar.host939.app"
  ; Direcciones .939host: si el archivo hosts tiene el bloque de 939 Host, se quita (pide permiso
  ; de administrador para «Windows PowerShell», igual que al crearlas). No se toca nada más.
  ClearErrors
  FileOpen $3 "$WINDIR\System32\drivers\etc\hosts" r
  StrCpy $4 ""
  ${IfNot} ${Errors}
    ${Do}
      FileRead $3 $5
      ${If} ${Errors}
        ${ExitDo}
      ${EndIf}
      ${UnStrStr} $6 $5 "# >>> 939 Host"
      ${If} $6 != ""
        StrCpy $4 "1"
        ${ExitDo}
      ${EndIf}
    ${Loop}
    FileClose $3
  ${EndIf}
  ${If} $4 == "1"
    MessageBox MB_OK|MB_ICONINFORMATION "Para quitar las direcciones .939host del archivo hosts, Windows va a pedir permiso de administrador para «Windows PowerShell». Si no lo aceptás, quedan esas líneas (no afectan a nada sin 939 Host)." /SD IDOK
    ExecShellWait "runas" "$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -EncodedCommand ${HOSTS_CLEAN_B64}" SW_HIDE
  ${EndIf}
  ${If} $DeleteData == "1"
    RMDir /r "$LOCALAPPDATA\939Host"
    MessageBox MB_OK "939 Host y todos sus datos se desinstalaron." /SD IDOK
  ${Else}
    MessageBox MB_OK "939 Host se desinstaló. Tus servidores, mundos y copias siguen en $LOCALAPPDATA\939Host." /SD IDOK
  ${EndIf}
SectionEnd
