# Seguridad de 939 Host: diseño, modelo de amenazas y riesgos residuales

Este documento describe cómo se protege 939 Host. Lo que dice está implementado y probado en
el código. Las pruebas están en `crates/cluster/src/cert.rs`, `crates/cluster/src/signed.rs`,
`crates/cluster/tests/seguridad.rs`, `crates/core/src/files.rs` y `crates/core/src/modguard.rs`.

939 Host **no es invulnerable**, no tiene ninguna certificación y no promete seguridad absoluta.
Al final se listan los riesgos que siguen existiendo. Los intentos de vulnerarlo, las fallas que
encontraron y las mediciones de rendimiento están en [`RED-TEAM.md`](RED-TEAM.md). La fase final
(Windows, Log4Shell, sistema de aprobación de mods, identidad y GitHub) está en
[`AUDITORIA-INTEGRAL.md`](AUDITORIA-INTEGRAL.md) y [`AUDITORIA-GITHUB.md`](AUDITORIA-GITHUB.md). El estado
de cierre y lo que falta para usuarios reales están en [`CIERRE.md`](CIERRE.md).

## Principios

- **Cada PC es una identidad.** Usa una clave ed25519 guardada con DPAPI (solo su usuario de
  Windows puede leerla). Las conexiones son QUIC con TLS 1.3 (iroh). El TLS verifica que del
  otro lado esté exactamente la clave esperada. Autenticar dice *quién es*. Autorizar dice *qué
  puede hacer* según su rol en la lista firmada. Son dos pasos separados.
- **Ser del grupo no da confianza ilimitada.** Cada pedido se autoriza en el backend según el rol
  (propietaria, coanfitriona, jugadora). La interfaz no decide nada.
- **Mínimo conocimiento.** Una jugadora solo habla con la anfitriona y con la propietaria. Solo
  recibe lo necesario para jugar.
- **Sin criptografía propia.** Usa firmas ed25519 de iroh, blake3 en modo derivación de claves,
  SHA-256/SHA-1 de repositorios oficiales y DPAPI de Windows.
- **Sin infraestructura propia.** Solo usa los relays gratuitos de n0 (opcionales).

## Qué puede hacer cada rol

| Operación | Propietaria | Coanfitriona | Jugadora |
|---|---|---|---|
| Jugar (túnel de Minecraft) | sí | sí | sí |
| Recibir copias del mundo (sincronización) | sí | sí | **no** |
| Configuración completa del servidor | sí | sí, sin datos locales | **no** |
| Direcciones de red de otras PCs | sí, sin IPs con relays | sí, sin IPs con relays | **no** |
| Administrar archivos en la anfitriona | sí | sí, con el mundo protegido | **no** |
| Alojar el servidor | sí | sí | **no** |
| Invitar, quitar PCs, cambiar roles | sí | no | no |
| Aprobar mods recibidos | sí | no | no |
| Designar sucesora o crear el código de recuperación | sí | no | no |

## Permiso de anfitrión (lease) firmado

Cada permiso lo firma la PC que lo emite y encadena con el anterior (`prev`). Una PC acepta un
permiso nuevo solo si se cumplen las tres condiciones:

1. La firma es válida.
2. Encadena con el permiso que ella tiene.
3. Quien lo emite tenía autoridad para ese tipo de cambio.

| Tipo | Quién lo emite | Condición |
|---|---|---|
| Inicial | Propietaria | Época 1 |
| Traspaso | Anfitriona actual | Destino que puede hostear |
| Soltar | Anfitriona actual | Misma época |
| Tomar | PC que puede hostear | El permiso estaba soltado |
| Elección | Candidata | Mayoría de votos firmados para esa época y esa candidata |
| Recuperación manual | Coanfitriona | Quien todavía ve a la anfitriona la rechaza; la anfitriona solo obedece si la mayoría ya la adoptó |
| Reasignación | Propietaria | Control total |

- **Sin replay:** las épocas solo crecen, y un permiso viejo o repetido no cambia nada.
- **PC atrasada:** pide la cadena y verifica cada eslabón con las mismas reglas.
- **Nunca dos anfitrionas a la vez:** se mantiene el aislamiento automático de antes (una
  anfitriona que pierde a la mayoría se detiene).

## Sincronización del mundo

El anuncio de un punto de restauración (época y orden) y su índice los escribe la PC que los
manda y no tienen firma. Por eso una PC copia un punto solo si:

- viene de una PC que puede hostear según la lista firmada (nunca de una jugadora);
- su época no es mayor que la del permiso que esta PC ya verificó;
- si es de la época actual, viene de la anfitriona actual (la única que crea puntos en ella).

Se verifica dos veces: al elegir de quién copiar y al bajar el índice. El origen del punto
(qué PC lo creó) lo fija quien lo recibe con la identidad autenticada de quien lo mandó, no lo
que dice el índice. Así los mods que trae cuentan como recibidos y pasan por la cuarentena.

## Lista de integrantes firmada

- La firma la propietaria. Cada cambio sube la revisión, y se rechaza cualquier lista con revisión
  igual o menor (**no se pueden recuperar permisos revocados reenviando una lista vieja**).
- **Código de recuperación:**
  - Son 160 bits.
  - Se muestra una sola vez.
  - Solo se guarda la clave pública derivada.
  - Recupera la propiedad desde otra PC del grupo.
  - Revoca a la propietaria anterior y genera un código nuevo.
  - La propietaria no puede cambiar el código sin el código actual, o sin una espera de 72 h
    visible para el grupo.
- **Sucesora designada:**
  - Solo la propietaria la designa.
  - La sucesora firma un reclamo y el grupo recibe un aviso.
  - Hay una espera de 72 h, que cada PC mide desde que ella misma vio el reclamo.
  - La propietaria cancela con cualquier cambio firmado.
  - El registro queda en la lista firmada.

## Privacidad de red

| Modo | Qué ven las otras PCs del grupo | Qué ve el relay de n0 |
|---|---|---|
| Directo (sin relays) | Tu IP pública o local y tu puerto, porque se conectan a vos | Nada |
| Normal (relays como respaldo) | Tu IP cuando la conexión pasa a ser directa (hole punching) | Tu IP, horarios y volumen mientras pasa por el relay |
| **Privado** (solo relay) | **Ni tu IP pública ni la local** (no hay transporte IP propio) | Tu IP, horarios y volumen; nunca el contenido |

- En **todos** los modos el contenido va cifrado de punta a punta entre las dos PCs. El relay no
  puede leerlo ni modificarlo sin que se note.
- **Modo privado:**
  - Lo activa cada PC en Configuración > Privacidad y persiste.
  - Si el relay no responde, no hay conexión: **nunca cambia a directo por su cuenta**.
  - No afecta a las demás PCs.
- Probado con relays reales en la CI de Windows (`modo_privado_oculta_la_ip_real`): las otras
  PCs solo conocen el relay de la PC privada.
- **Con relays, nunca se comparten IPs** en códigos de invitación, respuestas `Info`, uniones ni
  `peers.json`. Las otras PCs se encuentran por su identidad.
- **Al unirse, una jugadora solo recibe cómo llegar a la anfitriona** (antes recibía las
  direcciones de todas las PCs; en modo directo, con sus IPs).
- **Direcciones contadas por otra PC:** la dirección propia de una PC solo se acepta si es de
  ella; las de terceros, solo de quien puede hostear, para integrantes, sin reemplazar lo que ya
  se sabe (salvo la propietaria) y solo con relays de n0. Así nadie puede hacer que el resto
  intente llegar a otra PC por un relay propio.
- **UPnP/NAT-PMP desactivado:** 939 Host no abre puertos en el router.
- **Minecraft escucha solo en 127.0.0.1.** Permitir la red local es una opción explícita.
- Los informes de diagnóstico se limpian: sin IPs, sin el nombre de usuario de Windows en las
  rutas y sin contraseñas.

## Archivos

- Las rutas se resuelven como las ve Windows: mayúsculas, nombres cortos 8.3 y unidades.
- No se siguen enlaces simbólicos, junctions ni puntos de reanálisis.
- Después de abrir un archivo se verifica su ruta final (TOCTOU).
- Se rechazan los nombres de dispositivo (`CON`, `NUL`…), los flujos alternativos (`a:b`) y los
  caracteres prohibidos.
- Con el servidor encendido, el mundo no se modifica, no se descarga y no se borra, sin importar
  cómo se escriba la ruta.
- Una subida tiene como máximo 8 GB y tiene que dejar 2 GB libres. Las subidas cortadas no dejan
  archivos parciales: al cancelar, la PC que sube espera la confirmación de la anfitriona.
- **Desde otra PC no se toca lo que decide qué programa arranca:** `libraries/` (incluye los
  argumentos de Java de Forge y NeoForge), `versions/`, `.fabric/`, `mods-cuarentena/`,
  `server.jar`, el lanzador de Fabric y los archivos de arranque. No se pueden crear,
  reemplazar, mover ni borrar con ninguna forma de escribir la ruta, y el intento queda
  auditado. Desde la propia anfitriona sí (es su PC).
- Los ids de servidor que llegan por la red se validan.

## Mods (código de terceros)

- Un `.jar` o `.zip` recibido de otra PC (por sincronización o por el administrador remoto),
  también en subcarpetas de `mods/` (Forge viejo carga `mods/<versión>/`), **no se carga**
  hasta que la propietaria apruebe su huella SHA-256. Si se reemplaza un mod aprobado, vuelve a
  la cuarentena.
- **No es un aislamiento.** Un mod aprobado puede hacer lo mismo que cualquier programa del
  usuario.
- El proceso de Minecraft corre en un Job Object sin acceso al portapapeles, al escritorio ni a
  los ajustes del sistema. Es endurecimiento parcial, **no una sandbox**.
- Se evaluaron cuentas separadas de Windows y AppContainer. No se implementaron: exigen permisos
  de administrador o rompen Java y los mods.

## Límites de pedidos

| Canal | Límite por PC | Uso normal |
|---|---|---|
| Unión con código | 5 intentos por minuto | 1 |
| Control (consultas, permisos, votos) | 600 por minuto | menos de 100 por servidor |
| «Sincronizá ya» | 12 por minuto | pocos |
| Sincronización y archivos | 3000 por minuto | unos 260 para copiar 2 GB |

Pasado el límite, esa PC recibe «demasiados pedidos» y las demás siguen atendidas.

## Descargas y compilación

Verificación de cada descarga:

| Descarga | Verificación |
|---|---|
| Servidor de Mojang | SHA-1 oficial |
| Java (Adoptium) | SHA-256 oficial |
| Forge y NeoForge | SHA-1 del repositorio Maven oficial (obligatorio) |
| Fabric | Desde su Maven, cada librería con su SHA-1 |
| Modrinth y CurseForge | Huella del proveedor (obligatoria: sin huella no se descarga) |

Además:

- Las dependencias de Rust no tienen vulnerabilidades conocidas (`cargo audit`). Quedan avisos de
  crates sin mantenimiento.
- Las acciones de GitHub están fijadas por commit, con permisos mínimos.
- Ninguna clave va al ejecutable. `scripts/verificar-secretos.sh` lo comprueba en cada release.
- Los ganchos de prueba y el puerto de depuración de WebView2 solo existen con la característica
  `pruebas`. La compilación de prueba de la CI (`939Host-windows-prueba`) la lleva para la prueba
  de interfaz: abre el puerto de depuración solo si existe la variable `H939_E2E_DEVTOOLS_PORT`.
  Los releases se compilan sin ella.
- **CurseForge:** no hay una forma permitida de usar su API sin una clave. 939 Host no incluye
  ninguna. Cada persona puede cargar la suya, que se guarda con DPAPI. Sin clave, se importan los
  modpacks descargados a mano desde la web de CurseForge.

## Qué corre y qué queda expuesto

| Situación | Procesos | Puertos | Qué se puede alcanzar desde afuera |
|---|---|---|---|
| App cerrada desde la bandeja (o sin abrir) | Ninguno de 939 Host. Si «Iniciar con Windows» está activo, se abre al iniciar sesión y vuelve a la fila de abajo | Ninguno | Nada: no hay servicio de Windows, tarea programada ni proceso residente |
| App abierta, sin servidores | `939 Host.exe` (interfaz y nodo P2P) | Un socket UDP de iroh (QUIC) en un puerto alto aleatorio | Solo el nodo iroh: cada conexión se autentica con la clave de la PC remota y cada ALPN exige ser del grupo o amigo |
| Ventana cerrada con un servidor encendido | `939 Host.exe` en la bandeja + el guardián (`939 Host.exe --guardian`, sin ventana ni red) + `java.exe` dentro de un Job Object | Minecraft escucha en `127.0.0.1` (o en la dirección `.939host` de loopback); la voz, en esa misma dirección por UDP. Con «Acceso por LAN» activado, también en la red local | Lo mismo que arriba. Nada de Minecraft queda en internet salvo que la persona configure una dirección pública propia (playit.gg, router) |
| Salir desde la bandeja con servidores encendidos | Pregunta; pasa o suelta los servidores compartidos y detiene cada uno con `stop` (guardando); después termina todo, el guardián incluido | Se cierran todos | Nada |
| «Cerrar igual» mientras todavía guarda, o 939 Host muere de golpe | El guardián le manda `stop` a cada Java (guarda y se detiene, hasta 10 minutos si sigue guardando) y termina; al volver a abrir se cuenta qué pasó | Se cierran todos | Nada |
| Apagado, reinicio o cierre de sesión de Windows | Con servidores encendidos pide tiempo a Windows con un motivo visible, hace el cierre ordenado y lo libera (tope de 3 minutos); sin servidores no lo demora | Se cierran todos | Nada |
| Corte de luz, «Finalizar árbol de procesos» o falla de Windows | Nada se puede hacer en el momento. Queda la marca `en-marcha`: al volver a abrir se avisa una vez que el mundo no está confirmado como guardado; no se publica solo al grupo; si se pasa a otra PC, el punto queda rotulado «tras un cierre sin confirmar» y el anterior sigue disponible; lo que hubiera en la PC se aparta como punto «Antes de restaurar» si se restaura | — | — |

Datos locales (todos dentro de `%LOCALAPPDATA%\939Host`):

- Clave del nodo protegida con DPAPI; configuración, integrantes firmados, historial de permisos.
- Mundos, puntos de restauración y copias (con tope configurable de espacio).
- Amigos: lista, solicitudes, códigos de invitación y chats (como mucho 500 mensajes por chat),
  **cifrados con DPAPI** (`amigos.bin`, `mensajes/*.bin`). Solo la misma cuenta de Windows en la
  misma PC los descifra. Si esa clave se pierde (Windows reinstalado, cuenta borrada), el archivo
  se aparta como `*.ilegible-<fecha>` y se empieza de nuevo: los chats no se recuperan y hay que
  volver a invitar a los amigos.
- Fotos de perfil propias y de otras PCs (caché por huella SHA-256, 100 KB como máximo cada una).
- El desinstalador pregunta dos veces antes de borrar estos datos; por defecto los conserva.

Qué ve un relay de iroh: la IP pública de cada PC, cuándo se conecta y cuánto tráfico pasa. No
puede leer ni modificar nada (QUIC con TLS 1.3 de punta a punta, autenticado con las claves de
cada PC).

## Superficies nuevas de esta fase

- **Amigos (`939host/friend/1`).** Cualquier PC de internet que conozca la identidad del nodo
  puede abrir este canal. Sin código de invitación válido (un solo uso, vence) solo recibe
  «No son amigos». Límites: comparte el tope de pedidos del grupo por PC remota, tramas de 16 KB como máximo con
  10 s de espera, 20 solicitudes pendientes, 200 amigos, 20 mensajes por minuto, 500 caracteres
  sin caracteres de control. «Aceptó» solo vale si esta PC envió la solicitud. Las direcciones que
  anuncia otra PC solo se aceptan si son suyas. Prueba: `amigos_resisten_a_una_pc_desconocida`.
- **Presencia.** A cada amigo se le cuenta: nombre, si está disponible, huella de la foto y el
  nombre del servidor que se está jugando. No se cuenta IP ni hardware (la conexión directa sí
  revela la IP, igual que en el grupo). Se puede marcar «No disponible».
- **Voz (`939host/voice/1`).** Solo integrantes del grupo. Como mucho 4 canales por PC y 64 en
  total. El puerto UDP local escucha en la dirección `.939host` de loopback; no abre puertos del
  router ni usa UPnP.
- **Fotos de perfil.** Solo para integrantes del grupo y solo la huella pedida; quien la recibe
  comprueba la huella y el tamaño antes de guardarla.
- **Lectura de imágenes (`image_read`).** Comando local de la interfaz para arrastrar archivos:
  lee como mucho 20 MB y solo si el contenido es PNG, JPG, WebP o GIF por sus primeros bytes.
  No es alcanzable desde la red.
- **Estado compartido y OP de Minecraft.** El estado del servidor que muestran las otras PCs lo
  anuncia la anfitriona y se muestra como tal. La lista de OP autorizados es la firmada por la
  propietaria (OP de integrantes + otras cuentas que ella autorizó, incluido lo que ya tenía su
  `ops.json` al empezar). La anfitriona da OP a quien falte y quita, con un aviso y después de un
  margen de 15 s, cualquier OP fuera de la lista (consola de una coanfitriona, `/op` en el juego,
  `ops.json` editado). Un `op`/`deop` desde la consola o Jugadores de la propietaria actualiza la
  lista firmada. `ops.json` no se puede subir desde otra PC.
- **Guardián.** Proceso sin ventana, sin red y sin privilegios que solo recibe duplicados del Job
  Object, del proceso de Java y de su consola. No acepta órdenes de nada más que de 939 Host (su
  entrada es una tubería anónima).
- **Conexiones de control en los dos sentidos y `Nudge`.** La conexión que abre una PC del grupo
  (autenticada con su identidad por TLS) también se usa para hablarle, y la que abre esta PC
  también atiende pedidos; cada pedido sigue pasando por el control de integrante y el límite por
  PC. Solo se guardan conexiones de integrantes verificados. `Nudge` le pide a una integrante que
  abra una conexión nueva hacia esta PC: se marca solo a la dirección ya conocida de quien lo pide
  (no acepta direcciones en el pedido) y se cierra al segundo. Existe para corregir que iroh 1.4
  siga usando un camino viejo después de que una PC se reinicia.
- **Indicadores de operaciones.** Solo locales; nunca inventan porcentajes.

## Modelo de amenazas (resumen STRIDE)

| Atacante | Qué puede lograr hoy | Qué lo impide |
|---|---|---|
| Externo de internet sin invitación | Conectarse al nodo si conoce su identidad, mandar pedidos y forzar trabajo | Toda operación exige ser del grupo; límite de pedidos y tamaño; secreto de 128 bits |
| Invitación filtrada | Unirse como el rol de la invitación si llega primero | Un solo uso, vence en 24 h, revocación, aviso a la propietaria, quitar PC |
| Jugadora maliciosa | Jugar, ver quién aloja y cuándo, ver la lista de integrantes | Sin mundo, sin archivos, sin direcciones ni hardware; no vota por otra, no emite permisos; no es fuente de sincronización; sus anuncios de direcciones se ignoran; límite de pedidos |
| Coanfitriona maliciosa | Recibe el mundo (lo necesita para alojarlo); puede apagar a la anfitriona con «Recuperar aquí» **solo en grupos de 2 PCs** | Permisos firmados; mods en cuarentena (también por sincronización); no toca lo que define qué programa arranca; no puede cambiar roles ni aprobar mods |
| PC del grupo comprometida | Lo mismo que su rol, con sus claves | Quitarla del grupo; recuperación con código si era la propietaria |
| Propietaria comprometida | Control total del grupo | Código de recuperación (gana sobre su clave); cambiar el código exige espera visible |
| Relay comprometido | Ver IPs, horarios y volumen | Cifrado de punta a punta; no puede leer ni modificar |
| Misma Wi-Fi | Antes: entrar directo al puerto de Minecraft. Ahora no (solo 127.0.0.1) | Escucha local; sin UPnP |
| Mod o archivo malicioso | Si se aprueba, todo lo que puede el usuario de Windows | Cuarentena por huella; aprobación explícita; Job Object |
| Robo de archivos locales | La clave del nodo está con DPAPI (otro usuario no la lee; con la sesión del usuario, sí) | DPAPI; quitar la PC y regenerar la identidad |
| Desconocido con la identidad del nodo (amigos) | Mandar pedidos al canal de amigos | Sin código válido no pasa de «No son amigos»; límites por PC, tamaño y tiempo |
| Amigo malicioso | Mandar mensajes molestos; ver presencia y qué servidor se juega | 20 mensajes por minuto, 500 caracteres; quitar o bloquear; «No disponible» |
| Malware local con la sesión del usuario | Todo lo que puede el usuario: leer la clave del nodo, chats y mundos | Fuera del alcance de una app sin privilegios; Defender, cuentas separadas |

## Auditoría independiente (19/11/2026, noche)

Revisión hecha desde el lado de quien quiere comprometer 939 Host, no solo sobre la lista anterior.

### Qué puede hacer cada atacante

| Atacante | Puede | No puede (comprobado) | Gravedad / probabilidad | Riesgo residual |
|---|---|---|---|---|
| Cualquier PC de internet que conoce la identidad de un nodo | Abrir conexiones QUIC y mandar basura | Pasar de la autenticación por clave; unirse sin un código válido (5 intentos por minuto); tumbar al grupo con inundaciones (límite por PC, prueba de inundación) | Media / baja | Consumo de CPU y red mientras dura el ataque |
| Jugadora del grupo | Jugar; ver quién aloja; ver el historial de permisos | Recibir direcciones de las demás PCs; alojar; administrar archivos; dar OP; ver puntos ni hardware | Baja / media | Lo que cualquier jugador ve dentro de Minecraft |
| Coanfitriona | Recibir el mundo completo; alojar; administrar archivos de la anfitriona salvo lo que decide qué programa arranca, `ops.json` y `server.properties`; subir mods (quedan en cuarentena) | Ejecutar código en la PC de la propietaria: mods sin aprobar no se cargan, librerías/servidor/argumentos de Java bloqueados, argumentos de Java locales nunca viajan, RCON/query/administración apagados en servidores compartidos; dar OP fuera de la lista firmada; quedarse con el servidor sin permiso firmado | Alta / baja | **Mientras ella aloja** controla su copia: puede cambiar su `server.properties`, sus mods, ver IPs de jugadores en modo normal (no en modo privado). Scripts en `config/` de mods aprobados (riesgo 13) |
| Alguien con acceso a la PC anfitriona (misma cuenta de Windows) | Todo lo que esa cuenta puede: leer mundos, usar la clave del nodo, cambiar la app | — (límite de cualquier programa sin privilegios; DPAPI no protege contra la misma sesión) | Alta / baja | Inherente; se mitiga con la seguridad de Windows |
| Servidor de distribución comprometido | Negar o demorar actualizaciones | Colar un archivo o una versión vieja (firmas, secuencia, vencimiento; pruebas `ataques.rs`) | Alta / baja | Congelamiento hasta que vence la publicación (120 días como máximo) |
| Quien robe la clave de publicación | Firmar una versión maliciosa | — | Crítica / muy baja | Se revoca con la recuperación; las apps que ya instalaron esa versión quedan comprometidas. Por eso la clave está en el TPM y cada firma pide confirmación de Windows |

### Áreas revisadas

- **Ejecución de código y comandos**: Java (Temurin verificado), el guardián (mismo ejecutable) y el
  instalador descargado (verificado); nada se ejecuta con datos de otra PC. Consola de Minecraft solo
  desde esta PC.
- **Archivos y rutas**: limpieza de rutas, puntos de reanálisis (*junctions*), ruta final del archivo
  abierto, nombres reservados de Windows, Papelera en lugar de borrar; bloqueos por rol (arriba).
- **Instalador, actualizador y desinstalador**: por usuario, sin administrador; no reemplaza con la app
  abierta; conserva datos salvo pedido explícito; actualizador ver [ACTUALIZACIONES.md](ACTUALIZACIONES.md).
- **Procesos, servicios, puertos**: sin servicios ni tareas programadas; Minecraft en `127.0.0.1`; la app
  no escucha TCP fuera de la PC (prueba de la versión de prueba en Windows); un socket UDP de iroh.
- **P2P, NAT y relays**: QUIC con TLS 1.3 autenticado por clave; el relay ve IP, horarios y volumen; modo
  privado oculta la IP a las otras PCs. Corregido en esta fase: conexiones en uso que se cerraban.
- **Identidad, permisos de anfitrión, OP**: cadena de permisos firmada, votación, lista de integrantes
  firmada, OP solo por la lista firmada.
- **Transferencia y doble escritura**: punto incompleto no se usa; nunca dos anfitrionas; cierre que nadie
  vio rotulado.
- **Mods y modpacks**: cuarentena por huella, aprobación de la propietaria, verificación de hash de
  Modrinth/CurseForge, detección de Log4Shell.
- **Mensajes de otros usuarios**: tamaños máximos, límites por PC, JSON validado, mensajes enormes y
  malformados en los 5 protocolos probados.
- **Secretos e información personal**: revisión de cada ejecutable compilado (claves, rutas personales,
  IPs privadas, ganchos de prueba); informes de diagnóstico sin IPs ni rutas de usuario.
- **Logs y temporales**: registro de auditoría sin IPs ni mensajes; temporales de más de 6 horas se borran.
- **Abuso de recursos**: límites de pedidos, tamaño de subidas, tope de espacio de puntos, metadatos de
  actualización de hasta 256 KB y descargas que nunca superan lo firmado.

### Dependencias

`cargo audit` (base de RustSec del 19/11/2026) sobre todo el workspace:

- **0 vulnerabilidades** después de pasar `p256` a 0.14 (la 0.13 traía `crypto-bigint` 0.5 con
  RUSTSEC-2026-0387, tiempos variables en operaciones que deberían ser constantes; en 939 Host solo se
  usaba para verificar firmas públicas, sin secretos, pero se actualizó igual).
- Avisos sin vulnerabilidad: `chrono` sin mantenimiento (riesgo 29); `paste`, `proc-macro-error`, `zbus*`
  y `glib` vienen de la parte de Linux de Tauri y no entran en la compilación de Windows.
- No se actualizaron dependencias en bloque: solo `p256`, con todas las pruebas.

## Revisión de Windows (noviembre de 2026)

| Tema | Estado | Cómo se comprobó |
|---|---|---|
| Defender | Sin detecciones en el instalador, el ejecutable ni la carpeta instalada. No se pide ni se agrega ninguna exclusión | Análisis de Defender en la CI de Windows. Limitación: en los equipos de GitHub la protección en tiempo real está apagada, así que esto no reemplaza una prueba con Defender completo en una PC real |
| Firewall | No se agregan reglas ni se pide abrir puertos. La primera vez Windows puede preguntar por el acceso de red de 939 Host (nodo QUIC); con «Cancelar» sigue funcionando por relay | Revisión de código: no hay llamadas a la API del firewall ni a `netsh` |
| UAC | Instalación y uso sin administrador (`RequestExecutionLevel user`, carpeta por usuario). La única acción con administrador es opcional y explícita: los nombres `.939host` en el archivo `hosts` | Instalador silencioso sin elevación en la CI |
| Smart App Control | Bloquea la app mientras no esté firmada. No se indica a nadie que lo apague | Ver [firma/OPCIONES.md](firma/OPCIONES.md) |
| Permisos de archivos | Todo en `%LOCALAPPDATA%` del usuario; secretos con DPAPI de esa cuenta | Pruebas de cifrado con DPAPI real en Windows |
| Ejecución de procesos | Solo Java (Eclipse Temurin de Adoptium, verificado con el SHA-256 que publica Adoptium por HTTPS) y el propio ejecutable como guardián; Java dentro de un Job Object que lo cierra si todo muere | Prueba de interfaz en Windows: sin procesos huérfanos tras cerrar o matar la app |
| Credenciales | Clave del nodo y datos de amigos con DPAPI; sin contraseñas guardadas; sin claves de API embebidas (revisado en cada compilación) | `verificar-secretos.sh` sobre el ejecutable |
| Segundo plano y bandeja | Sin servicio de Windows ni tarea programada; «Iniciar con Windows» es opcional (entrada del usuario en `Run`). Sin ventana no consulta métricas de Java | Revisión de código y medición de CPU en bandeja en la CI |
| Puerto de depuración de WebView2 | Solo existe en la compilación de prueba de la CI; la de entrega no lo abre aunque exista la variable | Prueba de la versión de prueba en Windows |
| Desinstalación | Quita ejecutable, accesos directos y registro; conserva mundos por defecto | CI: instalar, actualizar, reinstalar, interrumpir y desinstalar en los dos modos |
| Actualizaciones | No hay autoactualización; diseño propuesto en [ACTUALIZACIONES.md](ACTUALIZACIONES.md) | — |

## Riesgos residuales — reclasificación de noviembre de 2026

Estados: **Corregido y verificado** (con prueba automática que lo cubre), **Mitigado** (sigue
existiendo con menos alcance), **Limitación de diseño** (inevitable con este diseño o esta plataforma),
**Pendiente** (falta trabajo o infraestructura), **Requiere prueba real externa** (solo se puede
confirmar en las PCs del equipo o desde otra red).

| # | Riesgo | Estado | Detalle |
|---|---|---|---|
| 1 | Mods aprobados pueden hacer todo lo que el usuario de Windows | Limitación de diseño | Minecraft no se puede aislar sin romper el juego; cuarentena por huella y aprobación explícita |
| 2 | Grupo de 2 PCs: la coanfitriona puede apagar a la anfitriona con «Recuperar aquí» | Mitigado | Queda registrado y la propietaria lo revierte. Se podría exigir su confirmación o una espera |
| 3 | La coanfitriona recibe el mundo completo | Limitación de diseño | Lo necesita para alojarlo |
| 4 | El relay ve metadatos; en modo normal la otra PC ve la IP | Mitigado | El modo privado oculta la IP a las otras PCs (probado); el relay siempre ve IP, horarios y volumen |
| 5 | DPAPI no protege contra malware con tu misma sesión | Limitación de diseño | Límite de cualquier app sin privilegios |
| 6 | Las jugadoras ven el historial de permisos | Limitación de diseño | Lo necesitan para verificar la cadena firmada |
| 7 | Las esperas de 72 h usan el reloj de cada PC | Mitigado | Se podría medir con marcas de tiempo de la mayoría del grupo |
| 8 | Una invitación filtrada usada primero da acceso | Mitigado | Un solo uso, vence en 24 h, aviso a la propietaria, quitar la PC (probado) |
| 9 | El ejecutable no está firmado | Pendiente | Ver [firma/OPCIONES.md](firma/OPCIONES.md). Smart App Control lo bloquea y SmartScreen avisa |
| 10 | Todas las PCs necesitan esta versión | Limitación de diseño | Los permisos sin firma de versiones viejas se rechazan a propósito |
| 11 | El nombre de un mod pendiente lo elige quien lo subió | Mitigado | Se muestra la huella; se podría mostrar su origen buscándola en Modrinth |
| 12 | Una coanfitriona elige qué anuncia de sí misma | Mitigado | Se podrían firmar los índices de los puntos |
| 13 | Scripts en `config/` de mods aprobados (KubeJS) | Pendiente | Se podría poner `config/` en cuarentena para mods que ejecutan scripts |
| 14 | Punto falso de una época anterior | Mitigado | No gana mientras la anfitriona crea puntos en la época actual (probado); se podrían firmar los índices |
| 15 | Traspaso automático con el servidor detenido | Limitación de diseño (decisión de producto) | Con el servidor encendido siempre pregunta; detenido, no hay nada que proteger |
| 16 | OP agregados fuera de la lista autorizada | Corregido y verificado | Lista firmada como única autoridad; prueba `estado_compartido_y_op_concedido_por_la_propietaria` |
| 17 | La voz por una dirección pública depende del servicio externo | Pendiente | Ver [DIRECCIONES-PUBLICAS.md](DIRECCIONES-PUBLICAS.md) |
| 18 | Si 939 Host muere de golpe, Java se cerraba sin guardar | Corregido y verificado | El guardián detiene Java con `stop`; probado en Windows matando el proceso |
| 19 | Apagar Windows con un servidor encendido | Requiere prueba real externa | Probado con el aviso simulado en Windows (motivo registrado, tope de 180 s); falta un apagado real |
| 20 | Chats de amigos en texto plano | Corregido y verificado | DPAPI real en Windows; migración automática; lo ilegible se aparta con aviso |
| 21 | Una PC que conoce la identidad del nodo puede seguir abriendo conexiones | Mitigado | Límites por PC, tamaño y tiempo (prueba de inundación); cada conexión cuesta un saludo TLS |
| 22 | Jugadores sin 939 Host sin revelar la IP de la casa | Pendiente | Investigación en curso; nada implementado ni prometido |
| 23 | «Finalizar árbol de procesos», corte de luz o falla de Windows | Mitigado (antes: inevitable) | Sigue sin poder guardarse en el momento. Ahora se detecta al volver a abrir (marca `en-marcha`), se avisa, no se publica solo y el traspaso queda rotulado; pruebas `un_cierre_que_nadie_vio…` y `tras_un_corte_sin_guardian…`. Falta un corte real |
| 24 | Una conexión nueva de una PC conocida cerraba la que estaba en uso (pedidos perdidos; dos PCs podían cerrarse mutuamente las conexiones) | Corregido y verificado | Se prueba la guardada antes de cerrarla; regresión en `mensajes_malformados_y_enormes` (falla con el código anterior) |
| 25 | Las pruebas de Windows corren con Defender sin tiempo real y en Windows Server 2025 | Requiere prueba real externa | Instalar y usar en Windows 11 con Defender y Smart App Control como vienen |
| 26 | Una coanfitriona cambiaba `server.properties` por el administrador remoto (por ejemplo, un paquete de recursos en un sitio suyo que ve las IPs de quienes lo descargan) | Corregido y verificado | Desde otra PC solo la propietaria puede escribirlo; prueba `mods_de_otra_pc_necesitan_aprobacion`. Mientras una coanfitriona **aloja**, controla su propia copia (limitación de diseño, ver la auditoría) |
| 27 | Actualizaciones: alguien publica una versión falsa, vieja o de otro canal | Mitigado (implementado, sin distribución activa) | Metadatos firmados por la clave del propietario, con secuencia, vencimiento y canal; ver [ACTUALIZACIONES.md](ACTUALIZACIONES.md). Riesgo residual: robo de la PC del propietario desbloqueada o de su archivo de recuperación |
| 28 | Malware con la misma cuenta de Windows cambia el instalador descargado entre la verificación y su ejecución | Limitación de diseño | Se verifica justo antes de abrirlo; un programa con la misma cuenta igual podría modificar la app instalada (es la misma frontera del riesgo 5) |
| 29 | `chrono` (fechas) figura como sin mantenimiento (RUSTSEC-2026-0543) | Pendiente | Sin vulnerabilidad conocida; cambiar a `jiff` o `time` es un trabajo grande que se hace aparte y con pruebas |
| 30 | La herramienta del propietario deja claves de prueba en el almacén de Windows de quien corre sus pruebas | Limitación (solo desarrollo) | En la CI los equipos se descartan; en una PC propia, borrar las claves `939Host-publicaciones-*` de prueba o correr las pruebas solo en la CI |

Historial: la tabla anterior (estados Corregido, Reducido, Reducible, Inevitable…) se reemplazó por
esta; ningún riesgo se quitó.

