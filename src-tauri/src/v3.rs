//! Comandos del Diseño v3: controles del servidor (pausa, aviso con cuenta regresiva),
//! perfil de jugadores, skins, imágenes del juego, asistente de reparación y tienda de mods.

use host939_core::error::{AppError, AppResult};
use host939_core::hardware::{self, Probe};
use host939_core::instance;
use host939_core::model::RunStatus;
use host939_core::modrinth::{Modrinth, ProjectDetail, SearchResult};
use host939_core::process::Manager;
use host939_core::{gameassets, mods, playerdata, players, repair, settings, skins};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager as _};

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::new("internal", "Error interno").with_detail(e))?
}

fn online_players(app: &AppHandle, id: &str) -> usize {
    instance::load(id).map(|cfg| players::list(&app.state::<Manager>(), &cfg).iter().filter(|p| p.online).count()).unwrap_or(0)
}

/* ───────── Controles del servidor ───────── */

#[tauri::command]
pub async fn pause_server(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || { let cfg = instance::load(&id)?; app.state::<Manager>().pause(&cfg) }).await
}

#[tauri::command]
pub async fn resume_server(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || app.state::<Manager>().resume(&id)).await
}

/// Vuelve a comprobar que el servidor recibe jugadores (consulta real de Minecraft).
#[tauri::command]
pub async fn server_check_connection(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || { let cfg = instance::load(&id)?; app.state::<Manager>().check_connection(&id, cfg.port) }).await
}

/// Marca como leído el último problema de entrada.
#[tauri::command]
pub async fn server_clear_join_issue(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || { app.state::<Manager>().clear_join_issue(&id); Ok(()) }).await
}

/// Detiene con aviso: `seconds` = 0 detiene ya. Si se cancela la cuenta, devuelve false.
#[tauri::command]
pub async fn stop_server_warned(app: AppHandle, id: String, seconds: u32) -> AppResult<bool> {
    blocking(move || {
        let mgr = app.state::<Manager>();
        if seconds > 0 && !mgr.countdown(&id, seconds.min(300), "stop") { return Ok(false); }
        mgr.stop(&id)?;
        Ok(true)
    }).await
}

/// Reinicia (con aviso opcional): guarda, detiene, espera y vuelve a arrancar.
#[tauri::command]
pub async fn restart_server_warned(app: AppHandle, id: String, seconds: u32) -> AppResult<bool> {
    let app2 = app.clone();
    let id2 = id.clone();
    let ok = blocking(move || {
        let mgr = app2.state::<Manager>();
        if seconds > 0 && !mgr.countdown(&id2, seconds.min(300), "restart") { return Ok(false); }
        mgr.set_restarting(&id2, true);
        if let Err(e) = mgr.stop(&id2) { mgr.set_restarting(&id2, false); return Err(e); }
        if !mgr.wait_inactive(&id2, Duration::from_secs(100)) {
            mgr.set_restarting(&id2, false);
            return Err(AppError::new("timeout", "El servidor tardó demasiado en detenerse."));
        }
        Ok(true)
    }).await?;
    if !ok { return Ok(false); }
    let started = async {
        crate::network::before_start(&app, &id).await?;
        let a = app.clone();
        let i = id.clone();
        blocking(move || { let cfg = instance::load(&i)?; a.state::<Manager>().start(&a.state::<Probe>(), &cfg) }).await
    }.await;
    if let Err(e) = started { app.state::<Manager>().set_restarting(&id, false); return Err(e); }
    Ok(true)
}

#[tauri::command]
pub fn cancel_countdown(app: AppHandle, id: String) -> bool { app.state::<Manager>().cancel_countdown(&id) }

#[tauri::command]
pub fn players_online_count(app: AppHandle, id: String) -> usize { online_players(&app, &id) }

/* ───────── Jugadores ───────── */

#[tauri::command]
pub async fn player_detail(app: AppHandle, id: String, name: String) -> AppResult<playerdata::PlayerDetail> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        let mgr = app.state::<Manager>();
        let list = players::list(&mgr, &cfg);
        let p = list.iter().find(|p| p.name.eq_ignore_ascii_case(&name)).ok_or_else(|| AppError::not_found("el jugador"))?;
        playerdata::detail(&mgr, &cfg, &p.name, &p.uuid, p.online && mgr.runtime(&id).lock().unwrap().status() == RunStatus::Running)
    }).await
}

/// Skin del jugador como data URL; `None` si están desactivadas o no hay conexión (la interfaz
/// muestra las iniciales).
#[tauri::command]
pub async fn player_skin(name: String) -> AppResult<Option<String>> {
    if !settings::load().skins_enabled || !players::valid_name(&name) { return Ok(None); }
    Ok(skins::skin_png(&name).await.ok().map(|b| skins::data_url_png(&b)))
}

/* ───────── Imágenes originales del juego ───────── */

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssetsStatus { pub enabled: bool, pub ready: bool, pub version: String }

#[derive(Serialize, Clone)]
struct AssetsProgress<'a> { version: &'a str, message: &'a str, fraction: f64 }

#[tauri::command]
pub fn game_assets_status(version: String) -> AssetsStatus {
    AssetsStatus { enabled: settings::load().game_assets_enabled, ready: gameassets::ready(&version), version }
}

/// Descarga (una vez por versión) las imágenes del juego desde Mojang a esta PC.
#[tauri::command]
pub async fn game_assets_prepare(app: AppHandle, version: String) -> AppResult<AssetsStatus> {
    if !settings::load().game_assets_enabled { return Ok(game_assets_status(version)); }
    let v = version.clone();
    gameassets::ensure(&version, |m, f| { let _ = app.emit("assets-progress", AssetsProgress { version: &v, message: m, fraction: f }); }).await?;
    Ok(game_assets_status(version))
}

/// Imágenes pedidas por clave (`item:diamond_sword`, `hud:heart/full`, `effect:speed`, `slot:helmet`).
#[tauri::command]
pub async fn game_sprites(version: String, keys: Vec<String>) -> AppResult<HashMap<String, String>> {
    if !settings::load().game_assets_enabled || !gameassets::ready(&version) { return Ok(HashMap::new()); }
    blocking(move || {
        let mut out = HashMap::new();
        for k in keys.into_iter().take(400) {
            if let Some(p) = gameassets::resolve(&version, &k) {
                if let Ok(b) = std::fs::read(&p) { out.insert(k, skins::data_url_png(&b)); }
            }
        }
        Ok(out)
    }).await
}

/// Nombres en español de ítems y efectos (del idioma oficial del juego descargado).
#[tauri::command]
pub async fn game_names(version: String) -> AppResult<HashMap<String, String>> {
    if !settings::load().game_assets_enabled { return Ok(HashMap::new()); }
    blocking(move || Ok(gameassets::names(&version))).await
}

/// Borra las imágenes descargadas (Configuración → Privacidad).
#[tauri::command]
pub async fn game_assets_clear() -> AppResult<()> {
    blocking(|| {
        let d = host939_core::paths::cache().join("game-assets");
        if d.exists() { std::fs::remove_dir_all(&d).map_err(|e| AppError::io("No se pudieron borrar las imágenes", e))?; }
        Ok(())
    }).await
}

/* ───────── Asistente de reparación ───────── */

fn max_memory(app: &AppHandle) -> u32 { hardware::live(&app.state::<Probe>(), 0).ram_total_mb.saturating_sub(1536).max(1024) as u32 }

#[tauri::command]
pub async fn repair_analyze(app: AppHandle, id: String) -> AppResult<repair::Diagnosis> {
    let cfg = instance::load(&id)?;
    let max = max_memory(&app);
    let mgr = app.state::<Manager>();
    Ok(repair::analyze(&Modrinth, &mgr, &cfg, max, true).await)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    /// "ok": arrancó y llegó a «Done»; "failed": sigue sin arrancar; "skipped": no se verificó.
    pub verification: String,
    pub applied: Vec<String>,
    pub can_undo: bool,
    pub remaining: Option<repair::Diagnosis>,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
struct RepairProgress<'a> { id: &'a str, message: &'a str, fraction: f64 }

/// Aplica las reparaciones elegidas (el plan se recalcula acá: no se confía en la interfaz),
/// y después verifica arrancando el servidor.
#[tauri::command]
pub async fn repair_apply(app: AppHandle, id: String, problem_ids: Vec<String>, authorized: bool, verify: bool) -> AppResult<RepairResult> {
    let cfg = instance::load(&id)?;
    let max = max_memory(&app);
    let diag = {
        let mgr = app.state::<Manager>();
        repair::analyze(&Modrinth, &mgr, &cfg, max, true).await
    };
    let mut actions = vec![];
    for p in diag.problems.iter().filter(|p| problem_ids.contains(&p.id)) {
        let Some(fix) = &p.fix else { continue };
        if fix.needs_authorization && !authorized { return Err(AppError::invalid("Esta reparación necesita tu autorización.")); }
        for a in &fix.actions { if !actions.contains(a) { actions.push(a.clone()); } }
    }
    if actions.is_empty() { return Err(AppError::invalid("Ya no hay nada para reparar con lo elegido. Volvé a analizar.")); }
    let emit = |m: &str, f: f64| { let _ = app.emit("repair-progress", RepairProgress { id: &id, message: m, fraction: f }); };
    let rec = {
        let mgr = app.state::<Manager>();
        repair::apply(&Modrinth, &mgr, &cfg, &actions, emit).await?
    };
    app.state::<Manager>().changed(&id);
    if !verify { return Ok(RepairResult { verification: "skipped".into(), applied: rec.applied, can_undo: true, remaining: None, error: None }); }

    emit("Arrancando el servidor para comprobar", 0.92);
    let start = async {
        crate::network::before_start(&app, &id).await?;
        let a = app.clone();
        let i = id.clone();
        blocking(move || { let cfg = instance::load(&i)?; a.state::<Manager>().start(&a.state::<Probe>(), &cfg) }).await
    }.await;
    if let Err(e) = start {
        return Ok(RepairResult { verification: "failed".into(), applied: rec.applied, can_undo: true, remaining: None, error: Some(e.message.clone()) });
    }
    let a = app.clone();
    let i = id.clone();
    let status = blocking(move || {
        let mgr = a.state::<Manager>();
        let deadline = std::time::Instant::now() + Duration::from_secs(240);
        loop {
            let st = mgr.runtime(&i).lock().unwrap().status();
            if matches!(st, RunStatus::Running | RunStatus::Crashed | RunStatus::Stopped) || std::time::Instant::now() > deadline { return Ok(st); }
            std::thread::sleep(Duration::from_millis(500));
        }
    }).await?;
    if status == RunStatus::Running {
        emit("El servidor arrancó bien", 1.0);
        return Ok(RepairResult { verification: "ok".into(), applied: rec.applied, can_undo: true, remaining: None, error: None });
    }
    let cfg = instance::load(&id)?;
    let remaining = { let mgr = app.state::<Manager>(); repair::analyze(&Modrinth, &mgr, &cfg, max, true).await };
    let err = app.state::<Manager>().runtime(&id).lock().unwrap().last_error.clone();
    Ok(RepairResult { verification: "failed".into(), applied: rec.applied, can_undo: true, remaining: Some(remaining), error: err })
}

#[tauri::command]
pub async fn repair_undo(app: AppHandle, id: String) -> AppResult<()> {
    let a = app.clone();
    let i = id.clone();
    blocking(move || repair::undo(&a.state::<Manager>(), &i)).await?;
    app.state::<Manager>().changed(&id);
    Ok(())
}

#[tauri::command]
pub fn repair_last(id: String) -> Option<repair::RepairRecord> { repair::last_record(&id) }

/* ───────── Tienda de mods ───────── */

#[tauri::command]
pub async fn mods_explore(id: String, query: String, categories: Vec<String>, side: String, sort: String, offset: u32) -> AppResult<SearchResult> {
    let cfg = instance::load(&id)?;
    let loader = mods::loader_id(cfg.loader)?;
    Modrinth.search_filtered(query.trim(), &cfg.mc_version, loader, &categories, &side, &sort, offset).await
}

#[tauri::command]
pub async fn mods_project(project: String) -> AppResult<ProjectDetail> {
    if project.is_empty() || project.len() > 64 || !project.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(AppError::invalid("Proyecto inválido."));
    }
    Modrinth.project_detail(&project).await
}

#[tauri::command]
pub async fn mods_import_files(app: AppHandle, id: String, paths: Vec<String>) -> AppResult<mods::ImportReport> {
    let cfg = instance::load(&id)?;
    if app.state::<Manager>().runtime(&id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de importar mods.")); }
    let files: Vec<std::path::PathBuf> = paths.into_iter().take(200).map(std::path::PathBuf::from).collect();
    let r = blocking(move || mods::import_files(&cfg, &files)).await?;
    app.state::<Manager>().changed(&id);
    Ok(r)
}

#[tauri::command]
pub async fn mods_link(app: AppHandle, id: String, file_name: String) -> AppResult<mods::InstalledMod> {
    let cfg = instance::load(&id)?;
    let r = mods::link_file(&Modrinth, &cfg, &file_name).await?;
    app.state::<Manager>().changed(&id);
    Ok(r)
}


/* ───────── Papelera ───────── */

#[tauri::command]
pub async fn trash_list() -> AppResult<Vec<host939_core::trash::TrashItem>> {
    blocking(|| Ok(host939_core::trash::list())).await
}

#[tauri::command]
pub async fn trash_restore(app: AppHandle, key: String) -> AppResult<String> {
    let id = blocking(move || host939_core::trash::restore(&key)).await?;
    let _ = app.emit("servers-changed", ());
    Ok(id)
}

#[tauri::command]
pub async fn trash_purge(key: String) -> AppResult<()> {
    blocking(move || host939_core::trash::purge(&key)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmptyResult { pub removed: usize, pub freed_bytes: u64, pub errors: Vec<String> }

#[tauri::command]
pub async fn trash_empty() -> AppResult<EmptyResult> {
    blocking(|| { let (removed, freed_bytes, errors) = host939_core::trash::empty(); Ok(EmptyResult { removed, freed_bytes, errors }) }).await
}


/* ───────── Cambio de versión ───────── */

use host939_core::version_change as vc;

#[tauri::command]
pub async fn version_analyze(id: String, to: String) -> AppResult<vc::Analysis> {
    let cfg = instance::load(&id)?;
    vc::analyze(&Modrinth, &cfg, &to).await
}

#[tauri::command]
pub async fn version_last_change(id: String) -> AppResult<Option<vc::ChangeRecord>> { Ok(vc::last_change(&id)) }

/// Actualiza en el mismo servidor. Informa el avance con el evento `version-progress`.
#[tauri::command]
pub async fn version_update(app: AppHandle, id: String, to: String) -> AppResult<vc::ChangeReport> {
    let app2 = app.clone();
    let id2 = id.clone();
    let r = vc::update_in_place(&app.state::<Manager>(), &Modrinth, &id, &to, move |msg, f| {
        let _ = app2.emit("version-progress", serde_json::json!({ "id": id2, "message": msg, "fraction": f }));
    }).await;
    app.state::<Manager>().changed(&id);
    r
}

#[tauri::command]
pub async fn version_rollback(app: AppHandle, id: String) -> AppResult<()> {
    let r = vc::rollback(&app.state::<Manager>(), &id).await;
    app.state::<Manager>().changed(&id);
    r
}

/// Copia independiente (opción B). Si `to` viene, la copia queda en esa versión: si trae el mundo,
/// se actualiza con el mismo procedimiento seguro; si no, se instala directo en la versión nueva.
#[tauri::command]
pub async fn server_duplicate(app: AppHandle, id: String, name: String, parts: vc::CopyParts, to: Option<String>) -> AppResult<String> {
    let total = hardware::live(&app.state::<Probe>(), 0).ram_total_mb;
    let new_id = vc::duplicate(&id, &name, &parts, total).await?;
    if let Some(to) = to.filter(|t| !t.is_empty()) {
        let mut c = instance::load(&new_id)?;
        if parts.world && paths_has_world(&new_id) {
            // Primero se instala la versión original y después se actualiza con copia verificada.
            instance::install(&app.state::<Manager>(), new_id.clone()).await;
            let app2 = app.clone();
            let nid = new_id.clone();
            let r = vc::update_in_place(&app.state::<Manager>(), &Modrinth, &new_id, &to, move |msg, f| {
                let _ = app2.emit("version-progress", serde_json::json!({ "id": nid, "message": msg, "fraction": f }));
            }).await?;
            if !r.ok { return Err(AppError::new("version", "La copia se creó, pero no se pudo pasar a la versión nueva.").with_detail(r.error.unwrap_or_default())); }
        } else {
            let a = vc::analyze(&Modrinth, &c, &to).await?;
            c.mc_version = to;
            c.loader_version = a.loader_version;
            c.installed = false;
            instance::save(&c)?;
            let mgr = app.state::<Manager>();
            instance::install(&mgr, new_id.clone()).await;
        }
    } else {
        instance::install(&app.state::<Manager>(), new_id.clone()).await;
    }
    let _ = app.emit("servers-changed", ());
    Ok(new_id)
}

fn paths_has_world(id: &str) -> bool { host939_core::paths::server_run_dir(id).join("world").join("level.dat").exists() }


/* ───────── Logo del servidor ───────── */

#[tauri::command]
pub async fn server_logo_set(app: AppHandle, id: String, kind: String, big: Vec<u8>, icon: Vec<u8>) -> AppResult<()> {
    host939_core::logo::set(&id, &kind, &big, &icon)?;
    app.state::<Manager>().changed(&id);
    let _ = app.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn server_logo_clear(app: AppHandle, id: String) -> AppResult<()> {
    host939_core::logo::clear(&id)?;
    app.state::<Manager>().changed(&id);
    let _ = app.emit("servers-changed", ());
    Ok(())
}

/* ───────── Perfil para jugar (.mrpack) ───────── */

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileExport { report: host939_core::profile::ExportReport, launchers: Vec<&'static str> }

/// Genera el perfil en la carpeta Descargas del usuario. No toca ninguna instalación de Minecraft.
#[tauri::command]
pub async fn profile_export(id: String, include_manual: bool) -> AppResult<ProfileExport> {
    let cfg = instance::load(&id)?;
    let base = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from).map(|h| h.join("Downloads")).filter(|d| d.is_dir())
        .unwrap_or_else(|| host939_core::paths::root().join("Perfiles"));
    std::fs::create_dir_all(&base).map_err(|e| AppError::io("No se pudo crear la carpeta", e))?;
    let safe: String = cfg.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    let dest = base.join(format!("{} ({} {}).mrpack", safe.trim(), cfg.loader.name(), cfg.mc_version));
    let report = host939_core::profile::export_mrpack(&Modrinth, &id, &dest, include_manual).await?;
    Ok(ProfileExport { report, launchers: host939_core::profile::launchers() })
}

/// Abre el `.mrpack` con el programa asociado (Modrinth App, Prism…) o muestra la carpeta.
#[tauri::command]
pub fn profile_open(path: String, reveal: bool) -> AppResult<()> {
    let p = std::path::PathBuf::from(&path);
    if p.extension().and_then(|x| x.to_str()) != Some("mrpack") || !p.is_file() { return Err(AppError::invalid("Archivo de perfil inválido.")); }
    if reveal {
        #[cfg(windows)]
        { std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn().map_err(|e| AppError::io("No se pudo abrir la carpeta", e))?; return Ok(()); }
        #[cfg(not(windows))]
        { return crate::commands::open_path(&p.parent().unwrap().display().to_string()); }
    }
    crate::commands::open_path(&path)
}

/* ───────── Mods de rendimiento ───────── */

use host939_core::optimize;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizeOptions { candidates: Vec<optimize::Candidate>, recommended: Vec<String>, max: Vec<String> }

/// Candidatos para una versión y loader (sirve en el asistente, antes de que exista el servidor).
#[tauri::command]
pub async fn optimize_options(mc: String, loader: host939_core::model::Loader, id: Option<String>) -> AppResult<OptimizeOptions> {
    let installed: Vec<String> = id.map(|i| host939_core::mods::load_manifest(&i).mods.into_iter().map(|m| m.project_id).collect()).unwrap_or_default();
    let candidates = optimize::candidates(&Modrinth, &mc, loader, &installed).await?;
    Ok(OptimizeOptions { recommended: optimize::preset(&candidates, "recommended"), max: optimize::preset(&candidates, "max"), candidates })
}

/// Para el asistente: se instalan cuando termine la instalación del servidor.
#[tauri::command]
pub fn optimize_set_pending(id: String, slugs: Vec<String>) -> AppResult<()> { optimize::set_pending(&id, &slugs) }

/// Servidor existente: copia de seguridad (si ya tiene mundo) y después instala.
#[tauri::command]
pub async fn optimize_install(app: AppHandle, id: String, slugs: Vec<String>) -> AppResult<optimize::InstallReport> {
    let cfg = instance::load(&id)?;
    let mgr = app.state::<Manager>();
    if mgr.runtime(&id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor para instalar mods: se cargan al arrancar.")); }
    if host939_core::paths::server_run_dir(&id).join("world").join("level.dat").exists() {
        host939_core::backup::create(&mgr, &id, "Antes de instalar mods de rendimiento", "seguridad", &cfg.mc_version)?;
    }
    let app2 = app.clone();
    let id2 = id.clone();
    let r = optimize::install(&Modrinth, &cfg, &slugs, move |m, f| { let _ = app2.emit("version-progress", serde_json::json!({ "id": id2, "message": m, "fraction": f })); }).await;
    mgr.changed(&id);
    r
}
