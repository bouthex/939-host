# Arquitectura y decisiones — 939 Host 0.9

## Piezas

```
ui (JS) ── Tauri ── src-tauri ──┬── crates/core     servidores, loaders, mods, jugadores, puntos, permiso de anfitrión
                                ├── crates/cluster  nodo del grupo: control, sincronización, traspaso, ruteo
                                └── crates/p2p      punto final iroh (QUIC cifrado), tickets
```

`core` no depende de la interfaz: casi todo se prueba sin Windows. `cluster` habla con la app a través del trait `LocalHost` (encender, apagar, tomar punto, puntaje), así el mismo código corre en la app y en `939node`, el nodo sin interfaz de las pruebas.

## Servidor lógico

Cada servidor compartido tiene en cada PC: `share.json` (integrantes y opciones), `lease.json` (permiso) y el almacén de puntos `restore/` (bloques SHA-256 + índices).

**Por qué un permiso con épocas y no un "candado" central:** no hay servidor central ni una PC siempre encendida. La época es un número que solo crece y viaja con cada consulta. Cualquier PC que vea una época mayor que la suya la adopta; si estaba hosteando con una menor, se detiene. Así, aunque dos PCs se hayan creído anfitrionas durante un corte, al reconectarse queda una sola y la otra conserva su copia como punto de seguridad.

**Recuperación por mayoría:** la PC candidata (estado más reciente, después mejor hardware, después id menor: todas eligen igual) pide votos. Un voto se da solo si quien vota tampoco ve a la anfitriona hace un rato, conoce la misma época, no votó a otra en esa época y la candidata no tiene un estado más viejo que el suyo. Con mayoría, la candidata sube la época con `claim(época_esperada)`, que falla si otra se adelantó.

**Aislamiento:** la anfitriona que pierde contacto con un número de PCs que podría formar mayoría sin ella (y que estaban conectadas hace poco) se detiene antes de que las demás puedan elegir otra (20 s contra 35 s). Si vuelve el contacto y nadie tomó el permiso, se reanuda sola.

**Soltar con "estado requerido":** al soltar, el permiso registra el punto `(época, orden)` que tiene que tener la próxima anfitriona. Nadie enciende con una copia más vieja salvo con «Recuperar aquí», que es explícito.

**Traspaso:** apagar → punto final → enviar el permiso nuevo y el id del punto a la destino → la destino adopta, copia lo que falta, coloca el estado, prepara (Java/loader) y enciende. Si el envío falla, la origen vuelve a encender; si la respuesta se pierde, se pregunta a la destino qué permiso tiene antes de decidir.

## Sincronización

Bloques de 64 KiB direccionados por contenido. Un índice (JSON comprimido) lista los archivos y sus bloques. Quien recibe calcula los faltantes, los pide en tandas de 128, verifica cada uno y recién con todos registra el índice. Colocar un punto usa una carpeta temporal y mueve lo actual a la Papelera: nunca hay un estado a medio escribir en la carpeta del servidor. Los índices recibidos se validan (sin `..`, solo carpetas `world/`, `mods/`, `config/` y archivos de configuración conocidos).

## Red

iroh 1.x: cada PC tiene una identidad ed25519 (la misma que su id de nodo, guardada con DPAPI). Protocolos (ALPN): control, sincronización, Minecraft, puente, unión. Solo integrantes del grupo pasan. Con relays desactivados se usa el preset mínimo de iroh (directo); con relays, el preset de n0 (descubrimiento y hole punching con relay de respaldo). Las direcciones conocidas de cada PC se guardan y se comparten entre integrantes para reencontrarse aunque cambie la IP.

## Decisiones con costo

- **Mayoría y no "el primero que llega"**: con dos PCs no hay recuperación automática. Se prefirió no arriesgar dos mundos encendidos.
- **Pérdida acotada en caídas**: no se replica en vivo (sería pesado y frágil); se replican puntos cada 15 min. En traspasos y al soltar la pérdida es cero.
- **Reconexión con un mod existente** (AutoReconnect Reforged, LGPL) en vez de un mod propio: está mantenido para 26.2 y se instala desde Modrinth sin redistribuirlo.
- **Fabric plan B desde Maven**: el servicio meta de Fabric es un punto único; el repositorio Maven es la fuente original de las mismas librerías.
