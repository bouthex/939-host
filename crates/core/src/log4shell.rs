//! Protección contra Log4Shell (CVE-2021-44228 y siguientes) en versiones viejas de Minecraft.
//!
//! Los servidores de Minecraft 1.7 a 1.18.0 usan versiones de Log4j que ejecutan código si un
//! jugador escribe un texto como `${jndi:ldap://…}` en el chat o en su nombre. En un grupo de
//! 939 Host, eso permitiría que una jugadora tome control de la PC anfitriona.
//!
//! Se aplica la mitigación oficial de Mojang:
//! * 1.17 a 1.18.0 (y snapshots de 2021): `-Dlog4j2.formatMsgNoLookups=true`.
//! * 1.12 a 1.16.5 (y snapshots de 2017 a 2020): su archivo `log4j2_112-116.xml`.
//! * 1.7 a 1.11.2 (y snapshots de 2013 a 2016): su archivo `log4j2_17-111.xml`.
//!
//! Los archivos se descargan de Mojang con su SHA-1 verificado al instalar, y se vuelven a
//! verificar en cada encendido: si faltan o cambiaron, el servidor no enciende (falla cerrado).

use crate::error::{AppError, AppResult};
use std::path::Path;

/// Nombre del archivo en la carpeta del servidor.
pub const CONFIG_FILE: &str = "log4j2-939host.xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// Versión sin la falla.
    None,
    /// Alcanza con desactivar las búsquedas (Log4j 2.10 o posterior).
    NoLookups,
    /// Hace falta la configuración de Mojang (Log4j anterior a 2.10).
    Config { url: &'static str, sha1: &'static str },
}

const CFG_112_116: Fix = Fix::Config { url: "https://piston-data.mojang.com/v1/objects/02937d122c86ce73319ef9975b58896fc1b491d1/log4j2_112-116.xml", sha1: "02937d122c86ce73319ef9975b58896fc1b491d1" };
const CFG_17_111: Fix = Fix::Config { url: "https://piston-data.mojang.com/v1/objects/4bb89a97a66f350bc9f73b3ca8509632682aea2e/log4j2_17-111.xml", sha1: "4bb89a97a66f350bc9f73b3ca8509632682aea2e" };

/// Qué protección necesita una versión de Minecraft.
pub fn fix_for(mc: &str) -> Fix {
    // Snapshots: «21w44a».
    if let Some((yy, rest)) = mc.split_once('w') {
        if let (Ok(year), true) = (yy.parse::<u32>(), rest.len() >= 3 && rest[..2].bytes().all(|b| b.is_ascii_digit())) {
            let week: u32 = rest[..2].parse().unwrap_or(0);
            return match year {
                13..=16 => CFG_17_111,
                17..=20 => CFG_112_116,
                21 if week <= 49 => Fix::NoLookups,
                _ => Fix::None,
            };
        }
    }
    // Versiones «1.x.y» (con o sin -preN / -rcN). Las de numeración nueva («26.1») no la tienen.
    let base = mc.split(['-', ' ']).next().unwrap_or(mc);
    let nums: Vec<u32> = base.split('.').map(|p| p.parse().unwrap_or(u32::MAX)).collect();
    match nums.as_slice() {
        [1, minor, rest @ ..] => {
            let patch = rest.first().copied().unwrap_or(0);
            match *minor {
                7..=11 => CFG_17_111,
                12..=16 => CFG_112_116,
                17 => Fix::NoLookups,
                18 if patch == 0 => Fix::NoLookups,
                _ => Fix::None,
            }
        }
        _ => Fix::None,
    }
}

/// Argumentos de Java para la versión (relativos a la carpeta del servidor, que es el directorio
/// de trabajo de Java).
pub fn jvm_flags(mc: &str) -> Vec<String> {
    match fix_for(mc) {
        Fix::None => vec![],
        Fix::NoLookups => vec!["-Dlog4j2.formatMsgNoLookups=true".into()],
        Fix::Config { .. } => vec![format!("-Dlog4j.configurationFile={CONFIG_FILE}"), "-Dlog4j2.formatMsgNoLookups=true".into()],
    }
}

/// Descarga (con SHA-1 verificado) el archivo de Mojang si la versión lo necesita.
pub async fn ensure(mc: &str, run_dir: &Path) -> AppResult<()> {
    let Fix::Config { url, sha1 } = fix_for(mc) else { return Ok(()) };
    let dst = run_dir.join(CONFIG_FILE);
    if verify_file(&dst, sha1) { return Ok(()); }
    crate::net::download(url, &dst, crate::net::Hash::Sha1(sha1), |_, _| {}).await
        .map_err(|e| AppError::new("log4shell", "No se pudo descargar la protección de Mojang contra Log4Shell.").with_detail(e.message))
}

fn verify_file(p: &Path, sha1: &str) -> bool {
    use sha1::Digest;
    std::fs::read(p).map(|b| hex::encode(sha1::Sha1::digest(&b)) == sha1).unwrap_or(false)
}

/// Antes de encender: si la versión necesita el archivo de Mojang, tiene que estar y ser el
/// original. Si no, no se enciende.
pub fn check(mc: &str, run_dir: &Path) -> AppResult<()> {
    let Fix::Config { sha1, .. } = fix_for(mc) else { return Ok(()) };
    if verify_file(&run_dir.join(CONFIG_FILE), sha1) { return Ok(()); }
    Err(AppError::new("log4shell", format!("Minecraft {mc} tiene una falla grave (Log4Shell) que permite tomar control de esta PC desde el chat. Falta la protección oficial de Mojang o fue modificada: reinstalá el servidor desde Ajustes → Avanzado para descargarla.")))
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn version_a_proteccion() {
        for v in ["1.7.10", "1.8.9", "1.11.2", "13w41a", "16w50a"] { assert_eq!(fix_for(v), CFG_17_111, "{v}"); }
        for v in ["1.12", "1.12.2", "1.16.5", "1.16.5-rc1", "1.13-pre2", "17w43a", "20w51a"] { assert_eq!(fix_for(v), CFG_112_116, "{v}"); }
        for v in ["1.17", "1.17.1", "1.18", "1.18-pre1", "21w44a"] { assert_eq!(fix_for(v), Fix::NoLookups, "{v}"); }
        for v in ["1.18.1", "1.18.2", "1.20.4", "1.21.11", "26.1", "26.3", "22w03a", "1.6.4", "algo"] { assert_eq!(fix_for(v), Fix::None, "{v}"); }
        assert_eq!(jvm_flags("1.12.2"), vec![format!("-Dlog4j.configurationFile={CONFIG_FILE}"), "-Dlog4j2.formatMsgNoLookups=true".into()]);
        assert!(jvm_flags("26.3").is_empty());
    }

    #[test]
    fn sin_el_archivo_original_no_enciende() {
        let d = std::env::temp_dir().join(format!("h939-log4shell-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(check("1.12.2", &d).is_err(), "falta el archivo");
        std::fs::write(d.join(CONFIG_FILE), b"<Configuration/> modificada").unwrap();
        assert!(check("1.12.2", &d).is_err(), "archivo cambiado");
        assert!(check("1.17.1", &d).is_ok() && check("26.3", &d).is_ok(), "esas versiones no necesitan el archivo");
    }
}
