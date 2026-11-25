# Plan de integración de las nuevas funciones

Revisión de la arquitectura actual (rama `rediseno-v3`, 15/11/2026) y cómo encaja cada pedido.
Estado: **hecho** = implementado y probado · **en curso** = parcialmente · **pendiente** = diseñado.

## Arquitectura de partida

- `crates/core`: servidores (instancias, instalación, proceso Java, propiedades, mods, Modrinth,
  copias, puntos incrementales, jugadores, reparación). Todo en `%LOCALAPPDATA%\939Host`.
- `crates/p2p` + `crates/cluster`: identidad de nodo (clave ed25519 de iroh), grupo por servidor
  (`share.json`), permiso con épocas (un solo escritor), traspaso en dos fases, recuperación por
  mayoría, ruta local `127.0.0.1:<puerto>` hacia la PC anfitriona.
- `src-tauri` + `ui`: comandos y la interfaz.

## 1. «Error de protocolo de red» — **hecho**

Causa real (registros del servidor «Eventos»): el jugador entró con el perfil del modpack 939 Z
a un servidor Fabric 26.2 **sin mods**. Minecraft autentica al jugador y su propio juego corta a
los 1–2 s porque no puede interpretar el mundo; el servidor no registra error. No era una falla
de red de 939 Host.

Cambios: estado «Listo para jugar» solo después de una consulta real del protocolo de Minecraft;
estados «Comprobando conexión» y «Sin conexión»; detección y explicación de entradas fallidas
(mods distintos, versión, lista blanca, bloqueo, verificación de cuenta) con la línea exacta de la
consola; tarjeta «Conexión» con requisitos para entrar y estado del grupo. Prueba con cliente real.

## 2. Direcciones personalizadas — **pendiente (diseño elegido)**

Cada jugador entra siempre por 939 Host en su propia PC (ruta local), así que la dirección **no
depende de la IP de la anfitriona** y no cambia con el traspaso. Opciones evaluadas:

| Opción | Costo | Resultado | Limitación |
|---|---|---|---|
| **Archivo hosts + IP de loopback por servidor** (recomendada) | Gratis | `eventos.939host` → `127.93.0.N`, puerto 25565 (sin escribir puerto) | Pide permiso de administrador una vez por PC al crear o cambiar el nombre |
| Dominio propio (`939host.net`) con registros A/SRV estáticos a 127.93.0.N | ~USD 10–15/año | Funciona sin tocar Windows | Servicio pago (no obligatorio); requiere comprar el dominio |
| sslip.io / nip.io | Gratis | `127-93-0-12.sslip.io` | No es amigable; depende de un servicio de terceros |
| mDNS `.local` | Gratis | `eventos.local` | Resolución a loopback no garantizada en Windows |

`servidor.939host` **no funciona por sí solo**: `.939host` no existe en el DNS público. Funciona
con la opción recomendada porque Windows lo resuelve desde el archivo hosts de cada PC.
Implementación prevista: IP 127.93.x.y derivada de la identidad permanente del servidor, ruta en
esa IP:25565 (o puerto elegido), ayudante elevado que solo agrega/quita líneas marcadas
`# 939 Host`, verificación real de resolución y conexión, y deshacer al borrar el servidor.

## 3. Administrador remoto de archivos — **pendiente (diseño)**

Nuevo protocolo `939host/files/1` sobre la misma conexión P2P cifrada, solo para integrantes con
permiso (propietario y coanfitriones; jugadores sin acceso). Operaciones: listar, leer por
bloques con hash, escribir a archivo temporal + reemplazo atómico, crear carpeta, renombrar,
copiar, mover, borrar (a la Papelera de 939 Host), con progreso y cancelación. Reglas de
seguridad: rutas encerradas en la carpeta del servidor (sin `..` ni enlaces), sin acceso a claves
ni `identity/`, mundo de solo lectura mientras el servidor está encendido (se escribe con el
servidor apagado o en `world` vía punto de restauración), y todo cambio se replica como un punto
incremental para que el resto del grupo lo reciba por la sincronización existente.

## 4. Catálogo de versiones y Java — **en curso**

Hecho: todas las versiones de Mojang (releases antiguas y snapshots), Java automático según el
requisito oficial de cada versión, instalaciones independientes por versión de Java, **elección
manual de Java por servidor** limitada a versiones compatibles (Forge anterior a 1.17 solo Java 8;
Java 16 se reemplaza por 17 porque no hay JRE oficial). Pendiente: verificación cruzada
Minecraft–loader–mods antes de instalar (Modrinth ya informa versiones y loaders).

## 5. Perfil para jugar — **pendiente (diseño)**

Exportar un paquete `.mrpack` (formato abierto de Modrinth, importable en Modrinth App, Prism,
ATLauncher y otros) con: versión exacta de Minecraft y loader, mods de cliente y ambos lados con
URL y SHA-1/SHA-512 de Modrinth, dependencias, configuraciones de la carpeta `config` y archivos
agregados a mano como `overrides`. Excluye mods de solo servidor, el mundo y archivos privados.
«Instalar perfil» en el launcher oficial: crea una carpeta de juego propia (no la `.minecraft`
existente), instala el loader y agrega un perfil al launcher; se probará solo en carpetas aisladas.

## 6. Propiedad, anfitrión e integrantes — **pendiente (hallazgo de seguridad)**

Hallazgo: hoy la lista de integrantes se propaga por número de revisión y **no está firmada**;
una PC modificada podría cambiar roles. Diseño: la lista de integrantes y la propiedad pasan a ser
documentos firmados con la clave ed25519 de la PC propietaria; cada PC acepta solo cambios
firmados por el propietario vigente. Transferir la propiedad = documento firmado por el
propietario actual que nombra al nuevo. Abandonar el grupo: cualquier integrante; si es la
anfitriona, primero se traspasa; el propietario debe transferir antes de irse. Borrado definitivo:
solo el propietario, con nombre escrito y una segunda confirmación; nunca borra copias de otras
PCs sin orden firmada.

## 7. Colores funcionales y logos — **en curso**

Hecho: tonos muy desaturados por acción (Iniciar, Pausar, Reiniciar, Detener, Borrar) en íconos,
bordes y fondos al pasar el mouse; estados con punto de color; estado de carga en botones.
Pendiente: logos por servidor (colección propia con monograma 939, imagen propia con recorte,
guardado como copia dentro del servidor y replicado con el grupo).
