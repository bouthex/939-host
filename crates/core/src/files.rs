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

/// Nombres que Windows reserva para dispositivos (con cualquier extensión): `CON`, `NUL.txt`…
fn reserved_name(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && matches!(stem.as_bytes()[3], b'0'..=b'9' | 0xB9 | 0xB2 | 0xB3))
}

/// Normaliza una ruta relativa («mods/a.jar», «» = raíz). Rechaza todo lo que salga de la carpeta
/// y lo que Windows interpreta de forma especial (unidades, flujos alternativos `a:b`, nombres de
/// dispositivo, caracteres prohibidos, puntos o espacios al final).
pub fn clean(rel: &str) -> AppResult<PathBuf> {
    let rel = rel.replace('\\', "/");
    if rel.len() > 1024 || rel.starts_with("//") { return Err(AppError::invalid("Ruta inválida.")); }
    let mut out = PathBuf::new();
    for c in Path::new(rel.trim_matches('/')).components() {
        match c {
            Component::Normal(p) => {
                let s = p.to_string_lossy();
                if s.contains(':') || s.trim().is_empty() || s.trim_end() != s || s.ends_with('.') || s.len() > 255
                    || s.chars().any(|ch| ch.is_control() || matches!(ch, '<' | '>' | '"' | '|' | '?' | '*')) || reserved_name(&s) {
                    return Err(AppError::invalid("Nombre de archivo inválido."));
                }
                out.push(p)
            }
            Component::CurDir => {}
            _ => return Err(AppError::invalid("La ruta tiene que estar dentro de la carpeta del servidor.")),
        }
    }
    Ok(out)
}

/// Ruta real dentro del servidor, como la ve el sistema de archivos: sigue la parte que existe con
/// `canonicalize` (en Windows resuelve mayúsculas/minúsculas y nombres cortos 8.3 como
/// `WORLD_~1`), rechaza enlaces simbólicos, junctions y otros puntos de reanálisis en el camino, y
/// verifica que todo quede dentro de la carpeta del servidor. Devuelve la ruta relativa real.
fn resolve(id: &str, rel: &str) -> AppResult<PathBuf> {
    let r = clean(rel)?;
    let root = paths::server_run_dir(id);
    let Ok(root_c) = root.canonicalize() else { return Ok(r) };
    let mut cur = root.clone();
    let mut existing = PathBuf::new();
    let mut tail = PathBuf::new();
    for c in r.components() {
        if !tail.as_os_str().is_empty() { tail.push(c); continue; }
        let next = cur.join(c);
        match std::fs::symlink_metadata(&next) {
            Ok(m) => {
                if m.file_type().is_symlink() || is_reparse(&m) { return Err(AppError::invalid("La ruta pasa por un enlace o junction: no se sigue.")); }
                cur = next;
                existing.push(c);
            }
            Err(_) => tail.push(c),
        }
    }
    let canon = cur.canonicalize().map_err(|e| AppError::io("No se pudo resolver la ruta", e))?;
    let inside = canon.strip_prefix(&root_c).map_err(|_| AppError::invalid("La ruta apunta fuera de la carpeta del servidor."))?;
    let _ = existing;
    let mut out = inside.to_path_buf();
    for c in tail.components() { out.push(c); }
    Ok(out)
}

#[cfg(windows)]
fn is_reparse(m: &std::fs::Metadata) -> bool { use std::os::windows::fs::MetadataExt; m.file_attributes() & 0x400 != 0 }
#[cfg(not(windows))]
fn is_reparse(_: &std::fs::Metadata) -> bool { false }

fn abs(id: &str, rel: &str) -> AppResult<PathBuf> {
    Ok(paths::server_run_dir(id).join(resolve(id, rel)?))
}

/// Ruta final de un archivo ya abierto (para comprobar, después de abrirlo, que nadie cambió una
/// carpeta por un enlace entre la verificación y la apertura).
#[cfg(windows)]
fn final_path(f: &std::fs::File) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 1024];
    // SAFETY: el handle es válido mientras `f` vive y el buffer tiene el tamaño indicado.
    let n = unsafe { GetFinalPathNameByHandleW(f.as_raw_handle() as _, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
    if n == 0 || n >= buf.len() { return None; }
    let s = std::ffi::OsString::from_wide(&buf[..n]);
    let s = s.to_string_lossy();
    Some(PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)))
}
#[cfg(unix)]
fn final_path(f: &std::fs::File) -> Option<PathBuf> { use std::os::unix::io::AsRawFd; std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())).ok() }
#[cfg(not(any(windows, unix)))]
fn final_path(_: &std::fs::File) -> Option<PathBuf> { None }

fn opened_inside(id: &str, f: &std::fs::File) -> AppResult<()> {
    let root = paths::server_run_dir(id).canonicalize().ok();
    match (final_path(f), root) {
        (Some(p), Some(r)) => {
            let norm = |x: &Path| x.to_string_lossy().to_lowercase().replace('\\', "/").trim_start_matches("//?/").to_string();
            if norm(&p).starts_with(&norm(&r)) { Ok(()) } else { Err(AppError::invalid("El archivo apunta fuera de la carpeta del servidor.")) }
        }
        _ => Ok(()),
    }
}

/// Límite de una subida: hasta 8 GB y dejando al menos 2 GB libres en el disco.
pub fn check_upload_size(id: &str, size: u64) -> AppResult<()> {
    const MAX: u64 = 8 * 1024 * 1024 * 1024;
    if size > MAX { return Err(AppError::invalid("El archivo es demasiado grande (máximo 8 GB por archivo).")); }
    let dir = paths::server_run_dir(id);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let free = disks.list().iter().filter(|d| dir.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()).map(|d| d.available_space());
    if let Some(free) = free { if size.saturating_add(2 * 1024 * 1024 * 1024) > free { return Err(AppError::invalid("No hay espacio suficiente en el disco de la PC anfitriona.")); } }
    Ok(())
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
/// Se compara sin distinguir mayúsculas (NTFS no las distingue: `WORLD` es `world`) y sobre la
/// ruta real (`resolve`), así un nombre corto 8.3 tampoco sirve para esquivar la protección.
pub fn protected(id: &str, rel: &Path, active: bool) -> Option<&'static str> {
    let real = resolve(id, &rel.to_string_lossy()).unwrap_or_else(|_| rel.to_path_buf());
    for r in [rel, real.as_path()] {
        if r.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("session.lock")).unwrap_or(false) { return Some("Minecraft usa ese archivo para bloquear el mundo: no se toca."); }
        if r.as_os_str().is_empty() { return Some("La carpeta del servidor no se puede modificar entera."); }
        let t = top(r).to_lowercase();
        if active && world_dirs(id).iter().any(|w| w.to_lowercase() == t) { return Some("El servidor está encendido y está escribiendo el mundo. Detenelo para modificar o descargar archivos del mundo (o usá Copias para una copia en caliente)."); }
    }
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

/// Lo que define qué programa arranca la PC anfitriona: el servidor, el lanzador de Fabric, las
/// librerías y los argumentos de Java de Forge/NeoForge (`libraries/…/win_args.txt`), y la
/// cuarentena de mods. Quien escribe acá decide qué código se ejecuta, sin pasar por la
/// aprobación de mods: desde otra PC no se puede crear, reemplazar, mover ni borrar.
const RUNTIME_DIRS: &[&str] = &["libraries", "versions", ".fabric", "mods-cuarentena"];
const RUNTIME_FILES: &[&str] = &["server.jar", "fabric-server-launch.jar", "fabric-server-launcher.properties", "run.bat", "run.sh", "user_jvm_args.txt"];

/// ¿Puede otra PC del grupo escribir en `rel`? (además de las reglas de `check_write`).
pub fn remote_write_check(id: &str, rel: &str) -> AppResult<()> {
    let r = clean(rel)?;
    let real = resolve(id, rel).unwrap_or_else(|_| r.clone());
    for p in [r.as_path(), real.as_path()] {
        let t = top(p).to_lowercase();
        let single = p.components().count() == 1;
        if RUNTIME_DIRS.contains(&t.as_str()) || (single && RUNTIME_FILES.contains(&t.as_str())) {
            return Err(AppError::new("denied", "Esa parte del servidor define qué programa se ejecuta: solo se cambia desde la PC anfitriona (o reinstalando el servidor)."));
        }
    }
    Ok(())
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
    opened_inside(id, &f)?;
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
    let p = match abs(id, rel) { Ok(p) => p, Err(e) => { let _ = std::fs::remove_file(tmp); return Err(e); } };
    if p.exists() {
        if !replace { let _ = std::fs::remove_file(tmp); return Err(AppError::new("exists", "Ya existe un archivo con ese nombre.")); }
        to_trash(id, &r, &p)?;
    }
    paths::rename_retry(tmp, &p).map_err(|e| { let _ = std::fs::remove_file(tmp); AppError::io("No se pudo guardar el archivo", e) })
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn windows_special_names_are_rejected() {
        for bad in ["CON", "con.txt", "mods/NUL", "Aux.json", "COM1", "lpt9.log", "a<b", "a|b", "a?b", "a*b", "a\"b", "x\u{1}y", "world/level.dat:stream", "mods/a.jar::$DATA", "//servidor/compartido", "\\\\?\\C:\\x"] {
            assert!(clean(bad).is_err(), "{bad} debía rechazarse");
        }
        for ok in ["console.txt", "mods/connector.jar", "COM10.txt", "config/aux-mod.toml"] { assert!(clean(ok).is_ok(), "{ok} es válido"); }
    }

    #[test]
    fn world_protection_ignores_case_and_follows_real_paths() {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-files-{}", std::process::id())));
        let id = &format!("caso-mayus-{}", std::process::id());
        let run = paths::server_run_dir(id);
        std::fs::create_dir_all(run.join("world").join("region")).unwrap();
        std::fs::write(run.join("world").join("level.dat"), b"x").unwrap();
        std::fs::create_dir_all(run.join("mods")).unwrap();
        for p in ["world/level.dat", "WORLD/level.dat", "World/region", "WoRlD", "world/session.lock", "mods/SESSION.LOCK"] {
            assert!(protected(id, &clean(p).unwrap(), true).is_some(), "{p} debía estar protegido con el servidor encendido");
        }
        assert!(protected(id, &clean("mods/a.jar").unwrap(), true).is_none());
        assert!(delete(id, "WORLD/level.dat", true).is_err(), "borrar con otra capitalización");
        assert!(transfer(id, "World/level.dat", "mods/copia.dat", false, false, true).is_err(), "copiar el mundo encendido");
        assert!(read_chunk(id, "WORLD/level.dat", 0, 10, true).is_err(), "descargar el mundo encendido");
        assert!(upload_temp(id, "wOrLd/region/r.0.0.mca", true).is_err(), "subir al mundo encendido");
        // Apagado: se puede.
        std::fs::write(run.join("world").join("level.dat"), b"x").unwrap();
        let r = read_chunk(id, "world/level.dat", 0, 1, false); assert!(r.is_ok(), "{:?}", r.err().map(|e| e.message));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let _ = paths::init(std::env::temp_dir().join(format!("h939-files-{}", std::process::id())));
        let id = &format!("enlaces-{}", std::process::id());
        let run = paths::server_run_dir(id);
        std::fs::create_dir_all(run.join("config")).unwrap();
        let outside = tmp.path().join("afuera");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secreto.txt"), b"privado").unwrap();
        std::os::unix::fs::symlink(&outside, run.join("config").join("atajo")).unwrap();
        std::os::unix::fs::symlink(outside.join("secreto.txt"), run.join("config").join("s.txt")).unwrap();
        assert!(read_chunk(id, "config/atajo/secreto.txt", 0, 7, false).is_err(), "carpeta enlazada afuera");
        assert!(read_chunk(id, "config/s.txt", 0, 7, false).is_err(), "archivo enlazado afuera");
        assert!(upload_temp(id, "config/atajo/nuevo.txt", false).is_err(), "escribir a través del enlace");
        assert!(mkdir(id, "config/atajo/x", false).is_err());
        assert!(delete(id, "config/atajo", false).is_err() || !outside.join("secreto.txt").exists() == false, "borrar el enlace no toca lo de afuera");
        assert!(outside.join("secreto.txt").exists());
    }

    /// Windows real: junction hacia afuera y nombre corto 8.3 de la carpeta del mundo.
    #[cfg(windows)]
    #[test]
    fn windows_junctions_and_short_names() {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-files-{}", std::process::id())));
        let id = &format!("win-{}", std::process::id());
        let run = paths::server_run_dir(id);
        std::fs::create_dir_all(run.join("world_nether").join("DIM-1")).unwrap();
        std::fs::write(run.join("world_nether").join("level.dat"), b"x").unwrap();
        std::fs::create_dir_all(run.join("config")).unwrap();
        let outside = std::env::temp_dir().join(format!("h939-afuera-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secreto.txt"), b"privado").unwrap();
        let j = run.join("config").join("atajo");
        let ok = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&j).arg(&outside).status().map(|s| s.success()).unwrap_or(false);
        assert!(ok, "no se pudo crear la junction de prueba");
        assert!(read_chunk(id, "config/atajo/secreto.txt", 0, 7, false).is_err(), "lectura a través de una junction");
        assert!(upload_temp(id, "config/atajo/x.txt", false).is_err(), "escritura a través de una junction");
        assert!(list(id, "config/atajo", false).is_err(), "listar a través de una junction");
        // Nombre corto 8.3 (si el volumen los tiene activados).
        let out = std::process::Command::new("cmd").args(["/C", "dir", "/X", "/AD"]).current_dir(&run).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let short = text.lines().find(|l| l.trim_end().ends_with("world_nether")).and_then(|l| l.split_whitespace().rev().nth(1)).filter(|s| s.contains('~')).map(String::from);
        match short {
            Some(sn) => {
                println!("nombre corto de world_nether: {sn}");
                assert!(protected(id, &clean(&format!("{sn}/level.dat")).unwrap(), true).is_some(), "8.3 esquivó la protección");
                assert!(delete(id, &format!("{sn}/level.dat"), true).is_err());
            }
            None => println!("el volumen no tiene nombres cortos 8.3: no aplica"),
        }
        let _ = std::fs::remove_dir(&j);
    }

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
