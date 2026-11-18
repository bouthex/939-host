//! Servidor lógico persistente entre las PCs del grupo.
//!
//! Cada PC corre un nodo (`GroupNode`) con una sola identidad P2P. Por cada servidor compartido:
//!
//! * **Un solo escritor.** El permiso de anfitrión (`lease`) tiene una época que solo crece.
//!   Solo la PC que tiene el permiso puede encender el servidor. Una PC que ve una época mayor
//!   que la suya y estaba hosteando se detiene y aparta su copia (nunca se mezclan mundos).
//! * **Sincronización.** Las PCs coanfitrionas copian el último punto de restauración del
//!   anfitrión: solo viajan los bloques nuevos, cada uno verificado por su SHA-256.
//! * **Traspaso planificado.** El anfitrión apaga, toma un punto final, entrega el permiso a la
//!   otra PC, que copia lo que le falta, coloca el estado y enciende.
//! * **Recuperación automática.** Si el anfitrión deja de responder, las demás PCs eligen por
//!   mayoría a la que tenga el estado más reciente y la levantan. Con dos PCs no hay mayoría
//!   posible: queda el botón "Recuperar aquí" (manual, con aviso de lo que se puede perder).
//! * **Misma dirección para jugar.** Cada jugador se conecta siempre a `127.0.0.1:<puerto fijo>`;
//!   el nodo lleva la conexión a la PC que sea anfitriona en ese momento (directo, o a través de
//!   otra PC del grupo si no hay camino directo). Si nadie está hosteando, el puerto responde con
//!   un mensaje claro en la lista de servidores de Minecraft.

pub mod cert;
pub mod files;
pub mod moving;
pub mod signed;
pub mod wire;

use anyhow::{anyhow, bail, Context, Result};
use host939_core::lease::{self, Lease, LeaseKind, LeaseState};
use host939_core::network::{self, Invite, Member, ShareConfig};
use host939_core::snapshots::{self, SnapshotInfo};
use host939_p2p::Relays;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

pub const ALPN_CTRL: &[u8] = b"939host/ctrl/1";
pub const ALPN_SYNC: &[u8] = b"939host/sync/1";
pub const ALPN_MC: &[u8] = b"939host/mc/2";
pub const ALPN_BRIDGE: &[u8] = b"939host/bridge/1";
pub const ALPN_JOIN: &[u8] = b"939host/join/2";

/* ─────────────────────────── Lo que el nodo necesita de la app ─────────────────────────── */

/// Operaciones locales que el nodo le pide a la app (o al nodo sin interfaz de las pruebas).
/// Las que tardan se llaman desde hilos de bloqueo.
pub trait LocalHost: Send + Sync + 'static {
    fn is_running(&self, server: &str) -> bool;
    /// Puerto del servidor de Minecraft en esta PC.
    fn mc_port(&self, server: &str) -> u16;
    /// Deja el servidor listo para encender (descarga Java, servidor y loader si faltan).
    fn prepare(&self, server: &str) -> Result<(), String>;
    /// Enciende y espera a que esté en línea.
    fn start(&self, server: &str) -> Result<(), String>;
    /// Apaga ordenadamente (guarda el mundo) y espera.
    fn stop(&self, server: &str) -> Result<(), String>;
    /// Toma un punto de restauración (`kind`: auto | manual | seguridad).
    fn snapshot(&self, server: &str, label: &str, kind: &str) -> Result<SnapshotInfo, String>;
    /// Puntaje de esta PC para hostear (0 = no debería hostear).
    fn score(&self) -> u32;
    /// Si esta PC acepta hostear este servidor (ajustes del usuario).
    fn allow_hosting(&self, server: &str) -> bool;
    /// Si esta PC acepta hacer de puente entre otras PCs del grupo.
    fn allow_bridge(&self) -> bool { true }
    /// Aviso para mostrar en la consola o en la interfaz.
    fn note(&self, server: &str, message: &str);
    /// El estado del grupo cambió (para refrescar la interfaz).
    fn changed(&self, server: &str);
    /// Resumen del hardware de esta PC para quien decide un cambio de anfitrión.
    fn profile(&self) -> HostProfile { HostProfile::default() }
    /// Llegó una solicitud para alojar un servidor que hostea esta PC (aviso visible).
    fn host_request(&self, _server: &str, _from_name: &str) {}
}

/// Lo que una PC cuenta de sí misma al pedir ser anfitriona (solo para mostrar y advertir).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct HostProfile {
    pub cpu: String,
    pub cores: u32,
    pub ram_total_mb: u64,
    pub ram_free_mb: u64,
    pub cpu_score: Option<u32>,
    pub disk_free_gb: Option<f64>,
}

/// Tiempos del protocolo. Los valores por defecto son los de uso real; las pruebas los acortan.
#[derive(Clone, Debug)]
pub struct Timing {
    /// Cada cuánto se consulta a las demás PCs.
    pub poll: Duration,
    /// Sin respuesta del anfitrión durante este tiempo, se considera caído.
    pub host_dead: Duration,
    /// El anfitrión que pierde contacto con una mayoría durante este tiempo se aísla solo
    /// (tiene que ser menor que `host_dead` para que nunca haya dos encendidos a la vez).
    pub fence: Duration,
    /// Ventana para considerar que una PC "estaba conectada".
    pub seen_window: Duration,
    /// Cuánto espera el anfitrión que suelta el servidor a que otra PC copie el último punto.
    pub release_wait: Duration,
    /// Una solicitud para ser anfitriona vence si nadie la responde en este tiempo.
    pub request_ttl: Duration,
    /// Espera obligatoria para la sucesión de la propietaria y para cambiar un código de
    /// recuperación perdido (72 h en uso real).
    pub ownership_wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            poll: Duration::from_secs(5),
            host_dead: Duration::from_secs(35),
            fence: Duration::from_secs(20),
            seen_window: Duration::from_secs(180),
            release_wait: Duration::from_secs(90),
            request_ttl: Duration::from_secs(600),
            ownership_wait: Duration::from_secs(72 * 3600),
        }
    }
}

/* ─────────────────────────────── Mensajes ─────────────────────────────── */

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Req {
    Info { server: String },
    Config { server: String },
    Lease { server: String, lease: Lease },
    Handover { server: String, lease: Lease, snapshot: Option<String> },
    Vote { server: String, epoch: u64, candidate: String, fresh: (u64, u64) },
    SyncNow { server: String },
    Bye { server: String },
    /// Una PC del grupo pide alojar el servidor (llega al anfitrión actual).
    HostRequest { server: String, name: String, symbol: String, profile: HostProfile },
    HostRequestCancel { server: String },
    /// Respuesta del anfitrión a quien pidió.
    HostAnswer { server: String, accepted: bool, message: String },
    /// Traspaso en dos fases: la PC destino copia el punto ANTES de recibir el permiso.
    Prefetch { server: String, snapshot: String },
    /// Una PC se va del grupo: se lo avisa a la propietaria, que firma la lista sin ella.
    Leave { server: String },
    /// Cadena de permisos firmados posteriores a la época `after` (para PCs atrasadas).
    LeaseChain { server: String, after: u64 },
    /// La sucesora designada avisa que inició la espera para reemplazar a la propietaria.
    Succession { server: String, claim: serde_json::Value },
}

impl Req {
    fn server(&self) -> &str {
        match self {
            Req::Info { server } | Req::Config { server } | Req::Lease { server, .. } | Req::Handover { server, .. }
            | Req::Vote { server, .. } | Req::SyncNow { server } | Req::Bye { server }
            | Req::HostRequest { server, .. } | Req::HostRequestCancel { server } | Req::HostAnswer { server, .. } | Req::Prefetch { server, .. } | Req::Leave { server } | Req::LeaseChain { server, .. } | Req::Succession { server, .. } => server,
        }
    }
}

/// Lo que una PC informa de sí misma sobre un servidor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub node: String,
    pub name: String,
    #[serde(default)]
    pub symbol: String,
    pub lease: Option<Lease>,
    pub latest: Option<SnapshotInfo>,
    pub has_copy: bool,
    pub running: bool,
    pub can_host: bool,
    pub score: u32,
    pub share_rev: u64,
    /// Propietaria según la lista que tiene esta PC (para contar testigos de una recuperación).
    #[serde(default)]
    pub owner: String,
    pub addr: Option<EndpointAddr>,
    #[serde(default)]
    pub peers: Vec<EndpointAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Resp {
    Info(Info),
    Config { config: Option<serde_json::Value>, share: ShareConfig },
    Ok,
    Vote { agree: bool, #[serde(default)] sig: Option<String> },
    Leases { leases: Vec<Lease> },
    Err { msg: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum SyncReq {
    Manifest { server: String, snap: String },
    Blobs { server: String, hashes: Vec<String> },
}

#[derive(Serialize, Deserialize)]
struct JoinReq { secret: String, name: String }

#[derive(Serialize, Deserialize)]
struct JoinResp { ok: bool, error: Option<String>, server: Option<String>, share: Option<ShareConfig>, role: Option<String>, peers: Vec<EndpointAddr> }

#[derive(Serialize, Deserialize)]
struct BridgeReq { server: String, target: String }

/* ─────────────────────────────── Estado ─────────────────────────────── */

#[derive(Default)]
struct SrvState {
    last_ok: HashMap<String, Instant>,
    infos: HashMap<String, Info>,
    left: HashSet<String>,
    voted: Option<(u64, String)>,
    fenced: bool,
    busy: Option<String>,
    syncing: bool,
    /// Solicitud que recibió esta PC (siendo anfitriona). Una sola a la vez.
    incoming: Option<Incoming>,
    /// Solicitud que hizo esta PC y espera respuesta.
    outgoing: Option<Outgoing>,
    /// Última respuesta recibida (para mostrarla un rato).
    answer: Option<(bool, String, Instant)>,
    /// Veces que una PC dejó de responder desde que se abrió 939 Host (estabilidad).
    drops: HashMap<String, u32>,
    /// PCs con una consulta en curso (no se les manda otra hasta que respondan o venza).
    inflight: HashSet<String>,
    /// Respuestas recibidas que todavía no se procesaron (permiso, grupo, nombres).
    inbox: Vec<(String, Info)>,
    /// Permisos rechazados por firma, autoridad o encadenamiento (para diagnóstico y pruebas).
    rejected: u32,
    /// Reclamo de sucesión en curso que vio esta PC.
    claim: Option<serde_json::Value>,
}

#[derive(Clone)]
struct Incoming { node: String, name: String, symbol: String, profile: HostProfile, at: Instant, epoch: u64 }

#[derive(Clone)]
struct Outgoing { host: String, host_name: String, at: Instant, epoch: u64 }

struct Inner {
    ep: Endpoint,
    /// Modo de conexión de esta PC (directo, con relays o privado).
    relays: Relays,
    /// Clave del nodo (la misma de iroh): firma los cambios de integrantes si esta PC es propietaria.
    key: iroh::SecretKey,
    me: String,
    name: RwLock<(String, String)>,
    local: Arc<dyn LocalHost>,
    timing: Timing,
    started: Instant,
    peers: RwLock<HashMap<String, EndpointAddr>>,
    conns: tokio::sync::Mutex<HashMap<(String, &'static [u8]), Connection>>,
    state: Mutex<HashMap<String, SrvState>>,
    routes: tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    stopping: std::sync::atomic::AtomicBool,
    /// Límite de intentos de unión (antes de autenticar).
    join_limit: Mutex<RateLimit>,
}

/// Estado de un servidor compartido tal como lo ve esta PC (para la interfaz).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupStatus {
    pub server: String,
    pub me: String,
    pub lease: Option<Lease>,
    pub i_am_host: bool,
    pub host_online: bool,
    pub my_latest: Option<SnapshotInfo>,
    pub has_copy: bool,
    pub fenced: bool,
    pub busy: Option<String>,
    pub can_start: Result<(), String>,
    pub auto_failover: bool,
    pub handover_on_quit: bool,
    pub majority: usize,
    pub members: Vec<MemberStatus>,
    /// Si esta PC puede pedir ser anfitriona ahora (y si no, por qué).
    pub can_request: Result<(), String>,
    /// Solicitud recibida (esta PC es la anfitriona).
    pub incoming_request: Option<RequestView>,
    /// Solicitud enviada por esta PC que espera respuesta.
    pub outgoing_request: Option<OutgoingView>,
    /// Última respuesta a una solicitud de esta PC (se muestra un minuto).
    pub last_answer: Option<AnswerView>,
}

/// Solicitud para alojar el servidor, tal como la ve el anfitrión actual.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestView {
    pub node_id: String,
    pub name: String,
    pub symbol: String,
    pub age_secs: u64,
    pub expires_in_secs: u64,
    pub profile: HostProfile,
    /// Hardware de esta PC (la anfitriona) para comparar.
    pub mine: HostProfile,
    pub online: bool,
    pub path: String,
    pub rtt_ms: Option<u64>,
    /// Limitaciones importantes (nunca bloquean la aceptación).
    pub warnings: Vec<String>,
    /// Lo que juega a favor del solicitante.
    pub strengths: Vec<String>,
    /// Si al solicitante le falta copiar parte del mundo.
    pub needs_copy: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingView { pub host_name: String, pub age_secs: u64, pub expires_in_secs: u64 }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerView { pub accepted: bool, pub message: String }

fn gb(mb: u64) -> String { format!("{:.1}", mb as f64 / 1024.0).replace(".0", "").replace('.', ",") }

/// Advertencias y fortalezas de una PC candidata frente a la anfitriona actual. Solo informan:
/// la decisión es de quien aloja (una PC con menos potencia puede tener mejor conexión).
pub fn assess(cand: &HostProfile, cand_score: u32, mine: &HostProfile, my_score: u32, server_mem_mb: u64, path: &str, rtt_ms: Option<u64>, drops: u32) -> (Vec<String>, Vec<String>) {
    let mut w = vec![];
    let mut g = vec![];
    if cand.ram_total_mb > 0 {
        if cand.ram_total_mb < server_mem_mb + 2048 {
            w.push(format!("Tiene {} GB de memoria en total y el servidor usa {} GB: puede quedarse sin memoria.", gb(cand.ram_total_mb), gb(server_mem_mb)));
        } else if cand.ram_free_mb < server_mem_mb + 512 {
            w.push(format!("Ahora tiene {} GB libres y el servidor necesita {} GB: conviene que cierre otros programas.", gb(cand.ram_free_mb), gb(server_mem_mb)));
        } else if mine.ram_free_mb > 0 && cand.ram_free_mb > mine.ram_free_mb + 2048 {
            g.push("Tiene más memoria libre que esta PC.".into());
        }
    }
    if cand_score > 0 && my_score > 0 {
        if (cand_score as f64) < my_score as f64 * 0.6 {
            w.push("Su procesador rinde bastante menos que el de esta PC: con muchos jugadores o mods puede haber lag.".into());
        } else if cand_score as f64 > my_score as f64 * 1.2 {
            g.push("Su procesador rinde más que el de esta PC.".into());
        }
    } else if cand_score == 0 {
        w.push("Ahora no está en condiciones de hostear (poca memoria libre o mucho uso).".into());
    }
    match (path, rtt_ms) {
        ("relay", _) => w.push("Se conecta al grupo por un relay: los jugadores pueden notar más latencia.".into()),
        (_, Some(r)) if r > 150 => w.push(format!("La latencia con esta PC es alta ({r} ms).")),
        ("directa", Some(r)) => g.push(format!("Conexión directa con el grupo ({r} ms).")),
        _ => {}
    }
    if drops >= 3 { w.push(format!("Se desconectó del grupo {drops} veces desde que abriste 939 Host.")); }
    if let Some(d) = cand.disk_free_gb { if d < 5.0 { w.push(format!("Le quedan {d:.0} GB libres en el disco.")); } }
    (w, g)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatus {
    pub node_id: String,
    pub name: String,
    pub symbol: String,
    pub role: String,
    pub is_me: bool,
    pub online: bool,
    pub running: bool,
    pub is_host: bool,
    pub can_host: bool,
    pub score: u32,
    pub latest_at: Option<String>,
    pub fresh: (u64, u64),
    /// "directa", "relay" o "" si no hay conexión ahora.
    pub path: String,
    pub rtt_ms: Option<u64>,
}

fn majority(n: usize) -> usize { n / 2 + 1 }

fn fresh(s: &Option<SnapshotInfo>) -> (u64, u64) { s.as_ref().map(|s| s.freshness()).unwrap_or((0, 0)) }

fn has_copy(server: &str) -> bool { host939_core::instance::load(server).is_ok() }

fn mc_version(server: &str) -> String { host939_core::instance::load(server).map(|c| c.mc_version).unwrap_or_default() }

fn server_name(server: &str) -> String {
    host939_core::instance::load(server).map(|c| c.name).ok()
        .or_else(|| network::load_share(server).card.and_then(|c| c.get("name").and_then(|n| n.as_str()).map(String::from)))
        .unwrap_or_else(|| "Servidor".into())
}

/* ─────────────────────────────── Nodo ─────────────────────────────── */

#[derive(Clone)]
pub struct GroupNode { inner: Arc<Inner>, router: Arc<Router> }

impl GroupNode {
    /// Abre el nodo de esta PC y empieza a atender al grupo.
    pub async fn start(secret: [u8; 32], name: &str, relays: Relays, local: Arc<dyn LocalHost>, timing: Timing) -> Result<GroupNode> {
        let alpns = vec![ALPN_CTRL.to_vec(), ALPN_SYNC.to_vec(), ALPN_MC.to_vec(), ALPN_BRIDGE.to_vec(), ALPN_JOIN.to_vec(), files::ALPN_FILES.to_vec()];
        let ep = host939_p2p::endpoint(secret, relays, alpns).await?;
        let me = ep.id().to_string();
        lease::set_local_node(&me);
        let mut peers = HashMap::new();
        for (k, v) in network::load_peers() {
            if let Ok(a) = serde_json::from_value::<EndpointAddr>(v) { peers.insert(k, a); }
        }
        let inner = Arc::new(Inner {
            ep: ep.clone(), relays, key: iroh::SecretKey::from_bytes(&secret), me, name: RwLock::new((name.to_string(), String::new())), local, timing, started: Instant::now(),
            peers: RwLock::new(peers), conns: Default::default(), state: Default::default(), routes: Default::default(),
            stopping: Default::default(), join_limit: Mutex::new(RateLimit::new(5, 30)),
        });
        let h = Handler(inner.clone());
        let router = Router::builder(ep)
            .accept(ALPN_CTRL, h.clone()).accept(ALPN_SYNC, SyncH(inner.clone())).accept(ALPN_MC, McH(inner.clone()))
            .accept(ALPN_BRIDGE, BridgeH(inner.clone())).accept(ALPN_JOIN, JoinH(inner.clone())).accept(files::ALPN_FILES, files::FilesH(inner.clone()))
            .spawn();
        let node = GroupNode { inner: inner.clone(), router: Arc::new(router) };
        let n2 = node.clone();
        tokio::spawn(async move {
            loop {
                if n2.inner.stopping.load(std::sync::atomic::Ordering::Relaxed) { break; }
                let servers = network::shared_servers();
                futures_util::future::join_all(servers.iter().map(|s| n2.tick(s))).await;
                tokio::time::sleep(n2.inner.timing.poll).await;
            }
        });
        Ok(node)
    }

    pub fn id(&self) -> String { self.inner.me.clone() }
    pub fn addr(&self) -> EndpointAddr { self.inner.ep.addr() }
    pub fn endpoint(&self) -> &Endpoint { &self.inner.ep }

    /// Cierra el nodo (sin traspasos: para eso está `quit`).
    pub async fn shutdown(&self) {
        self.inner.stopping.store(true, std::sync::atomic::Ordering::Relaxed);
        for (_, t) in self.inner.routes.lock().await.drain() { t.abort(); }
        let _ = self.router.shutdown().await;
    }

    /* ── Compartir e invitaciones ── */

    /// Empieza a compartir un servidor propio: esta PC queda como propietaria y anfitriona.
    pub fn share(&self, server: &str) -> Result<ShareConfig> {
        let cfg = host939_core::instance::load(server).map_err(|e| anyhow!(e.message))?;
        let mut s = network::load_share(server);
        s.enabled = true;
        if s.member(&self.inner.me).is_none() {
            s.members.insert(0, Member { node_id: self.inner.me.clone(), name: self.inner.name(), joined_at: chrono::Local::now().to_rfc3339(), role: "propietario".into() });
        }
        let prev_rev = rev(&s).unwrap_or(0);
        s.card = Some(card_for(&cfg, prev_rev));
        self.inner.commit(server, &mut s)?;
        if lease::read(server).is_none() { self.inner.new_lease(server, 0, LeaseKind::Genesis)?; }
        Ok(s)
    }

    /// Crea una invitación que vence en 24 h. `role`: "coanfitrion" (guarda copia y puede hostear) o "jugador".
    pub fn invite(&self, server: &str, role: &str, label: &str) -> Result<String> { self.invite_for(server, role, label, 24) }

    /// Crea una invitación de un solo uso que vence en `hours` horas (entre 1 y 720).
    pub fn invite_for(&self, server: &str, role: &str, label: &str, hours: u32) -> Result<String> {
        // Solo la propietaria invita: es la única que puede firmar la entrada de una PC nueva.
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede invitar a otras PCs."); }
        let mut s = network::load_share(server);
        if !s.enabled { bail!("Primero compartí el servidor."); }
        let secret = network::random_secret();
        let role = if role == "coanfitrion" { "coanfitrion" } else { "jugador" };
        let hours = hours.clamp(1, 720) as i64;
        s.invites.retain(|i| !i.expired());
        if s.invites.len() >= 50 { bail!("Hay demasiadas invitaciones sin usar: revocá algunas antes de crear otra."); }
        s.invites.push(Invite { secret: secret.clone(), label: label.chars().take(40).collect(), created_at: chrono::Local::now().to_rfc3339(), role: role.into(),
            expires_at: Some((chrono::Utc::now() + chrono::Duration::hours(hours)).to_rfc3339()) });
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        // Con relays el código lleva solo la identidad y el relay: nunca las IPs de esta PC.
        Ok(host939_p2p::make_ticket(&host939_p2p::shareable(&self.inner.ep.addr(), self.inner.relays), &secret))
    }

    pub fn revoke_invite(&self, server: &str, secret: &str) -> Result<()> {
        let mut s = network::load_share(server);
        s.invites.retain(|i| i.secret != secret);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))
    }

    /// Saca a una PC del grupo (no puede quitarse al anfitrión actual).
    pub fn remove_member(&self, server: &str, node: &str) -> Result<()> {
        if network::load_share(server).member(&self.inner.me).map(|m| m.role != "propietario").unwrap_or(true) {
            bail!("Solo la PC propietaria puede quitar PCs del grupo.");
        }
        if lease::read(server).map(|l| l.held_by(node)).unwrap_or(false) { bail!("Esa PC es la anfitriona ahora: primero pasá el servidor a otra."); }
        let mut s = network::load_share(server);
        s.members.retain(|m| m.node_id != node || m.role == "propietario");
        self.inner.commit(server, &mut s)
    }

    /// La propietaria firma y guarda un cambio de la lista (roles, por ejemplo).
    pub fn commit_share(&self, server: &str, s: &mut ShareConfig) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede cambiar el grupo."); }
        self.inner.commit(server, s)
    }

    /// Permisos rechazados desde que se abrió (diagnóstico y pruebas).
    pub fn rejected_count(&self, server: &str) -> u32 { self.inner.with_state(server, |s| s.rejected) }

    /// La propietaria vuelve a publicar los datos públicos (por ejemplo, tras cambiar el logo, el
    /// nombre o la versión). Las demás PCs lo toman en la próxima consulta.
    pub fn refresh_card(&self, server: &str) -> Result<bool> {
        if !self.inner.is_owner(server) { return Ok(false); }
        let cfg = host939_core::instance::load(server).map_err(|e| anyhow!(e.message))?;
        let mut s = network::load_share(server);
        if !s.enabled { return Ok(false); }
        let card = card_for(&cfg, rev(&s).unwrap_or(0));
        let same = s.card.as_ref().map(|c| { let mut a = c.clone(); let mut b = card.clone(); a["rev"] = 0.into(); b["rev"] = 0.into(); a == b }).unwrap_or(false);
        if same { return Ok(false); }
        s.card = Some(card);
        self.inner.commit(server, &mut s)?;
        Ok(true)
    }

    /// Pasa la propiedad a otra PC del grupo. La firma la propietaria actual; desde ese momento
    /// todas las PCs confían en la nueva, y esta queda como coanfitriona (o jugadora sin copia).
    pub async fn transfer_ownership(&self, server: &str, to: &str) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede transferir la propiedad."); }
        let mut s = network::load_share(server);
        if to == self.inner.me || s.member(to).is_none() { bail!("Elegí otra PC del grupo."); }
        let mine_role = if has_copy(server) { "coanfitrion" } else { "jugador" };
        for m in s.members.iter_mut() {
            if m.node_id == to { m.role = "propietario".into(); } else if m.node_id == self.inner.me { m.role = mine_role.into(); }
        }
        self.inner.commit(server, &mut s)?;
        self.inner.local.changed(server);
        // Que la nueva propietaria y el resto lo tomen ya.
        for m in s.members.iter().filter(|m| m.node_id != self.inner.me) {
            let _ = self.inner.rpc_timeout(&m.node_id, &Req::SyncNow { server: server.into() }, Duration::from_secs(3)).await;
        }
        Ok(())
    }

    /* ── Recuperación de la propiedad ── */

    /// Crea el código de recuperación (solo si el grupo todavía no tiene). Se muestra una sola vez:
    /// esta PC guarda únicamente la clave pública derivada, nunca el código.
    pub fn create_recovery_code(&self, server: &str) -> Result<String> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede crear el código de recuperación."); }
        let mut s = network::load_share(server);
        if s.recovery.is_some() { bail!("Este grupo ya tiene un código de recuperación. Para cambiarlo usá el código actual."); }
        let code = signed::new_recovery_code();
        s.recovery = Some(signed::recovery_key(&code).ok_or_else(|| anyhow!("código inválido"))?.public().to_string());
        self.inner.commit(server, &mut s)?;
        audit(server, "recovery-created", &self.inner.me);
        Ok(code)
    }

    /// Cambia el código usando el actual (el nuevo se muestra una sola vez).
    pub fn rotate_recovery_code(&self, server: &str, current: &str) -> Result<String> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede cambiar el código."); }
        let rk = signed::recovery_key(current).ok_or_else(|| anyhow!("El código no tiene el formato correcto."))?;
        let mut s = network::load_share(server);
        if s.recovery.as_deref() != Some(rk.public().to_string().as_str()) { bail!("Ese no es el código de recuperación de este grupo."); }
        let code = signed::new_recovery_code();
        s.recovery = Some(signed::recovery_key(&code).unwrap().public().to_string());
        s.recovery_pending = None;
        self.inner.commit(server, &mut s)?;
        let mut s = network::load_share(server);
        signed::sign_recovery(&rk, server, &mut s);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        audit(server, "recovery-rotated", &self.inner.me);
        Ok(code)
    }

    /// La propietaria perdió el código: pide uno nuevo, que vale recién tras la espera de seguridad
    /// (72 h). Si alguien robó la clave de la propietaria y lo intenta, quien tiene el código
    /// verdadero puede recuperar el grupo antes de que venza.
    pub fn request_recovery_reset(&self, server: &str) -> Result<String> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede pedirlo."); }
        let code = signed::new_recovery_code();
        let pk = signed::recovery_key(&code).unwrap().public().to_string();
        let mut s = network::load_share(server);
        s.recovery_pending = Some(pk.clone());
        self.inner.commit(server, &mut s)?;
        self.inner.mark_seen(server, &format!("reset:{pk}"));
        audit(server, "recovery-reset-requested", &self.inner.me);
        // El código queda guardado protegido en esta PC solo hasta que se aplique.
        host939_core::identity::protect_to_file(&host939_core::paths::server_dir(server).join("recovery-pending.bin"), code.as_bytes()).map_err(|e| anyhow!(e.message))?;
        Ok(code)
    }

    /// Aplica el código pedido con `request_recovery_reset` cuando ya pasó la espera.
    pub fn finish_recovery_reset(&self, server: &str) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede hacerlo."); }
        let mut s = network::load_share(server);
        let Some(pk) = s.recovery_pending.clone() else { bail!("No hay un cambio de código pendiente."); };
        let waited = self.inner.seen_for(server, &format!("reset:{pk}")).unwrap_or(0);
        if waited < self.inner.timing.ownership_wait.as_secs() { bail!("Todavía no pasó la espera de seguridad."); }
        s.recovery = Some(pk);
        s.recovery_pending = None;
        self.inner.commit(server, &mut s)?;
        let _ = std::fs::remove_file(host939_core::paths::server_dir(server).join("recovery-pending.bin"));
        audit(server, "recovery-reset-applied", &self.inner.me);
        Ok(())
    }

    /// Recupera la propiedad en ESTA PC con el código. La propietaria anterior queda revocada,
    /// se genera un código nuevo (se muestra una sola vez) y nada más cambia: mundos, copias,
    /// integrantes y configuración quedan intactos.
    pub async fn recover_with_code(&self, server: &str, code: &str) -> Result<String> {
        let rk = signed::recovery_key(code).ok_or_else(|| anyhow!("El código no tiene el formato correcto."))?;
        let cur = network::load_share(server);
        if cur.recovery.as_deref() != Some(rk.public().to_string().as_str()) { bail!("Ese no es el código de recuperación de este grupo."); }
        if cur.member(&self.inner.me).is_none() { bail!("Esta PC no es parte del grupo."); }
        let old = signed::pin(&cur).unwrap_or_default();
        if old == self.inner.me { bail!("Esta PC ya es la propietaria."); }
        if lease::read(server).map(|l| l.held_by(&old)).unwrap_or(false) && self.inner.recently_ok(server, &old, self.inner.timing.host_dead) {
            bail!("La propietaria está alojando el servidor y responde: pedile que te transfiera la propiedad.");
        }
        let mut s = cur.clone();
        s.members.retain(|m| m.node_id != old);
        for m in s.members.iter_mut() { if m.node_id == self.inner.me { m.role = "propietario".into(); } }
        let new_code = signed::new_recovery_code();
        s.recovery = Some(signed::recovery_key(&new_code).unwrap().public().to_string());
        s.recovery_pending = None;
        if s.successor.as_deref() == Some(self.inner.me.as_str()) { s.successor = None; }
        s.succession = None;
        s.rsig = None;
        bump_rev(&mut s);
        signed::sign(&self.inner.key, server, &mut s);
        signed::sign_recovery(&rk, server, &mut s);
        if signed::accept_with(server, &cur, &s, &signed::Ctx { seen_for: &|_| None, wait_secs: u64::MAX, witnesses: 0 }).is_err() { bail!("No se pudo armar la recuperación."); }
        s.owner_pin = Some(self.inner.me.clone());
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        audit(server, "recovered-with-code", &self.inner.me);
        self.inner.local.note(server, "Recuperaste la propiedad del grupo con el código. Guardá el código nuevo: el anterior ya no sirve.");
        self.inner.local.changed(server);
        // El permiso de anfitrión: si la anterior lo tenía y no responde, se reasigna con la autoridad nueva.
        if let Some(l) = lease::read(server) {
            if l.held_by(&old) { let _ = self.inner.new_lease(server, l.epoch, LeaseKind::Owner); }
        }
        for m in s.members.iter().filter(|m| m.node_id != self.inner.me) {
            let _ = self.inner.rpc_timeout(&m.node_id, &Req::SyncNow { server: server.into() }, Duration::from_secs(3)).await;
        }
        self.inner.broadcast_lease(server).await;
        Ok(new_code)
    }

    /// La propietaria designa (o quita) a su sucesora. Solo coanfitrionas.
    pub fn set_successor(&self, server: &str, node: Option<&str>) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede designar sucesora."); }
        let mut s = network::load_share(server);
        if let Some(n) = node { if !s.member(n).map(|m| m.role == "coanfitrion").unwrap_or(false) { bail!("La sucesora tiene que ser una coanfitriona del grupo."); } }
        s.successor = node.map(String::from);
        self.inner.commit(server, &mut s)?;
        audit(server, if node.is_some() { "successor-set" } else { "successor-cleared" }, node.unwrap_or(""));
        Ok(())
    }

    /// La sucesora inicia la espera: avisa a todas las PCs (la propietaria lo ve y puede cancelar).
    pub async fn start_succession(&self, server: &str) -> Result<()> {
        let s = network::load_share(server);
        if s.successor.as_deref() != Some(self.inner.me.as_str()) { bail!("Esta PC no es la sucesora designada."); }
        let owner = signed::pin(&s).unwrap_or_default();
        if self.inner.recently_ok(server, &owner, Duration::from_secs(300)) { bail!("La propietaria respondió hace poco: pedile que te transfiera la propiedad."); }
        let claim = signed::make_claim(&self.inner.key, server, &s);
        self.inner.mark_seen(server, &signed::claim_id(&claim));
        self.inner.with_state(server, |st| st.claim = Some(claim.clone()));
        audit(server, "succession-started", &self.inner.me);
        for m in s.members.iter().filter(|m| m.node_id != self.inner.me) {
            let _ = self.inner.rpc_timeout(&m.node_id, &Req::Succession { server: server.into(), claim: claim.clone() }, Duration::from_secs(5)).await;
        }
        self.inner.local.note(server, &format!("Sucesión iniciada. Se puede completar en {} h si la propietaria no la cancela.", self.inner.timing.ownership_wait.as_secs() / 3600));
        Ok(())
    }

    /// La propietaria cancela cualquier sucesión en curso (firma una revisión nueva: el reclamo queda viejo).
    pub fn cancel_succession(&self, server: &str) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede cancelarla."); }
        let mut s = network::load_share(server);
        self.inner.commit(server, &mut s)?;
        self.inner.with_state(server, |st| st.claim = None);
        audit(server, "succession-cancelled", &self.inner.me);
        Ok(())
    }

    /// La sucesora completa la sucesión cuando venció la espera sin que la propietaria la cancelara.
    pub async fn complete_succession(&self, server: &str) -> Result<()> {
        let cur = network::load_share(server);
        let claim = self.inner.with_state(server, |st| st.claim.clone()).ok_or_else(|| anyhow!("No hay una sucesión en curso."))?;
        let base = signed::check_claim(server, &cur, &claim).map_err(|e| anyhow!("{e}"))?;
        if claim["successor"].as_str() != Some(self.inner.me.as_str()) { bail!("Esta PC no es la sucesora."); }
        let waited = self.inner.seen_for(server, &signed::claim_id(&claim)).unwrap_or(0);
        if waited < self.inner.timing.ownership_wait.as_secs() { bail!("Todavía no pasó la espera ({} h restantes).", (self.inner.timing.ownership_wait.as_secs() - waited) / 3600 + 1); }
        let old = signed::pin(&cur).unwrap_or_default();
        let mut s = base.clone();
        s.members.retain(|m| m.node_id != old);
        for m in s.members.iter_mut() { if m.node_id == self.inner.me { m.role = "propietario".into(); } }
        s.successor = None;
        s.succession = Some(claim.clone());
        s.rsig = None;
        let mut card = s.card.take().unwrap_or_else(|| serde_json::json!({}));
        card["rev"] = serde_json::json!(signed::rev(&base) + 1);
        s.card = Some(card);
        signed::sign(&self.inner.key, server, &mut s);
        let seen = |id: &str| self.inner.seen_for(server, id);
        signed::accept_with(server, &cur, &s, &signed::Ctx { seen_for: &seen, wait_secs: self.inner.timing.ownership_wait.as_secs(), witnesses: 0 }).map_err(|e| anyhow!("{e}"))?;
        s.owner_pin = Some(self.inner.me.clone());
        s.enabled = true;
        s.invites.clear();
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        audit(server, "succession-completed", &self.inner.me);
        self.inner.with_state(server, |st| st.claim = None);
        if let Some(l) = lease::read(server) { if l.held_by(&old) { let _ = self.inner.new_lease(server, l.epoch, LeaseKind::Owner); } }
        self.inner.local.note(server, "Sucesión completada: esta PC es la nueva propietaria. La anterior quedó fuera del grupo.");
        for m in s.members.iter().filter(|m| m.node_id != self.inner.me) {
            let _ = self.inner.rpc_timeout(&m.node_id, &Req::SyncNow { server: server.into() }, Duration::from_secs(3)).await;
        }
        self.inner.broadcast_lease(server).await;
        self.inner.local.changed(server);
        Ok(())
    }

    /// Reclamo de sucesión en curso (para la interfaz): (quién, segundos que faltan).
    pub fn succession_status(&self, server: &str) -> Option<(String, u64)> {
        let c = self.inner.with_state(server, |st| st.claim.clone())?;
        let share = network::load_share(server);
        if signed::check_claim(server, &share, &c).is_err() { return None; }
        let waited = self.inner.seen_for(server, &signed::claim_id(&c)).unwrap_or(0);
        Some((c["successor"].as_str().unwrap_or("").to_string(), self.inner.timing.ownership_wait.as_secs().saturating_sub(waited)))
    }

    /// Esta PC sale del grupo. La propietaria no puede: primero transfiere. Devuelve si la
    /// propietaria confirmó (si estaba apagada, la quita cuando quiera; esta PC ya no participa).
    pub async fn leave_group(&self, server: &str) -> Result<bool> {
        let s = network::load_share(server);
        if s.member(&self.inner.me).map(|m| m.role == "propietario").unwrap_or(false) { bail!("Sos la PC propietaria: primero transferí la propiedad a otra PC del grupo."); }
        if lease::read(server).map(|l| l.held_by(&self.inner.me)).unwrap_or(false) { bail!("Esta PC aloja el servidor ahora: primero pasalo a otra PC del grupo."); }
        let owner = signed::owner_of(&s);
        let confirmed = match owner {
            Some(o) => matches!(self.inner.rpc_timeout(&o, &Req::Leave { server: server.into() }, Duration::from_secs(6)).await, Ok(Resp::Ok)),
            None => false,
        };
        self.close_route(server).await;
        self.close_alias(server).await;
        let mut mine = network::load_share(server);
        mine.enabled = false;
        mine.members.retain(|m| m.node_id != self.inner.me);
        network::save_share(server, &mine).map_err(|e| anyhow!(e.message))?;
        self.inner.local.changed(server);
        Ok(confirmed)
    }

    pub fn set_options(&self, server: &str, auto_failover: bool, handover_on_quit: bool) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede cambiar estas opciones del grupo."); }
        let mut s = network::load_share(server);
        s.auto_failover = auto_failover;
        s.handover_on_quit = handover_on_quit;
        self.inner.commit(server, &mut s)
    }

    /// Se une a un servidor con un código de invitación. Devuelve el id del servidor.
    pub async fn join(&self, ticket: &str) -> Result<String> {
        let (addr, secret) = host939_p2p::parse_ticket(ticket)?;
        let conn = tokio::time::timeout(Duration::from_secs(15), self.inner.ep.connect(addr.clone(), ALPN_JOIN)).await
            .context("tiempo de espera agotado")?.context("no se pudo contactar a la PC que invitó (¿está encendida con 939 Host abierto?)")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        wire::write_msg(&mut send, &JoinReq { secret, name: self.inner.name() }).await?;
        send.finish()?;
        let r: JoinResp = wire::read_msg(&mut recv).await?;
        if !r.ok { bail!(r.error.unwrap_or_else(|| "La invitación fue rechazada.".into())); }
        let server = r.server.ok_or_else(|| anyhow!("respuesta incompleta"))?;
        let mut share = r.share.ok_or_else(|| anyhow!("respuesta incompleta"))?;
        share.invites.clear();
        // La PC que invitó es la propietaria (autenticada por la conexión P2P) y firmó la lista.
        let inviter_id = addr.id.to_string();
        if signed::owner_of(&share).as_deref() != Some(inviter_id.as_str()) || !signed::verify(&server, &share, &inviter_id) {
            bail!("La respuesta de la PC que invitó no está firmada por la propietaria del servidor.");
        }
        share.owner_pin = Some(inviter_id);
        std::fs::create_dir_all(host939_core::paths::server_dir(&server))?;
        network::save_share(&server, &share).map_err(|e| anyhow!(e.message))?;
        self.inner.learn(addr.clone());
        for p in r.peers { self.inner.learn(p); }
        let inviter = addr.id.to_string();
        if r.role.as_deref() == Some("coanfitrion") {
            self.fetch_config(&server, &inviter).await?;
        }
        // Primer contacto con el grupo enseguida.
        self.tick(&server).await;
        Ok(server)
    }

    /// Trae la configuración del servidor desde otra PC y crea la copia local.
    async fn fetch_config(&self, server: &str, from: &str) -> Result<()> {
        let Resp::Config { config: Some(cfg), share } = self.inner.rpc(from, &Req::Config { server: server.into() }).await? else {
            bail!("La otra PC no tiene la configuración del servidor.");
        };
        let mut cfg: host939_core::model::ServerConfig = serde_json::from_value(cfg)?;
        // Lo que depende de cada PC no se copia.
        if let Ok(local) = host939_core::instance::load(server) {
            cfg.memory_mb = local.memory_mb; cfg.jvm_profile = local.jvm_profile; cfg.extra_jvm_args = local.extra_jvm_args;
            cfg.installed = local.installed && local.mc_version == cfg.mc_version && local.loader_version == cfg.loader_version;
            cfg.port = local.port;
        } else {
            cfg.installed = false;
            cfg.install_error = None;
        }
        host939_core::instance::save(&cfg).map_err(|e| anyhow!(e.message))?;
        let mut mine = network::load_share(server);
        // La lista de integrantes solo se toma si pasa la verificación; si no, se queda la local.
        mine.enabled = true;
        network::save_share(server, &mine).map_err(|e| anyhow!(e.message))?;
        self.inner.adopt_share(server, &share);
        Ok(())
    }

    /* ── Abrir el servidor para jugar ── */

    /// Escucha en `127.0.0.1:local_port` y lleva cada conexión de Minecraft a la PC anfitriona.
    pub async fn open_route(&self, server: &str, local_port: u16) -> Result<u16> {
        let mut routes = self.inner.routes.lock().await;
        if let Some(t) = routes.remove(server) { t.abort(); }
        let l = TcpListener::bind(("127.0.0.1", local_port)).await.with_context(|| format!("el puerto local {local_port} está ocupado"))?;
        let port = l.local_addr()?.port();
        let inner = self.inner.clone();
        let sid = server.to_string();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = l.accept().await else { break };
                let _ = tcp.set_nodelay(true);
                let inner = inner.clone();
                let sid = sid.clone();
                tokio::spawn(async move { let _ = inner.route_one(&sid, tcp).await; });
            }
        });
        routes.insert(server.to_string(), task);
        Ok(port)
    }

    /// Igual que `open_route`, pero en la IP propia del servidor (`127.93.x.y:25565`), para su
    /// dirección `nombre.939host`. Convive con la ruta de siempre.
    pub async fn open_alias(&self, server: &str, ip: std::net::Ipv4Addr) -> Result<()> {
        let key = format!("alias:{server}");
        let mut routes = self.inner.routes.lock().await;
        if let Some(t) = routes.remove(&key) { t.abort(); }
        let l = TcpListener::bind((ip, 25565)).await.with_context(|| format!("no se pudo usar {ip}:25565"))?;
        let inner = self.inner.clone();
        let sid = server.to_string();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = l.accept().await else { break };
                let _ = tcp.set_nodelay(true);
                let inner = inner.clone();
                let sid = sid.clone();
                tokio::spawn(async move { let _ = inner.route_one(&sid, tcp).await; });
            }
        });
        routes.insert(key, task);
        Ok(())
    }

    pub async fn close_alias(&self, server: &str) {
        if let Some(t) = self.inner.routes.lock().await.remove(&format!("alias:{server}")) { t.abort(); }
    }

    pub async fn close_route(&self, server: &str) {
        if let Some(t) = self.inner.routes.lock().await.remove(server) { t.abort(); }
    }

    /* ── Encender, pasar, soltar, recuperar ── */

    /// Comprueba (y toma, si corresponde) el permiso antes de encender en esta PC.
    /// Servidores no compartidos: siempre se puede.
    pub async fn prepare_start(&self, server: &str) -> Result<()> {
        let share = network::load_share(server);
        if !share.enabled { return Ok(()); }
        let Some(l) = lease::read(server) else {
            if !self.inner.is_owner(server) { bail!("Todavía no llegaron los datos del grupo. Esperá unos segundos con la anfitriona conectada."); }
            self.inner.new_lease(server, 0, LeaseKind::Genesis)?;
            return Ok(());
        };
        if l.held_by(&self.inner.me) {
            self.inner.with_state(server, |s| s.fenced = false);
            return Ok(());
        }
        match l.state {
            LeaseState::Hosting => {
                let online = self.inner.recently_ok(server, &l.host, self.inner.timing.host_dead);
                if online {
                    bail!("Lo está hosteando {}. Pedile que te lo pase (botón «Pasar a…» en su 939 Host).", nice(&l.host_name));
                }
                bail!("Lo tenía {} y no responde. Si sabés que se apagó, usá «Recuperar aquí».", nice(&l.host_name));
            }
            LeaseState::Released => {
                let mine = fresh(&snapshots::latest(server));
                if mine < l.required {
                    bail!("La última versión del mundo quedó en la PC de {}, que no está conectada. Esperá a que se conecte para copiarla, o usá «Recuperar aquí» con la versión que tenés.", nice(&l.host_name));
                }
                self.inner.new_lease(server, l.epoch, LeaseKind::Claim)?;
                self.inner.place_latest(server).await?;
                self.inner.broadcast_lease(server).await;
                Ok(())
            }
        }
    }

    /// Pasa el servidor a otra PC del grupo (planificado). `target`: id del nodo, o el mejor disponible.
    pub async fn handover(&self, server: &str, target: Option<&str>) -> Result<String> {
        let inner = &self.inner;
        let l = lease::read(server).ok_or_else(|| anyhow!("Este servidor no está compartido."))?;
        if !l.held_by(&inner.me) { bail!("Solo la PC anfitriona puede pasar el servidor."); }
        inner.tick_server(server).await;
        let target = match target { Some(t) => t.to_string(), None => inner.best_candidate(server, false).ok_or_else(|| anyhow!("No hay otra PC del grupo conectada que pueda hostear."))? };
        if target == inner.me { bail!("Elegí otra PC."); }
        let info = match inner.rpc(&target, &Req::Info { server: server.into() }).await? { Resp::Info(i) => i, _ => bail!("respuesta inesperada") };
        if !info.has_copy || !info.can_host { bail!("{} no puede hostear este servidor (no es coanfitriona o lo tiene desactivado).", nice(&info.name)); }
        // Un solo traspaso a la vez (tomar la marca es atómico).
        let label_busy = format!("Pasando el servidor a {}", info.name);
        if !inner.with_state(server, |s| if s.busy.is_some() { false } else { s.busy = Some(label_busy.clone()); true }) {
            bail!("Ya hay un traspaso en curso.");
        }
        inner.local.changed(server);
        let epoch0 = l.epoch;
        // ¿Esta PC sigue siendo la anfitriona con el mismo permiso? Si otra PC lo tomó mientras
        // tanto (recuperación) o esta PC se aisló, el traspaso se corta y no se vuelve a encender.
        let still_mine = || lease::read(server).map(|l| l.held_by(&inner.me) && l.epoch == epoch0).unwrap_or(false) && !inner.with_state(server, |s| s.fenced);
        let result = async {
            let was_running = inner.local.is_running(server);
            let take = |label: String| { let l2 = inner.local.clone(); let s2 = server.to_string(); async move { tokio::task::spawn_blocking(move || l2.snapshot(&s2, &label, "auto")).await?.map_err(|e| anyhow!(e)) } };
            let target_ref: &str = &target;
            let prefetch = |snap: String| async move {
                match inner.rpc_timeout(target_ref, &Req::Prefetch { server: server.into(), snapshot: snap }, Duration::from_secs(1800)).await {
                    Ok(Resp::Ok) => Ok(()),
                    Ok(Resp::Err { msg }) => Err(anyhow!(msg)),
                    Ok(_) => Err(anyhow!("respuesta inesperada")),
                    Err(e) => Err(e),
                }
            };
            // Fase 1, con el servidor todavía encendido: la otra PC copia casi todo el mundo.
            // Si falla acá, nadie se desconectó y el servidor sigue en esta PC.
            if was_running {
                inner.local.note(server, &format!("Preparando el traspaso a {}: copiando el mundo sin apagar…", info.name));
                let pre = take(format!("Antes del traspaso a {}", info.name)).await?;
                if let Err(e) = prefetch(pre.id.clone()).await {
                    bail!("{} no pudo copiar el mundo ({e}). El servidor sigue en esta PC.", info.name);
                }
                if !still_mine() { bail!("El servidor cambió de anfitriona mientras se preparaba el traspaso."); }
                inner.local.note(server, &format!("Pasando el servidor a {}: guardando y apagando…", info.name));
                inner.blocking(server, |l, s| l.stop(s)).await?;
            }
            // Fase 2: punto final y copia de lo que cambió (poco). Recién con todo copiado se
            // entrega el permiso: así nunca queda una PC con permiso y sin mundo.
            let snap = take(format!("Traspaso a {}", info.name)).await?;
            if let Err(e) = prefetch(snap.id.clone()).await {
                if was_running && still_mine() { let _ = inner.blocking(server, |l, s| l.start(s)).await; }
                bail!("{} no pudo copiar el último estado ({e}). El servidor siguió en esta PC.", info.name);
            }
            if !still_mine() { bail!("El servidor cambió de anfitriona durante el traspaso; esta PC no lo vuelve a encender."); }
            let mut next = lease::next_for(server, &target, &info.name);
            cert::sign(&inner.key, server, &mut next);
            let sent = inner.rpc(&target, &Req::Handover { server: server.into(), lease: next.clone(), snapshot: Some(snap.id.clone()) }).await;
            let accepted = match sent {
                Ok(Resp::Ok) => true,
                Ok(Resp::Err { msg }) => {
                    // Rechazo explícito: la otra PC NO tomó el permiso; volver a encender es seguro.
                    inner.local.note(server, &format!("{} rechazó el traspaso: {msg}", info.name));
                    if was_running && still_mine() { let _ = inner.blocking(server, |l, s| l.start(s)).await; }
                    bail!("No se pudo completar el traspaso; el servidor siguió en esta PC.");
                }
                // Sin respuesta: no se sabe si la otra PC tomó el permiso. Para que nunca haya dos
                // encendidos, se da por entregado (la otra PC ya tiene todo el mundo copiado). Si no
                // llegó, el grupo lo recupera como cuando una anfitriona deja de responder.
                _ => {
                    let confirmed = matches!(inner.rpc(&target, &Req::Info { server: server.into() }).await, Ok(Resp::Info(i)) if i.lease.as_ref() == Some(&next));
                    if !confirmed { inner.local.note(server, &format!("No se pudo confirmar que {} recibió el servidor. Queda apagado en esta PC para proteger el mundo.", info.name)); }
                    true
                }
            };
            let _ = accepted;
            lease::store(server, &next).map_err(|e| anyhow!(e.message))?;
            inner.broadcast_lease(server).await;
            inner.local.note(server, &format!("Servidor pasado a {} (época {}).", info.name, next.epoch));
            Ok(info.name.clone())
        }.await;
        inner.set_busy(server, None);
        inner.local.changed(server);
        result
    }

    /// Apaga y suelta el servidor dejando el último punto copiado en otra PC (si hay alguna).
    /// Devuelve cuántas PCs confirmaron tener el último estado.
    pub async fn release(&self, server: &str) -> Result<usize> {
        let inner = &self.inner;
        let l = lease::read(server).ok_or_else(|| anyhow!("Este servidor no está compartido."))?;
        if !l.held_by(&inner.me) { bail!("Esta PC no es la anfitriona."); }
        inner.set_busy(server, Some("Guardando y soltando el servidor".into()));
        let r = async {
            if inner.local.is_running(server) { inner.blocking(server, |l, s| l.stop(s)).await?; }
            let snap = { let l2 = inner.local.clone(); let s2 = server.to_string(); tokio::task::spawn_blocking(move || l2.snapshot(&s2, "Al soltar el servidor", "auto")).await?.map_err(|e| anyhow!(e))? };
            let need = snap.freshness();
            let share = network::load_share(server);
            let others: Vec<String> = share.members.iter().filter(|m| m.node_id != inner.me && m.can_host()).map(|m| m.node_id.clone()).collect();
            for o in &others { let _ = inner.rpc(o, &Req::SyncNow { server: server.into() }).await; }
            let deadline = Instant::now() + inner.timing.release_wait;
            let mut confirmed = 0;
            while Instant::now() < deadline {
                confirmed = 0;
                for o in &others {
                    if let Ok(Resp::Info(i)) = inner.rpc(o, &Req::Info { server: server.into() }).await {
                        if fresh(&i.latest) >= need { confirmed += 1; }
                    }
                }
                if confirmed > 0 || others.is_empty() { break; }
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            if let Some(mut l) = lease::release(server, need).map_err(|e| anyhow!(e.message))? {
                if l.held_by(&inner.me) || (l.is_free() && l.host == inner.me) {
                    cert::sign(&inner.key, server, &mut l);
                    lease::store(server, &l).map_err(|e| anyhow!(e.message))?;
                }
            }
            inner.broadcast_lease(server).await;
            Ok(confirmed)
        }.await;
        inner.set_busy(server, None);
        inner.local.changed(server);
        r
    }

    /// Toma el servidor en esta PC aunque el anfitrión no responda ("Recuperar aquí").
    /// Se pierde lo que el anfitrión anterior no llegó a sincronizar.
    pub async fn take_here(&self, server: &str) -> Result<()> {
        let inner = &self.inner;
        if !has_copy(server) { bail!("Esta PC no tiene copia de este servidor."); }
        if let Some(l) = lease::read(server) {
            if l.state == LeaseState::Hosting && l.host != inner.me && inner.recently_ok(server, &l.host, inner.timing.host_dead) {
                bail!("{} está respondiendo: pedile que te lo pase.", nice(&l.host_name));
            }
        }
        inner.set_busy(server, Some("Recuperando el servidor en esta PC".into()));
        let r = async {
            let cur = lease::read(server).map(|l| l.epoch).unwrap_or(0);
            let kind = if inner.is_owner(server) { LeaseKind::Owner } else if cur == 0 { bail!("Todavía no llegaron los datos del grupo.") } else { LeaseKind::Takeover };
            inner.new_lease(server, cur, kind)?;
            inner.broadcast_lease(server).await;
            inner.place_latest(server).await?;
            inner.blocking(server, |l, s| l.prepare(s)).await?;
            inner.blocking(server, |l, s| l.start(s)).await?;
            Ok(())
        }.await;
        inner.set_busy(server, None);
        inner.local.changed(server);
        r
    }

    /// Al cerrar la app: cada servidor que hostea esta PC pasa a otra PC encendida (si está activado)
    /// o se apaga y suelta con el último punto copiado. Después avisa que se va.
    pub async fn quit(&self) -> Vec<String> {
        let mut report = vec![];
        for server in network::shared_servers() {
            let Some(l) = lease::read(&server) else { continue };
            if l.held_by(&self.inner.me) {
                let share = network::load_share(&server);
                let mut done = false;
                if share.handover_on_quit && self.inner.local.is_running(&server) {
                    if let Some(_c) = self.inner.best_candidate(&server, false) {
                        match self.handover(&server, None).await {
                            Ok(to) => { report.push(format!("{}: pasado a {to}", server_name(&server))); done = true; }
                            Err(e) => report.push(format!("{}: no se pudo pasar ({e})", server_name(&server))),
                        }
                    }
                }
                if !done {
                    match self.release(&server).await {
                        Ok(n) => report.push(format!("{}: guardado y soltado ({n} PC con la última versión)", server_name(&server))),
                        Err(e) => report.push(format!("{}: {e}", server_name(&server))),
                    }
                }
            }
            let share = network::load_share(&server);
            for m in share.members.iter().filter(|m| m.node_id != self.inner.me) {
                let _ = self.inner.rpc_timeout(&m.node_id, &Req::Bye { server: server.clone() }, Duration::from_secs(2)).await;
            }
        }
        report
    }

    /// Fuerza una vuelta de consulta/sincronización ahora.
    pub async fn sync_now(&self, server: &str) { self.inner.tick_server(server).await; }

    async fn tick(&self, server: &str) { self.inner.tick_server(server).await; }

    pub async fn status(&self, server: &str) -> GroupStatus {
        let inner = &self.inner;
        let share = network::load_share(server);
        let l = lease::read(server);
        let my_latest = snapshots::latest(server);
        let (infos, last_ok, fenced, busy) = {
            let st = inner.state.lock().unwrap();
            st.get(server).map(|s| (s.infos.clone(), s.last_ok.clone(), s.fenced, s.busy.clone())).unwrap_or_default()
        };
        let online_window = inner.timing.poll * 3;
        let conns = inner.conns.lock().await.clone();
        let mut members = vec![];
        for m in &share.members {
            let is_me = m.node_id == inner.me;
            let info = infos.get(&m.node_id);
            let online = is_me || last_ok.get(&m.node_id).map(|t| t.elapsed() < online_window).unwrap_or(false);
            let (path, rtt_ms) = conns.get(&(m.node_id.clone(), ALPN_CTRL)).filter(|c| c.close_reason().is_none()).map(|c| {
                let paths = c.paths();
                let sel = paths.iter().find(|p| p.is_selected());
                (sel.as_ref().map(|p| if p.is_relay() { "relay" } else { "directa" }.to_string()).unwrap_or_default(), sel.as_ref().map(|p| p.rtt().as_millis() as u64))
            }).unwrap_or_default();
            let (running, can_host, score, latest) = if is_me {
                (inner.local.is_running(server), m.can_host() && has_copy(server) && inner.local.allow_hosting(server), inner.local.score(), my_latest.clone())
            } else {
                info.map(|i| (i.running && online, i.can_host, i.score, i.latest.clone())).unwrap_or((false, false, 0, None))
            };
            members.push(MemberStatus {
                node_id: m.node_id.clone(), name: if is_me { inner.name() } else { info.map(|i| i.name.clone()).unwrap_or_else(|| m.name.clone()) },
                symbol: if is_me { inner.symbol() } else { info.map(|i| i.symbol.clone()).unwrap_or_default() },
                role: m.role.clone(), is_me, online, running, is_host: l.as_ref().map(|l| l.held_by(&m.node_id)).unwrap_or(false),
                can_host, score, latest_at: latest.as_ref().map(|s| s.created_at.clone()), fresh: fresh(&latest), path: if is_me { String::new() } else { path }, rtt_ms,
            });
        }
        let i_am_host = l.as_ref().map(|l| l.held_by(&inner.me)).unwrap_or(false);
        let host_online = l.as_ref().map(|l| l.state == LeaseState::Hosting && (l.host == inner.me || inner.recently_ok(server, &l.host, online_window))).unwrap_or(false);
        let can_start = self.can_start_check(server, &l, &my_latest);
        let can_request = self.can_request_check(server);
        // El nombre visible del anfitrión es el actual (el del permiso es el que tenía al tomarlo).
        let mut l = l;
        if let Some(l) = l.as_mut() { if let Some(m) = members.iter().find(|m| m.node_id == l.host) { l.host_name = m.name.clone(); } }
        let (incoming, outgoing, answer, drops) = inner.with_state(server, |s| (s.incoming.clone(), s.outgoing.clone(), s.answer.clone(), s.drops.clone()));
        let ttl = inner.timing.request_ttl;
        let incoming_request = incoming.filter(|_| i_am_host).map(|r| {
            let mine = inner.local.profile();
            let m = members.iter().find(|m| m.node_id == r.node);
            let (path, rtt_ms, online, score, cand_fresh) = m.map(|m| (m.path.clone(), m.rtt_ms, m.online, m.score, m.fresh)).unwrap_or_default();
            let mem = host939_core::instance::load(server).map(|c| c.memory_mb as u64).unwrap_or(2048);
            let (warnings, strengths) = assess(&r.profile, score, &mine, inner.local.score(), mem, &path, rtt_ms, *drops.get(&r.node).unwrap_or(&0));
            RequestView {
                node_id: r.node.clone(), name: r.name.clone(), symbol: r.symbol.clone(), age_secs: r.at.elapsed().as_secs(),
                expires_in_secs: ttl.saturating_sub(r.at.elapsed()).as_secs(), profile: r.profile.clone(), mine, online, path, rtt_ms,
                warnings, strengths, needs_copy: cand_fresh < fresh(&my_latest),
            }
        });
        let outgoing_request = outgoing.map(|o| OutgoingView { host_name: o.host_name.clone(), age_secs: o.at.elapsed().as_secs(), expires_in_secs: ttl.saturating_sub(o.at.elapsed()).as_secs() });
        let last_answer = answer.filter(|a| a.2.elapsed() < Duration::from_secs(90)).map(|a| AnswerView { accepted: a.0, message: a.1 });
        GroupStatus {
            server: server.into(), me: inner.me.clone(), lease: l, i_am_host, host_online, my_latest, has_copy: has_copy(server), fenced, busy,
            can_start, auto_failover: share.auto_failover, handover_on_quit: share.handover_on_quit, majority: majority(share.members.len()), members,
            can_request, incoming_request, outgoing_request, last_answer,
        }
    }

    fn can_start_check(&self, server: &str, l: &Option<Lease>, mine: &Option<SnapshotInfo>) -> Result<(), String> {
        if !has_copy(server) { return Err("Esta PC no tiene copia de este servidor.".into()); }
        let Some(l) = l else { return Ok(()) };
        if l.held_by(&self.inner.me) { return Ok(()); }
        match l.state {
            LeaseState::Hosting => Err(format!("Lo tiene {}.", nice(&l.host_name))),
            LeaseState::Released if fresh(mine) < l.required => Err(format!("La última versión está en la PC de {}.", nice(&l.host_name))),
            LeaseState::Released => Ok(()),
        }
    }
}

impl GroupNode {
    /* ── Nombre visible de esta PC (solo visual: la identidad es la clave del nodo) ── */

    pub fn set_identity(&self, name: &str, symbol: &str) {
        *self.inner.name.write().unwrap() = (name.to_string(), symbol.to_string());
        for server in network::shared_servers() { self.inner.local.changed(&server); }
    }

    /* ── Pedir ser anfitriona ── */

    fn can_request_check(&self, server: &str) -> Result<(), String> {
        let inner = &self.inner;
        let share = network::load_share(server);
        if !share.enabled { return Err("Este servidor no está compartido.".into()); }
        if !share.member(&inner.me).map(|m| m.can_host()).unwrap_or(false) { return Err("Esta PC entró como jugadora: para alojarlo tiene que ser coanfitriona.".into()); }
        if !has_copy(server) { return Err("Esta PC todavía no tiene copia de este servidor.".into()); }
        if !inner.local.allow_hosting(server) { return Err("En esta PC está desactivado «Esta PC puede ser anfitriona» (Configuración → Red).".into()); }
        let Some(l) = lease::read(server) else { return Err("Todavía no hay datos del grupo.".into()) };
        if l.held_by(&inner.me) { return Err("Esta PC ya es la anfitriona.".into()); }
        if l.state != LeaseState::Hosting { return Err("Nadie lo está alojando: podés encenderlo directamente.".into()); }
        if !inner.recently_ok(server, &l.host, inner.timing.poll * 3) {
            return Err(format!("{} no responde. Si se apagó, el grupo lo recupera solo.", nice(&l.host_name)));
        }
        if inner.with_state(server, |s| s.busy.is_some()) { return Err("Hay un traspaso en curso.".into()); }
        Ok(())
    }

    /// Envía una solicitud para alojar el servidor al anfitrión actual.
    pub async fn request_host(&self, server: &str) -> Result<String> {
        let inner = &self.inner;
        if let Some(o) = inner.with_state(server, |s| s.outgoing.clone()) {
            bail!("Ya le pediste a {} alojar este servidor. Esperá su respuesta o cancelá la solicitud.", nice(&o.host_name));
        }
        // Antes de decir «no responde», se le pregunta de verdad a la anfitriona.
        if let Some(l) = lease::read(server) {
            if l.state == LeaseState::Hosting && !l.held_by(&inner.me) { inner.confirm_alive(server, &l.host).await; }
        }
        self.can_request_check(server).map_err(|e| anyhow!(e))?;
        let l = lease::read(server).ok_or_else(|| anyhow!("Este servidor no está compartido."))?;
        let host_name = self.status(server).await.lease.map(|l| l.host_name).unwrap_or_else(|| l.host_name.clone());
        let req = Req::HostRequest { server: server.into(), name: inner.name(), symbol: inner.symbol(), profile: inner.local.profile() };
        match inner.rpc(&l.host, &req).await? {
            Resp::Ok => {}
            Resp::Err { msg } => bail!(msg),
            _ => bail!("respuesta inesperada"),
        }
        inner.with_state(server, |s| { s.outgoing = Some(Outgoing { host: l.host.clone(), host_name: host_name.clone(), at: Instant::now(), epoch: l.epoch }); s.answer = None; });
        inner.local.changed(server);
        Ok(host_name)
    }

    /// Cancela la solicitud pendiente de esta PC.
    pub async fn cancel_host_request(&self, server: &str) -> Result<()> {
        let inner = &self.inner;
        let Some(o) = inner.with_state(server, |s| s.outgoing.take()) else { return Ok(()) };
        let _ = inner.rpc_timeout(&o.host, &Req::HostRequestCancel { server: server.into() }, Duration::from_secs(4)).await;
        inner.local.changed(server);
        Ok(())
    }

    /// La anfitriona responde. Aceptar usa el traspaso planificado de siempre: si algo falla,
    /// el servidor sigue en esta PC con el mundo intacto.
    pub async fn answer_host_request(&self, server: &str, accept: bool) -> Result<String> {
        let inner = &self.inner;
        let Some(r) = inner.with_state(server, |s| s.incoming.take()) else { bail!("Esa solicitud ya no está pendiente.") };
        inner.local.changed(server);
        let answer = |accepted: bool, message: String| {
            let req = Req::HostAnswer { server: server.into(), accepted, message };
            let node = r.node.clone();
            async move { let _ = inner.rpc_timeout(&node, &req, Duration::from_secs(4)).await; }
        };
        if !accept {
            answer(false, format!("{} prefirió seguir alojándolo por ahora.", inner.name())).await;
            inner.local.note(server, &format!("Rechazaste la solicitud de {}.", r.name));
            return Ok(r.name);
        }
        let l = lease::read(server).ok_or_else(|| anyhow!("Este servidor no está compartido."))?;
        if !l.held_by(&inner.me) || l.epoch != r.epoch { bail!("El servidor ya cambió de anfitriona; la solicitud quedó vieja."); }
        if !inner.recently_ok(server, &r.node, inner.timing.poll * 3) { bail!("{} no está conectada ahora: no se puede pasar el servidor.", r.name); }
        answer(true, format!("{} aceptó: preparando el traspaso…", inner.name())).await;
        match self.handover(server, Some(&r.node)).await {
            Ok(to) => Ok(to),
            Err(e) => {
                answer(false, format!("El traspaso no se pudo completar y el servidor siguió en {}: {e}", inner.name())).await;
                Err(e)
            }
        }
    }
}

fn debug(msg: &str) { if std::env::var("H939_DEBUG").is_ok() { eprintln!("[debug] {msg}"); } }

fn nice(name: &str) -> String { if name.trim().is_empty() { "otra PC del grupo".into() } else { name.to_string() } }

/// Datos públicos del servidor que viajan firmados al grupo (nombre, versión y logo de 64×64
/// del servidor; nunca el logo de 939 Host). Así lo ven las PCs que solo juegan y se mantienen
/// al cambiar de anfitriona.
fn card_for(cfg: &host939_core::model::ServerConfig, rev: u64) -> serde_json::Value {
    use base64::Engine;
    let icon = std::fs::read(host939_core::paths::server_run_dir(&cfg.id).join("server-icon.png")).ok()
        .filter(|b| cfg.logo.is_some() && b.len() < 64 * 1024 && host939_core::logo::png_size(b).map(|s| s == (64, 64)).unwrap_or(false))
        .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
    serde_json::json!({ "name": cfg.name, "mcVersion": cfg.mc_version, "loader": format!("{:?}", cfg.loader).to_lowercase(), "rev": rev, "logo": icon })
}

fn bump_rev(s: &mut ShareConfig) {
    let mut card = s.card.take().unwrap_or_else(|| serde_json::json!({}));
    let r = card.get("rev").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    card["rev"] = serde_json::json!(r);
    s.card = Some(card);
}
fn rev(s: &ShareConfig) -> Option<u64> { s.card.as_ref().and_then(|c| c.get("rev")).and_then(|v| v.as_u64()) }

/* ─────────────────────────────── Lógica interna ─────────────────────────────── */

impl Inner {
    /// Guarda un cambio del grupo hecho por la propietaria: nueva revisión, firma y guardado.
    fn commit(&self, server: &str, s: &mut ShareConfig) -> Result<()> {
        if signed::owner_of(s).as_deref() != Some(self.me.as_str()) && s.members.iter().any(|m| m.role == "propietario") {
            // Solo una transferencia (la firma la propietaria saliente) puede nombrar a otra.
            if signed::pin(&network::load_share(server)).as_deref() != Some(self.me.as_str()) { bail!("Solo la PC propietaria puede cambiar el grupo."); }
        }
        if signed::owner_of(s).as_deref() != Some(self.me.as_str()) {
            // Transferencia: la nueva propietaria no hereda el código ni la sucesora.
            s.recovery = None; s.recovery_pending = None; s.successor = None;
        }
        s.succession = None;
        s.rsig = None;
        if s.successor.as_deref().map(|x| !s.member(x).map(|m| m.role == "coanfitrion").unwrap_or(false)).unwrap_or(false) { s.successor = None; }
        bump_rev(s);
        signed::sign(&self.key, server, s);
        s.owner_pin = signed::owner_of(s);
        network::save_share(server, s).map_err(|e| anyhow!(e.message))
    }

    /// Segundos desde que esta PC vio por primera vez un pedido con espera.
    fn seen_for(&self, server: &str, id: &str) -> Option<u64> {
        let m: HashMap<String, i64> = host939_core::paths::read_json(&host939_core::paths::server_dir(server).join("authority-seen.json")).unwrap_or_default();
        m.get(id).map(|t| (chrono::Utc::now().timestamp() - t).max(0) as u64)
    }
    fn mark_seen(&self, server: &str, id: &str) -> bool {
        let p = host939_core::paths::server_dir(server).join("authority-seen.json");
        let mut m: HashMap<String, i64> = host939_core::paths::read_json(&p).unwrap_or_default();
        if m.contains_key(id) { return false; }
        if m.len() > 256 { m.clear(); }
        m.insert(id.to_string(), chrono::Utc::now().timestamp());
        let _ = host939_core::paths::write_json(&p, &m);
        true
    }

    /// Verifica una lista recibida y la adopta (si corresponde). Devuelve si cambió.
    fn adopt_share(&self, server: &str, remote: &ShareConfig) -> bool {
        let mut mine = network::load_share(server);
        let r = signed::rev(remote);
        let owner = signed::owner_of(remote).unwrap_or_default();
        let witnesses = self.with_state(server, |s| s.infos.values().filter(|i| i.share_rev == r && i.owner == owner).count());
        let wait = self.timing.ownership_wait.as_secs();
        let seen = |id: &str| self.seen_for(server, id);
        match signed::accept_with(server, &mine, remote, &signed::Ctx { seen_for: &seen, wait_secs: wait, witnesses }) {
            Ok(()) => {
                let prev_owner = signed::pin(&mine);
                mine.owner_pin = signed::owner_of(remote);
                mine.members = remote.members.clone(); mine.auto_failover = remote.auto_failover; mine.handover_on_quit = remote.handover_on_quit;
                mine.card = remote.card.clone(); mine.sig = remote.sig.clone();
                mine.recovery = remote.recovery.clone(); mine.recovery_pending = remote.recovery_pending.clone();
                mine.successor = remote.successor.clone(); mine.succession = remote.succession.clone(); mine.rsig = remote.rsig.clone();
                if let Some(pk) = &mine.recovery_pending {
                    if self.mark_seen(server, &format!("reset:{pk}")) {
                        self.local.note(server, "La propietaria pidió cambiar el código de recuperación sin el código actual. Se aplica después de la espera de seguridad.");
                    }
                }
                if prev_owner != mine.owner_pin {
                    audit(server, "owner-changed", mine.owner_pin.as_deref().unwrap_or(""));
                    self.local.note(server, "Cambió la propietaria del grupo (verificado con firma).");
                }
                // Si esta PC quedó fuera de la lista (revocada), deja de participar.
                if mine.member(&self.me).is_none() { mine.enabled = false; self.local.note(server, "Esta PC ya no es parte del grupo de este servidor."); }
                let _ = network::save_share(server, &mine);
                true
            }
            Err(why) => {
                debug(&format!("{} rechazó una lista de integrantes: {why}", self.name()));
                false
            }
        }
    }
    /// Emite un permiso nuevo para esta PC (firmado) a partir de la época `expected`.
    fn new_lease(&self, server: &str, expected: u64, kind: LeaseKind) -> Result<Lease> {
        let mut l = lease::draft_claim(server, expected, &self.name(), kind).map_err(|e| anyhow!(e.message))?;
        cert::sign(&self.key, server, &mut l);
        lease::store(server, &l).map_err(|e| anyhow!(e.message))?;
        Ok(l)
    }

    fn witnesses(&self, server: &str, l: &Lease) -> usize {
        let share = network::load_share(server);
        self.with_state(server, |s| s.infos.iter().filter(|(n, i)| share.member(n).is_some() && i.lease.as_ref() == Some(l)).count())
    }

    fn lease_verdict(&self, server: &str, share: &ShareConfig, cur: Option<&Lease>, next: &Lease) -> cert::Verdict {
        let alive = |n: &str| self.recently_ok(server, n, self.timing.host_dead);
        let wit = |l: &Lease| self.witnesses(server, l);
        cert::check(server, cur, next, share, &cert::View { me: &self.me, host_alive: &alive, witnesses: &wit })
    }

    /// Verifica y, si corresponde, adopta un permiso informado por `from`. Si esta PC está
    /// atrasada, pide la cadena a `from` y verifica cada eslabón. Devuelve si cambió.
    async fn accept_lease(self: &Arc<Self>, server: &str, next: &Lease, from: &str) -> bool {
        let share = network::load_share(server);
        let cur = lease::read(server);
        match self.lease_verdict(server, &share, cur.as_ref(), next) {
            cert::Verdict::Accept => lease::store(server, next).is_ok(),
            cert::Verdict::Stale => false,
            cert::Verdict::Reject(why) => {
                debug(&format!("{} rechazó un permiso de {}: {why}", self.name(), &from[..8.min(from.len())]));
                self.with_state(server, |s| s.rejected += 1);
                false
            }
            cert::Verdict::Gap => {
                let after = cur.as_ref().map(|c| c.epoch).unwrap_or(0);
                let chain = match self.rpc_timeout(from, &Req::LeaseChain { server: server.into(), after }, Duration::from_secs(6)).await {
                    Ok(Resp::Leases { leases }) => leases,
                    _ => vec![],
                };
                let alive = |n: &str| self.recently_ok(server, n, self.timing.host_dead);
                let wit = |l: &Lease| self.witnesses(server, l);
                let view = cert::View { me: &self.me, host_alive: &alive, witnesses: &wit };
                match cert::check_chain(server, cur.as_ref(), &chain, next, &share, &view) {
                    Ok(()) => {
                        let mut at = cur.clone();
                        for l in &chain {
                            if at.as_ref().map(|a| l.epoch < a.epoch || l == a).unwrap_or(false) { continue; }
                            if cert::check(server, at.as_ref(), l, &share, &view) == cert::Verdict::Accept {
                                if lease::store(server, l).is_err() { return false; }
                                at = Some(l.clone());
                            }
                            if l == next { break; }
                        }
                        true
                    }
                    Err(why) => {
                        // Ancla de confianza: lo que informa directamente la propietaria (conexión
                        // autenticada), si está bien firmado por quien lo emitió.
                        let from_owner = signed::pin(&share).as_deref() == Some(from);
                        let sig_ok = next.sig.as_deref().map(|s| cert::verify_sig(&next.issuer, &cert::payload(server, next), s)).unwrap_or(false);
                        if from_owner && sig_ok && cur.as_ref().map(|c| next.epoch > c.epoch).unwrap_or(true) {
                            return lease::store(server, next).is_ok();
                        }
                        debug(&format!("{} no pudo verificar la cadena de {}: {why}", self.name(), &from[..8.min(from.len())]));
                        self.with_state(server, |s| s.rejected += 1);
                        false
                    }
                }
            }
        }
    }

    fn is_owner(&self, server: &str) -> bool { network::load_share(server).member(&self.me).map(|m| m.role == "propietario").unwrap_or(false) }

    fn name(&self) -> String { self.name.read().unwrap().0.clone() }
    fn symbol(&self) -> String { self.name.read().unwrap().1.clone() }

    /// Guarda cómo llegar a una PC del grupo que se conectó a esta (lo que ya sabe iroh por la
    /// conexión autenticada). Así la propietaria puede pasarle a las coanfitrionas cómo llegar a
    /// cada jugadora (para votar y retransmitir el permiso), sin que las jugadoras se conecten entre sí.
    async fn learn_from(&self, node: EndpointId) {
        if let Some(info) = self.ep.remote_info(node).await {
            let a = EndpointAddr::from_parts(node, info.addrs().map(|x| x.addr().clone()));
            if !a.is_empty() { self.learn(a); }
        }
    }

    fn learn(&self, addr: EndpointAddr) {
        // Con relays no hace falta guardar IPs de otras PCs (se las encuentra por su identidad).
        let addr = host939_p2p::shareable(&addr, self.relays);
        let key = addr.id.to_string();
        if key == self.me { return; }
        let mut p = self.peers.write().unwrap();
        let changed = p.get(&key) != Some(&addr) && !addr.is_empty();
        if changed {
            p.insert(key, addr);
            let snapshot: HashMap<String, serde_json::Value> = p.iter().filter_map(|(k, v)| serde_json::to_value(v).ok().map(|v| (k.clone(), v))).collect();
            let _ = network::save_peers(&snapshot);
        }
    }

    fn with_state<R>(&self, server: &str, f: impl FnOnce(&mut SrvState) -> R) -> R {
        let mut st = self.state.lock().unwrap();
        f(st.entry(server.to_string()).or_default())
    }

    fn set_busy(&self, server: &str, b: Option<String>) { self.with_state(server, |s| s.busy = b); self.local.changed(server); }

    /// Si no hay una respuesta reciente de `node`, lo consulta en el momento (hasta 3 s).
    async fn confirm_alive(self: &Arc<Self>, server: &str, node: &str) -> bool {
        if self.recently_ok(server, node, self.timing.poll * 3) { return true; }
        match self.rpc_timeout(node, &Req::Info { server: server.into() }, Duration::from_secs(3)).await {
            Ok(Resp::Info(info)) => {
                self.with_state(server, |s| { s.last_ok.insert(node.to_string(), Instant::now()); s.infos.insert(node.to_string(), info); });
                true
            }
            _ => false,
        }
    }

    fn recently_ok(&self, server: &str, node: &str, within: Duration) -> bool {
        self.with_state(server, |s| s.last_ok.get(node).map(|t| t.elapsed() < within).unwrap_or(false))
    }

    async fn blocking(&self, server: &str, f: impl FnOnce(&dyn LocalHost, &str) -> Result<(), String> + Send + 'static) -> Result<()> {
        let l = self.local.clone();
        let s = server.to_string();
        tokio::task::spawn_blocking(move || f(l.as_ref(), &s)).await?.map_err(|e| anyhow!(e))
    }

    async fn conn(&self, node: &str, alpn: &'static [u8]) -> Result<Connection> {
        let key = (node.to_string(), alpn);
        if let Some(c) = self.conns.lock().await.get(&key) {
            if c.close_reason().is_none() { return Ok(c.clone()); }
        }
        let id: EndpointId = node.parse().map_err(|_| anyhow!("id de nodo inválido"))?;
        let addr = self.peers.read().unwrap().get(node).cloned().unwrap_or_else(|| EndpointAddr::new(id));
        let c = tokio::time::timeout(Duration::from_secs(6), self.ep.connect(addr, alpn)).await.context("sin respuesta")??;
        self.conns.lock().await.insert(key, c.clone());
        Ok(c)
    }

    async fn rpc(&self, node: &str, req: &Req) -> Result<Resp> { self.rpc_timeout(node, req, Duration::from_secs(8)).await }

    async fn rpc_timeout(&self, node: &str, req: &Req, t: Duration) -> Result<Resp> {
        let fut = async {
            let c = self.conn(node, ALPN_CTRL).await?;
            let (mut send, mut recv) = c.open_bi().await?;
            wire::write_msg(&mut send, req).await?;
            send.finish()?;
            let r: Resp = wire::read_msg(&mut recv).await?;
            Ok::<_, anyhow::Error>(r)
        };
        match tokio::time::timeout(t, fut).await {
            Ok(Ok(r)) => Ok(r),
            Ok(Err(e)) => { self.conns.lock().await.remove(&(node.to_string(), ALPN_CTRL)); Err(e) }
            Err(_) => { self.conns.lock().await.remove(&(node.to_string(), ALPN_CTRL)); Err(anyhow!("sin respuesta")) }
        }
    }

    /// Lo que esta PC informa de sí misma a `requester`. Mínimo conocimiento: a una jugadora solo
    /// lo necesario para jugar (quién aloja, si está encendido, la revisión del grupo); nunca
    /// direcciones de red, puntos de restauración ni capacidad de hardware. Las direcciones solo
    /// van a quien puede hostear, y sin IPs cuando hay relays.
    fn my_info(&self, server: &str, requester: &str) -> Info {
        let share = network::load_share(server);
        let copy = has_copy(server);
        let trusted = requester == self.me || share.member(requester).map(|m| m.can_host()).unwrap_or(false);
        let peers = if trusted {
            let p = self.peers.read().unwrap();
            share.members.iter().filter(|m| m.node_id != requester).filter_map(|m| p.get(&m.node_id).map(|a| host939_p2p::shareable(a, self.relays))).collect()
        } else { vec![] };
        let base = Info {
            node: self.me.clone(), name: self.name(), symbol: self.symbol(), lease: lease::read(server),
            latest: None, has_copy: false, running: self.local.is_running(server), can_host: false, score: 0,
            share_rev: rev(&share).unwrap_or(0), owner: signed::owner_of(&share).unwrap_or_default(), addr: None, peers: vec![],
        };
        if !trusted { return base; }
        Info {
            latest: if copy { snapshots::latest(server) } else { None }, has_copy: copy,
            can_host: copy && share.member(&self.me).map(|m| m.can_host()).unwrap_or(false) && self.local.allow_hosting(server),
            score: self.local.score(), addr: Some(host939_p2p::shareable(&self.ep.addr(), self.relays)), peers, ..base
        }
    }

    /// Mejor PC para hostear entre esta y las conectadas: estado más reciente, después mejor
    /// hardware, después id menor (para que todas elijan la misma).
    fn best_candidate(&self, server: &str, include_me: bool) -> Option<String> {
        let share = network::load_share(server);
        let window = self.timing.poll * 3;
        let mut c: Vec<((u64, u64), u32, String)> = vec![];
        if include_me && has_copy(server) && share.member(&self.me).map(|m| m.can_host()).unwrap_or(false) && self.local.allow_hosting(server) {
            c.push((fresh(&snapshots::latest(server)), self.local.score(), self.me.clone()));
        }
        let l = lease::read(server);
        self.with_state(server, |s| {
            for (node, info) in &s.infos {
                let ok = s.last_ok.get(node).map(|t| t.elapsed() < window).unwrap_or(false);
                let is_host = l.as_ref().map(|l| l.held_by(node)).unwrap_or(false);
                if ok && !is_host && info.can_host && share.member(node).map(|m| m.can_host()).unwrap_or(false) {
                    c.push((fresh(&info.latest), info.score, node.clone()));
                }
            }
        });
        c.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        c.into_iter().next().map(|x| x.2)
    }

    async fn broadcast_lease(&self, server: &str) {
        let Some(l) = lease::read(server) else { return };
        let share = network::load_share(server);
        let futs = share.members.iter().filter(|m| m.node_id != self.me).map(|m| {
            let req = Req::Lease { server: server.into(), lease: l.clone() };
            let node = m.node_id.clone();
            async move { let _ = self.rpc_timeout(&node, &req, Duration::from_secs(4)).await; }
        });
        futures_util::future::join_all(futs).await;
    }

    /// Coloca en disco el punto más reciente (si lo actual es distinto, lo actual queda como seguridad).
    async fn place_latest(&self, server: &str) -> Result<()> {
        let Some(latest) = snapshots::latest(server) else { return Ok(()) };
        let s = server.to_string();
        let mc = mc_version(server);
        tokio::task::spawn_blocking(move || snapshots::materialize(&s, &latest.id, &mc)).await?.map_err(|e| anyhow!(e.message))
    }

    /// Esta PC pasó a ser la anfitriona: copia lo que falte, coloca el estado y enciende.
    async fn become_host(self: &Arc<Self>, server: &str, from: Option<String>, snap: Option<String>) {
        self.set_busy(server, Some("Recibiendo el servidor".into()));
        let r = async {
            if let (Some(from), Some(snap)) = (&from, &snap) {
                let mut last = None;
                for _ in 0..4 {
                    match self.pull(server, from, snap).await { Ok(_) => { last = None; break; } Err(e) => { last = Some(e); tokio::time::sleep(Duration::from_secs(2)).await; } }
                }
                if let Some(e) = last { bail!("no se pudo copiar el último estado: {e}"); }
            }
            self.place_latest(server).await?;
            self.local.note(server, "Preparando el servidor en esta PC…");
            self.blocking(server, |l, s| l.prepare(s)).await?;
            self.blocking(server, |l, s| l.start(s)).await?;
            Ok(())
        }.await;
        self.set_busy(server, None);
        match r {
            Ok(()) => self.local.note(server, "Esta PC es ahora la anfitriona y el servidor está en línea."),
            Err(e) => self.local.note(server, &format!("Esta PC tiene el permiso de anfitriona pero no pudo encender: {e}. Probá encenderlo a mano.")),
        }
        self.local.changed(server);
    }

    /// Copia un punto desde otra PC (solo los bloques que faltan).
    async fn pull(&self, server: &str, from: &str, snap: &str) -> Result<SnapshotInfo> {
        if snapshots::has_snapshot(server, snap) {
            if let Some(i) = snapshots::list(server).into_iter().find(|s| s.id == snap) { return Ok(i); }
        }
        let c = self.conn(from, ALPN_SYNC).await?;
        let (mut send, mut recv) = c.open_bi().await?;
        wire::write_msg(&mut send, &SyncReq::Manifest { server: server.into(), snap: snap.into() }).await?;
        send.finish()?;
        let man = wire::read_frame(&mut recv, 64 * 1024 * 1024).await?;
        if man.is_empty() { bail!("la otra PC ya no tiene ese punto"); }
        let missing = snapshots::missing_blobs(server, &man).map_err(|e| anyhow!(e.message))?;
        let total = missing.len();
        let started = Instant::now();
        for chunk in missing.chunks(128) {
            let (mut send, mut recv) = c.open_bi().await?;
            wire::write_msg(&mut send, &SyncReq::Blobs { server: server.into(), hashes: chunk.to_vec() }).await?;
            send.finish()?;
            for h in chunk {
                let data = wire::read_frame(&mut recv, snapshots::BLOCK).await?;
                if data.is_empty() { bail!("a la otra PC le falta un bloque"); }
                snapshots::put_blob(server, h, &data).map_err(|e| anyhow!(e.message))?;
            }
        }
        let info = snapshots::import_manifest(server, &man).map_err(|e| anyhow!(e.message))?;
        if total > 0 {
            self.local.note(server, &format!("Sincronizado «{}»: {} bloques nuevos en {:.1} s.", info.label, total, started.elapsed().as_secs_f32()));
        }
        Ok(info)
    }

    /// Detiene el servidor local porque otra PC tiene el permiso (o se perdió el contacto).
    async fn fence(self: &Arc<Self>, server: &str, reason: &str) {
        let already = self.with_state(server, |s| std::mem::replace(&mut s.fenced, true));
        if already && !self.local.is_running(server) { return; }
        self.local.note(server, &format!("Deteniendo el servidor en esta PC: {reason}. Tu copia queda apartada como punto de seguridad."));
        let _ = self.blocking(server, |l, s| l.stop(s)).await;
        let label = format!("Copia apartada: {reason}");
        let l2 = self.local.clone();
        let s2 = server.to_string();
        let _ = tokio::task::spawn_blocking(move || l2.snapshot(&s2, &label, "seguridad")).await;
        self.local.changed(server);
    }

    /// Vencen las solicitudes viejas, las que perdieron sentido (cambió el anfitrión) y las de
    /// PCs que dejaron de responder. Si el anfitrión desaparece, actúa la recuperación de siempre.
    fn expire_requests(&self, server: &str) {
        let l = lease::read(server);
        let ttl = self.timing.request_ttl;
        let dead = self.timing.host_dead;
        let mut notes = vec![];
        self.with_state(server, |s| {
            if let Some(r) = &s.incoming {
                let gone = s.last_ok.get(&r.node).map(|t| t.elapsed() > dead).unwrap_or(true);
                let stale = !l.as_ref().map(|l| l.held_by(&self.me) && l.epoch == r.epoch).unwrap_or(false);
                if r.at.elapsed() > ttl || gone || stale {
                    if r.at.elapsed() > ttl { notes.push(format!("La solicitud de {} venció sin respuesta.", r.name)); }
                    s.incoming = None;
                }
            }
            if let Some(o) = &s.outgoing {
                let epoch_now = l.as_ref().map(|l| l.epoch).unwrap_or(0);
                let host_gone = s.last_ok.get(&o.host).map(|t| t.elapsed() > dead).unwrap_or(true);
                if epoch_now != o.epoch {
                    if l.as_ref().map(|l| l.held_by(&self.me)).unwrap_or(false) { s.answer = Some((true, "Esta PC es ahora la anfitriona.".into(), Instant::now())); }
                    s.outgoing = None;
                } else if host_gone {
                    s.answer = Some((false, format!("{} dejó de responder. Si no vuelve, el grupo recupera el servidor automáticamente.", nice(&o.host_name)), Instant::now()));
                    s.outgoing = None;
                } else if o.at.elapsed() > ttl {
                    s.answer = Some((false, format!("{} no respondió a tiempo. Podés volver a pedirlo.", nice(&o.host_name)), Instant::now()));
                    s.outgoing = None;
                }
            }
        });
        for n in notes { self.local.note(server, &n); }
    }

    async fn after_lease_change(self: &Arc<Self>, server: &str) {
        self.expire_requests(server);
        let l = lease::read(server);
        if let Some(l) = &l {
            if !l.held_by(&self.me) && self.local.is_running(server) && !(l.is_free() && l.host == self.me) {
                self.fence(server, &format!("{} tomó el servidor", nice(&l.host_name))).await;
            }
        }
        self.local.changed(server);
    }

    async fn tick_server(self: &Arc<Self>, server: &str) {
        let share = network::load_share(server);
        if !share.enabled { return; }
        // Una jugadora solo habla con quien aloja y con la propietaria: no necesita saber nada de
        // las demás PCs (ni su dirección ni cuándo están conectadas).
        let i_play = share.member(&self.me).map(|m| !m.can_host()).unwrap_or(true);
        let host_now = lease::read(server).filter(|l| l.state == LeaseState::Hosting).map(|l| l.host);
        let owner_now = signed::pin(&share);
        let others: Vec<Member> = share.members.iter().filter(|m| m.node_id != self.me)
            .filter(|m| !i_play || Some(&m.node_id) == host_now.as_ref() || Some(&m.node_id) == owner_now.as_ref() || m.can_host() && host_now.is_none())
            .cloned().collect();
        // Cada PC se consulta por separado y su respuesta cuenta en cuanto llega: una PC lenta
        // (por ejemplo, la anfitriona vieja apagando su servidor) no puede hacer que las demás
        // parezcan caídas. No se manda otra consulta a una PC que todavía no respondió la anterior.
        let mut handles = vec![];
        for m in &others {
            let node = m.node_id.clone();
            if !self.with_state(server, |s| s.inflight.insert(node.clone())) { continue; }
            let me = self.clone();
            let srv = server.to_string();
            handles.push(tokio::spawn(async move {
                let r = me.rpc_timeout(&node, &Req::Info { server: srv.clone() }, Duration::from_secs(4)).await;
                me.with_state(&srv, |s| {
                    s.inflight.remove(&node);
                    match r {
                        Ok(Resp::Info(info)) => {
                            s.last_ok.insert(node.clone(), Instant::now());
                            s.left.remove(&node);
                            s.inbox.push((node.clone(), info.clone()));
                            s.infos.insert(node, info);
                        }
                        other => {
                            debug(&format!("{} sin respuesta de {}: {:?}", me.name(), &node[..4], other.as_ref().err()));
                            // Contar cortes: respondía hace poco y ahora no.
                            if s.last_ok.get(&node).map(|t| t.elapsed() < me.timing.poll * 2).unwrap_or(false) { *s.drops.entry(node).or_default() += 1; }
                        }
                    }
                });
            }));
        }
        // Se espera un momento a las respuestas rápidas; las lentas siguen y se procesan después.
        let _ = tokio::time::timeout(self.timing.poll * 2, futures_util::future::join_all(handles)).await;
        let inbox = self.with_state(server, |s| std::mem::take(&mut s.inbox));
        let mut newest_share: Option<(u64, String)> = None;
        let mut names: Vec<(String, String)> = vec![];
        for (node, info) in inbox {
            names.push((node.clone(), info.name.clone()));
            if let Some(a) = &info.addr { self.learn(a.clone()); }
            for p in &info.peers { self.learn(p.clone()); }
            if let Some(remote) = &info.lease {
                if self.accept_lease(server, remote, &node).await { self.after_lease_change(server).await; }
            }
            if info.share_rev > rev(&share).unwrap_or(0) && newest_share.as_ref().map(|(r, _)| info.share_rev > *r).unwrap_or(true) {
                newest_share = Some((info.share_rev, node.clone()));
            }
        }
        // La PC propietaria guarda los nombres visibles actuales para quien esté desconectado.
        let me_owner = share.member(&self.me).map(|m| m.role == "propietario").unwrap_or(false);
        if me_owner && newest_share.is_none() {
            let my_name = self.name();
            let mut s2 = network::load_share(server);
            let mut touched = false;
            for m in s2.members.iter_mut() {
                let want = if m.node_id == self.me { Some(my_name.clone()) } else { names.iter().find(|(n, _)| *n == m.node_id).map(|(_, v)| v.clone()) };
                if let Some(w) = want.filter(|w| !w.trim().is_empty() && *w != m.name) { m.name = w; touched = true; }
            }
            // Datos públicos al día (nombre, versión, logo) y firmados.
            if let Ok(cfg) = host939_core::instance::load(server) {
                let card = card_for(&cfg, rev(&s2).unwrap_or(0));
                let differs = s2.card.as_ref().map(|c| { let mut a = c.clone(); let mut b = card.clone(); a["rev"] = 0.into(); b["rev"] = 0.into(); a != b }).unwrap_or(true);
                if differs { s2.card = Some(card); touched = true; }
            }
            // Grupos anteriores a las firmas: la propietaria firma su lista en el primer contacto.
            if touched || s2.sig.is_none() { let _ = self.commit(server, &mut s2); self.local.changed(server); }
        }
        if let Some((_, from)) = newest_share {
            if let Ok(Resp::Config { share: remote, .. }) = self.rpc(&from, &Req::Config { server: server.into() }).await {
                if self.adopt_share(server, &remote) { self.local.changed(server); }
            }
        }

        self.expire_requests(server);
        let l = lease::read(server);
        let copy = has_copy(server);
        let busy = self.with_state(server, |s| s.busy.is_some());
        if std::env::var("H939_DEBUG").is_ok() {
            let ages: Vec<String> = self.with_state(server, |s| s.last_ok.iter().map(|(k, t)| format!("{}:{:.1}", &k[..4], t.elapsed().as_secs_f32())).collect());
            debug(&format!("{} tick lease={:?} busy={busy} ages={ages:?}", self.name(), l.as_ref().map(|l| (l.epoch, &l.host[..4], l.state))));
        }
        if busy {
            // Durante un traspaso la anfitriona sigue vigilando su contacto con el grupo: si lo
            // pierde, se aísla igual (nunca dos encendidas, aunque el traspaso dure minutos).
            if l.as_ref().map(|l| l.held_by(&self.me)).unwrap_or(false) { self.host_checks(server, &share).await; }
            return;
        }
        match &l {
            Some(l) if l.held_by(&self.me) => self.host_checks(server, &share).await,
            Some(l) if copy => {
                self.sync_from_best(server).await;
                if l.state == LeaseState::Hosting && share.auto_failover {
                    self.failover_check(server, l, &share).await;
                }
            }
            None if copy => self.sync_from_best(server).await,
            _ => {}
        }
    }

    async fn host_checks(self: &Arc<Self>, server: &str, share: &ShareConfig) {
        if !share.auto_failover { return; }
        let n = share.members.len();
        let (lost, back, fenced) = self.with_state(server, |s| {
            let now = Instant::now();
            let mut lost = 0;
            let mut back = 0;
            for m in share.members.iter().filter(|m| m.node_id != self.me) {
                if s.left.contains(&m.node_id) { continue; }
                if let Some(t) = s.last_ok.get(&m.node_id) {
                    let age = now.duration_since(*t);
                    if age < self.timing.poll * 2 { back += 1; }
                    else if age > self.timing.fence && age < self.timing.seen_window { lost += 1; }
                }
            }
            (lost, back, s.fenced)
        });
        if self.local.is_running(server) && lost >= majority(n) {
            self.fence(server, "se perdió el contacto con la mayoría del grupo").await;
        } else if fenced && !self.local.is_running(server) && back > 0 && lost < majority(n) {
            // Volvió el contacto y nadie tomó el permiso: se reanuda en esta PC.
            self.with_state(server, |s| s.fenced = false);
            self.local.note(server, "Volvió el contacto con el grupo y nadie tomó el servidor: reanudando en esta PC.");
            let _ = self.blocking(server, |l, s| l.start(s)).await;
            self.local.changed(server);
        }
    }

    async fn sync_from_best(self: &Arc<Self>, server: &str) {
        let mine = fresh(&snapshots::latest(server));
        let window = self.timing.poll * 3;
        let best = self.with_state(server, |s| {
            if s.syncing { return None; }
            s.infos.iter()
                .filter(|(n, _)| s.last_ok.get(*n).map(|t| t.elapsed() < window).unwrap_or(false))
                .filter_map(|(n, i)| i.latest.as_ref().map(|l| (l.freshness(), n.clone(), l.id.clone())))
                .filter(|(f, _, _)| *f > mine)
                .max_by(|a, b| a.0.cmp(&b.0))
        });
        let Some((_, node, snap)) = best else { return };
        self.with_state(server, |s| s.syncing = true);
        let r = self.pull(server, &node, &snap).await;
        self.with_state(server, |s| s.syncing = false);
        match r {
            Ok(_) => { let s = server.to_string(); let _ = tokio::task::spawn_blocking(move || snapshots::prune(&s)).await; self.local.changed(server); }
            Err(e) => self.local.note(server, &format!("No se pudo sincronizar: {e}")),
        }
    }

    async fn failover_check(self: &Arc<Self>, server: &str, l: &Lease, share: &ShareConfig) {
        let since = self.with_state(server, |s| s.last_ok.get(&l.host).copied()).unwrap_or(self.started);
        if since.elapsed() < self.timing.host_dead { return; }
        let best = self.best_candidate(server, true);
        debug(&format!("{}: anfitrión sin respuesta hace {:?}; mejor candidata {:?}", self.name(), since.elapsed(), best.as_ref().map(|b| &b[..8])));
        let Some(best) = best else { return };
        if best != self.me { return; }
        let my_fresh = fresh(&snapshots::latest(server));
        let need = majority(share.members.len());
        let mut votes = 1;
        let mut voters = vec![self.name()];
        let mut signed_votes: Vec<(String, String)> = vec![];
        let msg = cert::vote_payload(server, l.epoch, &self.me);
        for m in share.members.iter().filter(|m| m.node_id != self.me && m.node_id != l.host) {
            let req = Req::Vote { server: server.into(), epoch: l.epoch, candidate: self.me.clone(), fresh: my_fresh };
            if let Ok(Resp::Vote { agree: true, sig: Some(sig) }) = self.rpc_timeout(&m.node_id, &req, Duration::from_secs(4)).await {
                if !cert::verify_sig(&m.node_id, &msg, &sig) { continue; }
                votes += 1;
                voters.push(m.name.clone());
                signed_votes.push((m.node_id.clone(), sig));
            }
        }
        debug(&format!("{}: votos {votes}/{need} ({})", self.name(), voters.join(", ")));
        if votes < need { return; }
        // Puede que otra PC haya avanzado mientras tanto: falla si la época cambió.
        let Ok(mut next) = lease::draft_claim(server, l.epoch, &self.name(), LeaseKind::Elected) else { return };
        next.votes = signed_votes;
        cert::sign(&self.key, server, &mut next);
        if lease::store(server, &next).is_err() { return; }
        self.local.note(server, &format!("{} no responde. Elegida esta PC por mayoría ({}): levantando el servidor acá.", nice(&l.host_name), voters.join(", ")));
        self.broadcast_lease(server).await;
        self.become_host(server, None, None).await;
    }

    /// Lleva una conexión de un jugador a la PC anfitriona.
    async fn route_one(self: &Arc<Self>, server: &str, tcp: TcpStream) -> Result<()> {
        let l = lease::read(server);
        let name = server_name(server);
        let busy = self.with_state(server, |s| s.busy.clone());
        let serving_here = l.as_ref().map(|l| l.held_by(&self.me)).unwrap_or(false) && self.local.is_running(server);
        if let (Some(b), false) = (busy, serving_here) {
            return moving::respond(tcp, &name, &format!("{b}… volvé a entrar en unos segundos.")).await;
        }
        match &l {
            Some(l) if l.held_by(&self.me) => {
                if self.local.is_running(server) {
                    let port = self.local.mc_port(server);
                    let up = TcpStream::connect(("127.0.0.1", port)).await?;
                    let _ = up.set_nodelay(true);
                    let (mut a, mut b) = (tcp, up);
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                    Ok(())
                } else {
                    moving::respond(tcp, &name, "Esta PC tiene el servidor pero está apagado. Encendelo en 939 Host.").await
                }
            }
            Some(l) if l.state == LeaseState::Hosting => {
                // 1) Directo a la anfitriona.
                if let Ok(bi) = self.open_mc(&l.host, server).await { return pipe(tcp, bi).await; }
                // 2) A través de otra PC del grupo que sí llegue.
                let window = self.timing.poll * 3;
                let bridges: Vec<String> = self.with_state(server, |s| s.last_ok.iter().filter(|(n, t)| **n != l.host && t.elapsed() < window).map(|(n, _)| n.clone()).collect());
                for b in bridges {
                    if let Ok(bi) = self.open_bridge(&b, server, &l.host).await { return pipe(tcp, bi).await; }
                }
                moving::respond(tcp, &name, &format!("La PC anfitriona ({}) no responde. Si se apagó, el grupo lo levanta en otra PC.", nice(&l.host_name))).await
            }
            Some(l) => moving::respond(tcp, &name, &format!("Nadie lo está hosteando ahora (lo soltó {}). Cualquiera del grupo puede encenderlo en 939 Host.", nice(&l.host_name))).await,
            None => moving::respond(tcp, &name, "Todavía no hay datos del grupo. Esperá unos segundos.").await,
        }
    }

    async fn open_mc(&self, node: &str, server: &str) -> Result<(SendStream, RecvStream)> {
        let c = self.conn(node, ALPN_MC).await?;
        let (mut send, recv) = c.open_bi().await?;
        wire::write_frame(&mut send, server.as_bytes()).await?;
        Ok((send, recv))
    }

    async fn open_bridge(&self, via: &str, server: &str, target: &str) -> Result<(SendStream, RecvStream)> {
        let c = self.conn(via, ALPN_BRIDGE).await?;
        let (mut send, recv) = c.open_bi().await?;
        wire::write_msg(&mut send, &BridgeReq { server: server.into(), target: target.into() }).await?;
        Ok((send, recv))
    }

    /// Solo la propietaria y las coanfitrionas reciben copias del mundo (las necesitan para
    /// poder alojarlo). Una jugadora nunca, aunque pida los bloques directamente.
    fn may_sync(&self, server: &str, node: &str) -> bool {
        let ok = network::load_share(server).member(node).map(|m| m.can_host()).unwrap_or(false);
        if !ok && self.is_member(server, node) {
            self.local.note(server, "Se rechazó un pedido de copia del mundo de una PC jugadora.");
            audit(server, "sync-denied", node);
        }
        ok
    }

    fn is_member(&self, server: &str, node: &str) -> bool {
        if !host939_core::paths::valid_server_id(server) { return false; }
        !server.contains(['/', '\\', '.']) && network::load_share(server).member(node).is_some()
    }
}

async fn pipe(tcp: TcpStream, (mut send, mut recv): (SendStream, RecvStream)) -> Result<()> {
    let (mut r, mut w) = tcp.into_split();
    let up = async { let _ = tokio::io::copy(&mut r, &mut send).await; let _ = send.finish(); };
    let down = async { let _ = tokio::io::copy(&mut recv, &mut w).await; let _ = w.shutdown().await; };
    tokio::join!(up, down);
    Ok(())
}

/* ─────────────────────────────── Protocolos entrantes ─────────────────────────────── */

#[derive(Clone)]
struct Handler(Arc<Inner>);
impl std::fmt::Debug for Handler { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "ctrl") } }

impl ProtocolHandler for Handler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        let mut learned = false;
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            let learn = !std::mem::replace(&mut learned, true);
            let rid = conn.remote_id();
            tokio::spawn(async move {
                let Ok(req) = wire::read_msg::<_, Req>(&mut recv).await else { return };
                if !host939_core::paths::valid_server_id(req.server()) {
                    let _ = wire::write_msg(&mut send, &Resp::Err { msg: "Servidor inválido.".into() }).await;
                    let _ = send.finish();
                    return;
                }
                if learn && inner.is_member(req.server(), &remote) { inner.learn_from(rid).await; }
                let resp = if inner.is_member(req.server(), &remote) {
                    inner.with_state(req.server(), |s| { s.last_ok.insert(remote.clone(), Instant::now()); });
                    handle(&inner, &remote, req).await
                } else {
                    Resp::Err { msg: "Esta PC no es parte del grupo de ese servidor.".into() }
                };
                let _ = wire::write_msg(&mut send, &resp).await;
                let _ = send.finish();
                let _ = send.stopped().await;
            });
        }
    }
}

async fn handle(inner: &Arc<Inner>, remote: &str, req: Req) -> Resp {
    match req {
        Req::Info { server } => {
            // Solo pruebas: simula una PC que responde lento (por ejemplo, ocupada apagando).
            #[cfg(feature = "pruebas")]
            if let Some(ms) = std::env::var("H939_TEST_SLOW_INFO_MS").ok().and_then(|v| v.parse::<u64>().ok()) { tokio::time::sleep(Duration::from_millis(ms)).await; }
            Resp::Info(inner.my_info(&server, remote))
        }
        Req::Config { server } => {
            let mut share = network::load_share(&server);
            share.invites.clear();
            // La configuración completa solo a quien puede hostear, y sin lo que depende de esta PC
            // (memoria, argumentos de Java, errores con rutas locales).
            let trusted = share.member(remote).map(|m| m.can_host()).unwrap_or(false);
            let config = if trusted {
                host939_core::instance::load(&server).ok().map(|mut c| { c.extra_jvm_args.clear(); c.install_error = None; c.java_choice = None; c }).and_then(|c| serde_json::to_value(c).ok())
            } else { None };
            Resp::Config { config, share }
        }
        Req::Lease { server, lease: l } => {
            if inner.accept_lease(&server, &l, remote).await { inner.after_lease_change(&server).await; }
            Resp::Ok
        }
        Req::Succession { server, claim } => {
            let share = network::load_share(&server);
            if claim["successor"].as_str() != Some(remote) { return Resp::Err { msg: "El reclamo no es de esta PC.".into() }; }
            match signed::check_claim(&server, &share, &claim) {
                Ok(_) => {
                    if inner.mark_seen(&server, &signed::claim_id(&claim)) {
                        let name = share.member(remote).map(|m| m.name.clone()).unwrap_or_default();
                        let hours = inner.timing.ownership_wait.as_secs() / 3600;
                        let msg = if inner.is_owner(&server) {
                            format!("{} inició la sucesión de la propiedad porque esta PC no respondía. Si seguís a cargo, cancelala en Grupo (tenés {hours} h).", nice(&name))
                        } else {
                            format!("{} inició la sucesión de la propiedad. Se completa en {hours} h si la propietaria no la cancela.", nice(&name))
                        };
                        inner.local.note(&server, &msg);
                        audit(&server, "succession-claim", remote);
                        inner.with_state(&server, |s| s.claim = Some(claim.clone()));
                        inner.local.changed(&server);
                    }
                    Resp::Ok
                }
                Err(why) => Resp::Err { msg: why.into() },
            }
        }
        Req::LeaseChain { server, after } => {
            Resp::Leases { leases: lease::history(&server).into_iter().filter(|l| l.epoch > after || (l.epoch == after && l.state == LeaseState::Released)).take(512).collect() }
        }
        Req::Handover { server, lease: l, snapshot } => {
            if l.host != inner.me { return Resp::Err { msg: "El permiso no es para esta PC.".into() }; }
            if l.kind != LeaseKind::Handover || l.issuer != remote {
                return Resp::Err { msg: "Quien pasa el servidor no es la anfitriona actual según esta PC.".into() };
            }
            if !has_copy(&server) || !inner.local.allow_hosting(&server) { return Resp::Err { msg: "Esta PC no puede hostear este servidor.".into() }; }
            if !inner.accept_lease(&server, &l, remote).await || lease::read(&server).as_ref() != Some(&l) {
                return Resp::Err { msg: "Quien pasa el servidor no es la anfitriona actual según esta PC.".into() };
            }
            let inner2 = inner.clone();
            let from = remote.to_string();
            tokio::spawn(async move { inner2.become_host(&server, Some(from), snapshot).await; });
            Resp::Ok
        }
        Req::Vote { server, epoch, candidate, fresh: cand } => {
            let l = lease::read(&server);
            let mine = fresh(&snapshots::latest(&server));
            let agree = inner.with_state(&server, |s| {
                let Some(l) = &l else { return false };
                if l.epoch != epoch || l.state != LeaseState::Hosting || l.host == candidate || l.host == inner.me { return false; }
                let host_seen = s.last_ok.get(&l.host).copied().unwrap_or(inner.started);
                if host_seen.elapsed() < inner.timing.host_dead.mul_f32(0.8) { return false; }
                if cand < mine { return false; }
                if !network::load_share(&server).member(&candidate).map(|m| m.can_host()).unwrap_or(false) { return false; }
                if candidate != remote { return false; }
                match &s.voted {
                    Some((e, c)) if *e == epoch && *c != candidate => false,
                    _ => { s.voted = Some((epoch, candidate.clone())); true }
                }
            });
            let sig = agree.then(|| cert::sign_vote(&inner.key, &server, epoch, &candidate));
            Resp::Vote { agree, sig }
        }
        Req::SyncNow { server } => {
            let inner2 = inner.clone();
            tokio::spawn(async move { inner2.tick_server(&server).await; });
            Resp::Ok
        }
        Req::Bye { server } => {
            inner.with_state(&server, |s| { s.left.insert(remote.to_string()); s.last_ok.remove(remote); });
            Resp::Ok
        }
        Req::Leave { server } => {
            if !inner.is_owner(&server) { return Resp::Err { msg: "Esta PC no es la propietaria.".into() }; }
            if lease::read(&server).map(|l| l.held_by(remote)).unwrap_or(false) { return Resp::Err { msg: "Esa PC aloja el servidor ahora: primero tiene que pasarlo a otra.".into() }; }
            let mut s = network::load_share(&server);
            let name = s.member(remote).map(|m| m.name.clone());
            s.members.retain(|m| m.node_id != remote || m.role == "propietario");
            match inner.commit(&server, &mut s) {
                Ok(()) => { if let Some(n) = name { inner.local.note(&server, &format!("{} salió del grupo.", nice(&n))); } inner.local.changed(&server); Resp::Ok }
                Err(e) => Resp::Err { msg: e.to_string() },
            }
        }
        Req::HostRequest { server, name, symbol, profile } => {
            let Some(l) = lease::read(&server).filter(|l| l.held_by(&inner.me)) else { return Resp::Err { msg: "Esta PC ya no aloja el servidor.".into() } };
            if !network::load_share(&server).member(remote).map(|m| m.can_host()).unwrap_or(false) {
                return Resp::Err { msg: "Solo las PCs coanfitrionas pueden alojar el servidor.".into() };
            }
            if inner.with_state(&server, |s| s.infos.get(remote).map(|i| !i.can_host).unwrap_or(false)) {
                return Resp::Err { msg: "Esa PC no tiene copia del servidor o tiene desactivado hostear.".into() };
            }
            let name: String = pcname_or(&name);
            let symbol = if host939_core::pcname::valid_symbol(&symbol) { symbol } else { String::new() };
            let ttl = inner.timing.request_ttl;
            let r = inner.with_state(&server, |s| {
                if s.busy.is_some() { return Err("La anfitriona está en medio de un traspaso. Probá en un rato.".to_string()); }
                if let Some(cur) = &s.incoming {
                    if cur.node != remote && cur.at.elapsed() < ttl {
                        return Err(format!("Ya hay una solicitud pendiente de {}. Probá en unos minutos.", cur.name));
                    }
                }
                let fresh_req = s.incoming.as_ref().map(|c| c.node != remote).unwrap_or(true);
                s.incoming = Some(Incoming { node: remote.to_string(), name: name.clone(), symbol, profile, at: Instant::now(), epoch: l.epoch });
                Ok(fresh_req)
            });
            match r {
                Ok(fresh_req) => {
                    if fresh_req {
                        inner.local.note(&server, &format!("{name} quiere alojar este servidor."));
                        inner.local.host_request(&server, &name);
                    }
                    inner.local.changed(&server);
                    Resp::Ok
                }
                Err(msg) => Resp::Err { msg },
            }
        }
        Req::HostRequestCancel { server } => {
            let name = inner.with_state(&server, |s| if s.incoming.as_ref().map(|c| c.node == remote).unwrap_or(false) { s.incoming.take().map(|c| c.name) } else { None });
            if let Some(n) = name { inner.local.note(&server, &format!("{n} canceló su solicitud.")); inner.local.changed(&server); }
            Resp::Ok
        }
        Req::HostAnswer { server, accepted, message } => {
            let mine = inner.with_state(&server, |s| {
                if s.outgoing.as_ref().map(|o| o.host == remote).unwrap_or(false) {
                    if !accepted { s.outgoing = None; }
                    s.answer = Some((accepted, message.clone(), Instant::now()));
                    true
                } else if !accepted && s.answer.as_ref().map(|a| a.0).unwrap_or(false) {
                    // El traspaso aceptado falló después: se avisa igual.
                    s.answer = Some((false, message.clone(), Instant::now()));
                    true
                } else { false }
            });
            if mine { inner.local.note(&server, &message); inner.local.changed(&server); }
            Resp::Ok
        }
        Req::Prefetch { server, snapshot } => {
            let cur = lease::read(&server);
            if !cur.as_ref().map(|c| c.held_by(remote)).unwrap_or(false) { return Resp::Err { msg: "Quien pide copiar no es la anfitriona según esta PC.".into() }; }
            if !has_copy(&server) || !inner.local.allow_hosting(&server) { return Resp::Err { msg: "Esta PC no puede hostear este servidor.".into() }; }
            // Solo en compilaciones de prueba (característica `pruebas`): nunca en la app que se entrega.
            #[cfg(feature = "pruebas")]
            {
                if std::env::var("H939_TEST_FAIL_PREFETCH").is_ok() { return Resp::Err { msg: "falla simulada al copiar".into() }; }
                if let Some(secs) = std::env::var("H939_TEST_SLOW_PREFETCH").ok().and_then(|v| v.parse::<u64>().ok()) { tokio::time::sleep(Duration::from_secs(secs)).await; }
            }
            // Una sola copia a la vez (la sincronización periódica espera).
            for _ in 0..600 {
                if inner.with_state(&server, |s| if s.syncing { false } else { s.syncing = true; true }) { break; }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let r = inner.pull(&server, remote, &snapshot).await;
            inner.with_state(&server, |s| s.syncing = false);
            if let Err(e) = r { return Resp::Err { msg: format!("no se pudo copiar el mundo: {e}") }; }
            // También deja instalado lo necesario (Java, servidor, loader) antes de aceptar el
            // permiso, para que el encendido posterior no falle por una descarga.
            match inner.blocking(&server, |l, s| l.prepare(s)).await {
                Ok(()) => Resp::Ok,
                Err(e) => Resp::Err { msg: format!("no se pudo preparar el servidor: {e}") },
            }
        }
    }
}

/// Comparación en tiempo constante (secretos de invitación).
fn ct_eq(a: &[u8], b: &[u8]) -> bool { a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0 }

/// Ventana deslizante simple: `per_node` intentos por minuto por PC y `total` en total.
struct RateLimit { per_node: usize, total: usize, hits: HashMap<String, Vec<Instant>>, all: Vec<Instant> }
impl RateLimit {
    fn new(per_node: usize, total: usize) -> Self { RateLimit { per_node, total, hits: HashMap::new(), all: vec![] } }
    fn allow(&mut self, who: &str) -> bool {
        let now = Instant::now();
        let win = Duration::from_secs(60);
        self.all.retain(|t| now.duration_since(*t) < win);
        self.hits.retain(|_, v| { v.retain(|t| now.duration_since(*t) < win); !v.is_empty() });
        if self.hits.len() > 4096 { self.hits.clear(); }
        let mine = self.hits.entry(who.to_string()).or_default();
        if mine.len() >= self.per_node || self.all.len() >= self.total { return false; }
        mine.push(now);
        self.all.push(now);
        true
    }
}

/// Registro local de operaciones sensibles (`audit.jsonl` del servidor): qué, quién y cuándo.
/// Sin contenido de archivos, IPs ni secretos.
pub fn audit(server: &str, what: &str, who: &str) {
    use std::io::Write;
    let p = host939_core::paths::server_dir(server).join("audit.jsonl");
    if std::fs::metadata(&p).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) { let _ = std::fs::rename(&p, p.with_extension("jsonl.1")); }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let line = serde_json::json!({ "at": chrono::Utc::now().to_rfc3339(), "what": what, "who": who });
        let _ = writeln!(f, "{line}");
    }
}

fn pcname_or(name: &str) -> String { host939_core::pcname::clean_name(name).unwrap_or_else(|| "Otra PC".into()) }

#[derive(Clone)]
struct SyncH(Arc<Inner>);
impl std::fmt::Debug for SyncH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "sync") } }
impl ProtocolHandler for SyncH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let Ok(req) = wire::read_msg::<_, SyncReq>(&mut recv).await else { return };
                match req {
                    SyncReq::Manifest { server, snap } => {
                        let bytes = if inner.may_sync(&server, &remote) { snapshots::manifest_bytes(&server, &snap).unwrap_or_default() } else { vec![] };
                        let _ = wire::write_frame(&mut send, &bytes).await;
                    }
                    SyncReq::Blobs { server, hashes } => {
                        let ok = inner.may_sync(&server, &remote);
                        for h in hashes.iter().take(1024) {
                            let data = if ok { snapshots::read_blob(&server, h).unwrap_or_default() } else { vec![] };
                            if wire::write_frame(&mut send, &data).await.is_err() { break; }
                        }
                    }
                }
                let _ = send.finish();
                let _ = send.stopped().await;
            });
        }
    }
}

#[derive(Clone)]
struct McH(Arc<Inner>);
impl std::fmt::Debug for McH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "mc") } }
impl ProtocolHandler for McH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        loop {
            let Ok((send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let Ok(sid) = wire::read_frame(&mut recv, 256).await else { return };
                let server = String::from_utf8_lossy(&sid).to_string();
                let mut send = send;
                if !inner.is_member(&server, &remote) { let _ = send.reset(1u32.into()); return; }
                let mine = lease::read(&server).map(|l| l.held_by(&inner.me)).unwrap_or(false);
                if mine && inner.local.is_running(&server) {
                    let Ok(tcp) = TcpStream::connect(("127.0.0.1", inner.local.mc_port(&server))).await else { let _ = send.reset(2u32.into()); return; };
                    let _ = tcp.set_nodelay(true);
                    let (mut r, mut w) = tcp.into_split();
                    let up = async { let _ = tokio::io::copy(&mut recv, &mut w).await; };
                    let down = async { let _ = tokio::io::copy(&mut r, &mut send).await; let _ = send.finish(); };
                    tokio::join!(up, down);
                } else {
                    let name = server_name(&server);
                    let msg = if mine { "Iniciando en la nueva PC… volvé a entrar en unos segundos." } else { "Esta PC ya no es la anfitriona; reintentá." };
                    let _ = moving::respond(tokio::io::join(recv, send), &name, msg).await;
                }
            });
        }
    }
}

#[derive(Clone)]
struct BridgeH(Arc<Inner>);
impl std::fmt::Debug for BridgeH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "bridge") } }
impl ProtocolHandler for BridgeH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let Ok(req) = wire::read_msg::<_, BridgeReq>(&mut recv).await else { return };
                if !inner.local.allow_bridge() || !inner.is_member(&req.server, &remote) || !inner.is_member(&req.server, &req.target) || req.target == inner.me {
                    let _ = send.reset(1u32.into());
                    return;
                }
                let Ok((mut us, mut ur)) = inner.open_mc(&req.target, &req.server).await else { let _ = send.reset(2u32.into()); return; };
                let up = async { let _ = tokio::io::copy(&mut recv, &mut us).await; let _ = us.finish(); };
                let down = async { let _ = tokio::io::copy(&mut ur, &mut send).await; let _ = send.finish(); };
                tokio::join!(up, down);
            });
        }
    }
}

#[derive(Clone)]
struct JoinH(Arc<Inner>);
impl std::fmt::Debug for JoinH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "join") } }
impl ProtocolHandler for JoinH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
        let inner = &self.0;
        let reply = match wire::read_msg::<_, JoinReq>(&mut recv).await {
            Ok(req) => {
                // Límite de intentos antes de autenticar (por PC y en total): adivinar un secreto de
                // 128 bits no es viable, pero así tampoco se puede usar para cargar a esta PC.
                let limited = !inner.join_limit.lock().unwrap().allow(&remote);
                let found = if limited || req.secret.len() != 32 { None } else { network::shared_servers().into_iter().find_map(|sid| {
                    let s = network::load_share(&sid);
                    s.invites.iter().find(|i| ct_eq(i.secret.as_bytes(), req.secret.as_bytes())).cloned().map(|i| (sid, i))
                }) };
                // Vencida: se borra y se rechaza igual que una inexistente.
                let found = match found {
                    Some((sid, inv)) if inv.expired() => {
                        let mut s = network::load_share(&sid);
                        s.invites.retain(|i| i.secret != inv.secret);
                        let _ = network::save_share(&sid, &s);
                        inner.local.note(&sid, "Alguien intentó usar una invitación vencida; se borró.");
                        audit(&sid, "invite-expired-used", &remote);
                        None
                    }
                    f => f,
                };
                match found {
                    Some((sid, inv)) => {
                        let mut s = network::load_share(&sid);
                        let name: String = req.name.trim().chars().take(32).collect();
                        let role = if inv.role == "coanfitrion" && has_copy(&sid) { "coanfitrion" } else { "jugador" };
                        s.members.retain(|m| m.node_id != remote);
                        s.members.push(Member { node_id: remote.clone(), name: if name.is_empty() { "Amigo".into() } else { name.clone() }, joined_at: chrono::Local::now().to_rfc3339(), role: role.into() });
                        // Cada invitación sirve para una sola PC.
                        s.invites.retain(|i| i.secret != req.secret);
                        let _ = inner.commit(&sid, &mut s);
                        inner.local.note(&sid, &format!("{} se unió al grupo como {}.", if name.is_empty() { "Un amigo" } else { &name }, role));
                        inner.local.changed(&sid);
                        let mut out = s.clone();
                        out.invites.clear();
                        audit(&sid, "join", &remote);
                        inner.learn_from(conn.remote_id()).await;
                        // Direcciones para llegar al grupo: con relays, solo identidad y relay (sin IPs).
                        let peers = { let p = inner.peers.read().unwrap(); s.members.iter().filter(|m| m.node_id != remote).filter_map(|m| p.get(&m.node_id).map(|a| host939_p2p::shareable(a, inner.relays))).collect() };
                        JoinResp { ok: true, error: None, server: Some(sid), share: Some(out), role: Some(role.into()), peers }
                    }
                    None if limited => JoinResp { ok: false, error: Some("Demasiados intentos. Esperá un minuto.".into()), server: None, share: None, role: None, peers: vec![] },
                    None => JoinResp { ok: false, error: Some("La invitación no es válida, venció, ya se usó o fue revocada.".into()), server: None, share: None, role: None, peers: vec![] },
                }
            }
            Err(_) => JoinResp { ok: false, error: Some("Solicitud inválida.".into()), server: None, share: None, role: None, peers: vec![] },
        };
        let _ = wire::write_msg(&mut send, &reply).await;
        let _ = send.finish();
        let _ = send.stopped().await;
        Ok(())
    }
}

