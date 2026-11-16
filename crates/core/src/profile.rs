//! Perfil para jugar: lo que necesita el Minecraft de cada jugador para entrar a un servidor con
//! mods, en formato `.mrpack` (formato abierto de Modrinth, documentado en
//! support.modrinth.com/en/articles/8802351). Lo importan Modrinth App, Prism Launcher,
//! ATLauncher y otros con doble clic.
//!
//! - Los mods de Modrinth se referencian por URL y hash (sha1 + sha512): el launcher los baja de
//!   Modrinth; 939 Host no redistribuye nada.
//! - Los mods que solo corren en el servidor no van (el cliente no los necesita).
//! - Los agregados a mano no se pueden verificar: solo se incluyen si el usuario lo pide, dentro
//!   de `overrides/`, con aviso de que redistribuirlos depende de su licencia.
//! - No toca la instalación de Minecraft de nadie: genera un archivo.

use crate::error::{AppError, AppResult};
use crate::model::Loader;
use crate::modrinth::Source;
use crate::{instance, mods, net};
use serde::Serialize;
use sha2::Digest;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub file: String,
    pub size_bytes: u64,
    pub from_modrinth: Vec<String>,
    pub manual_included: Vec<String>,
    pub manual_excluded: Vec<String>,
    pub server_only_skipped: Vec<String>,
}

fn loader_key(l: Loader) -> Option<&'static str> {
    match l { Loader::Fabric => Some("fabric-loader"), Loader::NeoForge => Some("neoforge"), Loader::Forge => Some("forge"), _ => None }
}

/// Dominios desde los que el formato permite descargas.
fn allowed(url: &str) -> bool {
    ["https://cdn.modrinth.com/", "https://github.com/", "https://raw.githubusercontent.com/", "https://gitlab.com/"].iter().any(|p| url.starts_with(p))
}

pub async fn export_mrpack<S: Source>(src: &S, id: &str, dest: &std::path::Path, include_manual: bool) -> AppResult<ExportReport> {
    let cfg = instance::load(id)?;
    let lk = loader_key(cfg.loader).ok_or_else(|| AppError::invalid("Este servidor no usa mods: para entrar alcanza con Minecraft sin modificar."))?;
    let lv = cfg.loader_version.clone().ok_or_else(|| AppError::invalid("Falta la versión del loader."))?;
    let dir = mods::mods_dir(id);
    let mut jars = vec![];
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("jar") { jars.push(p); }
    }
    jars.sort();
    let mut sha1s = vec![];
    for j in &jars { sha1s.push(net::sha1_file(j)?); }
    let found = if sha1s.is_empty() { Default::default() } else { src.versions_by_sha1(&sha1s).await.unwrap_or_default() };
    let pids: Vec<String> = found.values().map(|v| v.project_id.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let projects = src.projects(&pids).await.unwrap_or_default();
    let mut rep = ExportReport::default();
    let mut files = vec![];
    let mut overrides: Vec<(String, std::path::PathBuf)> = vec![];
    for (j, h) in jars.iter().zip(&sha1s) {
        let name = j.file_name().unwrap().to_string_lossy().to_string();
        match found.get(h).and_then(|v| v.files.iter().find(|f| &f.hashes.sha1 == h).map(|f| (v, f))) {
            Some((v, f)) if allowed(&f.url) => {
                let p = projects.iter().find(|p| p.id == v.project_id);
                let (cs, ss) = p.map(|p| (p.client_side.as_str(), p.server_side.as_str())).unwrap_or(("required", "required"));
                if cs == "unsupported" { rep.server_only_skipped.push(name); continue; }
                let sha512 = if f.hashes.sha512.is_empty() { hex::encode(sha2::Sha512::digest(std::fs::read(j).map_err(|e| AppError::io("No se pudo leer un mod", e))?)) } else { f.hashes.sha512.clone() };
                let env = |s: &str| if s == "unsupported" { "unsupported" } else if s == "optional" { "optional" } else { "required" };
                files.push(serde_json::json!({
                    "path": format!("mods/{}", f.filename), "hashes": { "sha1": h, "sha512": sha512 },
                    "env": { "client": env(cs), "server": env(ss) }, "downloads": [f.url], "fileSize": f.size,
                }));
                rep.from_modrinth.push(p.map(|p| p.title.clone()).unwrap_or(name));
            }
            _ => {
                if include_manual { overrides.push((format!("overrides/mods/{name}"), j.clone())); rep.manual_included.push(name); } else { rep.manual_excluded.push(name); }
            }
        }
    }
    let mut deps = serde_json::Map::new();
    deps.insert("minecraft".into(), cfg.mc_version.clone().into());
    deps.insert(lk.into(), lv.into());
    let index = serde_json::json!({
        "formatVersion": 1, "game": "minecraft", "versionId": format!("939host-{}", chrono::Local::now().format("%Y%m%d-%H%M")),
        "name": cfg.name, "summary": format!("Para entrar a «{}» ({} {}). Generado por 939 Host.", cfg.name, cfg.loader.name(), cfg.mc_version),
        "files": files, "dependencies": deps,
    });
    let tmp = dest.with_extension("mrpack.tmp");
    {
        let f = std::fs::File::create(&tmp).map_err(|e| AppError::io("No se pudo crear el archivo", e))?;
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("modrinth.index.json", o).map_err(|e| AppError::new("io", "No se pudo escribir el perfil").with_detail(e))?;
        z.write_all(serde_json::to_string_pretty(&index)?.as_bytes()).map_err(|e| AppError::io("No se pudo escribir el perfil", e))?;
        for (path, src) in &overrides {
            z.start_file(path.as_str(), o).map_err(|e| AppError::new("io", "No se pudo escribir el perfil").with_detail(e))?;
            let mut r = std::fs::File::open(src).map_err(|e| AppError::io("No se pudo leer un mod", e))?;
            std::io::copy(&mut r, &mut z).map_err(|e| AppError::io("No se pudo escribir el perfil", e))?;
        }
        z.finish().map_err(|e| AppError::new("io", "No se pudo cerrar el perfil").with_detail(e))?;
    }
    crate::paths::rename_retry(&tmp, dest).map_err(|e| AppError::io("No se pudo guardar el perfil", e))?;
    rep.file = dest.display().to_string();
    rep.size_bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    Ok(rep)
}

/// Launchers que importan `.mrpack`, detectados por su carpeta de datos (solo se mira si existe).
pub fn launchers() -> Vec<&'static str> {
    let Some(appdata) = std::env::var_os("APPDATA").map(std::path::PathBuf::from) else { return vec![] };
    let mut v = vec![];
    if appdata.join("ModrinthApp").is_dir() || appdata.join("com.modrinth.theseus").is_dir() { v.push("Modrinth App"); }
    if appdata.join("PrismLauncher").is_dir() { v.push("Prism Launcher"); }
    if appdata.join("ATLauncher").is_dir() { v.push("ATLauncher"); }
    v
}
