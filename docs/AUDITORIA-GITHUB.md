# Auditoría de GitHub y datos sensibles

Fecha: 18 de noviembre de 2026.

**Alcance:** todo lo accesible públicamente del repositorio `bouthex/939-host`:

- las ramas `main`, `rediseno-v3` y la de Dependabot;
- los 6 PRs de Dependabot (cerrados) y el tag `v0.9.0`;
- 69 commits, con 576 versiones de 193 archivos distintos;
- los logs y artefactos de GitHub Actions;
- la release v0.9.0 y sus archivos.

**Resultado:** no se encontró ninguna clave, token, contraseña, clave privada ni credencial, ni en
el código actual ni en el historial. Se encontraron algunos datos personales de baja
sensibilidad. Hacer privado el repositorio no los borra del historial ni de las copias que ya se
hayan hecho, pero no hace falta reescribir el historial.

## Cómo se revisó

| Qué | Cómo |
|---|---|
| Secretos en todo el historial | `gitleaks` 8.24.2 sobre los 69 commits de todas las ramas: **0 hallazgos**. |
| Patrones propios en cada versión de cada archivo | IPv4, correos, rutas `C:\Users\…` y `/home/…`, claves privadas, tokens de GitHub, formato de clave de CurseForge, códigos de invitación (`939-…`), identidades de nodo (64 hex), UUID, nombres de PC (`DESKTOP-…`), JWT, asignaciones `password=`/`token=`. |
| Archivos borrados que siguen en el historial | Comparación entre los 193 archivos del historial y los de las ramas actuales. |
| Capturas | Las 17 versiones de imágenes del historial: texto extraído con OCR y revisión visual de las de consola, grupo y archivos. |
| Python compilado (`.pyc`) | Se extrajo la ruta de origen guardada en cada uno. |
| Secreto de CurseForge | Historial de su uso y valor en los logs de las compilaciones de esa época. |
| Release v0.9.0 | Descarga del portable, verificación de SHA-256 y búsqueda de rutas, claves y tokens en el `.exe`. |
| Autores de commits | Nombres y correos de autor y de quien hizo el commit. |

## Hallazgos

| Dato | Dónde | Sensibilidad | Estado |
|---|---|---|---|
| Nombre de Minecraft «Bouthex» y su UUID, copiados de un log real | `crates/core/src/reach.rs` (prueba), desde el 15/11 | Baja: el UUID de una cuenta es público (se consulta por nombre), pero vincula la cuenta de GitHub con la de Minecraft | Reemplazado por datos ficticios en `95debff`. **Sigue en el historial.** |
| `C:\Users\Bout` y `/home/bout` | `crates/core/src/diagnostics.rs` (prueba de ocultar datos) | Baja: podría ser el nombre de una carpeta de usuario de Windows | Reemplazado en `95debff`. Sigue en el historial. |
| Nombre del mundo «939 Z» | `docs/PRUEBAS.md`, `docs/INSTALAR-PRUEBA.md`, `CHANGELOG.md` | Muy baja | Se deja: son instrucciones de no usarlo para pruebas. |
| «Archimak» y «Bout» como autores | Créditos de la app y del README | Ninguna: son créditos públicos a propósito | Sin cambios. |
| Tres `.pyc` en `ci/` | Árbol actual | Ninguna: solo guardan rutas relativas | Quitados del repositorio e ignorados en `95debff`. |
| Ruta `/home/runner/work/939-host/…` en el `.exe` de la v0.9.0 | Binario publicado | Ninguna: es la carpeta del runner de GitHub, no de una PC personal | Sin cambios. |
| Correos de terceros (autores de librerías) | `THIRD_PARTY_LICENSES.html` | Ninguna: avisos de licencia obligatorios | Sin cambios. |

**Descartados después de revisarlos:**

- **IPs:** todas son de loopback (`127.x`), privadas de ejemplo (`10.x`, `192.168.0.12`), de
  documentación (`203.0.113.9`, reservada para ejemplos) o números de versión que parecen IPs
  (`26.1.0.4`).
- **Cadenas de 64 caracteres hexadecimales:** hashes de `Cargo.lock` e identificadores públicos
  de texturas de Minecraft. No hay identidades de nodo.
- **UUID:** el de «Notch» (prueba de UUID sin conexión) y uno de ejemplo para datos de jugador.
- **Códigos de invitación, claves privadas, tokens, JWT y nombres de PC:** ninguno.
- **Capturas:** salieron de la CI, con un servidor ficticio («Prueba Windows») y nombres de PC
  generados al azar («Mate», «Bizcochito»). No muestran IPs, rutas, identidades ni códigos.
- **Autores de commits:** 62 de `bouthex <bouthex@users.noreply.github.com>` (correo privado de
  GitHub) y 7 de Dependabot. No hay correos personales.

## Secretos de CI

| Secreto | Uso | Exposición |
|---|---|---|
| `CURSEFORGE_API_KEY` | Del 16 al 18/11, las compilaciones lo pasaban como `H939_CURSEFORGE_KEY` y quedaba **dentro del `.exe`** (`option_env!`). Desde `efaa850` no se usa. | **No hubo exposición.** En los logs de las compilaciones de esos días el valor aparece vacío: el secreto no estaba cargado. La v0.9.0 es anterior (14/11). |
| `SIGNPATH_API_TOKEN` | Solo en `release-firmada.yml`, como entrada de la acción de SignPath, nunca en el binario. | Ese workflow **nunca se ejecutó**. |

No se pudo listar qué secretos están cargados en el repositorio: esta conexión no tiene acceso a
esa parte de la API. Se puede revisar en **Settings → Secrets and variables → Actions**. Si
existe `CURSEFORGE_API_KEY`, conviene borrarlo, porque ya no se usa.

## Logs y artefactos de Actions

- **Logs:** son públicos mientras el repositorio sea público. Muestran identidades de nodo y
  códigos de invitación **de los nodos de prueba de la CI**, que se crean y se destruyen en cada
  corrida y no corresponden a ninguna persona. No muestran secretos (GitHub los enmascara).
- **Artefactos:** los de prueba (instalador, capturas y resultados de las dos PCs) los puede
  descargar cualquier persona con cuenta de GitHub hasta que vencen (1 a 14 días). Tienen los
  mismos datos ficticios.

## Qué no cambia al hacer privado el repositorio

- Las copias que alguien ya haya hecho (clones, forks, archivos descargados) siguen existiendo.
- La release v0.9.0 deja de estar disponible para quien no tenga acceso.
- **No hace falta revocar nada:** no se encontraron credenciales expuestas.

## Recomendación sobre el historial

**No reescribirlo.** Lo que queda (un nombre de Minecraft con su UUID público y un posible nombre
de carpeta de usuario) es de baja sensibilidad. Reescribir el historial rompe los clones y las
referencias de commits, y no borra las copias ya hechas.

Si igual querés borrarlo, se puede hacer con `git filter-repo` y un force push de todas las ramas
y tags. Sería una operación destructiva: solo con tu autorización explícita.

## Limitaciones

- No se pudo descargar ningún artefacto de Actions desde esta sesión: la red bloquea el
  almacenamiento de GitHub. Se revisaron sus nombres y tamaños, y el contenido de los logs que
  los generaron.
- No se leyeron uno por uno todos los logs de las 36 corridas de «Pruebas en Windows»: se
  revisaron los de la ventana de riesgo de CurseForge.
- Los PRs cerrados de Dependabot solo cambian versiones de dependencias.
