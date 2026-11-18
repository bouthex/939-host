//! Comandos que invoca la interfaz. Todo lo pesado corre fuera del hilo principal.

use host939_core::backup::{self, BackupInfo};
use host939_core::error::{AppError, AppResult};
use host939_core::hardware::{self, HardwareSnapshot, LiveMetrics, Probe};
use host939_core::identity::NodeIdentity;
use host939_core::instance;
use host939_core::java::{self, RuntimeInfo};
use host939_core::meta::{self, FabricSupport, VersionOption};
use host939_core::model::*;
use host939_core::paths;
use host939_core::process::{self, Manager};
use host939_core::settings::{self, AppSettings};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager as _, State};

fn total_ram(probe: &Probe) -> u64 { hardware::live(probe, 0).ram_total_mb }

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| AppError::new("internal", "Error interno").with_detail(e))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub node: NodeIdentity,
    pub settings: AppSettings,
}

#[tauri::command]
pub fn app_info(node: State<'_, NodeIdentity>) -> AppInfo {
    let s = settings::ensure_pc_identity();
    AppInfo { version: env!("CARGO_PKG_VERSION").into(), data_dir: paths::root().display().to_string(), node: node.inner().clone(), settings: settings::for_ui(s) }
}

#[tauri::command]
pub fn save_settings(settings: AppSettings) -> AppResult<AppSettings> {
    let mut s = settings;
    // El nombre y el símbolo de la PC se cambian con `pc_identity_set` (actualiza también el grupo).
    let saved = settings::load();
    s.node_name = saved.node_name;
    s.node_symbol = saved.node_symbol;
    // La clave de CurseForge se cambia solo con su comando (la interfaz nunca la tiene).
    s.curseforge_key = saved.curseforge_key.clone();
    s.invite_hours = s.invite_hours.clamp(1, 720);
    s.contribute_cpu_percent = s.contribute_cpu_percent.clamp(10, 100);
    // Los perfiles de reconexión se manejan con sus propios comandos.
    s.reconnect_profiles = settings::load().reconnect_profiles;
    if s.start_with_windows != settings::load().start_with_windows {
        settings::apply_autostart(s.start_with_windows)?;
    }
    settings::save(&s)?;
    Ok(settings::for_ui(s))
}

#[tauri::command]
pub async fn list_servers(app: AppHandle) -> AppResult<Vec<ServerView>> {
    blocking(move || {
        let mgr = app.state::<Manager>();
        let probe = app.state::<Probe>();
        Ok(instance::list_configs().into_iter().map(|c| instance::view(&mgr, &probe, c, false)).collect())
    }).await
}

#[tauri::command]
pub async fn get_server(app: AppHandle, id: String) -> AppResult<ServerView> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        Ok(instance::view(&app.state::<Manager>(), &app.state::<Probe>(), cfg, true))
    }).await
}

#[tauri::command]
pub async fn list_versions(include_snapshots: bool) -> AppResult<Vec<VersionOption>> {
    meta::list_versions(include_snapshots).await
}

#[tauri::command]
pub async fn fabric_support(mc_version: String) -> AppResult<FabricSupport> {
    meta::fabric_support(&mc_version).await
}

#[tauri::command]
pub async fn loader_support(loader: host939_core::model::Loader, mc_version: String) -> AppResult<meta::LoaderSupport> {
    meta::loader_support(loader, &mc_version).await
}

#[tauri::command]
pub async fn suggest_port() -> u16 {
    let used: Vec<u16> = instance::list_configs().iter().map(|c| c.port).collect();
    (25565..26000).find(|p| !used.contains(p) && std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok()).unwrap_or(25565)
}

#[tauri::command]
pub async fn create_server(app: AppHandle, input: CreateServerInput, probe: State<'_, Probe>) -> AppResult<ServerConfig> {
    let total = total_ram(&probe);
    let cfg = instance::create(input, total).await?;
    spawn_install(app, cfg.id.clone());
    Ok(cfg)
}

fn spawn_install(app: AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        let mgr = app.state::<Manager>();
        instance::install(&mgr, id).await;
    });
}

#[tauri::command]
pub async fn reinstall_server(app: AppHandle, id: String) -> AppResult<()> {
    if app.state::<Manager>().runtime(&id).lock().unwrap().is_active() {
        return Err(AppError::busy("Detené el servidor antes de reinstalarlo."));
    }
    let mut cfg = instance::load(&id)?;
    cfg.installed = false;
    cfg.install_error = None;
    instance::save(&cfg)?;
    spawn_install(app, id);
    Ok(())
}

#[tauri::command]
pub async fn update_server(app: AppHandle, config: ServerConfig) -> AppResult<ServerConfig> {
    blocking(move || {
        let total = total_ram(&app.state::<Probe>());
        let mgr = app.state::<Manager>();
        let before = instance::load(&config.id)?.java_choice;
        let cfg = instance::update(&mgr, config, total)?;
        mgr.changed(&cfg.id);
        // Cambió el Java elegido: se prepara ya (descarga verificada) para el próximo encendido.
        if before != cfg.java_choice && cfg.installed {
            let app2 = app.clone();
            let id = cfg.id.clone();
            tauri::async_runtime::spawn(async move { instance::install(&app2.state::<Manager>(), id).await; });
        }
        Ok(cfg)
    }).await
}

#[tauri::command]
pub async fn server_properties_extra(id: String) -> AppResult<std::collections::BTreeMap<String, String>> {
    let cfg = instance::load(&id)?;
    Ok(host939_core::properties::extra_from_disk(&cfg))
}

#[tauri::command]
pub async fn delete_server(app: AppHandle, id: String, confirm_name: String) -> AppResult<()> {
    blocking(move || instance::delete(&app.state::<Manager>(), &id, &confirm_name)).await
}

#[tauri::command]
pub async fn start_server(app: AppHandle, id: String) -> AppResult<()> {
    crate::network::before_start(&app, &id).await?;
    blocking(move || {
        let cfg = instance::load(&id)?;
        app.state::<Manager>().start(&app.state::<Probe>(), &cfg)
    }).await
}

#[tauri::command]
pub async fn stop_server(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || app.state::<Manager>().stop(&id)).await
}

#[tauri::command]
pub async fn kill_server(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || app.state::<Manager>().kill(&id)).await
}

#[tauri::command]
pub async fn restart_server(app: AppHandle, id: String) -> AppResult<()> {
    blocking(move || {
        let mgr = app.state::<Manager>();
        mgr.stop(&id)?;
        if !mgr.wait_inactive(&id, Duration::from_secs(100)) {
            return Err(AppError::new("timeout", "El servidor tardó demasiado en detenerse."));
        }
        let cfg = instance::load(&id)?;
        mgr.start(&app.state::<Probe>(), &cfg)
    }).await
}

#[tauri::command]
pub fn send_command(id: String, command: String, mgr: State<'_, Manager>) -> AppResult<()> {
    mgr.send_command(&id, &command)
}

#[tauri::command]
pub fn console_history(id: String, mgr: State<'_, Manager>) -> Vec<String> {
    mgr.console(&id)
}

#[tauri::command]
pub async fn list_backups(id: String) -> AppResult<Vec<BackupInfo>> {
    Ok(backup::list(&id))
}

#[tauri::command]
pub async fn create_backup(app: AppHandle, id: String, label: String) -> AppResult<BackupInfo> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        let mgr = app.state::<Manager>();
        let r = backup::create(&mgr, &id, &label, "manual", &cfg.mc_version);
        mgr.changed(&id);
        r
    }).await
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, id: String, file: String) -> AppResult<()> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        backup::restore(&app.state::<Manager>(), &id, &file, &cfg.mc_version)?;
        // La configuración de la app manda sobre la que traía el backup.
        host939_core::properties::write_for(&cfg)
    }).await
}

#[tauri::command]
pub async fn delete_backup(id: String, file: String) -> AppResult<()> {
    blocking(move || backup::delete(&id, &file)).await
}

#[tauri::command]
pub async fn import_world(app: AppHandle, id: String, path: String) -> AppResult<f64> {
    blocking(move || {
        let bytes = instance::import_world(&app.state::<Manager>(), &id, std::path::Path::new(&path))?;
        Ok(bytes as f64 / 1_048_576.0)
    }).await
}

#[tauri::command]
pub fn validate_world(path: String) -> AppResult<String> {
    instance::validate_world_source(std::path::Path::new(&path)).map(|p| p.display().to_string())
}

#[tauri::command]
pub fn open_folder(id: Option<String>, which: String) -> AppResult<()> {
    let path = match (id.as_deref(), which.as_str()) {
        (Some(id), "server") => paths::server_run_dir(id),
        (Some(id), "backups") => paths::server_backups_dir(id),
        (Some(id), "logs") => paths::server_run_dir(id).join("logs"),
        (Some(id), "mods") => paths::server_run_dir(id).join("mods"),
        (Some(id), "world") => host939_core::playerdata::world_dir(&instance::load(id)?),
        (None, "data") => paths::root().to_path_buf(),
        (None, "trash") => paths::trash(),
        (None, "applogs") => paths::root().join("logs"),
        _ => return Err(AppError::invalid("Carpeta desconocida")),
    };
    if let Some(id) = id.as_deref() { instance::load(id)?; }
    std::fs::create_dir_all(&path).ok();
    open_path(&path.display().to_string())
}

#[tauri::command]
pub fn open_link(url: String) -> AppResult<()> {
    if !link_allowed(&url) { return Err(AppError::invalid("Enlace no permitido")); }
    open_path(&url)
}

/// Solo https a dominios exactos conocidos (antes se comparaba el prefijo: `https://modrinth.com.ejemplo`
/// pasaba). Sin usuario ni puerto en la dirección, sin espacios ni comillas.
pub fn link_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    if url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\') { return false; }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, path) = (&rest[..end], &rest[end..]);
    if host.contains(['@', ':']) { return false; }
    match host.to_ascii_lowercase().as_str() {
        "aka.ms" => path == "/MinecraftEULA",
        "modrinth.com" | "fabricmc.net" => true,
        // Consulta de un mod en VirusTotal por su huella SHA-256: la abre la propietaria en su
        // navegador; no se sube el archivo ni hace falta una clave.
        "www.virustotal.com" => path.strip_prefix("/gui/file/").map(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod link_tests {
    #[test]
    fn exact_domains_only() {
        for ok in ["https://aka.ms/MinecraftEULA", "https://modrinth.com/mod/sodium", "https://fabricmc.net", "https://www.virustotal.com/gui/file/a3f1c0de5b2e4f7a9d8c6b5a4e3f2d1c0b9a8f7e6d5c4b3a2f1e0d9c8b7a6f5e"] { assert!(super::link_allowed(ok), "{ok}"); }
        for bad in ["https://www.virustotal.com/gui/home/upload", "https://www.virustotal.com/gui/file/abc", "https://www.virustotal.com/gui/file/a3f1c0de5b2e4f7a9d8c6b5a4e3f2d1c0b9a8f7e6d5c4b3a2f1e0d9c8b7a6f5e/../x"] { assert!(!super::link_allowed(bad), "{bad}"); }
        for bad in ["https://modrinth.com.ejemplo.com/x", "https://aka.ms/otra", "http://modrinth.com", "https://evil@modrinth.com", "https://modrinth.com:444/x", "https://fabricmc.net\"&calc", "file:///C:/Windows", "https://ejemplo.com/?https://modrinth.com"] { assert!(!super::link_allowed(bad), "{bad}"); }
    }
}

pub fn open_path(target: &str) -> AppResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new(host939_core::paths::windows_program("explorer.exe")).arg(target).spawn().map_err(|e| AppError::io("No se pudo abrir", e))?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(target).spawn().map_err(|e| AppError::io("No se pudo abrir", e))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn hardware_snapshot(app: AppHandle) -> AppResult<HardwareSnapshot> {
    blocking(move || Ok(hardware::snapshot(&app.state::<Probe>()))).await
}

#[tauri::command]
pub fn hardware_live(need_mb: Option<u64>, probe: State<'_, Probe>) -> LiveMetrics {
    hardware::live(&probe, need_mb.unwrap_or(0))
}

#[tauri::command]
pub async fn run_benchmark() -> AppResult<hardware::BenchmarkResult> {
    blocking(hardware::run_benchmark).await
}

#[tauri::command]
pub fn list_runtimes() -> Vec<RuntimeInfo> { java::list_installed() }

#[tauri::command]
pub fn jvm_preview(config: ServerConfig) -> String { process::jvm_args(&config).join(" ") }

#[tauri::command]
pub fn quit_app(app: AppHandle) { crate::begin_quit(&app); }

/// Cierra la ventana dejando los servidores funcionando (después de explicarlo una vez).
#[tauri::command]
pub fn close_to_background(app: AppHandle) -> AppResult<()> {
    let mut s = settings::load();
    if !s.background_hint_shown { s.background_hint_shown = true; settings::save(&s)?; }
    if let Some(w) = app.get_webview_window("main") { let _ = w.destroy(); }
    Ok(())
}

#[tauri::command]
pub fn notify_ui_ready(app: AppHandle) { let _ = app.emit("ui-ready", ()); }

/* ───────── Mods (Modrinth) ───────── */

use host939_core::modrinth::{Modrinth, SearchResult, Version};
use host939_core::mods::{self, InstallPlan, ModFileStatus};

fn mark_restart(app: &AppHandle, id: &str) {
    let mgr = app.state::<Manager>();
    let rt = mgr.runtime(id);
    let mut r = rt.lock().unwrap();
    if r.is_active() { r.needs_restart = true; }
    drop(r);
    mgr.changed(id);
}

#[tauri::command]
pub async fn mods_search(id: String, query: String, offset: u32) -> AppResult<SearchResult> {
    let cfg = instance::load(&id)?;
    let loader = mods::loader_id(cfg.loader)?;
    Modrinth.search(query.trim(), &cfg.mc_version, loader, true, offset).await
}

#[tauri::command]
pub async fn mods_versions(id: String, project_id: String) -> AppResult<Vec<Version>> {
    use host939_core::modrinth::Source;
    let cfg = instance::load(&id)?;
    let loader = mods::loader_id(cfg.loader)?;
    let mut list = Modrinth.project_versions(&project_id, &cfg.mc_version, loader).await?;
    list.retain(|v| v.supports(&cfg.mc_version, loader));
    list.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    Ok(list)
}

#[tauri::command]
pub async fn mods_plan(id: String, project_id: String, version_id: Option<String>) -> AppResult<InstallPlan> {
    let cfg = instance::load(&id)?;
    mods::plan_install(&Modrinth, &cfg, &project_id, version_id.as_deref()).await
}

/// Instala: el plan se recalcula acá (nunca se confía en URLs que mande la interfaz).
#[tauri::command]
pub async fn mods_install(app: AppHandle, id: String, project_id: String, version_id: Option<String>) -> AppResult<Vec<mods::InstalledMod>> {
    let cfg = instance::load(&id)?;
    mods_writable(&id)?;
    let plan = mods::plan_install(&Modrinth, &cfg, &project_id, version_id.as_deref()).await?;
    let ev = app.state::<Manager>().events.clone();
    let res = mods::apply_plan(&cfg, &plan, |msg, f| ev.mods_progress(&id, msg, f)).await?;
    mark_restart(&app, &id);
    Ok(res)
}

#[tauri::command]
pub async fn mods_list(id: String, online: bool) -> AppResult<Vec<ModFileStatus>> {
    let cfg = instance::load(&id)?;
    let list = mods::scan(&Modrinth, &cfg, online).await?;
    // Un archivo agregado desde la PC que Modrinth reconoce se vincula solo (antes había que tocar
    // «Vincular»): así recibe actualizaciones y se ve con su nombre. Los que no están en ningún
    // catálogo quedan como mods locales; funcionan igual.
    let to_link: Vec<String> = list.iter().filter(|x| !x.managed && x.project_id.is_some() && !x.issues.iter().any(|i| i == "duplicate" || i == "client_only")).map(|x| x.file_name.clone()).collect();
    if online && !to_link.is_empty() {
        let mut linked = false;
        for f in &to_link { linked |= mods::link_file(&Modrinth, &cfg, f).await.is_ok(); }
        if linked { return mods::scan(&Modrinth, &cfg, online).await; }
    }
    Ok(list)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModsSettings { pub auto_updates: bool }

#[tauri::command]
pub fn mods_settings(id: String) -> AppResult<ModsSettings> {
    instance::load(&id)?;
    Ok(ModsSettings { auto_updates: mods::load_manifest(&id).auto_updates })
}

#[tauri::command]
pub fn mods_set_auto_updates(id: String, on: bool) -> AppResult<()> {
    instance::load(&id)?;
    let mut m = mods::load_manifest(&id);
    m.auto_updates = on;
    mods::save_manifest(&id, &m)
}

/// Mods de un servidor compartido: solo los cambia la PC que lo aloja (en otra PC se pisarían con
/// el estado del grupo y, al llegar a las demás, quedarían en cuarentena).
fn mods_writable(id: &str) -> AppResult<()> {
    if snapshots::writes_line(id) { Ok(()) } else { Err(AppError::invalid("Este servidor lo aloja otra PC del grupo: los mods se cambian desde la PC anfitriona.")) }
}

/// Quita todos los mods (con punto de recuperación previo).
#[tauri::command]
pub async fn mods_remove_all(app: AppHandle, id: String) -> AppResult<Vec<String>> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        let mgr = app.state::<Manager>();
        if mgr.runtime(&id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de quitar los mods.")); }
        mods_writable(&id)?;
        mgr.note(&id, "Creando un punto de recuperación antes de quitar todos los mods…");
        if paths::server_run_dir(&id).join("world").join("level.dat").exists() {
            snapshots::take(&mgr, &id, "Antes de quitar todos los mods", "manual", &cfg.mc_version)?;
        }
        let moved = mods::remove_all(&cfg)?;
        mgr.note(&id, &format!("Se quitaron {} mods (están en la Papelera de 939 Host).", moved.len()));
        mark_restart(&app, &id);
        mgr.changed(&id);
        Ok(moved)
    }).await
}

/// Aplica las actualizaciones elegidas (ya revisadas en la interfaz) con un punto previo.
#[tauri::command]
pub async fn mods_update_apply(app: AppHandle, id: String, project_ids: Vec<String>) -> AppResult<Vec<String>> {
    let cfg = instance::load(&id)?;
    mods_writable(&id)?;
    {
        let (app2, id2, mc) = (app.clone(), id.clone(), cfg.mc_version.clone());
        blocking(move || {
            if paths::server_run_dir(&id2).join("world").join("level.dat").exists() {
                snapshots::take(&app2.state::<Manager>(), &id2, "Antes de actualizar mods", "manual", &mc)?;
            }
            Ok(())
        }).await?;
    }
    let ev = app.state::<Manager>().events.clone();
    let mut done = vec![];
    for pid in project_ids.iter().take(500) {
        // Se recalcula todo: versión compatible con esta versión de Minecraft y este loader,
        // dependencias y hash verificado (no se confía en lo que manda la interfaz).
        let plan = mods::plan_install(&Modrinth, &cfg, pid, None).await?;
        mods::apply_plan(&cfg, &plan, |msg, f| ev.mods_progress(&id, msg, f)).await?;
        done.extend(plan.items.iter().filter(|i| i.project_id == *pid).map(|i| format!("{} {}", i.title, i.version.version_number)));
    }
    mark_restart(&app, &id);
    Ok(done)
}

#[tauri::command]
pub async fn mods_remove(app: AppHandle, id: String, project_id: String) -> AppResult<Vec<String>> {
    let cfg = instance::load(&id)?;
    mods_writable(&id)?;
    let r = mods::remove(&cfg, &project_id)?;
    mark_restart(&app, &id);
    Ok(r)
}

#[tauri::command]
pub async fn mods_set_aside(app: AppHandle, id: String, file_name: String) -> AppResult<()> {
    let cfg = instance::load(&id)?;
    mods_writable(&id)?;
    mods::set_aside_file(&cfg, &file_name)?;
    mark_restart(&app, &id);
    Ok(())
}

#[tauri::command]
pub async fn mods_updates(id: String) -> AppResult<std::collections::HashMap<String, String>> {
    let cfg = instance::load(&id)?;
    Ok(mods::check_updates(&Modrinth, &cfg).await?.into_iter().map(|(k, v)| (k, v.version_number)).collect())
}


/* ───────── Puntos de restauración incrementales ───────── */

use host939_core::snapshots::{self, SnapshotInfo, StoreStats};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotsView { pub items: Vec<SnapshotInfo>, pub stats: StoreStats }

#[tauri::command]
pub async fn snapshots_list(id: String) -> AppResult<SnapshotsView> {
    blocking(move || { instance::load(&id)?; Ok(SnapshotsView { items: snapshots::list(&id), stats: snapshots::stats(&id) }) }).await
}

#[tauri::command]
pub async fn snapshot_take(app: AppHandle, id: String, label: String) -> AppResult<SnapshotInfo> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        let mgr = app.state::<Manager>();
        let info = snapshots::take(&mgr, &id, &label, "manual", &cfg.mc_version)?;
        mgr.note(&id, &format!("punto de restauración creado ({} nuevos, pausa {} ms)", human_bytes(info.new_bytes), info.pause_ms));
        Ok(info)
    }).await
}

#[tauri::command]
pub async fn snapshot_restore(app: AppHandle, id: String, snap_id: String) -> AppResult<()> {
    blocking(move || {
        let cfg = instance::load(&id)?;
        // En un servidor compartido solo restaura la PC anfitriona: en otra PC el resultado se
        // pisaría al alojar con el estado del grupo (y parecería que no hizo nada).
        if !snapshots::writes_line(&id) {
            return Err(AppError::invalid("Este servidor lo aloja otra PC del grupo. Para volver a un punto, primero pedí el servidor (o pedile a quien lo aloja que restaure)."));
        }
        let mgr = app.state::<Manager>();
        snapshots::restore(&mgr, &id, &snap_id, &cfg.mc_version)?;
        host939_core::properties::write_for(&cfg)?;
        // Lo restaurado pasa a ser el estado más reciente del servidor (y lo copian las demás PCs).
        if host939_core::network::load_share(&id).enabled {
            let label = snapshots::list(&id).into_iter().find(|s| s.id == snap_id).map(|s| format!("Restaurado: {}", s.label)).unwrap_or_else(|| "Restaurado".into());
            snapshots::take(&mgr, &id, &label, "auto", &cfg.mc_version)?;
        }
        Ok(())
    }).await
}

#[tauri::command]
pub async fn snapshot_delete(id: String, snap_id: String) -> AppResult<()> {
    blocking(move || snapshots::delete(&id, &snap_id)).await
}

pub fn human_bytes(b: u64) -> String {
    if b >= 1 << 20 { format!("{:.1} MB", b as f64 / 1_048_576.0) } else { format!("{:.1} KB", b as f64 / 1024.0) }
}

/// Programador: cada minuto revisa los servidores encendidos y toma un punto automático
/// cuando corresponde según su intervalo. Liviano: si no hay nada que hacer, no lee nada.
pub fn start_snapshot_scheduler(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        let mgr = app.state::<Manager>();
        for id in mgr.active_ids() {
            let running = mgr.runtime(&id).lock().unwrap().status() == RunStatus::Running;
            if !running { continue; }
            let Ok(cfg) = instance::load(&id) else { continue };
            // Compartido con otras PCs: los puntos también son la sincronización, así que se
            // toman al intervalo más corto de los dos.
            let shared = { let s = host939_core::network::load_share(&id); s.enabled && s.members.len() > 1 };
            let interval = if shared { cfg.restore_interval_min.min(cfg.sync_interval_min) } else { cfg.restore_interval_min };
            if !snapshots::auto_due(&id, interval.max(1)) { continue; }
            match snapshots::take(&mgr, &id, "", "auto", &cfg.mc_version) {
                Ok(info) => {
                    let pruned = snapshots::prune(&id);
                    mgr.note(&id, &format!("punto automático ({} nuevos, pausa {} ms{})", human_bytes(info.new_bytes), info.pause_ms,
                        if pruned > 0 { format!(", {pruned} viejos depurados") } else { String::new() }));
                }
                Err(e) if e.code == "busy" => {}
                Err(e) => mgr.note(&id, &format!("no se pudo crear el punto automático: {}", e.message)),
            }
            mgr.changed(&id);
        }
    });
}


/* ───────── Reconexión automática en el Minecraft de esta PC ───────── */

fn minecraft_dir() -> std::path::PathBuf {
    std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_default().join(".minecraft")
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientProfileView {
    #[serde(flatten)]
    pub profile: host939_core::client::ClientProfile,
    pub enabled: bool,
}

/// Perfiles Fabric/NeoForge del launcher oficial (solo lectura) y los ya activados.
#[tauri::command]
pub fn client_profiles() -> AppResult<Vec<ClientProfileView>> {
    let enabled = settings::load().reconnect_profiles;
    let mut out: Vec<ClientProfileView> = host939_core::client::detect_profiles(&minecraft_dir())
        .into_iter().map(|p| { let on = enabled.contains(&p); ClientProfileView { profile: p, enabled: on } }).collect();
    for p in enabled { if !out.iter().any(|v| v.profile == p) { out.push(ClientProfileView { profile: p, enabled: true }); } }
    Ok(out)
}

#[tauri::command]
pub async fn client_reconnect_enable(profile: host939_core::client::ClientProfile) -> AppResult<host939_core::client::EnsureReport> {
    let r = host939_core::client::ensure_reconnect(&host939_core::modrinth::Modrinth, &profile).await?;
    let mut s = settings::load();
    if !s.reconnect_profiles.contains(&profile) { s.reconnect_profiles.push(profile); settings::save(&s)?; }
    Ok(r)
}

#[tauri::command]
pub fn client_reconnect_disable(profile: host939_core::client::ClientProfile) -> AppResult<Vec<String>> {
    let removed = host939_core::client::remove_reconnect(&profile)?;
    let mut s = settings::load();
    s.reconnect_profiles.retain(|p| p != &profile);
    settings::save(&s)?;
    Ok(removed)
}

/// Al abrir la app: si alguien borró el mod de reconexión por error, se vuelve a poner.
pub fn recheck_client_reconnect() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        for p in settings::load().reconnect_profiles {
            let _ = host939_core::client::ensure_reconnect(&host939_core::modrinth::Modrinth, &p).await;
        }
    });
}

/* ───────── Jugadores, mundo y diagnóstico ───────── */

#[tauri::command]
pub fn players_list(id: String, mgr: State<'_, Manager>) -> AppResult<Vec<host939_core::players::PlayerView>> {
    let cfg = instance::load(&id)?;
    Ok(host939_core::players::list(&mgr, &cfg))
}

#[tauri::command]
pub async fn player_action(app: AppHandle, id: String, name: String, action: host939_core::players::PlayerAction, reason: Option<String>) -> AppResult<String> {
    let cfg = instance::load(&id)?;
    let mgr = app.state::<Manager>();
    host939_core::players::apply(&mgr, &cfg, &name, action, reason.as_deref()).await
}

#[tauri::command]
pub async fn world_info(id: String) -> AppResult<host939_core::diagnostics::WorldInfo> {
    let cfg = instance::load(&id)?;
    blocking(move || Ok(host939_core::diagnostics::world_info(&cfg))).await
}

#[tauri::command]
pub async fn world_new(app: AppHandle, id: String, seed: Option<String>, confirm_name: String) -> AppResult<()> {
    let mut cfg = instance::load(&id)?;
    if confirm_name.trim() != cfg.name { return Err(AppError::invalid("El nombre no coincide.")); }
    if host939_core::network::load_share(&id).enabled {
        return Err(AppError::invalid("Este servidor está compartido con el grupo: dejá de compartirlo antes de cambiar de mundo, para que ninguna PC siga con el anterior."));
    }
    blocking(move || host939_core::diagnostics::new_world(&app.state::<Manager>(), &mut cfg, seed)).await
}

#[tauri::command]
pub async fn diagnostics_report(app: AppHandle, id: String) -> AppResult<String> {
    let cfg = instance::load(&id)?;
    blocking(move || {
        let hw = serde_json::to_string_pretty(&hardware::snapshot(&app.state::<Probe>())).unwrap_or_default();
        let p = host939_core::diagnostics::report(&app.state::<Manager>(), &cfg, &hw)?;
        Ok(p.to_string_lossy().to_string())
    }).await
}

#[tauri::command]
pub async fn mods_import_folder(app: AppHandle, id: String, path: String) -> AppResult<host939_core::mods::ImportReport> {
    let cfg = instance::load(&id)?;
    if app.state::<Manager>().runtime(&id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de importar mods.")); }
    blocking(move || host939_core::mods::import_folder(&cfg, std::path::Path::new(&path))).await
}
