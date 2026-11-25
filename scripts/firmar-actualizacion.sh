#!/usr/bin/env bash
# Firma estable.json para una versión ya empaquetada y la verifica contra la raíz de la app.
# Lo usa el workflow Release (clave real del entorno protegido «publicacion») y la CI de pruebas
# (clave descartable): así el camino de publicación se ensaya en cada ejecución.
# Uso: H939_CLAVE_PUBLICACION=… scripts/firmar-actualizacion.sh <versión> <carpeta-dist> <distribucion.json> <raiz-1.json> [raiz-2.json …]
set -euo pipefail
V="${1:?versión}"; DIST="${2:?carpeta}"; DISTJSON="${3:?distribucion.json}"; RAIZ1="${4:?raiz-1.json}"; shift 4
[ -n "${H939_CLAVE_PUBLICACION:-}" ] || { echo "::error::Falta la clave de publicación (H939_CLAVE_PUBLICACION): no se firma nada."; exit 1; }
BIN="${PUBLICAR_BIN:-target/release/939publicar}"
for f in "939Host-$V-instalador.exe" "939Host-$V-portable.zip"; do [ -f "$DIST/$f" ] || { echo "::error::Falta $DIST/$f"; exit 1; }; done
datos="$(sed -n 's/^pub const DATA_VERSION: u32 = \([0-9]*\);/\1/p' crates/core/src/datos.rs)"
sig=()
for r in "$@"; do sig+=(--raiz-siguiente "$r"); done
"$BIN" firmar-ci --carpeta . --raiz "$DISTJSON" "${sig[@]}" --artefactos "$DIST" --version "$V" --datos "$datos" --notas "939 Host $V" --salida "$DIST"
"$BIN" verificar --raiz-inicial "$RAIZ1" --publicacion "$DIST/estable.json" --instalada 0.0.1
