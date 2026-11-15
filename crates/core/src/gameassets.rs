//! Imágenes originales de Minecraft (ítems, bloques, corazones, hambre, armadura, efectos) y los
//! nombres de los objetos en español.
//!
//! No se distribuyen con 939 Host: el EULA de Minecraft no lo permite. Con el permiso del usuario
//! (Configuración → Privacidad y conexiones), se descarga una sola vez por versión el cliente
//! oficial desde los servidores de Mojang, se verifica su SHA-1, se extraen solo las imágenes que
//! usa la interfaz y se borra el archivo. Todo queda en la caché local de esta PC.

use crate::error::{AppError, AppResult};
use crate::meta;
use crate::net;
use crate::paths;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const PREFIXES: &[&str] = &[
    "assets/minecraft/textures/item/",
    "assets/minecraft/textures/block/",
    "assets/minecraft/textures/gui/sprites/hud/",
    "assets/minecraft/textures/gui/sprites/container/slot/",
    "assets/minecraft/textures/mob_effect/",
];

static LOCKS: Lazy<Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = Lazy::new(Default::default);

fn valid_version(v: &str) -> bool { !v.is_empty() && v.len() <= 40 && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) }

pub fn dir(version: &str) -> PathBuf { paths::cache().join("game-assets").join(version) }

pub fn ready(version: &str) -> bool { valid_version(version) && dir(version).join(".completo").exists() }

/// Extrae del jar del cliente solo las imágenes que usa la interfaz.
pub fn extract(jar: &Path, out: &Path) -> AppResult<usize> {
    let f = std::fs::File::open(jar).map_err(|e| AppError::io("No se pudo abrir el archivo del juego", e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| AppError::new("zip", format!("Archivo del juego dañado: {e}")))?;
    let mut n = 0;
    for i in 0..z.len() {
        let mut e = match z.by_index(i) { Ok(e) => e, Err(_) => continue };
        let name = e.name().to_string();
        let Some(prefix) = PREFIXES.iter().find(|p| name.starts_with(**p)) else { continue };
        if !name.ends_with(".png") || name.contains("..") || e.size() > 512 * 1024 { continue; }
        let rel = &name["assets/minecraft/textures/".len()..];
        let _ = prefix;
        let dest = out.join(rel);
        if let Some(p) = dest.parent() { std::fs::create_dir_all(p).map_err(|e| AppError::io("No se pudo crear la caché", e))?; }
        let mut buf = vec![];
        e.read_to_end(&mut buf).map_err(|e| AppError::io("No se pudo leer el archivo del juego", e))?;
        std::fs::write(&dest, &buf).map_err(|e| AppError::io("No se pudo guardar la imagen", e))?;
        n += 1;
    }
    Ok(n)
}

/// Descarga y prepara las imágenes de una versión (una sola vez).
pub async fn ensure(version: &str, mut progress: impl FnMut(&str, f64)) -> AppResult<()> {
    if !valid_version(version) { return Err(AppError::invalid("Versión inválida.")); }
    if ready(version) { return Ok(()); }
    let lock = LOCKS.lock().unwrap().entry(version.to_string()).or_default().clone();
    let _g = lock.lock().await;
    if ready(version) { return Ok(()); }
    let out = dir(version);
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).map_err(|e| AppError::io("No se pudo crear la caché", e))?;

    progress("Consultando a Mojang", 0.02);
    let v = meta::version_json(version).await?;
    let client = v.pointer("/downloads/client").ok_or_else(|| AppError::invalid("Mojang no publica el cliente de esta versión."))?;
    let url = client.get("url").and_then(|x| x.as_str()).ok_or_else(|| AppError::invalid("Datos de versión incompletos"))?;
    let sha1 = client.get("sha1").and_then(|x| x.as_str()).ok_or_else(|| AppError::invalid("Datos de versión incompletos"))?;
    if !url.starts_with("https://piston-data.mojang.com/") && !url.starts_with("https://launcher.mojang.com/") {
        return Err(AppError::invalid("La descarga no es de un servidor de Mojang."));
    }
    let jar = paths::cache().join(format!("cliente-{version}.jar.part"));
    net::download(url, &jar, net::Hash::Sha1(sha1), |done, total| {
        if let Some(t) = total { progress("Descargando imágenes del juego", 0.05 + 0.8 * done as f64 / t.max(1) as f64); }
    }).await?;
    progress("Extrayendo imágenes", 0.88);
    let res = extract(&jar, &out);
    let _ = std::fs::remove_file(&jar);
    res?;

    // Nombres en español desde el índice oficial de recursos (opcional: si falla, se usa inglés
    // o el identificador).
    progress("Nombres de los objetos", 0.94);
    if let Some(idx_url) = v.pointer("/assetIndex/url").and_then(|x| x.as_str()) {
        if idx_url.starts_with("https://piston-meta.mojang.com/") || idx_url.starts_with("https://launchermeta.mojang.com/") {
            if let Ok(idx) = net::get_json::<serde_json::Value>(idx_url, "el índice de recursos").await {
                if let Some(hash) = idx.pointer("/objects/minecraft~1lang~1es_es.json/hash").and_then(|h| h.as_str()) {
                    if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                        let url = format!("https://resources.download.minecraft.net/{}/{}", &hash[..2], hash);
                        let dest = out.join("es_es.json");
                        let _ = net::download(&url, &dest, net::Hash::Sha1(hash), |_, _| {}).await;
                    }
                }
            }
        }
    }
    std::fs::write(out.join(".completo"), version).map_err(|e| AppError::io("No se pudo guardar la caché", e))?;
    progress("Listo", 1.0);
    Ok(())
}

/// Ruta de la imagen para una clave: `item:minecraft:diamond_sword`, `hud:heart/full`,
/// `effect:minecraft:speed`, `slot:helmet`.
pub fn resolve(version: &str, key: &str) -> Option<PathBuf> {
    if !ready(version) { return None; }
    let base = dir(version);
    let (kind, name) = key.split_once(':')?;
    let name = name.trim_start_matches("minecraft:");
    if name.is_empty() || name.contains("..") || name.contains('\\') { return None; }
    let candidates: Vec<String> = match kind {
        "item" => {
            if name.contains('/') { return None; }
            vec![format!("item/{name}.png"), format!("item/{name}_00.png"), format!("item/{name}_standby.png"), format!("block/{name}.png"), format!("block/{name}_front.png"), format!("block/{name}_side.png"), format!("block/{name}_top.png"), format!("block/{name}_0.png")]
        }
        "hud" => vec![format!("gui/sprites/hud/{name}.png")],
        "effect" => { if name.contains('/') { return None; } vec![format!("mob_effect/{name}.png")] }
        "slot" => { if name.contains('/') { return None; } vec![format!("gui/sprites/container/slot/{name}.png")] }
        _ => return None,
    };
    candidates.into_iter().map(|c| base.join(c)).find(|p| p.is_file())
}

/// Nombres en español: `item.minecraft.diamond_sword` → «Espada de diamante».
pub fn names(version: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if !ready(version) { return out; }
    let Ok(text) = std::fs::read_to_string(dir(version).join("es_es.json")) else { return out };
    let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&text) else { return out };
    for (k, v) in map {
        for pre in ["item.minecraft.", "block.minecraft.", "effect.minecraft.", "enchantment.minecraft."] {
            if let Some(id) = k.strip_prefix(pre) {
                if !id.contains('.') {
                    let key = format!("{}:{}", pre.split('.').next().unwrap(), id);
                    out.insert(key, v.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_only_ui_images_and_resolves_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let jar = tmp.path().join("c.jar");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        let o: zip::write::SimpleFileOptions = Default::default();
        for f in ["assets/minecraft/textures/item/diamond_sword.png", "assets/minecraft/textures/block/stone.png", "assets/minecraft/textures/gui/sprites/hud/heart/full.png", "assets/minecraft/textures/entity/zombie.png", "net/minecraft/Main.class", "assets/minecraft/textures/item/../../evil.png"] {
            z.start_file(f, o).unwrap();
            z.write_all(b"\x89PNG").unwrap();
        }
        z.finish().unwrap();
        let out = tmp.path().join("out");
        assert_eq!(extract(&jar, &out).unwrap(), 3);
        assert!(out.join("item/diamond_sword.png").exists());
        assert!(!out.join("entity").exists());
    }
}
