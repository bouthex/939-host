# Registro de cambios

## 0.9.0 — 14/11/2026

Primera versión pública.

### Publicación
- Código abierto bajo licencia **MIT**, con licencias de terceros incluidas en cada descarga.
- Compilación, pruebas y publicación automáticas en GitHub Actions; instalador y portable con
  SHA-256 y atestación de procedencia.
- Integración preparada para la firma gratuita de SignPath Foundation (pendiente de aprobación;
  esta versión **no está firmada**).
- Política de privacidad, de seguridad y de firma de código; guía de instalación, compilación y
  solución de problemas.
- El instalador muestra el resumen de privacidad e instala `PRIVACIDAD.md`.

### Servidor compartido entre PCs
- Nuevo módulo `crates/cluster`: un nodo P2P por PC mantiene cada servidor compartido.
- **Permiso de anfitrión con épocas** (`lease`): una sola PC puede encender; la época solo crece; quien ve una época mayor se detiene.
- **Sincronización verificada**: solo viajan los bloques de 64 KiB que faltan, cada uno comprobado por SHA-256; un punto a medio copiar nunca se registra.
- **Traspaso planificado** («Pasar a…»), **Guardar y soltar** (exige que la próxima PC tenga el último punto) y **traspaso automático al salir**.
- **Recuperación automática por mayoría** (elige el estado más reciente y el mejor hardware) y **aislamiento** de la anfitriona que pierde contacto o vuelve con época vieja; su copia queda como punto de seguridad.
- **Recuperar aquí** para grupos de dos PCs.
- **Dirección fija para jugar** (`127.0.0.1:<puerto>`) que sigue a la anfitriona; puente por otra PC si no hay camino directo; respuesta «se está mudando» / «nadie lo está hosteando» en la lista de servidores.
- Invitaciones de un solo uso con rol (coanfitriona o jugador); la coanfitriona recibe la configuración y se instala sola.
- Los puntos de restauración ahora incluyen `mods/` y `config/`.
- `server.properties` se regenera desde la configuración en cada inicio (el puerto de cada PC se respeta tras un traspaso).

### Servidores
- **Forge y NeoForge**: elección de versión, instalador oficial, arranque por archivo de argumentos.
- **Fabric plan B**: si el servicio de Fabric no responde, se instala desde su repositorio Maven con verificación SHA-1.
- **Jugadores y permisos**: lista blanca, operadores, baneos, expulsar (pestaña Jugadores).
- **Mundo nuevo** seguro (deja punto de restauración) e **informe de diagnóstico** (.zip sin mundo ni claves).
- **Importar mods de una carpeta**: omite los solo de cliente salvo que otro mod los necesite; elige el más nuevo entre duplicados; no toca el origen.
- Lectura de metadatos de los jar sin internet (lado cliente/servidor, duplicados por id).

### Jugadores
- **Reconexión automática**: instala AutoReconnect (Reforged) en el perfil de Minecraft elegido, solo agregando archivos; lo repone si se borra.

### Otros
- Instalador NSIS por usuario (sin administrador), accesos directos, desinstalador que conserva los datos.
- Ajustes honestos: se quitaron opciones que no tenían efecto; «puede hacer de puente» ahora funciona.
- Pruebas multi-proceso del protocolo de grupo y de traspaso con Minecraft real.

## 0.2.0
- Mods de Modrinth con dependencias, puntos de restauración incrementales, compartir por P2P con invitaciones, inicio con Windows.

## 0.1.0
- Primera versión: servidores Vanilla y Fabric, consola, ajustes, copias, evaluación de hardware.
