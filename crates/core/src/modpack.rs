//! Modpacks como servidor.
//!
//! - `.mrpack` (formato de Modrinth): cada archivo declara `env.server`; se instalan solo los
//!   que el servidor necesita, verificados por SHA-1, y se aplican `overrides/` y
//!   `server-overrides/` (nunca `client-overrides/`). Descargas solo de los dominios que permite
//!   el formato.
//! - CurseForge (`.zip` con `manifest.json`): los archivos se piden por la API oficial (hace falta
//!   la clave) y respetan el permiso de distribución de cada autor. CurseForge no indica qué mods
//!   son solo de cliente: se avisa, y si el servidor no arranca, el asistente de reparación los
//!   identifica.
//! - Primero se crea e instala el servidor con la versión y el loader del pack; después se
//!   agregan sus archivos (cuando termina la instalación).

use crate::error::{AppError, AppResult};
use crate::model::Loader;
use crate::{files, mods, net, paths};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    /// mrpack | curseforge
    pub format: String,
    pub name: String,
    pub version: String,
    pub mc_version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
    pub server_files: usize,
    pub client_only: usize,
    pub warnings: Vec<String>,
}

fn zip_json(path: &Path, name: &str) -> AppResult<serde_json::Value> {
    let f = std::fs::File::open(path).map_err(|e| AppError::io("No se pudo abrir el modpack", e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| AppError::invalid("El archivo no es un modpack válido (no es un .zip).").with_detail(e))?;
    let mut e = z.by_name(name).map_err(|_| AppError::invalid(format!("Al modpack le falta {name}.")))?;
    let mut s = String::new();
    e.read_to_string(&mut s).map_err(|e| AppError::io("No se pudo leer el modpack", e))?;
    Ok(serde_json::from_str(&s)?)
}

fn is_mrpack(path: &Path) -> bool {
    std::fs::File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok()).map(|mut z| z.by_name("modrinth.index.json").is_ok()).unwrap_or(false)
}

/// Lee qué es el pack sin instalar nada.
pub fn inspect(path: &Path) -> AppResult<PackInfo> {
    if is_mrpack(path) {
        let idx = zip_json(path, "modrinth.index.json")?;
        let deps = &idx["dependencies"];
        let (loader, lv) = if let Some(v) = deps["fabric-loader"].as_str() { (Loader::Fabric, Some(v.to_string())) }
            else if let Some(v) = deps["neoforge"].as_str() { (Loader::NeoForge, Some(v.to_string())) }
            else if let Some(v) = deps["forge"].as_str() { (Loader::Forge, Some(v.to_string())) }
            else if deps["quilt-loader"].is_string() { return Err(AppError::invalid("Es un modpack de Quilt: 939 Host todavía no crea servidores Quilt.")); }
            else { (Loader::Vanilla, None) };
        let fs = idx["files"].as_array().cloned().unwrap_or_default();
        let server = fs.iter().filter(|f| f["env"]["server"].as_str() != Some("unsupported")).count();
        return Ok(PackInfo { format: "mrpack".into(), name: idx["name"].as_str().unwrap_or("Modpack").into(), version: idx["versionId"].as_str().unwrap_or("").into(),
            mc_version: deps["minecraft"].as_str().ok_or_else(|| AppError::invalid("El modpack no dice su versión de Minecraft."))?.into(), loader, loader_version: lv,
            server_files: server, client_only: fs.len() - server, warnings: vec![] });
    }
    let m = zip_json(path, "manifest.json")?;
    let mc = &m["minecraft"];
    let primary = mc["modLoaders"].as_array().and_then(|a| a.iter().find(|l| l["primary"].as_bool() == Some(true)).or(a.first())).and_then(|l| l["id"].as_str()).unwrap_or("");
    let (loader, lv) = match primary.split_once('-') {
        Some(("forge", v)) => (Loader::Forge, Some(v.to_string())),
        Some(("neoforge", v)) => (Loader::NeoForge, Some(v.to_string())),
        Some(("fabric", v)) => (Loader::Fabric, Some(v.to_string())),
        _ => return Err(AppError::invalid(format!("Loader del modpack no soportado: «{primary}»."))),
    };
    let n = m["files"].as_array().map(|a| a.len()).unwrap_or(0);
    Ok(PackInfo { format: "curseforge".into(), name: m["name"].as_str().unwrap_or("Modpack").into(), version: m["version"].as_str().unwrap_or("").into(),
        mc_version: mc["version"].as_str().ok_or_else(|| AppError::invalid("El modpack no dice su versión de Minecraft."))?.into(), loader, loader_version: lv,
        server_files: n, client_only: 0, warnings: vec!["CurseForge no indica qué mods son solo para jugadores: se instalan todos. Si el servidor no arranca, el asistente de reparación muestra cuál sacar.".into()] })
}

fn allowed(url: &str) -> bool { ["https://cdn.modrinth.com/", "https://github.com/", "https://raw.githubusercontent.com/", "https://gitlab.com/"].iter().any(|p| url.starts_with(p)) }

/// Extrae una carpeta del zip (`overrides/` …) dentro del servidor, sin salir de su carpeta.
fn extract_dir(path: &Path, prefix: &str, run: &Path) -> AppResult<usize> {
    let f = std::fs::File::open(path).map_err(|e| AppError::io("No se pudo abrir el modpack", e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| AppError::invalid("Modpack dañado").with_detail(e))?;
    let mut n = 0;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| AppError::invalid("Modpack dañado").with_detail(e))?;
        let name = e.name().to_string();
        let Some(rel) = name.strip_prefix(prefix) else { continue };
        if rel.is_empty() || e.is_dir() { continue; }
        let rel = files::clean(rel)?;
        let out = run.join(rel);
        if let Some(p) = out.parent() { std::fs::create_dir_all(p).ok(); }
        let mut w = std::fs::File::create(&out).map_err(|e| AppError::io("No se pudo escribir un archivo del modpack", e))?;
        std::io::copy(&mut e, &mut w).map_err(|e| AppError::io("No se pudo escribir un archivo del modpack", e))?;
        n += 1;
    }
    Ok(n)
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackReport { pub downloaded: usize, pub overrides: usize, pub skipped_client: usize, pub blocked: Vec<String>, pub warnings: Vec<String> }

/// Agrega el contenido del pack a un servidor ya instalado con su versión y loader.
pub async fn apply(id: &str, pack: &Path, mut progress: impl FnMut(&str, f64)) -> AppResult<PackReport> {
    let info = inspect(pack)?;
    let run = paths::server_run_dir(id);
    let mut rep = PackReport { warnings: info.warnings.clone(), ..Default::default() };
    let mut manifest = mods::load_manifest(id);
    if info.format == "mrpack" {
        let idx = zip_json(pack, "modrinth.index.json")?;
        let fs = idx["files"].as_array().cloned().unwrap_or_default();
        let total = fs.len().max(1) as f64;
        for (i, f) in fs.iter().enumerate() {
            if f["env"]["server"].as_str() == Some("unsupported") { rep.skipped_client += 1; continue; }
            let rel = files::clean(f["path"].as_str().unwrap_or_default())?;
            let url = f["downloads"].as_array().and_then(|d| d.iter().filter_map(|u| u.as_str()).find(|u| allowed(u))).ok_or_else(|| AppError::invalid(format!("«{}» no tiene una descarga permitida.", rel.display())))?;
            let sha1 = f["hashes"]["sha1"].as_str().ok_or_else(|| AppError::invalid("Un archivo del modpack no trae SHA-1."))?;
            progress(&format!("Descargando {}", rel.file_name().unwrap_or_default().to_string_lossy()), i as f64 / total);
            net::download(url, &run.join(&rel), net::Hash::Sha1(sha1), |_, _| {}).await?;
            rep.downloaded += 1;
            if rel.starts_with("mods") {
                let file_name = rel.file_name().unwrap().to_string_lossy().to_string();
                manifest.mods.retain(|m| m.file_name != file_name);
                manifest.mods.push(mods::InstalledMod { project_id: format!("pack:{file_name}"), version_id: String::new(), title: file_name.trim_end_matches(".jar").into(), version_number: String::new(),
                    file_name, sha1: sha1.into(), side: "unknown".into(), explicit: true, required_by: vec![], depends_on: vec![], origin: Some("modpack".into()) });
            }
        }
        rep.overrides = extract_dir(pack, "overrides/", &run)? + extract_dir(pack, "server-overrides/", &run)?;
    } else {
        let m = zip_json(pack, "manifest.json")?;
        let fs = m["files"].as_array().cloned().unwrap_or_default();
        let total = fs.len().max(1) as f64;
        let dir = mods::mods_dir(id);
        std::fs::create_dir_all(&dir).ok();
        for (i, f) in fs.iter().enumerate() {
            let (pid, fid) = (f["projectID"].as_u64().unwrap_or(0), f["fileID"].as_u64().unwrap_or(0));
            let cf = crate::curseforge::file(pid, fid).await?;
            progress(&format!("Descargando {}", cf.file_name), i as f64 / total);
            let Some(url) = cf.download_url.clone() else { rep.blocked.push(cf.display_name.clone()); continue };
            net::download(&url, &dir.join(&cf.file_name), cf.sha1().map(net::Hash::Sha1).unwrap_or(net::Hash::None), |_, _| {}).await?;
            rep.downloaded += 1;
            manifest.mods.retain(|x| x.project_id != crate::curseforge::manifest_id(pid));
            manifest.mods.push(mods::InstalledMod { project_id: crate::curseforge::manifest_id(pid), version_id: fid.to_string(), title: cf.display_name.clone(), version_number: cf.display_name.clone(),
                file_name: cf.file_name.clone(), sha1: cf.sha1().unwrap_or_default().into(), side: "unknown".into(), explicit: true, required_by: vec![], depends_on: vec![], origin: Some("modpack".into()) });
        }
        let ov = m["overrides"].as_str().unwrap_or("overrides");
        rep.overrides = extract_dir(pack, &format!("{ov}/"), &run)?;
        if !rep.blocked.is_empty() { rep.warnings.push(format!("{} archivos no permiten descarga desde otras apps: bajalos desde CurseForge y agregalos en Archivos → mods.", rep.blocked.len())); }
    }
    mods::save_manifest(id, &manifest)?;
    progress("Listo", 1.0);
    Ok(rep)
}

/* El pack se aplica cuando termina la instalación del servidor. */
fn pending_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("modpack-pendiente.mrpack") }
pub fn set_pending(id: &str, pack: &Path) -> AppResult<()> {
    std::fs::copy(pack, pending_path(id)).map_err(|e| AppError::io("No se pudo guardar el modpack", e))?;
    Ok(())
}
pub fn take_pending(id: &str) -> Option<std::path::PathBuf> { let p = pending_path(id); p.exists().then_some(p) }

/// Descarga el `.mrpack` de una versión de un modpack de Modrinth (archivo principal).
pub async fn download_modrinth(version_id: &str) -> AppResult<std::path::PathBuf> {
    let v: crate::modrinth::Version = net::get_json(&format!("{}/version/{version_id}", crate::modrinth::API), "Modrinth").await?;
    let f = v.files.iter().find(|f| f.filename.ends_with(".mrpack")).ok_or_else(|| AppError::invalid("Esa versión no trae un .mrpack."))?;
    let dst = paths::cache().join("modpacks").join(&f.filename);
    net::download(&f.url, &dst, net::Hash::Sha1(&f.hashes.sha1), |_, _| {}).await?;
    Ok(dst)
}

#[cfg(test)]
mod t {
    use super::*;
    use std::io::Write;
    fn zip(path: &Path, files: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, c) in files { z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(c.as_bytes()).unwrap(); }
        z.finish().unwrap();
    }
    #[test]
    fn inspects_both_formats() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.mrpack");
        zip(&a, &[("modrinth.index.json", r#"{"formatVersion":1,"game":"minecraft","versionId":"1.0","name":"Pack","dependencies":{"minecraft":"1.21.1","fabric-loader":"0.16.5"},
            "files":[{"path":"mods/a.jar","hashes":{"sha1":"x"},"env":{"client":"required","server":"required"},"downloads":["https://cdn.modrinth.com/a.jar"]},
                     {"path":"mods/b.jar","hashes":{"sha1":"y"},"env":{"client":"required","server":"unsupported"},"downloads":["https://cdn.modrinth.com/b.jar"]}]}"#)]);
        let i = inspect(&a).unwrap();
        assert_eq!((i.format.as_str(), i.mc_version.as_str(), i.loader, i.server_files, i.client_only), ("mrpack", "1.21.1", Loader::Fabric, 1, 1));
        let b = d.path().join("b.zip");
        zip(&b, &[("manifest.json", r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.2.0","primary":true}]},"name":"CF Pack","version":"2","files":[{"projectID":1,"fileID":2,"required":true}],"overrides":"overrides"}"#)]);
        let j = inspect(&b).unwrap();
        assert_eq!((j.format.as_str(), j.loader, j.loader_version.as_deref()), ("curseforge", Loader::Forge, Some("47.2.0")));
        let c = d.path().join("c.zip");
        zip(&c, &[("x.txt", "hola")]);
        assert!(inspect(&c).is_err());
    }
    #[test]
    fn overrides_cannot_escape() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("evil.mrpack");
        zip(&a, &[("overrides/config/ok.toml", "a=1"), ("overrides/../../fuera.txt", "x")]);
        let run = d.path().join("run");
        std::fs::create_dir_all(&run).unwrap();
        assert!(extract_dir(&a, "overrides/", &run).is_err());
        assert!(!d.path().join("fuera.txt").exists());
    }
}
