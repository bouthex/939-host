//! CurseForge por su API oficial (https://docs.curseforge.com/rest-api). Sin scraping.
//!
//! - Requiere una clave (`x-api-key`) emitida por CurseForge a quien la solicita. 939 Host NO
//!   incluye ninguna clave en el ejecutable ni en el instalador: cada persona que quiera usar
//!   CurseForge desde la app carga la suya, que se guarda protegida con DPAPI en su PC y nunca
//!   se muestra ni se registra. Sin clave, todo lo demás funciona (y los modpacks descargados a
//!   mano desde la web de CurseForge se pueden importar igual).
//! - Los autores pueden no permitir la descarga desde apps de terceros: entonces la API no da
//!   `downloadUrl` y 939 Host no descarga ese archivo; ofrece abrir su página en CurseForge.
//! - Los archivos se verifican con el SHA-1 que publica CurseForge.

use crate::error::{AppError, AppResult};
use crate::model::{Loader, ServerConfig};
use crate::{mods, net, paths};
use serde::{Deserialize, Serialize};

pub const API: &str = "https://api.curseforge.com/v1";
pub const MINECRAFT: u32 = 432;
pub const CLASS_MODS: u32 = 6;
pub const CLASS_MODPACKS: u32 = 4471;

/// La clave de esta PC (protegida con DPAPI). No hay clave de compilación.
pub fn key() -> Option<String> { crate::settings::curseforge_key().filter(|k| k.len() > 20) }
pub fn available() -> bool { key().is_some() }

/// Nombre de archivo que viene de la API: tiene que ser un nombre simple (sin carpetas, `..`
/// ni nombres especiales de Windows), para que nunca se escriba fuera de la carpeta destino.
pub fn safe_name(name: &str) -> AppResult<&str> {
    let ok = !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != ".." && crate::files::clean(name).map(|p| p.components().count() == 1).unwrap_or(false);
    if ok { Ok(name) } else { Err(AppError::invalid("CurseForge devolvió un nombre de archivo inválido.")) }
}

/// Huella obligatoria: sin SHA-1 publicado por CurseForge no se descarga.
pub fn required_sha1(f: &CfFile) -> AppResult<net::Hash<'_>> {
    f.sha1().map(net::Hash::Sha1).ok_or_else(|| AppError::new("cf_hash", format!("CurseForge no publicó la huella de «{}»: no se descarga sin poder verificarlo.", f.file_name)))
}

pub fn loader_type(l: Loader) -> Option<u32> {
    match l { Loader::Forge => Some(1), Loader::Fabric => Some(4), Loader::NeoForge => Some(6), Loader::Vanilla => None }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Logo { #[serde(default)] pub thumbnail_url: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Links { #[serde(default)] pub website_url: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfMod {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: f64,
    #[serde(default)]
    pub date_modified: String,
    #[serde(default)]
    pub logo: Option<Logo>,
    #[serde(default)]
    pub links: Links,
    /// `false` = el autor no permite descargas desde apps de terceros.
    #[serde(default)]
    pub allow_mod_distribution: Option<bool>,
    #[serde(default)]
    pub class_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfHash { pub value: String, pub algo: u32 }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfDep { pub mod_id: u64, pub relation_type: u32 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfFile {
    pub id: u64,
    pub mod_id: u64,
    pub display_name: String,
    pub file_name: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub file_length: u64,
    #[serde(default)]
    pub hashes: Vec<CfHash>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<CfDep>,
    #[serde(default)]
    pub file_date: String,
    #[serde(default)]
    pub server_pack_file_id: Option<u64>,
    #[serde(default)]
    pub is_server_pack: Option<bool>,
    /// 1 = release, 2 = beta, 3 = alpha.
    #[serde(default)]
    pub release_type: u32,
}
impl CfFile {
    pub fn sha1(&self) -> Option<&str> { self.hashes.iter().find(|h| h.algo == 1).map(|h| h.value.as_str()) }
    /// Sirve para esta versión de Minecraft y loader (CurseForge mezcla ambos en `gameVersions`).
    pub fn fits(&self, mc: &str, loader: Loader) -> bool {
        let gv: Vec<String> = self.game_versions.iter().map(|g| g.to_lowercase()).collect();
        let lname = match loader { Loader::Fabric => "fabric", Loader::Forge => "forge", Loader::NeoForge => "neoforge", Loader::Vanilla => "" };
        let has_loader_tags = gv.iter().any(|g| ["fabric", "forge", "neoforge", "quilt"].contains(&g.as_str()));
        gv.iter().any(|g| g == mc) && (!has_loader_tags || gv.iter().any(|g| g == lname))
    }
}

#[derive(Deserialize)]
struct Data<T> { data: T }

async fn get<T: serde::de::DeserializeOwned>(path: &str) -> AppResult<T> {
    let k = key().ok_or_else(|| AppError::new("curseforge_key", "Falta la clave de la API de CurseForge."))?;
    let r = net::HTTP.get(format!("{API}{path}")).header("x-api-key", k).header("Accept", "application/json").send().await
        .map_err(|e| AppError::new("network", "No se pudo conectar con CurseForge").with_detail(e))?;
    match r.status().as_u16() {
        200 => {}
        403 => return Err(AppError::new("curseforge_key", "CurseForge rechazó la clave de la API (403). Revisá que esté bien copiada y aprobada.")),
        404 => return Err(AppError::not_found("eso en CurseForge")),
        s => return Err(AppError::new("network", format!("CurseForge respondió {s}"))),
    }
    let d: Data<T> = r.json().await.map_err(|e| AppError::new("network", "Respuesta inesperada de CurseForge").with_detail(e))?;
    Ok(d.data)
}

fn enc(s: &str) -> String { url_escape(s) }
fn url_escape(s: &str) -> String { s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect() }

/// Busca mods (`class` = CLASS_MODS) o modpacks (CLASS_MODPACKS). Orden: popularidad.
pub async fn search(query: &str, mc: Option<&str>, loader: Option<Loader>, class: u32, index: u32) -> AppResult<Vec<CfMod>> {
    let mut q = format!("/mods/search?gameId={MINECRAFT}&classId={class}&sortField=2&sortOrder=desc&pageSize=30&index={index}&searchFilter={}", enc(query));
    if let Some(v) = mc { q.push_str(&format!("&gameVersion={}", enc(v))); }
    if let Some(t) = loader.and_then(loader_type) { q.push_str(&format!("&modLoaderType={t}")); }
    get(&q).await
}
pub async fn project(id: u64) -> AppResult<CfMod> { get(&format!("/mods/{id}")).await }
pub async fn file(mod_id: u64, file_id: u64) -> AppResult<CfFile> { get(&format!("/mods/{mod_id}/files/{file_id}")).await }
pub async fn files(mod_id: u64, mc: &str, loader: Loader) -> AppResult<Vec<CfFile>> {
    let mut q = format!("/mods/{mod_id}/files?gameVersion={}&pageSize=50", enc(mc));
    if let Some(t) = loader_type(loader) { q.push_str(&format!("&modLoaderType={t}")); }
    get(&q).await
}

/// Todos los archivos de un proyecto (para modpacks, sin filtrar por versión).
pub async fn search_files_any(mod_id: u64) -> AppResult<Vec<CfFile>> { get(&format!("/mods/{mod_id}/files?pageSize=50")).await }

/// El archivo más nuevo que sirve (prefiere estables).
pub fn best(mut fs: Vec<CfFile>, mc: &str, loader: Loader) -> Option<CfFile> {
    fs.retain(|f| f.fits(mc, loader) && f.is_server_pack != Some(true));
    fs.sort_by(|a, b| a.release_type.cmp(&b.release_type).then(b.file_date.cmp(&a.file_date)));
    fs.into_iter().next()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfPlanItem { pub mod_id: u64, pub title: String, pub file: CfFile, pub reason: String, pub page: Option<String> }
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CfPlan { pub items: Vec<CfPlanItem>, pub blocked: Vec<CfPlanItem>, pub incompatible: Vec<String>, pub total_bytes: u64 }

pub fn manifest_id(mod_id: u64) -> String { format!("cf:{mod_id}") }

/// Plan con dependencias obligatorias (relationType 3). Los archivos que el autor no deja
/// distribuir quedan en `blocked` (no se descargan). Incompatibilidades declaradas (tipo 5) con
/// lo ya instalado, en `incompatible`.
pub async fn plan(cfg: &ServerConfig, mod_id: u64) -> AppResult<CfPlan> {
    mods::loader_id(cfg.loader)?;
    let installed: Vec<String> = mods::load_manifest(&cfg.id).mods.iter().map(|m| m.project_id.clone()).collect();
    let mut items = vec![]; let mut blocked = vec![]; let mut incompatible = vec![];
    let mut queue = vec![(mod_id, "Elegido por vos".to_string())];
    let mut seen = std::collections::HashSet::new();
    while let Some((id, reason)) = queue.pop() {
        if !seen.insert(id) { continue; }
        if id != mod_id && installed.contains(&manifest_id(id)) { continue; }
        let p = project(id).await?;
        let f = best(files(id, &cfg.mc_version, cfg.loader).await?, &cfg.mc_version, cfg.loader)
            .ok_or_else(|| AppError::invalid(format!("«{}» no tiene versión para {} {} en CurseForge.", p.name, cfg.loader.name(), cfg.mc_version)))?;
        for d in &f.dependencies {
            if d.relation_type == 3 { queue.push((d.mod_id, format!("Lo necesita {}", p.name))); }
            if d.relation_type == 5 && installed.contains(&manifest_id(d.mod_id)) { incompatible.push(format!("{} no es compatible con un mod que ya tenés instalado.", p.name)); }
        }
        let it = CfPlanItem { mod_id: id, title: p.name.clone(), reason, page: p.links.website_url.clone(), file: f };
        if it.file.download_url.is_none() || p.allow_mod_distribution == Some(false) { blocked.push(it); } else { items.push(it); }
    }
    let total_bytes = items.iter().map(|i| i.file.file_length).sum();
    Ok(CfPlan { items, blocked, incompatible, total_bytes })
}

/// Descarga verificada a `mods/` y registro en el gestor de mods (aparece en la lista normal).
pub async fn apply(cfg: &ServerConfig, plan: &CfPlan) -> AppResult<Vec<String>> {
    if let Some(i) = plan.incompatible.first() { return Err(AppError::invalid(i.clone())); }
    if !plan.blocked.is_empty() {
        return Err(AppError::new("cf_blocked", format!("El autor de {} no permite descargarlo desde otras apps. Bajalo desde su página de CurseForge y arrastralo a la carpeta mods en Archivos.", plan.blocked.iter().map(|b| b.title.clone()).collect::<Vec<_>>().join(", "))));
    }
    let dir = mods::mods_dir(&cfg.id);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::io("No se pudo crear la carpeta de mods", e))?;
    let mut manifest = mods::load_manifest(&cfg.id);
    let mut done = vec![];
    for it in &plan.items {
        let url = it.file.download_url.clone().unwrap();
        let fname = safe_name(&it.file.file_name)?;
        let staging = paths::cache().join("curseforge").join(format!("{}-{}", it.file.id, fname));
        let hash = required_sha1(&it.file)?;
        net::download(&url, &staging, hash, |_, _| {}).await?;
        if let Some(old) = manifest.mods.iter().find(|m| m.project_id == manifest_id(it.mod_id)) { let _ = std::fs::remove_file(dir.join(&old.file_name)); }
        std::fs::copy(&staging, dir.join(fname)).map_err(|e| AppError::io("No se pudo copiar el mod", e))?;
        manifest.mods.retain(|m| m.project_id != manifest_id(it.mod_id));
        manifest.mods.push(mods::InstalledMod {
            project_id: manifest_id(it.mod_id), version_id: it.file.id.to_string(), title: it.title.clone(), version_number: it.file.display_name.clone(),
            file_name: it.file.file_name.clone(), sha1: it.file.sha1().unwrap_or_default().to_string(), side: "unknown".into(),
            explicit: it.reason == "Elegido por vos", required_by: vec![], depends_on: vec![], origin: Some("curseforge".into()),
        });
        done.push(it.title.clone());
    }
    mods::save_manifest(&cfg.id, &manifest)?;
    Ok(done)
}

/// Mods de CurseForge con versión nueva: (manifest id, nombre de la versión nueva).
pub async fn updates(cfg: &ServerConfig) -> AppResult<Vec<(String, String)>> {
    let mut out = vec![];
    for m in mods::load_manifest(&cfg.id).mods.iter().filter(|m| m.project_id.starts_with("cf:")) {
        let id: u64 = m.project_id[3..].parse().unwrap_or(0);
        if let Some(f) = best(files(id, &cfg.mc_version, cfg.loader).await?, &cfg.mc_version, cfg.loader) {
            if f.id.to_string() != m.version_id { out.push((m.project_id.clone(), f.display_name)); }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod t {
    use super::*;
    fn f(gv: &[&str], rt: u32, date: &str) -> CfFile {
        CfFile { id: 1, mod_id: 1, display_name: "x".into(), file_name: "x.jar".into(), download_url: None, file_length: 0, hashes: vec![CfHash { value: "ab".into(), algo: 1 }],
            game_versions: gv.iter().map(|s| s.to_string()).collect(), dependencies: vec![], file_date: date.into(), server_pack_file_id: None, is_server_pack: None, release_type: rt }
    }
    #[test]
    fn version_and_loader_matching() {
        assert!(f(&["1.21.1", "Fabric"], 1, "").fits("1.21.1", Loader::Fabric));
        assert!(!f(&["1.21.1", "Forge"], 1, "").fits("1.21.1", Loader::Fabric));
        assert!(!f(&["1.20.1", "Fabric"], 1, "").fits("1.21.1", Loader::Fabric));
        assert!(f(&["1.21.1"], 1, "").fits("1.21.1", Loader::NeoForge), "sin etiqueta de loader = sirve para cualquiera");
        let b = best(vec![f(&["1.21.1", "Fabric"], 2, "2026-02"), f(&["1.21.1", "Fabric"], 1, "2026-01")], "1.21.1", Loader::Fabric).unwrap();
        assert_eq!(b.release_type, 1, "prefiere la estable aunque sea más vieja");
        assert_eq!(f(&[], 1, "").sha1(), Some("ab"));
    }
    #[test]
    fn parses_official_response_shape() {
        let j = r#"{"data":{"id":238222,"modId":238222,"displayName":"JEI 19.0","fileName":"jei.jar","downloadUrl":null,"fileLength":10,"hashes":[{"value":"aa","algo":1},{"value":"bb","algo":2}],"gameVersions":["1.21.1","NeoForge"],"dependencies":[{"modId":5,"relationType":3}],"fileDate":"2026-01-01T00:00:00Z","releaseType":1,"serverPackFileId":null}}"#;
        let d: Data<CfFile> = serde_json::from_str(j).unwrap();
        assert!(d.data.download_url.is_none());
        assert_eq!(d.data.dependencies[0].relation_type, 3);
        assert_eq!(d.data.sha1(), Some("aa"));
    }
}
