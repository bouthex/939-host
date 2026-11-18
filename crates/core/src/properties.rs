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
    // Servidores compartidos: server.properties viaja entre las PCs del grupo con cada punto de
    // restauración. Para que otra PC no pueda abrir por ahí un acceso remoto a la consola de la
    // anfitriona (RCON, query o el protocolo de administración con una clave que ella conoce),
    // en una PC que aloja un servidor del grupo esos accesos quedan apagados o solo locales.
    // 939 Host maneja la consola por la entrada estándar: no los necesita.
    if crate::network::load_share(&cfg.id).enabled {
        map.insert("enable-rcon".into(), "false".into());
        map.insert("enable-query".into(), "false".into());
        map.insert("management-server-host".into(), "localhost".into());
        if map.get("management-server-enabled").map(|v| v == "true").unwrap_or(false) { map.insert("management-server-enabled".into(), "false".into()); }
    }

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

    /// Red team (vuelta 2): una coanfitriona activa la consola remota en el `server.properties`
    /// que viaja con cada punto de restauración, o en la configuración avanzada. En la PC que aloja
    /// un servidor del grupo, RCON, query y el protocolo de administración quedan apagados o locales.
    #[test]
    fn otra_pc_no_abre_la_consola_remota_en_un_servidor_del_grupo() {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-props-{}", std::process::id())));
        let cfg_for = |id: &str| -> ServerConfig {
            let mut c: ServerConfig = serde_json::from_value(serde_json::json!({
                "id": id, "name": "x", "description": "", "mcVersion": "1.21.1", "loader": "vanilla",
                "loaderVersion": null, "javaMajor": 21, "memoryMb": 1024, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565,
                "maxPlayers": 4, "motd": "", "onlineMode": true, "whitelist": false, "difficulty": "easy", "gamemode": "survival",
                "hardcore": false, "pvp": true, "levelSeed": "", "levelType": "minecraft:normal", "viewDistance": 8, "simulationDistance": 6,
                "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15, "restoreIntervalMin": 15,
                "installed": true, "installError": null, "eulaAcceptedAt": "x", "createdAt": "x",
                "identity": { "publicKey": "a", "accessCode": "X" }, "extraProperties": {}
            })).unwrap();
            c.extra_properties.insert("enable-rcon".into(), "true".into());
            c.extra_properties.insert("rcon.password".into(), "clave-de-otra-pc".into());
            c
        };
        let hostile = "enable-rcon=true\nrcon.port=25575\nenable-query=true\nmanagement-server-enabled=true\nmanagement-server-host=0.0.0.0\n";
        let prepare = |id: &str, shared: bool| {
            let dir = paths::server_run_dir(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("server.properties"), hostile).unwrap();
            crate::network::save_share(id, &crate::network::ShareConfig { enabled: shared, ..Default::default() }).unwrap();
            write_for(&cfg_for(id)).unwrap();
            read(&dir.join("server.properties"))
        };
        let p = prepare(&format!("props-grupo-{}", std::process::id()), true);
        assert_eq!(p.get("enable-rcon").map(String::as_str), Some("false"), "{p:?}");
        assert_eq!(p.get("enable-query").map(String::as_str), Some("false"), "{p:?}");
        assert_eq!(p.get("management-server-enabled").map(String::as_str), Some("false"), "{p:?}");
        assert_eq!(p.get("management-server-host").map(String::as_str), Some("localhost"), "{p:?}");
        // Un servidor que no se comparte es de esta PC: su dueña puede usar RCON si quiere.
        let p = prepare(&format!("props-solo-{}", std::process::id()), false);
        assert_eq!(p.get("enable-rcon").map(String::as_str), Some("true"), "{p:?}");
    }
}
