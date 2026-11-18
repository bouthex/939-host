//! Direcciones propias para entrar a un servidor, como `supervivencia.939host`.
//!
//! Cómo funciona (gratis, sin servidores de nadie):
//! - Cada servidor recibe una IP de loopback propia, `127.93.x.y`, derivada de su id. En
//!   Windows todo `127.0.0.0/8` apunta a la misma PC sin configurar nada, así que 939 Host
//!   puede escuchar en `127.93.x.y:25565` aunque haya otros servidores en el 25565 de siempre.
//!   Por eso la dirección no necesita puerto.
//! - El archivo `hosts` de Windows traduce `nombre.939host` a esa IP. Escribirlo pide permiso
//!   de administrador (UAC) una vez por cada cambio; 939 Host solo toca su propio bloque.
//! - 939 Host lleva cada conexión al servidor: el local o, si lo aloja otra PC del grupo, por
//!   la conexión P2P de siempre.
//!
//! Límites reales: es una dirección **interna** de cada PC con 939 Host abierto. No es pública:
//! alguien sin 939 Host no la puede usar, y no reemplaza a un dominio con DNS. `.939host` no es
//! un dominio real de Internet, así que no choca con nada existente.

use crate::error::{AppError, AppResult};
use std::net::Ipv4Addr;

pub const SUFFIX: &str = ".939host";
const BEGIN: &str = "# >>> 939 Host: direcciones de servidores (no editar a mano)";
const END: &str = "# <<< 939 Host";

/// IP de loopback propia del servidor (estable: siempre la misma para el mismo id).
pub fn ip_for(id: &str) -> Ipv4Addr {
    let h = id.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let x = ((h >> 8) & 0xff) as u8;
    let y = (h & 0xff) as u8;
    Ipv4Addr::new(127, 93, x.max(1), y.clamp(1, 254))
}

/// Etiqueta sugerida a partir del nombre: minúsculas, sin acentos ni espacios.
pub fn suggest(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        let c = match c { 'á' | 'à' | 'ä' | 'â' => 'a', 'é' | 'è' | 'ë' | 'ê' => 'e', 'í' | 'ì' | 'ï' | 'î' => 'i', 'ó' | 'ò' | 'ö' | 'ô' => 'o', 'ú' | 'ù' | 'ü' | 'û' => 'u', 'ñ' => 'n', c => c };
        if c.is_ascii_alphanumeric() { out.push(c); } else if !out.ends_with('-') && !out.is_empty() { out.push('-'); }
    }
    let s = out.trim_matches('-').chars().take(40).collect::<String>();
    if s.is_empty() { "servidor".into() } else { s }
}

/// Valida una etiqueta (la parte antes de `.939host`) con las reglas de nombres DNS.
pub fn validate(label: &str) -> AppResult<String> {
    let l = label.trim().trim_end_matches(SUFFIX).to_lowercase();
    if l.is_empty() || l.len() > 40 { return Err(AppError::invalid("La dirección tiene que tener entre 1 y 40 caracteres.")); }
    if !l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') { return Err(AppError::invalid("Usá solo letras sin acentos, números y guiones.")); }
    if l.starts_with('-') || l.ends_with('-') { return Err(AppError::invalid("La dirección no puede empezar ni terminar con guion.")); }
    Ok(l)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry { pub label: String, pub ip: Ipv4Addr }

/// Lo que hay hoy en el bloque de 939 Host del archivo hosts.
pub fn current(hosts: &str) -> Vec<Entry> {
    let mut inside = false;
    let mut v = vec![];
    for line in hosts.lines() {
        let t = line.trim();
        if t == BEGIN { inside = true; continue; }
        if t == END { inside = false; continue; }
        if !inside { continue; }
        let mut it = t.split_whitespace();
        if let (Some(ip), Some(name)) = (it.next(), it.next()) {
            if let (Ok(ip), Some(label)) = (ip.parse::<Ipv4Addr>(), name.strip_suffix(SUFFIX)) { v.push(Entry { label: label.into(), ip }); }
        }
    }
    v
}

/// Archivo hosts nuevo: igual al actual fuera del bloque de 939 Host, con el bloque reemplazado
/// (o quitado si no hay entradas). Conserva los finales de línea del original.
pub fn apply(hosts: &str, entries: &[Entry]) -> String {
    let nl = if hosts.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<&str> = vec![];
    let mut inside = false;
    for line in hosts.lines() {
        let t = line.trim();
        if t == BEGIN { inside = true; continue; }
        if t == END { inside = false; continue; }
        if !inside { out.push(line); }
    }
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) { out.pop(); }
    let mut s = out.join(nl);
    if !entries.is_empty() {
        if !s.is_empty() { s.push_str(nl); s.push_str(nl); }
        s.push_str(BEGIN); s.push_str(nl);
        let mut sorted = entries.to_vec();
        sorted.sort_by(|a, b| a.label.cmp(&b.label));
        for e in sorted { s.push_str(&format!("{}\t{}{}{}", e.ip, e.label, SUFFIX, nl)); }
        s.push_str(END);
    }
    s.push_str(nl);
    s
}

/// Solo se aceptan entradas de 939 Host: IP `127.93.*` y nombre `.939host` válido. Es lo que
/// verifica el ayudante con permisos de administrador antes de escribir.
pub fn check_entries(entries: &[Entry]) -> AppResult<()> {
    for e in entries {
        let o = e.ip.octets();
        if o[0] != 127 || o[1] != 93 { return Err(AppError::invalid("IP fuera del rango de 939 Host.")); }
        validate(&e.label)?;
    }
    let mut labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    labels.sort(); labels.dedup();
    if labels.len() != entries.len() { return Err(AppError::invalid("Hay dos servidores con la misma dirección.")); }
    Ok(())
}

/// Direcciones elegidas en esta PC (id del servidor → etiqueta). Es local a cada PC: no se
/// comparte con el grupo, así cada uno elige cómo llamar a cada servidor sin choques.
fn store() -> std::path::PathBuf { crate::paths::root().join("direcciones.json") }
pub fn load() -> std::collections::BTreeMap<String, String> { crate::paths::read_json(&store()).unwrap_or_default() }
pub fn set(id: &str, label: Option<&str>) -> AppResult<Option<String>> {
    let mut m = load();
    let label = match label { Some(l) => Some(validate(l)?), None => None };
    if let Some(l) = &label {
        if m.iter().any(|(k, v)| k != id && v == l) { return Err(AppError::invalid(format!("{l}{SUFFIX} ya la usa otro servidor en esta PC."))); }
        m.insert(id.into(), l.clone());
    } else { m.remove(id); }
    crate::paths::write_json(&store(), &m)?;
    Ok(label)
}

pub fn hosts_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    root.join("System32").join("drivers").join("etc").join("hosts")
}

/// Formato del pedido que la app le pasa al ayudante elevado: una línea `ip etiqueta` por entrada.
pub fn encode(entries: &[Entry]) -> String { entries.iter().map(|e| format!("{} {}\n", e.ip, e.label)).collect() }
pub fn decode(s: &str) -> AppResult<Vec<Entry>> {
    let mut v = vec![];
    for line in s.lines().filter(|l| !l.trim().is_empty()) {
        let mut it = line.split_whitespace();
        let ip = it.next().and_then(|x| x.parse().ok()).ok_or_else(|| AppError::invalid("Pedido inválido."))?;
        let label = it.next().ok_or_else(|| AppError::invalid("Pedido inválido."))?.to_string();
        v.push(Entry { label, ip });
    }
    check_entries(&v)?;
    Ok(v)
}

/// Escribe el bloque en el archivo hosts (solo con permisos de administrador). Hace una copia
/// `hosts.939host.bak` la primera vez y escribe de forma atómica.
pub fn write_hosts(entries: &[Entry]) -> AppResult<()> {
    check_entries(entries)?;
    let p = hosts_path();
    let old = std::fs::read(&p).map_err(|e| AppError::io("No se pudo leer el archivo hosts", e))?;
    let old = String::from_utf8_lossy(&old).to_string();
    let bak = p.with_file_name("hosts.939host.bak");
    if !bak.exists() { let _ = std::fs::write(&bak, &old); }
    let new = apply(&old, entries);
    if new == old { return Ok(()); }
    let tmp = p.with_file_name("hosts.939host.tmp");
    std::fs::write(&tmp, new.as_bytes()).map_err(|e| AppError::io("No se pudo escribir el archivo hosts", e))?;
    crate::paths::rename_retry(&tmp, &p).map_err(|e| AppError::io("No se pudo reemplazar el archivo hosts", e))?;
    Ok(())
}

/// Script de PowerShell que escribe el bloque de 939 Host en el archivo hosts, para ejecutarlo con
/// permisos de administrador. Lo ejecuta el PowerShell de `System32` (ruta absoluta en una carpeta
/// protegida), **no** `939Host.exe`: así ningún programa ni DLL que esté en la carpeta de la app
/// (donde el usuario puede escribir) corre como administrador. Las entradas van dentro del script
/// (no en un archivo que se pueda cambiar en el medio) y el script las vuelve a validar.
/// Deja el archivo igual que [`apply`] (mismo bloque, mismos finales de línea, copia `.bak`).
pub fn hosts_script(entries: &[Entry]) -> AppResult<String> {
    check_entries(entries)?;
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| a.label.cmp(&b.label));
    let list = sorted.iter().map(|e| format!("'{} {}'", e.ip, e.label)).collect::<Vec<_>>().join(",");
    Ok(format!(r#"$ErrorActionPreference = 'Stop'
$entries = @({list})
$re = '^127\.93\.(25[0-5]|2[0-4]\d|1?\d?\d)\.(25[0-5]|2[0-4]\d|1?\d?\d) [a-z0-9]([a-z0-9-]{{0,38}}[a-z0-9])?$'
foreach ($entry in $entries) {{ if ($entry -cnotmatch $re) {{ exit 3 }} }}
$p = Join-Path $env:SystemRoot 'System32\drivers\etc\hosts'
# Ojo: en PowerShell las variables no distinguen mayúsculas ($e y $E son la misma).
$markBegin = '{BEGIN}'
$markEnd = '{END}'
$old = [IO.File]::ReadAllText($p)
$nl = if ($old.Contains("`r`n")) {{ "`r`n" }} else {{ "`n" }}
$out = New-Object System.Collections.Generic.List[string]
$in = $false
foreach ($l in ($old -split "`r?`n")) {{ $t = $l.Trim(); if ($t -ceq $markBegin) {{ $in = $true; continue }}; if ($t -ceq $markEnd) {{ $in = $false; continue }}; if (-not $in) {{ $out.Add($l) }} }}
while ($out.Count -gt 0 -and $out[$out.Count - 1].Trim() -eq '') {{ $out.RemoveAt($out.Count - 1) }}
$s = [string]::Join($nl, $out)
if ($entries.Count -gt 0) {{
  if ($s -ne '') {{ $s += $nl + $nl }}
  $s += $markBegin + $nl
  foreach ($entry in $entries) {{ $parts = $entry -split ' '; $s += $parts[0] + "`t" + $parts[1] + '{SUFFIX}' + $nl }}
  $s += $markEnd
}}
$s += $nl
$dir = Split-Path $p
$bak = Join-Path $dir 'hosts.939host.bak'
if (-not (Test-Path -LiteralPath $bak)) {{ [IO.File]::WriteAllText($bak, $old, (New-Object Text.UTF8Encoding $false)) }}
if ($s -cne $old) {{
  $tmp = Join-Path $dir 'hosts.939host.tmp'
  [IO.File]::WriteAllText($tmp, $s, (New-Object Text.UTF8Encoding $false))
  Move-Item -LiteralPath $tmp -Destination $p -Force
}}
exit 0
"#))
}

/// `-EncodedCommand` de PowerShell: el script en UTF-16LE y base64 (evita problemas de comillas).
pub fn powershell_encoded(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// PowerShell de Windows por ruta absoluta (nunca se busca por nombre: Windows mira primero la
/// carpeta de la app).
pub fn powershell_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
}

/// Lleva cada conexión de `ip:25565` al servidor local en `127.0.0.1:port`. Para servidores no
/// compartidos (los compartidos usan la ruta del grupo).
pub async fn forward(ip: Ipv4Addr, port: impl Fn() -> Option<u16> + Send + Sync + 'static) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let l = tokio::net::TcpListener::bind((ip, 25565)).await?;
    let port = std::sync::Arc::new(port);
    Ok(tokio::spawn(async move {
        loop {
            let Ok((mut a, _)) = l.accept().await else { break };
            let port = port.clone();
            tokio::spawn(async move {
                let Some(p) = port() else { return };
                if let Ok(mut b) = tokio::net::TcpStream::connect(("127.0.0.1", p)).await {
                    let _ = a.set_nodelay(true); let _ = b.set_nodelay(true);
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
            });
        }
    }))
}

#[cfg(test)]
mod t {
    use super::*;
    #[test]
    fn script_elevado_solo_lleva_entradas_validadas() {
        let e = vec![Entry { label: "zeta".into(), ip: "127.93.1.2".parse().unwrap() }, Entry { label: "alfa".into(), ip: "127.93.1.3".parse().unwrap() }];
        let s = hosts_script(&e).unwrap();
        assert!(s.contains("$entries = @('127.93.1.3 alfa','127.93.1.2 zeta')"), "ordenadas como en apply: {s}");
        assert!(s.contains(BEGIN) && s.contains(END) && s.contains("exit 3"));
        // En PowerShell las variables no distinguen mayúsculas: $e y $E son la misma (así se rompió
        // el bloque en la CI). Ninguna variable puede chocar con otra.
        let mut vars: Vec<String> = s.lines().filter(|l| !l.trim_start().starts_with('#'))
            .flat_map(|l| l.split('$').skip(1).map(|t| t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>()).collect::<Vec<_>>())
            .filter(|v| !v.is_empty()).collect();
        vars.sort(); vars.dedup();
        let mut lower: Vec<String> = vars.iter().map(|v| v.to_lowercase()).collect();
        lower.sort(); lower.dedup();
        assert_eq!(vars.len(), lower.len(), "variables que chocan sin mayúsculas: {vars:?}");
        // Intentos de inyección en la etiqueta o fuera del rango: ni se arma el script.
        for bad in ["x';Start-Process calc;'", "a b", "ejemplo.com", "ñandu", ""] {
            assert!(hosts_script(&[Entry { label: bad.into(), ip: "127.93.1.2".parse().unwrap() }]).is_err(), "{bad}");
        }
        assert!(hosts_script(&[Entry { label: "ok".into(), ip: "8.8.8.8".parse().unwrap() }]).is_err());
        // Codificación de -EncodedCommand: UTF-16LE en base64.
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD.decode(powershell_encoded("exit 0")).unwrap();
        let back: Vec<u16> = raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&back).unwrap(), "exit 0");
        assert!(powershell_path().ends_with("System32/WindowsPowerShell/v1.0/powershell.exe") || powershell_path().to_string_lossy().contains("System32"));
    }
    #[test]
    fn ip_is_loopback_and_stable() {
        let a = ip_for("supervivencia-1a2b");
        assert_eq!(a, ip_for("supervivencia-1a2b"));
        assert!(a.is_loopback() && a.octets()[1] == 93);
        assert_ne!(ip_for("a-1"), ip_for("b-2"));
    }
    #[test]
    fn labels() {
        assert_eq!(suggest("Supervivencia Ñandú 2"), "supervivencia-nandu-2");
        assert_eq!(suggest("  ¡¡ !!"), "servidor");
        assert!(validate("mi-server").is_ok());
        assert_eq!(validate("Mi-Server.939host").unwrap(), "mi-server");
        assert!(validate("-x").is_err() && validate("con espacio").is_err() && validate("ñ").is_err());
    }
    #[test]
    fn hosts_block_only_touches_its_part() {
        let orig = "# Copyright Microsoft\r\n127.0.0.1 localhost\r\n10.0.0.5 nas.casa\r\n";
        let e = vec![Entry { label: "zombies".into(), ip: Ipv4Addr::new(127, 93, 4, 7) }];
        let with = apply(orig, &e);
        assert!(with.starts_with("# Copyright Microsoft\r\n127.0.0.1 localhost\r\n10.0.0.5 nas.casa\r\n"));
        assert!(with.contains("127.93.4.7\tzombies.939host\r\n"));
        assert_eq!(current(&with), e);
        // Reaplicar no duplica; quitar todo deja el original.
        assert_eq!(apply(&with, &e), with);
        assert_eq!(apply(&with, &[]), orig);
    }
    #[test]
    fn helper_rejects_foreign_entries() {
        assert!(decode("127.93.1.2 zombies\n").is_ok());
        assert!(decode("10.0.0.1 banco\n").is_err());
        assert!(decode("127.93.1.2 a\n127.93.1.3 a\n").is_err());
        assert!(decode("127.93.1.2 www.google.com\n").is_err());
    }
    #[tokio::test]
    async fn forwards_to_local_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let srv = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = srv.local_addr().unwrap().port();
        tokio::spawn(async move { let (mut s, _) = srv.accept().await.unwrap(); let mut b = [0u8; 4]; s.read_exact(&mut b).await.unwrap(); s.write_all(&b).await.unwrap(); });
        let ip = Ipv4Addr::new(127, 93, 250, 250);
        let h = match forward(ip, move || Some(p)).await { Ok(h) => h, Err(e) => { eprintln!("sin 127.93.* en este sistema: {e}"); return; } };
        let mut c = tokio::net::TcpStream::connect((ip, 25565)).await.unwrap();
        c.write_all(b"ping").await.unwrap();
        let mut b = [0u8; 4]; c.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"ping");
        h.abort();
    }
}

/// Dirección pública opcional de un servidor, para jugadores **sin** 939 Host. 939 Host no la crea:
/// la aporta quien use un servicio de túnel propio (por ejemplo, playit.gg con su cuenta) o un
/// dominio propio con un registro SRV. Se guarda y se muestra; nunca se usa para exponer la IP.
pub mod public {
    use crate::error::{AppError, AppResult};
    use crate::paths;
    fn path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("direccion-publica.txt") }
    pub fn get(id: &str) -> Option<String> { std::fs::read_to_string(path(id)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) }
    /// Valida «nombre.dominio» o «nombre.dominio:puerto». Una IP numérica se rechaza: mostrarla
    /// a todos podría revelar la conexión de la casa, que es justo lo que se quiere evitar.
    pub fn validate(addr: &str) -> AppResult<String> {
        let a = addr.trim().trim_end_matches('.').to_lowercase();
        let (host, port) = match a.rsplit_once(':') { Some((h, p)) => (h.to_string(), Some(p.to_string())), None => (a.clone(), None) };
        if let Some(p) = &port { if p.parse::<u16>().map(|n| n < 1).unwrap_or(true) { return Err(AppError::invalid("El puerto no es válido.")); } }
        if host.parse::<std::net::IpAddr>().is_ok() || host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::Ipv6Addr>().is_ok() {
            return Err(AppError::invalid("Usá un nombre (por ejemplo, el que te da tu servicio de túnel), no una IP: una IP numérica puede revelar la conexión de tu casa."));
        }
        let labels: Vec<&str> = host.split('.').collect();
        if host.len() > 200 || labels.len() < 2 || labels.iter().any(|l| l.is_empty() || l.len() > 63 || l.starts_with('-') || l.ends_with('-') || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
            return Err(AppError::invalid("No parece una dirección válida (por ejemplo: mundo.joinmc.link o mc.midominio.com:25565)."));
        }
        Ok(a)
    }
    pub fn set(id: &str, addr: Option<&str>) -> AppResult<()> {
        match addr.map(str::trim).filter(|a| !a.is_empty()) {
            Some(a) => { let v = validate(a)?; paths::write_atomic(&path(id), v.as_bytes()) }
            None => { let _ = std::fs::remove_file(path(id)); Ok(()) }
        }
    }
    #[cfg(test)]
    mod tests {
        #[test]
        fn direccion_publica() {
            assert_eq!(super::validate("Mundo.JoinMC.link").unwrap(), "mundo.joinmc.link");
            assert!(super::validate("mc.midominio.com:25570").is_ok());
            assert!(super::validate("181.45.3.2").is_err(), "IP numérica no");
            assert!(super::validate("181.45.3.2:25565").is_err());
            assert!(super::validate("[2800:810::1]:25565").is_err());
            assert!(super::validate("localhost").is_err());
            assert!(super::validate("mal..dominio").is_err());
            assert!(super::validate("a.b:99999").is_err());
        }
    }
}
