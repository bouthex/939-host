# Sistema de actualizaciones — diseño propuesto (NO implementado)

Estado: **solo diseño**. 939 Host hoy no se actualiza solo: cada versión se instala a mano encima de la
anterior (el instalador conserva mundos, copias y configuración; lo verifica la CI). Nada de este
documento está desplegado. Las decisiones marcadas «Decide el propietario» se toman antes de escribir
código, porque algunas no se pueden deshacer (una clave pública que ya viaja dentro de la app no se puede
retirar de las copias instaladas).

## Objetivos

1. Solo el propietario del proyecto autoriza una versión oficial. Ni la CI, ni GitHub, ni quien tenga
   acceso al repositorio pueden publicar algo que la app acepte como actualización.
2. La app verifica todo antes de instalar: firma, canal, versión y SHA-256 del paquete.
3. Sin servidores propios ni costos obligatorios: los archivos pueden vivir en GitHub Releases (u otro
   lugar), porque la seguridad depende de la firma y no de dónde se descargan.
4. Nunca se actualiza con un servidor encendido o guardando, ni sin que la persona lo acepte.

## Separación entre desarrollo y publicación

| | Desarrollo | Publicación |
|---|---|---|
| Rama | `rediseno-v3` (y otras de trabajo) | `main`, solo por decisión del propietario |
| Quién | Cualquier sesión de trabajo, la CI | El propietario |
| Qué produce | Artefactos de prueba de la CI (sin firma de actualización) | Paquete + manifiesto firmado |
| Firma Authenticode | No | Cuando exista un certificado (ver [firma/OPCIONES.md](firma/OPCIONES.md)) |
| Firma de actualización | Nunca | Siempre, en la PC del propietario |

La CI compila y prueba; **no tiene la clave de actualización** ni la tendrá. El manifiesto se firma en la
PC del propietario después de revisar el SHA-256 del paquete que armó la CI. Así, una cuenta de GitHub
comprometida o un cambio malicioso en un workflow no alcanzan para empujar una actualización.

## Identidad criptográfica del propietario

Propuesta: un **conjunto de claves autorizadas**, embebido en la app, del que basta una firma válida:

| Clave | Dónde vive | Uso |
|---|---|---|
| Principal | TPM de la PC del propietario, protegida con **Windows Hello** (no exportable; cada firma pide rostro, huella o PIN) | Firmar cada versión |
| Recuperación | Generada sin conexión, guardada cifrada en dos soportes separados (por ejemplo, un pendrive guardado y una copia impresa en papel en otro lugar) | Solo si se pierde la principal |

- Windows Hello: la API `KeyCredentialManager` de Windows crea un par de claves RSA-2048 en el TPM y firma
  datos arbitrarios después de verificar a la persona. La clave privada nunca sale del TPM: ni siquiera el
  propio propietario puede copiarla, y un programa malicioso en su PC no puede firmar sin el gesto de Hello.
- Lo que la app verifica son firmas RSA-2048 con SHA-256 (las de Hello) o Ed25519 (la de recuperación),
  con bibliotecas de Rust conocidas (`rsa`, `ed25519-dalek`). La documentación de Microsoft y el código
  de Chromium que usa esta misma API no coinciden en el relleno (PSS o PKCS#1 v1.5): se confirma con una
  firma real en la PC del propietario antes de fijarlo.
- La clave de Hello es de ese usuario en esa PC: no se sincroniza ni se puede respaldar. Por eso existe la
  de recuperación.
- La identidad es una clave, no una persona: el manifiesto no lleva nombre, correo ni datos del
  propietario.

### Recuperación de identidad

- Si se rompe o se reemplaza la PC: con la clave de recuperación se firma un **cambio de claves** (nueva
  principal en el TPM de la PC nueva). Las apps instaladas lo aceptan porque viene firmado por una clave
  del conjunto.
- Si se filtra una clave: se publica un cambio de claves firmado por la otra que la quita. Las apps que lo
  reciben dejan de aceptarla.
- Si se pierden **las dos**: no hay forma segura de recuperar la cadena; habría que distribuir una
  versión nueva a mano y cada persona la instalaría como una instalación nueva. Por eso la recuperación se
  guarda en dos lugares físicos.

**Decide el propietario:** si usar Windows Hello (requiere TPM 2.0 y Hello configurado en su PC) o una
llave física de seguridad; dónde guardar la recuperación; si quiere una segunda clave principal en otra PC
propia.

## Manifiesto firmado

```json
{
  "app": "939host",
  "canal": "estable",
  "version": "0.9.1",
  "secuencia": 12,
  "minima": "0.9.0",
  "publicado": "2026-12-01T00:00:00Z",
  "vence": "2027-03-01T00:00:00Z",
  "paquete": { "nombre": "939Host-0.9.1-instalador.exe", "bytes": 8421376, "sha256": "…" },
  "claves": null
}
```

- **Canales Estable y Beta**: el canal va dentro de lo firmado; un manifiesto de Beta no se puede hacer
  pasar por uno de Estable. La app solo sigue el canal elegido (Estable por omisión).
- **Anti-retroceso**: la app guarda la mayor `secuencia` aceptada y rechaza cualquier manifiesto con una
  menor o igual, aunque la firma sea válida (evita que alguien sirva una versión vieja y vulnerable).
- **Vencimiento**: un manifiesto vencido no se acepta (evita que alguien congele a la app mostrándole
  siempre el mismo manifiesto viejo para que nunca vea una corrección).
- **Paquete**: se descarga, se compara tamaño y SHA-256 con lo firmado, y recién entonces se ejecuta. Si
  existe firma Authenticode, se exige además que coincida con el editor esperado.
- `claves`: solo en un cambio de claves (ver arriba).

## Flujo en la app (cuando se implemente)

1. Consulta el manifiesto del canal como mucho una vez por día y al abrir, sin enviar identificadores
   (solo una petición HTTPS simple; nada de telemetría).
2. Si hay una versión nueva válida, lo avisa; no descarga en segundo plano sin permiso.
3. Al aceptar: descarga, verifica y espera a que no haya servidores encendidos (o pide apagarlos con el
   cierre normal, que guarda y publica el estado final al grupo).
4. Ejecuta el instalador por usuario (sin administrador) y vuelve a abrir la app.
5. Si algo falla, la versión instalada sigue intacta (el instalador no reemplaza archivos con la app
   abierta; lo prueba la CI).

## Infraestructura

- Archivos y manifiestos en **GitHub Releases** del repositorio del proyecto: sin costo, sin servidor
  propio. GitHub puede dejar de servirlos, pero no puede falsificarlos.
- Más adelante, las PCs de un grupo podrían pasarse la actualización entre sí por la red P2P existente:
  la verificación es la misma y no depende de quién la entregue.

## Qué no se va a hacer

- Firmar desde la CI o guardar la clave de actualización en secretos de GitHub.
- Actualizar en silencio, forzar reinicios o actualizar con un servidor encendido.
- Enviar datos de uso para decidir actualizaciones.

## Antes de implementar

1. Decisiones del propietario listadas arriba.
2. Pruebas: manifiesto con firma inválida, canal cambiado, secuencia menor, vencido, paquete con un byte
   cambiado, cambio de claves válido e inválido, corte de red a mitad de descarga, servidor encendido
   durante la actualización.
3. Recién después: la primera versión que incluya la clave pública. Desde ahí, el conjunto de claves solo
   cambia con un cambio de claves firmado.
