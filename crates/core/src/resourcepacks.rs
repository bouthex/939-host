//! Resource packs de un servidor: biblioteca, compatibilidad y entrega real a los jugadores.
//!
//! Los packs viven en la carpeta `resourcepacks` del servidor (la misma que se ve en Archivos), así
//! viajan con el servidor a las demás PCs del grupo. Cada pack es:
//!
//! - **Obligatorio** (uno solo: `server.properties` admite un único pack del servidor): Minecraft se
//!   lo envía a cada jugador al entrar (`resource-pack`, `resource-pack-sha1`,
//!   `require-resource-pack=true`). Quien lo rechaza no entra: lo aplica el propio Minecraft. La
//!   dirección es la descarga oficial de Modrinth si el pack viene de ahí; si es un archivo propio,
//!   `http://127.0.0.1:39393/rp/<servidor>/<sha1>.zip`, que sirve el 939 Host de cada jugador (lo
//!   trae de la PC anfitriona por la conexión cifrada del grupo y lo guarda en caché por su SHA-1).
//!   Así el archivo nunca se publica en internet. Jugadores que entran por IP sin 939 Host no pueden
//!   descargar un pack propio (sí uno de Modrinth).
//! - **Opcional**: no se exige al entrar. «Preparar para jugar» lo instala y lo activa en el
//!   Minecraft de cada jugador; cada uno lo puede desactivar.
//!
//! El SHA-1 va en la dirección y en `resource-pack-sha1`: si el pack cambia, la dirección cambia y
//! cada jugador baja la versión nueva; si no cambió, Minecraft usa la que ya tiene.

use crate::error::{AppError, AppResult};
use crate::{net, paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const LOCAL_PORT: u16 = 39393;
const META: &str = ".939host-packs.json";
/// Límite de Minecraft para un pack enviado por el servidor (250 MB desde 1.20.2; antes 100 MB).
pub const MAX_SERVER_PACK: u64 = 250 * 1024 * 1024;
const KEYS: [&str; 4] = ["resource-pack", "resource-pack-sha1", "require-resource-pack", "resource-pack-prompt"];

pub fn dir(id: &str) -> PathBuf { paths::server_run_dir(id).join("resourcepacks") }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PackMeta {
    /// "modrinth" | "propio"
    pub source: String,
    pub project_id: String,
    pub version_id: String,
    pub version_name: String,
    pub title: String,
    pub icon_url: String,
    /// Descarga oficial (solo Modrinth).
    pub url: String,
    pub game_versions: Vec<String>,
    /// "obligatorio" | "opcional"
    pub mode: String,
    pub active: bool,
    pub sha1: String,
    pub size: u64,
    pub mtime: i64,
    pub added_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Meta {
    pub packs: BTreeMap<String, PackMeta>,
    /// Valores de `server.properties` antes de que 939 Host administrara el pack del servidor (se
    /// vuelven a poner al quitar el pack obligatorio).
    pub previous: Option<BTreeMap<String, String>>,
}

fn meta_path(id: &str) -> PathBuf { dir(id).join(META) }
pub fn load_meta(id: &str) -> Meta { paths::read_json(&meta_path(id)).unwrap_or_default() }
fn save_meta(id: &str, m: &Meta) -> AppResult<()> {
    std::fs::create_dir_all(dir(id)).map_err(|e| AppError::io("No se pudo crear la carpeta resourcepacks", e))?;
    paths::write_json(&meta_path(id), m)
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    pub description: String,
    /// Formato principal declarado en pack.mcmeta.
    pub format: Option<u32>,
    /// Rango de formatos que dice soportar.
    pub range: Option<(u32, u32)>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackView {
    pub file: String,
    pub title: String,
    pub description: String,
    pub source: String,
    pub mode: String,
    pub active: bool,
    pub size: u64,
    pub sha1: String,
    pub icon: Option<String>,
    pub version_name: String,
    /// "ok" | "incompatible" | "desconocida"
    pub compat: String,
    pub compat_detail: String,
    /// "servidor" (lo envía Minecraft al entrar) | "jugadores" (Preparar para jugar) | "ninguno"
    pub delivery: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub packs: Vec<PackView>,
    /// Formato de resource packs de la versión del servidor (leído del propio servidor).
    pub server_format: Option<u32>,
    pub mc_version: String,
    /// `server.properties` tiene un pack puesto a mano (no por 939 Host).
    pub manual_pack: Option<String>,
    pub folder_exists: bool,
}

/// Texto plano de una descripción de Minecraft (texto, componente o lista de componentes).
fn flat_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.iter().map(flat_text).collect(),
        serde_json::Value::Object(o) => {
            let mut s = o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if let Some(e) = o.get("extra") { s.push_str(&flat_text(e)); }
            s
        }
        _ => String::new(),
    }
    .chars().filter(|c| !matches!(c, '§')).collect()
}

fn major(v: &serde_json::Value) -> Option<u32> {
    v.as_u64().map(|n| n as u32).or_else(|| v.as_array().and_then(|a| a.first()).and_then(|n| n.as_u64()).map(|n| n as u32))
}

/// Lee y valida un resource pack (.zip con `pack.mcmeta` en la raíz).
pub fn inspect(path: &Path) -> AppResult<PackInfo> {
    let f = std::fs::File::open(path).map_err(|e| AppError::io("No se pudo abrir el archivo", e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|_| AppError::invalid("No es un resource pack: el archivo no es un .zip válido."))?;
    let mut text = String::new();
    let found = match z.by_name("pack.mcmeta") {
        Ok(e) => { e.take(64 * 1024).read_to_string(&mut text).map_err(|_| AppError::invalid("El pack.mcmeta del resource pack está dañado."))?; true }
        Err(_) => false,
    };
    if !found {
        let nested = (0..z.len()).filter_map(|i| z.by_index(i).ok().and_then(|e| e.name().ok().map(|n| n.to_string()))).any(|n| n.ends_with("/pack.mcmeta"));
        return Err(AppError::invalid(if nested {
            "El pack.mcmeta está dentro de una carpeta del .zip. Minecraft lo necesita en la raíz: comprimí el contenido de la carpeta, no la carpeta."
        } else {
            "No es un resource pack: falta pack.mcmeta (los mods van en Mods y los datapacks en Datapacks)."
        }));
    }
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| AppError::invalid("El pack.mcmeta del resource pack no es JSON válido."))?;
    let pack = v.get("pack").ok_or_else(|| AppError::invalid("El pack.mcmeta no tiene la sección «pack»."))?;
    let format = pack.get("pack_format").and_then(major).or_else(|| pack.get("min_format").and_then(major));
    let range = (|| {
        if let Some(sf) = pack.get("supported_formats") {
            if let Some(n) = sf.as_u64() { return Some((n as u32, n as u32)); }
            if let Some(a) = sf.as_array() { return Some((a.first()?.as_u64()? as u32, a.get(1)?.as_u64()? as u32)); }
            return Some((sf.get("min_inclusive")?.as_u64()? as u32, sf.get("max_inclusive")?.as_u64()? as u32));
        }
        if let (Some(a), Some(b)) = (pack.get("min_format").and_then(major), pack.get("max_format").and_then(major)) { return Some((a, b)); }
        format.map(|f| (f, f))
    })();
    let mut icon = None;
    if let Ok(mut e) = z.by_name("pack.png") {
        if e.size() <= 96 * 1024 {
            let mut b = vec![];
            if e.read_to_end(&mut b).is_ok() && b.starts_with(b"\x89PNG") {
                use base64::Engine;
                icon = Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&b)));
            }
        }
    }
    Ok(PackInfo { description: pack.get("description").map(flat_text).unwrap_or_default(), format, range, icon })
}

/// Formato de resource packs de la versión del servidor, leído del `version.json` del jar de
/// Minecraft que usa el servidor (vanilla, Fabric, Forge o NeoForge).
pub fn server_format(id: &str) -> Option<u32> {
    let run = paths::server_run_dir(id);
    let mut jars = vec![run.join("server.jar")];
    let mut stack = vec![run.join("libraries/net/minecraft/server"), run.join(".fabric/server")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); } else if p.extension().map(|x| x == "jar").unwrap_or(false) { jars.push(p); }
        }
    }
    for j in jars {
        let Ok(f) = std::fs::File::open(&j) else { continue };
        let Ok(mut z) = zip::ZipArchive::new(f) else { continue };
        let Ok(mut e) = z.by_name("version.json") else { continue };
        let mut s = String::new();
        if e.read_to_string(&mut s).is_err() { continue; }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else { continue };
        let pv = &v["pack_version"];
        if let Some(n) = pv.get("resource").and_then(|x| x.as_u64()).or_else(|| pv.get("resource_major").and_then(|x| x.as_u64())).or_else(|| pv.as_u64()) { return Some(n as u32); }
    }
    None
}

fn compat(info: &PackInfo, server: Option<u32>, mc: &str, meta: &PackMeta) -> (String, String) {
    if meta.source == "modrinth" && !meta.game_versions.is_empty() {
        return if meta.game_versions.iter().any(|g| g == mc) { ("ok".into(), format!("Hecho para Minecraft {mc}.")) }
            else { ("incompatible".into(), format!("Hecho para {}; el servidor usa {mc}. Minecraft lo marca como incompatible pero deja usarlo; puede verse mal.", short_versions(&meta.game_versions))) };
    }
    match (info.range, server) {
        (Some((lo, hi)), Some(s)) if s >= lo && s <= hi => ("ok".into(), format!("Formato {s}, el de Minecraft {mc}.")),
        (Some((lo, hi)), Some(s)) => ("incompatible".into(), format!("Hecho para el formato {}; Minecraft {mc} usa el {s}. Minecraft lo marca como incompatible pero deja usarlo; puede verse mal.", if lo == hi { lo.to_string() } else { format!("{lo}–{hi}") })),
        _ => ("desconocida".into(), "No se pudo comprobar la versión (el servidor todavía no está instalado o el pack no la declara).".into()),
    }
}

fn short_versions(v: &[String]) -> String {
    if v.len() <= 3 { v.join(", ") } else { format!("{} … {}", v[0], v[v.len() - 1]) }
}

fn mtime(m: &std::fs::Metadata) -> i64 { m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0) }

/// Actualiza tamaño, fecha y SHA-1 de cada pack (solo recalcula lo que cambió) y quita de los
/// datos los que ya no están en la carpeta.
fn refresh(id: &str, m: &mut Meta) -> bool {
    let mut changed = false;
    let mut seen = vec![];
    for e in std::fs::read_dir(dir(id)).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if !p.is_file() || !name.to_ascii_lowercase().ends_with(".zip") || name.starts_with('.') { continue; }
        let Ok(md) = p.metadata() else { continue };
        seen.push(name.clone());
        let pm = m.packs.entry(name.clone()).or_insert_with(|| { changed = true; PackMeta { source: "propio".into(), mode: "opcional".into(), active: true, added_at: chrono::Local::now().to_rfc3339(), ..Default::default() } });
        if pm.size != md.len() || pm.mtime != mtime(&md) || pm.sha1.len() != 40 {
            pm.sha1 = net::sha1_file(&p).unwrap_or_default();
            pm.size = md.len();
            pm.mtime = mtime(&md);
            changed = true;
        }
    }
    let before = m.packs.len();
    m.packs.retain(|k, _| seen.contains(k));
    changed || before != m.packs.len()
}

pub fn library(id: &str) -> AppResult<Library> {
    let cfg = crate::instance::load(id)?;
    let mut m = load_meta(id);
    if refresh(id, &mut m) && dir(id).is_dir() { let _ = save_meta(id, &m); }
    let fmt = server_format(id);
    let mut packs = vec![];
    for (file, pm) in &m.packs {
        let info = inspect(&dir(id).join(file));
        let (compat, detail, info) = match info {
            Ok(i) => { let (c, d) = compat(&i, fmt, &cfg.mc_version, pm); (c, d, i) }
            Err(e) => ("invalido".to_string(), e.message, PackInfo::default()),
        };
        let mut warnings = vec![];
        if pm.mode == "obligatorio" && pm.size > MAX_SERVER_PACK { warnings.push(format!("Pesa {} MB: Minecraft no acepta packs del servidor de más de 250 MB.", pm.size / 1_048_576)); }
        if pm.mode == "obligatorio" && pm.source != "modrinth" { warnings.push("Es un archivo propio: lo reciben quienes entran con 939 Host. Quien entra por IP sin 939 Host no puede bajarlo y no podría entrar.".into()); }
        let delivery = if !pm.active { "ninguno" } else if pm.mode == "obligatorio" { "servidor" } else { "jugadores" };
        packs.push(PackView {
            file: file.clone(), title: if pm.title.is_empty() { file.trim_end_matches(".zip").to_string() } else { pm.title.clone() },
            description: info.description.clone(), source: pm.source.clone(), mode: pm.mode.clone(), active: pm.active, size: pm.size, sha1: pm.sha1.clone(),
            icon: info.icon.clone().or_else(|| (!pm.icon_url.is_empty()).then(|| pm.icon_url.clone())), version_name: pm.version_name.clone(),
            compat, compat_detail: detail, delivery: delivery.into(), warnings,
        });
    }
    packs.sort_by(|a, b| (b.mode == "obligatorio").cmp(&(a.mode == "obligatorio")).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    let props = crate::properties::read(&paths::server_run_dir(id).join("server.properties"));
    let manual_pack = if m.previous.is_none() { props.get("resource-pack").filter(|v| !v.trim().is_empty()).cloned() } else { None };
    Ok(Library { packs, server_format: fmt, mc_version: cfg.mc_version, manual_pack, folder_exists: dir(id).is_dir() })
}

fn safe_name(name: &str) -> AppResult<String> {
    let n: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() || !n.to_ascii_lowercase().ends_with(".zip") { return Err(AppError::invalid("El resource pack tiene que ser un archivo .zip.")); }
    Ok(n)
}

/// Agrega un archivo propio. `on_conflict`: "" (error `exists` si ya hay uno con ese nombre),
/// "reemplazar" (el anterior va a la Papelera) o "ambos" (se guarda con otro nombre).
pub fn add_file(id: &str, src: &Path, on_conflict: &str) -> AppResult<String> {
    inspect(src)?;
    let name = safe_name(&src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())?;
    let d = dir(id);
    std::fs::create_dir_all(&d).map_err(|e| AppError::io("No se pudo crear la carpeta resourcepacks", e))?;
    let mut dest = d.join(&name);
    let mut m = load_meta(id);
    let mut keep: Option<PackMeta> = None;
    if dest.exists() {
        match on_conflict {
            "reemplazar" => {
                keep = m.packs.get(&name).cloned();
                crate::files::delete(id, &format!("resourcepacks/{name}"), false)?;
            }
            "ambos" => {
                let stem = name[..name.len() - 4].to_string();
                let mut i = 2;
                while dest.exists() { dest = d.join(format!("{stem} ({i}).zip")); i += 1; }
            }
            _ => return Err(AppError::new("exists", format!("Ya hay un resource pack llamado «{name}»."))),
        }
    }
    let final_name = dest.file_name().unwrap().to_string_lossy().to_string();
    let tmp = d.join(format!(".{final_name}.939host-copia"));
    std::fs::copy(src, &tmp).map_err(|e| AppError::io("No se pudo copiar el resource pack", e))?;
    if net::sha1_file(&tmp)? != net::sha1_file(src)? { let _ = std::fs::remove_file(&tmp); return Err(AppError::invalid("La copia del resource pack no quedó igual; no se agregó.")); }
    paths::rename_retry(&tmp, &dest).map_err(|e| AppError::io("No se pudo guardar el resource pack", e))?;
    // Al reemplazar se conservan el modo y si estaba activo; pasa a ser un archivo propio.
    let pm = PackMeta { source: "propio".into(), mode: keep.as_ref().map(|k| k.mode.clone()).unwrap_or_else(|| "opcional".into()), active: keep.as_ref().map(|k| k.active).unwrap_or(true), added_at: chrono::Local::now().to_rfc3339(), ..Default::default() };
    m.packs.insert(final_name.clone(), pm);
    refresh(id, &mut m);
    save_meta(id, &m)?;
    Ok(final_name)
}

/// Quita un pack (va a la Papelera de 939 Host, de donde se puede restaurar).
pub fn remove(id: &str, file: &str) -> AppResult<()> {
    let name = safe_name(file)?;
    crate::files::delete(id, &format!("resourcepacks/{name}"), false)?;
    let mut m = load_meta(id);
    m.packs.remove(&name);
    save_meta(id, &m)
}

/// Cambia modo y/o estado. Solo un pack puede ser obligatorio: el anterior pasa a opcional.
/// Devuelve el nombre del pack que dejó de ser obligatorio, si hubo uno.
pub fn set(id: &str, file: &str, mode: Option<&str>, active: Option<bool>) -> AppResult<Option<String>> {
    let mut m = load_meta(id);
    refresh(id, &mut m);
    if !m.packs.contains_key(file) { return Err(AppError::not_found("ese resource pack")); }
    let mut demoted = None;
    if let Some(md) = mode {
        if md != "obligatorio" && md != "opcional" { return Err(AppError::invalid("Modo desconocido.")); }
        if md == "obligatorio" {
            let pm = &m.packs[file];
            if pm.size > MAX_SERVER_PACK { return Err(AppError::invalid("Ese pack pesa más de 250 MB: Minecraft no lo acepta como pack del servidor. Dejalo como opcional.")); }
            for (k, other) in m.packs.iter_mut() { if k != file && other.mode == "obligatorio" { other.mode = "opcional".into(); demoted = Some(k.clone()); } }
        }
        m.packs.get_mut(file).unwrap().mode = md.into();
    }
    if let Some(a) = active { m.packs.get_mut(file).unwrap().active = a; }
    save_meta(id, &m)?;
    Ok(demoted)
}

/// Dirección con la que los jugadores bajan el pack obligatorio.
pub fn pack_url(id: &str, pm: &PackMeta) -> String {
    // La descarga de Modrinth sirve solo si el archivo sigue siendo exactamente ese (SHA-1 de Modrinth).
    if pm.source == "modrinth" && pm.url.starts_with("https://cdn.modrinth.com/") && pm.url.contains("/data/") { pm.url.clone() } else { format!("http://127.0.0.1:{LOCAL_PORT}/rp/{id}/{}.zip", pm.sha1) }
}

/// El pack obligatorio activo, si hay.
pub fn required(id: &str) -> Option<(String, PackMeta)> {
    let mut m = load_meta(id);
    refresh(id, &mut m);
    m.packs.into_iter().find(|(_, p)| p.active && p.mode == "obligatorio" && p.sha1.len() == 40)
}

/// Aplica el pack del servidor sobre las claves de `server.properties` (lo llama `properties::write_for`).
/// Con un pack obligatorio, 939 Host administra las cuatro claves; al quitarlo, vuelven los valores
/// que había antes.
pub fn overlay(id: &str, map: &mut BTreeMap<String, String>) {
    let m = load_meta(id);
    match required(id) {
        Some((file, pm)) => {
            if m.previous.is_none() {
                let prev: BTreeMap<String, String> = KEYS.iter().filter_map(|k| map.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
                let mut m2 = m.clone();
                m2.previous = Some(prev);
                let _ = save_meta(id, &m2);
            }
            let title = if pm.title.is_empty() { file.trim_end_matches(".zip").to_string() } else { pm.title.clone() };
            map.insert("resource-pack".into(), pack_url(id, &pm));
            map.insert("resource-pack-sha1".into(), pm.sha1.clone());
            map.insert("require-resource-pack".into(), "true".into());
            map.insert("resource-pack-prompt".into(), serde_json::json!({ "text": format!("Este servidor usa el resource pack «{title}». Es obligatorio para jugar.") }).to_string());
        }
        None => {
            if let Some(prev) = &m.previous {
                for k in KEYS { match prev.get(k) { Some(v) => { map.insert(k.into(), v.clone()); } None => { map.remove(k); } } }
                let mut m2 = m.clone();
                m2.previous = None;
                let _ = save_meta(id, &m2);
            }
        }
    }
}

/// Archivo local de un pack de este servidor con ese SHA-1 (para servirlo o mandarlo al grupo).
pub fn find_local(id: &str, sha1: &str) -> Option<PathBuf> {
    if !paths::valid_server_id(id) || sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
    let mut m = load_meta(id);
    if refresh(id, &mut m) && dir(id).is_dir() { let _ = save_meta(id, &m); }
    m.packs.iter().find(|(_, p)| p.sha1.eq_ignore_ascii_case(sha1)).map(|(f, _)| dir(id).join(f)).filter(|p| p.is_file())
}

/// Packs que «Preparar para jugar» instala en el Minecraft del jugador (los opcionales activos).
pub fn client_packs(id: &str) -> Vec<crate::clientsetup::PlanMod> {
    let mut m = load_meta(id);
    refresh(id, &mut m);
    m.packs.iter().filter(|(_, p)| p.active && p.mode == "opcional" && p.sha1.len() == 40).map(|(f, p)| crate::clientsetup::PlanMod {
        filename: f.clone(), title: if p.title.is_empty() { f.trim_end_matches(".zip").to_string() } else { p.title.clone() }, sha1: p.sha1.clone(), size: p.size,
        url: (p.source == "modrinth" && p.url.starts_with("https://cdn.modrinth.com/")).then(|| p.url.clone()),
    }).collect()
}

/* ───────── Modrinth ───────── */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub project_id: String,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub versions: Vec<String>,
    pub latest_version: Option<String>,
    pub compatible: bool,
}

/// Busca resource packs en Modrinth. `all`: también los que no declaran la versión del servidor.
pub async fn search(query: &str, mc: &str, all: bool, offset: u32) -> AppResult<(Vec<SearchHit>, u64)> {
    let mut facets = vec![vec!["project_type:resourcepack".to_string()]];
    if !all { facets.push(vec![format!("versions:{mc}")]); }
    let enc = |s: &str| s.bytes().map(|b| match b { b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(), _ => format!("%{b:02X}") }).collect::<String>();
    let url = format!("https://api.modrinth.com/v2/search?query={}&facets={}&limit=20&offset={offset}&index=relevance", enc(query), enc(&serde_json::to_string(&facets)?));
    let v: serde_json::Value = net::get_json(&url, "Modrinth").await?;
    let hits = v["hits"].as_array().cloned().unwrap_or_default().into_iter().map(|h| {
        let versions: Vec<String> = h["versions"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
        SearchHit {
            project_id: h["project_id"].as_str().unwrap_or("").into(), title: h["title"].as_str().unwrap_or("").into(), description: h["description"].as_str().unwrap_or("").into(),
            icon_url: h["icon_url"].as_str().filter(|u| u.starts_with("https://")).map(String::from), downloads: h["downloads"].as_u64().unwrap_or(0),
            latest_version: h["latest_version"].as_str().map(String::from), compatible: versions.iter().any(|x| x == mc), versions,
        }
    }).collect();
    Ok((hits, v["total_hits"].as_u64().unwrap_or(0)))
}

/// Instala un resource pack de Modrinth: la versión más nueva para la versión del servidor o, con
/// `allow_incompatible`, la más nueva de todas. Verifica el SHA-1 y el pack.mcmeta.
pub async fn install_modrinth(id: &str, project: &str, allow_incompatible: bool) -> AppResult<String> {
    let cfg = crate::instance::load(id)?;
    if project.is_empty() || !project.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { return Err(AppError::invalid("Proyecto inválido.")); }
    let p: serde_json::Value = net::get_json(&format!("https://api.modrinth.com/v2/project/{project}"), "Modrinth").await?;
    if p["project_type"].as_str() != Some("resourcepack") { return Err(AppError::invalid("Ese proyecto de Modrinth no es un resource pack.")); }
    let versions: Vec<serde_json::Value> = net::get_json(&format!("https://api.modrinth.com/v2/project/{project}/version"), "Modrinth").await?;
    let fits = |v: &serde_json::Value| v["game_versions"].as_array().into_iter().flatten().any(|g| g.as_str() == Some(cfg.mc_version.as_str()));
    let chosen = versions.iter().find(|v| fits(v)).or_else(|| if allow_incompatible { versions.first() } else { None });
    let Some(ver) = chosen else {
        let latest = versions.first().map(|v| v["game_versions"].as_array().into_iter().flatten().filter_map(|g| g.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        return Err(AppError::new("incompatible", format!("«{}» no tiene una versión para Minecraft {} (la última es para {}).", p["title"].as_str().unwrap_or(project), cfg.mc_version, latest)));
    };
    let files = ver["files"].as_array().cloned().unwrap_or_default();
    let file = files.iter().find(|f| f["primary"].as_bool() == Some(true)).or_else(|| files.first()).ok_or_else(|| AppError::invalid("Esa versión no tiene archivos."))?;
    let url = file["url"].as_str().filter(|u| u.starts_with("https://cdn.modrinth.com/")).ok_or_else(|| AppError::invalid("La descarga no es de Modrinth."))?;
    let sha1 = file["hashes"]["sha1"].as_str().ok_or_else(|| AppError::invalid("Modrinth no informó el SHA-1."))?;
    let mut name = safe_name(file["filename"].as_str().unwrap_or("pack.zip"))?;
    let d = dir(id);
    std::fs::create_dir_all(&d).map_err(|e| AppError::io("No se pudo crear la carpeta resourcepacks", e))?;
    let mut m = load_meta(id);
    refresh(id, &mut m);
    // Ya instalado igual: no se baja de nuevo.
    if let Some((f, _)) = m.packs.iter().find(|(_, pm)| pm.sha1.eq_ignore_ascii_case(sha1)) { return Ok(f.clone()); }
    // Otra versión del mismo pack de Modrinth: se reemplaza (la anterior va a la Papelera) y se
    // conservan el modo y si estaba activo.
    let old = m.packs.iter().find(|(_, pm)| pm.source == "modrinth" && pm.project_id == project).map(|(f, pm)| (f.clone(), pm.clone()));
    if d.join(&name).exists() && old.as_ref().map(|(f, _)| f != &name).unwrap_or(true) {
        let stem = name[..name.len() - 4].to_string();
        let mut i = 2;
        while d.join(&name).exists() { name = format!("{stem} ({i}).zip"); i += 1; }
    }
    let tmp = d.join(format!(".{name}.939host-descarga"));
    net::download(url, &tmp, net::Hash::Sha1(sha1), |_, _| {}).await?;
    if let Err(e) = inspect(&tmp) { let _ = std::fs::remove_file(&tmp); return Err(e); }
    if let Some((f, _)) = &old { crate::files::delete(id, &format!("resourcepacks/{f}"), false)?; m.packs.remove(f); }
    paths::rename_retry(&tmp, &d.join(&name)).map_err(|e| AppError::io("No se pudo guardar el resource pack", e))?;
    let gv: Vec<String> = ver["game_versions"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(String::from)).collect();
    m.packs.insert(name.clone(), PackMeta {
        source: "modrinth".into(), project_id: project.into(), version_id: ver["id"].as_str().unwrap_or("").into(), version_name: ver["version_number"].as_str().unwrap_or("").into(),
        title: p["title"].as_str().unwrap_or("").into(), icon_url: p["icon_url"].as_str().filter(|u| u.starts_with("https://")).unwrap_or("").into(), url: url.into(), game_versions: gv,
        mode: old.as_ref().map(|(_, o)| o.mode.clone()).unwrap_or_else(|| "opcional".into()), active: old.as_ref().map(|(_, o)| o.active).unwrap_or(true),
        added_at: chrono::Local::now().to_rfc3339(), ..Default::default()
    });
    refresh(id, &mut m);
    save_meta(id, &m)?;
    Ok(name)
}

/* ───────── Entrega local a Minecraft ───────── */

static SERVED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Pedidos atendidos (para las pruebas y el diagnóstico): «<servidor>/<sha1>».
pub fn served() -> Vec<String> { SERVED.lock().unwrap().clone() }

/// Caché local de packs traídos de otra PC del grupo.
pub fn cache_path(sha1: &str) -> PathBuf { paths::cache().join("resourcepacks").join(format!("{}.zip", sha1.to_ascii_lowercase())) }

/// Busca el archivo de un pedido: en los servidores de esta PC, en la caché o (con `fetch`) en la
/// PC anfitriona del grupo. Lo traído se verifica por SHA-1 antes de guardarlo en la caché.
pub type Fetch = std::sync::Arc<dyn Fn(String, String) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<PathBuf>> + Send>> + Send + Sync>;

/// Servidor HTTP mínimo, solo en 127.0.0.1, que entrega `GET /rp/<servidor>/<sha1>.zip` al
/// Minecraft de esta PC. No atiende nada más.
pub async fn serve(listener: tokio::net::TcpListener, fetch: Option<Fetch>) {
    loop {
        let Ok((mut sock, peer)) = listener.accept().await else { continue };
        if !peer.ip().is_loopback() { continue; }
        let fetch = fetch.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 8192];
            let mut n = 0;
            loop {
                let Ok(Ok(r)) = tokio::time::timeout(std::time::Duration::from_secs(10), sock.read(&mut buf[n..])).await else { return };
                if r == 0 { return; }
                n += r;
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") { break; }
                if n == buf.len() { return; }
            }
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let line = head.lines().next().unwrap_or("");
            let mut parts = line.split(' ');
            let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            let reply = |code: &str, body: &str| format!("HTTP/1.1 {code}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
            let ok_req = (method == "GET" || method == "HEAD") && seg.len() == 3 && seg[0] == "rp" && seg[2].len() == 44 && seg[2].ends_with(".zip");
            if !ok_req { let _ = sock.write_all(reply("404 Not Found", "no").as_bytes()).await; return; }
            let (server, sha1) = (seg[1].to_string(), seg[2][..40].to_ascii_lowercase());
            let file = match find_local(&server, &sha1) {
                Some(p) => Some(p),
                None => {
                    let c = cache_path(&sha1);
                    if c.is_file() && net::sha1_file(&c).map(|h| h == sha1).unwrap_or(false) { Some(c) }
                    else if let Some(f) = &fetch { f(server.clone(), sha1.clone()).await.ok() } else { None }
                }
            };
            let Some(file) = file else { let _ = sock.write_all(reply("404 Not Found", "Ese resource pack no está disponible.").as_bytes()).await; return; };
            let Ok(mut f) = tokio::fs::File::open(&file).await else { let _ = sock.write_all(reply("500 Internal Server Error", "error").as_bytes()).await; return; };
            let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
            let h = format!("HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {len}\r\nCache-Control: max-age=31536000, immutable\r\nConnection: close\r\n\r\n");
            if sock.write_all(h.as_bytes()).await.is_err() { return; }
            if method == "GET" {
                SERVED.lock().unwrap().push(format!("{server}/{sha1}"));
                let _ = tokio::io::copy(&mut f, &mut sock).await;
            }
            let _ = sock.shutdown().await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub fn make_pack(path: &Path, format: u32, desc: &str) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(format!(r#"{{"pack":{{"pack_format":{format},"description":{}}}}}"#, serde_json::json!(desc)).as_bytes()).unwrap();
        z.start_file("assets/minecraft/texts/splashes.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(desc.as_bytes()).unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn valida_resource_packs_y_explica_los_errores() {
        let t = tempfile::tempdir().unwrap();
        let ok = t.path().join("ok.zip");
        make_pack(&ok, 34, "Hola");
        let i = inspect(&ok).unwrap();
        assert_eq!((i.format, i.range, i.description.as_str()), (Some(34), Some((34, 34)), "Hola"));
        let nested = t.path().join("carpeta.zip");
        { let mut z = zip::ZipWriter::new(std::fs::File::create(&nested).unwrap()); z.start_file("MiPack/pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(b"{}").unwrap(); z.finish().unwrap(); }
        assert!(inspect(&nested).unwrap_err().message.contains("raíz"));
        std::fs::write(t.path().join("mod.zip"), b"PK no").unwrap();
        assert!(inspect(&t.path().join("mod.zip")).is_err());
        // Formatos nuevos (min_format / max_format) y rangos.
        let r = t.path().join("rango.zip");
        { let mut z = zip::ZipWriter::new(std::fs::File::create(&r).unwrap()); z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(br#"{"pack":{"min_format":[69,0],"max_format":75,"description":{"text":"A","extra":[{"text":"B"}]}}}"#).unwrap(); z.finish().unwrap(); }
        let i = inspect(&r).unwrap();
        assert_eq!((i.range, i.description.as_str()), (Some((69, 75)), "AB"));
        let (c, _) = compat(&i, Some(70), "26.2", &PackMeta::default());
        assert_eq!(c, "ok");
        let (c, d) = compat(&i, Some(34), "1.21.1", &PackMeta::default());
        assert_eq!(c, "incompatible");
        assert!(d.contains("69–75"));
        assert_eq!(compat(&i, None, "1.21.1", &PackMeta::default()).0, "desconocida");
    }
}
