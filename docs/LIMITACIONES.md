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
- **Hole punching necesita el relay público para el primer contacto** (así funciona iroh). Si desactivás los relays públicos, solo queda conexión directa (misma red, IPv6, UPnP) o por otra PC del grupo.
- **El puente por otra PC** solo sirve si esa PC llega directo a la anfitriona.
- **Reconexión automática**: necesita un perfil Fabric o NeoForge (en Vanilla puro no hay mods). Depende de que el mod AutoReconnect (Reforged) tenga versión para tu Minecraft. Sin el mod, el jugador vuelve a entrar a mano con la misma dirección (la lista de servidores muestra «se está mudando» mientras tanto).
- **Modo offline y lista blanca**: si el servidor está en modo offline, banear u operar a alguien que nunca entró puede quedar registrado con el nombre en minúsculas (comportamiento de Minecraft). La lista blanca no tiene ese problema.
- **Zombie Horde 1.3.1** no arranca en un servidor dedicado con un mundo nuevo (error del mod). Con un mundo donde ya se jugó con el mod, sí.
- **Forge y NeoForge** están implementados con el instalador oficial pero no se pudieron probar en vivo.
- **Cambiar la versión de Minecraft** de un servidor existente no está disponible: se crea uno nuevo importando una copia del mundo.
- El ejecutable **no está firmado**. SmartScreen avisa y **Smart App Control (Windows 11) lo bloquea** sin opción de continuar. Se solicitó la firma gratuita de SignPath Foundation (ver `docs/firma/SIGNPATH.md`).
- No hay **actualizaciones automáticas** de 939 Host: se instala la versión nueva encima (el instalador conserva los datos).
- El mundo completo se copia a cada coanfitriona la primera vez (por ejemplo, 2,5 GB en nuestro modpack de prueba); después solo viajan los cambios.

## Pendiente para una próxima versión

- Firma de código del ejecutable.
- Actualizador integrado.
- Cambio de versión de Minecraft con punto de restauración previo.
- Descubrimiento de PCs del grupo en la red local (mDNS) para no depender de direcciones guardadas cuando cambia la IP y no hay relays.
- Compresión de los bloques de sincronización que no sean regiones (las regiones ya vienen comprimidas).
- Panel de historial del grupo (quién hosteó, traspasos, recuperaciones).
