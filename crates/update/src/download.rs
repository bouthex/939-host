//! Descarga reanudable y verificada de un archivo cuyo tamaño y SHA-256 vienen en metadatos firmados.
//!
//! - Continúa desde donde quedó (`Range`), y si el servidor no lo admite empieza de nuevo.
//! - Nunca escribe más bytes que los firmados (un servidor no puede llenar el disco).
//! - Solo da el archivo por bueno si el tamaño y el SHA-256 coinciden; si no, lo borra.
//! - HTTPS protege el camino, pero la confianza viene de la firma: un servidor de descargas
//!   comprometido solo puede negar el servicio, no colar otro archivo.

use crate::metadata::Artifact;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, PartialEq)]
pub enum DownloadError {
    Network(String),
    TooLarge,
    Mismatch,
    Io(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Network(e) => write!(f, "no se pudo descargar: {e}"),
            DownloadError::TooLarge => write!(f, "el servidor envió más datos de los firmados"),
            DownloadError::Mismatch => write!(f, "el archivo descargado no coincide con el firmado (se descartó)"),
            DownloadError::Io(e) => write!(f, "error de disco: {e}"),
        }
    }
}

fn io(e: std::io::Error) -> DownloadError { DownloadError::Io(e.to_string()) }

/// Archivo parcial que corresponde a este artefacto (el nombre incluye el SHA-256: no se mezclan
/// descargas de versiones distintas).
pub fn part_path(dir: &Path, a: &Artifact) -> PathBuf { dir.join(format!("{}.{}.part", a.name, &a.sha256[..16])) }

/// Calcula el SHA-256 de un archivo sin cargarlo entero en memoria.
pub async fn file_sha256(path: &Path) -> Result<String, DownloadError> {
    let mut f = tokio::fs::File::open(path).await.map_err(io)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await.map_err(io)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Descarga `url` a `dir`, continuando un parcial si existe. Devuelve la ruta del archivo final
/// verificado. `progress(hechos, total)`.
pub async fn fetch(client: &reqwest::Client, url: &str, dir: &Path, a: &Artifact, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, DownloadError> {
    tokio::fs::create_dir_all(dir).await.map_err(io)?;
    let done_path = dir.join(&a.name);
    if tokio::fs::metadata(&done_path).await.map(|m| m.len() == a.size).unwrap_or(false) && file_sha256(&done_path).await? == a.sha256 {
        return Ok(done_path);
    }
    let part = part_path(dir, a);
    let mut have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
    if have > a.size { let _ = tokio::fs::remove_file(&part).await; have = 0; }
    if have < a.size {
        let mut req = client.get(url);
        if have > 0 { req = req.header(reqwest::header::RANGE, format!("bytes={have}-")); }
        let resp = req.send().await.map_err(|e| DownloadError::Network(e.to_string()))?;
        let status = resp.status();
        let append = status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0;
        if !status.is_success() { return Err(DownloadError::Network(format!("respuesta HTTP {status}"))); }
        if !append { have = 0; }
        let mut f = tokio::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&part).await.map_err(io)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk { Ok(c) => c, Err(e) => { f.flush().await.map_err(io)?; return Err(DownloadError::Network(e.to_string())); } };
            if have + chunk.len() as u64 > a.size {
                drop(f);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(DownloadError::TooLarge);
            }
            f.write_all(&chunk).await.map_err(io)?;
            have += chunk.len() as u64;
            progress(have, a.size);
        }
        f.flush().await.map_err(io)?;
        f.sync_all().await.map_err(io)?;
        if have < a.size { return Err(DownloadError::Network("la descarga se cortó; se continúa la próxima vez".into())); }
    }
    if file_sha256(&part).await? != a.sha256 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(DownloadError::Mismatch);
    }
    tokio::fs::rename(&part, &done_path).await.map_err(io)?;
    Ok(done_path)
}
