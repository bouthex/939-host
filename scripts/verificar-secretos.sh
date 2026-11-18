#!/usr/bin/env bash
# Revisa que un ejecutable de 939 Host no lleve secretos ni ganchos de prueba.
# Uso: scripts/verificar-secretos.sh ruta/al/939Host.exe [--release]
# Falla si encuentra una clave con forma de clave de CurseForge ($2a$10$…), claves privadas PEM,
# o (con --release) los nombres de los ganchos de prueba y del puerto de depuración.
set -euo pipefail
exe="$1"; mode="${2:-}"
fail=0
check() { if LC_ALL=C grep -a -E -q -e "$1" "$exe"; then echo "✘ $2"; fail=1; else echo "✔ sin $2"; fi; }
check '\$2[aby]\$1[0-9]\$[./A-Za-z0-9]{53}' "claves con forma de clave de API de CurseForge"
check '-----BEGIN ([A-Z ]+ )?PRIVATE KEY-----' "claves privadas PEM"
check 'ghp_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}' "tokens de GitHub"
if [ "$mode" = "--release" ]; then
  check 'H939_E2E_DEVTOOLS_PORT|remote-debugging-port' "puerto de depuración de WebView2"
  check 'H939_TEST_(SLOW|FAIL)' "ganchos de prueba"
fi
exit $fail
