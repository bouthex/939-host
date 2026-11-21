//! Cambiar vida, hambre, saturación y nivel de experiencia de un jugador, con lo que Minecraft
//! permite de verdad en cada caso:
//!
//! - **Desconectado**: se edita su archivo de datos (`players/data/<uuid>.dat` o
//!   `playerdata/<uuid>.dat`) escribiendo encima de esos valores y nada más (ver
//!   `nbt::patch_root`). Antes se guarda una copia (`<uuid>.dat.antes-939host`). Minecraft lee
//!   el archivo cuando el jugador vuelve a entrar.
//! - **Conectado**: el servidor tiene los datos en memoria y pisaría el archivo, así que se usan
//!   comandos: vida exacta con efecto de curación + `/damage` (Minecraft 1.19.4+), hambre solo
//!   «llenar» (efecto de saturación: no existe un comando para un valor exacto) y experiencia con
//!   `/xp set`. Después se vuelve a leer el jugador y se informa el valor real que quedó.
//!
//! Quién puede: lo decide quien llama (en la app, solo la PC propietaria). Cada cambio queda en
//! el registro de auditoría del servidor.

use crate::error::{AppError, AppResult};
use crate::model::ServerConfig;
use crate::nbt::{self, Num};
use crate::paths;
use crate::playerdata;
use crate::process::Manager;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub health: Option<f64>,
    pub food: Option<i64>,
    pub saturation: Option<f64>,
    pub xp_level: Option<i64>,
}

impl Edit {
    pub fn is_empty(&self) -> bool { self.health.is_none() && self.food.is_none() && self.saturation.is_none() && self.xp_level.is_none() }
}

/// Vida máxima del jugador según sus atributos (20 si no tiene el atributo cambiado).
pub fn max_health(v: &serde_json::Value) -> f64 {
    let lists = [v.get("attributes"), v.get("Attributes")];
    for l in lists.into_iter().flatten().filter_map(|l| l.as_array()) {
        for a in l {
            let id = a.get("id").or_else(|| a.get("Name")).and_then(|x| x.as_str()).unwrap_or("");
            if id.ends_with("max_health") {
                if let Some(b) = a.get("base").or_else(|| a.get("Base")).and_then(|x| x.as_f64()) { return b.clamp(1.0, 1024.0); }
            }
        }
    }
    20.0
}

/// Comprueba rangos. `max_hp` es la vida máxima del jugador.
pub fn validate(e: &Edit, max_hp: f64) -> AppResult<()> {
    if e.is_empty() { return Err(AppError::invalid("No hay cambios para aplicar.")); }
    if let Some(h) = e.health { if !(h.is_finite() && h >= 0.5 && h <= max_hp) { return Err(AppError::invalid(&format!("La vida tiene que estar entre 0,5 y {} (corazones × 2).", fmt(max_hp)))); } }
    if let Some(f) = e.food { if !(0..=20).contains(&f) { return Err(AppError::invalid("El hambre va de 0 a 20.")); } }
    if let Some(s) = e.saturation { if !(s.is_finite() && (0.0..=20.0).contains(&s)) { return Err(AppError::invalid("La saturación va de 0 a 20.")); } }
    if let (Some(s), Some(f)) = (e.saturation, e.food) { if s > f as f64 { return Err(AppError::invalid("Minecraft no deja la saturación por encima del hambre.")); } }
    if let Some(x) = e.xp_level { if !(0..=21863).contains(&x) { return Err(AppError::invalid("El nivel de experiencia va de 0 a 21863.")); } }
    Ok(())
}

fn fmt(x: f64) -> String { if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}").replace('.', ",") } }

/// Versión «1.X.Y» ≥ dada. Las numeraciones nuevas (26.x) cuentan como más nuevas; las
/// instantáneas, por las dudas, no.
pub fn release_at_least(mc: &str, minor: u32, patch: u32) -> bool {
    let mc = mc.trim();
    if let Some(rest) = mc.strip_prefix("1.") {
        let n: Vec<u32> = rest.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(2).map(|p| p.parse().unwrap_or(0)).collect();
        return (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0)) >= (minor, patch);
    }
    mc.split('.').next().and_then(|n| n.parse::<u32>().ok()).map(|n| n >= 20 && n < 1000).unwrap_or(false)
}

/// Qué se puede editar con el jugador conectado en esta versión.
pub fn online_support(mc: &str) -> (bool, bool, bool) {
    // (vida exacta, llenar hambre, nivel de experiencia)
    (release_at_least(mc, 19, 4), release_at_least(mc, 13, 0), release_at_least(mc, 13, 0))
}

pub fn audit(server: &str, what: &str, who: &str) {
    use std::io::Write;
    let p = paths::server_dir(server).join("audit.jsonl");
    if std::fs::metadata(&p).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) { let _ = std::fs::rename(&p, p.with_extension("jsonl.1")); }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let line = serde_json::json!({ "at": chrono::Utc::now().to_rfc3339(), "what": what, "who": who });
        let _ = writeln!(f, "{line}");
    }
}

fn describe(e: &Edit) -> String {
    let mut v = vec![];
    if let Some(h) = e.health { v.push(format!("vida {}", fmt(h))); }
    if let Some(f) = e.food { v.push(format!("hambre {f}")); }
    if let Some(s) = e.saturation { v.push(format!("saturación {}", fmt(s))); }
    if let Some(x) = e.xp_level { v.push(format!("nivel {x}")); }
    v.join(", ")
}

/// Edición con el jugador desconectado. Devuelve el texto para mostrar.
pub fn apply_offline(mgr: &Manager, cfg: &ServerConfig, name: &str, uuid: &str, e: &Edit, who: &str) -> AppResult<String> {
    if !crate::players::valid_name(name) || uuid.len() != 36 || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(AppError::invalid("Jugador inválido."));
    }
    if mgr.runtime(&cfg.id).lock().unwrap().players.contains(name) {
        return Err(AppError::invalid(&format!("{name} acaba de entrar: probá de nuevo para cambiarlo en vivo.")));
    }
    let file = playerdata::player_file(&playerdata::world_dir(cfg), "data", uuid, "dat");
    let raw = std::fs::read(&file).map_err(|_| AppError::invalid(&format!("{name} todavía no tiene datos guardados en este mundo.")))?;
    let gz = raw.starts_with(&[0x1f, 0x8b]);
    let data = if gz {
        let mut d = flate2::read::GzDecoder::new(&raw[..]);
        let mut out = vec![];
        std::io::Read::read_to_end(&mut d, &mut out).map_err(|e| AppError::invalid(&format!("El archivo del jugador está dañado: {e}")))?;
        out
    } else { raw.clone() };
    let v = nbt::parse_nbt(&data).map_err(|e| AppError::invalid(&format!("El archivo del jugador está dañado: {e}")))?;
    validate(e, max_health(&v))?;
    let mut edits = vec![];
    if let Some(h) = e.health { edits.push(("Health", Num::Float(h as f32))); }
    if let Some(f) = e.food { edits.push(("foodLevel", Num::Int(f as i32))); }
    if let Some(x) = e.xp_level { edits.push(("XpLevel", Num::Int(x as i32))); }
    // La saturación no puede quedar por encima del hambre final.
    let food_after = e.food.or_else(|| v.get("foodLevel").and_then(|x| x.as_i64())).unwrap_or(20);
    if let Some(s) = e.saturation {
        if s > food_after as f64 { return Err(AppError::invalid("Minecraft no deja la saturación por encima del hambre.")); }
        edits.push(("foodSaturationLevel", Num::Float(s as f32)));
    } else if e.food.is_some() {
        if let Some(cur) = v.get("foodSaturationLevel").and_then(|x| x.as_f64()) { if cur > food_after as f64 { edits.push(("foodSaturationLevel", Num::Float(food_after as f32))); } }
    }
    let patched = nbt::patch_root(&data, &edits).map_err(|m| AppError::invalid(&format!("No se pudo cambiar: {m}")))?;
    // Comprobación: lo escrito se vuelve a leer igual.
    let check = nbt::parse_nbt(&patched).map_err(|m| AppError::invalid(&format!("No se pudo cambiar: {m}")))?;
    if let Some(h) = e.health { if (check["Health"].as_f64().unwrap_or(-1.0) - h).abs() > 0.01 { return Err(AppError::invalid("La comprobación del cambio falló; no se guardó nada.")); } }
    let out = if gz {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
        enc.write_all(&patched).map_err(|e| AppError::io("No se pudo preparar el archivo", e))?;
        enc.finish().map_err(|e| AppError::io("No se pudo preparar el archivo", e))?
    } else { patched };
    let backup = file.with_extension("dat.antes-939host");
    std::fs::write(&backup, &raw).map_err(|e| AppError::io("No se pudo guardar la copia previa", e))?;
    paths::write_atomic(&file, &out)?;
    audit(&cfg.id, &format!("jugador-editado {name} (desconectado): {}", describe(e)), who);
    Ok(format!("Listo: {} para {name}. Lo va a ver la próxima vez que entre.", describe(e)))
}

/// Edición con el jugador conectado (comandos). Devuelve el texto para mostrar.
pub fn apply_online(mgr: &Manager, cfg: &ServerConfig, name: &str, e: &Edit, who: &str) -> AppResult<String> {
    if !crate::players::valid_name(name) { return Err(AppError::invalid("Jugador inválido.")); }
    let (hp_ok, feed_ok, xp_ok) = online_support(&cfg.mc_version);
    let read = || -> AppResult<serde_json::Value> {
        let line = mgr.query(&cfg.id, &format!("data get entity {name}"), &[&format!("{name} has the following entity data"), "No entity was found"], Duration::from_secs(4))?;
        playerdata::parse_reply(&line, name)
    };
    let before = read()?;
    let max_hp = max_health(&before);
    validate(e, max_hp)?;
    if e.saturation.is_some() || e.food.map(|f| f != 20).unwrap_or(false) {
        return Err(AppError::invalid("Con el jugador conectado Minecraft solo permite llenar el hambre (y la saturación). Para un valor exacto, cambialo cuando esté desconectado."));
    }
    if e.health.is_some() && !hp_ok { return Err(AppError::invalid("Cambiar la vida con el jugador conectado necesita Minecraft 1.19.4 o más nuevo. Hacelo cuando esté desconectado.")); }
    if (e.food.is_some() && !feed_ok) || (e.xp_level.is_some() && !xp_ok) { return Err(AppError::invalid("Esta versión de Minecraft no tiene los comandos necesarios. Hacelo con el jugador desconectado.")); }
    let send = |c: &str| mgr.send_command(&cfg.id, c);
    if let Some(h) = e.health {
        // Si hay que subir la vida: curación (Minecraft la aplica en el tick siguiente, así que se
        // espera y se vuelve a leer). Después se quita lo que sobra con un daño que no reducen la
        // armadura ni los efectos.
        let mut cur = before.get("Health").and_then(|x| x.as_f64()).unwrap_or(max_hp);
        if h > cur + 0.01 {
            send(&format!("effect give {name} minecraft:instant_health 1 28 true"))?;
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(150));
                if let Some(v) = read().ok().and_then(|v| v.get("Health").and_then(|x| x.as_f64())) { cur = v; if cur >= h - 0.01 { break; } }
            }
        }
        let diff = cur - h;
        if diff > 0.01 { send(&format!("damage {name} {:.2} minecraft:generic_kill", diff))?; }
    }
    if e.food == Some(20) { send(&format!("effect give {name} minecraft:saturation 1 255 true"))?; }
    if let Some(x) = e.xp_level { send(&format!("xp set {name} {x} levels"))?; }
    std::thread::sleep(Duration::from_millis(400));
    let after = read().ok();
    let got_hp = after.as_ref().and_then(|v| v.get("Health")).and_then(|x| x.as_f64());
    let mut msg = format!("Listo para {name}");
    if let (Some(want), Some(got)) = (e.health, got_hp) {
        if (want - got).abs() > 0.25 { msg = format!("Se aplicó, pero {name} quedó con {} de vida (pedido {}): sus efectos o su absorción lo cambian.", fmt(got), fmt(want)); }
    }
    audit(&cfg.id, &format!("jugador-editado {name} (conectado): {}", describe(e)), who);
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rangos_y_versiones() {
        assert!(validate(&Edit::default(), 20.0).is_err());
        assert!(validate(&Edit { health: Some(20.0), ..Default::default() }, 20.0).is_ok());
        assert!(validate(&Edit { health: Some(0.0), ..Default::default() }, 20.0).is_err(), "nunca matar");
        assert!(validate(&Edit { health: Some(30.0), ..Default::default() }, 20.0).is_err());
        assert!(validate(&Edit { health: Some(30.0), ..Default::default() }, 40.0).is_ok(), "vida máxima de un mod");
        assert!(validate(&Edit { health: Some(f64::NAN), ..Default::default() }, 20.0).is_err());
        assert!(validate(&Edit { food: Some(21), ..Default::default() }, 20.0).is_err());
        assert!(validate(&Edit { food: Some(5), saturation: Some(6.0), ..Default::default() }, 20.0).is_err());
        assert!(validate(&Edit { xp_level: Some(-1), ..Default::default() }, 20.0).is_err());
        assert!(release_at_least("1.19.4", 19, 4) && release_at_least("1.21.11", 19, 4) && release_at_least("26.2", 19, 4));
        assert!(!release_at_least("1.19.3", 19, 4) && !release_at_least("1.12.2", 13, 0) && !release_at_least("24w14a", 19, 4));
        assert_eq!(max_health(&serde_json::json!({"attributes": [{"id": "minecraft:max_health", "base": 40.0}]})), 40.0);
        assert_eq!(max_health(&serde_json::json!({"Attributes": [{"Name": "generic.max_health", "Base": 24.0}]})), 24.0);
        assert_eq!(max_health(&serde_json::json!({})), 20.0);
    }
}
