//! Chat de voz por mods (Simple Voice Chat): puerto UDP del servidor.
//!
//! El chat de voz no usa la conexión de Minecraft: Simple Voice Chat abre un puerto UDP propio
//! (24454 por defecto, `port` en `config/voicechat/voicechat-server.properties`). 939 Host lo lee
//! para llevar la voz por la conexión cifrada del grupo (ver `cluster::open_voice`).

use crate::paths;

pub const SVC_DEFAULT_PORT: u16 = 24454;

/// ¿Tiene instalado Simple Voice Chat? (por el nombre del jar o su id de mod)
pub fn has_svc(server: &str) -> bool {
    let dir = paths::server_run_dir(server).join("mods");
    std::fs::read_dir(&dir).map(|rd| rd.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n.ends_with(".jar") && (n.contains("voicechat") || n.contains("voice-chat") || crate::mods::jar_meta(&e.path()).map(|m| m.id == "voicechat").unwrap_or(false))
    })).unwrap_or(false)
}

/// Puerto UDP de voz del servidor, si tiene Simple Voice Chat. `-1` en la configuración significa
/// «el mismo puerto que Minecraft» (también UDP).
pub fn port(server: &str, mc_port: u16) -> Option<u16> {
    if !has_svc(server) { return None; }
    let cfg = std::fs::read_to_string(paths::server_run_dir(server).join("config/voicechat/voicechat-server.properties")).unwrap_or_default();
    Some(parse_port(&cfg, mc_port))
}

pub fn parse_port(cfg: &str, mc_port: u16) -> u16 {
    let v = cfg.lines().map(str::trim).find_map(|l| l.strip_prefix("port=")).and_then(|v| v.trim().parse::<i32>().ok());
    match v { Some(-1) => mc_port, Some(p) if (1024..=65535).contains(&p) => p as u16, _ => SVC_DEFAULT_PORT }
}

#[cfg(test)]
mod tests {
    #[test]
    fn puerto_de_voz() {
        assert_eq!(super::parse_port("", 25565), 24454);
        assert_eq!(super::parse_port("# x\nport=24500\n", 25565), 24500);
        assert_eq!(super::parse_port("port=-1", 25570), 25570);
        assert_eq!(super::parse_port("port=80", 25565), 24454, "puertos reservados no");
    }
}
