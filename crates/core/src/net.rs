//! Cliente HTTP y descargas verificadas por hash. Nada descargado se usa sin verificar.

use crate::error::{AppError, AppResult};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use sha1::Digest;
use std::path::Path;
use std::time::Duration;

pub static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("939Host/", env!("CARGO_PKG_VERSION"), " (servidor de Minecraft propio)"))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .expect("cliente HTTP")
});

pub async fn get_json<T: serde::de::DeserializeOwned>(url: &str, what: &str) -> AppResult<T> {
    let resp = HTTP.get(url).send().await.map_err(|e| AppError::network(&format!("No se pudo consultar {what}"), e))?;
    if !resp.status().is_success() {
        return Err(AppError::network(&format!("{what} respondió con un error"), resp.status()));
    }
    resp.json::<T>().await.map_err(|e| AppError::network(&format!("La respuesta de {what} no tiene el formato esperado"), e))
}

#[derive(Clone, Copy)]
pub enum Hash<'a> {
    Sha1(&'a str),
    Sha256(&'a str),
    None,
}

pub async fn get_text(url: &str, what: &str) -> AppResult<String> {
    let resp = HTTP.get(url).send().await.map_err(|e| AppError::network(&format!("No se pudo consultar {what}"), e))?;
    if !resp.status().is_success() {
        return Err(AppError::network(&format!("{what} respondió con un error"), resp.status()));
    }
    resp.text().await.map_err(|e| AppError::network(&format!("No se pudo leer {what}"), e))
}

/// Descarga a `dest` pasando por un archivo temporal, verifica el hash y recién entonces renombra.
pub async fn download<F: FnMut(u64, Option<u64>)>(url: &str, dest: &Path, hash: Hash<'_>, mut progress: F) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io("No se pudo crear la carpeta de descarga", e))?;
    }
    let resp = HTTP.get(url).send().await.map_err(|e| AppError::network("No se pudo iniciar la descarga", e))?;
    if !resp.status().is_success() {
        return Err(AppError::network("El servidor de descarga respondió con un error", resp.status()));
    }
    let total = resp.content_length();
    let tmp = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&tmp).await.map_err(|e| AppError::io("No se pudo crear el archivo temporal", e))?;
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::network("La descarga se cortó", e))?;
        match hash {
            Hash::Sha1(_) => sha1.update(&chunk),
            Hash::Sha256(_) => sha256.update(&chunk),
            Hash::None => {}
        }
        file.write_all(&chunk).await.map_err(|e| AppError::io("No se pudo escribir la descarga", e))?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await.map_err(|e| AppError::io("No se pudo escribir la descarga", e))?;
    drop(file);
    let ok = match hash {
        Hash::Sha1(expected) => hex::encode(sha1.finalize()).eq_ignore_ascii_case(expected),
        Hash::Sha256(expected) => hex::encode(sha256.finalize()).eq_ignore_ascii_case(expected),
        Hash::None => true,
    };
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::new("integrity", "El archivo descargado no coincide con su firma de verificación y se descartó por seguridad. Intentá de nuevo."));
    }
    std::fs::rename(&tmp, dest).map_err(|e| AppError::io("No se pudo guardar la descarga", e))?;
    Ok(())
}

pub fn sha1_bytes(b: &[u8]) -> String { use sha1::Digest; hex::encode(sha1::Sha1::digest(b)) }

pub fn sha1_file(path: &Path) -> AppResult<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| AppError::io("No se pudo abrir el archivo", e))?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| AppError::io("No se pudo leer el archivo", e))?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
