# Actualizaciones de 939 Host

Estado al 21/11/2026 (desde 1.1.0):

| Parte | Estado |
|---|---|
| Fuente oficial | **GitHub Releases** del repositorio (`releases/latest/download`). Solo las releases oficiales llevan `estable.json` firmado; las compilaciones de prueba de la CI nunca, así que nunca se ofrecen |
| Raíz pública dentro de la app | **Configurada** (`src-tauri/distribucion.json`, copia en `actualizaciones/raiz-1.json`), vence en 2036 |
| Clave de publicación | Secreto `H939_CLAVE_PUBLICACION` del entorno protegido **`publicacion`** de GitHub: solo se usa cuando el propietario aprueba la publicación |
| Clave de recuperación | La guarda el propietario fuera de la PC. Firma las raíces (rotar o revocar la clave de publicación) |
| Detección al abrir y cada 6 horas, descarga reanudable, SHA-256 y firma | **Implementado y probado** (pruebas `ataques.rs`, `distribucion.rs` y CI de Windows) |
| Instalación automática con vuelta atrás real (instalada y portable) | **Implementado y probado** en Windows (`ci/windows/actualizacion_e2e.py`) |
| Pantalla de actualización (cubos, estados reales, «menos movimiento») | **Implementado y probado** (`ci/vista-previa/actualizacion.py` y capturas reales de Windows) |
| Publicación | Workflow `Release` (etiqueta `vX.Y.Z`), con aprobación del propietario |

## Cómo funciona para quien usa 939 Host

1. Al abrir, 939 Host consulta enseguida la release oficial más reciente (sin frenar la apertura: si la
   respuesta tarda, la app abre igual). Si no hay nada nuevo, no se descarga nada y no aparece ninguna
   pantalla. Después vuelve a consultar cada 6 horas.
2. Si hay una versión más nueva (comparación semántica: 1.0.10 es mayor que 1.0.2), se descarga en
   segundo plano y se verifica: firma de la publicación, tamaño y SHA-256 firmados del paquete.
3. Si eso pasa al abrir la app, aparece la pantalla de actualización (los cubos de la presentación,
   girando mientras se trabaja) con los estados reales: *Descargando* (con porcentaje real), *Verificando
   archivos*, *Instalando actualización*, *Reiniciando 939 Host*. Al volver a abrir: *Actualización
   completada*. Si la app ya está en uso, aparece un aviso arriba con «Actualizar ahora»: no se
   reinicia en medio del trabajo.
4. Nunca se instala con servidores encendidos ni operaciones en curso (copias, traspasos,
   sincronizaciones): se explica qué falta y se espera.
5. En Configuración → Actualizaciones: versión, estado, «Buscar actualizaciones» y «Actualizar ahora».

### Instalación y vuelta atrás

- Antes de tocar nada, 939 Host copia **los archivos del programa** a
  `%LOCALAPPDATA%\939Host\actualizacion\anterior` y comprueba la copia (SHA-256). Los datos (servidores,
  mundos, identidad, foto, grupos, configuración, historiales) están en otra carpeta y no se tocan.
- Abre un **vigía** (una copia de sí misma, `939Host-vigia.exe`, sin ventana) y se cierra normalmente.
  El vigía espera a que termine de cerrarse, aplica la versión nueva y la abre:
  - **Instalada**: ejecuta el instalador verificado en silencio (`/S /ESPERAR /D=<misma carpeta>`), por
    usuario, sin permisos de administrador.
  - **Portable**: escribe los archivos del .zip verificado al lado (`.nuevo`) y recién con todos completos
    los pone en su lugar. Si la carpeta no se puede escribir (por ejemplo, dentro de *Archivos de
    programa*), lo explica y no toca nada.
- La versión nueva tiene 150 segundos para confirmar que su interfaz abrió. Si el instalador falla, si
  la versión nueva se cierra sola o no termina de abrir, el vigía **restaura los archivos anteriores**,
  vuelve a abrir la versión anterior y queda anotado qué pasó: se muestra un aviso y se puede
  reintentar. Esa versión no se reinstala sola en cada apertura.
- Si la actualización queda cortada (por ejemplo, se apagó la PC), al volver a abrir se informa y se
  puede reintentar.
- Sin conexión, sin espacio o con un paquete alterado: se explica, no se instala nada y se vuelve a
  intentar más tarde (la descarga continúa donde quedó).

### Primera vez con actualizaciones

La 1.0.0 (versión de prueba) y la v0.9.0 no consultan actualizaciones: **hay que instalar 1.1.0 una vez
a mano**, con su instalador, encima de la versión que esté (se conservan todos los datos; probado en la
CI desde v0.9.0 y desde 1.0.0). Desde 1.1.0 en adelante, todo es automático.

## Publicar una versión oficial

Configuración única (una sola vez, en GitHub → *Settings* del repositorio):

1. *Environments* → *New environment* → nombre `publicacion`.
2. En ese entorno: *Required reviewers* → el propietario (`bouthex`). Guardar.
3. *Environment secrets* → *Add secret* → nombre `H939_CLAVE_PUBLICACION`, valor: el contenido del
   archivo `clave-publicacion.secreta` que se entregó junto con esta versión. Después borrar ese archivo.
4. Guardar el archivo de recuperación fuera de la PC (papel y pendrive) y borrarlo de la PC.

Cada versión nueva:

1. Se suben las mejoras a la rama y se cambia la versión (`src-tauri/tauri.conf.json`, `Cargo.toml` y el
   valor por defecto del instalador) a la siguiente (por ejemplo, 1.1.1 o 1.2.0).
2. «Pruebas en Windows» tiene que quedar en verde para ese commit.
3. Con la aprobación del propietario se sube la etiqueta `vX.Y.Z` a ese commit.
4. El workflow `Release` comprueba que el commit tiene las pruebas en verde, compila la versión de
   entrega sin ganchos de prueba, y **espera la aprobación del propietario** en GitHub (*Review
   deployments* → *Approve*) antes de usar la clave. Firma `estable.json`, lo verifica y crea la release
   (marcada como la última) con el instalador, la portable, `SHA256SUMS.txt`, `estable.json` y las raíces.
5. A partir de ahí, cada 939 Host 1.1.0 o posterior la detecta al abrir (o en menos de 6 horas) y se
   actualiza solo.

Nada se publica solo por un commit, un push o una compilación de prueba. Sin la aprobación (o sin el
secreto), el workflow se detiene antes de crear la release.

### Si la clave de publicación se expone o se pierde

Con la clave de recuperación se firma una raíz 2 con una clave de publicación nueva
(`939publicar` → raíz siguiente), se agrega en `actualizaciones/raiz-2.json` y se reemplaza el secreto.
Las apps descargan la raíz nueva antes de aceptar cualquier publicación y desde ahí rechazan la clave
anterior. Si se perdieran las dos claves, habría que distribuir a mano una versión con una raíz nueva.

### Firma de código (Authenticode)

Las actualizaciones ya van firmadas con la clave del proyecto y la app no instala nada sin esa firma.
La firma Authenticode de Windows (que evita los avisos de SmartScreen) es otra cosa y requiere un
certificado; la gratuita de SignPath sigue pendiente. Cuando exista, el workflow firma los archivos
antes de calcular sus SHA-256 y no cambia nada más.

## TUF: por qué se adaptan sus principios en lugar de usarlo tal cual

[TUF](https://theupdateframework.io) es el estándar para actualizaciones seguras. La biblioteca de Rust
más usada es `tough` (AWS). Se evaluó usarla directamente:

- TUF separa cuatro roles (root, targets, snapshot, timestamp). El de **timestamp** está pensado para una
  clave en línea que vuelve a firmar cada pocas horas desde un servidor. 939 Host no tiene ni quiere un
  servidor propio encendido, y el propietario firma desde su PC: el rol no aporta lo que promete.
- Las delegaciones (varios equipos que publican partes distintas) no hacen falta, y fueron justamente
  donde `tough` tuvo dos vulnerabilidades en 2026 (CVE-2026-6966: una firma repetida contaba dos veces
  para el umbral; CVE-2026-6967: metadatos delegados sin vencimiento, hash ni largo comprobados),
  corregidas en 0.22.
- La clave del propietario vive en Windows CNG (P-256 no exportable); con `tough` igual habría que
  escribir un firmante propio.

Decisión: **implementación propia, pequeña y auditable (unas 600 líneas), con bibliotecas criptográficas
conocidas (`ed25519-dalek`, `p256` de RustCrypto) y los principios de TUF**: raíz con umbral y rotación
encadenada (la raíz N+1 la firman el umbral de la N y el de la N+1), metadatos con versión y vencimiento,
tamaño y hash de cada archivo, separación de roles (raíz/publicación), y protección explícita contra la
lista de ataques de TUF. Las dos fallas de `tough` tienen prueba propia acá (firmas repetidas, y no hay
delegaciones). Si algún día hay varios publicadores o un servidor propio, se puede pasar a TUF completo.

## Arquitectura

```
CI (GitHub Actions)          Propietario (su PC)                 Distribución (cualquier servidor)      App
compila y prueba  ──────►  939publicar preparar  ──────►  raiz-N.json, estable.json, beta.json  ──►  verifica todo
SHA256SUMS, evidencias      bloqueos + autorizar           instaladores                             descarga, espera
(sin claves)                firma con su clave (CNG/TPM)   (no hace falta confiar en él)            momento seguro
```

- **Compilación** (CI): no tiene ninguna clave de publicación. Solo produce archivos y sus SHA-256.
- **Autorización** (propietario): `939publicar` vuelve a medir los archivos, exige las pruebas
  obligatorias, la rama, la versión mayor y (cuando haya certificado) la firma Authenticode, muestra un
  resumen y pide escribir la versión para confirmar. Recién ahí firma.
- **Distribución**: cualquier lugar que sirva archivos. Un servidor comprometido puede negar el servicio
  o servir algo viejo, pero no colar un archivo: la app lo rechaza.

### Formatos

Sobre: `{"type", "payload" (base64 de los bytes exactos), "signatures": [{keyid, sig}]}`. La firma cubre
`"939host-actualizacion-v1\n" + tipo + "\n" + bytes`, así una firma de raíz no sirve como publicación y no
hace falta JSON canónico.

- **Raíz** (`raiz-N.json`): versión, vencimiento (1 año), claves públicas y dos roles con umbral:
  `root` = {dispositivo, recuperación}, umbral 1; `release` = {dispositivo}, umbral 1.
- **Publicación** (`estable.json`, `beta.json`): canal, secuencia, fecha, vencimiento (120 días por
  defecto), versión semántica, formato de datos, vuelta atrás sí/no, notas, y cada archivo con nombre,
  tamaño y SHA-256.

### Reglas de la app (todas con prueba automática)

| Regla | Protege contra |
|---|---|
| Firmas válidas de claves distintas del rol, hasta el umbral; el id de cada clave tiene que corresponder a la clave | Clave ajena, firma repetida, id falsificado |
| Tipo dentro de lo firmado | Usar una raíz como publicación |
| Vencimiento | Congelamiento (servir siempre lo mismo para que no llegue una corrección) |
| Secuencia mayor que la ya vista; igual solo si es idéntica | Repetición de una publicación vieja (*replay*), dos versiones con el mismo número |
| Versión mayor que la instalada; menor solo con «vuelta atrás» firmada | Retroceso a una versión vulnerable (*downgrade*) |
| Estable nunca lleva `-beta`, Beta siempre; quien eligió Estable no recibe Beta | Mezcla de canales |
| Tamaño y SHA-256 del archivo; nunca más bytes que los firmados | Paquete manipulado, llenar el disco |
| Raíz nueva solo si la firman la anterior y la nueva, sin saltear números; la última no puede estar vencida | Rotación falsa, raíz vieja |
| Al cambiar las claves de publicación se olvidan las secuencias vistas | Recuperación después de una clave comprometida que publicó una secuencia enorme |
| El estado guardado se vuelve a encadenar desde la raíz que viene en la app | Archivo de estado manipulado |
| Formato de datos de la versión ofrecida ≥ el actual | Volver atrás el programa con datos ya migrados |
| Se vuelve a verificar el archivo justo antes de ejecutarlo | Cambio en disco después de descargar |
| Firmas P-256 solo con «s bajo»; Ed25519 con verificación estricta | Firmas maleables |

### Versionado

MAJOR.MINOR.PATCH (semver). Beta: `X.Y.Z-beta.N`. Una versión estable `X.Y.Z` es mayor que todas sus
betas. Los usuarios de Beta reciben la mayor entre Estable y Beta.

### Comprobación y descarga

- Primera consulta al abrir (en paralelo con la presentación; la app no la espera más de un instante);
  después cada 6 horas más un margen al azar de hasta 30 minutos. «Buscar actualizaciones» en
  Configuración. Sin conexión, la app sigue normal y lo vuelve a intentar más tarde.
- Solo se piden archivos públicos (sin identificadores, cookies ni datos de la PC). Metadatos de hasta
  256 KB; si un servidor manda más, se descarta. Una versión ya descargada y verificada no se vuelve a
  pedir ni a bajar.
- La descarga sigue en segundo plano y continúa donde quedó si se corta. Queda en la caché de 939 Host
  (`cache\actualizaciones`).

### Instalación

Ver «Instalación y vuelta atrás» más arriba. El instalador sigue aceptando `/REABRIR` (compatibilidad),
pero desde 1.1.0 la reapertura y la vuelta atrás las hace el vigía.

### Migraciones de datos y vuelta atrás

- `version-datos` en la carpeta de datos (hoy formato 1). Antes de migrar se copia todo lo que no es mundo,
  punto de restauración, copia ni caché a `respaldos-datos\v<N>-<fecha>` (se guardan los 3 últimos);
  después se valida; si algo falla, se restaura el respaldo y se avisa.
- Los mundos no se migran nunca (Minecraft migra los suyos al abrirlos con una versión nueva).
- Si los datos son de una versión más nueva, la app no los toca: muestra un aviso y no abre.
- **Volver atrás el programa no deshace una migración**: una vuelta atrás firmada solo se ofrece si la
  versión anterior entiende los datos actuales; si no, la app la bloquea y hay que restaurar el
  respaldo de la migración (guía en [RECUPERACION.md](RECUPERACION.md)).

## Identidad del propietario

> Modelo en uso desde 1.1.0: **publicación desde GitHub** (`939publicar iniciar-ci` / `firmar-ci`). La
> clave de publicación es un secreto del entorno protegido `publicacion`, que solo se entrega al
> workflow con la aprobación del propietario; la de recuperación (Ed25519) la guarda el propietario
> fuera de la PC y es la única que firma raíces. El modelo con la clave dentro de Windows CNG (abajo)
> sigue disponible en `939publicar` si algún día se prefiere firmar desde la PC del propietario: se
> pasa a él con una rotación de raíz firmada con la recuperación.

- `939publicar iniciar` crea, sin pedir contraseñas:
  - **Clave del dispositivo**: ECDSA P-256 dentro de Windows CNG, **no exportable**. Primero intenta el
    **TPM** («Microsoft Platform Crypto Provider»); si la PC no tiene TPM 2.0 usable, el almacén de
    software de Windows. Con la opción por defecto, Windows muestra su propio aviso de confirmación en
    cada firma (protección fuerte de CNG).
  - **Clave de recuperación**: Ed25519 generada en el momento, escrita en un archivo para guardar fuera de
    la PC (papel y pendrive en lugares distintos) y borrar de la PC.
  - La **raíz 1**, pública, que es la que iría dentro de la app.
- La identidad es un conjunto de claves, no una PC, una IP ni una cuenta: si se cambia de PC, la raíz
  siguiente la firman la clave vieja y la nueva (`rotar`). Si se perdió la PC, la firma la recuperación
  (`rotar --con-recuperacion`). En los dos casos la clave anterior queda revocada para todas las apps
  que reciban la raíz nueva.
- Si se pierden las dos claves no hay forma segura de recuperar la cadena: habría que distribuir a mano
  una versión con una raíz nueva.
- Quien tenga el archivo de recuperación puede tomar el control de las publicaciones: por eso va fuera de
  la PC y en dos lugares físicos. No se protege con contraseña por pedido explícito del propietario
  (nada de contraseñas ni PIN); la protección es física.

### Windows Hello, CNG y TPM

- **CNG + TPM** (implementado): la clave no sale del chip; un programa malicioso no puede copiarla, solo
  intentar usarla mientras la PC está encendida y desbloqueada.
- **Windows Hello** (no implementado todavía): la API `KeyCredentialManager` crea claves RSA-2048
  protegidas por Hello y firma con un gesto (rostro, huella o PIN). Necesita la biblioteca WinRT y no se
  puede probar en la CI (los equipos de GitHub no tienen Hello). Se agregaría como confirmación local
  adicional, verificada en la PC del propietario. Hello confirma que alguien está frente a la PC; no
  protege contra un sistema ya comprometido.
- Lo que nunca pasa: la clave privada no está en el repositorio, el ejecutable, el instalador, los
  paquetes, la CI ni los metadatos (la prueba lo verifica sobre la raíz generada).

## Panel de publicaciones (diseño)

Hoy las funciones del panel están en `939publicar`. Una interfaz gráfica sería un programa local del
propietario (no un sitio web), que usa la misma biblioteca:

| Pantalla | Contenido |
|---|---|
| Estado | Versión estable y beta publicadas, secuencia, vencimiento de cada una y de la raíz (con aviso 30 días antes) |
| Candidata | Commit, rama, versión, archivos con SHA-256, estado de cada prueba obligatoria de la CI, firma Authenticode, errores críticos abiertos |
| Bloqueos | Lista de lo que impide publicar (las mismas reglas de `publish::gate`); el botón de autorizar no existe mientras haya bloqueos |
| Autorizar | Resumen, escribir la versión para confirmar, aviso de Windows (CNG) y firma. Un clic, un commit, un push o una CI verde no son una autorización: solo la firma con la clave del dispositivo |
| Historial | `publicaciones.jsonl`: qué versión, cuándo, con qué clave, commit y archivos, con el SHA-256 de los metadatos firmados |
| Identidad | Cambiar de PC, recuperar, ver claves vigentes y revocadas |

Las evidencias de la CI las arma el propietario con `gh` desde su cuenta (no hacen falta credenciales
nuevas). Costo: ninguno; no necesita infraestructura.

## Distribución: opciones y costos

| Opción | Costo | Límites | Riesgos |
|---|---|---|---|
| **GitHub Releases** del repositorio del proyecto | 0 | Archivos de menos de 2 GiB; GitHub no limita el tamaño total ni el ancho de banda | Depende de que la cuenta u organización del proyecto siga activa; GitHub puede cambiar condiciones. No puede falsificar nada |
| GitHub Pages (espejo de los metadatos) | 0 | Sitio de hasta 1 GB, límite blando de 100 GB por mes | Mismo proveedor que el anterior |
| Cloudflare R2 | 0 hasta 10 GB y 10 millones de lecturas por mes; después 0,015 USD/GB-mes; salida gratis | Puede pedir un medio de pago al crear el almacenamiento | Exige una cuenta con medio de pago de alguien |
| Servidor propio (VPS) | Desde unos 4–6 EUR por mes (Hetzner, precios de 2026, que subieron dos veces este año) | — | Hay que mantenerlo y pagarlo; no hace falta para esto |

Recomendación: **GitHub Releases** con una publicación fija (por ejemplo, etiqueta `actualizaciones`)
cuyos archivos se reemplazan, en una **organización de GitHub del proyecto con al menos dos dueños**
(para que no dependa de una sola cuenta personal). Costo 0. Opcional: espejo en GitHub Pages. La app
puede tener más de una dirección y probar la siguiente si una falla; como todo está firmado, agregar
espejos no reduce la seguridad.

## Decisiones del propietario

1. ~~Dónde distribuir~~: GitHub Releases del repositorio (elegido el 21/11/2026).
2. ~~Raíz y dirección en la app~~: configuradas desde 1.1.0.
3. Pendiente (una sola vez): crear el entorno `publicacion` con su secreto (ver «Publicar una versión
   oficial»).
4. Firma Authenticode: cuando haya certificado (sin costo, SignPath) o si se decide pagar uno.

## Pruebas

| Tipo | Qué cubre |
|---|---|
| Automáticas (Linux y Windows) | Actualización correcta, firma inválida, clave ajena, tipo cambiado, firmas repetidas, firmas maleables, vencidos, retroceso, repetición, canales, vuelta atrás y datos migrados, rotación, revocación, recuperación, raíz falsa o salteada, estado manipulado, descarga cortada y reanudada, servidor sin Range, paquete manipulado, datos de más, cambio en disco antes de ejecutar, migraciones (éxito, falla a mitad, falla de validación, paso faltante, datos más nuevos), herramienta del propietario de punta a punta (bloqueos, confirmación, archivo cambiado entre preparar y autorizar, publicación repetida, rotación, recuperación ajena) |
| Windows (CI) | Clave en CNG no exportable, firma verificable, reabrir y borrar; instalador con `/ESPERAR` (no reemplaza mientras la app sigue abierta) y `/REABRIR` |
| Necesitan la PC del propietario | Clave en el TPM real, aviso de confirmación de CNG, Windows Hello |
| Windows (CI), actualización real | `ci/windows/actualizacion_e2e.py` con una distribución local firmada con una clave de prueba descartable: instalada (detecta, descarga, verifica, instala, reinicia, confirma, datos intactos), paquete alterado, sin conexión, versión que no abre (vuelta atrás, aviso, sin reintento solo, reintento manual), actualización cortada, portable, portable en carpeta sin permiso. Además: instalación de 1.1.0 encima de v0.9.0 y de la versión de prueba 1.0.0 sin perder identidad ni preferencias |
| Interfaz | `ci/vista-previa/actualizacion.py`: estados en orden, porcentaje real, sin parpadeo, error con reintento, aviso con la app en uso, «menos movimiento» |
| Con la release real | La primera publicación oficial firmada con la clave real (no se hizo: requiere la aprobación del propietario) |
