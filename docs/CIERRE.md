# 939 Host: informe de cierre

Fecha: 18 de noviembre de 2026.
Rama `rediseno-v3`, commit final: ver la sección 1.
Sin merge, sin PR, sin release; `main` no se tocó.

Este informe completa a [`SEGURIDAD.md`](SEGURIDAD.md), [`RED-TEAM.md`](RED-TEAM.md),
[`AUDITORIA-INTEGRAL.md`](AUDITORIA-INTEGRAL.md) y [`AUDITORIA-GITHUB.md`](AUDITORIA-GITHUB.md).

939 Host no tiene seguridad absoluta. Lo que sigue es lo comprobado, lo corregido y lo que falta.

## 1. Estado real del proyecto

- **Al empezar:** HEAD `ef1b9e9`, igual al remoto y sin trabajo pendiente.
- **Al terminar:** el código probado es `970e8b3` (CI de Windows verde, 6 de 6); el commit
  final solo agrega este informe.
- **Pruebas automáticas:** 118 en Linux, todas pasan.
- **CI de Windows:** ver la sección 5.
- **Producto:** funciona como antes (crear y administrar servidores, grupos, traspaso,
  sincronización, mods y modpacks, archivos, copias). En esta fase no se agregaron funciones
  visibles salvo textos.

## 2. Vulnerabilidades encontradas en esta fase

| # | Hallazgo | Gravedad | Explotación |
|---|---|---|---|
| C1 | Variables de entorno de Java (`_JAVA_OPTIONS`, `JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, `CLASSPATH`) llegaban a los procesos de Java de 939 Host. `_JAVA_OPTIONS` se aplica **después** de la línea de comandos: podía anular la protección contra Log4Shell o cargar un `-javaagent`. | Medio | Hace falta poder definir variables de entorno del usuario (malware local u otro programa instalado). Desde otra PC del grupo, no. |
| C2 | Activar un mod aprobado: entre leer su huella y moverlo a `mods/`, un reemplazo del archivo quedaba activo sin comprobarse. | Bajo | Solo con acceso local a la carpeta de cuarentena (desde otra PC no se puede escribir ahí). |
| C3 | Caché de análisis de mods sin tope. | Informativo | Crecimiento lento de archivos chicos. |
| C4 | Un nombre de Minecraft real en `docs/PRUEBAS.md`. | Informativo | Dato público; sigue en el historial. |

En esta revisión no apareció ningún hallazgo **crítico** ni **alto** nuevo.

**Histórico.** Las fallas 1 a 16 del red team y V17 a V21 de la fase anterior siguen corregidas:
sus pruebas siguen en la suite y pasan.

## 3. Vulnerabilidades corregidas en esta fase

| # | Corrección | Prueba |
|---|---|---|
| C1 | Los procesos de Java (servidor e instaladores) no heredan esas variables. | `java::env_t::java_no_hereda_opciones_del_entorno`. **En la CI de Windows**, los servidores reales de los cuatro loaders arrancan con `_JAVA_OPTIONS=-Xmx2m` en el entorno: si se heredara, ninguno arrancaría. |
| C2 | La activación es un rename y después se vuelve a calcular la huella del archivo colocado; si no coincide con una aprobada, vuelve a la cuarentena. | `modguard::t::activacion_comprueba_la_huella_otra_vez` (reemplazo en ese instante). **Falla si se quita la recomprobación** (comprobado). |
| C3 | Tope de 500 análisis guardados. | `modscan::t::la_cache_de_analisis_tiene_tope` |
| C4 | Quitado del documento. | — |

## 4. Riesgos que siguen abiertos

| Riesgo | Gravedad | Por qué sigue |
|---|---|---|
| **Ejecutable sin firma.** Con Smart App Control activo (Windows 11), Windows **bloquea** 939 Host y no hay forma de habilitarlo sin apagar esa protección, cosa que no se le pide a nadie. Con SmartScreen aparece un aviso que se puede pasar con «Más información → Ejecutar de todos modos». | **Alto** para la distribución a usuarios reales | La firma gratuita de SignPath Foundation depende de su aprobación (sección 11). |
| Un mod aprobado puede hacer todo lo que puede el usuario de Windows. | Medio | Minecraft no tiene aislamiento; el análisis no detecta todo. No hay sandbox (sección 9). |
| En un grupo de dos PCs, la coanfitriona puede apagar a la anfitriona («Recuperar aquí»). | Medio | Sin una tercera PC no hay mayoría para votar. Queda registrado. |
| La coanfitriona recibe el mundo completo. | Medio | Lo necesita para alojarlo. |
| En modo normal o directo, la otra PC ve tu IP; el relay ve metadatos en modo privado. | Bajo | Es cómo funciona una conexión directa; el modo privado la oculta. |
| DPAPI no protege contra malware en la misma sesión de Windows. | Bajo | Límite del sistema operativo. |
| La identidad de cada PC es persistente; no hay rotación de la clave de una PC. | Bajo | Se reemplaza quitándola y volviéndola a invitar. |
| Datos personales de baja sensibilidad en el historial de Git. | Informativo | Reescribir el historial necesita tu autorización y no borra copias ya hechas. |

## 5. Pruebas realizadas y resultados

| Qué | Resultado |
|---|---|
| Suite automática (Linux) | **118 pasan**, 0 fallan |
| Análisis estático (clippy) | Sin hallazgos de seguridad (solo estilo) |
| `cargo audit` (688 dependencias) | **0 vulnerabilidades**; 7 avisos de paquetes sin mantenimiento o con uso inseguro conocido (chrono, paste, proc-macro-error, zbus/zvariant, glib), todos de la interfaz o de Linux |
| gitleaks sobre los 79 commits | Sin hallazgos |
| CI de Windows con `970e8b3` | **6 de 6 partes pasan** (detalle abajo) |

Lo que comprobó la CI de Windows con `970e8b3`:

- **Servidores reales:** Vanilla, Fabric, Forge y NeoForge 26.3 encendieron y se apagaron
  limpio. Lo hicieron **con `_JAVA_OPTIONS=-Xmx2m` en el entorno**: Java no lo heredó.
- **Mods de rendimiento reales:** con Fabric (4 mods) y NeoForge (3 mods).
- **Dos máquinas Windows por P2P y un cliente real** de Minecraft que entra al servidor.
- **App instalada:** abre, funciona con la interfaz real y la desinstalación conserva los datos.
- **Script elevado de `hosts`:** en Windows real.
- **Protección de Mojang contra Log4Shell:** descargada y verificada.
- **Instalador de prueba:**
  - `939Host-0.9.0-instalador.exe`:
    `d66dec305eebfd825f11437df20b77b72b616e9a80043d78097b1e2eea12b5ad`
  - `939Host-0.9.0-portable.zip`:
    `d832797d42888f94d9a2772a9b59efb8d1d6702e077af33156fc827badcfa5da`

Red team de esta fase, en entornos aislados y con datos ficticios:

| Ataque | Resultado |
|---|---|
| Externo sin permiso, mensajes malformados y enormes | Rechazado (pruebas existentes) |
| Jugadora o coanfitriona maliciosa, apropiarse de la anfitriona, repetir mensajes | Rechazado (pruebas existentes) |
| Volver después de ser quitada | Rechazado (`pc_quitada_pierde_el_acceso`) |
| JAR manipulado, dependencia maliciosa, reemplazo después de aprobar | Cuarentena (pruebas de `modguard` y `modscan`, incluida la carrera de activación nueva) |
| Saltarse la aprobación cambiando el nombre o instalando a mano | Rechazado (`rechazo_y_cuarentena_inmediata`) |
| Rutas manipuladas, junctions, nombres especiales | Rechazado (pruebas en Windows real) |
| HTML malicioso en nombres | Escapado; la CSP bloquea scripts |
| Escalar privilegios por la carpeta de la app (DLL o `.exe` falsos) | Ya no se eleva nada desde ahí (V17) |
| Inyectar opciones de Java por el entorno | Ya no llegan (C1) |
| Transferencias interrumpidas, caídas de red simuladas | Sin parciales activos; recuperación (pruebas existentes) |

## 6. Pruebas que no se pudieron realizar

- **PC con Windows doméstica:**
  - aviso de UAC real de «Windows PowerShell»;
  - Smart App Control activo;
  - Defender con la configuración de una PC común.

  La CI corre en máquinas de GitHub como administrador y sin aviso de UAC.
- **Dos redes domésticas distintas, NAT estricto y cortes de internet reales.** La CI usa dos
  máquinas Windows separadas de GitHub, que no son redes domésticas.
- **Secrets de GitHub:** esta conexión no tiene acceso a esa parte de la configuración.
- **Mods maliciosos reales:** se usaron muestras armadas con los indicadores, no malware.

## 7. Windows y Defender

**Comprobado en código y CI:**

- La app no pide administrador.
- Lo único elevado es el PowerShell de `System32`, con un script fijo que se valida otra vez y
  pasa como argumento (nunca un archivo).
- Los programas de Windows se llaman por ruta absoluta.
- Java no hereda variables que cambien cómo arranca.
- El inicio con Windows viene apagado; cuando se activa, la ruta va entre comillas.
- La desinstalación quita el programa, los accesos y la clave de inicio, y **conserva** los
  datos (lo comprueba la CI).
- Los datos y la identidad están en `%LOCALAPPDATA%\939Host`, con los permisos del usuario, y
  las claves con DPAPI.

**Defender y Firewall en la CI** (máquinas Windows Server 2025 de GitHub), estado real
registrado durante la corrida:

- Defender: servicio y antivirus activos, pero **protección en tiempo real, análisis de
  descargas y supervisión de comportamiento apagados** (así vienen esas máquinas).
- Firewall: activo en los perfiles de dominio, privado y público.
- Detecciones de Defender durante la corrida: ninguna.

**Conclusión honesta:** como la protección en tiempo real estaba apagada, que no haya detecciones
**no demuestra** que Defender, en una PC común, no vaya a marcar algo. Por ejemplo, el PowerShell
elevado con `-EncodedCommand` de las direcciones `.939host`. Sigue pendiente la prueba manual. El
Firewall estaba activo y todo funcionó con él: dos PCs por P2P, cliente real y relays.

**Pendiente manual:** sección 15.

## 8. GitHub y secretos

- **Historial:** sin claves, tokens, contraseñas ni credenciales en los 79 commits (gitleaks y
  búsquedas propias), ni en la release v0.9.0, ni en los logs revisados.
- **CurseForge:** el secreto `CURSEFORGE_API_KEY` estaba vacío en las compilaciones que lo
  habrían metido en el `.exe`, y el código ya no lo usa.
- **SignPath:** `SIGNPATH_API_TOKEN` solo se usa en el workflow de firma, que nunca corrió.
- **No se pudo comprobar** qué secrets existen en Settings: esta conexión no tiene acceso. Ver la
  sección 15.
- **Datos personales:** quitados del árbol actual; en el historial quedan un nombre de Minecraft
  con su UUID y una carpeta de usuario (detalle en `AUDITORIA-GITHUB.md`). **No se recomienda
  reescribir el historial.**

## 9. Sistema de mods

| Requisito | Estado |
|---|---|
| Lo recibido entra en cuarentena al instante | Sí: se coloca y se aparta dentro del candado que lo registra. |
| Lo que está en cuarentena no se carga | Sí: está en `mods-cuarentena/`, fuera de `mods/`; ningún loader lo lee. |
| Aprobación expresa de la propietaria, por SHA-256 exacto | Sí, firmada. Un byte distinto es otro archivo. |
| Dependencias | Cada `.jar` que llega se aprueba por separado. Los jars incluidos dentro de otro quedan cubiertos por la huella del archivo que los trae y se analizan. |
| Rechazos registrados y no evitables cambiando el nombre | Sí: firmados, por huella; valen en todas las PCs y con cualquier política. |
| Protección contra manipulación, repetición y carreras | Firma de la propietaria, revisión monotónica y candado de entradas. |
| Activación atómica con huella recomprobada | **Sí (nuevo)**. |
| La coanfitriona no aprueba | Sí. «Confiar en las coanfitrionas» es un permiso separado y visible de la propietaria (los rechazos siguen valiendo). |
| Un error de análisis no activa nada | Sí: el análisis solo informa; lo que se carga lo decide la aprobación. |

**Análisis.**

- Es estático y nunca ejecuta el mod.
- Revisa la estructura del JAR (rutas que salen de la carpeta, cifrado, bombas de compresión,
  scripts y ejecutables, código nativo), los metadatos y las dependencias, los jars incluidos,
  los indicadores de fractureiser, el robo de credenciales, los webhooks, la persistencia en
  Windows, la carga remota de código, la propagación a otros `.jar`, la ofuscación y la
  presencia de Log4j vulnerable.
- La procedencia se consulta en Modrinth solo por la huella.
- VirusTotal solo se abre por la huella en el navegador: **nunca se sube un archivo**.
- Estados: sin alertas conocidas, **información insuficiente**, precaución, sospechoso, amenaza
  confirmada, análisis incompleto. Estar publicado en Modrinth no se presenta como «seguro».
- **Falsos positivos con mods reales (CI de Windows):** Lithium, FerriteCore, Fabric API y
  Chunky dan «sin alertas conocidas»; Krypton da «precaución» por su código nativo de red.

**Sandbox: no existe y no se inventa.**

- Windows Sandbox necesita Windows Pro y activar una función del sistema.
- Un AppContainer no aísla de verdad a Java con mods.
- Una máquina virtual es infraestructura que el proyecto no puede exigir.

La protección real es no ejecutar nunca el mod durante el análisis y exigir la aprobación.

## 10. CurseForge y Modrinth

- **Modrinth:** funciona sin credenciales (búsqueda, instalación con huella verificada y
  procedencia por huella).
- **CurseForge:** sin cambios en esta fase, por decisión de cumplimiento.
  - La API oficial pide una clave intransferible que no puede compartirse ni incrustarse en el
    `.exe`.
  - Se conserva la importación manual desde la web.
  - Se conserva la clave propia opcional, guardada con DPAPI, que nunca se muestra ni se
    registra.
  - Sin scraping ni APIs no oficiales.

## 11. Red, privacidad y recuperación

**Comprobado en código y pruebas:**

- El modo privado usa solo relay: sin transporte IP propio; sin relay no conecta.
- UPnP, PCP y NAT-PMP están desactivados en todos los modos.
- Las conexiones son QUIC/TLS 1.3 de punta a punta, con identidad por clave Ed25519: el relay no
  puede leer ni modificar el contenido.
- Las invitaciones son de un uso, vencen y se pueden revocar.
- Las PCs quitadas pierden el acceso.
- Los permisos de anfitrión van firmados y por épocas: nunca hubo dos anfitrionas en las
  pruebas.
- La recuperación espera **72 h reales** en la app (20 s solo en el nodo de prueba), con aviso y
  cancelación.

**No probado:** redes domésticas distintas, NAT estricto y cortes reales.

## 12. Rendimiento

Mismo equipo, tres corridas, servidores simulados:

| Medida | Línea de base (`321ca9a`) | Ahora |
|---|---|---|
| RAM en reposo (propietaria / coanfitriona / jugadora) | 20,9 / 22,2 / 15,6 MB | 21,9 / 22,6 / 15,3 MB |
| CPU en reposo | ≤0,55 % | ≤0,5 % |
| Sincronizar 64 MB | 0,81 s | 0,6–0,8 s |
| Traspaso | 0,75 s | 0,54 s |

- La aprobación automática ya no relee todos los mods cada 5 s: 40 MB pasaron de 885 ms a
  0,3 ms por vuelta.
- Los registros tienen tope: auditoría con rotación a 2 MB, historial de permisos, decisiones
  (500) y análisis (500).
- **No medido:** Windows con Minecraft real y servidores muy grandes.

## 13. Dependencias y cadena de suministro

- `cargo audit`: 0 vulnerabilidades en las 688 dependencias que se compilan.
- Licencias: sin GPL; las MPL-2.0 (de la interfaz) tienen su aviso.
- Acciones de GitHub fijadas a un commit exacto.
- Descargas verificadas:
  - Java por SHA-256;
  - servidor de Mojang, Forge y NeoForge por SHA-1 publicado;
  - Fabric por Maven con SHA-1 (si ese repositorio no responde, se usa el lanzador oficial de
    `meta.fabricmc.net`, que no publica huella: es el único respaldo sin verificar, por HTTPS);
  - Modrinth por SHA-512;
  - CurseForge por SHA-1;
  - protección contra Log4Shell por SHA-1.
- Herramientas de compilación desde los espejos oficiales de Ubuntu, con firma verificada por apt.

## 14. Instalador y portable

- **Se instalan para el usuario**, sin administrador.
- **Instalador de prueba de esta fase:** artefacto `939Host-windows-prueba` de la corrida de
  `970e8b3` (hashes en la sección 5; vence a los 14 días). Se descarga desde Actions con tu
  cuenta.
- **No es una versión final:** no está firmado y se compila con la característica de pruebas,
  que habilita el puerto de depuración solo si se define una variable de entorno. Los releases se
  compilan sin ella.
- **La CI comprueba:** que la app instalada abre y funciona con la interfaz real, que la portable
  abre, que el `.exe` no contiene claves y que la desinstalación conserva los datos.

## 15. Acciones manuales pendientes para vos

1. **Secrets de GitHub.**
   - Entrá a GitHub → repositorio `939-host` → **Settings → Secrets and variables → Actions**.
   - Si aparece `CURSEFORGE_API_KEY`: no se usa y se puede borrar con «Remove» (el valor nunca
     estuvo en el código ni en los logs).
   - `SIGNPATH_API_TOKEN` hace falta solo para la firma; dejalo si vas a firmar.
   - Fijate también en **Environments** y en **Variables** que no haya nada que no reconozcas.
2. **Prueba en tu PC o en la de un amigo, con Windows Defender activo y sin desactivar nada.**
   - Instalá el build de prueba.
   - Creá un servidor y en Ajustes → Dirección `.939host` tocá **Activar**. Windows tiene que
     mostrar el aviso de UAC de **«Windows PowerShell»** (editor: Microsoft Windows).
   - Aceptalo y comprobá que la dirección funcione.
   - Fijate si Defender muestra algún aviso y anotalo.
3. **Si tenés Windows 11 con Smart App Control activo:** comprobá que el ejecutable sin firma
   queda bloqueado. Es esperable: no lo desactives. Esto se resuelve con la firma.
4. **Prueba entre dos casas** (redes distintas): unirse con un código, jugar, pasar el servidor a
   otra PC, modo privado, y cortar el internet de una PC un minuto para ver la reconexión.
5. **Firma:** confirmá en SignPath si la solicitud gratuita de SignPath Foundation fue aprobada.
   Si fue aprobada, seguí los pasos de [`firma/SIGNPATH.md`](firma/SIGNPATH.md) cuando quieras
   publicar.
   - **Importante:** SignPath Foundation firma gratis **proyectos de código abierto públicos**.
     Si el repositorio pasa a privado, esa opción deja de estar disponible.
6. **CurseForge:** confirmá que seguimos con la importación manual y la clave propia opcional
   (recomendado).

## 16. Evaluación final de preparación para usuarios reales

**Listo en lo técnico para pruebas cerradas con personas de confianza.**

- La autenticación, los permisos, la sincronización, la cuarentena de mods, la protección de
  archivos y la red tienen pruebas automáticas y CI de Windows real.
- No queda ningún hallazgo crítico ni alto en el código.

**No está listo para usuarios en general hasta resolver:**

1. **La firma digital (alto).** Sin firma, Smart App Control bloquea la app y SmartScreen la
   marca.
2. **Las pruebas manuales de la sección 15.** UAC real, Defender en una PC común y dos redes
   domésticas no se pueden dar por aprobadas con la CI.

Cuando estén la firma y esas pruebas, el siguiente paso sería una versión candidata. Esa decisión
y su publicación son tuyas.
