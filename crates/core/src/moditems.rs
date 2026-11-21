//! Nombres e íconos de ítems de mods, leídos de los `.jar` instalados en el servidor.
//!
//! Funciona igual con Fabric, Quilt, Forge y NeoForge porque no depende del cargador: todos los
//! mods traen sus recursos con el formato de los paquetes de recursos de Minecraft
//! (`assets/<espacio>/...`). Lo que se lee, sin ejecutar nada del mod:
//!
//! - Nombres: `assets/<ns>/lang/es_*.json` (o `en_us.json`), claves `item.<ns>.<id>` y
//!   `block.<ns>.<id>`.
//! - Íconos, en este orden: definición de ítem de 1.21.4+ (`assets/<ns>/items/<id>.json`),
//!   modelo de ítem (`models/item/<id>.json`, siguiendo `parent` hasta 6 niveles y las
//!   variables `#textura`), y por último `textures/item(s)/<id>.png` o `textures/block(s)/`.
//!   Un ítem 3D (bloque) se muestra con una cara de su textura.
//! - Mods dentro de mods (jar-in-jar de Fabric `META-INF/jars/` y de Forge/NeoForge
//!   `META-INF/jarjar/`), un nivel.
//!
//! El índice de cada `.jar` se guarda en caché por su huella SHA-256: agregar, quitar o
//! actualizar un mod se refleja solo. Todo con topes de tamaño: un mod roto o enorme no traba la
//! interfaz; en el peor caso sus ítems se muestran sin ícono.

use crate::paths;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Sube cuando cambia la forma de indexar: los índices guardados se rehacen.
const INDEX_VERSION: u32 = 1;
const MAX_ENTRY: u64 = 2 * 1024 * 1024;
const MAX_NESTED: u64 = 24 * 1024 * 1024;
const MAX_PNG: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct JarIndex {
    pub v: u32,
    /// Nombre del mod (de sus metadatos) o del archivo.
    pub mod_name: String,
    /// Espacios de nombres con recursos propios en este jar.
    pub namespaces: Vec<String>,
    /// `ns:id` → nombre visible.
    pub names: HashMap<String, String>,
    /// `ns:id` → ruta de la textura dentro del jar (`anidado.jar!/assets/...` si es jar-in-jar).
    pub icons: HashMap<String, String>,
}

/// Lo que la interfaz necesita de un ítem que no es de Minecraft.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ItemInfo {
    /// El espacio de nombres pertenece a un mod instalado en el servidor.
    pub known: bool,
    pub name: Option<String>,
    pub mod_name: Option<String>,
    /// Imagen PNG como `data:`.
    pub icon: Option<String>,
}

fn cache_dir() -> PathBuf { paths::cache().join("items-de-mods") }

fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(hex::encode(h.finalize()))
}

fn read_entry<R: Read + Seek>(z: &mut zip::ZipArchive<R>, name: &str, max: u64) -> Option<Vec<u8>> {
    let mut e = z.by_name(name).ok()?;
    if e.size() > max { return None; }
    let mut b = Vec::with_capacity(e.size() as usize);
    e.by_ref().take(max).read_to_end(&mut b).ok()?;
    Some(b)
}

fn valid_ns(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')) }
fn valid_path(s: &str) -> bool { !s.is_empty() && s.len() <= 200 && !s.contains("..") && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.' | b'/')) }

/// `ns:path` o `path` (Minecraft) → (ns, path).
fn split_ref(r: &str, default_ns: &str) -> Option<(String, String)> {
    let (ns, p) = r.split_once(':').unwrap_or((default_ns, r));
    (valid_ns(ns) && valid_path(p)).then(|| (ns.to_string(), p.to_string()))
}

fn mod_display_name<R: Read + Seek>(z: &mut zip::ZipArchive<R>) -> Option<String> {
    for f in ["fabric.mod.json", "quilt.mod.json"] {
        if let Some(b) = read_entry(z, f, 256 * 1024) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                let n = v.get("name").or_else(|| v.pointer("/quilt_loader/metadata/name")).and_then(|x| x.as_str());
                if let Some(n) = n.filter(|n| !n.trim().is_empty()) { return Some(n.trim().chars().take(80).collect()); }
            }
        }
    }
    for f in ["META-INF/neoforge.mods.toml", "META-INF/mods.toml"] {
        if let Some(b) = read_entry(z, f, 256 * 1024) {
            let t = String::from_utf8_lossy(&b);
            static RE: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r#"(?m)^\s*displayName\s*=\s*["']([^"'\r\n]{1,80})["']"#).unwrap());
            if let Some(c) = RE.captures(&t) { return Some(c[1].trim().to_string()); }
        }
    }
    None
}

struct Scan { names: HashMap<String, (u8, String)>, models: HashMap<String, Vec<u8>>, item_defs: HashMap<String, Vec<u8>>, textures: std::collections::HashSet<String>, namespaces: std::collections::BTreeSet<String> }

/// Prioridad del idioma: español primero.
fn lang_rank(file: &str) -> Option<u8> {
    Some(match file { "es_ar.json" => 0, "es_es.json" => 1, "es_mx.json" => 2, "es_419.json" => 3, "en_us.json" => 9, _ if file.starts_with("es_") && file.ends_with(".json") => 4, _ => return None })
}

fn scan_zip<R: Read + Seek>(z: &mut zip::ZipArchive<R>, prefix: &str, depth: u32, s: &mut Scan, nested: &mut Vec<(String, Vec<u8>)>) {
    let names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().and_then(|e| e.name().ok().map(|n| n.to_string()))).collect();
    for name in names {
        if depth == 0 && (name.starts_with("META-INF/jars/") || name.starts_with("META-INF/jarjar/")) && name.ends_with(".jar") {
            if let Some(b) = read_entry(z, &name, MAX_NESTED) { nested.push((name.clone(), b)); }
            continue;
        }
        let Some(rest) = name.strip_prefix("assets/") else { continue };
        let Some((ns, rest)) = rest.split_once('/') else { continue };
        if !valid_ns(ns) || ns == "minecraft" || ns == "realms" { continue; }
        if let Some(file) = rest.strip_prefix("lang/") {
            if let Some(rank) = lang_rank(file) {
                if let Some(b) = read_entry(z, &name, MAX_ENTRY) {
                    if let Ok(map) = serde_json::from_slice::<HashMap<String, serde_json::Value>>(&b) {
                        s.namespaces.insert(ns.to_string());
                        for (k, v) in map {
                            let Some(text) = v.as_str() else { continue };
                            for kind in ["item.", "block."] {
                                if let Some(id) = k.strip_prefix(kind).and_then(|r| r.strip_prefix(ns)).and_then(|r| r.strip_prefix('.')) {
                                    if valid_path(id) && !id.contains('.') {
                                        let key = format!("{ns}:{id}");
                                        let better = s.names.get(&key).map(|(r, _)| rank < *r || (rank == *r && kind == "item.")).unwrap_or(true);
                                        if better { s.names.insert(key, (rank, text.chars().take(80).collect())); }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else if let Some(m) = rest.strip_prefix("models/").and_then(|m| m.strip_suffix(".json")) {
            if valid_path(m) { if let Some(b) = read_entry(z, &name, 256 * 1024) { s.models.insert(format!("{ns}:{m}"), b); s.namespaces.insert(ns.to_string()); } }
        } else if let Some(m) = rest.strip_prefix("items/").and_then(|m| m.strip_suffix(".json")) {
            if valid_path(m) { if let Some(b) = read_entry(z, &name, 256 * 1024) { s.item_defs.insert(format!("{ns}:{m}"), b); s.namespaces.insert(ns.to_string()); } }
        } else if let Some(t) = rest.strip_prefix("textures/").and_then(|t| t.strip_suffix(".png")) {
            if valid_path(t) { s.textures.insert(format!("{prefix}{ns}:{t}")); s.namespaces.insert(ns.to_string()); }
        }
    }
}

/// Busca la primera textura de un modelo, siguiendo `parent` y variables `#nombre`.
fn model_texture(s: &Scan, model: &str, prefix_of: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut cur = model.to_string();
    for _ in 0..6 {
        let Some(b) = s.models.get(&cur) else { break };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(b) else { break };
        if let Some(t) = v.get("textures").and_then(|t| t.as_object()) {
            for (k, val) in t { if let Some(val) = val.as_str() { vars.entry(k.clone()).or_insert_with(|| val.to_string()); } }
        }
        match v.get("parent").and_then(|p| p.as_str()) {
            Some(p) => { let (ns, path) = split_ref(p, "minecraft")?; if ns == "minecraft" { break; } cur = format!("{ns}:{path}"); }
            None => break,
        }
    }
    let resolve = |mut r: String| -> Option<String> {
        for _ in 0..8 { match r.strip_prefix('#') { Some(k) => r = vars.get(k)?.clone(), None => break } }
        let (ns, path) = split_ref(&r, "minecraft")?;
        prefix_of(&format!("{ns}:{path}"))
    };
    for k in ["layer0", "all", "side", "front", "top", "texture", "particle", "end", "bottom"] {
        if let Some(r) = vars.get(k).cloned() { if let Some(t) = resolve(r) { return Some(t); } }
    }
    let mut rest: Vec<String> = vars.values().cloned().collect();
    rest.sort();
    rest.into_iter().find_map(resolve)
}

/// Modelo al que apunta una definición de ítem de 1.21.4+ (el primero que encuentre).
fn item_def_model(b: &[u8]) -> Option<String> {
    fn walk(v: &serde_json::Value, depth: u32) -> Option<String> {
        if depth > 8 { return None; }
        if let Some(m) = v.get("model") {
            if let Some(s) = m.as_str() { return Some(s.to_string()); }
            if let Some(r) = walk(m, depth + 1) { return Some(r); }
        }
        match v {
            serde_json::Value::Object(o) => o.values().find_map(|x| walk(x, depth + 1)),
            serde_json::Value::Array(a) => a.iter().find_map(|x| walk(x, depth + 1)),
            _ => None,
        }
    }
    walk(&serde_json::from_slice(b).ok()?, 0)
}

/// Indexa un jar (sin caché).
pub fn index_jar(path: &Path) -> Option<JarIndex> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut s = Scan { names: HashMap::new(), models: HashMap::new(), item_defs: HashMap::new(), textures: Default::default(), namespaces: Default::default() };
    let mut nested = vec![];
    let mod_name = mod_display_name(&mut z).unwrap_or_else(|| path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    scan_zip(&mut z, "", 0, &mut s, &mut nested);
    for (name, bytes) in nested {
        if let Ok(mut nz) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
            scan_zip(&mut nz, &format!("{name}!/"), 1, &mut s, &mut vec![]);
        }
    }
    // Textura (`ns:ruta`) → ruta dentro del jar.
    let tex: HashMap<String, String> = s.textures.iter().map(|full| {
        let (prefix, nsp) = full.rsplit_once("!/").map(|(p, r)| (format!("{p}!/"), r.to_string())).unwrap_or((String::new(), full.clone()));
        let (ns, p) = nsp.split_once(':').unwrap();
        (nsp.clone(), format!("{prefix}assets/{ns}/textures/{p}.png"))
    }).collect();
    let prefix_of = |r: &str| tex.get(r).cloned();
    let mut ids: std::collections::BTreeSet<String> = s.names.keys().cloned().collect();
    for k in s.item_defs.keys() { ids.insert(k.clone()); }
    for k in s.models.keys() { if let Some((ns, rest)) = k.split_once(':') { if let Some(id) = rest.strip_prefix("item/") { ids.insert(format!("{ns}:{id}")); } } }
    // Mods viejos (1.12 y antes) sin modelos: la textura con el nombre del ítem.
    for k in tex.keys() {
        if let Some((ns, rest)) = k.split_once(':') {
            if let Some((_, id)) = rest.split_once('/').filter(|(d, id)| matches!(*d, "item" | "items") && !id.contains('/')) { ids.insert(format!("{ns}:{id}")); }
        }
    }
    let mut icons = HashMap::new();
    for id in &ids {
        let (ns, name) = id.split_once(':').unwrap();
        let from_def = s.item_defs.get(id).and_then(|b| item_def_model(b)).and_then(|m| split_ref(&m, ns)).and_then(|(mns, mp)| model_texture(&s, &format!("{mns}:{mp}"), &prefix_of));
        let found = from_def
            .or_else(|| model_texture(&s, &format!("{ns}:item/{name}"), &prefix_of))
            .or_else(|| model_texture(&s, &format!("{ns}:block/{name}"), &prefix_of))
            .or_else(|| ["item", "items", "block", "blocks"].iter().find_map(|d| prefix_of(&format!("{ns}:{d}/{name}"))));
        if let Some(t) = found { icons.insert(id.clone(), t); }
    }
    Some(JarIndex {
        v: INDEX_VERSION,
        mod_name,
        namespaces: s.namespaces.into_iter().collect(),
        names: s.names.into_iter().map(|(k, (_, v))| (k, v)).collect(),
        icons,
    })
}

/// Índice de un jar con caché por huella.
fn cached_index(path: &Path) -> Option<(String, JarIndex)> {
    let sha = sha256_file(path)?;
    let file = cache_dir().join(format!("{sha}.json"));
    if let Some(i) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<JarIndex>(&b).ok()).filter(|i| i.v == INDEX_VERSION) {
        return Some((sha, i));
    }
    let idx = index_jar(path)?;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = paths::write_json(&file, &idx);
    Some((sha, idx))
}

/// Mods activos del servidor: `mods/*.jar` (los desactivados tienen otra extensión).
fn server_jars(id: &str) -> Vec<PathBuf> {
    let dir = paths::server_run_dir(id).join("mods");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("jar")).unwrap_or(false) && p.is_file()).collect();
    v.sort();
    v
}

#[derive(Default)]
struct ServerIndex { stamp: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>, jars: Vec<(PathBuf, JarIndex)>, ns_mod: HashMap<String, String> }

static INDEXES: Lazy<Mutex<HashMap<String, std::sync::Arc<ServerIndex>>>> = Lazy::new(Default::default);

fn server_index(id: &str) -> std::sync::Arc<ServerIndex> {
    let jars = server_jars(id);
    let stamp: Vec<_> = jars.iter().map(|p| { let m = std::fs::metadata(p).ok(); (p.clone(), m.as_ref().map(|m| m.len()).unwrap_or(0), m.and_then(|m| m.modified().ok())) }).collect();
    if let Some(i) = INDEXES.lock().unwrap().get(id) { if i.stamp == stamp { return i.clone(); } }
    let mut idx = ServerIndex { stamp, ..Default::default() };
    for p in jars {
        if let Some((_, j)) = cached_index(&p) {
            for ns in &j.namespaces { idx.ns_mod.entry(ns.clone()).or_insert_with(|| j.mod_name.clone()); }
            idx.jars.push((p, j));
        }
    }
    let idx = std::sync::Arc::new(idx);
    INDEXES.lock().unwrap().insert(id.to_string(), idx.clone());
    idx
}

fn read_icon(jar: &Path, inner: &str) -> Option<Vec<u8>> {
    let f = std::fs::File::open(jar).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let b = match inner.split_once("!/") {
        Some((nested, rest)) => {
            let nb = read_entry(&mut z, nested, MAX_NESTED)?;
            let mut nz = zip::ZipArchive::new(std::io::Cursor::new(nb)).ok()?;
            read_entry(&mut nz, rest, MAX_PNG)?
        }
        None => read_entry(&mut z, inner, MAX_PNG)?,
    };
    b.starts_with(b"\x89PNG\r\n\x1a\n").then_some(b)
}

fn pretty(id: &str) -> String {
    let s = id.rsplit('/').next().unwrap_or(id).replace('_', " ");
    let mut c = s.chars();
    match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => s }
}

/// Información de ítems que no son de Minecraft (`ns:id`). Hasta 300 por pedido.
pub fn lookup(server: &str, ids: &[String]) -> HashMap<String, ItemInfo> {
    let idx = server_index(server);
    let mut out = HashMap::new();
    for raw in ids.iter().take(300) {
        let Some((ns, name)) = raw.split_once(':') else { continue };
        if ns == "minecraft" || !valid_ns(ns) || !valid_path(name) { continue; }
        let key = format!("{ns}:{name}");
        let mut info = ItemInfo { known: idx.ns_mod.contains_key(ns), name: None, mod_name: idx.ns_mod.get(ns).cloned(), icon: None };
        for (path, j) in &idx.jars {
            if info.name.is_none() { if let Some(n) = j.names.get(&key) { info.name = Some(n.clone()); } }
            if info.icon.is_none() {
                if let Some(t) = j.icons.get(&key) { info.icon = read_icon(path, t).map(|b| crate::skins::data_url_png(&b)); }
            }
            if info.name.is_some() && info.icon.is_some() { break; }
        }
        if info.known && info.name.is_none() { info.name = Some(pretty(name)); }
        out.insert(raw.clone(), info);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    fn jar(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o: zip::write::SimpleFileOptions = Default::default();
        for (n, b) in files { z.start_file(*n, o).unwrap(); z.write_all(b).unwrap(); }
        z.finish().unwrap();
    }
    fn jar_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut c = std::io::Cursor::new(vec![]);
        { let mut z = zip::ZipWriter::new(&mut c); let o: zip::write::SimpleFileOptions = Default::default();
          for (n, b) in files { z.start_file(*n, o).unwrap(); z.write_all(b).unwrap(); } z.finish().unwrap(); }
        c.into_inner()
    }

    #[test]
    fn nombres_e_iconos_de_mods_de_todos_los_cargadores() {
        let t = tempfile::tempdir().unwrap();
        // Fabric moderno: definición de ítem 1.21.4+ → modelo → textura; nombres en español e inglés.
        let fab = t.path().join("fabric.jar");
        jar(&fab, &[
            ("fabric.mod.json", br#"{"id":"gemas","name":"Gemas Brillantes"}"#),
            ("assets/gemas/lang/en_us.json", br#"{"item.gemas.rubi":"Ruby","block.gemas.bloque_rubi":"Ruby Block","item.gemas.solo_en":"Only English"}"#),
            ("assets/gemas/lang/es_ar.json", r#"{"item.gemas.rubi":"Rubí","block.gemas.bloque_rubi":"Bloque de rubí"}"#.as_bytes()),
            ("assets/gemas/items/rubi.json", br#"{"model":{"type":"minecraft:model","model":"gemas:item/rubi"}}"#),
            ("assets/gemas/models/item/rubi.json", br#"{"parent":"minecraft:item/generated","textures":{"layer0":"gemas:item/rubi"}}"#),
            ("assets/gemas/textures/item/rubi.png", PNG),
            // Bloque: el modelo del ítem hereda del bloque, que usa una variable.
            ("assets/gemas/models/item/bloque_rubi.json", br#"{"parent":"gemas:block/bloque_rubi"}"#),
            ("assets/gemas/models/block/bloque_rubi.json", br##"{"parent":"minecraft:block/cube_all","textures":{"all":"#cara","cara":"gemas:block/rubi_cara"}}"##),
            ("assets/gemas/textures/block/rubi_cara.png", PNG),
        ]);
        let i = index_jar(&fab).unwrap();
        assert_eq!(i.mod_name, "Gemas Brillantes");
        assert_eq!(i.names["gemas:rubi"], "Rubí", "español primero");
        assert_eq!(i.names["gemas:solo_en"], "Only English");
        assert_eq!(i.icons["gemas:rubi"], "assets/gemas/textures/item/rubi.png");
        assert_eq!(i.icons["gemas:bloque_rubi"], "assets/gemas/textures/block/rubi_cara.png");
        // Forge 1.12: carpetas en plural, sin modelos; NeoForge con jar-in-jar.
        let old = t.path().join("forge.jar");
        jar(&old, &[("META-INF/mods.toml", b"[[mods]]\nmodId=\"viejo\"\ndisplayName=\"Mod Viejo\"\n"), ("assets/viejo/textures/items/pala.png", PNG)]);
        let i = index_jar(&old).unwrap();
        assert_eq!(i.mod_name, "Mod Viejo");
        assert_eq!(i.icons["viejo:pala"], "assets/viejo/textures/items/pala.png", "mod viejo sin modelos");
        let inner = jar_bytes(&[("assets/dentro/lang/en_us.json", br#"{"item.dentro.llave":"Key"}"#), ("assets/dentro/textures/item/llave.png", PNG)]);
        let neo = t.path().join("neo.jar");
        jar(&neo, &[("META-INF/neoforge.mods.toml", b"displayName='Contenedor'"), ("META-INF/jarjar/dentro.jar", &inner)]);
        let i = index_jar(&neo).unwrap();
        assert_eq!(i.names["dentro:llave"], "Key");
        assert_eq!(i.icons["dentro:llave"], "META-INF/jarjar/dentro.jar!/assets/dentro/textures/item/llave.png");
        assert_eq!(read_icon(&neo, &i.icons["dentro:llave"]).as_deref(), Some(PNG));
        // Un archivo que no es zip, o roto: sin índice y sin pánico.
        let roto = t.path().join("roto.jar");
        std::fs::write(&roto, b"no soy un zip").unwrap();
        assert!(index_jar(&roto).is_none());
        // Referencias peligrosas no se siguen.
        assert!(split_ref("gemas:../../x", "minecraft").is_none());
        assert!(split_ref("GEMAS:item/x", "minecraft").is_none());
    }

    #[test]
    fn servidor_distingue_mod_instalado_y_desconocido() {
        // Carpeta que no se borra al terminar: la raíz es una sola para todas las pruebas.
        let _ = paths::init(std::env::temp_dir().join(format!("h939-items-{}", std::process::id())));
        let id = format!("items-{}", std::process::id());
        let mods = paths::server_run_dir(&id).join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        jar(&mods.join("g.jar"), &[("fabric.mod.json", br#"{"name":"Gemas"}"#), ("assets/gemas/lang/en_us.json", br#"{"item.gemas.rubi":"Ruby"}"#), ("assets/gemas/models/item/rubi.json", br#"{"textures":{"layer0":"gemas:item/rubi"}}"#), ("assets/gemas/textures/item/rubi.png", PNG), ("assets/gemas/textures/item/sin_nombre.png", PNG), ("assets/gemas/models/item/sin_nombre.json", br#"{"textures":{"layer0":"gemas:item/sin_nombre"}}"#)]);
        std::fs::write(mods.join("apagado.jar.disabled"), b"x").unwrap();
        let r = lookup(&id, &["gemas:rubi".into(), "gemas:sin_nombre".into(), "otro:cosa".into(), "minecraft:stone".into(), "mal".into()]);
        assert_eq!(r.len(), 3);
        let rubi = &r["gemas:rubi"];
        assert!(rubi.known && rubi.icon.as_deref().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!((rubi.name.as_deref(), rubi.mod_name.as_deref()), (Some("Ruby"), Some("Gemas")));
        assert_eq!(r["gemas:sin_nombre"].name.as_deref(), Some("Sin nombre"));
        assert_eq!(r["otro:cosa"], ItemInfo { known: false, name: None, mod_name: None, icon: None }, "mod no instalado: no se inventa nada");
        // Quitar el mod se refleja en la próxima consulta.
        std::fs::remove_file(mods.join("g.jar")).unwrap();
        assert!(!lookup(&id, &["gemas:rubi".into()])["gemas:rubi"].known);
    }
}
