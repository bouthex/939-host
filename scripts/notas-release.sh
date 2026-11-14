#!/usr/bin/env bash
# Notas de la release: descargas + estado de firma + sección de la versión en CHANGELOG.md.
# Uso: [FIRMADA=1] scripts/notas-release.sh <versión>
set -euo pipefail
V="${1:?versión}"
cd "$(dirname "$0")/.."
REPO="https://github.com/bouthex/939-host"

echo "## Descargar"
echo
echo "| Archivo | Para quién |"
echo "|---|---|"
echo "| **939Host-$V-instalador.exe** | La mayoría: instala 939 Host para tu usuario (no pide administrador) |"
echo "| **939Host-$V-portable.zip** | Sin instalar: descomprimir y abrir \`939Host.exe\` |"
echo "| SHA256SUMS.txt | Hashes para verificar las descargas |"
echo
echo "Requisitos: Windows 10 u 11 de 64 bits. No hace falta cuenta de GitHub ni instalar nada más"
echo "(Java y el servidor de Minecraft se descargan solos cuando creás un servidor)."
echo
if [ "${FIRMADA:-0}" = 1 ]; then
  echo "> **Firma:** el instalador, \`939Host.exe\` y el desinstalador están firmados (Authenticode)."
  echo "> Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by [SignPath Foundation](https://signpath.org/)."
  echo "> Ver la [política de firma de código]($REPO/blob/main/CODE_SIGNING_POLICY.md)."
else
  echo "> **Firma:** esta versión **no está firmada**. Windows SmartScreen puede avisar y **Smart App Control"
  echo "> (Windows 11) la bloquea**. Se solicitó la firma gratuita de SignPath Foundation; cuando se apruebe,"
  echo "> los archivos de esta release se reemplazan por versiones firmadas con los mismos nombres."
  echo "> Ver [docs/INSTALACION.md]($REPO/blob/main/docs/INSTALACION.md)."
fi
echo
echo "**[Code signing policy]($REPO/blob/main/CODE_SIGNING_POLICY.md)** · [Privacidad]($REPO/blob/main/PRIVACY.md) · Licencia MIT"
echo
echo "Cada archivo tiene una atestación de procedencia de GitHub (\`gh attestation verify <archivo> -R bouthex/939-host\`)."
echo
awk -v v="$V" '
  index($0, "## " v) == 1 { on=1; print "## Cambios"; next }
  on && /^## / { exit }
  on { print }
' CHANGELOG.md
