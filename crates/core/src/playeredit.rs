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
    // (vida, hambre y saturación, nivel de experiencia)
    (release_at_least(mc, 13, 0), release_at_least(mc, 13, 0), release_at_least(mc, 13, 0))
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
    let (_, feed_ok, xp_ok) = online_support(&cfg.mc_version);
    let hp_ok = release_at_least(&cfg.mc_version, 19, 4);
    let read = || -> AppResult<serde_json::Value> {
        let line = mgr.query(&cfg.id, &format!("data get entity {name}"), &[&format!("{name} has the following entity data"), "No entity was found"], Duration::from_secs(4))?;
        playerdata::parse_reply(&line, name)
    };
    let before = read()?;
    let max_hp = max_health(&before);
    validate(e, max_hp)?;
    let food_change = e.food.is_some() || e.saturation.is_some();
    // Antes de 1.19.4 no existe `/damage`: se baja con veneno (1 punto cada 12 ticks, nunca mata)
    // midiendo la vida real hasta llegar al valor, y se quita el efecto. Comprobado con un
    // servidor 1.16.5 real. El veneno no baja de 1 punto de vida.
    if let (Some(h), false) = (e.health, hp_ok) { if h < 1.0 { return Err(AppError::invalid("En esta versión de Minecraft la vida se puede bajar con el jugador conectado hasta medio corazón (1 punto).")); } }
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
        if diff > 0.01 {
            if hp_ok { send(&format!("damage {name} {:.2} minecraft:generic_kill", diff))?; }
            else {
                let health = || -> AppResult<f64> {
                    let line = mgr.query(&cfg.id, &format!("data get entity {name} Health"), &[&format!("{name} has the following entity data"), "No entity was found"], Duration::from_secs(3))?;
                    line.rsplit("entity data: ").next().unwrap_or("").trim().trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().map_err(|_| AppError::invalid("No se pudo leer la vida."))
                };
                send(&format!("effect give {name} minecraft:poison {} 1 true", (diff * 0.7).ceil() as i64 + 5))?;
                let end = std::time::Instant::now() + Duration::from_millis((diff * 700.0) as u64 + 5000);
                while std::time::Instant::now() < end { if health()? <= h + 0.01 { break; } }
                send(&format!("effect clear {name} minecraft:poison"))?;
            }
        }
    }
    let mut food_note = String::new();
    if food_change {
        let (f, s) = set_food_live(mgr, cfg, name, e.food, e.saturation)?;
        if e.food.map(|w| w != f).unwrap_or(false) || e.saturation.map(|w| (w - s).abs() > 1.0).unwrap_or(false) { food_note = format!(" (hambre {f}, saturación {}: Minecraft gasta primero la saturación y la sube junto con el hambre)", fmt(s)); }
    }
    if let Some(x) = e.xp_level { send(&format!("xp set {name} {x} levels"))?; }
    std::thread::sleep(Duration::from_millis(400));
    let after = read().ok();
    let got_hp = after.as_ref().and_then(|v| v.get("Health")).and_then(|x| x.as_f64());
    let mut msg = format!("Listo para {name}{food_note}");
    if let (Some(want), Some(got)) = (e.health, got_hp) {
        // Hasta 1 punto de diferencia es la regeneración natural del juego mientras se lee.
        if (want - got).abs() > 1.0 { msg = format!("Se aplicó, pero {name} quedó con {} de vida (pedido {}): sus efectos o su absorción lo cambian.", fmt(got), fmt(want)); }
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

/* ───────── Hambre y saturación con el jugador conectado ───────── */
//
// Minecraft no tiene un comando para fijar el hambre, pero sí efectos con reglas exactas:
// - «saturation» es instantáneo: con `effect give` la duración se cuenta en ticks, así que
//   `saturation 1 <a>` se aplica una sola vez: hambre +(a+1) y saturación +2·(a+1) (con tope en
//   el hambre).
// - «hunger» suma 0,005·(a+1) de cansancio por tick; cada vez que el cansancio pasa de 4 el juego
//   gasta 1 de saturación o, si la saturación está en 0, 1 de hambre.
// Con el cansancio actual del jugador (`foodExhaustionLevel`) se calcula el efecto justo para gastar
// exactamente k puntos, y después se vuelve a leer para confirmar el resultado real.
// Reglas del juego que no se pueden saltar: en creativo y espectador el juego no gasta hambre, y en
// pacífico la rellena solo.

fn read_food(mgr: &Manager, cfg: &ServerConfig, name: &str) -> AppResult<(i64, f64, f64, String)> {
    let line = mgr.query(&cfg.id, &format!("data get entity {name}"), &[&format!("{name} has the following entity data"), "No entity was found"], Duration::from_secs(4))?;
    let v = playerdata::parse_reply(&line, name)?;
    let food = v.get("foodLevel").and_then(|x| x.as_i64()).unwrap_or(20);
    let sat = v.get("foodSaturationLevel").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let ex = v.get("foodExhaustionLevel").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let gm = v.get("playerGameType").and_then(|x| x.as_i64()).map(|g| ["survival", "creative", "adventure", "spectator"].get(g as usize).unwrap_or(&"survival").to_string()).unwrap_or_else(|| "survival".into());
    Ok((food, sat, ex, gm))
}

/// Gasta exactamente `k` puntos (primero saturación, después hambre) con el efecto «hunger»:
/// se aplica un efecto fuerte (un punto cada ~5 ticks) y se mide el hambre y la saturación reales
/// hasta llegar al valor buscado; ahí se quita el efecto. Medido con un jugador real: el tiempo
/// que dura un efecto de «1 segundo» varía unos ticks, así que no se puede calcular de antemano.
fn spend(mgr: &Manager, cfg: &ServerConfig, name: &str, k: i64, _exhaustion: f64) -> AppResult<()> {
    if k <= 0 { return Ok(()); }
    let field = |f: &str| -> AppResult<f64> {
        let line = mgr.query(&cfg.id, &format!("data get entity {name} {f}"), &[&format!("{name} has the following entity data"), "No entity was found", "Found no elements"], Duration::from_secs(3))?;
        let v = line.rsplit("entity data: ").next().unwrap_or("").trim().trim_end_matches(|c: char| c.is_ascii_alphabetic()).to_string();
        v.parse::<f64>().map_err(|_| AppError::invalid(&format!("No se pudo leer {f}: {}", line.trim())))
    };
    let (f0, s0) = (field("foodLevel")? as i64, field("foodSaturationLevel")?);
    let on_sat = if s0 > 0.0 { (s0.ceil() as i64).min(k) } else { 0 };
    let target_sat = if on_sat > 0 { (s0 - on_sat as f64).max(0.0) } else { s0 };
    let target_food = (f0 - (k - on_sat)).max(0);
    mgr.send_command(&cfg.id, &format!("effect give {name} minecraft:hunger {} 159 true", (k + 5).min(1000)))?;
    let end = std::time::Instant::now() + Duration::from_millis(400 * k as u64 + 6000);
    let mut ok = false;
    while std::time::Instant::now() < end {
        let sat = field("foodSaturationLevel")?;
        if sat <= target_sat + 0.001 {
            if target_food == f0 || field("foodLevel")? as i64 <= target_food { ok = true; break; }
        }
    }
    mgr.send_command(&cfg.id, &format!("effect clear {name} minecraft:hunger"))?;
    std::thread::sleep(Duration::from_millis(150));
    if !ok { return Err(AppError::invalid(&format!("Minecraft todavía no aplica cambios a {name}: suele pasar mientras termina de cargar el mundo al entrar. Probá de nuevo en unos segundos."))); }
    Ok(())
}

/// Lleva el hambre y/o la saturación de un jugador conectado a los valores pedidos. Devuelve
/// (hambre, saturación) reales al terminar.
pub fn set_food_live(mgr: &Manager, cfg: &ServerConfig, name: &str, food: Option<i64>, sat: Option<f64>) -> AppResult<(i64, f64)> {
    if !release_at_least(&cfg.mc_version, 13, 0) { return Err(AppError::invalid("Con el jugador conectado hace falta Minecraft 1.13 o más nuevo. Cambialo cuando esté desconectado.")); }
    let (f0, s0, _e0, gm) = read_food(mgr, cfg, name)?;
    let want_f = food.unwrap_or(f0).clamp(0, 20);
    let want_s = sat.map(|s| s.clamp(0.0, 20.0)).unwrap_or(s0).min(want_f as f64);
    let lowers = want_f < f0 || want_s < s0 - 0.01;
    if lowers && (gm == "creative" || gm == "spectator") {
        return Err(AppError::invalid(&format!("{name} está en modo {}: ahí Minecraft no gasta hambre ni saturación, así que no se puede bajar. Se puede subir, o bajarlo cuando esté en supervivencia o aventura.", if gm == "creative" { "creativo" } else { "espectador" })));
    }
    if lowers && cfg.difficulty == "peaceful" {
        return Err(AppError::invalid("En dificultad pacífica Minecraft rellena el hambre solo: no se puede bajar."));
    }
    let give = |n: i64| -> AppResult<()> {
        if n <= 0 { return Ok(()); }
        mgr.send_command(&cfg.id, &format!("effect give {name} minecraft:saturation 1 {} true", (n - 1).clamp(0, 255)))?;
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    };
    // 1) Hambre hacia arriba: «saturation» (sube también la saturación; se ajusta en el paso 3).
    if want_f > f0 { give(want_f - f0)?; }
    // 2) Hambre hacia abajo: primero se gasta toda la saturación y después el hambre que sobra.
    if want_f < f0 {
        let (_, s, e, _) = read_food(mgr, cfg, name)?;
        spend(mgr, cfg, name, s.ceil() as i64 + (f0 - want_f), e)?;
    }
    // 3) Saturación: hacia arriba con «saturation» (si el hambre está llena no cambia; si no, el
    //    juego también la sube y se compensa), hacia abajo gastando de a 1.
    let (f1, s1, e1, _) = read_food(mgr, cfg, name)?;
    // Sin una saturación pedida: al bajar el hambre queda la que deja el juego (en 0, la gastó
    // primero); al subirla se devuelve la que tenía antes.
    let want_s = if sat.is_none() && want_f < f0 { s1 } else { want_s };
    if want_s > s1 + 0.01 {
        let need = ((want_s - s1) / 2.0).ceil() as i64;
        give(need)?;
        let (f2, s2, e2, _) = read_food(mgr, cfg, name)?;
        // El efecto subió el hambre además de la saturación: no se puede bajar el hambre sin gastar
        // antes toda la saturación, así que el hambre queda más alta (se informa).
        let _ = f2;
        if s2 > want_s + 0.01 { spend(mgr, cfg, name, (s2 - want_s).floor() as i64, e2)?; }
    } else if s1 > want_s + 0.01 {
        spend(mgr, cfg, name, (s1 - want_s).floor().max(if want_s <= 0.0 { s1.ceil() } else { 0.0 }) as i64, e1)?;
    }
    let _ = f1;
    let (f, s, _, _) = read_food(mgr, cfg, name)?;
    // Nada cambió: el juego todavía no procesa a ese jugador (desde 1.21.4 un jugador que recién
    // entra no recibe efectos hasta que su Minecraft termina de cargar el mundo).
    if f == f0 && (s - s0).abs() < 0.01 && (want_f != f0 || (want_s - s0).abs() > 0.01) {
        return Err(AppError::invalid(&format!("Minecraft todavía no aplica cambios a {name}: suele pasar mientras termina de cargar el mundo al entrar. Probá de nuevo en unos segundos.")));
    }
    Ok((f, s))
}

/* ───────── Controles + y − (de a un paso) y matar ───────── */

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Stat { Health, Food, Saturation, Xp }

/// Paso mínimo útil de cada atributo: medio corazón, una unidad de hambre, media de saturación
/// (la vista la muestra con un decimal) y un nivel.
pub fn step(stat: Stat) -> f64 { match stat { Stat::Health => 1.0, Stat::Food => 1.0, Stat::Saturation => 0.5, Stat::Xp => 1.0 } }

/// Un cambio a la vez por jugador: un toque rápido no puede pisar a otro.
static BUSY: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashSet<String>>> = once_cell::sync::Lazy::new(Default::default);
struct Turn(String);
impl Turn {
    fn take(server: &str, name: &str) -> AppResult<Turn> {
        let k = format!("{server}/{}", name.to_lowercase());
        if !BUSY.lock().unwrap().insert(k.clone()) { return Err(AppError::invalid("Todavía se está aplicando el cambio anterior.")); }
        Ok(Turn(k))
    }
}
impl Drop for Turn { fn drop(&mut self) { BUSY.lock().unwrap().remove(&self.0); } }

/// Sube (`up`) o baja un paso. `live` = el jugador está conectado y el servidor encendido.
pub fn adjust(mgr: &Manager, cfg: &ServerConfig, name: &str, uuid: &str, live: bool, stat: Stat, up: bool, who: &str) -> AppResult<String> {
    let _turn = Turn::take(&cfg.id, name)?;
    let d = playerdata::detail(mgr, cfg, name, uuid, live)?;
    let sign = if up { 1.0 } else { -1.0 };
    if live {
        match stat {
            Stat::Food | Stat::Saturation => {
                let (f0, s0) = (d.food.unwrap_or(20), d.saturation.unwrap_or(0.0));
                let (tf, ts) = if stat == Stat::Food { (Some(f0 + if up { 1 } else { -1 }), None) } else { (None, Some((s0.floor() + if up { 1.0 } else { -1.0 }).max(0.0))) };
                if let Some(t) = tf { if !(0..=20).contains(&t) { return Err(AppError::invalid(if up { "El hambre ya está llena." } else { "El hambre ya está en cero." })); } }
                if stat == Stat::Saturation && !up && s0 <= 0.0 { return Err(AppError::invalid("La saturación ya está en cero.")); }
                if stat == Stat::Saturation && up && s0 + 0.01 >= f0 as f64 && f0 >= 20 { return Err(AppError::invalid("La saturación ya está al máximo (no puede superar al hambre).")); }
                let (f, s) = set_food_live(mgr, cfg, name, tf, ts)?;
                audit(&cfg.id, &format!("jugador-editado {name} (conectado): hambre {f}, saturación {}", fmt(s)), who);
                let mut msg = format!("Hambre {f} · saturación {}", fmt(s));
                if stat == Stat::Food && !up && s0 > 0.0 { msg.push_str(". Minecraft gasta primero la saturación, por eso quedó en 0"); }
                if stat == Stat::Saturation && up && f > f0 { msg.push_str(&format!(". Para subir la saturación Minecraft también subió el hambre ({f0} → {f})")); }
                return Ok(msg);
            }
            Stat::Xp => {
                let (_, _, xp_ok) = online_support(&cfg.mc_version);
                if !xp_ok { return Err(AppError::invalid("Esta versión de Minecraft no tiene el comando para cambiar la experiencia.")); }
                let lvl = d.xp_level.unwrap_or(0);
                if !up && lvl <= 0 { return Err(AppError::invalid("Ya está en el nivel 0.")); }
                if up && lvl >= 21863 { return Err(AppError::invalid("Ya está en el nivel máximo.")); }
                mgr.send_command(&cfg.id, &format!("xp add {name} {} levels", if up { 1 } else { -1 }))?;
                audit(&cfg.id, &format!("jugador-editado {name} (conectado): nivel {}", lvl + if up { 1 } else { -1 }), who);
                return Ok(format!("Nivel {}", lvl + if up { 1 } else { -1 }));
            }
            Stat::Health => {
                let cur = d.health.unwrap_or(20.0);
                let target = ((cur + sign * step(stat)) * 2.0).round() / 2.0;
                if target < 0.5 { return Err(AppError::invalid("Para dejarlo sin vida usá «Matar»: así muere de verdad en el juego.")); }
                // Tope: la vida máxima real (se valida con sus atributos dentro de apply_online).
                return apply_online(mgr, cfg, name, &Edit { health: Some(target), ..Default::default() }, who);
            }
        }
    }
    let e = match stat {
        Stat::Health => {
            let t = ((d.health.unwrap_or(20.0) + sign) * 2.0).round() / 2.0;
            if t < 0.5 { return Err(AppError::invalid("La vida no puede quedar en cero con el jugador desconectado.")); }
            Edit { health: Some(t), ..Default::default() }
        }
        Stat::Food => {
            let t = d.food.unwrap_or(20) + if up { 1 } else { -1 };
            if !(0..=20).contains(&t) { return Err(AppError::invalid(if up { "El hambre ya está llena." } else { "El hambre ya está en cero." })); }
            Edit { food: Some(t), ..Default::default() }
        }
        Stat::Saturation => {
            let food = d.food.unwrap_or(20) as f64;
            let t = ((d.saturation.unwrap_or(0.0) * 2.0).round() / 2.0 + sign * step(stat)).max(0.0);
            if t > food { return Err(AppError::invalid("Minecraft no deja la saturación por encima del hambre.")); }
            if !up && d.saturation.unwrap_or(0.0) <= 0.0 { return Err(AppError::invalid("La saturación ya está en cero.")); }
            Edit { saturation: Some(t), ..Default::default() }
        }
        Stat::Xp => {
            let t = d.xp_level.unwrap_or(0) + if up { 1 } else { -1 };
            if !(0..=21863).contains(&t) { return Err(AppError::invalid("El nivel va de 0 a 21863.")); }
            Edit { xp_level: Some(t), ..Default::default() }
        }
    };
    apply_offline(mgr, cfg, name, uuid, &e, who)
}

/// Mata al jugador de verdad con el comando de Minecraft (cuenta como muerte en el juego:
/// suelta su inventario según las reglas del mundo y aparece en sus estadísticas).
pub fn kill(mgr: &Manager, cfg: &ServerConfig, name: &str, who: &str) -> AppResult<String> {
    if !crate::players::valid_name(name) { return Err(AppError::invalid("Jugador inválido.")); }
    let _turn = Turn::take(&cfg.id, name)?;
    if !mgr.runtime(&cfg.id).lock().unwrap().players.contains(name) {
        return Err(AppError::invalid(&format!("{name} no está conectado: Minecraft solo puede matar a un jugador que está en el juego.")));
    }
    let line = mgr.query(&cfg.id, &format!("kill {name}"), &[&format!("Killed {name}"), "No entity was found", "No player was found", "Unknown or incomplete command"], Duration::from_secs(4))?;
    if !line.contains("Killed") { return Err(AppError::invalid(&format!("Minecraft no pudo matar a {name}: {}", line.trim()))); }
    audit(&cfg.id, &format!("jugador-matado {name}"), who);
    Ok(format!("{name} murió."))
}

/* ───────── Inventario en vivo ───────── */
//
// Solo con el jugador conectado y con comandos oficiales de Minecraft (no se edita el archivo
// mientras el juego lo tiene abierto). Cada operación se comprueba leyendo otra vez los datos
// del jugador: si no quedó como se pidió, se informa el error real.
// - Dar: `give` (todas las versiones).
// - Quitar: `item replace … with air` (1.17+) o `replaceitem … air` (1.13 a 1.16).
// - Mover: `item replace … from entity` (1.17+), conserva encantamientos, nombre y daño.
// - Cambiar la cantidad: `item modify` con un modificador en línea (1.20.5+), conserva todo lo
//   demás. En versiones anteriores solo se puede bajar con `clear` si ese objeto está en un solo
//   casillero. (Sintaxis comprobada con servidores reales 1.16.5, 1.17.1, 1.20.2, 1.20.4,
//   1.20.5, 1.21.4, 1.21.11, 26.1 y 26.3.)

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum InvOp {
    Give { item: String, count: u32 },
    #[serde(rename_all = "camelCase")]
    SetCount { group: String, slot: i64, count: u32 },
    #[serde(rename_all = "camelCase")]
    Remove { group: String, slot: i64 },
    #[serde(rename_all = "camelCase")]
    Move { group: String, from: i64, to: i64 },
}

/// Nombre del casillero para `/item` (`hotbar.0`, `inventory.4`, `armor.head`, `enderchest.3`…).
pub fn slot_name(group: &str, slot: i64) -> Option<String> {
    match (group, slot) {
        ("ender", 0..=26) => Some(format!("enderchest.{slot}")),
        ("ender", _) => None,
        (_, 0..=8) => Some(format!("hotbar.{slot}")),
        (_, 9..=35) => Some(format!("inventory.{}", slot - 9)),
        (_, 100) => Some("armor.feet".into()),
        (_, 101) => Some("armor.legs".into()),
        (_, 102) => Some("armor.chest".into()),
        (_, 103) => Some("armor.head".into()),
        (_, -106) => Some("weapon.offhand".into()),
        _ => None,
    }
}

/// Id de objeto válido (`minecraft:diamond`, `diamond`, `create:cogwheel`).
pub fn item_id(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c));
    let full = if s.contains(':') { s } else { format!("minecraft:{s}") };
    let (ns, path) = full.split_once(':')?;
    (ok(ns) && ok(path) && !ns.contains('/')).then_some(full)
}

/// El objeto de un casillero como argumento de comando, con sus datos (encantamientos, nombre,
/// desgaste): `minecraft:diamond_sword{Damage: 5, …}`. Para versiones anteriores a 1.20.5 (en las
/// nuevas se usa `item modify`). Se lee el texto tal cual lo escribe Minecraft, con sus tipos.
fn item_with_tag(mgr: &Manager, cfg: &ServerConfig, name: &str, group: &str, slot: i64, id: &str) -> AppResult<String> {
    let list = if group == "ender" { "EnderItems" } else { "Inventory" };
    let line = mgr.query(&cfg.id, &format!("data get entity {name} {list}[{{Slot:{slot}b}}]"), &[&format!("{name} has the following entity data"), "Found no elements", "No entity was found"], Duration::from_secs(4))?;
    let marker = "has the following entity data: ";
    let Some(pos) = line.find(marker) else { return Err(AppError::invalid("Ese casillero está vacío.")) };
    let snbt = &line[pos + marker.len()..];
    // `tag: {…}` con llaves balanceadas (respetando textos entre comillas).
    let tag = snbt.find("tag: {").map(|i| {
        let b = &snbt[i + 5..];
        let (mut depth, mut q, mut esc) = (0i32, None::<char>, false);
        let mut end = b.len();
        for (j, c) in b.char_indices() {
            if let Some(qc) = q { if esc { esc = false } else if c == '\\' { esc = true } else if c == qc { q = None } continue; }
            match c { '"' | '\'' => q = Some(c), '{' => depth += 1, '}' => { depth -= 1; if depth == 0 { end = j + 1; break; } } _ => {} }
        }
        b[..end].to_string()
    }).unwrap_or_default();
    Ok(format!("{id}{tag}"))
}

/// Pone `count` de `item` en un casillero (1.13 a 1.20.4: `replaceitem` / `item replace … with`).
fn replace_cmd(mc: &str, name: &str, slot: &str, item: &str, count: i64) -> String {
    if release_at_least(mc, 17, 0) { format!("item replace entity {name} {slot} with {item} {count}") } else { format!("replaceitem entity {name} {slot} {item} {count}") }
}

fn item_at(d: &playerdata::PlayerDetail, group: &str, slot: i64) -> Option<playerdata::Item> {
    if group == "ender" { return d.ender.iter().find(|i| i.slot == slot).cloned(); }
    let e = &d.equipment;
    match slot {
        103 => e.head.clone(), 102 => e.chest.clone(), 101 => e.legs.clone(), 100 => e.feet.clone(), -106 => e.offhand.clone(),
        _ => d.inventory.iter().find(|i| i.slot == slot).cloned(),
    }
}

/// Aplica una operación de inventario con el jugador conectado. Devuelve el texto para mostrar.
pub fn inventory(mgr: &Manager, cfg: &ServerConfig, name: &str, uuid: &str, op: &InvOp, who: &str) -> AppResult<String> {
    if !crate::players::valid_name(name) { return Err(AppError::invalid("Jugador inválido.")); }
    let _turn = Turn::take(&cfg.id, name)?;
    if !mgr.runtime(&cfg.id).lock().unwrap().players.contains(name) {
        return Err(AppError::invalid(&format!("{name} no está conectado. El inventario se cambia con el jugador en el juego (con comandos de Minecraft); desconectado solo se puede ver.")));
    }
    let mc = &cfg.mc_version;
    let read = || playerdata::detail(mgr, cfg, name, uuid, true);
    let before = read()?;
    let send = |c: &str| mgr.send_command(&cfg.id, c);
    let settle = || std::thread::sleep(Duration::from_millis(350));
    let (msg, ok): (String, Box<dyn Fn(&playerdata::PlayerDetail) -> bool>) = match op {
        InvOp::Give { item, count } => {
            let id = item_id(item).ok_or_else(|| AppError::invalid("Ese no es un id de objeto válido (por ejemplo minecraft:diamond)."))?;
            if !(1..=2304).contains(count) { return Err(AppError::invalid("La cantidad va de 1 a 2304 (36 casilleros de 64).")); }
            let line = mgr.query(&cfg.id, &format!("give {name} {id} {count}"), &["Gave ", "Given ", "Unknown item", "No player was found", "Unknown or incomplete command", "Invalid", "Can't find element"], Duration::from_secs(4))?;
            if !(line.contains("Gave ") || line.contains("Given ")) { return Err(AppError::invalid(&format!("Minecraft no lo dio: {}", line.split("]: ").last().unwrap_or(&line).trim()))); }
            audit(&cfg.id, &format!("inventario {name}: dar {count} {id}"), who);
            return Ok(format!("Le diste {count} × {id} a {name}. Si no tenía lugar, lo que sobró quedó en el piso a sus pies."));
        }
        InvOp::Remove { group, slot } => {
            let sn = slot_name(group, *slot).ok_or_else(|| AppError::invalid("Casillero inválido."))?;
            let it = item_at(&before, group, *slot).ok_or_else(|| AppError::invalid("Ese casillero ya está vacío."))?;
            if release_at_least(mc, 17, 0) { send(&format!("item replace entity {name} {sn} with minecraft:air"))?; }
            else if release_at_least(mc, 13, 0) {
                if group == "ender" { return Err(AppError::invalid("En esta versión de Minecraft no hay un comando para el cofre de ender (llegó en 1.17).")); }
                send(&format!("replaceitem entity {name} {sn} minecraft:air"))?;
            } else { return Err(AppError::invalid("Esta versión de Minecraft no tiene un comando para vaciar un casillero (llegó en 1.13).")); }
            let (g, s) = (group.clone(), *slot);
            (format!("Quitaste {} × {} de {name}.", it.count, it.id), Box::new(move |d| item_at(d, &g, s).is_none()))
        }
        InvOp::Move { group, from, to } => {
            if !release_at_least(mc, 13, 0) { return Err(AppError::invalid("Mover objetos con el jugador conectado necesita Minecraft 1.13 o más nuevo.")); }
            let (sf, st) = (slot_name(group, *from).ok_or_else(|| AppError::invalid("Casillero inválido."))?, slot_name(group, *to).ok_or_else(|| AppError::invalid("Casillero inválido."))?);
            let it = item_at(&before, group, *from).ok_or_else(|| AppError::invalid("El casillero de origen está vacío."))?;
            if item_at(&before, group, *to).is_some() { return Err(AppError::invalid("El casillero de destino está ocupado: elegí uno vacío.")); }
            if release_at_least(mc, 17, 0) { send(&format!("item replace entity {name} {st} from entity {name} {sf}"))?; }
            else { let item = item_with_tag(mgr, cfg, name, group, *from, &it.id)?; send(&replace_cmd(mc, name, &st, &item, it.count))?; }
            settle();
            // Solo se vacía el origen si la copia quedó en el destino (nunca se pierde el objeto).
            let mid = read()?;
            if item_at(&mid, group, *to).map(|x| x.id != it.id || x.count != it.count).unwrap_or(true) {
                return Err(AppError::invalid("Minecraft no copió el objeto al casillero nuevo (puede que ese casillero no acepte ese objeto, como una armadura en otro lugar). No se cambió nada."));
            }
            if release_at_least(mc, 17, 0) { send(&format!("item replace entity {name} {sf} with minecraft:air"))?; } else { send(&format!("replaceitem entity {name} {sf} minecraft:air"))?; }
            let (g, f, t, id, n) = (group.clone(), *from, *to, it.id.clone(), it.count);
            (format!("Moviste {} × {} de casillero.", it.count, it.id), Box::new(move |d| item_at(d, &g, f).is_none() && item_at(d, &g, t).map(|x| x.id == id && x.count == n).unwrap_or(false)))
        }
        InvOp::SetCount { group, slot, count } => {
            let sn = slot_name(group, *slot).ok_or_else(|| AppError::invalid("Casillero inválido."))?;
            let it = item_at(&before, group, *slot).ok_or_else(|| AppError::invalid("Ese casillero está vacío."))?;
            if *count == 0 { return inventory_remove_again(mgr, cfg, name, uuid, group, *slot, who); }
            if !(1..=99).contains(count) { return Err(AppError::invalid("En un casillero entran de 1 a 99 (según el objeto, el máximo suele ser 64, 16 o 1).")); }
            if *count as i64 == it.count { return Ok("Ya tiene esa cantidad.".into()); }
            if release_at_least(mc, 20, 5) {
                // Modificador en línea: desde 1.20.5 con la clave «function»; en 26.3 pasó a
                // llamarse «type» (comprobado con servidores reales). Se prueba uno y, si
                // Minecraft pide el otro, se usa ese.
                let errs = ["Failed to parse", "Expected whitespace", "Unknown or incomplete command", "Incorrect argument", "No entity was found"];
                let try_key = |k: &str| mgr.query(&cfg.id, &format!("item modify entity {name} {sn} {{{k}:\"minecraft:set_count\",count:{count}}}"), &errs, Duration::from_millis(1500));
                if let Ok(line) = try_key("function") {
                    if line.contains("key type") || line.contains("No key function") && line.contains("type") {
                        if let Ok(l2) = try_key("type") { if l2.contains("Failed") || l2.contains("Expected") || l2.contains("Unknown") { return Err(AppError::invalid(&format!("Minecraft no aceptó el cambio: {}", l2.trim()))); } }
                    } else if !line.contains("No entity") { return Err(AppError::invalid(&format!("Minecraft no aceptó el cambio: {}", line.trim()))); }
                }
            } else if release_at_least(mc, 13, 0) {
                // Antes de 1.20.5: se vuelve a poner el mismo objeto (con todos sus datos) con la cantidad nueva.
                let item = item_with_tag(mgr, cfg, name, group, *slot, &it.id)?;
                send(&replace_cmd(mc, name, &sn, &item, *count as i64))?;
            } else {
                return Err(AppError::invalid("Cambiar la cantidad con el jugador conectado necesita Minecraft 1.13 o más nuevo. Podés usar «Dar» para agregar más."));
            }
            let (g, s, n) = (group.clone(), *slot, *count as i64);
            (format!("{} ahora tiene {count} × {} en ese casillero.", name, it.id), Box::new(move |d| item_at(d, &g, s).map(|x| x.count == n).unwrap_or(false)))
        }
    };
    settle();
    let after = read()?;
    if !ok(&after) {
        // Pequeña espera extra por si el juego tardó un tick más.
        std::thread::sleep(Duration::from_millis(600));
        if !ok(&read()?) { return Err(AppError::invalid("Minecraft recibió el comando pero el inventario no quedó como se pidió (puede que el objeto no admita esa cantidad o que un mod lo impida).")); }
    }
    audit(&cfg.id, &format!("inventario {name}: {msg}"), who);
    Ok(msg)
}

fn inventory_remove_again(mgr: &Manager, cfg: &ServerConfig, name: &str, uuid: &str, group: &str, slot: i64, who: &str) -> AppResult<String> {
    // `inventory` ya tomó el turno de este jugador: se libera para la operación de quitar.
    BUSY.lock().unwrap().remove(&format!("{}/{}", cfg.id, name.to_lowercase()));
    inventory(mgr, cfg, name, uuid, &InvOp::Remove { group: group.into(), slot }, who)
}

#[cfg(test)]
mod inv_tests {
    use super::*;
    #[test]
    fn casilleros_e_ids() {
        assert_eq!(slot_name("inv", 0).unwrap(), "hotbar.0");
        assert_eq!(slot_name("inv", 9).unwrap(), "inventory.0");
        assert_eq!(slot_name("inv", 35).unwrap(), "inventory.26");
        assert_eq!(slot_name("eq", 103).unwrap(), "armor.head");
        assert_eq!(slot_name("eq", -106).unwrap(), "weapon.offhand");
        assert_eq!(slot_name("ender", 26).unwrap(), "enderchest.26");
        assert!(slot_name("ender", 27).is_none() && slot_name("inv", 36).is_none());
        assert_eq!(item_id("Diamond").unwrap(), "minecraft:diamond");
        assert_eq!(item_id("create:cogwheel").unwrap(), "create:cogwheel");
        for bad in ["", "a b", "x;say hi", "minecraft:", ":x", "a:b c", "a:b\nsay x", "ns/x:y"] { assert!(item_id(bad).is_none(), "{bad:?}"); }
    }
}
