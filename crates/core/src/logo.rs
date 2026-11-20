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

/// Tipo de imagen por su contenido (no por la extensión): png | jpeg | webp | gif.
pub fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) { Some("png") }
    else if b.starts_with(&[0xFF, 0xD8, 0xFF]) { Some("jpeg") }
    else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" { Some("webp") }
    else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") { Some("gif") }
    else { None }
}

/// Máximo que se acepta leer de una imagen elegida o arrastrada.
pub const MAX_INPUT: u64 = 20 * 1024 * 1024;

/// Lee una imagen de cualquier carpeta de la PC (arrastrada a la ventana) para recortarla en la
/// interfaz. Solo lee: valida tamaño y tipo real; el recorte y la conversión a PNG se hacen en la
/// interfaz y lo que vuelve se valida otra vez al guardar.
pub fn read_input_image(path: &std::path::Path) -> AppResult<String> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::io("No se pudo leer la imagen", e))?;
    if !meta.is_file() { return Err(AppError::invalid("Eso no es un archivo de imagen.")); }
    if meta.len() > MAX_INPUT { return Err(AppError::invalid("La imagen pesa más de 20 MB.")); }
    if meta.len() == 0 { return Err(AppError::invalid("El archivo está vacío.")); }
    let b = std::fs::read(path).map_err(|e| AppError::io("No se pudo leer la imagen", e))?;
    let kind = image_kind(&b).ok_or_else(|| AppError::invalid("El archivo no es una imagen PNG, JPG, WebP o GIF (o está dañado)."))?;
    Ok(format!("data:image/{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
}

/// Logo de 939 Host que reciben los servidores nuevos. Se guarda como cualquier otro logo
/// (archivo del servidor), así se ve igual después de reiniciar y aloje la PC que aloje.
pub const DEFAULT_KIND: &str = "939";
const DEFAULT_BIG: &[u8] = include_bytes!("../assets/logo-939-256.png");
const DEFAULT_ICON: &[u8] = include_bytes!("../assets/logo-939-64.png");

/// Pone el logo de 939 Host. Lo usan los servidores nuevos y la opción «Logo de 939 Host».
pub fn set_default(id: &str) -> AppResult<()> { set(id, DEFAULT_KIND, DEFAULT_BIG, DEFAULT_ICON) }

/// Copia el logo de un servidor a otro (copias de servidor). Sin logo de origen no hace nada.
pub fn copy(from: &str, to: &str) -> AppResult<()> {
    let Some(kind) = instance::load(from)?.logo else { return Ok(()) };
    let big = std::fs::read(logo_path(from)).map_err(|e| AppError::io("No se pudo leer el logo", e))?;
    let icon = std::fs::read(icon_path(from)).map_err(|e| AppError::io("No se pudo leer el ícono", e))?;
    set(to, &kind, &big, &icon)
}

/// Guarda el logo. `kind` = `939`, `custom`, `preset:<n>` o `skin:<jugador>`. `big` es el PNG de 256×256 y `icon` el de 64×64.
pub fn set(id: &str, kind: &str, big: &[u8], icon: &[u8]) -> AppResult<()> {
    let skin_ok = kind.strip_prefix("skin:").map(crate::players::valid_name).unwrap_or(false);
    if !(kind == "custom" || kind == DEFAULT_KIND || skin_ok || kind.strip_prefix("preset:").map(|n| !n.is_empty() && n.len() < 4 && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)) {
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
    fn el_logo_de_939_tiene_los_tamanos_que_pide_minecraft() {
        assert_eq!(png_size(DEFAULT_BIG).unwrap(), (256, 256));
        assert_eq!(png_size(DEFAULT_ICON).unwrap(), (64, 64));
    }
    #[test]
    fn validates_png() {
        assert_eq!(png_size(&png(256, 256)).unwrap(), (256, 256));
        assert!(png_size(b"GIF89a....................................").is_err());
        let mut cut = png(64, 64); cut.truncate(30);
        assert!(png_size(&cut).is_err());
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    #[test]
    fn tipo_real_y_limites() {
        let tmp = tempfile::tempdir().unwrap();
        let png = tmp.path().join("foto.PNG");
        std::fs::write(&png, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]).unwrap();
        assert!(read_input_image(&png).unwrap().starts_with("data:image/png;base64,"));
        let fake = tmp.path().join("virus.png");
        std::fs::write(&fake, b"MZ\x90\x00 no es una imagen").unwrap();
        assert!(read_input_image(&fake).is_err(), "se mira el contenido, no la extensión");
        let jpg = tmp.path().join("a.jpg");
        std::fs::write(&jpg, [0xFF, 0xD8, 0xFF, 0xE0, 1, 2]).unwrap();
        assert!(read_input_image(&jpg).unwrap().starts_with("data:image/jpeg"));
        std::fs::write(tmp.path().join("vacio.png"), b"").unwrap();
        assert!(read_input_image(&tmp.path().join("vacio.png")).is_err());
        assert!(read_input_image(tmp.path()).is_err(), "una carpeta no");
    }
}
