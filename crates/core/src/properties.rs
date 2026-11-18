//! Lectura y escritura de server.properties conservando las claves que no administramos.

use crate::error::AppResult;
use crate::model::ServerConfig;
use crate::paths;
use std::collections::BTreeMap;
use std::path::Path;

/// Claves que la interfaz administra; el resto se conserva tal cual.
pub const MANAGED: &[&str] = &[
    "server-port", "max-players", "motd", "online-mode", "white-list", "enforce-whitelist",
    "difficulty", "gamemode", "hardcore", "pvp", "level-seed", "level-type", "level-name",
    "view-distance", "simulation-distance", "spawn-protection", "allow-flight", "enable-command-block",
    "server-ip",
];

pub fn read(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') { continue; }
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), unescape(v));
            }
        }
    }
    map
}

fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) { out.push(ch); }
                }
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Escapa como lo espera java.util.Properties: lo que no es ASCII va como \uXXXX.
fn escape(v: &str) -> String {
    let mut out = String::new();
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {}
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
        }
    }
    out
}

/// Genera server.properties a partir de la configuración del servidor.
pub fn write_for(cfg: &ServerConfig) -> AppResult<()> {
    let path = paths::server_run_dir(&cfg.id).join("server.properties");
    let mut map = read(&path);
    for (k, v) in &cfg.extra_properties {
        if !MANAGED.contains(&k.as_str()) { map.insert(k.clone(), v.clone()); }
    }
    let b = |x: bool| if x { "true".to_string() } else { "false".to_string() };
    let managed: Vec<(&str, String)> = vec![
        ("server-port", cfg.port.to_string()),
        ("max-players", cfg.max_players.to_string()),
        ("motd", cfg.motd.clone()),
        ("online-mode", b(cfg.online_mode)),
        ("white-list", b(cfg.whitelist)),
        ("enforce-whitelist", b(cfg.whitelist)),
        ("difficulty", cfg.difficulty.clone()),
        ("gamemode", cfg.gamemode.clone()),
        ("hardcore", b(cfg.hardcore)),
        ("pvp", b(cfg.pvp)),
        ("level-seed", cfg.level_seed.clone()),
        ("level-type", cfg.level_type.clone()),
        ("level-name", "world".to_string()),
        ("view-distance", cfg.view_distance.to_string()),
        ("simulation-distance", cfg.simulation_distance.to_string()),
        ("spawn-protection", cfg.spawn_protection.to_string()),
        ("allow-flight", b(cfg.allow_flight)),
        ("enable-command-block", b(cfg.enable_command_block)),
        // Por defecto el servidor escucha solo en esta PC: se juega a través de 939 Host (que
        // autentica a cada PC del grupo). «Permitir conexiones de mi red local» lo abre a la LAN.
        ("server-ip", if crate::settings::load().lan_access { String::new() } else { "127.0.0.1".to_string() }),
    ];
    for (k, v) in managed { map.insert(k.to_string(), v); }

    let mut text = String::from("# Generado por 939 Host. Los cambios hechos desde la app reemplazan las claves administradas.\n");
    for (k, v) in &map {
        text.push_str(k);
        text.push('=');
        text.push_str(&escape(v));
        text.push('\n');
    }
    paths::write_atomic(&path, text.as_bytes())
}

/// Claves no administradas del archivo actual (para el editor avanzado).
pub fn extra_from_disk(cfg: &ServerConfig) -> BTreeMap<String, String> {
    let path = paths::server_run_dir(&cfg.id).join("server.properties");
    read(&path).into_iter().filter(|(k, _)| !MANAGED.contains(&k.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_escape() {
        let s = "Zombis ñandú \\ prueba";
        assert_eq!(unescape(&escape(s)), s);
    }
}
