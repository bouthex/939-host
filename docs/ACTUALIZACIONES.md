# Actualizaciones de 939 Host

Estado al 19/11/2026:

| Parte | Estado |
|---|---|
| Verificación de metadatos firmados, canales, anti-retroceso, vencimiento, rotación y revocación de claves | **Implementado y probado** (crate `host939-update`, pruebas `ataques.rs`) |
| Descarga en segundo plano, reanudable, con tope de tamaño y SHA-256 | **Implementado y probado** (servidor HTTP de prueba con cortes, sin Range, contenido falso, datos de más) |
| Instalación en un momento seguro (sin servidores ni operaciones), con permiso, y reapertura | **Implementado**; el instalador `/ESPERAR /REABRIR` se prueba en la CI de Windows; el flujo completo dentro de la app solo se puede probar cuando exista una distribución configurada |
| Migraciones de datos con respaldo, validación y restauración | **Implementado y probado** (`core::datos`); todavía no hay ninguna migración real |
| Herramienta del propietario `939publicar` (identidad, recuperación, preparar, autorizar, rotar, verificar) | **Implementado y probado** de punta a punta en Linux; la clave en Windows CNG se prueba en la CI de Windows (almacén de software: los equipos de GitHub no tienen TPM) |
| Distribución real (dónde se suben los archivos) | **Pendiente: decisión del propietario** |
| Raíz pública dentro de la app | **Pendiente: decisión del propietario** (irreversible una vez distribuida) |
| Panel de publicaciones con interfaz | **Diseñado** (abajo); hoy las mismas funciones están en `939publicar` |

Mientras falten la raíz y la dirección (`src-tauri/distribucion.json`), la app **no consulta nada** y lo
dice en Configuración → Actualizaciones: las versiones se siguen instalando a mano, encima de la anterior.

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

- Primera consulta 1 minuto después de abrir; después cada 6 horas más un margen al azar de hasta 30
  minutos (para no pedir todos a la vez). «Buscar ahora» en Configuración.
- Solo se piden archivos públicos (sin identificadores, cookies ni datos de la PC). Metadatos de hasta
  256 KB; si un servidor manda más, se descarta.
- La descarga sigue en segundo plano y continúa donde quedó si se corta. Queda en la caché de 939 Host
  (`cache\actualizaciones`).

### Instalación

- Aviso discreto una vez por versión: «lista para instalar». Nunca se instala sola.
- «Instalar ahora»: si hay servidores encendidos u operaciones en curso (copias, traspasos,
  sincronizaciones), se explica qué falta y no se hace nada. Si está libre: se verifica el archivo de
  nuevo, se abre el instalador con `/S /ESPERAR /REABRIR` y la app hace su cierre normal. El instalador
  espera hasta 2 minutos a que la app termine, instala por usuario (sin administrador), conserva los
  datos y vuelve a abrir 939 Host. Si la app no se cerrara, el instalador no reemplaza nada.
- Si una instalación falla o se corta, la versión anterior sigue funcionando (probado en la CI: el
  instalador cortado a los 400 ms y repetido deja la app en orden).

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

## Decisiones del propietario (pendientes)

1. Crear la identidad con `939publicar iniciar` en su PC y guardar la recuperación fuera de la PC.
2. Dónde distribuir (recomendado: GitHub Releases de una organización del proyecto).
3. Poner la raíz y la dirección en `src-tauri/distribucion.json`. Desde la primera versión que la lleve,
   cambiarla solo se puede con una rotación firmada.
4. Si se exige firma Authenticode para publicar (`politica.json`, hoy no, porque no hay certificado).

## Pruebas

| Tipo | Qué cubre |
|---|---|
| Automáticas (Linux y Windows) | Actualización correcta, firma inválida, clave ajena, tipo cambiado, firmas repetidas, firmas maleables, vencidos, retroceso, repetición, canales, vuelta atrás y datos migrados, rotación, revocación, recuperación, raíz falsa o salteada, estado manipulado, descarga cortada y reanudada, servidor sin Range, paquete manipulado, datos de más, cambio en disco antes de ejecutar, migraciones (éxito, falla a mitad, falla de validación, paso faltante, datos más nuevos), herramienta del propietario de punta a punta (bloqueos, confirmación, archivo cambiado entre preparar y autorizar, publicación repetida, rotación, recuperación ajena) |
| Windows (CI) | Clave en CNG no exportable, firma verificable, reabrir y borrar; instalador con `/ESPERAR` (no reemplaza mientras la app sigue abierta) y `/REABRIR` |
| Necesitan la PC del propietario | Clave en el TPM real, aviso de confirmación de CNG, Windows Hello |
| Necesitan infraestructura | Flujo completo app → servidor de distribución real → instalación (cuando se elija la distribución) |
