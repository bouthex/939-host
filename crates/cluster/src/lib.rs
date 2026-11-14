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

pub mod moving;
pub mod wire;

use anyhow::{anyhow, bail, Context, Result};
use host939_core::lease::{self, Lease, LeaseState};
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
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            poll: Duration::from_secs(5),
            host_dead: Duration::from_secs(35),
            fence: Duration::from_secs(20),
            seen_window: Duration::from_secs(180),
            release_wait: Duration::from_secs(90),
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
}

impl Req {
    fn server(&self) -> &str {
        match self {
            Req::Info { server } | Req::Config { server } | Req::Lease { server, .. } | Req::Handover { server, .. }
            | Req::Vote { server, .. } | Req::SyncNow { server } | Req::Bye { server } => server,
        }
    }
}

/// Lo que una PC informa de sí misma sobre un servidor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub node: String,
    pub name: String,
    pub lease: Option<Lease>,
    pub latest: Option<SnapshotInfo>,
    pub has_copy: bool,
    pub running: bool,
    pub can_host: bool,
    pub score: u32,
    pub share_rev: u64,
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
    Vote { agree: bool },
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
}

struct Inner {
    ep: Endpoint,
    me: String,
    name: String,
    local: Arc<dyn LocalHost>,
    timing: Timing,
    started: Instant,
    peers: RwLock<HashMap<String, EndpointAddr>>,
    conns: tokio::sync::Mutex<HashMap<(String, &'static [u8]), Connection>>,
    state: Mutex<HashMap<String, SrvState>>,
    routes: tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    stopping: std::sync::atomic::AtomicBool,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberStatus {
    pub node_id: String,
    pub name: String,
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
        let alpns = vec![ALPN_CTRL.to_vec(), ALPN_SYNC.to_vec(), ALPN_MC.to_vec(), ALPN_BRIDGE.to_vec(), ALPN_JOIN.to_vec()];
        let ep = host939_p2p::endpoint(secret, relays, alpns).await?;
        let me = ep.id().to_string();
        lease::set_local_node(&me);
        let mut peers = HashMap::new();
        for (k, v) in network::load_peers() {
            if let Ok(a) = serde_json::from_value::<EndpointAddr>(v) { peers.insert(k, a); }
        }
        let inner = Arc::new(Inner {
            ep: ep.clone(), me, name: name.to_string(), local, timing, started: Instant::now(),
            peers: RwLock::new(peers), conns: Default::default(), state: Default::default(), routes: Default::default(),
            stopping: Default::default(),
        });
        let h = Handler(inner.clone());
        let router = Router::builder(ep)
            .accept(ALPN_CTRL, h.clone()).accept(ALPN_SYNC, SyncH(inner.clone())).accept(ALPN_MC, McH(inner.clone()))
            .accept(ALPN_BRIDGE, BridgeH(inner.clone())).accept(ALPN_JOIN, JoinH(inner.clone()))
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
            s.members.insert(0, Member { node_id: self.inner.me.clone(), name: self.inner.name.clone(), joined_at: chrono::Local::now().to_rfc3339(), role: "propietario".into() });
        }
        s.card = Some(serde_json::json!({ "name": cfg.name, "mcVersion": cfg.mc_version, "loader": format!("{:?}", cfg.loader).to_lowercase() }));
        bump_rev(&mut s);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        lease::ensure(server, &self.inner.name).map_err(|e| anyhow!(e.message))?;
        Ok(s)
    }

    /// Crea una invitación. `role`: "coanfitrion" (guarda copia y puede hostear) o "jugador".
    pub fn invite(&self, server: &str, role: &str, label: &str) -> Result<String> {
        let mut s = network::load_share(server);
        if !s.enabled { bail!("Primero compartí el servidor."); }
        let secret = network::random_secret();
        let role = if role == "coanfitrion" { "coanfitrion" } else { "jugador" };
        s.invites.push(Invite { secret: secret.clone(), label: label.chars().take(40).collect(), created_at: chrono::Local::now().to_rfc3339(), role: role.into() });
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))?;
        Ok(host939_p2p::make_ticket(&self.inner.ep.addr(), &secret))
    }

    pub fn revoke_invite(&self, server: &str, secret: &str) -> Result<()> {
        let mut s = network::load_share(server);
        s.invites.retain(|i| i.secret != secret);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))
    }

    /// Saca a una PC del grupo (no puede quitarse al anfitrión actual).
    pub fn remove_member(&self, server: &str, node: &str) -> Result<()> {
        if lease::read(server).map(|l| l.held_by(node)).unwrap_or(false) { bail!("Esa PC es la anfitriona ahora: primero pasá el servidor a otra."); }
        let mut s = network::load_share(server);
        s.members.retain(|m| m.node_id != node || m.role == "propietario");
        bump_rev(&mut s);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))
    }

    pub fn set_options(&self, server: &str, auto_failover: bool, handover_on_quit: bool) -> Result<()> {
        let mut s = network::load_share(server);
        s.auto_failover = auto_failover;
        s.handover_on_quit = handover_on_quit;
        bump_rev(&mut s);
        network::save_share(server, &s).map_err(|e| anyhow!(e.message))
    }

    /// Se une a un servidor con un código de invitación. Devuelve el id del servidor.
    pub async fn join(&self, ticket: &str) -> Result<String> {
        let (addr, secret) = host939_p2p::parse_ticket(ticket)?;
        let conn = tokio::time::timeout(Duration::from_secs(15), self.inner.ep.connect(addr.clone(), ALPN_JOIN)).await
            .context("tiempo de espera agotado")?.context("no se pudo contactar a la PC que invitó (¿está encendida con 939 Host abierto?)")?;
        let (mut send, mut recv) = conn.open_bi().await?;
        wire::write_msg(&mut send, &JoinReq { secret, name: self.inner.name.clone() }).await?;
        send.finish()?;
        let r: JoinResp = wire::read_msg(&mut recv).await?;
        if !r.ok { bail!(r.error.unwrap_or_else(|| "La invitación fue rechazada.".into())); }
        let server = r.server.ok_or_else(|| anyhow!("respuesta incompleta"))?;
        let mut share = r.share.ok_or_else(|| anyhow!("respuesta incompleta"))?;
        share.invites.clear();
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
        mine.members = share.members; mine.auto_failover = share.auto_failover; mine.handover_on_quit = share.handover_on_quit;
        mine.card = share.card; mine.enabled = true;
        network::save_share(server, &mine).map_err(|e| anyhow!(e.message))?;
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
            lease::ensure(server, &self.inner.name).map_err(|e| anyhow!(e.message))?;
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
                lease::claim(server, l.epoch, &self.inner.name).map_err(|e| anyhow!(e.message))?;
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
        inner.set_busy(server, Some(format!("Pasando el servidor a {}", info.name)));
        let result = async {
            let was_running = inner.local.is_running(server);
            if was_running {
                inner.local.note(server, &format!("Pasando el servidor a {}: guardando y apagando…", info.name));
                inner.blocking(server, |l, s| l.stop(s)).await?;
            }
            let label = format!("Traspaso a {}", info.name);
            let snap = { let l2 = inner.local.clone(); let s2 = server.to_string(); tokio::task::spawn_blocking(move || l2.snapshot(&s2, &label, "auto")).await?.map_err(|e| anyhow!(e))? };
            let next = lease::next_for(server, &target, &info.name);
            let sent = inner.rpc(&target, &Req::Handover { server: server.into(), lease: next.clone(), snapshot: Some(snap.id.clone()) }).await;
            let accepted = match sent {
                Ok(Resp::Ok) => true,
                Ok(Resp::Err { msg }) => { inner.local.note(server, &format!("{} rechazó el traspaso: {msg}", info.name)); false }
                _ => matches!(inner.rpc(&target, &Req::Info { server: server.into() }).await, Ok(Resp::Info(i)) if i.lease.as_ref() == Some(&next)),
            };
            if !accepted {
                if was_running { let _ = inner.blocking(server, |l, s| l.start(s)).await; }
                bail!("No se pudo completar el traspaso; el servidor siguió en esta PC.");
            }
            lease::adopt(server, &next).map_err(|e| anyhow!(e.message))?;
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
            lease::release(server, need).map_err(|e| anyhow!(e.message))?;
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
            if l.state == LeaseState::Hosting && l.host != inner.me && inner.recently_ok(server, &l.host, Duration::from_secs(10)) {
                bail!("{} está respondiendo: pedile que te lo pase.", nice(&l.host_name));
            }
        }
        inner.set_busy(server, Some("Recuperando el servidor en esta PC".into()));
        let r = async {
            lease::force_claim(server, &inner.name).map_err(|e| anyhow!(e.message))?;
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
                node_id: m.node_id.clone(), name: if is_me { inner.name.clone() } else { info.map(|i| i.name.clone()).unwrap_or_else(|| m.name.clone()) },
                role: m.role.clone(), is_me, online, running, is_host: l.as_ref().map(|l| l.held_by(&m.node_id)).unwrap_or(false),
                can_host, score, latest_at: latest.as_ref().map(|s| s.created_at.clone()), fresh: fresh(&latest), path: if is_me { String::new() } else { path }, rtt_ms,
            });
        }
        let i_am_host = l.as_ref().map(|l| l.held_by(&inner.me)).unwrap_or(false);
        let host_online = l.as_ref().map(|l| l.state == LeaseState::Hosting && (l.host == inner.me || inner.recently_ok(server, &l.host, online_window))).unwrap_or(false);
        let can_start = self.can_start_check(server, &l, &my_latest);
        GroupStatus {
            server: server.into(), me: inner.me.clone(), lease: l, i_am_host, host_online, my_latest, has_copy: has_copy(server), fenced, busy,
            can_start, auto_failover: share.auto_failover, handover_on_quit: share.handover_on_quit, majority: majority(share.members.len()), members,
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

fn debug(msg: &str) { if std::env::var("H939_DEBUG").is_ok() { eprintln!("[debug] {msg}"); } }

fn nice(name: &str) -> String { if name.trim().is_empty() { "otra PC del grupo".into() } else { name.to_string() } }

fn bump_rev(s: &mut ShareConfig) {
    let mut card = s.card.take().unwrap_or_else(|| serde_json::json!({}));
    let r = card.get("rev").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    card["rev"] = serde_json::json!(r);
    s.card = Some(card);
}
fn rev(s: &ShareConfig) -> Option<u64> { s.card.as_ref().and_then(|c| c.get("rev")).and_then(|v| v.as_u64()) }

/* ─────────────────────────────── Lógica interna ─────────────────────────────── */

impl Inner {
    fn learn(&self, addr: EndpointAddr) {
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

    fn my_info(&self, server: &str) -> Info {
        let share = network::load_share(server);
        let copy = has_copy(server);
        let peers = {
            let p = self.peers.read().unwrap();
            share.members.iter().filter_map(|m| p.get(&m.node_id).cloned()).collect()
        };
        Info {
            node: self.me.clone(), name: self.name.clone(), lease: lease::read(server),
            latest: if copy { snapshots::latest(server) } else { None }, has_copy: copy,
            running: self.local.is_running(server),
            can_host: copy && share.member(&self.me).map(|m| m.can_host()).unwrap_or(false) && self.local.allow_hosting(server),
            score: self.local.score(), share_rev: rev(&share).unwrap_or(0), addr: Some(self.ep.addr()), peers,
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

    async fn after_lease_change(self: &Arc<Self>, server: &str) {
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
        let others: Vec<Member> = share.members.iter().filter(|m| m.node_id != self.me).cloned().collect();
        let results = futures_util::future::join_all(others.iter().map(|m| {
            let req = Req::Info { server: server.into() };
            let node = m.node_id.clone();
            async move { (node.clone(), self.rpc_timeout(&node, &req, Duration::from_secs(4)).await) }
        })).await;
        let mut newest_share: Option<(u64, String)> = None;
        for (node, r) in results {
            let Ok(Resp::Info(info)) = r else { continue };
            if let Some(a) = &info.addr { self.learn(a.clone()); }
            for p in &info.peers { self.learn(p.clone()); }
            if let Some(remote) = &info.lease {
                if lease::adopt(server, remote).unwrap_or(false) { self.after_lease_change(server).await; }
            }
            if info.share_rev > rev(&share).unwrap_or(0) && newest_share.as_ref().map(|(r, _)| info.share_rev > *r).unwrap_or(true) {
                newest_share = Some((info.share_rev, node.clone()));
            }
            self.with_state(server, |s| { s.last_ok.insert(node.clone(), Instant::now()); s.left.remove(&node); s.infos.insert(node, info); });
        }
        if let Some((_, from)) = newest_share {
            if let Ok(Resp::Config { share: remote, .. }) = self.rpc(&from, &Req::Config { server: server.into() }).await {
                let mut mine = network::load_share(server);
                mine.members = remote.members; mine.auto_failover = remote.auto_failover; mine.handover_on_quit = remote.handover_on_quit; mine.card = remote.card;
                let _ = network::save_share(server, &mine);
                self.local.changed(server);
            }
        }

        let l = lease::read(server);
        let copy = has_copy(server);
        let busy = self.with_state(server, |s| s.busy.is_some());
        if std::env::var("H939_DEBUG").is_ok() {
            let ages: Vec<String> = self.with_state(server, |s| s.last_ok.iter().map(|(k, t)| format!("{}:{:.1}", &k[..4], t.elapsed().as_secs_f32())).collect());
            debug(&format!("{} tick lease={:?} busy={busy} ages={ages:?}", self.name, l.as_ref().map(|l| (l.epoch, &l.host[..4], l.state))));
        }
        if busy { return; }
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
        debug(&format!("{}: anfitrión sin respuesta hace {:?}; mejor candidata {:?}", self.name, since.elapsed(), best.as_ref().map(|b| &b[..8])));
        let Some(best) = best else { return };
        if best != self.me { return; }
        let my_fresh = fresh(&snapshots::latest(server));
        let need = majority(share.members.len());
        let mut votes = 1;
        let mut voters = vec![self.name.clone()];
        for m in share.members.iter().filter(|m| m.node_id != self.me && m.node_id != l.host) {
            let req = Req::Vote { server: server.into(), epoch: l.epoch, candidate: self.me.clone(), fresh: my_fresh };
            if let Ok(Resp::Vote { agree: true }) = self.rpc_timeout(&m.node_id, &req, Duration::from_secs(4)).await {
                votes += 1;
                voters.push(m.name.clone());
            }
        }
        debug(&format!("{}: votos {votes}/{need} ({})", self.name, voters.join(", ")));
        if votes < need { return; }
        // Puede que otra PC haya avanzado mientras tanto: `claim` falla si la época cambió.
        if lease::claim(server, l.epoch, &self.name).is_err() { return; }
        self.local.note(server, &format!("{} no responde. Elegida esta PC por mayoría ({}): levantando el servidor acá.", nice(&l.host_name), voters.join(", ")));
        self.broadcast_lease(server).await;
        self.become_host(server, None, None).await;
    }

    /// Lleva una conexión de un jugador a la PC anfitriona.
    async fn route_one(self: &Arc<Self>, server: &str, tcp: TcpStream) -> Result<()> {
        let l = lease::read(server);
        let name = server_name(server);
        let busy = self.with_state(server, |s| s.busy.clone());
        if let Some(b) = busy {
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

    fn is_member(&self, server: &str, node: &str) -> bool {
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
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let Ok(req) = wire::read_msg::<_, Req>(&mut recv).await else { return };
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
        Req::Info { server } => Resp::Info(inner.my_info(&server)),
        Req::Config { server } => {
            let mut share = network::load_share(&server);
            share.invites.clear();
            let config = host939_core::instance::load(&server).ok().and_then(|c| serde_json::to_value(c).ok());
            Resp::Config { config, share }
        }
        Req::Lease { server, lease: l } => {
            match lease::adopt(&server, &l) {
                Ok(true) => { inner.after_lease_change(&server).await; Resp::Ok }
                Ok(false) => Resp::Ok,
                Err(e) => Resp::Err { msg: e.message },
            }
        }
        Req::Handover { server, lease: l, snapshot } => {
            if l.host != inner.me { return Resp::Err { msg: "El permiso no es para esta PC.".into() }; }
            let cur = lease::read(&server);
            if cur.as_ref().map(|c| !c.held_by(remote) || l.epoch <= c.epoch).unwrap_or(false) {
                return Resp::Err { msg: "Quien pasa el servidor no es la anfitriona actual según esta PC.".into() };
            }
            if !has_copy(&server) || !inner.local.allow_hosting(&server) { return Resp::Err { msg: "Esta PC no puede hostear este servidor.".into() }; }
            if let Err(e) = lease::adopt(&server, &l) { return Resp::Err { msg: e.message }; }
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
                match &s.voted {
                    Some((e, c)) if *e == epoch && *c != candidate => false,
                    _ => { s.voted = Some((epoch, candidate.clone())); true }
                }
            });
            Resp::Vote { agree }
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
    }
}

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
                        let bytes = if inner.is_member(&server, &remote) { snapshots::manifest_bytes(&server, &snap).unwrap_or_default() } else { vec![] };
                        let _ = wire::write_frame(&mut send, &bytes).await;
                    }
                    SyncReq::Blobs { server, hashes } => {
                        let ok = inner.is_member(&server, &remote);
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
                let found = network::shared_servers().into_iter().find_map(|sid| {
                    let s = network::load_share(&sid);
                    s.invites.iter().find(|i| i.secret == req.secret).cloned().map(|i| (sid, i))
                });
                match found {
                    Some((sid, inv)) => {
                        let mut s = network::load_share(&sid);
                        let name: String = req.name.trim().chars().take(32).collect();
                        let role = if inv.role == "coanfitrion" && has_copy(&sid) { "coanfitrion" } else { "jugador" };
                        s.members.retain(|m| m.node_id != remote);
                        s.members.push(Member { node_id: remote.clone(), name: if name.is_empty() { "Amigo".into() } else { name.clone() }, joined_at: chrono::Local::now().to_rfc3339(), role: role.into() });
                        // Cada invitación sirve para una sola PC.
                        s.invites.retain(|i| i.secret != req.secret);
                        bump_rev(&mut s);
                        let _ = network::save_share(&sid, &s);
                        inner.local.note(&sid, &format!("{} se unió al grupo como {}.", if name.is_empty() { "Un amigo" } else { &name }, role));
                        inner.local.changed(&sid);
                        let mut out = s.clone();
                        out.invites.clear();
                        let peers = { let p = inner.peers.read().unwrap(); s.members.iter().filter_map(|m| p.get(&m.node_id).cloned()).collect() };
                        JoinResp { ok: true, error: None, server: Some(sid), share: Some(out), role: Some(role.into()), peers }
                    }
                    None => JoinResp { ok: false, error: Some("La invitación no es válida, ya se usó o fue revocada.".into()), server: None, share: None, role: None, peers: vec![] },
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

