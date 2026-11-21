# Limitaciones y tareas pendientes — 939 Host 0.9.0

## Lo que hay que probar entre ustedes antes de confiar del todo

1. **Abrir la app en Windows** y crear un servidor de prueba (no el mundo principal).
2. **Conexión entre casas**: compartir el servidor de prueba, invitar a un amigo como coanfitrión y jugar. En «Grupo y anfitriona» se ve si la conexión es «directa» o «por relay».
3. **Traspaso**: «Pasar a…» con alguien conectado. Esperado: unos segundos de desconexión y volver a entrar con la misma dirección.
4. **Caída**: con tres PCs, cerrar la anfitriona a la fuerza (Administrador de tareas). Esperado: otra la toma en menos de un minuto.
5. Recién después, migrar el mundo principal siguiendo el manual (§6), con Minecraft cerrado.

## Limitaciones conocidas

- **Pérdida máxima en una caída**: lo jugado desde el último punto sincronizado (15 min por defecto). En traspasos y al soltar no se pierde nada. Bajar el intervalo de sincronización reduce la pérdida a costa de más pausas de guardado (decenas de ms).
- **Dos PCs**: sin mayoría no hay recuperación automática; se usa «Recuperar aquí» a mano. Con tres o más PCs conectadas es automática.
- **Partición de red con PCs que se conectaron durante el corte**: si la anfitriona pierde contacto con el grupo pero otras PCs que ella nunca vio forman mayoría, por un rato puede haber dos copias encendidas en PCs distintas. Nunca escriben el mismo archivo y, al reconectarse, la de época vieja se detiene y aparta su copia (no se mezcla), pero lo jugado en ella queda solo como punto de seguridad.
- **Hole punching necesita el relay público para el primer contacto** (así funciona iroh). Si desactivás los relays públicos, solo queda conexión directa (misma red, IPv6 o un puerto que ya esté abierto) o por otra PC del grupo.
- **El puente por otra PC** solo sirve si esa PC llega directo a la anfitriona.
- **Reconexión automática**: necesita un perfil Fabric o NeoForge (en Vanilla puro no hay mods). Depende de que el mod AutoReconnect (Reforged) tenga versión para tu Minecraft. Sin el mod, el jugador vuelve a entrar a mano con la misma dirección (la lista de servidores muestra «se está mudando» mientras tanto).
- **Modo offline y lista blanca**: si el servidor está en modo offline, banear u operar a alguien que nunca entró puede quedar registrado con el nombre en minúsculas (comportamiento de Minecraft). La lista blanca no tiene ese problema.
- **Zombie Horde 1.3.1** no arranca en un servidor dedicado con un mundo nuevo (error del mod). Con un mundo donde ya se jugó con el mod, sí.
- **Forge y NeoForge**: en 0.9.0 no se habían probado en vivo. En la rama en desarrollo ya arrancan en las pruebas reales de Windows.
- **Cambiar la versión de Minecraft** no está en 0.9.0 (en desarrollo, ver abajo). Volver a una versión anterior con el mismo mundo nunca se hace sobre el servidor original: Minecraft no convierte mundos hacia atrás.
- El ejecutable **no está firmado**. SmartScreen avisa y **Smart App Control (Windows 11) lo bloquea** sin opción de continuar.
- **Actualizaciones automáticas** desde la 1.1.0. Las versiones anteriores (0.9.0 y la de prueba 1.0.0) no consultan: hay que instalar la 1.1.0 una vez a mano, encima (se conservan los datos). La primera release oficial firmada con la clave real todavía no se publicó.
- El mundo completo se copia a cada coanfitriona la primera vez (por ejemplo, 2,5 GB en nuestro modpack de prueba); después solo viajan los cambios.

## En desarrollo (rama `rediseno-v3`, todavía sin versión publicada)

Probado en Windows reales: cambio de versión con copia verificada y vuelta atrás, Papelera de 30 días,
mods de rendimiento elegidos por versión y loader, pregeneración con Chunky, perfil `.mrpack` para
los jugadores, servidores desde modpacks `.mrpack`, administrador de archivos local y remoto (también
entre dos PCs en lugares distintos), logo por servidor (también en Servidores de amigos), direcciones
internas `.939host` y cambios del grupo firmados.

Límites conocidos de esas funciones:

- Las direcciones `.939host` son **internas**: cada PC las crea en su 939 Host (pide permiso de
  administrador una vez) y no sirven para quien no tenga la app. No son direcciones públicas.
- **CurseForge** está implementado con su API oficial pero **no se pudo probar en vivo**: hace falta una
  clave aprobada por CurseForge (ver el Manual). Sin clave, la app lo explica y todo lo demás funciona.
  Los modpacks de CurseForge no indican qué mods son solo para jugadores: se instalan todos.
- En el administrador de archivos remoto, exportar carpetas grandes baja archivo por archivo (no
  comprime antes).
- La app no detecta si tu Minecraft ya tiene la versión y los mods: el perfil `.mrpack` se importa en
  un launcher compatible (Modrinth App, Prism Launcher, ATLauncher).
- Con la ventana minimizada la memoria baja poco (de unos 344 a 313 MB en RAM en las mediciones);
  la mayor parte la usa WebView2, el motor de la ventana.

## Pendiente para una próxima versión

- Firma de código del ejecutable.
- Activar el actualizador integrado (ya implementado): falta que el propietario cree su identidad de publicación y elija dónde distribuir.
- Conexión por IP: verificar con routers reales (UPnP/NAT-PMP/PCP), desde otra red y con CGNAT real (ver DIRECCIONES-PUBLICAS.md). Una dirección que no cambie con la IP pública necesita un nombre propio (DNS dinámico o dominio).
- Descubrimiento de PCs del grupo en la red local (mDNS) para no depender de direcciones guardadas cuando cambia la IP y no hay relays.
- Compresión de los bloques de sincronización que no sean regiones (las regiones ya vienen comprimidas).
- Panel de historial del grupo (quién hosteó, traspasos, recuperaciones).
