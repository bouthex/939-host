//! Análisis de seguridad de un mod **sin ejecutarlo**.
//!
//! Lee el `.jar` como un archivo zip y busca indicadores concretos en su estructura, sus
//! metadatos y las cadenas de sus clases (el «constant pool» de Java guarda nombres de clases,
//! métodos y textos literales tal cual). Nunca carga ni ejecuta código del mod.
//!
//! Qué puede y qué no puede hacer:
//!
//! * Detecta indicadores conocidos: robo de credenciales (rutas de Discord, navegadores,
//!   cuentas de lanzadores), envío a webhooks, persistencia en Windows, scripts o ejecutables
//!   dentro del mod, propagación a otros `.jar` (como fractureiser), carga de código desde
//!   internet, ofuscación, librerías nativas, bibliotecas vulnerables incluidas y los
//!   indicadores públicos de fractureiser.
//! * **No** detecta todo: un autor que cifra sus cadenas o arma los nombres en tiempo de
//!   ejecución puede esconderse de un análisis estático. Por eso el resultado nunca reemplaza la
//!   aprobación de la propietaria y «sin alertas conocidas» no significa «seguro».
//! * Muchas cosas normales de un mod (usar la red, leer archivos, reflexión) **no** generan
//!   alertas por sí solas.
//!
//! La procedencia se consulta en Modrinth **solo por la huella** (SHA-512): no se sube el archivo.

use crate::error::AppResult;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

/// Sube cuando cambian las reglas: los análisis guardados con otra versión se rehacen.
pub const ANALYZER_VERSION: u32 = 2;

const MAX_ENTRIES: usize = 50_000;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level { Info, Precaucion, Sospechoso, Amenaza }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    pub code: String,
    /// Qué se encontró, en palabras.
    pub title: String,
    /// Dónde (archivo dentro del mod) y el indicador concreto.
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModMeta {
    pub loader: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub depends: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub project: String,
    pub title: String,
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModReport {
    pub analyzer: u32,
    pub sha256: String,
    pub sha512: String,
    pub size: u64,
    /// "sin_alertas" | "desconocido" | "precaucion" | "sospechoso" | "amenaza" | "incompleto"
    pub status: String,
    /// Una frase para la interfaz.
    pub summary: String,
    pub findings: Vec<Finding>,
    pub meta: ModMeta,
    pub nested: Vec<String>,
    /// None = no está en Modrinth o no se pudo consultar (ver `unchecked`).
    pub modrinth: Option<Provenance>,
    /// Qué se revisó.
    pub checked: Vec<String>,
    /// Qué no se pudo revisar (por eso el análisis puede ser incompleto).
    pub unchecked: Vec<String>,
}

// ── Indicadores ──────────────────────────────────────────────────────────────────────────────

/// Indicadores públicos de fractureiser (junio de 2023): servidores de la etapa 0/1 y paquetes
/// del cliente que robaba credenciales y se propagaba a otros `.jar`.
const KNOWN_THREATS: &[(&str, &str)] = &[
    ("85.217.144.130", "servidor de fractureiser"),
    ("107.189.3.101", "servidor de fractureiser"),
    ("files-8ie.pages.dev", "servidor de fractureiser"),
    ("dev/neko/nekoclient", "cliente de fractureiser (nekoclient)"),
    ("dev/neko/nekoinjector", "inyector de fractureiser"),
];

/// Lugares donde se guardan credenciales o datos personales que un mod no tiene por qué leer.
const CREDENTIAL_TARGETS: &[(&str, &str)] = &[
    ("Local Storage\\leveldb", "tokens de Discord"),
    ("Local Storage/leveldb", "tokens de Discord"),
    ("discordcanary", "tokens de Discord"),
    ("Login Data", "contraseñas guardadas del navegador"),
    ("Google\\Chrome\\User Data", "datos del navegador Chrome"),
    ("Google/Chrome/User Data", "datos del navegador Chrome"),
    ("Mozilla\\Firefox\\Profiles", "datos del navegador Firefox"),
    ("BraveSoftware", "datos del navegador Brave"),
    ("launcher_accounts", "cuentas del lanzador de Minecraft"),
    ("microsoft_accounts.json", "cuentas de Minecraft de otro lanzador"),
    ("Lunar Client", "cuentas de Lunar Client"),
    (".feather/accounts", "cuentas de Feather"),
    ("wallet.dat", "billeteras de criptomonedas"),
    ("Exodus\\exodus.wallet", "billeteras de criptomonedas"),
    ("nkbihfbeogaeaoehlefnkodbefgpgknn", "extensión MetaMask"),
    ("\\Microsoft\\Credentials", "credenciales de Windows"),
];

const EXFIL: &[(&str, &str)] = &[
    ("discord.com/api/webhooks", "envío a un webhook de Discord"),
    ("discordapp.com/api/webhooks", "envío a un webhook de Discord"),
    ("api.telegram.org/bot", "envío a un bot de Telegram"),
    ("pastebin.com/raw", "descarga de contenido desde Pastebin"),
];

const PERSISTENCE: &[(&str, &str, Level)] = &[
    ("Start Menu\\Programs\\Startup", "la carpeta de inicio de Windows", Level::Sospechoso),
    ("CurrentVersion\\Run", "el arranque automático del registro de Windows", Level::Sospechoso),
    ("schtasks", "las tareas programadas de Windows", Level::Sospechoso),
    ("-EncodedCommand", "PowerShell con un comando oculto", Level::Sospechoso),
    ("reg add", "cambios en el registro de Windows", Level::Precaucion),
    ("powershell", "PowerShell", Level::Precaucion),
];

/// Partes de 939 Host que un mod no debería tocar.
const HOST939_TARGETS: &[(&str, &str)] = &[
    ("939Host\\identity", "identidad de 939 Host"),
    ("939Host/identity", "identidad de 939 Host"),
    ("node.key", "clave de la PC en 939 Host"),
    ("mods-recibidos.json", "registro de mods recibidos de 939 Host"),
];

const OBFUSCATORS: &[&str] = &["ALLATORIxDEMO", "Obfuscated by", "ZKM", "Stringer", "paramorphism", "Skidfuscator", "branchlock"];

const SCRIPT_EXT: &[&str] = &[".exe", ".bat", ".cmd", ".ps1", ".vbs", ".scr", ".msi", ".com", ".hta", ".js.lnk", ".lnk"];
const NATIVE_EXT: &[&str] = &[".dll", ".so", ".dylib", ".jnilib"];

/// Qué aparece en las clases de un jar (sumado, incluidos los anidados).
#[derive(Default)]
struct Signals {
    classes: usize,
    short_names: usize,
    exec: Option<String>,
    dynload: Option<String>,
    remote_load: Option<String>,
    native_load: Option<String>,
    network: bool,
    jar_writer: Option<String>,
    mods_ref: bool,
    findings: Vec<Finding>,
}

fn contains(hay: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    !n.is_empty() && hay.windows(n.len()).any(|w| w == n)
}

/// ¿Tiene el «constant pool» de la clase una entrada de texto exactamente igual a `s`? (en un
/// `.class`, cada nombre de clase, método o texto literal es: byte 1, largo en 2 bytes, el texto).
/// Evita confundir `exec` con `Executor` o `execute`.
fn cp_entry(hay: &[u8], s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() > u16::MAX as usize { return false; }
    let mut needle = vec![1u8, (b.len() >> 8) as u8, b.len() as u8];
    needle.extend_from_slice(b);
    hay.windows(needle.len()).any(|w| w == needle.as_slice())
}

fn contains_ci(hay: &[u8], needle: &str) -> bool {
    let n = needle.to_ascii_lowercase();
    let n = n.as_bytes();
    !n.is_empty() && hay.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

fn push(sig: &mut Signals, level: Level, code: &str, title: &str, detail: String) {
    if sig.findings.iter().any(|f| f.code == code && f.title == title) { return; }
    sig.findings.push(Finding { level, code: code.into(), title: title.into(), detail });
}

fn scan_class(name: &str, b: &[u8], sig: &mut Signals) {
    sig.classes += 1;
    let stem = name.trim_end_matches(".class");
    let last = stem.rsplit('/').next().unwrap_or(stem);
    if last.len() <= 2 && !last.contains('$') { sig.short_names += 1; }

    for (ioc, what) in KNOWN_THREATS {
        if contains(b, ioc) { push(sig, Level::Amenaza, "amenaza-conocida", &format!("Indicador conocido de malware: {what}"), format!("{name}: «{ioc}»")); }
    }
    for (t, what) in CREDENTIAL_TARGETS {
        if contains_ci(b, t) { push(sig, Level::Sospechoso, "credenciales", &format!("Busca {what}"), format!("{name}: «{t}»")); }
    }
    for (t, what) in EXFIL {
        if contains_ci(b, t) { push(sig, Level::Sospechoso, "exfiltracion", &format!("Contiene {what}"), format!("{name}: «{t}»")); }
    }
    for (t, what, lvl) in PERSISTENCE {
        if contains_ci(b, t) { push(sig, *lvl, "persistencia", &format!("Usa {what}"), format!("{name}: «{t}»")); }
    }
    for (t, what) in HOST939_TARGETS {
        if contains(b, t) { push(sig, Level::Sospechoso, "939host", &format!("Menciona la {what}"), format!("{name}: «{t}»")); }
    }
    for o in OBFUSCATORS {
        if contains(b, o) { push(sig, Level::Precaucion, "ofuscador", "Marcas de un ofuscador de código", format!("{name}: «{o}»")); }
    }
    if sig.exec.is_none() && (cp_entry(b, "java/lang/ProcessBuilder") || (cp_entry(b, "java/lang/Runtime") && cp_entry(b, "exec"))) { sig.exec = Some(name.into()); }
    if sig.dynload.is_none() && (contains(b, "java/net/URLClassLoader") || contains(b, "defineClass")) { sig.dynload = Some(name.into()); }
    if sig.remote_load.is_none() && contains(b, "java/net/URLClassLoader") && (contains(b, "http://") || contains(b, "https://")) { sig.remote_load = Some(name.into()); }
    if sig.native_load.is_none() && contains(b, "loadLibrary") && contains(b, "java/lang/System") { sig.native_load = Some(name.into()); }
    if contains(b, "java/net/URL") || contains(b, "java/net/http/HttpClient") || contains(b, "java/net/Socket") { sig.network = true; }
    if sig.jar_writer.is_none() && (contains(b, "java/util/jar/JarOutputStream") || contains(b, "java/util/zip/ZipOutputStream")) && contains_ci(b, ".jar") { sig.jar_writer = Some(name.into()); }
    if contains(b, "mods/") || contains(b, "mods\\") || contains(b, "\"mods\"") { sig.mods_ref = true; }
}

struct Budget { entries: usize, bytes: u64, incomplete: Vec<String> }

fn read_limited<R: Read>(r: &mut R, size: u64, budget: &mut Budget) -> Option<Vec<u8>> {
    if size > MAX_ENTRY_BYTES { budget.incomplete.push(format!("un archivo interno de {} MB supera el límite de análisis", size / 1_048_576)); return None; }
    if budget.bytes + size > MAX_TOTAL_BYTES { budget.incomplete.push("el contenido descomprimido supera el límite de análisis".into()); return None; }
    let mut buf = Vec::with_capacity(size.min(MAX_ENTRY_BYTES) as usize);
    // `take` protege contra un tamaño declarado falso (bomba de compresión).
    if r.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut buf).is_err() { budget.incomplete.push("un archivo interno no se pudo descomprimir".into()); return None; }
    if buf.len() as u64 > MAX_ENTRY_BYTES { budget.incomplete.push("un archivo interno se descomprime más allá del límite".into()); return None; }
    budget.bytes += buf.len() as u64;
    Some(buf)
}

fn scan_zip<R: Read + std::io::Seek>(z: &mut zip::ZipArchive<R>, prefix: &str, depth: u32, sig: &mut Signals, nested: &mut Vec<String>, budget: &mut Budget) {
    for i in 0..z.len() {
        budget.entries += 1;
        if budget.entries > MAX_ENTRIES { budget.incomplete.push(format!("más de {MAX_ENTRIES} archivos internos")); return; }
        let mut e = match z.by_index(i) {
            Ok(e) => e,
            Err(zip::result::ZipError::UnsupportedArchive(m)) => { budget.incomplete.push(format!("formato no soportado ({m})")); continue; }
            Err(_) => { budget.incomplete.push("una entrada del archivo está dañada".into()); continue; }
        };
        let raw_name = match e.name() { Ok(n) => n.to_string(), Err(_) => { budget.incomplete.push("una entrada tiene un nombre ilegible".into()); continue } };
        let shown = format!("{prefix}{raw_name}");
        if e.encrypted() { push(sig, Level::Precaucion, "cifrado", "Tiene archivos internos cifrados que no se pueden revisar", shown.clone()); budget.incomplete.push("hay archivos internos cifrados".into()); continue; }
        if e.enclosed_name().is_none() { push(sig, Level::Sospechoso, "ruta", "Tiene rutas internas que salen de su carpeta", shown.clone()); }
        if e.is_dir() { continue; }
        let lower = raw_name.to_ascii_lowercase();
        let size = e.size();
        if e.compressed_size() > 0 && size > 50 * 1024 * 1024 && size / e.compressed_size().max(1) > 200 {
            push(sig, Level::Precaucion, "bomba", "Un archivo interno se comprime de forma anormal", format!("{shown}: {} MB", size / 1_048_576));
        }
        if SCRIPT_EXT.iter().any(|x| lower.ends_with(x)) {
            push(sig, Level::Sospechoso, "ejecutable", "Trae un programa o script de Windows adentro", shown.clone());
            continue;
        }
        if NATIVE_EXT.iter().any(|x| lower.ends_with(x)) {
            push(sig, Level::Precaucion, "nativo", "Trae código nativo (algunos mods de rendimiento lo usan)", shown.clone());
            continue;
        }
        if lower.ends_with(".class") {
            if let Some(b) = read_limited(&mut e, size, budget) { scan_class(&shown, &b, sig); }
            continue;
        }
        if lower.ends_with("pom.properties") && lower.contains("log4j-core") {
            if let Some(b) = read_limited(&mut e, size, budget) {
                let t = String::from_utf8_lossy(&b);
                if let Some(v) = t.lines().find_map(|l| l.strip_prefix("version=")) {
                    if log4j_vulnerable(v.trim()) { push(sig, Level::Precaucion, "vulnerable", "Incluye una versión de Log4j con fallas graves conocidas (Log4Shell)", format!("{shown}: {v}")); }
                }
            }
            continue;
        }
        if lower.ends_with(".jar") || lower.ends_with(".zip") {
            nested.push(shown.clone());
            if depth >= MAX_DEPTH { budget.incomplete.push(format!("{shown}: jar anidado demasiado profundo")); continue; }
            if let Some(b) = read_limited(&mut e, size, budget) {
                match zip::ZipArchive::new(std::io::Cursor::new(b)) {
                    Ok(mut inner) => scan_zip(&mut inner, &format!("{shown}!/"), depth + 1, sig, nested, budget),
                    Err(_) => push(sig, Level::Precaucion, "anidado-danado", "Trae un archivo .jar interno que no se puede abrir", shown.clone()),
                }
            }
        }
    }
}

/// Log4j 2.0-beta9 a 2.17.0 tiene fallas de ejecución remota (CVE-2021-44228 y siguientes).
fn log4j_vulnerable(v: &str) -> bool {
    let nums: Vec<u32> = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    match nums.as_slice() {
        [2, minor, patch, ..] => *minor < 17 || (*minor == 17 && *patch < 1),
        [2, minor] => *minor < 17,
        _ => false,
    }
}

fn hashes(path: &Path) -> std::io::Result<(String, String, u64)> {
    let mut f = std::fs::File::open(path)?;
    let (mut a, mut b) = (sha2::Sha256::new(), sha2::Sha512::new());
    let mut buf = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        size += n as u64;
        a.update(&buf[..n]);
        b.update(&buf[..n]);
    }
    Ok((hex::encode(a.finalize()), hex::encode(b.finalize()), size))
}

/// Análisis local (sin red). La procedencia en Modrinth se agrega con [`with_modrinth`].
pub fn analyze(path: &Path) -> ModReport {
    let mut r = ModReport {
        analyzer: ANALYZER_VERSION, sha256: String::new(), sha512: String::new(), size: 0,
        status: "incompleto".into(), summary: String::new(), findings: vec![], meta: ModMeta::default(),
        nested: vec![], modrinth: None, checked: vec![], unchecked: vec![],
    };
    match hashes(path) {
        Ok((h256, h512, size)) => { r.sha256 = h256; r.sha512 = h512; r.size = size; }
        Err(e) => { r.unchecked.push(format!("no se pudo leer el archivo ({e})")); return finish(r, false); }
    }
    let info = crate::repair::read_jar(path);
    r.meta = ModMeta {
        loader: info.kind.clone(), id: info.id.clone(), name: info.name.clone(), version: info.version.clone(),
        depends: info.depends.keys().filter(|d| !["minecraft", "java", "fabricloader", "fabric-loader", "forge", "neoforge"].contains(&d.as_str())).cloned().collect(),
    };
    let file = match std::fs::File::open(path) { Ok(f) => f, Err(e) => { r.unchecked.push(format!("no se pudo abrir ({e})")); return finish(r, false); } };
    let mut z = match zip::ZipArchive::new(file) {
        Ok(z) => z,
        Err(_) => {
            r.findings.push(Finding { level: Level::Precaucion, code: "no-zip".into(), title: "No es un archivo .jar válido".into(), detail: "no se puede abrir como zip".into() });
            r.unchecked.push("contenido del mod (no es un .jar válido)".into());
            return finish(r, false);
        }
    };
    let (mut sig, mut nested) = (Signals::default(), vec![]);
    let mut budget = Budget { entries: 0, bytes: 0, incomplete: vec![] };
    scan_zip(&mut z, "", 0, &mut sig, &mut nested, &mut budget);
    r.nested = nested;
    r.checked.push(format!("estructura del archivo y {} clases", sig.classes));
    if !r.nested.is_empty() { r.checked.push(format!("{} jars incluidos", r.nested.len())); }
    if !info.kind.is_empty() { r.checked.push(format!("metadatos de {}", info.kind)); } else { r.unchecked.push("metadatos de mod (no tiene fabric.mod.json ni mods.toml)".into()); }

    // Combinaciones: cada cosa sola es normal en algunos mods; juntas, no.
    let obfuscated = sig.classes >= 20 && sig.short_names * 10 >= sig.classes * 6;
    if obfuscated { let d = format!("{} de {} clases con nombres de 1 o 2 letras", sig.short_names, sig.classes); push(&mut sig, Level::Precaucion, "ofuscado", "La mayor parte del código está ofuscado", d); }
    if let Some(c) = sig.remote_load.clone() { push(&mut sig, Level::Sospechoso, "carga-remota", "Puede descargar y ejecutar código desde internet", c); }
    if let Some(c) = sig.jar_writer.clone() { if sig.mods_ref { push(&mut sig, Level::Sospechoso, "propagacion", "Puede modificar otros archivos .jar de la carpeta de mods", c); } }
    if let Some(c) = sig.exec.clone() {
        let lvl = if obfuscated { Level::Sospechoso } else { Level::Precaucion };
        push(&mut sig, lvl, "comandos", "Puede ejecutar programas o comandos del sistema", c);
    }
    if let Some(c) = sig.dynload.clone() { if obfuscated { push(&mut sig, Level::Sospechoso, "carga-dinamica", "Código ofuscado que carga más código en tiempo de ejecución", c); } }
    if let Some(c) = sig.native_load.clone() { push(&mut sig, Level::Precaucion, "carga-nativa", "Carga librerías nativas", c); }
    if sig.network { push(&mut sig, Level::Info, "red", "Usa conexiones de red (es normal en muchos mods)", String::new()); }
    let complete = budget.incomplete.is_empty();
    for m in budget.incomplete { if !r.unchecked.contains(&m) { r.unchecked.push(m); } }
    r.findings = sig.findings;
    finish(r, complete)
}

fn finish(mut r: ModReport, complete: bool) -> ModReport {
    r.findings.sort_by(|a, b| b.level.cmp(&a.level));
    let worst = r.findings.iter().map(|f| f.level).max().unwrap_or(Level::Info);
    let (status, summary) = match worst {
        Level::Amenaza => ("amenaza", "Coincide con malware conocido. No lo apruebes."),
        Level::Sospechoso => ("sospechoso", "Tiene comportamientos que no son normales en un mod. No lo apruebes si no sabés por qué los necesita."),
        _ if !complete => ("incompleto", "No se pudo revisar todo el archivo. Que no aparezcan alertas no significa nada."),
        Level::Precaucion => ("precaucion", "Tiene características poco comunes que conviene revisar."),
        Level::Info if r.modrinth.is_some() => ("sin_alertas", "Sin alertas conocidas y publicado en Modrinth. No garantiza que sea seguro."),
        Level::Info => ("desconocido", "Sin alertas, pero no se pudo confirmar de dónde viene."),
    };
    r.status = status.into();
    r.summary = summary.into();
    r
}

/// Agrega la procedencia en Modrinth (consulta solo la huella SHA-512). Si Modrinth no responde,
/// el resultado queda como antes y se anota que no se pudo comprobar.
pub async fn with_modrinth(mut r: ModReport) -> ModReport {
    if r.sha512.is_empty() { return r; }
    r.unchecked.retain(|u| !u.starts_with("procedencia en Modrinth"));
    r.checked.retain(|c| !c.starts_with("procedencia en Modrinth"));
    match modrinth_lookup(&r.sha512).await {
        Ok(Some(p)) => { r.checked.push("procedencia en Modrinth (por huella)".into()); r.modrinth = Some(p); }
        Ok(None) => { r.checked.push("procedencia en Modrinth (por huella): no publicado ahí".into()); r.modrinth = None; }
        Err(e) => { r.unchecked.push(format!("procedencia en Modrinth (sin respuesta: {e})")); }
    }
    let complete = !r.unchecked.iter().any(|u| !u.starts_with("procedencia en Modrinth") && !u.starts_with("metadatos de mod"));
    finish(r, complete)
}

async fn modrinth_lookup(sha512: &str) -> Result<Option<Provenance>, String> {
    let url = format!("{}/version_file/{sha512}?algorithm=sha512", crate::modrinth::API);
    let req = crate::net::HTTP.get(&url).send();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(12), req).await.map_err(|_| "tiempo agotado".to_string())?.map_err(|e| e.to_string())?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND { return Ok(None); }
    if !resp.status().is_success() { return Err(format!("estado {}", resp.status())); }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let project = v["project_id"].as_str().unwrap_or("").to_string();
    if project.is_empty() { return Ok(None); }
    let version = v["version_number"].as_str().unwrap_or("").to_string();
    let title = match crate::net::HTTP.get(format!("{}/project/{project}", crate::modrinth::API)).send().await {
        Ok(p) if p.status().is_success() => p.json::<serde_json::Value>().await.ok().and_then(|j| j["title"].as_str().map(String::from)).unwrap_or_default(),
        _ => String::new(),
    };
    Ok(Some(Provenance { url: format!("https://modrinth.com/project/{project}"), project, title, version }))
}

// ── Caché por huella ─────────────────────────────────────────────────────────────────────────

fn cache_path(sha256: &str) -> std::path::PathBuf { crate::paths::root().join("cache").join("analisis-mods").join(format!("{sha256}.json")) }

/// Análisis guardado de esta huella, si es de la versión actual del analizador.
pub fn cached(sha256: &str) -> Option<ModReport> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    crate::paths::read_json::<ModReport>(&cache_path(sha256)).ok().filter(|r| r.analyzer == ANALYZER_VERSION && r.sha256 == sha256)
}

/// Analiza (o reutiliza el análisis guardado) y consulta Modrinth si todavía no se pudo.
pub async fn analyze_cached(path: &Path) -> AppResult<ModReport> {
    let p = path.to_path_buf();
    let (local, from_cache) = tokio::task::spawn_blocking(move || match hashes(&p).ok().and_then(|(h, _, _)| cached(&h)) {
        Some(c) => (c, true),
        None => (analyze(&p), false),
    }).await.map_err(|e| crate::error::AppError::new("scan", "El análisis se interrumpió").with_detail(e))?;
    if from_cache && !local.unchecked.iter().any(|u| u.starts_with("procedencia en Modrinth")) { return Ok(local); }
    let r = with_modrinth(local).await;
    if !r.sha256.is_empty() { let _ = crate::paths::write_json(&cache_path(&r.sha256), &r); }
    Ok(r)
}

/// Lista de indicadores revisados (para mostrar en «detalles técnicos»).
pub fn rules() -> BTreeSet<&'static str> {
    let mut s = BTreeSet::new();
    for (_, w) in KNOWN_THREATS.iter().chain(CREDENTIAL_TARGETS).chain(EXFIL).chain(HOST939_TARGETS) { s.insert(*w); }
    for (_, w, _) in PERSISTENCE { s.insert(*w); }
    s
}

#[cfg(test)]
mod t {
    use super::*;
    use std::io::Write;

    /// Arma un .jar de prueba. Las «clases» son bytes con las cadenas que buscaría el análisis:
    /// no son código ejecutable (el análisis nunca ejecuta nada).
    fn jar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in files { z.start_file(*n, o).unwrap(); z.write_all(b).unwrap(); }
        z.finish().unwrap().into_inner()
    }
    fn write(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("h939-modscan-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{tag}.jar"));
        std::fs::write(&p, bytes).unwrap();
        p
    }
    const FABRIC: &[u8] = br#"{"schemaVersion":1,"id":"ejemplo","name":"Mod de ejemplo","version":"1.2.3","depends":{"fabricloader":">=0.15","fabric-api":"*","minecraft":"~1.21"}}"#;
    fn status(files: &[(&str, &[u8])]) -> ModReport { analyze(&write(&format!("j{}", sha256_of(files)), &jar(files))) }
    fn sha256_of(files: &[(&str, &[u8])]) -> String { let mut h = sha2::Sha256::new(); for (n, b) in files { h.update(n.as_bytes()); h.update(b); } hex::encode(h.finalize())[..12].to_string() }
    fn codes(r: &ModReport) -> Vec<String> { r.findings.iter().map(|f| f.code.clone()).collect() }
    /// Entradas del «constant pool» como en un .class real.
    fn cp(items: &[&str]) -> Vec<u8> { let mut v = vec![0xCA, 0xFE, 0xBA, 0xBE]; for s in items { v.push(1); v.extend_from_slice(&(s.len() as u16).to_be_bytes()); v.extend_from_slice(s.as_bytes()); } v }

    #[test]
    fn mod_normal_sin_falsas_alarmas() {
        // Red, reflexión y archivos de configuración: normal en un mod.
        let r = status(&[("fabric.mod.json", FABRIC), ("ej/Mod.class", b"java/net/URL https://api.ejemplo.com java/lang/reflect/Method config/ejemplo.json"), ("ej/Util.class", b"java/nio/file/Files")]);
        assert_eq!(r.status, "desconocido", "{:?}", r.findings);
        assert!(r.findings.iter().all(|f| f.level == Level::Info), "{:?}", r.findings);
        assert_eq!((r.meta.id.as_str(), r.meta.version.as_str()), ("ejemplo", "1.2.3"));
        assert_eq!(r.meta.depends, vec!["fabric-api".to_string()], "dependencias sin minecraft ni el loader");
        assert_eq!(r.size, std::fs::metadata(write(&format!("j{}", sha256_of(&[("fabric.mod.json", FABRIC), ("ej/Mod.class", b"java/net/URL https://api.ejemplo.com java/lang/reflect/Method config/ejemplo.json"), ("ej/Util.class", b"java/nio/file/Files")])), &jar(&[("fabric.mod.json", FABRIC), ("ej/Mod.class", b"java/net/URL https://api.ejemplo.com java/lang/reflect/Method config/ejemplo.json"), ("ej/Util.class", b"java/nio/file/Files")]))).unwrap().len());
        // Abrir el navegador con un comando: precaución, no sospecha.
        let abrir = cp(&["java/lang/ProcessBuilder", "xdg-open"]);
        let r = status(&[("fabric.mod.json", FABRIC), ("ej/Abrir.class", &abrir)]);
        assert_eq!(r.status, "precaucion", "{:?}", r.findings);
        // Falso positivo visto con Fabric API y Chunky reales: Runtime (procesadores) + Executor no
        // es ejecutar comandos.
        let hilos = cp(&["java/lang/Runtime", "availableProcessors", "java/util/concurrent/Executors", "execute", "newFixedThreadPool"]);
        let r = status(&[("fabric.mod.json", FABRIC), ("ej/Hilos.class", &hilos)]);
        assert!(r.findings.iter().all(|f| f.level == Level::Info), "{:?}", r.findings);
        let exec = cp(&["java/lang/Runtime", "getRuntime", "exec"]);
        assert_eq!(status(&[("fabric.mod.json", FABRIC), ("ej/Exec.class", &exec)]).status, "precaucion", "Runtime.exec de verdad");
        // Código nativo de un mod de rendimiento: precaución.
        assert_eq!(status(&[("fabric.mod.json", FABRIC), ("natives/rapido.dll", b"MZ")]).status, "precaucion");
    }

    #[test]
    fn malware_conocido_y_robo_de_credenciales() {
        let r = status(&[("fabric.mod.json", FABRIC), ("a/B.class", b"dev/neko/nekoclient/Client")]);
        assert_eq!(r.status, "amenaza", "{:?}", r.findings);
        let r = status(&[("fabric.mod.json", FABRIC), ("a/C.class", b"\\Discord\\Local Storage\\leveldb https://discord.com/api/webhooks/1/x")]);
        assert_eq!(r.status, "sospechoso");
        assert!(codes(&r).contains(&"credenciales".into()) && codes(&r).contains(&"exfiltracion".into()));
        let r = status(&[("fabric.mod.json", FABRIC), ("a/D.class", b"Microsoft\\Windows\\Start Menu\\Programs\\Startup")]);
        assert_eq!(r.status, "sospechoso");
        let r = status(&[("fabric.mod.json", FABRIC), ("a/E.class", b"%LOCALAPPDATA%\\939Host\\identity\\node.key")]);
        assert_eq!(r.status, "sospechoso", "apuntar a la identidad de 939 Host");
        // Descargar y ejecutar código desde internet.
        let r = status(&[("fabric.mod.json", FABRIC), ("a/F.class", b"java/net/URLClassLoader http://198.51.100.7/x loadClass")]);
        assert!(codes(&r).contains(&"carga-remota".into()) && r.status == "sospechoso");
        // Propagarse a otros .jar (como fractureiser).
        let r = status(&[("fabric.mod.json", FABRIC), ("a/G.class", b"java/util/jar/JarOutputStream mods/ .jar")]);
        assert!(codes(&r).contains(&"propagacion".into()) && r.status == "sospechoso");
        // Un programa de Windows dentro del mod.
        assert_eq!(status(&[("fabric.mod.json", FABRIC), ("tools/run.ps1", b"Invoke-WebRequest")]).status, "sospechoso");
        // Rutas internas que salen de la carpeta (zip slip).
        assert_eq!(status(&[("fabric.mod.json", FABRIC), ("../../escapar.txt", b"x")]).status, "sospechoso");
    }

    #[test]
    fn ofuscacion_y_dependencias_maliciosas() {
        let mut files: Vec<(String, Vec<u8>)> = (0u8..30).map(|i| (format!("a/{}{}.class", (b'a' + i / 26) as char, (b'a' + i % 26) as char), b"x".to_vec())).collect();
        files.push(("fabric.mod.json".into(), FABRIC.to_vec()));
        let as_ref: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let r = status(&as_ref);
        assert!(codes(&r).contains(&"ofuscado".into()) && r.status == "precaucion", "ofuscado solo: precaución {:?}", r.findings);
        files.push(("a/zz.class".into(), cp(&["java/lang/ProcessBuilder"])));
        let as_ref: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        assert_eq!(status(&as_ref).status, "sospechoso", "ofuscado y ejecuta comandos");
        // Una librería incluida (jar dentro del jar) con un ladrón de credenciales.
        let inner = jar(&[("x/Robar.class", b"Google\\Chrome\\User Data\\Default\\Login Data")]);
        let r = status(&[("fabric.mod.json", FABRIC), ("META-INF/jars/libreria.jar", &inner)]);
        assert_eq!(r.status, "sospechoso");
        assert!(r.findings.iter().any(|f| f.detail.contains("libreria.jar!/x/Robar.class")), "dice dónde: {:?}", r.findings);
        assert_eq!(r.nested, vec!["META-INF/jars/libreria.jar".to_string()]);
        // Log4j vulnerable incluido.
        let r = status(&[("fabric.mod.json", FABRIC), ("META-INF/maven/org.apache.logging.log4j/log4j-core/pom.properties", b"version=2.14.1\n")]);
        assert!(codes(&r).contains(&"vulnerable".into()));
        assert!(!log4j_vulnerable("2.17.1") && log4j_vulnerable("2.16.0") && log4j_vulnerable("2.17.0"));
    }

    #[test]
    fn analisis_incompleto_nunca_es_seguro() {
        // No es un zip.
        let r = analyze(&write("dañado", b"esto no es un jar"));
        assert_eq!(r.status, "incompleto");
        // Bomba de compresión: un archivo interno que se descomprime por encima del límite.
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap(); z.write_all(FABRIC).unwrap();
        z.start_file("grande.class", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true)).unwrap();
        let chunk = vec![0u8; 1 << 20];
        for _ in 0..70 { z.write_all(&chunk).unwrap(); }
        let bomb = z.finish().unwrap().into_inner();
        assert!(bomb.len() < 2_000_000);
        let r = analyze(&write("bomba", &bomb));
        assert_eq!(r.status, "incompleto", "{:?} / {:?}", r.findings, r.unchecked);
        // Aunque no haya alertas, si falta revisar algo no se dice «sin alertas».
        let mut ok = analyze(&write("ok", &jar(&[("fabric.mod.json", FABRIC)])));
        ok.modrinth = Some(Provenance { project: "p".into(), title: "t".into(), version: "1".into(), url: String::new() });
        assert_eq!(finish(ok.clone(), true).status, "sin_alertas");
        assert_eq!(finish(ok, false).status, "incompleto");
    }

    /// Si Modrinth no responde (o no existe el archivo ahí), el resultado nunca es «sin alertas».
    #[tokio::test]
    async fn sin_procedencia_no_hay_sin_alertas() {
        let _ = crate::paths::init(std::env::temp_dir().join(format!("h939-modscan-root-{}", std::process::id())));
        let unico = format!("{{\"schemaVersion\":1,\"id\":\"unico{}\",\"version\":\"1\"}}", std::process::id());
        let r = with_modrinth(analyze(&write("unico", &jar(&[("fabric.mod.json", unico.as_bytes())])))).await;
        assert_eq!(r.status, "desconocido", "{:?}", r.unchecked);
        assert!(r.modrinth.is_none());
        assert!(r.checked.iter().chain(&r.unchecked).any(|x| x.starts_with("procedencia en Modrinth")), "dice qué pasó con la consulta");
    }
}
