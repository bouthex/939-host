//! Foto de perfil de la persona que usa esta PC (opcional).
//!
//! - Se guarda solo en esta PC (`foto-perfil.png`, 128×128). La interfaz recorta y convierte a PNG
//!   con un canvas; acá se valida otra vez lo que llega (firma, tamaño, peso).
//! - A las demás PCs del grupo solo se anuncia la huella SHA-256. Cada una la pide una vez por la
//!   conexión cifrada del grupo y la guarda en caché por huella: si la huella no coincide con lo
//!   recibido, se descarta. No se usan imágenes remotas de internet.

use crate::error::{AppError, AppResult};
use crate::paths;
use base64::Engine;
use sha2::{Digest, Sha256};

pub const SIZE: u32 = 128;
pub const MAX_BYTES: usize = 100 * 1024; // 128×128 RGBA sin comprimir ocupa 64 KB; margen para el peor caso de PNG

fn own_path() -> std::path::PathBuf { paths::root().join("foto-perfil.png") }
fn peer_path(hash: &str) -> std::path::PathBuf { paths::cache().join("fotos").join(format!("{hash}.png")) }

pub fn valid_hash(h: &str) -> bool { h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) }

fn check(png: &[u8]) -> AppResult<()> {
    if png.len() > MAX_BYTES { return Err(AppError::invalid("La foto es demasiado grande.")); }
    if crate::logo::png_size(png)? != (SIZE, SIZE) { return Err(AppError::invalid("La foto tiene que ser de 128×128.")); }
    Ok(())
}

pub fn set(png: &[u8]) -> AppResult<()> { check(png)?; paths::write_atomic(&own_path(), png) }
pub fn clear() -> AppResult<()> { let _ = std::fs::remove_file(own_path()); Ok(()) }
pub fn own_png() -> Option<Vec<u8>> { std::fs::read(own_path()).ok().filter(|b| check(b).is_ok()) }
pub fn own_hash() -> Option<String> { own_png().map(|b| hex::encode(Sha256::digest(&b))) }

/// Guarda la foto que mandó otra PC, solo si coincide con la huella que anunció.
pub fn store_peer(hash: &str, png: &[u8]) -> AppResult<()> {
    if !valid_hash(hash) || hex::encode(Sha256::digest(png)) != hash { return Err(AppError::invalid("La foto recibida no coincide con su huella.")); }
    check(png)?;
    let p = peer_path(hash);
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| AppError::io("No se pudo guardar la foto", e))?;
    paths::write_atomic(&p, png)
}
pub fn has_peer(hash: &str) -> bool { valid_hash(hash) && peer_path(hash).is_file() }

/// `data:` de una foto (propia o de otra PC) por su huella.
pub fn data_url(hash: &str) -> Option<String> {
    if !valid_hash(hash) { return None; }
    let b = if own_hash().as_deref() == Some(hash) { own_png()? } else { std::fs::read(peer_path(hash)).ok()? };
    (hex::encode(Sha256::digest(&b)) == hash).then(|| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR"); b.extend_from_slice(&w.to_be_bytes()); b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(&[0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]);
        b
    }
    #[test]
    fn foto_valida_huella_y_ajena() {
        let _ = paths::init(std::env::temp_dir().join("939host-avatar-tests"));
        assert!(set(&png(64, 64)).is_err(), "tamaño equivocado");
        assert!(set(b"no es png").is_err());
        let p = png(128, 128);
        set(&p).unwrap();
        let h = own_hash().unwrap();
        assert!(data_url(&h).unwrap().starts_with("data:image/png;base64,"));
        // Foto de otra PC: solo se acepta si coincide con la huella anunciada.
        let mut other = png(128, 128); other[30] = 7;
        let oh = hex::encode(Sha256::digest(&other));
        assert!(store_peer(&h, &other).is_err(), "huella que no coincide");
        assert!(store_peer("../../x", &other).is_err());
        store_peer(&oh, &other).unwrap();
        assert!(has_peer(&oh) && data_url(&oh).is_some());
        clear().unwrap();
        assert!(own_hash().is_none());
    }
}
