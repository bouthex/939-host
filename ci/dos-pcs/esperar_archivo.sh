#!/usr/bin/env bash
# Espera a que exista un archivo local. Uso: esperar_archivo.sh <ruta> [segundos] [registro]
set -u
f="$1"; limit="${2:-2400}"; logf="${3:-}"; t=0
while [ "$t" -lt "$limit" ]; do
  [ -f "$f" ] && { echo "listo: $f"; exit 0; }
  sleep 5; t=$((t + 5))
done
echo "no apareció $f"; [ -n "$logf" ] && tail -80 "$logf"; exit 1
