# Continuidad del trabajo — 939 Host 1.3.0

Rama: `rediseno-v3` · `main` sin tocar (77a6ad1) · releases: solo v0.9.0 · versión del código: 1.3.0.
Respaldo previo a esta etapa: `939host-respaldo-antes-etapa-final.bundle` (en la carpeta de entregas) y
la etiqueta local `respaldo-antes-etapa-final`.

## Objetivo
Entregar una versión **terminada**: problemas resueltos de raíz, no limitaciones enumeradas, y al final un
instalador y un portable de Windows con sus SHA-256 y un informe breve con las verificaciones de cada
resultado (pedido del 23/11, «condición fundamental»).

## Corregido y verificado

| Problema | Solución | Verificación |
|---|---|---|
| Traspaso: «época o PC no autorizadas» | `pull` refresca el permiso desde la PC de origen | multi_node (procesos P2P, misma máquina) |
| Iniciar exigía pedir ser anfitriona | `Req::StartRemote` / `Req::StopRemote` (cualquier integrante; lo enciende la PC que lo aloja) | multi_node `tierra_e_inventario…` |
| Estado no se actualizaba en las demás PCs | La anfitriona empuja `SyncNow` en cada cambio; `seen_key` + `remote_event` (sonido y aviso) | multi_node + UI |
| «Preparando» eterno | La comprobación de conexión fallida pasa a «listo» con aviso; la instalación interrumpida ofrece reintentar | código |
| `939z.939host` no funcionaba en otras PCs | La dirección viaja al grupo (tarjeta firmada o `Info.alias`); escucha automática cada 30 s; aviso para activarla | código (falta probarlo entre dos PCs) |
| Fabric 1.14 no arrancaba | Java 8 no entiende `@archivo`: los argumentos se pasan sueltos | Java 8 real + Fabric 1.14 → «Done» (esta máquina) |
| La reparación decía «sin problemas» | Regla del error de clase principal + líneas reales del registro si no reconoce el error | test unitario |
| Mods de rendimiento «ninguno» en 1.14 | Catálogo + categoría «optimization» de Modrinth en toda la serie (1.14.x); se lee `fabric.mod.json` del mod para confirmar la versión exacta (`mods::best_compatible`, `jar_accepts`) | **pendiente**: CI `e2e_optimizacion_fabric_vieja` (Modrinth no es accesible desde esta máquina) |
| Hambre/saturación en vivo | `set_food_live`: efectos «saturation» (instantáneo) y «hunger» en bucle cerrado, leyendo los valores reales | **cliente real 26.2 y 1.20.4: pasó** |
| Vida en vivo antes de 1.19.4 | Veneno medido en bucle (no baja de 1 punto). `/attribute` NO sirve (probado: no recorta la vida) | veneno comprobado en un servidor 1.16.5 real con una entidad |
| Inventario en vivo | Dar, quitar, mover y cantidad; en 1.13–1.20.4 se usa el objeto completo con sus datos; 1.20.5+ `item modify` (la clave es `function`; en 26.3 pasa a `type`, se detecta) | **cliente real 26.2 y 1.20.4: pasó** (incluye una espada encantada al moverla) |
| TPS en versiones viejas | `debug start/stop` y se borran los perfiles (`clean_debug_profiles`) | servidores reales 1.16.5 y 1.18.2; cliente real 1.20.4 y 26.2 |
| Estadísticas remotas | `RunStats` (jugadores, ms por tick, TPS, memoria, CPU, tiempo encendido, latencia) | código + UI |
| Jugadores administrados desde otra PC | `Req::PlayerOp` (ver: cualquier integrante; cambiar: solo la propietaria) | multi_node |
| Grupo de dos PCs sin recuperación automática | `failover_check`: con 2 PCs, `Takeover` automático tras el doble del tiempo si esta PC tiene internet (relay conectado) | multi_node `two_pcs_automatic_recovery` y `two_pcs_manual_recovery…` (sin internet no toma) |
| Voz solo por la dirección .939host | Escucha de voz también en la ruta 127.0.0.1; los sockets de voz se cierran con su escucha (`Abort`) | código (el túnel de voz ya tenía test) |
| Recomendación de anfitriona | Red medida (latencia, variación, pérdida, directa o relay), equipo y subida | test unitario |
| Chat de grupos y «escribiendo» | `friends.rs` (grupos, direcciones de integrantes, «escribiendo» que vence a los 6 s) | test con 3 procesos (`grupos_de_chat_y_escribiendo`) |
| Detener solo para coanfitrionas | Cualquier integrante enciende y detiene; botones también en la lista de servidores | código (commit 1326bf3) |
| Log4Shell no se bajaba en algunos entornos | URL `piston-data.mojang.com` (el mismo archivo y hash) | descarga real |

## Pruebas que NO hace falta repetir
- Suites locales: `cargo test -p host939-core --lib` (137), p2p + cluster (`multi_node` 14, `seguridad` 15). Todas pasaron después de los últimos cambios de Rust (excepto el commit 1326bf3, que solo cambia un permiso y la UI).
- `e2e_jugador_en_vivo` con cliente real 26.2 y 1.20.4 (local), y sintaxis de comandos en servidores reales 1.16.5–26.3.

## Cómo probar localmente con el cliente real (si hiciera falta)
Script `/tmp/vivo.sh "<versiones>" <sufijo>` (se pierde al reiniciar la máquina). Usa Java 8/17/21/25 de
`/usr/lib/jvm`, `CLIENTE_REAL=ci/cliente-real/cliente.py`, `xvfb-run` y la caché de cliente en
`/home/claude/cliente-cache`. En esta máquina los clientes viejos (1.16.5, 1.18.2) se cierran por el
OpenGL por software: es una limitación del entorno de prueba, no de 939 Host.

## Estado al 24/11 (después de retomar)
- CI 45327794606 (commit 6c4b562) en verde: Fabric 1.14 con mods de rendimiento reales de Modrinth
  arrancó (5 mods); el jugador en vivo con el cliente real pasó en 26.2 y 1.20.4. Faltaba terminar
  «App instalada en Windows (interfaz real)», que ya pasaba antes.
- Entrega: instalador y portable reconstruidos desde el registro (`scripts/extraer-entrega.py`), con el
  SHA-256 verificado. Informe en `docs/INFORME-1.3.0.md`. Guía actualizada con la actualización desde la 1.2.0.
- Nuevo: voz por la entrada pública (UDP + UPnP). La comprobación de la entrada reintenta a los 30 s.

## Pendiente (en orden, histórico)
1. **GitHub Actions** del último commit: revisar `Pruebas en Windows`, sobre todo:
   - «Mods de rendimiento en Fabric 1.14» (`e2e_optimizacion_fabric_vieja`): si falla, ajustar `optimize::candidates` y `mods::best_compatible` según lo que liste el registro;
   - «Jugador conectado con el cliente real (26.2 y 1.20.4)»;
   - el resto, que ya pasaba.
2. Si todo está en verde: bajar el instalador y el portable desde el job **«Versión de prueba en el registro»**. Imprime en base64 entre `@@INICIO <archivo>` y `@@FIN <archivo>`, porque desde la sesión no se puede bajar el artefacto. Con `gh api repos/bouthex/939-host/actions/jobs/<id>/logs`, reconstruir los archivos, verificar el SHA-256 contra `SHA256SUMS.txt` y entregarlos.
3. Informe final breve: qué se corrigió, cómo, y la verificación de cada punto; separar «probado con el cliente real o con servidores reales» de «verificado con pruebas automáticas».
4. Prueba real entre dos PCs, con la guía `docs/PRUEBA-1.3.0-DOS-PCS.md`. Se puede actualizar con la recuperación automática de dos PCs y la voz por 127.0.0.1.
5. Opcional: probar la voz por la ruta 127.0.0.1 con un test de dos procesos en puertos distintos.

## Decisiones técnicas
- Recuperación con dos PCs: se usa `Takeover` (la verificación ya lo acepta con n=2). Se exige internet en la PC que toma el servidor, para no levantarlo en una PC desconectada. La copia de la que vuelve queda apartada, no se pierde.
- La recuperación por mayoría (3+ PCs) sigue usando el puntaje de equipo: es determinista, todas las PCs ven el mismo. El ranking de red solo se usa para el traspaso.
- Hambre: no hay comando para fijarla, así que se usan efectos con reglas exactas y se mide hasta llegar. Al bajar el hambre, el juego gasta primero la saturación; al subir la saturación con el hambre incompleta, el juego también sube el hambre. Las dos cosas se informan en el mensaje.
- En modo creativo o espectador, y en dificultad pacífica, Minecraft no gasta hambre: se explica con ese motivo.

## Archivos principales tocados en esta etapa
- `crates/cluster/src/lib.rs` (remoto, estado, ranking, recuperación, voz, PlayerOp) y `crates/cluster/src/friends.rs`.
- `crates/core/src/`: `playeredit.rs`, `process.rs` (TPS), `repair.rs`, `optimize.rs`, `mods.rs`, `modrinth.rs`, `hardware.rs`, `log4shell.rs`, `model.rs`, `instance.rs`.
- `src-tauri/src/`: `network.rs`, `v3.rs`, `aliases.rs`, `commands.rs`, `main.rs`.
- `ui/`: `server.js`, `servers.js`, `players.js`, `people.js`, `alias.js`, `boot.js`, `core.js`, `optimize.js`, `repair.js`.
- Pruebas: `crates/core/tests/e2e_real.rs`, `crates/cluster/tests/{multi_node,seguridad}.rs`, `crates/cluster/src/bin/939node.rs`, `ci/cliente-real/cliente.py`.
- CI: `.github/workflows/pruebas-windows.yml`.
- Docs: `CHANGELOG.md`, `README.md`, `docs/PRUEBA-1.3.0-DOS-PCS.md`.

## Pendientes conocidos (sin ocultar)
- Optimización para Fabric 1.14 con Modrinth real: sin verificar hasta que corra la CI.
- Dirección `.939host` y voz entre dos PCs reales: falta la prueba con Archimak. Cada PC tiene que aceptar una vez el permiso de Windows para el archivo hosts; no hay forma de evitarlo sin administrador.
- Jugadores sin 939 Host (por IP): la voz UDP no pasa por la entrada pública (solo TCP de Minecraft).
- Edición del inventario con el jugador desconectado: no implementada; solo se puede ver.
- Clave de recuperación: queda una copia en `claves-939host-nuevas`; no se borra sin la confirmación del usuario.
- Publicación oficial: requiere la autorización del usuario. La 1.1.0 instalada no se actualiza sola (clave vieja): hay que instalar la 1.3.0 a mano una vez.

## Etapa 24/11 (resultados de la primera prueba entre dos PCs)

| Problema informado | Causa / solución | Verificación |
|---|---|---|
| 0 jugadores en 26.3 | 26.3 antepone `System chat: ` a «joined/left the game»: `RE_JOIN`/`RE_LEAVE`/`RE_LOST` lo aceptan | cliente real 26.3 local (`e2e_jugador_en_vivo`) + CI agrega Fabric 26.3 |
| Propiedad perdida en las dos PCs | Traspaso en dos fases: `OwnerOffer` → `OwnerAnswer` (la otra acepta) → `finish_ownership` firma y valida → `OwnerConfirm`; si algo falla, sigue la dueña anterior | multi_node `transferencia_de_propiedad_confirmada` |
| Pedido para alojar aceptado solo | Quitado el `auto_answer`; siempre espera respuesta | multi_node (3 pruebas esperan el pedido y responden) |
| «Sincronizando» eterno | `pull` siempre llama a `op_end` (también con error) y corta cada bloque a 60 s | código |
| Nombre de la coanfitriona no aparecía | La propietaria actualiza el nombre real en el grupo firmado; nombres vacíos se ignoran | multi_node completo pasó |
| `prueba.939host` seguía ocupada | Se libera al borrar el servidor; `alias::set` ignora registros de servidores que ya no existen | código |
| Teclas no se conservaban | `last_used_profile` prefiere el perfil propio (o `.minecraft`), no otro perfil de 939 Host con opciones por defecto | prueba unitaria nueva |
| Spawn protegido | Por defecto 0 al crear | código |
| Inventario | Arrastrar y soltar con `op: move` real y verificado; sin repintar mientras se arrastra | cliente real 26.3 (mover) |
| Chat | Selección, botón copiar, códigos detectados que se copian (nunca se ejecutan) | código |
| Grupo de chat | `group_add_members` + *Sumar amigos* | código |
| Cambiar versión | Selector de loader (Vanilla/Fabric/Forge/NeoForge), mismo Minecraft con otro loader, mods revisados para el loader nuevo, vuelta atrás restaura el loader | compila; `e2e_cambio_de_version` sigue igual |
| IPs que no servían | IP real de la red local con copia y «Copiar todas las direcciones» (solo las válidas) | código |
| Caché | *Ajustes → Liberar espacio de la caché* (no toca servidores, mundos, copias, fotos ni descargas recientes) | prueba unitaria |

No hecho en esta etapa: servidores Quilt (el modelo de servidor no lo tiene) y vincular un servidor
externo que no creó 939 Host («Otro puerto»). Guía de prueba: `docs/PRUEBA-1.3.0-DOS-PCS.md` pasos 22–32.
