//! Puntos de restauración incrementales y deduplicados.
//!
//! Cada archivo del mundo se corta en bloques de 64 KiB identificados por su SHA-256. Un bloque
//! que ya existe no se vuelve a guardar: diez puntos de restauración de un mundo que cambió poco
//! ocupan apenas más que uno. Los archivos que no cambiaron (mismo tamaño y fecha) ni se leen.
//! Es la misma base que usará la sincronización entre PCs: solo viajan los bloques nuevos.
//!
//! Con el servidor encendido la captura se hace en un punto consistente:
//! `save-off` → `save-all flush` → captura de lo que cambió → `save-on`.

use crate::error::{AppError, AppResult};
use crate::model::RunStatus;
use crate::paths;
use crate::process::Manager;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const BLOCK: usize = 64 * 1024;
const EXTRA_FILES: &[&str] = &[
    "server.properties", "whitelist.json", "ops.json", "banned-players.json", "banned-ips.json",
    "fabric-server-launcher.properties", "user_jvm_args.txt",
];
/// Carpetas que forman el estado del servidor: el mundo, los mods y sus configuraciones.
/// Con esto otra PC del grupo puede levantar exactamente el mismo servidor.
const STATE_DIRS: &[&str] = &["world", "mods", "config"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub blocks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub id: String,
    pub created_at: String,
    pub label: String,
    /// auto | manual | seguridad
    pub kind: String,
    pub mc_version: String,
    pub file_count: usize,
    pub total_bytes: u64,
    /// Bytes nuevos que agregó este punto al almacén (lo que realmente ocupa).
    pub new_bytes: u64,
    pub pause_ms: u64,
    /// Época del permiso de anfitrión con la que se tomó (ver `lease`).
    #[serde(default)]
    pub epoch: u64,
    /// Número de orden dentro del servidor lógico: el más alto es el estado más reciente.
    #[serde(default)]
    pub seq: u64,
    /// PC que lo tomó (id del nodo).
    #[serde(default)]
    pub origin: String,
}

impl SnapshotInfo {
    /// Clave de "frescura": primero la época, después el orden.
    pub fn freshness(&self) -> (u64, u64) { (self.epoch, self.seq) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    info: SnapshotInfo,
    files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStats {
    pub snapshots: usize,
    pub blob_count: usize,
    pub store_bytes: u64,
    pub logical_bytes: u64,
}

fn store_dir(id: &str) -> PathBuf { paths::server_dir(id).join("restore") }
fn blobs_dir(id: &str) -> PathBuf { store_dir(id).join("blobs") }
fn snaps_dir(id: &str) -> PathBuf { store_dir(id).join("snapshots") }
fn blob_path(id: &str, hash: &str) -> PathBuf { blobs_dir(id).join(&hash[..2]).join(hash) }
fn snap_path(id: &str, snap: &str) -> PathBuf { snaps_dir(id).join(format!("{snap}.json.gz")) }

fn write_snapshot(id: &str, s: &Snapshot) -> AppResult<()> {
    let json = serde_json::to_vec(s)?;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&json).map_err(|e| AppError::io("No se pudo comprimir el índice", e))?;
    let bytes = enc.finish().map_err(|e| AppError::io("No se pudo comprimir el índice", e))?;
    paths::write_atomic(&snap_path(id, &s.info.id), &bytes)
}

fn read_snapshot(id: &str, snap: &str) -> AppResult<Snapshot> {
    if snap.contains(['/', '\\', '.']) { return Err(AppError::invalid("Identificador de punto inválido.")); }
    let bytes = std::fs::read(snap_path(id, snap)).map_err(|e| AppError::io("No se encontró el punto de restauración", e))?;
    let mut json = vec![];
    flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut json).map_err(|e| AppError::io("El índice del punto está dañado", e))?;
    Ok(serde_json::from_slice(&json)?)
}

pub fn list(id: &str) -> Vec<SnapshotInfo> {
    let mut out: Vec<SnapshotInfo> = std::fs::read_dir(snaps_dir(id))
        .map(|rd| rd.flatten().filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let snap = name.strip_suffix(".json.gz")?;
            read_snapshot(id, snap).ok().map(|s| s.info)
        }).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Archivos que forman parte del estado restaurable (rutas relativas a la carpeta del servidor).
fn collect_files(run: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() { walk(base, &p, out); }
            else if ft.is_file() && e.file_name() != "session.lock" {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, p));
            }
        }
    }
    for d in STATE_DIRS { walk(run, &run.join(d), &mut out); }
    for f in EXTRA_FILES { let p = run.join(f); if p.is_file() { out.push((f.to_string(), p)); } }
    out.sort();
    out
}

/// Captura sin coordinar con el servidor (servidor apagado, o ya en save-off).
pub(crate) fn capture(id: &str, label: &str, kind: &str, mc_version: &str) -> AppResult<SnapshotInfo> {
    let run = paths::server_run_dir(id);
    if !run.join("world").join("level.dat").exists() {
        return Err(AppError::invalid("Todavía no hay mundo: iniciá el servidor una vez para que se genere."));
    }
    let started = Instant::now();
    // Índice del último punto para no releer archivos sin cambios.
    let previous: HashMap<String, FileEntry> = list(id).first()
        .and_then(|i| read_snapshot(id, &i.id).ok())
        .map(|s| s.files.into_iter().map(|f| (f.path.clone(), f)).collect())
        .unwrap_or_default();

    let mut files = vec![];
    let mut total = 0u64;
    let mut new_bytes = 0u64;
    let mut buf = vec![0u8; BLOCK];
    for (rel, path) in collect_files(&run) {
        let meta = std::fs::metadata(&path).map_err(|e| AppError::io("No se pudo leer un archivo del mundo", e))?;
        let (size, mt) = (meta.len(), mtime_ms(&meta));
        total += size;
        if let Some(prev) = previous.get(&rel) {
            if prev.size == size && prev.mtime_ms == mt && prev.blocks.iter().all(|h| blob_path(id, h).exists()) {
                files.push(prev.clone());
                continue;
            }
        }
        let mut f = std::fs::File::open(&path).map_err(|e| AppError::io("No se pudo abrir un archivo del mundo", e))?;
        let mut blocks = vec![];
        loop {
            let n = read_full(&mut f, &mut buf).map_err(|e| AppError::io("No se pudo leer un archivo del mundo", e))?;
            if n == 0 { break; }
            let hash = hex::encode(Sha256::digest(&buf[..n]));
            let bp = blob_path(id, &hash);
            if !bp.exists() {
                std::fs::create_dir_all(bp.parent().unwrap()).map_err(|e| AppError::io("No se pudo crear el almacén", e))?;
                let tmp = bp.with_extension("tmp");
                std::fs::write(&tmp, &buf[..n]).map_err(|e| AppError::io("No se pudo guardar un bloque", e))?;
                std::fs::rename(&tmp, &bp).map_err(|e| AppError::io("No se pudo guardar un bloque", e))?;
                new_bytes += n as u64;
            }
            blocks.push(hash);
            if n < BLOCK { break; }
        }
        files.push(FileEntry { path: rel, size, mtime_ms: mt, blocks });
    }
    let now = chrono::Local::now();
    let seq = list(id).iter().filter(|s| s.kind != "seguridad").map(|s| s.seq).max().unwrap_or(0) + 1;
    let info = SnapshotInfo {
        epoch: crate::lease::read(id).map(|l| l.epoch).unwrap_or(0),
        seq,
        origin: crate::lease::local_node().unwrap_or_default(),
        id: format!("{}-{}", now.format("%Y%m%d-%H%M%S-%3f"), &kind[..1]),
        created_at: now.to_rfc3339(),
        label: if label.trim().is_empty() { default_label(kind).into() } else { label.trim().chars().take(80).collect() },
        kind: kind.into(),
        mc_version: mc_version.into(),
        file_count: files.len(),
        total_bytes: total,
        new_bytes,
        pause_ms: started.elapsed().as_millis() as u64,
    };
    write_snapshot(id, &Snapshot { info: info.clone(), files })?;
    Ok(info)
}

fn default_label(kind: &str) -> &'static str {
    match kind { "auto" => "Automático", "seguridad" => "Antes de restaurar", _ => "Manual" }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let r = f.read(&mut buf[n..])?;
        if r == 0 { break; }
        n += r;
    }
    Ok(n)
}

/// Toma un punto de restauración. Si el servidor está encendido, pausa el guardado solo
/// mientras se capturan los archivos que cambiaron.
pub fn take(mgr: &Manager, id: &str, label: &str, kind: &str, mc_version: &str) -> AppResult<SnapshotInfo> {
    let rt = mgr.runtime(id);
    {
        let mut r = rt.lock().unwrap();
        if let Some(b) = &r.busy { return Err(AppError::busy(format!("Ya hay una operación en curso: {b}."))); }
        r.busy = Some("punto de restauración".into());
    }
    let result = (|| {
        let running = rt.lock().unwrap().status() == RunStatus::Running;
        if running {
            mgr.send_command(id, "save-off")?;
            let r = mgr.command_and_wait_save(id, "save-all flush", Duration::from_secs(120)).and_then(|_| capture(id, label, kind, mc_version));
            let _ = mgr.send_command(id, "save-on");
            r
        } else {
            capture(id, label, kind, mc_version)
        }
    })();
    rt.lock().unwrap().busy = None;
    result
}

/// Restaura un punto. El servidor tiene que estar apagado. Antes guarda el estado actual
/// como punto de seguridad y aparta las carpetas actuales a la Papelera.
pub fn restore(mgr: &Manager, id: &str, snap_id: &str, mc_version: &str) -> AppResult<()> {
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de restaurar.")); }
    materialize(id, snap_id, mc_version)
}

/// Coloca un punto en la carpeta del servidor (sin comprobar el proceso: quien llama se asegura
/// de que no esté corriendo). Se usa al restaurar y cuando esta PC pasa a ser la anfitriona.
/// Nunca destruye nada: lo actual queda como punto "seguridad" y en la Papelera.
pub fn materialize(id: &str, snap_id: &str, mc_version: &str) -> AppResult<()> {
    let snap = read_snapshot(id, snap_id)?;
    // Verificar que estén todos los bloques antes de tocar nada.
    for f in &snap.files {
        for h in &f.blocks {
            if !blob_path(id, h).exists() { return Err(AppError::new("integrity", "Faltan datos de este punto de restauración; no se tocó nada.")); }
        }
    }
    let run = paths::server_run_dir(id);
    if run.join("world").join("level.dat").exists() {
        // Si lo actual ya es exactamente este punto, no hay nada que hacer.
        if same_as_disk(&run, &snap) { return Ok(()); }
        capture(id, "", "seguridad", mc_version)?;
    }

    let staging = paths::server_dir(id).join(".restore-staging");
    let _ = std::fs::remove_dir_all(&staging);
    for f in &snap.files {
        let out = staging.join(&f.path);
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| AppError::io("No se pudo preparar la restauración", e))?;
        let mut w = std::fs::File::create(&out).map_err(|e| AppError::io("No se pudo preparar la restauración", e))?;
        for h in &f.blocks {
            let data = std::fs::read(blob_path(id, h)).map_err(|e| AppError::io("No se pudo leer un bloque", e))?;
            if hex::encode(Sha256::digest(&data)) != *h {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(AppError::new("integrity", "Un bloque guardado está dañado; la restauración se canceló sin tocar el mundo actual."));
            }
            w.write_all(&data).map_err(|e| AppError::io("No se pudo escribir la restauración", e))?;
        }
        w.sync_all().ok();
    }
    std::fs::create_dir_all(&run).ok();
    for d in STATE_DIRS {
        let cur = run.join(d);
        let st = staging.join(d);
        // Puntos viejos (antes de v0.3) no traen mods/config: en ese caso se dejan como están.
        let snapshot_has_dir = snap.files.iter().any(|f| f.path.starts_with(&format!("{d}/")));
        if *d != "world" && !snapshot_has_dir && !snap_covers_dirs(&snap) { continue; }
        if cur.exists() {
            paths::rename_retry(&cur, paths::trash_slot(&format!("{id}-{d}"))).map_err(|e| AppError::io("No se pudo apartar la carpeta actual", e))?;
        }
        if st.exists() {
            std::fs::rename(&st, &cur).map_err(|e| AppError::io("No se pudo colocar la carpeta restaurada", e))?;
        } else if *d != "world" {
            std::fs::create_dir_all(&cur).ok();
        }
    }
    for f in EXTRA_FILES {
        let p = staging.join(f);
        if p.exists() { let _ = std::fs::rename(&p, run.join(f)); }
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

/// Los puntos desde v0.3 (con `seq`) incluyen siempre mods/ y config/ aunque estén vacíos.
fn snap_covers_dirs(s: &Snapshot) -> bool { s.info.seq > 0 }

fn same_as_disk(run: &Path, snap: &Snapshot) -> bool {
    let disk = collect_files(run);
    if disk.len() != snap.files.len() { return false; }
    for ((rel, p), f) in disk.iter().zip(snap.files.iter()) {
        if *rel != f.path { return false; }
        let Ok(meta) = std::fs::metadata(p) else { return false };
        if meta.len() != f.size { return false; }
        // Comparación por contenido, bloque a bloque.
        let Ok(mut file) = std::fs::File::open(p) else { return false };
        let mut buf = vec![0u8; BLOCK];
        for h in &f.blocks {
            let Ok(n) = read_full(&mut file, &mut buf) else { return false };
            if hex::encode(Sha256::digest(&buf[..n])) != *h { return false; }
        }
    }
    true
}

// ───────────────────────── Sincronización entre PCs ─────────────────────────
//
// Otra PC del grupo pide el índice del punto más reciente, calcula qué bloques le faltan,
// los pide uno por uno (cada bloque se verifica por su SHA-256 al recibirlo) y recién
// cuando tiene todo registra el índice. Un punto a medio copiar nunca es visible.

/// El punto más reciente del servidor lógico (ignora los de seguridad, que son locales).
pub fn latest(id: &str) -> Option<SnapshotInfo> {
    list(id).into_iter().filter(|s| s.kind != "seguridad").max_by(|a, b| a.freshness().cmp(&b.freshness()).then(a.created_at.cmp(&b.created_at)))
}

/// Índice comprimido tal como está guardado, para enviarlo.
pub fn manifest_bytes(id: &str, snap_id: &str) -> AppResult<Vec<u8>> {
    read_snapshot(id, snap_id)?;
    std::fs::read(snap_path(id, snap_id)).map_err(|e| AppError::io("No se pudo leer el índice", e))
}

fn decode_manifest(bytes: &[u8]) -> AppResult<Snapshot> {
    if bytes.len() > 64 * 1024 * 1024 { return Err(AppError::invalid("Índice demasiado grande.")); }
    let mut json = vec![];
    flate2::read::GzDecoder::new(bytes).take(512 * 1024 * 1024).read_to_end(&mut json).map_err(|e| AppError::io("Índice recibido dañado", e))?;
    let s: Snapshot = serde_json::from_slice(&json)?;
    if s.info.id.contains(['/', '\\', '.']) || s.info.id.is_empty() { return Err(AppError::invalid("Identificador de punto inválido.")); }
    for f in &s.files {
        let ok_path = !f.path.is_empty() && !f.path.starts_with('/') && !f.path.contains('\\') && !f.path.split('/').any(|c| c == ".." || c.is_empty())
            && (STATE_DIRS.iter().any(|d| f.path.starts_with(&format!("{d}/"))) || EXTRA_FILES.contains(&f.path.as_str()));
        if !ok_path { return Err(AppError::invalid(format!("Ruta no permitida en el índice: {}", f.path))); }
        if f.blocks.iter().any(|h| h.len() != 64 || !h.bytes().all(|c| c.is_ascii_hexdigit())) {
            return Err(AppError::invalid("Bloque con identificador inválido."));
        }
    }
    Ok(s)
}

/// Información de un índice recibido, sin registrarlo.
pub fn manifest_info(bytes: &[u8]) -> AppResult<SnapshotInfo> { Ok(decode_manifest(bytes)?.info) }

/// Bloques de un índice recibido que todavía no están en este almacén (sin repetir).
pub fn missing_blobs(id: &str, bytes: &[u8]) -> AppResult<Vec<String>> {
    let s = decode_manifest(bytes)?;
    let mut seen = HashSet::new();
    Ok(s.files.iter().flat_map(|f| f.blocks.iter()).filter(|h| seen.insert((*h).clone()) && !blob_path(id, h).exists()).cloned().collect())
}

pub fn has_snapshot(id: &str, snap_id: &str) -> bool {
    !snap_id.contains(['/', '\\', '.']) && snap_path(id, snap_id).exists()
}

pub fn read_blob(id: &str, hash: &str) -> AppResult<Vec<u8>> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) { return Err(AppError::invalid("Bloque inválido.")); }
    std::fs::read(blob_path(id, hash)).map_err(|e| AppError::io("No existe ese bloque", e))
}

/// Guarda un bloque recibido. Rechaza el bloque si su contenido no coincide con su SHA-256.
pub fn put_blob(id: &str, hash: &str, data: &[u8]) -> AppResult<()> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) { return Err(AppError::invalid("Bloque inválido.")); }
    if data.len() > BLOCK || hex::encode(Sha256::digest(data)) != hash.to_ascii_lowercase() {
        return Err(AppError::new("integrity", "Un bloque recibido no coincide con su huella; se descartó."));
    }
    let bp = blob_path(id, hash);
    if bp.exists() { return Ok(()); }
    std::fs::create_dir_all(bp.parent().unwrap()).map_err(|e| AppError::io("No se pudo crear el almacén", e))?;
    let tmp = bp.with_extension(format!("rx{}", std::process::id()));
    std::fs::write(&tmp, data).map_err(|e| AppError::io("No se pudo guardar un bloque", e))?;
    std::fs::rename(&tmp, &bp).map_err(|e| AppError::io("No se pudo guardar un bloque", e))?;
    Ok(())
}

/// Registra un índice recibido. Solo si ya están todos sus bloques.
pub fn import_manifest(id: &str, bytes: &[u8]) -> AppResult<SnapshotInfo> {
    let s = decode_manifest(bytes)?;
    for f in &s.files {
        for h in &f.blocks {
            if !blob_path(id, h).exists() { return Err(AppError::new("integrity", "Faltan bloques; el punto no se registró.")); }
        }
    }
    write_snapshot(id, &s)?;
    Ok(s.info)
}

pub fn delete(id: &str, snap_id: &str) -> AppResult<()> {
    read_snapshot(id, snap_id)?;
    std::fs::remove_file(snap_path(id, snap_id)).map_err(|e| AppError::io("No se pudo borrar el punto", e))?;
    gc(id);
    Ok(())
}

/// Retención: todos los automáticos de las últimas 6 h, uno por hora durante 2 días,
/// uno por día durante 14 días y uno por semana durante 8 semanas. Los manuales y de
/// seguridad no se borran solos. Después libera los bloques que ya nadie usa.
pub fn prune(id: &str) -> usize {
    let now = chrono::Local::now();
    let mut keep_buckets: HashSet<String> = HashSet::new();
    let mut removed = 0;
    let mut autos: Vec<SnapshotInfo> = list(id).into_iter().filter(|s| s.kind == "auto").collect();
    autos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    for s in autos {
        let Ok(t) = chrono::DateTime::parse_from_rfc3339(&s.created_at) else { continue };
        let age = now.signed_duration_since(t);
        let bucket = if age < chrono::Duration::hours(6) { None }
            else if age < chrono::Duration::hours(48) { Some(format!("h{}", t.format("%Y%m%d%H"))) }
            else if age < chrono::Duration::days(14) { Some(format!("d{}", t.format("%Y%m%d"))) }
            else if age < chrono::Duration::weeks(8) { Some(format!("w{}", t.format("%G%V"))) }
            else { Some("drop".into()) };
        let keep = match bucket { None => true, Some(b) if b == "drop" => false, Some(b) => keep_buckets.insert(b) };
        if !keep && std::fs::remove_file(snap_path(id, &s.id)).is_ok() { removed += 1; }
    }
    if removed > 0 { gc(id); }
    removed
}

/// Borra los bloques que ningún punto referencia.
pub fn gc(id: &str) -> usize {
    let mut used: HashSet<String> = HashSet::new();
    for s in list(id) {
        if let Ok(full) = read_snapshot(id, &s.id) { for f in full.files { used.extend(f.blocks); } }
    }
    let mut removed = 0;
    if let Ok(rd) = std::fs::read_dir(blobs_dir(id)) {
        for sub in rd.flatten() {
            if let Ok(inner) = std::fs::read_dir(sub.path()) {
                for b in inner.flatten() {
                    let name = b.file_name().to_string_lossy().to_string();
                    if !used.contains(&name) && std::fs::remove_file(b.path()).is_ok() { removed += 1; }
                }
            }
        }
    }
    removed
}

pub fn stats(id: &str) -> StoreStats {
    let snaps = list(id);
    let logical = snaps.iter().map(|s| s.total_bytes).sum();
    let mut count = 0;
    let mut bytes = 0;
    if let Ok(rd) = std::fs::read_dir(blobs_dir(id)) {
        for sub in rd.flatten() {
            if let Ok(inner) = std::fs::read_dir(sub.path()) {
                for b in inner.flatten() { count += 1; bytes += b.metadata().map(|m| m.len()).unwrap_or(0); }
            }
        }
    }
    StoreStats { snapshots: snaps.len(), blob_count: count, store_bytes: bytes, logical_bytes: logical }
}

/// ¿Corresponde un punto automático? (según el intervalo configurado del servidor)
pub fn auto_due(id: &str, interval_min: u32) -> bool {
    let last = list(id).into_iter().find(|s| s.kind == "auto");
    match last.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s.created_at).ok()) {
        Some(t) => chrono::Local::now().signed_duration_since(t) >= chrono::Duration::minutes(interval_min as i64),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> String {
        let root = std::env::temp_dir().join("939host-snap-tests");
        let _ = paths::init(root);
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        let id = format!("s{}", hex::encode(b));
        let run = paths::server_run_dir(&id);
        std::fs::create_dir_all(run.join("world/region")).unwrap();
        std::fs::write(run.join("world/level.dat"), b"level-1").unwrap();
        // Una "región" de 1 MiB con contenido variado.
        let region: Vec<u8> = (0..(1 << 20)).map(|i| ((i * 31) % 251) as u8).collect();
        std::fs::write(run.join("world/region/r.0.0.mca"), &region).unwrap();
        std::fs::write(run.join("world/session.lock"), b"x").unwrap();
        std::fs::write(run.join("server.properties"), b"motd=a").unwrap();
        id
    }

    #[test]
    fn dedup_restore_and_retention() {
        let id = setup();
        let run = paths::server_run_dir(&id);
        let a = capture(&id, "", "manual", "26.2").unwrap();
        assert_eq!(a.file_count, 3, "level.dat, región y server.properties; session.lock se excluye");
        assert!(a.new_bytes >= 1 << 20);

        // Sin cambios: el segundo punto no agrega bloques.
        std::thread::sleep(Duration::from_millis(5));
        let b = capture(&id, "", "auto", "26.2").unwrap();
        assert_eq!(b.new_bytes, 0, "nada cambió, nada nuevo");

        // Cambia un solo "chunk" de 4 KiB en el medio de la región: entra 1 bloque de 64 KiB.
        let mut region = std::fs::read(run.join("world/region/r.0.0.mca")).unwrap();
        for byte in &mut region[300_000..304_096] { *byte = 0xAB; }
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(run.join("world/region/r.0.0.mca"), &region).unwrap();
        std::fs::write(run.join("world/level.dat"), b"level-2").unwrap();
        let c = capture(&id, "", "auto", "26.2").unwrap();
        assert!(c.new_bytes <= (BLOCK as u64) * 2 + 16, "solo los bloques tocados: {}", c.new_bytes);

        let st = stats(&id);
        assert_eq!(st.snapshots, 3);
        assert!(st.store_bytes < st.logical_bytes / 2, "deduplicación: {} vs {}", st.store_bytes, st.logical_bytes);

        // Restaurar el primero devuelve los bytes exactos.
        let mgr = Manager::new(std::sync::Arc::new(crate::events::NoEvents));
        restore(&mgr, &id, &a.id, "26.2").unwrap();
        assert_eq!(std::fs::read(run.join("world/level.dat")).unwrap(), b"level-1");
        let restored = std::fs::read(run.join("world/region/r.0.0.mca")).unwrap();
        assert_eq!(restored[300_000], ((300_000u64 * 31) % 251) as u8);
        assert!(list(&id).iter().any(|s| s.kind == "seguridad"), "guarda el estado previo");

        // Un bloque dañado cancela la restauración sin tocar nada.
        let snap = read_snapshot(&id, &c.id).unwrap();
        let victim = blob_path(&id, &snap.files.iter().find(|f| f.path.ends_with(".mca")).unwrap().blocks[4]);
        std::fs::write(&victim, b"corrupto").unwrap();
        let before = std::fs::read(run.join("world/level.dat")).unwrap();
        assert!(restore(&mgr, &id, &c.id, "26.2").is_err());
        assert_eq!(std::fs::read(run.join("world/level.dat")).unwrap(), before);

        // Borrar puntos libera bloques que nadie usa.
        let blobs_before = stats(&id).blob_count;
        for s in list(&id) { if s.id != a.id { delete(&id, &s.id).unwrap(); } }
        assert!(stats(&id).blob_count < blobs_before);
        assert_eq!(list(&id).len(), 1);
        assert!(auto_due(&id, 15));
    }

    #[test]
    fn sync_between_two_stores_is_verified_and_complete() {
        let a = setup();
        let run_a = paths::server_run_dir(&a);
        std::fs::create_dir_all(run_a.join("mods")).unwrap();
        std::fs::create_dir_all(run_a.join("config/sub")).unwrap();
        std::fs::write(run_a.join("mods/lithium.jar"), vec![7u8; 200_000]).unwrap();
        std::fs::write(run_a.join("config/sub/x.toml"), b"k=1").unwrap();
        let s1 = capture(&a, "", "auto", "26.2").unwrap();
        assert_eq!(s1.file_count, 5, "mundo + mods + config + server.properties");

        // "Otra PC": otro almacén vacío.
        let b = setup();
        let run_b = paths::server_run_dir(&b);
        std::fs::remove_dir_all(&run_b).unwrap();
        let man = manifest_bytes(&a, &s1.id).unwrap();
        let missing = missing_blobs(&b, &man).unwrap();
        assert!(!missing.is_empty());
        // Sin todos los bloques el índice no se registra.
        assert!(import_manifest(&b, &man).is_err());
        assert!(!has_snapshot(&b, &s1.id));
        // Un bloque adulterado se rechaza.
        let mut bad = read_blob(&a, &missing[0]).unwrap();
        bad[0] ^= 1;
        assert!(put_blob(&b, &missing[0], &bad).is_err());
        for h in &missing { put_blob(&b, h, &read_blob(&a, h).unwrap()).unwrap(); }
        assert!(missing_blobs(&b, &man).unwrap().is_empty());
        let info = import_manifest(&b, &man).unwrap();
        assert_eq!(latest(&b).unwrap().id, s1.id);
        assert_eq!(info.seq, s1.seq);
        materialize(&b, &s1.id, "26.2").unwrap();
        for f in ["world/level.dat", "world/region/r.0.0.mca", "mods/lithium.jar", "config/sub/x.toml", "server.properties"] {
            assert_eq!(std::fs::read(run_a.join(f)).unwrap(), std::fs::read(run_b.join(f)).unwrap(), "{f}");
        }
        // Materializar de nuevo lo mismo no genera puntos de seguridad.
        materialize(&b, &s1.id, "26.2").unwrap();
        assert!(!list(&b).iter().any(|s| s.kind == "seguridad"));

        // Segundo punto en A: solo viaja lo nuevo.
        std::fs::write(run_a.join("world/level.dat"), b"level-9").unwrap();
        std::fs::remove_file(run_a.join("mods/lithium.jar")).unwrap();
        let s2 = capture(&a, "", "auto", "26.2").unwrap();
        assert!(s2.seq > s1.seq);
        let man2 = manifest_bytes(&a, &s2.id).unwrap();
        let miss2 = missing_blobs(&b, &man2).unwrap();
        assert_eq!(miss2.len(), 1, "solo el level.dat nuevo");
        for h in &miss2 { put_blob(&b, h, &read_blob(&a, h).unwrap()).unwrap(); }
        import_manifest(&b, &man2).unwrap();
        // En B alguien había tocado el mundo (copia huérfana): se aparta como seguridad, no se pierde.
        std::fs::write(run_b.join("world/level.dat"), b"huerfano").unwrap();
        materialize(&b, &s2.id, "26.2").unwrap();
        assert_eq!(std::fs::read(run_b.join("world/level.dat")).unwrap(), b"level-9");
        assert!(!run_b.join("mods/lithium.jar").exists(), "el mod quitado en A se quita en B");
        assert!(list(&b).iter().any(|s| s.kind == "seguridad"));
        assert_eq!(latest(&b).unwrap().id, s2.id, "el de seguridad no cuenta como más reciente");

        // Índices con rutas peligrosas se rechazan.
        let evil = Snapshot { info: s2.clone(), files: vec![FileEntry { path: "world/../../x".into(), size: 0, mtime_ms: 0, blocks: vec![] }] };
        let json = serde_json::to_vec(&evil).unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&json).unwrap();
        assert!(import_manifest(&b, &enc.finish().unwrap()).is_err());
    }
}
