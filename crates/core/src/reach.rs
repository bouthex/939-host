//! Comprobación real de que un servidor recibe jugadores y explicación de entradas fallidas.
//!
//! - `status`: hace lo mismo que la lista de servidores de Minecraft (handshake + «status»)
//!   y devuelve la versión y el protocolo que anuncia el servidor.
//! - `JoinWatch`: mira la consola. Si un jugador se autentica pero nunca llega a «joined the
//!   game» y se desconecta enseguida, fue su propio Minecraft el que cortó: casi siempre porque
//!   tiene mods que el servidor no tiene (el juego muestra «Error de protocolo de red»).

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub version: String,
    pub protocol: i64,
    pub online: i64,
    pub max: i64,
    pub latency_ms: u64,
}

fn varint(mut v: u32, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 { out.push(b); break; }
        out.push(b | 0x80);
    }
}

fn read_varint(r: &mut impl Read) -> std::io::Result<u32> {
    let mut n = 0u32;
    for i in 0..5 {
        let mut b = [0u8];
        r.read_exact(&mut b)?;
        n |= ((b[0] & 0x7f) as u32) << (7 * i);
        if b[0] & 0x80 == 0 { return Ok(n); }
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "varint demasiado largo"))
}

/// Una consulta de estado como la del juego. `host` puede ser un nombre o una IP.
pub fn status(host: &str, port: u16, timeout: Duration) -> Result<StatusInfo, String> {
    let addr = (host, port).to_socket_addrs().map_err(|e| format!("no se pudo resolver {host}: {e}"))?.next().ok_or("sin dirección")?;
    let t0 = Instant::now();
    let mut c = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("no se pudo conectar a {host}:{port} ({e})"))?;
    c.set_read_timeout(Some(timeout)).ok();
    c.set_write_timeout(Some(timeout)).ok();
    let mut body = vec![0u8];
    varint(u32::MAX, &mut body); // protocolo -1: «solo quiero el estado»
    varint(host.len() as u32, &mut body);
    body.extend_from_slice(host.as_bytes());
    body.extend_from_slice(&port.to_be_bytes());
    varint(1, &mut body);
    let mut pkt = vec![];
    varint(body.len() as u32, &mut pkt);
    pkt.extend(body);
    pkt.extend_from_slice(&[1, 0]);
    c.write_all(&pkt).map_err(|e| format!("el servidor cortó la conexión ({e})"))?;
    let len = read_varint(&mut c).map_err(|e| format!("el servidor no respondió como Minecraft ({e})"))? as usize;
    if len == 0 || len > 1 << 20 { return Err("respuesta inválida".into()); }
    let mut buf = vec![0u8; len];
    c.read_exact(&mut buf).map_err(|e| format!("respuesta incompleta ({e})"))?;
    let latency_ms = t0.elapsed().as_millis() as u64;
    let mut cur = std::io::Cursor::new(&buf[..]);
    let id = read_varint(&mut cur).map_err(|e| e.to_string())?;
    if id != 0 { return Err(format!("paquete inesperado {id}")); }
    let slen = read_varint(&mut cur).map_err(|e| e.to_string())? as usize;
    let start = cur.position() as usize;
    let json = buf.get(start..start + slen).ok_or("respuesta cortada")?;
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("estado ilegible ({e})"))?;
    Ok(StatusInfo {
        version: v["version"]["name"].as_str().unwrap_or("").to_string(),
        protocol: v["version"]["protocol"].as_i64().unwrap_or(-1),
        online: v["players"]["online"].as_i64().unwrap_or(0),
        max: v["players"]["max"].as_i64().unwrap_or(0),
        latency_ms,
    })
}

/// Reintenta unos segundos: en Windows la primera consulta justo después de «Done» a veces se corta.
pub fn status_retry(host: &str, port: u16, tries: u32) -> Result<StatusInfo, String> {
    let mut last = String::new();
    for i in 0..tries.max(1) {
        match status(host, port, Duration::from_secs(5)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
        if i + 1 < tries { std::thread::sleep(Duration::from_millis(1200)); }
    }
    Err(last)
}

/// Reintenta hasta que pase `limit`. Recién encendido (convirtiendo el mundo, o en una PC lenta)
/// el servidor puede cortar las consultas al instante: con una cantidad fija de intentos rápidos
/// se agotaban en unos segundos y se informaba que no atendía conexiones cuando sí lo hacía.
pub fn status_retry_for(host: &str, port: u16, limit: Duration) -> Result<StatusInfo, String> {
    let end = Instant::now() + limit;
    loop {
        let e = match status(host, port, Duration::from_secs(5)) { Ok(s) => return Ok(s), Err(e) => e };
        if Instant::now() + Duration::from_millis(1200) >= end { return Err(e); }
        std::thread::sleep(Duration::from_millis(1200));
    }
}

/// Por qué no pudo entrar alguien, en palabras.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JoinIssue {
    pub player: String,
    /// client_rejected | outdated_client | outdated_server | whitelist | banned | auth | mods | other
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub at: String,
    /// Línea original de la consola, para correlacionar con lo que mostró el juego.
    pub log_line: String,
}

#[derive(Default)]
pub struct JoinWatch {
    pending: Vec<(String, Instant)>,
}

static RE_AUTH: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"UUID of player ([A-Za-z0-9_]{1,16}) is").unwrap());
// Anclada al prefijo del registro: el chat («<Bob> Ana lost connection: …») no cuenta.
static RE_LOST: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"^(?:\[[^\[\]]*\] ?)+: (?:System chat: )?([A-Za-z0-9_]{1,16})(?: \([0-9a-f-]{36}\))? lost connection: (.*)$").unwrap());
static RE_DISC: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"Disconnecting (?:([A-Za-z0-9_]{1,16})|.*?name=([A-Za-z0-9_]{1,16}).*?)(?: \(/[^)]*\))?: (.*)$").unwrap());

impl JoinWatch {
    /// Procesa una línea de la consola. Devuelve un problema si se puede explicar una entrada fallida.
    pub fn line(&mut self, line: &str, server_version: &str, server_loader: &str, mods_on_server: usize) -> Option<JoinIssue> {
        let now = Instant::now();
        self.pending.retain(|(_, t)| now.duration_since(*t) < Duration::from_secs(60));
        if let Some(c) = RE_AUTH.captures(line) {
            self.pending.push((c[1].to_string(), now));
            return None;
        }
        if let Some(crate::sessions::Event::Join { name, .. }) = crate::sessions::parse(line) {
            self.pending.retain(|(p, _)| p != &name);
            return None;
        }
        let at = chrono::Local::now().to_rfc3339();
        let reason_issue = |player: &str, reason: &str| -> Option<JoinIssue> {
            let r = reason.to_lowercase();
            let mk = |kind: &str, title: String, detail: String| Some(JoinIssue { player: player.into(), kind: kind.into(), title, detail, at: at.clone(), log_line: line.trim().to_string() });
            if r.contains("outdated client") || r.contains("incompatible client") || r.contains("please use") {
                return mk("outdated_client", format!("{player} usa otra versión de Minecraft"), format!("El servidor es {server_version}. En el launcher, elegí un perfil de Minecraft {server_version}."));
            }
            if r.contains("outdated server") {
                return mk("outdated_server", format!("{player} usa una versión más nueva"), format!("El servidor es {server_version}. Entrá con un perfil {server_version}, o creá el servidor en la versión del jugador."));
            }
            if r.contains("not white-listed") || r.contains("not whitelisted") {
                return mk("whitelist", format!("{player} no está en la lista blanca"), "Agregalo en Jugadores, o desactivá la lista blanca en Ajustes.".into());
            }
            if r.contains("banned") {
                return mk("banned", format!("{player} está bloqueado"), "Quitalo de los bloqueados en Jugadores si querés que entre.".into());
            }
            if r.contains("failed to verify username") || r.contains("invalid session") || r.contains("authentication") {
                return mk("auth", format!("Mojang no pudo verificar la cuenta de {player}"), "Cerrá y abrí el launcher para renovar la sesión. Si el servidor no necesita cuentas verificadas, se puede desactivar en Ajustes → Jugadores y red.".into());
            }
            if r.contains("mod") && (r.contains("missing") || r.contains("mismatch") || r.contains("incompatible") || r.contains("requires")) {
                return mk("mods", format!("Los mods de {player} no coinciden con los del servidor"), reason.trim().to_string());
            }
            None
        };
        if let Some(c) = RE_DISC.captures(line) {
            let player = c.get(1).or(c.get(2)).map(|m| m.as_str()).unwrap_or("Un jugador");
            if let Some(i) = reason_issue(player, &c[3]) { self.pending.retain(|(p, _)| p != player); return Some(i); }
        }
        if let Some(c) = RE_LOST.captures(line) {
            let player = c[1].to_string();
            let reason = c[2].to_string();
            let was_pending = self.pending.iter().position(|(p, _)| *p == player);
            if let Some(i) = reason_issue(&player, &reason) { self.pending.retain(|(p, _)| *p != player); return Some(i); }
            if let Some(i) = was_pending {
                let (_, t) = self.pending.remove(i);
                // Se autenticó pero nunca terminó de entrar y cortó su propio juego.
                if reason.trim() == "Disconnected" && now.duration_since(t) < Duration::from_secs(30) {
                    let detail = if server_loader == "vanilla" || mods_on_server == 0 {
                        format!("El servidor no tiene mods ({}). Si entraste con un perfil con mods (por ejemplo, un modpack), el juego no puede interpretar el mundo y muestra «Error de protocolo de red». Entrá con un perfil de Minecraft {server_version} sin mods, o instalá en el servidor los mismos mods que usa el jugador.", if server_loader == "vanilla" { "Vanilla".to_string() } else { format!("{server_loader} sin mods") })
                    } else {
                        format!("El servidor tiene {mods_on_server} mods. El Minecraft del jugador tiene que tener los mismos mods de contenido, en las mismas versiones, y Minecraft {server_version}. Si el juego mostró «Error de protocolo de red», es casi seguro una diferencia de mods.")
                    };
                    return Some(JoinIssue { player: player.clone(), kind: "client_rejected".into(), title: format!("El juego de {player} cortó la conexión al entrar"), detail, at, log_line: line.trim().to_string() });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    /// Servidor recién encendido que corta las consultas al instante durante `busy` y después
    /// responde como Minecraft.
    fn slow_server(busy: Duration) -> u16 {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t0 = Instant::now();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let mut c = c;
                if t0.elapsed() < busy { drop(c); continue; }
                let mut b = [0u8; 256];
                let _ = c.read(&mut b);
                let json = br#"{"version":{"name":"26.3","protocol":800},"players":{"online":0,"max":4}}"#;
                let mut body = vec![0u8];
                varint(json.len() as u32, &mut body);
                body.extend_from_slice(json);
                let mut pkt = vec![];
                varint(body.len() as u32, &mut pkt);
                pkt.extend(body);
                let _ = c.write_all(&pkt);
            }
        });
        port
    }

    #[test]
    fn la_comprobacion_espera_a_un_servidor_lento_en_atender() {
        // Antes: 8 intentos rápidos (unos 10 s) y «no atiende conexiones» aunque el servidor
        // estuviera bien. Ahora se reintenta por tiempo.
        let port = slow_server(Duration::from_secs(3));
        let t0 = Instant::now();
        let info = status_retry_for("127.0.0.1", port, Duration::from_secs(20)).expect("tenía que responder");
        assert_eq!(info.version, "26.3");
        assert!(t0.elapsed() >= Duration::from_secs(2), "respondió antes de tiempo: la prueba no prueba nada");
        // Si nunca atiende, se rinde al cumplirse el límite, no antes.
        let never = slow_server(Duration::from_secs(3600));
        let t0 = Instant::now();
        assert!(status_retry_for("127.0.0.1", never, Duration::from_secs(4)).is_err());
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(2500) && took < Duration::from_secs(8), "{took:?}");
    }

    use super::*;
    #[test]
    fn explains_client_side_rejection_from_real_log() {
        // Formato de un log real de Fabric 26.2 sin mods con un cliente con modpack (nombre y UUID ficticios).
        let mut w = JoinWatch::default();
        assert!(w.line("[20:55:35] [User Authenticator #1/INFO]: UUID of player Jugadora1 is 00000000-0000-4000-8000-000000000939", "26.2", "fabric", 0).is_none());
        let i = w.line("[20:55:37] [Server thread/INFO]: Jugadora1 (00000000-0000-4000-8000-000000000939) lost connection: Disconnected", "26.2", "fabric", 0).expect("debería explicar");
        assert_eq!(i.kind, "client_rejected");
        assert!(i.detail.contains("Error de protocolo de red"));
    }
    #[test]
    fn normal_leave_is_not_an_issue() {
        let mut w = JoinWatch::default();
        w.line("[x] [User Authenticator #1/INFO]: UUID of player Ana is 00000000-0000-4000-8000-000000000939", "26.2", "vanilla", 0);
        w.line("[x] [Server thread/INFO]: Ana joined the game", "26.2", "vanilla", 0);
        assert!(w.line("[x] [Server thread/INFO]: Ana lost connection: Disconnected", "26.2", "vanilla", 0).is_none());
    }
    #[test]
    fn version_and_whitelist_reasons() {
        let mut w = JoinWatch::default();
        let i = w.line("[x] [Server thread/INFO]: Disconnecting com.mojang.authlib.GameProfile@1[id=<null>,name=Leo,properties={}] (/127.0.0.1:5): Outdated client! Please use 26.2", "26.2", "vanilla", 0).unwrap();
        assert_eq!(i.kind, "outdated_client");
        let i = w.line("[x] [Server thread/INFO]: Leo (00000000-0000-4000-8000-000000000939) lost connection: You are not white-listed on this server!", "26.2", "vanilla", 0).unwrap();
        assert_eq!(i.kind, "whitelist");
    }
    #[test]
    fn status_against_local_listener_without_minecraft_fails_clearly() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || { let _ = l.accept().map(|(s, _)| drop(s)); });
        let e = status("127.0.0.1", port, Duration::from_secs(2)).unwrap_err();
        assert!(e.contains("Minecraft") || e.contains("cortó"), "{e}");
    }
}
