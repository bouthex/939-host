//! Actualizaciones de 939 Host dentro de la app.
//!
//! - Desactivadas hasta que el propietario configure la distribución (`distribucion.json`: raíz pública
//!   y dirección). Sin eso no se consulta nada en internet.
//! - Con configuración: consulta al abrir (después de 1 minuto) y cada 6 horas (con un margen al azar),
//!   verifica firmas, canal, secuencia y vencimiento (crate `host939-update`), descarga en segundo plano
//!   de forma reanudable y avisa. Nunca instala sola: la persona elige «Instalar ahora» o «Más tarde», y
//!   aun así se espera a que no haya servidores encendidos ni operaciones en curso.
//! - Sin identificadores en las consultas: solo pedidos HTTPS de archivos públicos.

use host939_core::paths;
use host939_update::{chrono::Utc, Client, Decision, Release, State};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Deserialize)]
struct Distribution { raiz: Option<serde_json::Value>, url: Option<String> }

fn distribution() -> Option<(Vec<u8>, String)> {
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
pub struct View {
    pub enabled: bool,
    pub channel: String,
    pub current: String,
    /// "desactivado" | "al-dia" | "buscando" | "descargando" | "lista" | "bloqueada" | "error"
    pub status: String,
    pub message: String,
    pub available: Option<String>,
    pub notes: String,
    pub progress: Option<f64>,
    pub last_check: Option<String>,
}

static VIEW: Mutex<Option<View>> = Mutex::new(None);
static READY: Mutex<Option<(Release, std::path::PathBuf)>> = Mutex::new(None);

fn current_version() -> String { env!("CARGO_PKG_VERSION").to_string() }

fn set(app: &AppHandle, f: impl FnOnce(&mut View)) {
    let mut g = VIEW.lock().unwrap();
    let v = g.get_or_insert_with(|| View { enabled: distribution().is_some(), channel: load().channel, current: current_version(), status: "desactivado".into(), ..Default::default() });
    f(v);
    let _ = app.emit("update-status", v.clone());
}

/// Motivos por los que ahora no se puede instalar (servidores, guardados, copias, traspasos).
fn busy(app: &AppHandle) -> Vec<String> {
    let mut r = vec![];
    let active = app.state::<host939_core::process::Manager>().active_ids();
    if !active.is_empty() { r.push(format!("{} servidor(es) encendido(s)", active.len())); }
    for op in app.state::<crate::ops::Ops>().running().into_iter().filter(|o| o.state == "running") { r.push(op.title.clone()); }
    r
}

async fn get_limited(http: &host939_update::reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let resp = http.get(url).send().await.ok()?;
    if !resp.status().is_success() { return None; }
    if resp.content_length().map(|l| l as usize > host939_update::metadata::MAX_METADATA_BYTES).unwrap_or(false) { return None; }
    let b = resp.bytes().await.ok()?;
    (b.len() <= host939_update::metadata::MAX_METADATA_BYTES).then(|| b.to_vec())
}

/// Una consulta completa: raíz, canales, decisión y descarga.
pub async fn check(app: AppHandle) {
    let Some((root, base)) = distribution() else {
        set(&app, |v| { v.enabled = false; v.status = "desactivado".into(); v.message = "Las actualizaciones automáticas todavía no están configuradas por el proyecto. Las versiones nuevas se instalan a mano.".into(); });
        return;
    };
    set(&app, |v| { v.status = "buscando".into(); v.message = "Buscando actualizaciones…".into(); });
    let mut saved = load();
    let http = host939_update::reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).user_agent("939Host").build().unwrap();
    let mut client = match Client::new(&root, saved.state.clone()) {
        Ok(c) => c,
        Err(e) => { set(&app, |v| { v.status = "error".into(); v.message = format!("Configuración de actualizaciones inválida: {e}"); }); return; }
    };
    // Raíces nuevas (rotación o revocación de claves): se descargan en orden.
    let mut roots = std::collections::HashMap::new();
    for n in client.root.version + 1..client.root.version + 16 {
        match get_limited(&http, &format!("{base}/raiz-{n}.json")).await { Some(b) => { roots.insert(n, b); } None => break }
    }
    if let Err(e) = client.update_root(|n| roots.get(&n).cloned(), Utc::now()) {
        set(&app, |v| { v.status = "error".into(); v.message = format!("No se pudo verificar la identidad del proyecto: {e}. No se instala nada."); });
        return;
    }
    let stable = get_limited(&http, &format!("{base}/estable.json")).await;
    let beta = if saved.channel == "beta" { get_limited(&http, &format!("{base}/beta.json")).await } else { None };
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
            let Some(a) = r.artifacts.iter().find(|a| a.name.ends_with("-instalador.exe")).cloned() else { return };
            set(&app, |v| { v.status = "descargando".into(); v.available = Some(r.version.clone()); v.notes = r.notes.clone(); v.message = format!("Descargando 939 Host {}…", r.version); v.last_check = last; });
            let dir = paths::cache().join("actualizaciones");
            let url = format!("{base}/{}", a.name);
            let app2 = app.clone();
            let mut last_emit = std::time::Instant::now();
            match host939_update::download::fetch(&http, &url, &dir, &a, move |done, total| {
                if last_emit.elapsed().as_millis() > 500 { last_emit = std::time::Instant::now(); set(&app2, |v| v.progress = Some(done as f64 / total as f64)); }
            }).await {
                Ok(path) => {
                    *READY.lock().unwrap() = Some((r.clone(), path));
                    set(&app, |v| { v.status = "lista".into(); v.progress = None; v.message = format!("939 Host {} está lista para instalar.", r.version); });
                }
                Err(e) => set(&app, |v| { v.status = "error".into(); v.progress = None; v.message = format!("{e}"); }),
            }
        }
    }
}

/// Consulta al abrir y cada 6 horas (más un margen al azar de hasta 30 minutos).
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
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
    VIEW.lock().unwrap().clone().unwrap_or_default()
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> View {
    check(app.clone()).await;
    VIEW.lock().unwrap().clone().unwrap_or_default()
}

#[tauri::command]
pub fn update_channel(app: AppHandle, channel: String) -> Result<View, String> {
    if channel != "estable" && channel != "beta" { return Err("Canal desconocido.".into()); }
    let mut s = load();
    s.channel = channel.clone();
    save(&s);
    set(&app, |v| v.channel = channel);
    Ok(VIEW.lock().unwrap().clone().unwrap_or_default())
}

/// «Instalar ahora»: si hay algo en curso, devuelve los motivos y no hace nada. Si está libre, vuelve
/// a verificar el archivo, lanza el instalador (que espera a que la app se cierre) y cierra la app
/// con el cierre normal.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<Vec<String>, String> {
    let reasons = busy(&app);
    if !reasons.is_empty() { return Ok(reasons); }
    let Some((r, path)) = READY.lock().unwrap().clone() else { return Err("No hay ninguna actualización descargada.".into()) };
    let a = r.artifacts.iter().find(|a| a.name.ends_with("-instalador.exe")).cloned().ok_or("Falta el instalador.")?;
    host939_update::install::verify_before_launch(&path, &a).await.map_err(|e| e.to_string())?;
    std::process::Command::new(&path).args(host939_update::install::INSTALLER_ARGS).spawn().map_err(|e| format!("No se pudo abrir el instalador: {e}"))?;
    crate::begin_quit(&app);
    Ok(vec![])
}
