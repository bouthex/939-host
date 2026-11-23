//! Asistente de reparación: detecta por qué un servidor no arranca, lo explica en palabras
//! simples y, cuando existe una solución segura, la aplica de verdad.
//!
//! Funciona 100 % en la PC, sin IA ni servicios pagos: reglas sobre el registro del servidor,
//! los metadatos de cada mod (`fabric.mod.json`, `mods.toml`, incluidos los jars anidados) y la
//! API pública y gratuita de Modrinth para encontrar versiones compatibles.
//!
//! Garantías:
//! - Antes de cambiar algo se crea un punto de restauración («Antes de reparar») y se guarda una
//!   copia de la configuración: todo se puede deshacer.
//! - Solo se tocan archivos del servidor administrado (su carpeta y su configuración).
//! - Lo que afecta al mundo o requiere una decisión legal (EULA) siempre pide autorización.

use crate::error::{AppError, AppResult};
use crate::model::{Loader, ServerConfig};
use crate::modrinth::{self, Source};
use crate::mods;
use crate::paths;
use crate::process::Manager;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Mueve un jar de `mods` a la Papelera de 939 Host.
    SetAside { file: String },
    /// Instala una versión de Modrinth (con sus dependencias) y aparta el archivo que reemplaza.
    #[serde(rename_all = "camelCase")]
    InstallVersion { project_id: String, version_id: String, title: String, version_number: String, replaces: Option<String> },
    /// Descarga el Java indicado y lo asigna al servidor.
    InstallJava { major: u32 },
    SetPort { port: u16 },
    #[serde(rename_all = "camelCase")]
    SetMemory { mb: u32 },
    /// Registra la aceptación del EULA (solo con autorización explícita).
    AcceptEula,
    /// Fabric: un mod del servidor declara como obligatoria una dependencia que solo existe en el
    /// cliente (por ejemplo, Mod Menu). Se usa el mecanismo oficial de Fabric Loader
    /// (`config/fabric_loader_dependencies.json`, «-depends») para que el servidor no la exija.
    /// No modifica ningún jar.
    #[serde(rename_all = "camelCase")]
    IgnoreClientDependency { mod_id: String, dep: String },
    /// Vuelve a un punto de restauración (afecta al mundo: siempre con autorización).
    #[serde(rename_all = "camelCase")]
    RestoreSnapshot { snap_id: String, label: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub summary: String,
    pub changes: Vec<String>,
    pub actions: Vec<Action>,
    /// true: hay que confirmar explícitamente (afecta al mundo, a la configuración o es legal).
    pub needs_authorization: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub explanation: String,
    pub fix: Option<Fix>,
    pub technical: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    pub problems: Vec<Problem>,
    /// Hay al menos una reparación automática disponible.
    pub fixable: bool,
    pub summary: String,
}

// ───────── rangos de versión de Fabric ─────────

fn ver_nums(v: &str) -> Vec<u64> {
    let core = v.split(['-', '+']).next().unwrap_or(v);
    core.split('.').map(|p| p.trim_matches(|c: char| !c.is_ascii_digit()).parse().unwrap_or(0)).collect()
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (ver_nums(a), ver_nums(b));
    for i in 0..x.len().max(y.len()) {
        let o = x.get(i).unwrap_or(&0).cmp(y.get(i).unwrap_or(&0));
        if o != std::cmp::Ordering::Equal { return o; }
    }
    std::cmp::Ordering::Equal
}

/// ¿`version` cumple una condición de Fabric? (`*`, `1.21.x`, `>=1.21 <1.22`, `~1.21.4`, `^1.21`, exacta).
pub fn fabric_matches(cond: &str, version: &str) -> bool {
    let cond = cond.trim();
    if cond.is_empty() || cond == "*" { return true; }
    cond.split_whitespace().all(|part| {
        let (op, v) = ["<=", ">=", "<", ">", "=", "~", "^"].iter().find_map(|op| part.strip_prefix(op).map(|r| (*op, r))).unwrap_or(("", part));
        if v.ends_with(".x") || v.ends_with(".X") || v.ends_with(".*") {
            let base = &v[..v.len() - 2];
            let (b, x) = (ver_nums(base), ver_nums(version));
            return x.len() >= b.len() && b.iter().zip(&x).all(|(a, c)| a == c);
        }
        use std::cmp::Ordering::*;
        let o = cmp_ver(version, v);
        match op {
            ">=" => o != Less, "<=" => o != Greater, ">" => o == Greater, "<" => o == Less,
            "~" => { let b = ver_nums(v); o != Less && ver_nums(version).get(..2.min(b.len())) == b.get(..2.min(b.len())) }
            "^" => { let b = ver_nums(v); o != Less && ver_nums(version).first() == b.first() }
            _ => o == Equal,
        }
    })
}

fn cond_list(v: &Value) -> Vec<String> {
    match v { Value::String(s) => vec![s.clone()], Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(), _ => vec!["*".into()] }
}

// ───────── lectura de jars ─────────

#[derive(Debug, Clone, Default)]
pub struct JarInfo {
    pub file: String,
    pub corrupt: bool,
    /// "fabric" | "forge" | "neoforge" | "" (desconocido)
    pub kind: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub environment: String,
    /// Dependencias OBLIGATORIAS en el servidor: id → condiciones (alguna tiene que cumplirse).
    /// En Forge/NeoForge las de `side="CLIENT"` no cuentan acá.
    pub depends: BTreeMap<String, Vec<String>>,
    /// Dependencias opcionales (Fabric `recommends`/`suggests`, Forge `mandatory=false`, NeoForge
    /// `type="optional"`, o de lado cliente): nunca hacen falta para que el servidor arranque.
    pub optional: BTreeSet<String>,
    /// Incompatibilidades declaradas (Fabric `breaks`, NeoForge `type="incompatible"`).
    pub breaks: BTreeMap<String, Vec<String>>,
    /// ids que aporta este jar (propio, `provides` y jars anidados)
    pub provides: BTreeSet<String>,
}

fn fabric_json(raw: &str) -> Option<Value> {
    let clean: String = raw.chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect();
    serde_json::from_str(&clean).ok()
}

fn read_entry<R: std::io::Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, name: &str, max: u64) -> Option<Vec<u8>> {
    let mut e = z.by_name(name).ok()?;
    if e.size() > max { return None; }
    let mut buf = vec![];
    e.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn provides_of<R: std::io::Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, depth: usize, out: &mut BTreeSet<String>) {
    let Some(raw) = read_entry(z, "fabric.mod.json", 2_000_000) else { return };
    let Some(v) = fabric_json(&String::from_utf8_lossy(&raw)) else { return };
    if let Some(id) = v.get("id").and_then(|x| x.as_str()) { out.insert(id.to_string()); }
    if let Some(Value::Array(p)) = v.get("provides") { for x in p { if let Some(s) = x.as_str() { out.insert(s.to_string()); } } }
    if depth >= 3 { return; }
    if let Some(Value::Array(jars)) = v.get("jars") {
        for j in jars {
            let Some(path) = j.get("file").and_then(|x| x.as_str()) else { continue };
            if let Some(bytes) = read_entry(z, path, 64_000_000) {
                if let Ok(mut inner) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) { provides_of(&mut inner, depth + 1, out); }
            }
        }
    }
}

pub fn read_jar(path: &Path) -> JarInfo {
    let mut info = JarInfo { file: path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), ..Default::default() };
    let Ok(f) = std::fs::File::open(path) else { info.corrupt = true; return info };
    let Ok(mut z) = zip::ZipArchive::new(f) else { info.corrupt = true; return info };
    if let Some(raw) = read_entry(&mut z, "fabric.mod.json", 2_000_000) {
        let Some(v) = fabric_json(&String::from_utf8_lossy(&raw)) else { info.corrupt = true; return info };
        info.kind = "fabric".into();
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        info.id = s("id");
        info.name = if s("name").is_empty() { s("id") } else { s("name") };
        info.version = s("version");
        info.environment = if s("environment").is_empty() { "*".into() } else { s("environment") };
        if let Some(Value::Object(d)) = v.get("depends") { for (k, c) in d { info.depends.insert(k.clone(), cond_list(c)); } }
        for key in ["recommends", "suggests"] {
            if let Some(Value::Object(d)) = v.get(key) { info.optional.extend(d.keys().cloned()); }
        }
        if let Some(Value::Object(d)) = v.get("breaks") { for (k, c) in d { info.breaks.insert(k.clone(), cond_list(c)); } }
        provides_of(&mut z, 0, &mut info.provides);
        return info;
    }
    let neo = read_entry(&mut z, "META-INF/neoforge.mods.toml", 1_000_000);
    let toml = neo.clone().or_else(|| read_entry(&mut z, "META-INF/mods.toml", 1_000_000));
    if let Some(raw) = toml {
        info.kind = if neo.is_some() { "neoforge".into() } else { "forge".into() };
        let t = String::from_utf8_lossy(&raw);
        let field = |k: &str| t.lines().map(str::trim).find(|l| l.starts_with(k) && l[k.len()..].trim_start().starts_with('='))
            .map(|l| l.splitn(2, '=').nth(1).unwrap_or("").trim().trim_matches('"').to_string()).unwrap_or_default();
        info.id = field("modId");
        info.name = field("displayName");
        info.version = field("version");
        info.environment = "*".into();
        info.provides.insert(info.id.clone());
        for d in forge_deps(&t) {
            if d.mod_id.is_empty() { continue; }
            if d.incompatible { info.breaks.insert(d.mod_id, vec![d.range]); }
            else if d.required && d.side != "CLIENT" { info.depends.insert(d.mod_id, vec![d.range]); }
            else { info.optional.insert(d.mod_id); }
        }
    }
    info
}

/// Una dependencia de `mods.toml` / `neoforge.mods.toml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForgeDep { pub mod_id: String, pub required: bool, pub incompatible: bool, pub side: String, pub range: String }

/// Lee los bloques `[[dependencies.<mod>]]`. Forge usa `mandatory=true|false`; NeoForge,
/// `type="required"|"optional"|"incompatible"|"discouraged"`. `side` = BOTH | CLIENT | SERVER.
pub fn forge_deps(toml: &str) -> Vec<ForgeDep> {
    let mut out: Vec<ForgeDep> = vec![];
    let mut cur: Option<ForgeDep> = None;
    for raw in toml.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with("[[") || (line.starts_with('[') && !line.starts_with("[[")) {
            if let Some(d) = cur.take() { out.push(d); }
            if line.starts_with("[[dependencies") {
                // Forge: obligatoria por defecto si no dice nada; NeoForge también («required»).
                cur = Some(ForgeDep { required: true, side: "BOTH".into(), ..Default::default() });
            }
            continue;
        }
        let Some(d) = cur.as_mut() else { continue };
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        match k.trim() {
            "modId" => d.mod_id = v,
            "mandatory" => d.required = v == "true",
            "type" => { let t = v.to_lowercase(); d.required = t == "required"; d.incompatible = t == "incompatible"; }
            "side" => d.side = v.to_uppercase(),
            "versionRange" => d.range = v,
            _ => {}
        }
    }
    if let Some(d) = cur { out.push(d); }
    out
}

/// ¿`version` está en un rango de Maven (Forge/NeoForge)? `[1.20.1,1.21)`, `[47,)`, `(,2.0]`,
/// varios rangos separados por coma, o una versión suelta (= «cualquiera desde esa»).
pub fn maven_matches(range: &str, version: &str) -> bool {
    let r = range.trim();
    if r.is_empty() || r == "*" { return true; }
    if !r.starts_with(['[', '(']) { return cmp_ver(version, r) != std::cmp::Ordering::Less; }
    let mut parts = vec![];
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in r.char_indices() {
        match c { '[' | '(' => { if depth == 0 { start = i; } depth += 1; } ']' | ')' => { depth -= 1; if depth == 0 { parts.push(&r[start..=i]); } } _ => {} }
    }
    parts.iter().any(|p| {
        let (lo_inc, hi_inc) = (p.starts_with('['), p.ends_with(']'));
        let inner = &p[1..p.len() - 1];
        let (lo, hi) = match inner.split_once(',') { Some((a, b)) => (a.trim(), b.trim()), None => (inner.trim(), inner.trim()) };
        use std::cmp::Ordering::*;
        let lo_ok = lo.is_empty() || match cmp_ver(version, lo) { Greater => true, Equal => lo_inc, Less => false };
        let hi_ok = hi.is_empty() || match cmp_ver(version, hi) { Less => true, Equal => hi_inc, Greater => false };
        lo_ok && hi_ok
    })
}

/// Mods conocidos que solo funcionan en el cliente (sirven aunque no haya internet). Con internet
/// también vale lo que dice Modrinth (`server_side: unsupported`).
pub const KNOWN_CLIENT_ONLY: &[&str] = &[
    "modmenu", "sodium", "iris", "indium", "continuity", "entityculling", "betterf3", "zoomify",
    "sodium-extra", "reeses-sodium-options", "lambdynlights", "dynamic_fps", "dynamicfps", "cull-leaves",
];

/// Archivo oficial de Fabric Loader para corregir dependencias mal declaradas.
pub const FABRIC_OVERRIDES: &str = "config/fabric_loader_dependencies.json";

/// Agrega «`mod_id` no exige `dep`» al archivo de Fabric, conservando lo que ya hubiera.
pub fn add_fabric_override(run_dir: &Path, mod_id: &str, dep: &str) -> AppResult<()> {
    let path = run_dir.join(FABRIC_OVERRIDES);
    let mut v: Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({ "version": 1, "overrides": {} }));
    if !v.get("overrides").map(|o| o.is_object()).unwrap_or(false) { v["overrides"] = serde_json::json!({}); }
    v["version"] = 1.into();
    let entry = v["overrides"].as_object_mut().unwrap().entry(mod_id.to_string()).or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() { *entry = serde_json::json!({}); }
    let minus = entry.as_object_mut().unwrap().entry("-depends".to_string()).or_insert_with(|| serde_json::json!({}));
    if !minus.is_object() { *minus = serde_json::json!({}); }
    minus.as_object_mut().unwrap().insert(dep.to_string(), "*".into());
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| AppError::io("No se pudo crear la carpeta config", e))?;
    paths::write_atomic(&path, serde_json::to_string_pretty(&v).unwrap().as_bytes())
}

/// Dependencias que el archivo de Fabric ya hace ignorar: (mod, dependencia).
pub fn fabric_overrides(run_dir: &Path) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let Some(v) = std::fs::read_to_string(run_dir.join(FABRIC_OVERRIDES)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return out };
    if let Some(Value::Object(o)) = v.get("overrides") {
        for (m, e) in o { if let Some(Value::Object(d)) = e.get("-depends") { for k in d.keys() { out.insert((m.clone(), k.clone())); } } }
    }
    out
}

const BUILTIN: &[&str] = &["minecraft", "java", "fabricloader", "fabric-loader", "quilt_loader", "forge", "neoforge"];

/// Nombres de proyecto en Modrinth para ids de mod que no coinciden con el slug.
fn modrinth_slug(mod_id: &str) -> String {
    match mod_id {
        "fabric" | "fabric-api" => "fabric-api",
        "cloth-config2" | "cloth_config" | "cloth-config" => "cloth-config",
        "architectury" => "architectury-api",
        "fabric-language-kotlin" | "fabric_language_kotlin" => "fabric-language-kotlin",
        "owo" | "owo-lib" => "owo-lib",
        "geckolib" | "geckolib3" => "geckolib",
        other => other,
    }.to_string()
}

fn loader_kind(l: Loader) -> &'static str { match l { Loader::Fabric => "fabric", Loader::Forge => "forge", Loader::NeoForge => "neoforge", Loader::Vanilla => "" } }

fn tail(text: &str, needle: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    match lines.iter().position(|l| l.contains(needle)) {
        Some(i) => lines[i..(i + n).min(lines.len())].iter().map(|s| s.to_string()).collect(),
        None => vec![],
    }
}

/// Texto del último arranque: la consola en memoria o `logs/latest.log`.
pub fn last_log(mgr: &Manager, id: &str) -> String {
    let lines = mgr.console(id);
    let from_mem = if let Some(i) = lines.iter().rposition(|l| l.starts_with("── 939 Host: iniciando")) { lines[i..].join("\n") } else { String::new() };
    if from_mem.lines().count() > 3 { return from_mem; }
    std::fs::read_to_string(paths::server_run_dir(id).join("logs").join("latest.log")).unwrap_or_default()
}

static RE_CLASS: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"class file version (\d+)(?:\.\d+)?").unwrap());
static RE_MIXIN_MOD: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?:Mixin apply for mod|from mod|mod) '?([a-z0-9_\-]{2,64})'? failed").unwrap());

/// Analiza el servidor. `max_memory_mb`: máximo seguro de RAM para esta PC.
pub async fn analyze<S: Source>(src: &S, mgr: &Manager, cfg: &ServerConfig, max_memory_mb: u32, online: bool) -> Diagnosis {
    let log = last_log(mgr, &cfg.id);
    let mut problems: Vec<Problem> = vec![];

    // ── problemas de arranque generales ──
    if log.contains("You need to agree to the EULA") || cfg.eula_accepted_at.is_none() {
        add(&mut problems, Problem { id: "eula".into(), kind: "eula".into(),
            title: "Falta aceptar el EULA de Minecraft".into(),
            explanation: "Mojang exige aceptar su contrato de licencia para ejecutar un servidor. Es un requisito legal: solo vos podés aceptarlo.".into(),
            fix: Some(Fix { summary: "Registrar que aceptás el EULA de Minecraft".into(), changes: vec!["Se guarda la aceptación con fecha en la configuración del servidor".into()], actions: vec![Action::AcceptEula], needs_authorization: true }),
            technical: tail(&log, "EULA", 2) });
    }
    if let Some(c) = RE_CLASS.captures(&log) {
        let class: u32 = c[1].parse().unwrap_or(0);
        if class > 44 {
            let need = class - 44;
            if need > cfg.java_major {
                add(&mut problems, Problem { id: format!("java-{need}"), kind: "java".into(),
                    title: format!("Este servidor necesita Java {need}"),
                    explanation: format!("Algún componente fue compilado para Java {need}, pero el servidor usa Java {}. No es un problema de tu PC.", cfg.java_major),
                    fix: Some(Fix { summary: format!("Descargar Java {need} (Eclipse Adoptium, gratis) y usarlo en este servidor"), changes: vec![format!("Java {} → Java {need}", cfg.java_major)], actions: vec![Action::InstallJava { major: need }], needs_authorization: false }),
                    technical: tail(&log, "UnsupportedClassVersionError", 3) });
            }
        }
    }
    // Java no pudo ni empezar: no encontró la clase principal.
    if let Some(line) = log.lines().find(|l| l.contains("Could not find or load main class") || l.contains("no se ha encontrado o cargado la clase principal") || l.contains("Main class name has not been specified")) {
        let argfile = line.contains(" @") || line.contains("principal @");
        add(&mut problems, Problem { id: "launch".into(), kind: "launch".into(),
            title: if argfile { format!("Java {} no entendía la forma de arrancar el servidor", cfg.java_major) } else { "Java no encontró el programa del servidor".into() },
            explanation: if argfile {
                "Las versiones anteriores de 939 Host arrancaban Fabric con un archivo de argumentos («@archivo»), algo que Java 8 no entiende (llegó en Java 9). Por eso los servidores Fabric de Minecraft 1.16 o anterior no arrancaban. Esta versión ya los arranca de la forma que entiende Java 8: volvé a iniciarlo. No hay que cambiar nada del servidor.".into()
            } else {
                "Faltan archivos del servidor o del loader (por ejemplo, una descarga incompleta). La solución es reinstalar los archivos del servidor desde Ajustes → Avanzado → «Reinstalar»: no toca el mundo.".into()
            },
            fix: None, technical: vec![line.trim().to_string()] });
    }
    let active = mgr.runtime(&cfg.id).lock().unwrap().is_active();
    // Solo si el puerto sigue ocupado ahora: un registro viejo con "FAILED TO BIND" no alcanza.
    // Comprobación igual a la de Java (modo exclusivo, IPv4 e IPv6): ver `ports`.
    if !active && !crate::ports::is_free(cfg.port) {
        let occ = crate::ports::occupant(cfg.port, &mgr.own_processes());
        let others: Vec<u16> = crate::instance::list_configs().into_iter().filter(|c| c.id != cfg.id).map(|c| c.port).collect();
        let port = crate::ports::free_near(cfg.port, &others);
        let lan = crate::access::lan_effective(&cfg.id);
        add(&mut problems, Problem { id: "port".into(), kind: "port".into(),
            title: format!("El puerto {} está ocupado", cfg.port),
            explanation: format!("Lo usa {}, así que Minecraft no puede escuchar ahí. 939 Host no cierra ni modifica ese programa: cambia el puerto de este servidor.", occ.describe()),
            fix: port.map(|p| Fix { summary: format!("Usar el puerto libre {p}"),
                changes: vec![format!("Puerto {} → {p}. Tus amigos que entran con 939 Host no tienen que cambiar nada.{}", cfg.port, if lan { " Quien entre directo por la red local tiene que usar el puerto nuevo." } else { "" })],
                actions: vec![Action::SetPort { port: p }], needs_authorization: false }),
            technical: tail(&log, "FAILED TO BIND", 3) });
    }
    if log.contains("OutOfMemoryError") {
        let next = (((cfg.memory_mb as f64 * 1.5) / 512.0).ceil() as u32 * 512).min(max_memory_mb.max(cfg.memory_mb));
        add(&mut problems, Problem { id: "memory".into(), kind: "memory".into(),
            title: "Al servidor le faltó memoria".into(),
            explanation: format!("Minecraft se quedó sin la RAM asignada ({} MB). Pasa con mundos grandes o muchos mods.", cfg.memory_mb),
            fix: (next > cfg.memory_mb).then(|| Fix { summary: format!("Subir la memoria a {next} MB (dentro del máximo seguro de esta PC)"), changes: vec![format!("Memoria {} MB → {next} MB", cfg.memory_mb)], actions: vec![Action::SetMemory { mb: next }], needs_authorization: true }),
            technical: tail(&log, "OutOfMemoryError", 3) });
    }
    if regex::Regex::new(r"(?i)(exception reading .*level\.dat|failed to load level|couldn't load chunk|corrupted chunk|region file .* is truncated)").unwrap().is_match(&log) {
        let snap = crate::snapshots::list(&cfg.id).into_iter().find(|s| s.counts_for_sync());
        add(&mut problems, Problem { id: "world".into(), kind: "world".into(),
            title: "Parte del mundo está dañada".into(),
            explanation: "Minecraft no pudo leer un archivo del mundo. Suele pasar si la PC se apagó de golpe mientras guardaba.".into(),
            fix: snap.map(|s| Fix { summary: format!("Volver al punto de restauración «{}»", s.label), changes: vec!["El mundo vuelve a ese momento. Lo jugado después se pierde, pero el estado actual queda guardado como punto de seguridad.".into()], actions: vec![Action::RestoreSnapshot { snap_id: s.id.clone(), label: s.label.clone() }], needs_authorization: true }),
            technical: log.lines().filter(|l| l.contains("level.dat") || l.to_lowercase().contains("chunk")).take(4).map(String::from).collect() });
    }

    // ── mods ──
    let want = loader_kind(cfg.loader);
    if !want.is_empty() {
        let dir = mods::mods_dir(&cfg.id);
        let mut jars: Vec<JarInfo> = std::fs::read_dir(&dir).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jar")).map(|e| read_jar(&e.path())).collect()).unwrap_or_default();
        jars.sort_by(|a, b| a.file.cmp(&b.file));
        let provided: BTreeSet<String> = jars.iter().filter(|j| !j.corrupt).flat_map(|j| j.provides.iter().cloned()).collect();
        let hashes: Vec<String> = if online { jars.iter().filter_map(|j| crate::net::sha1_file(&dir.join(&j.file)).ok()).collect() } else { vec![] };
        let by_hash: HashMap<String, modrinth::Version> = if online && !hashes.is_empty() { src.versions_by_sha1(&hashes).await.unwrap_or_default() } else { HashMap::new() };
        let file_version: HashMap<String, modrinth::Version> = jars.iter().filter_map(|j| {
            let h = crate::net::sha1_file(&dir.join(&j.file)).ok()?;
            by_hash.get(&h).map(|v| (j.file.clone(), v.clone()))
        }).collect();

        // ── Conjunto completo antes de proponer nada ──
        // Qué mods solo funcionan en el cliente, qué exige cada mod del servidor y qué dependencias
        // ya se corrigieron con el archivo oficial de Fabric. Con esto una misma dependencia nunca
        // recibe dos reparaciones opuestas («apartar» y «volver a instalar»): ese era el bucle de
        // Mod Menu (Stone Vaults, Dungeons and Towers… lo declaran como obligatorio).
        let run_dir = paths::server_run_dir(&cfg.id);
        let overrides = fabric_overrides(&run_dir);
        let client_jar_of: HashMap<String, &JarInfo> = jars.iter().filter(|j| !j.corrupt && j.environment == "client").flat_map(|j| j.provides.iter().map(move |p| (p.clone(), j))).collect();
        let server_provided: BTreeSet<String> = jars.iter().filter(|j| !j.corrupt && j.environment != "client").flat_map(|j| j.provides.iter().cloned()).collect();
        let mut server_side_cache: HashMap<String, Option<modrinth::Project>> = HashMap::new();
        // dependencia → mods del servidor que la exigen (id, nombre)
        let mut needs: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for j in jars.iter().filter(|j| !j.corrupt && j.environment != "client" && !j.kind.is_empty()) {
            for dep in j.depends.keys() {
                if BUILTIN.contains(&dep.as_str()) || overrides.contains(&(j.id.clone(), dep.clone())) { continue; }
                needs.entry(dep.clone()).or_default().push((j.id.clone(), if j.name.is_empty() { j.id.clone() } else { j.name.clone() }));
            }
        }
        let mut client_deps: BTreeSet<String> = BTreeSet::new();
        for dep in needs.keys() {
            if server_provided.contains(dep) { continue; }
            let client = if client_jar_of.contains_key(dep) || KNOWN_CLIENT_ONLY.contains(&dep.as_str()) { true } else if online {
                let p = src.projects(&[modrinth_slug(dep)]).await.ok().and_then(|p| p.into_iter().next());
                let c = p.as_ref().map(|p| !modrinth::runs_on_server(&p.server_side)).unwrap_or(false);
                server_side_cache.insert(dep.clone(), p);
                c
            } else { false };
            if client { client_deps.insert(dep.clone()); }
        }

        // Duplicados por id.
        let mut seen: HashMap<String, &JarInfo> = HashMap::new();
        for j in jars.iter().filter(|j| !j.id.is_empty()) {
            if let Some(prev) = seen.get(&j.id) {
                let older = if cmp_ver(&prev.version, &j.version) == std::cmp::Ordering::Less { prev.file.clone() } else { j.file.clone() };
                add(&mut problems, Problem { id: format!("dup-{}", j.id), kind: "duplicate".into(),
                    title: format!("{} está instalado dos veces", if j.name.is_empty() { &j.id } else { &j.name }),
                    explanation: "Hay dos archivos del mismo mod. Minecraft no arranca con duplicados.".into(),
                    fix: Some(Fix { summary: "Apartar la versión más vieja".into(), changes: vec![format!("Apartar {older} (va a la Papelera de 939 Host)")], actions: vec![Action::SetAside { file: older }], needs_authorization: false }),
                    technical: vec![prev.file.clone(), j.file.clone()] });
            } else { seen.insert(j.id.clone(), j); }
        }

        for j in &jars {
            let label = if j.name.is_empty() { j.file.trim_end_matches(".jar").to_string() } else { j.name.clone() };
            let replacement = |reason: &str| -> Option<Fix> {
                let v = file_version.get(&j.file)?;
                Some(Fix { summary: format!("Buscar la versión de {label} para {} {}", cfg.loader.name(), cfg.mc_version), changes: vec![reason.to_string()], actions: vec![Action::SetAside { file: j.file.clone() }, Action::InstallVersion { project_id: v.project_id.clone(), version_id: String::new(), title: label.clone(), version_number: String::new(), replaces: Some(j.file.clone()) }], needs_authorization: false })
            };
            if j.corrupt {
                add(&mut problems, Problem { id: format!("corrupt-{}", j.file), kind: "corrupt".into(),
                    title: format!("El archivo {} está dañado", j.file),
                    explanation: "No se puede abrir como un mod: la descarga se cortó o el archivo se rompió.".into(),
                    fix: Some(Fix { summary: "Apartar el archivo dañado".into(), changes: vec![format!("Apartar {} (va a la Papelera)", j.file)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }),
                    technical: vec![] });
                continue;
            }
            if j.kind.is_empty() { continue; }
            let fabric_server = want == "fabric";
            let wrong_loader = (fabric_server && j.kind != "fabric") || (!fabric_server && j.kind == "fabric");
            if wrong_loader {
                let fix = replacement("Se aparta el archivo y se instala la versión correcta desde Modrinth, si existe").or_else(|| Some(Fix { summary: format!("Apartar {label}"), changes: vec![format!("{} es para {}; se aparta a la Papelera", j.file, j.kind)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }));
                add(&mut problems, Problem { id: format!("loader-{}", j.file), kind: "loader".into(),
                    title: format!("{label} es para otro tipo de servidor"),
                    explanation: format!("Este mod es para {}, pero el servidor usa {}. No pueden cargarse juntos.", match j.kind.as_str() { "fabric" => "Fabric", "neoforge" => "NeoForge", _ => "Forge" }, cfg.loader.name()),
                    fix, technical: vec![j.file.clone()] });
                continue;
            }
            if j.environment == "client" {
                // Si un mod del servidor lo exige, se resuelve junto con esa dependencia (abajo).
                if j.provides.iter().any(|p| client_deps.contains(p) && needs.contains_key(p)) { continue; }
                add(&mut problems, Problem { id: format!("client-{}", j.file), kind: "client_only".into(),
                    title: format!("{label} es solo para el Minecraft de cada jugador"),
                    explanation: "Es un mod de cliente: no tiene que estar en el servidor y puede impedir que arranque.".into(),
                    fix: Some(Fix { summary: format!("Apartar {label} del servidor"), changes: vec![format!("Apartar {} (va a la Papelera). En tu Minecraft lo seguís usando.", j.file)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }),
                    technical: vec![] });
                continue;
            }
            let range_ok = |conds: &Vec<String>, v: &str| if j.kind == "fabric" { conds.iter().any(|c| fabric_matches(c, v)) } else { conds.iter().any(|c| maven_matches(c, v)) };
            if let Some(conds) = j.depends.get("minecraft") {
                if !range_ok(conds, &cfg.mc_version) {
                    let fix = replacement(&format!("La versión instalada pide Minecraft {}", conds.join(" o "))).or_else(|| Some(Fix { summary: format!("Apartar {label}"), changes: vec![format!("No se encontró una versión para {}; se aparta a la Papelera", cfg.mc_version)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }));
                    add(&mut problems, Problem { id: format!("mc-{}", j.file), kind: "mc_version".into(),
                        title: format!("{label} es para otra versión de Minecraft"),
                        explanation: format!("Instalaste la versión {} de {label}, que pide Minecraft {}. Este servidor usa {}.", j.version, conds.join(" o "), cfg.mc_version),
                        fix, technical: vec![format!("{} → minecraft {}", j.file, conds.join(" | "))] });
                }
            }
            for (dep, conds) in &j.depends {
                if BUILTIN.contains(&dep.as_str()) || overrides.contains(&(j.id.clone(), dep.clone())) || client_deps.contains(dep) { continue; }
                if provided.contains(dep) {
                    // Está, pero ¿en una versión que este mod acepta?
                    if let Some(d) = jars.iter().find(|x| !x.corrupt && x.id == *dep && !x.version.is_empty()) {
                        if !range_ok(conds, &d.version) {
                            let dl = if d.name.is_empty() { d.id.clone() } else { d.name.clone() };
                            add(&mut problems, Problem { id: format!("depver-{dep}-{}", j.id), kind: "dependency_version".into(),
                                title: format!("{label} necesita otra versión de {dl}"),
                                explanation: format!("{label} pide {dl} {}, y está instalada la {}.", conds.join(" o "), d.version),
                                fix: Some(Fix { summary: format!("Cambiar {dl} por una versión compatible con {} {}", cfg.loader.name(), cfg.mc_version), changes: vec![format!("Apartar {} e instalar la versión compatible de Modrinth (si existe)", d.file)],
                                    actions: vec![Action::SetAside { file: d.file.clone() }, Action::InstallVersion { project_id: modrinth_slug(dep), version_id: String::new(), title: dl.clone(), version_number: String::new(), replaces: Some(d.file.clone()) }], needs_authorization: false }),
                                technical: vec![format!("{} depende de {dep} {}; instalada {}", j.file, conds.join(" | "), d.version)] });
                        }
                    }
                    continue;
                }
                let slug = modrinth_slug(dep);
                let found = match server_side_cache.get(dep) { Some(p) => p.clone(), None if online => src.projects(&[slug.clone()]).await.ok().and_then(|p| p.into_iter().next()), None => None };
                add(&mut problems, Problem { id: format!("dep-{dep}"), kind: "missing_dependency".into(),
                    title: format!("A {label} le falta {dep}"),
                    explanation: format!("{label} necesita «{dep}» para funcionar, y no está instalado."),
                    fix: found.map(|p| Fix { summary: format!("Instalar {} desde Modrinth", p.title), changes: vec![format!("Agregar {} compatible con {} {}", p.title, cfg.loader.name(), cfg.mc_version)], actions: vec![Action::InstallVersion { project_id: p.id.clone(), version_id: String::new(), title: p.title.clone(), version_number: String::new(), replaces: None }], needs_authorization: false }),
                    technical: vec![format!("{} depende de {dep} {}", j.file, conds.join(" | "))] });
            }
        }
        // Incompatibilidades declaradas entre dos mods instalados.
        for j in jars.iter().filter(|j| !j.corrupt) {
            for (other, conds) in &j.breaks {
                let Some(o) = jars.iter().find(|x| !x.corrupt && x.provides.contains(other) && x.file != j.file) else { continue };
                let ok = if j.kind == "fabric" { conds.iter().any(|c| fabric_matches(c, &o.version)) } else { conds.iter().any(|c| maven_matches(c, &o.version)) };
                if !ok { continue; }
                let (a, b) = (if j.name.is_empty() { &j.id } else { &j.name }, if o.name.is_empty() { &o.id } else { &o.name });
                add(&mut problems, Problem { id: format!("breaks-{}-{}", j.id, o.id), kind: "conflict".into(),
                    title: format!("{a} no funciona junto con {b}"),
                    explanation: format!("{a} declara que es incompatible con {b} {}. Hay que elegir uno de los dos (o una versión de {b} que no esté en ese rango).", o.version),
                    fix: None, technical: vec![format!("{} rompe {other} {}", j.file, conds.join(" | "))] });
            }
        }

        // Dependencias obligatorias que solo existen en el cliente.
        for dep in &client_deps {
            let Some(who) = needs.get(dep) else { continue };
            let client_jar = client_jar_of.get(dep);
            let dl = client_jar.map(|c| if c.name.is_empty() { c.id.clone() } else { c.name.clone() }).unwrap_or_else(|| dep.clone());
            let names: Vec<String> = who.iter().map(|(_, n)| n.clone()).collect();
            let list = match names.len() { 1 => names[0].clone(), _ => format!("{} y {}", names[..names.len() - 1].join(", "), names[names.len() - 1]) };
            let mut actions: Vec<Action> = who.iter().map(|(m, _)| Action::IgnoreClientDependency { mod_id: m.clone(), dep: dep.clone() }).collect();
            let mut changes = vec![format!("En {FABRIC_OVERRIDES} (archivo oficial de Fabric Loader): {list} deja{} de exigir «{dep}» en el servidor. No se modifica ningún mod.", if names.len() > 1 { "n" } else { "" })];
            if let Some(c) = client_jar { actions.push(Action::SetAside { file: c.file.clone() }); changes.push(format!("Apartar {} del servidor (va a la Papelera)", c.file)); }
            changes.push(format!("{dl} sigue en el Minecraft de cada jugador, donde sí funciona."));
            add(&mut problems, Problem { id: format!("cdep-{dep}"), kind: "client_dependency".into(),
                title: format!("{list} {} {dl}, que solo funciona en el cliente", if names.len() > 1 { "piden" } else { "pide" }),
                explanation: format!("{dl} es un mod de cliente: en un servidor dedicado no se carga. {list} lo declara{} como obligatorio aunque en el servidor no lo usa{}. Por eso, al quitarlo, el servidor lo volvía a pedir. La solución no es instalarlo en el servidor: es indicarle a Fabric que no lo exija ahí.",
                    if names.len() > 1 { "n" } else { "" }, if names.len() > 1 { "n" } else { "" }),
                fix: (want == "fabric").then(|| Fix { summary: format!("Que el servidor no exija {dl} y dejarlo solo en el cliente"), changes, actions, needs_authorization: false }),
                technical: who.iter().map(|(m, _)| format!("{m} depende de {dep} (obligatoria); {dep} es solo de cliente")).collect() });
        }

        // Error de mixin que nombra a un mod (solo si no se encontró otra causa).
        if problems.is_empty() {
            if let Some(c) = RE_MIXIN_MOD.captures(&log) {
                let mid = c[1].to_string();
                if let Some(j) = jars.iter().find(|j| j.id == mid) {
                    add(&mut problems, Problem { id: format!("mixin-{mid}"), kind: "mod_crash".into(),
                        title: format!("{} hizo fallar el arranque", if j.name.is_empty() { &j.id } else { &j.name }),
                        explanation: "Este mod chocó con Minecraft o con otro mod al cargar. Suele resolverse quitándolo o actualizándolo.".into(),
                        fix: Some(Fix { summary: format!("Apartar {} y volver a intentar", j.file), changes: vec![format!("Apartar {} (va a la Papelera)", j.file)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: true }),
                        technical: tail(&log, &c[0], 4) });
                }
            }
        }
    }

    // Completar las versiones concretas de Modrinth para las acciones de instalación.
    if online {
        let loader = loader_kind(cfg.loader);
        for p in problems.iter_mut() {
            let Some(fix) = p.fix.as_mut() else { continue };
            let mut ok = true;
            for a in fix.actions.iter_mut() {
                if let Action::InstallVersion { project_id, version_id, version_number, .. } = a {
                    if version_id.is_empty() {
                        match src.project_versions(project_id, &cfg.mc_version, loader).await.ok().and_then(|l| modrinth::best_version(l, &cfg.mc_version, loader)) {
                            Some(v) => { *version_id = v.id.clone(); *version_number = v.version_number.clone(); }
                            None => ok = false,
                        }
                    }
                }
            }
            if !ok {
                // Sin versión compatible: queda solo apartar (si estaba) o ninguna reparación.
                fix.actions.retain(|a| !matches!(a, Action::InstallVersion { version_id, .. } if version_id.is_empty()));
                if fix.actions.is_empty() { p.fix = None; } else { fix.summary = "Apartar el mod (no hay una versión compatible en Modrinth)".into(); }
            } else {
                for a in &fix.actions { if let Action::InstallVersion { title, version_number, .. } = a { fix.changes.push(format!("Instalar {title} {version_number} (hash verificado)")); } }
            }
        }
    } else {
        for p in problems.iter_mut() {
            if let Some(fix) = p.fix.as_mut() {
                fix.actions.retain(|a| !matches!(a, Action::InstallVersion { .. }));
                if fix.actions.is_empty() { p.fix = None; }
            }
        }
    }

    // El último arranque falló y ninguna regla lo explica: no se dice «no hay problemas». Se
    // muestran las líneas de error reales del registro para que se vea qué pasó.
    let crashed = log.contains("se cerró inesperadamente") || log.contains("Crash report") || log.contains("---- Minecraft Crash Report");
    if problems.is_empty() && crashed {
        let errs: Vec<String> = log.lines().filter(|l| {
            let t = l.trim();
            !t.starts_with("── 939 Host") && (t.contains("ERROR") || t.contains("FATAL") || t.contains("Exception") || t.starts_with("Caused by") || t.contains("Error:") || t.starts_with("[stderr]") || t.contains("Description:"))
        }).take(14).map(|l| l.trim().chars().take(400).collect()).collect();
        add(&mut problems, Problem { id: "unknown".into(), kind: "unknown".into(),
            title: "El servidor no arrancó y no reconocemos la causa".into(),
            explanation: if errs.is_empty() {
                "El servidor se cerró sin dejar un error claro en el registro. Abrí la consola para ver lo último que escribió; si agregaste mods hace poco, probá quitarlos de a uno.".into()
            } else {
                "939 Host no tiene una solución automática segura para este error. Abajo están las líneas reales del registro: suelen nombrar el mod o el archivo que falló.".into()
            },
            fix: None, technical: errs });
    }

    let fixable = problems.iter().any(|p| p.fix.is_some());
    let summary = match problems.len() {
        0 => "No encontramos un problema conocido.".into(),
        1 => "Encontramos 1 problema.".into(),
        n => format!("Encontramos {n} problemas."),
    };
    Diagnosis { problems, fixable, summary }
}


fn add(list: &mut Vec<Problem>, p: Problem) { if !list.iter().any(|x| x.id == p.id) { list.push(p); } }


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairRecord {
    pub snap_id: Option<String>,
    pub config_before: ServerConfig,
    pub applied: Vec<String>,
    pub at: String,
}

fn record_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("ultima-reparacion.json") }

pub fn last_record(id: &str) -> Option<RepairRecord> { paths::read_json(&record_path(id)).ok() }

/// Aplica las acciones con un punto de restauración previo. El servidor tiene que estar apagado.
pub async fn apply<S: Source>(src: &S, mgr: &Manager, cfg: &ServerConfig, actions: &[Action], mut progress: impl FnMut(&str, f64)) -> AppResult<RepairRecord> {
    if mgr.runtime(&cfg.id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de reparar.")); }
    progress("Creando punto de restauración", 0.05);
    let snap = crate::snapshots::take(mgr, &cfg.id, "Antes de reparar", "seguridad", &cfg.mc_version).ok().map(|s| s.id);
    let before = cfg.clone();
    let mut cfg = cfg.clone();
    let mut applied = vec![];
    let n = actions.len().max(1) as f64;
    for (i, a) in actions.iter().enumerate() {
        let frac = 0.1 + 0.8 * (i as f64 / n);
        match a {
            Action::SetAside { file } => {
                progress(&format!("Apartando {file}"), frac);
                if mods::mods_dir(&cfg.id).join(file).exists() { mods::set_aside_file(&cfg, file)?; applied.push(format!("Apartado {file}")); }
            }
            Action::InstallVersion { project_id, version_id, title, version_number, .. } => {
                progress(&format!("Descargando {title}"), frac);
                let plan = mods::plan_install(src, &cfg, project_id, (!version_id.is_empty()).then_some(version_id.as_str())).await?;
                mods::apply_plan(&cfg, &plan, |m, _| progress(m, frac)).await?;
                applied.push(format!("Instalado {title} {version_number}"));
            }
            Action::InstallJava { major } => {
                progress(&format!("Descargando Java {major}"), frac);
                crate::java::ensure(*major, |m, _| progress(m, frac)).await?;
                cfg.java_major = *major;
                applied.push(format!("Java {major}"));
            }
            Action::SetPort { port } => {
                // Se vuelve a comprobar justo antes de aplicarlo (otro programa pudo tomarlo).
                let port = if crate::ports::is_free(*port) { *port } else { crate::ports::free_near(*port, &[]).ok_or_else(|| AppError::new("port", "No se encontró un puerto libre."))? };
                cfg.port = port; applied.push(format!("Puerto {port}"));
            }
            Action::SetMemory { mb } => { cfg.memory_mb = *mb; applied.push(format!("Memoria {mb} MB")); }
            Action::AcceptEula => { cfg.eula_accepted_at = Some(chrono::Local::now().to_rfc3339()); applied.push("EULA aceptado".into()); }
            Action::IgnoreClientDependency { mod_id, dep } => {
                progress(&format!("Ajustando dependencias de {mod_id}"), frac);
                add_fabric_override(&paths::server_run_dir(&cfg.id), mod_id, dep)?;
                applied.push(format!("{mod_id} ya no exige {dep} en el servidor"));
            }
            Action::RestoreSnapshot { snap_id, label } => {
                progress(&format!("Restaurando «{label}»"), frac);
                crate::snapshots::restore(mgr, &cfg.id, snap_id, &cfg.mc_version)?;
                applied.push(format!("Restaurado «{label}»"));
            }
        }
    }
    crate::instance::save(&cfg)?;
    crate::properties::write_for(&cfg)?;
    let rec = RepairRecord { snap_id: snap, config_before: before, applied, at: chrono::Local::now().to_rfc3339() };
    paths::write_json(&record_path(&cfg.id), &rec)?;
    progress("Cambios aplicados", 0.9);
    Ok(rec)
}

/// Deshace la última reparación: vuelve al punto «Antes de reparar» y a la configuración previa.
pub fn undo(mgr: &Manager, id: &str) -> AppResult<()> {
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de deshacer.")); }
    let rec = last_record(id).ok_or_else(|| AppError::not_found("una reparación para deshacer"))?;
    if let Some(s) = &rec.snap_id { crate::snapshots::restore(mgr, id, s, &rec.config_before.mc_version)?; }
    crate::instance::save(&rec.config_before)?;
    crate::properties::write_for(&rec.config_before)?;
    let _ = std::fs::remove_file(record_path(id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_ranges() {
        assert!(fabric_matches("*", "26.2"));
        assert!(fabric_matches(">=1.21.4", "1.21.5"));
        assert!(!fabric_matches(">=1.21.4", "1.21.3"));
        assert!(fabric_matches("1.21.x", "1.21.9"));
        assert!(!fabric_matches("1.21.x", "1.22"));
        assert!(fabric_matches(">=1.21 <1.22", "1.21.8"));
        assert!(!fabric_matches(">=1.21 <1.22", "1.22.0"));
        assert!(fabric_matches("~1.21.4", "1.21.6"));
        assert!(!fabric_matches("~1.21.4", "1.22"));
        assert!(fabric_matches("26.2", "26.2"));
        assert!(!fabric_matches("26.3", "26.2"));
        assert!(fabric_matches(">=26.1", "26.2"));
    }

    #[test]
    fn reads_nested_provides() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let mut inner = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o: zip::write::SimpleFileOptions = Default::default();
        inner.start_file("fabric.mod.json", o).unwrap();
        inner.write_all(br#"{"id":"fabric-lifecycle-events-v1","version":"1"}"#).unwrap();
        let inner_bytes = inner.finish().unwrap().into_inner();
        let p = tmp.path().join("fabric-api.jar");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("fabric.mod.json", o).unwrap();
        z.write_all(br#"{"id":"fabric-api","name":"Fabric API","version":"0.161","provides":["fabric"],"depends":{"minecraft":">=26.2","fabricloader":"*"},"jars":[{"file":"META-INF/jars/life.jar"}]}"#).unwrap();
        z.start_file("META-INF/jars/life.jar", o).unwrap();
        z.write_all(&inner_bytes).unwrap();
        z.finish().unwrap();
        let j = read_jar(&p);
        assert_eq!(j.kind, "fabric");
        assert!(j.provides.contains("fabric") && j.provides.contains("fabric-lifecycle-events-v1") && j.provides.contains("fabric-api"));
        assert_eq!(j.depends["minecraft"], vec![">=26.2".to_string()]);
        std::fs::write(tmp.path().join("roto.jar"), b"no es un zip").unwrap();
        assert!(read_jar(&tmp.path().join("roto.jar")).corrupt);
    }

    // ───── Regresión del bucle de Mod Menu ─────

    struct Offline;
    impl Source for Offline {
        async fn version(&self, id: &str) -> AppResult<modrinth::Version> { Err(AppError::not_found(id)) }
        async fn project_versions(&self, _: &str, _: &str, _: &str) -> AppResult<Vec<modrinth::Version>> { Ok(vec![]) }
        async fn projects(&self, _: &[String]) -> AppResult<Vec<modrinth::Project>> { Ok(vec![]) }
        async fn versions_by_sha1(&self, _: &[String]) -> AppResult<HashMap<String, modrinth::Version>> { Ok(HashMap::new()) }
    }

    fn cfg_fabric() -> ServerConfig {
        use crate::model::JvmProfile;
        let _ = paths::init(std::env::temp_dir().join("939host-repair-tests"));
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        let id = format!("r{}", hex::encode(b));
        std::fs::create_dir_all(paths::server_run_dir(&id).join("mods")).unwrap();
        ServerConfig {
            external_access: None, id, name: "T".into(), logo: None, description: String::new(), mc_version: "26.2".into(), loader: Loader::Fabric, loader_version: None, java_major: 25, java_required: 25, java_choice: None,
            // Puerto libre: otras pruebas en paralelo pueden usar el 25565 (y el reparador lo diría).
            memory_mb: 2048, jvm_profile: JvmProfile::Balanced, extra_jvm_args: String::new(), port: crate::ports::free_near(47000, &[]).unwrap_or(47001), max_players: 8, motd: String::new(),
            online_mode: true, whitelist: true, difficulty: "hard".into(), gamemode: "survival".into(), hardcore: false, pvp: true, level_seed: String::new(),
            level_type: "minecraft:normal".into(), view_distance: 10, simulation_distance: 8, spawn_protection: 16, allow_flight: false, enable_command_block: false,
            sync_interval_min: 15, restore_interval_min: 15, installed: true, install_error: None, eula_accepted_at: Some("hoy".into()), created_at: String::new(),
            identity: Default::default(), extra_properties: Default::default(),
        }
    }

    fn jar(dir: &Path, file: &str, entry: &str, body: &str) {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(dir.join(file)).unwrap());
        z.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body.as_bytes()).unwrap();
        z.finish().unwrap();
    }

    /// Metadatos como los reales: Mod Menu es `environment: client`; Stone Vaults y Dungeons and
    /// Towers lo declaran en `depends` (obligatorio) y JEI/EMI solo en `suggests`.
    fn mods_like_the_real_ones(dir: &Path, with_modmenu: bool) {
        jar(dir, "fabric-api.jar", "fabric.mod.json", r#"{"id":"fabric-api","name":"Fabric API","version":"0.140.0","provides":["fabric"],"depends":{"fabricloader":"*","minecraft":">=26.2"}}"#);
        jar(dir, "stonevaults.jar", "fabric.mod.json", r#"{"id":"stonevaults","name":"Stone Vaults","version":"1.4","environment":"*","depends":{"fabric-api":"*","modmenu":"*","minecraft":"26.2"},"suggests":{"emi":"*"}}"#);
        jar(dir, "dungeons-and-towers.jar", "fabric.mod.json", r#"{"id":"mr_dungeons_andtaverns","name":"Dungeons and Towers","version":"5.0","depends":{"modmenu":">=10","minecraft":"26.2"},"recommends":{"jei":"*"}}"#);
        if with_modmenu { jar(dir, "modmenu.jar", "fabric.mod.json", r#"{"id":"modmenu","name":"Mod Menu","version":"17.0.0","environment":"client","depends":{"minecraft":"26.2"}}"#); }
    }

    #[tokio::test]
    async fn mod_menu_no_genera_un_bucle() {
        let cfg = cfg_fabric();
        let dir = mods::mods_dir(&cfg.id);
        mods_like_the_real_ones(&dir, true);
        let mgr = Manager::new(std::sync::Arc::new(crate::events::NoEvents));
        let d = analyze(&Offline, &mgr, &cfg, 8192, false).await;
        // Una sola explicación para Mod Menu: ni «apartalo» por un lado ni «instalalo» por el otro.
        assert!(!d.problems.iter().any(|p| p.kind == "client_only"), "{:?}", d.problems.iter().map(|p| &p.title).collect::<Vec<_>>());
        assert!(!d.problems.iter().any(|p| p.kind == "missing_dependency"), "una dependencia de cliente no se manda a instalar");
        assert!(!d.problems.iter().any(|p| p.title.contains("emi") || p.title.contains("jei")), "las opcionales no son problemas");
        let p = d.problems.iter().find(|p| p.kind == "client_dependency").expect("explica la causa real");
        assert!(p.title.contains("Stone Vaults") && p.title.contains("Dungeons and Towers") && p.title.contains("Mod Menu"), "{}", p.title);
        let fix = p.fix.clone().unwrap();
        assert!(fix.actions.contains(&Action::SetAside { file: "modmenu.jar".into() }));
        assert!(fix.actions.contains(&Action::IgnoreClientDependency { mod_id: "stonevaults".into(), dep: "modmenu".into() }));
        assert!(!fix.actions.iter().any(|a| matches!(a, Action::InstallVersion { .. })));

        // Se aplica y se vuelve a analizar: ya no queda nada que reparar (sin bucle).
        apply(&Offline, &mgr, &cfg, &fix.actions, |_, _| {}).await.unwrap();
        assert!(!dir.join("modmenu.jar").exists());
        let ov = fabric_overrides(&paths::server_run_dir(&cfg.id));
        assert!(ov.contains(&("stonevaults".into(), "modmenu".into())) && ov.contains(&("mr_dungeons_andtaverns".into(), "modmenu".into())));
        let again = analyze(&Offline, &mgr, &cfg, 8192, false).await;
        assert!(again.problems.is_empty(), "después de reparar no vuelve a pedir nada: {:?}", again.problems.iter().map(|p| &p.title).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn fabric_viejo_con_java_8_explica_la_causa_y_un_error_desconocido_no_dice_que_esta_todo_bien() {
        let mut cfg = cfg_fabric();
        cfg.id = "rep-launch".into(); cfg.mc_version = "1.14".into(); cfg.java_major = 8;
        let logs = paths::server_run_dir(&cfg.id).join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        // El registro real del servidor Fabric 1.14 que no arrancaba.
        std::fs::write(logs.join("latest.log"), "── 939 Host: iniciando prueba 1.14 (Fabric 1.14) con Java 8 ──\n[stderr] Error: no se ha encontrado o cargado la clase principal @939host-fabric-args.txt\n── 939 Host: El servidor se cerró inesperadamente (código 1). Revisá la consola. ──\n").unwrap();
        let mgr = Manager::new(std::sync::Arc::new(crate::events::NoEvents));
        let d = analyze(&Offline, &mgr, &cfg, 8192, false).await;
        let p = d.problems.iter().find(|p| p.kind == "launch").expect("reconoce el arranque fallido");
        assert!(p.title.contains("Java 8") && p.explanation.contains("Java 9"), "{}", p.title);
        // Un cierre con un error que ninguna regla conoce: se muestran las líneas reales.
        std::fs::write(logs.join("latest.log"), "── 939 Host: iniciando x ──\n[12:00:00] [main/ERROR]: Algo raro pasó en com.ejemplo.Mod\njava.lang.IllegalStateException: raro\n── 939 Host: El servidor se cerró inesperadamente (código 1). Revisá la consola. ──\n").unwrap();
        let d = analyze(&Offline, &mgr, &cfg, 8192, false).await;
        let p = d.problems.iter().find(|p| p.kind == "unknown").expect("no dice «no encontramos un problema» si no arrancó");
        assert!(p.technical.iter().any(|l| l.contains("IllegalStateException")) && p.fix.is_none());
        assert!(d.problems.len() == 1);
    }

    #[tokio::test]
    async fn sin_mod_menu_tampoco_lo_manda_a_instalar() {
        let cfg = cfg_fabric();
        mods_like_the_real_ones(&mods::mods_dir(&cfg.id), false);
        let mgr = Manager::new(std::sync::Arc::new(crate::events::NoEvents));
        let d = analyze(&Offline, &mgr, &cfg, 8192, false).await;
        let p = d.problems.iter().find(|p| p.kind == "client_dependency").expect("reconoce Mod Menu como de cliente aunque no esté");
        assert!(!p.fix.as_ref().unwrap().actions.iter().any(|a| matches!(a, Action::InstallVersion { .. } | Action::SetAside { .. })));
    }

    #[test]
    fn importar_no_copia_mod_menu_y_ajusta_la_dependencia() {
        let cfg = cfg_fabric();
        let src = tempfile::tempdir().unwrap();
        mods_like_the_real_ones(src.path(), true);
        let r = mods::import_folder(&cfg, src.path()).unwrap();
        assert!(r.skipped_client.iter().any(|x| x.contains("Mod Menu")));
        assert!(!mods::mods_dir(&cfg.id).join("modmenu.jar").exists());
        assert_eq!(r.client_deps_ignored.len(), 2, "{:?}", r.client_deps_ignored);
        assert!(fabric_overrides(&paths::server_run_dir(&cfg.id)).contains(&("stonevaults".into(), "modmenu".into())));
    }

    #[test]
    fn archivo_de_fabric_conserva_lo_que_habia() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::write(tmp.path().join(FABRIC_OVERRIDES), r#"{"version":1,"overrides":{"otro":{"+depends":{"x":">=2"}}}}"#).unwrap();
        add_fabric_override(tmp.path(), "stonevaults", "modmenu").unwrap();
        add_fabric_override(tmp.path(), "stonevaults", "modmenu").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(tmp.path().join(FABRIC_OVERRIDES)).unwrap()).unwrap();
        assert_eq!(v["overrides"]["otro"]["+depends"]["x"], ">=2", "no se pierde lo que puso otra persona");
        assert_eq!(v["overrides"]["stonevaults"]["-depends"]["modmenu"], "*");
    }

    #[test]
    fn forge_y_neoforge_distinguen_obligatorias_opcionales_y_de_cliente() {
        let toml = r#"
modLoader="javafml"
[[mods]]
modId="aventura"
displayName="Aventura"
version="2.0"
[[dependencies.aventura]]
    modId="forge"
    mandatory=true
    versionRange="[47,)"
    side="BOTH"
[[dependencies.aventura]]
    modId="geckolib"
    mandatory=true
    versionRange="[4.4,5)"
    side="BOTH"
[[dependencies.aventura]]
    modId="jei"
    mandatory=false
    side="BOTH"
[[dependencies.aventura]]
    modId="oculus" # solo pantalla
    type="required"
    side="CLIENT"
[[dependencies.aventura]]
    modId="optifine"
    type="incompatible"
    versionRange="[1,)"
"#;
        let d = forge_deps(toml);
        assert_eq!(d.len(), 5);
        let tmp = tempfile::tempdir().unwrap();
        jar(tmp.path(), "aventura.jar", "META-INF/mods.toml", toml);
        let j = read_jar(&tmp.path().join("aventura.jar"));
        assert!(j.depends.contains_key("geckolib") && j.depends.contains_key("forge"));
        assert!(!j.depends.contains_key("jei") && j.optional.contains("jei"), "opcional");
        assert!(!j.depends.contains_key("oculus") && j.optional.contains("oculus"), "de lado cliente no es obligatoria en el servidor");
        assert!(j.breaks.contains_key("optifine"));
        assert!(maven_matches("[4.4,5)", "4.6.1") && !maven_matches("[4.4,5)", "5.0") && !maven_matches("[4.4,5)", "4.3"));
        assert!(maven_matches("[47,)", "52.0.1") && maven_matches("(,2.0]", "2.0") && !maven_matches("(,2.0)", "2.0"));
        assert!(maven_matches("[1.20.1,1.21),[1.21.4,1.22)", "1.21.5") && !maven_matches("[1.20.1,1.21),[1.21.4,1.22)", "1.21.2"));
    }

    /// Importar desde cualquier carpeta: un mod que no está en ningún catálogo entra igual; uno para
    /// otro loader u otra versión de Minecraft no se copia y se explica por qué.
    #[test]
    fn importar_desde_cualquier_carpeta_con_compatibilidad() {
        let cfg = cfg_fabric();
        let src = tempfile::tempdir().unwrap();
        let sub = src.path().join("Descargas").join("mis mods");
        std::fs::create_dir_all(&sub).unwrap();
        jar(&sub, "mi-mod-propio.jar", "fabric.mod.json", r#"{"id":"mimodpropio","name":"Mi mod propio","version":"0.1","depends":{"minecraft":"~26.2"}}"#);
        jar(&sub, "viejo.jar", "fabric.mod.json", r#"{"id":"viejo","name":"Mod viejo","version":"1","depends":{"minecraft":"1.20.1"}}"#);
        jar(&sub, "forge.jar", "META-INF/mods.toml", "[[mods]]\nmodId=\"deforge\"\nversion=\"1\"\n");
        let files: Vec<std::path::PathBuf> = ["mi-mod-propio.jar", "viejo.jar", "forge.jar"].iter().map(|f| sub.join(f)).collect();
        let r = mods::import_files(&cfg, &files).unwrap();
        assert_eq!(r.copied, vec!["mi-mod-propio.jar".to_string()]);
        assert_eq!(r.skipped_incompatible.len(), 2, "{:?}", r.skipped_incompatible);
        assert!(r.skipped_incompatible.iter().any(|x| x.contains("1.20.1")));
        assert!(r.skipped_incompatible.iter().any(|x| x.contains("Forge")));
        assert!(sub.join("viejo.jar").exists(), "los originales no se tocan");
    }

    #[test]
    fn eliminar_todos_los_mods_sin_tocar_lo_demas() {
        let cfg = cfg_fabric();
        let run = paths::server_run_dir(&cfg.id);
        let dir = mods::mods_dir(&cfg.id);
        mods_like_the_real_ones(&dir, true);
        std::fs::write(dir.join("notas.txt"), "no es un mod").unwrap();
        std::fs::create_dir_all(run.join("config")).unwrap();
        std::fs::write(run.join("config/stonevaults.json"), "{}").unwrap();
        std::fs::create_dir_all(run.join("world")).unwrap();
        std::fs::write(run.join("world/level.dat"), "mundo").unwrap();
        let moved = mods::remove_all(&cfg).unwrap();
        assert_eq!(moved.len(), 4);
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".jar")));
        assert!(dir.join("notas.txt").exists() && run.join("config/stonevaults.json").exists() && run.join("world/level.dat").exists());
        assert!(mods::load_manifest(&cfg.id).mods.is_empty());
        assert!(mods::remove_all(&cfg).unwrap().is_empty(), "sin mods no hace nada");
    }
}
