//! Reconexión automática en el Minecraft de cada jugador.
//!
//! Cuando el servidor cambia de PC, los jugadores se desconectan unos segundos. Para que vuelvan
//! a entrar solos, 939 Host instala en el perfil de Minecraft que el jugador elija el mod
//! «AutoReconnect (Reforged)» (Modrinth, LGPL-3.0) con sus dependencias obligatorias.
//!
//! Reglas:
//! * Solo se toca el perfil que el jugador eligió, y solo para AGREGAR archivos. Nunca se borra,
//!   reemplaza ni renombra un archivo que ya estaba.
//! * Lo que agrega 939 Host queda anotado en `.939host-cliente.json` dentro de esa carpeta de mods.
//! * Si alguien borra el mod por error, 939 Host lo vuelve a poner la próxima vez que se abre
//!   (mientras la opción siga activada). Se desactiva desde la app.
//! * Los perfiles se detectan leyendo `launcher_profiles.json` (solo lectura).

use crate::error::{AppError, AppResult};
use crate::modrinth::{self, DependencyType, Source, Version};
use crate::mods::jar_meta;
use crate::net;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Proyecto de Modrinth del mod de reconexión.
pub const RECONNECT_PROJECT: &str = "autoreconnectrf";
const MARKER: &str = ".939host-cliente.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientProfile {
    /// Nombre del perfil en el launcher (solo para mostrar).
    pub name: String,
    /// Carpeta del juego del perfil (la que contiene `mods`).
    pub game_dir: String,
    pub mc_version: String,
    /// "fabric" o "neoforge".
    pub loader: String,
}

impl ClientProfile {
    pub fn mods_dir(&self) -> PathBuf { Path::new(&self.game_dir).join("mods") }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    /// Archivos que agregó 939 Host (nombre → sha1).
    pub files: Vec<(String, String)>,
    pub project_versions: Vec<String>,
    pub updated_at: String,
}

fn read_marker(dir: &Path) -> Marker { paths::read_json(&dir.join(MARKER)).unwrap_or_default() }

/// Detecta perfiles Fabric/NeoForge del launcher oficial (solo lectura).
pub fn detect_profiles(minecraft_dir: &Path) -> Vec<ClientProfile> {
    #[derive(Deserialize)]
    struct Launcher { #[serde(default)] profiles: std::collections::HashMap<String, Prof> }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Prof { #[serde(default)] name: String, #[serde(default)] last_version_id: String, #[serde(default)] game_dir: Option<String> }
    let Ok(text) = std::fs::read_to_string(minecraft_dir.join("launcher_profiles.json")) else { return vec![] };
    let Ok(l) = serde_json::from_str::<Launcher>(&text) else { return vec![] };
    let mut out: Vec<ClientProfile> = l.profiles.into_values().filter_map(|p| {
        let (loader, mc) = parse_version_id(&p.last_version_id)?;
        let game_dir = p.game_dir.filter(|g| !g.trim().is_empty()).unwrap_or_else(|| minecraft_dir.to_string_lossy().to_string());
        Some(ClientProfile { name: if p.name.is_empty() { p.last_version_id.clone() } else { p.name }, game_dir, mc_version: mc, loader })
    }).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.game_dir == b.game_dir && a.mc_version == b.mc_version && a.loader == b.loader);
    out
}

/// "fabric-loader-0.20.1-26.2" → (fabric, 26.2); "neoforge-21.1.222" → (neoforge, 1.21.1).
pub fn parse_version_id(id: &str) -> Option<(String, String)> {
    if let Some(rest) = id.strip_prefix("fabric-loader-") {
        let mc = rest.splitn(2, '-').nth(1)?;
        return Some(("fabric".into(), mc.to_string()));
    }
    if let Some(v) = id.strip_prefix("neoforge-") {
        let parts: Vec<&str> = v.split(['.', '-']).collect();
        let (a, b) = (parts.first()?.parse::<u32>().ok()?, parts.get(1)?.parse::<u32>().ok()?);
        let mc = if a >= 26 {
            let c = parts.get(2).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
            if c == 0 { format!("{a}.{b}") } else { format!("{a}.{b}.{c}") }
        } else if b == 0 { format!("1.{a}") } else { format!("1.{a}.{b}") };
        return Some(("neoforge".into(), mc));
    }
    None
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureReport {
    pub added: Vec<String>,
    pub already: Vec<String>,
    pub message: String,
}

/// Mods ya presentes en la carpeta (por id del jar y por proyecto de Modrinth vía SHA-1).
async fn present_projects<S: Source>(src: &S, dir: &Path) -> (HashSet<String>, HashSet<String>) {
    let mut mod_ids = HashSet::new();
    let mut hashes = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "jar").unwrap_or(false) {
                if let Some(m) = jar_meta(&p) { mod_ids.insert(m.id); }
                if let Ok(h) = net::sha1_file(&p) { hashes.push(h); }
            }
        }
    }
    let projects = src.versions_by_sha1(&hashes).await.map(|m| m.into_values().map(|v| v.project_id).collect()).unwrap_or_default();
    (mod_ids, projects)
}

/// Deja instalado el mod de reconexión (y lo que necesita) en el perfil. Solo agrega archivos.
pub async fn ensure_reconnect<S: Source>(src: &S, profile: &ClientProfile) -> AppResult<EnsureReport> {
    let dir = profile.mods_dir();
    if !Path::new(&profile.game_dir).is_dir() {
        return Err(AppError::invalid("No existe la carpeta del perfil."));
    }
    let loader = profile.loader.as_str();
    let mc = profile.mc_version.as_str();
    let root = modrinth::best_version(src.project_versions(RECONNECT_PROJECT, mc, loader).await?, mc, loader)
        .ok_or_else(|| AppError::invalid(format!("Todavía no hay una versión de AutoReconnect para {} {mc}.", if loader == "fabric" { "Fabric" } else { "NeoForge" })))?;

    let (present_ids, present_projects) = present_projects(src, &dir).await;
    let mut to_add: Vec<Version> = vec![];
    let mut already = vec![];
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([root]);
    while let Some(v) = queue.pop_front() {
        if !seen.insert(v.project_id.clone()) { continue; }
        if present_projects.contains(&v.project_id) { already.push(v.name.clone()); continue; }
        for d in v.dependencies.iter().filter(|d| d.dependency_type == DependencyType::Required) {
            let dv = match (&d.version_id, &d.project_id) {
                (Some(vid), _) => src.version(vid).await?,
                (None, Some(pid)) => match modrinth::best_version(src.project_versions(pid, mc, loader).await?, mc, loader) {
                    Some(x) => x,
                    None => return Err(AppError::invalid(format!("Falta una dependencia para {mc}: {pid}."))),
                },
                _ => continue,
            };
            queue.push_back(dv);
        }
        to_add.push(v);
    }

    std::fs::create_dir_all(&dir).map_err(|e| AppError::io("No se pudo crear la carpeta de mods", e))?;
    let mut marker = read_marker(&dir);
    let mut added = vec![];
    for v in &to_add {
        let f = v.primary_file().ok_or_else(|| AppError::invalid("Versión sin archivo"))?;
        if !f.url.starts_with("https://cdn.modrinth.com/") || f.filename.contains(['/', '\\']) || !f.filename.ends_with(".jar") {
            return Err(AppError::invalid("Archivo de origen no confiable; se canceló."));
        }
        let target = dir.join(&f.filename);
        if target.exists() {
            // Hay un archivo con ese nombre: no se toca, aunque sea distinto.
            already.push(v.name.clone());
            continue;
        }
        // Un jar del mismo mod con otro nombre (instalado a mano) también cuenta como presente.
        let staging = paths::cache().join("mods").join(&f.hashes.sha1).with_extension("jar");
        if !staging.exists() || net::sha1_file(&staging).map(|h| h != f.hashes.sha1).unwrap_or(true) {
            net::download(&f.url, &staging, net::Hash::Sha1(&f.hashes.sha1), |_, _| {}).await?;
        }
        if let Some(m) = jar_meta(&staging) {
            if present_ids.contains(&m.id) { already.push(v.name.clone()); continue; }
        }
        let tmp = target.with_extension("jar.939tmp");
        std::fs::copy(&staging, &tmp).map_err(|e| AppError::io("No se pudo copiar el mod", e))?;
        std::fs::rename(&tmp, &target).map_err(|e| AppError::io("No se pudo copiar el mod", e))?;
        marker.files.retain(|(n, _)| n != &f.filename);
        marker.files.push((f.filename.clone(), f.hashes.sha1.clone()));
        added.push(v.name.clone());
    }
    marker.project_versions = to_add.iter().map(|v| format!("{}:{}", v.project_id, v.version_number)).collect();
    marker.updated_at = chrono::Local::now().to_rfc3339();
    paths::write_json(&dir.join(MARKER), &marker)?;
    let message = if added.is_empty() { "La reconexión automática ya estaba lista en este perfil.".into() } else { format!("Agregado: {}. Se activa la próxima vez que abras ese perfil.", added.join(", ")) };
    Ok(EnsureReport { added, already, message })
}

/// Quita solo lo que agregó 939 Host (y que nadie cambió desde entonces).
pub fn remove_reconnect(profile: &ClientProfile) -> AppResult<Vec<String>> {
    let dir = profile.mods_dir();
    let marker = read_marker(&dir);
    let mut removed = vec![];
    for (name, sha) in &marker.files {
        let p = dir.join(name);
        if p.exists() && net::sha1_file(&p).map(|h| &h == sha).unwrap_or(false) {
            std::fs::rename(&p, paths::trash_slot(&format!("cliente-{name}"))).map_err(|e| AppError::io("No se pudo quitar", e))?;
            removed.push(name.clone());
        }
    }
    let _ = std::fs::remove_file(dir.join(MARKER));
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modrinth::Project;
    use std::collections::HashMap;

    #[test]
    fn version_ids_map_to_minecraft_versions() {
        assert_eq!(parse_version_id("fabric-loader-0.20.1-26.2"), Some(("fabric".into(), "26.2".into())));
        assert_eq!(parse_version_id("fabric-loader-0.19.5-1.21.11"), Some(("fabric".into(), "1.21.11".into())));
        assert_eq!(parse_version_id("neoforge-21.1.222"), Some(("neoforge".into(), "1.21.1".into())));
        assert_eq!(parse_version_id("neoforge-26.1.0.4-beta"), Some(("neoforge".into(), "26.1".into())));
        assert_eq!(parse_version_id("1.21.8-forge-58.1.0"), None);
        assert_eq!(parse_version_id("26.2"), None);
    }

    #[test]
    fn detects_profiles_read_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("launcher_profiles.json"), r#"{"profiles":{
            "a":{"name":"Mi modpack","lastVersionId":"fabric-loader-0.20.1-26.2"},
            "b":{"name":"Vanilla","lastVersionId":"latest-release"},
            "c":{"name":"Neo","lastVersionId":"neoforge-21.1.222","gameDir":"C:\\Juegos\\neo"}}}"#).unwrap();
        let before = std::fs::read(d.path().join("launcher_profiles.json")).unwrap();
        let p = detect_profiles(d.path());
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "Mi modpack");
        assert_eq!(p[0].game_dir, d.path().to_string_lossy());
        assert_eq!(p[1].game_dir, "C:\\Juegos\\neo");
        assert_eq!(std::fs::read(d.path().join("launcher_profiles.json")).unwrap(), before, "no se modifica");
    }

    struct Fake(Vec<Version>);
    impl Source for Fake {
        async fn version(&self, id: &str) -> AppResult<Version> { self.0.iter().find(|v| v.id == id).cloned().ok_or_else(|| AppError::not_found(id)) }
        async fn project_versions(&self, p: &str, _: &str, _: &str) -> AppResult<Vec<Version>> { Ok(self.0.iter().filter(|v| v.project_id == p).cloned().collect()) }
        async fn projects(&self, _: &[String]) -> AppResult<Vec<Project>> { Ok(vec![]) }
        async fn versions_by_sha1(&self, h: &[String]) -> AppResult<HashMap<String, Version>> {
            Ok(self.0.iter().filter(|v| h.contains(&v.files[0].hashes.sha1)).map(|v| (v.files[0].hashes.sha1.clone(), v.clone())).collect())
        }
    }
    fn ver(id: &str, project: &str, file: &str, sha: &str, deps: Vec<&str>) -> Version {
        serde_json::from_value(serde_json::json!({
            "id": id, "project_id": project, "name": project, "version_number": "1", "version_type": "release",
            "game_versions": ["26.2"], "loaders": ["fabric"], "date_published": "2026-10-01",
            "dependencies": deps.iter().map(|d| serde_json::json!({ "project_id": d, "dependency_type": "required" })).collect::<Vec<_>>(),
            "files": [{ "url": format!("https://cdn.modrinth.com/{file}"), "filename": file, "primary": true, "size": 1, "hashes": { "sha1": sha, "sha512": "" } }]
        })).unwrap()
    }

    #[tokio::test]
    async fn only_adds_files_and_skips_what_is_present() {
        let root = tempfile::tempdir().unwrap();
        let _ = paths::init(std::env::temp_dir().join("939host-unit-tests"));
        // Archivos en caché simulando descargas ya verificadas.
        let jars: Vec<(&str, &[u8])> = vec![("ar.jar", b"autoreconnect"), ("yacl.jar", b"yacl"), ("api.jar", b"fabric-api")];
        let mut versions = vec![];
        for (name, data) in &jars {
            let sha = { use sha1::Digest; hex::encode(sha1::Sha1::digest(data)) };
            std::fs::create_dir_all(paths::cache().join("mods")).unwrap();
            std::fs::write(paths::cache().join("mods").join(format!("{sha}.jar")), data).unwrap();
            versions.push((name.to_string(), sha));
        }
        let src = Fake(vec![
            ver("v1", RECONNECT_PROJECT, "ar.jar", &versions[0].1, vec!["yacl", "fabric-api"]),
            ver("v2", "yacl", "yacl.jar", &versions[1].1, vec![]),
            ver("v3", "fabric-api", "api.jar", &versions[2].1, vec![]),
        ]);
        let game = root.path().join("perfil");
        std::fs::create_dir_all(game.join("mods")).unwrap();
        // El jugador ya tiene Fabric API (con otro nombre) y un archivo suyo cualquiera.
        std::fs::write(game.join("mods").join("fabric-api-0.161.jar"), b"fabric-api").unwrap();
        std::fs::write(game.join("mods").join("mio.jar"), b"del jugador").unwrap();
        let prof = ClientProfile { name: "x".into(), game_dir: game.to_string_lossy().into(), mc_version: "26.2".into(), loader: "fabric".into() };
        let r = ensure_reconnect(&src, &prof).await.unwrap();
        assert_eq!(r.added.len(), 2, "{r:?}");
        assert!(game.join("mods/ar.jar").exists() && game.join("mods/yacl.jar").exists());
        assert!(!game.join("mods/api.jar").exists(), "Fabric API ya estaba: no se duplica");
        assert_eq!(std::fs::read(game.join("mods/mio.jar")).unwrap(), b"del jugador");
        // Si se borra por error, vuelve.
        std::fs::remove_file(game.join("mods/ar.jar")).unwrap();
        let r = ensure_reconnect(&src, &prof).await.unwrap();
        assert_eq!(r.added, vec![RECONNECT_PROJECT.to_string()]);
        // Quitar solo saca lo agregado por 939 Host.
        let removed = remove_reconnect(&prof).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(game.join("mods/fabric-api-0.161.jar").exists() && game.join("mods/mio.jar").exists());
    }
}
