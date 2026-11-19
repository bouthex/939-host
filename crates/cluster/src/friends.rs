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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message { pub from_me: bool, pub text: String, pub at: String, #[serde(default)] pub delivered: bool }

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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum FResp { Ok, Hello { name: String, available: bool, avatar: Option<String>, playing: Option<String> }, Err { msg: String } }

/// Lo que se sabe en vivo de cada amigo (no se guarda).
#[derive(Debug, Clone, Default)]
pub struct Live { pub last_seen: Option<Instant>, pub available: bool, pub playing: Option<String>, pub msg_hits: Vec<Instant> }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendView { pub node: String, pub name: String, pub avatar: Option<String>, pub online: bool, pub available: bool, pub playing: Option<String>, pub unread: usize, pub shared_groups: Vec<String> }

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
        c.push(Message { from_me: true, text, at: chrono::Local::now().to_rfc3339(), delivered });
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
            FriendView { node: f.node.clone(), name: f.name.clone(), avatar: f.avatar.clone().filter(|h| host939_core::avatar::has_peer(h)), online, available: online && l.available, playing: if online { l.playing } else { None }, unread: *unread.get(&f.node).unwrap_or(&0), shared_groups }
        }).collect()
    }

    /// Una vuelta de presencia: saluda a cada amigo y entrega los mensajes pendientes.
    pub async fn presence_once(&self) {
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
            c.push(Message { from_me: false, text, at: chrono::Local::now().to_rfc3339(), delivered: true });
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
