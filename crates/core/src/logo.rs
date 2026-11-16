//! Logo de cada servidor. Nunca cambia el logo de 939 Host.
//!
//! - `logo.png` (256×256) en la carpeta del servidor: lo que muestra 939 Host.
//! - `run/server-icon.png` (64×64): función oficial de Minecraft; el servidor lo manda en la
//!   consulta de estado y aparece en la lista de servidores del juego y en «Servidores de amigos».
//! El recorte y el escalado se hacen en la interfaz (canvas); acá se valida lo que llega.

use crate::error::{AppError, AppResult};
use crate::{instance, paths};
use base64::Engine;

const MAX_BYTES: usize = 2 * 1024 * 1024;

/// Ancho y alto de un PNG válido (firma + IHDR), o error.
pub fn png_size(b: &[u8]) -> AppResult<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if b.len() < 33 || b[..8] != SIG || &b[12..16] != b"IHDR" { return Err(AppError::invalid("La imagen no es un PNG válido.")); }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    if !b.ends_with(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]) { return Err(AppError::invalid("La imagen está incompleta.")); }
    Ok((w, h))
}

fn logo_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("logo.png") }
fn icon_path(id: &str) -> std::path::PathBuf { paths::server_run_dir(id).join("server-icon.png") }

/// Guarda el logo. `kind` = `custom` o `preset:<n>`. `big` es el PNG de 256×256 y `icon` el de 64×64.
pub fn set(id: &str, kind: &str, big: &[u8], icon: &[u8]) -> AppResult<()> {
    if !(kind == "custom" || kind.strip_prefix("preset:").map(|n| !n.is_empty() && n.len() < 4 && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)) {
        return Err(AppError::invalid("Tipo de logo desconocido."));
    }
    if big.len() > MAX_BYTES || icon.len() > MAX_BYTES { return Err(AppError::invalid("La imagen es demasiado grande.")); }
    if png_size(big)? != (256, 256) { return Err(AppError::invalid("El logo tiene que ser de 256×256.")); }
    if png_size(icon)? != (64, 64) { return Err(AppError::invalid("Minecraft pide un ícono de 64×64.")); }
    let mut cfg = instance::load(id)?;
    paths::write_atomic(&logo_path(id), big)?;
    std::fs::create_dir_all(paths::server_run_dir(id)).ok();
    paths::write_atomic(&icon_path(id), icon)?;
    cfg.logo = Some(kind.into());
    instance::save(&cfg)
}

/// Vuelve al ícono genérico (borra solo los archivos de logo que puso 939 Host).
pub fn clear(id: &str) -> AppResult<()> {
    let mut cfg = instance::load(id)?;
    if cfg.logo.is_some() { let _ = std::fs::remove_file(icon_path(id)); }
    let _ = std::fs::remove_file(logo_path(id));
    cfg.logo = None;
    instance::save(&cfg)
}

/// `data:image/png;base64,…` del logo de una carpeta de servidor (sirve también en la Papelera).
pub fn data_url_in(dir: &std::path::Path) -> Option<String> {
    let b = std::fs::read(dir.join("logo.png")).ok()?;
    png_size(&b).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
}
/// Con caché por fecha de modificación: la lista de servidores se refresca seguido y no tiene
/// sentido leer y codificar la imagen cada vez.
pub fn data_url(id: &str) -> Option<String> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, (std::time::SystemTime, String)>>> = OnceLock::new();
    let p = logo_path(id);
    let mt = std::fs::metadata(&p).ok()?.modified().ok()?;
    let cache = CACHE.get_or_init(Default::default);
    if let Some((t, u)) = cache.lock().unwrap().get(id) { if *t == mt { return Some(u.clone()); } }
    let u = data_url_in(&paths::server_dir(id))?;
    cache.lock().unwrap().insert(id.to_string(), (mt, u.clone()));
    Some(u)
}

#[cfg(test)]
mod t {
    use super::*;
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        v.extend_from_slice(b"IHDR"); v.extend_from_slice(&w.to_be_bytes()); v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v.extend_from_slice(&[0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]);
        v
    }
    #[test]
    fn validates_png() {
        assert_eq!(png_size(&png(256, 256)).unwrap(), (256, 256));
        assert!(png_size(b"GIF89a....................................").is_err());
        let mut cut = png(64, 64); cut.truncate(30);
        assert!(png_size(&cut).is_err());
    }
}
