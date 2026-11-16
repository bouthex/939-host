//! Administrador de archivos de un servidor (local; el remoto usa estas mismas funciones en la
//! PC que lo aloja).
//!
//! Seguridad:
//! - Toda ruta es relativa a la carpeta del servidor y se resuelve sin salir de ella (sin `..`,
//!   rutas absolutas, unidades ni enlaces que apunten afuera).
//! - Con el servidor encendido, Minecraft tiene abierto el mundo y escribe las regiones en
//!   cualquier momento (guardado automático cada 5 minutos y al descargar chunks). Copiar o
//!   modificar esos archivos a la vez puede dejar regiones a medio escribir. Por eso, mientras
//!   está encendido, el mundo es de solo lectura y no se descarga (para una copia consistente
//!   está «Copias», que pausa el guardado con save-off/save-all). `session.lock` nunca se toca.
//! - No se sobrescribe nada sin pedirlo; lo que se reemplaza o se borra va a la Papelera de 939
//!   Host (30 días).
//! - Las escrituras se hacen en un temporal y se renombran al final: si se corta, no queda un
//!   archivo a medias con el nombre real.

use crate::error::{AppError, AppResult};
use crate::{paths, trash};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub modified: String,
    /// world | mods | config | datapacks | resourcepacks | plugins | logs | file
    pub kind: String,
    /// Protegido mientras el servidor está encendido.
    pub locked: bool,
}

/// Carpetas que son parte de un mundo de Minecraft (la de `level-name` y sus dimensiones).
pub fn world_dirs(id: &str) -> Vec<String> {
    let run = paths::server_run_dir(id);
    let level = crate::properties::read(&run.join("server.properties")).get("level-name").cloned().unwrap_or_else(|| "world".into());
    let mut v = vec![level.clone(), format!("{level}_nether"), format!("{level}_the_end")];
    for e in std::fs::read_dir(&run).into_iter().flatten().flatten() {
        if e.path().join("level.dat").exists() { let n = e.file_name().to_string_lossy().to_string(); if !v.contains(&n) { v.push(n); } }
    }
    v
}

/// Normaliza una ruta relativa («mods/a.jar», «» = raíz). Rechaza todo lo que salga de la carpeta.
pub fn clean(rel: &str) -> AppResult<PathBuf> {
    let rel = rel.replace('\\', "/");
    let mut out = PathBuf::new();
    for c in Path::new(rel.trim_matches('/')).components() {
        match c {
            Component::Normal(p) => {
                let s = p.to_string_lossy();
                if s.contains(':') || s.trim().is_empty() || s.trim_end() != s || s.ends_with('.') { return Err(AppError::invalid("Nombre de archivo inválido.")); }
                out.push(p)
            }
            Component::CurDir => {}
            _ => return Err(AppError::invalid("La ruta tiene que estar dentro de la carpeta del servidor.")),
        }
    }
    Ok(out)
}

fn abs(id: &str, rel: &str) -> AppResult<PathBuf> {
    let root = paths::server_run_dir(id);
    let p = root.join(clean(rel)?);
    // Enlaces: lo que exista tiene que resolver dentro de la carpeta del servidor.
    if let (Ok(r), Ok(c)) = (root.canonicalize(), p.canonicalize()) { if !c.starts_with(&r) { return Err(AppError::invalid("La ruta apunta fuera de la carpeta del servidor.")); } }
    Ok(p)
}

fn top(rel: &Path) -> String { rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default() }

fn kind_of(id: &str, rel: &Path, dir: bool) -> String {
    let t = top(rel);
    if world_dirs(id).contains(&t) { return if rel.components().any(|c| c.as_os_str() == "datapacks") { "datapacks".into() } else { "world".into() }; }
    match t.as_str() {
        "mods" | "config" | "resourcepacks" | "plugins" | "logs" | "datapacks" => t,
        _ if !dir && rel.extension().map(|e| e == "jar").unwrap_or(false) => "jar".into(),
        _ => "file".into(),
    }
}

/// ¿Está protegida esa ruta ahora? `active` = el servidor está encendido o arrancando.
pub fn protected(id: &str, rel: &Path, active: bool) -> Option<&'static str> {
    if rel.file_name().map(|n| n == "session.lock").unwrap_or(false) { return Some("Minecraft usa ese archivo para bloquear el mundo: no se toca."); }
    if rel.as_os_str().is_empty() { return Some("La carpeta del servidor no se puede modificar entera."); }
    if active && world_dirs(id).contains(&top(rel)) { return Some("El servidor está encendido y está escribiendo el mundo. Detenelo para modificar o descargar archivos del mundo (o usá Copias para una copia en caliente)."); }
    None
}

fn modified(m: &std::fs::Metadata) -> String {
    m.modified().ok().map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339()).unwrap_or_default()
}

pub fn list(id: &str, rel: &str, active: bool) -> AppResult<Vec<Entry>> {
    let dir = abs(id, rel)?;
    let base = clean(rel)?;
    let mut v = vec![];
    for e in std::fs::read_dir(&dir).map_err(|e| AppError::io("No se pudo abrir la carpeta", e))?.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        let r = base.join(&name);
        let size = if m.is_dir() { 0 } else { m.len() };
        v.push(Entry { kind: kind_of(id, &r, m.is_dir()), locked: protected(id, &r, active).is_some(), path: r.to_string_lossy().replace('\\', "/"), name, dir: m.is_dir(), size, modified: modified(&m) });
    }
    v.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(v)
}

/// Busca por nombre en todo el servidor (hasta `limit` resultados).
pub fn search(id: &str, q: &str, active: bool, limit: usize) -> AppResult<Vec<Entry>> {
    let q = q.trim().to_lowercase();
    if q.len() < 2 { return Ok(vec![]); }
    let root = paths::server_run_dir(id);
    let mut out = vec![];
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            let p = e.path();
            let rel = p.strip_prefix(&root).unwrap_or(&p).to_path_buf();
            // Las regiones del mundo son miles de archivos sin interés para buscar.
            if m.is_dir() && rel.file_name().map(|n| n == "region" || n == "entities" || n == "poi").unwrap_or(false) { continue; }
            let name = e.file_name().to_string_lossy().to_string();
            if name.to_lowercase().contains(&q) {
                out.push(Entry { kind: kind_of(id, &rel, m.is_dir()), locked: protected(id, &rel, active).is_some(), path: rel.to_string_lossy().replace('\\', "/"), name, dir: m.is_dir(), size: if m.is_dir() { 0 } else { m.len() }, modified: modified(&m) });
                if out.len() >= limit { return Ok(out); }
            }
            if m.is_dir() { stack.push(p); }
        }
    }
    Ok(out)
}

fn check_write(id: &str, rel: &Path, active: bool) -> AppResult<()> {
    if let Some(why) = protected(id, rel, active) { return Err(AppError::busy(why)); }
    Ok(())
}

pub fn mkdir(id: &str, rel: &str, active: bool) -> AppResult<()> {
    let r = clean(rel)?;
    check_write(id, &r, active)?;
    let p = abs(id, rel)?;
    if p.exists() { return Err(AppError::invalid("Ya existe algo con ese nombre.")); }
    std::fs::create_dir_all(&p).map_err(|e| AppError::io("No se pudo crear la carpeta", e))
}

/// Manda un archivo o carpeta a la Papelera de 939 Host (se puede restaurar 30 días).
pub fn delete(id: &str, rel: &str, active: bool) -> AppResult<()> {
    let r = clean(rel)?;
    check_write(id, &r, active)?;
    let p = abs(id, rel)?;
    if !p.exists() { return Err(AppError::not_found("ese archivo")); }
    to_trash(id, &r, &p)
}

fn to_trash(id: &str, rel: &Path, p: &Path) -> AppResult<()> {
    let slot = paths::trash_slot(&format!("{id}-archivo"));
    std::fs::create_dir_all(slot.join("contenido")).map_err(|e| AppError::io("No se pudo usar la Papelera", e))?;
    let name = p.file_name().unwrap().to_owned();
    paths::rename_retry(p, slot.join("contenido").join(&name)).map_err(|e| AppError::io("No se pudo mover a la Papelera (¿está en uso?)", e))?;
    let cfg = crate::instance::load(id).ok();
    trash::stamp(&slot, &trash::TrashMeta {
        kind: "archivos".into(), original_id: id.into(), name: rel.to_string_lossy().replace('\\', "/"), deleted_at: chrono::Local::now().to_rfc3339(),
        mc_version: cfg.as_ref().map(|c| c.name.clone()).unwrap_or_default(), loader: String::new(), logo: None,
    })
}

fn copy_any(from: &Path, to: &Path) -> AppResult<u64> {
    if from.is_dir() { paths::copy_dir(from, to) } else {
        if let Some(p) = to.parent() { std::fs::create_dir_all(p).ok(); }
        let tmp = to.with_file_name(format!(".939-{}.tmp", to.file_name().unwrap().to_string_lossy()));
        let n = std::fs::copy(from, &tmp).map_err(|e| AppError::io("No se pudo copiar", e))?;
        paths::rename_retry(&tmp, to).map_err(|e| AppError::io("No se pudo terminar la copia", e))?;
        Ok(n)
    }
}

/// Copia o mueve dentro del servidor. `replace`: si el destino existe, lo manda a la Papelera.
pub fn transfer(id: &str, from: &str, to: &str, mv: bool, replace: bool, active: bool) -> AppResult<()> {
    let (rf, rt) = (clean(from)?, clean(to)?);
    if mv { check_write(id, &rf, active)?; } else if let Some(why) = protected(id, &rf, active) { return Err(AppError::busy(why)); }
    check_write(id, &rt, active)?;
    if rt.starts_with(&rf) { return Err(AppError::invalid("No se puede copiar o mover una carpeta dentro de sí misma.")); }
    let (pf, pt) = (abs(id, from)?, abs(id, to)?);
    if !pf.exists() { return Err(AppError::not_found("el origen")); }
    if pt.exists() {
        if !replace { return Err(AppError::new("exists", "Ya existe un archivo con ese nombre en el destino.")); }
        to_trash(id, &rt, &pt)?;
    }
    if mv { if let Some(p) = pt.parent() { std::fs::create_dir_all(p).ok(); } paths::rename_retry(&pf, &pt).map_err(|e| AppError::io("No se pudo mover (¿está en uso?)", e)) }
    else { copy_any(&pf, &pt).map(|_| ()) }
}

/// Trae archivos o carpetas de esta PC al servidor (`dest` = carpeta relativa).
pub fn import(id: &str, sources: &[PathBuf], dest: &str, replace: bool, active: bool) -> AppResult<Vec<String>> {
    let rd = clean(dest)?;
    let mut done = vec![];
    for s in sources {
        let name = s.file_name().ok_or_else(|| AppError::invalid("Origen inválido"))?.to_string_lossy().to_string();
        let rel = rd.join(&name);
        check_write(id, &rel, active)?;
        let p = abs(id, &rel.to_string_lossy())?;
        if p.exists() {
            if !replace { return Err(AppError::new("exists", format!("«{name}» ya existe en esa carpeta."))); }
            to_trash(id, &rel, &p)?;
        }
        copy_any(s, &p)?;
        done.push(rel.to_string_lossy().replace('\\', "/"));
    }
    Ok(done)
}

/// Lleva un archivo o carpeta del servidor a una carpeta de esta PC (sin sobrescribir).
pub fn export(id: &str, rel: &str, dest_dir: &Path, active: bool) -> AppResult<PathBuf> {
    let r = clean(rel)?;
    if let Some(why) = protected(id, &r, active) { return Err(AppError::busy(why)); }
    let p = abs(id, rel)?;
    let name = p.file_name().ok_or_else(|| AppError::invalid("Elegí un archivo o carpeta"))?.to_string_lossy().to_string();
    let mut out = dest_dir.join(&name);
    let mut n = 2;
    while out.exists() {
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.clone());
        let ext = Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        out = dest_dir.join(format!("{stem} ({n}){ext}")); n += 1;
    }
    copy_any(&p, &out)?;
    Ok(out)
}

/// Para validar un `.jar` antes de ponerlo en `mods/`: qué mod es y si sirve para este loader.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JarCheck { pub ok: bool, pub name: String, pub loader: Option<String>, pub message: String }

pub fn check_jar(cfg: &crate::model::ServerConfig, path: &Path) -> JarCheck {
    let f = std::fs::File::open(path);
    let Ok(Ok(mut z)) = f.map(zip::ZipArchive::new) else { return JarCheck { ok: false, name: String::new(), loader: None, message: "No es un .jar válido (está dañado o no es un archivo de Java).".into() } };
    let has = |z: &mut zip::ZipArchive<std::fs::File>, n: &str| z.by_name(n).is_ok();
    let loader = if has(&mut z, "fabric.mod.json") { Some("fabric") } else if has(&mut z, "META-INF/neoforge.mods.toml") { Some("neoforge") } else if has(&mut z, "META-INF/mods.toml") { Some("forge") } else if has(&mut z, "plugin.yml") || has(&mut z, "paper-plugin.yml") { Some("plugin") } else { None };
    let name = crate::mods::jar_meta(path).map(|m| m.name).unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy().to_string());
    let want = crate::mods::loader_id(cfg.loader).ok();
    let (ok, message) = match (loader, want) {
        (None, _) => (false, "No parece un mod: no tiene la información de Fabric, Forge ni NeoForge.".to_string()),
        (Some("plugin"), _) => (false, "Es un plugin (Paper/Spigot). Este servidor usa mods, no plugins.".to_string()),
        (Some(l), Some(w)) if l == w => (true, format!("Mod de {} compatible con el loader del servidor.", cfg.loader.name())),
        (Some("forge"), Some("neoforge")) => (true, "Mod de Forge: algunos funcionan en NeoForge, otros no. Si el servidor no arranca, quitalo.".to_string()),
        (Some(l), Some(_)) => (false, format!("Es un mod para {l}, pero este servidor usa {}.", cfg.loader.name())),
        (Some(_), None) => (false, "Los servidores Vanilla no cargan mods.".to_string()),
    };
    JarCheck { ok, name, loader: loader.map(String::from), message }
}

/* Transferencias en partes (para la administración remota). */

/// Lee un trozo de un archivo (para mandarlo a otra PC).
pub fn read_chunk(id: &str, rel: &str, offset: u64, len: usize, active: bool) -> AppResult<(Vec<u8>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let r = clean(rel)?;
    if let Some(why) = protected(id, &r, active) { return Err(AppError::busy(why)); }
    let p = abs(id, rel)?;
    let mut f = std::fs::File::open(&p).map_err(|e| AppError::io("No se pudo abrir el archivo", e))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    f.seek(SeekFrom::Start(offset)).map_err(|e| AppError::io("No se pudo leer", e))?;
    let mut buf = vec![0u8; len.min(total.saturating_sub(offset) as usize)];
    f.read_exact(&mut buf).map_err(|e| AppError::io("No se pudo leer", e))?;
    Ok((buf, total))
}

/// Archivo temporal para recibir una subida; se confirma con `commit_upload`.
pub fn upload_temp(id: &str, rel: &str, active: bool) -> AppResult<PathBuf> {
    let r = clean(rel)?;
    check_write(id, &r, active)?;
    let p = abs(id, rel)?;
    if let Some(parent) = p.parent() { std::fs::create_dir_all(parent).map_err(|e| AppError::io("No se pudo crear la carpeta", e))?; }
    Ok(p.with_file_name(format!(".939-subida-{}.tmp", p.file_name().unwrap().to_string_lossy())))
}

pub fn commit_upload(id: &str, rel: &str, tmp: &Path, replace: bool, active: bool) -> AppResult<()> {
    let r = clean(rel)?;
    check_write(id, &r, active)?;
    let p = abs(id, rel)?;
    if p.exists() {
        if !replace { let _ = std::fs::remove_file(tmp); return Err(AppError::new("exists", "Ya existe un archivo con ese nombre.")); }
        to_trash(id, &r, &p)?;
    }
    paths::rename_retry(tmp, &p).map_err(|e| AppError::io("No se pudo guardar el archivo", e))
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn paths_stay_inside() {
        assert_eq!(clean("mods/a.jar").unwrap(), PathBuf::from("mods").join("a.jar"));
        assert_eq!(clean("").unwrap(), PathBuf::new());
        assert!(clean("../x").is_err());
        assert!(clean("mods/../../x").is_err());
        assert!(clean("/etc/passwd").is_err() || clean("/etc/passwd").unwrap() == PathBuf::from("etc").join("passwd"));
        assert!(clean("C:/Windows").is_err());
        assert!(clean("mods\\..\\..\\x").is_err());
        assert!(clean("nombre. ").is_err());
    }
}
