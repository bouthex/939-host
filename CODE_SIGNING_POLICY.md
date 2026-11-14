# Code signing policy (política de firma de código)

> **Estado actual (14/11/2026): ninguna versión de 939 Host está firmada.**
> Se solicitó la firma gratuita para proyectos open source de SignPath Foundation. Esta política entra
> en vigor cuando la solicitud se apruebe; hasta entonces describe cómo se va a firmar.

Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/)
*(solicitado; vale para las versiones firmadas una vez que SignPath Foundation apruebe el proyecto).*

## Qué se firma

| Archivo | Firma |
|---|---|
| `939Host-<versión>-instalador.exe` | SignPath Foundation (al aprobarse) |
| `939Host.exe` (dentro del instalador y del portable) | SignPath Foundation (al aprobarse) |
| `Desinstalar.exe` (dentro del instalador) | SignPath Foundation (al aprobarse) |
| `WebView2Loader.dll` | Ya viene firmado por Microsoft; se distribuye sin modificar y no se vuelve a firmar |

Solo se firman binarios compilados a partir de este repositorio por el workflow público
[`release-firmada.yml`](.github/workflows/release-firmada.yml), en runners alojados por GitHub, a partir
de una etiqueta de versión. No se firman binarios de terceros ni compilaciones locales.

## Equipo y roles

| Rol | Miembros | Responsabilidad |
|---|---|---|
| Autores | [@bouthex](https://github.com/bouthex) | Escriben el código. Las contribuciones externas entran por pull request |
| Revisores | [@bouthex](https://github.com/bouthex) | Revisan y aprueban cada pull request antes de integrarlo a `main` |
| Aprobadores | [@bouthex](https://github.com/bouthex) | Aprueban manualmente cada solicitud de firma en SignPath |

Todos los miembros usan autenticación de dos factores en GitHub y en SignPath.

## Proceso de firma

1. Los cambios entran a `main` por pull request revisado; la CI tiene que pasar.
2. Se publica la versión con una etiqueta `vX.Y.Z`; el workflow *Release* la compila y la publica sin firmar.
3. Un aprobador ejecuta *Release firmada (SignPath)* para esa etiqueta.
4. SignPath verifica que el artefacto venga de ese workflow del repositorio y espera la **aprobación
   manual** del aprobador.
5. Con la firma aprobada, el workflow verifica las firmas y reemplaza los archivos de la release.

Cada solicitud de firma queda registrada en SignPath y en el historial de GitHub Actions.

## Privacidad

939 Host no envía información a otros sistemas en red salvo lo descrito en la
[política de privacidad](PRIVACY.md): descargas que pide el usuario (Mojang, Adoptium, loaders,
Modrinth), las PCs de su grupo y, si está activado, los relays públicos de iroh, que se pueden
desactivar en *Configuración*. La política se muestra durante la instalación.

## Reportar un problema

Si encontrás un binario firmado con este certificado que no salió de este proceso, reportalo en privado
según [SECURITY.md](SECURITY.md).
