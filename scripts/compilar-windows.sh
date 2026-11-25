#!/usr/bin/env bash
# Compila 939Host.exe para Windows x64 desde Linux (MinGW-w64), igual que la CI.
# Requiere: rustup (usa la versión fijada en rust-toolchain.toml) y mingw-w64.
set -euo pipefail
cd "$(dirname "$0")/.."
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}"
# Rutas neutras dentro del binario (no queda la carpeta de quien compila).
SYSROOT="$(rustc --print sysroot)"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$PWD=. --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=cargo --remap-path-prefix=$SYSROOT=rustc"
# CARGO_FEATURES=pruebas solo en la compilación de prueba de la CI (nunca en releases).
cargo build --release --locked --target x86_64-pc-windows-gnu -p host939 ${CARGO_FEATURES:+--features "$CARGO_FEATURES"}
ls -l target/x86_64-pc-windows-gnu/release/939Host.exe target/x86_64-pc-windows-gnu/release/WebView2Loader.dll
