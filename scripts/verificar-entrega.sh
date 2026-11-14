#!/usr/bin/env bash
# Comprueba una carpeta de entrega: integridad de ZIP, estado de firma de cada .exe/.dll y SHA-256.
set -uo pipefail
dir="${1:-.}"
cd "$dir"
for z in *.zip; do [[ -e "$z" ]] && { unzip -tq "$z" >/dev/null && echo "ZIP íntegro: $z" || echo "ZIP DAÑADO: $z"; }; done
for f in $(find . -iname '*.exe' -o -iname '*.dll'); do
  info="$(osslsigncode verify -in "$f" 2>&1)"
  if grep -q "Signer's certificate" <<<"$info"; then
    # Quién firmó (la cadena completa la valida Windows con sus raíces: Get-AuthenticodeSignature).
    echo "Firmado: $f ($(grep -m1 'Subject:' <<<"$info" | sed 's/.*CN=//'))"
  else
    echo "SIN FIRMA: $f"
  fi
done
sha256sum $(ls *.zip *.exe 2>/dev/null) > SHA256SUMS.txt && cat SHA256SUMS.txt
