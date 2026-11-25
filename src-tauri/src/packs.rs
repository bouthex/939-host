//! Panel de Resource packs (ver `host939_core::resourcepacks`) y el servidor local que entrega los
//! packs propios al Minecraft de esta PC.

use host939_core::error::{AppError, AppResult};
use host939_core::process::Manager;
use host939_core::resourcepacks as rp;
use host939_core::{instance, paths, snapshots};
use serde::Serialize;
use tauri::{AppHandle, Manager as _, State};

static LOCAL: std::sync::Mutex<Option<Result<(), String>>> = std::sync::Mutex::new(None);

/// Abre el servidor local de packs (127.0.0.1:39393). Los packs que esta PC no tiene se traen de la
/// PC anfitriona del grupo (verificados por SHA-1) y quedan en caché.
pub fn start_local_server(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::bind(("127.0.0.1", rp::LOCAL_PORT)).await {
            Ok(l) => {
                *LOCAL.lock().unwrap() = Some(Ok(()));
                let app2 = app.clone();
                let fetch: rp::Fetch = std::sync::Arc::new(move |server: String, sha1: String| {
                    let app = app2.clone();
                    Box::pin(async move {
                        let node = app.state::<crate::network::Cluster>().try_node().cloned().ok_or_else(|| AppError::invalid("La red del grupo no está lista."))?;
                        node.pack_fetch(&server, &sha1).await.map_err(|e| AppError::invalid(e.to_string()))
                    })
                });
                rp::serve(l, Some(fetch)).await;
            }
            Err(e) => { *LOCAL.lock().unwrap() = Some(Err(format!("El puerto local {} está ocupado por otro programa ({e}).", rp::LOCAL_PORT))); }
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    #[serde(flatten)]
    pub lib: rp::Library,
    /// Esta PC puede cambiar los packs (tiene el servidor y no lo aloja otra PC ahora).
    pub editable: bool,
    pub read_only_reason: Option<String>,
    /// Problema del servidor local de packs de esta PC, si lo hay.
    pub local_problem: Option<String>,
    pub running: bool,
}

fn editable(id: &str) -> Option<String> {
    if !instance::load(id).map(|c| c.installed).unwrap_or(false) { return Some("Los resource packs se administran en la PC que tiene el servidor.".into()); }
    if !snapshots::writes_line(id) { return Some("Este servidor lo aloja otra PC del grupo ahora: los resource packs se cambian desde esa PC.".into()); }
    None
}

#[tauri::command]
pub fn rp_library(id: String, mgr: State<'_, Manager>) -> AppResult<LibraryView> {
    if !paths::valid_server_id(&id) { return Err(AppError::invalid("Servidor inválido.")); }
    let lib = rp::library(&id)?;
    let why = editable(&id);
    let local_problem = LOCAL.lock().unwrap().clone().and_then(|r| r.err());
    Ok(LibraryView { lib, editable: why.is_none(), read_only_reason: why, local_problem, running: mgr.is_active(&id) })
}

fn check(id: &str) -> AppResult<()> {
    if !paths::valid_server_id(id) { return Err(AppError::invalid("Servidor inválido.")); }
    if let Some(why) = editable(id) { return Err(AppError::invalid(why)); }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// Nombre del archivo afectado.
    pub file: String,
    /// Pack que dejó de ser obligatorio (solo puede haber uno).
    pub demoted: Option<String>,
    /// Cambió lo que el servidor envía al entrar: se aplica al reiniciar el servidor.
    pub restart_needed: bool,
}

/// Vuelve a escribir server.properties con el pack del servidor que corresponda.
fn apply(id: &str, mgr: &Manager, before: Option<(String, rp::PackMeta)>) -> AppResult<bool> {
    let after = rp::required(id);
    let cfg = instance::load(id)?;
    host939_core::properties::write_for(&cfg)?;
    let changed = before.map(|(f, p)| (f, p.sha1)) != after.map(|(f, p)| (f, p.sha1));
    Ok(changed && mgr.is_active(id))
}

#[tauri::command]
pub async fn rp_search(id: String, query: String, all: bool, offset: u32) -> AppResult<serde_json::Value> {
    let cfg = instance::load(&id)?;
    let (hits, total) = rp::search(&query, &cfg.mc_version, all, offset).await?;
    Ok(serde_json::json!({ "hits": hits, "total": total, "mcVersion": cfg.mc_version }))
}

#[tauri::command]
pub async fn rp_install(id: String, project: String, allow_incompatible: bool, app: AppHandle) -> AppResult<Change> {
    check(&id)?;
    let before = rp::required(&id);
    let file = rp::install_modrinth(&id, &project, allow_incompatible).await?;
    let restart_needed = apply(&id, &app.state::<Manager>(), before)?;
    Ok(Change { file, demoted: None, restart_needed })
}

#[tauri::command]
pub fn rp_add_file(id: String, path: String, on_conflict: String, mgr: State<'_, Manager>) -> AppResult<Change> {
    check(&id)?;
    let before = rp::required(&id);
    let file = rp::add_file(&id, std::path::Path::new(&path), &on_conflict)?;
    let restart_needed = apply(&id, &mgr, before)?;
    Ok(Change { file, demoted: None, restart_needed })
}

#[tauri::command]
pub fn rp_remove(id: String, file: String, mgr: State<'_, Manager>) -> AppResult<Change> {
    check(&id)?;
    let before = rp::required(&id);
    rp::remove(&id, &file)?;
    let restart_needed = apply(&id, &mgr, before)?;
    Ok(Change { file, demoted: None, restart_needed })
}

#[tauri::command]
pub fn rp_set(id: String, file: String, mode: Option<String>, active: Option<bool>, mgr: State<'_, Manager>) -> AppResult<Change> {
    check(&id)?;
    let before = rp::required(&id);
    let demoted = rp::set(&id, &file, mode.as_deref(), active)?;
    let restart_needed = apply(&id, &mgr, before)?;
    Ok(Change { file, demoted, restart_needed })
}

/// Crea la carpeta resourcepacks del servidor si falta (para abrirla y cargar archivos).
#[tauri::command]
pub fn rp_open_folder(id: String) -> AppResult<()> {
    check(&id)?;
    let d = rp::dir(&id);
    std::fs::create_dir_all(&d).map_err(|e| AppError::io("No se pudo crear la carpeta resourcepacks", e))?;
    crate::commands::open_path(&d.to_string_lossy())
}
