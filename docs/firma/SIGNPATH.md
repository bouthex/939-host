# Firma gratuita con SignPath Foundation

Objetivo: que el instalador, `939Host.exe` y el desinstalador lleven una firma Authenticode de una CA del
Microsoft Trusted Root Program. Smart App Control (Windows 11) deja correr software sin reputación si está
firmado así. SignPath Foundation la ofrece **gratis** a proyectos open source; el editor que muestra
Windows es **SignPath Foundation**.

Requisitos revisados el 14/11/2026 contra <https://signpath.org/terms> y
<https://docs.signpath.io/trusted-build-systems/github>.

## Cumplimiento

| Requisito de SignPath Foundation | Estado | Cómo se cumple |
|---|---|---|
| Licencia OSI en todo el código, sin licencia dual comercial | ✅ | [MIT](../../LICENSE). Dependencias: MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode-3.0, CDLA-Permissive-2.0 (datos), fuentes OFL ([THIRD_PARTY_LICENSES.html](../../THIRD_PARTY_LICENSES.html)) |
| Sin componentes propietarios (se permiten librerías del sistema) | ✅ | `WebView2Loader.dll` es del SDK de WebView2 de Microsoft, licencia BSD-3-Clause, firmado por Microsoft y **no** se vuelve a firmar |
| Mantenido activamente | ✅ | Historial público, CHANGELOG, issues abiertos |
| Ya publicado en la forma a firmar, con página de descarga que documente la función | ✅ al publicar 0.9.0 | Release 0.9.0 (sin firmar) en GitHub con instalador y portable; README |
| Build automatizado y verificable desde el propio código | ✅ | [`release-firmada.yml`](../../.github/workflows/release-firmada.yml): compila desde la etiqueta, todo en runners alojados por GitHub |
| Metadatos de producto y versión fijos | ✅ | `.signpath/*.xml` exige `product-name="939 Host"` y la versión de la etiqueta; el `.exe` y el instalador los traen |
| Roles: autores, revisores, aprobadores; el equipo es dueño del repo | ✅ | [CODE_SIGNING_POLICY.md](../../CODE_SIGNING_POLICY.md): @bouthex en los tres roles |
| MFA para todos en GitHub y SignPath | ⚠ **Te toca** | Confirmar 2FA en la cuenta de GitHub y activarlo en SignPath |
| Privacidad: política, visible durante la instalación, opción para desactivar | ✅ | [PRIVACY.md](../../PRIVACY.md); resumen en la página del instalador y `PRIVACIDAD.md` instalado; relays desactivables |
| Sección "Code signing policy" en la página principal y de descarga | ✅ | Enlazada desde el README y las notas de cada release |
| Atribución "Free code signing provided by SignPath.io, certificate by SignPath Foundation" | ✅ preparada | Comentada en `CODE_SIGNING_POLICY.md`; las notas de la release firmada la incluyen automáticamente |
| Sin malware/PUA ni herramientas de hacking; no cambia el sistema sin avisar; desinstalable | ✅ | Instalación por usuario sin administrador, desinstalador, inicio con Windows opcional y visible |
| Aprobación manual de cada firma | ✅ | El workflow espera la aprobación en SignPath (hasta 24 h) |
| Reputación del proyecto | ⚠ Riesgo | Proyecto chico y nuevo; SignPath decide a su criterio |

## Integración técnica (ya en el repositorio)

| Pieza | Valor esperado en SignPath |
|---|---|
| Proyecto | slug `939-host`, vinculado al repositorio `bouthex/939-host` |
| Trusted Build System | *GitHub.com* (predefinido), vinculado al proyecto |
| Política de firma | slug `release-signing`, certificado de SignPath Foundation, aprobación manual |
| Configuración de artefacto `binarios` | contenido de [`.signpath/artifact-configuration-binarios.xml`](../../.signpath/artifact-configuration-binarios.xml) |
| Configuración de artefacto `instalador` | contenido de [`.signpath/artifact-configuration-instalador.xml`](../../.signpath/artifact-configuration-instalador.xml) |
| Parámetro de usuario | `version` (texto), lo envía el workflow |
| Secreto de GitHub | `SIGNPATH_API_TOKEN` (token de un usuario con rol *submitter*) |
| Variable de GitHub | `SIGNPATH_ORGANIZATION_ID` |
| Entorno de GitHub | `firma` (opcional: exigir tu aprobación también en GitHub) |

Permisos mínimos: el workflow tiene `contents: read` por defecto; el trabajo de firma agrega solo
`actions: read` (para que SignPath descargue el artefacto) y únicamente el trabajo final tiene
`contents: write` para reemplazar los archivos de la release.

## Pasos que dependen de vos

1. **2FA en GitHub**: <https://github.com/settings/security> → *Two-factor authentication* (si no está activo).
2. **Solicitud**: completá el formulario de <https://signpath.org/apply> con las respuestas de abajo.
3. **Cuando aprueben** (SignPath crea la organización y te invita):
   1. Entrá a SignPath y activá 2FA.
   2. Instalá la *SignPath GitHub App* en el repositorio `bouthex/939-host` si te lo piden.
   3. Revisá/creá el proyecto, la política y las dos configuraciones de artefacto de la tabla anterior.
   4. Creá un usuario de CI (o usá el tuyo) con rol *submitter* y generá un API token.
   5. En GitHub → *Settings → Secrets and variables → Actions*: secreto `SIGNPATH_API_TOKEN` y variable
      `SIGNPATH_ORGANIZATION_ID`. **No me pases el token**: cargalo vos.
   6. *Actions → Release firmada (SignPath) → Run workflow* con `v0.9.0`, y aprobá la firma en SignPath
      (dos solicitudes: binarios e instalador).
   7. Descomentá la atribución en `CODE_SIGNING_POLICY.md`.

## Respuestas para el formulario

- **Project name:** 939 Host
- **Repository URL:** https://github.com/bouthex/939-host
- **Homepage / download page:** https://github.com/bouthex/939-host (README) · https://github.com/bouthex/939-host/releases
- **License:** MIT
- **Code signing policy URL:** https://github.com/bouthex/939-host/blob/main/CODE_SIGNING_POLICY.md
- **Privacy policy URL:** https://github.com/bouthex/939-host/blob/main/PRIVACY.md
- **Build system:** GitHub Actions on GitHub-hosted runners (`.github/workflows/release-firmada.yml`)
- **Artifacts to sign:** `939Host.exe` (Rust/Tauri desktop app), `Desinstalar.exe` and `939Host-<version>-instalador.exe` (NSIS per-user installer). `WebView2Loader.dll` is Microsoft-signed (BSD-3-Clause, WebView2 SDK) and is not re-signed.
- **Description (EN):** 939 Host is a free Windows desktop app to create, manage and share Minecraft Java servers between a group of friends' PCs, without paid hosting. A server belongs to the group: it can move between PCs (manually or automatically on failure) without losing progress, using lease epochs to guarantee a single writer, incremental SHA-256-verified sync and end-to-end encrypted P2P networking (iroh/QUIC). It supports Vanilla, Fabric, Forge and NeoForge, Modrinth mods, incremental restore points, players and permissions. No telemetry, no accounts.
- **Why code signing:** Windows 11 Smart App Control blocks unsigned executables without reputation and offers no override, so our users cannot run the app without disabling a Windows protection, which we don't want to ask of them.
- **Team / roles:** @bouthex — author, reviewer and approver; MFA enabled.
- **Release so far:** v0.9.0 (unsigned) published on GitHub Releases.

## Limitaciones verificadas

- La aceptación es discrecional; SignPath puede pausar o revocar el acceso si se incumplen las reglas.
- El editor visible es "SignPath Foundation", no "939 Host".
- Cada versión firmada requiere aprobación manual.
- SmartScreen puede seguir avisando en versiones nuevas hasta que el certificado acumule reputación;
  Smart App Control acepta la firma de una CA del Trusted Root Program cuando no puede predecir.
