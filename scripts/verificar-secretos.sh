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
check 'AKIA[0-9A-Z]{16}|xox[baprs]-[0-9A-Za-z-]{10,}|sk-[A-Za-z0-9]{32,}' "claves de AWS, Slack u OpenAI"
# Privacidad: rutas de carpetas personales de quien compiló (las del runner de GitHub no cuentan).
checkp() { if LC_ALL=C grep -a -P -q -e "$1" "$exe"; then echo "✘ $2:"; LC_ALL=C grep -a -P -o -e "$1" "$exe" | sort -u | head -5 | sed 's/^/    /'; fail=1; else echo "✔ sin $2"; fi; }
checkp '/home/(?!runner/)[a-z][a-z0-9._-]{1,30}/|/Users/(?!runner/)[A-Z][A-Za-z0-9._-]{1,30}/|C:\\Users\\(?!runneradmin|Public|Default)[A-Za-z][A-Za-z0-9._-]{1,30}\\' "rutas de carpetas personales"
# IPs literales: se listan para revisarlas (las de ejemplo de los avisos y de loopback son normales).
ips=$(LC_ALL=C grep -a -o -E '\b(10\.[0-9]{1,3}|192\.168|172\.(1[6-9]|2[0-9]|3[01]))\.[0-9]{1,3}\.[0-9]{1,3}\b' "$exe" | sort -u | tr '\n' ' ' || true)
echo "ℹ IPs privadas literales en el ejecutable: ${ips:-ninguna}"
if [ "$mode" = "--release" ]; then
  check 'H939_E2E_DEVTOOLS_PORT|remote-debugging-port' "puerto de depuración de WebView2"
  check 'H939_TEST_(SLOW|FAIL)' "ganchos de prueba"
  check 'H939_ACTUALIZACIONES_PRUEBA|H939_PRUEBA_VERSION_INSTALADA|H939_PRUEBA_FALLAR_TRAS_ACTUALIZAR' "fuente o versión de prueba de las actualizaciones"
fi
exit $fail
