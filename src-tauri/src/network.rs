//! Red del grupo en la app: un nodo por PC que mantiene vivo cada servidor compartido entre las
//! PCs del grupo (permiso de anfitrión, sincronización, traspasos y recuperación) y la dirección
//! fija para jugar (`127.0.0.1:<puerto>`), que siempre lleva a la PC anfitriona del momento.

use host939_cluster::{GroupNode, GroupStatus, LocalHost, Timing};
use host939_core::error::{AppError, AppResult};
use host939_core::hardware::Probe;
use host939_core::model::RunStatus;
use host939_core::network::{self, Invite};
use host939_core::process::Manager;
use host939_core::snapshots::{self, SnapshotInfo};
use host939_core::{identity, instance, paths, settings};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager as _, State};
use tokio::sync::OnceCell;

/// El nodo se abre en segundo plano al iniciar; los comandos esperan a que esté listo.
#[derive(Default)]
pub struct Cluster { node: OnceCell<GroupNode>, error: std::sync::Mutex<Option<String>> }

impl Cluster {
    pub async fn node(&self) -> AppResult<&GroupNode> {
        for _ in 0..100 {
            if let Some(n) = self.node.get() { return Ok(n); }
            if let Some(e) = self.error.lock().unwrap().clone() { return Err(AppError::new("p2p", format!("La red del grupo no pudo iniciar: {e}"))); }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(AppError::new("p2p", "La red del grupo todavía está iniciando. Probá de nuevo en unos segundos."))
    }
    pub fn try_node(&self) -> Option<&GroupNode> { self.node.get() }
}

/// Lo que el nodo necesita de esta app.
struct AppLocal(AppHandle, std::sync::Mutex<HashMap<(String, String), crate::ops::Op>>);

impl LocalHost for AppLocal {
    fn is_running(&self, server: &str) -> bool {
        matches!(self.0.state::<Manager>().runtime(server).lock().unwrap().status(), RunStatus::Running | RunStatus::Starting)
    }
    fn mc_port(&self, server: &str) -> u16 { instance::load(server).map(|c| c.port).unwrap_or(25565) }
    fn prepare(&self, server: &str) -> Result<(), String> {
        let cfg = instance::load(server).map_err(|e| e.message)?;
        if cfg.installed { return Ok(()); }
        let mgr = self.0.state::<Manager>();
        tauri::async_runtime::block_on(instance::install(&mgr, server.to_string()));
        let cfg = instance::load(server).map_err(|e| e.message)?;
        if cfg.installed { Ok(()) } else { Err(cfg.install_error.unwrap_or_else(|| "La instalación falló.".into())) }
    }
    fn start(&self, server: &str) -> Result<(), String> {
        let cfg = instance::load(server).map_err(|e| e.message)?;
        let mgr = self.0.state::<Manager>();
        mgr.start(&self.0.state::<Probe>(), &cfg).map_err(|e| e.message)?;
        let rt = mgr.runtime(server);
        let end = Instant::now() + Duration::from_secs(600);
        while Instant::now() < end {
            match rt.lock().unwrap().status() {
                RunStatus::Running => return Ok(()),
                RunStatus::Crashed | RunStatus::Stopped => return Err("El servidor se detuvo mientras iniciaba (mirá la consola).".into()),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        Err("El servidor tardó demasiado en iniciar.".into())
    }
    fn stop(&self, server: &str) -> Result<(), String> {
        let mgr = self.0.state::<Manager>();
        if !mgr.runtime(server).lock().unwrap().is_active() { return Ok(()); }
        mgr.stop(server).map_err(|e| e.message)?;
        if mgr.wait_inactive(server, Duration::from_secs(620)) { Ok(()) } else { Err("El servidor no se detuvo a tiempo.".into()) }
    }
    fn exited_unsaved(&self, server: &str) -> bool { self.0.state::<Manager>().take_exit_unsaved(server) }
    fn friend_message(&self, node: &str) { let _ = self.0.emit("friend-message", serde_json::json!({ "node": node })); }
    fn command(&self, server: &str, cmd: &str) -> Result<(), String> { self.0.state::<Manager>().send_command(server, cmd).map_err(|e| e.message) }
    fn run_state(&self, server: &str) -> (String, String, u32) {
        let rt = self.0.state::<Manager>().runtime(server);
        let r = rt.lock().unwrap();
        let players = r.players.len() as u32;
        let (st, detail) = match r.status() {
            RunStatus::Starting if r.restarting => ("restarting", r.phase.clone().unwrap_or_default()),
            RunStatus::Starting => ("starting", r.phase.clone().unwrap_or_default()),
            RunStatus::Running if r.saving => ("saving", String::new()),
            RunStatus::Running if r.reach.as_deref() == Some("ok") => (if players > 0 { "running" } else { "ready" }, String::new()),
            // La comprobación automática falló: el servidor está encendido igual (no «preparando»
            // para siempre en las demás PCs); se avisa el detalle.
            RunStatus::Running if r.reach.as_deref() == Some("fail") => (if players > 0 { "running" } else { "ready" }, format!("Encendido; la comprobación automática de conexión falló: {}", r.reach_error.clone().unwrap_or_default())),
            RunStatus::Running => ("preparing", "Comprobando la conexión".into()),
            RunStatus::Stopping if r.restarting => ("restarting", "Guardando y deteniendo".into()),
            RunStatus::Stopping => ("saving", "Guardando y deteniendo".into()),
            RunStatus::Crashed => ("error", r.last_error.clone().unwrap_or_default()),
            _ => ("stopped", String::new()),
        };
        (st.into(), detail, players)
    }
    fn player_op(&self, server: &str, op: &serde_json::Value, who: &str) -> Result<serde_json::Value, String> {
        crate::v3::player_op_local(&self.0, server, op, who).map_err(|e| e.message)
    }
    fn run_stats(&self, server: &str) -> Option<host939_cluster::RunStats> {
        let cfg = instance::load(server).ok()?;
        let rt = self.0.state::<Manager>().runtime(server);
        let (pid, mspt, p95, started, names) = {
            let r = rt.lock().unwrap();
            if !r.is_active() { return None; }
            (r.pid, r.mspt, r.mspt_p95, r.started_at, r.players.iter().cloned().collect::<Vec<_>>())
        };
        let usage = pid.and_then(|p| host939_core::hardware::process_usage(&self.0.state::<Probe>(), p));
        Some(host939_cluster::RunStats {
            mspt, mspt_p95: p95, memory_mb: usage.map(|u| u.0), memory_max_mb: cfg.memory_mb, cpu_pct: usage.map(|u| u.1),
            uptime_secs: started.map(|t| (chrono::Local::now() - t).num_seconds().max(0) as u64), max_players: cfg.max_players, player_names: names,
        })
    }
    fn op(&self, server: &str, title: &str, stage: &str, fraction: Option<f64>) {
        let mut m = self.1.lock().unwrap();
        let op = m.entry((server.to_string(), title.to_string())).or_insert_with(|| self.0.state::<crate::ops::Ops>().begin(&self.0, server, title, stage, false));
        op.stage(stage, fraction);
    }
    fn op_end(&self, server: &str, title: &str, error: Option<&str>) {
        if let Some(op) = self.1.lock().unwrap().remove(&(server.to_string(), title.to_string())) {
            match error { Some(e) => op.fail(e), None => op.done("Listo") }
        }
    }
    fn snapshot(&self, server: &str, label: &str, kind: &str) -> Result<SnapshotInfo, String> {
        let mc = instance::load(server).map(|c| c.mc_version).unwrap_or_default();
        snapshots::take(&self.0.state::<Manager>(), server, label, kind, &mc).map_err(|e| e.message)
    }
    fn score(&self) -> u32 { host939_core::hardware::host_score(&self.0.state::<Probe>()) }
    fn allow_hosting(&self, _: &str) -> bool { settings::load().allow_hosting }
    fn allow_bridge(&self) -> bool { settings::load().allow_relay }
    fn note(&self, server: &str, message: &str) {
        if instance::load(server).is_ok() { self.0.state::<Manager>().note(server, message); }
        let _ = self.0.emit("group-note", serde_json::json!({ "id": server, "message": message }));
    }
    fn changed(&self, server: &str) {
        let _ = self.0.emit("share-changed", serde_json::json!({ "id": server }));
        let _ = self.0.emit("server-changed", serde_json::json!({ "id": server }));
    }
    fn profile(&self) -> host939_cluster::HostProfile {
        let probe = self.0.state::<Probe>();
        let hw = host939_core::hardware::snapshot(&probe);
        let live = host939_core::hardware::live(&probe, 0);
        host939_cluster::HostProfile {
            cpu: hw.cpu_model.clone(), cores: hw.logical_cores as u32, ram_total_mb: live.ram_total_mb, ram_free_mb: live.ram_available_mb,
            cpu_score: hw.benchmark.as_ref().map(|b| b.cpu_single_score),
            disk_free_gb: hw.disks.iter().find(|d| d.is_data_disk).map(|d| d.free_gb),
        }
    }
    fn remote_event(&self, server: &str, kind: &str, host_name: &str, detail: &str) {
        let name = instance::load(server).map(|c| c.name).ok().or_else(|| network::load_share(server).card.as_ref().and_then(|c| c.get("name")).and_then(|v| v.as_str()).map(String::from)).unwrap_or_else(|| "El servidor".into());
        match kind {
            "ready" => {
                if settings::load().start_sound { host939_core::sound::play_start_chime(); }
                let _ = self.0.emit("group-note", serde_json::json!({ "id": server, "message": format!("{name} está en línea en la PC de {host_name}.") }));
            }
            _ => { let _ = self.0.emit("group-note", serde_json::json!({ "id": server, "message": format!("{name} tuvo un error en la PC de {host_name}{}", if detail.is_empty() { ".".into() } else { format!(": {detail}") }) })); }
        }
    }
    fn host_request(&self, server: &str, from_name: &str) {
        let _ = self.0.emit("host-request", serde_json::json!({ "id": server, "name": from_name }));
        // Hace titilar el botón en la barra de tareas si la ventana está abierta pero sin foco.
        if let Some(w) = self.0.get_webview_window("main") {
            if !w.is_focused().unwrap_or(false) { let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational)); }
        }
    }
}

fn node_name() -> String { settings::ensure_pc_identity().node_name }

fn p2p_err(context: &str, e: anyhow::Error) -> AppError {
    AppError::new("p2p", format!("{context}: {e}")).with_detail(format!("{e:#}"))
}

/* ── Puerto fijo para jugar ── */

fn routes_path() -> std::path::PathBuf { paths::root().join("routes.json") }
fn load_routes() -> HashMap<String, u16> { paths::read_json(&routes_path()).unwrap_or_default() }

/// Puerto local estable para un servidor: se deriva de su id (así suele ser el mismo en todas
/// las PCs del grupo) y se guarda para que nunca cambie en esta PC.
pub fn route_port(server: &str) -> u16 {
    let mut r = load_routes();
    if let Some(p) = r.get(server) { return *p; }
    let h = server.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let mut p = 41000 + (h % 900) as u16;
    while r.values().any(|x| *x == p) { p += 1; }
    r.insert(server.to_string(), p);
    let _ = paths::write_json(&routes_path(), &r);
    p
}

/// Abre el nodo y las direcciones fijas de los servidores en los que participa esta PC.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let cluster = app.state::<Cluster>();
        let r = async {
            let secret = identity::node_secret().map_err(|e| anyhow::anyhow!(e.message))?;
            let s = settings::load();
            // Modo privado: solo relays (gana sobre el ajuste de relays públicos, porque sin
            // relay no hay forma de conectarse sin mostrar la IP).
            let relays = if s.private_mode { host939_p2p::Relays::Private } else if s.allow_public_relays { host939_p2p::Relays::PublicFallback } else { host939_p2p::Relays::Disabled };
            GroupNode::start(secret, &node_name(), relays, Arc::new(AppLocal(app.clone(), Default::default())), Timing::default()).await
        }.await;
        match r {
            Ok(node) => {
                let s = settings::load();
                node.set_identity(&s.node_name, &s.node_symbol);
                for sid in network::shared_servers() {
                    let port = route_port(&sid);
                    if let Err(e) = node.open_route(&sid, port).await {
                        let _ = app.emit("group-note", serde_json::json!({ "id": sid, "message": format!("No se pudo abrir la dirección para jugar: {e}") }));
                    }
                }
                let _ = cluster.node.set(node);
            }
            Err(e) => { *cluster.error.lock().unwrap() = Some(format!("{e:#}")); }
        }
    });
}

/// Al salir: pasa o suelta los servidores que hostea esta PC (con aviso en la interfaz).
pub fn quit_blocking(app: &AppHandle) -> Vec<String> {
    let app = app.clone();
    tauri::async_runtime::block_on(async move {
        let cluster = app.state::<Cluster>();
        match cluster.try_node() { Some(n) => n.quit().await, None => vec![] }
    })
}

/// Antes de encender en esta PC: toma el permiso de anfitrión si corresponde.
pub async fn before_start(app: &AppHandle, id: &str) -> AppResult<()> {
    if !network::load_share(id).enabled { return Ok(()); }
    let node = app.state::<Cluster>().inner().node().await?.clone();
    node.prepare_start(id).await.map_err(|e| {
        let msg = e.to_string();
        // Pedido enviado a una anfitriona con el servidor apagado: no es un error, es un aviso.
        match msg.strip_prefix("PEDIDO:") { Some(m) => AppError::new("requested", m.to_string()), None => AppError::new("lease", msg) }
    })
}

/* ── Comandos ── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareView {
    pub enabled: bool,
    pub group: Option<GroupStatus>,
    pub invites: Vec<Invite>,
    pub route_port: Option<u16>,
    pub relays: bool,
    pub node_id: Option<String>,
    /// Esta PC está en modo privado (solo relays).
    pub private_mode: bool,
    /// El grupo tiene código de recuperación.
    pub recovery_set: bool,
    /// Hay un cambio de código pendiente (con espera).
    pub recovery_pending: bool,
    /// Sucesora designada (id de nodo).
    pub successor: Option<String>,
    /// Sucesión en curso: (id de la sucesora, segundos que faltan).
    pub succession: Option<(String, u64)>,
    /// Mods recibidos esperando aprobación: (nombre, huella, PC).
    pub pending_mods: Vec<(String, String, String)>,
    /// Política de mods: true = confiar en las coanfitrionas (no se aparta nada).
    pub trust_cohost_mods: bool,
}

#[tauri::command]
pub async fn share_status(id: String, cluster: State<'_, Cluster>) -> AppResult<ShareView> {
    let share = network::load_share(&id);
    let node = cluster.try_node();
    let group = match (share.enabled, node) { (true, Some(n)) => Some(n.status(&id).await), _ => None };
    Ok(ShareView {
        enabled: share.enabled, group, invites: share.invites,
        route_port: share.enabled.then(|| route_port(&id)), relays: settings::load().allow_public_relays,
        node_id: node.map(|n| n.id()),
        private_mode: settings::load().private_mode,
        recovery_set: share.recovery.is_some(), recovery_pending: share.recovery_pending.is_some(),
        successor: share.successor.clone(), succession: node.and_then(|n| n.succession_status(&id)),
        pending_mods: node.map(|n| n.pending_mods(&id)).unwrap_or_default(),
        trust_cohost_mods: share.mod_policy.as_deref() == Some("confiar-coanfitrionas"),
    })
}

#[tauri::command]
pub async fn share_set(id: String, enabled: bool, cluster: State<'_, Cluster>) -> AppResult<()> {
    let node = cluster.node().await?;
    if enabled {
        node.share(&id).map_err(|e| p2p_err("No se pudo compartir", e))?;
        node.open_route(&id, route_port(&id)).await.map_err(|e| p2p_err("No se pudo abrir la dirección para jugar", e))?;
    } else {
        let st = node.status(&id).await;
        if st.lease.as_ref().map(|l| !l.held_by(&st.me) && l.state == host939_core::lease::LeaseState::Hosting).unwrap_or(false) {
            return Err(AppError::invalid("Otra PC lo está hosteando ahora: primero pedile que te lo pase."));
        }
        let mut s = network::load_share(&id);
        s.enabled = false;
        network::save_share(&id, &s)?;
        node.close_route(&id).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn invite_create(id: String, role: String, label: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    let node = cluster.node().await?;
    if !network::load_share(&id).enabled {
        node.share(&id).map_err(|e| p2p_err("No se pudo compartir", e))?;
        let _ = node.open_route(&id, route_port(&id)).await;
    }
    node.invite_for(&id, &role, if label.trim().is_empty() { "Invitación" } else { label.trim() }, settings::load().invite_hours).map_err(|e| p2p_err("No se pudo crear la invitación", e))
}

/* ── Mods recibidos ── */

#[tauri::command]
pub async fn mods_approve(id: String, hashes: Vec<String>, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.approve_mods(&id, &hashes).map_err(|e| AppError::invalid(e.to_string()))
}

/// Una solicitud de mod para la propietaria: un archivo recibido de otra PC que espera decisión.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRequest {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    /// Nombre de la PC que lo mandó (o «esta PC» / desconocida).
    pub from: String,
    pub at: Option<String>,
    /// El archivo está en esta PC (si no, se analiza cuando llegue).
    pub here: bool,
    /// PC donde está apartado, si no es esta.
    pub held_by: Option<String>,
    pub rejected: bool,
    pub analysis: Option<host939_core::modscan::ModReport>,
}

/// Mods apartados esperando decisión, con su análisis (sin ejecutarlos). Los de esta PC se
/// analizan acá; los que están apartados en otra PC se muestran sin análisis: un análisis hecho
/// por otra PC no se puede verificar, y el archivo se analiza cuando llega a esta.
#[tauri::command]
pub async fn mod_requests(id: String, cluster: State<'_, Cluster>) -> AppResult<Vec<ModRequest>> {
    let share = network::load_share(&id);
    let name_of = |node: &str| share.members.iter().find(|m| m.node_id == node).map(|m| m.name.clone()).unwrap_or_else(|| "una PC que ya no está en el grupo".into());
    let mut out = vec![];
    for q in host939_core::modguard::quarantined(&id) {
        let analysis = match host939_core::modguard::quarantined_path(&id, &q.sha256) {
            Some(p) => host939_core::modscan::analyze_cached(&p).await.ok(),
            None => None,
        };
        out.push(ModRequest {
            from: q.from.as_deref().map(name_of).unwrap_or_else(|| "otra PC del grupo".into()),
            name: q.name, sha256: q.sha256, size: q.size, at: q.at, here: true, held_by: None, rejected: q.rejected, analysis,
        });
    }
    if let Some(node) = cluster.try_node() {
        for (name, h, pc) in node.pending_mods(&id) {
            if out.iter().any(|r| r.sha256 == h) { continue; }
            out.push(ModRequest { name, sha256: h, size: 0, from: "otra PC del grupo".into(), at: None, here: false, held_by: Some(pc), rejected: false, analysis: None });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn mods_reject(id: String, hashes: Vec<String>, discard: bool, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.reject_mods(&id, &hashes, discard).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub fn mod_decisions(id: String) -> Vec<host939_core::modguard::Decision> {
    let mut v = host939_core::modguard::decisions(&id);
    v.reverse();
    v
}

#[tauri::command]
pub async fn mods_policy(id: String, trust_cohosts: bool, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.set_mod_policy(&id, trust_cohosts).map_err(|e| AppError::invalid(e.to_string()))
}

/* ── Recuperación de la propiedad ── */

#[tauri::command]
pub async fn recovery_create(id: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    cluster.node().await?.create_recovery_code(&id).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn recovery_rotate(id: String, current: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    cluster.node().await?.rotate_recovery_code(&id, &current).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn recovery_reset_request(id: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    cluster.node().await?.request_recovery_reset(&id).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn recovery_reset_finish(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.finish_recovery_reset(&id).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn recovery_use(app: AppHandle, id: String, code: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    let r = cluster.node().await?.clone().recover_with_code(&id, &code).await.map_err(|e| AppError::invalid(e.to_string()))?;
    let _ = app.emit("servers-changed", ());
    Ok(r)
}

#[tauri::command]
pub async fn successor_set(id: String, node_id: Option<String>, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.set_successor(&id, node_id.as_deref()).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn succession_start(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.clone().start_succession(&id).await.map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn succession_cancel(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.cancel_succession(&id).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn succession_complete(app: AppHandle, id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.clone().complete_succession(&id).await.map_err(|e| AppError::invalid(e.to_string()))?;
    let _ = app.emit("servers-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn invite_revoke(id: String, secret: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.revoke_invite(&id, &secret).map_err(|e| p2p_err("No se pudo revocar", e))
}

#[tauri::command]
pub async fn member_remove(id: String, node_id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.remove_member(&id, &node_id).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn group_transfer(app: AppHandle, id: String, node_id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.transfer_ownership(&id, &node_id).await.map_err(|e| AppError::invalid(e.to_string()))?;
    let _ = app.emit("servers-changed", ());
    Ok(())
}

/// Sale del grupo. Devuelve si la propietaria lo confirmó en el momento.
#[tauri::command]
pub async fn group_leave(app: AppHandle, id: String, cluster: State<'_, Cluster>) -> AppResult<bool> {
    let r = cluster.node().await?.leave_group(&id).await.map_err(|e| AppError::invalid(e.to_string()))?;
    let _ = app.emit("servers-changed", ());
    Ok(r)
}

#[tauri::command]
pub async fn group_options(id: String, auto_failover: bool, handover_on_quit: bool, cluster: State<'_, Cluster>) -> AppResult<()> {
    cluster.node().await?.set_options(&id, auto_failover, handover_on_quit).map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn group_handover(id: String, target: Option<String>, cluster: State<'_, Cluster>) -> AppResult<String> {
    let node = cluster.node().await?.clone();
    node.handover(&id, target.as_deref()).await.map_err(|e| AppError::new("handover", e.to_string()))
}

#[tauri::command]
pub async fn group_mc_op(id: String, node: String, mc_name: Option<String>, cluster: State<'_, Cluster>) -> AppResult<()> {
    let n = cluster.node().await?;
    match mc_name {
        Some(name) => n.grant_mc_op(&id, &node, name.trim()),
        None => n.revoke_mc_op(&id, &node),
    }.map_err(|e| AppError::invalid(e.to_string()))
}

#[tauri::command]
pub async fn group_release(id: String, cluster: State<'_, Cluster>) -> AppResult<usize> {
    let node = cluster.node().await?.clone();
    node.release(&id).await.map_err(|e| AppError::new("release", e.to_string()))
}

#[tauri::command]
pub async fn group_take_here(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    let node = cluster.node().await?.clone();
    node.take_here(&id).await.map_err(|e| AppError::new("take", e.to_string()))
}

#[tauri::command]
pub async fn group_sync(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    let node = cluster.node().await?.clone();
    node.sync_now(&id).await;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedView {
    pub id: String,
    /// Logo del servidor (64×64) tal como lo publicó la propietaria.
    pub logo: Option<String>,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub has_copy: bool,
    pub loader_version: Option<String>,
    pub route_port: u16,
    pub host_name: String,
    pub host_online: bool,
    pub i_am_host: bool,
}

/// Servidores de otras PCs a los que se unió esta PC (como jugador o coanfitriona).
#[tauri::command]
pub async fn remote_list(cluster: State<'_, Cluster>) -> AppResult<Vec<JoinedView>> {
    let me = cluster.try_node().map(|n| n.id()).unwrap_or_default();
    let mut out = vec![];
    for sid in network::shared_servers() {
        let share = network::load_share(&sid);
        let owner = share.members.iter().find(|m| m.role == "propietario").map(|m| m.node_id.clone()).unwrap_or_default();
        if owner == me { continue; }
        let card = share.card.clone().unwrap_or_default();
        let get = |k: &str| card.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let (host_name, host_online, i_am_host) = match cluster.try_node() {
            Some(n) => { let st = n.status(&sid).await; (st.lease.as_ref().map(|l| l.host_name.clone()).unwrap_or_default(), st.host_online, st.i_am_host) }
            None => (String::new(), false, false),
        };
        out.push(JoinedView {
            id: sid.clone(), name: get("name"), mc_version: get("mcVersion"), loader: get("loader"), loader_version: Some(get("loaderVersion")).filter(|v| !v.is_empty()), has_copy: instance::load(&sid).is_ok(),
            route_port: route_port(&sid), host_name, host_online, i_am_host,
            logo: card.get("logo").and_then(|v| v.as_str()).filter(|u| u.starts_with("data:image/png;base64,")).map(String::from),
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn remote_join(app: AppHandle, ticket: String, cluster: State<'_, Cluster>) -> AppResult<JoinedView> {
    let node = cluster.node().await?.clone();
    let sid = node.join(ticket.trim()).await.map_err(|e| p2p_err("No se pudo unir", e))?;
    node.open_route(&sid, route_port(&sid)).await.map_err(|e| p2p_err("No se pudo abrir la dirección para jugar", e))?;
    // Coanfitriona: dejar el servidor instalado en segundo plano para poder recibirlo.
    if let Ok(cfg) = instance::load(&sid) {
        if !cfg.installed {
            let app2 = app.clone();
            let id2 = sid.clone();
            tauri::async_runtime::spawn(async move { instance::install(&app2.state::<Manager>(), id2).await; });
        }
    }
    let list = remote_list(cluster).await?;
    list.into_iter().find(|j| j.id == sid).ok_or_else(|| AppError::new("p2p", "Unido, pero no se encontró el servidor."))
}

#[tauri::command]
pub async fn remote_remove(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    let node = cluster.node().await?;
    let st = node.status(&id).await;
    if st.i_am_host { return Err(AppError::invalid("Esta PC es la anfitriona: primero pasá o soltá el servidor.")); }
    node.close_route(&id).await;
    if instance::load(&id).is_ok() {
        // Coanfitriona: se deja de participar pero la copia queda (se puede borrar como cualquier servidor).
        let mut s = network::load_share(&id);
        s.enabled = false;
        network::save_share(&id, &s)?;
    } else {
        std::fs::rename(paths::server_dir(&id), paths::trash_slot(&format!("{id}-grupo"))).map_err(|e| AppError::io("No se pudo quitar", e))?;
    }
    Ok(())
}

/* ── Nombre visible de esta PC y solicitudes para alojar ── */

/// Cambia el nombre visible y el símbolo de esta PC. Solo visual: la identidad, las claves y los
/// permisos no cambian.
#[tauri::command]
pub fn pc_identity_set(name: String, symbol: String, cluster: State<'_, Cluster>) -> AppResult<settings::AppSettings> {
    let name = host939_core::pcname::clean_name(&name).ok_or_else(|| AppError::invalid("Escribí un nombre (hasta 24 caracteres)."))?;
    if !host939_core::pcname::valid_symbol(&symbol) { return Err(AppError::invalid("Símbolo desconocido.")); }
    let mut s = settings::load();
    s.node_name = name;
    s.node_symbol = symbol;
    settings::save(&s)?;
    if let Some(n) = cluster.try_node() { n.set_identity(&s.node_name, &s.node_symbol); }
    Ok(s)
}

/// Un nombre al azar distinto del actual (para el botón de dado).
#[tauri::command]
pub fn pc_name_random(current: String) -> String {
    for _ in 0..8 { let n = host939_core::pcname::random_name(); if n != current { return n; } }
    host939_core::pcname::random_name()
}

#[tauri::command]
pub async fn host_request_send(id: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    let node = cluster.node().await?.clone();
    node.request_host(&id).await.map_err(|e| AppError::new("request", e.to_string()))
}

#[tauri::command]
pub async fn host_request_cancel(id: String, cluster: State<'_, Cluster>) -> AppResult<()> {
    let node = cluster.node().await?.clone();
    node.cancel_host_request(&id).await.map_err(|e| AppError::new("request", e.to_string()))
}

#[tauri::command]
pub async fn host_request_answer(id: String, accept: bool, cluster: State<'_, Cluster>) -> AppResult<String> {
    let node = cluster.node().await?.clone();
    node.answer_host_request(&id, accept).await.map_err(|e| AppError::new("handover", e.to_string()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest { pub id: String, pub server_name: String, pub request: host939_cluster::RequestView }

/// Solicitudes que esperan respuesta de esta PC (en todos sus servidores).
#[tauri::command]
pub async fn host_requests_pending(cluster: State<'_, Cluster>) -> AppResult<Vec<PendingRequest>> {
    let Some(node) = cluster.try_node() else { return Ok(vec![]) };
    let mut out = vec![];
    for sid in network::shared_servers() {
        let st = node.status(&sid).await;
        if let Some(r) = st.incoming_request {
            let server_name = instance::load(&sid).map(|c| c.name).unwrap_or_else(|_| "Servidor".into());
            out.push(PendingRequest { id: sid, server_name, request: r });
        }
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostingView { pub id: String, pub i_am_host: bool, pub hosting: bool, pub host_name: String, pub host_symbol: String, pub host_online: bool, pub state: String, pub detail: String, pub players: u32 }

/// Quién aloja cada servidor compartido (para la lista de servidores).
#[tauri::command]
pub async fn servers_hosting(cluster: State<'_, Cluster>) -> AppResult<Vec<HostingView>> {
    let Some(node) = cluster.try_node() else { return Ok(vec![]) };
    let mut out = vec![];
    for sid in network::shared_servers() {
        let st = node.status(&sid).await;
        let host = st.members.iter().find(|m| m.is_host);
        out.push(HostingView {
            id: sid, i_am_host: st.i_am_host, hosting: st.lease.as_ref().map(|l| l.state == host939_core::lease::LeaseState::Hosting).unwrap_or(false),
            host_name: host.map(|m| m.name.clone()).or_else(|| st.lease.as_ref().map(|l| l.host_name.clone())).unwrap_or_default(),
            host_symbol: host.map(|m| m.symbol.clone()).unwrap_or_default(), host_online: st.host_online,
            state: st.host_state.as_ref().map(|h| h.state.clone()).unwrap_or_default(), detail: st.host_state.as_ref().map(|h| h.detail.clone()).unwrap_or_default(),
            players: st.host_state.as_ref().map(|h| h.players).unwrap_or(0),
        });
    }
    Ok(out)
}

/// «Detener» un servidor que aloja otra PC del grupo (lo apaga esa PC, guardando).
#[tauri::command]
pub async fn group_stop_remote(id: String, cluster: State<'_, Cluster>) -> AppResult<String> {
    cluster.node().await?.stop_on_host(&id).await.map_err(|e| AppError::invalid(e.to_string()))
}

/* ── Amigos ── */

fn ferr(e: anyhow::Error) -> AppError { AppError::invalid(e.to_string()) }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendsView { friends: Vec<host939_cluster::friends::FriendView>, requests: Vec<host939_cluster::friends::FriendRequest>, blocked: usize, available: bool, notices: Vec<String> }

#[tauri::command]
pub async fn friends_view(cluster: State<'_, Cluster>) -> AppResult<FriendsView> {
    let n = cluster.node().await?;
    let s = host939_cluster::friends::load();
    Ok(FriendsView { friends: n.friends(), requests: n.friend_requests(), blocked: s.blocked.len(), available: s.available, notices: host939_cluster::friends::take_unreadable() })
}
#[tauri::command]
pub async fn friend_invite(cluster: State<'_, Cluster>) -> AppResult<String> { cluster.node().await?.friend_invite().map_err(ferr) }
#[tauri::command]
pub async fn friend_add(code: String, cluster: State<'_, Cluster>) -> AppResult<()> { cluster.node().await?.friend_add(&code).await.map(|_| ()).map_err(ferr) }
#[tauri::command]
pub async fn friend_answer(node: String, accept: bool, cluster: State<'_, Cluster>) -> AppResult<()> { cluster.node().await?.friend_answer(&node, accept).await.map_err(ferr) }
#[tauri::command]
pub async fn friend_remove(node: String, block: bool, cluster: State<'_, Cluster>) -> AppResult<()> { cluster.node().await?.friend_remove(&node, block).map_err(ferr) }
#[tauri::command]
pub async fn friend_unblock_all(cluster: State<'_, Cluster>) -> AppResult<()> {
    let n = cluster.node().await?;
    for b in host939_cluster::friends::blocked() { n.friend_unblock(&b).map_err(ferr)?; }
    Ok(())
}
#[tauri::command]
pub async fn friend_send(node: String, text: String, cluster: State<'_, Cluster>) -> AppResult<bool> { cluster.node().await?.friend_send(&node, &text).await.map_err(ferr) }
#[tauri::command]
pub async fn friend_chat(node: String, cluster: State<'_, Cluster>) -> AppResult<Vec<host939_cluster::friends::Message>> { Ok(cluster.node().await?.friend_chat(&node)) }
#[tauri::command]
pub async fn friend_set_available(on: bool, cluster: State<'_, Cluster>) -> AppResult<()> { cluster.node().await?.friend_set_available(on).map_err(ferr) }

/* ───────── Preparar para jugar ───────── */

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PrepareProgress<'a> { id: &'a str, message: &'a str, fraction: f64 }

/// Deja listo el Minecraft de esta PC para entrar al servidor: versión, cargador, mods en una
/// carpeta propia del servidor y perfil en el Launcher (ver `clientsetup`). Sirve en la PC que
/// tiene el servidor y en la de un jugador del grupo (pide la lista a la anfitriona).
#[tauri::command]
pub async fn client_prepare(app: AppHandle, id: String, cluster: State<'_, Cluster>) -> AppResult<host939_core::clientsetup::Report> {
    if !paths::valid_server_id(&id) { return Err(AppError::invalid("Servidor inválido.")); }
    let share = network::load_share(&id);
    let local = instance::load(&id).ok().filter(|c| c.installed);
    let node = cluster.try_node();
    let emit = |m: &str, f: f64| { let _ = app.emit("client-prepare-progress", PrepareProgress { id: &id, message: m, fraction: f }); };
    emit("Consultando qué necesita el servidor", 0.01);
    let plan = match (&local, &node) {
        (Some(_), _) => host939_core::profile::client_plan(&host939_core::modrinth::Modrinth, &id).await?,
        (None, Some(n)) if share.enabled => n.client_plan(&id).await.map_err(|e| AppError::invalid(e.to_string()))?,
        _ => return Err(AppError::invalid("Esta PC no tiene el servidor ni está en su grupo.")),
    };
    // Dirección con la que esta PC entra: por 939 Host si el servidor es del grupo; si es solo de
    // esta PC, directo al puerto local.
    let address = if share.enabled { format!("127.0.0.1:{}", route_port(&id)) } else { format!("127.0.0.1:{}", local.as_ref().map(|c| c.port).unwrap_or(25565)) };
    let icon = host939_core::logo::data_url(&id).or_else(|| share.card.as_ref().and_then(|c| c.get("logo")).and_then(|v| v.as_str()).map(String::from));
    let id2 = id.clone();
    let node2 = node.cloned();
    let has_local = local.is_some();
    let pack_sha: std::collections::HashMap<String, String> = plan.packs.iter().map(|p| (p.filename.clone(), p.sha1.clone())).collect();
    let fetch = move |filename: String| {
        let (id, node, pack_sha) = (id2.clone(), node2.clone(), pack_sha.clone());
        async move {
            // Resource packs propios: del servidor de esta PC o, en tramos, de la PC que lo tiene.
            if let Some(pack) = filename.strip_prefix("resourcepacks/") {
                let sha = pack_sha.get(pack).cloned().unwrap_or_default();
                if let Some(p) = host939_core::resourcepacks::find_local(&id, &sha) { return std::fs::read(p).map_err(|e| AppError::io("No se pudo leer el resource pack", e)); }
                let n = node.ok_or_else(|| AppError::invalid("La red del grupo no está lista."))?;
                let p = n.pack_fetch(&id, &sha).await.map_err(|e| AppError::invalid(e.to_string()))?;
                return std::fs::read(p).map_err(|e| AppError::io("No se pudo leer el resource pack", e));
            }
            if has_local {
                return std::fs::read(host939_core::mods::mods_dir(&id).join(&filename)).map_err(|e| AppError::io("No se pudo leer un mod del servidor", e));
            }
            match node { Some(n) => n.client_mod(&id, &filename).await.map_err(|e| AppError::invalid(e.to_string())), None => Err(AppError::invalid("La red del grupo no está lista.")) }
        }
    };
    host939_core::clientsetup::prepare(&plan, &address, icon.as_deref(), fetch, |m, f| emit(m, f)).await
}

/// Carpeta de juego del perfil de un servidor (la que ya usa su perfil, o la que usaría).
fn profile_dir(id: &str) -> AppResult<(std::path::PathBuf, std::path::PathBuf)> {
    let mc = host939_core::clientsetup::minecraft_dir().ok_or_else(|| AppError::invalid("No se encontró la carpeta de Minecraft."))?;
    let name = instance::load(id).map(|c| c.name).ok().or_else(|| network::load_share(id).card.as_ref().and_then(|c| c.get("name")).and_then(|v| v.as_str()).map(String::from)).unwrap_or_default();
    let plan = host939_core::clientsetup::Plan { server_id: id.to_string(), server_name: name, ..Default::default() };
    let g = host939_core::clientsetup::resolve_game_dir(&mc, &plan);
    Ok((mc, g))
}

/// Deshace el cambio de mods de «Preparar para jugar» (la carpeta anterior vuelve a ser «mods»).
#[tauri::command]
pub fn client_prepare_undo(id: String, previous: String) -> AppResult<String> {
    if !paths::valid_server_id(&id) { return Err(AppError::invalid("Servidor inválido.")); }
    let (_, g) = profile_dir(&id)?;
    host939_core::clientsetup::undo_mods(&g, &previous)
}

/// Usa las preferencias del último perfil usado en el perfil de este servidor (lo que había queda
/// guardado como copia).
#[tauri::command]
pub fn client_import_prefs(id: String) -> AppResult<Vec<String>> {
    if !paths::valid_server_id(&id) { return Err(AppError::invalid("Servidor inválido.")); }
    let (mc, g) = profile_dir(&id)?;
    let (name, from) = host939_core::clientsetup::last_used_profile(&mc, &g).ok_or_else(|| AppError::invalid("No se encontró otro perfil con preferencias para copiar."))?;
    let done = host939_core::clientsetup::import_preferences(&from, &g)?;
    host939_core::clientsetup::mark_seeded(&g, &name)?;
    Ok(done)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinecraftDirs { pub current: Option<String>, pub custom: String, pub found: Vec<host939_core::clientsetup::Install> }

/// Instalaciones de Minecraft encontradas y la que se usa.
#[tauri::command]
pub fn minecraft_dirs() -> MinecraftDirs {
    MinecraftDirs { current: host939_core::clientsetup::minecraft_dir().map(|p| p.to_string_lossy().to_string()), custom: host939_core::settings::load().minecraft_dir, found: host939_core::clientsetup::installations() }
}

/// Elige a mano la carpeta de Minecraft (vacío: volver a detectarla). Se comprueba que sea una.
#[tauri::command]
pub fn minecraft_dir_set(path: String) -> AppResult<MinecraftDirs> {
    let p = path.trim().to_string();
    if !p.is_empty() && !host939_core::clientsetup::looks_like_minecraft(std::path::Path::new(&p)) {
        return Err(AppError::invalid("Esa carpeta no parece la de Minecraft: no tiene launcher_profiles.json ni la carpeta versions. Abrí el Launcher una vez o elegí la carpeta correcta (normalmente se llama .minecraft)."));
    }
    let mut s = host939_core::settings::load();
    s.minecraft_dir = p;
    host939_core::settings::save(&s)?;
    Ok(minecraft_dirs())
}
