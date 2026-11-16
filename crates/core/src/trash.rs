//! Papelera de 939 Host: servidores borrados que se conservan 30 días.
//!
//! Al borrar un servidor, su carpeta completa (mundo, mods, configuración, copias, puntos de
//! restauración, grupo y permiso) se mueve a `Papelera/` con un archivo `.939-papelera.json` que
//! guarda la fecha real de borrado. El plazo se cuenta desde esa fecha, no desde la fecha de la
//! carpeta ni del último acceso, y sobrevive a reinicios. La clave de identidad del servidor
//! (`identity/servers/<id>.key`) no se toca, así que al restaurar conserva su identidad (salvo que
//! otro servidor ocupe su lugar: entonces la copia restaurada recibe una identidad propia).

use crate::error::{AppError, AppResult};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RETENTION_DAYS: i64 = 30;
const META: &str = ".939-papelera.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashMeta {
    pub kind: String,          // "server" | "world" | "mods" | "otro"
    pub original_id: String,
    pub name: String,
    pub deleted_at: String,    // RFC 3339
    #[serde(default)]
    pub mc_version: String,
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub logo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    /// Nombre de la carpeta dentro de la Papelera (identifica el elemento).
    pub key: String,
    pub meta: TrashMeta,
    pub size_bytes: u64,
    pub days_left: i64,
    /// Ya existe un servidor con ese id o nombre: al restaurar se crea con otro id.
    pub conflict: bool,
}

/// Registra el borrado dentro de la carpeta movida (se llama justo después de moverla).
pub fn stamp(dir: &Path, meta: &TrashMeta) -> AppResult<()> {
    paths::write_json(&dir.join(META), meta)
}

fn read_meta(dir: &Path) -> Option<TrashMeta> {
    if let Ok(m) = paths::read_json::<TrashMeta>(&dir.join(META)) { return Some(m); }
    // Servidores borrados antes de que existiera el registro: se reconstruye con lo que hay.
    let cfg: crate::model::ServerConfig = paths::read_json(&dir.join("instance.json")).ok()?;
    let when = std::fs::metadata(dir).ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339()).unwrap_or_default();
    Some(TrashMeta { kind: "server".into(), original_id: cfg.id.clone(), name: cfg.name.clone(), deleted_at: when, mc_version: cfg.mc_version.clone(), loader: cfg.loader.name().into(), logo: None })
}

fn days_left(deleted_at: &str, now: chrono::DateTime<chrono::Local>) -> i64 {
    chrono::DateTime::parse_from_rfc3339(deleted_at).map(|d| RETENTION_DAYS - (now - d.with_timezone(&chrono::Local)).num_days()).unwrap_or(RETENTION_DAYS)
}

fn name_taken(name: &str) -> bool {
    crate::instance::list_configs().iter().any(|c| c.name.eq_ignore_ascii_case(name))
}

/// Evita que dos operaciones toquen el mismo elemento a la vez (939 Host corre una sola vez por PC).
struct Guard(String);
static BUSY: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashSet<String>>> = once_cell::sync::Lazy::new(Default::default);
impl Guard {
    fn try_new(k: &str) -> Option<Guard> { BUSY.lock().unwrap().insert(k.to_string()).then(|| Guard(k.to_string())) }
}
impl Drop for Guard { fn drop(&mut self) { BUSY.lock().unwrap().remove(&self.0); } }

fn pid_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing());
    sys.process(p).is_some()
}

/// Servidores en la Papelera, del más reciente al más viejo.
pub fn list() -> Vec<TrashItem> {
    let now = chrono::Local::now();
    let mut out: Vec<TrashItem> = std::fs::read_dir(paths::trash()).into_iter().flatten().flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let dir = e.path();
            let meta = read_meta(&dir)?;
            if meta.kind != "server" { return None; }
            let conflict = paths::server_dir(&meta.original_id).exists() || name_taken(&meta.name);
            Some(TrashItem { key: e.file_name().to_string_lossy().to_string(), days_left: days_left(&meta.deleted_at, now).max(0), size_bytes: paths::dir_size(&dir), conflict, meta })
        })
        .collect();
    out.sort_by(|a, b| b.meta.deleted_at.cmp(&a.meta.deleted_at));
    out
}

fn item_dir(key: &str) -> AppResult<PathBuf> {
    if key.is_empty() || key.contains(['/', '\\']) || key.contains("..") { return Err(AppError::invalid("Elemento inválido.")); }
    let dir = paths::trash().join(key);
    if !dir.is_dir() { return Err(AppError::not_found("ese elemento de la Papelera")); }
    Ok(dir)
}

/// Restaura un servidor. Si su id o su nombre ya están en uso, se restaura como uno nuevo
/// («Nombre (restaurado)») sin pisar nada. Devuelve el id con el que quedó.
pub fn restore(key: &str) -> AppResult<String> {
    let dir = item_dir(key)?;
    let meta = read_meta(&dir).ok_or_else(|| AppError::invalid("No es un servidor."))?;
    let lock = Guard::try_new(&format!("papelera-{key}")).ok_or_else(|| AppError::busy("Ese servidor ya se está restaurando o borrando."))?;
    let mut cfg: crate::model::ServerConfig = paths::read_json(&dir.join("instance.json")).map_err(|_| AppError::invalid("Falta la configuración del servidor."))?;
    let mut id = meta.original_id.clone();
    let renamed = paths::server_dir(&id).exists() || name_taken(&cfg.name);
    if renamed {
        let base = format!("{}-restaurado", meta.original_id);
        id = base.clone();
        let mut n = 2;
        while paths::server_dir(&id).exists() { id = format!("{base}-{n}"); n += 1; }
        let mut name = format!("{} (restaurado)", cfg.name);
        let mut k = 2;
        while name_taken(&name) { name = format!("{} (restaurado {k})", cfg.name); k += 1; }
        cfg.name = name;
    }
    let dst = paths::server_dir(&id);
    std::fs::create_dir_all(paths::servers()).map_err(|e| AppError::io("No se pudo preparar la carpeta de servidores", e))?;
    paths::rename_retry(&dir, &dst).map_err(|e| AppError::io("No se pudo restaurar el servidor", e))?;
    let _ = std::fs::remove_file(dst.join(META));
    if renamed {
        cfg.id = id.clone();
        // Ya existe otro servidor con esa identidad: la copia restaurada es un servidor distinto y
        // recibe una identidad nueva (dos servidores con la misma identidad se confundirían).
        cfg.identity = crate::identity::create_server_identity(&id, &cfg.name)?;
        // Un servidor restaurado con otro id no puede seguir en el grupo del original (sería el mismo
        // servidor dos veces): queda sin compartir y sin permiso, como uno propio.
        let _ = std::fs::remove_file(dst.join("share.json"));
        let _ = std::fs::remove_file(dst.join("lease.json"));
    }
    let _ = std::fs::remove_file(dst.join("server").join(".939host.lock"));
    crate::instance::save(&cfg)?;
    drop(lock);
    Ok(id)
}

/// Borra para siempre un elemento de la Papelera.
pub fn purge(key: &str) -> AppResult<()> {
    let dir = item_dir(key)?;
    let _lock = Guard::try_new(&format!("papelera-{key}")).ok_or_else(|| AppError::busy("Ese servidor se está restaurando o borrando."))?;
    remove_dir(&dir)
}

fn remove_dir(dir: &Path) -> AppResult<()> {
    // Nunca se borra algo que un proceso esté usando: si queda un Java vivo, se espera.
    let lock = dir.join("server").join(".939host.lock");
    if let Some(pid) = std::fs::read_to_string(&lock).ok().and_then(|t| t.trim().parse::<u32>().ok()) {
        if pid_alive(pid) { return Err(AppError::busy("Un proceso todavía usa archivos de ese servidor.")); }
    }
    let mut last = None;
    for _ in 0..5 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) => { last = Some(e); std::thread::sleep(std::time::Duration::from_millis(400)); }
        }
    }
    Err(AppError::io("No se pudo borrar de la Papelera", last.unwrap()))
}

/// Vacía la Papelera de servidores. Devuelve (borrados, bytes liberados, errores).
pub fn empty() -> (usize, u64, Vec<String>) {
    let mut n = 0; let mut bytes = 0; let mut errs = vec![];
    for it in list() {
        match purge(&it.key) { Ok(()) => { n += 1; bytes += it.size_bytes; } Err(e) => errs.push(format!("{}: {}", it.meta.name, e.message)) }
    }
    (n, bytes, errs)
}

/// Borra lo que lleva más de 30 días (servidores y también mundos o mods apartados).
pub fn purge_expired() -> usize {
    let now = chrono::Local::now();
    let mut n = 0;
    for e in std::fs::read_dir(paths::trash()).into_iter().flatten().flatten() {
        let dir = e.path();
        let when = read_meta(&dir).map(|m| m.deleted_at).or_else(|| {
            std::fs::metadata(&dir).ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339())
        });
        let Some(when) = when else { continue };
        if days_left(&when, now) <= 0 {
            let key = e.file_name().to_string_lossy().to_string();
            let Some(_l) = Guard::try_new(&format!("papelera-{key}")) else { continue };
            let ok = if dir.is_dir() { remove_dir(&dir).is_ok() } else { std::fs::remove_file(&dir).is_ok() };
            if ok { n += 1; }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn days_left_counts_from_real_deletion_date() {
        let now = chrono::Local::now();
        assert_eq!(days_left(&now.to_rfc3339(), now), 30);
        assert_eq!(days_left(&(now - chrono::Duration::days(29)).to_rfc3339(), now), 1);
        assert!(days_left(&(now - chrono::Duration::days(31)).to_rfc3339(), now) <= 0);
    }
}
