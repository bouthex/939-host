# Política de privacidad de 939 Host

Última revisión: 15 de noviembre de 2026 (rama de Diseño v3, todavía sin publicar).

939 Host **no tiene telemetría, analíticas, publicidad ni cuentas**, y no envía datos a servidores del
proyecto (no existen). Todo lo que guarda queda en tu PC, en `%LOCALAPPDATA%\939Host`.

## Conexiones de red que hace la app

Esta lista sale de revisar el código fuente. Si agregamos una conexión nueva, la documentamos acá antes
de publicar la versión.

| Destino | Cuándo | Qué se envía |
|---|---|---|
| Mojang: `piston-meta.mojang.com`, `piston-data.mojang.com` | Al crear un servidor o descargar una versión | Pedido de la lista de versiones y del `server.jar` |
| Mojang: `api.mojang.com`, `sessionserver.mojang.com`, `textures.minecraft.net` | Al mostrar jugadores, solo si **Skins reales de los jugadores** está activado (viene activado) | El nombre del jugador, para obtener su skin. Se guarda 24 h en tu PC |
| Mojang: `piston-meta.mojang.com`, `piston-data.mojang.com`, `resources.download.minecraft.net` | La primera vez que abrís Jugadores en una versión, solo si **Imágenes originales del juego** está activado (viene activado) | Pedido del cliente oficial de esa versión (se verifica su SHA-1) y del idioma español. Se extraen solo los íconos de ítems, corazones, efectos y casilleros; el resto se borra. No se comparte con nadie |
| Mojang: `api.mojang.com` | Al agregar a la lista blanca, operar o banear a un jugador que el servidor todavía no conoce, **solo con el servidor en modo online** | El nombre de ese jugador, para obtener su UUID |
| Adoptium: `api.adoptium.net` y su CDN | Si no hay un Java compatible instalado | Versión de Java y sistema operativo pedidos |
| Fabric: `meta.fabricmc.net`, `maven.fabricmc.net` | Al crear un servidor Fabric | Pedidos de descarga |
| Forge: `files.minecraftforge.net`, `maven.minecraftforge.net` | Al crear un servidor Forge | Pedidos de descarga |
| NeoForge: `maven.neoforged.net` | Al crear un servidor NeoForge | Pedidos de descarga |
| Modrinth: `api.modrinth.com` | Al buscar, instalar, actualizar o vincular mods, al analizar un servidor con el asistente de reparación y al activar la reconexión automática del cliente | Texto de búsqueda y categorías, versión de Minecraft y loader, hashes de los mods instalados |
| Modrinth: `cdn.modrinth.com` | Al mostrar íconos y capturas de mods, solo si **Íconos y capturas de los mods** está activado (viene activado) | Pedido de cada imagen |
| Tu router (UPnP / NAT-PMP, red local) | Mientras compartís o estás unido a un servidor | Pedido de abrir un puerto UDP para la conexión directa |
| PCs de tu grupo (iroh / QUIC, cifrado de extremo a extremo) | Mientras compartís un servidor o estás unido a uno | Mundo, mods y configuración del servidor compartido; estado del grupo; nombre de la PC; tráfico de Minecraft. Solo con las PCs que aceptaron una invitación |
| Relays y descubrimiento públicos de iroh (number 0) | Solo si **Permitir relays públicos gratuitos** está activado (viene activado) | La clave pública del nodo y sus direcciones de red, para que el grupo se encuentre; si no hay conexión directa, paquetes **cifrados** que el relay no puede leer |

Las descargas de Mojang, Adoptium, loaders y Modrinth también exponen tu dirección IP a esos servicios,
como cualquier descarga.

### Políticas de privacidad de terceros

| Servicio | Política |
|---|---|
| Mojang / Microsoft | https://privacy.microsoft.com/privacystatement |
| Adoptium (Eclipse Foundation) | https://www.eclipse.org/legal/privacy/ |
| Modrinth (Rinth, Inc.) | https://modrinth.com/legal/privacy |
| iroh relays y descubrimiento (n0) | https://iroh.computer/legal |
| Fabric, Forge, NeoForge | Repositorios Maven públicos; solo reciben pedidos de descarga |

## Cómo desactivar lo que no quieras

- **Skins, imágenes del juego e imágenes de mods:** Configuración → Privacidad y conexiones. Apagadas,
  se muestran iniciales, nombres y formas simples. **Borrar imágenes descargadas** elimina la caché.
- **Relays públicos:** Configuración → Privacidad y conexiones → desmarcá **Permitir relays públicos gratuitos** (se aplica al
  reiniciar). Sin relays solo queda la conexión directa o por otra PC del grupo.
- **Compartir:** dejá de compartir el servidor o salí del grupo; la app deja de conectarse con otras PCs.
- **Puente:** Configuración → Red → desmarcá **Puede hacer de puente para otras PCs del grupo**.
- Las descargas solo ocurren cuando creás un servidor, instalás o actualizás algo.

## Datos de jugadores

El perfil de un jugador muestra solo lo que guarda el propio servidor: con el jugador conectado, 939 Host
le pide los datos al servidor por consola (cada 4 s, solo mientras el perfil está abierto y la ventana
visible; la consulta no aparece en la consola); desconectado, lee el último guardado de Minecraft.
Nada de esto sale de tu PC.

## Asistente de reparación

Funciona con reglas locales (sin IA ni servicios externos, salvo Modrinth para buscar versiones
compatibles). Antes de cambiar algo crea un punto de restauración en tu PC.

## Informes de diagnóstico

**Generar informe** crea un `.zip` **local** con registros y configuración, sin el mundo, sin claves,
sin invitaciones y sin la identidad del nodo. La app no lo envía a ningún lado; lo compartís vos si querés.

## Desinstalación

El desinstalador quita la app y sus accesos directos. Tus servidores, mundos y copias en
`%LOCALAPPDATA%\939Host` se conservan; podés borrarlos a mano.

## Contacto

Dudas sobre privacidad: abrí un issue en https://github.com/bouthex/939-host/issues
(o un aviso privado de seguridad, ver [SECURITY.md](SECURITY.md)).
