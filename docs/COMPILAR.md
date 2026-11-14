# Compilar 939 Host

Las versiones oficiales se compilan en GitHub Actions con estos mismos pasos (ver
[.github/workflows](../.github/workflows)). Para usar 939 Host **no** hace falta compilar nada: bajá el
instalador de la [última versión](https://github.com/bouthex/939-host/releases/latest).

## Requisitos

La compilación oficial es **desde Linux** (Ubuntu 24.04 o WSL en Windows) hacia Windows x64 con MinGW-w64:

```bash
sudo apt-get install -y mingw-w64 nsis zip python3
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # rustup
```

`rust-toolchain.toml` fija la versión exacta de Rust; `rustup` la instala sola.

## Compilar y empaquetar

```bash
scripts/compilar-windows.sh                 # target/x86_64-pc-windows-gnu/release/939Host.exe (+ WebView2Loader.dll)
scripts/empaquetar.sh "$(scripts/version.sh)"
```

Resultado en `dist/`: instalador, portable `.zip` y `SHA256SUMS.txt`.

Para desarrollar en Windows con MSVC también funciona `cargo build -p host939` (el cargador de WebView2
queda enlazado dentro del `.exe`), pero las versiones publicadas usan la compilación de arriba.

## Pruebas

```bash
cargo test --locked -p host939-core -p host939-p2p -p host939-cluster
```

Hay pruebas marcadas `#[ignore]` que levantan servidores de Minecraft reales: necesitan Java 25,
conexión a Mojang y variables `HOST939_TEST_*` (ver [PRUEBAS.md](PRUEBAS.md) y los comentarios de cada
prueba). No forman parte de la CI.

## Reproducibilidad

- Versión de Rust fija (`rust-toolchain.toml`) y dependencias fijas (`Cargo.lock`, `--locked`).
- Rutas de compilación neutralizadas (`--remap-path-prefix`) y `SOURCE_DATE_EPOCH` tomado del último commit.
- El `.zip` portable se arma con orden y fechas fijas.
- Las releases se compilan sin caché, en runners alojados por GitHub, y llevan atestación de procedencia.

El instalador NSIS y la versión de MinGW del runner pueden variar entre imágenes de Ubuntu, así que el
hash del instalador puede no coincidir byte a byte con una compilación local.

## Publicar una versión

1. Actualizar la versión en `src-tauri/tauri.conf.json` y `Cargo.toml` (`[workspace.package]`), y
   correr `cargo update --workspace`.
2. Agregar la sección `## <versión> — <fecha>` en `CHANGELOG.md`.
3. Commit en `main`, y después una de dos:
   - subir la etiqueta: `git tag -a v<versión> -m "939 Host <versión>" && git push origin v<versión>`, o
   - *Actions → Release → Run workflow* sobre `main` (crea la etiqueta `v<versión>` sobre ese commit).
4. El workflow **Release** prueba, compila, publica la release con instalador, portable y SHA-256, y
   genera la atestación de procedencia.
5. Si SignPath ya aprobó el proyecto: *Actions → Release firmada (SignPath) → Run workflow* con la
   etiqueta. Después de aprobar la firma en SignPath, los archivos de la release se reemplazan por los
   firmados. Ver [firma/SIGNPATH.md](firma/SIGNPATH.md).

## Licencias de terceros

`THIRD_PARTY_LICENSES.html` se genera con [cargo-about](https://github.com/EmbarkStudios/cargo-about):

```bash
cargo install cargo-about --locked --features cli
cargo about generate --locked about.hbs -o THIRD_PARTY_LICENSES.html
```

Regeneralo cuando cambien las dependencias. Solo se aceptan las licencias listadas en `about.toml`.
