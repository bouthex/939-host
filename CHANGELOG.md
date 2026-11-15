# Registro de cambios

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
