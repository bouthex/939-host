# Registro de cambios

## 1.3.0

Primera versión oficial desde la 0.9.0. Reúne todo lo de las versiones de prueba 1.1.0, 1.2.0 y 1.3.0.

**Servidor compartido entre PCs**
- «Iniciar» y «Detener» desde cualquier PC del grupo actúan en la PC que aloja; el estado, los jugadores, el tiempo por tick, los TPS y la memoria se ven en vivo en todas.
- Traspasos manuales y al salir sin perder el mundo; con dos PCs, si la anfitriona se cae, la otra lo levanta sola (con internet) y la que vuelve aparta su copia sin perderla.
- Pedidos para alojar que siempre esperan la respuesta de quien aloja (nunca se aceptan solos), con estados reales y aviso de sincronización que siempre se cierra.
- Traspaso de propiedad en dos pasos: la otra PC acepta y recién ahí cambia. Firmado, guardado y repetible: si un mensaje se pierde o una PC se cierra en el medio, termina igual y nunca quedan dos propietarias ni ninguna.
- Nombres reales de cada PC en el grupo, también en las PCs que solo juegan.

**Jugadores**
- Detección con la lista del propio servidor (UUID real), además del registro: ya no queda en 0 jugadores con alguien conectado (26.3 cambió el formato de los avisos) y una salida no vista se corrige sola.
- Ficha en vivo: UUID, tiempo conectado, vida, hambre, saturación, experiencia, inventario con objetos de mods y encantamientos. Cambios de a un paso que se hacen en el servidor real y se comprueban.
- Inventario visual: arrastrar y soltar, dar, quitar y cambiar cantidades, desde 1.13 hasta 26.3. La propietaria administra aunque aloje otra PC.

**Conexión**
- Dirección `nombre.939host` para todo el grupo (también lleva Simple Voice Chat); se libera al borrar el servidor.
- Jugadores sin 939 Host por IP (lo decide la propietaria; solo juegan): entradas desde Internet comprobadas desde otra PC, aviso de CGNAT y redirección manual.
- Resumen con las direcciones separadas por quién las usa (integrantes, jugadores sin 939 Host, red local, esta PC), la IP real de la red local y «Copiar todas las direcciones» solo con las que funcionan.
- Servidores encendidos fuera de 939 Host: se detecta uno propio encendido a mano (y no se abre una segunda copia sobre el mismo mundo) y se pueden vincular otros por dirección y puerto, con su estado real.

**Versiones, mods y perfiles**
- Cambiar la versión de Minecraft o el loader (Vanilla, Fabric, Forge, NeoForge) con revisión de cada mod, copia verificada y vuelta atrás.
- Fabric en versiones viejas (Java 8) arranca; mods de rendimiento para cada versión exacta.
- «Preparar para jugar» copia tus preferencias (teclas, idioma, accesibilidad, gráficos, sonido) desde tu perfil de siempre y conserva la lista de servidores; nada original se modifica.
- Panel de Resource packs (Modrinth o `.zip`, obligatorio u opcional).

**Además**
- Actualizaciones automáticas desde las releases oficiales, con firma, SHA-256, vuelta atrás real y la portable incluida.
- Chat con amigos y grupos de chat: sumar amigos a un grupo, copiar mensajes, direcciones, comandos e ids con un toque (nada se ejecuta solo).
- Protección del spawn apagada al crear un servidor. Liberar la caché de descargas desde Configuración.

## Historial de desarrollo (versiones de prueba, sin release)

### 1.3.0 — etapas del 22 al 25/11
- Servidor compartido: «Iniciar» desde cualquier integrante lo enciende en la PC que lo aloja (o, con el permiso libre, en la mejor PC conectada con la última versión); pedir ser anfitriona es otra acción. «Detener» desde otra PC (propietaria y coanfitrionas). Dos pedidos a la vez: gana uno y el otro ve quién lo está encendiendo.
- Traspaso manual y al salir: arreglado «no se pudo copiar el mundo: el punto … no se puede verificar (época o PC no autorizadas)». Causa: la PC que recibía todavía tenía el permiso viejo; ahora lo actualiza desde la PC de origen antes de verificar.
- Estado en vivo para todo el grupo: la anfitriona avisa cada cambio y las demás PCs lo muestran (lista de servidores y pantalla del servidor) y suenan al quedar en línea. Una comprobación de conexión fallida ya no deja «Preparando» para siempre; una instalación interrumpida ofrece reintentar.
- Estadísticas reales de la anfitriona para todos: jugadores, tiempo por tick y TPS, memoria y CPU de Java, tiempo encendido y latencia.
- Dirección `nombre.939host` para todo el grupo. Causa de que no funcionara en otra PC: quedaba solo en la PC que la eligió. También lleva el chat de voz (Simple Voice Chat).
- Fabric en Minecraft 1.16 o anterior (Java 8) no arrancaba: Java 8 no entiende «@archivo». Reproducido y arreglado con Java 8 real y Fabric 1.14. La reparación reconoce ese error y, si no reconoce otro, muestra las líneas reales del registro en lugar de «no hay problemas».
- Mods de rendimiento: además del catálogo, los de la categoría «optimización» de Modrinth que corren en el servidor para la versión exacta.
- Jugadores: inventario en vivo (dar, quitar, mover, cambiar cantidad) con comprobación; la propietaria administra desde su PC aunque lo aloje otra (la anfitriona comprueba el permiso).
- Recomendación de anfitriona con la red medida entre las PCs (latencia, variación, pérdida, directa o relay), equipo y subida.
- Amigos: grupos de chat (crear, cambiar nombre, salir) e indicador «está escribiendo».
- Hambre y saturación de a un paso con el jugador conectado (antes solo se podía llenar): efectos de Minecraft medidos en un bucle cerrado hasta el valor exacto. Vida con el jugador conectado desde 1.13 (antes 1.19.4), con veneno medido en versiones sin `/damage`. Probado con el cliente real de Minecraft 26.2 y 1.20.4.
- Inventario en vivo también en 1.13 a 1.20.4 (mover y cambiar cantidad conservando encantamientos, nombre y desgaste).
- TPS reales en todas las versiones (en las que no tienen `tick query`, con `debug start/stop`, sin dejar archivos).
- Grupo de dos PCs: si la anfitriona se cae, la otra lo levanta sola cuando tiene internet; la que vuelve aparta su copia sin perderla.
- Chat de voz también al entrar por 127.0.0.1 (no solo por la dirección .939host).
- Mods de rendimiento para versiones viejas de Fabric: se buscan en toda la serie (por ejemplo 1.14.x) y se comprueba en el propio mod que acepte la versión exacta.
- La dirección del grupo se ofrece para activar sola al llegar.
- La protección contra Log4Shell se baja del servidor actual de Mojang (piston-data).

### 1.2.0 (rama rediseno-v3, sin release)
- «Preparar para jugar» conserva los datos de Minecraft: el perfil nuevo recibe una copia de las preferencias del último perfil usado (idioma, teclas, accesibilidad, gráficos, sonido) y de la lista de servidores; nada original se modifica. Causa del problema anterior: el perfil usaba una carpeta propia vacía, por eso arrancaba con opciones por defecto y sin los servidores (los mundos y la lista seguían en el perfil de siempre).
- La lista de servidores conserva íconos y la elección de resource packs de los servidores que ya estaban (antes se perdían al agregar el de 939 Host). Copias previas de `launcher_profiles.json` (la original y la anterior) y de `servers.dat`.
- Dos servidores con el mismo nombre ya no comparten carpeta; se respeta la carpeta que tenga el perfil en el Launcher. Deshacer el cambio de mods. Usar las preferencias en un perfil que ya existía (las anteriores quedan guardadas).
- Carpeta de Minecraft detectada (Launcher y Microsoft Store) o elegida a mano si está en otro lugar.
- Panel de Resource packs: Modrinth, archivos propios, obligatorio u opcional con efecto real (server.properties y entrega local por el grupo, o instalación con «Preparar para jugar»).
- Archivos: las carpetas que faltan (resourcepacks, mods, config, datapacks) se crean desde ahí mismo.

### 1.1.0 (rama rediseno-v3, sin release)
- Actualizaciones automáticas desde las releases oficiales de GitHub: detección al abrir y cada 6 horas, descarga reanudable con progreso real, verificación de firma y SHA-256, instalación sola al abrir (o «Actualizar ahora» con la app en uso), reinicio y confirmación de la versión nueva.
- Vuelta atrás real: respaldo verificado de los archivos del programa y un vigía que restaura la versión anterior si la nueva no instala o no abre. Los datos nunca se tocan.
- Portable: se actualiza en su propia carpeta (o explica si la carpeta no se puede escribir).
- Pantalla de actualización con los cubos de la presentación y los estados reales; respeta «menos movimiento».
- Publicación con aprobación del propietario: el workflow `Release` exige las pruebas de Windows en verde y firma en el entorno protegido `publicacion`.

### Rediseño v3 — fase final (rama rediseno-v3, sin release)
- Administrador de archivos local y remoto (protocolo P2P propio en la PC anfitriona): rutas confinadas, mundo protegido con el servidor encendido, sin sobrescribir sin pedirlo, Papelera, transferencias en partes con cancelación. Probado entre dos PCs Windows reales.
- CurseForge por su API oficial (requiere clave; respeta el permiso de distribución de cada autor). Sin clave no se pudo probar en vivo.
- Servidores desde modpacks: `.mrpack` (solo archivos de servidor) y CurseForge.
- Logo del servidor en Servidores de amigos e invitaciones, firmado y estable entre anfitrionas.
- Mods de rendimiento guiados (Lithium, FerriteCore, ModernFix, Krypton…), presets y pregeneración con Chunky.
- Diseño: pasada completa, microinteracciones, controles monocromáticos, arreglos vistos en Windows.
- WebView2 más liviano. Medido en Windows (mismas condiciones, antes → después): reposo 125/344 → 116/330 MB (propia/en RAM), minimizada 126/313 → 117/306 MB, tras usarla 205/416 → 179/388 MB; arranque 3,6–4,6 s.
- Portada de GitHub con recursos propios y capturas reales.

### Rediseño v3 — etapa «evolución integral» (rama rediseno-v3, sin release)
- Papelera real: 30 días desde el borrado, restaurar completo (con conflicto → «(restaurado)» con identidad propia), borrar definitivo, vaciar.
- Operaciones visibles: franja de estado con explicación, giro y barra; píldora de estado más presente.
- Cambiar versión desde Ajustes: análisis concreto, actualizar con copia verificada + arranque de prueba + volver atrás, copia con la versión nueva, o empezar de cero. Volver a una versión anterior con mundo, bloqueado sobre el mismo servidor.
- Logo por servidor (diseños de 939 o imagen propia con recorte); también como server-icon.png de Minecraft.
- Direcciones `nombre.939host` internas (archivo hosts + IP de loopback propia; permiso de Windows). No son públicas.
- Perfil para jugar `.mrpack` verificado por hash.
- Integrantes firmados por la propietaria; transferir propiedad; salir del grupo.
- Mediciones reales en Windows (CI): arranque 4,6 s; reposo 122 MB privados / 339 MB en RAM (7 procesos con WebView2); minimizada sin reducción; instalado 12,7 MB.

## Sin publicar — Diseño v3 (rama `rediseno-v3`)

### Interfaz
- Interfaz nueva, monocromática y minimalista, con **tema claro y oscuro** diseñados por separado
  (o según Windows); se recuerda en Configuración.
- Logo **Cinta** en la app, el ícono, la bandeja, la pantalla de carga, el instalador y «Acerca de».
- Navegación simplificada: Servidores, Servidores de amigos y Configuración. Primera ejecución sin
  servidores precargados: Crear servidor, Unirme con un código o Evaluar esta PC.
- Asistente de creación en cuatro pasos con la memoria recomendada para esta PC.

### Servidores
- **Ocho estados** claros (Detenido, Iniciando con su etapa, Funcionando, Guardando, Pausado,
  Reiniciando, Deteniéndose, Error) y controles siempre visibles.
- **Pausar** con `/tick freeze` (Minecraft 1.20.3 o posterior), sin desconectar a nadie.
- Detener o reiniciar con jugadores conectados: **aviso con cuenta regresiva en el chat** (cancelable).
- Consola con filtros, autocompletado con ayuda de argumentos, historial y acciones rápidas; acepta
  comandos con o sin `/`. Las consultas internas de 939 Host no aparecen en la consola.
- Pestaña **Mundo** (tamaño, dimensiones, semilla, abrir carpeta, reemplazar, mundo nuevo).

### Jugadores
- Perfil con datos reales del servidor (sin mods complementarios): vida, hambre, armadura,
  experiencia, coordenadas, dimensión, orientación, punto de aparición, efectos, inventario con
  armadura y mano secundaria, cofre de ender, estadísticas y permisos. Conectado: se actualiza cada
  4 s solo con el perfil abierto; desconectado: último guardado de Minecraft.
- Skins reales e **imágenes originales del juego** descargadas de Mojang a tu PC (desactivables).

### Asistente de reparación
- Diagnóstico local en palabras simples (mods para otra versión o loader, dependencias faltantes,
  mods solo de cliente, duplicados, archivos dañados, Java, puerto, memoria, EULA, mundo dañado).
- «Solucionar automáticamente» con punto de restauración previo, autorización para cambios
  importantes, verificación real del arranque y **Deshacer**.

### Grupo
- **Nombre visible para cada PC**, aleatorio la primera vez (Mate, Pampa, Cuadradín…) y editable,
  con un símbolo. Es solo visual: no cambia la identidad, las claves ni los permisos.
- **Solicitar ser anfitrión**: cualquier PC coanfitriona puede pedirlo; a la anfitriona le llega un
  aviso con un resumen del equipo y sus limitaciones, y decide aceptar o rechazar. Una solicitud por
  vez, se puede cancelar y vence sola. Si la anfitriona no responde, actúa la recuperación de siempre.
- Traspaso más seguro: la otra PC copia el mundo **antes** de recibir el permiso (primero con el
  servidor encendido, después solo lo que cambió). Si la copia falla, el servidor sigue donde estaba.
- Protecciones extra en el traspaso: no se pueden iniciar dos a la vez; durante un traspaso largo la
  anfitriona sigue controlando que no la hayan reemplazado; si la confirmación no tiene respuesta, la
  PC vieja no vuelve a encender (evita dos mundos activos); la PC nueva instala lo necesario antes de
  aceptar. Solo la propietaria puede quitar PCs del grupo.
- La lista de servidores y la consola muestran «En otra PC» y quién lo aloja.

### Windows (probado en máquinas Windows reales)
- Mover un servidor o un mundo a la Papelera ya no falla cuando Windows (antivirus, indexador)
  retiene un archivo por un instante.
- Fichas de jugadores sin conexión, estadísticas y logros con el formato de mundo de 26.x
  (`players/data`); antes solo se leía el formato anterior.
- Instalación y desinstalación silenciosas sin diálogos que las frenen.
- Barra lateral sin cortes en la ventana mínima; memoria del servidor explicada («en uso ·
  asignados al mundo»); el último paso del asistente explica qué pasa al crear.

### Diseño
- Íconos propios «Bloque» y lenguaje de movimiento: indicadores que se deslizan, carga en escalón,
  progreso circular y transiciones cortas. Respeta «reducir animaciones» de Windows.

### Mods
- Tienda con Instalados, Explorar Modrinth (categorías, dónde se instala, orden), Actualizaciones y
  Archivos agregados a mano (con **Vincular** a Modrinth); página de detalle; agregar .jar
  arrastrando o eligiendo archivos.

## 0.9.0 — 14/11/2026

Primera versión pública.

### Publicación
- Código abierto bajo licencia **MIT**, con licencias de terceros incluidas en cada descarga.
- Compilación, pruebas y publicación automáticas en GitHub Actions; instalador y portable con
  SHA-256 y atestación de procedencia.
- Integración preparada para la firma gratuita de SignPath Foundation (pendiente de aprobación;
  esta versión **no está firmada**).
- Política de privacidad, de seguridad y de firma de código; guía de instalación, compilación y
  solución de problemas.
- El instalador muestra el resumen de privacidad e instala `PRIVACIDAD.md`.

### Servidor compartido entre PCs
- Nuevo módulo `crates/cluster`: un nodo P2P por PC mantiene cada servidor compartido.
- **Permiso de anfitrión con épocas** (`lease`): una sola PC puede encender; la época solo crece; quien ve una época mayor se detiene.
- **Sincronización verificada**: solo viajan los bloques de 64 KiB que faltan, cada uno comprobado por SHA-256; un punto a medio copiar nunca se registra.
- **Traspaso planificado** («Pasar a…»), **Guardar y soltar** (exige que la próxima PC tenga el último punto) y **traspaso automático al salir**.
- **Recuperación automática por mayoría** (elige el estado más reciente y el mejor hardware) y **aislamiento** de la anfitriona que pierde contacto o vuelve con época vieja; su copia queda como punto de seguridad.
- **Recuperar aquí** para grupos de dos PCs.
- **Dirección fija para jugar** (`127.0.0.1:<puerto>`) que sigue a la anfitriona; puente por otra PC si no hay camino directo; respuesta «se está mudando» / «nadie lo está hosteando» en la lista de servidores.
- Invitaciones de un solo uso con rol (coanfitriona o jugador); la coanfitriona recibe la configuración y se instala sola.
- Los puntos de restauración ahora incluyen `mods/` y `config/`.
- `server.properties` se regenera desde la configuración en cada inicio (el puerto de cada PC se respeta tras un traspaso).

### Servidores
- **Forge y NeoForge**: elección de versión, instalador oficial, arranque por archivo de argumentos.
- **Fabric plan B**: si el servicio de Fabric no responde, se instala desde su repositorio Maven con verificación SHA-1.
- **Jugadores y permisos**: lista blanca, operadores, baneos, expulsar (pestaña Jugadores).
- **Mundo nuevo** seguro (deja punto de restauración) e **informe de diagnóstico** (.zip sin mundo ni claves).
- **Importar mods de una carpeta**: omite los solo de cliente salvo que otro mod los necesite; elige el más nuevo entre duplicados; no toca el origen.
- Lectura de metadatos de los jar sin internet (lado cliente/servidor, duplicados por id).

### Jugadores
- **Reconexión automática**: instala AutoReconnect (Reforged) en el perfil de Minecraft elegido, solo agregando archivos; lo repone si se borra.

### Otros
- Instalador NSIS por usuario (sin administrador), accesos directos, desinstalador que conserva los datos.
- Ajustes honestos: se quitaron opciones que no tenían efecto; «puede hacer de puente» ahora funciona.
- Pruebas multi-proceso del protocolo de grupo y de traspaso con Minecraft real.

## 0.2.0
- Mods de Modrinth con dependencias, puntos de restauración incrementales, compartir por P2P con invitaciones, inicio con Windows.

## 0.1.0
- Primera versión: servidores Vanilla y Fabric, consola, ajustes, copias, evaluación de hardware.
