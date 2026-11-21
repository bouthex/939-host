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
pub mod friends;
pub mod moving;
pub mod signed;
pub mod wire;
mod entrance;
pub use entrance::EntranceInfo;

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
/// Voz (UDP de Simple Voice Chat) por datagramas QUIC, cifrados como el resto.
pub const ALPN_VOICE: &[u8] = b"939host/voice/1";

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
    /// ¿El servidor terminó (detención, cierre forzado o caída) sin que después se tomara un
    /// punto? Devuelve true una sola vez por salida.
    fn exited_unsaved(&self, _server: &str) -> bool { false }
    /// Operación larga del grupo en curso (traspaso, sincronización…), para mostrarla en la
    /// interfaz: etapa y progreso real si se conoce. `op_end` la cierra (con error o no).
    fn op(&self, _server: &str, _title: &str, _stage: &str, _fraction: Option<f64>) {}
    fn op_end(&self, _server: &str, _title: &str, _error: Option<&str>) {}
    /// Comando en la consola del servidor encendido (para aplicar el OP que concedió la propietaria).
    fn command(&self, _server: &str, _cmd: &str) -> Result<(), String> { Err("sin consola".into()) }
    /// Estado detallado del servidor en esta PC: stopped | starting | preparing | ready | running |
    /// saving | restarting | error, con un detalle en palabras y la cantidad de jugadores.
    /// Llegó un mensaje de un amigo (para avisar en la interfaz).
    fn friend_message(&self, _node: &str) {}
    fn run_state(&self, server: &str) -> (String, String, u32) { (if self.is_running(server) { "running" } else { "stopped" }.into(), String::new(), 0) }
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
    /// «Abrime una conexión nueva»: lo pide una PC que acaba de ver que esta se reinició, para que
    /// su transporte deje de usar el camino viejo (ver `serve_ctrl`).
    Nudge { server: String },
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
    /// Pide la foto de perfil con esa huella (solo entre integrantes del grupo).
    Avatar { server: String, hash: String },
    /// «Probá mi entrada pública desde tu conexión» (ping de estado de Minecraft).
    ProbeEntrance { server: String, addr: String },
}

impl Req {
    fn server(&self) -> &str {
        match self {
            Req::Info { server } | Req::Config { server } | Req::Lease { server, .. } | Req::Handover { server, .. }
            | Req::Vote { server, .. } | Req::SyncNow { server } | Req::Bye { server }
            | Req::HostRequest { server, .. } | Req::HostRequestCancel { server } | Req::HostAnswer { server, .. } | Req::Prefetch { server, .. } | Req::Leave { server } | Req::LeaseChain { server, .. } | Req::Succession { server, .. } | Req::Avatar { server, .. } | Req::Nudge { server } | Req::ProbeEntrance { server, .. } => server,
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
    /// Mods recibidos apartados en esta PC esperando aprobación: (nombre, huella). Solo para quien puede hostear.
    #[serde(default)]
    pub pending_mods: Vec<(String, String)>,
    pub addr: Option<EndpointAddr>,
    #[serde(default)]
    pub peers: Vec<EndpointAddr>,
    /// Huella SHA-256 de la foto de perfil de esta PC (la foto se pide aparte, una vez).
    #[serde(default)]
    pub avatar: Option<String>,
    /// Estado detallado del servidor en esta PC (ver `LocalHost::run_state`), el detalle y los
    /// jugadores conectados. Con un traspaso o una recuperación en curso: transferring / recovering.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub state_detail: String,
    #[serde(default)]
    pub players: u32,
    /// Puerto UDP de voz (Simple Voice Chat) del servidor encendido en esta PC.
    #[serde(default)]
    pub voice_port: Option<u16>,
    /// Entrada pública de esta PC para el servidor (jugadores sin 939 Host), si la tiene abierta.
    #[serde(default)]
    pub entrance: Option<EntranceInfo>,
}

/// Estado del servidor que ven todas las PCs del grupo (el de la PC que lo aloja).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HostState { pub state: String, pub detail: String, pub players: u32, pub host_name: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Resp {
    Info(Info),
    Config { config: Option<serde_json::Value>, share: ShareConfig },
    Ok,
    Vote { agree: bool, #[serde(default)] sig: Option<String> },
    Leases { leases: Vec<Lease> },
    /// Foto de perfil (PNG en base64).
    Avatar { png: String },
    /// Resultado de probar una entrada pública.
    Probe { ok: bool, detail: String },
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
    /// Mods pendientes de los que ya se avisó a la propietaria (por huella).
    mods_noticed: HashSet<String>,
    /// Hay una solicitud para alojar y acá el servidor está apagado: se acepta sola.
    auto_answer: bool,
    /// OP sin autorizar vistos en `ops.json` y desde cuándo: se quitan recién después de un margen
    /// (así una lista firmada que todavía viene en camino no hace quitar un OP legítimo).
    op_unauthorized: HashMap<String, Instant>,
    /// PCs a las que todavía no les llegó el aviso del estado final (estaban desconectadas o esta
    /// PC recién arrancaba): se les reenvía en cuanto responden.
    final_pending: HashSet<String>,
    /// Fotos de perfil que se están pidiendo (por huella).
    avatar_fetch: HashSet<String>,
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
    /// Conexiones de control que abre esta PC: una tarea fija también atiende por ellas los pedidos
    /// de la otra PC (ver `serve_ctrl`).
    serve_tx: tokio::sync::mpsc::UnboundedSender<Connection>,
    state: Mutex<HashMap<String, SrvState>>,
    routes: tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    stopping: std::sync::atomic::AtomicBool,
    /// Límite de intentos de unión (antes de autenticar).
    join_limit: Mutex<RateLimit>,
    /// Pedidos de control por PC (antes de mirar si es del grupo: cada uno lee disco).
    ctrl_limit: Mutex<RateLimit>,
    /// «Sincronizá ya»: cada uno dispara una vuelta de consultas a todo el grupo.
    sync_now_limit: Mutex<RateLimit>,
    /// Pedidos de datos (sincronización y archivos) por PC. Una copia de 2 GB son ~260 pedidos.
    data_limit: Mutex<RateLimit>,
    /// Presencia en vivo de los amigos y mensajes sin leer (ver `friends`).
    friends_live: Mutex<HashMap<String, friends::Live>>,
    unread: Mutex<HashMap<String, usize>>,
    /// Conexiones de voz abiertas por PC (con tope).
    voice_slots: Mutex<HashMap<String, usize>>,
    /// Entradas públicas abiertas en esta PC, por servidor (ver `entrance`).
    entrances: Mutex<HashMap<String, entrance::Open>>,
    /// Comprobaciones de entradas pedidas por otras PCs.
    probe_limit: Mutex<RateLimit>,
}

/// Estado de un servidor compartido tal como lo ve esta PC (para la interfaz).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupStatus {
    /// Estado del servidor en la PC que lo aloja (el mismo para todas las PCs del grupo).
    pub host_state: Option<HostState>,
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
    /// Huella de su foto de perfil (si esta PC ya la tiene guardada).
    pub avatar: Option<String>,
    /// Cuenta de Minecraft con OP concedido por la propietaria.
    pub mc_op: Option<String>,
}

fn majority(n: usize) -> usize { n / 2 + 1 }

fn fresh(s: &Option<SnapshotInfo>) -> (u64, u64) { s.as_ref().map(|s| s.freshness()).unwrap_or((0, 0)) }

/// ¿Se puede copiar de `source` un punto de la época `snap_epoch`? El anuncio de un punto (época
/// y secuencia en `Info`) y el índice que se baja no tienen firma: los escribe quien los manda.
/// Por eso solo se copia de quien puede hostear según la lista firmada (nunca de una jugadora),
/// nunca de una época que esta PC todavía no verificó, y la época actual solo de la anfitriona
/// actual (la única que crea puntos en esa época). Sin esto, un integrante podía anunciar un punto
/// «más nuevo que todos» y hacer que el resto lo copiara y lo levantara al alojar.
pub fn sync_source_ok(cur: Option<&Lease>, share: &ShareConfig, source: &str, snap_epoch: u64) -> bool {
    let Some(c) = cur else { return false };
    if !share.member(source).map(|m| m.can_host()).unwrap_or(false) { return false; }
    if snap_epoch > c.epoch { return false; }
    snap_epoch < c.epoch || c.host == source
}

/// Frescura de un punto anunciado por `node`, o (0, 0) si no se puede copiar de esa PC.
fn usable_fresh(cur: Option<&Lease>, share: &ShareConfig, node: &str, latest: &Option<SnapshotInfo>) -> (u64, u64) {
    match latest { Some(l) if sync_source_ok(cur, share, node, l.epoch) => l.freshness(), _ => (0, 0) }
}

/// ¿Esta PC puede votar por `candidate` para reemplazar a la anfitriona de la época `epoch`?
/// Sin estado: el voto único por época se controla aparte. `cand` es la frescura que anuncia la
/// candidata (sin firma): no puede ser de una época que esta PC no verificó.
#[allow(clippy::too_many_arguments)]
fn vote_conditions(l: &Lease, me: &str, epoch: u64, candidate: &str, remote: &str, cand: (u64, u64), mine: (u64, u64), host_dead: bool, candidate_can_host: bool) -> bool {
    if l.epoch != epoch || l.state != LeaseState::Hosting || l.host == candidate || l.host == me { return false; }
    if !host_dead { return false; }
    if cand < mine || cand.0 > l.epoch { return false; }
    candidate_can_host && candidate == remote
}

/// ¿`next` reemplaza a `cur` por desempate? Solo entre dos tomas del mismo permiso soltado.
fn claim_conflict_wins(cur: &Lease, next: &Lease) -> bool {
    cur.epoch == next.epoch && cur.state == LeaseState::Hosting && next.state == LeaseState::Hosting
        && cur.kind == LeaseKind::Claim && next.kind == LeaseKind::Claim && cur.prev == next.prev
        && cur.host != next.host && next.host < cur.host
}

fn is_sha256(h: &str) -> bool { h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) }

/// Nombres con OP en `ops.json` del servidor (vacío si no existe).
fn read_ops(server: &str) -> Vec<String> {
    host939_core::paths::read_json::<Vec<serde_json::Value>>(&host939_core::paths::server_run_dir(server).join("ops.json")).unwrap_or_default()
        .iter().filter_map(|o| o.get("name").and_then(|n| n.as_str()).map(String::from)).collect()
}

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
        let alpns = vec![ALPN_CTRL.to_vec(), ALPN_SYNC.to_vec(), ALPN_MC.to_vec(), ALPN_BRIDGE.to_vec(), ALPN_JOIN.to_vec(), files::ALPN_FILES.to_vec(), ALPN_VOICE.to_vec(), friends::ALPN_FRIEND.to_vec()];
        let ep = host939_p2p::endpoint(secret, relays, alpns).await?;
        let me = ep.id().to_string();
        lease::set_local_node(&me);
        let mut peers = HashMap::new();
        for (k, v) in network::load_peers() {
            if let Ok(a) = serde_json::from_value::<EndpointAddr>(v) { peers.insert(k, a); }
        }
        let (serve_tx, mut serve_rx) = tokio::sync::mpsc::unbounded_channel::<Connection>();
        let inner = Arc::new(Inner {
            ep: ep.clone(), relays, key: iroh::SecretKey::from_bytes(&secret), me, name: RwLock::new((name.to_string(), String::new())), local, timing, started: Instant::now(),
            peers: RwLock::new(peers), conns: Default::default(), state: Default::default(), routes: Default::default(),
            stopping: Default::default(), join_limit: Mutex::new(RateLimit::new(5, 30)),
            ctrl_limit: Mutex::new(RateLimit::new(CTRL_PER_MIN, 20_000)), sync_now_limit: Mutex::new(RateLimit::new(12, 200)),
            data_limit: Mutex::new(RateLimit::new(3000, 50_000)),
            friends_live: Default::default(), unread: Default::default(), voice_slots: Default::default(),
            entrances: Default::default(), probe_limit: Mutex::new(RateLimit::new(6, 120)),
            serve_tx,
        });
        {
            let inner = inner.clone();
            tokio::spawn(async move { while let Some(c) = serve_rx.recv().await { tokio::spawn(serve_ctrl(inner.clone(), c)); } });
        }
        let h = Handler(inner.clone());
        let router = Router::builder(ep)
            .accept(ALPN_CTRL, h.clone()).accept(ALPN_SYNC, SyncH(inner.clone())).accept(ALPN_MC, McH(inner.clone()))
            .accept(ALPN_BRIDGE, BridgeH(inner.clone())).accept(ALPN_JOIN, JoinH(inner.clone())).accept(files::ALPN_FILES, files::FilesH(inner.clone()))
            .accept(ALPN_VOICE, VoiceH(inner.clone())).accept(friends::ALPN_FRIEND, friends::FriendH(inner.clone()))
            .spawn();
        let node = GroupNode { inner: inner.clone(), router: Arc::new(router) };
        let n2 = node.clone();
        // Presencia de amigos: una vuelta cada 30 s (mucho más liviana que la del grupo).
        let n3 = node.clone();
        tokio::spawn(async move {
            loop {
                if n3.inner.stopping.load(std::sync::atomic::Ordering::Relaxed) { break; }
                n3.presence_once().await;
                tokio::time::sleep(friends::PRESENCE_EVERY).await;
            }
        });
        tokio::spawn(async move {
            loop {
                if n2.inner.stopping.load(std::sync::atomic::Ordering::Relaxed) { break; }
                let servers = network::shared_servers();
                futures_util::future::join_all(servers.iter().map(|s| n2.tick(s))).await;
                for s in &servers { n2.auto_answer(s); }
                n2.sync_entrances().await;
                tokio::time::sleep(n2.inner.timing.poll).await;
            }
        });
        Ok(node)
    }

    /// Acepta una solicitud para alojar mientras el servidor está apagado en esta PC.
    fn auto_answer(&self, server: &str) {
        if !self.inner.with_state(server, |s| std::mem::take(&mut s.auto_answer)) { return; }
        if self.inner.local.is_running(server) { return; }
        let Some(name) = self.inner.with_state(server, |s| s.incoming.as_ref().map(|r| r.name.clone())) else { return };
        let inner = self.inner.clone();
        let srv = server.to_string();
        tokio::spawn(async move {
            inner.local.note(&srv, &format!("{name} quiere encenderlo y acá está apagado: se le pasa el servidor con el último estado."));
            if let Err(e) = inner.answer_host_request(&srv, true).await { inner.local.note(&srv, &format!("No se pudo pasar el servidor: {e}")); }
        });
    }

    pub fn id(&self) -> String { self.inner.me.clone() }
    pub fn addr(&self) -> EndpointAddr { self.inner.ep.addr() }
    pub fn endpoint(&self) -> &Endpoint { &self.inner.ep }

    /// Cierra el nodo (sin traspasos: para eso está `quit`).
    pub async fn shutdown(&self) {
        self.inner.stopping.store(true, std::sync::atomic::Ordering::Relaxed);
        for (_, t) in self.inner.routes.lock().await.drain() { t.abort(); }
        self.inner.entrances.lock().unwrap().clear();
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

    /// Mods esperando aprobación en cualquier PC del grupo (lo que informan las que pueden hostear
    /// y los de esta PC): (nombre, huella, PC).
    pub fn pending_mods(&self, server: &str) -> Vec<(String, String, String)> {
        let mut v: Vec<(String, String, String)> = host939_core::modguard::pending(server).into_iter().map(|(n, h)| (n, h, self.inner.name())).collect();
        self.inner.with_state(server, |s| for i in s.infos.values() { for (n, h) in &i.pending_mods { v.push((n.clone(), h.clone(), i.name.clone())); } });
        let approved: HashSet<String> = network::load_share(server).approved_mods.into_iter().collect();
        v.retain(|x| !approved.contains(&x.1));
        v.sort(); v.dedup_by(|a, b| a.1 == b.1);
        v
    }

    /// La propietaria aprueba mods por su huella exacta (firma la lista nueva). Aprobar quita un
    /// rechazo anterior de la misma huella.
    pub fn approve_mods(&self, server: &str, hashes: &[String]) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria aprueba mods."); }
        let mut s = network::load_share(server);
        for h in hashes {
            if !is_sha256(h) { bail!("Huella inválida."); }
            if !s.approved_mods.contains(h) { s.approved_mods.push(h.clone()); }
            s.rejected_mods.retain(|r| r != h);
        }
        self.inner.commit(server, &mut s)?;
        for h in hashes { host939_core::modguard::log_decision(server, h, "aprobado"); }
        audit(server, "mods-approved", &hashes.join(","));
        Ok(())
    }

    /// La propietaria rechaza mods por su huella (firma la lista nueva): no se cargan nunca en
    /// ninguna PC del grupo. Con `discard`, además se borran de la cuarentena de esta PC.
    pub fn reject_mods(&self, server: &str, hashes: &[String], discard: bool) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria rechaza mods."); }
        let mut s = network::load_share(server);
        for h in hashes {
            if !is_sha256(h) { bail!("Huella inválida."); }
            s.approved_mods.retain(|a| a != h);
            if !s.rejected_mods.contains(h) { s.rejected_mods.push(h.clone()); }
        }
        self.inner.commit(server, &mut s)?;
        for h in hashes {
            host939_core::modguard::log_decision(server, h, "rechazado");
            if discard && host939_core::modguard::discard(server, h) > 0 { host939_core::modguard::log_decision(server, h, "descartado"); }
        }
        // Si alguno estaba en la carpeta activa (por ejemplo, aprobado antes), se aparta ya.
        let _ = host939_core::modguard::enforce(server);
        audit(server, "mods-rejected", &hashes.join(","));
        Ok(())
    }

    /// La propietaria elige la política de mods del grupo.
    pub fn set_mod_policy(&self, server: &str, trust_cohosts: bool) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria cambia esta política."); }
        let mut s = network::load_share(server);
        s.mod_policy = Some(if trust_cohosts { "confiar-coanfitrionas" } else { "estricta" }.into());
        self.inner.commit(server, &mut s)?;
        audit(server, "mod-policy", if trust_cohosts { "confiar-coanfitrionas" } else { "estricta" });
        Ok(())
    }

    /// La propietaria concede OP de Minecraft a un integrante (por su cuenta de Minecraft).
    /// Queda en la lista firmada: nadie más puede concederlo, ni a sí mismo.
    pub fn grant_mc_op(&self, server: &str, node: &str, mc_name: &str) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria concede el OP de Minecraft."); }
        self.inner.init_mc_ops(server)?;
        if !host939_core::players::valid_name(mc_name) { bail!("Ese no es un nombre de Minecraft válido (3 a 16 letras, números o _)."); }
        let mut s = network::load_share(server);
        if s.member(node).is_none() { bail!("Esa PC no es del grupo."); }
        s.mc_ops.retain(|o| o.node_id != node && !o.mc_name.eq_ignore_ascii_case(mc_name));
        s.mc_ops.push(network::McOp { node_id: node.into(), mc_name: mc_name.into() });
        s.mc_ops_revoked.retain(|n| !n.eq_ignore_ascii_case(mc_name));
        self.inner.commit(server, &mut s)?;
        audit(server, "mc-op-grant", mc_name);
        Ok(())
    }

    /// La propietaria quita el OP concedido: la anfitriona se lo quita en el servidor.
    pub fn revoke_mc_op(&self, server: &str, node: &str) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria quita el OP de Minecraft."); }
        self.inner.init_mc_ops(server)?;
        let mut s = network::load_share(server);
        let gone: Vec<String> = s.mc_ops.iter().filter(|o| o.node_id == node).map(|o| o.mc_name.clone()).collect();
        if gone.is_empty() { return Ok(()); }
        s.mc_ops.retain(|o| o.node_id != node);
        for n in gone { if !s.mc_ops_revoked.iter().any(|x| x.eq_ignore_ascii_case(&n)) { s.mc_ops_revoked.push(n.clone()); } audit(server, "mc-op-revoke", &n); }
        if s.mc_ops_revoked.len() > 200 { let k = s.mc_ops_revoked.len() - 200; s.mc_ops_revoked.drain(0..k); }
        self.inner.commit(server, &mut s)
    }

    /// La persona de esta PC dio o quitó OP desde 939 Host (consola o Jugadores). Si esta PC es la
    /// propietaria de un servidor compartido, eso actualiza la lista firmada de OP autorizados
    /// (así la decisión vale en cualquier anfitriona). En cualquier otra PC no cambia nada: la
    /// anfitriona lo va a revertir con un aviso si no está autorizado.
    pub fn local_op_change(&self, server: &str, mc_name: &str, granted: bool) -> Result<()> {
        let s0 = network::load_share(server);
        if !s0.enabled || !self.inner.is_owner(server) || !host939_core::players::valid_name(mc_name) { return Ok(()); }
        self.inner.init_mc_ops(server)?;
        let mut s = network::load_share(server);
        let before = (s.mc_ops_base.clone(), s.mc_ops.clone(), s.mc_ops_revoked.clone());
        let eq = |x: &String| x.eq_ignore_ascii_case(mc_name);
        if granted {
            s.mc_ops_revoked.retain(|x| !eq(x));
            if !s.mc_ops_base.iter().any(eq) && !s.mc_ops.iter().any(|o| eq(&o.mc_name)) { s.mc_ops_base.push(mc_name.to_string()); }
        } else {
            s.mc_ops_base.retain(|x| !eq(x));
            if s.mc_ops.iter().any(|o| eq(&o.mc_name)) {
                s.mc_ops.retain(|o| !eq(&o.mc_name));
                if !s.mc_ops_revoked.iter().any(eq) { s.mc_ops_revoked.push(mc_name.to_string()); }
            }
        }
        if (s.mc_ops_base.clone(), s.mc_ops.clone(), s.mc_ops_revoked.clone()) == before { return Ok(()); }
        self.inner.commit(server, &mut s)?;
        audit(server, if granted { "mc-op-autorizado" } else { "mc-op-desautorizado" }, mc_name);
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

    /// «Permitir conexiones sin 939 Host»: solo la propietaria, firmado y enviado a todo el grupo.
    /// Encenderlo exige el modo online de Minecraft: sin él, cualquiera podría entrar con el nombre
    /// de un operador.
    pub fn set_external_access(&self, server: &str, allow: bool) -> Result<()> {
        if !self.inner.is_owner(server) { bail!("Solo la PC propietaria puede decidir quién entra sin 939 Host."); }
        if allow {
            if let Ok(c) = host939_core::instance::load(server) {
                if !c.online_mode { bail!("Primero activá «Verificar cuentas con Mojang» (modo online): sin eso, cualquiera podría entrar desde Internet con el nombre de un operador."); }
            }
        }
        let mut s = network::load_share(server);
        s.external_access = Some(allow);
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
        // Voz: la misma dirección escucha también el UDP de Simple Voice Chat cuando otra PC aloja.
        let vkey = format!("voice-watch:{server}");
        if let Some(t) = routes.remove(&vkey) { t.abort(); }
        let node = self.clone();
        let sid = server.to_string();
        routes.insert(vkey, tokio::spawn(async move { node.voice_watch(&sid, ip).await; }));
        Ok(())
    }

    /// Mantiene la escucha de voz de una dirección: abierta en el puerto de voz que informa la
    /// anfitriona (si es otra PC y tiene Simple Voice Chat), cerrada si no.
    async fn voice_watch(&self, server: &str, ip: std::net::Ipv4Addr) {
        let mut open: Option<(u16, tokio::task::JoinHandle<()>)> = None;
        loop {
            let want = lease::read(server).filter(|l| l.state == LeaseState::Hosting && l.host != self.inner.me).and_then(|l| {
                self.inner.with_state(server, |s| s.infos.get(&l.host).and_then(|i| i.voice_port))
            });
            match (&open, want) {
                (Some((p, _)), Some(w)) if *p == w => {}
                (_, Some(w)) => {
                    if let Some((_, t)) = open.take() { t.abort(); }
                    match self.open_voice(server, std::net::SocketAddr::from((ip, w))).await {
                        Ok(t) => open = Some((w, t)),
                        Err(e) => debug(&format!("voz: no se pudo escuchar en {ip}:{w}: {e}")),
                    }
                }
                (Some(_), None) => { if let Some((_, t)) = open.take() { t.abort(); } }
                (None, None) => {}
            }
            tokio::time::sleep(self.inner.timing.poll).await;
        }
    }

    /// Escucha UDP local y lleva cada cliente de voz a la anfitriona actual por su propia conexión
    /// cifrada (la anfitriona usa un socket por cliente: Simple Voice Chat los distingue así).
    pub async fn open_voice(&self, server: &str, bind: std::net::SocketAddr) -> Result<tokio::task::JoinHandle<()>> {
        let sock = Arc::new(tokio::net::UdpSocket::bind(bind).await.with_context(|| format!("no se pudo usar {bind} (UDP)"))?);
        let inner = self.inner.clone();
        let sid = server.to_string();
        Ok(tokio::spawn(async move {
            let mut peers: HashMap<std::net::SocketAddr, (Connection, Instant)> = HashMap::new();
            let mut buf = vec![0u8; 2048];
            loop {
                let Ok((n, src)) = sock.recv_from(&mut buf).await else { break };
                peers.retain(|_, (c, t)| c.close_reason().is_none() && t.elapsed() < Duration::from_secs(120));
                if !peers.contains_key(&src) {
                    let Some(host) = lease::read(&sid).filter(|l| l.state == LeaseState::Hosting).map(|l| l.host) else { continue };
                    let Ok(id) = host.parse::<EndpointId>() else { continue };
                    let addr = inner.peers.read().unwrap().get(&host).cloned().unwrap_or_else(|| EndpointAddr::new(id));
                    let Ok(Ok(c)) = tokio::time::timeout(Duration::from_secs(6), inner.ep.connect(addr, ALPN_VOICE)).await else { continue };
                    let Ok(mut hdr) = c.open_uni().await else { continue };
                    if wire::write_frame(&mut hdr, sid.as_bytes()).await.is_err() { continue; }
                    let _ = hdr.finish();
                    let (c2, s2) = (c.clone(), sock.clone());
                    tokio::spawn(async move { while let Ok(d) = c2.read_datagram().await { let _ = s2.send_to(&d, src).await; } });
                    peers.insert(src, (c, Instant::now()));
                }
                if let Some((c, t)) = peers.get_mut(&src) {
                    *t = Instant::now();
                    if n <= c.max_datagram_size().unwrap_or(0) { let _ = c.send_datagram(bytes::Bytes::copy_from_slice(&buf[..n])); }
                }
            }
        }))
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
                    // Si allá está apagado, se le pide el servidor: lo pasa solo, con el estado final.
                    let host_running = matches!(self.inner.rpc_timeout(&l.host, &Req::Info { server: server.into() }, Duration::from_secs(4)).await, Ok(Resp::Info(i)) if i.running);
                    if !host_running && self.can_request_check(server).is_ok() {
                        let name = self.request_host(server).await?;
                        bail!("PEDIDO:{} lo tenía apagado: te lo está pasando con el último estado del mundo. Se enciende solo en esta PC en unos segundos.", nice(&name));
                    }
                    bail!("Lo está hosteando {}. Pedile que te lo pase (botón «Pasar a…» en su 939 Host).", nice(&l.host_name));
                }
                bail!("Lo tenía {} y no responde. Si sabés que se apagó, usá «Recuperar aquí».", nice(&l.host_name));
            }
            LeaseState::Released => {
                // Antes de decidir, se consulta al grupo y se copia el último estado si alguien lo tiene.
                self.inner.tick_server(server).await;
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
    pub async fn handover(&self, server: &str, target: Option<&str>) -> Result<String> { self.inner.handover(server, target).await }

    /// Apaga y suelta el servidor dejando el último punto copiado en otra PC (si hay alguna).
    /// Devuelve cuántas PCs confirmaron tener el último estado.
    pub async fn release(&self, server: &str) -> Result<usize> {
        let inner = &self.inner;
        let l = lease::read(server).ok_or_else(|| anyhow!("Este servidor no está compartido."))?;
        if !l.held_by(&inner.me) { bail!("Esta PC no es la anfitriona."); }
        inner.set_busy(server, Some("Guardando y soltando el servidor".into()));
        inner.local.op(server, "Guardando y soltando el servidor", "Guardando el mundo y esperando que otra PC copie el último estado…", None);
        let r = async {
            if inner.local.is_running(server) { inner.blocking(server, |l, s| l.stop(s)).await?; }
            let snap = { let l2 = inner.local.clone(); let s2 = server.to_string(); tokio::task::spawn_blocking(move || l2.snapshot(&s2, "Al soltar el servidor", "auto")).await?.map_err(|e| anyhow!(e))? };
            let need = snap.freshness();
            let share = network::load_share(server);
            // Solo se espera a las PCs conectadas: esperar a una apagada solo demoraba el cierre
            // 90 s (lo copia cuando vuelva; el permiso soltado exige este punto para encender).
            let others: Vec<String> = share.members.iter().filter(|m| m.node_id != inner.me && m.can_host())
                .filter(|m| inner.recently_ok(server, &m.node_id, inner.timing.poll * 3)).map(|m| m.node_id.clone()).collect();
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
        inner.local.op_end(server, "Guardando y soltando el servidor", r.as_ref().err().map(|e: &anyhow::Error| e.to_string()).as_deref());
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
        self.friends_bye().await;
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
                avatar: if is_me { host939_core::avatar::own_hash() } else { info.and_then(|i| i.avatar.clone()).filter(|h| host939_core::avatar::has_peer(h)) },
                mc_op: share.mc_ops.iter().find(|o| o.node_id == m.node_id).map(|o| o.mc_name.clone()),
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
        let host_state = l.as_ref().filter(|l| l.state == LeaseState::Hosting).map(|l| {
            let (state, detail, players) = if l.host == inner.me { inner.state_of(server) } else if host_online {
                infos.get(&l.host).map(|i| (i.state.clone(), i.state_detail.clone(), i.players)).unwrap_or_default()
            } else { ("unreachable".into(), String::new(), 0) };
            HostState { state: if state.is_empty() { "unknown".into() } else { state }, detail, players, host_name: l.host_name.clone() }
        });
        GroupStatus {
            host_state,
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
    pub async fn answer_host_request(&self, server: &str, accept: bool) -> Result<String> { self.inner.answer_host_request(server, accept).await }
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
    serde_json::json!({ "name": cfg.name, "mcVersion": cfg.mc_version, "loader": format!("{:?}", cfg.loader).to_lowercase(), "loaderVersion": cfg.loader_version, "rev": rev, "logo": icon, "publicAddress": host939_core::alias::public::get(&cfg.id) })
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
    /// Pasa el servidor a otra PC del grupo (planificado). `target`: id del nodo, o el mejor disponible.
    async fn handover(self: &Arc<Self>, server: &str, target: Option<&str>) -> Result<String> {
        let inner = self;
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
                inner.local.op(server, &label_busy, "Copiando el mundo sin apagar el servidor…", None);
                inner.local.note(server, &format!("Preparando el traspaso a {}: copiando el mundo sin apagar…", info.name));
                let pre = take(format!("Antes del traspaso a {}", info.name)).await?;
                if let Err(e) = prefetch(pre.id.clone()).await {
                    bail!("{} no pudo copiar el mundo ({e}). El servidor sigue en esta PC.", info.name);
                }
                if !still_mine() { bail!("El servidor cambió de anfitriona mientras se preparaba el traspaso."); }
                inner.local.op(server, &label_busy, "Guardando el mundo y apagando…", None);
                inner.local.note(server, &format!("Pasando el servidor a {}: guardando y apagando…", info.name));
                inner.blocking(server, |l, s| l.stop(s)).await?;
            }
            // Fase 2: punto final y copia de lo que cambió (poco). Recién con todo copiado se
            // entrega el permiso: así nunca queda una PC con permiso y sin mundo.
            inner.local.op(server, &label_busy, "Copiando el último estado a la otra PC…", None);
            // Si el último cierre en esta PC no lo vio nadie (corte de luz, proceso terminado
            // junto con el guardián), lo que hay en disco no está confirmado como guardado: se
            // pasa igual (es lo más nuevo que existe), pero rotulado y avisando, y los puntos
            // anteriores siguen disponibles para volver.
            let unconfirmed = !was_running && host939_core::process::unconfirmed_close(server);
            if unconfirmed {
                inner.local.note(server, &format!("El último cierre de este servidor en esta PC no se completó (corte de luz o cierre forzado). Se pasa a {} lo que quedó en disco; si algo falta o falla, en Copias está el punto anterior.", info.name));
            }
            let label = if unconfirmed { format!("Traspaso a {} (tras un cierre sin confirmar)", info.name) } else { format!("Traspaso a {}", info.name) };
            let snap = take(label).await?;
            if unconfirmed { host939_core::process::take_unconfirmed(server); }
            if let Err(e) = prefetch(snap.id.clone()).await {
                if was_running && still_mine() { let _ = inner.blocking(server, |l, s| l.start(s)).await; }
                bail!("{} no pudo copiar el último estado ({e}). El servidor siguió en esta PC.", info.name);
            }
            if !still_mine() { bail!("El servidor cambió de anfitriona durante el traspaso; esta PC no lo vuelve a encender."); }
            inner.local.op(server, &label_busy, "Entregando el permiso de anfitriona…", None);
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
        inner.local.op_end(server, &label_busy, result.as_ref().err().map(|e| e.to_string()).as_deref());
        inner.local.changed(server);
        result
    }

    /// La anfitriona responde. Aceptar usa el traspaso planificado de siempre: si algo falla,
    /// el servidor sigue en esta PC con el mundo intacto.
    async fn answer_host_request(self: &Arc<Self>, server: &str, accept: bool) -> Result<String> {
        let inner = self;
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
        match inner.handover(server, Some(&r.node)).await {
            Ok(to) => Ok(to),
            Err(e) => {
                answer(false, format!("El traspaso no se pudo completar y el servidor siguió en {}: {e}", inner.name())).await;
                Err(e)
            }
        }
    }

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
                // Se toma la lista firmada completa (antes se copiaba campo por campo y un campo
                // nuevo que se olvidara dejaba la firma sin coincidir). Solo quedan los datos que
                // son de esta PC: si participa y sus invitaciones.
                let (enabled, invites) = (mine.enabled, std::mem::take(&mut mine.invites));
                mine = remote.clone();
                mine.enabled = enabled;
                mine.invites = invites;
                mine.owner_pin = signed::owner_of(remote);
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
            cert::Verdict::Stale => {
                // Dos PCs tomaron a la vez el mismo permiso soltado (misma época): cada una se
                // quedaba con el suyo y las dos encendían el mismo mundo. Gana siempre el de id menor
                // (la misma regla en todas las PCs) y la otra se detiene y aparta su copia.
                let Some(c) = cur.as_ref() else { return false };
                if !claim_conflict_wins(c, next) { return false; }
                let base = lease::history(server).into_iter().rev().find(|h| Some((h.epoch, h.host.clone(), h.state)) == next.prev);
                let ok = base.as_ref().map(|b| self.lease_verdict(server, &share, Some(b), next) == cert::Verdict::Accept).unwrap_or(false);
                if ok { audit(server, "claim-conflict", &next.host); }
                ok && lease::store(server, next).is_ok()
            }
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

    /// Dirección de una PC contada por otra (ver `host939_p2p::hearsay`).
    fn learn_hearsay(&self, share: &ShareConfig, addr: &EndpointAddr, may_replace: bool) {
        let key = addr.id.to_string();
        // La propietaria agrega a las PCs y es de confianza total: lo suyo vale aunque esta PC
        // todavía no haya adoptado la lista con la PC nueva. Una coanfitriona, solo integrantes.
        if key == self.me || (!may_replace && share.member(&key).is_none()) { return; }
        if !may_replace && self.peers.read().unwrap().contains_key(&key) { return; }
        self.learn(host939_p2p::hearsay(addr, self.relays));
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
        let stored = self.peers.read().unwrap().get(node).cloned().unwrap_or_else(|| EndpointAddr::new(id));
        // Lo guardado puede ser de antes de que la otra PC se reiniciara (otro puerto): se suman las
        // direcciones que el transporte ya conoce de esa PC (por ejemplo, por una conexión que ella
        // abrió hace un momento). En modo privado no hay transporte IP propio, así que esto nunca
        // abre una conexión directa.
        let info = self.ep.remote_info(id).await;
        let active: Vec<_> = info.as_ref().map(|i| i.addrs().filter(|x| matches!(x.usage(), iroh::endpoint::TransportAddrUsage::Active)).map(|x| x.addr().clone()).collect()).unwrap_or_default();
        let addr = if active.iter().any(|a| a.is_ip()) {
            // Hay un camino directo en uso ahora mismo: se usa ese (más los relays guardados), no
            // direcciones viejas que pueden ser de antes de un reinicio.
            EndpointAddr::from_parts(id, active.into_iter().chain(stored.addrs.iter().filter(|t| !t.is_ip()).cloned()))
        } else {
            let known: Vec<_> = info.map(|i| i.addrs().map(|x| x.addr().clone()).collect()).unwrap_or_default();
            EndpointAddr::from_parts(id, stored.addrs.iter().cloned().chain(known))
        };
        let c = tokio::time::timeout(Duration::from_secs(6), self.ep.connect(addr, alpn)).await.context("sin respuesta")??;
        self.conns.lock().await.insert(key, c.clone());
        // La conexión de control sirve en los dos sentidos: la otra PC también puede usarla para
        // consultarle a esta (ver `serve_ctrl`).
        if alpn == ALPN_CTRL { let _ = self.serve_tx.send(c.clone()); }
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
            share_rev: rev(&share).unwrap_or(0), owner: signed::owner_of(&share).unwrap_or_default(), addr: None, peers: vec![], pending_mods: vec![],
            avatar: host939_core::avatar::own_hash(),
            state: String::new(), state_detail: String::new(), players: 0,
            voice_port: if self.local.is_running(server) && lease::read(server).map(|l| l.held_by(&self.me)).unwrap_or(false) { host939_core::voice::port(server, self.local.mc_port(server)) } else { None },
            entrance: self.entrance_info(server),
        };
        let (state, state_detail, players) = self.state_of(server);
        let base = Info { state, state_detail, players, ..base };
        if !trusted { return base; }
        Info {
            latest: if copy { snapshots::latest(server) } else { None }, has_copy: copy,
            can_host: copy && share.member(&self.me).map(|m| m.can_host()).unwrap_or(false) && self.local.allow_hosting(server),
            score: self.local.score(), addr: Some(host939_p2p::shareable(&self.ep.addr(), self.relays)), peers,
            pending_mods: host939_core::modguard::pending(server), ..base
        }
    }

    /// Estado detallado del servidor en esta PC, incluido lo que hace el grupo (traspaso o recuperación).
    fn state_of(&self, server: &str) -> (String, String, u32) {
        if let Some(b) = self.with_state(server, |s| s.busy.clone()) {
            let st = if b.starts_with("Pasando") { "transferring" } else { "recovering" };
            return (st.into(), b, 0);
        }
        self.local.run_state(server)
    }

    /// Aplica la lista de OP de Minecraft autorizada por la propietaria (lista firmada). Lo hace
    /// la PC que aloja, con el servidor encendido, comparando con `ops.json` (así vale aunque
    /// cambie la anfitriona y aunque alguien edite el archivo o use `/op` dentro del juego):
    /// - a quien está autorizado y no tiene OP, se le da;
    /// - a quien tiene OP sin estar autorizado (una coanfitriona que se lo dio, un `/op` en el
    ///   juego), se le quita, siempre con un aviso que dice por qué y cómo autorizarlo.
    /// La primera vez, en la PC propietaria, la lista se fija con el `ops.json` que ya tenía: nunca
    /// se quitan OP que existían antes de esta función.
    fn apply_mc_ops(&self, server: &str, share: &ShareConfig) {
        if !self.local.is_running(server) { return; }
        if !share.mc_ops_init && self.is_owner(server) {
            let _ = self.init_mc_ops(server);
            return;
        }
        let ops_now = read_ops(server);
        let (give, take) = network::op_plan(&ops_now, share);
        for n in give {
            if self.local.command(server, &format!("op {n}")).is_ok() {
                self.local.note(server, &format!("OP de Minecraft para {n} (lo autorizó la propietaria)."));
            }
        }
        let grace = self.timing.poll * 3;
        let take: Vec<String> = self.with_state(server, |st| {
            st.op_unauthorized.retain(|k, _| take.iter().any(|n| n.eq_ignore_ascii_case(k)));
            take.into_iter().filter(|n| st.op_unauthorized.entry(n.to_ascii_lowercase()).or_insert_with(Instant::now).elapsed() >= grace).collect()
        });
        for n in take {
            self.with_state(server, |st| { st.op_unauthorized.remove(&n.to_ascii_lowercase()); });
            if self.local.command(server, &format!("deop {n}")).is_ok() {
                audit(server, "mc-op-quitado", &n);
                let why = if share.mc_ops_revoked.iter().any(|x| x.eq_ignore_ascii_case(&n)) { "lo decidió la propietaria".to_string() }
                    else { "no está en la lista de OP que autorizó la propietaria; ella puede concederlo desde Jugadores o la consola de 939 Host".to_string() };
                self.local.note(server, &format!("Se quitó el OP de Minecraft a {n}: {why}."));
            }
        }
    }

    /// Fija la lista de OP autorizados con el `ops.json` que tiene la propietaria (una sola vez).
    fn init_mc_ops(&self, server: &str) -> Result<()> {
        let mut s = network::load_share(server);
        if s.mc_ops_init || !s.enabled || !self.is_owner(server) { return Ok(()); }
        for n in read_ops(server) {
            if host939_core::players::valid_name(&n) && !s.mc_ops_base.iter().any(|x| x.eq_ignore_ascii_case(&n)) && !s.mc_ops.iter().any(|o| o.mc_name.eq_ignore_ascii_case(&n)) {
                s.mc_ops_base.push(n);
            }
        }
        s.mc_ops_init = true;
        self.commit(server, &mut s)?;
        audit(server, "mc-op-lista-fijada", &s.mc_ops_base.join(","));
        Ok(())
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
                    // Una época que esta PC no verificó no cuenta (el anuncio no tiene firma).
                    let f = match &info.latest { Some(x) if l.as_ref().map(|l| x.epoch <= l.epoch).unwrap_or(false) => x.freshness(), _ => (0, 0) };
                    c.push((f, info.score, node.clone()));
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
        tokio::task::spawn_blocking(move || snapshots::materialize(&s, &latest.id, &mc)).await?.map_err(|e| anyhow!(e.message))?;
        // Lo que quedó de una detención anterior ya no es el estado del servidor: se reemplazó.
        let _ = self.local.exited_unsaved(server);
        Ok(())
    }

    /// Esta PC pasó a ser la anfitriona: copia lo que falte, coloca el estado y enciende.
    async fn become_host(self: &Arc<Self>, server: &str, from: Option<String>, snap: Option<String>) {
        self.set_busy(server, Some("Recibiendo el servidor".into()));
        const T: &str = "Recibiendo el servidor";
        self.local.op(server, T, "Copiando el último estado del mundo…", None);
        let r = async {
            if let (Some(from), Some(snap)) = (&from, &snap) {
                let mut last = None;
                for _ in 0..4 {
                    match self.pull(server, from, snap).await { Ok(_) => { last = None; break; } Err(e) => { last = Some(e); tokio::time::sleep(Duration::from_secs(2)).await; } }
                }
                if let Some(e) = last { bail!("no se pudo copiar el último estado: {e}"); }
            }
            self.local.op(server, T, "Colocando y verificando el mundo…", None);
            self.place_latest(server).await?;
            self.local.op(server, T, "Encendiendo el servidor…", None);
            self.local.note(server, "Preparando el servidor en esta PC…");
            self.blocking(server, |l, s| l.prepare(s)).await?;
            self.blocking(server, |l, s| l.start(s)).await?;
            Ok(())
        }.await;
        self.set_busy(server, None);
        self.local.op_end(server, T, r.as_ref().err().map(|e| e.to_string()).as_deref());
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
        // Lo que dice el índice (época) se verifica contra el permiso de esta PC antes de bajar nada.
        let mi = snapshots::manifest_info(&man).map_err(|e| anyhow!(e.message))?;
        if mi.id != snap || !sync_source_ok(lease::read(server).as_ref(), &network::load_share(server), from, mi.epoch) {
            audit(server, "sync-source-rejected", from);
            bail!("el punto que ofrece esa PC no se puede verificar (época o PC no autorizadas)");
        }
        let missing = snapshots::missing_blobs(server, &man).map_err(|e| anyhow!(e.message))?;
        let total = missing.len();
        let started = Instant::now();
        let title = format!("Sincronizando «{}»", server_name(server));
        let mut got = 0usize;
        for chunk in missing.chunks(128) {
            if total > 256 { self.local.op(server, &title, &format!("{} de {} bloques del mundo", got, total), Some(got as f64 / total as f64)); }
            got += chunk.len();
            let (mut send, mut recv) = c.open_bi().await?;
            wire::write_msg(&mut send, &SyncReq::Blobs { server: server.into(), hashes: chunk.to_vec() }).await?;
            send.finish()?;
            for h in chunk {
                let data = wire::read_frame(&mut recv, snapshots::BLOCK).await?;
                if data.is_empty() { bail!("a la otra PC le falta un bloque"); }
                snapshots::put_blob(server, h, &data).map_err(|e| anyhow!(e.message))?;
            }
        }
        let info = snapshots::import_manifest(server, &man, from).map_err(|e| anyhow!(e.message))?;
        if total > 256 { self.local.op_end(server, &title, None); }
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
            // Foto de perfil nueva o cambiada: se pide una vez (se guarda por huella).
            if let Some(h) = info.avatar.clone().filter(|h| host939_core::avatar::valid_hash(h) && !host939_core::avatar::has_peer(h)) {
                if self.with_state(server, |s| s.avatar_fetch.insert(h.clone())) {
                    let (me, srv, n) = (self.clone(), server.to_string(), node.clone());
                    tokio::spawn(async move {
                        if let Ok(Resp::Avatar { png }) = me.rpc_timeout(&n, &Req::Avatar { server: srv.clone(), hash: h.clone() }, Duration::from_secs(8)).await {
                            if let Ok(b) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, png) {
                                if host939_core::avatar::store_peer(&h, &b).is_ok() { me.local.changed(&srv); }
                            }
                        }
                        me.with_state(&srv, |s| { s.avatar_fetch.remove(&h); });
                    });
                }
            }
            // Su propia dirección, solo si es de ella (no puede anunciar la de otra identidad).
            if let Some(a) = &info.addr { if a.id.to_string() == node { self.learn(a.clone()); } }
            // Direcciones de terceros: solo de quien puede hostear, para integrantes del grupo, y
            // sin reemplazar lo que esta PC ya sabe salvo que lo diga la propietaria.
            if share.member(&node).map(|m| m.can_host()).unwrap_or(false) {
                let from_owner = signed::pin(&share).as_deref() == Some(node.as_str());
                for p in &info.peers { self.learn_hearsay(&share, p, from_owner); }
            }
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
            // Un mod nuevo espera decisión (en esta PC o en otra del grupo): se avisa una vez.
            let waiting: Vec<String> = self.with_state(server, |st| {
                let mut v: Vec<String> = host939_core::modguard::pending(server).into_iter().map(|(_, h)| h).collect();
                for i in st.infos.values() { v.extend(i.pending_mods.iter().map(|(_, h)| h.clone())); }
                v.retain(|h| !s2.approved_mods.contains(h) && !s2.rejected_mods.contains(h));
                let fresh = v.iter().filter(|h| !st.mods_noticed.contains(*h)).count();
                st.mods_noticed.extend(v.iter().cloned());
                if fresh > 0 { v } else { vec![] }
            });
            if !waiting.is_empty() {
                self.local.note(server, &format!("{} mod(s) de otra PC esperan tu aprobación: revisalos en Mods → Solicitudes.", waiting.len()));
            }
            // Mods instalados por la propietaria en su propia PC: aprobados sin preguntar.
            for h in host939_core::modguard::local_jars(server) {
                if !s2.approved_mods.contains(&h) { host939_core::modguard::log_decision(server, &h, "propio"); s2.approved_mods.push(h); touched = true; }
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
            Some(l) if l.held_by(&self.me) => {
                if !self.local.is_running(server) && self.local.exited_unsaved(server) { self.publish_final(server).await; }
                self.retry_final(server).await;
                self.apply_mc_ops(server, &share);
                self.host_checks(server, &share).await;
                // La anfitriona tiene el servidor apagado y otra coanfitriona pidió alojarlo: no hay
                // nada encendido que proteger, así que se le pasa sin esperar una respuesta manual.
                if !self.local.is_running(server) {
                    // Lo hace el ciclo del nodo (`GroupNode::start`): el traspaso vuelve a llamar a
                    // `tick_server` y una llamada directa desde acá sería recursiva.
                    self.with_state(server, |s| if s.incoming.is_some() && s.busy.is_none() { s.auto_answer = true; });
                }
            }
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

    /// El servidor se detuvo en la anfitriona: el estado final queda en un punto y se avisa a las
    /// demás PCs para que lo copien ya (antes, «Detener» lo dejaba solo en esta PC hasta el
    /// próximo traspaso; si después esta PC se apagaba, el grupo seguía con un estado viejo).
    async fn publish_final(self: &Arc<Self>, server: &str) {
        let l2 = self.local.clone();
        let s2 = server.to_string();
        match tokio::task::spawn_blocking(move || l2.snapshot(&s2, "Al detener el servidor", "auto")).await {
            Ok(Ok(info)) => {
                let share = network::load_share(server);
                let targets: Vec<String> = share.members.iter().filter(|m| m.node_id != self.me && m.can_host()).map(|m| m.node_id.clone()).collect();
                let failed = self.notify_final(server, &targets).await;
                let msg = if failed.is_empty() {
                    format!("Estado final guardado (punto {}) y enviado a las demás PCs del grupo.", info.seq)
                } else {
                    format!("Estado final guardado (punto {}). Se enviará a las PCs que no respondieron en cuanto se conecten.", info.seq)
                };
                self.with_state(server, |st| st.final_pending.extend(failed));
                self.local.note(server, &msg);
                self.local.changed(server);
            }
            Ok(Err(e)) => self.local.note(server, &format!("No se pudo guardar el estado final para el grupo: {e}")),
            Err(_) => {}
        }
    }

    /// Avisa a esas PCs que copien el estado final ya. Devuelve las que no respondieron.
    async fn notify_final(self: &Arc<Self>, server: &str, nodes: &[String]) -> Vec<String> {
        let futs = nodes.iter().map(|node| {
            let req = Req::SyncNow { server: server.into() };
            async move { (node.clone(), self.rpc_timeout(node, &req, Duration::from_secs(4)).await.is_ok()) }
        });
        futures_util::future::join_all(futs).await.into_iter().filter(|(_, ok)| !ok).map(|(n, _)| n).collect()
    }

    /// Reintenta el aviso del estado final a las PCs que antes no respondieron y ahora sí.
    async fn retry_final(self: &Arc<Self>, server: &str) {
        let ready: Vec<String> = self.with_state(server, |st| {
            st.final_pending.iter().filter(|n| st.last_ok.get(*n).map(|t| t.elapsed() < self.timing.poll * 3).unwrap_or(false)).cloned().collect()
        });
        if ready.is_empty() { return; }
        let failed = self.notify_final(server, &ready).await;
        self.with_state(server, |st| { for n in &ready { if !failed.contains(n) { st.final_pending.remove(n); } } });
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
        let cur = lease::read(server);
        let share = network::load_share(server);
        let best = self.with_state(server, |s| {
            if s.syncing { return None; }
            s.infos.iter()
                .filter(|(n, _)| s.last_ok.get(*n).map(|t| t.elapsed() < window).unwrap_or(false))
                .filter(|(n, i)| usable_fresh(cur.as_ref(), &share, n, &i.latest) != (0, 0))
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
            Err(e) => {
                // La próxima vuelta abre una conexión nueva (la guardada puede ser de antes de que
                // la otra PC se reiniciara).
                self.conns.lock().await.remove(&(node.clone(), ALPN_SYNC));
                self.local.note(server, &format!("No se pudo sincronizar: {e}"));
            }
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
        serve_ctrl(self.0.clone(), conn).await;
        Ok(())
    }
}

/// Atiende los pedidos de control que llegan por una conexión, la haya abierto la otra PC o esta.
///
/// La primera vez que una PC del grupo pide algo por una conexión, esa conexión pasa a ser la que
/// esta PC usa para hablarle. Así, si la otra PC se reinició (otra dirección, otro puerto), esta le
/// contesta y le consulta por la conexión nueva que ella misma abrió, sin insistir con una
/// dirección vieja (lo detectó la prueba de cierre inesperado: B no podía volver a consultarle a A).
/// Solo se guardan conexiones de integrantes verificados: una PC desconocida no llena la caché.
async fn serve_ctrl(inner: Arc<Inner>, conn: Connection) {
    let remote = conn.remote_id().to_string();
    let mut learned = false;
    loop {
        let Ok((mut send, mut recv)) = conn.accept_bi().await else { return };
        let inner = inner.clone();
        let remote = remote.clone();
        let learn = !std::mem::replace(&mut learned, true);
        let rid = conn.remote_id();
        let conn2 = conn.clone();
        tokio::spawn(async move {
            let Ok(req) = wire::read_req::<_, Req>(&mut recv).await else { return };
            // Límite por PC: el uso normal es de unos pocos pedidos por segundo; una PC que manda
            // cientos (del grupo o no) no puede ocupar disco, CPU ni red de esta.
            if !inner.ctrl_limit.lock().unwrap().allow(&remote) {
                let _ = wire::write_msg(&mut send, &Resp::Err { msg: "Demasiados pedidos; esperá un momento.".into() }).await;
                let _ = send.finish();
                return;
            }
            if !host939_core::paths::valid_server_id(req.server()) {
                let _ = wire::write_msg(&mut send, &Resp::Err { msg: "Servidor inválido.".into() }).await;
                let _ = send.finish();
                return;
            }
            if learn && inner.is_member(req.server(), &remote) {
                inner.learn_from(rid).await;
                let key = (remote.clone(), ALPN_CTRL);
                let (old, others) = {
                    let c = inner.conns.lock().await;
                    (c.get(&key).cloned(), c.iter().any(|((n, _), x)| n == &remote && x.stable_id() != conn2.stable_id()))
                };
                match old {
                    Some(old) if old.stable_id() == conn2.stable_id() => {}
                    None if !others => { inner.conns.lock().await.insert(key, conn2.clone()); }
                    old => {
                        // Las conexiones nuevas usan desde ya la recién llegada (si la otra PC se
                        // reinició, es la única que anda); la guardada no se cierra hasta saber si
                        // sigue viva.
                        inner.conns.lock().await.insert(key, conn2.clone());
                        // Conexión nueva de una PC con la que esta ya tenía una guardada. Puede ser
                        // normal (las dos se conectaron a la vez, o la otra abrió una aparte) o que
                        // la otra se haya reiniciado y la guardada sea de antes. Se prueba la
                        // guardada con un pedido corto: si responde, no se cierra nada (cerrarla
                        // cortaría pedidos en curso y, si las dos PCs hicieran lo mismo a la vez, se
                        // cerrarían mutuamente las conexiones una y otra vez). Si no responde, es de
                        // antes: se cierran las viejas (no solo se olvidan) para que el transporte
                        // deje de usar el camino viejo, y se le pide a la otra una conexión nueva:
                        // con iroh 1.4, si la otra PC se reinició rápido, el transporte puede quedar
                        // usando el camino viejo mientras la conexión vieja figura viva.
                        // Se resuelve antes de atender el pedido: si la otra PC se reinició, lo que
                        // pida (por ejemplo, que se le copie su estado final) tiene que encontrar
                        // ya cerradas las conexiones viejas, o se intentaría por ellas.
                        let (inner, remote, server, conn2) = (inner.clone(), remote.clone(), req.server().to_string(), conn2.clone());
                        {
                            // Sin conexión de control guardada pero con otras (sincronización,
                            // archivos): la de control ya se descartó por fallar, así que las demás
                            // también se consideran de antes.
                            // Se manda un pedido corto por la guardada y se mira si llega algo de vuelta a
                            // nivel de transporte (aunque sea solo el acuse de recibo): una conexión viva
                            // contesta en lo que tarda la red aunque la otra PC esté muy ocupada; una de
                            // antes de un reinicio no recibe nada (en Windows tampoco avisa que murió).
                            // Mientras la vieja sigue abierta, el transporte puede seguir usando el camino
                            // viejo para esa PC, por eso no se espera más de 1 s. Equivocarse con una
                            // conexión lentísima solo hace lo de antes (cerrar y pedir una nueva).
                            let alive = match &old {
                                None => false,
                                Some(old) => {
                                    let rx0 = old.stats().udp_rx.datagrams;
                                    let probe = Req::Info { server: server.clone() };
                                    let old2 = old.clone();
                                    let ask = tokio::spawn(async move {
                                        let (mut s, mut r) = old2.open_bi().await?;
                                        wire::write_msg(&mut s, &probe).await?;
                                        s.finish()?;
                                        wire::read_msg::<_, Resp>(&mut r).await
                                    });
                                    let end = Instant::now() + PROBE_WAIT;
                                    let mut alive = false;
                                    while Instant::now() < end {
                                        if old.stats().udp_rx.datagrams > rx0 { alive = old.close_reason().is_none(); break; }
                                        if old.close_reason().is_some() { break; }
                                        tokio::time::sleep(Duration::from_millis(25)).await;
                                    }
                                    ask.abort();
                                    alive
                                }
                            };
                            debug(&format!("conexión nueva de {}: la guardada {}", &remote[..4.min(remote.len())], if alive { "responde, no se cierra" } else { "no responde, se cierran las viejas" }));
                            if !alive {
                                let mut conns = inner.conns.lock().await;
                                // La guardada ya no está en la lista (se reemplazó arriba): se suma aparte.
                                let stale: Vec<Connection> = conns.iter().filter(|((n, _), c)| n == &remote && c.stable_id() != conn2.stable_id()).map(|(_, c)| c.clone()).chain(old).collect();
                                conns.retain(|(n, _), _| n != &remote);
                                conns.insert((remote.clone(), ALPN_CTRL), conn2);
                                drop(conns);
                                for c in stale { c.close(0u32.into(), b"reemplazada"); }
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(300)).await;
                                    let _ = inner.rpc_timeout(&remote, &Req::Nudge { server }, Duration::from_secs(8)).await;
                                });
                            }
                        }
                    }
                }
            }
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

async fn handle(inner: &Arc<Inner>, remote: &str, req: Req) -> Resp {
    match req {
        Req::Info { server } => {
            // Solo pruebas: simula una PC que responde lento (por ejemplo, ocupada apagando).
            #[cfg(feature = "pruebas")]
            if let Some(ms) = std::env::var("H939_TEST_SLOW_INFO_MS").ok().and_then(|v| v.parse::<u64>().ok()) { tokio::time::sleep(Duration::from_millis(ms)).await; }
            #[allow(unused_mut)]
            let mut info = inner.my_info(&server, remote);
            // Solo pruebas: un nodo malicioso anuncia direcciones falsas (la suya y la de otras PCs).
            #[cfg(feature = "pruebas")]
            if let Some(v) = std::env::var("H939_TEST_EVIL_ADDRS").ok().and_then(|j| serde_json::from_str::<Vec<EndpointAddr>>(&j).ok()) { info.addr = v.first().cloned(); info.peers = v; }
            Resp::Info(info)
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
                let host_seen = s.last_ok.get(&l.host).copied().unwrap_or(inner.started);
                let host_dead = host_seen.elapsed() >= inner.timing.host_dead.mul_f32(0.8);
                let can_host = network::load_share(&server).member(&candidate).map(|m| m.can_host()).unwrap_or(false);
                if !vote_conditions(l, &inner.me, epoch, &candidate, remote, cand, mine, host_dead, can_host) { return false; }
                match &s.voted {
                    Some((e, c)) if *e == epoch && *c != candidate => false,
                    _ => { s.voted = Some((epoch, candidate.clone())); true }
                }
            });
            let sig = agree.then(|| cert::sign_vote(&inner.key, &server, epoch, &candidate));
            Resp::Vote { agree, sig }
        }
        Req::SyncNow { server } => {
            if !inner.sync_now_limit.lock().unwrap().allow(remote) { return Resp::Ok; }
            let inner2 = inner.clone();
            tokio::spawn(async move { inner2.tick_server(&server).await; });
            Resp::Ok
        }
        Req::Avatar { server, hash } => {
            if !inner.is_member(&server, remote) { return Resp::Err { msg: "No es integrante del grupo.".into() }; }
            match host939_core::avatar::own_png().filter(|_| host939_core::avatar::own_hash().as_deref() == Some(hash.as_str())) {
                Some(b) => Resp::Avatar { png: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b) },
                None => Resp::Err { msg: "Esa foto ya no está.".into() },
            }
        }
        Req::ProbeEntrance { addr, .. } => inner.probe_for(remote, &addr).await,
        Req::Nudge { .. } => {
            // Una conexión nueva hacia esa PC, que se cierra enseguida (solo sirve para que su
            // transporte vuelva a elegir el camino). Sin caché: tiene que ser una conexión nueva.
            let (inner, remote) = (inner.clone(), remote.to_string());
            tokio::spawn(async move {
                let Ok(id) = remote.parse::<EndpointId>() else { return };
                let stored = inner.peers.read().unwrap().get(&remote).cloned().unwrap_or_else(|| EndpointAddr::new(id));
                if let Ok(Ok(c)) = tokio::time::timeout(Duration::from_secs(6), inner.ep.connect(stored, ALPN_CTRL)).await {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    c.close(0u32.into(), b"listo");
                }
            });
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
                // Lo guardado puede ser de antes de que esa PC terminara de copiar la configuración:
                // se le pregunta de nuevo antes de rechazar.
                let fresh_ok = matches!(inner.rpc_timeout(remote, &Req::Info { server: server.clone() }, Duration::from_secs(4)).await, Ok(Resp::Info(i)) if i.can_host);
                if !fresh_ok { return Resp::Err { msg: "Esa PC no tiene copia del servidor o tiene desactivado hostear.".into() }; }
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
                    if inner.local.is_running(&server) {
                        if fresh_req {
                            inner.local.note(&server, &format!("{name} quiere alojar este servidor."));
                            inner.local.host_request(&server, &name);
                        }
                    } else {
                        // Apagado acá: el traspaso sale solo en la próxima vuelta (ver `tick_server`).
                        let (inner2, srv) = (inner.clone(), server.clone());
                        tokio::spawn(async move { inner2.tick_server(&srv).await; });
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
/// Pedidos de control por minuto y por PC. Con la consulta normal (cada 5 s, o 0,7 s en las
/// pruebas) una PC manda menos de 100 por minuto por servidor.
const CTRL_PER_MIN: usize = 600;

/// Cuánto se espera la respuesta de una conexión guardada antes de darla por vieja (ver `serve_ctrl`).
const PROBE_WAIT: Duration = Duration::from_millis(1000);

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
                let Ok(req) = wire::read_req::<_, SyncReq>(&mut recv).await else { return };
                if !inner.data_limit.lock().unwrap().allow(&remote) { let _ = send.reset(3u32.into()); return; }
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

/// Voz en la anfitriona: un socket UDP por cliente hacia el puerto de Simple Voice Chat local.
#[derive(Clone)]
struct VoiceH(Arc<Inner>);
impl std::fmt::Debug for VoiceH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "voice") } }
impl ProtocolHandler for VoiceH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        let inner = self.0.clone();
        let Ok(mut hdr) = conn.accept_uni().await else { return Ok(()) };
        let Ok(sid) = wire::read_frame(&mut hdr, 256).await else { return Ok(()) };
        let server = String::from_utf8_lossy(&sid).to_string();
        // Solo integrantes del grupo, y solo si esta PC aloja el servidor y tiene voz.
        if !inner.is_member(&server, &remote) || !lease::read(&server).map(|l| l.held_by(&inner.me)).unwrap_or(false) { conn.close(1u32.into(), b"no"); return Ok(()); }
        let Some(port) = host939_core::voice::port(&server, inner.local.mc_port(&server)).filter(|_| inner.local.is_running(&server)) else { conn.close(2u32.into(), b"sin voz"); return Ok(()) };
        // Un socket por cliente de voz, con tope: hasta 4 por PC y 64 en total.
        let slot = {
            let mut v = inner.voice_slots.lock().unwrap();
            let mine = v.get(&remote).copied().unwrap_or(0);
            let total: usize = v.values().sum();
            if mine >= 4 || total >= 64 { false } else { *v.entry(remote.clone()).or_default() += 1; true }
        };
        if !slot { conn.close(3u32.into(), b"demasiadas"); return Ok(()); }
        struct Release(Arc<Inner>, String);
        impl Drop for Release { fn drop(&mut self) { let mut v = self.0.voice_slots.lock().unwrap(); if let Some(n) = v.get_mut(&self.1) { *n = n.saturating_sub(1); if *n == 0 { v.remove(&self.1); } } } }
        let _release = Release(inner.clone(), remote.clone());
        let Ok(sock) = tokio::net::UdpSocket::bind(("127.0.0.1", 0)).await else { return Ok(()) };
        if sock.connect(("127.0.0.1", port)).await.is_err() { return Ok(()); }
        let mut buf = vec![0u8; 2048];
        loop {
            tokio::select! {
                d = conn.read_datagram() => { let Ok(d) = d else { break }; let _ = sock.send(&d).await; }
                r = sock.recv(&mut buf) => {
                    let Ok(n) = r else { break };
                    if n <= conn.max_datagram_size().unwrap_or(0) { let _ = conn.send_datagram(bytes::Bytes::copy_from_slice(&buf[..n])); }
                }
                _ = tokio::time::sleep(Duration::from_secs(120)) => break,
            }
        }
        Ok(())
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
                let Ok(req) = wire::read_req::<_, BridgeReq>(&mut recv).await else { return };
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
        let reply = match wire::read_req::<_, JoinReq>(&mut recv).await {
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
                        // Mínimo conocimiento: una jugadora solo necesita llegar a la propietaria (esta PC,
                        // ya la conoce) y a la anfitriona; las direcciones del resto no se le pasan.
                        let host_now = lease::read(&sid).map(|l| l.host).unwrap_or_default();
                        let peers = { let p = inner.peers.read().unwrap(); s.members.iter().filter(|m| m.node_id != remote && (role != "jugador" || m.node_id == host_now)).filter_map(|m| p.get(&m.node_id).map(|a| host939_p2p::shareable(a, inner.relays))).collect() };
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


#[cfg(test)]
mod sync_sources {
    //! Red team: integrantes que anuncian un punto «más nuevo que todos» para que el resto lo copie.
    use super::*;
    use host939_core::network::Member;

    fn member(id: &str, role: &str) -> Member { Member { node_id: id.into(), name: id.into(), joined_at: "t".into(), role: role.into() } }
    fn lease_at(epoch: u64, host: &str) -> Lease {
        Lease { epoch, host: host.into(), host_name: "h".into(), state: LeaseState::Hosting, since: "t".into(), required: (0, 0), kind: LeaseKind::Handover, issuer: host.into(), prev: None, votes: vec![], sig: None }
    }

    #[test]
    fn solo_se_copia_de_quien_puede_hostear_y_de_epocas_verificadas() {
        let share = ShareConfig { enabled: true, members: vec![member("dueña", "propietario"), member("coan", "coanfitrion"), member("jug", "jugador")], ..Default::default() };
        let cur = lease_at(5, "dueña");
        // Jugadora maliciosa: nunca es fuente, anuncie lo que anuncie.
        assert!(!sync_source_ok(Some(&cur), &share, "jug", 4));
        assert!(!sync_source_ok(Some(&cur), &share, "jug", 5));
        // Alguien de afuera del grupo.
        assert!(!sync_source_ok(Some(&cur), &share, "extraño", 1));
        // Época que esta PC no verificó (el anuncio «época 999» no tiene firma).
        assert!(!sync_source_ok(Some(&cur), &share, "dueña", 6));
        assert!(!sync_source_ok(Some(&cur), &share, "coan", 999));
        // Época actual: solo de la anfitriona actual.
        assert!(sync_source_ok(Some(&cur), &share, "dueña", 5));
        assert!(!sync_source_ok(Some(&cur), &share, "coan", 5));
        // Épocas anteriores (la anfitriona se cayó y quedaron copias en coanfitrionas): sí.
        assert!(sync_source_ok(Some(&cur), &share, "coan", 4));
        // Sin permiso verificado no se copia nada.
        assert!(!sync_source_ok(None, &share, "dueña", 1));
        // La frescura de un anuncio no utilizable no compite (no gana el candidato a anfitriona).
        let fake: Option<SnapshotInfo> = serde_json::from_value(serde_json::json!({ "id": "x", "createdAt": "", "label": "", "kind": "auto", "mcVersion": "", "fileCount": 0, "totalBytes": 0, "newBytes": 0, "pauseMs": 0, "epoch": 999u64, "seq": u64::MAX })).ok();
        assert!(fake.is_some());
        assert_eq!(usable_fresh(Some(&cur), &share, "coan", &fake), (0, 0));
    }

    /// Red team (vuelta 3): una candidata anuncia una copia «de la época 999» (el anuncio no tiene
    /// firma) para ganarle a las demás en la votación. Solo cuenta lo que esta PC verificó.
    /// Dos PCs toman a la vez el mismo permiso soltado: todas eligen la misma ganadora.
    #[test]
    fn desempate_de_tomas_simultaneas() {
        let base = |host: &str| Lease { epoch: 5, host: host.into(), host_name: host.into(), state: LeaseState::Hosting, since: String::new(), required: (0, 0), kind: LeaseKind::Claim, issuer: host.into(), prev: Some((4, "vieja".into(), LeaseState::Released)), votes: vec![], sig: None };
        let (a, b) = (base("aaaa"), base("bbbb"));
        assert!(claim_conflict_wins(&b, &a), "gana la de id menor");
        assert!(!claim_conflict_wins(&a, &b), "y nunca al revés");
        let mut other_prev = base("aaaa");
        other_prev.prev = Some((3, "x".into(), LeaseState::Released));
        assert!(!claim_conflict_wins(&b, &other_prev), "solo entre tomas del mismo permiso soltado");
        let mut handover = base("aaaa");
        handover.kind = LeaseKind::Handover;
        assert!(!claim_conflict_wins(&b, &handover), "un traspaso no entra en el desempate");
    }

    #[test]
    fn voto_rechaza_epocas_no_verificadas_y_suplantaciones() {
        let l = lease_at(5, "anfitriona");
        let ok = |cand: (u64, u64), remote: &str, dead: bool, can: bool| vote_conditions(&l, "yo", 5, "cand", remote, cand, (5, 3), dead, can);
        assert!(ok((5, 3), "cand", true, true), "control: voto legítimo");
        assert!(!ok((999, 0), "cand", true, true), "época no verificada");
        assert!(!ok((6, 0), "cand", true, true), "época siguiente sin permiso");
        assert!(!ok((5, 2), "cand", true, true), "copia más vieja que la mía");
        assert!(!ok((5, 3), "otra", true, true), "pide el voto por otra PC");
        assert!(!ok((5, 3), "cand", false, true), "la anfitriona todavía responde");
        assert!(!ok((5, 3), "cand", true, false), "una jugadora no puede ser candidata");
        assert!(!vote_conditions(&l, "yo", 4, "cand", "cand", (5, 3), (5, 3), true, true), "época de la votación vieja");
        assert!(!vote_conditions(&l, "anfitriona", 5, "cand", "cand", (5, 3), (5, 3), true, true), "la anfitriona no vota contra sí misma");
    }
}
