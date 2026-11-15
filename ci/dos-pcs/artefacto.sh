#!/usr/bin/env bash
# Espera un artefacto de esta misma ejecución y lo descarga. Uso: artefacto.sh <nombre> <carpeta> [segundos]
set -u
name="$1"; dest="$2"; limit="${3:-2400}"; t=0
while [ "$t" -lt "$limit" ]; do
  if gh run download "$GITHUB_RUN_ID" -R "$GITHUB_REPOSITORY" -n "$name" -D "$dest" >/dev/null 2>&1; then echo "llegó $name"; exit 0; fi
  sleep 10; t=$((t + 10))
done
echo "no llegó $name en ${limit}s"; exit 1
