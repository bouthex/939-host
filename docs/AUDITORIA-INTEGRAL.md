# Auditoría integral de seguridad: fase final

Fecha: 18 de noviembre de 2026. Rama `rediseno-v3`; parte del commit `a5c5df5`.

Este informe cierra las áreas que no estaban demostradas en `docs/SEGURIDAD.md` y
`docs/RED-TEAM.md`. No repite las seis vueltas anteriores: se apoya en sus pruebas.

939 Host **no es invulnerable** y no tiene ninguna certificación. Lo que sigue es lo que se pudo
demostrar con código y pruebas, y lo que no.

## 1. Qué ya estaba implementado (de fases anteriores)

| Área | Estado previo, comprobado en código y pruebas |
|---|---|
| Permiso de anfitrión | Firmado, por épocas, con cadena verificada eslabón por eslabón. Sin dos anfitrionas a la vez en las pruebas. |
| Lista de integrantes | Firmada por la propietaria; revisión monotónica y protección contra repetición. |
| Sincronización | Solo de PCs que pueden alojar y de épocas verificadas. |
| Cuarentena de mods | Por huella SHA-256 y aprobación firmada; corrección de la carrera de aprobación automática (falla 16). |
| Archivos | Rutas limpiadas, mayúsculas, nombres cortos 8.3, junctions y symlinks; mundo protegido con el servidor encendido. |
| Invitaciones | Un uso, vencimiento de 24 h, revocables y con límite de intentos. |
| Red | Modo privado opcional (apagado por defecto) solo por relay y sin caer a directo; UPnP/PCP/NAT-PMP apagados; límites de pedidos y de tamaño. |
| Minecraft | RCON, query y administración apagados o locales en servidores del grupo. |
| CI de Windows | Dos máquinas reales, servidores y cliente reales, app instalada con interfaz real. |

## 2. Qué se verificó en esta fase

### Windows

- **Privilegios.** La app se instala para el usuario y no pide administrador. El único paso con
  administrador era el ayudante del archivo `hosts`: **tenía una falla (V17, abajo)**.
- **Programas que se ejecutan:**
  - Java se lanza por ruta absoluta.
  - `taskkill`, `reg`, `explorer` y `powershell` se llamaban por nombre; Windows busca primero en
    la carpeta de la app. Corregido: ruta absoluta.
- **DLL hijacking.** El `.exe` importa `WebView2Loader.dll` y DLL del sistema que no están entre
  las «KnownDLLs» (`dwmapi`, `pdh`, `powrprof`, `iphlpapi`, `secur32`, …). Sin elevación no da
  privilegios nuevos: quien puede escribir en esa carpeta ya puede reemplazar `939Host.exe`.
  Con elevación sí: era V17.
- **Rutas especiales.** Comprobado en código y pruebas: UNC, `\\?\`, flujos alternativos (`:`),
  nombres reservados (`CON`, `NUL`, `COM1`…), punto o espacio final, comodines, junctions,
  symlinks y nombres cortos 8.3, con pruebas en Windows real.
- **Persistencia.**
  - El inicio con Windows viene apagado y solo lo activa el usuario.
  - El desinstalador quita el programa, los accesos, la clave de inicio y la de desinstalación.
  - Deja los datos a propósito, y también las líneas `.939host` del archivo `hosts` (inofensivas:
    borrarlas pide administrador).
- **IPC y WebView2.**
  - La CSP prohíbe scripts en línea.
  - Todo texto de otra PC se escapa. Se comprobó dibujando la sección nueva de solicitudes con
    nombres que contenían HTML: no se creó ningún elemento.
  - Los enlaces externos solo van a dominios exactos.
  - La app desactiva `msSmartScreenProtection` dentro de su propio WebView2. No cambia nada en
    Windows; ese WebView2 solo muestra la interfaz local y los enlaces se abren en el navegador.
- **Recursos.** La aprobación automática de mods leía todos los mods cada 5 s: corregido con una
  caché (abajo).
- **No se cambió ninguna configuración de Windows.** No se desactivó ninguna protección.

### Minecraft

- **Versiones vulnerables.** 939 Host ofrece todas las versiones oficiales. Las de 1.7 a 1.18.0
  tienen Log4Shell: **V18, corregida**.
- **Mods y modpacks.** Sistema de aprobación nuevo (sección 4). Se revisaron vías alternativas a
  la falla 16:
  - punto restaurado, subida, copia y renombrado remoto;
  - `.jar.disabled` renombrado a `.jar`;
  - subcarpetas y `.zip`;
  - instalación a mano de un archivo rechazado.

  Todas cubiertas.
- **Lo que decide qué programa arranca** (`server.jar`, librerías, argumentos de Java, lanzador
  de Fabric y ahora la protección contra Log4Shell) no se puede escribir desde otra PC.
- **Puertos, RCON, traspaso, sincronización, copias y corrupción del mundo.** Ya cubiertos por
  pruebas anteriores. En esta fase se volvieron a correr y siguen pasando.

### Red

Comprobado en código y en pruebas existentes:

- **UPnP, PCP y NAT-PMP:** desactivados en todos los modos.
- **Modo privado** (opcional, apagado por defecto):
  - no abre ningún transporte IP propio, solo relay;
  - sin relay no conecta, aunque haya camino directo;
  - con relays reales, ninguna otra PC conoce su IP (CI de Windows).
- **Modos normal y directo:** la otra PC ve tu IP; no hay forma de ocultarla en una conexión
  directa.
- **Abuso:** límites de tamaño, de pedidos y de intentos.

**No probado:** NAT estricto entre redes domésticas reales, y cortes de internet reales (solo
nodos congelados y reconexiones locales).

### Identidad y recuperación

| Característica | ¿Implementada? | Demostración |
|---|---|---|
| Firmas y validación de integrantes | Sí | `signed::only_owner_signatures_are_accepted` |
| Revocación (quitar una PC) | Sí | **Nueva:** `pc_quitada_pierde_el_acceso`. La PC quitada ya no administra archivos, no pide el servidor y no recibe el mundo nuevo. |
| Protección contra repetición | Sí | `old_lists_cannot_be_replayed`, `member_cannot_forge_or_replay_a_lease` |
| Código de recuperación opcional | Sí | `recovery_code_rules` y `recuperacion_con_codigo_y_sucesion` |
| Sucesora designada opcional | Sí | `succession_waits_and_owner_can_cancel` |
| Espera de 72 h | Sí, **72 h en la app** (`Timing::default`); 20 s solo en el nodo de prueba | `lost_code_reset_waits` y las pruebas de sucesión |
| Avisos y cancelación | Sí | Avisos en Grupo; `succcancel` en las pruebas |
| Una sola propietaria | Sí | `accept()` exige exactamente una propietaria |
| Pérdida o robo de la clave propietaria | Sí, **si se creó el código de recuperación** | `recovery_code_rules`: quien robó la clave no puede cambiar el código sin el código actual |
| Rotación de la clave de una PC | **No** como función. Se hace quitando la PC y volviéndola a invitar con identidad nueva, o recuperando la propiedad con el código. | — |

## 3. Vulnerabilidades encontradas en esta fase

| # | Falla | Gravedad | Quién podía | Estado |
|---|---|---|---|---|
| V17 | **El ayudante del archivo `hosts` se elevaba siendo el mismo `939Host.exe`, en una carpeta del usuario.** Windows cargaba DLL de esa carpeta: un programa que ya corriera con el usuario podía dejar una DLL y conseguir administrador cuando se aceptara el aviso de UAC. | Media (necesita malware previo con el usuario y que se acepte UAC; da administrador) | Malware local | **Corregida** |
| V18 | **Log4Shell en Minecraft 1.7 a 1.18.0.** Un texto `${jndi:…}` en el chat ejecutaba código en la PC anfitriona. | Alta para quien use esas versiones | Cualquier jugadora del grupo, o quien llegue al puerto | **Corregida** |
| V19 | Lo que llegaba de otra PC quedaba en la carpeta activa de mods hasta el próximo encendido (se apartaba al encender). | Baja (no se cargaba sin reiniciar, y al reiniciar se apartaba) | Coanfitriona | **Corregida**: va directo a la cuarentena |
| V20 | La propietaria leía todos los mods cada 5 s (segundos de CPU por vuelta con modpacks grandes). | Baja (rendimiento) | — | **Corregida** |
| V21 | `taskkill`, `reg`, `explorer` y `powershell` se ejecutaban por nombre. | Informativa (sin privilegios nuevos) | — | **Corregida** |
| G1 | Nombre de Minecraft real y su UUID, y una carpeta de usuario, en pruebas del repositorio. | Baja | — | Corregida en el código actual; **sigue en el historial** |
| G2 | `src-tauri/Cargo.lock` viejo con `zip 2.6.1` (no se usaba). | Informativa (falsas alarmas) | — | Borrado |

**Encontrado por la CI en esta misma fase:** el primer script elevado de `hosts` dejaba el bloque
sin cerrar. En PowerShell, `$e` (el bucle) y `$E` (la marca de cierre) son la misma variable. La
prueba de Windows lo detectó antes de que llegara a una PC real. Se corrigió, y la prueba de Linux
ahora rechaza variables que choquen sin distinguir mayúsculas (se comprobó con el script viejo).

## 4. Qué se corrigió y cómo

- **V17:** lo que corre como administrador es el PowerShell de `System32` (ruta absoluta, carpeta
  protegida), con un script fijo pasado como `-EncodedCommand`, no desde un archivo que se pueda
  cambiar en el medio.
  - El script vuelve a validar cada entrada (solo `127.93.*.*` y nombres `.939host`).
  - Deja el archivo igual que la lógica de Rust.
  - Se quitó el modo `--939-hosts` del ejecutable.
  - El aviso de UAC ahora dice «Windows PowerShell».
- **V18:** mitigación oficial de Mojang.
  - Configuración de Log4j para 1.7 a 1.16.5, descargada con SHA-1 verificado.
  - `-Dlog4j2.formatMsgNoLookups=true` para 1.17 a 1.18.0.
  - En cada encendido se verifica la huella: si falta o cambió, **no enciende**.
  - Desde otra PC no se puede escribir.
- **V19 y sistema de aprobación:** ver la sección siguiente.
- **V20:** caché de huellas por tamaño y fecha, solo para los recorridos periódicos.
  - Registrar lo recibido y el control antes de encender calculan siempre la huella completa.
  - Cada entrada desde otra PC vacía la caché.
  - 40 MB de mods: de 885 ms a 0,3 ms por vuelta.

### Sistema de aprobación y análisis de mods (sección 12 del pedido)

| Requisito | Cómo quedó |
|---|---|
| Cuarentena antes de la carpeta activa | Lo que llega de otra PC se coloca y se aparta dentro del mismo candado que lo registra. Nunca queda en `mods/`. |
| Notificación y solicitudes | Aviso a la propietaria cuando llega uno nuevo. **Mods → Solicitudes** muestra nombre, versión, tamaño, SHA-256, PC de origen, fecha, dependencias y análisis. |
| Aprobación ligada al archivo exacto | Por SHA-256, firmada por la propietaria; un byte distinto es otro archivo. Las actualizaciones necesitan aprobación nueva. |
| Rechazo | Firmado y permanente: no se carga nunca en ninguna PC, con ninguna política, aunque vuelva con otro nombre o se instale a mano. «Rechazar y borrar» lo quita de la cuarentena. |
| Sin aprobación automática por carreras, sincronización o mensajes | Candado de entradas (falla 16), firma de la propietaria sobre aprobados y rechazados, y revisión monotónica contra listas viejas. |
| La coanfitriona propone pero no aprueba | Aprobar y rechazar es solo de la propietaria. «Confiar en las coanfitrionas» es un permiso separado y visible (los rechazos siguen valiendo). |
| Registro de decisiones | Aprobado, rechazado, borrado y propio, con el estado del análisis; se consulta en Solicitudes. |
| Análisis multicapa sin ejecutar | `modscan` lee el `.jar` como zip: estructura (rutas que salen de la carpeta, cifrado, bombas de compresión, scripts y ejecutables, código nativo), metadatos y dependencias, jars incluidos (hasta 2 niveles), indicadores de fractureiser, robo de credenciales (Discord, navegadores, lanzadores, billeteras), webhooks, persistencia en Windows, identidad de 939 Host, propagación a otros `.jar`, carga de código desde internet, ofuscación y Log4j vulnerable incluido. |
| Fuentes externas | Modrinth por huella SHA-512 (oficial, sin clave, no sube el archivo). VirusTotal: botón que abre su página por huella en el navegador (sin clave y sin subir nada). |
| Estados | Sin alertas conocidas (solo con procedencia confirmada), desconocido, precaución, sospechoso, amenaza confirmada, análisis incompleto. Uno incompleto nunca se muestra como seguro. |
| Sin falsas alarmas | La red, los archivos de configuración y la reflexión solas no alertan; ejecutar comandos solo es «precaución».

En la CI de Windows se analizaron mods reales. Ninguno salió como sospechoso o amenaza:

- Lithium y FerriteCore, en Fabric y NeoForge: «sin alertas conocidas».
- Krypton: «precaución», por su código nativo de red, que es legítimo.

La primera corrida marcó «puede ejecutar programas» en Fabric API y Chunky. La regla confundía
`Executor` y `execute` con `exec`. Ahora busca las entradas exactas del archivo `.class` y hay una
prueba para ese caso. |
| Fallas del analizador | El análisis solo informa: **no decide** qué se carga. Ninguna falla del analizador, de Modrinth o de la red puede hacer que un archivo no aprobado se ejecute. |

**Lo que no hace:**

- **Análisis aislado (sandbox): no se implementó.** Windows Sandbox necesita Windows Pro y
  activar una función del sistema, y no se cambian configuraciones de Windows. Un AppContainer no
  aísla de verdad a Java con mods. En su lugar, el mod **nunca se ejecuta** durante el análisis.
- **VirusTotal y CurseForge por API: no.** Las dos necesitan una clave.
- **Ofuscación avanzada.** Un autor que cifra sus cadenas o arma los nombres en tiempo de
  ejecución puede esconderse del análisis estático.

## 5. Pruebas agregadas en esta fase

| Prueba | Qué demuestra |
|---|---|
| `modscan::t::mod_normal_sin_falsas_alarmas` | Un mod normal (red, reflexión, configuración) no alerta; comandos = precaución; nativo = precaución. |
| `modscan::t::malware_conocido_y_robo_de_credenciales` | Fractureiser = amenaza. Robo de credenciales, webhooks, persistencia, identidad de 939 Host, carga remota, propagación, scripts y zip slip = sospechoso. |
| `modscan::t::ofuscacion_y_dependencias_maliciosas` | Ofuscación sola = precaución; con comandos = sospechoso. Ladrón dentro de un jar incluido, con la ruta exacta. Log4j vulnerable. |
| `modscan::t::analisis_incompleto_nunca_es_seguro` | Archivo dañado y bomba de compresión = incompleto; sin procedencia nunca es «sin alertas». |
| `modscan::t::sin_procedencia_no_hay_sin_alertas` | Con Modrinth caído o sin el archivo, el estado es «desconocido». |
| `modguard::t::lo_recibido_va_directo_a_la_cuarentena` | Nunca en la carpeta activa; un byte distinto anula la aprobación; subcarpetas y `.zip`; las subidas a medias no son mods. |
| `modguard::t::un_rechazo_vale_siempre` | Ni con confianza total, ni instalado a mano, ni aprobado a la vez. |
| `modguard::t::procedencia_y_registro_de_decisiones` | PC de origen, tamaño, fecha; registro con tope de 500. |
| `modguard::t::cache_de_huellas_segura_y_barata` | La caché no confunde contenido de otra PC con el mismo tamaño y fecha; la vuelta siguiente es barata. |
| `seguridad::rechazo_y_cuarentena_inmediata` (nodos reales) | Lo subido por la coanfitriona queda apartado al instante, con su procedencia. La coanfitriona no aprueba ni rechaza. El rechazo viaja firmado y vale en otra PC aunque se instale a mano. |
| `seguridad::pc_quitada_pierde_el_acceso` (nodos reales) | Revocación efectiva. |
| `alias::t::script_elevado_solo_lleva_entradas_validadas` | El script elevado no acepta inyecciones ni IPs ajenas. |
| `direcciones::script_elevado_escribe_igual_que_rust` (CI de Windows) | El script real de PowerShell deja `hosts` igual que Rust, rechaza entradas ajenas y restaura el archivo. |
| `log4shell::t::*` | Versión → protección (versiones finales, pre-releases y snapshots); no enciende sin el archivo original. |
| `e2e_log4shell_archivos` (CI de Windows) | Los archivos reales de Mojang se descargan con la huella del código. |
| Falsos positivos con mods reales (CI de Windows) | Lithium y los mods de rendimiento reales no salen como sospechosos. |

Se comprobó quitando la corrección que su prueba falla en: cuarentena inmediata, rechazo,
protección contra Log4Shell (esta fase) y la carrera de la aprobación automática (fase anterior).
El script elevado de `hosts` solo se puede ejecutar en Windows: lo prueba la CI.

## 6. Resultados

- **Linux (este entorno):** 115 pruebas automáticas, todas pasan.
- **CI de Windows:** las 6 partes pasaron con `30eb8dc`:
  - dos máquinas reales, servidores y cliente reales, app instalada;
  - script elevado de `hosts` en Windows real;
  - archivos de Mojang contra Log4Shell;
  - revocación, rechazo y cuarentena inmediata con nodos reales.
- **Mods reales sin falsos positivos:** Lithium, FerriteCore, Fabric API y Chunky dan «sin
  alertas conocidas»; Krypton da «precaución» por su código nativo.
- **Instalador de prueba de `30eb8dc`:** artefacto `939Host-windows-prueba` en Actions, vence el
  2 de diciembre.
  - `939Host-0.9.0-instalador.exe`:
    `222961b89d24eff0cf36e326249118dc3f94d4abaa6f77a26e631b107a7ad3fc`
  - `939Host-0.9.0-portable.zip`:
    `e70485f7f062758ecd2d8aa22041be0ae6ccbc94a2da8a55015b254347039486`
- **Rendimiento:** sin cambios medibles en reposo, sincronización, traspaso ni uniones.

| Medida (mediana de 3) | Ayer | Ahora |
|---|---|---|
| RAM en reposo (propietaria / coanfitriona / jugadora) | 21,7 / 23,0 / 15,1 MB | 21,9 / 22,6 / 15,3 MB |
| CPU en reposo | 0,45 / 0,42 / 0,25 % | 0,47 / 0,43 / 0,27 % |
| Sincronizar 64 MB | 0,81 s | 0,61 s (0,60–0,81: salta según la consulta periódica) |
| Traspaso | 0,54 s | 0,54 s |

## 7. Qué no se pudo comprobar

- Windows real con antivirus: si Windows Defender u otro antivirus marcan el
  `powershell -EncodedCommand` elevado de las direcciones `.939host`. En la CI funciona; en una PC
  con Defender activo, no se probó.
- El aviso de UAC real: la CI corre como administrador sin aviso.
- NAT estricto entre redes domésticas reales y un relay propio comprometido.
- Mods maliciosos reales (se usaron muestras armadas con los indicadores, no malware).
- Los logs de las 36 corridas de Actions no se leyeron uno por uno; tampoco se pudo descargar
  ningún artefacto desde esta sesión.
- Qué secretos están cargados en GitHub: esta conexión no puede listarlos.

## 8. Riesgos que siguen

1. **Un mod aprobado puede hacer todo lo que puede el usuario de Windows.** El análisis ayuda,
   pero no detecta todo.
2. **Grupo de dos PCs:** la coanfitriona puede apagar a la anfitriona con «Recuperar aquí».
3. **La coanfitriona recibe el mundo completo** (lo necesita para alojarlo).
4. **En modo normal o directo, la otra PC ve tu IP.** El relay ve metadatos.
5. **DPAPI no protege contra malware en tu misma sesión.**
6. **La identidad de cada PC es persistente.**
7. **El ejecutable no está firmado.**
8. **Las versiones viejas de Minecraft** tienen otras fallas además de Log4Shell (por ejemplo,
   paquetes que tiran el servidor). Conviene usar versiones actuales.
9. **Datos personales en el historial de Git** (G1, baja sensibilidad).

## 9. GitHub y datos sensibles

Detalle en [`AUDITORIA-GITHUB.md`](AUDITORIA-GITHUB.md).

- **Secretos:** ni claves, ni tokens, ni contraseñas, ni credenciales en el historial, la release
  ni los logs.
- **CurseForge:** el secreto `CURSEFORGE_API_KEY` estaba vacío en las compilaciones que lo
  habrían metido en el `.exe`.
- **SignPath:** el workflow de firma nunca corrió.
- **Datos personales:** se encontraron un nombre de Minecraft con su UUID y una carpeta de usuario
  en pruebas; quitados del código actual, siguen en el historial.
- **Historial:** no hace falta reescribirlo.

## 10. Pendientes y decisiones tuyas

1. **CurseForge sin clave personal y sin pedirle una clave a cada usuario: no hay una forma
   permitida.** La API oficial pide una clave que se solicita a Overwolf, que es intransferible y
   no puede compartirse. Opciones reales:
   - **a) (lo que hay hoy)** Importar mods y modpacks descargados a mano desde la web de
     CurseForge, y una clave propia opcional, guardada con DPAPI, para quien quiera.
   - **b)** Pedir una clave para 939 Host como proyecto (gratis, con revisión de Overwolf) y
     meterla en las compilaciones. Contradice «ninguna clave dentro del EXE» y sus términos
     prohíben divulgarla: no se recomienda.
   - **c)** Un intermediario propio que guarde la clave. Contradice «sin infraestructura propia».
   - **Recomendación:** seguir con (a) y preferir Modrinth cuando el mismo mod está ahí.
2. **Historial de Git:** no reescribirlo (se puede, con tu autorización, pero es destructivo).
3. **Secreto `CURSEFORGE_API_KEY`:** si existe en Settings → Secrets, conviene borrarlo (no se
   usa).
4. **Firma del ejecutable:** sigue pendiente la aprobación de SignPath.

## 11. Qué falta para las pruebas finales con usuarios

- [x] CI de Windows verde con todos los commits de esta fase (`30eb8dc`).
- [ ] Probar en una PC con Windows Defender activo el botón de direcciones `.939host` (aviso de
  UAC de «Windows PowerShell»).
- [ ] Probar en dos o tres PCs reales de redes distintas: unirse, jugar, traspaso, modo privado.
- [ ] Probar el flujo de Solicitudes con un mod real subido por una coanfitriona.
- [ ] Decidir la opción de CurseForge.
- [ ] Ejecutable firmado o, mientras tanto, instrucciones claras para SmartScreen (sin desactivar
  protecciones).
