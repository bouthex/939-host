//! Respuesta "el servidor se está mudando".
//!
//! Mientras el servidor cambia de PC (o nadie lo está hosteando) el puerto local del jugador
//! sigue atendiendo: en la lista de servidores se ve un mensaje claro en lugar de "no se pudo
//! conectar", y si alguien intenta entrar recibe una explicación en vez de un error genérico.
//! Habla lo mínimo del protocolo de Minecraft Java: handshake, estado, ping y desconexión de login.

use anyhow::{bail, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

async fn read_varint<R: AsyncRead + Unpin>(r: &mut R) -> Result<i32> {
    let mut value: i32 = 0;
    for i in 0..5 {
        let b = r.read_u8().await?;
        value |= ((b & 0x7f) as i32) << (7 * i);
        if b & 0x80 == 0 { return Ok(value); }
    }
    bail!("VarInt demasiado largo")
}

fn varint(mut v: i32, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v = ((v as u32) >> 7) as i32;
        if v != 0 { b |= 0x80; }
        out.push(b);
        if v == 0 { break; }
    }
}

fn string(s: &str, out: &mut Vec<u8>) {
    varint(s.len() as i32, out);
    out.extend_from_slice(s.as_bytes());
}

async fn read_packet<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let len = read_varint(r).await?;
    if !(1..=32 * 1024).contains(&len) { bail!("paquete inválido"); }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_packet<W: AsyncWrite + Unpin>(w: &mut W, body: &[u8]) -> Result<()> {
    let mut out = vec![];
    varint(body.len() as i32, &mut out);
    out.extend_from_slice(body);
    w.write_all(&out).await?;
    w.flush().await?;
    Ok(())
}

fn parse_varint(buf: &[u8], pos: &mut usize) -> Result<i32> {
    let mut value: i32 = 0;
    for i in 0..5 {
        let Some(&b) = buf.get(*pos) else { bail!("corto") };
        *pos += 1;
        value |= ((b & 0x7f) as i32) << (7 * i);
        if b & 0x80 == 0 { return Ok(value); }
    }
    bail!("VarInt demasiado largo")
}

fn escape(s: &str) -> String { serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()) }

/// Atiende una conexión de Minecraft con el mensaje dado.
pub async fn respond<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, server_name: &str, message: &str) -> Result<()> {
    let hs = read_packet(&mut s).await?;
    let mut pos = 0;
    if parse_varint(&hs, &mut pos)? != 0 { bail!("no es un handshake"); }
    let protocol = parse_varint(&hs, &mut pos)?;
    let addr_len = parse_varint(&hs, &mut pos)? as usize;
    pos += addr_len + 2;
    let next = parse_varint(&hs, &mut pos)?;
    match next {
        1 => {
            let _status_request = read_packet(&mut s).await?;
            let json = format!(
                r#"{{"version":{{"name":"939 Host","protocol":{protocol}}},"players":{{"max":0,"online":0}},"description":{{"text":{}}}}}"#,
                escape(&format!("{server_name}\n§7{message}"))
            );
            let mut body = vec![0x00];
            string(&json, &mut body);
            write_packet(&mut s, &body).await?;
            // Ping → pong (mismo número)
            if let Ok(ping) = read_packet(&mut s).await {
                if ping.first() == Some(&0x01) { write_packet(&mut s, &ping).await?; }
            }
        }
        _ => {
            // Login o transferencia: desconexión con motivo (texto JSON).
            let mut body = vec![0x00];
            string(&format!(r#"{{"text":{}}}"#, escape(&format!("{server_name}: {message}"))), &mut body);
            write_packet(&mut s, &body).await?;
        }
    }
    Ok(())
}

/// Cliente mínimo de estado (Server List Ping) para pruebas y diagnóstico.
pub async fn status<S: AsyncRead + AsyncWrite + Unpin>(mut s: S) -> Result<String> {
    let mut hs = vec![0x00];
    varint(-1, &mut hs);
    string("localhost", &mut hs);
    hs.extend_from_slice(&25565u16.to_be_bytes());
    varint(1, &mut hs);
    write_packet(&mut s, &hs).await?;
    write_packet(&mut s, &[0x00]).await?;
    let resp = read_packet(&mut s).await?;
    let mut pos = 0;
    if parse_varint(&resp, &mut pos)? != 0 { bail!("respuesta inesperada"); }
    let len = parse_varint(&resp, &mut pos)? as usize;
    Ok(String::from_utf8_lossy(&resp[pos..pos + len.min(resp.len() - pos)]).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_and_login_get_a_clear_message() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2 {
                let (c, _) = l.accept().await.unwrap();
                tokio::spawn(respond(c, "Servidor de prueba", "se está mudando de PC, volvé a entrar en unos segundos"));
            }
        });
        let c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let json = status(c).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["description"]["text"].as_str().unwrap().contains("mudando"));

        // Login: debe recibir un paquete de desconexión con el motivo.
        let mut c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut hs = vec![0x00];
        varint(774, &mut hs);
        string("localhost", &mut hs);
        hs.extend_from_slice(&25565u16.to_be_bytes());
        varint(2, &mut hs);
        write_packet(&mut c, &hs).await.unwrap();
        let p = read_packet(&mut c).await.unwrap();
        assert_eq!(p[0], 0x00);
        assert!(String::from_utf8_lossy(&p).contains("mudando"));
    }
}
