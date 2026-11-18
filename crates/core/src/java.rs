//! Runtimes de Java administrados: el que pide cada versión de Minecraft, descargado de
//! Eclipse Temurin (Adoptium) y verificado con su SHA-256.

use crate::error::{AppError, AppResult};
use crate::net;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct AdoptiumAsset { binary: AdoptiumBinary, version: AdoptiumVersion }
#[derive(Debug, Deserialize)]
struct AdoptiumBinary { package: AdoptiumPackage }
#[derive(Debug, Deserialize)]
struct AdoptiumPackage { link: String, checksum: String, name: String }
#[derive(Debug, Deserialize)]
struct AdoptiumVersion { semver: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub major: u32,
    pub version: String,
    pub path: String,
    pub size_mb: f64,
}

fn runtime_dir(major: u32) -> PathBuf { paths::runtimes().join(format!("java-{major}")) }

fn find_java(dir: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    let direct = dir.join("bin").join(exe);
    if direct.exists() { return Some(direct); }
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path().join("bin").join(exe);
        if p.exists() { return Some(p); }
    }
    None
}

pub fn installed_java(major: u32) -> Option<PathBuf> { find_java(&runtime_dir(major)) }

pub fn list_installed() -> Vec<RuntimeInfo> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(paths::runtimes()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(major) = name.strip_prefix("java-").and_then(|m| m.parse::<u32>().ok()) {
                if let Some(java) = find_java(&e.path()) {
                    let version = std::fs::read_to_string(e.path().join(".939-version")).unwrap_or_default();
                    out.push(RuntimeInfo { major, version: version.trim().to_string(), path: java.display().to_string(), size_mb: paths::dir_size(&e.path()) as f64 / 1_048_576.0 });
                }
            }
        }
    }
    out.sort_by_key(|r| r.major);
    out
}

/// Versiones de Java que 939 Host sabe instalar (Eclipse Temurin, imágenes JRE oficiales).
pub const AVAILABLE: &[u32] = &[8, 11, 17, 21, 25];

/// Java a instalar para un requisito de Mojang. Java 16 (Minecraft 1.17) no tiene imagen JRE
/// publicada: Java 17 es compatible y se usa en su lugar.
pub fn install_major(required: u32) -> u32 {
    match required { 0 => 21, 16 => 17, r if AVAILABLE.contains(&r) => r, r => AVAILABLE.iter().copied().find(|a| *a >= r).unwrap_or(r) }
}

/// Java a usar: el elegido si es compatible, si no el recomendado. Devuelve (java, aviso).
/// Reglas: nunca menos que lo que pide Mojang; Forge/NeoForge anteriores a 1.17 solo con Java 8.
pub fn pick(required: u32, choice: Option<u32>, mc: &str, loader: crate::model::Loader) -> (u32, Option<String>) {
    let rec = install_major(required);
    let Some(c) = choice else { return (rec, None) };
    if !AVAILABLE.contains(&c) { return (rec, Some(format!("Java {c} no está disponible; se usa Java {rec}."))); }
    if c < required.max(8) { return (rec, Some(format!("Minecraft {mc} necesita Java {required} o posterior; se usa Java {rec}."))); }
    let old = mc.starts_with("1.") && mc.split('.').nth(1).and_then(|m| m.parse::<u32>().ok()).map(|m| m < 17).unwrap_or(false);
    if old && c != 8 && matches!(loader, crate::model::Loader::Forge | crate::model::Loader::NeoForge) {
        return (8, Some("Forge para versiones anteriores a 1.17 solo funciona con Java 8; se usa Java 8.".into()));
    }
    (c, None)
}

/// Garantiza que exista Java `major`; lo descarga si hace falta.
pub async fn ensure<F: FnMut(&str, f64)>(major: u32, mut progress: F) -> AppResult<PathBuf> {
    if let Some(p) = installed_java(major) { return Ok(p); }
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let url = format!("https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os={os}&vendor=eclipse");
    let assets: Vec<AdoptiumAsset> = net::get_json(&url, "el catálogo de Java (Adoptium)").await?;
    let asset = assets.into_iter().next().ok_or_else(|| AppError::not_found(&format!("Java {major} para este sistema")))?;
    let pkg = asset.binary.package;
    let archive = paths::cache().join(&pkg.name);
    progress(&format!("Descargando Java {major}"), 0.0);
    net::download(&pkg.link, &archive, net::Hash::Sha256(&pkg.checksum), |done, total| {
        if let Some(t) = total { progress(&format!("Descargando Java {major}"), done as f64 / t as f64); }
    }).await?;
    progress(&format!("Instalando Java {major}"), 1.0);
    let dir = runtime_dir(major);
    let staging = paths::runtimes().join(format!(".staging-java-{major}"));
    let _ = std::fs::remove_dir_all(&staging);
    let archive_c = archive.clone();
    let staging_c = staging.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive_c, &staging_c))
        .await
        .map_err(|e| AppError::new("internal", "Falló la instalación de Java").with_detail(e))??;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| AppError::io("No se pudo instalar Java", e))?;
    let _ = std::fs::write(dir.join(".939-version"), &asset.version.semver);
    let _ = std::fs::remove_file(&archive);
    find_java(&dir).ok_or_else(|| AppError::new("java", "Java se descargó pero no se encontró el ejecutable."))
}

/// Extrae un zip protegiéndose de rutas maliciosas (zip-slip).
pub fn extract_zip(archive: &Path, dest: &Path) -> AppResult<()> {
    let file = std::fs::File::open(archive).map_err(|e| AppError::io("No se pudo abrir el archivo comprimido", e))?;
    let mut zip = zip::ZipArchive::new(file)?;
    std::fs::create_dir_all(dest).map_err(|e| AppError::io("No se pudo crear la carpeta", e))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| AppError::io("No se pudo crear una carpeta", e))?;
        } else {
            if let Some(p) = out.parent() { std::fs::create_dir_all(p).map_err(|e| AppError::io("No se pudo crear una carpeta", e))?; }
            let mut f = std::fs::File::create(&out).map_err(|e| AppError::io("No se pudo extraer un archivo", e))?;
            std::io::copy(&mut entry, &mut f).map_err(|e| AppError::io("No se pudo extraer un archivo", e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod pick_tests {
    use super::*;
    use crate::model::Loader;
    #[test]
    fn picks_compatible_java() {
        assert_eq!(pick(25, None, "26.2", Loader::Vanilla), (25, None));
        assert_eq!(pick(16, None, "1.17.1", Loader::Vanilla).0, 17);
        assert_eq!(pick(21, Some(17), "1.21.4", Loader::Fabric).0, 21, "no puede ser menor que lo requerido");
        assert_eq!(pick(17, Some(21), "1.20.1", Loader::Fabric), (21, None));
        assert_eq!(pick(8, Some(17), "1.12.2", Loader::Forge).0, 8, "Forge viejo solo con Java 8");
        assert_eq!(pick(8, Some(17), "1.12.2", Loader::Vanilla), (17, None));
    }
}

/// Variables de entorno que Java lee por su cuenta y que pueden cambiar cómo arranca (agregar un
/// `-javaagent`, cambiar la configuración de Log4j o anular la protección contra Log4Shell, ya que
/// `_JAVA_OPTIONS` se aplica después de la línea de comandos). Los procesos de Java de 939 Host no
/// las heredan: solo valen los argumentos que arma 939 Host.
pub const JAVA_ENV_BLOCKLIST: &[&str] = &["JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JDK_JAVA_OPTIONS", "CLASSPATH", "JAVA_OPTIONS", "JAVA_OPTS"];

/// Quita esas variables del entorno de un proceso hijo.
pub fn sanitize_env(c: &mut std::process::Command) -> &mut std::process::Command {
    for k in JAVA_ENV_BLOCKLIST { c.env_remove(k); }
    c
}

#[cfg(test)]
mod env_t {
    /// Un `_JAVA_OPTIONS` del entorno (se aplica después de la línea de comandos) no llega a Java.
    #[test]
    fn java_no_hereda_opciones_del_entorno() {
        let mut c = std::process::Command::new("x");
        c.env("_JAVA_OPTIONS", "-javaagent:malo.jar").env("PATH", "/usr/bin");
        super::sanitize_env(&mut c);
        let envs: Vec<(String, Option<String>)> = c.get_envs().map(|(k, v)| (k.to_string_lossy().into(), v.map(|v| v.to_string_lossy().into()))).collect();
        for k in super::JAVA_ENV_BLOCKLIST { assert!(envs.iter().any(|(n, v)| n == k && v.is_none()), "{k} tiene que quitarse: {envs:?}"); }
        assert!(envs.iter().any(|(n, v)| n == "PATH" && v.is_some()), "lo demás se conserva");
    }
}
