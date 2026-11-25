//! Skins reales de los jugadores, desde los servidores oficiales de Mojang.
//!
//! Solo se usa si el usuario lo permite (Configuración → Privacidad y conexiones). Se pide el
//! perfil público del jugador, se toma la URL de su skin (siempre de textures.minecraft.net) y se
//! guarda una copia en caché por 24 horas. Si algo falla, la interfaz muestra iniciales.

use crate::error::{AppError, AppResult};
use crate::net;
use crate::paths;
use serde_json::Value;
use std::time::{Duration, SystemTime};

const TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_PNG: usize = 128 * 1024;

fn cache_file(key: &str) -> std::path::PathBuf { paths::cache().join("skins").join(format!("{key}.png")) }

/// La URL de la textura tiene que ser de Mojang y por HTTPS.
pub fn valid_texture_url(url: &str) -> bool {
    url.strip_prefix("https://textures.minecraft.net/texture/").or_else(|| url.strip_prefix("http://textures.minecraft.net/texture/"))
        .map(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or(false)
}

/// Extrae la URL de la skin del perfil de sessionserver.
pub fn skin_url_from_profile(profile: &Value) -> Option<String> {
    let props = profile.get("properties")?.as_array()?;
    let tex = props.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some("textures"))?;
    let raw = tex.get("value")?.as_str()?;
    let bytes = data_encoding::BASE64.decode(raw.as_bytes()).or_else(|_| data_encoding::BASE64_NOPAD.decode(raw.trim_end_matches('=').as_bytes())).ok()?;
    let j: Value = serde_json::from_slice(&bytes).ok()?;
    let url = j.get("textures")?.get("SKIN")?.get("url")?.as_str()?;
    valid_texture_url(url).then(|| url.replacen("http://", "https://", 1))
}

async fn fetch(name: &str) -> AppResult<Vec<u8>> {
    let id: Value = net::get_json(&format!("https://api.mojang.com/users/profiles/minecraft/{name}"), "Mojang").await?;
    let uuid = id.get("id").and_then(|x| x.as_str()).filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| AppError::not_found("la cuenta"))?;
    let profile: Value = net::get_json(&format!("https://sessionserver.mojang.com/session/minecraft/profile/{uuid}"), "Mojang").await?;
    let url = skin_url_from_profile(&profile).ok_or_else(|| AppError::not_found("la skin"))?;
    let resp = net::HTTP.get(&url).send().await.map_err(|e| AppError::network("No se pudo descargar la skin", e))?;
    if !resp.status().is_success() { return Err(AppError::network("Mojang respondió con un error", resp.status())); }
    let bytes = resp.bytes().await.map_err(|e| AppError::network("No se pudo descargar la skin", e))?;
    if bytes.len() > MAX_PNG || !bytes.starts_with(&[0x89, b'P', b'N', b'G']) { return Err(AppError::invalid("La skin recibida no es válida.")); }
    Ok(bytes.to_vec())
}

/// PNG de la skin (64×64) del jugador, desde la caché o desde Mojang.
pub async fn skin_png(name: &str) -> AppResult<Vec<u8>> {
    if !crate::players::valid_name(name) { return Err(AppError::invalid("Nombre de jugador inválido.")); }
    let key = name.to_ascii_lowercase();
    let file = cache_file(&key);
    if let Ok(meta) = std::fs::metadata(&file) {
        let fresh = meta.modified().ok().and_then(|t| SystemTime::now().duration_since(t).ok()).map(|age| age < TTL).unwrap_or(false);
        if fresh { if let Ok(b) = std::fs::read(&file) { return Ok(b); } }
    }
    match fetch(name).await {
        Ok(png) => {
            let _ = std::fs::create_dir_all(file.parent().unwrap());
            let _ = paths::write_atomic(&file, &png);
            Ok(png)
        }
        // Sin conexión: una copia vieja es mejor que nada.
        Err(e) => std::fs::read(&file).map_err(|_| e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_validates_skin_url() {
        let tex = r#"{"timestamp":1,"profileId":"x","profileName":"jeb_","textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/7fd9ba42a7c81eeea22f1524271ae85a8e045ce0af5a6ae16c6406ae917e68b5"}}}"#;
        let profile = serde_json::json!({"properties": [{"name": "textures", "value": data_encoding::BASE64.encode(tex.as_bytes())}]});
        assert_eq!(skin_url_from_profile(&profile).as_deref(), Some("https://textures.minecraft.net/texture/7fd9ba42a7c81eeea22f1524271ae85a8e045ce0af5a6ae16c6406ae917e68b5"));
        let evil = r#"{"textures":{"SKIN":{"url":"https://evil.example/texture/abc"}}}"#;
        let profile = serde_json::json!({"properties": [{"name": "textures", "value": data_encoding::BASE64.encode(evil.as_bytes())}]});
        assert_eq!(skin_url_from_profile(&profile), None);
        assert!(!valid_texture_url("https://textures.minecraft.net/texture/../x"));
    }
}

/// `data:image/png;base64,…` para mostrar una imagen en la interfaz sin servir archivos.
pub fn data_url_png(bytes: &[u8]) -> String { format!("data:image/png;base64,{}", data_encoding::BASE64.encode(bytes)) }
