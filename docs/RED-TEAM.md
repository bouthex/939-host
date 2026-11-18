# Informe de red team de 939 Host

Este informe es independiente de la auditoría defensiva (`docs/SEGURIDAD.md`). Cuenta qué se
intentó para vulnerar 939 Host, qué funcionó, cómo se corrigió y qué no se pudo comprobar.

939 Host **no es invulnerable**. Que las pruebas actuales no encuentren más fallas no prueba que
no las haya.

- **Rama:** `rediseno-v3`.
- **Commits:** la línea de base es `321ca9a`, antes del trabajo de seguridad. El estado auditado
  es el commit que agrega este informe.
- **Alcance:** solo 939 Host, en entornos aislados con datos ficticios:
  - nodos de prueba (`939node`) en una misma máquina;
  - dos máquinas Windows separadas de GitHub Actions;
  - relays públicos de n0 solo para comprobar el modo privado, sin atacarlos.
- **Qué no se atacó:** GitHub, Mojang, Modrinth, CurseForge, los servidores de n0, redes reales y
  la instalación de Minecraft del usuario.

## Resumen en números

| Concepto | Cantidad |
|---|---|
| Vueltas de red team | 6 (la sexta no encontró fallas nuevas en el código; agregó las pruebas que faltaban) |
| Pruebas automáticas que corren sin red (Linux) | 98, todas pasan |
| Pruebas que necesitan servidores reales, relays o Windows (CI de Windows) | 21, pasan en la CI |
| Pruebas ofensivas de grupo (`crates/cluster/tests/seguridad.rs`) | 10 pruebas con 28 comprobaciones de ataque |
| Variantes de permiso de anfitrión falsificado | 9 |
| Fallas nuevas encontradas por el red team (además de R1–R7) | 15 |
| Fallas nuevas corregidas | 15 |
| Fallas nuevas sin corregir | 0 (los riesgos de diseño que quedan están en «Riesgos residuales») |
| Regresiones de funcionamiento introducidas por las correcciones | 2, corregidas (ver abajo) |

Cada falla corregida tiene una prueba que reproduce el ataque. Esa prueba falla con el código
anterior y pasa con la corrección.

## Método

1. **Caja blanca:** se leyó cada manejador de pedidos de red (control, sincronización, unión,
   archivos, túnel de Minecraft y túnel de nombres) buscando qué decide quién manda cada dato.
2. **Nodos maliciosos:** el nodo de prueba tiene órdenes para:
   - falsificar permisos de anfitrión y votos;
   - repetir mensajes viejos;
   - mandar bytes malformados, enormes o desconocidos a los 5 protocolos;
   - anunciar direcciones falsas;
   - conocer direcciones por fuera del protocolo (`knowaddr`), para probar el peor caso.
3. **Grupos reales de 2 a 4 nodos** con roles mezclados, traspasos, PCs congeladas y
   reconexiones.
4. **CI de Windows:**
   - dos máquinas distintas por P2P;
   - servidores reales de Vanilla, Fabric, Forge y NeoForge;
   - cliente real de Minecraft;
   - relays reales para el modo privado;
   - búsqueda de secretos en el ejecutable.
5. **Interfaz (WebView2):** se revisó cada lugar donde se muestra texto que viene de otra PC, y
   la política de contenido (CSP).
6. **Dependencias:** `cargo audit`, acciones de GitHub fijadas y descargas verificadas.
7. **Repetición:** después de cada corrección se volvieron a correr todas las pruebas ofensivas y
   de funcionamiento. Una vuelta nueva usaba lo aprendido en la anterior.

No se usaron herramientas de fuzzing con cobertura (`cargo-fuzz`) ni análisis de memoria: el
código propio no usa `unsafe` fuera de las llamadas a DPAPI. Los mensajes malformados se
probaron con casos dirigidos, no con fuzzing aleatorio.

## Fallas encontradas por el red team

Gravedad: **alta** = permite ejecutar código, robar el mundo o tomar el servidor; **media** =
afecta a la privacidad, a la disponibilidad o a la integridad de una parte; **baja** = difícil de
aprovechar o de efecto acotado.

| # | Vuelta | Falla | Gravedad | Quién podía | Corrección | Prueba |
|---|---|---|---|---|---|---|
| 1 | 1 | Ids de servidor recibidos por la red sin validar: podían apuntar fuera de la carpeta | Alta | Cualquier integrante | Ids validados al entrar | `mensajes_malformados_y_enormes` |
| 2 | 1 | La cadena de permisos no se podía enviar (error de formato): una PC atrasada no podía ponerse al día | Media (disponibilidad) | Nadie a propósito; afectaba al grupo | Formato corregido | `pc_atrasada_verifica_la_cadena_completa` |
| 3 | 2 | Pedidos de hasta 8 MB leídos antes de saber si la PC era del grupo | Media (DoS) | Externo con la identidad del nodo | Límite de 512 KB | `mensajes_malformados_y_enormes` |
| 4 | 2 | Índices de puntos descomprimidos hasta 512 MB | Media (DoS) | Coanfitriona | Límite de 128 MB | `snapshots::tests::bomba_de_compresion_en_el_indice_se_rechaza` |
| 5 | 2 | Una coanfitriona podía activar RCON, query o el protocolo de administración desde `server.properties` | Alta | Coanfitriona | Apagados o solo locales en servidores del grupo | `properties::tests::otra_pc_no_abre_la_consola_remota_en_un_servidor_del_grupo` |
| 6 | 3 | Sincronización desde cualquier integrante: una jugadora anunciaba un punto «más nuevo que todos» y el resto lo copiaba | Alta | Jugadora | Solo de quien puede hostear y de épocas verificadas | `sync_sources::…` |
| 7 | 3 | El índice decía qué PC lo creó y se le creía: sus mods no contaban como recibidos y no pasaban por la cuarentena | Alta | Coanfitriona | Origen = identidad autenticada de quien manda | `snapshots::indice_ajeno_no_se_hace_pasar_por_local` |
| 8 | 3 | La cuarentena solo miraba `mods/*.jar` (Forge viejo carga subcarpetas y `.zip`) | Media | Coanfitriona | Recorre subcarpetas e incluye `.zip` | `modguard::t::received_jars_wait_for_approval` |
| 9 | 3 | Al unirse, una jugadora recibía las direcciones de todas las PCs (con IPs en modo directo) | Media (privacidad) | Jugadora | Solo la anfitriona | `invitaciones_vencidas_usadas_revocadas_y_manipuladas` |
| 10 | 3 | Secuencia de puntos con suma que desbordaba | Baja | Coanfitriona | Suma saturada | `snapshots::secuencia_enorme_no_desborda` |
| 11 | 3 | Subida cancelada: el temporal se veía un instante | Baja | Nadie a propósito | El cliente espera la confirmación | `administrador_de_archivos` |
| 12 | 3 | Una candidata podía votar con una época mayor que la verificada | Media | Coanfitriona | Rechazado | `sync_sources::voto_rechaza_epocas_no_verificadas_y_suplantaciones` |
| 13 | 4 | Desvío de la cuarentena: desde su PC, una coanfitriona reemplazaba los argumentos de Java de Forge, una librería, `server.jar` o el lanzador de Fabric, y su código corría en la anfitriona | Alta | Coanfitriona | Nada de lo que decide qué programa arranca se toca desde otra PC | `mods_de_otra_pc_necesitan_aprobacion` |
| 14 | 4 | Inundación de pedidos: solo la unión tenía límite | Media (DoS) | Cualquier integrante | Límites por PC y por canal | `mensajes_malformados_y_enormes` |
| 15 | 5 | Envenenamiento de direcciones: cualquier integrante podía reemplazar la dirección de otra PC por un relay propio o una IP ajena | Media (privacidad y DoS) | Jugadora | Dirección propia solo de esa PC; las de terceros solo de quien puede hostear y con relays de n0 | `direcciones_falsas_de_otras_pcs_no_se_aceptan` |

**Regresiones de funcionamiento introducidas por las correcciones, ya corregidas:**

- Una solicitud para alojar se rechazaba por datos viejos (lo encontró la CI de Windows con un
  traspaso lento). Ahora se vuelve a consultar a la PC antes de rechazar (`a31b65f`).
- Las invitaciones no fallaban cerrado si el sistema no daba números aleatorios. Ahora fallan
  cerrado (`6cde971`).

## Sexta vuelta (sin fallas nuevas en el código)

Tres correcciones de vueltas anteriores no tenían una prueba que reprodujera su ataque: la #4, la
#5 y la #12. Se agregaron. Cada una **falla si se quita la corrección** (se comprobó quitándola) y
pasa con ella. Para probar el voto (#12) sin depender de tiempos, sus condiciones pasaron a una
función sin estado (`vote_conditions`), con el mismo comportamiento.

| Qué se intentó | Resultado |
|---|---|
| Nombre de PC, de servidor, de archivo o de mod con HTML (`<img onerror=…>`) para ejecutar código en la interfaz de otra PC y llegar al backend | **No funciona.** Cada lugar que muestra datos de otra PC escapa el texto, y la CSP (`default-src 'self'`, sin `unsafe-inline` para scripts) bloquea scripts en línea. El nombre de PC solo se limpia de caracteres de control, así que la protección depende de esas dos capas. |
| Abrir sitios externos desde la interfaz | Solo dominios exactos de una lista (ya corregido en R15). |
| Subida de RAM de una jugadora (de 15 a 49 MB) en la medición | **Falsa alarma.** El entorno de pruebas tenía `RUST_BACKTRACE=1`, y un error esperado cargaba los símbolos de depuración. Sin esa variable: 15 MB. |
| Sincronización más lenta (0,60 a 0,80 s) | **No es una regresión.** La versión anterior a la vuelta 3 también da 0,80 s, y una posterior da 0,60 s. El valor salta de a 0,2 s según cuándo cae la consulta periódica. |

## Rendimiento antes y después

Medido con `ci/medir-seguridad.py`: tres nodos `939node` en la misma máquina (Linux), servidores
simulados y datos ficticios. Tres corridas de cada versión, en la misma máquina, la misma hora y
sin `RUST_BACKTRACE`. Se muestra la mediana y, entre paréntesis, el rango.

| Medida | Antes (`321ca9a`) | Ahora |
|---|---|---|
| Arranque del nodo | 7 ms | 6 ms |
| Unión al grupo con código | 128 ms (36–152) | 6 ms (6–7) |
| Punto de restauración de 64 MB | 213 ms (199–264) | 204 ms (184–209) |
| Sincronizar 64 MB a otra PC | 0,81 s (0,81–1,61) | 0,81 s (0,80–0,81) |
| Consulta de control (mediana) | 1,9 ms (1,8–2,9) | 1,9 ms (1,7–2,6) |
| Traspaso del anfitrión | 0,75 s (0,64–0,98) | 0,54 s (0,54–0,56) |
| RAM en reposo: propietaria, coanfitriona, jugadora | 20,9 / 22,2 / 15,6 MB | 21,7 / 23,0 / 15,1 MB |
| CPU en reposo: propietaria, coanfitriona, jugadora | 0,50 / 0,55 / 0,27 % | 0,45 / 0,42 / 0,25 % |
| Verificar un permiso de anfitrión | — | 62 µs |
| Verificar una lista firmada de 20 PCs | — | 91 µs |

- **La seguridad no tiene un costo medible** en estas pruebas.
- **La unión y el traspaso dan más rápidos que antes.** No se midió qué cambio lo produce, así que
  no se le atribuye a uno en particular.
- **Cada permiso se verifica una sola vez** al recibirlo. Después se usa el guardado; no se
  repiten verificaciones en segundo plano.
- **No se midió en Windows con Minecraft real.** La CI de Windows comprueba que todo funciona,
  pero no mide tiempos comparables.

## Matriz por perfil de atacante

| # | Perfil | Qué intentó | Qué consiguió antes | Qué consigue ahora |
|---|---|---|---|---|
| 1 | Externo sin invitación | Conectarse, mandar pedidos, mensajes enormes y malformados, adivinar códigos | Hacer que el nodo leyera 8 MB por pedido | Nada más que una respuesta de rechazo. Límite de tamaño y de pedidos; secreto de 128 bits con 5 intentos por minuto. Necesita conocer la identidad del nodo. |
| 2 | Invitación filtrada | Usarla antes que el destinatario, reutilizarla, modificarla | Entrar sin vencimiento | Entra **solo si llega primero y antes de 24 h**, con el rol de la invitación; la propietaria ve la unión y puede quitarla. Usada, vencida, revocada o modificada: rechazada. |
| 3 | Jugadora maliciosa | Bajar el mundo, leer archivos, ver IPs y hardware, falsificar permisos y votos, anunciar puntos y direcciones falsas | Bajar el mundo completo; hacer que el grupo copiara su punto; envenenar direcciones; tomar el servidor con un permiso sin firma | Jugar y ver la lista de integrantes y el historial de permisos. Todo lo demás se rechaza en el backend y queda auditado. |
| 4 | Coanfitriona maliciosa | Ejecutar código en la anfitriona, desviar la cuarentena, activar RCON, apagar a la anfitriona, cambiar roles, aprobar mods | Ejecutar código en la anfitriona (mods, argumentos de Java, librerías, RCON) y apagarla | Recibe el mundo (lo necesita para alojarlo) y puede alojar. **En grupos de 2 PCs** puede apagar a la anfitriona con «Recuperar aquí» (queda registrado). No ejecuta código en otra PC sin aprobación de la propietaria. |
| 5 | PC del grupo comprometida | Lo mismo que su rol, con sus claves | Lo mismo que el perfil 3 o 4 antes | Lo mismo que su rol hoy. Se corta quitándola del grupo. |
| 6 | Acceso a los archivos locales | Copiar `node.key`, la lista firmada, la clave de CurseForge | Lo mismo | Otro usuario de Windows no puede leer las claves (DPAPI). **Con la sesión del usuario, sí.** |
| 7 | Suplantar a la propietaria | Firmar listas, reenviar listas viejas, autodesignarse sucesora, crear códigos de recuperación | Reenviar una lista vieja para recuperar un rol revocado | Nada sin su clave o el código de recuperación. La sucesión exige designación previa y una espera de 72 h que la propietaria puede cancelar. |
| 8 | Relay comprometido | Leer o modificar el tráfico, asociar identidades con IPs | — | No lee ni modifica (TLS 1.3 de punta a punta, comprobado por diseño de iroh, no atacado en un relay propio). **Sí ve** IP, horarios y volumen de las PCs que pasan por él. |
| 9 | Manipulación de pedidos y mensajes | Replay de permisos y listas, épocas falsas, ids manipulados | Replay de listas; ids fuera de la carpeta | Rechazado: épocas y revisiones monotónicas, cadena verificada, ids validados. |
| 10 | Mod, archivo o modpack malicioso | Que se ejecute sin aprobación; escapar de la carpeta con un modpack | Se ejecutaba en la anfitriona | No se carga hasta que la propietaria aprueba su huella; los modpacks no escapan de la carpeta. **Un mod aprobado puede hacer todo lo que puede el usuario.** |
| 11 | Fallos de sincronización y transferencias | Puntos falsos, índices enormes, subidas cortadas | Imponer su punto al grupo; agotar memoria | Solo fuentes verificadas; límites de tamaño; sin parciales. Queda el riesgo residual 14 (punto falso de época anterior, acotado). |
| 12 | Permisos, estados inconsistentes y operaciones simultáneas | Dos anfitrionas a la vez, recuperación mientras la anfitriona funciona, PC congelada durante traspasos | Dos anfitrionas posibles con un permiso sin firma | Nunca dos anfitrionas en las pruebas; la PC atrasada verifica cada eslabón. |

## Qué ve cada participante de la red

| Modo | Otras PCs del grupo | Relay de n0 |
|---|---|---|
| Directo (sin relays) | Tu IP pública o local y tu puerto. **No hay forma de ocultarla: se conectan a vos.** | Nada |
| Normal (relay de respaldo) | Tu IP cuando la conexión pasa a directa | Tu IP, horarios y volumen mientras pasa por él |
| Privado (solo relay) | Ni tu IP pública ni la local (comprobado con relays reales en la CI) | Tu IP, horarios y volumen. Nunca el contenido. |

En todos los modos:

- Las otras PCs ven tu identidad, que es persistente: es tu clave.
- Ven cuándo estás conectado.
- Una jugadora no ve las direcciones de las demás PCs, salvo la de la anfitriona. En modo directo,
  eso incluye la IP de la anfitriona.

## Riesgos residuales

Los 14 riesgos de `docs/SEGURIDAD.md` siguen vigentes. Los más importantes:

1. **Un mod aprobado puede hacer todo lo que puede el usuario de Windows.** El Job Object es
   endurecimiento parcial, no una sandbox.
2. **Grupo de dos PCs:** la coanfitriona puede apagar a la anfitriona.
3. **La coanfitriona recibe el mundo completo.**
4. **El relay ve metadatos.** En modo normal o directo, la otra PC ve tu IP.
5. **DPAPI no protege contra malware en tu misma sesión.**
6. **La identidad de cada PC es persistente** y permite reconocerla entre sesiones.
7. **El ejecutable no está firmado.**

## Limitaciones de estas pruebas

- **Varias PCs:** las pruebas de grupo usan nodos en una misma máquina; las de dos máquinas
  reales son dos máquinas Windows de GitHub Actions, en la misma red de datos.
- **Redes y relays:** no se probó entre redes domésticas reales con NAT simétrico, ni con un
  relay propio comprometido. Que el relay no puede leer el contenido sale del diseño de iroh, no
  de un ataque al relay.
- **Interfaz:** no se probó contra WebView2 con un fuzzer; la revisión de la interfaz fue manual,
  más la CSP.
- **Mods:** no se probaron mods maliciosos reales; se probó que un archivo nuevo o modificado no
  se carga sin aprobación.
- **Fuzzing:** no hubo fuzzing con cobertura de los formatos de mensaje.
- **Mediciones:** son en Linux con nodos locales y servidores simulados, no en Windows con
  Minecraft real.
