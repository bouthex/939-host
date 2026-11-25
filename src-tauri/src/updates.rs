//! Actualizaciones de 939 Host dentro de la app.
//!
//! - Fuente única: las releases oficiales de GitHub del proyecto (`distribucion.json`: raíz pública y
//!   dirección). Las compilaciones de prueba de la CI nunca tienen metadatos firmados, así que nunca se
//!   ofrecen.
//! - Al abrir se consulta enseguida (sin frenar la apertura) y después cada 6 horas. Se verifican
//!   firma, canal, secuencia y vencimiento (crate `host939-update`); si hay una versión nueva se
//!   descarga en segundo plano, de forma reanudable, con tamaño y SHA-256 firmados.
//! - Al abrir la app, si hay una versión lista, la interfaz muestra la pantalla de actualización y
//!   se instala sola (nunca con servidores encendidos ni operaciones en curso). Con la app en uso,
//!   se avisa y se instala cuando la persona toca «Actualizar ahora».
//! - Instalar: respaldo verificado de los archivos del programa, y un vigía (ver `vigia.rs`) que
//!   aplica la versión, la abre y vuelve a la anterior si no arranca. Los datos no se tocan.
//! - Sin identificadores en las consultas: solo pedidos HTTPS de archivos públicos.

use host939_core::paths;
use host939_update::{chrono::Utc, Client, Decision, Release, State};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Deserialize)]
struct Distribution { raiz: Option<serde_json::Value>, url: Option<String> }

fn distribution() -> Option<(Vec<u8>, String)> {
    // Solo en la compilación de pruebas de la CI: una distribución local (127.0.0.1) con una raíz de
    // prueba. Las releases se compilan sin esto y solo aceptan la fuente oficial por HTTPS.
    #[cfg(feature = "pruebas")]
    if let Some(p) = std::env::var_os("H939_ACTUALIZACIONES_PRUEBA") {
        let d: Distribution = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
        let url = d.url.filter(|u| u.starts_with("http://127.0.0.1:"))?;
        return Some((serde_json::to_vec(&d.raiz?).ok()?, url.trim_end_matches('/').to_string()));
    }
    let d: Distribution = serde_json::from_str(include_str!("../distribucion.json")).ok()?;
    let root = serde_json::to_vec(&d.raiz?).ok()?;
    let url = d.url.filter(|u| u.starts_with("https://"))?;
    Some((root, url.trim_end_matches('/').to_string()))
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default)]
    state: State,
    /// "estable" | "beta"
    #[serde(default = "estable")]
    channel: String,
    #[serde(default)]
    last_check: Option<String>,
}
fn estable() -> String { "estable".into() }

fn saved_path() -> std::path::PathBuf { paths::root().join("actualizaciones.json") }
fn load() -> Saved { std::fs::read(saved_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn save(s: &Saved) { let _ = paths::write_json(&saved_path(), s); }

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice { pub kind: String, pub text: String, pub version: String }

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub enabled: bool,
    pub channel: String,
    pub current: String,
    /// "instalada" | "portable"
    pub mode: String,
    /// "desactivado" | "buscando" | "al-dia" | "sin-conexion" | "descargando" | "verificando" | "lista"
    /// | "instalando" | "reiniciando" | "bloqueada" | "error"
    pub status: String,
    pub message: String,
    pub available: Option<String>,
    pub notes: String,
    pub progress: Option<f64>,
    pub last_check: Option<String>,
    /// Versión cuya instalación falló y se volvió atrás (no se reinstala sola al abrir).
    pub failed: Option<String>,
    /// Resultado de la última actualización, para mostrarlo una vez.
    pub notice: Option<Notice>,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);
static READY: Mutex<Option<(Release, std::path::PathBuf)>> = Mutex::new(None);
static CHECKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn current_version() -> String {
    // Solo pruebas: hacerse pasar por una versión anterior para recibir la actual como actualización.
    #[cfg(feature = "pruebas")]
    if let Ok(v) = std::env::var("H939_PRUEBA_VERSION_INSTALADA") { if !v.is_empty() { return v; } }
    env!("CARGO_PKG_VERSION").to_string()
}

fn app_dir() -> Option<std::path::PathBuf> { std::env::current_exe().ok()?.parent().map(|p| p.to_path_buf()) }

/// Instalada (con su desinstalador al lado) o portable.
fn mode() -> &'static str {
    if app_dir().map(|d| d.join("Desinstalar.exe").is_file()).unwrap_or(false) { "instalada" } else { "portable" }
}

fn package_suffix() -> &'static str { if mode() == "instalada" { "-instalador.exe" } else { "-portable.zip" } }

fn set(app: &AppHandle, f: impl FnOnce(&mut View)) {
    let mut g = VIEW.lock().unwrap();
    let v = g.get_or_insert_with(|| View { enabled: distribution().is_some(), channel: load().channel, current: current_version(), mode: mode().into(), status: "desactivado".into(), ..Default::default() });
    f(v);
    let _ = app.emit("update-status", v.clone());
}

fn view() -> View { VIEW.lock().unwrap().clone().unwrap_or_default() }

/// Motivos por los que ahora no se puede instalar (servidores, guardados, copias, traspasos).
fn busy(app: &AppHandle) -> Vec<String> {
    let mut r = vec![];
    let active = app.state::<host939_core::process::Manager>().active_ids();
    if !active.is_empty() { r.push(format!("{} servidor(es) encendido(s)", active.len())); }
    for op in app.state::<crate::ops::Ops>().running().into_iter().filter(|o| o.state == "running") { r.push(op.title.clone()); }
    r
}

/// Descarga un metadato chico. `Ok(None)`: el archivo no existe (por ejemplo, la release oficial
/// más reciente es anterior al sistema de actualizaciones). `Err`: sin conexión o servidor caído.
async fn get_meta(http: &host939_update::reqwest::Client, url: &str) -> Result<Option<Vec<u8>>, String> {
    let resp = http.get(url).send().await.map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 404 { return Ok(None); }
    if !resp.status().is_success() { return Err(format!("respuesta HTTP {}", resp.status())); }
    if resp.content_length().map(|l| l as usize > host939_update::metadata::MAX_METADATA_BYTES).unwrap_or(false) { return Err("metadato demasiado grande".into()); }
    let b = resp.bytes().await.map_err(|e| e.to_string())?;
    if b.len() > host939_update::metadata::MAX_METADATA_BYTES { return Err("metadato demasiado grande".into()); }
    Ok(Some(b.to_vec()))
}

/// Una consulta completa: raíz, canales, decisión y descarga. Si ya hay una en curso, no se repite.
pub async fn check(app: AppHandle) {
    if CHECKING.swap(true, std::sync::atomic::Ordering::SeqCst) { return; }
    check_inner(&app).await;
    CHECKING.store(false, std::sync::atomic::Ordering::SeqCst);
}

async fn check_inner(app: &AppHandle) {
    let app = app.clone();
    let Some((root, base)) = distribution() else {
        set(&app, |v| { v.enabled = false; v.status = "desactivado".into(); v.message = "Las actualizaciones automáticas no están configuradas en esta compilación.".into(); });
        return;
    };
    // Una versión ya descargada y verificada no se vuelve a buscar ni a bajar.
    if READY.lock().unwrap().is_some() && view().status == "lista" { return; }
    set(&app, |v| { v.status = "buscando".into(); v.message = "Buscando actualizaciones…".into(); v.progress = None; });
    let mut saved = load();
    let http = host939_update::reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(60)).user_agent("939Host").build().unwrap();
    let mut client = match Client::new(&root, saved.state.clone()) {
        Ok(c) => c,
        Err(e) => { set(&app, |v| { v.status = "error".into(); v.message = format!("Configuración de actualizaciones inválida: {e}"); }); return; }
    };
    let stable = match get_meta(&http, &format!("{base}/estable.json")).await {
        Ok(b) => b,
        Err(e) => {
            let last = saved.last_check.clone();
            set(&app, |v| { v.status = "sin-conexion".into(); v.message = format!("No se pudo comprobar si hay actualizaciones (sin conexión: {e}). Se vuelve a intentar más tarde."); v.last_check = last; });
            return;
        }
    };
    // Raíces nuevas (rotación o revocación de claves): se descargan en orden.
    let mut roots = std::collections::HashMap::new();
    for n in client.root.version + 1..client.root.version + 16 {
        match get_meta(&http, &format!("{base}/raiz-{n}.json")).await { Ok(Some(b)) => { roots.insert(n, b); } _ => break }
    }
    if let Err(e) = client.update_root(|n| roots.get(&n).cloned(), Utc::now()) {
        set(&app, |v| { v.status = "error".into(); v.message = format!("No se pudo verificar la identidad del proyecto: {e}. No se instala nada."); });
        return;
    }
    let beta = if saved.channel == "beta" { get_meta(&http, &format!("{base}/beta.json")).await.ok().flatten() } else { None };
    let data_version = host939_core::datos::read_version(paths::root());
    let decision = client.decide(&saved.channel, stable.as_deref(), beta.as_deref(), &current_version(), data_version, Utc::now());
    for (ch, b) in [("estable", &stable), ("beta", &beta)] { if let Some(b) = b { let _ = client.remember(ch, b, Utc::now()); } }
    saved.state = client.state.clone();
    saved.last_check = Some(chrono::Local::now().to_rfc3339());
    save(&saved);
    let last = saved.last_check.clone();
    match decision {
        Err(e) => set(&app, |v| { v.status = "error".into(); v.message = format!("Se rechazó la información de actualización: {e}. No se instala nada."); v.last_check = last; }),
        Ok(Decision::UpToDate) => set(&app, |v| { v.status = "al-dia".into(); v.message = "939 Host está al día.".into(); v.available = None; v.last_check = last; }),
        Ok(Decision::Blocked(m)) => set(&app, |v| { v.status = "bloqueada".into(); v.message = m; v.last_check = last; }),
        Ok(Decision::Offer(r)) => {
            let suffix = package_suffix();
            let Some(a) = r.artifacts.iter().find(|a| a.name.ends_with(suffix)).cloned() else {
                set(&app, |v| { v.status = "bloqueada".into(); v.message = format!("La versión {} no trae paquete para esta forma de instalación.", r.version); v.last_check = last; });
                return;
            };
            set(&app, |v| { v.status = "descargando".into(); v.available = Some(r.version.clone()); v.notes = r.notes.clone(); v.progress = Some(0.0); v.message = format!("Descargando 939 Host {}…", r.version); v.last_check = last; });
            let dir = paths::cache().join("actualizaciones");
            let url = format!("{base}/{}", a.name);
            let app2 = app.clone();
            let mut last_emit = std::time::Instant::now();
            let mut verifying = false;
            match host939_update::download::fetch(&http, &url, &dir, &a, move |done, total| {
                if done >= total && !verifying {
                    verifying = true;
                    set(&app2, |v| { v.status = "verificando".into(); v.progress = Some(1.0); v.message = "Verificando archivos…".into(); });
                } else if last_emit.elapsed().as_millis() > 200 {
                    last_emit = std::time::Instant::now();
                    set(&app2, |v| v.progress = Some(done as f64 / total as f64));
                }
            }).await {
                Ok(path) => {
                    *READY.lock().unwrap() = Some((r.clone(), path));
                    set(&app, |v| { v.status = "lista".into(); v.progress = None; v.message = format!("939 Host {} está lista para instalar.", r.version); });
                }
                Err(e) => set(&app, |v| { v.status = "error".into(); v.progress = None; v.message = format!("{e}. Se vuelve a intentar más tarde (la descarga continúa donde quedó)."); }),
            }
        }
    }
}

/// Resultado de una actualización anterior (o una que se cortó a la mitad) para mostrarlo una vez.
fn previous_result(app: &AppHandle) {
    let sp = crate::vigia::state_path(paths::root());
    let Some(mut p) = crate::vigia::read(&sp) else { return };
    let age = chrono::DateTime::parse_from_rfc3339(&p.at).map(|t| chrono::Local::now().signed_duration_since(t).num_seconds()).unwrap_or(i64::MAX);
    // Quedó a mitad de camino (por ejemplo, se apagó la PC) y el vigía ya no está.
    if ["preparada", "instalando", "abriendo"].contains(&p.phase.as_str()) && age > 6 * 60 {
        if p.to == current_version() { p.phase = "ok".into(); p.message = format!("939 Host se actualizó a {}.", p.to); }
        else { p.phase = "fallida".into(); p.message = format!("La actualización a {} se interrumpió (por ejemplo, porque se apagó la PC). Sigue la versión {} con tus datos intactos; podés reintentar.", p.to, current_version()); }
        p.shown = false;
        let _ = crate::vigia::write(&sp, &p);
    }
    let failed = matches!(p.phase.as_str(), "revertida" | "fallida").then(|| p.to.clone());
    let notice = (!p.shown && matches!(p.phase.as_str(), "ok" | "revertida" | "fallida")).then(|| Notice { kind: if p.phase == "ok" { "ok".into() } else { "error".into() }, text: p.message.clone(), version: p.to.clone() });
    set(app, |v| { v.failed = failed; v.notice = notice; });
}

/// Consulta al abrir (enseguida, sin frenar la apertura) y cada 6 horas (más un margen al azar de
/// hasta 30 minutos).
pub fn start(app: AppHandle) {
    previous_result(&app);
    tauri::async_runtime::spawn(async move {
        loop {
            check(app.clone()).await;
            let mut b = [0u8; 2];
            let _ = getrandom::getrandom(&mut b);
            let jitter = u16::from_le_bytes(b) as u64 % 1800;
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600 + jitter)).await;
        }
    });
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> View {
    set(&app, |_| {});
    view()
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> View {
    if CHECKING.load(std::sync::atomic::Ordering::SeqCst) {
        // Ya hay una consulta en curso (por ejemplo, la del arranque): se espera a que termine.
        for _ in 0..600 { if !CHECKING.load(std::sync::atomic::Ordering::SeqCst) { break; } tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
    } else {
        check(app.clone()).await;
    }
    view()
}

#[tauri::command]
pub fn update_channel(app: AppHandle, channel: String) -> Result<View, String> {
    if channel != "estable" && channel != "beta" { return Err("Canal desconocido.".into()); }
    let mut s = load();
    s.channel = channel.clone();
    save(&s);
    set(&app, |v| v.channel = channel);
    Ok(view())
}

/// La versión nueva confirma que abrió bien (la llama la interfaz cuando quedó usable). Devuelve el
/// aviso a mostrar, si lo hay.
#[tauri::command]
pub fn update_confirm(app: AppHandle) -> Option<Notice> {
    let sp = crate::vigia::state_path(paths::root());
    if let Some(mut p) = crate::vigia::read(&sp) {
        if p.phase == "abriendo" && p.to == current_version() {
            p.phase = "ok".into();
            p.message = format!("939 Host se actualizó a {}.", p.to);
            p.at = chrono::Local::now().to_rfc3339();
            p.shown = false;
            let _ = crate::vigia::write(&sp, &p);
            let n = Notice { kind: "ok".into(), text: p.message.clone(), version: p.to.clone() };
            set(&app, |v| { v.notice = Some(n.clone()); v.failed = None; });
            return Some(n);
        }
    }
    view().notice
}

#[tauri::command]
pub fn update_notice_seen(app: AppHandle) {
    let sp = crate::vigia::state_path(paths::root());
    if let Some(mut p) = crate::vigia::read(&sp) { p.shown = true; let _ = crate::vigia::write(&sp, &p); }
    set(&app, |v| v.notice = None);
}

/// Espacio libre en el disco de `p` (None si no se puede saber).
fn free_space(p: &std::path::Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    disks.iter().filter(|d| p.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()).map(|d| d.available_space())
}

/// «Actualizar ahora» (o la instalación automática al abrir). Si hay algo en curso, devuelve los
/// motivos y no hace nada. Si está libre: verifica de nuevo el paquete, respalda los archivos del
/// programa, abre el vigía y cierra la app con el cierre normal.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<Vec<String>, String> {
    let reasons = busy(&app);
    if !reasons.is_empty() { return Ok(reasons); }
    let Some((r, package)) = READY.lock().unwrap().clone() else { return Err("No hay ninguna actualización descargada.".into()) };
    let fail = |app: &AppHandle, m: String| { set(app, |v| { v.status = "lista".into(); v.message = m.clone(); }); m };
    set(&app, |v| { v.status = "instalando".into(); v.message = "Instalando actualización…".into(); });
    let a = r.artifacts.iter().find(|a| a.name.ends_with(package_suffix())).cloned().ok_or_else(|| fail(&app, "Falta el paquete de esta versión.".into()))?;
    host939_update::install::verify_before_launch(&package, &a).await.map_err(|e| fail(&app, e.to_string()))?;
    let dir = app_dir().ok_or_else(|| fail(&app, "No se encontró la carpeta del programa.".into()))?;
    let mode = mode();
    let data = paths::root().to_path_buf();
    let need = a.size.saturating_mul(3) + 200 * 1024 * 1024;
    for p in [&data, &dir] {
        if free_space(p).map(|f| f < need).unwrap_or(false) {
            return Err(fail(&app, format!("Falta espacio en el disco para actualizar (hacen falta unos {} MB libres).", need / (1024 * 1024))));
        }
    }
    // La portable se actualiza en su propia carpeta: tiene que poder escribirse.
    let probe = dir.join(".939host-prueba-escritura");
    if std::fs::write(&probe, b"x").is_err() {
        return Err(fail(&app, format!("No se puede escribir en la carpeta de 939 Host ({}). Movela a una carpeta tuya (por ejemplo, Documentos) o usá el instalador.", dir.display())));
    }
    let _ = std::fs::remove_file(&probe);
    let state = crate::vigia::state_path(&data);
    let backup = crate::vigia::backup_dir(&data);
    let files = crate::vigia::files_to_backup(mode, &dir, &package).map_err(|e| fail(&app, format!("No se pudo preparar el respaldo: {e}")))?;
    let (dir2, files2, backup2) = (dir.clone(), files.clone(), backup.clone());
    tokio::task::spawn_blocking(move || crate::vigia::backup(&dir2, &files2, &backup2)).await.map_err(|e| e.to_string())?
        .map_err(|e| fail(&app, format!("No se pudo respaldar la versión actual (no se cambió nada): {e}")))?;
    // El vigía es una copia de esta versión, con otro nombre (el instalador espera a que no haya
    // ningún 939Host.exe abierto).
    let vdir = crate::vigia::dir(&data);
    let vigia = vdir.join(if cfg!(windows) { "939Host-vigia.exe" } else { "939host-vigia" });
    std::fs::copy(backup.join(crate::vigia::APP_EXE), &vigia).map_err(|e| fail(&app, format!("No se pudo preparar la actualización: {e}")))?;
    if backup.join("WebView2Loader.dll").is_file() { let _ = std::fs::copy(backup.join("WebView2Loader.dll"), vdir.join("WebView2Loader.dll")); }
    let p = crate::vigia::Pending { from: current_version(), to: r.version.clone(), mode: mode.into(), app_dir: dir, package, files, phase: "preparada".into(), message: String::new(), at: chrono::Local::now().to_rfc3339(), shown: true };
    crate::vigia::write(&state, &p).map_err(|e| fail(&app, format!("No se pudo preparar la actualización: {e}")))?;
    let mut cmd = std::process::Command::new(&vigia);
    cmd.arg("--vigilar-actualizacion").arg(&state).arg(std::process::id().to_string()).current_dir(&vdir);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000 | 0x0000_0008); } // CREATE_NO_WINDOW | DETACHED_PROCESS
    cmd.spawn().map_err(|e| fail(&app, format!("No se pudo iniciar la actualización: {e}")))?;
    set(&app, |v| { v.status = "reiniciando".into(); v.message = "Reiniciando 939 Host…".into(); });
    // La interfaz avisa cuando ya mostró «Reiniciando 939 Host…» (así se leen todos los estados);
    // si no avisa en 6 segundos (por ejemplo, la ventana no responde), se cierra igual.
    RESTART_NOW.store(false, std::sync::atomic::Ordering::SeqCst);
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        for _ in 0..60 {
            if RESTART_NOW.load(std::sync::atomic::Ordering::SeqCst) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        crate::begin_quit(&app2);
    });
    Ok(vec![])
}

static RESTART_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// La pantalla de actualización ya mostró «Reiniciando 939 Host…».
#[tauri::command]
pub fn update_restart_now() { RESTART_NOW.store(true, std::sync::atomic::Ordering::SeqCst); }
