#!/usr/bin/env bash
# Arma la entrega de Windows a partir del .exe ya compilado:
#   dist/payload/                      archivos de la app
#   dist/939Host-<v>-instalador.exe    instalador NSIS (por usuario, sin administrador)
#   dist/939Host-<v>-portable.zip      versión portable
#   dist/SHA256SUMS.txt
# Uso: scripts/empaquetar.sh <versión> [carpeta-del-exe]
# Requiere: makensis (NSIS 3), zip. Respeta SOURCE_DATE_EPOCH para fechas fijas.
set -euo pipefail
VERSION="${1:?versión}"
BIN="${2:-target/x86_64-pc-windows-gnu/release}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}"

CONF_VERSION="$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")"
[ "$CONF_VERSION" = "$VERSION" ] || { echo "La versión $VERSION no coincide con tauri.conf.json ($CONF_VERSION)" >&2; exit 1; }

rm -rf dist && mkdir -p dist/payload/licencias
cp "$BIN/939Host.exe" "$BIN/WebView2Loader.dll" dist/payload/
cp src-tauri/icons/icon.ico installer/LEEME-INSTALACION.txt dist/payload/
cp PRIVACY.md dist/payload/PRIVACIDAD.md
cp LICENSE dist/payload/licencias/LICENSE-939Host.txt
cp licencias-terceros/LICENSE-*.txt dist/payload/licencias/
[ -f THIRD_PARTY_LICENSES.html ] && cp THIRD_PARTY_LICENSES.html dist/payload/licencias/

makensis -V2 -DVERSION="$VERSION" -DSRCDIR="$ROOT/dist/payload" ${SIGNED_UNINST:+-DSIGNED_UNINST="$SIGNED_UNINST"} installer/939host.nsi
mv "installer/939Host-$VERSION-instalador.exe" dist/

# ZIP portable con orden y fechas fijas (reproducible).
find dist/payload -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
(cd dist/payload && find . -type f | LC_ALL=C sort | sed 's|^\./||' | TZ=UTC zip -q -X -D -@ "../939Host-$VERSION-portable.zip")

(cd dist && sha256sum "939Host-$VERSION-instalador.exe" "939Host-$VERSION-portable.zip" > SHA256SUMS.txt \
  && (cd payload && sha256sum 939Host.exe WebView2Loader.dll) | sed 's|  |  payload/|' >> SHA256SUMS.txt)
cat dist/SHA256SUMS.txt
