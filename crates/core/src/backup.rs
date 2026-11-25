//! Backups recuperables. En esta versión son archivos .zip consistentes; el motor incremental
//! deduplicado (puntos de restauración baratos cada 15 min) llega en la etapa 4.
//!
//! Con el servidor encendido, la copia se toma en un punto consistente:
//! `save-off` → `save-all flush` (espera la confirmación) → copia local → `save-on`.

use crate::error::{AppError, AppResult};
use crate::paths;
use crate::process::Manager;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const EXTRA_FILES: &[&str] = &["server.properties", "whitelist.json", "ops.json", "banned-players.json", "banned-ips.json"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub file: String,
    pub label: String,
    pub kind: String,
    pub created_at: String,
    pub size_mb: f64,
    pub mc_version: String,
}

fn sidecar(zip: &Path) -> PathBuf { zip.with_extension("json") }

pub fn list(id: &str) -> Vec<BackupInfo> {
    let dir = paths::server_backups_dir(id);
    let mut out: Vec<BackupInfo> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten()
            .filter(|e| e.path().extension().map(|x| x == "zip").unwrap_or(false))
            .filter_map(|e| {
                let mut info: BackupInfo = paths::read_json(&sidecar(&e.path())).ok()?;
                info.size_mb = e.metadata().map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0);
                Some(info)
            })
            .collect())
        .unwrap_or_default();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Copia el mundo a una carpeta temporal (consistente) y la comprime.
pub fn create(mgr: &Manager, id: &str, label: &str, kind: &str, mc_version: &str) -> AppResult<BackupInfo> {
    let rt = mgr.runtime(id);
    {
        let mut r = rt.lock().unwrap();
        if let Some(b) = &r.busy { return Err(AppError::busy(format!("Ya hay una operación en curso: {b}."))); }
        r.busy = Some("backup".into());
    }
    let result = (|| {
        let run = paths::server_run_dir(id);
        if !run.join("world").join("level.dat").exists() {
            return Err(AppError::invalid("Todavía no hay mundo para respaldar: iniciá el servidor una vez para que se genere."));
        }
        let staging = paths::server_dir(id).join(".backup-staging");
        let _ = std::fs::remove_dir_all(&staging);
        let running = rt.lock().unwrap().status() == crate::model::RunStatus::Running;
        if running {
            mgr.send_command(id, "save-off")?;
            let copy = mgr.command_and_wait_save(id, "save-all flush", Duration::from_secs(120))
                .and_then(|_| paths::copy_dir(&run.join("world"), &staging.join("world")));
            // save-on siempre, aunque la copia falle.
            let _ = mgr.send_command(id, "save-on");
            copy?;
        } else {
            paths::copy_dir(&run.join("world"), &staging.join("world"))?;
        }
        for f in EXTRA_FILES {
            let p = run.join(f);
            if p.exists() { let _ = std::fs::copy(&p, staging.join(f)); }
        }
        let now = chrono::Local::now();
        let file = format!("{}.zip", now.format("%Y-%m-%d_%H-%M-%S"));
        let dest = paths::server_backups_dir(id).join(&file);
        zip_dir(&staging, &dest)?;
        let _ = std::fs::remove_dir_all(&staging);
        let info = BackupInfo {
            file,
            label: if label.trim().is_empty() { "Backup manual".into() } else { label.trim().chars().take(80).collect() },
            kind: kind.into(),
            created_at: now.to_rfc3339(),
            size_mb: std::fs::metadata(&dest).map(|m| m.len() as f64 / 1_048_576.0).unwrap_or(0.0),
            mc_version: mc_version.into(),
        };
        paths::write_json(&sidecar(&dest), &info)?;
        Ok(info)
    })();
    rt.lock().unwrap().busy = None;
    result
}

fn zip_dir(src: &Path, dest: &Path) -> AppResult<()> {
    let tmp = dest.with_extension("zip.part");
    let file = std::fs::File::create(&tmp).map_err(|e| AppError::io("No se pudo crear el backup", e))?;
    let mut zw = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(3)).large_file(true);
    fn walk(zw: &mut zip::ZipWriter<std::io::BufWriter<std::fs::File>>, base: &Path, dir: &Path, opts: zip::write::SimpleFileOptions) -> AppResult<()> {
        for e in std::fs::read_dir(dir).map_err(|e| AppError::io("No se pudo leer el mundo", e))?.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                zw.add_directory(format!("{rel}/"), opts)?;
                walk(zw, base, &p, opts)?;
            } else {
                zw.start_file(rel, opts)?;
                let mut f = std::fs::File::open(&p).map_err(|e| AppError::io("No se pudo leer un archivo del mundo", e))?;
                std::io::copy(&mut f, zw).map_err(|e| AppError::io("No se pudo comprimir un archivo", e))?;
            }
        }
        Ok(())
    }
    walk(&mut zw, src, src, opts)?;
    let mut inner = zw.finish()?;
    inner.flush().map_err(|e| AppError::io("No se pudo terminar el backup", e))?;
    drop(inner);
    std::fs::rename(&tmp, dest).map_err(|e| AppError::io("No se pudo guardar el backup", e))?;
    Ok(())
}

/// Restaura un backup. Antes guarda el estado actual como "Antes de restaurar" y aparta el mundo.
pub fn restore(mgr: &Manager, id: &str, file: &str, mc_version: &str) -> AppResult<()> {
    if file.contains(['/', '\\']) || !file.ends_with(".zip") { return Err(AppError::invalid("Nombre de backup inválido.")); }
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de restaurar.")); }
    let zip_path = paths::server_backups_dir(id).join(file);
    if !zip_path.exists() { return Err(AppError::not_found("el backup")); }
    {
        let f = std::fs::File::open(&zip_path).map_err(|e| AppError::io("No se pudo abrir el backup", e))?;
        let mut z = zip::ZipArchive::new(f)?;
        if z.by_name("world/level.dat").is_err() { return Err(AppError::invalid("Ese backup no contiene un mundo válido.")); }
    }
    let run = paths::server_run_dir(id);
    if run.join("world").join("level.dat").exists() {
        create(mgr, id, "Antes de restaurar", "seguridad", mc_version)?;
        paths::rename_retry(run.join("world"), paths::trash_slot(&format!("{id}-world"))).map_err(|e| AppError::io("No se pudo apartar el mundo actual", e))?;
    }
    crate::java::extract_zip(&zip_path, &run)
}

pub fn delete(id: &str, file: &str) -> AppResult<()> {
    if file.contains(['/', '\\']) || !file.ends_with(".zip") { return Err(AppError::invalid("Nombre de backup inválido.")); }
    let p = paths::server_backups_dir(id).join(file);
    std::fs::remove_file(&p).map_err(|e| AppError::io("No se pudo borrar el backup", e))?;
    let _ = std::fs::remove_file(sidecar(&p));
    Ok(())
}
