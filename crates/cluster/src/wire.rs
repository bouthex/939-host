//! Mensajes con longitud prefijada sobre streams QUIC.

use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_MSG: usize = 8 * 1024 * 1024;

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, bytes: &[u8]) -> Result<()> {
    w.write_u32(bytes.len() as u32).await?;
    w.write_all(bytes).await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R, max: usize) -> Result<Vec<u8>> {
    let len = r.read_u32().await? as usize;
    if len > max { bail!("mensaje demasiado grande ({len} bytes)"); }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

pub async fn write_msg<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, v: &T) -> Result<()> {
    write_frame(w, &serde_json::to_vec(v)?).await
}

/// Pedidos entrantes (lo que manda otra PC antes de saber si es del grupo): mucho más chico que
/// una respuesta, para que nadie pueda hacer reservar 8 MB por pedido.
pub const MAX_REQ: usize = 512 * 1024;

pub async fn read_req<R: AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> Result<T> {
    Ok(serde_json::from_slice(&read_frame(r, MAX_REQ).await?)?)
}

pub async fn read_msg<R: AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> Result<T> {
    Ok(serde_json::from_slice(&read_frame(r, MAX_MSG).await?)?)
}
