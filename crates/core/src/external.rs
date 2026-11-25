//! Servidores de Minecraft encendidos fuera de 939 Host.
//!
//! Dos casos distintos:
//! - **Un servidor de 939 Host encendido a mano** (por ejemplo, con `run.bat` en su carpeta):
//!   `started_outside` lo detecta para no mostrarlo «Detenido» ni encender una segunda copia
//!   sobre el mismo mundo. Hace falta una prueba de que es *ese* servidor, no solo un puerto
//!   abierto: el puerto responde como Minecraft **y** el mundo de esa carpeta está en uso por otro
//!   proceso (Windows: `session.lock` bloqueado; Linux: el proceso que escucha trabaja en esa carpeta).
//! - **Servidores vinculados**: cualquier servidor (otra carpeta, otro programa, otra PC de la red)
//!   que la persona agrega con su dirección y puerto. 939 Host no lo administra (no tiene su
//!   consola), pero muestra si responde de verdad como Minecraft, su versión y jugadores, y si la
//!   dirección es de esta PC, de la red local o de Internet.
//!
//! Nunca se marca «encendido» solo porque un puerto acepta conexiones: hace falta que responda al
//! protocolo de estado de Minecraft.

use crate::error::{AppError, AppResult};
use crate::model::ServerConfig;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutsideRun {
    pub pid: Option<u32>,
    /// Ejecutable del proceso («java.exe»), si se pudo saber.
    pub program: String,
    pub version: String,
    pub online: i64,
    pub max: i64,
}

static CACHE: once_cell::sync::Lazy<Mutex<HashMap<String, (Instant, Option<OutsideRun>)>>> = once_cell::sync::Lazy::new(Default::default);

/// ¿Este servidor de 939 Host está encendido fuera de 939 Host? (Llamar solo si 939 Host no lo
/// está ejecutando.) El resultado se guarda 5 s: la interfaz pregunta seguido.
pub fn started_outside(cfg: &ServerConfig) -> Option<OutsideRun> {
    if let Some((t, v)) = CACHE.lock().unwrap().get(&cfg.id) { if t.elapsed() < Duration::from_secs(5) { return v.clone(); } }
    let v = detect(cfg);
    CACHE.lock().unwrap().insert(cfg.id.clone(), (Instant::now(), v.clone()));
    v
}

/// Olvida lo guardado (al encender o detener desde 939 Host).
pub fn forget(id: &str) { CACHE.lock().unwrap().remove(id); }

fn detect(cfg: &ServerConfig) -> Option<OutsideRun> {
    if crate::ports::is_free(cfg.port) { return None; }
    let run = paths::server_run_dir(&cfg.id);
    let user = crate::ports::user_of(cfg.port);
    if !folder_in_use(&run, user.as_ref().map(|u| u.pid)) { return None; }
    let st = crate::reach::status("127.0.0.1", cfg.port, Duration::from_secs(2)).ok()?;
    Some(OutsideRun { pid: user.as_ref().map(|u| u.pid), program: user.map(|u| u.name).unwrap_or_default(), version: st.version, online: st.online, max: st.max })
}

/// La carpeta del servidor la está usando otro proceso.
fn folder_in_use(run: &std::path::Path, pid: Option<u32>) -> bool {
    #[cfg(target_os = "linux")]
    if let Some(pid) = pid {
        if let (Ok(cwd), Ok(dir)) = (std::fs::read_link(format!("/proc/{pid}/cwd")), run.canonicalize()) { return cwd == dir; }
    }
    let _ = pid;
    let lock = run.join("world").join("session.lock");
    if !lock.exists() { return false; }
    // Minecraft bloquea session.lock mientras el mundo está abierto: abrirlo para escribir falla
    // con «violación de uso compartido» (Windows) aunque el archivo exista.
    match std::fs::OpenOptions::new().write(true).open(&lock) {
        Err(e) => matches!(e.raw_os_error(), Some(32) | Some(33)),
        Ok(_) => false,
    }
}

/* ───────── Servidores vinculados ───────── */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Linked { pub id: String, pub name: String, pub host: String, pub port: u16, pub added_at: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedView {
    #[serde(flatten)]
    pub link: Linked,
    /// "minecraft": responde como Minecraft · "abierto": acepta conexiones pero no responde como
    /// Minecraft · "cerrado": no acepta conexiones · "sin-resolver": el nombre no existe.
    pub state: String,
    pub detail: String,
    pub version: Option<String>,
    pub online: Option<i64>,
    pub max: Option<i64>,
    pub latency_ms: Option<u64>,
    /// "esta-pc" | "red-local" | "internet" (según la dirección, no según una prueba desde afuera).
    pub scope: String,
    /// Si es un servidor de 939 Host de esta PC (mismo puerto en esta PC), su nombre.
    pub same_as: Option<String>,
    /// Proceso que escucha en ese puerto (solo si es de esta PC).
    pub program: Option<String>,
}

fn store() -> std::path::PathBuf { paths::root().join("servidores-vinculados.json") }
pub fn list() -> Vec<Linked> { paths::read_json(&store()).unwrap_or_default() }

pub fn add(name: &str, host: &str, port: u16) -> AppResult<Linked> {
    let host = host.trim().trim_start_matches("minecraft://").trim_end_matches('/').to_string();
    let name = name.trim().chars().take(48).collect::<String>();
    if host.is_empty() || host.len() > 253 || host.chars().any(|c| c.is_whitespace() || "/\\@?#".contains(c)) { return Err(AppError::invalid("Escribí una dirección válida (por ejemplo 127.0.0.1, 192.168.1.20 o mc.ejemplo.com), sin el puerto.")); }
    if port == 0 { return Err(AppError::invalid("Elegí un puerto entre 1 y 65535.")); }
    let mut all = list();
    if all.iter().any(|l| l.host.eq_ignore_ascii_case(&host) && l.port == port) { return Err(AppError::invalid("Ese servidor ya está vinculado.")); }
    if all.len() >= 50 { return Err(AppError::invalid("Llegaste al máximo de servidores vinculados.")); }
    let mut b = [0u8; 6];
    getrandom::getrandom(&mut b).map_err(|e| AppError::new("internal", "Sin generador aleatorio").with_detail(e))?;
    let l = Linked { id: hex::encode(b), name: if name.is_empty() { format!("{host}:{port}") } else { name }, host, port, added_at: chrono::Local::now().to_rfc3339() };
    all.push(l.clone());
    paths::write_json(&store(), &all)?;
    Ok(l)
}

pub fn remove(id: &str) -> AppResult<()> {
    let mut all = list();
    all.retain(|l| l.id != id);
    paths::write_json(&store(), &all)
}

/// Clasifica una IP: esta PC, red local (incluye CGNAT 100.64/10, que tampoco es Internet) o Internet.
pub fn scope_of(ip: IpAddr) -> &'static str {
    match ip {
        IpAddr::V4(v) if v.is_loopback() || v.is_unspecified() => "esta-pc",
        IpAddr::V4(v) if v.is_private() || v.is_link_local() || (v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64) => "red-local",
        IpAddr::V6(v) if v.is_loopback() || v.is_unspecified() => "esta-pc",
        IpAddr::V6(v) if (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80 => "red-local",
        _ => "internet",
    }
}

/// Estado real de un servidor vinculado. `own` = servidores de 939 Host de esta PC (id, nombre, puerto).
pub fn probe(l: &Linked, own: &[(String, String, u16)]) -> LinkedView {
    let mut v = LinkedView { link: l.clone(), state: "sin-resolver".into(), detail: String::new(), version: None, online: None, max: None, latency_ms: None, scope: "internet".into(), same_as: None, program: None };
    let addr = match (l.host.as_str(), l.port).to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(a) => a,
        None => { v.detail = format!("No se encontró «{}». Revisá que esté bien escrito.", l.host); return v; }
    };
    v.scope = scope_of(addr.ip()).into();
    if v.scope == "esta-pc" {
        v.same_as = own.iter().find(|(_, _, p)| *p == l.port).map(|(_, n, _)| n.clone());
        v.program = crate::ports::user_of(l.port).map(|u| if u.name.is_empty() { format!("proceso {}", u.pid) } else { format!("{} (proceso {})", u.name, u.pid) });
    }
    match crate::reach::status(&l.host, l.port, Duration::from_secs(3)) {
        Ok(s) => {
            v.state = "minecraft".into();
            v.detail = format!("Minecraft {} responde en {} ms.", s.version, s.latency_ms);
            v.version = Some(s.version); v.online = Some(s.online); v.max = Some(s.max); v.latency_ms = Some(s.latency_ms);
        }
        Err(_) if TcpStream::connect_timeout(&addr, Duration::from_secs(3)).is_ok() => {
            v.state = "abierto".into();
            v.detail = "El puerto acepta conexiones, pero no responde como un servidor de Minecraft (puede estar arrancando o ser otro programa).".into();
        }
        Err(_) => {
            v.state = "cerrado".into();
            v.detail = match v.scope.as_str() {
                "esta-pc" => "No hay nada escuchando en ese puerto de esta PC: el servidor está apagado o usa otro puerto.".into(),
                "red-local" => "No responde desde esta PC: está apagado, usa otro puerto o el firewall de esa PC lo bloquea.".into(),
                _ => "No responde desde esta PC: puede estar apagado, el router no reenvía el puerto o el proveedor usa CGNAT.".into(),
            };
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alcance_de_cada_direccion() {
        assert_eq!(scope_of("127.0.0.1".parse().unwrap()), "esta-pc");
        assert_eq!(scope_of("192.168.1.20".parse().unwrap()), "red-local");
        assert_eq!(scope_of("10.0.0.5".parse().unwrap()), "red-local");
        assert_eq!(scope_of("100.70.1.4".parse().unwrap()), "red-local", "CGNAT no es Internet");
        assert_eq!(scope_of("203.0.113.25".parse().unwrap()), "internet");
        assert_eq!(scope_of("fd00::1".parse().unwrap()), "red-local");
        assert_eq!(scope_of("2001:db8::1".parse().unwrap()), "internet");
    }

    #[test]
    fn un_puerto_abierto_que_no_es_minecraft_no_cuenta_como_encendido() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        // Acepta y cierra sin responder.
        std::thread::spawn(move || { for s in l.incoming().flatten() { drop(s); } });
        let v = probe(&Linked { id: "x".into(), name: "x".into(), host: "127.0.0.1".into(), port, added_at: String::new() }, &[]);
        assert_eq!(v.state, "abierto");
        assert_eq!(v.scope, "esta-pc");
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let v = probe(&Linked { id: "y".into(), name: "y".into(), host: "127.0.0.1".into(), port: free, added_at: String::new() }, &[]);
        assert_eq!(v.state, "cerrado");
    }
}
