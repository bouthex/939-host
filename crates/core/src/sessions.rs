//! Entradas y salidas de jugadores, leídas de la consola del servidor, y su historial.
//!
//! - La detección exige que la línea sea **solo** el prefijo del registro de Minecraft seguido de
//!   «<nombre> joined the game». Un jugador que escribe en el chat «Ana joined the game» no
//!   aparece como conectado: su línea lleva «<Jugador>» antes del texto.
//! - Cambio de nombre: Minecraft avisa «Nuevo (formerly known as Viejo) joined the game». El
//!   historial guarda los dos nombres para que el jugador no aparezca duplicado.
//! - Historial: un renglón JSON por sesión terminada en `939host-sesiones.jsonl`, dentro de la
//!   carpeta del servidor (viaja con el mundo cuando cambia la PC anfitriona). Solo datos que
//!   vio la consola: nada estimado. Se recorta a las últimas `KEEP` sesiones.

use crate::paths;
use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

const FILE: &str = "939host-sesiones.jsonl";
const KEEP: usize = 2000;

// Prefijo: uno o más bloques «[...]» al comienzo de la línea (vanilla, Paper, Forge, NeoForge,
// Fabric), y después «: ». El chat siempre trae «<Nombre>» o «[Nombre]:» después de eso.
static RE_JOIN: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(?:\[[^\[\]]*\] ?)+: (?:System chat: )?([A-Za-z0-9_]{1,16})(?: \(formerly known as ([A-Za-z0-9_]{1,16})\))? joined the game\s*$").unwrap());
static RE_LEAVE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(?:\[[^\[\]]*\] ?)+: (?:System chat: )?([A-Za-z0-9_]{1,16}) left the game\s*$").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event { Join { name: String, previously: Option<String> }, Leave { name: String } }

pub fn parse(line: &str) -> Option<Event> {
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(c) = RE_JOIN.captures(line) {
        return Some(Event::Join { name: c[1].to_string(), previously: c.get(2).map(|m| m.as_str().to_string()) });
    }
    RE_LEAVE.captures(line).map(|c| Event::Leave { name: c[1].to_string() })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// Nombre con el que jugó.
    pub n: String,
    /// Nombre anterior, si Minecraft avisó un cambio en esta entrada.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<String>,
    /// Entrada y salida (RFC 3339).
    pub j: String,
    pub l: String,
    /// La salida no se vio en la consola: el servidor se cerró o se cortó con el jugador adentro.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

fn path(id: &str) -> PathBuf { paths::server_run_dir(id).join(FILE) }

/// Guarda una sesión terminada. Nunca falla hacia afuera: el historial no puede frenar al servidor.
pub fn record(id: &str, s: &Session) {
    let p = path(id);
    let Ok(line) = serde_json::to_string(s) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{line}");
    }
    // Recorte ocasional (por tamaño, sin leer el archivo en cada sesión).
    if std::fs::metadata(&p).map(|m| m.len() > (KEEP as u64) * 200).unwrap_or(false) {
        let all = load(id);
        if all.len() > KEEP {
            let text: String = all[all.len() - KEEP..].iter().filter_map(|s| serde_json::to_string(s).ok()).map(|l| l + "\n").collect();
            let _ = paths::write_atomic(&p, text.as_bytes());
        }
    }
}

pub fn load(id: &str) -> Vec<Session> {
    std::fs::read_to_string(path(id)).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Sesiones abiertas de un servidor en marcha (nombre → hora de entrada).
#[derive(Debug, Default, Clone)]
pub struct Open(HashMap<String, (DateTime<Local>, Option<String>)>);

impl Open {
    pub fn join(&mut self, name: &str, previously: Option<String>, at: DateTime<Local>) { self.0.insert(name.to_string(), (at, previously)); }
    pub fn leave(&mut self, id: &str, name: &str, at: DateTime<Local>) {
        if let Some((j, p)) = self.0.remove(name) { record(id, &Session { n: name.into(), p, j: j.to_rfc3339(), l: at.to_rfc3339(), cut: false }); }
    }
    /// El servidor terminó: cierra las sesiones abiertas marcándolas como cortadas.
    pub fn close_all(&mut self, id: &str, at: DateTime<Local>) {
        for (n, (j, p)) in self.0.drain() { record(id, &Session { n, p, j: j.to_rfc3339(), l: at.to_rfc3339(), cut: true }); }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub sessions: u64,
    pub total_secs: i64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
    /// Otros nombres con los que entró (según los avisos de Minecraft).
    pub other_names: Vec<String>,
    /// Últimas sesiones, la más nueva primero (máximo 10).
    pub recent: Vec<RecentSession>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecentSession { pub join: String, pub leave: String, pub secs: i64, pub cut: bool }

/// Todos los nombres que pertenecen a la misma persona, siguiendo los avisos de cambio de nombre.
pub fn aliases(all: &[Session], name: &str) -> Vec<String> {
    let mut names = vec![name.to_lowercase()];
    let mut i = 0;
    while i < names.len() {
        let cur = names[i].clone();
        for s in all {
            if let Some(p) = &s.p {
                let (a, b) = (s.n.to_lowercase(), p.to_lowercase());
                if a == cur && !names.contains(&b) { names.push(b); } else if b == cur && !names.contains(&a) { names.push(a); }
            }
        }
        i += 1;
    }
    names
}

pub fn history_of(all: &[Session], name: &str) -> History {
    let names = aliases(all, name);
    let mine: Vec<&Session> = all.iter().filter(|s| names.contains(&s.n.to_lowercase())).collect();
    let mut h = History { sessions: mine.len() as u64, ..Default::default() };
    for s in &mine {
        let (Ok(j), Ok(l)) = (DateTime::parse_from_rfc3339(&s.j), DateTime::parse_from_rfc3339(&s.l)) else { continue };
        let secs = (l - j).num_seconds().max(0);
        h.total_secs += secs;
        if h.first_seen.as_ref().map(|f| s.j < *f).unwrap_or(true) { h.first_seen = Some(s.j.clone()); }
        if h.last_seen.as_ref().map(|f| s.l > *f).unwrap_or(true) { h.last_seen = Some(s.l.clone()); }
    }
    let mut other: Vec<String> = mine.iter().map(|s| s.n.clone()).chain(mine.iter().filter_map(|s| s.p.clone())).filter(|n| !n.eq_ignore_ascii_case(name)).collect();
    other.sort_by_key(|n| n.to_lowercase());
    other.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    h.other_names = other;
    h.recent = mine.iter().rev().take(10).filter_map(|s| {
        let secs = (DateTime::parse_from_rfc3339(&s.l).ok()? - DateTime::parse_from_rfc3339(&s.j).ok()?).num_seconds().max(0);
        Some(RecentSession { join: s.j.clone(), leave: s.l.clone(), secs, cut: s.cut })
    }).collect();
    h
}

pub fn history(id: &str, name: &str) -> History { history_of(&load(id), name) }

/// Última salida registrada por nombre (en minúsculas), con los alias resueltos al nombre dado.
pub fn last_seen_map(id: &str) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    for s in load(id) {
        for n in std::iter::once(&s.n).chain(s.p.iter()) {
            let e = m.entry(n.to_lowercase()).or_default();
            if s.l > *e { *e = s.l.clone(); }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_entradas_reales_y_no_las_del_chat() {
        let j = |n: &str| Some(Event::Join { name: n.into(), previously: None });
        assert_eq!(parse("[12:00:01] [Server thread/INFO]: Ana joined the game"), j("Ana"));
        assert_eq!(parse("[12:00:01 INFO]: Ana joined the game"), j("Ana"), "Paper");
        assert_eq!(parse("[12:00:01] [Server thread/INFO] [minecraft/MinecraftServer]: Ana_2 joined the game"), j("Ana_2"), "Forge/NeoForge");
        assert_eq!(parse("[12:00:01] [Server thread/INFO]: Ana left the game\r\n"), Some(Event::Leave { name: "Ana".into() }));
        // Minecraft 26.3: los avisos del sistema llegan con «System chat: » (visto con un cliente
        // real; sin esto 939 Host mostraba 0 jugadores con gente conectada).
        assert_eq!(parse("[22:06:37] [Server thread/INFO]: System chat: Jugador939 joined the game"), j("Jugador939"));
        assert_eq!(parse("[22:06:37] [Server thread/INFO]: System chat: Jugador939 left the game"), Some(Event::Leave { name: "Jugador939".into() }));
        assert_eq!(parse("[22:06:37] [Server thread/INFO]: <Bob> System chat: Ana joined the game"), None, "el chat de un jugador no cuenta");
        assert_eq!(parse("[12:00:01] [Server thread/INFO]: Nuevo (formerly known as Viejo) joined the game"), Some(Event::Join { name: "Nuevo".into(), previously: Some("Viejo".into()) }));
        // Chat: no cuenta, escriba lo que escriba.
        for chat in [
            "[12:00:01] [Server thread/INFO]: <Bob> Ana joined the game",
            "[12:00:01] [Server thread/INFO]: <Bob> x: Ana joined the game",
            "[12:00:01] [Server thread/INFO]: <Bob> [a]: Ana joined the game",
            "[12:00:01] [Server thread/INFO]: [Bob] Ana left the game",
            "[12:00:01] [Server thread/INFO]: [Not Secure] <Bob> Ana joined the game",
            "[12:00:01] [Server thread/INFO]: * Bob Ana joined the game",
            "Ana joined the game",
        ] { assert_eq!(parse(chat), None, "{chat}"); }
    }

    #[test]
    fn historial_une_cambios_de_nombre_y_suma_solo_lo_visto() {
        let s = |n: &str, p: Option<&str>, j: &str, l: &str| Session { n: n.into(), p: p.map(String::from), j: j.into(), l: l.into(), cut: false };
        let all = vec![
            s("Viejo", None, "2026-11-01T10:00:00-03:00", "2026-11-01T11:00:00-03:00"),
            s("Otra", None, "2026-11-02T10:00:00-03:00", "2026-11-02T10:30:00-03:00"),
            s("Nuevo", Some("Viejo"), "2026-11-03T10:00:00-03:00", "2026-11-03T10:15:00-03:00"),
        ];
        let h = history_of(&all, "Nuevo");
        assert_eq!(h.sessions, 2);
        assert_eq!(h.total_secs, 3600 + 900);
        assert_eq!(h.first_seen.as_deref(), Some("2026-11-01T10:00:00-03:00"));
        assert_eq!(h.last_seen.as_deref(), Some("2026-11-03T10:15:00-03:00"));
        assert_eq!(h.other_names, vec!["Viejo".to_string()]);
        assert_eq!(h.recent[0].secs, 900, "la más nueva primero");
        assert_eq!(history_of(&all, "Viejo").sessions, 2, "también desde el nombre viejo");
        assert_eq!(history_of(&all, "Nadie"), History::default());
    }

    #[test]
    fn graba_y_cierra_sesiones_abiertas() {
        // Carpeta que no se borra al terminar: la raíz es una sola para todas las pruebas.
        let _ = paths::init(std::env::temp_dir().join(format!("h939-ses-{}", std::process::id())));
        let id = format!("sesiones-{}", std::process::id());
        std::fs::create_dir_all(paths::server_run_dir(&id)).unwrap();
        let mut o = Open::default();
        let t0 = Local::now();
        o.join("Ana", None, t0);
        o.join("Bob", None, t0);
        o.leave(&id, "Ana", t0 + chrono::Duration::seconds(30));
        o.leave(&id, "Nadie", t0);
        o.close_all(&id, t0 + chrono::Duration::seconds(60));
        let all = load(&id);
        assert_eq!(all.len(), 2);
        assert!(!all[0].cut && all[1].cut, "la de Bob quedó cortada por el cierre");
        assert_eq!(history(&id, "Ana").total_secs, 30);
        assert_eq!(last_seen_map(&id).len(), 2);
    }
}
