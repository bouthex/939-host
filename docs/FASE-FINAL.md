# 939 Host — Informe de la fase final

Rama: `rediseno-v3` (sin merge, sin PR, sin release). Punto de partida comprobado en GitHub:
`cea681d`. Commits de esta fase: `473559a` → último commit de este informe.

Todo lo que aparece como **comprobado** se ejecutó en esta fase: pruebas automáticas en Linux o en
las máquinas Windows de GitHub Actions, con servidores y clientes reales de Minecraft. Lo que no
se pudo ejecutar está en las secciones 9 y 12, junto con el motivo.

---

## 1. Funciones que ya existían (inspeccionadas antes de tocar)

- Creación de servidores Vanilla, Fabric, Forge y NeoForge; consola, jugadores, archivos, papelera.
- Grupo P2P (iroh/QUIC) con permiso de anfitrión firmado, sincronización del mundo, invitaciones,
  roles, recuperación con código.
- Mods con búsqueda en Modrinth/CurseForge, cuarentena por huella y reparador.
- Puntos de restauración y copias de seguridad.
- Direcciones `.939host` en loopback, logos del servidor, bandeja del sistema, instalador NSIS.

## 2. Funciones mejoradas

| Área | Antes | Ahora |
|---|---|---|
| Mods locales (§2) | Había que «Vincular» cada archivo; lista separada en «Archivos» | Una sola lista; los archivos reconocidos se vinculan solos; importar desde la PC nunca copia mods solo para jugadores |
| Actualizaciones de mods (§2) | Automáticas sin opción | Interruptor por servidor; con él apagado se revisan antes de aplicar; nunca bajan de versión |
| Reparador (§7) | Regla por regla: Mod Menu volvía en cada vuelta | Análisis del conjunto: detecta dependencias que solo usa el cliente, las ignora con `fabric_loader_dependencies.json` y aparta lo de cliente; detecta versiones incompatibles y `breaks` |
| Puerto 25565 (§10) | Falso «libre» (la prueba no era igual a la de Java) | Prueba exclusiva igual a la de Java, sin `listen` (no dispara el Firewall); muestra qué programa lo usa; cambia solo a uno libre si no hay acceso por LAN |
| Logos (§4) | La imagen no se veía, el arrastre no funcionaba | Arreglados los tres fallos (CSS `hidden`, CSP que bloqueaba `blob:`, arrastre nativo de Tauri); vista previa fiel; diseños con las skins de bouthex, Archimak y lescano8 |
| Versiones (§9) | Solo la de Minecraft | Minecraft + loader con su versión en tarjetas, encabezado y detalles |
| Cierre (§14/§17) | Cerrar la ventana podía cortar el servidor; salir tardaba | La X o la barra de tareas dejan el servidor en la bandeja; «Salir» pregunta y destruye la ventana al instante mientras guarda |
| Mundo (§15) | Una PC que no alojaba podía ganarle al estado final de la anfitriona (bloque de tierra e inventario volvían atrás) | Solo la anfitriona escribe la línea del mundo; las demás guardan copias locales; el estado al detener se publica y se pide sincronizar al instante |
| Copias (§19) | Sin límite de espacio | Tope configurable, copias protegidas, verificación de integridad |

## 3. Funciones nuevas

- **Indicadores de operaciones (§3):** panel con lo que está en curso (descargas, traspasos,
  copias), con progreso real o indeterminado; nunca inventa porcentajes.
- **Sonido al quedar listo (§5):** sintetizado en código, sin archivo externo; se puede apagar.
- **Eliminar todos los mods (§6):** a la Papelera, con confirmación.
- **Modpacks (§11):** sección en Mods, vista de detalles (autores, versiones, capturas,
  contenido) e incorporación a un servidor existente.
- **Amigos (§12):** invitación por código, solicitudes, presencia («Disponible», «Jugando en…»),
  mensajes, quitar y bloquear. Sin servidor central.
- **Estado compartido y OP (§13):** todas las PCs ven el estado real del servidor de la
  anfitriona y quién juega; la propietaria concede OP de Minecraft a coanfitrionas con una
  decisión firmada.
- **Dirección pública y jugadores sin 939 Host (§8):** la dirección amigable es la principal; se
  puede guardar una dirección propia (playit.gg, dominio con SRV, router) que la app explica y
  valida. No expone la IP doméstica por defecto.
- **Voz (§16):** el UDP de Simple Voice Chat viaja por la conexión cifrada del grupo.
- **Foto de perfil (§18):** recorte cuadrado, la ven las otras PCs del grupo y los amigos.
- **Desinstalador completo (§20):** detecta la app abierta, pregunta dos veces antes de borrar
  datos (por defecto los conserva), limpia el bloque de `hosts` que agregó la app y nada más.
- **Mantener la PC despierta mientras aloja** (opcional) y aviso la primera vez que se cierra la
  ventana con un servidor encendido.

## 4. Errores corregidos

1. Regresión del mundo al cambiar de PC (tierra e inventario). Causa: un punto de una PC que no
   alojaba quedaba más «fresco» que el estado final de la anfitriona.
2. Bucle de Mod Menu en el reparador.
3. Falso negativo de puerto ocupado (IPv6, comodín, uso exclusivo).
4. Tres fallos de carga de imágenes (logo y foto).
5. Detener el servidor podía darse por vencido antes de que Minecraft terminara de guardar
   mundos grandes: ahora la espera se adapta a la actividad hasta 10 minutos.
6. `adopt_share` copiaba campo por campo y perdía campos firmados nuevos.
7. Casillas de la interfaz que no respondían (el despachador cancelaba el evento).
8. Diálogos que perdían lo escrito al cerrarse.
9. Nombre global `OPS` repetido entre scripts: impedía cargar la página del servidor. Se agregó
   `scripts/verificar-ui.js` a la CI para que no vuelva a pasar.
10. Pedidos de anfitrión con el servidor detenido quedaban esperando una respuesta manual.

## 5. Vulnerabilidades encontradas

| # | Hallazgo | Gravedad | Estado |
|---|---|---|---|
| V1 | El canal nuevo de amigos leía tramas de hasta 8 MB de cualquier PC de internet | Media (consumo de memoria) | Corregido: 16 KB y 10 s de espera |
| V2 | El canal de amigos no estaba bajo el límite de pedidos por PC | Media | Corregido: comparte el tope del grupo |
| V3 | Sin tope de canales de voz por PC | Media (agotamiento de sockets) | Corregido: 4 por PC, 64 en total |
| V4 | La prueba de puerto con `listen` en 0.0.0.0 habría abierto el aviso del Firewall | Baja | Evitado en el diseño: solo `bind` |
| V5 | Una dirección pública con IP literal habría expuesto la IP doméstica | Baja | `validate` rechaza IPs literales |
| V6 | El desinstalador podía dejar el bloque de `hosts` de la app | Baja | Lo quita, y solo ese bloque (prueba incluida) |

No se encontró forma de que una PC sin invitación lea el mundo, archivos o direcciones, ni de que
una jugadora o coanfitriona se dé permisos que no tiene. El modelo de amenazas actualizado y los
21 riesgos residuales están en [`SEGURIDAD.md`](SEGURIDAD.md).

## 6. Medidas de seguridad agregadas

- Límites de tamaño, tiempo y frecuencia en amigos; código de invitación de un solo uso con
  vencimiento; «Aceptó» solo vale si esta PC envió la solicitud; las direcciones anunciadas solo
  se aceptan si son de quien las manda.
- Topes de voz por PC y totales; voz solo para integrantes.
- Fotos: solo integrantes, huella SHA-256 comprobada, 100 KB como máximo.
- `image_read`: 20 MB como máximo y tipo comprobado por contenido.
- OP de Minecraft solo por decisión firmada de la propietaria.
- Copias verificadas y protegidas; borrado por tope que nunca toca copias protegidas.
- Desinstalador que pregunta dos veces y no toca nada fuera de lo que creó la app.
- Revisión automática en la CI de que no haya claves de API, claves privadas ni tokens en el
  código.

## 7. Pruebas ejecutadas

**Linux (este entorno), último estado:**

| Conjunto | Resultado |
|---|---|
| `host939-core` (unidad) | 99 pasan |
| `host939-cluster` (unidad) | 17 pasan, 1 ignorada |
| `multi_node` (varias PCs simuladas) | 10 pasan, 1 ignorada |
| `seguridad` (equipo rojo) | 14 pasan, 1 ignorada; dos corridas seguidas |
| `host939-p2p` (túnel) | 2 pasan, 1 ignorada |
| `scripts/verificar-ui.js` | 284 nombres globales sin repetir, sintaxis correcta |
| Instalador NSIS | Compila con `makensis` |

Pruebas nuevas destacadas: `tierra_e_inventario_no_retroceden`, `mod_menu_no_genera_un_bucle`,
`estado_compartido_y_op_concedido_por_la_propietaria`, `voz_por_udp_llega_a_la_anfitriona`,
`amigos_presencia_y_mensajes`, `amigos_resisten_a_una_pc_desconocida`,
`desinstalador_quita_solo_el_bloque`, `e2e_modrinth_modpack_detalle`.

**Windows (GitHub Actions, commit `363f33e`; todos los trabajos en verde):**

- Núcleo, P2P y grupo con servidores reales: Vanilla, Fabric, Forge y NeoForge 26.3 en línea en
  16–18 s y apagado limpio; modpack real de 181 mods analizado.
- Cliente real de Minecraft 26.2 entra a servidores Vanilla y Fabric.
- Dos PCs en máquinas distintas: invitación, unión por P2P en 0,7 s, servidor encendido.
- App instalada con interfaz real (22 pasos): asistente, instalación real, consola, todas las
  pestañas, copia en caliente, temas, archivos, logo y foto desde imagen de la PC, amigos,
  **cerrar la ventana sin detener el servidor**, reabrir y recuperar el estado, detener limpio,
  sin errores de JavaScript.
- Instalador y portable compilados; escaneo de secretos limpio.

Mediciones de la app (Windows de la CI): arranque 8,6 s; en reposo 116 MB de memoria privada y
0,2 % de CPU; después de usar todas las pestañas 155 MB; instalada ocupa 14,5 MB.

## 8. Pruebas aprobadas y fallidas

- **Aprobadas:** todas las listadas en la sección 7 en su última corrida.
- **Fallidas durante la fase y corregidas:**
  - la interfaz de la CI no llegaba a la página del servidor (nombre `OPS` repetido);
  - una prueba de reparación chocaba con el puerto 25565 en paralelo (ahora usa uno libre);
  - `amigos_resisten_a_una_pc_desconocida` fallaba con carga en paralelo (la prueba esperaba una
    sola forma de corte; ahora acepta las dos formas válidas).
- **Fallidas al cierre:** ninguna conocida.
- Las corridas de la CI marcadas «cancelled» son las que reemplazó un push posterior.

## 9. Lo que no se pudo verificar

- **UAC, Defender y Smart App Control en una PC de usuario:** las máquinas de la CI no los
  tienen igual. El ejecutable **no está firmado**; Smart App Control puede bloquearlo.
- **Dos casas reales con routers y NAT distintos:** la CI usa dos máquinas de GitHub (redes de
  centro de datos).
- **Voz con Simple Voice Chat real y dos personas hablando:** se probó el transporte UDP con
  tráfico simulado, no el mod con audio.
- **Jugadores sin 939 Host por playit.gg o un dominio:** se valida y se explica la dirección,
  pero no se conectó un cliente desde internet.
- **Borrado de datos al desinstalar y limpieza de `hosts` con UAC:** el script se probó en Linux
  y el instalador compila; no se ejecutó la desinstalación completa en Windows.
- **Apagar Windows con un servidor encendido.**
- **Búsquedas reales de Modrinth y la API de Mojang desde este entorno** (bloqueadas aquí; sí
  funcionan en la CI de Windows).
- **`cargo clippy` y `cargo audit` completos:** no se corrieron en esta fase.

## 10. Limitaciones técnicas

- Minecraft y los mods aprobados corren con los permisos del usuario de Windows: no se pueden
  aislar sin romper el juego.
- Sin servidor central, los amigos solo ven presencia y reciben mensajes cuando ambas PCs están
  abiertas (los mensajes quedan pendientes y se entregan después).
- La voz por la dirección pública necesita además el puerto UDP de Simple Voice Chat abierto en
  el servicio que use la persona.
- Si el proceso de 939 Host muere de golpe, Java se cierra sin guardar (lo jugado desde el último
  guardado se pierde).
- En un grupo de dos PCs no hay terceros que desempaten una recuperación.

## 11. Infraestructura externa necesaria

- **Ninguna paga ni propia de bouthex.** Relays públicos de iroh (gratuitos) para conectar PCs
  detrás de NAT.
- Descargas: Mojang (piston), Fabric, Forge, NeoForge, Modrinth y CurseForge (esta última solo si la persona
  carga su propia clave en Ajustes; no viene ninguna en el ejecutable).
- Opcional y elegido por cada persona: playit.gg, un dominio con registro SRV o reenvío de puertos
  en el router, solo para jugadores sin 939 Host.
- Opcional: firma de código (SignPath u otro certificado) para que Windows no avise.

## 12. Qué probar a mano

1. Instalar en una PC con Defender y Smart App Control activos; anotar avisos.
2. Dos casas: crear grupo, invitar, jugar, traspasar la anfitriona y comprobar que el mundo no
   retrocede (poner un bloque, cambiar de PC, verlo).
3. Cerrar con la X y desde la barra de tareas con el servidor encendido; volver a abrir.
4. «Salir» desde la bandeja con el servidor encendido: debe guardar y traspasar.
5. Voz: Simple Voice Chat en servidor y clientes, dos personas por la dirección `.939host`.
6. Un jugador sin 939 Host entrando por playit.gg o por el router.
7. Amigos entre dos casas: invitación, presencia, mensajes con la otra PC cerrada y luego abierta.
8. OP concedido por la propietaria a una coanfitriona y revocado.
9. Logos con las skins y foto de perfil arrastrando una imagen.
10. Modpack desde la sección Modpacks y reparador con Mod Menu instalado.
11. Llenar el tope de copias y comprobar que las protegidas quedan.
12. Desinstalar dos veces: una conservando datos y otra borrándolos; revisar `hosts`.

## 13. Estado de compilación y CI

- `Pruebas en Windows` en `363f33e`: **6 de 6 trabajos en verde** (núcleo y servidores reales,
  cliente real, dos PCs, instalador, app instalada).
- `ed8f259` (límites contra abuso + este informe): `Pruebas en Windows` **en verde** y el flujo
  `CI` general (pruebas en Linux y compilación de Windows x64), lanzado a mano sobre esta rama,
  **en verde**.
- Este último commit solo cambia esta sección del informe.

## 14. Commit final en `rediseno-v3`

El commit que agrega este informe y la actualización de `SEGURIDAD.md` es el último de la fase en
`rediseno-v3`. `main` no se tocó; no hubo merge, PR ni release.
