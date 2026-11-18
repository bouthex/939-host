#!/usr/bin/env bash
# Instala paquetes de Ubuntu en los runners de GitHub Actions sin depender del espejo que le toque.
#
# Algunos runners usan us-east-2.ec2.archive.ubuntu.com, que a veces baja a pocos KB/s y deja el
# paso «Herramientas» trabado más de 30 minutos. Este script fija espejos oficiales de Ubuntu
# (primero el de Azure, que usan la mayoría de los runners; después el principal de Canonical).
#
# La seguridad no cambia: apt verifica la firma de Ubuntu de cada índice y el hash de cada paquete,
# venga del espejo que venga.
#
# Uso: scripts/ci-apt.sh paquete1 paquete2 ...
set -euo pipefail
[ "$#" -gt 0 ] || { echo "uso: $0 paquete..." >&2; exit 2; }

MIRRORS='http://azure.archive.ubuntu.com/ubuntu/	priority:1
http://archive.ubuntu.com/ubuntu/	priority:2'

# Los runners de Ubuntu 24.04 leen la lista de espejos de este archivo (mirror+file:).
if [ -f /etc/apt/apt-mirrors.txt ]; then
  printf '%s\n' "$MIRRORS" | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
fi
# Por si alguna imagen apunta directo a un espejo de EC2 en sus fuentes.
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
  [ -f "$f" ] || continue
  sudo sed -i -E 's#https?://[a-z0-9.-]*\.ec2\.archive\.ubuntu\.com/ubuntu/?#http://azure.archive.ubuntu.com/ubuntu/#g' "$f"
done

# Sin datos durante 30 s, se corta y se reintenta (en el siguiente espejo si hace falta).
APT_OPTS=(-o Acquire::Retries=5 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
sudo apt-get "${APT_OPTS[@]}" update
sudo apt-get "${APT_OPTS[@]}" install -y --no-install-recommends "$@"
