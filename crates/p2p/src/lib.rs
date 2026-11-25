//! Túnel P2P de 939 Host.
//!
//! El cliente de Minecraft se conecta por TCP a `127.0.0.1:<puerto local fijo>`; 939 Host lleva
//! esa conexión por QUIC cifrado (iroh) hasta el anfitrión, que la entrega a su servidor local.
//! El anfitrión solo acepta conexiones de los nodos autorizados del grupo.

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use tokio::net::{TcpListener, TcpStream};

/// Protocolo del túnel de Minecraft (versión 1).
pub const ALPN_MC: &[u8] = b"939host/mc/1";
/// Protocolo de invitación: el amigo presenta el secreto del código y queda autorizado.
pub const ALPN_JOIN: &[u8] = b"939host/join/1";

/// Cómo se conecta esta PC con las demás.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relays {
    /// Solo conexiones directas (red local, IPv6, puerto abierto). Sin servicios de terceros.
    Disabled,
    /// Directo cuando se puede (hole punching) y relays públicos de iroh como último recurso.
    PublicFallback,
    /// Modo privado: SOLO relays. Esta PC no abre ningún socket UDP propio hacia las demás, así
    /// que ninguna otra PC del grupo puede ver su IP pública ni local (no hay transporte IP que
    /// anunciar ni por el que conectarse). Si el relay no responde, no hay conexión: nunca se
    /// cambia a directo. El relay sí ve la IP de esta PC, cuándo se conecta y cuánto tráfico pasa,
    /// pero no el contenido (QUIC/TLS 1.3 de punta a punta entre las dos PCs).
    Private,
}

impl Relays {
    pub fn uses_relays(self) -> bool { self != Relays::Disabled }
}

/// Crea el endpoint de este nodo con su identidad permanente.
///
/// UPnP/PCP/NAT-PMP queda desactivado en todos los modos: 939 Host no abre puertos en el router
/// por su cuenta.
pub async fn endpoint(secret: [u8; 32], relays: Relays, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let builder = match relays {
        Relays::Disabled => Endpoint::builder(iroh::endpoint::presets::Minimal).relay_mode(RelayMode::Disabled),
        Relays::PublicFallback => Endpoint::builder(iroh::endpoint::presets::N0),
        Relays::Private => Endpoint::builder(iroh::endpoint::presets::N0).clear_ip_transports(),
    };
    let ep = builder
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .secret_key(SecretKey::from_bytes(&secret))
        .alpns(alpns)
        .bind()
        .await
        .context("no se pudo abrir el endpoint P2P")?;
    Ok(ep)
}

/// La misma dirección sin IPs (solo identidad y relay): lo que se comparte cuando hay relays.
pub fn without_ips(a: &EndpointAddr) -> EndpointAddr {
    EndpointAddr::from_parts(a.id, a.addrs.iter().filter(|t| !t.is_ip()).cloned())
}

/// ¿Es uno de los relays públicos de n0 (los únicos que configura 939 Host)?
pub fn is_n0_relay(url: &iroh::RelayUrl) -> bool {
    url.host_str().map(|h| h.ends_with(".relay.n0.iroh.link") || h == "relay.n0.iroh.link").unwrap_or(false)
}

/// Dirección de una PC contada por OTRA PC (no por ella misma): con relays sin IPs, y solo con
/// relays de n0. Así un integrante no puede hacer que el resto intente llegar a otra PC a través
/// de un relay propio (vería sus IPs y podría cortarles la conexión).
pub fn hearsay(a: &EndpointAddr, relays: Relays) -> EndpointAddr {
    let a = shareable(a, relays);
    EndpointAddr::from_parts(a.id, a.addrs.iter().filter(|t| match t { iroh::TransportAddr::Relay(u) => is_n0_relay(u), _ => true }).cloned())
}

/// Lo que esta PC publica de sí misma según su modo: con relays, nunca sus IPs (las demás la
/// encuentran por su identidad y el relay); sin relays, las IPs son la única forma de llegar.
pub fn shareable(a: &EndpointAddr, relays: Relays) -> EndpointAddr {
    if relays.uses_relays() { without_ips(a) } else { a.clone() }
}

/// Lista de nodos autorizados (miembros del grupo). Compartida y actualizable en vivo.
#[derive(Clone, Default)]
pub struct Allowlist(Arc<RwLock<HashSet<EndpointId>>>);

impl Allowlist {
    pub fn allow(&self, id: EndpointId) { self.0.write().unwrap().insert(id); }
    pub fn revoke(&self, id: &EndpointId) { self.0.write().unwrap().remove(id); }
    pub fn contains(&self, id: &EndpointId) -> bool { self.0.read().unwrap().contains(id) }
}

/// Lado anfitrión: cada stream entrante de un miembro autorizado se conecta al servidor local.
#[derive(Clone, Debug)]
struct McHost { target: SocketAddr, allow: Allowlist }

impl std::fmt::Debug for Allowlist {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "Allowlist({})", self.0.read().unwrap().len()) }
}

impl ProtocolHandler for McHost {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        if !self.allow.contains(&remote) {
            conn.close(1u32.into(), b"no autorizado");
            return Ok(());
        }
        loop {
            let (mut send, mut recv) = match conn.accept_bi().await {
                Ok(s) => s,
                Err(_) => return Ok(()), // conexión cerrada
            };
            let target = self.target;
            tokio::spawn(async move {
                let Ok(tcp) = TcpStream::connect(target).await else { let _ = send.reset(2u32.into()); return; };
                let _ = tcp.set_nodelay(true);
                let (mut r, mut w) = tcp.into_split();
                let up = async { let _ = tokio::io::copy(&mut recv, &mut w).await; };
                let down = async { let _ = tokio::io::copy(&mut r, &mut send).await; let _ = send.finish(); };
                tokio::join!(up, down);
            });
        }
    }
}

/* ───────── Invitaciones ───────── */

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Datos públicos del servidor que recibe quien se une.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerCard { pub server_id: String, pub name: String, pub mc_version: String, pub loader: String }

#[derive(Serialize, Deserialize)]
struct JoinRequest { secret: String, name: String }
#[derive(Serialize, Deserialize)]
struct JoinReply { ok: bool, error: Option<String>, server: Option<ServerCard> }

/// Invitaciones vigentes: secreto → servidor. La app las persiste y las carga.
#[derive(Clone, Default)]
pub struct Invites(Arc<RwLock<HashMap<String, ServerCard>>>);
impl Invites {
    pub fn add(&self, secret: String, card: ServerCard) { self.0.write().unwrap().insert(secret, card); }
    pub fn revoke(&self, secret: &str) { self.0.write().unwrap().remove(secret); }
    fn get(&self, secret: &str) -> Option<ServerCard> { self.0.read().unwrap().get(secret).cloned() }
}
impl std::fmt::Debug for Invites { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "Invites") } }

pub type OnJoin = Arc<dyn Fn(EndpointId, String, ServerCard) + Send + Sync>;

#[derive(Clone)]
struct JoinHost { invites: Invites, allow: Allowlist, on_join: OnJoin }
impl std::fmt::Debug for JoinHost { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "JoinHost") } }

impl ProtocolHandler for JoinHost {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id();
        let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
        let req: Option<JoinRequest> = async {
            let len = recv.read_u32().await.ok()? as usize;
            if len > 4096 { return None; }
            let mut buf = vec![0u8; len];
            recv.read_exact(&mut buf).await.ok()?;
            serde_json::from_slice(&buf).ok()
        }.await;
        let reply = match req.and_then(|r| self.invites.get(&r.secret).map(|c| (r.name, c))) {
            Some((name, card)) => {
                self.allow.allow(remote);
                (self.on_join)(remote, name.chars().take(32).collect(), card.clone());
                JoinReply { ok: true, error: None, server: Some(card) }
            }
            None => JoinReply { ok: false, error: Some("Código de invitación inválido o revocado.".into()), server: None },
        };
        let bytes = serde_json::to_vec(&reply).unwrap_or_default();
        let _ = send.write_u32(bytes.len() as u32).await;
        let _ = send.write_all(&bytes).await;
        let _ = send.finish();
        let _ = send.stopped().await;
        Ok(())
    }
}

/// Código de invitación: dirección del anfitrión (clave pública + caminos conocidos) y secreto.
#[derive(Serialize, Deserialize)]
struct TicketData { a: EndpointAddr, s: String }

pub fn make_ticket(addr: &EndpointAddr, secret: &str) -> String {
    let json = serde_json::to_vec(&TicketData { a: addr.clone(), s: secret.to_string() }).unwrap_or_default();
    format!("939-{}", data_encoding::BASE32_NOPAD.encode(&json).to_lowercase())
}

pub fn parse_ticket(ticket: &str) -> Result<(EndpointAddr, String)> {
    let t = ticket.trim();
    let body = t.strip_prefix("939-").context("el código no es de 939 Host")?;
    let raw = data_encoding::BASE32_NOPAD.decode(body.to_uppercase().as_bytes()).context("el código está incompleto o dañado")?;
    let d: TicketData = serde_json::from_slice(&raw).context("el código está incompleto o dañado")?;
    Ok((d.a, d.s))
}

/// Une este nodo a un servidor usando un código de invitación.
pub async fn join(secret_key: [u8; 32], ticket: &str, my_name: &str, relays: Relays) -> Result<(EndpointAddr, ServerCard)> {
    let (addr, secret) = parse_ticket(ticket)?;
    let ep = endpoint(secret_key, relays, vec![]).await?;
    let conn = ep.connect(addr.clone(), ALPN_JOIN).await.context("no se pudo contactar al anfitrión (¿está encendido y compartiendo?)")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let req = serde_json::to_vec(&JoinRequest { secret, name: my_name.to_string() })?;
    send.write_u32(req.len() as u32).await?;
    send.write_all(&req).await?;
    send.finish()?;
    let len = recv.read_u32().await? as usize;
    anyhow::ensure!(len <= 65536, "respuesta inválida");
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    let reply: JoinReply = serde_json::from_slice(&buf)?;
    ep.close().await;
    match (reply.ok, reply.server) {
        (true, Some(card)) => Ok((addr, card)),
        _ => anyhow::bail!(reply.error.unwrap_or_else(|| "El anfitrión rechazó la invitación.".into())),
    }
}

/// Túnel del anfitrión en marcha.
pub struct HostTunnel { router: Router, pub allow: Allowlist, pub invites: Invites }

impl HostTunnel {
    pub fn endpoint(&self) -> &Endpoint { self.router.endpoint() }
    pub fn addr(&self) -> EndpointAddr { self.router.endpoint().addr() }
    pub async fn shutdown(self) { let _ = self.router.shutdown().await; }
}

/// Publica el servidor local `mc_port` para los miembros autorizados.
pub async fn host(secret: [u8; 32], mc_port: u16, relays: Relays, allow: Allowlist) -> Result<HostTunnel> {
    host_with_invites(secret, mc_port, relays, allow, Invites::default(), Arc::new(|_, _, _| {})).await
}

/// Anfitrión con invitaciones: acepta el túnel de los autorizados y el protocolo de unión.
pub async fn host_with_invites(secret: [u8; 32], mc_port: u16, relays: Relays, allow: Allowlist, invites: Invites, on_join: OnJoin) -> Result<HostTunnel> {
    let ep = endpoint(secret, relays, vec![ALPN_MC.to_vec(), ALPN_JOIN.to_vec()]).await?;
    let handler = McHost { target: SocketAddr::from(([127, 0, 0, 1], mc_port)), allow: allow.clone() };
    let joiner = JoinHost { invites: invites.clone(), allow: allow.clone(), on_join };
    let router = Router::builder(ep).accept(ALPN_MC, handler).accept(ALPN_JOIN, joiner).spawn();
    Ok(HostTunnel { router, allow, invites })
}

/// Túnel del jugador: escucha en un puerto local fijo y lleva cada conexión al anfitrión.
pub struct ClientTunnel { pub local_port: u16, task: tokio::task::JoinHandle<()>, pub endpoint: Endpoint }

impl ClientTunnel {
    pub async fn shutdown(self) { self.task.abort(); self.endpoint.close().await; }
}

pub async fn client(secret: [u8; 32], host: EndpointAddr, local_port: u16, relays: Relays) -> Result<ClientTunnel> {
    let ep = endpoint(secret, relays, vec![]).await?;
    let listener = TcpListener::bind(("127.0.0.1", local_port)).await.with_context(|| format!("el puerto local {local_port} está ocupado"))?;
    let local_port = listener.local_addr()?.port();
    let conn = ep.connect(host.clone(), ALPN_MC).await.context("no se pudo conectar con el anfitrión")?;
    let ep2 = ep.clone();
    let task = tokio::spawn(async move {
        let mut conn = conn;
        loop {
            let Ok((tcp, _)) = listener.accept().await else { break };
            let _ = tcp.set_nodelay(true);
            // Si la conexión QUIC se cayó (por ejemplo, cambió el anfitrión), se reconecta.
            let bi = match conn.open_bi().await {
                Ok(s) => s,
                Err(_) => match ep2.connect(host.clone(), ALPN_MC).await {
                    Ok(c) => { conn = c; match conn.open_bi().await { Ok(s) => s, Err(_) => continue } }
                    Err(_) => continue,
                },
            };
            tokio::spawn(async move {
                let (mut send, mut recv) = bi;
                let (mut r, mut w) = tcp.into_split();
                let up = async { let _ = tokio::io::copy(&mut r, &mut send).await; let _ = send.finish(); };
                let down = async { let _ = tokio::io::copy(&mut recv, &mut w).await; };
                tokio::join!(up, down);
            });
        }
    });
    Ok(ClientTunnel { local_port, task, endpoint: ep })
}

