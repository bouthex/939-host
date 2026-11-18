# Seguridad de 939 Host: diseño, modelo de amenazas y riesgos residuales

Este documento describe cómo se protege 939 Host. Lo que dice está implementado y probado en
el código. Las pruebas están en `crates/cluster/src/cert.rs`, `crates/cluster/src/signed.rs`,
`crates/cluster/tests/seguridad.rs`, `crates/core/src/files.rs` y `crates/core/src/modguard.rs`.

939 Host **no es invulnerable**, no tiene ninguna certificación y no promete seguridad absoluta.
Al final se listan los riesgos que siguen existiendo.

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
  archivos parciales.
- Los ids de servidor que llegan por la red se validan.

## Mods (código de terceros)

- Un `.jar` recibido de otra PC (por sincronización o por el administrador remoto) **no se
  carga** hasta que la propietaria apruebe su huella SHA-256. Si se reemplaza un mod aprobado,
  vuelve a la cuarentena.
- **No es un aislamiento.** Un mod aprobado puede hacer lo mismo que cualquier programa del
  usuario.
- El proceso de Minecraft corre en un Job Object sin acceso al portapapeles, al escritorio ni a
  los ajustes del sistema. Es endurecimiento parcial, **no una sandbox**.
- Se evaluaron cuentas separadas de Windows y AppContainer. No se implementaron: exigen permisos
  de administrador o rompen Java y los mods.

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
  `pruebas` (compilaciones de CI).
- **CurseForge:** no hay una forma permitida de usar su API sin una clave. 939 Host no incluye
  ninguna. Cada persona puede cargar la suya, que se guarda con DPAPI. Sin clave, se importan los
  modpacks descargados a mano desde la web de CurseForge.

## Modelo de amenazas (resumen STRIDE)

| Atacante | Qué puede lograr hoy | Qué lo impide |
|---|---|---|
| Externo de internet sin invitación | Conectarse al nodo si conoce su identidad, mandar pedidos y forzar trabajo | Toda operación exige ser del grupo; límite de pedidos y tamaño; secreto de 128 bits |
| Invitación filtrada | Unirse como el rol de la invitación si llega primero | Un solo uso, vence en 24 h, revocación, aviso a la propietaria, quitar PC |
| Jugadora maliciosa | Jugar, ver quién aloja y cuándo, ver la lista de integrantes | Sin mundo, sin archivos, sin direcciones ni hardware; no vota por otra, no emite permisos |
| Coanfitriona maliciosa | Recibe el mundo (lo necesita para alojarlo); puede apagar a la anfitriona con «Recuperar aquí» **solo en grupos de 2 PCs** | Permisos firmados; mods en cuarentena; no puede cambiar roles ni aprobar mods |
| PC del grupo comprometida | Lo mismo que su rol, con sus claves | Quitarla del grupo; recuperación con código si era la propietaria |
| Propietaria comprometida | Control total del grupo | Código de recuperación (gana sobre su clave); cambiar el código exige espera visible |
| Relay comprometido | Ver IPs, horarios y volumen | Cifrado de punta a punta; no puede leer ni modificar |
| Misma Wi-Fi | Antes: entrar directo al puerto de Minecraft. Ahora no (solo 127.0.0.1) | Escucha local; sin UPnP |
| Mod o archivo malicioso | Si se aprueba, todo lo que puede el usuario de Windows | Cuarentena por huella; aprobación explícita; Job Object |
| Robo de archivos locales | La clave del nodo está con DPAPI (otro usuario no la lee; con la sesión del usuario, sí) | DPAPI; quitar la PC y regenerar la identidad |

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
