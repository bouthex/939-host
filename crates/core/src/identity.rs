//! Identidades criptográficas: la del nodo (esta PC) y la de cada servidor lógico.
//! Las claves privadas se guardan cifradas con DPAPI de Windows (solo este usuario puede leerlas).

use crate::error::{AppError, AppResult};
use crate::model::ServerIdentity;
use crate::paths;
use data_encoding::BASE32_NOPAD;
use ed25519_dalek::SigningKey;
use serde::Serialize;
use sha2::Digest;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIdentity {
    pub node_id: String,
    pub short_id: String,
}

fn random_key() -> AppResult<SigningKey> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| AppError::new("crypto", "No se pudo generar una clave segura.").with_detail(e))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn short(pubkey: &[u8]) -> String {
    let digest = sha2::Sha256::digest(pubkey);
    BASE32_NOPAD.encode(&digest)[..10].to_string()
}

pub fn load_or_create_node() -> AppResult<NodeIdentity> {
    let path = paths::identity().join("node.key");
    let key = match std::fs::read(&path) {
        Ok(blob) => {
            let raw = unprotect(&blob)?;
            let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| AppError::new("crypto", "La identidad de esta PC está dañada."))?;
            SigningKey::from_bytes(&arr)
        }
        Err(_) => {
            let k = random_key()?;
            paths::write_atomic(&path, &protect(k.as_bytes())?)?;
            k
        }
    };
    let public = key.verifying_key().to_bytes();
    Ok(NodeIdentity { node_id: hex::encode(public), short_id: short(&public) })
}

/// Clave privada del nodo (para el endpoint P2P). Nunca sale de esta PC.
pub fn node_secret() -> AppResult<[u8; 32]> {
    let blob = std::fs::read(paths::identity().join("node.key")).map_err(|e| AppError::io("No se encontró la identidad de esta PC", e))?;
    let raw = unprotect(&blob)?;
    raw.as_slice().try_into().map_err(|_| AppError::new("crypto", "La identidad de esta PC está dañada."))
}

/// Crea la identidad permanente de un servidor lógico y su código de acceso.
pub fn create_server_identity(server_id: &str, name: &str) -> AppResult<ServerIdentity> {
    let key = random_key()?;
    let path = paths::identity().join("servers").join(format!("{server_id}.key"));
    paths::write_atomic(&path, &protect(key.as_bytes())?)?;
    let public = key.verifying_key().to_bytes();
    Ok(ServerIdentity { public_key: hex::encode(public), access_code: access_code(name, &public) })
}

/// Código legible: NOMBRE · XXXX-XXXX-XX. La segunda parte es una huella de la clave pública,
/// así nadie puede presentar otro servidor con el mismo nombre.
pub fn access_code(name: &str, public: &[u8]) -> String {
    let slug: String = name
        .to_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(20).collect();
    let fp = short(public);
    format!("{} · {}-{}-{}", if slug.is_empty() { "939" } else { &slug }, &fp[0..4], &fp[4..8], &fp[8..10])
}

#[cfg(windows)]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> {
    dpapi(data, true)
}
#[cfg(windows)]
fn unprotect(data: &[u8]) -> AppResult<Vec<u8>> {
    dpapi(data, false)
}

#[cfg(windows)]
fn dpapi(data: &[u8], encrypt: bool) -> AppResult<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: los punteros son válidos durante la llamada y la salida se libera con LocalFree.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut output)
        }
    };
    if ok == 0 {
        return Err(AppError::new("crypto", "Windows no pudo proteger o leer una clave de 939 Host."));
    }
    let out = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData as _) };
    Ok(out)
}

#[cfg(not(windows))]
fn protect(data: &[u8]) -> AppResult<Vec<u8>> { Ok(data.to_vec()) }
#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> AppResult<Vec<u8>> { Ok(data.to_vec()) }
