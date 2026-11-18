//! Diagnóstico y mundo: informe para pedir ayuda y "mundo nuevo" sin perder el actual.

use crate::error::{AppError, AppResult};
use crate::model::ServerConfig;
use crate::paths;
use crate::process::Manager;
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInfo {
    pub size_bytes: u64,
    pub level_name: String,
    pub seed: String,
    pub dimensions: Vec<String>,
    pub players_data: usize,
    pub exists: bool,
}

pub fn world_info(cfg: &ServerConfig) -> WorldInfo {
    let w = paths::server_run_dir(&cfg.id).join("world");
    let mut dims = vec![];
    if w.join("region").is_dir() { dims.push("Overworld".to_string()); }
    if w.join("DIM-1").is_dir() || w.join("dimensions/minecraft/the_nether").is_dir() { dims.push("Nether".into()); }
    if w.join("DIM1").is_dir() || w.join("dimensions/minecraft/the_end").is_dir() { dims.push("End".into()); }
    if let Ok(rd) = std::fs::read_dir(w.join("dimensions")) {
        for ns in rd.flatten().filter(|e| e.file_name() != "minecraft") {
            if let Ok(r2) = std::fs::read_dir(ns.path()) {
                for d in r2.flatten() { dims.push(format!("{}:{}", ns.file_name().to_string_lossy(), d.file_name().to_string_lossy())); }
            }
        }
    }
    let players = crate::playerdata::saved_player_count(&w);
    WorldInfo {
        size_bytes: paths::dir_size(&w),
        level_name: "world".into(),
        seed: cfg.level_seed.clone(),
        dimensions: dims,
        players_data: players,
        exists: w.join("level.dat").exists(),
    }
}

/// Aparta el mundo actual (a la Papelera, con punto de seguridad antes) para que el próximo
/// inicio genere uno nuevo. Opcionalmente cambia la semilla.
pub fn new_world(mgr: &Manager, cfg: &mut ServerConfig, seed: Option<String>) -> AppResult<()> {
    if mgr.runtime(&cfg.id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor primero.")); }
    let w = paths::server_run_dir(&cfg.id).join("world");
    if w.join("level.dat").exists() {
        crate::snapshots::capture(&cfg.id, "Antes de crear un mundo nuevo", "manual", &cfg.mc_version)?;
        paths::rename_retry(&w, paths::trash_slot(&format!("{}-mundo", cfg.id))).map_err(|e| AppError::io("No se pudo apartar el mundo", e))?;
    }
    if let Some(s) = seed { cfg.level_seed = s.trim().chars().take(64).collect(); }
    crate::instance::save(cfg)
}

fn tail(path: &std::path::Path, max: usize) -> Vec<u8> {
    let Ok(data) = std::fs::read(path) else { return vec![] };
    if data.len() <= max { data } else { data[data.len() - max..].to_vec() }
}

/// Arma un .zip con lo necesario para entender un problema (sin el mundo, sin claves).
/// Quita de un texto lo que no debe salir de la PC en un informe: direcciones IPv4 e IPv6
/// (salvo 127.x y 0.0.0.0, que no identifican a nadie) y el nombre de usuario en rutas
/// (`C:\\Users\\nombre`, `/home/nombre`).
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // IPv4
        if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.')) {
            let mut j = i; let mut parts = vec![]; let mut ok = true;
            for k in 0..4 {
                let st = j;
                while j < b.len() && b[j].is_ascii_digit() && j - st < 3 { j += 1; }
                if j == st { ok = false; break; }
                parts.push(text[st..j].parse::<u16>().unwrap_or(999));
                if k < 3 { if j < b.len() && b[j] == b'.' { j += 1; } else { ok = false; break; } }
            }
            let tail_ok = j >= b.len() || !(b[j].is_ascii_alphanumeric() || (b[j] == b'.' && j + 1 < b.len() && b[j + 1].is_ascii_digit()));
            if ok && tail_ok && parts.iter().all(|p| *p <= 255) {
                if parts[0] == 127 || parts.iter().all(|p| *p == 0) { out.push_str(&text[i..j]); } else { out.push_str("x.x.x.x"); }
                i = j; continue;
            }
        }
        // IPv6 (al menos tres grupos hexadecimales con dos puntos; no 0:0:0 ni ::1)
        if b[i].is_ascii_hexdigit() || b[i] == b':' {
            let st = i; let mut j = i; let mut colons = 0;
            while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b':') { if b[j] == b':' { colons += 1; } j += 1; }
            let cand = &text[st..j];
            let prev_ok = st == 0 || !b[st - 1].is_ascii_alphanumeric();
            if prev_ok && colons >= 3 && cand.len() >= 7 && cand.chars().any(|c| c.is_ascii_hexdigit() && c != '0') && cand != "::1" && cand.split(':').all(|g| g.len() <= 4) {
                out.push_str("x:x:x:x"); i = j; continue;
            }
        }
        out.push(text[i..].chars().next().unwrap());
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    // Nombre de usuario en rutas.
    let mut s = out;
    for pre in ["\\Users\\", "/Users/", "/home/", "\\\\Users\\\\"] {
        let mut res = String::new(); let mut rest = s.as_str();
        while let Some(pos) = rest.find(pre) {
            res.push_str(&rest[..pos + pre.len()]);
            let after = &rest[pos + pre.len()..];
            let end = after.find(|c: char| c == '\\' || c == '/' || c == '"' || c.is_whitespace()).unwrap_or(after.len());
            res.push_str("<usuario>");
            rest = &after[end..];
        }
        res.push_str(rest); s = res;
    }
    s
}

pub fn report(mgr: &Manager, cfg: &ServerConfig, hardware_json: &str) -> AppResult<std::path::PathBuf> {
    let run = paths::server_run_dir(&cfg.id);
    let out_dir = paths::root().join("logs");
    std::fs::create_dir_all(&out_dir).ok();
    let out = out_dir.join(format!("diagnostico-{}-{}.zip", cfg.id, chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let f = std::fs::File::create(&out).map_err(|e| AppError::io("No se pudo crear el informe", e))?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // Todo lo que entra al informe pasa por `redact`: sin IPs, sin el nombre de usuario de
    // Windows en las rutas y sin contraseñas o secretos de server.properties.
    let mut add = |name: &str, data: &[u8]| -> AppResult<()> {
        z.start_file(name, opt)?;
        z.write_all(redact(&String::from_utf8_lossy(data)).as_bytes()).map_err(|e| AppError::io("informe", e))?;
        Ok(())
    };
    let mut safe = serde_json::to_value(cfg)?;
    if let Some(o) = safe.as_object_mut() { o.remove("identity"); }
    add("configuracion.json", serde_json::to_string_pretty(&safe)?.as_bytes())?;
    add("consola-939host.txt", mgr.console(&cfg.id).join("\n").as_bytes())?;
    add("equipo.json", hardware_json.as_bytes())?;
    add("latest.log", &tail(&run.join("logs/latest.log"), 4 * 1024 * 1024))?;
    add("instalador.log", &tail(&run.join("logs/instalador.log"), 1024 * 1024))?;
    let mut crashes: Vec<_> = std::fs::read_dir(run.join("crash-reports")).map(|r| r.flatten().collect()).unwrap_or_default();
    crashes.sort_by_key(|e| e.file_name());
    for c in crashes.iter().rev().take(3) {
        add(&format!("crash-reports/{}", c.file_name().to_string_lossy()), &tail(&c.path(), 2 * 1024 * 1024))?;
    }
    let mods: Vec<String> = std::fs::read_dir(run.join("mods")).map(|r| r.flatten().map(|e| {
        let meta = crate::mods::jar_meta(&e.path());
        format!("{}\t{}", e.file_name().to_string_lossy(), meta.map(|m| format!("{} {} [{}]", m.id, m.version, m.environment)).unwrap_or_default())
    }).collect()).unwrap_or_default();
    add("mods.txt", mods.join("\n").as_bytes())?;
    let props = std::fs::read_to_string(run.join("server.properties")).unwrap_or_default()
        .lines().filter(|l| { let k = l.split('=').next().unwrap_or("").to_ascii_lowercase(); !(k.contains("password") || k.contains("secret") || k.contains("token") || k == "server-ip") }).collect::<Vec<_>>().join("\n");
    add("server.properties", props.as_bytes())?;
    z.finish()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_logs_but_no_identity_or_secrets() {
        let _ = paths::init(std::env::temp_dir().join("939host-diag-tests"));
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        let cfg: ServerConfig = serde_json::from_value(serde_json::json!({
            "id": format!("d{}", hex::encode(b)), "name": "x", "description": "", "mcVersion": "26.2", "loader": "fabric",
            "loaderVersion": "0.20.1", "javaMajor": 25, "memoryMb": 1024, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565,
            "maxPlayers": 4, "motd": "", "onlineMode": true, "whitelist": false, "difficulty": "easy", "gamemode": "survival",
            "hardcore": false, "pvp": true, "levelSeed": "939", "levelType": "minecraft:normal", "viewDistance": 8, "simulationDistance": 6,
            "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15, "restoreIntervalMin": 15,
            "installed": true, "installError": null, "eulaAcceptedAt": "x", "createdAt": "x",
            "identity": { "publicKey": "CLAVE-SECRETA", "accessCode": "X" }, "extraProperties": {}
        })).unwrap();
        let run = paths::server_run_dir(&cfg.id);
        std::fs::create_dir_all(run.join("logs")).unwrap();
        std::fs::create_dir_all(run.join("crash-reports")).unwrap();
        std::fs::write(run.join("logs/latest.log"), "Done (3s)! Steve[/203.0.113.9:5555] logged in").unwrap();
        std::fs::write(run.join("crash-reports/crash-1.txt"), "NullPointerException").unwrap();
        std::fs::write(run.join("server.properties"), "server-port=1\nrcon.password=hunter2\n").unwrap();
        let mgr = Manager::new(std::sync::Arc::new(crate::events::NoEvents));
        let p = report(&mgr, &cfg, "{}").unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&p).unwrap()).unwrap();
        let mut all = String::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            all.push_str(&f.name().map(|n| n.to_string()).unwrap_or_default());
            std::io::Read::read_to_string(&mut f, &mut all).unwrap();
        }
        assert!(!all.contains("203.0.113.9"), "el informe no lleva IPs");
        assert!(all.contains("Done (3s)!") && all.contains("NullPointerException") && all.contains("server-port=1"));
        assert!(!all.contains("CLAVE-SECRETA") && !all.contains("hunter2"));
        let info = world_info(&cfg);
        assert!(!info.exists);
    }
}

#[cfg(test)]
mod redact_tests {
    #[test]
    fn removes_ips_users_and_keeps_the_rest() {
        let t = "[12:00:01] Steve[/203.0.113.9:51234] logged in; 192.168.0.12 y 10.0.0.1:25565; local 127.0.0.1:25565; ver 1.21.4; 2001:db8:85a3::8a2e:370:7334; ::1; C:\\Users\\Bout\\AppData\\Local\\939Host y /home/bout/x; 12:00:01";
        let r = super::redact(t);
        for leak in ["203.0.113.9", "192.168.0.12", "10.0.0.1", "2001:db8", "Bout", "/home/bout"] { assert!(!r.contains(leak), "{leak} en {r}"); }
        for keep in ["127.0.0.1:25565", "1.21.4", "12:00:01", "logged in", "::1", "AppData"] { assert!(r.contains(keep), "{keep} falta en {r}"); }
    }
}
