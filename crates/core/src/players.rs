//! Jugadores y permisos: lista blanca, operadores y baneos.
//!
//! Con el servidor encendido se usan los comandos de Minecraft (el servidor mantiene sus archivos
//! y resuelve los UUID). Apagado, se editan los archivos JSON con el mismo formato que usa
//! Minecraft; el UUID se obtiene de Mojang (modo online) o se calcula (modo offline, igual que el
//! servidor: UUID v3 de "OfflinePlayer:<nombre>").

use crate::error::{AppError, AppResult};
use crate::model::ServerConfig;
use crate::net;
use crate::paths;
use crate::process::Manager;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerView {
    pub name: String,
    pub uuid: String,
    pub online: bool,
    pub whitelisted: bool,
    pub op_level: Option<u8>,
    pub banned: bool,
    pub ban_reason: Option<String>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerAction { WhitelistAdd, WhitelistRemove, Op, Deop, Ban, Pardon, Kick }

/// Si `cmd` es un `op <nombre>` o `deop <nombre>` de consola (con o sin `/`), devuelve
/// (dio OP, nombre).
pub fn op_command(cmd: &str) -> Option<(bool, String)> {
    let mut it = cmd.trim().trim_start_matches('/').split_whitespace();
    let granted = match it.next()?.to_ascii_lowercase().as_str() { "op" => true, "deop" => false, _ => return None };
    let name = it.next()?;
    (it.next().is_none() && valid_name(name)).then(|| (granted, name.to_string()))
}

fn file(cfg: &ServerConfig, name: &str) -> PathBuf { paths::server_run_dir(&cfg.id).join(name) }

fn read_list(cfg: &ServerConfig, name: &str) -> Vec<Value> {
    std::fs::read_to_string(file(cfg, name)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_list(cfg: &ServerConfig, name: &str, list: &[Value]) -> AppResult<()> {
    let text = serde_json::to_string_pretty(list)?;
    paths::write_atomic(&file(cfg, name), text.as_bytes())
}

pub fn valid_name(name: &str) -> bool { (3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') }

/// UUID de jugador sin cuenta (modo offline), idéntico al que calcula Minecraft.
pub fn offline_uuid(name: &str) -> String {
    use md5::Digest;
    let mut h: [u8; 16] = md5::Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    let x = hex::encode(h);
    format!("{}-{}-{}-{}-{}", &x[0..8], &x[8..12], &x[12..16], &x[16..20], &x[20..32])
}

fn dashed(raw: &str) -> String {
    if raw.len() == 32 { format!("{}-{}-{}-{}-{}", &raw[0..8], &raw[8..12], &raw[12..16], &raw[16..20], &raw[20..32]) } else { raw.to_string() }
}

/// Busca el UUID y el nombre exacto. Primero en los archivos del servidor, después en Mojang.
async fn resolve(cfg: &ServerConfig, name: &str) -> AppResult<(String, String)> {
    for f in ["usercache.json", "whitelist.json", "ops.json", "banned-players.json"] {
        for e in read_list(cfg, f) {
            if e["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false) {
                if let Some(u) = e["uuid"].as_str() { return Ok((u.to_string(), e["name"].as_str().unwrap_or(name).to_string())); }
            }
        }
    }
    if !cfg.online_mode { return Ok((offline_uuid(name), name.to_string())); }
    #[derive(Deserialize)]
    struct Profile { id: String, name: String }
    let p: Profile = net::get_json(&format!("https://api.mojang.com/users/profiles/minecraft/{name}"), "Mojang")
        .await
        .map_err(|_| AppError::new("not_found", format!("No existe una cuenta de Minecraft llamada «{name}» (o Mojang no respondió).")))?;
    Ok((dashed(&p.id), p.name))
}

pub fn list(mgr: &Manager, cfg: &ServerConfig) -> Vec<PlayerView> {
    let mut map: BTreeMap<String, PlayerView> = BTreeMap::new();
    let key = |n: &str| n.to_ascii_lowercase();
    let mut touch = |name: &str, uuid: &str| -> String {
        let k = key(name);
        let e = map.entry(k.clone()).or_insert_with(|| PlayerView { name: name.into(), ..Default::default() });
        if e.uuid.is_empty() { e.uuid = uuid.into(); }
        k
    };
    let mut seen = vec![];
    for e in read_list(cfg, "usercache.json") {
        if let (Some(n), Some(u)) = (e["name"].as_str(), e["uuid"].as_str()) {
            let k = touch(n, u);
            seen.push((k, e["expiresOn"].as_str().map(String::from)));
        }
    }
    let mut wl = vec![];
    for e in read_list(cfg, "whitelist.json") { if let (Some(n), Some(u)) = (e["name"].as_str(), e["uuid"].as_str()) { wl.push(touch(n, u)); } }
    let mut ops = vec![];
    for e in read_list(cfg, "ops.json") { if let (Some(n), Some(u)) = (e["name"].as_str(), e["uuid"].as_str()) { ops.push((touch(n, u), e["level"].as_u64().unwrap_or(4) as u8)); } }
    let mut bans = vec![];
    for e in read_list(cfg, "banned-players.json") { if let (Some(n), Some(u)) = (e["name"].as_str(), e["uuid"].as_str()) { bans.push((touch(n, u), e["reason"].as_str().map(String::from))); } }
    let online: Vec<String> = mgr.runtime(&cfg.id).lock().unwrap().players.iter().cloned().collect();
    for n in &online { touch(n, ""); }
    for (k, exp) in seen { if let Some(p) = map.get_mut(&k) { p.last_seen = exp; } }
    for k in wl { if let Some(p) = map.get_mut(&k) { p.whitelisted = true; } }
    for (k, l) in ops { if let Some(p) = map.get_mut(&k) { p.op_level = Some(l); } }
    for (k, r) in bans { if let Some(p) = map.get_mut(&k) { p.banned = true; p.ban_reason = r; } }
    for n in &online { if let Some(p) = map.get_mut(&key(n)) { p.online = true; } }
    let mut v: Vec<PlayerView> = map.into_values().collect();
    v.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// Aplica una acción. Devuelve un texto para mostrar.
pub async fn apply(mgr: &Manager, cfg: &ServerConfig, name: &str, action: PlayerAction, reason: Option<&str>) -> AppResult<String> {
    let name = name.trim();
    if !valid_name(name) { return Err(AppError::invalid("Nombre de jugador inválido (3 a 16 letras, números o _).")); }
    let reason = reason.map(|r| r.replace(['\n', '\r'], " ").chars().take(120).collect::<String>()).filter(|r| !r.trim().is_empty());
    let running = mgr.runtime(&cfg.id).lock().unwrap().status() == crate::model::RunStatus::Running;
    let whitelist_action = matches!(action, PlayerAction::WhitelistAdd | PlayerAction::WhitelistRemove);
    // La lista blanca se edita siempre en el archivo (con el UUID exacto) y, si está encendido,
    // se recarga: en modo offline los comandos de Minecraft buscan el nombre en minúsculas y
    // pueden no coincidir con el jugador real.
    if running && !whitelist_action {
        let cmd = match action {
            PlayerAction::WhitelistAdd | PlayerAction::WhitelistRemove => unreachable!(),
            PlayerAction::Op => format!("op {name}"),
            PlayerAction::Deop => format!("deop {name}"),
            PlayerAction::Ban => format!("ban {name}{}", reason.as_ref().map(|r| format!(" {r}")).unwrap_or_default()),
            PlayerAction::Pardon => format!("pardon {name}"),
            PlayerAction::Kick => format!("kick {name}{}", reason.as_ref().map(|r| format!(" {r}")).unwrap_or_default()),
        };
        let before = mgr.console(&cfg.id).len();
        mgr.send_command(&cfg.id, &cmd)?;
        // Respuesta del servidor (Minecraft no confirma por otro medio).
        for _ in 0..20 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let lines = mgr.console(&cfg.id);
            let fresh: Vec<&String> = lines.iter().skip(before.min(lines.len())).filter(|l| !l.contains(&format!("/{cmd}")) && !l.starts_with("> ")).collect();
            if let Some(reply) = fresh.iter().rev().find(|l| l.contains("[Server thread/INFO]") || l.contains("INFO]")) {
                let text = reply.rsplit("]: ").next().unwrap_or(reply).trim().to_string();
                let low = text.to_lowercase();
                if low.contains("does not exist") || low.contains("no player was found") || low.contains("unknown") || low.contains("incorrect argument") {
                    return Err(AppError::invalid(format!("El servidor no lo aceptó: «{text}». Si ese jugador nunca entró a este servidor, Minecraft no lo conoce todavía.")));
                }
                return Ok(text);
            }
        }
        return Ok(format!("Enviado al servidor: /{cmd}"));
    }
    if action == PlayerAction::Kick { return Err(AppError::invalid("El servidor está apagado: no hay a quién expulsar.")); }
    let (uuid, exact) = resolve(cfg, name).await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z").to_string();
    let same = |e: &Value| e["uuid"].as_str() == Some(uuid.as_str()) || e["name"].as_str().map(|n| n.eq_ignore_ascii_case(&exact)).unwrap_or(false);
    let edit = |fname: &str, add: Option<Value>| -> AppResult<()> {
        let mut l = read_list(cfg, fname);
        l.retain(|e| !same(e));
        if let Some(v) = add { l.push(v); }
        write_list(cfg, fname, &l)
    };
    match action {
        PlayerAction::WhitelistAdd => edit("whitelist.json", Some(json!({ "uuid": uuid, "name": exact })))?,
        PlayerAction::WhitelistRemove => edit("whitelist.json", None)?,
        PlayerAction::Op => edit("ops.json", Some(json!({ "uuid": uuid, "name": exact, "level": 4, "bypassesPlayerLimit": false })))?,
        PlayerAction::Deop => edit("ops.json", None)?,
        PlayerAction::Ban => edit("banned-players.json", Some(json!({ "uuid": uuid, "name": exact, "created": now, "source": "939 Host", "expires": "forever", "reason": reason.unwrap_or_else(|| "Baneado por un administrador".into()) })))?,
        PlayerAction::Pardon => edit("banned-players.json", None)?,
        PlayerAction::Kick => unreachable!(),
    }
    if running {
        mgr.send_command(&cfg.id, "whitelist reload")?;
        return Ok(format!("Lista blanca actualizada para {exact}."));
    }
    Ok(format!("Guardado para {exact}. Se aplica al encender."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_minecraft() {
        // Valor conocido: UUID offline de "Notch".
        assert_eq!(offline_uuid("Notch"), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert!(valid_name("Alex_939"));
        assert!(!valid_name("a b"));
        assert!(!valid_name("ab"));
        assert!(!valid_name("x; stop"));
    }
}

#[cfg(test)]
mod op_command_tests {
    #[test]
    fn reconoce_op_de_consola() {
        assert_eq!(super::op_command("op Archi"), Some((true, "Archi".into())));
        assert_eq!(super::op_command(" /DEOP kaki_1 "), Some((false, "kaki_1".into())));
        assert_eq!(super::op_command("op @a"), None);
        assert_eq!(super::op_command("op a b"), None);
        assert_eq!(super::op_command("opx Archi"), None);
    }
}
