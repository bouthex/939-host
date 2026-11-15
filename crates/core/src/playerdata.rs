//! Ficha de un jugador con datos reales del servidor, sin mods complementarios:
//! - Conectado: `/data get entity <nombre>` por consola (respuesta oculta de la consola).
//! - Desconectado: el archivo `world/playerdata/<uuid>.dat` que guarda Minecraft.
//! - Estadísticas: `world/stats/<uuid>.json`; logros: `world/advancements/<uuid>.json`.
//!
//! Soporta el formato antiguo (Inventory con ranuras 100-103) y el de 1.21.5+ (`equipment`).

use crate::error::{AppError, AppResult};
use crate::model::ServerConfig;
use crate::nbt;
use crate::paths;
use crate::process::Manager;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub slot: i64,
    pub id: String,
    pub count: i64,
    pub damage: Option<i64>,
    pub max_damage: Option<i64>,
    pub enchantments: Vec<(String, i64)>,
    pub custom_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effect { pub id: String, pub amplifier: i64, pub duration_ticks: i64 }

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Equipment { pub head: Option<Item>, pub chest: Option<Item>, pub legs: Option<Item>, pub feet: Option<Item>, pub offhand: Option<Item> }

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub play_time_ticks: Option<i64>,
    pub deaths: Option<i64>,
    pub mob_kills: Option<i64>,
    pub player_kills: Option<i64>,
    pub distance_cm: Option<i64>,
    pub jumps: Option<i64>,
    pub damage_dealt: Option<i64>,
    pub damage_taken: Option<i64>,
    pub time_since_death_ticks: Option<i64>,
    pub advancements: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerDetail {
    pub name: String,
    pub uuid: String,
    /// "live" (consultado ahora) | "saved" (último guardado de Minecraft) | "none"
    pub source: String,
    pub saved_at: Option<String>,
    pub health: Option<f64>,
    pub food: Option<i64>,
    pub saturation: Option<f64>,
    pub xp_level: Option<i64>,
    pub xp_progress: Option<f64>,
    pub gamemode: Option<String>,
    pub dimension: Option<String>,
    pub pos: Option<[f64; 3]>,
    pub facing: Option<String>,
    pub spawn: Option<(String, [i64; 3])>,
    pub armor_points: i64,
    pub equipment: Equipment,
    pub inventory: Vec<Item>,
    pub selected_slot: Option<i64>,
    pub ender: Vec<Item>,
    pub effects: Vec<Effect>,
    pub stats: Stats,
}

fn max_damage(id: &str) -> Option<i64> {
    let id = id.trim_start_matches("minecraft:");
    let tier = |p: &str| -> Option<i64> {
        Some(match p { "wooden" => 59, "stone" => 131, "iron" => 250, "golden" => 32, "diamond" => 1561, "netherite" => 2031, _ => return None })
    };
    for tool in ["_sword", "_pickaxe", "_axe", "_shovel", "_hoe"] {
        if let Some(p) = id.strip_suffix(tool) { return tier(p); }
    }
    let armor = |mat: &str, slot: usize| -> Option<i64> {
        let t: [i64; 4] = match mat { "leather" => [55, 80, 75, 65], "chainmail" => [165, 240, 225, 195], "iron" => [165, 240, 225, 195], "golden" => [77, 112, 105, 91], "diamond" => [363, 528, 495, 429], "netherite" => [407, 592, 555, 481], _ => return None };
        Some(t[slot])
    };
    for (i, s) in ["_helmet", "_chestplate", "_leggings", "_boots"].iter().enumerate() {
        if let Some(m) = id.strip_suffix(s) { return armor(m, i); }
    }
    Some(match id {
        "turtle_helmet" => 275, "bow" => 384, "crossbow" => 465, "trident" => 250, "shield" => 336, "elytra" => 432,
        "fishing_rod" => 64, "flint_and_steel" => 64, "shears" => 238, "carrot_on_a_stick" => 25, "warped_fungus_on_a_stick" => 100,
        "brush" => 64, "mace" => 500, _ => return None,
    })
}

/// Puntos de armadura de vanilla por pieza.
fn armor_value(id: &str) -> i64 {
    let id = id.trim_start_matches("minecraft:");
    let table: [(&str, [i64; 4]); 6] = [("leather", [1, 3, 2, 1]), ("chainmail", [2, 5, 4, 1]), ("iron", [2, 6, 5, 2]), ("golden", [2, 5, 3, 1]), ("diamond", [3, 8, 6, 3]), ("netherite", [3, 8, 6, 3])];
    if id == "turtle_helmet" { return 2; }
    for (i, s) in ["_helmet", "_chestplate", "_leggings", "_boots"].iter().enumerate() {
        if let Some(m) = id.strip_suffix(s) { if let Some((_, v)) = table.iter().find(|(n, _)| *n == m) { return v[i]; } }
    }
    0
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            // Puede ser JSON de texto de Minecraft serializado como cadena.
            if s.starts_with('{') || s.starts_with('"') || s.starts_with('[') {
                if let Ok(j) = serde_json::from_str::<Value>(s) { return text_of(&j); }
            }
            Some(s.clone())
        }
        Value::Object(o) => {
            let mut t = o.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if let Some(Value::Array(extra)) = o.get("extra") { for e in extra { t.push_str(&text_of(e).unwrap_or_default()); } }
            Some(t)
        }
        Value::Array(a) => Some(a.iter().filter_map(text_of).collect()),
        _ => None,
    }
}

fn parse_item(v: &Value) -> Option<Item> {
    let id = v.get("id")?.as_str()?.to_string();
    if id == "minecraft:air" { return None; }
    let count = v.get("count").or_else(|| v.get("Count")).and_then(|c| c.as_i64()).unwrap_or(1);
    let slot = v.get("Slot").and_then(|s| s.as_i64()).unwrap_or(-1);
    let comps = v.get("components");
    let tag = v.get("tag");
    let damage = comps.and_then(|c| c.get("minecraft:damage")).and_then(|d| d.as_i64())
        .or_else(|| tag.and_then(|t| t.get("Damage")).and_then(|d| d.as_i64()));
    let max = comps.and_then(|c| c.get("minecraft:max_damage")).and_then(|d| d.as_i64()).or_else(|| max_damage(&id));
    let mut ench = vec![];
    if let Some(e) = comps.and_then(|c| c.get("minecraft:enchantments")) {
        let map = e.get("levels").unwrap_or(e);
        if let Some(o) = map.as_object() { for (k, l) in o { if k != "show_in_tooltip" { if let Some(l) = l.as_i64() { ench.push((k.clone(), l)); } } } }
    } else if let Some(Value::Array(list)) = tag.and_then(|t| t.get("Enchantments")) {
        for e in list { if let (Some(i), Some(l)) = (e.get("id").and_then(|x| x.as_str()), e.get("lvl").and_then(|x| x.as_i64())) { ench.push((i.to_string(), l)); } }
    }
    let custom_name = comps.and_then(|c| c.get("minecraft:custom_name")).and_then(text_of)
        .or_else(|| tag.and_then(|t| t.get("display")).and_then(|d| d.get("Name")).and_then(text_of))
        .filter(|s| !s.is_empty());
    Some(Item { slot, id, count, damage, max_damage: max, enchantments: ench, custom_name })
}

fn items(v: Option<&Value>) -> Vec<Item> {
    v.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(parse_item).collect()).unwrap_or_default()
}

fn facing(yaw: f64) -> &'static str {
    let y = ((yaw % 360.0) + 360.0) % 360.0;
    match ((y + 45.0) / 90.0) as i64 % 4 { 0 => "Sur", 1 => "Oeste", 2 => "Norte", _ => "Este" }
}

/// Convierte los datos de jugador de Minecraft (de `/data get` o de `playerdata`) en la ficha.
pub fn from_value(v: &Value) -> PlayerDetail {
    let mut d = PlayerDetail::default();
    let f = |k: &str| v.get(k).and_then(|x| x.as_f64());
    let i = |k: &str| v.get(k).and_then(|x| x.as_i64());
    d.health = f("Health");
    d.food = i("foodLevel");
    d.saturation = f("foodSaturationLevel");
    d.xp_level = i("XpLevel");
    d.xp_progress = f("XpP");
    d.gamemode = i("playerGameType").map(|g| ["survival", "creative", "adventure", "spectator"].get(g as usize).unwrap_or(&"survival").to_string());
    d.dimension = v.get("Dimension").and_then(|x| x.as_str()).map(String::from);
    if let Some(Value::Array(p)) = v.get("Pos") {
        if p.len() == 3 { d.pos = Some([p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0), p[2].as_f64().unwrap_or(0.0)]); }
    }
    if let Some(yaw) = v.get("Rotation").and_then(|r| r.get(0)).and_then(|x| x.as_f64()) { d.facing = Some(facing(yaw).into()); }
    if let Some(r) = v.get("respawn") {
        if let (Some(Value::Array(p)), dim) = (r.get("pos"), r.get("dimension").and_then(|x| x.as_str()).unwrap_or("minecraft:overworld")) {
            if p.len() == 3 { d.spawn = Some((dim.to_string(), [p[0].as_i64().unwrap_or(0), p[1].as_i64().unwrap_or(0), p[2].as_i64().unwrap_or(0)])); }
        }
    } else if let (Some(x), Some(y), Some(z)) = (i("SpawnX"), i("SpawnY"), i("SpawnZ")) {
        d.spawn = Some((v.get("SpawnDimension").and_then(|x| x.as_str()).unwrap_or("minecraft:overworld").to_string(), [x, y, z]));
    }
    let mut inv = items(v.get("Inventory"));
    let mut eq = Equipment::default();
    if let Some(e) = v.get("equipment") {
        eq.head = e.get("head").and_then(parse_item);
        eq.chest = e.get("chest").and_then(parse_item);
        eq.legs = e.get("legs").and_then(parse_item);
        eq.feet = e.get("feet").and_then(parse_item);
        eq.offhand = e.get("offhand").and_then(parse_item);
    }
    // Formato antiguo: armadura y mano secundaria dentro del inventario.
    let take = |inv: &mut Vec<Item>, slot: i64| -> Option<Item> { inv.iter().position(|x| x.slot == slot).map(|p| inv.remove(p)) };
    if eq.head.is_none() { eq.head = take(&mut inv, 103); }
    if eq.chest.is_none() { eq.chest = take(&mut inv, 102); }
    if eq.legs.is_none() { eq.legs = take(&mut inv, 101); }
    if eq.feet.is_none() { eq.feet = take(&mut inv, 100); }
    if eq.offhand.is_none() { eq.offhand = take(&mut inv, -106); }
    inv.retain(|x| (0..36).contains(&x.slot));
    d.armor_points = [&eq.head, &eq.chest, &eq.legs, &eq.feet].iter().filter_map(|x| x.as_ref()).map(|x| armor_value(&x.id)).sum::<i64>().min(20);
    d.equipment = eq;
    d.inventory = inv;
    d.selected_slot = i("SelectedItemSlot").or_else(|| i("selected_item_slot"));
    d.ender = items(v.get("EnderItems"));
    let eff = v.get("active_effects").or_else(|| v.get("ActiveEffects"));
    if let Some(Value::Array(list)) = eff {
        for e in list {
            let id = e.get("id").and_then(|x| x.as_str()).map(String::from)
                .or_else(|| e.get("Id").and_then(|x| x.as_i64()).map(|n| format!("efecto {n}")))
                .unwrap_or_default();
            d.effects.push(Effect {
                id,
                amplifier: e.get("amplifier").or_else(|| e.get("Amplifier")).and_then(|x| x.as_i64()).unwrap_or(0),
                duration_ticks: e.get("duration").or_else(|| e.get("Duration")).and_then(|x| x.as_i64()).unwrap_or(0),
            });
        }
    }
    d
}

/// Extrae el SNBT de la respuesta de `/data get entity`.
pub fn parse_reply(line: &str, name: &str) -> AppResult<Value> {
    let marker = format!("{name} has the following entity data: ");
    let pos = line.find(&marker).ok_or_else(|| AppError::new("not_found", format!("{name} no está conectado.")))?;
    nbt::parse_snbt(&line[pos + marker.len()..]).map_err(|e| AppError::new("parse", format!("No se pudieron leer los datos del jugador: {e}")))
}

pub fn world_dir(cfg: &ServerConfig) -> std::path::PathBuf {
    let level = crate::properties::read(&paths::server_run_dir(&cfg.id).join("server.properties")).get("level-name").cloned().unwrap_or_else(|| "world".into());
    let level = if level.contains(['/', '\\']) || level.contains("..") { "world".into() } else { level };
    paths::server_run_dir(&cfg.id).join(level)
}

pub fn read_stats(cfg: &ServerConfig, uuid: &str) -> Stats {
    let mut st = Stats::default();
    if uuid.len() != 36 || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') { return st; }
    let w = world_dir(cfg);
    if let Some(j) = std::fs::read_to_string(w.join("stats").join(format!("{uuid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        let c = &j["stats"]["minecraft:custom"];
        let g = |k: &str| c.get(format!("minecraft:{k}")).and_then(|x| x.as_i64());
        st.play_time_ticks = g("play_time").or_else(|| g("play_one_minute"));
        st.deaths = g("deaths");
        st.mob_kills = g("mob_kills");
        st.player_kills = g("player_kills");
        st.jumps = g("jump");
        st.damage_dealt = g("damage_dealt");
        st.damage_taken = g("damage_taken");
        st.time_since_death_ticks = g("time_since_death");
        let dist: i64 = ["walk_one_cm", "sprint_one_cm", "swim_one_cm", "fly_one_cm", "walk_under_water_one_cm", "walk_on_water_one_cm", "crouch_one_cm", "climb_one_cm", "boat_one_cm", "horse_one_cm", "minecart_one_cm", "aviate_one_cm"].iter().filter_map(|k| g(k)).sum();
        if dist > 0 { st.distance_cm = Some(dist); }
    }
    if let Some(j) = std::fs::read_to_string(w.join("advancements").join(format!("{uuid}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        if let Some(o) = j.as_object() {
            st.advancements = Some(o.iter().filter(|(k, v)| !k.contains("recipes/") && v.get("done").and_then(|d| d.as_bool()).unwrap_or(false)).count() as i64);
        }
    }
    st
}

fn read_saved(cfg: &ServerConfig, uuid: &str) -> Option<(Value, Option<String>)> {
    if uuid.len() != 36 || !uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') { return None; }
    let p = world_dir(cfg).join("playerdata").join(format!("{uuid}.dat"));
    let when = std::fs::metadata(&p).ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339());
    nbt::read_nbt_file(&p).ok().map(|v| (v, when))
}

/// Ficha completa. Si el jugador está conectado se consulta en vivo; si no, el último guardado.
pub fn detail(mgr: &Manager, cfg: &ServerConfig, name: &str, uuid: &str, online: bool) -> AppResult<PlayerDetail> {
    if !crate::players::valid_name(name) { return Err(AppError::invalid("Nombre de jugador inválido.")); }
    let mut d = if online {
        let line = mgr.query(&cfg.id, &format!("data get entity {name}"), &[&format!("{name} has the following entity data"), "No entity was found"], Duration::from_secs(4))?;
        let mut d = from_value(&parse_reply(&line, name)?);
        d.source = "live".into();
        d
    } else if let Some((v, when)) = read_saved(cfg, uuid) {
        let mut d = from_value(&v);
        d.source = "saved".into();
        d.saved_at = when;
        d
    } else {
        PlayerDetail { source: "none".into(), ..Default::default() }
    };
    d.name = name.to_string();
    d.uuid = uuid.to_string();
    d.stats = read_stats(cfg, uuid);
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_player_reply() {
        let line = r#"[12:00:00] [Server thread/INFO]: Lucia has the following entity data: {Health: 17.0f, foodLevel: 14, foodSaturationLevel: 2.5f, XpLevel: 17, XpP: 0.62f, playerGameType: 0, Dimension: "minecraft:overworld", Pos: [412.3d, 68.0d, -1290.7d], Rotation: [180.0f, 10.0f], SelectedItemSlot: 0, respawn: {pos: [I; 398, 70, -1305], dimension: "minecraft:overworld"}, equipment: {head: {id: "minecraft:iron_helmet", count: 1}, chest: {id: "minecraft:diamond_chestplate", count: 1, components: {"minecraft:damage": 10, "minecraft:enchantments": {"minecraft:protection": 3}}}, offhand: {id: "minecraft:shield", count: 1}}, Inventory: [{Slot: 0b, id: "minecraft:diamond_sword", count: 1, components: {"minecraft:damage": 280, "minecraft:custom_name": "Filosa"}}, {Slot: 2b, id: "minecraft:torch", count: 64}], EnderItems: [{Slot: 0b, id: "minecraft:diamond", count: 12}], active_effects: [{id: "minecraft:night_vision", amplifier: 0b, duration: 5040}]}"#;
        let d = from_value(&parse_reply(line, "Lucia").unwrap());
        assert_eq!(d.health, Some(17.0));
        assert_eq!(d.food, Some(14));
        assert_eq!(d.gamemode.as_deref(), Some("survival"));
        assert_eq!(d.pos.unwrap()[2], -1290.7);
        assert_eq!(d.facing.as_deref(), Some("Norte"));
        assert_eq!(d.spawn.as_ref().unwrap().1, [398, 70, -1305]);
        assert_eq!(d.equipment.chest.as_ref().unwrap().enchantments, vec![("minecraft:protection".to_string(), 3)]);
        assert_eq!(d.armor_points, 2 + 8);
        let sword = &d.inventory[0];
        assert_eq!((sword.damage, sword.max_damage), (Some(280), Some(1561)));
        assert_eq!(sword.custom_name.as_deref(), Some("Filosa"));
        assert_eq!(d.ender[0].count, 12);
        assert_eq!(d.effects[0].duration_ticks, 5040);
        assert!(parse_reply("No entity was found", "Lucia").is_err());
    }

    #[test]
    fn legacy_inventory_armor_slots() {
        let v = serde_json::json!({"Inventory": [{"Slot": 103, "id": "minecraft:leather_helmet", "Count": 1}, {"Slot": 100, "id": "minecraft:golden_boots", "Count": 1}, {"Slot": -106, "id": "minecraft:torch", "Count": 5}, {"Slot": 8, "id": "minecraft:bread", "Count": 3, "tag": {"Enchantments": [{"id": "minecraft:unbreaking", "lvl": 2}]}}], "SpawnX": 1, "SpawnY": 2, "SpawnZ": 3});
        let d = from_value(&v);
        assert_eq!(d.equipment.head.as_ref().unwrap().id, "minecraft:leather_helmet");
        assert_eq!(d.equipment.offhand.as_ref().unwrap().count, 5);
        assert_eq!(d.inventory.len(), 1);
        assert_eq!(d.inventory[0].enchantments[0].1, 2);
        assert_eq!(d.armor_points, 2);
        assert_eq!(d.spawn.unwrap().1, [1, 2, 3]);
    }
}
