# Resultados de pruebas — 939 Host 0.9.0

Fecha: 14/11/2026. Entorno: Linux aislado (2 núcleos, 7 GB), Java 25 (OpenJDK 25.0.5), servidor oficial de Minecraft 26.2 descargado de Mojang. Cada "PC" en las pruebas de grupo es un proceso independiente con su propia carpeta de datos e identidad, comunicándose por P2P real (QUIC de iroh) sobre localhost, sin relays.

Todo lo que figura acá se ejecutó y pasó. Lo que no se pudo ejecutar está en la última sección.

## 1. Pruebas automáticas (sin servidor real)

`cargo test -p host939-core -p host939-p2p -p host939-cluster` → **26 pruebas, 0 fallas.**

| Prueba | Qué verifica |
|---|---|
| `snapshots::dedup_restore_and_retention` | Bloques deduplicados, restauración con punto de seguridad, retención |
| `snapshots::sync_between_two_stores_is_verified_and_complete` | Sincronización entre almacenes: un índice no se registra sin todos sus bloques; un bloque adulterado se rechaza; solo viaja lo nuevo; mods quitados se quitan; copia huérfana apartada; rutas peligrosas (`../`) rechazadas |
| `lease::epochs_only_grow_and_others_cannot_start` | Épocas que solo crecen, permiso viejo no pisa al nuevo, nadie enciende con el permiso ajeno |
| `mods::*` (6) | Plan de instalación con dependencias, incompatibles, mods solo de cliente, quitar con dependencias inversas, duplicados, desconocidos |
| `client::*` (3) | Detección de perfiles del launcher (solo lectura), mapeo de versiones, la reconexión solo agrega archivos, no duplica lo que ya está, vuelve si se borra, al desactivar quita solo lo suyo |
| `players::offline_uuid_matches_minecraft` | UUID offline idéntico al de Minecraft; nombres inválidos rechazados |
| `diagnostics::report_*` | El informe incluye registros y errores pero no la identidad ni contraseñas |
| `meta::loader_tests`, `meta::fabric_maven_tests` | Elección de versión de Forge/NeoForge, rutas de instaladores, rutas Maven |
| `process::launch_tests` | Punto de arranque por loader (jar, archivo de argumentos, Forge viejo) |
| `moving::status_and_login_get_a_clear_message` | Respuesta «se está mudando» para la lista de servidores y para el intento de entrar |
| `p2p tunnel` (2) | Miembro autorizado pasa, extraño rechazado; invitación y revocación |
| **`group_protocol_fake_servers`** (3 PCs) | Ver §3 |
| **`two_pcs_manual_recovery_and_no_stale_start`** (2 PCs) | Ver §3 |
| **`stale_copy_cannot_start_after_release`** (3 PCs) | Ver §3 |

Las tres pruebas de grupo se repitieron 3 veces seguidas sin fallas.

## 2. Servidores reales de Minecraft 26.2

| Prueba | Resultado |
|---|---|
| Ciclo completo Vanilla (`full_server_lifecycle`) | Crear → instalar (SHA-1 de Mojang) → en línea; doble inicio bloqueado; MSPT 2,1 ms (p95 2,8); copia `.zip` en caliente con save-off/save-on; 1.er punto 2,2 MB (pausa 41 ms), 2.º punto 0,6 KB (pausa 2 ms); detención limpia; puerto ocupado detectado; restauración `.zip` y por punto; borrado a Papelera. **28,6 s** |
| Jugadores y permisos | Apagado: lista blanca y operadores escritos con el formato de Minecraft (Minecraft los aceptó al encender). Encendido: banear, desbanear y quitar de la lista blanca. **Hallazgo corregido**: en modo offline, Minecraft busca el nombre en minúsculas y `whitelist remove` no encontraba al jugador; ahora la lista blanca se edita por UUID exacto y se recarga |
| Traspaso real entre dos PCs (`real_minecraft_handover`) | A enciende, crea un marcador en el scoreboard, pasa el servidor a B. **11,4 s** desde la orden hasta B en línea; el marcador vale 939 en B; la misma dirección `127.0.0.1:41911` ahora llega al servidor de B |
| Túnel P2P con servidor real | Consulta de estado de Minecraft a través del túnel |

## 3. Servidor compartido entre PCs (protocolo, con servidores simulados)

**Tres PCs (A propietaria, B y C coanfitrionas):**

- Invitaciones: B y C se unen; un código ya usado se rechaza.
- **Un solo escritor**: con A hosteando, B no puede encender.
- Sincronización incremental verificada: B y C copian el punto de A.
- Dirección fija: C entra por `127.0.0.1:41901` y llega a A.
- **Traspaso A→B**: B enciende con el mundo de A; la dirección de C pasa a llevar a B; A ya no puede encender.
- **Caída de la anfitriona** (B congelada con SIGSTOP, equivalente a un corte de red): **recuperación en ~10 s** con tiempos de prueba (en uso real: ~35-45 s). Se eligió C (mismo estado, mejor puntaje), con mayoría A+C; mundo con el último progreso sincronizado; A nunca encendió en paralelo.
- **Vuelve la anfitriona vieja**: B detecta que C tomó el servidor, se detiene sola y aparta su copia como punto de seguridad; no puede volver a encender.
- **Soltar**: C apaga y suelta con 2 confirmaciones; la dirección muestra «Nadie lo está hosteando»; A enciende con el último estado.
- **Salir de la app**: al salir A, el servidor pasa solo a otra PC conectada.

**Dos PCs:** sin mayoría, B **no** toma el servidor sola cuando A se congela; ofrece «Recuperar aquí». Al usarlo enciende con el último estado sincronizado; A vuelve, ve la época nueva y se aísla.

**Copia vieja:** si la PC que soltó el servidor se apaga antes de que otra copie el último punto, una PC con copia vieja **no puede encender** («La última versión del mundo quedó en la PC de A…»), el jugador ve «Nadie lo está hosteando», y solo «Recuperar aquí» (explícito) permite seguir con lo que hay.

## 4. Consumo del nodo del grupo

Medido con el nodo sin interfaz (compilación de depuración, la versión final es más liviana):

- Nodo sin grupo: 40 MB de RAM, menos de 1 s de CPU en 60 s.
- Dos PCs en un grupo, consultándose cada 5 s, durante 60 s: 44 MB de RAM cada una, menos de 1 s de CPU.

## 5. Modpack privado del grupo — compatibilidad con copias

Condiciones respetadas: el mundo llevaba más de 12 h sin escribirse (Minecraft cerrado); solo lectura sobre los originales; copia de seguridad independiente verificada antes de usarla; todo el trabajo sobre copias de esa copia; nada se escribió en la PC.

| Paso | Resultado |
|---|---|
| Copia de los mods (42 jar, sin duplicados «(1)» ni Atmosfera/Essential, que son de cliente y pesan 116 MB) | Tamaños idénticos a los informados por la PC; los 42 jar pasan la verificación de integridad ZIP; SHA-256 guardados |
| Copia del mundo: todos los archivos de datos, jugadores y configuración, y las 10 regiones del overworld (+ entidades y POI) donde se jugó más recientemente, más el Nether alrededor del portal: 84 archivos, 119 MB | `diff` de SHA-256 entre la copia recibida y la de seguridad: idénticas. **No se copiaron las ~240 regiones restantes (~2,4 GB)** para no cargar la conexión; Minecraft regeneraría esas zonas en la copia de prueba |
| Lectura de metadatos de los 42 mods sin internet | 8 son solo de cliente (Sodium, Mod Menu, LambDynamicLights, Shield Status, Simple Blood, Take Your Stream Chat, Infinity FullBright, Walksy Lib) |
| Servidor Fabric 26.2 armado a mano con los mods | Arranca. **Hallazgo:** Zombie Horde 1.3.1 hace fallar a un servidor dedicado en un mundo nuevo sin `zombie_horde.properties` (llama a la lista de jugadores antes de que exista). Con el mundo original (que ya tiene ese archivo), arranca bien |
| **Ensayo de migración con 939 Host** (`migration_rehearsal_with_copies`) | Servidor Fabric 26.2 creado por 939 Host (Fabric instalado por el plan B desde Maven, porque el servicio de Fabric no es accesible desde el entorno), mundo importado, mods importados: **en línea en 16 s con 35 mods, MSPT 0,7 ms, 0 errores en el registro**, punto de restauración en caliente (171 archivos), detención limpia. **Hallazgo corregido**: la importación omitía Mod Menu por ser de cliente, pero Stonevaults lo exige; ahora se conservan los mods de cliente que otro mod necesita |
| Puntos incrementales sobre la copia real | 1.er punto 119 MB en 2,3 s; tras cambiar dos regiones y el `level.dat`, 2.º punto **+0,2 MB en 0,4 s**; restaurar el 1.º deja el mundo idéntico a la copia original (`diff -r` sin diferencias) |

## 6. Interfaz

Prueba con jsdom (DOM simulado y comandos de la app reemplazados por respuestas fijas): todas las pantallas y acciones principales (inicio, asistente con los 4 loaders, servidor, consola, jugadores, mods, restauración, grupo, pasar a…, soltar, opciones, invitar, red, unirse, salir del grupo, este equipo, configuración, reconexión) se renderizan y llaman a los comandos correctos; **0 errores de JavaScript**.

## 7. No verificado (y por qué)

| Qué | Motivo |
|---|---|
| La aplicación abierta en Windows real (ventana, WebView2, bandeja, DPAPI, inicio con Windows, Firewall) | El entorno es Linux; el `.exe` se compila por cruce. Revisé que solo dependa de DLL del sistema y de `WebView2Loader.dll` |
| Conexión entre casas distintas: hole punching, UPnP, relays públicos, puente entre PCs en redes distintas | Necesita redes reales; los relays y el DNS de iroh están bloqueados en el entorno. Lo probado es la conexión directa |
| Forge y NeoForge | Sus repositorios no son accesibles desde el entorno |
| Descargas reales de Modrinth, Adoptium (Java) y del servicio de Fabric | Bloqueados en el entorno. Modrinth se probó con datos en su formato oficial; Fabric por su repositorio Maven (accesible) |
| Mundo completo del modpack (2,5 GB) | Se usó una muestra de 119 MB para no cargar la conexión de la PC |
| Reconexión automática dentro del juego | Necesita un cliente de Minecraft; se probó la instalación del mod en una carpeta de perfil simulada |
