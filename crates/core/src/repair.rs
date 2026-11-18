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
    /// id de dependencia → condiciones (alguna tiene que cumplirse)
    pub depends: BTreeMap<String, Vec<String>>,
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
    }
    info
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
    let active = mgr.runtime(&cfg.id).lock().unwrap().is_active();
    // Solo si el puerto sigue ocupado ahora: un registro viejo con "FAILED TO BIND" no alcanza.
    if !active && port_busy_any(cfg.port) {
        let port = free_port(cfg.port);
        add(&mut problems, Problem { id: "port".into(), kind: "port".into(),
            title: format!("El puerto {} está ocupado por otro programa", cfg.port),
            explanation: "Otro programa (u otro servidor) ya usa ese puerto, así que Minecraft no puede escuchar ahí.".into(),
            fix: port.map(|p| Fix { summary: format!("Usar el puerto libre {p}"), changes: vec![format!("Puerto {} → {p}. Tus amigos que entran con 939 Host no tienen que cambiar nada.", cfg.port)], actions: vec![Action::SetPort { port: p }], needs_authorization: false }),
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
                add(&mut problems, Problem { id: format!("client-{}", j.file), kind: "client_only".into(),
                    title: format!("{label} es solo para el Minecraft de cada jugador"),
                    explanation: "Es un mod de cliente: no tiene que estar en el servidor y puede impedir que arranque.".into(),
                    fix: Some(Fix { summary: format!("Apartar {label} del servidor"), changes: vec![format!("Apartar {} (va a la Papelera). En tu Minecraft lo seguís usando.", j.file)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }),
                    technical: vec![] });
                continue;
            }
            if let Some(conds) = j.depends.get("minecraft") {
                if !conds.iter().any(|c| fabric_matches(c, &cfg.mc_version)) {
                    let fix = replacement(&format!("La versión instalada pide Minecraft {}", conds.join(" o "))).or_else(|| Some(Fix { summary: format!("Apartar {label}"), changes: vec![format!("No se encontró una versión para {}; se aparta a la Papelera", cfg.mc_version)], actions: vec![Action::SetAside { file: j.file.clone() }], needs_authorization: false }));
                    add(&mut problems, Problem { id: format!("mc-{}", j.file), kind: "mc_version".into(),
                        title: format!("{label} es para otra versión de Minecraft"),
                        explanation: format!("Instalaste la versión {} de {label}, que pide Minecraft {}. Este servidor usa {}.", j.version, conds.join(" o "), cfg.mc_version),
                        fix, technical: vec![format!("{} → minecraft {}", j.file, conds.join(" | "))] });
                }
            }
            for (dep, conds) in &j.depends {
                if BUILTIN.contains(&dep.as_str()) || provided.contains(dep) { continue; }
                let slug = modrinth_slug(dep);
                let found = if online { src.projects(&[slug.clone()]).await.ok().and_then(|p| p.into_iter().next()) } else { None };
                add(&mut problems, Problem { id: format!("dep-{dep}"), kind: "missing_dependency".into(),
                    title: format!("A {label} le falta {dep}"),
                    explanation: format!("{label} necesita «{dep}» para funcionar, y no está instalado."),
                    fix: found.map(|p| Fix { summary: format!("Instalar {} desde Modrinth", p.title), changes: vec![format!("Agregar {} compatible con {} {}", p.title, cfg.loader.name(), cfg.mc_version)], actions: vec![Action::InstallVersion { project_id: p.id.clone(), version_id: String::new(), title: p.title.clone(), version_number: String::new(), replaces: None }], needs_authorization: false }),
                    technical: vec![format!("{} depende de {dep} {}", j.file, conds.join(" | "))] });
            }
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

    let fixable = problems.iter().any(|p| p.fix.is_some());
    let summary = match problems.len() {
        0 => "No encontramos un problema conocido.".into(),
        1 => "Encontramos 1 problema.".into(),
        n => format!("Encontramos {n} problemas."),
    };
    Diagnosis { problems, fixable, summary }
}

fn port_busy_any(port: u16) -> bool { std::net::TcpListener::bind(("0.0.0.0", port)).is_err() || std::net::TcpListener::bind(("127.0.0.1", port)).is_err() }

fn add(list: &mut Vec<Problem>, p: Problem) { if !list.iter().any(|x| x.id == p.id) { list.push(p); } }

fn free_port(current: u16) -> Option<u16> {
    (current.saturating_add(1)..=current.saturating_add(50)).chain(25565..25700).find(|p| *p >= 1024 && std::net::TcpListener::bind(("0.0.0.0", *p)).is_ok())
}

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
            Action::SetPort { port } => { cfg.port = *port; applied.push(format!("Puerto {port}")); }
            Action::SetMemory { mb } => { cfg.memory_mb = *mb; applied.push(format!("Memoria {mb} MB")); }
            Action::AcceptEula => { cfg.eula_accepted_at = Some(chrono::Local::now().to_rfc3339()); applied.push("EULA aceptado".into()); }
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
}
