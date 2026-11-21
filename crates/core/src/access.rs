//! Quién puede entrar a **jugar** sin 939 Host. Es independiente de la administración y del
//! permiso para alojar, que siguen exigiendo ser integrante del grupo con identidad firmada.
//!
//! - Interruptor por servidor «Permitir conexiones sin 939 Host» (`external_allowed`): lo decide
//!   la propietaria. En servidores compartidos va en la lista firmada, así que ninguna otra PC
//!   puede encenderlo.
//! - Caminos de entrada para quien no tiene 939 Host, cada uno decidido por la PC donde ocurre:
//!   la red local (`settings.lan_access`) y la entrada pública desde Internet
//!   (`settings.public_entrance`).
//! - Con el interruptor apagado: Minecraft escucha solo en `127.0.0.1` (lo único que llega es lo
//!   que trae el túnel autenticado de 939 Host) y ninguna PC abre la entrada pública.

use crate::{instance, network, paths, settings};
use std::collections::BTreeMap;

/// `Some(true)` permitido, `Some(false)` bloqueado, `None` sin decidir (comportamiento anterior).
pub fn external_allowed(id: &str) -> Option<bool> {
    let s = network::load_share(id);
    if s.enabled { s.external_access } else { instance::load(id).ok().and_then(|c| c.external_access) }
}

/// ¿Minecraft escucha en la red local? Solo si la PC lo permite y el servidor no lo bloquea.
pub fn lan_effective(id: &str) -> bool { settings::load().lan_access && external_allowed(id) != Some(false) }

/// ¿Esta PC abre la entrada pública para este servidor?
pub fn internet_entrance(id: &str) -> bool { settings::load().public_entrance && external_allowed(id) == Some(true) }

fn ports_path() -> std::path::PathBuf { paths::root().join("entrada-publica.json") }

/// Puerto público de la entrada de este servidor en esta PC. Estable: se guarda la primera vez,
/// empezando por 25565 (el que Minecraft usa si no se escribe ninguno).
pub fn entrance_port(id: &str) -> u16 {
    let mut map: BTreeMap<String, u16> = paths::read_json(&ports_path()).unwrap_or_default();
    if let Some(p) = map.get(id) { return *p; }
    let used: Vec<u16> = map.values().copied().collect();
    let p = (25565..u16::MAX).find(|p| !used.contains(p)).unwrap_or(25565);
    map.insert(id.to_string(), p);
    let _ = paths::write_json(&ports_path(), &map);
    p
}

pub fn set_entrance_port(id: &str, port: u16) -> crate::error::AppResult<()> {
    if port < 1024 { return Err(crate::error::AppError::invalid("Usá un puerto de 1024 en adelante.")); }
    let mut map: BTreeMap<String, u16> = paths::read_json(&ports_path()).unwrap_or_default();
    if map.iter().any(|(k, p)| k != id && *p == port) { return Err(crate::error::AppError::invalid("Ese puerto ya lo usa la entrada de otro servidor en esta PC.")); }
    map.insert(id.to_string(), port);
    paths::write_json(&ports_path(), &map)
}

/// IPv4 «pública» de verdad: no privada, no CGNAT (100.64/10), no loopback ni enlace local.
pub fn is_global_v4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.is_documentation() || ip.is_multicast()
        || (o[0] == 100 && (64..128).contains(&o[1])) || o[0] == 0 || o[0] >= 240)
}

/// IPv6 global (2000::/3).
pub fn is_global_v6(ip: std::net::Ipv6Addr) -> bool { (ip.segments()[0] & 0xe000) == 0x2000 && !ip.is_loopback() }

/// La IP que el router le dio a esta PC como «WAN» está en el rango que usan los proveedores
/// para compartir una IP entre muchos clientes (CGNAT) o es privada: desde Internet no se llega.
pub fn behind_cgnat(wan: std::net::Ipv4Addr) -> bool { !is_global_v4(wan) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rangos_de_ip() {
        assert!(is_global_v4("181.45.3.2".parse().unwrap()));
        for ip in ["192.168.1.10", "10.0.0.1", "172.16.5.4", "100.64.0.1", "100.127.255.254", "127.0.0.1", "169.254.1.1", "0.0.0.0"] {
            assert!(!is_global_v4(ip.parse().unwrap()), "{ip}");
            assert!(behind_cgnat(ip.parse().unwrap()));
        }
        assert!(is_global_v4("100.128.0.1".parse().unwrap()));
        assert!(is_global_v6("2800:810:4a1::1".parse().unwrap()));
        assert!(!is_global_v6("fe80::1".parse().unwrap()) && !is_global_v6("fd00::1".parse().unwrap()) && !is_global_v6("::1".parse().unwrap()));
    }
    #[test]
    fn puertos_estables_y_sin_choques() {
        // Carpeta que no se borra al terminar: la raíz es una sola para todas las pruebas.
        let _ = paths::init(std::env::temp_dir().join(format!("h939-acc-{}", std::process::id())));
        let (a, b) = (format!("acc-a-{}", std::process::id()), format!("acc-b-{}", std::process::id()));
        let pa = entrance_port(&a);
        let pb = entrance_port(&b);
        assert_ne!(pa, pb);
        assert_eq!(entrance_port(&a), pa, "estable");
        assert!(set_entrance_port(&b, pa).is_err(), "no repite el de otro servidor");
        assert!(set_entrance_port(&b, 80).is_err());
        set_entrance_port(&b, 30000).unwrap();
        assert_eq!(entrance_port(&b), 30000);
    }
}
