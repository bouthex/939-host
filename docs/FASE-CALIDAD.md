# 939 Host — Fase final de calidad, seguridad, automatización y confiabilidad

Rama: `rediseno-v3` (sin merge, sin PR, sin release, sin cambios de configuración de GitHub).
Punto de partida comprobado: `a43aedf` (local y remoto iguales, árbol limpio).

Cómo leer las marcas:

- **[auto]** probado con pruebas automáticas en Linux (este entorno);
- **[Windows CI]** probado en las máquinas Windows de GitHub Actions, con servidores y clientes
  reales de Minecraft;
- **[estático]** revisado con análisis estático (clippy, auditoría de dependencias, escaneo de
  secretos);
- **[manual]** pendiente de prueba manual;
- **[dos PCs reales]**: nada de esta fase se probó en dos casas reales.

---

## 1. Cambios realizados

### Guardado y cierre (§3, §4, §5)

- **Guardián ante cierres inesperados** (`host939_core::guardian`). Al encender el primer servidor,
  939 Host abre un proceso chico (el mismo ejecutable con `--guardian`, sin ventana, sin red, sin
  privilegios) y le duplica el Job Object, el proceso de Java y la entrada de su consola. Si 939
  Host termina de golpe (Administrador de tareas, fallo, «Cerrar igual»), el guardián le manda
  `stop` a cada Java: Minecraft guarda y se detiene igual que en una detención normal. Espera con
  la misma regla (hasta 10 minutos si sigue guardando), anota el resultado y termina. Java nunca
  queda sin supervisión: si el guardián también muere, el Job Object cierra Java como antes.
  **[Windows CI]**: matar el proceso principal con el servidor encendido → Java guardó y se detuvo
  en 1 s, no quedó ningún proceso; al reabrir, aviso y el servidor vuelve a encender.
- **Aviso al reabrir** de qué pasó con cada servidor (guardado, detenido sin confirmación o cerrado
  a la fuerza) y, si el servidor era compartido, publicación del estado final al grupo.
- **Apagado, reinicio y cierre de sesión de Windows**. Una ventana invisible atiende
  `WM_QUERYENDSESSION`. Con servidores encendidos pide tiempo con `ShutdownBlockReasonCreate`
  («Guardando el mundo de Minecraft…», visible en la pantalla de apagado, con «Apagar de todos
  modos» siempre disponible), hace el cierre ordenado y libera a Windows; nunca más de 3 minutos.
  Sin servidores no demora el apagado. **[Windows CI]** con el aviso simulado: se comprueba que
  Windows tiene registrado el motivo, que el proceso sigue vivo hasta terminar, que el mundo se
  guarda, el tope de 180 s y que el registro final es del cierre ordenado (no del guardián).
  **[manual]** un apagado real.
- **«Salir» y el apagado terminaban el proceso a mitad del guardado** (error encontrado por la
  prueba de apagado simulado de la noche del 19/11): la ventana se cierra al instante y, sin
  ventanas, Tauri pedía salir; durante el cierre no se impedía, así que el proceso moría mientras
  todavía detenía los servidores. El guardián salvó el mundo, pero se salteaban el traspaso y la
  publicación al grupo; antes del guardián, Java se habría cerrado sin guardar. Afectaba también a
  «Salir» desde la bandeja. Ahora solo se sale cuando el cierre ordenado termina.
- **Estado «Guardando» atascado con Minecraft 26.3** (error encontrado por la nueva prueba):
  26.3 escribe «Saving chunks for level…» al arrancar sin mensaje de cierre y la interfaz quedaba
  en «Guardando». Ahora solo cuenta «Saving the game». **[auto]** **[Windows CI]**
- El veredicto del guardián usa el código de salida de Java, porque 26.3 ya no escribe «All
  dimensions are saved» (comprobado con el servidor real de 26.3).
- **Tras un cierre inesperado en un servidor compartido, lo que guardó el guardián llega al grupo**
  (prueba multi-PC nueva `tras_un_cierre_inesperado_se_publica_lo_que_guardo_el_guardian`). La
  prueba encontró que **el grupo no volvía a conectarse con una PC que se reinició rápido**: con
  iroh 1.4, si una PC vuelve con la misma identidad en otro puerto antes de que la conexión vieja
  venza, las demás seguían usando el camino viejo para toda conexión nueva (consultar, copiar el
  mundo, y probablemente el túnel de Minecraft). Corregido sin APIs inestables: las conexiones de
  control sirven en los dos sentidos, las viejas se cierran y se le pide a la PC reiniciada una
  conexión nueva que hace que el transporte vuelva a elegir el camino. También: una copia fallida
  ahora reabre la conexión, y el estado final se reenvía a las PCs que no respondieron (antes el
  aviso decía «enviado» aunque no hubiera llegado). Ninguna de estas fallas perdía el mundo (la
  PC que recibe el servidor copia el estado al recibirlo), pero el grupo podía ver un estado viejo.
- Lo que ya existía y se revisó: cada punto de restauración, copia y traspaso hace `save-off` →
  `save-all flush` → espera la confirmación de Minecraft → copia → `save-on`; el mundo de la línea
  solo lo escribe la anfitriona; una copia vieja nunca reemplaza un estado más nuevo (fase
  anterior, prueba `tierra_e_inventario_no_retroceden`).

### Seguridad (§6, §7, §8, §9)

- **Chats y lista de amigos cifrados con DPAPI** (`host939_core::sealed`). Migración automática
  de los `.json` en texto plano (se quitan solo después de verificar la copia cifrada). Un archivo
  que no se puede descifrar (otra cuenta de Windows, Windows reinstalado, archivo dañado) se aparta
  como `*.ilegible-<fecha>`, no se borra, y Amigos muestra un aviso que lo explica. **[auto]** y
  **[Windows CI]** con DPAPI real (`cifra_migra_y_aparta_lo_ilegible`: en Windows comprueba que el
  texto no aparece en el disco) y `amigos_ilegibles_se_apartan_con_aviso`.
- **OP de Minecraft coherente con la lista firmada**. Autorizados = OP concedidos a integrantes +
  otras cuentas que la propietaria autorizó (al principio, lo que ya tenía su `ops.json`: nunca se
  quitan OP preexistentes). La anfitriona da OP a quien falte y quita, con aviso y después de un
  margen, cualquier OP fuera de la lista (consola de una coanfitriona, `/op` en el juego). Un
  `op`/`deop` desde la consola o Jugadores de la propietaria actualiza la lista firmada. `ops.json`
  no se puede subir desde otra PC. **[auto]**: la prueba multi-PC cubre concesión, traspaso,
  revocación, un OP que se da la coanfitriona desde la consola (revertido) y un deop de un OP
  autorizado (restaurado). Encontró y se corrigió una carrera: tras un traspaso la nueva anfitriona
  quitaba un OP legítimo porque la lista firmada todavía no le había llegado.
- **Auditoría de los riesgos**: los 21 de `SEGURIDAD.md` más dos nuevos, clasificados uno por uno
  (sección 3).

### Almacenamiento (§11)

- El tope de copias **nunca deja solo un punto dañado**: antes de borrar puntos verifica que el que
  queda esté íntegro; si no, no borra ninguno. **[auto]** (prueba que daña el almacén).
- **Limpieza automática de temporales** de operaciones cortadas (escrituras atómicas, subidas y
  copias a medias), solo los de más de 6 horas y nunca dentro de las regiones del mundo; al abrir y
  cada 6 horas. **[auto]**
- Lo que ya existía: puntos incrementales y deduplicados por bloques SHA-256, copias manuales y
  protegidas que nunca se borran solas, verificación de integridad y barra de espacio usado.

### Rendimiento (§10)

- **Java ya no reserva toda la memoria al arrancar** (perfil equilibrado). Antes: `Xms = Xmx` con
  `AlwaysPreTouch`, y un mundo vacío con 6 GB ocupaba 6,5 GB de memoria privada desde el primer
  segundo (medido en la CI). Ahora el heap empieza en un cuarto del máximo (al menos 1 GB) y crece
  cuando hace falta; el máximo no cambia. El perfil de baja latencia (ZGC), que se elige a
  propósito, conserva la reserva completa.
  - **[Windows CI]** misma prueba, mismo máximo de 6 GB: Java pasó de **6.536 MB a 1.822 MB** de
    memoria privada con el servidor encendido.
  - **Prueba de carga con el servidor real de 26.3** (Linux, 2 CPU, máximo 4 GB, mismos parámetros
    de G1, mundo nuevo con semilla fija):

    | | Viejo (4 GB reservados y tocados) | Nuevo (empieza en 1 GB) |
    |---|---|---|
    | Memoria en reposo | 4.410 MB | 816 MB |
    | Con 1.024 chunks cargados | 4.470 MB | 1.017 MB |
    | Con 4.096 chunks cargados | 4.513 MB | 1.303 MB |
    | Pausas reales de GC (carga pesada) | 32, máx. 232 ms, media 47 ms | 91, máx. 105 ms, media 27 ms |
    | Tiempo total en GC | 0,3 % | 0,4 % |
    | Tick medio con 1.024 chunks | 5,9 ms | 6,2 ms |

    Con el heap nuevo hay más recolecciones, pero más cortas (la pausa máxima bajó a menos de la
    mitad); el tiempo total en GC es casi igual y el tick queda muy lejos del límite de 50 ms.
  - **Crece cuando lo necesita**: arrancado a propósito con 256 MB bajo carga, el heap creció solo
    hasta 1.536 MB. Con 4.096 chunks, 1 GB alcanzó (unos 500 MB vivos).
  - Límite de la medición: una sola máquina de 2 CPU, sin jugadores reales ni granjas; las cifras
    de tick tienen ruido (la generación de mundo compite por la CPU).

### Desinstalación (§12)

- `Desinstalar.exe /S /BORRARDATOS` borra los datos solo si se pide así, de forma explícita; en la
  interfaz siguen las dos confirmaciones (por defecto se conservan).
- **[Windows CI]**: se prueban los dos casos. En ambos no quedan programa, procesos, inicio con
  Windows, entrada de Programas instalados, accesos del menú Inicio, caché de la interfaz, bloque
  en `hosts`, tareas programadas, servicios ni reglas de Firewall. Conservando datos quedan el
  mundo (`level.dat`), las copias de seguridad y los puntos de restauración (comprobación agregada
  el 19/11); con `/BORRARDATOS`, no queda nada.
- La confirmación clara para borrar datos es la doble pregunta del desinstalador con interfaz (por
  defecto conserva). En modo silencioso solo se borra con `/BORRARDATOS` escrito a propósito. La
  doble pregunta con interfaz no se puede automatizar en la CI: **[manual]**.

### Direcciones y privacidad (§13, §16)

- El diálogo de «Jugadores sin 939 Host» ahora dice que **un nombre no oculta la IP por sí solo**
  (un dominio o DNS dinámico que apunta a la casa la revela), que **939 Host no comprueba** la
  dirección guardada, y que abrir un puerto en el router además exige «Acceso por LAN».
- **§16 queda pendiente de infraestructura, no descartado**: `docs/DIRECCIONES-PUBLICAS.md`
  separa qué existe (`.939host`, túnel P2P, relays de iroh, dirección pública mostrada) de lo que
  falta (un punto de entrada público y un relay TCP de Minecraft con registro firmado), por qué
  `.939` no puede ser un dominio real, qué vería quien opere el relay, las opciones investigadas y
  las 10 pruebas obligatorias antes de declararlo listo.

## 2. Problemas corregidos

| Problema | Cómo se encontró | Estado |
|---|---|---|
| Java se cerraba sin guardar si 939 Host moría | Riesgo 18 | Corregido, probado en Windows |
| Apagar Windows con un servidor encendido no esperaba el guardado | Riesgo 19 | Corregido, probado con el aviso simulado |
| Chats y códigos de invitación en texto plano | Riesgo 20 | Corregido |
| OP fuera de la lista autorizada | Riesgo 16 | Corregido |
| Carrera que quitaba un OP legítimo tras un traspaso | Prueba nueva | Corregido |
| «Guardando» para siempre con Minecraft 26.3 | Prueba nueva de Windows | Corregido |
| El tope de copias podía dejar solo un punto dañado | Revisión | Corregido |
| Temporales abandonados tras un corte | Revisión | Corregido |
| Java reservaba toda la memoria al arrancar | Medición en la CI | Corregido (perfil equilibrado) |
| El diálogo de dirección pública sugería más de lo que hace | Verificación anterior | Corregido |
| «Salir» y el apagado terminaban el proceso a mitad del guardado | Prueba de apagado simulado (CI de la noche) | Corregido; la prueba lo verifica |
| El grupo no volvía a conectarse con una PC reiniciada rápido (camino viejo de iroh) | Prueba multi-PC nueva | Corregido |
| El estado final no se reintentaba y el aviso decía «enviado» igual | Prueba multi-PC nueva | Corregido |
| Una copia fallida no reabría la conexión | Prueba multi-PC nueva | Corregido |
| Archivos de amigos ilegibles sin explicación | Revisión del cifrado | Aviso agregado |
| `SEGURIDAD.md` describía mal «Cerrar 939 Host mientras aloja» | Verificación anterior | Corregido |

## 3. Riesgos restantes

La tabla completa, riesgo por riesgo, está en [`SEGURIDAD.md`](SEGURIDAD.md#riesgos-residuales-revisados-uno-por-uno-en-la-fase-de-calidad). Resumen:

- **Corregidos:** 16 (OP), 18 (cierre inesperado; salvo corte de luz), 19 (apagado de Windows;
  falta un apagado real) y 20 (chats).
- **Reducidos:** 8 (invitación filtrada) y 14 (punto falso de época anterior).
- **Reducibles, con el camino anotado:** 2, 7, 11, 12 y 13.
- **Inevitables:** 1, 3, 4, 5, 6, 10, 15 (decisión de producto), 21 y 23 («Finalizar árbol de
  procesos» también termina al guardián).
- **Pendientes de infraestructura:** 9 (firma del ejecutable), 17 y 22 (jugadores sin 939 Host
  sin revelar la IP de la casa).
- **Pendientes de pruebas reales:** apagado real de Windows, dos casas, UAC, Defender y Smart App
  Control en una PC de usuario.

## 4. Mediciones de rendimiento [Windows CI]

Máquina de GitHub Actions con Windows Server. Memoria privada = lo que el proceso ocupa de verdad.

| Situación | 939 Host (app + WebView2 + guardián) | CPU | Notas |
|---|---|---|---|
| Arranque hasta que la app se puede usar | — | — | 3,6 a 7,2 s según la corrida |
| Ventana abierta, en reposo | 117–119 MB | 0,0–0,16 % | 7 procesos (app y WebView2) |
| Minimizada | 119–120 MB | 0,0–0,31 % | |
| **Bandeja con un servidor encendido** | **21,6–23,5 MB** | **0,0–0,47 %** | La ventana se destruye; el guardián ocupa 1,7 MB |
| Después de recorrer todas las pantallas | 132–215 MB | 0,6–0,9 % | El WebView2 crece con el uso; vuelve a bajar al cerrar la ventana |
| Java con 6 GB de máximo, antes del cambio | — | — | 6.536 MB privados con un mundo vacío |
| Java con 6 GB de máximo, después del cambio | — | — | **1.822 MB** privados (corrida 44480263331) |

Presupuestos que se proponen para no retroceder: ventana abierta en reposo < 150 MB y < 1 % de CPU;
en la bandeja < 40 MB y < 0,5 %; guardián < 5 MB; arranque < 10 s.

Actividad en segundo plano (revisada en el código): presencia de amigos cada 30 s, control del
grupo cada 5 s solo para servidores compartidos, revisión de la bandeja cada 10 s, Papelera y
temporales cada 6 horas. Sin consultas a disco constantes del mundo; los puntos son incrementales.

## 5. Consumo de almacenamiento [Windows CI]

Instalado: 14,6 MB (ejecutable 13,9 MB). Datos tras la prueba completa: 371 MB, de los cuales
130 MB son el servidor de prueba (mundo, Java del servidor, copias) y 61 MB la caché. Los puntos de
restauración son incrementales y deduplicados; el tope de espacio es configurable.

## 6. Pruebas ejecutadas

**[auto] Linux, último estado:**

| Conjunto | Resultado |
|---|---|
| `host939-core` | 107 pasan |
| `host939-cluster` (unidad) | 18 pasan, 1 ignorada |
| `multi_node` (incluye el cierre inesperado de un servidor compartido) | 11 pasan, 1 ignorada |
| `seguridad` (equipo rojo, incluye OP coherente y amigos) | 14 pasan, 1 ignorada |
| `host939-p2p` | 2 pasan, 1 ignorada |
| Interfaz (`verificar-ui.js`) | 285 nombres globales sin repetir, sintaxis correcta |
| Servidor real de Minecraft 26.3 (Java 25) | Arranque, `save-all flush`, `stop` y código de salida 0 comprobados para ajustar el guardián; prueba de carga de memoria y GC (sección 1) |

**[estático]**

- `cargo clippy` en todo el workspace: sin errores; avisos de estilo (unos 37) sin efecto funcional.
- `cargo audit`: **ninguna vulnerabilidad conocida**. 7 avisos: 6 crates sin mantenimiento (chrono,
  paste, proc-macro-error, zbus_names, zvariant, zvariant_derive) y 1 de `glib` 0.18 marcado como
  unsound, que solo entra en la compilación de Linux, no en la de Windows.
- Escaneo de secretos del código (claves de CurseForge, claves privadas, tokens de GitHub, AWS,
  Slack): limpio. En la CI se escanea además el ejecutable.

**[Windows CI]** — ver sección 9 para el commit exacto y su resultado.

- Núcleo con servidores reales Vanilla, Fabric, Forge y NeoForge; cliente real de Minecraft que
  entra; dos PCs en máquinas distintas; instalador y portable.
- App instalada, 26 pasos con la interfaz real, entre ellos: cerrar la ventana sin detener el
  servidor (y que exista el guardián), reabrir, **matar 939 Host con el servidor encendido**,
  **aviso y reencendido**, **apagado de Windows simulado con servidor** y **sin servidores**.
- Desinstalación en los dos escenarios, comprobando que conservar datos deja el mundo, las copias y
  los puntos.
- Nota: en las máquinas de la CI la protección en tiempo real de Defender está apagada. La CI
  confirma que no hubo detecciones, pero eso no equivale a una PC de usuario con Defender activo.

## 7. Limitaciones

- Un corte de luz no se puede atajar: protegen los guardados de Minecraft y los puntos (cada
  15 minutos por defecto).
- «Finalizar árbol de procesos» en el Administrador de tareas termina también al guardián; Java se
  cierra sin guardar, como en un corte.
- El apagado se prueba con el aviso que manda Windows, no con un apagado real de la máquina de la
  CI (eso cortaría la prueba).
- Si la persona elige «Apagar de todos modos» antes de que termine el guardado, Windows corta.
- Los chats cifrados no se recuperan si se pierde la clave de DPAPI (Windows reinstalado, cuenta
  borrada); quedan apartados como `*.ilegible-<fecha>`.
- Un `/op` dentro del juego en un servidor compartido se revierte, aunque lo haga la propietaria
  desde el juego: ella lo autoriza desde Jugadores o la consola de 939 Host.
- Jugadores sin 939 Host sin revelar la IP de la casa: pendiente de infraestructura (§16).
- El ejecutable sigue sin firma.
- La reconexión con una PC reiniciada rápido depende de un rodeo para un comportamiento de iroh
  1.4: probado en Linux con la prueba multi-PC; en Windows y en redes reales, pendiente.

## 8. Pruebas manuales necesarias

1. **Apagado real de Windows** con un servidor encendido: tiene que aparecer «939 Host — Guardando
   el mundo de Minecraft…» y, al terminar, seguir el apagado solo. Al encender, el aviso.
2. **Cierre de sesión** con un servidor encendido (igual que el punto 1).
3. **Administrador de tareas → Finalizar tarea** sobre 939 Host (no el árbol) con un servidor
   encendido: el mundo tiene que quedar guardado y, al reabrir, el aviso.
4. Instalar en una PC con **Defender, Smart App Control y UAC** activos; anotar avisos.
5. **Dos casas reales**: traspasar la anfitriona y comprobar el mundo; OP de una coanfitriona
   desde la consola (se revierte) y desde Jugadores de la propietaria (queda).
6. **Chats**: mandar mensajes, cerrar, reabrir, ver que siguen; abrir `mensajes\*.bin` con el Bloc
   de notas y comprobar que no se lee el texto.
7. **Desinstalar** desde Configuración de Windows, eligiendo conservar y después borrar.
8. **Memoria de Java**: con el perfil equilibrado, ver en el Administrador de tareas que Java
   arranca con poca memoria y crece con jugadores.
9. **Reinicio de la anfitriona**: con un servidor compartido, cerrar 939 Host en la PC anfitriona y
   volver a abrirlo enseguida; en la otra PC, el estado tiene que actualizarse y poder entrar.

## 9. HEAD final y estado de la CI

### Corridas de «Pruebas en Windows» revisadas el 19/11

| Corrida | Commit | Resultado | Qué pasó |
|---|---|---|---|
| 44480263331 (noche) | `b8d4c67` | 5 de 6 en verde | **Fallo real**: el apagado simulado lo terminó el guardián, no el cierre ordenado («Salir» y el apagado mataban el proceso a mitad del guardado). Corregido en `5ddccfe`. Los otros cinco trabajos pasaron, incluido el guardián |
| 44591486909 | `5ddccfe` | 5 de 6 en verde | App instalada en verde con la prueba de apagado reforzada. Falló `remote_files_follow_host_and_protect_world` en el núcleo: **carrera de la prueba** (pedía un punto mientras el programa guardaba el estado final al detener; el producto respondió bien «operación en curso»). La prueba ahora espera ese punto |
| **44600054138** | **`0438f24`** | **6 de 6 en verde** | Primera corrida con la corrección de reconexión. Comprobado en los registros: el guardián guarda y detiene Java en 1 s sin procesos huérfanos; aviso y reencendido; apagado simulado con motivo visible registrado en Windows, guardado y fin del proceso en 1,5 s; sin servidores no demora el apagado; desinstalación en los dos modos conservando mundo, copias y puntos; núcleo 107/107, grupo 18, multi-PC 11 (incluida la nueva de reconexión tras un cierre inesperado), seguridad 14; Java 1.818 MB con 6 GB de máximo; Defender sin detecciones |

Corridas anteriores de la fase (`2c557f8` y `9eec316`): no llegaron a compilar por el espejo de
Ubuntu de Azure caído (infraestructura de GitHub); se agregó un espejo alternativo en `b8d4c67`.

### Medición que superó el presupuesto propuesto

En la corrida de `0438f24`, la CPU de 939 Host en la bandeja con el servidor encendido fue 0,94 %
en la muestra de 10 s tomada justo después de cerrar la ventana (otras corridas: 0,0 % y 0,47 %).
Supera el presupuesto propuesto de 0,5 %. Puede ser el cierre del WebView2 dentro de la muestra;
**queda para revisar** con una muestra más larga, no se da por resuelto.

### HEAD

El commit que agrega esta sección (corrección de la carrera de la prueba y documentación) no
cambia código del producto. Su corrida de Windows se revisa al publicarlo; el estado final se
informa en la respuesta.
