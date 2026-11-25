#!/usr/bin/env bash
# Espera un artefacto de esta misma ejecución y lo descarga, mostrando el registro mientras tanto.
# Uso: artefacto.sh <nombre> <carpeta> [segundos] [registro]
set -u
name="$1"; dest="$2"; limit="${3:-2400}"; logf="${4:-}"; t=0; shown=0
show() { [ -n "$logf" ] && [ -f "$logf" ] || return 0; n=$(wc -l < "$logf"); [ "$n" -gt "$shown" ] && sed -n "$((shown + 1)),${n}p" "$logf"; shown=$n; }
while [ "$t" -lt "$limit" ]; do
  show
  if gh run download "$GITHUB_RUN_ID" -R "$GITHUB_REPOSITORY" -n "$name" -D "$dest" >/dev/null 2>&1; then echo "llegó $name"; exit 0; fi
  sleep 10; t=$((t + 10))
done
show; echo "no llegó $name en ${limit}s"; exit 1
