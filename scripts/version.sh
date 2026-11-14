#!/usr/bin/env bash
# Imprime la versión de 939 Host (fuente única: src-tauri/tauri.conf.json).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])"
