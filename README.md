<p align="center"><img src="src-tauri/icons/128x128.png" width="96" alt="Ícono de 939 Host"></p>

<h1 align="center">939 Host</h1>

<p align="center">
Servidores de Minecraft Java compartidos entre las PCs de un grupo de amigos.<br>
Sin hosting pago, sin VPS y sin una PC que tenga que estar siempre encendida.
</p>

<p align="center">
<a href="https://github.com/bouthex/939-host/releases/latest"><b>Descargar para Windows</b></a> ·
<a href="docs/MANUAL.md">Manual</a> ·
<a href="docs/PROBLEMAS.md">Solución de problemas</a> ·
<a href="CHANGELOG.md">Cambios</a> ·
<a href="CODE_SIGNING_POLICY.md">Code signing policy</a> ·
<a href="PRIVACY.md">Privacidad</a>
</p>

<p align="center">
<a href="https://github.com/bouthex/939-host/actions/workflows/ci.yml"><img src="https://github.com/bouthex/939-host/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/licencia-MIT-black" alt="Licencia MIT"></a>
</p>

---

## Qué hace

- **Crea servidores** Vanilla, Fabric, Forge o NeoForge con un asistente. Descarga sola la versión de
  Minecraft y el Java que haga falta.
- **Mods de Modrinth** con dependencias, actualizaciones y desinstalación segura. También importa mods
  de una carpeta.
- **Consola, jugadores y permisos**: lista blanca, operadores, baneos, rendimiento (MSPT) y registros.
- **Puntos de restauración incrementales** del mundo, los mods y la configuración, y copias `.zip`.
- **Un servidor, varias PCs**: el servidor es del grupo, no de una PC. Cualquier coanfitriona puede
  hostearlo; se pasa de una PC a otra sin perder progreso, y si la anfitriona se apaga, otra lo
  retoma sola (con tres o más PCs).
- **Una sola dirección para jugar**, que sigue a la PC que lo esté hosteando. Sin abrir puertos:
  conexión directa cifrada, hole punching, otra PC del grupo como puente y, solo como último recurso,
  relays públicos gratuitos.

## Instalar

1. Descargá **`939Host-<versión>-instalador.exe`** de la
   [última versión](https://github.com/bouthex/939-host/releases/latest). No hace falta cuenta de GitHub.
2. Ejecutalo. Se instala para tu usuario, sin pedir permisos de administrador.
3. Abrí **939 Host** desde el menú Inicio.

¿Preferís no instalar nada? Bajá el `.zip` portable, descomprimilo y abrí `939Host.exe`.

**Requisitos:** Windows 10 u 11 de 64 bits y Microsoft Edge WebView2 (ya viene en Windows 11 y en
Windows 10 actualizado). Más detalles, verificación de descargas y firma digital en
[docs/INSTALACION.md](docs/INSTALACION.md).

> **Estado de la firma:** la versión 0.9.0 **todavía no está firmada**. En Windows 11 con
> Smart App Control activado, Windows la bloquea. Pedimos la firma gratuita de
> [SignPath Foundation](https://signpath.org/); cuando se apruebe, las descargas se reemplazan por
> versiones firmadas. No recomendamos desactivar protecciones de Windows.

## Documentación

| Documento | Para qué |
|---|---|
| [Manual](docs/MANUAL.md) | Uso completo: servidores, mods, copias, grupo, traspasos |
| [Instalación](docs/INSTALACION.md) | Descargar, verificar, actualizar, desinstalar |
| [Solución de problemas](docs/PROBLEMAS.md) | Errores comunes y cómo pedir ayuda |
| [Limitaciones](docs/LIMITACIONES.md) | Lo que todavía no hace o no se pudo probar |
| [Compilar](docs/COMPILAR.md) | Compilar desde el código, pruebas, publicar versiones |
| [Arquitectura](docs/ARQUITECTURA.md) | Cómo funciona por dentro |
| [Pruebas](docs/PRUEBAS.md) | Qué se probó y con qué resultados |
| [Privacidad](PRIVACY.md) | Qué conexiones hace la app y cómo desactivarlas |
| [Code signing policy](CODE_SIGNING_POLICY.md) | Cómo se firman las versiones, equipo y roles |

## Privacidad

Sin telemetría, sin analíticas y sin cuentas. 939 Host solo se conecta para descargar lo que pedís
(Minecraft, Java, loaders, mods), para hablar con las PCs de tu grupo y, si lo dejás activado, con los
relays públicos de iroh. Detalle en [PRIVACY.md](PRIVACY.md).

## Código

| Carpeta | Contenido |
|---|---|
| `crates/core` | Servidores, loaders, mods, jugadores, puntos de restauración, permiso de anfitrión, hardware, identidad |
| `crates/p2p` | Conexión P2P cifrada (iroh / QUIC), invitaciones |
| `crates/cluster` | Servidor lógico entre PCs: sincronización, traspaso, recuperación, ruteo de jugadores |
| `src-tauri`, `ui` | App de escritorio (Tauri 2, interfaz en JavaScript sin frameworks) |
| `installer` | Instalador NSIS por usuario |
| `scripts` | Compilación, empaquetado y verificación |
| `.github/workflows` | CI, publicación de versiones y firma con SignPath |

```bash
cargo test --locked -p host939-core -p host939-p2p -p host939-cluster
```

## Contribuir y seguridad

- Cómo colaborar: [CONTRIBUTING.md](CONTRIBUTING.md)
- Reportar una vulnerabilidad: [SECURITY.md](SECURITY.md) (en privado, no en un issue público)

## Licencia

[MIT](LICENSE). Las dependencias usan licencias permisivas compatibles; el detalle se incluye en cada
descarga (`licencias/`).

939 Host no es un producto oficial de Minecraft ni está aprobado por Mojang o Microsoft.
Minecraft es una marca de Mojang AB.
