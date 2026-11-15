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

## 8. Diseño v3, nombres de PC y «Solicitar ser anfitrión» (15/11/2026)

Mismo entorno Linux aislado. Nada de esto se ejecutó contra la instalación de Minecraft del usuario ni contra el mundo 939 Z: los servidores reales son servidores oficiales de Mojang descargados en carpetas temporales.

| Prueba | Tipo | Qué verifica |
|---|---|---|
| `cargo test` core + p2p + cluster | Automática | 31 + 5 + 2 pruebas, 0 fallas (incluye `pcname`: nombres válidos, sin caracteres de control, 1–24) |
| `host_requests_and_pc_names` (3 PCs, P2P real por localhost) | Protocolo con servidores simulados | Renombrar no cambia la identidad ni el permiso; un jugador sin rol de coanfitrión no puede solicitar; solicitud duplicada o simultánea rechazada; cancelar y rechazar; aceptar usa el traspaso existente y el mundo llega intacto; si la copia previa falla, la anfitriona vieja sigue; si la anfitriona no responde, no se crea solicitud |
| `slow_two_phase_handover_keeps_playing` | Protocolo con servidores simulados | Durante una fase 1 lenta (copia en caliente) el juego sigue en la anfitriona vieja y nunca hay dos encendidos |
| `real_minecraft_handover` | **Servidor real** 26.2 | Traspaso en dos fases entre dos procesos: el marcador del scoreboard llega a B y la dirección fija apunta a B |
| `v3_features` | **Servidor real** 26.2 | Fases de arranque, consulta oculta, pausa, guardado, SNBT real de jugadores, cuenta regresiva, reinicio, recursos del juego (1,1 MB, no se redistribuyen), reparación de puerto ocupado + deshacer, reparación de mods |
| `full_server_lifecycle`, `players_and_permissions_real_server` | **Servidor real** 26.2 | Sin regresiones respecto de 0.9.0 |
| Capturas en claro y oscuro, 1024×680 a 2560×1440 | Interfaz con backend simulado (Playwright) | Sin desbordes ni textos cortados |
| Rendimiento de animaciones (CPU ×4 más lenta) | Interfaz con backend simulado | Mediana 16,7 ms por cuadro, p99 33 ms, 3 cuadros > 50 ms al cambiar de pantalla |
| Movimiento reducido | Interfaz con backend simulado | Con `prefers-reduced-motion` o la opción de la app: transiciones 0 s, sin animación de páginas |

Errores encontrados en la revisión y corregidos antes de esta tanda: durante un traspaso largo se salteaba el control de aislamiento (riesgo de dos escritores); tras perder el permiso se podía reiniciar localmente; una confirmación sin respuesta podía reencender en la PC vieja; dos traspasos simultáneos; cualquier miembro podía quitar a otro (ahora solo la propietaria).

**Hallazgo de esta tanda (corregido):** la primera corrida de `real_minecraft_handover` falló porque la PC que recibe ahora instala el servidor *antes* de tener el mundo (fase de preparación) y la carpeta del servidor todavía no existía. El traspaso se canceló de forma segura —el servidor siguió encendido en A, sin dos escritores— y tras crear la carpeta en la instalación la prueba pasó: traspaso real en 11,8 s, marcador = 939 en B. Las pruebas con servidores simulados no lo detectaban porque no instalan nada.

### Limitaciones conocidas de esta versión

- Una PC con 939 Host anterior a esta rama no puede **recibir** un traspaso desde una nueva (no conoce la orden `Prefetch`). Todas las PC del grupo deben actualizarse juntas.
- Si después de confirmar el traspaso la PC nueva no logra encender, queda con el permiso y el servidor apagado. El mundo está a salvo y cualquiera puede usar «Recuperar aquí» o reintentar; no se vuelve atrás sola para no arriesgar dos escritores.
- Si la confirmación se envía pero no llega respuesta, la PC vieja asume que se entregó y **no** reenciende. Puede quedar el servidor apagado hasta que la nueva encienda o se recupere.
- Skins reales de Mojang y la búsqueda de Modrinth en vivo solo se probaron con datos de prueba en su formato oficial (los servicios están bloqueados en el entorno).
- Arrastrar y soltar `.jar` solo se probó en el navegador del entorno, no en WebView2 de Windows.

## 9. Validación real pendiente (hacer en PCs reales antes de publicar)

Nada de esta lista se ejecutó. **No hay que presentarlo como probado** hasta completarlo en equipos reales.

| # | Qué validar | Cómo | Resultado esperado |
|---|---|---|---|
| 1 | Windows 11 real — *app instalada y usada en Windows Server 2025; falta Windows 11 de escritorio* | Instalar el `.exe` en una PC de prueba (no en la PC donde corre el mundo 939 Z) | Abre, bandeja, inicio con Windows, Firewall pide permiso una vez, tema claro y oscuro |
| 2 | Arrastrar y soltar `.jar` | Arrastrar 2–3 mods sobre la pantalla Mods de un servidor de prueba | Se importan, se marcan los de cliente, no se duplican |
| 3 | Modrinth en vivo — *hecho en Windows (Lithium en Fabric 26.3)* | Buscar «sodium», filtrar por versión y loader, abrir ficha, instalar en servidor de prueba | Resultados reales, dependencias resueltas, hash verificado |
| 4 | Skins reales de Mojang | Servidor de prueba en modo online con una cuenta real | Se ve la skin; con la opción desactivada no se descarga nada |
| 5 | Traspaso entre dos PC físicas (misma red) — *hecho entre dos máquinas Windows de GitHub; falta en casa* | Solicitar ser anfitrión desde B, aceptar en A | B enciende con el mundo de A, A queda apagada, los jugadores vuelven a entrar por la misma dirección |
| 6 | Traspaso entre dos casas/redes — *hecho entre regiones de Azure; falta con routers hogareños* | Igual que 5 con B en otra red | Conecta directo o por relay; avisos de conectividad visibles en la solicitud |
| 7 | Reconexión de jugadores tras el cambio | Con un jugador dentro del juego durante el traspaso | Ve «se está mudando», puede volver a entrar sin cambiar la dirección |
| 8 | Cortes de conexión y recuperación | Desconectar el cable o Wi-Fi de la anfitriona con 3 PC en el grupo | Se detiene sola en la PC aislada; otra toma el servidor en ~35–45 s; al volver, la vieja no reenciende |
| 9 | Rechazo, cancelación y vencimiento con PCs reales | Solicitar y cancelar; solicitar y dejar vencer (10 min) | El aviso desaparece en ambas PC |

Recomendación: hacer estas pruebas con un **servidor de prueba nuevo** y una copia del mundo, nunca con el mundo 939 Z en uso.

## 10. Pruebas reales en Windows (GitHub Actions, Windows Server 2025)

Flujo `.github/workflows/pruebas-windows.yml`, en la rama `rediseno-v3`. Corre en máquinas
Windows limpias de GitHub (misma base que Windows 11, con internet real): **no es la PC del
usuario** y no toca ninguna instalación de Minecraft. No publica nada.

| Prueba | Resultado (15/11/2026) |
|---|---|
| Núcleo, P2P y grupo (38 pruebas), con PCs congeladas de verdad (`NtSuspendProcess`) | ✔ incluida la recuperación por mayoría en ~14 s y «Recuperar aquí» con 2 PCs |
| Vanilla 26.3: descarga real de Java 25 y del servidor de Mojang, encendido, comando, apagado | ✔ en línea en ~15 s |
| Fabric 26.3 (loader 0.20.1) | ✔ ~17 s |
| Forge 26.3 (66.1.8, instalador oficial) | ✔ ~18 s |
| NeoForge 26.3 (26.3.0.103, instalador oficial) | ✔ ~17 s |
| Modrinth real: búsqueda, plan, descarga con hash verificado, servidor Fabric con Lithium, quitar | ✔ |
| Ciclo completo: copia en caliente, puntos incrementales, puerto ocupado, restaurar, borrar a la Papelera | ✔ (después de corregir el bloqueo de Windows, ver abajo) |
| Jugadores y permisos con servidor real | ✔ |
| Funciones v3 (pausa, guardado, SNBT, cuenta regresiva, reinicio, recursos del juego, reparación) | ✔ |
| **Traspaso de anfitrión real entre dos «PCs» por P2P** (dos nodos en la misma máquina, sin relays) | ✔ en 27,9 s; el marcador del mundo llegó intacto y la misma dirección apunta a la nueva anfitriona |
| Instalador NSIS silencioso sin administrador, SHA-256, app abre y queda abierta | ✔ |
| Interfaz de la app instalada, manejada por WebView2 como una persona: crear un servidor con el asistente, descarga real, encender, comando en la consola, todas las pestañas, copia en caliente, tema claro y oscuro, detener con confirmación | ✔ 15/15 pasos, sin errores de JavaScript (ventana de 1024×768) |
| **Dos máquinas Windows distintas** (regiones centralus y westcentralus de Azure), P2P con relays habilitados como la app | ✔ unión en 1 s; la invitada entra al servidor real de la otra máquina por `127.0.0.1:41911`; «Solicitar ser anfitrión» → aceptar → traspaso en 16 s con el marcador intacto; la PC vieja deja de alojar y entra por su dirección fija al servidor nuevo |

**Problemas que solo aparecieron en Windows y se corrigieron:**
- Mover un servidor (o un mundo) a la Papelera fallaba con «Acceso denegado» cuando un archivo
  quedaba abierto un instante. Ahora se reintenta unos segundos (antivirus/indexador).
- Los diálogos del instalador/desinstalador bloqueaban la desinstalación silenciosa.
- La barra lateral cortaba «Servidores de amigos» en la ventana mínima (1024 px).

**Observado y no corregido (bajo impacto):** en Windows, la primera consulta de estado justo
después de «Done» se corta y la siguiente (1 s después) responde. Pasó con los cuatro loaders y
con la ruta P2P. Para un jugador significa, a lo sumo, un «No se pudo conectar» que se resuelve
al reintentar.

**Lo que estas pruebas no cubren:** routers hogareños con NAT estricto (las máquinas de Azure
tienen redes de datacenter; la conexión fue directa), Smart App Control activo, antivirus de
terceros, PCs con poca memoria, y un cliente de Minecraft real conectado jugando durante el
traspaso.

Nota: la máquina Linux de una prueba intermedia no pudo preparar Java para Linux (la app solo
se distribuye para Windows); el traspaso se canceló de forma segura y el servidor siguió en la
PC anfitriona. Por eso la prueba definitiva usa dos Windows.
