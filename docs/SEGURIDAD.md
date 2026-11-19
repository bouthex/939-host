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
| Ventana cerrada con un servidor encendido | `939 Host.exe` en la bandeja + `java.exe` dentro de un Job Object | Minecraft escucha en `127.0.0.1` (o en la dirección `.939host` de loopback); la voz, en esa misma dirección por UDP. Con «Acceso por LAN» activado, también en la red local | Lo mismo que arriba. Nada de Minecraft queda en internet salvo que la persona configure una dirección pública propia (playit.gg, router) |
| Cerrar 939 Host mientras aloja | Primero traspasa o suelta cada servidor con el último estado guardado, después detiene Java | Se cierran todos | Nada |

Datos locales (todos dentro de `%LOCALAPPDATA%\939Host`):

- Clave del nodo protegida con DPAPI; configuración, integrantes firmados, historial de permisos.
- Mundos, puntos de restauración y copias (con tope configurable de espacio).
- Amigos: lista, solicitudes, bloqueados y chats (como mucho 500 mensajes por chat). Sin cifrado
  propio en disco: se protege con los permisos de la cuenta de Windows.
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
  anuncia la anfitriona y se muestra como tal. Los OP para coanfitrionas viajan en la
  configuración firmada por la propietaria: una coanfitriona no puede darse OP a sí misma.
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

## Riesgos residuales

1. **Mods aprobados.** Pueden hacer todo lo que puede el usuario de Windows. Minecraft no se
   puede aislar sin romper el juego.
2. **Grupo de dos PCs.** La coanfitriona puede apagar a la anfitriona con «Recuperar aquí»: no
   hay terceros que confirmen. Queda registrado y la propietaria puede revertirlo.
3. **Una coanfitriona recibe el mundo completo** (lo necesita para alojarlo), incluido
   `server.properties` y la carpeta de jugadores.
4. **El relay ve metadatos** (IP, horarios, volumen). En modo normal, una conexión directa
   muestra la IP a la otra PC.
5. **DPAPI protege contra otros usuarios, no contra malware** que corra con tu misma sesión.
6. **Las jugadoras ven el historial de permisos** (qué PC alojó y cuándo). Lo necesitan para
   verificar la cadena.
7. **Las esperas de 72 h usan el reloj de cada PC.** Una PC con el reloj adelantado vería
   terminar la espera antes.
8. **Una invitación filtrada usada antes que el destinatario** da acceso con ese rol hasta que la
   propietaria quite la PC.
9. **El ejecutable no está firmado digitalmente** (la firma depende de SignPath).
10. **Todas las PCs del grupo tienen que tener esta versión:** los permisos sin firma de versiones
    anteriores se rechazan.
11. **El nombre de un mod pendiente lo elige quien lo subió.** La propietaria aprueba una huella;
    un mod malicioso puede llamarse como uno conocido. Aprobar es confiar en el origen.
12. **Una coanfitriona elige qué anuncia de sí misma** (frescura de su copia, puntaje): puede
    quedar primera para alojar si la anfitriona se cae. Puede alojar igual por su rol.
13. **Configuración de mods aprobados:** algunos mods ejecutan scripts desde `config/` (por
    ejemplo, KubeJS). Si la propietaria aprueba un mod así, una coanfitriona puede cambiar lo que
    hace subiendo archivos de configuración.
14. **Una coanfitriona puede ofrecer un punto falso de una época anterior.** No gana mientras la
    anfitriona cree puntos en la época actual (con el servidor encendido, cada 15 minutos por
    defecto, y siempre antes de un traspaso).
15. **Traspaso automático con el servidor detenido.** Si la PC que tiene el permiso no está
    alojando, acepta sola un pedido de otra PC con permiso para alojar. Con el servidor
    encendido, siempre pregunta.
16. **La anfitriona controla su Minecraft mientras aloja.** Desde la consola puede dar OP a
    cualquiera en esa sesión; la lista firmada se vuelve a comparar con `ops.json` en cada vuelta de control mientras aloja; un OP agregado a mano fuera de esa lista no se quita solo.
17. **La voz solo funciona por la dirección `.939host`.** Quien entra por una dirección pública
    propia (playit.gg, router) necesita además el puerto UDP de Simple Voice Chat abierto ahí.
18. **Si el proceso de 939 Host muere de golpe** (Administrador de tareas, corte de luz), el Job
    Object cierra Java sin guardar: se pierde lo jugado desde el último guardado de Minecraft o el
    último punto (cada 15 minutos por defecto).
19. **Apagar Windows con un servidor encendido** da poco tiempo: Windows puede cortar antes de que
    termine el guardado. Conviene detener el servidor antes.
20. **Los chats de amigos se guardan sin cifrado propio** en la carpeta del usuario.
21. **Los límites contra abuso frenan el volumen, no la existencia de pedidos:** una PC que conoce
    la identidad del nodo puede seguir abriendo conexiones (cada una cuesta un saludo TLS).
