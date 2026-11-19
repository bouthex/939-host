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
  Sin servidores no demora el apagado. **[Windows CI]** con el aviso simulado (ver sección 6).
  **[manual]** un apagado real.
- **Estado «Guardando» atascado con Minecraft 26.3** (error encontrado por la nueva prueba):
  26.3 escribe «Saving chunks for level…» al arrancar sin mensaje de cierre y la interfaz quedaba
  en «Guardando». Ahora solo cuenta «Saving the game». **[auto]** **[Windows CI]**
- El veredicto del guardián usa el código de salida de Java, porque 26.3 ya no escribe «All
  dimensions are saved» (comprobado con el servidor real de 26.3).
- Lo que ya existía y se revisó: cada punto de restauración, copia y traspaso hace `save-off` →
  `save-all flush` → espera la confirmación de Minecraft → copia → `save-on`; el mundo de la línea
  solo lo escribe la anfitriona; una copia vieja nunca reemplaza un estado más nuevo (fase
  anterior, prueba `tierra_e_inventario_no_retroceden`).

### Seguridad (§6, §7, §8, §9)

- **Chats y lista de amigos cifrados con DPAPI** (`host939_core::sealed`). Migración automática
  de los `.json` en texto plano (se quitan solo después de verificar la copia cifrada). Un archivo
  que no se puede descifrar se aparta, no se borra. **[auto]**; la prueba con DPAPI real corre en
  **[Windows CI]**.
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
  propósito, conserva la reserva completa. **[auto]**; la medición nueva, en **[Windows CI]**.

### Desinstalación (§12)

- `Desinstalar.exe /S /BORRARDATOS` borra los datos solo si se pide así, de forma explícita; en la
  interfaz siguen las dos confirmaciones (por defecto se conservan).
- **[Windows CI]**: se prueban los dos casos. En ambos no quedan programa, procesos, inicio con
  Windows, entrada de Programas instalados, accesos del menú Inicio, caché de la interfaz, bloque
  en `hosts`, tareas programadas, servicios ni reglas de Firewall. Conservando datos, la carpeta de
  datos queda intacta; con `/BORRARDATOS`, no queda nada.

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
| Arranque hasta que la app se puede usar | — | — | 4,5 a 7,2 s según la corrida |
| Ventana abierta, en reposo | 117–118 MB | 0,16 % | 7 procesos (app y WebView2) |
| Minimizada | 119–120 MB | 0,16–0,31 % | |
| **Bandeja con un servidor encendido** | **23,5 MB** | **0,0 %** | La ventana se destruye; el guardián ocupa 1,7 MB |
| Después de recorrer todas las pantallas | 215 MB | 0,62 % | El WebView2 crece con el uso; vuelve a bajar al cerrar la ventana |
| Java con 6 GB de máximo, antes del cambio | — | — | 6,5 GB privados con un mundo vacío |
| Java con 6 GB de máximo, después del cambio | — | — | Ver sección 9 |

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
| `host939-cluster` (unidad) | 17 pasan, 1 ignorada |
| `multi_node` | 10 pasan, 1 ignorada |
| `seguridad` (equipo rojo, incluye OP coherente y amigos) | 14 pasan, 1 ignorada |
| `host939-p2p` | 2 pasan, 1 ignorada |
| Interfaz (`verificar-ui.js`) | 285 nombres globales sin repetir, sintaxis correcta |
| Servidor real de Minecraft 26.3 (Java 25) | Arranque, `save-all flush`, `stop` y código de salida 0 comprobados a mano para ajustar el guardián |

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
- Desinstalación en los dos escenarios.

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

## 9. HEAD final y estado de la CI

Se completa en el commit que agrega este informe (ver el mensaje del commit y la respuesta final).
