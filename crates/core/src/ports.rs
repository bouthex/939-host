//! Puertos de los servidores: si están libres (comprobado igual que lo hace Java) y quién los usa.
//!
//! Causa del aviso «el puerto 25565 ya está en uso» que el reparador no resolvía: 939 Host
//! comprobaba el puerto con una apertura común en 127.0.0.1, pero Java en Windows abre el puerto
//! en modo exclusivo. Si otro programa escucha en todas las direcciones (0.0.0.0 o [::]) con el
//! mismo puerto, la comprobación común pasa y la de Java falla: el servidor no arrancaba y el
//! reparador decía que no había problema. Ahora se comprueba en modo exclusivo y en IPv4 e IPv6.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

/// ¿Puede un servidor de Minecraft abrir este puerto ahora?
pub fn is_free(port: u16) -> bool {
    if port < 1024 { return false; }
    let v4 = [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED].into_iter().all(|ip| probe(SocketAddr::from((ip, port))).is_ok());
    // IPv6 puede no estar disponible en la PC: en ese caso no cuenta como ocupado.
    let v6 = match probe(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))) {
        Ok(()) => true,
        Err(e) => matches!(e.kind(), std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported) || e.raw_os_error() == Some(10047) || e.raw_os_error() == Some(97),
    };
    v4 && v6
}

/// Reserva y suelta el puerto en modo exclusivo (Windows) o sin reutilización (resto). Solo
/// `bind`, sin `listen`: el choque se detecta igual y no aparece el aviso del Firewall de Windows
/// (que salta cuando un programa empieza a escuchar en la red).
fn probe(addr: SocketAddr) -> std::io::Result<()> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() { let _ = s.set_only_v6(true); }
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{setsockopt, SOL_SOCKET, SO_EXCLUSIVEADDRUSE};
        let on: i32 = 1;
        setsockopt(s.as_raw_socket() as usize, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, &on as *const i32 as *const u8, 4);
    }
    s.bind(&addr.into())?;
    Ok(())
}

/// El primer puerto libre cerca del actual (o en el rango habitual de Minecraft).
pub fn free_near(current: u16, avoid: &[u16]) -> Option<u16> {
    (current.saturating_add(1)..=current.saturating_add(50)).chain(25565..25700)
        .find(|p| *p >= 1024 && !avoid.contains(p) && is_free(*p))
}

/// Programa que escucha en un puerto (si se puede saber).
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortUser {
    pub pid: u32,
    /// Nombre del ejecutable («java.exe», «Discord.exe»…), vacío si Windows no lo informa.
    pub name: String,
}

pub fn user_of(port: u16) -> Option<PortUser> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new(crate::paths::windows_program("NETSTAT.EXE")).args(["-ano", "-p", "TCP"]).creation_flags(0x0800_0000).output().ok()?;
        let mut pid = parse_netstat(&String::from_utf8_lossy(&out.stdout), port);
        if pid.is_none() {
            let out6 = std::process::Command::new(crate::paths::windows_program("NETSTAT.EXE")).args(["-ano", "-p", "TCPv6"]).creation_flags(0x0800_0000).output().ok()?;
            pid = parse_netstat(&String::from_utf8_lossy(&out6.stdout), port);
        }
        let pid = pid?;
        let list = std::process::Command::new(crate::paths::windows_program("tasklist.exe")).args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).creation_flags(0x0800_0000).output().ok();
        let name = list.map(|o| parse_tasklist(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
        Some(PortUser { pid, name })
    }
    #[cfg(target_os = "linux")]
    {
        let pid = linux_listener_pid(port)?;
        let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_default();
        Some(PortUser { pid, name })
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    { let _ = port; None }
}

/// PID que escucha en `port` según `netstat -ano` (independiente del idioma de Windows: se
/// reconoce por la dirección remota vacía, no por la palabra «LISTENING»).
pub fn parse_netstat(text: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 || !f[0].eq_ignore_ascii_case("TCP") { continue; }
        let (local, remote, pid) = (f[1], f[2], f[f.len() - 1]);
        let listening = remote == "0.0.0.0:0" || remote == "[::]:0" || remote == "*:*";
        if listening && local.ends_with(&suffix) {
            if let Ok(p) = pid.parse::<u32>() { if p != 0 { return Some(p); } }
        }
    }
    None
}

/// Nombre del ejecutable en la salida CSV de `tasklist` («"java.exe","1234",…»).
pub fn parse_tasklist(text: &str) -> String {
    let line = text.lines().find(|l| l.starts_with('"')).unwrap_or("");
    line.split("\",\"").next().unwrap_or("").trim_matches('"').to_string()
}

#[cfg(target_os = "linux")]
fn linux_listener_pid(port: u16) -> Option<u32> {
    let mut inodes = vec![];
    for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(t) = std::fs::read_to_string(f) else { continue };
        for line in t.lines().skip(1) {
            let c: Vec<&str> = line.split_whitespace().collect();
            if c.len() < 10 || c[3] != "0A" { continue; }
            let p = c[1].rsplit(':').next().and_then(|h| u16::from_str_radix(h, 16).ok());
            if p == Some(port) { inodes.push(format!("socket:[{}]", c[9])); }
        }
    }
    if inodes.is_empty() { return None; }
    for e in std::fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else { continue };
        for fd in fds.flatten() {
            if let Ok(l) = std::fs::read_link(fd.path()) { if inodes.iter().any(|i| l.to_string_lossy() == *i) { return Some(pid); } }
        }
    }
    None
}

/// Quién ocupa el puerto, en palabras, distinguiendo los servidores de 939 Host.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Occupant {
    /// Otro servidor de 939 Host de esta PC que está encendido.
    OwnServer { id: String, name: String },
    /// 939 Host mismo (una dirección de juego o un túnel).
    ThisApp,
    /// Otro programa.
    Program { name: String, pid: u32 },
    /// No se pudo saber.
    Unknown,
}

impl Occupant {
    pub fn describe(&self) -> String {
        match self {
            Occupant::OwnServer { name, .. } => format!("tu servidor «{name}», que está encendido"),
            Occupant::ThisApp => "939 Host mismo (una dirección de juego)".into(),
            Occupant::Program { name, pid } if !name.is_empty() => format!("otro programa ({name}, proceso {pid})"),
            Occupant::Program { pid, .. } => format!("otro programa (proceso {pid})"),
            Occupant::Unknown => "otro programa".into(),
        }
    }
}

/// Clasifica al que usa `port`. `own` son los servidores de 939 Host con su proceso (id, nombre, pid).
pub fn occupant(port: u16, own: &[(String, String, u32)]) -> Occupant { classify(user_of(port), std::process::id(), own) }

fn classify(user: Option<PortUser>, self_pid: u32, own: &[(String, String, u32)]) -> Occupant {
    match user {
        Some(u) if u.pid == self_pid => Occupant::ThisApp,
        Some(u) => match own.iter().find(|(_, _, pid)| *pid == u.pid) {
            Some((id, name, _)) => Occupant::OwnServer { id: id.clone(), name: name.clone() },
            None => Occupant::Program { name: u.name, pid: u.pid },
        },
        None => Occupant::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstat_en_cualquier_idioma() {
        let es = "\r\nConexiones activas\r\n\r\n  Proto  Dirección local          Dirección remota        Estado           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              ESCUCHANDO       1052\r\n  TCP    0.0.0.0:25565          0.0.0.0:0              ESCUCHANDO       8812\r\n  TCP    127.0.0.1:25565        127.0.0.1:50122        ESTABLECIDO      8812\r\n";
        assert_eq!(parse_netstat(es, 25565), Some(8812));
        assert_eq!(parse_netstat(es, 255), None, "no confunde 25565 con 255");
        let v6 = "  TCP    [::]:25565             [::]:0                 LISTENING       4410\r\n";
        assert_eq!(parse_netstat(v6, 25565), Some(4410));
        assert_eq!(parse_tasklist("\"java.exe\",\"8812\",\"Console\",\"1\",\"512.000 KB\"\r\n"), "java.exe");
        assert_eq!(parse_tasklist("INFO: No hay tareas ejecutándose que coincidan con los criterios especificados.\r\n"), "");
    }

    /// Otro programa escucha en todas las direcciones: el puerto cuenta como ocupado (antes la
    /// comprobación en 127.0.0.1 lo daba por libre) y se sabe quién es.
    #[test]
    fn ocupado_por_otro_programa_en_todas_las_direcciones() {
        let l = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!is_free(port));
        let near = free_near(port, &[]).unwrap();
        assert_ne!(near, port);
        assert!(is_free(near));
        #[cfg(any(windows, target_os = "linux"))]
        assert_eq!(user_of(port).map(|u| u.pid), Some(std::process::id()), "se sabe qué proceso lo usa");
        assert_eq!(occupant(port, &[]), Occupant::ThisApp);
    }

    #[test]
    fn distingue_servidor_propio_de_otro_programa() {
        let own = vec![("srv1".to_string(), "Supervivencia".to_string(), 4410u32)];
        let u = |pid, name: &str| Some(PortUser { pid, name: name.into() });
        assert_eq!(classify(u(4410, "java.exe"), 1, &own), Occupant::OwnServer { id: "srv1".into(), name: "Supervivencia".into() });
        assert_eq!(classify(u(77, "java.exe"), 1, &own), Occupant::Program { name: "java.exe".into(), pid: 77 }, "un Java ajeno no se trata como propio");
        assert_eq!(classify(u(1, "939Host.exe"), 1, &own), Occupant::ThisApp);
        assert_eq!(classify(None, 1, &own), Occupant::Unknown);
    }

    #[test]
    fn ocupado_solo_en_ipv6() {
        let Ok(l) = std::net::TcpListener::bind(("::", 0)) else { return };
        let port = l.local_addr().unwrap().port();
        assert!(!is_free(port), "un programa en [::] también ocupa el puerto");
    }
}
