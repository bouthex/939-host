//! Amigos, presencia y mensajes de texto entre PCs con 939 Host, sin servidor central.
//!
//! - Identidad: la misma clave Ed25519 de cada PC (la del grupo). Quien manda un mensaje es la
//!   PC del otro lado de la conexión autenticada (QUIC/TLS 1.3): no se puede hacer pasar por otra.
//! - Invitación: un código de un solo uso que vence. Usarlo manda una solicitud que la otra
//!   persona acepta o rechaza. Ser amigos no da acceso a ningún servidor ni a ningún grupo.
//! - Presencia: cada 30 s una consulta liviana a cada amigo (no cada 5 s como el grupo). Al cerrar
//!   939 Host se avisa; si se corta internet, a los 90 s figura desconectado.
//! - Mensajes: texto de hasta 500 caracteres, cifrados de punta a punta por la conexión (un relay
//!   solo ve bytes cifrados). Si la otra PC no está conectada, quedan en la bandeja de salida de
//!   esta PC y se entregan cuando aparece. Límites contra el spam y bloqueo.
//! - Nada se publica: la lista de amigos y los mensajes quedan solo en cada PC.

use crate::{wire, Inner};
use anyhow::{anyhow, bail, Result};
use host939_core::{paths, sealed};
use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ALPN_FRIEND: &[u8] = b"939host/friend/1";
pub const MAX_TEXT: usize = 500;
/// Tope de cada pedido de amistad, presencia o mensaje (texto de 500 caracteres + dirección).
const MAX_FRAME: usize = 16 * 1024;
const MAX_FRIENDS: usize = 200;
const MAX_CHAT: usize = 500;
const MAX_INBOX_REQ: usize = 20;
/// Mensajes por minuto que se aceptan de cada amigo.
const MSG_PER_MIN: usize = 20;
pub const PRESENCE_EVERY: Duration = Duration::from_secs(30);
const OFFLINE_AFTER: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Friend { pub node: String, pub name: String, #[serde(default)] pub avatar: Option<String>, pub since: String, #[serde(default)] pub addr: Option<EndpointAddr> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendRequest { pub node: String, pub name: String, pub at: String, #[serde(default)] pub addr: Option<EndpointAddr> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendInvite { pub secret: String, pub expires: String }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub friends: Vec<Friend>,
    pub requests: Vec<FriendRequest>,
    pub blocked: Vec<String>,
    pub invites: Vec<FriendInvite>,
    /// «Disponible para jugar» (lo ven los amigos).
    #[serde(default)]
    pub available: bool,
    /// Solicitudes que mandó esta PC (solo esas PCs pueden responder «aceptada»).
    #[serde(default)]
    pub sent: Vec<String>,
    /// Grupos de chat de los que forma parte esta PC.
    #[serde(default)]
    pub groups: Vec<ChatGroup>,
}

/// Grupo de chat entre amigos (no tiene nada que ver con los grupos de servidores ni da acceso a
/// ninguno). Lo crea una PC con amigos suyos; cualquier integrante puede cambiarle el nombre.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatGroup {
    pub id: String,
    pub name: String,
    /// (PC, nombre visible). Incluye a esta PC.
    pub members: Vec<(String, String)>,
    /// Sube con cada cambio de nombre o de integrantes: gana el cambio más nuevo.
    pub rev: u64,
    pub created: String,
    /// Direcciones conocidas de cada integrante (dos integrantes pueden no ser amigas entre sí y
    /// no conocerse). La conexión igual se autentica con la identidad de cada PC.
    #[serde(default)]
    pub addrs: Vec<(String, EndpointAddr)>,
}
const MAX_GROUP: usize = 16;
const MAX_GROUPS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message { pub from_me: bool, pub text: String, pub at: String, #[serde(default)] pub delivered: bool,
    /// Grupos: quién lo mandó, su nombre, id del mensaje (para no duplicarlo) y a quiénes falta entregárselo.
    #[serde(default)] pub from: Option<String>, #[serde(default)] pub from_name: Option<String>,
    #[serde(default)] pub mid: Option<String>, #[serde(default)] pub pending: Vec<String> }

// La lista de amigos (incluye códigos de invitación) y los chats se guardan cifrados con DPAPI
// (`host939_core::sealed`). Los archivos `.json` en texto plano de versiones anteriores se migran
// al leerlos por primera vez.
fn store_path() -> std::path::PathBuf { paths::root().join("amigos.bin") }
fn store_legacy() -> std::path::PathBuf { paths::root().join("amigos.json") }
fn chat_file(node: &str, ext: &str) -> std::path::PathBuf { paths::root().join("mensajes").join(format!("{}.{ext}", &node[..node.len().min(64)])) }
pub fn load() -> Store { unsealed(sealed::read(&store_path(), Some(&store_legacy())), "la lista de amigos") }
fn save(s: &Store) -> Result<()> { sealed::write(&store_path(), s).map_err(|e| anyhow!(e.message)) }
pub fn chat(node: &str) -> Vec<Message> { if valid_node(node) { unsealed(sealed::read(&chat_file(node, "bin"), Some(&chat_file(node, "json"))), "un chat") } else { vec![] } }

/// Lo que no se pudo descifrar (otra cuenta de Windows, Windows reinstalado, archivo dañado) se
/// apartó sin borrarlo; acá queda el aviso para mostrarlo una vez en Amigos.
static UNREADABLE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
fn unsealed<T: Default>(r: sealed::Read<T>, what: &str) -> T {
    if let sealed::Read::Unreadable(p) = &r {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        UNREADABLE.lock().unwrap().push(format!("No se pudo descifrar {what} (pasa si cambió la cuenta de Windows o se reinstaló Windows). El archivo se guardó aparte como «{name}», sin borrarlo."));
    }
    r.or_default()
}
/// Avisos pendientes de archivos que no se pudieron descifrar (se devuelven una sola vez).
pub fn take_unreadable() -> Vec<String> { std::mem::take(&mut *UNREADABLE.lock().unwrap()) }
fn save_chat(node: &str, m: &[Message]) { let _ = sealed::write(&chat_file(node, "bin"), &m); }
fn valid_node(n: &str) -> bool { n.len() == 64 && n.bytes().all(|b| b.is_ascii_hexdigit()) }

/// Texto de un mensaje: sin caracteres de control (salvo saltos de línea), recortado y con tope.
pub fn clean_text(t: &str) -> Option<String> {
    let s: String = t.chars().filter(|c| !c.is_control() || *c == '\n').collect::<String>().trim().chars().take(MAX_TEXT).collect();
    (!s.is_empty()).then_some(s)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum FReq {
    /// Usar un código de invitación: pide la amistad.
    Request { secret: String, name: String, addr: Option<EndpointAddr> },
    /// La otra PC aceptó.
    Accepted { name: String, addr: Option<EndpointAddr> },
    Hello { name: String, available: bool, avatar: Option<String>, playing: Option<String> },
    Bye,
    Msg { text: String },
    Removed,
    /// Grupo nuevo o cambiado (nombre o integrantes).
    GroupSync { id: String, name: String, members: Vec<(String, String)>, rev: u64, #[serde(default)] addrs: Vec<(String, EndpointAddr)> },
    GroupMsg { id: String, text: String, mid: String, name: String },
    GroupLeave { id: String },
    /// «Está escribiendo». `to`: grupo, o `None` para el chat de a dos. Vence solo a los 6 s.
    Typing { to: Option<String>, on: bool },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum FResp { Ok, Hello { name: String, available: bool, avatar: Option<String>, playing: Option<String> }, Err { msg: String } }

/// Lo que se sabe en vivo de cada amigo (no se guarda).
#[derive(Debug, Clone, Default)]
pub struct Live { pub last_seen: Option<Instant>, pub available: bool, pub playing: Option<String>, pub msg_hits: Vec<Instant> }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendView { pub node: String, pub name: String, pub avatar: Option<String>, pub online: bool, pub available: bool, pub playing: Option<String>, pub unread: usize, pub shared_groups: Vec<String>,
    /// Está escribiendo en el chat con esta PC.
    pub typing: bool }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMemberView { pub node: String, pub name: String, pub online: bool, pub me: bool }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView { pub id: String, pub name: String, pub members: Vec<GroupMemberView>, pub unread: usize, pub typing: Vec<String> }

/// Quién está escribiendo: (chat, PC) → hasta cuándo. No se guarda; vence solo, así un aviso
/// de «dejó de escribir» que se pierde nunca deja el indicador pegado.
static TYPING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, String), Instant>>> = std::sync::LazyLock::new(Default::default);
const TYPING_TTL: Duration = Duration::from_secs(6);
fn typing_set(chat: &str, node: &str, on: bool) {
    let mut t = TYPING.lock().unwrap();
    t.retain(|_, until| *until > Instant::now());
    if on { t.insert((chat.into(), node.into()), Instant::now() + TYPING_TTL); } else { t.remove(&(chat.to_string(), node.to_string())); }
}
fn typing_clear(chat: &str, node: &str) { typing_set(chat, node, false); }
pub fn typing_in(chat: &str) -> Vec<String> {
    let now = Instant::now();
    TYPING.lock().unwrap().iter().filter(|((c, _), u)| c == chat && **u > now).map(|((_, n), _)| n.clone()).collect()
}
fn hint<'a>(g: &'a ChatGroup, node: &str) -> Option<&'a EndpointAddr> { g.addrs.iter().find(|(n, _)| n == node).map(|(_, a)| a) }
fn new_id() -> String { let mut b = [0u8; 32]; getrandom::getrandom(&mut b).expect("generador aleatorio del sistema"); hex::encode(b) }
fn group_of(s: &Store, id: &str) -> Option<ChatGroup> { s.groups.iter().find(|g| g.id == id).cloned() }
fn is_member(g: &ChatGroup, node: &str) -> bool { g.members.iter().any(|(n, _)| n == node) }

impl Inner {
    fn my_hello(&self) -> (String, bool, Option<String>, Option<String>) {
        let playing = host939_core::network::shared_servers().into_iter().find(|s| self.local.is_running(s)).map(|s| crate::server_name(&s));
        (self.name(), load().available, host939_core::avatar::own_hash(), playing)
    }

    async fn friend_rpc(&self, node: &str, req: &FReq, addr_hint: Option<&EndpointAddr>) -> Result<FResp> {
        let id: EndpointId = node.parse().map_err(|_| anyhow!("id inválido"))?;
        let addr = addr_hint.cloned().or_else(|| self.peers.read().unwrap().get(node).cloned()).unwrap_or_else(|| EndpointAddr::new(id));
        let c = tokio::time::timeout(Duration::from_secs(6), self.ep.connect(addr, ALPN_FRIEND)).await.map_err(|_| anyhow!("sin respuesta"))??;
        let (mut send, mut recv) = c.open_bi().await?;
        wire::write_msg(&mut send, req).await?;
        send.finish()?;
        let r: FResp = tokio::time::timeout(Duration::from_secs(6), wire::read_msg(&mut recv)).await.map_err(|_| anyhow!("sin respuesta"))??;
        Ok(r)
    }
}

impl crate::GroupNode {
    /// Código para que otra persona pida ser tu amiga (un solo uso, vence en 48 h).
    pub fn friend_invite(&self) -> Result<String> {
        let mut s = load();
        let now = chrono::Utc::now();
        s.invites.retain(|i| chrono::DateTime::parse_from_rfc3339(&i.expires).map(|e| e > now).unwrap_or(false));
        if s.invites.len() >= 20 { bail!("Hay demasiados códigos sin usar. Esperá a que venzan."); }
        let secret = host939_core::network::random_secret();
        s.invites.push(FriendInvite { secret: secret.clone(), expires: (now + chrono::Duration::hours(48)).to_rfc3339() });
        save(&s)?;
        // Igual que las invitaciones de grupo: con relays el código no lleva las IPs de esta PC.
        Ok(format!("amigo:{}", host939_p2p::make_ticket(&host939_p2p::shareable(&self.inner.ep.addr(), self.inner.relays), &secret)))
    }

    /// Usa el código de otra persona: le llega una solicitud que puede aceptar o rechazar.
    pub async fn friend_add(&self, code: &str) -> Result<String> {
        let t = code.trim().strip_prefix("amigo:").ok_or_else(|| anyhow!("Ese no es un código de amistad (empieza con «amigo:»)."))?;
        let (addr, secret) = host939_p2p::parse_ticket(t)?;
        let node = addr.id.to_string();
        if node == self.inner.me { bail!("Ese código es tuyo."); }
        if load().friends.iter().any(|f| f.node == node) { bail!("Ya son amigos."); }
        let me = host939_p2p::shareable(&self.inner.ep.addr(), self.inner.relays);
        match self.inner.friend_rpc(&node, &FReq::Request { secret, name: self.inner.name(), addr: Some(me) }, Some(&addr)).await? {
            FResp::Ok => {
                // Se recuerda que la pidió esta PC (sin agregarla como amiga todavía).
                let mut s = load();
                if !s.sent.contains(&node) { s.sent.push(node.clone()); }
                if s.sent.len() > 50 { let k = s.sent.len() - 50; s.sent.drain(0..k); }
                save(&s)?;
                self.inner.peers.write().unwrap().entry(node.clone()).or_insert(addr);
                Ok(node)
            }
            FResp::Err { msg } => bail!(msg),
            _ => bail!("respuesta inesperada"),
        }
    }

    pub fn friend_requests(&self) -> Vec<FriendRequest> { load().requests }

    /// Acepta o rechaza una solicitud recibida.
    pub async fn friend_answer(&self, node: &str, accept: bool) -> Result<()> {
        let mut s = load();
        let Some(i) = s.requests.iter().position(|r| r.node == node) else { bail!("Esa solicitud ya no está.") };
        let r = s.requests.remove(i);
        if accept {
            if s.friends.len() >= MAX_FRIENDS { bail!("Llegaste al máximo de amigos."); }
            s.friends.push(Friend { node: r.node.clone(), name: host939_core::pcname::clean_name(&r.name).unwrap_or_else(|| "Amigo".into()), avatar: None, since: chrono::Local::now().to_rfc3339(), addr: r.addr.clone() });
        }
        save(&s)?;
        if accept {
            let me = host939_p2p::shareable(&self.inner.ep.addr(), self.inner.relays);
            let _ = self.inner.friend_rpc(node, &FReq::Accepted { name: self.inner.name(), addr: Some(me) }, r.addr.as_ref()).await;
            self.presence_once().await;
        }
        Ok(())
    }

    pub fn friend_remove(&self, node: &str, block: bool) -> Result<()> {
        let mut s = load();
        s.friends.retain(|f| f.node != node);
        s.requests.retain(|r| r.node != node);
        if block && valid_node(node) && !s.blocked.iter().any(|b| b == node) { s.blocked.push(node.to_string()); }
        save(&s)?;
        let inner = self.inner.clone();
        let n = node.to_string();
        tokio::spawn(async move { let _ = inner.friend_rpc(&n, &FReq::Removed, None).await; });
        Ok(())
    }

    pub fn friend_unblock(&self, node: &str) -> Result<()> { let mut s = load(); s.blocked.retain(|b| b != node); save(&s) }
    pub fn friend_set_available(&self, on: bool) -> Result<()> { let mut s = load(); s.available = on; save(&s) }

    /// Manda un mensaje. Si la otra PC no está conectada queda pendiente y se entrega después.
    pub async fn friend_send(&self, node: &str, text: &str) -> Result<bool> {
        if !load().friends.iter().any(|f| f.node == node) { bail!("Solo se pueden mandar mensajes a amigos."); }
        let text = clean_text(text).ok_or_else(|| anyhow!("El mensaje está vacío."))?;
        let mut c = chat(node);
        let delivered = matches!(self.inner.friend_rpc(node, &FReq::Msg { text: text.clone() }, None).await, Ok(FResp::Ok));
        c.push(Message { from_me: true, text, at: chrono::Local::now().to_rfc3339(), delivered, from: None, from_name: None, mid: None, pending: vec![] });
        if c.len() > MAX_CHAT { let k = c.len() - MAX_CHAT; c.drain(0..k); }
        save_chat(node, &c);
        Ok(delivered)
    }

    pub fn friend_chat(&self, node: &str) -> Vec<Message> {
        self.inner.unread.lock().unwrap().remove(node);
        chat(node)
    }

    pub fn friends(&self) -> Vec<FriendView> {
        let s = load();
        let live = self.inner.friends_live.lock().unwrap().clone();
        let unread = self.inner.unread.lock().unwrap().clone();
        s.friends.iter().map(|f| {
            let l = live.get(&f.node).cloned().unwrap_or_default();
            let online = l.last_seen.map(|t| t.elapsed() < OFFLINE_AFTER).unwrap_or(false);
            // Grupos en común: servidores donde las dos PCs figuran en la lista firmada.
            let shared_groups = host939_core::network::shared_servers().into_iter()
                .filter(|sid| { let sh = host939_core::network::load_share(sid); sh.member(&f.node).is_some() && sh.member(&self.inner.me).is_some() })
                .map(|sid| crate::server_name(&sid)).collect();
            FriendView { node: f.node.clone(), name: f.name.clone(), avatar: f.avatar.clone().filter(|h| host939_core::avatar::has_peer(h)), online, available: online && l.available, playing: if online { l.playing } else { None }, unread: *unread.get(&f.node).unwrap_or(&0), shared_groups, typing: online && typing_in(&f.node).iter().any(|n| *n == f.node) }
        }).collect()
    }

    /* ── Grupos de chat ── */

    /// Crea un grupo con amigos de esta PC. Devuelve su id.
    pub async fn group_create(&self, name: &str, members: &[String]) -> Result<String> {
        let mut s = load();
        let name = clean_group_name(name).ok_or_else(|| anyhow!("Poné un nombre para el grupo."))?;
        if s.groups.len() >= MAX_GROUPS { bail!("Llegaste al máximo de grupos."); }
        let mut list: Vec<(String, String)> = vec![(self.inner.me.clone(), self.inner.name())];
        for m in members {
            let f = s.friends.iter().find(|f| f.node == *m).ok_or_else(|| anyhow!("Solo podés agregar a tus amigos."))?;
            if !list.iter().any(|(n, _)| n == m) { list.push((f.node.clone(), f.name.clone())); }
        }
        if list.len() < 2 { bail!("Elegí al menos un amigo."); }
        if list.len() > MAX_GROUP { bail!("Un grupo puede tener hasta {MAX_GROUP} personas."); }
        let addrs = self.group_addrs(&s, &list, &[]);
        let g = ChatGroup { id: new_id(), name, members: list, rev: 1, created: chrono::Local::now().to_rfc3339(), addrs };
        s.groups.push(g.clone());
        save(&s)?;
        self.group_broadcast(&g, &FReq::GroupSync { id: g.id.clone(), name: g.name.clone(), members: g.members.clone(), rev: g.rev, addrs: g.addrs.clone() }).await;
        Ok(g.id)
    }

    /// Cambia el nombre (cualquier integrante). Lo ven todos.
    pub async fn group_rename(&self, id: &str, name: &str) -> Result<()> {
        let mut s = load();
        let name = clean_group_name(name).ok_or_else(|| anyhow!("Poné un nombre para el grupo."))?;
        let g = s.groups.iter_mut().find(|g| g.id == id).ok_or_else(|| anyhow!("Ese grupo ya no existe."))?;
        g.name = name; g.rev += 1;
        let mut g = g.clone();
        g.addrs = self.group_addrs(&load(), &g.members, &g.addrs);
        if let Some(x) = s.groups.iter_mut().find(|x| x.id == id) { x.addrs = g.addrs.clone(); }
        save(&s)?;
        self.group_broadcast(&g, &FReq::GroupSync { id: g.id.clone(), name: g.name.clone(), members: g.members.clone(), rev: g.rev, addrs: g.addrs.clone() }).await;
        Ok(())
    }

    /// Suma amigos de esta PC a un grupo que ya existe (cualquier integrante puede). Las demás
    /// PCs lo aceptan porque viene de un integrante; la persona nueva, porque es amiga de quien
    /// la agregó.
    pub async fn group_add_members(&self, id: &str, members: &[String]) -> Result<usize> {
        let mut s = load();
        let mut g = group_of(&s, id).ok_or_else(|| anyhow!("Ese grupo ya no existe."))?;
        let mut added = 0;
        for m in members {
            if is_member(&g, m) { continue; }
            let f = s.friends.iter().find(|f| f.node == *m).ok_or_else(|| anyhow!("Solo podés agregar a tus amigos."))?;
            g.members.push((f.node.clone(), f.name.clone()));
            added += 1;
        }
        if added == 0 { bail!("Esas personas ya están en el grupo."); }
        if g.members.len() > MAX_GROUP { bail!("Un grupo puede tener hasta {MAX_GROUP} personas."); }
        g.rev += 1;
        g.addrs = self.group_addrs(&s, &g.members, &g.addrs);
        if let Some(x) = s.groups.iter_mut().find(|x| x.id == id) { *x = g.clone(); }
        save(&s)?;
        self.group_broadcast(&g, &FReq::GroupSync { id: g.id.clone(), name: g.name.clone(), members: g.members.clone(), rev: g.rev, addrs: g.addrs.clone() }).await;
        Ok(added)
    }

    /// Salir del grupo: se avisa a los demás y se borra de esta PC (el historial queda guardado).
    pub async fn group_leave(&self, id: &str) -> Result<()> {
        let mut s = load();
        let g = group_of(&s, id).ok_or_else(|| anyhow!("Ese grupo ya no existe."))?;
        s.groups.retain(|x| x.id != id);
        save(&s)?;
        self.group_broadcast(&g, &FReq::GroupLeave { id: id.into() }).await;
        Ok(())
    }

    pub async fn group_send(&self, id: &str, text: &str) -> Result<usize> {
        let g = group_of(&load(), id).ok_or_else(|| anyhow!("Ese grupo ya no existe."))?;
        let text = clean_text(text).ok_or_else(|| anyhow!("El mensaje está vacío."))?;
        let mid = new_id();
        let req = FReq::GroupMsg { id: id.into(), text: text.clone(), mid: mid.clone(), name: self.inner.name() };
        let others: Vec<String> = g.members.iter().map(|(n, _)| n.clone()).filter(|n| *n != self.inner.me).collect();
        let futs = others.iter().map(|n| { let inner = self.inner.clone(); let req = &req; let n = n.clone(); let h = hint(&g, &n).cloned(); async move { (n.clone(), matches!(tokio::time::timeout(Duration::from_secs(6), inner.friend_rpc(&n, req, h.as_ref())).await, Ok(Ok(FResp::Ok)))) } });
        let pending: Vec<String> = futures_util::future::join_all(futs).await.into_iter().filter(|(_, ok)| !ok).map(|(n, _)| n).collect();
        let mut c = chat(id);
        c.push(Message { from_me: true, text, at: chrono::Local::now().to_rfc3339(), delivered: pending.is_empty(), from: Some(self.inner.me.clone()), from_name: None, mid: Some(mid), pending: pending.clone() });
        if c.len() > MAX_CHAT { let k = c.len() - MAX_CHAT; c.drain(0..k); }
        save_chat(id, &c);
        Ok(pending.len())
    }

    pub fn group_chat(&self, id: &str) -> Vec<Message> {
        self.inner.unread.lock().unwrap().remove(id);
        chat(id)
    }

    pub fn groups(&self) -> Vec<GroupView> {
        let s = load();
        let live = self.inner.friends_live.lock().unwrap().clone();
        let unread = self.inner.unread.lock().unwrap().clone();
        s.groups.iter().map(|g| {
            let online = |n: &str| n == self.inner.me || live.get(n).and_then(|l| l.last_seen).map(|t| t.elapsed() < OFFLINE_AFTER).unwrap_or(false);
            let name_of = |n: &str| s.friends.iter().find(|f| f.node == n).map(|f| f.name.clone()).or_else(|| g.members.iter().find(|(x, _)| x == n).map(|(_, nm)| nm.clone())).unwrap_or_default();
            GroupView { id: g.id.clone(), name: g.name.clone(), unread: *unread.get(&g.id).unwrap_or(&0),
                members: g.members.iter().map(|(n, _)| GroupMemberView { node: n.clone(), name: if *n == self.inner.me { self.inner.name() } else { name_of(n) }, online: online(n), me: *n == self.inner.me }).collect(),
                typing: typing_in(&g.id).iter().filter(|n| **n != self.inner.me && is_member(g, n)).map(|n| name_of(n)).collect() }
        }).collect()
    }

    /// «Está escribiendo» hacia un amigo (`to` = su PC) o un grupo (`to` = id del grupo).
    pub async fn typing(&self, to: &str, on: bool) {
        let s = load();
        if let Some(g) = group_of(&s, to) {
            self.group_broadcast(&g, &FReq::Typing { to: Some(to.into()), on }).await;
        } else if s.friends.iter().any(|f| f.node == to) {
            let _ = tokio::time::timeout(Duration::from_secs(3), self.inner.friend_rpc(to, &FReq::Typing { to: None, on }, None)).await;
        }
    }

    async fn group_broadcast(&self, g: &ChatGroup, req: &FReq) {
        let futs = g.members.iter().filter(|(n, _)| *n != self.inner.me).map(|(n, _)| {
            let inner = self.inner.clone(); let n = n.clone(); let h = hint(g, &n).cloned();
            async move { let _ = tokio::time::timeout(Duration::from_secs(4), inner.friend_rpc(&n, req, h.as_ref())).await; }
        });
        futures_util::future::join_all(futs).await;
    }

    /// Direcciones para el grupo: la de esta PC y las de sus amigos (sin IPs si hay relays),
    /// conservando las que ya se conocían de las demás.
    fn group_addrs(&self, s: &Store, members: &[(String, String)], prev: &[(String, EndpointAddr)]) -> Vec<(String, EndpointAddr)> {
        members.iter().filter_map(|(n, _)| {
            if *n == self.inner.me { return Some((n.clone(), host939_p2p::shareable(&self.inner.ep.addr(), self.inner.relays))); }
            s.friends.iter().find(|f| f.node == *n).and_then(|f| f.addr.clone()).or_else(|| prev.iter().find(|(x, _)| x == n).map(|(_, a)| a.clone())).map(|a| (n.clone(), a))
        }).collect()
    }

    /// Reintenta los mensajes de grupo que no llegaron a alguien.
    async fn group_retry(&self) {
        let s = load();
        for g in &s.groups {
            let mut c = chat(&g.id);
            let mut changed = false;
            for m in c.iter_mut().filter(|m| m.from_me && !m.pending.is_empty()) {
                let Some(mid) = m.mid.clone() else { continue };
                let req = FReq::GroupMsg { id: g.id.clone(), text: m.text.clone(), mid, name: self.inner.name() };
                let mut still = vec![];
                for n in std::mem::take(&mut m.pending) {
                    if !is_member(g, &n) { continue; }
                    if matches!(tokio::time::timeout(Duration::from_secs(4), self.inner.friend_rpc(&n, &req, hint(g, &n))).await, Ok(Ok(FResp::Ok))) { changed = true; } else { still.push(n); }
                }
                m.pending = still;
                if m.pending.is_empty() { m.delivered = true; }
            }
            if changed { save_chat(&g.id, &c); }
        }
    }

    /// Una vuelta de presencia: saluda a cada amigo y entrega los mensajes pendientes.
    pub async fn presence_once(&self) {
        self.group_retry().await;
        let s = load();
        let (name, available, avatar, playing) = self.inner.my_hello();
        for f in s.friends.iter().take(MAX_FRIENDS) {
            let inner = self.inner.clone();
            let f = f.clone();
            let hello = FReq::Hello { name: name.clone(), available, avatar: avatar.clone(), playing: playing.clone() };
            tokio::spawn(async move {
                match inner.friend_rpc(&f.node, &hello, f.addr.as_ref()).await {
                    Ok(FResp::Hello { name, available, avatar, playing }) => {
                        inner.friend_seen(&f.node, &name, available, avatar, playing).await;
                        // Mensajes que no se pudieron entregar antes.
                        let mut c = chat(&f.node);
                        let mut changed = false;
                        for m in c.iter_mut().filter(|m| m.from_me && !m.delivered) {
                            if matches!(inner.friend_rpc(&f.node, &FReq::Msg { text: m.text.clone() }, f.addr.as_ref()).await, Ok(FResp::Ok)) { m.delivered = true; changed = true; } else { break; }
                        }
                        if changed { save_chat(&f.node, &c); }
                    }
                    _ => {}
                }
            });
        }
    }

    /// Al cerrar 939 Host: avisa a los amigos conectados.
    pub async fn friends_bye(&self) {
        let live = self.inner.friends_live.lock().unwrap().clone();
        let futs = load().friends.into_iter().filter(|f| live.get(&f.node).and_then(|l| l.last_seen).map(|t| t.elapsed() < OFFLINE_AFTER).unwrap_or(false))
            .map(|f| { let inner = self.inner.clone(); async move { let _ = tokio::time::timeout(Duration::from_secs(2), inner.friend_rpc(&f.node, &FReq::Bye, f.addr.as_ref())).await; } });
        futures_util::future::join_all(futs).await;
    }
}

impl Inner {
    async fn friend_seen(&self, node: &str, name: &str, available: bool, avatar: Option<String>, playing: Option<String>) {
        {
            let mut live = self.friends_live.lock().unwrap();
            let l = live.entry(node.to_string()).or_default();
            l.last_seen = Some(Instant::now());
            l.available = available;
            l.playing = playing.map(|p| p.chars().take(60).collect());
        }
        // Nombre visible y foto al día (la foto se pide aparte, por huella, con el mismo control).
        let mut s = load();
        if let Some(f) = s.friends.iter_mut().find(|f| f.node == node) {
            let n = host939_core::pcname::clean_name(name).unwrap_or_else(|| f.name.clone());
            let a = avatar.filter(|h| host939_core::avatar::valid_hash(h));
            if f.name != n || f.avatar != a { f.name = n; f.avatar = a; let _ = save(&s); }
        }
        self.local.changed("");
    }
}

/// Atiende a otras PCs: solicitudes, presencia y mensajes.
#[derive(Clone)]
pub(crate) struct FriendH(pub(crate) Arc<Inner>);
impl std::fmt::Debug for FriendH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "friend") } }
impl iroh::protocol::ProtocolHandler for FriendH {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let remote = conn.remote_id().to_string();
        if load().blocked.iter().any(|b| *b == remote) { conn.close(1u32.into(), b"bloqueado"); return Ok(()); }
        // Cualquier PC de Internet que conozca esta identidad puede llegar hasta acá: mismo límite de
        // pedidos por PC que el control del grupo y mensajes chicos (no los 8 MB generales).
        if !self.0.ctrl_limit.lock().unwrap().allow(&remote) { conn.close(2u32.into(), b"limite"); return Ok(()); }
        let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
        let Ok(Ok(raw)) = tokio::time::timeout(Duration::from_secs(10), wire::read_frame(&mut recv, MAX_FRAME)).await else { return Ok(()) };
        let Ok(req) = serde_json::from_slice::<FReq>(&raw) else { return Ok(()) };
        let resp = handle(&self.0, &remote, req).await;
        let _ = wire::write_msg(&mut send, &resp).await;
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
        Ok(())
    }
}

async fn handle(inner: &Arc<Inner>, remote: &str, req: FReq) -> FResp {
    let is_friend = load().friends.iter().any(|f| f.node == remote);
    match req {
        FReq::Request { secret, name, addr } => {
            let mut s = load();
            let now = chrono::Utc::now();
            let valid = s.invites.iter().position(|i| crate::ct_eq(i.secret.as_bytes(), secret.as_bytes()) && chrono::DateTime::parse_from_rfc3339(&i.expires).map(|e| e > now).unwrap_or(false));
            let Some(i) = valid else { return FResp::Err { msg: "El código no es válido o ya venció.".into() } };
            s.invites.remove(i);
            if is_friend { let _ = save(&s); return FResp::Ok; }
            if s.requests.len() >= MAX_INBOX_REQ { return FResp::Err { msg: "La otra persona tiene demasiadas solicitudes pendientes.".into() }; }
            s.requests.retain(|r| r.node != remote);
            // La dirección que manda solo puede ser la suya.
            let addr = addr.filter(|a| a.id.to_string() == remote).map(|a| host939_p2p::hearsay(&a, inner.relays));
            s.requests.push(FriendRequest { node: remote.into(), name: host939_core::pcname::clean_name(&name).unwrap_or_else(|| "Alguien".into()), at: chrono::Local::now().to_rfc3339(), addr });
            let _ = save(&s);
            inner.local.note("", &format!("{} quiere ser tu amigo en 939 Host.", host939_core::pcname::clean_name(&name).unwrap_or_else(|| "Alguien".into())));
            inner.local.changed("");
            FResp::Ok
        }
        FReq::Accepted { name, addr } => {
            // Solo vale si esta PC mandó la solicitud (tiene su dirección recordada) y no es amiga ya.
            let mut s = load();
            if s.friends.iter().any(|f| f.node == remote) { return FResp::Ok; }
            let Some(i) = s.sent.iter().position(|n| n == remote) else { return FResp::Err { msg: "No hay una solicitud pendiente.".into() } };
            s.sent.remove(i);
            let addr = addr.filter(|a| a.id.to_string() == remote).map(|a| host939_p2p::hearsay(&a, inner.relays));
            s.friends.push(Friend { node: remote.into(), name: host939_core::pcname::clean_name(&name).unwrap_or_else(|| "Amigo".into()), avatar: None, since: chrono::Local::now().to_rfc3339(), addr });
            let _ = save(&s);
            inner.local.note("", &format!("{} aceptó tu solicitud de amistad.", host939_core::pcname::clean_name(&name).unwrap_or_else(|| "Alguien".into())));
            inner.local.changed("");
            FResp::Ok
        }
        FReq::GroupSync { id, name, members, rev, addrs } => {
            let mut s = load();
            let Some(name) = clean_group_name(&name) else { return FResp::Err { msg: "Nombre inválido.".into() } };
            if !valid_node(&id) || members.len() > MAX_GROUP || !members.iter().all(|(n, _)| valid_node(n)) { return FResp::Err { msg: "Grupo inválido.".into() }; }
            if !members.iter().any(|(n, _)| n == remote) || !members.iter().any(|(n, _)| *n == inner.me) { return FResp::Err { msg: "Grupo inválido.".into() }; }
            let members: Vec<(String, String)> = members.into_iter().map(|(n, nm)| (n, host939_core::pcname::clean_name(&nm).unwrap_or_else(|| "Alguien".into()))).collect();
            // Solo direcciones de integrantes, que correspondan a su identidad (y sin IPs si hay relays).
            let addrs: Vec<(String, EndpointAddr)> = addrs.into_iter().filter(|(n, a)| *n != inner.me && a.id.to_string() == *n && members.iter().any(|(m, _)| m == n)).take(MAX_GROUP).map(|(n, a)| (n, host939_p2p::hearsay(&a, inner.relays))).collect();
            match s.groups.iter_mut().find(|g| g.id == id) {
                // Cambio de un grupo existente: solo de un integrante y si es más nuevo.
                Some(g) => {
                    if !is_member(g, remote) { return FResp::Err { msg: "No sos parte de ese grupo.".into() }; }
                    if rev <= g.rev { return FResp::Ok; }
                    g.name = name; g.members = members; g.rev = rev; g.addrs = addrs;
                }
                // Grupo nuevo: solo si lo crea un amigo (nadie te mete en un grupo sin serlo).
                None => {
                    if !is_friend { return FResp::Err { msg: "No son amigos.".into() }; }
                    if s.groups.len() >= MAX_GROUPS { return FResp::Err { msg: "La otra persona tiene demasiados grupos.".into() }; }
                    s.groups.push(ChatGroup { id: id.clone(), name, members, rev, created: chrono::Local::now().to_rfc3339(), addrs });
                }
            }
            let _ = save(&s);
            inner.local.changed("");
            inner.local.friend_message(&id);
            FResp::Ok
        }
        FReq::GroupMsg { id, text, mid, name } => {
            let s = load();
            let Some(g) = group_of(&s, &id) else { return FResp::Err { msg: "No estás en ese grupo.".into() } };
            if !is_member(&g, remote) { return FResp::Err { msg: "No sos parte de ese grupo.".into() }; }
            let Some(text) = clean_text(&text) else { return FResp::Err { msg: "Mensaje vacío.".into() } };
            let allowed = {
                let mut live = inner.friends_live.lock().unwrap();
                let l = live.entry(remote.to_string()).or_default();
                l.msg_hits.retain(|t| t.elapsed() < Duration::from_secs(60));
                if l.msg_hits.len() >= MSG_PER_MIN { false } else { l.msg_hits.push(Instant::now()); true }
            };
            if !allowed { return FResp::Err { msg: "Demasiados mensajes seguidos. Esperá un momento.".into() }; }
            let mut c = chat(&id);
            // El mismo mensaje reintentado no se guarda dos veces.
            if c.iter().any(|m| m.mid.as_deref() == Some(mid.as_str())) { return FResp::Ok; }
            c.push(Message { from_me: false, text, at: chrono::Local::now().to_rfc3339(), delivered: true, from: Some(remote.into()), from_name: host939_core::pcname::clean_name(&name), mid: Some(mid), pending: vec![] });
            if c.len() > MAX_CHAT { let k = c.len() - MAX_CHAT; c.drain(0..k); }
            save_chat(&id, &c);
            typing_clear(&id, remote);
            *inner.unread.lock().unwrap().entry(id.clone()).or_default() += 1;
            inner.local.friend_message(&id);
            FResp::Ok
        }
        FReq::GroupLeave { id } => {
            let mut s = load();
            if let Some(g) = s.groups.iter_mut().find(|g| g.id == id) {
                if is_member(g, remote) { g.members.retain(|(n, _)| n != remote); let _ = save(&s); inner.local.friend_message(&id); }
            }
            FResp::Ok
        }
        FReq::Typing { to, on } => {
            let chat_key = match &to {
                Some(gid) => match group_of(&load(), gid) { Some(g) if is_member(&g, remote) => gid.clone(), _ => return FResp::Ok },
                None if is_friend => remote.to_string(),
                None => return FResp::Ok,
            };
            typing_set(&chat_key, remote, on);
            inner.local.friend_typing(&chat_key);
            FResp::Ok
        }
        _ if !is_friend => FResp::Err { msg: "No son amigos.".into() },
        FReq::Hello { name, available, avatar, playing } => {
            inner.friend_seen(remote, &name, available, avatar, playing).await;
            let (name, available, avatar, playing) = inner.my_hello();
            FResp::Hello { name, available, avatar, playing }
        }
        FReq::Bye => {
            if let Some(l) = inner.friends_live.lock().unwrap().get_mut(remote) { l.last_seen = None; }
            inner.local.changed("");
            FResp::Ok
        }
        FReq::Msg { text } => {
            let Some(text) = clean_text(&text) else { return FResp::Err { msg: "Mensaje vacío.".into() } };
            // Límite contra el spam: hasta 20 mensajes por minuto de cada amigo.
            let allowed = {
                let mut live = inner.friends_live.lock().unwrap();
                let l = live.entry(remote.to_string()).or_default();
                l.msg_hits.retain(|t| t.elapsed() < Duration::from_secs(60));
                if l.msg_hits.len() >= MSG_PER_MIN { false } else { l.msg_hits.push(Instant::now()); true }
            };
            if !allowed { return FResp::Err { msg: "Demasiados mensajes seguidos. Esperá un momento.".into() }; }
            let mut c = chat(remote);
            c.push(Message { from_me: false, text, at: chrono::Local::now().to_rfc3339(), delivered: true, from: None, from_name: None, mid: None, pending: vec![] });
            typing_clear(remote, remote);
            if c.len() > MAX_CHAT { let k = c.len() - MAX_CHAT; c.drain(0..k); }
            save_chat(remote, &c);
            *inner.unread.lock().unwrap().entry(remote.to_string()).or_default() += 1;
            inner.local.friend_message(remote);
            FResp::Ok
        }
        FReq::Removed => {
            let mut s = load();
            s.friends.retain(|f| f.node != remote);
            let _ = save(&s);
            inner.local.changed("");
            FResp::Ok
        }
    }
}

fn clean_group_name(n: &str) -> Option<String> {
    let s: String = n.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(40).collect();
    (!s.is_empty()).then_some(s)
}

/// Vista de las solicitudes y la lista (para pruebas y la interfaz).
pub fn blocked() -> Vec<String> { load().blocked }


#[cfg(test)]
mod tests {
    #[test]
    fn texto_limpio_y_con_tope() {
        assert_eq!(super::clean_text("  hola\u{0007}  "), Some("hola".into()));
        assert_eq!(super::clean_text("   "), None);
        assert_eq!(super::clean_text(&"a".repeat(900)).unwrap().len(), super::MAX_TEXT);
        assert_eq!(super::clean_text("línea 1\nlínea 2").unwrap(), "línea 1\nlínea 2");
    }

    #[test]
    fn amigos_ilegibles_se_apartan_con_aviso() {
        let _ = host939_core::paths::init(std::env::temp_dir().join(format!("939host-amigos-{}", std::process::id())));
        let root = host939_core::paths::root().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        // Lista vieja en texto plano: se migra a cifrado y se puede volver a leer.
        let _ = std::fs::remove_file(root.join("amigos.bin"));
        std::fs::write(root.join("amigos.json"), br#"{"friends":[],"requests":[],"blocked":[],"invites":[],"available":true,"sent":[]}"#).unwrap();
        assert!(super::load().available);
        assert!(!root.join("amigos.json").exists() && root.join("amigos.bin").exists());
        assert!(super::take_unreadable().is_empty());
        // Archivo que no se puede descifrar: lista vacía, archivo apartado y un aviso (una vez).
        std::fs::write(root.join("amigos.bin"), b"no es de esta cuenta").unwrap();
        assert!(!super::load().available);
        let n = super::take_unreadable();
        assert_eq!(n.len(), 1, "{n:?}");
        assert!(n[0].contains("ilegible"));
        assert!(super::take_unreadable().is_empty(), "el aviso se muestra una sola vez");
        assert!(std::fs::read_dir(&root).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("ilegible")));
    }
}
