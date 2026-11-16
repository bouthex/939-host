//! Administrador de archivos: local si el servidor está en esta PC, remoto (P2P) si lo aloja
//! otra PC del grupo. Mismas reglas en los dos casos (las aplica `host939_core::files`).

use crate::network::Cluster;
use host939_core::error::{AppError, AppResult};
use host939_core::files as lf;
use host939_core::process::Manager;
use host939_core::{network, paths};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager as _, State};

static CANCEL: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing { entries: Vec<lf::Entry>, remote: bool, host_name: Option<String>, active: bool }

/// `Some(nombre)` si el servidor lo aloja otra PC del grupo ahora.
async fn remote_host(cluster: &Cluster, id: &str) -> Option<(String, String)> {
    if !network::load_share(id).enabled { return None; }
    let node = cluster.node().await.ok()?;
    let host = node.files_host(id)?;
    if host == node.id() { return None; }
    let name = network::load_share(id).member(&host).map(|m| m.name.clone()).unwrap_or_else(|| "otra PC".into());
    Some((host, name))
}
fn active(app: &AppHandle, id: &str) -> bool { let m = app.state::<Manager>(); let r = m.runtime(id); let x = r.lock().unwrap(); x.is_active() || x.installing }
fn rerr(e: anyhow::Error) -> AppError {
    let s = e.to_string();
    if let Some(rest) = s.strip_prefix("[exists] ") { return AppError::new("exists", rest.to_string()); }
    let msg = s.splitn(2, "] ").last().unwrap_or(&s).to_string();
    AppError::new("remote", msg)
}
fn lock(id: &str) -> AppResult<host939_cluster::files::WriteGuard> {
    host939_cluster::files::write_lock(id).ok_or_else(|| AppError::busy("Hay otra operación con archivos en curso en este servidor."))
}

#[tauri::command]
pub async fn files_list(app: AppHandle, cluster: State<'_, Cluster>, id: String, path: String) -> AppResult<Listing> {
    host939_core::instance::load(&id)?;
    if let Some((_, name)) = remote_host(&cluster, &id).await {
        let entries = cluster.node().await?.files_list(&id, &path).await.map_err(rerr)?;
        return Ok(Listing { active: entries.iter().any(|e| e.locked && e.kind == "world"), entries, remote: true, host_name: Some(name) });
    }
    let a = active(&app, &id);
    Ok(Listing { entries: lf::list(&id, &path, a)?, remote: false, host_name: None, active: a })
}

#[tauri::command]
pub async fn files_search(app: AppHandle, cluster: State<'_, Cluster>, id: String, q: String) -> AppResult<Vec<lf::Entry>> {
    if remote_host(&cluster, &id).await.is_some() { return cluster.node().await?.files_search(&id, &q).await.map_err(rerr); }
    lf::search(&id, &q, active(&app, &id), 200)
}

#[tauri::command]
pub async fn files_mkdir(app: AppHandle, cluster: State<'_, Cluster>, id: String, path: String) -> AppResult<()> {
    if remote_host(&cluster, &id).await.is_some() { return cluster.node().await?.files_mkdir(&id, &path).await.map_err(rerr); }
    let _g = lock(&id)?;
    lf::mkdir(&id, &path, active(&app, &id))
}

#[tauri::command]
pub async fn files_delete(app: AppHandle, cluster: State<'_, Cluster>, id: String, paths: Vec<String>) -> AppResult<()> {
    let remote = remote_host(&cluster, &id).await.is_some();
    for p in &paths {
        if remote { cluster.node().await?.files_delete(&id, p).await.map_err(rerr)?; } else { let _g = lock(&id)?; lf::delete(&id, p, active(&app, &id))?; }
    }
    let _ = app.emit("trash-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn files_transfer(app: AppHandle, cluster: State<'_, Cluster>, id: String, from: String, to: String, mv: bool, replace: bool) -> AppResult<()> {
    if remote_host(&cluster, &id).await.is_some() { return cluster.node().await?.files_transfer(&id, &from, &to, mv, replace).await.map_err(rerr); }
    let _g = lock(&id)?;
    lf::transfer(&id, &from, &to, mv, replace, active(&app, &id))
}

#[tauri::command]
pub fn files_cancel() { CANCEL.store(true, Ordering::SeqCst); }

fn progress(app: &AppHandle, id: &str, op: &str, name: &str, done: u64, total: u64) {
    let _ = app.emit("files-progress", serde_json::json!({ "id": id, "op": op, "name": name, "done": done, "total": total }));
}

/// Archivos de una carpeta local, con su ruta relativa (para subir carpetas enteras).
fn walk(base: &Path, p: &Path, out: &mut Vec<(PathBuf, String)>) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() { walk(base, &e.path(), out); }
    } else {
        let rel = p.strip_prefix(base.parent().unwrap_or(base)).unwrap_or(p).to_string_lossy().replace('\\', "/");
        out.push((p.to_path_buf(), rel));
    }
}

/// Trae archivos o carpetas de esta PC a la carpeta `dest` del servidor.
#[tauri::command]
pub async fn files_import(app: AppHandle, cluster: State<'_, Cluster>, id: String, sources: Vec<String>, dest: String, replace: bool) -> AppResult<Vec<String>> {
    CANCEL.store(false, Ordering::SeqCst);
    let srcs: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
    if remote_host(&cluster, &id).await.is_some() {
        let node = cluster.node().await?;
        let mut files = vec![];
        for s in &srcs { walk(s, s, &mut files); }
        let total: u64 = files.iter().map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
        let mut done_bytes = 0u64;
        let mut done = vec![];
        for (p, rel) in files {
            let target = if dest.is_empty() { rel.clone() } else { format!("{}/{rel}", dest.trim_end_matches('/')) };
            let base = done_bytes;
            let app2 = app.clone(); let id2 = id.clone(); let name = rel.clone();
            node.files_upload(&id, &p, &target, replace, move |d, _| { progress(&app2, &id2, "upload", &name, base + d, total); !CANCEL.load(Ordering::SeqCst) }).await.map_err(rerr)?;
            done_bytes += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            done.push(target);
        }
        return Ok(done);
    }
    let _g = lock(&id)?;
    let a = active(&app, &id);
    progress(&app, &id, "import", "", 0, 1);
    let id2 = id.clone();
    let r = tokio::task::spawn_blocking(move || lf::import(&id2, &srcs, &dest, replace, a)).await.map_err(|e| AppError::new("internal", "Se interrumpió").with_detail(e))?;
    progress(&app, &id, "import", "", 1, 1);
    r
}

/// Lleva un archivo o carpeta del servidor a una carpeta de esta PC.
#[tauri::command]
pub async fn files_export(app: AppHandle, cluster: State<'_, Cluster>, id: String, path: String, dest_dir: String) -> AppResult<String> {
    CANCEL.store(false, Ordering::SeqCst);
    let dest = PathBuf::from(&dest_dir);
    if !dest.is_dir() { return Err(AppError::invalid("Elegí una carpeta de destino.")); }
    if remote_host(&cluster, &id).await.is_some() {
        let node = cluster.node().await?;
        // Lista recursiva en la anfitriona; se baja archivo por archivo.
        let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "servidor".into());
        let mut out_root = dest.join(&name);
        let mut n = 2;
        while out_root.exists() { out_root = dest.join(format!("{name} ({n})")); n += 1; }
        let mut queue = vec![path.clone()];
        let mut files: Vec<(String, u64)> = vec![];
        let parent = node.files_list(&id, Path::new(&path).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default().as_str()).await.map_err(rerr)?;
        let me = parent.iter().find(|e| e.path == path).ok_or_else(|| AppError::not_found("ese archivo"))?;
        if me.dir {
            while let Some(d) = queue.pop() {
                for e in node.files_list(&id, &d).await.map_err(rerr)? { if e.dir { queue.push(e.path); } else { files.push((e.path, e.size)); } }
            }
        } else { files.push((path.clone(), me.size)); }
        let total: u64 = files.iter().map(|f| f.1).sum();
        let mut base = 0u64;
        for (f, size) in files {
            let rel = Path::new(&f).strip_prefix(&path).map(|p| p.to_path_buf()).unwrap_or_default();
            let target = if rel.as_os_str().is_empty() { out_root.clone() } else { out_root.join(rel) };
            if let Some(p) = target.parent() { std::fs::create_dir_all(p).ok(); }
            let app2 = app.clone(); let id2 = id.clone(); let nm = f.clone(); let b = base;
            node.files_download(&id, &f, &target, move |d, _| { progress(&app2, &id2, "download", &nm, b + d, total); !CANCEL.load(Ordering::SeqCst) }).await.map_err(rerr)?;
            base += size;
        }
        return Ok(out_root.display().to_string());
    }
    let a = active(&app, &id);
    let id2 = id.clone();
    let out = tokio::task::spawn_blocking(move || lf::export(&id2, &path, &dest, a)).await.map_err(|e| AppError::new("internal", "Se interrumpió").with_detail(e))??;
    Ok(out.display().to_string())
}

/// Valida un .jar antes de agregarlo a mods/.
#[tauri::command]
pub fn files_check_jar(id: String, path: String) -> AppResult<lf::JarCheck> {
    let cfg = host939_core::instance::load(&id)?;
    Ok(lf::check_jar(&cfg, Path::new(&path)))
}

#[tauri::command]
pub fn files_open_local(id: String, path: String) -> AppResult<()> {
    let p = paths::server_run_dir(&id).join(lf::clean(&path)?);
    let target = if p.is_dir() { p } else { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) };
    crate::commands::open_path(&target.display().to_string())
}
