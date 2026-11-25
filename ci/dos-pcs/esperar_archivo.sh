#!/usr/bin/env bash
# Espera a que exista un archivo local y, mientras tanto, muestra lo nuevo del registro.
# Uso: esperar_archivo.sh <ruta> [segundos] [registro]
set -u
f="$1"; limit="${2:-2400}"; logf="${3:-}"; t=0; shown=0
show() { [ -n "$logf" ] && [ -f "$logf" ] || return 0; n=$(wc -l < "$logf"); [ "$n" -gt "$shown" ] && sed -n "$((shown + 1)),${n}p" "$logf"; shown=$n; }
while [ "$t" -lt "$limit" ]; do
  show
  [ -f "$f" ] && { echo "listo: $f"; exit 0; }
  sleep 5; t=$((t + 5))
done
show; echo "no apareció $f"; exit 1
